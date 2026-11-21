# Ky 2.2.0: Scala.js port

This port is written by hand. It replaces the earlier port that `build/library-matrix/dev-ky/translate.py` generated from a shared core. It is a faithful port of Ky 2.2.0 (`distribution/index.js`, the subset in `libraries/ky/CONTRACT.md`) and of what `libraries/ky/reference.mjs` does with it, to Scala 3.8 and Scala.js 1.22. It follows Ky's own functions: `normalizeRetryOptions`, `validateTimeoutOption`, the constructor (`Ky.create`), `#run`, `#retry`, `#retryFromError`, `#fetch`, `#calculateRetryDelay`, `#calculateDelay`, `#getResponseData`, `#throwProcessedError`, `utils/timeout.js` and `core/retry-timing.js`.

| File | What it is |
| --- | --- |
| `src/Ky.scala` | Ky: its errors, options and their checks, header merging, the network-error test, the content-type and charset rules, and the `Ky` class |
| `src/RetryTiming.scala` | `core/retry-timing.js`: the timing headers and their delays |
| `src/Host.scala` | `@js.native` facades for Fetch, AbortController and AbortSignal, TextDecoder and `Number.isInteger` |
| `src/Main.scala` | the reference adapter (each case's transport, hooks and trace) and `run`, exported with `@JSExportTopLevel` |

## Boundary

The corpus input is the JS object itself, seen through `@js.native` facade traits (no copy). The option values Ky checks stay the JS values they are (`js.UndefOr[Any]`, then `Option[Any]`) until Ky's own checks read them with type patterns (`Double`, `Boolean`, `String`), `js.Array.isArray` and `Number.isInteger`. Each result is the JS array `[trace, outcome]`, and `run` returns a `js.Promise`.

## Interop decisions

- **Futures.** Ky's async methods give `Future`s on `JSExecutionContext.queue`, whose callbacks run as promise jobs, as an async function's do. Host promises cross with `toFuture` and the result with `toJSPromise`. `retryRequest` and `retryFromError` call each other as `#retry` and `#retryFromError` do. The transport is Ky's `fetch` option, a function to a `Future[Response]`.
- **Errors.** Ky's own errors are case classes of `KyError` (`HTTPError`, `NetworkError`, `TimeoutError`). Everything else is the JS error that was thrown, in a `js.JavaScriptException`: a host constructor's TypeError or RangeError, `JSON.parse`'s SyntaxError, an abort reason. Ky's option checks throw real JS errors (`js.TypeError`, `js.RangeError`, `js.Error`). `KyError.name` reads the JS `name`.
- **Ky's type tests.** Arrays are tested with `js.Array.isArray`, Ky's own `Array.isArray` (a `js.Array` type pattern would be `instanceof Array`). Ky's `isError` is `instanceof Error` or the `[object Error]` brand. Any thrown JS value reaches Scala as a `js.JavaScriptException`.
- **The timeout race** is Ky's `utils/timeout.js`: a `Promise` that the timer fails with a `TimeoutError` after aborting the controller, and that the transport's outcome completes otherwise; the first completion wins, as the first settlement of Ky's promise does.
- **JS semantics.** Strings are JS strings, and Scala.js's `toLowerCase` and `toUpperCase` are JS's. Ky's patterns run as `js.RegExp`s: Scala's own regular expressions follow `java.util.regex`, whose `\s` is ASCII only. Trimming uses `isJsWhitespace`: Java's `trim` removes every character up to U+0020 instead.
- **Dates.** `createTimestamp` counts days as ECMAScript's `MakeDay` does (`daysFromCivil`). It reproduces `Date.UTC`'s reading of a year from 0 to 99 as 1900 + year: the day has to exist in that year and in the year itself, so 29 Feb 0000 is rejected as Ky rejects it.
- **Host functions.** `js.JSON.parse` and `js.JSON.stringify`, `Number.isInteger`, `js.Object.keys`, `js.Object.hasProperty` (Ky's `in` test), `js.Date.now`, `js.Math.min` and `js.Math.max`, and `js.timers`.

## Deviations from the npm source

None changes a result in the parity domain.

- **One call is the whole `ky(input, options)[read]()`.** `Ky.request` runs the init hooks, builds the request, sets the shortcut's Accept header, runs the request and reads the body. Ky starts its run one microtask after the shortcut call; every step happens in the same order.
- **Hooks.** They receive what the adapter's hooks read (the request, response, error and retry count), not Ky's normalized options snapshot, and return nothing. The adapter's hooks never throw, so Ky's handling of a returned Request, Response, `ky.stop` or options, and of a throwing beforeRetry hook, would have nothing to act on. The afterResponse hooks receive the response itself rather than `response.clone()`; the adapter's hook reads only the status.
- **`#raceWithTotalTimeout` around a hook is its first step, the user-abort check.** `totalTimeout` is never set, and an abort inside a synchronous hook does not win the race (the hook's own promise settles first), so the race decides nothing else.
- **Body reads.** A successful body is read with `text()` and parsed with `JSON.parse`, without `#raceBodyRead`'s timeout race. The bodies are in memory, and the user signal cannot abort once a response is accepted. An error body is read whole (`arrayBuffer`) and decoded once rather than chunk by chunk, which gives the same text, and without the error-data timeout race. Its length is checked against the 10 MiB limit.
- **The retry wait is a plain timer.** Ky's `delay` also listens to the user signal, which the adapter never aborts during a wait.
- **Option merging** is the header part of `deepMerge`: the names are lowercased, and the last spelling of a name wins at the place of the first (a `LinkedHashMap`). The `content-type` test of `mergeHeaders({}, options.headers).has(...)` looks at those merged names, which `Headers.set` has already validated.
- **Not ported, because the contract never sets them:** `totalTimeout`, `maxResponseSize`, `parseJson`, `stringifyJson`, `retry.jitter`, `retry.shouldRetry`, `ky.retry()` (ForceRetryError), `searchParams`, `prefix`, `baseUrl` and the progress callbacks. Also left out is what only releases resources (`cancelBody`) or only matters for options outside the contract (`findUnknownOptions`, `duplex`, `NonError.wrap`, and the referrer that `#withManagedSignal` copies).
- **Error messages** follow Ky's where Ky's own checks throw. They are not compared.

## Performance accommodations

None.
