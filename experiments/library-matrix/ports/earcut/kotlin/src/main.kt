external interface Polygon {
    val data: Array<Double>
    val holeIndices: Array<Int>?
    val dim: Int?
    val refine: Boolean?
}

external interface Input : Polygon {
    val op: String
    val coords: Array<Double>
    val triangles: Array<Int>
    val rings: Array<Array<Array<Double>>>
    val items: Array<Polygon>
}

private fun triangulate(polygon: Polygon, refine: Boolean): Array<Int> {
    val core = Earcut()
    val dim = polygon.dim ?: 2
    val triangles = core.earcut(polygon.data, polygon.holeIndices ?: emptyArray(), dim)
    if (refine) core.refine(triangles, polygon.data, dim)
    return triangles.toTypedArray()
}

@JsName("Object")
private external class PlainObject()

private fun flatten(rings: Array<Array<Array<Double>>>): dynamic {
    val dimensions = rings[0][0].size
    val vertices = mutableListOf<Double>()
    val holes = mutableListOf<Int>()
    var previousLength = 0
    var offset = 0
    for (ring in rings) {
        for (point in ring) for (d in 0 until dimensions) vertices.add(point[d])
        if (previousLength > 0) {
            offset += previousLength
            holes.add(offset)
        }
        previousLength = ring.size
    }
    val out = PlainObject().asDynamic()
    out.vertices = vertices.toTypedArray()
    out.holes = holes.toTypedArray()
    out.dimensions = dimensions
    return out
}

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: Input): dynamic = when (input.op) {
    "earcut" -> triangulate(input, false)
    "earcutRefine" -> triangulate(input, true)
    "refine" -> {
        val triangles = input.triangles.toMutableList()
        Earcut().refine(triangles, input.coords, input.dim ?: 2)
        triangles.toTypedArray()
    }
    "deviation" -> Earcut().deviation(input.data, input.holeIndices ?: emptyArray(), input.dim ?: 2, input.triangles.toMutableList())
    "flatten" -> flatten(input.rings)
    "batch" -> Array(input.items.size) { i ->
        val item = input.items[i]
        triangulate(item, item.refine ?: false)
    }
    else -> throw IllegalArgumentException("unknown operation " + input.op)
}
