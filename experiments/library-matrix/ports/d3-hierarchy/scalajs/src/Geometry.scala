package hierarchy

// d3-hierarchy 3.1.3 layout algorithms. Child and contour links are arena indices.
final class Node {
  var id: String = ""
  var kids: Array[Int] = Array.empty[Int]
  var parent: Int = -1
  var depth: Int = 0
  var height: Int = 0
  var number: Int = 0
  var ancestor: Int = 0
  var defaultAncestor: Int = -1
  var thread: Int = -1
  var value: Double = 0.0
  var x: Double = 0.0
  var y: Double = 0.0
  var r: Double = 0.0
  var x0: Double = 0.0
  var y0: Double = 0.0
  var x1: Double = 0.0
  var y1: Double = 0.0
  var z: Double = 0.0
  var m: Double = 0.0
  var c: Double = 0.0
  var s: Double = 0.0
}

object Geometry {
def real(x: Int): Double = x.toDouble
def integer(x: Double): Int = x.toInt
def minf(a: Double, b: Double): Double = math.min(a,b)
def maxf(a: Double, b: Double): Double = math.max(a,b)
def sqrtf(x: Double): Double = math.sqrt(x)
def absf(x: Double): Double = math.abs(x)
def floorf(x: Double): Double = math.floor(x)
def roundf(x: Double): Double = if (x >= -0.5 && x < 0) -0.0 else math.floor(x+0.5)
def ints(n: Int): Array[Int] = new Array[Int](n)
def floats(n: Int): Array[Double] = new Array[Double](n)
def invalidBasis(): Int = throw new IllegalArgumentException("invalid enclosing basis")
def circle(x: Double, y: Double, r: Double): Node = { val n = new Node; n.x=x; n.y=y; n.r=r; n }

  def separation(nodes: Array[Node], a: Int, b: Int): Double = {
    return (if ((nodes(a).parent == nodes(b).parent)) 1.0 else 2.0)
  }

  def contour(nodes: Array[Node], v: Int, left: Boolean): Int = {
    val kids = nodes(v).kids
    if ((kids.length == 0)) {
      return nodes(v).thread
    }
    return (if (left) kids(0) else kids((kids.length - 1)))
  }

  def apportion(nodes: Array[Node], v: Int, previous: Int, fallback: Int): Int = {
    var ancestor = fallback
    if ((previous < 0)) {
      return ancestor
    }
    var vip = v
    var vop = v
    var vim = previous
    var vom = nodes(nodes(v).parent).kids(0)
    var sip = nodes(vip).m
    var sop = nodes(vop).m
    var sim = nodes(vim).m
    var som = nodes(vom).m
    vim = contour(nodes, vim, false)
    vip = contour(nodes, vip, true)
    while (((vim >= 0) && (vip >= 0))) {
      vom = contour(nodes, vom, true)
      vop = contour(nodes, vop, false)
      nodes(vop).ancestor = v
      val shift = ((((nodes(vim).z + sim) - nodes(vip).z) - sip) + separation(nodes, vim, vip))
      if ((shift > 0.0)) {
        val anc = nodes(vim).ancestor
        val wm = (if ((nodes(anc).parent == nodes(v).parent)) anc else ancestor)
        val change = (shift / real((nodes(v).number - nodes(wm).number)))
        nodes(v).c = (nodes(v).c - change)
        nodes(v).s = (nodes(v).s + shift)
        nodes(wm).c = (nodes(wm).c + change)
        nodes(v).z = (nodes(v).z + shift)
        nodes(v).m = (nodes(v).m + shift)
        sip = (sip + shift)
        sop = (sop + shift)
      }
      sim = (sim + nodes(vim).m)
      sip = (sip + nodes(vip).m)
      som = (som + nodes(vom).m)
      sop = (sop + nodes(vop).m)
      vim = contour(nodes, vim, false)
      vip = contour(nodes, vip, true)
    }
    if (((vim >= 0) && (contour(nodes, vop, false) < 0))) {
      nodes(vop).thread = vim
      nodes(vop).m = (nodes(vop).m + (sim - sop))
    }
    if (((vip >= 0) && (contour(nodes, vom, true) < 0))) {
      nodes(vom).thread = vip
      nodes(vom).m = (nodes(vom).m + (sip - som))
      ancestor = v
    }
    return ancestor
  }

