(* Entry of the js_of_ocaml port: [run] is published with [Js.export], which writes to
   js_of_ocaml's [jsoo_exports] object (the builder's facade binds it to a fresh object and
   re-exports [run]). *)

open Js_of_ocaml

let () = Js.export "run" (Js.wrap_callback Script.run)
