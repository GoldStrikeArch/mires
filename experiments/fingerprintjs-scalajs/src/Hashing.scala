package fp

import scala.scalajs.js
import scala.scalajs.js.typedarray.Uint8Array

/** MurmurHash3 x64 128, matching FingerprintJS 5.2.0. */
object Hashing {
  private final class X64(var high: Int, var low: Int) {
    def add(other: X64): Unit = {
      val m0 = high >>> 16
      val m1 = high & 0xffff
      val m2 = low >>> 16
      val m3 = low & 0xffff
      val n0 = other.high >>> 16
      val n1 = other.high & 0xffff
      val n2 = other.low >>> 16
      val n3 = other.low & 0xffff
      var o0 = 0
      var o1 = 0
      var o2 = 0
      var o3 = 0
      o3 += m3 + n3
      o2 += o3 >>> 16
      o3 &= 0xffff
      o2 += m2 + n2
      o1 += o2 >>> 16
      o2 &= 0xffff
      o1 += m1 + n1
      o0 += o1 >>> 16
      o1 &= 0xffff
      o0 += m0 + n0
      o0 &= 0xffff
      high = (o0 << 16) | o1
      low = (o2 << 16) | o3
    }

    def multiply(other: X64): Unit = {
      val m0 = high >>> 16
      val m1 = high & 0xffff
      val m2 = low >>> 16
      val m3 = low & 0xffff
      val n0 = other.high >>> 16
      val n1 = other.high & 0xffff
      val n2 = other.low >>> 16
      val n3 = other.low & 0xffff
      var o0 = 0
      var o1 = 0
      var o2 = 0
      var o3 = 0
      o3 += m3 * n3
      o2 += o3 >>> 16
      o3 &= 0xffff
      o2 += m2 * n3
      o1 += o2 >>> 16
      o2 &= 0xffff
      o2 += m3 * n2
      o1 += o2 >>> 16
      o2 &= 0xffff
      o1 += m1 * n3
      o0 += o1 >>> 16
      o1 &= 0xffff
      o1 += m2 * n2
      o0 += o1 >>> 16
      o1 &= 0xffff
      o1 += m3 * n1
      o0 += o1 >>> 16
      o1 &= 0xffff
      o0 += m0 * n3 + m1 * n2 + m2 * n1 + m3 * n0
      o0 &= 0xffff
      high = (o0 << 16) | o1
      low = (o2 << 16) | o3
    }

    def rotateLeft(bits0: Int): Unit = {
      val savedHigh = high
      var bits = bits0 % 64
      if (bits == 32) {
        high = low
        low = savedHigh
      } else if (bits < 32) {
        high = (savedHigh << bits) | (low >>> (32 - bits))
        low = (low << bits) | (savedHigh >>> (32 - bits))
      } else {
        bits -= 32
        high = (low << bits) | (savedHigh >>> (32 - bits))
        low = (savedHigh << bits) | (low >>> (32 - bits))
      }
    }

    def xor(other: X64): Unit = {
      high ^= other.high
      low ^= other.low
    }

    def xorByte(byte: Int, shift: Int): Unit = {
      val value = byte & 0xff
      if (shift < 32) low ^= value << shift
      else high ^= value << (shift - 32)
    }

    def finalMix(): Unit = {
      xor(new X64(0, high >>> 1))
      multiply(F1)
      xor(new X64(0, high >>> 1))
      multiply(F2)
      xor(new X64(0, high >>> 1))
    }
  }

  private val F1 = new X64(0xff51afd7, 0xed558ccd)
  private val F2 = new X64(0xc4ceb9fe, 0x1a85ec53)
  private val C1 = new X64(0x87c37b91, 0x114253d5)
  private val C2 = new X64(0x4cf5ad43, 0x2745937f)
  private val M = new X64(0, 5)
  private val N1 = new X64(0, 0x52dce729)
  private val N2 = new X64(0, 0x38495ab5)

  def getUTF8Bytes(input: String): Uint8Array = {
    val result = new Uint8Array(input.length)
    var i = 0
    while (i < input.length) {
      val charCode = input.charAt(i).toInt
      if (charCode > 127) return Host.textEncoderEncode(input)
      result(i) = charCode.toShort
      i += 1
    }
    result
  }

  private def hex8(n: Int): String = {
    val hex = Integer.toUnsignedString(n, 16)
    "0" * (8 - hex.length) + hex
  }

  // JavaScript bitwise seed coercion wraps modulo 2^32; Double.toInt clamps.
  // In particular, 0xffffffff must initialize the low word to -1, not Int.MaxValue.
  private def seedBits(value: Double): Int = {
    if (value.isNaN || value.isInfinity) 0
    else {
      val truncated = if (value < 0.0) math.ceil(value) else math.floor(value)
      val wrapped = truncated % 4294967296.0
      val signed = if (wrapped >= 2147483648.0) wrapped - 4294967296.0
        else if (wrapped < -2147483648.0) wrapped + 4294967296.0
        else wrapped
      signed.toInt
    }
  }

  def x64hash128(input: String, seed0: js.UndefOr[Double] = js.undefined): String = {
    val key = getUTF8Bytes(input)
    val seed = seedBits(seed0.getOrElse(0.0))
    val remainder = key.length % 16
    val bytes = key.length - remainder
    val length = new X64(0, key.length)
    val h1 = new X64(0, seed)
    val h2 = new X64(0, seed)
    var i = 0
    while (i < bytes) {
      val k1 = new X64(
        u32(key, i + 4),
        u32(key, i),
      )
      val k2 = new X64(
        u32(key, i + 12),
        u32(key, i + 8),
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
    val k1 = new X64(0, 0)
    val k2 = new X64(0, 0)
    var offset = 0
    while (offset < remainder && offset < 8) {
      k1.xorByte(key(i + offset).toInt, offset * 8)
      offset += 1
    }
    while (offset < remainder) {
      k2.xorByte(key(i + offset).toInt, (offset - 8) * 8)
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
    hex8(h1.high) + hex8(h1.low) + hex8(h2.high) + hex8(h2.low)
  }

  private def u32(key: Uint8Array, i: Int): Int =
    (key(i).toInt & 0xff) |
      ((key(i + 1).toInt & 0xff) << 8) |
      ((key(i + 2).toInt & 0xff) << 16) |
      ((key(i + 3).toInt & 0xff) << 24)

  val murmurX64Hash128: js.Function2[String, js.UndefOr[Double], String] =
    (input, seed) => x64hash128(input, seed)
}