  def tidy(nodes: Array[Node], pre: Array[Int], post: Array[Int], w: Double, h: Double, fixed: Boolean): Unit = {
    val root = pre(0)
    for (v <- pre) {
      nodes(v).ancestor = v
      nodes(v).thread = (-1)
      nodes(v).defaultAncestor = (-1)
      val kids = nodes(v).kids
      for (i <- 0 until kids.length) {
        nodes(kids(i)).number = i
      }
    }
    for (v <- post) {
      val n = nodes(v)
      val p = n.parent
      var prev = (-1)
      if ((n.number > 0)) {
        prev = nodes(p).kids((n.number - 1))
      }
      val kids = n.kids
      if ((kids.length > 0)) {
        var shift = 0.0
        var change = 0.0
        for (i <- (0 until kids.length).reverse) {
          val child = nodes(kids(i))
          child.z = (child.z + shift)
          child.m = (child.m + shift)
          change = (change + child.c)
          shift = (shift + (child.s + change))
        }
        val mid = ((nodes(kids(0)).z + nodes(kids((kids.length - 1))).z) / 2.0)
        if ((prev < 0)) {
          n.z = mid
        } else {
          n.z = (nodes(prev).z + separation(nodes, v, prev))
          n.m = (n.z - mid)
        }
      } else {
        if ((prev >= 0)) {
          n.z = (nodes(prev).z + separation(nodes, v, prev))
        }
      }
      if ((p >= 0)) {
        var ancestor = nodes(p).defaultAncestor
        if ((ancestor < 0)) {
          ancestor = nodes(p).kids(0)
        }
        nodes(p).defaultAncestor = apportion(nodes, v, prev, ancestor)
      }
    }
    val offset = (-nodes(root).z)
    var left = root
    var right = root
    var depth = 0
    for (v <- pre) {
      val n = nodes(v)
      val modifier = (if ((v == root)) offset else nodes(n.parent).m)
      n.x = (n.z + modifier)
      n.m = (n.m + modifier)
      if ((n.x < nodes(left).x)) {
        left = v
      }
      if ((n.x > nodes(right).x)) {
        right = v
      }
      if ((n.depth > depth)) {
        depth = n.depth
      }
    }
    val margin = (if ((left == right)) 1.0 else (separation(nodes, left, right) / 2.0))
    val tx = (margin - nodes(left).x)
    val kx = (w / ((nodes(right).x + margin) + tx))
    val ky = (h / real((if ((depth == 0)) 1 else depth)))
    for (v <- pre) {
      val n = nodes(v)
      if (fixed) {
        n.x = (n.x * w)
        n.y = (real(n.depth) * h)
      } else {
        n.x = ((n.x + tx) * kx)
        n.y = (real(n.depth) * ky)
      }
    }
  }

  def cluster(nodes: Array[Node], pre: Array[Int], post: Array[Int], w: Double, h: Double, fixed: Boolean): Unit = {
    var previous = (-1)
    var x = 0.0
    for (v <- post) {
      val n = nodes(v)
      if ((n.kids.length == 0)) {
        if ((previous >= 0)) {
          x = (x + separation(nodes, v, previous))
        }
        n.x = x
        previous = v
      } else {
        var height = 0.0
        var total = 0.0
        for (c <- n.kids) {
          total = (total + nodes(c).x)
          height = maxf(height, nodes(c).y)
        }
        n.x = (total / real(n.kids.length))
        n.y = (1.0 + height)
      }
    }
    val left = post(0)
    val right = previous
    val x0 = (nodes(left).x - (separation(nodes, left, right) / 2.0))
    val x1 = (nodes(right).x + (separation(nodes, right, left) / 2.0))
    val rx = nodes(pre(0)).x
    val ry = nodes(pre(0)).y
    for (v <- post) {
      val n = nodes(v)
      if (fixed) {
        n.x = ((n.x - rx) * w)
        n.y = ((ry - n.y) * h)
      } else {
        n.x = (((n.x - x0) / (x1 - x0)) * w)
        n.y = ((1.0 - (if ((ry != 0.0)) (n.y / ry) else 1.0)) * h)
      }
    }
  }

  def tileSlice(nodes: Array[Node], kids: Array[Int], start: Int, stop: Int, total: Double, x0: Double, y0: Double, x1: Double, y1: Double, dice: Boolean): Unit = {
    var x = x0
    var y = y0
    val scale = (if ((total == 0.0)) total else (if (dice) ((x1 - x) / total) else ((y1 - y) / total)))
    for (i <- start until stop) {
      val n = nodes(kids(i))
      n.x0 = x
      n.y0 = y
      n.x1 = x1
      n.y1 = y1
      if (dice) {
        x = (x + (n.value * scale))
        n.x1 = x
      } else {
        y = (y + (n.value * scale))
        n.y1 = y
      }
    }
  }

