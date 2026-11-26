# mires

`mires` is a Rust-to-JavaScript compiler implemented as a `rustc` codegen backend (`rustc_codegen_mires`) paired with a whole-program linker and optimizer (`jsc-link`). It compiles monomorphized Rust MIR, including the real `core` and `alloc` libraries, directly into JavaScript ES modules with no WebAssembly runtime and no linear memory heap.

## How it works

Compilation splits cleanly across two executables so `rustc` handles front-end analysis, borrow checking, monomorphization, and per-crate MIR encoding, while `jsc-link` sees the entire program at once for dead-code elimination, representation planning, optimization, and JavaScript emission.

```text
  Rust sources (#![no_std] + core + alloc + deps)
                         |
                         v
  +-------------------------------------------------------------------+
  | rustc  (-Zcodegen-backend=librustc_codegen_mires, -Cpanic=unwind) |
  |   * Parses, typechecks, borrow-checks, and monomorphizes MIR      |
  |   * Targets targets/mires32-unwind.json (32-bit pointers/usize)   |
  |   * Encodes each item's MIR + type/origin tables into JIR-S       |
  +-------------------------------------------------------------------+
                         |
                         |  .o / .rlib files  (RCGJS\0 binary format)
                         |  + version-4 link manifest
                         v
  +-------------------------------------------------------------------+
  | jsc-link  (whole-program linker, optimizer, and JS emitter)       |
  |                                                                   |
  |   Phase A: Load objects -> verify headers/schemas -> merge        |
  |            duplicate items -> reachability walk (DCE)             |
  |                                                                   |
  |   Phase B: JIR-S -> JIR-R (SSA) -> interprocedural analyses       |
  |            (effects, escape, aliasing, range, carrier planning)   |
  |            -> JIR optimizer passes + fail-closed witness checks   |
  |                                                                   |
  |   Phase C: Control-flow structurizer -> JS AST lowering ->        |
  |            item/local minification, ES module chunking,           |
  |            printing (ES2018 / latest), and source maps            |
  +-------------------------------------------------------------------+
                         |
                         v
  +-------------------------------------------------------------------+
  | Output artifacts                                                  |
  |   app.mjs          ES module with exported Rust functions/statics |
  |   app.runtime.mjs  Trimmed __rt shim (only helpers actually used) |
  |   app.mjs.map      VLQ source map back to original .rs lines      |
  |   *.js (optional)  Code-split chunks + shared module (js-chunk=)  |
  +-------------------------------------------------------------------+
```

### Inside `jsc-link`

Because `jsc-link` has no dependency on `rustc` internals (it reads self-contained `rcgjs-object` archives), whole-program passes can inspect every caller and callee before choosing JavaScript representations:

```text
  .o / .rlib objects
         |
         v
  +--------------------+     +-----------------------+     +---------------------+
  | Summary & Merge    | --> | Whole-Program DCE     | --> | SSA Build (JIR-R)   |
  | * header/ABI check |     | * walk from exports   |     | * RPO blocks + phis |
  | * min-hash dedup   |     | * drop dead items     |     | * cell promotion    |
  +--------------------+     +-----------------------+     +---------------------+
                                                                      |
         +------------------------------------------------------------+
         v
  +----------------------------------------------------------------------------+
  | Whole-Program Analysis & Representation Planning                           |
  | * Effect & escape analysis     * Native carriers (NativeVec, ByteStr)      |
  | * Enum repr (null niche / tag) * Flat direct-call ABI (multi-arg / tuple)  |
  | * SoA Vec layout & bounds/range proofs   * Landing-pad pruning             |
  +----------------------------------------------------------------------------+
                                       |
                                       v
  +----------------------------------------------------------------------------+
  | JIR Optimization Passes  (verified by Func::verify + optional witnesses)   |
  | * Inlining & single-use folding  * Copy/const propagation & GVN            |
  | * Slice-iterator cursors         * Dead store & dead code elimination      |
  +----------------------------------------------------------------------------+
                                       |
                                       v
  +--------------------+     +-----------------------+     +---------------------+
  | Structurizer       | --> | JS AST & Peepholes    | --> | Print & Package     |
  | * CFG -> if/while/ |     | * SROA & expression   |     | * ES2018 or latest  |
  |   switch/try-catch |     |   inlining            |     | * minify + chunks   |
  | * dispatch fallback|     | * link_dedup aliases  |     | * .map + .d.ts      |
  +--------------------+     +-----------------------+     +---------------------+
```

