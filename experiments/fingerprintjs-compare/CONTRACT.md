# FingerprintJS compiler-comparison contract

Every port in this comparison implements the same public program as
FingerprintJS 5.2.0. This file is the law for those ports. The idiomatic
Rust experiment at `../fingerprintjs-rust` is a frozen reference, not a
workspace to edit.

## Oracle

| Field | Value |
| --- | --- |
| Package | `@fingerprintjs/fingerprintjs` |
| Tag | `v5.2.0` |
| Version | `5.2.0` |
| Commit | `e196578ba35362fdf15647e013d66ac28b3c9fb5` |
| Checkout recorded in rust oracle | `feab7913fbb536466bab72b1795d27ec68423003` |
| Source tree | `../fingerprintjs` |
| Frozen checksums | `../fingerprintjs-rust/oracle/frozen.json` |

Do not read or copy `../fingerprintjs/env.sh`.

## Required artifacts

Each port ships at least these files under its `dist/`:

| File | Meaning |
| --- | --- |
| `fp.esm.js` | Unminified ESM |
| `fp.esm.min.js` | Terser-minified ESM, oracle `terser.config.ts` |
| `fp.min.js` | Minified IIFE (the gated number) |
| `fp.d.ts` | TypeScript declaration; the oracle file is acceptable |

CJS and UMD are optional for the Melange, JSOO, and ReScript ports. Compact
and hybrid Rust should keep the four-format wrapper if they reuse the rust
build script.

Every shipped file starts with the FingerprintJS MIT attribution banner.

## Exports

Named exports, in this order, matching `frozen.json`:

```
componentsToDebugString
default
getFullscreenElement
getUnstableAudioFingerprint
getUnstableCanvasFingerprint
getUnstableHardwareConcurrency
getUnstableScreenFrame
getUnstableScreenResolution
getWebGLContext
hashComponents
isAndroid
isChromium
isDesktopWebKit
isEdgeHTML
isGecko
isSamsungInternet
isTrident
isWebKit
load
loadSources
murmurX64Hash128
prepareForSources
sources
transformSource
withIframe
```

`default` is `{ load, hashComponents, componentsToDebugString }`, and each
of those three is the same function object as the named export.

`sources` is an object whose keys are the 42 source names below, in that
order. `LoadOptions.monitoring` keeps its upstream type and defaults to
`true`. When monitoring is enabled, `window.__fpjs_d_m` is falsy, and
`Math.random() < 0.001`, the port sends an asynchronous
`GET https://m1.openfpcdn.io/fingerprintjs/v5.2.0/npm-monitoring` via
`XMLHttpRequest`. `monitoring: false` disables the request. Tests mock XHR
and force or suppress the sample; they do not hit the network.

## Sources

Built-in entropy sources, in observable collection order:

```
userAgentData
fonts
domBlockers
fontPreferences
audio
screenFrame
canvas
osCpu
languages
colorDepth
deviceMemory
screenResolution
hardwareConcurrency
timezone
sessionStorage
localStorage
indexedDB
openDatabase
cpuClass
platform
plugins
touchSupport
vendor
vendorFlavors
cookiesEnabled
colorGamut
invertedColors
forcedColors
monochrome
contrast
reducedMotion
reducedTransparency
hdr
math
pdfViewerEnabled
architecture
applePay
privateClickMeasurement
audioBaseLatency
dateTimeLocale
webGlBasics
webGlExtensions
```

Scheduler is the upstream `mapWithBreaks` lifecycle only: 16 ms `Date.now()`
budget, `MessageChannel` task yield, non-awaited callback starts, final
`Promise.all` ordering. No fiber scheduler.

## Parity

The first success check for a port that emits ESM:

```sh
node ../fingerprintjs-rust/tests/differential.mjs \
  --candidate dist/fp.esm.js \
  --output tests/results/differential
```

A complete port has zero normalized differences on the local Chrome / Firefox
/ Safari matrix, plus SSR-safe imports and a declaration that matches
`frozen.json`'s `fp.d.ts` SHA-256
`d20e2e6b3f60afb7679c1f87dd62800b7a7ce7155e86efbf6206190a1c185731`.

A port that builds but fails parity is reported as `parity: incomplete`. It
stays in the size table.

Wrappers:

```sh
node tools/run-differential.mjs --port rescript
node tools/run-differential.mjs --dir ../fingerprintjs-rescript
```

## Measurement

Sizes are raw bytes, gzip-9, and brotli-11, matching
`../fingerprintjs-rust/tools/common.mjs`. Totals are summed per artifact.

Runtime bytes are counted on their own side:

