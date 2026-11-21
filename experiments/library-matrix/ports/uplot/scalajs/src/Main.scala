// The boundary: what libraries/uplot/reference.mjs does, on the port's uPlot slice. The corpus
// input is viewed through @js.native facade traits with no copy; the result is built from
// js.Arrays. `run` is a native ES module export through @JSExportTopLevel.

package uplot

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

@js.native
trait Input extends js.Object:
  val spanGaps: Boolean = js.native
  val frames: js.Array[Frame] = js.native

@js.native
trait Frame extends js.Object:
  val size: js.Array[Double] = js.native
  val xRange: js.Array[Double] = js.native
  val yRange: js.Array[Double] = js.native
  /** The aligned data: x values, then one array per series with null for a missing value. */
  val data: js.Array[js.Array[Double | Null]] = js.native
  val probes: js.Array[js.Array[Double]] = js.native

object Main:
  /** The adapter's precise(): Math.round(x * 1e6) || 0, which maps -0 and NaN to 0. */
  private def precise(value: Double): Double =
    val n = js.Math.round(value * 1e6)
    if n == 0 || n.isNaN then 0.0 else n

  /** One frame: setSize, setData and setScale in a batch, then the recorded paths and the probes. */
  private def frame(canvas: Canvas, ctx: Context2D, spanGaps: Boolean, f: Frame, pxRatio: Double, state: ScaleState): js.Array[Any] =
    // The adapter passes CSS sizes; uPlot multiplies them back by pxRatio.
    val widthCss = f.size(0) / pxRatio
    val heightCss = f.size(1) / pxRatio
    val plot = Plot(pxRatio, widthCss, heightCss)
    canvas.width = js.Math.round(widthCss * pxRatio)
    canvas.height = js.Math.round(heightCss * pxRatio)

    // x values are never null.
    val xs = f.data(0).asInstanceOf[js.Array[Double]]
    val chart = state.applyScales(plot, Scale(f.xRange(0), f.xRange(1)), Scale(f.yRange(0), f.yRange(1)), xs)
    val series = f.data.drop(1)

    // drawSeries runs only with data; the adapter then reports an empty trace for each series.
    val paths = if xs.nonEmpty then Some(series.map(linear(chart, _, spanGaps))) else None
    paths.foreach(_.foreach(drawPath(ctx, plot, _)))
    val traces = paths match
      case Some(ps) => ps.map(p => js.Array[Any](p.stroke.vertices, p.gaps.orNull))
      case None => series.map(_ => js.Array[Any](js.Array[Any](), if spanGaps then null else js.Array[Any]()))

    val probes = f.probes.map { q =>
      val x = q(0)
      val y = q(1)
      val px = q(2)
      val py = q(3)
      js.Array[Any](
        precise(chart.valToPosX(x)), precise(chart.valToPosY(y)),
        precise(chart.posToValX(px)), precise(chart.posToValY(py)),
        chart.valToIdx(x), chart.posToIdx(px),
      )
    }
    js.Array(canvas.width, canvas.height, traces, probes)

  @JSExportTopLevel("run")
  def run(input: Input): js.Array[js.Array[Any]] =
    val pxRatio = Window.devicePixelRatio
    // The canvas stands in for uPlot's elements in the adapter's host div; both leave the page.
    val canvas = Window.document.createElement("canvas")
    Window.document.body.appendChild(canvas)
    try
      val ctx = canvas.getContext("2d")
      val state = ScaleState()
      input.frames.map(frame(canvas, ctx, input.spanGaps, _, pxRatio, state))
    finally canvas.remove()
