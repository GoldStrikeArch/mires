/**
 * Typed-looking wrappers over the few JS primitives that have no Kotlin/JS binding
 * in the toolchain's js/app stdlib. Every `js()` call in this port lives here.
 */

internal val jsUndefined: dynamic = js("undefined")
internal val jsNull: dynamic = js("null")

internal fun obj(): dynamic = js("({})")

internal fun arr(): dynamic = js("[]")

internal fun jsGlobal(): dynamic = js("globalThis")

internal fun jsWindow(): dynamic = js("window")

internal fun jsDocument(): dynamic = js("document")

internal fun jsNavigator(): dynamic = js("navigator")

internal fun jsScreen(): dynamic = js("screen")

internal fun jsMath(): dynamic = js("Math")

internal fun isUndefined(value: dynamic): Boolean = js("value === undefined") as Boolean

internal fun isNull(value: dynamic): Boolean = js("value === null") as Boolean

internal fun isMissing(value: dynamic): Boolean = js("value === undefined || value === null") as Boolean

internal fun isTruthy(value: dynamic): Boolean = js("!!value") as Boolean

internal fun jsTypeof(value: dynamic): String = js("typeof value") as String

internal fun jsHas(holder: dynamic, name: String): Boolean = js("name in holder") as Boolean

internal fun jsIn(name: String, holder: dynamic): Boolean = js("name in holder") as Boolean

internal fun jsGet(holder: dynamic, name: String): dynamic = js("holder[name]")

internal fun jsSet(holder: dynamic, name: String, value: dynamic) {
    js("holder[name] = value")
}

internal fun jsIndex(holder: dynamic, index: Int): dynamic = js("holder[index]")

internal fun jsSetIndex(holder: dynamic, index: Int, value: dynamic) {
    js("holder[index] = value")
}

internal fun jsKeys(holder: dynamic): dynamic = js("Object.keys(holder)")

internal fun jsAssign(target: dynamic, source: dynamic): dynamic = js("Object.assign(target, source)")

internal fun jsDefineProperty(holder: dynamic, name: String, descriptor: dynamic): dynamic =
    js("Object.defineProperty(holder, name, descriptor)")

internal fun jsIsArray(value: dynamic): Boolean = js("Array.isArray(value)") as Boolean

internal fun jsIsNaN(value: dynamic): Boolean = js("typeof value === 'number' && isNaN(value)") as Boolean

internal fun jsIsFiniteNumber(value: dynamic): Boolean = js("typeof value === 'number' && isFinite(value)") as Boolean

internal fun jsParseInt(value: dynamic): dynamic = js("parseInt(value)")

internal fun jsParseFloat(value: dynamic): dynamic = js("parseFloat(value)")

internal fun jsString(value: dynamic): String = js("String(value)") as String

internal fun jsNumber(value: dynamic): Double = js("+value") as Double

internal fun jsDateNow(): Double = js("Date.now()") as Double

internal fun jsNewDate(): dynamic = js("new Date()")

internal fun jsNewDateYMD(year: dynamic, month: Int, day: Int): dynamic = js("new Date(year, month, day)")

internal fun jsMathRound(value: Double): Double = js("Math.round(value)") as Double

internal fun jsMathAbs(value: Double): Double = js("Math.abs(value)") as Double

internal fun jsMathMax(a: Double, b: Double): Double = js("Math.max(a, b)") as Double

internal fun jsMathMin(a: Double, b: Double): Double = js("Math.min(a, b)") as Double

internal fun jsMathPow(base: Double, exp: Double): Double = js("Math.pow(base, exp)") as Double

internal fun jsMathCall(name: String, value: Double): Double {
    val math = jsMath()
    val fn = jsGet(math, name)
    return if (jsTypeof(fn) == "function") {
        js("fn.call(math, value)") as Double
    } else {
        0.0
    }
}

internal fun jsPi(): Double = js("Math.PI") as Double

internal fun jsJsonStringify(value: dynamic): dynamic = js("JSON.stringify(value)")

internal fun jsJsonStringifyPretty(value: dynamic, replacer: dynamic, space: Int): dynamic =
    js("JSON.stringify(value, replacer, space)")

internal fun jsAtob(value: String): String = js("atob(value)") as String

internal fun jsPromise(executor: (dynamic, dynamic) -> Unit): dynamic = js("new Promise(executor)")

internal fun jsPromiseResolve(value: dynamic): dynamic = js("Promise.resolve(value)")

