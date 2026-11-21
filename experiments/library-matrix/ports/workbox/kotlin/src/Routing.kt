// workbox-routing: Route and Router (registration per method, the http(s) guard, matching with
// parameter normalisation, default handlers, route and global catch handlers).

package workbox

import kotlin.js.Promise
import kotlin.js.json

/** What a route's match callback sees. */
class MatchCallbackOptions(val url: URL, val sameOrigin: Boolean, val request: Request, val event: ExtendableEvent)

/** `new Route(match, handler, method)`. A match answers a falsy value (null or false) for no
 *  match, `true` for a match without parameters, or the [Params] the handler receives. */
open class Route(val match: (MatchCallbackOptions) -> Any?, val handler: RouteHandler, val method: String = "GET") {
    var catchHandler: RouteHandler? = null
        private set

    fun setCatchHandler(handler: RouteHandler) {
        catchHandler = handler
    }
}

/** `Router`. */
class Router {
    private val routes = LinkedHashMap<String, MutableList<Route>>()
    private val defaultHandlerMap = LinkedHashMap<String, RouteHandler>()
    private var catchHandler: RouteHandler? = null

    /** `handleRequest({request, event})`: null when the request is not http(s) or nothing handles
     *  it, else the response promise, with the catch handlers chained when there are any. */
    fun handleRequest(request: Request, event: ExtendableEvent): Promise<Response>? {
        val url = URL(request.url, location.href)
        if (!url.protocol.startsWith("http")) return null
        val sameOrigin = url.origin == location.origin
        val (params, route) = findMatchingRoute(MatchCallbackOptions(url, sameOrigin, request, event))
        val method = request.method
        val handler = route?.handler ?: defaultHandlerMap[method] ?: return null
        var responsePromise = try {
            handler.handle(HandlerCallbackOptions(request, event, params))
        } catch (error: Throwable) {
            Promise.reject(error)
        }
        // Every handler in the subset answers a Promise, so only the catch handlers decide.
        val routeCatchHandler = route?.catchHandler
        if (catchHandler != null || routeCatchHandler != null) {
            val handled = responsePromise
            responsePromise = launch {
                try {
                    handled.await()
                } catch (caught: Throwable) {
                    var error = caught
                    if (routeCatchHandler != null) {
                        try {
                            return@launch routeCatchHandler.handle(HandlerCallbackOptions(request, event, params)).await()
                        } catch (catchErr: Throwable) {
                            if (catchErr.isError) error = catchErr
                        }
                    }
                    // The global catch handler is the one set when the rejection arrives; it gets
                    // no params.
                    catchHandler?.let { return@launch it.handle(HandlerCallbackOptions(request, event)).await() }
                    throw error
                }
            }
        }
        return responsePromise
    }

    /** `findMatchingRoute`: the first route for the request's method that matches. A match of
     *  `true` carries no params. */
    private fun findMatchingRoute(options: MatchCallbackOptions): Pair<Params?, Route?> {
        for (route in routes[options.request.method].orEmpty()) {
            val matchResult = route.match(options)
            if (matchResult != null && matchResult != false) return (matchResult as? Params) to route
        }
        return null to null
    }

    fun setDefaultHandler(handler: RouteHandler, method: String = "GET") {
        defaultHandlerMap[method] = handler
    }

    fun setCatchHandler(handler: RouteHandler) {
        catchHandler = handler
    }

    fun registerRoute(route: Route) {
        routes.getOrPut(route.method) { mutableListOf() } += route
    }

    fun unregisterRoute(route: Route) {
        val list = routes[route.method]
            ?: throw WorkboxError("unregister-route-but-not-found-with-method", json("method" to route.method))
        val routeIndex = list.indexOfFirst { it === route }
        if (routeIndex < 0) throw WorkboxError("unregister-route-route-not-registered")
        list.removeAt(routeIndex)
    }
}
