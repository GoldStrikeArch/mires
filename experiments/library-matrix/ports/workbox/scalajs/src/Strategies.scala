// workbox-strategies: the plugin interface, Strategy with its handle/handleAll lifecycle,
// StrategyHandler, and the CacheFirst, CacheOnly, NetworkFirst and NetworkOnly strategies.

package workbox

import scala.collection.mutable
import scala.concurrent.{Future, Promise}
import scala.scalajs.js
import scala.scalajs.js.JSConverters.*
import scala.util.{Failure, Success}

/** The state object a handler keeps for one plugin (`state` in every callback), as Workbox's
  * `MapLikeObject`.
  */
type PluginState = mutable.Map[String, Any]

/** The route parameters a handler receives: `{cacheKey, integrity}` from a PrecacheRoute match,
  * `{cacheKey}` from createHandlerBoundToURL.
  */
final case class Params(cacheKey: Option[String] = None, integrity: Option[String] = None)

final case class HandlerWillStartParam(request: Request, event: ExtendableEvent, state: Option[PluginState])
final case class CacheKeyWillBeUsedParam(mode: String, request: Request, event: ExtendableEvent, params: Option[Params], state: Option[PluginState])
final case class CachedResponseWillBeUsedParam(cacheName: String, request: Request, cachedResponse: Option[Response], event: ExtendableEvent, state: Option[PluginState])
final case class CacheWillUpdateParam(request: Request, response: Response, event: ExtendableEvent, state: Option[PluginState])
final case class CacheDidUpdateParam(cacheName: String, oldResponse: Option[Response], newResponse: Response, request: Request, event: ExtendableEvent, state: Option[PluginState])
final case class RequestWillFetchParam(request: Request, event: ExtendableEvent, state: Option[PluginState])
final case class FetchDidSucceedParam(request: Request, response: Response, event: ExtendableEvent, state: Option[PluginState])
final case class FetchDidFailParam(error: Throwable, originalRequest: Request, request: Request, event: ExtendableEvent, state: Option[PluginState])
final case class HandlerWillRespondParam(request: Request, response: Response, event: ExtendableEvent, state: Option[PluginState])
final case class HandlerDidRespondParam(request: Request, response: Option[Response], event: ExtendableEvent, state: Option[PluginState])
final case class HandlerDidCompleteParam(request: Request, response: Option[Response], error: Option[Throwable], event: ExtendableEvent, state: Option[PluginState])
final case class HandlerDidErrorParam(request: Request, error: Throwable, event: ExtendableEvent, state: Option[PluginState])

type Callback[P, R] = P => Future[R]

/** `WorkboxPlugin`: an object of optional lifecycle callbacks. A plugin defines a callback by
  * overriding it with `Some`; Workbox calls only the callbacks a plugin defines.
  */
trait WorkboxPlugin:
  def handlerWillStart: Option[Callback[HandlerWillStartParam, Unit]] = None
  def cacheKeyWillBeUsed: Option[Callback[CacheKeyWillBeUsedParam, Request]] = None
  def cachedResponseWillBeUsed: Option[Callback[CachedResponseWillBeUsedParam, Option[Response]]] = None
  def cacheWillUpdate: Option[Callback[CacheWillUpdateParam, Option[Response]]] = None
  def cacheDidUpdate: Option[Callback[CacheDidUpdateParam, Unit]] = None
  def requestWillFetch: Option[Callback[RequestWillFetchParam, Request]] = None
  def fetchDidSucceed: Option[Callback[FetchDidSucceedParam, Response]] = None
  def fetchDidFail: Option[Callback[FetchDidFailParam, Unit]] = None
  def handlerWillRespond: Option[Callback[HandlerWillRespondParam, Response]] = None
  def handlerDidRespond: Option[Callback[HandlerDidRespondParam, Unit]] = None
  def handlerDidComplete: Option[Callback[HandlerDidCompleteParam, Unit]] = None
  def handlerDidError: Option[Callback[HandlerDidErrorParam, Option[Response]]] = None

/** `HandlerCallbackOptions`: what a route handler is called with. */
final case class HandlerCallbackOptions(request: Request, event: ExtendableEvent, params: Option[Params] = None)

