(* Canonical Huffman maps, UZIP.js linear-merge tree builder with length clamping,
   code-length RLE encoder, and bit-cost calculation (fflate 0.8.3). *)

open Host

let h_map cd mb rev_mode =
  let s = U8.length cd in
  let l = U16.create mb in
  for i = 0 to s - 1 do
    let c = U8.get cd i in
    if c <> 0 then U16.set l (c - 1) (U16.get l (c - 1) + 1)
  done;
  let le = U16.create mb in
  for i = 1 to mb - 1 do
    U16.set le i (((U16.get le (i - 1) + U16.get l (i - 1)) lsl 1) land 0xffff)
  done;
  if rev_mode then begin
    let co = U16.create (1 lsl mb) in
    let rvb = 15 - mb in
    let rev = Tables.rev in
    for i = 0 to s - 1 do
      let c = U8.get cd i in
      if c <> 0 then begin
        let sv = (i lsl 4) lor c in
        let r1 = mb - c in
        let cur = U16.get le (c - 1) in
        U16.set le (c - 1) ((cur + 1) land 0xffff);
        let v0 = cur lsl r1 in
        let m = v0 lor ((1 lsl r1) - 1) in
        for v = v0 to m do
          let idx = if v < 32768 then U16.get rev v lsr rvb else 0 in
          U16.set co idx sv
        done
      end
    done;
    co
  end
  else begin
    let co = U16.create s in
    let rev = Tables.rev in
    for i = 0 to s - 1 do
      let c = U8.get cd i in
      if c <> 0 then begin
        let cur = U16.get le (c - 1) in
        U16.set le (c - 1) ((cur + 1) land 0xffff);
        let rv = if cur < 32768 then U16.get rev cur else 0 in
        U16.set co i (rv lsr (15 - c))
      end
    done;
    co
  end

(* Immutable precomputed fixed Huffman tables *)
let flm = h_map Tables.flt 9 false
let flrm = h_map Tables.flt 9 true
let fdm = h_map Tables.fdt 5 false
let fdrm = h_map Tables.fdt 5 true

(* Stable sort of parallel (sym, freq) slices [0..n-1] by ascending (freq, sym).
   n <= 288; bottom-up merge sort is O(n log n), stable, allocation-light, and avoids Array.stable_sort. *)
let sort_by_freq_sym (sym : int array) (freq : int array) n =
  let aux_s = Array.make n 0 in
  let aux_f = Array.make n 0 in
  let width = ref 1 in
  while !width < n do
    let w = !width in
    let i = ref 0 in
    while !i < n do
      let lo = !i in
      let mid = let m = lo + w in if m < n then m else n in
      let hi = let h = lo + (w lsl 1) in if h < n then h else n in
      let a = ref lo in
      let b = ref mid in
      let k = ref lo in
      while !a < mid && !b < hi do
        let fa = freq.(!a) in
        let fb = freq.(!b) in
        if fa < fb || (fa = fb && sym.(!a) <= sym.(!b)) then begin
          aux_s.(!k) <- sym.(!a);
          aux_f.(!k) <- fa;
          incr a
        end
        else begin
          aux_s.(!k) <- sym.(!b);
          aux_f.(!k) <- fb;
          incr b
        end;
        incr k
      done;
      while !a < mid do
        aux_s.(!k) <- sym.(!a);
        aux_f.(!k) <- freq.(!a);
        incr a;
        incr k
      done;
      while !b < hi do
        aux_s.(!k) <- sym.(!b);
        aux_f.(!k) <- freq.(!b);
        incr b;
        incr k
      done;
      for j = lo to hi - 1 do
        sym.(j) <- aux_s.(j);
        freq.(j) <- aux_f.(j)
      done;
      i := hi
    done;
    width := w lsl 1
  done

