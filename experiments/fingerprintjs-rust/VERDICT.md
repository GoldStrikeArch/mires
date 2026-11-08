# Rust FingerprintJS 5.2.0 parity experiment verdict

The experiment is complete. API and browser behavior parity passed. The performance and bundle-size gates failed. The boundary build is the default artifact, and this implementation is not a viable shipping replacement for FingerprintJS 5.2.0 in its current form.

## Scope and oracle

The read-only behavior oracle is FingerprintJS tag `v5.2.0` at commit `e196578ba35362fdf15647e013d66ac28b3c9fb5`. [oracle/frozen.json](oracle/frozen.json) records its checksums, 25 runtime exports, default export shape, 42 signal keys and order, declaration checksum, and bundle-size gates.

The rebuilt upstream baseline remained green with 186 browser test cases in Chrome and Firefox, plus its declaration and SSR checks.

The unpublished package is named `fingerprintjs-rust-experiment`, while its reported algorithm version remains `5.2.0`. All product logic, signal algorithms, callbacks, and schedulers are safe Rust. Product source contains no `unsafe` or `js!`. Compiler-owned runtime code, mechanically generated format wrappers, and test and benchmark JavaScript are the only JavaScript outside the oracle.

`LoadOptions.monitoring` retains its upstream default and type shape but is a no-op. Neither the default nor explicit monitoring path performs a network request.

| Deliverable | Location |
| --- | --- |
| Safe Rust implementation | [src](src) |
| Boundary distributions | [dist/boundary](dist/boundary) |
| Fiber distributions | [dist/fiber](dist/fiber) |
| Default boundary distributions | [dist](dist) |
| Frozen oracle manifest | [oracle/frozen.json](oracle/frozen.json) |
| Final differential and package results | [tests/results/final-current](tests/results/final-current) |
| Final scheduler benchmark and summary | [bench/results/schedulers-current](bench/results/schedulers-current) |
| Final bundle measurements | [bench/results/bundle-sizes-current.json](bench/results/bundle-sizes-current.json) |

## Parity

The direct signal audit compared every one of the 42 built-in sources in both execution orders. Chrome 151, Firefox 153.0.4, and Safari 26.6 produced 252 matching normalized comparisons and zero differences. The packaged contract suite then tested both schedulers with two alternating upstream-first and Rust-first captures per browser.

| Contract | Boundary | Fiber | Evidence |
| --- | --- | --- | --- |
| 42 source values, errors, keys, and order | Pass, 0 differences | Pass, 0 differences | [source audit](tests/results/source-audit.jsonl) |
| Full built-in components, visitor ID, confidence, version, and repeated get | Pass | Pass | [boundary](tests/results/final-current/boundary-differential), [fiber](tests/results/final-current/fiber-differential) |
| Hash, Unicode vectors, canonicalization, debug output, and lazy accessor descriptors | Pass | Pass | [boundary](tests/results/final-current/boundary-differential), [fiber](tests/results/final-current/fiber-differential) |
| Custom sync, async, one-stage, and two-stage sources, including failures and exclusions | Pass | Pass | [boundary](tests/results/final-current/boundary-differential), [fiber](tests/results/final-current/fiber-differential) |
| All 25 runtime exports, default export identity, source callback names and arities | Pass | Pass | [package boundary](tests/results/final-current/boundary-package.json), [package fiber](tests/results/final-current/fiber-package.json) |
| ESM, CJS, IIFE, UMD, SSR-safe imports, and `sideEffects: false` | Pass | Pass | Package and SSR checks |
| Exact attributed TypeScript declaration contract | Pass | Pass | SHA-256 `d20e2e6b3f60afb7679c1f87dd62800b7a7ce7155e86efbf6206190a1c185731` |
| Monitoring makes no network requests | Pass | Pass | `monitoring.default` and `monitoring.explicit` are empty in each browser result |

Durations were compared as distributions. Error comparisons retained the branch, constructor, name, message, own fields, and user-origin stack frames while removing generated implementation file and line locations. No other result was normalized.

