// workbox-precaching: PrecacheController, PrecacheStrategy, PrecacheRoute with its URL variations,
// createCacheKey, and the cache-key, install-report and fallback plugins.

package workbox

import scala.collection.mutable
import scala.concurrent.Future
import scala.scalajs.js
import scala.scalajs.js.JSConverters.*

private val RevisionSearchParam = "__WB_REVISION__"

/** A precache manifest entry as the caller wrote it (`PrecacheEntry | string`). */
enum PrecacheEntry:
  /** A string: a URL that is its own cache key. */
  case Url(url: String)

  /** A `PrecacheEntry` object. `revision` stays the caller's value (a string, a number, null or
    * undefined): its truthiness decides, and the host prints a number. `source` is the object
    * itself, which add-to-cache-list-unexpected-type reports.
    */
  case Entry(url: Option[String], revision: js.Any, integrity: Option[String], source: js.Any)

  /** `null`, `''` or another falsy value. */
  case Falsy(source: js.Any)

private def unexpectedType(entry: js.Any) =
  WorkboxError("add-to-cache-list-unexpected-type", js.Dynamic.literal(entry = entry))

/** `createCacheKey(entry)`: the URL resolved against the location, and with a truthy revision a
  * cache key that carries it in `__WB_REVISION__`. Answers `(cacheKey, url)`.
  */
private def createCacheKey(entry: PrecacheEntry): (String, String) = entry match
  case PrecacheEntry.Falsy(source) => throw unexpectedType(source)
  case PrecacheEntry.Url(url) =>
    val href = URL(url, Globals.location.href).href
    (href, href)
  case PrecacheEntry.Entry(url, revision, _, source) =>
    val entryURL = url.filter(_.nonEmpty).getOrElse(throw unexpectedType(source))
    if !isTruthy(revision) then
      val href = URL(entryURL, Globals.location.href).href
      (href, href)
    else
      val cacheKeyURL = URL(entryURL, Globals.location.href)
      val originalURL = URL(entryURL, Globals.location.href)
      cacheKeyURL.searchParams.set(RevisionSearchParam, revision)
      (cacheKeyURL.href, originalURL.href)

final case class InstallResult(updatedURLs: Seq[String], notUpdatedURLs: Seq[String])

final case class CleanupResult(deletedURLs: Seq[String])

