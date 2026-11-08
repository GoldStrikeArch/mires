internal fun getOsCpu(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic = jsNavigator().oscpu

internal fun getLanguages(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    val n = jsNavigator()
    val result = arr()
    val language = n.language ?: n.userLanguage ?: n.browserLanguage ?: n.systemLanguage
    if (!isUndefined(language)) {
        val first = arr()
        jsPush(first, language)
        jsPush(result, first)
    }
    val languages = n.languages
    if (jsIsArray(languages)) {
        if (!(isChromium() && isChromium86OrNewer())) {
            jsPush(result, languages)
        }
    } else if (jsTypeof(languages) == "string") {
        if (isTruthy(languages)) {
            jsPush(result, jsSplit(languages, ","))
        }
    }
    return result
}

internal fun getColorDepth(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic = jsScreen().colorDepth

internal fun getDeviceMemory(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    return replaceNaN(toFloat(jsNavigator().deviceMemory), jsUndefined)
}

internal fun getUnstableScreenResolution(): dynamic {
    val s = jsScreen()
    val dimensions = arr()
    jsPush(dimensions, replaceNaN(toInt(s.width), jsNull))
    jsPush(dimensions, replaceNaN(toInt(s.height), jsNull))
    jsSort(dimensions)
    jsReverse(dimensions)
    return dimensions
}

internal fun getScreenResolution(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    if (isWebKit() && isWebKit616OrNewer() && isSafariWebKit()) {
        return jsUndefined
    }
    return getUnstableScreenResolution()
}

internal fun getUnstableHardwareConcurrency(): dynamic {
    return replaceNaN(toInt(jsNavigator().hardwareConcurrency), jsUndefined)
}

internal fun getHardwareConcurrency(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    val value = getUnstableHardwareConcurrency()
    if (!isUndefined(value) && isGecko() && isGecko143OrNewer()) {
        return if (jsNumber(value) >= 8.0) 8 else 4
    }
    return value
}

internal fun getTimezone(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    val dateTimeFormat = jsWindow().Intl?.DateTimeFormat
    if (isTruthy(dateTimeFormat)) {
        val timezone = jsNew(dateTimeFormat).resolvedOptions().timeZone
        if (isTruthy(timezone)) {
            return timezone
        }
    }
    val currentYear = jsNewDate().getFullYear()
    val offset = -jsMathMax(
        jsNumber(toFloat(jsNewDateYMD(currentYear, 0, 1).getTimezoneOffset())),
        jsNumber(toFloat(jsNewDateYMD(currentYear, 6, 1).getTimezoneOffset())),
    )
    return "UTC${if (offset >= 0) "+" else ""}$offset"
}

private fun storagePresent(name: String): dynamic {
    val result = jsTry { jsGet(jsWindow(), name) }
    return if (jsTryOk(result)) isTruthy(result.value) else true
}

internal fun getSessionStorage(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic =
    storagePresent("sessionStorage")

internal fun getLocalStorage(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic =
    storagePresent("localStorage")

internal fun getIndexedDB(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    if (isTrident() || isEdgeHTML()) return jsUndefined
    return storagePresent("indexedDB")
}

internal fun getOpenDatabase(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic =
    isTruthy(jsWindow().openDatabase)

internal fun getCpuClass(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic = jsNavigator().cpuClass

internal fun getPlatform(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    val platform = jsNavigator().platform
    if (platform == "MacIntel" && isWebKit() && !isDesktopWebKit()) {
        return if (isIPad()) "iPad" else "iPhone"
    }
    return platform
}

internal fun getPlugins(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    val rawPlugins = jsNavigator().plugins
    if (isMissing(rawPlugins)) return jsUndefined
    val plugins = arr()
    val pluginCount = jsLength(rawPlugins)
    var i = 0
    while (i < pluginCount) {
        val plugin = jsIndex(rawPlugins, i)
        if (isTruthy(plugin)) {
            val mimeTypes = arr()
            val mimeCount = jsLength(plugin)
            var j = 0
            while (j < mimeCount) {
                val mimeType = jsIndex(plugin, j)
                val mime = obj()
                jsSet(mime, "type", mimeType.type)
                jsSet(mime, "suffixes", mimeType.suffixes)
                jsPush(mimeTypes, mime)
                j += 1
            }
            val item = obj()
            jsSet(item, "name", plugin.name)
            jsSet(item, "description", plugin.description)
            jsSet(item, "mimeTypes", mimeTypes)
            jsPush(plugins, item)
        }
        i += 1
    }
    return plugins
}

internal fun getTouchSupport(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    val n = jsNavigator()
    val maxTouchPoints = when {
        !isUndefined(n.maxTouchPoints) -> toInt(n.maxTouchPoints)
        !isUndefined(n.msMaxTouchPoints) -> n.msMaxTouchPoints
        else -> 0
    }
    val created = jsTry { jsDocument().createEvent("TouchEvent") }
    val touchEvent = jsTryOk(created)
    val result = obj()
    jsSet(result, "maxTouchPoints", maxTouchPoints)
    jsSet(result, "touchEvent", touchEvent)
    jsSet(result, "touchStart", jsIn("ontouchstart", jsWindow()))
    return result
}

internal fun getVendor(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    val vendor = jsNavigator().vendor
    return if (isTruthy(vendor)) vendor else ""
}

internal fun getVendorFlavors(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    val flavors = arr()
    val keys = arrayOf(
        "chrome",
        "safari",
        "__crWeb",
        "__gCrWeb",
        "yandex",
        "__yb",
        "__ybro",
        "__firefox__",
        "__edgeTrackingPreventionStatistics",
        "webkit",
        "oprt",
        "samsungAr",
        "ucweb",
        "UCShellJava",
        "puffinDevice",
    )
    val w = jsWindow()
    for (key in keys) {
        val value = jsGet(w, key)
        if (isTruthy(value) && jsTypeof(value) == "object") {
            jsPush(flavors, key)
        }
    }
    return jsSort(flavors)
}

internal fun areCookiesEnabled(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    val d = jsDocument()
    val result = jsTry {
        d.cookie = "cookietest=1; SameSite=Strict;"
        val found = jsString(d.cookie).indexOf("cookietest=") != -1
        d.cookie = "cookietest=1; SameSite=Strict; expires=Thu, 01-Jan-1970 00:00:01 GMT"
        found
    }
    return if (jsTryOk(result)) result.value else false
}

private fun mediaMatches(query: String): Boolean {
    return isTruthy(jsWindow().matchMedia(query).matches)
}

private fun boolOrUndefined(yes: String, no: String): dynamic {
    return when {
        mediaMatches(yes) -> true
        mediaMatches(no) -> false
        else -> jsUndefined
    }
}

internal fun getColorGamut(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    for (gamut in arrayOf("rec2020", "p3", "srgb")) {
        if (mediaMatches("(color-gamut: $gamut)")) return gamut
    }
    return jsUndefined
}

internal fun areColorsInverted(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic =
    boolOrUndefined("(inverted-colors: inverted)", "(inverted-colors: none)")

internal fun areColorsForced(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic =
    boolOrUndefined("(forced-colors: active)", "(forced-colors: none)")

internal fun getMonochromeDepth(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    if (!mediaMatches("(min-monochrome: 0)")) return jsUndefined
    var i = 0
    while (i <= 100) {
        if (mediaMatches("(max-monochrome: $i)")) return i
        i += 1
    }
    jsThrow(jsError("Too high value"))
}

internal fun getContrastPreference(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    return when {
        mediaMatches("(prefers-contrast: no-preference)") -> 0
        mediaMatches("(prefers-contrast: high)") || mediaMatches("(prefers-contrast: more)") -> 1
        mediaMatches("(prefers-contrast: low)") || mediaMatches("(prefers-contrast: less)") -> -1
        mediaMatches("(prefers-contrast: forced)") -> 10
        else -> jsUndefined
    }
}

internal fun isMotionReduced(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic =
    boolOrUndefined("(prefers-reduced-motion: reduce)", "(prefers-reduced-motion: no-preference)")

internal fun isTransparencyReduced(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic =
    boolOrUndefined("(prefers-reduced-transparency: reduce)", "(prefers-reduced-transparency: no-preference)")

internal fun isHDR(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic =
    boolOrUndefined("(dynamic-range: high)", "(dynamic-range: standard)")

internal fun getMathFingerprint(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    fun native(name: String, value: Double): Double = jsMathCall(name, value)
    val log = { value: Double -> jsMathCall("log", value) }
    val sqrt = { value: Double -> jsMathCall("sqrt", value) }
    val exp = { value: Double -> jsMathCall("exp", value) }
    val result = obj()
    jsSet(result, "acos", native("acos", 0.12312423423423424))
    jsSet(result, "acosh", native("acosh", 1e308))
    jsSet(result, "acoshPf", log(1e154 + sqrt(1e154 * 1e154 - 1.0)))
    jsSet(result, "asin", native("asin", 0.12312423423423424))
    jsSet(result, "asinh", native("asinh", 1.0))
    jsSet(result, "asinhPf", log(1.0 + sqrt(1.0 * 1.0 + 1.0)))
    jsSet(result, "atanh", native("atanh", 0.5))
    jsSet(result, "atanhPf", log((1.0 + 0.5) / (1.0 - 0.5)) / 2.0)
    jsSet(result, "atan", native("atan", 0.5))
    jsSet(result, "sin", native("sin", -1e300))
    jsSet(result, "sinh", native("sinh", 1.0))
    jsSet(result, "sinhPf", exp(1.0) - 1.0 / exp(1.0) / 2.0)
    jsSet(result, "cos", native("cos", 10.000000000123))
    jsSet(result, "cosh", native("cosh", 1.0))
    jsSet(result, "coshPf", (exp(1.0) + 1.0 / exp(1.0)) / 2.0)
    jsSet(result, "tan", native("tan", -1e300))
    jsSet(result, "tanh", native("tanh", 1.0))
    jsSet(result, "tanhPf", (exp(2.0) - 1.0) / (exp(2.0) + 1.0))
    jsSet(result, "exp", native("exp", 1.0))
    jsSet(result, "expm1", native("expm1", 1.0))
    jsSet(result, "expm1Pf", exp(1.0) - 1.0)
    jsSet(result, "log1p", native("log1p", 10.0))
    jsSet(result, "log1pPf", log(1.0 + 10.0))
    jsSet(result, "powPI", jsMathPow(jsPi(), -100.0))
    return result
}

internal fun isPdfViewerEnabled(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic =
    jsNavigator().pdfViewerEnabled

internal fun getArchitecture(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    val f = jsNewFloat32Array(1)
    val u8 = jsNewUint8Array(f.buffer)
    jsSetIndex(f, 0, js("Infinity"))
    val first = jsIndex(f, 0)
    jsSetIndex(f, 0, js("first - first"))
    return jsIndex(u8, 3)
}

internal fun getApplePayState(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    val applePaySession = jsWindow().ApplePaySession
    if (jsTypeof(applePaySession?.canMakePayments) != "function") {
        return -1
    }
    if (isAnyParentCrossOrigin()) {
        return -3
    }
    val result = jsTry { applePaySession.canMakePayments() }
    if (jsTryOk(result)) {
        return if (isTruthy(result.value)) 1 else 0
    }
    val error = result.error
    if (jsIsError(error) && error.name == "InvalidAccessError" &&
        jsRegexpTest("\\bfrom\\b.*\\binsecure\\b", "i", error.message)
    ) {
        return -2
    }
    jsThrow(error)
}

internal fun getPrivateClickMeasurement(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    val link = jsDocument().createElement("a")
    val sourceId = if (!isUndefined(link.attributionSourceId)) {
        link.attributionSourceId
    } else {
        link.attributionsourceid
    }
    return if (isUndefined(sourceId)) jsUndefined else jsString(sourceId)
}

internal fun getAudioContextBaseLatency(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    if (!isAndroid() && !isWebKit()) return -2
    val ctor = jsWindow().AudioContext
    if (!isTruthy(ctor)) return -1
    val latency = jsNew(ctor).baseLatency
    if (isMissing(latency)) return -1
    if (!jsIsFiniteNumber(latency)) return -3
    return latency
}

internal fun getDateTimeLocale(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    val intl = jsWindow().Intl
    if (!isTruthy(intl)) return -1
    val dateTimeFormat = intl.DateTimeFormat
    if (!isTruthy(dateTimeFormat)) return -2
    val locale = dateTimeFormat().resolvedOptions().locale
    if (!isTruthy(locale) && locale != "") return -3
    return locale
}

internal fun getUserAgentData(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    val uaData = jsNavigator().userAgentData
    if (!isTruthy(uaData)) return jsPromiseResolve(jsUndefined)
    val brandsRaw = uaData.brands
    val filtered = arr()
    val brandCount = jsLength(brandsRaw)
    var i = 0
    while (i < brandCount) {
        val entry = jsIndex(brandsRaw, i)
        if (!jsRegexpTest("not", "i", entry.brand)) {
            jsPush(filtered, entry.brand)
        }
        i += 1
    }
    val brands = if (jsLength(filtered) > 1) {
        val withoutChromium = arr()
        var j = 0
        while (j < jsLength(filtered)) {
            val brand = jsIndex(filtered, j)
            if (brand != "Chromium") jsPush(withoutChromium, brand)
            j += 1
        }
        withoutChromium
    } else {
        filtered
    }
    val result = obj()
    jsSet(result, "brands", brands)
    jsSet(result, "mobile", uaData.mobile)
    jsSet(result, "platform", uaData.platform)
    if (!isTruthy(uaData.getHighEntropyValues)) {
        return jsPromiseResolve(result)
    }
    return jsPromise { resolve, reject ->
        val hints = arr()
        jsPush(hints, "architecture")
        jsPush(hints, "bitness")
        jsPush(hints, "model")
        jsPush(hints, "platformVersion")
        val requested = jsTry { uaData.getHighEntropyValues(hints) }
        if (!jsTryOk(requested)) {
            handleHighEntropyError(requested.error, result, resolve, reject)
            return@jsPromise
        }
        val value = requested.value
        if (isPromise(value)) {
            value.then({ highEntropy: dynamic ->
                jsSet(result, "architecture", highEntropy.architecture)
                jsSet(result, "bitness", highEntropy.bitness)
                jsSet(result, "model", highEntropy.model)
                jsSet(result, "platformVersion", highEntropy.platformVersion)
                resolve(result)
            }, { error: dynamic ->
                handleHighEntropyError(error, result, resolve, reject)
            })
        } else {
            resolve(result)
        }
    }
}

private fun handleHighEntropyError(error: dynamic, result: dynamic, resolve: dynamic, reject: dynamic) {
    val isNotAllowed = js("typeof DOMException !== 'undefined' && error instanceof DOMException") as Boolean &&
        error.name == "NotAllowedError"
    if (isNotAllowed) {
        jsSet(result, "highEntropyStatus", "not_allowed")
        resolve(result)
    } else {
        reject(error)
    }
}
