// workbox-precaching: PrecacheController, PrecacheStrategy, PrecacheRoute with its URL variations,
// createCacheKey, and the cache-key, install-report and fallback plugins.

package workbox

import kotlin.js.Promise
import kotlin.js.RegExp
import kotlin.js.json

private const val REVISION_SEARCH_PARAM = "__WB_REVISION__"

/** A precache manifest entry as the caller wrote it (`PrecacheEntry | string`). */
sealed interface PrecacheEntry {
    /** A string: a URL that is its own cache key. */
    class Url(val url: String) : PrecacheEntry

    /** A `PrecacheEntry` object. [revision] stays the caller's value (a string, a number, null or
     *  undefined): its truthiness decides, and the host prints a number. [source] is the object
     *  itself, which add-to-cache-list-unexpected-type reports. */
    class Entry(val url: String?, val revision: Any?, val integrity: String?, val source: Any) : PrecacheEntry

    /** `null`, `''` or another falsy value. */
    class Falsy(val source: Any?) : PrecacheEntry
}

private data class CacheKey(val cacheKey: String, val url: String)

/** `createCacheKey(entry)`: the URL resolved against the location, and with a truthy revision a
 *  cache key that carries it in `__WB_REVISION__`. */
private fun createCacheKey(entry: PrecacheEntry): CacheKey = when (entry) {
    is PrecacheEntry.Falsy -> throw WorkboxError("add-to-cache-list-unexpected-type", json("entry" to entry.source))
    is PrecacheEntry.Url -> URL(entry.url, location.href).href.let { CacheKey(it, it) }
    is PrecacheEntry.Entry -> {
        val url = entry.url
        if (url.isNullOrEmpty()) throw WorkboxError("add-to-cache-list-unexpected-type", json("entry" to entry.source))
        if (!isTruthy(entry.revision)) {
            URL(url, location.href).href.let { CacheKey(it, it) }
        } else {
            val cacheKeyURL = URL(url, location.href)
            val originalURL = URL(url, location.href)
            cacheKeyURL.searchParams.set(REVISION_SEARCH_PARAM, entry.revision)
            CacheKey(cacheKeyURL.href, originalURL.href)
        }
    }
}

class InstallResult(val updatedURLs: List<String>, val notUpdatedURLs: List<String>)

class CleanupResult(val deletedURLs: List<String>)

/** `PrecacheController`. */
class PrecacheController(cacheName: String, plugins: List<WorkboxPlugin> = emptyList(), fallbackToNetwork: Boolean = true) {
    private val urlsToCacheKeys = LinkedHashMap<String, String>()
    private val urlsToCacheModes = LinkedHashMap<String, String>()
    private val cacheKeysToIntegrities = LinkedHashMap<String, String>()

    val strategy = PrecacheStrategy(cacheName, plugins + PrecacheCacheKeyPlugin(this), fallbackToNetwork)

    /** `addToCacheList(entries)`: entry by entry, so the entries before a failing one stay added.
     *  Like the production build it warns about revisionless entries. */
    fun addToCacheList(entries: List<PrecacheEntry>) {
        val urlsToWarnAbout = mutableListOf<String?>()
        for (entry in entries) {
            when (entry) {
                is PrecacheEntry.Url -> urlsToWarnAbout += entry.url
                is PrecacheEntry.Entry -> if (isUndefined(entry.revision)) urlsToWarnAbout += entry.url
                is PrecacheEntry.Falsy -> {}
            }
            val (cacheKey, url) = createCacheKey(entry)
            val cacheMode = if (entry is PrecacheEntry.Entry && isTruthy(entry.revision)) "reload" else "default"
            val firstEntry = urlsToCacheKeys[url]
            if (firstEntry != null && firstEntry != cacheKey) {
                throw WorkboxError("add-to-cache-list-conflicting-entries", json("firstEntry" to firstEntry, "secondEntry" to cacheKey))
            }
            val integrity = (entry as? PrecacheEntry.Entry)?.integrity
            if (!integrity.isNullOrEmpty()) {
                val known = cacheKeysToIntegrities[cacheKey]
                if (known != null && known != integrity) {
                    throw WorkboxError("add-to-cache-list-conflicting-integrities", json("url" to url))
                }
                cacheKeysToIntegrities[cacheKey] = integrity
            }
            urlsToCacheKeys[url] = cacheKey
            urlsToCacheModes[url] = cacheMode
            if (urlsToWarnAbout.isNotEmpty()) {
                val warningMessage = "Workbox is precaching URLs without revision info: ${urlsToWarnAbout.joinToString(", ") { it ?: "" }}" +
                    "\nThis is generally NOT safe. Learn more at https://bit.ly/wb-precache"
                console.warn(warningMessage)
            }
        }
    }

