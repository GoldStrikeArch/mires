// Ky 2.2.0 (core/Ky.js, core/constants.js, utils/normalize.js, utils/merge.js, utils/timeout.js,
// utils/is-network-error.js and errors/*.js), the subset in libraries/ky/CONTRACT.md. Ky's async
// methods are suspend functions; what Ky throws, they throw.

package ky

import kotlin.js.Json
import kotlin.js.Promise
import kotlin.js.json
import kotlin.math.min

/** The errors/ modules: Ky's own errors. Everything else a call can fail with (a TypeError, RangeError,
 *  SyntaxError or abort reason, from a host API, the transport or Ky's option checks) is the JS
 *  error that was thrown. */
sealed class KyError(val errorName: String, message: String) : Exception(message)

class HTTPError(val response: Response, val request: Request, val data: Any?) :
    KyError("HTTPError", "Request failed with status code ${response.status}: ${request.method} ${request.url}")

class NetworkError(val request: Request, override val cause: Throwable) :
    KyError("NetworkError", "Request failed due to a network error: ${request.method} ${request.url}")

class TimeoutError(val request: Request) :
    KyError("TimeoutError", "Request timed out: ${request.method} ${request.url}")

/** The JS value a Throwable stands for: the value itself, or what a [HostError] holds. */
internal val Throwable.jsValue: dynamic
    get() = if (this is HostError) value else this

/** `error.name`, as the hooks and the caller read it. */
val Throwable.name: String
    get() = if (this is KyError) errorName else jsValue.name as? String ?: "Error"

class Hooks(
    val init: List<() -> Unit>,
    val beforeRequest: List<(request: Request, retryCount: Int) -> Unit>,
    val afterResponse: List<(request: Request, response: Response, retryCount: Int) -> Unit>,
    val beforeRetry: List<(request: Request, error: Throwable, retryCount: Int) -> Unit>,
    val beforeError: List<(request: Request, error: Throwable, retryCount: Int) -> Unit>,
)

/** The caller's retry options, as the JS values given (undefined where omitted);
 *  [normalizeRetryOptions] validates them. */
class RetryOptions(
    val limit: Any?,
    val methods: Any?,
    val statusCodes: Any?,
    val maxRetryAfter: Any?,
    val backoffLimit: Any?,
    val retryOnTimeout: Any?,
    val delay: (attemptCount: Int) -> Double,
)

class Options(
    val method: String?,
    /** A plain object of header names to values. */
    val headers: Any?,
    val json: Any?,
    val retry: RetryOptions,
    val timeout: Any?,
    val throwHttpErrors: Any?,
    val hooks: Hooks,
    val fetch: (Request) -> Promise<Response>,
    val signal: AbortSignal?,
)

private class Retry(
    val limit: Double,
    val methods: List<String>,
    val statusCodes: List<Double>,
    val afterStatusCodes: List<Double>,
    val maxRetryAfter: Double,
    val backoffLimit: Double,
    val delay: (attemptCount: Int) -> Double,
    val retryOnTimeout: Boolean,
)

// core/constants.js and utils/normalize.js
private val requestMethods = listOf("get", "post", "put", "patch", "head", "delete", "query")
private val caseInsensitiveMethods = requestMethods + listOf("options", "trace")
private val retryMethods = listOf("get", "put", "head", "delete", "options", "trace", "query")
private val retryStatusCodes = listOf(408.0, 413.0, 429.0, 500.0, 502.0, 503.0, 504.0)
private val retryAfterStatusCodes = listOf(413.0, 429.0, 503.0)

/** The maximum value of a 32bit int (see issue #117) */
private const val MAX_SAFE_TIMEOUT = 2_147_483_647.0
private const val MAX_ERROR_RESPONSE_BODY_SIZE = 10 * 1024 * 1024

// Kotlin/JS's lowercase() and uppercase() are JS's toLowerCase() and toUpperCase().
private fun normalizeRequestMethod(input: String) =
    if (input.lowercase() in requestMethods) input.uppercase() else input

