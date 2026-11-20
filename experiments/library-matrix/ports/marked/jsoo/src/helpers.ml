(* Exact port of marked@18.1.1 src/helpers.ts.

   Uses pure Host.Jstr / Host.Re / Host.Vec operations on native JS UTF-16 strings with
   pre-wrapped stateless regex replacers (zero closure allocation per call) and zero
   polymorphic comparisons. *)

open Host

let s_amp = Jstr.of_string "&amp;"
let s_lt = Jstr.of_string "&lt;"
let s_gt = Jstr.of_string "&gt;"
let s_quot = Jstr.of_string "&quot;"
let s_apos = Jstr.of_string "&#39;"
let s_percent = Jstr.of_string "%"
let s_pipe = Jstr.of_string "|"
let s_space_pipe = Jstr.of_string " |"
let s_nl = Jstr.of_string "\n"
let s_tab = Jstr.of_string "\t"
let s_spaces_1 = Jstr.of_string " "
let s_spaces_2 = Jstr.of_string "  "
let s_spaces_3 = Jstr.of_string "   "
let s_spaces_4 = Jstr.of_string "    "
let s_replacement_char = Jstr.from_code_point 0xFFFD

let escape_replacer =
  Re.make_replacer1 (fun ch ->
      match Jstr.code_unit_at ch 0 with
      | 38 -> s_amp
      | 60 -> s_lt
      | 62 -> s_gt
      | 34 -> s_quot
      | 39 -> s_apos
      | _ -> ch)

let escape_html_entities (html : Jstr.t) (encode : bool) : Jstr.t =
  if encode then
    if Re.test Rules.Other.escape_test html then
      Re.replace_with1 html Rules.Other.escape_replace escape_replacer
    else html
  else if Re.test Rules.Other.escape_test_no_encode html then
    Re.replace_with1 html Rules.Other.escape_replace_no_encode escape_replacer
  else html

let decode_ncr_replacer =
  Re.make_replacer3 (fun _ref dec hex ->
      let dec_len = Jstr.length dec in
      let hex_len = Jstr.length hex in
      if dec_len = 0 && hex_len = 0 then s_amp
      else
        let code = if dec_len > 0 then Jstr.parse_int dec 10 else Jstr.parse_int hex 16 in
        if code = 0 || code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff) then
          s_replacement_char
        else if code = 38 then s_amp
        else Jstr.from_code_point code)

let decode_numeric_character_references (text : Jstr.t) : Jstr.t =
  if not (Re.test Rules.Other.numeric_character_reference text) then text
  else Re.replace_with3 text Rules.Other.numeric_character_reference_or_ampersand decode_ncr_replacer

let clean_url (href : Jstr.t) : Jstr.t option =
  match Jstr.encode_uri href with
  | None -> None
  | Some encoded -> Some (Re.replace_str encoded Rules.Other.percent_decode s_percent)

let split_cells (table_row : Jstr.t) (count : int) : Jstr.t Vec.t =
  let len = Jstr.length table_row in
  let first_pipe = Jstr.index_of table_row s_pipe in
  let row =
    if first_pipe < 0 then table_row
    else
      let parts = Vec.create () in
      let last = ref 0 in
      let pos = ref first_pipe in
      while !pos >= 0 do
        let p = !pos in
        let escaped = ref false in
        let curr = ref (p - 1) in
        while !curr >= 0 && Jstr.code_unit_at table_row !curr = 92 do
          escaped := not !escaped;
          decr curr
        done;
        if not !escaped then (
          if p > !last then Vec.push parts (Jstr.slice table_row !last p);
          Vec.push parts s_space_pipe;
          last := p + 1);
        pos := if p + 1 < len then Jstr.index_of_from table_row s_pipe (p + 1) else -1
      done;
      if !last = 0 then table_row
      else (
        if !last < len then Vec.push parts (Jstr.slice_from table_row !last);
        Jstr.join parts Jstr.empty)
  in
  let cells = Re.split row Rules.Other.split_pipe in
  let start = ref 0 in
  let stop = ref (Vec.length cells) in
  if !start < !stop && Jstr.length (Jstr.trim (Vec.get cells !start)) = 0 then incr start;
  if !stop > !start && Jstr.length (Jstr.trim (Vec.get cells (!stop - 1))) = 0 then decr stop;
  let avail = !stop - !start in
  let final_len = if count > 0 then count else avail in
  let out = Vec.create () in
  for i = 0 to final_len - 1 do
    if i < avail then
      Vec.push out
        (Re.replace_str (Jstr.trim (Vec.get cells (!start + i))) Rules.Other.slash_pipe s_pipe)
    else Vec.push out Jstr.empty
  done;
  out

