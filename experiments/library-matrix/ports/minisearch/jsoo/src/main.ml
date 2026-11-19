(* Entry of the js_of_ocaml port: [run] is published with [Js.export], which writes to
   js_of_ocaml's [jsoo_exports] object (the builder's facade binds it to a fresh object and
   re-exports [run]). The input is the corpus case's JavaScript object itself (Host.Value), the
   result a JavaScript object (Host.Json); the core is Script and the modules it uses, the same
   OCaml source as the Melange port. *)

open Js_of_ocaml

let () = Js.export "run" (Js.wrap_callback Script.run)
