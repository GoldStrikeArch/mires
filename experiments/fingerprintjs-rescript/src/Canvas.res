open Bindings
open Browser

let doesBrowserPerformAntiFingerprinting = () => {
  let isSafari17OrAbove = isWebKit() && isWebKit616OrNewer() && isSafariWebKit()
  let isFirefox120OrAbove = isGecko() && isGecko120OrNewer()
  isSafari17OrAbove || isFirefox120OrAbove
}

let makeCanvasContext = () => {
  let canvas = createElement(document(), "canvas")
  set(canvas, "width", 1)
  set(canvas, "height", 1)
  (canvas, getContext(canvas, "2d"))
}

let isSupported = (canvas, context) => isTruthy(context) && isTruthy(get(canvas, "toDataURL"))

let doesSupportWinding = context => {
  call4(context, "rect", 0, 0, 10, 10)->ignore
  call4(context, "rect", 2, 2, 6, 6)->ignore
  !asBool(call3(context, "isPointInPath", 5, 5, "evenodd"))
}

let canvasToString = canvas => toDataURL(canvas)

let renderTextImage = (canvas, context) => {
  set(canvas, "width", 240)
  set(canvas, "height", 60)
  set(context, "textBaseline", "alphabetic")
  set(context, "fillStyle", "#f60")
  call4(context, "fillRect", 100, 1, 62, 20)->ignore
  set(context, "fillStyle", "#069")
  set(context, "font", "11pt \"Times New Roman\"")
  let printedText = "Cwm fjordbank gly " ++ stringFromCharCode2(55357, 56835)
  call3(context, "fillText", printedText, 2, 15)->ignore
  set(context, "fillStyle", "rgba(102, 204, 0, 0.2)")
  set(context, "font", "18pt Arial")
  call3(context, "fillText", printedText, 4, 45)->ignore
}

let renderGeometryImage = (canvas, context) => {
  set(canvas, "width", 122)
  set(canvas, "height", 110)
  set(context, "globalCompositeOperation", "multiply")
  let colors = [("#f2f", 40, 40), ("#2ff", 80, 40), ("#ff2", 60, 80)]
  let rec loop = i =>
    if i >= arrayLength(colors) {
      ()
    } else {
      let (color, x, y) = getInt(colors, i)
      set(context, "fillStyle", color)
      call0(context, "beginPath")->ignore
      call6(context, "arc", x, y, 40, 0., mathPI *. 2., true)->ignore
      call0(context, "closePath")->ignore
      call0(context, "fill")->ignore
      loop(i + 1)
    }
  loop(0)
  set(context, "fillStyle", "#f9c")
  call6(context, "arc", 60, 60, 60, 0., mathPI *. 2., true)->ignore
  call6(context, "arc", 60, 60, 20, 0., mathPI *. 2., true)->ignore
  call1(context, "fill", "evenodd")->ignore
}

let renderImages = (canvas, context) => {
  renderTextImage(canvas, context)
  let textImage1 = canvasToString(canvas)
  let textImage2 = canvasToString(canvas)
  if textImage1 != textImage2 {
    ("unstable", "unstable")
  } else {
    renderGeometryImage(canvas, context)
    (canvasToString(canvas), textImage1)
  }
}

let getUnstableCanvasFingerprint = (skipImages: any) => {
  let winding = ref(false)
  let (canvas, context) = makeCanvasContext()
  let (geometry, text) = if !isSupported(canvas, context) {
    ("unsupported", "unsupported")
  } else {
    winding := doesSupportWinding(context)
    if isTruthy(skipImages) {
      ("skipped", "skipped")
    } else {
      renderImages(canvas, context)
    }
  }
  let result = emptyObject()
  set(result, "winding", winding.contents)
  set(result, "geometry", geometry)
  set(result, "text", text)
  result
}

let getCanvasFingerprint = () =>
  getUnstableCanvasFingerprint(identity(doesBrowserPerformAntiFingerprinting()))
