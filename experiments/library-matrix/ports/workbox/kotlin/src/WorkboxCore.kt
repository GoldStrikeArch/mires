// workbox-core: WorkboxError with its production messages, copyResponse, cacheMatchIgnoreParams,
// timeout, waitUntil and Deferred, and the ExtendableEvent a strategy, install or activate runs
// under.

package workbox

import kotlin.js.JSON
import kotlin.js.Json
import kotlin.js.Promise
import kotlin.js.json

/** `new WorkboxError(code, details)` from the production build: the message is
 *  `code :: JSON.stringify([details])`, so an absent details object prints `[null]`. [name] is the
 *  code, as Workbox sets it. */
class WorkboxError(val name: String, val details: Json? = null) : Exception(name + " :: " + JSON.stringify(arrayOf(details)))

/** What `JSON.stringify` prints for an error nested in a WorkboxError's details: a WorkboxError's
 *  own enumerable properties (`name`, `details`), and nothing for a native Error. */
fun Throwable.asDetail(): Json =
    if (this is WorkboxError) json("name" to name, "details" to (details ?: undefined)) else json()

/** The event a strategy, `install` or `activate` runs under: a native ExtendableEvent, or the
 *  stand-in a page supplies. */
interface ExtendableEvent {
    val type: String
    fun waitUntil(promise: Promise<*>)
}

/** `waitUntil(event, asyncFn)`: the started call extends the event's lifetime and is returned. */
fun <T> waitUntil(event: ExtendableEvent, asyncFn: suspend () -> T): Promise<T> {
    val returnPromise = launch(asyncFn)
    event.waitUntil(returnPromise)
    return returnPromise
}

/** `timeout(ms)`: one timer task. */
suspend fun timeout(ms: Int) {
    Promise<Unit> { resolve, _ -> setTimeout({ resolve(Unit) }, ms) }.await()
}

/** `Deferred`: a promise and the function that resolves it. */
class Deferred {
    private lateinit var settle: (Nothing?) -> Unit
    val promise = Promise<Nothing?> { resolve, _ -> settle = resolve }

    fun resolve() = settle(null)
}

/** `copyResponse(response)`: refuses a response from another origin (or without a URL), then
 *  builds a new Response from a clone's headers, status, status text and body. Workbox's
 *  `canConstructResponseFromBodyStream` always answers false, so the body is read as a Blob. */
suspend fun copyResponse(response: Response): Response {
    val origin = if (response.url.isNotEmpty()) URL(response.url).origin else null
    if (origin != location.origin) throw WorkboxError("cross-origin-copy-response", json("origin" to origin))
    val clonedResponse = response.clone()
    val responseInit = json(
        "headers" to Headers(clonedResponse.headers),
        "status" to clonedResponse.status,
        "statusText" to clonedResponse.statusText,
    )
    val body = clonedResponse.blob().await()
    return Response(body, responseInit)
}

private fun stripParams(fullURL: String, ignoreParams: List<String>): String {
    val strippedURL = URL(fullURL)
    for (param in ignoreParams) strippedURL.searchParams.delete(param)
    return strippedURL.href
}

/** `cacheMatchIgnoreParams(cache, request, ignoreParams)`: an ordinary match when the request has
 *  none of the parameters, else the first key that is the request once both are stripped. */
suspend fun cacheMatchIgnoreParams(cache: Cache, request: Request, ignoreParams: List<String>): Response? {
    val strippedRequestURL = stripParams(request.url, ignoreParams)
    if (request.url == strippedRequestURL) return cache.match(request).await()
    val keysOptions = json("ignoreSearch" to true)
    val cacheKeys = cache.keys(request, keysOptions).await()
    for (cacheKey in cacheKeys) {
        if (strippedRequestURL == stripParams(cacheKey.url, ignoreParams)) return cache.match(cacheKey).await()
    }
    return null
}
