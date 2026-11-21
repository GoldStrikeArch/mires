// Entry of the Kotlin/JS port: what libraries/workbox/reference.mjs does, on the port's Workbox. One
// call runs one script of Workbox API calls (CONTRACT.md "Steps") with the fixture plugins, builds
// the canonical output, and deletes the script's caches on every path.
//
// The boundary: the corpus input is a JS object typed by external interfaces and read in place;
// manifest entries become `PrecacheEntry` values, with each revision kept as the caller's value.
// The output is built from JS objects and arrays in the contract's key order. `run` is a native ES
// module export through @JsExport.

import kotlin.js.Json
import kotlin.js.Promise
import kotlin.js.RegExp
import kotlin.js.json
import workbox.CacheFirst
import workbox.CacheOnly
import workbox.CacheWillUpdateParam
import workbox.CacheDidUpdateParam
import workbox.CacheKeyWillBeUsedParam
import workbox.CachedResponseWillBeUsedParam
import workbox.ExtendableEvent
import workbox.FetchDidFailParam
import workbox.FetchDidSucceedParam
import workbox.HandlerDidCompleteParam
import workbox.HandlerDidErrorParam
import workbox.HandlerDidRespondParam
import workbox.HandlerWillRespondParam
import workbox.HandlerWillStartParam
import workbox.HostError
import workbox.JsError
import workbox.JsObject
import workbox.JsPromise
import workbox.JsTypeError
import workbox.MatchCallbackOptions
import workbox.NetworkFirst
import workbox.NetworkOnly
import workbox.PluginState
import workbox.PrecacheController
import workbox.PrecacheEntry
import workbox.PrecacheFallbackPlugin
import workbox.PrecacheRoute
import workbox.PrecacheRouteOptions
import workbox.Request
import workbox.RequestWillFetchParam
import workbox.Response
import workbox.Route
import workbox.RouteHandler
import workbox.Router
import workbox.URL
import workbox.WorkboxError
import workbox.WorkboxPlugin
import workbox.await
import workbox.caches
import workbox.crypto
import workbox.isError
import workbox.isTruthy
import workbox.isUndefined
import workbox.jsName
import workbox.jsString
import workbox.launch
import workbox.location
import workbox.requestInit

private const val FIXTURE = "experiments/library-matrix/libraries/workbox/"

/** Well-formed SRI metadata that matches no fixture, so the fetch fails integrity. */
private const val WRONG_INTEGRITY = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="

external interface NativeEvent {
    val type: String
    fun waitUntil(promise: Promise<*>)
}

external interface PluginSpec {
    val kind: String
    val label: String
    val on: String
    val value: Any?
    val url: String
    val controller: String
}

external interface HandlerSpec {
    val strategy: String?
    val cacheName: String
    val plugins: Array<PluginSpec>?
    val precache: String?
    val bound: String?
    val controller: String
    val respond: Any?
    val status: Any?
}

external interface RouteOptionsSpec {
    val directoryIndex: Any?
    val cleanURLs: Any?
    val ignoreURLParametersMatching: Array<String>?
    val urlManipulation: Array<String>?
}

external interface Step {
    val op: String
    val id: String?
    val source: String
    val cacheName: String
    val plugins: Array<PluginSpec>?
    val fallbackToNetwork: Boolean?
    val controller: String
    val entries: Array<Any?>
    val router: String
    val route: String?
    val options: RouteOptionsSpec?
    val prefix: String
    val crossOrigin: Boolean?
    val handler: HandlerSpec
    val method: String?
    val url: String?
    val mode: String?
    val integrity: String?
    val request: Any?
}

external interface Input {
    val steps: Array<Step>
    val cachePrefix: String?
    val event: NativeEvent?
    val request: Request?
}

/** One handler's trace: the trace plugin's label, the request and the records of its callbacks. */
private class HandlerTrace(val label: String, val url: String) {
    val log = mutableListOf<Array<Any?>>()
}

/** The stand-in ExtendableEvent of one event-driven step: it collects its lifetime promises. */
private class StandInEvent(override val type: String) : ExtendableEvent {
    val pending = mutableListOf<Promise<*>>()

