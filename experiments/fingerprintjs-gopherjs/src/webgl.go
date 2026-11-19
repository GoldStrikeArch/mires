package main

import (
	"strings"

	"github.com/gopherjs/gopherjs/js"
)

const (
	statusNoGLContext              = -1 // WebGL context is not available
	statusGetParameterNotAFunction = -2 // The context's getParameter isn't a function
	rendererInfoExtensionName      = "WEBGL_debug_renderer_info"
	polygonModeExtensionName       = "WEBGL_polygon_mode"
)

// validContextParameters are the parameters whose values are read. SAMPLE_ALPHA_TO_COVERAGE
// (32926) and SAMPLE_COVERAGE (32928) are excluded: they trigger a console warning in IE,
// Chrome ≤ 59 and Safari ≤ 13 and give no entropy.
var validContextParameters = numberSet(
	10752, 2849, 2884, 2885, 2886, 2928, 2929, 2930, 2931, 2932, 2960, 2961, 2962, 2963, 2964, 2965, 2966, 2967, 2968,
	2978, 3024, 3042, 3088, 3089, 3106, 3107, 32773, 32777, 32777, 32823, 32824, 32936, 32937, 32938, 32939, 32968, 32969,
	32970, 32971, 3317, 33170, 3333, 3379, 3386, 33901, 33902, 34016, 34024, 34076, 3408, 3410, 3411, 3412, 3413, 3414,
	3415, 34467, 34816, 34817, 34818, 34819, 34877, 34921, 34930, 35660, 35661, 35724, 35738, 35739, 36003, 36004, 36005,
	36347, 36348, 36349, 37440, 37441, 37443, 7936, 7937, 7938,
)

var validExtensionParams = numberSet(
	34047, // MAX_TEXTURE_MAX_ANISOTROPY_EXT
	35723, // FRAGMENT_SHADER_DERIVATIVE_HINT_OES
	36063, // MAX_COLOR_ATTACHMENTS_WEBGL
	34852, // MAX_DRAW_BUFFERS_WEBGL
	34853, // DRAW_BUFFER0_WEBGL
	34854, // DRAW_BUFFER1_WEBGL
	34229, // VERTEX_ARRAY_BINDING_OES
	36392, // TIMESTAMP_EXT
	36795, // GPU_DISJOINT_EXT
	38449, // MAX_VIEWS_OVR
)

var (
	shaderTypes    = []string{"FRAGMENT_SHADER", "VERTEX_SHADER"}
	precisionTypes = []string{"LOW_FLOAT", "MEDIUM_FLOAT", "HIGH_FLOAT", "LOW_INT", "MEDIUM_INT", "HIGH_INT"}
)

func numberSet(values ...float64) map[float64]bool {
	set := make(map[float64]bool, len(values))
	for _, value := range values {
		set[value] = true
	}
	return set
}

// hasNumber is Set.prototype.has for a set of numbers: only a number can be a member.
func hasNumber(set map[float64]bool, value *js.Object) bool {
	return typeOf(value) == "number" && set[value.Float()]
}

// getWebGlBasics gets the basic and simple WebGL parameters.
func getWebGlBasics(options *js.Object) *js.Object {
	gl := getWebGLContext(options.Get("cache"))
	if !gl.Bool() {
		return toJS(statusNoGLContext)
	}
	if !isValidParameterGetter(gl) {
		return toJS(statusGetParameterNotAFunction)
	}

	var debugExtension *js.Object
	if !shouldAvoidDebugRendererInfo() {
		debugExtension = gl.Call("getExtension", rendererInfoExtensionName)
	}
	parameterString := func(code *js.Object) *js.Object {
		return optionalToString(gl.Call("getParameter", code))
	}
	unmasked := func(name string) *js.Object {
		if !debugExtension.Bool() {
			return toJS("")
		}
		return parameterString(debugExtension.Get(name))
	}

	return object(
		"version", or(parameterString(gl.Get("VERSION")), toJS("")),
		"vendor", or(parameterString(gl.Get("VENDOR")), toJS("")),
		"vendorUnmasked", unmasked("UNMASKED_VENDOR_WEBGL"),
		"renderer", or(parameterString(gl.Get("RENDERER")), toJS("")),
		"rendererUnmasked", unmasked("UNMASKED_RENDERER_WEBGL"),
		"shadingLanguageVersion", or(parameterString(gl.Get("SHADING_LANGUAGE_VERSION")), toJS("")),
	)
}

// optionalToString is value?.toString().
func optionalToString(value *js.Object) *js.Object {
	if isNullish(value) {
		return js.Undefined
	}
	return value.Call("toString")
}

