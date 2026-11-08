package fp

import scala.scalajs.js
import scala.scalajs.js.typedarray.{Float32Array, Uint8Array}

object SourcesRest {
  def getOsCpu(_options: js.Any): js.Any = {
    val value = Host.navigator.selectDynamic("oscpu")
    if (Host.isUndef(value.asInstanceOf[js.Any])) js.undefined else value
  }

  def getLanguages(_options: js.Any): js.Array[js.Array[String]] = {
    val n = Host.navigator
    val result = js.Array[js.Array[String]]()
    val language = {
      val a = n.language
      if (!Host.isUndef(a.asInstanceOf[js.Any])) a
      else if (!Host.isUndef(n.userLanguage.asInstanceOf[js.Any])) n.userLanguage
      else if (!Host.isUndef(n.browserLanguage.asInstanceOf[js.Any])) n.browserLanguage
      else n.systemLanguage
    }
    if (!Host.isUndef(language.asInstanceOf[js.Any]))
      result.push(js.Array(Host.toJsString(language.asInstanceOf[js.Any])))
    val languages = n.languages
    if (Host.g.Array.isArray(languages).asInstanceOf[Boolean]) {
      if (!(Browser.isChromium() && Browser.isChromium86OrNewer()))
        result.push(languages.asInstanceOf[js.Array[String]])
    } else if (Host.jsType(languages.asInstanceOf[js.Any]) == "string") {
      val asString = Host.toJsString(languages.asInstanceOf[js.Any])
      if (asString.nonEmpty) result.push(splitCsv(asString))
    }
    result
  }

  private def splitCsv(value: String): js.Array[String] = {
    val parts = value.split(",")
    val out = js.Array[String]()
    var i = 0
    while (i < parts.length) {
      out.push(parts(i))
      i += 1
    }
    out
  }

  def getColorDepth(_options: js.Any): js.Any = Host.screen.colorDepth

  def getDeviceMemory(_options: js.Any): js.Any =
    Data.replaceNaN(Data.toFloat(Host.navigator.selectDynamic("deviceMemory").asInstanceOf[js.Any]), js.undefined)

  def getUnstableScreenResolution(): js.Array[js.Any] = {
    val s = Host.screen
    def parseDimension(value: js.Any): js.Any = Data.replaceNaN(Data.toInt(value), null)
    val dimensions = js.Array(parseDimension(s.width.asInstanceOf[js.Any]), parseDimension(s.height.asInstanceOf[js.Any]))
    dimensions.sort()
    Host.dyn(dimensions).reverse()
    dimensions
  }

  def getScreenResolution(_options: js.Any): js.Any =
    if (Browser.isWebKit() && Browser.isWebKit616OrNewer() && Browser.isSafariWebKit()) js.undefined
    else getUnstableScreenResolution()

  def getUnstableHardwareConcurrency(): js.Any =
    Data.replaceNaN(Data.toInt(Host.navigator.hardwareConcurrency.asInstanceOf[js.Any]), js.undefined)

  def getHardwareConcurrency(_options: js.Any): js.Any = {
    val value = getUnstableHardwareConcurrency()
    if (!Host.isUndef(value) && Browser.isGecko() && Browser.isGecko143OrNewer()) {
      val n = Host.asNumber(value)
      if (n >= 8) 8 else 4
    } else value
  }

  def getTimezone(_options: js.Any): String = {
    val intl = Host.window.Intl
    val DateTimeFormat =
      if (Host.jsTruthy(intl.asInstanceOf[js.Any])) intl.selectDynamic("DateTimeFormat")
      else js.undefined
    if (Host.jsTruthy(DateTimeFormat.asInstanceOf[js.Any])) {
      val timezone = Host.jsNew(DateTimeFormat)().resolvedOptions().timeZone
      if (Host.jsTruthy(timezone.asInstanceOf[js.Any])) return Host.toJsString(timezone.asInstanceOf[js.Any])
    }
    val offset = -getTimezoneOffset()
    val sign = if (offset >= 0) "+" else ""
    s"UTC$sign$offset"
  }

  private def getTimezoneOffset(): Double = {
    val currentYear = new js.Date().getFullYear().toInt
    math.max(
      Data.toFloat(new js.Date(currentYear, 0, 1).getTimezoneOffset().asInstanceOf[js.Any]),
      Data.toFloat(new js.Date(currentYear, 6, 1).getTimezoneOffset().asInstanceOf[js.Any]),
    )
  }