/** `PrecacheController`. */
final class PrecacheController(cacheName: String, plugins: Seq[WorkboxPlugin] = Nil, fallbackToNetwork: Boolean = true):
  private val urlsToCacheKeys = mutable.LinkedHashMap.empty[String, String]
  private val urlsToCacheModes = mutable.LinkedHashMap.empty[String, String]
  private val cacheKeysToIntegrities = mutable.LinkedHashMap.empty[String, String]

  val strategy: PrecacheStrategy = PrecacheStrategy(cacheName, plugins :+ PrecacheCacheKeyPlugin(this), fallbackToNetwork)

  /** `addToCacheList(entries)`: entry by entry, so the entries before a failing one stay added.
    * Like the production build it warns about revisionless entries.
    */
  def addToCacheList(entries: Seq[PrecacheEntry]): Unit =
    val urlsToWarnAbout = mutable.ArrayBuffer.empty[String]
    for entry <- entries do
      entry match
        case PrecacheEntry.Url(url) => urlsToWarnAbout += url
        case PrecacheEntry.Entry(url, revision, _, _) if js.isUndefined(revision) => urlsToWarnAbout += url.getOrElse("")
        case _ => ()
      val (cacheKey, url) = createCacheKey(entry)
      val cacheMode = entry match
        case PrecacheEntry.Entry(_, revision, _, _) if isTruthy(revision) => "reload"
        case _ => "default"
      urlsToCacheKeys.get(url).filter(_ != cacheKey).foreach { firstEntry =>
        throw WorkboxError("add-to-cache-list-conflicting-entries", js.Dynamic.literal(firstEntry = firstEntry, secondEntry = cacheKey))
      }
      entry match
        case PrecacheEntry.Entry(_, _, Some(integrity), _) if integrity.nonEmpty =>
          if cacheKeysToIntegrities.get(cacheKey).exists(_ != integrity) then
            throw WorkboxError("add-to-cache-list-conflicting-integrities", js.Dynamic.literal(url = url))
          cacheKeysToIntegrities(cacheKey) = integrity
        case _ => ()
      urlsToCacheKeys(url) = cacheKey
      urlsToCacheModes(url) = cacheMode
      if urlsToWarnAbout.nonEmpty then
        Globals.console.warn(
          s"Workbox is precaching URLs without revision info: ${urlsToWarnAbout.mkString(", ")}\nThis is generally NOT safe. Learn more at https://bit.ly/wb-precache"
        )

  /** `install(event)`: precaches each URL in turn through the strategy, under an install report
    * plugin, and extends the event's lifetime until it is done.
    */
  def install(event: ExtendableEvent): Future[InstallResult] = waitUntil(event) {
    val installReportPlugin = PrecacheInstallReportPlugin()
    strategy.plugins += installReportPlugin
    // Steps run one at a time, so the map cannot change during an install.
    val entries = urlsToCacheKeys.toList
    def precache(remaining: List[(String, String)]): Future[Unit] = remaining match
      case Nil => Future.unit
      case (url, cacheKey) :: rest =>
        val integrity = cacheKeysToIntegrities.get(cacheKey)
        val cacheMode = urlsToCacheModes.get(url)
        val init = requestInit(integrity = integrity, cache = cacheMode, credentials = Some("same-origin"))
        val request = Request(url, init)
        val (responseDone, handlerDone) = strategy.handleAll(HandlerCallbackOptions(request, event, Some(Params(cacheKey = Some(cacheKey)))))
        // Promise.all: the response's failure, else the completion's (which comes later).
        responseDone.flatMap(_ => handlerDone).flatMap(_ => precache(rest))
    precache(entries).map(_ => InstallResult(installReportPlugin.updatedURLs.toList, installReportPlugin.notUpdatedURLs.toList))
  }

  /** `activate(event)`: deletes every request in the precache whose URL is no current cache key. */
  def activate(event: ExtendableEvent): Future[CleanupResult] = waitUntil(event) {
    for
      cache <- Globals.caches.open(strategy.cacheName).toFuture
      currentlyCachedRequests <- cache.keys().toFuture
      expectedCacheKeys = urlsToCacheKeys.values.toSet
      deletedURLs <- currentlyCachedRequests.toList.foldLeft(Future.successful(Vector.empty[String])) { (deleted, request) =>
        deleted.flatMap { deletedURLs =>
          if expectedCacheKeys.contains(request.url) then Future.successful(deletedURLs)
          else cache.delete(request).toFuture.map(_ => deletedURLs :+ request.url)
        }
      }
    yield CleanupResult(deletedURLs)
  }

  def getURLsToCacheKeys: collection.Map[String, String] = urlsToCacheKeys

  def getCachedURLs: Seq[String] = urlsToCacheKeys.keys.toList

  /** `getCacheKeyForURL(url)`: the exact URL resolved against the location, without variations. */
  def getCacheKeyForURL(url: String): Option[String] = urlsToCacheKeys.get(URL(url, Globals.location.href).href)

  def getIntegrityForCacheKey(cacheKey: String): Option[String] = cacheKeysToIntegrities.get(cacheKey)

  /** `matchPrecache(request)`: opens the precache only for a URL in the manifest. */
  def matchPrecache(request: Request | String): Future[Option[Response]] = async {
    val url = request match
      case request: Request => request.url
      case url: String => url
    getCacheKeyForURL(url) match
      case Some(cacheKey) => Globals.caches.open(strategy.cacheName).toFuture.flatMap(_.matching(cacheKey).toFuture).map(_.toOption)
      case None => Future.successful(None)
  }

  /** `createHandlerBoundToURL(url)`: a handler that answers any request with this precached URL. */
  def createHandlerBoundToURL(url: String): RouteHandler =
    val cacheKey = getCacheKeyForURL(url).getOrElse(throw WorkboxError("non-precached-url", js.Dynamic.literal(url = url)))
    options =>
      // `{cacheKey, ...options.params}`.
      val params = Params(options.params.flatMap(_.cacheKey).orElse(Some(cacheKey)), options.params.flatMap(_.integrity))
      strategy.handle(HandlerCallbackOptions(Request(url), options.event, Some(params)))

/** `PrecacheStrategy`: a Strategy that answers from the precache, populates it during install,
  * and otherwise falls back to the network.
  */
