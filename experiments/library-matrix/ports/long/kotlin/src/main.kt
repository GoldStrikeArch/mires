external interface Command {
    val op: String
    val s: String
    val n: Int
    val r: Int
    val a: Array<Int>
    val b: Array<Int>
    val u: Boolean
    val v: Boolean
}
external interface Input { val cases: Array<Command> }

private fun radix(value: Int): Int {
    if (value == 0) return 10
    require(value in 2..36)
    return value
}

private fun parse(text: String, base: Int): Long {
    require(text.isNotEmpty())
    if (text == "NaN" || text == "Infinity" || text == "+Infinity" || text == "-Infinity") return 0
    val r = radix(base)
    if (text[0] == '-') return -parse(text.substring(1), r)
    require('-' !in text)
    var result = 0L
    var start = 0
    while (start < text.length) {
        val end = minOf(start+8, text.length)
        var i = start
        while (i < end && text[i] in " \u000b\u000c\r\n\t") i++
        if (i < end && text[i] == '+') i++
        if (r == 16 && i+1 < end && text[i] == '0' && text[i+1] in "xX") i+=2
        var part = 0L
        while (i < end) {
            val d = text[i].digitToIntOrNull(r) ?: break
            part = part*r+d
            i++
        }
        repeat(end-start) { result *= r }
        result += part
        start = end
    }
    return result
}

private fun words(p: Array<Int>): Long = (p[1].toLong() shl 32) or (p[0].toLong() and 0xffffffffL)

private fun format(a: Long, unsigned: Boolean, r: Int): String =
    if (unsigned) a.toULong().toString(radix(r)) else a.toString(radix(r))

private fun divide(a: Long, b: Long, u: Boolean): Long = when {
    u -> (a.toULong() / b.toULong()).toLong()
    (a shr 32) == Int.MIN_VALUE.toLong() && b == -1L -> a
    else -> a / b
}

private fun compare(a: Long, b: Long, u: Boolean, v: Boolean): Int {
    if (a == b && (u == v || a >= 0)) return 0
    val an = !u && a < 0
    val bn = !v && b < 0
    val less = if (an != bn) an else if (u) a.toULong() < b.toULong() else a-b < 0
    return if (less) -1 else 1
}

private fun evaluate(c: Command): String {
    val a = words(c.a)
    val b = words(c.b)
    val n = c.n
    val value = when (c.op) {
        "fromBits", "toString" -> a
        "fromString" -> parse(c.s,n)
        "add" -> a+b
        "subtract" -> a-b
        "multiply" -> a*b
        "divide" -> if (b == 0L) return "!" else divide(a,b,c.u)
        "modulo" -> if (b == 0L) return "!" else if (c.u) (a.toULong() % b.toULong()).toLong() else a % b
        "and" -> a and b
        "or" -> a or b
        "xor" -> a xor b
        "shiftLeft" -> a shl n
        "shiftRight" -> a shr n
        "shiftRightUnsigned" -> a ushr n
        "rotl" -> a.rotateLeft(n)
        "clz" -> return a.countLeadingZeroBits().toString()
        "ctz" -> return a.countTrailingZeroBits().toString()
        "compare" -> return compare(a,b,c.u,c.v).toString()
        "mix" -> {
            var acc = a
            var checksum = 0
            for (i in 0 until n) {
                acc = (acc*(b or 1L)+i+1).rotateLeft(11) xor b
                acc += divide(acc,(3+(i and 255)).toLong(),c.u)
                if ((i and 7) == 0) {
                    val s = format(acc,c.u,c.r)
                    checksum = checksum*33+s.length+s[0].code
                }
            }
            return format(acc,c.u,c.r)+":"+checksum
        }
        else -> return "!"
    }
    return format(value,c.u,c.r)
}

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: Input): String = input.cases.joinToString("\n") {
    try { evaluate(it) } catch (_: IllegalArgumentException) { "!" }
}
