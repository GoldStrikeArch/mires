// Entry of the Scala.js port: what libraries/jmespath/reference.mjs does, on the port's jmespath.
//
// The boundary: the corpus input is the JS object itself, seen through @js.native facade traits
// (no copy). The document stays the JS value it is (jmespath returns parts of it), each expression
// is a JS string, and each result is a plain object `{ value }` or `{ error }` with the reference's
// error category. `run` is a native ES module export.

package jmespath

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

@js.native
trait Input extends js.Object:
  val data: js.Any = js.native
  val queries: js.Array[Query] = js.native

@js.native
trait Query extends js.Object:
  val op: String = js.native
  val expression: String = js.native

object Main:
  /** jmespath's `compile`. */
  def compile(expression: String): js.UndefOr[Node] = Parser().parse(expression)

  /** jmespath's `search`: a fresh parser and interpreter per call. */
  def search(data: js.Any, expression: String): js.Any = TreeInterpreter().search(Parser().parse(expression), data)

  @JSExportTopLevel("run")
  def run(input: Input): js.Array[js.Dictionary[js.Any]] = input.queries.map { query =>
    try
      val value: js.Any = if query.op == "compile" then compile(query.expression).asInstanceOf[js.Any] else search(input.data, query.expression)
      js.Dictionary("value" -> value)
    catch
      case e: JmesPathException => js.Dictionary[js.Any]("error" -> e.category.label)
      // An exhausted stack propagates, as it does from the reference adapter.
      case e: Throwable if Host.isStackExhaustion(e) => throw e
      // The reference's category() default: JSON.parse's SyntaxError and engine errors.
      case _: Throwable => js.Dictionary[js.Any]("error" -> ErrorCategory.Syntax.label)
  }
