private const val DEFAULT_FONT_TEXT = "mmMwWLliI0fiflO&1"

internal fun getFontPreferences(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    return withNaturalFonts({ document: dynamic, container: dynamic, iframeWindow: dynamic ->
        val elements = obj()
        val sizes = obj()
        val keys = arrayOf("default", "apple", "serif", "sans", "mono", "min", "system")
        for (key in keys) {
            val element = document.createElement("span")
            element.textContent = DEFAULT_FONT_TEXT
            element.style.whiteSpace = "nowrap"
            when (key) {
                "apple" -> element.style.font = "-apple-system-body"
                "serif" -> element.style.fontFamily = "serif"
                "sans" -> element.style.fontFamily = "sans-serif"
                "mono" -> element.style.fontFamily = "monospace"
                "min" -> element.style.fontSize = "1px"
                "system" -> element.style.fontFamily = "system-ui"
            }
            jsSet(elements, key, element)
            container.append(document.createElement("br"), element)
        }
        val shouldNormalizeMeasurement = isChromium() && isChromium128OrNewer()
        for (key in keys) {
            val rawWidth = jsNumber(jsGet(elements, key).getBoundingClientRect().width)
            jsSet(
                sizes,
                key,
                if (shouldNormalizeMeasurement) {
                    normalizeFontMeasurement(rawWidth * jsNumber(iframeWindow.devicePixelRatio))
                } else {
                    rawWidth
                },
            )
        }
        sizes
    })
}

private fun normalizeFontMeasurement(value: Double): Double {
    val decimals = if (isAndroid()) 0 else 3
    val pow = jsMathPow(10.0, decimals.toDouble())
    return js("Math.floor(value * pow)") as Double / pow
}

private fun withNaturalFonts(action: (dynamic, dynamic, dynamic) -> dynamic, containerWidthPx: Int = 4000): dynamic {
    return withIframe({ _: dynamic, iframeWindow: dynamic ->
        val iframeDocument = iframeWindow.document
        val iframeBody = iframeDocument.body
        val bodyStyle = iframeBody.style
        bodyStyle.width = "${containerWidthPx}px"
        bodyStyle.webkitTextSizeAdjust = "none"
        bodyStyle.textSizeAdjust = "none"
        if (isChromium()) {
            iframeBody.style.zoom = jsString(1.0 / jsNumber(iframeWindow.devicePixelRatio))
        } else if (isWebKit()) {
            iframeBody.style.zoom = "reset"
        }
        val linesOfText = iframeDocument.createElement("div")
        val words = arr()
        val count = containerWidthPx / 20
        var i = 0
        while (i < count) {
            jsPush(words, "word")
            i += 1
        }
        linesOfText.textContent = jsJoin(words, " ")
        iframeBody.appendChild(linesOfText)
        action(iframeDocument, iframeBody, iframeWindow)
    }, "<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">", jsUndefined)
}
