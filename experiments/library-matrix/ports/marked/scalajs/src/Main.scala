// Boundary for the marked 18.1.1 Scala.js port: Scala port of libraries/marked/reference.mjs.
// Reads the JSON corpus input through @js.native facade traits (no copy) and returns the
// rendered HTML JS String directly. Exported as a native ES module export via @JSExportTopLevel("run").

package marked

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

@js.native
trait OptionsJson extends js.Object:
  val gfm: js.UndefOr[Boolean] = js.native
  val breaks: js.UndefOr[Boolean] = js.native
  val pedantic: js.UndefOr[Boolean] = js.native

@js.native
trait Input extends js.Object:
  val op: String = js.native
  val markdown: String = js.native
  val options: js.UndefOr[OptionsJson] = js.native

object Marked:
  def parse(markdown: String, options: MarkedOptions): String =
    val tokens = Lexer.lex(markdown, options)
    Parser.parse(tokens, options)

  def parseInline(markdown: String, options: MarkedOptions): String =
    val tokens = Lexer.lexInline(markdown, options)
    Parser.parseInline(tokens, options)

object Main:
  private def readOptions(optJson: js.UndefOr[OptionsJson]): MarkedOptions =
    if js.isUndefined(optJson) || (optJson.asInstanceOf[AnyRef] eq null) then MarkedOptions.defaults
    else
      val j = optJson.asInstanceOf[OptionsJson]
      val gfm = if js.isUndefined(j.gfm) then true else j.gfm.asInstanceOf[Boolean]
      val breaks = if js.isUndefined(j.breaks) then false else j.breaks.asInstanceOf[Boolean]
      val pedantic = if js.isUndefined(j.pedantic) then false else j.pedantic.asInstanceOf[Boolean]
      new MarkedOptions(gfm, breaks, pedantic)

  @JSExportTopLevel("run")
  def run(input: Input): String =
    val opts = readOptions(input.options)
    if input.op == "parse" then Marked.parse(input.markdown, opts)
    else if input.op == "parseInline" then Marked.parseInline(input.markdown, opts)
    else throw js.JavaScriptException(new js.TypeError("unknown operation " + input.op))
