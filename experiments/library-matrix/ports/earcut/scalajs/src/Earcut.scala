// Adapted from earcut 3.2.4. See LICENSE for the upstream ISC license.
package earcut

import scala.scalajs.js

final class Vertex(val i: Int, val x: Double, val y: Double) {
  var prev: Vertex = this
  var next: Vertex = this
  var prevZ: Vertex = this
  var nextZ: Vertex = this
  var z = 0
  var steiner = false
}

final class Earcut {
  private val none = new Vertex(-1, 0.0, 0.0)
  private var filteredOut = false
  private var indexActive = false
  private var numBlocks = 0
  private var blockBBox = Array.empty[Double]
  private var blockHead = new js.Array[Vertex]()
  private var blockStop = new js.Array[Vertex]()

  // Triangulate the outer ring, merging holes before clipping.
  def earcut(data: js.Array[Double], holeIndices: js.Array[Int], dim: Int = 2): js.Array[Int] = {
    val hasHoles = holeIndices.length > 0
    val outerLen = if (hasHoles) holeIndices(0) * dim else data.length
    var outerNode = linkedList(data, 0, outerLen, dim, true)
    val triangles = new js.Array[Int]()
    if (outerNode == none || outerNode.next == outerNode.prev) {
      return triangles
    }
    var minX = 0.0
    var minY = 0.0
    var invSize = 0.0
    if (hasHoles) {
      outerNode = eliminateHoles(data, holeIndices, outerNode, dim)
    }
    if (data.length > 80 * dim) {
      minX = data(0)
      minY = data(1)
      var maxX = minX
      var maxY = minY
      for (i <- dim until outerLen by dim) {
        val x = data(i)
        val y = data(i + 1)
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
      invSize = math.max(maxX - minX, maxY - minY)
      invSize = if (invSize != 0.0) 32767.0 / invSize else 0.0
    }
    earcutLinked(outerNode, triangles, minX, minY, invSize)
    return triangles
  }

  private def linkedList(data: js.Array[Double], start: Int, end: Int, dim: Int, clockwise: Boolean): Vertex = {
    var last = none
    if (clockwise == signedArea(data, start, end, dim) > 0) {
      for (i <- start until end by dim) {
        last = insertNode((i / dim), data(i), data(i + 1), last)
      }
    } else {
      for (i <- end - dim to start by -dim) {
        last = insertNode((i / dim), data(i), data(i + 1), last)
      }
    }
    if (last != none && samePoint(last, last.next)) {
      removeNode(last)
      last = last.next
    }
    return last
  }

  // Sweep to a fixpoint, or heal only the short window between two bridge vertices.
  private def filterPoints(start: Vertex, initialEnd: Vertex): Vertex = {
    var end = initialEnd
    val full = end == start
    var p = start
    var again = false
    var firstPass = true
    while (firstPass || again || p != end) {
      firstPass = false
      again = false
      if (p != p.next && !p.steiner && (samePoint(p, p.next) || area(p.prev, p, p.next) == 0)) {
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
    }
    return end
  }

  // On a stalled ring, filter collinear points, cure local crossings, then try a diagonal.
  private def earcutLinked(initialEar: Vertex, triangles: js.Array[Int], minX: Double, minY: Double, invSize: Double): Unit = {
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
        triangles.push(prev.i, ear.i, next.i)
        removeNode(ear)
        ear = next
        stop = next
      } else {
        ear = next
        if (ear == stop) {
          filteredOut = false
          ear = filterPoints(ear)
          if (filteredOut) {
            stop = ear
          } else if (!cured) {
            ear = cureLocalIntersections(ear, triangles)
            stop = ear
            cured = true
          } else {
            splitEarcut(ear, triangles, minX, minY, invSize)
            return
          }
        }
      }
    }
  }

