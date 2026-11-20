(* Checksums (CRC-32, Adler-32), GZIP/Zlib framing, and format auto-detection (fflate 0.8.3). *)

open Host

type crc_state = { mutable c : int }

let crc_create () = { c = -1 }

let crc_push st d =
  let crct = Tables.crct in
  let len = U8.length d in
  let cr = ref st.c in
  for i = 0 to len - 1 do
    let cur = !cr in
    cr := I32.get crct ((cur land 255) lxor U8.get d i) lxor (cur lsr 8)
  done;
  st.c <- !cr

let crc_digest st = lnot st.c

type adler_state = {
  mutable a : int;
  mutable b : int;
}

let adler_create () = { a = 1; b = 0 }

let adler_push st d =
  let len = U8.length d in
  let n = ref st.a in
  let m = ref st.b in
  let i = ref 0 in
  while !i <> len do
    let e =
      let next = !i + 2655 in
      if next < len then next else len
    in
    let nv = ref !n in
    let mv = ref !m in
    for k = !i to e - 1 do
      let n2 = !nv + U8.get d k in
      nv := n2;
      mv := !mv + n2
    done;
    let nv' = !nv in
    let mv' = !mv in
    n := (nv' land 65535) + (15 * (nv' asr 16));
    m := (mv' land 65535) + (15 * (mv' asr 16));
    i := e
  done;
  st.a <- !n;
  st.b <- !m

let adler_digest st =
  let a = st.a mod 65521 in
  let b = st.b mod 65521 in
  st.a <- a;
  st.b <- b;
  ((a land 255) lsl 24) lor ((a land 0xff00) lsl 8) lor ((b land 255) lsl 8) lor (b lsr 8)

let wbytes d b v =
  let rec loop b v =
    if v <> 0 then begin
      U8.set d b (v land 255);
      loop (b + 1) (v lsr 8)
    end
  in
  loop b v