/** Common methods for retry matching are case-insensitive; custom ones keep their case. */
private fun normalizeRetryMethod(method: String) =
    method.lowercase().let { if (it in caseInsensitiveMethods) it else method }

private fun nonNegative(value: Any?): Double? = (value as? Double)?.takeIf { it >= 0 }

/** The defaults for undefined values, and Ky's checks, in Ky's order. */
private fun normalizeRetryOptions(retry: RetryOptions): Retry {
    val limitValue = retry.limit
    val limit = when {
        isUndefined(limitValue) -> 2.0
        limitValue is Double && JsNumber.isInteger(limitValue) && limitValue >= 0 -> limitValue
        else -> throw typeError("`retry.limit` must be a finite, non-negative integer")
    }
    fun array(key: String, value: Any?): Array<Any?>? = when {
        isUndefined(value) -> null
        value is Array<*> -> value.unsafeCast<Array<Any?>>()
        else -> throw plainError("retry.$key must be an array")
    }
    val methods = array("methods", retry.methods)
    val statusCodes = array("statusCodes", retry.statusCodes)
    // A mistyped entry only ever fails to match, so it is reported instead.
    if (methods != null && methods.any { it !is String }) {
        throw typeError("`retry.methods` must only contain strings")
    }
    if (statusCodes != null && statusCodes.any { !JsNumber.isInteger(it) }) {
        throw typeError("`retry.statusCodes` must only contain numbers")
    }
    fun limitOption(key: String, value: Any?) = if (isUndefined(value)) {
        Double.POSITIVE_INFINITY
    } else {
        nonNegative(value) ?: throw typeError("`retry.$key` must be a non-negative number or `Infinity`")
    }
    val maxRetryAfter = limitOption("maxRetryAfter", retry.maxRetryAfter)
    val backoffLimit = limitOption("backoffLimit", retry.backoffLimit)
    val retryOnTimeoutValue = retry.retryOnTimeout
    val retryOnTimeout = when {
        isUndefined(retryOnTimeoutValue) -> false
        retryOnTimeoutValue is Boolean -> retryOnTimeoutValue
        else -> throw typeError("`retry.retryOnTimeout` must be a boolean")
    }
    return Retry(
        limit = limit,
        methods = (methods?.map { it as String } ?: retryMethods).map(::normalizeRetryMethod),
        statusCodes = statusCodes?.map { it as Double } ?: retryStatusCodes,
        afterStatusCodes = retryAfterStatusCodes,
        maxRetryAfter = maxRetryAfter,
        backoffLimit = backoffLimit,
        delay = retry.delay,
        retryOnTimeout = retryOnTimeout,
    )
}

/** Milliseconds, or null for `false`. */
private fun validateTimeoutOption(value: Any?): Double? {
    if (value == false) return null
    val milliseconds = nonNegative(value)
        ?: throw typeError("The `timeout` option must be a non-negative number or `false`")
    // Infinity lands here too, since it is not a usable delay either.
    if (milliseconds > MAX_SAFE_TIMEOUT) throw rangeError("The `timeout` option cannot be greater than 2147483647")
    return milliseconds
}

/** Option merging (deepMerge, mergeHeaderContainers) keys the headers by their lowercased name, the
 *  last spelling of a name winning at the place of its first. */
private fun lowercaseHeaders(headers: Any): Map<String, Any?> {
    val merged = linkedMapOf<String, Any?>()
    for (name in JsObject.keys(headers)) merged[name.lowercase()] = headers.asDynamic()[name]
    return merged
}

/** mergeHeaders: each header set on a fresh Headers, which validates it. */
private suspend fun mergeHeaders(headers: Map<String, Any?>): Headers {
    val result = Headers()
    for ((name, value) in headers) hostCall { result.set(name, value) }
    return result
}

private val networkErrorMessages = setOf(
    "network error",
    "NetworkError when attempting to fetch resource.",
    "The Internet connection appears to be offline.",
    "Network request failed",
    "fetch failed",
    "terminated",
    " A network error occurred.",
    "Network connection lost",
)

