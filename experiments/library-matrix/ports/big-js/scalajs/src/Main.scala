// The boundary: what libraries/big-js/reference.mjs does. `dp`, `rm` and `repeat` take their
// defaults when undefined or null, as the adapter's ?? does; `n` and `mode` become Options (None
// when undefined) and reach the methods as upstream's optional arguments.
package bigjs

import scala.collection.mutable.ArrayBuffer
import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

@js.native
trait Operation extends js.Object:
  val op: String = js.native
  val x: js.Any = js.native
  val y: js.Any = js.native
  val n: js.UndefOr[Double | Null] = js.native
  val mode: js.UndefOr[Double | Null] = js.native
  val dp: js.UndefOr[Double | Null] = js.native
  val rm: js.UndefOr[Double | Null] = js.native

@js.native
trait Input extends js.Object:
  val cases: js.Array[Operation] = js.native
  val repeat: js.UndefOr[Int | Null] = js.native

object Main:
  // A number option, None when it is undefined or null.
  private def option(v: js.UndefOr[Double | Null]): Option[Double] = v match
    case d: Double => Some(d)
    case _ => None

  // The Big constructor's conversion: a number goes through String(), except that -0 becomes "-0".
  private def text(v: js.Any): String = (v: Any) match
    case s: String => s
    case d: Double => if d == 0 && 1 / d < 0 then "-0" else d.toString
    case other => String.valueOf(other)

  private def operate(c: Operation): String =
    val dp = option(c.dp).getOrElse(20.0)
    val rm = option(c.rm).getOrElse(1.0)
    val x = Big.parse(text(c.x))
    val n = option(c.n)
    val mode = option(c.mode).getOrElse(rm)
    def y = Big.parse(text(c.y))
    c.op match
      case "toString" => x.toString
      case "sqrt" => x.sqrt(dp, rm).toString
      case "plus" => x.plus(y).toString
      case "minus" => x.minus(y).toString
      case "times" => x.times(y).toString
      case "div" => x.div(y, dp, rm).toString
      case "cmp" => x.cmp(y).toString
      case "pow" => x.pow(n, dp, rm).toString
      case "round" => x.round(n, mode).toString
      case "toFixed" => x.toFixed(n, mode)
      case "toExponential" => x.toExponential(n, mode)
      case "toPrecision" => x.toPrecision(n, mode)
      case _ => throw BigError("unknown decimal operation")

  @JSExportTopLevel("run")
  def run(input: Input): String =
    val repeat = input.repeat match { case n: Int => n; case _ => 1 }
    val outputs = ArrayBuffer.empty[String]
    var k = 0
    while k < repeat do
      input.cases.foreach(c => outputs += (try operate(c) catch case error: BigError => error.getMessage))
      k += 1
    outputs.mkString("\n")
