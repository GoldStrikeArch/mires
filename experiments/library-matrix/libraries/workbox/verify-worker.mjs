import assert from 'node:assert/strict'
import {randomUUID} from 'node:crypto'
import {readFile} from 'node:fs/promises'
import {createServer} from 'node:http'
import {fileURLToPath} from 'node:url'
import {rollup} from 'rollup'
import {openBrowser} from '../../tools/lib/browser.mjs'

const [browser='chrome',...requested]=process.argv.slice(2)
const languages=requested.length?requested:['reference','rust','rescript','melange','jsoo','scalajs','dart','kotlin','elm','gopherjs']
const file=path=>fileURLToPath(new URL(path,import.meta.url))
const prefix='/experiments/library-matrix/libraries/workbox/'
const assets=new Map(await Promise.all(['alpha','beta'].map(async id=>[prefix+id+'.mjs',await readFile(file(id+'.mjs'),'utf8')])))
let worker
const server=createServer((request,response)=>{
  const {pathname}=new URL(request.url,'http://localhost')
  const headers={'cache-control':'no-store'}
  if(pathname==='/worker.js') response.writeHead(200,{'content-type':'text/javascript',...headers}).end(worker)
  else if(assets.has(pathname)) response.writeHead(200,{'content-type':'text/javascript',...headers}).end(assets.get(pathname))
  else if(pathname==='/page') response.writeHead(200,{'content-type':'text/html',...headers}).end('<!doctype html><title>Workbox integration</title>')
  else response.writeHead(404).end('no route')
})
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve))
const origin='http://127.0.0.1:'+server.address().port

// Runs in the controlled client, using its native service worker container, fetch and CacheStorage.
async function exercise(cacheName,done) {
  const base='/experiments/library-matrix/libraries/workbox/'
  try {
    const controlled = navigator.serviceWorker.controller ? Promise.resolve() : new Promise(resolve=>{
      navigator.serviceWorker.addEventListener('controllerchange',resolve,{once:true})
    })
    await navigator.serviceWorker.register('/worker.js?cache='+cacheName)
    await navigator.serviceWorker.ready
    await controlled
    const message=data=>new Promise((resolve,reject)=>{
      const channel=new MessageChannel()
      channel.port1.onmessage=({data})=>{
        channel.port1.close()
        if(data.error) reject(Error(data.error)); else resolve(data.value)
      }
      navigator.serviceWorker.controller.postMessage(data,[channel.port2])
    })
    const fetchPair=async path=>{
      const response=await fetch(base+path)
      return [response.status,await response.text()]
    }
    const result=[]
    result.push(await message({report:true}))
    result.push(await fetchPair('guide?utm_source=client'))
    await message({source:'beta'})
    result.push(await fetchPair('first/item'))
    const cache=await caches.open(cacheName+'-runtime')
    for(let i=0;i<100 && !(await cache.match(base+'first/item'));i++) await new Promise(resolve=>setTimeout(resolve,10))
    await message({source:'offline'})
    result.push(await fetchPair('first/item'),await fetchPair('network/item'),await fetchPair('unmatched'))
    result.push(await message({caches:true}))
    done({result})
  } catch(error){done({error:String(error)})}
}

let session
try {
  session=await openBrowser(browser,{scriptTimeoutMs:30000})
  if(session.error) throw Error(session.error)
  for(const language of languages){
    const target=file(language==='reference'?'reference.mjs':'../../ports/workbox/'+language+'/dist/index.min.js')
    const bundle=await rollup({input:file('worker-fixture.mjs'),plugins:[{name:'matrix-target',resolveId(id){if(id==='matrix-workbox-target') return target}}],onwarn(warn,report){if(warn.code!=='EVAL') report(warn)}})
    try {worker=(await bundle.generate({format:'iife'})).output[0].code} finally {await bundle.close()}
    await session.driver.get(origin+'/page')
    const name='mires-worker-integration-'+randomUUID()
    try {
      const response=await session.driver.executeAsyncScript(exercise,name)
      if(response.error) throw Error(language+' '+response.error)
      const alpha=assets.get(prefix+'alpha.mjs'), beta=assets.get(prefix+'beta.mjs')
      const path=prefix.slice(1)
      assert.deepEqual(response.result,[
        [{updatedURLs:[path+'guide.html'],notUpdatedURLs:[]},{deletedURLs:[]}],
        [200,alpha],[200,beta],[200,beta],[503,'no-response'],[404,'no route'],
        [['precache',[[path+'guide.html?__WB_REVISION__=release-1',{status:200,url:path+'alpha.mjs',body:alpha}]]],
          ['runtime',[[path+'first/item',{status:200,url:path+'beta.mjs',body:beta}]]]],
      ],language)
    } finally {
      await session.driver.executeAsyncScript(function(name,done){
        Promise.all([navigator.serviceWorker.getRegistrations().then(rows=>Promise.all(rows.map(r=>r.unregister()))),
          caches.delete(name),caches.delete(name+'-runtime')]).then(()=>done(true),error=>done(String(error)))
      },name)
      await session.driver.get('about:blank')
    }
    console.log(browser+' '+language+' passed native install, activate and fetch events, offline fetches and the cache state')
  }
} finally {await session?.driver?.quit();await new Promise(resolve=>server.close(resolve))}
