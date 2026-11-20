external interface Operation {
    val op:String
    val x:dynamic
    val y:dynamic
    val dp:Double?
    val rm:Double?
    val n:Double?
    val mode:Double?
}
external interface DecimalInput {
    val cases:Array<Operation>
    val repeat:Int?
}
private fun integer(x:Double?,fallback:Int):Int {
    if(x==null) return fallback
    return if(x==x.toInt().toDouble() && x != -2000002.0) x.toInt() else -2000001
}
@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input:DecimalInput):String {
    val lines=mutableListOf<String>()
    repeat(input.repeat ?: 1) {
        for(c in input.cases) {
            val rm=integer(c.rm,1)
            lines.add(try {
                operate(c.op,c.x.toString(),if(c.y==null) "" else c.y.toString(),integer(c.n,-2000002),integer(c.dp,20),rm,integer(c.mode,rm))
            } catch(e:IllegalStateException) {e.message ?: throw e})
        }
    }
    return lines.joinToString("\n")
}
