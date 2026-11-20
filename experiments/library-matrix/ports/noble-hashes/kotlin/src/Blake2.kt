@file:OptIn(ExperimentalUnsignedTypes::class)

// noble's blake2.ts and _blake.ts: BLAKE2b and BLAKE2s on noble's shared _BLAKE2 base.
//
// BLAKE2b keeps noble's G1b/G2b on 64-bit words as low and high halves in a 32-word work vector
// (noble's BBUF); BLAKE2s keeps G1s/G2s on 32-bit words. The two do not share a mix function.

/** Message word order of each round (noble's BSIGMA); BLAKE2b's rounds 10 and 11 repeat 0 and 1. */
private val BSIGMA = intArrayOf(
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3,
    11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4,
    7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8,
    9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13,
    2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9,
    12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11,
    13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10,
    6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5,
    10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3,
)

/** noble's Blake2Opts: the options reference.mjs passes to every hash. */
class Blake2Opts(
    val dkLen: Double?,
    val key: ByteArray?,
    val salt: ByteArray?,
    val personalization: ByteArray?,
)

/** noble's checkBlake2Opts: [keyLen] bounds both the digest and the key length. */
private fun checkBlake2Opts(outputLen: Int, opts: Blake2Opts, keyLen: Int, saltLen: Int, persLen: Int) {
    require(outputLen in 1..keyLen) { "\"dkLen\" must be 1..$keyLen, got $outputLen" }
    val key = opts.key
    require(key == null || key.size in 1..keyLen) { "\"key\" expected to be undefined or of length=1..$keyLen" }
    require(opts.salt == null || opts.salt.size == saltLen) { "\"salt\" expected Uint8Array of length $saltLen" }
    require(opts.personalization == null || opts.personalization.size == persLen) {
        "\"personalization\" expected Uint8Array of length $persLen"
    }
}

/** noble's _BLAKE2: the final block is compressed with a flag, so a full buffer waits for more input. */
abstract class Blake2(final override val blockLen: Int, dkLen: Double) : Hash {
    final override val outputLen = anumber(dkLen, "outputLen")
    private val buffer = ByteArray(blockLen)

    /** Bytes compressed or buffered so far: the BLAKE2 offset counter. */
    protected var length = 0.0
    private var pos = 0

    /** Compresses the block of [data] at [offset]. */
    protected abstract fun compress(data: ByteArray, offset: Int, isLast: Boolean)

    /** The state as little-endian 32-bit words. */
    protected abstract fun get(): IntArray

    override fun update(data: ByteArray): Blake2 {
        val len = data.size
        var pos = 0
        while (pos < len) {
            if (this.pos == blockLen) {
                compress(buffer, 0, false)
                this.pos = 0
            }
            val take = minOf(blockLen - this.pos, len - pos)
            // Whole blocks that are not the last of the input are compressed straight from it.
            if (take == blockLen && pos + take < len) {
                while (pos + blockLen < len) {
                    length += blockLen
                    compress(data, pos, false)
                    pos += blockLen
                }
                continue
            }
            data.copyInto(buffer, this.pos, pos, pos + take)
            this.pos += take
            length += take
            pos += take
        }
        return this
    }

    override fun digest(): ByteArray {
        buffer.fill(0, pos)
        compress(buffer, 0, true)
        val out = ByteArray(outputLen)
        wordsToBytesLE(get(), out, outputLen)
        return out
    }

    /** noble's constructor tail: the zero-padded key is the first block. */
    protected fun absorbKey(key: ByteArray?) {
        if (key == null) return
        val block = ByteArray(blockLen)
        key.copyInto(block)
        update(block)
    }
}

/** The SHA-512 IV as little-endian low and high halves (noble's B2B_IV). */
private val B2B_IV = words(
    0xf3bcc908u, 0x6a09e667u, 0x84caa73bu, 0xbb67ae85u, 0xfe94f82bu, 0x3c6ef372u, 0x5f1d36f1u, 0xa54ff53au,
    0xade682d1u, 0x510e527fu, 0x2b3e6c1fu, 0x9b05688cu, 0xfb41bd6bu, 0x1f83d9abu, 0x137e2179u, 0x5be0cd19u,
)

/** noble's _BLAKE2b. */
class Blake2b(opts: Blake2Opts) : Blake2(128, opts.dkLen ?: 64.0) {
    /** v0l, v0h, ..., v7l, v7h. */
    private val v = B2B_IV.copyOf()

    /** The work vector of 16 words as low and high halves (noble's BBUF). */
    private val bbuf = IntArray(32)

    /** The block being compressed as little-endian 32-bit words. */
    private val msg = IntArray(32)