| Port | Runtime |
| --- | --- |
| TypeScript oracle | none (it is the handwritten program) |
| Idiomatic / compact / hybrid Rust | extracted `rustc-codegen-js` shim |
| ReScript | `@rescript/runtime` and anything else the emit imports |
| Melange | `melange.js/*` if present; zero if the emit is import-free |
| js_of_ocaml | the JSOO runtime embedded in the whole-program emit |
| Hybrid JS kernels | `js/` plus `js!{}` bodies, recorded in `dist/attribution.json` |
| Dart2js | dart2js prelude / rts if separable |
| Scala.js | Scala.js runtime leftovers after DCE |
| Elm | Elm runtime plus `js/` host, split in `dist/attribution.json` |
| Kotlin/JS | Kotlin/JS stdlib / runtime leftover |
| GopherJS | GopherJS prelude plus the Go runtime / stdlib packages the program reaches, measured against a one-export empty program |

Readable output remains a first-class diagnostic column, but every toolchain
also exposes an explicit production profile where possible:

| Family | Readable row | Production row |
| --- | --- | --- |
| Dart | `dart` / `-O1` | `dart-o2` / safe production `-O2` |
| Scala.js | `scalajs` / `fastLinkJS` | `scalajs-full` / `fullLinkJS` |
| Elm | `elm` / ordinary Terser | `elm-production` / Elm-safe minification applied to generated Elm only |
| Kotlin | `kotlin` / Toolchain 0.11.1 `js/app` | `kotlin-production` / Gradle plugin 2.4.10 `compileProductionExecutableKotlinJs`, whole-program ESM, ES2015, production DCE and member minimization |

The installed Kotlin Toolchain 0.11.1 `js/app` still reports that JS build
variants are unsupported. Its tree-shaken packaging experiment is retained as
`kotlin-proxy` and is never the production-default row. `kotlin-production`
uses the mainstream Kotlin/JS Gradle production linker on the same `src/` tree.

The comparison is runtime-first. A production candidate is considered in this
order:

1. semantic, export, declaration, and SSR parity;
2. end-to-end browser p95 and protected-regression gates;
3. evaluation, immediate-get, lazy-hash, scheduling, responsiveness, and heap;
4. isolated hot-kernel diagnostics;
5. gzip and Brotli as deployment guardrails.

No composite score may exchange bytes for milliseconds. `runtime-matrix.md`
orders by the runtime gate and total p95 only; size is displayed but cannot
improve the rank. PASS requires aggregate and every measured total-p95 cell to
stay within 5% of the oracle. WATCH is within 10%.

```sh
node tools/measure-matrix.mjs --output matrix.json
node tools/summarize.mjs --input matrix.json --profile production --output matrix.production.md
node tools/run-runtime-matrix.mjs
node tools/summarize-runtime.mjs
node tools/run-kernel-matrix.mjs
```

## Allowed techniques

| Port | `unsafe` | `js!{}` / `%raw` / `Js.Unsafe` | Handwritten `.js` | Stdlib / runtime | Goal |
| --- | --- | --- | --- | --- | --- |
| ReScript | n/a | `%raw` only when no typed external exists | No product `.js` | `@rescript/runtime` counted | Idiomatic ReScript |
| Melange | n/a | `[@@mel.xxx]` externals | No product `.js` | Prefer import-free; count `melange.js` if pulled | JS-aware OCaml |
| JSOO | n/a | JSOO PPX / `Js.Unsafe` as normal | No product `.js` | OCaml stdlib + JSOO runtime, counted | Idiomatic OCaml |
| Compact Rust | Forbidden | Forbidden | Forbidden | `no_std`; `alloc` only if a measurement shows it wins | Smallest 100% Rust |
| Hybrid Rust | Forbidden | Allowed, allow-listed | Allowed under `js/`, allow-listed | Same compiler; JS kernels counted separately | Smallest honest Rust+JS |
| Dart | n/a | `dart:js_interop` | Tiny ESM facade only if dart2js cannot emit named exports | dart2js runtime, counted | Idiomatic Dart 3 |
| Scala.js | n/a | `@JSExportTopLevel`, `js.Dynamic` | No product `.js` | Scala.js + DCE'd stdlib, counted | Idiomatic Scala 3 |
| Elm | n/a | Ports and flags only | JS host under `js/`, allow-listed | Elm runtime + JS host, split | Elm kernel + JS host |
| Kotlin/JS | n/a | `@JsExport`; `js()` only when needed | No product `.js` | Kotlin/JS runtime, counted | Idiomatic Kotlin |
| GopherJS | n/a (no `unsafe` package) | `github.com/gopherjs/gopherjs/js`; embedded JS (`eval` / `Function`) only when Go cannot express it, listed in `NOTES.md` | No product `.js`; the build script generates the ESM / IIFE facade, as for JSOO | GopherJS prelude + Go runtime/stdlib, counted | Idiomatic Go 1.21 |

### Compact Rust

100% safe Rust compiled by `rustc-codegen-js`. Delete `format!`, `Debug`,
`Display`, panic messages, `async`/`await`, owned `String`/`Vec` for host
values, and generic component types. Hash as unboxed `u32` lanes. Components
stay `JsValue`. Build with `js-opt=on`, `JSC_RUST_OPT_LEVEL=z`,
`JSC_RUST_MIR_INLINE=on`, `--scheduler boundary`.

### Hybrid Rust