internal fun jsPromiseAll(values: dynamic): dynamic = js("Promise.all(values)")

internal fun jsSetTimeout(fn: () -> Unit, ms: Double): dynamic = js("setTimeout(fn, ms)")

internal fun jsClearTimeout(id: dynamic) {
    js("clearTimeout(id)")
}

internal fun jsNewMessageChannel(): dynamic = js("new MessageChannel()")

internal fun jsNewTextEncoder(): dynamic = js("new TextEncoder()")

internal fun jsNewFloat32Array(length: Int): dynamic = js("new Float32Array(length)")

internal fun jsNewUint8Array(buffer: dynamic): dynamic = js("new Uint8Array(buffer)")

internal fun jsFromCharCode(a: Int, b: Int): String = js("String.fromCharCode(a, b)") as String

internal fun jsBind(fn: dynamic, thisArg: dynamic, arg: dynamic): dynamic = js("fn.bind(thisArg, arg)")

internal fun jsCall0(fn: dynamic, thisArg: dynamic): dynamic = js("fn.call(thisArg)")

internal fun jsCall1(fn: dynamic, thisArg: dynamic, arg: dynamic): dynamic = js("fn.call(thisArg, arg)")

internal fun jsApply(fn: dynamic, thisArg: dynamic, args: dynamic): dynamic = js("fn.apply(thisArg, args)")

internal fun jsNew(ctor: dynamic): dynamic = js("new ctor()")

internal fun jsNew3(ctor: dynamic, a: dynamic, b: dynamic, c: dynamic): dynamic = js("new ctor(a, b, c)")

internal fun jsInstanceof(value: dynamic, ctor: dynamic): Boolean = js("value instanceof ctor") as Boolean

internal fun jsIsError(value: dynamic): Boolean = js("value instanceof Error") as Boolean

internal fun jsError(message: String): dynamic = js("new Error(message)")

internal fun jsThrow(error: dynamic): Nothing {
    js("throw error")
    throw RuntimeException("unreachable")
}

internal fun jsTry(fn: () -> dynamic): dynamic =
    js("(function(){try{return{ok:true,value:fn()}}catch(e){return{ok:false,error:e}}})()")

internal fun jsTryOk(result: dynamic): Boolean = js("!!result.ok") as Boolean

internal fun jsRegexpTest(source: String, flags: String, value: dynamic): Boolean {
    return js("(new RegExp(source, flags)).test(value)") as Boolean
}

internal fun jsReplace(value: dynamic, source: String, flags: String, replacement: String): String {
    return js("String(value).replace(new RegExp(source, flags), replacement)") as String
}

internal fun jsSplit(value: dynamic, separator: String): dynamic = js("String(value).split(separator)")

internal fun jsJoin(value: dynamic, separator: String): String = js("value.join(separator)") as String

internal fun jsPush(target: dynamic, value: dynamic) {
    js("target.push(value)")
}

internal fun jsSort(target: dynamic): dynamic = js("target.sort()")

internal fun jsReverse(target: dynamic): dynamic = js("target.reverse()")

internal fun jsSlice(target: dynamic, start: Int): dynamic = js("target.slice(start)")

internal fun jsConcat(target: dynamic, other: dynamic): dynamic = js("target.concat(other)")

internal fun jsArrayFrom(value: dynamic): dynamic = js("Array.from(value)")

internal fun jsLength(value: dynamic): Int = js("value.length") as Int

internal fun jsProto(value: dynamic): dynamic = js("value.__proto__")

internal fun jsConsoleLog(message: dynamic) {
    js("console.log(message)")
}

internal fun jsConsoleError(message: dynamic) {
    js("console.error(message)")
}

internal fun jsObjectCreate(proto: dynamic): dynamic = js("Object.create(proto)")

internal fun installGlobalExports(exports: dynamic) {
    js("globalThis.__fp_kotlin_exports = exports")
}

internal fun namedFunction(name: String, fn: dynamic): dynamic {
    jsDefineProperty(fn, "name", js("{ value: name, configurable: true }"))
    return fn
}

internal fun source0(name: String, impl: dynamic): dynamic {
    val fn = js("function(){ return impl(arguments[0]) }")
    return namedFunction(name, fn)
}

internal fun source1(name: String, impl: dynamic): dynamic {
    val fn = js("function(options){ return impl(options) }")
    return namedFunction(name, fn)
}
