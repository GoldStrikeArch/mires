// Adapted from earcut 3.2.4. See LICENSE for the upstream ISC license.
import kotlin.math.abs
import kotlin.math.min
import kotlin.math.max

internal class Vertex(val i: Int, val x: Double, val y: Double) {
    var prev: Vertex = this
    var next: Vertex = this
    var prevZ: Vertex = this
    var nextZ: Vertex = this
    var z = 0
    var steiner = false
}

internal class Earcut {
    private val none = Vertex(-1, 0.0, 0.0)
    private var filteredOut = false
    private var indexActive = false
    private var numBlocks = 0
    private var blockBBox = DoubleArray(0)
    private var blockHead = mutableListOf<Vertex>()
    private var blockStop = mutableListOf<Vertex>()

    // Triangulate the outer ring, merging holes before clipping.
    fun earcut(data: Array<Double>, holeIndices: Array<Int>, dim: Int = 2): MutableList<Int> {
        val hasHoles = holeIndices.size > 0
        val outerLen = if (hasHoles) holeIndices[0] * dim else data.size
        var outerNode = linkedList(data, 0, outerLen, dim, true)
        val triangles = mutableListOf<Int>()
        if (outerNode == none || outerNode.next == outerNode.prev) {
            return triangles
        }
        var minX = 0.0
        var minY = 0.0
        var invSize = 0.0
        if (hasHoles) {
            outerNode = eliminateHoles(data, holeIndices, outerNode, dim)
        }
        if (data.size > 80 * dim) {
            minX = data[0]
            minY = data[1]
            var maxX = minX
            var maxY = minY
            for (i in dim until outerLen step dim) {
                val x = data[i]
                val y = data[i + 1]
                if (x < minX) {
                    minX = x
                }
                if (y < minY) {
                    minY = y
                }
                if (x > maxX) {
                    maxX = x
                }
                if (y > maxY) {
                    maxY = y
                }
            }
            invSize = max(maxX - minX, maxY - minY)
            invSize = if (invSize != 0.0) 32767.0 / invSize else 0.0
        }
        earcutLinked(outerNode, triangles, minX, minY, invSize)
        return triangles
    }

    private fun linkedList(data: Array<Double>, start: Int, end: Int, dim: Int, clockwise: Boolean): Vertex {
        var last = none
        if (clockwise == signedArea(data, start, end, dim) > 0) {
            for (i in start until end step dim) {
                last = insertNode((i / dim), data[i], data[i + 1], last)
            }
        } else {
            for (i in end - dim downTo start step dim) {
                last = insertNode((i / dim), data[i], data[i + 1], last)
            }
        }
        if (last != none && samePoint(last, last.next)) {
            removeNode(last)
            last = last.next
        }
        return last
    }

    // Sweep to a fixpoint, or heal only the short window between two bridge vertices.
    private fun filterPoints(start: Vertex, initialEnd: Vertex = start): Vertex {
        var end = initialEnd
        val full = end == start
        var p = start
        var again = false
        do {
            again = false
            if (p != p.next && !p.steiner && (samePoint(p, p.next) || area(p.prev, p, p.next) == 0.0)) {
                if (full || p == end) {
                    end = p.prev
                }
                filteredOut = true
                removeNode(p)
                p = p.prev
                again = true
            } else if (full || p != end) {
                p = p.next
                again = !full
            }
        } while (again || p != end)
        return end
    }

    // On a stalled ring, filter collinear points, cure local crossings, then try a diagonal.
    private fun earcutLinked(initialEar: Vertex, triangles: MutableList<Int>, minX: Double, minY: Double, invSize: Double): Unit {
        var ear = initialEar
        if (invSize != 0.0) {
            indexCurve(ear, minX, minY, invSize)
        }
        var stop = ear
        var cured = false
        while (ear.prev != ear.next) {
            val prev = ear.prev
            val next = ear.next
            if (area(prev, ear, next) < 0 && (if (invSize != 0.0) isEarHashed(ear, minX, minY, invSize) else isEar(ear))) {
                triangles.add(prev.i)
                triangles.add(ear.i)
                triangles.add(next.i)
                removeNode(ear)
                ear = next
                stop = next
                continue
            }
            ear = next
            if (ear == stop) {
                filteredOut = false
                ear = filterPoints(ear)
                if (filteredOut) {
                    stop = ear
                    continue
                }
                if (!cured) {
                    ear = cureLocalIntersections(ear, triangles)
                    stop = ear
                    cured = true
                    continue
                }
                splitEarcut(ear, triangles, minX, minY, invSize)
                break
            }
        }
    }

