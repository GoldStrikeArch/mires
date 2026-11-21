// workbox-core 7.4.1: WorkboxError and its production messages, copyResponse,
// cacheMatchIgnoreParams, timeout, waitUntil and Deferred.
open Host

// A WorkboxError's details object, in insertion order. Its values are strings, numbers, null,
// the caller's own manifest entry, or a caught error.
type details = dict<unknown>
external toUnknown: 'a => unknown = "%identity"

module WorkboxError = {
  @set external setName: (JsError.t, string) => unit = "name"
  @set external setDetails: (JsError.t, option<details>) => unit = "details"

  // The errors this module made: what `instanceof WorkboxError` tests in JavaScript.
  let instances: WeakSet.t<JsExn.t> = WeakSet.make()

  // `new WorkboxError(code, details)` of the production build: an Error whose own `name` is the
  // code and own `details` the details. The production messageGenerator prints the argument list
  // as JSON, so absent details print as `[null]`, and a nested error prints its own enumerable
  // properties.
  let make = (code, ~details=?) => {
    let error = JsError.make(code ++ " :: " ++ JSON.stringifyAny([details])->Option.getOr(""))
    error->setName(code)
    error->setDetails(details)
    let error = error->JsError.toJsExn
    instances->WeakSet.add(error)->ignore
    error
  }

  let throw = (code, ~details=?) => JsExn.throw(make(code, ~details?))

  let isInstance = (value: JsExn.t) => instances->WeakSet.has(value)
}

// `copyResponse(response)`: refuses a response that is not from this origin (a URL-less one has
// origin null), then builds a new Response from a clone's headers, status, status text and body.
// `canConstructResponseFromBodyStream()` always answers false in 7.4.1, so the body is a Blob.
let copyResponse = async response => {
  let origin = switch response->Response.url {
  | "" => None
  | url => Some(Url.make(url)->Url.origin)
  }
  if origin != Some(Location.origin) {
    WorkboxError.throw(
      "cross-origin-copy-response",
      ~details=dict{"origin": origin->Null.fromOption->toUnknown},
    )
  }
  let clonedResponse = response->Response.clone
  let responseInit: Response.parts = {
    headers: Headers.copy(clonedResponse->Response.headers),
    status: clonedResponse->Response.status,
    statusText: clonedResponse->Response.statusText,
  }
  let body = await clonedResponse->Response.blob
  Response.fromBlob(body, responseInit)
}

let stripParams = (fullURL, ignoreParams) => {
  let strippedURL = Url.make(fullURL)
  ignoreParams->Array.forEach(param => strippedURL->Url.searchParams->SearchParams.delete(param))
  strippedURL->Url.href
}

// `cacheMatchIgnoreParams(cache, request, ignoreParams)`: an ordinary match when stripping the
// parameters changes nothing (deleting re-serialises the query), else the first key that strips
// to the same URL.
let cacheMatchIgnoreParams = async (cache, request, ignoreParams) => {
  let strippedRequestURL = stripParams(request->Request.url, ignoreParams)
  if request->Request.url === strippedRequestURL {
    await cache->Cache.matchRequest(request)
  } else {
    let cacheKeys = await cache->Cache.keysMatching(request, {ignoreSearch: true})
    switch cacheKeys->Array.find(cacheKey =>
      strippedRequestURL === stripParams(cacheKey->Request.url, ignoreParams)
    ) {
    | Some(cacheKey) => await cache->Cache.matchRequest(cacheKey)
    | None => None
    }
  }
}

// `timeout(ms)`: one timer task.
let timeout = ms => Promise.make((resolve, _) => setTimeout(() => resolve(), ms)->ignore)

// `waitUntil(event, asyncFn)`: the started promise extends the event's lifetime and is returned.
let waitUntil = (event, asyncFn) => {
  let returnPromise = asyncFn()
  event->Event.waitUntil(returnPromise)
  returnPromise
}

// `Deferred`: a promise with its resolver.
module Deferred = {
  type t = {promise: promise<unit>, resolve: unit => unit}
  let make = () => {
    let resolver = ref(() => ())
    let promise = Promise.make((resolve, _) => resolver := (() => resolve()))
    {promise, resolve: () => resolver.contents()}
  }
}
