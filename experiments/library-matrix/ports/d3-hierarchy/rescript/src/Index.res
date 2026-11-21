// Entry of the ReScript port: one layout per input, as libraries/d3-hierarchy/reference.mjs runs
// it. The input record is the corpus object itself (no conversion): strings are JS strings and
// numbers are floats. The result is a JS array of rows.

open Hierarchy

type input = {
  op: string,
  source: string,
  data?: datum,
  rows?: array<datum>,
  width: float,
  height: float,
  padding: float,
  sort: bool,
  nodeSize: bool,
  round: bool,
  tile: string,
}

/** A cell of a result row: a string, null or a number. */
type cell
external cell: 'a => cell = "%identity"

let tile = name =>
  switch name {
  | "binary" => Treemap.binary
  | "sliceDice" => Treemap.sliceDice
  | "squarify" => Treemap.squarify
  | _ => JsError.throwWithMessage("unknown tile") // `.tile(undefined)` throws
  }

/** The reference's `n.id ?? n.data.id`: the datum's id as the input has it, null or undefined included. */
let id = node =>
  switch node.id {
  | Some(id) => cell(id)
  | None => cell((node.data->Option.getOrThrow).id)
  }

let run = (input: input) => {
  let root = switch input.source {
  | "id" => stratify(input.rows->Option.getOrThrow)
  | "path" => stratify(~path=d => d.id->Nullable.getOrThrow, input.rows->Option.getOrThrow)
  | _ => hierarchy(input.data->Option.getOrThrow)
  }
  root->sum(data => data->Option.flatMap(d => d.value->Nullable.toOption)->Option.getOr(0.))->ignore
  if input.sort {
    root->sort((a, b) => b.value -. a.value)->ignore
  }
  let {width: w, height: h, padding: p} = input
  switch input.op {
  | "tree" => Tree.tree(root, w, h, input.nodeSize)
  | "cluster" => Tree.cluster(root, w, h, input.nodeSize)
  | "partition" => Treemap.partition(root, w, h, p, input.round)
  | "pack" => Pack.pack(root, w, h, p)
  | "treemap" => Treemap.treemap(root, w, h, tile(input.tile), p, input.round)
  | "hierarchy" | "iterator" | "stratify" => ()
  | _ => JsError.throwWithMessage("unknown layout")
  }
  let coordinates = node =>
    switch input.op {
    | "tree" | "cluster" => [node.x, node.y]
    | "pack" => [node.x, node.y, node.r]
    | "partition" | "treemap" => [node.x0, node.y0, node.x1, node.y1]
    | _ => []
    }
  let record = node =>
    [
      id(node),
      switch node.parent {
      | Some(parent) => id(parent)
      | None => cell(Null.null)
      },
      cell(node.depth),
      cell(node.height),
      cell(node.value),
    ]->Array.concat(coordinates(node)->Array.map(cell))
  let output = []
  if input.op == "iterator" {
    root->breadthFirst->Array.forEach(node => output->Array.push(record(node)))
  } else {
    root->eachBefore(node => output->Array.push(record(node)))->ignore
  }
  output
}