/** A route handler as `normalizeHandler` leaves it: an object with `handle`. */
trait RouteHandler:
  def handle(options: HandlerCallbackOptions): Future[Response]

/** `Strategy`: a cache name, a mutable plugin list and the subclass's `_handle`. The cache name is
  * always supplied (`cacheNames.getRuntimeName(cacheName)` returns it unchanged).
  */
abstract class Strategy(val cacheName: String, initialPlugins: Seq[WorkboxPlugin]) extends RouteHandler:
  val plugins: mutable.ArrayBuffer[WorkboxPlugin] = mutable.ArrayBuffer.from(initialPlugins)

  /** `handle(options)`: the response promise of `handleAll`; the completion keeps running. */
  def handle(options: HandlerCallbackOptions): Future[Response] = handleAll(options)._1

  /** `handleAll(options)`: a handler for the request, and its response and completion promises,
    * both already running.
    */
  def handleAll(options: HandlerCallbackOptions): (Future[Response], Future[Unit]) =
    val handler = StrategyHandler(this, options)
    val responseDone = async(getResponse(handler, options.request, options.event))
    val handlerDone = async(awaitComplete(responseDone, handler, options.request, options.event))
    (responseDone, handlerDone)

  /** The subclass's `_handle(request, handler)`. */
  protected def _handle(request: Request, handler: StrategyHandler): Future[Response]

  /** `_getResponse`: handlerWillStart, `_handle`, handlerDidError on failure, handlerWillRespond. */
  private def getResponse(handler: StrategyHandler, request: Request, event: ExtendableEvent): Future[Response] =
    for
      _ <- handler.runCallbacks(_.handlerWillStart)((callback, state) => callback(HandlerWillStartParam(request, event, state)))
      response <- handleOrRecover(handler, request, event)
      response <- handler.foldCallbacks(_.handlerWillRespond, response) { (callback, state, response) =>
        callback(HandlerWillRespondParam(request, response, event, state))
      }
    yield response

  /** The `try` of `_getResponse`: `_handle`, and on a thrown Error the handlerDidError callbacks,
    * each result replacing the response until one is a response. A response of type "error" stays
    * the response unless a callback replaces it, so it is returned when none does.
    */
  private def handleOrRecover(handler: StrategyHandler, request: Request, event: ExtendableEvent): Future[Response] =
    var response: Option[Response] = None
    async(_handle(request, handler))
      .flatMap { handled =>
        response = Some(handled)
        if handled.responseType == "error" then Future.failed(WorkboxError("no-response", js.Dynamic.literal(url = request.url)))
        else Future.successful(handled)
      }
      .recoverWith { case error =>
        val recovered =
          if error.isError then
            handler.foldCallbacks(_.handlerDidError, response, until = (_: Option[Response]).isDefined) { (callback, state, _) =>
              callback(HandlerDidErrorParam(request, error, event, state))
            }
          else Future.successful(response)
        recovered.flatMap(_.fold(Future.failed(error))(Future.successful))
      }

  /** `_awaitComplete`: handlerDidRespond and the handler's extended lifetimes, then
    * handlerDidComplete with the Error among them, then the handler's own promise resolves.
    */
  private def awaitComplete(responseDone: Future[Response], handler: StrategyHandler, request: Request, event: ExtendableEvent): Future[Unit] =
    for
      response <- responseDone.transform(outcome => Success(outcome.toOption))
      waited = for
        _ <- handler.runCallbacks(_.handlerDidRespond)((callback, state) => callback(HandlerDidRespondParam(request, response, event, state)))
        _ <- handler.doneWaiting()
      yield ()
      error <- waited.transform(outcome => Success(outcome.failed.toOption.filter(_.isError)))
      _ <- handler.runCallbacks(_.handlerDidComplete)((callback, state) => callback(HandlerDidCompleteParam(request, response, error, event, state)))
      _ = handler.destroy()
      _ <- error.fold(Future.unit)(Future.failed)
    yield ()

