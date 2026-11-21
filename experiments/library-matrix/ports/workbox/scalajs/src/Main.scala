// Entry of the Scala.js port: what libraries/workbox/reference.mjs does, on the port's Workbox. One
// call runs one script of Workbox API calls (CONTRACT.md "Steps") with the fixture plugins, builds
// the canonical output, and deletes the script's caches on every path.
//
// The boundary: the corpus input is the JS object itself, seen through @js.native facade traits
// (no copy); manifest entries become `PrecacheEntry` values, with each revision kept as the
// caller's value. The output is built from JS objects and arrays in the contract's key order.
// `run` is a native ES module export.

package workbox

import scala.collection.mutable
import scala.concurrent.Future
import scala.scalajs.js
import scala.scalajs.js.JSConverters.*
import scala.scalajs.js.annotation.JSExportTopLevel
import scala.util.{Failure, Success, Try}

private val Fixture = "experiments/library-matrix/libraries/workbox/"

/** Well-formed SRI metadata that matches no fixture, so the fetch fails integrity. */
private val WrongIntegrity = "sha256-" + "A" * 43 + "="

@js.native
trait NativeEvent extends js.Object:
  val `type`: String = js.native
  def waitUntil(promise: js.Promise[Any]): Unit = js.native

@js.native
trait PluginSpec extends js.Object:
  val kind: String = js.native
  val label: String = js.native
  val on: String = js.native
  val value: js.UndefOr[js.Any] = js.native
  val url: String = js.native
  val controller: String = js.native

@js.native
trait HandlerSpec extends js.Object:
  val strategy: js.UndefOr[String] = js.native
  val cacheName: String = js.native
  val plugins: js.UndefOr[js.Array[PluginSpec]] = js.native
  val precache: js.UndefOr[String] = js.native
  val bound: js.UndefOr[String] = js.native
  val controller: String = js.native
  val respond: js.Any = js.native
  val status: js.Any = js.native

@js.native
trait RouteOptionsSpec extends js.Object:
  val directoryIndex: js.UndefOr[String | Null] = js.native
  val cleanURLs: js.UndefOr[js.Any] = js.native
  val ignoreURLParametersMatching: js.UndefOr[js.Array[String]] = js.native
  val urlManipulation: js.UndefOr[js.Array[String]] = js.native

@js.native
trait Step extends js.Object:
  val op: String = js.native
  val id: js.UndefOr[String] = js.native
  val source: String = js.native
  val cacheName: String = js.native
  val plugins: js.UndefOr[js.Array[PluginSpec]] = js.native
  val fallbackToNetwork: js.UndefOr[js.Any] = js.native
  val controller: String = js.native
  val entries: js.Array[Any] = js.native
  val router: String = js.native
  val route: js.UndefOr[String] = js.native
  val options: js.UndefOr[RouteOptionsSpec] = js.native
  val prefix: String = js.native
  val crossOrigin: js.UndefOr[Boolean] = js.native
  val handler: HandlerSpec = js.native
  val method: js.UndefOr[String] = js.native
  val url: js.UndefOr[String] = js.native
  val mode: js.UndefOr[String] = js.native
  val integrity: js.UndefOr[String] = js.native
  val request: js.UndefOr[js.Any] = js.native

@js.native
trait Input extends js.Object:
  val steps: js.Array[Step] = js.native
  val cachePrefix: js.UndefOr[String] = js.native
  val event: js.UndefOr[NativeEvent] = js.native
  val request: js.UndefOr[Request] = js.native

/** One handler's trace: the trace plugin's label, the request and the records of its callbacks. */
private final class HandlerTrace(val label: String, val url: String):
  val log: js.Array[js.Array[Any]] = js.Array()

/** The stand-in ExtendableEvent of one event-driven step: it collects its lifetime promises. */
private final class StandInEvent(val eventType: String) extends ExtendableEvent:
  val pending: mutable.ArrayBuffer[Future[Any]] = mutable.ArrayBuffer.empty
  def waitUntil(promise: Future[Any]): Unit = pending += promise

