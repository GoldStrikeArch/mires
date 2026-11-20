import Elm from 'elm-program'

const baseUrl = () => new URL('experiments/library-matrix/libraries/workbox/',location.href)
const absolute = (path,base) => new URL(path,base).href
const relative = (path,base) => path.slice(base.href.length)

function manifest(items,base) {
  const keys=new Map()
  for(const item of items) {
    const url=new URL(item.path,base)
    const original=url.href
    if(item.revision)url.searchParams.set('__WB_REVISION__',item.revision)
    if(keys.has(original) && keys.get(original)!==url.href) throw Error('conflicting revisions')
    keys.set(original,url.href)
  }
  return keys
}
function candidates(path,base) {
  const url=new URL(path,base)
  url.hash=''
  const result=[url.href]
  for(const key of [...url.searchParams.keys()]) {
    if(key==='fbclid'||key.startsWith('utm_')) url.searchParams.delete(key)
  }
  result.push(url.href)
  const pathname=url.pathname
  if(pathname.endsWith('/')){url.pathname=pathname+'index.html';result.push(url.href)}
  url.pathname=pathname+'.html'
  result.push(url.href)
  return result
}

async function phase(step,name,items,base,route) {
  const keys=manifest(items,base)
  const cache=await caches.open(name)
  const source=step.source??'alpha'
  const network=async()=>{
    if(source==='offline')throw Error('offline')
    return fetch(new Request(absolute(source+'.mjs',base)))
  }
  if(step.phase==='install') {
    const updated=[], unchanged=[]
    for(const [original,key] of keys) {
      if(await cache.match(key)) unchanged.push(relative(original,base))
      else{
        const res=await network()
        if(res.status >= 400)throw Error('bad-precaching-response')
        await cache.put(key,res.clone())
        updated.push(relative(original,base))
      }
    }
    return [updated,unchanged]
  }
  if(step.phase==='activate'){
    const retained=new Set(keys.values())
    const removed=[]
    for(const req of await cache.keys()) {
      if(!retained.has(req.url)){await cache.delete(req.url);removed.push(relative(req.url,base))}
    }
    return removed.sort()
  }
  if(step.phase==='message'){
    if(step.message==='SKIP_WAITING')await self.skipWaiting()
    return step.message==='SKIP_WAITING'
  }
  const request=step.request || new Request(absolute(step.path,base),{method:step.method || 'GET'})
  const {method,url:path}=request
  const choice=route({method,path:relative(path,base),entries:[...keys],candidates:candidates(path,base)})
  if(!choice)return null
  try{
    let res
    if(choice.kind==='precache')res=await cache.match(choice.key)??await network()
    else if(choice.kind==='network')res=await network()
    else {
      const runtime=await caches.open(name+'-runtime')
      res=await runtime.match(path)
      if(!res) {
        if(choice.kind==='only')throw Error('no-response')
        res=await network()
        if(res.status===200)await runtime.put(path,res.clone())
      }
    }
    return [res.status,await res.text()]
  }catch{return ['no-response']}
}

export async function run(input) {
  const app=Elm.Main.init()
  let reply
  const listener=v=>{reply=v}
  app.ports.response.subscribe(listener)
  const route=v=>{reply=undefined;app.ports.request.send(v);return reply}
  const base=baseUrl()
  try{
    if(input.phase){
      const task=phase(input,input.name,input.manifest??[],base,route)
      input.event?.waitUntil(task)
      return await task
    }
    const name='mires-workbox-'+crypto.randomUUID()
    let items=[]
    const results=[]
    try{
      for(const step of input.steps){items=step.manifest??items;results.push(await phase(step,name,items,base,route))}
      return results
    }finally{await Promise.all([caches.delete(name),caches.delete(name+'-runtime')])}
  } finally {app.ports.response.unsubscribe(listener)}
}
