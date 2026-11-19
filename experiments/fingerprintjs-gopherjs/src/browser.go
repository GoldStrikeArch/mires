package main

import (
	"strings"

	"github.com/gopherjs/gopherjs/js"
)

// Functions that tell browsers apart by their features, without using the user agent.
// Each detector evaluates every one of its probes before counting them, as the oracle's
// countTruthy([...]) array does.

func prototypeOf(constructor *js.Object) *js.Object { return constructor.Get("prototype") }

// cssSupports is CSS.supports on the window's CSS object.
func cssSupports(args ...any) bool {
	return window().Get("CSS").Call("supports", args...).Bool()
}

// isTrident reports a browser based on Trident, the Internet Explorer engine.
func isTrident() bool {
	w, n := window(), navigator()
	// The properties are checked to be in IE 10, IE 11 and not to be in other browsers in October 2020
	return countTruthy(
		has(w, "MSCSSMatrix"),
		has(w, "msSetImmediate"),
		has(w, "msIndexedDB"),
		has(n, "msMaxTouchPoints"),
		has(n, "msPointerEnabled"),
	) >= 4
}

// isEdgeHTML reports a browser based on EdgeHTML, the pre-Chromium Edge engine.
func isEdgeHTML() bool {
	// Based on research in October 2020
	w, n := window(), navigator()
	return countTruthy(
		has(w, "msWriteProfilerMark"),
		has(w, "MSStream"),
		has(n, "msLaunchUri"),
		has(n, "msSaveBlob"),
	) >= 3 && !isTrident()
}

// isChromium reports a browser based on Chromium.
func isChromium() bool {
	// Based on research in October 2020. Tested to detect Chromium 42-86.
	w, n := window(), navigator()
	vendor := n.Get("vendor")
	if !vendor.Bool() {
		vendor = toJS("")
	}
	return countTruthy(
		has(n, "webkitPersistentStorage"),
		has(n, "webkitTemporaryStorage"),
		vendor.Call("indexOf", "Google").Int() == 0,
		has(w, "webkitResolveLocalFileSystemURL"),
		has(w, "BatteryManager"),
		has(w, "webkitMediaStream"),
		has(w, "webkitSpeechGrammar"),
	) >= 5
}

// isWebKit reports a browser based on WebKit, the Safari engine; all iOS browsers are.
func isWebKit() bool {
	// Based on research in August 2024
	w, n := window(), navigator()
	return countTruthy(
		has(w, "ApplePayError"),
		has(w, "CSSPrimitiveValue"),
		has(w, "Counter"),
		n.Get("vendor").Call("indexOf", "Apple").Int() == 0,
		has(w, "RGBColor"),
		has(w, "WebKitMediaKeys"),
	) >= 4
}

// isDesktopWebKit reports whether a WebKit browser is a desktop browser. It doesn't check
// that the browser is based on WebKit.
func isDesktopWebKit() bool {
	// Checked in Safari and DuckDuckGo
	w := window()
	htmlElement, document := w.Get("HTMLElement"), w.Get("Document")
	return countTruthy(
		has(w, "safari"), // Always false in Karma and BrowserStack Automate
		!has(w, "ongestureend"),
		!has(w, "TouchEvent"),
		!has(w, "orientation"),
		htmlElement.Bool() && !has(prototypeOf(htmlElement), "autocapitalize"),
		document.Bool() && has(prototypeOf(document), "pointerLockElement"),
	) >= 4
}

// isSafariWebKit reports whether a WebKit browser is Safari. It works properly only for
// Safari 15.4 and newer.
func isSafariWebKit() bool {
	w := window()
	// isFunctionNative filters out Chrome, Yandex, DuckDuckGo (macOS and iOS) and Edge
	return isFunctionNative(w.Get("print")) &&
		// Doesn't work in Safari < 15.4
		js.Global.Call("String", w.Get("browser")).String() == "[object WebPageNamespace]"
}

