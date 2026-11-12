# mires — standalone Rust-to-JavaScript compiler

`mires` compiles monomorphized Rust MIR, including real `core` and `alloc`, into JavaScript. This workspace now contains the recovered **v2 producer / semantic-object / independent-linker architecture**, rather than the earlier text-first backend.

This is a working development reconstruction based on the coherent September 7 v2 snapshot, with selected later safety and packaging requirements restored. **It is not a byte-exact October dev13 recovery or a production-qualified release.** The remaining optimization and semantic gaps are listed in [the restoration record](docs/V2-RESTORATION.md) and [current verification results](VERDICT.md). Historical archived verdicts are kept separately.

## Current optimization-restoration checkpoint

Selected round-1 wins are integrated locally; cleanup remains enabled. See [the comprehensive promotion and remaining-work report](round_1_integration_report.md) and [verification receipt](docs/round1-promotion-verification.json). The audited cleanup-edge-removal pass and failed/unverified size candidates are intentionally held back. Known correctness failures remain visible; this is not production admission.

## Preserved restoration work

The source, pending patches and verifier programs from both restoration rounds are preserved in [the research archive](research/v2-restoration/rounds-1-2/README.md). Read [WORKTREE-CLEANUP-HANDOFF.md](WORKTREE-CLEANUP-HANDOFF.md) before removing old worktrees. It distinguishes public reconstruction patches from private raw evidence and records the fresh combined-main checks.

## Compile and run Rust

macOS/Linux, Rustup and Node.js 24+ are required. `rust-toolchain.toml` pins the backend's nightly and rustc development components. Application build scripts can remain on stable Rust.

```sh
npm ci --ignore-scripts
bash scripts/build.sh --locked
node scripts/mires.mjs build examples/standalone/hello.rs --out build/hello.mjs
node --input-type=module -e 'import {sum_squares, greeting} from "./build/hello.mjs"; console.log(greeting(), sum_squares(5))'
```

The example uses `Vec`, iterator adapters, and exported Rust functions. It prints `Hello from Rust 30`. No Topcoat framework, view macro, server or DOM runtime is needed.

The compiler command writes an ES module, its reached-only runtime module, an authored source map, a linker-owned artifact manifest, and a build receipt with source/binary/artifact hashes. `node scripts/mires.mjs --help` describes profile and dependency options.

Input is currently a `#![no_std]` library with `core`/`alloc`, exported functions and a panic handler, not an arbitrary `std` desktop executable. JavaScript/browser integration is explicit through `mires-js`, `#[js_extern]` and `js!`.

### Stable Cargo build-script integration

```sh
cargo +stable run --manifest-path examples/standalone-consumer/Cargo.toml
```

The example's stable `build.rs` uses `mires_build::BuildConfig` to compile a separate Rust client using the pinned backend. `MIRES_ROOT` selects another checkout; `JSC_SPIKE_ROOT` remains accepted for old experiment drivers. Both compiler binaries are watched for incremental invalidation.

## Source layout

| Directory | Role |
| --- | --- |
| `backend/` | `rustc_codegen_mires`: rustc-facing semantic producer |
| `rustc-cleanup/` | Hash-verified adaptations of the pinned rustc drop/coroutine passes |
| `rcgjs-object/` | Rustc-independent object format, semantic IR, identities, origins, contracts |
| `jsc-link/` | Independent linker, SSA, analyses, optimizer, representation/ABI planner, JavaScript emission |
| `structurizer/` | Control-flow graph to structured JavaScript |
| `mires-build/` | Stable build-script bridge |
| `mires-js/` | General host values, JavaScript bindings, collections and future interoperation |
| `js-extern-macro/`, `js-macro/` | Typed external bindings and explicit JavaScript expressions |
| `runtime/` | Canonical runtime, extracted per artifact |
| `view-abi/`, `view-async/` | Retained small compatibility ABI/executor crates; no Topcoat framework dependency |
| `scripts/`, `examples/` | Build drivers, compiler fixtures and standalone examples |
| `experiments/` | Recovered OSS reference, Rust ports and reduced workload corpus |
| `experiments-pro/` | Private recovered Pro reference, Rust ports and historical tools; ignored by Git |
| `recovery/v2/` | Unmodified snapshots, content-addressed variants, evidence and recovery manifests |

Legacy template-marker lowering remains internally for compatibility with recovered code; the Topcoat UI implementation and framework crates are not workspace members or dependencies. Removing all legacy marker support is separate from restoring a verified generic compiler.

## Profiles and cleanup

`release-size`, `release-balanced`, and `release-speed` select the JIR lane and optimizer, an ES2018 syntax ceiling, and **host-exception cleanup on**. `js-cleanup=off` is an explicit opt-out and changes recoverable-exception semantics. ES2018 is a syntax ceiling, not a promise that every reached host capability exists in an ES2018 engine.

The primitive build drivers remain available:

```sh
SKIP_BUILD=1 JS_EXTRA_ARGS='js-profile=release-balanced' bash scripts/build_sysroot.sh
JS_EXTRA_ARGS='js-profile=release-balanced' bash scripts/compile_core.sh source.rs build/program.js
```

These lower-level commands emit compiler artifacts; use `scripts/mires.mjs` for a module with a verified runtime package. An unsupported operation must be diagnosed, not silently replaced. Reachable functions whose semantic body is unavailable cannot fall back to text that drops required cleanup.

## Verify the compiler

```sh
SKIP_BUILD=1 JS_EXTRA_ARGS='js-lane=jir js-opt=on' bash scripts/build_sysroot.sh
SKIP_BUILD=1 JS_EXTRA_ARGS='js-lane=jir js-opt=on' bash scripts/test.sh --require-core
bash scripts/check.sh
npm run test:standalone
SKIP_BUILD=1 npm run test:cleanup
bash scripts/cleanup-admission-test.sh
```

`check.sh` requires a nonempty real compiler corpus, then checks formatting, Clippy, Rust tests, runtime tests and cache identities. The cleanup tests exercise ordinary/active host throws, drop order and thrown-value identity; the admission test requires a reachable unencodable cleanup dependency to be rejected and an unreachable one to remain harmless.

## Experiments

See [experiments/README.md](experiments/README.md). The OSS reference can be rebuilt from its original TypeScript source. Rust ports build through the active standalone compiler and use manifest-bound packaging.

```sh
npm run build:oss-reference
npm run build:corpus
npm run test:corpus
```

Fingerprint Pro's exact self-contained 4.1.4 reference module and declarations are present locally, but its original source checkout is incomplete. Its Rust ports can be rebuilt independently. Do not mistake an API/import check for full browser parity, and do not publish private recovered material.

The later October cleanup/formatter optimizations, union-string work, constant pooling, trailing arguments and remaining memory-model repairs are not all reimplemented. Restored historical measurements are targets and evidence about their original inputs—not performance claims for this compiler. Other-language comparison ports have not been restored.

## Other-language compiler comparisons

The restored ReScript, Melange, js_of_ocaml, Scala.js, Dart2JS, Kotlin/JS, and Elm implementations are available for both OSS and private Pro experiments. See [the comparator guide](docs/COMPARATORS.md) for installation, build/check commands, verified scope, and production-variant selection.
