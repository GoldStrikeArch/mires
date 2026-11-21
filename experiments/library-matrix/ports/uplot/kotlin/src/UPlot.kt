// The slice of uPlot 1.6.32 (node_modules/uplot/dist/uPlot.esm.js) that the reference chart runs:
// the rounding helpers, calcSize's plot box, the value/pixel conversions, setScales' index window,
// paths.linear with findGaps and clipGaps, and drawPath. As in the reference, the x scale has
// ori 0 and the y scale ori 1, both dir 1, and the series has pxAlign 1, alignGaps 0, width 1,
// no fill, no bands and no points.

import kotlin.math.floor
import kotlin.math.pow

/** Number.EPSILON, 2^-52. */
private const val EPSILON = 2.220446049250313e-16

/** Number.isInteger. */
private fun isInt(value: Double) = value.isFinite() && floor(value) == value

/** roundDec: rounds half away from zero at `dec` decimals. */
internal fun roundDec(value: Double, dec: Int = 0): Double {
    if (isInt(value)) return value
    val p = 10.0.pow(dec)
    val n = value * p * (1 + EPSILON)
    return JsMath.round(n) / p
}

/** uPlot's regex6: the decimals before a run of six or more 9s or 0s. */
private val regex6 = Regex("""\.\d*?(?=9{6,}|0{6,})""")

/**
 * fixFloat: snaps a value whose JS string has a run of six or more 9s or 0s after the point to the
 * decimals before the run (17999.204999999998 becomes 17999.205). uPlot first returns members of
 * its fixedDec map unchanged; the string path returns every member unchanged too. Its branch for
 * strings with `e-` (values below 1e-6) is not reached by plot sizes of 1 px or more.
 */
internal fun fixFloat(value: Double): Double {
    if (isInt(value)) return value
    val match = regex6.find(value.toString()) ?: return value
    return roundDec(value, match.value.length - 1)
}

internal fun incrRound(num: Double, incr: Double) = fixFloat(roundDec(fixFloat(num / incr)) * incr)

/**
 * closestIdx: the index of the value in `arr` closest to `num`. JS `lo || 0` and
 * `hi || arr.length - 1` make a zero `hi` the last index.
 */
internal fun closestIdx(num: Double, arr: Array<Double>, lo: Int = 0, hi: Int = 0): Int {
    var low = lo
    var high = if (hi != 0) hi else arr.size - 1
    while (high - low > 1) {
        val mid = (low + high) shr 1
        if (arr[mid] < num) low = mid else high = mid
    }
    // An empty array reads undefined on both sides, so the comparison fails and `hi` (-1) wins.
    if (arr.isNotEmpty() && num - arr[low] <= arr[high] - num) return low
    return high
}

/** A linear scale after setScale: uPlot's `_min` and `_max`. */
internal class Scale(val min: Double, val max: Double) {
    fun valToPct(value: Double) = (value - min) / (max - min)
}

/**
 * The plot box. uPlot gets CSS dimensions (the adapter divides the physical size by pxRatio) and
 * calcSize multiplies them back, rounding to half pixels with incrRound. Without padding or axes
 * the box starts at the canvas origin.
 */
internal class Plot(val pxRatio: Double, val widthCss: Double, val heightCss: Double) {
    val leftCss = 0.0
    val topCss = 0.0
    val left = incrRound(leftCss * pxRatio, 0.5)
    val top = incrRound(topCss * pxRatio, 0.5)
    val width = incrRound(widthCss * pxRatio, 0.5)
    val height = incrRound(heightCss * pxRatio, 0.5)
}

/** One frame's chart: what uPlot's valToPos, posToVal, valToIdx and posToIdx read. */
internal class Chart(val plot: Plot, val x: Scale, val y: Scale, val xs: Array<Double>) {
    // setScales' visible window. With no data uPlot leaves it null, which closestIdx reads as 0.
    private val i0: Int
    private val i1: Int

    init {
        var first = 0
        var last = 0
        if (xs.isNotEmpty()) {
            first = closestIdx(x.min, xs)
            last = closestIdx(x.max, xs)
            // Don't contract same or adjacent indices; the closest ones can be outside the view.
            if (last - first > 1) {
                if (xs[first] < x.min) first++
                if (xs[last] > x.max) last--
            }
        }
        i0 = first
        i1 = last
    }