// isGecko reports a browser based on Gecko, the Firefox engine.
func isGecko() bool {
	w := window()
	style := newObject()
	if documentElement := document().Get("documentElement"); !isNullish(documentElement) {
		if s := documentElement.Get("style"); !isNullish(s) {
			style = s
		}
	}
	// Based on research in September 2020
	return countTruthy(
		has(navigator(), "buildID"),
		has(style, "MozAppearance"),
		has(w, "onmozfullscreenchange"),
		has(w, "mozInnerScreenX"),
		has(w, "CSSMozDocumentRule"),
		has(w, "CanvasCaptureMediaStream"),
	) >= 4
}

// isGecko120OrNewer reports Gecko 120 or newer (Firefox 120+). It doesn't check that the
// browser is based on Gecko.
func isGecko120OrNewer() bool {
	// Checked in Firefox 119 vs. Firefox 120+. Three of four, because globalPrivacyControl
	// arrived in Firefox 120 on desktop but only in Firefox 122 on mobile.
	n := navigator()
	return countTruthy(
		has(n, "userActivation"),                       // User Activation API, Firefox 120
		cssSupports("color", "light-dark(#000, #fff)"), // CSS light-dark(), Firefox 120
		cssSupports("height", "1lh"),                   // CSS lh unit, Firefox 120
		has(n, "globalPrivacyControl"),                 // Firefox 120 (desktop) / 122 (mobile)
	) >= 3
}

// isGecko143OrNewer reports Gecko 143 or newer (Firefox 143+), which shipped Phase 2
// fingerprinting protections in Private Browsing and ETP Strict mode. It doesn't check
// that the browser is based on Gecko.
func isGecko143OrNewer() bool {
	// Checked in Firefox 142 vs. Firefox 143+
	return countTruthy(
		cssSupports("selector(::details-content)"),                     // Firefox 143
		cssSupports("selector(::before::marker)"),                      // Firefox 143
		cssSupports("selector(::after::marker)"),                       // Firefox 143
		!has(prototypeOf(js.Global.Get("CompositionEvent")), "locale"), // removed in Firefox 143
	) >= 3
}

// isChromium86OrNewer reports Chromium 86 or newer. It doesn't check that the browser is
// based on Chromium.
func isChromium86OrNewer() bool {
	// Checked in Chrome 85 vs Chrome 86 both on desktop and Android
	w := window()
	return countTruthy(
		!has(w, "MediaSettingsRange"),
		has(w, "RTCEncodedAudioFrame"),
		jsString(w.Get("Intl")) == "[object Intl]",
		jsString(w.Get("Reflect")) == "[object Reflect]",
	) >= 3
}

// isChromium122OrNewer reports Chromium 122 or newer. It doesn't check that the browser is
// based on Chromium.
func isChromium122OrNewer() bool {
	// Checked in Chrome 121 vs Chrome 122 and 129 both on desktop and Android
	w := window()
	urlPattern := w.Get("URLPattern")
	return countTruthy(
		has(prototypeOf(js.Global.Get("Set")), "union"),
		has(w, "Iterator"),
		urlPattern.Bool() && has(prototypeOf(urlPattern), "hasRegExpGroups"),
		has(prototypeOf(js.Global.Get("WebGLRenderingContext")), "RGB8"),
	) >= 3
}

// isChromium128OrNewer reports Chromium 128 or newer. It doesn't check that the browser is
// based on Chromium.
func isChromium128OrNewer() bool {
	// Checked in Chrome 127 vs Chrome 128+
	w, d := window(), document()
	promise, audioContext := w.Get("Promise"), w.Get("AudioContext")
	return countTruthy(
		promise.Bool() && has(promise, "try"),
		has(d, "caretPositionFromPoint"),
		audioContext.Bool() && has(prototypeOf(audioContext), "onerror"),
		cssSupports("ruby-align", "space-around"),
	) >= 3
}

// isWebKit606OrNewer reports WebKit 606 or newer (Safari 12+). It doesn't check that the
// browser is based on WebKit.
func isWebKit606OrNewer() bool {
	// Checked in Safari 9–18
	w := window()
	return countTruthy(
		has(w, "DOMRectList"),
		has(w, "RTCPeerConnectionIceEvent"),
		has(w, "SVGGeometryElement"),
		has(w, "ontransitioncancel"),
	) >= 3
}

