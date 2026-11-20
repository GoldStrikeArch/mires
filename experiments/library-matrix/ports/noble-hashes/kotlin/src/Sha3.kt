// SHA3-256 and Keccak-256, adapted from @noble/hashes 2.4.0 (MIT).
object Sha3 {
    private val pi = intArrayOf(20, 14, 22, 34, 36, 6, 10, 32, 16, 42, 48, 8, 30, 46, 38, 26, 24, 4, 40, 28, 44, 18, 12, 2)
    private val rotations = intArrayOf(1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44)
    private val rcLo = intArrayOf(1, 32898, 32906, -2147450880, 32907, -2147483647, -2147450751, 32777, 138, 136, -2147450871, -2147483638, -2147450741, 139, 32905, 32771, 32770, 128, 32778, -2147483638, -2147450751, 32896, -2147483647, -2147450872)
    private val rcHi = intArrayOf(0, 0, -2147483648, -2147483648, 0, 0, -2147483648, -2147483648, 0, 0, 0, 0, 0, -2147483648, -2147483648, -2147483648, -2147483648, -2147483648, 0, -2147483648, -2147483648, -2147483648, 0, -2147483648)
    // The 25 little-endian lanes occupy 50 words.
    private fun permute(s: IntArray) {
        val columns = IntArray(10)
        for (round in 0 until 24) {
            // Theta mixes column parities into every lane.
            for (x in 0 until 10) {
                columns[x] = s[x] xor s[x + 10] xor s[x + 20] xor s[x + 30] xor s[x + 40]
            }
            for (col in 0 until 5) {
                val x = 2 * col
                val left = (x + 8) % 10
                val right = (x + 2) % 10
                val low = columns[left] xor ((columns[right] shl 1) or (columns[right + 1] ushr 31))
                val high = columns[left + 1] xor ((columns[right + 1] shl 1) or (columns[right] ushr 31))
                for (row in 0 until 5) {
                    val i = row * 10 + x
                    s[i] = s[i] xor low
                    s[i + 1] = s[i + 1] xor high
                }
            }
            // Rho rotates and Pi moves the 24 non-origin lanes in a single cycle.
            var low = s[2]
            var high = s[3]
            for (t in 0 until 24) {
                val shift = rotations[t]
                val k = shift and 31
                val a = if (shift > 32) high else low
                val b = if (shift > 32) low else high
                val p = pi[t]
                low = s[p]
                high = s[p + 1]
                s[p] = ((a shl k) or (b ushr (32 - k)))
                s[p + 1] = ((b shl k) or (a ushr (32 - k)))
            }
            // Chi reads the original five lanes in each row.
            for (row in 0 until 5) {
                val y = 10 * row
                val saved0 = s[y]
                val saved1 = s[y + 1]
                val saved2 = s[y + 2]
                val saved3 = s[y + 3]
                for (x in 0 until 6) {
                    s[y + x] = s[y + x] xor (s[y + x + 2].inv() and s[y + x + 4])
                }
                s[y + 6] = s[y + 6] xor (s[y + 8].inv() and saved0)
                s[y + 7] = s[y + 7] xor (s[y + 9].inv() and saved1)
                s[y + 8] = s[y + 8] xor (saved0.inv() and saved2)
                s[y + 9] = s[y + 9] xor (saved1.inv() and saved3)
            }
            s[0] = s[0] xor rcLo[round]
            s[1] = s[1] xor rcHi[round]
        }
    }
    fun digest(legacy: Boolean, chunks: Array<IntArray>): IntArray {
        val s = IntArray(50)
        var pos = 0
        for (chunk in chunks) {
            var index = 0
            while (index < chunk.size) {
                if (pos == 0 && chunk.size - index >= 136) {
                    for (word in 0 until 34) {
                        val i = index + 4 * word
                        s[word] = s[word] xor (chunk[i] or (chunk[i + 1] shl 8) or (chunk[i + 2] shl 16) or (chunk[i + 3] shl 24))
                    }
                    index += 136
                    permute(s)
                } else {
                    s[pos / 4] = s[pos / 4] xor (chunk[index] shl (8 * (pos % 4)))
                    index++
                    pos++
                    if (pos == 136) {
                        permute(s)
                        pos = 0
                    }
                }
            }
        }
        val suffix = if (legacy) 1 else 6
        s[pos / 4] = s[pos / 4] xor (suffix shl (8 * (pos % 4)))
        s[33] = s[33] xor -2147483648
        permute(s)
        val result = IntArray(32)
        for (i in 0 until 32) {
            result[i] = (s[i / 4] ushr (8 * (i % 4))) and 255
        }
        return result
    }
}
