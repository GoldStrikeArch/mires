# The wasm32-LLVM vs. JS differential fuzzer

This fuzzer generates a single `#![no_std]` Rust program per seed, compiles it twice—once with LLVM for `wasm32-unknown-unknown` and once with `rustc_codegen_mires`—and runs both builds. Both runs must produce the same stdout and finish with the same exit class (normal exit or abort).

```sh
scripts/fuzz/run.sh 200                  # seeds 0..199, all three families
scripts/fuzz/run.sh 20 --seed 1000       # seeds 1000..1019
scripts/fuzz/run.sh 10 --family c        # destructors and the required abort
scripts/fuzz/run.sh 5 --keep             # keep build/fuzz/<seed>/ for every seed
scripts/fuzz/run.sh 50 --opt on          # optimizer enabled
```

`--opt off|report|on` appends `js-opt=` to `$JS_EXTRA_ARGS`. Because `js-opt` changes emitted items, each `--opt` setting uses its own keyed sysroot: `run.sh` selects that sysroot, builds it if it is missing, and prints the key in its banner so it never falls back silently to the default sysroot. (`--lane jir` is still accepted for script compatibility; JIR is the only backend lane in v5.)

## Why differential fuzzing helps

Every other test suite in this repository checks compiler output against hand-maintained `.expected` files. The fuzzer checks against LLVM as a reference oracle for Rust semantics: `i64::MIN.wrapping_neg()`, saturating float casts, `Drop` order, and `#[track_caller]` locations are all decided by LLVM. When the JavaScript build disagrees, the mismatch points at a compiler bug instead of a stale golden file.

For that comparison to stay meaningful, every generated program must have a single defined behavior. `gen.mjs` avoids undefined or unspecified constructs: arithmetic uses `wrapping_`, `checked_`, and `saturating_` methods, shifts go through `wrapping_shl` and `wrapping_shr`, slice indices stay in bounds, division uses `checked_div`, and values are never read after a move. The single intentional panic is at the end of family `c`, where both builds must print identical lines and abort.

## Files

| File | Role |
| --- | --- |
| `gen.mjs` | `node gen.mjs <seed> [--family a\|b\|c]` writes one program to stdout. Generation is a pure function of the seed, so any failure reproduces from the seed alone. |
| `shim.mjs` | `node shim.mjs <program.wasm>` runs the LLVM build under `WebAssembly.instantiate`, providing the same `js_log_*` imports that `runtime/shim.js` attaches to `__rt`. |
| `run.sh` | Runs the full loop: generate, compile with both backends, execute both builds, compare outputs, and shrink mismatches. |

## Seed families

| Family | Coverage |
| --- | --- |
| `a` | Integer arithmetic: wrapping, checked, and saturating ops; `i64` and `u128`; shifts, rotates, bit counting, byte and bit reversal; integer and saturating float casts; comparisons. |
| `b` | Aggregates: structs, tuple structs, payload enums, `Option`/`Result` combinators, `?`, arrays, subslices, iterator chains (`map`, `filter`, `fold`, `zip`, `rev`, `enumerate`), `FnMut` closures, and a `dyn Trait` call. |
| `c` | Destructors: printing `Drop` guards, nested scopes, moves, drops inside callees that take values by move, early returns past live guards, and a final `#[track_caller]` panic. |

When `--family` is omitted, the family is chosen by `seed % 3`, so a multi-seed run covers all three.

## Keeping both runs comparable

- Both builds compile the same source file, which declares the `extern "C"` print helpers from `examples/core-tests/prelude.rs` (`js_log_i32`, `js_log_i64`, `js_log_f64`, `js_log_bool`, `js_log_str`, `js_abort`) matching `runtime/shim.js`. The JS backend lowers foreign calls to `__rt.<name>(...)`; the LLVM build turns them into wasm imports (`-Clink-arg=--import-undefined`) supplied by `shim.mjs`.
- Both runtimes print through `console.log`. `shim.mjs` converts wasm values to the same JavaScript representations used by the JS backend: `i64` values become `BigInt` printed with `.toString()`, `bool` values arrive as `0`/`1` and print as booleans, and `&str` is read from exported wasm memory. Because the `wasm32-unknown-unknown` C ABI passes `&str` indirectly as a pointer to `{ptr, len}`, `shim.mjs` accepts both that form and the two-argument form.
- Both compilations pass the same `--remap-path-prefix`, so `#[track_caller]` panic locations print identical file paths.
- Exit status is compared by class (normal vs. aborted), because the exact non-zero exit code is a shim detail: `js_abort` throws in `runtime/shim.js` (Node exits 1) while `shim.mjs` exits 101.

## Failures and shrinking

When a seed diverges, `run.sh` preserves `build/fuzz/<seed>/` containing `prog.rs`, `prog.wasm`, `prog.js`, both outputs, both compiler logs, and `stdout.diff`. A line-drop shrinker then removes one line at a time, keeping each deletion whenever both compilers still accept the program and their outputs still differ, and writes the reduced program to `build/fuzz/<seed>/shrunk.rs`. `SHRINK_BUDGET=<n>` caps shrink attempts (default 400).

Seeds that fail to compile on either side are reported as `COMPILE ERROR` and preserved as well; they are counted separately from runtime mismatches because they indicate a generator bug or an unsupported construct.

## Prerequisites

- `scripts/build.sh` — builds the backend library and `jsc-link`.
- `scripts/build_sysroot.sh` — builds the default sysroot for the JS build.
- The `wasm32-unknown-unknown` target for the pinned nightly (`run.sh` installs it automatically if missing).