let rtrim (str : Jstr.t) (c_code : int) (invert : bool) : Jstr.t =
  let l = Jstr.length str in
  if l = 0 then Jstr.empty
  else
    let suff_len = ref 0 in
    let cont = ref true in
    while !cont && !suff_len < l do
      let curr = Jstr.code_unit_at str (l - !suff_len - 1) in
      if (curr = c_code && not invert) || (curr <> c_code && invert) then incr suff_len
      else cont := false
    done;
    if !suff_len = 0 then str else Jstr.slice str 0 (l - !suff_len)

let trim_trailing_blank_lines (str : Jstr.t) : Jstr.t =
  let lines = Jstr.split_str str s_nl in
  let n = Vec.length lines in
  let end_idx = ref (n - 1) in
  while !end_idx >= 0 && Re.test Rules.Other.blank_line (Vec.get lines !end_idx) do
    decr end_idx
  done;
  if n - !end_idx <= 2 then str else Jstr.join (Vec.slice lines 0 (!end_idx + 1)) s_nl

let normalize_label (label : Jstr.t) : Jstr.t =
  Jstr.to_lower (Jstr.to_upper (Jstr.to_lower (Jstr.trim label)))

let find_closing_bracket (str : Jstr.t) (open_s : Jstr.t) (close_s : Jstr.t) (open_c : int)
    (close_c : int) : int =
  if Jstr.index_of str open_s = -1 && Jstr.index_of str close_s = -1 then -1
  else
    let len = Jstr.length str in
    let level = ref 0 in
    let i = ref 0 in
    let found = ref (-1) in
    while !i < len && !found = -1 do
      let c = Jstr.code_unit_at str !i in
      if c = 92 then i := !i + 2
      else if c = open_c then (
        incr level;
        incr i)
      else if c = close_c then (
        decr level;
        if !level < 0 then found := !i else incr i)
      else incr i
    done;
    if !found <> -1 then !found else if !level > 0 then -2 else -1

let spaces_for (n : int) : Jstr.t =
  match n with 1 -> s_spaces_1 | 2 -> s_spaces_2 | 3 -> s_spaces_3 | _ -> s_spaces_4

(* Matches JS `for (const char of line)`: surrogate pairs advance column by 1, not 2. *)
let expand_tabs (line : Jstr.t) (indent : int) : Jstr.t =
  if not (Jstr.includes line s_tab) then line
  else
    let len = Jstr.length line in
    let parts = Vec.create () in
    let col = ref indent in
    let seg_start = ref 0 in
    let i = ref 0 in
    while !i < len do
      let c = Jstr.code_unit_at line !i in
      if c = 9 then (
        if !i > !seg_start then Vec.push parts (Jstr.slice line !seg_start !i);
        let added = 4 - (!col mod 4) in
        Vec.push parts (spaces_for added);
        col := !col + added;
        incr i;
        seg_start := !i)
      else if c >= 0xd800 && c <= 0xdbff && !i + 1 < len then (
        let c2 = Jstr.code_unit_at line (!i + 1) in
        if c2 >= 0xdc00 && c2 <= 0xdfff then i := !i + 2 else incr i;
        incr col)
      else (
        incr i;
        incr col)
    done;
    if !seg_start < len then Vec.push parts (Jstr.slice_from line !seg_start);
    Jstr.join parts Jstr.empty

let expand_tabs_zero_replacer = Re.make_replacer1 (fun ws -> expand_tabs ws 0)

let code_point_count (s : Jstr.t) : int =
  let len = Jstr.length s in
  let count = ref 0 in
  let i = ref 0 in
  while !i < len do
    let c = Jstr.code_unit_at s !i in
    if c >= 0xd800 && c <= 0xdbff && !i + 1 < len then
      let c2 = Jstr.code_unit_at s (!i + 1) in
      if c2 >= 0xdc00 && c2 <= 0xdfff then i := !i + 2 else incr i
    else incr i;
    incr count
  done;
  !count

let first_code_point_len (s : Jstr.t) : int =
  let len = Jstr.length s in
  if len >= 2 then
    let c0 = Jstr.code_unit_at s 0 in
    if c0 >= 0xd800 && c0 <= 0xdbff then
      let c1 = Jstr.code_unit_at s 1 in
      if c1 >= 0xdc00 && c1 <= 0xdfff then 2 else 1
    else 1
  else 1

let first_line (s : Jstr.t) : Jstr.t =
  let i = Jstr.index_of s s_nl in
  if i < 0 then s else Jstr.slice s 0 i