    private fun isEar(ear: Vertex): Boolean {
        val a = ear.prev
        val b = ear
        val c = ear.next
        val ax = a.x
        val bx = b.x
        val cx = c.x
        val ay = a.y
        val by = b.y
        val cy = c.y
        val x0 = min(min(ax, bx), cx)
        val y0 = min(min(ay, by), cy)
        val x1 = max(max(ax, bx), cx)
        val y1 = max(max(ay, by), cy)
        var p = c.next
        while (p != a) {
            if (p.x >= x0 && p.x <= x1 && p.y >= y0 && p.y <= y1 && !(ax == p.x && ay == p.y) &&
                pointInTriangle(ax, ay, bx, by, cx, cy, p.x, p.y) && area(p.prev, p, p.next) >= 0) {
                return false
            }
            p = p.next
        }
        return true
    }

    private fun isEarHashed(ear: Vertex, minX: Double, minY: Double, invSize: Double): Boolean {
        val a = ear.prev
        val b = ear
        val c = ear.next
        val ax = a.x
        val bx = b.x
        val cx = c.x
        val ay = a.y
        val by = b.y
        val cy = c.y
        val x0 = min(min(ax, bx), cx)
        val y0 = min(min(ay, by), cy)
        val x1 = max(max(ax, bx), cx)
        val y1 = max(max(ay, by), cy)
        val minZ = zOrder(x0, y0, minX, minY, invSize)
        val maxZ = zOrder(x1, y1, minX, minY, invSize)
        var p = ear.prevZ
        while (p != none && p.z >= minZ) {
            if (p.x >= x0 && p.x <= x1 && p.y >= y0 && p.y <= y1 && p != c && !(ax == p.x && ay == p.y) &&
                pointInTriangle(ax, ay, bx, by, cx, cy, p.x, p.y) && area(p.prev, p, p.next) >= 0) {
                return false
            }
            p = p.prevZ
        }
        var n = ear.nextZ
        while (n != none && n.z <= maxZ) {
            if (n.x >= x0 && n.x <= x1 && n.y >= y0 && n.y <= y1 && n != c && !(ax == n.x && ay == n.y) &&
                pointInTriangle(ax, ay, bx, by, cx, cy, n.x, n.y) && area(n.prev, n, n.next) >= 0) {
                return false
            }
            n = n.nextZ
        }
        return true
    }

    private fun cureLocalIntersections(initialStart: Vertex, triangles: MutableList<Int>): Vertex {
        var start = initialStart
        var p = start
        var cured = false
        do {
            val a = p.prev
            val b = p.next.next
            if (intersects(a, p, p.next, b, false) && locallyInside(a, b) && locallyInside(b, a)) {
                triangles.add(a.i)
                triangles.add(p.i)
                triangles.add(b.i)
                removeNode(p)
                removeNode(p.next)
                p = b
                start = b
                cured = true
            }
            p = p.next
        } while (p != start)
        return if (cured) filterPoints(p) else p
    }

    private fun splitEarcut(start: Vertex, triangles: MutableList<Int>, minX: Double, minY: Double, invSize: Double): Unit {
        var a = start
        do {
            var b = a.next.next
            while (b != a.prev) {
                if (a.i != b.i && isValidDiagonal(a, b)) {
                    var c = splitPolygon(a, b)
                    a = filterPoints(a, a.next)
                    c = filterPoints(c, c.next)
                    earcutLinked(a, triangles, minX, minY, invSize)
                    earcutLinked(c, triangles, minX, minY, invSize)
                    return
                }
                b = b.next
            }
            a = a.next
        } while (a != start)
    }

