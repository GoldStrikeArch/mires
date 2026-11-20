package earcut

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

@js.native
trait Polygon extends js.Object {
  val data: js.Array[Double] = js.native
  val holeIndices: js.UndefOr[js.Array[Int]] = js.native
  val dim: js.UndefOr[Int] = js.native
  val refine: js.UndefOr[Boolean] = js.native
}

@js.native
trait Input extends Polygon {
  val op: String = js.native
  val triangles: js.Array[Int] = js.native
  val coords: js.Array[Double] = js.native
  val rings: js.Array[js.Array[js.Array[js.UndefOr[Double]]]] = js.native
  val items: js.Array[Polygon] = js.native
}

object Main {
  private def holes(p: Polygon): js.Array[Int] =
    p.holeIndices.toOption.filter(_ != null).getOrElse(new js.Array[Int]())

  private def triangulate(p: Polygon, refine: Boolean): js.Array[Int] = {
    val core = new Earcut()
    val dim = p.dim.getOrElse(2)
    val triangles = core.earcut(p.data, holes(p), dim)
    if (refine) core.refine(triangles, p.data, dim)
    triangles
  }

  private def flatten(rings: js.Array[js.Array[js.Array[js.UndefOr[Double]]]]): js.Object = {
    val dimensions = rings(0)(0).length
    val vertices = new js.Array[js.UndefOr[Double]]()
    val offsets = new js.Array[Int]()
    var previousLength = 0
    var offset = 0
    for (ring <- rings) {
      for (point <- ring; d <- 0 until dimensions) vertices.push(point(d))
      if (previousLength > 0) {
        offset += previousLength
        offsets.push(offset)
      }
      previousLength = ring.length
    }
    js.Dynamic.literal(vertices = vertices, holes = offsets, dimensions = dimensions)
  }

  @JSExportTopLevel("run")
  def run(input: Input): js.Any = input.op match {
    case "earcut" => triangulate(input, false)
    case "earcutRefine" => triangulate(input, true)
    case "refine" =>
      val copy = input.triangles.slice(0, input.triangles.length)
      new Earcut().refine(copy, input.coords, input.dim.getOrElse(2))
      copy
    case "deviation" => new Earcut().deviation(input.data, holes(input), input.dim.getOrElse(2), input.triangles)
    case "flatten" => flatten(input.rings)
    case "batch" => input.items.map(item => triangulate(item, item.refine.getOrElse(false)))
    case _ => throw js.JavaScriptException(new js.TypeError("unknown operation " + input.op))
  }
}
