// Immutable lookup tables, canonical Huffman map construction (`hMap`), byte/buffer helpers, and
// FlateError for the Scala.js port of fflate 0.8.3.
// All module-level values here are read-only lookup tables computed once at module load, matching
// fflate/esm/browser.js (no mutable cross-call state).

package fflate

import scala.scalajs.js.typedarray.{Int8Array, Uint8Array, byteArray2Int8Array, int8Array2ByteArray}

final class FlateError(val code: Int, message: String) extends RuntimeException(message)

object Tables:
  val emptyBytes: Array[Byte] = new Array[Byte](0)

  // Fixed length extra bits (32 entries).
  val fleb: Array[Int] = Array(
    0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 2, 2, 2, 2,
    3, 3, 3, 3, 4, 4, 4, 4,
    5, 5, 5, 5, 0, 0, 0, 0,
  )

  // Fixed distance extra bits (32 entries).
  val fdeb: Array[Int] = Array(
    0, 0, 0, 0, 1, 1, 2, 2,
    3, 3, 4, 4, 5, 5, 6, 6,
    7, 7, 8, 8, 9, 9, 10, 10,
    11, 11, 12, 12, 13, 13, 0, 0,
  )

  // Code length alphabet order (19 entries).
  val clim: Array[Int] = Array(
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
  )

  // Deflate level options: (nice << 13) | chain for levels 1..9.
  val deo: Array[Int] = Array(
    65540, 131080, 131088, 131104, 262176, 1048704, 1048832, 2114560, 2117632,
  )

  private def buildBase(eb: Array[Int], start0: Int): Array[Int] =
    val b = new Array[Int](31)
    var st = start0
    var i = 0
    while i < 31 do
      st += (if i == 0 then 1 else 1 << eb(i - 1))
      b(i) = st
      i += 1
    b

  private def buildRevBase(b: Array[Int]): Array[Int] =
    val r = new Array[Int](b(30))
    var i = 1
    while i < 30 do
      var j = b(i)
      val end = b(i + 1)
      while j < end do
        r(j) = ((j - b(i)) << 5) | i
        j += 1
      i += 1
    r

  val fl: Array[Int] =
    val b = buildBase(fleb, 2)
    b(28) = 258
    b

  val revfl: Array[Int] =
    val b = buildBase(fleb, 2)
    val r = buildRevBase(b)
    r(258) = 28
    r

  val fd: Array[Int] = buildBase(fdeb, 0)
  val revfd: Array[Int] = buildRevBase(fd)

  // 15-bit reversal table (32768 entries, unsigned 16-bit in Array[Char] -> Uint16Array).
  val rev: Array[Char] =
    val t = new Array[Char](32768)
    var i = 0
    while i < 32768 do
      var x = ((i & 0xaaaa) >> 1) | ((i & 0x5555) << 1)
      x = ((x & 0xcccc) >> 2) | ((x & 0x3333) << 2)
      x = ((x & 0xf0f0) >> 4) | ((x & 0x0f0f) << 4)
      t(i) = ((((x & 0xff00) >> 8) | ((x & 0x00ff) << 8)) >> 1).toChar
      i += 1
    t

  // Canonical Huffman table construction (mode r == 0: encode, r == 1: decode).
  // Operates on slice cd[offset .. offset + len).
  def hMapSlice(cd: Array[Byte], offset: Int, len: Int, mb: Int, r: Int): Array[Char] =
    val l = new Array[Char](mb)
    var i = 0
    while i < len do
      val c = cd(offset + i) & 0xff
      if c != 0 then
        val idx = c - 1
        l(idx) = (l(idx).toInt + 1).toChar
      i += 1

    val le = new Array[Char](mb)
    i = 1
    while i < mb do
      le(i) = (((le(i - 1).toInt + l(i - 1).toInt) << 1) & 0xffff).toChar
      i += 1

    val revTbl = rev
    if r != 0 then
      val co = new Array[Char](1 << mb)
      val rvb = 15 - mb
      i = 0
      while i < len do
        val c = cd(offset + i) & 0xff
        if c != 0 then
          val sv = ((i << 4) | c).toChar
          val r1 = mb - c
          val cur = le(c - 1).toInt
          le(c - 1) = ((cur + 1) & 0xffff).toChar
          var v = cur << r1
          val m = v | ((1 << r1) - 1)
          while v <= m do
            val dest = if v < 32768 then revTbl(v).toInt >>> rvb else 0
            co(dest) = sv
            v += 1
        i += 1
      co
    else
      val co = new Array[Char](len)
      i = 0
      while i < len do
        val c = cd(offset + i) & 0xff
        if c != 0 then
          val cur = le(c - 1).toInt
          le(c - 1) = ((cur + 1) & 0xffff).toChar
          val code = if cur < 32768 then revTbl(cur).toInt >>> (15 - c) else 0
          co(i) = code.toChar
        i += 1
      co

  def hMap(cd: Array[Byte], mb: Int, r: Int): Array[Char] =
    hMapSlice(cd, 0, cd.length, mb, r)

  // Fixed literal/length code lengths (288 entries).
  val flt: Array[Byte] =
    val t = new Array[Byte](288)
    var i = 0
    while i < 144 do { t(i) = 8; i += 1 }
    while i < 256 do { t(i) = 9; i += 1 }
    while i < 280 do { t(i) = 7; i += 1 }
    while i < 288 do { t(i) = 8; i += 1 }
    t

  // Fixed distance code lengths (32 entries).
  val fdt: Array[Byte] =
    val t = new Array[Byte](32)
    var i = 0
    while i < 32 do { t(i) = 5; i += 1 }
    t

  val flm: Array[Char] = hMap(flt, 9, 0)
  val flrm: Array[Char] = hMap(flt, 9, 1)
  val fdm: Array[Char] = hMap(fdt, 5, 0)
  val fdrm: Array[Char] = hMap(fdt, 5, 1)

  // Reflected IEEE CRC-32 lookup table (256 entries, polynomial 0xEDB88320 == -306674912).
  val crct: Array[Int] =
    val t = new Array[Int](256)
    var i = 0
    while i < 256 do
      var c = i
      var k = 8
      while k > 0 do
        c = (if (c & 1) != 0 then -306674912 else 0) ^ (c >>> 1)
        k -= 1
      t(i) = c
      i += 1
    t

  // Exact integer cutoffs for Math.ceil(Math.max(8, Math.min(13, Math.log(len))) * 1.5).
  def defaultPlvl(len: Int): Int =
    if len <= 2980 then 12
    else if len <= 5806 then 13
    else if len <= 11308 then 14
    else if len <= 22026 then 15
    else if len <= 42901 then 16
    else if len <= 83561 then 17
    else if len <= 162754 then 18
    else if len <= 317003 then 19
    else 20

  inline def shft(p: Int): Int = (p + 7) >> 3

  def maxSlice(a: Array[Byte], offset: Int, len: Int): Int =
    if len <= 0 then 0
    else
      var m = a(offset) & 0xff
      var i = 1
      while i < len do
        val v = a(offset + i) & 0xff
        if v > m then m = v
        i += 1
      m

  def wbytes(d: Array[Byte], b: Int, v0: Int): Unit =
    var v = v0
    var i = b
    while v != 0 do
      d(i) = v.toByte
      v >>>= 8
      i += 1

  // Convert non-negative Double in [0, 0xFFFFFFFF] to unsigned 32-bit pattern in Int.
  def doubleToU32(x: Double): Int =
    if x >= 2147483648.0 then (x - 4294967296.0).toInt
    else x.toInt

  // Clamped fresh byte-array slice matching fflate's `slc(v, s, e)`.
  def sliceBytes(v: Array[Byte], s0: Int, e0: Int): Array[Byte] =
    val s = if s0 < 0 then 0 else if s0 > v.length then v.length else s0
    val e = if e0 < s then s else if e0 > v.length then v.length else e0
    val n = e - s
    if n <= 0 then emptyBytes
    else
      val out = new Array[Byte](n)
      System.arraycopy(v, s, out, 0, n)
      out

  // Zero-copy view from Scala Array[Byte] (Int8Array) to JS Uint8Array.
  def bytesToU8(bytes: Array[Byte]): Uint8Array =
    val i8 = byteArray2Int8Array(bytes)
    new Uint8Array(i8.buffer, i8.byteOffset, i8.byteLength)

  // Clamped fresh Uint8Array copy of bytes[s0 .. e0).
  def bytesSliceToU8(v: Array[Byte], s0: Int, e0: Int): Uint8Array =
    bytesToU8(sliceBytes(v, s0, e0))

  // Zero-copy view from JS Uint8Array to Scala Array[Byte] (Int8Array).
  def u8ToBytes(u8: Uint8Array): Array[Byte] =
    int8Array2ByteArray(new Int8Array(u8.buffer, u8.byteOffset, u8.byteLength))
