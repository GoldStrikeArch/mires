(* Floor port (js_of_ocaml): the boundary alone, a JS string to an OCaml UTF-8 string and back. *)
open Js_of_ocaml

class type input = object
  method text : Js.js_string Js.t Js.readonly_prop
end

let run (input : input Js.t) = Js.string (Js.to_string input##.text)

let () = Js.export "run" (Js.wrap_callback run)
