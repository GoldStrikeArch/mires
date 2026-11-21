// Ky 2.2.0 (core/Ky.js, core/constants.js, utils/normalize.js, utils/merge.js, utils/timeout.js,
// utils/is-network-error.js and errors/*.js), the subset in libraries/ky/CONTRACT.md.

open Host

// errors/*.js. Ky's own failures are ReScript exceptions; whatever a host API, the transport or
// Ky's option checks threw (TypeError, RangeError, SyntaxError, an abort reason) stays the JS
// value it is, as `JsExn`.
exception HTTPError({response: Response.t, request: Request.t, data: option<JSON.t>})
exception NetworkError({request: Request.t, cause: JsExn.t})
exception TimeoutError({request: Request.t})

let errorName = (error: exn) =>
  switch error {
  | HTTPError(_) => "HTTPError"
  | NetworkError(_) => "NetworkError"
  | TimeoutError(_) => "TimeoutError"
  | JsExn(value) => value->JsExn.name->Option.getOr("Error")
  | _ => "Error"
  }

type beforeRequestState = {request: Request.t, retryCount: int}
type afterResponseState = {request: Request.t, response: Response.t, retryCount: int}
type beforeRetryState = {request: Request.t, error: exn, retryCount: int}
type beforeErrorState = {request: Request.t, error: exn, retryCount: int}

type hooks = {
  init: array<unit => unit>,
  beforeRequest: array<beforeRequestState => unit>,
  afterResponse: array<afterResponseState => unit>,
  beforeRetry: array<beforeRetryState => unit>,
  beforeError: array<beforeErrorState => unit>,
}

// The caller's retry options, as the JS values given; `normalizeRetryOptions` validates them.
type retryOptions = {
  limit?: JSON.t,
  methods?: JSON.t,
  statusCodes?: JSON.t,
  maxRetryAfter?: JSON.t,
  backoffLimit?: JSON.t,
  retryOnTimeout?: JSON.t,
  delay: int => float,
}

type options = {
  method?: string,
  headers?: dict<JSON.t>,
  json?: JSON.t,
  retry: retryOptions,
  timeout?: JSON.t,
  throwHttpErrors?: JSON.t,
  hooks: hooks,
  fetch: Request.t => promise<Response.t>,
  signal?: AbortSignal.t,
}

type retry = {
  limit: float,
  methods: array<string>,
  statusCodes: array<float>,
  afterStatusCodes: array<float>,
  maxRetryAfter: float,
  backoffLimit: float,
  delay: int => float,
  retryOnTimeout: bool,
}

// core/constants.js and utils/normalize.js.
let requestMethods = ["get", "post", "put", "patch", "head", "delete", "query"]
let caseInsensitiveMethods = [...requestMethods, "options", "trace"]
let retryMethods = ["get", "put", "head", "delete", "options", "trace", "query"]
let retryStatusCodes = [408., 413., 429., 500., 502., 503., 504.]
let retryAfterStatusCodes = [413., 429., 503.]
// The maximum value of a 32bit int (see issue #117)
let maxSafeTimeout = 2_147_483_647.
let maxErrorResponseBodySize = 10 * 1024 * 1024

let normalizeRequestMethod = input =>
  requestMethods->Array.includes(input->String.toLowerCase) ? input->String.toUpperCase : input

// Normalizes common methods for retry matching while preserving case-sensitive custom methods.
let normalizeRetryMethod = method =>
  caseInsensitiveMethods->Array.includes(method->String.toLowerCase)
    ? method->String.toLowerCase
    : method

@val @scope("Number") external isInteger: float => bool = "isInteger"