    /** getHPos (ori 0, dir 1) in canvas pixels: valToPos(value, "x", true). */
    fun valToPosX(value: Double) = plot.left + plot.width * x.valToPct(value)

    /** getVPos (ori 1, dir 1) in canvas pixels: valToPos(value, "y", true). */
    fun valToPosY(value: Double) = plot.top + plot.height * (1 - y.valToPct(value))

    /** posToVal(pos, "x", true). */
    fun posToValX(pos: Double): Double {
        val css = pos / plot.pxRatio - plot.leftCss
        return x.min + (x.max - x.min) * (css / plot.widthCss)
    }

    /** posToVal(pos, "y", true): ori 1 measures from the bottom. */
    fun posToValY(pos: Double): Double {
        val css = pos / plot.pxRatio - plot.topCss
        val dim = plot.heightCss
        return y.min + (y.max - y.min) * ((dim - css) / dim)
    }

    fun valToIdx(value: Double) = closestIdx(value, xs)

    fun posToIdx(pos: Double) = closestIdx(posToValX(pos), xs, i0, i1)
}

/** The stroke path, recording each lineTo as the adapter's RecordedPath does. */
internal class RecordedPath {
    val path = Path2D()
    val vertices = mutableListOf<Array<Double>>()

    fun lineTo(x: Double, y: Double) {
        vertices.add(arrayOf(x, y))
        path.lineTo(x, y)
    }
}

/** What paths.linear returns, less the parts this chart never sets (fill, band). */
internal class SeriesPaths(val stroke: RecordedPath, val gaps: List<Array<Double>>?, val clip: Path2D?)

/** One pixel column while decimating: uPlot's inY, outY, minY and maxY, which exist while minY != null. */
private class Accumulator(val inY: Double) {
    var outY = inY
    var minY = inY
    var maxY = inY

    fun add(y: Double) {
        outY = y
        if (y < minY) minY = y else if (y > maxY) maxY = y
    }
}

/** _drawAcc with lineToH: a column's extremes, then its last value, all in pixels. */
private fun drawAcc(stroke: RecordedPath, accX: Double, minY: Double, maxY: Double, inY: Double, outY: Double) {
    if (minY != maxY) {
        if (inY != minY && outY != minY) stroke.lineTo(accX, minY)
        if (inY != maxY && outY != maxY) stroke.lineTo(accX, maxY)
        stroke.lineTo(accX, outY)
    }
}

/**
 * findGaps for dir 1 and alignGaps 0: a run of nulls spans from the pixel of the value before it to
 * the pixel of the value after it. Both neighbours exist, since idx0 and idx1 hold values, so
 * uPlot's fallbacks to the run's own ends never apply.
 */
private fun findGaps(xs: Array<Double>, ys: Array<Double?>, idx0: Int, idx1: Int, pixelForX: (Double) -> Double): List<Array<Double>> {
    val gaps = mutableListOf<Array<Double>>()
    var i = idx0
    while (i <= idx1) {
        if (ys[i] == null) {
            val fr = i
            var to = i
            while (++i <= idx1 && ys[i] == null) to = i
            val frPx = pixelForX(xs[fr - 1])
            val toPx = pixelForX(xs[to + 1])
            if (toPx >= frPx) gaps.add(arrayOf(frPx, toPx))
        }
        i++
    }
    return gaps
}

/** clipGaps for ori 0: rectangles over everything but the gaps. */
private fun clipGaps(gaps: List<Array<Double>>, plot: Plot): Path2D? {
    if (gaps.isEmpty()) return null
    val clip = Path2D()
    var prevGapEnd = plot.left
    for (g in gaps) {
        if (g[1] > g[0]) {
            val w = g[0] - prevGapEnd
            if (w > 0) clip.rect(prevGapEnd, plot.top, w, plot.top + plot.height)
            prevGapEnd = g[1]
        }
    }
    val w = plot.left + plot.width - prevGapEnd
    // uPlot's hack to keep strokes at the edges from being cut off.
    val maxStrokeWidth = 10.0
    if (w > 0) clip.rect(prevGapEnd, plot.top - maxStrokeWidth / 2, w, plot.top + plot.height + maxStrokeWidth)
    return clip
}