    override fun waitUntil(promise: Promise<*>) {
        pending += promise
    }
}

/** A native event the caller supplied (service-worker mode). */
private class HostEvent(private val event: NativeEvent) : ExtendableEvent {
    override val type: String get() = event.type

    override fun waitUntil(promise: Promise<*>) = event.waitUntil(promise)
}

/** A response as a followed redirect leaves it: it and every clone of it report redirected. */
private fun redirected(response: Response): Response {
    val clone = response.asDynamic().clone.bind(response)
    JsObject.defineProperty(response, "redirected", json("value" to true))
    JsObject.defineProperty(response, "clone", json("value" to { redirected(clone().unsafeCast<Response>()) }))
    return response
}

private fun status(response: Response?): Int? = response?.status

/** An Error's name; a thrown non-Error (the fail fixture's `value`) prints as its string. */
private fun nameOf(error: Throwable): String = when (error) {
    is WorkboxError -> error.name
    is HostError -> jsString(error.value)
    else -> error.jsName
}

/** A manifest entry from the corpus: falsy, a string, or a `PrecacheEntry` object. */
private fun precacheEntry(value: Any?): PrecacheEntry = when {
    !isTruthy(value) -> PrecacheEntry.Falsy(value)
    value is String -> PrecacheEntry.Url(value)
    else -> {
        val entry = value.asDynamic()
        PrecacheEntry.Entry(entry.url.unsafeCast<String?>(), entry.revision, entry.integrity.unsafeCast<String?>(), value!!)
    }
}

private class Scenario(private val input: Input) {
    private val base = URL("./", location.href).href
    private val prefix = input.cachePrefix ?: ("mires-workbox-" + crypto.randomUUID())
    private val owned = input.cachePrefix == null
    private var source = "alpha"
    private val controllers = HashMap<String, PrecacheController>()
    private val routers = HashMap<String, Router>()
    private val routes = HashMap<String, Route>()
    private val results = mutableListOf<Any?>()
    private val events = mutableListOf<Array<Any>>()
    private val handlers = mutableListOf<HandlerTrace>()

    private fun relative(url: String) = url.removePrefix(base)

    private fun cacheName(name: String) = "$prefix-$name"

    private fun cacheLabel(name: String) = name.removePrefix("$prefix-")

    /** Runs every step, reports, and deletes the scenario's caches whatever happened. */
    suspend fun run(): Json {
        try {
            for (step in input.steps) {
                val result = try {
                    step(step)
                } catch (error: Throwable) {
                    failure(error)
                }
                results.add(result)
            }
            val traces = handlers.map { arrayOf(it.label, it.url, it.log.toTypedArray()) }
            return json("results" to results.toTypedArray(), "events" to events.toTypedArray(), "handlers" to traces.toTypedArray(), "caches" to dump())
        } finally {
            if (owned) {
                for (name in caches.keys().await()) if (name.startsWith("$prefix-")) caches.delete(name).await()
            }
        }
    }

