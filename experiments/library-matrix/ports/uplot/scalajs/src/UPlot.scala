// The slice of uPlot 1.6.32 (node_modules/uplot/dist/uPlot.esm.js) that the reference chart runs:
// the rounding helpers, calcSize's plot box, the value/pixel conversions, setScales' index window,
// paths.linear with findGaps and clipGaps, and drawPath. As in the reference, the x scale has
// ori 0 and the y scale ori 1, both dir 1, and the series has pxAlign 1, alignGaps 0, width 1,
// no fill, no bands and no points.

package uplot

import scala.scalajs.js

/** Number.EPSILON, 2^-52. */
private val Epsilon = 2.220446049250313e-16

/** Number.isInteger. */
private def isInt(value: Double): Boolean = java.lang.Double.isFinite(value) && math.floor(value) == value

/** roundDec: rounds half away from zero at `dec` decimals. */
def roundDec(value: Double, dec: Int = 0): Double =
  if isInt(value) then value
  else
    val p = math.pow(10, dec)
    val n = value * p * (1 + Epsilon)
    js.Math.round(n) / p

/** uPlot's regex6: the decimals before a run of six or more 9s or 0s. */
private val regex6 = js.RegExp("""\.\d*?(?=9{6,}|0{6,})""")

/** fixFloat: snaps a value whose JS string has a run of six or more 9s or 0s after the point to the
  * decimals before the run (17999.204999999998 becomes 17999.205). uPlot first returns members of
  * its fixedDec map unchanged; the string path returns every member unchanged too. Its branch for
  * strings with `e-` (values below 1e-6) is not reached by plot sizes of 1 px or more.
  */
def fixFloat(value: Double): Double =
  if isInt(value) then value
  else
    // Double.toString is the JS string on Scala.js.
    val m = regex6.exec(value.toString)
    if m == null then value else roundDec(value, m(0).get.length - 1)

def incrRound(num: Double, incr: Double): Double = fixFloat(roundDec(fixFloat(num / incr)) * incr)

/** closestIdx: the index of the value in `arr` closest to `num`. JS `lo || 0` and
  * `hi || arr.length - 1` make a zero `hi` the last index.
  */
def closestIdx(num: Double, arr: js.Array[Double], lo: Int = 0, hi: Int = 0): Int =
  var low = lo
  var high = if hi != 0 then hi else arr.length - 1
  while high - low > 1 do
    val mid = (low + high) >> 1
    if arr(mid) < num then low = mid else high = mid
  // An empty array reads undefined on both sides, so the comparison fails and `hi` (-1) wins.
  if arr.nonEmpty && num - arr(low) <= arr(high) - num then low else high

/** A linear scale after setScale: uPlot's `_min` and `_max`. */
final class Scale(val min: Double, val max: Double):
  def valToPct(value: Double): Double = (value - min) / (max - min)

/** The plot box. uPlot gets CSS dimensions (the adapter divides the physical size by pxRatio) and
  * calcSize multiplies them back, rounding to half pixels with incrRound. Without padding or axes
  * the box starts at the canvas origin.
  */
final class Plot(val pxRatio: Double, val widthCss: Double, val heightCss: Double):
  val leftCss = 0.0
  val topCss = 0.0
  val left: Double = incrRound(leftCss * pxRatio, 0.5)
  val top: Double = incrRound(topCss * pxRatio, 0.5)
  val width: Double = incrRound(widthCss * pxRatio, 0.5)
  val height: Double = incrRound(heightCss * pxRatio, 0.5)

