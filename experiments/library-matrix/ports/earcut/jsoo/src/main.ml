open Js_of_ocaml

let field v key = Js.Unsafe.get v key
let array_field v key = Js.to_array (field v key)
let dim v = Js.Optdef.get (field v "dim") (fun () -> 2)
let holes v =
  let h = field v "holeIndices" in
  if Js.Unsafe.equals h Js.null then [||] else Js.to_array h

let triangulate item refine =
  let data = array_field item "data" and stride = dim item in
  let triangles = Clip.earcut data (holes item) stride in
  if refine then Refine.refine triangles data stride;
  Js.array triangles

let run input =
  let box = Js.Unsafe.inject in
  match Js.to_string (field input "op") with
  | "earcut" -> box (triangulate input false)
  | "earcutRefine" -> box (triangulate input true)
  | "deviation" -> box (Geometry.deviation (array_field input "data") (holes input) (dim input) (array_field input "triangles"))
  | "refine" ->
    let triangles = array_field input "triangles" in
    Refine.refine triangles (array_field input "coords") (dim input); box (Js.array triangles)
  | "flatten" ->
    let rings = Array.map (fun r -> Array.map Js.to_array (Js.to_array r)) (array_field input "rings") in
    let vertices, holes, dimensions = Flatten.flatten ~missing:Js.undefined rings in
    box (Js.Unsafe.obj [| "vertices", box (Js.array vertices); "holes", box (Js.array holes); "dimensions", box dimensions |])
  | "batch" ->
    let items = array_field input "items" in
    box (Js.array (Array.map (fun item -> triangulate item (Js.to_bool (field item "refine"))) items))
  | _ -> invalid_arg "unknown earcut operation"

let () = Js.export "run" (Js.wrap_callback run)
