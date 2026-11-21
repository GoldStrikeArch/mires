// idb 8.0.4 (build/index.js) for Scala.js: IndexedDB requests become Futures, a transaction gets a
// `done` Future when it is wrapped, cursors move by returning a Future of the next cursor, the
// database has the database-extras shortcuts, and `iterate` walks a store or an index.
//
// idb wraps each IDB object in a Proxy that forwards every property and wraps what comes back.
// Here each wrapped object is a class over the host object; its members call the host member and
// wrap the result the same way. A request's Future is completed by idb's own listener logic, so a
// request is issued when its method is called; a host exception is thrown at once, before any
// Future exists. Callbacks run on JSExecutionContext.queue, whose jobs are promise microtasks.

package idb

import scala.collection.mutable.ArrayBuffer
import scala.concurrent.{ExecutionContext, Future, Promise}
import scala.scalajs.concurrent.JSExecutionContext
import scala.scalajs.js

given ExecutionContext = JSExecutionContext.queue

/** Listeners added together and removed together, as idb's `unlisten` removes its own. */
private final class Listeners(target: EventTarget):
  private val added = ArrayBuffer.empty[(String, js.Function1[Event, Unit])]

  def on(kind: String)(listener: Event => Unit): Unit =
    val function: js.Function1[Event, Unit] = listener
    added += kind -> function
    target.addEventListener(kind, function)

  def remove(): Unit = for (kind, function) <- added do target.removeEventListener(kind, function)

/** A wrapped IndexedDB object; [[unwrap]] gives the host object behind it. */
abstract class Wrapped[+T](private[idb] val raw: T)

/** idb's `unwrap`. */
def unwrap[T](value: Wrapped[T]): T = value.raw

/** idb's `promisifyRequest`: completes once, with the request's result on `success` (wrapped by
  * `wrapResult`, as idb's `wrap(request.result)`) or with its error on `error`, then removes both
  * listeners. A cursor request fires once per move, so each move promisifies it again.
  */
private[idb] def promisify[T](request: IDBRequest)(wrapResult: Any => T): Future[T] =
  val settled = Promise[T]()
  val listeners = Listeners(request)
  listeners.on("success") { _ =>
    settled.success(wrapResult(request.result))
    listeners.remove()
  }
  listeners.on("error") { _ =>
    settled.failure(js.JavaScriptException(request.error))
    listeners.remove()
  }
  settled.future

/** idb's `wrap` of a request: a Future of its result, which is undefined for a miss. */
def wrap(request: IDBRequest): Future[Any] = promisify(request)(identity)

private def wrapCursor(request: IDBRequest): Future[Option[Cursor]] =
  promisify(request)(result => Option(result).map(cursor => Cursor(cursor.asInstanceOf[IDBCursor])))

/** idb's `cacheDonePromiseForTransaction`: completed by the first `complete`, `error` or `abort`.
  * A request error bubbles to the transaction before the transaction records it, so `done` then
  * fails with a new AbortError.
  */
private def cacheDone(tx: IDBTransaction): Future[Unit] =
  val settled = Promise[Unit]()
  val listeners = Listeners(tx)
  listeners.on("complete") { _ =>
    settled.success(())
    listeners.remove()
  }
  val failed = (_: Event) =>
    val error = if tx.error == null then DOMException("AbortError", "AbortError") else tx.error
    settled.failure(js.JavaScriptException(error))
    listeners.remove()
  listeners.on("error")(failed)
  listeners.on("abort")(failed)
  settled.future

/** The request methods idb promisifies on a store or an index, with database-extras'
  * `readMethods` and `writeMethods` split.
  */
enum Method(val jsName: String, val isWrite: Boolean):
  case Get extends Method("get", false)
  case GetKey extends Method("getKey", false)
  case GetAll extends Method("getAll", false)
  case GetAllKeys extends Method("getAllKeys", false)
  case Count extends Method("count", false)
  case Put extends Method("put", true)
  case Add extends Method("add", true)
  case Delete extends Method("delete", true)
  case Clear extends Method("clear", true)

  private[idb] def call(source: IDBSource, args: Seq[Any]): IDBRequest = this match
    case Get => source.get(args*)
    case GetKey => source.getKey(args*)
    case GetAll => source.getAll(args*)
    case GetAllKeys => source.getAllKeys(args*)
    case Count => source.count(args*)
    case Put => source.put(args*)
    case Add => source.add(args*)
    case Delete => source.delete(args*)
    case Clear => source.clear(args*)

object Method:
  def of(name: String): Option[Method] = values.find(_.jsName == name)

