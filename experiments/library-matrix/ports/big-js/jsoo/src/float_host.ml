open Js_of_ocaml
let text x = Js.to_string (Js.Unsafe.fun_call (Js.Unsafe.get Js.Unsafe.global "String") [|Js.Unsafe.inject x|])
let root s = sqrt (Js.float_of_number (Js.Unsafe.fun_call (Js.Unsafe.get Js.Unsafe.global "Number") [|Js.Unsafe.inject (Js.string s)|]))
