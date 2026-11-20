open Geometry

type block = {
  mutable head : node; mutable stop : node;
  mutable xmin : float; mutable ymin : float; mutable xmax : float; mutable ymax : float;
}
type state = {
  mutable blocks : block Vector.t option;
  mutable filtered : bool;
}

let grow b n =
  if n.x < b.xmin then b.xmin <- n.x;
  if n.x > b.xmax then b.xmax <- n.x;
  if n.y < b.ymin then b.ymin <- n.y;
  if n.y > b.ymax then b.ymax <- n.y

let remove st p =
  p.next.prev <- p.prev; p.prev.next <- p.next;
  (match p.prev_z with None -> () | Some prev -> prev.next_z <- p.next_z);
  (match p.next_z with None -> () | Some next -> next.prev_z <- p.prev_z);
  match st.blocks with None -> () | Some blocks -> grow blocks.items.(p.prev.z) p.next

let linked_list st data first limit dim clockwise =
  let last = ref None in
  let insert i =
    let p = make_node (i / dim) data.(i) data.(i+1) in
    (match !last with None -> () | Some n ->
      p.next <- n.next; p.prev <- n; n.next.prev <- p; n.next <- p);
    last := Some p
  in
  if clockwise = (signed_area data first limit dim > 0.) then begin
    let i = ref first in
    while !i < limit do insert !i; i := !i + dim done
  end else begin
    let i = ref (limit - dim) in
    while !i >= first do insert !i; i := !i - dim done
  end;
  match !last with
  | Some p when equals p p.next -> remove st p; Some p.next
  | x -> x

let filter st start stop =
  let full = start == stop in
  let rec walk p bound =
    if p != p.next && not p.steiner && (equals p p.next || area p.prev p p.next = 0.) then begin
      let bound = if full || p == bound then p.prev else bound in
      st.filtered <- true;
      remove st p;
      walk p.prev bound
    end else if full then
      if p.next == bound then bound else walk p.next bound
    else if p == bound then bound else walk p.next bound
  in walk start stop

let index_segment blocks head stop =
  let rec segment head =
    let block = {head; stop = head; xmin = head.x; ymin = head.y; xmax = head.x; ymax = head.y} in
    let idx = blocks.Vector.size in
    let rec edges p k =
      let q = p.next in
      p.z <- idx; grow block q;
      if k = 16 || q == stop then q else edges q (k+1)
    in
    let bound = edges head 1 in
    block.stop <- bound;
    Vector.push blocks block;
    if bound != stop then segment bound
  in segment head

let rec live p = if p.prev.next == p then p else live p.next

(* Each retained block can outlive a removed head or stop. Repair both before
   visiting its live edges, preserving the upstream append order. *)
let scan_block block visit =
  let stop = live block.stop in
  block.stop <- stop;
  let head = live block.head in
  block.head <- head;
  let rec walk p =
    if p.prev.next == p then visit p;
    if p.next != stop then walk p.next
  in walk head

exception Touch of node

let find_bridge blocks hole outer =
  if equals hole outer then Some outer else
  let hx = hole.x and hy = hole.y in
  let qx = ref neg_infinity and candidate = ref None in
  try
    for i = 0 to blocks.Vector.size - 1 do
      let b = blocks.items.(i) in
      if not (hy < b.ymin || hy > b.ymax || b.xmin > hx || b.xmax <= !qx) then
        scan_block b (fun p ->
          let n = p.next in
          if equals hole n then raise (Touch n);
          if hy <= p.y && hy >= n.y && n.y <> p.y then begin
            let x = p.x +. (hy -. p.y) *. (n.x -. p.x) /. (n.y -. p.y) in
            if x <= hx && x > !qx then begin
              qx := x;
              let m = if p.x < n.x then p else n in
              candidate := Some m;
              if x = hx then raise (Touch m)
            end
          end)
    done;
    match !candidate with
    | None -> None
    | Some initial ->
      let mx = initial.x and my = initial.y in
      let ymin = min hy my and ymax = max hy my in
      let ax = if hy < my then hx else !qx and cx = if hy < my then !qx else hx in
      let best = ref initial and angle = ref infinity in
      for i = 0 to blocks.size - 1 do
        let b = blocks.items.(i) in
        if not (b.xmax < mx || b.xmin > hx || b.ymax < ymin || b.ymin > ymax) then
          scan_block b (fun p ->
            if hx >= p.x && p.x >= mx && hx <> p.x &&
               inside_triangle ax hy mx my cx hy p.x p.y then begin
              let tan = abs_float (hy -. p.y) /. (hx -. p.x) and m = !best in
              if (locally_inside p hole || (p.y = hy && p.next.y = hy && p.next.x > hx)) &&
                 (tan < !angle || (tan = !angle &&
                    (p.x > m.x || (p.x = m.x && area m.prev m p.prev < 0. && area p.next m m.next < 0.))))
              then begin best := p; angle := tan end
            end)
      done;
      Some !best
  with Touch n -> Some n

let leftmost start =
  let rec walk p best =
    let best = if p.x < best.x || (p.x = best.x && p.y < best.y) then p else best in
    if p.next == start then best else walk p.next best
  in walk start start

let hole_order a b =
  let cmp x = if x < 0. then -1 else if x > 0. then 1 else 0 in
  if a.x <> b.x then cmp (a.x -. b.x)
  else if a.y <> b.y then cmp (a.y -. b.y)
  else cmp ((a.next.y -. a.y) /. (a.next.x -. a.x) -. (b.next.y -. b.y) /. (b.next.x -. b.x))

let eliminate_holes st data holes dim outer =
  let nh = Array.length holes in
  let queue = Array.init nh (fun i ->
    let limit = if i + 1 < nh then holes.(i+1)*dim else Array.length data in
    match linked_list st data (holes.(i)*dim) limit dim false with
    | None -> invalid_arg "empty hole"
    | Some ring -> if ring == ring.next then ring.steiner <- true; leftmost ring)
  in
  Array.stable_sort hole_order queue;
  let blocks = Vector.create {head=outer; stop=outer; xmin=0.; ymin=0.; xmax=0.; ymax=0.} in
  index_segment blocks outer outer;
  st.blocks <- Some blocks;
  let root = ref outer in
  for i = 0 to nh - 1 do
    let hole = queue.(i) in
    match find_bridge blocks hole !root with
    | None -> ()
    | Some bridge ->
      let rev = split_polygon bridge hole in
      index_segment blocks bridge rev.next.next;
      ignore (filter st rev rev.next);
      root := filter st bridge bridge.next
  done;
  st.blocks <- None;
  filter st !root !root
