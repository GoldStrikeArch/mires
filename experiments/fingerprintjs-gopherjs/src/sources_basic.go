package main

import (
	"math"

	"github.com/gopherjs/gopherjs/js"
)

// The small synchronous entropy sources. Values read from the browser are returned as
// they are, so the components keep the host's types.

func getOsCpu() *js.Object { return navigator().Get("oscpu") }

func getLanguages() *js.Object {
	n := navigator()
	result := array()

	language := or(n.Get("language"), n.Get("userLanguage"), n.Get("browserLanguage"), n.Get("systemLanguage"))
	if language != js.Undefined {
		result.Call("push", array(language))
	}

	if js.Global.Get("Array").Call("isArray", n.Get("languages")).Bool() {
		// Starting from Chromium 86 there is only a single value in navigator.languages in
		// Incognito mode, the value of navigator.language, so the value is ignored there.
		if !(isChromium() && isChromium86OrNewer()) {
			result.Call("push", n.Get("languages"))
		}
	} else if typeOf(n.Get("languages")) == "string" {
		if languages := n.Get("languages"); languages.Bool() {
			result.Call("push", languages.Call("split", ","))
		}
	}

	return result
}

func getColorDepth() *js.Object { return window().Get("screen").Get("colorDepth") }

func getDeviceMemory() *js.Object {
	// navigator.deviceMemory is a string containing a number in some unidentified cases
	return replaceNaN(toFloat(navigator().Get("deviceMemory")), js.Undefined)
}

// getScreenResolution is the stabilised source. The window resolution is always the document
// size in private mode of Safari 17, so the source is skipped in Safari 17+.
func getScreenResolution() *js.Object {
	if isWebKit() && isWebKit616OrNewer() && isSafariWebKit() {
		return js.Undefined
	}
	return getUnstableScreenResolution()
}

func getUnstableScreenResolution() *js.Object {
	s := screen()
	// Some browsers return the screen resolution as strings, e.g. "1200", or as not numbers
	parseDimension := func(value *js.Object) *js.Object { return replaceNaN(toInt(value), nil) }
	dimensions := array(parseDimension(s.Get("width")), parseDimension(s.Get("height")))
	dimensions.Call("sort").Call("reverse")
	return dimensions
}

// getHardwareConcurrency is the stabilised source. Firefox 143+ reports tiered values in
// private browsing and strict ETP mode (≤4 cores → 4, >4 cores → 8); the same idempotent
// tiering is applied in every mode.
func getHardwareConcurrency() *js.Object {
	value := getUnstableHardwareConcurrency()
	if value != js.Undefined && isGecko() && isGecko143OrNewer() {
		if value.Float() >= 8 {
			return toJS(8)
		}
		return toJS(4)
	}
	return value
}

func getUnstableHardwareConcurrency() *js.Object {
	// sometimes hardware concurrency is a string
	return replaceNaN(toInt(navigator().Get("hardwareConcurrency")), js.Undefined)
}

func getTimezone() *js.Object {
	if intl := window().Get("Intl"); !isNullish(intl) {
		if dateTimeFormat := intl.Get("DateTimeFormat"); dateTimeFormat.Bool() {
			if timezone := dateTimeFormat.New().Call("resolvedOptions").Get("timeZone"); timezone.Bool() {
				return timezone
			}
		}
	}

	// For browsers that don't support timezone names. The minus is intentional because the
	// JS offset is opposite to the real offset.
	offset := -getTimezoneOffset()
	sign := ""
	if offset >= 0 {
		sign = "+"
	}
	return toJS("UTC" + sign + jsString(offset))
}

// getTimezoneOffset is the non-DST offset: the larger of the January and July offsets, as
// the DST season differs between the hemispheres.
func getTimezoneOffset() float64 {
	date := js.Global.Get("Date")
	currentYear := date.New().Call("getFullYear")
	// getTimezoneOffset returns a number as a string in some unidentified cases
	return math.Max(
		toFloat(date.New(currentYear, 0, 1).Call("getTimezoneOffset")),
		toFloat(date.New(currentYear, 6, 1).Call("getTimezoneOffset")),
	)
}

