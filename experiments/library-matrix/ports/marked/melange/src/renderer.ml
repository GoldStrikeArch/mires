(* Exact port of marked@18.1.1 src/Renderer.ts and src/TextRenderer.ts. *)

open Host
open Tokens
open Helpers

let s_nl = Jstr.of_string "\n"
let s_pre_code = Jstr.of_string "<pre><code>"
let s_pre_code_lang = Jstr.of_string "<pre><code class=\"language-"
let s_gt_quote = Jstr.of_string "\">"
let s_code_pre_end = Jstr.of_string "</code></pre>\n"
let s_bq_open = Jstr.of_string "<blockquote>\n"
let s_bq_close = Jstr.of_string "</blockquote>\n"
let s_h_open = Jstr.of_string "<h"
let s_gt = Jstr.of_string ">"
let s_h_close_open = Jstr.of_string "</h"
let s_gt_nl = Jstr.of_string ">\n"
let s_hr = Jstr.of_string "<hr>\n"
let s_ol = Jstr.of_string "ol"
let s_ul = Jstr.of_string "ul"
let s_lt = Jstr.of_string "<"
let s_lt_slash = Jstr.of_string "</"
let s_start_attr = Jstr.of_string " start=\""
let s_quote = Jstr.of_string "\""
let s_li_open = Jstr.of_string "<li>"
let s_li_close = Jstr.of_string "</li>\n"
let s_cb_checked = Jstr.of_string "<input checked=\"\" disabled=\"\" type=\"checkbox\"> "
let s_cb_unchecked = Jstr.of_string "<input disabled=\"\" type=\"checkbox\"> "
let s_p_open = Jstr.of_string "<p>"
let s_p_close = Jstr.of_string "</p>\n"
let s_table_open = Jstr.of_string "<table>\n<thead>\n"
let s_thead_close = Jstr.of_string "</thead>\n"
let s_tbody_open = Jstr.of_string "<tbody>"
let s_tbody_close = Jstr.of_string "</tbody>"
let s_table_close = Jstr.of_string "</table>\n"
let s_tr_open = Jstr.of_string "<tr>\n"
let s_tr_close = Jstr.of_string "</tr>\n"
let s_th = Jstr.of_string "th"
let s_td = Jstr.of_string "td"
let s_align_left = Jstr.of_string " align=\"left\">"
let s_align_center = Jstr.of_string " align=\"center\">"
let s_align_right = Jstr.of_string " align=\"right\">"
let s_strong_open = Jstr.of_string "<strong>"
let s_strong_close = Jstr.of_string "</strong>"
let s_em_open = Jstr.of_string "<em>"
let s_em_close = Jstr.of_string "</em>"
let s_code_open = Jstr.of_string "<code>"
let s_code_close = Jstr.of_string "</code>"
let s_br = Jstr.of_string "<br>"
let s_del_open = Jstr.of_string "<del>"
let s_del_close = Jstr.of_string "</del>"
let s_a_open = Jstr.of_string "<a href=\""
let s_title_attr = Jstr.of_string " title=\""
let s_a_close = Jstr.of_string "</a>"
let s_img_open = Jstr.of_string "<img src=\""
let s_alt_attr = Jstr.of_string "\" alt=\""

module Text_renderer = struct
  let render_token (tok : token) : Jstr.t =
    match tok with
    | Strong { text; _ } -> text
    | Em { text; _ } -> text
    | Codespan { text; _ } -> text
    | Del { text; _ } -> text
    | Html { text; _ } -> text
    | Text { t_text; _ } -> t_text
    | Escape { text; _ } -> text
    | Link { text; _ } -> text
    | Image { text; _ } -> text
    | Br _ -> Jstr.empty
    | Checkbox { raw; _ } -> raw
    | _ -> invalid_arg "marked port: unexpected block token in TextRenderer"
end

let space () : Jstr.t = Jstr.empty

let code ~(text : Jstr.t) ~(lang : Jstr.t option) ~(escaped : bool) : Jstr.t =
  let lang_str =
    match lang with
    | Some l when Jstr.length l > 0 ->
        let m = Re.exec_raw Rules.Other.not_space_start l in
        if Re.is_match m then Re.match_full m else Jstr.empty
    | _ -> Jstr.empty
  in
  let body_code =
    if Jstr.length text > 0 then
      Jstr.concat (Re.replace_str text Rules.Other.ending_newline Jstr.empty) s_nl
    else Jstr.empty
  in
  let body = if escaped then body_code else escape_html_entities body_code true in
  if Jstr.length lang_str = 0 then Jstr.concat3 s_pre_code body s_code_pre_end
  else
    Jstr.concat4 s_pre_code_lang
      (escape_html_entities lang_str false)
      (Jstr.concat s_gt_quote body)
      s_code_pre_end

let blockquote ~(parse : token Vec.t -> Jstr.t) (tokens : token Vec.t) : Jstr.t =
  Jstr.concat3 s_bq_open (parse tokens) s_bq_close

let html (text : Jstr.t) : Jstr.t = text
let def () : Jstr.t = Jstr.empty

let heading ~(parse_inline : token Vec.t -> Jstr.t) ~(depth : int) (tokens : token Vec.t) : Jstr.t =
  let d = Jstr.of_int depth in
  Jstr.concat4
    (Jstr.concat3 s_h_open d s_gt)
    (parse_inline tokens)
    (Jstr.concat s_h_close_open d)
    s_gt_nl

let hr () : Jstr.t = s_hr

let listitem ~(parse : token Vec.t -> Jstr.t) (item : list_item) : Jstr.t =
  Jstr.concat3 s_li_open (parse item.item_tokens) s_li_close

