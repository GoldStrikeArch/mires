package marked

interface InlineRenderer {
    fun strong(token: Token.Strong): String
    fun em(token: Token.Em): String
    fun codespan(token: Token.Codespan): String
    fun del(token: Token.Del): String
    fun html(token: Token.Html): String
    fun text(token: Token.Text): String
    fun escape(token: Token.Escape): String
    fun link(token: Token.Link): String
    fun image(token: Token.Image): String
    fun br(token: Token.Br): String
    fun checkbox(token: Token.Checkbox): String
}

class Renderer(
    val options: MarkedOptions,
    val parser: Parser,
) : InlineRenderer {
    fun space(@Suppress("UNUSED_PARAMETER") token: Token.Space): String = ""

    fun code(token: Token.Code): String {
        val lang = token.lang
        val langString = if (!lang.isNullOrBlankStr()) {
            OTHER_RULES.notSpaceStart.exec(lang.unsafeCast<String>())?.opt(0) ?: ""
        } else {
            ""
        }
        val code = if (token.text.length > 0) {
            token.text.asJs().replaceRe(OTHER_RULES.endingNewline, "") + "\n"
        } else {
            ""
        }

        if (langString.length == 0) {
            return "<pre><code>" +
                (if (token.escaped) code else escapeHtmlEntities(code, true)) +
                "</code></pre>\n"
        }

        return "<pre><code class=\"language-" +
            escapeHtmlEntities(langString) +
            "\">" +
            (if (token.escaped) code else escapeHtmlEntities(code, true)) +
            "</code></pre>\n"
    }

    fun blockquote(token: Token.Blockquote): String {
        val body = parser.parse(token.tokens)
        return "<blockquote>\n$body</blockquote>\n"
    }

    override fun html(token: Token.Html): String = token.text

    fun def(@Suppress("UNUSED_PARAMETER") token: Token.Def): String = ""

    fun heading(token: Token.Heading): String {
        return "<h${token.depth}>${parser.parseInline(token.tokens)}</h${token.depth}>\n"
    }

    fun hr(@Suppress("UNUSED_PARAMETER") token: Token.Hr): String = "<hr>\n"

    fun list(token: Token.List): String {
        val ordered = token.ordered
        val start = token.start
        var body = ""
        for (j in 0 until token.items.length) {
            body += listitem(token.items[j])
        }
        val type = if (ordered) "ol" else "ul"
        val startAttr = if (ordered && start != 1) " start=\"$start\"" else ""
        return "<$type$startAttr>\n$body</$type>\n"
    }

    fun listitem(item: Token.ListItem): String {
        return "<li>${parser.parse(item.tokens)}</li>\n"
    }

    override fun checkbox(token: Token.Checkbox): String {
        return "<input " +
            (if (token.checked) "checked=\"\" " else "") +
            "disabled=\"\" type=\"checkbox\"> "
    }

    fun paragraph(token: Token.Paragraph): String {
        return "<p>${parser.parseInline(token.tokens)}</p>\n"
    }

    fun table(token: Token.Table): String {
        var cell = ""
        for (j in 0 until token.header.length) {
            cell += tablecell(token.header[j])
        }
        val header = tablerow(cell)

        var body = ""
        for (j in 0 until token.rows.length) {
            val row = token.rows[j]
            cell = ""
            for (k in 0 until row.length) {
                cell += tablecell(row[k])
            }
            body += tablerow(cell)
        }
        if (body.length > 0) {
            body = "<tbody>$body</tbody>"
        }

        return "<table>\n<thead>\n$header</thead>\n$body</table>\n"
    }

    fun tablerow(text: String): String = "<tr>\n$text</tr>\n"

    fun tablecell(cell: TableCell): String {
        val content = parser.parseInline(cell.tokens)
        val type = if (cell.header) "th" else "td"
        val align = cell.align
        val tag = if (align != null) "<$type align=\"$align\">" else "<$type>"
        return "$tag$content</$type>\n"
    }

    override fun strong(token: Token.Strong): String = "<strong>${parser.parseInline(token.tokens)}</strong>"

    override fun em(token: Token.Em): String = "<em>${parser.parseInline(token.tokens)}</em>"

    override fun codespan(token: Token.Codespan): String = "<code>${escapeHtmlEntities(token.text, true)}</code>"

    override fun br(token: Token.Br): String = "<br>"

    override fun del(token: Token.Del): String = "<del>${parser.parseInline(token.tokens)}</del>"

    override fun link(token: Token.Link): String {
        val parsedText = if (token.autolink) {
            escapeHtmlEntities(token.text, true)
        } else {
            parser.parseInline(token.tokens)
        }
        val cleanHref = cleanUrl(token.href) ?: return parsedText
        val href = escapeHtmlEntities(cleanHref, token.autolink)
        var out = "<a href=\"$href\""
        val title = token.title
        if (!title.isNullOrBlankStr()) {
            out += " title=\"${escapeHtmlEntities(title.unsafeCast<String>())}\""
        }
        out += ">$parsedText</a>"
        return out
    }

    override fun image(token: Token.Image): String {
        val text = if (token.tokens != null) {
            parser.parseInline(token.tokens, parser.textRenderer)
        } else {
            token.text
        }
        val cleanHref = cleanUrl(token.href) ?: return escapeHtmlEntities(text)
        var out = "<img src=\"${escapeHtmlEntities(cleanHref)}\" alt=\"${escapeHtmlEntities(text)}\""
        val title = token.title
        if (!title.isNullOrBlankStr()) {
            out += " title=\"${escapeHtmlEntities(title.unsafeCast<String>())}\""
        }
        out += ">"
        return out
    }

    override fun text(token: Token.Text): String {
        return when {
            token.tokens != null -> parser.parseInline(token.tokens)
            token.escaped -> token.text
            else -> escapeHtmlEntities(token.text)
        }
    }

    override fun escape(token: Token.Escape): String = escapeHtmlEntities(token.text)
}

class TextRenderer : InlineRenderer {
    override fun strong(token: Token.Strong): String = token.text
    override fun em(token: Token.Em): String = token.text
    override fun codespan(token: Token.Codespan): String = token.text
    override fun del(token: Token.Del): String = token.text
    override fun html(token: Token.Html): String = token.text
    override fun text(token: Token.Text): String = token.text
    override fun escape(token: Token.Escape): String = token.text
    override fun link(token: Token.Link): String = token.text
    override fun image(token: Token.Image): String = token.text
    override fun br(token: Token.Br): String = ""
    override fun checkbox(token: Token.Checkbox): String = token.raw
}
