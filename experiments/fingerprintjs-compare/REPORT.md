> **Historical comparison archive.** The matrices in this directory are retained historical results. Use `npm run build:comparators`, `npm run check:comparators`, and `node experiments/tools/measure-comparators.mjs` for the restored checkout. See `../../docs/COMPARATORS.md`.

# FingerprintJS compiler comparison

Status report, 2026-08-14. Nine compiled ports of FingerprintJS 5.2.0 sit next to the frozen TypeScript oracle and the frozen idiomatic Rust experiment. Every complete port passed the existing three-browser differential (zero normalized differences), SSR-safe ESM import, and the 25-export / 42-source namespace check. Size gates against the handwritten oracle still fail for every compiled port. That is the result.

The contract is [CONTRACT.md](CONTRACT.md). The size table is [matrix.json](matrix.json). Per-port notes live in each experiment's `NOTES.md` / `STATUS.md`. This file is the connective tissue and the lessons for `rustc-codegen-js`.

**Strict upstream-parity follow-up, 2026-08-15.** All ten canonical ports match the exact 42-source component/agent contract across Chrome, Firefox, and Safari, and they now also match upstream installation monitoring, Safari DOM-blocker debug text, and Dart `withIframe` rejection identity. Remaining gaps are function `name`/`length` metadata, IIFE key order, and optional CJS/UMD artifacts. See [feature-parity.md](feature-parity.md) and [../../FINGERPRINTJS-FEATURE-PARITY-AUDIT.md](../../FINGERPRINTJS-FEATURE-PARITY-AUDIT.md).

## What was compared

Same public program: oracle commit `e196578ba35362fdf15647e013d66ac28b3c9fb5`, 42 sources in order, 25 runtime exports, `mapWithBreaks` only, upstream `monitoring` sample (mocked XHR in tests), oracle `fp.d.ts` (SHA-256 `d20e2e6b3f60afb7679c1f87dd62800b7a7ce7155e86efbf6206190a1c185731`).

| Implementation | Compiler | Source rule | Runtime in the shipped file |
| --- | --- | --- | --- |
| TypeScript oracle | tsc + Rollup + Terser | Handwritten | none |
| ReScript 12.3.0 | `rescript` → Rollup + Terser | Idiomatic ReScript | none (Rollup dropped `@rescript/runtime`) |
| Melange 7.0.1-53 | `melange.emit` → Rollup + Terser | JS-aware OCaml | none (import-free by construction) |
| js_of_ocaml 6.4.1 | whole-program bytecode → JS | Idiomatic OCaml + Lwt | 36,485 B DCE'd runtime |
| Compact Rust | `rustc-codegen-js` opt-on / rust-z / mir-inline | 100% Rust, written to the cost model | 19,535 B extracted shim |
| Hybrid Rust | `rustc-codegen-js` opt-on + `js/` kernels | Rust orchestration, allow-listed JS | 22,492 B shim + 10,020 B JS kernels |
| Idiomatic Rust | `rustc-codegen-js` (frozen `dist/boundary`, opt-off) | Safe Rust, no `js!` | 21,173 B shim |
| Dart 3.13 dart2js | `dart compile js -O1` + Terser | Idiomatic Dart 3 + `dart:js_interop` | inseparable dart2js prelude |
| Scala.js 1.22 | scala-cli `--js` fastLink + Terser | Idiomatic Scala 3 + `@JSExportTopLevel` | ~42 KB fullLink empty-export floor |
| Elm 0.19.2 | `elm make --optimize` + JS host | Elm kernel + counted JS host | Elm runtime 60,935 B + JS host 63,352 B |
| Kotlin/JS | Kotlin toolchain 0.11.1 `js/app` | Idiomatic Kotlin + `@JsExport` | 108,234 B DCE'd stdlib |

Melange and js_of_ocaml do not share a source tree. ReScript `%raw` and Melange `[%mel.raw]` were allowed only where a typed external could not express the call. Compact forbids `js!` and product `.js`. Hybrid's `js/` is the four allow-listed kernels plus one `js!{}` for `canvas.toDataURL`.

## Size

