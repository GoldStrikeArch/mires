// Drives workbox-precaching 7.4.1 (with the workbox-core, -routing and -strategies modules it
// imports) through the contract's subset: one call runs a script of API calls on real
// CacheStorage, Request, Response, URL and same-origin fetch. CONTRACT.md defines every step,
// fixture plugin and output record; this file is that definition in JavaScript.
import wb from '../../node_modules/workbox-precaching/matrix-entry.mjs'
// The network fixture's two responses. Importing them puts them in this module's graph, which is
// what the checker serves; the fixture fetches them at run time and never executes them.
import './alpha.mjs'
import './beta.mjs'

const {PrecacheController, PrecacheFallbackPlugin, PrecacheRoute} = wb.precaching
const {Route, Router} = wb.routing
const {CacheFirst, CacheOnly, NetworkFirst, NetworkOnly} = wb.strategies
const {WorkboxError} = wb.core._private
const STRATEGIES = {CacheFirst, CacheOnly, NetworkFirst, NetworkOnly}

const FIXTURE = 'experiments/library-matrix/libraries/workbox/'
// Well-formed SRI metadata that matches no fixture, so the fetch fails integrity.
const WRONG_INTEGRITY = 'sha256-' + 'A'.repeat(43) + '='

export async function run(input) {
  const base = new URL('./', location.href).href
  const prefix = input.cachePrefix ?? 'mires-workbox-' + crypto.randomUUID()
  const owned = input.cachePrefix === undefined
  const relative = url => url.startsWith(base) ? url.slice(base.length) : url
  const cacheName = name => prefix + '-' + name
  const cacheLabel = name => name.startsWith(prefix + '-') ? name.slice(prefix.length + 1) : name

  const network = {source: 'alpha'}
  const controllers = new Map()
  const routers = new Map()
  const routes = new Map()
  const results = []
  const events = []
  const handlers = []

  // Output records.
  // A thrown non-Error (the fail fixture's `value`) has no name; it prints as its string.
  const nameOf = error => error instanceof Error ? error.name : String(error)
  const failure = error => error instanceof WorkboxError
    ? {error: error.name, message: error.message.split(base).join('').split(prefix + '-').join('')}
    : error instanceof Error ? {error: error.name} : {thrown: String(error)}
  const record = async response => ({status: response.status, url: relative(response.url), body: await response.text()})
  const status = response => response ? response.status : null
  async function dump() {
    const out = []
    for (const name of await caches.keys()) {
      if (!name.startsWith(prefix + '-')) continue
      const cache = await caches.open(name)
      const entries = []
      for (const request of await cache.keys()) entries.push([relative(request.url), await record(await cache.match(request))])
      out.push([cacheLabel(name), entries])
    }
    return out
  }

  // Fixture plugins (CONTRACT.md "Fixture plugins").
  // A response as a followed redirect leaves it: it and every clone of it report redirected.
  const redirected = response => {
    const clone = response.clone.bind(response)
    return Object.defineProperties(response, {redirected: {value: true}, clone: {value: () => redirected(clone())}})
  }
  const networkPlugin = () => ({
    requestWillFetch: async ({request}) => {
      if (network.source === 'offline') throw new TypeError('offline')
      const target = new URL(FIXTURE + (network.source === 'beta' ? 'beta' : 'alpha') + '.mjs', location.href).href
      return new Request(target, {integrity: network.source === 'broken' ? WRONG_INTEGRITY : request.integrity})
    },
    fetchDidSucceed: async ({response}) => {
      if (network.source === 'missing') return new Response('missing', {status: 404})
      return network.source === 'redirected' ? redirected(response) : response
    },
  })
  // Records go only into a log the plugin's own handlerWillStart created: a plugin before it can
  // fail that callback, and the trace must not throw in the callbacks that follow.
  const tracePlugin = label => ({
    handlerWillStart: async ({request, state}) => {
      state.log = []
      handlers.push([label, relative(request.url), state.log])
    },
    cacheKeyWillBeUsed: async ({mode, request, state}) => {
      state.log?.push(['cacheKeyWillBeUsed', mode, relative(request.url)])
      return request
    },
    cachedResponseWillBeUsed: async ({cacheName, request, cachedResponse, state}) => {
      state.log?.push(['cachedResponseWillBeUsed', cacheLabel(cacheName), relative(request.url), status(cachedResponse)])
      return cachedResponse
    },
    requestWillFetch: async ({request, state}) => {
      state.log?.push(['requestWillFetch', relative(request.url)])
      return request
    },
    fetchDidSucceed: async ({request, response, state}) => {
      state.log?.push(['fetchDidSucceed', relative(request.url), response.status])
      return response
    },
    fetchDidFail: async ({originalRequest, request, error, state}) => {
      state.log?.push(['fetchDidFail', relative(originalRequest.url), relative(request.url), nameOf(error)])
    },
    cacheDidUpdate: async ({cacheName, request, oldResponse, newResponse, state}) => {
      state.log?.push(['cacheDidUpdate', cacheLabel(cacheName), relative(request.url), status(oldResponse), newResponse.status])
    },
    handlerWillRespond: async ({response, state}) => {
      state.log?.push(['handlerWillRespond', response.status])
      return response
    },
    handlerDidRespond: async ({response, state}) => {
      state.log?.push(['handlerDidRespond', status(response)])
    },
    handlerDidComplete: async ({response, error, state}) => {
      state.log?.push(['handlerDidComplete', status(response), error ? nameOf(error) : null])
    },
    handlerDidError: async ({error, state}) => {
      state.log?.push(['handlerDidError', nameOf(error)])
      return undefined
    },
  })
  const plugin = spec => {
    switch (spec.kind) {
      case 'trace': return tracePlugin(spec.label)
      case 'keep': return {cacheWillUpdate: async ({response}) => response}
      case 'drop': return {cacheWillUpdate: async () => null}
      // `value` makes the callback throw that value itself, a non-Error.
      case 'fail': return {[spec.on]: async () => { throw spec.value === undefined ? new Error('fail') : spec.value }}
      case 'fallback': return new PrecacheFallbackPlugin({fallbackURL: spec.url, precacheController: controllers.get(spec.controller)})
    }
    throw new TypeError('unknown plugin ' + spec.kind)
  }
  const plugins = (specs = []) => [networkPlugin(), ...specs.map(plugin)]

  // Route handlers and options (CONTRACT.md "Steps").
  const handler = spec => {
    if (spec.strategy) return new STRATEGIES[spec.strategy]({cacheName: cacheName(spec.cacheName), plugins: plugins(spec.plugins)})
    if (spec.precache) return controllers.get(spec.precache).strategy
    if (spec.bound) return controllers.get(spec.controller).createHandlerBoundToURL(spec.bound)
    return async () => new Response(spec.respond, {status: spec.status})
  }
  const routeOptions = options => options && {
    directoryIndex: options.directoryIndex,
    cleanURLs: options.cleanURLs,
    ignoreURLParametersMatching: options.ignoreURLParametersMatching?.map(source => new RegExp(source)),
    urlManipulation: options.urlManipulation && (({url}) => options.urlManipulation.map(suffix => {
      const variant = new URL(url.href)
      variant.pathname += suffix
      return variant
    })),
  }
  const register = (step, route) => {
    if (step.id !== undefined) routes.set(step.id, route)
    routers.get(step.router).registerRoute(route)
    return null
  }

  // One event per event-driven step: a fresh ExtendableEvent stand-in whose lifetime promises the
  // step awaits before it reports, or the host event the caller supplied (service-worker mode).
  async function lifetime(type, action) {
    if (input.event !== undefined) return action(input.event)
    const pending = []
    let count = 0
    try {
      return await action({type, waitUntil(promise) { pending.push(promise) }})
    } finally {
      while (pending.length) {
        const batch = pending.splice(0)
        count += batch.length
        await Promise.allSettled(batch)
      }
      events.push([type, count])
    }
  }

  const steps = {
    network(step) {
      network.source = step.source
      return null
    },
    controller(step) {
      controllers.set(step.id, new PrecacheController({cacheName: cacheName(step.cacheName), plugins: plugins(step.plugins), fallbackToNetwork: step.fallbackToNetwork}))
      return null
    },
    addToCacheList(step) {
      controllers.get(step.controller).addToCacheList(step.entries)
      return null
    },
    addPlugins(step) {
      controllers.get(step.controller).strategy.plugins.push(...step.plugins.map(plugin))
      return null
    },
    async install(step) {
      const {updatedURLs, notUpdatedURLs} = await lifetime('install', event => controllers.get(step.controller).install(event))
      return {updatedURLs: updatedURLs.map(relative), notUpdatedURLs: notUpdatedURLs.map(relative)}
    },
    async activate(step) {
      const {deletedURLs} = await lifetime('activate', event => controllers.get(step.controller).activate(event))
      return {deletedURLs: deletedURLs.map(relative)}
    },
    getCachedURLs(step) {
      return controllers.get(step.controller).getCachedURLs().map(relative)
    },
    getURLsToCacheKeys(step) {
      return [...controllers.get(step.controller).getURLsToCacheKeys()].map(([url, key]) => [relative(url), relative(key)])
    },
    getCacheKeyForURL(step) {
      const key = controllers.get(step.controller).getCacheKeyForURL(step.url)
      return key === undefined ? null : relative(key)
    },
    async matchPrecache(step) {
      const response = await controllers.get(step.controller).matchPrecache(step.request ? new Request(step.url) : step.url)
      return response === undefined ? null : record(response)
    },
    router(step) {
      routers.set(step.id, new Router())
      return null
    },
    precacheRoute(step) {
      return register(step, new PrecacheRoute(controllers.get(step.controller), routeOptions(step.options)))
    },
    route(step) {
      const match = step.crossOrigin ? ({sameOrigin}) => !sameOrigin : ({url}) => relative(url.href).startsWith(step.prefix)
      return register(step, new Route(match, handler(step.handler), step.method))
    },
    defaultHandler(step) {
      routers.get(step.router).setDefaultHandler(handler(step.handler), step.method)
      return null
    },
    catchHandler(step) {
      (step.route === undefined ? routers.get(step.router) : routes.get(step.route)).setCatchHandler(handler(step.handler))
      return null
    },
    unregisterRoute(step) {
      routers.get(step.router).unregisterRoute(routes.get(step.route))
      return null
    },
    async fetch(step) {
      const request = step.url === undefined ? input.request
        : new Request(step.url, {method: step.method, mode: step.mode, integrity: step.integrity})
      const response = await lifetime('fetch', event => routers.get(step.router).handleRequest({request, event}))
      return response === undefined ? null : record(response)
    },
    caches: dump,
  }

  try {
    for (const step of input.steps) {
      try {
        results.push(await steps[step.op](step))
      } catch (error) {
        results.push(failure(error))
      }
    }
    return {results, events, handlers, caches: await dump()}
  } finally {
    if (owned) {
      for (const name of await caches.keys()) if (name.startsWith(prefix + '-')) await caches.delete(name)
    }
  }
}
