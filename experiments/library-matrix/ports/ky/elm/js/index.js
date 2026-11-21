// JS host of the Elm port: it performs the Fetch, abort, decoding and timer operations Ky's Elm
// code asks for, and answers with what each gave or threw. It holds no Ky logic: the request
// construction, option checks, timeouts and aborts, error wrapping, error data and the retry loop
// are all Elm (src/Ky.elm), and so is the reference adapter's transport (src/Main.elm).
//
// The program sends `{ id, op, ...arguments }` through `command`; the host performs `op` and
// answers through `reply` with `{ id, ok }` or `{ id, error }`, always on a later microtask. Host
// objects stay here, behind the handles the answers carry, in a table that lives for one call. A
// `timer` answers when it fires, `onAbort` when its signal aborts, and `{ clear }` cancels a
// timer. The program ends a call with `{ done }` or, on a host answer it cannot read, `{ failed }`.
//
// Module state, as in the other Elm hosts: `worker` is the one Platform.worker instance, whose
// model is reset by every `start`, and `current` is the call in progress; an answer from an
// earlier call is dropped.
import Elm from 'elm-program'

let worker
let current

/** A thrown value, as Ky reads one. */
const describe = (error) => ({
  name: typeof error?.name === 'string' ? error.name : 'Error',
  message: typeof error?.message === 'string' ? error.message : null,
  // Ky's isError: an Error of this realm, or one of another realm by its internal brand.
  isError: error instanceof Error || Object.prototype.toString.call(error) === '[object Error]',
  hasStack: error?.stack !== undefined,
  sentryCaptured: error !== null && typeof error === 'object' && '__sentry_captured__' in error,
})

const describeRequest = (request, store) => ({
  handle: store.put(request),
  method: request.method,
  url: request.url,
  headers: store.put(request.headers),
  signal: store.put(request.signal),
})

const operations = {
  abortController(_, store) {
    const controller = new AbortController()
    return { handle: store.put(controller), signal: store.put(controller.signal) }
  },
  abort({ controller, reason }, store) {
    store.get(controller).abort(reason === null ? undefined : Object.assign(new Error(reason.message), { name: reason.name }))
    return null
  },
  anySignal({ signals }, store) {
    return store.put(AbortSignal.any(signals.map(store.get)))
  },
  aborted({ signal }, store) {
    const target = store.get(signal)
    return target.aborted ? describe(target.reason) : null
  },
  onAbort({ signal }, store) {
    const target = store.get(signal)
    return new Promise((resolve) => target.addEventListener('abort', () => resolve(describe(target.reason)), { once: true }))
  },
  newHeaders(_, store) {
    return store.put(new Headers())
  },
  setHeader({ headers, name, value }, store) {
    store.get(headers).set(name, value)
    return null
  },
  getHeader({ headers, name }, store) {
    return store.get(headers).get(name)
  },
  newRequest({ input, method, headers, body, signal }, store) {
    const init = { method, headers: store.get(headers), signal: store.get(signal) }
    if (body !== null) init.body = body
    return describeRequest(new Request(input, init), store)
  },
  withSignal({ request, signal }, store) {
    return describeRequest(new Request(store.get(request), { signal: store.get(signal) }), store)
  },
  cloneRequest({ request }, store) {
    return describeRequest(store.get(request).clone(), store)
  },
  requestText({ request }, store) {
    return store.get(request).text()
  },
  newResponse({ body, status, headers }, store) {
    const response = new Response(body, { status, headers })
    return { handle: store.put(response), status: response.status, ok: response.ok, headers: store.put(response.headers), hasBody: response.body !== null }
  },
  responseText({ response }, store) {
    return store.get(response).text()
  },
  async arrayBuffer({ response }, store) {
    const buffer = await store.get(response).arrayBuffer()
    return { handle: store.put(buffer), length: buffer.byteLength }
  },
  textDecoder({ label }, store) {
    return store.put(label === null ? new TextDecoder() : new TextDecoder(label))
  },
  decode({ decoder, bytes }, store) {
    return store.get(decoder).decode(store.get(bytes))
  },
  now() {
    return Date.now()
  },
  timer({ milliseconds }, store, id) {
    return new Promise((resolve) => store.timers.set(id, setTimeout(() => resolve(null), milliseconds)))
  },
}

export function run(input) {
  if (!worker) {
    worker = Elm.Main.init()
    worker.ports.command.subscribe((message) => current?.receive(message))
  }
  return new Promise((resolve, reject) => {
    const handles = []
    const store = { put: (value) => handles.push(value) - 1, get: (handle) => handles[handle], timers: new Map() }
    const call = {
      receive(message) {
        if ('done' in message) {
          current = undefined
          resolve(message.done)
        } else if ('failed' in message) {
          current = undefined
          reject(new Error(message.failed))
        } else if ('clear' in message) {
          clearTimeout(store.timers.get(message.clear))
        } else {
          const { id, op, ...argumentsOf } = message
          const answer = (outcome) => {
            if (current === call) worker.ports.reply.send({ id, ...outcome })
          }
          Promise.resolve()
            .then(() => operations[op](argumentsOf, store, id))
            .then((ok) => answer({ ok }), (error) => answer({ error: describe(error) }))
        }
      },
    }
    current = call
    worker.ports.start.send(input)
  })
}
