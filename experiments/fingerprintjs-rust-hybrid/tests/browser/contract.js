(function () {
  'use strict'

  function descriptor(value, key) {
    const item = Object.getOwnPropertyDescriptor(value, key)
    return {
      configurable: item.configurable,
      enumerable: item.enumerable,
      getter: typeof item.get === 'function',
      getterName: item.get?.name,
      setter: typeof item.set === 'function',
      setterName: item.set?.name,
      writable: item.writable,
    }
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
    if (value instanceof Error) {
      const own = {}
      for (const key of Object.getOwnPropertyNames(value)) {
        if (key !== 'message' && key !== 'name' && key !== 'stack') own[key] = encode(value[key], seen)
      }
      return {
        $type: 'error',
        constructor: value.constructor && value.constructor.name,
        message: value.message,
        name: value.name,
        own,
        stack: typeof value.stack === 'string' ? value.stack : undefined,
      }
    }
    if (seen.has(value)) return { $type: 'cycle' }
    seen.add(value)
    if (Array.isArray(value)) return value.map((item) => encode(item, seen))
    const result = {}
    for (const key of Object.keys(value)) result[key] = encode(value[key], seen)
    return result
  }

  function components(value) {
    const structure = {}
    const durations = {}
    for (const key of Object.keys(value)) {
      const component = value[key]
      durations[key] = component.duration
      structure[key] = 'error' in component
        ? { branch: 'error', error: encode(component.error) }
        : { branch: 'value', value: encode(component.value) }
    }
    return { durations, structure }
  }

  function testError(message) {
    const error = new TypeError(message)
    error.fixture = { code: 17 }
    return error
  }

  async function customSources(module) {
    const events = []
    let stageGets = 0
    let unhandled = 0
    const onUnhandled = (event) => {
      ++unhandled
      event.preventDefault()
    }
    addEventListener('unhandledrejection', onUnhandled)
    const source = function (options) {
      events.push(['this', this === null, options.token])
      return 'sync'
    }
    const sources = {
      sync: source,
      promise: async () => 'promise',
      stage: () => () => `stage-${++stageGets}`,
      asyncStage: async () => async () => 'async-stage',
      syncThrow: () => { throw testError('sync load') },
      loadReject: () => Promise.reject(testError('async load')),
      getThrow: () => () => { throw testError('sync get') },
      getReject: () => () => Promise.reject(testError('async get')),
      transformed: module.transformSource(() => () => 21, (value) => value * 2),
      excluded: () => { throw new Error('excluded source ran') },
    }
    const getComponents = module.loadSources(sources, { token: 'token' }, ['excluded'])
    const immediateEvents = encode(events)
    const first = await getComponents()
    const second = await getComponents()
    await new Promise((resolve) => setTimeout(resolve, 0))
    removeEventListener('unhandledrejection', onUnhandled)
    return {
      immediateEvents,
      events: encode(events),
      first: components(first),
      second: components(second),
      unhandled,
    }
  }

  async function builtins(module) {
    const agent = await module.load({ delayFallback: 0, monitoring: false })
    const first = await agent.get()
    const before = descriptor(first, 'visitorId')
    const visitorId = first.visitorId
    const after = descriptor(first, 'visitorId')
    const debug = JSON.parse(module.componentsToDebugString(first.components))
    first.visitorId = 'fixture-override'
    const second = await agent.get()
    return {
      first: {
        componentKeys: Object.keys(first.components),
        components: components(first.components),
        confidence: encode(first.confidence),
        debug: encode(debug),
        descriptorAfterRead: after,
        descriptorBeforeRead: before,
        keys: Object.keys(first),
        overriddenVisitorId: first.visitorId,
        version: first.version,
        visitorId,
      },
      second: {
        components: components(second.components),
        confidence: encode(second.confidence),
        keys: Object.keys(second),
        version: second.version,
        visitorId: second.visitorId,
      },
    }
  }

  globalThis.captureContract = async function (moduleUrl) {
    const module = await import(`${moduleUrl}?capture=${Math.random()}`)
    const error = testError('canonical error')
    const hashInput = {
      'z|': { value: '\ud83d\ude00' },
      'a:': { value: { b: 2, a: 1 } },
      'b\\': { error, duration: 1 },
    }
    return {
      builtins: await builtins(module),
      customSources: await customSources(module),
      debugVector: encode(JSON.parse(module.componentsToDebugString(hashInput))),
      defaultExports: Object.keys(module.default),
      exports: Object.keys(module),
      hashVector: module.hashComponents(hashInput),
      murmurVectors: [
        module.murmurX64Hash128('', 0),
        module.murmurX64Hash128('hello', 0),
        module.murmurX64Hash128('Unicode: \ud83d\ude00', 0),
        module.murmurX64Hash128('\ud800 lone surrogate', 17),
      ],
      sourceKeys: Object.keys(module.sources),
      sourceFunctions: Object.fromEntries(
        Object.entries(module.sources).map(([key, source]) => [key, { length: source.length, name: source.name }]),
      ),
    }
  }

  globalThis.captureMonitoring = async function (moduleUrl, explicit) {
    const module = await import(`${moduleUrl}?monitoring=${Math.random()}`)
    const requests = []
    const originalRandom = Math.random
    Math.random = () => 0
    const originalFetch = globalThis.fetch
    globalThis.fetch = (...args) => {
      requests.push(['fetch', String(args[0])])
      return Promise.resolve(new Response())
    }
    const originalBeacon = navigator.sendBeacon
    navigator.sendBeacon = (...args) => {
      requests.push(['beacon', String(args[0])])
      return true
    }
    const originalOpen = XMLHttpRequest.prototype.open
    const originalSend = XMLHttpRequest.prototype.send
    XMLHttpRequest.prototype.open = function (method, url) {
      this.__fixtureUrl = String(url)
      return originalOpen.apply(this, arguments)
    }
    XMLHttpRequest.prototype.send = function () {
      requests.push(['xhr', this.__fixtureUrl])
    }
    try {
      await module.load(explicit ? { delayFallback: 0, monitoring: true } : { delayFallback: 0 })
      return requests
    } finally {
      Math.random = originalRandom
      globalThis.fetch = originalFetch
      navigator.sendBeacon = originalBeacon
      XMLHttpRequest.prototype.open = originalOpen
      XMLHttpRequest.prototype.send = originalSend
    }
  }
})()