/** paths.linear's decimating loop: one vertex per value until a pixel column holds two or more. */
private fun strokeDecimated(
    u: Chart, stroke: RecordedPath, dataY: Array<Double?>, idx0: Int, idx1: Int,
    pixelForX: (Double) -> Double, pixelForY: (Double) -> Double,
): Boolean {
    val dataX = u.xs
    val xForPixel = { pos: Double -> u.posToValX(pos) }
    var hasGap = false
    var acc: Accumulator? = null
    var accX = pixelForX(dataX[idx0])
    val idx0px = pixelForX(dataX[idx0])
    // The limit of the current x bucket, to avoid computing the pixel of every x value.
    var nextAccXVal = xForPixel(idx0px + 1)

    for (i in idx0..idx1) {
        val xVal = dataX[i]
        val reuseAccX = xVal < nextAccXVal
        val x = if (reuseAccX) accX else pixelForX(xVal)
        val yVal = dataY[i]

        if (x == accX) {
            if (yVal != null) {
                val current = acc
                if (current == null) {
                    stroke.lineTo(x, pixelForY(yVal))
                    acc = Accumulator(yVal)
                } else {
                    current.add(yVal)
                }
            } else {
                hasGap = true
            }
        } else {
            acc?.let { drawAcc(stroke, accX, pixelForY(it.minY), pixelForY(it.maxY), pixelForY(it.inY), pixelForY(it.outY)) }
            if (yVal != null) {
                stroke.lineTo(x, pixelForY(yVal))
                acc = Accumulator(yVal)
            } else {
                acc = null
                hasGap = true
            }
            accX = x
            nextAccXVal = xForPixel(accX + 1)
        }
    }

    acc?.let {
        if (it.minY != it.maxY) drawAcc(stroke, accX, pixelForY(it.minY), pixelForY(it.maxY), pixelForY(it.inY), pixelForY(it.outY))
    }
    return hasGap
}

/** paths.linear for one series over the whole data range, as the adapter calls it. */
internal fun linear(u: Chart, dataY: Array<Double?>, spanGaps: Boolean): SeriesPaths {
    val dataX = u.xs
    // nonNullIdxs over the whole range: the first and last indices holding a value, or -1.
    val idx0 = dataY.indexOfFirst { it != null }
    val idx1 = dataY.indexOfLast { it != null }
    val pixelForX = { value: Double -> JsMath.round(u.valToPosX(value)) }
    val pixelForY = { value: Double -> JsMath.round(u.valToPosY(value)) }
    val stroke = RecordedPath()
    // Decimate when the number of points is at least 4x the available pixels.
    val decimate = idx1 - idx0 >= u.plot.width * 4

    val hasGap = when {
        // An all-null series has idx0 = idx1 = -1, where uPlot's loop reads undefined and draws nothing.
        idx0 < 0 -> false
        decimate -> strokeDecimated(u, stroke, dataY, idx0, idx1, pixelForX, pixelForY)
        else -> {
            var nulls = false
            for (i in idx0..idx1) {
                val yVal = dataY[i]
                if (yVal == null) nulls = true else stroke.lineTo(pixelForX(dataX[i]), pixelForY(yVal))
            }
            nulls
        }
    }

    if (spanGaps) return SeriesPaths(stroke, null, null)
    // series.gaps is the identity.
    val gaps = if (hasGap) findGaps(dataX, dataY, idx0, idx1, pixelForX) else emptyList()
    return SeriesPaths(stroke, gaps, clipGaps(gaps, u.plot))
}

/** The adapter's series options: stroke colour and width. */
private const val SERIES_STROKE = "#172839"
private const val SERIES_WIDTH = 1.0

/** drawPath for a line series, with strokeFill's clips and setCtxStyle's defaults. */
internal fun drawPath(ctx: Context2D, plot: Plot, paths: SeriesPaths) {
    val width = roundDec(SERIES_WIDTH * plot.pxRatio, 3)
    val offset = (width % 2) / 2
    val pxAlign = offset > 0

    if (pxAlign) ctx.translate(offset, offset)

    val boundsClip = Path2D()
    boundsClip.rect(plot.left - width / 2, plot.top - width / 2, plot.width + width, plot.height + width)

    ctx.strokeStyle = SERIES_STROKE
    ctx.lineWidth = width
    ctx.lineJoin = "round"
    ctx.lineCap = "butt"
    ctx.save()
    ctx.clip(boundsClip)
    paths.clip?.let { ctx.clip(it) }
    if (width > 0) ctx.stroke(paths.stroke.path)
    ctx.restore()

    if (pxAlign) ctx.translate(-offset, -offset)
}
