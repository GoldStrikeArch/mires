// jmespath's TreeInterpreter and the value helpers it uses (strictDeepEqual, isFalse, objValues).

package jmespath

import kotlin.js.json

/** jmespath's `strictDeepEqual`: `===`, then same `Object.prototype.toString` tag, then by content.
 *  Objects compare every own key of [first] against `second[key]` (so an undefined value equals a
 *  missing key), then require [second]'s own keys to be among them. */
internal fun strictDeepEqual(first: Any?, second: Any?): Boolean {
    if (first === second) return true
    val firstType = typeTag(first)
    if (firstType != typeTag(second)) return false
    if (firstType == "array") {
        val a = first.unsafeCast<Array<Any?>>()
        val b = second.unsafeCast<Array<Any?>>()
        if (a.size != b.size) return false
        return a.indices.all { strictDeepEqual(a[it], b[it]) }
    }
    if (firstType == "object") {
        val keysSeen = mutableSetOf<String>()
        for (key in JsObject.keys(first!!)) {
            if (!strictDeepEqual(first.property(key), second!!.property(key))) return false
            keysSeen += key
        }
        return JsObject.keys(second!!).all { it in keysSeen }
    }
    return false
}

/** The part of `Object.prototype.toString` that strictDeepEqual distinguishes. */
private fun typeTag(value: Any?): String = when {
    isNull(value) -> "null"
    isArray(value) -> "array"
    else -> jsTypeOf(value)
}

/** jmespath's `isFalse`: "", false, null, an empty array and an object with no own keys. */
internal fun isFalse(value: Any?): Boolean = when {
    value == "" || value == false || isNull(value) -> true
    isArray(value) -> value.unsafeCast<Array<Any?>>().isEmpty()
    isObject(value) -> JsObject.keys(value!!).isEmpty()
    else -> false
}

internal fun objValues(obj: Any): Array<Any?> = JsObject.keys(obj).map { obj.property(it) }.toTypedArray()

internal class TreeInterpreter {
    private val runtime = Runtime(this)

    fun search(node: Node?, value: Any?): Any? = visit(node, value)

    fun visit(node: Node?, value: Any?): Any? {
        // `node.type` of a missing node is a TypeError in jmespath.
        if (node == null) throw syntaxError("Cannot read properties of ${node}")
        return when (node.type) {
            "Field" -> if (isObject(value)) value!!.property(node.name).takeUnless(::isUndefined) else null
            "Subexpression" -> visit(node.children[1], visit(node.children[0], value))
            "IndexExpression" -> visit(node.children[1], visit(node.children[0], value))
            "Index" -> {
                if (!isArray(value)) return null
                val array = value.unsafeCast<Array<Any?>>()
                var index = node.value.unsafeCast<Double>()
                if (index < 0) index += array.size
                array.elementAt(index).takeUnless(::isUndefined)
            }
            "Slice" -> if (isArray(value)) slice(value.unsafeCast<Array<Any?>>(), node.children.unsafeCast<Array<Double?>>()) else null
            "Projection" -> {
                val base = visit(node.children[0], value)
                if (!isArray(base)) return null
                collect(base.unsafeCast<Array<Any?>>(), node.children[1])
            }
            "ValueProjection" -> {
                val base = visit(node.children[0], value)
                if (!isObject(base)) return null
                collect(objValues(base!!), node.children[1])
            }
            "FilterProjection" -> {
                val base = visit(node.children[0], value)
                if (!isArray(base)) return null
                // Every condition first, then the projection: the first error is jmespath's.
                val filtered = base.unsafeCast<Array<Any?>>().filter { !isFalse(visit(node.children[2], it)) }
                collect(filtered.toTypedArray(), node.children[1])
            }
            "Comparator" -> {
                val first = visit(node.children[0], value)
                val second = visit(node.children[1], value)
                when (node.name) {
                    "EQ" -> strictDeepEqual(first, second)
                    "NE" -> !strictDeepEqual(first, second)
                    "GT" -> jsGreater(first, second)
                    "GTE" -> jsGreaterOrEqual(first, second)
                    "LT" -> jsLess(first, second)
                    "LTE" -> jsLessOrEqual(first, second)
                    else -> throw syntaxError("Unknown comparator: ${node.name}")
                }
            }
            "Flatten" -> {
                val original = visit(node.children[0], value)
                if (!isArray(original)) return null
                val merged = mutableListOf<Any?>()
                for (current in original.unsafeCast<Array<Any?>>()) {
                    if (isArray(current)) merged.addAll(current.unsafeCast<Array<Any?>>()) else merged.add(current)
                }
                merged.toTypedArray()
            }
            "Identity", "Current" -> value
            "MultiSelectList" -> if (isNull(value)) null else node.children.map { visit(it, value) }.toTypedArray()
            "MultiSelectHash" -> {
                if (isNull(value)) return null
                val collected = json()
                for (child in node.children) collected[child!!.name.unsafeCast<String>()] = visit(child.value.unsafeCast<Node?>(), value)
                collected
            }
            "OrExpression" -> visit(node.children[0], value).let { if (isFalse(it)) visit(node.children[1], value) else it }
            "AndExpression" -> visit(node.children[0], value).let { if (isFalse(it)) it else visit(node.children[1], value) }
            "NotExpression" -> isFalse(visit(node.children[0], value))
            "Literal" -> node.value
            "Pipe" -> visit(node.children[1], visit(node.children[0], value))
            "Function" -> runtime.callFunction(node.name, node.children.map { visit(it, value) })
            "ExpressionReference" -> {
                // jmespath tags the referenced node in place and returns the node itself.
                val refNode = node.children[0] ?: throw syntaxError("Cannot set properties of undefined")
                refNode.jmespathType = "Expref"
                refNode
            }
            else -> throw syntaxError("Unknown node type: ${node.type}")
        }
    }

    /** Projections keep every result except null; undefined (an empty `max_by`) is kept. */
    private fun collect(base: Array<Any?>, right: Node?): Array<Any?> {
        val collected = mutableListOf<Any?>()
        for (element in base) {
            val current = visit(right, element)
            if (!isNull(current)) collected.add(current)
        }
        return collected.toTypedArray()
    }

    /** `Slice` with jmespath's computeSliceParams, in JS numbers: the tokens come from parseInt, so
     *  a bound may be NaN (`[0:3:-]`) or past 2^53, and the loop reads `value[i]` as it goes. */
    private fun slice(array: Array<Any?>, params: Array<Double?>): Array<Any?> {
        val length = array.size.toDouble()
        val step = params[2] ?: 1.0
        if (params[2] != null && step == 0.0) throw runtimeError("Invalid slice, step cannot be 0")
        val stepValueNegative = step < 0
        val start = params[0]?.let { capSliceRange(length, it, step) } ?: if (stepValueNegative) length - 1 else 0.0
        val stop = params[1]?.let { capSliceRange(length, it, step) } ?: if (stepValueNegative) -1.0 else length
        val result = mutableListOf<Any?>()
        var i = start
        if (step > 0) {
            while (i < stop) {
                result.add(array.elementAt(i))
                i += step
            }
        } else {
            while (i > stop) {
                result.add(array.elementAt(i))
                i += step
            }
        }
        return result.toTypedArray()
    }

    private fun capSliceRange(arrayLength: Double, actualValue: Double, step: Double): Double = when {
        actualValue < 0 -> (actualValue + arrayLength).let { if (it < 0) (if (step < 0) -1.0 else 0.0) else it }
        actualValue >= arrayLength -> if (step < 0) arrayLength - 1 else arrayLength
        else -> actualValue
    }
}

