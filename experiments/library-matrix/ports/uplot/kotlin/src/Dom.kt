// The browser APIs the chart uses, as external declarations: the document, the canvas and its 2D
// context, Path2D, devicePixelRatio and JS Math.round.

internal external val document: Document

/** window.devicePixelRatio, which uPlot reads as its pxRatio. */
internal external val devicePixelRatio: Double

internal external interface Document {
    val body: Element
    fun createElement(tagName: String): Canvas
}

internal external interface Element {
    fun appendChild(node: Canvas)
}

internal external interface Canvas {
    var width: Double
    var height: Double
    fun getContext(contextId: String): Context2D
    fun remove()
}

internal external interface Context2D {
    var strokeStyle: String
    var lineWidth: Double
    var lineJoin: String
    var lineCap: String
    fun save()
    fun restore()
    fun translate(x: Double, y: Double)
    fun clip(path: Path2D)
    fun stroke(path: Path2D)
}

internal external class Path2D {
    fun lineTo(x: Double, y: Double)
    fun rect(x: Double, y: Double, w: Double, h: Double)
}

/** JS Math.round: halves round toward +Infinity (kotlin.math.round rounds them to even). */
@JsName("Math")
internal external object JsMath {
    fun round(x: Double): Double
}
