// The parts of noble's utils.ts the selected hashes use: the Hash interface, the number check, and
// the byte-order reads and writes its DataView and Uint32Array views perform.

/** noble's Hash: an incremental hash with its block and digest lengths. */
interface Hash {
    val blockLen: Int
    val outputLen: Int
    fun update(data: ByteArray): Hash
    fun digest(): ByteArray
}

/** noble's anumber: [n] must be a safe integer >= 0. Values above Int.MAX_VALUE saturate. */
internal fun anumber(n: Double, title: String): Int {
    require(n >= 0 && n <= 9007199254740991.0 && n == kotlin.math.floor(n)) {
        "\"$title\" expected integer >= 0, got $n"
    }
    return n.toInt()
}

/** 32-bit words from unsigned hexadecimal literals, as noble writes its tables. */
@ExperimentalUnsignedTypes
internal fun words(vararg values: UInt): IntArray = values.asIntArray()

/** The byte at [index] as an unsigned value. */
internal fun ByteArray.u8(index: Int): Int = this[index].toInt() and 0xff

/** DataView.getUint32(offset, false). */
internal fun ByteArray.getUint32BE(offset: Int): Int =
    (u8(offset) shl 24) or (u8(offset + 1) shl 16) or (u8(offset + 2) shl 8) or u8(offset + 3)

/** A little-endian 32-bit word, as noble's Uint32Array views read on little-endian hosts. */
internal fun ByteArray.getUint32LE(offset: Int): Int =
    u8(offset) or (u8(offset + 1) shl 8) or (u8(offset + 2) shl 16) or (u8(offset + 3) shl 24)

/** DataView.setUint32(offset, value, false). */
internal fun ByteArray.setUint32BE(offset: Int, value: Int) {
    for (i in 0 until 4) this[offset + i] = (value ushr 24 - 8 * i).toByte()
}

/** Writes the first [count] bytes of the little-endian encoding of [words] into [out]. */
internal fun wordsToBytesLE(words: IntArray, out: ByteArray, count: Int) {
    for (i in 0 until count) out[i] = (words[i / 4] ushr 8 * (i % 4)).toByte()
}

/** noble's bytesToHex. */
internal fun bytesToHex(bytes: ByteArray): String {
    val digits = "0123456789abcdef"
    return buildString(bytes.size * 2) {
        for (b in bytes) {
            append(digits[(b.toInt() shr 4) and 15])
            append(digits[b.toInt() and 15])
        }
    }
}
