package marked

class MarkedOptions(
    val gfm: Boolean = true,
    val breaks: Boolean = false,
    val pedantic: Boolean = false,
)

class LinkDef(
    val href: String,
    val title: String?,
)

class TableCell(
    val text: String,
    val tokens: JsArray<Token>,
    val header: Boolean,
    val align: String?,
)

class InlineQueueItem(
    var src: String,
    val tokens: JsArray<Token>,
)

class LexerState(
    var inLink: Boolean = false,
    var inRawBlock: Boolean = false,
    var linkEmitted: Boolean = false,
    var linkParenPossible: Boolean = true,
    var top: Boolean = true,
)

sealed class Token(var raw: String) {
    class Space(raw: String) : Token(raw)

    class Code(
        raw: String,
        val text: String,
        val lang: String? = null,
        val codeBlockStyle: String? = null,
        val escaped: Boolean = false,
    ) : Token(raw)

    class Heading(
        raw: String,
        val depth: Int,
        val text: String,
        val tokens: JsArray<Token>,
    ) : Token(raw)

    class Hr(raw: String) : Token(raw)

    class Blockquote(
        raw: String,
        val tokens: JsArray<Token>,
        val text: String,
    ) : Token(raw)

    class List(
        raw: String,
        val ordered: Boolean,
        val start: Int,
        var loose: Boolean,
        val items: JsArray<ListItem>,
    ) : Token(raw)

    class ListItem(
        raw: String,
        var task: Boolean,
        var checked: Boolean,
        var loose: Boolean,
        var text: String,
        var tokens: JsArray<Token>,
    ) : Token(raw)

    class Checkbox(
        raw: String,
        val checked: Boolean,
    ) : Token(raw)

    class Html(
        raw: String,
        val block: Boolean,
        val text: String,
        val pre: Boolean = false,
        val inLink: Boolean = false,
        val inRawBlock: Boolean = false,
    ) : Token(raw)

    class Def(
        val tag: String,
        raw: String,
        val href: String,
        val title: String?,
    ) : Token(raw)

    class Table(
        raw: String,
        val header: JsArray<TableCell>,
        val align: JsArray<String?>,
        val rows: JsArray<JsArray<TableCell>>,
    ) : Token(raw)

    class Paragraph(
        raw: String,
        var text: String,
        val tokens: JsArray<Token>,
    ) : Token(raw)

    class Text(
        raw: String,
        var text: String,
        val tokens: JsArray<Token>? = null,
        val escaped: Boolean = false,
    ) : Token(raw)

    class Escape(
        raw: String,
        val text: String,
    ) : Token(raw)

    class Link(
        raw: String,
        val href: String,
        val title: String?,
        val text: String,
        val tokens: JsArray<Token>,
        val autolink: Boolean = false,
    ) : Token(raw)

    class Image(
        raw: String,
        val href: String,
        val title: String?,
        val text: String,
        val tokens: JsArray<Token>?,
    ) : Token(raw)

    class Strong(
        raw: String,
        val text: String,
        val tokens: JsArray<Token>,
    ) : Token(raw)

    class Em(
        raw: String,
        val text: String,
        val tokens: JsArray<Token>,
    ) : Token(raw)

    class Codespan(
        raw: String,
        val text: String,
    ) : Token(raw)

    class Br(raw: String) : Token(raw)

    class Del(
        raw: String,
        val text: String,
        val tokens: JsArray<Token>,
    ) : Token(raw)
}
