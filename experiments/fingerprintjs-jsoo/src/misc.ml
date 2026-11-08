open Js_of_ocaml
open Js_host

let error_to_object (error : js) : js =
  let result = obj () in
  set result "name" (get error "name");
  set result "message" (get error "message");
  let stack = get error "stack" in
  if is_undefined stack then set result "stack" undefined
  else set result "stack" (meth stack "split" [| js_string "\n" |]);
  let keys = object_keys error in
  let n = array_length keys in
  for i = 0 to n - 1 do
    let key = array_get keys i in
    set_key result key (get_key error key)
  done;
  result

let is_function_native (func : js) : bool =
  let re =
    new%js Js.regExp (Js.string {|^function\s.*?\{\s*\[native code]\s*}$|})
  in
  Js.to_bool (re##test (Js.Unsafe.coerce (fun_call (host_object "String") [| func |])))
