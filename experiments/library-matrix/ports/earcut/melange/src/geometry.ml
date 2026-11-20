(* Ear clipping and polygon predicates, adapted from Mapbox earcut 3.2.4 (ISC). *)

type node = {
  i : int; x : float; y : float;
  mutable prev : node; mutable next : node;
  mutable z : int; mutable prev_z : node option; mutable next_z : node option;
  mutable steiner : bool;
}

let make_node i x y =
  let rec node = { i; x; y; prev = node; next = node; z = 0;
                   prev_z = None; next_z = None; steiner = false } in
  node

let equals a b = a.x = b.x && a.y = b.y
let area p q r = (q.y -. p.y) *. (r.x -. q.x) -. (q.x -. p.x) *. (r.y -. q.y)
let inside_triangle ax ay bx by cx cy px py =
  (cx -. px) *. (ay -. py) >= (ax -. px) *. (cy -. py) &&
  (ax -. px) *. (by -. py) >= (bx -. px) *. (ay -. py) &&
  (bx -. px) *. (cy -. py) >= (cx -. px) *. (by -. py)

let on_segment p q r =
  q.x <= max p.x r.x && q.x >= min p.x r.x &&
  q.y <= max p.y r.y && q.y >= min p.y r.y

let crosses x y = (x > 0. && y < 0.) || (x < 0. && y > 0.)
let intersects a b c d boundary =
  let ab_c = area a b c and ab_d = area a b d in
  let cd_a = area c d a and cd_b = area c d b in
  (crosses ab_c ab_d && crosses cd_a cd_b) ||
  (boundary && ((ab_c = 0. && on_segment a c b) || (ab_d = 0. && on_segment a d b) ||
                (cd_a = 0. && on_segment c a d) || (cd_b = 0. && on_segment c b d)))

let locally_inside a b =
  if area a.prev a a.next < 0.
  then area a b a.next >= 0. && area a a.prev b >= 0.
  else area a b a.prev < 0. || area a a.next b < 0.

let intersects_polygon a b =
  let xmin = min a.x b.x and xmax = max a.x b.x in
  let ymin = min a.y b.y and ymax = max a.y b.y in
  let rec walk p =
    let n = p.next in
    let disjoint = (p.x > xmax && n.x > xmax) || (p.x < xmin && n.x < xmin) ||
                   (p.y > ymax && n.y > ymax) || (p.y < ymin && n.y < ymin) in
    if not disjoint && p.i <> a.i && n.i <> a.i && p.i <> b.i && n.i <> b.i &&
       intersects p n a b true then true
    else if n == a then false else walk n
  in walk a

let middle_inside a b =
  let x = (a.x +. b.x) /. 2. and y = (a.y +. b.y) /. 2. in
  let rec walk p inside =
    let n = p.next in
    let crossing = (p.y > y) <> (n.y > y) &&
                   x < (n.x -. p.x) *. (y -. p.y) /. (n.y -. p.y) +. p.x in
    let inside = inside <> crossing in
    if n == a then inside else walk n inside
  in walk a false

let valid_diagonal a b =
  let zero = equals a b && area a.prev a a.next > 0. && area b.prev b b.next > 0. in
  a.next.i <> b.i &&
  (zero || locally_inside a b && locally_inside b a &&
           (area a.prev a b.prev <> 0. || area a b.prev b <> 0.)) &&
  not (intersects_polygon a b) && (zero || middle_inside a b)

let split_polygon a b =
  let a2 = make_node a.i a.x a.y and b2 = make_node b.i b.x b.y in
  let an = a.next and bp = b.prev in
  a.next <- b; b.prev <- a;
  a2.next <- an; an.prev <- a2;
  b2.next <- a2; a2.prev <- b2;
  bp.next <- b2; b2.prev <- bp;
  b2

let signed_area data first limit dim =
  let rec sum i j total =
    if i >= limit then total
    else sum (i + dim) i (total +. (data.(j) -. data.(i)) *. (data.(i + 1) +. data.(j + 1)))
  in sum first (limit - dim) 0.

let deviation data holes dim triangles =
  let len = Array.length data and nh = Array.length holes in
  let outer = if nh = 0 then len else holes.(0) * dim in
  let polygon = ref (abs_float (signed_area data 0 outer dim)) in
  for i = 0 to nh - 1 do
    let limit = if i + 1 < nh then holes.(i+1) * dim else len in
    polygon := !polygon -. abs_float (signed_area data (holes.(i)*dim) limit dim)
  done;
  let rec sum i total =
    if i >= Array.length triangles then total else
    let a = triangles.(i)*dim and b = triangles.(i+1)*dim and c = triangles.(i+2)*dim in
    sum (i+3) (total +. abs_float ((data.(a) -. data.(c)) *. (data.(b+1) -. data.(a+1)) -.
                                   (data.(a) -. data.(b)) *. (data.(c+1) -. data.(a+1))))
  in
  let total = sum 0 0. in
  if total <> 0. then abs_float ((total -. !polygon) /. !polygon)
  else
    let rec extent i bound =
      if i >= len then bound else extent (i+dim) (max bound (max (abs_float data.(i)) (abs_float data.(i+1))))
    in
    let bound = extent 0 0. in
    if abs_float !polygon <= float_of_int len *. bound *. bound *. 2.220446049250313e-16 then 0. else 1.