    /** `install(event)`: precaches each URL in turn through the strategy, under an install report
     *  plugin, and extends the event's lifetime until it is done. */
    fun install(event: ExtendableEvent): Promise<InstallResult> = waitUntil(event) {
        val installReportPlugin = PrecacheInstallReportPlugin()
        strategy.plugins += installReportPlugin
        // Steps run one at a time, so the map cannot change during an install.
        for ((url, cacheKey) in urlsToCacheKeys.entries.toList()) {
            val integrity = cacheKeysToIntegrities[cacheKey]
            val cacheMode = urlsToCacheModes[url]
            val request = Request(url, requestInit(integrity = integrity, cache = cacheMode, credentials = "same-origin"))
            val (responseDone, handlerDone) = strategy.handleAll(HandlerCallbackOptions(request, event, Params(cacheKey = cacheKey)))
            Promise.all(arrayOf<Promise<Any?>>(responseDone, handlerDone)).await()
        }
        InstallResult(installReportPlugin.updatedURLs.toList(), installReportPlugin.notUpdatedURLs.toList())
    }

    /** `activate(event)`: deletes every request in the precache whose URL is no current cache key. */
    fun activate(event: ExtendableEvent): Promise<CleanupResult> = waitUntil(event) {
        val cache = caches.open(strategy.cacheName).await()
        val currentlyCachedRequests = cache.keys().await()
        val expectedCacheKeys = urlsToCacheKeys.values.toSet()
        val deletedURLs = mutableListOf<String>()
        for (request in currentlyCachedRequests) {
            if (request.url !in expectedCacheKeys) {
                cache.delete(request).await()
                deletedURLs += request.url
            }
        }
        CleanupResult(deletedURLs)
    }

    fun getURLsToCacheKeys(): Map<String, String> = urlsToCacheKeys

    fun getCachedURLs(): List<String> = urlsToCacheKeys.keys.toList()

    /** `getCacheKeyForURL(url)`: the exact URL resolved against the location, without variations. */
    fun getCacheKeyForURL(url: String): String? = urlsToCacheKeys[URL(url, location.href).href]

    fun getIntegrityForCacheKey(cacheKey: String): String? = cacheKeysToIntegrities[cacheKey]

    /** `matchPrecache(request)`: opens the precache only for a URL in the manifest. */
    suspend fun matchPrecache(request: Request): Response? = matchPrecache(request.url)

    suspend fun matchPrecache(url: String): Response? {
        val cacheKey = getCacheKeyForURL(url) ?: return null
        return caches.open(strategy.cacheName).await().match(cacheKey).await()
    }

    /** `createHandlerBoundToURL(url)`: a handler that answers any request with this precached URL. */
    fun createHandlerBoundToURL(url: String): RouteHandler {
        val cacheKey = getCacheKeyForURL(url) ?: throw WorkboxError("non-precached-url", json("url" to url))
        return RouteHandler { options ->
            // `{cacheKey, ...options.params}`.
            val params = Params(options.params?.cacheKey ?: cacheKey, options.params?.integrity)
            strategy.handle(HandlerCallbackOptions(Request(url), options.event, params))
        }
    }
}

/** `PrecacheStrategy`: a Strategy that answers from the precache, populates it during install,
 *  and otherwise falls back to the network. */
class PrecacheStrategy(cacheName: String, plugins: List<WorkboxPlugin>, private val fallbackToNetwork: Boolean) : Strategy(cacheName, plugins) {
    init {
        this.plugins += copyRedirectedCacheableResponsesPlugin
    }

