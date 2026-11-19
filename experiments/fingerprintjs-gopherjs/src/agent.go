package main

import (
	"strings"

	"github.com/gopherjs/gopherjs/js"
)

const version = "5.2.0"

const monitoringURL = "https://m1.openfpcdn.io/fingerprintjs/v" + version + "/npm-monitoring"

// escapeComponentKey escapes the separators of the canonical string with a backslash.
func escapeComponentKey(key string) string {
	var b strings.Builder
	for i := 0; i < len(key); i++ {
		switch key[i] {
		case ':', '|', '\\':
			b.WriteByte('\\')
		}
		b.WriteByte(key[i])
	}
	return b.String()
}

// componentsToCanonicalString serialises the components as sorted key:value pairs joined
// by "|", as UTF-8. The pieces are joined at ASCII separators, so converting them one by
// one gives the same bytes as encoding the oracle's whole string.
func componentsToCanonicalString(components *js.Object) string {
	var result strings.Builder
	sortedKeys := keys(components).Call("sort")
	for i := 0; i < sortedKeys.Length(); i++ {
		componentKey := sortedKeys.Index(i)
		component := getKey(components, componentKey)
		value := "error"
		if !has(component, "error") {
			value = jsonString(component.Get("value"))
		}
		if result.Len() > 0 {
			result.WriteByte('|')
		}
		result.WriteString(escapeComponentKey(utf8String(componentKey)))
		result.WriteByte(':')
		result.WriteString(value)
	}
	return result.String()
}

// jsonString is the template-literal form of JSON.stringify(value): "undefined" when the
// value has no JSON representation.
func jsonString(value *js.Object) string {
	text := js.Global.Get("JSON").Call("stringify", value)
	if text == js.Undefined {
		return "undefined"
	}
	return utf8String(text)
}

func componentsToDebugString(components *js.Object) *js.Object {
	replacer := func(_, value *js.Object) *js.Object {
		if isError(value) {
			return errorToObject(value)
		}
		return value
	}
	return js.Global.Get("JSON").Call("stringify", components, replacer, 2)
}

func hashComponents(components *js.Object) string {
	return x64hash128([]byte(componentsToCanonicalString(components)), 0)
}

// makeLazyGetResult makes a result whose visitorId is hashed on first read. A plain object
// with an accessor is used because the getters and setters of a class aren't enumerable.
func makeLazyGetResult(components *js.Object) *js.Object {
	visitorIDCache := js.Undefined

	// This function runs very fast, so there is no need to make it lazy
	confidence := getConfidence(components)

	getVisitorID := jsFunc("get visitorId", 0, func(this *js.Object, _ []*js.Object) any {
		if visitorIDCache == js.Undefined {
			visitorIDCache = toJS(hashComponents(this.Get("components")))
		}
		return visitorIDCache
	})
	setVisitorID := jsFunc("set visitorId", 1, func(_ *js.Object, args []*js.Object) any {
		visitorIDCache = arg(args, 0)
		return js.Undefined
	})

	result := newObject()
	js.Global.Get("Object").Call("defineProperty", result, "visitorId", object(
		"get", getVisitorID, "set", setVisitorID, "enumerable", true, "configurable", true))
	result.Set("confidence", confidence)
	result.Set("components", components)
	result.Set("version", version)
	return result
}

// prepareForSources waits the delay that keeps the entropy components consistent: an idle
// callback, or a timeout where requestIdleCallback is missing. The idle deadline is twice
// the fallback timeout, so both cases have the same average time.
func prepareForSources(delayFallback *js.Object) *js.Object {
	if delayFallback == js.Undefined {
		delayFallback = toJS(50)
	}
	return requestIdleCallbackIfAvailable(delayFallback, toNumber(delayFallback)*2)
}

func makeAgent(getComponents func() *js.Object, debug *js.Object) *js.Object {
	creationTime := dateNow()

	get := jsFunc("get", 1, func(_ *js.Object, args []*js.Object) any {
		options := arg(args, 0)
		return async(func(resolve, reject *js.Object) {
			startTime := dateNow()
			await(getComponents(), reject, func(components *js.Object) {
				result := makeLazyGetResult(components)

				if debug.Bool() || (!isNullish(options) && options.Get("debug").Bool()) {
					var message strings.Builder
					message.WriteString("Copy the text below to get the debug data:\n\n```\nversion: ")
					message.WriteString(jsString(result.Get("version")))
					message.WriteString("\nuserAgent: ")
					message.WriteString(jsString(navigator().Get("userAgent")))
					message.WriteString("\ntimeBetweenLoadAndGet: ")
					message.WriteString(jsString(startTime - creationTime))
					message.WriteString("\nvisitorId: ")
					message.WriteString(jsString(result.Get("visitorId")))
					message.WriteString("\ncomponents: ")
					message.WriteString(jsString(componentsToDebugString(components)))
					message.WriteString("\n```")
					console().Call("log", message.String())
				}

				resolve.Invoke(result)
			})
		})
	})

	return object("get", get)
}

// monitor sends an unpersonalised request that collects installation statistics, for one
// load in a thousand. The FingerprintJS CDN sets window.__fpjs_d_m to disable it.
func monitor() {
	if window().Get("__fpjs_d_m").Bool() || mathCall("random") >= 0.001 {
		return
	}
	if exception, threw := tryJS(func() {
		request := js.Global.Get("XMLHttpRequest").New()
		request.Call("open", "get", monitoringURL, true)
		request.Call("send")
	}); threw {
		console().Call("error", exception)
	}
}

// load builds an agent and waits the delay required for a proper operation.
func load(options *js.Object) *js.Object {
	return async(func(resolve, reject *js.Object) {
		if options == js.Undefined {
			options = newObject()
		}
		delayFallback := options.Get("delayFallback")
		debug := options.Get("debug")
		monitoring := options.Get("monitoring")
		if monitoring == js.Undefined || monitoring.Bool() {
			monitor()
		}
		await(prepareForSources(delayFallback), reject, func(*js.Object) {
			getComponents := loadBuiltinSources(object("cache", newObject(), "debug", debug))
			resolve.Invoke(makeAgent(getComponents, debug))
		})
	})
}
