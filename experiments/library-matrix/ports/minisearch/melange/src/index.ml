(* Entry module of the Melange port: [run] is the ES module's named export. The input is the
   corpus case's JavaScript object itself (Host.Value), the result a JavaScript object (Host.Json);
   the core is Script and the modules it uses, the same OCaml source as the js_of_ocaml port. *)

let run input = Script.run input
