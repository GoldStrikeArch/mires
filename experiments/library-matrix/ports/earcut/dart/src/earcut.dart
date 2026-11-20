// Adapted from earcut 3.2.4. See LICENSE for the upstream ISC license.
import 'dart:math' as math;
import 'dart:typed_data';

final class Vertex {
  Vertex(this.i, this.x, this.y);
  final int i;
  final double x, y;
  late Vertex prev = this, next = this, prevZ = this, nextZ = this;
  int z = 0;
  bool steiner = false;
}

final class Earcut {
  final none = Vertex(-1, 0.0, 0.0);
  bool filteredOut = false, indexActive = false;
  int numBlocks = 0;
  Float64List blockBBox = Float64List(0);
  List<Vertex> blockHead = [], blockStop = [];

  // Triangulate the outer ring, merging holes before clipping.
  List<int> earcut(List<double> data, List<int> holeIndices, [int dim = 2]) {
    final hasHoles = holeIndices.length > 0;
    final outerLen = hasHoles ? holeIndices[0] * dim : data.length;
    var outerNode = linkedList(data, 0, outerLen, dim, true);
    final triangles = <int>[];
    if (outerNode == none || outerNode.next == outerNode.prev) {
      return triangles;
    }
    var minX = 0.0;
    var minY = 0.0;
    var invSize = 0.0;
    if (hasHoles) {
      outerNode = eliminateHoles(data, holeIndices, outerNode, dim);
    }
    if (data.length > 80 * dim) {
      minX = data[0];
      minY = data[1];
      var maxX = minX;
      var maxY = minY;
      for (var i = dim; i < outerLen; i += dim) {
        final x = data[i];
        final y = data[i + 1];
        if (x < minX) {
          minX = x;
        }
        if (y < minY) {
          minY = y;
        }
        if (x > maxX) {
          maxX = x;
        }
        if (y > maxY) {
          maxY = y;
        }
      }
      invSize = math.max(maxX - minX, maxY - minY);
      invSize = invSize != 0.0 ? 32767.0 / invSize : 0.0;
    }
    earcutLinked(outerNode, triangles, minX, minY, invSize);
    return triangles;
  }

  Vertex linkedList(List<double> data, int start, int end, int dim, bool clockwise) {
    var last = none;
    if (clockwise == signedArea(data, start, end, dim) > 0) {
      for (var i = start; i < end; i += dim) {
        last = insertNode((i ~/ dim), data[i], data[i + 1], last);
      }
    } else {
      for (var i = end - dim; i >= start; i -= dim) {
        last = insertNode((i ~/ dim), data[i], data[i + 1], last);
      }
    }
    if (last != none && samePoint(last, last.next)) {
      removeNode(last);
      last = last.next;
    }
    return last;
  }

  // Sweep to a fixpoint, or heal only the short window between two bridge vertices.
  Vertex filterPoints(Vertex start, [Vertex? limit]) {
    var end = limit ?? start;
    final full = end == start;
    var p = start;
    var again = false;
    do {
      again = false;
      if (p != p.next && !p.steiner && (samePoint(p, p.next) || area(p.prev, p, p.next) == 0)) {
        if (full || p == end) {
          end = p.prev;
        }
        filteredOut = true;
        removeNode(p);
        p = p.prev;
        again = true;
      } else if (full || p != end) {
        p = p.next;
        again = !full;
      }
    } while (again || p != end);
    return end;
  }

