// DEFLATE compressor: bit writers (`wbits`, `wbits16`), stored/fixed/dynamic block emission
// (`wfblk`, `wblk`), LZ77 hash-chain matcher (`dflt`), option normalization (`dopt`), and
// `deflateSync`. Exact port of fflate 0.8.3.

package fflate

import scala.scalajs.js.typedarray.Uint8Array

final class CompressOptions(
    val hasLevel: Boolean = false,
    val level: Int = 6,
    val hasMem: Boolean = false,
    val mem: Int = 0,
    val dictionary: Array[Byte] | Null = null,
    val hasMtime: Boolean = false,
    val mtime: Double = 0.0,
    val filename: String | Null = null,
)

object CompressOptions:
  val empty: CompressOptions = new CompressOptions()

final class DeflateState(
    var l: Int = 0,
    var i: Int = 0,
    var w: Int = 0,
    var z: Int = 0,
    var r: Int = 0,
    var p: Array[Char] | Null = null,
    var h: Array[Char] | Null = null,
)

object DeflateCore:
  inline def wbits(d: Array[Byte], pre: Int, p: Int, v0: Int): Unit =
    val v = v0 << (p & 7)
    val o = pre + (p >> 3)
    d(o) = ((d(o) & 0xff) | v).toByte
    d(o + 1) = ((d(o + 1) & 0xff) | (v >> 8)).toByte

  inline def wbits16(d: Array[Byte], pre: Int, p: Int, v0: Int): Unit =
    val v = v0 << (p & 7)
    val o = pre + (p >> 3)
    d(o) = ((d(o) & 0xff) | v).toByte
    d(o + 1) = ((d(o + 1) & 0xff) | (v >> 8)).toByte
    d(o + 2) = ((d(o + 2) & 0xff) | (v >> 16)).toByte

  // Writes a stored (type 0) block from dat[datStart .. datStart + datLen) into out at byte offset pre.
  def wfblk(out: Array[Byte], pre: Int, pos: Int, dat: Array[Byte], datStart: Int, datLen: Int): Int =
    val o = Tables.shft(pos + 2)
    val base = pre + o
    val lo = datLen & 255
    val hi = datLen >> 8
    if base < out.length then out(base) = lo.toByte
    if base + 1 < out.length then out(base + 1) = hi.toByte
    if base + 2 < out.length then out(base + 2) = (lo ^ 255).toByte
    if base + 3 < out.length then out(base + 3) = (hi ^ 255).toByte
    if datLen > 0 then System.arraycopy(dat, datStart, out, base + 4, datLen)
    (o + 4 + datLen) * 8

  private def wblk(
      dat: Array[Byte],
      out: Array[Byte],
      pre: Int,
      isFinal: Int,
      syms: Array[Int],
      lf: Array[Char],
      df: Array[Char],
      eb: Int,
      li: Int,
      bs: Int,
      bl: Int,
      p0: Int,
  ): Int =
    var p = p0
    wbits(out, pre, p, isFinal)
    p += 1
    lf(256) = (lf(256).toInt + 1).toChar

    val ltRes = Huffman.hTree(lf, 15)
    val dlt = ltRes.t
    val mlb = ltRes.l
    val dtRes = Huffman.hTree(df, 15)
    val ddt = dtRes.t
    val mdb = dtRes.l
    val lcltRes = Huffman.lc(dlt)
    val lclt = lcltRes.c
    val lcltLen = lcltRes.len
    val nlc = lcltRes.n
    val lcdtRes = Huffman.lc(ddt)
    val lcdt = lcdtRes.c
    val lcdtLen = lcdtRes.len
    val ndc = lcdtRes.n

    val lcfreq = new Array[Char](19)
    var i = 0
    while i < lcltLen do
      val idx = lclt(i).toInt & 31
      lcfreq(idx) = (lcfreq(idx).toInt + 1).toChar
      i += 1
    i = 0
    while i < lcdtLen do
      val idx = lcdt(i).toInt & 31
      lcfreq(idx) = (lcfreq(idx).toInt + 1).toChar
      i += 1

    val lctRes = Huffman.hTree(lcfreq, 7)
    val lct = lctRes.t
    val mlcb = lctRes.l
    val clim = Tables.clim
    var nlcc = 19
    while nlcc > 4 && {
        val ci = clim(nlcc - 1)
        ci >= lct.length || lct(ci) == 0
      } do nlcc -= 1

    val flen = (bl + 5) << 3
    val ftlen = Huffman.clen(lf, Tables.flt) + Huffman.clen(df, Tables.fdt) + eb
    val dtlen =
      Huffman.clen(lf, dlt) + Huffman.clen(df, ddt) + eb + 14 + 3 * nlcc +
        Huffman.clen(lcfreq, lct) + 2 * lcfreq(16).toInt + 3 * lcfreq(17).toInt + 7 * lcfreq(18).toInt

    if bs >= 0 && flen <= ftlen && flen <= dtlen then
      return wfblk(out, pre, p, dat, bs, bl)

    val isDynamic = dtlen < ftlen
    wbits(out, pre, p, if isDynamic then 2 else 1)
    p += 2

    val lm: Array[Char] =
      if isDynamic then
        val dlm = Tables.hMap(dlt, mlb, 0)
        val llm = Tables.hMap(lct, mlcb, 0)
        wbits(out, pre, p, nlc - 257)
        wbits(out, pre, p + 5, ndc - 1)
        wbits(out, pre, p + 10, nlcc - 4)
        p += 14
        i = 0
        while i < nlcc do
          val ci = clim(i)
          val cl = if ci < lct.length then lct(ci) & 0xff else 0
          wbits(out, pre, p + 3 * i, cl)
          i += 1
        p += 3 * nlcc

        var it = 0
        while it < 2 do
          val clct = if it == 0 then lclt else lcdt
          val clctLen = if it == 0 then lcltLen else lcdtLen
          i = 0
          while i < clctLen do
            val v = clct(i).toInt
            val len = v & 31
            wbits(out, pre, p, llm(len).toInt)
            p += lct(len) & 0xff
            if len > 15 then
              wbits(out, pre, p, (v >> 5) & 127)
              p += v >> 12
            i += 1
          it += 1
        dlm
      else Tables.flm

    val ll: Array[Byte] = if isDynamic then dlt else Tables.flt
    val dm: Array[Char] = if isDynamic then Tables.hMap(ddt, mdb, 0) else Tables.fdm
    val dl: Array[Byte] = if isDynamic then ddt else Tables.fdt
    val fleb = Tables.fleb
    val fdeb = Tables.fdeb

    i = 0
    while i < li do
      val sym = syms(i)
      if sym > 255 then
        val len = (sym >> 18) & 31
        wbits16(out, pre, p, lm(len + 257).toInt)
        p += ll(len + 257) & 0xff
        if len > 7 then
          wbits(out, pre, p, (sym >> 23) & 31)
          p += fleb(len)
        val dst = sym & 31
        wbits16(out, pre, p, dm(dst).toInt)
        p += dl(dst) & 0xff
        if dst > 3 then
          wbits16(out, pre, p, (sym >> 5) & 8191)
          p += fdeb(dst)
      else
        wbits16(out, pre, p, lm(sym).toInt)
        p += ll(sym) & 0xff
      i += 1

    wbits16(out, pre, p, lm(256).toInt)
    p + (ll(256) & 0xff)

  def dflt(dat: Array[Byte], lvl: Int, plvl: Int, pre: Int, post: Int, st: DeflateState): Array[Byte] =
    val s = if st.z != 0 then st.z else dat.length
    // +2 slack bytes ensure wbits16 never writes past the end of `o`.
    val o = new Array[Byte](pre + s + 5 * (1 + ((s + 6999) / 7000)) + post + 2)
    val lst = st.l
    var pos = st.r & 7

    if lvl != 0 then
      if pos != 0 then o(pre) = (st.r >> 3).toByte
      val opt = Tables.deo(lvl - 1)
      val n = opt >> 13
      val c = opt & 8191
      val msk = (1 << plvl) - 1
      val prev = if st.p != null then st.p.asInstanceOf[Array[Char]] else new Array[Char](32768)
      val head = if st.h != null then st.h.asInstanceOf[Array[Char]] else new Array[Char](msk + 1)
      val bs1 = (plvl + 2) / 3
      val bs2 = 2 * bs1

      val syms = new Array[Int](25000)
      val lf = new Array[Char](288)
      val df = new Array[Char](32)
      val revfl = Tables.revfl
      val revfd = Tables.revfd
      val fleb = Tables.fleb
      val fdeb = Tables.fdeb

      var lcCnt = 0
      var eb = 0
      var i = st.i
      var li = 0
      var wi = st.w
      var bs = 0

      val limit = s - 2
      while i < limit do
        val hv = ((dat(i) & 0xff) ^ ((dat(i + 1) & 0xff) << bs1) ^ ((dat(i + 2) & 0xff) << bs2)) & msk
        var imod = i & 32767
        var pimod = head(hv).toInt
        prev(imod) = pimod.toChar
        head(hv) = imod.toChar

        if wi <= i then
          val rem = s - i
          if (lcCnt > 7000 || li > 24576) && (rem > 423 || lst == 0) then
            pos = wblk(dat, o, pre, 0, syms, lf, df, eb, li, bs, i - bs, pos)
            li = 0
            lcCnt = 0
            eb = 0
            bs = i
            var j = 0
            while j < 286 do { lf(j) = 0; j += 1 }
            j = 0
            while j < 30 do { df(j) = 0; j += 1 }

          var l = 2
          var d = 0
          var ch = c
          var dif = (imod - pimod) & 32767
          if rem > 2 && i >= dif then
            val prevIdx = i - dif
            val phv =
              ((dat(prevIdx) & 0xff) ^ ((dat(prevIdx + 1) & 0xff) << bs1) ^ ((dat(prevIdx + 2) & 0xff) << bs2)) & msk
            if hv == phv then
              val maxn = (if n < rem then n else rem) - 1
              val maxd = if 32767 < i then 32767 else i
              val ml = if 258 < rem then 258 else rem
              var searching = true
              while searching && dif <= maxd do
                ch -= 1
                if ch == 0 || imod == pimod then searching = false
                else
                  if dat(i + l) == dat(i + l - dif) then
                    var nl = 0
                    while nl < ml && dat(i + nl) == dat(i + nl - dif) do nl += 1
                    if nl > l then
                      l = nl
                      d = dif
                      if nl > maxn then searching = false
                      else
                        val mmd = if dif < nl - 2 then dif else nl - 2
                        var md = 0
                        var j = 0
                        while j < mmd do
                          val ti = (i - dif + j) & 32767
                          val pti = prev(ti).toInt
                          val cd = (ti - pti) & 32767
                          if cd > md then
                            md = cd
                            pimod = ti
                          j += 1
                  if searching then
                    imod = pimod
                    pimod = prev(imod).toInt
                    dif += (imod - pimod) & 32767

          if d != 0 then
            val rfl = revfl(l)
            val rfd = revfd(d)
            syms(li) = 268435456 | (rfl << 18) | rfd
            li += 1
            val lin = rfl & 31
            val din = rfd & 31
            eb += fleb(lin) + fdeb(din)
            val lIdx = 257 + lin
            lf(lIdx) = (lf(lIdx).toInt + 1).toChar
            df(din) = (df(din).toInt + 1).toChar
            wi = i + l
            lcCnt += 1
          else
            val b = dat(i) & 0xff
            syms(li) = b
            li += 1
            lf(b) = (lf(b).toInt + 1).toChar

        i += 1

      if wi > i then i = wi
      while i < s do
        val b = dat(i) & 0xff
        syms(li) = b
        li += 1
        lf(b) = (lf(b).toInt + 1).toChar
        i += 1

      pos = wblk(dat, o, pre, lst, syms, lf, df, eb, li, bs, i - bs, pos)
      if lst == 0 then
        st.r = (pos & 7) | ((o(pre + (pos >> 3)) & 0xff) << 3)
        pos -= 7
        st.h = head
        st.p = prev
        st.i = i
        st.w = wi
    else
      var i = st.w
      val stop = s + lst
      while i < stop do
        var e = i + 65535
        if e >= s then
          o(pre + (pos >> 3)) = lst.toByte
          e = s
        pos = wfblk(o, pre, pos + 1, dat, i, e - i)
        i += 65535
      st.i = s

    Tables.sliceBytes(o, 0, pre + Tables.shft(pos) + post)

  def doptBytes(
      dat0: Array[Byte],
      opt: CompressOptions,
      pre: Int,
      post: Int,
      st0: DeflateState | Null,
  ): Array[Byte] =
    var dat = dat0
    val st: DeflateState =
      if st0 == null then
        val fresh = new DeflateState(l = 1)
        if opt.dictionary != null then
          val rawDict = opt.dictionary.asInstanceOf[Array[Byte]]
          val dLen = if rawDict.length > 32768 then 32768 else rawDict.length
          val newDat = new Array[Byte](dLen + dat.length)
          System.arraycopy(rawDict, rawDict.length - dLen, newDat, 0, dLen)
          System.arraycopy(dat, 0, newDat, dLen, dat.length)
          dat = newDat
          fresh.w = dLen
        fresh
      else st0.asInstanceOf[DeflateState]

    val lvl = if opt.hasLevel then opt.level else 6
    val plvl =
      if opt.hasMem then 12 + opt.mem
      else if st.l != 0 then Tables.defaultPlvl(dat.length)
      else 20

    dflt(dat, lvl, plvl, pre, post, st)

  def deflateSync(data: Array[Byte], opts: CompressOptions): Uint8Array =
    Tables.bytesToU8(doptBytes(data, opts, 0, 0, null))
