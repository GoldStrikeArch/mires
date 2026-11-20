(* noble's Hash, and the parts of its utils.ts the selected hashes use. Bytes are int arrays of
   values in 0..255; words are int32. *)

(* An incremental hash with its block and digest lengths. *)
type t = {
  block_len : int;
  output_len : int;
  update : int array -> unit;
  digest : unit -> int array;
}

(* noble's anumber: [n] must be a safe integer >= 0. Values above the int range saturate. *)
let anumber n title =
  if n >= 0. && n <= 9007199254740991. && Float.is_integer n then
    if n > 2147483647. then 2147483647 else int_of_float n
  else invalid_arg ("\"" ^ title ^ "\" expected integer >= 0")

(* DataView.getUint32(offset, false). *)
let get_uint32_be data offset =
  let byte i = Int32.of_int data.(offset + i) in
  U32.((byte 0 lsl 24) lor (byte 1 lsl 16) lor (byte 2 lsl 8) lor byte 3)

(* A little-endian 32-bit word, as noble's Uint32Array views read on little-endian hosts. *)
let get_uint32_le data offset =
  let byte i = Int32.of_int data.(offset + i) in
  U32.(byte 0 lor (byte 1 lsl 8) lor (byte 2 lsl 16) lor (byte 3 lsl 24))

(* The byte of [word] at bit [shift]. *)
let byte_at word shift = Int32.to_int U32.((word lsr shift) land 0xffl)

(* DataView.setUint32(offset, value, false). *)
let set_uint32_be data offset value =
  for i = 0 to 3 do
    data.(offset + i) <- byte_at value (24 - (8 * i))
  done

(* The first [count] bytes of the little-endian encoding of [words]. *)
let words_to_bytes_le words count = Array.init count (fun i -> byte_at words.(i / 4) (8 * (i mod 4)))

(* noble's bytesToHex. *)
let bytes_to_hex bytes =
  let digits = "0123456789abcdef" in
  String.init (2 * Array.length bytes) (fun i ->
      let b = bytes.(i / 2) in
      digits.[if i mod 2 = 0 then b lsr 4 else b land 15])
