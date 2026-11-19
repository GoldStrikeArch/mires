(* Floor port (Melange): Melange strings are JS strings, so the value passes through unchanged. *)
let run (input : < text : string > Js.t) = input##text