/** Ky's `isError`: an Error, or an Error of another realm by its internal brand. */
private fun isError(value: dynamic): Boolean =
    value is Throwable || ObjectPrototype.prototype.toString.call(value) == "[object Error]"

/** utils/is-network-error.js, inlined by Ky from is-network-error 1.3.2. */
private fun isRawNetworkError(error: Throwable): Boolean {
    val value = error.jsValue
    if (error is KyError || !isError(value) || error.name != "TypeError") return false
    val message = value.message as? String ?: return false
    fun wrapped(prefix: String) = message.startsWith(prefix) && message.endsWith(")")
    if (message == "Load failed" || wrapped("Load failed (")) {
        // Safari 17+ has generic message but no stack for network errors
        return isUndefined(value.stack) || JsReflect.has(value.unsafeCast<Any>(), "__sentry_captured__")
    }
    return message.startsWith("error sending request for url") ||
        message == "Failed to fetch" || wrapped("Failed to fetch (") ||
        message in networkErrorMessages
}

/** JS whitespace: what String.prototype.trim removes and `\s` matches. Kotlin's isWhitespace adds
 *  U+001C to U+001F and U+0085, and lacks U+FEFF. */
private fun isJsWhitespace(char: Char) = when (char) {
    '\t', '\n', '\u000B', '\u000C', '\r', ' ', '\u00A0', '\u1680', in '\u2000'..'\u200A',
    '\u2028', '\u2029', '\u202F', '\u205F', '\u3000', '\uFEFF' -> true
    else -> false
}

/** Matches JSON subtypes like `json`, `problem+json` and `vnd.api+json`, written out rather than
 *  matched with a pattern, as Ky does. */
private fun isJsonContentType(contentType: String): Boolean {
    val mimeType = contentType.substringBefore(';').trim(::isJsWhitespace).lowercase()
    if (!mimeType.endsWith("json")) return false
    return when (mimeType.getOrNull(mimeType.length - 5)) {
        '/' -> true
        // The separator is not the slash, so any slash in the type comes before it.
        '.', '+', '-' -> '/' in mimeType
        else -> false
    }
}

private val charsetPattern = Regex(""";\s*charset\s*=\s*(?:"([^"]+)"|([^;,\s]+))""", RegexOption.IGNORE_CASE)

private suspend fun createTextDecoder(contentType: String): TextDecoder {
    val groups = charsetPattern.find(contentType)?.groups
    val charset = groups?.get(1)?.value ?: groups?.get(2)?.value
    if (!charset.isNullOrEmpty()) {
        try {
            return hostCall { TextDecoder(charset) }
        } catch (_: Throwable) {
            // An unknown label falls back to UTF-8.
        }
    }
    return TextDecoder()
}

/** An error body as text, or null when it cannot be read or is larger than
 *  maxErrorResponseBodySize. JSON uses UTF-8 regardless of the charset parameter (RFC 8259). */
private suspend fun readResponseText(response: Response): String? {
    if (response.body == null) {
        return try {
            response.text().await()
        } catch (_: Throwable) {
            null
        }
    }
    val contentType = response.headers.get("content-type") ?: ""
    val decoder = if (isJsonContentType(contentType)) TextDecoder() else createTextDecoder(contentType)
    return try {
        val bytes = response.arrayBuffer().await()
        if (bytes.byteLength > MAX_ERROR_RESPONSE_BODY_SIZE) null else decoder.decode(bytes)
    } catch (_: Throwable) {
        null
    }
}

/** #getResponseData: the parsed JSON or the text of an error body, or null when it is empty,
 *  unreadable or unparsable. */
private suspend fun getResponseData(response: Response): Any? {
    val text = readResponseText(response)
    if (text.isNullOrEmpty()) return null
    if (!isJsonContentType(response.headers.get("content-type") ?: "")) return text
    return try {
        JSON.parse<Any?>(text)
    } catch (_: Throwable) {
        null
    }
}