Minified IIFE, gzip-9, brotli-11. Oracle IIFE gzip is the gated number (16,178 B).

| Implementation | IIFE min | gzip-9 | brotli-11 | vs oracle gzip | readable ESM | min ESM |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| TypeScript oracle | 36,746 | 16,178 | 14,239 | 1.00× | 129,311 | — |
| ReScript | 42,283 | 17,453 | 15,422 | **1.08×** | 96,158 | 42,198 |
| Melange 7.0.1 | 49,856 | 18,840 | 16,655 | **1.16×** | 107,682 | 49,769 |
| js_of_ocaml | 136,140 | 47,988 | 41,870 | 2.97× | 358,357 | 136,383 |
| Compact Rust | 192,830 | 53,594 | 41,549 | 3.31× | 608,782 | 192,850 |
| Hybrid Rust | 269,643 | 68,646 | 56,222 | 4.24× | 882,461 | 269,652 |
| Idiomatic Rust (opt-off boundary) | 332,361 | 73,917 | 59,297 | 4.57× | 1,087,270 | 332,381 |

The 2026-08-14 optimizer-on idiomatic artifact was 277,937 B min ESM / 70,401 B gzip. Compact is 31% smaller than that neighbor (192,850 vs 277,937) and 42% smaller than the opt-off boundary this matrix measures.

JSOO's unminified ESM is a `--pretty` second compile of the same bytecode (358 KB). Its compact whole-program emit before the facade is 138,894 B: 102,409 B program + 36,485 B DCE'd runtime. The unused full runtime is 469,772 B; whole-program DCE threw almost all of it away.

Hybrid `dist/attribution.json`: `{ rustProgram: 907634, rustShim: 22492, jsKernels: 10020, facade: 2687 }`. JS kernels are 3.7% of the minified IIFE. Not mostly JS.

Compact remaining `core` after the cost-model pass: 76 items / 96,807 B, dominated by `str::slice_error_fail_rt`, `fmt::write`, `pad_integral`, `char::escape_debug_ext`, and Unicode tables pulled by `&str` indexing — not by `format!` or `Display`. `flt2dec` is gone. No Future poller.

## Parity and contract (Wave 2 recheck)

`tools/check-exports.mjs` and `tools/check-ssr.mjs` were rerun on every port after the agents finished. All six compiled artifacts (five new + frozen idiomatic) passed. Three-browser differentials were run by each port agent against the existing rust harness; each reported zero normalized differences on Chrome, Firefox, and Safari.

Chrome idle benches, p50 / p95 milliseconds. ReScript used a longer sample (n=100); the others used `--runs 8` (n=8). Candidate is the rust harness's boundary slot. Each row's upstream is the same oracle artifact measured in that run.

| Port | Eval | Immediate `get` | `load` | Lazy hash | Total |
| --- | ---: | ---: | ---: | ---: | ---: |
| Upstream (ReScript run) | 1.4 / 1.6 | 1.1 / 1.2 | 26.8 / 31.4 | 2.7 / 2.9 | 133.8 / 144.2 |
| ReScript | 1.6 / 1.9 | 2.6 / 2.9 | 25.0 / 30.2 | 2.7 / 2.9 | 134.0 / 144.4 |
| Melange | 1.9 / 2.2 | 2.8 / 3.2 | 28.7 / 31.5 | 2.8 / 3.1 | 143.8 / 144.5 |
| js_of_ocaml | 4.9 / 5.6 | 5.8 / 11.9 | 25.2 / 30.2 | 3.2 / 3.5 | 144.5 / 155.4 |
| Compact Rust | 4.3 / 4.7 | 3.4 / 3.5 | 26.2 / 34.2 | 4.6 / 4.8 | 146.4 / 156.0 |

ReScript and Melange totals are noise against the oracle. Immediate `get` is ~2.5× (Promise wrapping). Compact's immediate `get` is 3.4 ms p50, against the idiomatic rust verdict's 11.5 ms p50 (+516%). Deleting `async`/`await` removed the executor hop. Eval still tracks bytes: ReScript 1.6 ms at 17 KB gzip, compact 4.3 ms at 54 KB, JSOO 4.9 ms at 48 KB. Idiomatic rust was 9.6 ms at 76 KB. Hash is the one compact leftover (4.6 vs 2.8 ms) — the unboxed `u32` lanes are still rustc objects, not JS numbers.

