import { requestIdleCallbackIfAvailable } from './async.js'
import { elmCall, elmHash } from './elm_bridge.js'
import { errorToObject } from './misc.js'
import {
  isAndroid,
  isDesktopWebKit,
  isSafariWebKit,
  isWebKit,
  isWebKit616OrNewer,
} from './browser.js'
import { loadSources } from './entropy.js'
import { sources } from './sources/index.js'

const version = '5.2.0'

function componentItems(components) {
  return Object.keys(components).map((key) => {
    const component = components[key]
    if ('error' in component) {
      return { key, value: 'error' }
    }
    const json = JSON.stringify(component.value)
    return { key, value: json === undefined ? 'undefined' : json }
  })
}

export function componentsToDebugString(components) {
  return JSON.stringify(
    components,
    (_key, value) => {
      if (value instanceof Error) {
        return errorToObject(value)
      }
      return value
    },
    2,
  )
}

export function hashComponents(components) {
  const canonical = elmCall({ op: 'canonical', items: componentItems(components) })
  return elmHash(canonical, 0)
}

export function murmurX64Hash128(input, seed) {
  return elmHash(input, seed)
}

function getConfidence(components) {
  const platform = 'value' in components.platform ? components.platform.value : ''
  return elmCall({
    op: 'confidence',
    platform,
    android: isAndroid(),
    webkit: isWebKit(),
    desktopWebkit: isDesktopWebKit(),
    webkit616: isWebKit616OrNewer(),
    safariWebkit: isSafariWebKit(),
  })
}

function makeLazyGetResult(components) {
  let visitorIdCache
  const confidence = getConfidence(components)
  return {
    get visitorId() {
      if (visitorIdCache === undefined) {
        visitorIdCache = hashComponents(this.components)
      }
      return visitorIdCache
    },
    set visitorId(visitorId) {
      visitorIdCache = visitorId
    },
    confidence,
    components,
    version,
  }
}

export function prepareForSources(delayFallback = 50) {
  return requestIdleCallbackIfAvailable(delayFallback, delayFallback * 2)
}

function makeAgent(getComponents, debug) {
  const creationTime = Date.now()
  return {
    async get(options) {
      const startTime = Date.now()
      const components = await getComponents()
      const result = makeLazyGetResult(components)
      if (debug || options?.debug) {
        console.log(`Copy the text below to get the debug data:

\`\`\`
version: ${result.version}
userAgent: ${navigator.userAgent}
timeBetweenLoadAndGet: ${startTime - creationTime}
visitorId: ${result.visitorId}
components: ${componentsToDebugString(components)}
\`\`\``)
      }
      return result
    },
  }
}

function loadBuiltinSources(options) {
  return loadSources(sources, options, [])
}

function monitor() {
  if (window.__fpjs_d_m || Math.random() >= 0.001) {
    return
  }
  try {
    const request = new XMLHttpRequest()
    request.open('get', `https://m1.openfpcdn.io/fingerprintjs/v${version}/npm-monitoring`, true)
    request.send()
  } catch (error) {
    console.error(error)
  }
}

export async function load(options = {}) {
  const { delayFallback, debug, monitoring = true } = options
  if (monitoring) {
    monitor()
  }
  await prepareForSources(delayFallback)
  const getComponents = loadBuiltinSources({ cache: {}, debug })
  return makeAgent(getComponents, debug)
}
