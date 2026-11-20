(* Exact port of marked@18.1.1 src/Tokenizer.ts. *)

open Host
open Tokens
open Helpers

let s_dollar1 = Jstr.of_string "$1"
let s_nl = Jstr.of_string "\n"
let s_space = Jstr.of_string " "
let s_two_spaces = Jstr.of_string "  "
let s_four_spaces = Jstr.of_string "    "
let s_setext_repl = Jstr.of_string "\n    $1"
let s_pipe = Jstr.of_string "|"
let s_lparen = Jstr.of_string "("
let s_rparen = Jstr.of_string ")"
let s_lt = Jstr.of_string "<"
let s_backslash = Jstr.of_string "\\"
let s_ordered_prefix = Jstr.of_string "\\d{1,9}\\"
let s_pedantic_bullet = Jstr.of_string "[*+-]"
let s_unchecked = Jstr.of_string "[ ]"
let s_at = Jstr.of_string "@"
let s_mailto = Jstr.of_string "mailto:"
let s_www = Jstr.of_string "www."
let s_http = Jstr.of_string "http://"

let opt_nonempty (o : Jstr.t option) : Jstr.t option =
  match o with Some s when Jstr.length s > 0 -> Some s | _ -> None

let output_link (cap0 : Jstr.t) (cap1 : Jstr.t) (href : Jstr.t) (title_opt : Jstr.t option)
    (raw : Jstr.t) (lex : lexer) : token option =
  let title = opt_nonempty title_opt in
  let text = Re.replace_str cap1 Rules.Other.output_link_replace s_dollar1 in
  let is_image = Jstr.length cap0 > 0 && Jstr.code_unit_at cap0 0 = 33 in
  let outer_in_link = lex.state.in_link in
  let outer_link_emitted = lex.state.link_emitted in
  let outer_in_raw_block = lex.state.in_raw_block in
  lex.state.in_link <- true;
  lex.state.link_emitted <- false;
  let tokens = lex.inline_tokens_fn text (Vec.create ()) in
  let text_has_link = lex.state.link_emitted in
  lex.state.link_emitted <- outer_link_emitted;
  lex.state.in_link <- outer_in_link;
  if not is_image then (
    if text_has_link then (
      lex.state.in_raw_block <- outer_in_raw_block;
      None)
    else (
      lex.state.link_emitted <- true;
      Some (Link { raw; href; title; text; autolink = false; tokens })))
  else Some (Image { raw; href; title; text; tokens = Some tokens })

let indent_code_compensation (raw : Jstr.t) (text : Jstr.t) : Jstr.t =
  let m = Re.exec_raw Rules.Other.indent_code_compensation raw in
  if not (Re.is_match m) then text
  else
    let indent_len = Jstr.length (Re.match_group_or_empty m 1) in
    let lines = Jstr.split_str text s_nl in
    let n = Vec.length lines in
    let out = Vec.create () in
    for i = 0 to n - 1 do
      let node = Vec.get lines i in
      let m2 = Re.exec_raw Rules.Other.beginning_space node in
      let compensated =
        if not (Re.is_match m2) then node
        else
          let node_indent = Jstr.length (Re.match_full m2) in
          let cut = if node_indent < indent_len then node_indent else indent_len in
          Jstr.slice_from node cut
      in
      Vec.push out compensated
    done;
    Jstr.join out s_nl

let is_label_end_inside_token (src : Jstr.t) (label : Jstr.t) (label_start : int)
    (rules : Rules.rules) : bool =
  if not (Jstr.includes label s_lt) then false
  else
    let label_len = Jstr.length label in
    let i = ref 0 in
    let found = ref false in
    while !i < label_len && not !found do
      let c = Jstr.code_unit_at label !i in
      if c = 92 then i := !i + 2
      else if c = 96 then (
        let code = Re.exec_raw rules.inline.code (Jstr.slice_from label !i) in
        if Re.is_match code then i := !i + Jstr.length (Re.match_full code) else incr i)
      else if c <> 60 then incr i
      else
        let token_src = Jstr.slice_from src (label_start + !i) in
        let m_tag = Re.exec_raw rules.inline.tag token_src in
        let tok =
          if Re.is_match m_tag then m_tag else Re.exec_raw rules.inline.autolink token_src
        in
        if not (Re.is_match tok) then incr i
        else
          let tok_len = Jstr.length (Re.match_full tok) in
          if tok_len > label_len - !i then found := true else i := !i + tok_len
    done;
    !found