/** A native event the caller supplied (service-worker mode). */
private final class HostEvent(event: NativeEvent) extends ExtendableEvent:
  def eventType: String = event.`type`
  def waitUntil(promise: Future[Any]): Unit = event.waitUntil(promise.toJSPromise)

/** A response as a followed redirect leaves it: it and every clone of it report redirected. */
private def redirected(response: Response): Response =
  val clone = response.asInstanceOf[js.Dynamic].selectDynamic("clone").bind(response)
  js.Object.defineProperty(response, "redirected", js.Dynamic.literal(value = true).asInstanceOf[js.PropertyDescriptor])
  val markedClone: js.Function0[Response] = () => redirected(clone().asInstanceOf[Response])
  js.Object.defineProperty(response, "clone", js.Dynamic.literal(value = markedClone).asInstanceOf[js.PropertyDescriptor])
  response

private def status(response: Option[Response]): Any = response.fold(null)(_.status)

/** An Error's name; a thrown non-Error (the fail fixture's `value`) prints as its string. */
private def nameOf(error: Throwable): String = error match
  case js.JavaScriptException(value) if !value.isInstanceOf[js.Error] => js.Dynamic.global.String(value.asInstanceOf[js.Any]).asInstanceOf[String]
  case _ => error.jsName

/** A manifest entry from the corpus: falsy, a string, or a `PrecacheEntry` object. */
private def precacheEntry(value: Any): PrecacheEntry = value match
  case _ if !isTruthy(value) => PrecacheEntry.Falsy(value.asInstanceOf[js.Any])
  case url: String => PrecacheEntry.Url(url)
  case _ =>
    val entry = value.asInstanceOf[js.Dynamic]
    def text(name: String) = entry.selectDynamic(name).asInstanceOf[js.UndefOr[String]].toOption
    PrecacheEntry.Entry(text("url"), entry.revision, text("integrity"), entry)

