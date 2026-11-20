(* SHA-256 and SHA-512 as in @noble/hashes 2.4.0. A 64-bit word consists of two 32-bit lanes. *)
let iv256 = [|
  1779033703; -1150833019; 1013904242; -1521486534; 1359893119; -1694144372; 528734635; 1541459225;
|]

let iv512 = [|
  1779033703; -205731576; -1150833019; -2067093701; 1013904242; -23791573; -1521486534; 1595750129;
  1359893119; -1377402159; -1694144372; 725511199; 528734635; -79577749; 1541459225; 327033209;
|]

let k256 = [|
  1116352408; 1899447441; -1245643825; -373957723; 961987163; 1508970993; -1841331548; -1424204075;
  -670586216; 310598401; 607225278; 1426881987; 1925078388; -2132889090; -1680079193; -1046744716;
  -459576895; -272742522; 264347078; 604807628; 770255983; 1249150122; 1555081692; 1996064986;
  -1740746414; -1473132947; -1341970488; -1084653625; -958395405; -710438585; 113926993; 338241895;
  666307205; 773529912; 1294757372; 1396182291; 1695183700; 1986661051; -2117940946; -1838011259;
  -1564481375; -1474664885; -1035236496; -949202525; -778901479; -694614492; -200395387; 275423344;
  430227734; 506948616; 659060556; 883997877; 958139571; 1322822218; 1537002063; 1747873779;
  1955562222; 2024104815; -2067236844; -1933114872; -1866530822; -1538233109; -1090935817; -965641998;
|]

let k512h = [|
  1116352408; 1899447441; -1245643825; -373957723; 961987163; 1508970993; -1841331548; -1424204075;
  -670586216; 310598401; 607225278; 1426881987; 1925078388; -2132889090; -1680079193; -1046744716;
  -459576895; -272742522; 264347078; 604807628; 770255983; 1249150122; 1555081692; 1996064986;
  -1740746414; -1473132947; -1341970488; -1084653625; -958395405; -710438585; 113926993; 338241895;
  666307205; 773529912; 1294757372; 1396182291; 1695183700; 1986661051; -2117940946; -1838011259;
  -1564481375; -1474664885; -1035236496; -949202525; -778901479; -694614492; -200395387; 275423344;
  430227734; 506948616; 659060556; 883997877; 958139571; 1322822218; 1537002063; 1747873779;
  1955562222; 2024104815; -2067236844; -1933114872; -1866530822; -1538233109; -1090935817; -965641998;
  -903397682; -779700025; -354779690; -176337025; 116418474; 174292421; 289380356; 460393269;
  685471733; 852142971; 1017036298; 1126000580; 1288033470; 1501505948; 1607167915; 1816402316;
|]

let k512l = [|
  -685199838; 602891725; -330482897; -2121671748; -213338824; -1241133031; -1357295717; -630357736;
  -1560083902; 1164996542; 1323610764; -704662302; -226784913; 991336113; 633803317; -815192428;
  -1628353838; 944711139; -1953704523; 2007800933; 1495990901; 1856431235; -1119749164; -2096016459;
  -295247957; 766784016; -1728372417; -1091629340; 1034457026; -1828018395; -536640913; 168717936;
  1188179964; 1546045734; 1522805485; -1651133473; -1951439906; 1014477480; 1206759142; 344077627;
  1290863460; -1136513023; -789014639; 106217008; -688958952; 1432725776; 1467031594; 851169720;
  -1194143544; 1363258195; -544281703; -509917016; -976659869; -482243893; 2003034995; -692930397;
  1575990012; 1125592928; -1578062990; 442776044; 593698344; -561857047; -1295615723; -479046869;
  -366583396; 566280711; -840897762; -294727304; 1914138554; -1563912026; -1090974290; 320620315;
  587496836; 1086792851; 365543100; -1676669620; -885112138; -60457430; 987167468; 1246189591;
|]

