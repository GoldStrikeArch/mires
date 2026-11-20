(* Build half-edge twins with a linear-probing hash, then legalize interior
   edges with a LIFO worklist. All scratch arrays belong to one invocation. *)
let orient ax ay bx by_ cx cy = (bx -. ax) *. (cy -. ay) -. (by_ -. ay) *. (cx -. ax)

let in_circle ax ay bx by_ cx cy px py =
  let dx = ax -. px and dy = ay -. py and ex = bx -. px and ey = by_ -. py in
  let fx = cx -. px and fy = cy -. py in
  let ap = dx *. dx +. dy *. dy and bp = ex *. ex +. ey *. ey and cp = fx *. fx +. fy *. fy in
  let s = ap +. bp +. cp in
  dx *. (ey *. cp -. bp *. fy) -. dy *. (ex *. cp -. bp *. fx) +.
  ap *. (ex *. fy -. ey *. fx) <= 1e-13 *. s *. s

let next_edge e = e - e mod 3 + (e+1) mod 3

let refine triangles coords dim =
  let n = Array.length triangles in
  if n >= 6 then begin
    let size = ref 1 in while !size < 4*n do size := !size lsl 1 done;
    let mask = !size-1 in
    (* -2 is empty, -1 is a linked (consumed) pair. *)
    let table = Array.make !size (-2) and twin = Array.make n (-1) in
    let stack = Array.make n 0 and queued = Array.make n false and count = ref 0 in
    let push e =
      if twin.(e) <> -1 && not queued.(e) then begin
        queued.(e) <- true; stack.(!count) <- e; incr count
      end
    in
    for edge = 0 to n - 1 do
      let a = triangles.(edge) and b = triangles.(next_edge edge) in
      let lo = min a b and hi = max a b in
      let hash = ((lo * (-1640531535)) lxor (hi * (-2048144789))) land mask in
      let rec probe slot =
        let s = table.(slot) in
        if s = -2 then table.(slot) <- edge
        else if s >= 0 &&
                ((triangles.(s) = lo && triangles.(next_edge s) = hi) ||
                 (triangles.(s) = hi && triangles.(next_edge s) = lo)) then begin
          twin.(edge) <- s; twin.(s) <- edge; table.(slot) <- -1; push s
        end else probe ((slot+1) land mask)
      in probe hash
    done;
    while !count > 0 do
      decr count;
      let a = stack.(!count) in queued.(a) <- false;
      let b = twin.(a) in
      if b <> -1 then begin
        let a0 = a-a mod 3 and b0 = b-b mod 3 in
        let ar = a0 + (a+2) mod 3 and al = a0 + (a+1) mod 3 in
        let bl = b0 + (b+2) mod 3 and br = b0 + (b+1) mod 3 in
        let p0 = triangles.(ar) and pr = triangles.(a) and pl = triangles.(al) and p1 = triangles.(bl) in
        let x p = coords.(p*dim) and y p = coords.(p*dim+1) in
        let x0 = x p0 and y0 = y p0 and xr = x pr and yr = y pr in
        let xl = x pl and yl = y pl and x1 = x p1 and y1 = y p1 in
        if not (in_circle x0 y0 xr yr xl yl x1 y1) &&
           orient x0 y0 xr yr x1 y1 > 0. && orient x0 y0 x1 y1 xl yl > 0. then begin
          triangles.(a) <- p1; triangles.(b) <- p0;
          let hbl = twin.(bl) and har = twin.(ar) in
          twin.(a) <- hbl; if hbl <> -1 then twin.(hbl) <- a;
          twin.(b) <- har; if har <> -1 then twin.(har) <- b;
          twin.(ar) <- bl; twin.(bl) <- ar;
          push a; push b; push al; push br
        end
      end
    done
  end