  // On a stalled ring, filter collinear points, cure local crossings, then try a diagonal.
  void earcutLinked(Vertex ear, List<int> triangles, double minX, double minY, double invSize) {
    if (invSize != 0.0) {
      indexCurve(ear, minX, minY, invSize);
    }
    var stop = ear;
    var cured = false;
    while (ear.prev != ear.next) {
      final prev = ear.prev;
      final next = ear.next;
      if (area(prev, ear, next) < 0 &&
          (invSize != 0.0 ? isEarHashed(ear, minX, minY, invSize) : isEar(ear))) {
        triangles.add(prev.i);
        triangles.add(ear.i);
        triangles.add(next.i);
        removeNode(ear);
        ear = next;
        stop = next;
        continue;
      }
      ear = next;
      if (ear == stop) {
        filteredOut = false;
        ear = filterPoints(ear);
        if (filteredOut) {
          stop = ear;
          continue;
        }
        if (!cured) {
          ear = cureLocalIntersections(ear, triangles);
          stop = ear;
          cured = true;
          continue;
        }
        splitEarcut(ear, triangles, minX, minY, invSize);
        break;
      }
    }
  }

  bool isEar(Vertex ear) {
    final a = ear.prev;
    final b = ear;
    final c = ear.next;
    final ax = a.x;
    final bx = b.x;
    final cx = c.x;
    final ay = a.y;
    final by = b.y;
    final cy = c.y;
    final x0 = math.min(math.min(ax, bx), cx);
    final y0 = math.min(math.min(ay, by), cy);
    final x1 = math.max(math.max(ax, bx), cx);
    final y1 = math.max(math.max(ay, by), cy);
    var p = c.next;
    while (p != a) {
      if (p.x >= x0 &&
          p.x <= x1 &&
          p.y >= y0 &&
          p.y <= y1 &&
          !(ax == p.x && ay == p.y) &&
          pointInTriangle(ax, ay, bx, by, cx, cy, p.x, p.y) &&
          area(p.prev, p, p.next) >= 0) {
        return false;
      }
      p = p.next;
    }
    return true;
  }

  bool isEarHashed(Vertex ear, double minX, double minY, double invSize) {
    final a = ear.prev;
    final b = ear;
    final c = ear.next;
    final ax = a.x;
    final bx = b.x;
    final cx = c.x;
    final ay = a.y;
    final by = b.y;
    final cy = c.y;
    final x0 = math.min(math.min(ax, bx), cx);
    final y0 = math.min(math.min(ay, by), cy);
    final x1 = math.max(math.max(ax, bx), cx);
    final y1 = math.max(math.max(ay, by), cy);
    final minZ = zOrder(x0, y0, minX, minY, invSize);
    final maxZ = zOrder(x1, y1, minX, minY, invSize);
    var p = ear.prevZ;
    while (p != none && p.z >= minZ) {
      if (p.x >= x0 &&
          p.x <= x1 &&
          p.y >= y0 &&
          p.y <= y1 &&
          p != c &&
          !(ax == p.x && ay == p.y) &&
          pointInTriangle(ax, ay, bx, by, cx, cy, p.x, p.y) &&
          area(p.prev, p, p.next) >= 0) {
        return false;
      }
      p = p.prevZ;
    }
    var n = ear.nextZ;
    while (n != none && n.z <= maxZ) {
      if (n.x >= x0 &&
          n.x <= x1 &&
          n.y >= y0 &&
          n.y <= y1 &&
          n != c &&
          !(ax == n.x && ay == n.y) &&
          pointInTriangle(ax, ay, bx, by, cx, cy, n.x, n.y) &&
          area(n.prev, n, n.next) >= 0) {
        return false;
      }
      n = n.nextZ;
    }
    return true;
  }

  Vertex cureLocalIntersections(Vertex start, List<int> triangles) {
    var p = start;
    var cured = false;
    do {
      final a = p.prev;
      final b = p.next.next;
      if (intersects(a, p, p.next, b, false) && locallyInside(a, b) && locallyInside(b, a)) {
        triangles.add(a.i);
        triangles.add(p.i);
        triangles.add(b.i);
        removeNode(p);
        removeNode(p.next);
        p = b;
        start = b;
        cured = true;
      }
      p = p.next;
    } while (p != start);
    return cured ? filterPoints(p) : p;
  }