    init {
        checkBlake2Opts(outputLen, opts, 64, 16, 16)
        val keyLength = opts.key?.size ?: 0
        // RFC 7693 §2.5: p[0] = 0x0101kknn goes into the low half of h[0].
        v[0] = v[0] xor (outputLen or (keyLength shl 8) or (0x01 shl 16) or (0x01 shl 24))
        opts.salt?.let { salt -> for (i in 0 until 4) v[8 + i] = v[8 + i] xor salt.getUint32LE(4 * i) }
        opts.personalization?.let { pers -> for (i in 0 until 4) v[12 + i] = v[12 + i] xor pers.getUint32LE(4 * i) }
        absorbKey(opts.key)
    }

    override fun get(): IntArray = v

    // G's first half: a = a + b + x, d = rotr(d ^ a, 32), c = c + d, b = rotr(b ^ c, 24).
    private fun g1b(a: Int, b: Int, c: Int, d: Int, x: Int) {
        val xl = msg[x]
        val xh = msg[x + 1]
        var al = bbuf[2 * a]
        var ah = bbuf[2 * a + 1]
        var bl = bbuf[2 * b]
        var bh = bbuf[2 * b + 1]
        var cl = bbuf[2 * c]
        var ch = bbuf[2 * c + 1]
        var dl = bbuf[2 * d]
        var dh = bbuf[2 * d + 1]
        add3(ah, al, bh, bl, xh, xl).let { (h, l) -> ah = h; al = l }
        var th = dh xor ah
        var tl = dl xor al
        dh = rotr32H(th, tl)
        dl = rotr32L(th, tl)
        add(ch, cl, dh, dl).let { (h, l) -> ch = h; cl = l }
        th = bh xor ch
        tl = bl xor cl
        bh = rotrSH(th, tl, 24)
        bl = rotrSL(th, tl, 24)
        bbuf[2 * a] = al
        bbuf[2 * a + 1] = ah
        bbuf[2 * b] = bl
        bbuf[2 * b + 1] = bh
        bbuf[2 * c] = cl
        bbuf[2 * c + 1] = ch
        bbuf[2 * d] = dl
        bbuf[2 * d + 1] = dh
    }

    // G's second half: a = a + b + y, d = rotr(d ^ a, 16), c = c + d, b = rotr(b ^ c, 63).
    private fun g2b(a: Int, b: Int, c: Int, d: Int, x: Int) {
        val xl = msg[x]
        val xh = msg[x + 1]
        var al = bbuf[2 * a]
        var ah = bbuf[2 * a + 1]
        var bl = bbuf[2 * b]
        var bh = bbuf[2 * b + 1]
        var cl = bbuf[2 * c]
        var ch = bbuf[2 * c + 1]
        var dl = bbuf[2 * d]
        var dh = bbuf[2 * d + 1]
        add3(ah, al, bh, bl, xh, xl).let { (h, l) -> ah = h; al = l }
        var th = dh xor ah
        var tl = dl xor al
        dh = rotrSH(th, tl, 16)
        dl = rotrSL(th, tl, 16)
        add(ch, cl, dh, dl).let { (h, l) -> ch = h; cl = l }
        th = bh xor ch
        tl = bl xor cl
        bh = rotrBH(th, tl, 63)
        bl = rotrBL(th, tl, 63)
        bbuf[2 * a] = al
        bbuf[2 * a + 1] = ah
        bbuf[2 * b] = bl
        bbuf[2 * b + 1] = bh
        bbuf[2 * c] = cl
        bbuf[2 * c + 1] = ch
        bbuf[2 * d] = dl
        bbuf[2 * d + 1] = dh
    }

    override fun compress(data: ByteArray, offset: Int, isLast: Boolean) {
        for (i in 0 until 32) msg[i] = data.getUint32LE(offset + 4 * i)
        v.copyInto(bbuf)
        B2B_IV.copyInto(bbuf, 16)
        // The byte counter goes into v12; v14 is inverted on the last block.
        bbuf[24] = B2B_IV[8] xor fromNumL(length)
        bbuf[25] = B2B_IV[9] xor fromNumH(length)
        if (isLast) {
            bbuf[28] = bbuf[28].inv()
            bbuf[29] = bbuf[29].inv()
        }
        // BSIGMA picks 64-bit message words; each spans two entries of msg.
        var j = 0
        repeat(12) {
            g1b(0, 4, 8, 12, 2 * BSIGMA[j++])
            g2b(0, 4, 8, 12, 2 * BSIGMA[j++])
            g1b(1, 5, 9, 13, 2 * BSIGMA[j++])
            g2b(1, 5, 9, 13, 2 * BSIGMA[j++])
            g1b(2, 6, 10, 14, 2 * BSIGMA[j++])
            g2b(2, 6, 10, 14, 2 * BSIGMA[j++])
            g1b(3, 7, 11, 15, 2 * BSIGMA[j++])
            g2b(3, 7, 11, 15, 2 * BSIGMA[j++])
            g1b(0, 5, 10, 15, 2 * BSIGMA[j++])
            g2b(0, 5, 10, 15, 2 * BSIGMA[j++])
            g1b(1, 6, 11, 12, 2 * BSIGMA[j++])
            g2b(1, 6, 11, 12, 2 * BSIGMA[j++])
            g1b(2, 7, 8, 13, 2 * BSIGMA[j++])
            g2b(2, 7, 8, 13, 2 * BSIGMA[j++])
            g1b(3, 4, 9, 14, 2 * BSIGMA[j++])
            g2b(3, 4, 9, 14, 2 * BSIGMA[j++])
        }
        for (i in 0 until 16) v[i] = v[i] xor bbuf[i] xor bbuf[i + 16]
    }
}

