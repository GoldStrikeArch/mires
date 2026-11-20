(* Raw DEFLATE compressor (wbits, wfblk, wblk, dflt, dopt, deflateSync) for fflate 0.8.3. *)

open Host

type compress_options = {
  level : int option;
  mem : int option;
  dictionary : U8.t option;
  mtime : float option;
  filename : Text.t option;
}

let empty_compress_options =
  { level = None; mem = None; dictionary = None; mtime = None; filename = None }

type dstate = {
  mutable l : int;
  mutable i : int;
  mutable w : int;
  mutable z : int;
  mutable r : int;
  mutable h : U16.t option;
  mutable p : U16.t option;
}

let fresh_dstate ~l ~i ~w ~z = { l; i; w; z; r = 0; h = None; p = None }

let[@inline] wbits d p v =
  let v = v lsl (p land 7) in
  let o = p lsr 3 in
  U8.set d o (U8.get d o lor (v land 0xff));
  U8.set d (o + 1) (U8.get d (o + 1) lor ((v lsr 8) land 0xff))

let[@inline] wbits16 d p v =
  let v = v lsl (p land 7) in
  let o = p lsr 3 in
  U8.set d o (U8.get d o lor (v land 0xff));
  U8.set d (o + 1) (U8.get d (o + 1) lor ((v lsr 8) land 0xff));
  U8.set d (o + 2) (U8.get d (o + 2) lor ((v lsr 16) land 0xff))

let wfblk out pos dat start len =
  let o = Tables.shft (pos + 2) in
  let lo = len land 255 in
  let hi = (len lsr 8) land 255 in
  U8.set out o lo;
  U8.set out (o + 1) hi;
  U8.set out (o + 2) (lo lxor 255);
  U8.set out (o + 3) (hi lxor 255);
  if len > 0 then U8.set_from out (U8.subarray dat start (start + len)) (o + 4);
  (o + 4 + len) * 8

let wblk dat out final syms lf df eb li bs bl p =
  wbits out p final;
  let p = p + 1 in
  U16.set lf 256 (U16.get lf 256 + 1);
  let dlt, mlb = Huffman.h_tree lf 15 in
  let ddt, mdb = Huffman.h_tree df 15 in
  let lclt, nlc = Huffman.lc dlt in
  let lcdt, ndc = Huffman.lc ddt in
  let lcfreq = U16.create 19 in
  let lclt_len = U16.length lclt in
  for i = 0 to lclt_len - 1 do
    let idx = U16.get lclt i land 31 in
    U16.set lcfreq idx (U16.get lcfreq idx + 1)
  done;
  let lcdt_len = U16.length lcdt in
  for i = 0 to lcdt_len - 1 do
    let idx = U16.get lcdt i land 31 in
    U16.set lcfreq idx (U16.get lcfreq idx + 1)
  done;
  let lct, mlcb = Huffman.h_tree lcfreq 7 in
  let lct_len = U8.length lct in
  let clim = Tables.clim in
  let nlcc = ref 19 in
  while
    !nlcc > 4
    &&
    let ci = U8.get clim (!nlcc - 1) in
    ci >= lct_len || U8.get lct ci = 0
  do
    decr nlcc
  done;
  let nlcc = !nlcc in
  let flen = (bl + 5) lsl 3 in
  let ftlen = Huffman.clen lf Tables.flt + Huffman.clen df Tables.fdt + eb in
  let dtlen =
    Huffman.clen lf dlt + Huffman.clen df ddt + eb + 14 + (3 * nlcc) + Huffman.clen lcfreq lct
    + (2 * U16.get lcfreq 16) + (3 * U16.get lcfreq 17) + (7 * U16.get lcfreq 18)
  in
  if bs >= 0 && flen <= ftlen && flen <= dtlen then wfblk out p dat bs bl
  else begin
    let use_dyn = dtlen < ftlen in
    wbits out p (if use_dyn then 2 else 1);
    let p = ref (p + 2) in
    let lm, ll, dm, dl =
      if use_dyn then begin
        let lm = Huffman.h_map dlt mlb false in
        let dm = Huffman.h_map ddt mdb false in
        let llm = Huffman.h_map lct mlcb false in
        wbits out !p (nlc - 257);
        wbits out (!p + 5) (ndc - 1);
        wbits out (!p + 10) (nlcc - 4);
        p := !p + 14;
        for i = 0 to nlcc - 1 do
          let ci = U8.get clim i in
          let v = if ci < lct_len then U8.get lct ci else 0 in
          wbits out (!p + (3 * i)) v
        done;
        p := !p + (3 * nlcc);
        let write_cl clct cl_len =
          for i = 0 to cl_len - 1 do
            let entry = U16.get clct i in
            let len = entry land 31 in
            wbits out !p (U16.get llm len);
            p := !p + U8.get lct len;
            if len > 15 then begin
              wbits out !p ((entry lsr 5) land 127);
              p := !p + (entry lsr 12)
            end
          done
        in
        write_cl lclt lclt_len;
        write_cl lcdt lcdt_len;
        (lm, dlt, dm, ddt)
      end
      else (Huffman.flm, Tables.flt, Huffman.fdm, Tables.fdt)
    in
    let fleb = Tables.fleb in
    let fdeb = Tables.fdeb in
    let pos = ref !p in
    for i = 0 to li - 1 do
      let sym = I32.get syms i in
      if sym > 255 then begin
        let len = (sym lsr 18) land 31 in
        let lidx = len + 257 in
        wbits16 out !pos (U16.get lm lidx);
        pos := !pos + U8.get ll lidx;
        if len > 7 then begin
          wbits out !pos ((sym lsr 23) land 31);
          pos := !pos + U8.get fleb len
        end;
        let dst = sym land 31 in
        wbits16 out !pos (U16.get dm dst);
        pos := !pos + U8.get dl dst;
        if dst > 3 then begin
          wbits16 out !pos ((sym lsr 5) land 8191);
          pos := !pos + U8.get fdeb dst
        end
      end
      else begin
        wbits16 out !pos (U16.get lm sym);
        pos := !pos + U8.get ll sym
      end
    done;
    wbits16 out !pos (U16.get lm 256);
    !pos + U8.get ll 256
  end