let normalizeRetryOptions = (retry: retryOptions): retry => {
  // An omitted value takes the default; any other value, `null` included, is validated.
  let limit = switch retry.limit {
  | None => 2.
  | Some(Number(limit)) if isInteger(limit) && limit >= 0. => limit
  | Some(_) => JsError.TypeError.throwWithMessage("`retry.limit` must be a finite, non-negative integer")
  }
  let array = (key, value, default) =>
    switch value {
    | None => default
    | Some(JSON.Array(values)) => values
    | Some(_) => JsError.throwWithMessage(`retry.${key} must be an array`)
    }
  let methods = array("methods", retry.methods, retryMethods->Array.map(method => JSON.String(method)))
  let statusCodes = array(
    "statusCodes",
    retry.statusCodes,
    retryStatusCodes->Array.map(status => JSON.Number(status)),
  )
  // A mistyped entry only ever fails to match, so it is reported instead.
  let methods = methods->Array.map(method =>
    switch method {
    | String(method) => method
    | _ => JsError.TypeError.throwWithMessage("`retry.methods` must only contain strings")
    }
  )
  let statusCodes = statusCodes->Array.map(status =>
    switch status {
    | Number(status) if isInteger(status) => status
    | _ => JsError.TypeError.throwWithMessage("`retry.statusCodes` must only contain numbers")
    }
  )
  let limitOption = (key, value) =>
    switch value {
    | None => Float.Constants.positiveInfinity
    | Some(JSON.Number(value)) if value >= 0. => value
    | Some(_) =>
      JsError.TypeError.throwWithMessage(`\`retry.${key}\` must be a non-negative number or \`Infinity\``)
    }
  let maxRetryAfter = limitOption("maxRetryAfter", retry.maxRetryAfter)
  let backoffLimit = limitOption("backoffLimit", retry.backoffLimit)
  let retryOnTimeout = switch retry.retryOnTimeout {
  | None => false
  | Some(Boolean(retryOnTimeout)) => retryOnTimeout
  | Some(_) => JsError.TypeError.throwWithMessage("`retry.retryOnTimeout` must be a boolean")
  }
  {
    limit,
    methods: methods->Array.map(normalizeRetryMethod),
    statusCodes,
    afterStatusCodes: retryAfterStatusCodes,
    maxRetryAfter,
    backoffLimit,
    delay: retry.delay,
    retryOnTimeout,
  }
}

// Both timeout options are milliseconds or `false`.
let validateTimeoutOption = (value: JSON.t, name) =>
  switch value {
  | Boolean(false) => None
  | Number(milliseconds) if milliseconds >= 0. =>
    milliseconds > maxSafeTimeout
      ? JsError.RangeError.throwWithMessage(
          `The \`${name}\` option cannot be greater than ${Float.toString(maxSafeTimeout)}`,
        )
      : Some(milliseconds)
  | _ => JsError.TypeError.throwWithMessage(`The \`${name}\` option must be a non-negative number or \`false\``)
  }

// utils/merge.js. Option merging keys the headers by their lowercased name (the last spelling of a
// name wins, at the place of its first), and `mergeHeaders` sets each on a fresh Headers.
let lowercaseHeaders = (headers: dict<JSON.t>) => {
  let result = Map.make()
  headers->Dict.forEachWithKey((value, key) => result->Map.set(key->String.toLowerCase, value))
  result
}

let mergeHeaders = (headers: Map.t<string, JSON.t>) => {
  let result = Headers.make()
  headers->Map.forEachWithKey((value, key) => result->Headers.set(key, value))
  result
}

// utils/is-network-error.js, inlined from is-network-error 1.3.2.
let networkErrorMessages = [
  "network error",
  "NetworkError when attempting to fetch resource.",
  "The Internet connection appears to be offline.",
  "Network request failed",
  "fetch failed",
  "terminated",
  " A network error occurred.",
  "Network connection lost",
]

@get external errorStack: JsExn.t => Nullable.t<unknown> = "stack"
@val @scope("Reflect") external has: (JsExn.t, string) => bool = "has"

type prototype
@val @scope("Error") external errorPrototype: prototype = "prototype"
@send external isPrototypeOf: (prototype, JsExn.t) => bool = "isPrototypeOf"
@val @scope(("Object", "prototype", "toString")) external toStringTag: JsExn.t => string = "call"

// Ky's isError: an Error of this realm, or one of another realm by its internal brand.
let isError = error => errorPrototype->isPrototypeOf(error) || toStringTag(error) == "[object Error]"

