package fflate

internal open class DeflateStream(
    opts: CompressOptions? = null,
    var ondata: ((ByteArray, Boolean) -> Unit)? = null,
) {
    protected var o: CompressOptions = opts ?: CompressOptions()
    protected var s: DeflateState = DeflateState(l = 0, i = 32768, w = 32768, z = 32768)
    protected var b: ByteArray = ByteArray(98304)

    init {
        val rawDict = o.dictionary
        if (rawDict != null) {
            val dictStart = if (rawDict.size > 32768) rawDict.size - 32768 else 0
            val dictLen = rawDict.size - dictStart
            if (dictLen > 0) rawDict.copyInto(b, 32768 - dictLen, dictStart, rawDict.size)
            s.i = 32768 - dictLen
        }
    }

    protected open fun p(c: ByteArray, f: Boolean) {
        ondata!!(dopt(c, o, 0, 0, s), f)
    }

    open fun push(chunk: ByteArray, final: Boolean = false) {
        if (ondata == null) throw flateErr(5)
        if (s.l != 0) throw flateErr(4)
        val endLen = chunk.size + s.z
        if (endLen > b.size) {
            if (endLen > 2 * b.size - 32768) {
                val newBuf = ByteArray(endLen and -32768)
                b.copyInto(newBuf, 0, 0, s.z)
                b = newBuf
            }
            val split = b.size - s.z
            if (split > 0) chunk.copyInto(b, s.z, 0, split)
            s.z = b.size
            p(b, false)
            b.copyInto(b, 0, b.size - 32768, b.size)
            if (split < chunk.size) chunk.copyInto(b, 32768, split, chunk.size)
            s.z = chunk.size - split + 32768
            s.i = 32766
            s.w = 32768
        } else {
            if (chunk.isNotEmpty()) chunk.copyInto(b, s.z, 0, chunk.size)
            s.z += chunk.size
        }
        s.l = if (final) 1 else 0
        if (s.z > s.w + 8191 || final) {
            p(b, final)
            s.w = s.i
            s.i -= 2
        }
        if (final) {
            s = DeflateState()
            o = CompressOptions()
            b = EMPTY_BYTES
        }
    }

    open fun flush(sync: Boolean = false) {
        if (ondata == null) throw flateErr(5)
        if (s.l != 0) throw flateErr(4)
        p(b, false)
        s.w = s.i
        s.i -= 2
        if (sync) {
            val c = ByteArray(6)
            c[0] = (s.r ushr 3).toByte()
            val ep = wfblk(c, 0, s.r, EMPTY_BYTES, 0, 0)
            s.r = 0
            ondata!!(c.copyOf(ep ushr 3), false)
        }
    }
}

internal class GzipStream(
    opts: CompressOptions? = null,
    cb: ((ByteArray, Boolean) -> Unit)? = null,
) : DeflateStream(opts, cb) {
    private val c = Crc32()
    private var l: Int = 0
    private var v: Int = 1

    override fun push(chunk: ByteArray, final: Boolean) {
        c.push(chunk)
        l += chunk.size
        super.push(chunk, final)
    }

    override fun p(c: ByteArray, f: Boolean) {
        val raw = dopt(c, o, if (v != 0) gzhl(o) else 0, if (f) 8 else 0, s)
        if (v != 0) {
            gzh(raw, o)
            v = 0
        }
        if (f) {
            wbytes(raw, raw.size - 8, this.c.digest())
            wbytes(raw, raw.size - 4, l)
        }
        ondata!!(raw, f)
    }
}

internal class ZlibStream(
    opts: CompressOptions? = null,
    cb: ((ByteArray, Boolean) -> Unit)? = null,
) : DeflateStream(opts, cb) {
    private val c = Adler32()
    private var v: Int = 1

    override fun push(chunk: ByteArray, final: Boolean) {
        c.push(chunk)
        super.push(chunk, final)
    }

    override fun p(c: ByteArray, f: Boolean) {
        val raw = dopt(c, o, if (v != 0) (if (o.dictionary != null) 6 else 2) else 0, if (f) 4 else 0, s)
        if (v != 0) {
            zlh(raw, o)
            v = 0
        }
        if (f) {
            wbytes(raw, raw.size - 4, this.c.digest())
        }
        ondata!!(raw, f)
    }
}

