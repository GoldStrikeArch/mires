(* Raw DEFLATE decompressor (inflt, inflateSync) for fflate 0.8.3. *)

open Host

type decompress_options = {
  dictionary : U8.t option;
  out : U8.t option;
}

let empty_decompress_options = { dictionary = None; out = None }

type istate = {
  mutable i : int;
  mutable b : int;
  mutable p : int;
  mutable f : int;
  mutable l : U16.t option;
  mutable d : U16.t option;
  mutable m : int;
  mutable n : int;
}

let fresh_istate ~i ~b = { i; b; p = 0; f = 0; l = None; d = None; m = 0; n = 0 }

let[@inline] bits dat sl p m =
  let o = p lsr 3 in
  let v =
    if o + 1 < sl then U8.get dat o lor (U8.get dat (o + 1) lsl 8)
    else if o < sl then U8.get dat o
    else 0
  in
  (v lsr (p land 7)) land m

let[@inline] bits16 dat sl p =
  let o = p lsr 3 in
  if o + 2 < sl then
    (U8.get dat o lor (U8.get dat (o + 1) lsl 8) lor (U8.get dat (o + 2) lsl 16)) lsr (p land 7)
  else
    let b0 = if o < sl then U8.get dat o else 0 in
    let b1 = if o + 1 < sl then U8.get dat (o + 1) else 0 in
    (b0 lor (b1 lsl 8)) lsr (p land 7)

