package fp

import scala.scalajs.js

object FontPreferences {
  private val defaultText = "mmMwWLliI0fiflO&1"

  private val presetKeys = js.Array("default", "apple", "serif", "sans", "mono", "min", "system")

  private def presetStyle(key: String): js.Dictionary[String] =
    key match {
      case "apple"  => js.Dictionary("font" -> "-apple-system-body")
      case "serif"  => js.Dictionary("fontFamily" -> "serif")
      case "sans"   => js.Dictionary("fontFamily" -> "sans-serif")
      case "mono"   => js.Dictionary("fontFamily" -> "monospace")
      case "min"    => js.Dictionary("fontSize" -> "1px")
      case "system" => js.Dictionary("fontFamily" -> "system-ui")
      case _        => js.Dictionary[String]()
    }

  def getFontPreferences(_options: js.Any): js.Promise[js.Dictionary[Double]] =
    withNaturalFonts { (document, container, iframeWindow) =>
      val elements = js.Dictionary.empty[js.Dynamic]
      val sizes = js.Dictionary.empty[Double]
      var i = 0
      while (i < presetKeys.length) {
        val key = presetKeys(i)
        val style = presetStyle(key)
        val element = document.createElement("span")
        element.textContent = defaultText
        element.style.whiteSpace = "nowrap"
        val names = js.Object.keys(style.asInstanceOf[js.Object])
        var n = 0
        while (n < names.length) {
          val name = names(n)
          element.style.selectDynamic(name)
          Host.dyn(element.style).updateDynamic(name)(style(name))
          n += 1
        }
        elements(key) = element
        container.append(document.createElement("br"), element)
        i += 1
      }
      val shouldNormalizeMeasurement = Browser.isChromium() && Browser.isChromium128OrNewer()
      var k = 0
      while (k < presetKeys.length) {
        val key = presetKeys(k)
        val rawWidth = elements(key).getBoundingClientRect().width.asInstanceOf[Double]
        sizes(key) =
          if (shouldNormalizeMeasurement)
            normalizeFontMeasurement(rawWidth * iframeWindow.devicePixelRatio.asInstanceOf[Double])
          else rawWidth
        k += 1
      }
      sizes
    }

  private def normalizeFontMeasurement(value: Double): Double = {
    val decimals = if (Browser.isAndroid()) 0 else 3
    val pow = math.pow(10, decimals)
    math.floor(value * pow) / pow
  }

  private def withNaturalFonts[T](
      action: (js.Dynamic, js.Dynamic, js.Dynamic) => T,
      containerWidthPx: Int = 4000,
  ): js.Promise[T] =
    Dom.withIframe[T](
      (_, iframeWindow) => {
        val iframeDocument = iframeWindow.document
        val iframeBody = iframeDocument.body
        val bodyStyle = iframeBody.style
        bodyStyle.width = s"${containerWidthPx}px"
        bodyStyle.webkitTextSizeAdjust = "none"
        bodyStyle.textSizeAdjust = "none"
        if (Browser.isChromium())
          iframeBody.style.zoom = s"${1 / iframeWindow.devicePixelRatio.asInstanceOf[Double]}"
        else if (Browser.isWebKit()) iframeBody.style.zoom = "reset"
        val linesOfText = iframeDocument.createElement("div")
        val words = new js.Array[String]((containerWidthPx / 20).toInt)
        var i = 0
        while (i < words.length) {
          words(i) = "word"
          i += 1
        }
        linesOfText.textContent = words.join(" ")
        iframeBody.appendChild(linesOfText)
        action(iframeDocument, iframeBody, iframeWindow).asInstanceOf[js.Any]
      },
      "<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    )

  val source: js.Function1[js.Any, js.Promise[js.Dictionary[Double]]] = getFontPreferences
}
