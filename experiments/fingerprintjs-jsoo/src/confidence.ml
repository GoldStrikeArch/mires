open Js_host
open Data
open Browser

let comment_template =
  "$ if upgrade to Pro: https://fingerprint.com/github/?utm_source=oss&utm_medium=referral&utm_campaign=confidence_score"

let get_open_confidence_score (components : js) : float =
  if is_android () then 0.4
  else if is_webkit () then
    if is_desktop_webkit () && not (is_webkit_616_or_newer () && is_safari_webkit ())
    then 0.5
    else 0.3
  else
    let platform_comp = get components "platform" in
    let platform =
      if js_in_s "value" platform_comp then
        let v = get platform_comp "value" in
        if is_nullish v then "" else to_string v
      else ""
    in
    if
      try String.sub platform 0 (min 3 (String.length platform)) = "Win"
      with _ -> false
    then 0.6
    else if
      try String.sub platform 0 (min 3 (String.length platform)) = "Mac"
      with _ -> false
    then 0.5
    else 0.7

let derive_pro_confidence_score open_score = round (0.99 +. 0.01 *. open_score) ~base:0.0001 ()

let get_confidence (components : js) : js =
  let open_score = get_open_confidence_score components in
  let pro_score = derive_pro_confidence_score open_score in
  (* JS String(pro_score) for the comment substitution. *)
  let pro_s = to_string (fun_call (host_object "String") [| js_float pro_score |]) in
  let comment =
    let re = new%js Js_of_ocaml.Js.regExp_withFlags (Js_of_ocaml.Js.string "\\$") (Js_of_ocaml.Js.string "g") in
    Js_of_ocaml.Js.to_string
      ((Js_of_ocaml.Js.string comment_template)##replace re (Js_of_ocaml.Js.string pro_s))
  in
  obj_fields [| "score", js_float open_score; "comment", js_string comment |]