Parity verdict: **Pass** for the specified local browser matrix.

## Performance

The final benchmark used the rebuilt minified artifacts, a fresh page for every sample, five warmups, randomized implementation order, and 30 measured runs per implementation, browser, and scenario. This produced 270 measured records in each browser. Every raw result records the exact upstream, boundary, and fiber artifact hashes. The scenarios were idle, calibrated main-thread CPU work, and DOM, layout, animation, and input work.

Total time is measured from module evaluation through `load`, immediate `get`, and lazy visitor-ID access. Values below are p50/p95 milliseconds.

| Browser | Scenario | Upstream | Boundary | Fiber |
| --- | --- | ---: | ---: | ---: |
| Chrome 151.0.7922.137 | Idle | 133.1 / 134.5 | 155.3 / 158.2 | 155.6 / 158.2 |
| Chrome 151.0.7922.137 | CPU | 134.5 / 142.9 | 153.8 / 161.7 | 161.2 / 176.3 |
| Chrome 151.0.7922.137 | DOM | 135.2 / 143.7 | 156.5 / 163.6 | 155.9 / 163.1 |
| Firefox 153.0.4 | Idle | 129.0 / 137.0 | 148.0 / 155.0 | 148.0 / 153.0 |
| Firefox 153.0.4 | CPU | 127.0 / 133.0 | 148.0 / 153.0 | 152.0 / 158.0 |
| Firefox 153.0.4 | DOM | 127.0 / 134.0 | 148.0 / 154.0 | 150.0 / 156.0 |
| Safari 26.6 | Idle | 119.0 / 121.0 | 138.0 / 143.0 | 141.0 / 144.0 |
| Safari 26.6 | CPU | 114.0 / 117.0 | 129.0 / 133.0 | 139.0 / 154.0 |
| Safari 26.6 | DOM | 118.0 / 120.0 | 138.0 / 143.0 | 141.0 / 149.0 |

The cross-browser and cross-scenario p95 geometric means show where the cost appears.

| Metric | Upstream | Boundary | Boundary change | Fiber | Fiber change |
| --- | ---: | ---: | ---: | ---: | ---: |
| Total latency | 131.12 ms | 151.31 ms | +15.4% | 156.61 ms | +19.4% |
| Module evaluation | 2.99 ms | 9.60 ms | +220.7% | 9.93 ms | +231.7% |
| Immediate `get` | 1.87 ms | 11.53 ms | +516.3% | 23.75 ms | +1169.7% |
| Lazy hash | 2.23 ms | 3.64 ms | +63.1% | 3.50 ms | +56.7% |
| `load` | 19.51 ms | 24.25 ms | +24.3% | 16.42 ms | -15.9% |
| Event-loop delay p95 | 34.34 ms | 47.79 ms | +39.2% | 40.94 ms | +19.2% |
| Maximum rAF gap, relative | Baseline | - | +68.5% | - | +14.6% |
| Input delay p95, relative | Baseline | - | +17.3% | - | +8.9% |
| Page-work progress, relative | Baseline | - | +4.0% | - | +8.0% |

Neither variant stayed within 5% for every primary p50, p95, and responsiveness metric. Boundary had 7 qualifying improvements and 150 of 234 protected comparisons over the regression limit; fiber had 25 qualifying improvements and 122 regressions. Both therefore failed the no-regression half of the gate. The 2,000-sample deterministic bootstrap details are in [summary.json](bench/results/schedulers-current/summary.json).

No benchmark record contained a `PerformanceObserver` long-task entry, so no new observed 50 ms long task or measured TBT was recorded. GC observer arrays were also empty. Chrome exposed heap data; its all-scenario heap-delta p95 was 163,975 bytes upstream, 6,959,368 bytes for boundary, and 6,271,988 bytes for fiber. Firefox and Safari did not expose comparable heap data.

Performance verdict: **Fail** for both schedulers.

## Scheduler comparison

