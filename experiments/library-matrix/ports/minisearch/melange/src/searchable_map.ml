(* MiniSearch's SearchableMap: a radix tree over JavaScript strings (UTF-16 code units).

   A node is what MiniSearch's node is, an insertion-ordered map: edge labels to child nodes, and
   the empty key (LEAF) to the value of the key that ends at the node. The order of a node's
   entries is observable (prefix and fuzzy results, toJSON), so every operation keeps exactly the
   order a JavaScript [Map] would: [set] of a new key appends, [delete] removes in place, and
   replacing an edge (a split in [create_path], a merge after a removal) is [set] of the new label
   followed by [delete] of the old one, which moves it to the end. Here a node is the list of its
   entries in insertion order. Labels are compared and split on code units, so an edge may end
   between the two halves of a surrogate pair. *)

open Host

type 'v node = { mutable entries : 'v entry list }
and 'v entry = Leaf of 'v | Edge of Text.t * 'v node

type 'v t = { root : 'v node }

let create () = { root = { entries = [] } }
let empty_text = Text.of_string ""

(* [key.startsWith(prefix, at)]. *)
let starts_with_at key at prefix =
  let length = Text.length prefix in
  if Text.length key - at < length then false
  else begin
    let index = ref 0 in
    while !index < length && Text.unit_at key (at + !index) = Text.unit_at prefix !index do
      incr index
    done;
    !index = length
  end

(* [node.get(LEAF)]. *)
let rec leaf_of = function [] -> None | Leaf value :: _ -> Some value | Edge _ :: older -> leaf_of older

let leaf node = leaf_of node.entries

