private const val FONT_TEST_STRING = "mmMwWLliI0O&1"
private const val FONT_TEXT_SIZE = "48px"
private val BASE_FONTS = arrayOf("monospace", "sans-serif", "serif")
private val FONT_LIST = arrayOf(
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

internal fun getFonts(@Suppress("UNUSED_PARAMETER") options: dynamic = jsUndefined): dynamic {
    return withIframe({ _: dynamic, iWindow: dynamic ->
        val document = iWindow.document
        val holder = document.body
        holder.style.fontSize = FONT_TEXT_SIZE
        val spansContainer = document.createElement("div")
        spansContainer.style.setProperty("visibility", "hidden", "important")
        val defaultWidth = obj()
        val defaultHeight = obj()

        fun createSpan(fontFamily: String): dynamic {
            val span = document.createElement("span")
            val style = span.style
            style.position = "absolute"
            style.top = "0"
            style.left = "0"
            style.fontFamily = fontFamily
            span.textContent = FONT_TEST_STRING
            spansContainer.appendChild(span)
            return span
        }

        val baseFontsSpans = arr()
        for (font in BASE_FONTS) {
            jsPush(baseFontsSpans, createSpan(font))
        }
        val fontsSpans = obj()
        for (font in FONT_LIST) {
            val spans = arr()
            for (baseFont in BASE_FONTS) {
                jsPush(spans, createSpan("'$font',$baseFont"))
            }
            jsSet(fontsSpans, font, spans)
        }
        holder.appendChild(spansContainer)
        var index = 0
        while (index < BASE_FONTS.size) {
            jsSet(defaultWidth, BASE_FONTS[index], jsIndex(baseFontsSpans, index).offsetWidth)
            jsSet(defaultHeight, BASE_FONTS[index], jsIndex(baseFontsSpans, index).offsetHeight)
            index += 1
        }
        val available = arr()
        for (font in FONT_LIST) {
            val fontSpans = jsGet(fontsSpans, font)
            var detected = false
            var baseIndex = 0
            while (baseIndex < BASE_FONTS.size) {
                val span = jsIndex(fontSpans, baseIndex)
                if (span.offsetWidth != jsGet(defaultWidth, BASE_FONTS[baseIndex]) ||
                    span.offsetHeight != jsGet(defaultHeight, BASE_FONTS[baseIndex])
                ) {
                    detected = true
                    break
                }
                baseIndex += 1
            }
            if (detected) jsPush(available, font)
        }
        available
    }, jsUndefined, jsUndefined)
}
