package fflate

import org.khronos.webgl.Int8Array
import org.khronos.webgl.Uint8Array

private external class TextEncoder() {
    fun encode(input: String): Uint8Array
}

private external class TextDecoder() {
    fun decode(input: Uint8Array): String
}

private val textEncoder = TextEncoder()
private val textDecoder = TextDecoder()

internal fun ByteArray.toUint8Array(): Uint8Array {
    val i8 = this.unsafeCast<Int8Array>()
    return Uint8Array(i8.buffer, i8.byteOffset, i8.byteLength)
}

internal fun Uint8Array.toByteArray(): ByteArray =
    Int8Array(this.buffer, this.byteOffset, this.byteLength).unsafeCast<ByteArray>()

internal fun strToU8(str: String, latin1: Boolean): ByteArray {
    if (latin1) {
        val len = str.length
        val out = ByteArray(len)
        for (i in 0 until len) {
            out[i] = str[i].code.toByte()
        }
        return out
    }
    return textEncoder.encode(str).toByteArray()
}

internal fun strFromU8(dat: ByteArray, latin1: Boolean): String {
    if (latin1) {
        val len = dat.size
        val chars = CharArray(len)
        for (i in 0 until len) {
            chars[i] = Char(dat[i].toInt() and 0xFF)
        }
        return chars.concatToString()
    }
    return textDecoder.decode(dat.toUint8Array())
}

internal fun stringToUtf16Le(text: String): ByteArray {
    val len = text.length
    val out = ByteArray(len * 2)
    for (i in 0 until len) {
        val unit = text[i].code
        out[i * 2] = unit.toByte()
        out[i * 2 + 1] = (unit ushr 8).toByte()
    }
    return out
}
