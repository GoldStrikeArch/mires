(* noble's _u64.ts: a 64-bit word is a high and a low 32-bit half.

   Shifts and rotations return one half, as in noble. Sums return both halves as a pair; the
   carry out of the low halves comes from an unsigned comparison of the wrapped sum with an
   addend, where noble divides its unmasked JS-number sum by 2^32. The 3- to 5-term sums are
   noble's add3L/add3H ... add5L/add5H, each computed in one function. *)

open U32

(* High half of a 64-bit logical right shift by [s] in 0..31. *)
let shr_sh h _l s = h lsr s

(* Low half of a 64-bit logical right shift by [s] in 1..31. *)
let shr_sl h l s = (h lsl (32 - s)) lor (l lsr s)

(* High half of a 64-bit right rotation by [s] in 1..31. *)
let rotr_sh h l s = (h lsr s) lor (l lsl (32 - s))

(* Low half of a 64-bit right rotation by [s] in 1..31. *)
let rotr_sl h l s = (h lsl (32 - s)) lor (l lsr s)

(* High half of a 64-bit right rotation by [s] in 33..63. *)
let rotr_bh h l s = (h lsl (64 - s)) lor (l lsr (s - 32))

(* Low half of a 64-bit right rotation by [s] in 33..63. *)
let rotr_bl h l s = (h lsr (s - 32)) lor (l lsl (64 - s))

(* High half of a 64-bit right rotation by 32: the low half. *)
let rotr32_h _h l = l

(* Low half of a 64-bit right rotation by 32: the high half. *)
let rotr32_l h _l = h

(* noble's fromNumH: the high half of an integer in [0, 2^53). *)
let from_num_h n = of_float (n /. 4294967296.)

(* noble's fromNumL ([n >>> 0]): the low half of an integer in [0, 2^53). *)
let from_num_l n =
  let low = Float.rem n 4294967296. in
  of_float (if low >= 2147483648. then low -. 4294967296. else low)

(* 1 when adding [addend] wrapped the 32-bit [sum], else 0. *)
let carry_out sum addend = if unsigned_compare sum addend < 0 then 1l else 0l

(* The sum of two 64-bit words, as (high, low). *)
let add ah al bh bl =
  let l = al + bl in
  (ah + bh + carry_out l bl, l)

(* The sum of three 64-bit words, as (high, low). *)
let add3 ah al bh bl ch cl =
  let ab = al + bl in
  let l = ab + cl in
  (ah + bh + ch + carry_out ab bl + carry_out l cl, l)

(* The sum of four 64-bit words, as (high, low). *)
let add4 ah al bh bl ch cl dh dl =
  let ab = al + bl in
  let abc = ab + cl in
  let l = abc + dl in
  (ah + bh + ch + dh + carry_out ab bl + carry_out abc cl + carry_out l dl, l)

(* The sum of five 64-bit words, as (high, low). *)
let add5 ah al bh bl ch cl dh dl eh el =
  let ab = al + bl in
  let abc = ab + cl in
  let abcd = abc + dl in
  let l = abcd + el in
  let carries = carry_out ab bl + carry_out abc cl + carry_out abcd dl + carry_out l el in
  (ah + bh + ch + dh + eh + carries, l)