    // Bridge holes in stable leftmost/slope order and heal both sides of each splice.
    private fun eliminateHoles(data: Array<Double>, holeIndices: Array<Int>, initialOuterNode: Vertex, dim: Int): Vertex {
        var outerNode = initialOuterNode
        val queue = mutableListOf<Vertex>()
        for (i in 0 until holeIndices.size) {
            val start = holeIndices[i] * dim
            val end = if (i < holeIndices.size - 1) holeIndices[i + 1] * dim else data.size
            val list = linkedList(data, start, end, dim, false)
            if (list == list.next) {
                list.steiner = true
            }
            queue.add(getLeftmost(list))
        }
        queue.sortWith { a, b ->
            val d = compareXYSlope(a, b)
            if (d < 0) -1 else if (d > 0) 1 else 0
        }
        buildBlockIndex((data.size / dim), holeIndices.size)
        indexSegment(outerNode, outerNode)
        indexActive = true
        for (i in 0 until queue.size) {
            outerNode = eliminateHole(queue[i], outerNode)
        }
        indexActive = false
        return filterPoints(outerNode)
    }

    private fun compareXYSlope(a: Vertex, b: Vertex): Double {
        if (a.x != b.x) {
            return a.x - b.x
        }
        if (a.y != b.y) {
            return a.y - b.y
        }
        return (a.next.y - a.y) / (a.next.x - a.x) - (b.next.y - b.y) / (b.next.x - b.x)
    }

    private fun eliminateHole(hole: Vertex, outerNode: Vertex): Vertex {
        val bridge = findHoleBridge(hole, outerNode)
        if (bridge == none) {
            return outerNode
        }
        val bridgeReverse = splitPolygon(bridge, hole)
        val bridge2 = bridgeReverse.next
        indexSegment(bridge, bridge2.next)
        filterPoints(bridgeReverse, bridgeReverse.next)
        return filterPoints(bridge, bridge.next)
    }

    // Each block bounds 16 consecutive ring edges. Merged holes append new segments.
    private fun buildBlockIndex(maxNodes: Int, numHoles: Int): Unit {
        val blocks = (maxNodes + 2 * numHoles + 15) / 16 + numHoles + 2
        blockBBox = DoubleArray(blocks * 4)
        blockHead = MutableList(blocks) { none }
        blockStop = MutableList(blocks) { none }
        numBlocks = 0
    }

    private fun indexSegment(head: Vertex, stop: Vertex): Unit {
        var p = head
        do {
            val b = numBlocks
            numBlocks += 1
            blockHead[b] = p
            var minX = p.x
            var minY = p.y
            var maxX = p.x
            var maxY = p.y
            var k = 0
            do {
                val c = p.next
                p.z = b
                if (c.x < minX) {
                    minX = c.x
                }
                if (c.x > maxX) {
                    maxX = c.x
                }
                if (c.y < minY) {
                    minY = c.y
                }
                if (c.y > maxY) {
                    maxY = c.y
                }
                p = c
                k += 1
            } while (k < 16 && p != stop)
            blockStop[b] = p
            val g = b * 4
            blockBBox[g] = minX
            blockBBox[g + 1] = minY
            blockBBox[g + 2] = maxX
            blockBBox[g + 3] = maxY
        } while (p != stop)
    }

    // A healed edge can extend the old block bounds when its endpoint lived in another block.
    private fun growBlock(head: Vertex, tail: Vertex): Unit {
        val g = head.z * 4
        if (tail.x < blockBBox[g]) {
            blockBBox[g] = tail.x
        }
        if (tail.y < blockBBox[g + 1]) {
            blockBBox[g + 1] = tail.y
        }
        if (tail.x > blockBBox[g + 2]) {
            blockBBox[g + 2] = tail.x
        }
        if (tail.y > blockBBox[g + 3]) {
            blockBBox[g + 3] = tail.y
        }
    }

    private fun liveBlockStop(b: Int): Vertex {
        var stop = blockStop[b]
        while (stop.prev.next != stop) {
            stop = stop.next
        }
        blockStop[b] = stop
        return stop
    }

    private fun liveBlockHead(b: Int): Vertex {
        var head = blockHead[b]
        while (head.prev.next != head) {
            head = head.next
        }
        blockHead[b] = head
        return head
    }

