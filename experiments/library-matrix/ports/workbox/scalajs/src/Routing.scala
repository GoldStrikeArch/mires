// workbox-routing: Route and Router (registration per method, the http(s) guard, matching with
// parameter normalisation, default handlers, route and global catch handlers).

package workbox

import scala.collection.mutable
import scala.concurrent.Future
import scala.scalajs.js

/** What a route's match callback sees. */
final case class MatchCallbackOptions(url: URL, sameOrigin: Boolean, request: Request, event: ExtendableEvent)

/** `new Route(match, handler, method)`. A match answers None for no match, or the params; a match
  * without params (`true`, `{}`) answers empty `Params`, which the router normalises away.
  */
class Route(val `match`: MatchCallbackOptions => Option[Params], val handler: RouteHandler, val method: String = "GET"):
  private var routeCatchHandler: Option[RouteHandler] = None

  def catchHandler: Option[RouteHandler] = routeCatchHandler

  def setCatchHandler(handler: RouteHandler): Unit = routeCatchHandler = Some(handler)

/** `Router`. */
final class Router:
  private val routes = mutable.LinkedHashMap.empty[String, mutable.ArrayBuffer[Route]]
  private val defaultHandlerMap = mutable.LinkedHashMap.empty[String, RouteHandler]
  private var catchHandler: Option[RouteHandler] = None

  /** `handleRequest({request, event})`: None when the request is not http(s) or nothing handles
    * it, else the response promise, with the catch handlers chained when there are any.
    */
  def handleRequest(request: Request, event: ExtendableEvent): Option[Future[Response]] =
    val url = URL(request.url, Globals.location.href)
    if !url.protocol.startsWith("http") then return None
    val sameOrigin = url.origin == Globals.location.origin
    val (route, params) = findMatchingRoute(MatchCallbackOptions(url, sameOrigin, request, event))
    route.map(_.handler).orElse(defaultHandlerMap.get(request.method)).map { handler =>
      val responsePromise = async(handler.handle(HandlerCallbackOptions(request, event, params)))
      // Every handler in the subset answers a promise, so only the catch handlers decide.
      val routeCatchHandler = route.flatMap(_.catchHandler)
      if catchHandler.isEmpty && routeCatchHandler.isEmpty then responsePromise
      else
        responsePromise.recoverWith { case error =>
          val routeCaught = routeCatchHandler match
            case Some(routeCatchHandler) =>
              async(routeCatchHandler.handle(HandlerCallbackOptions(request, event, params))).map(Right(_)).recover {
                case catchErr if catchErr.isError => Left(catchErr)
                case _ => Left(error)
              }
            case None => Future.successful(Left(error))
          routeCaught.flatMap {
            case Right(response) => Future.successful(response)
            // The global catch handler is the one set when the rejection arrives; it gets no params.
            case Left(error) => catchHandler.fold(Future.failed(error))(handler => handler.handle(HandlerCallbackOptions(request, event)))
          }
        }
    }

  /** `findMatchingRoute`: the first route for the request's method that matches. Empty params (a
    * match of `true` or `{}`) become none.
    */
  private def findMatchingRoute(options: MatchCallbackOptions): (Option[Route], Option[Params]) =
    routes.getOrElse(options.request.method, mutable.ArrayBuffer.empty).iterator
      .flatMap(route => route.`match`(options).map(params => (Some(route), Option(params).filter(_ != Params()))))
      .nextOption()
      .getOrElse((None, None))

  def setDefaultHandler(handler: RouteHandler, method: String = "GET"): Unit = defaultHandlerMap(method) = handler

  def setCatchHandler(handler: RouteHandler): Unit = catchHandler = Some(handler)

  def registerRoute(route: Route): Unit = routes.getOrElseUpdate(route.method, mutable.ArrayBuffer.empty) += route

  def unregisterRoute(route: Route): Unit =
    val list = routes.getOrElse(
      route.method,
      throw WorkboxError("unregister-route-but-not-found-with-method", js.Dynamic.literal(method = route.method)),
    )
    val routeIndex = list.indexWhere(_ eq route)
    if routeIndex < 0 then throw WorkboxError("unregister-route-route-not-registered")
    list.remove(routeIndex)
