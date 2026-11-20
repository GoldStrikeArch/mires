// jsdiff 9.0.0 diff/base.js with the character, line, JSON and array diffs.
package diff

/** The synchronous options of the contract; patch creation passes none of them. */
class DiffOptions(
    val ignoreCase: Boolean = false,
    val oneChangePerToken: Boolean = false,
    val stripTrailingCr: Boolean = false,
    val ignoreWhitespace: Boolean = false,
    val newlineIsToken: Boolean = false,
    val ignoreNewlineAtEof: Boolean = false,
)

/** A change object: [count] tokens kept, added or removed, and their joined [value]. */
class Change<V>(val count: Int, val added: Boolean, val removed: Boolean, var value: V)

/** An edit-history node; each path shares its predecessors with its neighbours. */
private class Component(val count: Int, val added: Boolean, val removed: Boolean, val previous: Component?)

private class Path(var oldPos: Int, var lastComponent: Component?)

/** base.js: Myers' O(ND) search over tokens of type [T], joined into values of type [V]. */
abstract class Diff<T, V> {
    protected open val useLongestToken: Boolean get() = false

    protected open fun removeEmpty(tokens: List<T>): List<T> = tokens

    protected open fun tokensEqual(left: T, right: T, options: DiffOptions): Boolean = left == right

    protected abstract fun join(tokens: List<T>): V

    protected open fun postProcess(changes: List<Change<V>>, options: DiffOptions): List<Change<V>> = changes

    /** The longer of an old and a new token that compare equal (diffJson's dangling commas). */
    protected open fun longest(old: T, new: T): T = new

    fun diffTokens(oldTokens: List<T>, newTokens: List<T>, options: DiffOptions): List<Change<V>> {
        val oldTokens = removeEmpty(oldTokens)
        val newTokens = removeEmpty(newTokens)
        return postProcess(buildValues(search(oldTokens, newTokens, options), newTokens, oldTokens), options)
    }

    private fun search(oldTokens: List<T>, newTokens: List<T>, options: DiffOptions): Component? {
        val newLen = newTokens.size
        val oldLen = oldTokens.size
        val maxEditLength = newLen + oldLen
        // The best path on each diagonal, which runs from -maxEditLength - 1 to maxEditLength + 1.
        val bestPath = arrayOfNulls<Path>(2 * maxEditLength + 3)
        fun at(diagonal: Int) = diagonal + maxEditLength + 1
        val origin = Path(-1, null)
        bestPath[at(0)] = origin
        var newPos = extractCommon(origin, newTokens, oldTokens, 0, options)
        if (origin.oldPos + 1 >= oldLen && newPos + 1 >= newLen) return origin.lastComponent
        // Once a path reaches the right or bottom edge, the diagonals beyond it are pointless.
        var minDiagonalToConsider = Int.MIN_VALUE
        var maxDiagonalToConsider = Int.MAX_VALUE
        for (editLength in 1..maxEditLength) {
            var diagonalPath = maxOf(minDiagonalToConsider, -editLength)
            while (diagonalPath <= minOf(maxDiagonalToConsider, editLength)) {
                val removePath = bestPath[at(diagonalPath - 1)]
                val addPath = bestPath[at(diagonalPath + 1)]
                // No one else is going to use this value.
                if (removePath != null) bestPath[at(diagonalPath - 1)] = null
                // The new position after an insertion must lie inside the new tokens.
                val canAdd = addPath != null && addPath.oldPos - diagonalPath in 0 until newLen
                val canRemove = removePath != null && removePath.oldPos + 1 < oldLen
                if (!canAdd && !canRemove) {
                    bestPath[at(diagonalPath)] = null
                    diagonalPath += 2
                    continue
                }
                // Branch from the path that is furthest along the old tokens (a path that cannot
                // remove can add).
                val basePath = if (!canRemove || (canAdd && removePath.oldPos < addPath.oldPos)) {
                    addToPath(addPath!!, added = true, removed = false, oldPosInc = 0, options)
                } else {
                    addToPath(removePath, added = false, removed = true, oldPosInc = 1, options)
                }
                newPos = extractCommon(basePath, newTokens, oldTokens, diagonalPath, options)
                if (basePath.oldPos + 1 >= oldLen && newPos + 1 >= newLen) return basePath.lastComponent
                bestPath[at(diagonalPath)] = basePath
                if (basePath.oldPos + 1 >= oldLen) maxDiagonalToConsider = minOf(maxDiagonalToConsider, diagonalPath - 1)
                if (newPos + 1 >= newLen) minDiagonalToConsider = maxOf(minDiagonalToConsider, diagonalPath + 1)
                diagonalPath += 2
            }
        }
        error("the edit graph has no path")
    }

