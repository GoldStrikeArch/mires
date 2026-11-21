// jsdiff 9.0.0 patch/create.js, parse.js, apply.js, reverse.js and line-endings.js.
package diff

import kotlin.js.Json
import kotlin.js.json

/** A hunk. Its ranges are JavaScript numbers: a parsed header can exceed Int or be NaN. */
class Hunk(
    var oldStart: Double,
    var oldLines: Double,
    var newStart: Double,
    var newLines: Double,
    val lines: MutableList<String> = ArrayList(),
) {
    fun toJson(): Json = json(
        "oldStart" to oldStart, "oldLines" to oldLines, "newStart" to newStart, "newLines" to newLines,
        "lines" to lines.toTypedArray(),
    )
}

/**
 * A file patch. [properties] holds its metadata in the order jsdiff first assigns each property,
 * which is the key order of the object it returns; `hunks` stands for [hunks].
 */
class FilePatch(val hunks: MutableList<Hunk> = ArrayList()) {
    val properties = LinkedHashMap<String, Any?>()

    operator fun get(key: String): Any? = properties[key]

    operator fun set(key: String, value: Any?) {
        properties[key] = value
    }

    fun toJson(): Json {
        val out = json()
        for ((key, value) in properties) out[key] = if (key == "hunks") hunks.map(Hunk::toJson).toTypedArray() else value
        return out
    }
}

// ---- structuredPatch and formatPatch ----

fun structuredPatch(
    oldFileName: String?, newFileName: String?, oldStr: String, newStr: String,
    oldHeader: String?, newHeader: String?, context: Double,
): FilePatch {
    // context is a JavaScript number, compared and sliced as npm does it.
    class Entry(val added: Boolean, val removed: Boolean, val lines: List<String>)
    // Step 1: hunks of lines that keep their newlines; an empty entry at the end closes the last one.
    val diff = LineDiff.diff(oldStr, newStr, DiffOptions()).map { Entry(it.added, it.removed, splitLines(it.value)) } +
        Entry(added = false, removed = false, lines = emptyList())
    val hunks = ArrayList<Hunk>()
    var oldRangeStart = 0
    var newRangeStart = 0
    var curRange = ArrayList<String>()
    var oldLine = 1
    var newLine = 1
    for ((i, current) in diff.withIndex()) {
        val lines = current.lines
        if (current.added || current.removed) {
            // If we have previous context, start with that.
            if (oldRangeStart == 0) {
                val prev = diff.getOrNull(i - 1)
                oldRangeStart = oldLine
                newRangeStart = newLine
                if (prev != null) {
                    curRange = if (context > 0) {
                        prev.lines.takeLast(minOf(context, prev.lines.size.toDouble()).toInt()).mapTo(ArrayList()) { " $it" }
                    } else {
                        ArrayList()
                    }
                    oldRangeStart -= curRange.size
                    newRangeStart -= curRange.size
                }
            }
            for (line in lines) curRange.add((if (current.added) "+" else "-") + line)
            if (current.added) newLine += lines.size else oldLine += lines.size
        } else {
            if (oldRangeStart != 0) {
                if (lines.size <= context * 2 && i < diff.size - 2) {
                    // Overlapping: the context joins the next change.
                    for (line in lines) curRange.add(" $line")
                } else {
                    val contextSize = minOf(lines.size.toDouble(), context).toInt()
                    for (line in lines.take(contextSize)) curRange.add(" $line")
                    hunks.add(
                        Hunk(
                            oldRangeStart.toDouble(), (oldLine - oldRangeStart + contextSize).toDouble(),
                            newRangeStart.toDouble(), (newLine - newRangeStart + contextSize).toDouble(), curRange,
                        ),
                    )
                    oldRangeStart = 0
                    newRangeStart = 0
                    curRange = ArrayList()
                }
            }
            oldLine += lines.size
            newLine += lines.size
        }
    }
    // Step 2: drop each line's newline, or mark that it has none.
    for (hunk in hunks) {
        var i = 0
        while (i < hunk.lines.size) {
            if (hunk.lines[i].endsWith("\n")) {
                hunk.lines[i] = hunk.lines[i].dropLast(1)
            } else {
                hunk.lines.add(i + 1, "\\ No newline at end of file")
                i++
            }
            i++
        }
    }
    val patch = FilePatch(hunks)
    patch["oldFileName"] = oldFileName
    patch["newFileName"] = newFileName
    patch["oldHeader"] = oldHeader
    patch["newHeader"] = newHeader
    patch["hunks"] = hunks
    return patch
}

