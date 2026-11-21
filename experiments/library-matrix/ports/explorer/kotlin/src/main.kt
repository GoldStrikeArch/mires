import kotlin.js.Json
import kotlin.js.json

// One idb script (idb CONTRACT.md, "Input"): create the store, put every row in one readwrite
// transaction, then walk the store with a cursor in primary-key order.
private fun persist(rows: Array<Json>): Json {
    fun transaction(mode: String, step: Json) = json(
        "op" to "tx", "conn" to "explorer", "stores" to "rows", "mode" to mode,
        "steps" to arrayOf(step), "catch" to false
    )
    val schema = arrayOf(
        json("op" to "createStore", "name" to "rows", "options" to json("keyPath" to "id")),
        json("op" to "createIndex", "store" to "rows", "name" to "byGroup", "keyPath" to "group")
    )
    val openDb = json(
        "op" to "openDB", "db" to "explorer", "conn" to "explorer", "version" to 1,
        "upgrade" to schema, "catch" to false
    )
    val puts = rows.map { row ->
        json("op" to "put", "store" to null, "index" to null, "args" to arrayOf(row), "catch" to false)
    }.toTypedArray()
    val write = json("op" to "batch", "requests" to puts, "abort" to false, "done" to true)
    val walk = json(
        "op" to "openCursor", "store" to null, "index" to null, "args" to emptyArray<Any>(),
        "positions" to emptyArray<Any>(), "rest" to arrayOf(arrayOf("continue")), "catch" to false
    )
    return json("steps" to arrayOf(openDb, transaction("readwrite", write), transaction("readonly", walk)))
}

// The stored rows' `value` texts, from the cursor entries of the idb log.
private fun stored(history: dynamic): List<String> {
    val log: Array<dynamic> = history[0].log
    return log.filter { entry -> entry[0] as String == "cursor" }.map { entry -> entry[3].value as String }
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
            command("idb", persist(rows))
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