let isRawNetworkError = (error: JsExn.t) =>
  switch (error->JsExn.name, error->JsExn.message) {
  | (Some("TypeError"), Some(message)) if isError(error) =>
    if message == "Load failed" || (message->String.startsWith("Load failed (") && message->String.endsWith(")")) {
      // Safari 17+ has generic message but no stack for network errors
      errorStack(error) == Nullable.undefined || has(error, "__sentry_captured__")
    } else {
      message->String.startsWith("error sending request for url") ||
      message == "Failed to fetch" ||
      (message->String.startsWith("Failed to fetch (") && message->String.endsWith(")")) ||
      networkErrorMessages->Array.includes(message)
    }
  | _ => false
  }

// Matches JSON subtypes like `json`, `problem+json`, and `vnd.api+json`, written out rather than
// matched with a pattern, as Ky does.
let isJsonContentType = contentType => {
  let mimeType =
    contentType
    ->String.splitAtMost(";", ~limit=1)
    ->Array.get(0)
    ->Option.getOr("")
    ->String.trim
    ->String.toLowerCase
  if !(mimeType->String.endsWith("json")) {
    false
  } else {
    let index = mimeType->String.length - 5
    switch index >= 0 ? mimeType->String.charAt(index) : "" {
    | "/" => true
    // The separator is not the slash, so any slash in the type comes before it.
    | "." | "+" | "-" => mimeType->String.includes("/")
    | _ => false
    }
  }
}

let charsetPattern = /;\s*charset\s*=\s*(?:"([^"]+)"|([^;,\s]+))/i

let createTextDecoder = contentType => {
  let charset = charsetPattern->RegExp.exec(contentType)->Option.flatMap(match => {
    let groups = match->RegExp.Result.matches
    groups->Array.getUnsafe(0)->Option.orElse(groups->Array.getUnsafe(1))
  })
  switch charset {
  | Some(charset) if charset != "" =>
    try TextDecoder.make(charset) catch {
    | JsExn(_) => TextDecoder.utf8()
    }
  | _ => TextDecoder.utf8()
  }
}

// Reads an error body as text, or `None` when the body cannot be read or is larger than
// `maxErrorResponseBodySize`. JSON uses UTF-8 regardless of the charset parameter (RFC 8259).
let readResponseText = async response =>
  switch response->Response.body->Null.toOption {
  | None =>
    switch await response->Response.text {
    | text => Some(text)
    | exception JsExn(_) => None
    }
  | Some(_) =>
    let contentType = response->Response.headers->Headers.get("content-type")->Option.getOr("")
    let decoder = isJsonContentType(contentType) ? TextDecoder.utf8() : createTextDecoder(contentType)
    switch await response->Response.arrayBuffer {
    | bytes if bytes->ArrayBuffer.byteLength > maxErrorResponseBodySize => None
    | bytes => Some(decoder->TextDecoder.decode(bytes))
    | exception JsExn(_) => None
    }
  }

// utils/timeout.js: `Promise.race()` workaround (#91).
let timeout = (request, abortController, milliseconds, fetch) => {
  let timeoutId = ref(None)
  let timedOut = ref(false)
  Promise.make((resolve, reject) => {
    timeoutId := Some(setTimer(() => {
          timedOut := true
          abortController->AbortController.abort
          reject(TimeoutError({request: request}))
        }, milliseconds))
    let forward = async () =>
      switch await fetch(request) {
      // A response arriving after the timeout already won is discarded.
      | response => if !timedOut.contents {
          resolve(response)
        }
      | exception error => reject(error)
      }
    forward()->ignore
  })->Promise.finally(() => timeoutId.contents->Option.forEach(clearTimer))
}

type t = {
  mutable request: Request.t,
  mutable abortController: AbortController.t,
  mutable signal: AbortSignal.t,
  mutable retryCount: int,
  mutable originalRequest: option<Request.t>,
  userProvidedAbortSignal: option<AbortSignal.t>,
  retry: retry,
  timeout: option<float>,
  throwHttpErrors: bool,
  hooks: hooks,
  fetch: Request.t => promise<Response.t>,
}

let createManagedSignal = (userProvidedAbortSignal, abortController) =>
  switch userProvidedAbortSignal {
  | Some(signal) => AbortSignal.any([signal, abortController->AbortController.signal])
  | None => abortController->AbortController.signal
  }

