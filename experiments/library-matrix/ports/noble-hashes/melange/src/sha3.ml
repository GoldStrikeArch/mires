(* SHA3-256 and Keccak-256 from @noble/hashes 2.4.0 (MIT). *)

let pi = [|
  20; 14; 22; 34; 36; 6; 10; 32;
  16; 42; 48; 8; 30; 46; 38; 26;
  24; 4; 40; 28; 44; 18; 12; 2
|]
let rotations = [|
  1; 3; 6; 10; 15; 21; 28; 36;
  45; 55; 2; 14; 27; 41; 56; 8;
  25; 43; 62; 18; 39; 61; 20; 44
|]
let rc_lo = [|
  1; 32898; 32906; -2147450880; 32907; -2147483647; -2147450751; 32777;
  138; 136; -2147450871; -2147483638; -2147450741; 139; 32905; 32771;
  32770; 128; 32778; -2147483638; -2147450751; 32896; -2147483647; -2147450872
|]
let rc_hi = [|
  0; 0; -2147483648; -2147483648; 0; 0; -2147483648; -2147483648;
  0; 0; 0; 0; 0; -2147483648; -2147483648; -2147483648;
  -2147483648; -2147483648; 0; -2147483648; -2147483648; -2147483648; 0; -2147483648
|]

let permute s =
  let columns = Array.make 10 0 in
  for round = 0 to 23 do
    (* Theta mixes column parities into every lane. *)
    for x = 0 to 9 do
      columns.(x) <- s.(x) lxor s.(x+10) lxor s.(x+20) lxor s.(x+30) lxor s.(x+40)
    done;
    for col = 0 to 4 do
      let x = 2 * col in
      let left = (x+8) mod 10 and right = (x+2) mod 10 in
      let a = columns.(right) and b = columns.(right+1) in
      let lo = columns.(left) lxor ((a lsl 1) lor (b lsr 31)) in
      let hi = columns.(left+1) lxor ((b lsl 1) lor (a lsr 31)) in
      for row = 0 to 4 do
        let i = row*10+x in
        s.(i) <- s.(i) lxor lo;
        s.(i+1) <- s.(i+1) lxor hi
      done
    done;
    (* Rotate and permute the 24 non-origin lanes in one cycle. *)
    let lo = ref s.(2) and hi = ref s.(3) in
    for t = 0 to 23 do
      let shift = rotations.(t) in
      let a = if shift > 32 then !hi else !lo in
      let b = if shift > 32 then !lo else !hi in
      let k = shift land 31 and p = pi.(t) in
      lo := s.(p); hi := s.(p+1);
      s.(p) <- (a lsl k) lor (b lsr (32-k));
      s.(p+1) <- (b lsl k) lor (a lsr (32-k))
    done;
    (* Chi reads the original five lanes in each row. *)
    for row = 0 to 4 do
      let y = row * 10 in
      let b0 = s.(y) and b1 = s.(y+1) and b2 = s.(y+2) and b3 = s.(y+3) in
      for x = 0 to 5 do
        s.(y+x) <- s.(y+x) lxor ((lnot s.(y+x+2)) land s.(y+x+4))
      done;
      s.(y+6) <- s.(y+6) lxor ((lnot s.(y+8)) land b0);
      s.(y+7) <- s.(y+7) lxor ((lnot s.(y+9)) land b1);
      s.(y+8) <- s.(y+8) lxor ((lnot b0) land b2);
      s.(y+9) <- s.(y+9) lxor ((lnot b1) land b3)
    done;
    s.(0) <- s.(0) lxor rc_lo.(round);
    s.(1) <- s.(1) lxor rc_hi.(round)
  done

let digest legacy chunks =
  let s = Array.make 50 0 and pos = ref 0 in
  let xor_byte i byte =
    let word = i lsr 2 in
    s.(word) <- s.(word) lxor (byte lsl (8*(i land 3)))
  in
  Array.iter (fun chunk ->
    let i = ref 0 in
    let len = Array.length chunk in
    while !i < len do
      if !pos = 0 && len - !i >= 136 then begin
        for word = 0 to 33 do
          let j = !i + word*4 in
          let data = chunk.(j) lor (chunk.(j+1) lsl 8) lor (chunk.(j+2) lsl 16) lor (chunk.(j+3) lsl 24) in
          s.(word) <- s.(word) lxor data
        done;
        i := !i + 136;
        permute s
      end else begin
        xor_byte !pos chunk.(!i);
        incr pos;
        incr i;
        if !pos = 136 then begin permute s; pos := 0 end
      end
    done) chunks;
  xor_byte !pos (if legacy then 1 else 6);
  xor_byte 135 128;
  permute s;
  Array.init 32 (fun i -> (s.(i lsr 2) lsr (8*(i land 3))) land 255)
