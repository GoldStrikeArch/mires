import wb from '../../node_modules/workbox-precaching/matrix-entry.mjs'
import './alpha.mjs'
import './beta.mjs'

const base = () => new URL('experiments/library-matrix/libraries/workbox/', location.href)
const absolute = path => new URL(path, base()).href
const relative = url => url.slice(base().href.length)
const lifetime = type => ({ type, waitUntil(promise) { promise.catch(() => {}) } })

function strategyPlugin(source) {
  return { requestWillFetch: async () => {
    if (source === 'offline') throw new TypeError('offline')
    return new Request(absolute(source + '.mjs'))
  } }
}

function controller(name, manifest, source) {
  const pc = new wb.precaching.PrecacheController({ cacheName: name, plugins: [strategyPlugin(source)] })
  pc.addToCacheList(manifest.map(({ path, revision }) => ({url:absolute(path), revision})))
  return pc
}

async function phase(input) {
  const { name, manifest, source = 'alpha', event } = input
  const pc = controller(name, manifest, source)
  if (input.phase === 'install') {
    const { updatedURLs, notUpdatedURLs } = await pc.install(event || lifetime('install'))
    return [updatedURLs.map(relative), notUpdatedURLs.map(relative)]
  }
  if (input.phase === 'activate') {
    const { deletedURLs } = await pc.activate(event || lifetime('activate'))
    return deletedURLs.map(relative).sort()
  }
  if (input.phase === 'message') {
    if (input.message === 'SKIP_WAITING') await self.skipWaiting()
    return input.message === 'SKIP_WAITING'
  }
  const router = new wb.routing.Router()
  const plugins = [strategyPlugin(source)]
  router.registerRoute(new wb.precaching.PrecacheRoute(pc))
  const types = [['first', wb.strategies.CacheFirst], ['only', wb.strategies.CacheOnly], ['network', wb.strategies.NetworkOnly]]
  for (const [path, Strategy] of types) {
    router.registerRoute(new wb.routing.Route(({url}) => relative(url.href).startsWith(path+'/'),
      new Strategy({cacheName:name+'-runtime', plugins})))
  }
  const request = input.request || new Request(absolute(input.path), {method:input.method || 'GET'})
  const pending = []
  const response = router.handleRequest({request,event:event || {type:'fetch',waitUntil(p){pending.push(p)}}})
  if (!response) return null
  try {
    const res = await response
    await Promise.allSettled(pending)
    return [res.status, await res.text()]
  } catch {
    await Promise.allSettled(pending)
    return ['no-response']
  }
}

export async function run(input) {
  if (input.phase) return phase(input)
  const name = 'mires-workbox-' + crypto.randomUUID()
  let manifest = []
  const result = []
  try {
    for (const step of input.steps) {
      if (step.manifest) manifest = step.manifest
      result.push(await phase({...step,name,manifest}))
    }
    return result
  } finally {
    await Promise.all([caches.delete(name),caches.delete(name+'-runtime')])
  }
}
