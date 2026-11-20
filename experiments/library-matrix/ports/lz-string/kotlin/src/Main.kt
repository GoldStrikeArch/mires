private const val base64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/="
private const val uri = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+-$"

private fun width(format: String) = when (format) {
    "utf16" -> 15
    "base64", "uri" -> 6
    "raw", "bytes" -> 16
    else -> error("unknown format")
}

private class Width(var bits: Int, var left: Int) {
    fun advance() {
        if (--left == 0) { left = 1 shl bits; bits++ }
    }
}

private fun compress(text: String, format: String): List<Int> {
    val dictionary = HashMap<String, Int>()
    val pending = HashSet<String>()
    val words = ArrayList<Int>()
    val width = width(format)
    val codes = Width(2, 2)
    var word = 0
    var used = 0
    var next = 3
    fun write(count: Int, value: Int) {
        for (bit in 0 until count) {
            word = (word shl 1) or ((value ushr bit) and 1)
            if (++used == width) { words.add(word); word = 0; used = 0 }
        }
    }
    fun emit(phrase: String) {
        if (pending.remove(phrase)) {
            val c = phrase[0].code
            write(codes.bits, if (c < 256) 0 else 1)
            write(if (c < 256) 8 else 16, c)
            codes.advance()
        } else write(codes.bits, dictionary.getValue(phrase))
        codes.advance()
    }
    var prefix = ""
    for (unit in text) {
        val c = unit.toString()
        if (c !in dictionary) { dictionary[c] = next++; pending.add(c) }
        val combined = prefix + c
        if (combined in dictionary) { prefix = combined; continue }
        emit(prefix)
        if (combined != "__proto__") dictionary[combined] = next
        next++
        prefix = c
    }
    if (prefix.isNotEmpty()) emit(prefix)
    write(codes.bits, 2)
    write(width - used, 0)
    return when (format) {
        "raw" -> words
        "utf16" -> words.map { it + 32 } + 32
        "bytes" -> words.flatMap { listOf(it ushr 8, it and 255) }
        else -> {
            val alphabet = if (format == "uri") uri else base64
            val out = words.mapTo(ArrayList()) { alphabet[it].code }
            if (format == "base64") while (out.size % 4 != 0) out.add(61)
            out
        }
    }
}

private fun decompress(data: List<Int>, format: String): String? {
    if (data.isEmpty()) return null
    val width = width(format)
    val words = when (format) {
        "raw" -> data
        "utf16" -> data.map { it - 32 }
        "bytes" -> {
            require(data.size % 2 == 0)
            List(data.size / 2) { data[2 * it] * 256 + data[2 * it + 1] }
        }
        else -> {
            val alphabet = if (format == "uri") uri else base64
            data.map { alphabet.indexOf(if (format == "uri" && it == 32) '+' else it.toChar()).coerceAtLeast(0) }
        }
    }
    var index = 0
    var position = width
    fun read(count: Int): Int {
        var value = 0
        for (bit in 0 until count) {
            val word = words.getOrElse(index) { 0 }
            value = value or (((word ushr --position) and 1) shl bit)
            if (position == 0) { position = width; index++ }
        }
        return value
    }
    val header = read(2)
    if (header == 2) return ""
    var previous = if (header == 3) null else read(if (header == 0) 8 else 16).toChar().toString()
    val dictionary = arrayListOf(null, null, null, previous)
    val result = StringBuilder(previous ?: "")
    val codes = Width(3, 4)
    while (index < words.size) {
        var code = read(codes.bits)
        if (code == 2) return result.toString()
        if (code <= 1) {
            val c = read(if (code == 0) 8 else 16).toChar().toString()
            code = dictionary.size
            dictionary.add(c)
            codes.advance()
        }
        val entry = when {
            code < dictionary.size -> dictionary[code] ?: return null
            code == dictionary.size -> previous!! + previous[0]
            else -> return null
        }
        result.append(entry)
        dictionary.add((previous ?: "undefined") + entry[0])
        previous = entry
        codes.advance()
    }
    return ""
}

external interface Input {
    val op: String
    val format: String
    val data: Array<Int>?
    val repeat: Int?
}

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: Input): Array<Int>? {
    val data = input.data ?: return emptyArray()
    if (input.op == "decompress") return decompress(data.asList(), input.format)?.map { it.code }?.toTypedArray()
    val text = buildString { for (c in data) append(c.toChar()) }.repeat(input.repeat ?: 1)
    val packed = compress(text, input.format)
    if (input.op == "compress") return packed.toTypedArray()
    require(input.op == "roundtrip" && decompress(packed, input.format) == text)
    return arrayOf(packed.size, text.length)
}
