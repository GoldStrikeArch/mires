// Boundary records match the JavaScript API. A refine call owns its triangle copy.
type item = {data: array<float>, dim?: int, holeIndices: Nullable.t<array<int>>, refine?: bool}
type input = {
  op: string,
  data: array<float>,
  dim?: int,
  holeIndices: Nullable.t<array<int>>,
  triangles: array<int>,
  coords: array<float>,
  items: array<item>,
  rings: array<array<array<float>>>,
}
type result
external output: 'a => result = "%identity"

let triangulate = (data, holes, dim, refine) => {
  let triangles = Clip.earcut(data, holes->Nullable.getOr([]), dim)
  if refine {
    Refine.refine(triangles, data, dim)
  }
  triangles
}

let run = (input: input): result => {
  let dim = input.dim->Option.getOr(2)
  switch input.op {
  | "earcut" | "earcutRefine" =>
    output(triangulate(input.data, input.holeIndices, dim, input.op == "earcutRefine"))
  | "batch" =>
    output(
      input.items->Array.map(item =>
        triangulate(
          item.data,
          item.holeIndices,
          item.dim->Option.getOr(2),
          item.refine->Option.getOr(false),
        )
      ),
    )
  | "refine" =>
    let triangles = input.triangles->Array.copy
    Refine.refine(triangles, input.coords, dim)
    output(triangles)
  | "deviation" =>
    output(
      Geometry.deviation(input.data, input.holeIndices->Nullable.getOr([]), dim, input.triangles),
    )
  | "flatten" =>
    let (vertices, holes, dimensions) = Flatten.flatten(input.rings)
    output({"vertices": vertices, "holes": holes, "dimensions": dimensions})
  | op => JsError.TypeError.make(`unknown operation ${op}`)->JsError.throw
  }
}