    private suspend fun step(step: Step): Any? = when (step.op) {
        "network" -> {
            source = step.source
            null
        }
        "controller" -> {
            controllers[step.id!!] = PrecacheController(cacheName(step.cacheName), plugins(step.plugins), step.fallbackToNetwork != false)
            null
        }
        "addToCacheList" -> {
            controller(step.controller).addToCacheList(step.entries.map(::precacheEntry))
            null
        }
        "addPlugins" -> {
            controller(step.controller).strategy.plugins += step.plugins.orEmpty().map(::plugin)
            null
        }
        "install" -> {
            val result = lifetime("install") { event -> controller(step.controller).install(event).await() }
            json("updatedURLs" to urls(result.updatedURLs), "notUpdatedURLs" to urls(result.notUpdatedURLs))
        }
        "activate" -> {
            val result = lifetime("activate") { event -> controller(step.controller).activate(event).await() }
            json("deletedURLs" to urls(result.deletedURLs))
        }
        "getCachedURLs" -> urls(controller(step.controller).getCachedURLs())
        "getURLsToCacheKeys" -> controller(step.controller).getURLsToCacheKeys().map { (url, key) -> arrayOf(relative(url), relative(key)) }.toTypedArray()
        "getCacheKeyForURL" -> controller(step.controller).getCacheKeyForURL(step.url!!)?.let(::relative)
        "matchPrecache" -> {
            val precacheController = controller(step.controller)
            val response = if (isTruthy(step.request)) precacheController.matchPrecache(Request(step.url!!)) else precacheController.matchPrecache(step.url!!)
            response?.let { record(it) }
        }
        "router" -> {
            routers[step.id!!] = Router()
            null
        }
        "precacheRoute" -> register(step, PrecacheRoute(controller(step.controller), routeOptions(step.options)))
        "route" -> {
            val match: (MatchCallbackOptions) -> Any? =
                if (step.crossOrigin == true) { options -> !options.sameOrigin } else { options -> relative(options.url.href).startsWith(step.prefix) }
            register(step, Route(match, handler(step.handler), step.method ?: "GET"))
        }
        "defaultHandler" -> {
            router(step.router).setDefaultHandler(handler(step.handler), step.method ?: "GET")
            null
        }
        "catchHandler" -> {
            val route = step.route
            if (route == null) router(step.router).setCatchHandler(handler(step.handler)) else routes.getValue(route).setCatchHandler(handler(step.handler))
            null
        }
        "unregisterRoute" -> {
            router(step.router).unregisterRoute(routes.getValue(step.route!!))
            null
        }
        "fetch" -> {
            val url = step.url
            val request = if (url == null) input.request!! else Request(url, requestInit(method = step.method, mode = step.mode, integrity = step.integrity))
            val response = lifetime("fetch") { event -> router(step.router).handleRequest(request, event)?.await() }
            response?.let { record(it) }
        }
        "caches" -> dump()
        else -> throw JsTypeError("unknown step " + step.op).unsafeCast<Throwable>()
    }

    private fun controller(id: String) = controllers.getValue(id)

    private fun router(id: String) = routers.getValue(id)

    private fun register(step: Step, route: Route): Nothing? {
        step.id?.let { routes[it] = route }
        router(step.router).registerRoute(route)
        return null
    }

    private fun urls(urls: List<String>) = urls.map(::relative).toTypedArray()

    /** The network fixture, then the plugins the script names. */
    private fun plugins(specs: Array<PluginSpec>?): List<WorkboxPlugin> = listOf(NetworkPlugin()) + specs.orEmpty().map(::plugin)

    private fun plugin(spec: PluginSpec): WorkboxPlugin = when (spec.kind) {
        "trace" -> TracePlugin(spec.label)
        "keep" -> object : WorkboxPlugin {
            override val cacheWillUpdate: suspend (CacheWillUpdateParam) -> Response? = { it.response }
        }
        "drop" -> object : WorkboxPlugin {
            override val cacheWillUpdate: suspend (CacheWillUpdateParam) -> Response? = { null }
        }
        "fail" -> FailPlugin(spec.on, spec.value)
        "fallback" -> PrecacheFallbackPlugin(spec.url, controller(spec.controller))
        else -> throw JsTypeError("unknown plugin " + spec.kind).unsafeCast<Throwable>()
    }

    /** A route handler: a strategy, a controller's precache strategy, a handler bound to a
     *  precached URL, or an async function answering a fixed response. */
    private fun handler(spec: HandlerSpec): RouteHandler {
        spec.strategy?.let { name ->
            val cacheName = cacheName(spec.cacheName)
            val plugins = plugins(spec.plugins)
            return when (name) {
                "CacheFirst" -> CacheFirst(cacheName, plugins)
                "CacheOnly" -> CacheOnly(cacheName, plugins)
                "NetworkFirst" -> NetworkFirst(cacheName, plugins)
                "NetworkOnly" -> NetworkOnly(cacheName, plugins)
                else -> throw JsTypeError("unknown strategy $name").unsafeCast<Throwable>()
            }
        }
        spec.precache?.let { return controller(it).strategy }
        spec.bound?.let { return controller(spec.controller).createHandlerBoundToURL(it) }
        return RouteHandler { launch { Response(spec.respond, json("status" to spec.status)) } }
    }