let inflt dat st buf_opt dict_opt =
  let sl = U8.length dat in
  let dl = match dict_opt with Some d -> U8.length d | None -> 0 in
  if sl = 0 || (st.f <> 0 && Tables.is_none st.l) then
    match buf_opt with Some b -> b | None -> U8.create 0
  else begin
    let no_buf = Tables.is_none buf_opt in
    let resize = no_buf || st.i <> 2 in
    let no_st = st.i in
    let buf = ref (match buf_opt with Some b -> b | None -> U8.create (sl * 3)) in
    let cbuf l =
      let cur = !buf in
      let bl = U8.length cur in
      if l > bl then begin
        let dbl = bl lsl 1 in
        let nlen = if dbl > l then dbl else l in
        let nbuf = U8.create nlen in
        if bl > 0 then U8.set_from nbuf cur 0;
        buf := nbuf
      end
    in
    let final = ref st.f in
    let pos = ref st.p in
    let bt = ref st.b in
    let lm = ref st.l in
    let dm = ref st.d in
    let lbt = ref st.m in
    let dbt = ref st.n in
    let tbts = sl * 8 in
    let fleb = Tables.fleb in
    let fdeb = Tables.fdeb in
    let fl = Tables.fl in
    let fd = Tables.fd in
    let clim = Tables.clim in
    let done_outer = ref false in
    while not !done_outer do
      let need_header = Tables.is_none !lm in
      let skip_to_next_iter = ref false in
      let break_outer = ref false in
      if need_header then begin
        final := bits dat sl !pos 1;
        let btype = bits dat sl (!pos + 1) 3 in
        pos := !pos + 3;
        if btype = 0 then begin
          let s = Tables.shft !pos + 4 in
          let b0 = if s - 4 < sl then U8.get dat (s - 4) else 0 in
          let b1 = if s - 3 < sl then U8.get dat (s - 3) else 0 in
          let l = b0 lor (b1 lsl 8) in
          let t = s + l in
          if t > sl then begin
            if no_st <> 0 then Tables.err 0 "";
            break_outer := true
          end
          else begin
            if resize then cbuf (!bt + l);
            (* When !resize and !bt + l > buf.length, JS TypedArray.set throws RangeError. *)
            U8.set_from !buf (U8.subarray dat s t) !bt;
            bt := !bt + l;
            pos := t * 8;
            st.b <- !bt;
            st.p <- !pos;
            st.f <- !final;
            skip_to_next_iter := true
          end
        end
        else if btype = 1 then begin
          lm := Some Huffman.flrm;
          dm := Some Huffman.fdrm;
          lbt := 9;
          dbt := 5
        end
        else if btype = 2 then begin
          let h_lit = bits dat sl !pos 31 + 257 in
          let hc_len = bits dat sl (!pos + 10) 15 + 4 in
          let tl = h_lit + bits dat sl (!pos + 5) 31 + 1 in
          pos := !pos + 14;
          let ldt = U8.create tl in
          let clt = U8.create 19 in
          for i = 0 to hc_len - 1 do
            U8.set clt (U8.get clim i) (bits dat sl (!pos + (i * 3)) 7)
          done;
          pos := !pos + (hc_len * 3);
          let clb = Tables.max_u8 clt in
          let clbmsk = (1 lsl clb) - 1 in
          let clm = Huffman.h_map clt clb true in
          let i = ref 0 in
          while !i < tl do
            let r = U16.get clm (bits dat sl !pos clbmsk) in
            pos := !pos + (r land 15);
            let sym = r lsr 4 in
            if sym < 16 then begin
              U8.set ldt !i sym;
              incr i
            end
            else begin
              let c = ref 0 in
              let n = ref 0 in
              if sym = 16 then begin
                n := 3 + bits dat sl !pos 3;
                pos := !pos + 2;
                c := if !i > 0 then U8.get ldt (!i - 1) else 0
              end
              else if sym = 17 then begin
                n := 3 + bits dat sl !pos 7;
                pos := !pos + 3
              end
              else if sym = 18 then begin
                n := 11 + bits dat sl !pos 127;
                pos := !pos + 7
              end;
              let cv = !c in
              while !n > 0 do
                if !i < tl then U8.set ldt !i cv;
                incr i;
                decr n
              done
            end
          done;
          let lt = U8.subarray ldt 0 h_lit in
          let dt = U8.subarray ldt h_lit tl in
          let l_bits = Tables.max_u8 lt in
          let d_bits = Tables.max_u8 dt in
          lbt := l_bits;
          dbt := d_bits;
          lm := Some (Huffman.h_map lt l_bits true);
          dm := Some (Huffman.h_map dt d_bits true)
        end
        else Tables.err 1 "";
        if (not !break_outer) && (not !skip_to_next_iter) && !pos > tbts then begin
          if no_st <> 0 then Tables.err 0 "";
          break_outer := true
        end
      end;
      if !break_outer then done_outer := true
      else if !skip_to_next_iter then (if !final <> 0 then done_outer := true)
      else begin
        if resize then cbuf (!bt + 131072);
        let lm_tbl = match !lm with Some t -> t | None -> Tables.err 2 "" in
        let dm_tbl = match !dm with Some t -> t | None -> Tables.err 3 "" in
        let lms = (1 lsl !lbt) - 1 in
        let dms = (1 lsl !dbt) - 1 in
        let lpos = ref !pos in
        let in_block = ref true in
        while !in_block do
          lpos := !pos;
          let c = U16.get lm_tbl (bits16 dat sl !pos land lms) in
          let sym = c lsr 4 in
          pos := !pos + (c land 15);
          if !pos > tbts then begin
            if no_st <> 0 then Tables.err 0 "";
            in_block := false
          end
          else if c = 0 then Tables.err 2 ""
          else if sym < 256 then begin
            let cur_buf = !buf in
            let b_idx = !bt in
            if resize || b_idx < U8.length cur_buf then U8.set cur_buf b_idx sym;
            bt := b_idx + 1
          end
          else if sym = 256 then begin
            lpos := !pos;
            lm := None;
            in_block := false
          end
          else begin
            let add =
              if sym > 264 then begin
                let idx = sym - 257 in
                let b = U8.get fleb idx in
                let a = bits dat sl !pos ((1 lsl b) - 1) + U16.get fl idx in
                pos := !pos + b;
                a
              end
              else sym - 254
            in
            let d = U16.get dm_tbl (bits16 dat sl !pos land dms) in
            let dsym = d lsr 4 in
            if d = 0 then Tables.err 3 "";
            pos := !pos + (d land 15);
            let dt =
              let base = U16.get fd dsym in
              if dsym > 3 then begin
                let b = U8.get fdeb dsym in
                let extra = bits16 dat sl !pos land ((1 lsl b) - 1) in
                pos := !pos + b;
                base + extra
              end
              else base
            in
            if !pos > tbts then begin
              if no_st <> 0 then Tables.err 0 "";
              in_block := false
            end
            else begin
              if resize then cbuf (!bt + 131072);
              let cur_buf = !buf in
              let cur_bl = U8.length cur_buf in
              let end_pos = !bt + add in
              if !bt < dt then begin
                let shift = dl - dt in
                let dend = if dt < end_pos then dt else end_pos in
                if shift + !bt < 0 then Tables.err 3 "";
                match dict_opt with
                | Some dict ->
                    while !bt < dend do
                      let b_idx = !bt in
                      if resize || b_idx < cur_bl then U8.set cur_buf b_idx (U8.get dict (shift + b_idx));
                      bt := b_idx + 1
                    done
                | None -> Tables.err 3 ""
              end;
              while !bt < end_pos do
                let b_idx = !bt in
                if resize || b_idx < cur_bl then U8.set cur_buf b_idx (U8.get cur_buf (b_idx - dt));
                bt := b_idx + 1
              done
            end
          end
        done;
        st.l <- !lm;
        st.p <- !lpos;
        st.b <- !bt;
        st.f <- !final;
        if Tables.is_some !lm then begin
          final := 1;
          st.m <- !lbt;
          st.d <- !dm;
          st.n <- !dbt
        end;
        if !final <> 0 then done_outer := true
      end
    done;
    let cur_buf = !buf in
    let bl = U8.length cur_buf in
    let out_len = if !bt < bl then !bt else bl in
    if !bt <> bl && no_buf then Tables.slc cur_buf 0 out_len
    else U8.subarray cur_buf 0 out_len
  end

let inflate_sync dat (opts : decompress_options) =
  inflt dat (fresh_istate ~i:2 ~b:0) opts.out opts.dictionary
