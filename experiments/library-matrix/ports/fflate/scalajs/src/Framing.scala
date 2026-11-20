// CRC-32, Adler-32, GZIP and Zlib header/footer framing, and one-shot sync wrappers
// (`gzipSync`, `gunzipSync`, `zlibSync`, `unzlibSync`, `decompressSync`). Exact port of fflate 0.8.3.

package fflate

import scala.scalajs.js
import scala.scalajs.js.typedarray.Uint8Array

final class Crc32:
  private var c: Int = -1

  def push(d: Array[Byte]): Unit =
    var cr = c
    var i = 0
    val len = d.length
    val t = Tables.crct
    while i < len do
      cr = t((cr & 255) ^ (d(i) & 0xff)) ^ (cr >>> 8)
      i += 1
    c = cr

  def digest(): Int = ~c

final class Adler32:
  private var a: Int = 1
  private var b: Int = 0

  def push(d: Array[Byte]): Unit =
    var n = a
    var m = b
    val l = d.length
    var i = 0
    while i != l do
      val e = if i + 2655 < l then i + 2655 else l
      while i < e do
        n += d(i) & 0xff
        m += n
        i += 1
      n = (n & 65535) + 15 * (n >> 16)
      m = (m & 65535) + 15 * (m >> 16)
    a = n
    b = m

  def digest(): Int =
    a %= 65521
    b %= 65521
    ((a & 255) << 24) | ((a & 0xff00) << 8) | ((b & 255) << 8) | (b >> 8)

object Framing:
  def gzhl(o: CompressOptions): Int =
    val fn = o.filename
    10 + (if fn != null && fn.asInstanceOf[String].length > 0 then fn.asInstanceOf[String].length + 1 else 0)

  def gzh(c: Array[Byte], o: CompressOptions): Unit =
    val fn = o.filename
    c(0) = 31
    c(1) = 139.toByte
    c(2) = 8
    c(8) = (if o.hasLevel && o.level < 2 then 4 else if o.hasLevel && o.level == 9 then 2 else 0).toByte
    c(9) = 3
    if !o.hasMtime then
      val sec = Math.floor(js.Date.now() / 1000.0)
      Tables.wbytes(c, 4, Tables.doubleToU32(sec))
    else if o.mtime != 0.0 then
      val sec = Math.floor(o.mtime / 1000.0)
      Tables.wbytes(c, 4, Tables.doubleToU32(sec))
    if fn != null then
      val name = fn.asInstanceOf[String]
      if name.length > 0 then
        c(3) = 8
        var i = 0
        val len = name.length
        while i < len do
          c(i + 10) = (name.charAt(i).toInt & 0xff).toByte
          i += 1
        c(len + 10) = 0

  def gzs(d: Array[Byte]): Int =
    val len = d.length
    val d0 = if len > 0 then d(0) & 0xff else 0
    val d1 = if len > 1 then d(1) & 0xff else 0
    val d2 = if len > 2 then d(2) & 0xff else 0
    if d0 != 31 || d1 != 139 || d2 != 8 then throw new FlateError(6, "invalid gzip data")
    val flg = if len > 3 then d(3) & 0xff else 0
    var st = 10
    if (flg & 4) != 0 then
      val b10 = if len > 10 then d(10) & 0xff else 0
      val b11 = if len > 11 then d(11) & 0xff else 0
      st += (b10 | (b11 << 8)) + 2
    var zs = ((flg >> 3) & 1) + ((flg >> 4) & 1)
    while zs > 0 do
      val b = if st < len then d(st) & 0xff else 0
      st += 1
      if b == 0 then zs -= 1
    st + (flg & 2)

  def gzl(d: Array[Byte]): Int =
    val l = d.length
    (d(l - 4) & 0xff) | ((d(l - 3) & 0xff) << 8) | ((d(l - 2) & 0xff) << 16) | ((d(l - 1) & 0xff) << 24)

  def gzipSync(data: Array[Byte], opts: CompressOptions): Uint8Array =
    val c = new Crc32()
    val l = data.length
    c.push(data)
    val d = DeflateCore.doptBytes(data, opts, gzhl(opts), 8, null)
    val s = d.length
    gzh(d, opts)
    Tables.wbytes(d, s - 8, c.digest())
    Tables.wbytes(d, s - 4, l)
    Tables.bytesToU8(d)

  def gunzipSync(data: Array[Byte], opts: DecompressOptions): Uint8Array =
    val st = gzs(data)
    if st + 8 > data.length then throw new FlateError(6, "invalid gzip data")
    val outBuf: Array[Byte] | Null =
      if opts.out != null then opts.out
      else
        val isize = gzl(data)
        if isize >= 0 then new Array[Byte](isize) else null
    val body = Tables.sliceBytes(data, st, data.length - 8)
    InflateCore.inflt(body, new InflateState(i = 2), outBuf, opts.dictionary)

  def zlh(c: Array[Byte], o: CompressOptions): Unit =
    val fl =
      if !o.hasLevel then 2
      else if o.level == 0 then 0
      else if o.level < 6 then 1
      else if o.level == 9 then 3
      else 2
    c(0) = 120
    var c1 = (fl << 6) | (if o.dictionary != null then 32 else 0)
    c1 |= 31 - (((120 << 8) | c1) % 31)
    c(1) = c1.toByte
    if o.dictionary != null then
      val h = new Adler32()
      h.push(o.dictionary.asInstanceOf[Array[Byte]])
      Tables.wbytes(c, 2, h.digest())

  def zls(d: Array[Byte], hasDict: Boolean): Int =
    val len = d.length
    val d0 = if len > 0 then d(0) & 0xff else 0
    val d1 = if len > 1 then d(1) & 0xff else 0
    if (d0 & 15) != 8 || (d0 >> 4) > 7 || (((d0 << 8) | d1) % 31) != 0 then
      throw new FlateError(6, "invalid zlib data")
    val fdict = (d1 >> 5) & 1
    if fdict == (if hasDict then 0 else 1) then
      throw new FlateError(
        6,
        if (d1 & 32) != 0 then "invalid zlib data: need dictionary" else "invalid zlib data: unexpected dictionary",
      )
    ((d1 >> 3) & 4) + 2

  def zlibSync(data: Array[Byte], opts: CompressOptions): Uint8Array =
    val a = new Adler32()
    a.push(data)
    val pre = if opts.dictionary != null then 6 else 2
    val d = DeflateCore.doptBytes(data, opts, pre, 4, null)
    zlh(d, opts)
    Tables.wbytes(d, d.length - 4, a.digest())
    Tables.bytesToU8(d)

  def unzlibSync(data: Array[Byte], opts: DecompressOptions): Uint8Array =
    val start = zls(data, opts.dictionary != null)
    val end = if data.length > 4 then data.length - 4 else 0
    val body = Tables.sliceBytes(data, start, end)
    InflateCore.inflt(body, new InflateState(i = 2), opts.out, opts.dictionary)

  def decompressSync(data: Array[Byte], opts: DecompressOptions): Uint8Array =
    val len = data.length
    val d0 = if len > 0 then data(0) & 0xff else 0
    val d1 = if len > 1 then data(1) & 0xff else 0
    val d2 = if len > 2 then data(2) & 0xff else 0
    if d0 == 31 && d1 == 139 && d2 == 8 then gunzipSync(data, opts)
    else if (d0 & 15) != 8 || (d0 >> 4) > 7 || (((d0 << 8) | d1) % 31) != 0 then InflateCore.inflateSync(data, opts)
    else unzlibSync(data, opts)
