// pack/ (index.js, siblings.js, enclose.js), lcg.js and array.js's shuffle of d3-hierarchy 3.1.3.
package hierarchy

import scala.collection.mutable

// lcg.js: https://en.wikipedia.org/wiki/Linear_congruential_generator#Parameters_in_common_use
private val A = 1664525.0
private val C = 1013904223.0
private val M = 4294967296.0 // 2^32

/** lcg.js: a seeded generator of [0, 1). Every product stays below 2^53, so doubles are exact. */
private def lcg(): () => Double =
  var s = 1.0
  () =>
    s = (A * s + C) % M
    s / M

/** array.js's `shuffle`: Fisher-Yates from the end, with `random() * m-- | 0`. */
private def shuffle[T](array: mutable.ArrayBuffer[T], random: () => Double): mutable.ArrayBuffer[T] =
  var m = array.size
  while m != 0 do
    val i = (random() * m).toInt
    m -= 1
    val t = array(m)
    array(m) = array(i)
    array(i) = t
  array

/** A circle that `encloseBasis` computes. */
private final class Enclosure(val x: Double, val y: Double, val r: Double) extends Circle

/** `packEncloseRandom`: the smallest circle enclosing `circles`, by Welzl's algorithm. */
private def packEncloseRandom(circles: collection.Seq[Circle], random: () => Double): Circle =
  val shuffled = shuffle(mutable.ArrayBuffer.from(circles), random)
  val n = shuffled.size
  var i = 0
  var basis = List.empty[Circle]
  var e: Option[Circle] = None
  while i < n do
    val p = shuffled(i)
    if e.exists(enclosesWeak(_, p)) then i += 1
    else
      basis = extendBasis(basis, p)
      e = Some(encloseBasis(basis))
      i = 0
  e.get

private def extendBasis(b: List[Circle], p: Circle): List[Circle] =
  if enclosesWeakAll(p, b) then List(p)
  else
    // If we get here then B must have at least one element.
    b.find(x => enclosesNot(p, x) && enclosesWeakAll(encloseBasis2(x, p), b)) match
      case Some(x) => List(x, p)
      case None =>
        // If we get here then B must have at least two elements. The pairs come lazily, in d3's
        // order.
        val pairs = for (x, i) <- b.iterator.zipWithIndex; y <- b.iterator.drop(i + 1) yield (x, y)
        pairs.find((x, y) =>
          enclosesNot(encloseBasis2(x, y), p)
            && enclosesNot(encloseBasis2(x, p), y)
            && enclosesNot(encloseBasis2(y, p), x)
            && enclosesWeakAll(encloseBasis3(x, y, p), b)
        ) match
          case Some((x, y)) => List(x, y, p)
          // If we get here then something is very wrong.
          case None => throw IllegalStateException()

private def enclosesNot(a: Circle, b: Circle): Boolean =
  val dr = a.r - b.r
  val dx = b.x - a.x
  val dy = b.y - a.y
  dr < 0 || dr * dr < dx * dx + dy * dy

private def enclosesWeak(a: Circle, b: Circle): Boolean =
  val dr = a.r - b.r + math.max(math.max(a.r, b.r), 1.0) * 1e-9
  val dx = b.x - a.x
  val dy = b.y - a.y
  dr > 0 && dr * dr > dx * dx + dy * dy

private def enclosesWeakAll(a: Circle, b: List[Circle]): Boolean = b.forall(enclosesWeak(a, _))

private def encloseBasis(b: List[Circle]): Circle = b match
  case List(a) => Enclosure(a.x, a.y, a.r)
  case List(a, b) => encloseBasis2(a, b)
  case List(a, b, c) => encloseBasis3(a, b, c)
  case _ => throw IllegalStateException()

