# The wasm32-LLVM vs. JS differential fuzzer

This fuzzer generates a single defined-behavior `#![no_std]` Rust program per seed, compiles it with LLVM for `wasm32-unknown-unknown` and with `rustc_codegen_mires` across one or more compiler profiles, and runs every build under Node. Every JS build must produce the exact same stdout and finish with the same exit class (normal exit or abort) as LLVM.

```sh
scripts/fuzz/run.sh 200                                  # seeds 0..199, all 6 families
scripts/fuzz/run.sh 24 --all-profiles -j 4               # 4 seeds/family vs dev, release-size, release-balanced, release-speed
scripts/fuzz/run.sh 180 --all-profiles --mir-inline -j 4 # nightly deep run including JSC_RUST_MIR_INLINE=on
scripts/fuzz/run.sh --ci-smoke -j 4                      # 24 seeds across dev, release-balanced, release-speed
scripts/fuzz/run.sh 20 --seed 1000 --family d -j 4       # heap/collections family starting at seed 1000
scripts/fuzz/run.sh 10 --profile release-speed --keep    # single profile, keep artifacts for all seeds
scripts/fuzz/run.sh 50 --opt on                          # legacy optimizer flag spelling
```

## Profiles, MIR inline, and parallel execution

- `--profile <dev|release-size|release-balanced|release-speed>` runs each seed against one mires profile (`js-profile=<name>`).
- `--all-profiles` compares LLVM `wasm32` against **all four** standard profiles (`dev`, `release-size`, `release-balanced`, `release-speed`) on every seed, compiling and running LLVM once per seed.
- `--mir-inline` adds `release-speed` with `JSC_RUST_MIR_INLINE=on` (`-Zinline-mir`) to the comparison matrix (works standalone or combined with `--profile` / `--all-profiles`).
- `--ci-smoke` is a convenience preset that runs 24 seeds (4 per family across all 6 families) against `dev`, `release-balanced`, and `release-speed`.
- `-j <N>` / `--jobs <N>` runs up to `N` seeds concurrently. `run.sh` acquires the repository integration lock once in the parent and **pre-builds every required keyed sysroot sequentially** before spawning workers, so `build_sysroot.sh` locks never collide and parallel `compile_core.sh` invocations compile concurrently.
- `RCGJS_WITNESS_CHECK=1` is enabled by default so optimizer witness checks run on every fuzzed build.

## Why differential fuzzing helps

Every other test suite in this repository checks compiler output against hand-maintained `.expected` files. The fuzzer checks against LLVM as a reference oracle for Rust semantics: `i128` wrapping/overflowing arithmetic, `f32`/`f64` rounding and `-0.0` bits, saturating float-to-int casts, `Drop` order across early `break`/`continue`, `alloc` collections, copy isolation, and `#[track_caller]` locations are all decided by LLVM. When any JavaScript profile disagrees, the mismatch points directly at a compiler or optimizer bug.

For that comparison to stay meaningful, every generated program has a single defined behavior: integer arithmetic uses `wrapping_`, `checked_`, `saturating_`, and `overflowing_` methods, shifts use `wrapping_shl`/`wrapping_shr`, slice indices stay in bounds, division uses `checked_div`, floats avoid unspecified NaN payload bit comparisons and signed-zero `minNum`/`maxNum` ties, and values are never read after a move. The single intentional panic is at the end of family `c`, where both builds must print identical lines (including file + line) and abort.

## Files

| File | Role |
| --- | --- |
| `gen.mjs` | `node gen.mjs <seed> [--family a\|b\|c\|d\|e\|f] [--include-large-enums]` writes one deterministic program to stdout. |
| `shim.mjs` | `node shim.mjs <program.wasm>` runs the LLVM `wasm32` build under `WebAssembly.instantiate`, providing the same `js_log_*` / `js_abort` imports that `runtime/shim.js` installs on `__rt`. |
| `run.sh` | Runs the full loop: pre-build keyed sysroots, generate, compile & execute LLVM + all selected mires profiles, diff outputs, and shrink mismatches. |

## Seed families

When `--family` is omitted, the family is chosen deterministically by `seed % 6`, so any contiguous batch of `6 * K` seeds covers all six families equally:

