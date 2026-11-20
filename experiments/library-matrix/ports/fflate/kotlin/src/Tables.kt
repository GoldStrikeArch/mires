package fflate

internal val EMPTY_BYTES = ByteArray(0)
internal val EMPTY_INTS = IntArray(0)

internal val FLEB = intArrayOf(
    0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 2, 2, 2, 2,
    3, 3, 3, 3, 4, 4, 4, 4,
    5, 5, 5, 5, 0, 0, 0, 0,
)

internal val FDEB = intArrayOf(
    0, 0, 0, 0, 1, 1, 2, 2,
    3, 3, 4, 4, 5, 5, 6, 6,
    7, 7, 8, 8, 9, 9, 10, 10,
    11, 11, 12, 12, 13, 13, 0, 0,
)

internal val CLIM = intArrayOf(
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
)

internal class FrebTable(val b: IntArray, val r: IntArray)

private fun freb(eb: IntArray, startInit: Int): FrebTable {
    val b = IntArray(31)
    var start = startInit
    for (i in 0 until 31) {
        val shift = if (i > 0) eb[i - 1] else 0
        start += 1 shl shift
        b[i] = start
    }
    val r = IntArray(b[30])
    for (i in 1 until 30) {
        val lo = b[i]
        val hi = b[i + 1]
        for (j in lo until hi) {
            r[j] = ((j - lo) shl 5) or i
        }
    }
    return FrebTable(b, r)
}

private val FL_TABLE = freb(FLEB, 2).also {
    it.b[28] = 258
    it.r[258] = 28
}

internal val FL: IntArray = FL_TABLE.b
internal val REVFL: IntArray = FL_TABLE.r

private val FD_TABLE = freb(FDEB, 0)

internal val FD: IntArray = FD_TABLE.b
internal val REVFD: IntArray = FD_TABLE.r

internal val REV: ShortArray = ShortArray(32768).also { rev ->
    for (i in 0 until 32768) {
        var x = ((i and 0xAAAA) ushr 1) or ((i and 0x5555) shl 1)
        x = ((x and 0xCCCC) ushr 2) or ((x and 0x3333) shl 2)
        x = ((x and 0xF0F0) ushr 4) or ((x and 0x0F0F) shl 4)
        rev[i] = ((((x and 0xFF00) ushr 8) or ((x and 0x00FF) shl 8)) ushr 1).toShort()
    }
}

internal val FLT: IntArray = IntArray(288).also { flt ->
    for (i in 0 until 144) flt[i] = 8
    for (i in 144 until 256) flt[i] = 9
    for (i in 256 until 280) flt[i] = 7
    for (i in 280 until 288) flt[i] = 8
}

internal val FDT: IntArray = IntArray(32) { 5 }

internal val FLM: IntArray = hMap(FLT, 9, false)
internal val FLRM: IntArray = hMap(FLT, 9, true)
internal val FDM: IntArray = hMap(FDT, 5, false)
internal val FDRM: IntArray = hMap(FDT, 5, true)

// Deflate options packed as (nice << 13) | chain for levels 1..9.
internal val DEO = intArrayOf(
    65540, 131080, 131088, 131104, 262176, 1048704, 1048832, 2114560, 2117632,
)

internal val CRCT: IntArray = IntArray(256).also { t ->
    for (i in 0 until 256) {
        var c = i
        var k = 9
        while (--k != 0) {
            c = (if ((c and 1) != 0) -306674912 else 0) xor (c ushr 1)
        }
        t[i] = c
    }
}

internal class FlateException(val code: Int, message: String) : RuntimeException(message)

private val ERROR_MESSAGES = arrayOf(
    "unexpected EOF",
    "invalid block type",
    "invalid length/literal",
    "invalid distance",
    "stream finished",
    "no stream handler",
    "invalid header",
    "no callback",
    "invalid UTF-8 data",
)

internal fun flateErr(code: Int, msg: String? = null): FlateException =
    FlateException(code, msg ?: if (code in ERROR_MESSAGES.indices) ERROR_MESSAGES[code] else "flate error $code")

internal fun shft(p: Int): Int = (p + 7) shr 3

internal fun byteAt(d: ByteArray, index: Int): Int =
    if (index >= 0 && index < d.size) d[index].toInt() and 0xFF else 0

internal fun slc(v: ByteArray, start: Int, end: Int): ByteArray {
    val s = if (start < 0) 0 else start
    val e = if (end > v.size) v.size else end
    if (s >= e) return EMPTY_BYTES
    return v.copyOfRange(s, e)
}

internal fun maxOfArray(a: IntArray): Int {
    var m = a[0]
    for (i in 1 until a.size) {
        if (a[i] > m) m = a[i]
    }
    return m
}

// Exact integer cutoffs for Math.ceil(Math.max(8, Math.min(13, Math.log(len))) * 1.5).
internal fun defaultPlvl(len: Int): Int = when {
    len <= 2980 -> 12
    len <= 5806 -> 13
    len <= 11308 -> 14
    len <= 22026 -> 15
    len <= 42901 -> 16
    len <= 83561 -> 17
    len <= 162754 -> 18
    len <= 317003 -> 19
    else -> 20
}

internal fun hMap(cd: IntArray, mb: Int, reverse: Boolean): IntArray {
    val s = cd.size
    val l = IntArray(mb)
    for (i in 0 until s) {
        val len = cd[i]
        if (len != 0) l[len - 1]++
    }
    val le = IntArray(mb)
    for (i in 1 until mb) {
        le[i] = ((le[i - 1] + l[i - 1]) shl 1) and 0xFFFF
    }
    val rev = REV
    if (reverse) {
        val co = IntArray(1 shl mb)
        val rvb = 15 - mb
        for (i in 0 until s) {
            val len = cd[i]
            if (len != 0) {
                val sv = (i shl 4) or len
                val r1 = mb - len
                val code = le[len - 1]
                le[len - 1] = (code + 1) and 0xFFFF
                var v = code shl r1
                val m = v or ((1 shl r1) - 1)
                while (v <= m) {
                    val idx = if (v < 32768) rev[v].toInt() ushr rvb else 0
                    co[idx] = sv
                    v++
                }
            }
        }
        return co
    } else {
        val co = IntArray(s)
        for (i in 0 until s) {
            val len = cd[i]
            if (len != 0) {
                val code = le[len - 1]
                le[len - 1] = (code + 1) and 0xFFFF
                co[i] = if (code < 32768) rev[code].toInt() ushr (15 - len) else 0
            }
        }
        return co
    }
}
