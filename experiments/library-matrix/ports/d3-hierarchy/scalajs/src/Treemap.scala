// treemap/ (index.js, dice.js, slice.js, sliceDice.js, squarify.js, binary.js, round.js) and
// partition.js of d3-hierarchy 3.1.3.
package hierarchy

import scala.collection.mutable
import scala.scalajs.js

/** A tiling method: lays the children of a node out in the rectangle (x0, y0, x1, y1). */
type Tile = (Node, Double, Double, Double, Double) => Unit

/** round.js. Math.round rounds halves up, keeps -0 and returns -0 for [-0.5, 0). */
private def roundNode(node: Node): Unit =
  node.x0 = js.Math.round(node.x0)
  node.y0 = js.Math.round(node.y0)
  node.x1 = js.Math.round(node.x1)
  node.y1 = js.Math.round(node.y1)

/**
 * dice.js on the children `nodes` of a parent, or of a squarified row, whose value is `value`:
 * columns from left to right.
 */
def treemapDice(nodes: collection.Seq[Node], value: Double, x0: Double, y0: Double, x1: Double, y1: Double): Unit =
  val k = if truthy(value) then (x1 - x0) / value else value
  var x = x0
  for node <- nodes do
    node.y0 = y0
    node.y1 = y1
    node.x0 = x
    x += node.value * k
    node.x1 = x

/** slice.js, as [[treemapDice]]: rows from top to bottom. */
def treemapSlice(nodes: collection.Seq[Node], value: Double, x0: Double, y0: Double, x1: Double, y1: Double): Unit =
  val k = if truthy(value) then (y1 - y0) / value else value
  var y = y0
  for node <- nodes do
    node.x0 = x0
    node.x1 = x1
    node.y0 = y
    y += node.value * k
    node.y1 = y

/** sliceDice.js: slice at odd depths, dice at even ones. */
def treemapSliceDice(parent: Node, x0: Double, y0: Double, x1: Double, y1: Double): Unit =
  if (parent.depth & 1) != 0 then treemapSlice(parent.children.get, parent.value, x0, y0, x1, y1)
  else treemapDice(parent.children.get, parent.value, x0, y0, x1, y1)

private val phi = (1 + math.sqrt(5)) / 2

/** squarify.js with the default ratio, the golden ratio. */
def treemapSquarify(parent: Node, x0: Double, y0: Double, x1: Double, y1: Double): Unit =
  squarifyRatio(phi, parent, x0, y0, x1, y1)

/** `squarifyRatio`. d3 also collects the rows, for treemapResquarify; this port only lays them out. */
private def squarifyRatio(ratio: Double, parent: Node, left: Double, top: Double, x1: Double, y1: Double): Unit =
  val nodes = parent.children.get
  val n = nodes.size
  var x0 = left
  var y0 = top
  var i0 = 0
  var i1 = 0
  var value = parent.value

  while i0 < n do
    val dx = x1 - x0
    val dy = y1 - y0

    // Find the next non-empty node.
    var sumValue = nodes(i1).value
    i1 += 1
    while !truthy(sumValue) && i1 < n do
      sumValue = nodes(i1).value
      i1 += 1
    var minValue = sumValue
    var maxValue = sumValue
    val alpha = math.max(dy / dx, dx / dy) / (value * ratio)
    var beta = sumValue * sumValue * alpha
    var minRatio = math.max(maxValue / beta, beta / minValue)

    // Keep adding nodes while the aspect ratio maintains or improves.
    var growing = true
    while growing && i1 < n do
      val nodeValue = nodes(i1).value
      sumValue += nodeValue
      if nodeValue < minValue then minValue = nodeValue
      if nodeValue > maxValue then maxValue = nodeValue
      beta = sumValue * sumValue * alpha
      val newRatio = math.max(maxValue / beta, beta / minValue)
      if newRatio > minRatio then
        sumValue -= nodeValue
        growing = false
      else
        minRatio = newRatio
        i1 += 1

    // Position the row in its orientation.
    val row = nodes.view.slice(i0, i1).toSeq
    if dx < dy then
      val bottom = if truthy(value) then y0 + dy * sumValue / value else y1
      treemapDice(row, sumValue, x0, y0, x1, bottom)
      if truthy(value) then y0 = bottom
    else
      val right = if truthy(value) then x0 + dx * sumValue / value else x1
      treemapSlice(row, sumValue, x0, y0, right, y1)
      if truthy(value) then x0 = right
    value -= sumValue
    i0 = i1