## How Rust maps to JavaScript

`mires` represents Rust values directly as JavaScript values so compiled code interops cleanly with JS objects and avoids shipping a Wasm memory allocator or UTF-8/UTF-16 trampoline layer at every call boundary.

```text
  Rust value                     Baseline JS representation
  ----------------------------   --------------------------------------------
  i8..i32, u8..u32, isize/usize  JS Number (masked: |0, >>>0, Math.imul)
  i64, u64, i128, u128           JS BigInt (BigInt.asIntN / asUintN)
  f32, f64, bool, char           JS Number, Boolean, Unicode scalar Number
  struct { a, b }                JS Object { a, b }  (aliased by identity)
  (T, U), [T; N]                 JS Array  [v0, v1, ...]
  fieldless enum                 Discriminant Number (#[repr(int)]) or String
  payload enum                   { TAG: "Variant", ... } (or null-niche/int)
  &mut x (primitive local)       Slot { buf: [val], off: 0 }  ->  p.buf[p.off]
  &[T] / &mut [T]                Slice { buf, off, len } (subslice shares buf)
  &dyn Trait                     Fat pointer { ptr, meta: vtable_array }
  &str                           JS String (dev) or ByteStr (release profiles)
  Vec<T> / String                Heap array block or native JS Array (NativeVec)
  ZST / ()                       undefined
```

Key design choices:

- **Element-indexed pointer slots (`{ buf, off }`).** A pointer to an element inside an array or boxed local stores the backing JS container and an element index (`p.buf[p.off]`). Pointer arithmetic produces a new slot, and dereferencing is an ordinary assignable JS place.
- **Typed heap blocks.** `__rt.alloc` creates a JS array block that specializes on its first `*mut u8 -> *mut T` cast. `dealloc` clears and poisons freed blocks so use-after-free casts fail loudly.
- **Whole-program specialization.** When `js-opt=on` is active, `jsc-link` replaces baseline representations wherever escape and alias analyses prove it safe: `Vec<T>` becomes a plain JS array (`NativeVec`) or parallel arrays (`SoA`), `&str`/`String` operates on UTF-8 byte views (`js-str=bytes`), `Option<&T>` uses a null niche, and private functions flatten struct arguments and return values across direct calls.
- **Unwind cleanup vs. Rust panics.** Every build compiles for `targets/mires32-unwind.json` with `-Cpanic=unwind`, so `rustc` keeps unwind edges in MIR. When a JavaScript host function throws an exception, lowered `try`/`catch` handlers run live Rust destructors (`Drop`) and rethrow the original JS exception unchanged. Rust panics still call `#[panic_handler]` and abort immediately via `__rt.js_abort` without running cleanup handlers.

## Quick start

Prerequisites: macOS or Linux, Rustup, and Node.js 24+. [`rust-toolchain.toml`](rust-toolchain.toml) pins the nightly toolchain and `rustc-dev` components used to build the compiler; downstream Cargo crates using `mires-build` can build on stable Rust.

```sh
npm ci --ignore-scripts
bash scripts/build.sh --locked
node scripts/mires.mjs build examples/standalone/hello.rs --out build/hello.mjs
node --input-type=module -e 'import {sum_squares, greeting} from "./build/hello.mjs"; console.log(greeting(), sum_squares(5))'
```

[`examples/standalone/hello.rs`](examples/standalone/hello.rs) is a `#![no_std]` crate using `alloc::vec::Vec` and iterator adapters:

```rust
#![no_std]
#![no_main]
extern crate alloc;
use alloc::vec::Vec;

#[unsafe(no_mangle)]
pub fn sum_squares(n: u32) -> u32 {
    let values: Vec<u32> = (0..n).map(|n| n * n).collect();
    values.iter().copied().sum()
}

#[unsafe(no_mangle)]
pub fn greeting() -> &'static str { "Hello from Rust" }

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop {} }
```

Running the Node command prints `Hello from Rust 30`.

