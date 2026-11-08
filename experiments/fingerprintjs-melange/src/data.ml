open Host

let includes (haystack : js) (needle : js) : bool =
  let l = array_length haystack in
  let rec loop i =
    if i >= l then false
    else if get_i haystack i == needle then true
    else loop (i + 1)
  in
  loop 0

let excludes haystack needle = not (includes haystack needle)

let to_int_js (value : js) : float = parse_int value
let to_float_js (value : js) : float = parse_float value

let replace_nan (value : js) (replacement : js) : js =
  if is_number value && is_nan (to_float value) then replacement else value

let count_truthy (values : js) : int =
  let l = array_length values in
  let rec loop i sum =
    if i >= l then sum
    else loop (i + 1) (sum + if truthy (get_i values i) then 1 else 0)
  in
  loop 0 0

let round (value : float) (base : float) : float =
  if math_abs base >= 1. then math_round (value /. base) *. base
  else
    let counter_base = 1. /. base in
    math_round (value *. counter_base) /. counter_base

let add_attribute (attributes : js) (name : string) (value : string) : unit =
  let existing = get attributes name in
  let list =
    if is_undefined existing then
      let created = array_ () in
      set attributes name created;
      created
    else existing
  in
  ignore (push list (of_string value))

let parse_simple_css_selector (selector : string) : js * js =
  let error_message = "Unexpected syntax '" ^ selector ^ "'" in
  let tag_re = new_re "^\\s*([a-z-]*)(.*)$" "i" in
  let tag_match = re_exec tag_re selector in
  let tag_str = to_string (get_i tag_match 1) in
  let tag = if tag_str = "" then undefined else of_string tag_str in
  let attributes = object_ () in
  let parts_re = new_re "([.:#][\\w-]+|\\[.+?\\])" "gi" in
  let rest = to_string (get_i tag_match 2) in
  let rec loop () =
    let match_ = re_exec parts_re rest in
    if is_null match_ then ()
    else
      let part = to_string (get_i match_ 0) in
      let first = string_slice2 part 0 1 in
      (match first with
      | "." -> add_attribute attributes "class" (string_slice part 1)
      | "#" -> add_attribute attributes "id" (string_slice part 1)
      | "[" ->
          let attr_re =
            new_re
              "^\\[([\\w-]+)([~|^$*]?=(\"(.*?)\"|([\\w-]+)))?(\\s+[is])?\\]$"
              ""
          in
          let attribute_match = re_exec attr_re part in
          if not (is_null attribute_match) then
            let v4 = get_i attribute_match 4 in
            let v5 = get_i attribute_match 5 in
            let value =
              if not (is_nullish v4) then to_string v4
              else if not (is_nullish v5) then to_string v5
              else ""
            in
            add_attribute attributes (to_string (get_i attribute_match 1)) value
          else
            let throw : string -> unit =
              [%mel.raw {| function(m) { throw new Error(m); } |}]
            in
            throw error_message
      | _ ->
          let throw : string -> unit =
            [%mel.raw {| function(m) { throw new Error(m); } |}]
          in
          throw error_message);
      loop ()
  in
  loop ();
  (tag, attributes)

let get_utf8_bytes (input : string) : js =
  let len = string_length input in
  let result = new_uint8_array len in
  let rec loop i =
    if i >= len then result
    else
      let char_code = char_code_at input i in
      if char_code > 127 then text_encode input
      else (
        set_i result i (of_int char_code);
        loop (i + 1))
  in
  loop 0
