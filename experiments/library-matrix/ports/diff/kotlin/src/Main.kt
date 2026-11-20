// Entry of the Kotlin/JS port: one operation per input, as libraries/diff/reference.mjs runs it.
//
// The boundary: the input is a JS object typed by external interfaces (read in place). Strings are
// JS strings throughout; JSON documents and array elements stay host values. Results are plain JS
// objects (`kotlin.js.json`) and arrays in jsdiff's key order, `false` for a patch that does not
// apply. `run` is a native ES module export through @JsExport.

import diff.ArrayDiff
import diff.Change
import diff.CharacterDiff
import diff.DiffOptions
import diff.JsonDiff
import diff.LineDiff
import diff.TextDiff
import diff.WordDiff
import diff.applyPatch
import diff.formatPatch
import diff.isJsArray
import diff.parsePatch
import diff.readFilePatch
import diff.reversePatch
import diff.structuredPatch
import kotlin.js.Json
import kotlin.js.json

external interface Input {
    val op: String
    val before: Any?
    val after: Any?
    val patch: Any?
    val oldFileName: String?
    val newFileName: String?
    val oldHeader: String?
    val newHeader: String?
    val options: OptionsJson?
}

external interface OptionsJson {
    val ignoreCase: Boolean?
    val oneChangePerToken: Boolean?
    val stripTrailingCr: Boolean?
    val ignoreWhitespace: Boolean?
    val newlineIsToken: Boolean?
    val ignoreNewlineAtEof: Boolean?
    val context: Int?
}

private fun options(json: OptionsJson?) = DiffOptions(
    ignoreCase = json?.ignoreCase == true,
    oneChangePerToken = json?.oneChangePerToken == true,
    stripTrailingCr = json?.stripTrailingCr == true,
    ignoreWhitespace = json?.ignoreWhitespace == true,
    newlineIsToken = json?.newlineIsToken == true,
    ignoreNewlineAtEof = json?.ignoreNewlineAtEof == true,
)

private fun <V> changes(changes: List<Change<V>>): Array<Json> = changes.map {
    json("count" to it.count, "added" to it.added, "removed" to it.removed, "value" to it.value)
}.toTypedArray()

private fun textDiff(diff: TextDiff, input: Input) = changes(diff.diff(input.before, input.after, options(input.options)))

private fun patch(input: Input) = structuredPatch(
    input.oldFileName, input.newFileName, input.before.unsafeCast<String>(), input.after.unsafeCast<String>(),
    input.oldHeader, input.newHeader, input.options?.context ?: 4,
)

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: Input): Any? = when (input.op) {
    "diffChars" -> textDiff(CharacterDiff, input)
    "diffWords" -> textDiff(WordDiff, input)
    "diffLines" -> textDiff(LineDiff, input)
    "diffJson" -> textDiff(JsonDiff, input)
    "diffArrays" -> changes(
        ArrayDiff.diffTokens(
            input.before.unsafeCast<Array<Any?>>().asList(), input.after.unsafeCast<Array<Any?>>().asList(), options(input.options),
        ),
    )
    "structuredPatch" -> patch(input).toJson()
    "createTwoFilesPatch" -> formatPatch(patch(input))
    "parsePatch" -> parsePatch(input.patch.unsafeCast<String>()).map { it.toJson() }.toTypedArray()
    "applyPatch" -> {
        val patch = input.patch
        val patches = when {
            jsTypeOf(patch) == "string" -> parsePatch(patch.unsafeCast<String>())
            patch.isJsArray() -> patch.unsafeCast<Array<Any?>>().map(::readFilePatch)
            else -> listOf(readFilePatch(patch))
        }
        applyPatch(input.before.unsafeCast<String>(), patches) ?: false
    }
    "reversePatch" -> reversePatch(input.patch)
    else -> throw IllegalArgumentException("unknown diff operation")
}