  def binary(nodes: Array[Node], kids: Array[Int], sums: Array[Double], start: Int, stop: Int, total: Double, x0: Double, y0: Double, x1: Double, y1: Double): Unit = {
    if ((start >= (stop - 1))) {
      val n = nodes(kids(start))
      n.x0 = x0
      n.y0 = y0
      n.x1 = x1
      n.y1 = y1
    } else {
      val off = sums(start)
      val target = ((total / 2.0) + off)
      var k = (start + 1)
      var hi = (stop - 1)
      while ((k < hi)) {
        val mid = ((k + hi) / 2)
        if ((sums(mid) < target)) {
          k = (mid + 1)
        } else {
          hi = mid
        }
      }
      if ((((target - sums((k - 1))) < (sums(k) - target)) && ((start + 1) < k))) {
        k = (k - 1)
      }
      val left = (sums(k) - off)
      val right = (total - left)
      if (((x1 - x0) > (y1 - y0))) {
        val x = (if ((total != 0.0)) (((x0 * right) + (x1 * left)) / total) else x1)
        binary(nodes, kids, sums, start, k, left, x0, y0, x, y1)
        binary(nodes, kids, sums, k, stop, right, x, y0, x1, y1)
      } else {
        val y = (if ((total != 0.0)) (((y0 * right) + (y1 * left)) / total) else y1)
        binary(nodes, kids, sums, start, k, left, x0, y0, x1, y)
        binary(nodes, kids, sums, k, stop, right, x0, y, x1, y1)
      }
    }
  }

  def squarify(nodes: Array[Node], kids: Array[Int], value: Double, left: Double, top: Double, x1: Double, y1: Double): Unit = {
    var total = value
    var x = left
    var y = top
    var i = 0
    var j = 0
    val ratio = ((1.0 + sqrtf(5.0)) / 2.0)
    while ((i < kids.length)) {
      val dx = (x1 - x)
      val dy = (y1 - y)
      var row = nodes(kids(j)).value
      j = (j + 1)
      while (((row == 0.0) && (j < kids.length))) {
        row = nodes(kids(j)).value
        j = (j + 1)
      }
      var low = row
      var high = row
      val alpha = (maxf((dy / dx), (dx / dy)) / (total * ratio))
      var beta = ((row * row) * alpha)
      var best = maxf((high / beta), (beta / low))
      var finished = false
      while (((j < kids.length) && (!finished))) {
        val v = nodes(kids(j)).value
        row = (row + v)
        low = minf(low, v)
        high = maxf(high, v)
        beta = ((row * row) * alpha)
        val score = maxf((high / beta), (beta / low))
        if ((score > best)) {
          row = (row - v)
          finished = true
        } else {
          best = score
          j = (j + 1)
        }
      }
      if ((dx < dy)) {
        val bottom = (if ((total != 0.0)) (y + ((dy * row) / total)) else y1)
        tileSlice(nodes, kids, i, j, row, x, y, x1, bottom, true)
        if ((total != 0.0)) {
          y = bottom
        }
      } else {
        val right = (if ((total != 0.0)) (x + ((dx * row) / total)) else x1)
        tileSlice(nodes, kids, i, j, row, x, y, right, y1, false)
        if ((total != 0.0)) {
          x = right
        }
      }
      total = (total - row)
      i = j
    }
  }

