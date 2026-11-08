# Dart2js port notes

FingerprintJS 5.2.0, compiled by Dart SDK 3.13.0 (`/opt/homebrew/bin/dart compile js`).
Idiomatic Dart 3 with `dart:js_interop`, `dart:js_interop_unsafe`, and `package:web`.

## Compilation

- Readable emit: `dart compile js -O1 --no-source-maps`
- Footnote emit: `dart compile js -O2 --no-source-maps` (not shipped)
- dart2js cannot emit named ESM exports. `tools/build.mjs` wraps the
  whole-program script in a tiny facade that only assigns the 25 contract
  names (plus `default` identity). The facade is not a second FingerprintJS.
- SSR wrap: the dart2js prelude reads `typeof navigator` / `typeof self` /
  `typeof document` at init. The wrapper binds those free identifiers to
  `undefined` (and `self` to `globalThis`) so import-time poison getters
  do not fire. Browser globals are still reached via `globalThis.window`
  from exported functions.

```sh
node tools/build.mjs
```

Terser comes from `experiments/fingerprintjs/node_modules`
(`comments: false`, `safari10: true`).

## -O1 vs -O2

Sizes are the dart2js program *before* the ESM/IIFE facade, measured with
the rust `tools/common.mjs` gzip-9 / brotli-11 method.

| Emit | raw | gzip-9 | brotli-11 |
| --- | ---: | ---: | ---: |
| `-O1` (`dist/raw/program.o1.js`) | 433186 | 83505 | 67478 |
| `-O2` (`dist/raw/program.o2.js`) | 164977 | 53847 | 46061 |

Shipped artifacts are the `-O1` emit plus facade + oracle Terser. `-O2`
minifies identifiers inside dart2js; Terser of `-O1` is what the size
table should weigh so the readable column stays first-class.

## Runtime vs program bytes

dart2js does **not** have a separable runtime the way JSOO `--no-runtime`
does. The isolate prelude, type universe, and helpers are mixed into the
whole-program script. There is no honest `program-only.js`.

A two-line `main` that only writes `globalThis.__hello` is a lower bound
on the prelude, not a DCE'd leftover that can be subtracted from the
product:

| Hello-world dart2js | raw |
| --- | ---: |
| `-O1` | 11653 |
| `-O2` | 4894 |

Count the dart2js runtime as **not separable**. The whole artifact is the
runtime-inclusive program.

## Shipped artifacts

| File | raw | gzip-9 | brotli-11 |
| --- | ---: | ---: | ---: |
| `dist/fp.esm.js` (`-O1` + facade) | 435032 | 83985 | 67802 |
| `dist/fp.esm.min.js` | 252175 | 64818 | 53642 |
| `dist/fp.min.js` (IIFE, gated number) | 251899 | 64816 | 53633 |
| `dist/fp.d.ts` | oracle copy, SHA-256 `d20e2e6b3f60afb7679c1f87dd62800b7a7ce7155e86efbf6206190a1c185731` |

Size gates will fail. That is the result: Dart 3 + dart2js runtime is
larger than handwritten TypeScript.

## Hashing

Murmur x64 is pure Dart with 32-bit lanes (`>>>` / `& 0xffffffff`),
matching the TS schoolbook 16-bit arithmetic. UTF-8 is the oracle
`charCodeAt` ASCII fast path, then TextEncoder semantics (unpaired
surrogates → U+FFFD).

## Interop

Browser probes and the `loadSource` / `transformSource` / `mapWithBreaks`
Promise/`MessageChannel` lifecycle are Dart control flow calling host
functions through `dart:js_interop`. A small JS helpers object (created
lazily, no browser globals) exists for things dart2js calling convention
gets wrong if done naively: named exports with correct `.name`/`.length`,
`fn.apply(null, …)`, `instanceof Error`, typed-array hashing, and
`new C(…)`.

`.toJS` closures are arity-sensitive (`call$N`). Public functions are
wrapped with a 4-argument impl so JS can call them with 0–3 args.

## Remaining gaps

None against the contract checks in `STATUS.md`. Size gates fail by
design of the comparison.

## Explicit comparison variants (2026-08-15)

`tools/build.mjs` now packages both compiler outputs instead of treating `-O2` as a notes-only footnote. Each variant contains `fp.esm.js`, `fp.esm.min.js`, `fp.min.js`, `fp.d.ts`, `variant.json`, `attribution.json`, and `raw/program.js`.

| Variant | Compiler | IIFE min | gzip-9 | brotli-11 |
| --- | --- | ---: | ---: | ---: |
| `dist/o1` | `dart compile js -O1` | 251,899 | 64,816 | 53,633 |
| `dist/o2` | `dart compile js -O2` | 151,168 | 52,147 | 44,842 |

The runtime remains inseparable from the program in both rows. `-O2` is the safe production comparison; `-O3` and `-O4` are intentionally excluded because they make additional semantic assumptions.