/** The lines of [text], each with its newline where it has one. */
private fun splitLines(text: String): List<String> {
    val result = text.split("\n").mapTo(ArrayList()) { "$it\n" }
    if (text.endsWith("\n")) result.removeAt(result.lastIndex) else result[result.lastIndex] = result.last().dropLast(1)
    return result
}

/** C-style quoting of a name, over its UTF-8 bytes, as Git and GNU diffutils write it. */
private fun quoteFileNameIfNeeded(s: String): String {
    if (s.none { it < ' ' || it > '~' || it == '"' || it == '\\' }) return s
    val result = StringBuilder("\"")
    for (byte in s.encodeToByteArray()) {
        when (val b = byte.toInt() and 0xff) {
            0x07 -> result.append("\\a")
            0x08 -> result.append("\\b")
            0x09 -> result.append("\\t")
            0x0a -> result.append("\\n")
            0x0b -> result.append("\\v")
            0x0c -> result.append("\\f")
            0x0d -> result.append("\\r")
            0x22 -> result.append("\\\"")
            0x5c -> result.append("\\\\")
            in 0x20..0x7e -> result.append(b.toChar())
            else -> result.append('\\').append(b.toString(8).padStart(3, '0'))
        }
    }
    return result.append('"').toString()
}

/** formatPatch with every header, for a patch from [structuredPatch]. */
fun formatPatch(patch: FilePatch): String {
    val ret = ArrayList<String>()
    val oldFileName = patch["oldFileName"]
    val newFileName = patch["newFileName"]
    if (oldFileName is String && oldFileName == newFileName) ret.add("Index: $oldFileName")
    ret.add(UNDERLINE)
    if (oldFileName is String && newFileName is String) {
        fun header(value: Any?) = (value as? String)?.takeIf { it.isNotEmpty() }?.let { "\t$it" } ?: ""
        ret.add("--- " + quoteFileNameIfNeeded(oldFileName) + header(patch["oldHeader"]))
        ret.add("+++ " + quoteFileNameIfNeeded(newFileName) + header(patch["newHeader"]))
    }
    for (hunk in patch.hunks) {
        // The start printed for an empty range is one lower.
        val oldStart = if (hunk.oldLines == 0.0) hunk.oldStart - 1 else hunk.oldStart
        val newStart = if (hunk.newLines == 0.0) hunk.newStart - 1 else hunk.newStart
        ret.add("@@ -${oldStart.toInt()},${hunk.oldLines.toInt()} +${newStart.toInt()},${hunk.newLines.toInt()} @@")
        ret.addAll(hunk.lines)
    }
    return ret.joinToString("\n") + "\n"
}

// ---- parsePatch ----

private const val UNDERLINE = "==================================================================="

// parse.js's line patterns; Kotlin/JS compiles each to a host RegExp.
private val gitDiffHeader = Regex("^diff --git ")
private val indexHeader = Regex("^Index:\\s")
private val hgHeader = Regex("^diff(?: -r \\w+)+\\s")
private val headerPrefix = Regex("^(?:Index:|diff(?: -r \\w+)+)\\s+")
private val fileHeader = Regex("^(---|\\+\\+\\+)\\s")
private val fileHeaderPrefix = Regex("^(---|\\+\\+\\+)\\s+")
private val hunkHeader = Regex("^@@\\s")
private val chunkHeader = Regex("@@ -(\\d+)(?:,(\\d+))? \\+(\\d+)(?:,(\\d+))? @@")
private val hunkBodyLine = Regex("^[+ -]")
private val renameFrom = Regex("^rename from (.*)")
private val renameTo = Regex("^rename to (.*)")
private val copyFrom = Regex("^copy from (.*)")
private val copyTo = Regex("^copy to (.*)")
private val newFileMode = Regex("^new file mode (\\d+)")
private val deletedFileMode = Regex("^deleted file mode (\\d+)")
private val oldMode = Regex("^old mode (\\d+)")
private val newMode = Regex("^new mode (\\d+)")
private val binaryFiles = Regex("^Binary files ")