  void splitEarcut(Vertex start, List<int> triangles, double minX, double minY, double invSize) {
    var a = start;
    do {
      var b = a.next.next;
      while (b != a.prev) {
        if (a.i != b.i && isValidDiagonal(a, b)) {
          var c = splitPolygon(a, b);
          a = filterPoints(a, a.next);
          c = filterPoints(c, c.next);
          earcutLinked(a, triangles, minX, minY, invSize);
          earcutLinked(c, triangles, minX, minY, invSize);
          return;
        }
        b = b.next;
      }
      a = a.next;
    } while (a != start);
  }

  // Bridge holes in stable leftmost/slope order and heal both sides of each splice.
  Vertex eliminateHoles(List<double> data, List<int> holeIndices, Vertex outerNode, int dim) {
    final queue = <Vertex>[];
    for (var i = 0; i < holeIndices.length; i += 1) {
      final start = holeIndices[i] * dim;
      final end = i < holeIndices.length - 1 ? holeIndices[i + 1] * dim : data.length;
      final list = linkedList(data, start, end, dim, false);
      if (list == list.next) {
        list.steiner = true;
      }
      queue.add(getLeftmost(list));
    }
    sortHoles(queue);
    buildBlockIndex((data.length ~/ dim), holeIndices.length);
    indexSegment(outerNode, outerNode);
    indexActive = true;
    for (var i = 0; i < queue.length; i += 1) {
      outerNode = eliminateHole(queue[i], outerNode);
    }
    indexActive = false;
    return filterPoints(outerNode);
  }

  double compareXYSlope(Vertex a, Vertex b) {
    if (a.x != b.x) {
      return a.x - b.x;
    }
    if (a.y != b.y) {
      return a.y - b.y;
    }
    return (a.next.y - a.y) / (a.next.x - a.x) - (b.next.y - b.y) / (b.next.x - b.x);
  }

  Vertex eliminateHole(Vertex hole, Vertex outerNode) {
    final bridge = findHoleBridge(hole, outerNode);
    if (bridge == none) {
      return outerNode;
    }
    final bridgeReverse = splitPolygon(bridge, hole);
    final bridge2 = bridgeReverse.next;
    indexSegment(bridge, bridge2.next);
    filterPoints(bridgeReverse, bridgeReverse.next);
    return filterPoints(bridge, bridge.next);
  }

  // Each block bounds 16 consecutive ring edges. Merged holes append new segments.
  void buildBlockIndex(int maxNodes, int numHoles) {
    final blocks = (maxNodes + 2 * numHoles + 15 ~/ 16) + numHoles + 2;
    blockBBox = Float64List(blocks * 4);
    blockHead = List<Vertex>.filled(blocks, none);
    blockStop = List<Vertex>.filled(blocks, none);
    numBlocks = 0;
  }

  void indexSegment(Vertex head, Vertex stop) {
    var p = head;
    do {
      final b = numBlocks;
      numBlocks += 1;
      blockHead[b] = p;
      var minX = p.x;
      var minY = p.y;
      var maxX = p.x;
      var maxY = p.y;
      var k = 0;
      do {
        final c = p.next;
        p.z = b;
        if (c.x < minX) {
          minX = c.x;
        }
        if (c.x > maxX) {
          maxX = c.x;
        }
        if (c.y < minY) {
          minY = c.y;
        }
        if (c.y > maxY) {
          maxY = c.y;
        }
        p = c;
        k += 1;
      } while (k < 16 && p != stop);
      blockStop[b] = p;
      final g = b * 4;
      blockBBox[g] = minX;
      blockBBox[g + 1] = minY;
      blockBBox[g + 2] = maxX;
      blockBBox[g + 3] = maxY;
    } while (p != stop);
  }