/** `StrategyHandler`: one request's run through a strategy. */
final class StrategyHandler(strategy: Strategy, options: HandlerCallbackOptions):
  val request: Request = options.request
  val event: ExtendableEvent = options.event
  val params: Option[Params] = options.params
  private val cacheKeys = mutable.HashMap.empty[String, Request]
  private val handlerDeferred = Promise[Null]()
  private val extendLifetimePromises = mutable.ArrayBuffer.empty[Future[Any]]

  // The plugins present now get a state; a plugin added later is called without one.
  private val pluginStateMap: Seq[(WorkboxPlugin, PluginState)] = strategy.plugins.toSeq.map(_ -> mutable.HashMap.empty[String, Any])

  event.waitUntil(handlerDeferred.future)

  /** `fetch(input)`: requestWillFetch, the network, then fetchDidSucceed or fetchDidFail. */
  def fetch(input: Request): Future[Response] =
    var request = input
    val originalRequest = Option.when(hasCallback(_.fetchDidFail))(request.copy())
    val requestWillFetchDone = runCallbacks(_.requestWillFetch) { (callback, state) =>
      callback(RequestWillFetchParam(request.copy(), event, state)).map(request = _)
    }.recoverWith {
      case error if error.isError =>
        Future.failed(WorkboxError("plugin-error-request-will-fetch", js.Dynamic.literal(thrownErrorMessage = error.jsMessage)))
      // A thrown non-Error ends the loop, and the request goes out as it is.
      case _ => Future.unit
    }
    requestWillFetchDone.flatMap { _ =>
      val pluginFilteredRequest = request.copy()
      Globals.fetch(request).toFuture
        .flatMap { fetchResponse =>
          foldCallbacks(_.fetchDidSucceed, fetchResponse)((callback, state, response) => callback(FetchDidSucceedParam(pluginFilteredRequest, response, event, state)))
        }
        .recoverWith { case error =>
          val reported = originalRequest match
            case Some(original) =>
              // One clone of each for every callback, as runCallbacks passes one param object.
              val (originalClone, filteredClone) = (original.copy(), pluginFilteredRequest.copy())
              runCallbacks(_.fetchDidFail)((callback, state) => callback(FetchDidFailParam(error, originalClone, filteredClone, event, state)))
            case None => Future.unit
          reported.flatMap(_ => Future.failed(error))
        }
    }

  /** `fetchAndCachePut(input)`: the response now, and its cachePut as an extended lifetime. */
  def fetchAndCachePut(input: Request): Future[Response] =
    fetch(input).map { response =>
      val responseClone = response.copy()
      waitUntil(async(cachePut(input, responseClone)))
      response
    }

  /** `cacheMatch(key)`: `caches.match` of the read cache key in the strategy's cache (which opens
    * no cache), through cachedResponseWillBeUsed.
    */
  def cacheMatch(key: Request): Future[Option[Response]] =
    for
      effectiveRequest <- getCacheKey(key, "read")
      cachedResponse <- Globals.caches.matching(effectiveRequest, js.Dynamic.literal(cacheName = strategy.cacheName)).toFuture
      cachedResponse <- foldCallbacks(_.cachedResponseWillBeUsed, cachedResponse.toOption) { (callback, state, cachedResponse) =>
        callback(CachedResponseWillBeUsedParam(strategy.cacheName, effectiveRequest, cachedResponse, event, state))
      }
    yield cachedResponse

  /** `cachePut(key, response)`: one timer task later, the write cache key, cacheWillUpdate (or the
    * status-200 rule), the put, then cacheDidUpdate. Answers whether it cached.
    */
  def cachePut(key: Request, response: Response): Future[Boolean] =
    for
      _ <- timeout(0)
      effectiveRequest <- getCacheKey(key, "write")
      responseToCache <- ensureResponseSafeToCache(response)
      wasCached <- responseToCache.fold(Future.successful(false))(put(effectiveRequest, _))
    yield wasCached

  private def put(effectiveRequest: Request, responseToCache: Response): Future[Boolean] =
    for
      cache <- Globals.caches.open(strategy.cacheName).toFuture
      hasCacheUpdateCallback = hasCallback(_.cacheDidUpdate)
      oldResponse <-
        if hasCacheUpdateCallback then cacheMatchIgnoreParams(cache, effectiveRequest.copy(), Seq("__WB_REVISION__"))
        else Future.successful(None)
      // An Error is rethrown (a QuotaExceededError after the registered quota callbacks, of which
      // the subset has none); anything else is ignored.
      _ <- cache.put(effectiveRequest, if hasCacheUpdateCallback then responseToCache.copy() else responseToCache).toFuture
        .recover { case error if !error.isError => () }
      _ <- runCallbacks(_.cacheDidUpdate) { (callback, state) =>
        callback(CacheDidUpdateParam(strategy.cacheName, oldResponse, responseToCache.copy(), effectiveRequest, event, state))
      }
    yield true

  /** `getCacheKey(request, mode)`: cacheKeyWillBeUsed once per URL and mode for this handler. */
  def getCacheKey(request: Request, mode: String): Future[Request] =
    val key = s"${request.url} | $mode"
    cacheKeys.get(key) match
      case Some(known) => Future.successful(known)
      case None =>
        foldCallbacks(_.cacheKeyWillBeUsed, request)((callback, state, request) => callback(CacheKeyWillBeUsedParam(mode, request, event, params, state)))
          .map { effectiveRequest =>
            cacheKeys(key) = effectiveRequest
            effectiveRequest
          }

  /** `hasCallback(name)`, over the strategy's current plugins. */
  def hasCallback(callback: WorkboxPlugin => Option[Any]): Boolean = strategy.plugins.exists(callback(_).isDefined)

  /** `iterateCallbacks(name)`: the strategy's plugins that define the callback, read from the live
    * list at every step, so a plugin added meanwhile is reached (without state), as Workbox's
    * generator over `this._strategy.plugins` reaches it.
    */
  def iterateCallbacks[T](callback: WorkboxPlugin => Option[T]): Iterator[(T, Option[PluginState])] =
    Iterator.from(0).takeWhile(_ < strategy.plugins.length).flatMap { index =>
      val plugin = strategy.plugins(index)
      callback(plugin).map(_ -> pluginStateMap.collectFirst { case (known, state) if known eq plugin => state })
    }

  /** `runCallbacks(name, param)`: each callback in turn, each awaited before the next. */
  def runCallbacks[T](callback: WorkboxPlugin => Option[T])(call: (T, Option[PluginState]) => Future[Any]): Future[Unit] =
    foldCallbacks(callback, ())((each, state, _) => call(each, state).map(_ => ()))

  /** The loops of the form `for (callback of iterateCallbacks(name)) value = await callback(…)`,
    * which stop once `until` holds for a callback's result.
    */
  def foldCallbacks[T, A](callback: WorkboxPlugin => Option[T], initial: A, until: A => Boolean = (_: A) => false)(
      call: (T, Option[PluginState], A) => Future[A]
  ): Future[A] =
    val callbacks = iterateCallbacks(callback)
    def loop(value: A): Future[A] =
      if callbacks.hasNext then
        val (each, state) = callbacks.next()
        async(call(each, state, value)).flatMap(next => if until(next) then Future.successful(next) else loop(next))
      else Future.successful(value)
    loop(initial)

  /** `waitUntil(promise)`: extends the handler's lifetime until the promise settles. */
  def waitUntil[T](promise: Future[T]): Future[T] =
    extendLifetimePromises += promise
    promise

  /** `doneWaiting()`: settles every extended lifetime, including ones added meanwhile, and fails
    * with the first rejection of a batch.
    */
  def doneWaiting(): Future[Unit] =
    if extendLifetimePromises.isEmpty then Future.unit
    else
      val batch = extendLifetimePromises.toList
      extendLifetimePromises.clear()
      Future.sequence(batch.map(_.transform(Success(_)))).flatMap { outcomes =>
        outcomes.collectFirst { case Failure(error) => error } match
          case Some(error) => Future.failed(error)
          case None => doneWaiting()
      }

  def destroy(): Unit = handlerDeferred.success(null)

  /** `_ensureResponseSafeToCache(response)`: every cacheWillUpdate in turn until one refuses, or,
    * without any, only a status 200.
    */
  private def ensureResponseSafeToCache(response: Response): Future[Option[Response]] =
    var pluginsUsed = false
    foldCallbacks(_.cacheWillUpdate, Option(response), until = (_: Option[Response]).isEmpty) { (callback, state, responseToCache) =>
      pluginsUsed = true
      callback(CacheWillUpdateParam(request, responseToCache.get, event, state))
    }.map(responseToCache => if pluginsUsed then responseToCache else responseToCache.filter(_.status == 200))