private fun isGitDiffHeader(line: String) = gitDiffHeader.containsMatchIn(line)

/** A line that starts the next file: `diff --git`, `Index:` or Mercurial's `diff -r`. */
private fun isDiffHeader(line: String) =
    isGitDiffHeader(line) || indexHeader.containsMatchIn(line) || hgHeader.containsMatchIn(line)

private fun isFileHeader(line: String) = fileHeader.containsMatchIn(line)

private fun isHunkHeader(line: String) = hunkHeader.containsMatchIn(line)

private class QuotedName(val fileName: String, val rawLength: Int)

/**
 * A C-style quoted name as Git and GNU diff write it: the unescaped name and the length of the
 * quoted text, or null for an unterminated quote or an unknown escape.
 */
private fun parseQuotedFileName(s: String): QuotedName? {
    if (!s.startsWith("\"")) return null
    val result = StringBuilder()
    var j = 1
    while (j < s.length) {
        if (s[j] == '"') return QuotedName(result.toString(), j + 1)
        if (s[j] == '\\' && j + 1 < s.length) {
            j++
            when (s[j]) {
                'a' -> result.append('\u0007')
                'b' -> result.append('\b')
                'f' -> result.append('\u000C')
                'n' -> result.append('\n')
                'r' -> result.append('\r')
                't' -> result.append('\t')
                'v' -> result.append('\u000B')
                '\\' -> result.append('\\')
                '"' -> result.append('"')
                in '0'..'7' -> {
                    // Octal escapes are raw bytes: a run of them is decoded together as UTF-8.
                    if (j + 2 >= s.length || s[j + 1] !in '0'..'7' || s[j + 2] !in '0'..'7') return null
                    val bytes = ArrayList<Byte>()
                    bytes.add(s.substring(j, j + 3).toInt(8).toByte())
                    j += 3
                    while (j + 1 < s.length && s[j] == '\\' && s[j + 1] in '0'..'7') {
                        if (j + 3 >= s.length || s[j + 2] !in '0'..'7' || s[j + 3] !in '0'..'7') return null
                        bytes.add(s.substring(j + 1, j + 4).toInt(8).toByte())
                        j += 4
                    }
                    // TextDecoder's replacement of ill-formed bytes, and its consumed BOM.
                    result.append(bytes.toByteArray().decodeToString().removePrefix("﻿"))
                    continue
                }
                else -> return null
            }
        } else {
            result.append(s[j])
        }
        j++
    }
    return null
}

private fun unquoteIfQuoted(s: String): String =
    if (s.startsWith("\"")) parseQuotedFileName(s)?.fileName ?: s else s

/** The names in a `diff --git` line, or null when they cannot be told apart. */
private fun parseGitDiffHeader(line: String): Pair<String, String>? {
    val rest = line.substring("diff --git ".length)
    if (rest.startsWith("\"")) {
        val oldPath = parseQuotedFileName(rest) ?: return null
        // Skip the one unit after the quote, assumed to be a space.
        val afterOld = rest.substring(minOf(oldPath.rawLength + 1, rest.length))
        val newFileName = if (afterOld.startsWith("\"")) {
            (parseQuotedFileName(afterOld) ?: return null).fileName
        } else {
            afterOld
        }
        return oldPath.fileName to newFileName
    }
    val quoteIdx = rest.indexOf('"')
    if (quoteIdx > 0) {
        val newPath = parseQuotedFileName(rest.substring(quoteIdx)) ?: return null
        return rest.substring(0, quoteIdx - 1) to newPath.fileName
    }
    if (rest.startsWith("a/")) {
        // Split on the middle " b/", which is right whenever both names are the same.
        val splits = generateSequence(rest.indexOf(" b/", 1).takeIf { it >= 0 }) { at ->
            rest.indexOf(" b/", at + 1).takeIf { it >= 0 }
        }.toList()
        if (splits.isNotEmpty()) {
            val mid = splits[splits.size / 2]
            return rest.substring(0, mid) to rest.substring(mid + 1)
        }
    }
    return null
}

/** parsePatch: the unified diff text as file patches. */
fun parsePatch(uniDiff: String): List<FilePatch> = PatchParser(uniDiff.split("\n")).parse()

