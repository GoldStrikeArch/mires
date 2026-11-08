package fp

import scala.scalajs.js

object Canvas {
  val Unsupported = "unsupported"
  val Skipped = "skipped"
  val Unstable = "unstable"

  def getCanvasFingerprint(_options: js.Any): js.Dynamic =
    getUnstableCanvasFingerprint(doesBrowserPerformAntiFingerprinting())

  def getUnstableCanvasFingerprint(skipImages: js.UndefOr[Boolean] = js.undefined): js.Dynamic = {
    val skip = skipImages.getOrElse(false)
    var winding = false
    var geometry = ""
    var text = ""
    val pair = makeCanvasContext()
    val canvas = pair._1
    val context = pair._2
    if (!isSupported(canvas, context)) {
      geometry = Unsupported
      text = Unsupported
    } else {
      winding = doesSupportWinding(context)
      if (skip) {
        geometry = Skipped
        text = Skipped
      } else {
        val images = renderImages(canvas, context)
        geometry = images._1
        text = images._2
      }
    }
    js.Dynamic.literal(winding = winding, geometry = geometry, text = text)
  }

  private def makeCanvasContext(): (js.Dynamic, js.Any) = {
    val canvas = Host.document.createElement("canvas")
    canvas.width = 1
    canvas.height = 1
    (canvas, canvas.getContext("2d").asInstanceOf[js.Any])
  }

  private def isSupported(canvas: js.Dynamic, context: js.Any): Boolean =
    Host.jsTruthy(context) && Host.jsTruthy(canvas.toDataURL.asInstanceOf[js.Any])

  private def doesSupportWinding(context: js.Any): Boolean = {
    val ctx = Host.dyn(context)
    ctx.rect(0, 0, 10, 10)
    ctx.rect(2, 2, 6, 6)
    !ctx.isPointInPath(5, 5, "evenodd").asInstanceOf[Boolean]
  }

  private def renderImages(canvas: js.Dynamic, context: js.Any): (String, String) = {
    val ctx = Host.dyn(context)
    renderTextImage(canvas, ctx)
    val textImage1 = canvasToString(canvas)
    val textImage2 = canvasToString(canvas)
    if (textImage1 != textImage2) return (Unstable, Unstable)
    renderGeometryImage(canvas, ctx)
    (canvasToString(canvas), textImage1)
  }

  private def renderTextImage(canvas: js.Dynamic, context: js.Dynamic): Unit = {
    canvas.width = 240
    canvas.height = 60
    context.textBaseline = "alphabetic"
    context.fillStyle = "#f60"
    context.fillRect(100, 1, 62, 20)
    context.fillStyle = "#069"
    context.font = """11pt "Times New Roman""""
    val printedText = "Cwm fjordbank gly " + Host.fromCharCode(55357, 56835)
    context.fillText(printedText, 2, 15)
    context.fillStyle = "rgba(102, 204, 0, 0.2)"
    context.font = "18pt Arial"
    context.fillText(printedText, 4, 45)
  }

  private def renderGeometryImage(canvas: js.Dynamic, context: js.Dynamic): Unit = {
    canvas.width = 122
    canvas.height = 110
    context.globalCompositeOperation = "multiply"
    val colors = js.Array(("#f2f", 40, 40), ("#2ff", 80, 40), ("#ff2", 60, 80))
    var i = 0
    while (i < colors.length) {
      val triple = colors(i)
      context.fillStyle = triple._1
      context.beginPath()
      context.arc(triple._2, triple._3, 40, 0, math.Pi * 2, true)
      context.closePath()
      context.fill()
      i += 1
    }
    context.fillStyle = "#f9c"
    context.arc(60, 60, 60, 0, math.Pi * 2, true)
    context.arc(60, 60, 20, 0, math.Pi * 2, true)
    context.fill("evenodd")
  }

  private def canvasToString(canvas: js.Dynamic): String =
    canvas.toDataURL().asInstanceOf[String]

  private def doesBrowserPerformAntiFingerprinting(): Boolean = {
    val isSafari17OrAbove = Browser.isWebKit() && Browser.isWebKit616OrNewer() && Browser.isSafariWebKit()
    val isFirefox120OrAbove = Browser.isGecko() && Browser.isGecko120OrNewer()
    isSafari17OrAbove || isFirefox120OrAbove
  }

  val source: js.Function1[js.Any, js.Dynamic] = getCanvasFingerprint
  val getUnstableCanvasFingerprintFn: js.Function1[js.UndefOr[Boolean], js.Dynamic] =
    skip => getUnstableCanvasFingerprint(skip)
}
