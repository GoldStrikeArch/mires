# STATUS — fingerprintjs-gopherjs

Idiomatic Go 1.21 port of FingerprintJS 5.2.0, compiled by GopherJS 1.21.0. Results below are from the build of 2026-11-18; `NOTES.md` has the design, the runtime attribution and the GopherJS findings.

## What works

- All 42 sources in contract order, ported from the TypeScript oracle; the 25 named exports in order; `default = { load, hashComponents, componentsToDebugString }` with the same function objects.
- Every exported function, source function, `agent.get` and the `visitorId` accessor has the oracle's `name` and `length`.
- MurmurHash3 x64 128 with Go `uint64`; the oracle's TextEncoder UTF-8 for lone surrogates, BOM, CRLF and astral characters; JavaScript ToUint32 seeds.
- `hashComponents` and `componentsToDebugString`, including Error own properties and `{ error: null }` components.
- `loadSources` / `transformSource`: `this === null`, synchronous, asynchronous and two-stage sources, thrown and rejected values (including `null`), thenables that report twice, exclusion, no unhandled rejections.
- `mapWithBreaks`: 16 ms `Date.now()` budget, `MessageChannel` yield, non-awaited starts, `Promise.all` ordering, all on JavaScript promises.
- `LoadOptions.monitoring` request rule, `debug` logging, `withIframe` styling, rejection and cleanup, the WebGL context cache.
- No embedded JavaScript: no `eval`, no `Function` constructor, no product `.js`.
- SSR-safe import; `fp.d.ts` is the oracle file.
- A Node.js edge-case comparison of the pure API against the oracle (32 cases: non-function sources, interval 0, `__proto__` keys, thenables, `undefined` values, nested Errors in the debug string, lone surrogates, non-string hash input, `null` options) differs only in two engine-worded `TypeError` messages.

## Results

| Check | Result |
| --- | --- |
| `run-comparator-builds.py --languages gopherjs --suite oss` | `oss-gopherjs PASS`, source inventory unchanged |
| `check-comparators.py --languages gopherjs --suite oss` | package 3/3, kernels 58/58, seed boundaries 477/477 |
| `check-ssr.mjs --port gopherjs` | ESM import is SSR-safe |
| `run-differential.mjs --port gopherjs --repetitions 1` | Chrome 158: 0 normalized differences; Firefox: 0; Safari: not run (see below); the minified ESM also gives 0 in Chrome and Firefox |
| `run-feature-parity.mjs --port gopherjs` | Chrome: 0 extended differences; Firefox: 0 |
| `check-feature-parity-static.mjs` | contract API PASS, strict API PASS, 0 metadata differences, IIFE namespace exact; formats MISSING (no CJS / UMD, like every port) |

Safari could not be driven: `SessionNotCreatedError: You must enable 'Allow remote automation' in the Developer section of Safari Settings to control Safari via WebDriver.` To cover the WebKit-only code, the differential contract and the feature-parity contract were also run in Chrome with the WebKit signals spoofed for both the oracle and the port (DOM blockers over every selector, with and without a stylesheet that hides three filter lists, the Apple Pay `InvalidAccessError` mapping, the base-latency source), and with the Android signals spoofed: 0 differences in each. These were scratch harnesses, not a Safari result.

## Sizes

| File | raw | gzip-9 | brotli-11 |
| --- | ---: | ---: | ---: |
| `dist/fp.min.js` | 201,163 | 57,372 | 47,821 |
| `dist/fp.esm.min.js` | 201,379 | 57,352 | 47,866 |
| runtime floor, `fp.min.js` shape | 42,619 | 14,111 | 12,707 |

## Remaining gaps

- Safari differential not run on this machine.
- The GopherJS prelude's import side effects (`globalThis.fs`, `Error.stackTraceLimit = Infinity`) remain; see `NOTES.md`.
- No CJS / UMD builds (optional for the comparators).

## How to build

```sh
bash experiments/tools/setup-go-comparators.sh   # once, from the repository root
cd experiments/fingerprintjs-gopherjs
node tools/build.mjs
```
