# mires — standalone Rust-to-JavaScript compiler

`mires` compiles monomorphized Rust MIR, including real `core` and `alloc`, into JavaScript using a JIR-only semantic-object producer and independent whole-program linker/optimizer (`rustc_codegen_mires` + `jsc-link`).

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
| `backend/` | `rustc_codegen_mires`: rustc-facing JIR semantic producer |
| `rcgjs-object/` | Rustc-independent object format, semantic IR, identities, origins, contracts |
| `jsc-link/` | Independent linker, SSA, analyses, optimizer, representation/ABI planner, JavaScript emission |
| `structurizer/` | Control-flow graph to structured JavaScript |
| `mires-build/` | Stable build-script bridge |
| `mires-js/` | General host values, JavaScript bindings, collections and future interoperation |
| `js-extern-macro/`, `js-macro/` | Typed external bindings and explicit JavaScript expressions |
| `runtime/` | Canonical runtime, extracted per artifact |
| `scripts/`, `examples/` | Build drivers, compiler fixtures and standalone examples |
| `experiments/` | OSS benchmarks, library matrix, and comparator ports |
| `experiments-pro/` | Private Pro benchmarks and ports; ignored by Git |

## Profiles and unwind semantics

`release-size`, `release-balanced`, and `release-speed` select the JIR lane and optimizer with an ES2018 syntax ceiling. `mires` runs strictly in `JSC_PANIC=unwind` mode (`targets/mires32-unwind.json`) so rustc's own MIR unwind edges carry host-exception cleanup (`js-cleanup=off`). Rust panics still abort. See [CONTRACT.md](CONTRACT.md), [JIR.md](JIR.md), and [OPTIMIZER.md](OPTIMIZER.md) for ABI rules and limits. Set `RCGJS_WITNESS_CHECK=1` to enable independent optimization-witness verification.

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

The useful October capabilities have been partly recovered and partly replaced by newer implementations. Union-string policy, broad immutable pooling/trailing-argument work, and remaining memory-model repairs are not all restored. Historical measurements remain scoped to their original inputs. All seven other-language comparator families have been restored; their browser/product qualification is distinct from successful offline builds and kernel checks.

## Other-language compiler comparisons

The restored ReScript, Melange, js_of_ocaml, Scala.js, Dart2JS, Kotlin/JS, and Elm implementations are available for both OSS and private Pro experiments. See [the comparator guide](docs/COMPARATORS.md) for installation, build/check commands, verified scope, and production-variant selection.
