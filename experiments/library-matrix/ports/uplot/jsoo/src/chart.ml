(* The chart, shared byte for byte by the Melange and js_of_ocaml ports: the slice of uPlot 1.6.32
   (node_modules/uplot/dist/uPlot.esm.js) that the reference chart runs. It covers the rounding
   helpers, calcSize's plot box, getHPos/getVPos and posToVal, closestIdx and setScales' window,
   paths.linear with its gap scan, and drawPath, over the JS values a [Host] provides. As in the
   reference, the x scale has ori 0 and the y scale ori 1, both dir 1, and the series has
   pxAlign 1, alignGaps 0, width 1, no fill, no bands and no points. *)

module type Host = sig
  type v

  (* Reading the input. *)
  val field : v -> string -> v
  val at : v -> int -> v
  val len : v -> int
  val is_null : v -> bool
  val bool : v -> bool
  val num : v -> float

  (* Building the result. *)
  val number : float -> v
  val null : v
  val array : unit -> v
  val push : v -> v -> unit

  (* JS numbers: Math.round, Number.isInteger, String(x) and devicePixelRatio. *)
  val round : float -> float
  val is_integer : float -> bool
  val to_string : float -> string
  val pixel_ratio : unit -> float

  (* The canvas, appended to document.body, and its 2D context. *)
  val canvas : unit -> v
  val remove : v -> unit
  val context : v -> v
  val set_number : v -> string -> float -> unit
  val set_string : v -> string -> string -> unit
  val path : unit -> v
  val line : v -> float -> float -> unit
  val rect : v -> float -> float -> float -> float -> unit
  val save : v -> unit
  val restore : v -> unit
  val translate : v -> float -> float -> unit
  val clip : v -> v -> unit
  val stroke : v -> v -> unit
end

