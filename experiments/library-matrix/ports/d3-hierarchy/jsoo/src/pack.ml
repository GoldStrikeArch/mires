(* pack/ (index.js, siblings.js, enclose.js), lcg.js and array.js's shuffle of d3-hierarchy
   3.1.3. *)

open Hierarchy

(* lcg.js: a seeded generator of [0, 1). Every product stays below 2^53, so floats are exact.
   https://en.wikipedia.org/wiki/Linear_congruential_generator#Parameters_in_common_use *)
let lcg () =
  let a = 1664525. and c = 1013904223. and m = 4294967296. (* 2^32 *) in
  let s = ref 1. in
  fun () ->
    s := Float.rem ((a *. !s) +. c) m;
    !s /. m

(* A circle as enclose.js reads one. The front chain's nodes are copied in: they do not move
   while their enclosure is computed. *)
type circle = { cx : float; cy : float; cr : float }

let circle_of node = { cx = node.x; cy = node.y; cr = node.r }

(* array.js's [shuffle]: Fisher-Yates from the end, with [random() * m-- | 0]. *)
let shuffle array random =
  for m = Array.length array downto 1 do
    let i = int_of_float (random () *. float_of_int m) in
    let t = array.(m - 1) in
    array.(m - 1) <- array.(i);
    array.(i) <- t
  done

let encloses_not a b =
  let dr = a.cr -. b.cr and dx = b.cx -. a.cx and dy = b.cy -. a.cy in
  dr < 0. || dr *. dr < (dx *. dx) +. (dy *. dy)

let encloses_weak a b =
  let dr = a.cr -. b.cr +. (Float.max (Float.max a.cr b.cr) 1. *. 1e-9) and dx = b.cx -. a.cx and dy = b.cy -. a.cy in
  dr > 0. && dr *. dr > (dx *. dx) +. (dy *. dy)

let encloses_weak_all a basis = List.for_all (encloses_weak a) basis

let enclose_basis2 a b =
  let x1 = a.cx and y1 = a.cy and r1 = a.cr in
  let x2 = b.cx and y2 = b.cy and r2 = b.cr in
  let x21 = x2 -. x1 and y21 = y2 -. y1 and r21 = r2 -. r1 in
  let l = sqrt ((x21 *. x21) +. (y21 *. y21)) in
  { cx = (x1 +. x2 +. (x21 /. l *. r21)) /. 2.; cy = (y1 +. y2 +. (y21 /. l *. r21)) /. 2.; cr = (l +. r1 +. r2) /. 2. }

let enclose_basis3 a b c =
  let x1 = a.cx and y1 = a.cy and r1 = a.cr in
  let x2 = b.cx and y2 = b.cy and r2 = b.cr in
  let x3 = c.cx and y3 = c.cy and r3 = c.cr in
  let a2 = x1 -. x2 and a3 = x1 -. x3 and b2 = y1 -. y2 and b3 = y1 -. y3 and c2 = r2 -. r1 and c3 = r3 -. r1 in
  let d1 = (x1 *. x1) +. (y1 *. y1) -. (r1 *. r1) in
  let d2 = d1 -. (x2 *. x2) -. (y2 *. y2) +. (r2 *. r2) in
  let d3 = d1 -. (x3 *. x3) -. (y3 *. y3) +. (r3 *. r3) in
  let ab = (a3 *. b2) -. (a2 *. b3) in
  let xa = (((b2 *. d3) -. (b3 *. d2)) /. (ab *. 2.)) -. x1 in
  let xb = ((b3 *. c2) -. (b2 *. c3)) /. ab in
  let ya = (((a3 *. d2) -. (a2 *. d3)) /. (ab *. 2.)) -. y1 in
  let yb = ((a2 *. c3) -. (a3 *. c2)) /. ab in
  let qa = (xb *. xb) +. (yb *. yb) -. 1. in
  let qb = 2. *. (r1 +. (xa *. xb) +. (ya *. yb)) in
  let qc = (xa *. xa) +. (ya *. ya) -. (r1 *. r1) in
  let r =
    -.(if Float.abs qa > 1e-6 then (qb +. sqrt ((qb *. qb) -. (4. *. qa *. qc))) /. (2. *. qa) else qc /. qb)
  in
  { cx = x1 +. xa +. (xb *. r); cy = y1 +. ya +. (yb *. r); cr = r }

let enclose_basis = function
  | [ a ] -> a
  | [ a; b ] -> enclose_basis2 a b
  | [ a; b; c ] -> enclose_basis3 a b c
  | _ -> invalid_arg "enclose_basis"

let extend_basis b p =
  if encloses_weak_all p b then [ p ]
  else
    (* If we get here then B must have at least one element. *)
    match List.find_opt (fun x -> encloses_not p x && encloses_weak_all (enclose_basis2 x p) b) b with
    | Some x -> [ x; p ]
    | None -> (
        (* If we get here then B must have at least two elements. *)
        let triple x y =
          encloses_not (enclose_basis2 x y) p
          && encloses_not (enclose_basis2 x p) y
          && encloses_not (enclose_basis2 y p) x
          && encloses_weak_all (enclose_basis3 x y p) b
        in
        let rec first_pair = function
          | [] -> None
          | x :: rest -> (
              match List.find_opt (triple x) rest with Some y -> Some (x, y) | None -> first_pair rest)
        in
        match first_pair b with
        | Some (x, y) -> [ x; y; p ]
        (* If we get here then something is very wrong. *)
        | None -> failwith "no enclosing basis")

(* [packEncloseRandom]: the smallest circle enclosing [circles], by Welzl's algorithm. *)
let pack_enclose_random circles random =
  shuffle circles random;
  let n = Array.length circles in
  let rec scan i basis e =
    if i >= n then e
    else
      let p = circles.(i) in
      match e with
      | Some enclosure when encloses_weak enclosure p -> scan (i + 1) basis e
      | _ ->
          let basis = extend_basis basis p in
          scan 0 basis (Some (enclose_basis basis))
  in
  Option.get (scan 0 [] None)

(* Places [c] tangent to [a] and [b]. *)
let place b a c =
  let dx = b.x -. a.x and dy = b.y -. a.y in
  let d2 = (dx *. dx) +. (dy *. dy) in
  if truthy d2 then begin
    let a2 = a.r +. c.r and b2 = b.r +. c.r in
    let a2 = a2 *. a2 and b2 = b2 *. b2 in
    if a2 > b2 then begin
      let x = (d2 +. b2 -. a2) /. (2. *. d2) in
      let y = sqrt (Float.max 0. ((b2 /. d2) -. (x *. x))) in
      c.x <- b.x -. (x *. dx) -. (y *. dy);
      c.y <- b.y -. (x *. dy) +. (y *. dx)
    end
    else begin
      let x = (d2 +. a2 -. b2) /. (2. *. d2) in
      let y = sqrt (Float.max 0. ((a2 /. d2) -. (x *. x))) in
      c.x <- a.x +. (x *. dx) -. (y *. dy);
      c.y <- a.y +. (x *. dy) +. (y *. dx)
    end
  end
  else begin
    c.x <- a.x +. c.r;
    c.y <- a.y
  end

let intersects a b =
  let dr = a.r +. b.r -. 1e-6 and dx = b.x -. a.x and dy = b.y -. a.y in
  dr > 0. && dr *. dr > (dx *. dx) +. (dy *. dy)

(* siblings.js's front-chain [Node]: a circle ([_]) and its neighbours on the chain. A link is its
   own neighbour until it joins the chain. *)
type link = { circle : Hierarchy.node; mutable next : link; mutable previous : link }

let link circle =
  let rec l = { circle; next = l; previous = l } in
  l

(* The squared distance from the origin to the weighted centre of [node] and its successor. *)
let score node =
  let a = node.circle and b = node.next.circle in
  let ab = a.r +. b.r in
  let dx = ((a.x *. b.r) +. (b.x *. a.r)) /. ab and dy = ((a.y *. b.r) +. (b.y *. a.r)) /. ab in
  (dx *. dx) +. (dy *. dy)

(* The circle on the front chain between [a] and [b] that circle [c] would intersect first, or
   [None]: [`Behind k] replaces [a], [`Ahead j] replaces [b]. "Closeness" is determined by linear
   distance along the front-chain, and so are "ahead" and "behind". *)
let find_intersection a b c =
  let rec scan j k sj sk =
    if sj <= sk then
      if intersects j.circle c then Some (`Ahead j)
      else
        let j' = j.next in
        if j' == k.next then None else scan j' k (sj +. j.circle.r) sk
    else if intersects k.circle c then Some (`Behind k)
    else
      let k' = k.previous in
      if j == k'.next then None else scan j k' sj (sk +. k.circle.r)
  in
  scan b.next a.previous b.circle.r a.circle.r

(* [packSiblingsRandom]: packs [circles] around the origin and returns the enclosing radius. *)
let pack_siblings_random circles random =
  let n = Array.length circles in
  if n = 0 then 0.
  else begin
    (* Place the first circle. *)
    let first = circles.(0) in
    first.x <- 0.;
    first.y <- 0.;
    if n <= 1 then first.r
    else begin
      (* Place the second circle. *)
      let second = circles.(1) in
      first.x <- -.second.r;
      second.x <- first.r;
      second.y <- 0.;
      if n <= 2 then first.r +. second.r
      else begin
        (* Place the third circle. *)
        place second first circles.(2);

        (* Initialize the front-chain using the first three circles a, b and c. *)
        let a = link first and b = link second and c = link circles.(2) in
        a.next <- b;
        c.previous <- b;
        b.next <- c;
        a.previous <- c;
        c.next <- a;
        b.previous <- a;

        (* Attempt to place each remaining circle… *)
        let rec place_from i a b =
          if i >= n then b
          else begin
            place a.circle b.circle circles.(i);
            match find_intersection a b circles.(i) with
            | Some (`Ahead j) ->
                j.previous <- a;
                a.next <- j;
                place_from i a j
            | Some (`Behind k) ->
                k.next <- b;
                b.previous <- k;
                place_from i k b
            | None ->
                (* Success! Insert the new circle c between a and b. *)
                let c = link circles.(i) in
                c.previous <- a;
                c.next <- b;
                a.next <- c;
                b.previous <- c;

                (* Compute the new closest circle pair to the centroid. *)
                let rec closest node best score_best =
                  if node == c then best
                  else
                    let s = score node in
                    if s < score_best then closest node.next node s else closest node.next best score_best
                in
                let a = closest b a (score a) in
                place_from (i + 1) a a.next
          end
        in
        let b = place_from 3 a b in

        (* Compute the enclosing circle of the front chain. *)
        let rec front node acc = if node == b then List.rev acc else front node.next (circle_of node.circle :: acc) in
        let e = pack_enclose_random (Array.of_list (front b.next [ circle_of b.circle ])) random in

        (* Translate the circles to put the enclosing circle around the origin. *)
        Array.iter
          (fun node ->
            node.x <- node.x -. e.cx;
            node.y <- node.y -. e.cy)
          circles;
        e.cr
      end
    end
  end

(* [radiusLeaf(defaultRadius)]: a leaf's radius is the square root of its value. *)
let radius_leaf node =
  if Array.length node.children = 0 then begin
    let r = sqrt node.value in
    node.r <- Float.max 0. (if truthy r then r else 0.)
  end

(* [packChildrenRandom(padding, k, random)] with a constant padding. *)
let pack_children_random padding k random node =
  if Array.length node.children > 0 then begin
    let r = padding *. k in
    let r = if truthy r then r else 0. in
    if truthy r then Array.iter (fun child -> child.r <- child.r +. r) node.children;
    let e = pack_siblings_random node.children random in
    if truthy r then Array.iter (fun child -> child.r <- child.r -. r) node.children;
    node.r <- e +. r
  end

(* [translateChild(k)]. *)
let translate_child k node =
  node.r <- node.r *. k;
  match node.parent with
  | Some parent ->
      node.x <- parent.x +. (k *. node.x);
      node.y <- parent.y +. (k *. node.y)
  | None -> ()

(* [pack().size([dx, dy]).padding(padding)]: one generator serves both packing passes. *)
let pack root dx dy padding =
  let random = lcg () in
  root.x <- dx /. 2.;
  root.y <- dy /. 2.;
  each_before radius_leaf root;
  each_after (pack_children_random 0. 1. random) root;
  let k = root.r /. Float.min dx dy in
  each_after (pack_children_random padding k random) root;
  each_before (translate_child (Float.min dx dy /. (2. *. root.r))) root
