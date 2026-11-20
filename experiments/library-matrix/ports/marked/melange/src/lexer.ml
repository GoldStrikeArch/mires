(* Exact port of marked@18.1.1 src/Lexer.ts. *)

open Host
open Tokens
open Helpers

let s_nl = Jstr.of_string "\n"
let s_four_spaces = Jstr.of_string "    "
let s_lbracket = Jstr.of_string "["
let s_rbracket = Jstr.of_string "]"
let s_rbracket_lbracket = Jstr.of_string "]["
let s_rparen = Jstr.of_string ")"
let s_a = Jstr.of_string "a"
let s_plus = Jstr.of_string "+"
let s_underscore = Jstr.of_string "_"

(* Pre-wrapped stateless masking replacers (zero closure allocation per inline run). *)
let mask_punct_outside_code_replacer =
  Re.make_replacer3 (fun match0 code punct ->
      if Jstr.length punct > 0 then Jstr.repeat s_plus (Jstr.length match0)
      else
        let clen = Jstr.length code in
        if clen > 0 then
          let inner = Jstr.slice match0 clen (-clen) in
          Jstr.concat3 code (Re.replace_str inner Rules.Other.backtick_global s_plus) code
        else match0)

let mask_block_skip_replacer =
  Re.make_replacer3 (fun match0 _link context ->
      let offset = Jstr.length context in
      Jstr.concat4
        (Jstr.slice match0 0 offset)
        s_lbracket
        (Jstr.repeat s_a (Jstr.length match0 - offset - 2))
        s_rbracket)

let find_all_global (re : Re.t) (text : Jstr.t) : (Jstr.t * int) Vec.t =
  Re.set_last_index re 0;
  let out = Vec.create () in
  let rec loop () =
    let m = Re.exec_raw re text in
    if Re.is_match m then (
      let m0 = Re.match_full m in
      let idx = Re.match_index m in
      Vec.push out (m0, idx);
      if Jstr.length m0 = 0 then Re.set_last_index re (idx + 1);
      loop ())
  in
  loop ();
  out

let rec link_in_text (lex : lexer) (text : Jstr.t) : bool =
  if not (Jstr.includes text s_lbracket) then false
  else
    let link_rule = lex.rules.inline.link in
    let skip_matches = find_all_global lex.rules.inline.block_skip text in
    let n_skip = Vec.length skip_matches in
    let rec check_skip i =
      if i >= n_skip then false
      else
        let m0, idx = Vec.get skip_matches i in
        if Re.test link_rule m0 && (idx = 0 || Jstr.code_unit_at text (idx - 1) <> 33) then true
        else check_skip (i + 1)
    in
    if check_skip 0 then true
    else
      let ref_matches = find_all_global lex.rules.inline.reflink_search text in
      let n_ref = Vec.length ref_matches in
      let rec check_ref j =
        if j >= n_ref then false
        else
          let m0, _ = Vec.get ref_matches j in
          let ref_start = Jstr.last_index_of m0 s_lbracket in
          let is_img = Jstr.length m0 > 0 && Jstr.code_unit_at m0 0 = 33 in
          if
            (not is_img)
            && StrMap.has lex.links (normalize_label (Jstr.slice m0 (ref_start + 1) (-1)))
            && not (ref_start > 1 && link_in_text lex (Jstr.slice m0 1 (ref_start - 1)))
          then true
          else check_ref (j + 1)
      in
      check_ref 0

let infinite_loop_error (src : Jstr.t) =
  let code = if Jstr.length src > 0 then Jstr.code_unit_at src 0 else 0 in
  failwith ("Infinite loop on byte: " ^ string_of_int code)

let last_token_opt (tokens : token Vec.t) : token option =
  let n = Vec.length tokens in
  if n > 0 then Some (Vec.get tokens (n - 1)) else None

