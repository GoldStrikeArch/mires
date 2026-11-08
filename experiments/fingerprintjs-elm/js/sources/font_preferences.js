import { isAndroid, isChromium, isChromium128OrNewer, isWebKit } from '../browser.js'
import { withIframe } from '../dom.js'

const defaultText = 'mmMwWLliI0fiflO&1'

export const presets = {
  default: [],
  apple: [{ font: '-apple-system-body' }],
  serif: [{ fontFamily: 'serif' }],
  sans: [{ fontFamily: 'sans-serif' }],
  mono: [{ fontFamily: 'monospace' }],
  min: [{ fontSize: '1px' }],
  system: [{ fontFamily: 'system-ui' }],
}

export function getFontPreferences() {
  return withNaturalFonts((document, container, iframeWindow) => {
    const elements = {}
    const sizes = {}

    for (const key of Object.keys(presets)) {
      const [style = {}, text = defaultText] = presets[key]
      const element = document.createElement('span')
      element.textContent = text
      element.style.whiteSpace = 'nowrap'
      for (const name of Object.keys(style)) {
        const value = style[name]
        if (value !== undefined) element.style[name] = value
      }
      elements[key] = element
      container.append(document.createElement('br'), element)
    }

    const shouldNormalizeMeasurement = isChromium() && isChromium128OrNewer()
    for (const key of Object.keys(presets)) {
      const rawWidth = elements[key].getBoundingClientRect().width
      sizes[key] = shouldNormalizeMeasurement
        ? normalizeFontMeasurement(rawWidth * iframeWindow.devicePixelRatio)
        : rawWidth
    }

    return sizes
  })
}

function normalizeFontMeasurement(value) {
  const decimals = isAndroid() ? 0 : 3
  const pow = Math.pow(10, decimals)
  return Math.floor(value * pow) / pow
}

function withNaturalFonts(action, containerWidthPx = 4000) {
  return withIframe((_, iframeWindow) => {
    const iframeDocument = iframeWindow.document
    const iframeBody = iframeDocument.body
    const bodyStyle = iframeBody.style
    bodyStyle.width = `${containerWidthPx}px`
    bodyStyle.webkitTextSizeAdjust = bodyStyle.textSizeAdjust = 'none'

    if (isChromium()) {
      iframeBody.style.zoom = `${1 / iframeWindow.devicePixelRatio}`
    } else if (isWebKit()) {
      iframeBody.style.zoom = 'reset'
    }

    const linesOfText = iframeDocument.createElement('div')
    linesOfText.textContent = [...Array((containerWidthPx / 20) << 0)].map(() => 'word').join(' ')
    iframeBody.appendChild(linesOfText)

    return action(iframeDocument, iframeBody, iframeWindow)
  }, '<!doctype html><html><head><meta name="viewport" content="width=device-width, initial-scale=1">')
}