final class PrecacheStrategy(cacheName: String, initialPlugins: Seq[WorkboxPlugin], fallbackToNetwork: Boolean) extends Strategy(cacheName, initialPlugins):
  import PrecacheStrategy.*

  this.plugins += copyRedirectedCacheableResponsesPlugin

  protected def _handle(request: Request, handler: StrategyHandler): Future[Response] =
    handler.cacheMatch(request).flatMap {
      case Some(response) => Future.successful(response)
      case None => if handler.event.eventType == "install" then handleInstall(request, handler) else handleFetch(request, handler)
    }

  /** `_handleFetch`: the network, with the manifest's integrity unless the request is no-cors, and
    * a cache repair only when that integrity vouches for the response.
    */
  private def handleFetch(request: Request, handler: StrategyHandler): Future[Response] = async {
    val params = handler.params.getOrElse(Params())
    if !fallbackToNetwork then
      throw WorkboxError("missing-precache-entry", js.Dynamic.literal(cacheName = cacheName, url = request.url))
    val integrityInManifest = params.integrity.filter(_.nonEmpty)
    val integrityInRequest = request.integrity
    val noIntegrityConflict = integrityInRequest.isEmpty || integrityInManifest.contains(integrityInRequest)
    val cors = request.mode != "no-cors"
    val integrity = if cors then Option(integrityInRequest).filter(_.nonEmpty).orElse(integrityInManifest) else None
    handler.fetch(Request(request, requestInit(integrity = integrity))).flatMap { response =>
      if integrityInManifest.isDefined && noIntegrityConflict && cors then
        useDefaultCacheabilityPluginIfNeeded()
        handler.cachePut(request, response.copy()).map(_ => response)
      else Future.successful(response)
    }
  }

  /** `_handleInstall`: fetch, then cache, and fail the install when the response may not be
    * cached.
    */
  private def handleInstall(request: Request, handler: StrategyHandler): Future[Response] =
    useDefaultCacheabilityPluginIfNeeded()
    for
      response <- handler.fetch(request)
      wasCached <- handler.cachePut(request, response.copy())
      _ <-
        if wasCached then Future.unit
        else Future.failed(WorkboxError("bad-precaching-response", js.Dynamic.literal(url = request.url, status = response.status)))
    yield response

  /** `_useDefaultCacheabilityPluginIfNeeded`: exactly one cacheWillUpdate besides the redirect
    * copy, adding the default one when there is none and dropping it when there are more.
    */
  private def useDefaultCacheabilityPluginIfNeeded(): Unit =
    var defaultPluginIndex: Option[Int] = None
    var cacheWillUpdatePluginCount = 0
    for (plugin, index) <- plugins.zipWithIndex if plugin ne copyRedirectedCacheableResponsesPlugin do
      if plugin eq defaultPrecacheCacheabilityPlugin then defaultPluginIndex = Some(index)
      if plugin.cacheWillUpdate.isDefined then cacheWillUpdatePluginCount += 1
    if cacheWillUpdatePluginCount == 0 then plugins += defaultPrecacheCacheabilityPlugin
    else if cacheWillUpdatePluginCount > 1 then defaultPluginIndex.foreach(index => plugins.remove(index))

object PrecacheStrategy:
  /** Statuses below 400. */
  val defaultPrecacheCacheabilityPlugin: WorkboxPlugin = new WorkboxPlugin:
    override val cacheWillUpdate: Option[Callback[CacheWillUpdateParam, Option[Response]]] =
      Some(param => Future.successful(Option.when(param.response.status < 400)(param.response)))

  /** A redirected response is cached as a copy without its redirect. */
  val copyRedirectedCacheableResponsesPlugin: WorkboxPlugin = new WorkboxPlugin:
    override val cacheWillUpdate: Option[Callback[CacheWillUpdateParam, Option[Response]]] =
      Some(param => (if param.response.redirected then copyResponse(param.response) else Future.successful(param.response)).map(Some(_)))

/** `PrecacheCacheKeyPlugin`: the route's cache key, else the manifest's key for the exact URL. */
final class PrecacheCacheKeyPlugin(precacheController: PrecacheController) extends WorkboxPlugin:
  override val cacheKeyWillBeUsed: Option[Callback[CacheKeyWillBeUsedParam, Request]] = Some { param =>
    async {
      val cacheKey = param.params.flatMap(_.cacheKey).filter(_.nonEmpty).orElse(precacheController.getCacheKeyForURL(param.request.url))
      Future.successful(cacheKey.fold(param.request)(cacheKey => Request(cacheKey, requestInit(headers = Some(param.request.headers)))))
    }
  }

