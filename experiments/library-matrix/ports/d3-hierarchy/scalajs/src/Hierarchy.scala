// The node of d3-hierarchy 3.1.3 and the operations the contract uses: hierarchy/index.js
// (`hierarchy`, `computeHeight`), eachBefore.js, eachAfter.js, iterator.js, sum.js and sort.js.
package hierarchy

import scala.collection.mutable
import scala.scalajs.js

/** A record of the input: a nested datum or a stratify row, read in place. */
@js.native trait Datum extends js.Object:
  val id: js.UndefOr[String] = js.native
  val parentId: js.UndefOr[String] = js.native
  val value: js.UndefOr[Double] = js.native
  val children: js.UndefOr[js.Array[Datum]] = js.native

/** A node with children: [[Node]], and tree.js's `TreeNode`, whose prototype is Node's. */
trait Parent[N <: Parent[N]]:
  this: N =>
  def children: Option[mutable.ArrayBuffer[N]]

  /** `node.eachBefore(callback)`: pre-order, on an explicit stack. */
  final def eachBefore(callback: N => Unit): N =
    val nodes = mutable.ArrayBuffer[N](this)
    while nodes.nonEmpty do
      val node = nodes.remove(nodes.size - 1)
      callback(node)
      for children <- node.children do
        var i = children.size - 1
        while i >= 0 do
          nodes += children(i)
          i -= 1
    this

  /** `node.eachAfter(callback)`: post-order, from the reversed visiting order of a stack walk. */
  final def eachAfter(callback: N => Unit): N =
    val nodes = mutable.ArrayBuffer[N](this)
    val next = mutable.ArrayBuffer.empty[N]
    while nodes.nonEmpty do
      val node = nodes.remove(nodes.size - 1)
      next += node
      for children <- node.children do nodes ++= children
    while next.nonEmpty do callback(next.remove(next.size - 1))
    this

/** A circle, as pack/enclose.js reads one: a node or a computed enclosure. */
trait Circle:
  def x: Double
  def y: Double
  def r: Double

/**
 * `Node` of hierarchy/index.js. `data` is None for an ancestor that stratify imputed from a path,
 * and `children` is None for a leaf, as in d3. The layouts write the coordinates of their kind.
 */
final class Node(var data: Option[Datum]) extends Parent[Node], Circle:
  var id: Option[String] = None
  var parent: Option[Node] = None
  var children: Option[mutable.ArrayBuffer[Node]] = None
  var depth = 0
  var height = 0
  var value = 0.0
  var x = 0.0
  var y = 0.0
  var r = 0.0
  var x0 = 0.0
  var y0 = 0.0
  var x1 = 0.0
  var y1 = 0.0

  /** The order of `node[Symbol.iterator]`: breadth-first, each level from left to right. */
  def breadthFirst: mutable.ArrayBuffer[Node] =
    val order = mutable.ArrayBuffer.empty[Node]
    var next = mutable.ArrayBuffer[Node](this)
    while next.nonEmpty do
      val current = next
      next = mutable.ArrayBuffer.empty[Node]
      for node <- current do
        order += node
        for children <- node.children do next ++= children
    order

  /** `node.sum(value)`: each aggregate is `+value(data) || 0` plus the children's, last child first. */
  def sum(value: Option[Datum] => Double): Node = eachAfter { node =>
    val own = value(node.data)
    var sum = if truthy(own) then own else 0.0
    for children <- node.children do
      var i = children.size - 1
      while i >= 0 do
        sum += children(i).value
        i -= 1
    node.value = sum
  }

  /**
   * `node.sort(compare)` with a JavaScript comparator. Array.prototype.sort is stable and counts a
   * NaN result as 0; sortInPlaceWith is stable too.
   */
  def sort(compare: (Node, Node) => Double): Node = eachBefore { node =>
    for children <- node.children do children.sortInPlaceWith((a, b) => compare(a, b) < 0)
  }

/** JavaScript's ToBoolean of a number: false for 0, -0 and NaN. d3 tests numbers this way. */
def truthy(x: Double): Boolean = !(x == 0.0 || x.isNaN)

/** `hierarchy(data)` with the default children accessor, `d.children`. */
def hierarchy(data: Datum): Node =
  val root = Node(Some(data))
  val nodes = mutable.ArrayBuffer(root)
  while nodes.nonEmpty do
    val node = nodes.remove(nodes.size - 1)
    for
      datum <- node.data
      childs <- datum.children.toOption
      if childs.length > 0
    do
      val children = mutable.ArrayBuffer.from(childs.iterator.map(child => Node(Some(child))))
      node.children = Some(children)
      var i = children.size - 1
      while i >= 0 do
        val child = children(i)
        nodes += child
        child.parent = Some(node)
        child.depth = node.depth + 1
        i -= 1
  root.eachBefore(computeHeight)

/** `computeHeight`: raises the heights of the ancestors the node makes taller. */
def computeHeight(start: Node): Unit =
  var node = start
  var height = 0
  var climbing = true
  while climbing do
    node.height = height
    height += 1
    node.parent match
      case Some(parent) if parent.height < height => node = parent
      case _ => climbing = false