  // A healed edge can extend the old block bounds when its endpoint lived in another block.
  void growBlock(Vertex head, Vertex tail) {
    final g = head.z * 4;
    if (tail.x < blockBBox[g]) {
      blockBBox[g] = tail.x;
    }
    if (tail.y < blockBBox[g + 1]) {
      blockBBox[g + 1] = tail.y;
    }
    if (tail.x > blockBBox[g + 2]) {
      blockBBox[g + 2] = tail.x;
    }
    if (tail.y > blockBBox[g + 3]) {
      blockBBox[g + 3] = tail.y;
    }
  }

  Vertex liveBlockStop(int b) {
    var stop = blockStop[b];
    while (stop.prev.next != stop) {
      stop = stop.next;
    }
    blockStop[b] = stop;
    return stop;
  }

  Vertex liveBlockHead(int b) {
    var head = blockHead[b];
    while (head.prev.next != head) {
      head = head.next;
    }
    blockHead[b] = head;
    return head;
  }

  // Cast a horizontal ray to the left and then choose the locally visible minimum-angle bridge.
  Vertex findHoleBridge(Vertex hole, Vertex outerNode) {
    var p = outerNode;
    final hx = hole.x;
    final hy = hole.y;
    var qx = -double.infinity;
    var m = none;
    if (samePoint(hole, p)) {
      return p;
    }
    for (var b = 0; b < numBlocks; b += 1) {
      final g = b * 4;
      if (hy < blockBBox[g + 1] ||
          hy > blockBBox[g + 3] ||
          blockBBox[g] > hx ||
          blockBBox[g + 2] <= qx) {
        continue;
      }
      final stop = liveBlockStop(b);
      p = liveBlockHead(b);
      do {
        if (p.prev.next == p) {
          if (samePoint(hole, p.next)) {
            return p.next;
          } else if (hy <= p.y && hy >= p.next.y && p.next.y != p.y) {
            final x = p.x + (hy - p.y) * (p.next.x - p.x) / (p.next.y - p.y);
            if (x <= hx && x > qx) {
              qx = x;
              m = p.x < p.next.x ? p : p.next;
              if (x == hx) {
                return m;
              }
            }
          }
        }
        p = p.next;
      } while (p != stop);
    }
    if (m == none) {
      return none;
    }
    final mx = m.x;
    final my = m.y;
    final tminY = math.min(hy, my);
    final tmaxY = math.max(hy, my);
    var tanMin = double.infinity;
    for (var b = 0; b < numBlocks; b += 1) {
      final g = b * 4;
      if (blockBBox[g + 2] < mx ||
          blockBBox[g] > hx ||
          blockBBox[g + 3] < tminY ||
          blockBBox[g + 1] > tmaxY) {
        continue;
      }
      final stop = liveBlockStop(b);
      p = liveBlockHead(b);
      do {
        if (p.prev.next == p &&
            hx >= p.x &&
            p.x >= mx &&
            hx != p.x &&
            pointInTriangle(hy < my ? hx : qx, hy, mx, my, hy < my ? qx : hx, hy, p.x, p.y)) {
          final tan = (hy - p.y).abs() / (hx - p.x);
          if ((locallyInside(p, hole) || p.y == hy && p.next.y == hy && p.next.x > hx) &&
              (tan < tanMin ||
                  tan == tanMin && (p.x > m.x || p.x == m.x && sectorContainsSector(m, p)))) {
            m = p;
            tanMin = tan;
          }
        }
        p = p.next;
      } while (p != stop);
    }
    return m;
  }

  bool sectorContainsSector(Vertex m, Vertex p) {
    return area(m.prev, m, p.prev) < 0 && area(p.next, m, m.next) < 0;
  }

  // Recompute Morton keys after hole merging, then relink the sorted nodes.
  void indexCurve(Vertex start, double minX, double minY, double invSize) {
    final sortArr = <Vertex>[];
    var p = start;
    var n = 0;
    do {
      p.z = zOrder(p.x, p.y, minX, minY, invSize);
      sortArr.add(p);
      n += 1;
      p = p.next;
    } while (p != start);
    sortNodes(sortArr, n);
    var prev = none;
    for (var i = 0; i < n; i += 1) {
      final node = sortArr[i];
      node.prevZ = prev;
      if (prev != none) {
        prev.nextZ = node;
      }
      prev = node;
    }
    prev.nextZ = none;
  }

