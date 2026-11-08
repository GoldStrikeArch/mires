open Bindings
open Data
open Misc

let isTrident = () => {
  let w = window()
  let n = navigator()
  countTruthy([
    hasKey(w, "MSCSSMatrix"),
    hasKey(w, "msSetImmediate"),
    hasKey(w, "msIndexedDB"),
    hasKey(n, "msMaxTouchPoints"),
    hasKey(n, "msPointerEnabled"),
  ]) >= 4
}

let isEdgeHTML = () => {
  let w = window()
  let n = navigator()
  countTruthy([
    hasKey(w, "msWriteProfilerMark"),
    hasKey(w, "MSStream"),
    hasKey(n, "msLaunchUri"),
    hasKey(n, "msSaveBlob"),
  ]) >= 3 &&
    !isTrident()
}

let isChromium = () => {
  let w = window()
  let n = navigator()
  countTruthy([
    hasKey(n, "webkitPersistentStorage"),
    hasKey(n, "webkitTemporaryStorage"),
    stringIndexOf(asString(orElse(get(n, "vendor"), identity(""))), "Google") == 0,
    hasKey(w, "webkitResolveLocalFileSystemURL"),
    hasKey(w, "BatteryManager"),
    hasKey(w, "webkitMediaStream"),
    hasKey(w, "webkitSpeechGrammar"),
  ]) >= 5
}

let isWebKit = () => {
  let w = window()
  let n = navigator()
  countTruthy([
    hasKey(w, "ApplePayError"),
    hasKey(w, "CSSPrimitiveValue"),
    hasKey(w, "Counter"),
    stringIndexOf(asString(get(n, "vendor")), "Apple") == 0,
    hasKey(w, "RGBColor"),
    hasKey(w, "WebKitMediaKeys"),
  ]) >= 4
}

let isDesktopWebKit = () => {
  let w = window()
  let htmlElement = get(w, "HTMLElement")
  let documentCtor = get(w, "Document")
  countTruthy([
    hasKey(w, "safari"),
    !hasKey(w, "ongestureend"),
    !hasKey(w, "TouchEvent"),
    !hasKey(w, "orientation"),
    isTruthy(htmlElement) && !hasKey(get(htmlElement, "prototype"), "autocapitalize"),
    isTruthy(documentCtor) && hasKey(get(documentCtor, "prototype"), "pointerLockElement"),
  ]) >= 4
}

let isSafariWebKit = () => {
  let w = window()
  isFunctionNative(get(w, "print")) &&
    jsString(get(w, "browser")) == "[object WebPageNamespace]"
}

let isGecko = () => {
  let w = window()
  let el = get(document(), "documentElement")
  let style = if isNullish(el) {
    emptyObject()
  } else {
    coalesce(get(el, "style"), emptyObject())
  }
  countTruthy([
    hasKey(navigator(), "buildID"),
    hasKey(style, "MozAppearance"),
    hasKey(w, "onmozfullscreenchange"),
    hasKey(w, "mozInnerScreenX"),
    hasKey(w, "CSSMozDocumentRule"),
    hasKey(w, "CanvasCaptureMediaStream"),
  ]) >= 4
}

let isGecko120OrNewer = () => {
  let w = window()
  let n = navigator()
  let css = get(w, "CSS")
  countTruthy([
    hasKey(n, "userActivation"),
    asBool(call2(css, "supports", "color", "light-dark(#000, #fff)")),
    asBool(call2(css, "supports", "height", "1lh")),
    hasKey(n, "globalPrivacyControl"),
  ]) >= 3
}

let isGecko143OrNewer = () => {
  let css = get(window(), "CSS")
  countTruthy([
    asBool(call1(css, "supports", "selector(::details-content)")),
    asBool(call1(css, "supports", "selector(::before::marker)")),
    asBool(call1(css, "supports", "selector(::after::marker)")),
    !hasKey(get(get(globalThis, "CompositionEvent"), "prototype"), "locale"),
  ]) >= 3
}

let isChromium86OrNewer = () => {
  let w = window()
  countTruthy([
    !hasKey(w, "MediaSettingsRange"),
    hasKey(w, "RTCEncodedAudioFrame"),
    plusString("", get(w, "Intl")) == "[object Intl]",
    plusString("", get(w, "Reflect")) == "[object Reflect]",
  ]) >= 3
}

