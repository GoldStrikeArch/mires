> **Historical record.** This file describes the archived port, not a fresh qualification. Current restoration/build results and limitations are in `../../docs/COMPARATORS.md`.

# STATUS — fingerprintjs-jsoo

Idiomatic OCaml / js_of_ocaml 6.4.1 port of FingerprintJS 5.2.0.

## What works

- All 42 sources, in contract order, transliterated from the TypeScript oracle.
- All 25 named exports plus `default = { load, hashComponents, componentsToDebugString }` with identity equality.
- `sources` function names and arities match the oracle.
- Murmur x64 128 matches the oracle on ASCII, UTF-8 emoji, and unpaired-surrogate vectors (including seed 17).
- `hashComponents` / `componentsToDebugString` match, including Error own-properties.
- `loadSources` / `transformSource`: `this === null`, sync/async/two-stage, exclude list, no unhandled rejections.
- `mapWithBreaks` is the upstream 16 ms `Date.now()` budget + `MessageChannel` yield (Lwt loop, not a fiber scheduler).
- `LoadOptions.monitoring` is a no-op (no XHR / fetch / beacon).
- SSR-safe ESM import (poisoned `window` / `document` / `navigator` / `screen` / `location` / `self`).
- `fp.d.ts` is the oracle file (SHA-256 matches `frozen.json`).
- Differential: **0 normalized differences** on Chrome, Firefox, and Safari.

## Remaining gaps

- None required for the contract checks that were run.
- Size gates will fail; that is expected. The IIFE gzip-9 number is 48 307 bytes.
- `js_of_ocaml-lwt` was not needed after binding `setTimeout` / `MessageChannel` directly; Lwt is still used for `mapWithBreaks` and `withIframe`.

## How to build

```sh
eval "$(opam env --switch=fingerprintjs-compare --set-switch)"
node tools/build.mjs
```

Produces `dist/fp.esm.js`, `dist/fp.esm.min.js`, `dist/fp.min.js`, `dist/fp.d.ts`.

## Runtime vs program

| | raw bytes |
| --- | ---: |
| Whole-program JS emit | 138894 |
| Program only (`--no-runtime`) | 102409 |
| **DCE'd JSOO runtime (tax)** | **36485** |
| Full unused JSOO runtime | 469772 |
| Shipped minified IIFE | 136140 (gzip-9 48307) |

See `NOTES.md` for gzip/brotli and exact package versions.
