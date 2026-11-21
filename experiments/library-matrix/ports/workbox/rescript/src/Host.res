// The host APIs Workbox drives: CacheStorage and Cache, Fetch (Request, Response, Headers), URL
// and URLSearchParams, the ExtendableEvent, the location and crypto. Workbox is a layer over these,
// so their semantics (URL parsing and serialisation, request and response construction, cache
// matching, integrity checks) are the engine's own.

module Headers = {
  type t
  @new external copy: t => t = "Headers"
}

module Request = {
  type t
  type init = {
    method?: string,
    mode?: string,
    integrity?: string,
    cache?: string,
    credentials?: string,
    headers?: Headers.t,
  }
  @new external make: (string, init) => t = "Request"
  @new external fromUrl: string => t = "Request"
  @new external copy: (t, init) => t = "Request"
  @send external clone: t => t = "clone"
  @get external url: t => string = "url"
  @get external method: t => string = "method"
  @get external mode: t => string = "mode"
  @get external integrity: t => string = "integrity"
  @get external headers: t => Headers.t = "headers"
}

module Blob = {
  type t
}

module Response = {
  type t
  type init = {status?: int}
  type parts = {headers: Headers.t, status: int, statusText: string}
  @new external make: (string, init) => t = "Response"
  @new external fromBlob: (Blob.t, parts) => t = "Response"
  @send external clone: t => t = "clone"
  @get external status: t => int = "status"
  @get external statusText: t => string = "statusText"
  @get external url: t => string = "url"
  @get external redirected: t => bool = "redirected"
  @get external type_: t => string = "type"
  @get external headers: t => Headers.t = "headers"
  @send external text: t => promise<string> = "text"
  @send external blob: t => promise<Blob.t> = "blob"
}

@val external fetch: Request.t => promise<Response.t> = "fetch"

module Cache = {
  type t
  type queryOptions = {ignoreSearch: bool}
  // Each match resolves with undefined when nothing matches.
  @send external matchRequest: (t, Request.t) => promise<option<Response.t>> = "match"
  @send external matchUrl: (t, string) => promise<option<Response.t>> = "match"
  @send external put: (t, Request.t, Response.t) => promise<unit> = "put"
  @send external delete: (t, Request.t) => promise<bool> = "delete"
  @send external keys: t => promise<array<Request.t>> = "keys"
  @send external keysMatching: (t, Request.t, queryOptions) => promise<array<Request.t>> = "keys"
}

module CacheStorage = {
  type multiCacheQueryOptions = {cacheName: string}
  @val @scope("caches") external openCache: string => promise<Cache.t> = "open"
  @val @scope("caches")
  external matchRequest: (Request.t, multiCacheQueryOptions) => promise<option<Response.t>> =
    "match"
  @val @scope("caches") external keys: unit => promise<array<string>> = "keys"
  @val @scope("caches") external delete: string => promise<bool> = "delete"
}

module SearchParams = {
  type t
  @send external keys: t => Iterator.t<string> = "keys"
  // The host converts a number value with Number-to-String.
  @send external set: (t, string, JSON.t) => unit = "set"
  @send external delete: (t, string) => unit = "delete"
}

module Url = {
  type t
  @new external make: string => t = "URL"
  @new external makeWithBase: (string, string) => t = "URL"
  @get external href: t => string = "href"
  @get external origin: t => string = "origin"
  @get external protocol: t => string = "protocol"
  @get external pathname: t => string = "pathname"
  @set external setPathname: (t, string) => unit = "pathname"
  @set external setHash: (t, string) => unit = "hash"
  @get external searchParams: t => SearchParams.t = "searchParams"
}

module Location = {
  @val @scope("location") external href: string = "href"
  @val @scope("location") external origin: string = "origin"
}

// An ExtendableEvent: a native install, activate or fetch event, or the stand-in a page supplies.
module Event = {
  type t
  @get external type_: t => string = "type"
  @send external waitUntil: (t, promise<'a>) => unit = "waitUntil"
}

@val @scope("crypto") external randomUUID: unit => string = "randomUUID"

// `value instanceof Error`: Workbox tests it before it keeps or reports a caught value, because a
// plugin may throw anything.
@val @scope(("Error", "prototype")) external isError: JsExn.t => bool = "isPrototypeOf"
@get external errorName: JsExn.t => string = "name"
@get external errorMessage: JsExn.t => string = "message"

// `Promise.reject(value)` with the value itself; the Stdlib's `reject` takes a ReScript exception.
@val @scope("Promise") external rejected: JsExn.t => promise<'a> = "reject"
