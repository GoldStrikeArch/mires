// jmespath's Runtime: the function table, argument validation and the 26 built-in functions.

package jmespath

import scala.scalajs.js
import scala.scalajs.js.JSStringOps.*

import Errors.{argumentError, typeError, unknownFunction}
import Host.*
import Interpreter.objValues

/** jmespath's TYPE_* constants, with the names its error messages use. */
enum JmesType(val label: String):
  case Number extends JmesType("number")
  case Any extends JmesType("any")
  case String extends JmesType("string")
  case Array extends JmesType("array")
  case Object extends JmesType("object")
  case Boolean extends JmesType("boolean")
  case Expref extends JmesType("expression")
  case Null extends JmesType("null")
  case ArrayNumber extends JmesType("Array<number>")
  case ArrayString extends JmesType("Array<string>")

private object Runtime:
  /** jmespath's `_getTypeName`; None where its switch falls through (undefined). */
  def typeName(value: js.Any): Option[JmesType] = js.typeOf(value) match
    case "string" => Some(JmesType.String)
    case "number" => Some(JmesType.Number)
    case "boolean" => Some(JmesType.Boolean)
    case "object" =>
      if isNull(value) then Some(JmesType.Null)
      else if isArray(value) then Some(JmesType.Array)
      else if value.asInstanceOf[Node].jmespathType.contains("Expref") then Some(JmesType.Expref)
      else Some(JmesType.Object)
    case _ => None

  final case class Parameter(types: Seq[JmesType], variadic: Boolean = false)

  final case class Function(signature: Seq[Parameter], body: (Runtime, js.Array[js.Any]) => js.Any)

  private def param(types: JmesType*) = Parameter(types)

  extension (args: js.Array[js.Any])
    private def array(i: Int) = args(i).asInstanceOf[js.Array[js.Any]]
    private def number(i: Int) = args(i).asInstanceOf[Double]
    private def string(i: Int) = args(i).asInstanceOf[String]
    private def node(i: Int) = args(i).asInstanceOf[Node]

  /** `for (key in value)` over a value jmespath's merge did not type-check: an object's or array's
    * own keys, a string's indices, and nothing for other primitives, null and undefined.
    */
  private def forIn(value: js.Any)(visit: (String, js.Any) => Unit): Unit =
    if isString(value) then
      val text = value.asInstanceOf[String]
      for i <- 0 until text.length do visit(i.toString, text.charAt(i).toString)
    else if js.typeOf(value) == "object" && !isNull(value) then
      for key <- keys(value) do visit(key, property(value, key))

  private def extremeBy(runtime: Runtime, args: js.Array[js.Any], better: (js.Any, js.Any) => Boolean, initial: Double): js.Any =
    val keyFunction = runtime.createKeyFunction(args.node(1), Seq(JmesType.Number, JmesType.String))
    var best: js.Any = initial
    var record: js.Any = undefined
    for element <- args.array(0) do
      val current = keyFunction(element)
      if better(current, best) then
        best = current
        record = element
    record

  val functionTable: Map[String, Function] = Map(
    "abs" -> Function(Seq(param(JmesType.Number)), (_, args) => math.abs(args.number(0))),
    "avg" -> Function(
      Seq(param(JmesType.ArrayNumber)),
      (_, args) =>
        val inputArray = args.array(0)
        var sum = 0.0
        for x <- inputArray do sum += x.asInstanceOf[Double]
        sum / inputArray.length
    ),
    "ceil" -> Function(Seq(param(JmesType.Number)), (_, args) => math.ceil(args.number(0))),
    "contains" -> Function(
      Seq(param(JmesType.String, JmesType.Array), param(JmesType.Any)),
      // `indexOf`: String.prototype.indexOf converts its argument with String(); the array version
      // compares with `===`, so only the same object is found.
      (_, args) =>
        if isString(args(0)) then args.string(0).indexOf(Globals.String(args(1))) >= 0
        else args.array(0).exists(strictEquals(_, args(1)))
    ),
    "ends_with" -> Function(
      Seq(param(JmesType.String), param(JmesType.String)),
      (_, args) =>
        val searchStr = args.string(0)
        val suffix = args.string(1)
        searchStr.indexOf(suffix, searchStr.length - suffix.length) != -1
    ),
    "floor" -> Function(Seq(param(JmesType.Number)), (_, args) => math.floor(args.number(0))),
    "length" -> Function(
      Seq(param(JmesType.String, JmesType.Array, JmesType.Object)),
      (_, args) =>
        if isString(args(0)) then args.string(0).length
        else if isArray(args(0)) then args.array(0).length
        else keys(args(0)).length
    ),
    "map" -> Function(
      Seq(param(JmesType.Expref), param(JmesType.Array)),
      (runtime, args) => args.array(1).map(runtime.interpreter.visit(args.node(0), _))
    ),
    "max" -> Function(
      Seq(param(JmesType.ArrayNumber, JmesType.ArrayString)),
      (_, args) =>
        val elements = args.array(0)
        if elements.length == 0 then null
        // Math.max.apply: NaN wins, and +0 is larger than -0 (java.lang.Math.max agrees).
        else if typeName(elements(0)).contains(JmesType.Number) then
          elements.foldLeft(Double.NegativeInfinity)((acc, x) => math.max(acc, x.asInstanceOf[Double]))
        else
          elements.reduceLeft((maxElement, x) =>
            if maxElement.asInstanceOf[String].localeCompare(x.asInstanceOf[String]) < 0 then x else maxElement
          )
    ),
    "merge" -> Function(
      Seq(Parameter(Seq(JmesType.Object), variadic = true)),
      // `for (key in current)`: only the first argument is type-checked, so later ones may be
      // strings or arrays (their index keys) or primitives and null (no keys).
      (_, args) =>
        val merged = js.Dictionary.empty[js.Any]
        for current <- args do forIn(current)((key, value) => merged(key) = value)
        merged
    ),
    "max_by" -> Function(
      Seq(param(JmesType.Array), param(JmesType.Expref)),
      (runtime, args) => extremeBy(runtime, args, jsGreater, Double.NegativeInfinity)
    ),
    "sum" -> Function(
      Seq(param(JmesType.ArrayNumber)),
      (_, args) => args.array(0).foldLeft(0.0)((sum, x) => sum + x.asInstanceOf[Double])
    ),
    "starts_with" -> Function(
      Seq(param(JmesType.String), param(JmesType.String)),
      // `lastIndexOf(prefix) === 0`, searching from the end: `starts_with('abab', 'ab')` is false.
      (_, args) => args.string(0).lastIndexOf(args.string(1), args.string(0).length) == 0
    ),
    "min" -> Function(
      Seq(param(JmesType.ArrayNumber, JmesType.ArrayString)),
      (_, args) =>
        val elements = args.array(0)
        if elements.length == 0 then null
        else if typeName(elements(0)).contains(JmesType.Number) then
          elements.foldLeft(Double.PositiveInfinity)((acc, x) => math.min(acc, x.asInstanceOf[Double]))
        else
          elements.reduceLeft((minElement, x) =>
            if x.asInstanceOf[String].localeCompare(minElement.asInstanceOf[String]) < 0 then x else minElement
          )
    ),
    "min_by" -> Function(
      Seq(param(JmesType.Array), param(JmesType.Expref)),
      (runtime, args) => extremeBy(runtime, args, jsLess, Double.PositiveInfinity)
    ),
    "type" -> Function(
      Seq(param(JmesType.Any)),
      (_, args) =>
        typeName(args(0)) match
          case Some(JmesType.Number) => "number"
          case Some(JmesType.String) => "string"
          case Some(JmesType.Array) => "array"
          case Some(JmesType.Object) => "object"
          case Some(JmesType.Boolean) => "boolean"
          case Some(JmesType.Expref) => "expref"
          case Some(JmesType.Null) => "null"
          case _ => undefined
    ),
    "keys" -> Function(Seq(param(JmesType.Object)), (_, args) => keys(args(0))),
    "values" -> Function(Seq(param(JmesType.Object)), (_, args) => objValues(args(0))),
    "sort" -> Function(
      Seq(param(JmesType.ArrayString, JmesType.ArrayNumber)),
      // The engine's default sort: elements compared as strings, numbers too.
      (_, args) => args.array(0).jsSlice().sort()
    ),
    "sort_by" -> Function(
      Seq(param(JmesType.Array), param(JmesType.Expref)),
      (runtime, args) => runtime.sortBy(args.array(0), args.node(1))
    ),
    "join" -> Function(
      Seq(param(JmesType.String), param(JmesType.ArrayString)),
      (_, args) => args.array(1).map(_.asInstanceOf[String]).mkString(args.string(0))
    ),
    "reverse" -> Function(
      Seq(param(JmesType.String, JmesType.Array)),
      // Code unit by code unit, as jmespath does: surrogate pairs come out reversed.
      (_, args) =>
        if isString(args(0)) then
          val text = args.string(0)
          val reversed = new StringBuilder(text.length)
          for i <- text.length - 1 to 0 by -1 do reversed += text.charAt(i)
          reversed.result()
        else args.array(0).reverse
    ),
    "to_array" -> Function(
      Seq(param(JmesType.Any)),
      (_, args) => if typeName(args(0)).contains(JmesType.Array) then args(0) else js.Array(args(0))
    ),
    "to_string" -> Function(
      Seq(param(JmesType.Any)),
      (_, args) => if typeName(args(0)).contains(JmesType.String) then args(0) else js.JSON.stringify(args(0))
    ),
    "to_number" -> Function(
      Seq(param(JmesType.Any)),
      (_, args) =>
        typeName(args(0)) match
          case Some(JmesType.Number) => args(0)
          case Some(JmesType.String) =>
            val convertedValue = Globals.Number(args(0))
            if convertedValue.isNaN then null else convertedValue
          case _ => null
    ),
    "not_null" -> Function(
      Seq(Parameter(Seq(JmesType.Any), variadic = true)),
      // Undefined is not null here: `not_null(max_by(`[]`, &a))` is undefined.
      (_, args) => args.find(!typeName(_).contains(JmesType.Null)).getOrElse(null)
    ),
  )

