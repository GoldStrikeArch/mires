(* The collections the core is built on, over the OCaml standard library.

   MiniSearch keeps nearly everything in JavaScript [Map]s, whose insertion order is observable:
   setting a new key appends it, setting an existing key keeps its place, deleting removes it in
   place, and iteration follows that order. [Map.Make] orders by key, so [Ordered] pairs such a
   map (lookups) with the list of its slots (order). [Vec] is a growable array, for the tables
   MiniSearch indexes by short document id (ids are handed out 0, 1, 2, ..., so ascending short id
   is those maps' insertion order).

   [Map] rather than [Hashtbl]: under Melange, [Hashtbl] brings [Random], [Domain] and, through
   [Fun], [Printexc] and [Printf] into the bundle (a one-table program is 65.6 KB minified, a
   one-map program 13.5 KB; NOTES.md). *)

open Host

(* A growable array. *)
module Vec = struct
  type 'a t = { mutable items : 'a array; mutable length : int }

  let create () = { items = [||]; length = 0 }
  let length vec = vec.length

  let get vec index =
    if index < 0 || index >= vec.length then invalid_arg "Vec.get";
    vec.items.(index)

  let set vec index value =
    if index < 0 || index >= vec.length then invalid_arg "Vec.set";
    vec.items.(index) <- value

  let push vec value =
    if vec.length = Array.length vec.items then begin
      let items = Array.make (Int.max 8 (2 * vec.length)) value in
      Array.blit vec.items 0 items 0 vec.length;
      vec.items <- items
    end;
    vec.items.(vec.length) <- value;
    vec.length <- vec.length + 1

  let iteri f vec =
    for index = 0 to vec.length - 1 do
      f index vec.items.(index)
    done
end

(* Strings in code-unit order (as JavaScript's [<] compares them). *)
let text_compare a b =
  let length_a = Text.length a and length_b = Text.length b in
  let shorter = Int.min length_a length_b in
  let rec from index =
    if index = shorter then Int.compare length_a length_b
    else
      let difference = Text.unit_at a index - Text.unit_at b index in
      if difference <> 0 then difference else from (index + 1)
  in
  from 0

module Text_key = struct
  type t = Text.t

  let compare = text_compare
end

module Text_map = Map.Make (Text_key)

(* An insertion-ordered map: the semantics of a JavaScript [Map]. *)
module Ordered (Key : Map.OrderedType) = struct
  module Table = Map.Make (Key)

  type 'a slot = { key : Key.t; mutable value : 'a; mutable live : bool }

  (* [order] holds every slot ever added, the most recent first; a deleted slot is marked dead. *)
  type 'a t = { mutable table : 'a slot Table.t; mutable order : 'a slot list }

  let create () = { table = Table.empty; order = [] }
  let find_opt map key = match Table.find_opt key map.table with Some slot -> Some slot.value | None -> None

  (* [map.set(key, value)]. *)
  let set map key value =
    match Table.find_opt key map.table with
    | Some slot -> slot.value <- value
    | None ->
        let slot = { key; value; live = true } in
        map.table <- Table.add key slot map.table;
        map.order <- slot :: map.order

  (* [map.delete(key)]. *)
  let delete map key =
    match Table.find_opt key map.table with
    | Some slot ->
        slot.live <- false;
        map.table <- Table.remove key map.table
    | None -> ()

  (* [for (const [key, value] of map)]: the live entries in insertion order. *)
  let iter f map = List.iter (fun slot -> if slot.live then f slot.key slot.value) (List.rev map.order)

  let to_list map =
    List.fold_left (fun entries slot -> if slot.live then (slot.key, slot.value) :: entries else entries) [] map.order
end
