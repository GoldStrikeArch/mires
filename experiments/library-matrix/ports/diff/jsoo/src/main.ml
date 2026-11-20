open Js_of_ocaml
let ()=Js.export "run" (Js.wrap_callback Program.run)
