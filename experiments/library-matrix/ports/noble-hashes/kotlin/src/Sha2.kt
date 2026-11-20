// SHA-256 and SHA-512 compression and streaming HMAC, adapted from @noble/hashes 2.4.0.
object Sha2 {

    private val shaIV256 = intArrayOf(
        1779033703, -1150833019, 1013904242, -1521486534, 1359893119, -1694144372,
        528734635, 1541459225,
    )

    private val shaIV512 = intArrayOf(
        1779033703, -205731576, -1150833019, -2067093701, 1013904242, -23791573,
        -1521486534, 1595750129, 1359893119, -1377402159, -1694144372, 725511199,
        528734635, -79577749, 1541459225, 327033209,
    )

    private val shaK256 = intArrayOf(
        1116352408, 1899447441, -1245643825, -373957723, 961987163, 1508970993,
        -1841331548, -1424204075, -670586216, 310598401, 607225278, 1426881987,
        1925078388, -2132889090, -1680079193, -1046744716, -459576895, -272742522,
        264347078, 604807628, 770255983, 1249150122, 1555081692, 1996064986,
        -1740746414, -1473132947, -1341970488, -1084653625, -958395405, -710438585,
        113926993, 338241895, 666307205, 773529912, 1294757372, 1396182291,
        1695183700, 1986661051, -2117940946, -1838011259, -1564481375, -1474664885,
        -1035236496, -949202525, -778901479, -694614492, -200395387, 275423344,
        430227734, 506948616, 659060556, 883997877, 958139571, 1322822218,
        1537002063, 1747873779, 1955562222, 2024104815, -2067236844, -1933114872,
        -1866530822, -1538233109, -1090935817, -965641998,
    )

    private val shaK512h = intArrayOf(
        1116352408, 1899447441, -1245643825, -373957723, 961987163, 1508970993,
        -1841331548, -1424204075, -670586216, 310598401, 607225278, 1426881987,
        1925078388, -2132889090, -1680079193, -1046744716, -459576895, -272742522,
        264347078, 604807628, 770255983, 1249150122, 1555081692, 1996064986,
        -1740746414, -1473132947, -1341970488, -1084653625, -958395405, -710438585,
        113926993, 338241895, 666307205, 773529912, 1294757372, 1396182291,
        1695183700, 1986661051, -2117940946, -1838011259, -1564481375, -1474664885,
        -1035236496, -949202525, -778901479, -694614492, -200395387, 275423344,
        430227734, 506948616, 659060556, 883997877, 958139571, 1322822218,
        1537002063, 1747873779, 1955562222, 2024104815, -2067236844, -1933114872,
        -1866530822, -1538233109, -1090935817, -965641998, -903397682, -779700025,
        -354779690, -176337025, 116418474, 174292421, 289380356, 460393269,
        685471733, 852142971, 1017036298, 1126000580, 1288033470, 1501505948,
        1607167915, 1816402316,
    )

    private val shaK512l = intArrayOf(
        -685199838, 602891725, -330482897, -2121671748, -213338824, -1241133031,
        -1357295717, -630357736, -1560083902, 1164996542, 1323610764, -704662302,
        -226784913, 991336113, 633803317, -815192428, -1628353838, 944711139,
        -1953704523, 2007800933, 1495990901, 1856431235, -1119749164, -2096016459,
        -295247957, 766784016, -1728372417, -1091629340, 1034457026, -1828018395,
        -536640913, 168717936, 1188179964, 1546045734, 1522805485, -1651133473,
        -1951439906, 1014477480, 1206759142, 344077627, 1290863460, -1136513023,
        -789014639, 106217008, -688958952, 1432725776, 1467031594, 851169720,
        -1194143544, 1363258195, -544281703, -509917016, -976659869, -482243893,
        2003034995, -692930397, 1575990012, 1125592928, -1578062990, 442776044,
        593698344, -561857047, -1295615723, -479046869, -366583396, 566280711,
        -840897762, -294727304, 1914138554, -1563912026, -1090974290, 320620315,
        587496836, 1086792851, 365543100, -1676669620, -885112138, -60457430,
        987167468, 1246189591,
    )

