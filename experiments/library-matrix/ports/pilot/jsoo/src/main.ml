(* Entry of the js_of_ocaml port. The boundary: the corpus input is a JS object read through a
   class type; [text] is a JS string converted to an OCaml UTF-8 string ([Js.to_string]), [top] a
   JS number read as an OCaml int; the OCaml result string is converted back with [Js.string].
   [run] is published with [Js.export], which writes to js_of_ocaml's [jsoo_exports] object. *)

open Js_of_ocaml

class type input = object
  method text : Js.js_string Js.t Js.readonly_prop
  method top : int Js.readonly_prop
end

let run (input : input Js.t) = Js.string (Pilot.analyse (Js.to_string input##.text) input##.top)

let () = Js.export "run" (Js.wrap_callback run)