| Family | Coverage |
| --- | --- |
| `a` | **Expanded arithmetic**: all 12 integer widths (`i8`..`i128`, `u8`..`u128`, `isize`, `usize`) + `f32` and `f64` (`-0.0`, `INFINITY`, `NEG_INFINITY`, `NAN`, subnormals, `to_bits`/`from_bits`), wrapping/checked/saturating/overflowing ops, shifts, rotates, bit ops, cross-width & float casts, comparisons, and `total_cmp`. |
| `b` | **Aggregates & control flow**: structs, nested structs, 6-variant enums (plus 10-variant enums with `--include-large-enums`), nested `Option`/`Result`, arrays, subslices, `for`/`while`/`loop` with `break`/`continue` and labeled loops/blocks (stressing `structurizer`, `loop_opt`, and `range` BCE), `Fn`/`FnMut`/`FnOnce` closures, and `dyn Trait` dispatch. |
| `c` | **Destructors & control-flow drops**: printing `Drop` guards across nested scopes, loops with early `break`/`continue`, `match` arms, `mem::replace`/`mem::swap`, moves, early returns, and a required `#[track_caller]` panic abort. |
| `d` | **Heap & collections (`extern crate alloc`)**: `Vec<i32>`, `Vec<u8>`, `Vec<f64>`, `Vec<Item>` (SoA / product lanes), `String`, `format!`, `Box<T>`, `Box<[T]>`, `Box<dyn Trait>`, `Rc<T>`, `BTreeMap<i32, i32>`, `sort_unstable`, `push`/`pop`/`extend`/`retain`/`remove`/`swap_remove`/`chunks`/`windows`. Uses a `#[cfg(fuzz_wasm_alloc)]` static bump allocator on LLVM `wasm32` while `mires` uses its real sysroot allocator. |
| `e` | **Aliasing, mutable borrows, interior mutability & statics**: `Cell`, `RefCell`, disjoint `&mut` field reborrows, `#[inline(never)]` `&mut` helper mutations (`abi_flat` copy-back + PEA), explicit `Copy` mutation isolation checks, `static`/`const` tables with `Option<&T>` and `NonZero` niches, and `black_box` barriers. |
| `f` | **JS-engine & representation stress**: structs with JS-reserved/prototype-adjacent field names (`constructor`, `toString`, `valueOf`, `hasOwnProperty`, `then`, `TAG`), `i64`/`u64` fields inside structs & loops (`i64_pair` & `bigint_props`), multi-arm `match` & byte-delimiter scans (`switch_lut` & `tag_encode`), and UTF-8 `&str`/`String` with ASCII + multi-byte + 4-byte astral Unicode characters (`chars`, `bytes`, `char_indices`, slicing, `contains`, `split`, `trim`, ASCII case conversion). |

## Nightly deep fuzzing

`.github/workflows/nightly-fuzz.yml` runs both suites on a daily schedule (and on `workflow_dispatch` with a configurable `seeds` count, default `180`):

1. `scripts/metamorphic-test.sh --full --skip-build -j 4` — all `examples/core-tests/` fixtures across all 5 mires profiles + LLVM `wasm32`.
2. `scripts/fuzz/run.sh 180 --all-profiles --mir-inline -j 4` — 30 seeds per family across all 5 mires profiles vs LLVM `wasm32`.

On failure, `build/metamorphic/` logs/diffs and `build/fuzz/` shrunk repros are uploaded as workflow artifacts.

## Failures and shrinking

When a seed diverges on any profile, `run.sh` preserves `build/fuzz/<seed>/` containing `prog.rs`, `prog.wasm`, `llvm.out`, and per-profile `<profile>/prog.js`, `<profile>/js.out`, `<profile>/js.compile.log`, and `<profile>/stdout.diff`. A line-drop shrinker then reduces the first failing profile's repro line-by-line and writes `build/fuzz/<seed>/shrunk.rs` (and `shrunk.<profile>.rs`). `SHRINK_BUDGET=<n>` caps shrink attempts (default 400).

Seeds that fail to compile on either side are reported as `COMPILE ERROR` (`rejected`) and preserved because they indicate either a generator bug or an unsupported construct.

## Prerequisites

- `scripts/build.sh` — builds the backend library and `jsc-link`.
- The `wasm32-unknown-unknown` target for the pinned nightly (`run.sh` installs it automatically if missing).
- Keyed sysroots for the selected profiles (`run.sh` builds and caches any missing sysroot automatically).
