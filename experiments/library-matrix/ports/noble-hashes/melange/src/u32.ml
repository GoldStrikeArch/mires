(* 32-bit words as Int32 values, with the bitwise operators and [+] under their int names for
   local opens such as [U32.(a lxor b + c)]. Int32 keeps the arithmetic 32-bit on every OCaml
   target: native ints are 63-bit, Melange's and js_of_ocaml's 32-bit. *)

include Int32

let ( + ) = add
let ( land ) = logand
let ( lor ) = logor
let ( lxor ) = logxor
let ( lsl ) = shift_left
let ( lsr ) = shift_right_logical
let lnot = lognot

(* noble's [rotr] for 32-bit words. *)
let rotr word shift = (word lsr shift) lor (word lsl (32 - shift))
