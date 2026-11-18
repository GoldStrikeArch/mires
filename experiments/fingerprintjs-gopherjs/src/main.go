package main

import "github.com/gopherjs/gopherjs/js"

// main publishes the public API on js.Module.exports, the GopherJS export object. The
// build's ESM and IIFE facades re-export it under the oracle's 25 names. Every function
// keeps the oracle's name and length.
func main() {
	sources = makeSources()

	componentsToDebugStringFn := jsFunc("componentsToDebugString", 1, func(_ *js.Object, args []*js.Object) any {
		return componentsToDebugString(arg(args, 0))
	})
	hashComponentsFn := jsFunc("hashComponents", 1, func(_ *js.Object, args []*js.Object) any {
		return hashComponents(arg(args, 0))
	})
	loadFn := jsFunc("load", 0, func(_ *js.Object, args []*js.Object) any {
		return load(arg(args, 0))
	})

	exports := js.Module.Get("exports")
	exports.Set("componentsToDebugString", componentsToDebugStringFn)
	// The default export is syntax sugar (import FP from ...) with all the public values.
	exports.Set("default", object(
		"load", loadFn,
		"hashComponents", hashComponentsFn,
		"componentsToDebugString", componentsToDebugStringFn,
	))
	exports.Set("getFullscreenElement", jsFunc("getFullscreenElement", 0, func(*js.Object, []*js.Object) any {
		return getFullscreenElement()
	}))
	exports.Set("getUnstableAudioFingerprint", jsFunc("getUnstableAudioFingerprint", 0, func(*js.Object, []*js.Object) any {
		return getUnstableAudioFingerprint()
	}))
	exports.Set("getUnstableCanvasFingerprint", jsFunc("getUnstableCanvasFingerprint", 1, func(_ *js.Object, args []*js.Object) any {
		return getUnstableCanvasFingerprint(arg(args, 0).Bool())
	}))
	exports.Set("getUnstableHardwareConcurrency", jsFunc("getUnstableHardwareConcurrency", 0, func(*js.Object, []*js.Object) any {
		return getUnstableHardwareConcurrency()
	}))
	exports.Set("getUnstableScreenFrame", jsFunc("getUnstableScreenFrame", 0, func(*js.Object, []*js.Object) any {
		return getUnstableScreenFrame()
	}))
	exports.Set("getUnstableScreenResolution", jsFunc("getUnstableScreenResolution", 0, func(*js.Object, []*js.Object) any {
		return getUnstableScreenResolution()
	}))
	exports.Set("getWebGLContext", jsFunc("getWebGLContext", 1, func(_ *js.Object, args []*js.Object) any {
		return getWebGLContext(arg(args, 0))
	}))
	exports.Set("hashComponents", hashComponentsFn)
	for _, detector := range []struct {
		name   string
		detect func() bool
	}{
		{"isAndroid", isAndroid},
		{"isChromium", isChromium},
		{"isDesktopWebKit", isDesktopWebKit},
		{"isEdgeHTML", isEdgeHTML},
		{"isGecko", isGecko},
		{"isSamsungInternet", isSamsungInternet},
		{"isTrident", isTrident},
		{"isWebKit", isWebKit},
	} {
		detect := detector.detect
		exports.Set(detector.name, jsFunc(detector.name, 0, func(*js.Object, []*js.Object) any { return detect() }))
	}
	exports.Set("load", loadFn)
	exports.Set("loadSources", jsFunc("loadSources", 4, func(_ *js.Object, args []*js.Object) any {
		loopReleaseInterval := float64(defaultLoopReleaseInterval)
		if interval := arg(args, 3); interval != js.Undefined {
			loopReleaseInterval = toNumber(interval)
		}
		getComponents := loadSources(arg(args, 0), arg(args, 1), arg(args, 2), loopReleaseInterval)
		return jsFunc("getComponents", 0, func(*js.Object, []*js.Object) any { return getComponents() })
	}))
	exports.Set("murmurX64Hash128", jsFunc("x64hash128", 2, func(_ *js.Object, args []*js.Object) any {
		return murmurX64Hash128(arg(args, 0), arg(args, 1))
	}))
	exports.Set("prepareForSources", jsFunc("prepareForSources", 0, func(_ *js.Object, args []*js.Object) any {
		return prepareForSources(arg(args, 0))
	}))
	exports.Set("sources", sources)
	exports.Set("transformSource", jsFunc("transformSource", 2, func(_ *js.Object, args []*js.Object) any {
		return transformSource(arg(args, 0), arg(args, 1))
	}))
	exports.Set("withIframe", jsFunc("withIframe", 2, func(_ *js.Object, args []*js.Object) any {
		action := arg(args, 0)
		domPollInterval := any(defaultDomPollInterval)
		if interval := arg(args, 2); interval != js.Undefined {
			domPollInterval = interval
		}
		return withIframe(func(iframe, iWindow *js.Object) *js.Object {
			return action.Invoke(iframe, iWindow)
		}, arg(args, 1), domPollInterval)
	}))
}