(* Exact integer cutoffs for Math.ceil(Math.max(8, Math.min(13, Math.log(len))) * 1.5). *)
let default_plvl len =
  if len <= 2980 then 12
  else if len <= 5806 then 13
  else if len <= 11308 then 14
  else if len <= 22026 then 15
  else if len <= 42901 then 16
  else if len <= 83561 then 17
  else if len <= 162754 then 18
  else if len <= 317003 then 19
  else 20

let dflt dat lvl plvl pre post st =
  let s = if st.z <> 0 then st.z else U8.length dat in
  let body_cap = s + (5 * (1 + ((s + 6999) / 7000))) in
  (* +2 slack bytes in w so wbits16 never writes past w.length into undefined *)
  let o = U8.create (pre + body_cap + post + 2) in
  let w = U8.subarray o pre (pre + body_cap + 2) in
  let lst = st.l in
  let pos = ref (st.r land 7) in
  if lvl <> 0 then begin
    if !pos <> 0 then U8.set w 0 (st.r lsr 3);
    let opt = I32.get Tables.deo (lvl - 1) in
    let n = opt lsr 13 in
    let c = opt land 8191 in
    let msk = (1 lsl plvl) - 1 in
    let prev = match st.p with Some p -> p | None -> U16.create 32768 in
    let head = match st.h with Some h -> h | None -> U16.create (msk + 1) in
    let bs1 = (plvl + 2) / 3 in
    let bs2 = bs1 lsl 1 in
    let syms = I32.create 25000 in
    let lf = U16.create 288 in
    let df = U16.create 32 in
    let revfl = Tables.revfl in
    let revfd = Tables.revfd in
    let fleb = Tables.fleb in
    let fdeb = Tables.fdeb in
    let lc = ref 0 in
    let eb = ref 0 in
    let i = ref st.i in
    let li = ref 0 in
    let wi = ref st.w in
    (* bs always starts at 0 even when st.w > 0, matching fflate 0.8.3 *)
    let bs = ref 0 in
    while !i + 2 < s do
      let idx = !i in
      let hv =
        (U8.get dat idx lxor (U8.get dat (idx + 1) lsl bs1) lxor (U8.get dat (idx + 2) lsl bs2))
        land msk
      in
      let imod = idx land 32767 in
      let pimod = U16.get head hv in
      U16.set prev imod pimod;
      U16.set head hv imod;
      if !wi <= idx then begin
        let rem = s - idx in
        if (!lc > 7000 || !li > 24576) && (rem > 423 || lst = 0) then begin
          pos := wblk dat w 0 syms lf df !eb !li !bs (idx - !bs) !pos;
          li := 0;
          lc := 0;
          eb := 0;
          bs := idx;
          for j = 0 to 285 do
            U16.set lf j 0
          done;
          for j = 0 to 29 do
            U16.set df j 0
          done
        end;
        let dif0 = (imod - pimod) land 32767 in
        let best_l = ref 2 in
        let best_d = ref 0 in
        if rem > 2 && idx >= dif0 then begin
          let pi = idx - dif0 in
          let phv =
            (U8.get dat pi lxor (U8.get dat (pi + 1) lsl bs1) lxor (U8.get dat (pi + 2) lsl bs2))
            land msk
          in
          if hv = phv then begin
            let maxn = (if n < rem then n else rem) - 1 in
            let maxd = if 32767 < idx then 32767 else idx in
            let ml = if 258 < rem then 258 else rem in
            let ch = ref c in
            let cur_imod = ref imod in
            let cur_pimod = ref pimod in
            let dif = ref dif0 in
            let keep = ref true in
            while !keep && !dif <= maxd && !ch > 1 && !cur_imod <> !cur_pimod do
              decr ch;
              let d_cur = !dif in
              let l_cur = !best_l in
              if U8.get dat (idx + l_cur) = U8.get dat (idx + l_cur - d_cur) then begin
                let nl = ref 0 in
                while !nl < ml && U8.get dat (idx + !nl) = U8.get dat (idx + !nl - d_cur) do
                  incr nl
                done;
                let nl_v = !nl in
                if nl_v > l_cur then begin
                  best_l := nl_v;
                  best_d := d_cur;
                  if nl_v > maxn then keep := false
                  else begin
                    let mmd =
                      let x = nl_v - 2 in
                      if d_cur < x then d_cur else x
                    in
                    let md = ref 0 in
                    for j = 0 to mmd - 1 do
                      let ti = (idx - d_cur + j) land 32767 in
                      let pti = U16.get prev ti in
                      let cd = (ti - pti) land 32767 in
                      if cd > !md then begin
                        md := cd;
                        cur_pimod := ti
                      end
                    done
                  end
                end
              end;
              if !keep then begin
                let next_imod = !cur_pimod in
                let next_pimod = U16.get prev next_imod in
                cur_imod := next_imod;
                cur_pimod := next_pimod;
                dif := d_cur + ((next_imod - next_pimod) land 32767)
              end
            done
          end
        end;
        let d_res = !best_d in
        if d_res <> 0 then begin
          let l_res = !best_l in
          let rfl = I32.get revfl l_res in
          let rfd = I32.get revfd d_res in
          I32.set syms !li (268435456 lor (rfl lsl 18) lor rfd);
          incr li;
          let lin = rfl land 31 in
          let din = rfd land 31 in
          eb := !eb + U8.get fleb lin + U8.get fdeb din;
          let lsym = 257 + lin in
          U16.set lf lsym (U16.get lf lsym + 1);
          U16.set df din (U16.get df din + 1);
          wi := idx + l_res;
          incr lc
        end
        else begin
          let b = U8.get dat idx in
          I32.set syms !li b;
          incr li;
          U16.set lf b (U16.get lf b + 1)
        end
      end;
      incr i
    done;
    let tail_i = if !i > !wi then !i else !wi in
    for k = tail_i to s - 1 do
      let b = U8.get dat k in
      I32.set syms !li b;
      incr li;
      U16.set lf b (U16.get lf b + 1)
    done;
    let final_i = if tail_i < s then s else tail_i in
    pos := wblk dat w lst syms lf df !eb !li !bs (final_i - !bs) !pos;
    if lst = 0 then begin
      let p_cur = !pos in
      st.r <- (p_cur land 7) lor (U8.get w (p_cur lsr 3) lsl 3);
      pos := p_cur - 7;
      st.h <- Some head;
      st.p <- Some prev;
      st.i <- final_i;
      st.w <- !wi
    end
  end
  else begin
    let i = ref st.w in
    let limit = s + lst in
    while !i < limit do
      let start = !i in
      let e = start + 65535 in
      let e =
        if e >= s then begin
          U8.set w (!pos lsr 3) lst;
          s
        end
        else e
      in
      pos := wfblk w (!pos + 1) dat start (e - start);
      i := start + 65535
    done;
    st.i <- s
  end;
  Tables.slc o 0 (pre + Tables.shft !pos + post)

let dopt dat opts pre post st_opt =
  let dat, st =
    match st_opt with
    | Some st -> (dat, st)
    | None -> (
        match opts.dictionary with
        | None -> (dat, fresh_dstate ~l:1 ~i:0 ~w:0 ~z:0)
        | Some dict_full ->
            let dict = Tables.last_32768 dict_full in
            let dl = U8.length dict in
            let new_dat = U8.create (dl + U8.length dat) in
            if dl > 0 then U8.set_from new_dat dict 0;
            if U8.length dat > 0 then U8.set_from new_dat dat dl;
            (new_dat, fresh_dstate ~l:1 ~i:0 ~w:dl ~z:0))
  in
  let lvl = match opts.level with Some lv -> lv | None -> 6 in
  let plvl =
    match opts.mem with
    | Some m -> 12 + m
    | None -> if st.l <> 0 then default_plvl (U8.length dat) else 20
  in
  dflt dat lvl plvl pre post st

let deflate_sync dat opts = dopt dat opts 0 0 None
