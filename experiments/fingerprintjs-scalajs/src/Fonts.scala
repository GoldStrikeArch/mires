package fp

import scala.scalajs.js

object Fonts {
  private val testString = "mmMwWLliI0O&1"
  private val textSize = "48px"
  private val baseFonts = js.Array("monospace", "sans-serif", "serif")
  private val fontList = js.Array(
    "sans-serif-thin",
    "ARNO PRO",
    "Agency FB",
    "Arabic Typesetting",
    "Arial Unicode MS",
    "AvantGarde Bk BT",
    "BankGothic Md BT",
    "Batang",
    "Bitstream Vera Sans Mono",
    "Calibri",
    "Century",
    "Century Gothic",
    "Clarendon",
    "EUROSTILE",
    "Franklin Gothic",
    "Futura Bk BT",
    "Futura Md BT",
    "GOTHAM",
    "Gill Sans",
    "HELV",
    "Haettenschweiler",
    "Helvetica Neue",
    "Humanst521 BT",
    "Leelawadee",
    "Letter Gothic",
    "Levenim MT",
    "Lucida Bright",
    "Lucida Sans",
    "Menlo",
    "MS Mincho",
    "MS Outlook",
    "MS Reference Specialty",
    "MS UI Gothic",
    "MT Extra",
    "MYRIAD PRO",
    "Marlett",
    "Meiryo UI",
    "Microsoft Uighur",
    "Minion Pro",
    "Monotype Corsiva",
    "PMingLiU",
    "Pristina",
    "SCRIPTINA",
    "Segoe UI Light",
    "Serifa",
    "SimHei",
    "Small Fonts",
    "Staccato222 BT",
    "TRAJAN PRO",
    "Univers CE 55 Medium",
    "Vrinda",
    "ZWAdobeF",
  )

  def getFonts(_options: js.Any): js.Promise[js.Array[String]] =
    Dom.withIframe[js.Array[String]](
      (_, iWindow) => {
        val document = iWindow.document
        val holder = document.body
        holder.style.fontSize = textSize
        val spansContainer = document.createElement("div")
        spansContainer.style.setProperty("visibility", "hidden", "important")

        val defaultWidth = js.Dictionary.empty[Double]
        val defaultHeight = js.Dictionary.empty[Double]

        def createSpan(fontFamily: String): js.Dynamic = {
          val span = document.createElement("span")
          val style = span.style
          style.position = "absolute"
          style.top = "0"
          style.left = "0"
          style.fontFamily = fontFamily
          span.textContent = testString
          spansContainer.appendChild(span)
          span.asInstanceOf[js.Dynamic]
        }

        def createSpanWithFonts(fontToDetect: String, baseFont: String): js.Dynamic =
          createSpan(s"'$fontToDetect',$baseFont")

        val baseFontsSpans = baseFonts.map(createSpan)
        val fontsSpans = js.Dictionary.empty[js.Array[js.Dynamic]]
        var f = 0
        while (f < fontList.length) {
          val font = fontList(f)
          fontsSpans(font) = baseFonts.map(baseFont => createSpanWithFonts(font, baseFont))
          f += 1
        }

        holder.appendChild(spansContainer)
        var index = 0
        while (index < baseFonts.length) {
          defaultWidth(baseFonts(index)) = baseFontsSpans(index).offsetWidth.asInstanceOf[Double]
          defaultHeight(baseFonts(index)) = baseFontsSpans(index).offsetHeight.asInstanceOf[Double]
          index += 1
        }

        def isFontAvailable(fontSpans: js.Array[js.Dynamic]): Boolean = {
          var baseFontIndex = 0
          while (baseFontIndex < baseFonts.length) {
            val baseFont = baseFonts(baseFontIndex)
            if (
              fontSpans(baseFontIndex).offsetWidth.asInstanceOf[Double] != defaultWidth(baseFont) ||
              fontSpans(baseFontIndex).offsetHeight.asInstanceOf[Double] != defaultHeight(baseFont)
            ) return true
            baseFontIndex += 1
          }
          false
        }

        fontList.filter(font => isFontAvailable(fontsSpans(font)))
      },
    )

  val source: js.Function1[js.Any, js.Promise[js.Array[String]]] = getFonts
}
