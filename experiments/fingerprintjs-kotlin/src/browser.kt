internal fun isTrident(): Boolean {
    val w = jsWindow()
    val n = jsNavigator()
    return countTruthy(
        arrayOf(
            jsIn("MSCSSMatrix", w),
            jsIn("msSetImmediate", w),
            jsIn("msIndexedDB", w),
            jsIn("msMaxTouchPoints", n),
            jsIn("msPointerEnabled", n),
        ),
    ) >= 4
}

internal fun isEdgeHTML(): Boolean {
    val w = jsWindow()
    val n = jsNavigator()
    return countTruthy(
        arrayOf(
            jsIn("msWriteProfilerMark", w),
            jsIn("MSStream", w),
            jsIn("msLaunchUri", n),
            jsIn("msSaveBlob", n),
        ),
    ) >= 3 && !isTrident()
}

internal fun isChromium(): Boolean {
    val w = jsWindow()
    val n = jsNavigator()
    val vendor = n.vendor
    val vendorText = if (isMissing(vendor)) "" else jsString(vendor)
    return countTruthy(
        arrayOf(
            jsIn("webkitPersistentStorage", n),
            jsIn("webkitTemporaryStorage", n),
            vendorText.indexOf("Google") == 0,
            jsIn("webkitResolveLocalFileSystemURL", w),
            jsIn("BatteryManager", w),
            jsIn("webkitMediaStream", w),
            jsIn("webkitSpeechGrammar", w),
        ),
    ) >= 5
}

internal fun isWebKit(): Boolean {
    val w = jsWindow()
    val n = jsNavigator()
    return countTruthy(
        arrayOf(
            jsIn("ApplePayError", w),
            jsIn("CSSPrimitiveValue", w),
            jsIn("Counter", w),
            jsString(n.vendor).indexOf("Apple") == 0,
            jsIn("RGBColor", w),
            jsIn("WebKitMediaKeys", w),
        ),
    ) >= 4
}

internal fun isDesktopWebKit(): Boolean {
    val w = jsWindow()
    val htmlElement = w.HTMLElement
    val documentCtor = w.Document
    return countTruthy(
        arrayOf(
            jsIn("safari", w),
            !jsIn("ongestureend", w),
            !jsIn("TouchEvent", w),
            !jsIn("orientation", w),
            isTruthy(htmlElement) && !jsIn("autocapitalize", htmlElement.prototype),
            isTruthy(documentCtor) && jsIn("pointerLockElement", documentCtor.prototype),
        ),
    ) >= 4
}

internal fun isSafariWebKit(): Boolean {
    val w = jsWindow()
    return isFunctionNative(w.print) &&
        jsString(w.browser) == "[object WebPageNamespace]"
}

internal fun isGecko(): Boolean {
    val w = jsWindow()
    val style = js("document.documentElement ? document.documentElement.style : {}")
    return countTruthy(
        arrayOf(
            jsIn("buildID", jsNavigator()),
            jsIn("MozAppearance", style),
            jsIn("onmozfullscreenchange", w),
            jsIn("mozInnerScreenX", w),
            jsIn("CSSMozDocumentRule", w),
            jsIn("CanvasCaptureMediaStream", w),
        ),
    ) >= 4
}

internal fun isGecko120OrNewer(): Boolean {
    val w = jsWindow()
    val n = jsNavigator()
    val css = w.CSS
    return countTruthy(
        arrayOf(
            jsIn("userActivation", n),
            css.supports("color", "light-dark(#000, #fff)"),
            css.supports("height", "1lh"),
            jsIn("globalPrivacyControl", n),
        ),
    ) >= 3
}

internal fun isGecko143OrNewer(): Boolean {
    val css = jsWindow().CSS
    return countTruthy(
        arrayOf(
            css.supports("selector(::details-content)"),
            css.supports("selector(::before::marker)"),
            css.supports("selector(::after::marker)"),
            !jsIn("locale", jsWindow().CompositionEvent.prototype),
        ),
    ) >= 3
}

internal fun isChromium86OrNewer(): Boolean {
    val w = jsWindow()
    return countTruthy(
        arrayOf(
            !jsIn("MediaSettingsRange", w),
            jsIn("RTCEncodedAudioFrame", w),
            jsString(w.Intl) == "[object Intl]",
            jsString(w.Reflect) == "[object Reflect]",
        ),
    ) >= 3
}

