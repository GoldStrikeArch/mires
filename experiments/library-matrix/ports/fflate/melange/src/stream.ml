(* Synchronous streaming classes for fflate 0.8.3:
   Deflate, Gzip, Zlib, Inflate, Gunzip, Unzlib, Decompress. *)

open Host

type handler = U8.t -> bool -> unit

type compress_kind =
  | Kind_deflate
  | Kind_gzip of Framing.crc_state * int ref * int ref
  | Kind_zlib of Framing.adler_state * int ref

type compress_stream = {
  kind : compress_kind;
  mutable opts : Deflate.compress_options;
  mutable s : Deflate.dstate;
  mutable b : U8.t;
  ondata : handler option;
}

let create_compress_stream kind (opts : Deflate.compress_options) ondata =
  let s = Deflate.fresh_dstate ~l:0 ~i:32768 ~w:32768 ~z:32768 in
  let b = U8.create 98304 in
  (match opts.dictionary with
  | Some dict_full ->
      let dict = Tables.last_32768 dict_full in
      let dl = U8.length dict in
      if dl > 0 then U8.set_from b dict (32768 - dl);
      s.i <- 32768 - dl
  | None -> ());
  { kind; opts; s; b; ondata }

let create_deflate opts ondata = create_compress_stream Kind_deflate opts ondata

let create_gzip opts ondata =
  create_compress_stream (Kind_gzip (Framing.crc_create (), ref 0, ref 1)) opts ondata

let create_zlib opts ondata =
  create_compress_stream (Kind_zlib (Framing.adler_create (), ref 1)) opts ondata

let emit_compress st ondata c final =
  match st.kind with
  | Kind_deflate -> ondata (Deflate.dopt c st.opts 0 0 (Some st.s)) final
  | Kind_gzip (crc, total_len, v) ->
      let pre = if !v <> 0 then Framing.gzhl st.opts else 0 in
      let post = if final then 8 else 0 in
      let raw = Deflate.dopt c st.opts pre post (Some st.s) in
      if !v <> 0 then begin
        Framing.gzh raw st.opts;
        v := 0
      end;
      if final then begin
        let rlen = U8.length raw in
        Framing.wbytes raw (rlen - 8) (Framing.crc_digest crc);
        Framing.wbytes raw (rlen - 4) !total_len
      end;
      ondata raw final
  | Kind_zlib (adler, v) ->
      let pre = if !v <> 0 then if Tables.is_some st.opts.dictionary then 6 else 2 else 0 in
      let post = if final then 4 else 0 in
      let raw = Deflate.dopt c st.opts pre post (Some st.s) in
      if !v <> 0 then begin
        Framing.zlh raw st.opts;
        v := 0
      end;
      if final then Framing.wbytes raw (U8.length raw - 4) (Framing.adler_digest adler);
      ondata raw final

let push_compress st chunk final =
  (match st.kind with
  | Kind_deflate -> ()
  | Kind_gzip (crc, total_len, _) ->
      Framing.crc_push crc chunk;
      total_len := !total_len + U8.length chunk
  | Kind_zlib (adler, _) -> Framing.adler_push adler chunk);
  let ondata = match st.ondata with Some f -> f | None -> Tables.err 5 "" in
  if st.s.l <> 0 then Tables.err 4 "";
  let clen = U8.length chunk in
  let end_len = clen + st.s.z in
  if end_len > U8.length st.b then begin
    let cur_blen = U8.length st.b in
    if end_len > (2 * cur_blen) - 32768 then begin
      let new_buf = U8.create (end_len land -32768) in
      U8.set_from new_buf (U8.subarray st.b 0 st.s.z) 0;
      st.b <- new_buf
    end;
    let blen = U8.length st.b in
    let split = blen - st.s.z in
    if split > 0 then U8.set_from st.b (U8.subarray chunk 0 split) st.s.z;
    st.s.z <- blen;
    emit_compress st ondata st.b false;
    U8.set_from st.b (U8.subarray st.b (blen - 32768) blen) 0;
    if clen > split then U8.set_from st.b (U8.subarray chunk split clen) 32768;
    st.s.z <- clen - split + 32768;
    st.s.i <- 32766;
    st.s.w <- 32768
  end
  else begin
    if clen > 0 then U8.set_from st.b chunk st.s.z;
    st.s.z <- st.s.z + clen
  end;
  st.s.l <- (if final then 1 else 0);
  if st.s.z > st.s.w + 8191 || final then begin
    emit_compress st ondata st.b final;
    st.s.w <- st.s.i;
    st.s.i <- st.s.i - 2
  end;
  if final then begin
    st.s <- Deflate.fresh_dstate ~l:0 ~i:0 ~w:0 ~z:0;
    st.opts <- Deflate.empty_compress_options;
    st.b <- Tables.empty_u8
  end