Records: `results/<port>/bench/chrome.json`. Firefox / Safari benches were not rerun this wave; the original rust three-browser matrix still stands for the idiomatic port.

## Qualitative emit

ReScript's murmur is handwritten-shaped. The inner loop is a `while` over a UTF-8 `Uint8Array`, mutating `{high, low}` objects with `<<` / `|` / `>>>`, then the same 16-bit multiply the oracle uses:

```js
function x64hash128(input, seed) {
  let key = getUTF8Bytes(input);
  // ...
  k1.low = jsIndex(key, i) | (jsIndex(key, i + 1 | 0) << 8) | ...
  x64Multiply(k1, c1);
  x64Rotl(k1, 31);
```

That is the oracle's algorithm with ReScript's names. There is no shim, no TAG object, no `BigInt`.

Melange stayed import-free only by refusing the OCaml stdlib the same way the store microbench did. `NOTES.md` is a list of threshold effects: `a.(i)` → `caml_array.js`; integer `mod` → `caml_int32.js`; `Js.Exn` → `caml_exceptions.js`; `string_of_float` → `caml_format.js`; `=` on unknown JS values → `caml_equal`. Murmur is `[%mel.raw]` because Melange `int` is signed 32-bit and corrupts the high hash lane. That is our `u32` vs `i32` story under another name.

js_of_ocaml compiled a real OCaml program (`use-js-string=true`, `--effects=disabled`, Lwt for `mapWithBreaks`). The 36 KB runtime that survived DCE is the honest "language on JS" tax. It is the same order as our 21 KB shim, and it does not also drag 200 KB of `Display`.

Compact Rust deleted `format!`, `async`/`await`, owned `String`/`Vec`, and the `{high,low}` murmur struct. The next bytes it cannot delete without a compiler change are panic-formatting behind `&str` indexing.

Hybrid moved murmur, canonicalization, navigator/screen/`matchMedia`, and the one-function canvas/WebGL/audio wrappers into 10 KB of JS. The Rust program stayed 908 KB raw. Putting the hot loop in JS does not evict `core`.

## Lessons for `rustc-codegen-js`

Each lesson cites a number from this run.

1. **The 16 KB gzip gate is a representation problem, not a library-irreducible floor.** ReScript is 1.08× the oracle gzip. Melange, written to stay on JS values and to refuse `Format`, is 1.16×. FingerprintJS itself is not a 50 KB program. When the language's strings, arrays, and records *are* JS strings, arrays, and objects, the gate is in reach. Native representations of `String` / `Vec` / `fmt::Arguments`, and host-boundary unpinning so those regions can commit, stay the top compiler work. This measurement replaces the earlier hedge that "the gate effectively demands hand-written density." It demands *JS-native values*. ReScript is the existence proof.

2. **A faithful language runtime is a 30–40 KB tax, not a 200 KB tax.** JSOO's DCE'd runtime is 36,485 B. Our extracted shim is 19–22 KB. Those are the same kind of number. The 200 KB we paid in the idiomatic rust port was `Display` / `format!` / `flt2dec` / Unicode-behind-`str::contains`, which compact deleted and which Melange also refused (`caml_format.js`). The architecture of "compile real `core`" is fine if `core` is not reachable. The bug is the threshold, not the decision to use rustc.

3. **Source discipline is a real lever and it has a ceiling.** Compact is −42% min ESM and −27% gzip versus opt-off idiomatic, −31% versus opt-on idiomatic, and still 3.31× the oracle. Immediate `get` fell from 11.5 ms p50 (idiomatic) to 3.4 ms (compact) because `async`/`await` and the executor are gone. The first experiment paid for readable Rust (`async`, `format!`, `String`, structs) in both bytes and the get path. About two-fifths of the minified bytes were that bill. The remaining 97 KB of `core` is compiler-owned panic formatting. Compact cannot delete it. The compiler can, by making slice/`str` panics not pull `fmt`.

