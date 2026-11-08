open Bindings
open FpDom
open Async

let testString = "mmMwWLliI0O&1"
let textSize = "48px"
let baseFonts = ["monospace", "sans-serif", "serif"]
let fontList = [
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
]

let getFonts = () =>
  withIframe(async (_iframe, iWindow) => {
    let doc = get(iWindow, "document")
    let holder = get(doc, "body")
    set(get(holder, "style"), "fontSize", textSize)
    let spansContainer = createElement(doc, "div")
    setProperty(get(spansContainer, "style"), "visibility", "hidden", "important")
    let defaultWidth = emptyObject()
    let defaultHeight = emptyObject()
    let createSpan = fontFamily => {
      let span = createElement(doc, "span")
      let style = get(span, "style")
      set(style, "position", "absolute")
      set(style, "top", "0")
      set(style, "left", "0")
      set(style, "fontFamily", fontFamily)
      set(span, "textContent", testString)
      appendChild(spansContainer, span)->ignore
      span
    }
    let createSpanWithFonts = (fontToDetect, baseFont) => createSpan("'" ++ fontToDetect ++ "'," ++ baseFont)
    let baseFontsSpans = arrayMap(baseFonts, (font, _) => createSpan(font))
    let fontsSpans = emptyObject()
    let rec initFonts = i =>
      if i >= arrayLength(fontList) {
        ()
      } else {
        let font = getInt(fontList, i)
        set(fontsSpans, font, arrayMap(baseFonts, (baseFont, _) => createSpanWithFonts(font, baseFont)))
        initFonts(i + 1)
      }
    initFonts(0)
    appendChild(holder, spansContainer)->ignore
    let rec measureBase = index =>
      if index >= arrayLength(baseFonts) {
        ()
      } else {
        let font = getInt(baseFonts, index)
        set(defaultWidth, font, get(getInt(baseFontsSpans, index), "offsetWidth"))
        set(defaultHeight, font, get(getInt(baseFontsSpans, index), "offsetHeight"))
        measureBase(index + 1)
      }
    measureBase(0)
    let isFontAvailable = fontSpans =>
      arraySome(baseFonts, (baseFont, baseFontIndex) =>
        !strictEq(get(getInt(fontSpans, baseFontIndex), "offsetWidth"), get(defaultWidth, baseFont)) ||
          !strictEq(get(getInt(fontSpans, baseFontIndex), "offsetHeight"), get(defaultHeight, baseFont))
      )
    arrayFilter(fontList, (font, _) => isFontAvailable(asArray(get(fontsSpans, font))))
  }, jsUndefined, jsUndefined)
