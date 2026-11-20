package fflate

internal class CompressOptions(
    val level: Int? = null,
    val mem: Int? = null,
    val dictionary: ByteArray? = null,
    val mtime: Double? = null,
    val filename: String? = null,
)

internal class DeflateState(
    var l: Int = 0,
    var i: Int = 0,
    var w: Int = 0,
    var z: Int = 0,
    var r: Int = 0,
    var p: ShortArray? = null,
    var h: ShortArray? = null,
)

internal fun wbits(d: ByteArray, base: Int, p: Int, v: Int) {
    val shifted = v shl (p and 7)
    val o = base + (p ushr 3)
    d[o] = (d[o].toInt() or shifted).toByte()
    d[o + 1] = (d[o + 1].toInt() or (shifted ushr 8)).toByte()
}

internal fun wbits16(d: ByteArray, base: Int, p: Int, v: Int) {
    val shifted = v shl (p and 7)
    val o = base + (p ushr 3)
    d[o] = (d[o].toInt() or shifted).toByte()
    d[o + 1] = (d[o + 1].toInt() or (shifted ushr 8)).toByte()
    d[o + 2] = (d[o + 2].toInt() or (shifted ushr 16)).toByte()
}

internal fun wfblk(out: ByteArray, base: Int, pos: Int, dat: ByteArray, start: Int, end: Int): Int {
    val s = end - start
    val o = shft(pos + 2)
    val dst = base + o
    out[dst] = s.toByte()
    out[dst + 1] = (s ushr 8).toByte()
    out[dst + 2] = (s.inv()).toByte()
    out[dst + 3] = ((s ushr 8).inv()).toByte()
    if (s > 0) dat.copyInto(out, dst + 4, start, end)
    return (o + 4 + s) * 8
}

private fun wblk(
    dat: ByteArray,
    out: ByteArray,
    base: Int,
    final: Int,
    syms: IntArray,
    lf: IntArray,
    df: IntArray,
    eb: Int,
    li: Int,
    bs: Int,
    bl: Int,
    pInit: Int,
): Int {
    var p = pInit
    wbits(out, base, p++, final)
    lf[256]++
    val ltTree = hTree(lf, 15)
    val dlt = ltTree.t
    val mlb = ltTree.l
    val dtTree = hTree(df, 15)
    val ddt = dtTree.t
    val mdb = dtTree.l
    val lcltRes = lc(dlt)
    val lclt = lcltRes.c
    val lcltLen = lcltRes.len
    val nlc = lcltRes.n
    val lcdtRes = lc(ddt)
    val lcdt = lcdtRes.c
    val lcdtLen = lcdtRes.len
    val ndc = lcdtRes.n

    val lcfreq = IntArray(19)
    for (i in 0 until lcltLen) lcfreq[lclt[i] and 31]++
    for (i in 0 until lcdtLen) lcfreq[lcdt[i] and 31]++
    val lctTree = hTree(lcfreq, 7)
    val lct = lctTree.t
    val mlcb = lctTree.l

    var nlcc = 19
    while (nlcc > 4) {
        val sym = CLIM[nlcc - 1]
        if (sym < lct.size && lct[sym] != 0) break
        nlcc--
    }

    val flen = (bl + 5) shl 3
    val ftlen = clen(lf, FLT) + clen(df, FDT) + eb
    val dtlen = clen(lf, dlt) + clen(df, ddt) + eb + 14 + 3 * nlcc +
        clen(lcfreq, lct) + 2 * lcfreq[16] + 3 * lcfreq[17] + 7 * lcfreq[18]

    if (bs >= 0 && flen <= ftlen && flen <= dtlen) {
        return wfblk(out, base, p, dat, bs, bs + bl)
    }

    val isDynamic = dtlen < ftlen
    wbits(out, base, p, if (isDynamic) 2 else 1)
    p += 2

    val lm: IntArray
    val ll: IntArray
    val dm: IntArray
    val dl: IntArray
    if (isDynamic) {
        lm = hMap(dlt, mlb, false)
        ll = dlt
        dm = hMap(ddt, mdb, false)
        dl = ddt
        val llm = hMap(lct, mlcb, false)
        wbits(out, base, p, nlc - 257)
        wbits(out, base, p + 5, ndc - 1)
        wbits(out, base, p + 10, nlcc - 4)
        p += 14
        for (i in 0 until nlcc) {
            val sym = CLIM[i]
            wbits(out, base, p + 3 * i, if (sym < lct.size) lct[sym] else 0)
        }
        p += 3 * nlcc
        for (it in 0 until 2) {
            val clct = if (it == 0) lclt else lcdt
            val clctLen = if (it == 0) lcltLen else lcdtLen
            for (i in 0 until clctLen) {
                val entry = clct[i]
                val len = entry and 31
                wbits(out, base, p, llm[len])
                p += lct[len]
                if (len > 15) {
                    wbits(out, base, p, (entry ushr 5) and 127)
                    p += entry ushr 12
                }
            }
        }
    } else {
        lm = FLM
        ll = FLT
        dm = FDM
        dl = FDT
    }

    val fleb = FLEB
    val fdeb = FDEB
    for (i in 0 until li) {
        val sym = syms[i]
        if (sym > 255) {
            val len = (sym ushr 18) and 31
            wbits16(out, base, p, lm[len + 257])
            p += ll[len + 257]
            if (len > 7) {
                wbits(out, base, p, (sym ushr 23) and 31)
                p += fleb[len]
            }
            val dst = sym and 31
            wbits16(out, base, p, dm[dst])
            p += dl[dst]
            if (dst > 3) {
                wbits16(out, base, p, (sym ushr 5) and 8191)
                p += fdeb[dst]
            }
        } else {
            wbits16(out, base, p, lm[sym])
            p += ll[sym]
        }
    }
    wbits16(out, base, p, lm[256])
    return p + ll[256]
}

