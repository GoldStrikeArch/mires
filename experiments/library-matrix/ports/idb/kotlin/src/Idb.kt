// idb 8.0.4 (build/index.js) for Kotlin/JS: IndexedDB requests become promises, a transaction gets
// a `done` promise when it is wrapped, cursors move by returning a promise of the next cursor, the
// database has the database-extras shortcuts, and `iterate` walks a store or an index.
//
// idb wraps each IDB object in a Proxy that forwards every property and wraps what comes back.
// Here each wrapped object is a class over the host object; its members call the host member and
// wrap the result the same way. Promises are host promises (kotlin.js.Promise), created and
// settled by idb's own listener logic, so a request is issued when its method is called.

package idb

import kotlin.js.Promise

/** A wrapped IndexedDB object; [unwrap] gives the host object behind it. */
abstract class Wrapped<T : Any> internal constructor(internal val raw: T)

/** idb's `unwrap`. */
fun <T : Any> unwrap(value: Wrapped<T>): T = value.raw

/** idb's `promisifyRequest`: settles once, with the request's result on `success` (wrapped by
 *  [wrapResult], as idb's `wrap(request.result)`) or with its error on `error`, then removes both
 *  listeners. A cursor request fires once per move, so each move promisifies it again. */
internal fun <T> promisify(request: IDBRequest, wrapResult: (Any?) -> T): Promise<T> = Promise { resolve, reject ->
    lateinit var unlisten: () -> Unit
    val success: (Event) -> Unit = {
        resolve(wrapResult(request.result))
        unlisten()
    }
    val error: (Event) -> Unit = {
        reject(request.error!!)
        unlisten()
    }
    unlisten = {
        request.removeEventListener("success", success)
        request.removeEventListener("error", error)
    }
    request.addEventListener("success", success)
    request.addEventListener("error", error)
}

/** idb's `wrap` of a request: a promise of its result, which is undefined for a miss. */
fun wrap(request: IDBRequest): Promise<Any?> = promisify(request) { it }

private fun wrapCursor(request: IDBRequest): Promise<Cursor?> =
    promisify(request) { result -> result?.let { Cursor(it.unsafeCast<IDBCursor>()) } }

/** idb's `cacheDonePromiseForTransaction`: settled by the first `complete`, `error` or `abort`.
 *  A request error bubbles to the transaction before the transaction records it, so `done` then
 *  rejects with a new AbortError. */
private fun cacheDone(tx: IDBTransaction): Promise<Unit> = Promise { resolve, reject ->
    lateinit var unlisten: () -> Unit
    val complete: (Event) -> Unit = {
        resolve(Unit)
        unlisten()
    }
    val error: (Event) -> Unit = {
        reject(tx.error ?: DOMException("AbortError", "AbortError").unsafeCast<Throwable>())
        unlisten()
    }
    unlisten = {
        tx.removeEventListener("complete", complete)
        tx.removeEventListener("error", error)
        tx.removeEventListener("abort", error)
    }
    tx.addEventListener("complete", complete)
    tx.addEventListener("error", error)
    tx.addEventListener("abort", error)
}

/** The request methods idb promisifies on a store or an index, with database-extras'
 *  `readMethods` and `writeMethods` split. */
enum class Method(val jsName: String, val isWrite: Boolean) {
    GET("get", false),
    GET_KEY("getKey", false),
    GET_ALL("getAll", false),
    GET_ALL_KEYS("getAllKeys", false),
    COUNT("count", false),
    PUT("put", true),
    ADD("add", true),
    DELETE("delete", true),
    CLEAR("clear", true);

    internal fun call(source: IDBSource, args: Array<out Any?>): IDBRequest = when (this) {
        GET -> source.get(*args)
        GET_KEY -> source.getKey(*args)
        GET_ALL -> source.getAll(*args)
        GET_ALL_KEYS -> source.getAllKeys(*args)
        COUNT -> source.count(*args)
        PUT -> source.put(*args)
        ADD -> source.add(*args)
        DELETE -> source.delete(*args)
        CLEAR -> source.clear(*args)
    }

    companion object {
        fun of(name: String): Method? = entries.firstOrNull { it.jsName == name }
    }
}

/** An object store or an index: the methods both have. A host exception is thrown at once,
 *  before any promise exists. */
sealed class Source(raw: IDBSource) : Wrapped<IDBSource>(raw) {
    val name: String get() = raw.name
    val keyPath: Any? get() = raw.keyPath

    fun request(method: Method, vararg args: Any?): Promise<Any?> = wrap(method.call(raw, args))

