// The independent verifier links "matrix-workbox-target" to the selected production artifact.
import {run} from 'matrix-workbox-target'

const name = new URL(self.location.href).searchParams.get('cache')
const manifest = [{path:'guide.html', revision:'release-1'}]
const base = '/experiments/library-matrix/libraries/workbox/'
let source = 'alpha'
let installed, activated
self.addEventListener('install', event => {
  event.waitUntil(run({phase:'install',name,manifest,source,event}).then(value=>{installed=value}))
})
self.addEventListener('activate', event => {
  event.waitUntil(run({phase:'activate',name,manifest,event}).then(async value=>{
    activated=value
    await self.clients.claim()
  }))
})
self.addEventListener('message', event => {
  const reply = event.ports[0]
  const message = event.data
  event.waitUntil((async()=>{
    if (message.source) {source=message.source; return source}
    if (message.report) return [installed,activated]
    return run({phase:'message',name,manifest,message:message.type,event})
  })().then(value=>reply.postMessage({value}),error=>reply.postMessage({error:String(error)})))
})
self.addEventListener('fetch', event => {
  const path=new URL(event.request.url).pathname
  if(!path.startsWith(base) || /\/(alpha|beta)\.mjs$/.test(path)) return
  event.respondWith(run({phase:'fetch',name,manifest,source,request:event.request,event}).then(pair=>{
    if (pair===null) return fetch(event.request)
    if (pair[0]==='no-response') return new Response('offline',{status:503})
    return new Response(pair[1],{status:pair[0]})
  }))
})