(* The first edge (in insertion order) whose label is a prefix of the key's rest: the loop of
   MiniSearch's [lookup] and [trackDown], [k !== LEAF && key.startsWith(k)]. *)
let rec edge_prefix_of entries key at =
  match entries with
  | [] -> None
  | (Edge (label, _) as edge) :: _ when starts_with_at key at label -> Some edge
  | _ :: later -> edge_prefix_of later key at

(* The edge whose label starts with [unit] ([key[pos] === k[0]] in [createPath]); no two edges of
   a node share their first unit. *)
let rec edge_starting_with entries unit =
  match entries with
  | [] -> None
  | (Edge (label, _) as edge) :: _ when Text.unit_at label 0 = unit -> Some edge
  | _ :: later -> edge_starting_with later unit

(* [node.delete(label)] of an edge. *)
let without_edge entries label =
  List.filter (function Edge (key, _) -> not (Text.equal key label) | Leaf _ -> true) entries

(* [node.set(label, child)] of a label the node does not hold: appended. *)
let add_edge node label child = node.entries <- node.entries @ [ Edge (label, child) ]

let rec lookup node key at =
  if at >= Text.length key then Some node
  else
    match edge_prefix_of node.entries key at with
    | Some (Edge (label, child)) -> lookup child key (at + Text.length label)
    | Some (Leaf _) | None -> None

let get map key = match lookup map.root key 0 with Some node -> leaf node | None -> None
let has map key = Option.is_some (get map key)

(* [createPath]: the node [key] ends at, made (splitting an edge where the key leaves it) when it
   does not exist. *)
let rec create_path node key pos =
  let key_length = Text.length key in
  if pos >= key_length then node
  else
    match edge_starting_with node.entries (Text.unit_at key pos) with
    | Some (Leaf _) | None ->
        let child = { entries = [] } in
        add_edge node (Text.slice key pos key_length) child;
        child
    | Some (Edge (label, child)) ->
        let label_length = Text.length label in
        let limit = Int.min (key_length - pos) label_length in
        let offset = ref 1 in
        while !offset < limit && Text.unit_at key (pos + !offset) = Text.unit_at label !offset do
          incr offset
        done;
        let offset = !offset in
        if offset = label_length then create_path child key (pos + offset)
        else begin
          (* A partial match: an intermediate node takes the rest of the old edge. It is set under
             the shared part and the old edge deleted, so it moves to the end of the node. *)
          let intermediate = { entries = [ Edge (Text.slice label offset label_length, child) ] } in
          node.entries <- without_edge node.entries label;
          add_edge node (Text.slice key pos (pos + offset)) intermediate;
          create_path intermediate key (pos + offset)
        end

(* [fetch(key, initial)]: the value at [key], set to [initial ()] first when there is none. *)
let fetch map key initial =
  let node = create_path map.root key 0 in
  match leaf node with
  | Some value -> value
  | None ->
      let value = initial () in
      node.entries <- node.entries @ [ Leaf value ];
      value

(* [trackDown]: the node [key] ends at, if any, and the path to it as (node, label) pairs, the
   innermost first. *)
let rec track_down node key at path =
  if at >= Text.length key then (Some node, path)
  else
    match edge_prefix_of node.entries key at with
    | Some (Edge (label, child)) -> track_down child key (at + Text.length label) ((node, label) :: path)
    | Some (Leaf _) | None -> (None, path)

(* [merge]: the node reached by the path's last label holds the single edge [label] to [child];
   its parent replaces both labels by their concatenation, which moves the edge to the end. *)
let merge path label child =
  match path with
  | [] -> ()
  | (node, node_label) :: _ ->
      node.entries <- without_edge node.entries node_label;
      add_edge node (Text.append node_label label) child

(* [cleanup]: the node reached by the path's last label is empty; its parent deletes the edge to
   it, and is cleaned up or merged in turn. *)
let rec cleanup path =
  match path with
  | [] -> ()
  | (node, label) :: parent_path -> (
      node.entries <- without_edge node.entries label;
      match node.entries with
      | [] -> cleanup parent_path
      | [ Edge (only, child) ] -> merge parent_path only child
      | _ -> ())

(* [delete(key)] ([remove]): delete the leaf, then clean up or merge on the way back up. Neither
   happens at the root, whose path is empty. *)
let delete map key =
  match track_down map.root key 0 [] with
  | None, _ -> ()
  | Some node, path -> (
      node.entries <- List.filter (function Leaf _ -> false | Edge _ -> true) node.entries;
      match node.entries with
      | [] -> cleanup path
      | [ Edge (label, child) ] -> merge path label child
      | _ -> ())

(* The order of a [TreeIterator]: depth first, each node's entries from the most recently
   inserted to the oldest (the iterator pops a snapshot of the keys from its end); a leaf yields
   the key spelled by the labels on the path. [collect entries prefix later] is the keys and
   values below [entries] (oldest first, so visited last to first) in that order, followed by
   [later]. *)
let rec collect entries prefix later =
  match entries with
  | [] -> later
  | Leaf value :: older -> collect older prefix ((prefix, value) :: later)
  | Edge (label, child) :: older -> collect older prefix (collect child.entries (Text.append prefix label) later)

(* [entries()] of the whole map, collected. MiniSearch deletes terms while its iterator walks the
   tree (vacuuming); the iterator holds a snapshot of each node's keys and only the term just
   visited is ever deleted, so it visits exactly the terms of the tree as it was when the walk
   began, which is this list. *)
let entries map = collect map.root.entries empty_text []

(* [atPrefix(prefix).entries()], collected (as [entries]): when the prefix ends at a node, that
   node's subtree; when it ends inside an edge whose label starts with the rest, a one-entry view
   holding the rest of that label; otherwise nothing. *)
let prefix_entries map prefix =
  let prefix_length = Text.length prefix in
  let rec track node at =
    if at >= prefix_length then Some node
    else
      match edge_prefix_of node.entries prefix at with
      | Some (Edge (label, child)) -> track child (at + Text.length label)
      | Some (Leaf _) | None ->
          let rest = Text.slice prefix at prefix_length in
          let rest_length = prefix_length - at in
          List.find_map
            (function
              | Edge (label, child) when starts_with_at label 0 rest ->
                  Some { entries = [ Edge (Text.slice label rest_length (Text.length label), child) ] }
              | _ -> None)
            node.entries
  in
  match track map.root 0 with None -> [] | Some node -> collect node.entries prefix []

(* [size]: the number of keys. *)
let size map =
  let rec count total = function
    | [] -> total
    | Leaf _ :: older -> count (total + 1) older
    | Edge (_, child) :: older -> count (count total child.entries) older
  in
  count 0 map.root.entries

(* [fuzzyGet(query, maxDistance)]: every key within [max_distance] (Levenshtein, over code units)
   of [query], with its value and distance, in the order the walk meets them: depth first, each
   node's entries oldest first.

   The matrix has one column per query prefix and [query.length + 1 + maxDistance] rows; it is a
   Uint8Array in MiniSearch, so every store is reduced modulo 256 (inside the parity domain no
   value reaches 256). The rows are shared by every branch of the walk, as in MiniSearch. *)
let fuzzy_get map query max_distance =
  let n = Text.length query + 1 in
  let m = n + max_distance in
  let matrix = Array.make (m * n) ((max_distance + 1) land 255) in
  for j = 0 to n - 1 do
    matrix.(j) <- j land 255
  done;
  for i = 1 to m - 1 do
    matrix.(i * n) <- i land 255
  done;
  let query_units = Array.init (n - 1) (Text.unit_at query) in
  let results = ref [] in
  (* [recurse(node, ...)]: [row] is the next row to fill, one past the length of [prefix]. *)
  let rec recurse entries row prefix =
    match entries with
    | [] -> ()
    | entry :: later ->
        (match entry with
        | Leaf value ->
            let distance = matrix.((row * n) - 1) in
            if distance <= max_distance then results := (prefix, value, distance) :: !results
        | Edge (label, child) ->
            let label_length = Text.length label in
            let i = ref row in
            let pos = ref 0 in
            let pruned = ref false in
            while (not !pruned) && !pos < label_length do
              if !i >= m then
                (* Past the last row MiniSearch reads undefined, which never prunes and never
                   matches: no key below can match, so the walk may stop here. *)
                pruned := true
              else begin
                let char = Text.unit_at label !pos in
                let this_row = n * !i in
                let previous_row = this_row - n in
                let min_distance = ref matrix.(this_row) in
                let jmin = Int.max 0 (!i - max_distance - 1) in
                let jmax = Int.min (n - 1) (!i + max_distance) in
                for j = jmin to jmax - 1 do
                  let different = if char <> query_units.(j) then 1 else 0 in
                  let rpl = matrix.(previous_row + j) + different in
                  let del = matrix.(previous_row + j + 1) + 1 in
                  let ins = matrix.(this_row + j) + 1 in
                  let dist = Int.min rpl (Int.min del ins) in
                  matrix.(this_row + j + 1) <- dist land 255;
                  if dist < !min_distance then min_distance := dist
                done;
                if !min_distance > max_distance then pruned := true
                else begin
                  incr pos;
                  incr i
                end
              end
            done;
            if not !pruned then recurse child.entries !i (Text.append prefix label));
        recurse later row prefix
  in
  recurse map.root.entries 1 empty_text;
  List.rev !results