// storageExists reads a storage property of window; a SecurityError when referencing it
// means that it exists.
func storageExists(name string) bool {
	exists := true
	tryJS(func() { exists = window().Get(name).Bool() })
	return exists
}

func getSessionStorage() bool { return storageExists("sessionStorage") }

// getLocalStorage: https://bugzilla.mozilla.org/show_bug.cgi?id=781447
func getLocalStorage() bool { return storageExists("localStorage") }

func getIndexedDB() *js.Object {
	// IE and Edge don't allow accessing indexedDB in private mode, so they would have
	// different visitor identifiers in normal and private modes.
	if isTrident() || isEdgeHTML() {
		return js.Undefined
	}
	return toJS(storageExists("indexedDB"))
}

func getOpenDatabase() bool { return window().Get("openDatabase").Bool() }

func getCpuClass() *js.Object { return navigator().Get("cpuClass") }

func getPlatform() *js.Object {
	// Android Chrome 86 and 87 and Android Firefox 80 and 84 don't mock the platform value
	// when the desktop mode is requested
	platform := navigator().Get("platform")

	// iOS mocks the platform value when the desktop version is requested, and iPad uses the
	// desktop mode by default since iOS 13. The value is "MacIntel" on M1 Macs and "iPhone"
	// on iPod Touch.
	if equals(platform, "MacIntel") && isWebKit() && !isDesktopWebKit() {
		if isIPad() {
			return toJS("iPad")
		}
		return toJS("iPhone")
	}

	return platform
}

func getPlugins() *js.Object {
	rawPlugins := navigator().Get("plugins")
	if !rawPlugins.Bool() {
		return js.Undefined
	}

	plugins := array()
	// Safari 10 doesn't support iterating navigator.plugins with for...of
	for i := 0; i < rawPlugins.Length(); i++ {
		plugin := rawPlugins.Index(i)
		if !plugin.Bool() {
			continue
		}

		mimeTypes := array()
		for j := 0; j < plugin.Length(); j++ {
			mimeType := plugin.Index(j)
			mimeTypes.Call("push", object("type", mimeType.Get("type"), "suffixes", mimeType.Get("suffixes")))
		}

		plugins.Call("push", object(
			"name", plugin.Get("name"),
			"description", plugin.Get("description"),
			"mimeTypes", mimeTypes,
		))
	}
	return plugins
}

// getTouchSupport is a crude touch screen detection: a touch screen can't be detected
// reliably without subscribing to a touch event.
func getTouchSupport() *js.Object {
	n := navigator()

	var maxTouchPoints any = 0
	if n.Get("maxTouchPoints") != js.Undefined {
		maxTouchPoints = toInt(n.Get("maxTouchPoints"))
	} else if n.Get("msMaxTouchPoints") != js.Undefined {
		maxTouchPoints = n.Get("msMaxTouchPoints")
	}
	_, failed := tryJS(func() { document().Call("createEvent", "TouchEvent") })
	touchStart := has(window(), "ontouchstart")

	return object("maxTouchPoints", maxTouchPoints, "touchEvent", !failed, "touchStart", touchStart)
}

func getVendor() *js.Object { return or(navigator().Get("vendor"), toJS("")) }

// vendorFlavorKeys are browser-specific (not engine-specific) globals that tell browsers
// with the same engine apart.
var vendorFlavorKeys = []string{
	"chrome",                             // Blink and some browsers on iOS
	"safari",                             // Safari on macOS
	"__crWeb",                            // Chrome on iOS (checked in 85 on 13 and 87 on 14)
	"__gCrWeb",                           // Chrome on iOS
	"yandex",                             // Yandex Browser on iOS, macOS and Android (checked in 21.2)
	"__yb",                               // Yandex Browser on iOS (checked in 21.2 on 14)
	"__ybro",                             // Yandex Browser on iOS
	"__firefox__",                        // Firefox on iOS (checked in 32 on 14)
	"__edgeTrackingPreventionStatistics", // Edge on iOS (checked in 46 on 14)
	"webkit",                             // Edge on iOS
	"oprt",                               // Opera Touch on iOS (checked in 2.6 on 14)
	"samsungAr",                          // Samsung Internet on Android (checked in 11.1)
	"ucweb",                              // UC Browser on Android (checked in 12.10 and 13.0)
	"UCShellJava",                        // UC Browser on Android
	"puffinDevice",                       // Puffin on Android (checked in 9.0)
	// UC on iOS and Opera on Android have no specific global variables. Edge for Android
	// isn't checked.
}

