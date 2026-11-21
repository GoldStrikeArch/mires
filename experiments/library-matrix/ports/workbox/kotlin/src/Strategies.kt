// workbox-strategies: the plugin interface, Strategy with its handle/handleAll lifecycle,
// StrategyHandler, and the CacheFirst, CacheOnly, NetworkFirst and NetworkOnly strategies.

package workbox

import kotlin.js.Promise
import kotlin.js.json

/** The state object a handler keeps for one plugin (`state` in every callback), as Workbox's
 *  `MapLikeObject`. */
typealias PluginState = MutableMap<String, Any?>

/** The route parameters a handler receives: `{cacheKey, integrity}` from a PrecacheRoute match,
 *  `{cacheKey}` from createHandlerBoundToURL. */
class Params(val cacheKey: String? = null, val integrity: String? = null)

class HandlerWillStartParam(val request: Request, val event: ExtendableEvent, val state: PluginState?)
class CacheKeyWillBeUsedParam(val mode: String, val request: Request, val event: ExtendableEvent, val params: Params?, val state: PluginState?)
class CachedResponseWillBeUsedParam(val cacheName: String, val request: Request, val cachedResponse: Response?, val event: ExtendableEvent, val state: PluginState?)
class CacheWillUpdateParam(val request: Request, val response: Response, val event: ExtendableEvent, val state: PluginState?)
class CacheDidUpdateParam(val cacheName: String, val oldResponse: Response?, val newResponse: Response, val request: Request, val event: ExtendableEvent, val state: PluginState?)
class RequestWillFetchParam(val request: Request, val event: ExtendableEvent, val state: PluginState?)
class FetchDidSucceedParam(val request: Request, val response: Response, val event: ExtendableEvent, val state: PluginState?)
class FetchDidFailParam(val error: Throwable, val originalRequest: Request, val request: Request, val event: ExtendableEvent, val state: PluginState?)
class HandlerWillRespondParam(val request: Request, val response: Response, val event: ExtendableEvent, val state: PluginState?)
class HandlerDidRespondParam(val request: Request, val response: Response?, val event: ExtendableEvent, val state: PluginState?)
class HandlerDidCompleteParam(val request: Request, val response: Response?, val error: Throwable?, val event: ExtendableEvent, val state: PluginState?)
class HandlerDidErrorParam(val request: Request, val error: Throwable, val event: ExtendableEvent, val state: PluginState?)

/** `WorkboxPlugin`: an object of optional lifecycle callbacks. A plugin defines a callback by
 *  overriding its property; Workbox calls only the callbacks a plugin defines. */
interface WorkboxPlugin {
    val handlerWillStart: (suspend (HandlerWillStartParam) -> Unit)? get() = null
    val cacheKeyWillBeUsed: (suspend (CacheKeyWillBeUsedParam) -> Request)? get() = null
    val cachedResponseWillBeUsed: (suspend (CachedResponseWillBeUsedParam) -> Response?)? get() = null
    val cacheWillUpdate: (suspend (CacheWillUpdateParam) -> Response?)? get() = null
    val cacheDidUpdate: (suspend (CacheDidUpdateParam) -> Unit)? get() = null
    val requestWillFetch: (suspend (RequestWillFetchParam) -> Request)? get() = null
    val fetchDidSucceed: (suspend (FetchDidSucceedParam) -> Response)? get() = null
    val fetchDidFail: (suspend (FetchDidFailParam) -> Unit)? get() = null
    val handlerWillRespond: (suspend (HandlerWillRespondParam) -> Response)? get() = null
    val handlerDidRespond: (suspend (HandlerDidRespondParam) -> Unit)? get() = null
    val handlerDidComplete: (suspend (HandlerDidCompleteParam) -> Unit)? get() = null
    val handlerDidError: (suspend (HandlerDidErrorParam) -> Response?)? get() = null
}

/** `HandlerCallbackOptions`: what a route handler is called with. */
class HandlerCallbackOptions(val request: Request, val event: ExtendableEvent, val params: Params? = null)

/** A route handler as `normalizeHandler` leaves it: an object with `handle`. */
fun interface RouteHandler {
    fun handle(options: HandlerCallbackOptions): Promise<Response>
}

/** `Strategy`: a cache name, a mutable plugin list and the subclass's `_handle`. The cache name
 *  is always supplied (`cacheNames.getRuntimeName(cacheName)` returns it unchanged). */
