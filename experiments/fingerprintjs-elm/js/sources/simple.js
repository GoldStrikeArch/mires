import { replaceNaN, toFloat, toInt } from '../data.js'
import {
  isChromium,
  isChromium86OrNewer,
  isDesktopWebKit,
  isEdgeHTML,
  isGecko,
  isGecko143OrNewer,
  isIPad,
  isTrident,
  isWebKit,
  isWebKit616OrNewer,
  isSafariWebKit,
} from '../browser.js'
import { isAnyParentCrossOrigin } from '../dom.js'

export async function getUserAgentData() {
  const uaData = navigator.userAgentData
  if (!uaData) return undefined

  const filteredBrands = uaData.brands.filter(({ brand }) => !/not/i.test(brand)).map(({ brand }) => brand)
  const brands = filteredBrands.length > 1 ? filteredBrands.filter((b) => b !== 'Chromium') : filteredBrands
  const result = {
    brands,
    mobile: uaData.mobile,
    platform: uaData.platform,
  }

  if (uaData.getHighEntropyValues) {
    try {
      const highEntropy = await uaData.getHighEntropyValues(['architecture', 'bitness', 'model', 'platformVersion'])
      result.architecture = highEntropy.architecture
      result.bitness = highEntropy.bitness
      result.model = highEntropy.model
      result.platformVersion = highEntropy.platformVersion
    } catch (error) {
      if (error instanceof DOMException && error.name === 'NotAllowedError') {
        result.highEntropyStatus = 'not_allowed'
      } else {
        throw error
      }
    }
  }

  return result
}

export function getOsCpu() {
  return navigator.oscpu
}

export function getLanguages() {
  const n = navigator
  const result = []
  const language = n.language || n.userLanguage || n.browserLanguage || n.systemLanguage
  if (language !== undefined) {
    result.push([language])
  }
  if (Array.isArray(n.languages)) {
    if (!(isChromium() && isChromium86OrNewer())) {
      result.push(n.languages)
    }
  } else if (typeof n.languages === 'string') {
    if (n.languages) result.push(n.languages.split(','))
  }
  return result
}

export function getColorDepth() {
  return window.screen.colorDepth
}

export function getDeviceMemory() {
  return replaceNaN(toFloat(navigator.deviceMemory), undefined)
}

export function getUnstableScreenResolution() {
  const s = screen
  const parseDimension = (value) => replaceNaN(toInt(value), null)
  const dimensions = [parseDimension(s.width), parseDimension(s.height)]
  dimensions.sort().reverse()
  return dimensions
}

export function getScreenResolution() {
  if (isWebKit() && isWebKit616OrNewer() && isSafariWebKit()) {
    return undefined
  }
  return getUnstableScreenResolution()
}

export function getUnstableHardwareConcurrency() {
  return replaceNaN(toInt(navigator.hardwareConcurrency), undefined)
}

export function getHardwareConcurrency() {
  const value = getUnstableHardwareConcurrency()
  if (value !== undefined && isGecko() && isGecko143OrNewer()) {
    return value >= 8 ? 8 : 4
  }
  return value
}

export function getTimezone() {
  const DateTimeFormat = window.Intl?.DateTimeFormat
  if (DateTimeFormat) {
    const timezone = new DateTimeFormat().resolvedOptions().timeZone
    if (timezone) return timezone
  }
  const offset = -getTimezoneOffset()
  return `UTC${offset >= 0 ? '+' : ''}${offset}`
}

function getTimezoneOffset() {
  const currentYear = new Date().getFullYear()
  return Math.max(
    toFloat(new Date(currentYear, 0, 1).getTimezoneOffset()),
    toFloat(new Date(currentYear, 6, 1).getTimezoneOffset()),
  )
}

export function getSessionStorage() {
  try {
    return !!window.sessionStorage
  } catch {
    return true
  }
}

export function getLocalStorage() {
  try {
    return !!window.localStorage
  } catch {
    return true
  }
}

