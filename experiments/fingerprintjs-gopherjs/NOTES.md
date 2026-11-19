# GopherJS port notes

Idiomatic Go 1.21 port of FingerprintJS 5.2.0, compiled to JavaScript by GopherJS 1.21.0. The runtime and standard-library tax is the measurement; nothing was hand-tuned for size or speed.

## Toolchain and build

| Item | Value |
| --- | --- |
| Go | 1.21.13 (`.toolchains/go/go1.21.13`; GopherJS 1.21 needs a Go 1.21 distribution) |
| GopherJS | 1.21.0 (`.toolchains/go/bin/gopherjs`, reports `GopherJS 1.21.0+go1.21.13`) |
| Module | `fingerprintjs-gopherjs`, requires `github.com/gopherjs/gopherjs v1.21.0` (module cache `.toolchains/go/gopath`) |
| Terser | 5.31.6 from the repository root `node_modules`, oracle options |

`bash experiments/tools/setup-go-comparators.sh` installs the toolchain. `node tools/build.mjs` (or `npm run build`) then:

1. sets `GOROOT`, `GOPHERJS_GOROOT`, `GOPATH`, `GOCACHE` (`<MIRES_GO_ROOT>/cache`), `GOTOOLCHAIN=local` and `GOFLAGS=-modcacherw` from `MIRES_GO_ROOT` (default `../../.toolchains/go`) and checks the GopherJS version;
2. runs `gopherjs build --source_map=false ./src` (readable) and `gopherjs build -m --source_map=false ./src` (compact) into `build/gopherjs/`;
3. wraps the readable emit in the ESM facade with the MIT banner → `dist/fp.esm.js`;
4. wraps the compact emit in the ESM / IIFE facade, runs Terser (`format.comments: false`, `safari10: true`, `module` for ESM) → `dist/fp.esm.min.js`, `dist/fp.min.js`;
5. copies the oracle `fp.d.ts`, builds the runtime floor (`tools/runtime-floor`) the same way, and writes `dist/raw/attribution.json`, `dist/raw/program.js` (the compact emit) and `dist/variant.json`.

GopherJS has no optimisation levels beyond `-m`, so the root `dist/` is the production row; there is one variant. The Go build cache is `<MIRES_GO_ROOT>/cache`. GopherJS's own package cache is not controlled by `GOCACHE`: it is `os.UserCacheDir()/gopherjs/build_cache` (`~/Library/Caches/gopherjs` on macOS). Nothing is generated in the port outside `build/` and `dist/`.

## Program shape

`src/` is one `package main` (3,644 lines, 2,783 without blank and comment lines), one file per oracle module: `agent.go`, `confidence.go`, `async.go`, `entropy.go`, `hashing.go`, `data.go`, `misc.go`, `dom.go`, `browser.go`, `sources.go` (registry), `audio.go`, `canvas.go`, `webgl.go`, `fonts.go`, `font_preferences.go`, `dom_blockers.go`, `screen_frame.go`, `user_agent_data.go`, `math_source.go`, `sources_basic.go` (the small sources), `main.go` (exports) and `host.go` (interop helpers).

- **Values.** Values read from the browser stay `*js.Object` and are returned as they are, so components keep the host's types (a string `screen.width` stays a string). Go types are used for program state: `[]string`, maps, a `[4]float64` screen frame, the filter table.
- **Hashing.** MurmurHash3 x64 128 is written with `uint64` and `math/bits.RotateLeft64`, as idiomatic Go would; GopherJS emulates `uint64`. The canonical string is built with `strings.Builder` as UTF-8 and hashed as `[]byte`.
- **Async.** The observable lifecycle is JavaScript promises, `MessageChannel` and timers, never goroutines or channels: a Go function called from JavaScript cannot block, and GopherJS's scheduler yields through its own `setTimeout` queue. An oracle `async function` is ported as `async(func(resolve, reject))`, which runs synchronously like the start of an async function, and each `await x` as `await(x, reject, continuation)` = `Promise.resolve(x).then(continuation, reject)`, which continues in the same microtask as the oracle's `await`. `mapWithBreaks` keeps the 16 ms `Date.now()` budget, the `MessageChannel` yield, the synchronous callbacks before the first break and `Promise.all` ordering; source loads and gets start without being awaited.
- **Exceptions.** `tryJS` recovers the `*js.Error` panic GopherJS makes of a JavaScript exception and returns the original thrown value; `throw` rethrows a raw value through the prelude's `$throw`, so errors keep their identity, constructor and own properties. `window()`, `document()`, `navigator()` and `screen()` throw a `ReferenceError` for a missing global, as the oracle's bare identifiers do.
- **Exports.** `main` publishes the API on `js.Module.exports`. Every exported function, the 42 source functions, the agent's `get` and the `visitorId` accessor are `js.MakeFunc` functions whose `name` and `length` are redefined with `Object.defineProperty` to the oracle's values (`murmurX64Hash128.name` is `x64hash128`). `default` holds the same three function objects as the named exports.
- **Monitoring and debug** follow the oracle: `window.__fpjs_d_m`, `Math.random() < 0.001`, an asynchronous `XMLHttpRequest` `GET https://m1.openfpcdn.io/fingerprintjs/v5.2.0/npm-monitoring`, `console.error` on failure, `monitoring: false` disables it; `debug` (load) or `get({ debug })` prints the oracle's message and DOM-blockers report.

