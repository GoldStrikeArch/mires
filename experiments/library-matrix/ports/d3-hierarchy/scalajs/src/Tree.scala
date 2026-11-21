// tree.js (Buchheim et al.'s linear-time Reingold-Tilford layout) and cluster.js of d3-hierarchy
// 3.1.3, with the default separation.
package hierarchy

import scala.collection.mutable

/** `defaultSeparation`, the same function in tree.js and cluster.js. */
private def separation(a: Node, b: Node): Double = if a.parent == b.parent then 1.0 else 2.0

/** tree.js's `TreeNode`: the layout's bookkeeping for `node` (`_`), the child number `i`. */
private final class TreeNode(val node: Node, val i: Int) extends Parent[TreeNode]:
  var parent: TreeNode = null // set for every node; d3's extra parent of the root has none
  var children: Option[mutable.ArrayBuffer[TreeNode]] = None
  var defaultAncestor: Option[TreeNode] = None // A
  var ancestor: TreeNode = this // a
  var prelim = 0.0 // z
  var mod = 0.0 // m
  var change = 0.0 // c
  var shift = 0.0 // s
  var thread: Option[TreeNode] = None // t

/** The left contour's successor of `v`: its first child, or its thread. */
private def nextLeft(v: TreeNode): Option[TreeNode] = v.children.fold(v.thread)(c => Some(c(0)))

/** The right contour's successor of `v`: its last child, or its thread. */
private def nextRight(v: TreeNode): Option[TreeNode] = v.children.fold(v.thread)(c => Some(c.last))

/** Shifts the subtree rooted at `wp` and prepares the shifts of the smaller subtrees in between. */
private def moveSubtree(wm: TreeNode, wp: TreeNode, shift: Double): Unit =
  val change = shift / (wp.i - wm.i)
  wp.change -= change
  wp.shift += shift
  wm.change += change
  wp.prelim += shift
  wp.mod += shift

/** Applies the shifts that [[moveSubtree]] prepared to the children of a node. */
private def executeShifts(children: mutable.ArrayBuffer[TreeNode]): Unit =
  var shift = 0.0
  var change = 0.0
  var i = children.size - 1
  while i >= 0 do
    val w = children(i)
    w.prelim += shift
    w.mod += shift
    change += w.change
    shift += w.shift + change
    i -= 1

/** `vim`'s ancestor if it is a sibling of `v`, otherwise the default `ancestor`. */
private def nextAncestor(vim: TreeNode, v: TreeNode, ancestor: TreeNode): TreeNode =
  if vim.ancestor.parent eq v.parent then vim.ancestor else ancestor

/** `treeRoot`: wraps every node in a [[TreeNode]], under an extra parent of the root. */
private def treeRoot(root: Node): TreeNode =
  val tree = TreeNode(root, 0)
  val nodes = mutable.ArrayBuffer(tree)
  while nodes.nonEmpty do
    val node = nodes.remove(nodes.size - 1)
    for children <- node.node.children do
      val wrapped = mutable.ArrayBuffer.tabulate(children.size)(i => TreeNode(children(i), i))
      node.children = Some(wrapped)
      var i = wrapped.size - 1
      while i >= 0 do
        val child = wrapped(i)
        nodes += child
        child.parent = node
        i -= 1
  // d3's extra parent wraps null; it is never read, so it wraps the root here.
  tree.parent = TreeNode(root, 0)
  tree.parent.children = Some(mutable.ArrayBuffer(tree))
  tree

/** Computes the preliminary x of `v`, after its children; then combines it with its left siblings. */
private def firstWalk(v: TreeNode): Unit =
  val siblings = v.parent.children.get
  val w = if v.i != 0 then Some(siblings(v.i - 1)) else None
  v.children match
    case Some(children) =>
      executeShifts(children)
      val midpoint = (children(0).prelim + children.last.prelim) / 2
      w match
        case Some(w) =>
          v.prelim = w.prelim + separation(v.node, w.node)
          v.mod = v.prelim - midpoint
        case None =>
          v.prelim = midpoint
    case None =>
      for w <- w do v.prelim = w.prelim + separation(v.node, w.node)
  v.parent.defaultAncestor = Some(apportion(v, w, v.parent.defaultAncestor.getOrElse(siblings(0))))

/** Computes the real x of every node by summing the modifiers of its ancestors. */
private def secondWalk(v: TreeNode): Unit =
  v.node.x = v.prelim + v.parent.mod
  v.mod += v.parent.mod

