import Elm from 'elm-program'

const abortReason = () => Object.assign(new Error('cancelled'),{name:'AbortError'})
const named = name => Object.assign(new Error(name),{name})
const pause = ms => new Promise(resolve=>setTimeout(resolve,ms))

async function requestCase(c,plan) {
  const trace=[['init']]
  const user=new AbortController()
  if(c.abort==='before') user.abort(abortReason())
  let base
  try {
    const headers=new Headers(c.headers)
    const method=c.method??'GET'
    const common=['get','post','put','patch','head','delete','query'].includes(method.toLowerCase())
    const init={method:common?method.toUpperCase():method,headers}
    if(c.json!==undefined) {
      init.body=JSON.stringify(c.json)
      if(!headers.has('content-type')) headers.set('content-type','application/json')
    }
    if(!headers.get('accept')) headers.set('accept',c.read==='json'?'application/json':'text/*')
    base=new Request(c.url,init)
  } catch(error) {
    return [trace,['error',error.name,null,null]]
  }
  let count=0
  const failure=(name,status=null,data=null,hook=true)=>{
    if(hook) trace.push(['beforeError',count,name])
    return [trace,['error',name,status,data]]
  }
  if(user.signal.aborted)return failure(user.signal.reason.name)
  trace.push(['beforeRequest',0])
  base.headers.set('x-matrix','initial')
  async function fetch(request) {
    const h=request.headers
    trace.push(['fetch',request.method,request.url,h.get('accept'),h.get('content-type'),h.get('x-matrix'),await request.text()])
    if(c.abort==='fetch')user.abort(abortReason())
    request.signal.throwIfAborted()
    const spec=c.replies[Math.min(count,c.replies.length-1)]
    if(spec.kind==='network')throw new TypeError('Failed to fetch')
    if(spec.kind==='stall')return new Promise((_,reject)=>request.signal.addEventListener('abort',()=>reject(request.signal.reason),{once:true}))
    return new Response(spec.body,{status:spec.status,headers:spec.headers})
  }
  for(;;){
    const controller=new AbortController()
    const signal=AbortSignal.any([user.signal,controller.signal])
    const request=new Request(base.clone(),{signal})
    let timer, name, status=null, data=null, after=null, response
    try {
      const sent=fetch(request)
      response=c.timeout===false?await sent:await Promise.race([sent,new Promise((_,reject)=>{
        timer=setTimeout(()=>{reject(named('TimeoutError'));controller.abort()},c.timeout)
      })])
    } catch(e) {
      name=e.name==='TypeError' && e.message==='Failed to fetch'?'NetworkError':e.name
    } finally {clearTimeout(timer)}
    if(response) {
      trace.push(['afterResponse',count,response.status])
      const body=await response.text()
      if(response.ok || !c.throwHttpErrors) {
        try {return [trace,['ok',c.read==='json'?JSON.parse(body):body]]}
        catch(e){return failure(e.name,null,null,false)}
      }
      name='HTTPError'
      status=response.status
      after=response.headers.get('Retry-After')
      if(body!==''){
        const mime=response.headers.get('content-type')?.split(';')[0].trim().toLowerCase()??''
        if(mime.includes('/') && ['/json','+json','-json','.json'].some(s=>mime.endsWith(s))){
          try {data=JSON.parse(body)} catch {data=null}
        }else data=body
      }
    }
    const delay=plan({config:c,count,method:base.method,name,status,after})
    if(delay===null)return failure(name,status,data)
    if(delay.backoff!==null)trace.push(['backoff',count+1,delay.backoff])
    await pause(delay.ms)
    count++
    trace.push(['beforeRetry',count,name])
    base.headers.set('x-matrix','retry-'+count)
    if(c.abort==='retry')user.abort(abortReason())
  }
}

export async function run(input) {
  const app=Elm.Main.init()
  let reply
  const listener=value=>{reply=value}
  app.ports.decision.subscribe(listener)
  const plan=message=>{reply=undefined;app.ports.retry.send(message);return reply}
  try {
    const results=[]
    for(const c of input.cases) results.push(await requestCase(c,plan))
    return results
  } finally {app.ports.decision.unsubscribe(listener)}
}