/** `new WorkboxError('no-response', {url, error})`; the caught value is kept only when it is an
  * Error (`error` is then left out of the details).
  */
private def noResponse(request: Request, error: Option[Throwable] = None) =
  WorkboxError("no-response", js.Dynamic.literal(url = request.url, error = error.map(_.asDetail).orUndefined))

/** The caught value of a strategy's `catch`, kept only when it is an Error. */
private def keptError[T](attempt: Future[T]): Future[(Option[T], Option[Throwable])] =
  attempt.transform {
    case Success(value) => Success((Some(value), None))
    case Failure(error) => Success((None, Option.when(error.isError)(error)))
  }

final class CacheFirst(cacheName: String, initialPlugins: Seq[WorkboxPlugin] = Nil) extends Strategy(cacheName, initialPlugins):
  protected def _handle(request: Request, handler: StrategyHandler): Future[Response] =
    handler.cacheMatch(request).flatMap {
      case Some(response) => Future.successful(response)
      case None =>
        keptError(handler.fetchAndCachePut(request)).flatMap {
          case (Some(response), _) => Future.successful(response)
          case (None, error) => Future.failed(noResponse(request, error))
        }
    }

final class CacheOnly(cacheName: String, initialPlugins: Seq[WorkboxPlugin] = Nil) extends Strategy(cacheName, initialPlugins):
  protected def _handle(request: Request, handler: StrategyHandler): Future[Response] =
    handler.cacheMatch(request).flatMap(_.fold(Future.failed(noResponse(request)))(Future.successful))