let make = (input, options: options) => {
  let headers = lowercaseHeaders(options.headers->Option.getOr(dict{}))
  let requestHeaders = mergeHeaders(headers)
  let method = normalizeRequestMethod(options.method->Option.getOr("GET"))
  let retry = normalizeRetryOptions(options.retry)
  // Defaults only replace `undefined`, so a `null` reaches the validation and is reported.
  let throwHttpErrors = switch options.throwHttpErrors {
  | None => true
  | Some(Boolean(throwHttpErrors)) => throwHttpErrors
  | Some(_) => JsError.TypeError.throwWithMessage("The `throwHttpErrors` option must be a boolean or a function")
  }
  let timeout = validateTimeoutOption(options.timeout->Option.getOr(Number(10_000.)), "timeout")
  let abortController = AbortController.make()
  let signal = createManagedSignal(options.signal, abortController)
  // A content-type from `options.headers` counts as user-provided.
  let userProvidedContentType = options.headers->Option.isSome && headers->Map.has("content-type")
  let body = options.json->Option.map(json => JSON.stringify(json))
  if body->Option.isSome && !userProvidedContentType {
    requestHeaders->Headers.set("content-type", String("application/json"))
  }
  {
    request: Request.make(input, {method, headers: requestHeaders, ?body, signal}),
    abortController,
    signal,
    retryCount: 0,
    originalRequest: None,
    userProvidedAbortSignal: options.signal,
    retry,
    timeout,
    throwHttpErrors,
    hooks: options.hooks,
    fetch: options.fetch,
  }
}

// `#fetch()` replaces `request` with the clone it prepares for a possible retry, so anything
// reporting the request that produced a response or failed uses the one that was actually sent.
let sentRequest = ky => ky.originalRequest->Option.getOr(ky.request)

// Chromium reports a user abort during a fetch as a network error; surface the abort reason instead.
let throwIfAbortedByUser = ky => ky.userProvidedAbortSignal->Option.forEach(AbortSignal.throwIfAborted)

let withManagedSignal = (ky, request) => Request.copy(request, {signal: ky.signal})

let fetch = async ky => {
  // Cloning is done here to prepare in advance for retries, and skipped when retries are disabled.
  let retryRequest = ky.retry.limit > 0. ? Some(ky->withManagedSignal(ky.request->Request.clone)) : None
  let request = ky.request
  ky.originalRequest = Some(request)
  retryRequest->Option.forEach(retryRequest => ky.request = retryRequest)
  switch await (
    switch ky.timeout {
    | None => ky.fetch(request)
    | Some(milliseconds) => timeout(request, ky.abortController, milliseconds, ky.fetch)
    }
  ) {
  | response => response
  | exception error =>
    ky->throwIfAbortedByUser
    switch error {
    | JsExn(cause) if isRawNetworkError(cause) => throw(NetworkError({request, cause}))
    | error => throw(error)
    }
  }
}

let calculateDelay = (ky, _error) => {
  let retryDelay = ky.retry.delay(ky.retryCount + 1)
  if !(retryDelay >= 0.) {
    JsError.TypeError.throwWithMessage("`retry.delay` must return a non-negative number or `Infinity`")
  }
  Math.min(ky.retry.backoffLimit, retryDelay)
}

let calculateRetryDelay = (ky, error) => {
  if Int.toFloat(ky.retryCount) >= ky.retry.limit {
    throw(error)
  }
  if !(ky.retry.methods->Array.includes(normalizeRetryMethod(ky.request->Request.method))) {
    throw(error)
  }
  switch error {
  | TimeoutError(_) => ky.retry.retryOnTimeout ? ky->calculateDelay(error) : throw(error)
  | HTTPError({response}) =>
    let status = response->Response.status->Int.toFloat
    if !(ky.retry.statusCodes->Array.includes(status)) {
      throw(error)
    }
    switch RetryTiming.getRetryTimingHeader(response->Response.headers) {
    | Some(header) if ky.retry.afterStatusCodes->Array.includes(status) =>
      switch RetryTiming.calculateRetryTimingDelay(header) {
      // Malformed retry timing headers should not disable retries; they only lose their timing.
      | None => ky->calculateDelay(error)
      // Don't apply jitter when server provides explicit retry timing
      | Some(after) => Math.min(ky.retry.maxRetryAfter, after)
      }
    | _ => status == 413. ? throw(error) : ky->calculateDelay(error)
    }
  // Only retry known retriable error types.
  | NetworkError(_) => ky->calculateDelay(error)
  | _ => throw(error)
  }
}

