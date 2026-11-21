// Entry of the Kotlin/JS port: one layout per input, as libraries/d3-hierarchy/reference.mjs runs
// it. The input is read in place through external interfaces; the rows are JS arrays of strings,
// numbers and null. `run` is a native ES module export through @JsExport.

import hierarchy.Datum
import hierarchy.Node
import hierarchy.Tile
import hierarchy.breadthFirst
import hierarchy.cluster
import hierarchy.eachBefore
import hierarchy.hierarchy
import hierarchy.pack
import hierarchy.partition
import hierarchy.sort
import hierarchy.stratify
import hierarchy.sum
import hierarchy.tree
import hierarchy.treemap
import hierarchy.treemapBinary
import hierarchy.treemapSliceDice
import hierarchy.treemapSquarify

external interface Input {
    val op: String
    val source: String
    val data: Datum?
    val rows: Array<Datum>?
    val width: Double
    val height: Double
    val padding: Double
    val sort: Boolean
    val round: Boolean
    val nodeSize: Boolean
    val tile: String
}

private fun tile(name: String): Tile = when (name) {
    "binary" -> ::treemapBinary
    "sliceDice" -> ::treemapSliceDice
    "squarify" -> ::treemapSquarify
    else -> error("unknown tile") // `.tile(undefined)` throws
}

/** The reference's `n.id ?? n.data.id`. */
private fun id(node: Node): String? = node.id ?: node.data!!.id

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: Input): Array<Array<Any?>> {
    val root = when (input.source) {
        "id" -> stratify(input.rows!!)
        "path" -> stratify(input.rows!!) { it.id!! }
        else -> hierarchy(input.data!!)
    }
    root.sum { it?.value ?: 0.0 }
    if (input.sort) root.sort { a, b -> b.value - a.value }
    val w = input.width
    val h = input.height
    val p = input.padding
    when (input.op) {
        "tree" -> tree(root, w, h, input.nodeSize)
        "cluster" -> cluster(root, w, h, input.nodeSize)
        "partition" -> partition(root, w, h, p, input.round)
        "pack" -> pack(root, w, h, p)
        "treemap" -> treemap(root, w, h, tile(input.tile), p, input.round)
        "hierarchy", "iterator", "stratify" -> {}
        else -> error("unknown layout")
    }
    val output = mutableListOf<Array<Any?>>()
    fun visit(n: Node) {
        val record = mutableListOf<Any?>(id(n), n.parent?.let(::id), n.depth, n.height, n.value)
        when (input.op) {
            "tree", "cluster" -> record.addAll(listOf(n.x, n.y))
            "pack" -> record.addAll(listOf(n.x, n.y, n.r))
            "partition", "treemap" -> record.addAll(listOf(n.x0, n.y0, n.x1, n.y1))
        }
        output.add(record.toTypedArray())
    }
    if (input.op == "iterator") root.breadthFirst().forEach(::visit) else root.eachBefore(::visit)
    return output.toTypedArray()
}
