// Entry of the Kotlin/JS port: what libraries/idb/reference.mjs does, on the port's idb. A case is
// a script of idb calls; each call works on databases of its own, named by a fresh prefix, and
// closes and deletes them before it settles, also after a failure.
//
// The boundary: the corpus input is a JS object typed by external interfaces and read in place.
// Keys, values, versions, counts, directions, modes and options stay the JS values they are
// (`Any?`, undefined where a field or argument is absent) and reach IndexedDB unchanged. The
// result is a plain object { log, databases } of JS arrays. `run` is a native ES module export
// through @JsExport.

import idb.Cursor
import idb.CursorIterator
import idb.Database
import idb.IDBDatabase
import idb.IDBKeyRange
import idb.IteratedCursor
import idb.JsReflect
import idb.JsTypeError
import idb.Method
import idb.Shortcut
import idb.Source
import idb.Transaction
import idb.await
import idb.crypto
import idb.deleteDB
import idb.isUndefined
import idb.launch
import idb.name
import idb.openDB
import idb.unwrap
import idb.wrap
import kotlin.js.Json
import kotlin.js.Promise
import kotlin.js.json

external interface Input {
    val steps: Array<Step>
}

/** A step of any kind; [op] says which of the interfaces below it is. */
external interface Step {
    val op: String

    @JsName("catch")
    val catches: Boolean?
}

external interface OpenStep : Step {
    val db: String
    val conn: String
    val version: Any?
    val upgrade: Array<UpgradeStep>?
    val blocking: String?

    /** `"log"` or `{ close: conn }`. */
    val blocked: Any?
}

external interface CloseStep : Step {
    val conn: String
}

external interface DeleteStep : Step {
    val db: String
    val blocked: Any?
}

external interface BlockedClose {
    val close: String
}

external interface TxStep : Step {
    val conn: String
    val stores: Any?
    val mode: Any?
    val steps: Array<Step>
}

external interface CallStep : Step {
    val conn: String
    val method: String
    val store: String
    val index: String?
    val args: Array<Any?>
}

/** A request on `store` (null for `tx.store`) or its `index`. */
external interface RequestStep : Step {
    val store: String?
    val index: String?
    val args: Array<Any?>
}

external interface WrapStep : RequestStep {
    val method: String
}

external interface BatchStep : Step {
    val requests: Array<RequestStep>
    val abort: Boolean
    val done: Boolean
}

/** openCursor, openKeyCursor or iterate; each action is `[name, ...values]`. */
external interface WalkStep : RequestStep {
    val positions: Array<Array<Array<Any?>>>
    val rest: Array<Array<Any?>>
}

external interface UpgradeStep {
    val op: String
    val name: String
    val options: Any?
    val store: String
    val keyPath: Any?
    val args: Array<Any?>
}

external interface KeyRangeSpec {
    val only: Any?
    val lower: Any?
    val upper: Any?
    val lowerOpen: Any?
    val upperOpen: Any?
}

/** A failure that the step where it happened has already logged. */
private class Failed : Throwable()

/** An input outside the contract (CONTRACT.md, "Parity domain"): it rejects the call after
 *  cleanup, with a TypeError, and is never logged. */
private class OutOfContract(message: String) : Throwable(message)

private val UPGRADES = setOf("createStore", "deleteStore", "createIndex", "deleteIndex", "put", "add", "abort")

/** The methods whose first argument is a query: a key, or a key range written as an object. */
private val QUERY_FIRST = setOf("get", "getKey", "getAll", "getAllKeys", "count", "delete", "openCursor", "openKeyCursor", "iterate")

/** `[label, "ok"]` when the result is undefined, else `[label, "ok", value]`. */
private fun ok(label: String, value: Any?): Array<Any?> =
    if (isUndefined(value)) arrayOf(label, "ok") else arrayOf(label, "ok", value)

private fun requestMethod(op: String): Method = Method.of(op) ?: throw OutOfContract("not a request method: $op")

