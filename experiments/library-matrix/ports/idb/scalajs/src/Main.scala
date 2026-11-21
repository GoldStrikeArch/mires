// Entry of the Scala.js port: what libraries/idb/reference.mjs does, on the port's idb. A case is a
// script of idb calls; each call works on databases of its own, named by a fresh prefix, and
// closes and deletes them before it settles, also after a failure.
//
// The boundary: the corpus input is the JS object itself, seen through @js.native facade traits
// (no copy). Keys, values, versions, counts, directions, modes and options stay the JS values they
// are (`Any`, `js.UndefOr` where a field may be absent) and reach IndexedDB unchanged. The result
// is a plain object { log, databases } of JS arrays. `run` is a native ES module export.

package idb

import scala.collection.mutable
import scala.concurrent.Future
import scala.scalajs.js
import scala.scalajs.js.JSConverters.*
import scala.scalajs.js.annotation.JSExportTopLevel
import scala.util.{Failure, Success, Try}

@js.native
trait Input extends js.Object:
  val steps: js.Array[Step] = js.native

/** A step of any kind; `op` says which of the traits below it is. */
@js.native
trait Step extends js.Object:
  val op: String = js.native
  val `catch`: js.UndefOr[Boolean] = js.native

@js.native
trait OpenStep extends Step:
  val db: String = js.native
  val conn: String = js.native
  val version: js.UndefOr[Any] = js.native
  val upgrade: js.UndefOr[js.Array[UpgradeStep]] = js.native
  val blocking: js.UndefOr[String] = js.native
  /** `"log"` or `{ close: conn }`. */
  val blocked: js.UndefOr[Any] = js.native

@js.native
trait CloseStep extends Step:
  val conn: String = js.native

@js.native
trait DeleteStep extends Step:
  val db: String = js.native
  val blocked: js.UndefOr[Any] = js.native

@js.native
trait BlockedClose extends js.Object:
  val close: String = js.native

@js.native
trait TxStep extends Step:
  val conn: String = js.native
  val stores: Any = js.native
  val mode: Any = js.native
  val steps: js.Array[Step] = js.native

@js.native
trait CallStep extends Step:
  val conn: String = js.native
  val method: String = js.native
  val store: String = js.native
  val index: String | Null = js.native
  val args: js.Array[Any] = js.native

/** A request on `store` (null for `tx.store`) or its `index`. */
@js.native
trait RequestStep extends Step:
  val store: String | Null = js.native
  val index: String | Null = js.native
  val args: js.Array[Any] = js.native

@js.native
trait WrapStep extends RequestStep:
  val method: String = js.native

@js.native
trait BatchStep extends Step:
  val requests: js.Array[RequestStep] = js.native
  val abort: Boolean = js.native
  val done: Boolean = js.native

/** openCursor, openKeyCursor or iterate; each action is `[name, ...values]`. */
@js.native
trait WalkStep extends RequestStep:
  val positions: js.Array[js.Array[js.Array[Any]]] = js.native
  val rest: js.Array[js.Array[Any]] = js.native

@js.native
trait UpgradeStep extends js.Object:
  val op: String = js.native
  val name: String = js.native
  val options: js.UndefOr[Any] = js.native
  val store: String = js.native
  val keyPath: Any = js.native
  val args: js.Array[Any] = js.native

@js.native
trait KeyRangeSpec extends js.Object:
  val only: Any = js.native
  val lower: Any = js.native
  val upper: Any = js.native
  val lowerOpen: Any = js.native
  val upperOpen: Any = js.native

/** A failure that the step where it happened has already logged. */
final class Failed extends Exception

/** An input outside the contract (CONTRACT.md, "Parity domain"): it rejects the call after
  * cleanup, with a TypeError, and is never logged.
  */
final class OutOfContract(message: String) extends Exception(message)

private val Upgrades = Set("createStore", "deleteStore", "createIndex", "deleteIndex", "put", "add", "abort")

/** The methods whose first argument is a query: a key, or a key range written as an object. */
private val QueryFirst = Set("get", "getKey", "getAll", "getAllKeys", "count", "delete", "openCursor", "openKeyCursor", "iterate")

/** A log entry. */
private def entry(values: Any*): js.Array[Any] = values.toJSArray

/** `[label, "ok"]` when the result is undefined, else `[label, "ok", value]`. */
private def ok(label: String, value: Any): js.Array[Any] =
  if js.isUndefined(value) then entry(label, "ok") else entry(label, "ok", value)

