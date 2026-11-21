// The node of d3-hierarchy 3.1.3 and the operations the contract uses: hierarchy/index.js
// (`hierarchy`, `computeHeight`), eachBefore.js, eachAfter.js, iterator.js, sum.js and sort.js.
package hierarchy

/** A record of the input: a nested datum or a stratify row, read in place. */
external interface Datum {
    val id: String?
    val parentId: String?
    val value: Double?
    val children: Array<Datum>?
}

/** A node with children: [Node], and tree.js's `TreeNode`, whose prototype is Node's. */
interface Parent<N : Parent<N>> {
    val children: List<N>?
}

/** A circle, as pack/enclose.js reads one: a node or a computed enclosure. */
interface Circle {
    val x: Double
    val y: Double
    val r: Double
}

/**
 * `Node` of hierarchy/index.js. [data] is null for an ancestor that stratify imputed from a path,
 * and `children` is null for a leaf, as in d3. The layouts write the coordinates of their kind.
 */
class Node(var data: Datum?) : Parent<Node>, Circle {
    var id: String? = null
    var parent: Node? = null
    override var children: MutableList<Node>? = null
    var depth = 0
    var height = 0
    var value = 0.0
    override var x = 0.0
    override var y = 0.0
    override var r = 0.0
    var x0 = 0.0
    var y0 = 0.0
    var x1 = 0.0
    var y1 = 0.0
}

/** JavaScript's ToBoolean of a number: false for 0, -0 and NaN. d3 tests numbers this way. */
fun truthy(x: Double): Boolean = !(x == 0.0 || x.isNaN())

/** `node.eachBefore(callback)`: pre-order, on an explicit stack. */
inline fun <N : Parent<N>> N.eachBefore(callback: (N) -> Unit): N {
    val nodes = mutableListOf(this)
    while (nodes.isNotEmpty()) {
        val node = nodes.removeLast()
        callback(node)
        val children = node.children ?: continue
        for (i in children.lastIndex downTo 0) nodes.add(children[i])
    }
    return this
}

/** `node.eachAfter(callback)`: post-order, from the reversed visiting order of a stack walk. */
inline fun <N : Parent<N>> N.eachAfter(callback: (N) -> Unit): N {
    val nodes = mutableListOf(this)
    val next = mutableListOf<N>()
    while (nodes.isNotEmpty()) {
        val node = nodes.removeLast()
        next.add(node)
        node.children?.let { nodes.addAll(it) }
    }
    while (next.isNotEmpty()) callback(next.removeLast())
    return this
}

/** The order of `node[Symbol.iterator]`: breadth-first, each level from left to right. */
fun Node.breadthFirst(): List<Node> {
    val order = mutableListOf<Node>()
    var next = mutableListOf(this)
    do {
        val current = next
        next = mutableListOf()
        for (node in current) {
            order.add(node)
            node.children?.let { next.addAll(it) }
        }
    } while (next.isNotEmpty())
    return order
}

/** `hierarchy(data)` with the default children accessor, `d.children`. */
fun hierarchy(data: Datum): Node {
    val root = Node(data)
    val nodes = mutableListOf(root)
    while (nodes.isNotEmpty()) {
        val node = nodes.removeLast()
        val childs = node.data?.children
        if (childs != null && childs.isNotEmpty()) {
            val children = childs.mapTo(ArrayList(childs.size)) { Node(it) }
            node.children = children
            for (i in children.lastIndex downTo 0) {
                val child = children[i]
                nodes.add(child)
                child.parent = node
                child.depth = node.depth + 1
            }
        }
    }
    return root.eachBefore(::computeHeight)
}

/** `computeHeight`: raises the heights of the ancestors the node makes taller. */
fun computeHeight(start: Node) {
    var node = start
    var height = 0
    while (true) {
        node.height = height
        val parent = node.parent ?: return
        if (parent.height >= ++height) return
        node = parent
    }
}

/** `node.sum(value)`: each aggregate is `+value(data) || 0` plus the children's, last child first. */
inline fun Node.sum(value: (Datum?) -> Double): Node = eachAfter { node ->
    val own = value(node.data)
    var sum = if (truthy(own)) own else 0.0
    val children = node.children
    if (children != null) for (i in children.lastIndex downTo 0) sum += children[i].value
    node.value = sum
}

/**
 * `node.sort(compare)` with a JavaScript comparator. Array.prototype.sort is stable and counts a
 * NaN result as 0; Kotlin's sortWith is stable too.
 */
inline fun Node.sort(crossinline compare: (Node, Node) -> Double): Node = eachBefore { node ->
    node.children?.sortWith { a, b ->
        val order = compare(a, b)
        if (order < 0) -1 else if (order > 0) 1 else 0
    }
}
