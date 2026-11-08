package fp

import scala.scalajs.js

object Webgl {
  val STATUS_NO_GL_CONTEXT = -1
  val STATUS_GET_PARAMETER_NOT_A_FUNCTION = -2

  private val validContextParameters = Set(
    10752, 2849, 2884, 2885, 2886, 2928, 2929, 2930, 2931, 2932, 2960, 2961, 2962, 2963, 2964, 2965, 2966, 2967, 2968,
    2978, 3024, 3042, 3088, 3089, 3106, 3107, 32773, 32777, 32823, 32824, 32936, 32937, 32938, 32939, 32968, 32969,
    32970, 32971, 3317, 33170, 3333, 3379, 3386, 33901, 33902, 34016, 34024, 34076, 3408, 3410, 3411, 3412, 3413, 3414,
    3415, 34467, 34816, 34817, 34818, 34819, 34877, 34921, 34930, 35660, 35661, 35724, 35738, 35739, 36003, 36004, 36005,
    36347, 36348, 36349, 37440, 37441, 37443, 7936, 7937, 7938,
  )
  private val validExtensionParams = Set(
    34047, 35723, 36063, 34852, 34853, 34854, 34229, 36392, 36795, 38449,
  )
  private val shaderTypes = js.Array("FRAGMENT_SHADER", "VERTEX_SHADER")
  private val precisionTypes = js.Array("LOW_FLOAT", "MEDIUM_FLOAT", "HIGH_FLOAT", "LOW_INT", "MEDIUM_INT", "HIGH_INT")
  private val rendererInfoExtensionName = "WEBGL_debug_renderer_info"
  private val polygonModeExtensionName = "WEBGL_polygon_mode"

  def getWebGlBasics(options: js.Any): js.Any = {
    val cache = Host.get(options, "cache")
    val gl = getWebGLContext(cache)
    if (Host.isUndef(gl)) return STATUS_NO_GL_CONTEXT
    if (!isValidParameterGetter(gl)) return STATUS_GET_PARAMETER_NOT_A_FUNCTION
    val debugExtension =
      if (shouldAvoidDebugRendererInfo()) null
      else Host.dyn(gl).getExtension(rendererInfoExtensionName)
    def param(code: js.Any): String = {
      val value = Host.dyn(gl).getParameter(code)
      if (Host.isUndef(value.asInstanceOf[js.Any]) || value == null) ""
      else Host.dyn(value).toString().asInstanceOf[String]
    }
    val vendorUnmasked =
      if (Host.jsTruthy(debugExtension.asInstanceOf[js.Any]))
        param(Host.dyn(debugExtension).UNMASKED_VENDOR_WEBGL)
      else ""
    val rendererUnmasked =
      if (Host.jsTruthy(debugExtension.asInstanceOf[js.Any]))
        param(Host.dyn(debugExtension).UNMASKED_RENDERER_WEBGL)
      else ""
    js.Dynamic.literal(
      version = param(Host.dyn(gl).VERSION),
      vendor = param(Host.dyn(gl).VENDOR),
      vendorUnmasked = vendorUnmasked,
      renderer = param(Host.dyn(gl).RENDERER),
      rendererUnmasked = rendererUnmasked,
      shadingLanguageVersion = param(Host.dyn(gl).SHADING_LANGUAGE_VERSION),
    )
  }

