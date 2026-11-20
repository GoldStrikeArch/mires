// SHA-256 and SHA-512 compression and streaming HMAC, adapted from @noble/hashes 2.4.0.
package noblehashes

object Sha2 {

    private val shaIV256 = Array(
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
        0x1f83d9ab, 0x5be0cd19,
    )

    private val shaIV512 = Array(
        0x6a09e667, 0xf3bcc908, 0xbb67ae85, 0x84caa73b, 0x3c6ef372, 0xfe94f82b,
        0xa54ff53a, 0x5f1d36f1, 0x510e527f, 0xade682d1, 0x9b05688c, 0x2b3e6c1f,
        0x1f83d9ab, 0xfb41bd6b, 0x5be0cd19, 0x137e2179,
    )

    private val shaK256 = Array(
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
        0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
        0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    )

    private val shaK512h = Array(
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
        0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
        0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2, 0xca273ece, 0xd186b8c7,
        0xeada7dd6, 0xf57d4f7f, 0x06f067aa, 0x0a637dc5, 0x113f9804, 0x1b710b35,
        0x28db77f5, 0x32caab7b, 0x3c9ebe0a, 0x431d67c4, 0x4cc5d4be, 0x597f299c,
        0x5fcb6fab, 0x6c44198c,
    )

    private val shaK512l = Array(
        0xd728ae22, 0x23ef65cd, 0xec4d3b2f, 0x8189dbbc, 0xf348b538, 0xb605d019,
        0xaf194f9b, 0xda6d8118, 0xa3030242, 0x45706fbe, 0x4ee4b28c, 0xd5ffb4e2,
        0xf27b896f, 0x3b1696b1, 0x25c71235, 0xcf692694, 0x9ef14ad2, 0x384f25e3,
        0x8b8cd5b5, 0x77ac9c65, 0x592b0275, 0x6ea6e483, 0xbd41fbd4, 0x831153b5,
        0xee66dfab, 0x2db43210, 0x98fb213f, 0xbeef0ee4, 0x3da88fc2, 0x930aa725,
        0xe003826f, 0x0a0e6e70, 0x46d22ffc, 0x5c26c926, 0x5ac42aed, 0x9d95b3df,
        0x8baf63de, 0x3c77b2a8, 0x47edaee6, 0x1482353b, 0x4cf10364, 0xbc423001,
        0xd0f89791, 0x0654be30, 0xd6ef5218, 0x5565a910, 0x5771202a, 0x32bbd1b8,
        0xb8d2d0c8, 0x5141ab53, 0xdf8eeb99, 0xe19b48a8, 0xc5c95a63, 0xe3418acb,
        0x7763e373, 0xd6b2b8a3, 0x5defb2fc, 0x43172f60, 0xa1f0ab72, 0x1a6439ec,
        0x23631e28, 0xde82bde9, 0xb2c67915, 0xe372532b, 0xea26619c, 0x21c0c207,
        0xcde0eb1e, 0xee6ed178, 0x72176fba, 0xa2c898a6, 0xbef90dae, 0x131c471b,
        0x23047d84, 0x40c72493, 0x15c9bebc, 0x9c100d4c, 0xcb3e42b6, 0xfc657e2a,
        0x3ad6faec, 0x4a475817,
    )

    private def shaRor(x: Int, n: Int): Int = {
        return ((x >>> n) | (x << (32 - n)))
    }
    private def shaChoice(a: Int, b: Int, c: Int): Int = {
        return ((a & b) ^ (~a & c))
    }
    private def shaMajority(a: Int, b: Int, c: Int): Int = {
        return (((a & b) ^ (a & c)) ^ (b & c))
    }
    private def shaCarry(a: Int, b: Int): Int = {
        if (java.lang.Integer.compareUnsigned(a + b, a) < 0) 1 else 0
    }
    private def shaRead(data: Array[Int], at: Int): Int = {
        return ((((data(at) << 24) | (data(at + 1) << 16)) | (data(at + 2) << 8)) | data(at + 3))
    }