private def requestMethod(op: String): Method = Method.of(op).getOrElse(throw OutOfContract("not a request method: " + op))

/** Runs `body` now; a synchronous throw becomes a failed Future, as in an async function. */
private def now[T](body: => Future[T]): Future[T] =
  try body
  catch case error: Throwable => Future.failed(error)

/** The query methods take a key, or a key range written as an object, first. */
private def args(method: String, list: js.Array[Any]): Seq[Any] =
  if QueryFirst(method) && list.nonEmpty then query(list(0)) +: list.toSeq.drop(1) else list.toSeq

/** A key is passed as it is, null included. Presence is tested with `in`, so `{ only: null }` is
  * only(null); an absent open flag is undefined, which IndexedDB reads as false.
  */
private def query(value: Any): Any =
  if value == null || js.typeOf(value) != "object" || js.Array.isArray(value) then value
  else
    val range = value.asInstanceOf[KeyRangeSpec]
    val has = (key: String) => js.Object.hasProperty(range, key)
    if has("only") then IDBKeyRange.only(range.only)
    else if has("lower") && has("upper") then IDBKeyRange.bound(range.lower, range.upper, range.lowerOpen, range.upperOpen)
    else if has("lower") then IDBKeyRange.lowerBound(range.lower, range.lowerOpen)
    else IDBKeyRange.upperBound(range.upper, range.upperOpen)

private def actions(step: WalkStep, position: Int): js.Array[js.Array[Any]] =
  if position < step.positions.length then step.positions(position) else step.rest

private def isWrite(action: js.Array[Any]) = action(0) == "update" || action(0) == "delete"

/** One move made at a walk's position: the Future it returned, and the move's name. */
private final case class Move(label: String, next: Future[Option[Cursor]])

/** `values(i)`, or undefined past the end. */
private def argument(values: js.Array[Any], i: Int): Any = if i < values.length then values(i) else js.undefined