private class PatchParser(private val diffstr: List<String>) {
    private var i = 0
    private val list = ArrayList<FilePatch>()

    fun parse(): List<FilePatch> {
        while (i < diffstr.size) parseIndex()
        return list
    }

    private fun parseIndex() {
        val index = FilePatch()
        index["hunks"] = index.hunks
        list.add(index)
        // Diff metadata, up to the file or hunk headers.
        var seenDiffHeader = false
        while (i < diffstr.size) {
            val line = diffstr[i]
            if (isFileHeader(line) || isHunkHeader(line)) break
            if (isGitDiffHeader(line)) {
                // A second diff header belongs to the next file.
                if (seenDiffHeader) return
                seenDiffHeader = true
                index["isGit"] = true
                parseGitDiffHeader(line)?.let { (oldName, newName) ->
                    index["oldFileName"] = oldName
                    index["newFileName"] = newName
                }
                i++
                // Git extended headers.
                while (i < diffstr.size) {
                    val extLine = diffstr[i]
                    if (isFileHeader(extLine) || isHunkHeader(extLine) || isDiffHeader(extLine)) break
                    renameFrom.find(extLine)?.let {
                        index["oldFileName"] = "a/" + unquoteIfQuoted(it.groupValues[1])
                        index["isRename"] = true
                    }
                    renameTo.find(extLine)?.let {
                        index["newFileName"] = "b/" + unquoteIfQuoted(it.groupValues[1])
                        index["isRename"] = true
                    }
                    copyFrom.find(extLine)?.let {
                        index["oldFileName"] = "a/" + unquoteIfQuoted(it.groupValues[1])
                        index["isCopy"] = true
                    }
                    copyTo.find(extLine)?.let {
                        index["newFileName"] = "b/" + unquoteIfQuoted(it.groupValues[1])
                        index["isCopy"] = true
                    }
                    newFileMode.find(extLine)?.let {
                        index["isCreate"] = true
                        index["newMode"] = it.groupValues[1]
                    }
                    deletedFileMode.find(extLine)?.let {
                        index["isDelete"] = true
                        index["oldMode"] = it.groupValues[1]
                    }
                    oldMode.find(extLine)?.let { index["oldMode"] = it.groupValues[1] }
                    newMode.find(extLine)?.let { index["newMode"] = it.groupValues[1] }
                    if (binaryFiles.containsMatchIn(extLine)) index["isBinary"] = true
                    i++
                }
                continue
            } else if (isDiffHeader(line)) {
                if (seenDiffHeader) return
                seenDiffHeader = true
                // The index is the rest of an `Index:` or Mercurial header, trimmed.
                headerPrefix.find(line)?.let { index["index"] = line.substring(it.value.length).jsTrim() }
            }
            i++
        }
        parseFileHeader(index)
        parseFileHeader(index)
        // One file header without the other is a malformed patch.
        val oldFileName = index["oldFileName"]
        val newFileName = index["newFileName"]
        require((oldFileName == null) == (newFileName == null)) {
            "Missing " + (if (oldFileName != null) "\"+++ ...\"" else "\"--- ...\"") + " file header for " + (oldFileName ?: newFileName)
        }
        while (i < diffstr.size) {
            val line = diffstr[i]
            if (isDiffHeader(line) || isFileHeader(line) || line.startsWith(UNDERLINE)) {
                break
            } else if (isHunkHeader(line)) {
                index.hunks.add(parseHunk())
            } else {
                // Blank lines and other content between or after hunks are skipped.
                i++
            }
        }
    }

    /** The `---` or `+++` line at the cursor, if there is one. */
    private fun parseFileHeader(index: FilePatch) {
        val line = diffstr.getOrNull(i) ?: return
        val fileHeaderMatch = fileHeaderPrefix.find(line) ?: return
        val data = line.substring(3).jsTrim().split("\t")
        val header = data.getOrElse(1) { "" }.jsTrim()
        val fileName = data[0].let { if (it.startsWith("\"")) unquoteIfQuoted(it) else it.replace("\\\\", "\\") }
        if (fileHeaderMatch.groupValues[1] == "---") {
            index["oldFileName"] = fileName
            index["oldHeader"] = header
        } else {
            index["newFileName"] = fileName
            index["newHeader"] = header
        }
        i++
    }

