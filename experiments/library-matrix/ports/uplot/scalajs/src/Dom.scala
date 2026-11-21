// The browser APIs the chart uses, as native facades: the document, the canvas and its 2D
// context, Path2D and devicePixelRatio. JS Math.round is the standard js.Math facade.

package uplot

import scala.scalajs.js
import scala.scalajs.js.annotation.{JSGlobal, JSGlobalScope}

@js.native
@JSGlobalScope
object Window extends js.Object:
  val document: Document = js.native
  /** The ratio uPlot reads as its pxRatio. */
  val devicePixelRatio: Double = js.native

@js.native
trait Document extends js.Object:
  val body: Element = js.native
  def createElement(tagName: String): Canvas = js.native

@js.native
trait Element extends js.Object:
  def appendChild(node: Canvas): Unit = js.native

@js.native
trait Canvas extends js.Object:
  var width: Double = js.native
  var height: Double = js.native
  def getContext(contextId: String): Context2D = js.native
  def remove(): Unit = js.native

@js.native
trait Context2D extends js.Object:
  var strokeStyle: String = js.native
  var lineWidth: Double = js.native
  var lineJoin: String = js.native
  var lineCap: String = js.native
  def save(): Unit = js.native
  def restore(): Unit = js.native
  def translate(x: Double, y: Double): Unit = js.native
  def clip(path: Path2D): Unit = js.native
  def stroke(path: Path2D): Unit = js.native

@js.native
@JSGlobal
class Path2D() extends js.Object:
  def lineTo(x: Double, y: Double): Unit = js.native
  def rect(x: Double, y: Double, w: Double, h: Double): Unit = js.native
