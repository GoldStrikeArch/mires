package fp

import scala.scalajs.js
import scala.scalajs.js.annotation.JSGlobal
import scala.scalajs.js.typedarray.Uint8Array

/** Small JS-host helpers. Browser globals are only read inside methods. */
object Host {
  def g: js.Dynamic = js.Dynamic.global.globalThis.asInstanceOf[js.Dynamic]

  def window: js.Dynamic = g.selectDynamic("window")
  def document: js.Dynamic = g.selectDynamic("document")
  def navigator: js.Dynamic = g.selectDynamic("navigator")
  def screen: js.Dynamic = g.selectDynamic("screen")
  def location: js.Dynamic = g.selectDynamic("location")

  def jsType(value: js.Any): String = js.typeOf(value)

  def isUndef(value: js.Any): Boolean =
    js.isUndefined(value) || value == null

  def jsTruthy(value: js.Any): Boolean =
    if (isUndef(value)) false
    else
      jsType(value) match {
        case "boolean" => value.asInstanceOf[Boolean]
        case "number" =>
          val n = value.asInstanceOf[Double]
          n != 0 && !n.isNaN
        case "string" => value.asInstanceOf[String].length != 0
        case _        => true
      }

  def has(obj: js.Any, key: String): Boolean =
    !isUndef(obj) && js.Object.hasProperty(obj.asInstanceOf[js.Object], key)

  def dyn(value: js.Any): js.Dynamic = value.asInstanceOf[js.Dynamic]

  def get(obj: js.Any, key: String): js.Any =
    if (isUndef(obj)) js.undefined
    else dyn(obj).selectDynamic(key)

  def set(obj: js.Any, key: String, value: js.Any): Unit =
    dyn(obj).updateDynamic(key)(value.asInstanceOf[js.Any])

  def call(fn: js.Any, thisArg: js.Any, args: js.Any*): js.Any = {
    val applied = js.Array(thisArg) ++ args
    dyn(fn).applyDynamic("call")(applied.toSeq*).asInstanceOf[js.Any]
  }

  def jsNew(ctor: js.Any)(args: js.Any*): js.Dynamic =
    js.Dynamic.newInstance(dyn(ctor))(args*)

  def isUndefined(value: js.Any): Boolean = js.isUndefined(value)

  def toJsString(value: js.Any): String =
    if (js.isUndefined(value)) "undefined"
    else if (value == null) "null"
    else "" + value

  def nameFunction(fn: js.Any, name: String): js.Any = {
    g.Object.defineProperty(
      fn,
      "name",
      js.Dynamic.literal(value = name, configurable = true),
    )
    fn
  }

  def source0(name: String, impl: js.Function1[js.Any, js.Any]): js.Function = {
    val factory = js.Dynamic.newInstance(g.Function)(
      "impl",
      "name",
      """return Object.defineProperty(function(){ return impl.apply(null, arguments); }, "name", { value: name, configurable: true });""",
    )
    factory(impl, name).asInstanceOf[js.Function]
  }

  def source1(name: String, impl: js.Function1[js.Any, js.Any]): js.Function = {
    val factory = js.Dynamic.newInstance(g.Function)(
      "impl",
      "name",
      """return Object.defineProperty(function(options){ return impl(options); }, "name", { value: name, configurable: true });""",
    )
    factory(impl, name).asInstanceOf[js.Function]
  }

  def orEmpty(value: js.Any): String =
    if (jsTruthy(value)) toJsString(value) else ""

  def asNumber(value: js.Any): Double =
    if (isUndef(value) || value == null) Double.NaN
    else
      jsType(value) match {
        case "number" => value.asInstanceOf[Double]
        case _        => g.Number(value).asInstanceOf[Double]
      }

  def parseInt(value: js.Any): Double =
    g.parseInt(value).asInstanceOf[Double]

  def parseFloat(value: js.Any): Double =
    g.parseFloat(value).asInstanceOf[Double]

  def dateNow(): Double = js.Date.now()

  def objectKeys(value: js.Any): js.Array[String] =
    if (isUndef(value) || value == null) js.Array()
    else js.Object.keys(value.asInstanceOf[js.Object])

  def jsonStringify(value: js.Any): js.UndefOr[String] =
    js.JSON.stringify(value).asInstanceOf[js.UndefOr[String]]

  def jsonStringifyPretty(value: js.Any, replacer: js.Function2[js.Any, js.Any, js.Any], space: Int): String =
    g.JSON.stringify(value, replacer, space).asInstanceOf[String]

  def isJsFunction(value: js.Any): Boolean =
    jsType(value) == "function"

  def isJsObject(value: js.Any): Boolean =
    jsTruthy(value) && jsType(value) == "object"

  def instanceofError(value: js.Any): Boolean =
    !isUndef(value) && js.special.instanceof(value, g.Error)

  def instanceofDomException(value: js.Any): Boolean =
    !isUndef(value) && js.special.instanceof(value, g.DOMException)

  def matchMedia(query: String): js.Dynamic =
    g.matchMedia(query).asInstanceOf[js.Dynamic]

  def mediaMatches(query: String): Boolean =
    jsTruthy(matchMedia(query).matches.asInstanceOf[js.Any])

  def textEncoderEncode(input: String): Uint8Array =
    jsNew(g.TextEncoder)().asInstanceOf[js.Dynamic].encode(input).asInstanceOf[Uint8Array]

  def defineEnumerableAccessor(obj: js.Any, key: String, getter: js.Function0[js.Any], setter: js.Function1[js.Any, Unit]): Unit = {
    g.Object.defineProperty(
      obj,
      key,
      js.Dynamic.literal(
        enumerable = true,
        configurable = true,
        get = getter,
        set = setter,
      ),
    )
    ()
  }

  def consoleLog(message: String): Unit =
    g.console.log(message)

  def consoleError(error: js.Any): Unit =
    g.console.error(error)

  def atob(value: String): String =
    g.atob(value).asInstanceOf[String]

  def fromCharCode(units: Int*): String =
    g.String.fromCharCode(units.map(_.asInstanceOf[js.Any])*).asInstanceOf[String]
}

@js.native
@JSGlobal("Object")
object JsObjectCtor extends js.Object {
  def keys(o: js.Any): js.Array[String] = js.native
  def defineProperty(o: js.Any, p: String, attributes: js.Any): js.Any = js.native
}