abstract class Strategy(val cacheName: String, plugins: List<WorkboxPlugin>) : RouteHandler {
    val plugins: MutableList<WorkboxPlugin> = plugins.toMutableList()

    /** `handle(options)`: the response promise of `handleAll`; the completion keeps running. */
    override fun handle(options: HandlerCallbackOptions): Promise<Response> = handleAll(options).first

    /** `handleAll(options)`: a handler for the request, and its response and completion
     *  promises, both already running. */
    fun handleAll(options: HandlerCallbackOptions): Pair<Promise<Response>, Promise<Unit>> {
        val handler = StrategyHandler(this, options)
        val responseDone = launch { getResponse(handler, options.request, options.event) }
        val handlerDone = launch { awaitComplete(responseDone, handler, options.request, options.event) }
        return responseDone to handlerDone
    }

    /** The subclass's `_handle(request, handler)`. */
    protected abstract suspend fun _handle(request: Request, handler: StrategyHandler): Response

    /** `_getResponse`: handlerWillStart, `_handle`, handlerDidError on failure, handlerWillRespond. */
    private suspend fun getResponse(handler: StrategyHandler, request: Request, event: ExtendableEvent): Response {
        for ((callback, state) in handler.iterateCallbacks { it.handlerWillStart }) {
            callback(HandlerWillStartParam(request, event, state))
        }
        var response: Response? = null
        try {
            response = _handle(request, handler)
            // A response of type "error" stays the response unless a handlerDidError callback
            // replaces it, so it is returned when no callback does.
            if (response.type == "error") throw WorkboxError("no-response", json("url" to request.url))
        } catch (error: Throwable) {
            if (error.isError) {
                for ((callback, state) in handler.iterateCallbacks { it.handlerDidError }) {
                    response = callback(HandlerDidErrorParam(request, error, event, state))
                    if (response != null) break
                }
            }
            if (response == null) throw error
        }
        // `_handle` answered, or a handlerDidError callback did.
        var result: Response = response
        for ((callback, state) in handler.iterateCallbacks { it.handlerWillRespond }) {
            result = callback(HandlerWillRespondParam(request, result, event, state))
        }
        return result
    }

    /** `_awaitComplete`: handlerDidRespond and the handler's extended lifetimes, then
     *  handlerDidComplete with the Error among them, then the handler's own promise resolves. */
    private suspend fun awaitComplete(responseDone: Promise<Response>, handler: StrategyHandler, request: Request, event: ExtendableEvent) {
        val response = try {
            responseDone.await()
        } catch (_: Throwable) {
            null
        }
        var error: Throwable? = null
        try {
            for ((callback, state) in handler.iterateCallbacks { it.handlerDidRespond }) {
                callback(HandlerDidRespondParam(request, response, event, state))
            }
            handler.doneWaiting()
        } catch (waitUntilError: Throwable) {
            if (waitUntilError.isError) error = waitUntilError
        }
        for ((callback, state) in handler.iterateCallbacks { it.handlerDidComplete }) {
            callback(HandlerDidCompleteParam(request, response, error, event, state))
        }
        handler.destroy()
        if (error != null) throw error
    }
}

/** `StrategyHandler`: one request's run through a strategy. */
class StrategyHandler(private val strategy: Strategy, options: HandlerCallbackOptions) {
    val request = options.request
    val event = options.event
    val params = options.params
    private val cacheKeys = HashMap<String, Request>()
    private val handlerDeferred = Deferred()
    private val extendLifetimePromises = mutableListOf<Promise<*>>()

    // The plugins present now get a state; a plugin added later is called without one.
    private val pluginStateMap: Map<WorkboxPlugin, PluginState> = strategy.plugins.associateWith { HashMap() }

    init {
        event.waitUntil(handlerDeferred.promise)
    }

    /** `fetch(input)`: requestWillFetch, the network, then fetchDidSucceed or fetchDidFail. */
    suspend fun fetch(input: Request): Response {
        var request = input
        val originalRequest = if (hasCallback { it.fetchDidFail }) request.clone() else null
        try {
            for ((callback, state) in iterateCallbacks { it.requestWillFetch }) {
                request = callback(RequestWillFetchParam(request.clone(), event, state))
            }
        } catch (error: Throwable) {
            // A thrown non-Error ends the loop, and the request goes out as it is.
            if (error.isError) throw WorkboxError("plugin-error-request-will-fetch", json("thrownErrorMessage" to error.jsMessage))
        }
        val pluginFilteredRequest = request.clone()
        try {
            var fetchResponse = hostFetch(request).await()
            for ((callback, state) in iterateCallbacks { it.fetchDidSucceed }) {
                fetchResponse = callback(FetchDidSucceedParam(pluginFilteredRequest, fetchResponse, event, state))
            }
            return fetchResponse
        } catch (error: Throwable) {
            if (originalRequest != null) {
                // One clone of each for every callback, as runCallbacks passes one param object.
                val original = originalRequest.clone()
                val filtered = pluginFilteredRequest.clone()
                for ((callback, state) in iterateCallbacks { it.fetchDidFail }) {
                    callback(FetchDidFailParam(error, original, filtered, event, state))
                }
            }
            throw error
        }
    }