    private fun addToPath(path: Path, added: Boolean, removed: Boolean, oldPosInc: Int, options: DiffOptions): Path {
        val last = path.lastComponent
        val component = if (last != null && !options.oneChangePerToken && last.added == added && last.removed == removed) {
            Component(last.count + 1, added, removed, last.previous)
        } else {
            Component(1, added, removed, last)
        }
        return Path(path.oldPos + oldPosInc, component)
    }

    private fun extractCommon(basePath: Path, newTokens: List<T>, oldTokens: List<T>, diagonalPath: Int, options: DiffOptions): Int {
        var oldPos = basePath.oldPos
        var newPos = oldPos - diagonalPath
        var commonCount = 0
        while (newPos + 1 < newTokens.size && oldPos + 1 < oldTokens.size &&
            tokensEqual(oldTokens[oldPos + 1], newTokens[newPos + 1], options)
        ) {
            newPos++
            oldPos++
            commonCount++
            if (options.oneChangePerToken) {
                basePath.lastComponent = Component(1, added = false, removed = false, basePath.lastComponent)
            }
        }
        if (commonCount > 0 && !options.oneChangePerToken) {
            basePath.lastComponent = Component(commonCount, added = false, removed = false, basePath.lastComponent)
        }
        basePath.oldPos = oldPos
        return newPos
    }

    private fun buildValues(lastComponent: Component?, newTokens: List<T>, oldTokens: List<T>): List<Change<V>> {
        val components = generateSequence(lastComponent) { it.previous }.toList().asReversed()
        var newPos = 0
        var oldPos = 0
        return components.map { component ->
            val tokens = if (!component.removed) {
                val kept = newTokens.subList(newPos, newPos + component.count)
                val value = if (!component.added && useLongestToken) {
                    kept.mapIndexed { i, token -> longest(oldTokens[oldPos + i], token) }
                } else {
                    kept
                }
                newPos += component.count
                if (!component.added) oldPos += component.count
                value
            } else {
                oldTokens.subList(oldPos, oldPos + component.count).also { oldPos += component.count }
            }
            Change(component.count, component.added, component.removed, join(tokens))
        }
    }
}

/** A diff over string tokens, which are dropped when empty and compared case-insensitively on request. */
abstract class TextDiff : Diff<String, String>() {
    open fun castInput(value: Any?): String = value.unsafeCast<String>()

    abstract fun tokenize(value: String, options: DiffOptions): List<String>

    fun diff(oldValue: Any?, newValue: Any?, options: DiffOptions): List<Change<String>> =
        diffTokens(tokenize(castInput(oldValue), options), tokenize(castInput(newValue), options), options)

    override fun removeEmpty(tokens: List<String>): List<String> = tokens.filter { it.isNotEmpty() }

    override fun tokensEqual(left: String, right: String, options: DiffOptions): Boolean =
        left == right || (options.ignoreCase && left.lowercase() == right.lowercase())

    override fun join(tokens: List<String>): String = tokens.joinToString("")
}

/** character.js: tokens are code points (`Array.from`), so a lone surrogate is one token. */
object CharacterDiff : TextDiff() {
    override fun tokenize(value: String, options: DiffOptions): List<String> = codePoints(value)
}