/** An object store or an index: the methods both have. */
sealed abstract class Source(raw: IDBSource) extends Wrapped[IDBSource](raw):
  def name: String = raw.name
  def keyPath: Any = raw.keyPath

  def request(method: Method, args: Any*): Future[Any] = wrap(method.call(raw, args))

  def openCursor(args: Any*): Future[Option[Cursor]] = wrapCursor(raw.openCursor(args*))

  def openKeyCursor(args: Any*): Future[Option[Cursor]] = wrapCursor(raw.openKeyCursor(args*))

  /** idb's async iterator, `openCursor(...args)` walked by [[CursorIterator.next]]. */
  def iterate(args: Any*): CursorIterator = CursorIterator(this, args)

final class ObjectStore private[idb] (store: IDBObjectStore) extends Source(store):
  def autoIncrement: Boolean = store.autoIncrement
  def indexNames: Seq[String] = names(store.indexNames)

  def index(name: String): Index = Index(store.index(name))

  def createIndex(name: String, keyPath: Any, options: js.UndefOr[Any] = js.undefined): Index =
    Index(store.createIndex(name, keyPath, options))

  def deleteIndex(name: String): Unit = store.deleteIndex(name)

final class Index private[idb] (index: IDBIndex) extends Source(index):
  def unique: Boolean = index.unique
  def multiEntry: Boolean = index.multiEntry

private def names(list: DOMStringList): Seq[String] = (0 until list.length).map(list.item)

/** A wrapped IDBCursor. Its moves are idb's cursor advance methods: they call the host method,
  * then promisify the cursor's request again, which gives this cursor or None past the end.
  */
final class Cursor private[idb] (raw: IDBCursor) extends Wrapped[IDBCursor](raw):
  def key: Any = raw.key
  def primaryKey: Any = raw.primaryKey
  def value: Any = raw.value

  def advance(args: Any*): Future[Option[Cursor]] =
    raw.advance(args*)
    wrapCursor(raw.request)

  def continue(args: Any*): Future[Option[Cursor]] =
    raw.continue(args*)
    wrapCursor(raw.request)

  def continuePrimaryKey(args: Any*): Future[Option[Cursor]] =
    raw.continuePrimaryKey(args*)
    wrapCursor(raw.request)

  def update(value: Any): Future[Any] = wrap(raw.update(value))

  def delete(): Future[Any] = wrap(raw.delete())

final class Transaction private[idb] (raw: IDBTransaction) extends Wrapped[IDBTransaction](raw):
  /** Created with the wrapper, as idb caches it when it first wraps the transaction, so it also
    * completes for a caller that only asks after the transaction finished.
    */
  val done: Future[Unit] = cacheDone(raw)

  /** idb's `tx.store`: the only store in the scope, or None when there are several. idb tests the
    * second name before it calls objectStore(), so on a finished transaction this throws
    * InvalidStateError for one store and gives None for several.
    */
  def store: Option[ObjectStore] =
    val storeNames = raw.objectStoreNames
    val second = storeNames.item(1)
    if second != null && second.nonEmpty then None else Some(objectStore(storeNames.item(0)))

  def objectStore(name: String): ObjectStore = ObjectStore(raw.objectStore(name))

  def abort(): Unit = raw.abort()

/** A database shortcut (database-extras' `getMethod`): a request method on the store, or with
  * `useIndex` (`getFromIndex`, …) on one of its indexes. Only read methods have index forms.
  */
final class Shortcut private (val method: Method, val useIndex: Boolean)

object Shortcut:
  def of(name: String): Option[Shortcut] =
    val target = name.stripSuffix("FromIndex")
    val useIndex = target != name
    Method.of(target).filter(method => !(useIndex && method.isWrite)).map(Shortcut(_, useIndex))

final class Database private[idb] (raw: IDBDatabase) extends Wrapped[IDBDatabase](raw):
  def version: Double = raw.version
  def objectStoreNames: Seq[String] = names(raw.objectStoreNames)

  def createObjectStore(name: String, options: js.UndefOr[Any] = js.undefined): ObjectStore =
    ObjectStore(raw.createObjectStore(name, options))

  def deleteObjectStore(name: String): Unit = raw.deleteObjectStore(name)

  /** `storeNames` is a name or an array of names; an undefined mode is readonly. */
  def transaction(storeNames: Any, mode: js.UndefOr[Any] = js.undefined): Transaction =
    Transaction(raw.transaction(storeNames, mode))

  def close(): Unit = raw.close()

  /** One shortcut call, as getMethod's async method: a transaction per call, `readwrite` for a
    * write, on `tx.store` or its index, and `Promise.all([request, isWrite && tx.done])`, so a
    * write completes only after `done` and the request's error comes first. Everything up to the
    * request happens now; a synchronous failure fails the Future.
    */
  def shortcut(shortcut: Shortcut, storeName: String, indexName: Option[String], args: Any*): Future[Any] =
    try
      val method = shortcut.method
      val tx = transaction(storeName, if method.isWrite then "readwrite" else "readonly")
      val store = tx.store.getOrElse(throw IllegalStateException("a one-store transaction has a store"))
      val target = if shortcut.useIndex then store.index(indexName.get) else store
      val request = target.request(method, args*)
      if method.isWrite then all(request, tx.done) else request
    catch case error: Throwable => Future.failed(error)