    /** `fetchAndCachePut(input)`: the response now, and its cachePut as an extended lifetime. */
    suspend fun fetchAndCachePut(input: Request): Response {
        val response = fetch(input)
        val responseClone = response.clone()
        waitUntil(launch { cachePut(input, responseClone) })
        return response
    }

    /** `cacheMatch(key)`: `caches.match` of the read cache key in the strategy's cache (which opens
     *  no cache), through cachedResponseWillBeUsed. */
    suspend fun cacheMatch(key: Request): Response? {
        val effectiveRequest = getCacheKey(key, "read")
        var cachedResponse = caches.match(effectiveRequest, json("cacheName" to strategy.cacheName)).await()
        for ((callback, state) in iterateCallbacks { it.cachedResponseWillBeUsed }) {
            cachedResponse = callback(CachedResponseWillBeUsedParam(strategy.cacheName, effectiveRequest, cachedResponse, event, state))
        }
        return cachedResponse
    }

    /** `cachePut(key, response)`: one timer task later, the write cache key, cacheWillUpdate (or
     *  the status-200 rule), the put, then cacheDidUpdate. Answers whether it cached. */
    suspend fun cachePut(key: Request, response: Response): Boolean {
        timeout(0)
        val effectiveRequest = getCacheKey(key, "write")
        val responseToCache = ensureResponseSafeToCache(response) ?: return false
        val cache = caches.open(strategy.cacheName).await()
        val hasCacheUpdateCallback = hasCallback { it.cacheDidUpdate }
        val oldResponse = if (hasCacheUpdateCallback) cacheMatchIgnoreParams(cache, effectiveRequest.clone(), listOf("__WB_REVISION__")) else null
        try {
            cache.put(effectiveRequest, if (hasCacheUpdateCallback) responseToCache.clone() else responseToCache).await()
        } catch (error: Throwable) {
            // An Error is rethrown (a QuotaExceededError after the registered quota callbacks, of
            // which the subset has none); anything else is ignored.
            if (error.isError) throw error
        }
        for ((callback, state) in iterateCallbacks { it.cacheDidUpdate }) {
            callback(CacheDidUpdateParam(strategy.cacheName, oldResponse, responseToCache.clone(), effectiveRequest, event, state))
        }
        return true
    }

    /** `getCacheKey(request, mode)`: cacheKeyWillBeUsed once per URL and mode for this handler. */
    suspend fun getCacheKey(request: Request, mode: String): Request {
        val key = "${request.url} | $mode"
        cacheKeys[key]?.let { return it }
        var effectiveRequest = request
        for ((callback, state) in iterateCallbacks { it.cacheKeyWillBeUsed }) {
            effectiveRequest = callback(CacheKeyWillBeUsedParam(mode, effectiveRequest, event, params, state))
        }
        cacheKeys[key] = effectiveRequest
        return effectiveRequest
    }

    /** `hasCallback(name)`, over the strategy's current plugins. */
    fun hasCallback(callback: (WorkboxPlugin) -> Any?): Boolean = strategy.plugins.any { callback(it) != null }

    /** `iterateCallbacks(name)`: a generator over the strategy's plugins that define the callback.
     *  It reads the live list at every step, so a plugin added meanwhile is reached (without
     *  state), as Workbox's `for…of` over `this._strategy.plugins` reaches it. */
    fun <T : Any> iterateCallbacks(callback: (WorkboxPlugin) -> T?): Sequence<Pair<T, PluginState?>> = sequence {
        var index = 0
        while (index < strategy.plugins.size) {
            val plugin = strategy.plugins[index++]
            callback(plugin)?.let { yield(it to pluginStateMap[plugin]) }
        }
    }

    /** `waitUntil(promise)`: extends the handler's lifetime until the promise settles. */
    fun <T> waitUntil(promise: Promise<T>): Promise<T> {
        extendLifetimePromises += promise
        return promise
    }

