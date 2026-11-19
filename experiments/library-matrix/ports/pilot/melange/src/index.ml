(* Entry module of the Melange port. The boundary: the corpus input is a JS object typed as
   [< text : string; top : int > Js.t]; Melange strings and ints are the JS values themselves, so
   no conversion happens on either side. *)

type input = < text : string ; top : int > Js.t

let run (input : input) = Pilot.analyse input##text input##top
