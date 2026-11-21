# Ky 2.2.0: GopherJS port

This port is written by hand. It replaces the earlier port that `build/library-matrix/dev-ky/translate.py` generated from a shared core. It is a faithful port of Ky 2.2.0 (`distribution/index.js`, the subset in `libraries/ky/CONTRACT.md`) and of what `libraries/ky/reference.mjs` does with it, to Go 1.21 compiled by GopherJS 1.21. It follows Ky's own functions: `normalizeRetryOptions`, `validateTimeoutOption`, the constructor (`newKy`), `#run`, `#retry`, `#retryFromError`, `#fetch`, `#calculateRetryDelay`, `#calculateDelay`, `#getResponseData`, `#throwProcessedError`, `utils/timeout.js` and `core/retry-timing.js`. It embeds no JS.

| File | What it is |
| --- | --- |
| `src/ky.go` | Ky: its errors, options and their checks, header merging, the network-error test, the content-type and charset rules, and the `Ky` request with its run |
| `src/retrytiming.go` | `core/retry-timing.js`: the timing headers and their delays |
| `src/host.go` | Go types over Fetch, AbortController and AbortSignal, TextDecoder, timers and the JS globals Ky calls; `catch` and `await` |
| `src/main.go` | the reference adapter (each case's transport, hooks and trace) and `run`, published on `js.Module.exports` |

## Boundary

Each corpus case is a `*js.Object` read in place, field by field. The option values Ky checks stay `*js.Object`s (`js.Undefined` where omitted) until Ky's own checks read them through `Object.prototype.toString`, `Array.isArray` and `Number.isInteger`. Each result is the JS array `[trace, outcome]`. `run` returns a Promise that a goroutine settles.

## Interop decisions

- **Goroutines.** Ky's async methods are blocking functions on the goroutine `run` starts; `await` blocks it on a host promise through a channel. The transport is Ky's `fetch` option, a blocking function. A JS callback (a promise reaction, an abort listener, a timer) only sends on a buffered channel, so it never blocks.
- **Errors.** Ky's own errors are `*HTTPError`, `*NetworkError` and `*TimeoutError`. Everything else is the JS error that was thrown, a `*js.Error`: a host constructor's TypeError or RangeError, `JSON.parse`'s SyntaxError, an abort reason. A host call that throws is recovered by `catch`. Ky's option checks make real JS errors (`newError`). `errorName` is a type switch, and a `*js.Error` gives its JS `name`.
- **Loops.** `#retry` and `#retryFromError` call each other in Ky; here they are one loop (`retryRequest`, with `prepareRetry` as `#retryFromError` up to its final `#retry()`), as is natural for blocking Go. The order of every step is Ky's.
- **The timeout race** is Ky's `utils/timeout.js`: the transport runs on its own goroutine, and the first of it and the timer to finish decides, as the first settlement of Ky's promise does. The timer decides before it aborts the controller, since the abort can wake the transport's goroutine at once.
- **Timers** are the host's `setTimeout` with Ky's own millisecond value. GopherJS's `time` package adds a millisecond to every timer, which takes Ky's largest timeout (2147483647) past `setTimeout`'s 32-bit range, where Node runs it after 1 ms.
- **Regular expressions.** Ky's patterns are anchored and made of ASCII literals and `\d`. `scanner` matches the three HTTP-date forms piece by piece, and `charset` follows `/;\s*charset\s*=\s*(?:"([^"]+)"|([^;,\s]+))/i` with JS's whitespace set (`isJSSpace`) and an ASCII-only case match, as a non-Unicode pattern has. Go's `regexp` would read `\s` as ASCII only.
- **Dates.** `createTimestamp` counts days as ECMAScript's `MakeDay` does (`daysFromCivil`). It reproduces `Date.UTC`'s reading of a year from 0 to 99 as 1900 + year: the day has to exist in that year and in the year itself, so 29 Feb 0000 is rejected as Ky rejects it. Go's `time.Date` does not shift those years.
- **JS string semantics.** Go strings are UTF-8. Header names are lowercased with JS's `toLowerCase` (`jsLowerCase`): Go's `strings.ToLower` maps U+0130 to `i`, where JS gives two code units. Method names go through `strings.ToLower` and `ToUpper`, which agree with JS on every name that can match Ky's lists. Trimming uses `isJSSpace`: Go's `unicode.IsSpace` adds U+0085 and lacks U+FEFF. The character before `json` is ASCII whenever it matters, so a byte of UTF-8 decides as a code unit would.
- **Host functions.** `JSON.parse` and `JSON.stringify`, `Number()`, `Number.isInteger`, `Object.keys`, `Reflect.has`, `Error.prototype.isPrototypeOf` and `Object.prototype.toString` (Ky's `isError`, which accepts an Error of another realm by its brand, as the Fetch constructors throw under Firefox's WebDriver), `Math.pow`; Go's `math.Max`, `math.Min` and `time.Now`.

## Deviations from the npm source

None changes a result in the parity domain.

- **One call is the whole `ky(input, options)[read]()`.** `send` runs the init hooks, builds the request, sets the shortcut's Accept header, runs the request and reads the body. Ky starts its run one microtask after the shortcut call; every step happens in the same order.
- **Hooks.** They receive what the adapter's hooks read (the request, response, error and retry count), not Ky's normalized options snapshot, and return nothing. The adapter's hooks never throw, so Ky's handling of a returned Request, Response, `ky.stop` or options, and of a throwing beforeRetry hook, would have nothing to act on. The afterResponse hooks receive the response itself rather than `response.clone()`; the adapter's hook reads only the status.
- **`#raceWithTotalTimeout` around a hook is its first step, the user-abort check.** `totalTimeout` is never set, and an abort inside a synchronous hook does not win the race (the hook's own promise settles first), so the race decides nothing else.
- **Body reads.** A successful body is read with `text()` and parsed with `JSON.parse`, without `#raceBodyRead`'s timeout race. The bodies are in memory, and the user signal cannot abort once a response is accepted. An error body is read whole (`arrayBuffer`) and decoded once rather than chunk by chunk, which gives the same text, and without the error-data timeout race. Its length is checked against the 10 MiB limit.
- **The retry wait is a plain timer.** Ky's `delay` also listens to the user signal, which the adapter never aborts during a wait.
- **Option merging** is the header part of `deepMerge`: the names are lowercased, and the last spelling of a name wins at the place of the first. The `content-type` test of `mergeHeaders({}, options.headers).has(...)` looks at those merged names, which `Headers.set` has already validated.
- **Not ported, because the contract never sets them:** `totalTimeout`, `maxResponseSize`, `parseJson`, `stringifyJson`, `retry.jitter`, `retry.shouldRetry`, `ky.retry()` (ForceRetryError), `searchParams`, `prefix`, `baseUrl` and the progress callbacks. Also left out is what only releases resources (`cancelBody`) or only matters for options outside the contract (`findUnknownOptions`, `duplex`, `NonError.wrap`, and the referrer that `#withManagedSignal` copies).
- **Error messages** follow Ky's where Ky's own checks throw. They are not compared.

## GopherJS quirk

An omitted option's `*js.Object`, stored in a struct inside a slice literal, no longer compared equal to `js.Undefined`, so an omitted `maxRetryAfter` or `backoffLimit` failed its check. `normalizeRetryOptions` passes each value to `limitOption` as a function argument instead, which keeps the comparison.

## Performance accommodations

None.
