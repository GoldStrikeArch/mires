// tree.js (Buchheim et al.'s linear-time Reingold-Tilford layout) and cluster.js of d3-hierarchy
// 3.1.3, with the default separation.
package hierarchy

import kotlin.math.max

/** `defaultSeparation`, the same function in tree.js and cluster.js. */
private fun separation(a: Node, b: Node): Double = if (a.parent === b.parent) 1.0 else 2.0

/** tree.js's `TreeNode`: the layout's bookkeeping for [node] (`_`), the child number [i]. */
private class TreeNode(val node: Node, val i: Int) : Parent<TreeNode> {
    lateinit var parent: TreeNode
    override var children: List<TreeNode>? = null
    var defaultAncestor: TreeNode? = null // A
    var ancestor: TreeNode = this // a
    var prelim = 0.0 // z
    var mod = 0.0 // m
    var change = 0.0 // c
    var shift = 0.0 // s
    var thread: TreeNode? = null // t
}

/** The left contour's successor of [v]: its first child, or its thread. */
private fun nextLeft(v: TreeNode): TreeNode? = v.children?.first() ?: v.thread

/** The right contour's successor of [v]: its last child, or its thread. */
private fun nextRight(v: TreeNode): TreeNode? = v.children?.last() ?: v.thread

/** Shifts the subtree rooted at [wp] and prepares the shifts of the smaller subtrees in between. */
private fun moveSubtree(wm: TreeNode, wp: TreeNode, shift: Double) {
    val change = shift / (wp.i - wm.i)
    wp.change -= change
    wp.shift += shift
    wm.change += change
    wp.prelim += shift
    wp.mod += shift
}

/** Applies the shifts that [moveSubtree] prepared to the children of a node. */
private fun executeShifts(children: List<TreeNode>) {
    var shift = 0.0
    var change = 0.0
    for (i in children.lastIndex downTo 0) {
        val w = children[i]
        w.prelim += shift
        w.mod += shift
        change += w.change
        shift += w.shift + change
    }
}

/** [vim]'s ancestor if it is a sibling of [v], otherwise the default [ancestor]. */
private fun nextAncestor(vim: TreeNode, v: TreeNode, ancestor: TreeNode): TreeNode =
    if (vim.ancestor.parent === v.parent) vim.ancestor else ancestor

/** `treeRoot`: wraps every node in a [TreeNode], under an extra parent of the root. */
private fun treeRoot(root: Node): TreeNode {
    val tree = TreeNode(root, 0)
    val nodes = mutableListOf(tree)
    while (nodes.isNotEmpty()) {
        val node = nodes.removeLast()
        val children = node.node.children ?: continue
        val wrapped = List(children.size) { TreeNode(children[it], it) }
        node.children = wrapped
        for (i in wrapped.lastIndex downTo 0) {
            val child = wrapped[i]
            nodes.add(child)
            child.parent = node
        }
    }
    // d3's extra parent wraps null; it is never read, so it wraps the root here.
    tree.parent = TreeNode(root, 0).also { it.children = listOf(tree) }
    return tree
}

/** Computes the preliminary x of [v], after its children; then combines it with its left siblings. */
private fun firstWalk(v: TreeNode) {
    val children = v.children
    val siblings = v.parent.children!!
    val w = if (v.i != 0) siblings[v.i - 1] else null
    if (children != null) {
        executeShifts(children)
        val midpoint = (children[0].prelim + children[children.lastIndex].prelim) / 2
        if (w != null) {
            v.prelim = w.prelim + separation(v.node, w.node)
            v.mod = v.prelim - midpoint
        } else {
            v.prelim = midpoint
        }
    } else if (w != null) {
        v.prelim = w.prelim + separation(v.node, w.node)
    }
    v.parent.defaultAncestor = apportion(v, w, v.parent.defaultAncestor ?: siblings[0])
}

/** Computes the real x of every node by summing the modifiers of its ancestors. */
private fun secondWalk(v: TreeNode) {
    v.node.x = v.prelim + v.parent.mod
    v.mod += v.parent.mod
}

/**
 * The core of the algorithm: combines the subtree of [v] with those of its left siblings, walking
 * the inside and outside contours, and returns the new default ancestor.
 */