/** `cacheOkAndOpaquePlugin`: status 200, or 0 for an opaque response. */
object CacheOkAndOpaquePlugin extends WorkboxPlugin:
  override val cacheWillUpdate: Option[Callback[CacheWillUpdateParam, Option[Response]]] =
    Some(param => Future.successful(Option.when(param.response.status == 200 || param.response.status == 0)(param.response)))

/** `NetworkFirst` without `networkTimeoutSeconds`. Its constructor puts the 200-or-opaque plugin
  * first unless a plugin has a cacheWillUpdate.
  */
final class NetworkFirst(cacheName: String, initialPlugins: Seq[WorkboxPlugin] = Nil) extends Strategy(cacheName, initialPlugins):
  if !this.plugins.exists(_.cacheWillUpdate.isDefined) then this.plugins.prepend(CacheOkAndOpaquePlugin)

  protected def _handle(request: Request, handler: StrategyHandler): Future[Response] =
    val networkPromise = getNetworkPromise(request, handler)
    val promises = Seq(networkPromise)
    handler
      .waitUntil(async(handler.waitUntil(Future.firstCompletedOf(promises)).flatMap(_.fold(networkPromise)(response => Future.successful(Some(response))))))
      .flatMap(_.fold(Future.failed(noResponse(request)))(Future.successful))

  /** `_getNetworkPromise`: fetchAndCachePut, and the cache when that failed or answered nothing. */
  private def getNetworkPromise(request: Request, handler: StrategyHandler): Future[Option[Response]] =
    keptError(handler.fetchAndCachePut(request)).flatMap {
      case (Some(response), None) => Future.successful(Some(response))
      case _ => handler.cacheMatch(request)
    }

/** `NetworkOnly` without `networkTimeoutSeconds`. */
final class NetworkOnly(cacheName: String, initialPlugins: Seq[WorkboxPlugin] = Nil) extends Strategy(cacheName, initialPlugins):
  protected def _handle(request: Request, handler: StrategyHandler): Future[Response] =
    keptError(handler.fetch(request)).flatMap {
      case (Some(response), _) => Future.successful(response)
      case (None, error) => Future.failed(noResponse(request, error))
    }
