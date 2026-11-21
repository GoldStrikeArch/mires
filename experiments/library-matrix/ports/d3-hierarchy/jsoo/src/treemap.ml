(* treemap/ (index.js, dice.js, slice.js, sliceDice.js, squarify.js, binary.js, round.js) and
   partition.js of d3-hierarchy 3.1.3. *)

open Hierarchy

(* round.js. Math.round, from the host, rounds halves up, keeps -0 and returns -0 for
   [-0.5, 0). *)
let round_node node =
  node.x0 <- Host.math_round node.x0;
  node.y0 <- Host.math_round node.y0;
  node.x1 <- Host.math_round node.x1;
  node.y1 <- Host.math_round node.y1

(* dice.js on [nodes.(first)] to [nodes.(last - 1)], the children of a parent or of a squarified
   row, whose value is [value]: columns from left to right. *)
let dice nodes first last value x0 y0 x1 y1 =
  let k = if truthy value then (x1 -. x0) /. value else value in
  let x = ref x0 in
  for i = first to last - 1 do
    let node = nodes.(i) in
    node.y0 <- y0;
    node.y1 <- y1;
    node.x0 <- !x;
    x := !x +. (node.value *. k);
    node.x1 <- !x
  done

(* slice.js, as [dice]: rows from top to bottom. *)
let slice nodes first last value x0 y0 x1 y1 =
  let k = if truthy value then (y1 -. y0) /. value else value in
  let y = ref y0 in
  for i = first to last - 1 do
    let node = nodes.(i) in
    node.x0 <- x0;
    node.x1 <- x1;
    node.y0 <- !y;
    y := !y +. (node.value *. k);
    node.y1 <- !y
  done

(* [treemapDice] and [treemapSlice] of a parent. *)
let treemap_dice parent = dice parent.children 0 (Array.length parent.children) parent.value
let treemap_slice parent = slice parent.children 0 (Array.length parent.children) parent.value

(* sliceDice.js: slice at odd depths, dice at even ones. *)
let slice_dice parent = if parent.depth land 1 <> 0 then treemap_slice parent else treemap_dice parent

let phi = (1. +. sqrt 5.) /. 2.

(* [squarifyRatio]. d3 also collects the rows, for treemapResquarify; this port only lays them
   out. *)
let squarify_ratio ratio parent x0 y0 x1 y1 =
  let nodes = parent.children in
  let n = Array.length nodes in
  let x0 = ref x0 and y0 = ref y0 and i0 = ref 0 and i1 = ref 0 and value = ref parent.value in
  while !i0 < n do
    let dx = x1 -. !x0 and dy = y1 -. !y0 in

    (* Find the next non-empty node. *)
    let sum_value = ref nodes.(!i1).value in
    incr i1;
    while (not (truthy !sum_value)) && !i1 < n do
      sum_value := nodes.(!i1).value;
      incr i1
    done;
    let min_value = ref !sum_value and max_value = ref !sum_value in
    let alpha = Float.max (dy /. dx) (dx /. dy) /. (!value *. ratio) in
    let beta = !sum_value *. !sum_value *. alpha in
    let min_ratio = ref (Float.max (!max_value /. beta) (beta /. !min_value)) in

    (* Keep adding nodes while the aspect ratio maintains or improves. *)
    let growing = ref true in
    while !growing && !i1 < n do
      let node_value = nodes.(!i1).value in
      sum_value := !sum_value +. node_value;
      if node_value < !min_value then min_value := node_value;
      if node_value > !max_value then max_value := node_value;
      let beta = !sum_value *. !sum_value *. alpha in
      let new_ratio = Float.max (!max_value /. beta) (beta /. !min_value) in
      if new_ratio > !min_ratio then begin
        sum_value := !sum_value -. node_value;
        growing := false
      end
      else begin
        min_ratio := new_ratio;
        incr i1
      end
    done;

    (* Position the row in its orientation. *)
    if dx < dy then begin
      let bottom = if truthy !value then !y0 +. (dy *. !sum_value /. !value) else y1 in
      dice nodes !i0 !i1 !sum_value !x0 !y0 x1 bottom;
      if truthy !value then y0 := bottom
    end
    else begin
      let right = if truthy !value then !x0 +. (dx *. !sum_value /. !value) else x1 in
      slice nodes !i0 !i1 !sum_value !x0 !y0 right y1;
      if truthy !value then x0 := right
    end;
    value := !value -. !sum_value;
    i0 := !i1
  done

