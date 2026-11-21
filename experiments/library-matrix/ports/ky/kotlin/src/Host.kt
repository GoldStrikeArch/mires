// The host APIs Ky drives: Fetch (Headers, Request, Response), AbortController and AbortSignal,
// TextDecoder, timers and promises, as external declarations. Ky is a thin layer over these, so
// their semantics (method and URL normalisation, header validation, body encoding, encoding
// labels) are the engine's own. A JS exception thrown by one of them is a Kotlin Throwable (a JS
// Error), and keeps its JS name.

package ky

import kotlin.coroutines.Continuation
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlin.coroutines.startCoroutine
import kotlin.coroutines.suspendCoroutine
import kotlin.js.Json
import kotlin.js.Promise
import org.khronos.webgl.ArrayBuffer

external class Headers {
    /** Validates the name and the value (a TypeError) and stores the value's ByteString conversion,
     *  so any JSON value is accepted as JS accepts it. */
    fun set(name: String, value: Any?)
    fun get(name: String): String?
}

external class Request(input: Any, init: Json) {
    val method: String
    val url: String
    val headers: Headers
    val signal: AbortSignal
    fun clone(): Request
    fun text(): Promise<String>
}

external class Response(body: Any?, init: Json) {
    val status: Int
    val ok: Boolean
    val headers: Headers
    val body: Any?
    fun text(): Promise<String>
    fun arrayBuffer(): Promise<ArrayBuffer>
}

external class AbortController {
    val signal: AbortSignal
    /** Without a reason, the platform's AbortError DOMException. */
    fun abort(reason: Any? = definedExternally)
}

external class AbortSignal {
    val aborted: Boolean
    val reason: Throwable
    fun throwIfAborted()
    fun addEventListener(type: String, listener: () -> Unit, options: Json)

    companion object {
        /** Aborted as soon as one of [signals] is, with that one's reason. */
        fun any(signals: Array<AbortSignal>): AbortSignal
    }
}

external class TextDecoder(label: String = definedExternally) {
    fun decode(input: ArrayBuffer): String
}

external fun setTimeout(handler: () -> Unit, timeout: Double): Any
external fun clearTimeout(handle: Any)

@JsName("Error")
external class JsError(message: String)

@JsName("TypeError")
external class JsTypeError(message: String)

@JsName("RangeError")
external class JsRangeError(message: String)

/** A JS `TypeError`, `RangeError` or `Error`, which Ky throws from its option checks. */
internal fun typeError(message: String): Throwable = JsTypeError(message).unsafeCast<Throwable>()
internal fun rangeError(message: String): Throwable = JsRangeError(message).unsafeCast<Throwable>()
internal fun plainError(message: String): Throwable = JsError(message).unsafeCast<Throwable>()

@JsName("Number")
internal external object JsNumber {
    fun isInteger(value: Any?): Boolean
}

@JsName("Object")
internal external object JsObject {
    fun keys(value: Any): Array<String>
}

@JsName("Reflect")
internal external object JsReflect {
    fun has(target: Any, key: String): Boolean
}

@JsName("Object")
internal external object ObjectPrototype {
    val prototype: dynamic
}

@JsName("Date")
internal external object JsDate {
    fun now(): Double
}

internal fun isUndefined(value: Any?) = jsTypeOf(value) == "undefined"

/** A JS value thrown that is no Error of this realm, as a Throwable. Kotlin/JS catches only
 *  `instanceof Error`, and a page can hand the port host objects of another realm: under Firefox's
 *  WebDriver, the Fetch constructors throw TypeErrors and RangeErrors that fail that test. */
class HostError(val value: dynamic) : Exception()

internal fun thrown(value: dynamic): Throwable = if (value is Throwable) value else HostError(value)

internal suspend fun <T> Promise<T>.await(): T = suspendCoroutine { continuation ->
    then({ value -> continuation.resume(value) }, { error -> continuation.resumeWithException(thrown(error)) })
}

/** Runs a host operation that can throw inside a JS Promise executor, whose rejection keeps the
 *  thrown value whatever its realm. */
internal suspend fun <T> hostCall(operation: () -> T): T = Promise<T> { resolve, _ -> resolve(operation()) }.await()

/** Runs [block] as a coroutine, settling the promise with its outcome. */
internal fun <T> launch(block: suspend () -> T): Promise<T> = Promise { resolve, reject ->
    block.startCoroutine(Continuation(EmptyCoroutineContext) { result -> result.fold(resolve, reject) })
}

internal suspend fun sleep(milliseconds: Double) {
    Promise<Unit> { resolve, _ -> setTimeout({ resolve(Unit) }, milliseconds) }.await()
}
