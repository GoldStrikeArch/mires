@file:Suppress("UNREACHABLE_CODE", "UnsafeCastFromDynamic")

private typealias V = Any?
private data class Tok(val kind: String, val value: V)
private typealias Tokens = MutableList<Tok>
private fun tokensBuffer(): Tokens = mutableListOf()
private fun addToken(tokens: Tokens, kind: String, value: V) { tokens.add(Tok(kind, value)) }

@JsName("Object")
private external object HostObject { fun keys(value: V): Array<String> }
@JsName("Array")
private external object HostArray { fun isArray(value: V): Boolean }
@JsName("Number")
private external fun hostNumber(value: V): Double
@JsName("String")
private external fun hostString(value: V): String
@JsName("parseInt")
private external fun parseInteger(value: String): Double

private val undefinedValue: V get() = undefined
private fun nil(): V = null
private fun syntaxError() = IllegalArgumentException("syntax")
private fun str(s: String): V = s
private fun codeAt(s: String, i: Int) = s[i].code
private fun part(s: String, i: Int, j: Int) = s.substring(i,j)
private fun strLen(s: String) = s.length
private fun slash() = "\\"
private fun replaceFirst(s:String, from:String, to:String) = s.replaceFirst(from,to)
private fun decodeJson(s:String): V = JSON.parse<dynamic>(s)
private fun literal(input: String): V {
    val s = input.trimStart().replaceFirst("\\`", "`")
    if (s.startsWith("[") || s.startsWith("{") || s.startsWith("\"") || s in listOf("true","false","null")) return decodeJson(s)
    if (s.firstOrNull()?.let { it == '-' || it in '0'..'9' } == true) {
        try { return decodeJson(s) } catch (_: Throwable) {}
    }
    return decodeJson("\"" + s + "\"")
}
private fun alpha(c: Int): Boolean {
  return (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || c == 95
}
private fun digit(c: Int): Boolean {
  return c >= 48 && c <= 57
}
private fun lex(source: String): Tokens {
  val result: Tokens = tokensBuffer()
  val length: Int = strLen(source)
  var i: Int = 0
  while (i < length) {
    val start: Int = i
    val c: Int = codeAt(source, i)
    i += 1
    var following: Int = 0
    if (i < length) { following = codeAt(source, i) }
    var k: String = ""
    var value: V = undefinedValue
    if (alpha(c)) {
      while (i < length && (alpha(codeAt(source, i)) || digit(codeAt(source, i)))) { i += 1 }
      value = str(part(source, start, i))
      k = "Field"
    } else if (c == 45 || digit(c)) {
      while (i < length && (digit(codeAt(source, i)) || codeAt(source, i) == 45)) { i += 1 }
      value = parseInteger(part(source, start, i))
      k = "Number"
    } else if (c == 34 || c == 39 || c == 96) {
      while (i < length && codeAt(source, i) != c) {
        if (codeAt(source, i) == 92 && i+1 < length && (codeAt(source, i+1) == c || codeAt(source, i+1) == 92)) {
          i += 1
        }
        i += 1
      }
      val inner: String = part(source, start+1, i)
      if (i < length) { i += 1 }
      if (c == 34) {
        value = decodeJson(part(source, start, i))
        k = "Quoted"
      } else if (c == 39) {
        value = str(replaceFirst(inner, slash() + "'", "'"))
        k = "Literal"
      } else {
        value = literal(inner)
        k = "Literal"
      }
    } else if (c == 32 || c == 9 || c == 10) {
      k = "Skip"
    } else {
      val pair: String = doubleToken(c, following)
      if (pair != "") {
        i += 1
        k = pair
      } else {
        k = singleToken(c)
      }
    }
    if (k != "Skip") { addToken(result, k, value) }
  }
  addToken(result, "EOF", undefinedValue)
  return result
}
private fun doubleToken(c: Int, next: Int): String {
  if (c == 91 && next == 63) { return "Filter" }
  if (c == 91 && next == 93) { return "Flatten" }
  if (c == 38 && next == 38) { return "And" }
  if (c == 124 && next == 124) { return "Or" }
  if (c == 33 && next == 61) { return "NE" }
  if (c == 61 && next == 61) { return "EQ" }
  if (c == 60 && next == 61) { return "LTE" }
  if (c == 62 && next == 61) { return "GTE" }
  return ""
}
private fun singleToken(c: Int): String {
  if (c == 60) { return "LT" }
  if (c == 62) { return "GT" }
  if (c == 33) { return "Not" }
  if (c == 38) { return "Expref" }
  if (c == 124) { return "Pipe" }
  if (c == 91) { return "Lbracket" }
  if (c == 93) { return "Rbracket" }
  if (c == 123) { return "Lbrace" }
  if (c == 125) { return "Rbrace" }
  if (c == 40) { return "Lparen" }
  if (c == 41) { return "Rparen" }
  if (c == 42) { return "Star" }
  if (c == 64) { return "Current" }
  if (c == 44) { return "Comma" }
  if (c == 46) { return "Dot" }
  if (c == 58) { return "Colon" }
  throw syntaxError()
}
private class Parser(source: String) {
  private val tokens = lex(source)
  private var index = 0
  private fun tokenAt(i: Int) = tokens.getOrElse(i) { Tok("EOF", undefinedValue) }
  fun current(offset: Int): String {
    return tokenAt(index + offset).kind
  }
  fun next(): Tok {
    val result: Tok = tokenAt(index)
    if (result.kind == "EOF") {
      throw syntaxError()
    }
    index += 1
    return result
  }
  fun take(kind: String): Unit {
    if (next().kind != kind) {
      throw syntaxError()
    }
  }
  fun power(k: String): Int {
    if (k == "Pipe") { return 1 }
    if (k == "Or") { return 2 }
    if (k == "And") { return 3 }
    if (k == "EQ" || k == "NE" || k == "GT" || k == "LT" || k == "GTE" || k == "LTE") { return 5 }
    if (k == "Flatten") { return 9 }
    if (k == "Star") { return 20 }
    if (k == "Filter") { return 21 }
    if (k == "Dot") { return 40 }
    if (k == "Not") { return 45 }
    if (k == "Lbrace") { return 50 }
    if (k == "Lbracket") { return 55 }
    if (k == "Lparen") { return 60 }
    return 0
  }
  fun parse(): V {
    val ast: V = expression(0)
    if (current(0) != "EOF") { throw syntaxError() }
    return ast
  }
  fun expression(rbp: Int): V {
    var left: V = applyPrefix(next())
    while (power(current(0)) > rbp) {
      val op: String = next().kind
      left = applyInfix(op, left)
    }
    return left
  }
  fun projection(rbp: Int): V {
    val k: String = current(0)
    if (power(k) < 10) { return node("Identity") }
    if (k == "Lbracket" || k == "Filter") { return expression(rbp) }
    if (k == "Dot") {
      next()
      return dot(rbp)
    }
    throw syntaxError()
  }
  fun dot(rbp: Int): V {
    val k: String = current(0)
    if (k == "Field" || k == "Quoted" || k == "Star") { return expression(rbp) }
    if (k == "Lbracket") {
      next()
      return listExpression()
    }
    if (k == "Lbrace") {
      next()
      return hashExpression()
    }
    return undefinedValue
  }
  fun listExpression(): V {
    val items: V = arr()
    while (current(0) != "Rbracket") {
      append(items, expression(0))
      if (current(0) == "Comma") {
        next()
        if (current(0) == "Rbracket") { throw syntaxError() }
      }
    }
    take("Rbracket")
    return branch("MultiSelectList", items)
  }
  fun hashExpression(): V {
    val items: V = arr()
    while (true) {
      val key: Tok = next()
      if (key.kind != "Field" && key.kind != "Quoted") { throw syntaxError() }
      take("Colon")
      val pair: V = named("KeyValuePair", key.value)
      put(pair, "value", expression(0))
      append(items, pair)
      if (current(0) == "Comma") {
        next()
      } else if (current(0) == "Rbrace") {
        next()
        return branch("MultiSelectHash", items)
      }
    }
    throw syntaxError()
  }
  fun bracket(left: V): V {
    if (current(0) == "Colon" || current(1) == "Colon") {
      val parts: V = three(nil(), nil(), nil())
      var i: Int = 0
      while (current(0) != "Rbracket" && i < 3) {
        val token: Tok = next()
        if (token.kind == "Colon") {
          i += 1
        } else if (token.kind == "Number") {
          setIndex(parts, i, token.value)
        } else {
          throw syntaxError()
        }
      }
      take("Rbracket")
      val ix: V = branch("IndexExpression", two(left, branch("Slice", parts)))
      return branch("Projection", two(ix, projection(20)))
    }
    val token: Tok = next()
    take("Rbracket")
    return branch("IndexExpression", two(left, leaf("Index", token.value)))
  }
  fun arguments(commas: Boolean): V {
    val args: V = arr()
    while (current(0) != "Rparen") {
      if (current(0) == "Current") {
        next()
        append(args, node("Current"))
      } else {
        append(args, expression(0))
      }
      if (commas && current(0) == "Comma") { next() }
    }
    take("Rparen")
    return args
  }
  fun applyPrefix(t: Tok): V {
    val k: String = t.kind
    if (k == "Literal") { return leaf("Literal", t.value) }
    if (k == "Quoted" && current(0) == "Lparen") { throw syntaxError() }
    if (k == "Quoted" || k == "Field") { return named("Field", t.value) }
    if (k == "Not") { return branch("NotExpression", one(expression(45))) }
    if (k == "Current") { return node("Current") }
    if (k == "Expref") { return branch("ExpressionReference", one(expression(0))) }
    if (k == "Star") { return branch("ValueProjection", two(node("Identity"), projection(20))) }
    if (k == "Filter" || k == "Flatten") { return applyInfix(k, node("Identity")) }
    if (k == "Lbrace") { return hashExpression() }
    if (k == "Lbracket") {
      if (current(0) == "Colon" || current(0) == "Number") { return bracket(node("Identity")) }
      if (current(0) == "Star" && current(1) == "Rbracket") {
        index += 2
        return branch("Projection", two(node("Identity"), projection(20)))
      }
      return listExpression()
    }
    if (k == "Lparen") { return at(arguments(false), 0) }
    throw syntaxError()
  }
  fun applyInfix(k: String, left: V): V {
    if (k == "Dot") {
      if (current(0) == "Star") {
        next()
        return branch("ValueProjection", two(left, projection(40)))
      }
      return branch("Subexpression", two(left, dot(40)))
    }
    if (k == "Or" || k == "And") { return branch(k + "Expression", two(left, expression(power(k)))) }
    if (k == "Pipe") { return branch(k, two(left, expression(1))) }
    if (k == "Lparen") {
      val f: V = named("Function", field(left, "name"))
      put(f, "children", arguments(true))
      return f
    }
    if (k == "Filter") {
      val condition: V = expression(0)
      take("Rbracket")
      var rhs: V = node("Identity")
      if (current(0) != "Flatten") { rhs = projection(21) }
      return branch("FilterProjection", three(left, rhs, condition))
    }
    if (k == "Flatten") { return branch("Projection", two(branch("Flatten", one(left)), projection(9))) }
    if (power(k) == 5) {
      val cmp: V = named("Comparator", str(k))
      put(cmp, "children", two(left, expression(5)))
      return cmp
    }
    if (k == "Lbracket") {
      if (current(0) == "Colon" || current(0) == "Number") { return bracket(left) }
      take("Star")
      take("Rbracket")
      return branch("Projection", two(left, projection(20)))
    }
    throw syntaxError()
  }
}
private class QueryError(val category: String) : RuntimeException(category)
private fun invalid(category: String) = QueryError(category)
private fun arr(): V = emptyArray<Any?>()
private fun obj(): V = kotlin.js.json()
private fun missing(v: V) = jsTypeOf(v) == "undefined"
private fun isNil(v: V) = v == null && !missing(v)
private fun flag(v: V) = v.unsafeCast<Boolean>()
private fun boolValue(v: Boolean): V = v
private fun intValue(v: Int): V = v
private fun numeric(v: Double): V = v
private fun infinity(positive: Boolean): V = if (positive) Double.POSITIVE_INFINITY else Double.NEGATIVE_INFINITY
private fun nan(v: Double) = v.isNaN()
private fun stringValue(v: V) = hostString(v)
private fun integer(v: V) = hostNumber(v).toInt()
private fun toNumber(v: V) = hostNumber(v)
private fun same(a: V,b: V) = a === b
private fun clamp(i: Int, lo: Int, hi: Int) = i.coerceIn(lo,hi)
private fun field(v: V,k: String): V = v.asDynamic()[k]
private fun at(v: V,i: Int): V = v.asDynamic()[i]
private fun size(v: V): Int = v.asDynamic().length
private fun put(v: V,k: String,x: V) { v.asDynamic()[k] = x }
private fun append(v: V,x: V) { v.asDynamic().push(x) }
private fun setIndex(v: V,i: Int,x: V) { v.asDynamic()[i] = x }
private fun keys(v: V): V = HostObject.keys(v)
private fun kind(v: V): String = when {
    missing(v) -> "undefined"
    isNil(v) -> "null"
    HostArray.isArray(v) -> "array"
    jsTypeOf(v) == "object" && field(v,"jmespathType") == "Expref" -> "expref"
    else -> jsTypeOf(v)
}
private fun compare(a: V,b: V): Int {
    val x = a.asDynamic()
    val y = b.asDynamic()
    return when { x < y -> -1; x > y -> 1; x <= y -> 0; else -> 2 }
}
private fun localeOrder(a: V,b: V): Int = a.asDynamic().localeCompare(b)
private fun jsonText(a: V): V = JSON.stringify(a)
private fun sortByExpression(a: V,b: V): V {
    val entries = (0 until size(a)).map { val v=at(a,it); Pair(evaluate(b,v),v) }
    val t = entries.firstOrNull()?.let { kind(it.first) } ?: return arr()
    if ((t!="number" && t!="string") || entries.any { kind(it.first)!=t }) throw invalid("invalid-type")
    return entries.sortedWith(Comparator { x,y -> compare(x.first,y.first) }).map { it.second }.toTypedArray()
}
private fun primitiveFunction(name:String,a:V,b:V): V {
    val x = a.asDynamic()
    return when(name) {
        "abs" -> kotlin.math.abs(toNumber(a))
        "ceil" -> kotlin.math.ceil(toNumber(a))
        "floor" -> kotlin.math.floor(toNumber(a))
        "contains" -> x.indexOf(b) >= 0
        "starts_with" -> x.lastIndexOf(b) == 0
        "ends_with" -> x.indexOf(b,size(a)-size(b)) >= 0
        "join" -> b.asDynamic().join(a)
        "sort" -> x.slice().sort()
        "reverse" -> if (kind(a)=="string") buildString {
            val original = stringValue(a); for (i in original.indices.reversed()) append(original[i])
        } else x.slice().reverse()
        else -> throw invalid("unknown-function")
    }
}
private fun one(a: V): V {
  val xs: V = arr()
  append(xs, a)
  return xs
}
private fun two(a: V, b: V): V {
  val xs: V = one(a)
  append(xs, b)
  return xs
}
private fun three(a: V, b: V, c: V): V {
  val xs: V = two(a, b)
  append(xs, c)
  return xs
}
private fun node(t: String): V {
  val n: V = obj()
  put(n, "type", str(t))
  return n
}
private fun named(t: String, name: V): V {
  val n: V = node(t)
  put(n, "name", name)
  return n
}
private fun leaf(t: String, value: V): V {
  val n: V = node(t)
  put(n, "value", value)
  return n
}
private fun branch(t: String, xs: V): V {
  val n: V = node(t)
  put(n, "children", xs)
  return n
}
private fun child(n: V, i: Int): V {
  return at(field(n, "children"), i)
}
private fun truth(v: V): Boolean {
  val t: String = kind(v)
  if (t == "null") { return false }
  if (t == "boolean") { return flag(v) }
  if (t == "array" || t == "string") { return size(v) != 0 }
  if (t == "object" || t == "expref") { return size(keys(v)) != 0 }
  return true
}
private fun equal(a: V, b: V): Boolean {
  if (same(a, b)) { return true }
  val t: String = kind(a)
  if (t != kind(b)) { return false }
  if (t == "array") {
    if (size(a) != size(b)) { return false }
    var i: Int = 0
    while (i < size(a)) {
      if (!equal(at(a, i), at(b, i))) { return false }
      i += 1
    }
    return true
  }
  if (t == "object") {
    val ks: V = keys(a)
    if (size(ks) != size(keys(b))) { return false }
    var i: Int = 0
    while (i < size(ks)) {
      val key: String = stringValue(at(ks, i))
      if (!equal(field(a, key), field(b, key))) { return false }
      i += 1
    }
    return true
  }
  return false
}
private fun cap(v: V, defaultValue: Int, step: Int, length: Int): Int {
  if (isNil(v)) { return defaultValue }
  var y: Int = integer(v)
  if (y < 0) { y += length }
  if (step < 0) { return clamp(y, -1, length-1) }
  return clamp(y, 0, length)
}
private fun evaluate(n: V, v: V): V {
  val k: String = stringValue(field(n, "type"))
  if (k == "Field") {
    if (kind(v) != "object" && kind(v) != "expref") { return nil() }
    val result: V = field(v, stringValue(field(n, "name")))
    if (missing(result)) { return nil() }
    return result
  }
  if (k == "Current" || k == "Identity") { return v }
  if (k == "Literal") { return field(n, "value") }
  if (k == "Pipe" || k == "Subexpression" || k == "IndexExpression") {
    return evaluate(child(n, 1), evaluate(child(n, 0), v))
  }
  if (k == "Index") {
    if (kind(v) != "array") { return nil() }
    val ix: V = field(n, "value")
    if (nan(toNumber(ix))) { return nil() }
    var i: Int = integer(ix)
    if (i < 0) { i += size(v) }
    if (i < 0 || i >= size(v)) { return nil() }
    return at(v, i)
  }
  if (k == "Slice") {
    if (kind(v) != "array") { return nil() }
    val p: V = child(n, 2)
    var step: Int = 1
    if (!isNil(p)) { step = integer(p) }
    if (step == 0) { throw invalid("invalid-value") }
    val end: Int = size(v)
    var startDefault: Int = 0
    var stopDefault: Int = end
    if (step < 0) {
      startDefault = end-1
      stopDefault = -1
    }
    var i: Int = cap(child(n, 0), startDefault, step, end)
    val stop: Int = cap(child(n, 1), stopDefault, step, end)
    val out: V = arr()
    while ((step > 0 && i < stop) || (step < 0 && i > stop)) {
      append(out, at(v, i))
      i += step
    }
    return out
  }
  if (k == "Projection" || k == "ValueProjection" || k == "FilterProjection" || k == "Flatten") {
    var base: V = evaluate(child(n, 0), v)
    if (k == "ValueProjection") {
      if (kind(base) != "object" && kind(base) != "expref") { return nil() }
      base = objectValues(base)
    }
    if (kind(base) != "array") { return nil() }
    val out: V = arr()
    var i: Int = 0
    while (i < size(base)) {
      val item: V = at(base, i)
      if (k == "Flatten") {
        if (kind(item) == "array") {
          var j: Int = 0
          while (j < size(item)) {
            append(out, at(item, j))
            j += 1
          }
        } else {
          append(out, item)
        }
      } else if (k != "FilterProjection" || truth(evaluate(child(n, 2), item))) {
        val x: V = evaluate(child(n, 1), item)
        if (!isNil(x)) { append(out, x) }
      }
      i += 1
    }
    return out
  }
  if (k == "Comparator") {
    val a: V = evaluate(child(n, 0), v)
    val b: V = evaluate(child(n, 1), v)
    val op: String = stringValue(field(n, "name"))
    if (op == "EQ") { return boolValue(equal(a, b)) }
    if (op == "NE") { return boolValue(!equal(a, b)) }
    val ord: Int = compare(a, b)
    return boolValue((ord == -1 && (op == "LT" || op == "LTE")) || (ord == 1 && (op == "GT" || op == "GTE")) || (ord == 0 && (op == "LTE" || op == "GTE")))
  }
  if (k == "NotExpression") { return boolValue(!truth(evaluate(child(n, 0), v))) }
  if (k == "AndExpression" || k == "OrExpression") {
    val left: V = evaluate(child(n, 0), v)
    if (truth(left) == (k == "AndExpression")) { return evaluate(child(n, 1), v) }
    return left
  }
  if (k == "ExpressionReference") {
    val result: V = child(n, 0)
    put(result, "jmespathType", str("Expref"))
    return result
  }
  val cs: V = field(n, "children")
  if (k == "Function") {
    val args: V = arr()
    var i: Int = 0
    while (i < size(cs)) {
      append(args, evaluate(at(cs, i), v))
      i += 1
    }
    return callFunction(stringValue(field(n, "name")), args)
  }
  if (k == "MultiSelectList" || k == "MultiSelectHash") {
    if (isNil(v)) { return nil() }
    var out: V = arr()
    if (k == "MultiSelectHash") { out = obj() }
    var i: Int = 0
    while (i < size(cs)) {
      val c: V = at(cs, i)
      if (k == "MultiSelectHash") {
        put(out, stringValue(field(c, "name")), evaluate(field(c, "value"), v))
      } else {
        append(out, evaluate(c, v))
      }
      i += 1
    }
    return out
  }
  throw syntaxError()
}
private fun objectValues(v: V): V {
  val ks: V = keys(v)
  val result: V = arr()
  var i: Int = 0
  while (i < size(ks)) {
    append(result, field(v, stringValue(at(ks, i))))
    i += 1
  }
  return result
}
private fun matchesType(v: V, t: String): Boolean {
  if (t == "any") { return true }
  if (t == "numbers" || t == "strings") {
    if (kind(v) != "array") { return false }
    var i: Int = 0
    while (i < size(v)) {
      if (kind(at(v, i)) + "s" != t) { return false }
      i += 1
    }
    return true
  }
  return kind(v) == t
}
private fun callFunction(name: String, args: V): V {
  checkArguments(name, args)
  val a: V = at(args, 0)
  val b: V = at(args, 1)
  if (name == "type") { return str(kind(a)) }
  if (name == "length") {
    if (kind(a) == "object") { return intValue(size(keys(a))) }
    return intValue(size(a))
  }
  if (name == "keys") { return keys(a) }
  if (name == "values") { return objectValues(a) }
  if (name == "to_array") {
    if (kind(a) == "array") { return a }
    return one(a)
  }
  if (name == "to_number") {
    if (kind(a) == "number") { return a }
    if (kind(a) == "string" && !nan(toNumber(a))) { return numeric(toNumber(a)) }
    return nil()
  }
  if (name == "to_string") {
    if (kind(a) == "string") { return a }
    return jsonText(a)
  }
  if (name == "not_null") {
    var i: Int = 0
    while (i < size(args)) {
      if (!isNil(at(args, i))) { return at(args, i) }
      i += 1
    }
    return nil()
  }
  if (name == "sum" || name == "avg") {
    var sum: Double = 0.0
    var i: Int = 0
    while (i < size(a)) {
      sum += toNumber(at(a, i))
      i += 1
    }
    if (name == "avg") { return numeric(sum / size(a)) }
    return numeric(sum)
  }
  if (name == "min" || name == "max") {
    if (size(a) == 0) { return nil() }
    var best: V = at(a, 0)
    var i: Int = 1
    while (i < size(a)) {
      val item: V = at(a, i)
      var ord: Int = compare(item, best)
      if (kind(item) == "string") { ord = localeOrder(item, best) }
      if ((name == "min" && ord < 0) || (name == "max" && ord > 0)) { best = item }
      i += 1
    }
    return best
  }
  if (name == "max_by" || name == "min_by") {
    var extreme: V = infinity(name == "min_by")
    var result: V = undefinedValue
    var i: Int = 0
    while (i < size(a)) {
      val item: V = at(a, i)
      val key: V = evaluate(b, item)
      val t: String = kind(key)
      if (t != "number" && t != "string") { throw invalid("invalid-type") }
      val ord: Int = compare(key, extreme)
      if ((ord == -1 && name == "min_by") || (ord == 1 && name == "max_by")) {
        extreme = key
        result = item
      }
      i += 1
    }
    return result
  }
  if (name == "merge") {
    val out: V = obj()
    var i: Int = 0
    while (i < size(args)) {
      val item: V = at(args, i)
      if (!isNil(item)) {
        val ks: V = keys(item)
        var j: Int = 0
        while (j < size(ks)) {
          val key: String = stringValue(at(ks, j))
          put(out, key, field(item, key))
          j += 1
        }
      }
      i += 1
    }
    return out
  }
  if (name == "map") {
    val out: V = arr()
    var i: Int = 0
    while (i < size(b)) {
      append(out, evaluate(a, at(b, i)))
      i += 1
    }
    return out
  }
  if (name == "sort_by") {
    return sortByExpression(a, b)
  }
  return primitiveFunction(name, a, b)
}
private fun checkArguments(name: String, args: V): Unit {
  if (name == "abs" || name == "ceil" || name == "floor") {
    if (size(args) < 1 || (size(args) > 1 && name != "merge" && name != "not_null")) { throw invalid("invalid-arity") }
    if (!(matchesType(at(args, 0), "number"))) { throw invalid("invalid-type") }
    return
  }
  if (name == "avg" || name == "sum") {
    if (size(args) < 1 || (size(args) > 1 && name != "merge" && name != "not_null")) { throw invalid("invalid-arity") }
    if (!(matchesType(at(args, 0), "numbers"))) { throw invalid("invalid-type") }
    return
  }
  if (name == "contains") {
    if (size(args) < 2 || (size(args) > 2 && name != "merge" && name != "not_null")) { throw invalid("invalid-arity") }
    if (!(matchesType(at(args, 0), "string") || matchesType(at(args, 0), "array"))) { throw invalid("invalid-type") }
    if (!(matchesType(at(args, 1), "any"))) { throw invalid("invalid-type") }
    return
  }
  if (name == "starts_with" || name == "ends_with") {
    if (size(args) < 2 || (size(args) > 2 && name != "merge" && name != "not_null")) { throw invalid("invalid-arity") }
    if (!(matchesType(at(args, 0), "string"))) { throw invalid("invalid-type") }
    if (!(matchesType(at(args, 1), "string"))) { throw invalid("invalid-type") }
    return
  }
  if (name == "length") {
    if (size(args) < 1 || (size(args) > 1 && name != "merge" && name != "not_null")) { throw invalid("invalid-arity") }
    if (!(matchesType(at(args, 0), "object") || matchesType(at(args, 0), "string") || matchesType(at(args, 0), "array"))) { throw invalid("invalid-type") }
    return
  }
  if (name == "map") {
    if (size(args) < 2 || (size(args) > 2 && name != "merge" && name != "not_null")) { throw invalid("invalid-arity") }
    if (!(matchesType(at(args, 0), "expref"))) { throw invalid("invalid-type") }
    if (!(matchesType(at(args, 1), "array"))) { throw invalid("invalid-type") }
    return
  }
  if (name == "max" || name == "min" || name == "sort") {
    if (size(args) < 1 || (size(args) > 1 && name != "merge" && name != "not_null")) { throw invalid("invalid-arity") }
    if (!(matchesType(at(args, 0), "numbers") || matchesType(at(args, 0), "strings"))) { throw invalid("invalid-type") }
    return
  }
  if (name == "keys" || name == "values" || name == "merge") {
    if (size(args) < 1 || (size(args) > 1 && name != "merge" && name != "not_null")) { throw invalid("invalid-arity") }
    if (!(matchesType(at(args, 0), "object"))) { throw invalid("invalid-type") }
    return
  }
  if (name == "max_by" || name == "min_by" || name == "sort_by") {
    if (size(args) < 2 || (size(args) > 2 && name != "merge" && name != "not_null")) { throw invalid("invalid-arity") }
    if (!(matchesType(at(args, 0), "array"))) { throw invalid("invalid-type") }
    if (!(matchesType(at(args, 1), "expref"))) { throw invalid("invalid-type") }
    return
  }
  if (name == "join") {
    if (size(args) < 2 || (size(args) > 2 && name != "merge" && name != "not_null")) { throw invalid("invalid-arity") }
    if (!(matchesType(at(args, 0), "string"))) { throw invalid("invalid-type") }
    if (!(matchesType(at(args, 1), "strings"))) { throw invalid("invalid-type") }
    return
  }
  if (name == "reverse") {
    if (size(args) < 1 || (size(args) > 1 && name != "merge" && name != "not_null")) { throw invalid("invalid-arity") }
    if (!(matchesType(at(args, 0), "array") || matchesType(at(args, 0), "string"))) { throw invalid("invalid-type") }
    return
  }
  if (name == "to_array" || name == "to_number" || name == "to_string" || name == "type" || name == "not_null") {
    if (size(args) < 1 || (size(args) > 1 && name != "merge" && name != "not_null")) { throw invalid("invalid-arity") }
    if (!(matchesType(at(args, 0), "any"))) { throw invalid("invalid-type") }
    return
  }
  throw invalid("unknown-function")
}
external interface QueryInput { val data: Any?; val queries: Array<Query> }
external interface Query { val op: String; val expression: String }

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: QueryInput): Array<Any?> = input.queries.map { q ->
    val out = obj()
    try {
        val ast = Parser(q.expression).parse()
        put(out,"value", if (q.op == "compile") ast else evaluate(ast,input.data))
    } catch (e: QueryError) {
        put(out,"error",e.category)
    } catch (_: Throwable) {
        put(out,"error","syntax")
    }
    out
}.toTypedArray()
