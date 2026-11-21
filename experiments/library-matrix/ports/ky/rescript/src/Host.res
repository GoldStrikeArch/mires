// The host APIs Ky drives: Fetch (Headers, Request, Response), AbortController and AbortSignal,
// TextDecoder, timers and the clock. Ky is a thin layer over these, so the port binds them as they
// are and every one of their semantics (method and URL normalisation, header validation, body
// encoding, label resolution) is the engine's own.

module AbortSignal = {
  type t
  @get external aborted: t => bool = "aborted"
  @get external reason: t => JsExn.t = "reason"
  @send external throwIfAborted: t => unit = "throwIfAborted"
  @val @scope("AbortSignal") external any: array<t> => t = "any"
  @send
  external onAbort: (t, @as("abort") _, unit => unit, @as(json`{"once":true}`) _) => unit =
    "addEventListener"
}

module AbortController = {
  type t
  @new external make: unit => t = "AbortController"
  @get external signal: t => AbortSignal.t = "signal"
  @send external abort: t => unit = "abort"
  @send external abortWith: (t, JsError.t) => unit = "abort"
}

module Headers = {
  type t
  @new external make: unit => t = "Headers"
  // `set` converts the value with the IDL ByteString rules, so any JSON value is accepted as JS would.
  @send external set: (t, string, JSON.t) => unit = "set"
  @send @return(nullable) external get: (t, string) => option<string> = "get"
}

module Request = {
  type t
  type init = {method?: string, headers?: Headers.t, body?: string, signal?: AbortSignal.t}
  @new external make: (string, init) => t = "Request"
  @new external copy: (t, init) => t = "Request"
  @send external clone: t => t = "clone"
  @get external method: t => string = "method"
  @get external url: t => string = "url"
  @get external headers: t => Headers.t = "headers"
  @get external signal: t => AbortSignal.t = "signal"
  @send external text: t => promise<string> = "text"
}

module Response = {
  type t
  type init = {status: JSON.t, headers: JSON.t}
  @new external make: (Null.t<string>, init) => t = "Response"
  @get external status: t => int = "status"
  @get external ok: t => bool = "ok"
  @get external headers: t => Headers.t = "headers"
  @get external body: t => Null.t<unknown> = "body"
  @send external text: t => promise<string> = "text"
  @send external arrayBuffer: t => promise<ArrayBuffer.t> = "arrayBuffer"
}

module TextDecoder = {
  type t
  @new external utf8: unit => t = "TextDecoder"
  // Throws a RangeError for a label the WHATWG Encoding Standard does not know.
  @new external make: string => t = "TextDecoder"
  @send external decode: (t, ArrayBuffer.t) => string = "decode"
}

type timer
@val external setTimer: (unit => unit, float) => timer = "setTimeout"
@val external clearTimer: timer => unit = "clearTimeout"

let sleep = milliseconds =>
  Promise.make((resolve, _) => setTimer(() => resolve(), milliseconds)->ignore)

// `Number(value)` and `Date.UTC`, which Ky's retry-timing helpers apply to its captures.
@val external toNumber: string => float = "Number"
@val @scope("Date")
external utc: (float, float, float, float, float, float) => float = "UTC"
@val @scope("Date")
external utcMilliseconds: (float, float, float, float, float, float, float) => float = "UTC"
@set external setErrorName: (JsError.t, string) => unit = "name"