    fun openCursor(vararg args: Any?): Promise<Cursor?> = wrapCursor(raw.openCursor(*args))

    fun openKeyCursor(vararg args: Any?): Promise<Cursor?> = wrapCursor(raw.openKeyCursor(*args))

    /** idb's async iterator, `openCursor(...args)` walked by [CursorIterator.next]. */
    fun iterate(vararg args: Any?): CursorIterator = CursorIterator(this, args)
}

class ObjectStore internal constructor(private val store: IDBObjectStore) : Source(store) {
    val autoIncrement: Boolean get() = store.autoIncrement
    val indexNames: List<String> get() = store.indexNames.toList()

    fun index(name: String): Index = Index(store.index(name))

    fun createIndex(name: String, keyPath: Any?, options: Any? = undefined): Index =
        Index(store.createIndex(name, keyPath, options))

    fun deleteIndex(name: String) = store.deleteIndex(name)
}

class Index internal constructor(private val index: IDBIndex) : Source(index) {
    val unique: Boolean get() = index.unique
    val multiEntry: Boolean get() = index.multiEntry
}

/** A wrapped IDBCursor. Its moves are idb's cursor advance methods: they call the host method,
 *  then promisify the cursor's request again, which gives this cursor or null past the end. */
class Cursor internal constructor(raw: IDBCursor) : Wrapped<IDBCursor>(raw) {
    val key: Any? get() = raw.key
    val primaryKey: Any? get() = raw.primaryKey
    val value: Any? get() = raw.value

    fun advance(vararg args: Any?): Promise<Cursor?> {
        raw.advance(*args)
        return wrapCursor(raw.request)
    }

    /** idb's `continue`. */
    fun continueTo(vararg args: Any?): Promise<Cursor?> {
        raw.continueTo(*args)
        return wrapCursor(raw.request)
    }

    fun continuePrimaryKey(vararg args: Any?): Promise<Cursor?> {
        raw.continuePrimaryKey(*args)
        return wrapCursor(raw.request)
    }

    fun update(value: Any?): Promise<Any?> = wrap(raw.update(value))

    fun delete(): Promise<Any?> = wrap(raw.delete())
}

class Transaction internal constructor(raw: IDBTransaction) : Wrapped<IDBTransaction>(raw) {
    /** Created with the wrapper, as idb caches it when it first wraps the transaction, so it also
     *  settles for a caller that only asks after the transaction finished. */
    val done: Promise<Unit> = cacheDone(raw)

    /** idb's `tx.store`: the only store in the scope, or null when there are several. idb tests
     *  the second name before it calls objectStore(), so on a finished transaction this throws
     *  InvalidStateError for one store and gives null for several. */
    val store: ObjectStore?
        get() {
            val names = raw.objectStoreNames
            return if (!names.item(1).isNullOrEmpty()) null else objectStore(names.item(0)!!)
        }

    fun objectStore(name: String): ObjectStore = ObjectStore(raw.objectStore(name))

    fun abort() = raw.abort()
}

/** A database shortcut (database-extras' `getMethod`): a request method on the store, or with
 *  [useIndex] (`getFromIndex`, …) on one of its indexes. Only read methods have index forms. */
class Shortcut private constructor(val method: Method, val useIndex: Boolean) {
    companion object {
        fun of(name: String): Shortcut? {
            val target = name.removeSuffix("FromIndex")
            val useIndex = target != name
            val method = Method.of(target) ?: return null
            return if (useIndex && method.isWrite) null else Shortcut(method, useIndex)
        }
    }
}

class Database internal constructor(raw: IDBDatabase) : Wrapped<IDBDatabase>(raw) {
    val version: Double get() = raw.version
    val objectStoreNames: List<String> get() = raw.objectStoreNames.toList()

    fun createObjectStore(name: String, options: Any? = undefined): ObjectStore =
        ObjectStore(raw.createObjectStore(name, options))

    fun deleteObjectStore(name: String) = raw.deleteObjectStore(name)

    /** [storeNames] is a name or an array of names; an undefined [mode] is readonly. */
    fun transaction(storeNames: Any?, mode: Any? = undefined): Transaction =
        Transaction(raw.transaction(storeNames, mode))

    fun close() = raw.close()