    /** The hunk at the cursor. A header the range pattern does not match gives NaN starts. */
    private fun parseHunk(): Hunk {
        val chunkHeaderIndex = i
        val range = chunkHeader.find(diffstr[i++])?.groups
        fun number(group: Int) = range?.get(group)?.value?.toDouble()
        val hunk = Hunk(number(1) ?: Double.NaN, number(2) ?: 1.0, number(3) ?: Double.NaN, number(4) ?: 1.0)
        // Unified diff quirk: the start of an empty range is one lower.
        if (hunk.oldLines == 0.0) hunk.oldStart += 1
        if (hunk.newLines == 0.0) hunk.newStart += 1
        var addCount = 0
        var removeCount = 0
        while (i < diffstr.size && (removeCount < hunk.oldLines || addCount < hunk.newLines || diffstr[i].startsWith("\\"))) {
            val line = diffstr[i]
            // An empty line is context, except the last one (whose `[0]` is undefined).
            val operation = if (line.isEmpty() && i != diffstr.size - 1) ' ' else line.firstOrNull()
            when (operation) {
                '+' -> addCount++
                '-' -> removeCount++
                ' ' -> {
                    addCount++
                    removeCount++
                }
                '\\' -> {}
                else -> throw IllegalArgumentException("Hunk at line ${chunkHeaderIndex + 1} contained invalid line $line")
            }
            hunk.lines.add(line)
            i++
        }
        // The empty block count case.
        if (addCount == 0 && hunk.newLines == 1.0) hunk.newLines = 0.0
        if (removeCount == 0 && hunk.oldLines == 1.0) hunk.oldLines = 0.0
        require(addCount.toDouble() == hunk.newLines) { "Added line count did not match for hunk at line ${chunkHeaderIndex + 1}" }
        require(removeCount.toDouble() == hunk.oldLines) { "Removed line count did not match for hunk at line ${chunkHeaderIndex + 1}" }
        // A body line right after the counted ones means the counts were wrong, unless it is a file header.
        val next = diffstr.getOrNull(i)
        require(next.isNullOrEmpty() || !hunkBodyLine.containsMatchIn(next) || isFileHeader(next)) {
            "Hunk at line ${chunkHeaderIndex + 1} has more lines than expected"
        }
        return hunk
    }
}

// ---- applyPatch ----

/** applyPatch: the patched text, or null when a hunk does not fit. */
fun applyPatch(source: String, patches: List<FilePatch>): String? {
    require(patches.size <= 1) { "applyPatch only works with a single input." }
    val patch = requireNotNull(patches.firstOrNull()) { "applyPatch needs a patch" }
    return applyStructuredPatch(source, patch.hunks)
}

private fun hasOnlyWinLineEndings(s: String) =
    "\r\n" in s && !s.startsWith("\n") && s.indices.none { it > 0 && s[it] == '\n' && s[it - 1] != '\r' }

private fun hasOnlyUnixLineEndings(s: String) = "\r\n" !in s && "\n" in s

private fun List<String>.beforeMarker(i: Int) = getOrNull(i + 1)?.startsWith("\\") == true

private fun isUnix(hunks: List<Hunk>) = hunks.none { hunk -> hunk.lines.any { !it.startsWith("\\") && it.endsWith("\r") } }

private fun isWin(hunks: List<Hunk>) =
    hunks.any { hunk -> hunk.lines.any { it.endsWith("\r") } } &&
        hunks.all { hunk ->
            hunk.lines.withIndex().all { (i, line) -> line.startsWith("\\") || line.endsWith("\r") || hunk.lines.beforeMarker(i) }
        }

private fun List<Hunk>.mapLines(transform: (lines: List<String>, i: Int) -> String) = map { hunk ->
    Hunk(hunk.oldStart, hunk.oldLines, hunk.newStart, hunk.newLines, hunk.lines.indices.mapTo(ArrayList()) { transform(hunk.lines, it) })
}

private fun unixToWin(hunks: List<Hunk>) = hunks.mapLines { lines, i ->
    val line = lines[i]
    if (line.startsWith("\\") || line.endsWith("\r") || lines.beforeMarker(i)) line else "$line\r"
}

