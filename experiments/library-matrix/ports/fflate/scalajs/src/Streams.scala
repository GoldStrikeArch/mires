// Synchronous streaming classes (`Deflate`, `Gzip`, `Zlib`, `Inflate`, `Gunzip`, `Unzlib`,
// `Decompress`). Exact port of fflate 0.8.3, preserving exact chunk boundaries, flush semantics,
// and Gunzip's recursive multi-member emission sequence.

package fflate

import scala.scalajs.js.typedarray.Uint8Array

type DataHandler = (Uint8Array, Boolean) => Unit

class Deflate(opts0: CompressOptions, val ondata: DataHandler | Null):
  protected var o: CompressOptions = opts0
  protected var s: DeflateState = new DeflateState(l = 0, i = 32768, w = 32768, z = 32768)
  protected var b: Array[Byte] = new Array[Byte](98304)

  if o.dictionary != null then
    val rawDict = o.dictionary.asInstanceOf[Array[Byte]]
    val dLen = if rawDict.length > 32768 then 32768 else rawDict.length
    System.arraycopy(rawDict, rawDict.length - dLen, b, 32768 - dLen, dLen)
    s.i = 32768 - dLen

  protected def emit(chunk: Uint8Array, isFinal: Boolean): Unit =
    ondata.asInstanceOf[DataHandler](chunk, isFinal)

  protected def p(c: Array[Byte], f: Boolean): Unit =
    emit(Tables.bytesToU8(DeflateCore.doptBytes(c, o, 0, 0, s)), f)

  def push(chunk: Array[Byte], isFinal: Boolean): Unit =
    if ondata == null then throw new FlateError(5, "no stream handler")
    if s.l != 0 then throw new FlateError(4, "stream finished")
    val endLen = chunk.length + s.z
    if endLen > b.length then
      if endLen > 2 * b.length - 32768 then
        val newBuf = new Array[Byte](endLen & -32768)
        System.arraycopy(b, 0, newBuf, 0, s.z)
        b = newBuf
      val split = b.length - s.z
      if split > 0 then System.arraycopy(chunk, 0, b, s.z, split)
      s.z = b.length
      p(b, false)
      System.arraycopy(b, b.length - 32768, b, 0, 32768)
      val rem = chunk.length - split
      if rem > 0 then System.arraycopy(chunk, split, b, 32768, rem)
      s.z = rem + 32768
      s.i = 32766
      s.w = 32768
    else
      if chunk.length > 0 then System.arraycopy(chunk, 0, b, s.z, chunk.length)
      s.z += chunk.length

    s.l = if isFinal then 1 else 0
    if s.z > s.w + 8191 || isFinal then
      p(b, isFinal)
      s.w = s.i
      s.i -= 2
    if isFinal then
      s = new DeflateState()
      o = CompressOptions.empty
      b = Tables.emptyBytes

  def flush(sync: Boolean): Unit =
    if ondata == null then throw new FlateError(5, "no stream handler")
    if s.l != 0 then throw new FlateError(4, "stream finished")
    p(b, false)
    s.w = s.i
    s.i -= 2
    if sync then
      val c = new Array[Byte](6)
      c(0) = (s.r >> 3).toByte
      val ep = DeflateCore.wfblk(c, 0, s.r, Tables.emptyBytes, 0, 0)
      s.r = 0
      emit(Tables.bytesSliceToU8(c, 0, ep >> 3), false)

final class Gzip(opts: CompressOptions, ondata: DataHandler | Null) extends Deflate(opts, ondata):
  private val crc = new Crc32()
  private var totalLen: Int = 0
  private var headerPending: Int = 1

  override def push(chunk: Array[Byte], isFinal: Boolean): Unit =
    crc.push(chunk)
    totalLen += chunk.length
    super.push(chunk, isFinal)

  override protected def p(c: Array[Byte], f: Boolean): Unit =
    val pre = if headerPending != 0 then Framing.gzhl(o) else 0
    val post = if f then 8 else 0
    val raw = DeflateCore.doptBytes(c, o, pre, post, s)
    if headerPending != 0 then
      Framing.gzh(raw, o)
      headerPending = 0
    if f then
      Tables.wbytes(raw, raw.length - 8, crc.digest())
      Tables.wbytes(raw, raw.length - 4, totalLen)
    emit(Tables.bytesToU8(raw), f)

final class Zlib(opts: CompressOptions, ondata: DataHandler | Null) extends Deflate(opts, ondata):
  private val adler = new Adler32()
  private var headerPending: Int = 1

  override def push(chunk: Array[Byte], isFinal: Boolean): Unit =
    adler.push(chunk)
    super.push(chunk, isFinal)

  override protected def p(c: Array[Byte], f: Boolean): Unit =
    val pre = if headerPending != 0 then (if o.dictionary != null then 6 else 2) else 0
    val post = if f then 4 else 0
    val raw = DeflateCore.doptBytes(c, o, pre, post, s)
    if headerPending != 0 then
      Framing.zlh(raw, o)
      headerPending = 0
    if f then Tables.wbytes(raw, raw.length - 4, adler.digest())
    emit(Tables.bytesToU8(raw), f)