(* Stable sort of parallel (sym, freq) slices [0..n-1] by descending tr[sym], ascending freq, ascending sym. *)
let sort_by_depth_freq_sym (tr : int array) (sym : int array) (freq : int array) n =
  let aux_s = Array.make n 0 in
  let aux_f = Array.make n 0 in
  let width = ref 1 in
  while !width < n do
    let w = !width in
    let i = ref 0 in
    while !i < n do
      let lo = !i in
      let mid = let m = lo + w in if m < n then m else n in
      let hi = let h = lo + (w lsl 1) in if h < n then h else n in
      let a = ref lo in
      let b = ref mid in
      let k = ref lo in
      while !a < mid && !b < hi do
        let sa = sym.(!a) in
        let sb = sym.(!b) in
        let da = tr.(sa) in
        let db = tr.(sb) in
        let take_a =
          if da <> db then da > db
          else
            let fa = freq.(!a) in
            let fb = freq.(!b) in
            if fa <> fb then fa < fb else sa <= sb
        in
        if take_a then begin
          aux_s.(!k) <- sa;
          aux_f.(!k) <- freq.(!a);
          incr a
        end
        else begin
          aux_s.(!k) <- sb;
          aux_f.(!k) <- freq.(!b);
          incr b
        end;
        incr k
      done;
      while !a < mid do
        aux_s.(!k) <- sym.(!a);
        aux_f.(!k) <- freq.(!a);
        incr a;
        incr k
      done;
      while !b < hi do
        aux_s.(!k) <- sym.(!b);
        aux_f.(!k) <- freq.(!b);
        incr b;
        incr k
      done;
      for j = lo to hi - 1 do
        sym.(j) <- aux_s.(j);
        freq.(j) <- aux_f.(j)
      done;
      i := hi
    done;
    width := w lsl 1
  done

let rec assign_depths node_s node_l node_r tr idx depth =
  let sv = node_s.(idx) in
  if sv = -1 then
    let dl = assign_depths node_s node_l node_r tr node_l.(idx) (depth + 1) in
    let dr = assign_depths node_s node_l node_r tr node_r.(idx) (depth + 1) in
    if dl > dr then dl else dr
  else begin
    tr.(sv) <- depth;
    depth
  end

let h_tree d mb =
  let dlen = U16.length d in
  let count = ref 0 in
  for i = 0 to dlen - 1 do
    if U16.get d i <> 0 then incr count
  done;
  let s = !count in
  if s = 0 then (Tables.empty_u8, 0)
  else if s = 1 then begin
    let sym = ref 0 in
    let i = ref 0 in
    while !i < dlen do
      if U16.get d !i <> 0 then begin
        sym := !i;
        i := dlen
      end
      else incr i
    done;
    let v = U8.create (!sym + 1) in
    U8.set v !sym 1;
    (v, 1)
  end
  else begin
    (* Flat node storage: [0..s-1] sorted leaves, [s] sentinel, [s+1..2*s-1] internal nodes. *)
    let total = s lsl 1 in
    let node_s = Array.make total (-1) in
    let node_f = Array.make total 0 in
    let node_l = Array.make total (-1) in
    let node_r = Array.make total (-1) in
    let t2_s = Array.make s 0 in
    let t2_f = Array.make s 0 in
    let k = ref 0 in
    for i = 0 to dlen - 1 do
      let f = U16.get d i in
      if f <> 0 then begin
        let idx = !k in
        node_s.(idx) <- i;
        node_f.(idx) <- f;
        t2_s.(idx) <- i;
        t2_f.(idx) <- f;
        incr k
      end
    done;
    let max_sym = t2_s.(s - 1) in
    sort_by_freq_sym node_s node_f s;
    (* Sentinel at index s *)
    node_s.(s) <- -1;
    node_f.(s) <- 25001;
    (* UZIP.js two-queue linear merge; strict '<' prefers leaves on frequency ties *)
    let q_base = s + 1 in
    node_s.(q_base) <- -1;
    node_f.(q_base) <- node_f.(0) + node_f.(1);
    node_l.(q_base) <- 0;
    node_r.(q_base) <- 1;
    let i0 = ref q_base in
    let i1 = ref (q_base + 1) in
    let i2 = ref 2 in
    let target_i1 = q_base + s - 1 in
    while !i1 <> target_i1 do
      let l_idx =
        if node_f.(!i0) < node_f.(!i2) then (
          let x = !i0 in
          incr i0;
          x)
        else (
          let x = !i2 in
          incr i2;
          x)
      in
      let r_idx =
        if !i0 <> !i1 && node_f.(!i0) < node_f.(!i2) then (
          let x = !i0 in
          incr i0;
          x)
        else (
          let x = !i2 in
          incr i2;
          x)
      in
      let dst = !i1 in
      node_s.(dst) <- -1;
      node_f.(dst) <- node_f.(l_idx) + node_f.(r_idx);
      node_l.(dst) <- l_idx;
      node_r.(dst) <- r_idx;
      incr i1
    done;
    let tr = Array.make (max_sym + 1) 0 in
    let mbt = ref (assign_depths node_s node_l node_r tr (!i1 - 1) 0) in
    if !mbt > mb then begin
      let cur_mbt = !mbt in
      let lft = cur_mbt - mb in
      let cst = 1 lsl lft in
      sort_by_depth_freq_sym tr t2_s t2_f s;
      let i = ref 0 in
      let dt = ref 0 in
      while !i < s && tr.(t2_s.(!i)) > mb do
        let sym_i = t2_s.(!i) in
        dt := !dt + cst - (1 lsl (cur_mbt - tr.(sym_i)));
        tr.(sym_i) <- mb;
        incr i
      done;
      dt := !dt asr lft;
      while !dt > 0 do
        let sym_i = t2_s.(!i) in
        let d_cur = tr.(sym_i) in
        if d_cur < mb then begin
          tr.(sym_i) <- d_cur + 1;
          dt := !dt - (1 lsl (mb - d_cur - 1))
        end
        else incr i
      done;
      while !i >= 0 && !dt <> 0 do
        let sym_i = t2_s.(!i) in
        if tr.(sym_i) = mb then begin
          tr.(sym_i) <- mb - 1;
          incr dt
        end;
        decr i
      done;
      mbt := mb
    end;
    let out = U8.create (max_sym + 1) in
    for j = 0 to max_sym do
      let v = tr.(j) in
      if v <> 0 then U8.set out j v
    done;
    (out, !mbt)
  end