  // Use stable insertion sort for short rings and four stable radix passes for larger rings.
  void sortNodes(List<Vertex> sortArr, int n) {
    if (n <= 32) {
      for (var i = 1; i < n; i += 1) {
        final node = sortArr[i];
        final z = node.z;
        var j = i - 1;
        while (j >= 0 && sortArr[j].z > z) {
          sortArr[j + 1] = sortArr[j];
          j -= 1;
        }
        sortArr[j + 1] = node;
      }
      return;
    }
    final zArr = Int32List(n);
    final zBuf = Int32List(n);
    final sortBuf = List<Vertex>.filled(n, none);
    for (var i = 0; i < n; i += 1) {
      zArr[i] = sortArr[i].z;
    }
    radixPass(n, sortArr, zArr, sortBuf, zBuf, 0);
    radixPass(n, sortBuf, zBuf, sortArr, zArr, 8);
    radixPass(n, sortArr, zArr, sortBuf, zBuf, 16);
    radixPass(n, sortBuf, zBuf, sortArr, zArr, 24);
  }

  void radixPass(
    int n,
    List<Vertex> src,
    Int32List srcZ,
    List<Vertex> dst,
    Int32List dstZ,
    int shift,
  ) {
    final counts = Int32List(256);
    for (var i = 0; i < n; i += 1) {
      counts[srcZ[i] >>> shift & 0xff] += 1;
    }
    var sum = 0;
    for (var b = 0; b < 256; b += 1) {
      final c = counts[b];
      counts[b] = sum;
      sum += c;
    }
    for (var i = 0; i < n; i += 1) {
      final z = srcZ[i];
      final bucket = z >>> shift & 0xff;
      final pos = counts[bucket];
      counts[bucket] += 1;
      dst[pos] = src[i];
      dstZ[pos] = z;
    }
  }

  int zOrder(double x, double y, double minX, double minY, double invSize) {
    var xx = toInt32((x - minX) * invSize);
    var yy = toInt32((y - minY) * invSize);
    xx = (xx | xx << 8) & 0x00FF00FF;
    xx = (xx | xx << 4) & 0x0F0F0F0F;
    xx = (xx | xx << 2) & 0x33333333;
    xx = (xx | xx << 1) & 0x55555555;
    yy = (yy | yy << 8) & 0x00FF00FF;
    yy = (yy | yy << 4) & 0x0F0F0F0F;
    yy = (yy | yy << 2) & 0x33333333;
    yy = (yy | yy << 1) & 0x55555555;
    return (xx | yy << 1).toSigned(32);
  }

  Vertex getLeftmost(Vertex start) {
    var p = start;
    var leftmost = start;
    do {
      if (p.x < leftmost.x || p.x == leftmost.x && p.y < leftmost.y) {
        leftmost = p;
      }
      p = p.next;
    } while (p != start);
    return leftmost;
  }

  bool pointInTriangle(
    double ax,
    double ay,
    double bx,
    double by,
    double cx,
    double cy,
    double px,
    double py,
  ) {
    return (cx - px) * (ay - py) >= (ax - px) * (cy - py) &&
        (ax - px) * (by - py) >= (bx - px) * (ay - py) &&
        (bx - px) * (cy - py) >= (cx - px) * (by - py);
  }

  bool isValidDiagonal(Vertex a, Vertex b) {
    final zeroLength =
        samePoint(a, b) && area(a.prev, a, a.next) > 0 && area(b.prev, b, b.next) > 0;
    return a.next.i != b.i &&
        (zeroLength ||
            locallyInside(a, b) &&
                locallyInside(b, a) &&
                (area(a.prev, a, b.prev) != 0 || area(a, b.prev, b) != 0)) &&
        !intersectsPolygon(a, b) &&
        (zeroLength || middleInside(a, b));
  }

