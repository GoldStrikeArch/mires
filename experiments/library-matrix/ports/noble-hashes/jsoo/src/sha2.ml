(* noble's sha2.ts: SHA-256 (SHA2_32B) and SHA-512 (SHA2_64B), with the IVs from _md.ts. *)

open U64

let sha256_iv =
  [| 0x6a09e667l; 0xbb67ae85l; 0x3c6ef372l; 0xa54ff53al; 0x510e527fl; 0x9b05688cl; 0x1f83d9abl; 0x5be0cd19l |]

let sha256_k =
  [| 0x428a2f98l; 0x71374491l; 0xb5c0fbcfl; 0xe9b5dba5l; 0x3956c25bl; 0x59f111f1l; 0x923f82a4l; 0xab1c5ed5l;
     0xd807aa98l; 0x12835b01l; 0x243185bel; 0x550c7dc3l; 0x72be5d74l; 0x80deb1fel; 0x9bdc06a7l; 0xc19bf174l;
     0xe49b69c1l; 0xefbe4786l; 0x0fc19dc6l; 0x240ca1ccl; 0x2de92c6fl; 0x4a7484aal; 0x5cb0a9dcl; 0x76f988dal;
     0x983e5152l; 0xa831c66dl; 0xb00327c8l; 0xbf597fc7l; 0xc6e00bf3l; 0xd5a79147l; 0x06ca6351l; 0x14292967l;
     0x27b70a85l; 0x2e1b2138l; 0x4d2c6dfcl; 0x53380d13l; 0x650a7354l; 0x766a0abbl; 0x81c2c92el; 0x92722c85l;
     0xa2bfe8a1l; 0xa81a664bl; 0xc24b8b70l; 0xc76c51a3l; 0xd192e819l; 0xd6990624l; 0xf40e3585l; 0x106aa070l;
     0x19a4c116l; 0x1e376c08l; 0x2748774cl; 0x34b0bcb5l; 0x391c0cb3l; 0x4ed8aa4al; 0x5b9cca4fl; 0x682e6ff3l;
     0x748f82eel; 0x78a5636fl; 0x84c87814l; 0x8cc70208l; 0x90befffal; 0xa4506cebl; 0xbef9a3f7l; 0xc67178f2l |]

(* noble's SHA2_32B: [state] holds A ... H, [w] the message schedule (noble shares one
   module-level SHA256_W). *)
let process256 state w data offset =
  for i = 0 to 15 do
    w.(i) <- Hash.get_uint32_be data (offset + (4 * i))
  done;
  for i = 16 to 63 do
    let w15 = w.(i - 15) and w2 = w.(i - 2) in
    let s0 = U32.(rotr w15 7 lxor rotr w15 18 lxor (w15 lsr 3)) in
    let s1 = U32.(rotr w2 17 lxor rotr w2 19 lxor (w2 lsr 10)) in
    w.(i) <- U32.(s1 + w.(i - 7) + s0 + w.(i - 16))
  done;
  let a = ref state.(0) and b = ref state.(1) and c = ref state.(2) and d = ref state.(3) in
  let e = ref state.(4) and f = ref state.(5) and g = ref state.(6) and h = ref state.(7) in
  for i = 0 to 63 do
    let sigma1 = U32.(rotr !e 6 lxor rotr !e 11 lxor rotr !e 25) in
    let t1 = U32.(!h + sigma1 + Md.chi !e !f !g + sha256_k.(i) + w.(i)) in
    let sigma0 = U32.(rotr !a 2 lxor rotr !a 13 lxor rotr !a 22) in
    let t2 = U32.(sigma0 + Md.maj !a !b !c) in
    h := !g;
    g := !f;
    f := !e;
    e := U32.(!d + t1);
    d := !c;
    c := !b;
    b := !a;
    a := U32.(t1 + t2)
  done;
  List.iteri (fun i word -> state.(i) <- U32.(state.(i) + word)) [ !a; !b; !c; !d; !e; !f; !g; !h ]

let sha256 () =
  let state = Array.copy sha256_iv and w = Array.make 64 0l in
  Md.to_hash
    (Md.make ~block_len:64 ~output_len:32 ~pad_offset:8 ~process:(process256 state w) ~get:(fun () -> state))

let sha512_iv =
  [| 0x6a09e667l; 0xf3bcc908l; 0xbb67ae85l; 0x84caa73bl; 0x3c6ef372l; 0xfe94f82bl; 0xa54ff53al; 0x5f1d36f1l;
     0x510e527fl; 0xade682d1l; 0x9b05688cl; 0x2b3e6c1fl; 0x1f83d9abl; 0xfb41bd6bl; 0x5be0cd19l; 0x137e2179l |]