let lc c =
  let len = U8.length c in
  let s = ref len in
  while !s > 0 && U8.get c (!s - 1) = 0 do
    decr s
  done;
  (* Empty or all-zero c yields s = 1, producing { c: [0], n: 1 } just like fflate. *)
  let s = if !s = 0 then 1 else !s in
  let cl = U16.create s in
  let cli = ref 0 in
  let cln = ref (if len > 0 then U8.get c 0 else 0) in
  let cls = ref 1 in
  let emit v =
    U16.set cl !cli v;
    incr cli
  in
  for i = 1 to s do
    let ci = if i < len then U8.get c i else 0 in
    if i <> s && ci = !cln then incr cls
    else begin
      let cur_cln = !cln in
      let cur_cls = ref !cls in
      if cur_cln = 0 && !cur_cls > 2 then begin
        while !cur_cls > 138 do
          emit 32754;
          cur_cls := !cur_cls - 138
        done;
        if !cur_cls > 2 then begin
          let v =
            if !cur_cls > 10 then ((!cur_cls - 11) lsl 5) lor 28690
            else ((!cur_cls - 3) lsl 5) lor 12305
          in
          emit v;
          cur_cls := 0
        end
      end
      else if !cur_cls > 3 then begin
        emit cur_cln;
        decr cur_cls;
        while !cur_cls > 6 do
          emit 8304;
          cur_cls := !cur_cls - 6
        done;
        if !cur_cls > 2 then begin
          emit (((!cur_cls - 3) lsl 5) lor 8208);
          cur_cls := 0
        end
      end;
      while !cur_cls > 0 do
        emit cur_cln;
        decr cur_cls
      done;
      cls := 1;
      cln := ci
    end
  done;
  (U16.subarray cl 0 !cli, s)

let clen cf cl =
  let len = U8.length cl in
  let sum = ref 0 in
  for i = 0 to len - 1 do
    sum := !sum + (U16.get cf i * U8.get cl i)
  done;
  !sum
