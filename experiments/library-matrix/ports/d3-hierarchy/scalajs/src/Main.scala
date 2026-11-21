// Entry of the Scala.js port: one layout per input, as libraries/d3-hierarchy/reference.mjs runs
// it. The input is read in place through @js.native facades; the rows are JS arrays of strings,
// numbers and null.
package hierarchy

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

@js.native trait Input extends js.Object:
  val op: String = js.native
  val source: String = js.native
  val data: Datum = js.native
  val rows: js.Array[Datum] = js.native
  val width: Double = js.native
  val height: Double = js.native
  val padding: Double = js.native
  val sort: Boolean = js.native
  val round: Boolean = js.native
  val nodeSize: Boolean = js.native
  val tile: String = js.native

object Main:
  private def tile(name: String): Tile = name match
    case "binary" => treemapBinary
    case "sliceDice" => treemapSliceDice
    case "squarify" => treemapSquarify
    case _ => throw IllegalArgumentException("unknown tile") // `.tile(undefined)` throws

  /** The reference's `n.id ?? n.data.id`: the datum's id as the input has it, null or undefined included. */
  private def id(node: Node): js.UndefOr[String] = node.id match
    case Some(id) => id
    case None => node.data.get.id

  @JSExportTopLevel("run")
  def run(input: Input): js.Array[js.Array[Any]] =
    val root = input.source match
      case "id" => stratify(input.rows)
      case "path" => stratify(input.rows, Some(_.id.get))
      case _ => hierarchy(input.data)
    root.sum(_.flatMap(d => present(d.value)).getOrElse(0.0))
    if input.sort then root.sort((a, b) => b.value - a.value)
    val w = input.width
    val h = input.height
    val p = input.padding
    input.op match
      case "tree" => tree(root, w, h, input.nodeSize)
      case "cluster" => cluster(root, w, h, input.nodeSize)
      case "partition" => partition(root, w, h, p, input.round)
      case "pack" => pack(root, w, h, p)
      case "treemap" => treemap(root, w, h, tile(input.tile), p, input.round)
      case "hierarchy" | "iterator" | "stratify" => ()
      case _ => throw IllegalArgumentException("unknown layout")
    val output = js.Array[js.Array[Any]]()
    def visit(n: Node): Unit =
      val record = js.Array[Any](id(n), n.parent.fold[Any](null)(id), n.depth, n.height, n.value)
      input.op match
        case "tree" | "cluster" => record.push(n.x, n.y)
        case "pack" => record.push(n.x, n.y, n.r)
        case "partition" | "treemap" => record.push(n.x0, n.y0, n.x1, n.y1)
        case _ => ()
      output.push(record)
    if input.op == "iterator" then root.breadthFirst.foreach(visit) else root.eachBefore(visit)
    output
