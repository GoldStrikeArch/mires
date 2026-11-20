// noble's hmac.ts.

/** noble's _HMAC over the hash that [create] makes. */
class Hmac(create: () -> Hash, key: ByteArray) : Hash {
    private val iHash = create()
    private val oHash: Hash
    override val blockLen = iHash.blockLen
    override val outputLen = iHash.outputLen

    init {
        val pad = ByteArray(blockLen)
        // A key longer than a block is replaced by its digest.
        (if (key.size > blockLen) create().update(key).digest() else key).copyInto(pad)
        for (i in pad.indices) pad[i] = (pad[i].toInt() xor 0x36).toByte()
        iHash.update(pad)
        oHash = create()
        // Undo the inner XOR and apply the outer one.
        for (i in pad.indices) pad[i] = (pad[i].toInt() xor (0x36 xor 0x5c)).toByte()
        oHash.update(pad)
    }

    override fun update(data: ByteArray): Hmac {
        iHash.update(data)
        return this
    }

    override fun digest(): ByteArray = oHash.update(iHash.digest()).digest()
}
