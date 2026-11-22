# Ky 2.2.0: Elm port

This port is written by hand. Its earlier version kept nearly all of Ky in the JS host (request construction, timeouts and aborts, network-error wrapping, HTTPError data and the retry loop), with Elm computing only the retry delay. Now all of Ky's logic, and the reference adapter's transport and hooks, are Elm. The JS host only performs primitive host operations and answers with their results (the list is in `libraries/ky/CONTRACT.md`, "Elm host primitives"). It is a faithful port of Ky 2.2.0 (`distribution/index.js`, the subset in `libraries/ky/CONTRACT.md`) and of what `libraries/ky/reference.mjs` does with it, to Elm 0.19.2. It follows Ky's own functions: `normalizeRetryOptions`, `validateTimeoutOption`, the constructor (`create`), `#run`, `#retry`, `#retryFromError`, `#fetch`, `#calculateRetryDelay`, `#calculateDelay`, `#getResponseData`, `#throwProcessedError`, `utils/timeout.js` and `core/retry-timing.js`.

| File | What it is |
| --- | --- |
| `src/Ky.elm` | Ky: `Failure`, the options and their checks, header merging, the network-error test, the content-type and charset rules, and the request's run |
| `src/RetryTiming.elm` | `core/retry-timing.js`: the timing headers and their delays |
| `src/Host.elm` | `Io`, the type Ky is written in, and the host operations as `Io` actions |
| `src/Main.elm` | the reference adapter (each case's transport, hooks and trace), the interpreter that runs `Io` against the host, and the ports |
| `js/index.js` | the host: performs each operation and answers; no Ky logic |

## Boundary

The host sends the corpus input through `start`. Each request case is read in place with elm/json, field by field. The option values Ky checks stay `D.Value`s until Ky's own checks decode them: elm/json's `float`, `bool`, `string` and `list` decoders are `typeof` and `Array.isArray` tests, and its `int` decoder accepts exactly the values `Number.isInteger` does. An option is omitted when its property is missing or undefined (`present`). The program asks the host for each operation through `command` and gets each answer through `reply`; the result, one `[trace, outcome]` per case, goes back through `command` as `{ done }`.

## Io

- **`Io s a`** is an action that gives an `a`: `Done`, `Ask` (one host operation, and what to do with its answer: what the operation gave, or the JS error it threw), `Modify` (the caller's state `s`, here the adapter's trace and reply count) or `Timed` (the transport raced against a timer, for Ky's per-attempt timeout). `Fail` ends the call when the host answers against the protocol, so `run` rejects instead of going on with an invented value.
- **Ky's async methods are `Io` actions**, and the request object Ky mutates is a `Ky` record that each step hands to the next. `retry` and `retryFromError` call each other as `#retry` and `#retryFromError` do; each recursive call is the last step of a continuation, so neither the stack nor the continuations grow with the number of retries.
- **The interpreter** (`Main.elm`) runs the program until it waits on the host, then resumes it with the matching answer. During a timed attempt it waits for the transport's next answer and for the timer at once. If the transport settles first, the timer is cleared; if the timer fires first, the transport is abandoned, its late answer is dropped, and Ky aborts the controller and fails the attempt with a TimeoutError, as `utils/timeout.js` does.
- **Host objects stay in the host**, behind handles (`Request`, `Response` and `AbortController` records carry their handles and the properties Ky reads: method, URL, status, `ok`, whether there is a body). Each `run` call instantiates its own Elm worker and handle table with no module-level state, unsubscribes and clears any remaining timers on completion, and drops any late answer arriving after its call has ended.

## Interop decisions

- **Errors.** `Failure` is `HttpError`, `NetworkError`, `TimeoutError` (Ky's own errors) or `Thrown` of a JS error as Ky reads one: its name, its message, whether it is an Error, whether it has a stack and whether it carries `__sentry_captured__`. The host describes every error it catches this way (whether it is an Error by Ky's `isError`: `instanceof Error` or the `[object Error]` brand); Ky's option checks make their TypeErrors, RangeErrors and Errors as such descriptions.
- **JS semantics.** Elm strings are JS strings. `String.toLower`, `toUpper`, `trim` and `trimLeft` are JS's `toLowerCase`, `toUpperCase`, `trim` and `trimStart`. `Char.isDigit` is ASCII only, as `\d` is.
- **Patterns.** elm/core has no regular expressions. Each of Ky's three HTTP-date patterns has a fixed length once the weekday is known, so `RetryTiming` checks each field at the position the pattern fixes. `charset` follows `/;\s*charset\s*=\s*(?:"([^"]+)"|([^;,\s]+))/i` with JS's whitespace set (`isJsWhitespaceUnit`) and an ASCII-only case match, as a non-Unicode pattern has, scanning `;` positions once in O(N) with tail-recursive index helpers so long or semicolon-rich `Content-Type` headers neither overflow the stack nor go quadratic.
- **Dates.** `createTimestamp` counts days as ECMAScript's `MakeDay` does (`daysFromCivil`). It reproduces `Date.UTC`'s reading of a year from 0 to 99 as 1900 + year: the day has to exist in that year and in the year itself, so 29 Feb 0000 is rejected as Ky rejects it. The clock is the host's `Date.now()`.
- **JSON.** `Json.Encode.encode 0` is `JSON.stringify` (deferred until after option validation so validation errors take precedence, as in Ky), and `Json.Decode.decodeString` parses with `JSON.parse`; a failed parse is Ky's SyntaxError.

## Deviations from the npm source

None changes a result in the parity domain.

- **One call is the whole `ky(input, options)[read]()`.** `Ky.request` runs the init hooks, builds the request, sets the shortcut's Accept header, runs the request and reads the body. Ky starts its run one microtask after the shortcut call; every step happens in the same order.
- **Hooks.** They receive what the adapter's hooks read (the request, response, error and retry count), not Ky's normalized options snapshot, and give nothing back. The adapter's hooks never throw, so Ky's handling of a returned Request, Response, `ky.stop` or options, and of a throwing beforeRetry hook, would have nothing to act on. The afterResponse hooks receive the response itself rather than `response.clone()`; the adapter's hook reads only the status.
- **`#raceWithTotalTimeout` around a hook is its first step, the user-abort check.** `totalTimeout` is never set, and an abort inside a synchronous hook does not win the race (the hook's own promise settles first), so the race decides nothing else.
- **Body reads.** A successful body is read with `text()` and parsed, without `#raceBodyRead`'s timeout race. The bodies are in memory, and the user signal cannot abort once a response is accepted. An error body is read whole (`arrayBuffer`) and decoded once rather than chunk by chunk, which gives the same text, and without the error-data timeout race. Its length is checked against the 10 MiB limit.
- **The retry wait is a plain timer.** Ky's `delay` also listens to the user signal, which the adapter never aborts during a wait.
- **Option merging** is the header part of `deepMerge`: the names are lowercased, and the last spelling of a name wins at the place of the first. The `content-type` test of `mergeHeaders({}, options.headers).has(...)` looks at those merged names, which `Headers.set` has already validated.
- **Not ported, because the contract never sets them:** `totalTimeout`, `maxResponseSize`, `parseJson`, `stringifyJson`, `retry.jitter`, `retry.shouldRetry`, `ky.retry()` (ForceRetryError), `searchParams`, `prefix`, `baseUrl` and the progress callbacks. Also left out is what only releases resources (`cancelBody`) or only matters for options outside the contract (`findUnknownOptions`, `duplex`, `NonError.wrap`, and the referrer that `#withManagedSignal` copies).
- **Error messages** follow Ky's where Ky's own checks throw. They are not compared.

## Performance accommodations

`lowercaseHeaders` uses `Dict` plus a reversed unique-key list (folded back into first-spelling order) so wide header objects merge in O(N log N) time instead of O(N²) list appends.
