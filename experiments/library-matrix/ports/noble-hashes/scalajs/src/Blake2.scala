package noblehashes

// BLAKE2s and BLAKE2b, with 64-bit words represented by low/high 32-bit limbs.
object Blake2 {
    private val ivLow = Array(-205731576, -2067093701, -23791573, 1595750129, -1377402159, 725511199, -79577749, 327033209)
    private val ivHigh = Array(1779033703, -1150833019, 1013904242, -1521486534, 1359893119, -1694144372, 528734635, 1541459225)
    private val sigma = Array(
        Array(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15),
        Array(14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3),
        Array(11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4),
        Array(7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8),
        Array(9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13),
        Array(2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9),
        Array(12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11),
        Array(13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10),
        Array(6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5),
        Array(10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0)
    )

    private final case class Word(lo: Int, hi: Int)


    private def add(a: Word, b: Word): Word = {
        val low = a.lo + b.lo
        val carry = if ((low ^ Int.MinValue) < (a.lo ^ Int.MinValue)) 1 else 0
        return Word(low, a.hi + b.hi + carry)
    }

    private def xorWord(a: Word, b: Word): Word = {
        return Word(a.lo ^ b.lo, a.hi ^ b.hi)
    }

    private def rotate(w: Word, n: Int, wide: Boolean): Word = {
        if (!wide) { return Word((w.lo >>> n) | (w.lo << (32 - n)), 0) }
        if (n == 32) { return Word(w.hi, w.lo) }
        val low = if (n > 32) w.hi else w.lo
        val high = if (n > 32) w.lo else w.hi
        val bits = n & 31
        return Word((low >>> bits) | (high << (32 - bits)), (high >>> bits) | (low << (32 - bits)))
    }

    private def initial(i: Int, wide: Boolean): Word = {
        return if (wide) Word(ivLow(i), ivHigh(i)) else Word(ivHigh(i), 0)
    }

    private def load32(bytes: Array[Int], pos: Int): Int = {
        return bytes(pos) | (bytes(pos + 1) << 8) | (bytes(pos + 2) << 16) | (bytes(pos + 3) << 24)
    }

    private def load(bytes: Array[Int], i: Int, wide: Boolean): Word = {
        return if (wide) Word(load32(bytes, 8 * i), load32(bytes, 8 * i + 4)) else Word(load32(bytes, 4 * i), 0)
    }

    private def mix(v: Array[Word], a: Int, b: Int, c: Int, d: Int, x: Word, y: Word, wide: Boolean): Unit = {
        v(a) = add(add(v(a), v(b)), x)
        v(d) = rotate(xorWord(v(d), v(a)), if (wide) 32 else 16, wide)
        v(c) = add(v(c), v(d))
        v(b) = rotate(xorWord(v(b), v(c)), if (wide) 24 else 12, wide)
        v(a) = add(add(v(a), v(b)), y)
        v(d) = rotate(xorWord(v(d), v(a)), if (wide) 16 else 8, wide)
        v(c) = add(v(c), v(d))
        v(b) = rotate(xorWord(v(b), v(c)), if (wide) 63 else 7, wide)
    }

    private def compress(h: Array[Word], block: Array[Int], count: Word, last: Boolean, wide: Boolean): Unit = {
        val v = Array.tabulate(16)(i => if (i < 8) h(i) else initial(i - 8, wide))
        val msg = Array.tabulate(16)(i => load(block, i, wide))
        v(12) = Word(v(12).lo ^ count.lo, v(12).hi)
        if (wide) {
            v(12) = Word(v(12).lo, v(12).hi ^ count.hi)
        } else {
            v(13) = Word(v(13).lo ^ count.hi, 0)
        }
        if (last) { v(14) = Word(~v(14).lo, ~v(14).hi) }
        val rounds = if (wide) 12 else 10
        for (r <- 0 until rounds) {
            val schedule = sigma(r % 10)
            for (i <- 0 until 4) {
                mix(v, i, i + 4, i + 8, i + 12, msg(schedule(2 * i)), msg(schedule(2 * i + 1)), wide)
            }
            for (i <- 0 until 4) {
                mix(v, i, 4 + (i + 1) % 4, 8 + (i + 2) % 4, 12 + (i + 3) % 4,
                    msg(schedule(8 + 2 * i)), msg(schedule(9 + 2 * i)), wide)
            }
        }
        for (i <- 0 until 8) { h(i) = xorWord(h(i), xorWord(v(i), v(i+8))) }
    }

    def digest(wide: Boolean, chunks: Array[Array[Int]], dkLen: Int, key: Array[Int], salt: Array[Int], personalization: Array[Int]): Array[Int] = {
        val stride = if (wide) 8 else 4
        val blockLen = stride * 16
        val h = Array.tabulate(8)(i => initial(i, wide))
        h(0) = Word(h(0).lo ^ 0x01010000 ^ (key.length << 8) ^ dkLen, h(0).hi)
        for (i <- 0 until 2) {
            if (salt.length > 0) { h(4+i) = xorWord(h(4+i), load(salt, i, wide)) }
            if (personalization.length > 0) { h(6+i) = xorWord(h(6+i), load(personalization, i, wide)) }
        }
        val block = new Array[Int](blockLen)
        for (i <- 0 until key.length) { block(i) = key(i) }
        var pos = if (key.length > 0) blockLen else 0
        var count = Word(pos, 0)
        for (chunk <- chunks) {
            var at = 0
            while (at < chunk.length) {
                // A full block remains pending until the next nonempty update.
                if (pos == blockLen) { compress(h, block, count, false, wide); pos = 0 }
                val take = math.min(blockLen - pos, chunk.length - at)
                Array.copy(chunk, at, block, pos, take)
                pos += take
                at += take
                count = add(count, Word(take, 0))
            }
        }
        while (pos < blockLen) { block(pos) = 0; pos += 1 }
        compress(h, block, count, true, wide)
        return Array.tabulate(dkLen) { i =>
            val word = h(i / stride)
            val limb = if (i % stride >= 4) word.hi else word.lo
            (limb >>> ((i % 4) * 8)) & 255
        }
    }
}