    /** One shortcut call, as getMethod's async method: a transaction per call, `readwrite` for a
     *  write, on `tx.store` or its index [indexName], and `Promise.all([request, isWrite &&
     *  tx.done])`, so a write resolves only after `done`, the request's error comes first, and
     *  both promises are handled. A synchronous failure rejects the call. */
    fun shortcut(shortcut: Shortcut, storeName: String, indexName: String?, vararg args: Any?): Promise<Any?> = launch {
        val method = shortcut.method
        val tx = transaction(storeName, if (method.isWrite) "readwrite" else "readonly")
        val store = checkNotNull(tx.store) { "a one-store transaction has a store" }
        val target = if (shortcut.useIndex) store.index(checkNotNull(indexName)) else store
        val request = target.request(method, *args)
        Promise.all(if (method.isWrite) arrayOf(request, tx.done) else arrayOf(request)).await()[0]
    }
}

/** idb's `iterate` generator. The first [next] opens `openCursor(...args)`; each later one
 *  awaits the move the loop body made on the yielded cursor, or calls `continue()` when it made
 *  none. A failed step ends the iteration, as a throwing generator is done. */
class CursorIterator internal constructor(private val source: Source, private val args: Array<out Any?>) {
    private var opened = false
    private var cursor: Cursor? = null

    /** The move the loop body made: the iterator's `advanceResults` entry. */
    internal var advanced: Promise<Cursor?>? = null

    suspend fun next(): IteratedCursor? {
        val step = if (!opened) {
            opened = true
            source.openCursor(*args)
        } else {
            val current = cursor ?: return null
            cursor = null
            advanced ?: current.continueTo()
        }
        val next = step.await()
        advanced = null
        cursor = next
        return next?.let { IteratedCursor(this, it) }
    }
}

/** The cursor an iteration yields (idb's proxied cursor): its moves call the host method and
 *  record the result for the iterator instead of returning it. */
class IteratedCursor internal constructor(private val iterator: CursorIterator, val cursor: Cursor) {
    fun advance(vararg args: Any?) {
        iterator.advanced = cursor.advance(*args)
    }

    fun continueTo(vararg args: Any?) {
        iterator.advanced = cursor.continueTo(*args)
    }

    fun continuePrimaryKey(vararg args: Any?) {
        iterator.advanced = cursor.continuePrimaryKey(*args)
    }
}

/** openDB's `upgrade(db, oldVersion, newVersion, transaction)`. */
typealias Upgrade = (db: Database, oldVersion: Double, newVersion: Double?, transaction: Transaction) -> Unit

/** openDB's `blocked` and `blocking`: `(currentVersion, blockedVersion, event)`. */
typealias VersionChange = (currentVersion: Double, blockedVersion: Double?, event: IDBVersionChangeEvent) -> Unit

/** idb's `openDB(name, version, { upgrade, blocked, blocking })`. An undefined [version] opens
 *  the current version, or 1 for a new database; an invalid one throws here. `blocking` is
 *  attached to the connection once the open succeeds. */
fun openDB(
    name: String,
    version: Any? = undefined,
    upgrade: Upgrade? = null,
    blocked: VersionChange? = null,
    blocking: VersionChange? = null,
): Promise<Database> {
    val request = indexedDB.open(name, version)
    val openPromise = promisify(request) { Database(it.unsafeCast<IDBDatabase>()) }
    if (upgrade != null) {
        request.addEventListener("upgradeneeded") { event ->
            val change = event.unsafeCast<IDBVersionChangeEvent>()
            upgrade(Database(request.result.unsafeCast<IDBDatabase>()), change.oldVersion, change.newVersion, Transaction(request.transaction!!))
        }
    }
    if (blocked != null) {
        request.addEventListener("blocked") { event ->
            val change = event.unsafeCast<IDBVersionChangeEvent>()
            blocked(change.oldVersion, change.newVersion, change)
        }
    }
    openPromise.then({ db ->
        if (blocking != null) {
            db.raw.addEventListener("versionchange") { event ->
                val change = event.unsafeCast<IDBVersionChangeEvent>()
                blocking(change.oldVersion, change.newVersion, change)
            }
        }
    }, {})
    return openPromise
}

/** idb's `deleteDB(name, { blocked })`; `blocked(currentVersion, event)`. */
fun deleteDB(name: String, blocked: ((currentVersion: Double, event: IDBVersionChangeEvent) -> Unit)? = null): Promise<Unit> {
    val request = indexedDB.deleteDatabase(name)
    if (blocked != null) {
        request.addEventListener("blocked") { event ->
            val change = event.unsafeCast<IDBVersionChangeEvent>()
            blocked(change.oldVersion, change)
        }
    }
    return wrap(request).then { }
}