Orchestration, source registry, custom sources, confidence, agent lifecycle,
and exports stay Rust. JS is allowed only for:

1. Murmur x64 inner loop (the four 16-bit lanes plus the UTF-8 walk).
2. Canonical string assembly (`key:value|key:value`, including
   `JSON.stringify` of component values).
3. Host probes that read `navigator.X` / `screen.X` / `matchMedia` with no
   FingerprintJS control flow.
4. DOM / canvas / WebGL / audio helpers that wrap one browser API. The
   decision of which probe to run, in which order, with which timeout, stays
   Rust.

JS enters through `js!{}` expressions or files under `js/` included by the
build and called via `js_bindings!`. Generation scripts live in `tools/` and
are deterministic. `dist/attribution.json` records
`{rustProgram, rustShim, jsKernels, facade}`. If JS kernels exceed 50% of the
minified IIFE, the row is labeled "mostly JS".

### Elm

Elm 0.19 has no native modules and no direct DOM. The port is Elm for murmur,
canonicalization, confidence, the 42-key order, and pure sources, plus a JS
host under `js/` for browser probes, `mapWithBreaks`, and the 25-export
facade. Ports or `Elm.Fingerprint.init` are the only Elm/JS boundary.
`dist/attribution.json` records `{elmProgram, elmRuntime, jsHost, facade}`.
If `jsHost` exceeds 50% of the minified IIFE, the row is labeled "mostly JS".

## Porting map

Reference order: oracle TypeScript for behavior, idiomatic Rust for a typed
walkthrough. Do not invent a third semantics.

| Concern | TypeScript | Idiomatic Rust |
| --- | --- | --- |
| Public API | `src/agent.ts`, `src/index.ts` | `api.rs`, `entry.rs` |
| Source registry | `src/sources/index.ts` | `manifest.rs`, `entropy.rs` |
| Hash | `src/utils/hashing.ts` | `hashing.rs` |
| Canonicalization | inside agent | `canonical.rs` + host `JSON.stringify` |
| Confidence | `src/confidence.ts` | `confidence.rs` |
| Browser sniff | `src/utils/browser.ts` | `browser.rs`, `browser_runtime.rs` |
| Sync sources | `src/sources/*.ts` | `sources_sync.rs`, `sources_runtime.rs` |
| DOM / fonts / blockers | `fonts.ts`, `dom_blockers.ts`, `font_preferences.ts` | `sources_dom.rs`, `blocker_data.rs`, `host_dom.rs` |
| Canvas / WebGL / audio | `canvas.ts`, `webgl.ts`, `audio.ts` | `sources_graphics.rs`, `sources_async.rs`, `host_graphics.rs` |
| Scheduler | `src/utils/async.ts` | `scheduler.rs` boundary |

Heavy sources first: `dom_blockers`, `webgl`, `audio`, `canvas`, `fonts`,
`font_preferences`, `hashing`, `agent` / `entropy_source`.

## Directories

| Implementation | Directory | Notes |
| --- | --- | --- |
| TypeScript oracle | `../fingerprintjs` | Frozen |
| Idiomatic Rust | `../fingerprintjs-rust` | Frozen |
| ReScript | `../fingerprintjs-rescript` | Wave 1 |
| Melange 7.0.1 | `../fingerprintjs-melange` | Wave 1 |
| js_of_ocaml | `../fingerprintjs-jsoo` | Wave 1 |
| Compact Rust | `../fingerprintjs-rust-compact` | Copy-then-mutate |
| Hybrid Rust | `../fingerprintjs-rust-hybrid` | Copy-then-mutate |
| Dart2js | `../fingerprintjs-dart` | Dart 3.13 |
| Scala.js | `../fingerprintjs-scalajs` | Scala 3.8 + scala-cli |
| Elm | `../fingerprintjs-elm` | Elm 0.19 + JS host |
| Kotlin/JS | `../fingerprintjs-kotlin` | Kotlin toolchain 0.11 |
| GopherJS | `../fingerprintjs-gopherjs` | GopherJS 1.21.0 + Go 1.21.13 (added 2026-11-18) |

A port agent edits only its own directory. Compiler changes to
`spike/rustc-codegen-js` are out of scope for the ports.

## License

Unpublished experiment names only (`fingerprintjs-*-experiment`). Every
distribution keeps the FingerprintJS MIT notice and the declaration
attribution.

## Strict upstream-parity qualification

The contract above is the historical compiler-comparison contract. Its deliberate `LoadOptions.monitoring` no-op means that a port marked complete here matches the benchmark, but is not automatically a literal 1:1 implementation of the unmodified FingerprintJS package.

The strict audit in `tools/check-feature-parity-static.mjs` and `tools/run-feature-parity.mjs` additionally checks the upstream monitoring request, debug side effects, direct utility behavior, iframe rejection/cleanup, observable function metadata, IIFE namespace shape, and oracle artifact formats. See `feature-parity.md` and `../../FINGERPRINTJS-FEATURE-PARITY-AUDIT.md`.
