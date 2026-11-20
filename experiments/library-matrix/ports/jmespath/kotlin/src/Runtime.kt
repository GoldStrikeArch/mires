// jmespath's Runtime: the function table, argument validation and the 26 built-in functions.

package jmespath

import kotlin.js.json
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

/** jmespath's TYPE_* constants, with the names its error messages use. */
internal enum class JmesType(val label: String) {
    Number("number"),
    Any("any"),
    String("string"),
    Array("array"),
    Object("object"),
    Boolean("boolean"),
    Expref("expression"),
    Null("null"),
    ArrayNumber("Array<number>"),
    ArrayString("Array<string>"),
}

/** jmespath's `_getTypeName`; null where its switch falls through (undefined). */
internal fun typeName(value: Any?): JmesType? = when (jsTypeOf(value)) {
    "string" -> JmesType.String
    "number" -> JmesType.Number
    "boolean" -> JmesType.Boolean
    "object" -> when {
        value == null -> JmesType.Null
        isArray(value) -> JmesType.Array
        value.unsafeCast<Node>().jmespathType == "Expref" -> JmesType.Expref
        else -> JmesType.Object
    }
    else -> null
}

private class Parameter(vararg val types: JmesType, val variadic: Boolean = false)

private class Function(val signature: List<Parameter>, val body: Runtime.(List<Any?>) -> Any?)

private fun function(vararg signature: Parameter, body: Runtime.(List<Any?>) -> Any?) = Function(signature.toList(), body)

private fun List<Any?>.array(i: Int) = this[i].unsafeCast<Array<Any?>>()
private fun List<Any?>.number(i: Int) = this[i].unsafeCast<Double>()
private fun List<Any?>.string(i: Int) = this[i].unsafeCast<String>()
private fun List<Any?>.node(i: Int) = this[i].unsafeCast<Node>()

