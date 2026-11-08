internal const val ALGORITHM_VERSION = "5.2.0"

internal fun componentsToCanonicalString(components: dynamic): String {
    val keys = jsSort(jsKeys(components))
    var result = ""
    val length = jsLength(keys)
    var i = 0
    while (i < length) {
        val componentKey = jsString(jsIndex(keys, i))
        val component = jsGet(components, componentKey)
        val value = if (jsHas(component, "error")) {
            "error"
        } else {
            val serialized = jsJsonStringify(component.value)
            if (isUndefined(serialized)) "undefined" else jsString(serialized)
        }
        val escaped = jsReplace(componentKey, "([:|\\\\])", "g", "\\$1")
        result += "${if (result.isNotEmpty()) "|" else ""}$escaped:$value"
        i += 1
    }
    return result
}

internal fun componentsToDebugString(components: dynamic): String {
    val replacer = { _: dynamic, value: dynamic ->
        if (jsIsError(value)) errorToObject(value) else value
    }
    return jsString(jsJsonStringifyPretty(components, replacer, 2))
}

internal fun hashComponents(components: dynamic): String {
    return x64hash128(componentsToCanonicalString(components))
}

internal fun makeLazyGetResult(components: dynamic): dynamic {
    var visitorIdCache: dynamic = jsUndefined
    val confidence = getConfidence(components)
    val result = obj()
    val getter = {
        if (isUndefined(visitorIdCache)) {
            visitorIdCache = hashComponents(jsGet(result, "components"))
        }
        visitorIdCache
    }
    val setter = { visitorId: dynamic ->
        visitorIdCache = visitorId
    }
    namedFunction("get visitorId", getter)
    namedFunction("set visitorId", setter)
    val descriptor = obj()
    jsSet(descriptor, "get", getter)
    jsSet(descriptor, "set", setter)
    jsSet(descriptor, "enumerable", true)
    jsSet(descriptor, "configurable", true)
    jsDefineProperty(result, "visitorId", descriptor)
    jsSet(result, "confidence", confidence)
    jsSet(result, "components", components)
    jsSet(result, "version", ALGORITHM_VERSION)
    return result
}

internal fun prepareForSources(delayFallback: dynamic): dynamic {
    val delay = if (isUndefined(delayFallback)) 50.0 else jsNumber(delayFallback)
    return requestIdleCallbackIfAvailable(delay, delay * 2)
}

internal fun makeAgent(getComponents: dynamic, debug: dynamic): dynamic {
    val creationTime = jsDateNow()
    val agent = obj()
    jsSet(agent, "get") { options: dynamic ->
        val startTime = jsDateNow()
        jsCall0(getComponents, jsUndefined).then({ components: dynamic ->
            val result = makeLazyGetResult(components)
            val optionDebug = !isMissing(options) && isTruthy(options.debug)
            if (isTruthy(debug) || optionDebug) {
                val n = jsNavigator()
                val message =
                    "Copy the text below to get the debug data:\n\n```\nversion: ${jsGet(result, "version")}\nuserAgent: ${n.userAgent}\ntimeBetweenLoadAndGet: ${startTime - creationTime}\nvisitorId: ${jsGet(result, "visitorId")}\ncomponents: ${componentsToDebugString(components)}\n```"
                jsConsoleLog(message)
            }
            result
        })
    }
    return agent
}

internal fun monitor() {
    val w = jsWindow()
    if (isTruthy(jsGet(w, "__fpjs_d_m"))) return
    if (js("Math.random()") as Double >= 0.001) return
    try {
        val request = jsNew(jsGlobal().XMLHttpRequest)
        request.open("get", "https://m1.openfpcdn.io/fingerprintjs/v$ALGORITHM_VERSION/npm-monitoring", true)
        request.send()
    } catch (error: Throwable) {
        jsConsoleError(error)
    }
}

internal fun load(options: dynamic): dynamic {
    val opts = if (isMissing(options)) obj() else options
    val delayFallback = opts.delayFallback
    val debug = opts.debug
    val monitoring = opts.monitoring
    if (isUndefined(monitoring) || isTruthy(monitoring)) {
        monitor()
    }
    return prepareForSources(delayFallback).then({
        val sourceOptions = obj()
        jsSet(sourceOptions, "cache", obj())
        jsSet(sourceOptions, "debug", debug)
        val getComponents = loadSources(builtinSources, sourceOptions, arr(), jsUndefined)
        makeAgent(getComponents, debug)
    })
}