  def getSessionStorage(_options: js.Any): Boolean =
    try Host.jsTruthy(Host.window.sessionStorage.asInstanceOf[js.Any])
    catch { case _: Throwable => true }

  def getLocalStorage(_options: js.Any): Boolean =
    try Host.jsTruthy(Host.window.localStorage.asInstanceOf[js.Any])
    catch { case _: Throwable => true }

  def getIndexedDB(_options: js.Any): js.Any =
    if (Browser.isTrident() || Browser.isEdgeHTML()) js.undefined
    else
      try Host.jsTruthy(Host.window.indexedDB.asInstanceOf[js.Any])
      catch { case _: Throwable => true }

  def getOpenDatabase(_options: js.Any): Boolean =
    Host.g.Boolean(Host.window.openDatabase).asInstanceOf[Boolean]

  def getCpuClass(_options: js.Any): js.Any = {
    val value = Host.navigator.selectDynamic("cpuClass")
    if (Host.isUndef(value.asInstanceOf[js.Any])) js.undefined else value
  }

  def getPlatform(_options: js.Any): String = {
    val platform = Host.toJsString(Host.navigator.platform.asInstanceOf[js.Any])
    if (platform == "MacIntel" && Browser.isWebKit() && !Browser.isDesktopWebKit())
      if (Browser.isIPad()) "iPad" else "iPhone"
    else platform
  }

  def getPlugins(_options: js.Any): js.Any = {
    val rawPlugins = Host.navigator.plugins
    if (!Host.jsTruthy(rawPlugins.asInstanceOf[js.Any])) return js.undefined
    val plugins = js.Array[js.Dynamic]()
    val length = rawPlugins.length.asInstanceOf[Int]
    var i = 0
    while (i < length) {
      val plugin = rawPlugins.selectDynamic(i.toString)
      if (Host.jsTruthy(plugin.asInstanceOf[js.Any])) {
        val mimeTypes = js.Array[js.Dynamic]()
        val mimeLength = plugin.length.asInstanceOf[Int]
        var j = 0
        while (j < mimeLength) {
          val mimeType = plugin.selectDynamic(j.toString)
          mimeTypes.push(
            js.Dynamic.literal(
              `type` = mimeType.`type`,
              suffixes = mimeType.suffixes,
            ),
          )
          j += 1
        }
        plugins.push(
          js.Dynamic.literal(
            name = plugin.name,
            description = plugin.description,
            mimeTypes = mimeTypes,
          ),
        )
      }
      i += 1
    }
    plugins
  }

  def getTouchSupport(_options: js.Any): js.Dynamic = {
    val n = Host.navigator
    var maxTouchPoints = 0.0
    if (!Host.isUndef(n.maxTouchPoints.asInstanceOf[js.Any]))
      maxTouchPoints = Data.toInt(n.maxTouchPoints.asInstanceOf[js.Any])
    else if (!Host.isUndef(n.msMaxTouchPoints.asInstanceOf[js.Any]))
      maxTouchPoints = Host.asNumber(n.msMaxTouchPoints.asInstanceOf[js.Any])
    val touchEvent =
      try {
        Host.document.createEvent("TouchEvent")
        true
      } catch { case _: Throwable => false }
    val touchStart = Host.has(Host.window, "ontouchstart")
    js.Dynamic.literal(
      maxTouchPoints = maxTouchPoints,
      touchEvent = touchEvent,
      touchStart = touchStart,
    )
  }

  def getVendor(_options: js.Any): String = Host.orEmpty(Host.navigator.vendor.asInstanceOf[js.Any])

  def getVendorFlavors(_options: js.Any): js.Array[String] = {
    val flavors = js.Array[String]()
    val keys = js.Array(
      "chrome",
      "safari",
      "__crWeb",
      "__gCrWeb",
      "yandex",
      "__yb",
      "__ybro",
      "__firefox__",
      "__edgeTrackingPreventionStatistics",
      "webkit",
      "oprt",
      "samsungAr",
      "ucweb",
      "UCShellJava",
      "puffinDevice",
    )
    var i = 0
    while (i < keys.length) {
      val key = keys(i)
      val value = Host.window.selectDynamic(key)
      if (Host.jsTruthy(value.asInstanceOf[js.Any]) && Host.jsType(value.asInstanceOf[js.Any]) == "object")
        flavors.push(key)
      i += 1
    }
    flavors.sort()
    flavors
  }

