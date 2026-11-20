package fflate

internal class DecompressOptions(
    val dictionary: ByteArray? = null,
    val out: ByteArray? = null,
)

internal class InflateState(
    var i: Int = 0,
    var f: Int = 0,
    var p: Int = 0,
    var b: Int = 0,
    var l: IntArray? = null,
    var d: IntArray? = null,
    var m: Int = 0,
    var n: Int = 0,
    var outLen: Int = 0,
)

private fun bits(d: ByteArray, datStart: Int, datEnd: Int, p: Int, m: Int): Int {
    val o = datStart + (p ushr 3)
    val b0 = if (o < datEnd) d[o].toInt() and 0xFF else 0
    val b1 = if (o + 1 < datEnd) d[o + 1].toInt() and 0xFF else 0
    return ((b0 or (b1 shl 8)) ushr (p and 7)) and m
}

private fun bits16(d: ByteArray, datStart: Int, datEnd: Int, p: Int): Int {
    val o = datStart + (p ushr 3)
    val b0 = if (o < datEnd) d[o].toInt() and 0xFF else 0
    val b1 = if (o + 1 < datEnd) d[o + 1].toInt() and 0xFF else 0
    val b2 = if (o + 2 < datEnd) d[o + 2].toInt() and 0xFF else 0
    return (b0 or (b1 shl 8) or (b2 shl 16)) ushr (p and 7)
}

