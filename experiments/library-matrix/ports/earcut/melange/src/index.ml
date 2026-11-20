(* Melange emits native JS arrays, numbers and records at the boundary. *)
type value
external get : value -> string -> 'a = "" [@@mel.get_index]
external output : 'a -> value = "%identity"
external flattened : vertices:value Js.undefined array -> holes:int array -> dimensions:int -> unit -> value = "" [@@mel.obj]

let dim v = match Js.Nullable.toOption (get v "dim") with None -> 2 | Some d -> d
let holes v = match Js.Nullable.toOption (get v "holeIndices") with None -> [||] | Some h -> h
let triangulate item refine =
  let data = get item "data" and stride = dim item in
  let triangles = Clip.earcut data (holes item) stride in
  if refine then Refine.refine triangles data stride;
  triangles

let run input =
  match (get input "op" : string) with
  | "earcut" -> output (triangulate input false)
  | "earcutRefine" -> output (triangulate input true)
  | "deviation" -> output (Geometry.deviation (get input "data") (holes input) (dim input) (get input "triangles"))
  | "refine" ->
    let triangles = Array.copy (get input "triangles") in
    Refine.refine triangles (get input "coords") (dim input); output triangles
  | "flatten" ->
    let vertices, holes, dimensions = Flatten.flatten ~missing:Js.undefined (get input "rings") in
    flattened ~vertices ~holes ~dimensions ()
  | "batch" -> output (Array.map (fun item -> triangulate item (get item "refine")) (get input "items"))
  | _ -> invalid_arg "unknown earcut operation"
