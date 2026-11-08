package fp

import scala.scalajs.js

object Agent {
  val version = "5.2.0"

  def componentsToCanonicalString(components: js.Dynamic): String = {
    val keys = Host.objectKeys(components)
    keys.sort()
    var result = ""
    var i = 0
    while (i < keys.length) {
      val componentKey = keys(i)
      val component = components.selectDynamic(componentKey)
      val value =
        if (Host.has(component.asInstanceOf[js.Any], "error")) "error"
        else {
          val serialized = Host.g.JSON.stringify(component.value)
          if (Host.isUndef(serialized.asInstanceOf[js.Any])) "undefined"
          else Host.toJsString(serialized.asInstanceOf[js.Any])
        }
      val escaped = new js.RegExp("""([:|\\])""", "g")
      val escapedKey = Host.dyn(componentKey.asInstanceOf[js.Any]).replace(escaped, """\$1""").asInstanceOf[String]
      result += (if (result.nonEmpty) "|" else "") + escapedKey + ":" + value
      i += 1
    }
    result
  }

  def componentsToDebugString(components: js.Any): String = {
    val replacer: js.Function2[js.Any, js.Any, js.Any] = (_key, value) =>
      if (Host.instanceofError(value)) Misc.errorToObject(Host.dyn(value))
      else value
    Host.jsonStringifyPretty(components, replacer, 2)
  }

  def hashComponents(components: js.Dynamic): String =
    Hashing.x64hash128(componentsToCanonicalString(components))

  def makeLazyGetResult(components: js.Dynamic): js.Dynamic = {
    var visitorIdCache: js.UndefOr[String] = js.undefined
    val confidence = Confidence.getConfidence(components)
    val getVisitorId: js.Function0[js.Any] = () => {
      if (visitorIdCache.isEmpty) visitorIdCache = hashComponents(components)
      visitorIdCache.get
    }
    val setVisitorId: js.Function1[js.Any, Unit] = (visitorId: js.Any) => {
      visitorIdCache = Host.toJsString(visitorId)
      ()
    }
    Host.nameFunction(getVisitorId, "get visitorId")
    Host.nameFunction(setVisitorId, "set visitorId")
    val result = js.Dynamic.literal()
    Host.defineEnumerableAccessor(result, "visitorId", getVisitorId, setVisitorId)
    result.confidence = confidence
    result.components = components
    result.version = version
    result
  }

  def prepareForSources(delayFallback: js.UndefOr[Double] = js.undefined): js.Promise[Unit] = {
    val delay = delayFallback.getOrElse(50.0)
    Async.requestIdleCallbackIfAvailable(delay, delay * 2)
  }

  def makeAgent(getComponents: js.Function0[js.Promise[js.Dynamic]], debug: js.UndefOr[Boolean]): js.Dynamic = {
    val creationTime = Host.dateNow()
    js.Dynamic.literal(
      get = (options: js.UndefOr[js.Dynamic]) => {
        val startTime = Host.dateNow()
        getComponents().`then` { (components: js.Dynamic) =>
          val result = makeLazyGetResult(components)
          val optionDebug =
            options.isDefined && Host.jsTruthy(options.get.selectDynamic("debug").asInstanceOf[js.Any])
          if (debug.getOrElse(false) || optionDebug) {
            val userAgent = Host.toJsString(Host.navigator.userAgent.asInstanceOf[js.Any])
            Host.consoleLog(
              s"""Copy the text below to get the debug data:
                 |
                 |```
                 |version: ${result.version}
                 |userAgent: $userAgent
                 |timeBetweenLoadAndGet: ${startTime - creationTime}
                 |visitorId: ${result.visitorId}
                 |components: ${componentsToDebugString(components)}
                 |```""".stripMargin,
            )
          }
          result
        }
      },
    )
  }

  def monitor(): Unit = {
    val w = Host.window
    if (Host.jsTruthy(w.selectDynamic("__fpjs_d_m").asInstanceOf[js.Any])) return
    if (js.Math.random() >= 0.001) return
    try {
      val request = js.Dynamic.newInstance(js.Dynamic.global.XMLHttpRequest)()
      request.open("get", s"https://m1.openfpcdn.io/fingerprintjs/v$version/npm-monitoring", true)
      request.send()
    } catch {
      case error: Throwable =>
        js.Dynamic.global.console.error(error.asInstanceOf[js.Any])
    }
  }

  def load(options: js.UndefOr[js.Dynamic] = js.undefined): js.Promise[js.Dynamic] = {
    val opts = options.getOrElse(js.Dynamic.literal())
    val delayFallback = opts.selectDynamic("delayFallback").asInstanceOf[js.UndefOr[Double]]
    val debug = opts.selectDynamic("debug").asInstanceOf[js.UndefOr[Boolean]]
    val monitoring = opts.selectDynamic("monitoring").asInstanceOf[js.Any]
    if (js.isUndefined(monitoring) || Host.jsTruthy(monitoring)) {
      monitor()
    }
    prepareForSources(delayFallback).`then` { (_: Unit) =>
      val sourceOptions = js.Dynamic.literal(cache = js.Dynamic.literal(), debug = debug)
      val getComponents = Sources.loadBuiltin(sourceOptions)
      makeAgent(getComponents, debug)
    }
  }

  val loadFn: js.Function1[js.UndefOr[js.Dynamic], js.Promise[js.Dynamic]] =
    options => load(options)
  val hashComponentsFn: js.Function1[js.Dynamic, String] =
    components => hashComponents(components)
  val componentsToDebugStringFn: js.Function1[js.Any, String] =
    components => componentsToDebugString(components)
  val prepareForSourcesFn: js.Function1[js.UndefOr[Double], js.Promise[Unit]] =
    delay => prepareForSources(delay)
}
