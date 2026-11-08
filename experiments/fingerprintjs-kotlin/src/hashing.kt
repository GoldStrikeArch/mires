private class X64(var high: Int, var low: Int) {
    fun add(other: X64) {
        val sum = (low.toUInt().toLong()) + (other.low.toUInt().toLong())
        low = sum.toInt()
        val carry = (sum ushr 32).toInt()
        high = (high.toUInt() + other.high.toUInt() + carry.toUInt()).toInt()
    }

    fun multiply(other: X64) {
        val m0 = high ushr 16
        val m1 = high and 0xffff
        val m2 = low ushr 16
        val m3 = low and 0xffff
        val n0 = other.high ushr 16
        val n1 = other.high and 0xffff
        val n2 = other.low ushr 16
        val n3 = other.low and 0xffff

        var o0 = 0
        var o1 = 0
        var o2 = 0
        var o3 = mul16(m3, n3)
        o2 += o3 ushr 16
        o3 = o3 and 0xffff
        o2 += mul16(m2, n3)
        o1 += o2 ushr 16
        o2 = o2 and 0xffff
        o2 += mul16(m3, n2)
        o1 += o2 ushr 16
        o2 = o2 and 0xffff
        o1 += mul16(m1, n3)
        o0 += o1 ushr 16
        o1 = o1 and 0xffff
        o1 += mul16(m2, n2)
        o0 += o1 ushr 16
        o1 = o1 and 0xffff
        o1 += mul16(m3, n1)
        o0 += o1 ushr 16
        o1 = o1 and 0xffff
        o0 = (o0 + mul16(m0, n3) + mul16(m1, n2) + mul16(m2, n1) + mul16(m3, n0)) and 0xffff

        high = (o0 shl 16) or o1
        low = (o2 shl 16) or o3
    }

    fun rotateLeft(bitsIn: Int) {
        val bits = bitsIn % 64
        val originalHigh = high
        when {
            bits == 0 -> {}
            bits == 32 -> {
                high = low
                low = originalHigh
            }
            bits < 32 -> {
                high = (originalHigh shl bits) or (low ushr (32 - bits))
                low = (low shl bits) or (originalHigh ushr (32 - bits))
            }
            else -> {
                val shifted = bits - 32
                high = (low shl shifted) or (originalHigh ushr (32 - shifted))
                low = (originalHigh shl shifted) or (low ushr (32 - shifted))
            }
        }
    }

    fun xor(other: X64) {
        high = high xor other.high
        low = low xor other.low
    }

    fun xorByte(byte: Int, shift: Int) {
        if (shift < 32) {
            low = low xor (byte shl shift)
        } else {
            high = high xor (byte shl (shift - 32))
        }
    }

    fun finalMix() {
        xor(X64(0, high ushr 1))
        multiply(F1)
        xor(X64(0, high ushr 1))
        multiply(F2)
        xor(X64(0, high ushr 1))
    }
}

private fun mul16(a: Int, b: Int): Int {
    return ((a.toLong() and 0xffffL) * (b.toLong() and 0xffffL)).toInt()
}

private val F1 = X64(0xff51afd7.toInt(), 0xed558ccd.toInt())
private val F2 = X64(0xc4ceb9fe.toInt(), 0x1a85ec53)
private val C1 = X64(0x87c37b91.toInt(), 0x114253d5)
private val C2 = X64(0x4cf5ad43, 0x2745937f)
private val M = X64(0, 5)
private val N1 = X64(0, 0x52dce729)
private val N2 = X64(0, 0x38495ab5)

internal fun x64hash128(input: String, seed: Int = 0): String {
    val key = getUTF8Bytes(input)
    val remainder = key.size % 16
    val bytes = key.size - remainder
    val length = X64(0, key.size)
    val h1 = X64(0, seed)
    val h2 = X64(0, seed)

    var i = 0
    while (i < bytes) {
        val k1 = X64(
            key[i + 4] or (key[i + 5] shl 8) or (key[i + 6] shl 16) or (key[i + 7] shl 24),
            key[i] or (key[i + 1] shl 8) or (key[i + 2] shl 16) or (key[i + 3] shl 24),
        )
        val k2 = X64(
            key[i + 12] or (key[i + 13] shl 8) or (key[i + 14] shl 16) or (key[i + 15] shl 24),
            key[i + 8] or (key[i + 9] shl 8) or (key[i + 10] shl 16) or (key[i + 11] shl 24),
        )
        k1.multiply(C1)
        k1.rotateLeft(31)
        k1.multiply(C2)
        h1.xor(k1)
        h1.rotateLeft(27)
        h1.add(h2)
        h1.multiply(M)
        h1.add(N1)
        k2.multiply(C2)
        k2.rotateLeft(33)
        k2.multiply(C1)
        h2.xor(k2)
        h2.rotateLeft(31)
        h2.add(h1)
        h2.multiply(M)
        h2.add(N2)
        i += 16
    }

    val k1 = X64(0, 0)
    val k2 = X64(0, 0)
    val tail = bytes
    var offset = 0
    while (offset < remainder && offset < 8) {
        k1.xorByte(key[tail + offset], offset * 8)
        offset += 1
    }
    while (offset < remainder) {
        k2.xorByte(key[tail + offset], (offset - 8) * 8)
        offset += 1
    }
    if (remainder > 8) {
        k2.multiply(C2)
        k2.rotateLeft(33)
        k2.multiply(C1)
        h2.xor(k2)
    }
    if (remainder > 0) {
        k1.multiply(C1)
        k1.rotateLeft(31)
        k1.multiply(C2)
        h1.xor(k1)
    }

    h1.xor(length)
    h2.xor(length)
    h1.add(h2)
    h2.add(h1)
    h1.finalMix()
    h2.finalMix()
    h1.add(h2)
    h2.add(h1)

    return hex8(h1.high) + hex8(h1.low) + hex8(h2.high) + hex8(h2.low)
}

private fun hex8(value: Int): String {
    val hex = value.toUInt().toString(16)
    return hex.padStart(8, '0')
}

// Match JavaScript's bitwise seed coercion instead of Double.toInt saturation.
private fun seedBits(value: Double): Int {
    if (!value.isFinite()) return 0
    val truncated = if (value < 0.0) kotlin.math.ceil(value) else kotlin.math.floor(value)
    val wrapped = truncated % 4294967296.0
    val signed = if (wrapped >= 2147483648.0) wrapped - 4294967296.0
        else if (wrapped < -2147483648.0) wrapped + 4294967296.0
        else wrapped
    return signed.toInt()
}

internal fun murmurX64Hash128(input: dynamic, seed: dynamic): String {
    val text = if (isMissing(input)) "undefined" else jsString(input)
    val seedValue = if (isMissing(seed) || !isTruthy(seed)) 0 else seedBits(jsNumber(seed))
    return x64hash128(text, seedValue)
}