let flush_compress st sync =
  let ondata = match st.ondata with Some f -> f | None -> Tables.err 5 "" in
  if st.s.l <> 0 then Tables.err 4 "";
  emit_compress st ondata st.b false;
  st.s.w <- st.s.i;
  st.s.i <- st.s.i - 2;
  if sync then begin
    let c = U8.create 6 in
    U8.set c 0 (st.s.r lsr 3);
    let ep = Deflate.wfblk c st.s.r Tables.empty_u8 0 0 in
    st.s.r <- 0;
    ondata (U8.subarray c 0 (ep lsr 3)) false
  end

(* Inflate base stream *)
type inflate_stream = {
  mutable s : Inflate.istate;
  mutable o : U8.t;
  mutable p : U8.t;
  mutable d : bool;
  ondata : handler option;
}

let create_inflate (opts : Inflate.decompress_options) ondata =
  let dict_opt = match opts.dictionary with Some d -> Some (Tables.last_32768 d) | None -> None in
  let dl = match dict_opt with Some d -> U8.length d | None -> 0 in
  let o = U8.create 32768 in
  (match dict_opt with Some d when dl > 0 -> U8.set_from o d 0 | _ -> ());
  { s = Inflate.fresh_istate ~i:0 ~b:dl; o; p = U8.create 0; d = false; ondata }

let inflate_e st chunk =
  if Tables.is_none st.ondata then Tables.err 5 "";
  if st.d then Tables.err 4 "";
  let plen = U8.length st.p in
  let clen = U8.length chunk in
  if plen = 0 then st.p <- chunk
  else if clen > 0 then begin
    let n = U8.create (plen + clen) in
    U8.set_from n st.p 0;
    U8.set_from n chunk plen;
    st.p <- n
  end

let inflate_c st final =
  let ondata = match st.ondata with Some f -> f | None -> Tables.err 5 "" in
  st.d <- final;
  st.s.i <- (if final then 1 else 0);
  let bts = st.s.b in
  let dt = Inflate.inflt st.p st.s (Some st.o) None in
  ondata (Tables.slc dt bts st.s.b) st.d;
  st.o <- Tables.slc dt (st.s.b - 32768) (U8.length dt);
  st.s.b <- U8.length st.o;
  st.p <- Tables.slc st.p (st.s.p lsr 3) (U8.length st.p);
  st.s.p <- st.s.p land 7

let push_inflate st chunk final =
  inflate_e st chunk;
  inflate_c st final

(* Gunzip stream (supports concatenated multi-member GZIP via exact recursive push) *)
type gunzip_stream = {
  inf : inflate_stream;
  mutable v : int;
  mutable r : int;
}

let create_gunzip opts ondata = { inf = create_inflate opts ondata; v = 1; r = 0 }

let rec push_gunzip st chunk final =
  inflate_e st.inf chunk;
  st.r <- st.r + U8.length chunk;
  let waiting = ref false in
  if st.v <> 0 then begin
    let p = Tables.subarray_from st.inf.p (st.v - 1) in
    let plen = U8.length p in
    let s = if plen > 3 then Framing.gzs p else 4 in
    if s > plen && not final then waiting := true
    else begin
      st.inf.p <- Tables.subarray_from p s;
      st.v <- 0
    end
  end;
  if not !waiting then begin
    inflate_c st.inf false;
    if st.inf.s.f <> 0 && Tables.is_none st.inf.s.l then begin
      st.v <- Tables.shft st.inf.s.p + 9;
      st.inf.s <- Inflate.fresh_istate ~i:0 ~b:0;
      st.inf.o <- U8.create 0;
      push_gunzip st (U8.create 0) final
    end
    else if final then inflate_c st.inf true
  end

