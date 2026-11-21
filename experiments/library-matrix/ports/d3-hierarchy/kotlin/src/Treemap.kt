// treemap/ (index.js, dice.js, slice.js, sliceDice.js, squarify.js, binary.js, round.js) and
// partition.js of d3-hierarchy 3.1.3.
package hierarchy

import kotlin.math.max
import kotlin.math.sqrt

/** A tiling method: lays the children of a node out in the rectangle [x0, y0, x1, y1]. */
typealias Tile = (parent: Node, x0: Double, y0: Double, x1: Double, y1: Double) -> Unit

@JsName("Math")
private external object JsMath {
    fun round(x: Double): Double
}

/** round.js. Math.round rounds halves up, keeps -0 and returns -0 for [-0.5, 0). */
private fun roundNode(node: Node) {
    node.x0 = JsMath.round(node.x0)
    node.y0 = JsMath.round(node.y0)
    node.x1 = JsMath.round(node.x1)
    node.y1 = JsMath.round(node.y1)
}

/**
 * dice.js on the children [nodes] of a parent, or of a squarified row, whose value is [value]:
 * columns from left to right.
 */
fun treemapDice(nodes: List<Node>, value: Double, x0: Double, y0: Double, x1: Double, y1: Double) {
    val k = if (truthy(value)) (x1 - x0) / value else value
    var x = x0
    for (node in nodes) {
        node.y0 = y0
        node.y1 = y1
        node.x0 = x
        x += node.value * k
        node.x1 = x
    }
}

/** slice.js, as [treemapDice]: rows from top to bottom. */
fun treemapSlice(nodes: List<Node>, value: Double, x0: Double, y0: Double, x1: Double, y1: Double) {
    val k = if (truthy(value)) (y1 - y0) / value else value
    var y = y0
    for (node in nodes) {
        node.x0 = x0
        node.x1 = x1
        node.y0 = y
        y += node.value * k
        node.y1 = y
    }
}

/** sliceDice.js: slice at odd depths, dice at even ones. */
fun treemapSliceDice(parent: Node, x0: Double, y0: Double, x1: Double, y1: Double) {
    if ((parent.depth and 1) != 0) treemapSlice(parent.children!!, parent.value, x0, y0, x1, y1)
    else treemapDice(parent.children!!, parent.value, x0, y0, x1, y1)
}

private val phi = (1 + sqrt(5.0)) / 2

/** squarify.js with the default ratio, the golden ratio. */
fun treemapSquarify(parent: Node, x0: Double, y0: Double, x1: Double, y1: Double) =
    squarifyRatio(phi, parent, x0, y0, x1, y1)

/**
 * `squarifyRatio`. d3 also collects the rows, for treemapResquarify; this port only lays them out.
 */
private fun squarifyRatio(ratio: Double, parent: Node, left: Double, top: Double, x1: Double, y1: Double) {
    val nodes = parent.children!!
    val n = nodes.size
    var x0 = left
    var y0 = top
    var i0 = 0
    var i1 = 0
    var value = parent.value

    while (i0 < n) {
        val dx = x1 - x0
        val dy = y1 - y0

        // Find the next non-empty node.
        var sumValue: Double
        do {
            sumValue = nodes[i1++].value
        } while (!truthy(sumValue) && i1 < n)
        var minValue = sumValue
        var maxValue = sumValue
        val alpha = max(dy / dx, dx / dy) / (value * ratio)
        var beta = sumValue * sumValue * alpha
        var minRatio = max(maxValue / beta, beta / minValue)

        // Keep adding nodes while the aspect ratio maintains or improves.
        while (i1 < n) {
            val nodeValue = nodes[i1].value
            sumValue += nodeValue
            if (nodeValue < minValue) minValue = nodeValue
            if (nodeValue > maxValue) maxValue = nodeValue
            beta = sumValue * sumValue * alpha
            val newRatio = max(maxValue / beta, beta / minValue)
            if (newRatio > minRatio) {
                sumValue -= nodeValue
                break
            }
            minRatio = newRatio
            ++i1
        }

        // Position the row in its orientation.
        val row = nodes.subList(i0, i1)
        if (dx < dy) {
            val bottom = if (truthy(value)) y0 + dy * sumValue / value else y1
            treemapDice(row, sumValue, x0, y0, x1, bottom)
            if (truthy(value)) y0 = bottom
        } else {
            val right = if (truthy(value)) x0 + dx * sumValue / value else x1
            treemapSlice(row, sumValue, x0, y0, right, y1)
            if (truthy(value)) x0 = right
        }
        value -= sumValue
        i0 = i1
    }
}

