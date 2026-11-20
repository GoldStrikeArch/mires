// The JavaScript values jmespath works on, and the host functions it calls.
//
// jmespath 0.16.0 evaluates over plain JS values (the JSON document, objects it builds, and its own
// AST nodes, which `&expr` hands to functions and to the caller) and leans on JS semantics for them:
// `Object.keys` order, `===` identity in `contains`, relational operators with type coercion,
// `JSON.parse`/`JSON.stringify`, `parseInt`, `Number(string)`, `localeCompare` and the default
// `Array.prototype.sort`. The port keeps those values as `js.Any` and reaches those functions through
// Scala.js facades, so every one of those semantics is the engine's own.

package jmespath

import scala.scalajs.js
import scala.scalajs.js.annotation.JSGlobalScope

/** An AST node as jmespath builds it: a plain object whose properties depend on `type`. */
@js.native
trait Node extends js.Object:
  val `type`: js.Any = js.native
  val name: js.Any = js.native
  val value: js.Any = js.native
  val children: js.Array[Node] = js.native
  var jmespathType: js.UndefOr[String] = js.native

@js.native
@JSGlobalScope
private[jmespath] object Globals extends js.Object:
  def parseInt(text: String): Double = js.native

  /** `Number(value)`: the unary `+` that `to_number` applies to strings. */
  def Number(value: js.Any): Double = js.native

  /** `String(value)`: the conversion `String.prototype.indexOf` applies to its argument. */
  def String(value: js.Any): String = js.native

private[jmespath] object Host:
  /** `undefined` as a JS value: an empty `max_by`, `type()` of it, an array read past the end. */
  val undefined: js.Any = js.undefined.asInstanceOf[js.Any]

  def isUndefined(value: js.Any): Boolean = js.isUndefined(value)

  def isNull(value: js.Any): Boolean = value eq null

  def isArray(value: js.Any): Boolean = js.Array.isArray(value)

  def isString(value: js.Any): Boolean = js.typeOf(value) == "string"

  def isNumber(value: js.Any): Boolean = js.typeOf(value) == "number"

  /** jmespath's `isObject`: `Object.prototype.toString` is `[object Object]`, which for these values
    * means a non-null, non-array object (JSON objects, built objects and AST nodes alike).
    */
  def isObject(value: js.Any): Boolean = js.typeOf(value) == "object" && !isNull(value) && !isArray(value)

  /** A JS string as a Scala string, or None for any other value. */
  def stringValue(value: js.Any): Option[String] = if isString(value) then Some(value.asInstanceOf[String]) else None

  /** `object[key]`, through the prototype chain as in JS. */
  def property(obj: js.Any, key: String): js.Any = obj.asInstanceOf[js.Dynamic].selectDynamic(key)

  def keys(obj: js.Any): js.Array[String] = js.Object.keys(obj.asInstanceOf[js.Object])

  /** `array[index]` for a JS number index: an element, or undefined off the end, at NaN and at ±∞. */
  def elementAt(array: js.Array[js.Any], index: Double): js.Any =
    if index >= 0 && index < array.length && index == math.floor(index) then array(index.toInt) else undefined

  def parseJson(text: String): js.Any = js.JSON.parse(text)

  def strictEquals(a: js.Any, b: js.Any): Boolean = js.special.strictEquals(a, b)

  /** The relational operators of jmespath's comparators, `max_by` and `sort_by`, with JS coercion. */
  def jsGreater(a: js.Any, b: js.Any): Boolean = (a.asInstanceOf[js.Dynamic] > b.asInstanceOf[js.Dynamic]).asInstanceOf[Boolean]
  def jsGreaterOrEqual(a: js.Any, b: js.Any): Boolean = (a.asInstanceOf[js.Dynamic] >= b.asInstanceOf[js.Dynamic]).asInstanceOf[Boolean]
  def jsLess(a: js.Any, b: js.Any): Boolean = (a.asInstanceOf[js.Dynamic] < b.asInstanceOf[js.Dynamic]).asInstanceOf[Boolean]
  def jsLessOrEqual(a: js.Any, b: js.Any): Boolean = (a.asInstanceOf[js.Dynamic] <= b.asInstanceOf[js.Dynamic]).asInstanceOf[Boolean]

  /** JS `text.replace(from, to)` with a string pattern: the first occurrence only, no regex (Java's
    * `replaceFirst` would read `\'` as a regex for a bare quote).
    */
  def replaceFirst(text: String, from: String, to: String): String =
    val at = text.indexOf(from)
    if at < 0 then text else text.substring(0, at) + to + text.substring(at + from.length)