  def getWebGlExtensions(options: js.Any): js.Any = {
    val cache = Host.get(options, "cache")
    val gl = getWebGLContext(cache)
    if (Host.isUndef(gl)) return STATUS_NO_GL_CONTEXT
    if (!isValidParameterGetter(gl)) return STATUS_GET_PARAMETER_NOT_A_FUNCTION

    val extensions = Host.dyn(gl).getSupportedExtensions()
    val contextAttributes = Host.dyn(gl).getContextAttributes()
    val unsupportedExtensions = js.Array[String]()
    val attributes = js.Array[String]()
    val parameters = js.Array[String]()
    val extensionParameters = js.Array[String]()
    val shaderPrecisions = js.Array[String]()

    if (Host.jsTruthy(contextAttributes.asInstanceOf[js.Any])) {
      val names = Host.objectKeys(contextAttributes.asInstanceOf[js.Any])
      var i = 0
      while (i < names.length) {
        val attributeName = names(i)
        attributes.push(s"$attributeName=${Host.toJsString(Host.get(contextAttributes.asInstanceOf[js.Any], attributeName))}")
        i += 1
      }
    }

    val constants = getConstantsFromPrototype(gl)
    var c = 0
    while (c < constants.length) {
      val constant = constants(c)
      val code = Host.get(gl, constant)
      val codeNum = Host.asNumber(code).toInt
      val extra =
        if (validContextParameters.contains(codeNum))
          s"=${Host.toJsString(Host.dyn(gl).getParameter(code).asInstanceOf[js.Any])}"
        else ""
      parameters.push(s"$constant=${Host.toJsString(code)}$extra")
      c += 1
    }

    if (Host.jsTruthy(extensions.asInstanceOf[js.Any])) {
      val extArr = extensions.asInstanceOf[js.Array[String]]
      var e = 0
      while (e < extArr.length) {
        val name = extArr(e)
        if (
          (name == rendererInfoExtensionName && shouldAvoidDebugRendererInfo()) ||
          (name == polygonModeExtensionName && shouldAvoidPolygonModeExtensions())
        ) ()
        else {
          val extension = Host.dyn(gl).getExtension(name)
          if (!Host.jsTruthy(extension.asInstanceOf[js.Any])) unsupportedExtensions.push(name)
          else {
            val extConstants = getConstantsFromPrototype(extension.asInstanceOf[js.Any])
            var k = 0
            while (k < extConstants.length) {
              val constant = extConstants(k)
              val code = Host.get(extension.asInstanceOf[js.Any], constant)
              val codeNum = Host.asNumber(code).toInt
              val extra =
                if (validExtensionParams.contains(codeNum))
                  s"=${Host.toJsString(Host.dyn(gl).getParameter(code).asInstanceOf[js.Any])}"
                else ""
              extensionParameters.push(s"$constant=${Host.toJsString(code)}$extra")
              k += 1
            }
          }
        }
        e += 1
      }
    }

    var s = 0
    while (s < shaderTypes.length) {
      var p = 0
      while (p < precisionTypes.length) {
        val shaderType = shaderTypes(s)
        val precisionType = precisionTypes(p)
        val shaderPrecision = getShaderPrecision(gl, shaderType, precisionType)
        shaderPrecisions.push(s"$shaderType.$precisionType=${shaderPrecision.join(",")}")
        p += 1
      }
      s += 1
    }

    extensionParameters.sort()
    parameters.sort()
    js.Dynamic.literal(
      contextAttributes = attributes,
      parameters = parameters,
      shaderPrecisions = shaderPrecisions,
      extensions = extensions,
      extensionParameters = extensionParameters,
      unsupportedExtensions = unsupportedExtensions,
    )
  }

  def getWebGLContext(cache: js.Any): js.Any = {
    val existing = Host.get(cache, "webgl")
    if (Host.jsTruthy(existing)) return Host.get(existing, "context")
    val canvas = Host.document.createElement("canvas")
    var context: js.Any = js.undefined
    canvas.addEventListener("webglCreateContextError", () => { context = js.undefined })
    val types = js.Array("webgl", "experimental-webgl")
    var i = 0
    while (i < types.length && !Host.jsTruthy(context)) {
      try context = canvas.getContext(types(i)).asInstanceOf[js.Any]
      catch { case _: Throwable => () }
      i += 1
    }
    if (!Host.isUndef(cache) && cache != null)
      Host.set(cache, "webgl", js.Dynamic.literal(context = context))
    context
  }

  private def getShaderPrecision(gl: js.Any, shaderType: String, precisionType: String): js.Array[js.Any] = {
    val shaderPrecision =
      Host.dyn(gl).getShaderPrecisionFormat(Host.get(gl, shaderType), Host.get(gl, precisionType))
    if (Host.jsTruthy(shaderPrecision.asInstanceOf[js.Any]))
      js.Array(
        Host.dyn(shaderPrecision).rangeMin.asInstanceOf[js.Any],
        Host.dyn(shaderPrecision).rangeMax.asInstanceOf[js.Any],
        Host.dyn(shaderPrecision).precision.asInstanceOf[js.Any],
      )
    else js.Array()
  }

  private def getConstantsFromPrototype(obj: js.Any): js.Array[String] = {
    val proto = Host.dyn(obj).selectDynamic("__proto__")
    Host.objectKeys(proto.asInstanceOf[js.Any]).filter(isConstantLike)
  }

  private def isConstantLike(key: String): Boolean =
    !new js.RegExp("[^A-Z0-9_x]").test(key)

  def shouldAvoidDebugRendererInfo(): Boolean = Browser.isGecko()

  def shouldAvoidPolygonModeExtensions(): Boolean = Browser.isChromium() || Browser.isWebKit()

  private def isValidParameterGetter(gl: js.Any): Boolean =
    Host.isJsFunction(Host.get(gl, "getParameter"))

  val basicsSource: js.Function1[js.Any, js.Any] = getWebGlBasics
  val extensionsSource: js.Function1[js.Any, js.Any] = getWebGlExtensions
  val getWebGLContextFn: js.Function1[js.Any, js.Any] = getWebGLContext
}