internal fun infltRaw(
    dat: ByteArray,
    datStart: Int,
    datEnd: Int,
    st: InflateState,
    buf: ByteArray?,
    dict: ByteArray?,
): ByteArray {
    val sl = datEnd - datStart
    val dl = dict?.size ?: 0
    if (sl <= 0 || (st.f != 0 && st.l == null)) {
        val res = buf ?: EMPTY_BYTES
        st.outLen = res.size
        return res
    }
    val noBuf = buf == null
    val resize = noBuf || st.i != 2
    val noSt = st.i
    var out = buf ?: ByteArray(sl * 3)

    var final = st.f
    var pos = st.p
    var bt = st.b
    var lm = st.l
    var dm = st.d
    var lbt = st.m
    var dbt = st.n
    val tbts = sl * 8
    val fleb = FLEB
    val fdeb = FDEB
    val fl = FL
    val fd = FD

    do {
        if (lm == null) {
            final = bits(dat, datStart, datEnd, pos, 1)
            val type = bits(dat, datStart, datEnd, pos + 1, 3)
            pos += 3
            if (type == 0) {
                val s = shft(pos) + 4
                val hdrOff = datStart + s - 4
                val b0 = if (hdrOff < datEnd) dat[hdrOff].toInt() and 0xFF else 0
                val b1 = if (hdrOff + 1 < datEnd) dat[hdrOff + 1].toInt() and 0xFF else 0
                val l = b0 or (b1 shl 8)
                val t = s + l
                if (t > sl) {
                    if (noSt != 0) throw flateErr(0)
                    break
                }
                if (resize) {
                    val need = bt + l
                    if (need > out.size) {
                        val nextSize = maxOf(out.size * 2, need)
                        val nbuf = ByteArray(nextSize)
                        out.copyInto(nbuf, 0, 0, out.size)
                        out = nbuf
                    }
                } else if (bt + l > out.size) {
                    throw IndexOutOfBoundsException("Offset is outside the bounds of the TypedArray")
                }
                if (l > 0) dat.copyInto(out, bt, datStart + s, datStart + t)
                bt += l
                st.b = bt
                pos = t * 8
                st.p = pos
                st.f = final
                continue
            } else if (type == 1) {
                lm = FLRM
                dm = FDRM
                lbt = 9
                dbt = 5
            } else if (type == 2) {
                val hLit = bits(dat, datStart, datEnd, pos, 31) + 257
                val hcLen = bits(dat, datStart, datEnd, pos + 10, 15) + 4
                val tl = hLit + bits(dat, datStart, datEnd, pos + 5, 31) + 1
                pos += 14
                val ldt = IntArray(tl)
                val clt = IntArray(19)
                for (i in 0 until hcLen) {
                    clt[CLIM[i]] = bits(dat, datStart, datEnd, pos + i * 3, 7)
                }
                pos += hcLen * 3
                val clb = maxOfArray(clt)
                val clbmsk = (1 shl clb) - 1
                val clm = hMap(clt, clb, true)
                var i = 0
                while (i < tl) {
                    val r = clm[bits(dat, datStart, datEnd, pos, clbmsk)]
                    pos += r and 15
                    val s = r ushr 4
                    if (s < 16) {
                        ldt[i++] = s
                    } else {
                        var c = 0
                        var n = 0
                        if (s == 16) {
                            n = 3 + bits(dat, datStart, datEnd, pos, 3)
                            pos += 2
                            c = if (i > 0) ldt[i - 1] else 0
                        } else if (s == 17) {
                            n = 3 + bits(dat, datStart, datEnd, pos, 7)
                            pos += 3
                        } else if (s == 18) {
                            n = 11 + bits(dat, datStart, datEnd, pos, 127)
                            pos += 7
                        }
                        while (n-- > 0) {
                            if (i < tl) ldt[i] = c
                            i++
                        }
                    }
                }
                val lt = ldt.copyOfRange(0, hLit)
                val dt = ldt.copyOfRange(hLit, tl)
                lbt = maxOfArray(lt)
                dbt = maxOfArray(dt)
                lm = hMap(lt, lbt, true)
                dm = hMap(dt, dbt, true)
            } else {
                throw flateErr(1)
            }
            if (pos > tbts) {
                if (noSt != 0) throw flateErr(0)
                break
            }
        }

        if (resize) {
            val need = bt + 131072
            if (need > out.size) {
                val nextSize = maxOf(out.size * 2, need)
                val nbuf = ByteArray(nextSize)
                out.copyInto(nbuf, 0, 0, out.size)
                out = nbuf
            }
        }

        val lms = (1 shl lbt) - 1
        val dms = (1 shl dbt) - 1
        val lmArr = lm
        val dmArr = dm!!
        var lpos = pos

        while (true) {
            lpos = pos
            val c = lmArr[bits16(dat, datStart, datEnd, pos) and lms]
            val sym = c ushr 4
            pos += c and 15
            if (pos > tbts) {
                if (noSt != 0) throw flateErr(0)
                break
            }
            if (c == 0) throw flateErr(2)
            if (sym < 256) {
                if (bt < out.size) {
                    out[bt] = sym.toByte()
                } else if (resize) {
                    val nextSize = maxOf(out.size * 2, bt + 131072)
                    val nbuf = ByteArray(nextSize)
                    out.copyInto(nbuf, 0, 0, out.size)
                    out = nbuf
                    out[bt] = sym.toByte()
                }
                bt++
            } else if (sym == 256) {
                lpos = pos
                lm = null
                break
            } else {
                var add = sym - 254
                if (sym > 264) {
                    val idx = sym - 257
                    val b = fleb[idx]
                    add = bits(dat, datStart, datEnd, pos, (1 shl b) - 1) + fl[idx]
                    pos += b
                }
                val d = dmArr[bits16(dat, datStart, datEnd, pos) and dms]
                val dsym = d ushr 4
                if (d == 0) throw flateErr(3)
                pos += d and 15
                var dt = if (dsym < fd.size) fd[dsym] else -1
                if (dsym > 3) {
                    val b = fdeb[dsym]
                    if (dt >= 0) dt += bits16(dat, datStart, datEnd, pos) and ((1 shl b) - 1)
                    pos += b
                }
                if (pos > tbts) {
                    if (noSt != 0) throw flateErr(0)
                    break
                }
                if (resize) {
                    val need = bt + 131072
                    if (need > out.size) {
                        val nextSize = maxOf(out.size * 2, need)
                        val nbuf = ByteArray(nextSize)
                        out.copyInto(nbuf, 0, 0, out.size)
                        out = nbuf
                    }
                }
                val end = bt + add
                if (dt < 0) {
                    while (bt < end) {
                        if (bt < out.size) out[bt] = 0
                        bt++
                    }
                } else {
                    if (bt < dt) {
                        val shift = dl - dt
                        val dend = if (dt < end) dt else end
                        if (shift + bt < 0) throw flateErr(3)
                        val dictArr = dict!!
                        while (bt < dend) {
                            if (bt < out.size) out[bt] = dictArr[shift + bt]
                            bt++
                        }
                    }
                    if (end <= out.size) {
                        while (bt < end) {
                            out[bt] = out[bt - dt]
                            bt++
                        }
                    } else {
                        val outLen = out.size
                        while (bt < end) {
                            if (bt < outLen) {
                                val srcIdx = bt - dt
                                out[bt] = if (srcIdx < outLen) out[srcIdx] else 0
                            }
                            bt++
                        }
                    }
                }
            }
        }

        st.l = lm
        st.p = lpos
        st.b = bt
        st.f = final
        if (lm != null) {
            final = 1
            st.m = lbt
            st.d = dm
            st.n = dbt
        }
    } while (final == 0)

    st.outLen = if (bt < out.size) bt else out.size
    return out
}

internal fun infltSync(
    dat: ByteArray,
    datStart: Int,
    datEnd: Int,
    buf: ByteArray?,
    dict: ByteArray?,
): ByteArray {
    val st = InflateState(i = 2)
    val out = infltRaw(dat, datStart, datEnd, st, buf, dict)
    return if (st.outLen == out.size) out else out.copyOf(st.outLen)
}

internal fun inflateSync(data: ByteArray, opts: DecompressOptions?): ByteArray =
    infltSync(data, 0, data.size, opts?.out, opts?.dictionary)