The boundary artifact preserves the upstream two-pass `mapWithBreaks` lifecycle, source order, 16 ms `Date.now()` budget, `MessageChannel` task yield, non-awaited callback starts, and final `Promise.all` ordering. The fiber artifact runs the same public contract through the standalone Rust-future ready queue, uses an 8 ms source-boundary budget, and adds task checkpoints between independent DOM batches and resumable canvas phases. The checkpoints are cooperative and do not claim to preempt synchronous browser calls.

| Measure | Boundary | Fiber |
| --- | ---: | ---: |
| Total p95 geometric mean | 151.31 ms | 156.61 ms |
| Event-loop-delay p95 geometric mean | 47.79 ms | 40.94 ms |
| Change from upstream total p95 | +15.4% | +19.4% |
| Change from upstream event-loop delay | +39.2% | +19.2% |
| Change from upstream maximum rAF gap | +68.5% | +14.6% |
| Performance gate | Fail | Fail |

Fiber improved event-loop delay and maximum rAF gap relative to boundary by 14.3% and 32.0%, but its total p95 geometric mean was 3.5% slower and it still missed the upstream responsiveness gates. The predeclared selection rule says that when neither scheduler passes, boundary remains the semantic default. `dist/` therefore contains the boundary build, while both variants remain available for research.

## Bundle size

All figures include the compiled program, reachable runtime shim, generated facade, format wrapper, and license banner. Raw, gzip-9, and Brotli-11 bytes are shown in that order.

| Minified format | Upstream | Boundary | Fiber |
| --- | ---: | ---: | ---: |
| IIFE | 36,746 / 16,178 / 14,239 | 340,309 / 76,175 / 60,833 | 345,918 / 76,759 / 61,406 |
| UMD | 36,937 / 16,253 / 14,371 | 340,901 / 76,038 / 60,804 | 346,523 / 76,642 / 61,286 |
| Identically Terser-minified ESM | 35,423 / 15,432 / 13,749 | 340,329 / 76,188 / 60,831 | 345,938 / 76,775 / 61,315 |

The boundary IIFE is 9.26x the upstream raw size, 4.71x gzip size, and 4.27x Brotli size. Before Rollup and Terser, the boundary compiler program is 1,243,691 bytes and the fiber program is 1,266,541 bytes. Each artifact uses a 21,173-byte transitively extracted runtime shim and a 2,687-byte generated facade, so wrapper and shim bytes are not the main source of the gap. The exact final records are in [bundle-sizes-current.json](bench/results/bundle-sizes-current.json).

Bundle verdict: **Fail** for both schedulers and every gated metric.

## Compiler extensions

| Extension | Status | Verification |
| --- | --- | --- |
| Safe `js_bindings!` and `#[js_export]` macros | Pass | Macro tests and host fixture |
| Opaque `JsValue` and safe type, erase, and value operations | Pass | Host fixture and experiment build |
| Callback values with arities 0, 1, and 2 | Pass | Callback retention and invocation traces |
| Explicit-`this` invocation and construction | Pass | `Reflect.apply` and `Reflect.construct` traces |
| Synchronous host throw capture and thrown-value identity | Pass | `try_host` and `throw_value` traces |
| Promise-to-future suspension and future-to-Promise export | Pass | Resolve, reject, thenable, and concurrent-future traces |
| Safe local executor and true browser-task yield | Pass | Immediate poll, resumption, and `MessageChannel` traces |
| Transitive minimal-shim extraction | Pass | Four extractor tests and both production builds |

The compiler substrate is sufficient to express the library safely and exactly. The measured output shows that current Rust data, formatting, allocation, and generated-core costs remain too high for a size-sensitive JavaScript library.

## Reproduction

The experiment uses the existing pinned toolchains and the oracle's installed package tools. From `experiments/fingerprintjs-rust`:

```sh
npm run build
cargo +nightly-2026-07-27 test
cargo +nightly-2026-07-27 clippy --all-targets -- -D warnings
node tools/check-package.mjs --dist dist/boundary --allow-size-failure
node tools/check-package.mjs --dist dist/fiber --allow-size-failure
node tools/check-ssr.mjs --dist dist/boundary
node tools/check-ssr.mjs --dist dist/fiber
node tools/check-types.mjs --types dist/boundary/fp.d.ts
node tools/check-types.mjs --types dist/fiber/fp.d.ts
node tests/differential.mjs --candidate dist/boundary/fp.esm.js --output tests/results/final-current/boundary-differential
node tests/differential.mjs --candidate dist/fiber/fp.esm.js --output tests/results/final-current/fiber-differential
node bench/run.mjs --minified --output bench/results/schedulers-current
node bench/summarize.mjs --input bench/results/schedulers-current --output bench/results/schedulers-current/summary.json
node tools/measure-bundles.mjs --output bench/results/bundle-sizes-current.json
node tools/measure-esm.mjs --candidate dist/boundary/fp.esm.js --output bench/results/esm-boundary.json
node tools/measure-esm.mjs --candidate dist/fiber/fp.esm.js --output bench/results/esm-fiber.json
```

The package checker exits unsuccessfully without `--allow-size-failure`; that is the intended bundle verdict. Chrome, Firefox, and Safari WebDriver support must be available to reproduce browser runs.

The compiler regressions include `31_host`, `32_host_boundary`, `34_callback_ownership`, the host-value guards, and the direct host Future-driver test. The focused compiler checks are:

```sh
cd spike/rustc-codegen-js
SKIP_BUILD=1 ./scripts/test.sh 31_host
SKIP_BUILD=1 ./scripts/test.sh 32_host_boundary
SKIP_BUILD=1 ./scripts/test.sh 34_callback_ownership
SKIP_BUILD=1 ./scripts/test.sh guard_host_read
node --test scripts/host-future-driver.test.mjs
node scripts/minimal-shim.test.mjs
cargo test -p js-extern-macro
cargo test -p jsc-build
cargo test -p rustc_codegen_mires
```

## Limits, licensing, and security

The browser verdict covers the requested locally installed Chrome 151.0.7922.137, Firefox 153.0.4, and Safari 26.6 only. It does not claim the upstream package's older minimum-browser matrix because no remote BrowserStack credentials were used.

The implementation and every distribution retain the FingerprintJS MIT notice and the pinned declaration attribution. It is an unpublished derivative experiment and does not use FingerprintJS branding as a package identity.

The untracked `experiments/fingerprintjs/env.sh` was not read, copied, executed, or packaged. Its apparent BrowserStack and ngrok credentials still need separate rotation or revocation by their owner.

## Final verdict

The requested experiment is complete: all signals, lifecycle rules, dynamic APIs, exports, formats, declarations, and both schedulers were implemented and delivered. Behavioral parity passed with zero normalized differences in the requested browser matrix. Performance and bundle size failed by wide margins, so this should remain a research artifact. Boundary is the default because neither scheduler met the gate and it had the lower total p95. The shortest useful next step is compiler work on generated core, host-string, formatting, and allocation costs; a broader fiber framework is not justified by these results.

## Generic compiler optimizer follow-up (2026-08-12)

This follow-up evaluates the generic `rustc-codegen-js` optimizer against the boundary build. It does not change the original experiment verdict above. Optimizer `off` is the roadmap's new safe-executor baseline, not a byte-identical reconstruction of the pre-roadmap compiler. The optimizer remains opt-in because its size reduction did not produce a safe runtime win.

### Compatibility

`js-opt=off` and `js-opt=report` produced byte-identical raw programs, shims, facades, declarations, and ESM, CJS, IIFE, and UMD distributions, including every Terser output. Package namespace, SSR import, and TypeScript checks passed in off, report, and on modes. Off and on each produced zero normalized differences across two alternating captures in Chrome 151.0.7922.137, Firefox 153.0.4, and Safari 26.6; report inherits the off result from exact artifact identity. Monitoring remained network-free.

