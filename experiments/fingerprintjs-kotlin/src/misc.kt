internal fun errorToObject(error: dynamic): dynamic {
    val result = obj()
    jsSet(result, "name", error.name)
    jsSet(result, "message", error.message)
    val stack = error.stack
    if (isUndefined(stack)) {
        jsSet(result, "stack", jsUndefined)
    } else {
        jsSet(result, "stack", jsSplit(stack, "\n"))
    }
    val keys = jsKeys(error)
    val length = jsLength(keys)
    var i = 0
    while (i < length) {
        val key = jsIndex(keys, i)
        jsSet(result, jsString(key), jsGet(error, jsString(key)))
        i += 1
    }
    return result
}

internal fun isFunctionNative(func: dynamic): Boolean {
    return jsRegexpTest("^function\\s.*?\\{\\s*\\[native code]\\s*}$", "", jsString(func))
}