    // Cast a horizontal ray to the left and then choose the locally visible minimum-angle bridge.
    private fun findHoleBridge(hole: Vertex, outerNode: Vertex): Vertex {
        var p = outerNode
        val hx = hole.x
        val hy = hole.y
        var qx = -Double.POSITIVE_INFINITY
        var m = none
        if (samePoint(hole, p)) {
            return p
        }
        for (b in 0 until numBlocks) {
            val g = b * 4
            if (hy < blockBBox[g + 1] || hy > blockBBox[g + 3] || blockBBox[g] > hx || blockBBox[g + 2] <= qx) {
                continue
            }
            val stop = liveBlockStop(b)
            p = liveBlockHead(b)
            do {
                if (p.prev.next == p) {
                    if (samePoint(hole, p.next)) {
                        return p.next
                    } else if (hy <= p.y && hy >= p.next.y && p.next.y != p.y) {
                        val x = p.x + (hy - p.y) * (p.next.x - p.x) / (p.next.y - p.y)
                        if (x <= hx && x > qx) {
                            qx = x
                            m = if (p.x < p.next.x) p else p.next
                            if (x == hx) {
                                return m
                            }
                        }
                    }
                }
                p = p.next
            } while (p != stop)
        }
        if (m == none) {
            return none
        }
        val mx = m.x
        val my = m.y
        val tminY = min(hy, my)
        val tmaxY = max(hy, my)
        var tanMin = Double.POSITIVE_INFINITY
        for (b in 0 until numBlocks) {
            val g = b * 4
            if (blockBBox[g + 2] < mx || blockBBox[g] > hx || blockBBox[g + 3] < tminY || blockBBox[g + 1] > tmaxY) {
                continue
            }
            val stop = liveBlockStop(b)
            p = liveBlockHead(b)
            do {
                if (p.prev.next == p && hx >= p.x && p.x >= mx && hx != p.x &&
                    pointInTriangle(if (hy < my) hx else qx, hy, mx, my, if (hy < my) qx else hx, hy, p.x, p.y)) {
                    val tan = abs(hy - p.y) / (hx - p.x)
                    if ((locallyInside(p, hole) || p.y == hy && p.next.y == hy && p.next.x > hx) && (tan < tanMin ||
                        tan == tanMin && (p.x > m.x || p.x == m.x && sectorContainsSector(m, p)))) {
                        m = p
                        tanMin = tan
                    }
                }
                p = p.next
            } while (p != stop)
        }
        return m
    }

    private fun sectorContainsSector(m: Vertex, p: Vertex): Boolean {
        return area(m.prev, m, p.prev) < 0 && area(p.next, m, m.next) < 0
    }

    // Recompute Morton keys after hole merging, then relink the sorted nodes.
    private fun indexCurve(start: Vertex, minX: Double, minY: Double, invSize: Double): Unit {
        val sortArr = mutableListOf<Vertex>()
        var p = start
        var n = 0
        do {
            p.z = zOrder(p.x, p.y, minX, minY, invSize)
            sortArr.add(p)
            n += 1
            p = p.next
        } while (p != start)
        sortNodes(sortArr, n)
        var prev = none
        for (i in 0 until n) {
            val node = sortArr[i]
            node.prevZ = prev
            if (prev != none) {
                prev.nextZ = node
            }
            prev = node
        }
        prev.nextZ = none
    }

    // Use stable insertion sort for short rings and four stable radix passes for larger rings.
    private fun sortNodes(sortArr: MutableList<Vertex>, n: Int): Unit {
        if (n <= 32) {
            for (i in 1 until n) {
                val node = sortArr[i]
                val z = node.z
                var j = i - 1
                while (j >= 0 && sortArr[j].z > z) {
                    sortArr[j + 1] = sortArr[j]
                    j -= 1
                }
                sortArr[j + 1] = node
            }
            return
        }
        val zArr = IntArray(n)
        val zBuf = IntArray(n)
        val sortBuf = MutableList(n) { none }
        for (i in 0 until n) {
            zArr[i] = sortArr[i].z
        }
        radixPass(n, sortArr, zArr, sortBuf, zBuf, 0)
        radixPass(n, sortBuf, zBuf, sortArr, zArr, 8)
        radixPass(n, sortArr, zArr, sortBuf, zBuf, 16)
        radixPass(n, sortBuf, zBuf, sortArr, zArr, 24)
    }

    private fun radixPass(n: Int, src: MutableList<Vertex>, srcZ: IntArray, dst: MutableList<Vertex>, dstZ: IntArray, shift: Int): Unit {
        val counts = IntArray(256)
        for (i in 0 until n) {
            counts[(srcZ[i] ushr shift) and 0xff] += 1
        }
        var sum = 0
        for (b in 0 until 256) {
            val c = counts[b]
            counts[b] = sum
            sum += c
        }
        for (i in 0 until n) {
            val z = srcZ[i]
            val bucket = (z ushr shift) and 0xff
            val pos = counts[bucket]
            counts[bucket] += 1
            dst[pos] = src[i]
            dstZ[pos] = z
        }
    }

