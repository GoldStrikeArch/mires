// jmespath's TreeInterpreter and the value helpers it uses (strictDeepEqual, isFalse, objValues).

package jmespath

import scala.collection.mutable
import scala.scalajs.js

import Errors.{runtimeError, syntaxError}
import Host.*

private object Interpreter:
  /** The part of `Object.prototype.toString` that strictDeepEqual distinguishes. */
  private def typeTag(value: js.Any): String =
    if isNull(value) then "null" else if isArray(value) then "array" else js.typeOf(value)

  /** jmespath's `strictDeepEqual`: `===`, then the same `Object.prototype.toString` tag, then by
    * content. Objects compare every own key of `first` against `second[key]` (so an undefined value
    * equals a missing key), then require `second`'s own keys to be among them.
    */
  def strictDeepEqual(first: js.Any, second: js.Any): Boolean =
    if strictEquals(first, second) then true
    else
      val firstType = typeTag(first)
      if firstType != typeTag(second) then false
      else if firstType == "array" then
        val a = first.asInstanceOf[js.Array[js.Any]]
        val b = second.asInstanceOf[js.Array[js.Any]]
        a.length == b.length && a.indices.forall(i => strictDeepEqual(a(i), b(i)))
      else if firstType == "object" then
        val keysSeen = mutable.Set.empty[String]
        keys(first).forall { key =>
          keysSeen += key
          strictDeepEqual(property(first, key), property(second, key))
        } && keys(second).forall(keysSeen.contains)
      else false

  /** jmespath's `isFalse`: "", false, null, an empty array and an object with no own keys. */
  def isFalse(value: js.Any): Boolean =
    if stringValue(value).contains("") || strictEquals(value, false) || isNull(value) then true
    else if isArray(value) then value.asInstanceOf[js.Array[js.Any]].length == 0
    else if isObject(value) then keys(value).length == 0
    else false

  def objValues(obj: js.Any): js.Array[js.Any] = keys(obj).map(property(obj, _))

