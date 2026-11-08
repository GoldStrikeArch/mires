> **Historical record.** This file describes the archived port, not a fresh qualification. Current restoration/build results and limitations are in `../../docs/COMPARATORS.md`.

# STATUS — fingerprintjs-kotlin

Idiomatic Kotlin port of FingerprintJS 5.2.0, built through both Kotlin Toolchain 0.11.1 `js/app` and the mainstream Kotlin/JS Gradle 2.4.10 production linker.

## What works

- `kotlin build -p js` emits `build/tasks/_fingerprintjs-kotlin_linkJs/fingerprintjs-kotlin.mjs`.
- All 42 sources, in contract order, transliterated from the TypeScript oracle.
- All 25 named exports plus `default = { load, hashComponents, componentsToDebugString }` with identity equality.
- `sources` function names and arities match the oracle (0 except `webGlBasics` / `webGlExtensions`).
- Murmur x64 128 matches the oracle on ASCII, UTF-8 emoji, unpaired surrogates, and seed 17.
- `hashComponents` / `componentsToDebugString` match, including Error own-properties.
- `loadSources` / `transformSource`: `this === null` via `bind(null, …)`, sync/async/two-stage, exclude list, no unhandled rejections.
- `mapWithBreaks` is the upstream 16 ms `Date.now()` budget + `MessageChannel` yield (not a fiber scheduler).
- `LoadOptions.monitoring` is a no-op (no XHR / fetch / beacon).
- SSR-safe ESM import.
- `fp.d.ts` is the oracle file (SHA-256 matches `frozen.json`).
- Differential: **0 normalized differences** on Chrome, Firefox, and Safari.

## Remaining gaps

- None required for the contract checks that were run.
- Size gates will fail; that is expected. The IIFE gzip-9 number is 27 517 bytes.
- `js/app` ignores `-v release`. Readable vs release is Rollup vs Terser, not a toolchain variant.
- Neither lane directly exposes the frozen dynamic API as named ESM exports, so the build adds a small generated facade around `globalThis.__fp_kotlin_exports`.

## How to build

```sh
node tools/build.mjs
```

Requires `~/.local/bin/kotlin` 0.11.1 and the oracle `experiments/fingerprintjs/node_modules` (Rollup + Terser). Produces `dist/fp.esm.js`, `dist/fp.esm.min.js`, `dist/fp.min.js`, `dist/fp.d.ts`.

## Runtime vs program

| | raw bytes |
| --- | ---: |
| Whole-program JS emit (Rollup) | 212076 |
| App modules | 162059 |
| **DCE'd Kotlin/JS stdlib (tax)** | **108234** |
| Shipped minified IIFE | 74653 (gzip-9 27517) |

See `NOTES.md` for gzip/brotli, `js()` inventory, and the emit path.

## Production-profile follow-up (2026-08-15)

The build publishes three explicit rows:

- `dist/readable/`: Kotlin Toolchain 0.11.1 `js/app`, bundled without Rollup tree-shaking, then normal Terser;
- `dist/proxy-production/`: the same `js/app` emit with Rollup tree-shaking and two Terser passes. The toolchain reports that `-v release` is ignored, so this remains a packaging proxy;
- `dist/production/`: Kotlin/JS Gradle plugin 2.4.10, Gradle 9.4.1, `compileProductionExecutableKotlinJs`, whole-program ES modules, ES2015 output, production DCE, minimized member names, then the common facade and two Terser passes.

Root `dist/fp.*` remains an alias of `readable`. `kotlin-production` passes exports, SSR import, and a fresh one-repetition Chrome / Firefox / Safari differential with zero normalized differences. Its minified IIFE is 77,057 B, gzip-9 28,817 B, and Brotli-11 25,514 B.

The retained `js/app` proxy is slightly smaller at 27,176 B gzip. That difference is reported rather than used to relabel the proxy as a compiler-production build.