internal open class InflateStream(
    opts: DecompressOptions? = null,
    var ondata: ((ByteArray, Boolean) -> Unit)? = null,
) {
    protected var s: InflateState
    protected var o: ByteArray = ByteArray(32768)
    protected var p: ByteArray = EMPTY_BYTES
    protected var d: Boolean = false

    init {
        val rawDict = opts?.dictionary
        if (rawDict != null) {
            val dictStart = if (rawDict.size > 32768) rawDict.size - 32768 else 0
            val dictLen = rawDict.size - dictStart
            if (dictLen > 0) rawDict.copyInto(o, 0, dictStart, rawDict.size)
            s = InflateState(i = 0, b = dictLen)
        } else {
            s = InflateState(i = 0, b = 0)
        }
    }

    protected fun e(c: ByteArray) {
        if (ondata == null) throw flateErr(5)
        if (d) throw flateErr(4)
        if (p.isEmpty()) {
            p = c
        } else if (c.isNotEmpty()) {
            val n = ByteArray(p.size + c.size)
            p.copyInto(n, 0, 0, p.size)
            c.copyInto(n, p.size, 0, c.size)
            p = n
        }
    }

    protected fun c(final: Boolean) {
        d = final
        s.i = if (final) 1 else 0
        val bts = s.b
        val dt = infltRaw(p, 0, p.size, s, o, null)
        val dtLen = s.outLen
        val emitEnd = if (s.b < dtLen) s.b else dtLen
        ondata!!(slc(dt, bts, emitEnd), d)
        o = slc(dt, s.b - 32768, dtLen)
        s.b = o.size
        p = slc(p, s.p ushr 3, p.size)
        s.p = s.p and 7
    }

    open fun push(chunk: ByteArray, final: Boolean = false) {
        e(chunk)
        c(final)
    }
}

internal class GunzipStream(
    opts: DecompressOptions? = null,
    cb: ((ByteArray, Boolean) -> Unit)? = null,
) : InflateStream(opts, cb) {
    private var v: Int = 1
    private var r: Int = 0

    override fun push(chunk: ByteArray, final: Boolean) {
        e(chunk)
        r += chunk.size
        if (v != 0) {
            val subStart = v - 1
            val remP = if (subStart < p.size) p.copyOfRange(subStart, p.size) else EMPTY_BYTES
            val headerLen = if (remP.size > 3) gzs(remP) else 4
            if (headerLen > remP.size) {
                if (!final) return
            }
            p = if (headerLen < remP.size) remP.copyOfRange(headerLen, remP.size) else EMPTY_BYTES
            v = 0
        }
        c(false)
        if (s.f != 0 && s.l == null) {
            v = shft(s.p) + 9
            s = InflateState(i = 0)
            o = EMPTY_BYTES
            push(EMPTY_BYTES, final)
        } else if (final) {
            c(true)
        }
    }
}

internal class UnzlibStream(
    opts: DecompressOptions? = null,
    cb: ((ByteArray, Boolean) -> Unit)? = null,
) : InflateStream(opts, cb) {
    private var v: Int = if (opts?.dictionary != null) 2 else 1

    override fun push(chunk: ByteArray, final: Boolean) {
        e(chunk)
        if (v != 0) {
            if (p.size < 6 && !final) return
            val hdr = zls(p, v - 1 != 0)
            p = if (hdr < p.size) p.copyOfRange(hdr, p.size) else EMPTY_BYTES
            v = 0
        }
        if (final) {
            if (p.size < 4) throw flateErr(6, "invalid zlib data")
            p = p.copyOf(p.size - 4)
        }
        c(final)
    }
}

internal class DecompressStream(
    private val opts: DecompressOptions? = null,
    var ondata: ((ByteArray, Boolean) -> Unit)? = null,
) {
    private var s: InflateStream? = null
    private var p: ByteArray? = null

    fun push(chunk: ByteArray, final: Boolean = false) {
        if (ondata == null) throw flateErr(5)
        val inner = s
        if (inner == null) {
            val cur = p
            if (cur != null && cur.isNotEmpty()) {
                val n = ByteArray(cur.size + chunk.size)
                cur.copyInto(n, 0, 0, cur.size)
                chunk.copyInto(n, cur.size, 0, chunk.size)
                p = n
            } else {
                p = chunk
            }
            val buf = p!!
            if (buf.size > 2) {
                val d0 = buf[0].toInt() and 0xFF
                val d1 = buf[1].toInt() and 0xFF
                val d2 = buf[2].toInt() and 0xFF
                val stream: InflateStream =
                    if (d0 == 31 && d1 == 139 && d2 == 8) {
                        GunzipStream(opts, ondata)
                    } else if ((d0 and 15) != 8 || (d0 ushr 4) > 7 || (((d0 shl 8) or d1) % 31) != 0) {
                        InflateStream(opts, ondata)
                    } else {
                        UnzlibStream(opts, ondata)
                    }
                s = stream
                stream.push(buf, final)
                p = null
            }
        } else {
            inner.push(chunk, final)
        }
    }
}
