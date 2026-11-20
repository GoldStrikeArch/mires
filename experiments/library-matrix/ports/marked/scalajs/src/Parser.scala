// Token Parser ported directly from marked 18.1.1 (src/Parser.ts).

package marked

import scala.scalajs.js

final class Parser(val options: MarkedOptions):
  val renderer: Renderer = new Renderer(options, this)

  def parse(tokens: js.Array[Token]): String =
    var out = ""
    var i = 0
    val len = tokens.length
    while i < len do
      tokens(i) match
        case _: SpaceToken      => out += renderer.space()
        case _: HrToken         => out += renderer.hr()
        case t: HeadingToken    => out += renderer.heading(t)
        case t: CodeToken       => out += renderer.code(t)
        case t: TableToken      => out += renderer.table(t)
        case t: BlockquoteToken => out += renderer.blockquote(t)
        case t: ListToken       => out += renderer.list(t)
        case t: CheckboxToken   => out += renderer.checkbox(t)
        case t: HtmlToken       => out += renderer.html(t)
        case _: DefToken        => out += renderer.defToken()
        case t: ParagraphToken  => out += renderer.paragraph(t)
        case t: TextToken       => out += renderer.text(t)
        case _                => throw new RuntimeException("Token type not found in block parse")
      i += 1
    out

  def parseInline(tokens: js.Array[Token]): String =
    var out = ""
    var i = 0
    val len = tokens.length
    while i < len do
      tokens(i) match
        case t: EscapeToken   => out += renderer.escapeText(t)
        case t: HtmlToken     => out += renderer.html(t)
        case t: LinkToken     => out += renderer.link(t)
        case t: ImageToken    => out += renderer.image(t)
        case t: CheckboxToken => out += renderer.checkbox(t)
        case t: StrongToken   => out += renderer.strong(t)
        case t: EmToken       => out += renderer.em(t)
        case t: CodespanToken => out += renderer.codespan(t)
        case _: BrToken       => out += renderer.br()
        case t: DelToken      => out += renderer.del(t)
        case t: TextToken     => out += renderer.text(t)
        case _             => throw new RuntimeException("Token type not found in inline parse")
      i += 1
    out

  def parseInlineText(tokens: js.Array[Token]): String =
    var out = ""
    var i = 0
    val len = tokens.length
    while i < len do
      out += TextRenderer.renderToken(tokens(i))
      i += 1
    out

object Parser:
  def parse(tokens: js.Array[Token], options: MarkedOptions): String =
    new Parser(options).parse(tokens)

  def parseInline(tokens: js.Array[Token], options: MarkedOptions): String =
    new Parser(options).parseInline(tokens)
