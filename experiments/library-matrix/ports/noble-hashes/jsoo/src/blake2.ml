(* noble's blake2.ts and _blake.ts: BLAKE2b and BLAKE2s on noble's shared _BLAKE2 base.

   BLAKE2b keeps noble's G1b/G2b on 64-bit words as low and high halves in a 32-word work vector
   (noble's BBUF); BLAKE2s keeps G1s/G2s on 32-bit words in a 16-word work vector, where noble
   passes and returns them as locals. The two do not share a mix function. *)

open U64

(* Message word order of each round (noble's BSIGMA); BLAKE2b's rounds 10 and 11 repeat 0 and 1. *)
let bsigma =
  [| 0; 1; 2; 3; 4; 5; 6; 7; 8; 9; 10; 11; 12; 13; 14; 15;
     14; 10; 4; 8; 9; 15; 13; 6; 1; 12; 0; 2; 11; 7; 5; 3;
     11; 8; 12; 0; 5; 2; 15; 13; 10; 14; 3; 6; 7; 1; 9; 4;
     7; 9; 3; 1; 13; 12; 11; 14; 2; 6; 5; 10; 4; 0; 15; 8;
     9; 0; 5; 7; 2; 4; 10; 15; 14; 1; 11; 12; 6; 8; 3; 13;
     2; 12; 6; 10; 0; 11; 8; 3; 4; 13; 7; 5; 15; 14; 1; 9;
     12; 5; 1; 15; 14; 13; 4; 10; 0; 7; 6; 3; 9; 2; 8; 11;
     13; 11; 7; 14; 12; 1; 3; 9; 5; 0; 15; 4; 8; 6; 2; 10;
     6; 15; 14; 9; 11; 3; 0; 8; 12; 2; 13; 7; 1; 4; 10; 5;
     10; 2; 8; 4; 7; 6; 1; 5; 15; 11; 9; 14; 3; 12; 13; 0;
     0; 1; 2; 3; 4; 5; 6; 7; 8; 9; 10; 11; 12; 13; 14; 15;
     14; 10; 4; 8; 9; 15; 13; 6; 1; 12; 0; 2; 11; 7; 5; 3 |]

(* noble's Blake2Opts: the options reference.mjs passes to every hash. *)
type opts = {
  dk_len : float option;
  key : int array option;
  salt : int array option;
  personalization : int array option;
}

(* noble's checkBlake2Opts: [key_len] bounds both the digest and the key length. *)
let check_blake2_opts output_len opts ~key_len ~salt_len ~pers_len =
  if output_len <= 0 || output_len > key_len then invalid_arg "\"dkLen\" out of range";
  (match opts.key with
   | Some key when Array.length key < 1 || Array.length key > key_len ->
     invalid_arg "\"key\" expected to be undefined or of a valid length"
   | _ -> ());
  (match opts.salt with
   | Some salt when Array.length salt <> salt_len -> invalid_arg "\"salt\" has the wrong length"
   | _ -> ());
  match opts.personalization with
  | Some pers when Array.length pers <> pers_len -> invalid_arg "\"personalization\" has the wrong length"
  | _ -> ()

(* noble's _BLAKE2: the final block is compressed with a flag, so a full buffer waits for more
   input. [compress] takes the block, its offset, the byte counter and the last-block flag; [get]
   returns the state as little-endian 32-bit words. *)
type t = {
  block_len : int;
  output_len : int;
  buffer : int array;
  mutable length : float;
  mutable pos : int;
  compress : int array -> int -> float -> bool -> unit;
  get : unit -> int32 array;
}

let update h data =
  let len = Array.length data in
  let pos = ref 0 in
  while !pos < len do
    if h.pos = h.block_len then begin
      h.compress h.buffer 0 h.length false;
      h.pos <- 0
    end;
    let take = min (h.block_len - h.pos) (len - !pos) in
    if take = h.block_len && !pos + take < len then
      (* Whole blocks that are not the last of the input are compressed straight from it. *)
      while !pos + h.block_len < len do
        h.length <- h.length +. float_of_int h.block_len;
        h.compress data !pos h.length false;
        pos := !pos + h.block_len
      done
    else begin
      Array.blit data !pos h.buffer h.pos take;
      h.pos <- h.pos + take;
      h.length <- h.length +. float_of_int take;
      pos := !pos + take
    end
  done

let digest h =
  Array.fill h.buffer h.pos (h.block_len - h.pos) 0;
  h.compress h.buffer 0 h.length true;
  Hash.words_to_bytes_le (h.get ()) h.output_len

(* The base's constructor; noble's subclass constructors end by absorbing the zero-padded key as
   the first block. *)
let make ~block_len ~output_len ~key ~compress ~get : Hash.t =
  let h = { block_len; output_len; buffer = Array.make block_len 0; length = 0.; pos = 0; compress; get } in
  Option.iter
    (fun key ->
      let block = Array.make block_len 0 in
      Array.blit key 0 block 0 (Array.length key);
      update h block)
    key;
  { Hash.block_len; output_len; update = update h; digest = (fun () -> digest h) }

(* The parameter block's first word, RFC 7693 §2.5: p[0] = 0x0101kknn. *)
let param_word output_len key =
  let key_length = Option.fold ~none:0 ~some:Array.length key in
  Int32.of_int (output_len lor (key_length lsl 8) lor (0x01 lsl 16) lor (0x01 lsl 24))

(* XORs the little-endian words of [bytes] into [state] from [at]. *)
let xor_words state at bytes =
  Option.iter
    (fun bytes ->
      for i = 0 to (Array.length bytes / 4) - 1 do
        state.(at + i) <- Int32.logxor state.(at + i) (Hash.get_uint32_le bytes (4 * i))
      done)
    bytes

(* The SHA-512 IV as little-endian low and high halves (noble's B2B_IV). *)
let b2b_iv =
  [| 0xf3bcc908l; 0x6a09e667l; 0x84caa73bl; 0xbb67ae85l; 0xfe94f82bl; 0x3c6ef372l; 0x5f1d36f1l; 0xa54ff53al;
     0xade682d1l; 0x510e527fl; 0x2b3e6c1fl; 0x9b05688cl; 0xfb41bd6bl; 0x1f83d9abl; 0x137e2179l; 0x5be0cd19l |]

(* G's first half for BLAKE2b on the work vector [v] with message word [x] of [m]:
   a = a + b + x, d = rotr(d ^ a, 32), c = c + d, b = rotr(b ^ c, 24). *)
let g1b v m a b c d x =
  let al = v.(2 * a) and ah = v.((2 * a) + 1) and bl = v.(2 * b) and bh = v.((2 * b) + 1) in
  let cl = v.(2 * c) and ch = v.((2 * c) + 1) and dl = v.(2 * d) and dh = v.((2 * d) + 1) in
  let xl = m.(x) and xh = m.(x + 1) in
  let ah, al = add3 ah al bh bl xh xl in
  let th = U32.(dh lxor ah) and tl = U32.(dl lxor al) in
  let dh = rotr32_h th tl and dl = rotr32_l th tl in
  let ch, cl = add ch cl dh dl in
  let th = U32.(bh lxor ch) and tl = U32.(bl lxor cl) in
  v.(2 * a) <- al;
  v.((2 * a) + 1) <- ah;
  v.(2 * b) <- rotr_sl th tl 24;
  v.((2 * b) + 1) <- rotr_sh th tl 24;
  v.(2 * c) <- cl;
  v.((2 * c) + 1) <- ch;
  v.(2 * d) <- dl;
  v.((2 * d) + 1) <- dh

(* G's second half for BLAKE2b: a = a + b + y, d = rotr(d ^ a, 16), c = c + d, b = rotr(b ^ c, 63). *)
let g2b v m a b c d x =
  let al = v.(2 * a) and ah = v.((2 * a) + 1) and bl = v.(2 * b) and bh = v.((2 * b) + 1) in
  let cl = v.(2 * c) and ch = v.((2 * c) + 1) and dl = v.(2 * d) and dh = v.((2 * d) + 1) in
  let xl = m.(x) and xh = m.(x + 1) in
  let ah, al = add3 ah al bh bl xh xl in
  let th = U32.(dh lxor ah) and tl = U32.(dl lxor al) in
  let dh = rotr_sh th tl 16 and dl = rotr_sl th tl 16 in
  let ch, cl = add ch cl dh dl in
  let th = U32.(bh lxor ch) and tl = U32.(bl lxor cl) in
  v.(2 * a) <- al;
  v.((2 * a) + 1) <- ah;
  v.(2 * b) <- rotr_bl th tl 63;
  v.((2 * b) + 1) <- rotr_bh th tl 63;
  v.(2 * c) <- cl;
  v.((2 * c) + 1) <- ch;
  v.(2 * d) <- dl;
  v.((2 * d) + 1) <- dh

(* noble's _BLAKE2b. *)
let blake2b opts =
  let output_len = Hash.anumber (Option.value opts.dk_len ~default:64.) "outputLen" in
  check_blake2_opts output_len opts ~key_len:64 ~salt_len:16 ~pers_len:16;
  (* v0l, v0h, ..., v7l, v7h; the parameter word goes into the low half of h[0]. *)
  let state = Array.copy b2b_iv in
  state.(0) <- U32.(state.(0) lxor param_word output_len opts.key);
  xor_words state 8 opts.salt;
  xor_words state 12 opts.personalization;
  let v = Array.make 32 0l and m = Array.make 32 0l in
  let compress data offset length is_last =
    for i = 0 to 31 do
      m.(i) <- Hash.get_uint32_le data (offset + (4 * i))
    done;
    Array.blit state 0 v 0 16;
    Array.blit b2b_iv 0 v 16 16;
    (* The byte counter goes into v12; v14 is inverted on the last block. *)
    v.(24) <- U32.(b2b_iv.(8) lxor from_num_l length);
    v.(25) <- U32.(b2b_iv.(9) lxor from_num_h length);
    if is_last then begin
      v.(28) <- Int32.lognot v.(28);
      v.(29) <- Int32.lognot v.(29)
    end;
    (* bsigma picks 64-bit message words; each spans two entries of m. *)
    for round = 0 to 11 do
      let s i = 2 * bsigma.((16 * round) + i) in
      g1b v m 0 4 8 12 (s 0);
      g2b v m 0 4 8 12 (s 1);
      g1b v m 1 5 9 13 (s 2);
      g2b v m 1 5 9 13 (s 3);
      g1b v m 2 6 10 14 (s 4);
      g2b v m 2 6 10 14 (s 5);
      g1b v m 3 7 11 15 (s 6);
      g2b v m 3 7 11 15 (s 7);
      g1b v m 0 5 10 15 (s 8);
      g2b v m 0 5 10 15 (s 9);
      g1b v m 1 6 11 12 (s 10);
      g2b v m 1 6 11 12 (s 11);
      g1b v m 2 7 8 13 (s 12);
      g2b v m 2 7 8 13 (s 13);
      g1b v m 3 4 9 14 (s 14);
      g2b v m 3 4 9 14 (s 15)
    done;
    for i = 0 to 15 do
      let high = v.(i + 16) in
      state.(i) <- U32.(state.(i) lxor v.(i) lxor high)
    done
  in
  make ~block_len:128 ~output_len ~key:opts.key ~compress ~get:(fun () -> state)

(* G's first half for BLAKE2s: a = a + b + x, d = rotr(d ^ a, 16), c = c + d, b = rotr(b ^ c, 12). *)
let g1s v a b c d x =
  v.(a) <- U32.(v.(a) + v.(b) + x);
  v.(d) <- U32.(rotr (v.(d) lxor v.(a)) 16);
  v.(c) <- U32.(v.(c) + v.(d));
  v.(b) <- U32.(rotr (v.(b) lxor v.(c)) 12)

(* G's second half for BLAKE2s: a = a + b + y, d = rotr(d ^ a, 8), c = c + d, b = rotr(b ^ c, 7). *)
let g2s v a b c d x =
  v.(a) <- U32.(v.(a) + v.(b) + x);
  v.(d) <- U32.(rotr (v.(d) lxor v.(a)) 8);
  v.(c) <- U32.(v.(c) + v.(d));
  v.(b) <- U32.(rotr (v.(b) lxor v.(c)) 7)

(* noble's _BLAKE2s, whose IV is SHA-256's. *)
let blake2s opts =
  let output_len = Hash.anumber (Option.value opts.dk_len ~default:32.) "outputLen" in
  check_blake2_opts output_len opts ~key_len:32 ~salt_len:8 ~pers_len:8;
  (* v0 ... v7. *)
  let iv = Sha2.sha256_iv in
  let state = Array.copy iv in
  state.(0) <- U32.(state.(0) lxor param_word output_len opts.key);
  xor_words state 4 opts.salt;
  xor_words state 6 opts.personalization;
  let v = Array.make 16 0l and m = Array.make 16 0l in
  let compress data offset length is_last =
    for i = 0 to 15 do
      m.(i) <- Hash.get_uint32_le data (offset + (4 * i))
    done;
    (* v8..v15 come from the IV, with the byte counter in v12 and v13 and v14 inverted last. *)
    Array.blit state 0 v 0 8;
    Array.blit iv 0 v 8 8;
    v.(12) <- U32.(v.(12) lxor from_num_l length);
    v.(13) <- U32.(v.(13) lxor from_num_h length);
    if is_last then v.(14) <- Int32.lognot v.(14);
    for round = 0 to 9 do
      let x i = m.(bsigma.((16 * round) + i)) in
      g1s v 0 4 8 12 (x 0);
      g2s v 0 4 8 12 (x 1);
      g1s v 1 5 9 13 (x 2);
      g2s v 1 5 9 13 (x 3);
      g1s v 2 6 10 14 (x 4);
      g2s v 2 6 10 14 (x 5);
      g1s v 3 7 11 15 (x 6);
      g2s v 3 7 11 15 (x 7);
      g1s v 0 5 10 15 (x 8);
      g2s v 0 5 10 15 (x 9);
      g1s v 1 6 11 12 (x 10);
      g2s v 1 6 11 12 (x 11);
      g1s v 2 7 8 13 (x 12);
      g2s v 2 7 8 13 (x 13);
      g1s v 3 4 9 14 (x 14);
      g2s v 3 4 9 14 (x 15)
    done;
    for i = 0 to 7 do
      let high = v.(i + 8) in
      state.(i) <- U32.(state.(i) lxor v.(i) lxor high)
    done
  in
  make ~block_len:64 ~output_len ~key:opts.key ~compress ~get:(fun () -> state)
