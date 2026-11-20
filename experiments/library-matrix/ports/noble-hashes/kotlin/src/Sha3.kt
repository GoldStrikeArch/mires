@file:OptIn(ExperimentalUnsignedTypes::class)

// noble's sha3.ts: Keccak-f[1600] and the Keccak sponge behind SHA3-256 and Keccak-256.
//
// The state is noble's state32: 25 lanes of two 32-bit words, the low word first. noble derives
// the pi, rho and iota tables at load time; they are written out here.

/** Pi: the word index each rho step moves its lane to (noble's SHA3_PI). */
private val SHA3_PI = intArrayOf(20, 14, 22, 34, 36, 6, 10, 32, 16, 42, 48, 8, 30, 46, 38, 26, 24, 4, 40, 28, 44, 18, 12, 2)

/** Rho: the left rotation of each step (noble's SHA3_ROTL). */
private val SHA3_ROTL = intArrayOf(1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44)

/** Iota: the low and high words of each round constant (noble's SHA3_IOTA_H and SHA3_IOTA_L). */
private val SHA3_IOTA_H = words(
    0x00000001u, 0x00008082u, 0x0000808au, 0x80008000u, 0x0000808bu, 0x80000001u, 0x80008081u, 0x00008009u,
    0x0000008au, 0x00000088u, 0x80008009u, 0x8000000au, 0x8000808bu, 0x0000008bu, 0x00008089u, 0x00008003u,
    0x00008002u, 0x00000080u, 0x0000800au, 0x8000000au, 0x80008081u, 0x00008080u, 0x80000001u, 0x80008008u,
)
private val SHA3_IOTA_L = words(
    0x00000000u, 0x00000000u, 0x80000000u, 0x80000000u, 0x00000000u, 0x00000000u, 0x80000000u, 0x80000000u,
    0x00000000u, 0x00000000u, 0x00000000u, 0x00000000u, 0x00000000u, 0x80000000u, 0x80000000u, 0x80000000u,
    0x80000000u, 0x80000000u, 0x00000000u, 0x80000000u, 0x80000000u, 0x80000000u, 0x00000000u, 0x80000000u,
)

// Left rotations of a lane by s in 1..31 (S) or 33..63 (B). As in noble, h is the lane's first
// (low) word and l its second (high) word.
private fun rotlSH(h: Int, l: Int, s: Int) = (h shl s) or (l ushr 32 - s)
private fun rotlSL(h: Int, l: Int, s: Int) = (l shl s) or (h ushr 32 - s)
private fun rotlBH(h: Int, l: Int, s: Int) = (l shl s - 32) or (h ushr 64 - s)
private fun rotlBL(h: Int, l: Int, s: Int) = (h shl s - 32) or (l ushr 64 - s)
private fun rotlH(h: Int, l: Int, s: Int) = if (s > 32) rotlBH(h, l, s) else rotlSH(h, l, s)
private fun rotlL(h: Int, l: Int, s: Int) = if (s > 32) rotlBL(h, l, s) else rotlSL(h, l, s)

/** noble's keccakP: the 24 rounds of Keccak-f[1600] on [s], with [b] as the theta scratch. */
private fun keccakP(s: IntArray, b: IntArray) {
    for (round in 0 until 24) {
        // Theta
        for (x in 0 until 10) b[x] = s[x] xor s[x + 10] xor s[x + 20] xor s[x + 30] xor s[x + 40]
        for (column in 0 until 5) {
            val x = 2 * column
            val idx1 = (x + 8) % 10
            val idx0 = (x + 2) % 10
            val b0 = b[idx0]
            val b1 = b[idx0 + 1]
            val th = rotlH(b0, b1, 1) xor b[idx1]
            val tl = rotlL(b0, b1, 1) xor b[idx1 + 1]
            for (row in 0 until 5) {
                val y = 10 * row
                s[x + y] = s[x + y] xor th
                s[x + y + 1] = s[x + y + 1] xor tl
            }
        }
        // Rho and pi
        var curH = s[2]
        var curL = s[3]
        for (t in 0 until 24) {
            val shift = SHA3_ROTL[t]
            val th = rotlH(curH, curL, shift)
            val tl = rotlL(curH, curL, shift)
            val pi = SHA3_PI[t]
            curH = s[pi]
            curL = s[pi + 1]
            s[pi] = th
            s[pi + 1] = tl
        }
        // Chi
        for (row in 0 until 5) {
            val y = 10 * row
            val b0 = s[y]
            val b1 = s[y + 1]
            val b2 = s[y + 2]
            val b3 = s[y + 3]
            for (x in 0 until 6) s[y + x] = s[y + x] xor (s[y + x + 2].inv() and s[y + x + 4])
            s[y + 6] = s[y + 6] xor (s[y + 8].inv() and b0)
            s[y + 7] = s[y + 7] xor (s[y + 9].inv() and b1)
            s[y + 8] = s[y + 8] xor (b0.inv() and b2)
            s[y + 9] = s[y + 9] xor (b1.inv() and b3)
        }
        // Iota
        s[0] = s[0] xor SHA3_IOTA_H[round]
        s[1] = s[1] xor SHA3_IOTA_L[round]
    }
}

/** noble's Keccak sponge with a byte-length rate [blockLen] and domain [suffix]. */
class Keccak(
    override val blockLen: Int,
    private val suffix: Int,
    override val outputLen: Int,
) : Hash {
    private val state32 = IntArray(50)
    private val theta = IntArray(10)
    private var pos = 0
    private var posOut = 0

    /** XORs [byte] into state byte [index], as noble does through its byte view of state32. */
    private fun xorByte(index: Int, byte: Int) {
        state32[index shr 2] = state32[index shr 2] xor (byte shl 8 * (index and 3))
    }

    /** The state byte at [index]. */
    private fun stateByte(index: Int): Byte = (state32[index shr 2] ushr 8 * (index and 3)).toByte()

    private fun keccak() {
        keccakP(state32, theta)
        posOut = 0
        pos = 0
    }

    override fun update(data: ByteArray): Keccak {
        val len = data.size
        var pos = 0
        while (pos < len) {
            // Whole blocks with nothing buffered are absorbed a word at a time.
            if (this.pos == 0 && len - pos >= blockLen) {
                for (i in 0 until blockLen / 4) state32[i] = state32[i] xor data.getUint32LE(pos + 4 * i)
                pos += blockLen
                this.pos = blockLen
                keccak()
                continue
            }
            val take = minOf(blockLen - this.pos, len - pos)
            repeat(take) { xorByte(this.pos++, data.u8(pos++)) }
            if (this.pos == blockLen) keccak()
        }
        return this
    }

    private fun finish() {
        // The suffix holds the domain bits and the first padding bit; 0x80 closes pad10*1.
        xorByte(pos, suffix)
        if (suffix and 0x80 != 0 && pos == blockLen - 1) keccak()
        xorByte(blockLen - 1, 0x80)
        keccak()
    }

    override fun digest(): ByteArray {
        finish()
        val out = ByteArray(outputLen)
        var pos = 0
        while (pos < outputLen) {
            if (posOut >= blockLen) keccak()
            val take = minOf(blockLen - posOut, outputLen - pos)
            repeat(take) { out[pos++] = stateByte(posOut++) }
        }
        return out
    }
}
