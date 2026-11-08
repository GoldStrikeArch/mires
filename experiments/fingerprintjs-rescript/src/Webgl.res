open Bindings
open Browser
open Async

let statusNoGlContext = -1
let statusGetParameterNotAFunction = -2
let rendererInfoExtensionName = "WEBGL_debug_renderer_info"
let polygonModeExtensionName = "WEBGL_polygon_mode"
let shaderTypes = ["FRAGMENT_SHADER", "VERTEX_SHADER"]
let precisionTypes = ["LOW_FLOAT", "MEDIUM_FLOAT", "HIGH_FLOAT", "LOW_INT", "MEDIUM_INT", "HIGH_INT"]

let validContextParameters = newSet([
  10752, 2849, 2884, 2885, 2886, 2928, 2929, 2930, 2931, 2932, 2960, 2961, 2962, 2963, 2964, 2965, 2966,
  2967, 2968, 2978, 3024, 3042, 3088, 3089, 3106, 3107, 32773, 32777, 32777, 32823, 32824, 32936, 32937,
  32938, 32939, 32968, 32969, 32970, 32971, 3317, 33170, 3333, 3379, 3386, 33901, 33902, 34016, 34024,
  34076, 3408, 3410, 3411, 3412, 3413, 3414, 3415, 34467, 34816, 34817, 34818, 34819, 34877, 34921,
  34930, 35660, 35661, 35724, 35738, 35739, 36003, 36004, 36005, 36347, 36348, 36349, 37440, 37441,
  37443, 7936, 7937, 7938,
])

let validExtensionParams = newSet([34047, 35723, 36063, 34852, 34853, 34854, 34229, 36392, 36795, 38449])

let shouldAvoidDebugRendererInfo = () => isGecko()
let shouldAvoidPolygonModeExtensions = () => isChromium() || isWebKit()

let isValidParameterGetter = gl => isFunction(get(gl, "getParameter"))

let isConstantLike = key => isString(key) && isNullish(execRe(newRegExp("[^A-Z0-9_x]", ""), asString(key)))

let getConstantsFromPrototype = obj => arrayFilter(objectKeys(protoOf(obj)), (key, _i) => isConstantLike(key))

let getShaderPrecision = (gl, shaderType, precisionType) => {
  let shaderPrecision = call2(gl, "getShaderPrecisionFormat", get(gl, shaderType), get(gl, precisionType))
  if isTruthy(shaderPrecision) {
    [get(shaderPrecision, "rangeMin"), get(shaderPrecision, "rangeMax"), get(shaderPrecision, "precision")]
  } else {
    []
  }
}

let getWebGLContext = (cache: any) => {
  let cached = get(cache, "webgl")
  if isTruthy(cached) {
    get(cached, "context")
  } else {
    let canvas = createElement(document(), "canvas")
    let context = ref(jsUndefined)
    addEventListener(canvas, "webglCreateContextError", () => {
      context := jsUndefined
    })
    let types = ["webgl", "experimental-webgl"]
    let rec loop = i =>
      if i >= arrayLength(types) {
        ()
      } else {
        let outcome = runAction(() => getContext(canvas, getInt(types, i)))
        if asBool(getInt(outcome, 0)) {
          context := getInt(outcome, 1)
        }
        if isTruthy(context.contents) {
          ()
        } else {
          loop(i + 1)
        }
      }
    loop(0)
    let entry = emptyObject()
    set(entry, "context", context.contents)
    set(cache, "webgl", entry)
    context.contents
  }
}

let getWebGlBasics = (options: any) => {
  let cache = get(options, "cache")
  let gl = getWebGLContext(if isTruthy(cache) {
    cache
  } else {
    emptyObject()
  })
  if isFalsy(gl) {
    identity(statusNoGlContext)
  } else if !isValidParameterGetter(gl) {
    identity(statusGetParameterNotAFunction)
  } else {
    let debugExtension = if shouldAvoidDebugRendererInfo() {
      jsNull
    } else {
      call1(gl, "getExtension", rendererInfoExtensionName)
    }
    let toText = value =>
      if isNullish(value) {
        ""
      } else {
        orElse(toString(value), "")
      }
    let result = emptyObject()
    set(result, "version", toText(call1(gl, "getParameter", get(gl, "VERSION"))))
    set(result, "vendor", toText(call1(gl, "getParameter", get(gl, "VENDOR"))))
    set(
      result,
      "vendorUnmasked",
      if isTruthy(debugExtension) {
        toText(call1(gl, "getParameter", get(debugExtension, "UNMASKED_VENDOR_WEBGL")))
      } else {
        ""
      },
    )
    set(result, "renderer", toText(call1(gl, "getParameter", get(gl, "RENDERER"))))
    set(
      result,
      "rendererUnmasked",
      if isTruthy(debugExtension) {
        toText(call1(gl, "getParameter", get(debugExtension, "UNMASKED_RENDERER_WEBGL")))
      } else {
        ""
      },
    )
    set(
      result,
      "shadingLanguageVersion",
      toText(call1(gl, "getParameter", get(gl, "SHADING_LANGUAGE_VERSION"))),
    )
    identity(result)
  }
}