private fun actions(step: WalkStep, position: Int): Array<Array<Any?>> =
    if (position < step.positions.size) step.positions[position] else step.rest

/** The query methods take a key, or a key range written as an object, first. */
private fun args(method: String, list: Array<Any?>): Array<Any?> =
    if (method in QUERY_FIRST && list.isNotEmpty()) arrayOf(query(list[0]), *list.copyOfRange(1, list.size)) else list

/** A key is passed as it is, null included. Presence is tested with `in`, so `{ only: null }`
 *  is only(null); an absent open flag is undefined, which IndexedDB reads as false. */
private fun query(value: Any?): Any? {
    if (value == null || jsTypeOf(value) != "object" || value is Array<*>) return value
    val range = value.unsafeCast<KeyRangeSpec>()
    val lower = JsReflect.has(value, "lower")
    val upper = JsReflect.has(value, "upper")
    return when {
        JsReflect.has(value, "only") -> IDBKeyRange.only(range.only)
        lower && upper -> IDBKeyRange.bound(range.lower, range.upper, range.lowerOpen, range.upperOpen)
        lower -> IDBKeyRange.lowerBound(range.lower, range.lowerOpen)
        else -> IDBKeyRange.upperBound(range.upper, range.upperOpen)
    }
}

/** `values[i]`, or undefined past the end. */
private fun Array<Any?>.argument(i: Int): Any? = if (i < size) this[i] else undefined

private fun MutableList<String>.remember(name: String) {
    if (name !in this) add(name)
}

/** One move made by a walk's position: the promise it returned, and the move's name. */
private class Move(val label: String, val promise: Promise<Cursor?>)

private class Session(private val input: Input) {
    private val prefix = "mires-matrix-idb-" + crypto.randomUUID() + "-"
    private val log = mutableListOf<Array<Any?>>()
    private val connections = mutableMapOf<String, Database>()
    private val opened = mutableListOf<Database>()
    private val attempted = mutableListOf<String>()
    private val live = mutableListOf<String>()

    suspend fun run(): Json {
        try {
            try {
                for (step in input.steps) perform(step, null)
            } catch (_: Failed) {
                // A logged failure ends the script there; anything else rejects the call.
            }
            val databases = live.map { dump(it) }
            return json("log" to log.toTypedArray(), "databases" to databases.toTypedArray())
        } finally {
            for (db in opened) db.close()
            for (name in attempted) deleteDB(prefix + name).await()
        }
    }

    /** Logs a failure where it happens; the Failed that propagates says it is logged. */
    private fun failed(label: String, error: Throwable): Throwable {
        if (error is OutOfContract) return error
        log += arrayOf(label, "error", error.name)
        return Failed()
    }

    /** One call: logs `[label, "ok", shown]` (just `[label, "ok"]` when [show] gives undefined)
     *  or `[label, "error", name]`. */
    private suspend fun <T> attempt(label: String, show: (T) -> Any?, action: suspend () -> T): T {
        val value = try {
            action()
        } catch (error: Throwable) {
            throw failed(label, error)
        }
        log += ok(label, show(value))
        return value
    }

    /** A call whose result is a host value, undefined for a miss. */
    private suspend fun attempt(label: String, action: suspend () -> Any?) {
        attempt(label, { it }, action)
    }

    /** A call with no result to show. */
    private suspend fun attemptUnit(label: String, action: suspend () -> Unit) {
        attempt(label, { undefined }, action)
    }

    private fun connection(conn: String): Database =
        connections[conn] ?: throw OutOfContract("no connection named $conn")

    private fun source(tx: Transaction?, step: RequestStep): Source {
        if (tx == null) throw OutOfContract("a transaction step outside a transaction")
        val store = step.store?.let { tx.objectStore(it) } ?: tx.store ?: throw OutOfContract("store: null in a transaction with several stores")
        return step.index?.let { store.index(it) } ?: store
    }

    private fun inside(tx: Transaction?): Transaction = tx ?: throw OutOfContract("a transaction step outside a transaction")

