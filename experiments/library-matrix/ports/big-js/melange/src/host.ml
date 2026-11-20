(* Melange host implementation for the shared big.js OCaml core: JS strings are OCaml strings,
   so the conversions are direct externals. *)

external number : string -> float = "Number"
external string_of_number : float -> string = "String"
external to_exponential : float -> string = "toExponential" [@@mel.send]
