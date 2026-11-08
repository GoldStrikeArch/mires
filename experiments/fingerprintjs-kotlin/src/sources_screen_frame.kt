private const val SCREEN_FRAME_CHECK_INTERVAL = 2500.0
private const val SCREEN_FRAME_ROUNDING = 10.0

private var screenFrameBackup: dynamic = jsUndefined
private var screenFrameSizeTimeoutId: dynamic = jsUndefined

private fun watchScreenFrame() {
    if (!isUndefined(screenFrameSizeTimeoutId)) return
    fun checkScreenFrame() {
        val frameSize = getCurrentScreenFrame()
        if (isFrameSizeNull(frameSize)) {
            screenFrameSizeTimeoutId = jsSetTimeout({ checkScreenFrame() }, SCREEN_FRAME_CHECK_INTERVAL)
        } else {
            screenFrameBackup = frameSize
            screenFrameSizeTimeoutId = jsUndefined
        }
    }
    checkScreenFrame()
}

internal fun getUnstableScreenFrame(): dynamic {
    watchScreenFrame()
    return {
        var frameSize = getCurrentScreenFrame()
        fun finish(size: dynamic): dynamic {
            if (!isFrameSizeNull(size)) {
                screenFrameBackup = size
            }
            return size
        }
        when {
            !isFrameSizeNull(frameSize) -> jsPromiseResolve(finish(frameSize))
            isTruthy(screenFrameBackup) -> jsPromiseResolve(jsSlice(screenFrameBackup, 0))
            isTruthy(getFullscreenElement()) -> exitFullscreen().then({
                frameSize = getCurrentScreenFrame()
                finish(frameSize)
            })
            else -> jsPromiseResolve(finish(frameSize))
        }
    }
}

internal fun getScreenFrame(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    val isSafari17OrAbove = isWebKit() && isWebKit616OrNewer() && isSafariWebKit()
    val isFirefox143OrAbove = isGecko() && isGecko143OrNewer()
    if (isSafari17OrAbove || isFirefox143OrAbove) {
        return { jsPromiseResolve(jsUndefined) }
    }
    val screenFrameGetter = getUnstableScreenFrame()
    return {
        jsCall0(screenFrameGetter, jsUndefined).then({ frameSize: dynamic ->
            fun processSize(sideSize: dynamic): dynamic {
                return if (isNull(sideSize)) jsNull else round(jsNumber(sideSize), SCREEN_FRAME_ROUNDING)
            }
            val result = arr()
            jsPush(result, processSize(jsIndex(frameSize, 0)))
            jsPush(result, processSize(jsIndex(frameSize, 1)))
            jsPush(result, processSize(jsIndex(frameSize, 2)))
            jsPush(result, processSize(jsIndex(frameSize, 3)))
            result
        })
    }
}

private fun getCurrentScreenFrame(): dynamic {
    val s = jsScreen()
    val result = arr()
    jsPush(result, replaceNaN(toFloat(s.availTop), jsNull))
    jsPush(
        result,
        replaceNaN(
            jsNumber(toFloat(s.width)) - jsNumber(toFloat(s.availWidth)) - jsNumber(replaceNaN(toFloat(s.availLeft), 0)),
            jsNull,
        ),
    )
    jsPush(
        result,
        replaceNaN(
            jsNumber(toFloat(s.height)) - jsNumber(toFloat(s.availHeight)) - jsNumber(replaceNaN(toFloat(s.availTop), 0)),
            jsNull,
        ),
    )
    jsPush(result, replaceNaN(toFloat(s.availLeft), jsNull))
    return result
}

private fun isFrameSizeNull(frameSize: dynamic): Boolean {
    var i = 0
    while (i < 4) {
        if (isTruthy(jsIndex(frameSize, i))) return false
        i += 1
    }
    return true
}