## Embedded JavaScript

None. There is no `eval`, no `Function` constructor and no product `.js` source. Where Go cannot express a JavaScript operator, the port calls a host built-in or a function of the GopherJS prelude through the native `js.Global.Call("$name", …)` mechanism that `js.MakeFunc` and GopherJS's own standard-library natives use:

| Need | Mechanism |
| --- | --- |
| `typeof` (thenable predicate, two-stage sources, `getParameter` check) | prelude `$typeOf` |
| `instanceof` (`Error`, `DOMException`) | prelude `$instanceOf` |
| Go value → `*js.Object` | prelude `$id` (identity, through externalisation) |
| rethrow a JavaScript value unchanged | prelude `$throw` |
| `in` operator | `Reflect.has` |
| named functions with an arity | `js.MakeFunc` + `Object.defineProperty(fn, 'name' / 'length', …)` |
| template-literal ToString | `String.prototype.concat.call('', value)` |
| ToNumber, `parseInt`, `parseFloat`, `isFinite`, `String()` | the host globals |
| `Math.round` (JavaScript rounding) and the fingerprinted `Math` functions | the host `Math` |
| the oracle's regular expressions (CSS selector and style parser, native-function test, `/not/i`, `/android/i`, the Apple Pay message test) | the host `RegExp` with the oracle's patterns |
| sorts of JavaScript-observable arrays (UTF-16 order, numbers sorted as strings) | `Array.prototype.sort` |
| base64 filter selectors, UTF-8 of non-ASCII strings | `atob`, `TextEncoder` |