let getWebGlExtensions = (options: any) => {
  let cache = get(options, "cache")
  let gl = getWebGLContext(if isTruthy(cache) {
    cache
  } else {
    emptyObject()
  })
  if isFalsy(gl) {
    identity(statusNoGlContext)
  } else if !isValidParameterGetter(gl) {
    identity(statusGetParameterNotAFunction)
  } else {
    let extensions = call0(gl, "getSupportedExtensions")
    let contextAttributes = call0(gl, "getContextAttributes")
    let unsupportedExtensions = []
    let attributes = []
    let parameters = []
    let extensionParameters = []
    let shaderPrecisions = []
    if isTruthy(contextAttributes) {
      let names = objectKeys(contextAttributes)
      let rec loop = i =>
        if i >= arrayLength(names) {
          ()
        } else {
          let attributeName = getInt(names, i)
          arrayPush(attributes, attributeName ++ "=" ++ jsString(get(contextAttributes, attributeName)))->ignore
          loop(i + 1)
        }
      loop(0)
    }
    let constants = getConstantsFromPrototype(gl)
    let rec paramLoop = i =>
      if i >= arrayLength(constants) {
        ()
      } else {
        let constant = getInt(constants, i)
        let code = get(gl, constant)
        arrayPush(
          parameters,
          constant ++
          "=" ++
          jsString(code) ++ if setHas(validContextParameters, asInt(code)) {
            "=" ++ jsString(call1(gl, "getParameter", code))
          } else {
            ""
          },
        )->ignore
        paramLoop(i + 1)
      }
    paramLoop(0)
    if isTruthy(extensions) {
      let rec extLoop = i =>
        if i >= arrayLength(asArray(extensions)) {
          ()
        } else {
          let name = getInt(asArray(extensions), i)
          if (
            name == rendererInfoExtensionName && shouldAvoidDebugRendererInfo() ||
              name == polygonModeExtensionName && shouldAvoidPolygonModeExtensions()
          ) {
            extLoop(i + 1)
          } else {
            let extension = call1(gl, "getExtension", name)
            if isFalsy(extension) {
              arrayPush(unsupportedExtensions, name)->ignore
            } else {
              let extConstants = getConstantsFromPrototype(extension)
              let rec extParamLoop = j =>
                if j >= arrayLength(extConstants) {
                  ()
                } else {
                  let constant = getInt(extConstants, j)
                  let code = get(extension, constant)
                  arrayPush(
                    extensionParameters,
                    constant ++
                    "=" ++
                    jsString(code) ++ if setHas(validExtensionParams, asInt(code)) {
                      "=" ++ jsString(call1(gl, "getParameter", code))
                    } else {
                      ""
                    },
                  )->ignore
                  extParamLoop(j + 1)
                }
              extParamLoop(0)
            }
            extLoop(i + 1)
          }
        }
      extLoop(0)
    }
    let rec shaderLoop = i =>
      if i >= arrayLength(shaderTypes) {
        ()
      } else {
        let shaderType = getInt(shaderTypes, i)
        let rec precisionLoop = j =>
          if j >= arrayLength(precisionTypes) {
            ()
          } else {
            let precisionType = getInt(precisionTypes, j)
            let shaderPrecision = getShaderPrecision(gl, shaderType, precisionType)
            arrayPush(
              shaderPrecisions,
              shaderType ++ "." ++ precisionType ++ "=" ++ arrayJoin(arrayMap(shaderPrecision, (item, _) => jsString(item)), ","),
            )->ignore
            precisionLoop(j + 1)
          }
        precisionLoop(0)
        shaderLoop(i + 1)
      }
    shaderLoop(0)
    arraySort(extensionParameters)->ignore
    arraySort(parameters)->ignore
    let result = emptyObject()
    set(result, "contextAttributes", attributes)
    set(result, "parameters", parameters)
    set(result, "shaderPrecisions", shaderPrecisions)
    set(result, "extensions", extensions)
    set(result, "extensionParameters", extensionParameters)
    set(result, "unsupportedExtensions", unsupportedExtensions)
    identity(result)
  }
}