`scripts/mires.mjs` emits:
- `build/hello.mjs` — the compiled ES module
- `build/hello.runtime.mjs` — a trimmed `__rt` module containing only the helpers reached by the program
- `build/hello.mjs.map` — a source map
- an artifact manifest and build receipt with source/output SHA-256 digests

Run `node scripts/mires.mjs --help` for profile and dependency flags (`--profile`, `--extern`, `--search`, `--skip-build`).

### Using `mires-build` from a stable Cargo crate

```sh
cargo +stable run --manifest-path examples/standalone-consumer/Cargo.toml
```

That example's `build.rs` calls `mires_build::BuildConfig` to drive the pinned backend from a normal stable Cargo build and emits Cargo rerun directives for both compiler binaries. Set `MIRES_ROOT` to point at another checkout (`JSC_SPIKE_ROOT` is still accepted by older experiment scripts).

### Driving `build_sysroot.sh` and `compile_core.sh` directly

```sh
SKIP_BUILD=1 JS_EXTRA_ARGS='js-profile=release-balanced' bash scripts/build_sysroot.sh
JS_EXTRA_ARGS='js-profile=release-balanced' bash scripts/compile_core.sh source.rs build/program.js
```

Sysroots are cached under `build/sysroot-cache/` and keyed by emit-affecting backend options, so switching profiles or flags rebuilds only when needed.

## JavaScript and browser interop

Three crates cover calling into JavaScript and browser APIs:

- **`mires-js`** — typed wrappers for host values (`JsValue`, `JsString`, `JsArray`, `JsUint8Array`), host-backed `HashMap`/`HashSet` over JS `Map`/`Set`, callback registration, and `Future` <-> `Promise` interop.
- **`#[js_extern]` (`js-extern-macro`)** — declares zero-marshalling bindings for constructors (`new`), methods, properties (`get`/`set`), indexed collections (`index`), and static/global calls, emitting direct JS syntax and ES module imports.
- **`js!{}` (`js-macro`)** — embeds inline JavaScript expressions with lexically analyzed Rust variable captures for ad-hoc interop (such as options objects with hyphenated keys like `"content-type"`).

See [`contract/JS-EXTERN.md`](contract/JS-EXTERN.md) and [`CONTRACT.md`](CONTRACT.md) for the exact FFI and byte-buffer copy contracts.

## Profiles

Pass `-Cllvm-args=js-profile=<name>` (or `--profile <name>` to `scripts/mires.mjs`). Profiles expand before explicit flags, so any flag you pass after or before a profile overrides it.

| Profile | Minification | `js-str` | Syntax ceiling | Intended use |
| --- | --- | --- | --- | --- |
| `dev` | `off` | `utf16` | `latest` | Fast builds and readable JS output |
| `report` | `off` | `utf16` | `latest` | Optimizer analysis without changing emitted code |
| `release-size` | `on` | `bytes` | `2018` | Smallest transfer size (item/local renaming, compact print, conservative inlining) |
| `release-balanced` | `on` | `bytes` | `2018` | Balanced bundle size and execution speed |
| `release-speed` | `locals` | `bytes` | `2018` | Highest throughput (keeps item names for profiling; enables aggressive inlining, SoA `Vec`, bounds-check elimination, zero-offset slices) |

`js-ecma-target=2018` constrains emitted JS syntax (for example, printing `BigInt("...")` calls instead of `1n` literals); runtime globals such as `BigInt`, `globalThis`, `queueMicrotask`, and `TextEncoder` still depend on the host engine. Set `RCGJS_WITNESS_CHECK=1` to run independent proof-witness checks on optimizer transforms during linking.

## Supported Rust and known boundaries

