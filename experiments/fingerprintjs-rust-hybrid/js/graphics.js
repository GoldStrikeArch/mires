// One-function DOM / canvas / WebGL / audio wrappers. Probe choice stays in Rust.

export function createCanvas() {
  return document.createElement('canvas')
}

export function get2dContext(canvas) {
  return canvas.getContext('2d')
}

export function canvasToDataURL(canvas) {
  return canvas.toDataURL()
}

export function canvasTestText() {
  return 'Cwm fjordbank gly ' + String.fromCharCode(55357, 56835)
}

export function supportsWinding(context) {
  context.rect(0, 0, 10, 10)
  context.rect(2, 2, 6, 6)
  return !context.isPointInPath(5, 5, 'evenodd')
}

export function renderTextImage(canvas, context) {
  canvas.width = 240
  canvas.height = 60
  context.textBaseline = 'alphabetic'
  context.fillStyle = '#f60'
  context.fillRect(100, 1, 62, 20)
  context.fillStyle = '#069'
  context.font = '11pt "Times New Roman"'
  const text = canvasTestText()
  context.fillText(text, 2, 15)
  context.fillStyle = 'rgba(102, 204, 0, 0.2)'
  context.font = '18pt Arial'
  context.fillText(text, 4, 45)
}

export function renderGeometryImage(canvas, context) {
  canvas.width = 122
  canvas.height = 110
  context.globalCompositeOperation = 'multiply'
  const circles = [
    ['#f2f', 40, 40],
    ['#2ff', 80, 40],
    ['#ff2', 60, 80],
  ]
  for (let i = 0; i < circles.length; i++) {
    const [color, x, y] = circles[i]
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

export function cachedWebglContext(cache) {
  if (cache.webgl) return cache.webgl.context
  const canvas = document.createElement('canvas')
  let context
  canvas.addEventListener('webglCreateContextError', function () {
    context = undefined
  })
  const types = ['webgl', 'experimental-webgl']
  for (let i = 0; i < types.length; i++) {
    try {
      context = canvas.getContext(types[i])
    } catch (_) {
      // The rust host used try_host around getContext.
    }
    if (context) break
  }
  cache.webgl = { context }
  return context
}

export function createOfflineAudioContext(channels, length, sampleRate) {
  const Ctor = window.OfflineAudioContext || window.webkitOfflineAudioContext
  return new Ctor(channels, length, sampleRate)
}

export function configureAudioGraph(context) {
  const oscillator = context.createOscillator()
  oscillator.type = 'triangle'
  oscillator.frequency.value = 10000
  const compressor = context.createDynamicsCompressor()
  compressor.threshold.value = -50
  compressor.knee.value = 40
  compressor.ratio.value = 12
  compressor.attack.value = 0
  compressor.release.value = 0.25
  oscillator.connect(compressor)
  compressor.connect(context.destination)
  oscillator.start(0)
}

export function hashAudioSignal(signal) {
  let hash = 0
  for (let i = 0; i < signal.length; i++) hash += Math.abs(signal[i])
  return hash
}