4. **`js!{}` plus a `js/` include is the wrong next product feature if the goal is this library's size.** Hybrid's allow-listed kernels are 10,020 B (3.7% of the IIFE). The row is 4.24× the oracle, worse than compact, only slightly better than opt-on idiomatic. The kernels were already cheap as JS. The expensive thing is still the Rust value model and reachable `core`. Productize host-boundary unpinning and a panic-without-formatting core before productizing `js/` includes. `js!{}` remains useful for host-shaped calls (the `toDataURL` block is 18 bytes). It is not a size strategy.

5. **Threshold effects are not a Rust bug.** Melange's rejected-idiom list is the same shape as ours: one stdlib feature, one imported runtime file, a lump of bytes. JSOO pays a smaller lump because whole-program DCE sees the bytecode. Our zombie/DCE graph is the right idea; it fails when a panic message or a `Display` impl keeps a cluster alive. The analogous "do not reach this" list for rustc-codegen-js is now measured on both sides: do not reach `fmt` from product code, and do not let `str` indexing reach `fmt` from the compiler.

6. **Readable-to-minified ratio tracks how far the emit is from JS.** Oracle IIFE raw/gzip is 2.27. ReScript 2.42. Melange 2.65. JSOO 2.84. Compact 3.60. Idiomatic 4.50. We still emit more structure than Terser can crush, because the structure is object-shaped rustc values. ReScript's murmur is the readable-output goal made concrete: keep that, lose the TAG objects.

## What this does not change

The original idiomatic rust verdict stands. That port remains a research artifact, not a shipping replacement. Compact and hybrid are also research artifacts. ReScript and Melange are the density the compiler should aim at, not a proposal to rewrite Topcoat in OCaml.

The island/application design point is untouched. A 847 B gzipped counter island is still the thing this compiler is for. The library experiment is still the stress test.

## What to do next in the compiler

In this order, each with a fixture before a FingerprintJS rebuild:

1. Panic-without-formatting (or a formatting-free `slice_error_fail`) so `&str` indexing does not pull `fmt` / Unicode. Compact's remaining 97 KB is the budget.
2. Host-boundary unpinning so immutable native strings can commit. The 30 born-dead `String`/`Vec` regions in the original report, plus compact's "components stay `JsValue`" workaround, are the same hole.
3. `fmt::Arguments` as a native representation, so the idiomatic port can keep `format!` without the 200 KB cluster. Compact proves the cluster is optional; idiomatic Rust should not have to give it up.
4. Only then: treat `js!{}` host calls as unpinning edges for native-repr planning. Hybrid showed the include convention works as a build step. It is not worth a compiler feature until (1)–(3) land.

## Reproduction

```sh
eval "$(opam env --switch=fingerprintjs-compare --set-switch)"
# each port: node tools/build.mjs   (rust ports use their own flags; see STATUS.md)
cd experiments/fingerprintjs-compare
node tools/measure-matrix.mjs --output matrix.json
node tools/check-exports.mjs --port rescript   # and melange, jsoo, rust-compact, rust-hybrid
node tools/check-ssr.mjs --port rescript
node tools/run-differential.mjs --port rescript
```

Toolchains: [toolchains.md](toolchains.md). Node v24.5.0, ReScript 12.3.0, OCaml 5.3.0, Melange 7.0.1-53, js_of_ocaml 6.4.1, rustc nightly-2026-07-27.

## Dart, Scala.js, Elm, and Kotlin/JS

Four more compilers, same contract. Agents reported 0 normalized diffs on Chrome / Firefox / Safari. Wave 2 rechecked `check-exports` and `check-ssr` on all four. Differentials were not rerun by the harness after the agents; treat the agent 0-diff reports as the parity receipt.

Minified IIFE from `matrix.json` (2026-08-14T21:59Z). Compact row here is the post-panic-stub rebuild.