- **Supported:** `#![no_std]` crates with real `core` and `alloc`; generics, traits, `&dyn Trait` vtables, closures (`Fn`/`FnMut`/`FnOnce`), `Drop` and unwind cleanup on host throws, `Box`, `Vec`, `String`, `Rc<[T]>`, `Arc<[T]>`, `core::fmt` / `Display` (including floats), `async`/coroutines, cyclic statics, source maps, and ES module code splitting (`js-chunk=`).
- **Out of scope by design:** reinterpreting a buffer at a different element granularity (`as_chunks`, `align_to`, `hashbrown`'s dual-granularity layout — use `mires_js::collections::HashMap`), non-slice unsized heap tails (`Rc<str>`, `Arc<dyn Trait>`), writing through `&mut str`, `ptr_mask`, `catch_unwind`, `f16`/`f128`, `simd_*`, and multi-threaded atomics (atomics lower to single-threaded operations).
- **Fail-closed diagnostics:** any unsupported construct that stays reachable after dead-code elimination fails the link with an error naming the construct and its reachability chain from the entry point.

## Repository layout

| Directory | Purpose |
| --- | --- |
| [`backend/`](backend/) | `rustc_codegen_mires`: `rustc` codegen backend that encodes MIR into JIR object files |
| [`rcgjs-object/`](rcgjs-object/README.md) | `rustc`-independent object format, JIR schemas, JS AST, printer, minifier, source maps, and options |
| [`jsc-link/`](jsc-link/README.md) | Whole-program linker, SSA builder, analyses, optimizer passes, ABI/layout planners, and JS emission |
| [`structurizer/`](structurizer/) | Converts control-flow graphs into structured JavaScript control flow |
| [`mires-build/`](mires-build/) | Stable Rust `build.rs` wrapper around `rustc` and `jsc-link` |
| [`mires-js/`](mires-js/) | Host value wrappers (`JsValue`, `JsString`, `JsArray`, `JsUint8Array`), bindings, collections, and Promise interop |
| `js-extern-macro/`, `js-macro/` | Proc macros for `#[js_extern]` bindings and inline `js!{}` blocks |
| `runtime/` | Canonical runtime shim (`runtime/shim.js`), trimmed per build |
| `patches/` | Small `core`/`alloc` patches (scalar `memchr`, non-formatting panic stubs, native string/vec markers, unwind personality) |
| `scripts/`, `examples/` | Build drivers, test suites, differential fuzzer, and standalone examples |
| [`experiments/`](experiments/README.md) | OSS benchmarks, 9-compiler library matrix, and comparator ports |
| `experiments-pro/` | Local-only private benchmarks (ignored by Git) |

## Verification

```sh
bash scripts/check.sh
SKIP_BUILD=1 JS_EXTRA_ARGS='js-lane=jir js-opt=on' bash scripts/test.sh --require-core
npm run test:standalone
SKIP_BUILD=1 npm run test:cleanup
```

`scripts/check.sh` runs formatting checks, Clippy (`-D warnings`), workspace unit and property tests with `RCGJS_WITNESS_CHECK=1`, runtime shim tests, sysroot cache-key checks, landing-pad pruning tests, and multi-chunk linking tests.

## Documentation

- [`CONTRACT.md`](CONTRACT.md) — JavaScript value representation, pointers, allocation, modules, chunks, coroutines, callbacks, and FFI contracts
- [`JIR.md`](JIR.md) — JIR body normalization and duplicate/alias merging
- [`OPTIMIZER.md`](OPTIMIZER.md) — Link-time optimizer passes, profiles, and sidecar reports
- [`jsc-link/README.md`](jsc-link/README.md) — Linker phases, manifest format, and crate structure
- [`rcgjs-object/README.md`](rcgjs-object/README.md) — Object file sections, JIR schemas, and baseline carrier model
- [`contract/JS-EXTERN.md`](contract/JS-EXTERN.md) — `#[js_extern]` call shapes and descriptor grammar
- [`examples/core-tests/README.md`](examples/core-tests/README.md) — Core test suite coverage, sysroot patches, and known compiler limits
- [`scripts/fuzz/README.md`](scripts/fuzz/README.md) — Differential fuzzer comparing LLVM `wasm32` output against `mires`
- [`docs/COMPARATORS.md`](docs/COMPARATORS.md) — Running the eight other-language comparator builds

## Experiments

See [`experiments/README.md`](experiments/README.md) and [`experiments/library-matrix/README.md`](experiments/library-matrix/README.md) for the OSS reference builds, the 15-library (+ Explorer, `pilot`, `floor`) cross-language parity/size/timing harness, and comparator ports across ReScript, Melange, js_of_ocaml, Scala.js, Dart2JS, Kotlin/JS, Elm, and GopherJS.