(* The 80 round constants of RFC 6234 §5.2, split into high and low halves as noble's K512 is. *)
let sha512_kh =
  [| 0x428a2f98l; 0x71374491l; 0xb5c0fbcfl; 0xe9b5dba5l; 0x3956c25bl; 0x59f111f1l; 0x923f82a4l; 0xab1c5ed5l;
     0xd807aa98l; 0x12835b01l; 0x243185bel; 0x550c7dc3l; 0x72be5d74l; 0x80deb1fel; 0x9bdc06a7l; 0xc19bf174l;
     0xe49b69c1l; 0xefbe4786l; 0x0fc19dc6l; 0x240ca1ccl; 0x2de92c6fl; 0x4a7484aal; 0x5cb0a9dcl; 0x76f988dal;
     0x983e5152l; 0xa831c66dl; 0xb00327c8l; 0xbf597fc7l; 0xc6e00bf3l; 0xd5a79147l; 0x06ca6351l; 0x14292967l;
     0x27b70a85l; 0x2e1b2138l; 0x4d2c6dfcl; 0x53380d13l; 0x650a7354l; 0x766a0abbl; 0x81c2c92el; 0x92722c85l;
     0xa2bfe8a1l; 0xa81a664bl; 0xc24b8b70l; 0xc76c51a3l; 0xd192e819l; 0xd6990624l; 0xf40e3585l; 0x106aa070l;
     0x19a4c116l; 0x1e376c08l; 0x2748774cl; 0x34b0bcb5l; 0x391c0cb3l; 0x4ed8aa4al; 0x5b9cca4fl; 0x682e6ff3l;
     0x748f82eel; 0x78a5636fl; 0x84c87814l; 0x8cc70208l; 0x90befffal; 0xa4506cebl; 0xbef9a3f7l; 0xc67178f2l;
     0xca273ecel; 0xd186b8c7l; 0xeada7dd6l; 0xf57d4f7fl; 0x06f067aal; 0x0a637dc5l; 0x113f9804l; 0x1b710b35l;
     0x28db77f5l; 0x32caab7bl; 0x3c9ebe0al; 0x431d67c4l; 0x4cc5d4bel; 0x597f299cl; 0x5fcb6fabl; 0x6c44198cl |]

let sha512_kl =
  [| 0xd728ae22l; 0x23ef65cdl; 0xec4d3b2fl; 0x8189dbbcl; 0xf348b538l; 0xb605d019l; 0xaf194f9bl; 0xda6d8118l;
     0xa3030242l; 0x45706fbel; 0x4ee4b28cl; 0xd5ffb4e2l; 0xf27b896fl; 0x3b1696b1l; 0x25c71235l; 0xcf692694l;
     0x9ef14ad2l; 0x384f25e3l; 0x8b8cd5b5l; 0x77ac9c65l; 0x592b0275l; 0x6ea6e483l; 0xbd41fbd4l; 0x831153b5l;
     0xee66dfabl; 0x2db43210l; 0x98fb213fl; 0xbeef0ee4l; 0x3da88fc2l; 0x930aa725l; 0xe003826fl; 0x0a0e6e70l;
     0x46d22ffcl; 0x5c26c926l; 0x5ac42aedl; 0x9d95b3dfl; 0x8baf63del; 0x3c77b2a8l; 0x47edaee6l; 0x1482353bl;
     0x4cf10364l; 0xbc423001l; 0xd0f89791l; 0x0654be30l; 0xd6ef5218l; 0x5565a910l; 0x5771202al; 0x32bbd1b8l;
     0xb8d2d0c8l; 0x5141ab53l; 0xdf8eeb99l; 0xe19b48a8l; 0xc5c95a63l; 0xe3418acbl; 0x7763e373l; 0xd6b2b8a3l;
     0x5defb2fcl; 0x43172f60l; 0xa1f0ab72l; 0x1a6439ecl; 0x23631e28l; 0xde82bde9l; 0xb2c67915l; 0xe372532bl;
     0xea26619cl; 0x21c0c207l; 0xcde0eb1el; 0xee6ed178l; 0x72176fbal; 0xa2c898a6l; 0xbef90dael; 0x131c471bl;
     0x23047d84l; 0x40c72493l; 0x15c9bebcl; 0x9c100d4cl; 0xcb3e42b6l; 0xfc657e2al; 0x3ad6faecl; 0x4a475817l |]

(* noble's SHA2_64B: [state] holds Ah, Al, ..., Hh, Hl; [wh] and [wl] the halves of the message
   schedule (noble's SHA512_W_H and SHA512_W_L). *)
let process512 state wh wl data offset =
  for i = 0 to 15 do
    wh.(i) <- Hash.get_uint32_be data (offset + (8 * i));
    wl.(i) <- Hash.get_uint32_be data (offset + (8 * i) + 4)
  done;
  for i = 16 to 79 do
    (* s0 = rotr(w[i-15], 1) ^ rotr(w[i-15], 8) ^ (w[i-15] >>> 7) *)
    let w15h = wh.(i - 15) and w15l = wl.(i - 15) in
    let s0h = U32.(rotr_sh w15h w15l 1 lxor rotr_sh w15h w15l 8 lxor shr_sh w15h w15l 7) in
    let s0l = U32.(rotr_sl w15h w15l 1 lxor rotr_sl w15h w15l 8 lxor shr_sl w15h w15l 7) in
    (* s1 = rotr(w[i-2], 19) ^ rotr(w[i-2], 61) ^ (w[i-2] >>> 6) *)
    let w2h = wh.(i - 2) and w2l = wl.(i - 2) in
    let s1h = U32.(rotr_sh w2h w2l 19 lxor rotr_bh w2h w2l 61 lxor shr_sh w2h w2l 6) in
    let s1l = U32.(rotr_sl w2h w2l 19 lxor rotr_bl w2h w2l 61 lxor shr_sl w2h w2l 6) in
    let sum_h, sum_l = add4 s0h s0l s1h s1l wh.(i - 7) wl.(i - 7) wh.(i - 16) wl.(i - 16) in
    wh.(i) <- sum_h;
    wl.(i) <- sum_l
  done;
  let ah = ref state.(0) and al = ref state.(1) and bh = ref state.(2) and bl = ref state.(3) in
  let ch = ref state.(4) and cl = ref state.(5) and dh = ref state.(6) and dl = ref state.(7) in
  let eh = ref state.(8) and el = ref state.(9) and fh = ref state.(10) and fl = ref state.(11) in
  let gh = ref state.(12) and gl = ref state.(13) and hh = ref state.(14) and hl = ref state.(15) in
  for i = 0 to 79 do
    (* S1 = rotr(e, 14) ^ rotr(e, 18) ^ rotr(e, 41) *)
    let sigma1h = U32.(rotr_sh !eh !el 14 lxor rotr_sh !eh !el 18 lxor rotr_bh !eh !el 41) in
    let sigma1l = U32.(rotr_sl !eh !el 14 lxor rotr_sl !eh !el 18 lxor rotr_bl !eh !el 41) in
    (* T1 = H + S1 + Chi(E, F, G) + K[i] + W[i] *)
    let t1h, t1l =
      add5 !hh !hl sigma1h sigma1l (Md.chi !eh !fh !gh) (Md.chi !el !fl !gl) sha512_kh.(i) sha512_kl.(i)
        wh.(i) wl.(i)
    in
    (* S0 = rotr(a, 28) ^ rotr(a, 34) ^ rotr(a, 39) *)
    let sigma0h = U32.(rotr_sh !ah !al 28 lxor rotr_bh !ah !al 34 lxor rotr_bh !ah !al 39) in
    let sigma0l = U32.(rotr_sl !ah !al 28 lxor rotr_bl !ah !al 34 lxor rotr_bl !ah !al 39) in
    let maj_h = Md.maj !ah !bh !ch and maj_l = Md.maj !al !bl !cl in
    hh := !gh;
    hl := !gl;
    gh := !fh;
    gl := !fl;
    fh := !eh;
    fl := !el;
    let next_eh, next_el = add !dh !dl t1h t1l in
    eh := next_eh;
    el := next_el;
    dh := !ch;
    dl := !cl;
    ch := !bh;
    cl := !bl;
    bh := !ah;
    bl := !al;
    let next_ah, next_al = add3 t1h t1l sigma0h sigma0l maj_h maj_l in
    ah := next_ah;
    al := next_al
  done;
  (* Add the compressed block to the state. *)
  List.iteri
    (fun i (h, l) ->
      let sum_h, sum_l = add state.(2 * i) state.((2 * i) + 1) h l in
      state.(2 * i) <- sum_h;
      state.((2 * i) + 1) <- sum_l)
    [ (!ah, !al); (!bh, !bl); (!ch, !cl); (!dh, !dl); (!eh, !el); (!fh, !fl); (!gh, !gl); (!hh, !hl) ]

let sha512 () =
  let state = Array.copy sha512_iv and wh = Array.make 80 0l and wl = Array.make 80 0l in
  Md.to_hash
    (Md.make ~block_len:128 ~output_len:64 ~pad_offset:16 ~process:(process512 state wh wl)
       ~get:(fun () -> state))
