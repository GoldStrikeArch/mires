# Standalone v2 restoration

> Historical assembly record. The current main-promotion scope and open work are in [round_1_integration_report.md](../round_1_integration_report.md); comparator restoration is recorded in [COMPARATORS.md](COMPARATORS.md).

The implementation base is the coherent September 7 producer-integration snapshot, not a per-file merge of experimental branches. The source archives remain untouched under `recovery/v2/`. Original checkpoint identities do not identify this renamed and repaired compiler.

## Installed structure

The rustc-loaded `backend` produces versioned semantic objects through `rcgjs-object`; the independent `jsc-link` lowers, optimizes, plans representations, emits JavaScript, and owns deployment manifests. `rustc-cleanup` verifies and adapts the pinned rustc drop/coroutine sources. `mires-build` is the stable build-script bridge. `mires-js` provides general host bindings and async helpers. No Topcoat framework crate, view macro, server, island application, or procedure runtime is a Cargo dependency. Historical template marker lowering and the small `view-abi`/`view-async` compatibility crates remain as internal legacy support; they are not required by the standalone example.

## Reconstruction changes

- Public backend artifact is `librustc_codegen_mires`; `jsc-link` and the IR format retain their established internal names.
- `mires-build` and `mires-js` replace the original package names; public build-root configuration is `MIRES_ROOT` with `JSC_SPIKE_ROOT` accepted for old scripts.
- The build bridge watches both the backend dylib and the independent linker. A linker-only change must invalidate the client build.
- Release profiles enable host-exception cleanup by default in the compiler, build bridge and sysroot key normalizer. Explicit `js-cleanup=off` wins, and is an opt-out, not equivalent semantics.
- A reachable function with cleanup work but no semantic JIR body is a deferred compile error. This reconstructs the later fail-closed requirement using existing zombie transport, rather than pretending the lost October implementation was recovered. Unreachable such functions do not poison a program.
- Later recovered packaging fixes bind Rollup inputs to compiler manifests, preserve program nontermination, and publish only the selected runtime closure.
- Standalone runtime tests no longer require a sibling v1 repository. Their original positive and negative behavior checks remain.
- `scripts/mires.mjs` builds a real-core Rust source into an ES module, minimal runtime, source map and verified artifact manifest. Its JSON receipt identifies the exact binaries and resulting files.

## Still missing from October

The final cleanup optimizer, later formatter recognition/size fixes, union strings and literal pinning, final constant pool and trailing-argument passes, and final pointer/array repairs are not all present. They must be reconstructed with regression tests, not inferred from an earlier pass count. `recovery/v2/LATE-STAGE-WINS.md` lists the historical targets. Known dev12 audit probes are preserved in `recovery/v2/reconstruction/from-conversation/`; a passing older corpus is not evidence that these later cases pass.

## Experiments

`experiments/fingerprintjs` is the recovered upstream OSS source. The three `fingerprintjs-rust*` trees are the recovered Rust ports. `experiments/browser-workloads` holds ten reduced Rust workloads. These are not full ports of Marked/MiniSearch/etc. The browser corpus tools now use the standalone root and hash compiler input directories rather than the old monorepo path.

`experiments-pro/fp-pro` contains a partial original source tree and the exact frozen self-contained reference module/declarations. Other original Pro distribution formats and the full source checkout are missing. Rust Pro ports and shared tools are restored beside it. Pro material is ignored and must not be accidentally published. Historical port parity, size, and browser claims are not automatically requalified.

All Dart/Scala/Kotlin/ReScript/etc. port restoration remains out of this change. No benchmark timing collected on this shared development machine is release timing evidence.

## Current verification and next engineering work

`VERDICT.md` and `v2-assembly-verification.json` record the assembled candidate, including four strict later-regression failures and the incomplete optimized Pro link. Fix zero-sized-array destruction and aggregate-place identity before treating this as faithful production Rust. Restore cleanup-preserving optimization and avoid repeated whole-program name/materialization work during exact-cost trials; the large cleanup-on Pro build exposed this cost. Recover formatter and string-scaling improvements with measured full-package and operation-count tests rather than targeting a historical size without matched semantics.

The corpus builder now fails its command when the build receipt fails, and its runtime driver rejects a failed input-build receipt. A fresh stable-source rerun and an injected-failed-receipt negative control both ran. Node tools work using the single root dependency install; no local node_modules symlink is required for the verified workflows.