    private fun shaRor(x: Int, n: Int): Int {
        return ((x ushr n) or (x shl (32 - n)))
    }
    private fun shaChoice(a: Int, b: Int, c: Int): Int {
        return ((a and b) xor (a.inv() and c))
    }
    private fun shaMajority(a: Int, b: Int, c: Int): Int {
        return (((a and b) xor (a and c)) xor (b and c))
    }
    private fun shaCarry(a: Int, b: Int): Int {
        return if ((a + b).toUInt() < a.toUInt()) 1 else 0
    }
    private fun shaRead(data: IntArray, at: Int): Int {
        return ((((data[at] shl 24) or (data[at + 1] shl 16)) or (data[at + 2] shl 8)) or data[at + 3])
    }

    private fun shaCompress256(state: IntArray, data: IntArray, at: Int, wh: IntArray) {
        for (i in 0 until 16) {
            wh[i] = shaRead(data, at + i * 4)
        }
        for (i in 16 until 64) {
            val x = wh[i - 15]
            val y = wh[i - 2]
            val s0 = ((shaRor(x, 7) xor shaRor(x, 18)) xor (x ushr 3))
            val s1 = ((shaRor(y, 17) xor shaRor(y, 19)) xor (y ushr 10))
            wh[i] = (s1 + wh[i - 7] + s0 + wh[i - 16])
        }
        var a = state[0]
        var b = state[1]
        var c = state[2]
        var d = state[3]
        var e = state[4]
        var f = state[5]
        var g = state[6]
        var h = state[7]
        for (i in 0 until 64) {
            val s1 = ((shaRor(e, 6) xor shaRor(e, 11)) xor shaRor(e, 25))
            val t1 = (h + s1 + shaChoice(e, f, g) + shaK256[i] + wh[i])
            val s0 = ((shaRor(a, 2) xor shaRor(a, 13)) xor shaRor(a, 22))
            val newA = (t1 + s0 + shaMajority(a, b, c))
            val newE = (d + t1)
            h = g
            g = f
            f = e
            e = newE
            d = c
            c = b
            b = a
            a = newA
        }
        state[0] = (state[0] + a)
        state[1] = (state[1] + b)
        state[2] = (state[2] + c)
        state[3] = (state[3] + d)
        state[4] = (state[4] + e)
        state[5] = (state[5] + f)
        state[6] = (state[6] + g)
        state[7] = (state[7] + h)
    }

