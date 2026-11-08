private val validContextParameters = hashSetOf(
    10752, 2849, 2884, 2885, 2886, 2928, 2929, 2930, 2931, 2932, 2960, 2961, 2962, 2963, 2964, 2965, 2966, 2967, 2968,
    2978, 3024, 3042, 3088, 3089, 3106, 3107, 32773, 32777, 32823, 32824, 32936, 32937, 32938, 32939, 32968, 32969,
    32970, 32971, 3317, 33170, 3333, 3379, 3386, 33901, 33902, 34016, 34024, 34076, 3408, 3410, 3411, 3412, 3413, 3414,
    3415, 34467, 34816, 34817, 34818, 34819, 34877, 34921, 34930, 35660, 35661, 35724, 35738, 35739, 36003, 36004, 36005,
    36347, 36348, 36349, 37440, 37441, 37443, 7936, 7937, 7938,
)

private val validExtensionParams = hashSetOf(
    34047, 35723, 36063, 34852, 34853, 34854, 34229, 36392, 36795, 38449,
)

private val shaderTypes = arrayOf("FRAGMENT_SHADER", "VERTEX_SHADER")
private val precisionTypes = arrayOf("LOW_FLOAT", "MEDIUM_FLOAT", "HIGH_FLOAT", "LOW_INT", "MEDIUM_INT", "HIGH_INT")
private const val RENDERER_INFO_EXTENSION = "WEBGL_debug_renderer_info"
private const val POLYGON_MODE_EXTENSION = "WEBGL_polygon_mode"
private const val STATUS_NO_GL_CONTEXT = -1
private const val STATUS_GET_PARAMETER_NOT_A_FUNCTION = -2

internal fun getWebGlBasics(options: dynamic): dynamic {
    val cache = options?.cache ?: obj()
    val gl = getWebGLContextImpl(cache)
    if (!isTruthy(gl)) return STATUS_NO_GL_CONTEXT
    if (!isValidParameterGetter(gl)) return STATUS_GET_PARAMETER_NOT_A_FUNCTION
    val debugExtension = if (shouldAvoidDebugRendererInfo()) jsNull else gl.getExtension(RENDERER_INFO_EXTENSION)
    val result = obj()
    jsSet(result, "version", parameterString(gl, gl.VERSION))
    jsSet(result, "vendor", parameterString(gl, gl.VENDOR))
    jsSet(
        result,
        "vendorUnmasked",
        if (isTruthy(debugExtension)) parameterString(gl, debugExtension.UNMASKED_VENDOR_WEBGL) else "",
    )
    jsSet(result, "renderer", parameterString(gl, gl.RENDERER))
    jsSet(
        result,
        "rendererUnmasked",
        if (isTruthy(debugExtension)) parameterString(gl, debugExtension.UNMASKED_RENDERER_WEBGL) else "",
    )
    jsSet(result, "shadingLanguageVersion", parameterString(gl, gl.SHADING_LANGUAGE_VERSION))
    return result
}

