(* Entry module of the js_of_ocaml port: publishes run, which the builder's facade re-exports. *)
open Js_of_ocaml

let () = Js.export "run" (Js.wrap_callback (fun input -> Host.export (Session.run input)))
