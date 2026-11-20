package bigjs
import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

@js.native trait Operation extends js.Object {
  val op:String = js.native
  val x:js.Any = js.native
  val y:js.Any = js.native
  val dp:js.UndefOr[Double] = js.native
  val rm:js.UndefOr[Double] = js.native
  val n:js.UndefOr[Double] = js.native
  val mode:js.UndefOr[Double] = js.native
}
@js.native trait Input extends js.Object {
  val cases:js.Array[Operation] = js.native
  val repeat:js.UndefOr[Int] = js.native
}
object Main {
  private def integer(x:js.UndefOr[Double], fallback:Int):Int =
    x.toOption.fold(fallback)(v=>if(v==v.toInt.toDouble && v != -2000002.0) v.toInt else -2000001)
  private def text(x:js.Any):String=js.Dynamic.global.String(x).asInstanceOf[String]
  @JSExportTopLevel("run")
  def run(input:Input):String = {
    val lines=Array.newBuilder[String]
    for(_ <- 0 until input.repeat.getOrElse(1); c <- input.cases) {
      val rm=integer(c.rm,1)
      lines += (try {
        DecimalMath.operate(c.op,text(c.x),text(c.y),integer(c.n,-2000002),integer(c.dp,20),rm,integer(c.mode,rm))
      } catch {case e:IllegalArgumentException => e.getMessage})
    }
    lines.result().mkString("\n")
  }
}