let band x n = x land n
let bor x n = x lor n
let bxor x n = x lxor n
let shl x n = x lsl n
let shr x n = x lsr n
let inv = lnot
let ror x n = (bor (shr x n) (shl x (32 - n)))
let choice a b c = (bxor (band a b) (band (inv a) c))
let majority a b c = (bxor (bxor (band a b) (band a c)) (band b c))
let carry a b = (shr (bor (band a b) (band (bor a b) (inv (a + b)))) 31)
let readBe data at = (bor (bor (shl data.(at) 24) (shl (data.(at + 1)) 16)) (bor (shl (data.(at + 2)) 8) (data.(at + 3))))
let compress256 state data at wh =
  for i = 0 to 15 do
    wh.(i) <- (readBe data (at + i * 4));
  done;
  for i = 16 to 63 do
    let x = wh.(i - 15) in
    let y = wh.(i - 2) in
    let s0 = (bxor (bxor (ror x 7) (ror x 18)) (shr x 3)) in
    let s1 = (bxor (bxor (ror y 17) (ror y 19)) (shr y 10)) in
    wh.(i) <- (s1 + wh.(i - 7) + s0 + wh.(i - 16));
  done;
  let a = (ref state.(0)) in
  let b = (ref state.(1)) in
  let c = (ref state.(2)) in
  let d = (ref state.(3)) in
  let e = (ref state.(4)) in
  let f = (ref state.(5)) in
  let g = (ref state.(6)) in
  let h = (ref state.(7)) in
  for i = 0 to 63 do
    let s1 = (bxor (bxor (ror !e 6) (ror !e 11)) (ror !e 25)) in
    let t1 = (!h + s1 + (choice !e !f !g) + k256.(i) + wh.(i)) in
    let s0 = (bxor (bxor (ror !a 2) (ror !a 13)) (ror !a 22)) in
    let newA = (t1 + s0 + (majority !a !b !c)) in
    let newE = (!d + t1) in
    h := !g;
    g := !f;
    f := !e;
    e := newE;
    d := !c;
    c := !b;
    b := !a;
    a := newA;
  done;
  state.(0) <- (state.(0) + !a);
  state.(1) <- (state.(1) + !b);
  state.(2) <- (state.(2) + !c);
  state.(3) <- (state.(3) + !d);
  state.(4) <- (state.(4) + !e);
  state.(5) <- (state.(5) + !f);
  state.(6) <- (state.(6) + !g);
  state.(7) <- (state.(7) + !h);
  ()

let compress512 state data at wh wl =
  for i = 0 to 15 do
    wh.(i) <- (readBe data (at + i * 8));
    wl.(i) <- (readBe data (at + i * 8 + 4));
  done;
  for i = 16 to 79 do
    let xh = wh.(i - 15) in
    let xl = wl.(i - 15) in
    let yh = wh.(i - 2) in
    let yl = wl.(i - 2) in
    let s0h = (bxor (bxor (bor (shr xh 1) (shl xl 31)) (bor (shr xh 8) (shl xl 24))) (shr xh 7)) in
    let s0l = (bxor (bxor (bor (shr xl 1) (shl xh 31)) (bor (shr xl 8) (shl xh 24))) (bor (shr xl 7) (shl xh 25))) in
    let s1h = (bxor (bxor (bor (shr yh 19) (shl yl 13)) (bor (shr yl 29) (shl yh 3))) (shr yh 6)) in
    let s1l = (bxor (bxor (bor (shr yl 19) (shl yh 13)) (bor (shr yh 29) (shl yl 3))) (bor (shr yl 6) (shl yh 26))) in
    let nextl2 = (s0l + s1l) in
    let nextl3 = (nextl2 + wl.(i - 7)) in
    let nextl = (nextl3 + wl.(i - 16)) in
    let nexth = (s0h + s1h + wh.(i - 7) + wh.(i - 16) + (carry s0l s1l) + (carry nextl2 (wl.(i - 7))) + (carry nextl3 (wl.(i - 16)))) in
    wh.(i) <- nexth;
    wl.(i) <- nextl;
  done;
  let ah = (ref state.(0)) in
  let al = (ref state.(1)) in
  let bh = (ref state.(2)) in
  let bl = (ref state.(3)) in
  let ch = (ref state.(4)) in
  let cl = (ref state.(5)) in
  let dh = (ref state.(6)) in
  let dl = (ref state.(7)) in
  let eh = (ref state.(8)) in
  let el = (ref state.(9)) in
  let fh = (ref state.(10)) in
  let fl = (ref state.(11)) in
  let gh = (ref state.(12)) in
  let gl = (ref state.(13)) in
  let hh = (ref state.(14)) in
  let hl = (ref state.(15)) in
  for i = 0 to 79 do
    let s1h = (bxor (bxor (bor (shr !eh 14) (shl !el 18)) (bor (shr !eh 18) (shl !el 14))) (bor (shr !el 9) (shl !eh 23))) in
    let s1l = (bxor (bxor (bor (shr !el 14) (shl !eh 18)) (bor (shr !el 18) (shl !eh 14))) (bor (shr !eh 9) (shl !el 23))) in
    let chh = (choice !eh !fh !gh) in
    let chl = (choice !el !fl !gl) in
    let t1l2 = (!hl + s1l) in
    let t1l3 = (t1l2 + chl) in
    let t1l4 = (t1l3 + k512l.(i)) in
    let t1l = (t1l4 + wl.(i)) in
    let t1h = (!hh + s1h + chh + k512h.(i) + wh.(i) + (carry !hl s1l) + (carry t1l2 chl) + (carry t1l3 k512l.(i)) + (carry t1l4 wl.(i))) in
    let s0h = (bxor (bxor (bor (shr !ah 28) (shl !al 4)) (bor (shr !al 2) (shl !ah 30))) (bor (shr !al 7) (shl !ah 25))) in
    let s0l = (bxor (bxor (bor (shr !al 28) (shl !ah 4)) (bor (shr !ah 2) (shl !al 30))) (bor (shr !ah 7) (shl !al 25))) in
    let majh = (majority !ah !bh !ch) in
    let majl = (majority !al !bl !cl) in
    let newAl2 = (t1l + s0l) in
    let newAl = (newAl2 + majl) in
    let newAh = (t1h + s0h + majh + (carry t1l s0l) + (carry newAl2 majl)) in
    let newEl = (!dl + t1l) in
    let newEh = (!dh + t1h + (carry !dl t1l)) in
    hh := !gh;
    hl := !gl;
    gh := !fh;
    gl := !fl;
    fh := !eh;
    fl := !el;
    eh := newEh;
    el := newEl;
    dh := !ch;
    dl := !cl;
    ch := !bh;
    cl := !bl;
    bh := !ah;
    bl := !al;
    ah := newAh;
    al := newAl;
  done;
  state.(0) <- (state.(0) + !ah + (carry state.(1) !al));
  state.(1) <- (state.(1) + !al);
  state.(2) <- (state.(2) + !bh + (carry state.(3) !bl));
  state.(3) <- (state.(3) + !bl);
  state.(4) <- (state.(4) + !ch + (carry state.(5) !cl));
  state.(5) <- (state.(5) + !cl);
  state.(6) <- (state.(6) + !dh + (carry state.(7) !dl));
  state.(7) <- (state.(7) + !dl);
  state.(8) <- (state.(8) + !eh + (carry state.(9) !el));
  state.(9) <- (state.(9) + !el);
  state.(10) <- (state.(10) + !fh + (carry state.(11) !fl));
  state.(11) <- (state.(11) + !fl);
  state.(12) <- (state.(12) + !gh + (carry state.(13) !gl));
  state.(13) <- (state.(13) + !gl);
  state.(14) <- (state.(14) + !hh + (carry state.(15) !hl));
  state.(15) <- (state.(15) + !hl);
  ()

