package fflate

internal class HTreeResult(val t: IntArray, val l: Int)

internal class LcResult(val c: IntArray, val len: Int, val n: Int)

private fun calcDepths(
    idx: Int,
    nodeSym: IntArray,
    nodeLeft: IntArray,
    nodeRight: IntArray,
    tr: IntArray,
    depth: Int,
): Int {
    val sym = nodeSym[idx]
    if (sym == -1) {
        val l = calcDepths(nodeLeft[idx], nodeSym, nodeLeft, nodeRight, tr, depth + 1)
        val r = calcDepths(nodeRight[idx], nodeSym, nodeLeft, nodeRight, tr, depth + 1)
        return if (l > r) l else r
    }
    tr[sym] = depth
    return depth
}

internal fun hTree(d: IntArray, mb: Int): HTreeResult {
    var s = 0
    for (i in 0 until d.size) {
        if (d[i] != 0) s++
    }
    if (s == 0) return HTreeResult(EMPTY_INTS, 0)

    val nodeCount = s * 2
    val nodeSym = IntArray(nodeCount)
    val nodeFreq = IntArray(nodeCount)
    val t2Sym = IntArray(s)
    val t2Freq = IntArray(s)
    var k = 0
    for (i in 0 until d.size) {
        val f = d[i]
        if (f != 0) {
            nodeSym[k] = i
            nodeFreq[k] = f
            t2Sym[k] = i
            t2Freq[k] = f
            k++
        }
    }
    if (s == 1) {
        val sym = t2Sym[0]
        val v = IntArray(sym + 1)
        v[sym] = 1
        return HTreeResult(v, 1)
    }

    // Stable insertion sort on leaves by ascending frequency (ties keep ascending symbol).
    for (i in 1 until s) {
        val kf = nodeFreq[i]
        val ks = nodeSym[i]
        var j = i - 1
        while (j >= 0 && nodeFreq[j] > kf) {
            nodeFreq[j + 1] = nodeFreq[j]
            nodeSym[j + 1] = nodeSym[j]
            j--
        }
        nodeFreq[j + 1] = kf
        nodeSym[j + 1] = ks
    }

    // Sentinel at index s.
    nodeSym[s] = -1
    nodeFreq[s] = 25001

    val nodeLeft = IntArray(nodeCount)
    val nodeRight = IntArray(nodeCount)
    val intBase = s + 1
    var i0 = intBase
    var i1 = intBase
    var i2 = 2

    nodeSym[i1] = -1
    nodeFreq[i1] = nodeFreq[0] + nodeFreq[1]
    nodeLeft[i1] = 0
    nodeRight[i1] = 1
    i1++

    val endI1 = intBase + s - 1
    while (i1 != endI1) {
        val lIdx = if (nodeFreq[i0] < nodeFreq[i2]) i0++ else i2++
        val rIdx = if (i0 != i1 && nodeFreq[i0] < nodeFreq[i2]) i0++ else i2++
        nodeSym[i1] = -1
        nodeFreq[i1] = nodeFreq[lIdx] + nodeFreq[rIdx]
        nodeLeft[i1] = lIdx
        nodeRight[i1] = rIdx
        i1++
    }

    val maxSym = t2Sym[s - 1]
    val tr = IntArray(maxSym + 1)
    var mbt = calcDepths(i1 - 1, nodeSym, nodeLeft, nodeRight, tr, 0)

    if (mbt > mb) {
        // Stable insertion sort t2 by descending depth, then ascending frequency, then ascending symbol.
        for (idx in 1 until s) {
            val ks = t2Sym[idx]
            val kf = t2Freq[idx]
            val kd = tr[ks]
            var j = idx - 1
            while (j >= 0) {
                val jd = tr[t2Sym[j]]
                if (jd < kd || (jd == kd && t2Freq[j] > kf)) {
                    t2Sym[j + 1] = t2Sym[j]
                    t2Freq[j + 1] = t2Freq[j]
                    j--
                } else {
                    break
                }
            }
            t2Sym[j + 1] = ks
            t2Freq[j + 1] = kf
        }

        var i = 0
        var dt = 0
        val lft = mbt - mb
        val cst = 1 shl lft
        while (i < s) {
            val sym = t2Sym[i]
            val depth = tr[sym]
            if (depth > mb) {
                dt += cst - (1 shl (mbt - depth))
                tr[sym] = mb
                i++
            } else {
                break
            }
        }
        dt = dt shr lft
        while (dt > 0) {
            val sym = t2Sym[i]
            val depth = tr[sym]
            if (depth < mb) {
                tr[sym] = depth + 1
                dt -= 1 shl (mb - depth - 1)
            } else {
                i++
            }
        }
        while (i >= 0 && dt != 0) {
            val sym = t2Sym[i]
            if (tr[sym] == mb) {
                tr[sym] = mb - 1
                dt++
            }
            i--
        }
        mbt = mb
    }

    return HTreeResult(tr, mbt)
}

internal fun lc(c: IntArray): LcResult {
    var s = c.size
    while (s > 0 && c[--s] == 0) {
        // trim trailing zeros
    }
    s++
    val cl = IntArray(s)
    var cli = 0
    var cln = if (c.isNotEmpty()) c[0] else 0
    var cls = 1
    for (i in 1..s) {
        val ci = if (i < c.size) c[i] else 0
        if (i != s && ci == cln) {
            cls++
        } else {
            if (cln == 0 && cls > 2) {
                while (cls > 138) {
                    cl[cli++] = 32754
                    cls -= 138
                }
                if (cls > 2) {
                    cl[cli++] = if (cls > 10) ((cls - 11) shl 5) or 28690 else ((cls - 3) shl 5) or 12305
                    cls = 0
                }
            } else if (cls > 3) {
                cl[cli++] = cln
                cls--
                while (cls > 6) {
                    cl[cli++] = 8304
                    cls -= 6
                }
                if (cls > 2) {
                    cl[cli++] = ((cls - 3) shl 5) or 8208
                    cls = 0
                }
            }
            while (cls > 0) {
                cl[cli++] = cln
                cls--
            }
            cls = 1
            cln = ci
        }
    }
    return LcResult(cl, cli, s)
}

internal fun clen(cf: IntArray, cl: IntArray): Int {
    var l = 0
    for (i in 0 until cl.size) {
        l += cf[i] * cl[i]
    }
    return l
}
