> **Historical record.** This file describes the archived port, not a fresh qualification. Current restoration/build results and limitations are in `../../docs/COMPARATORS.md`.

# Melange port status

Package name: `fingerprintjs-melange-experiment`.
Toolchain: Melange `7.0.1-53` / OCaml 5.3.0 / dune 3.24.2 on the
`fingerprintjs-compare` switch.

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
- Scheduler is the upstream `mapWithBreaks` lifecycle only.
- Browser differential, 0 normalized differences:

      node ../fingerprintjs-compare/tools/run-differential.mjs --port melange
      chrome: 0
      firefox: 0
      safari: 0

- SSR-safe import:

      node ../fingerprintjs-compare/tools/check-ssr.mjs --port melange

## Emit

The packed ESM is **import-free**. No `melange.js/*` is pulled; runtime
bytes for this port are zero. See `NOTES.md` for the idioms that were
rejected because they would have imported the runtime.

## How to build

    eval "$(opam env --switch=fingerprintjs-compare --set-switch)"
    node tools/build.mjs

`tools/build.mjs` runs `dune build`, Rollups `_build/default/src/fp/src/fingerprint.js`
with the oracle Rollup, Terser-minifies with the oracle
`comments: false` / `safari10: true` options, prepends the MIT banner,
and copies the oracle `fp.d.ts`.

## Remaining gaps

None against the contract checks above. Size gates will fail: the
minified IIFE is a Melange/Terser artifact, not the handwritten oracle,
and the comparison treats that as a result rather than a build failure.
Parity is complete.
