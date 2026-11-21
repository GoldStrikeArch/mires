// pack/ (index.js, siblings.js, enclose.js), lcg.js and array.js's shuffle of d3-hierarchy 3.1.3.
package hierarchy

import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sqrt

// lcg.js: https://en.wikipedia.org/wiki/Linear_congruential_generator#Parameters_in_common_use
private const val A = 1664525.0
private const val C = 1013904223.0
private const val M = 4294967296.0 // 2^32

/** lcg.js: a seeded generator of [0, 1). Every product stays below 2^53, so doubles are exact. */
private fun lcg(): () -> Double {
    var s = 1.0
    return {
        s = (A * s + C) % M
        s / M
    }
}

/** array.js's `shuffle`: Fisher-Yates from the end, with `random() * m-- | 0`. */
private fun <T> shuffle(array: MutableList<T>, random: () -> Double): MutableList<T> {
    var m = array.size
    while (m != 0) {
        val i = (random() * m--).toInt()
        val t = array[m]
        array[m] = array[i]
        array[i] = t
    }
    return array
}

/** A circle that `encloseBasis` computes. */
private class Enclosure(override val x: Double, override val y: Double, override val r: Double) : Circle

/** `packEncloseRandom`: the smallest circle enclosing [circles], by Welzl's algorithm. */
private fun packEncloseRandom(circles: List<Circle>, random: () -> Double): Circle {
    val shuffled = shuffle(circles.toMutableList(), random)
    val n = shuffled.size
    var i = 0
    var basis = emptyList<Circle>()
    var e: Circle? = null
    while (i < n) {
        val p = shuffled[i]
        if (e != null && enclosesWeak(e, p)) {
            ++i
        } else {
            basis = extendBasis(basis, p)
            e = encloseBasis(basis)
            i = 0
        }
    }
    return checkNotNull(e)
}

private fun extendBasis(b: List<Circle>, p: Circle): List<Circle> {
    if (enclosesWeakAll(p, b)) return listOf(p)

    // If we get here then B must have at least one element.
    for (i in b.indices) {
        if (enclosesNot(p, b[i]) && enclosesWeakAll(encloseBasis2(b[i], p), b)) return listOf(b[i], p)
    }

    // If we get here then B must have at least two elements.
    for (i in 0 until b.size - 1) {
        for (j in i + 1 until b.size) {
            if (enclosesNot(encloseBasis2(b[i], b[j]), p) &&
                enclosesNot(encloseBasis2(b[i], p), b[j]) &&
                enclosesNot(encloseBasis2(b[j], p), b[i]) &&
                enclosesWeakAll(encloseBasis3(b[i], b[j], p), b)
            ) {
                return listOf(b[i], b[j], p)
            }
        }
    }

    // If we get here then something is very wrong.
    throw IllegalStateException()
}

private fun enclosesNot(a: Circle, b: Circle): Boolean {
    val dr = a.r - b.r
    val dx = b.x - a.x
    val dy = b.y - a.y
    return dr < 0 || dr * dr < dx * dx + dy * dy
}

private fun enclosesWeak(a: Circle, b: Circle): Boolean {
    val dr = a.r - b.r + max(max(a.r, b.r), 1.0) * 1e-9
    val dx = b.x - a.x
    val dy = b.y - a.y
    return dr > 0 && dr * dr > dx * dx + dy * dy
}

private fun enclosesWeakAll(a: Circle, b: List<Circle>): Boolean = b.all { enclosesWeak(a, it) }

private fun encloseBasis(b: List<Circle>): Circle = when (b.size) {
    1 -> Enclosure(b[0].x, b[0].y, b[0].r)
    2 -> encloseBasis2(b[0], b[1])
    else -> encloseBasis3(b[0], b[1], b[2])
}

private fun encloseBasis2(a: Circle, b: Circle): Circle {
    val x1 = a.x
    val y1 = a.y
    val r1 = a.r
    val x2 = b.x
    val y2 = b.y
    val r2 = b.r
    val x21 = x2 - x1
    val y21 = y2 - y1
    val r21 = r2 - r1
    val l = sqrt(x21 * x21 + y21 * y21)
    return Enclosure((x1 + x2 + x21 / l * r21) / 2, (y1 + y2 + y21 / l * r21) / 2, (l + r1 + r2) / 2)
}

private fun encloseBasis3(a: Circle, b: Circle, c: Circle): Circle {
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
    val r = -(if (abs(qa) > 1e-6) (qb + sqrt(qb * qb - 4 * qa * qc)) / (2 * qa) else qc / qb)
    return Enclosure(x1 + xa + xb * r, y1 + ya + yb * r, r)
}

/** Places [c] tangent to [a] and [b]. */
private fun place(b: Circle, a: Circle, c: Node) {
    val dx = b.x - a.x
    val dy = b.y - a.y
    val d2 = dx * dx + dy * dy
    if (truthy(d2)) {
        var a2 = a.r + c.r
        a2 *= a2
        var b2 = b.r + c.r
        b2 *= b2
        if (a2 > b2) {
            val x = (d2 + b2 - a2) / (2 * d2)
            val y = sqrt(max(0.0, b2 / d2 - x * x))
            c.x = b.x - x * dx - y * dy
            c.y = b.y - x * dy + y * dx
        } else {
            val x = (d2 + a2 - b2) / (2 * d2)
            val y = sqrt(max(0.0, a2 / d2 - x * x))
            c.x = a.x + x * dx - y * dy
            c.y = a.y + x * dy + y * dx
        }
    } else {
        c.x = a.x + c.r
        c.y = a.y
    }
}

