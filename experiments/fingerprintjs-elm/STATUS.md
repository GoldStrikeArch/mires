> **Historical record.** This file describes the archived port, not a fresh qualification. Current restoration/build results and limitations are in `../../docs/COMPARATORS.md`.

# STATUS — fingerprintjs-elm

Elm 0.19.2 + JS host port of FingerprintJS 5.2.0. Package name
`fingerprintjs-elm-experiment`.

## What works

- All 42 sources, in contract order. Elm owns murmur, canonical join,
  confidence, the 42-key list, and the math source (polyfills + result shape).
  The JS host owns browser probes, `mapWithBreaks`, and the 25-export facade.
- All 25 named exports plus `default = { load, hashComponents, componentsToDebugString }`
  with identity equality.
- `sources` function names and arities match the oracle.
- Murmur x64 128 matches the oracle on ASCII, UTF-8 emoji, long text, and
  unpaired-surrogate vectors (including seed 17).
- `hashComponents` / `componentsToDebugString` match, including Error own-properties.
- `loadSources` / `transformSource`: `this === null`, sync/async/two-stage,
  exclude list, no unhandled rejections.
- `mapWithBreaks` is the upstream 16 ms `Date.now()` budget + `MessageChannel`
  yield + `Promise.all` ordering.
- `LoadOptions.monitoring` is a no-op (no XHR / fetch / beacon).
- SSR-safe ESM import (poisoned `window` / `document` / `navigator` / `screen`
  / `location` / `self`).
- `fp.d.ts` is the oracle file (SHA-256 matches `frozen.json`).
- Differential: **0 normalized differences** on Chrome, Firefox, and Safari.

## mostly JS

Yes. `dist/attribution.json` records
`{ elmProgram: 38033, elmRuntime: 60935, jsHost: 63352, facade: 1370 }`.
`jsHost` is 109% of minified IIFE `fp.min.js` (57 980). The row is **mostly JS**.

## How to build

```sh
node tools/build.mjs
```

Requires `/usr/local/bin/elm` 0.19.2 and the oracle `node_modules` (Rollup +
Terser). Produces `dist/fp.esm.js`, `dist/fp.esm.min.js`, `dist/fp.min.js`,
`dist/fp.d.ts`, `dist/attribution.json`.

```sh
node tools/test-hash.mjs
node ../fingerprintjs-compare/tools/check-exports.mjs --port elm
node ../fingerprintjs-compare/tools/check-ssr.mjs --port elm
node ../fingerprintjs-compare/tools/run-differential.mjs --port elm
```

## Remaining gaps

- None required for the contract checks that were run.
- Size gates will fail; that is expected. Minified IIFE gzip-9 is 22 992 bytes.
- Outgoing Elm ports are patched at wrap time (`_Process_sleep(0)` →
  `_Scheduler_succeed(null)`) so `hashComponents` stays synchronous. Documented
  in `NOTES.md`.

## Production-profile follow-up (2026-08-15)

The build now publishes:

- `dist/standard/`: `elm make --optimize` followed by ordinary Terser;
- `dist/production/`: Elm's compiler-aware pure-helper / unsafe compression applied only to generated Elm, followed by ordinary Rollup/Terser packaging of the JS host.

Root `dist/fp.*` remains an alias of `standard`. `elm-production` passes exports, SSR import, and a fresh one-repetition Chrome / Firefox / Safari differential with zero normalized differences. Its minified IIFE is 53,949 B, gzip-9 21,964 B, and Brotli-11 19,616 B.
