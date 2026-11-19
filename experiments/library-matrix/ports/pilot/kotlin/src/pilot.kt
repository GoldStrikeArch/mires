// Word frequencies, FNV-1a and a fixed-decimal summary, as specified in
// libraries/pilot/CONTRACT.md. Kotlin strings are UTF-16; code points are assembled from
// surrogate pairs because String.codePointAt is JVM-only.

import kotlin.math.roundToInt

class Entry(val word: String, val length: Int) {
    var count = 1
}

private fun isWordCodePoint(cp: Int): Boolean =
    cp >= 0x80 || cp in 0x30..0x39 || cp in 0x41..0x5a || cp in 0x61..0x7a

/** FNV-1a 32 over the UTF-8 bytes of [text]; UInt arithmetic wraps at 32 bits. */
fun fnv1a(text: String): UInt =
    text.encodeToByteArray().fold(0x811c9dc5u) { hash, byte -> (hash xor byte.toUByte().toUInt()) * 0x01000193u }

private fun hex8(value: UInt): String = value.toString(16).padStart(8, '0')

/** Fixed three decimals; `roundToInt` rounds ties towards positive infinity like JS toFixed. */
private fun fixed3(value: Double): String {
    val scaled = (value * 1000).roundToInt()
    return "${scaled / 1000}.${(scaled % 1000).toString().padStart(3, '0')}"
}

fun analyse(text: String, top: Int): String {
    val entries = LinkedHashMap<String, Entry>()
    var tokens = 0
    var codePoints = 0
    val word = StringBuilder()
    var length = 0
    fun flush() {
        if (length == 0) return
        tokens++
        codePoints += length
        val key = word.toString()
        val entry = entries[key]
        if (entry != null) entry.count++ else entries[key] = Entry(key, length)
        word.clear()
        length = 0
    }

    var index = 0
    while (index < text.length) {
        val high = text[index]
        val pair = high.isHighSurrogate() && index + 1 < text.length && text[index + 1].isLowSurrogate()
        val cp = if (pair) 0x10000 + ((high.code - 0xd800) shl 10) + (text[index + 1].code - 0xdc00) else high.code
        if (isWordCodePoint(cp)) {
            // ASCII-only case folding.
            if (cp in 0x41..0x5a) word.append((cp + 0x20).toChar())
            else if (pair) word.append(high).append(text[index + 1])
            else word.append(high)
            length++
        } else {
            flush()
        }
        index += if (pair) 2 else 1
    }
    flush()

    // sortedWith is stable; LinkedHashMap iterates in first-occurrence order.
    val ranked = entries.values.sortedWith(compareByDescending<Entry> { it.count }.thenByDescending { it.length })
    val mean = if (tokens == 0) 0.0 else codePoints.toDouble() / tokens
    val lines = mutableListOf(
        "tokens $tokens",
        "distinct ${entries.size}",
        "mean ${fixed3(mean)}",
        "fnv ${hex8(fnv1a(text))}",
    )
    for (entry in ranked.take(top)) {
        lines += "${entry.word} ${entry.count} ${hex8(fnv1a(entry.word))} ${fixed3(entry.count.toDouble() / tokens)}"
    }
    return lines.joinToString("\n")
}
