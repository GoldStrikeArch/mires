open Js_of_ocaml
open Js_host

let includes (haystack : js) (needle : js) : bool =
  let len = array_length haystack in
  let rec loop i =
    if i >= len then false
    else if js_equals (array_get haystack i) needle then true
    else loop (i + 1)
  in
  loop 0

let excludes haystack needle = not (includes haystack needle)

let to_int (value : js) : float = parse_int value
let to_float (value : js) : float = parse_float value

let replace_nan (value : float) (replacement : js) : js =
  if classify_float value = FP_nan then replacement else js_float value

let count_truthy (values : bool list) : int =
  List.fold_left (fun acc v -> if v then acc + 1 else acc) 0 values

let count_truthy_js (values : js list) : int =
  List.fold_left (fun acc v -> if is_truthy v then acc + 1 else acc) 0 values

let round (value : float) ?(base = 1.) () : float =
  if js_abs base >= 1. then js_round (value /. base) *. base
  else
    let counter_base = 1. /. base in
    js_round (value *. counter_base) /. counter_base

let parse_simple_css_selector (selector : string) : string option * (string * string list) list =
  let error_message = "Unexpected syntax '" ^ selector ^ "'" in
  let tag_re = new%js Js.regExp_withFlags (Js.string {|^\s*([a-z-]*)(.*)$|}) (Js.string "i") in
  let tag_match = tag_re##exec (Js.string selector) in
  let tag_match =
    Js.match_result (Js.Opt.get tag_match (fun () -> failwith error_message))
  in
  let tag_raw = Js.to_string (Js.Optdef.get (Js.array_get tag_match 1) (fun () -> Js.string "")) in
  let rest = Js.to_string (Js.Optdef.get (Js.array_get tag_match 2) (fun () -> Js.string "")) in
  let tag = if tag_raw = "" then None else Some tag_raw in
  let attributes : (string, string list) Hashtbl.t = Hashtbl.create 8 in
  let add_attribute name value =
    let prev = try Hashtbl.find attributes name with Not_found -> [] in
    Hashtbl.replace attributes name (prev @ [ value ])
  in
  let parts_re = new%js Js.regExp_withFlags (Js.string {|([.:#][\w-]+|\[.+?\])|}) (Js.string "gi") in
  let rec loop () =
    match Js.Opt.to_option (parts_re##exec (Js.string rest)) with
    | None -> ()
    | Some handle ->
      let m = Js.match_result handle in
      let part = Js.to_string (Js.Optdef.get (Js.array_get m 0) (fun () -> Js.string "")) in
      (match part.[0] with
      | '.' -> add_attribute "class" (String.sub part 1 (String.length part - 1))
      | '#' -> add_attribute "id" (String.sub part 1 (String.length part - 1))
      | '[' ->
        let attr_re =
          new%js Js.regExp (Js.string {|^\[([\w-]+)([~|^$*]?=("(.*?)"|([\w-]+)))?(\s+[is])?\]$|})
        in
        (match Js.Opt.to_option (attr_re##exec (Js.string part)) with
        | None -> failwith error_message
        | Some handle ->
          let am = Js.match_result handle in
          let name = Js.to_string (Js.Optdef.get (Js.array_get am 1) (fun () -> Js.string "")) in
          let quoted = Js.Optdef.to_option (Js.array_get am 4) in
          let ident = Js.Optdef.to_option (Js.array_get am 5) in
          let value =
            match quoted, ident with
            | Some q, _ -> Js.to_string q
            | None, Some i -> Js.to_string i
            | None, None -> ""
          in
          add_attribute name value)
      | _ -> failwith error_message);
      loop ()
  in
  loop ();
  let attrs = Hashtbl.fold (fun k v acc -> (k, v) :: acc) attributes [] in
  tag, attrs