Evidence is in the [final package and differential results](tests/results/final-current). The compiler reports are [off](bench/results/optimizer-reports/off-boundary.json), [report](bench/results/optimizer-reports/report-boundary.json), and [on](bench/results/optimizer-reports/on-boundary.json).

### Size

| Output | Off | On | Change |
| --- | ---: | ---: | ---: |
| Raw compiler program | 1,243,691 | 1,031,767 | -17.04% |
| Identical-Terser ESM raw | 339,006 | 305,036 | -10.02% |
| Identical-Terser ESM gzip-9 | 75,415 | 73,374 | -2.71% |
| Identical-Terser ESM Brotli-11 | 60,383 | 59,030 | -2.24% |
| Shipped minified ESM, including notice | 340,329 | 306,359 | -9.98% |
| IIFE raw / gzip-9 / Brotli-11 | 340,309 / 76,175 / 60,833 | 306,356 / 74,144 / 59,536 | -9.98% / -2.67% / -2.13% |
| UMD raw / gzip-9 / Brotli-11 | 340,901 / 76,038 / 60,804 | 307,035 / 74,073 / 59,464 | -9.93% / -2.58% / -2.20% |

Terser retained the raw reduction, but gzip and Brotli retained only 2% to 3%. Every original bundle-size gate still failed. The exact ESM measurements are [off](bench/results/optimizer-esm-off.json), [report](bench/results/optimizer-esm-report.json), and [on](bench/results/optimizer-esm-on.json); the package measurements are in the [final package results](tests/results/final-current).

### Runtime

The final minified benchmark collected 810 measured records after five warmups: 270 in each browser. In the raw files, `boundary` means optimizer off and `fiber` means optimizer on; both point to boundary-scheduler artifacts. Each file records the exact artifact hashes.

| Cross-browser and scenario p95 geometric mean | Off | On | Change |
| --- | ---: | ---: | ---: |
| Module evaluation | 9.824 ms | 9.568 ms | -2.61% |
| `load` | 24.591 ms | 25.103 ms | +2.08% |
| Immediate `get` | 10.031 ms | 10.104 ms | +0.72% |
| Delayed `get` | 0.950 ms | 0.922 ms | -2.98% |
| Lazy hash | 3.646 ms | 3.663 ms | +0.48% |
| Repeated `get` | 0.790 ms | 0.732 ms | -7.41% |
| Total | 152.401 ms | 151.946 ms | -0.30% |
| Event-loop delay | 47.121 ms | 47.624 ms | +1.07% |
| Input delay | Baseline | - | -0.08% |
| Maximum rAF gap | Baseline | - | +7.77% |
| Workload progress | Baseline | - | +0.84% |

On had ten metrics with an improvement of at least 5% and a bootstrap confidence interval below parity, but 17 of 234 protected p50 and p95 comparisons regressed by more than 5%. The maximum-rAF-gap p95 geometric mean regressed 7.77%. No run reported an error or a new task of at least 50 ms. Chrome's all-scenario heap-delta p95 was 5,953,406 bytes for off and 6,846,598 bytes for on, versus 163,816 bytes upstream. The raw records and upstream-relative summary are in [optimizer-minified-final results](bench/results/optimizer-minified-final); [on-vs-off.json](bench/results/optimizer-minified-final/on-vs-off.json) contains the direct bootstrap comparison.

### Compiler conclusion

The emitted improvements come from conservative AST dead-code elimination and the compiler-generated monomorphic Future poller. Heap candidate analysis is report-only; heap emission does not exist. Native Promise-continuation eligibility selection and lowering are unimplemented and record an explicit fallback. Executable late IR and cross-crate specialized clones are also unimplemented. The current phase therefore passes exact parity and reduces raw size, but fails the no-runtime-regression gate and must not become the release default.

## Compiler optimization round follow-up (2026-08-14)