    private suspend fun perform(step: Step, tx: Transaction?) {
        try {
            execute(step, tx)
        } catch (failure: Failed) {
            if (step.catches != true) throw failure
        }
    }

    private suspend fun execute(step: Step, tx: Transaction?) {
        when (step.op) {
            "openDB" -> open(step.unsafeCast<OpenStep>())
            "close" -> attemptUnit("close") { connection(step.unsafeCast<CloseStep>().conn).close() }
            "deleteDB" -> remove(step.unsafeCast<DeleteStep>())
            "tx" -> transaction(step.unsafeCast<TxStep>())
            "call" -> shortcut(step.unsafeCast<CallStep>())
            "batch" -> batch(step.unsafeCast<BatchStep>(), inside(tx))
            "openCursor", "openKeyCursor" -> walk(step.unsafeCast<WalkStep>(), tx)
            "iterate" -> iterate(step.unsafeCast<WalkStep>(), tx)
            "abort" -> attemptUnit("abort") { inside(tx).abort() }
            "done" -> attemptUnit("done") { inside(tx).done.await() }
            "store" -> attempt("store") { inside(tx).store?.name ?: undefined }
            "wrap" -> {
                val request = step.unsafeCast<WrapStep>()
                val method = requestMethod(request.method)
                attempt("wrap") { wrap(method.call(unwrap(source(tx, request)), args(method.jsName, request.args))).await() }
            }
            else -> {
                val request = step.unsafeCast<RequestStep>()
                val method = requestMethod(step.op)
                attempt(step.op) { source(tx, request).request(method, *args(step.op, request.args)).await() }
            }
        }
    }

    private suspend fun open(step: OpenStep) {
        val upgrades = step.upgrade
        upgrades?.forEach { if (it.op !in UPGRADES) throw OutOfContract("not an upgrade step: ${it.op}") }
        attempted.remember(step.db)
        val blocked = step.blocked
        val closing = if (blocked != null && jsTypeOf(blocked) == "object") connection(blocked.unsafeCast<BlockedClose>().close) else null
        val db = attempt("openDB", { db: Database -> arrayOf(db.version, db.objectStoreNames.toTypedArray()) }) {
            openDB(
                prefix + step.db,
                step.version,
                upgrade = upgrades?.let { ops ->
                    { db, oldVersion, newVersion, transaction ->
                        log += arrayOf("upgrade", oldVersion, newVersion)
                        for (op in ops) upgrade(op, db, transaction)
                    }
                },
                blocked = blocked?.let {
                    { currentVersion, blockedVersion, _ ->
                        log += arrayOf("blocked", currentVersion, blockedVersion)
                        closing?.close()
                    }
                },
                blocking = step.blocking?.let { kind ->
                    { currentVersion, blockedVersion, event ->
                        log += arrayOf("blocking", step.conn, currentVersion, blockedVersion)
                        if (kind == "close") event.target.unsafeCast<IDBDatabase>().close()
                    }
                },
            ).await()
        }
        connections[step.conn] = db
        opened += db
        live.remember(step.db)
    }

    private suspend fun remove(step: DeleteStep) {
        attempted.remember(step.db)
        val blocked = step.blocked
        val closing = if (blocked != null && jsTypeOf(blocked) == "object") connection(blocked.unsafeCast<BlockedClose>().close) else null
        attemptUnit("deleteDB") {
            deleteDB(prefix + step.db, blocked?.let {
                { currentVersion, _ ->
                    log += arrayOf("blocked", currentVersion)
                    closing?.close()
                }
            }).await()
        }
        live.remove(step.db)
    }

