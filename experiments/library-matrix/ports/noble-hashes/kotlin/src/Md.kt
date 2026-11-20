// noble's _md.ts: the Merkle–Damgård base class of SHA-256 and SHA-512, and its Chi and Maj.

/** Bits of [b] where [a] is set, else of [c]. */
internal fun chi(a: Int, b: Int, c: Int): Int = (a and b) xor (a.inv() and c)

/** The majority of each bit of [a], [b] and [c]. */
internal fun maj(a: Int, b: Int, c: Int): Int = (a and b) xor (a and c) xor (b and c)

/** noble's HashMD: buffers partial blocks, then pads with 0x80, zeros and the bit length. */
abstract class HashMD(
    final override val blockLen: Int,
    final override val outputLen: Int,
    private val padOffset: Int,
) : Hash {
    private val buffer = ByteArray(blockLen)
    private var length = 0.0
    private var pos = 0

    /** Compresses the [blockLen] bytes of [data] at [offset]. */
    protected abstract fun process(data: ByteArray, offset: Int)

    /** The state words, the high half before the low one for 64-bit words. */
    protected abstract fun get(): IntArray

    override fun update(data: ByteArray): HashMD {
        val len = data.size
        var pos = 0
        while (pos < len) {
            val take = minOf(blockLen - this.pos, len - pos)
            // With nothing buffered, whole blocks are compressed straight from the input.
            if (take == blockLen) {
                while (blockLen <= len - pos) {
                    process(data, pos)
                    pos += blockLen
                }
                continue
            }
            data.copyInto(buffer, this.pos, pos, pos + take)
            this.pos += take
            pos += take
            if (this.pos == blockLen) {
                process(buffer, 0)
                this.pos = 0
            }
        }
        length += len
        return this
    }

    override fun digest(): ByteArray {
        var pos = this.pos
        buffer[pos++] = 0x80.toByte()
        buffer.fill(0, pos)
        // No room for the length field: pad this block and put the length in the next.
        if (padOffset > blockLen - pos) {
            process(buffer, 0)
            buffer.fill(0)
        }
        // noble's setU64FromNum: only the low 64 bits of the length field; the rest stays zero.
        val bits = length * 8
        buffer.setUint32BE(blockLen - 8, fromNumH(bits))
        buffer.setUint32BE(blockLen - 4, fromNumL(bits))
        process(buffer, 0)
        val state = get()
        val out = ByteArray(outputLen)
        for (i in 0 until outputLen / 4) out.setUint32BE(4 * i, state[i])
        return out
    }
}