    private fun zOrder(x: Double, y: Double, minX: Double, minY: Double, invSize: Double): Int {
        var xx = toInt32((x - minX) * invSize)
        var yy = toInt32((y - minY) * invSize)
        xx = (xx or (xx shl 8)) and 0x00FF00FF
        xx = (xx or (xx shl 4)) and 0x0F0F0F0F
        xx = (xx or (xx shl 2)) and 0x33333333
        xx = (xx or (xx shl 1)) and 0x55555555
        yy = (yy or (yy shl 8)) and 0x00FF00FF
        yy = (yy or (yy shl 4)) and 0x0F0F0F0F
        yy = (yy or (yy shl 2)) and 0x33333333
        yy = (yy or (yy shl 1)) and 0x55555555
        return xx or (yy shl 1)
    }

    private fun getLeftmost(start: Vertex): Vertex {
        var p = start
        var leftmost = start
        do {
            if (p.x < leftmost.x || p.x == leftmost.x && p.y < leftmost.y) {
                leftmost = p
            }
            p = p.next
        } while (p != start)
        return leftmost
    }

    private fun pointInTriangle(ax: Double, ay: Double, bx: Double, by: Double, cx: Double, cy: Double, px: Double, py: Double): Boolean {
        return (cx - px) * (ay - py) >= (ax - px) * (cy - py) && (ax - px) * (by - py) >= (bx - px) * (ay - py) &&
            (bx - px) * (cy - py) >= (cx - px) * (by - py)
    }

    private fun isValidDiagonal(a: Vertex, b: Vertex): Boolean {
        val zeroLength = samePoint(a, b) && area(a.prev, a, a.next) > 0 && area(b.prev, b, b.next) > 0
        return a.next.i != b.i && (zeroLength || locallyInside(a, b) && locallyInside(b, a) &&
            (area(a.prev, a, b.prev) != 0.0 || area(a, b.prev, b) != 0.0)) && !intersectsPolygon(a, b) &&
            (zeroLength || middleInside(a, b))
    }

    private fun area(p: Vertex, q: Vertex, r: Vertex): Double {
        return (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y)
    }

    private fun samePoint(p1: Vertex, p2: Vertex): Boolean {
        return p1.x == p2.x && p1.y == p2.y
    }

    private fun intersects(p1: Vertex, q1: Vertex, p2: Vertex, q2: Vertex, includeBoundary: Boolean = true): Boolean {
        val o1 = area(p1, q1, p2)
        val o2 = area(p1, q1, q2)
        val o3 = area(p2, q2, p1)
        val o4 = area(p2, q2, q1)
        if ((o1 > 0 && o2 < 0 || o1 < 0 && o2 > 0) && (o3 > 0 && o4 < 0 || o3 < 0 && o4 > 0)) {
            return true
        }
        if (!includeBoundary) {
            return false
        }
        if (o1 == 0.0 && onSegment(p1, p2, q1)) {
            return true
        }
        if (o2 == 0.0 && onSegment(p1, q2, q1)) {
            return true
        }
        if (o3 == 0.0 && onSegment(p2, p1, q2)) {
            return true
        }
        if (o4 == 0.0 && onSegment(p2, q1, q2)) {
            return true
        }
        return false
    }

    private fun onSegment(p: Vertex, q: Vertex, r: Vertex): Boolean {
        return q.x <= max(p.x, r.x) && q.x >= min(p.x, r.x) && q.y <= max(p.y, r.y) && q.y >= min(p.y, r.y)
    }

    private fun intersectsPolygon(a: Vertex, b: Vertex): Boolean {
        val minX = min(a.x, b.x)
        val maxX = max(a.x, b.x)
        val minY = min(a.y, b.y)
        val maxY = max(a.y, b.y)
        var p = a
        do {
            val n = p.next
            if (p.x > maxX && n.x > maxX || p.x < minX && n.x < minX || p.y > maxY && n.y > maxY || p.y < minY &&
                n.y < minY) {
                p = n
                continue
            }
            if (p.i != a.i && n.i != a.i && p.i != b.i && n.i != b.i && intersects(p, n, a, b)) {
                return true
            }
            p = n
        } while (p != a)
        return false
    }

