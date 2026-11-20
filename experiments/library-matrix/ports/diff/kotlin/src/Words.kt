// jsdiff 9.0.0 diff/word.js and the whitespace helpers of util/string.js.
package diff

// Latin letters with diacritics and the spacing modifier letters count as word characters.
private const val EXTENDED_WORD_CHARS =
    "a-zA-Z0-9_\\u{AD}\\u{C0}-\\u{D6}\\u{D8}-\\u{F6}\\u{F8}-\\u{2C6}\\u{2C8}-\\u{2D7}\\u{2DE}-\\u{2FF}\\u{1E00}-\\u{1EFF}"

// Kotlin/JS compiles a Regex to a host RegExp with the `u` flag, as jsdiff's `ug` pattern has.
private val tokenizeIncludingWhitespace = Regex("[$EXTENDED_WORD_CHARS]+|\\s+|[^$EXTENDED_WORD_CHARS]")
private val whitespace = Regex("\\s")

private fun String.isWhitespaceRun() = whitespace.containsMatchIn(this)

object WordDiff : TextDiff() {
    override fun tokenize(value: String, options: DiffOptions): List<String> {
        // Whitespace runs are joined to the neighbouring word or punctuation tokens.
        val tokens = ArrayList<String>()
        var prevPart: String? = null
        for (match in tokenizeIncludingWhitespace.findAll(value)) {
            val part = match.value
            if (part.isWhitespaceRun()) {
                if (prevPart == null) tokens.add(part) else tokens.add(tokens.removeLast() + part)
            } else if (prevPart != null && prevPart.isWhitespaceRun()) {
                if (tokens.last() == prevPart) tokens.add(tokens.removeLast() + part) else tokens.add(prevPart + part)
            } else {
                tokens.add(part)
            }
            prevPart = part
        }
        return tokens
    }

    override fun tokensEqual(left: String, right: String, options: DiffOptions): Boolean =
        if (options.ignoreCase) left.lowercase().jsTrim() == right.lowercase().jsTrim() else left.jsTrim() == right.jsTrim()

    // Tokens that are joined appeared consecutively, so each but the first drops its leading whitespace.
    override fun join(tokens: List<String>): String =
        tokens.mapIndexed { i, token -> if (i == 0) token else token.substring(leadingWs(token).length) }.joinToString("")

    override fun postProcess(changes: List<Change<String>>, options: DiffOptions): List<Change<String>> {
        if (options.oneChangePerToken) return changes
        var lastKeep: Change<String>? = null
        // The insertion and deletion since the last kept change; there is at most one of each.
        var insertion: Change<String>? = null
        var deletion: Change<String>? = null
        for (change in changes) {
            if (change.added) {
                insertion = change
            } else if (change.removed) {
                deletion = change
            } else {
                if (insertion != null || deletion != null) dedupeWhitespace(lastKeep, deletion, insertion, change)
                lastKeep = change
                insertion = null
                deletion = null
            }
        }
        if (insertion != null || deletion != null) dedupeWhitespace(lastKeep, deletion, insertion, null)
        return changes
    }
}

/**
 * dedupeWhitespaceInChangeObjects: whitespace that trails one change object and leads the next is
 * kept once, on the side where both texts have it.
 */
