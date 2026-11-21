import ky from '../../node_modules/ky/distribution/index.js'

async function requestCase(c) {
  const trace = []
  const controller = new AbortController()
  const abort = () => {
    const reason = new Error('cancelled')
    reason.name = 'AbortError'
    controller.abort(reason)
  }
  if (c.abort === 'before') abort()
  let sent = 0
  const fetch = async request => {
    const headers = request.headers
    const body = await request.text()
    trace.push(['fetch',request.method,request.url,headers.get('accept'),headers.get('content-type'),headers.get('x-matrix'),body])
    if (c.abort === 'fetch') abort()
    request.signal.throwIfAborted()
    const reply = c.replies[Math.min(sent++,c.replies.length-1)]
    if (reply.kind === 'network') throw new TypeError('Failed to fetch')
    if (reply.kind === 'stall') return new Promise((_,reject) => request.signal.addEventListener('abort',()=>reject(request.signal.reason),{once:true}))
    const options = {status:reply.status,headers:reply.headers}
    return new Response(reply.body,options)
  }
  const retry = {
    limit:c.limit, methods:c.methods, statusCodes:c.statusCodes, retryOnTimeout:c.retryOnTimeout,
    backoffLimit:c.backoffLimit,maxRetryAfter:c.maxRetryAfter,
    delay:attempt => {
      const ms = 300 * 2 ** (attempt-1)
      trace.push(['backoff',attempt,ms])
      return ms
    },
  }
  const hooks = {
    init:[() => {trace.push(['init'])}],
    beforeRequest:[({request,retryCount}) => {
      trace.push(['beforeRequest',retryCount])
      request.headers.set('x-matrix','initial')
    }],
    afterResponse:[({response,retryCount}) => {trace.push(['afterResponse',retryCount,response.status])}],
    beforeRetry:[({request,error,retryCount}) => {
      trace.push(['beforeRetry',retryCount,error.name])
      request.headers.set('x-matrix','retry-'+retryCount)
      if (c.abort==='retry') abort()
    }],
    beforeError:[({error,retryCount}) => {trace.push(['beforeError',retryCount,error.name])}],
  }
  try {
    const result = await ky(c.url,{method:c.method,headers:c.headers,json:c.json, retry,hooks,fetch,
      timeout:c.timeout,throwHttpErrors:c.throwHttpErrors,signal:controller.signal})[c.read]()
    return [trace,['ok',result]]
  } catch(error) {return [trace,['error',error.name,error.response?.status??null,error.data??null]]}
}

export async function run(input) {
  const out=[]
  for (const c of input.cases) out.push(await requestCase(c))
  return out
}
