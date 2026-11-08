package fp

import scala.scalajs.js

object Browser {
  def isTrident(): Boolean = {
    val w = Host.window
    val n = Host.navigator
    Data.countTruthyAny(
      Host.has(w, "MSCSSMatrix"),
      Host.has(w, "msSetImmediate"),
      Host.has(w, "msIndexedDB"),
      Host.has(n, "msMaxTouchPoints"),
      Host.has(n, "msPointerEnabled"),
    ) >= 4
  }

  def isEdgeHTML(): Boolean = {
    val w = Host.window
    val n = Host.navigator
    Data.countTruthyAny(
      Host.has(w, "msWriteProfilerMark"),
      Host.has(w, "MSStream"),
      Host.has(n, "msLaunchUri"),
      Host.has(n, "msSaveBlob"),
    ) >= 3 && !isTrident()
  }

  def isChromium(): Boolean = {
    val w = Host.window
    val n = Host.navigator
    Data.countTruthyAny(
      Host.has(n, "webkitPersistentStorage"),
      Host.has(n, "webkitTemporaryStorage"),
      Host.orEmpty(n.vendor).indexOf("Google") == 0,
      Host.has(w, "webkitResolveLocalFileSystemURL"),
      Host.has(w, "BatteryManager"),
      Host.has(w, "webkitMediaStream"),
      Host.has(w, "webkitSpeechGrammar"),
    ) >= 5
  }

  def isWebKit(): Boolean = {
    val w = Host.window
    val n = Host.navigator
    Data.countTruthyAny(
      Host.has(w, "ApplePayError"),
      Host.has(w, "CSSPrimitiveValue"),
      Host.has(w, "Counter"),
      Host.toJsString(n.vendor.asInstanceOf[js.Any]).indexOf("Apple") == 0,
      Host.has(w, "RGBColor"),
      Host.has(w, "WebKitMediaKeys"),
    ) >= 4
  }

  def isDesktopWebKit(): Boolean = {
    val w = Host.window
    val HTMLElement = w.HTMLElement
    val Document = w.Document
    Data.countTruthyAny(
      Host.has(w, "safari"),
      !Host.has(w, "ongestureend"),
      !Host.has(w, "TouchEvent"),
      !Host.has(w, "orientation"),
      Host.jsTruthy(HTMLElement.asInstanceOf[js.Any]) &&
        !Host.has(HTMLElement.prototype.asInstanceOf[js.Any], "autocapitalize"),
      Host.jsTruthy(Document.asInstanceOf[js.Any]) &&
        Host.has(Document.prototype.asInstanceOf[js.Any], "pointerLockElement"),
    ) >= 4
  }

  def isSafariWebKit(): Boolean = {
    val w = Host.window
    Misc.isFunctionNative(w.print.asInstanceOf[js.Any]) &&
    Host.toJsString(w.browser.asInstanceOf[js.Any]) == "[object WebPageNamespace]"
  }

  def isGecko(): Boolean = {
    val w = Host.window
    val style =
      if (Host.isUndef(Host.document.documentElement.asInstanceOf[js.Any])) js.Dynamic.literal()
      else {
        val s = Host.document.documentElement.selectDynamic("style")
        if (Host.isUndef(s.asInstanceOf[js.Any])) js.Dynamic.literal() else s
      }
    Data.countTruthyAny(
      Host.has(Host.navigator, "buildID"),
      Host.has(style.asInstanceOf[js.Any], "MozAppearance"),
      Host.has(w, "onmozfullscreenchange"),
      Host.has(w, "mozInnerScreenX"),
      Host.has(w, "CSSMozDocumentRule"),
      Host.has(w, "CanvasCaptureMediaStream"),
    ) >= 4
  }

  def isGecko120OrNewer(): Boolean = {
    val w = Host.window
    val n = Host.navigator
    val CSS = w.CSS
    Data.countTruthyAny(
      Host.has(n, "userActivation"),
      Host.jsTruthy(CSS.asInstanceOf[js.Any]) &&
        CSS.supports("color", "light-dark(#000, #fff)").asInstanceOf[Boolean],
      Host.jsTruthy(CSS.asInstanceOf[js.Any]) &&
        CSS.supports("height", "1lh").asInstanceOf[Boolean],
      Host.has(n, "globalPrivacyControl"),
    ) >= 3
  }