internal fun getWebGlExtensions(options: dynamic): dynamic {
    val cache = options?.cache ?: obj()
    val gl = getWebGLContextImpl(cache)
    if (!isTruthy(gl)) return STATUS_NO_GL_CONTEXT
    if (!isValidParameterGetter(gl)) return STATUS_GET_PARAMETER_NOT_A_FUNCTION

    val extensions = gl.getSupportedExtensions()
    val contextAttributes = gl.getContextAttributes()
    val unsupportedExtensions = arr()
    val attributes = arr()
    val parameters = arr()
    val extensionParameters = arr()
    val shaderPrecisions = arr()

    if (isTruthy(contextAttributes)) {
        val names = jsKeys(contextAttributes)
        var i = 0
        while (i < jsLength(names)) {
            val attributeName = jsString(jsIndex(names, i))
            jsPush(attributes, "$attributeName=${jsGet(contextAttributes, attributeName)}")
            i += 1
        }
    }

    val constants = getConstantsFromPrototype(gl)
    var c = 0
    while (c < jsLength(constants)) {
        val constant = jsString(jsIndex(constants, c))
        val code = jsGet(gl, constant)
        val extra = if (code is Int && validContextParameters.contains(code) ||
            (jsTypeof(code) == "number" && validContextParameters.contains(jsNumber(code).toInt()))
        ) {
            "=${gl.getParameter(code)}"
        } else {
            ""
        }
        jsPush(parameters, "$constant=$code$extra")
        c += 1
    }

    if (isTruthy(extensions)) {
        var e = 0
        while (e < jsLength(extensions)) {
            val name = jsString(jsIndex(extensions, e))
            if ((name == RENDERER_INFO_EXTENSION && shouldAvoidDebugRendererInfo()) ||
                (name == POLYGON_MODE_EXTENSION && shouldAvoidPolygonModeExtensions())
            ) {
                e += 1
                continue
            }
            val extension = gl.getExtension(name)
            if (!isTruthy(extension)) {
                jsPush(unsupportedExtensions, name)
                e += 1
                continue
            }
            val extConstants = getConstantsFromPrototype(extension)
            var k = 0
            while (k < jsLength(extConstants)) {
                val constant = jsString(jsIndex(extConstants, k))
                val code = jsGet(extension, constant)
                val extra = if (jsTypeof(code) == "number" && validExtensionParams.contains(jsNumber(code).toInt())) {
                    "=${gl.getParameter(code)}"
                } else {
                    ""
                }
                jsPush(extensionParameters, "$constant=$code$extra")
                k += 1
            }
            e += 1
        }
    }

    for (shaderType in shaderTypes) {
        for (precisionType in precisionTypes) {
            val precision = getShaderPrecision(gl, shaderType, precisionType)
            jsPush(shaderPrecisions, "$shaderType.$precisionType=${jsJoin(precision, ",")}")
        }
    }

    jsSort(extensionParameters)
    jsSort(parameters)

    val result = obj()
    jsSet(result, "contextAttributes", attributes)
    jsSet(result, "parameters", parameters)
    jsSet(result, "shaderPrecisions", shaderPrecisions)
    jsSet(result, "extensions", extensions)
    jsSet(result, "extensionParameters", extensionParameters)
    jsSet(result, "unsupportedExtensions", unsupportedExtensions)
    return result
}

internal fun getWebGLContext(cache: dynamic): dynamic {
    val holder = if (isMissing(cache)) obj() else cache
    return getWebGLContextImpl(holder)
}

internal fun getWebGLContextImpl(cache: dynamic): dynamic {
    if (isTruthy(cache.webgl)) {
        return cache.webgl.context
    }
    val canvas = jsDocument().createElement("canvas")
    var context: dynamic = jsUndefined
    canvas.addEventListener("webglCreateContextError", { context = jsUndefined })
    for (type in arrayOf("webgl", "experimental-webgl")) {
        val created = jsTry { canvas.getContext(type) }
        if (jsTryOk(created) && isTruthy(created.value)) {
            context = created.value
            break
        }
    }
    val entry = obj()
    jsSet(entry, "context", context)
    jsSet(cache, "webgl", entry)
    return context
}

private fun parameterString(gl: dynamic, parameter: dynamic): String {
    val value = gl.getParameter(parameter)
    return if (isMissing(value)) "" else jsString(value.toString())
}

private fun getShaderPrecision(gl: dynamic, shaderType: String, precisionType: String): dynamic {
    val shaderPrecision = gl.getShaderPrecisionFormat(jsGet(gl, shaderType), jsGet(gl, precisionType))
    val result = arr()
    if (isTruthy(shaderPrecision)) {
        jsPush(result, shaderPrecision.rangeMin)
        jsPush(result, shaderPrecision.rangeMax)
        jsPush(result, shaderPrecision.precision)
    }
    return result
}

private fun getConstantsFromPrototype(obj: dynamic): dynamic {
    val keys = jsKeys(jsProto(obj))
    val result = arr()
    var i = 0
    while (i < jsLength(keys)) {
        val key = jsIndex(keys, i)
        if (jsTypeof(key) == "string" && !jsRegexpTest("[^A-Z0-9_x]", "", key)) {
            jsPush(result, key)
        }
        i += 1
    }
    return result
}

private fun shouldAvoidDebugRendererInfo(): Boolean = isGecko()

private fun shouldAvoidPolygonModeExtensions(): Boolean = isChromium() || isWebKit()

private fun isValidParameterGetter(gl: dynamic): Boolean = jsTypeof(gl.getParameter) == "function"