private final class Scenario(input: Input):
  private val base = URL("./", Globals.location.href).href
  private val prefix = input.cachePrefix.getOrElse("mires-workbox-" + Globals.crypto.randomUUID())
  private val owned = input.cachePrefix.isEmpty
  private var source = "alpha"
  private val controllers = mutable.HashMap.empty[String, PrecacheController]
  private val routers = mutable.HashMap.empty[String, Router]
  private val routes = mutable.HashMap.empty[String, Route]
  private val results = js.Array[Any]()
  private val events = js.Array[Any]()
  private val handlers = mutable.ArrayBuffer.empty[HandlerTrace]

  private def relative(url: String) = url.stripPrefix(base)

  private def cacheName(name: String) = s"$prefix-$name"

  private def cacheLabel(name: String) = name.stripPrefix(s"$prefix-")

  /** Runs every step, reports, and deletes the scenario's caches whatever happened. */
  def run(): Future[js.Object] =
    val report = runSteps(input.steps.toList).flatMap { _ =>
      dump().map { caches =>
        val traces = handlers.map(trace => js.Array[Any](trace.label, trace.url, trace.log)).toJSArray
        js.Dynamic.literal(results = results, events = events, handlers = traces, caches = caches)
      }
    }
    report.transformWith { outcome =>
      val cleanup = if owned then deleteCaches() else Future.unit
      cleanup.flatMap(_ => Future.fromTry(outcome))
    }

  /** A failed step is recorded and the script goes on. */
  private def runSteps(steps: List[Step]): Future[Unit] = steps match
    case Nil => Future.unit
    case step :: rest =>
      async(this.step(step))
        .transform(outcome => Success(outcome.fold(failure, identity)))
        .flatMap { result =>
          results.push(result)
          runSteps(rest)
        }

  private def deleteCaches(): Future[Unit] =
    Globals.caches.keys().toFuture.flatMap { names =>
      names.filter(_.startsWith(s"$prefix-")).foldLeft(Future.unit)((done, name) => done.flatMap(_ => Globals.caches.delete(name).toFuture.map(_ => ())))
    }

  private def step(step: Step): Future[Any] = step.op match
    case "network" =>
      source = step.source
      Future.successful(null)
    case "controller" =>
      val fallbackToNetwork = !step.fallbackToNetwork.contains(false)
      controllers(step.id.get) = PrecacheController(cacheName(step.cacheName), plugins(step.plugins), fallbackToNetwork)
      Future.successful(null)
    case "addToCacheList" =>
      controller(step.controller).addToCacheList(step.entries.toSeq.map(precacheEntry))
      Future.successful(null)
    case "addPlugins" =>
      controller(step.controller).strategy.plugins ++= step.plugins.fold(Seq.empty)(_.toSeq).map(plugin)
      Future.successful(null)
    case "install" =>
      lifetime("install")(event => controller(step.controller).install(event)).map { result =>
        js.Dynamic.literal(updatedURLs = urls(result.updatedURLs), notUpdatedURLs = urls(result.notUpdatedURLs))
      }
    case "activate" =>
      lifetime("activate")(event => controller(step.controller).activate(event)).map(result => js.Dynamic.literal(deletedURLs = urls(result.deletedURLs)))
    case "getCachedURLs" => Future.successful(urls(controller(step.controller).getCachedURLs))
    case "getURLsToCacheKeys" =>
      Future.successful(controller(step.controller).getURLsToCacheKeys.toSeq.map((url, key) => js.Array(relative(url), relative(key))).toJSArray)
    case "getCacheKeyForURL" => Future.successful(controller(step.controller).getCacheKeyForURL(step.url.get).map(relative).orNull)
    case "matchPrecache" =>
      val url = step.url.get
      val request: Request | String = if isTruthy(step.request) then Request(url) else url
      controller(step.controller).matchPrecache(request).flatMap(record)
    case "router" =>
      routers(step.id.get) = Router()
      Future.successful(null)
    case "precacheRoute" => register(step, PrecacheRoute(controller(step.controller), routeOptions(step.options.toOption)))
    case "route" =>
      val matches: MatchCallbackOptions => Boolean =
        if step.crossOrigin.contains(true) then options => !options.sameOrigin else options => relative(options.url.href).startsWith(step.prefix)
      val routeHandler = handler(step.handler)
      register(step, Route(options => Option.when(matches(options))(Params()), routeHandler, step.method.getOrElse("GET")))
    case "defaultHandler" =>
      router(step.router).setDefaultHandler(handler(step.handler), step.method.getOrElse("GET"))
      Future.successful(null)
    case "catchHandler" =>
      step.route.toOption match
        case Some(route) => routes(route).setCatchHandler(handler(step.handler))
        case None => router(step.router).setCatchHandler(handler(step.handler))
      Future.successful(null)
    case "unregisterRoute" =>
      router(step.router).unregisterRoute(routes(step.route.get))
      Future.successful(null)
    case "fetch" =>
      val request = step.url.toOption match
        case None => input.request.get
        case Some(url) => Request(url, requestInit(method = step.method.toOption, mode = step.mode.toOption, integrity = step.integrity.toOption))
      lifetime("fetch") { event =>
        router(step.router).handleRequest(request, event) match
          case Some(response) => response.map(Some(_))
          case None => Future.successful(None)
      }.flatMap(record)
    case "caches" => dump()
    case op => throw js.JavaScriptException(js.TypeError("unknown step " + op))

  private def controller(id: String) = controllers(id)

  private def router(id: String) = routers(id)

  private def register(step: Step, route: Route): Future[Any] =
    step.id.foreach(routes(_) = route)
    router(step.router).registerRoute(route)
    Future.successful(null)

  private def urls(urls: Seq[String]): js.Array[String] = urls.map(relative).toJSArray

  /** The network fixture, then the plugins the script names. */
  private def plugins(specs: js.UndefOr[js.Array[PluginSpec]]): Seq[WorkboxPlugin] =
    NetworkPlugin() +: specs.fold(Seq.empty)(_.toSeq).map(plugin)

  private def plugin(spec: PluginSpec): WorkboxPlugin = spec.kind match
    case "trace" => TracePlugin(spec.label)
    case "keep" =>
      new WorkboxPlugin:
        override val cacheWillUpdate: Option[Callback[CacheWillUpdateParam, Option[Response]]] = Some(param => Future.successful(Some(param.response)))
    case "drop" =>
      new WorkboxPlugin:
        override val cacheWillUpdate: Option[Callback[CacheWillUpdateParam, Option[Response]]] = Some(_ => Future.successful(None))
    case "fail" => FailPlugin(spec.on, spec.value.toOption)
    case "fallback" => PrecacheFallbackPlugin(spec.url, controller(spec.controller))
    case kind => throw js.JavaScriptException(js.TypeError("unknown plugin " + kind))

  /** A route handler: a strategy, a controller's precache strategy, a handler bound to a precached
    * URL, or an async function answering a fixed response.
    */
  private def handler(spec: HandlerSpec): RouteHandler =
    if spec.strategy.isDefined then
      val (name, plugins) = (cacheName(spec.cacheName), this.plugins(spec.plugins))
      spec.strategy.get match
        case "CacheFirst" => CacheFirst(name, plugins)
        case "CacheOnly" => CacheOnly(name, plugins)
        case "NetworkFirst" => NetworkFirst(name, plugins)
        case "NetworkOnly" => NetworkOnly(name, plugins)
        case other => throw js.JavaScriptException(js.TypeError("unknown strategy " + other))
    else if spec.precache.isDefined then controller(spec.precache.get).strategy
    else if spec.bound.isDefined then controller(spec.controller).createHandlerBoundToURL(spec.bound.get)
    else _ => async(Future.successful(Response(spec.respond, js.Dynamic.literal(status = spec.status))))

  /** PrecacheRoute options as given: an absent member keeps the default, `null` disables
    * `directoryIndex` and `cleanURLs`.
    */
  private def routeOptions(options: Option[RouteOptionsSpec]): PrecacheRouteOptions =
    val defaults = PrecacheRouteOptions()
    options.fold(defaults) { options =>
      PrecacheRouteOptions(
        directoryIndex = options.directoryIndex.fold(defaults.directoryIndex)(Option(_)),
        ignoreURLParametersMatching = options.ignoreURLParametersMatching.fold(defaults.ignoreURLParametersMatching)(_.toSeq.map(js.RegExp(_))),
        cleanURLs = options.cleanURLs.fold(defaults.cleanURLs)(isTruthy),
        urlManipulation = options.urlManipulation.toOption.map { suffixes => (url: URL) =>
          suffixes.toSeq.map { suffix =>
            val variant = URL(url.href)
            variant.pathname += suffix
            variant
          }
        },
      )
    }

  /** Runs one event-driven step under a fresh stand-in event and waits for every lifetime it was
    * given, in rounds, or, in service-worker mode, under the caller's native event.
    */
  private def lifetime[T](eventType: String)(action: ExtendableEvent => Future[T]): Future[T] =
    input.event.toOption match
      case Some(native) => async(action(HostEvent(native)))
      case None =>
        val event = StandInEvent(eventType)
        def settle(count: Int): Future[Int] =
          if event.pending.isEmpty then Future.successful(count)
          else
            val batch = event.pending.toList
            event.pending.clear()
            Future.sequence(batch.map(_.transform(Success(_)))).flatMap(_ => settle(count + batch.length))
        async(action(event)).transformWith { outcome =>
          settle(0).flatMap { count =>
            events.push(js.Array[Any](eventType, count))
            Future.fromTry(outcome)
          }
        }

  /** `{status, url, body}` of a response, reading its body; null without a response. */
  private def record(response: Option[Response]): Future[Any] = response match
    case None => Future.successful(null)
    case Some(response) =>
      val (status, url) = (response.status, relative(response.url))
      response.text().toFuture.map(body => js.Dynamic.literal(status = status, url = url, body = body))

  /** `{error, message}` for a WorkboxError, `{error}` for another Error, `{thrown}` for a thrown
    * non-Error.
    */
  private def failure(error: Throwable): Any = error match
    case workboxError: WorkboxError =>
      js.Dynamic.literal(error = workboxError.name, message = workboxError.getMessage.replace(base, "").replace(s"$prefix-", ""))
    case _ if error.isError => js.Dynamic.literal(error = error.jsName)
    case _ => js.Dynamic.literal(thrown = nameOf(error))

  /** Every cache of the scenario, in CacheStorage order, with each entry's response record. */
  private def dump(): Future[js.Array[Any]] =
    Globals.caches.keys().toFuture.flatMap { names =>
      sequentially(names.toList.filter(_.startsWith(s"$prefix-"))) { name =>
        Globals.caches.open(name).toFuture.flatMap { cache =>
          cache.keys().toFuture.flatMap { requests =>
            sequentially(requests.toList) { request =>
              cache.matching(request).toFuture.flatMap(response => record(Some(response.get))).map(record => js.Array[Any](relative(request.url), record))
            }
          }.map(entries => js.Array[Any](cacheLabel(name), entries))
        }
      }
    }

  private def sequentially[A](items: List[A])(each: A => Future[Any]): Future[js.Array[Any]] =
    items.foldLeft(Future.successful(js.Array[Any]())) { (done, item) =>
      done.flatMap(out => each(item).map { value => out.push(value); out })
    }

  /** The network fixture: every fetch goes to one of the two fixture files, or fails. */
  private final class NetworkPlugin extends WorkboxPlugin:
    override val requestWillFetch: Option[Callback[RequestWillFetchParam, Request]] = Some { param =>
      async {
        if source == "offline" then throw js.JavaScriptException(js.TypeError("offline"))
        val target = URL(Fixture + (if source == "beta" then "beta" else "alpha") + ".mjs", Globals.location.href).href
        Future.successful(Request(target, requestInit(integrity = Some(if source == "broken" then WrongIntegrity else param.request.integrity))))
      }
    }

    override val fetchDidSucceed: Option[Callback[FetchDidSucceedParam, Response]] = Some { param =>
      Future.successful(source match
        case "missing" => Response("missing", js.Dynamic.literal(status = 404))
        case "redirected" => redirected(param.response)
        case _ => param.response
      )
    }

  /** The trace plugin: every lifecycle callback but cacheWillUpdate, recorded per handler. A
    * callback records only into a log its own handlerWillStart created: a plugin before it can
    * fail that callback.
    */
  private final class TracePlugin(label: String) extends WorkboxPlugin:
    private def record(state: Option[PluginState], entry: Any*): Future[Unit] =
      state.flatMap(_.get("log")).collect { case trace: HandlerTrace => trace.log.push(entry.toJSArray) }
      Future.unit

    override val handlerWillStart: Option[Callback[HandlerWillStartParam, Unit]] = Some { param =>
      val trace = HandlerTrace(label, relative(param.request.url))
      param.state.foreach(_("log") = trace)
      handlers += trace
      Future.unit
    }
    override val cacheKeyWillBeUsed: Option[Callback[CacheKeyWillBeUsedParam, Request]] = Some { param =>
      record(param.state, "cacheKeyWillBeUsed", param.mode, relative(param.request.url)).map(_ => param.request)
    }
    override val cachedResponseWillBeUsed: Option[Callback[CachedResponseWillBeUsedParam, Option[Response]]] = Some { param =>
      record(param.state, "cachedResponseWillBeUsed", cacheLabel(param.cacheName), relative(param.request.url), status(param.cachedResponse))
        .map(_ => param.cachedResponse)
    }
    override val requestWillFetch: Option[Callback[RequestWillFetchParam, Request]] = Some { param =>
      record(param.state, "requestWillFetch", relative(param.request.url)).map(_ => param.request)
    }
    override val fetchDidSucceed: Option[Callback[FetchDidSucceedParam, Response]] = Some { param =>
      record(param.state, "fetchDidSucceed", relative(param.request.url), param.response.status).map(_ => param.response)
    }
    override val fetchDidFail: Option[Callback[FetchDidFailParam, Unit]] = Some { param =>
      record(param.state, "fetchDidFail", relative(param.originalRequest.url), relative(param.request.url), nameOf(param.error))
    }
    override val cacheDidUpdate: Option[Callback[CacheDidUpdateParam, Unit]] = Some { param =>
      record(param.state, "cacheDidUpdate", cacheLabel(param.cacheName), relative(param.request.url), status(param.oldResponse), param.newResponse.status)
    }
    override val handlerWillRespond: Option[Callback[HandlerWillRespondParam, Response]] = Some { param =>
      record(param.state, "handlerWillRespond", param.response.status).map(_ => param.response)
    }
    override val handlerDidRespond: Option[Callback[HandlerDidRespondParam, Unit]] = Some { param =>
      record(param.state, "handlerDidRespond", status(param.response))
    }
    override val handlerDidComplete: Option[Callback[HandlerDidCompleteParam, Unit]] = Some { param =>
      record(param.state, "handlerDidComplete", status(param.response), param.error.map(nameOf).orNull)
    }
    override val handlerDidError: Option[Callback[HandlerDidErrorParam, Option[Response]]] = Some { param =>
      record(param.state, "handlerDidError", nameOf(param.error)).map(_ => None)
    }