| Implementation | IIFE min | gzip-9 | brotli-11 | vs oracle gzip |
| --- | ---: | ---: | ---: | ---: |
| TypeScript oracle | 36,746 | 16,178 | 14,239 | 1.00× |
| ReScript | 42,283 | 17,453 | 15,422 | 1.08× |
| Melange 7.0.1 | 49,856 | 18,840 | 16,655 | 1.16× |
| Elm 0.19 (mostly JS) | 57,980 | 22,992 | 20,488 | 1.42× |
| Kotlin/JS | 74,653 | 27,409 | 24,225 | 1.69× |
| Compact Rust (after `slice_error_fail` stub) | 159,745 | 43,910 | 33,627 | 2.71× |
| js_of_ocaml | 136,140 | 47,988 | 41,870 | 2.97× |
| Dart2js `-O1` + Terser | 251,899 | 64,816 | 53,633 | 4.01× |
| Hybrid Rust | 269,643 | 68,646 | 56,222 | 4.24× |
| Idiomatic Rust opt-off | 332,361 | 73,917 | 59,297 | 4.57× |
| Scala.js fastLink + Terser | 485,126 | 91,364 | 73,517 | 5.65× |

**Dart.** dart2js cannot emit named ESM exports. The facade only assigns the 25 names. The prelude reads `typeof navigator` / `self` / `document` at init; the wrapper binds those identifiers to `undefined` so SSR poison getters do not fire. Runtime is mixed into the script and is not separable. `-O1` program 433,186 B vs `-O2` 164,977 B (footnote). Terser of `-O1` is the gated row. 4.01× gzip: a production JS compiler with a real runtime, not ReScript density.

**Scala.js.** `@JSExportTopLevel` gave a real ESM namespace with no facade. The gated row is Terser over `fastLink` (readable). `fullLink` is 608,539 B raw / 106,045 B gzip, still larger than Kotlin or compact Rust after gzip. An empty `@JSExportTopLevel` `fullLink` is **41,969 B** (the DCE'd Scala.js + javalib floor). The FingerprintJS program then adds ~567 KB of Scala before Terser. Scala.js DCE did not save this library. Idiomatic Scala collections and `js.Dynamic` probes are expensive in a way ReScript's shared data types are not.

**Elm.** Labeled **mostly JS**. `dist/attribution.json`: `{elmProgram: 38033, elmRuntime: 60935, jsHost: 63352, facade: 1370, iifeMin: 57980, mostlyJs: true}`. `jsHost` is 109% of the minified IIFE because Terser shares strings across Elm and the host. Elm 0.19 cannot probe the browser except through ports, so the 25-export library is a JS host that calls Elm for murmur, UTF-8, canonical join, confidence, and math. That is the honest Elm result, not a failure of the port. The Elm kernel (38 KB raw program + 61 KB runtime before wrap) is the number to compare to our murmur + `core` tax.

**Kotlin/JS.** `js/app` cannot export a library namespace. `main()` installs `globalThis.__fp_kotlin_exports`; the build script generates a tiny facade. Differential 0 diffs. DCE'd Kotlin/JS stdlib is **108,234 B** in the linker emit; shipped IIFE is 74,653 / 27,409 gzip (**1.69×** oracle). That is the best of these four that is still a real compiled language (Elm's 1.42× is the JS host). 108 KB stdlib vs our 21 KB shim vs JSOO's 36 KB runtime: Kotlin's leftover is larger, but the shipped gzip is closer to Melange than to dart2js.

### Lessons for `rustc-codegen-js` from this round

1. **JS-native values still win.** ReScript and Melange remain the density club. None of Dart, Scala.js, or Kotlin joined it. Dart `String` is a JS string and it still shipped 4× gzip; the dart2js prelude and whole-program object model cost real bytes.
2. **A 40–110 KB language runtime is normal.** Scala.js empty fullLink 42 KB, JSOO 36 KB, our shim 17–22 KB, Kotlin leftover 108 KB. Our shim is not the problem. Reachable `fmt` was.
3. **Scala.js DCE is not a free 16 KB library.** The 42 KB floor is fine; the 567 KB of program above it is the Scala that survived DCE. Interop-friendly Scala is not ReScript.
4. **Elm confirms the hybrid lesson.** When the language cannot express the host, the JS host becomes the library. rust-hybrid's 3.7% JS kernels were the opposite split. Do not chase Elm as a size strategy for Topcoat islands.
5. **Kotlin 1.69× gzip is the new "faithful OO language" mark**, better than dart2js and our idiomatic rust, worse than ReScript. Native representations are still the path from 2.7× compact rust toward 1.1×.

