private const val AUDIO_KNOWN_SUSPENDING = -1
private const val AUDIO_NOT_SUPPORTED = -2
private const val AUDIO_TIMEOUT = -3
private const val AUDIO_ANTIFINGERPRINTING = -4

internal fun getAudioFingerprint(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    if (doesBrowserPerformAudioAntifingerprinting()) {
        return AUDIO_ANTIFINGERPRINTING
    }
    return getUnstableAudioFingerprint()
}

internal fun getUnstableAudioFingerprint(): dynamic {
    val w = jsWindow()
    val AudioContext = w.OfflineAudioContext ?: w.webkitOfflineAudioContext
    if (!isTruthy(AudioContext)) {
        return AUDIO_NOT_SUPPORTED
    }
    if (doesBrowserSuspendAudioContext()) {
        return AUDIO_KNOWN_SUSPENDING
    }

    val hashFromIndex = 4500
    val hashToIndex = 5000
    val context = jsNew3(AudioContext, 1, hashToIndex, 44100)
    val oscillator = context.createOscillator()
    oscillator.type = "triangle"
    oscillator.frequency.value = 10000
    val compressor = context.createDynamicsCompressor()
    compressor.threshold.value = -50
    compressor.knee.value = 40
    compressor.ratio.value = 12
    compressor.attack.value = 0
    compressor.release.value = 0.25
    oscillator.connect(compressor)
    compressor.connect(context.destination)
    oscillator.start(0)

    val rendering = startRenderingAudio(context)
    val renderPromise = jsIndex(rendering, 0)
    val finishRendering = jsIndex(rendering, 1)
    val fingerprintPromise = suppressUnhandledRejectionWarning(
        renderPromise.then(
            { buffer: dynamic ->
                audioHash(buffer.getChannelData(0).subarray(hashFromIndex))
            },
            { error: dynamic ->
                if (error.name == "timeout" || error.name == "suspended") {
                    AUDIO_TIMEOUT
                } else {
                    jsThrow(error)
                }
            },
        ),
    )
    return {
        jsCall0(finishRendering, jsUndefined)
        fingerprintPromise
    }
}

private fun doesBrowserSuspendAudioContext(): Boolean {
    return isWebKit() && !isDesktopWebKit() && !isWebKit606OrNewer()
}

private fun doesBrowserPerformAudioAntifingerprinting(): Boolean {
    return (isWebKit() && isWebKit616OrNewer() && isSafariWebKit()) ||
        (isChromium() && isSamsungInternet() && isChromium122OrNewer())
}

private fun startRenderingAudio(context: dynamic): dynamic {
    val renderTryMaxCount = 3
    val renderRetryDelay = 500.0
    val runningMaxAwaitTime = 500.0
    val runningSufficientTime = 5000.0
    var finalize: () -> Unit = {}

    val resultPromise = jsPromise { resolve, reject ->
        var isFinalized = false
        var renderTryCount = 0
        var startedRunningAt = 0.0

        context.oncomplete = { event: dynamic -> resolve(event.renderedBuffer) }

        val startRunningTimeout = {
            jsSetTimeout({
                reject(makeInnerError("timeout"))
            }, jsMathMin(runningMaxAwaitTime, startedRunningAt + runningSufficientTime - jsDateNow()))
        }

        fun tryRender() {
            val attempt = jsTry {
                val renderingPromise = context.startRendering()
                if (isPromise(renderingPromise)) {
                    suppressUnhandledRejectionWarning(renderingPromise)
                }
                when (jsString(context.state)) {
                    "running" -> {
                        startedRunningAt = jsDateNow()
                        if (isFinalized) startRunningTimeout()
                    }
                    "suspended" -> {
                        if (!isTruthy(jsDocument().hidden)) {
                            renderTryCount += 1
                        }
                        if (isFinalized && renderTryCount >= renderTryMaxCount) {
                            reject(makeInnerError("suspended"))
                        } else {
                            jsSetTimeout({ tryRender() }, renderRetryDelay)
                        }
                    }
                }
                jsUndefined
            }
            if (!jsTryOk(attempt)) {
                reject(attempt.error)
            }
        }

        tryRender()
        finalize = {
            if (!isFinalized) {
                isFinalized = true
                if (startedRunningAt > 0) {
                    startRunningTimeout()
                }
            }
        }
    }

    val pair = arr()
    jsPush(pair, resultPromise)
    jsPush(pair, { finalize() })
    return pair
}

private fun audioHash(signal: dynamic): Double {
    var hash = 0.0
    val length = jsLength(signal)
    var i = 0
    while (i < length) {
        hash += jsMathAbs(jsNumber(jsIndex(signal, i)))
        i += 1
    }
    return hash
}

private fun makeInnerError(name: String): dynamic {
    val error = jsError(name)
    error.name = name
    return error
}