/** A plugin whose one callback, the one named `on`, throws `new Error('fail')`, or, with a value,
  * that value itself (a non-Error).
  */
private final class FailPlugin(on: String, value: Option[js.Any]) extends WorkboxPlugin:
  private def failing[P, R](name: String): Option[Callback[P, R]] =
    Option.when(name == on)(_ => Future.failed(js.JavaScriptException(value.getOrElse(js.Error("fail")))))

  override val handlerWillStart: Option[Callback[HandlerWillStartParam, Unit]] = failing("handlerWillStart")
  override val cacheKeyWillBeUsed: Option[Callback[CacheKeyWillBeUsedParam, Request]] = failing("cacheKeyWillBeUsed")
  override val cachedResponseWillBeUsed: Option[Callback[CachedResponseWillBeUsedParam, Option[Response]]] = failing("cachedResponseWillBeUsed")
  override val cacheWillUpdate: Option[Callback[CacheWillUpdateParam, Option[Response]]] = failing("cacheWillUpdate")
  override val cacheDidUpdate: Option[Callback[CacheDidUpdateParam, Unit]] = failing("cacheDidUpdate")
  override val requestWillFetch: Option[Callback[RequestWillFetchParam, Request]] = failing("requestWillFetch")
  override val fetchDidSucceed: Option[Callback[FetchDidSucceedParam, Response]] = failing("fetchDidSucceed")
  override val fetchDidFail: Option[Callback[FetchDidFailParam, Unit]] = failing("fetchDidFail")
  override val handlerWillRespond: Option[Callback[HandlerWillRespondParam, Response]] = failing("handlerWillRespond")
  override val handlerDidRespond: Option[Callback[HandlerDidRespondParam, Unit]] = failing("handlerDidRespond")
  override val handlerDidComplete: Option[Callback[HandlerDidCompleteParam, Unit]] = failing("handlerDidComplete")
  override val handlerDidError: Option[Callback[HandlerDidErrorParam, Option[Response]]] = failing("handlerDidError")

object Main:
  /** Runs one corpus script and resolves with its output. */
  @JSExportTopLevel("run")
  def run(input: Input): js.Promise[js.Object] = async(Scenario(input).run()).toJSPromise