private final class Session(input: Input):
  private val prefix = "mires-matrix-idb-" + Globals.crypto.randomUUID() + "-"
  private val log = mutable.ArrayBuffer.empty[js.Array[Any]]
  private val connections = mutable.Map.empty[String, Database]
  private val opened = mutable.ArrayBuffer.empty[Database]
  private val attempted = mutable.ArrayBuffer.empty[String]
  private val live = mutable.ArrayBuffer.empty[String]

  private def remember(names: mutable.ArrayBuffer[String], name: String): Unit =
    if !names.contains(name) then names += name

  def run(): Future[js.Object] =
    // A logged failure ends the script there; anything else rejects the call.
    val script = inOrder(input.steps.toSeq)(perform(_, None)).map(_ => ()).recover { case _: Failed => () }
    val output = script.flatMap(_ => inOrder(live.toSeq)(dump)).map { databases =>
      js.Dynamic.literal(log = log.toJSArray, databases = databases.toJSArray)
    }
    // As the reference's `finally`: cleanup always runs, and its failure replaces the result.
    output.transformWith(result => cleanup().flatMap(_ => Future.fromTry(result)))

  /** Runs `each` on the items one after another, collecting the results. */
  private def inOrder[A, B](items: Seq[A])(each: A => Future[B]): Future[Seq[B]] =
    items.foldLeft(Future.successful(Vector.empty[B])) { (done, item) =>
      done.flatMap(results => now(each(item)).map(results :+ _))
    }

  private def cleanup(): Future[Unit] =
    opened.foreach(_.close())
    inOrder(attempted.toSeq)(name => deleteDB(prefix + name)).map(_ => ())

  /** Logs a failure where it happens; the Failed that propagates says it is logged. */
  private def failed(label: String, error: Throwable): Throwable = error match
    case outside: OutOfContract => outside
    case _ =>
      log += entry(label, "error", jsErrorName(error))
      Failed()

  /** One call: logs `[label, "ok", shown]` (just `[label, "ok"]` when `show` gives undefined) or
    * `[label, "error", name]`.
    */
  private def attempt[T](label: String, show: T => Any)(action: => Future[T]): Future[T] =
    now(action).transform {
      case Success(value) =>
        log += ok(label, show(value))
        Success(value)
      case Failure(error) => Failure(failed(label, error))
    }

  /** A call whose result is a host value, undefined for a miss. */
  private def attempt(label: String)(action: => Future[Any]): Future[Unit] = attempt[Any](label, identity)(action).map(_ => ())

  /** A call with no result to show. */
  private def attemptUnit(label: String)(action: => Unit): Future[Unit] =
    attempt[Unit](label, _ => js.undefined)(Future.successful(action))

  private def connection(conn: String): Database =
    connections.getOrElse(conn, throw OutOfContract("no connection named " + conn))

  private def inside(tx: Option[Transaction]): Transaction =
    tx.getOrElse(throw OutOfContract("a transaction step outside a transaction"))

  private def source(tx: Option[Transaction], step: RequestStep): Source =
    val transaction = inside(tx)
    val store = step.store match
      case null => transaction.store.getOrElse(throw OutOfContract("store: null in a transaction with several stores"))
      case name: String => transaction.objectStore(name)
    step.index match
      case null => store
      case name: String => store.index(name)

  private def perform(step: Step, tx: Option[Transaction]): Future[Unit] =
    now(execute(step, tx)).recover { case failure: Failed if step.`catch`.contains(true) => () }

  private def execute(step: Step, tx: Option[Transaction]): Future[Unit] = step.op match
    case "openDB" => open(step.asInstanceOf[OpenStep])
    case "close" => attemptUnit("close")(connection(step.asInstanceOf[CloseStep].conn).close())
    case "deleteDB" => remove(step.asInstanceOf[DeleteStep])
    case "tx" => transaction(step.asInstanceOf[TxStep])
    case "call" => shortcut(step.asInstanceOf[CallStep])
    case "batch" => batch(step.asInstanceOf[BatchStep], inside(tx))
    case "openCursor" | "openKeyCursor" => walk(step.asInstanceOf[WalkStep], tx)
    case "iterate" => iterate(step.asInstanceOf[WalkStep], tx)
    case "abort" => attemptUnit("abort")(inside(tx).abort())
    case "done" => attempt[Unit]("done", _ => js.undefined)(inside(tx).done).map(_ => ())
    case "store" => attempt("store")(Future.successful(inside(tx).store.map[Any](_.name).getOrElse(js.undefined)))
    case "wrap" =>
      val request = step.asInstanceOf[WrapStep]
      val method = requestMethod(request.method)
      attempt("wrap")(idb.wrap(method.call(unwrap(source(tx, request)), args(method.jsName, request.args))))
    case op =>
      val request = step.asInstanceOf[RequestStep]
      val method = requestMethod(op)
      attempt(op)(source(tx, request).request(method, args(op, request.args)*))

  private def closingConnection(blocked: js.UndefOr[Any]): Option[Database] =
    blocked.toOption.filter(js.typeOf(_) == "object").map(close => connection(close.asInstanceOf[BlockedClose].close))

  private def open(step: OpenStep): Future[Unit] =
    val upgrades = step.upgrade.toOption
    for ops <- upgrades; op <- ops.find(op => !Upgrades(op.op)) do throw OutOfContract("not an upgrade step: " + op.op)
    remember(attempted, step.db)
    val closing = closingConnection(step.blocked)
    val upgrade = upgrades.map { ops => (db: Database, oldVersion: Double, newVersion: Double | Null, transaction: Transaction) =>
      log += entry("upgrade", oldVersion, newVersion)
      ops.foreach(op => runUpgrade(op, db, transaction))
    }
    val blocked = step.blocked.toOption.map { _ => (currentVersion: Double, blockedVersion: Double | Null, _: IDBVersionChangeEvent) =>
      log += entry("blocked", currentVersion, blockedVersion)
      closing.foreach(_.close())
    }
    val blocking = step.blocking.toOption.map { kind => (currentVersion: Double, blockedVersion: Double | Null, event: IDBVersionChangeEvent) =>
      log += entry("blocking", step.conn, currentVersion, blockedVersion)
      if kind == "close" then event.target.asInstanceOf[IDBDatabase].close()
    }
    attempt[Database]("openDB", db => entry(db.version, db.objectStoreNames.toJSArray)) {
      openDB(prefix + step.db, step.version, upgrade, blocked, blocking)
    }.map { db =>
      connections(step.conn) = db
      opened += db
      remember(live, step.db)
    }

  private def remove(step: DeleteStep): Future[Unit] =
    remember(attempted, step.db)
    val closing = closingConnection(step.blocked)
    val blocked = step.blocked.toOption.map { _ => (currentVersion: Double, _: IDBVersionChangeEvent) =>
      log += entry("blocked", currentVersion)
      closing.foreach(_.close())
    }
    attempt[Unit]("deleteDB", _ => js.undefined)(deleteDB(prefix + step.db, blocked)).map(_ => live -= step.db)

  /** Runs synchronously inside the upgrade callback; a failure is logged and the callback goes on. */
  private def runUpgrade(op: UpgradeStep, db: Database, transaction: Transaction): Unit =
    try
      op.op match
        case "createStore" => log += entry("createStore", "ok", db.createObjectStore(op.name, op.options).name)
        case "deleteStore" =>
          db.deleteObjectStore(op.name)
          log += entry("deleteStore", "ok")
        case "createIndex" => log += entry("createIndex", "ok", transaction.objectStore(op.store).createIndex(op.name, op.keyPath, op.options).name)
        case "deleteIndex" =>
          transaction.objectStore(op.store).deleteIndex(op.name)
          log += entry("deleteIndex", "ok")
        case "abort" =>
          transaction.abort()
          log += entry("abort", "ok")
        // put or add: issued, not awaited. A failed write aborts the upgrade, which rejects the open.
        case write => transaction.objectStore(op.store).request(requestMethod(write), op.args.toSeq*)
    catch case error: Throwable => log += entry(op.op, "error", jsErrorName(error))

  private def transaction(step: TxStep): Future[Unit] =
    val tx =
      try connection(step.conn).transaction(step.stores, step.mode)
      catch case error: Throwable => throw failed("tx", error)
    inOrder(step.steps.toSeq)(perform(_, Some(tx))).map(_ => ())

  private def shortcut(step: CallStep): Future[Unit] =
    val shortcut = Shortcut.of(step.method).getOrElse(throw OutOfContract("not a database shortcut: " + step.method))
    val index = Option(step.index).map(_.asInstanceOf[String])
    if shortcut.useIndex != index.isDefined then throw OutOfContract("an index goes with the FromIndex forms only")
    attempt(step.method) {
      connection(step.conn).shortcut(shortcut, step.store, index, args(shortcut.method.jsName, step.args)*)
    }

  /** Issues every request before awaiting any, then logs each outcome in issue order. */
  private def batch(step: BatchStep, tx: Transaction): Future[Unit] =
    val requests = step.requests.toSeq
    val methods = requests.map(request => requestMethod(request.op))
    // Each issue is the reference's async callback: a synchronous failure is a failed Future.
    val pending = requests.zip(methods).map((request, method) => now(source(Some(tx), request).request(method, args(request.op, request.args)*)))
    val aborted = if step.abort then attemptUnit("abort")(tx.abort()).recover { case _: Failed => () } else Future.unit
    val labelled = requests.map(_.op).zip(pending) ++ (if step.done then Seq("done" -> tx.done.map(_ => js.undefined)) else Nil)
    // Promise.allSettled: every outcome, in issue order.
    val settled = Future.sequence(labelled.map((label, outcome) => outcome.transform(result => Success(label -> result))))
    for
      _ <- aborted
      outcomes <- settled
    yield
      outcomes.collectFirst { case (_, Failure(outside: OutOfContract)) => outside }.foreach(outside => throw outside)
      for (label, outcome) <- outcomes do
        log += (outcome match
          case Success(value) => ok(label, value)
          case Failure(error) => entry(label, "error", jsErrorName(error)))

  private def position(step: WalkStep, cursor: Cursor): Unit =
    log += (if step.op == "openKeyCursor" then entry("cursor", cursor.key, cursor.primaryKey) else entry("cursor", cursor.key, cursor.primaryKey, cursor.value))

  /** `cursor.update(value)` and `cursor.delete()`, awaited and logged. */
  private def write(cursor: Cursor, action: js.Array[Any]): Future[Unit] =
    if action(0) == "update" then attempt("update")(cursor.update(argument(action, 1))) else attempt("delete")(cursor.delete())

  /** A walk with the promise-returning moves; a position without a move ends it. */
  private def walk(step: WalkStep, tx: Option[Transaction]): Future[Unit] =
    val opening = now {
      val from = source(tx, step)
      val arguments = args(step.op, step.args)
      if step.op == "openKeyCursor" then from.openKeyCursor(arguments*) else from.openCursor(arguments*)
    }.recover { case error => throw failed(step.op, error) }

    // The position's actions, one at a time: the last move, or None after a break or without one,
    // which both end the walk.
    def act(cursor: Cursor, pending: List[js.Array[Any]], moved: Option[Move]): Future[Option[Move]] = pending match
      case Nil => Future.successful(moved)
      case action :: _ if action(0) == "break" => Future.successful(None)
      case action :: rest if isWrite(action) => write(cursor, action).flatMap(_ => act(cursor, rest, moved))
      case action :: rest => act(cursor, rest, Some(move(cursor, action)))

    def visit(cursor: Option[Cursor], at: Int): Future[Unit] = cursor match
      case None =>
        log += entry("end")
        Future.unit
      case Some(current) =>
        position(step, current)
        act(current, actions(step, at).toList, None).flatMap {
          case Some(Move(label, next)) => next.recover { case error => throw failed(label, error) }.flatMap(visit(_, at + 1))
          case None => Future.unit
        }

    opening.flatMap(visit(_, 0))

  private def move(cursor: Cursor, action: js.Array[Any]): Move =
    val name = action(0).asInstanceOf[String]
    val values = action.toSeq.drop(1)
    try
      Move(name, name match
        case "continue" => cursor.continue(values*)
        case "advance" => cursor.advance(values*)
        case "continuePrimaryKey" => cursor.continuePrimaryKey(values*)
        case _ => throw OutOfContract("not a cursor action: " + name))
    catch case error: Throwable => throw failed(name, error)

  /** The async iterator: a move only records the next step, and a position without one continues. */
  private def iterate(step: WalkStep, tx: Option[Transaction]): Future[Unit] =
    val iterator =
      try source(tx, step).iterate(args("iterate", step.args)*)
      catch case error: Throwable => throw failed("iterate", error)

    // The position's actions, one at a time; false when one of them is a break.
    def act(cursor: IteratedCursor, pending: List[js.Array[Any]]): Future[Boolean] = pending match
      case Nil => Future.successful(true)
      case action :: _ if action(0) == "break" => Future.successful(false)
      case action :: rest if isWrite(action) => write(cursor.cursor, action).flatMap(_ => act(cursor, rest))
      case action :: rest =>
        intercepted(cursor, action)
        act(cursor, rest)

    def loop(at: Int): Future[Unit] =
      iterator.next().recover { case error => throw failed("iterate", error) }.flatMap {
        case None =>
          log += entry("end")
          Future.unit
        case Some(cursor) =>
          log += entry("cursor", cursor.cursor.key, cursor.cursor.primaryKey, cursor.cursor.value)
          act(cursor, actions(step, at).toList).flatMap(continues => if continues then loop(at + 1) else Future.unit)
      }

    loop(0)

  private def intercepted(cursor: IteratedCursor, action: js.Array[Any]): Unit =
    val name = action(0).asInstanceOf[String]
    val values = action.toSeq.drop(1)
    try
      name match
        case "continue" => cursor.continue(values*)
        case "advance" => cursor.advance(values*)
        case "continuePrimaryKey" => cursor.continuePrimaryKey(values*)
        case _ => throw OutOfContract("not a cursor action: " + name)
    catch case error: Throwable => throw failed(name, error)

  /** `[name, version, stores]`, each store `[name, keyPath, autoIncrement, indexes, records]`, read
    * in one readonly transaction; records are getAllKeys() zipped with getAll().
    */
  private def dump(name: String): Future[js.Array[Any]] =
    openDB(prefix + name).flatMap { db =>
      val storeNames = db.objectStoreNames
      val stores =
        if storeNames.isEmpty then Future.successful(Seq.empty)
        else
          now {
            val tx = db.transaction(storeNames.toJSArray)
            inOrder(storeNames) { storeName =>
              val store = tx.objectStore(storeName)
              val indexes = store.indexNames.map { indexName =>
                val index = store.index(indexName)
                js.Array[Any](indexName, index.keyPath, index.unique, index.multiEntry)
              }
              for
                keys <- store.request(Method.GetAllKeys)
                values <- store.request(Method.GetAll)
              yield
                val records = keys.asInstanceOf[js.Array[Any]].zip(values.asInstanceOf[js.Array[Any]]).map((key, value) => entry(key, value))
                js.Array[Any](storeName, store.keyPath, store.autoIncrement, indexes.toJSArray, records)
            }.flatMap(stores => tx.done.map(_ => stores))
          }
      stores.transform { result =>
        db.close()
        result.map(stores => js.Array[Any](name, db.version, stores.toJSArray))
      }
    }

@JSExportTopLevel("run")
def run(input: Input): js.Promise[js.Object] =
  now(Session(input).run())
    .recover { case outside: OutOfContract => throw js.JavaScriptException(js.TypeError(outside.getMessage)) }
    .toJSPromise