func getVendorFlavors() *js.Object {
	flavors := array()
	w := window()
	for _, key := range vendorFlavorKeys {
		if value := w.Get(key); value.Bool() && typeOf(value) == "object" {
			flavors.Call("push", key)
		}
	}
	return flavors.Call("sort")
}

// areCookiesEnabled sets and reads a test cookie. navigator.cookieEnabled cannot detect
// custom or nuanced cookie blocking configurations. Taken from Modernizr.
func areCookiesEnabled() bool {
	d := document()
	var enabled bool
	// document.cookie can be exposed but throw a SecurityError, e.g. in documents created from
	// data URIs or in sandboxed iframes
	if _, threw := tryJS(func() {
		d.Set("cookie", "cookietest=1; SameSite=Strict;")
		enabled = d.Get("cookie").Call("indexOf", "cookietest=").Int() != -1
		d.Set("cookie", "cookietest=1; SameSite=Strict; expires=Thu, 01-Jan-1970 00:00:01 GMT")
	}); threw {
		return false
	}
	return enabled
}

func matchMedia(query string) bool {
	return js.Global.Call("matchMedia", query).Get("matches").Bool()
}

// mediaBool is a media feature with an on value, an off value or neither.
func mediaBool(feature, on, off string) *js.Object {
	switch {
	case matchMedia("(" + feature + ": " + on + ")"):
		return toJS(true)
	case matchMedia("(" + feature + ": " + off + ")"):
		return toJS(false)
	default:
		return js.Undefined
	}
}

func getColorGamut() *js.Object {
	// rec2020 includes p3 and p3 includes srgb
	for _, gamut := range []string{"rec2020", "p3", "srgb"} {
		if matchMedia("(color-gamut: " + gamut + ")") {
			return toJS(gamut)
		}
	}
	return js.Undefined
}

func areColorsInverted() *js.Object { return mediaBool("inverted-colors", "inverted", "none") }

func areColorsForced() *js.Object { return mediaBool("forced-colors", "active", "none") }

// getMonochromeDepth is the bits per pixel of a monochrome display, 0 for a display that
// isn't monochrome, or undefined when the media feature isn't supported.
func getMonochromeDepth() *js.Object {
	if !matchMedia("(min-monochrome: 0)") {
		return js.Undefined
	}
	// The expected values are very small (≤10), so a linear search is enough
	for i := 0; i <= 100; i++ {
		if matchMedia("(max-monochrome: " + jsString(i) + ")") {
			return toJS(i)
		}
	}
	throw(newError("Too high value"))
	return nil
}

const (
	contrastLess         = -1
	contrastNone         = 0
	contrastMore         = 1
	contrastForcedColors = 10
)

func getContrastPreference() *js.Object {
	doesMatch := func(value string) bool { return matchMedia("(prefers-contrast: " + value + ")") }
	switch {
	case doesMatch("no-preference"):
		return toJS(contrastNone)
	// The sources contradict on the keywords; "high" and "low" may never be implemented
	case doesMatch("high") || doesMatch("more"):
		return toJS(contrastMore)
	case doesMatch("low") || doesMatch("less"):
		return toJS(contrastLess)
	case doesMatch("forced"):
		return toJS(contrastForcedColors)
	default:
		return js.Undefined
	}
}

func isMotionReduced() *js.Object {
	return mediaBool("prefers-reduced-motion", "reduce", "no-preference")
}

func isTransparencyReduced() *js.Object {
	return mediaBool("prefers-reduced-transparency", "reduce", "no-preference")
}