trait DecompressStream:
  def push(chunk: Array[Byte], isFinal: Boolean): Unit

class Inflate(opts: DecompressOptions, val ondata: DataHandler | Null) extends DecompressStream:
  protected var s: InflateState =
    val dictLen =
      if opts.dictionary != null then
        val raw = opts.dictionary.asInstanceOf[Array[Byte]]
        if raw.length > 32768 then 32768 else raw.length
      else 0
    new InflateState(i = 0, b = dictLen)

  protected var o: Array[Byte] =
    val buf = new Array[Byte](32768)
    if opts.dictionary != null then
      val raw = opts.dictionary.asInstanceOf[Array[Byte]]
      val dLen = if raw.length > 32768 then 32768 else raw.length
      System.arraycopy(raw, raw.length - dLen, buf, 0, dLen)
    buf

  protected var p: Array[Byte] = Tables.emptyBytes
  protected var d: Boolean = false

  protected def e(c: Array[Byte]): Unit =
    if ondata == null then throw new FlateError(5, "no stream handler")
    if d then throw new FlateError(4, "stream finished")
    if p.length == 0 then p = c
    else if c.length > 0 then
      val n = new Array[Byte](p.length + c.length)
      System.arraycopy(p, 0, n, 0, p.length)
      System.arraycopy(c, 0, n, p.length, c.length)
      p = n

  protected def c(isFinal: Boolean): Unit =
    d = isFinal
    s.i = if isFinal then 1 else 0
    val bts = s.b
    val dtRes = InflateCore.infltRaw(p, s, o, null)
    val dt = dtRes.buf
    val dtLen = dtRes.len
    val emitEnd = if s.b < dtLen then s.b else dtLen
    ondata.asInstanceOf[DataHandler](Tables.bytesSliceToU8(dt, bts, emitEnd), d)
    o = Tables.sliceBytes(dt, s.b - 32768, dtLen)
    s.b = o.length
    p = Tables.sliceBytes(p, s.p >> 3, p.length)
    s.p &= 7

  def push(chunk: Array[Byte], isFinal: Boolean): Unit =
    e(chunk)
    c(isFinal)

final class Gunzip(opts: DecompressOptions, ondata: DataHandler | Null) extends Inflate(opts, ondata):
  private var v: Int = 1
  private var r: Int = 0

  override def push(chunk: Array[Byte], isFinal: Boolean): Unit =
    e(chunk)
    r += chunk.length
    if v != 0 then
      val pv = Tables.sliceBytes(p, v - 1, p.length)
      val sLen = if pv.length > 3 then Framing.gzs(pv) else 4
      if sLen > pv.length then
        if !isFinal then return
      p = Tables.sliceBytes(pv, sLen, pv.length)
      v = 0
    c(false)
    if s.f != 0 && s.l == null then
      v = Tables.shft(s.p) + 9
      s = new InflateState(i = 0)
      o = Tables.emptyBytes
      push(Tables.emptyBytes, isFinal)
    else if isFinal then
      c(true)

final class Unzlib(opts: DecompressOptions, ondata: DataHandler | Null) extends Inflate(opts, ondata):
  private var v: Int = if opts.dictionary != null then 2 else 1

  override def push(chunk: Array[Byte], isFinal: Boolean): Unit =
    e(chunk)
    if v != 0 then
      if p.length < 6 && !isFinal then return
      p = Tables.sliceBytes(p, Framing.zls(p, v - 1 != 0), p.length)
      v = 0
    if isFinal then
      if p.length < 4 then throw new FlateError(6, "invalid zlib data")
      p = Tables.sliceBytes(p, 0, p.length - 4)
    c(isFinal)

final class Decompress(opts: DecompressOptions, val ondata: DataHandler | Null) extends DecompressStream:
  private var inner: Inflate | Null = null
  private var p: Array[Byte] | Null = null

  def push(chunk: Array[Byte], isFinal: Boolean): Unit =
    if ondata == null then throw new FlateError(5, "no stream handler")
    if inner == null then
      if p != null && p.asInstanceOf[Array[Byte]].length > 0 then
        val prev = p.asInstanceOf[Array[Byte]]
        val n = new Array[Byte](prev.length + chunk.length)
        System.arraycopy(prev, 0, n, 0, prev.length)
        System.arraycopy(chunk, 0, n, prev.length, chunk.length)
        p = n
      else p = chunk

      val cur = p.asInstanceOf[Array[Byte]]
      if cur.length > 2 then
        val d0 = cur(0) & 0xff
        val d1 = cur(1) & 0xff
        val d2 = cur(2) & 0xff
        val strm: Inflate =
          if d0 == 31 && d1 == 139 && d2 == 8 then new Gunzip(opts, ondata)
          else if (d0 & 15) != 8 || (d0 >> 4) > 7 || (((d0 << 8) | d1) % 31) != 0 then new Inflate(opts, ondata)
          else new Unzlib(opts, ondata)
        inner = strm
        strm.push(cur, isFinal)
        p = null
    else inner.asInstanceOf[Inflate].push(chunk, isFinal)