// A hook runs inside `#raceWithTotalTimeout()`, which first throws a user abort.
let runHook = (ky, hook, state) => {
  ky->throwIfAbortedByUser
  hook(state)
}

let rec retry = async ky =>
  switch await ky->fetch {
  | response => response
  | exception error => await ky->retryFromError(error)
  }
and retryFromError = async (ky, error) => {
  ky->throwIfAbortedByUser
  let retryDelay = Math.min(ky->calculateRetryDelay(error), maxSafeTimeout)
  await sleep(retryDelay)
  // Reset abortController if it was aborted (happens on timeout retry) so hooks and the retried
  // request get a fresh signal.
  if ky.abortController->AbortController.signal->AbortSignal.aborted {
    ky.abortController = AbortController.make()
    ky.signal = createManagedSignal(ky.userProvidedAbortSignal, ky.abortController)
    ky.request = ky->withManagedSignal(ky.request)
  }
  ky.hooks.beforeRetry->Array.forEach(hook =>
    ky->runHook(hook, {request: ky.request, error, retryCount: ky.retryCount + 1})
  )
  ky.retryCount = ky.retryCount + 1
  await ky->retry
}

let getResponseData = async response =>
  switch await readResponseText(response) {
  | None | Some("") => None
  | Some(text)
    if !isJsonContentType(response->Response.headers->Headers.get("content-type")->Option.getOr("")) =>
    Some(JSON.String(text))
  // Unparsable error data is given up on, like unreadable error data.
  | Some(text) =>
    try Some(JSON.parseOrThrow(text)) catch {
    | JsExn(_) => None
    }
  }

let run = async ky => {
  ky.hooks.beforeRequest->Array.forEach(hook =>
    ky->runHook(hook, {request: ky.request, retryCount: 0})
  )
  let response = ref(await ky->retry)
  let settled = ref(false)
  while !settled.contents {
    ky.hooks.afterResponse->Array.forEach(hook =>
      ky->runHook(hook, {request: ky.request, response: response.contents, retryCount: ky.retryCount})
    )
    if response.contents->Response.ok || !ky.throwHttpErrors {
      settled := true
    } else {
      let data = await getResponseData(response.contents)
      let httpError = HTTPError({response: response.contents, request: ky->sentRequest, data})
      ky->throwIfAbortedByUser
      response := await ky->retryFromError(httpError)
    }
  }
  response.contents
}

let throwProcessedError = (ky, error) => {
  ky.hooks.beforeError->Array.forEach(hook =>
    hook({request: ky->sentRequest, error, retryCount: ky.retryCount})
  )
  throw(error)
}

// The body shortcuts and the media type each advertises.
type shortcut = Json | Text

let mimeType = shortcut =>
  switch shortcut {
  | Json => "application/json"
  | Text => "text/*"
  }

// `ky(input, options)[shortcut]()`: the init hooks, the request, the run and the body read. A
// construction error rejects without running the beforeError hooks, and so does a body that is
// not JSON.
let request = async (input, options: options, shortcut) => {
  options.hooks.init->Array.forEach(hook => hook())
  let ky = make(input, options)
  let headers = ky.request->Request.headers
  let accept = switch headers->Headers.get("accept") {
  | Some(accept) if accept != "" => accept
  | _ => mimeType(shortcut)
  }
  headers->Headers.set("accept", String(accept))
  let response = switch await ky->run {
  | response => response
  | exception error => ky->throwProcessedError(error)
  }
  let text = await response->Response.text
  switch shortcut {
  | Text => JSON.String(text)
  | Json => JSON.parseOrThrow(text)
  }
}
