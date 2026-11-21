// JS host of the Elm port. It executes the primitives of libraries/workbox/CONTRACT.md ("Elm: the
// I/O boundary") and reports each result as a message; all of Workbox and of the reference
// adapter is Elm (src/WorkboxCore.elm, src/Strategies.elm, src/Precaching.elm, src/Routing.elm,
// src/Scenario.elm).
//
// The program sends `{ id, op, ...arguments }` through `command`. The host answers before it
// returns: `{ kind: 'value', id, value }` with what a primitive gave, or `{ kind: 'threw', id,
// error }`. A primitive that answers with a promise is answered `{ kind: 'started', id }`, and its
// outcome follows in the continuation of that promise, never through a timer: `{ kind:
// 'resolved', id, value }` or `{ kind: 'rejected', id, error }`. A thrown or rejected value
// travels as `{ name, message, isError, text }`, `isError` being `instanceof Error`.
//
// Host objects stay here, in a handle table that lives for one call; a Request, Response or URL
// travels as its handle with the properties Workbox reads. `exit` ends the call with the output.
// Each call has its own Elm program instance; the module keeps no state.
import Elm from 'elm-program'

const describeError = (error) => error instanceof Error
  ? { name: String(error.name), message: String(error.message), isError: true, text: String(error) }
  : { name: '', message: '', isError: false, text: String(error) }

export function run(input) {
  return new Promise((resolve, reject) => {
    const app = Elm.Main.init()
    const send = (message) => app.ports.receive.send(message)
    const handles = []
    const put = (value) => handles.push(value) - 1
    const get = (handle) => handles[handle]
    const lifetimes = []

    const describeRequest = (request) => ({
      handle: put(request), url: request.url, method: request.method, mode: request.mode, integrity: request.integrity,
    })
    const describeResponse = (response, handle = put(response)) => ({
      handle, status: response.status, statusText: response.statusText, url: response.url, redirected: response.redirected, type: response.type,
    })
    const describeUrl = (handle) => {
      const url = get(handle)
      return { handle, href: url.href, origin: url.origin, protocol: url.protocol, pathname: url.pathname }
    }
    const requestInit = ({ headers, ...members }) => (headers === undefined ? members : { ...members, headers: get(headers) })
    // The network fixture's `redirected` source: a response as a followed redirect leaves it.
    const markRedirected = (response) => {
      const clone = response.clone.bind(response)
      return Object.defineProperties(response, { redirected: { value: true }, clone: { value: () => markRedirected(clone()) } })
    }

    const answers = {
      locationHref: () => location.href,
      locationOrigin: () => location.origin,
      randomUUID: () => crypto.randomUUID(),
      warn: ({ message }) => console.warn(message),
      newUrl: ({ input, base }) => describeUrl(put(base === null ? new URL(input) : new URL(input, base))),
      setPathname: ({ url, value }) => { get(url).pathname = value; return describeUrl(url) },
      setHash: ({ url, value }) => { get(url).hash = value; return describeUrl(url) },
      searchParamKeys: ({ url }) => [...get(url).searchParams.keys()],
      searchParamSet: ({ url, name, value }) => { get(url).searchParams.set(name, value); return describeUrl(url) },
      searchParamDelete: ({ url, name }) => { get(url).searchParams.delete(name); return describeUrl(url) },
      newRegExp: ({ source }) => put(new RegExp(source)),
      regExpTest: ({ regExp, input }) => get(regExp).test(input),
      newRequest: ({ input, init }) => describeRequest(new Request(typeof input === 'number' ? get(input) : input, requestInit(init))),
      cloneRequest: ({ request }) => describeRequest(get(request).clone()),
      requestHeaders: ({ request }) => put(get(request).headers),
      newResponse: ({ body, status }) => describeResponse(new Response(body, { status })),
      responseFromParts: ({ blob, headers, status, statusText }) =>
        describeResponse(new Response(get(blob), { headers: new Headers(get(headers)), status, statusText })),
      cloneResponse: ({ response }) => describeResponse(get(response).clone()),
      responseHeaders: ({ response }) => put(get(response).headers),
      markRedirected: ({ response }) => describeResponse(markRedirected(get(response)), response),
      eventType: () => input.event.type,
      eventWaitUntil: () => {
        let lifetime
        input.event.waitUntil(new Promise((fulfil, fail) => { lifetime = { fulfil, fail } }))
        return lifetimes.push(lifetime) - 1
      },
      settleLifetime: ({ token, failure }) => {
        if (failure === null) lifetimes[token].fulfil()
        else lifetimes[token].fail(Object.assign(new Error(failure.message), { name: failure.name }))
      },
    }

    const promises = {
      cachesOpen: ({ name }) => caches.open(name).then(put),
      cachesMatch: ({ request, cacheName }) => caches.match(get(request), { cacheName }).then((response) => response && describeResponse(response)),
      cachesKeys: () => caches.keys(),
      cachesDelete: ({ name }) => caches.delete(name),
      cacheMatch: ({ cache, request }) => get(cache).match(typeof request === 'number' ? get(request) : request).then((response) => response && describeResponse(response)),
      cachePut: ({ cache, request, response }) => get(cache).put(get(request), get(response)),
      cacheDelete: ({ cache, request }) => get(cache).delete(get(request)),
      cacheKeys: ({ cache, request }) =>
        (request === null ? get(cache).keys() : get(cache).keys(get(request), { ignoreSearch: true })).then((requests) => requests.map(describeRequest)),
      fetch: ({ request }) => fetch(get(request)).then(describeResponse),
      responseText: ({ response }) => get(response).text(),
      responseBlob: ({ response }) => get(response).blob().then(put),
    }

    app.ports.command.subscribe(({ id, op, ...args }) => {
      if (op === 'exit') {
        if ('failed' in args) reject(new Error(args.failed))
        else resolve(args.output)
        send({ kind: 'value', id, value: null })
      } else if (Object.hasOwn(answers, op)) {
        let value
        try {
          value = answers[op](args)
        } catch (error) {
          send({ kind: 'threw', id, error: describeError(error) })
          return
        }
        send({ kind: 'value', id, value: value ?? null })
      } else if (Object.hasOwn(promises, op)) {
        let promise
        try {
          promise = promises[op](args)
        } catch (error) {
          promise = Promise.reject(error)
        }
        send({ kind: 'started', id })
        promise.then(
          (value) => send({ kind: 'resolved', id, value: value ?? null }),
          (error) => send({ kind: 'rejected', id, error: describeError(error) }),
        )
      } else {
        reject(new Error('no primitive ' + op))
      }
    })
    send({ kind: 'start', input, native: input.event !== undefined, request: input.request === undefined ? null : describeRequest(input.request) })
  })
}
