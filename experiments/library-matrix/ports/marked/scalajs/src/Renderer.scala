// HTML Renderer and TextRenderer ported directly from marked 18.1.1
// (src/Renderer.ts and src/TextRenderer.ts).

package marked

import scala.scalajs.js
import JsInterop.*

final class Renderer(val options: MarkedOptions, val parser: Parser):
  def space(): String = ""

  def code(token: CodeToken): String =
    val langRaw = if token.lang != null then token.lang.asInstanceOf[String] else ""
    val m = Rules.notSpaceStart.exec(langRaw)
    val langString = if m != null then undefStr(m(0)) else ""
    val codeStr =
      if token.text.length > 0 then token.text.jsReplaceRe(Rules.endingNewline, "") + "\n"
      else ""
    val renderedCode = if token.escaped then codeStr else Helpers.escapeHtmlEntities(codeStr, true)
    if langString.isEmpty then "<pre><code>" + renderedCode + "</code></pre>\n"
    else
      "<pre><code class=\"language-" + Helpers.escapeHtmlEntities(langString, false) + "\">" +
        renderedCode + "</code></pre>\n"

  def blockquote(token: BlockquoteToken): String =
    "<blockquote>\n" + parser.parse(token.tokens) + "</blockquote>\n"

  def html(token: HtmlToken): String = token.text

  def defToken(): String = ""

  def heading(token: HeadingToken): String =
    "<h" + token.depth + ">" + parser.parseInline(token.tokens) + "</h" + token.depth + ">\n"

  def hr(): String = "<hr>\n"

  def list(token: ListToken): String =
    var body = ""
    var j = 0
    val len = token.items.length
    while j < len do
      body += listitem(token.items(j))
      j += 1
    val tag = if token.ordered then "ol" else "ul"
    val startAttr = if token.ordered && token.start != 1 then " start=\"" + token.start + "\"" else ""
    "<" + tag + startAttr + ">\n" + body + "</" + tag + ">\n"

  def listitem(item: ListItemToken): String =
    "<li>" + parser.parse(item.tokens) + "</li>\n"

  def checkbox(token: CheckboxToken): String =
    "<input " + (if token.checked then "checked=\"\" " else "") + "disabled=\"\" type=\"checkbox\"> "

  def paragraph(token: ParagraphToken): String =
    "<p>" + parser.parseInline(token.tokens) + "</p>\n"

  def table(token: TableToken): String =
    var cell = ""
    var j = 0
    val hLen = token.header.length
    while j < hLen do
      cell += tablecell(token.header(j))
      j += 1
    val header = tablerow(cell)

    var body = ""
    j = 0
    val rLen = token.rows.length
    while j < rLen do
      val row = token.rows(j)
      cell = ""
      var k = 0
      val cLen = row.length
      while k < cLen do
        cell += tablecell(row(k))
        k += 1
      body += tablerow(cell)
      j += 1

    if body.length > 0 then body = "<tbody>" + body + "</tbody>"
    "<table>\n<thead>\n" + header + "</thead>\n" + body + "</table>\n"

  def tablerow(text: String): String =
    "<tr>\n" + text + "</tr>\n"

  def tablecell(cell: TableCell): String =
    val content = parser.parseInline(cell.tokens)
    val tag = if cell.header then "th" else "td"
    val open =
      if cell.align != null then "<" + tag + " align=\"" + cell.align.asInstanceOf[String] + "\">"
      else "<" + tag + ">"
    open + content + "</" + tag + ">\n"

  def strong(token: StrongToken): String =
    "<strong>" + parser.parseInline(token.tokens) + "</strong>"

  def em(token: EmToken): String =
    "<em>" + parser.parseInline(token.tokens) + "</em>"

  def codespan(token: CodespanToken): String =
    "<code>" + Helpers.escapeHtmlEntities(token.text, true) + "</code>"

  def br(): String = "<br>"

  def del(token: DelToken): String =
    "<del>" + parser.parseInline(token.tokens) + "</del>"

  def link(token: LinkToken): String =
    val parsedText =
      if token.autolink then Helpers.escapeHtmlEntities(token.text, true)
      else parser.parseInline(token.tokens)
    val cleanHref = Helpers.cleanUrl(token.href)
    if cleanHref == null then return parsedText
    val href = Helpers.escapeHtmlEntities(cleanHref.asInstanceOf[String], token.autolink)
    var out = "<a href=\"" + href + "\""
    if token.title != null && token.title.asInstanceOf[String].length > 0 then
      out += " title=\"" + Helpers.escapeHtmlEntities(token.title.asInstanceOf[String], false) + "\""
    out + ">" + parsedText + "</a>"

  def image(token: ImageToken): String =
    val alt =
      if token.tokens != null then parser.parseInlineText(token.tokens.asInstanceOf[js.Array[Token]])
      else token.text
    val cleanHref = Helpers.cleanUrl(token.href)
    if cleanHref == null then return Helpers.escapeHtmlEntities(alt, false)
    var out =
      "<img src=\"" + Helpers.escapeHtmlEntities(cleanHref.asInstanceOf[String], false) +
        "\" alt=\"" + Helpers.escapeHtmlEntities(alt, false) + "\""
    if token.title != null && token.title.asInstanceOf[String].length > 0 then
      out += " title=\"" + Helpers.escapeHtmlEntities(token.title.asInstanceOf[String], false) + "\""
    out + ">"

  def text(token: TextToken): String =
    if token.tokens != null then parser.parseInline(token.tokens.asInstanceOf[js.Array[Token]])
    else if token.escaped then token.text
    else Helpers.escapeHtmlEntities(token.text, false)

  def escapeText(token: EscapeToken): String =
    Helpers.escapeHtmlEntities(token.text, false)

object TextRenderer:
  def renderToken(token: Token): String =
    token match
      case t: StrongToken   => t.text
      case t: EmToken       => t.text
      case t: CodespanToken => t.text
      case t: DelToken      => t.text
      case t: HtmlToken     => t.text
      case t: TextToken     => t.text
      case t: EscapeToken   => t.text
      case t: LinkToken     => t.text
      case t: ImageToken    => t.text
      case _: BrToken       => ""
      case t: CheckboxToken => t.raw
      case _                => throw new RuntimeException("Token type not supported in TextRenderer")