/** One frame's chart: what uPlot's valToPos, posToVal, valToIdx and posToIdx read. */
final class Chart(val plot: Plot, val x: Scale, val y: Scale, val xs: js.Array[Double]):
  // setScales' visible window. With no data uPlot leaves it null, which closestIdx reads as 0.
  private val (i0, i1) =
    if xs.isEmpty then (0, 0)
    else
      var first = closestIdx(x.min, xs)
      var last = closestIdx(x.max, xs)
      // Don't contract same or adjacent indices; the closest ones can be outside the view.
      if last - first > 1 then
        if xs(first) < x.min then first += 1
        if xs(last) > x.max then last -= 1
      (first, last)

  /** getHPos (ori 0, dir 1) in canvas pixels: valToPos(value, "x", true). */
  def valToPosX(value: Double): Double = plot.left + plot.width * x.valToPct(value)

  /** getVPos (ori 1, dir 1) in canvas pixels: valToPos(value, "y", true). */
  def valToPosY(value: Double): Double = plot.top + plot.height * (1 - y.valToPct(value))

  /** posToVal(pos, "x", true). */
  def posToValX(pos: Double): Double =
    val css = pos / plot.pxRatio - plot.leftCss
    x.min + (x.max - x.min) * (css / plot.widthCss)

  /** posToVal(pos, "y", true): ori 1 measures from the bottom. */
  def posToValY(pos: Double): Double =
    val css = pos / plot.pxRatio - plot.topCss
    val dim = plot.heightCss
    y.min + (y.max - y.min) * ((dim - css) / dim)

  def valToIdx(value: Double): Int = closestIdx(value, xs)

  def posToIdx(pos: Double): Int = closestIdx(posToValX(pos), xs, i0, i1)

/** The stroke path, recording each lineTo as the adapter's RecordedPath does. */
final class RecordedPath:
  val path = new Path2D()
  val vertices = js.Array[js.Array[Double]]()

  def lineTo(x: Double, y: Double): Unit =
    vertices.push(js.Array(x, y))
    path.lineTo(x, y)

/** What paths.linear returns, less the parts this chart never sets (fill, band). `gaps` and `clip`
  * are None with spanGaps; `clip` is also None without gaps.
  */
final class SeriesPaths(val stroke: RecordedPath, val gaps: Option[js.Array[js.Array[Double]]], val clip: Option[Path2D])

/** One pixel column while decimating: uPlot's inY, outY, minY and maxY, which exist while
  * minY != null.
  */
private final class Accumulator(val inY: Double):
  var outY: Double = inY
  var minY: Double = inY
  var maxY: Double = inY

  def add(y: Double): Unit =
    outY = y
    if y < minY then minY = y else if y > maxY then maxY = y

/** _drawAcc with lineToH: a column's extremes, then its last value, all in pixels. */
private def drawAcc(stroke: RecordedPath, accX: Double, minY: Double, maxY: Double, inY: Double, outY: Double): Unit =
  if minY != maxY then
    if inY != minY && outY != minY then stroke.lineTo(accX, minY)
    if inY != maxY && outY != maxY then stroke.lineTo(accX, maxY)
    stroke.lineTo(accX, outY)

/** findGaps for dir 1 and alignGaps 0: a run of nulls spans from the pixel of the value before it
  * to the pixel of the value after it. Both neighbours exist, since idx0 and idx1 hold values, so
  * uPlot's fallbacks to the run's own ends never apply.
  */
private def findGaps(xs: js.Array[Double], ys: js.Array[Double | Null], idx0: Int, idx1: Int,
    pixelForX: Double => Double): js.Array[js.Array[Double]] =
  val gaps = js.Array[js.Array[Double]]()
  var i = idx0
  while i <= idx1 do
    if ys(i) == null then
      val fr = i
      var to = i
      i += 1
      while i <= idx1 && ys(i) == null do
        to = i
        i += 1
      val frPx = pixelForX(xs(fr - 1))
      val toPx = pixelForX(xs(to + 1))
      if toPx >= frPx then gaps.push(js.Array(frPx, toPx))
    i += 1
  gaps

/** clipGaps for ori 0: rectangles over everything but the gaps. */
private def clipGaps(gaps: js.Array[js.Array[Double]], plot: Plot): Option[Path2D] =
  if gaps.isEmpty then None
  else
    val clip = new Path2D()
    var prevGapEnd = plot.left
    for g <- gaps do
      if g(1) > g(0) then
        val w = g(0) - prevGapEnd
        if w > 0 then clip.rect(prevGapEnd, plot.top, w, plot.top + plot.height)
        prevGapEnd = g(1)
    val w = plot.left + plot.width - prevGapEnd
    // uPlot's hack to keep strokes at the edges from being cut off.
    val maxStrokeWidth = 10.0
    if w > 0 then clip.rect(prevGapEnd, plot.top - maxStrokeWidth / 2, w, plot.top + plot.height + maxStrokeWidth)
    Some(clip)

/** paths.linear's decimating loop: one vertex per value until a pixel column holds two or more.
  * Returns hasGap.
  */
