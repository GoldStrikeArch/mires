module type Host = sig
  type v
  val field : v -> string -> v
  val at : v -> int -> v
  val len : v -> int
  val is_null : v -> bool
  val bool : v -> bool
  val num : v -> float
  val number : float -> v
  val null : v
  val round : float -> float
  val array : unit -> v
  val push : v -> v -> unit
  val canvas : unit -> v
  val remove : v -> unit
  val context : v -> v
  val resize : v -> v -> float -> float -> unit
  val path : unit -> v
  val line : v -> float -> float -> unit
  val rect : v -> float -> float -> float -> float -> unit
  val pixel_ratio : unit -> float
  val stroke_style : v -> float -> unit
  val translate : v -> float -> float -> unit
  val save : v -> unit
  val restore : v -> unit
  val stroke : v -> v -> unit
  val clip : v -> v -> unit
end

module Make(H:Host) = struct
  type scale={min:float;max:float;dim:float;requested:float;vertical:bool}
  let num a i=H.num(H.at a i)
  let scale range dim vertical={min=num range 0;max=num range 1;dim=H.round(dim*.2.)/.2.;requested=dim;vertical}
  let pixel s v=let p=(v-.s.min)/.(s.max-.s.min) in s.dim*.(if s.vertical then 1.-.p else p)
  let value s p=let q=if s.vertical then s.requested-.p else p in s.min+.(s.max-.s.min)*.(q/.s.requested)
  let nearest xs v lo hi=
    let rec search l h=if h-l<=1 then (if v-.num xs l<=num xs h-.v then l else h)
      else let m=(l+h)/2 in if num xs m<v then search m h else search l m in
    if H.len xs=0 then -1 else search lo (if hi=0 then H.len xs-1 else hi)
  let pair a b=let out=H.array() in H.push out a;H.push out b;out
  let coords a b=pair (H.number a) (H.number b)
  let precise v=let x=H.round(v*.1e6) in H.number(if x=0. then 0. else x)

  let draw ctx path gaps w h=
    H.save ctx;
    Fun.protect ~finally:(fun()->H.restore ctx) (fun()->
      let width = H.round (H.pixel_ratio () *. 1000.) /. 1000. in
      let offset = mod_float width 2. /. 2. in
      H.stroke_style ctx width;
      H.translate ctx offset offset;
      let bounds = H.path () in
      H.rect bounds (-.width /. 2.) (-.width /. 2.) (w +. width) (h +. width);
      H.clip ctx bounds;
      if H.len gaps>0 then begin
        let mask=H.path() and last=ref 0. in
        for i=0 to H.len gaps-1 do
          let gap=H.at gaps i in let a=num gap 0 and b=num gap 1 in
          if b>a then begin if a> !last then H.rect mask !last 0. (a-. !last) h;last:=b end
        done;
        if w> !last then H.rect mask !last (-5.) (w-. !last) (h+.10.);
        H.clip ctx mask
      end;
      H.stroke ctx path)

  type bucket={first:float;mutable last:float;mutable low:float;mutable high:float}
  let render ctx x y xs ys span=
    let path=H.path() and pts=H.array() and gaps=H.array() in
    let point a b=H.line path a b;H.push pts (coords a b) in
    let roundy v=H.round(pixel y v) in
    let flush col b=
      let low=roundy b.low and high=roundy b.high and first=roundy b.first and last=roundy b.last in
      if low<>high then begin
        if first<>low && last<>low then point col low;
        if first<>high && last<>high then point col high;point col last
      end in
    let n=H.len ys and first=ref 0 in
    while !first<n && H.is_null(H.at ys !first) do incr first done;
    if !first<n then begin
      let last=ref (n-1) in while !last> !first && H.is_null(H.at ys !last) do decr last done;
      let dense=float_of_int(!last- !first)>=x.dim*.4. in
      let col=ref(H.round(pixel x (num xs !first))) in
      let limit=ref(value x (!col+.1.)) and bucket=ref None in
      for i= !first to !last do
        let xv=num xs i in
        if dense && xv>= !limit then begin
          let next=H.round(pixel x xv) in if next<> !col then begin
            Option.iter (flush !col) !bucket;bucket:=None;col:=next;limit:=value x (next+.1.)
          end
        end;
        if not(H.is_null(H.at ys i)) then begin
          let v=num ys i in
          if not dense then point (H.round(pixel x xv)) (roundy v)
          else match !bucket with
            | None->point !col (roundy v);bucket:=Some{first=v;last=v;low=v;high=v}
            | Some b->b.last<-v;if v<b.low then b.low<-v else if v>b.high then b.high<-v
        end
      done;
      Option.iter (flush !col) !bucket;
      if not span then begin
        let i=ref !first in
        while !i<= !last do
          if H.is_null(H.at ys !i) then begin
            let a=H.round(pixel x (num xs (!i-1))) in
            while !i< !last && H.is_null(H.at ys !i) do incr i done;
            let b=H.round(pixel x (num xs !i)) in if b>=a then H.push gaps (coords a b)
          end;
          incr i
        done
      end
    end;
    draw ctx path gaps x.dim y.dim;pair pts (if span then H.null else gaps)

  let frame canvas ctx f span=
    let dims=H.field f "size" in let w=num dims 0 and h=num dims 1 in
    H.resize canvas ctx (H.round w) (H.round h);
    let x=scale (H.field f "xRange") w false and y=scale (H.field f "yRange") h true in
    let data=H.field f "data" and paths=H.array() in let xs=H.at data 0 in
    for i=1 to H.len data-1 do H.push paths (render ctx x y xs (H.at data i) span) done;
    let end_=H.len xs-1 in let lo=ref(nearest xs x.min 0 end_) and hi=ref(nearest xs x.max 0 end_) in
    if !hi> !lo+1 then begin if num xs !lo<x.min then incr lo;if num xs !hi>x.max then decr hi end;
    let probes=H.field f "probes" and answers=H.array() in
    for i=0 to H.len probes-1 do
      let q=H.at probes i in let xv=num q 0 and px=num q 2 in
      let result=pair (precise(pixel x xv)) (precise(pixel y (num q 1))) in
      H.push result (precise(value x px));H.push result (precise(value y (num q 3)));
      H.push result (H.number(float_of_int(nearest xs xv 0 end_)));
      H.push result (H.number(float_of_int(nearest xs (value x px) !lo !hi)));H.push answers result
    done;
    let out=pair (H.field canvas "width") (H.field canvas "height") in H.push out paths;H.push out answers;out

  let run input=
    let canvas=H.canvas() in
    Fun.protect ~finally:(fun()->H.remove canvas) (fun()->
      let ctx=H.context canvas and frames=H.field input "frames" and out=H.array() in
      let span=H.bool(H.field input "spanGaps") in
      for i=0 to H.len frames-1 do H.push out (frame canvas ctx (H.at frames i) span) done;out)
end