Five sequential compiler workstreams landed after the optimizer follow-up above: a link-time duplicate-item merge (`link-dedup`), native lanes for primitive `to_string`, a scalar-replacement pass (`sroa`) with opt-in application-crate MIR inlining, scoped-dispatch structurizing for multi-entry coroutine loops, and decline instrumentation plus call-boundary severing in the native-region planner. This section records their measured effect on this experiment; it does not change the original verdict, and every original bundle-size and runtime gate still fails.

### Size

Minified bytes are raw, gzip-9, and Brotli-11 in that order. The pre-round numbers are the 2026-08-14 object-format-19 baseline.

| Artifact | Pre-round | Post-round | Change |
| --- | ---: | ---: | ---: |
| Optimizer-on boundary minified ESM | 306,304 | 277,937 | -9.3% |
| Optimizer-on boundary ESM gzip-9 | 74,266 | 70,401 | -5.2% |
| Optimizer-off boundary minified ESM | 339,840 | 332,381 | -2.2% |
| Optimizer-on raw compiler program | 1,031,528 | 947,810 | -8.1% |

The post-round Brotli-11 sizes are 57,360 B for the on boundary IIFE, 57,422 B for the on boundary ESM, and 59,355 B for the off boundary ESM, against the 14,239 B upstream IIFE gate.

The dedup pass merges 268 duplicate monomorphizations in 77 groups, and the scoped-dispatch structurizer collapses the five async source whales from 337 dispatch states to 37 at suspension-count order. An optimizer-on fiber build now exists for the first time at 280,073 B minified ESM. An opt-in fast configuration (`--rust-opt-level 1 --mir-inline on`, optimizer on) trades 5.1 KB of gzip for the murmur hot loop falling from 2.7 times upstream to 1.55 times; the shipped default keeps the smaller bytes.

### Parity and runtime

Both the off and on boundary builds produced zero normalized differences in Chrome 151.0.7922.140, Firefox 153.0.4, and Safari 26.6 ([off](tests/results/round-final/off-differential), [on](tests/results/round-final/on-differential)). The full minified three-browser benchmark reran with the boundary slot holding the off build and the fiber slot holding the on build ([records](bench/results/round-bench/)). Cross-browser and cross-scenario p95 geometric means, with the 2026-08-13 optimizer-final run for comparison:

| Metric | Upstream | Off (was) | On (was) |
| --- | ---: | ---: | ---: |
| Total | 128.88 ms | 148.04 (152.40) | 146.79 (151.95) |
| Module evaluation | 2.63 ms | 9.51 (9.82) | 8.55 (9.57) |
| Immediate `get` | 1.51 ms | 9.44 (10.03) | 8.65 (10.10) |
| `load` | 18.32 ms | 23.52 (24.59) | 23.00 (25.10) |
| Lazy hash | 2.21 ms | 3.37 (3.65) | 3.59 (3.66) |
| Event-loop delay p95 | 29.23 ms | 41.04 (47.12) | 41.37 (47.62) |
| Maximum rAF gap | 17.76 ms | 27.29 | 25.77 |

The on build's total gap over upstream fell from 15.9 percent to 13.9 percent. This run recorded no input-delay, blocking-time, or workload-progress samples in any implementation including upstream, so its responsiveness evidence is the event-loop delay and rAF gap rows only. Part of one browser's window overlapped a background compile of the new fiber artifact; the randomized implementation order spreads any such noise across all three implementations.

### Remaining blockers, recorded precisely

The planner now records a reason on every planned-but-declined native variant. All 30 remaining String and Vec declines on this workload are regions whose every consumer chain ends in a host boundary, so a native variant would have no live consumer; the named next unlock is letting host boundaries admit immutable native strings. The formatting cluster splits into 125 KB behind `print_debug`'s float Display, 35 KB behind the panic message inside `str::contains`, and 37 KB of `format_args!` templates blocked on an exact owned-String construction primitive. The native-continuation analysis now shows three promise adapters eligible for a future native-async lowering, with the async whales blocked on borrowed state across suspension rather than custom polls.