module Make (H : Host) = struct
  (* Number.EPSILON, 2^-52. *)
  let epsilon = 2.220446049250313e-16

  (* roundDec: rounds half away from zero at [dec] decimals. *)
  let round_dec ?(dec = 0) v =
    if H.is_integer v then v
    else
      let p = 10. ** float_of_int dec in
      H.round (v *. p *. (1. +. epsilon)) /. p

  (* uPlot's regex6, /\.\d*?(?=9{6,}|0{6,})/: the number of digits between the point and the first
     run of six or more 9s or 0s, if there is one. *)
  let decimals_before_run s =
    let n = String.length s in
    let rec find_dot i = if i >= n then None else if s.[i] = '.' then Some i else find_dot (i + 1) in
    match find_dot 0 with
    | None -> None
    | Some dot ->
        let rec digits_end i = if i < n && s.[i] >= '0' && s.[i] <= '9' then digits_end (i + 1) else i in
        let stop = digits_end (dot + 1) in
        let rec run c k count = count = 6 || (s.[k] = c && run c (k + 1) (count + 1)) in
        let rec find k =
          if k + 6 > stop then None
          else if run '9' k 0 || run '0' k 0 then Some (k - dot - 1)
          else find (k + 1)
        in
        find (dot + 1)

  (* fixFloat: snaps a value whose JS string has a run of six or more 9s or 0s after the point to
     the decimals before the run (17999.204999999998 becomes 17999.205). uPlot first returns
     members of its fixedDec map unchanged; the string path returns every member unchanged too.
     Its branch for strings with "e-" (values below 1e-6) is not reached by plot sizes of 1 px or
     more. *)
  let fix_float v =
    if H.is_integer v then v
    else match decimals_before_run (H.to_string v) with Some dec -> round_dec ~dec v | None -> v

  let incr_round v incr = fix_float (round_dec (fix_float (v /. incr)) *. incr)

  (* One axis of the plot: the scale range, calcSize's offset and size in canvas pixels, and the
     CSS size uPlot was given (the adapter divides the physical size by pxRatio). *)
  type scale = {
    min : float;
    max : float;
    off : float;
    dim : float;
    css : float;
    ratio : float;
    vertical : bool;
  }

  let num a i = H.num (H.at a i)

  let scale range ~css ~ratio ~vertical =
    {
      min = num range 0;
      max = num range 1;
      off = incr_round (0. *. ratio) 0.5;
      dim = incr_round (css *. ratio) 0.5;
      css;
      ratio;
      vertical;
    }

  (* getHPos / getVPos (dir 1) in canvas pixels, keeping uPlot's [off +], which turns -0 into 0. *)
  let pixel s v =
    let pct = (v -. s.min) /. (s.max -. s.min) in
    s.off +. (s.dim *. if s.vertical then 1. -. pct else pct)

  (* posToVal (p, key, true): the CSS position (the plot's CSS offset is 0), from the bottom for
     y. *)
  let value s p =
    let pos = p /. s.ratio in
    let q = if s.vertical then s.css -. pos else pos in
    s.min +. ((s.max -. s.min) *. (q /. s.css))

  (* closestIdx: JS [hi || arr.length - 1] makes a zero [hi] the last index; an empty array gives
     -1. *)
  let nearest xs v lo hi =
    let rec search l h =
      if h - l <= 1 then if v -. num xs l <= num xs h -. v then l else h
      else
        let m = (l + h) / 2 in
        if num xs m < v then search m h else search l m
    in
    if H.len xs = 0 then -1 else search lo (if hi = 0 then H.len xs - 1 else hi)

  let pair a b =
    let out = H.array () in
    H.push out a;
    H.push out b;
    out

  let coords a b = pair (H.number a) (H.number b)

  (* The adapter's precise(): Math.round(x * 1e6) || 0, which maps -0 and NaN to 0. *)
  let precise v =
    let x = H.round (v *. 1e6) in
    H.number (if x = 0. || x <> x then 0. else x)

  (* drawPath: the series width 1 in canvas pixels, the half-pixel translate, the bounds clip and
     clipGaps' mask. *)
  let draw ctx path gaps x y =
    let width = round_dec ~dec:3 x.ratio in
    let offset = mod_float width 2. /. 2. in
    if offset > 0. then H.translate ctx offset offset;
    let bounds = H.path () in
    H.rect bounds (x.off -. (width /. 2.)) (y.off -. (width /. 2.)) (x.dim +. width) (y.dim +. width);
    H.set_string ctx "strokeStyle" "#172839";
    H.set_number ctx "lineWidth" width;
    H.set_string ctx "lineJoin" "round";
    H.set_string ctx "lineCap" "butt";
    H.save ctx;
    H.clip ctx bounds;
    if H.len gaps > 0 then begin
      let mask = H.path () in
      let last = ref x.off in
      for i = 0 to H.len gaps - 1 do
        let gap = H.at gaps i in
        let a = num gap 0 and b = num gap 1 in
        if b > a then begin
          if a -. !last > 0. then H.rect mask !last y.off (a -. !last) (y.off +. y.dim);
          last := b
        end
      done;
      let rest = x.off +. x.dim -. !last in
      if rest > 0. then H.rect mask !last (y.off -. 5.) rest (y.off +. y.dim +. 10.);
      H.clip ctx mask
    end;
    if width > 0. then H.stroke ctx path;
    H.restore ctx;
    if offset > 0. then H.translate ctx (-.offset) (-.offset)

  (* One pixel column while decimating: the first and last values and the extremes. *)
  type bucket = { first : float; mutable last : float; mutable low : float; mutable high : float }

  let render ctx x y xs ys span =
    let path = H.path () and pts = H.array () and gaps = H.array () in
    let point a b =
      H.line path a b;
      H.push pts (coords a b)
    in
    let roundy v = H.round (pixel y v) in
    let flush col b =
      let low = roundy b.low and high = roundy b.high in
      let first = roundy b.first and last = roundy b.last in
      if low <> high then begin
        if first <> low && last <> low then point col low;
        if first <> high && last <> high then point col high;
        point col last
      end
    in
    let n = H.len ys and first = ref 0 in
    while !first < n && H.is_null (H.at ys !first) do
      incr first
    done;
    if !first < n then begin
      let last = ref (n - 1) in
      while !last > !first && H.is_null (H.at ys !last) do
        decr last
      done;
      let dense = float_of_int (!last - !first) >= x.dim *. 4. in
      let col = ref (H.round (pixel x (num xs !first))) in
      let limit = ref (value x (!col +. 1.)) and bucket = ref None in
      for i = !first to !last do
        let xv = num xs i in
        if dense && xv >= !limit then begin
          let next = H.round (pixel x xv) in
          if next <> !col then begin
            Option.iter (flush !col) !bucket;
            bucket := None;
            col := next;
            limit := value x (next +. 1.)
          end
        end;
        if not (H.is_null (H.at ys i)) then begin
          let v = num ys i in
          if not dense then point (H.round (pixel x xv)) (roundy v)
          else
            match !bucket with
            | None ->
                point !col (roundy v);
                bucket := Some { first = v; last = v; low = v; high = v }
            | Some b ->
                b.last <- v;
                if v < b.low then b.low <- v else if v > b.high then b.high <- v
        end
      done;
      Option.iter (flush !col) !bucket;
      if not span then begin
        let i = ref !first in
        while !i <= !last do
          if H.is_null (H.at ys !i) then begin
            let a = H.round (pixel x (num xs (!i - 1))) in
            while !i < !last && H.is_null (H.at ys !i) do
              incr i
            done;
            let b = H.round (pixel x (num xs !i)) in
            if b >= a then H.push gaps (coords a b)
          end;
          incr i
        done
      end
    end;
    draw ctx path gaps x y;
    pair pts (if span then H.null else gaps)

  let frame canvas ctx f span ratio =
    (* The adapter passes CSS sizes; uPlot multiplies them back by pxRatio. *)
    let size = H.field f "size" in
    let w = num size 0 /. ratio and h = num size 1 /. ratio in
    H.set_number canvas "width" (H.round (w *. ratio));
    H.set_number canvas "height" (H.round (h *. ratio));
    let x = scale (H.field f "xRange") ~css:w ~ratio ~vertical:false in
    let y = scale (H.field f "yRange") ~css:h ~ratio ~vertical:true in
    let data = H.field f "data" in
    let xs = H.at data 0 in
    let paths = H.array () in
    for i = 1 to H.len data - 1 do
      (* drawSeries runs only with data; the adapter then reports an empty trace. *)
      H.push paths
        (if H.len xs > 0 then render ctx x y xs (H.at data i) span
         else pair (H.array ()) (if span then H.null else H.array ()))
    done;
    let end_ = H.len xs - 1 in
    let lo = ref (nearest xs x.min 0 end_) and hi = ref (nearest xs x.max 0 end_) in
    if !hi > !lo + 1 then begin
      if num xs !lo < x.min then incr lo;
      if num xs !hi > x.max then decr hi
    end;
    let probes = H.field f "probes" and answers = H.array () in
    for i = 0 to H.len probes - 1 do
      let q = H.at probes i in
      let xv = num q 0 and px = num q 2 in
      let result = pair (precise (pixel x xv)) (precise (pixel y (num q 1))) in
      H.push result (precise (value x px));
      H.push result (precise (value y (num q 3)));
      H.push result (H.number (float_of_int (nearest xs xv 0 end_)));
      H.push result (H.number (float_of_int (nearest xs (value x px) !lo !hi)));
      H.push answers result
    done;
    let out = pair (H.field canvas "width") (H.field canvas "height") in
    H.push out paths;
    H.push out answers;
    out

  let frames canvas input =
    let ctx = H.context canvas in
    let ratio = H.pixel_ratio () in
    let span = H.bool (H.field input "spanGaps") in
    let frames = H.field input "frames" and out = H.array () in
    for i = 0 to H.len frames - 1 do
      H.push out (frame canvas ctx (H.at frames i) span ratio)
    done;
    out

  (* The reference removes its host in [finally]. *)
  let run input =
    let canvas = H.canvas () in
    match frames canvas input with
    | out ->
        H.remove canvas;
        out
    | exception e ->
        H.remove canvas;
        raise e
end
