// The boundary of the Scala.js port: one operation per input, as libraries/diff/reference.mjs runs
// it. The input is the JS object itself, seen through @js.native facade traits (no copy). Strings
// are JS strings throughout; JSON documents and array elements stay host values. Results are plain
// JS objects (js.Dynamic.literal) and arrays in jsdiff's key order, `false` for a patch that does
// not apply. `run` is a native ES module export.

package diff

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

@js.native
trait Input extends js.Object:
  val op: String = js.native
  val before: js.Any = js.native
  val after: js.Any = js.native
  val patch: js.Any = js.native
  val oldFileName: js.UndefOr[String] = js.native
  val newFileName: js.UndefOr[String] = js.native
  val oldHeader: js.UndefOr[String] = js.native
  val newHeader: js.UndefOr[String] = js.native
  val options: js.UndefOr[OptionsJson] = js.native

@js.native
trait OptionsJson extends js.Object:
  val ignoreCase: js.UndefOr[Boolean] = js.native
  val oneChangePerToken: js.UndefOr[Boolean] = js.native
  val stripTrailingCr: js.UndefOr[Boolean] = js.native
  val ignoreWhitespace: js.UndefOr[Boolean] = js.native
  val newlineIsToken: js.UndefOr[Boolean] = js.native
  val ignoreNewlineAtEof: js.UndefOr[Boolean] = js.native
  val context: js.UndefOr[Double] = js.native

private def diffOptions(input: Input): DiffOptions =
  def enabled(read: OptionsJson => js.UndefOr[Boolean]) = input.options.exists(read(_).contains(true))
  DiffOptions(
    ignoreCase = enabled(_.ignoreCase),
    oneChangePerToken = enabled(_.oneChangePerToken),
    stripTrailingCr = enabled(_.stripTrailingCr),
    ignoreWhitespace = enabled(_.ignoreWhitespace),
    newlineIsToken = enabled(_.newlineIsToken),
    ignoreNewlineAtEof = enabled(_.ignoreNewlineAtEof)
  )

private def changes[V](changes: IndexedSeq[Change[V]]): js.Array[js.Object] =
  js.Array(changes.map(c => js.Dynamic.literal(count = c.count, added = c.added, removed = c.removed, value = c.value.asInstanceOf[js.Any]))*)

private def textDiff(diff: TextDiff, input: Input) = changes(diff.diff(input.before, input.after, diffOptions(input)))

private def patch(input: Input) = structuredPatch(
  input.oldFileName,
  input.newFileName,
  input.before.asInstanceOf[String],
  input.after.asInstanceOf[String],
  input.oldHeader,
  input.newHeader,
  input.options.flatMap(_.context).getOrElse(4.0)
)

@JSExportTopLevel("run")
def run(input: Input): js.Any = input.op match
  case "diffChars" => textDiff(CharacterDiff, input)
  case "diffWords" => textDiff(WordDiff, input)
  case "diffLines" => textDiff(LineDiff, input)
  case "diffJson" => textDiff(JsonDiff, input)
  case "diffArrays" =>
    def tokens(value: js.Any) = value.asInstanceOf[js.Array[Any]].toIndexedSeq
    changes(ArrayDiff.diffTokens(tokens(input.before), tokens(input.after), diffOptions(input)))
  case "structuredPatch" => patch(input).toJs
  case "createTwoFilesPatch" => formatPatch(patch(input))
  case "parsePatch" => js.Array(parsePatch(input.patch.asInstanceOf[String]).map(_.toJs)*)
  case "applyPatch" =>
    // Array.isArray, as jsdiff tests it, does not depend on the value's realm.
    val patches = (input.patch: Any) match
      case text: String => parsePatch(text)
      case _ if js.Array.isArray(input.patch) => input.patch.asInstanceOf[js.Array[js.Any]].toIndexedSeq.map(readFilePatch)
      case _ => IndexedSeq(readFilePatch(input.patch))
    applyPatch(input.before.asInstanceOf[String], patches).fold[js.Any](false)(identity)
  case "reversePatch" => reversePatch(input.patch)
  case _ => throw IllegalArgumentException("unknown diff operation")