final class TreeInterpreter:
  import Interpreter.*

  private val runtime = Runtime(this)

  def search(node: js.UndefOr[Node], value: js.Any): js.Any = visit(node, value)

  def visit(maybeNode: js.UndefOr[Node], value: js.Any): js.Any =
    // `node.type` of a missing node is a TypeError in jmespath.
    val node = maybeNode.filter(n => !isNull(n)).getOrElse(throw syntaxError("Cannot read properties of undefined"))
    def child(i: Int): js.UndefOr[Node] = node.children(i)
    stringValue(node.`type`).getOrElse("") match
      case "Field" =>
        if !isObject(value) then null
        else
          val field = property(value, node.name.asInstanceOf[String])
          if isUndefined(field) then null else field
      case "Subexpression" | "IndexExpression" | "Pipe" => visit(child(1), visit(child(0), value))
      case "Index" =>
        if !isArray(value) then null
        else
          val array = value.asInstanceOf[js.Array[js.Any]]
          val index = node.value.asInstanceOf[Double]
          val result = elementAt(array, if index < 0 then index + array.length else index)
          if isUndefined(result) then null else result
      case "Slice" =>
        if isArray(value) then slice(value.asInstanceOf[js.Array[js.Any]], node.children.asInstanceOf[js.Array[js.Any]]) else null
      case "Projection" =>
        val base = visit(child(0), value)
        if !isArray(base) then null else collect(base.asInstanceOf[js.Array[js.Any]], child(1))
      case "ValueProjection" =>
        val base = visit(child(0), value)
        if !isObject(base) then null else collect(objValues(base), child(1))
      case "FilterProjection" =>
        val base = visit(child(0), value)
        if !isArray(base) then null
        else
          // Every condition first, then the projection: the first error is jmespath's.
          val filtered = base.asInstanceOf[js.Array[js.Any]].filter(element => !isFalse(visit(child(2), element)))
          collect(filtered, child(1))
      case "Comparator" =>
        val first = visit(child(0), value)
        val second = visit(child(1), value)
        stringValue(node.name).getOrElse("") match
          case "EQ" => strictDeepEqual(first, second)
          case "NE" => !strictDeepEqual(first, second)
          case "GT" => jsGreater(first, second)
          case "GTE" => jsGreaterOrEqual(first, second)
          case "LT" => jsLess(first, second)
          case "LTE" => jsLessOrEqual(first, second)
          case _ => throw syntaxError(s"Unknown comparator: ${node.name}")
      case "Flatten" =>
        val original = visit(child(0), value)
        if !isArray(original) then null
        else
          val merged = js.Array[js.Any]()
          for current <- original.asInstanceOf[js.Array[js.Any]] do
            if isArray(current) then current.asInstanceOf[js.Array[js.Any]].foreach(merged.push(_)) else merged.push(current)
          merged
      case "Identity" | "Current" => value
      case "MultiSelectList" =>
        if isNull(value) then null else node.children.map(visit(_, value))
      case "MultiSelectHash" =>
        if isNull(value) then null
        else
          val collected = js.Dictionary.empty[js.Any]
          for pair <- node.children do collected(pair.name.asInstanceOf[String]) = visit(pair.value.asInstanceOf[js.UndefOr[Node]], value)
          collected
      case "OrExpression" =>
        val matched = visit(child(0), value)
        if isFalse(matched) then visit(child(1), value) else matched
      case "AndExpression" =>
        val first = visit(child(0), value)
        if isFalse(first) then first else visit(child(1), value)
      case "NotExpression" => isFalse(visit(child(0), value))
      case "Literal" => node.value
      case "Function" => runtime.callFunction(node.name, node.children.map(visit(_, value)))
      case "ExpressionReference" =>
        // jmespath tags the referenced node in place and returns the node itself.
        val refNode = child(0).getOrElse(throw syntaxError("Cannot set properties of undefined"))
        refNode.jmespathType = "Expref"
        refNode
      case _ => throw syntaxError(s"Unknown node type: ${node.`type`}")

  /** Projections keep every result except null; undefined (an empty `max_by`) is kept. */
  private def collect(base: js.Array[js.Any], right: js.UndefOr[Node]): js.Array[js.Any] =
    val collected = js.Array[js.Any]()
    for element <- base do
      val current = visit(right, element)
      if !isNull(current) then collected.push(current)
    collected

  /** `Slice` with jmespath's computeSliceParams, in JS numbers: the tokens come from parseInt, so a
    * bound may be NaN (`[0:3:-]`) or past 2^53, and the loop reads `value[i]` as it goes.
    */
  private def slice(array: js.Array[js.Any], params: js.Array[js.Any]): js.Array[js.Any] =
    def param(i: Int): Option[Double] = if isNull(params(i)) then None else Some(params(i).asInstanceOf[Double])
    val length = array.length.toDouble
    val step = param(2) match
      case Some(0) => throw runtimeError("Invalid slice, step cannot be 0")
      case Some(step) => step
      case None => 1.0
    val stepValueNegative = step < 0
    val start = param(0).fold(if stepValueNegative then length - 1 else 0.0)(capSliceRange(length, _, step))
    val stop = param(1).fold(if stepValueNegative then -1.0 else length)(capSliceRange(length, _, step))
    val result = js.Array[js.Any]()
    var i = start
    if step > 0 then
      while i < stop do
        result.push(elementAt(array, i))
        i += step
    else
      while i > stop do
        result.push(elementAt(array, i))
        i += step
    result

  private def capSliceRange(arrayLength: Double, actualValue: Double, step: Double): Double =
    if actualValue < 0 then
      val value = actualValue + arrayLength
      if value < 0 then (if step < 0 then -1.0 else 0.0) else value
    else if actualValue >= arrayLength then (if step < 0 then arrayLength - 1 else arrayLength)
    else actualValue
