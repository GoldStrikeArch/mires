import { withIframe } from '../dom.js'

const testString = 'mmMwWLliI0O&1'
const textSize = '48px'
const baseFonts = ['monospace', 'sans-serif', 'serif']
const fontList = [
  'sans-serif-thin',
  'ARNO PRO',
  'Agency FB',
  'Arabic Typesetting',
  'Arial Unicode MS',
  'AvantGarde Bk BT',
  'BankGothic Md BT',
  'Batang',
  'Bitstream Vera Sans Mono',
  'Calibri',
  'Century',
  'Century Gothic',
  'Clarendon',
  'EUROSTILE',
  'Franklin Gothic',
  'Futura Bk BT',
  'Futura Md BT',
  'GOTHAM',
  'Gill Sans',
  'HELV',
  'Haettenschweiler',
  'Helvetica Neue',
  'Humanst521 BT',
  'Leelawadee',
  'Letter Gothic',
  'Levenim MT',
  'Lucida Bright',
  'Lucida Sans',
  'Menlo',
  'MS Mincho',
  'MS Outlook',
  'MS Reference Specialty',
  'MS UI Gothic',
  'MT Extra',
  'MYRIAD PRO',
  'Marlett',
  'Meiryo UI',
  'Microsoft Uighur',
  'Minion Pro',
  'Monotype Corsiva',
  'PMingLiU',
  'Pristina',
  'SCRIPTINA',
  'Segoe UI Light',
  'Serifa',
  'SimHei',
  'Small Fonts',
  'Staccato222 BT',
  'TRAJAN PRO',
  'Univers CE 55 Medium',
  'Vrinda',
  'ZWAdobeF',
]

export function getFonts() {
  return withIframe(async (_, { document }) => {
    const holder = document.body
    holder.style.fontSize = textSize

    const spansContainer = document.createElement('div')
    spansContainer.style.setProperty('visibility', 'hidden', 'important')

    const defaultWidth = {}
    const defaultHeight = {}

    const createSpan = (fontFamily) => {
      const span = document.createElement('span')
      const { style } = span
      style.position = 'absolute'
      style.top = '0'
      style.left = '0'
      style.fontFamily = fontFamily
      span.textContent = testString
      spansContainer.appendChild(span)
      return span
    }

    const createSpanWithFonts = (fontToDetect, baseFont) => createSpan(`'${fontToDetect}',${baseFont}`)

    const baseFontsSpans = baseFonts.map(createSpan)
    const fontsSpans = {}
    for (const font of fontList) {
      fontsSpans[font] = baseFonts.map((baseFont) => createSpanWithFonts(font, baseFont))
    }

    holder.appendChild(spansContainer)

    for (let index = 0; index < baseFonts.length; index++) {
      defaultWidth[baseFonts[index]] = baseFontsSpans[index].offsetWidth
      defaultHeight[baseFonts[index]] = baseFontsSpans[index].offsetHeight
    }

    return fontList.filter((font) =>
      baseFonts.some(
        (baseFont, baseFontIndex) =>
          fontsSpans[font][baseFontIndex].offsetWidth !== defaultWidth[baseFont] ||
          fontsSpans[font][baseFontIndex].offsetHeight !== defaultHeight[baseFont],
      ),
    )
  })
}