func isHDR() *js.Object { return mediaBool("dynamic-range", "high", "standard") }

func isPdfViewerEnabled() *js.Object { return navigator().Get("pdfViewerEnabled") }

// getArchitecture tells x86/x86-64 from other architectures: there, a NaN produced from
// non-NaN operands has its sign bit set. Infinity - Infinity must produce NaN.
func getArchitecture() *js.Object {
	f := js.Global.Get("Float32Array").New(1)
	u8 := js.Global.Get("Uint8Array").New(f.Get("buffer"))
	f.SetIndex(0, math.Inf(1))
	f.SetIndex(0, f.Index(0).Float()-f.Index(0).Float())
	return u8.Index(3)
}

const (
	applePayDisabled                      = 0
	applePayEnabled                       = 1
	applePayNoAPI                         = -1
	applePayNotAvailableInInsecureContext = -2
	applePayNotAvailableInFrame           = -3
)

func getApplePayState() int {
	applePaySession := window().Get("ApplePaySession")
	canMakePayments := js.Undefined
	if !isNullish(applePaySession) {
		canMakePayments = applePaySession.Get("canMakePayments")
	}
	if !isFunction(canMakePayments) {
		return applePayNoAPI
	}

	// Since Safari 15 canMakePayments() prints a console error in a cross-origin iframe
	// without allow="payment *". A permission can't be checked in Safari yet, so any
	// cross-origin parent counts.
	if isAnyParentCrossOrigin() {
		return applePayNotAvailableInFrame
	}

	state := applePayDisabled
	if exception, threw := tryJS(func() {
		if applePaySession.Call("canMakePayments").Bool() {
			state = applePayEnabled
		}
	}); threw {
		return getStateFromError(exception)
	}
	return state
}

func getStateFromError(err *js.Object) int {
	if isError(err) && equals(err.Get("name"), "InvalidAccessError") &&
		regExpTest(`\bfrom\b.*\binsecure\b`, "i", err.Get("message")) {
		return applePayNotAvailableInInsecureContext
	}
	throw(err)
	return 0
}

// getPrivateClickMeasurement reads Safari's Privacy Preserving Ad Measurement setting,
// which is on when the value is defined. It doesn't work in mobile Safari.
func getPrivateClickMeasurement() *js.Object {
	link := document().Call("createElement", "a")
	sourceID := link.Get("attributionSourceId")
	if isNullish(sourceID) {
		sourceID = link.Get("attributionsourceid")
	}
	if sourceID == js.Undefined {
		return js.Undefined
	}
	return js.Global.Call("String", sourceID)
}

const (
	audioLatencyNotSupported = -1
	audioLatencyDisabled     = -2
	audioLatencyNotFinite    = -3
)

func getAudioContextBaseLatency() *js.Object {
	// The source prints a warning in Chrome and Firefox, so it is enabled only on Safari,
	// where it doesn't, and on Android, where the warning is less visible
	if !(isAndroid() || isWebKit()) {
		return toJS(audioLatencyDisabled)
	}
	if !window().Get("AudioContext").Bool() {
		return toJS(audioLatencyNotSupported)
	}

	latency := js.Global.Get("AudioContext").New().Get("baseLatency")
	if isNullish(latency) {
		return toJS(audioLatencyNotSupported)
	}
	if !js.Global.Call("isFinite", latency).Bool() {
		return toJS(audioLatencyNotFinite)
	}
	return latency
}

const (
	intlAPINotSupported        = -1
	dateTimeFormatNotSupported = -2
	localeNotAvailable         = -3
)

func getDateTimeLocale() *js.Object {
	if !window().Get("Intl").Bool() {
		return toJS(intlAPINotSupported)
	}

	dateTimeFormat := window().Get("Intl").Get("DateTimeFormat")
	if !dateTimeFormat.Bool() {
		return toJS(dateTimeFormatNotSupported)
	}

	locale := dateTimeFormat.Invoke().Call("resolvedOptions").Get("locale")
	if !locale.Bool() && !equals(locale, "") {
		return toJS(localeNotAvailable)
	}
	return locale
}