internal fun dflt(
    dat: ByteArray,
    lvl: Int,
    plvl: Int,
    pre: Int,
    post: Int,
    st: DeflateState,
): ByteArray {
    val s = if (st.z != 0) st.z else dat.size
    val blocks = (s + 6999) / 7000
    // Allocate +2 slack bytes inside the write area so wbits16 never writes past the buffer end.
    val o = ByteArray(pre + s + 5 * (1 + blocks) + post + 2)
    val lst = st.l
    var pos = st.r and 7

    if (lvl != 0) {
        if (pos != 0) o[pre] = (st.r ushr 3).toByte()
        val opt = DEO[lvl - 1]
        val n = opt ushr 13
        val c = opt and 8191
        val msk = (1 shl plvl) - 1
        val prev = st.p ?: ShortArray(32768)
        val head = st.h ?: ShortArray(msk + 1)
        val bs1 = (plvl + 2) / 3
        val bs2 = bs1 shl 1
        val syms = IntArray(25000)
        val lf = IntArray(288)
        val df = IntArray(32)
        val revfl = REVFL
        val revfd = REVFD
        val fleb = FLEB
        val fdeb = FDEB
        var lcCount = 0
        var eb = 0
        var i = st.i
        var li = 0
        var wi = st.w
        var bs = 0

        while (i + 2 < s) {
            val hv = ((dat[i].toInt() and 0xFF) xor
                ((dat[i + 1].toInt() and 0xFF) shl bs1) xor
                ((dat[i + 2].toInt() and 0xFF) shl bs2)) and msk
            var imod = i and 32767
            var pimod = head[hv].toInt()
            prev[imod] = pimod.toShort()
            head[hv] = imod.toShort()

            if (wi <= i) {
                val rem = s - i
                if ((lcCount > 7000 || li > 24576) && (rem > 423 || lst == 0)) {
                    pos = wblk(dat, o, pre, 0, syms, lf, df, eb, li, bs, i - bs, pos)
                    li = 0
                    lcCount = 0
                    eb = 0
                    bs = i
                    for (j in 0 until 286) lf[j] = 0
                    for (j in 0 until 30) df[j] = 0
                }
                var l = 2
                var d = 0
                var ch = c
                var dif = (imod - pimod) and 32767
                if (rem > 2 && i >= dif) {
                    val idx = i - dif
                    val hvPrev = ((dat[idx].toInt() and 0xFF) xor
                        ((dat[idx + 1].toInt() and 0xFF) shl bs1) xor
                        ((dat[idx + 2].toInt() and 0xFF) shl bs2)) and msk
                    if (hv == hvPrev) {
                        val maxn = (if (n < rem) n else rem) - 1
                        val maxd = if (32767 < i) 32767 else i
                        val ml = if (258 < rem) 258 else rem
                        while (dif <= maxd && --ch != 0 && imod != pimod) {
                            if (dat[i + l] == dat[i + l - dif]) {
                                var nl = 0
                                while (nl < ml && dat[i + nl] == dat[i + nl - dif]) nl++
                                if (nl > l) {
                                    l = nl
                                    d = dif
                                    if (nl > maxn) break
                                    val mmd = if (dif < nl - 2) dif else nl - 2
                                    var md = 0
                                    for (j in 0 until mmd) {
                                        val ti = (i - dif + j) and 32767
                                        val pti = prev[ti].toInt()
                                        val cd = (ti - pti) and 32767
                                        if (cd > md) {
                                            md = cd
                                            pimod = ti
                                        }
                                    }
                                }
                            }
                            imod = pimod
                            pimod = prev[imod].toInt()
                            dif += (imod - pimod) and 32767
                        }
                    }
                }
                if (d != 0) {
                    val rfl = revfl[l]
                    val rfd = revfd[d]
                    syms[li++] = 268435456 or (rfl shl 18) or rfd
                    val lin = rfl and 31
                    val din = rfd and 31
                    eb += fleb[lin] + fdeb[din]
                    lf[257 + lin]++
                    df[din]++
                    wi = i + l
                    lcCount++
                } else {
                    val b = dat[i].toInt() and 0xFF
                    syms[li++] = b
                    lf[b]++
                }
            }
            i++
        }

        if (wi > i) i = wi
        while (i < s) {
            val b = dat[i].toInt() and 0xFF
            syms[li++] = b
            lf[b]++
            i++
        }

        pos = wblk(dat, o, pre, lst, syms, lf, df, eb, li, bs, i - bs, pos)
        if (lst == 0) {
            st.r = (pos and 7) or ((o[pre + (pos ushr 3)].toInt() and 0xFF) shl 3)
            pos -= 7
            st.h = head
            st.p = prev
            st.i = i
            st.w = wi
        }
    } else {
        var i = st.w
        while (i < s + lst) {
            var e = i + 65535
            if (e >= s) {
                o[pre + (pos ushr 3)] = lst.toByte()
                e = s
            }
            pos = wfblk(o, pre, pos + 1, dat, i, e)
            i += 65535
        }
        st.i = s
    }

    val outLen = pre + shft(pos) + post
    return o.copyOf(outLen)
}

internal fun dopt(
    dat: ByteArray,
    opt: CompressOptions,
    pre: Int,
    post: Int,
    st: DeflateState? = null,
): ByteArray {
    var data = dat
    val state: DeflateState
    if (st == null) {
        state = DeflateState(l = 1)
        val rawDict = opt.dictionary
        if (rawDict != null) {
            val dictStart = if (rawDict.size > 32768) rawDict.size - 32768 else 0
            val dictLen = rawDict.size - dictStart
            val newDat = ByteArray(dictLen + dat.size)
            if (dictLen > 0) rawDict.copyInto(newDat, 0, dictStart, rawDict.size)
            if (dat.isNotEmpty()) dat.copyInto(newDat, dictLen, 0, dat.size)
            data = newDat
            state.w = dictLen
        }
    } else {
        state = st
    }
    val lvl = opt.level ?: 6
    val plvl = if (opt.mem != null) {
        12 + opt.mem
    } else if (state.l == 0) {
        20
    } else {
        defaultPlvl(data.size)
    }
    return dflt(data, lvl, plvl, pre, post, state)
}

internal fun deflateSync(data: ByteArray, opts: CompressOptions?): ByteArray =
    dopt(data, opts ?: CompressOptions(), 0, 0)
