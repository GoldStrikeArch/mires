> **Historical record.** This file describes the archived port, not a fresh qualification. Current restoration/build results and limitations are in `../../docs/COMPARATORS.md`.

# Dart2js port status

Package name: `fingerprintjs-dart-experiment`.
Toolchain: Dart SDK 3.13.0 (`/opt/homebrew/bin/dart compile js`).

## What works

- Required artifacts under `dist/`: `fp.esm.js`, `fp.esm.min.js`,
  `fp.min.js`, `fp.d.ts` (oracle copy).
- `fp.d.ts` SHA-256
  `d20e2e6b3f60afb7679c1f87dd62800b7a7ce7155e86efbf6206190a1c185731`.
- 25 named exports in namespace order, default export
  `{ load, hashComponents, componentsToDebugString }` with function
  identity, 42 sources in collection order, oracle function `.name` /
  `.length`.
- Murmur x64 vectors, `hashComponents` canonicalization, custom
  `loadSources` / `transformSource` lifecycle (`this === null` on load,
  two-stage sources, excluded keys, unhandled-rejection suppression).
- `LoadOptions.monitoring` accepted and ignored; no XHR / fetch / beacon.
- Scheduler is the upstream `mapWithBreaks` lifecycle only (16 ms
  `Date.now()` budget, `MessageChannel` yield, non-awaited callback
  starts, final `Promise.all`).
- Browser differential, 0 normalized differences:

      node ../fingerprintjs-compare/tools/run-differential.mjs --port dart
      chrome: 0
      firefox: 0
      safari: 0

- SSR-safe import:

      node ../fingerprintjs-compare/tools/check-ssr.mjs --port dart

- Export namespace:

      node ../fingerprintjs-compare/tools/check-exports.mjs --port dart

## Emit

The dart2js runtime is **not separable** from the program. See `NOTES.md`.
Shipped JS is `-O1` (readable) plus a tiny named-export facade; `-O2`
bytes are recorded as a footnote only.

## How to build

    /opt/homebrew/bin/dart pub get
    node tools/build.mjs

`tools/build.mjs` runs `dart compile js -O1` and `-O2`, wraps the `-O1`
script so dart2js does not touch poisoned `window`/`document`/`self` at
import time, assigns the 25 ESM names, Terser-minifies with the oracle
`comments: false` / `safari10: true` options, prepends the MIT banner,
and copies the oracle `fp.d.ts`.

## Remaining gaps

None against the contract checks above. Size gates will fail: the
minified IIFE is a dart2js/Terser artifact, not the handwritten oracle,
and the comparison treats that as a result rather than a build failure.
Parity is complete.

## Production-profile follow-up (2026-08-15)

The build now publishes two complete variants:

- `dist/o1/`: readable-first `dart compile js -O1`, plus the common facade and Terser packaging;
- `dist/o2/`: safe production `dart compile js -O2`, plus the same facade and Terser packaging.

Root `dist/fp.*` remains an alias of `o1` for historical compatibility. The comparison registry uses `dart-o2` as the production-default row.

`dart-o2` passes exports, SSR import, and a fresh one-repetition Chrome / Firefox / Safari differential with zero normalized differences. Its minified IIFE is 151,168 B, gzip-9 52,147 B, and Brotli-11 44,842 B.