let isChromium122OrNewer = () => {
  let w = window()
  let urlPattern = get(w, "URLPattern")
  countTruthy([
    hasKey(get(get(globalThis, "Set"), "prototype"), "union"),
    hasKey(w, "Iterator"),
    isTruthy(urlPattern) && hasKey(get(urlPattern, "prototype"), "hasRegExpGroups"),
    hasKey(get(get(globalThis, "WebGLRenderingContext"), "prototype"), "RGB8"),
  ]) >= 3
}

let isChromium128OrNewer = () => {
  let w = window()
  let d = document()
  let css = get(w, "CSS")
  let promiseCtor = get(w, "Promise")
  let audioContext = get(w, "AudioContext")
  countTruthy([
    isTruthy(promiseCtor) && hasKey(promiseCtor, "try"),
    hasKey(d, "caretPositionFromPoint"),
    isTruthy(audioContext) && hasKey(get(audioContext, "prototype"), "onerror"),
    asBool(call2(css, "supports", "ruby-align", "space-around")),
  ]) >= 3
}

let isWebKit606OrNewer = () => {
  let w = window()
  countTruthy([
    hasKey(w, "DOMRectList"),
    hasKey(w, "RTCPeerConnectionIceEvent"),
    hasKey(w, "SVGGeometryElement"),
    hasKey(w, "ontransitioncancel"),
  ]) >= 3
}

let isWebKit616OrNewer = () => {
  let w = window()
  let n = navigator()
  let css = get(w, "CSS")
  let htmlButton = get(w, "HTMLButtonElement")
  countTruthy([
    !hasKey(n, "getStorageUpdates"),
    isTruthy(htmlButton) && hasKey(get(htmlButton, "prototype"), "popover"),
    hasKey(w, "CSSCounterStyleRule"),
    asBool(call1(css, "supports", "font-size-adjust: ex-height 0.5")),
    asBool(call1(css, "supports", "text-transform: full-width")),
  ]) >= 4
}

let isIPad = () => {
  if asString(get(navigator(), "platform")) == "iPad" {
    true
  } else {
    let s = screen()
    let screenRatio = asFloat(get(s, "width")) /. asFloat(get(s, "height"))
    countTruthy([
      hasKey(window(), "MediaSource"),
      isTruthy(get(get(get(globalThis, "Element"), "prototype"), "webkitRequestFullscreen")),
      screenRatio > 0.65 && screenRatio < 1.53,
    ]) >= 2
  }
}

let getFullscreenElement = () => {
  let d = document()
  orElse(
    get(d, "fullscreenElement"),
    orElse(
      get(d, "msFullscreenElement"),
      orElse(get(d, "mozFullScreenElement"), orElse(get(d, "webkitFullscreenElement"), jsNull)),
    ),
  )
}

let exitFullscreen = () => {
  let d = document()
  let fn = orElse(
    get(d, "exitFullscreen"),
    orElse(get(d, "msExitFullscreen"), orElse(get(d, "mozCancelFullScreen"), get(d, "webkitExitFullscreen"))),
  )
  jsCall0(fn, d)
}

let isAndroid = () => {
  let isItChromium = isChromium()
  let isItGecko = isGecko()
  let w = window()
  let n = navigator()
  if isItChromium {
    countTruthy([
      !hasKey(w, "SharedWorker"),
      {
        let connection = get(n, "connection")
        isTruthy(connection) && hasKey(connection, "ontypechange")
      },
      !hasKey(construct0(get(globalThis, "Audio")), "sinkId"),
    ]) >= 2
  } else if isItGecko {
    countTruthy([
      hasKey(w, "onorientationchange"),
      hasKey(w, "orientation"),
      testRe(newRegExp("android", "i"), asString(get(n, "appVersion"))),
    ]) >= 2
  } else {
    false
  }
}

let isSamsungInternet = () => {
  let n = navigator()
  let w = window()
  let audioPrototype = get(get(globalThis, "Audio"), "prototype")
  let visualViewport = get(w, "visualViewport")
  countTruthy([
    hasKey(audioPrototype, "srLatency"),
    hasKey(audioPrototype, "srChannelCount"),
    hasKey(n, "devicePosture"),
    isTruthy(visualViewport) && hasKey(visualViewport, "segments"),
    hasKey(get(get(globalThis, "Image"), "prototype"), "getTextInformation"),
  ]) >= 3
}
