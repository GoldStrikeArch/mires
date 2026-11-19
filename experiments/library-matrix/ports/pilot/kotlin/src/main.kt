// Entry of the Kotlin/JS port. The boundary: the corpus input is a JS object typed by an external
// interface, so `text` is the JS string itself and `top` a JS number read as Int; the String
// result is a JS string. `run` is a native ES module export through @JsExport.

external interface Input {
    val text: String
    val top: Int
}

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: Input): String = analyse(input.text, input.top)