(* Unzlib stream *)
type unzlib_stream = {
  inf : inflate_stream;
  mutable v : int;
}

let create_unzlib (opts : Inflate.decompress_options) ondata =
  { inf = create_inflate opts ondata; v = (if Tables.is_some opts.dictionary then 2 else 1) }

let push_unzlib st chunk final =
  inflate_e st.inf chunk;
  let waiting = ref false in
  if st.v <> 0 then begin
    if U8.length st.inf.p < 6 && not final then waiting := true
    else begin
      let skip = Framing.zls st.inf.p (st.v - 1 <> 0) in
      st.inf.p <- Tables.subarray_from st.inf.p skip;
      st.v <- 0
    end
  end;
  if not !waiting then begin
    if final then begin
      let plen = U8.length st.inf.p in
      if plen < 4 then Tables.err 6 "invalid zlib data";
      st.inf.p <- U8.subarray st.inf.p 0 (plen - 4)
    end;
    inflate_c st.inf final
  end

(* Decompress auto-detecting stream *)
type inner_decompress =
  | Sub_gunzip of gunzip_stream
  | Sub_inflate of inflate_stream
  | Sub_unzlib of unzlib_stream

type decompress_stream = {
  opts : Inflate.decompress_options;
  ondata : handler option;
  mutable sub : inner_decompress option;
  mutable p : U8.t option;
}

let create_decompress opts ondata = { opts; ondata; sub = None; p = None }

let push_inner sub chunk final =
  match sub with
  | Sub_gunzip g -> push_gunzip g chunk final
  | Sub_inflate i -> push_inflate i chunk final
  | Sub_unzlib z -> push_unzlib z chunk final

let push_decompress st chunk final =
  if Tables.is_none st.ondata then Tables.err 5 "";
  match st.sub with
  | Some sub -> push_inner sub chunk final
  | None -> (
      let buf =
        match st.p with
        | Some prev when U8.length prev > 0 ->
            let plen = U8.length prev in
            let clen = U8.length chunk in
            let n = U8.create (plen + clen) in
            U8.set_from n prev 0;
            if clen > 0 then U8.set_from n chunk plen;
            n
        | _ -> chunk
      in
      st.p <- Some buf;
      if U8.length buf > 2 then begin
        let sub =
          if Framing.is_gzip_header buf then Sub_gunzip (create_gunzip st.opts st.ondata)
          else if Framing.is_raw_deflate_not_zlib buf then
            Sub_inflate (create_inflate st.opts st.ondata)
          else Sub_unzlib (create_unzlib st.opts st.ondata)
        in
        st.sub <- Some sub;
        st.p <- None;
        push_inner sub buf final
      end)

(* Record accumulator & framing for streaming test ops:
   each ondata(chunk, final) emits [u32le(chunk.length), final ? 1 : 0, ...chunk]. *)
type record = {
  chunk : U8.t;
  is_final : bool;
}

let frame_records rev_records =
  let rec rev_and_sum acc total = function
    | [] -> (acc, total)
    | r :: tl -> rev_and_sum (r :: acc) (total + 5 + U8.length r.chunk) tl
  in
  let records, total = rev_and_sum [] 0 rev_records in
  let out = U8.create total in
  let rec write off = function
    | [] -> ()
    | r :: tl ->
        let len = U8.length r.chunk in
        U8.set out off (len land 255);
        U8.set out (off + 1) ((len lsr 8) land 255);
        U8.set out (off + 2) ((len lsr 16) land 255);
        U8.set out (off + 3) ((len lsr 24) land 255);
        U8.set out (off + 4) (if r.is_final then 1 else 0);
        if len > 0 then U8.set_from out r.chunk (off + 5);
        write (off + 5 + len) tl
  in
  write 0 records;
  out