(* squarify.js with the default ratio, the golden ratio. *)
let squarify parent = squarify_ratio phi parent

(* binary.js: splits the children in two of about equal value, recursively. *)
let binary parent x0 y0 x1 y1 =
  let nodes = parent.children in
  let n = Array.length nodes in
  let sums = Array.make (n + 1) 0. in
  for i = 0 to n - 1 do
    sums.(i + 1) <- sums.(i) +. nodes.(i).value
  done;

  let rec partition i j value x0 y0 x1 y1 =
    if i >= j - 1 then begin
      let node = nodes.(i) in
      node.x0 <- x0;
      node.y0 <- y0;
      node.x1 <- x1;
      node.y1 <- y1
    end
    else begin
      let value_offset = sums.(i) in
      let value_target = (value /. 2.) +. value_offset in
      let rec search k hi =
        if k < hi then
          let mid = (k + hi) lsr 1 in
          if sums.(mid) < value_target then search (mid + 1) hi else search k mid
        else k
      in
      let k = search (i + 1) (j - 1) in
      let k = if value_target -. sums.(k - 1) < sums.(k) -. value_target && i + 1 < k then k - 1 else k in

      let value_left = sums.(k) -. value_offset in
      let value_right = value -. value_left in
      if x1 -. x0 > y1 -. y0 then begin
        let xk = if truthy value then ((x0 *. value_right) +. (x1 *. value_left)) /. value else x1 in
        partition i k value_left x0 y0 xk y1;
        partition k j value_right xk y0 x1 y1
      end
      else begin
        let yk = if truthy value then ((y0 *. value_right) +. (y1 *. value_left)) /. value else y1 in
        partition i k value_left x0 y0 x1 yk;
        partition k j value_right x0 yk x1 y1
      end
    end
  in
  partition 0 n parent.value x0 y0 x1 y1

(* The treemap's and partition's [if (x1 < x0) x0 = x1 = (x0 + x1) / 2], and the same for y. *)
let collapse x0 y0 x1 y1 =
  let x0, x1 = if x1 < x0 then let x = (x0 +. x1) /. 2. in (x, x) else (x0, x1) in
  let y0, y1 = if y1 < y0 then let y = (y0 +. y1) /. 2. in (y, y) else (y0, y1) in
  (x0, y0, x1, y1)

(* [treemap().size([dx, dy]).tile(tile).padding(padding).round(round)]: the inner and every
   outer padding are [padding]. *)
let treemap root dx dy tile padding round =
  (* d3 keeps the stack between calls and resets it to [0]; depth d reads index d. *)
  let padding_stack = Array.make (root.height + 1) 0. in
  root.x0 <- 0.;
  root.y0 <- 0.;
  root.x1 <- dx;
  root.y1 <- dy;
  each_before
    (fun node ->
      let p = padding_stack.(node.depth) in
      let x0, y0, x1, y1 = collapse (node.x0 +. p) (node.y0 +. p) (node.x1 -. p) (node.y1 -. p) in
      node.x0 <- x0;
      node.y0 <- y0;
      node.x1 <- x1;
      node.y1 <- y1;
      if Array.length node.children > 0 then begin
        let p = padding /. 2. in
        padding_stack.(node.depth + 1) <- p;
        let x0, y0, x1, y1 =
          collapse (x0 +. (padding -. p)) (y0 +. (padding -. p)) (x1 -. (padding -. p)) (y1 -. (padding -. p))
        in
        tile node x0 y0 x1 y1
      end)
    root;
  if round then each_before round_node root

(* [partition().size([dx, dy]).padding(padding).round(round)]. *)
let partition root dx dy padding round =
  let n = float_of_int (root.height + 1) in
  root.x0 <- padding;
  root.y0 <- padding;
  root.x1 <- dx;
  root.y1 <- dy /. n;
  each_before
    (fun node ->
      if Array.length node.children > 0 then
        treemap_dice node node.x0
          (dy *. float_of_int (node.depth + 1) /. n)
          node.x1
          (dy *. float_of_int (node.depth + 2) /. n);
      let x0, y0, x1, y1 = collapse node.x0 node.y0 (node.x1 -. padding) (node.y1 -. padding) in
      node.x0 <- x0;
      node.y0 <- y0;
      node.x1 <- x1;
      node.y1 <- y1)
    root;
  if round then each_before round_node root