    /** `doneWaiting()`: settles every extended lifetime, including ones added meanwhile, and
     *  throws the first rejection of a batch. */
    suspend fun doneWaiting() {
        while (extendLifetimePromises.isNotEmpty()) {
            val batch = extendLifetimePromises.toTypedArray()
            extendLifetimePromises.clear()
            val rejected = JsPromise.allSettled(batch).await().firstOrNull { it.status == "rejected" }
            if (rejected != null) throw thrown(rejected.reason)
        }
    }

    fun destroy() = handlerDeferred.resolve()

    /** `_ensureResponseSafeToCache(response)`: every cacheWillUpdate in turn until one refuses,
     *  or, without any, only a status 200. */
    private suspend fun ensureResponseSafeToCache(response: Response): Response? {
        var responseToCache: Response = response
        var pluginsUsed = false
        for ((callback, state) in iterateCallbacks { it.cacheWillUpdate }) {
            pluginsUsed = true
            responseToCache = callback(CacheWillUpdateParam(request, responseToCache, event, state)) ?: return null
        }
        return if (pluginsUsed || responseToCache.status == 200) responseToCache else null
    }
}

/** `new WorkboxError('no-response', {url, error})`; the caught value is kept only when it is an
 *  Error (`error` is then left out of the details). */
private fun noResponse(request: Request, error: Throwable? = null) =
    WorkboxError("no-response", json("url" to request.url, "error" to (error?.asDetail() ?: undefined)))

class CacheFirst(cacheName: String, plugins: List<WorkboxPlugin> = emptyList()) : Strategy(cacheName, plugins) {
    override suspend fun _handle(request: Request, handler: StrategyHandler): Response {
        var error: Throwable? = null
        var response = handler.cacheMatch(request)
        if (response == null) {
            try {
                response = handler.fetchAndCachePut(request)
            } catch (caught: Throwable) {
                if (caught.isError) error = caught
            }
        }
        return response ?: throw noResponse(request, error)
    }
}

class CacheOnly(cacheName: String, plugins: List<WorkboxPlugin> = emptyList()) : Strategy(cacheName, plugins) {
    override suspend fun _handle(request: Request, handler: StrategyHandler): Response =
        handler.cacheMatch(request) ?: throw noResponse(request)
}

/** `cacheOkAndOpaquePlugin`: status 200, or 0 for an opaque response. */
object CacheOkAndOpaquePlugin : WorkboxPlugin {
    override val cacheWillUpdate: suspend (CacheWillUpdateParam) -> Response? = { param ->
        param.response.takeIf { it.status == 200 || it.status == 0 }
    }
}

/** `NetworkFirst` without `networkTimeoutSeconds`. Its constructor puts the 200-or-opaque plugin
 *  first unless a plugin has a cacheWillUpdate. */
class NetworkFirst(cacheName: String, plugins: List<WorkboxPlugin> = emptyList()) : Strategy(cacheName, plugins) {
    init {
        if (this.plugins.none { it.cacheWillUpdate != null }) this.plugins.add(0, CacheOkAndOpaquePlugin)
    }

    override suspend fun _handle(request: Request, handler: StrategyHandler): Response {
        val networkPromise = launch { getNetworkPromise(request, handler) }
        val promises = arrayOf(networkPromise)
        val response = handler.waitUntil(launch { handler.waitUntil(Promise.race(promises)).await() ?: networkPromise.await() }).await()
        return response ?: throw noResponse(request)
    }

    /** `_getNetworkPromise`: fetchAndCachePut, and the cache when that failed or answered nothing. */
    private suspend fun getNetworkPromise(request: Request, handler: StrategyHandler): Response? {
        var error: Throwable? = null
        var response: Response? = null
        try {
            response = handler.fetchAndCachePut(request)
        } catch (caught: Throwable) {
            if (caught.isError) error = caught
        }
        if (error != null || response == null) response = handler.cacheMatch(request)
        return response
    }
}

/** `NetworkOnly` without `networkTimeoutSeconds`. */
class NetworkOnly(cacheName: String, plugins: List<WorkboxPlugin> = emptyList()) : Strategy(cacheName, plugins) {
    override suspend fun _handle(request: Request, handler: StrategyHandler): Response {
        var error: Throwable? = null
        var response: Response? = null
        try {
            response = handler.fetch(request)
        } catch (caught: Throwable) {
            if (caught.isError) error = caught
        }
        return response ?: throw noResponse(request, error)
    }
}