(* Pushes [tok] onto [tokens] and returns the rest of [src] after the token's raw text. *)
let push_token (tokens : token Vec.t) (src : Jstr.t) (tok : token) : Jstr.t =
  Vec.push tokens tok;
  Jstr.substring_from src (Jstr.length (token_raw tok))

let merge_into_last_text (lex : lexer) (last : text_token) (raw : Jstr.t) (text : Jstr.t)
    ~(pop_queue : bool) : unit =
  let sep = if Jstr.ends_with last.t_raw s_nl then Jstr.empty else s_nl in
  last.t_raw <- Jstr.concat3 last.t_raw sep raw;
  last.t_text <- Jstr.concat3 last.t_text s_nl text;
  if pop_queue then Vec.pop_unit lex.inline_queue;
  let qn = Vec.length lex.inline_queue in
  if qn > 0 then (Vec.get lex.inline_queue (qn - 1)).iq_src <- last.t_text

let block_tokens (lex : lexer) (src_in : Jstr.t) (tokens : token Vec.t)
    (last_paragraph_clipped_in : bool) : token Vec.t =
  let initial_src =
    if lex.options.pedantic then
      Re.replace_str
        (Re.replace_str src_in Rules.Other.tab_char_global s_four_spaces)
        Rules.Other.space_line Jstr.empty
    else src_in
  in
  (* Every [loop] call below is a direct self tail call, which Melange and js_of_ocaml both
     compile to a JS loop, so the stack stays flat however many tokens the source holds. *)
  let rec loop s prev_len last_clipped =
    let cur_len = Jstr.length s in
    if cur_len = 0 then ()
    else if cur_len >= prev_len then infinite_loop_error s
    else
      match Tokenizer.space lex s with
      | Some (Space sp as tok) ->
          if Jstr.length sp.raw = 1 then (
            match last_token_opt tokens with
            | Some last -> append_raw_nl last
            | None -> Vec.push tokens tok)
          else Vec.push tokens tok;
          loop (Jstr.substring_from s (Jstr.length sp.raw)) cur_len last_clipped
      | _ -> (
          match Tokenizer.code lex s with
          | Some (Code c as tok) ->
              (match last_token_opt tokens with
              | Some (Text last) -> merge_into_last_text lex last c.raw c.text ~pop_queue:false
              | _ -> Vec.push tokens tok);
              loop (Jstr.substring_from s (Jstr.length c.raw)) cur_len last_clipped
          | _ -> (
              match Tokenizer.fences lex s with
              | Some tok -> loop (push_token tokens s tok) cur_len last_clipped
              | None -> (
                  match Tokenizer.heading lex s with
                  | Some tok -> loop (push_token tokens s tok) cur_len last_clipped
                  | None -> (
                      match Tokenizer.hr lex s with
                      | Some tok -> loop (push_token tokens s tok) cur_len last_clipped
                      | None -> (
                          match Tokenizer.blockquote lex s with
                          | Some tok -> loop (push_token tokens s tok) cur_len last_clipped
                          | None -> (
                              match Tokenizer.list lex s with
                              | Some tok -> loop (push_token tokens s tok) cur_len last_clipped
                              | None -> (
                                  match Tokenizer.html lex s with
                                  | Some tok -> loop (push_token tokens s tok) cur_len last_clipped
                                  | None -> (
                                      match Tokenizer.def lex s with
                                      | Some (Def d as tok) ->
                                          (match last_token_opt tokens with
                                          | Some (Text last) ->
                                              merge_into_last_text lex last d.raw d.raw
                                                ~pop_queue:false
                                          | _ ->
                                              if not (StrMap.has lex.links d.tag) then (
                                                StrMap.set lex.links d.tag
                                                  { link_href = d.href; link_title = d.title };
                                                Vec.push tokens tok));
                                          loop (Jstr.substring_from s (Jstr.length d.raw)) cur_len
                                            last_clipped
                                      | _ -> (
                                          match Tokenizer.table lex s with
                                          | Some tok ->
                                              loop (push_token tokens s tok) cur_len last_clipped
                                          | None -> (
                                              match Tokenizer.lheading lex s with
                                              | Some tok ->
                                                  loop (push_token tokens s tok) cur_len last_clipped
                                              | None ->
                                                  let para =
                                                    if lex.state.top then Tokenizer.paragraph lex s
                                                    else None
                                                  in
                                                  (match para with
                                                  | Some (Text p as tok) ->
                                                      (match last_token_opt tokens with
                                                      | Some (Text last)
                                                        when last_clipped && last.t_is_paragraph ->
                                                          merge_into_last_text lex last p.t_raw
                                                            p.t_text ~pop_queue:true
                                                      | _ -> Vec.push tokens tok);
                                                      loop
                                                        (Jstr.substring_from s (Jstr.length p.t_raw))
                                                        cur_len false
                                                  | _ -> (
                                                      match Tokenizer.block_text lex s with
                                                      | Some (Text t as tok) ->
                                                          (match last_token_opt tokens with
                                                          | Some (Text last)
                                                            when not last.t_is_paragraph ->
                                                              merge_into_last_text lex last t.t_raw
                                                                t.t_text ~pop_queue:true
                                                          | _ -> Vec.push tokens tok);
                                                          loop
                                                            (Jstr.substring_from s
                                                               (Jstr.length t.t_raw))
                                                            cur_len last_clipped
                                                      | _ -> infinite_loop_error s))))))))))))
  in
  loop initial_src (Jstr.length initial_src + 1) last_paragraph_clipped_in;
  lex.state.top <- true;
  tokens

let inline_tokens_inner (lex : lexer) (src_in : Jstr.t) (tokens : token Vec.t) : token Vec.t =
  let masked =
    if StrMap.size lex.links > 0 && Jstr.includes src_in s_lbracket then
      let reflink_search = lex.rules.inline.reflink_search in
      let rec mask_reflink (match0 : Jstr.t) : Jstr.t =
        let ref_start = Jstr.last_index_of match0 s_lbracket in
        if not (StrMap.has lex.links (normalize_label (Jstr.slice match0 (ref_start + 1) (-1))))
        then match0
        else if
          ref_start > 1 && (Jstr.length match0 = 0 || Jstr.code_unit_at match0 0 <> 33)
        then
          let text = Jstr.slice match0 1 (ref_start - 1) in
          if link_in_text lex text then
            Jstr.concat4
              (Jstr.concat s_lbracket (Re.replace_fn1 text reflink_search mask_reflink))
              s_rbracket_lbracket
              (Jstr.repeat s_a (Jstr.length match0 - ref_start - 2))
              s_rbracket
          else Jstr.concat3 s_lbracket (Jstr.repeat s_a (Jstr.length match0 - 2)) s_rbracket
        else Jstr.concat3 s_lbracket (Jstr.repeat s_a (Jstr.length match0 - 2)) s_rbracket
      in
      Re.replace_fn1 src_in reflink_search mask_reflink
    else src_in
  in
  let masked =
    Re.replace_with3 masked lex.rules.inline.any_punctuation_outside_code
      mask_punct_outside_code_replacer
  in
  let masked_src = Re.replace_with3 masked lex.rules.inline.block_skip mask_block_skip_replacer in
  (* As in [block_tokens], every [loop] call is a direct self tail call. *)
  let rec loop s prev_len prev_char keep_prev =
    let cur_len = Jstr.length s in
    if cur_len = 0 then ()
    else if cur_len >= prev_len then infinite_loop_error s
    else
      let prev_char = if keep_prev then prev_char else Jstr.empty in
      match Tokenizer.escape lex s with
      | Some tok -> loop (push_token tokens s tok) cur_len prev_char false
      | None -> (
          match Tokenizer.tag lex s with
          | Some tok -> loop (push_token tokens s tok) cur_len prev_char false
          | None -> (
              match Tokenizer.link lex s with
              | Some tok -> loop (push_token tokens s tok) cur_len prev_char false
              | None -> (
                  match Tokenizer.reflink lex s with
                  | Some tok ->
                      (match (tok, last_token_opt tokens) with
                      | Text t, Some (Text last) ->
                          last.t_raw <- Jstr.concat last.t_raw t.t_raw;
                          last.t_text <- Jstr.concat last.t_text t.t_text
                      | _ -> Vec.push tokens tok);
                      loop (Jstr.substring_from s (Jstr.length (token_raw tok))) cur_len prev_char
                        false
                  | None -> (
                      match Tokenizer.em_strong lex s masked_src prev_char with
                      | Some tok -> loop (push_token tokens s tok) cur_len prev_char false
                      | None -> (
                          match Tokenizer.codespan lex s with
                          | Some tok -> loop (push_token tokens s tok) cur_len prev_char false
                          | None -> (
                              match Tokenizer.br lex s with
                              | Some tok -> loop (push_token tokens s tok) cur_len prev_char false
                              | None -> (
                                  match Tokenizer.del lex s masked_src prev_char with
                                  | Some tok ->
                                      loop (push_token tokens s tok) cur_len prev_char false
                                  | None -> (
                                      match Tokenizer.autolink lex s with
                                      | Some tok ->
                                          loop (push_token tokens s tok) cur_len prev_char false
                                      | None ->
                                          let url_tok =
                                            if not lex.state.in_link then Tokenizer.url lex s
                                            else None
                                          in
                                          (match url_tok with
                                          | Some tok ->
                                              loop (push_token tokens s tok) cur_len prev_char false
                                          | None -> (
                                              match Tokenizer.inline_text lex s with
                                              | Some (Text t as tok) ->
                                                  let last_ch = Jstr.slice_from t.t_raw (-1) in
                                                  let next_prev =
                                                    if Jstr.equal last_ch s_underscore then
                                                      prev_char
                                                    else last_ch
                                                  in
                                                  (match last_token_opt tokens with
                                                  | Some (Text last) ->
                                                      last.t_raw <- Jstr.concat last.t_raw t.t_raw;
                                                      last.t_text <-
                                                        Jstr.concat last.t_text t.t_text
                                                  | _ -> Vec.push tokens tok);
                                                  loop
                                                    (Jstr.substring_from s (Jstr.length t.t_raw))
                                                    cur_len next_prev true
                                              | _ -> infinite_loop_error s))))))))))
  in
  loop src_in (Jstr.length src_in + 1) Jstr.empty false;
  tokens

let inline_tokens (lex : lexer) (src : Jstr.t) (tokens : token Vec.t) : token Vec.t =
  let prev = lex.state.link_paren_possible in
  lex.state.link_paren_possible <- prev && Jstr.includes src s_rparen;
  match inline_tokens_inner lex src tokens with
  | res ->
      lex.state.link_paren_possible <- prev;
      res
  | exception exn ->
      lex.state.link_paren_possible <- prev;
      raise exn

let create (options : options) : lexer =
  let rules = Rules.select ~gfm:options.gfm ~breaks:options.breaks ~pedantic:options.pedantic in
  let dummy_block _ toks _ = toks in
  let dummy_inline _ toks = toks in
  let lex =
    {
      options;
      rules;
      state =
        {
          in_link = false;
          in_raw_block = false;
          link_emitted = false;
          link_paren_possible = true;
          top = true;
        };
      tokens = Vec.create ();
      links = StrMap.create ();
      inline_queue = Vec.create ();
      block_tokens_fn = dummy_block;
      inline_tokens_fn = dummy_inline;
    }
  in
  lex.block_tokens_fn <- (fun src toks clipped -> block_tokens lex src toks clipped);
  lex.inline_tokens_fn <- (fun src toks -> inline_tokens lex src toks);
  lex

let lex (src_in : Jstr.t) (options : options) : token Vec.t =
  let lexer = create options in
  let src = Re.replace_str src_in Rules.Other.carriage_return s_nl in
  ignore (block_tokens lexer src lexer.tokens false);
  let i = ref 0 in
  while !i < Vec.length lexer.inline_queue do
    let entry = Vec.get lexer.inline_queue !i in
    ignore (inline_tokens lexer entry.iq_src entry.iq_tokens);
    incr i
  done;
  lexer.tokens

let lex_inline (src : Jstr.t) (options : options) : token Vec.t =
  let lexer = create options in
  inline_tokens lexer src (Vec.create ())