(* Convert a non-negative integer float in [0, 2^32 - 1] to its two's-complement 32-bit int. *)
let u32_of_float f =
  let f = floor f in
  if f >= 2147483648.0 then int_of_float (f -. 4294967296.0) else int_of_float f

let gzhl (o : Deflate.compress_options) =
  match o.filename with
  | Some fn ->
      let flen = Text.length fn in
      if flen > 0 then 10 + flen + 1 else 10
  | None -> 10

let gzh c (o : Deflate.compress_options) =
  let lv = match o.level with Some l -> l | None -> 6 in
  U8.set c 0 31;
  U8.set c 1 139;
  U8.set c 2 8;
  U8.set c 8 (if lv < 2 then 4 else if lv = 9 then 2 else 0);
  U8.set c 9 3;
  (match o.mtime with
  | Some m -> if m <> 0.0 then wbytes c 4 (u32_of_float (m /. 1000.0))
  | None -> wbytes c 4 (u32_of_float (Host.date_now () /. 1000.0)));
  match o.filename with
  | Some fn ->
      let flen = Text.length fn in
      if flen > 0 then begin
        U8.set c 3 8;
        for i = 0 to flen - 1 do
          U8.set c (i + 10) (Text.unit_at fn i land 255)
        done;
        U8.set c (flen + 10) 0
      end
  | None -> ()

let[@inline] u8_z d len i = if i < len then U8.get d i else 0

let gzs d =
  let len = U8.length d in
  if u8_z d len 0 <> 31 || u8_z d len 1 <> 139 || u8_z d len 2 <> 8 then
    Tables.err 6 "invalid gzip data";
  let flg = u8_z d len 3 in
  let st = ref 10 in
  if flg land 4 <> 0 then st := !st + (u8_z d len 10 lor (u8_z d len 11 lsl 8)) + 2;
  let zs = ref (((flg lsr 3) land 1) + ((flg lsr 4) land 1)) in
  while !zs > 0 do
    let b = u8_z d len !st in
    incr st;
    if b = 0 then decr zs
  done;
  !st + (flg land 2)

let gzl d =
  let l = U8.length d in
  U8.get d (l - 4)
  lor (U8.get d (l - 3) lsl 8)
  lor (U8.get d (l - 2) lsl 16)
  lor (U8.get d (l - 1) lsl 24)

let gzip_sync data (opts : Deflate.compress_options) =
  let c = crc_create () in
  let l = U8.length data in
  crc_push c data;
  let d = Deflate.dopt data opts (gzhl opts) 8 None in
  let s = U8.length d in
  gzh d opts;
  wbytes d (s - 8) (crc_digest c);
  wbytes d (s - 4) l;
  d

let gunzip_sync data (opts : Inflate.decompress_options) =
  let st = gzs data in
  let len = U8.length data in
  if st + 8 > len then Tables.err 6 "invalid gzip data";
  let out =
    match opts.out with
    | Some b -> Some b
    | None ->
        let sz = gzl data in
        (* In 32-bit signed OCaml int, an ISIZE with bit 31 set (>= 2 GiB) wraps negative;
           treat it as an unbounded dynamically resized buffer instead of passing a negative
           length to Uint8Array. *)
        if sz < 0 then None else Some (U8.create sz)
  in
  Inflate.inflt (U8.subarray data st (len - 8)) (Inflate.fresh_istate ~i:2 ~b:0) out opts.dictionary

let zlh c (o : Deflate.compress_options) =
  let lv = match o.level with Some l -> l | None -> 6 in
  let fl = if lv = 0 then 0 else if lv < 6 then 1 else if lv = 9 then 3 else 2 in
  let has_dict = Tables.is_some o.dictionary in
  let c0 = 120 in
  let c1_base = (fl lsl 6) lor (if has_dict then 32 else 0) in
  let c1 = c1_base lor (31 - (((c0 lsl 8) lor c1_base) mod 31)) in
  U8.set c 0 c0;
  U8.set c 1 c1;
  match o.dictionary with
  | Some dict ->
      let h = adler_create () in
      adler_push h dict;
      wbytes c 2 (adler_digest h)
  | None -> ()

let zls d has_dict =
  let len = U8.length d in
  let b0 = u8_z d len 0 in
  let b1 = u8_z d len 1 in
  if b0 land 15 <> 8 || b0 lsr 4 > 7 || ((b0 lsl 8) lor b1) mod 31 <> 0 then
    Tables.err 6 "invalid zlib data";
  let fdict = (b1 lsr 5) land 1 in
  if fdict = if has_dict then 0 else 1 then
    Tables.err 6
      (if b1 land 32 <> 0 then "invalid zlib data: need dictionary"
       else "invalid zlib data: unexpected dictionary");
  ((b1 lsr 3) land 4) + 2

let zlib_sync data (opts : Deflate.compress_options) =
  let a = adler_create () in
  adler_push a data;
  let pre = if Tables.is_some opts.dictionary then 6 else 2 in
  let d = Deflate.dopt data opts pre 4 None in
  zlh d opts;
  wbytes d (U8.length d - 4) (adler_digest a);
  d

let unzlib_sync data (opts : Inflate.decompress_options) =
  let start = zls data (Tables.is_some opts.dictionary) in
  let len = U8.length data in
  let stop = let e = len - 4 in if e > start then e else start in
  Inflate.inflt (U8.subarray data start stop) (Inflate.fresh_istate ~i:2 ~b:0) opts.out
    opts.dictionary

let is_gzip_header d =
  let len = U8.length d in
  u8_z d len 0 = 31 && u8_z d len 1 = 139 && u8_z d len 2 = 8

let is_raw_deflate_not_zlib d =
  let len = U8.length d in
  let b0 = u8_z d len 0 in
  let b1 = u8_z d len 1 in
  b0 land 15 <> 8 || b0 lsr 4 > 7 || ((b0 lsl 8) lor b1) mod 31 <> 0

let decompress_sync data (opts : Inflate.decompress_options) =
  if is_gzip_header data then gunzip_sync data opts
  else if is_raw_deflate_not_zlib data then Inflate.inflate_sync data opts
  else unzlib_sync data opts