  private def isEar(ear: Vertex): Boolean = {
    val a = ear.prev
    val b = ear
    val c = ear.next
    val ax = a.x
    val bx = b.x
    val cx = c.x
    val ay = a.y
    val by = b.y
    val cy = c.y
    val x0 = math.min(math.min(ax, bx), cx)
    val y0 = math.min(math.min(ay, by), cy)
    val x1 = math.max(math.max(ax, bx), cx)
    val y1 = math.max(math.max(ay, by), cy)
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

  private def isEarHashed(ear: Vertex, minX: Double, minY: Double, invSize: Double): Boolean = {
    val a = ear.prev
    val b = ear
    val c = ear.next
    val ax = a.x
    val bx = b.x
    val cx = c.x
    val ay = a.y
    val by = b.y
    val cy = c.y
    val x0 = math.min(math.min(ax, bx), cx)
    val y0 = math.min(math.min(ay, by), cy)
    val x1 = math.max(math.max(ax, bx), cx)
    val y1 = math.max(math.max(ay, by), cy)
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

  private def cureLocalIntersections(initialStart: Vertex, triangles: js.Array[Int]): Vertex = {
    var start = initialStart
    var p = start
    var cured = false
    var firstPass = true
    while (firstPass || p != start) {
      firstPass = false
      val a = p.prev
      val b = p.next.next
      if (intersects(a, p, p.next, b, false) && locallyInside(a, b) && locallyInside(b, a)) {
        triangles.push(a.i, p.i, b.i)
        removeNode(p)
        removeNode(p.next)
        p = b
        start = b
        cured = true
      }
      p = p.next
    }
    return if (cured) filterPoints(p) else p
  }

  private def splitEarcut(start: Vertex, triangles: js.Array[Int], minX: Double, minY: Double, invSize: Double): Unit = {
    var a = start
    var firstPass = true
    while (firstPass || a != start) {
      firstPass = false
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
    }
  }

  // Bridge holes in stable leftmost/slope order and heal both sides of each splice.
  private def eliminateHoles(data: js.Array[Double], holeIndices: js.Array[Int], initialOuterNode: Vertex, dim: Int): Vertex = {
    var outerNode = initialOuterNode
    val queue = new js.Array[Vertex]()
    for (i <- 0 until holeIndices.length) {
      val start = holeIndices(i) * dim
      val end = if (i < holeIndices.length - 1) holeIndices(i + 1) * dim else data.length
      val list = linkedList(data, start, end, dim, false)
      if (list == list.next) {
        list.steiner = true
      }
      queue.push(getLeftmost(list))
    }
    queue.sort { (a, b) =>
      val d = compareXYSlope(a, b)
      if (d < 0) -1 else if (d > 0) 1 else 0
    }
    buildBlockIndex((data.length / dim), holeIndices.length)
    indexSegment(outerNode, outerNode)
    indexActive = true
    for (i <- 0 until queue.length) {
      outerNode = eliminateHole(queue(i), outerNode)
    }
    indexActive = false
    return filterPoints(outerNode)
  }

  private def compareXYSlope(a: Vertex, b: Vertex): Double = {
    if (a.x != b.x) {
      return a.x - b.x
    }
    if (a.y != b.y) {
      return a.y - b.y
    }
    return (a.next.y - a.y) / (a.next.x - a.x) - (b.next.y - b.y) / (b.next.x - b.x)
  }

  private def eliminateHole(hole: Vertex, outerNode: Vertex): Vertex = {
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
  private def buildBlockIndex(maxNodes: Int, numHoles: Int): Unit = {
    val blocks = (maxNodes + 2 * numHoles + 15 / 16) + numHoles + 2
    blockBBox = new Array[Double](blocks * 4)
    blockHead = new js.Array[Vertex](blocks)
    blockStop = new js.Array[Vertex](blocks)
    numBlocks = 0
  }

  private def indexSegment(head: Vertex, stop: Vertex): Unit = {
    var p = head
    var firstPass = true
    while (firstPass || p != stop) {
      firstPass = false
      val b = numBlocks
      numBlocks += 1
      blockHead(b) = p
      var minX = p.x
      var minY = p.y
      var maxX = p.x
      var maxY = p.y
      var k = 0
      var firstStep = true
      while (firstStep || k < 16 && p != stop) {
        firstStep = false
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
      }
      blockStop(b) = p
      val g = b * 4
      blockBBox(g) = minX
      blockBBox(g + 1) = minY
      blockBBox(g + 2) = maxX
      blockBBox(g + 3) = maxY
    }
  }

  // A healed edge can extend the old block bounds when its endpoint lived in another block.
  private def growBlock(head: Vertex, tail: Vertex): Unit = {
    val g = head.z * 4
    if (tail.x < blockBBox(g)) {
      blockBBox(g) = tail.x
    }
    if (tail.y < blockBBox(g + 1)) {
      blockBBox(g + 1) = tail.y
    }
    if (tail.x > blockBBox(g + 2)) {
      blockBBox(g + 2) = tail.x
    }
    if (tail.y > blockBBox(g + 3)) {
      blockBBox(g + 3) = tail.y
    }
  }

  private def liveBlockStop(b: Int): Vertex = {
    var stop = blockStop(b)
    while (stop.prev.next != stop) {
      stop = stop.next
    }
    blockStop(b) = stop
    return stop
  }

  private def liveBlockHead(b: Int): Vertex = {
    var head = blockHead(b)
    while (head.prev.next != head) {
      head = head.next
    }
    blockHead(b) = head
    return head
  }

  // Cast a horizontal ray to the left and then choose the locally visible minimum-angle bridge.
  private def findHoleBridge(hole: Vertex, outerNode: Vertex): Vertex = {
    var p = outerNode
    val hx = hole.x
    val hy = hole.y
    var qx = -Double.PositiveInfinity
    var m = none
    if (samePoint(hole, p)) {
      return p
    }
    locally {
      var b = 0
      while (b < numBlocks) {
        val g = b * 4
        if (!(hy < blockBBox(g + 1) || hy > blockBBox(g + 3) || blockBBox(g) > hx || blockBBox(g + 2) <= qx)) {
          val stop = liveBlockStop(b)
          p = liveBlockHead(b)
          var firstStep = true
          while (firstStep || p != stop) {
            firstStep = false
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
          }
        }
        b += 1
      }
    }
    if (m == none) {
      return none
    }
    val mx = m.x
    val my = m.y
    val tminY = math.min(hy, my)
    val tmaxY = math.max(hy, my)
    var tanMin = Double.PositiveInfinity
    for (b <- 0 until numBlocks) {
      val g = b * 4
      if (!(blockBBox(g + 2) < mx || blockBBox(g) > hx || blockBBox(g + 3) < tminY || blockBBox(g + 1) > tmaxY)) {
        val stop = liveBlockStop(b)
        p = liveBlockHead(b)
        var firstStep = true
        while (firstStep || p != stop) {
          firstStep = false
          if (p.prev.next == p && hx >= p.x && p.x >= mx && hx != p.x &&
              pointInTriangle(if (hy < my) hx else qx, hy, mx, my, if (hy < my) qx else hx, hy, p.x, p.y)) {
            val tan = math.abs(hy - p.y) / (hx - p.x)
            if ((locallyInside(p, hole) || p.y == hy && p.next.y == hy && p.next.x > hx) && (tan < tanMin ||
                tan == tanMin && (p.x > m.x || p.x == m.x && sectorContainsSector(m, p)))) {
              m = p
              tanMin = tan
            }
          }
          p = p.next
        }
      }
    }
    return m
  }

  private def sectorContainsSector(m: Vertex, p: Vertex): Boolean = {
    return area(m.prev, m, p.prev) < 0 && area(p.next, m, m.next) < 0
  }

  // Recompute Morton keys after hole merging, then relink the sorted nodes.
  private def indexCurve(start: Vertex, minX: Double, minY: Double, invSize: Double): Unit = {
    val sortArr = new js.Array[Vertex]()
    var p = start
    var n = 0
    var firstPass = true
    while (firstPass || p != start) {
      firstPass = false
      p.z = zOrder(p.x, p.y, minX, minY, invSize)
      sortArr.push(p)
      n += 1
      p = p.next
    }
    sortNodes(sortArr, n)
    var prev = none
    for (i <- 0 until n) {
      val node = sortArr(i)
      node.prevZ = prev
      if (prev != none) {
        prev.nextZ = node
      }
      prev = node
    }
    prev.nextZ = none
  }

  // Use stable insertion sort for short rings and four stable radix passes for larger rings.
  private def sortNodes(sortArr: js.Array[Vertex], n: Int): Unit = {
    if (n <= 32) {
      for (i <- 1 until n) {
        val node = sortArr(i)
        val z = node.z
        var j = i - 1
        while (j >= 0 && sortArr(j).z > z) {
          sortArr(j + 1) = sortArr(j)
          j -= 1
        }
        sortArr(j + 1) = node
      }
      return
    }
    val zArr = new Array[Int](n)
    val zBuf = new Array[Int](n)
    val sortBuf = new js.Array[Vertex](n)
    for (i <- 0 until n) {
      zArr(i) = sortArr(i).z
    }
    radixPass(n, sortArr, zArr, sortBuf, zBuf, 0)
    radixPass(n, sortBuf, zBuf, sortArr, zArr, 8)
    radixPass(n, sortArr, zArr, sortBuf, zBuf, 16)
    radixPass(n, sortBuf, zBuf, sortArr, zArr, 24)
  }

  private def radixPass(n: Int, src: js.Array[Vertex], srcZ: Array[Int], dst: js.Array[Vertex], dstZ: Array[Int], shift: Int): Unit = {
    val counts = new Array[Int](256)
    for (i <- 0 until n) {
      counts(srcZ(i) >>> shift & 0xff) += 1
    }
    var sum = 0
    for (b <- 0 until 256) {
      val c = counts(b)
      counts(b) = sum
      sum += c
    }
    for (i <- 0 until n) {
      val z = srcZ(i)
      val bucket = z >>> shift & 0xff
      val pos = counts(bucket)
      counts(bucket) += 1
      dst(pos) = src(i)
      dstZ(pos) = z
    }
  }

  private def zOrder(x: Double, y: Double, minX: Double, minY: Double, invSize: Double): Int = {
    var xx = toInt32((x - minX) * invSize)
    var yy = toInt32((y - minY) * invSize)
    xx = (xx | xx << 8) & 0x00FF00FF
    xx = (xx | xx << 4) & 0x0F0F0F0F
    xx = (xx | xx << 2) & 0x33333333
    xx = (xx | xx << 1) & 0x55555555
    yy = (yy | yy << 8) & 0x00FF00FF
    yy = (yy | yy << 4) & 0x0F0F0F0F
    yy = (yy | yy << 2) & 0x33333333
    yy = (yy | yy << 1) & 0x55555555
    return xx | yy << 1
  }

  private def getLeftmost(start: Vertex): Vertex = {
    var p = start
    var leftmost = start
    var firstPass = true
    while (firstPass || p != start) {
      firstPass = false
      if (p.x < leftmost.x || p.x == leftmost.x && p.y < leftmost.y) {
        leftmost = p
      }
      p = p.next
    }
    return leftmost
  }

  private def pointInTriangle(ax: Double, ay: Double, bx: Double, by: Double, cx: Double, cy: Double, px: Double, py: Double): Boolean = {
    return (cx - px) * (ay - py) >= (ax - px) * (cy - py) && (ax - px) * (by - py) >= (bx - px) * (ay - py) &&
        (bx - px) * (cy - py) >= (cx - px) * (by - py)
  }

  private def isValidDiagonal(a: Vertex, b: Vertex): Boolean = {
    val zeroLength = samePoint(a, b) && area(a.prev, a, a.next) > 0 && area(b.prev, b, b.next) > 0
    return a.next.i != b.i && (zeroLength || locallyInside(a, b) && locallyInside(b, a) &&
        (area(a.prev, a, b.prev) != 0 || area(a, b.prev, b) != 0)) && !intersectsPolygon(a, b) && (zeroLength ||
        middleInside(a, b))
  }

  private def area(p: Vertex, q: Vertex, r: Vertex): Double = {
    return (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y)
  }

  private def samePoint(p1: Vertex, p2: Vertex): Boolean = {
    return p1.x == p2.x && p1.y == p2.y
  }

  private def intersects(p1: Vertex, q1: Vertex, p2: Vertex, q2: Vertex, includeBoundary: Boolean = true): Boolean = {
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
    if (o1 == 0 && onSegment(p1, p2, q1)) {
      return true
    }
    if (o2 == 0 && onSegment(p1, q2, q1)) {
      return true
    }
    if (o3 == 0 && onSegment(p2, p1, q2)) {
      return true
    }
    if (o4 == 0 && onSegment(p2, q1, q2)) {
      return true
    }
    return false
  }

  private def onSegment(p: Vertex, q: Vertex, r: Vertex): Boolean = {
    return q.x <= math.max(p.x, r.x) && q.x >= math.min(p.x, r.x) && q.y <= math.max(p.y, r.y) &&
        q.y >= math.min(p.y, r.y)
  }

  private def intersectsPolygon(a: Vertex, b: Vertex): Boolean = {
    val minX = math.min(a.x, b.x)
    val maxX = math.max(a.x, b.x)
    val minY = math.min(a.y, b.y)
    val maxY = math.max(a.y, b.y)
    var p = a
    var firstPass = true
    while (firstPass || p != a) {
      firstPass = false
      val n = p.next
      if (!(p.x > maxX && n.x > maxX || p.x < minX && n.x < minX || p.y > maxY && n.y > maxY || p.y < minY &&
          n.y < minY)) {
        if (p.i != a.i && n.i != a.i && p.i != b.i && n.i != b.i && intersects(p, n, a, b)) {
          return true
        }
      }
      p = n
    }
    return false
  }

  private def locallyInside(a: Vertex, b: Vertex): Boolean = {
    return if (area(a.prev, a, a.next) < 0) area(a, b, a.next) >= 0 &&
        area(a, a.prev, b) >= 0 else area(a, b, a.prev) < 0 || area(a, a.next, b) < 0
  }

  private def middleInside(a: Vertex, b: Vertex): Boolean = {
    var p = a
    var inside = false
    val px = (a.x + b.x) / 2
    val py = (a.y + b.y) / 2
    var firstPass = true
    while (firstPass || p != a) {
      firstPass = false
      val n = p.next
      if (p.y > py != n.y > py && px < (n.x - p.x) * (py - p.y) / (n.y - p.y) + p.x) {
        inside = !inside
      }
      p = n
    }
    return inside
  }

  private def splitPolygon(a: Vertex, b: Vertex): Vertex = {
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

  private def insertNode(i: Int, x: Double, y: Double, last: Vertex): Vertex = {
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

  private def removeNode(p: Vertex): Unit = {
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

  def deviation(data: js.Array[Double], holeIndices: js.Array[Int], dim: Int, triangles: js.Array[Int]): Double = {
    val hasHoles = holeIndices.length > 0
    val outerLen = if (hasHoles) holeIndices(0) * dim else data.length
    var polygonArea = math.abs(signedArea(data, 0, outerLen, dim))
    if (hasHoles) {
      for (i <- 0 until holeIndices.length) {
        val start = holeIndices(i) * dim
        val end = if (i < holeIndices.length - 1) holeIndices(i + 1) * dim else data.length
        polygonArea -= math.abs(signedArea(data, start, end, dim))
      }
    }
    var trianglesArea = 0.0
    for (i <- 0 until triangles.length by 3) {
      val a = triangles(i) * dim
      val b = triangles(i + 1) * dim
      val c = triangles(i + 2) * dim
      trianglesArea += math.abs((data(a) - data(c)) * (data(b + 1) - data(a + 1)) - (data(a) - data(b)) * (data(c + 1) - data(a + 1)))
    }
    if (trianglesArea == 0) {
      var max = 0.0
      for (i <- 0 until data.length by dim) {
        max = math.max(math.max(max, math.abs(data(i))), math.abs(data(i + 1)))
      }
      return if (math.abs(polygonArea) <= data.length * max * max * 2.220446049250313e-16) 0.0 else 1.0
    }
    return math.abs((trianglesArea - polygonArea) / polygonArea)
  }

  private def signedArea(data: js.Array[Double], start: Int, end: Int, dim: Int): Double = {
    var sum = 0.0
    var j = end - dim
    for (i <- start until end by dim) {
      sum += (data(j) - data(i)) * (data(i + 1) + data(j + 1))
      j = i
    }
    return sum
  }

  // Build half-edge twins and legalize interior edges with Lawson flips. The input triangle list is owned by the caller.
  def refine(triangles: js.Array[Int], coords: js.Array[Double], dim: Int = 2): Unit = {
    val t = triangles
    val n = t.length
    if (n < 6) {
      return
    }
    var size = 1
    while (size < n * 4) {
      size = size << 1
    }
    val hMask = size - 1
    val hTable = new Array[Int](size)
    val hStamp = new Array[Int](size)
    val edgeStack = new Array[Int](n)
    val edgeStamp = new Array[Int](n)
    val he = Array.fill(n)(-1)
    var i = 0
    for (e <- 0 until n) {
      val a = t(e)
      val b = t(nextHE(e))
      val lo = if (a < b) a else b
      val hi = if (a < b) b else a
      var h = ((lo * -1640531535) ^ (hi * -2048144789)) & hMask
      var matched = false
      while (!matched && hStamp(h) == 1) {
        val s = hTable(h)
        if (s != -1) {
          val sa = t(s)
          val sb = t(nextHE(s))
          if (sa == lo && sb == hi || sa == hi && sb == lo) {
            he(e) = s
            he(s) = e
            hTable(h) = -1
            edgeStamp(s) = 1
            edgeStack(i) = s
            i += 1
            matched = true
          }
        }
        if (!matched) {
          h = h + 1 & hMask
        }
      }
      if (hStamp(h) != 1) {
        hTable(h) = e
        hStamp(h) = 1
      }
    }
    while (i > 0) {
      i -= 1
      val a = edgeStack(i)
      edgeStamp(a) = 0
      val b = he(a)
      if (b != -1) {
        val a0 = a - a % 3
        val b0 = b - b % 3
        val ar = a0 + (a + 2) % 3
        val al = a0 + (a + 1) % 3
        val bl = b0 + (b + 2) % 3
        val br = b0 + (b + 1) % 3
        val p0 = t(ar)
        val pr = t(a)
        val pl = t(al)
        val p1 = t(bl)
        val x0 = coords(p0 * dim)
        val y0 = coords(p0 * dim + 1)
        val xr = coords(pr * dim)
        val yr = coords(pr * dim + 1)
        val xl = coords(pl * dim)
        val yl = coords(pl * dim + 1)
        val x1 = coords(p1 * dim)
        val y1 = coords(p1 * dim + 1)
        if (!inCircle(x0, y0, xr, yr, xl, yl, x1, y1) && orient(x0, y0, xr, yr, x1, y1) > 0 &&
            orient(x0, y0, x1, y1, xl, yl) > 0) {
          t(a) = p1
          t(b) = p0
          val hbl = he(bl)
          val har = he(ar)
          he(a) = hbl
          if (hbl != -1) {
            he(hbl) = a
          }
          he(b) = har
          if (har != -1) {
            he(har) = b
          }
          he(ar) = bl
          he(bl) = ar
          if (hbl != -1 && edgeStamp(a) == 0) {
            edgeStamp(a) = 1
            edgeStack(i) = a
            i += 1
          }
          if (har != -1 && edgeStamp(b) == 0) {
            edgeStamp(b) = 1
            edgeStack(i) = b
            i += 1
          }
          if (he(al) != -1 && edgeStamp(al) == 0) {
            edgeStamp(al) = 1
            edgeStack(i) = al
            i += 1
          }
          if (he(br) != -1 && edgeStamp(br) == 0) {
            edgeStamp(br) = 1
            edgeStack(i) = br
            i += 1
          }
        }
      }
    }
  }

  private def orient(ax: Double, ay: Double, bx: Double, by: Double, cx: Double, cy: Double): Double = {
    return (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
  }

  // The relative tolerance makes near-cocircular diagonals legal ties and prevents flip cycles.
  private def inCircle(ax: Double, ay: Double, bx: Double, by: Double, cx: Double, cy: Double, px: Double, py: Double): Boolean = {
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

  private def nextHE(e: Int): Int = {
    return e - e % 3 + (e + 1) % 3
  }

  private def filterPoints(start: Vertex): Vertex = filterPoints(start, start)

  // Z-order uses ECMAScript ToInt32 even for holes far outside the outer bounds.
  private def toInt32(value: Double): Int = {
    if (value >= -2147483648.0 && value < 2147483648.0) return value.toInt
    val magnitude = math.abs(value) % 4294967296.0
    val bits = if (magnitude >= 2147483648.0) (magnitude - 2147483648.0).toInt + Int.MinValue else magnitude.toInt
    if (value < 0) -bits else bits
  }

  private def createVertex(i: Int, x: Double, y: Double): Vertex = {
    val p = new Vertex(i, x, y)
    p.prevZ = none
    p.nextZ = none
    p
  }
}