/** binary.js: splits the children in two of about equal value, recursively. */
def treemapBinary(parent: Node, x0: Double, y0: Double, x1: Double, y1: Double): Unit =
  val nodes = parent.children.get
  val n = nodes.size
  val sums = new Array[Double](n + 1)
  var sum = 0.0
  for i <- 0 until n do
    sum += nodes(i).value
    sums(i + 1) = sum

  def partition(i: Int, j: Int, value: Double, x0: Double, y0: Double, x1: Double, y1: Double): Unit =
    if i >= j - 1 then
      val node = nodes(i)
      node.x0 = x0
      node.y0 = y0
      node.x1 = x1
      node.y1 = y1
    else
      val valueOffset = sums(i)
      val valueTarget = value / 2 + valueOffset
      var k = i + 1
      var hi = j - 1
      while k < hi do
        val mid = (k + hi) >>> 1
        if sums(mid) < valueTarget then k = mid + 1 else hi = mid
      if valueTarget - sums(k - 1) < sums(k) - valueTarget && i + 1 < k then k -= 1

      val valueLeft = sums(k) - valueOffset
      val valueRight = value - valueLeft
      if x1 - x0 > y1 - y0 then
        val xk = if truthy(value) then (x0 * valueRight + x1 * valueLeft) / value else x1
        partition(i, k, valueLeft, x0, y0, xk, y1)
        partition(k, j, valueRight, xk, y0, x1, y1)
      else
        val yk = if truthy(value) then (y0 * valueRight + y1 * valueLeft) / value else y1
        partition(i, k, valueLeft, x0, y0, x1, yk)
        partition(k, j, valueRight, x0, yk, x1, y1)

  partition(0, n, parent.value, x0, y0, x1, y1)

/**
 * `treemap().size((dx, dy)).tile(tile).padding(padding).round(round)`: the inner and every outer
 * padding are `padding`.
 */
def treemap(root: Node, dx: Double, dy: Double, tile: Tile, padding: Double, round: Boolean): Node =
  // d3 keeps the stack between calls and resets it to [0]; depth d reads index d.
  val paddingStack = new Array[Double](root.height + 1)
  root.x0 = 0.0
  root.y0 = 0.0
  root.x1 = dx
  root.y1 = dy
  root.eachBefore { node =>
    var p = paddingStack(node.depth)
    var x0 = node.x0 + p
    var y0 = node.y0 + p
    var x1 = node.x1 - p
    var y1 = node.y1 - p
    if x1 < x0 then { x0 = (x0 + x1) / 2; x1 = x0 }
    if y1 < y0 then { y0 = (y0 + y1) / 2; y1 = y0 }
    node.x0 = x0
    node.y0 = y0
    node.x1 = x1
    node.y1 = y1
    if node.children.isDefined then
      p = padding / 2
      paddingStack(node.depth + 1) = p
      x0 += padding - p
      y0 += padding - p
      x1 -= padding - p
      y1 -= padding - p
      if x1 < x0 then { x0 = (x0 + x1) / 2; x1 = x0 }
      if y1 < y0 then { y0 = (y0 + y1) / 2; y1 = y0 }
      tile(node, x0, y0, x1, y1)
  }
  if round then root.eachBefore(roundNode)
  root

/** `partition().size((dx, dy)).padding(padding).round(round)`. */
def partition(root: Node, dx: Double, dy: Double, padding: Double, round: Boolean): Node =
  val n = root.height + 1
  root.x0 = padding
  root.y0 = padding
  root.x1 = dx
  root.y1 = dy / n
  root.eachBefore { node =>
    for children <- node.children do
      treemapDice(children, node.value, node.x0, dy * (node.depth + 1) / n, node.x1, dy * (node.depth + 2) / n)
    var x0 = node.x0
    var y0 = node.y0
    var x1 = node.x1 - padding
    var y1 = node.y1 - padding
    if x1 < x0 then { x0 = (x0 + x1) / 2; x1 = x0 }
    if y1 < y0 then { y0 = (y0 + y1) / 2; y1 = y0 }
    node.x0 = x0
    node.y0 = y0
    node.x1 = x1
    node.y1 = y1
  }
  if round then root.eachBefore(roundNode)
  root