/** binary.js: splits the children in two of about equal value, recursively. */
fun treemapBinary(parent: Node, x0: Double, y0: Double, x1: Double, y1: Double) {
    val nodes = parent.children!!
    val n = nodes.size
    val sums = DoubleArray(n + 1)
    var sum = 0.0
    for (i in 0 until n) {
        sum += nodes[i].value
        sums[i + 1] = sum
    }

    fun partition(i: Int, j: Int, value: Double, x0: Double, y0: Double, x1: Double, y1: Double) {
        if (i >= j - 1) {
            val node = nodes[i]
            node.x0 = x0
            node.y0 = y0
            node.x1 = x1
            node.y1 = y1
            return
        }

        val valueOffset = sums[i]
        val valueTarget = value / 2 + valueOffset
        var k = i + 1
        var hi = j - 1
        while (k < hi) {
            val mid = (k + hi) ushr 1
            if (sums[mid] < valueTarget) k = mid + 1 else hi = mid
        }
        if (valueTarget - sums[k - 1] < sums[k] - valueTarget && i + 1 < k) --k

        val valueLeft = sums[k] - valueOffset
        val valueRight = value - valueLeft
        if (x1 - x0 > y1 - y0) {
            val xk = if (truthy(value)) (x0 * valueRight + x1 * valueLeft) / value else x1
            partition(i, k, valueLeft, x0, y0, xk, y1)
            partition(k, j, valueRight, xk, y0, x1, y1)
        } else {
            val yk = if (truthy(value)) (y0 * valueRight + y1 * valueLeft) / value else y1
            partition(i, k, valueLeft, x0, y0, x1, yk)
            partition(k, j, valueRight, x0, yk, x1, y1)
        }
    }

    partition(0, n, parent.value, x0, y0, x1, y1)
}

/**
 * `treemap().size([dx, dy]).tile(tile).padding(padding).round(round)`: the inner and every outer
 * padding are [padding].
 */
fun treemap(root: Node, dx: Double, dy: Double, tile: Tile, padding: Double, round: Boolean): Node {
    // d3 keeps the stack between calls and resets it to [0]; depth d reads index d.
    val paddingStack = DoubleArray(root.height + 1)
    root.x0 = 0.0
    root.y0 = 0.0
    root.x1 = dx
    root.y1 = dy
    root.eachBefore { node ->
        var p = paddingStack[node.depth]
        var x0 = node.x0 + p
        var y0 = node.y0 + p
        var x1 = node.x1 - p
        var y1 = node.y1 - p
        if (x1 < x0) { x0 = (x0 + x1) / 2; x1 = x0 }
        if (y1 < y0) { y0 = (y0 + y1) / 2; y1 = y0 }
        node.x0 = x0
        node.y0 = y0
        node.x1 = x1
        node.y1 = y1
        if (node.children != null) {
            p = padding / 2
            paddingStack[node.depth + 1] = p
            x0 += padding - p
            y0 += padding - p
            x1 -= padding - p
            y1 -= padding - p
            if (x1 < x0) { x0 = (x0 + x1) / 2; x1 = x0 }
            if (y1 < y0) { y0 = (y0 + y1) / 2; y1 = y0 }
            tile(node, x0, y0, x1, y1)
        }
    }
    if (round) root.eachBefore(::roundNode)
    return root
}

/** `partition().size([dx, dy]).padding(padding).round(round)`. */
fun partition(root: Node, dx: Double, dy: Double, padding: Double, round: Boolean): Node {
    val n = root.height + 1
    root.x0 = padding
    root.y0 = padding
    root.x1 = dx
    root.y1 = dy / n
    root.eachBefore { node ->
        node.children?.let {
            treemapDice(it, node.value, node.x0, dy * (node.depth + 1) / n, node.x1, dy * (node.depth + 2) / n)
        }
        var x0 = node.x0
        var y0 = node.y0
        var x1 = node.x1 - padding
        var y1 = node.y1 - padding
        if (x1 < x0) { x0 = (x0 + x1) / 2; x1 = x0 }
        if (y1 < y0) { y0 = (y0 + y1) / 2; y1 = y0 }
        node.x0 = x0
        node.y0 = y0
        node.x1 = x1
        node.y1 = y1
    }
    if (round) root.eachBefore(::roundNode)
    return root
}