private def encloseBasis2(a: Circle, b: Circle): Circle =
  val x1 = a.x
  val y1 = a.y
  val r1 = a.r
  val x2 = b.x
  val y2 = b.y
  val r2 = b.r
  val x21 = x2 - x1
  val y21 = y2 - y1
  val r21 = r2 - r1
  val l = math.sqrt(x21 * x21 + y21 * y21)
  Enclosure((x1 + x2 + x21 / l * r21) / 2, (y1 + y2 + y21 / l * r21) / 2, (l + r1 + r2) / 2)

private def encloseBasis3(a: Circle, b: Circle, c: Circle): Circle =
  val x1 = a.x
  val y1 = a.y
  val r1 = a.r
  val x2 = b.x
  val y2 = b.y
  val r2 = b.r
  val x3 = c.x
  val y3 = c.y
  val r3 = c.r
  val a2 = x1 - x2
  val a3 = x1 - x3
  val b2 = y1 - y2
  val b3 = y1 - y3
  val c2 = r2 - r1
  val c3 = r3 - r1
  val d1 = x1 * x1 + y1 * y1 - r1 * r1
  val d2 = d1 - x2 * x2 - y2 * y2 + r2 * r2
  val d3 = d1 - x3 * x3 - y3 * y3 + r3 * r3
  val ab = a3 * b2 - a2 * b3
  val xa = (b2 * d3 - b3 * d2) / (ab * 2) - x1
  val xb = (b3 * c2 - b2 * c3) / ab
  val ya = (a3 * d2 - a2 * d3) / (ab * 2) - y1
  val yb = (a2 * c3 - a3 * c2) / ab
  val qa = xb * xb + yb * yb - 1
  val qb = 2 * (r1 + xa * xb + ya * yb)
  val qc = xa * xa + ya * ya - r1 * r1
  val r = -(if math.abs(qa) > 1e-6 then (qb + math.sqrt(qb * qb - 4 * qa * qc)) / (2 * qa) else qc / qb)
  Enclosure(x1 + xa + xb * r, y1 + ya + yb * r, r)

/** Places `c` tangent to `a` and `b`. */
private def place(b: Circle, a: Circle, c: Node): Unit =
  val dx = b.x - a.x
  val dy = b.y - a.y
  val d2 = dx * dx + dy * dy
  if truthy(d2) then
    var a2 = a.r + c.r
    a2 *= a2
    var b2 = b.r + c.r
    b2 *= b2
    if a2 > b2 then
      val x = (d2 + b2 - a2) / (2 * d2)
      val y = math.sqrt(math.max(0, b2 / d2 - x * x))
      c.x = b.x - x * dx - y * dy
      c.y = b.y - x * dy + y * dx
    else
      val x = (d2 + a2 - b2) / (2 * d2)
      val y = math.sqrt(math.max(0, a2 / d2 - x * x))
      c.x = a.x + x * dx - y * dy
      c.y = a.y + x * dy + y * dx
  else
    c.x = a.x + c.r
    c.y = a.y

private def intersects(a: Circle, b: Circle): Boolean =
  val dr = a.r + b.r - 1e-6
  val dx = b.x - a.x
  val dy = b.y - a.y
  dr > 0 && dr * dr > dx * dx + dy * dy

/** siblings.js's front-chain `Node`: a circle (`circle` is `_`) and its neighbours on the chain. */
private final class Link(val circle: Node):
  var next: Link = this
  var previous: Link = this

/** The squared distance from the origin to the weighted centre of `node` and its successor. */
private def score(node: Link): Double =
  val a = node.circle
  val b = node.next.circle
  val ab = a.r + b.r
  val dx = (a.x * b.r + b.x * a.r) / ab
  val dy = (a.y * b.r + b.y * a.r) / ab
  dx * dx + dy * dy