    override suspend fun _handle(request: Request, handler: StrategyHandler): Response {
        val response = handler.cacheMatch(request)
        if (response != null) return response
        return if (handler.event.type == "install") handleInstall(request, handler) else handleFetch(request, handler)
    }

    /** `_handleFetch`: the network, with the manifest's integrity unless the request is no-cors,
     *  and a cache repair only when that integrity vouches for the response. */
    private suspend fun handleFetch(request: Request, handler: StrategyHandler): Response {
        val params = handler.params ?: Params()
        if (!fallbackToNetwork) throw WorkboxError("missing-precache-entry", json("cacheName" to cacheName, "url" to request.url))
        val integrityInManifest = params.integrity
        val integrityInRequest = request.integrity
        val noIntegrityConflict = integrityInRequest.isEmpty() || integrityInRequest == integrityInManifest
        val integrity = if (request.mode != "no-cors") integrityInRequest.ifEmpty { integrityInManifest } else null
        val response = handler.fetch(Request(request, requestInit(integrity = integrity)))
        if (!integrityInManifest.isNullOrEmpty() && noIntegrityConflict && request.mode != "no-cors") {
            useDefaultCacheabilityPluginIfNeeded()
            handler.cachePut(request, response.clone())
        }
        return response
    }

    /** `_handleInstall`: fetch, then cache, and fail the install when the response may not be
     *  cached. */
    private suspend fun handleInstall(request: Request, handler: StrategyHandler): Response {
        useDefaultCacheabilityPluginIfNeeded()
        val response = handler.fetch(request)
        val wasCached = handler.cachePut(request, response.clone())
        if (!wasCached) throw WorkboxError("bad-precaching-response", json("url" to request.url, "status" to response.status))
        return response
    }

    /** `_useDefaultCacheabilityPluginIfNeeded`: exactly one cacheWillUpdate besides the redirect
     *  copy, adding the default one when there is none and dropping it when there are more. */
    private fun useDefaultCacheabilityPluginIfNeeded() {
        var defaultPluginIndex: Int? = null
        var cacheWillUpdatePluginCount = 0
        for ((index, plugin) in plugins.withIndex()) {
            if (plugin === copyRedirectedCacheableResponsesPlugin) continue
            if (plugin === defaultPrecacheCacheabilityPlugin) defaultPluginIndex = index
            if (plugin.cacheWillUpdate != null) cacheWillUpdatePluginCount++
        }
        if (cacheWillUpdatePluginCount == 0) {
            plugins += defaultPrecacheCacheabilityPlugin
        } else if (cacheWillUpdatePluginCount > 1 && defaultPluginIndex != null) {
            plugins.removeAt(defaultPluginIndex)
        }
    }

    companion object {
        /** Statuses below 400. */
        val defaultPrecacheCacheabilityPlugin = object : WorkboxPlugin {
            override val cacheWillUpdate: suspend (CacheWillUpdateParam) -> Response? = { param ->
                param.response.takeIf { it.status < 400 }
            }
        }

        /** A redirected response is cached as a copy without its redirect. */
        val copyRedirectedCacheableResponsesPlugin = object : WorkboxPlugin {
            override val cacheWillUpdate: suspend (CacheWillUpdateParam) -> Response? = { param ->
                if (param.response.redirected) copyResponse(param.response) else param.response
            }
        }
    }
}

/** `PrecacheCacheKeyPlugin`: the route's cache key, else the manifest's key for the exact URL. */
class PrecacheCacheKeyPlugin(private val precacheController: PrecacheController) : WorkboxPlugin {
    override val cacheKeyWillBeUsed: suspend (CacheKeyWillBeUsedParam) -> Request = { param ->
        val cacheKey = param.params?.cacheKey?.ifEmpty { null } ?: precacheController.getCacheKeyForURL(param.request.url)
        if (cacheKey != null) Request(cacheKey, requestInit(headers = param.request.headers)) else param.request
    }
}