internal fun isChromium122OrNewer(): Boolean {
    val w = jsWindow()
    val urlPattern = w.URLPattern
    return countTruthy(
        arrayOf(
            jsIn("union", js("Set.prototype")),
            jsIn("Iterator", w),
            isTruthy(urlPattern) && jsIn("hasRegExpGroups", urlPattern.prototype),
            jsIn("RGB8", js("WebGLRenderingContext.prototype")),
        ),
    ) >= 3
}

internal fun isChromium128OrNewer(): Boolean {
    val w = jsWindow()
    val d = jsDocument()
    val css = w.CSS
    val promise = w.Promise
    val audioContext = w.AudioContext
    return countTruthy(
        arrayOf(
            isTruthy(promise) && jsIn("try", promise),
            jsIn("caretPositionFromPoint", d),
            isTruthy(audioContext) && jsIn("onerror", audioContext.prototype),
            css.supports("ruby-align", "space-around"),
        ),
    ) >= 3
}

internal fun isWebKit606OrNewer(): Boolean {
    val w = jsWindow()
    return countTruthy(
        arrayOf(
            jsIn("DOMRectList", w),
            jsIn("RTCPeerConnectionIceEvent", w),
            jsIn("SVGGeometryElement", w),
            jsIn("ontransitioncancel", w),
        ),
    ) >= 3
}

internal fun isWebKit616OrNewer(): Boolean {
    val w = jsWindow()
    val n = jsNavigator()
    val css = w.CSS
    val htmlButton = w.HTMLButtonElement
    return countTruthy(
        arrayOf(
            !jsIn("getStorageUpdates", n),
            isTruthy(htmlButton) && jsIn("popover", htmlButton.prototype),
            jsIn("CSSCounterStyleRule", w),
            css.supports("font-size-adjust: ex-height 0.5"),
            css.supports("text-transform: full-width"),
        ),
    ) >= 4
}

internal fun isIPad(): Boolean {
    if (jsNavigator().platform == "iPad") {
        return true
    }
    val s = jsScreen()
    val screenRatio = jsNumber(s.width) / jsNumber(s.height)
    return countTruthy(
        arrayOf(
            jsIn("MediaSource", jsWindow()),
            isTruthy(js("Element.prototype.webkitRequestFullscreen")),
            screenRatio > 0.65 && screenRatio < 1.53,
        ),
    ) >= 2
}

internal fun getFullscreenElement(): dynamic {
    val d = jsDocument()
    return when {
        !isMissing(d.fullscreenElement) -> d.fullscreenElement
        !isMissing(d.msFullscreenElement) -> d.msFullscreenElement
        !isMissing(d.mozFullScreenElement) -> d.mozFullScreenElement
        !isMissing(d.webkitFullscreenElement) -> d.webkitFullscreenElement
        else -> jsNull
    }
}

internal fun exitFullscreen(): dynamic {
    val d = jsDocument()
    val fn = d.exitFullscreen ?: d.msExitFullscreen ?: d.mozCancelFullScreen ?: d.webkitExitFullscreen
    return jsCall0(fn, d)
}

internal fun isAndroid(): Boolean {
    val isItChromium = isChromium()
    val isItGecko = isGecko()
    val w = jsWindow()
    val n = jsNavigator()
    return when {
        isItChromium -> {
            val connection = n.connection
            countTruthy(
                arrayOf(
                    !jsIn("SharedWorker", w),
                    isTruthy(connection) && jsIn("ontypechange", connection),
                    !jsIn("sinkId", jsNew(w.Audio)),
                ),
            ) >= 2
        }
        isItGecko -> {
            countTruthy(
                arrayOf(
                    jsIn("onorientationchange", w),
                    jsIn("orientation", w),
                    jsRegexpTest("android", "i", n.appVersion),
                ),
            ) >= 2
        }
        else -> false
    }
}

internal fun isSamsungInternet(): Boolean {
    val n = jsNavigator()
    val w = jsWindow()
    val audioPrototype = w.Audio.prototype
    val visualViewport = w.visualViewport
    return countTruthy(
        arrayOf(
            jsIn("srLatency", audioPrototype),
            jsIn("srChannelCount", audioPrototype),
            jsIn("devicePosture", n),
            isTruthy(visualViewport) && jsIn("segments", visualViewport),
            jsIn("getTextInformation", w.Image.prototype),
        ),
    ) >= 3
}
