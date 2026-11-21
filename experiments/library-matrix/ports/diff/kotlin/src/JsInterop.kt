// The host facilities the port uses: JavaScript string trimming and whitespace, Object.keys and
// Array.isArray. Kotlin's own trim() and isWhitespace() follow a different whitespace set (they
// include U+001C to U+001F and exclude U+FEFF), and jsdiff relies on ECMAScript's.
package diff

@JsName("Object")
external object JsObject {
    fun keys(value: Any?): Array<String>
}

@JsName("Array")
private external object JsArrayStatic {
    fun isArray(value: Any?): Boolean
}

fun Any?.isJsArray(): Boolean = JsArrayStatic.isArray(this)

/** JavaScript truthiness: `Boolean(value)`. */
@JsName("Boolean")
external fun truthy(value: Any?): Boolean

/** `String.prototype.trim`, which removes ECMAScript white space and line terminators. */
fun String.jsTrim(): String = asDynamic().trim().unsafeCast<String>()

/** The class `\s`: ECMAScript white space and line terminators. */
fun Char.isJsWhitespace(): Boolean = when (this) {
    '\t', '\n', '\u000B', '\u000C', '\r', ' ', ' ', ' ', ' ', ' ', ' ', ' ', '　', '﻿' -> true
    else -> this in ' '..' '
}
