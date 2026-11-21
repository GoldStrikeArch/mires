// Entry module of the ReScript port: what libraries/ky/reference.mjs does, on the port's Ky. Each
// request case gets its own abort controller, transport and hooks, which record the trace.
//
// The boundary: the input records are the corpus JS objects themselves, read in place. The option
// values Ky validates stay JSON values (`JSON.t`) until `Ky.normalizeRetryOptions` and its siblings
// check them, as Ky checks its own options. Each result is `[trace, outcome]`, built as `JSON.t`.

open Host

type reply = {kind: string, status: JSON.t, body: Null.t<string>, headers: JSON.t}

type case = {
  url: string,
  method?: string,
  headers?: dict<JSON.t>,
  json?: JSON.t,
  read: string,
  replies: array<reply>,
  limit?: JSON.t,
  backoffLimit?: JSON.t,
  maxRetryAfter?: JSON.t,
  timeout?: JSON.t,
  retryOnTimeout?: JSON.t,
  throwHttpErrors?: JSON.t,
  abort: string,
  methods?: JSON.t,
  statusCodes?: JSON.t,
}

type input = {cases: array<case>}

let number = value => JSON.Number(Int.toFloat(value))
let nullable = value => value->Option.mapOr(JSON.Null, text => JSON.String(text))

let requestCase = async (c: case) => {
  let trace = []
  let record = entry => trace->Array.push(JSON.Array(entry))
  let controller = AbortController.make()
  let abort = () => {
    let reason = JsError.make("cancelled")
    reason->setErrorName("AbortError")
    controller->AbortController.abortWith(reason)
  }
  if c.abort == "before" {
    abort()
  }
  let sent = ref(0)
  // The injected transport: it records what it was sent and answers with the next reply.
  let fetch = async (request: Request.t) => {
    let headers = request->Request.headers
    let body = await request->Request.text
    record([
      String("fetch"),
      String(request->Request.method),
      String(request->Request.url),
      nullable(headers->Headers.get("accept")),
      nullable(headers->Headers.get("content-type")),
      nullable(headers->Headers.get("x-matrix")),
      String(body),
    ])
    if c.abort == "fetch" {
      abort()
    }
    let signal = request->Request.signal
    signal->AbortSignal.throwIfAborted
    let reply = c.replies->Array.getUnsafe(Math.Int.min(sent.contents, c.replies->Array.length - 1))
    sent := sent.contents + 1
    switch reply.kind {
    | "network" => JsError.TypeError.throwWithMessage("Failed to fetch")
    | "stall" =>
      await Promise.make((_, reject) => signal->AbortSignal.onAbort(() => reject(signal->AbortSignal.reason)))
    | _ => Response.make(reply.body, {status: reply.status, headers: reply.headers})
    }
  }
  let delay = attempt => {
    let milliseconds = 300. *. Math.pow(2., ~exp=Int.toFloat(attempt - 1))
    record([String("backoff"), number(attempt), Number(milliseconds)])
    milliseconds
  }
  let hooks: Ky.hooks = {
    init: [() => record([String("init")])],
    beforeRequest: [
      ({request, retryCount}) => {
        record([String("beforeRequest"), number(retryCount)])
        request->Request.headers->Headers.set("x-matrix", String("initial"))
      },
    ],
    afterResponse: [
      ({response, retryCount}) =>
        record([String("afterResponse"), number(retryCount), number(response->Response.status)]),
    ],
    beforeRetry: [
      ({request, error, retryCount}) => {
        record([String("beforeRetry"), number(retryCount), String(Ky.errorName(error))])
        request->Request.headers->Headers.set("x-matrix", String(`retry-${Int.toString(retryCount)}`))
        if c.abort == "retry" {
          abort()
        }
      },
    ],
    beforeError: [
      ({error, retryCount}) =>
        record([String("beforeError"), number(retryCount), String(Ky.errorName(error))]),
    ],
  }
  let options: Ky.options = {
    method: ?c.method,
    headers: ?c.headers,
    json: ?c.json,
    retry: {
      limit: ?c.limit,
      methods: ?c.methods,
      statusCodes: ?c.statusCodes,
      retryOnTimeout: ?c.retryOnTimeout,
      backoffLimit: ?c.backoffLimit,
      maxRetryAfter: ?c.maxRetryAfter,
      delay,
    },
    hooks,
    fetch,
    timeout: ?c.timeout,
    throwHttpErrors: ?c.throwHttpErrors,
    signal: controller->AbortController.signal,
  }
  let outcome = switch await Ky.request(c.url, options, c.read == "json" ? Json : Text) {
  | result => JSON.Array([String("ok"), result])
  | exception Ky.HTTPError({response, data}) =>
    JSON.Array([
      String("error"),
      String("HTTPError"),
      number(response->Response.status),
      data->Option.getOr(JSON.Null),
    ])
  | exception error => JSON.Array([String("error"), String(Ky.errorName(error)), Null, Null])
  }
  JSON.Array([JSON.Array(trace), outcome])
}

let run = async (input: input) => {
  let results = []
  for index in 0 to input.cases->Array.length - 1 {
    results->Array.push(await requestCase(input.cases->Array.getUnsafe(index)))
  }
  results
}
