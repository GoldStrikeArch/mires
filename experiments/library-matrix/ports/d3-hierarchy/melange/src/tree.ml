(* tree.js (Buchheim et al.'s linear-time Reingold-Tilford layout) and cluster.js of d3-hierarchy
   3.1.3, with the default separation. *)

open Hierarchy

(* [defaultSeparation], the same function in tree.js and cluster.js. *)
let separation a b =
  match (a.parent, b.parent) with
  | Some p, Some q when p == q -> 1.
  | None, None -> 1.
  | _ -> 2.

(* tree.js's [TreeNode]: the layout's bookkeeping for [node] ([_]), the child number [i]. Every
   node has a parent; d3's extra parent of the root has none. *)
type tree_node = {
  node : Hierarchy.node;
  i : int;
  mutable parent : tree_node option;
  mutable children : tree_node array;
  mutable default_ancestor : tree_node option; (* A *)
  mutable ancestor : tree_node; (* a *)
  mutable prelim : float; (* z *)
  mutable modifier : float; (* m *)
  mutable change : float; (* c *)
  mutable shift : float; (* s *)
  mutable thread : tree_node option; (* t *)
}

let tree_node node i =
  let rec t =
    { node; i; parent = None; children = [||]; default_ancestor = None; ancestor = t; prelim = 0.;
      modifier = 0.; change = 0.; shift = 0.; thread = None }
  in
  t

let parent_of v = Option.get v.parent

(* The left contour's successor of [v]: its first child, or its thread. *)
let next_left v = if Array.length v.children > 0 then Some v.children.(0) else v.thread

(* The right contour's successor of [v]: its last child, or its thread. *)
let next_right v =
  let n = Array.length v.children in
  if n > 0 then Some v.children.(n - 1) else v.thread

(* Shifts the subtree rooted at [wp] and prepares the shifts of the smaller subtrees in
   between. *)
let move_subtree wm wp shift =
  let change = shift /. float_of_int (wp.i - wm.i) in
  wp.change <- wp.change -. change;
  wp.shift <- wp.shift +. shift;
  wm.change <- wm.change +. change;
  wp.prelim <- wp.prelim +. shift;
  wp.modifier <- wp.modifier +. shift

(* Applies the shifts that [move_subtree] prepared to the children of [v]. *)
let execute_shifts v =
  let shift = ref 0. and change = ref 0. in
  for i = Array.length v.children - 1 downto 0 do
    let w = v.children.(i) in
    w.prelim <- w.prelim +. !shift;
    w.modifier <- w.modifier +. !shift;
    change := !change +. w.change;
    shift := !shift +. (w.shift +. !change)
  done

(* [vim]'s ancestor if it is a sibling of [v], otherwise the default [ancestor]. *)
let next_ancestor vim v ancestor = if parent_of vim.ancestor == parent_of v then vim.ancestor else ancestor

(* [treeRoot]: wraps every node in a [tree_node], under an extra parent of the root. *)
let tree_root root =
  let tree = tree_node root 0 in
  let rec wrap = function
    | [] -> ()
    | node :: rest ->
        let children = Array.mapi (fun i child -> tree_node child i) node.node.children in
        Array.iter (fun child -> child.parent <- Some node) children;
        node.children <- children;
        wrap (Array.fold_right List.cons children rest)
  in
  wrap [ tree ];
  (* d3's extra parent wraps null; it is never read, so it wraps the root here. *)
  let extra = tree_node root 0 in
  extra.children <- [| tree |];
  tree.parent <- Some extra;
  tree

(* The core of the algorithm: combines the subtree of [v] with those of its left siblings,
   walking the inside and outside contours, and returns the new default ancestor. *)
let apportion v w ancestor =
  match w with
  | None -> ancestor
  | Some w ->
      let ancestor = ref ancestor in
      let vop = ref v and vom = ref (parent_of v).children.(0) in
      let sip = ref v.modifier and sop = ref v.modifier and sim = ref w.modifier and som = ref !vom.modifier in
      let vim = ref (next_right w) and vip = ref (next_left v) in
      let rec contours () =
        match (!vim, !vip) with
        | Some im, Some ip ->
            vom := Option.get (next_left !vom);
            vop := Option.get (next_right !vop);
            !vop.ancestor <- v;
            let shift = im.prelim +. !sim -. ip.prelim -. !sip +. separation im.node ip.node in
            if shift > 0. then begin
              move_subtree (next_ancestor im v !ancestor) v shift;
              sip := !sip +. shift;
              sop := !sop +. shift
            end;
            sim := !sim +. im.modifier;
            sip := !sip +. ip.modifier;
            som := !som +. !vom.modifier;
            sop := !sop +. !vop.modifier;
            vim := next_right im;
            vip := next_left ip;
            contours ()
        | _ -> ()
      in
      contours ();
      (match !vim with
      | Some im when Option.is_none (next_right !vop) ->
          !vop.thread <- Some im;
          !vop.modifier <- !vop.modifier +. (!sim -. !sop)
      | _ -> ());
      (match !vip with
      | Some ip when Option.is_none (next_left !vom) ->
          !vom.thread <- Some ip;
          !vom.modifier <- !vom.modifier +. (!sip -. !som);
          ancestor := v
      | _ -> ());
      !ancestor

(* Computes the preliminary x of [v], after its children; then combines it with its left
   siblings. *)
let first_walk v =
  let parent = parent_of v in
  let siblings = parent.children in
  let w = if v.i <> 0 then Some siblings.(v.i - 1) else None in
  let n = Array.length v.children in
  if n > 0 then begin
    execute_shifts v;
    let midpoint = (v.children.(0).prelim +. v.children.(n - 1).prelim) /. 2. in
    match w with
    | Some w ->
        v.prelim <- w.prelim +. separation v.node w.node;
        v.modifier <- v.prelim -. midpoint
    | None -> v.prelim <- midpoint
  end
  else Option.iter (fun w -> v.prelim <- w.prelim +. separation v.node w.node) w;
  parent.default_ancestor <- Some (apportion v w (Option.value parent.default_ancestor ~default:siblings.(0)))

(* Computes the real x of [v] by summing the modifiers of its ancestors. *)
let second_walk v =
  let parent = parent_of v in
  v.node.x <- v.prelim +. parent.modifier;
  v.modifier <- v.modifier +. parent.modifier

(* [tree().size([dx, dy])], or [tree().nodeSize([dx, dy])] when [node_size] is set. *)
let tree root dx dy node_size =
  let t = tree_root root in
  let children v = v.children in

  (* Compute the layout using Buchheim et al.'s algorithm. *)
  each_after_by children first_walk t;
  (parent_of t).modifier <- -.t.prelim;
  each_before_by children second_walk t;

  if node_size then
    (* A fixed node size scales x and y. *)
    each_before
      (fun node ->
        node.x <- node.x *. dx;
        node.y <- float_of_int node.depth *. dy)
      root
  else begin
    (* A fixed tree size scales x and y by the extent of the left-most, right-most and deepest
       nodes. *)
    let left = ref root and right = ref root and bottom = ref root in
    each_before
      (fun node ->
        if node.x < !left.x then left := node;
        if node.x > !right.x then right := node;
        if node.depth > !bottom.depth then bottom := node)
      root;
    let s = if !left == !right then 1. else separation !left !right /. 2. in
    let tx = s -. !left.x in
    let kx = dx /. (!right.x +. s +. tx) in
    let ky = dy /. float_of_int (if !bottom.depth <> 0 then !bottom.depth else 1) in
    each_before
      (fun node ->
        node.x <- (node.x +. tx) *. kx;
        node.y <- float_of_int node.depth *. ky)
      root
  end

let rec leaf_left (node : Hierarchy.node) =
  if Array.length node.children > 0 then leaf_left node.children.(0) else node

let rec leaf_right (node : Hierarchy.node) =
  let n = Array.length node.children in
  if n > 0 then leaf_right node.children.(n - 1) else node

(* [cluster().size([dx, dy])], or [cluster().nodeSize([dx, dy])] when [node_size] is set. *)
let cluster root dx dy node_size =
  let previous_node = ref None and x = ref 0. in

  (* First walk, computing the initial x and y values. *)
  each_after
    (fun node ->
      let children = node.children in
      if Array.length children > 0 then begin
        node.x <- Array.fold_left (fun sum c -> sum +. c.x) 0. children /. float_of_int (Array.length children);
        (* meanX *)
        node.y <- 1. +. Array.fold_left (fun y c -> Float.max y c.y) 0. children (* maxY *)
      end
      else begin
        (match !previous_node with
        | Some previous ->
            x := !x +. separation node previous;
            node.x <- !x
        | None -> node.x <- 0.);
        node.y <- 0.;
        previous_node := Some node
      end)
    root;

  let left = leaf_left root and right = leaf_right root in
  let x0 = left.x -. (separation left right /. 2.) and x1 = right.x +. (separation right left /. 2.) in

  (* Second walk, normalizing x and y to the desired size. The root comes last, so the walk reads
     the root's unnormalized position throughout. *)
  let normalize =
    if node_size then (fun node ->
      node.x <- (node.x -. root.x) *. dx;
      node.y <- (root.y -. node.y) *. dy)
    else (fun node ->
      node.x <- (node.x -. x0) /. (x1 -. x0) *. dx;
      node.y <- (1. -. (if truthy root.y then node.y /. root.y else 1.)) *. dy)
  in
  each_after normalize root