The ESM and IIFE facades are generated by `tools/build.mjs` and are not product code. Each evaluates the GopherJS emit inside `(function (window, module) { … }).call(globalThis, globalThis, exportsModule)`: the prelude picks its global object from `window` / `self` / `global` / `this` and `js.Module` from `module`, so binding `window` to `globalThis` (the same object on a web page) keeps the import from reading the browser `window` getter or taking the Node.js `require` branch, and `module` receives the exports. The facade then re-exports the 25 names (IIFE: a `FingerprintJS` namespace in the oracle's key order, with `__esModule`).

## Deviations and GopherJS findings

1. **Import side effects of the GopherJS prelude.** Importing either facade (in Node.js or a browser) defines `globalThis.fs`, the prelude's console-backed file-system shim for `os` writes; sets `Error.stackTraceLimit = Infinity`, which GopherJS's `recover()` needs because it compares stack depths read from `new Error().stack`; and queues and clears one `setTimeout` while `main` runs. It prints nothing, throws nothing and touches no network, so the SSR check passes. The shim is built with `new TextDecoder('utf-8')`, so the import needs a global `TextDecoder`: in a bare JavaScript host without it (a `vm` context with only `console` and timers) `fp.min.js` throws `ReferenceError: TextDecoder is not defined`, where the oracle imports. Every browser that runs the oracle's build has `TextDecoder`. When the page has a global `require`, the untouched prelude also calls `require('fs')` and `require('util')` (the facade makes `$global` the page's `globalThis`) and installs whatever `require('fs')` returns as `globalThis.fs`: on a RequireJS page the call throws and is caught, and in Electron with node integration the real `fs` module is installed. The verifier's reproduction is `node build/comparators/gopherjs-verify/probes/global-require.mjs port`; the oracle makes no such calls. None of this affects the fingerprint. The oracle's import has no such effects. They are GopherJS runtime behaviour, kept on purpose: the build ships the GopherJS emit untouched, with only the packaging needed to load it, so that the size and behaviour rows measure the compiler's own output.
2. **`math.Trunc` is wrong beyond ±2³¹.** GopherJS implements it as `Copysign(float64(int(x)), x)` with a 32-bit `int`, so `math.Trunc(4294967295)` is `1`. The first build failed 12 of the 58 kernel comparisons (seed `0xffffffff`) and the seed-boundary comparisons for 23 of the 53 seeds on this. The seed's ToUint32 now takes the integer part from `math.Modf`, which GopherJS implements correctly.
3. **Unpaired surrogates.** GopherJS's JavaScript → Go string conversion (`$internalize`) combines a high surrogate with whatever code unit follows it, without checking for a low surrogate, and skips both: the following unit is eaten (`"\ud800 x"` becomes U+2420 + `x`, the space lost; `"\ud800a"` becomes U+2461; `"\ud800\ud800"` becomes U+FC00), and a high surrogate at the end of a string produces the invalid UTF-8 `f0 80 80 80`. A lone low surrogate correctly becomes U+FFFD, and valid pairs are exact. TextEncoder writes U+FFFD for every unpaired surrogate. Every string that reaches the hash (the murmur input, component keys, JSON values) is converted with `utf8String`, which falls back to `TextEncoder` for any non-ASCII string, and component keys are read through `Reflect.get` with the original key value. Strings read from browser APIs (WebGL parameters, the user agent, the debug message) use the ordinary conversion, exact for well-formed UTF-16.
4. **A thrown `null`.** GopherJS uses `throw null` internally to unwind after a recovered panic, and a JavaScript `throw null` that reaches a Go function with a `defer` runs the deferred calls without a panic. The first build therefore hung `getComponents` when a source threw `null`. `tryJS` now treats an uncompleted call without a panic as a thrown `null`.
5. **Cross-realm `Array`.** GopherJS's slice constructor compares `array.constructor` with the global `Array` binding and wraps any other array, so the program only runs in a realm whose global `Array` is that realm's own; a page has one realm. `check-feature-parity-static.mjs` used to evaluate `fp.min.js` in a `vm` context into which it injected the host realm's intrinsics, and the IIFE failed there with `TypeError: Cannot read properties of undefined (reading 'length')`. The shared tool now gives the context its own intrinsics and only adds missing host APIs; its gopherjs row is contract API PASS, strict API PASS, 0 metadata differences, IIFE namespace exact (formats MISSING only for the optional CJS / UMD builds). The facade never bound `Array` to work around the old sandbox: that would have let the probe read the namespace while calls (`js.MakeFunc` slices `arguments` with `$global.Array`) still received foreign arrays.
6. **Error messages.** Misuse errors have the oracle's error type but engine-generated messages differ: `hashComponents({ a: 5 })` throws `TypeError: Reflect.has called on non-object` instead of `Cannot use 'in' operator…`, `load(null)` throws `Cannot read properties of null` instead of `Cannot destructure…`. Globals other than `window`, `document`, `navigator` and `screen` are read as properties of `globalThis`, so outside a browser a missing one (`XMLHttpRequest`, `MessageChannel`, …) fails with a `TypeError` where the oracle has a `ReferenceError`, and the message names GopherJS's internal global: with no `XMLHttpRequest`, monitoring logs `TypeError: $global.XMLHttpRequest is not a constructor` where the oracle logs `ReferenceError: XMLHttpRequest is not defined`.
7. **Inputs outside the typed contract.** A non-number `loopReleaseInterval` passed to `loadSources` is coerced with `Number()`; the oracle adds it to a number with `+`, which concatenates a string, so `"5"` never yields there (only scheduling changes). A BigInt seed is converted with `Number()`: `murmurX64Hash128('a', 5n)` returns a hash where the oracle throws `TypeError: Cannot mix BigInt and other types`. Both parameters are typed `number`.
8. **`isSourceLoaded`.** The oracle's test-only `cpuClass` loading flag is not ported; it is not exported.
9. **Promise class.** The oracle's `async` functions (`load`, `agent.get`, `getComponents`, `withIframe`, four sources, the screen-frame getter) return intrinsic promises and `await` never looks up `.then`. Go has no `async`, so the port builds them on `globalThis.Promise` at call time. On a page where Zone.js or a polyfill replaces `Promise`, those results are instances of the replacement and their microtasks go through it; the values are the same. The functions themselves are `js.MakeFunc` functions, not async functions: the oracle's async exports and sources (`load`, `withIframe`, the `getComponents` that `loadSources` returns, `agent.get`, `userAgentData`, `domBlockers`, `fonts`, `fontPreferences`) have no `prototype` and throw `TypeError: … is not a constructor` under `new`, and their prototype is `AsyncFunction.prototype`; the port's have a `prototype`, `new load()` runs `load` and returns its promise, and their prototype is `Function.prototype`. Names and lengths match.