    /** Runs synchronously inside the upgrade callback; a failure is logged and the callback goes on. */
    private fun upgrade(op: UpgradeStep, db: Database, transaction: Transaction) {
        try {
            when (op.op) {
                "createStore" -> log += arrayOf("createStore", "ok", db.createObjectStore(op.name, op.options).name)
                "deleteStore" -> {
                    db.deleteObjectStore(op.name)
                    log += arrayOf("deleteStore", "ok")
                }
                "createIndex" -> log += arrayOf("createIndex", "ok", transaction.objectStore(op.store).createIndex(op.name, op.keyPath, op.options).name)
                "deleteIndex" -> {
                    transaction.objectStore(op.store).deleteIndex(op.name)
                    log += arrayOf("deleteIndex", "ok")
                }
                "abort" -> {
                    transaction.abort()
                    log += arrayOf("abort", "ok")
                }
                // put or add: issued, not awaited. A failed write aborts the upgrade, which rejects the open.
                else -> transaction.objectStore(op.store).request(requestMethod(op.op), *op.args).catch { }
            }
        } catch (error: Throwable) {
            log += arrayOf(op.op, "error", error.name)
        }
    }

    private suspend fun transaction(step: TxStep) {
        val tx = try {
            connection(step.conn).transaction(step.stores, step.mode)
        } catch (error: Throwable) {
            throw failed("tx", error)
        }
        for (inner in step.steps) perform(inner, tx)
    }

    private suspend fun shortcut(step: CallStep) {
        val shortcut = Shortcut.of(step.method) ?: throw OutOfContract("not a database shortcut: ${step.method}")
        if (shortcut.useIndex != (step.index != null)) throw OutOfContract("an index goes with the FromIndex forms only")
        attempt(step.method) {
            val db = connection(step.conn)
            db.shortcut(shortcut, step.store, step.index, *args(shortcut.method.jsName, step.args)).await()
        }
    }

    /** Issues every request before awaiting any, then logs each outcome in issue order. */
    private suspend fun batch(step: BatchStep, tx: Transaction) {
        val methods = step.requests.map { requestMethod(it.op) }
        // Each issue is the reference's async callback: a synchronous failure is a rejected promise.
        val pending = step.requests.mapIndexed { i, request ->
            try {
                source(tx, request).request(methods[i], *args(request.op, request.args))
            } catch (error: Throwable) {
                Promise.reject(error)
            }
        }
        if (step.abort) {
            try {
                attemptUnit("abort") { tx.abort() }
            } catch (_: Failed) {
            }
        }
        val settled = (pending + listOfNotNull(tx.done.takeIf { step.done }))
            .map { promise -> promise.then({ Result.success(it) }, { Result.failure(it) }) }
        val outcomes = Promise.all(settled.toTypedArray()).await()
        outcomes.firstNotNullOfOrNull { it.exceptionOrNull() as? OutOfContract }?.let { throw it }
        outcomes.forEachIndexed { i, outcome ->
            val label = if (i < pending.size) step.requests[i].op else "done"
            log += outcome.fold(
                { value -> ok(label, if (i < pending.size) value else undefined) },
                { error -> arrayOf(label, "error", error.name) },
            )
        }
    }

    private fun position(step: WalkStep, cursor: Cursor) {
        log += if (step.op == "openKeyCursor") arrayOf("cursor", cursor.key, cursor.primaryKey) else arrayOf("cursor", cursor.key, cursor.primaryKey, cursor.value)
    }

    /** `cursor.update(value)` and `cursor.delete()`, awaited and logged. */
    private suspend fun write(cursor: Cursor, action: Array<Any?>) {
        if (action[0] == "update") attempt("update") { cursor.update(action.argument(1)).await() } else attempt("delete") { cursor.delete().await() }
    }

    private fun isWrite(action: Array<Any?>) = action[0] == "update" || action[0] == "delete"

