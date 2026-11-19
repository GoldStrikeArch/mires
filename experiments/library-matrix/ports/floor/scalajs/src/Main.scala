// Floor port (Scala.js): a native facade trait and a top-level ES module export.
package floor

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

@js.native
trait Input extends js.Object:
  val text: String = js.native

object Main:
  @JSExportTopLevel("run")
  def run(input: Input): String = input.text
