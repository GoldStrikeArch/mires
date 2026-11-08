import { isGecko, isGecko120OrNewer, isSafariWebKit, isWebKit, isWebKit616OrNewer } from '../browser.js'

export function getCanvasFingerprint() {
  return getUnstableCanvasFingerprint(doesBrowserPerformAntiFingerprinting())
}

export function getUnstableCanvasFingerprint(skipImages) {
  let winding = false
  let geometry
  let text
  const [canvas, context] = makeCanvasContext()
  if (!isSupported(canvas, context)) {
    geometry = text = 'unsupported'
  } else {
    winding = doesSupportWinding(context)
    if (skipImages) {
      geometry = text = 'skipped'
    } else {
      ;[geometry, text] = renderImages(canvas, context)
    }
  }
  return { winding, geometry, text }
}

function makeCanvasContext() {
  const canvas = document.createElement('canvas')
  canvas.width = 1
  canvas.height = 1
  return [canvas, canvas.getContext('2d')]
}

function isSupported(canvas, context) {
  return !!(context && canvas.toDataURL)
}

function doesSupportWinding(context) {
  context.rect(0, 0, 10, 10)
  context.rect(2, 2, 6, 6)
  return !context.isPointInPath(5, 5, 'evenodd')
}

function renderImages(canvas, context) {
  renderTextImage(canvas, context)
  const textImage1 = canvasToString(canvas)
  const textImage2 = canvasToString(canvas)
  if (textImage1 !== textImage2) {
    return ['unstable', 'unstable']
  }
  renderGeometryImage(canvas, context)
  const geometryImage = canvasToString(canvas)
  return [geometryImage, textImage1]
}

function renderTextImage(canvas, context) {
  canvas.width = 240
  canvas.height = 60
  context.textBaseline = 'alphabetic'
  context.fillStyle = '#f60'
  context.fillRect(100, 1, 62, 20)
  context.fillStyle = '#069'
  context.font = '11pt "Times New Roman"'
  const printedText = `Cwm fjordbank gly ${String.fromCharCode(55357, 56835)}`
  context.fillText(printedText, 2, 15)
  context.fillStyle = 'rgba(102, 204, 0, 0.2)'
  context.font = '18pt Arial'
  context.fillText(printedText, 4, 45)
}

function renderGeometryImage(canvas, context) {
  canvas.width = 122
  canvas.height = 110
  context.globalCompositeOperation = 'multiply'
  for (const [color, x, y] of [
    ['#f2f', 40, 40],
    ['#2ff', 80, 40],
    ['#ff2', 60, 80],
  ]) {
    context.fillStyle = color
    context.beginPath()
    context.arc(x, y, 40, 0, Math.PI * 2, true)
    context.closePath()
    context.fill()
  }
  context.fillStyle = '#f9c'
  context.arc(60, 60, 60, 0, Math.PI * 2, true)
  context.arc(60, 60, 20, 0, Math.PI * 2, true)
  context.fill('evenodd')
}

function canvasToString(canvas) {
  return canvas.toDataURL()
}

function doesBrowserPerformAntiFingerprinting() {
  const isSafari17OrAbove = isWebKit() && isWebKit616OrNewer() && isSafariWebKit()
  const isFirefox120OrAbove = isGecko() && isGecko120OrNewer()
  return isSafari17OrAbove || isFirefox120OrAbove
}