  def rectangles(nodes: Array[Node], pre: Array[Int], w: Double, h: Double, p: Double, partition: Boolean, tile: String, rounded: Boolean): Unit = {
    val root = nodes(pre(0))
    val levels = real((root.height + 1))
    root.x1 = w
    root.y1 = h
    if (partition) {
      root.x0 = p
      root.y0 = p
      root.y1 = (h / levels)
    }
    for (v <- pre) {
      val n = nodes(v)
      val kids = n.kids
      val inset = (if ((n.parent >= 0)) (p / 2.0) else 0.0)
      if (partition) {
        if ((kids.length > 0)) {
          tileSlice(nodes, kids, 0, kids.length, n.value, n.x0, ((h * real((n.depth + 1))) / levels), n.x1, ((h * real((n.depth + 2))) / levels), true)
        }
        n.x1 = (n.x1 - p)
        n.y1 = (n.y1 - p)
      } else {
        n.x0 = (n.x0 + inset)
        n.y0 = (n.y0 + inset)
        n.x1 = (n.x1 - inset)
        n.y1 = (n.y1 - inset)
      }
      if ((n.x1 < n.x0)) {
        n.x0 = ((n.x0 + n.x1) / 2.0)
        n.x1 = n.x0
      }
      if ((n.y1 < n.y0)) {
        n.y0 = ((n.y0 + n.y1) / 2.0)
        n.y1 = n.y0
      }
      if (((!partition) && (kids.length > 0))) {
        val gap = (p - (p / 2.0))
        var x = (n.x0 + gap)
        var y = (n.y0 + gap)
        var xx = (n.x1 - gap)
        var yy = (n.y1 - gap)
        if ((xx < x)) {
          x = ((x + xx) / 2.0)
          xx = x
        }
        if ((yy < y)) {
          y = ((y + yy) / 2.0)
          yy = y
        }
        if ((tile == "binary")) {
          val sums = floats((kids.length + 1))
          for (j <- 0 until kids.length) {
            sums((j + 1)) = (sums(j) + nodes(kids(j)).value)
          }
          binary(nodes, kids, sums, 0, kids.length, n.value, x, y, xx, yy)
        } else {
          if ((tile == "sliceDice")) {
            tileSlice(nodes, kids, 0, kids.length, n.value, x, y, xx, yy, ((n.depth % 2) == 0))
          } else {
            squarify(nodes, kids, n.value, x, y, xx, yy)
          }
        }
      }
    }
    if (rounded) {
      for (v <- pre) {
        val n = nodes(v)
        n.x0 = roundf(n.x0)
        n.x1 = roundf(n.x1)
        n.y0 = roundf(n.y0)
        n.y1 = roundf(n.y1)
      }
    }
  }

  def weak(a: Node, b: Node): Boolean = {
    val dr = ((a.r - b.r) + (maxf(maxf(a.r, b.r), 1.0) * 1e-09))
    val dx = (b.x - a.x)
    val dy = (b.y - a.y)
    return ((dr > 0.0) && ((dr * dr) > ((dx * dx) + (dy * dy))))
  }

  def outside(a: Node, b: Node): Boolean = {
    val dr = (a.r - b.r)
    val dx = (b.x - a.x)
    val dy = (b.y - a.y)
    return ((dr < 0.0) || ((dr * dr) < ((dx * dx) + (dy * dy))))
  }

  def pair(a: Node, b: Node): Node = {
    val dx = (b.x - a.x)
    val dy = (b.y - a.y)
    val dr = (b.r - a.r)
    val distance = sqrtf(((dx * dx) + (dy * dy)))
    return circle((((a.x + b.x) + ((dx / distance) * dr)) / 2.0), (((a.y + b.y) + ((dy / distance) * dr)) / 2.0), (((distance + a.r) + b.r) / 2.0))
  }

  def triple(a: Node, b: Node, c: Node): Node = {
    val a2 = (a.x - b.x)
    val a3 = (a.x - c.x)
    val b2 = (a.y - b.y)
    val b3 = (a.y - c.y)
    val c2 = (b.r - a.r)
    val c3 = (c.r - a.r)
    val d1 = (((a.x * a.x) + (a.y * a.y)) - (a.r * a.r))
    val d2 = (((d1 - (b.x * b.x)) - (b.y * b.y)) + (b.r * b.r))
    val d3 = (((d1 - (c.x * c.x)) - (c.y * c.y)) + (c.r * c.r))
    val ab = ((a3 * b2) - (a2 * b3))
    val xa = ((((b2 * d3) - (b3 * d2)) / (ab * 2.0)) - a.x)
    val xb = (((b3 * c2) - (b2 * c3)) / ab)
    val ya = ((((a3 * d2) - (a2 * d3)) / (ab * 2.0)) - a.y)
    val yb = (((a2 * c3) - (a3 * c2)) / ab)
    val qa = (((xb * xb) + (yb * yb)) - 1.0)
    val qb = (2.0 * ((a.r + (xa * xb)) + (ya * yb)))
    val qc = (((xa * xa) + (ya * ya)) - (a.r * a.r))
    val r = (-(if ((absf(qa) > 1e-06)) ((qb + sqrtf(((qb * qb) - ((4.0 * qa) * qc)))) / (2.0 * qa)) else (qc / qb)))
    return circle(((a.x + xa) + (xb * r)), ((a.y + ya) + (yb * r)), r)
  }