/** noble's _BLAKE2s, whose IV is SHA-256's. */
class Blake2s(opts: Blake2Opts) : Blake2(64, opts.dkLen ?: 32.0) {
    /** v0 ... v7. */
    private val v = SHA256_IV.copyOf()

    /** v0 ... v15 of one compression (noble's _compress locals). */
    private val work = IntArray(16)

    /** The block being compressed as little-endian 32-bit words. */
    private val msg = IntArray(16)

    init {
        checkBlake2Opts(outputLen, opts, 32, 8, 8)
        val keyLength = opts.key?.size ?: 0
        v[0] = v[0] xor (outputLen or (keyLength shl 8) or (0x01 shl 16) or (0x01 shl 24))
        opts.salt?.let { salt -> for (i in 0 until 2) v[4 + i] = v[4 + i] xor salt.getUint32LE(4 * i) }
        opts.personalization?.let { pers -> for (i in 0 until 2) v[6 + i] = v[6 + i] xor pers.getUint32LE(4 * i) }
        absorbKey(opts.key)
    }

    override fun get(): IntArray = v

    // G's first half: a = a + b + x, d = rotr(d ^ a, 16), c = c + d, b = rotr(b ^ c, 12).
    private fun g1s(a: Int, b: Int, c: Int, d: Int, x: Int) {
        work[a] = work[a] + work[b] + x
        work[d] = (work[d] xor work[a]).rotateRight(16)
        work[c] = work[c] + work[d]
        work[b] = (work[b] xor work[c]).rotateRight(12)
    }

    // G's second half: a = a + b + y, d = rotr(d ^ a, 8), c = c + d, b = rotr(b ^ c, 7).
    private fun g2s(a: Int, b: Int, c: Int, d: Int, x: Int) {
        work[a] = work[a] + work[b] + x
        work[d] = (work[d] xor work[a]).rotateRight(8)
        work[c] = work[c] + work[d]
        work[b] = (work[b] xor work[c]).rotateRight(7)
    }

    override fun compress(data: ByteArray, offset: Int, isLast: Boolean) {
        for (i in 0 until 16) msg[i] = data.getUint32LE(offset + 4 * i)
        // v8..v15 come from the IV, with the byte counter in v12 and v13 and v14 inverted last.
        v.copyInto(work)
        SHA256_IV.copyInto(work, 8)
        work[12] = work[12] xor fromNumL(length)
        work[13] = work[13] xor fromNumH(length)
        if (isLast) work[14] = work[14].inv()
        var j = 0
        repeat(10) {
            g1s(0, 4, 8, 12, msg[BSIGMA[j++]])
            g2s(0, 4, 8, 12, msg[BSIGMA[j++]])
            g1s(1, 5, 9, 13, msg[BSIGMA[j++]])
            g2s(1, 5, 9, 13, msg[BSIGMA[j++]])
            g1s(2, 6, 10, 14, msg[BSIGMA[j++]])
            g2s(2, 6, 10, 14, msg[BSIGMA[j++]])
            g1s(3, 7, 11, 15, msg[BSIGMA[j++]])
            g2s(3, 7, 11, 15, msg[BSIGMA[j++]])
            g1s(0, 5, 10, 15, msg[BSIGMA[j++]])
            g2s(0, 5, 10, 15, msg[BSIGMA[j++]])
            g1s(1, 6, 11, 12, msg[BSIGMA[j++]])
            g2s(1, 6, 11, 12, msg[BSIGMA[j++]])
            g1s(2, 7, 8, 13, msg[BSIGMA[j++]])
            g2s(2, 7, 8, 13, msg[BSIGMA[j++]])
            g1s(3, 4, 9, 14, msg[BSIGMA[j++]])
            g2s(3, 4, 9, 14, msg[BSIGMA[j++]])
        }
        for (i in 0 until 8) v[i] = v[i] xor work[i] xor work[i + 8]
    }
}