  def areCookiesEnabled(_options: js.Any): Boolean = {
    val d = Host.document
    try {
      d.cookie = "cookietest=1; SameSite=Strict;"
      val result = Host.toJsString(d.cookie.asInstanceOf[js.Any]).indexOf("cookietest=") != -1
      d.cookie = "cookietest=1; SameSite=Strict; expires=Thu, 01-Jan-1970 00:00:01 GMT"
      result
    } catch { case _: Throwable => false }
  }

  def getColorGamut(_options: js.Any): js.Any = {
    val gamuts = js.Array("rec2020", "p3", "srgb")
    var i = 0
    while (i < gamuts.length) {
      if (Host.mediaMatches(s"(color-gamut: ${gamuts(i)})")) return gamuts(i)
      i += 1
    }
    js.undefined
  }

  def areColorsInverted(_options: js.Any): js.Any =
    if (Host.mediaMatches("(inverted-colors: inverted)")) true
    else if (Host.mediaMatches("(inverted-colors: none)")) false
    else js.undefined

  def areColorsForced(_options: js.Any): js.Any =
    if (Host.mediaMatches("(forced-colors: active)")) true
    else if (Host.mediaMatches("(forced-colors: none)")) false
    else js.undefined

  def getMonochromeDepth(_options: js.Any): js.Any = {
    if (!Host.mediaMatches("(min-monochrome: 0)")) return js.undefined
    var i = 0
    while (i <= 100) {
      if (Host.mediaMatches(s"(max-monochrome: $i)")) return i
      i += 1
    }
    throw js.JavaScriptException(new js.Error("Too high value"))
  }

  def getContrastPreference(_options: js.Any): js.Any =
    if (Host.mediaMatches("(prefers-contrast: no-preference)")) 0
    else if (Host.mediaMatches("(prefers-contrast: high)") || Host.mediaMatches("(prefers-contrast: more)")) 1
    else if (Host.mediaMatches("(prefers-contrast: low)") || Host.mediaMatches("(prefers-contrast: less)")) -1
    else if (Host.mediaMatches("(prefers-contrast: forced)")) 10
    else js.undefined

  def isMotionReduced(_options: js.Any): js.Any =
    if (Host.mediaMatches("(prefers-reduced-motion: reduce)")) true
    else if (Host.mediaMatches("(prefers-reduced-motion: no-preference)")) false
    else js.undefined

  def isTransparencyReduced(_options: js.Any): js.Any =
    if (Host.mediaMatches("(prefers-reduced-transparency: reduce)")) true
    else if (Host.mediaMatches("(prefers-reduced-transparency: no-preference)")) false
    else js.undefined

  def isHDR(_options: js.Any): js.Any =
    if (Host.mediaMatches("(dynamic-range: high)")) true
    else if (Host.mediaMatches("(dynamic-range: standard)")) false
    else js.undefined

  def getMathFingerprint(_options: js.Any): js.Dynamic = {
    val M = Host.g.Math
    def fn(name: String): js.Function1[Double, Double] = {
      val f = M.selectDynamic(name)
      if (Host.isJsFunction(f.asInstanceOf[js.Any])) f.asInstanceOf[js.Function1[Double, Double]]
      else (_: Double) => 0
    }
    val acos = fn("acos")
    val acosh = fn("acosh")
    val asin = fn("asin")
    val asinh = fn("asinh")
    val atanh = fn("atanh")
    val atan = fn("atan")
    val sin = fn("sin")
    val sinh = fn("sinh")
    val cos = fn("cos")
    val cosh = fn("cosh")
    val tan = fn("tan")
    val tanh = fn("tanh")
    val exp = fn("exp")
    val expm1 = fn("expm1")
    val log1p = fn("log1p")
    def num(value: js.Any): Double = value.asInstanceOf[Double]
    def powPI(value: Double): Double = num(M.pow(M.PI, value))
    def acoshPf(value: Double): Double =
      num(M.log(value + num(M.sqrt(value * value - 1))))
    def asinhPf(value: Double): Double =
      num(M.log(value + num(M.sqrt(value * value + 1))))
    def atanhPf(value: Double): Double =
      num(M.log((1 + value) / (1 - value))) / 2
    def sinhPf(value: Double): Double =
      num(M.exp(value)) - 1 / num(M.exp(value)) / 2
    def coshPf(value: Double): Double =
      (num(M.exp(value)) + 1 / num(M.exp(value))) / 2
    def expm1Pf(value: Double): Double = num(M.exp(value)) - 1
    def tanhPf(value: Double): Double =
      (num(M.exp(2 * value)) - 1) / (num(M.exp(2 * value)) + 1)
    def log1pPf(value: Double): Double = num(M.log(1 + value))
    js.Dynamic.literal(
      acos = acos(0.123124234234234242),
      acosh = acosh(1e308),
      acoshPf = acoshPf(1e154),
      asin = asin(0.123124234234234242),
      asinh = asinh(1),
      asinhPf = asinhPf(1),
      atanh = atanh(0.5),
      atanhPf = atanhPf(0.5),
      atan = atan(0.5),
      sin = sin(-1e300),
      sinh = sinh(1),
      sinhPf = sinhPf(1),
      cos = cos(10.000000000123),
      cosh = cosh(1),
      coshPf = coshPf(1),
      tan = tan(-1e300),
      tanh = tanh(1),
      tanhPf = tanhPf(1),
      exp = exp(1),
      expm1 = expm1(1),
      expm1Pf = expm1Pf(1),
      log1p = log1p(10),
      log1pPf = log1pPf(10),
      powPI = powPI(-100),
    )
  }