private fun winToUnix(hunks: List<Hunk>) = hunks.mapLines { lines, i -> lines[i].removeSuffix("\r") }

/**
 * distance-iterator.js: positions after [start], alternating forward and backward, within the
 * limits. Positions are JavaScript numbers, so a NaN start yields nothing, as in npm.
 */
private class DistanceIterator(private val start: Double, private val minLine: Double, private val maxLine: Double) {
    private var wantForward = true
    private var backwardExhausted = false
    private var forwardExhausted = false
    private var localOffset = 1.0

    fun next(): Double? {
        if (wantForward && !forwardExhausted) {
            if (backwardExhausted) localOffset++ else wantForward = false
            // Fit after the offset location, unless that is beyond the text.
            if (start + localOffset <= maxLine) return start + localOffset
            forwardExhausted = true
        }
        if (!backwardExhausted) {
            if (!forwardExhausted) wantForward = true
            // Fit before the offset location, unless that is before the text.
            if (minLine <= start - localOffset) return start - localOffset++
            backwardExhausted = true
            return next()
        }
        return null
    }
}

private class HunkResult(val patchedLines: List<String>, val oldLineLastI: Double)

/** `lines[i]`, where a position outside the source, or NaN, reads as undefined. */
private fun List<String>.at(i: Double): String? = if (i >= 0 && i < size) this[i.toInt()] else null

private fun applyStructuredPatch(source: String, patchHunks: List<Hunk>): String? {
    val hunks = when {
        hasOnlyWinLineEndings(source) && isUnix(patchHunks) -> unixToWin(patchHunks)
        hasOnlyUnixLineEndings(source) && isWin(patchHunks) -> winToUnix(patchHunks)
        else -> patchHunks
    }
    val lines = source.split("\n").toMutableList()
    if (hunks.isEmpty()) return source
    // A change to the newline at the end of the file is in the last hunk.
    var prevLine = ""
    var removeEOFNL = false
    var addEOFNL = false
    for (line in hunks.last().lines) {
        if (line.startsWith("\\")) {
            if (prevLine.startsWith("+")) removeEOFNL = true else if (prevLine.startsWith("-")) addEOFNL = true
        }
        prevLine = line
    }
    if (removeEOFNL) {
        if (addEOFNL) {
            // The last line changes but has no newline on either side: the source must not have one.
            if (lines.last() == "") return null
        } else if (lines.last() == "") {
            lines.removeAt(lines.lastIndex)
        } else {
            return null
        }
    } else if (addEOFNL) {
        if (lines.last() != "") lines.add("") else return null
    }

    /**
     * applyHunk with no errors allowed: every removed and context line must equal the source line at
     * its position. Trailing context is given back, so the next hunk may start inside it.
     */
    fun applyHunk(hunkLines: List<String>, position: Double): HunkResult? {
        var toPos = position
        val patchedLines = ArrayList<String>()
        var nConsecutiveOldContextLines = 0
        for (hunkLine in hunkLines) {
            val operation = hunkLine.firstOrNull() ?: ' '
            val content = if (hunkLine.isNotEmpty()) hunkLine.substring(1) else hunkLine
            when (operation) {
                '-' -> {
                    if (lines.at(toPos) != content) return null
                    toPos++
                    nConsecutiveOldContextLines = 0
                }
                '+' -> {
                    patchedLines.add(content)
                    nConsecutiveOldContextLines = 0
                }
                ' ' -> {
                    nConsecutiveOldContextLines++
                    if (lines.at(toPos) != content) return null
                    patchedLines.add(content)
                    toPos++
                }
            }
        }
        repeat(nConsecutiveOldContextLines) { patchedLines.removeAt(patchedLines.lastIndex) }
        return HunkResult(patchedLines, toPos - nConsecutiveOldContextLines - 1)
    }

    // Positions are JavaScript numbers: a NaN start fits only a hunk with no removed or context
    // line, and then every later position is NaN too. A position outside the source reads as
    // undefined, which joins as an empty line.
    val resultLines = ArrayList<String>()
    fun copy(from: Double, until: Double) {
        var i = from
        while (i < until) resultLines.add(lines.at(i++) ?: "")
    }
    var minLine = 0.0
    var prevHunkOffset = 0.0
    for (hunk in hunks) {
        val maxLine = lines.size - hunk.oldLines
        // The declared position first, then the positions around it.
        var toPos = hunk.oldStart + prevHunkOffset - 1
        val iterator = DistanceIterator(toPos, minLine, maxLine)
        var hunkResult = applyHunk(hunk.lines, toPos)
        while (hunkResult == null) {
            toPos = iterator.next() ?: return null
            hunkResult = applyHunk(hunk.lines, toPos)
        }
        copy(minLine, toPos)
        resultLines.addAll(hunkResult.patchedLines)
        // Later hunks may not fit over text this one patched.
        minLine = hunkResult.oldLineLastI + 1
        prevHunkOffset = toPos + 1 - hunk.oldStart
    }
    copy(minLine, lines.size.toDouble())
    return resultLines.joinToString("\n")
}

