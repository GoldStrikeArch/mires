open Geometry
open Rings

let spread x =
  let x = (x lor (x lsl 8)) land 0x00ff00ff in
  let x = (x lor (x lsl 4)) land 0x0f0f0f0f in
  let x = (x lor (x lsl 2)) land 0x33333333 in
  (x lor (x lsl 1)) land 0x55555555

let z_order x y xmin ymin inv =
  spread (int_of_float ((x -. xmin) *. inv)) lor (spread (int_of_float ((y -. ymin) *. inv)) lsl 1)

let sort_nodes nodes n =
  if n <= 32 then begin
    for i = 1 to n - 1 do
      let p = nodes.(i) in
      let j = ref (i-1) in
      while !j >= 0 && nodes.(!j).z > p.z do
        nodes.(!j+1) <- nodes.(!j); decr j
      done;
      nodes.(!j+1) <- p
    done
  end else begin
    let tmp = Array.make n nodes.(0) and keys = Array.init n (fun i -> nodes.(i).z) in
    let next_keys = Array.make n 0 and counts = Array.make 256 0 in
    let pass src src_keys dst dst_keys shift =
      Array.fill counts 0 256 0;
      for i = 0 to n - 1 do
        let bucket = (src_keys.(i) lsr shift) land 255 in
        counts.(bucket) <- counts.(bucket) + 1
      done;
      let total = ref 0 in
      for bucket = 0 to 255 do
        let count = counts.(bucket) in counts.(bucket) <- !total; total := !total + count
      done;
      for i = 0 to n - 1 do
        let key = src_keys.(i) in
        let bucket = (key lsr shift) land 255 in
        let offset = counts.(bucket) in
        dst.(offset) <- src.(i); dst_keys.(offset) <- key; counts.(bucket) <- offset + 1
      done
    in
    pass nodes keys tmp next_keys 0;
    pass tmp next_keys nodes keys 8;
    pass nodes keys tmp next_keys 16;
    pass tmp next_keys nodes keys 24
  end

let index_curve root xmin ymin inv =
  let nodes = Vector.create root in
  let rec collect p =
    p.z <- z_order p.x p.y xmin ymin inv; Vector.push nodes p;
    if p.next != root then collect p.next
  in collect root;
  sort_nodes nodes.items nodes.size;
  let prev = ref None in
  for i = 0 to nodes.size - 1 do
    let p = nodes.items.(i) in
    p.prev_z <- !prev;
    (match !prev with None -> () | Some before -> before.next_z <- Some p);
    prev := Some p
  done;
  nodes.items.(nodes.size-1).next_z <- None

let is_ear b xmin ymin inv =
  let a = b.prev and c = b.next in
  let x0 = min a.x (min b.x c.x) and y0 = min a.y (min b.y c.y) in
  let x1 = max a.x (max b.x c.x) and y1 = max a.y (max b.y c.y) in
  let blocks p =
    p.x >= x0 && p.x <= x1 && p.y >= y0 && p.y <= y1 &&
    not (a.x = p.x && a.y = p.y) &&
    inside_triangle a.x a.y b.x b.y c.x c.y p.x p.y && area p.prev p p.next >= 0.
  in
  if inv = 0. then
    let rec walk p =
      if p == a then true else if blocks p then false else walk p.next
    in walk c.next
  else
    let low = z_order x0 y0 xmin ymin inv and high = z_order x1 y1 xmin ymin inv in
    let rec backward link = match link with
      | Some p when p.z >= low ->
        if p != c && blocks p then false else backward p.prev_z
      | _ -> true
    in
    let rec forward link = match link with
      | Some p when p.z <= high ->
        if p != c && blocks p then false else forward p.next_z
      | _ -> true
    in backward b.prev_z && forward b.next_z

let triangle out a b c = Vector.push out a.i; Vector.push out b.i; Vector.push out c.i

let cure st root out =
  let rec scan p start cured =
    let a = p.prev and b = p.next.next in
    let p, start, cured =
      if intersects a p p.next b false && locally_inside a b && locally_inside b a then begin
        triangle out a p b;
        remove st p; remove st p.next;
        b, b, true
      end else p, start, cured
    in
    if p.next == start then (if cured then filter st start start else start)
    else scan p.next start cured
  in scan root root false

let rec clip_ring st root out xmin ymin inv =
  if inv <> 0. then index_curve root xmin ymin inv;
  let rec slice p stop cured =
    let prev = p.prev and next = p.next in
    if prev != next then
      if area prev p next < 0. && is_ear p xmin ymin inv then begin
        triangle out prev p next;
        remove st p;
        slice next next cured
      end else if next != stop then slice next stop cured
      else begin
        st.filtered <- false;
        let p = filter st next next in
        if st.filtered then slice p p cured
        else if not cured then let p = cure st p out in slice p p true
        else split_ring st p out xmin ymin inv
      end
  in slice root root false

and split_ring st root out xmin ymin inv =
  let rec find_b a b =
    if b == a.prev then None
    else if a.i <> b.i && valid_diagonal a b then Some b
    else find_b a b.next
  in
  let rec find_a a =
    match find_b a a.next.next with
    | Some b ->
      let c = split_polygon a b in
      let a = filter st a a.next in
      let c = filter st c c.next in
      clip_ring st a out xmin ymin inv;
      clip_ring st c out xmin ymin inv
    | None -> if a.next != root then find_a a.next
  in find_a root

let earcut data holes dim =
  let st = {blocks=None; filtered=false} in
  let outer_len = if Array.length holes = 0 then Array.length data else holes.(0)*dim in
  match linked_list st data 0 outer_len dim true with
  | None -> [||]
  | Some outer when outer.next == outer.prev -> [||]
  | Some outer ->
    let outer = if Array.length holes = 0 then outer else eliminate_holes st data holes dim outer in
    let xmin = ref 0. and ymin = ref 0. and inv = ref 0. in
    if Array.length data > 80*dim then begin
      xmin := data.(0); ymin := data.(1);
      let xmax = ref !xmin and ymax = ref !ymin in
      let i = ref dim in
      while !i < outer_len do
        let x = data.(!i) and y = data.(!i+1) in
        xmin := min !xmin x; ymin := min !ymin y;
        xmax := max !xmax x; ymax := max !ymax y; i := !i+dim
      done;
      let extent = max (!xmax -. !xmin) (!ymax -. !ymin) in
      if extent <> 0. then inv := 32767. /. extent
    end;
    let out = Vector.create 0 in
    clip_ring st outer out !xmin !ymin !inv;
    Vector.finish out