/** `Promise.all([request, done])`: the request's value once both complete, or the first failure
  * as it happens.
  */
private def all(request: Future[Any], done: Future[Unit]): Future[Any] =
  val settled = Promise[Any]()
  request.failed.foreach(settled.tryFailure)
  done.failed.foreach(settled.tryFailure)
  request.zip(done).foreach((value, _) => settled.trySuccess(value))
  settled.future

/** idb's `iterate` generator. The first [[next]] opens `openCursor(...args)`; each later one
  * awaits the move the loop body made on the yielded cursor, or calls `continue()` when it made
  * none. A failed step ends the iteration, as a throwing generator is done.
  */
final class CursorIterator private[idb] (source: Source, args: Seq[Any]):
  private var opened = false
  private var cursor: Option[Cursor] = None

  /** The move the loop body made: the iterator's `advanceResults` entry. */
  private[idb] var advanced: Option[Future[Option[Cursor]]] = None

  def next(): Future[Option[IteratedCursor]] =
    val step =
      try
        if !opened then
          opened = true
          Some(source.openCursor(args*))
        else
          val current = cursor
          cursor = None
          current.map(cursor => advanced.getOrElse(cursor.continue()))
      catch case error: Throwable => Some(Future.failed(error))
    step match
      case None => Future.successful(None)
      case Some(moved) =>
        moved.map { next =>
          advanced = None
          cursor = next
          next.map(IteratedCursor(this, _))
        }

/** The cursor an iteration yields (idb's proxied cursor): its moves call the host method and
  * record the result for the iterator instead of returning it.
  */
final class IteratedCursor private[idb] (iterator: CursorIterator, val cursor: Cursor):
  def advance(args: Any*): Unit = iterator.advanced = Some(cursor.advance(args*))
  def continue(args: Any*): Unit = iterator.advanced = Some(cursor.continue(args*))
  def continuePrimaryKey(args: Any*): Unit = iterator.advanced = Some(cursor.continuePrimaryKey(args*))

/** openDB's `upgrade(db, oldVersion, newVersion, transaction)`. */
type Upgrade = (Database, Double, Double | Null, Transaction) => Unit

/** openDB's `blocked` and `blocking`: `(currentVersion, blockedVersion, event)`. */
type VersionChange = (Double, Double | Null, IDBVersionChangeEvent) => Unit

/** idb's `openDB(name, version, { upgrade, blocked, blocking })`. An undefined version opens the
  * current version, or 1 for a new database; an invalid one throws here. `blocking` is attached to
  * the connection once the open succeeds, before the returned Future completes.
  */
def openDB(
    name: String,
    version: js.UndefOr[Any] = js.undefined,
    upgrade: Option[Upgrade] = None,
    blocked: Option[VersionChange] = None,
    blocking: Option[VersionChange] = None
): Future[Database] =
  val request = Globals.indexedDB.open(name, version)
  val openPromise = promisify(request)(db => Database(db.asInstanceOf[IDBDatabase]))
  for callback <- upgrade do
    request.addEventListener("upgradeneeded", { (event: Event) =>
      val change = event.asInstanceOf[IDBVersionChangeEvent]
      callback(Database(request.result.asInstanceOf[IDBDatabase]), change.oldVersion, change.newVersion, Transaction(request.transaction))
    })
  for callback <- blocked do
    request.addEventListener("blocked", { (event: Event) =>
      val change = event.asInstanceOf[IDBVersionChangeEvent]
      callback(change.oldVersion, change.newVersion, change)
    })
  openPromise.map { db =>
    for callback <- blocking do
      db.raw.addEventListener("versionchange", { (event: Event) =>
        val change = event.asInstanceOf[IDBVersionChangeEvent]
        callback(change.oldVersion, change.newVersion, change)
      })
    db
  }

/** idb's `deleteDB(name, { blocked })`; `blocked(currentVersion, event)`. */
def deleteDB(name: String, blocked: Option[(Double, IDBVersionChangeEvent) => Unit] = None): Future[Unit] =
  val request = Globals.indexedDB.deleteDatabase(name)
  for callback <- blocked do
    request.addEventListener("blocked", { (event: Event) =>
      val change = event.asInstanceOf[IDBVersionChangeEvent]
      callback(change.oldVersion, change)
    })
  wrap(request).map(_ => ())