An independent line-by-line review against the oracle, with Node.js probes, found further edge cases that the browser contracts do not reach; all are fixed and reprobed: a thenable reporting more than one outcome now keeps the first, as `resolveLoad` does; the audio hash sums any array-like signal (a `[]float32` assertion panicked on anything but a same-realm `Float32Array`) and a non-`Uint8Array` `TextEncoder` result is copied; a non-string `murmurX64Hash128` input gets the oracle's `charCodeAt` walk; the screen-frame backup is the array returned to the caller, as in the oracle; font sizes are compared as JavaScript values (`undefined === undefined`); the iframe is removed exactly once whichever way `withIframe` settles, and an exception from that removal rejects it, as from the oracle's `finally` (an independent verification found that a successful action whose removal threw removed a second time); the user-agent-data `try` also covers the high-entropy property reads; `isIPad` and `prepareForSources` coerce like the oracle.

## Runtime attribution

`dist/raw/attribution.json` splits the `gopherjs build -m` emit (before the facade and Terser) at its `$packages["…"]` blocks. The runtime floor is `tools/runtime-floor`, a one-export program (`load` returning 1), built and packaged exactly like the port.

| Part of the 243,719-byte compact emit | bytes |
| --- | ---: |
| GopherJS prelude (runtime, type system, goroutines, JS mapping) | 41,828 |
| `runtime` + `github.com/gopherjs/gopherjs/js` natives | 9,410 |
| `strings` and what it links: `internal/abi`, `internal/reflectlite`, `errors`, `sync`, `sync/atomic`, `io`, `unicode`, `unicode/utf8`, `internal/bytealg`, `internal/cpu`, `internal/race`, `internal/goarch` | 61,840 |
| `math`, `math/bits` | 3,829 |
| the FingerprintJS program (`fingerprintjs-gopherjs/src`) | 126,570 |
| start-up | 242 |

| Runtime floor (one-export program) | raw | gzip-9 | brotli-11 |
| --- | ---: | ---: | ---: |
| compact emit | 51,990 | | |
| `fp.min.js` shape (IIFE facade + Terser) | 42,619 | 14,111 | 12,707 |
| `fp.esm.min.js` shape (ESM facade + Terser) | 42,835 | 14,086 | 12,731 |

`strings` costs about 62 KB of emit for `Builder`, `HasPrefix`, `Join`, `Split` and `ReplaceAll`, mostly `internal/abi` and `internal/reflectlite`: `errors` initialises `reflectlite.TypeOf((*error)(nil))` at package level, and GopherJS keeps package-level initialisers. Measured on a one-function program, other natural choices would cost more (`gopherjs build -m` emit over the 51.9 KB floor): `regexp` +530 KB (`MustCompile` + `MatchString`), `strconv.FormatFloat` +168 KB, `sort.Strings` +61 KB, `strings.ToLower` about +36 KB of case tables over `strings` alone. The port uses the host `RegExp`, the host number-to-string conversion and the host sort instead, which also keep JavaScript semantics.

## Shipped artifacts

| File | raw | gzip-9 | brotli-11 |
| --- | ---: | ---: | ---: |
| `dist/fp.esm.js` (readable emit + facade + banner) | 436,277 | 83,063 | 66,638 |
| `dist/fp.esm.min.js` | 201,379 | 57,352 | 47,866 |
| `dist/fp.min.js` (IIFE) | 201,163 | 57,372 | 47,821 |
| `dist/fp.d.ts` | oracle copy, SHA-256 `d20e2e6b3f60afb7679c1f87dd62800b7a7ce7155e86efbf6206190a1c185731` | | |

Gzip uses level 9 with `mtime: 0` and Brotli quality 11 (Node zlib), each file compressed separately, as in `experiments/tools/measure-comparators.mjs`.

## 32-bit-lane Murmur (separate measurement, not shipped)

A scratch variant replaced the `uint64` Murmur with the oracle's representation, a struct of two `uint32` lanes and 16-bit multiply lanes (`math/bits.Mul32` itself goes through `uint64` in GopherJS). It passed all 58 kernel and 477 seed-boundary comparisons, was about 2 KB larger minified, and was about 5 times slower than the shipped `uint64` code in Node.js 26 (murmur over a 12 KB ASCII string: 2.3 ms versus 0.43 ms; the oracle takes 0.11 ms), because GopherJS allocates and copies struct values on every lane operation. The shipped port keeps the idiomatic `uint64`.

## Verification

```sh
python3 experiments/tools/run-comparator-builds.py --languages gopherjs --suite oss
python3 experiments/tools/check-comparators.py --languages gopherjs --suite oss
node experiments/fingerprintjs-compare/tools/check-ssr.mjs --port gopherjs
node experiments/fingerprintjs-compare/tools/run-differential.mjs --port gopherjs --repetitions 1
node experiments/fingerprintjs-compare/tools/run-feature-parity.mjs --port gopherjs --browsers chrome,firefox
node experiments/fingerprintjs-compare/tools/check-feature-parity-static.mjs --output build/comparators/gopherjs-feature-parity-static.json
```

Results are in `STATUS.md`.
