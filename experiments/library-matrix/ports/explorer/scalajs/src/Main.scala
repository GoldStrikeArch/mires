package explorer

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel
import scala.scalajs.js.Dynamic.{literal => obj}

object Main {
  private type Value = js.Dynamic

  private def items(value: Value): js.Array[Value] = value.asInstanceOf[js.Array[Value]]
  private def stored(history: js.Array[Value]): js.Array[String] =
    items(items(history(0))(3)).map(row => items(row)(2).asInstanceOf[String])
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
        val rows = items(input.records).map { doc =>
          obj(id = doc.id, group = "documents", value = js.JSON.stringify(doc))
        }
        command("idb", obj(batches = js.Array(obj(rows = rows)), keys = js.Array(), groups = js.Array(), step = 1))
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