/**
 * The core of the algorithm: combines the subtree of `v` with those of its left siblings, walking
 * the inside and outside contours, and returns the new default ancestor.
 */
private def apportion(v: TreeNode, previous: Option[TreeNode], defaultAncestor: TreeNode): TreeNode =
  previous match
    case None => defaultAncestor
    case Some(w) =>
      var ancestor = defaultAncestor
      var vop = v
      var vom = v.parent.children.get(0)
      var sip = v.mod
      var sop = v.mod
      var sim = w.mod
      var som = vom.mod
      var vim = nextRight(w)
      var vip = nextLeft(v)
      while vim.isDefined && vip.isDefined do
        val im = vim.get
        val ip = vip.get
        vom = nextLeft(vom).get
        vop = nextRight(vop).get
        vop.ancestor = v
        val shift = im.prelim + sim - ip.prelim - sip + separation(im.node, ip.node)
        if shift > 0 then
          moveSubtree(nextAncestor(im, v, ancestor), v, shift)
          sip += shift
          sop += shift
        sim += im.mod
        sip += ip.mod
        som += vom.mod
        sop += vop.mod
        vim = nextRight(im)
        vip = nextLeft(ip)
      if vim.isDefined && nextRight(vop).isEmpty then
        vop.thread = vim
        vop.mod += sim - sop
      if vip.isDefined && nextLeft(vom).isEmpty then
        vom.thread = vip
        vom.mod += sip - som
        ancestor = v
      ancestor

/** `tree().size((dx, dy))`, or `tree().nodeSize((dx, dy))` when `nodeSize` is set. */
def tree(root: Node, dx: Double, dy: Double, nodeSize: Boolean): Node =
  val t = treeRoot(root)

  // Compute the layout using Buchheim et al.'s algorithm.
  t.eachAfter(firstWalk)
  t.parent.mod = -t.prelim
  t.eachBefore(secondWalk)

  if nodeSize then
    // A fixed node size scales x and y.
    root.eachBefore { node =>
      node.x *= dx
      node.y = node.depth * dy
    }
  else
    // A fixed tree size scales x and y by the extent of the left-most, right-most and deepest
    // nodes.
    var left = root
    var right = root
    var bottom = root
    root.eachBefore { node =>
      if node.x < left.x then left = node
      if node.x > right.x then right = node
      if node.depth > bottom.depth then bottom = node
    }
    val s = if left eq right then 1.0 else separation(left, right) / 2
    val tx = s - left.x
    val kx = dx / (right.x + s + tx)
    val ky = dy / (if bottom.depth != 0 then bottom.depth else 1)
    root.eachBefore { node =>
      node.x = (node.x + tx) * kx
      node.y = node.depth * ky
    }
  root

/** `cluster().size((dx, dy))`, or `cluster().nodeSize((dx, dy))` when `nodeSize` is set. */
def cluster(root: Node, dx: Double, dy: Double, nodeSize: Boolean): Node =
  var previousNode: Option[Node] = None
  var x = 0.0

  // First walk, computing the initial x and y values.
  root.eachAfter { node =>
    node.children match
      case Some(children) =>
        node.x = children.foldLeft(0.0)(_ + _.x) / children.size // meanX
        node.y = 1 + children.foldLeft(0.0)((y, c) => math.max(y, c.y)) // maxY
      case None =>
        node.x = previousNode match
          case Some(previous) =>
            x += separation(node, previous)
            x
          case None => 0.0
        node.y = 0.0
        previousNode = Some(node)
  }

  val left = leafLeft(root)
  val right = leafRight(root)
  val x0 = left.x - separation(left, right) / 2
  val x1 = right.x + separation(right, left) / 2

  // Second walk, normalizing x and y to the desired size. The root comes last, so the walk reads
  // the root's unnormalized position throughout.
  if nodeSize then
    root.eachAfter { node =>
      node.x = (node.x - root.x) * dx
      node.y = (root.y - node.y) * dy
    }
  else
    root.eachAfter { node =>
      node.x = (node.x - x0) / (x1 - x0) * dx
      node.y = (1 - (if truthy(root.y) then node.y / root.y else 1.0)) * dy
    }

private def leafLeft(root: Node): Node =
  var node = root
  while node.children.isDefined do node = node.children.get(0)
  node

private def leafRight(root: Node): Node =
  var node = root
  while node.children.isDefined do node = node.children.get.last
  node