  double area(Vertex p, Vertex q, Vertex r) {
    return (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y);
  }

  bool samePoint(Vertex p1, Vertex p2) {
    return p1.x == p2.x && p1.y == p2.y;
  }

  bool intersects(Vertex p1, Vertex q1, Vertex p2, Vertex q2, [bool includeBoundary = true]) {
    final o1 = area(p1, q1, p2);
    final o2 = area(p1, q1, q2);
    final o3 = area(p2, q2, p1);
    final o4 = area(p2, q2, q1);
    if ((o1 > 0 && o2 < 0 || o1 < 0 && o2 > 0) && (o3 > 0 && o4 < 0 || o3 < 0 && o4 > 0)) {
      return true;
    }
    if (!includeBoundary) {
      return false;
    }
    if (o1 == 0 && onSegment(p1, p2, q1)) {
      return true;
    }
    if (o2 == 0 && onSegment(p1, q2, q1)) {
      return true;
    }
    if (o3 == 0 && onSegment(p2, p1, q2)) {
      return true;
    }
    if (o4 == 0 && onSegment(p2, q1, q2)) {
      return true;
    }
    return false;
  }

  bool onSegment(Vertex p, Vertex q, Vertex r) {
    return q.x <= math.max(p.x, r.x) &&
        q.x >= math.min(p.x, r.x) &&
        q.y <= math.max(p.y, r.y) &&
        q.y >= math.min(p.y, r.y);
  }

  bool intersectsPolygon(Vertex a, Vertex b) {
    final minX = math.min(a.x, b.x);
    final maxX = math.max(a.x, b.x);
    final minY = math.min(a.y, b.y);
    final maxY = math.max(a.y, b.y);
    var p = a;
    do {
      final n = p.next;
      if (p.x > maxX && n.x > maxX ||
          p.x < minX && n.x < minX ||
          p.y > maxY && n.y > maxY ||
          p.y < minY && n.y < minY) {
        p = n;
        continue;
      }
      if (p.i != a.i && n.i != a.i && p.i != b.i && n.i != b.i && intersects(p, n, a, b)) {
        return true;
      }
      p = n;
    } while (p != a);
    return false;
  }

  bool locallyInside(Vertex a, Vertex b) {
    return area(a.prev, a, a.next) < 0
        ? area(a, b, a.next) >= 0 && area(a, a.prev, b) >= 0
        : area(a, b, a.prev) < 0 || area(a, a.next, b) < 0;
  }

  bool middleInside(Vertex a, Vertex b) {
    var p = a;
    var inside = false;
    final px = (a.x + b.x) / 2;
    final py = (a.y + b.y) / 2;
    do {
      final n = p.next;
      if (p.y > py != n.y > py && px < (n.x - p.x) * (py - p.y) / (n.y - p.y) + p.x) {
        inside = !inside;
      }
      p = n;
    } while (p != a);
    return inside;
  }

  Vertex splitPolygon(Vertex a, Vertex b) {
    final a2 = createVertex(a.i, a.x, a.y);
    final b2 = createVertex(b.i, b.x, b.y);
    final an = a.next;
    final bp = b.prev;
    a.next = b;
    b.prev = a;
    a2.next = an;
    an.prev = a2;
    b2.next = a2;
    a2.prev = b2;
    bp.next = b2;
    b2.prev = bp;
    return b2;
  }

  Vertex insertNode(int i, double x, double y, Vertex last) {
    final p = createVertex(i, x, y);
    if (last == none) {
      p.prev = p;
      p.next = p;
    } else {
      p.next = last.next;
      p.prev = last;
      last.next.prev = p;
      last.next = p;
    }
    return p;
  }

