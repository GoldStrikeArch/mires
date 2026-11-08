import { countTruthy } from './data.js'
import { isFunctionNative } from './misc.js'

export function isTrident() {
  const w = window
  const n = navigator
  return (
    countTruthy([
      'MSCSSMatrix' in w,
      'msSetImmediate' in w,
      'msIndexedDB' in w,
      'msMaxTouchPoints' in n,
      'msPointerEnabled' in n,
    ]) >= 4
  )
}

export function isEdgeHTML() {
  const w = window
  const n = navigator
  return (
    countTruthy(['msWriteProfilerMark' in w, 'MSStream' in w, 'msLaunchUri' in n, 'msSaveBlob' in n]) >= 3 &&
    !isTrident()
  )
}

export function isChromium() {
  const w = window
  const n = navigator
  return (
    countTruthy([
      'webkitPersistentStorage' in n,
      'webkitTemporaryStorage' in n,
      (n.vendor || '').indexOf('Google') === 0,
      'webkitResolveLocalFileSystemURL' in w,
      'BatteryManager' in w,
      'webkitMediaStream' in w,
      'webkitSpeechGrammar' in w,
    ]) >= 5
  )
}

export function isWebKit() {
  const w = window
  const n = navigator
  return (
    countTruthy([
      'ApplePayError' in w,
      'CSSPrimitiveValue' in w,
      'Counter' in w,
      n.vendor.indexOf('Apple') === 0,
      'RGBColor' in w,
      'WebKitMediaKeys' in w,
    ]) >= 4
  )
}

export function isDesktopWebKit() {
  const w = window
  const { HTMLElement, Document } = w
  return (
    countTruthy([
      'safari' in w,
      !('ongestureend' in w),
      !('TouchEvent' in w),
      !('orientation' in w),
      HTMLElement && !('autocapitalize' in HTMLElement.prototype),
      Document && 'pointerLockElement' in Document.prototype,
    ]) >= 4
  )
}

export function isSafariWebKit() {
  const w = window
  return isFunctionNative(w.print) && String(w.browser) === '[object WebPageNamespace]'
}

export function isGecko() {
  const w = window
  return (
    countTruthy([
      'buildID' in navigator,
      'MozAppearance' in (document.documentElement?.style ?? {}),
      'onmozfullscreenchange' in w,
      'mozInnerScreenX' in w,
      'CSSMozDocumentRule' in w,
      'CanvasCaptureMediaStream' in w,
    ]) >= 4
  )
}

export function isGecko120OrNewer() {
  const w = window
  const n = navigator
  const { CSS } = w
  return (
    countTruthy([
      'userActivation' in n,
      CSS.supports('color', 'light-dark(#000, #fff)'),
      CSS.supports('height', '1lh'),
      'globalPrivacyControl' in n,
    ]) >= 3
  )
}

export function isGecko143OrNewer() {
  const { CSS } = window
  return (
    countTruthy([
      CSS.supports('selector(::details-content)'),
      CSS.supports('selector(::before::marker)'),
      CSS.supports('selector(::after::marker)'),
      !('locale' in CompositionEvent.prototype),
    ]) >= 3
  )
}

export function isChromium86OrNewer() {
  const w = window
  return (
    countTruthy([
      !('MediaSettingsRange' in w),
      'RTCEncodedAudioFrame' in w,
      '' + w.Intl === '[object Intl]',
      '' + w.Reflect === '[object Reflect]',
    ]) >= 3
  )
}

export function isChromium122OrNewer() {
  const w = window
  const { URLPattern } = w
  return (
    countTruthy([
      'union' in Set.prototype,
      'Iterator' in w,
      URLPattern && 'hasRegExpGroups' in URLPattern.prototype,
      'RGB8' in WebGLRenderingContext.prototype,
    ]) >= 3
  )
}

export function isChromium128OrNewer() {
  const w = window
  const d = document
  const { CSS, Promise, AudioContext } = w
  return (
    countTruthy([
      Promise && 'try' in Promise,
      'caretPositionFromPoint' in d,
      AudioContext && 'onerror' in AudioContext.prototype,
      CSS.supports('ruby-align', 'space-around'),
    ]) >= 3
  )
}

export function isWebKit606OrNewer() {
  const w = window
  return (
    countTruthy([
      'DOMRectList' in w,
      'RTCPeerConnectionIceEvent' in w,
      'SVGGeometryElement' in w,
      'ontransitioncancel' in w,
    ]) >= 3
  )
}

export function isWebKit616OrNewer() {
  const w = window
  const n = navigator
  const { CSS, HTMLButtonElement } = w
  return (
    countTruthy([
      !('getStorageUpdates' in n),
      HTMLButtonElement && 'popover' in HTMLButtonElement.prototype,
      'CSSCounterStyleRule' in w,
      CSS.supports('font-size-adjust: ex-height 0.5'),
      CSS.supports('text-transform: full-width'),
    ]) >= 4
  )
}

export function isIPad() {
  if (navigator.platform === 'iPad') return true
  const s = screen
  const screenRatio = s.width / s.height
  return (
    countTruthy([
      'MediaSource' in window,
      !!Element.prototype.webkitRequestFullscreen,
      screenRatio > 0.65 && screenRatio < 1.53,
    ]) >= 2
  )
}

export function getFullscreenElement() {
  const d = document
  return d.fullscreenElement || d.msFullscreenElement || d.mozFullScreenElement || d.webkitFullscreenElement || null
}

export function exitFullscreen() {
  const d = document
  return (d.exitFullscreen || d.msExitFullscreen || d.mozCancelFullScreen || d.webkitExitFullscreen).call(d)
}

export function isAndroid() {
  const isItChromium = isChromium()
  const isItGecko = isGecko()
  const w = window
  const n = navigator
  const c = 'connection'
  if (isItChromium) {
    return (
      countTruthy([
        !('SharedWorker' in w),
        n[c] && 'ontypechange' in n[c],
        !('sinkId' in new Audio()),
      ]) >= 2
    )
  }
  if (isItGecko) {
    return countTruthy(['onorientationchange' in w, 'orientation' in w, /android/i.test(n.appVersion)]) >= 2
  }
  return false
}

export function isSamsungInternet() {
  const n = navigator
  const w = window
  const audioPrototype = Audio.prototype
  const { visualViewport } = w
  return (
    countTruthy([
      'srLatency' in audioPrototype,
      'srChannelCount' in audioPrototype,
      'devicePosture' in n,
      visualViewport && 'segments' in visualViewport,
      'getTextInformation' in Image.prototype,
    ]) >= 3
  )
}