fun codePoints(value: String): List<String> {
    val points = ArrayList<String>()
    var i = 0
    while (i < value.length) {
        val width = if (value[i].isHighSurrogate() && i + 1 < value.length && value[i + 1].isLowSurrogate()) 2 else 1
        points.add(value.substring(i, i + width))
        i += width
    }
    return points
}

/** line.js `tokenize`: lines with their terminators, or the terminators as separate tokens. */
fun lineTokens(value: String, options: DiffOptions): List<String> {
    val text = if (options.stripTrailingCr) value.replace("\r\n", "\n") else value
    // text.split(/(\n|\r\n)/): each \n is a separator, together with a \r before it.
    val linesAndNewlines = ArrayList<String>()
    var start = 0
    for (i in text.indices) {
        if (text[i] != '\n') continue
        val separator = if (i > start && text[i - 1] == '\r') i - 1 else i
        linesAndNewlines.add(text.substring(start, separator))
        linesAndNewlines.add(text.substring(separator, i + 1))
        start = i + 1
    }
    linesAndNewlines.add(text.substring(start))
    // Ignore the final empty token that occurs if the string ends with a new line.
    if (linesAndNewlines.last().isEmpty()) linesAndNewlines.removeAt(linesAndNewlines.lastIndex)
    val retLines = ArrayList<String>()
    for ((i, line) in linesAndNewlines.withIndex()) {
        if (i % 2 == 1 && !options.newlineIsToken) {
            retLines[retLines.lastIndex] += line
        } else {
            retLines.add(line)
        }
    }
    return retLines
}

object LineDiff : TextDiff() {
    override fun tokenize(value: String, options: DiffOptions) = lineTokens(value, options)

    override fun tokensEqual(left: String, right: String, options: DiffOptions): Boolean {
        var l = left
        var r = right
        // With newlineIsToken, a newline token is not trimmed down to an empty line.
        if (options.ignoreWhitespace) {
            if (!options.newlineIsToken || '\n' !in l) l = l.jsTrim()
            if (!options.newlineIsToken || '\n' !in r) r = r.jsTrim()
        } else if (options.ignoreNewlineAtEof && !options.newlineIsToken) {
            l = l.removeSuffix("\n")
            r = r.removeSuffix("\n")
        }
        return super.tokensEqual(l, r, options)
    }
}

/** json.js: a pretty-printed canonical document, diffed by lines. */
object JsonDiff : TextDiff() {
    private val danglingComma = Regex(",([\r\n])")

    override val useLongestToken get() = true

    override fun castInput(value: Any?): String =
        if (jsTypeOf(value) == "string") value.unsafeCast<String>() else JSON.stringify(canonicalize(value), null, "  ")

    override fun tokenize(value: String, options: DiffOptions) = lineTokens(value, options)

    override fun tokensEqual(left: String, right: String, options: DiffOptions): Boolean =
        super.tokensEqual(left.replace(danglingComma, "$1"), right.replace(danglingComma, "$1"), options)

    override fun longest(old: String, new: String) = if (old.length > new.length) old else new
}

/**
 * `canonicalize` for JSON documents: arrays in order, object keys sorted and assigned to a plain
 * object, so a "__proto__" key sets the prototype and is not serialized. JSON input has no cycles
 * and no `toJSON`, and the default replacer keeps every value.
 */
fun canonicalize(value: Any?): Any? = when {
    value.isJsArray() -> value.unsafeCast<Array<Any?>>().map(::canonicalize).toTypedArray()
    jsTypeOf(value) == "object" && value != null -> {
        val canonical = kotlin.js.json()
        for (key in JsObject.keys(value).sorted()) canonical[key] = canonicalize(value.asDynamic()[key])
        canonical
    }
    else -> value
}

/** array.js: tokens are the JSON values themselves, compared with `===`. */
object ArrayDiff : Diff<Any?, Array<Any?>>() {
    override fun tokensEqual(left: Any?, right: Any?, options: DiffOptions) = left === right

    override fun join(tokens: List<Any?>): Array<Any?> = tokens.toTypedArray()
}
