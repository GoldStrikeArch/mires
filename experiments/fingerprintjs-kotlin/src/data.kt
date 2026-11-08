internal fun includes(haystack: dynamic, needle: dynamic): Boolean {
    val length = jsLength(haystack)
    var i = 0
    while (i < length) {
        if (js("haystack[i] === needle") as Boolean) {
            return true
        }
        i += 1
    }
    return false
}

internal fun excludes(haystack: dynamic, needle: dynamic): Boolean = !includes(haystack, needle)

internal fun toInt(value: dynamic): dynamic = jsParseInt(value)

internal fun toFloat(value: dynamic): dynamic = jsParseFloat(value)

internal fun replaceNaN(value: dynamic, replacement: dynamic): dynamic {
    return if (jsIsNaN(value)) replacement else value
}

internal fun countTruthy(values: Array<dynamic>): Int {
    var sum = 0
    for (value in values) {
        if (isTruthy(value)) sum += 1
    }
    return sum
}

internal fun countTruthyDynamic(values: dynamic): Int {
    var sum = 0
    val length = jsLength(values)
    var i = 0
    while (i < length) {
        if (isTruthy(jsIndex(values, i))) sum += 1
        i += 1
    }
    return sum
}

internal fun round(value: Double, base: Double = 1.0): Double {
    return if (jsMathAbs(base) >= 1.0) {
        jsMathRound(value / base) * base
    } else {
        val counterBase = 1.0 / base
        jsMathRound(value * counterBase) / counterBase
    }
}

internal fun parseSimpleCssSelector(selector: String): Pair<String?, dynamic> {
    val errorMessage = "Unexpected syntax '$selector'"
    val tagMatch = js("( /^\\s*([a-z-]*)(.*)$/i.exec(selector) )")
    val tagRaw = jsIndex(tagMatch, 1)
    val tag = if (isTruthy(tagRaw)) jsString(tagRaw) else null
    val attributes = obj()
    val partsRegex = js("/([.:#][\\w-]+|\\[.+?\\])/gi")

    fun addAttribute(name: String, value: String) {
        if (isMissing(jsGet(attributes, name))) {
            jsSet(attributes, name, arr())
        }
        jsPush(jsGet(attributes, name), value)
    }

    val rest = jsIndex(tagMatch, 2)
    while (true) {
        val match = js("partsRegex.exec(rest)")
        if (!isTruthy(match)) break
        val part = jsString(jsIndex(match, 0))
        when (part[0]) {
            '.' -> addAttribute("class", part.substring(1))
            '#' -> addAttribute("id", part.substring(1))
            '[' -> {
                val attributeMatch = js("( /^\\[([\\w-]+)([~|^$*]?=(\"(.*?)\"|([\\w-]+)))?(\\s+[is])?\\]$/.exec(part) )")
                if (isTruthy(attributeMatch)) {
                    val quoted = jsIndex(attributeMatch, 4)
                    val unquoted = jsIndex(attributeMatch, 5)
                    val value = when {
                        !isMissing(quoted) && !isUndefined(quoted) -> jsString(quoted)
                        !isMissing(unquoted) && !isUndefined(unquoted) -> jsString(unquoted)
                        else -> ""
                    }
                    addAttribute(jsString(jsIndex(attributeMatch, 1)), value)
                } else {
                    jsThrow(jsError(errorMessage))
                }
            }
            else -> jsThrow(jsError(errorMessage))
        }
    }
    return Pair(tag, attributes)
}

internal fun getUTF8Bytes(input: String): IntArray {
    val result = IntArray(input.length)
    var i = 0
    while (i < input.length) {
        val charCode = input[i].code
        if (charCode > 127) {
            val encoded = js("new TextEncoder().encode(input)")
            val length = jsLength(encoded)
            val out = IntArray(length)
            var j = 0
            while (j < length) {
                out[j] = jsIndex(encoded, j) as Int
                j += 1
            }
            return out
        }
        result[i] = charCode
        i += 1
    }
    return result
}
