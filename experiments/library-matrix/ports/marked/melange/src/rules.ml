(* Exact port of marked@18.1.1 src/rules.ts.

   All regexes are compiled once at module load as immutable values using the host JS RegExp
   engine (matching marked@18.1.1's own module-level grammar tables), preserving exact UTF-16
   indices, u-flag Unicode property escapes, lookbehinds, and named backreferences. *)

open Host

let caret_re = Re.of_string {re|(^|[^\[])\^|re} "g"
let dollar1 = Jstr.of_string "$1"

let strip_caret (s : Jstr.t) : Jstr.t = Re.replace_str s caret_re dollar1
let edit_str (src : string) : Jstr.t = Jstr.of_string src
let edit_re (re : Re.t) : Jstr.t = Re.source re

let repl_s (name : string) (val_ : string) (src : Jstr.t) : Jstr.t =
  Jstr.replace_str_lit src (Jstr.of_string name) (strip_caret (Jstr.of_string val_))

let repl_s_re (name : string) (val_ : Re.t) (src : Jstr.t) : Jstr.t =
  Jstr.replace_str_lit src (Jstr.of_string name) (strip_caret (Re.source val_))

let repl_r (name : Re.t) (val_ : string) (src : Jstr.t) : Jstr.t =
  Re.replace_str src name (strip_caret (Jstr.of_string val_))

let repl_r_re (name : Re.t) (val_ : Re.t) (src : Jstr.t) : Jstr.t =
  Re.replace_str src name (strip_caret (Re.source val_))

let to_re (flags : string) (src : Jstr.t) : Re.t = Re.create src (Jstr.of_string flags)

let supports_lookbehind : bool =
  try
    ignore (Re.of_string "(?<=1)(?<!1)" "");
    true
  with _ -> false

let _tag =
  "address|article|aside|base|basefont|blockquote|body|caption"
  ^ "|center|col|colgroup|dd|details|dialog|dir|div|dl|dt|fieldset|figcaption"
  ^ "|figure|footer|form|frame|frameset|h[1-6]|head|header|hr|html|iframe"
  ^ "|legend|li|link|main|menu|menuitem|meta|nav|noframes|ol|optgroup|option"
  ^ "|p|param|search|section|summary|table|tbody|td|tfoot|th|thead|title"
  ^ "|tr|track|ul"

let make_indent_table (f : int -> Re.t) : Re.t array = [| f 0; f 1; f 2; f 3 |]

let lookup_indent (tbl : Re.t array) (indent : int) : Re.t =
  let idx = indent - 1 in
  let clamped = if idx < 0 then 0 else if idx > 3 then 3 else idx in
  tbl.(clamped)

let s_list_item_pre = Jstr.of_string {re|^( {0,3}|re}
let s_list_item_post = Jstr.of_string {re|)((?:[\t ][^\n]*)?(?:\n|$))|re}

let make_list_item_regex (bull : Jstr.t) : Re.t =
  Re.create (Jstr.concat3 s_list_item_pre bull s_list_item_post) Jstr.empty

let next_bullet_table =
  make_indent_table (fun n ->
      let s = string_of_int n in
      Re.of_string ({re|^ {0,|re} ^ s ^ {re|}(?:[*+-]|\d{1,9}[.)])((?:[ \t][^\n]*)?(?:\n|$))|re}) "")

let hr_indent_table =
  make_indent_table (fun n ->
      let s = string_of_int n in
      Re.of_string
        ({re|^ {0,|re} ^ s ^ {re|}((?:-[ \t]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)|re})
        "")

let fences_begin_table =
  make_indent_table (fun n ->
      let s = string_of_int n in
      Re.of_string ({re|^ {0,|re} ^ s ^ {re|}(?:```|~~~)|re}) "")

let heading_begin_table =
  make_indent_table (fun n ->
      let s = string_of_int n in
      Re.of_string ({re|^ {0,|re} ^ s ^ {re|}#|re}) "")

let html_begin_table =
  make_indent_table (fun n ->
      let s = string_of_int n in
      Re.of_string
        ({re|^ {0,|re} ^ s ^ {re|}(?:</?(?:|re} ^ _tag
       ^ {re|})(?: +|$|/?>)|<(?:(?:script|pre|style|textarea)(?:[\s>]|$)|!--)|<\?|<![A-Za-z]|<!\[CDATA\[)|re})
        "i")

let blockquote_begin_table =
  make_indent_table (fun n ->
      let s = string_of_int n in
      Re.of_string ({re|^ {0,|re} ^ s ^ {re|}>|re}) "")

module Other = struct
  let code_remove_indent = Re.of_string {re|^(?: {0,3}\t| {1,4})|re} "gm"
  let output_link_replace = Re.of_string {re|\\([\[\]])|re} "g"
  let indent_code_compensation = Re.of_string {re|^(\s+)(?:```)|re} ""
  let beginning_space = Re.of_string {re|^\s+|re} ""
  let ending_hash = Re.of_string {re|#$|re} ""
  let starting_space_char = Re.of_string {re|^ |re} ""
  let ending_space_char = Re.of_string {re| $|re} ""
  let ending_space_tab_char = Re.of_string {re|[ \t]$|re} ""
  let non_space_char = Re.of_string {re|[^ ]|re} ""
  let new_line_char_global = Re.of_string {re|\n|re} "g"
  let tab_char_global = Re.of_string {re|\t|re} "g"
  let leading_space_tab = Re.of_string {re|^[ \t]+|re} ""
  let multiple_space_global = Re.of_string {re|\s+|re} "g"
  let blank_line = Re.of_string {re|^[ \t]*$|re} ""
  let double_blank_line = Re.of_string {re|\n[ \t]*\n[ \t]*$|re} ""
  let blockquote_start = Re.of_string {re|^ {0,3}>|re} ""
  let blockquote_setext_replace = Re.of_string {re|\n {0,3}((?:=+|-+) *)(?=\n|$)|re} "g"
  let blockquote_setext_replace2 = Re.of_string {re|^ {0,3}>[ \t]?|re} "gm"
  let leading_newline = Re.of_string {re|^\n|re} ""
  let backtick_global = Re.of_string {re|`|re} "g"
  let list_replace_nesting = Re.of_string {re|^ {1,4}(?=( {4})*[^ ])|re} "g"
  let list_is_task = Re.of_string {re|^\[[ xX]\] +\S|re} ""
  let list_replace_task = Re.of_string {re|^\[[ xX]\] +|re} ""
  let list_task_checkbox = Re.of_string {re|\[[ xX]\]|re} ""
  let any_line = Re.of_string {re|\n.*\n|re} ""
  let href_brackets = Re.of_string {re|^<(.*)>$|re} ""
  let table_delimiter = Re.of_string {re|[:|]|re} ""
  let table_align_chars = Re.of_string {re|^\||\| *$|re} "g"
  let table_row_blank_line = Re.of_string {re|\n[ \t]*$|re} ""
  let table_align_right = Re.of_string {re|^ *-+: *$|re} ""
  let table_align_center = Re.of_string {re|^ *:-+: *$|re} ""
  let table_align_left = Re.of_string {re|^ *:-+ *$|re} ""
  let start_a_tag = Re.of_string {re|^<a |re} "i"
  let end_a_tag = Re.of_string {re|^<\/a>|re} "i"
  let start_pre_script_tag = Re.of_string {re|^<(pre|code|kbd|script)(\s|>)|re} "i"
  let end_pre_script_tag = Re.of_string {re|^<\/(pre|code|kbd|script)(\s|>)|re} "i"
  let start_angle_bracket = Re.of_string {re|^<|re} ""
  let end_angle_bracket = Re.of_string {re|>$|re} ""
  let pedantic_href_title = Re.of_string {re|^([^'"]*[^\s])\s+(['"])(.*)\2|re} ""
  let unicode_alpha_numeric = Re.of_string {re|[\p{L}\p{N}]|re} "u"
  let numeric_character_reference = Re.of_string {re|&#(?:(\d{1,7})|[Xx]([A-Fa-f0-9]{1,6}));|re} ""

  let numeric_character_reference_or_ampersand =
    Re.of_string
      {re|&#(?:(\d{1,7})|[Xx]([A-Fa-f0-9]{1,6}));|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)|re}
      "g"

  let escape_test = Re.of_string {re|[&<>"']|re} ""
  let escape_replace = Re.of_string {re|[&<>"']|re} "g"

  let escape_test_no_encode =
    Re.of_string {re|[<>"']|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)|re} ""

  let escape_replace_no_encode =
    Re.of_string {re|[<>"']|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)|re} "g"

  let percent_decode = Re.of_string {re|%25|re} "g"
  let split_pipe = Re.of_string " \\|" ""
  let slash_pipe = Re.of_string "\\\\\\|" "g"
  let carriage_return = Re.of_string {re|\r\n|\r|re} "g"
  let space_line = Re.of_string {re|^ +$|re} "gm"
  let not_space_start = Re.of_string {re|^\S*|re} ""
  let ending_newline = Re.of_string {re|\n$|re} ""
  let list_item_regex = make_list_item_regex
  let next_bullet_regex indent = lookup_indent next_bullet_table indent
  let hr_regex indent = lookup_indent hr_indent_table indent
  let fences_begin_regex indent = lookup_indent fences_begin_table indent
  let heading_begin_regex indent = lookup_indent heading_begin_table indent
  let html_begin_regex indent = lookup_indent html_begin_table indent
  let blockquote_begin_regex indent = lookup_indent blockquote_begin_table indent
end

(* Helper global regexes used only during grammar construction *)
let g_bull = Re.of_string "bull" "g"
let g_block_code = Re.of_string "blockCode" "g"
let g_fences = Re.of_string "fences" "g"
let g_blockquote = Re.of_string "blockquote" "g"
let g_heading = Re.of_string "heading" "g"
let g_hr = Re.of_string "hr" "g"
let g_html = Re.of_string "html" "g"
let g_pipe_table = Re.of_string "\\|table" "g"
let g_table = Re.of_string "table" "g"
let g_tag = Re.of_string "tag" "g"
let g_punct_space = Re.of_string "punctSpace" "g"
let g_not_punct_space = Re.of_string "notPunctSpace" "g"
let g_punct = Re.of_string "punct" "g"
let g_open_quote = Re.of_string "openQuote" "g"
let g_email = Re.of_string "email" "g"
let g_email_protocol = Re.of_string "emailProtocol" "g"
let g_brace_2 = Re.of_string "\\{2,\\}" "g"

(* Block-level grammar *)

let block_newline = Re.of_string {re|^(?:[ \t]*(?:\n|$))+|re} ""
let block_code = Re.of_string {re|^((?: {4}| {0,3}\t)[^\n]+(?:\n(?:[ \t]*(?:\n|$))*)?)+|re} ""

let block_fences =
  Re.of_string
    {re|^ {0,3}(`{3,}(?=[^`\n]*(?:\n|$))|~{3,})([^\n]*)(?:\n|$)(?:|([\s\S]*?)(?:\n|$))(?: {0,3}\1[~`]* *(?=\n|$)|$)|re}
    ""

let block_hr =
  Re.of_string {re|^ {0,3}((?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)|re} ""

let block_heading = Re.of_string {re|^ {0,3}(#{1,6})(?=\s|$)(.*)(?:\n+|$)|re} ""
let bullet = Re.of_string {re| {0,3}(?:[*+-]|\d{1,9}[.)])|re} ""

let lheading_core =
  {re|^(?!bull |blockCode|fences|blockquote|heading|html|table)((?:.|\n(?!\s*?\n|bull |fences|blockquote|heading|hr|html|table))+?)\n {0,3}(=+|-+) *(?:\n+|$)|re}

let block_lheading =
  edit_str lheading_core
  |> repl_r_re g_bull bullet
  |> repl_r g_block_code {re|(?: {4}| {0,3}\t)|re}
  |> repl_r g_fences {re| {0,3}(?:`{3,}|~{3,})|re}
  |> repl_r g_blockquote {re| {0,3}>|re}
  |> repl_r g_heading {re| {0,3}#{1,6}(?:\s|$)|re}
  |> repl_r g_hr {re| {0,3}(?:(?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)|re}
  |> repl_r g_html {re| {0,3}<[^\n>]+>\n|re}
  |> repl_r g_pipe_table ""
  |> to_re ""

let block_lheading_gfm =
  edit_str lheading_core
  |> repl_r_re g_bull bullet
  |> repl_r g_block_code {re|(?: {4}| {0,3}\t)|re}
  |> repl_r g_fences {re| {0,3}(?:`{3,}|~{3,})|re}
  |> repl_r g_blockquote {re| {0,3}>|re}
  |> repl_r g_heading {re| {0,3}#{1,6}(?:\s|$)|re}
  |> repl_r g_hr {re| {0,3}(?:(?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)|re}
  |> repl_r g_html {re| {0,3}<[^\n>]+>\n|re}
  |> repl_r g_table {re| {0,3}\|?(?:[:\- ]*\|)+[\:\- ]*\n|re}
  |> to_re ""

let _paragraph =
  {re|^([^\n]+(?:\n(?!hr|heading|lheading|blockquote|fences|list|html|table|[ \t]+\n)[^\n]+)*)|re}

let block_text = Re.of_string {re|^[^\n]+|re} ""
let _block_label = Re.of_string {re|(?!\s*\])(?:\\[\s\S]|[^\[\]\\])+|re} ""

let block_def =
  edit_str
    {re|^ {0,3}\[(label)\]: *(?:\n[ \t]*)?([^<\s][^\s]*|<.*?>)(?:(?: +(?:\n[ \t]*)?| *\n[ \t]*)(title))? *(?:\n+|$)|re}
  |> repl_s_re "label" _block_label
  |> repl_s "title" {re|(?:"(?:\\"?|[^"\\])*"|'[^'\n]*(?:\n[^'\n]+)*\n?'|\([^()]*\))|re}
  |> to_re ""

let block_list = edit_str {re|^(bull)([ \t][^\n]*?)?(?:\n|$)|re} |> repl_r_re g_bull bullet |> to_re ""
let _comment = Re.of_string {re|<!--(?:-?>|[\s\S]*?(?:-->|$))|re} ""

let block_html =
  edit_str
    ({re|^ {0,3}(?:<(script|pre|style|textarea)[\s>][\s\S]*?(?:</\1>[^\n]*\n*|$)|comment[^\n]*(\n+|$)|<\?[\s\S]*?(?:\?>[^\n]*\n*|$)|<![A-Z][\s\S]*?(?:>[^\n]*\n*|$)|<!\[CDATA\[[\s\S]*?(?:\]\]>[^\n]*\n*|$)|</?(tag)(?: +|\n|/?>)[\s\S]*?(?:(?:\n[ \t]*)+\n|$)|<(?!(?:script|pre|style|textarea)(?![a-z0-9-]))([a-z][a-z0-9-]*)(?:attribute)*? */?>(?=[ \t]*(?:\n|$))[\s\S]*?(?:(?:\n[ \t]*)+\n|$)|</[a-z][a-z0-9-]*\s*>(?=[ \t]*(?:\n|$))[\s\S]*?(?:(?:\n[ \t]*)+\n|$))|re})
  |> repl_s_re "comment" _comment
  |> repl_s "tag" _tag
  |> repl_s "attribute"
       {re| +[a-zA-Z:_][\w.:-]*(?: *= *"[^"\n]*"| *= *'[^'\n]*'| *= *[^\s"'=<>`]+)?|re}
  |> to_re "i"

let html_interrupt =
  {re| {0,3}(?:</?(?:tag)(?: +|\n|/?>)|<(?:(?:script|pre|style|textarea)(?:[\s>]|$)|!--)|<\?|<![A-Za-z]|<!\[CDATA\[)|re}

let fences_interrupt = {re| {0,3}(?:`{3,}(?=[^`\n]*(?:\n|$))|~~~)[^\n]*(?:\n|$)|re}

let create_paragraph (list_interrupt : string) : Re.t =
  edit_str _paragraph
  |> repl_s_re "hr" block_hr
  |> repl_s "heading" {re| {0,3}#{1,6}(?:\s|$)|re}
  |> repl_s "|lheading" ""
  |> repl_s "|table" ""
  |> repl_s "blockquote" {re| {0,3}>|re}
  |> repl_s "fences" fences_interrupt
  |> repl_s "list" list_interrupt
  |> repl_s "html" html_interrupt
  |> repl_s "tag" _tag
  |> to_re ""

let block_paragraph = create_paragraph {re| {0,3}(?:[*+-]|1[.)])[ \t]+[^ \t\n]|re}
let blockquote_paragraph = create_paragraph {re| {0,3}(?:[*+-]|\d{1,9}[.)])(?:[ \t]|\n|$)|re}

let block_blockquote =
  edit_str {re|^( {0,3}> ?(paragraph|[^\n]*)(?:\n|$))+|re}
  |> repl_s_re "paragraph" blockquote_paragraph
  |> to_re ""

let gfm_table =
  edit_str
    {re|^ *([^\n ].*)\n {0,3}((?:\| *)?:?-+:? *(?:\| *:?-+:? *)*(?:\| *)?)(?:\n((?:(?! *\n|hr|heading|blockquote|code|fences|list|html).*(?:\n|$))*)\n*|$)|re}
  |> repl_s_re "hr" block_hr
  |> repl_s "heading" {re| {0,3}#{1,6}(?:\s|$)|re}
  |> repl_s "blockquote" {re| {0,3}>|re}
  |> repl_s "code" {re|(?: {4}| {0,3}\t)[^\n]|re}
  |> repl_s "fences" fences_interrupt
  |> repl_s "list" {re| {0,3}(?:[*+-]|1[.)])[ \t]|re}
  |> repl_s "html" html_interrupt
  |> repl_s "tag" _tag
  |> to_re ""

let block_gfm_paragraph =
  edit_str _paragraph
  |> repl_s_re "hr" block_hr
  |> repl_s "heading" {re| {0,3}#{1,6}(?:\s|$)|re}
  |> repl_s "|lheading" ""
  |> repl_s_re "table" gfm_table
  |> repl_s "blockquote" {re| {0,3}>|re}
  |> repl_s "fences" fences_interrupt
  |> repl_s "list" {re| {0,3}(?:[*+-]|1[.)])[ \t]+[^ \t\n]|re}
  |> repl_s "html" html_interrupt
  |> repl_s "tag" _tag
  |> to_re ""

let block_pedantic_html =
  edit_str
    {re|^ *(?:comment *(?:\n|\s*$)|<(tag)[\s\S]+?</\1> *(?:\n{2,}|\s*$)|<tag(?:"[^"]*"|'[^']*'|\s[^'"/>\s]*)*?/?> *(?:\n{2,}|\s*$))|re}
  |> repl_s_re "comment" _comment
  |> repl_r g_tag
       {re|(?!(?:a|em|strong|small|s|cite|q|dfn|abbr|data|time|code|var|samp|kbd|sub|sup|i|b|u|mark|ruby|rt|rp|bdi|bdo|span|br|wbr|ins|del|img)\b)\w+(?!:|[^\w\s@]*@)\b|re}
  |> to_re ""

let block_pedantic_def =
  Re.of_string {re|^ *\[([^\]]+)\]: *<?([^\s>]+)>?(?: +(["(][^\n]+[")]))? *(?:\n+|$)|re} ""

let block_pedantic_heading = Re.of_string {re|^(#{1,6})(.*)(?:\n+|$)|re} ""
let block_pedantic_lheading = Re.of_string {re|^(.+?)\n {0,3}(=+|-+) *(?:\n+|$)|re} ""

let block_pedantic_paragraph =
  edit_str _paragraph
  |> repl_s_re "hr" block_hr
  |> repl_s "heading" " *#{1,6} *[^\n]"
  |> repl_s_re "lheading" block_lheading
  |> repl_s "|table" ""
  |> repl_s "blockquote" {re| {0,3}>|re}
  |> repl_s "|fences" ""
  |> repl_s "|list" ""
  |> repl_s "|html" ""
  |> repl_s "|tag" ""
  |> to_re ""

type block_rules = {
  blockquote : Re.t;
  code : Re.t;
  def : Re.t;
  fences : Re.t option;
  heading : Re.t;
  hr : Re.t;
  html : Re.t;
  lheading : Re.t;
  list : Re.t;
  newline : Re.t;
  paragraph : Re.t;
  table : Re.t option;
  text : Re.t;
}

let block_normal : block_rules =
  {
    blockquote = block_blockquote;
    code = block_code;
    def = block_def;
    fences = Some block_fences;
    heading = block_heading;
    hr = block_hr;
    html = block_html;
    lheading = block_lheading;
    list = block_list;
    newline = block_newline;
    paragraph = block_paragraph;
    table = None;
    text = block_text;
  }

let block_gfm : block_rules =
  {
    block_normal with
    lheading = block_lheading_gfm;
    table = Some gfm_table;
    paragraph = block_gfm_paragraph;
  }

let block_pedantic : block_rules =
  {
    block_normal with
    html = block_pedantic_html;
    def = block_pedantic_def;
    heading = block_pedantic_heading;
    fences = None;
    lheading = block_pedantic_lheading;
    paragraph = block_pedantic_paragraph;
  }

(* Inline-level grammar *)

let inline_escape = Re.of_string {re|^\\([!"#$%&'()*+,\-./:;<=>?@\[\]\\^_`{|}~])|re} ""
let inline_code = Re.of_string {re|^(`+)([^`]|[^`][\s\S]*?[^`])\1(?!`)|re} ""
let inline_br = Re.of_string {re|^( {2,}|\\)\n(?!\s*$)[ \t]*|re} ""

let inline_text =
  Re.of_string {re|^(`+|[^`])(?:(?= {2,}\n)|[\s\S]*?(?:(?=[\\<!\[`*_]|\b_|$)|[^ ](?= {2,}\n)))|re} ""

let _punctuation = {re|[\p{P}\p{S}]|re}
let _punctuation_or_space = {re|[\s\p{P}\p{S}]|re}
let _not_punctuation_or_space = {re|[^\s\p{P}\p{S}]|re}

let inline_punctuation =
  edit_str {re|^((?![*_])punctSpace)|re} |> repl_r g_punct_space _punctuation_or_space |> to_re "u"

let _open_quote = {re|[\p{Pi}\p{Ps}"']|re}
let _punctuation_gfm_strong_em = {re|(?!~)[\p{P}\p{S}]|re}
let _punctuation_or_space_gfm_strong_em = {re|(?!~)[\s\p{P}\p{S}]|re}
let _not_punctuation_or_space_gfm_strong_em = {re|(?:[^\s\p{P}\p{S}]|~)|re}

let inline_block_skip =
  edit_str {re|link|precode-code|html|re}
  |> repl_s "link"
       {re|\[(?:[^\[\]`]|(?<a>`+)[^`]+\k<a>(?!`))*?\]\((?:\\[\s\S]|[^\\\(\)]|\((?:\\[\s\S]|[^\\\(\)])*\))*\)|re}
  |> repl_s "precode-" (if supports_lookbehind then {re|(?<!`)()|re} else {re|(^^|[^`])|re})
  |> repl_s "code" {re|(?<b>`+)[^`]+\k<b>(?!`)|re}
  |> repl_s "html" {re|<(?! )[^<>]*?>|re}
  |> to_re "g"

let em_strong_l_delim_core =
  {re|^(?:\*+(?:((?!\*)punct)|([^\s*]))?)|^_+(?:((?!_)punct)|([^\s_]))?|re}

let em_strong_l_delim =
  edit_str em_strong_l_delim_core |> repl_r g_punct _punctuation |> to_re "u"

let em_strong_l_delim_gfm =
  edit_str em_strong_l_delim_core |> repl_r g_punct _punctuation_gfm_strong_em |> to_re "u"

let em_strong_l_delim_pedantic =
  edit_str
    {re|^(?:\*+(?:((?!\*)(?!openQuote)punct)|([^\s*]))?)|^_+(?:((?!_)(?!openQuote)punct)|([^\s_]))?|re}
  |> repl_r g_open_quote _open_quote
  |> repl_r g_punct _punctuation
  |> to_re "u"

let em_strong_r_delim_ast_core =
  {re|^[^_*]*?__[^_*]*?\*[^_*]*?(?=__)|[^*]+(?=[^*])|(?!\*)punct(\*+)(?=[\s]|$)|notPunctSpace(\*+)(?!\*)(?=punctSpace|$)|(?!\*)punctSpace(\*+)(?=notPunctSpace)|[\s](\*+)(?!\*)(?=punct)|(?!\*)punct(\*+)(?!\*)(?=punct)|notPunctSpace(\*+)(?=notPunctSpace)|re}

let em_strong_r_delim_ast =
  edit_str em_strong_r_delim_ast_core
  |> repl_r g_not_punct_space _not_punctuation_or_space
  |> repl_r g_punct_space _punctuation_or_space
  |> repl_r g_punct _punctuation
  |> to_re "gu"

let em_strong_r_delim_ast_gfm =
  edit_str em_strong_r_delim_ast_core
  |> repl_r g_not_punct_space _not_punctuation_or_space_gfm_strong_em
  |> repl_r g_punct_space _punctuation_or_space_gfm_strong_em
  |> repl_r g_punct _punctuation_gfm_strong_em
  |> to_re "gu"

let em_strong_r_delim_ast_pedantic =
  edit_str
    {re|^[^_*]*?__[^_*]*?\*[^_*]*?(?=__)|[^*]+(?=[^*])|(?!\*)punct(\*+)(?=[\s]|$)|notPunctSpace(\*+)(?!\*)(?=punctSpace|$)|(?!\*)[\s](\*+)(?=notPunctSpace)|[\s](\*+)(?!\*)(?=punct)|(?!\*)punct(\*+)(?!\*)(?=punct)|(?:(?!\*)punct|notPunctSpace)(\*+)(?!\*)(?=notPunctSpace)|re}
  |> repl_r g_not_punct_space _not_punctuation_or_space
  |> repl_r g_punct_space _punctuation_or_space
  |> repl_r g_punct _punctuation
  |> to_re "gu"

let em_strong_r_delim_und =
  edit_str
    {re|^[^_*]*?\*\*[^_*]*?_[^_*]*?(?=\*\*)|[^_]+(?=[^_])|(?!_)punct(_+)(?=[\s]|$)|notPunctSpace(_+)(?!_)(?=punctSpace|$)|(?!_)punctSpace(_+)(?=notPunctSpace)|[\s](_+)(?!_)(?=punct)|(?!_)punct(_+)(?!_)(?=punct)|re}
  |> repl_r g_not_punct_space _not_punctuation_or_space
  |> repl_r g_punct_space _punctuation_or_space
  |> repl_r g_punct _punctuation
  |> to_re "gu"

let em_strong_r_delim_und_pedantic =
  edit_str
    {re|^[^_*]*?\*\*[^_*]*?_[^_*]*?(?=\*\*)|[^_]+(?=[^_])|(?!_)punct(_+)(?=[\s]|$)|notPunctSpace(_+)(?!_)(?=punctSpace|$)|(?!_)[\s](_+)(?=notPunctSpace)|[\s](_+)(?!_)(?=punct)|(?!_)punct(_+)(?!_)(?=punct)|(?:(?!_)punct|notPunctSpace)(_+)(?!_)(?=notPunctSpace)|re}
  |> repl_r g_not_punct_space _not_punctuation_or_space
  |> repl_r g_punct_space _punctuation_or_space
  |> repl_r g_punct _punctuation
  |> to_re "gu"

let del_l_delim =
  edit_str {re|^~~?(?:((?!~)punct)|[^\s~])|re} |> repl_r g_punct _punctuation |> to_re "u"

let del_r_delim =
  edit_str
    {re|^[^~]+(?=[^~])|(?!~)punct(~~?)(?=[\s]|$)|notPunctSpace(~~?)(?!~)(?=punctSpace|$)|(?!~)punctSpace(~~?)(?=notPunctSpace)|[\s](~~?)(?!~)(?=punct)|(?!~)punct(~~?)(?!~)(?=punct)|notPunctSpace(~~?)(?=notPunctSpace)|re}
  |> repl_r g_not_punct_space _not_punctuation_or_space
  |> repl_r g_punct_space _punctuation_or_space
  |> repl_r g_punct _punctuation
  |> to_re "gu"

let any_punctuation = edit_str {re|\\(punct)|re} |> repl_r g_punct _punctuation |> to_re "gu"

let any_punctuation_outside_code =
  edit_str {re|(`+)(?:[^`]|[^`][\s\S]*?[^`])\1(?!`)|`+|\\(punct)|re}
  |> repl_r g_punct _punctuation
  |> to_re "gu"

let inline_autolink =
  edit_str {re|^<(scheme:[^\s\x00-\x1f<>]*|email)>|re}
  |> repl_s "scheme" {re|[a-zA-Z][a-zA-Z0-9+.-]{1,31}|re}
  |> repl_s "email"
       {re|[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+(@)[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)+(?![-_])|re}
  |> to_re ""

let _inline_comment = edit_re _comment |> repl_s "(?:-->|$)" "-->" |> to_re ""

let inline_tag =
  edit_str
    {re|^comment|^</[a-zA-Z][a-zA-Z0-9-]*\s*>|^<[a-zA-Z][a-zA-Z0-9-]*(?:attribute)*?\s*/?>|^<\?[\s\S]*?\?>|^<![a-zA-Z]+\s[\s\S]*?>|^<!\[CDATA\[[\s\S]*?\]\]>|re}
  |> repl_s_re "comment" _inline_comment
  |> repl_s "attribute"
       {re|\s+[a-zA-Z:_][\w.:-]*(?:\s*=\s*"[^"]*"|\s*=\s*'[^']*'|\s*=\s*[^\s"'=<>`]+)?|re}
  |> to_re ""

let _inline_label_brackets = Re.of_string {re|\[(?:\\[\s\S]|[^\[\]\\])*\]|re} ""

let _inline_label =
  edit_str {re|(?:\[(?:brackets|\\[\s\S]|[^\[\]\\])*\]|\\[\s\S]|`+(?!`)[^`]*?`+(?!`)|``+(?=\])|[^\[\]\\`])*?|re}
  |> repl_s_re "brackets" _inline_label_brackets
  |> to_re ""

let inline_link =
  edit_str
    {re|^!?\[(label)\]\([ \t\n]*(href)(?:(?:[ \t]+(?:\n[ \t]*)?|\n[ \t]*)(title))?[ \t\n]*\)|re}
  |> repl_s_re "label" _inline_label
  |> repl_s "href" {re|<(?:\\.|[^\n<>\\])+>|[^ \t\n\x00-\x1f]+|(?=\))|re}
  |> repl_s "title" {re|"(?:\\"?|[^"\\])*"|'(?:\\'?|[^'\\])*'|\((?:\\\)?|[^)\\])*\)|re}
  |> to_re ""

let inline_reflink =
  edit_str {re|^!?\[(label)\]\[(ref)\]|re}
  |> repl_s_re "label" _inline_label
  |> repl_s_re "ref" _block_label
  |> to_re ""

let inline_nolink =
  edit_str {re|^!?\[(ref)\](?:\[\])?|re} |> repl_s_re "ref" _block_label |> to_re ""

let _bounded_block_label = Re.of_string {re|(?!\s*\])(?:\\[\s\S]|[^\[\]\\]){1,999}|re} ""

let _bounded_inline_label =
  edit_str
    {re|(?:[^\[\]\\`]*(?:\[(?:brackets|\\[\s\S]|[^\[\]\\])*\]|\\[\s\S]|`+(?!`)[^`]*?`+(?!`)|``+(?=\]))){0,999}?[^\[\]\\`]*?|re}
  |> repl_s_re "brackets" _inline_label_brackets
  |> to_re ""

let inline_reflink_search =
  let rf =
    edit_str {re|^!?\[(label)\]\[(ref)\]|re}
    |> repl_s_re "label" _bounded_inline_label
    |> repl_s_re "ref" _bounded_block_label
    |> to_re ""
  in
  let nl = edit_str {re|^!?\[(ref)\](?:\[\])?|re} |> repl_s_re "ref" _bounded_block_label |> to_re "" in
  edit_str {re|reflink|nolink(?!\()|re}
  |> repl_s_re "reflink" rf
  |> repl_s_re "nolink" nl
  |> to_re "g"

let _case_insensitive_protocol = {re|[hH][tT][tT][pP][sS]?|[fF][tT][pP]|re}
let _extended_email = {re|[A-Za-z0-9._+-]+@[a-zA-Z0-9-_]+(?:\.[a-zA-Z0-9-_]*[a-zA-Z0-9])+(?![\w-])|re}

let _extended_email_protocol =
  edit_str {re|(?:mailto:email|xmpp:email(?:\/[A-Za-z0-9@.]+)?)|re}
  |> repl_r g_email _extended_email
  |> to_re ""

let inline_pedantic_link =
  edit_str {re|^!?\[(label)\]\((.*?)\)|re} |> repl_s_re "label" _inline_label |> to_re ""

let inline_pedantic_reflink =
  edit_str {re|^!?\[(label)\]\s*\[([^\]]*)\]|re} |> repl_s_re "label" _inline_label |> to_re ""

let inline_gfm_url =
  edit_str {re|^emailProtocol|^((?:protocol):\/\/|www\.)(?:[a-zA-Z0-9\-]+\.?)+[^\s<]*|^email|re}
  |> repl_s_re "emailProtocol" _extended_email_protocol
  |> repl_s "protocol" _case_insensitive_protocol
  |> repl_s "email"
       {re|[A-Za-z0-9._+-]+(@)[a-zA-Z0-9-_]+(?:\.[a-zA-Z0-9-_]*[a-zA-Z0-9])+(?![\w-])|re}
  |> to_re ""

let inline_gfm_backpedal =
  Re.of_string {re|(?:[^?!.,:;*_'"~()&]+|\([^)]*\)|&(?![a-zA-Z0-9]+;$)|[?!.,:;*_'"~)]+(?!$))+|re} ""

let inline_gfm_text =
  edit_str
    {re|^(?:[^a-zA-Z0-9](?=emailProtocol)|(`+|~+|[^`~])(?:(?=[`~])|(?= {2,}\n)|(?=[a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-]+@)|[\s\S]*?(?:(?=[\\<!\[`*~_]|\b_|protocol:\/\/|www\.|$)|[^ ](?= {2,}\n)|[^a-zA-Z0-9](?=emailProtocol)|[^a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-](?=[a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-]+@))))|re}
  |> repl_s "protocol" _case_insensitive_protocol
  |> repl_r g_email_protocol {re|(?:mailto|xmpp):|re}
  |> to_re ""

let inline_breaks_br = edit_re inline_br |> repl_s "{2,}" "*" |> to_re ""

let inline_breaks_text =
  edit_re inline_gfm_text
  |> repl_s {re|\b_|re} {re|\b_| {2,}\n|re}
  |> repl_r g_brace_2 "*"
  |> to_re ""

type inline_rules = {
  backpedal : Re.t option;
  any_punctuation : Re.t;
  any_punctuation_outside_code : Re.t;
  autolink : Re.t;
  block_skip : Re.t;
  br : Re.t;
  code : Re.t;
  del_l_delim : Re.t option;
  del_r_delim : Re.t option;
  em_strong_l_delim : Re.t;
  em_strong_r_delim_ast : Re.t;
  em_strong_r_delim_und : Re.t;
  escape : Re.t;
  link : Re.t;
  nolink : Re.t;
  punctuation : Re.t;
  reflink : Re.t;
  reflink_search : Re.t;
  tag : Re.t;
  text : Re.t;
  url : Re.t option;
}

let inline_normal : inline_rules =
  {
    backpedal = None;
    any_punctuation;
    any_punctuation_outside_code;
    autolink = inline_autolink;
    block_skip = inline_block_skip;
    br = inline_br;
    code = inline_code;
    del_l_delim = None;
    del_r_delim = None;
    em_strong_l_delim;
    em_strong_r_delim_ast;
    em_strong_r_delim_und;
    escape = inline_escape;
    link = inline_link;
    nolink = inline_nolink;
    punctuation = inline_punctuation;
    reflink = inline_reflink;
    reflink_search = inline_reflink_search;
    tag = inline_tag;
    text = inline_text;
    url = None;
  }

let inline_pedantic : inline_rules =
  {
    inline_normal with
    em_strong_l_delim = em_strong_l_delim_pedantic;
    em_strong_r_delim_ast = em_strong_r_delim_ast_pedantic;
    em_strong_r_delim_und = em_strong_r_delim_und_pedantic;
    link = inline_pedantic_link;
    reflink = inline_pedantic_reflink;
  }

let inline_gfm : inline_rules =
  {
    inline_normal with
    em_strong_r_delim_ast = em_strong_r_delim_ast_gfm;
    em_strong_l_delim = em_strong_l_delim_gfm;
    del_l_delim = Some del_l_delim;
    del_r_delim = Some del_r_delim;
    url = Some inline_gfm_url;
    backpedal = Some inline_gfm_backpedal;
    text = inline_gfm_text;
  }

let inline_breaks : inline_rules =
  { inline_gfm with br = inline_breaks_br; text = inline_breaks_text }

type rules = {
  block : block_rules;
  inline : inline_rules;
}

let select ~gfm ~breaks ~pedantic : rules =
  if pedantic then { block = block_pedantic; inline = inline_pedantic }
  else if gfm then
    { block = block_gfm; inline = (if breaks then inline_breaks else inline_gfm) }
  else { block = block_normal; inline = inline_normal }