  def isPdfViewerEnabled(_options: js.Any): js.Any = {
    val value = Host.navigator.selectDynamic("pdfViewerEnabled")
    if (Host.isUndef(value.asInstanceOf[js.Any])) js.undefined else value
  }

  def getArchitecture(_options: js.Any): Int = {
    val f = new Float32Array(1)
    val u8 = new Uint8Array(f.buffer)
    f(0) = Float.PositiveInfinity
    f(0) = (f(0) - f(0)).toFloat
    u8(3).toInt
  }

  def getApplePayState(_options: js.Any): Int = {
    val ApplePaySession = Host.window.ApplePaySession
    if (!Host.isJsFunction(Host.get(ApplePaySession.asInstanceOf[js.Any], "canMakePayments"))) return -1
    if (Dom.isAnyParentCrossOrigin()) return -3
    try
      if (ApplePaySession.canMakePayments().asInstanceOf[Boolean]) 1 else 0
    catch {
      case js.JavaScriptException(error) => getStateFromError(error.asInstanceOf[js.Any])
      case error: Throwable              => getStateFromError(error.asInstanceOf[js.Any])
    }
  }

  private def getStateFromError(error: js.Any): Int = {
    if (
      Host.instanceofError(error) &&
      Host.toJsString(Host.dyn(error).name.asInstanceOf[js.Any]) == "InvalidAccessError" &&
      new js.RegExp("""\bfrom\b.*\binsecure\b""", "i").test(Host.toJsString(Host.dyn(error).message.asInstanceOf[js.Any]))
    ) return -2
    throw js.JavaScriptException(error)
  }

  def getPrivateClickMeasurement(_options: js.Any): js.Any = {
    val link = Host.document.createElement("a")
    val sourceId = {
      val a = link.selectDynamic("attributionSourceId")
      if (!Host.isUndef(a.asInstanceOf[js.Any])) a else link.selectDynamic("attributionsourceid")
    }
    if (Host.isUndef(sourceId.asInstanceOf[js.Any])) js.undefined
    else Host.toJsString(sourceId.asInstanceOf[js.Any])
  }

  def getAudioContextBaseLatency(_options: js.Any): Double = {
    val isAllowedPlatform = Browser.isAndroid() || Browser.isWebKit()
    if (!isAllowedPlatform) return -2
    if (!Host.jsTruthy(Host.window.AudioContext.asInstanceOf[js.Any])) return -1
    val latency = Host.jsNew(Host.window.AudioContext)().baseLatency.asInstanceOf[js.Any]
    if (latency == null || Host.isUndef(latency)) return -1
    val n = Host.asNumber(latency)
    if (!n.isFinite) -3 else n
  }

  def getDateTimeLocale(_options: js.Any): js.Any = {
    if (!Host.jsTruthy(Host.window.Intl.asInstanceOf[js.Any])) return -1
    val DateTimeFormat = Host.window.Intl.DateTimeFormat
    if (!Host.jsTruthy(DateTimeFormat.asInstanceOf[js.Any])) return -2
    val locale = Host.jsNew(DateTimeFormat)().resolvedOptions().locale
    if (!Host.jsTruthy(locale.asInstanceOf[js.Any]) && Host.toJsString(locale.asInstanceOf[js.Any]) != "") return -3
    locale
  }

  val getUnstableHardwareConcurrencyFn: js.Function0[js.Any] = () => getUnstableHardwareConcurrency()
  val getUnstableScreenResolutionFn: js.Function0[js.Array[js.Any]] = () => getUnstableScreenResolution()
}
