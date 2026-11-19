// Floor port (Kotlin/JS): an external interface and a native ES module export.
external interface Input {
    val text: String
}

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: Input): String = input.text