    /** PrecacheRoute options as given: an absent member keeps the default, `null` disables
     *  `directoryIndex` and `cleanURLs`. */
    private fun routeOptions(options: RouteOptionsSpec?): PrecacheRouteOptions {
        if (options == null) return PrecacheRouteOptions()
        val defaults = PrecacheRouteOptions()
        val suffixes = options.urlManipulation
        return PrecacheRouteOptions(
            directoryIndex = if (isUndefined(options.directoryIndex)) defaults.directoryIndex else options.directoryIndex.unsafeCast<String?>(),
            ignoreURLParametersMatching = options.ignoreURLParametersMatching?.map { RegExp(it) } ?: defaults.ignoreURLParametersMatching,
            cleanURLs = if (isUndefined(options.cleanURLs)) defaults.cleanURLs else isTruthy(options.cleanURLs),
            urlManipulation = suffixes?.let { { url: URL -> suffixes.map { suffix -> URL(url.href).apply { pathname += suffix } } } },
        )
    }

    /** Runs one event-driven step under a fresh stand-in event and waits for every lifetime it was
     *  given, in rounds, or, in service-worker mode, under the caller's native event. */
    private suspend fun <T> lifetime(type: String, action: suspend (ExtendableEvent) -> T): T {
        input.event?.let { return action(HostEvent(it)) }
        val event = StandInEvent(type)
        var count = 0
        try {
            return action(event)
        } finally {
            while (event.pending.isNotEmpty()) {
                val batch = event.pending.toTypedArray()
                event.pending.clear()
                count += batch.size
                JsPromise.allSettled(batch).await()
            }
            events.add(arrayOf(type, count))
        }
    }

    /** `{status, url, body}` of a response, reading its body. */
    private suspend fun record(response: Response): Json {
        val status = response.status
        val url = relative(response.url)
        return json("status" to status, "url" to url, "body" to response.text().await())
    }

    /** `{error, message}` for a WorkboxError, `{error}` for another Error, `{thrown}` for a thrown
     *  non-Error. */
    private fun failure(error: Throwable): Json = when {
        error is WorkboxError -> json("error" to error.name, "message" to error.message!!.replace(base, "").replace("$prefix-", ""))
        error.isError -> json("error" to error.jsName)
        else -> json("thrown" to nameOf(error))
    }

    /** Every cache of the scenario, in CacheStorage order, with each entry's response record. */
    private suspend fun dump(): Array<Any> {
        val out = mutableListOf<Array<Any>>()
        for (name in caches.keys().await()) {
            if (!name.startsWith("$prefix-")) continue
            val cache = caches.open(name).await()
            val entries = mutableListOf<Array<Any>>()
            for (request in cache.keys().await()) entries.add(arrayOf(relative(request.url), record(cache.match(request).await()!!)))
            out.add(arrayOf(cacheLabel(name), entries.toTypedArray()))
        }
        return out.toTypedArray()
    }

    /** The network fixture: every fetch goes to one of the two fixture files, or fails. */
    private inner class NetworkPlugin : WorkboxPlugin {
        override val requestWillFetch: suspend (RequestWillFetchParam) -> Request = { param ->
            if (source == "offline") throw JsTypeError("offline").unsafeCast<Throwable>()
            val target = URL(FIXTURE + (if (source == "beta") "beta" else "alpha") + ".mjs", location.href).href
            Request(target, requestInit(integrity = if (source == "broken") WRONG_INTEGRITY else param.request.integrity))
        }

        override val fetchDidSucceed: suspend (FetchDidSucceedParam) -> Response = { param ->
            when (source) {
                "missing" -> Response("missing", json("status" to 404))
                "redirected" -> redirected(param.response)
                else -> param.response
            }
        }
    }