private fun apportion(v: TreeNode, w: TreeNode?, defaultAncestor: TreeNode): TreeNode {
    if (w == null) return defaultAncestor
    var ancestor = defaultAncestor
    var vop = v
    var vom = v.parent.children!![0]
    var sip = v.mod
    var sop = v.mod
    var sim = w.mod
    var som = vom.mod
    var vim = nextRight(w)
    var vip = nextLeft(v)
    while (vim != null && vip != null) {
        vom = nextLeft(vom)!!
        vop = nextRight(vop)!!
        vop.ancestor = v
        val shift = vim.prelim + sim - vip.prelim - sip + separation(vim.node, vip.node)
        if (shift > 0) {
            moveSubtree(nextAncestor(vim, v, ancestor), v, shift)
            sip += shift
            sop += shift
        }
        sim += vim.mod
        sip += vip.mod
        som += vom.mod
        sop += vop.mod
        vim = nextRight(vim)
        vip = nextLeft(vip)
    }
    if (vim != null && nextRight(vop) == null) {
        vop.thread = vim
        vop.mod += sim - sop
    }
    if (vip != null && nextLeft(vom) == null) {
        vom.thread = vip
        vom.mod += sip - som
        ancestor = v
    }
    return ancestor
}

/** `tree().size([dx, dy])`, or `tree().nodeSize([dx, dy])` when [nodeSize] is set. */
fun tree(root: Node, dx: Double, dy: Double, nodeSize: Boolean): Node {
    val t = treeRoot(root)

    // Compute the layout using Buchheim et al.'s algorithm.
    t.eachAfter(::firstWalk)
    t.parent.mod = -t.prelim
    t.eachBefore(::secondWalk)

    if (nodeSize) {
        // A fixed node size scales x and y.
        root.eachBefore { node ->
            node.x *= dx
            node.y = node.depth * dy
        }
    } else {
        // A fixed tree size scales x and y by the extent of the left-most, right-most and
        // deepest nodes.
        var left = root
        var right = root
        var bottom = root
        root.eachBefore { node ->
            if (node.x < left.x) left = node
            if (node.x > right.x) right = node
            if (node.depth > bottom.depth) bottom = node
        }
        val s = if (left === right) 1.0 else separation(left, right) / 2
        val tx = s - left.x
        val kx = dx / (right.x + s + tx)
        val ky = dy / (if (bottom.depth != 0) bottom.depth else 1)
        root.eachBefore { node ->
            node.x = (node.x + tx) * kx
            node.y = node.depth * ky
        }
    }
    return root
}

/** `cluster().size([dx, dy])`, or `cluster().nodeSize([dx, dy])` when [nodeSize] is set. */
fun cluster(root: Node, dx: Double, dy: Double, nodeSize: Boolean): Node {
    var previousNode: Node? = null
    var x = 0.0

    // First walk, computing the initial x and y values.
    root.eachAfter { node ->
        val children = node.children
        if (children != null) {
            node.x = children.fold(0.0) { sum, c -> sum + c.x } / children.size // meanX
            node.y = 1 + children.fold(0.0) { y, c -> max(y, c.y) } // maxY
        } else {
            val previous = previousNode
            node.x = if (previous != null) {
                x += separation(node, previous)
                x
            } else {
                0.0
            }
            node.y = 0.0
            previousNode = node
        }
    }

    val left = leafLeft(root)
    val right = leafRight(root)
    val x0 = left.x - separation(left, right) / 2
    val x1 = right.x + separation(right, left) / 2

    // Second walk, normalizing x and y to the desired size. The root comes last, so the walk
    // reads the root's unnormalized position throughout.
    return if (nodeSize) {
        root.eachAfter { node ->
            node.x = (node.x - root.x) * dx
            node.y = (root.y - node.y) * dy
        }
    } else {
        root.eachAfter { node ->
            node.x = (node.x - x0) / (x1 - x0) * dx
            node.y = (1 - (if (truthy(root.y)) node.y / root.y else 1.0)) * dy
        }
    }
}

private fun leafLeft(root: Node): Node {
    var node = root
    while (true) node = node.children?.first() ?: return node
}

private fun leafRight(root: Node): Node {
    var node = root
    while (true) node = node.children?.last() ?: return node
}
