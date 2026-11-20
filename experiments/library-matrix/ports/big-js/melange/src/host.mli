(* The JavaScript number conversions upstream relies on, as the shared big.js core sees them.

   big.ml and host.mli are byte-identical in ports/big-js/melange/src/ and
   ports/big-js/jsoo/src/; host.ml and the entry module (index.ml for Melange, main.ml for
   js_of_ocaml) are per compiler. *)

(* [Number(s)]: upstream's [+s] and Math.sqrt's coercion; NaN where s is not a number. *)
val number : string -> float

(* [String(n)]: how upstream prints exponents and reads estimates. *)
val string_of_number : float -> string

(* [n.toExponential()]: the shortest digits of n in d.ddde±x form. *)
val to_exponential : float -> string
