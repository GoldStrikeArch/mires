import marked.Marked
import marked.MarkedOptions

external interface OptionsJson {
    val gfm: Boolean?
    val breaks: Boolean?
    val pedantic: Boolean?
}

external interface InputJson {
    val op: String
    val markdown: String
    val options: OptionsJson?
}

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: InputJson): String {
    val optsJson = input.options
    val options = MarkedOptions(
        gfm = optsJson?.gfm ?: true,
        breaks = optsJson?.breaks ?: false,
        pedantic = optsJson?.pedantic ?: false,
    )
    return when (input.op) {
        "parse" -> Marked(options).parse(input.markdown)
        "parseInline" -> Marked(options).parseInline(input.markdown)
        else -> throw IllegalArgumentException("unknown operation ${input.op}")
    }
}