/** `packSiblingsRandom`: packs `circles` around the origin and returns the enclosing radius. */
private def packSiblingsRandom(circles: mutable.ArrayBuffer[Node], random: () => Double): Double =
  val n = circles.size
  if n == 0 then return 0

  // Place the first circle.
  val first = circles(0)
  first.x = 0
  first.y = 0
  if n <= 1 then return first.r

  // Place the second circle.
  val second = circles(1)
  first.x = -second.r
  second.x = first.r
  second.y = 0
  if n <= 2 then return first.r + second.r

  // Place the third circle.
  place(second, first, circles(2))

  // Initialize the front-chain using the first three circles a, b and c.
  var a = Link(first)
  var b = Link(second)
  var c = Link(circles(2))
  a.next = b
  c.previous = b
  b.next = c
  a.previous = c
  c.next = a
  b.previous = a

  // Attempt to place each remaining circle…
  var i = 3
  while i < n do
    place(a.circle, b.circle, circles(i))
    c = Link(circles(i))

    // Find the closest intersecting circle on the front-chain, if any. "Closeness" is determined
    // by linear distance along the front-chain, and so are "ahead" and "behind".
    var j = b.next
    var k = a.previous
    var sj = b.circle.r
    var sk = a.circle.r
    var retry = false
    while
      if sj <= sk then
        if intersects(j.circle, c.circle) then
          b = j
          a.next = b
          b.previous = a
          retry = true
        else
          sj += j.circle.r
          j = j.next
      else if intersects(k.circle, c.circle) then
        a = k
        a.next = b
        b.previous = a
        retry = true
      else
        sk += k.circle.r
        k = k.previous
      !retry && (j ne k.next)
    do ()

    // On an intersection, circle i is placed again between the new a and b.
    if !retry then
      // Success! Insert the new circle c between a and b.
      c.previous = a
      c.next = b
      a.next = c
      b.previous = c
      b = c

      // Compute the new closest circle pair to the centroid.
      var aa = score(a)
      c = c.next
      while c ne b do
        val ca = score(c)
        if ca < aa then
          a = c
          aa = ca
        c = c.next
      b = a.next
      i += 1

  // Compute the enclosing circle of the front chain.
  val front = mutable.ArrayBuffer[Circle](b.circle)
  c = b.next
  while c ne b do
    front += c.circle
    c = c.next
  val e = packEncloseRandom(front, random)

  // Translate the circles to put the enclosing circle around the origin.
  for circle <- circles do
    circle.x -= e.x
    circle.y -= e.y
  e.r

/** `radiusLeaf(defaultRadius)`: a leaf's radius is the square root of its value. */
private def radiusLeaf(node: Node): Unit =
  if node.children.isEmpty then
    val r = math.sqrt(node.value)
    node.r = math.max(0, if truthy(r) then r else 0.0)

/** `packChildrenRandom(padding, k, random)` with a constant padding. */
private def packChildrenRandom(node: Node, padding: Double, k: Double, random: () => Double): Unit =
  for children <- node.children do
    val scaled = padding * k
    val r = if truthy(scaled) then scaled else 0.0
    if truthy(r) then for child <- children do child.r += r
    val e = packSiblingsRandom(children, random)
    if truthy(r) then for child <- children do child.r -= r
    node.r = e + r

/** `translateChild(k)`. */
private def translateChild(node: Node, k: Double): Unit =
  node.r *= k
  for parent <- node.parent do
    node.x = parent.x + k * node.x
    node.y = parent.y + k * node.y

/** `pack().size((dx, dy)).padding(padding)`: one generator serves both packing passes. */
def pack(root: Node, dx: Double, dy: Double, padding: Double): Node =
  val random = lcg()
  root.x = dx / 2
  root.y = dy / 2
  root.eachBefore(radiusLeaf).eachAfter(packChildrenRandom(_, 0, 1, random))
  val k = root.r / math.min(dx, dy)
  root.eachAfter(packChildrenRandom(_, padding, k, random))
  val scale = math.min(dx, dy) / (2 * root.r)
  root.eachBefore(translateChild(_, scale))
  root