private val functionTable: Map<String, Function> = mapOf(
    "abs" to function(Parameter(JmesType.Number)) { abs(it.number(0)) },
    "avg" to function(Parameter(JmesType.ArrayNumber)) { args ->
        val inputArray = args.array(0)
        var sum = 0.0
        for (x in inputArray) sum += x.unsafeCast<Double>()
        sum / inputArray.size
    },
    "ceil" to function(Parameter(JmesType.Number)) { ceil(it.number(0)) },
    "contains" to function(Parameter(JmesType.String, JmesType.Array), Parameter(JmesType.Any)) { args ->
        // `indexOf`: String.prototype.indexOf converts its argument with String(); the array
        // version compares with `===`, so only the same object is found.
        val subject = args[0]
        if (subject is String) subject.indexOf(jsString(args[1])) >= 0 else args.array(0).any { it === args[1] }
    },
    "ends_with" to function(Parameter(JmesType.String), Parameter(JmesType.String)) { args ->
        val searchStr = args.string(0)
        val suffix = args.string(1)
        searchStr.indexOf(suffix, searchStr.length - suffix.length) != -1
    },
    "floor" to function(Parameter(JmesType.Number)) { floor(it.number(0)) },
    "length" to function(Parameter(JmesType.String, JmesType.Array, JmesType.Object)) { args ->
        when (val subject = args[0]) {
            is String -> subject.length
            else -> if (isArray(subject)) args.array(0).size else JsObject.keys(subject!!).size
        }
    },
    "map" to function(Parameter(JmesType.Expref), Parameter(JmesType.Array)) { args ->
        val exprefNode = args.node(0)
        args.array(1).map { interpreter.visit(exprefNode, it) }.toTypedArray()
    },
    "max" to function(Parameter(JmesType.ArrayNumber, JmesType.ArrayString)) { args ->
        val elements = args.array(0)
        when {
            elements.isEmpty() -> null
            // Math.max.apply: NaN wins, and +0 is larger than -0.
            typeName(elements[0]) == JmesType.Number -> elements.fold(Double.NEGATIVE_INFINITY) { acc, x -> max(acc, x.unsafeCast<Double>()) }
            else -> elements.reduce { maxElement, x -> if (maxElement.unsafeCast<String>().localeCompare(x.unsafeCast<String>()) < 0) x else maxElement }
        }
    },
    "merge" to function(Parameter(JmesType.Object, variadic = true)) { args ->
        // `for (key in current)`: only the first argument is type-checked, so later ones may be
        // strings or arrays (their index keys) or primitives and null (no keys).
        val merged = json()
        for (current in args) {
            if (current == null) continue
            for (key in JsObject.keys(current)) merged[key] = current.property(key)
        }
        merged
    },
    "max_by" to function(Parameter(JmesType.Array), Parameter(JmesType.Expref)) { args ->
        val keyFunction = createKeyFunction(args.node(1), listOf(JmesType.Number, JmesType.String))
        var maxNumber: Any? = Double.NEGATIVE_INFINITY
        var maxRecord: Any? = undefined
        for (element in args.array(0)) {
            val current = keyFunction(element)
            if (jsGreater(current, maxNumber)) {
                maxNumber = current
                maxRecord = element
            }
        }
        maxRecord
    },
    "sum" to function(Parameter(JmesType.ArrayNumber)) { args -> args.array(0).fold(0.0) { sum, x -> sum + x.unsafeCast<Double>() } },
    "starts_with" to function(Parameter(JmesType.String), Parameter(JmesType.String)) { args ->
        // `lastIndexOf(prefix) === 0`, searching from the end: `starts_with('abab', 'ab')` is false.
        val subject = args.string(0)
        subject.lastIndexOf(args.string(1), subject.length) == 0
    },
    "min" to function(Parameter(JmesType.ArrayNumber, JmesType.ArrayString)) { args ->
        val elements = args.array(0)
        when {
            elements.isEmpty() -> null
            typeName(elements[0]) == JmesType.Number -> elements.fold(Double.POSITIVE_INFINITY) { acc, x -> min(acc, x.unsafeCast<Double>()) }
            else -> elements.reduce { minElement, x -> if (x.unsafeCast<String>().localeCompare(minElement.unsafeCast<String>()) < 0) x else minElement }
        }
    },
    "min_by" to function(Parameter(JmesType.Array), Parameter(JmesType.Expref)) { args ->
        val keyFunction = createKeyFunction(args.node(1), listOf(JmesType.Number, JmesType.String))
        var minNumber: Any? = Double.POSITIVE_INFINITY
        var minRecord: Any? = undefined
        for (element in args.array(0)) {
            val current = keyFunction(element)
            if (jsLess(current, minNumber)) {
                minNumber = current
                minRecord = element
            }
        }
        minRecord
    },
    "type" to function(Parameter(JmesType.Any)) { args ->
        when (typeName(args[0])) {
            JmesType.Number -> "number"
            JmesType.String -> "string"
            JmesType.Array -> "array"
            JmesType.Object -> "object"
            JmesType.Boolean -> "boolean"
            JmesType.Expref -> "expref"
            JmesType.Null -> "null"
            else -> undefined
        }
    },
    "keys" to function(Parameter(JmesType.Object)) { JsObject.keys(it[0]!!) },
    "values" to function(Parameter(JmesType.Object)) { objValues(it[0]!!) },
    "sort" to function(Parameter(JmesType.ArrayString, JmesType.ArrayNumber)) { args ->
        // The engine's default sort: elements compared as strings, numbers too.
        args.array(0).copyOf().also { it.asDynamic().sort() }
    },
    "sort_by" to function(Parameter(JmesType.Array), Parameter(JmesType.Expref)) { args -> sortBy(args.array(0), args.node(1)) },
    "join" to function(Parameter(JmesType.String), Parameter(JmesType.ArrayString)) { args -> args.array(1).joinToString(args.string(0)) },
    "reverse" to function(Parameter(JmesType.String, JmesType.Array)) { args ->
        when (val subject = args[0]) {
            // Code unit by code unit, as jmespath does: surrogate pairs come out reversed.
            is String -> buildString { for (i in subject.indices.reversed()) append(subject[i]) }
            else -> args.array(0).reversedArray()
        }
    },
    "to_array" to function(Parameter(JmesType.Any)) { args -> if (typeName(args[0]) == JmesType.Array) args[0] else arrayOf(args[0]) },
    "to_string" to function(Parameter(JmesType.Any)) { args -> if (typeName(args[0]) == JmesType.String) args[0] else JSON.stringify(args[0]) },
    "to_number" to function(Parameter(JmesType.Any)) { args ->
        when (typeName(args[0])) {
            JmesType.Number -> args[0]
            JmesType.String -> jsNumber(args[0]).takeUnless { it.isNaN() }
            else -> null
        }
    },
    "not_null" to function(Parameter(JmesType.Any, variadic = true)) { args ->
        // Undefined is not null here: `not_null(max_by(`[]`, &a))` is undefined.
        args.firstOrNull { typeName(it) != JmesType.Null }
    },
)

