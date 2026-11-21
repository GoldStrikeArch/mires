# Ky 2.2.0: Dart port

This port is written by hand. It replaces the earlier port that `build/library-matrix/dev-ky/dart.py` generated from a shared core. It is a faithful port of Ky 2.2.0 (`distribution/index.js`, the subset in `libraries/ky/CONTRACT.md`) and of what `libraries/ky/reference.mjs` does with it, to Dart 3 compiled by dart2js. It follows Ky's own functions: `normalizeRetryOptions`, `validateTimeoutOption`, the constructor (`Ky._create`), `#run`, `#retry`, `#retryFromError`, `#fetch`, `#calculateRetryDelay`, `#calculateDelay`, `#getResponseData`, `#throwProcessedError`, `utils/timeout.js` and `core/retry-timing.js`.

| File | What it is |
| --- | --- |
| `src/ky.dart` | Ky: its errors, options and their checks, header merging, the network-error test, the content-type and charset rules, and the `Ky` class |
| `src/retry_timing.dart` | `core/retry-timing.js`: the timing headers and their delays |
| `src/host.dart` | `dart:js_interop` extension types for Fetch, AbortController and AbortSignal, TextDecoder, the JS error constructors and the JS globals Ky calls; `hostCall` |
| `src/main.dart` | the reference adapter (each case's transport, hooks and trace) and `run`, which `main()` publishes on `globalThis.__matrixExports` for the builder's facade |

## Boundary

The corpus input is the JS object itself, seen through extension types (no copy). The option values Ky checks stay the JS values they are (`JSAny?`) until Ky's own checks read them with `isUndefined` and `isA<JSNumber>()` and its siblings; on the web, JS null and undefined are both Dart null, and `isUndefined` tells them apart. Each result is the JS array `[trace, outcome]`, and `run` returns a `JSPromise`.

## Interop decisions

- **Async functions.** Ky's async methods are Dart `async` functions, and `_retryRequest` and `_retryFromError` call each other as `#retry` and `#retryFromError` do. The transport is Ky's `fetch` option, a function to a `Future<Response>`.
- **Errors.** Ky's own errors are the `KyError` classes (`HTTPError`, `NetworkError`, `TimeoutError`). Everything else is the JS error that was thrown, a `JSObject`: a host constructor's TypeError or RangeError, `JSON.parse`'s SyntaxError, an abort reason. Ky's option checks throw real JS errors (the `HostTypeError`, `HostRangeError` and `HostError` constructors). `errorName` reads the JS `name`.
- **`hostCall`.** dart2js turns a JS TypeError or RangeError caught by `catch` into a Dart error that has lost its name (a TypeError becomes an unknown type error, a RangeError an `ArgumentError`). The host calls that can throw one, and whose error Ky reports (`Headers.set`, `new Request`, and the transport's `new Response`), therefore run inside a JS Promise executor, whose rejection keeps the thrown value. A promise's rejection and a JS value Dart throws itself keep their identity through `await` and `catch`. `isHostError` is Ky's `isError`: `instanceof Error` or the `[object Error]` brand.
- **The timeout race** is Ky's `utils/timeout.js`: a `Completer` that the timer fails with a `TimeoutError` after aborting the controller, and that the transport's outcome completes otherwise; the first completion wins, as the first settlement of Ky's promise does.
- **JS semantics.** On the web, Dart strings are JS strings; `toLowerCase` and `toUpperCase` are JS's, and `RegExp` is the JS one, so Ky's own patterns keep their meaning. Trimming uses `_isJsWhitespace`: Dart's `trim` would also strip U+0085.
- **Dates.** `_createTimestamp` counts days as ECMAScript's `MakeDay` does (`_daysFromCivil`). It reproduces `Date.UTC`'s reading of a year from 0 to 99 as 1900 + year: the day has to exist in that year and in the year itself, so 29 Feb 0000 is rejected as Ky rejects it. Dart's `DateTime.utc` does not shift those years.
- **Timers** are `Timer` and `Future.delayed`, whose durations dart2js hands to `setTimeout` in whole milliseconds.
- **Host functions.** `JSON.parse` and `JSON.stringify`, `Number.isInteger`, `Object.keys`, `Reflect.has` (Ky's `in` test), and `DateTime.now` for `Date.now()`.

## Deviations from the npm source

None changes a result in the parity domain.

- **One call is the whole `ky(input, options)[read]()`.** `Ky.send` runs the init hooks, builds the request, sets the shortcut's Accept header, runs the request and reads the body. Ky starts its run one microtask after the shortcut call; every step happens in the same order. The constructor is async here, for `hostCall`; it still runs before anything else of the request.
- **Hooks.** They receive what the adapter's hooks read (the request, response, error and retry count), not Ky's normalized options snapshot, and return nothing. The adapter's hooks never throw, so Ky's handling of a returned Request, Response, `ky.stop` or options, and of a throwing beforeRetry hook, would have nothing to act on. The afterResponse hooks receive the response itself rather than `response.clone()`; the adapter's hook reads only the status.
- **`#raceWithTotalTimeout` around a hook is its first step, the user-abort check.** `totalTimeout` is never set, and an abort inside a synchronous hook does not win the race (the hook's own promise settles first), so the race decides nothing else.
- **Body reads.** A successful body is read with `text()` and parsed with `JSON.parse`, without `#raceBodyRead`'s timeout race. The bodies are in memory, and the user signal cannot abort once a response is accepted. An error body is read whole (`arrayBuffer`) and decoded once rather than chunk by chunk, which gives the same text, and without the error-data timeout race. Its length is checked against the 10 MiB limit.
- **The retry wait is a plain timer.** Ky's `delay` also listens to the user signal, which the adapter never aborts during a wait.
- **Option merging** is the header part of `deepMerge`: the names are lowercased, and the last spelling of a name wins at the place of the first (a Dart map keeps insertion order). The `content-type` test of `mergeHeaders({}, options.headers).has(...)` looks at those merged names, which `Headers.set` has already validated.
- **Not ported, because the contract never sets them:** `totalTimeout`, `maxResponseSize`, `parseJson`, `stringifyJson`, `retry.jitter`, `retry.shouldRetry`, `ky.retry()` (ForceRetryError), `searchParams`, `prefix`, `baseUrl` and the progress callbacks. Also left out is what only releases resources (`cancelBody`) or only matters for options outside the contract (`findUnknownOptions`, `duplex`, `NonError.wrap`, and the referrer that `#withManagedSignal` copies).
- **Error messages** follow Ky's where Ky's own checks throw. They are not compared.

## Performance accommodations

None.