let list ~(parse : token Vec.t -> Jstr.t) ~(ordered : bool) ~(start : int) (items : list_item Vec.t)
    : Jstr.t =
  let n = Vec.length items in
  let parts = Vec.create () in
  for i = 0 to n - 1 do
    Vec.push parts (listitem ~parse (Vec.get items i))
  done;
  let body = Jstr.join parts Jstr.empty in
  let tag = if ordered then s_ol else s_ul in
  let start_attr =
    if ordered && start <> 1 then Jstr.concat3 s_start_attr (Jstr.of_int start) s_quote
    else Jstr.empty
  in
  Jstr.concat4
    (Jstr.concat4 s_lt tag start_attr s_gt_nl)
    body
    (Jstr.concat s_lt_slash tag)
    s_gt_nl

let checkbox (checked : bool) : Jstr.t = if checked then s_cb_checked else s_cb_unchecked

let paragraph ~(parse_inline : token Vec.t -> Jstr.t) (tokens : token Vec.t) : Jstr.t =
  Jstr.concat3 s_p_open (parse_inline tokens) s_p_close

let tablerow (text : Jstr.t) : Jstr.t = Jstr.concat3 s_tr_open text s_tr_close

let tablecell ~(parse_inline : token Vec.t -> Jstr.t) (cell : table_cell) : Jstr.t =
  let content = parse_inline cell.cell_tokens in
  let tag = if cell.cell_header then s_th else s_td in
  let open_tail =
    match cell.cell_align with
    | Align_none -> s_gt
    | Align_left -> s_align_left
    | Align_center -> s_align_center
    | Align_right -> s_align_right
  in
  Jstr.concat4 (Jstr.concat3 s_lt tag open_tail) content (Jstr.concat s_lt_slash tag) s_gt_nl

let table ~(parse_inline : token Vec.t -> Jstr.t) ~(header : table_cell Vec.t)
    ~(rows : table_cell Vec.t Vec.t) : Jstr.t =
  let h_cells = Vec.create () in
  let n_cols = Vec.length header in
  for i = 0 to n_cols - 1 do
    Vec.push h_cells (tablecell ~parse_inline (Vec.get header i))
  done;
  let header_html = tablerow (Jstr.join h_cells Jstr.empty) in
  let n_rows = Vec.length rows in
  let body_html =
    if n_rows = 0 then Jstr.empty
    else
      let row_parts = Vec.create () in
      for r = 0 to n_rows - 1 do
        let row = Vec.get rows r in
        let r_cells = Vec.create () in
        let nr = Vec.length row in
        for c = 0 to nr - 1 do
          Vec.push r_cells (tablecell ~parse_inline (Vec.get row c))
        done;
        Vec.push row_parts (tablerow (Jstr.join r_cells Jstr.empty))
      done;
      Jstr.concat3 s_tbody_open (Jstr.join row_parts Jstr.empty) s_tbody_close
  in
  Jstr.concat4 s_table_open header_html (Jstr.concat s_thead_close body_html) s_table_close

let strong ~(parse_inline : token Vec.t -> Jstr.t) (tokens : token Vec.t) : Jstr.t =
  Jstr.concat3 s_strong_open (parse_inline tokens) s_strong_close

let em ~(parse_inline : token Vec.t -> Jstr.t) (tokens : token Vec.t) : Jstr.t =
  Jstr.concat3 s_em_open (parse_inline tokens) s_em_close

let codespan (text : Jstr.t) : Jstr.t =
  Jstr.concat3 s_code_open (escape_html_entities text true) s_code_close

let br () : Jstr.t = s_br

let del ~(parse_inline : token Vec.t -> Jstr.t) (tokens : token Vec.t) : Jstr.t =
  Jstr.concat3 s_del_open (parse_inline tokens) s_del_close

let link ~(parse_inline : token Vec.t -> Jstr.t) ~(href : Jstr.t) ~(title : Jstr.t option)
    ~(text : Jstr.t) ~(tokens : token Vec.t) ~(autolink : bool) : Jstr.t =
  let parsed_text = if autolink then escape_html_entities text true else parse_inline tokens in
  match clean_url href with
  | None -> parsed_text
  | Some clean_href ->
      let href_esc = escape_html_entities clean_href autolink in
      let open_tag =
        match title with
        | Some t when Jstr.length t > 0 ->
            Jstr.concat4
              (Jstr.concat3 s_a_open href_esc s_quote)
              s_title_attr
              (escape_html_entities t false)
              s_gt_quote
        | _ -> Jstr.concat3 s_a_open href_esc s_gt_quote
      in
      Jstr.concat3 open_tag parsed_text s_a_close

let image ~(parse_inline_text : token Vec.t -> Jstr.t) ~(href : Jstr.t) ~(title : Jstr.t option)
    ~(text : Jstr.t) ~(tokens : token Vec.t option) : Jstr.t =
  let alt_text = match tokens with Some ts -> parse_inline_text ts | None -> text in
  match clean_url href with
  | None -> escape_html_entities alt_text false
  | Some clean_href ->
      let base =
        Jstr.concat4 s_img_open
          (escape_html_entities clean_href false)
          s_alt_attr
          (escape_html_entities alt_text false)
      in
      (match title with
      | Some t when Jstr.length t > 0 ->
          Jstr.concat4 (Jstr.concat base s_quote) s_title_attr (escape_html_entities t false) s_gt_quote
      | _ -> Jstr.concat base s_gt_quote)

let text ~(parse_inline : token Vec.t -> Jstr.t) (t : text_token) : Jstr.t =
  match t.t_tokens with
  | Some ts -> parse_inline ts
  | None -> if t.t_escaped then t.t_text else escape_html_entities t.t_text false

let escape_text (text : Jstr.t) : Jstr.t = escape_html_entities text false
