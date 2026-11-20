import kotlin.js.json

private fun stored(history: dynamic): Array<String> {
    val cursor = history[0][3]
    return Array(cursor.length as Int) { index -> cursor[index][2] as String }
}

private fun command(component: String?, value: dynamic) = arrayOf<dynamic>(component, value)

@JsExport
fun run(packet: dynamic): dynamic {
    val input = packet[0]
    val history = packet[1]
    return when (history.length as Int) {
        0 -> {
            val documents = input.records
            val rows = Array(documents.length as Int) { i ->
                val document = documents[i]
                json("id" to document.id, "group" to "documents", "value" to JSON.stringify(document))
            }
            command("idb", json(
                "batches" to arrayOf(json("rows" to rows)), "keys" to emptyArray<String>(),
                "groups" to emptyArray<String>(), "step" to 1
            ))
        }
        1 -> {
            val options = json("fields" to arrayOf("title", "body"), "storeFields" to arrayOf("body"))
            val add = json("op" to "addAll", "documents" to stored(history).map { JSON.parse<dynamic>(it) }.toTypedArray())
            val search = json("op" to "search", "query" to input.query)
            command("minisearch", json("options" to options, "operations" to arrayOf(add, search)))
        }
        2 -> {
            val hits = history[1].results[1].hits
            val markdown = if (hits.length as Int == 0) "" else hits[0].body as String
            command("marked", json("op" to "parse", "markdown" to markdown))
        }
        3 -> command("fflate", json("op" to "strToU8", "text" to stored(history).joinToString("\n")))
        4 -> command("fflate", json(
            "op" to "gzipSync", "data" to json("bytes" to history[3]),
            "options" to json("level" to 6, "mtime" to 0)
        ))
        5 -> command("uplot", input.chart)
        else -> {
            val hits = history[1].results[1].hits
            val ids = Array(hits.length as Int) { i -> hits[i].id as String }
            command(null, arrayOf<dynamic>(ids, history[2], history[4], history[3].length, history[5]))
        }
    }
}
