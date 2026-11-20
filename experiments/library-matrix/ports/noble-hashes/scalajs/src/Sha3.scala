// SHA3-256 and Keccak-256, adapted from @noble/hashes 2.4.0 (MIT).
package noblehashes

object Sha3 {
    private val pi = Array(20, 14, 22, 34, 36, 6, 10, 32, 16, 42, 48, 8, 30, 46, 38, 26, 24, 4, 40, 28, 44, 18, 12, 2)
    private val rotations = Array(1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44)
    private val rcLo = Array(1, 32898, 32906, -2147450880, 32907, -2147483647, -2147450751, 32777, 138, 136, -2147450871, -2147483638, -2147450741, 139, 32905, 32771, 32770, 128, 32778, -2147483638, -2147450751, 32896, -2147483647, -2147450872)
    private val rcHi = Array(0, 0, -2147483648, -2147483648, 0, 0, -2147483648, -2147483648, 0, 0, 0, 0, 0, -2147483648, -2147483648, -2147483648, -2147483648, -2147483648, 0, -2147483648, -2147483648, -2147483648, 0, -2147483648)
    // The 25 little-endian lanes occupy 50 words.
    private def permute(s: Array[Int]): Unit = {
        val columns = new Array[Int](10)
        var round = 0
        while (round < 24) {
            // Theta mixes column parities into every lane.
            var x = 0
            while (x < 10) {
                columns(x) = s(x) ^ s(x + 10) ^ s(x + 20) ^ s(x + 30) ^ s(x + 40)
                x += 1
            }
            var col = 0
            while (col < 5) {
                val x = 2 * col
                val left = (x + 8) % 10
                val right = (x + 2) % 10
                val low = columns(left) ^ ((columns(right) << 1) | (columns(right + 1) >>> 31))
                val high = columns(left + 1) ^ ((columns(right + 1) << 1) | (columns(right) >>> 31))
                var row = 0
                while (row < 5) {
                    val i = row * 10 + x
                    s(i) = s(i) ^ low
                    s(i + 1) = s(i + 1) ^ high
                    row += 1
                }
                col += 1
            }
            // Rho rotates and Pi moves the 24 non-origin lanes in a single cycle.
            var low = s(2)
            var high = s(3)
            var t = 0
            while (t < 24) {
                val shift = rotations(t)
                val k = shift & 31
                val a = if (shift > 32) high else low
                val b = if (shift > 32) low else high
                val p = pi(t)
                low = s(p)
                high = s(p + 1)
                s(p) = ((a << k) | (b >>> (32 - k)))
                s(p + 1) = ((b << k) | (a >>> (32 - k)))
                t += 1
            }
            // Chi reads the original five lanes in each row.
            var row = 0
            while (row < 5) {
                val y = 10 * row
                val saved0 = s(y)
                val saved1 = s(y + 1)
                val saved2 = s(y + 2)
                val saved3 = s(y + 3)
                var x = 0
                while (x < 6) {
                    s(y + x) = s(y + x) ^ (~s(y + x + 2) & s(y + x + 4))
                    x += 1
                }
                s(y + 6) = s(y + 6) ^ (~s(y + 8) & saved0)
                s(y + 7) = s(y + 7) ^ (~s(y + 9) & saved1)
                s(y + 8) = s(y + 8) ^ (~saved0 & saved2)
                s(y + 9) = s(y + 9) ^ (~saved1 & saved3)
                row += 1
            }
            s(0) = s(0) ^ rcLo(round)
            s(1) = s(1) ^ rcHi(round)
            round += 1
        }
    }
    def digest(legacy: Boolean, chunks: Array[Array[Int]]): Array[Int] = {
        val s = new Array[Int](50)
        var pos = 0
        var part = 0
        while (part < chunks.length) {
            val chunk = chunks(part)
            var index = 0
            while (index < chunk.length) {
                if (pos == 0 && chunk.length - index >= 136) {
                    var word = 0
                    while (word < 34) {
                        val i = index + 4 * word
                        s(word) = s(word) ^ (chunk(i) | (chunk(i + 1) << 8) | (chunk(i + 2) << 16) | (chunk(i + 3) << 24))
                        word += 1
                    }
                    index += 136
                    permute(s)
                } else {
                    s(pos / 4) = s(pos / 4) ^ (chunk(index) << (8 * (pos % 4)))
                    index += 1
                    pos += 1
                    if (pos == 136) {
                        permute(s)
                        pos = 0
                    }
                }
            }
            part += 1
        }
        val suffix = if (legacy) 1 else 6
        s(pos / 4) = s(pos / 4) ^ (suffix << (8 * (pos % 4)))
        s(33) = s(33) ^ -2147483648
        permute(s)
        val result = new Array[Int](32)
        var i = 0
        while (i < 32) {
            result(i) = (s(i / 4) >>> (8 * (i % 4))) & 255
            i += 1
        }
        result
    }
}
