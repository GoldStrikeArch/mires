// The JavaScript values jmespath works on, and the few host functions it calls.
//
// jmespath 0.16.0 evaluates over plain JS values (the JSON document, objects it builds, and its own
// AST nodes, which `&expr` hands to functions and to the caller) and leans on JS semantics for them:
// `Object.keys` order, `===` identity in `contains`, relational operators with type coercion,
// `JSON.parse`/`JSON.stringify`, `parseInt`, `Number(string)`. The port keeps those values as they
// are and reads them through typed facades, so every one of those semantics is the engine's own.

package jmespath

import kotlin.js.Json

/** An AST node as jmespath builds it: a plain object whose properties depend on [type]. */
internal external interface Node {
    val type: String
    val name: Any?
    val value: Any?
    val children: Array<Node?>
    var jmespathType: String?
}

@JsName("Object")
internal external object JsObject {
    fun keys(value: Any): Array<String>
}

@JsName("Array")
internal external object JsArray {
    fun isArray(value: Any?): Boolean
}

/** `Number(value)`: the unary `+` that `to_number` applies to strings. */
@JsName("Number")
internal external fun jsNumber(value: Any?): Double

/** `String(value)`: the conversion `String.prototype.indexOf` applies to its argument. */
@JsName("String")
internal external fun jsString(value: Any?): String

@JsName("parseInt")
internal external fun parseInt(text: String): Double

internal fun isUndefined(value: Any?) = jsTypeOf(value) == "undefined"

internal fun isNull(value: Any?) = value == null && jsTypeOf(value) == "object"

internal fun isArray(value: Any?) = JsArray.isArray(value)

/** jmespath's `isObject`: `Object.prototype.toString` is `[object Object]`, which for these values
 *  means a non-null, non-array object (JSON objects, built objects and AST nodes alike). */
internal fun isObject(value: Any?) = jsTypeOf(value) == "object" && value != null && !isArray(value)

internal fun Any.property(key: Any?): Any? = asDynamic()[key]

/** `array[index]` for a JS number index: an element, or undefined off the end, at NaN and at ±∞. */
internal fun Array<Any?>.elementAt(index: Double): Any? =
    if (index >= 0 && index < size && index == kotlin.math.floor(index)) this[index.toInt()] else undefined

internal fun parseJson(text: String): Any? = JSON.parse(text)

/** The relational operators of jmespath's comparators and `max_by`/`sort_by`, with JS coercion. */
internal fun jsGreater(a: Any?, b: Any?): Boolean = a.asDynamic() > b
internal fun jsGreaterOrEqual(a: Any?, b: Any?): Boolean = a.asDynamic() >= b
internal fun jsLess(a: Any?, b: Any?): Boolean = a.asDynamic() < b
internal fun jsLessOrEqual(a: Any?, b: Any?): Boolean = a.asDynamic() <= b

internal fun String.localeCompare(other: String): Int = asDynamic().localeCompare(other).unsafeCast<Int>()

internal fun Json.asNode(): Node = unsafeCast<Node>()