  void removeNode(Vertex p) {
    p.next.prev = p.prev;
    p.prev.next = p.next;
    if (p.prevZ != none) {
      p.prevZ.nextZ = p.nextZ;
    }
    if (p.nextZ != none) {
      p.nextZ.prevZ = p.prevZ;
    }
    if (indexActive) {
      growBlock(p.prev, p.next);
    }
  }

  double deviation(List<double> data, List<int> holeIndices, int dim, List<int> triangles) {
    final hasHoles = holeIndices.length > 0;
    final outerLen = hasHoles ? holeIndices[0] * dim : data.length;
    var polygonArea = (signedArea(data, 0, outerLen, dim)).abs();
    if (hasHoles) {
      for (var i = 0; i < holeIndices.length; i += 1) {
        final start = holeIndices[i] * dim;
        final end = i < holeIndices.length - 1 ? holeIndices[i + 1] * dim : data.length;
        polygonArea -= (signedArea(data, start, end, dim)).abs();
      }
    }
    var trianglesArea = 0.0;
    for (var i = 0; i < triangles.length; i += 3) {
      final a = triangles[i] * dim;
      final b = triangles[i + 1] * dim;
      final c = triangles[i + 2] * dim;
      trianglesArea +=
          ((data[a] - data[c]) * (data[b + 1] - data[a + 1]) -
                  (data[a] - data[b]) * (data[c + 1] - data[a + 1]))
              .abs();
    }
    if (trianglesArea == 0) {
      var max = 0.0;
      for (var i = 0; i < data.length; i += dim) {
        max = math.max(math.max(max, (data[i]).abs()), (data[i + 1]).abs());
      }
      return (polygonArea).abs() <= data.length * max * max * 2.220446049250313e-16 ? 0.0 : 1.0;
    }
    return ((trianglesArea - polygonArea) / polygonArea).abs();
  }

  double signedArea(List<double> data, int start, int end, int dim) {
    var sum = 0.0;
    var j = end - dim;
    for (var i = start; i < end; i += dim) {
      sum += (data[j] - data[i]) * (data[i + 1] + data[j + 1]);
      j = i;
    }
    return sum;
  }