let space (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.block.newline src in
  if not (Re.is_match cap) then None
  else
    let raw = Re.match_full cap in
    if Jstr.length raw > 0 then Some (Space { raw }) else None

let code (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.block.code src in
  if not (Re.is_match cap) then None
  else
    let cap0 = Re.match_full cap in
    let raw = if lex.options.pedantic then cap0 else trim_trailing_blank_lines cap0 in
    let text = Re.replace_str raw Rules.Other.code_remove_indent Jstr.empty in
    Some (Code { raw; lang = None; text; escaped = false })

let fences (lex : lexer) (src : Jstr.t) : token option =
  match lex.rules.block.fences with
  | None -> None
  | Some fences_re ->
      let cap = Re.exec_raw fences_re src in
      if not (Re.is_match cap) then None
      else
        let raw = Re.match_full cap in
        let body = Re.match_group_or_empty cap 3 in
        let text = indent_code_compensation raw body in
        let g2 = Re.match_group_or_empty cap 2 in
        let lang =
          if Jstr.length g2 > 0 then
            Some (Re.replace_str (Jstr.trim g2) lex.rules.inline.any_punctuation s_dollar1)
          else None
        in
        Some (Code { raw; lang; text; escaped = false })

let heading (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.block.heading src in
  if not (Re.is_match cap) then None
  else
    let cap0 = Re.match_full cap in
    let cap1 = Re.match_group_or_empty cap 1 in
    let cap2 = Re.match_group_or_empty cap 2 in
    let text = ref (Jstr.trim cap2) in
    if Re.test Rules.Other.ending_hash !text then (
      let trimmed = rtrim !text 35 false in
      if
        lex.options.pedantic
        || Jstr.length trimmed = 0
        || Re.test Rules.Other.ending_space_tab_char trimmed
      then text := Jstr.trim trimmed);
    let final_text = !text in
    Some
      (Heading
         {
           raw = rtrim cap0 10 false;
           depth = Jstr.length cap1;
           text = final_text;
           tokens = enqueue_inline lex final_text;
         })

let hr (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.block.hr src in
  if not (Re.is_match cap) then None else Some (Hr { raw = rtrim (Re.match_full cap) 10 false })

let list (lex : lexer) (src_in : Jstr.t) : token option =
  let init_cap = Re.exec_raw lex.rules.block.list src_in in
  if not (Re.is_match init_cap) then None
  else
    let list_src = src_in in
    let list_src_len = Jstr.length list_src in
    let src = ref src_in in
    let taken () = list_src_len - Jstr.length !src in
    let bull_trimmed = Jstr.trim (Re.match_group_or_empty init_cap 1) in
    let is_ordered = Jstr.length bull_trimmed > 1 in
    let start =
      if is_ordered then Jstr.parse_int (Jstr.slice bull_trimmed 0 (-1)) 10 else 0
    in
    let bull_last = Jstr.slice_from bull_trimmed (-1) in
    let bull_re_str =
      if is_ordered then Jstr.concat s_ordered_prefix bull_last
      else if lex.options.pedantic then s_pedantic_bullet
      else Jstr.concat s_backslash bull_last
    in
    let item_regex = Rules.Other.list_item_regex bull_re_str in
    let items = Vec.create () in
    let loose = ref false in
    let ends_with_blank_line = ref false in
    let outer_done = ref false in
    while (not !outer_done) && Jstr.length !src > 0 do
      let cap = Re.exec_raw item_regex !src in
      if (not (Re.is_match cap)) || Re.test lex.rules.block.hr !src then outer_done := true
      else (
        let item_start = taken () in
        let cap0 = Re.match_full cap in
        let cap1 = Re.match_group_or_empty cap 1 in
        let cap2 = Re.match_group_or_empty cap 2 in
        src := Jstr.substring_from !src (Jstr.length cap0);
        let first = first_line cap2 in
        let bullet_indent = Jstr.length cap1 in
        let line =
          ref
            (if lex.options.pedantic then expand_tabs first bullet_indent
             else
               Re.replace_fn1 first Rules.Other.leading_space_tab (fun ws ->
                   expand_tabs ws bullet_indent))
        in
        let next_line = ref (first_line !src) in
        let blank_line = ref (Jstr.length (Jstr.trim !line) = 0) in
        let indent = ref 0 in
        let item_parts = Vec.create () in
        if lex.options.pedantic then (
          indent := 2;
          Vec.push item_parts (Jstr.trim_start !line))
        else if !blank_line then indent := bullet_indent + 1
        else (
          let idx = Re.search !line Rules.Other.non_space_char in
          let adj = if idx > 4 then 1 else idx in
          Vec.push item_parts (Jstr.slice_from !line adj);
          indent := adj + bullet_indent);
        let end_early = ref false in
        if !blank_line && Re.test Rules.Other.blank_line !next_line then (
          src := Jstr.substring_from !src (Jstr.length !next_line + 1);
          end_early := true);
        if not !end_early then (
          let ind = !indent in
          let next_bullet_re = Rules.Other.next_bullet_regex ind in
          let hr_re = Rules.Other.hr_regex ind in
          let fences_begin_re = Rules.Other.fences_begin_regex ind in
          let heading_begin_re = Rules.Other.heading_begin_regex ind in
          let html_begin_re = Rules.Other.html_begin_regex ind in
          let blockquote_begin_re = Rules.Other.blockquote_begin_regex ind in
          let inner_done = ref false in
          while (not !inner_done) && Jstr.length !src > 0 do
            let raw_line = first_line !src in
            let nl, nl_no_tabs =
              if lex.options.pedantic then
                let r = Re.replace_str raw_line Rules.Other.list_replace_nesting s_two_spaces in
                (r, r)
              else
                ( raw_line,
                  Re.replace_with1 raw_line Rules.Other.leading_space_tab expand_tabs_zero_replacer
                )
            in
            next_line := nl;
            if
              Re.test fences_begin_re nl
              || Re.test heading_begin_re nl
              || Re.test html_begin_re nl
              || Re.test blockquote_begin_re nl
              || Re.test next_bullet_re nl
              || Re.test hr_re nl
            then inner_done := true
            else (
              let nl_trim_empty = Jstr.length (Jstr.trim nl) = 0 in
              if Re.search nl_no_tabs Rules.Other.non_space_char >= ind || nl_trim_empty then (
                Vec.push item_parts s_nl;
                Vec.push item_parts (Jstr.slice_from nl_no_tabs ind);
                blank_line := nl_trim_empty;
                src := Jstr.substring_from !src (Jstr.length raw_line + 1);
                line := Jstr.slice_from nl_no_tabs ind)
              else if
                !blank_line
                || Re.search
                     (Re.replace_str !line Rules.Other.tab_char_global s_four_spaces)
                     Rules.Other.non_space_char
                   >= 4
                || Re.test fences_begin_re !line
                || Re.test heading_begin_re !line
                || Re.test hr_re !line
              then inner_done := true
              else (
                Vec.push item_parts s_nl;
                Vec.push item_parts nl;
                blank_line := nl_trim_empty;
                src := Jstr.substring_from !src (Jstr.length raw_line + 1);
                line := Jstr.slice_from nl_no_tabs ind))
          done);
        let item_contents = Jstr.join item_parts Jstr.empty in
        let raw = Jstr.slice list_src item_start (taken ()) in
        if not !loose then
          if !ends_with_blank_line then loose := true
          else if Re.test Rules.Other.double_blank_line raw then ends_with_blank_line := true;
        let is_task = lex.options.gfm && Re.test Rules.Other.list_is_task item_contents in
        Vec.push items
          {
            item_raw = raw;
            item_task = is_task;
            item_checked = false;
            item_loose = false;
            item_text = item_contents;
            item_tokens = Vec.create ();
          })
    done;
    let n_items = Vec.length items in
    if n_items = 0 then None
    else (
      let last_item = Vec.get items (n_items - 1) in
      last_item.item_raw <- Jstr.trim_end last_item.item_raw;
      last_item.item_text <- Jstr.trim_end last_item.item_text;
      let list_raw = Jstr.trim_end (Jstr.slice list_src 0 (taken ())) in
      for i = 0 to n_items - 1 do
        let item = Vec.get items i in
        lex.state.top <- false;
        let toks = lex.block_tokens_fn item.item_text (Vec.create ()) false in
        item.item_tokens <- toks;
        if not !loose then (
          let k = ref 0 in
          let nt = Vec.length toks in
          let has_multi = ref false in
          while !k < nt && not !has_multi do
            (match Vec.get toks !k with
            | Space s -> if Re.test Rules.Other.any_line s.raw then has_multi := true
            | _ -> ());
            incr k
          done;
          if !has_multi then loose := true)
      done;
      let is_loose = !loose in
      for i = 0 to n_items - 1 do
        let item = Vec.get items i in
        if item.item_task then
          let matched_text =
            if Vec.length item.item_tokens > 0 then
              match Vec.get item.item_tokens 0 with Text t -> Some t | _ -> None
            else None
          in
          match matched_text with
          | None -> item.item_task <- false
          | Some item_tok ->
              item.item_text <-
                Re.replace_str item.item_text Rules.Other.list_replace_task Jstr.empty;
              item_tok.t_raw <-
                Re.replace_str item_tok.t_raw Rules.Other.list_replace_task Jstr.empty;
              item_tok.t_text <-
                Re.replace_str item_tok.t_text Rules.Other.list_replace_task Jstr.empty;
              let q = ref (Vec.length lex.inline_queue - 1) in
              let q_done = ref false in
              while !q >= 0 && not !q_done do
                let entry = Vec.get lex.inline_queue !q in
                if Re.test Rules.Other.list_is_task entry.iq_src then (
                  entry.iq_src <-
                    Re.replace_str entry.iq_src Rules.Other.list_replace_task Jstr.empty;
                  q_done := true)
                else decr q
              done;
              let task_cap = Re.exec_raw Rules.Other.list_task_checkbox item.item_raw in
              if Re.is_match task_cap then (
                let task_raw = Re.match_full task_cap in
                let cb_raw = Jstr.concat task_raw s_space in
                let is_checked = not (Jstr.equal task_raw s_unchecked) in
                let cb_tok = Checkbox { raw = cb_raw; checked = is_checked } in
                item.item_checked <- is_checked;
                if is_loose then (
                  match item_tok.t_tokens with
                  | Some sub_toks ->
                      item_tok.t_raw <- Jstr.concat cb_raw item_tok.t_raw;
                      item_tok.t_text <- Jstr.concat cb_raw item_tok.t_text;
                      Vec.unshift sub_toks cb_tok
                  | None ->
                      let sub = Vec.create () in
                      Vec.push sub cb_tok;
                      Vec.unshift item.item_tokens
                        (Text
                           {
                             t_raw = cb_raw;
                             t_text = cb_raw;
                             t_escaped = false;
                             t_is_paragraph = true;
                             t_tokens = Some sub;
                           }))
                else Vec.unshift item.item_tokens cb_tok)
      done;
      if is_loose then
        for i = 0 to n_items - 1 do
          let item = Vec.get items i in
          item.item_loose <- true;
          let nt = Vec.length item.item_tokens in
          for j = 0 to nt - 1 do
            match Vec.get item.item_tokens j with
            | Text t -> t.t_is_paragraph <- true
            | _ -> ()
          done
        done;
      Some (List { raw = list_raw; ordered = is_ordered; start; loose = is_loose; items }))

let rec blockquote (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.block.blockquote src in
  if not (Re.is_match cap) then None
  else
    let lines = ref (Jstr.split_str (rtrim (Re.match_full cap) 10 false) s_nl) in
    let raw = ref Jstr.empty in
    let text = ref Jstr.empty in
    let tokens = Vec.create () in
    let stop = ref false in
    while (not !stop) && Vec.length !lines > 0 do
      let in_bq = ref false in
      let cur_lines = Vec.create () in
      let cur = !lines in
      let n_cur = Vec.length cur in
      let i = ref 0 in
      let scan_done = ref false in
      while !i < n_cur && not !scan_done do
        let ln = Vec.get cur !i in
        if Re.test Rules.Other.blockquote_start ln then (
          Vec.push cur_lines ln;
          in_bq := true;
          incr i)
        else if not !in_bq then (
          Vec.push cur_lines ln;
          incr i)
        else scan_done := true
      done;
      lines := Vec.slice_from cur !i;
      let current_raw = Jstr.join cur_lines s_nl in
      let current_text =
        Re.replace_str
          (Re.replace_str current_raw Rules.Other.blockquote_setext_replace s_setext_repl)
          Rules.Other.blockquote_setext_replace2 Jstr.empty
      in
      raw := if Jstr.length !raw > 0 then Jstr.concat3 !raw s_nl current_raw else current_raw;
      text :=
        if Jstr.length !text > 0 then Jstr.concat3 !text s_nl current_text else current_text;
      let saved_top = lex.state.top in
      lex.state.top <- true;
      ignore (lex.block_tokens_fn current_text tokens true);
      lex.state.top <- saved_top;
      if Vec.length !lines = 0 then stop := true
      else
        let nt = Vec.length tokens in
        if nt > 0 then
          match Vec.get tokens (nt - 1) with
          | Code _ | Heading _ | Hr _ | Html _ -> stop := true
          | Blockquote old_bq -> (
              let continuation = Jstr.join !lines s_nl in
              let stripped_cont =
                Re.replace_str continuation Rules.Other.blockquote_setext_replace2 Jstr.empty
              in
              let new_text = Jstr.concat3 old_bq.raw s_nl stripped_cont in
              match blockquote lex new_text with
              | Some (Blockquote new_bq as new_tok) ->
                  Vec.set tokens (nt - 1) new_tok;
                  let leftover =
                    Re.replace_str
                      (Jstr.substring_from new_text (Jstr.length new_bq.raw))
                      Rules.Other.leading_newline Jstr.empty
                  in
                  let leftover_count =
                    if Jstr.length leftover > 0 then Vec.length (Jstr.split_str leftover s_nl)
                    else 0
                  in
                  let rem_len = Vec.length !lines in
                  let consumed_len =
                    if leftover_count > 0 then
                      let k = rem_len - leftover_count in
                      if k > 0 then k else 0
                    else rem_len
                  in
                  if consumed_len > 0 then (
                    let consumed = Vec.slice !lines 0 consumed_len in
                    raw := Jstr.concat3 !raw s_nl (Jstr.join consumed s_nl));
                  text :=
                    Jstr.concat
                      (Jstr.substring !text 0 (Jstr.length !text - Jstr.length old_bq.text))
                      new_bq.text;
                  stop := true
              | _ -> stop := true)
          | List old_list -> (
              let new_text = Jstr.concat3 old_list.raw s_nl (Jstr.join !lines s_nl) in
              match list lex new_text with
              | Some (List new_list as new_tok) ->
                  Vec.set tokens (nt - 1) new_tok;
                  raw :=
                    Jstr.concat
                      (Jstr.substring !raw 0 (Jstr.length !raw - Jstr.length old_list.raw))
                      new_list.raw;
                  text :=
                    Jstr.concat
                      (Jstr.substring !text 0 (Jstr.length !text - Jstr.length old_list.raw))
                      new_list.raw;
                  lines :=
                    Jstr.split_str (Jstr.substring_from new_text (Jstr.length new_list.raw)) s_nl
              | _ -> stop := true)
          | _ -> ()
    done;
    Some (Blockquote { raw = !raw; tokens; text = !text })

let html (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.block.html src in
  if not (Re.is_match cap) then None
  else
    let raw = trim_trailing_blank_lines (Re.match_full cap) in
    Some (Html { raw; block = true; text = raw })

let def (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.block.def src in
  if not (Re.is_match cap) then None
  else
    let cap0 = Re.match_full cap in
    let cap1 = Re.match_group_or_empty cap 1 in
    let cap2 = Re.match_group_or_empty cap 2 in
    if
      (not (Re.test Rules.Other.start_angle_bracket cap2))
      && find_closing_bracket cap2 s_lparen s_rparen 40 41 <> -1
    then None
    else
      let tag = Re.replace_str (normalize_label cap1) Rules.Other.multiple_space_global s_space in
      let href =
        if Jstr.length cap2 > 0 then
          Re.replace_str
            (Re.replace_str cap2 Rules.Other.href_brackets s_dollar1)
            lex.rules.inline.any_punctuation s_dollar1
        else Jstr.empty
      in
      let g3 = Re.match_group_or_empty cap 3 in
      let title =
        if Jstr.length g3 > 0 then
          let inner = Jstr.substring g3 1 (Jstr.length g3 - 1) in
          Some (Re.replace_str inner lex.rules.inline.any_punctuation s_dollar1)
        else None
      in
      Some (Def { tag; raw = rtrim cap0 10 false; href; title })

let table (lex : lexer) (src : Jstr.t) : token option =
  match lex.rules.block.table with
  | None -> None
  | Some table_re ->
      let cap = Re.exec_raw table_re src in
      if not (Re.is_match cap) then None
      else
        let cap0 = Re.match_full cap in
        let cap1 = Re.match_group_or_empty cap 1 in
        let cap2 = Re.match_group_or_empty cap 2 in
        if not (Re.test Rules.Other.table_delimiter cap2) then None
        else
          let headers = split_cells cap1 0 in
          let align_strs =
            Jstr.split_str (Re.replace_str cap2 Rules.Other.table_align_chars Jstr.empty) s_pipe
          in
          let n_cols = Vec.length headers in
          if n_cols <> Vec.length align_strs then None
          else
            let align = Vec.create () in
            for i = 0 to n_cols - 1 do
              let a = Vec.get align_strs i in
              let al =
                if Re.test Rules.Other.table_align_right a then Align_right
                else if Re.test Rules.Other.table_align_center a then Align_center
                else if Re.test Rules.Other.table_align_left a then Align_left
                else Align_none
              in
              Vec.push align al
            done;
            let header = Vec.create () in
            for i = 0 to n_cols - 1 do
              let text = Vec.get headers i in
              Vec.push header
                {
                  cell_text = text;
                  cell_tokens = enqueue_inline lex text;
                  cell_header = true;
                  cell_align = Vec.get align i;
                }
            done;
            let g3 = Re.match_group_or_empty cap 3 in
            let row_lines =
              if Jstr.length (Jstr.trim g3) > 0 then
                Jstr.split_str
                  (Re.replace_str g3 Rules.Other.table_row_blank_line Jstr.empty)
                  s_nl
              else Vec.create ()
            in
            let n_rows = Vec.length row_lines in
            let rows = Vec.create () in
            for r = 0 to n_rows - 1 do
              let cells = split_cells (Vec.get row_lines r) n_cols in
              let row_vec = Vec.create () in
              for c = 0 to n_cols - 1 do
                let text = Vec.get cells c in
                Vec.push row_vec
                  {
                    cell_text = text;
                    cell_tokens = enqueue_inline lex text;
                    cell_header = false;
                    cell_align = Vec.get align c;
                  }
              done;
              Vec.push rows row_vec
            done;
            Some (Table { raw = rtrim cap0 10 false; header; align; rows })

let lheading (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.block.lheading src in
  if not (Re.is_match cap) then None
  else
    let cap0 = Re.match_full cap in
    let cap1 = Re.match_group_or_empty cap 1 in
    let cap2 = Re.match_group_or_empty cap 2 in
    let text = Jstr.trim cap1 in
    let depth = if Jstr.length cap2 > 0 && Jstr.code_unit_at cap2 0 = 61 then 1 else 2 in
    Some (Heading { raw = rtrim cap0 10 false; depth; text; tokens = enqueue_inline lex text })

let paragraph (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.block.paragraph src in
  if not (Re.is_match cap) then None
  else
    let cap0 = Re.match_full cap in
    let cap1 = Re.match_group_or_empty cap 1 in
    let l1 = Jstr.length cap1 in
    let text =
      if l1 > 0 && Jstr.code_unit_at cap1 (l1 - 1) = 10 then Jstr.slice cap1 0 (-1) else cap1
    in
    Some
      (Text
         {
           t_raw = cap0;
           t_text = text;
           t_escaped = false;
           t_is_paragraph = true;
           t_tokens = Some (enqueue_inline lex text);
         })

let block_text (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.block.text src in
  if not (Re.is_match cap) then None
  else
    let cap0 = Re.match_full cap in
    Some
      (Text
         {
           t_raw = cap0;
           t_text = cap0;
           t_escaped = false;
           t_is_paragraph = false;
           t_tokens = Some (enqueue_inline lex cap0);
         })

(* Inline tokenizers *)

let escape (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.inline.escape src in
  if not (Re.is_match cap) then None
  else Some (Escape { raw = Re.match_full cap; text = Re.match_group_or_empty cap 1 })

let tag (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.inline.tag src in
  if not (Re.is_match cap) then None
  else
    let cap0 = Re.match_full cap in
    if (not lex.state.in_link) && Re.test Rules.Other.start_a_tag cap0 then
      lex.state.in_link <- true
    else if lex.state.in_link && Re.test Rules.Other.end_a_tag cap0 then
      lex.state.in_link <- false;
    if (not lex.state.in_raw_block) && Re.test Rules.Other.start_pre_script_tag cap0 then
      lex.state.in_raw_block <- true
    else if lex.state.in_raw_block && Re.test Rules.Other.end_pre_script_tag cap0 then
      lex.state.in_raw_block <- false;
    Some (Html { raw = cap0; block = false; text = cap0 })

let link (lex : lexer) (src : Jstr.t) : token option =
  if not lex.state.link_paren_possible then None
  else
    let cap = Re.exec_raw lex.rules.inline.link src in
    if not (Re.is_match cap) then None
    else
      let cap0 = ref (Re.match_full cap) in
      let cap1 = Re.match_group_or_empty cap 1 in
      let cap2 = ref (Re.match_group_or_empty cap 2) in
      let cap3 = ref (Re.match_group_or_empty cap 3) in
      let label_start =
        if Jstr.length !cap0 > 0 && Jstr.code_unit_at !cap0 0 = 33 then 2 else 1
      in
      if (not lex.options.pedantic) && is_label_end_inside_token src cap1 label_start lex.rules
      then None
      else
        let trimmed_url = Jstr.trim !cap2 in
        let valid =
          if (not lex.options.pedantic) && Re.test Rules.Other.start_angle_bracket trimmed_url then
            if not (Re.test Rules.Other.end_angle_bracket trimmed_url) then false
            else
              let rtrim_slash = rtrim (Jstr.slice trimmed_url 0 (-1)) 92 false in
              (Jstr.length trimmed_url - Jstr.length rtrim_slash) mod 2 <> 0
          else
            let last_paren = find_closing_bracket !cap2 s_lparen s_rparen 40 41 in
            if last_paren = -2 then false
            else (
              if last_paren > -1 then (
                let start =
                  if Jstr.length !cap0 > 0 && Jstr.code_unit_at !cap0 0 = 33 then 5 else 4
                in
                let link_len = start + Jstr.length cap1 + last_paren in
                cap2 := Jstr.substring !cap2 0 last_paren;
                cap0 := Jstr.trim (Jstr.substring !cap0 0 link_len);
                cap3 := Jstr.empty);
              true)
        in
        if not valid then None
        else
          let href = ref !cap2 in
          let title = ref Jstr.empty in
          if lex.options.pedantic then (
            let pm = Re.exec_raw Rules.Other.pedantic_href_title !href in
            if Re.is_match pm then (
              href := Re.match_group_or_empty pm 1;
              title := Re.match_group_or_empty pm 3))
          else if Jstr.length !cap3 > 0 then title := Jstr.slice !cap3 1 (-1);
          href := Jstr.trim !href;
          if Re.test Rules.Other.start_angle_bracket !href then
            if lex.options.pedantic && not (Re.test Rules.Other.end_angle_bracket trimmed_url) then
              href := Jstr.slice_from !href 1
            else href := Jstr.slice !href 1 (-1);
          let final_href =
            if Jstr.length !href > 0 then
              Re.replace_str !href lex.rules.inline.any_punctuation s_dollar1
            else !href
          in
          let final_title =
            if Jstr.length !title > 0 then
              Some (Re.replace_str !title lex.rules.inline.any_punctuation s_dollar1)
            else None
          in
          output_link !cap0 cap1 final_href final_title !cap0 lex

let reflink (lex : lexer) (src : Jstr.t) : token option =
  let m1 = Re.exec_raw lex.rules.inline.reflink src in
  let cap = if Re.is_match m1 then m1 else Re.exec_raw lex.rules.inline.nolink src in
  if not (Re.is_match cap) then None
  else
    let cap0 = Re.match_full cap in
    let cap1 = Re.match_group_or_empty cap 1 in
    let label_start = if Jstr.length cap0 > 0 && Jstr.code_unit_at cap0 0 = 33 then 2 else 1 in
    if (not lex.options.pedantic) && is_label_end_inside_token src cap1 label_start lex.rules then
      None
    else
      let g2 = Re.match_group_or_empty cap 2 in
      let raw_ref = if Jstr.length g2 > 0 then g2 else cap1 in
      let link_str = Re.replace_str raw_ref Rules.Other.multiple_space_global s_space in
      let key = normalize_label link_str in
      if not (StrMap.has lex.links key) then
        let ch = Jstr.slice cap0 0 1 in
        Some
          (Text
             {
               t_raw = ch;
               t_text = ch;
               t_escaped = false;
               t_is_paragraph = false;
               t_tokens = None;
             })
      else
        let ld = StrMap.get_exn lex.links key in
        output_link cap0 cap1 ld.link_href ld.link_title cap0 lex

let first_group_1_to_6 (m : Re.match_result) : Jstr.t =
  let g1 = Re.match_group_or_empty m 1 in
  if Jstr.length g1 > 0 then g1
  else
    let g2 = Re.match_group_or_empty m 2 in
    if Jstr.length g2 > 0 then g2
    else
      let g3 = Re.match_group_or_empty m 3 in
      if Jstr.length g3 > 0 then g3
      else
        let g4 = Re.match_group_or_empty m 4 in
        if Jstr.length g4 > 0 then g4
        else
          let g5 = Re.match_group_or_empty m 5 in
          if Jstr.length g5 > 0 then g5 else Re.match_group_or_empty m 6

let em_strong (lex : lexer) (src : Jstr.t) (masked_src : Jstr.t) (prev_char : Jstr.t) :
    token option =
  let m = Re.exec_raw lex.rules.inline.em_strong_l_delim src in
  if not (Re.is_match m) then None
  else
    let has1 = Re.match_group_is_nonempty m 1 in
    let has2 = Re.match_group_is_nonempty m 2 in
    let has3 = Re.match_group_is_nonempty m 3 in
    let has4 = Re.match_group_is_nonempty m 4 in
    if not (has1 || has2 || has3 || has4) then None
    else if has4 && Re.test Rules.Other.unicode_alpha_numeric prev_char then None
    else if
      (not (has1 || has3))
      || Jstr.length prev_char = 0
      || Re.test lex.rules.inline.punctuation prev_char
    then (
      let m0 = Re.match_full m in
      let l_length = code_point_count m0 - 1 in
      let delim_char = Jstr.code_unit_at m0 0 in
      let mid_run = Jstr.length prev_char = 1 && Jstr.code_unit_at prev_char 0 = delim_char in
      let end_reg =
        if delim_char = 42 then lex.rules.inline.em_strong_r_delim_ast
        else lex.rules.inline.em_strong_r_delim_und
      in
      Re.set_last_index end_reg 0;
      let clipped = Jstr.slice_from masked_src (-(Jstr.length src) + l_length) in
      let rec scan delim_total mid_delim_total =
        let rm = Re.exec_raw end_reg clipped in
        if not (Re.is_match rm) then None
        else
          let r_delim = first_group_1_to_6 rm in
          let r_length = Jstr.length r_delim in
          if r_length = 0 then scan delim_total mid_delim_total
          else if Re.match_group_is_nonempty rm 3 || Re.match_group_is_nonempty rm 4 then
            scan (delim_total + r_length) mid_delim_total
          else
            let is_56 = Re.match_group_is_nonempty rm 5 || Re.match_group_is_nonempty rm 6 in
            if is_56 && l_length mod 3 <> 0 && (l_length + r_length) mod 3 = 0 then
              scan delim_total (mid_delim_total + r_length)
            else if is_56 && mid_run then None
            else
              let delim_total = delim_total - r_length in
              if delim_total > 0 then scan delim_total mid_delim_total
              else
                let adj = r_length + delim_total + mid_delim_total in
                let final_r = if r_length < adj then r_length else adj in
                let last_char_len = first_code_point_len (Re.match_full rm) in
                let raw =
                  Jstr.slice src 0 (l_length + Re.match_index rm + last_char_len + final_r)
                in
                let min_lr = if l_length < final_r then l_length else final_r in
                if min_lr mod 2 <> 0 then
                  let text = Jstr.slice raw 1 (-1) in
                  Some (Em { raw; text; tokens = lex.inline_tokens_fn text (Vec.create ()) })
                else
                  let text = Jstr.slice raw 2 (-2) in
                  Some (Strong { raw; text; tokens = lex.inline_tokens_fn text (Vec.create ()) })
      in
      scan l_length 0)
    else None

let codespan (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.inline.code src in
  if not (Re.is_match cap) then None
  else
    let raw = Re.match_full cap in
    let raw_text =
      Re.replace_str (Re.match_group_or_empty cap 2) Rules.Other.new_line_char_global s_space
    in
    let has_non_space = Re.test Rules.Other.non_space_char raw_text in
    let has_both_spaces =
      Re.test Rules.Other.starting_space_char raw_text
      && Re.test Rules.Other.ending_space_char raw_text
    in
    let text =
      if has_non_space && has_both_spaces then Jstr.substring raw_text 1 (Jstr.length raw_text - 1)
      else raw_text
    in
    Some (Codespan { raw; text })

let br (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.inline.br src in
  if not (Re.is_match cap) then None else Some (Br { raw = Re.match_full cap })

let del (lex : lexer) (src : Jstr.t) (masked_src : Jstr.t) (prev_char : Jstr.t) : token option =
  match (lex.rules.inline.del_l_delim, lex.rules.inline.del_r_delim) with
  | Some l_re, Some end_reg ->
      let m = Re.exec_raw l_re src in
      if not (Re.is_match m) then None
      else if
        (not (Re.match_group_is_nonempty m 1))
        || Jstr.length prev_char = 0
        || Re.test lex.rules.inline.punctuation prev_char
      then (
        let l_length = code_point_count (Re.match_full m) - 1 in
        Re.set_last_index end_reg 0;
        let clipped = Jstr.slice_from masked_src (-(Jstr.length src) + l_length) in
        let rec scan delim_total =
          let rm = Re.exec_raw end_reg clipped in
          if not (Re.is_match rm) then None
          else
            let r_delim = first_group_1_to_6 rm in
            let r_length = Jstr.length r_delim in
            if r_length <> l_length then scan delim_total
            else if Re.match_group_is_nonempty rm 3 || Re.match_group_is_nonempty rm 4 then
              scan (delim_total + r_length)
            else
              let delim_total = delim_total - r_length in
              if delim_total > 0 then scan delim_total
              else
                let adj = r_length + delim_total in
                let final_r = if r_length < adj then r_length else adj in
                let last_char_len = first_code_point_len (Re.match_full rm) in
                let raw =
                  Jstr.slice src 0 (l_length + Re.match_index rm + last_char_len + final_r)
                in
                let text = Jstr.slice raw l_length (-l_length) in
                Some (Del { raw; text; tokens = lex.inline_tokens_fn text (Vec.create ()) })
        in
        scan l_length)
      else None
  | _ -> None

let autolink (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.inline.autolink src in
  if not (Re.is_match cap) then None
  else
    let cap0 = Re.match_full cap in
    let text = Re.match_group_or_empty cap 1 in
    let is_email = Jstr.equal (Re.match_group_or_empty cap 2) s_at in
    let href = if is_email then Jstr.concat s_mailto text else text in
    let inner = Vec.create () in
    Vec.push inner
      (Text
         {
           t_raw = text;
           t_text = text;
           t_escaped = false;
           t_is_paragraph = false;
           t_tokens = None;
         });
    Some (Link { raw = cap0; text; href; title = None; autolink = true; tokens = inner })

let url (lex : lexer) (src : Jstr.t) : token option =
  match (lex.rules.inline.url, lex.rules.inline.backpedal) with
  | Some url_re, Some backpedal_re ->
      let cap = Re.exec_raw url_re src in
      if not (Re.is_match cap) then None
      else
        let is_email = Jstr.equal (Re.match_group_or_empty cap 2) s_at in
        let raw, text, href =
          if is_email then
            let m0 = Re.match_full cap in
            (m0, m0, Jstr.concat s_mailto m0)
          else
            let rec backpedal prev =
              let bm = Re.exec_raw backpedal_re prev in
              let next = if Re.is_match bm then Re.match_full bm else Jstr.empty in
              if Jstr.equal prev next then next else backpedal next
            in
            let final_url = backpedal (Re.match_full cap) in
            let is_www = Jstr.equal (Re.match_group_or_empty cap 1) s_www in
            let h = if is_www then Jstr.concat s_http final_url else final_url in
            (final_url, final_url, h)
        in
        let inner = Vec.create () in
        Vec.push inner
          (Text
             {
               t_raw = text;
               t_text = text;
               t_escaped = false;
               t_is_paragraph = false;
               t_tokens = None;
             });
        Some (Link { raw; text; href; title = None; autolink = true; tokens = inner })
  | _ -> None

let inline_text (lex : lexer) (src : Jstr.t) : token option =
  let cap = Re.exec_raw lex.rules.inline.text src in
  if not (Re.is_match cap) then None
  else
    let cap0 = Re.match_full cap in
    let escaped = lex.state.in_raw_block in
    let text = if escaped then cap0 else decode_numeric_character_references cap0 in
    Some
      (Text
         {
           t_raw = cap0;
           t_text = text;
           t_escaped = escaped;
           t_is_paragraph = false;
           t_tokens = None;
         })
