(* Entry module of the Melange port: the module's one export, run. *)
let run input = Host.export (Session.run input)
