// The Web APIs Workbox runs on, as external declarations: CacheStorage and Cache, fetch, Request,
// Response and Headers, URL and URLSearchParams, RegExp, timers, and the JS globals the adapter
// reads. A JS exception one of them throws is a Kotlin Throwable (a JS Error) and keeps its JS
// name; `await` and `launch` put Kotlin coroutines on host promises.

package workbox

import kotlin.coroutines.Continuation
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlin.coroutines.startCoroutine
import kotlin.coroutines.suspendCoroutine
import kotlin.js.Json
import kotlin.js.Promise
import kotlin.js.json

external class Headers(init: Headers)

external class Request(input: Any, init: Json = definedExternally) {
    val url: String
    val method: String
    val mode: String
    val integrity: String
    val headers: Headers
    fun clone(): Request
}

external class Response(body: Any?, init: Json) {
    val status: Int
    val statusText: String
    val url: String
    val redirected: Boolean
    val type: String
    val headers: Headers
    fun clone(): Response
    fun text(): Promise<String>
    fun blob(): Promise<Any>
}

external class URLSearchParams {
    fun keys(): JsIterator<String>
    fun set(name: String, value: Any?)
    fun delete(name: String)
}

external interface JsIterator<T>

external class URL(url: String, base: String = definedExternally) {
    val href: String
    val origin: String
    val protocol: String
    var pathname: String
    var hash: String
    val searchParams: URLSearchParams
}

external class Cache {
    fun match(request: Any): Promise<Response?>
    fun put(request: Request, response: Response): Promise<Unit>
    fun delete(request: Request): Promise<Boolean>
    fun keys(request: Request = definedExternally, options: Json = definedExternally): Promise<Array<Request>>
}

external interface CacheStorage {
    fun open(cacheName: String): Promise<Cache>
    fun match(request: Request, options: Json): Promise<Response?>
    fun keys(): Promise<Array<String>>
    fun delete(cacheName: String): Promise<Boolean>
}

external interface Location {
    val href: String
    val origin: String
}

external interface Crypto {
    fun randomUUID(): String
}

external val caches: CacheStorage
external val location: Location
external val crypto: Crypto

@JsName("fetch")
external fun hostFetch(request: Request): Promise<Response>

external fun setTimeout(handler: () -> Unit, timeout: Int): Int

@JsName("Error")
external class JsError(message: String)

@JsName("TypeError")
external class JsTypeError(message: String)

@JsName("String")
external fun jsString(value: Any?): String

@JsName("Array")
external object JsArray {
    fun <T> from(iterable: JsIterator<T>): Array<T>
}

@JsName("Object")
external object JsObject {
    fun defineProperty(target: Any, key: String, descriptor: Json): Any
}

@JsName("Promise")
external object JsPromise {
    fun allSettled(promises: Array<out Promise<*>>): Promise<Array<SettledResult>>
}

external interface SettledResult {
    val status: String
    val reason: Any?
}

/** `Array.from(url.searchParams.keys())`. */
fun URLSearchParams.names(): List<String> = JsArray.from(keys()).asList()

/** The members of `new Request(input, init)` the subset uses; an absent one stays out of the
 *  dictionary (a null would be converted to the string "null"). */
fun requestInit(
    method: String? = null,
    mode: String? = null,
    integrity: String? = null,
    cache: String? = null,
    credentials: String? = null,
    headers: Headers? = null,
): Json {
    val init = json()
    method?.let { init["method"] = it }
    mode?.let { init["mode"] = it }
    integrity?.let { init["integrity"] = it }
    cache?.let { init["cache"] = it }
    credentials?.let { init["credentials"] = it }
    headers?.let { init["headers"] = it }
    return init
}

fun isUndefined(value: Any?) = jsTypeOf(value) == "undefined"

/** JavaScript truthiness of a JSON value (`null` covers undefined as well). */
fun isTruthy(value: Any?): Boolean = when (value) {
    null -> false
    is String -> value.isNotEmpty()
    is Number -> value.toDouble().let { it != 0.0 && !it.isNaN() }
    is Boolean -> value
    else -> true
}

/** A JS value thrown that is no Error, as a Throwable: Kotlin/JS can only throw and catch a
 *  Throwable. The fail fixture's `value` form throws one. */
class HostError(val value: Any?) : Exception()

fun thrown(value: Any?): Throwable = value as? Throwable ?: HostError(value)

/** `error instanceof Error`, the guard Workbox applies before it keeps or reports a caught value. */
val Throwable.isError: Boolean get() = this !is HostError

suspend fun <T> Promise<T>.await(): T = suspendCoroutine { continuation ->
    then({ value -> continuation.resume(value) }, { error -> continuation.resumeWithException(thrown(error)) })
}

/** Starts [block] at once, as an async function starts: it runs up to its first suspension
 *  before `launch` returns, and the promise settles with its outcome. */
fun <T> launch(block: suspend () -> T): Promise<T> = Promise { resolve, reject ->
    block.startCoroutine(Continuation(EmptyCoroutineContext) { result -> result.fold(resolve, reject) })
}

/** The JS `name` and `message` of a thrown Error. */
val Throwable.jsName: String get() = asDynamic().name as String
val Throwable.jsMessage: String? get() = asDynamic().message as String?