  def isGecko143OrNewer(): Boolean = {
    val CSS = Host.window.CSS
    Data.countTruthyAny(
      Host.jsTruthy(CSS.asInstanceOf[js.Any]) &&
        CSS.supports("selector(::details-content)").asInstanceOf[Boolean],
      Host.jsTruthy(CSS.asInstanceOf[js.Any]) &&
        CSS.supports("selector(::before::marker)").asInstanceOf[Boolean],
      Host.jsTruthy(CSS.asInstanceOf[js.Any]) &&
        CSS.supports("selector(::after::marker)").asInstanceOf[Boolean],
      !Host.has(Host.g.CompositionEvent.prototype.asInstanceOf[js.Any], "locale"),
    ) >= 3
  }

  def isChromium86OrNewer(): Boolean = {
    val w = Host.window
    Data.countTruthyAny(
      !Host.has(w, "MediaSettingsRange"),
      Host.has(w, "RTCEncodedAudioFrame"),
      ("" + w.Intl) == "[object Intl]",
      ("" + w.Reflect) == "[object Reflect]",
    ) >= 3
  }

  def isChromium122OrNewer(): Boolean = {
    val w = Host.window
    val URLPattern = w.URLPattern
    Data.countTruthyAny(
      Host.has(Host.g.Set.prototype.asInstanceOf[js.Any], "union"),
      Host.has(w, "Iterator"),
      Host.jsTruthy(URLPattern.asInstanceOf[js.Any]) &&
        Host.has(URLPattern.prototype.asInstanceOf[js.Any], "hasRegExpGroups"),
      Host.has(Host.g.WebGLRenderingContext.prototype.asInstanceOf[js.Any], "RGB8"),
    ) >= 3
  }

  def isChromium128OrNewer(): Boolean = {
    val w = Host.window
    val d = Host.document
    val CSS = w.CSS
    val Promise = w.Promise
    val AudioContext = w.AudioContext
    Data.countTruthyAny(
      Host.jsTruthy(Promise.asInstanceOf[js.Any]) && Host.has(Promise.asInstanceOf[js.Any], "try"),
      Host.has(d, "caretPositionFromPoint"),
      Host.jsTruthy(AudioContext.asInstanceOf[js.Any]) &&
        Host.has(AudioContext.prototype.asInstanceOf[js.Any], "onerror"),
      Host.jsTruthy(CSS.asInstanceOf[js.Any]) &&
        CSS.supports("ruby-align", "space-around").asInstanceOf[Boolean],
    ) >= 3
  }

  def isWebKit606OrNewer(): Boolean = {
    val w = Host.window
    Data.countTruthyAny(
      Host.has(w, "DOMRectList"),
      Host.has(w, "RTCPeerConnectionIceEvent"),
      Host.has(w, "SVGGeometryElement"),
      Host.has(w, "ontransitioncancel"),
    ) >= 3
  }

  def isWebKit616OrNewer(): Boolean = {
    val w = Host.window
    val n = Host.navigator
    val CSS = w.CSS
    val HTMLButtonElement = w.HTMLButtonElement
    Data.countTruthyAny(
      !Host.has(n, "getStorageUpdates"),
      Host.jsTruthy(HTMLButtonElement.asInstanceOf[js.Any]) &&
        Host.has(HTMLButtonElement.prototype.asInstanceOf[js.Any], "popover"),
      Host.has(w, "CSSCounterStyleRule"),
      Host.jsTruthy(CSS.asInstanceOf[js.Any]) &&
        CSS.supports("font-size-adjust: ex-height 0.5").asInstanceOf[Boolean],
      Host.jsTruthy(CSS.asInstanceOf[js.Any]) &&
        CSS.supports("text-transform: full-width").asInstanceOf[Boolean],
    ) >= 4
  }