/** `PrecacheInstallReportPlugin`: an install-time cache hit is not updated, a miss is. */
class PrecacheInstallReportPlugin : WorkboxPlugin {
    val updatedURLs = mutableListOf<String>()
    val notUpdatedURLs = mutableListOf<String>()

    override val handlerWillStart: suspend (HandlerWillStartParam) -> Unit = { param ->
        param.state?.set("originalRequest", param.request)
    }

    override val cachedResponseWillBeUsed: suspend (CachedResponseWillBeUsedParam) -> Response? = { param ->
        val originalRequest = param.state?.get("originalRequest") as? Request
        if (param.event.type == "install" && originalRequest != null) {
            if (param.cachedResponse != null) notUpdatedURLs += originalRequest.url else updatedURLs += originalRequest.url
        }
        param.cachedResponse
    }
}

/** `PrecacheFallbackPlugin`: a failed strategy answers with a precached URL, when it is there. */
class PrecacheFallbackPlugin(private val fallbackURL: String, private val precacheController: PrecacheController) : WorkboxPlugin {
    override val handlerDidError: suspend (HandlerDidErrorParam) -> Response? = {
        precacheController.matchPrecache(fallbackURL)
    }
}

/** `PrecacheRouteOptions`. A falsy `directoryIndex` or `cleanURLs` disables it. */
class PrecacheRouteOptions(
    val directoryIndex: String? = "index.html",
    val ignoreURLParametersMatching: List<RegExp> = listOf(RegExp("^utm_"), RegExp("^fbclid$")),
    val cleanURLs: Boolean = true,
    val urlManipulation: ((URL) -> List<URL>)? = null,
)

/** `removeIgnoredSearchParams(urlObject, ignoreURLParametersMatching)`, on the URL itself. */
private fun removeIgnoredSearchParams(urlObject: URL, ignoreURLParametersMatching: List<RegExp>): URL {
    for (paramName in urlObject.searchParams.names()) {
        if (ignoreURLParametersMatching.any { it.test(paramName) }) urlObject.searchParams.delete(paramName)
    }
    return urlObject
}

/** `generateURLVariations(url, options)`: a lazy generator, as Workbox's is, so a later variation
 *  (and `urlManipulation`) is computed only when the earlier ones missed. */
private fun generateURLVariations(url: String, options: PrecacheRouteOptions): Sequence<String> = sequence {
    val urlObject = URL(url, location.href)
    urlObject.hash = ""
    yield(urlObject.href)
    // The URL object itself loses its ignored parameters; urlManipulation sees it that way.
    val urlWithoutIgnoredParams = removeIgnoredSearchParams(urlObject, options.ignoreURLParametersMatching)
    yield(urlWithoutIgnoredParams.href)
    if (!options.directoryIndex.isNullOrEmpty() && urlWithoutIgnoredParams.pathname.endsWith("/")) {
        val directoryURL = URL(urlWithoutIgnoredParams.href)
        directoryURL.pathname += options.directoryIndex
        yield(directoryURL.href)
    }
    if (options.cleanURLs) {
        val cleanURL = URL(urlWithoutIgnoredParams.href)
        cleanURL.pathname += ".html"
        yield(cleanURL.href)
    }
    options.urlManipulation?.let { urlManipulation ->
        for (urlToAttempt in urlManipulation(urlObject)) yield(urlToAttempt.href)
    }
}

/** `new PrecacheRoute(controller, options)`: a GET route to the controller's strategy that matches
 *  the first variation of the request URL in the manifest, with `{cacheKey, integrity}`. */
class PrecacheRoute(precacheController: PrecacheController, routeOptions: PrecacheRouteOptions = PrecacheRouteOptions()) : Route(
    match = { options ->
        val urlsToCacheKeys = precacheController.getURLsToCacheKeys()
        generateURLVariations(options.request.url, routeOptions).firstNotNullOfOrNull { possibleURL ->
            urlsToCacheKeys[possibleURL]?.let { cacheKey -> Params(cacheKey, precacheController.getIntegrityForCacheKey(cacheKey)) }
        }
    },
    handler = precacheController.strategy,
)