## Compiler follow-up (the three unlocks)

The three slices named in "What to do next" landed in `spike/rustc-codegen-js` after this comparison. Optimizer default is still `off`. Compact and idiomatic-`on` rebuilds both kept 0 normalized differences on Chrome, Firefox, and Safari.

| Workload | Before | After | What moved |
| --- | ---: | ---: | --- |
| Compact min ESM | 192,850 | **159,764** | `slice_error_fail_rt` no longer pulls `fmt` / Unicode Debug |
| Compact IIFE gzip-9 | 53,594 | **43,910** | same |
| Compact raw program | 614,026 | **521,642** | ~92 KB, matching the old 97 KB `core` cluster |
| Idiomatic `on` min ESM | 277,937 | **258,721** | host-sink unpin + panic stub; `print_debug` still reaches `flt2dec` |
| Idiomatic `on` gzip-9 | 70,401 | **64,493** | same |
| `55_native_host_string` `on` | 24,964 | 5,992 | `&str` host args are sinks |
| `56_native_format_args` `on` | 275,291 | 3,405 | proven `format!("{x}")` for `u32` / `&str` / default `f64` |

`54_str_index_no_fmt` (54,990 B) is the no-fmt indexing receipt. Compact still has a `white_space` Unicode map from `is_whitespace`, not from `Debug`. Native `format!` does not yet eat FingerprintJS `print_debug`; that path is not a default-shortest template. ReScript at 17,453 B gzip remains the representation endgame.

## Limits

Local Chrome / Firefox / Safari only, same as the rust verdict. No BrowserStack. ReScript's differential was run with `--repetitions 1`; the others used the harness default of 2. CJS/UMD were not required of the ML ports. The TypeScript oracle has no `fp.esm.min.js`, so its "complete" flag in `matrix.json` is false; the gated IIFE is present and matches `frozen.json`.

## Production-profile correction and runtime-first follow-up (2026-08-15)

The earlier matrix intentionally used a readable-first common packaging path. That remains useful for inspecting compiler output, but it understated the strongest safe production modes of Dart and Scala.js and omitted Elm's compiler-aware minifier recipe. The comparison now keeps both profiles instead of replacing the historical rows:

- Dart: `dart` is `-O1`; `dart-o2` is the safe production `-O2` compiler output, then the same facade and Terser packaging.
- Scala.js: `scalajs` is `fastLinkJS`; `scalajs-full` is `fullLinkJS` / release, then the same Terser packaging.
- Elm: `elm` uses ordinary Terser; `elm-production` first applies Elm's compiler-safe pure-helper / unsafe compression flags to generated Elm only, then bundles the JS host with ordinary Terser.
- Kotlin: `kotlin` is the readable Kotlin Toolchain 0.11.1 `js/app` row; `kotlin-proxy` retains the old tree-shaken packaging proxy; `kotlin-production` compiles the same sources with Kotlin/JS Gradle plugin 2.4.10, `compileProductionExecutableKotlinJs`, whole-program ES modules, ES2015 output, production DCE, and minimized member names.

The old root `dist/` artifacts remain aliases of the readable-first rows. Complete production artifacts live under each port's variant directory, and `ports.json` marks which row is the default production comparison.

### Corrected production size matrix

| Implementation | IIFE min | gzip-9 | brotli-11 | gzip vs oracle |
| --- | ---: | ---: | ---: | ---: |
| TypeScript oracle | 36,746 | 16,178 | 14,239 | 1.00× |
| ReScript | 42,283 | 17,453 | 15,422 | 1.08× |
| Melange 7.0.1 | 49,856 | 18,840 | 16,655 | 1.16× |
| Elm compiler-aware minifier | 53,949 | 21,964 | 19,616 | 1.36× |
| Kotlin/JS Gradle production | 77,057 | 28,817 | 25,514 | 1.78× |
| Compact Rust | 159,745 | 43,910 | 33,627 | 2.71× |
| js_of_ocaml | 136,140 | 47,988 | 41,870 | 2.97× |
| Dart2js `-O2` | 151,168 | 52,147 | 44,842 | 3.22× |
| Scala.js `fullLink` | 228,513 | 67,338 | 56,734 | 4.16× |