    private fun locallyInside(a: Vertex, b: Vertex): Boolean {
        return if (area(a.prev, a, a.next) < 0) area(a, b, a.next) >= 0 &&
            area(a, a.prev, b) >= 0 else area(a, b, a.prev) < 0 || area(a, a.next, b) < 0
    }

    private fun middleInside(a: Vertex, b: Vertex): Boolean {
        var p = a
        var inside = false
        val px = (a.x + b.x) / 2
        val py = (a.y + b.y) / 2
        do {
            val n = p.next
            if (p.y > py != n.y > py && px < (n.x - p.x) * (py - p.y) / (n.y - p.y) + p.x) {
                inside = !inside
            }
            p = n
        } while (p != a)
        return inside
    }

    private fun splitPolygon(a: Vertex, b: Vertex): Vertex {
        val a2 = createVertex(a.i, a.x, a.y)
        val b2 = createVertex(b.i, b.x, b.y)
        val an = a.next
        val bp = b.prev
        a.next = b
        b.prev = a
        a2.next = an
        an.prev = a2
        b2.next = a2
        a2.prev = b2
        bp.next = b2
        b2.prev = bp
        return b2
    }

    private fun insertNode(i: Int, x: Double, y: Double, last: Vertex): Vertex {
        val p = createVertex(i, x, y)
        if (last == none) {
            p.prev = p
            p.next = p
        } else {
            p.next = last.next
            p.prev = last
            last.next.prev = p
            last.next = p
        }
        return p
    }

    private fun removeNode(p: Vertex): Unit {
        p.next.prev = p.prev
        p.prev.next = p.next
        if (p.prevZ != none) {
            p.prevZ.nextZ = p.nextZ
        }
        if (p.nextZ != none) {
            p.nextZ.prevZ = p.prevZ
        }
        if (indexActive) {
            growBlock(p.prev, p.next)
        }
    }

    fun deviation(data: Array<Double>, holeIndices: Array<Int>, dim: Int, triangles: MutableList<Int>): Double {
        val hasHoles = holeIndices.size > 0
        val outerLen = if (hasHoles) holeIndices[0] * dim else data.size
        var polygonArea = abs(signedArea(data, 0, outerLen, dim))
        if (hasHoles) {
            for (i in 0 until holeIndices.size) {
                val start = holeIndices[i] * dim
                val end = if (i < holeIndices.size - 1) holeIndices[i + 1] * dim else data.size
                polygonArea -= abs(signedArea(data, start, end, dim))
            }
        }
        var trianglesArea = 0.0
        for (i in 0 until triangles.size step 3) {
            val a = triangles[i] * dim
            val b = triangles[i + 1] * dim
            val c = triangles[i + 2] * dim
            trianglesArea += abs((data[a] - data[c]) * (data[b + 1] - data[a + 1]) - (data[a] - data[b]) * (data[c + 1] - data[a + 1]))
        }
        if (trianglesArea == 0.0) {
            var max = 0.0
            for (i in 0 until data.size step dim) {
                max = max(max(max, abs(data[i])), abs(data[i + 1]))
            }
            return if (abs(polygonArea) <= data.size * max * max * 2.220446049250313e-16) 0.0 else 1.0
        }
        return abs((trianglesArea - polygonArea) / polygonArea)
    }

    private fun signedArea(data: Array<Double>, start: Int, end: Int, dim: Int): Double {
        var sum = 0.0
        var j = end - dim
        for (i in start until end step dim) {
            sum += (data[j] - data[i]) * (data[i + 1] + data[j + 1])
            j = i
        }
        return sum
    }

