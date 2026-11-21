// The host APIs idb wraps: IndexedDB (the factory, requests, databases, transactions, object
// stores, indexes, cursors and key ranges) and DOM events, as Scala.js facades. A JS exception one
// of them throws reaches Scala as `js.JavaScriptException` holding the JS error (a DOMException or
// a TypeError), which keeps its JS name.

package idb

import scala.scalajs.js
import scala.scalajs.js.annotation.{JSGlobal, JSGlobalScope}

@js.native
trait Event extends js.Object:
  val target: js.Any = js.native

@js.native
trait EventTarget extends js.Object:
  def addEventListener(`type`: String, listener: js.Function1[Event, Unit]): Unit = js.native
  def removeEventListener(`type`: String, listener: js.Function1[Event, Unit]): Unit = js.native

@js.native
trait IDBVersionChangeEvent extends Event:
  val oldVersion: Double = js.native
  /** Null for a deletion. */
  val newVersion: Double | Null = js.native

@js.native
trait DOMStringList extends js.Object:
  val length: Int = js.native
  /** Null past the end. */
  def item(index: Int): String = js.native

@js.native
trait IDBRequest extends EventTarget:
  val result: Any = js.native
  val error: js.Any = js.native
  val transaction: IDBTransaction = js.native

@js.native
trait IDBFactory extends js.Object:
  /** An undefined version is absent: the current version, or 1 for a new database. */
  def open(name: String, version: js.UndefOr[Any]): IDBRequest = js.native
  def deleteDatabase(name: String): IDBRequest = js.native

@js.native
trait IDBDatabase extends EventTarget:
  val version: Double = js.native
  val objectStoreNames: DOMStringList = js.native
  def createObjectStore(name: String, options: js.UndefOr[Any]): IDBObjectStore = js.native
  def deleteObjectStore(name: String): Unit = js.native
  def transaction(storeNames: Any, mode: js.UndefOr[Any]): IDBTransaction = js.native
  def close(): Unit = js.native

@js.native
trait IDBTransaction extends EventTarget:
  val objectStoreNames: DOMStringList = js.native
  val error: js.Any = js.native
  def objectStore(name: String): IDBObjectStore = js.native
  def abort(): Unit = js.native

/** The request methods of IDBObjectStore and IDBIndex. idb passes their arguments on as it gets
  * them, so they are variadic here; a write method is undefined on an index, and calling it throws
  * a TypeError, as it does through idb's proxy.
  */
@js.native
trait IDBSource extends js.Object:
  val name: String = js.native
  val keyPath: Any = js.native
  def get(args: Any*): IDBRequest = js.native
  def getKey(args: Any*): IDBRequest = js.native
  def getAll(args: Any*): IDBRequest = js.native
  def getAllKeys(args: Any*): IDBRequest = js.native
  def count(args: Any*): IDBRequest = js.native
  def put(args: Any*): IDBRequest = js.native
  def add(args: Any*): IDBRequest = js.native
  def delete(args: Any*): IDBRequest = js.native
  def clear(args: Any*): IDBRequest = js.native
  def openCursor(args: Any*): IDBRequest = js.native
  def openKeyCursor(args: Any*): IDBRequest = js.native

@js.native
trait IDBObjectStore extends IDBSource:
  val autoIncrement: Boolean = js.native
  val indexNames: DOMStringList = js.native
  def index(name: String): IDBIndex = js.native
  def createIndex(name: String, keyPath: Any, options: js.UndefOr[Any]): IDBIndex = js.native
  def deleteIndex(name: String): Unit = js.native

@js.native
trait IDBIndex extends IDBSource:
  val unique: Boolean = js.native
  val multiEntry: Boolean = js.native

/** An IDBCursor; `value` is undefined on a key cursor. */
@js.native
trait IDBCursor extends js.Object:
  val key: Any = js.native
  val primaryKey: Any = js.native
  val value: Any = js.native
  val request: IDBRequest = js.native
  def advance(args: Any*): Unit = js.native
  def continue(args: Any*): Unit = js.native
  def continuePrimaryKey(args: Any*): Unit = js.native
  def update(value: Any): IDBRequest = js.native
  def delete(): IDBRequest = js.native

@js.native
@JSGlobal
object IDBKeyRange extends js.Object:
  def only(value: Any): js.Any = js.native
  def lowerBound(lower: Any, open: Any): js.Any = js.native
  def upperBound(upper: Any, open: Any): js.Any = js.native
  def bound(lower: Any, upper: Any, lowerOpen: Any, upperOpen: Any): js.Any = js.native

@js.native
@JSGlobal
class DOMException(message: String, name: String) extends js.Object

@js.native
trait Crypto extends js.Object:
  def randomUUID(): String = js.native

@js.native
@JSGlobalScope
object Globals extends js.Object:
  val indexedDB: IDBFactory = js.native
  val crypto: Crypto = js.native

/** A JS exception's `name`, as the log reads it. */
def jsErrorName(error: Throwable): String = error match
  case js.JavaScriptException(value) => value.asInstanceOf[js.Dynamic].name.asInstanceOf[js.UndefOr[String]].getOrElse("Error")
  case _ => "Error"