  def covers(nodes: Array[Node], basis: Array[Int], count: Int, a: Node): Boolean = {
    var i = 0
    while (((i < count) && weak(a, nodes(basis(i))))) {
      i = (i + 1)
    }
    return (i == count)
  }

  def extend(nodes: Array[Node], basis: Array[Int], count: Int, p: Int): Int = {
    if (covers(nodes, basis, count, nodes(p))) {
      basis(0) = p
      return 1
    }
    var i = 0
    while (i < count) {
      val x = basis(i)
      if (outside(nodes(p), nodes(x)) && covers(nodes, basis, count, pair(nodes(x), nodes(p)))) {
        basis(0) = x
        basis(1) = p
        return 2
      }
      i += 1
    }
    i = 0
    while (i < count) {
      val x = basis(i)
      var j = i + 1
      while (j < count) {
        val y = basis(j)
        if (outside(pair(nodes(x), nodes(y)), nodes(p)) && outside(pair(nodes(x), nodes(p)), nodes(y)) &&
            outside(pair(nodes(y), nodes(p)), nodes(x)) && covers(nodes, basis, count, triple(nodes(x), nodes(y), nodes(p)))) {
          basis(0) = x
          basis(1) = y
          basis(2) = p
          return 3
        }
        j += 1
      }
      i += 1
    }
    return invalidBasis()
  }

  def enclose(nodes: Array[Node], front: Array[Int], n: Int, seed: Array[Double]): Node = {
    for (j <- (0 until n).reverse) {
      val step = ((1664525.0 * seed(0)) + 1013904223.0)
      seed(0) = (step - (floorf((step / 4294967296.0)) * 4294967296.0))
      val k = integer(((seed(0) / 4294967296.0) * real((j + 1))))
      val save = front(j)
      front(j) = front(k)
      front(k) = save
    }
    val basis = ints(3)
    var count = 0
    var i = 0
    var result = circle(0.0, 0.0, 0.0)
    while ((i < n)) {
      val p = front(i)
      if (((count > 0) && weak(result, nodes(p)))) {
        i = (i + 1)
      } else {
        count = extend(nodes, basis, count, p)
        if ((count == 1)) {
          result = circle(nodes(p).x, nodes(p).y, nodes(p).r)
        } else {
          if ((count == 2)) {
            result = pair(nodes(basis(0)), nodes(basis(1)))
          } else {
            result = triple(nodes(basis(0)), nodes(basis(1)), nodes(basis(2)))
          }
        }
        i = 0
      }
    }
    return result
  }

  def place(b: Node, a: Node, c: Node): Unit = {
    val dx = (b.x - a.x)
    val dy = (b.y - a.y)
    val d2 = ((dx * dx) + (dy * dy))
    if ((d2 != 0.0)) {
      val a2 = ((a.r + c.r) * (a.r + c.r))
      val b2 = ((b.r + c.r) * (b.r + c.r))
      if ((a2 > b2)) {
        val x = (((d2 + b2) - a2) / (2.0 * d2))
        val y = sqrtf(maxf(0.0, ((b2 / d2) - (x * x))))
        c.x = ((b.x - (x * dx)) - (y * dy))
        c.y = ((b.y - (x * dy)) + (y * dx))
      } else {
        val x = (((d2 + a2) - b2) / (2.0 * d2))
        val y = sqrtf(maxf(0.0, ((a2 / d2) - (x * x))))
        c.x = ((a.x + (x * dx)) - (y * dy))
        c.y = ((a.y + (x * dy)) + (y * dx))
      }
    } else {
      c.x = (a.x + c.r)
      c.y = a.y
    }
  }

  def intersects(a: Node, b: Node): Boolean = {
    val dr = ((a.r + b.r) - 1e-06)
    val dx = (b.x - a.x)
    val dy = (b.y - a.y)
    return ((dr > 0.0) && ((dr * dr) > ((dx * dx) + (dy * dy))))
  }

  def score(a: Node, b: Node): Double = {
    val ab = (a.r + b.r)
    val dx = (((a.x * b.r) + (b.x * a.r)) / ab)
    val dy = (((a.y * b.r) + (b.y * a.r)) / ab)
    return ((dx * dx) + (dy * dy))
  }