    private def shaCompress256(state: Array[Int], data: Array[Int], at: Int, wh: Array[Int]): Unit = {
        for (i <- 0 until 16) {
            wh(i) = shaRead(data, at + i * 4)
        }
        for (i <- 16 until 64) {
            val x = wh(i - 15)
            val y = wh(i - 2)
            val s0 = ((shaRor(x, 7) ^ shaRor(x, 18)) ^ (x >>> 3))
            val s1 = ((shaRor(y, 17) ^ shaRor(y, 19)) ^ (y >>> 10))
            wh(i) = (s1 + wh(i - 7) + s0 + wh(i - 16))
        }
        var a = state(0)
        var b = state(1)
        var c = state(2)
        var d = state(3)
        var e = state(4)
        var f = state(5)
        var g = state(6)
        var h = state(7)
        for (i <- 0 until 64) {
            val s1 = ((shaRor(e, 6) ^ shaRor(e, 11)) ^ shaRor(e, 25))
            val t1 = (h + s1 + shaChoice(e, f, g) + shaK256(i) + wh(i))
            val s0 = ((shaRor(a, 2) ^ shaRor(a, 13)) ^ shaRor(a, 22))
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
        state(0) = (state(0) + a)
        state(1) = (state(1) + b)
        state(2) = (state(2) + c)
        state(3) = (state(3) + d)
        state(4) = (state(4) + e)
        state(5) = (state(5) + f)
        state(6) = (state(6) + g)
        state(7) = (state(7) + h)
    }

    private def shaCompress512(state: Array[Int], data: Array[Int], at: Int, wh: Array[Int], wl: Array[Int]): Unit = {
        for (i <- 0 until 16) {
            wh(i) = shaRead(data, at + i * 8)
            wl(i) = shaRead(data, at + i * 8 + 4)
        }
        for (i <- 16 until 80) {
            val xh = wh(i - 15)
            val xl = wl(i - 15)
            val yh = wh(i - 2)
            val yl = wl(i - 2)
            val s0h = ((((xh >>> 1) | (xl << 31)) ^ ((xh >>> 8) | (xl << 24))) ^ (xh >>> 7))
            val s0l = ((((xl >>> 1) | (xh << 31)) ^ ((xl >>> 8) | (xh << 24))) ^ ((xl >>> 7) | (xh << 25)))
            val s1h = ((((yh >>> 19) | (yl << 13)) ^ ((yl >>> 29) | (yh << 3))) ^ (yh >>> 6))
            val s1l = ((((yl >>> 19) | (yh << 13)) ^ ((yh >>> 29) | (yl << 3))) ^ ((yl >>> 6) | (yh << 26)))
            val nextl2 = (s0l + s1l)
            val nextl3 = (nextl2 + wl(i - 7))
            val nextl = (nextl3 + wl(i - 16))
            val nexth = (s0h + s1h + wh(i - 7) + wh(i - 16) + shaCarry(s0l, s1l) + shaCarry(nextl2, wl(i - 7)) + shaCarry(nextl3, wl(i - 16)))
            wh(i) = nexth
            wl(i) = nextl
        }
        var ah = state(0)
        var al = state(1)
        var bh = state(2)
        var bl = state(3)
        var ch = state(4)
        var cl = state(5)
        var dh = state(6)
        var dl = state(7)
        var eh = state(8)
        var el = state(9)
        var fh = state(10)
        var fl = state(11)
        var gh = state(12)
        var gl = state(13)
        var hh = state(14)
        var hl = state(15)
        for (i <- 0 until 80) {
            val s1h = ((((eh >>> 14) | (el << 18)) ^ ((eh >>> 18) | (el << 14))) ^ ((el >>> 9) | (eh << 23)))
            val s1l = ((((el >>> 14) | (eh << 18)) ^ ((el >>> 18) | (eh << 14))) ^ ((eh >>> 9) | (el << 23)))
            val chh = shaChoice(eh, fh, gh)
            val chl = shaChoice(el, fl, gl)
            val t1l2 = (hl + s1l)
            val t1l3 = (t1l2 + chl)
            val t1l4 = (t1l3 + shaK512l(i))
            val t1l = (t1l4 + wl(i))
            val t1h = (hh + s1h + chh + shaK512h(i) + wh(i) + shaCarry(hl, s1l) + shaCarry(t1l2, chl) + shaCarry(t1l3, shaK512l(i)) + shaCarry(t1l4, wl(i)))
            val s0h = ((((ah >>> 28) | (al << 4)) ^ ((al >>> 2) | (ah << 30))) ^ ((al >>> 7) | (ah << 25)))
            val s0l = ((((al >>> 28) | (ah << 4)) ^ ((ah >>> 2) | (al << 30))) ^ ((ah >>> 7) | (al << 25)))
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
        state(0) = (state(0) + ah + shaCarry(state(1), al))
        state(1) = (state(1) + al)
        state(2) = (state(2) + bh + shaCarry(state(3), bl))
        state(3) = (state(3) + bl)
        state(4) = (state(4) + ch + shaCarry(state(5), cl))
        state(5) = (state(5) + cl)
        state(6) = (state(6) + dh + shaCarry(state(7), dl))
        state(7) = (state(7) + dl)
        state(8) = (state(8) + eh + shaCarry(state(9), el))
        state(9) = (state(9) + el)
        state(10) = (state(10) + fh + shaCarry(state(11), fl))
        state(11) = (state(11) + fl)
        state(12) = (state(12) + gh + shaCarry(state(13), gl))
        state(13) = (state(13) + gl)
        state(14) = (state(14) + hh + shaCarry(state(15), hl))
        state(15) = (state(15) + hl)
    }

    private def hash(wide: Boolean, chunks: Array[Array[Int]]): Array[Int] = {
        val block = if (wide) 128 else 64
        val state = (if (wide) shaIV512 else shaIV256).clone()
        val wh = new Array[Int](80)
        val wl = new Array[Int](80)
        val tail = new Array[Int](block)
        def process(data: Array[Int], at: Int): Unit =
            if (wide) shaCompress512(state, data, at, wh, wl) else shaCompress256(state, data, at, wh)
        var pos = 0
        var length = 0.0
        for (chunk <- chunks) {
            var at = 0
            length += chunk.length
            while (at < chunk.length) {
                if (pos == 0 && chunk.length - at >= block) {
                    process(chunk, at)
                    at += block
                } else {
                    val take = math.min(block - pos, chunk.length - at)
                    Array.copy(chunk, at, tail, pos, take)
                    pos += take
                    at += take
                    if (pos == block) { process(tail, 0); pos = 0 }
                }
            }
        }
        tail(pos) = 128
        java.util.Arrays.fill(tail, pos+1, block, 0)
        if (pos+1 > block-block/8) { process(tail, 0); java.util.Arrays.fill(tail, 0) }
        var bits = length * 8
        for (i <- 0 until 8) {
            tail(block-1-i) = (bits % 256).toInt
            bits = math.floor(bits/256)
        }
        process(tail,0)
        Array.tabulate(state.length*4)(i => (state(i/4) >>> (24 - i%4*8)) & 255)
    }

    def digest(mode: Int, chunks: Array[Array[Int]], key: Array[Int]): Array[Int] = {
        if (mode != 2) return hash(mode == 1, chunks)
        val k = if (key.length > 64) hash(false, Array(key)) else key
        val inner = Array.tabulate(64)(i => (if (i < k.length) k(i) else 0) ^ 0x36)
        val outer = Array.tabulate(64)(i => (if (i < k.length) k(i) else 0) ^ 0x5c)
        hash(false, Array(outer, hash(false, inner +: chunks)))
    }
}
