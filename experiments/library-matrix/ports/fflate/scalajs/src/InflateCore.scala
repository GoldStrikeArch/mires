// DEFLATE decompressor (`inflt` state machine and `inflateSync`). Exact port of fflate 0.8.3.

package fflate

import scala.scalajs.js
import scala.scalajs.js.typedarray.Uint8Array

final class DecompressOptions(
    val dictionary: Array[Byte] | Null = null,
    val out: Array[Byte] | Null = null,
)

object DecompressOptions:
  val empty: DecompressOptions = new DecompressOptions()

final class InflateState(
    var i: Int = 0,
    var b: Int = 0,
    var p: Int = 0,
    var f: Int = 0,
    var l: Array[Char] | Null = null,
    var d: Array[Char] | Null = null,
    var m: Int = 0,
    var n: Int = 0,
)

final class InflateResult(val buf: Array[Byte], val len: Int)

object InflateCore:
  private inline def bits(d: Array[Byte], p: Int, m: Int): Int =
    val o = p >> 3
    val len = d.length
    val b0 = if o < len then d(o) & 0xff else 0
    val b1 = if o + 1 < len then d(o + 1) & 0xff else 0
    ((b0 | (b1 << 8)) >> (p & 7)) & m

  private inline def bits16(d: Array[Byte], p: Int): Int =
    val o = p >> 3
    val len = d.length
    if o + 2 < len then
      ((d(o) & 0xff) | ((d(o + 1) & 0xff) << 8) | ((d(o + 2) & 0xff) << 16)) >> (p & 7)
    else
      val b0 = if o < len then d(o) & 0xff else 0
      val b1 = if o + 1 < len then d(o + 1) & 0xff else 0
      (b0 | (b1 << 8)) >> (p & 7)

  // Runs `inflt` and returns (backing array, valid length) to avoid extra copies in streaming Inflate.
  def infltRaw(
      dat: Array[Byte],
      st: InflateState,
      buf0: Array[Byte] | Null,
      dict0: Array[Byte] | Null,
  ): InflateResult =
    val sl = dat.length
    val dl = if dict0 != null then dict0.asInstanceOf[Array[Byte]].length else 0
    if sl == 0 || (st.f != 0 && st.l == null) then
      val ret = if buf0 != null then buf0.asInstanceOf[Array[Byte]] else Tables.emptyBytes
      return new InflateResult(ret, ret.length)

    val noBuf = buf0 == null
    val resize = noBuf || st.i != 2
    val noSt = st.i
    var buf: Array[Byte] = if noBuf then new Array[Byte](sl * 3) else buf0.asInstanceOf[Array[Byte]]

    def cbuf(req: Int): Unit =
      val bl = buf.length
      if req > bl then
        val dbl = bl * 2
        val nlen = if dbl > req then dbl else req
        val nbuf = new Array[Byte](nlen)
        System.arraycopy(buf, 0, nbuf, 0, bl)
        buf = nbuf

    var isFinal = st.f
    var pos = st.p
    var bt = st.b
    var lm: Array[Char] | Null = st.l
    var dm: Array[Char] | Null = st.d
    var lbt = st.m
    var dbt = st.n
    val tbts = sl * 8

    val fleb = Tables.fleb
    val fdeb = Tables.fdeb
    val fl = Tables.fl
    val fd = Tables.fd
    val clim = Tables.clim

    var outerDone = false
    while !outerDone do
      var skipSymbols = false
      if lm == null then
        isFinal = bits(dat, pos, 1)
        val btype = bits(dat, pos + 1, 3)
        pos += 3
        if btype == 0 then
          val s = Tables.shft(pos) + 4
          val b0 = if s - 4 < sl then dat(s - 4) & 0xff else 0
          val b1 = if s - 3 < sl then dat(s - 3) & 0xff else 0
          val l = b0 | (b1 << 8)
          val t = s + l
          if t > sl then
            if noSt != 0 then throw new FlateError(0, "unexpected EOF")
            outerDone = true
            skipSymbols = true
          else
            if resize then cbuf(bt + l)
            else if bt + l > buf.length then
              throw new js.JavaScriptException(new js.RangeError("offset is out of bounds"))
            if l > 0 then System.arraycopy(dat, s, buf, bt, l)
            bt += l
            pos = t * 8
            st.b = bt
            st.p = pos
            st.f = isFinal
            if isFinal != 0 then outerDone = true
            skipSymbols = true
        else if btype == 1 then
          lm = Tables.flrm
          dm = Tables.fdrm
          lbt = 9
          dbt = 5
        else if btype == 2 then
          val hLit = bits(dat, pos, 31) + 257
          val hcLen = bits(dat, pos + 10, 15) + 4
          val tl = hLit + bits(dat, pos + 5, 31) + 1
          pos += 14
          val ldt = new Array[Byte](tl)
          val clt = new Array[Byte](19)
          var i = 0
          while i < hcLen do
            clt(clim(i)) = bits(dat, pos + i * 3, 7).toByte
            i += 1
          pos += hcLen * 3
          val clb = Tables.maxSlice(clt, 0, 19)
          val clbmsk = (1 << clb) - 1
          val clm = Tables.hMap(clt, clb, 1)
          i = 0
          while i < tl do
            val r = clm(bits(dat, pos, clbmsk)).toInt
            pos += r & 15
            val s = r >> 4
            if s < 16 then
              ldt(i) = s.toByte
              i += 1
            else
              var cv: Byte = 0
              var n = 0
              if s == 16 then
                n = 3 + bits(dat, pos, 3)
                pos += 2
                cv = if i > 0 then ldt(i - 1) else 0
              else if s == 17 then
                n = 3 + bits(dat, pos, 7)
                pos += 3
              else if s == 18 then
                n = 11 + bits(dat, pos, 127)
                pos += 7
              while n > 0 do
                if i < tl then ldt(i) = cv
                i += 1
                n -= 1
          lbt = Tables.maxSlice(ldt, 0, hLit)
          dbt = Tables.maxSlice(ldt, hLit, tl - hLit)
          lm = Tables.hMapSlice(ldt, 0, hLit, lbt, 1)
          dm = Tables.hMapSlice(ldt, hLit, tl - hLit, dbt, 1)
        else
          throw new FlateError(1, "invalid block type")

        if !skipSymbols && pos > tbts then
          if noSt != 0 then throw new FlateError(0, "unexpected EOF")
          outerDone = true
          skipSymbols = true

      if !skipSymbols then
        if resize then cbuf(bt + 131072)
        val lms = (1 << lbt) - 1
        val dms = (1 << dbt) - 1
        val lmArr = lm.asInstanceOf[Array[Char]]
        val dmArr = dm.asInstanceOf[Array[Char]]
        var lpos = pos
        var symDone = false
        while !symDone do
          lpos = pos
          val c = lmArr(bits16(dat, pos) & lms).toInt
          val sym = c >> 4
          pos += c & 15
          if pos > tbts then
            if noSt != 0 then throw new FlateError(0, "unexpected EOF")
            symDone = true
          else if c == 0 then
            throw new FlateError(2, "invalid length/literal")
          else if sym < 256 then
            if bt < buf.length then buf(bt) = sym.toByte
            bt += 1
          else if sym == 256 then
            lpos = pos
            lm = null
            symDone = true
          else
            var add = sym - 254
            if sym > 264 then
              val idx = sym - 257
              val b = fleb(idx)
              add = bits(dat, pos, (1 << b) - 1) + fl(idx)
              pos += b
            val d = dmArr(bits16(dat, pos) & dms).toInt
            val dsym = d >> 4
            if d == 0 then throw new FlateError(3, "invalid distance")
            pos += d & 15
            val validDist = dsym < 31
            var dt = if validDist then fd(dsym) else 0
            if validDist && dsym > 3 then
              val b = fdeb(dsym)
              dt += bits16(dat, pos) & ((1 << b) - 1)
              pos += b
            if pos > tbts then
              if noSt != 0 then throw new FlateError(0, "unexpected EOF")
              symDone = true
            else
              if resize then cbuf(bt + 131072)
              val end = bt + add
              if !validDist then
                while bt < end do
                  if bt < buf.length then buf(bt) = 0
                  bt += 1
              else if bt >= dt && end <= buf.length then
                val outBuf = buf
                while bt < end do
                  outBuf(bt) = outBuf(bt - dt)
                  bt += 1
              else
                if bt < dt then
                  val shift = dl - dt
                  val dend = if dt < end then dt else end
                  if shift + bt < 0 then throw new FlateError(3, "invalid distance")
                  val dictArr = dict0.asInstanceOf[Array[Byte]]
                  while bt < dend do
                    if bt < buf.length then buf(bt) = dictArr(shift + bt)
                    bt += 1
                while bt < end do
                  val srcIdx = bt - dt
                  val v: Byte = if srcIdx >= 0 && srcIdx < buf.length then buf(srcIdx) else 0
                  if bt < buf.length then buf(bt) = v
                  bt += 1

        st.l = lm
        st.p = lpos
        st.b = bt
        st.f = isFinal
        if lm != null then
          isFinal = 1
          st.m = lbt
          st.d = dm
          st.n = dbt
        if isFinal != 0 then outerDone = true

    val retLen = if bt < buf.length then bt else buf.length
    if noBuf && retLen != buf.length then
      val trimmed = Tables.sliceBytes(buf, 0, retLen)
      new InflateResult(trimmed, retLen)
    else
      new InflateResult(buf, retLen)

  def inflt(
      dat: Array[Byte],
      st: InflateState,
      buf0: Array[Byte] | Null,
      dict0: Array[Byte] | Null,
  ): Uint8Array =
    val res = infltRaw(dat, st, buf0, dict0)
    if res.len == res.buf.length then Tables.bytesToU8(res.buf)
    else Tables.bytesSliceToU8(res.buf, 0, res.len)

  def inflateSync(data: Array[Byte], opts: DecompressOptions): Uint8Array =
    inflt(data, new InflateState(i = 2), opts.out, opts.dictionary)
