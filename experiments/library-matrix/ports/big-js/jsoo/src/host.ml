(* js_of_ocaml host implementation for the shared big.js OCaml core: OCaml floats are JS numbers,
   and the strings involved are ASCII, so Js.string and Js.to_string are exact. *)

open Js_of_ocaml

let number s : float =
  Js.float_of_number (Js.Unsafe.fun_call Js.Unsafe.global##._Number [| Js.Unsafe.inject (Js.string s) |])

let string_of_number n = Js.to_string (Js.number_of_float n)##toString
let to_exponential n = Js.to_string (Js.number_of_float n)##toExponential
