(* MurmurHash3 x64 128, matching FingerprintJS 5.2.0 / murmurHash3.js. *)

open Js_of_ocaml
open Js_host

type x64 = {
  mutable high : int32;
  mutable low : int32;
}

let x64 high low = { high; low }

let add (m : x64) (n : x64) =
  let m0 = Int32.shift_right_logical m.high 16 in
  let m1 = Int32.logand m.high 0xffffl in
  let m2 = Int32.shift_right_logical m.low 16 in
  let m3 = Int32.logand m.low 0xffffl in
  let n0 = Int32.shift_right_logical n.high 16 in
  let n1 = Int32.logand n.high 0xffffl in
  let n2 = Int32.shift_right_logical n.low 16 in
  let n3 = Int32.logand n.low 0xffffl in
  let o3 = ref 0l in
  let o2 = ref 0l in
  let o1 = ref 0l in
  let o0 = ref 0l in
  o3 := Int32.add !o3 (Int32.add m3 n3);
  o2 := Int32.add !o2 (Int32.shift_right_logical !o3 16);
  o3 := Int32.logand !o3 0xffffl;
  o2 := Int32.add !o2 (Int32.add m2 n2);
  o1 := Int32.add !o1 (Int32.shift_right_logical !o2 16);
  o2 := Int32.logand !o2 0xffffl;
  o1 := Int32.add !o1 (Int32.add m1 n1);
  o0 := Int32.add !o0 (Int32.shift_right_logical !o1 16);
  o1 := Int32.logand !o1 0xffffl;
  o0 := Int32.add !o0 (Int32.add m0 n0);
  o0 := Int32.logand !o0 0xffffl;
  m.high <- Int32.logor (Int32.shift_left !o0 16) !o1;
  m.low <- Int32.logor (Int32.shift_left !o2 16) !o3

let multiply (m : x64) (n : x64) =
  let m0 = Int32.shift_right_logical m.high 16 in
  let m1 = Int32.logand m.high 0xffffl in
  let m2 = Int32.shift_right_logical m.low 16 in
  let m3 = Int32.logand m.low 0xffffl in
  let n0 = Int32.shift_right_logical n.high 16 in
  let n1 = Int32.logand n.high 0xffffl in
  let n2 = Int32.shift_right_logical n.low 16 in
  let n3 = Int32.logand n.low 0xffffl in
  let o3 = ref (Int32.mul m3 n3) in
  let o2 = ref (Int32.shift_right_logical !o3 16) in
  o3 := Int32.logand !o3 0xffffl;
  o2 := Int32.add !o2 (Int32.mul m2 n3);
  let o1 = ref (Int32.shift_right_logical !o2 16) in
  o2 := Int32.logand !o2 0xffffl;
  o2 := Int32.add !o2 (Int32.mul m3 n2);
  o1 := Int32.add !o1 (Int32.shift_right_logical !o2 16);
  o2 := Int32.logand !o2 0xffffl;
  o1 := Int32.add !o1 (Int32.mul m1 n3);
  let o0 = ref (Int32.shift_right_logical !o1 16) in
  o1 := Int32.logand !o1 0xffffl;
  o1 := Int32.add !o1 (Int32.mul m2 n2);
  o0 := Int32.add !o0 (Int32.shift_right_logical !o1 16);
  o1 := Int32.logand !o1 0xffffl;
  o1 := Int32.add !o1 (Int32.mul m3 n1);
  o0 := Int32.add !o0 (Int32.shift_right_logical !o1 16);
  o1 := Int32.logand !o1 0xffffl;
  o0 :=
    Int32.logand
      (Int32.add !o0
         (Int32.add (Int32.mul m0 n3)
            (Int32.add (Int32.mul m1 n2) (Int32.add (Int32.mul m2 n1) (Int32.mul m3 n0)))))
      0xffffl;
  m.high <- Int32.logor (Int32.shift_left !o0 16) !o1;
  m.low <- Int32.logor (Int32.shift_left !o2 16) !o3

let rotl (m : x64) (bits : int) =
  let bits = bits mod 64 in
  let m0 = m.high in
  if bits = 0 then ()
  else if bits = 32 then (
    m.high <- m.low;
    m.low <- m0)
  else if bits < 32 then (
    m.high <-
      Int32.logor (Int32.shift_left m0 bits) (Int32.shift_right_logical m.low (32 - bits));
    m.low <-
      Int32.logor (Int32.shift_left m.low bits) (Int32.shift_right_logical m0 (32 - bits)))
  else
    let bits = bits - 32 in
    m.high <-
      Int32.logor (Int32.shift_left m.low bits) (Int32.shift_right_logical m0 (32 - bits));
    m.low <-
      Int32.logor (Int32.shift_left m0 bits) (Int32.shift_right_logical m.low (32 - bits))

