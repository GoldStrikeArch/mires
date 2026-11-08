private const val IMAGE_UNSUPPORTED = "unsupported"
private const val IMAGE_SKIPPED = "skipped"
private const val IMAGE_UNSTABLE = "unstable"

internal fun getCanvasFingerprint(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    return getUnstableCanvasFingerprintImpl(doesBrowserPerformCanvasAntiFingerprinting())
}

internal fun getUnstableCanvasFingerprint(skipImages: dynamic): dynamic {
    return getUnstableCanvasFingerprintImpl(isTruthy(skipImages))
}

private fun getUnstableCanvasFingerprintImpl(skipImages: Boolean): dynamic {
    var winding = false
    var geometry: String
    var text: String
    val canvas = jsDocument().createElement("canvas")
    canvas.width = 1
    canvas.height = 1
    val context = canvas.getContext("2d")
    if (!(isTruthy(context) && isTruthy(canvas.toDataURL))) {
        geometry = IMAGE_UNSUPPORTED
        text = IMAGE_UNSUPPORTED
    } else {
        winding = doesSupportWinding(context)
        if (skipImages) {
            geometry = IMAGE_SKIPPED
            text = IMAGE_SKIPPED
        } else {
            val images = renderImages(canvas, context)
            geometry = jsString(jsIndex(images, 0))
            text = jsString(jsIndex(images, 1))
        }
    }
    val result = obj()
    jsSet(result, "winding", winding)
    jsSet(result, "geometry", geometry)
    jsSet(result, "text", text)
    return result
}

private fun doesSupportWinding(context: dynamic): Boolean {
    context.rect(0, 0, 10, 10)
    context.rect(2, 2, 6, 6)
    return !isTruthy(context.isPointInPath(5, 5, "evenodd"))
}

private fun renderImages(canvas: dynamic, context: dynamic): dynamic {
    renderTextImage(canvas, context)
    val textImage1 = jsString(canvas.toDataURL())
    val textImage2 = jsString(canvas.toDataURL())
    if (textImage1 != textImage2) {
        val unstable = arr()
        jsPush(unstable, IMAGE_UNSTABLE)
        jsPush(unstable, IMAGE_UNSTABLE)
        return unstable
    }
    renderGeometryImage(canvas, context)
    val pair = arr()
    jsPush(pair, jsString(canvas.toDataURL()))
    jsPush(pair, textImage1)
    return pair
}

private fun renderTextImage(canvas: dynamic, context: dynamic) {
    canvas.width = 240
    canvas.height = 60
    context.textBaseline = "alphabetic"
    context.fillStyle = "#f60"
    context.fillRect(100, 1, 62, 20)
    context.fillStyle = "#069"
    context.font = "11pt \"Times New Roman\""
    val printedText = "Cwm fjordbank gly " + jsFromCharCode(55357, 56835)
    context.fillText(printedText, 2, 15)
    context.fillStyle = "rgba(102, 204, 0, 0.2)"
    context.font = "18pt Arial"
    context.fillText(printedText, 4, 45)
}

private fun renderGeometryImage(canvas: dynamic, context: dynamic) {
    canvas.width = 122
    canvas.height = 110
    context.globalCompositeOperation = "multiply"
    val colors = arrayOf("#f2f", "#2ff", "#ff2")
    val xs = arrayOf(40, 80, 60)
    val ys = arrayOf(40, 40, 80)
    var i = 0
    while (i < 3) {
        context.fillStyle = colors[i]
        context.beginPath()
        context.arc(xs[i], ys[i], 40, 0, jsPi() * 2, true)
        context.closePath()
        context.fill()
        i += 1
    }
    context.fillStyle = "#f9c"
    context.arc(60, 60, 60, 0, jsPi() * 2, true)
    context.arc(60, 60, 20, 0, jsPi() * 2, true)
    context.fill("evenodd")
}

private fun doesBrowserPerformCanvasAntiFingerprinting(): Boolean {
    val isSafari17OrAbove = isWebKit() && isWebKit616OrNewer() && isSafariWebKit()
    val isFirefox120OrAbove = isGecko() && isGecko120OrNewer()
    return isSafari17OrAbove || isFirefox120OrAbove
}
