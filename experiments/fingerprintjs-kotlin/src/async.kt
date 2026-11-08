internal fun wait(durationMs: Double, resolveWith: dynamic = jsUndefined): dynamic {
    return jsPromise { resolve, _ ->
        jsSetTimeout({ resolve(resolveWith) }, durationMs)
    }
}

internal fun releaseEventLoop(): dynamic {
    return jsPromise { resolve, _ ->
        val channel = jsNewMessageChannel()
        channel.port1.onmessage = { resolve(jsUndefined) }
        channel.port2.postMessage(jsNull)
    }
}

internal fun requestIdleCallbackIfAvailable(fallbackTimeout: Double, deadlineTimeout: Double): dynamic {
    val w = jsWindow()
    val requestIdleCallback = w.requestIdleCallback
    return if (isTruthy(requestIdleCallback)) {
        jsPromise { resolve, _ ->
            val options = obj()
            jsSet(options, "timeout", deadlineTimeout)
            js("requestIdleCallback.call(w, function(){ resolve(void 0) }, options)")
        }
    } else {
        wait(jsMathMin(fallbackTimeout, deadlineTimeout))
    }
}

internal fun isPromise(value: dynamic): Boolean {
    return isTruthy(value) && jsTypeof(value.then) == "function"
}

internal fun awaitIfAsync(action: () -> dynamic, callback: (Boolean, dynamic) -> Unit) {
    val result = jsTry { action() }
    if (!jsTryOk(result)) {
        callback(false, result.error)
        return
    }
    val returnedValue = result.value
    if (isPromise(returnedValue)) {
        returnedValue.then(
            { value: dynamic -> callback(true, value) },
            { error: dynamic -> callback(false, error) },
        )
    } else {
        callback(true, returnedValue)
    }
}

internal fun mapWithBreaks(items: dynamic, callback: (dynamic, Int) -> dynamic, loopReleaseInterval: Double): dynamic {
    val length = jsLength(items)
    val results = arr()
    var lastLoopReleaseTime = jsDateNow()
    var i = 0

    fun step(resolve: dynamic, reject: dynamic) {
        while (i < length) {
            val outcome = jsTry { callback(jsIndex(items, i), i) }
            if (!jsTryOk(outcome)) {
                reject(outcome.error)
                return
            }
            jsSetIndex(results, i, outcome.value)
            i += 1
            val now = jsDateNow()
            if (now >= lastLoopReleaseTime + loopReleaseInterval) {
                lastLoopReleaseTime = now
                releaseEventLoop().then({
                    step(resolve, reject)
                }, reject)
                return
            }
        }
        resolve(results)
    }

    return jsPromise { resolve, reject -> step(resolve, reject) }
}

internal fun suppressUnhandledRejectionWarning(promise: dynamic): dynamic {
    promise.then(jsUndefined, { })
    return promise
}