/** `PrecacheInstallReportPlugin`: an install-time cache hit is not updated, a miss is. */
final class PrecacheInstallReportPlugin extends WorkboxPlugin:
  val updatedURLs: mutable.ArrayBuffer[String] = mutable.ArrayBuffer.empty
  val notUpdatedURLs: mutable.ArrayBuffer[String] = mutable.ArrayBuffer.empty

  override val handlerWillStart: Option[Callback[HandlerWillStartParam, Unit]] = Some { param =>
    param.state.foreach(_("originalRequest") = param.request)
    Future.unit
  }

  override val cachedResponseWillBeUsed: Option[Callback[CachedResponseWillBeUsedParam, Option[Response]]] = Some { param =>
    if param.event.eventType == "install" then
      param.state.flatMap(_.get("originalRequest")).collect { case originalRequest: Request => originalRequest }.foreach { originalRequest =>
        if param.cachedResponse.isDefined then notUpdatedURLs += originalRequest.url else updatedURLs += originalRequest.url
      }
    Future.successful(param.cachedResponse)
  }

/** `PrecacheFallbackPlugin`: a failed strategy answers with a precached URL, when it is there. */
final class PrecacheFallbackPlugin(fallbackURL: String, precacheController: PrecacheController) extends WorkboxPlugin:
  override val handlerDidError: Option[Callback[HandlerDidErrorParam, Option[Response]]] =
    Some(_ => precacheController.matchPrecache(fallbackURL))

/** `PrecacheRouteOptions`. A falsy `directoryIndex` or `cleanURLs` disables it. */
final case class PrecacheRouteOptions(
    directoryIndex: Option[String] = Some("index.html"),
    ignoreURLParametersMatching: Seq[js.RegExp] = Seq(js.RegExp("^utm_"), js.RegExp("^fbclid$")),
    cleanURLs: Boolean = true,
    urlManipulation: Option[URL => Seq[URL]] = None,
)

/** `removeIgnoredSearchParams(urlObject, ignoreURLParametersMatching)`, on the URL itself. */
private def removeIgnoredSearchParams(urlObject: URL, ignoreURLParametersMatching: Seq[js.RegExp]): URL =
  for paramName <- js.Array.from(urlObject.searchParams.keys()) do
    if ignoreURLParametersMatching.exists(_.test(paramName)) then urlObject.searchParams.delete(paramName)
  urlObject

/** `generateURLVariations(url, options)`: a lazy generator, as Workbox's is, so a later variation
  * (and `urlManipulation`) is computed only when the earlier ones missed.
  */
private def generateURLVariations(url: String, options: PrecacheRouteOptions): Iterator[String] =
  val urlObject = URL(url, Globals.location.href)
  urlObject.hash = ""
  // The URL object itself loses its ignored parameters; urlManipulation sees it that way.
  lazy val urlWithoutIgnoredParams = removeIgnoredSearchParams(urlObject, options.ignoreURLParametersMatching)
  def directoryIndexURL = options.directoryIndex.filter(_.nonEmpty && urlWithoutIgnoredParams.pathname.endsWith("/")).map { directoryIndex =>
    val directoryURL = URL(urlWithoutIgnoredParams.href)
    directoryURL.pathname += directoryIndex
    directoryURL.href
  }
  def cleanURL = Option.when(options.cleanURLs) {
    val cleanURL = URL(urlWithoutIgnoredParams.href)
    cleanURL.pathname += ".html"
    cleanURL.href
  }
  // Each `++` operand is evaluated only once the variations before it are used up.
  Iterator.single(urlObject.href)
    ++ Iterator.single(urlWithoutIgnoredParams.href)
    ++ directoryIndexURL
    ++ cleanURL
    ++ options.urlManipulation.iterator.flatMap(urlManipulation => urlManipulation(urlObject).map(_.href))

/** `new PrecacheRoute(controller, options)`: a GET route to the controller's strategy that matches
  * the first variation of the request URL in the manifest, with `{cacheKey, integrity}`.
  */
final class PrecacheRoute(precacheController: PrecacheController, routeOptions: PrecacheRouteOptions = PrecacheRouteOptions())
    extends Route(
      options => {
        val urlsToCacheKeys = precacheController.getURLsToCacheKeys
        generateURLVariations(options.request.url, routeOptions).flatMap(urlsToCacheKeys.get).nextOption()
          .map(cacheKey => Params(Some(cacheKey), precacheController.getIntegrityForCacheKey(cacheKey)))
      },
      precacheController.strategy,
    )