private fun intersects(a: Circle, b: Circle): Boolean {
    val dr = a.r + b.r - 1e-6
    val dx = b.x - a.x
    val dy = b.y - a.y
    return dr > 0 && dr * dr > dx * dx + dy * dy
}

/** siblings.js's front-chain `Node`: a circle ([circle] is `_`) and its neighbours on the chain. */
private class Link(val circle: Node) {
    lateinit var next: Link
    lateinit var previous: Link
}

/** The squared distance from the origin to the weighted centre of [node] and its successor. */
private fun score(node: Link): Double {
    val a = node.circle
    val b = node.next.circle
    val ab = a.r + b.r
    val dx = (a.x * b.r + b.x * a.r) / ab
    val dy = (a.y * b.r + b.y * a.r) / ab
    return dx * dx + dy * dy
}

/** `packSiblingsRandom`: packs [circles] around the origin and returns the enclosing radius. */
private fun packSiblingsRandom(circles: List<Node>, random: () -> Double): Double {
    val n = circles.size
    if (n == 0) return 0.0

    // Place the first circle.
    val first = circles[0]
    first.x = 0.0
    first.y = 0.0
    if (n <= 1) return first.r

    // Place the second circle.
    val second = circles[1]
    first.x = -second.r
    second.x = first.r
    second.y = 0.0
    if (n <= 2) return first.r + second.r

    // Place the third circle.
    place(second, first, circles[2])

    // Initialize the front-chain using the first three circles a, b and c.
    var a = Link(first)
    var b = Link(second)
    var c = Link(circles[2])
    a.next = b
    c.previous = b
    b.next = c
    a.previous = c
    c.next = a
    b.previous = a

    // Attempt to place each remaining circle…
    var i = 3
    pack@ while (i < n) {
        place(a.circle, b.circle, circles[i])
        c = Link(circles[i])

        // Find the closest intersecting circle on the front-chain, if any. "Closeness" is
        // determined by linear distance along the front-chain, and so are "ahead" and "behind".
        var j = b.next
        var k = a.previous
        var sj = b.circle.r
        var sk = a.circle.r
        do {
            if (sj <= sk) {
                if (intersects(j.circle, c.circle)) {
                    b = j
                    a.next = b
                    b.previous = a
                    continue@pack // retry circle i
                }
                sj += j.circle.r
                j = j.next
            } else {
                if (intersects(k.circle, c.circle)) {
                    a = k
                    a.next = b
                    b.previous = a
                    continue@pack // retry circle i
                }
                sk += k.circle.r
                k = k.previous
            }
        } while (j !== k.next)

        // Success! Insert the new circle c between a and b.
        c.previous = a
        c.next = b
        a.next = c
        b.previous = c
        b = c

        // Compute the new closest circle pair to the centroid.
        var aa = score(a)
        c = c.next
        while (c !== b) {
            val ca = score(c)
            if (ca < aa) {
                a = c
                aa = ca
            }
            c = c.next
        }
        b = a.next
        ++i
    }

    // Compute the enclosing circle of the front chain.
    val front = mutableListOf<Circle>(b.circle)
    c = b.next
    while (c !== b) {
        front.add(c.circle)
        c = c.next
    }
    val e = packEncloseRandom(front, random)

    // Translate the circles to put the enclosing circle around the origin.
    for (circle in circles) {
        circle.x -= e.x
        circle.y -= e.y
    }
    return e.r
}

/** `radiusLeaf(defaultRadius)`: a leaf's radius is the square root of its value. */
private fun radiusLeaf(node: Node) {
    if (node.children == null) {
        val r = sqrt(node.value)
        node.r = max(0.0, if (truthy(r)) r else 0.0)
    }
}

/** `packChildrenRandom(padding, k, random)` with a constant padding. */
private fun packChildrenRandom(node: Node, padding: Double, k: Double, random: () -> Double) {
    val children = node.children ?: return
    val scaled = padding * k
    val r = if (truthy(scaled)) scaled else 0.0
    if (truthy(r)) for (child in children) child.r += r
    val e = packSiblingsRandom(children, random)
    if (truthy(r)) for (child in children) child.r -= r
    node.r = e + r
}

/** `translateChild(k)`. */
private fun translateChild(node: Node, k: Double) {
    val parent = node.parent
    node.r *= k
    if (parent != null) {
        node.x = parent.x + k * node.x
        node.y = parent.y + k * node.y
    }
}

/** `pack().size([dx, dy]).padding(padding)`: one generator serves both packing passes. */
fun pack(root: Node, dx: Double, dy: Double, padding: Double): Node {
    val random = lcg()
    root.x = dx / 2
    root.y = dy / 2
    root.eachBefore(::radiusLeaf)
        .eachAfter { packChildrenRandom(it, 0.0, 1.0, random) }
    val k = root.r / min(dx, dy)
    root.eachAfter { packChildrenRandom(it, padding, k, random) }
    val scale = min(dx, dy) / (2 * root.r)
    root.eachBefore { translateChild(it, scale) }
    return root
}