private fun dedupeWhitespace(
    startKeep: Change<String>?,
    deletion: Change<String>?,
    insertion: Change<String>?,
    endKeep: Change<String>?,
) {
    if (deletion != null && insertion != null) {
        val oldWsPrefix = leadingWs(deletion.value)
        val oldWsSuffix = trailingWs(deletion.value)
        val newWsPrefix = leadingWs(insertion.value)
        val newWsSuffix = trailingWs(insertion.value)
        if (startKeep != null) {
            val commonWsPrefix = longestCommonPrefix(oldWsPrefix, newWsPrefix)
            startKeep.value = replaceSuffix(startKeep.value, newWsPrefix, commonWsPrefix)
            deletion.value = removePrefix(deletion.value, commonWsPrefix)
            insertion.value = removePrefix(insertion.value, commonWsPrefix)
        }
        if (endKeep != null) {
            val commonWsSuffix = longestCommonSuffix(oldWsSuffix, newWsSuffix)
            endKeep.value = replacePrefix(endKeep.value, newWsSuffix, commonWsSuffix)
            deletion.value = removeSuffix(deletion.value, commonWsSuffix)
            insertion.value = removeSuffix(insertion.value, commonWsSuffix)
        }
    } else if (insertion != null) {
        // The whitespace is all from the new text: each object keeps its trailing whitespace.
        if (startKeep != null) insertion.value = insertion.value.substring(leadingWs(insertion.value).length)
        if (endKeep != null) endKeep.value = endKeep.value.substring(leadingWs(endKeep.value).length)
    } else if (deletion != null && startKeep != null && endKeep != null) {
        val newWsFull = leadingWs(endKeep.value)
        val delWsStart = leadingWs(deletion.value)
        val delWsEnd = trailingWs(deletion.value)
        // Whitespace right after startKeep in both texts goes to startKeep.
        val newWsStart = longestCommonPrefix(newWsFull, delWsStart)
        deletion.value = removePrefix(deletion.value, newWsStart)
        // Whitespace right before endKeep in both texts, not given to startKeep, goes to endKeep.
        val newWsEnd = longestCommonSuffix(removePrefix(newWsFull, newWsStart), delWsEnd)
        deletion.value = removeSuffix(deletion.value, newWsEnd)
        endKeep.value = replacePrefix(endKeep.value, newWsFull, newWsEnd)
        // Any whitespace from the new text not yet assigned goes to the start.
        startKeep.value = replaceSuffix(startKeep.value, newWsFull, newWsFull.substring(0, newWsFull.length - newWsEnd.length))
    } else if (deletion != null && endKeep != null) {
        // At the start of the text: drop the deletion's trailing whitespace that endKeep repeats.
        val overlap = maximumOverlap(trailingWs(deletion.value), leadingWs(endKeep.value))
        deletion.value = removeSuffix(deletion.value, overlap)
    } else if (deletion != null && startKeep != null) {
        // At the end of the text: drop the deletion's leading whitespace that startKeep repeats.
        val overlap = maximumOverlap(trailingWs(startKeep.value), leadingWs(deletion.value))
        deletion.value = removePrefix(deletion.value, overlap)
    }
}

/** leadingWs: the whitespace units at the start. */
fun leadingWs(s: String): String {
    var i = 0
    while (i < s.length && s[i].isJsWhitespace()) i++
    return s.substring(0, i)
}

/** trailingWs: the whitespace units at the end. */
fun trailingWs(s: String): String {
    var i = s.length
    while (i > 0 && s[i - 1].isJsWhitespace()) i--
    return s.substring(i)
}

private fun longestCommonPrefix(a: String, b: String): String {
    var i = 0
    while (i < a.length && i < b.length && a[i] == b[i]) i++
    return a.substring(0, i)
}

private fun longestCommonSuffix(a: String, b: String): String {
    var i = 0
    while (i < a.length && i < b.length && a[a.length - 1 - i] == b[b.length - 1 - i]) i++
    return a.substring(a.length - i)
}

private fun replacePrefix(string: String, oldPrefix: String, newPrefix: String): String {
    check(string.startsWith(oldPrefix)) { "string doesn't start with prefix; this is a bug" }
    return newPrefix + string.substring(oldPrefix.length)
}

private fun replaceSuffix(string: String, oldSuffix: String, newSuffix: String): String {
    check(string.endsWith(oldSuffix)) { "string doesn't end with suffix; this is a bug" }
    return string.substring(0, string.length - oldSuffix.length) + newSuffix
}

private fun removePrefix(string: String, oldPrefix: String) = replacePrefix(string, oldPrefix, "")

private fun removeSuffix(string: String, oldSuffix: String) = replaceSuffix(string, oldSuffix, "")

/** The longest suffix of [a] that is a prefix of [b], by Knuth-Morris-Pratt (overlapCount). */
private fun maximumOverlap(a: String, b: String): String {
    val startA = if (a.length > b.length) a.length - b.length else 0
    val endB = minOf(a.length, b.length)
    val map = IntArray(endB)
    var k = 0
    for (j in 1 until endB) {
        map[j] = if (b[j] == b[k]) map[k] else k
        while (k > 0 && b[j] != b[k]) k = map[k]
        if (b[j] == b[k]) k++
    }
    k = 0
    for (i in startA until a.length) {
        while (k > 0 && a[i] != b[k]) k = map[k]
        if (a[i] == b[k]) k++
    }
    return b.substring(0, k)
}
