// Entry of the Scala.js port. The boundary: the corpus input is a JS object seen through a native
// facade trait; `text` is a JS string (a Scala String with no conversion) and `top` a JS number
// read as Double (saturated to Int.MaxValue); the result String is a JS string. `run` is a native ES module export.

package pilot

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

@js.native
trait Input extends js.Object:
  val text: String = js.native
  val top: Double = js.native

object Main:
  @JSExportTopLevel("run")
  def run(input: Input): String =
    val top = if input.top >= Int.MaxValue then Int.MaxValue else input.top.toInt
    Pilot.analyse(input.text, top)