// ---- reversePatch, over the structured patch's own JSON values ----

@JsName("Reflect")
private external object JsReflect {
    fun deleteProperty(target: Any, key: String): Boolean
}


private fun swapPrefix(fileName: Any?): Any? {
    if (fileName === undefined || fileName == "/dev/null") return fileName
    val name = fileName.unsafeCast<String>()
    return when {
        name.startsWith("a/") -> "b/" + name.substring(2)
        name.startsWith("b/") -> "a/" + name.substring(2)
        else -> name
    }
}

private fun reverseLine(line: String) = when {
    line.startsWith("-") -> "+" + line.substring(1)
    line.startsWith("+") -> "-" + line.substring(1)
    else -> line
}

/** reversePatch: a copy of each structured patch with its sides swapped, the list reversed. */
fun reversePatch(structuredPatch: Any?): Any? {
    if (structuredPatch.isJsArray()) {
        return structuredPatch.unsafeCast<Array<Any?>>().map(::reversePatch).asReversed().toTypedArray()
    }
    val patch = structuredPatch.unsafeCast<Json>()
    val isGit = truthy(patch["isGit"])
    fun name(key: String) = if (isGit) swapPrefix(patch[key]) else patch[key]
    // Object.assign({}, patch, {...}): the patch's own keys first, then the new ones.
    val reversed = json()
    for (key in JsObject.keys(patch)) reversed[key] = patch[key]
    reversed["oldFileName"] = name("newFileName")
    reversed["oldHeader"] = patch["newHeader"]
    reversed["newFileName"] = name("oldFileName")
    reversed["newHeader"] = patch["oldHeader"]
    reversed["oldMode"] = patch["newMode"]
    reversed["newMode"] = patch["oldMode"]
    reversed["isCreate"] = patch["isDelete"]
    reversed["isDelete"] = patch["isCreate"]
    reversed["hunks"] = patch["hunks"].unsafeCast<Array<Json>>().map { hunk ->
        json(
            "oldLines" to hunk["newLines"], "oldStart" to hunk["newStart"],
            "newLines" to hunk["oldLines"], "newStart" to hunk["oldStart"],
            "lines" to hunk["lines"].unsafeCast<Array<String>>().map(::reverseLine).toTypedArray(),
        )
    }.toTypedArray()
    if (truthy(patch["isCopy"])) {
        // Reversing a copy deletes the copy destination; the hunks cannot be reversed into a deletion.
        reversed["newFileName"] = "/dev/null"
        reversed["newHeader"] = undefined
        reversed["isDelete"] = true
        for (key in listOf("isCreate", "isCopy", "isRename")) JsReflect.deleteProperty(reversed, key)
        reversed["hunks"] = emptyArray<Json>()
    }
    return reversed
}

/** A structured patch from JSON, for applyPatch. */
fun readFilePatch(value: Any?): FilePatch {
    val hunks = value.asDynamic().hunks.unsafeCast<Array<Json>>().mapTo(ArrayList()) { hunk ->
        Hunk(
            hunk["oldStart"].unsafeCast<Double>(), hunk["oldLines"].unsafeCast<Double>(),
            hunk["newStart"].unsafeCast<Double>(), hunk["newLines"].unsafeCast<Double>(),
            hunk["lines"].unsafeCast<Array<String>>().toMutableList(),
        )
    }
    return FilePatch(hunks)
}
