import { isChromium, isGecko, isWebKit } from '../browser.js'

export const STATUS_NO_GL_CONTEXT = -1
export const STATUS_GET_PARAMETER_NOT_A_FUNCTION = -2

const validContextParameters = new Set([
  10752, 2849, 2884, 2885, 2886, 2928, 2929, 2930, 2931, 2932, 2960, 2961, 2962, 2963, 2964, 2965, 2966, 2967, 2968,
  2978, 3024, 3042, 3088, 3089, 3106, 3107, 32773, 32777, 32777, 32823, 32824, 32936, 32937, 32938, 32939, 32968, 32969,
  32970, 32971, 3317, 33170, 3333, 3379, 3386, 33901, 33902, 34016, 34024, 34076, 3408, 3410, 3411, 3412, 3413, 3414,
  3415, 34467, 34816, 34817, 34818, 34819, 34877, 34921, 34930, 35660, 35661, 35724, 35738, 35739, 36003, 36004, 36005,
  36347, 36348, 36349, 37440, 37441, 37443, 7936, 7937, 7938,
])
const validExtensionParams = new Set([
  34047, 35723, 36063, 34852, 34853, 34854, 34229, 36392, 36795, 38449,
])
const shaderTypes = ['FRAGMENT_SHADER', 'VERTEX_SHADER']
const precisionTypes = ['LOW_FLOAT', 'MEDIUM_FLOAT', 'HIGH_FLOAT', 'LOW_INT', 'MEDIUM_INT', 'HIGH_INT']
const rendererInfoExtensionName = 'WEBGL_debug_renderer_info'
const polygonModeExtensionName = 'WEBGL_polygon_mode'

export function getWebGlBasics({ cache }) {
  const gl = getWebGLContext(cache)
  if (!gl) return STATUS_NO_GL_CONTEXT
  if (!isValidParameterGetter(gl)) return STATUS_GET_PARAMETER_NOT_A_FUNCTION

  const debugExtension = shouldAvoidDebugRendererInfo() ? null : gl.getExtension(rendererInfoExtensionName)
  return {
    version: gl.getParameter(gl.VERSION)?.toString() || '',
    vendor: gl.getParameter(gl.VENDOR)?.toString() || '',
    vendorUnmasked: debugExtension ? gl.getParameter(debugExtension.UNMASKED_VENDOR_WEBGL)?.toString() : '',
    renderer: gl.getParameter(gl.RENDERER)?.toString() || '',
    rendererUnmasked: debugExtension ? gl.getParameter(debugExtension.UNMASKED_RENDERER_WEBGL)?.toString() : '',
    shadingLanguageVersion: gl.getParameter(gl.SHADING_LANGUAGE_VERSION)?.toString() || '',
  }
}

export function getWebGlExtensions({ cache }) {
  const gl = getWebGLContext(cache)
  if (!gl) return STATUS_NO_GL_CONTEXT
  if (!isValidParameterGetter(gl)) return STATUS_GET_PARAMETER_NOT_A_FUNCTION

  const extensions = gl.getSupportedExtensions()
  const contextAttributes = gl.getContextAttributes()
  const unsupportedExtensions = []
  const attributes = []
  const parameters = []
  const extensionParameters = []
  const shaderPrecisions = []

  if (contextAttributes) {
    for (const attributeName of Object.keys(contextAttributes)) {
      attributes.push(`${attributeName}=${contextAttributes[attributeName]}`)
    }
  }

  const constants = getConstantsFromPrototype(gl)
  for (const constant of constants) {
    const code = gl[constant]
    parameters.push(`${constant}=${code}${validContextParameters.has(code) ? `=${gl.getParameter(code)}` : ''}`)
  }

  if (extensions) {
    for (const name of extensions) {
      if (
        (name === rendererInfoExtensionName && shouldAvoidDebugRendererInfo()) ||
        (name === polygonModeExtensionName && shouldAvoidPolygonModeExtensions())
      ) {
        continue
      }
      const extension = gl.getExtension(name)
      if (!extension) {
        unsupportedExtensions.push(name)
        continue
      }
      for (const constant of getConstantsFromPrototype(extension)) {
        const code = extension[constant]
        extensionParameters.push(
          `${constant}=${code}${validExtensionParams.has(code) ? `=${gl.getParameter(code)}` : ''}`,
        )
      }
    }
  }

  for (const shaderType of shaderTypes) {
    for (const precisionType of precisionTypes) {
      const shaderPrecision = getShaderPrecision(gl, shaderType, precisionType)
      shaderPrecisions.push(`${shaderType}.${precisionType}=${shaderPrecision.join(',')}`)
    }
  }

  extensionParameters.sort()
  parameters.sort()

  return {
    contextAttributes: attributes,
    parameters,
    shaderPrecisions,
    extensions,
    extensionParameters,
    unsupportedExtensions,
  }
}

export function getWebGLContext(cache) {
  if (cache.webgl) return cache.webgl.context

  const canvas = document.createElement('canvas')
  let context

  canvas.addEventListener('webglCreateContextError', () => (context = undefined))

  for (const type of ['webgl', 'experimental-webgl']) {
    try {
      context = canvas.getContext(type)
    } catch {
      // continue
    }
    if (context) break
  }

  cache.webgl = { context }
  return context
}

function getShaderPrecision(gl, shaderType, precisionType) {
  const shaderPrecision = gl.getShaderPrecisionFormat(gl[shaderType], gl[precisionType])
  return shaderPrecision ? [shaderPrecision.rangeMin, shaderPrecision.rangeMax, shaderPrecision.precision] : []
}

function getConstantsFromPrototype(obj) {
  const keys = Object.keys(obj.__proto__)
  return keys.filter(isConstantLike)
}

function isConstantLike(key) {
  return typeof key === 'string' && !key.match(/[^A-Z0-9_x]/)
}

export function shouldAvoidDebugRendererInfo() {
  return isGecko()
}

export function shouldAvoidPolygonModeExtensions() {
  return isChromium() || isWebKit()
}

function isValidParameterGetter(gl) {
  return typeof gl.getParameter === 'function'
}
