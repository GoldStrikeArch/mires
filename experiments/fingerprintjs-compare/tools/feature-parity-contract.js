(function () {
  'use strict'

  function meta(fn) {
    return { type: typeof fn, name: fn && fn.name, length: fn && fn.length }
  }

  function encode(value, seen = new WeakSet()) {
    if (value === undefined) return { $type: 'undefined' }
    if (typeof value === 'number') {
      if (Number.isNaN(value)) return { $type: 'number', value: 'NaN' }
      if (value === Infinity) return { $type: 'number', value: 'Infinity' }
      if (value === -Infinity) return { $type: 'number', value: '-Infinity' }
      if (Object.is(value, -0)) return { $type: 'number', value: '-0' }
      return value
    }
    if (typeof value === 'bigint') return { $type: 'bigint', value: String(value) }
    if (typeof value === 'function') return { $type: 'function', name: value.name, length: value.length }
    if (!value || typeof value !== 'object') return value
    if (value instanceof Error) return { $type: 'error', name: value.name, message: value.message }
    if (seen.has(value)) return { $type: 'cycle' }
    seen.add(value)
    if (Array.isArray(value)) return value.map(item => encode(item, seen))
    const result = {}
    for (const key of Object.keys(value)) result[key] = encode(value[key], seen)
    return result
  }


  function timedValue(label, milliseconds = 5000) {
    return new Promise((resolve) => setTimeout(() => resolve({ $type: 'timeout', label }), milliseconds))
  }

  function bounded(value, label, milliseconds = 5000) {
    return Promise.race([Promise.resolve(value), timedValue(label, milliseconds)])
  }

  async function settleMaybeTwoStage(value) {
    let result = await value
    if (typeof result === 'function') result = await result()
    return encode(result)
  }

  async function iframeAudit(module) {
    const before = document.querySelectorAll('iframe').length
    const value = await bounded(module.withIframe(async (iframe, iWindow) => {
      await Promise.resolve()
      return {
        connected: iframe.isConnected,
        display: iframe.style.getPropertyValue('display'),
        displayPriority: iframe.style.getPropertyPriority('display'),
        position: iframe.style.position,
        top: iframe.style.top,
        left: iframe.style.left,
        visibility: iframe.style.visibility,
        sameWindow: iWindow === iframe.contentWindow,
        bodyText: iWindow.document.body && iWindow.document.body.textContent,
        marker: !!iWindow.document.getElementById('feature-parity-marker'),
      }
    }, '<!doctype html><html><body><div id="feature-parity-marker">marker</div></body></html>', 1), 'iframe-success')
    const afterSuccess = document.querySelectorAll('iframe').length
    let error
    try {
      const rejected = await bounded(module.withIframe(() => { throw new TypeError('feature iframe error') }, undefined, 1), 'iframe-error')
      if (rejected && rejected.$type === 'timeout') error = rejected
    } catch (caught) {
      error = encode(caught)
    }
    const afterError = document.querySelectorAll('iframe').length
    return { before, value, afterSuccess, error, afterError }
  }

  async function webglAudit(module) {
    const cache = {}
    const first = module.getWebGLContext(cache)
    const second = module.getWebGLContext(cache)
    return {
      available: !!first,
      same: first === second,
      cacheKeys: Object.keys(cache),
      webglKeys: cache.webgl ? Object.keys(cache.webgl) : [],
      constructor: first && first.constructor && first.constructor.name,
      canvasTag: first && first.canvas && first.canvas.tagName,
      hasGetParameter: !!first && typeof first.getParameter === 'function',
    }
  }



  function summarizeDebugLog(args) {
    const text = String(args[0])
    if (text.startsWith('DOM blockers debug:\n```')) {
      return { kind: 'domBlockers', text }
    }
    if (text.startsWith('Copy the text below to get the debug data:')) {
      const version = /(?:^|\n)version: ([^\n]*)/.exec(text)?.[1]
      const userAgent = /(?:^|\n)userAgent: ([^\n]*)/.exec(text)?.[1]
      const timing = /(?:^|\n)timeBetweenLoadAndGet: ([^\n]*)/.exec(text)?.[1]
      const visitorId = /(?:^|\n)visitorId: ([^\n]*)/.exec(text)?.[1]
      const componentsText = /(?:^|\n)components: ([\s\S]*)\n```$/.exec(text)?.[1]
      let componentKeys
      let componentKinds
      let componentsParseError
      try {
        const components = JSON.parse(componentsText)
        componentKeys = Object.keys(components)
        componentKinds = Object.fromEntries(
          Object.entries(components).map(([key, component]) => [
            key,
            component && Object.prototype.hasOwnProperty.call(component, 'error') ? 'error' : 'value',
          ]),
        )
      } catch (error) {
        componentsParseError = String(error)
      }
      return {
        kind: 'agent',
        version,
        userAgent,
        timingIsNumeric: timing !== undefined && Number.isFinite(Number(timing)),
        visitorId,
        componentKeys,
        componentKinds,
        componentsParseError,
      }
    }
    return { kind: 'other', args: args.map((value) => encode(value)) }
  }

  async function captureDebug(module, loadDebug, getDebug, label) {
    const logs = []
    const originalLog = console.log
    console.log = (...args) => logs.push(summarizeDebugLog(args))
    try {
      const agent = await bounded(
        module.load({ delayFallback: 0, monitoring: false, debug: loadDebug }),
        `debug-load-${label}`,
      )
      if (agent && agent.$type === 'timeout') return { timeout: agent, logs }
      const result = await bounded(
        agent.get(getDebug ? { debug: true } : undefined),
        `debug-get-${label}`,
      )
      return { timeout: result && result.$type === 'timeout' ? result : undefined, logs }
    } finally {
      console.log = originalLog
    }
  }

  async function debugAudit(module) {
    return {
      disabled: await captureDebug(module, false, false, 'disabled'),
      loadOption: await captureDebug(module, true, false, 'load'),
      getOption: await captureDebug(module, false, true, 'get'),
    }
  }

  async function captureMonitoring(module, options, label) {
    const requests = []
    const originalRandom = Math.random
    const originalXhr = globalThis.XMLHttpRequest
    const hadDisableMarker = Object.prototype.hasOwnProperty.call(window, '__fpjs_d_m')
    const originalDisableMarker = window.__fpjs_d_m
    Math.random = () => 0
    try {
      window.__fpjs_d_m = false
    } catch (_) {
      // Best effort; the local oracle build does not freeze this marker.
    }
    function MockXMLHttpRequest() {}
    MockXMLHttpRequest.prototype.open = function (method, url, async) {
      this.__request = [String(method), String(url), Boolean(async)]
    }
    MockXMLHttpRequest.prototype.send = function () {
      requests.push(this.__request || ['<send-without-open>'])
    }
    globalThis.XMLHttpRequest = MockXMLHttpRequest
    try {
      const loaded = await bounded(module.load(options), `monitoring-${label}`)
      return { requests, load: encode(loaded && loaded.$type === 'timeout' ? loaded : undefined) }
    } finally {
      Math.random = originalRandom
      globalThis.XMLHttpRequest = originalXhr
      try {
        if (hadDisableMarker) window.__fpjs_d_m = originalDisableMarker
        else delete window.__fpjs_d_m
      } catch (_) {
        // Best effort restoration.
      }
    }
  }

  async function monitoringAudit(module) {
    return {
      default: await captureMonitoring(module, { delayFallback: 0 }, 'default'),
      explicitTrue: await captureMonitoring(module, { delayFallback: 0, monitoring: true }, 'true'),
      explicitFalse: await captureMonitoring(module, { delayFallback: 0, monitoring: false }, 'false'),
    }
  }

  async function agentAudit(module) {
    const agent = await bounded(module.load({ delayFallback: 0, monitoring: false }), 'agent-load')
    if (agent && agent.$type === 'timeout') return agent
    const descriptor = Object.getOwnPropertyDescriptor(agent, 'get')
    return {
      keys: Object.keys(agent),
      get: meta(agent.get),
      descriptor: descriptor && {
        configurable: descriptor.configurable,
        enumerable: descriptor.enumerable,
        writable: descriptor.writable,
        hasValue: typeof descriptor.value === 'function',
      },
    }
  }

  globalThis.captureFeatureParity = async function (moduleUrl) {
    const module = await import(`${moduleUrl}?feature=${Math.random()}`)
    const exportFunctions = {}
    for (const key of Object.keys(module)) {
      if (typeof module[key] === 'function') exportFunctions[key] = meta(module[key])
    }
    const prepare = module.prepareForSources(0)
    const prepareShape = { then: typeof prepare?.then, constructor: prepare?.constructor?.name }
    const prepareResult = await bounded(prepare, 'prepare')

    const screenFrameGetter = module.getUnstableScreenFrame()
    return {
      exportFunctions,
      agent: await agentAudit(module),
      browserFlags: {
        isAndroid: module.isAndroid(),
        isChromium: module.isChromium(),
        isDesktopWebKit: module.isDesktopWebKit(),
        isEdgeHTML: module.isEdgeHTML(),
        isGecko: module.isGecko(),
        isSamsungInternet: module.isSamsungInternet(),
        isTrident: module.isTrident(),
        isWebKit: module.isWebKit(),
      },
      fullscreen: encode(module.getFullscreenElement()),
      unstable: {
        audio: encode(module.getUnstableAudioFingerprint()),
        canvasFalse: encode(module.getUnstableCanvasFingerprint(false)),
        canvasTrue: encode(module.getUnstableCanvasFingerprint(true)),
        hardwareConcurrency: encode(module.getUnstableHardwareConcurrency()),
        screenFrame: await bounded(settleMaybeTwoStage(screenFrameGetter), 'screen-frame'),
        screenResolution: encode(module.getUnstableScreenResolution()),
      },
      webgl: await webglAudit(module),
      iframe: await iframeAudit(module),
      prepare: { ...prepareShape, result: encode(prepareResult) },
      monitoring: await monitoringAudit(module),
      debug: await debugAudit(module),
    }
  }
})()