  // Build half-edge twins and legalize interior edges with Lawson flips. The input triangle list is owned by the caller.
  void refine(List<int> triangles, List<double> coords, [int dim = 2]) {
    final t = triangles;
    final n = t.length;
    if (n < 6) {
      return;
    }
    var size = 1;
    while (size < n * 4) {
      size = size << 1;
    }
    final hMask = size - 1;
    final hTable = Int32List(size);
    final hStamp = Int32List(size);
    final edgeStack = Int32List(n);
    final edgeStamp = Int32List(n);
    final he = (Int32List(n)..fillRange(0, n, -1));
    var i = 0;
    for (var e = 0; e < n; e += 1) {
      final a = t[e];
      final b = t[nextHE(e)];
      final lo = a < b ? a : b;
      final hi = a < b ? b : a;
      var h = (mul32(lo, 0x9e3779b1) ^ mul32(hi, 0x85ebca6b)) & hMask;
      while (hStamp[h] == 1) {
        final s = hTable[h];
        if (s != -1) {
          final sa = t[s];
          final sb = t[nextHE(s)];
          if (sa == lo && sb == hi || sa == hi && sb == lo) {
            he[e] = s;
            he[s] = e;
            hTable[h] = -1;
            edgeStamp[s] = 1;
            edgeStack[i] = s;
            i += 1;
            break;
          }
        }
        h = h + 1 & hMask;
      }
      if (hStamp[h] != 1) {
        hTable[h] = e;
        hStamp[h] = 1;
      }
    }
    while (i > 0) {
      i -= 1;
      final a = edgeStack[i];
      edgeStamp[a] = 0;
      final b = he[a];
      if (b == -1) {
        continue;
      }
      final a0 = a - a % 3;
      final b0 = b - b % 3;
      final ar = a0 + (a + 2) % 3;
      final al = a0 + (a + 1) % 3;
      final bl = b0 + (b + 2) % 3;
      final br = b0 + (b + 1) % 3;
      final p0 = t[ar];
      final pr = t[a];
      final pl = t[al];
      final p1 = t[bl];
      final x0 = coords[p0 * dim];
      final y0 = coords[p0 * dim + 1];
      final xr = coords[pr * dim];
      final yr = coords[pr * dim + 1];
      final xl = coords[pl * dim];
      final yl = coords[pl * dim + 1];
      final x1 = coords[p1 * dim];
      final y1 = coords[p1 * dim + 1];
      if (!inCircle(x0, y0, xr, yr, xl, yl, x1, y1) &&
          orient(x0, y0, xr, yr, x1, y1) > 0 &&
          orient(x0, y0, x1, y1, xl, yl) > 0) {
        t[a] = p1;
        t[b] = p0;
        final hbl = he[bl];
        final har = he[ar];
        he[a] = hbl;
        if (hbl != -1) {
          he[hbl] = a;
        }
        he[b] = har;
        if (har != -1) {
          he[har] = b;
        }
        he[ar] = bl;
        he[bl] = ar;
        if (hbl != -1 && edgeStamp[a] == 0) {
          edgeStamp[a] = 1;
          edgeStack[i] = a;
          i += 1;
        }
        if (har != -1 && edgeStamp[b] == 0) {
          edgeStamp[b] = 1;
          edgeStack[i] = b;
          i += 1;
        }
        if (he[al] != -1 && edgeStamp[al] == 0) {
          edgeStamp[al] = 1;
          edgeStack[i] = al;
          i += 1;
        }
        if (he[br] != -1 && edgeStamp[br] == 0) {
          edgeStamp[br] = 1;
          edgeStack[i] = br;
          i += 1;
        }
      }
    }
  }

  double orient(double ax, double ay, double bx, double by, double cx, double cy) {
    return (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
  }

  // The relative tolerance makes near-cocircular diagonals legal ties and prevents flip cycles.
  bool inCircle(
    double ax,
    double ay,
    double bx,
    double by,
    double cx,
    double cy,
    double px,
    double py,
  ) {
    final dx = ax - px;
    final dy = ay - py;
    final ex = bx - px;
    final ey = by - py;
    final fx = cx - px;
    final fy = cy - py;
    final ap = dx * dx + dy * dy;
    final bp = ex * ex + ey * ey;
    final cp = fx * fx + fy * fy;
    final s = ap + bp + cp;
    return dx * (ey * cp - bp * fy) - dy * (ex * cp - bp * fx) + ap * (ex * fy - ey * fx) <=
        1e-13 * s * s;
  }

  int nextHE(int e) {
    return e - e % 3 + (e + 1) % 3;
  }

  // Z-order uses ECMAScript ToInt32 even for holes far outside the outer bounds.
  int toInt32(double value) {
    if (!value.isFinite) return 0;
    if (value >= -2147483648.0 && value < 2147483648.0) return value.truncate();
    return value.remainder(4294967296.0).truncate().toSigned(32);
  }

  Vertex createVertex(int i, double x, double y) {
    final p = Vertex(i, x, y);
    p.prevZ = p.nextZ = none;
    return p;
  }

  // Dart's sort permits unstable ties. Use insertion order to resolve equal or NaN slopes.
  void sortHoles(List<Vertex> queue) {
    final order = {for (var i = 0; i < queue.length; i++) queue[i]: i};
    queue.sort((a, b) {
      final d = compareXYSlope(a, b);
      return d < 0
          ? -1
          : d > 0
          ? 1
          : order[a]! - order[b]!;
    });
  }

  // Keep both partial products within the exact integer range of a JS number.
  int mul32(int a, int b) => (a * (b & 65535) + ((a * (b >>> 16)) << 16)) & 0xffffffff;
}
