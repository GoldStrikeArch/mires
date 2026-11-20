(* BLAKE2s and BLAKE2b from @noble/hashes 2.4.0. The wide variant uses low/high limbs. *)
type word = { lo : int32; hi : int32 }

let iv_low = [| 0xf3bcc908l; 0x84caa73bl; 0xfe94f82bl; 0x5f1d36f1l; 0xade682d1l; 0x2b3e6c1fl; 0xfb41bd6bl; 0x137e2179l |]
let iv_high = [| 0x6a09e667l; 0xbb67ae85l; 0x3c6ef372l; 0xa54ff53al; 0x510e527fl; 0x9b05688cl; 0x1f83d9abl; 0x5be0cd19l |]
let sigma = [|
  [| 0; 1; 2; 3; 4; 5; 6; 7; 8; 9; 10; 11; 12; 13; 14; 15 |];
  [| 14; 10; 4; 8; 9; 15; 13; 6; 1; 12; 0; 2; 11; 7; 5; 3 |];
  [| 11; 8; 12; 0; 5; 2; 15; 13; 10; 14; 3; 6; 7; 1; 9; 4 |];
  [| 7; 9; 3; 1; 13; 12; 11; 14; 2; 6; 5; 10; 4; 0; 15; 8 |];
  [| 9; 0; 5; 7; 2; 4; 10; 15; 14; 1; 11; 12; 6; 8; 3; 13 |];
  [| 2; 12; 6; 10; 0; 11; 8; 3; 4; 13; 7; 5; 15; 14; 1; 9 |];
  [| 12; 5; 1; 15; 14; 13; 4; 10; 0; 7; 6; 3; 9; 2; 8; 11 |];
  [| 13; 11; 7; 14; 12; 1; 3; 9; 5; 0; 15; 4; 8; 6; 2; 10 |];
  [| 6; 15; 14; 9; 11; 3; 0; 8; 12; 2; 13; 7; 1; 4; 10; 5 |];
  [| 10; 2; 8; 4; 7; 6; 1; 5; 15; 11; 9; 14; 3; 12; 13; 0 |]
|]

let zero = { lo = 0l; hi = 0l }

let add a b =
  let lo = Int32.add a.lo b.lo in
  let carry = if Int32.unsigned_compare lo a.lo < 0 then 1l else 0l in
  { lo; hi = Int32.add (Int32.add a.hi b.hi) carry }

let xor a b = { lo = Int32.logxor a.lo b.lo; hi = Int32.logxor a.hi b.hi }

let rotate wide w n =
  let combine lo hi n =
    Int32.logor (Int32.shift_right_logical lo n) (Int32.shift_left hi (32 - n)) in
  if not wide then { lo = combine w.lo w.lo n; hi = 0l }
  else if n = 32 then { lo = w.hi; hi = w.lo }
  else
    let lo, hi, bits = if n > 32 then w.hi, w.lo, n - 32 else w.lo, w.hi, n in
    { lo = combine lo hi bits; hi = combine hi lo bits }

let initial wide i =
  if wide then { lo = iv_low.(i); hi = iv_high.(i) } else { lo = iv_high.(i); hi = 0l }

let load32 bytes pos =
  let limb n = Int32.shift_left (Int32.of_int bytes.(pos+n)) (8*n) in
  Int32.logor (Int32.logor (limb 0) (limb 1)) (Int32.logor (limb 2) (limb 3))

let load wide bytes i =
  if wide then { lo = load32 bytes (8*i); hi = load32 bytes (8*i+4) }
  else { lo = load32 bytes (4*i); hi = 0l }

let mix wide v a b c d x y =
  let r1,r2,r3,r4 = if wide then 32,24,16,63 else 16,12,8,7 in
  v.(a) <- add (add v.(a) v.(b)) x;
  v.(d) <- rotate wide (xor v.(d) v.(a)) r1;
  v.(c) <- add v.(c) v.(d);
  v.(b) <- rotate wide (xor v.(b) v.(c)) r2;
  v.(a) <- add (add v.(a) v.(b)) y;
  v.(d) <- rotate wide (xor v.(d) v.(a)) r3;
  v.(c) <- add v.(c) v.(d);
  v.(b) <- rotate wide (xor v.(b) v.(c)) r4

let compress wide h block count last =
  let v = Array.init 16 (fun i -> if i < 8 then h.(i) else initial wide (i-8)) in
  let msg = Array.init 16 (load wide block) in
  v.(12) <- xor v.(12) { lo = count.lo; hi = if wide then count.hi else 0l };
  if not wide then v.(13) <- xor v.(13) { zero with lo = count.hi };
  if last then v.(14) <- xor v.(14) { lo = -1l; hi = -1l };
  let rounds = if wide then 12 else 10 in
  for r = 0 to rounds - 1 do
    let s = sigma.(r mod 10) in
    for i = 0 to 3 do
      mix wide v i (i+4) (i+8) (i+12) msg.(s.(2*i)) msg.(s.(2*i+1))
    done;
    for i = 0 to 3 do
      mix wide v i (4+(i+1) mod 4) (8+(i+2) mod 4) (12+(i+3) mod 4)
        msg.(s.(8+2*i)) msg.(s.(9+2*i))
    done
  done;
  for i = 0 to 7 do h.(i) <- xor h.(i) (xor v.(i) v.(i+8)) done

let digest wide chunks dk_len key salt personalization =
  let stride = if wide then 8 else 4 in
  let block_len = 16 * stride in
  let h = Array.init 8 (initial wide) in
  let param = 0x01010000 lor (Array.length key lsl 8) lor dk_len in
  h.(0) <- xor h.(0) { zero with lo = Int32.of_int param };
  for i = 0 to 1 do
    if Array.length salt > 0 then h.(4+i) <- xor h.(4+i) (load wide salt i);
    if Array.length personalization > 0 then h.(6+i) <- xor h.(6+i) (load wide personalization i)
  done;
  let block = Array.make block_len 0 in
  Array.blit key 0 block 0 (Array.length key);
  let pos = ref (if Array.length key > 0 then block_len else 0) in
  let count = ref { zero with lo = Int32.of_int !pos } in
  Array.iter (fun chunk ->
    let at = ref 0 in
    while !at < Array.length chunk do
      if !pos = block_len then (compress wide h block !count false; pos := 0);
      let take = min (block_len - !pos) (Array.length chunk - !at) in
      Array.blit chunk !at block !pos take;
      count := add !count { zero with lo = Int32.of_int take };
      pos := !pos + take;
      at := !at + take
    done
  ) chunks;
  Array.fill block !pos (block_len - !pos) 0;
  compress wide h block !count true;
  Array.init dk_len (fun i ->
    let w = h.(i / stride) in
    let limb = if i mod stride >= 4 then w.hi else w.lo in
    Int32.to_int (Int32.shift_right_logical limb (8 * (i mod 4))) land 255)