    private fun shaCompress512(state: IntArray, data: IntArray, at: Int, wh: IntArray, wl: IntArray) {
        for (i in 0 until 16) {
            wh[i] = shaRead(data, at + i * 8)
            wl[i] = shaRead(data, at + i * 8 + 4)
        }
        for (i in 16 until 80) {
            val xh = wh[i - 15]
            val xl = wl[i - 15]
            val yh = wh[i - 2]
            val yl = wl[i - 2]
            val s0h = ((((xh ushr 1) or (xl shl 31)) xor ((xh ushr 8) or (xl shl 24))) xor (xh ushr 7))
            val s0l = ((((xl ushr 1) or (xh shl 31)) xor ((xl ushr 8) or (xh shl 24))) xor ((xl ushr 7) or (xh shl 25)))
            val s1h = ((((yh ushr 19) or (yl shl 13)) xor ((yl ushr 29) or (yh shl 3))) xor (yh ushr 6))
            val s1l = ((((yl ushr 19) or (yh shl 13)) xor ((yh ushr 29) or (yl shl 3))) xor ((yl ushr 6) or (yh shl 26)))
            val nextl2 = (s0l + s1l)
            val nextl3 = (nextl2 + wl[i - 7])
            val nextl = (nextl3 + wl[i - 16])
            val nexth = (s0h + s1h + wh[i - 7] + wh[i - 16] + shaCarry(s0l, s1l) + shaCarry(nextl2, wl[i - 7]) + shaCarry(nextl3, wl[i - 16]))
            wh[i] = nexth
            wl[i] = nextl
        }
        var ah = state[0]
        var al = state[1]
        var bh = state[2]
        var bl = state[3]
        var ch = state[4]
        var cl = state[5]
        var dh = state[6]
        var dl = state[7]
        var eh = state[8]
        var el = state[9]
        var fh = state[10]
        var fl = state[11]
        var gh = state[12]
        var gl = state[13]
        var hh = state[14]
        var hl = state[15]
        for (i in 0 until 80) {
            val s1h = ((((eh ushr 14) or (el shl 18)) xor ((eh ushr 18) or (el shl 14))) xor ((el ushr 9) or (eh shl 23)))
            val s1l = ((((el ushr 14) or (eh shl 18)) xor ((el ushr 18) or (eh shl 14))) xor ((eh ushr 9) or (el shl 23)))
            val chh = shaChoice(eh, fh, gh)
            val chl = shaChoice(el, fl, gl)
            val t1l2 = (hl + s1l)
            val t1l3 = (t1l2 + chl)
            val t1l4 = (t1l3 + shaK512l[i])
            val t1l = (t1l4 + wl[i])
            val t1h = (hh + s1h + chh + shaK512h[i] + wh[i] + shaCarry(hl, s1l) + shaCarry(t1l2, chl) + shaCarry(t1l3, shaK512l[i]) + shaCarry(t1l4, wl[i]))
            val s0h = ((((ah ushr 28) or (al shl 4)) xor ((al ushr 2) or (ah shl 30))) xor ((al ushr 7) or (ah shl 25)))
            val s0l = ((((al ushr 28) or (ah shl 4)) xor ((ah ushr 2) or (al shl 30))) xor ((ah ushr 7) or (al shl 25)))
            val majh = shaMajority(ah, bh, ch)
            val majl = shaMajority(al, bl, cl)
            val newAl2 = (t1l + s0l)
            val newAl = (newAl2 + majl)
            val newAh = (t1h + s0h + majh + shaCarry(t1l, s0l) + shaCarry(newAl2, majl))
            val newEl = (dl + t1l)
            val newEh = (dh + t1h + shaCarry(dl, t1l))
            hh = gh
            hl = gl
            gh = fh
            gl = fl
            fh = eh
            fl = el
            eh = newEh
            el = newEl
            dh = ch
            dl = cl
            ch = bh
            cl = bl
            bh = ah
            bl = al
            ah = newAh
            al = newAl
        }
        state[0] = (state[0] + ah + shaCarry(state[1], al))
        state[1] = (state[1] + al)
        state[2] = (state[2] + bh + shaCarry(state[3], bl))
        state[3] = (state[3] + bl)
        state[4] = (state[4] + ch + shaCarry(state[5], cl))
        state[5] = (state[5] + cl)
        state[6] = (state[6] + dh + shaCarry(state[7], dl))
        state[7] = (state[7] + dl)
        state[8] = (state[8] + eh + shaCarry(state[9], el))
        state[9] = (state[9] + el)
        state[10] = (state[10] + fh + shaCarry(state[11], fl))
        state[11] = (state[11] + fl)
        state[12] = (state[12] + gh + shaCarry(state[13], gl))
        state[13] = (state[13] + gl)
        state[14] = (state[14] + hh + shaCarry(state[15], hl))
        state[15] = (state[15] + hl)
    }

    private fun hash(wide: Boolean, chunks: Array<IntArray>): IntArray {
        val block = if (wide) 128 else 64
        val state = (if (wide) shaIV512 else shaIV256).copyOf()
        val wh = IntArray(80)
        val wl = IntArray(80)
        val tail = IntArray(block)
        fun process(data: IntArray, at: Int) {
            if (wide) shaCompress512(state, data, at, wh, wl) else shaCompress256(state, data, at, wh)
        }
        var pos = 0
        var length = 0.0
        for (chunk in chunks) {
            var at = 0
            length += chunk.size
            while (at < chunk.size) {
                if (pos == 0 && chunk.size - at >= block) {
                    process(chunk, at)
                    at += block
                } else {
                    val take = minOf(block - pos, chunk.size - at)
                    chunk.copyInto(tail, pos, at, at + take)
                    pos += take
                    at += take
                    if (pos == block) { process(tail, 0); pos = 0 }
                }
            }
        }
        tail[pos] = 128
        tail.fill(0, pos + 1, block)
        if (pos + 1 > block - block / 8) { process(tail, 0); tail.fill(0) }
        var bits = length * 8
        for (i in 0 until 8) {
            tail[block-1-i] = (bits % 256).toInt()
            bits = kotlin.math.floor(bits / 256)
        }
        process(tail, 0)
        return IntArray(state.size * 4) { i -> (state[i / 4] ushr (24 - i % 4 * 8)) and 255 }
    }

    fun digest(mode: Int, chunks: Array<IntArray>, key: IntArray): IntArray {
        if (mode != 2) return hash(mode == 1, chunks)
        val k = if (key.size > 64) hash(false, arrayOf(key)) else key
        val padded = k.copyOf(64)
        val inner = IntArray(64) { padded[it] xor 0x36 }
        val outer = IntArray(64) { padded[it] xor 0x5c }
        val data = Array(chunks.size + 1) { if (it == 0) inner else chunks[it - 1] }
        return hash(false, arrayOf(outer, hash(false, data)))
    }
}