let xor (m : x64) (n : x64) =
  m.high <- Int32.logxor m.high n.high;
  m.low <- Int32.logxor m.low n.low

let xor_byte (m : x64) (byte : int) (shift : int) =
  let value = Int32.of_int byte in
  if shift < 32 then m.low <- Int32.logxor m.low (Int32.shift_left value shift)
  else m.high <- Int32.logxor m.high (Int32.shift_left value (shift - 32))

let f1 = x64 0xff51afd7l 0xed558ccdl
let f2 = x64 0xc4ceb9fel 0x1a85ec53l
let c1 = x64 0x87c37b91l 0x114253d5l
let c2 = x64 0x4cf5ad43l 0x2745937fl
let m5 = x64 0l 5l
let n1 = x64 0l 0x52dce729l
let n2 = x64 0l 0x38495ab5l

let fmix (h : x64) =
  xor h (x64 0l (Int32.shift_right_logical h.high 1));
  multiply h f1;
  xor h (x64 0l (Int32.shift_right_logical h.high 1));
  multiply h f2;
  xor h (x64 0l (Int32.shift_right_logical h.high 1))

let u32_le (key : int array) i =
  Int32.logor (Int32.of_int key.(i))
    (Int32.logor
       (Int32.shift_left (Int32.of_int key.(i + 1)) 8)
       (Int32.logor
          (Int32.shift_left (Int32.of_int key.(i + 2)) 16)
          (Int32.shift_left (Int32.of_int key.(i + 3)) 24)))

let hex32 (n : int32) = Printf.sprintf "%08lx" n

let x64hash128_bytes (key : int array) (seed : int32) : string =
  let length_val = x64 0l (Int32.of_int (Array.length key)) in
  let remainder = Array.length key mod 16 in
  let bytes = Array.length key - remainder in
  let h1 = x64 0l seed in
  let h2 = x64 0l seed in
  let i = ref 0 in
  while !i < bytes do
    let k1 = x64 (u32_le key (!i + 4)) (u32_le key !i) in
    let k2 = x64 (u32_le key (!i + 12)) (u32_le key (!i + 8)) in
    multiply k1 c1;
    rotl k1 31;
    multiply k1 c2;
    xor h1 k1;
    rotl h1 27;
    add h1 h2;
    multiply h1 m5;
    add h1 n1;
    multiply k2 c2;
    rotl k2 33;
    multiply k2 c1;
    xor h2 k2;
    rotl h2 31;
    add h2 h1;
    multiply h2 m5;
    add h2 n2;
    i := !i + 16
  done;
  let tail_off = bytes in
  let k1 = x64 0l 0l in
  let k2 = x64 0l 0l in
  for offset = 0 to min 7 (remainder - 1) do
    xor_byte k1 key.(tail_off + offset) (offset * 8)
  done;
  for offset = 0 to remainder - 9 do
    xor_byte k2 key.(tail_off + 8 + offset) (offset * 8)
  done;
  if remainder > 8 then (
    multiply k2 c2;
    rotl k2 33;
    multiply k2 c1;
    xor h2 k2);
  if remainder > 0 then (
    multiply k1 c1;
    rotl k1 31;
    multiply k1 c2;
    xor h1 k1);
  xor h1 length_val;
  xor h2 length_val;
  add h1 h2;
  add h2 h1;
  fmix h1;
  fmix h2;
  add h1 h2;
  add h2 h1;
  hex32 h1.high ^ hex32 h1.low ^ hex32 h2.high ^ hex32 h2.low

let bytes_of_ocaml_string (s : string) =
  Array.init (String.length s) (fun i -> Char.code s.[i])

let x64hash128_ocaml ?(seed = 0l) (s : string) =
  x64hash128_bytes (bytes_of_ocaml_string s) seed

let get_utf8_bytes (input : js) : int array =
  let s = Js.Unsafe.coerce input in
  let len = s##.length in
  let rec all_ascii i =
    if i >= len then true
    else
      let code = int_of_float (Js.to_float (s##charCodeAt i)) in
      if code > 127 then false else all_ascii (i + 1)
  in
  if all_ascii 0 then
    Array.init len (fun i -> int_of_float (Js.to_float (s##charCodeAt i)))
  else
    let encoded = text_encoder_encode input in
    let n = array_length encoded in
    Array.init n (fun i -> int_of_float (to_float (array_get encoded i)))

let x64hash128_js (input : js) (seed : int32) : string =
  x64hash128_bytes (get_utf8_bytes input) seed

let murmur_x64_hash128 (args : js array) : js =
  let input = arg args 0 in
  let seed_arg = arg args 1 in
  let seed =
    if is_undefined seed_arg then 0l else Int32.of_float (to_float seed_arg)
  in
  js_string (x64hash128_js input seed)
