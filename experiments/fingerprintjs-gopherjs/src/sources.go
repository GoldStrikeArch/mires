package main

import "github.com/gopherjs/gopherjs/js"

// builtinSource is an entry of the built-in source registry: the component key, the name
// and arity of the oracle's source function, and its Go implementation.
type builtinSource struct {
	key    string
	name   string
	length int
	get    func(options *js.Object) any
}

// builtinSources run in this exact order. The asynchronous sources are at the start to run
// in parallel with the others; WebGL can affect canvas, so the WebGL sources run last.
var builtinSources = []builtinSource{
	{"userAgentData", "getUserAgentData", 0, func(*js.Object) any { return getUserAgentData() }},
	{"fonts", "getFonts", 0, func(*js.Object) any { return getFonts() }},
	{"domBlockers", "getDomBlockers", 0, func(options *js.Object) any { return getDomBlockers(options) }},
	{"fontPreferences", "getFontPreferences", 0, func(*js.Object) any { return getFontPreferences() }},
	{"audio", "getAudioFingerprint", 0, func(*js.Object) any { return getAudioFingerprint() }},
	{"screenFrame", "getScreenFrame", 0, func(*js.Object) any { return getScreenFrame() }},

	{"canvas", "getCanvasFingerprint", 0, func(*js.Object) any { return getCanvasFingerprint() }},
	{"osCpu", "getOsCpu", 0, func(*js.Object) any { return getOsCpu() }},
	{"languages", "getLanguages", 0, func(*js.Object) any { return getLanguages() }},
	{"colorDepth", "getColorDepth", 0, func(*js.Object) any { return getColorDepth() }},
	{"deviceMemory", "getDeviceMemory", 0, func(*js.Object) any { return getDeviceMemory() }},
	{"screenResolution", "getScreenResolution", 0, func(*js.Object) any { return getScreenResolution() }},
	{"hardwareConcurrency", "getHardwareConcurrency", 0, func(*js.Object) any { return getHardwareConcurrency() }},
	{"timezone", "getTimezone", 0, func(*js.Object) any { return getTimezone() }},
	{"sessionStorage", "getSessionStorage", 0, func(*js.Object) any { return getSessionStorage() }},
	{"localStorage", "getLocalStorage", 0, func(*js.Object) any { return getLocalStorage() }},
	{"indexedDB", "getIndexedDB", 0, func(*js.Object) any { return getIndexedDB() }},
	{"openDatabase", "getOpenDatabase", 0, func(*js.Object) any { return getOpenDatabase() }},
	{"cpuClass", "getCpuClass", 0, func(*js.Object) any { return getCpuClass() }},
	{"platform", "getPlatform", 0, func(*js.Object) any { return getPlatform() }},
	{"plugins", "getPlugins", 0, func(*js.Object) any { return getPlugins() }},
	{"touchSupport", "getTouchSupport", 0, func(*js.Object) any { return getTouchSupport() }},
	{"vendor", "getVendor", 0, func(*js.Object) any { return getVendor() }},
	{"vendorFlavors", "getVendorFlavors", 0, func(*js.Object) any { return getVendorFlavors() }},
	{"cookiesEnabled", "areCookiesEnabled", 0, func(*js.Object) any { return areCookiesEnabled() }},
	{"colorGamut", "getColorGamut", 0, func(*js.Object) any { return getColorGamut() }},
	{"invertedColors", "areColorsInverted", 0, func(*js.Object) any { return areColorsInverted() }},
	{"forcedColors", "areColorsForced", 0, func(*js.Object) any { return areColorsForced() }},
	{"monochrome", "getMonochromeDepth", 0, func(*js.Object) any { return getMonochromeDepth() }},
	{"contrast", "getContrastPreference", 0, func(*js.Object) any { return getContrastPreference() }},
	{"reducedMotion", "isMotionReduced", 0, func(*js.Object) any { return isMotionReduced() }},
	{"reducedTransparency", "isTransparencyReduced", 0, func(*js.Object) any { return isTransparencyReduced() }},
	{"hdr", "isHDR", 0, func(*js.Object) any { return isHDR() }},
	{"math", "getMathFingerprint", 0, func(*js.Object) any { return getMathFingerprint() }},
	{"pdfViewerEnabled", "isPdfViewerEnabled", 0, func(*js.Object) any { return isPdfViewerEnabled() }},
	{"architecture", "getArchitecture", 0, func(*js.Object) any { return getArchitecture() }},
	{"applePay", "getApplePayState", 0, func(*js.Object) any { return getApplePayState() }},
	{"privateClickMeasurement", "getPrivateClickMeasurement", 0, func(*js.Object) any { return getPrivateClickMeasurement() }},
	{"audioBaseLatency", "getAudioContextBaseLatency", 0, func(*js.Object) any { return getAudioContextBaseLatency() }},
	{"dateTimeLocale", "getDateTimeLocale", 0, func(*js.Object) any { return getDateTimeLocale() }},

	{"webGlBasics", "getWebGlBasics", 1, func(options *js.Object) any { return getWebGlBasics(options) }},
	{"webGlExtensions", "getWebGlExtensions", 1, func(options *js.Object) any { return getWebGlExtensions(options) }},
}

// makeSources builds the exported `sources` object of JavaScript source functions.
func makeSources() *js.Object {
	sources := newObject()
	for _, source := range builtinSources {
		get := source.get
		sources.Set(source.key, jsFunc(source.name, source.length, func(_ *js.Object, args []*js.Object) any {
			return get(arg(args, 0))
		}))
	}
	return sources
}

// sources is the exported registry; load() collects the components from it.
var sources *js.Object

// loadBuiltinSources starts loading the built-in sources and returns a function that
// collects their components.
func loadBuiltinSources(options *js.Object) func() *js.Object {
	return loadSources(sources, options, array(), defaultLoopReleaseInterval)
}
