// The boundary: what libraries/uplot/reference.mjs does, on the port's uPlot slice. The corpus
// input is typed by external interfaces and read in place; the result is built from Kotlin arrays,
// which are JS arrays. `run` is a native ES module export through @JsExport.

external interface Input {
    val spanGaps: Boolean
    val frames: Array<Frame>
}

external interface Frame {
    val size: Array<Double>
    val xRange: Array<Double>
    val yRange: Array<Double>
    /** The aligned data: x values, then one array per series with null for a missing value. */
    val data: Array<Array<Double?>>
    val probes: Array<Array<Double>>
}

/** The adapter's precise(): Math.round(x * 1e6) || 0, which maps -0 and NaN to 0. */
private fun precise(value: Double): Double {
    val n = JsMath.round(value * 1e6)
    return if (n == 0.0 || n.isNaN()) 0.0 else n
}

/** One frame: setSize, setData and setScale in a batch, then the recorded paths and the probes. */
private fun frame(canvas: Canvas, ctx: Context2D, spanGaps: Boolean, f: Frame, pxRatio: Double): Array<Any?> {
    // The adapter passes CSS sizes; uPlot multiplies them back by pxRatio.
    val widthCss = f.size[0] / pxRatio
    val heightCss = f.size[1] / pxRatio
    val plot = Plot(pxRatio, widthCss, heightCss)
    canvas.width = JsMath.round(widthCss * pxRatio)
    canvas.height = JsMath.round(heightCss * pxRatio)

    val xs = f.data[0].unsafeCast<Array<Double>>()
    val chart = Chart(plot, Scale(f.xRange[0], f.xRange[1]), Scale(f.yRange[0], f.yRange[1]), xs)
    val series = f.data.drop(1)

    // drawSeries runs only with data; the adapter then reports an empty trace for each series.
    val paths = if (xs.isNotEmpty()) series.map { linear(chart, it, spanGaps) } else null
    paths?.forEach { drawPath(ctx, plot, it) }
    val traces = series.indices.map { i ->
        val p = paths?.get(i)
        if (p != null) {
            arrayOf(p.stroke.vertices.toTypedArray(), p.gaps?.toTypedArray())
        } else {
            arrayOf(emptyArray<Any>(), if (spanGaps) null else emptyArray<Any>())
        }
    }

    val probes = f.probes.map { (x, y, px, py) ->
        arrayOf(
            precise(chart.valToPosX(x)), precise(chart.valToPosY(y)),
            precise(chart.posToValX(px)), precise(chart.posToValY(py)),
            chart.valToIdx(x), chart.posToIdx(px),
        )
    }
    return arrayOf(canvas.width, canvas.height, traces.toTypedArray(), probes.toTypedArray())
}

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: Input): Array<Array<Any?>> {
    val pxRatio = devicePixelRatio
    // The canvas stands in for uPlot's elements in the adapter's host div; both leave the page.
    val canvas = document.createElement("canvas")
    document.body.appendChild(canvas)
    try {
        val ctx = canvas.getContext("2d")
        return input.frames.map { frame(canvas, ctx, input.spanGaps, it, pxRatio) }.toTypedArray()
    } finally {
        canvas.remove()
    }
}
