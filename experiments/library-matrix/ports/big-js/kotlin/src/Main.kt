// The boundary: what libraries/big-js/reference.mjs does. `dp` and `rm` default with ?:, as the
// adapter's ?? does (JS null and undefined are both Kotlin null); `n` and `mode` stay nullable and
// reach the methods as upstream's optional arguments.
external interface Operation {
    val op: String
    val x: Any?
    val y: Any?
    val n: Double?
    val mode: Double?
    val dp: Double?
    val rm: Double?
}

external interface Input {
    val cases: Array<Operation>
    val repeat: Int?
}

// The Big constructor's conversion: a number goes through String(), except that -0 becomes "-0".
private fun text(v: Any?): String = when (v) {
    is String -> v
    is Double -> if (v == 0.0 && 1 / v < 0) "-0" else v.toString()
    else -> v.toString()
}

private fun operate(c: Operation): String {
    val dp = c.dp ?: 20.0
    val rm = c.rm ?: 1.0
    val x = Big.parse(text(c.x))
    val mode = c.mode ?: rm
    return when (c.op) {
        "toString" -> x.toString()
        "sqrt" -> x.sqrt(dp, rm).toString()
        "plus" -> x.plus(Big.parse(text(c.y))).toString()
        "minus" -> x.minus(Big.parse(text(c.y))).toString()
        "times" -> x.times(Big.parse(text(c.y))).toString()
        "div" -> x.div(Big.parse(text(c.y)), dp, rm).toString()
        "cmp" -> x.cmp(Big.parse(text(c.y))).toString()
        "pow" -> x.pow(c.n, dp, rm).toString()
        "round" -> x.round(c.n, mode).toString()
        "toFixed" -> x.toFixed(c.n, mode)
        "toExponential" -> x.toExponential(c.n, mode)
        "toPrecision" -> x.toPrecision(c.n, mode)
        else -> throw BigError("unknown decimal operation")
    }
}

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: Input): String {
    val outputs = mutableListOf<String>()
    repeat(input.repeat ?: 1) {
        for (c in input.cases) {
            outputs += try {
                operate(c)
            } catch (error: BigError) {
                error.message
            }
        }
    }
    return outputs.joinToString("\n")
}