The production correction is material but does not reverse the representation result. Dart improves by about 12.7 KB gzip relative to the `-O1` row. Scala.js improves by about 24.0 KB gzip relative to `fastLink`. Elm saves about 1.0 KB gzip through compiler-aware minification. The true Kotlin Gradle production row is 28,817 B gzip; the retained `js/app` packaging proxy is smaller at 27,176 B, so the proxy was not concealing an unfair size disadvantage.

All four new production rows passed:

- the 25-export / 42-source namespace check;
- SSR-safe ESM import;
- a fresh one-repetition Chrome / Firefox / Safari differential with zero normalized differences.

### Runtime now precedes bundle size

The comparison no longer treats the smallest compressed file as the implicit winner. The production decision order is:

1. exact behavior and API parity;
2. end-to-end browser p95 and protected-regression gates;
3. compiler-sensitive phases: module evaluation, immediate `get`, lazy hash, scheduling, responsiveness, and heap;
4. isolated hot-kernel diagnostics;
5. gzip and Brotli as deployment constraints.

There is deliberately no formula that trades milliseconds for bytes. A smaller bundle cannot compensate for a runtime regression, and a faster hot loop does not excuse an unbounded package.

`tools/run-runtime-matrix.mjs` runs the existing protected browser harness for every `runtimeDefault` row. `tools/summarize-runtime.mjs` reports end-to-end p95 first. PASS requires both the aggregate and every measured total-p95 cell to stay within 5% of the oracle; WATCH is within 10%. Bundle size is shown but never affects ordering.

### Full three-browser runtime matrix

The production matrix was rerun consistently across Chrome, Firefox, and Safari and across the idle, CPU, and DOM scenarios. Every row began with 8 measured runs and 2 warmups per browser/scenario; the existing harness automatically expanded near-threshold cells. The final records ranged from 72 measurements per browser to 900 for Scala.js Safari. These are now the primary runtime receipts; the earlier one-cell Chrome smoke is superseded.

| Implementation | Gate | Total p95 | Worst total cell | Compiler-overhead p95 | Eval p95 | Immediate get p95 | Lazy hash p95 | Event-loop p95 | IIFE gzip vs oracle |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Melange 7.0.1 | **PASS** | 1.00× | 1.02× | 1.37× | 1.34× | 2.09× | 0.93× | 1.05× | 1.16× |
| ReScript | **PASS** | 1.01× | 1.03× | 1.39× | 1.31× | 2.02× | 1.02× | 1.04× | 1.08× |
| Scala.js `fullLink` | FAIL | 1.04× | 1.14× | 1.90× | 3.78× | 2.09× | 0.87× | 1.07× | 4.16× |
| js_of_ocaml | FAIL | 1.05× | 1.12× | 1.74× | 2.61× | 1.68× | 1.20× | 1.14× | 2.97× |
| Elm compiler-aware | FAIL | 1.08× | 1.13× | 1.97× | 1.57× | 0.97× | 5.02× | 1.23× | 1.36× |
| Compact Rust | FAIL | 1.09× | 1.12× | 2.04× | 2.19× | 2.63× | 1.47× | 1.31× | 2.71× |
| Dart2js `-O2` | FAIL | 1.10× | 1.15× | 2.62× | 3.33× | 2.87× | 1.87× | 1.26× | 3.22× |
| Kotlin/JS Gradle production | FAIL | 1.20× | 1.25× | 3.86× | 2.06× | 2.16× | 12.92× | 1.75× | 1.78× |

`compiler-overhead p95` is the geometric mean of evaluation, immediate-get, and lazy-hash ratios. PASS requires both the aggregate and every measured total-p95 cell to stay within 5% of the oracle. The result is stricter than the earlier Chrome-idle smoke: only ReScript and Melange pass the protected runtime rule across the complete matrix.

