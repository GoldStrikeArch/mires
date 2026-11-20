package marked

class Parser(val options: MarkedOptions) {
    val renderer: Renderer = Renderer(options, this)
    val textRenderer: TextRenderer = TextRenderer()

    fun parse(tokens: JsArray<Token>): String {
        var out = ""
        for (i in 0 until tokens.length) {
            val token = tokens[i]
            out += when (token) {
                is Token.Space -> renderer.space(token)
                is Token.Hr -> renderer.hr(token)
                is Token.Heading -> renderer.heading(token)
                is Token.Code -> renderer.code(token)
                is Token.Table -> renderer.table(token)
                is Token.Blockquote -> renderer.blockquote(token)
                is Token.List -> renderer.list(token)
                is Token.Checkbox -> renderer.checkbox(token)
                is Token.Html -> renderer.html(token)
                is Token.Def -> renderer.def(token)
                is Token.Paragraph -> renderer.paragraph(token)
                is Token.Text -> renderer.text(token)
                else -> throw RuntimeException("Unexpected block token.")
            }
        }
        return out
    }

    fun parseInline(tokens: JsArray<Token>, r: InlineRenderer = renderer): String {
        var out = ""
        for (i in 0 until tokens.length) {
            val token = tokens[i]
            out += when (token) {
                is Token.Escape -> r.escape(token)
                is Token.Html -> r.html(token)
                is Token.Link -> r.link(token)
                is Token.Image -> r.image(token)
                is Token.Checkbox -> r.checkbox(token)
                is Token.Strong -> r.strong(token)
                is Token.Em -> r.em(token)
                is Token.Codespan -> r.codespan(token)
                is Token.Br -> r.br(token)
                is Token.Del -> r.del(token)
                is Token.Text -> r.text(token)
                else -> throw RuntimeException("Unexpected inline token.")
            }
        }
        return out
    }
}

class Marked(private val options: MarkedOptions) {
    fun parse(src: String): String {
        val tokens = Lexer(options).lex(src)
        return Parser(options).parse(tokens)
    }

    fun parseInline(src: String): String {
        val tokens = Lexer(options).inlineTokens(src)
        return Parser(options).parseInline(tokens)
    }
}
