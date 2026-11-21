// Entry of the Kotlin/JS port: what libraries/jmespath/reference.mjs does, on the port's jmespath.
//
// The boundary: the corpus input is a JS object typed by external interfaces and read in place.
// The document stays the JS value it is (jmespath returns parts of it), each expression is a JS
// string, and each result is a plain object `{ value }` or `{ error }` with the reference's error
// category. `run` is a native ES module export through @JsExport.

import jmespath.ErrorCategory
import jmespath.JmesPathException
import jmespath.Parser
import jmespath.TreeInterpreter
import jmespath.isStackExhaustion
import kotlin.js.Json
import kotlin.js.json

external interface Input {
    val data: Any?
    val queries: Array<Query>
}

external interface Query {
    val op: String
    val expression: String
}

/** jmespath's `compile`. */
private fun compile(expression: String): Any? = Parser().parse(expression)

/** jmespath's `search`: a fresh parser and interpreter per call. */
private fun search(data: Any?, expression: String): Any? = TreeInterpreter().search(Parser().parse(expression), data)

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: Input): Array<Json> = input.queries.map { query ->
    try {
        json("value" to if (query.op == "compile") compile(query.expression) else search(input.data, query.expression))
    } catch (e: JmesPathException) {
        json("error" to e.category.label)
    } catch (e: Throwable) {
        // An exhausted stack propagates, as it does from the reference adapter.
        if (e.isStackExhaustion()) throw e
        // The reference's category() default: JSON.parse's SyntaxError and engine errors.
        json("error" to ErrorCategory.Syntax.label)
    }
}.toTypedArray()
