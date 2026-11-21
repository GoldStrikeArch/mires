// The service worker verify-worker.mjs registers. Rollup links "matrix-workbox-target" to the
// selected production artifact. Every native install, activate and fetch event runs one script
// (CONTRACT.md "Service-worker mode"): the event and, for fetch, its Request are host objects, and
// the caches persist under the registration's prefix across events.
import {run} from 'matrix-workbox-target'

const cachePrefix = new URL(self.location.href).searchParams.get('cache')
const base = 'experiments/library-matrix/libraries/workbox/'
const version = [
  {op: 'controller', id: 'sw', cacheName: 'precache'},
  {op: 'addToCacheList', controller: 'sw', entries: [{url: base + 'guide.html', revision: 'release-1'}]},
]
const routes = [
  {op: 'router', id: 'r'},
  {op: 'precacheRoute', router: 'r', controller: 'sw'},
  {op: 'route', router: 'r', prefix: base + 'first/', handler: {strategy: 'CacheFirst', cacheName: 'runtime'}},
  {op: 'route', router: 'r', prefix: base + 'network/', handler: {strategy: 'NetworkOnly', cacheName: 'runtime'}},
]
let source = 'alpha'
let installed, activated
const last = output => output.results[output.results.length - 1]

self.addEventListener('install', event => {
  event.waitUntil(run({cachePrefix, event, steps: [{op: 'network', source}, ...version, {op: 'install', controller: 'sw'}]}).then(output => { installed = last(output) }))
})
self.addEventListener('activate', event => {
  event.waitUntil(run({cachePrefix, event, steps: [...version, {op: 'activate', controller: 'sw'}]}).then(async output => {
    activated = last(output)
    await self.clients.claim()
  }))
})
self.addEventListener('message', event => {
  const reply = event.ports[0]
  const message = event.data
  event.waitUntil((async () => {
    if (message.source) return (source = message.source)
    if (message.report) return [installed, activated]
    return (await run({cachePrefix, steps: [{op: 'caches'}]})).caches
  })().then(value => reply.postMessage({value}), error => reply.postMessage({error: String(error)})))
})
self.addEventListener('fetch', event => {
  const path = new URL(event.request.url).pathname
  if (!path.startsWith('/' + base) || /\/(alpha|beta)\.mjs$/.test(path)) return
  event.respondWith(run({cachePrefix, event, request: event.request, steps: [{op: 'network', source}, ...version, ...routes, {op: 'fetch', router: 'r'}]}).then(output => {
    const response = last(output)
    if (response === null) return fetch(event.request)
    if ('error' in response) return new Response(response.error, {status: 503})
    return new Response(response.body, {status: response.status})
  }))
})