    /** A walk with the promise-returning moves; a position without a move ends it. */
    private suspend fun walk(step: WalkStep, tx: Transaction?) {
        var cursor = try {
            val source = source(tx, step)
            val args = args(step.op, step.args)
            (if (step.op == "openKeyCursor") source.openKeyCursor(*args) else source.openCursor(*args)).await()
        } catch (error: Throwable) {
            throw failed(step.op, error)
        }
        var position = 0
        while (cursor != null) {
            position(step, cursor)
            var moved: Move? = null
            for (action in actions(step, position)) {
                if (action[0] == "break") return
                if (isWrite(action)) write(cursor, action) else moved = move(cursor, action)
            }
            if (moved == null) return
            cursor = try {
                moved.promise.await()
            } catch (error: Throwable) {
                throw failed(moved.label, error)
            }
            position++
        }
        log += arrayOf<Any?>("end")
    }

    private fun move(cursor: Cursor, action: Array<Any?>): Move {
        val name = action[0] as String
        val values = action.copyOfRange(1, action.size)
        return try {
            Move(name, when (name) {
                "continue" -> cursor.continueTo(*values)
                "advance" -> cursor.advance(*values)
                "continuePrimaryKey" -> cursor.continuePrimaryKey(*values)
                else -> throw OutOfContract("not a cursor action: $name")
            })
        } catch (error: Throwable) {
            throw failed(name, error)
        }
    }

    /** The async iterator: a move only records the next step, and a position without one continues. */
    private suspend fun iterate(step: WalkStep, tx: Transaction?) {
        val iterator: CursorIterator = try {
            source(tx, step).iterate(*args("iterate", step.args))
        } catch (error: Throwable) {
            throw failed("iterate", error)
        }
        var position = 0
        while (true) {
            val next = try {
                iterator.next()
            } catch (error: Throwable) {
                throw failed("iterate", error)
            }
            val cursor: IteratedCursor = next ?: break
            log += arrayOf("cursor", cursor.cursor.key, cursor.cursor.primaryKey, cursor.cursor.value)
            for (action in actions(step, position)) {
                if (action[0] == "break") return
                if (isWrite(action)) write(cursor.cursor, action) else intercepted(cursor, action)
            }
            position++
        }
        log += arrayOf<Any?>("end")
    }

    private fun intercepted(cursor: IteratedCursor, action: Array<Any?>) {
        val name = action[0] as String
        val values = action.copyOfRange(1, action.size)
        try {
            when (name) {
                "continue" -> cursor.continueTo(*values)
                "advance" -> cursor.advance(*values)
                "continuePrimaryKey" -> cursor.continuePrimaryKey(*values)
                else -> throw OutOfContract("not a cursor action: $name")
            }
        } catch (error: Throwable) {
            throw failed(name, error)
        }
    }

    /** `[name, version, stores]`, each store `[name, keyPath, autoIncrement, indexes, records]`,
     *  read in one readonly transaction; records are getAllKeys() zipped with getAll(). */
    private suspend fun dump(name: String): Array<Any?> {
        val db = openDB(prefix + name).await()
        try {
            val names = db.objectStoreNames
            val stores = mutableListOf<Array<Any?>>()
            if (names.isNotEmpty()) {
                val tx = db.transaction(names.toTypedArray())
                for (storeName in names) {
                    val store = tx.objectStore(storeName)
                    val indexes = store.indexNames.map { indexName ->
                        val index = store.index(indexName)
                        arrayOf(indexName, index.keyPath, index.unique, index.multiEntry)
                    }
                    val keys = store.request(Method.GET_ALL_KEYS).await().unsafeCast<Array<Any?>>()
                    val values = store.request(Method.GET_ALL).await().unsafeCast<Array<Any?>>()
                    val records = keys.mapIndexed { i, key -> arrayOf(key, values[i]) }
                    stores += arrayOf(storeName, store.keyPath, store.autoIncrement, indexes.toTypedArray(), records.toTypedArray())
                }
                tx.done.await()
            }
            return arrayOf(name, db.version, stores.toTypedArray())
        } finally {
            db.close()
        }
    }
}

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: Input): Promise<Json> = launch {
    try {
        Session(input).run()
    } catch (error: OutOfContract) {
        throw JsTypeError(error.message ?: "outside the contract").unsafeCast<Throwable>()
    }
}