/** The body shortcuts, and the media type each one advertises. */
enum class Shortcut(val mimeType: String) { Json("application/json"), Text("text/*") }

class Ky private constructor(
    private var request: Request,
    private var abortController: AbortController,
    /** #options.signal: the user's signal and the abort controller's, combined. */
    private var signal: AbortSignal,
    private val userProvidedAbortSignal: AbortSignal?,
    private val retry: Retry,
    private val timeout: Double?,
    private val throwHttpErrors: Boolean,
    private val hooks: Hooks,
    /** The `fetch` option. */
    private val transport: (Request) -> Promise<Response>,
) {
    private var retryCount = 0
    private var originalRequest: Request? = null

    /** #sentRequest: [fetch] replaces [request] with the clone it prepares for a possible retry,
     *  so the request that was actually sent is reported. */
    private val sentRequest get() = originalRequest ?: request

    /** Chromium reports a user abort during a fetch as a network error; the abort reason is
     *  surfaced instead. */
    private fun throwIfAbortedByUser() {
        userProvidedAbortSignal?.throwIfAborted()
    }

    /** A hook runs inside #raceWithTotalTimeout(), which first throws a user abort. */
    private fun <H> runHooks(hooks: List<H>, call: (H) -> Unit) {
        for (hook in hooks) {
            throwIfAbortedByUser()
            call(hook)
        }
    }

    private suspend fun run(): Response {
        runHooks(hooks.beforeRequest) { it(request, 0) }
        var response = retry()
        while (true) {
            runHooks(hooks.afterResponse) { it(request, response, retryCount) }
            if (response.ok || !throwHttpErrors) return response
            val error = HTTPError(response, sentRequest, getResponseData(response))
            throwIfAbortedByUser()
            response = retryFromError(error)
        }
    }

    private suspend fun retry(): Response = try {
        fetch()
    } catch (error: Throwable) {
        retryFromError(error)
    }

    private suspend fun retryFromError(error: Throwable): Response {
        throwIfAbortedByUser()
        val retryDelay = min(calculateRetryDelay(error), MAX_SAFE_TIMEOUT)
        sleep(retryDelay)
        // Reset the abort controller if it was aborted (on a timeout retry), so hooks and the
        // retried request get a fresh signal.
        if (abortController.signal.aborted) {
            abortController = AbortController()
            signal = createManagedSignal(userProvidedAbortSignal, abortController)
            request = request.withSignal(signal)
        }
        runHooks(hooks.beforeRetry) { it(request, error, retryCount + 1) }
        retryCount++
        return retry()
    }

    private suspend fun fetch(): Response {
        // Cloning is done here to prepare in advance for retries, and skipped when they are disabled.
        val retryRequest = if (retry.limit > 0) request.clone().withSignal(signal) else null
        val request = request
        originalRequest = request
        if (retryRequest != null) this.request = retryRequest
        try {
            return (timeout?.let { timeout(request, it) } ?: transport(request)).await()
        } catch (error: Throwable) {
            throwIfAbortedByUser()
            if (isRawNetworkError(error)) throw NetworkError(request, error)
            throw error
        }
    }

    /** utils/timeout.js: the transport raced against a timer that aborts the controller and
     *  rejects with a TimeoutError. A response arriving after the timeout is discarded. */
    private fun timeout(request: Request, milliseconds: Double): Promise<Response> {
        var timedOut = false
        lateinit var timer: Any
        val race = Promise<Response> { resolve, reject ->
            timer = setTimeout({
                timedOut = true
                abortController.abort()
                reject(TimeoutError(request))
            }, milliseconds)
            transport(request).then({ response -> if (!timedOut) resolve(response) }, reject)
        }
        return race.finally { clearTimeout(timer) }
    }

    /** #calculateDelay(): the retry.delay backoff, capped by retry.backoffLimit. */
    private fun calculateDelay(): Double {
        val retryDelay = retry.delay(retryCount + 1)
        if (!(retryDelay >= 0)) throw typeError("`retry.delay` must return a non-negative number or `Infinity`")
        return min(retry.backoffLimit, retryDelay)
    }

    /** #calculateRetryDelay(): the wait before retrying [error], which is thrown again when it is
     *  not retried. */
    private fun calculateRetryDelay(error: Throwable): Double {
        if (retryCount >= retry.limit) throw error
        if (normalizeRetryMethod(request.method) !in retry.methods) throw error
        return when (error) {
            is TimeoutError -> if (retry.retryOnTimeout) calculateDelay() else throw error
            is HTTPError -> {
                val status = error.response.status.toDouble()
                if (status !in retry.statusCodes) throw error
                val header = getRetryTimingHeader(error.response.headers)
                when {
                    header != null && status in retry.afterStatusCodes ->
                        // Malformed retry timing headers should not disable retries; they only
                        // lose their server-provided timing. No jitter applies to a server's timing.
                        calculateRetryTimingDelay(header)?.let { min(retry.maxRetryAfter, it) } ?: calculateDelay()
                    status == 413.0 -> throw error
                    else -> calculateDelay()
                }
            }
            is NetworkError -> calculateDelay()
            // Only known retriable error types are retried.
            else -> throw error
        }
    }

    /** #throwProcessedError(): the beforeError hooks see the error, which then fails the call. */
    private fun throwProcessedError(error: Throwable): Nothing {
        for (hook in hooks.beforeError) hook(sentRequest, error, retryCount)
        throw error
    }

    private fun Request.withSignal(signal: AbortSignal) = Request(this, json("signal" to signal))

    companion object {
        private fun createManagedSignal(user: AbortSignal?, controller: AbortController) =
            if (user != null) AbortSignal.any(arrayOf(user, controller.signal)) else controller.signal

        private suspend fun create(input: String, options: Options): Ky {
            val headers = options.headers?.let(::lowercaseHeaders) ?: emptyMap()
            val requestHeaders = mergeHeaders(headers)
            val method = normalizeRequestMethod(options.method ?: "GET")
            val retry = normalizeRetryOptions(options.retry)
            // Defaults only replace undefined, so a null reaches the validation and is reported.
            val throwHttpErrorsValue = options.throwHttpErrors
            val throwHttpErrors = when {
                isUndefined(throwHttpErrorsValue) -> true
                throwHttpErrorsValue is Boolean -> throwHttpErrorsValue
                else -> throw typeError("The `throwHttpErrors` option must be a boolean or a function")
            }
            val timeout = validateTimeoutOption(if (isUndefined(options.timeout)) 10_000.0 else options.timeout)
            val abortController = AbortController()
            val signal = createManagedSignal(options.signal, abortController)
            // A content-type from options.headers counts as user-provided.
            val body = if (isUndefined(options.json)) null else JSON.stringify(options.json)
            if (body != null && "content-type" !in headers) requestHeaders.set("content-type", "application/json")
            val init: Json = json("method" to method, "headers" to requestHeaders, "signal" to signal)
            if (body != null) init["body"] = body
            return Ky(
                hostCall { Request(input, init) }, abortController, signal, options.signal, retry, timeout,
                throwHttpErrors, options.hooks, options.fetch,
            )
        }

        /** `ky(input, options)[shortcut]()`: the init hooks, the request, the run and the body
         *  read. A construction error fails without running the beforeError hooks, and so does a
         *  body that is not JSON. */
        suspend fun request(input: String, options: Options, shortcut: Shortcut): Any? {
            for (hook in options.hooks.init) hook()
            val ky = create(input, options)
            // Before dispatch, ky.request is the request that will be sent.
            val headers = ky.request.headers
            headers.set("accept", headers.get("accept")?.takeIf { it.isNotEmpty() } ?: shortcut.mimeType)
            val response = try {
                ky.run()
            } catch (error: Throwable) {
                ky.throwProcessedError(error)
            }
            val text = response.text().await()
            return when (shortcut) {
                Shortcut.Text -> text
                Shortcut.Json -> JSON.parse<Any?>(text)
            }
        }
    }
}
