package marked

private val getEscapeReplacement: (String) -> String = { ch ->
    when (ch) {
        "&" -> "&amp;"
        "<" -> "&lt;"
        ">" -> "&gt;"
        "\"" -> "&quot;"
        "'" -> "&#39;"
        else -> ch
    }
}

fun escapeHtmlEntities(html: String, encode: Boolean = false): String {
    if (encode) {
        if (OTHER_RULES.escapeTest.test(html)) {
            return html.asJs().replaceFn1(OTHER_RULES.escapeReplace, getEscapeReplacement)
        }
    } else {
        if (OTHER_RULES.escapeTestNoEncode.test(html)) {
            return html.asJs().replaceFn1(OTHER_RULES.escapeReplaceNoEncode, getEscapeReplacement)
        }
    }
    return html
}

private val decodeNcrReplacer: (String, String?, String?) -> String = { _, dec, hex ->
    if (dec == null && hex == null) {
        "&amp;"
    } else {
        val code = if (dec == null) parseInt(hex.unsafeCast<String>(), 16) else parseInt(dec, 10)
        if (code == 0 || code > 0x10FFFF || (code >= 0xD800 && code <= 0xDFFF)) {
            "\uFFFD"
        } else if (code == 38) {
            "&amp;"
        } else {
            JsStringStatic.fromCodePoint(code)
        }
    }
}

fun decodeNumericCharacterReferences(text: String): String {
    if (!OTHER_RULES.numericCharacterReference.test(text)) {
        return text
    }
    return text.asJs().replaceFn3(OTHER_RULES.numericCharacterReferenceOrAmpersand, decodeNcrReplacer)
}

fun cleanUrl(href: String): String? {
    return try {
        encodeURI(href).asJs().replaceRe(OTHER_RULES.percentDecode, "%")
    } catch (_: Throwable) {
        null
    }
}

private val findPipeReplacer: (String, Int, String) -> String = { _, offset, str ->
    var escaped = false
    var curr = offset
    while (--curr >= 0 && str[curr] == '\\') {
        escaped = !escaped
    }
    if (escaped) "|" else " |"
}

fun splitCells(tableRow: String, count: Int = 0): JsArray<String> {
    val row = tableRow.asJs().replaceOffset(OTHER_RULES.findPipe, findPipeReplacer)
    val cells = row.asJs().splitRe(OTHER_RULES.splitPipe)

    if (cells[0].asJs().trim().length == 0) {
        cells.shift()
    }
    if (cells.length > 0 && cells[cells.length - 1].asJs().trim().length == 0) {
        cells.pop()
    }

    if (count > 0) {
        if (cells.length > count) {
            cells.splice(count)
        } else {
            while (cells.length < count) {
                cells.push("")
            }
        }
    }

    for (i in 0 until cells.length) {
        cells[i] = cells[i].asJs().trim().asJs().replaceRe(OTHER_RULES.slashPipe, "|")
    }
    return cells
}

fun rtrim(str: String, c: Char, invert: Boolean = false): String {
    val l = str.length
    if (l == 0) {
        return ""
    }
    var suffLen = 0
    while (suffLen < l) {
        val currChar = str[l - suffLen - 1]
        if (currChar == c && !invert) {
            suffLen++
        } else if (currChar != c && invert) {
            suffLen++
        } else {
            break
        }
    }
    return str.asJs().slice(0, l - suffLen)
}

fun trimTrailingBlankLines(str: String): String {
    val lines = str.asJs().splitStr("\n")
    var end = lines.length - 1
    while (end >= 0 && OTHER_RULES.blankLine.test(lines[end])) {
        end--
    }
    if (lines.length - end <= 2) {
        return str
    }
    return lines.slice(0, end + 1).join("\n")
}

fun normalizeLabel(label: String): String {
    return label.asJs().trim().asJs().toLowerCase().asJs().toUpperCase().asJs().toLowerCase()
}

fun findClosingBracket(str: String, b: String): Int {
    val openStr = b.asJs().substring(0, 1)
    val closeStr = b.asJs().substring(1, 2)
    if (str.asJs().indexOf(openStr) == -1 && str.asJs().indexOf(closeStr) == -1) {
        return -1
    }
    val open = b[0]
    val close = b[1]
    var level = 0
    var i = 0
    val len = str.length
    while (i < len) {
        val ch = str[i]
        if (ch == '\\') {
            i++
        } else if (ch == open) {
            level++
        } else if (ch == close) {
            level--
            if (level < 0) {
                return i
            }
        }
        i++
    }
    if (level > 0) {
        return -2
    }
    return -1
}

fun expandTabs(line: String, indent: Int = 0): String {
    var col = indent
    var expanded = ""
    var i = 0
    val len = line.length
    while (i < len) {
        val c = line[i]
        if (c == '\t') {
            val added = 4 - (col % 4)
            expanded += " ".asJs().repeat(added)
            col += added
            i++
        } else {
            val code = c.code
            if (code >= 0xD800 && code <= 0xDBFF && i + 1 < len) {
                val next = line[i + 1].code
                if (next >= 0xDC00 && next <= 0xDFFF) {
                    expanded += line.asJs().substring(i, i + 2)
                    col++
                    i += 2
                    continue
                }
            }
            expanded += line.asJs().substring(i, i + 1)
            col++
            i++
        }
    }
    return expanded
}

fun codePointCount(s: String): Int {
    var count = 0
    var i = 0
    val len = s.length
    while (i < len) {
        val c = s[i].code
        if (c >= 0xD800 && c <= 0xDBFF && i + 1 < len) {
            val next = s[i + 1].code
            if (next >= 0xDC00 && next <= 0xDFFF) {
                i += 2
                count++
                continue
            }
        }
        i++
        count++
    }
    return count
}

fun firstCodePointLength(s: String): Int {
    if (s.length >= 2) {
        val c = s[0].code
        val next = s[1].code
        if (c >= 0xD800 && c <= 0xDBFF && next >= 0xDC00 && next <= 0xDFFF) {
            return 2
        }
    }
    return 1
}