  def siblings(nodes: Array[Node], kids: Array[Int], seed: Array[Double]): Double = {
    val n = kids.length
    var a = 0
    var b = 1
    nodes(kids(0)).x = 0.0
    nodes(kids(0)).y = 0.0
    if ((n == 1)) {
      return nodes(kids(0)).r
    }
    nodes(kids(0)).x = (-nodes(kids(1)).r)
    nodes(kids(1)).x = nodes(kids(0)).r
    nodes(kids(1)).y = 0.0
    if ((n == 2)) {
      return (nodes(kids(0)).r + nodes(kids(1)).r)
    }
    place(nodes(kids(1)), nodes(kids(0)), nodes(kids(2)))
    val after = ints(n)
    val before = ints(n)
    after(0) = 1
    before(2) = 1
    after(1) = 2
    before(0) = 2
    after(2) = 0
    before(1) = 0
    var i = 3
    while ((i < n)) {
      place(nodes(kids(a)), nodes(kids(b)), nodes(kids(i)))
      var j = after(b)
      var k = before(a)
      var sj = nodes(kids(b)).r
      var sk = nodes(kids(a)).r
      var retry = false
      var more = true
      while ((more && (!retry))) {
        if ((sj <= sk)) {
          if (intersects(nodes(kids(j)), nodes(kids(i)))) {
            b = j
            after(a) = b
            before(b) = a
            retry = true
          } else {
            sj = (sj + nodes(kids(j)).r)
            j = after(j)
          }
        } else {
          if (intersects(nodes(kids(k)), nodes(kids(i)))) {
            a = k
            after(a) = b
            before(b) = a
            retry = true
          } else {
            sk = (sk + nodes(kids(k)).r)
            k = before(k)
          }
        }
        more = (j != after(k))
      }
      if ((!retry)) {
        before(i) = a
        after(i) = b
        after(a) = i
        before(b) = i
        b = i
        var best = score(nodes(kids(a)), nodes(kids(after(a))))
        j = after(i)
        while ((j != b)) {
          val candidateScore = score(nodes(kids(j)), nodes(kids(after(j))))
          if ((candidateScore < best)) {
            best = candidateScore
            a = j
          }
          j = after(j)
        }
        b = after(a)
        i = (i + 1)
      }
    }
    val front = ints(n)
    front(0) = kids(b)
    var count = 1
    var cursor = after(b)
    while ((cursor != b)) {
      front(count) = kids(cursor)
      count = (count + 1)
      cursor = after(cursor)
    }
    val enclosure = enclose(nodes, front, count, seed)
    for (id <- kids) {
      nodes(id).x = (nodes(id).x - enclosure.x)
      nodes(id).y = (nodes(id).y - enclosure.y)
    }
    return enclosure.r
  }

  def packChildren(nodes: Array[Node], post: Array[Int], padding: Double, seed: Array[Double]): Unit = {
    val pad = (if ((padding != padding)) 0.0 else padding)
    for (i <- post) {
      val kids = nodes(i).kids
      if ((kids.length > 0)) {
        if ((pad != 0.0)) {
          for (k <- kids) {
            nodes(k).r = (nodes(k).r + pad)
          }
        }
        val radius = siblings(nodes, kids, seed)
        if ((pad != 0.0)) {
          for (k <- kids) {
            nodes(k).r = (nodes(k).r - pad)
          }
        }
        nodes(i).r = (radius + pad)
      }
    }
  }

  def pack(nodes: Array[Node], pre: Array[Int], post: Array[Int], w: Double, h: Double, p: Double): Unit = {
    val root = nodes(pre(0))
    root.x = (w / 2.0)
    root.y = (h / 2.0)
    for (i <- pre) {
      val n = nodes(i)
      if ((n.kids.length == 0)) {
        val raw = sqrtf(n.value)
        n.r = (if ((raw == raw)) maxf(0.0, raw) else 0.0)
      }
    }
    val seed = floats(1)
    seed(0) = 1.0
    packChildren(nodes, post, 0.0, seed)
    packChildren(nodes, post, (p * (root.r / minf(w, h))), seed)
    val k = (minf(w, h) / (2.0 * root.r))
    for (i <- pre) {
      val n = nodes(i)
      n.r = (n.r * k)
      if ((n.parent >= 0)) {
        n.x = (nodes(n.parent).x + (k * n.x))
        n.y = (nodes(n.parent).y + (k * n.y))
      }
    }
  }

}