    /** The trace plugin: every lifecycle callback but cacheWillUpdate, recorded per handler. A
     *  callback records only into a log its own handlerWillStart created: a plugin before it can
     *  fail that callback. */
    private inner class TracePlugin(private val label: String) : WorkboxPlugin {
        private fun record(state: PluginState?, vararg entry: Any?) {
            (state?.get("log") as? HandlerTrace)?.log?.add(arrayOf(*entry))
        }

        override val handlerWillStart: suspend (HandlerWillStartParam) -> Unit = { param ->
            val trace = HandlerTrace(label, relative(param.request.url))
            param.state?.set("log", trace)
            handlers += trace
        }
        override val cacheKeyWillBeUsed: suspend (CacheKeyWillBeUsedParam) -> Request = { param ->
            record(param.state, "cacheKeyWillBeUsed", param.mode, relative(param.request.url))
            param.request
        }
        override val cachedResponseWillBeUsed: suspend (CachedResponseWillBeUsedParam) -> Response? = { param ->
            record(param.state, "cachedResponseWillBeUsed", cacheLabel(param.cacheName), relative(param.request.url), status(param.cachedResponse))
            param.cachedResponse
        }
        override val requestWillFetch: suspend (RequestWillFetchParam) -> Request = { param ->
            record(param.state, "requestWillFetch", relative(param.request.url))
            param.request
        }
        override val fetchDidSucceed: suspend (FetchDidSucceedParam) -> Response = { param ->
            record(param.state, "fetchDidSucceed", relative(param.request.url), param.response.status)
            param.response
        }
        override val fetchDidFail: suspend (FetchDidFailParam) -> Unit = { param ->
            record(param.state, "fetchDidFail", relative(param.originalRequest.url), relative(param.request.url), nameOf(param.error))
        }
        override val cacheDidUpdate: suspend (CacheDidUpdateParam) -> Unit = { param ->
            record(param.state, "cacheDidUpdate", cacheLabel(param.cacheName), relative(param.request.url), status(param.oldResponse), param.newResponse.status)
        }
        override val handlerWillRespond: suspend (HandlerWillRespondParam) -> Response = { param ->
            record(param.state, "handlerWillRespond", param.response.status)
            param.response
        }
        override val handlerDidRespond: suspend (HandlerDidRespondParam) -> Unit = { param ->
            record(param.state, "handlerDidRespond", status(param.response))
        }
        override val handlerDidComplete: suspend (HandlerDidCompleteParam) -> Unit = { param ->
            record(param.state, "handlerDidComplete", status(param.response), param.error?.let(::nameOf))
        }
        override val handlerDidError: suspend (HandlerDidErrorParam) -> Response? = { param ->
            record(param.state, "handlerDidError", nameOf(param.error))
            null
        }
    }
}

/** A plugin whose one callback, the one named [on], throws `new Error('fail')`, or, with a
 *  [value], that value itself (a non-Error). */
private class FailPlugin(private val on: String, private val value: Any?) : WorkboxPlugin {
    private fun <P, R> failing(name: String): (suspend (P) -> R)? =
        if (name != on) null else { _ -> throw if (isUndefined(value)) JsError("fail").unsafeCast<Throwable>() else HostError(value) }

    override val handlerWillStart = failing<HandlerWillStartParam, Unit>("handlerWillStart")
    override val cacheKeyWillBeUsed = failing<CacheKeyWillBeUsedParam, Request>("cacheKeyWillBeUsed")
    override val cachedResponseWillBeUsed = failing<CachedResponseWillBeUsedParam, Response?>("cachedResponseWillBeUsed")
    override val cacheWillUpdate = failing<CacheWillUpdateParam, Response?>("cacheWillUpdate")
    override val cacheDidUpdate = failing<CacheDidUpdateParam, Unit>("cacheDidUpdate")
    override val requestWillFetch = failing<RequestWillFetchParam, Request>("requestWillFetch")
    override val fetchDidSucceed = failing<FetchDidSucceedParam, Response>("fetchDidSucceed")
    override val fetchDidFail = failing<FetchDidFailParam, Unit>("fetchDidFail")
    override val handlerWillRespond = failing<HandlerWillRespondParam, Response>("handlerWillRespond")
    override val handlerDidRespond = failing<HandlerDidRespondParam, Unit>("handlerDidRespond")
    override val handlerDidComplete = failing<HandlerDidCompleteParam, Unit>("handlerDidComplete")
    override val handlerDidError = failing<HandlerDidErrorParam, Response?>("handlerDidError")
}

/** Runs one corpus script and resolves with its output. */
@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: Input): Promise<Json> = launch { Scenario(input).run() }
