# mires

A Rust-to-JavaScript compiler: an out-of-tree rustc codegen backend that lowers
monomorphized MIR to JavaScript, with the pieces a compiled program needs around
it. The name is MIR + ES.

It compiles real `core` and `alloc`. A `#![no_std]` crate becomes a readable,
minifiable ES module or script with source maps, whole-program dead-code
elimination, `#[js_extern]` bindings to JavaScript libraries, and an executor for
Rust futures. The template ABI in `view-abi` is what a UI framework's view macro
can lower to so that a server-rendered page hydrates in the browser; that
lowering lives with the framework, not here.

## Layout

| path | what |
|---|---|
| `backend/` | the codegen backend, `rustc_codegen_mires`, loaded with `-Zcodegen-backend` |
| `structurizer/` | CFG to structured control flow (a relooper), no rustc dependency |
| `mires-build/` | compiles a client crate to JavaScript from a Cargo `build.rs` |
| `js-macro/` | `js!{}`, an inline JavaScript expression with captured Rust values |
| `js-extern-macro/` | `#[js_extern]` on `extern "C"` blocks: typed bindings to a JavaScript library |
| `view-abi/` | the template ABI between a view lowering and the backend |
| `view-async/` | the executor that polls Rust futures inside a compiled program |
| `mires-js/` | host `Map`/`Set` exposed to Rust as `HashMap`/`HashSet` |
| `runtime/` | `shim.js`, the `__rt` object every compiled program calls, and the vendored DOM runtime bundle |
| `patches/` | the two patches applied to the toolchain's `rust-src` before the sysroot is built |
| `scripts/` | build and test drivers (see below) |
| `examples/` | the fixture suites |
| `contract/` | the dom-expressions contract: what the template emission and the DOM runtime must reproduce, extracted from pinned upstream sources |
| `bench/` | the krausest js-framework-benchmark harness and a Melange comparison |
| `demo/` | a `#![no_core]` counter for the browser |
| `CONTRACT.md` | the interface spec: compilation command, options, value model, zombies, modules, chunks |
| `VERDICT.md` | the gate record: what is proven, what is out |

## Setup

The backend is a rustc plugin and loads only into the nightly `rust-toolchain.toml`
pins. rustup installs it on first use in this directory, with the `rust-src`,
`rustc-dev`, and `llvm-tools` components. macOS and Linux only.

```sh
scripts/build.sh                            # the backend, release
JS_EXTRA_ARGS='' scripts/build_sysroot.sh   # core, compiler-builtins and alloc compiled by it, into build/sysroot
```

Both take minutes. Everything that compiles a crate asks for the pinned nightly
by name, so `rustup default` is never touched.

## Tests

```sh
cargo test --workspace      # the crates' own tests
scripts/test.sh             # examples/tests (no_core) and examples/core-tests (real core), run under node
scripts/emit-test.sh        # goldens of the emitted JavaScript; UPDATE_EXPECT=1 rebaselines
scripts/module-test.sh      # script and ESM output agree item for item
scripts/extern-test.sh      # #[js_extern]
scripts/async-test.sh       # the executor
scripts/dts-test.sh         # the .d.ts output
scripts/matrix.sh           # test.sh under every emit-option mode
```

Node 24 or later is needed. The contract harness has its own setup, in
[`contract/README.md`](contract/README.md).

## Compiling a crate

`scripts/compile_core.sh <src.rs> <out.js>` compiles one `#![no_std]` root file
against the sysroot. From a Cargo build script, `mires-build` does the same with
rerun tracking, chunking, and setup checks:

```toml
[build-dependencies]
mires-build = { path = "../mires/mires-build" }
```

```rust
mires_build::BuildConfig::new("client/app.rs").render().unwrap();
```

A crate that uses a proc macro is expanded by plain rustc first, because the
backend cannot run one: `.js_extern(true)` for `#[js_extern]`, `.proc_macro(..)`
for a macro built elsewhere, `.view_abi(true)` or `.rlib(..)` for a crate the
backend compiles to an rlib first. `MIRES_ROOT` points a build at a checkout
other than the one `mires-build` was compiled from.

## Options

Backend options travel through `-Cllvm-args`, for example `js-minify=on` or
`js-modules=esm`. An option that changes how an item is spelled has to match
between the sysroot and the crate, which is why `build/sysroot/.js-args` records
the set the sysroot was built with. `CONTRACT.md` lists every option.