private def strokeDecimated(u: Chart, stroke: RecordedPath, dataY: js.Array[Double | Null], idx0: Int, idx1: Int,
    pixelForX: Double => Double, pixelForY: Double => Double): Boolean =
  val dataX = u.xs
  val xForPixel = (pos: Double) => u.posToValX(pos)
  var hasGap = false
  var acc: Option[Accumulator] = None
  var accX = pixelForX(dataX(idx0))
  val idx0px = pixelForX(dataX(idx0))
  // The limit of the current x bucket, to avoid computing the pixel of every x value.
  var nextAccXVal = xForPixel(idx0px + 1)

  def flush(a: Accumulator): Unit =
    drawAcc(stroke, accX, pixelForY(a.minY), pixelForY(a.maxY), pixelForY(a.inY), pixelForY(a.outY))

  for i <- idx0 to idx1 do
    val xVal = dataX(i)
    val reuseAccX = xVal < nextAccXVal
    val x = if reuseAccX then accX else pixelForX(xVal)

    if x == accX then
      dataY(i) match
        case yVal: Double =>
          acc match
            case Some(a) => a.add(yVal)
            case None =>
              stroke.lineTo(x, pixelForY(yVal))
              acc = Some(Accumulator(yVal))
        case null => hasGap = true
    else
      acc.foreach(flush)
      dataY(i) match
        case yVal: Double =>
          stroke.lineTo(x, pixelForY(yVal))
          acc = Some(Accumulator(yVal))
        case null =>
          acc = None
          hasGap = true
      accX = x
      nextAccXVal = xForPixel(accX + 1)

  acc.foreach(a => if a.minY != a.maxY then flush(a))
  hasGap

/** paths.linear for one series over the whole data range, as the adapter calls it. */
def linear(u: Chart, dataY: js.Array[Double | Null], spanGaps: Boolean): SeriesPaths =
  val dataX = u.xs
  // nonNullIdxs over the whole range: the first and last indices holding a value, or -1.
  val idx0 = dataY.indexWhere(_ != null)
  val idx1 = dataY.lastIndexWhere(_ != null)
  val pixelForX = (value: Double) => js.Math.round(u.valToPosX(value))
  val pixelForY = (value: Double) => js.Math.round(u.valToPosY(value))
  val stroke = RecordedPath()
  // Decimate when the number of points is at least 4x the available pixels.
  val decimate = idx1 - idx0 >= u.plot.width * 4

  val hasGap =
    // An all-null series has idx0 = idx1 = -1, where uPlot's loop reads undefined and draws nothing.
    if idx0 < 0 then false
    else if decimate then strokeDecimated(u, stroke, dataY, idx0, idx1, pixelForX, pixelForY)
    else
      var nulls = false
      for i <- idx0 to idx1 do
        dataY(i) match
          case yVal: Double => stroke.lineTo(pixelForX(dataX(i)), pixelForY(yVal))
          case null => nulls = true
      nulls

  if spanGaps then SeriesPaths(stroke, None, None)
  else
    // series.gaps is the identity.
    val gaps = if hasGap then findGaps(dataX, dataY, idx0, idx1, pixelForX) else js.Array[js.Array[Double]]()
    SeriesPaths(stroke, Some(gaps), clipGaps(gaps, u.plot))

/** The adapter's series options: stroke colour and width. */
private val SeriesStroke = "#172839"
private val SeriesWidth = 1.0

/** drawPath for a line series, with strokeFill's clips and setCtxStyle's defaults. */
def drawPath(ctx: Context2D, plot: Plot, paths: SeriesPaths): Unit =
  val width = roundDec(SeriesWidth * plot.pxRatio, 3)
  val offset = (width % 2) / 2
  val pxAlign = offset > 0

  if pxAlign then ctx.translate(offset, offset)

  val boundsClip = new Path2D()
  boundsClip.rect(plot.left - width / 2, plot.top - width / 2, plot.width + width, plot.height + width)

  ctx.strokeStyle = SeriesStroke
  ctx.lineWidth = width
  ctx.lineJoin = "round"
  ctx.lineCap = "butt"
  ctx.save()
  ctx.clip(boundsClip)
  paths.clip.foreach(clip => ctx.clip(clip))
  if width > 0 then ctx.stroke(paths.stroke.path)
  ctx.restore()

  if pxAlign then ctx.translate(-offset, -offset)
