(* noble's sha3.ts: Keccak-f[1600] and the Keccak sponge behind SHA3-256 and Keccak-256.

   The state is noble's state32: 25 lanes of two 32-bit words, the low word first. noble derives
   the pi, rho and iota tables at load time; they are written out here. *)

(* Pi: the word index each rho step moves its lane to (noble's SHA3_PI). *)
let sha3_pi = [| 20; 14; 22; 34; 36; 6; 10; 32; 16; 42; 48; 8; 30; 46; 38; 26; 24; 4; 40; 28; 44; 18; 12; 2 |]

(* Rho: the left rotation of each step (noble's SHA3_ROTL). *)
let sha3_rotl = [| 1; 3; 6; 10; 15; 21; 28; 36; 45; 55; 2; 14; 27; 41; 56; 8; 25; 43; 62; 18; 39; 61; 20; 44 |]

(* Iota: the low and high words of each round constant (noble's SHA3_IOTA_H and SHA3_IOTA_L). *)
let sha3_iota_h =
  [| 0x00000001l; 0x00008082l; 0x0000808al; 0x80008000l; 0x0000808bl; 0x80000001l; 0x80008081l; 0x00008009l;
     0x0000008al; 0x00000088l; 0x80008009l; 0x8000000al; 0x8000808bl; 0x0000008bl; 0x00008089l; 0x00008003l;
     0x00008002l; 0x00000080l; 0x0000800al; 0x8000000al; 0x80008081l; 0x00008080l; 0x80000001l; 0x80008008l |]

let sha3_iota_l =
  [| 0x00000000l; 0x00000000l; 0x80000000l; 0x80000000l; 0x00000000l; 0x00000000l; 0x80000000l; 0x80000000l;
     0x00000000l; 0x00000000l; 0x00000000l; 0x00000000l; 0x00000000l; 0x80000000l; 0x80000000l; 0x80000000l;
     0x80000000l; 0x80000000l; 0x00000000l; 0x80000000l; 0x80000000l; 0x80000000l; 0x00000000l; 0x80000000l |]

(* Left rotations of a lane by s in 1..31 (s) or 33..63 (b). As in noble, h is the lane's first
   (low) word and l its second (high) word. *)
let rotl_sh h l s = U32.((h lsl s) lor (l lsr (32 - s)))
let rotl_sl h l s = U32.((l lsl s) lor (h lsr (32 - s)))
let rotl_bh h l s = U32.((l lsl (s - 32)) lor (h lsr (64 - s)))
let rotl_bl h l s = U32.((h lsl (s - 32)) lor (l lsr (64 - s)))
let rotl_h h l s = if s > 32 then rotl_bh h l s else rotl_sh h l s
let rotl_l h l s = if s > 32 then rotl_bl h l s else rotl_sl h l s

(* The parity of five words, and chi's [~a & b]. *)
let xor5 a b c d e = U32.(a lxor b lxor c lxor d lxor e)
let andn a b = U32.(lnot a land b)

(* noble's keccakP: the 24 rounds of Keccak-f[1600] on [s], with [b] as the theta scratch. *)
let keccak_p s b =
  let xor i v = s.(i) <- U32.(s.(i) lxor v) in
  for round = 0 to 23 do
    (* Theta *)
    for x = 0 to 9 do
      b.(x) <- xor5 s.(x) s.(x + 10) s.(x + 20) s.(x + 30) s.(x + 40)
    done;
    for column = 0 to 4 do
      let x = 2 * column in
      let idx1 = (x + 8) mod 10 and idx0 = (x + 2) mod 10 in
      let b0 = b.(idx0) and b1 = b.(idx0 + 1) in
      let d0 = b.(idx1) and d1 = b.(idx1 + 1) in
      let th = U32.(rotl_h b0 b1 1 lxor d0) and tl = U32.(rotl_l b0 b1 1 lxor d1) in
      for row = 0 to 4 do
        xor (x + (10 * row)) th;
        xor (x + (10 * row) + 1) tl
      done
    done;
    (* Rho and pi *)
    let cur_h = ref s.(2) and cur_l = ref s.(3) in
    for t = 0 to 23 do
      let shift = sha3_rotl.(t) in
      let th = rotl_h !cur_h !cur_l shift and tl = rotl_l !cur_h !cur_l shift in
      let pi = sha3_pi.(t) in
      cur_h := s.(pi);
      cur_l := s.(pi + 1);
      s.(pi) <- th;
      s.(pi + 1) <- tl
    done;
    (* Chi *)
    for row = 0 to 4 do
      let y = 10 * row in
      let b0 = s.(y) and b1 = s.(y + 1) and b2 = s.(y + 2) and b3 = s.(y + 3) in
      for x = 0 to 5 do
        xor (y + x) (andn s.(y + x + 2) s.(y + x + 4))
      done;
      xor (y + 6) (andn s.(y + 8) b0);
      xor (y + 7) (andn s.(y + 9) b1);
      xor (y + 8) (andn b0 b2);
      xor (y + 9) (andn b1 b3)
    done;
    (* Iota *)
    xor 0 sha3_iota_h.(round);
    xor 1 sha3_iota_l.(round)
  done

(* noble's Keccak sponge with a byte-length rate [block_len] and domain [suffix]. *)
type t = {
  block_len : int;
  suffix : int;
  output_len : int;
  state32 : int32 array;
  theta : int32 array;
  mutable pos : int;
  mutable pos_out : int;
}

(* XORs [byte] into state byte [index], as noble does through its byte view of state32. *)
let xor_byte k index byte =
  let word = index / 4 and shift = 8 * (index mod 4) in
  k.state32.(word) <- U32.(k.state32.(word) lxor (of_int byte lsl shift))

(* The state byte at [index]. *)
let state_byte k index = Hash.byte_at k.state32.(index / 4) (8 * (index mod 4))

let keccak k =
  keccak_p k.state32 k.theta;
  k.pos_out <- 0;
  k.pos <- 0

let update k data =
  let len = Array.length data in
  let pos = ref 0 in
  while !pos < len do
    if k.pos = 0 && len - !pos >= k.block_len then begin
      (* Whole blocks with nothing buffered are absorbed a word at a time. *)
      for i = 0 to (k.block_len / 4) - 1 do
        k.state32.(i) <- Int32.logxor k.state32.(i) (Hash.get_uint32_le data (!pos + (4 * i)))
      done;
      pos := !pos + k.block_len;
      k.pos <- k.block_len;
      keccak k
    end
    else begin
      let take = min (k.block_len - k.pos) (len - !pos) in
      for _ = 1 to take do
        xor_byte k k.pos data.(!pos);
        k.pos <- k.pos + 1;
        incr pos
      done;
      if k.pos = k.block_len then keccak k
    end
  done

let finish k =
  (* The suffix holds the domain bits and the first padding bit; 0x80 closes pad10*1. *)
  xor_byte k k.pos k.suffix;
  if k.suffix land 0x80 <> 0 && k.pos = k.block_len - 1 then keccak k;
  xor_byte k (k.block_len - 1) 0x80;
  keccak k

let digest k =
  finish k;
  let out = Array.make k.output_len 0 in
  let pos = ref 0 in
  while !pos < k.output_len do
    if k.pos_out >= k.block_len then keccak k;
    let take = min (k.block_len - k.pos_out) (k.output_len - !pos) in
    for _ = 1 to take do
      out.(!pos) <- state_byte k k.pos_out;
      k.pos_out <- k.pos_out + 1;
      incr pos
    done
  done;
  out

let make ~block_len ~suffix ~output_len : Hash.t =
  let k =
    { block_len; suffix; output_len; state32 = Array.make 50 0l; theta = Array.make 10 0l; pos = 0; pos_out = 0 }
  in
  { Hash.block_len; output_len; update = update k; digest = (fun () -> digest k) }