// isWebKit616OrNewer reports WebKit 616 or newer (Safari 17+). It doesn't check that the
// browser is based on WebKit.
func isWebKit616OrNewer() bool {
	w, n := window(), navigator()
	htmlButtonElement := w.Get("HTMLButtonElement")
	return countTruthy(
		!has(n, "getStorageUpdates"),
		htmlButtonElement.Bool() && has(prototypeOf(htmlButtonElement), "popover"),
		has(w, "CSSCounterStyleRule"),
		cssSupports("font-size-adjust: ex-height 0.5"),
		cssSupports("text-transform: full-width"),
	) >= 4
}

// isIPad reports an iPad. It doesn't check that the engine is WebKit and that the WebKit
// isn't desktop.
func isIPad() bool {
	// Before iOS 13. Safari tampers the value in "request desktop site" mode since iOS 13.
	if equals(navigator().Get("platform"), "iPad") {
		return true
	}

	s := screen()
	screenRatio := toNumber(s.Get("width")) / toNumber(s.Get("height"))

	return countTruthy(
		// Since iOS 13. Doesn't work in Chrome on iPadOS <15, but works in desktop mode.
		has(window(), "MediaSource"),
		// Since iOS 12. Doesn't work in Chrome on iPadOS.
		prototypeOf(js.Global.Get("Element")).Get("webkitRequestFullscreen").Bool(),
		// iPhone 4S that runs iOS 9 matches this, but it is not supported. Doesn't work in
		// incognito mode of Safari ≥17 with split screen because of tracking prevention.
		screenRatio > 0.65 && screenRatio < 1.53,
	) >= 2
}

// or is values[0] || values[1] || ... || values[len-1].
func or(values ...*js.Object) *js.Object {
	for _, value := range values[:len(values)-1] {
		if value.Bool() {
			return value
		}
	}
	return values[len(values)-1]
}

func getFullscreenElement() *js.Object {
	d := document()
	return or(d.Get("fullscreenElement"), d.Get("msFullscreenElement"),
		d.Get("mozFullScreenElement"), d.Get("webkitFullscreenElement"), nil)
}

func exitFullscreen() *js.Object {
	d := document()
	exit := or(d.Get("exitFullscreen"), d.Get("msExitFullscreen"),
		d.Get("mozCancelFullScreen"), d.Get("webkitExitFullscreen"))
	// call is required because the function throws without a proper this
	return exit.Call("call", d)
}

// isAndroid reports a device running Android.
func isAndroid() bool {
	isItChromium, isItGecko := isChromium(), isGecko()
	w, n := window(), navigator()

	// Chrome removes all words "Android" from navigator when the desktop version is requested;
	// Firefox keeps "Android" in navigator.appVersion.
	switch {
	case isItChromium:
		connection := n.Get("connection")
		return countTruthy(
			!has(w, "SharedWorker"),
			// typechange is deprecated but still present on Android (Chrome Mobile 117).
			// It is also present on ChromeOS, hence the other detectors.
			connection.Bool() && has(connection, "ontypechange"),
			!has(js.Global.Get("Audio").New(), "sinkId"),
		) >= 2
	case isItGecko:
		return countTruthy(
			has(w, "onorientationchange"),
			has(w, "orientation"),
			regExpTest("android", "i", n.Get("appVersion")),
		) >= 2
	default:
		// Only two browser engines are present on Android.
		return false
	}
}

// isSamsungInternet reports Samsung Internet. It doesn't check that the browser is based
// on Chromium.
func isSamsungInternet() bool {
	// Checked in Samsung Internet 21, 25 and 27
	n, w := navigator(), window()
	audioPrototype := prototypeOf(js.Global.Get("Audio"))
	visualViewport := w.Get("visualViewport")
	return countTruthy(
		has(audioPrototype, "srLatency"),
		has(audioPrototype, "srChannelCount"),
		has(n, "devicePosture"), // Not available in HTTP
		visualViewport.Bool() && has(visualViewport, "segments"),
		has(prototypeOf(js.Global.Get("Image")), "getTextInformation"), // Not in Samsung Internet 21
	) >= 3
}

// startsWith is RegExp ^prefix applied to a value's string form.
func startsWith(value *js.Object, prefix string) bool {
	return strings.HasPrefix(jsString(value), prefix)
}
