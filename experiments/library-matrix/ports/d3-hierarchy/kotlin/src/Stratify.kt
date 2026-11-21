// stratify.js of d3-hierarchy 3.1.3, with the default id and parentId accessors.
package hierarchy

/**
 * `stratify()(data)`, or `stratify().path(path)(data)` when [path] is given. An imputed ancestor
 * is a node whose data is null; d3 marks one with the `imputed` object and replaces that by null
 * before it returns.
 */
fun stratify(data: Array<Datum>, path: ((Datum) -> String)? = null): Node {
    val rows = data.toMutableList<Datum?>()
    val ids: List<String?>
    val parentIds: List<String?>
    if (path != null) {
        val i = data.mapTo(mutableListOf()) { normalize(path(it)) }
        val p = i.mapTo(mutableListOf()) { parentof(it) }
        val s = i.toHashSet().apply { add("") }
        var k = 0
        while (k < p.size) {
            val id = p[k++]
            if (s.add(id)) {
                i.add(id)
                p.add(parentof(id))
                rows.add(null)
            }
        }
        ids = i
        parentIds = p
    } else {
        ids = data.map { it.id }
        parentIds = data.map { it.parentId }
    }

    var n = rows.size
    val ambiguous = Node(null)
    val nodeByKey = HashMap<String, Node>()
    val nodes = ArrayList<Node>(n)
    val parentKeys = arrayOfNulls<String>(n)
    for (i in 0 until n) {
        val node = Node(rows[i])
        nodes.add(node)
        val nodeId = ids[i]
        if (!nodeId.isNullOrEmpty()) {
            node.id = nodeId
            nodeByKey[nodeId] = if (nodeId in nodeByKey) ambiguous else node
        }
        val parentId = parentIds[i]
        if (!parentId.isNullOrEmpty()) parentKeys[i] = parentId
    }

    var first: Node? = null
    for (i in 0 until n) {
        val node = nodes[i]
        val key = parentKeys[i]
        if (key != null) {
            val parent = nodeByKey[key] ?: error("missing: $key")
            if (parent === ambiguous) error("ambiguous: $key")
            parent.children?.add(node) ?: run { parent.children = mutableListOf(node) }
            node.parent = parent
        } else {
            if (first != null) error("multiple roots")
            first = node
        }
    }
    var root = first ?: error("no root")

    // When imputing internal nodes, only introduce roots if needed.
    if (path != null) {
        while (root.data == null && root.children!!.size == 1) {
            root = root.children!![0]
            --n
        }
    }

    // d3 parents the root to `preroot`, whose depth is -1, while it walks.
    root.parent = null
    root.eachBefore { node ->
        node.depth = (node.parent?.depth ?: -1) + 1
        --n
    }.eachBefore(::computeHeight)
    if (n > 0) error("cycle")
    return root
}

/**
 * Coerces nothing (ids are strings here), strips a trailing slash unless another slash precedes
 * it, and adds a leading slash if missing.
 */
private fun normalize(path: String): String {
    val i = path.length
    val p = if (slash(path, i - 1) && !slash(path, i - 2)) path.substring(0, i - 1) else path
    return if (p.startsWith('/')) p else "/$p"
}

/** The path up to its last delimiting slash after the first character: "/a/b" to "/a", "/" to "". */
private fun parentof(path: String): String {
    var i = path.length
    if (i < 2) return ""
    while (--i > 1) if (slash(path, i)) break
    return path.substring(0, i)
}

/** Whether `path[i]` is a delimiting slash: one preceded by an even number of backslashes. */
private fun slash(path: String, i: Int): Boolean {
    if (path.getOrNull(i) == '/') {
        var k = 0
        var j = i
        while (j > 0 && path[--j] == '\\') ++k
        if ((k and 1) == 0) return true
    }
    return false
}
