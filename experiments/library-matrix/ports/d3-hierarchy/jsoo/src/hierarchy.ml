(* The node of d3-hierarchy 3.1.3 and the operations the contract uses: hierarchy/index.js
   (hierarchy, computeHeight), eachBefore.js, eachAfter.js, iterator.js, sum.js, sort.js, and
   stratify.js with the default id and parentId accessors. *)

(* [Node] of hierarchy/index.js. [data] is [None] for an ancestor that stratify imputed from a
   path, and [children] is empty for a leaf, where d3 leaves it undefined. The layouts write the
   coordinates of their kind. *)
type node = {
  data : Host.datum option;
  mutable id : string option;
  mutable parent : node option;
  mutable children : node array;
  mutable depth : int;
  mutable height : int;
  mutable value : float;
  mutable x : float;
  mutable y : float;
  mutable r : float;
  mutable x0 : float;
  mutable y0 : float;
  mutable x1 : float;
  mutable y1 : float;
}

let make data =
  { data; id = None; parent = None; children = [||]; depth = 0; height = 0; value = 0.;
    x = 0.; y = 0.; r = 0.; x0 = 0.; y0 = 0.; x1 = 0.; y1 = 0. }

(* JavaScript's ToBoolean of a number: false for 0, -0 and NaN. d3 tests numbers this way. *)
let truthy (x : float) = not (x = 0. || Float.is_nan x)

(* [node.eachBefore(callback)] over any tree whose children [children] reads: pre-order, on an
   explicit stack (a list). *)
let each_before_by children f root =
  let rec visit = function
    | [] -> ()
    | node :: rest ->
        f node;
        visit (Array.fold_right List.cons (children node) rest)
  in
  visit [ root ]

(* [node.eachAfter(callback)]: post-order, the reverse of a stack walk's visiting order. *)
let each_after_by children f root =
  let rec walk next = function
    | [] -> next
    | node :: rest -> walk (node :: next) (Array.fold_left (fun stack c -> c :: stack) rest (children node))
  in
  List.iter f (walk [] [ root ])

let each_before f root = each_before_by (fun node -> node.children) f root
let each_after f root = each_after_by (fun node -> node.children) f root

(* The order of [node[Symbol.iterator]]: breadth-first, each level from left to right. *)
let breadth_first root =
  let rec level order = function
    | [] -> List.rev order
    | current ->
        let order = List.rev_append current order in
        level order (List.concat_map (fun node -> Array.to_list node.children) current)
  in
  level [] [ root ]

(* [computeHeight]: raises the heights of the ancestors the node makes taller. *)
let compute_height node =
  let rec climb node height =
    node.height <- height;
    match node.parent with
    | Some parent when parent.height < height + 1 -> climb parent (height + 1)
    | _ -> ()
  in
  climb node 0

(* [hierarchy(data)] with the default children accessor, [d.children]. *)
let hierarchy data =
  let root = make (Some data) in
  let rec build = function
    | [] -> ()
    | node :: rest ->
        let childs = match node.data with Some d -> Host.children d | None -> [||] in
        if Array.length childs = 0 then build rest
        else begin
          let children = Array.map (fun d -> make (Some d)) childs in
          node.children <- children;
          Array.iter (fun child -> child.parent <- Some node; child.depth <- node.depth + 1) children;
          build (Array.fold_right List.cons children rest)
        end
  in
  build [ root ];
  each_before compute_height root;
  root

(* [node.sum(value)]: each aggregate is [+value(data) || 0] plus the children's, last child
   first. *)
let sum value root =
  each_after
    (fun node ->
      let own = value node.data in
      let sum = ref (if truthy own then own else 0.) in
      for i = Array.length node.children - 1 downto 0 do
        sum := !sum +. node.children.(i).value
      done;
      node.value <- !sum)
    root

(* [node.sort(compare)] with a JavaScript comparator. Array.prototype.sort is stable and counts a
   NaN result as 0; Array.stable_sort is stable too. *)
let sort compare root =
  each_before
    (fun node ->
      Array.stable_sort
        (fun a b ->
          let order = compare a b in
          if order < 0. then -1 else if order > 0. then 1 else 0)
        node.children)
    root

(* Whether [path.[i]] is a delimiting slash: one preceded by an even number of backslashes. *)
let slash path i =
  i >= 0 && i < String.length path && path.[i] = '/'
  &&
  let rec backslashes j k = if j > 0 && path.[j - 1] = '\\' then backslashes (j - 1) (k + 1) else k in
  backslashes i 0 land 1 = 0

(* Coerces nothing (ids are strings here), strips a trailing slash unless another slash precedes
   it, and adds a leading slash if missing. *)
let normalize path =
  let i = String.length path in
  let path = if slash path (i - 1) && not (slash path (i - 2)) then String.sub path 0 (i - 1) else path in
  if String.starts_with ~prefix:"/" path then path else "/" ^ path

(* The path up to its last delimiting slash after the first character: "/a/b" to "/a", "/" to
   "". *)
let parentof path =
  let rec back i = if i > 1 && not (slash path i) then back (i - 1) else i in
  let n = String.length path in
  if n < 2 then "" else String.sub path 0 (back (n - 1))

(* A key of stratify's [nodeByKey]: the index of the one node with that id, or d3's
   [ambiguous]. *)
type key = Unique of int | Ambiguous

(* [stratify()(data)], or [stratify().path(path)(data)] when [path] is given. d3 tests ids as
   [id != null && (id += "")]: an undefined, null or empty id names nothing, and Host reads the
   three as "". An imputed ancestor is a node whose data is [None]; d3 marks one with its
   [imputed] object and replaces that by null before it returns. *)
let stratify ?path data =
  let ids, parent_ids, rows =
    match path with
    | None -> (Array.map Host.id data, Array.map Host.parent_id data, Array.map Option.some data)
    | Some path ->
        let i = Array.map (fun d -> normalize (path d)) data in
        let p = Array.map parentof i in
        (* d3 appends each missing parent to I and P while it iterates P: a queue keeps that
           order. *)
        let seen = Hashtbl.create (Array.length i + 1) in
        Hashtbl.replace seen "" ();
        Array.iter (fun id -> Hashtbl.replace seen id ()) i;
        let pending = Queue.of_seq (Array.to_seq p) and imputed = ref [] in
        while not (Queue.is_empty pending) do
          let id = Queue.pop pending in
          if not (Hashtbl.mem seen id) then begin
            Hashtbl.replace seen id ();
            imputed := id :: !imputed;
            Queue.push (parentof id) pending
          end
        done;
        let imputed = Array.of_list (List.rev !imputed) in
        ( Array.append i imputed,
          Array.append p (Array.map parentof imputed),
          Array.append (Array.map Option.some data) (Array.map (fun _ -> None) imputed) )
  in
  let n = Array.length rows in
  let nodes = Array.map make rows in
  let node_by_key = Hashtbl.create n in
  Array.iteri
    (fun i node ->
      let node_id = ids.(i) in
      if node_id <> "" then begin
        node.id <- Some node_id;
        Hashtbl.replace node_by_key node_id (if Hashtbl.mem node_by_key node_id then Ambiguous else Unique i)
      end)
    nodes;

  (* d3 pushes each child onto its parent's children; the lists collect them in reverse. *)
  let children = Array.make n [] and root = ref None in
  Array.iteri
    (fun i node ->
      let key = parent_ids.(i) in
      if key <> "" then begin
        match Hashtbl.find_opt node_by_key key with
        | None -> failwith ("missing: " ^ key)
        | Some Ambiguous -> failwith ("ambiguous: " ^ key)
        | Some (Unique p) ->
            children.(p) <- node :: children.(p);
            node.parent <- Some nodes.(p)
      end
      else begin
        if Option.is_some !root then failwith "multiple roots";
        root := Some node
      end)
    nodes;
  Array.iteri (fun i node -> node.children <- Array.of_list (List.rev children.(i))) nodes;
  let root = match !root with Some root -> root | None -> failwith "no root" in

  (* When imputing internal nodes, only introduce roots if needed. *)
  let rec trim root n =
    if Option.is_none root.data && Array.length root.children = 1 then trim root.children.(0) (n - 1)
    else (root, n)
  in
  let root, n = if Option.is_some path then trim root n else (root, n) in

  (* d3 parents the root to [preroot], whose depth is -1, while it walks. *)
  root.parent <- None;
  let n = ref n in
  each_before
    (fun node ->
      node.depth <- (match node.parent with Some parent -> parent.depth + 1 | None -> 0);
      decr n)
    root;
  each_before compute_height root;
  if !n > 0 then failwith "cycle";
  root
