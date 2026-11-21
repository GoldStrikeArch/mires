package explorer

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel
import scala.scalajs.js.Dynamic.{literal => obj}

object Main {
  private type Value = js.Dynamic

  private def items(value: Value): js.Array[Value] = value.asInstanceOf[js.Array[Value]]
  // One idb script (idb CONTRACT.md, "Input"): create the store, put every row in one readwrite
  // transaction, then walk the store with a cursor in primary-key order.
  private def persist(rows: js.Array[Value]): Value = {
    def transaction(mode: String, step: Value) =
      obj(op = "tx", conn = "explorer", stores = "rows", mode = mode, steps = js.Array(step), `catch` = false)
    val schema = js.Array(
      obj(op = "createStore", name = "rows", options = obj(keyPath = "id")),
      obj(op = "createIndex", store = "rows", name = "byGroup", keyPath = "group")
    )
    val openDb = obj(op = "openDB", db = "explorer", conn = "explorer", version = 1, upgrade = schema, `catch` = false)
    val puts = rows.map(row => obj(op = "put", store = null, index = null, args = js.Array(row), `catch` = false))
    val write = obj(op = "batch", requests = puts, abort = false, done = true)
    val walk = obj(
      op = "openCursor", store = null, index = null, args = js.Array(), positions = js.Array(),
      rest = js.Array(js.Array("continue")), `catch` = false
    )
    obj(steps = js.Array(openDb, transaction("readwrite", write), transaction("readonly", walk)))
  }
  // The stored rows' `value` texts, from the cursor entries of the idb log.
  private def stored(history: js.Array[Value]): js.Array[String] =
    items(history(0).log)
      .filter(entry => items(entry)(0).asInstanceOf[String] == "cursor")
      .map(entry => items(entry)(3).value.asInstanceOf[String])
  private def hits(history: js.Array[Value]): js.Array[Value] =
    items(items(history(1).results)(1).hits)
  private def command(name: String | Null, value: js.Any): js.Array[js.Any] =
    js.Array[js.Any](name.asInstanceOf[js.Any], value)

  @JSExportTopLevel("run")
  def run(packet: js.Array[Value]): js.Array[js.Any] = {
    val input = packet(0)
    val history = items(packet(1))
    history.length match {
      case 0 =>
        val rows: js.Array[Value] = items(input.records).map { doc =>
          obj(id = doc.id, group = "documents", value = js.JSON.stringify(doc))
        }
        command("idb", persist(rows))
      case 1 =>
        val options = obj(fields = js.Array("title", "body"), storeFields = js.Array("body"))
        val add = obj(op = "addAll", documents = stored(history).map(text => js.JSON.parse(text)))
        val query = obj(op = "search", query = input.query)
        command("minisearch", obj(options = options, operations = js.Array(add, query)))
      case 2 =>
        val result = hits(history)
        val markdown = if (result.isEmpty) "" else result(0).body.asInstanceOf[String]
        command("marked", obj(op = "parse", markdown = markdown))
      case 3 =>
        command("fflate", obj(op = "strToU8", text = stored(history).join("\n")))
      case 4 =>
        command("fflate", obj(op = "gzipSync", data = obj(bytes = history(3)), options = obj(level = 6, mtime = 0)))
      case 5 => command("uplot", input.chart)
      case _ =>
        command(null, js.Array[js.Any](hits(history).map(_.id), history(2), history(4), items(history(3)).length, history(5)))
    }
  }
}