export function getIndexedDB() {
  if (isTrident() || isEdgeHTML()) return undefined
  try {
    return !!window.indexedDB
  } catch {
    return true
  }
}

export function getOpenDatabase() {
  return !!window.openDatabase
}

export function getCpuClass() {
  return navigator.cpuClass
}

export function getPlatform() {
  const { platform } = navigator
  if (platform === 'MacIntel') {
    if (isWebKit() && !isDesktopWebKit()) {
      return isIPad() ? 'iPad' : 'iPhone'
    }
  }
  return platform
}

export function getPlugins() {
  const rawPlugins = navigator.plugins
  if (!rawPlugins) return undefined
  const plugins = []
  for (let i = 0; i < rawPlugins.length; ++i) {
    const plugin = rawPlugins[i]
    if (!plugin) continue
    const mimeTypes = []
    for (let j = 0; j < plugin.length; ++j) {
      const mimeType = plugin[j]
      mimeTypes.push({ type: mimeType.type, suffixes: mimeType.suffixes })
    }
    plugins.push({ name: plugin.name, description: plugin.description, mimeTypes })
  }
  return plugins
}

export function getTouchSupport() {
  const n = navigator
  let maxTouchPoints = 0
  let touchEvent
  if (n.maxTouchPoints !== undefined) {
    maxTouchPoints = toInt(n.maxTouchPoints)
  } else if (n.msMaxTouchPoints !== undefined) {
    maxTouchPoints = n.msMaxTouchPoints
  }
  try {
    document.createEvent('TouchEvent')
    touchEvent = true
  } catch {
    touchEvent = false
  }
  return {
    maxTouchPoints,
    touchEvent,
    touchStart: 'ontouchstart' in window,
  }
}

export function getVendor() {
  return navigator.vendor || ''
}

export function getVendorFlavors() {
  const flavors = []
  for (const key of [
    'chrome',
    'safari',
    '__crWeb',
    '__gCrWeb',
    'yandex',
    '__yb',
    '__ybro',
    '__firefox__',
    '__edgeTrackingPreventionStatistics',
    'webkit',
    'oprt',
    'samsungAr',
    'ucweb',
    'UCShellJava',
    'puffinDevice',
  ]) {
    const value = window[key]
    if (value && typeof value === 'object') flavors.push(key)
  }
  return flavors.sort()
}

export function areCookiesEnabled() {
  const d = document
  try {
    d.cookie = 'cookietest=1; SameSite=Strict;'
    const result = d.cookie.indexOf('cookietest=') !== -1
    d.cookie = 'cookietest=1; SameSite=Strict; expires=Thu, 01-Jan-1970 00:00:01 GMT'
    return result
  } catch {
    return false
  }
}

export function isPdfViewerEnabled() {
  return navigator.pdfViewerEnabled
}

export function getArchitecture() {
  const f = new Float32Array(1)
  const u8 = new Uint8Array(f.buffer)
  f[0] = Infinity
  f[0] = f[0] - f[0]
  return u8[3]
}

export function getApplePayState() {
  const { ApplePaySession } = window
  if (typeof ApplePaySession?.canMakePayments !== 'function') return -1
  if (isAnyParentCrossOrigin()) return -3
  try {
    return ApplePaySession.canMakePayments() ? 1 : 0
  } catch (error) {
    if (error instanceof Error && error.name === 'InvalidAccessError' && /\bfrom\b.*\binsecure\b/i.test(error.message)) {
      return -2
    }
    throw error
  }
}

export function getPrivateClickMeasurement() {
  const link = document.createElement('a')
  const sourceId = link.attributionSourceId ?? link.attributionsourceid
  return sourceId === undefined ? undefined : String(sourceId)
}

export function getDateTimeLocale() {
  if (!window.Intl) return -1
  const DateTimeFormat = window.Intl.DateTimeFormat
  if (!DateTimeFormat) return -2
  const locale = DateTimeFormat().resolvedOptions().locale
  if (!locale && locale !== '') return -3
  return locale
}
