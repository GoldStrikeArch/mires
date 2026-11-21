// The host APIs idb wraps: IndexedDB (the factory, requests, databases, transactions, object
// stores, indexes, cursors and key ranges) and DOM events, as external declarations, plus the
// coroutine bridge to promises. A JS exception thrown by one of them is a Kotlin Throwable at run
// time (a DOMException or a TypeError), and keeps its JS name.

package idb

import kotlin.coroutines.Continuation
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlin.coroutines.startCoroutine
import kotlin.coroutines.suspendCoroutine
import kotlin.js.Promise

external interface Event {
    val target: Any?
}

external interface EventTarget {
    fun addEventListener(type: String, listener: (Event) -> Unit)
    fun removeEventListener(type: String, listener: (Event) -> Unit)
}

external interface IDBVersionChangeEvent : Event {
    val oldVersion: Double
    val newVersion: Double?
}

external interface DOMStringList {
    val length: Int
    fun item(index: Int): String?
}

external interface IDBRequest : EventTarget {
    val result: Any?
    val error: Throwable?
    val transaction: IDBTransaction?
}

external interface IDBFactory {
    /** An undefined [version] is absent: the current version, or 1 for a new database. */
    fun open(name: String, version: Any?): IDBRequest
    fun deleteDatabase(name: String): IDBRequest
}

external val indexedDB: IDBFactory

external interface IDBDatabase : EventTarget {
    val version: Double
    val objectStoreNames: DOMStringList
    fun createObjectStore(name: String, options: Any?): IDBObjectStore
    fun deleteObjectStore(name: String)
    fun transaction(storeNames: Any?, mode: Any?): IDBTransaction
    fun close()
}

external interface IDBTransaction : EventTarget {
    val objectStoreNames: DOMStringList
    val error: Throwable?
    fun objectStore(name: String): IDBObjectStore
    fun abort()
}

/** The request methods of IDBObjectStore and IDBIndex. idb passes their arguments on as it gets
 *  them, so they are variadic here; a write method is undefined on an index, and calling it
 *  throws a TypeError, as it does through idb's proxy. */
external interface IDBSource {
    val name: String
    val keyPath: Any?
    fun get(vararg args: Any?): IDBRequest
    fun getKey(vararg args: Any?): IDBRequest
    fun getAll(vararg args: Any?): IDBRequest
    fun getAllKeys(vararg args: Any?): IDBRequest
    fun count(vararg args: Any?): IDBRequest
    fun put(vararg args: Any?): IDBRequest
    fun add(vararg args: Any?): IDBRequest
    fun delete(vararg args: Any?): IDBRequest
    fun clear(vararg args: Any?): IDBRequest
    fun openCursor(vararg args: Any?): IDBRequest
    fun openKeyCursor(vararg args: Any?): IDBRequest
}

external interface IDBObjectStore : IDBSource {
    val autoIncrement: Boolean
    val indexNames: DOMStringList
    fun index(name: String): IDBIndex
    fun createIndex(name: String, keyPath: Any?, options: Any?): IDBIndex
    fun deleteIndex(name: String)
}

external interface IDBIndex : IDBSource {
    val unique: Boolean
    val multiEntry: Boolean
}

/** An IDBCursor; `value` is undefined on a key cursor. */
external interface IDBCursor {
    val key: Any?
    val primaryKey: Any?
    val value: Any?
    val request: IDBRequest
    fun advance(vararg args: Any?)

    @JsName("continue")
    fun continueTo(vararg args: Any?)
    fun continuePrimaryKey(vararg args: Any?)
    fun update(value: Any?): IDBRequest
    fun delete(): IDBRequest
}

external class IDBKeyRange {
    companion object {
        fun only(value: Any?): IDBKeyRange
        fun lowerBound(lower: Any?, open: Any?): IDBKeyRange
        fun upperBound(upper: Any?, open: Any?): IDBKeyRange
        fun bound(lower: Any?, upper: Any?, lowerOpen: Any?, upperOpen: Any?): IDBKeyRange
    }
}

external class DOMException(message: String, name: String)

@JsName("TypeError")
external class JsTypeError(message: String)

external object crypto {
    fun randomUUID(): String
}

@JsName("Reflect")
external object JsReflect {
    fun has(target: Any, key: String): Boolean
}

/** A JS exception's `name`. */
val Throwable.name: String
    get() = asDynamic().name as? String ?: "Error"

fun isUndefined(value: Any?) = jsTypeOf(value) == "undefined"

fun DOMStringList.toList(): List<String> = List(length) { item(it)!! }

suspend fun <T> Promise<T>.await(): T = suspendCoroutine { continuation ->
    then({ value -> continuation.resume(value) }, { error -> continuation.resumeWithException(error) })
}

/** Runs [block] as a coroutine, which starts at once and settles the promise with its outcome. */
fun <T> launch(block: suspend () -> T): Promise<T> = Promise { resolve, reject ->
    block.startCoroutine(Continuation(EmptyCoroutineContext) { result -> result.fold(resolve, reject) })
}