    // Build half-edge twins and legalize interior edges with Lawson flips. The input triangle list is owned by the caller.
    fun refine(triangles: MutableList<Int>, coords: Array<Double>, dim: Int = 2): Unit {
        val t = triangles
        val n = t.size
        if (n < 6) {
            return
        }
        var size = 1
        while (size < n * 4) {
            size = size shl 1
        }
        val hMask = size - 1
        val hTable = IntArray(size)
        val hStamp = IntArray(size)
        val edgeStack = IntArray(n)
        val edgeStamp = IntArray(n)
        val he = IntArray(n) { -1 }
        var i = 0
        for (e in 0 until n) {
            val a = t[e]
            val b = t[nextHE(e)]
            val lo = if (a < b) a else b
            val hi = if (a < b) b else a
            var h = ((lo * -1640531535) xor (hi * -2048144789)) and hMask
            while (hStamp[h] == 1) {
                val s = hTable[h]
                if (s != -1) {
                    val sa = t[s]
                    val sb = t[nextHE(s)]
                    if (sa == lo && sb == hi || sa == hi && sb == lo) {
                        he[e] = s
                        he[s] = e
                        hTable[h] = -1
                        edgeStamp[s] = 1
                        edgeStack[i] = s
                        i += 1
                        break
                    }
                }
                h = h + 1 and hMask
            }
            if (hStamp[h] != 1) {
                hTable[h] = e
                hStamp[h] = 1
            }
        }
        while (i > 0) {
            i -= 1
            val a = edgeStack[i]
            edgeStamp[a] = 0
            val b = he[a]
            if (b == -1) {
                continue
            }
            val a0 = a - a % 3
            val b0 = b - b % 3
            val ar = a0 + (a + 2) % 3
            val al = a0 + (a + 1) % 3
            val bl = b0 + (b + 2) % 3
            val br = b0 + (b + 1) % 3
            val p0 = t[ar]
            val pr = t[a]
            val pl = t[al]
            val p1 = t[bl]
            val x0 = coords[p0 * dim]
            val y0 = coords[p0 * dim + 1]
            val xr = coords[pr * dim]
            val yr = coords[pr * dim + 1]
            val xl = coords[pl * dim]
            val yl = coords[pl * dim + 1]
            val x1 = coords[p1 * dim]
            val y1 = coords[p1 * dim + 1]
            if (!inCircle(x0, y0, xr, yr, xl, yl, x1, y1) && orient(x0, y0, xr, yr, x1, y1) > 0 &&
                orient(x0, y0, x1, y1, xl, yl) > 0) {
                t[a] = p1
                t[b] = p0
                val hbl = he[bl]
                val har = he[ar]
                he[a] = hbl
                if (hbl != -1) {
                    he[hbl] = a
                }
                he[b] = har
                if (har != -1) {
                    he[har] = b
                }
                he[ar] = bl
                he[bl] = ar
                if (hbl != -1 && edgeStamp[a] == 0) {
                    edgeStamp[a] = 1
                    edgeStack[i] = a
                    i += 1
                }
                if (har != -1 && edgeStamp[b] == 0) {
                    edgeStamp[b] = 1
                    edgeStack[i] = b
                    i += 1
                }
                if (he[al] != -1 && edgeStamp[al] == 0) {
                    edgeStamp[al] = 1
                    edgeStack[i] = al
                    i += 1
                }
                if (he[br] != -1 && edgeStamp[br] == 0) {
                    edgeStamp[br] = 1
                    edgeStack[i] = br
                    i += 1
                }
            }
        }
    }

    private fun orient(ax: Double, ay: Double, bx: Double, by: Double, cx: Double, cy: Double): Double {
        return (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
    }

    // The relative tolerance makes near-cocircular diagonals legal ties and prevents flip cycles.
    private fun inCircle(ax: Double, ay: Double, bx: Double, by: Double, cx: Double, cy: Double, px: Double, py: Double): Boolean {
        val dx = ax - px
        val dy = ay - py
        val ex = bx - px
        val ey = by - py
        val fx = cx - px
        val fy = cy - py
        val ap = dx * dx + dy * dy
        val bp = ex * ex + ey * ey
        val cp = fx * fx + fy * fy
        val s = ap + bp + cp
        return dx * (ey * cp - bp * fy) - dy * (ex * cp - bp * fx) + ap * (ex * fy - ey * fx) <= 1e-13 * s * s
    }

    private fun nextHE(e: Int): Int {
        return e - e % 3 + (e + 1) % 3
    }

    // Z-order uses ECMAScript ToInt32 even for holes far outside the outer bounds.
    private fun toInt32(value: Double): Int {
        if (value >= -2147483648.0 && value < 2147483648.0) return value.toInt()
        val magnitude = abs(value) % 4294967296.0
        val bits = if (magnitude >= 2147483648.0) (magnitude - 2147483648.0).toInt() + Int.MIN_VALUE else magnitude.toInt()
        return if (value < 0) -bits else bits
    }

    private fun createVertex(i: Int, x: Double, y: Double): Vertex {
        val p = Vertex(i, x, y)
        p.prevZ = none
        p.nextZ = none
        return p
    }
}