Several lessons become clearer in the full run:

- Scala.js has an oracle-shaped hash and only 4% aggregate total overhead, but one protected cell reaches 14%, module evaluation is 3.78×, and the bundle is 4.16× gzip. Fast compute does not make the application result acceptable.
- js_of_ocaml also sits close on aggregate total, but its worst cell is 12%, event-loop p95 is 14% higher, and its bundle is 2.97× gzip.
- Compact Rust is 9% over the oracle in aggregate and 12% in its worst cell. The remaining compiler-sensitive overhead is visible in evaluation, immediate-get, and hash even after the large size reductions.
- Elm demonstrates the opposite imbalance: a lean 1.36× gzip artifact and near-oracle immediate-get, but a 5.02× browser lazy-hash phase and a 13% worst total cell.
- Kotlin's true Gradle production artifact is still lean at 1.78× gzip, but lazy hash is 12.92×, aggregate total is 20% higher, and the worst protected cell is 25% higher.
- Dart `-O2` materially improves size over `-O1`, yet remains 10% over aggregate total, 15% in the worst cell, and over 3× gzip.

Heap-delta ratios remain reported in `runtime-matrix.md`, but they are not used to infer allocation quality in isolation: some rows lack a comparable measurement, and GC timing can make a smaller delta misleading. Allocation-site and retained-heap diagnostics remain separate required evidence for v2.

### Isolated Murmur diagnostic

`tools/run-kernel-matrix.mjs` imports each production ESM in a fresh Node process and measures the exported Murmur path on fixed ASCII and Unicode inputs. It validates every result against the oracle. This is a hot-kernel diagnostic, not protected browser evidence.

| Implementation | Kernel p50 vs oracle | Module import vs oracle |
| --- | ---: | ---: |
| ReScript | 0.98× | 0.90× |
| Scala.js `fullLink` | 1.00× | 4.66× |
| Melange | 1.03× | 1.37× |
| Compact Rust | 1.46× | 3.14× |
| Dart2js `-O2` | 1.77× | 4.73× |
| js_of_ocaml | 2.09× | 2.68× |
| Elm compiler-aware | 8.67× | 1.49× |
| Kotlin/JS Gradle production | 62.44× | 1.88× |

The strongest new finding is that size and hot-loop speed are independent enough to require separate objectives. Scala.js has the largest corrected production bundle but an oracle-shaped hash. The true Kotlin Gradle production artifact is only 1.78× the oracle gzip yet its unsigned/wide arithmetic path is roughly 62× the oracle in this diagnostic. Elm packages compactly because the host remains JS, while its port-mediated Elm hash is roughly 9× the oracle.

### Compiler implications

1. **Production modes must be first-class compiler policies.** V2 should expose `dev`, `report`, `release-size`, `release-balanced`, and `release-speed` over one legality system rather than one optimizer boolean.
2. **Legality and profitability must be separate.** A representation or ABI rewrite can be legal but wrong for a selected speed/size profile.
3. **The cost model must include hot arithmetic form and allocations.** Kotlin demonstrates that a compact graph can still select disastrous BigInt-backed unsigned helpers.
4. **End-to-end and kernel performance must both be gated.** Browser totals can hide compiler overhead behind DOM and audio work.
5. **Size remains a guardrail.** Scala.js demonstrates that near-oracle compute speed does not make a 4.16× gzip library lean.

Reproduction:

```sh
# Build explicit variants
node ../fingerprintjs-dart/tools/build.mjs
node ../fingerprintjs-scalajs/tools/build.mjs
node ../fingerprintjs-elm/tools/build.mjs
node ../fingerprintjs-kotlin/tools/build.mjs

# Size and profile tables
node tools/measure-matrix.mjs --output matrix.json
node tools/summarize.mjs --input matrix.json --profile production --output matrix.production.md

# Production runtime and kernel matrices
node tools/run-runtime-matrix.mjs --browsers chrome,firefox,safari --scenarios idle,cpu,dom --runs 8 --warmups 2
node tools/summarize-runtime.mjs
node tools/run-kernel-matrix.mjs --rounds 12 --iterations 750
```