// getWebGlExtensions gets the advanced and massive WebGL parameters and extensions.
func getWebGlExtensions(options *js.Object) *js.Object {
	gl := getWebGLContext(options.Get("cache"))
	if !gl.Bool() {
		return toJS(statusNoGLContext)
	}
	if !isValidParameterGetter(gl) {
		return toJS(statusGetParameterNotAFunction)
	}

	extensions := gl.Call("getSupportedExtensions")
	contextAttributes := gl.Call("getContextAttributes")
	unsupportedExtensions := array()

	// Features
	attributes := array()
	parameters := array()
	extensionParameters := array()
	shaderPrecisions := array()

	// Context attributes
	if contextAttributes.Bool() {
		attributeNames := keys(contextAttributes)
		for i := 0; i < attributeNames.Length(); i++ {
			name := attributeNames.Index(i)
			attributes.Call("push", jsString(name)+"="+jsString(getKey(contextAttributes, name)))
		}
	}

	// Context parameters
	for _, constant := range getConstantsFromPrototype(gl) {
		code := gl.Get(constant)
		entry := constant + "=" + jsString(code)
		if hasNumber(validContextParameters, code) {
			entry += "=" + jsString(gl.Call("getParameter", code))
		}
		parameters.Call("push", entry)
	}

	// Extension parameters
	if extensions.Bool() {
		for i := 0; i < extensions.Length(); i++ {
			name := extensions.Index(i)
			if (equals(name, rendererInfoExtensionName) && shouldAvoidDebugRendererInfo()) ||
				(equals(name, polygonModeExtensionName) && shouldAvoidPolygonModeExtensions()) {
				continue
			}

			extension := gl.Call("getExtension", name)
			if !extension.Bool() {
				unsupportedExtensions.Call("push", name)
				continue
			}

			for _, constant := range getConstantsFromPrototype(extension) {
				code := extension.Get(constant)
				entry := constant + "=" + jsString(code)
				if hasNumber(validExtensionParams, code) {
					entry += "=" + jsString(gl.Call("getParameter", code))
				}
				extensionParameters.Call("push", entry)
			}
		}
	}

	// Shader precision
	for _, shaderType := range shaderTypes {
		for _, precisionType := range precisionTypes {
			shaderPrecision := getShaderPrecision(gl, shaderType, precisionType)
			shaderPrecisions.Call("push", shaderType+"."+precisionType+"="+strings.Join(shaderPrecision, ","))
		}
	}

	// Postprocess
	extensionParameters.Call("sort")
	parameters.Call("sort")

	return object(
		"contextAttributes", attributes,
		"parameters", parameters,
		"shaderPrecisions", shaderPrecisions,
		"extensions", extensions,
		"extensionParameters", extensionParameters,
		"unsupportedExtensions", unsupportedExtensions,
	)
}

// getWebGLContext creates the WebGL context once per cache: it is usually the slowest
// operation of all the sources.
func getWebGLContext(cache *js.Object) *js.Object {
	if webgl := cache.Get("webgl"); webgl.Bool() {
		return webgl.Get("context")
	}

	canvas := document().Call("createElement", "canvas")
	context := js.Undefined

	canvas.Call("addEventListener", "webglCreateContextError", func() { context = js.Undefined })

	for _, contextType := range []string{"webgl", "experimental-webgl"} {
		tryJS(func() { context = canvas.Call("getContext", contextType) }) // Ok, continue
		if context.Bool() {
			break
		}
	}

	cache.Set("webgl", object("context", context))
	return context
}

// getShaderPrecision returns [rangeMin, rangeMax, precision] in their string forms, as
// Array.prototype.join writes them.
// https://developer.mozilla.org/en-US/docs/Web/API/WebGLRenderingContext/getShaderPrecisionFormat
func getShaderPrecision(gl *js.Object, shaderType, precisionType string) []string {
	shaderPrecision := gl.Call("getShaderPrecisionFormat", gl.Get(shaderType), gl.Get(precisionType))
	if !shaderPrecision.Bool() {
		return nil
	}
	var values []string
	for _, field := range []string{"rangeMin", "rangeMax", "precision"} {
		values = append(values, joinElement(shaderPrecision.Get(field)))
	}
	return values
}

// joinElement is the string form Array.prototype.join gives an element.
func joinElement(value *js.Object) string {
	if isNullish(value) {
		return ""
	}
	return jsString(value)
}

// getConstantsFromPrototype lists the constant-like own keys of the object's prototype.
func getConstantsFromPrototype(obj *js.Object) []string {
	prototypeKeys := keys(obj.Get("__proto__"))
	var constants []string
	for i := 0; i < prototypeKeys.Length(); i++ {
		if key := prototypeKeys.Index(i).String(); isConstantLike(key) {
			constants = append(constants, key)
		}
	}
	return constants
}

// isConstantLike is !key.match(/[^A-Z0-9_x]/).
func isConstantLike(key string) bool {
	for i := 0; i < len(key); i++ {
		c := key[i]
		if !(c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '_' || c == 'x') {
			return false
		}
	}
	return true
}

// shouldAvoidDebugRendererInfo: some browsers print a console warning when the
// WEBGL_debug_renderer_info extension is requested.
func shouldAvoidDebugRendererInfo() bool { return isGecko() }

// shouldAvoidPolygonModeExtensions: some browsers print a console warning when the
// WEBGL_polygon_mode extension is requested.
func shouldAvoidPolygonModeExtensions() bool { return isChromium() || isWebKit() }

// isValidParameterGetter: some unknown browsers have no getParameter method.
func isValidParameterGetter(gl *js.Object) bool { return isFunction(gl.Get("getParameter")) }