internal class Runtime(val interpreter: TreeInterpreter) {
    fun callFunction(name: Any?, resolvedArgs: List<Any?>): Any? {
        val functionEntry = (name as? String)?.let { functionTable[it] } ?: throw unknownFunction(name)
        validateArgs(name, resolvedArgs, functionEntry.signature)
        return functionEntry.body(this, resolvedArgs)
    }

    private fun validateArgs(name: String, args: List<Any?>, signature: List<Parameter>) {
        val pluralized = if (signature.size == 1) " argument" else " arguments"
        if (signature.last().variadic) {
            if (args.size < signature.size) throw argumentError("$name() takes at least ${signature.size}$pluralized but received ${args.size}")
        } else if (args.size != signature.size) {
            throw argumentError("$name() takes ${signature.size}$pluralized but received ${args.size}")
        }
        for ((i, parameter) in signature.withIndex()) {
            val actualType = typeName(args[i])
            if (parameter.types.none { typeMatches(actualType, it, args[i]) }) {
                val expected = parameter.types.joinToString(",") { it.label }
                throw typeError("$name() expected argument ${i + 1} to be type $expected but received type ${actualType?.label} instead.")
            }
        }
    }

    private fun typeMatches(actual: JmesType?, expected: JmesType, argValue: Any?): Boolean = when (expected) {
        JmesType.Any -> true
        JmesType.Array -> actual == JmesType.Array
        JmesType.ArrayNumber, JmesType.ArrayString -> {
            val subtype = if (expected == JmesType.ArrayNumber) JmesType.Number else JmesType.String
            actual == JmesType.Array && argValue.unsafeCast<Array<Any?>>().all { typeMatches(typeName(it), subtype, it) }
        }
        else -> actual == expected
    }

    /** `sort_by`: decorate with the index, sort with the engine's sort (Kotlin/JS `sortWith` calls
     *  `Array.prototype.sort`), and check each key's type inside the comparator, as jmespath does.
     *  With NaN keys the comparator is inconsistent, and the engine's algorithm decides the order. */
    fun sortBy(array: Array<Any?>, exprefNode: Node): Array<Any?> {
        val sortedArray = array.copyOf()
        if (sortedArray.isEmpty()) return sortedArray
        val requiredType = typeName(interpreter.visit(exprefNode, sortedArray[0]))
        if (requiredType != JmesType.Number && requiredType != JmesType.String) throw typeError("sort_by")
        val decorated = sortedArray.mapIndexed { i, element -> IndexedValue(i, element) }.toTypedArray()
        decorated.sortWith { a, b ->
            val exprA = interpreter.visit(exprefNode, a.value)
            val exprB = interpreter.visit(exprefNode, b.value)
            if (typeName(exprA) != requiredType) {
                throw typeError("expected ${requiredType.label}, received ${typeName(exprA)?.label}")
            } else if (typeName(exprB) != requiredType) {
                throw typeError("expected ${requiredType.label}, received ${typeName(exprB)?.label}")
            }
            when {
                jsGreater(exprA, exprB) -> 1
                jsLess(exprA, exprB) -> -1
                else -> a.index - b.index
            }
        }
        for (j in decorated.indices) sortedArray[j] = decorated[j].value
        return sortedArray
    }

    fun createKeyFunction(exprefNode: Node, allowedTypes: List<JmesType>): (Any?) -> Any? = { x ->
        val current = interpreter.visit(exprefNode, x)
        if (typeName(current) !in allowedTypes) throw typeError("expected one of $allowedTypes, received ${typeName(current)?.label}")
        current
    }
}
