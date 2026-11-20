// Huffman tree construction, 15-bit/7-bit Kraft debt length clamping, RLE code-length encoding,
// and bit-cost calculation — exact port of fflate 0.8.3's `hTree`, `ln`, `lc`, and `clen`.

package fflate

final class HTreeResult(val t: Array[Byte], val l: Int)
final class LCResult(val c: Array[Char], val len: Int, val n: Int)

object Huffman:
  private def assignDepths(
      idx: Int,
      nodeSym: Array[Int],
      nodeLeft: Array[Int],
      nodeRight: Array[Int],
      tr: Array[Int],
      depth: Int,
  ): Int =
    val sym = nodeSym(idx)
    if sym == -1 then
      val dl = assignDepths(nodeLeft(idx), nodeSym, nodeLeft, nodeRight, tr, depth + 1)
      val dr = assignDepths(nodeRight(idx), nodeSym, nodeLeft, nodeRight, tr, depth + 1)
      if dl > dr then dl else dr
    else
      tr(sym) = depth
      depth

  def hTree(d: Array[Char], mb: Int): HTreeResult =
    val dLen = d.length
    var s = 0
    var i = 0
    while i < dLen do
      if d(i).toInt != 0 then s += 1
      i += 1

    if s == 0 then return new HTreeResult(Tables.emptyBytes, 0)

    if s == 1 then
      i = 0
      while d(i).toInt == 0 do i += 1
      val v = new Array[Byte](i + 1)
      v(i) = 1
      return new HTreeResult(v, 1)

    // Flat arrays: [0..s-1] leaves, [s] sentinel (freq 25001), [s+1..2*s-1] internal nodes.
    val totalNodes = s * 2
    val nodeSym = new Array[Int](totalNodes)
    val nodeFreq = new Array[Int](totalNodes)
    val nodeLeft = new Array[Int](totalNodes)
    val nodeRight = new Array[Int](totalNodes)
    val t2Sym = new Array[Int](s)
    val t2Freq = new Array[Int](s)

    var k = 0
    i = 0
    while i < dLen do
      val f = d(i).toInt
      if f != 0 then
        nodeSym(k) = i
        nodeFreq(k) = f
        t2Sym(k) = i
        t2Freq(k) = f
        k += 1
      i += 1

    val maxSym = t2Sym(s - 1)

    // Stable insertion sort of leaves [0..s-1] by ascending frequency (ties keep ascending symbol).
    i = 1
    while i < s do
      val kSym = nodeSym(i)
      val kFreq = nodeFreq(i)
      var j = i - 1
      while j >= 0 && nodeFreq(j) > kFreq do
        nodeSym(j + 1) = nodeSym(j)
        nodeFreq(j + 1) = nodeFreq(j)
        j -= 1
      nodeSym(j + 1) = kSym
      nodeFreq(j + 1) = kFreq
      i += 1

    // Sentinel at index s.
    nodeSym(s) = -1
    nodeFreq(s) = 25001

    // UZIP.js two-queue linear merge (strict `<` picks leaf on frequency tie).
    var q0 = s + 1
    var q1 = s + 1
    var q2 = 2
    nodeSym(q1) = -1
    nodeFreq(q1) = nodeFreq(0) + nodeFreq(1)
    nodeLeft(q1) = 0
    nodeRight(q1) = 1
    q1 += 1

    var merged = 1
    while merged != s - 1 do
      val l =
        if nodeFreq(q0) < nodeFreq(q2) then
          val x = q0
          q0 += 1
          x
        else
          val x = q2
          q2 += 1
          x
      val r =
        if q0 != q1 && nodeFreq(q0) < nodeFreq(q2) then
          val x = q0
          q0 += 1
          x
        else
          val x = q2
          q2 += 1
          x
      nodeSym(q1) = -1
      nodeFreq(q1) = nodeFreq(l) + nodeFreq(r)
      nodeLeft(q1) = l
      nodeRight(q1) = r
      q1 += 1
      merged += 1

    val tr = new Array[Int](maxSym + 1)
    var mbt = assignDepths(q1 - 1, nodeSym, nodeLeft, nodeRight, tr, 0)

    if mbt > mb then
      // Stable insertion sort of t2 by descending depth, then ascending frequency, then ascending symbol.
      i = 1
      while i < s do
        val kSym = t2Sym(i)
        val kFreq = t2Freq(i)
        val kDepth = tr(kSym)
        var j = i - 1
        var shift = true
        while j >= 0 && shift do
          val cDepth = tr(t2Sym(j))
          if kDepth > cDepth || (kDepth == cDepth && t2Freq(j) > kFreq) then
            t2Sym(j + 1) = t2Sym(j)
            t2Freq(j + 1) = t2Freq(j)
            j -= 1
          else shift = false
        t2Sym(j + 1) = kSym
        t2Freq(j + 1) = kFreq
        i += 1

      var idx = 0
      var dt = 0
      val lft = mbt - mb
      val cst = 1 << lft
      while idx < s && tr(t2Sym(idx)) > mb do
        val sym = t2Sym(idx)
        dt += cst - (1 << (mbt - tr(sym)))
        tr(sym) = mb
        idx += 1

      dt >>= lft
      while dt > 0 do
        val sym = t2Sym(idx)
        val cur = tr(sym)
        if cur < mb then
          tr(sym) = cur + 1
          dt -= 1 << (mb - cur - 1)
        else idx += 1

      while idx >= 0 && dt != 0 do
        val sym = t2Sym(idx)
        if tr(sym) == mb then
          tr(sym) = mb - 1
          dt += 1
        idx -= 1

      mbt = mb

    val out = new Array[Byte](maxSym + 1)
    i = 0
    while i <= maxSym do
      out(i) = tr(i).toByte
      i += 1
    new HTreeResult(out, mbt)

  // Code-length RLE encoding (`lc`).
  def lc(c: Array[Byte]): LCResult =
    var s = c.length
    while s > 0 && { s -= 1; c(s) == 0 } do ()
    s += 1
    val cl = new Array[Char](s)
    var cli = 0
    var cln = if c.length > 0 then c(0) & 0xff else 0
    var cls = 1
    var i = 1
    while i <= s do
      if i != s && i < c.length && (c(i) & 0xff) == cln then cls += 1
      else
        if cln == 0 && cls > 2 then
          while cls > 138 do
            cl(cli) = 32754.toChar
            cli += 1
            cls -= 138
          if cls > 2 then
            cl(cli) = (if cls > 10 then ((cls - 11) << 5) | 28690 else ((cls - 3) << 5) | 12305).toChar
            cli += 1
            cls = 0
        else if cls > 3 then
          cl(cli) = cln.toChar
          cli += 1
          cls -= 1
          while cls > 6 do
            cl(cli) = 8304.toChar
            cli += 1
            cls -= 6
          if cls > 2 then
            cl(cli) = (((cls - 3) << 5) | 8208).toChar
            cli += 1
            cls = 0
        while cls > 0 do
          cl(cli) = cln.toChar
          cli += 1
          cls -= 1
        cls = 1
        cln = if i < c.length then c(i) & 0xff else 0
      i += 1
    new LCResult(cl, cli, s)

  def clen(cf: Array[Char], cl: Array[Byte]): Int =
    var l = 0
    var i = 0
    val len = cl.length
    while i < len do
      l += cf(i).toInt * (cl(i) & 0xff)
      i += 1
    l
