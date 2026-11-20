package fflate

import kotlin.js.Date
import kotlin.math.floor

internal class Crc32 {
    private var c: Int = -1

    fun push(d: ByteArray) {
        var cr = c
        val tbl = CRCT
        for (i in 0 until d.size) {
            cr = tbl[(cr and 255) xor (d[i].toInt() and 0xFF)] xor (cr ushr 8)
        }
        c = cr
    }

    fun digest(): Int = c.inv()
}

internal class Adler32 {
    private var a: Int = 1
    private var b: Int = 0

    fun push(d: ByteArray) {
        var n = a
        var m = b
        val l = d.size
        var i = 0
        while (i != l) {
            val e = if (i + 2655 < l) i + 2655 else l
            while (i < e) {
                n += d[i].toInt() and 0xFF
                m += n
                i++
            }
            n = (n and 65535) + 15 * (n shr 16)
            m = (m and 65535) + 15 * (m shr 16)
        }
        a = n
        b = m
    }

    fun digest(): Int {
        a %= 65521
        b %= 65521
        return ((a and 255) shl 24) or ((a and 0xFF00) shl 8) or ((b and 255) shl 8) or (b ushr 8)
    }
}

internal fun wbytes(d: ByteArray, start: Int, value: Int) {
    var b = start
    var v = value
    while (v != 0) {
        d[b++] = v.toByte()
        v = v ushr 8
    }
}

// Convert a non-negative integer Double in [0, 2^32 - 1] to its unsigned 32-bit Int bit pattern
// without hitting Kotlin's Double.toInt() saturation at 2^31 - 1.
private fun doubleToU32Bits(v: Double): Int {
    val mod = v % 4294967296.0
    val nonNeg = if (mod < 0.0) mod + 4294967296.0 else mod
    return if (nonNeg >= 2147483648.0) (nonNeg - 4294967296.0).toInt() else nonNeg.toInt()
}

internal fun gzhl(o: CompressOptions): Int {
    val fn = o.filename
    return 10 + if (!fn.isNullOrEmpty()) fn.length + 1 else 0
}

internal fun gzh(c: ByteArray, o: CompressOptions) {
    val fn = o.filename
    val lvl = o.level
    c[0] = 31
    c[1] = 139.toByte()
    c[2] = 8
    c[8] = (if (lvl != null && lvl < 2) 4 else if (lvl == 9) 2 else 0).toByte()
    c[9] = 3
    val mtime = o.mtime
    if (mtime == null || mtime != 0.0) {
        val ms = mtime ?: Date.now()
        wbytes(c, 4, doubleToU32Bits(floor(ms / 1000.0)))
    }
    if (!fn.isNullOrEmpty()) {
        c[3] = 8
        for (i in 0 until fn.length) {
            c[i + 10] = fn[i].code.toByte()
        }
        c[fn.length + 10] = 0
    }
}

internal fun gzs(d: ByteArray): Int {
    if (byteAt(d, 0) != 31 || byteAt(d, 1) != 139 || byteAt(d, 2) != 8) {
        throw flateErr(6, "invalid gzip data")
    }
    val flg = byteAt(d, 3)
    var st = 10
    if ((flg and 4) != 0) {
        st += (byteAt(d, 10) or (byteAt(d, 11) shl 8)) + 2
    }
    var zs = ((flg ushr 3) and 1) + ((flg ushr 4) and 1)
    while (zs > 0) {
        if (byteAt(d, st++) == 0) zs--
    }
    return st + (flg and 2)
}

internal fun gzl(d: ByteArray): Int {
    val l = d.size
    return byteAt(d, l - 4) or
        (byteAt(d, l - 3) shl 8) or
        (byteAt(d, l - 2) shl 16) or
        (byteAt(d, l - 1) shl 24)
}

internal fun gzipSync(data: ByteArray, opts: CompressOptions?): ByteArray {
    val o = opts ?: CompressOptions()
    val c = Crc32()
    val l = data.size
    c.push(data)
    val d = dopt(data, o, gzhl(o), 8)
    val s = d.size
    gzh(d, o)
    wbytes(d, s - 8, c.digest())
    wbytes(d, s - 4, l)
    return d
}

internal fun gunzipSync(data: ByteArray, opts: DecompressOptions?): ByteArray {
    val st = gzs(data)
    if (st + 8 > data.size) throw flateErr(6, "invalid gzip data")
    val outBuf = opts?.out ?: run {
        val sz = gzl(data)
        if (sz < 0) null else ByteArray(sz)
    }
    return infltSync(data, st, data.size - 8, outBuf, opts?.dictionary)
}

internal fun zlh(c: ByteArray, o: CompressOptions) {
    val lv = o.level
    val fl = if (lv == 0) 0 else if (lv != null && lv < 6) 1 else if (lv == 9) 3 else 2
    val c0 = 120
    var c1 = (fl shl 6) or (if (o.dictionary != null) 32 else 0)
    c1 = c1 or (31 - (((c0 shl 8) or c1) % 31))
    c[0] = c0.toByte()
    c[1] = c1.toByte()
    val dict = o.dictionary
    if (dict != null) {
        val h = Adler32()
        h.push(dict)
        wbytes(c, 2, h.digest())
    }
}

internal fun zls(d: ByteArray, hasDict: Boolean): Int {
    val d0 = byteAt(d, 0)
    val d1 = byteAt(d, 1)
    if ((d0 and 15) != 8 || (d0 ushr 4) > 7 || (((d0 shl 8) or d1) % 31) != 0) {
        throw flateErr(6, "invalid zlib data")
    }
    if (((d1 ushr 5) and 1) == (if (hasDict) 0 else 1)) {
        val reason = if ((d1 and 32) != 0) "need" else "unexpected"
        throw flateErr(6, "invalid zlib data: $reason dictionary")
    }
    return ((d1 ushr 3) and 4) + 2
}

internal fun zlibSync(data: ByteArray, opts: CompressOptions?): ByteArray {
    val o = opts ?: CompressOptions()
    val a = Adler32()
    a.push(data)
    val d = dopt(data, o, if (o.dictionary != null) 6 else 2, 4)
    zlh(d, o)
    wbytes(d, d.size - 4, a.digest())
    return d
}

internal fun unzlibSync(data: ByteArray, opts: DecompressOptions?): ByteArray {
    val start = zls(data, opts?.dictionary != null)
    val end = maxOf(start, data.size - 4)
    return infltSync(data, start, end, opts?.out, opts?.dictionary)
}

internal fun decompressSync(data: ByteArray, opts: DecompressOptions?): ByteArray {
    val d0 = byteAt(data, 0)
    val d1 = byteAt(data, 1)
    val d2 = byteAt(data, 2)
    return if (d0 == 31 && d1 == 139 && d2 == 8) {
        gunzipSync(data, opts)
    } else if ((d0 and 15) != 8 || (d0 ushr 4) > 7 || (((d0 shl 8) or d1) % 31) != 0) {
        inflateSync(data, opts)
    } else {
        unzlibSync(data, opts)
    }
}