let hash wide chunks =
  let block = if wide then 128 else 64 in
  let state = Array.copy (if wide then iv512 else iv256) in
  let wh = Array.make 80 0 and wl = Array.make 80 0 in
  let tail = Array.make block 0 in
  let process data at =
    if wide then compress512 state data at wh wl else compress256 state data at wh
  in
  let pos = ref 0 and length = ref 0.0 in
  Array.iter (fun chunk ->
    let len = Array.length chunk and at = ref 0 in
    length := !length +. float_of_int len;
    while !at < len do
      if !pos = 0 && len - !at >= block then begin
        process chunk !at;
        at := !at + block
      end else begin
        let take = min (block - !pos) (len - !at) in
        Array.blit chunk !at tail !pos take;
        at := !at + take;
        pos := !pos + take;
        if !pos = block then begin process tail 0; pos := 0 end
      end
    done) chunks;
  tail.(!pos) <- 128;
  Array.fill tail (!pos+1) (block - !pos-1) 0;
  if !pos+1 > block-block/8 then begin process tail 0; Array.fill tail 0 block 0 end;
  let bits = ref (!length *. 8.) in
  for i = 0 to 7 do
    tail.(block-1-i) <- int_of_float (mod_float !bits 256.);
    bits := floor (!bits /. 256.)
  done;
  process tail 0;
  Array.init (Array.length state * 4) (fun i -> (state.(i/4) lsr (24 - i mod 4*8)) land 255)

let digest mode chunks key =
  if mode <> 2 then hash (mode = 1) chunks else
  let k = if Array.length key > 64 then hash false [|key|] else key in
  let pad mask = Array.init 64 (fun i -> (if i < Array.length k then k.(i) else 0) lxor mask) in
  let inner = Array.append [|pad 0x36|] chunks in
  hash false [|pad 0x5c; hash false inner|]
