(* Fixed immutable lookup tables and core helpers for fflate 0.8.3.

   Every table matches node_modules/fflate/esm/browser.js bit-for-bit and is computed once at
   module initialisation without any cross-call mutable state. *)

open Host

exception Flate_error of int * string

let default_error_message code =
  match code with
  | 0 -> "unexpected EOF"
  | 1 -> "invalid block type"
  | 2 -> "invalid length/literal"
  | 3 -> "invalid distance"
  | 4 -> "stream finished"
  | 5 -> "no stream handler"
  | 6 -> "invalid header"
  | 7 -> "no callback"
  | 8 -> "invalid UTF-8 data"
  | _ -> "flate error"

let err code msg =
  let message = if String.length msg > 0 then msg else default_error_message code in
  raise (Flate_error (code, message))

let empty_u8 = U8.create 0

let[@inline] is_none = function None -> true | Some _ -> false
let[@inline] is_some = function Some _ -> true | None -> false

let[@inline] shft p = (p + 7) lsr 3

let slc v s e =
  let len = U8.length v in
  let s = if s < 0 then 0 else if s > len then len else s in
  let e = if e < s then s else if e > len then len else e in
  U8.copy_of (U8.subarray v s e)

let subarray_from v s =
  let len = U8.length v in
  let s = if s < 0 then 0 else if s > len then len else s in
  U8.subarray v s len

let last_32768 v =
  let len = U8.length v in
  if len > 32768 then U8.subarray v (len - 32768) len else v

let max_u8 a =
  let len = U8.length a in
  if len = 0 then 0
  else
    let m = ref (U8.get a 0) in
    for i = 1 to len - 1 do
      let v = U8.get a i in
      if v > !m then m := v
    done;
    !m

(* Fixed length extra bits (32 bytes) *)
let fleb =
  U8.of_array
    [|
      0; 0; 0; 0; 0; 0; 0; 0; 1; 1; 1; 1; 2; 2; 2; 2;
      3; 3; 3; 3; 4; 4; 4; 4; 5; 5; 5; 5; 0; 0; 0; 0;
    |]

(* Fixed distance extra bits (32 bytes) *)
let fdeb =
  U8.of_array
    [|
      0; 0; 0; 0; 1; 1; 2; 2; 3; 3; 4; 4; 5; 5; 6; 6;
      7; 7; 8; 8; 9; 9; 10; 10; 11; 11; 12; 12; 13; 13; 0; 0;
    |]

(* Code length index map (19 bytes) *)
let clim =
  U8.of_array [| 16; 17; 18; 0; 8; 7; 9; 6; 10; 5; 11; 4; 12; 3; 13; 2; 14; 1; 15 |]

let build_freb eb start =
  let b = U16.create 31 in
  let cur = ref (start + 1) in
  U16.set b 0 !cur;
  for i = 1 to 30 do
    cur := !cur + (1 lsl U8.get eb (i - 1));
    U16.set b i !cur
  done;
  let r = I32.create (U16.get b 30) in
  for i = 1 to 29 do
    let bi = U16.get b i in
    let bi1 = U16.get b (i + 1) in
    for j = bi to bi1 - 1 do
      I32.set r j (((j - bi) lsl 5) lor i)
    done
  done;
  (b, r)

let fl, revfl =
  let b, r = build_freb fleb 2 in
  U16.set b 28 258;
  I32.set r 258 28;
  (b, r)

let fd, revfd = build_freb fdeb 0

(* 15-bit reversal lookup table for 0..32767 *)
let rev =
  let t = U16.create 32768 in
  for i = 0 to 32767 do
    let x = ((i land 0xaaaa) lsr 1) lor ((i land 0x5555) lsl 1) in
    let x = ((x land 0xcccc) lsr 2) lor ((x land 0x3333) lsl 2) in
    let x = ((x land 0xf0f0) lsr 4) lor ((x land 0x0f0f) lsl 4) in
    U16.set t i ((((x land 0xff00) lsr 8) lor ((x land 0x00ff) lsl 8)) lsr 1)
  done;
  t

(* Fixed literal/length code lengths (288 bytes) *)
let flt =
  let t = U8.create 288 in
  for i = 0 to 143 do
    U8.set t i 8
  done;
  for i = 144 to 255 do
    U8.set t i 9
  done;
  for i = 256 to 279 do
    U8.set t i 7
  done;
  for i = 280 to 287 do
    U8.set t i 8
  done;
  t

(* Fixed distance code lengths (32 bytes) *)
let fdt =
  let t = U8.create 32 in
  for i = 0 to 31 do
    U8.set t i 5
  done;
  t

(* Deflate options per level 1..9: (nice << 13) | chain *)
let deo =
  I32.of_array [| 65540; 131080; 131088; 131104; 262176; 1048704; 1048832; 2114560; 2117632 |]

(* Reflected IEEE CRC-32 table (polynomial 0xEDB88320 = -306674912 in signed 32-bit int) *)
let crct =
  let t = I32.create 256 in
  for i = 0 to 255 do
    let c = ref i in
    for _ = 0 to 7 do
      let cur = !c in
      c := if cur land 1 <> 0 then -306674912 lxor (cur lsr 1) else cur lsr 1
    done;
    I32.set t i !c
  done;
  t