  def isIPad(): Boolean = {
    if (Host.toJsString(Host.navigator.platform.asInstanceOf[js.Any]) == "iPad") return true
    val s = Host.screen
    val screenRatio = Host.asNumber(s.width.asInstanceOf[js.Any]) / Host.asNumber(s.height.asInstanceOf[js.Any])
    Data.countTruthyAny(
      Host.has(Host.window, "MediaSource"),
      Host.jsTruthy(Host.g.Element.prototype.webkitRequestFullscreen.asInstanceOf[js.Any]),
      screenRatio > 0.65 && screenRatio < 1.53,
    ) >= 2
  }

  def getFullscreenElement(): js.Any = {
    val d = Host.document
    val a = d.fullscreenElement
    if (Host.jsTruthy(a.asInstanceOf[js.Any])) a
    else {
      val b = d.msFullscreenElement
      if (Host.jsTruthy(b.asInstanceOf[js.Any])) b
      else {
        val c = d.mozFullScreenElement
        if (Host.jsTruthy(c.asInstanceOf[js.Any])) c
        else {
          val e = d.webkitFullscreenElement
          if (Host.jsTruthy(e.asInstanceOf[js.Any])) e else null
        }
      }
    }
  }

  def exitFullscreen(): js.Promise[Unit] = {
    val d = Host.document
    val fn =
      if (Host.isJsFunction(d.exitFullscreen.asInstanceOf[js.Any])) d.exitFullscreen
      else if (Host.isJsFunction(d.msExitFullscreen.asInstanceOf[js.Any])) d.msExitFullscreen
      else if (Host.isJsFunction(d.mozCancelFullScreen.asInstanceOf[js.Any])) d.mozCancelFullScreen
      else d.webkitExitFullscreen
    Host.dyn(fn).call(d).asInstanceOf[js.Promise[Unit]]
  }

  def isAndroid(): Boolean = {
    val isItChromium = isChromium()
    val isItGecko = isGecko()
    val w = Host.window
    val n = Host.navigator
    if (isItChromium) {
      val connection = n.selectDynamic("connection")
      Data.countTruthyAny(
        !Host.has(w, "SharedWorker"),
        Host.jsTruthy(connection.asInstanceOf[js.Any]) && Host.has(connection.asInstanceOf[js.Any], "ontypechange"),
        !Host.has(Host.jsNew(Host.g.Audio)().asInstanceOf[js.Any], "sinkId"),
      ) >= 2
    } else if (isItGecko) {
      Data.countTruthyAny(
        Host.has(w, "onorientationchange"),
        Host.has(w, "orientation"),
        new js.RegExp("android", "i").test(Host.toJsString(n.appVersion.asInstanceOf[js.Any])),
      ) >= 2
    } else false
  }

  def isSamsungInternet(): Boolean = {
    val n = Host.navigator
    val w = Host.window
    val audioPrototype = Host.g.Audio.prototype
    val visualViewport = w.visualViewport
    Data.countTruthyAny(
      Host.has(audioPrototype.asInstanceOf[js.Any], "srLatency"),
      Host.has(audioPrototype.asInstanceOf[js.Any], "srChannelCount"),
      Host.has(n, "devicePosture"),
      Host.jsTruthy(visualViewport.asInstanceOf[js.Any]) &&
        Host.has(visualViewport.asInstanceOf[js.Any], "segments"),
      Host.has(Host.g.Image.prototype.asInstanceOf[js.Any], "getTextInformation"),
    ) >= 3
  }

  val getFullscreenElementFn: js.Function0[js.Any] = () => getFullscreenElement()
  val isAndroidFn: js.Function0[Boolean] = () => isAndroid()
  val isChromiumFn: js.Function0[Boolean] = () => isChromium()
  val isDesktopWebKitFn: js.Function0[Boolean] = () => isDesktopWebKit()
  val isEdgeHTMLFn: js.Function0[Boolean] = () => isEdgeHTML()
  val isGeckoFn: js.Function0[Boolean] = () => isGecko()
  val isSamsungInternetFn: js.Function0[Boolean] = () => isSamsungInternet()
  val isTridentFn: js.Function0[Boolean] = () => isTrident()
  val isWebKitFn: js.Function0[Boolean] = () => isWebKit()
}
