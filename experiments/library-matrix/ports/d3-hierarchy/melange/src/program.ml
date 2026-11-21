(* One layout per input, as libraries/d3-hierarchy/reference.mjs runs it. [Host] reads the input
   and builds the result. *)

open Hierarchy

let tile = function
  | "binary" -> Treemap.binary
  | "sliceDice" -> Treemap.slice_dice
  | "squarify" -> Treemap.squarify
  | _ -> invalid_arg "unknown tile" (* [.tile(undefined)] throws *)

(* The reference's [n.id ?? n.data.id]: the datum's id as the input has it, null or undefined
   included. *)
let id node = match node.id with Some id -> Host.string id | None -> Host.id_value (Option.get node.data)

let run input =
  let root =
    match Host.source input with
    | "id" -> stratify (Host.rows input)
    | "path" -> stratify ~path:Host.id (Host.rows input)
    | _ -> hierarchy (Host.data input)
  in
  sum (function Some d -> Host.value d | None -> 0.) root;
  if Host.sort input then sort (fun a b -> b.value -. a.value) root;
  let w = Host.width input and h = Host.height input and p = Host.padding input in
  let op = Host.op input in
  (match op with
  | "tree" -> Tree.tree root w h (Host.node_size input)
  | "cluster" -> Tree.cluster root w h (Host.node_size input)
  | "partition" -> Treemap.partition root w h p (Host.round input)
  | "pack" -> Pack.pack root w h p
  | "treemap" -> Treemap.treemap root w h (tile (Host.tile input)) p (Host.round input)
  | "hierarchy" | "iterator" | "stratify" -> ()
  | _ -> invalid_arg "unknown layout");
  let coordinates node =
    match op with
    | "tree" | "cluster" -> [ node.x; node.y ]
    | "pack" -> [ node.x; node.y; node.r ]
    | "partition" | "treemap" -> [ node.x0; node.y0; node.x1; node.y1 ]
    | _ -> []
  in
  let record node =
    let parent = match node.parent with Some parent -> id parent | None -> Host.null in
    Host.array
      (Array.of_list
         (id node :: parent
         :: Host.number (float_of_int node.depth)
         :: Host.number (float_of_int node.height)
         :: Host.number node.value
         :: List.map Host.number (coordinates node)))
  in
  let order =
    if op = "iterator" then breadth_first root
    else begin
      let visited = ref [] in
      each_before (fun node -> visited := node :: !visited) root;
      List.rev !visited
    end
  in
  Host.array (Array.of_list (List.map record order))
