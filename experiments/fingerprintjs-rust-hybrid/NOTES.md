# Hybrid Rust+JS notes

Copy of `experiments/fingerprintjs-rust`, renamed to `fingerprintjs-rust-hybrid-experiment`, fiber dropped, allow-listed kernels moved to JavaScript.

## How to build

From this directory, with the pinned `nightly-2026-07-27` toolchain and `spike/rustc-codegen-js`:

```sh
npm run build
```

That is `node tools/build.mjs --optimizer on --scheduler boundary`. Other useful commands:

```sh
npm run build:off          # js-opt=off
npm run check:kernels      # murmur + canonical vectors
npm run check:ssr
npm run check:types
npm run check:package      # exports + types; size gates are allowed to fail
```

Compare-suite success check:

```sh
node ../fingerprintjs-compare/tools/run-differential.mjs --port rust-hybrid
node ../fingerprintjs-compare/tools/check-ssr.mjs --port rust-hybrid
```

Oracle Rollup / Terser are reused from `experiments/fingerprintjs/node_modules`. Do not add a second Terser.

## JS kernels

A reviewer can grep `js/` and see only allow-listed kernels. Nothing from `experiments/fingerprintjs/src` was copied.

| Kernel | File | How it is included | Allow-list |
| --- | --- | --- | --- |
| `murmurX64Hash128` | `js/hash.js` | `js_bindings! { module = "./hash.js"; }` | 1. Murmur inner loop + UTF-8 walk |
| `componentsToCanonicalString` | `js/canonical.js` | imported by `hashComponents` | 2. Canonical `key:value\|key:value` |
| `hashComponents` | `js/canonical.js` | `js_bindings! { module = "./canonical.js"; }` | 2. plus murmur |
| `navigatorGet` | `js/probes.js` | `js_bindings! { module = "./probes.js"; }` | 3. `navigator.X` |
| `screenGet` | `js/probes.js` | same | 3. `screen.X` |
| `matchMediaMatches` | `js/probes.js` | same | 3. `matchMedia` |
| `createCanvas` | `js/graphics.js` | `js_bindings! { module = "./graphics.js"; }` | 4. one DOM API |
| `get2dContext` | `js/graphics.js` | same | 4. get context |
| `supportsWinding` | `js/graphics.js` | same | 4. one canvas probe |
| `renderTextImage` | `js/graphics.js` | same | 4. draw the known scene |
| `renderGeometryImage` | `js/graphics.js` | same | 4. draw the known scene |
| `cachedWebglContext` | `js/graphics.js` | same | 4. create WebGL context |
| `createOfflineAudioContext` | `js/graphics.js` | same | 4. create audio context |
| `configureAudioGraph` | `js/graphics.js` | same | 4. one audio graph |
| `hashAudioSignal` | `js/graphics.js` | same | 4. read samples |
| `canvas.toDataURL()` | `src/kernels.rs` | `js! { canvas.toDataURL() }` | 4. read pixels |

`tools/build.mjs` copies `js/*.js` next to the rustc-codegen-js `program.js` so the emitted `import { … } from "./hash.js"` resolves. Rollup then folds them into the four-format wrapper.

`canvasTestText` lives in `js/graphics.js` for the text scene; it is not a rust binding because `renderTextImage` already uses it.

## Byte split

From `dist/attribution.json` after `js-opt=on`:

| Bucket | bytes | What it is |
| --- | ---: | --- |
| `rustProgram` | 907634 | rustc-codegen-js emit (`raw/program.js`) |
| `rustShim` | 22492 | extracted `rustc-codegen-js` shim |
| `jsKernels` | 10020 | `js/*.js` (10002) + `js!{}` body (18) |
| `facade` | 2687 | named-export wrapper |
| `fp.min.js` | 269643 | gated minified IIFE |

`jsKernels / fp.min.js` ≈ 3.7%. Not "mostly JS".

## Rust-owned work that stayed Rust

Orchestration and policy are still Rust:

- Agent lifecycle (`load`, `get`, lazy `visitorId`, debug print)
- Source registry, `loadSources`, `transformSource`, custom sources
- Confidence score
- Boundary scheduler (`mapWithBreaks`, 16 ms `Date.now()`, `MessageChannel` yield, `Promise.all` order)
- Browser-sniff control flow (which probes count, version cuts, Android / Safari / Gecko branches)
- Which entropy source to run, in which order, with which timeout
- Canvas skip / unstable / unsupported decisions
- Audio suspend / antifingerprinting / retry / timeout state machine
- WebGL parameter and extension enumeration
- DOM blockers, fonts, font preferences, screen frame, user-agent data

## What was dropped

- Fiber scheduler (`--cfg fingerprint_fiber`, `build:fiber`, dual-variant default)
- Rust Murmur four-lane implementation in the product crate
- Rust canonical join / key escape / `JSON.stringify` loop
- Rust canvas scene, WebGL context factory, audio graph, sample sum

Native `#[cfg(test)]` murmur tests were deleted with the rust hash body. Vectors live in `tools/check-kernels.mjs`.

## Size gate

The oracle IIFE is ~37 KB. This port is ~270 KB minified / ~69 KB gzip-9 because the rustc-codegen-js program and shim still dominate. That is a valid hybrid result: kernels are small and honest, the remaining program is still Rust.
