package marked

import kotlin.js.RegExp
import kotlin.js.RegExpMatch

external interface JsString {
    fun trim(): String
    fun trimStart(): String
    fun trimEnd(): String
    fun toLowerCase(): String
    fun toUpperCase(): String
    @JsName("replace")
    fun replaceStr(substr: String, replacement: String): String
    @JsName("replace")
    fun replaceRe(regex: RegExp, replacement: String): String
    @JsName("replace")
    fun replaceFn1(regex: RegExp, replacer: (String) -> String): String
    @JsName("replace")
    fun replaceFn3(regex: RegExp, replacer: (String, String?, String?) -> String): String
    @JsName("replace")
    fun replaceOffset(regex: RegExp, replacer: (String, Int, String) -> String): String
    fun search(regex: RegExp): Int
    @JsName("split")
    fun splitStr(separator: String): JsArray<String>
    @JsName("split")
    fun splitStrLimit(separator: String, limit: Int): JsArray<String>
    @JsName("split")
    fun splitRe(regex: RegExp): JsArray<String>
    fun slice(start: Int): String
    fun slice(start: Int, end: Int): String
    fun substring(start: Int): String
    fun substring(start: Int, end: Int): String
    fun includes(searchString: String): Boolean
    fun repeat(count: Int): String
    fun endsWith(searchString: String): Boolean
    fun indexOf(searchString: String): Int
    fun lastIndexOf(searchString: String): Int
    fun charCodeAt(index: Int): Int
}

internal inline fun String.asJs(): JsString = this.unsafeCast<JsString>()

internal inline val RegExp.source: String get() = asDynamic().source.unsafeCast<String>()

internal inline fun RegExpMatch.req(index: Int): String = asDynamic()[index].unsafeCast<String>()

internal inline fun RegExpMatch.opt(index: Int): String? = asDynamic()[index].unsafeCast<String?>()

internal inline fun String?.isNullOrBlankStr(): Boolean = this == null || this.length == 0

@JsName("String")
external object JsStringStatic {
    fun fromCodePoint(codePoint: Int): String
}

@JsName("Array")
external class JsArray<T>() {
    val length: Int
    fun push(element: T): Int
    fun pop(): T?
    fun shift(): T?
    fun unshift(element: T): Int
    fun slice(start: Int = definedExternally, end: Int = definedExternally): JsArray<T>
    fun splice(start: Int, deleteCount: Int = definedExternally): JsArray<T>
    fun join(separator: String = definedExternally): String
}

internal inline operator fun <T> JsArray<T>.get(index: Int): T = asDynamic()[index].unsafeCast<T>()

internal inline operator fun <T> JsArray<T>.set(index: Int, value: T) {
    asDynamic()[index] = value
}

internal inline fun <T> JsArray<T>.lastOrNull(): T? = if (length > 0) this[length - 1] else null

@JsName("Map")
external class JsMap<K, V>() {
    val size: Int
    fun get(key: K): V?
    fun set(key: K, value: V): JsMap<K, V>
    fun has(key: K): Boolean
}

external fun encodeURI(uri: String): String

external fun parseInt(s: String, radix: Int): Int
