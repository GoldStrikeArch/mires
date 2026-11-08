open Bindings
open Browser
open FpDom
open Async

let defaultText = "mmMwWLliI0fiflO&1"
let presets = {
  let value = emptyObject()
  set(value, "default", [])
  set(value, "apple", [{"font": "-apple-system-body"}])
  set(value, "serif", [{"fontFamily": "serif"}])
  set(value, "sans", [{"fontFamily": "sans-serif"}])
  set(value, "mono", [{"fontFamily": "monospace"}])
  set(value, "min", [{"fontSize": "1px"}])
  set(value, "system", [{"fontFamily": "system-ui"}])
  value
}

let normalizeFontMeasurement = (value: float) => {
  let decimals = if isAndroid() {
    0.
  } else {
    3.
  }
  let pow = mathPow(10., decimals)
  mathFloor(value *. pow) /. pow
}

let withNaturalFonts = (action, containerWidthPx) =>
  withIframe(( _iframe, iframeWindow) => {
    let iframeDocument = get(iframeWindow, "document")
    let iframeBody = get(iframeDocument, "body")
    let bodyStyle = get(iframeBody, "style")
    set(bodyStyle, "width", Int.toString(containerWidthPx) ++ "px")
    set(bodyStyle, "webkitTextSizeAdjust", "none")
    set(bodyStyle, "textSizeAdjust", "none")
    if isChromium() {
      set(get(iframeBody, "style"), "zoom", jsString(1. /. asFloat(get(iframeWindow, "devicePixelRatio"))))
    } else if isWebKit() {
      set(get(iframeBody, "style"), "zoom", "reset")
    }
    let linesOfText = createElement(iframeDocument, "div")
    let words = []
    let count = lsl(containerWidthPx / 20, 0)
    let rec loop = i =>
      if i >= count {
        ()
      } else {
        arrayPush(words, "word")->ignore
        loop(i + 1)
      }
    loop(0)
    set(linesOfText, "textContent", arrayJoin(words, " "))
    appendChild(iframeBody, linesOfText)->ignore
    apply3(action, iframeDocument, iframeBody, iframeWindow)
  }, identity("<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"), jsUndefined)

let getFontPreferences = () =>
  withNaturalFonts((doc, container, iframeWindow) => {
    let elements = emptyObject()
    let sizes = emptyObject()
    let keys = objectKeys(presets)
    let rec create = i =>
      if i >= arrayLength(keys) {
        ()
      } else {
        let key = getInt(keys, i)
        let preset = asArray(get(presets, key))
        let style = if arrayLength(preset) > 0 {
          getInt(preset, 0)
        } else {
          emptyObject()
        }
        let text = if arrayLength(preset) > 1 {
          getInt(preset, 1)
        } else {
          identity(defaultText)
        }
        let element = createElement(doc, "span")
        set(element, "textContent", text)
        set(get(element, "style"), "whiteSpace", "nowrap")
        let styleKeys = objectKeys(style)
        let rec applyStyle = j =>
          if j >= arrayLength(styleKeys) {
            ()
          } else {
            let name = getInt(styleKeys, j)
            let value = get(style, name)
            if !isUndefined(value) {
              set(get(element, "style"), name, value)
            }
            applyStyle(j + 1)
          }
        applyStyle(0)
        set(elements, key, element)
        call2(container, "append", createElement(doc, "br"), element)->ignore
        create(i + 1)
      }
    create(0)
    let shouldNormalizeMeasurement = isChromium() && isChromium128OrNewer()
    let rec measure = i =>
      if i >= arrayLength(keys) {
        ()
      } else {
        let key = getInt(keys, i)
        let rawWidth = asFloat(get(getBoundingClientRect(get(elements, key)), "width"))
        set(
          sizes,
          key,
          if shouldNormalizeMeasurement {
            normalizeFontMeasurement(rawWidth *. asFloat(get(iframeWindow, "devicePixelRatio")))
          } else {
            rawWidth
          },
        )
        measure(i + 1)
      }
    measure(0)
    sizes
  }, 4000)