final class Runtime(val interpreter: TreeInterpreter):
  import Runtime.*

  def callFunction(name: js.Any, resolvedArgs: js.Array[js.Any]): js.Any =
    val functionName = stringValue(name)
    val functionEntry = functionName.flatMap(functionTable.get).getOrElse(throw unknownFunction(name))
    validateArgs(functionName.get, resolvedArgs, functionEntry.signature)
    functionEntry.body(this, resolvedArgs)

  private def validateArgs(name: String, args: js.Array[js.Any], signature: Seq[Parameter]): Unit =
    val pluralized = if signature.length == 1 then " argument" else " arguments"
    if signature.last.variadic then
      if args.length < signature.length then
        throw argumentError(s"$name() takes at least ${signature.length}$pluralized but received ${args.length}")
    else if args.length != signature.length then
      throw argumentError(s"$name() takes ${signature.length}$pluralized but received ${args.length}")
    for (parameter, i) <- signature.zipWithIndex do
      val actualType = typeName(args(i))
      if !parameter.types.exists(typeMatches(actualType, _, args(i))) then
        val expected = parameter.types.map(_.label).mkString(",")
        throw typeError(s"$name() expected argument ${i + 1} to be type $expected but received type ${actualType.map(_.label)} instead.")

  private def typeMatches(actual: Option[JmesType], expected: JmesType, argValue: js.Any): Boolean = expected match
    case JmesType.Any => true
    case JmesType.Array => actual.contains(JmesType.Array)
    case JmesType.ArrayNumber | JmesType.ArrayString =>
      val subtype = if expected == JmesType.ArrayNumber then JmesType.Number else JmesType.String
      actual.contains(JmesType.Array) &&
      argValue.asInstanceOf[js.Array[js.Any]].forall(element => typeMatches(typeName(element), subtype, element))
    case _ => actual.contains(expected)

  /** `sort_by`: decorate with the index, sort with the engine's `Array.prototype.sort`, and check each
    * key's type inside the comparator, as jmespath does. With NaN keys the comparator is
    * inconsistent, and the engine's algorithm decides the order.
    */
  def sortBy(array: js.Array[js.Any], exprefNode: Node): js.Array[js.Any] =
    val sortedArray = array.jsSlice()
    if sortedArray.length == 0 then return sortedArray
    val requiredType = typeName(interpreter.visit(exprefNode, sortedArray(0)))
    if !requiredType.exists(t => t == JmesType.Number || t == JmesType.String) then throw typeError("sort_by")
    val decorated = js.Array[Int]((0 until sortedArray.length)*)
    decorated.sort { (a: Int, b: Int) =>
      val exprA = interpreter.visit(exprefNode, sortedArray(a))
      val exprB = interpreter.visit(exprefNode, sortedArray(b))
      if typeName(exprA) != requiredType then throw typeError(s"expected $requiredType, received ${typeName(exprA)}")
      else if typeName(exprB) != requiredType then throw typeError(s"expected $requiredType, received ${typeName(exprB)}")
      if jsGreater(exprA, exprB) then 1 else if jsLess(exprA, exprB) then -1 else a - b
    }
    decorated.map(sortedArray(_))

  def createKeyFunction(exprefNode: Node, allowedTypes: Seq[JmesType]): js.Any => js.Any = x =>
    val current = interpreter.visit(exprefNode, x)
    if !typeName(current).exists(allowedTypes.contains) then
      throw typeError(s"expected one of $allowedTypes, received ${typeName(current)}")
    current
