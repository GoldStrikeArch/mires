# The wasm32-LLVM-vs-JS differential fuzzer

One generated Rust source, compiled twice — by the ordinary LLVM backend for
`wasm32-unknown-unknown` and by `rustc_codegen_js` — and run twice. The two runs must print the
same stdout and end the same way.

```sh
scripts/fuzz/run.sh 200                # seeds 0..199, all three families
scripts/fuzz/run.sh 20 --seed 1000     # seeds 1000..1019
scripts/fuzz/run.sh 10 --family c      # destructors and the required abort
scripts/fuzz/run.sh 5 --keep           # keep build/fuzz/<seed>/ for every seed
scripts/fuzz/run.sh 50 --lane jir --opt on   # the JIR lane, optimizer on
```

`--lane baseline|jir` and `--opt off|report|on` compose the matching `js-lane=`/`js-opt=` backend
options onto `$JS_EXTRA_ARGS`. `js-opt` is emit-affecting, so each `--opt` value keys its own
sysroot: the run selects it, builds it if missing, and prints the key in its banner — the JS side
never silently falls back to the default sysroot.

## Why this is worth having

Every other suite in this checkout compares the compiler against an expectation someone wrote
down. This one compares it against the reference implementation of what Rust means. LLVM decides
what `i64::MIN.wrapping_neg()`, a saturating float cast, a `Drop` order or a `#[track_caller]`
location is; the JS backend has to agree, and where it does not, the disagreement is a miscompile
rather than a stale golden.

That only holds if the program has ONE meaning, so `gen.mjs` never emits anything whose behaviour
is undefined or unspecified: arithmetic is `wrapping_`/`checked_`/`saturating_`, shifts go through
`wrapping_shl`/`wrapping_shr`, slices are indexed inside their bounds, division goes through
`checked_div`, and nothing is read after a move. The one deliberate panic is the family `c` finale,
where both sides must abort after printing the same lines.

## The three files

| file | what it is |
| --- | --- |
| `gen.mjs` | `node gen.mjs <seed> [--family a\|b\|c]` writes one program to stdout. Output is a pure function of the seed, so a reported mismatch is reproducible from the seed alone. |
| `shim.mjs` | `node shim.mjs <program.wasm>` runs the LLVM build under `WebAssembly.instantiate`, supplying the same `js_log_*` imports `runtime/shim.js` installs on `__rt`. |
| `run.sh` | the loop: generate, compile both ways, run both ways, compare, shrink a mismatch. |

## The seed families

| family | what it covers |
| --- | --- |
| `a` | integer arithmetic: wrapping/checked/saturating, `i64` and `u128`, shifts, rotates, bit counting, byte and bit reversal, integer and (saturating) float casts, comparisons. |
| `b` | aggregates: structs, tuple structs, enums with payloads, `Option`/`Result` combinators, `?`, arrays, subslices, iterator chains (`map`/`filter`/`fold`/`zip`/`rev`/`enumerate`), `FnMut` closures, and one `dyn Trait` call. |
| `c` | destructors: `Drop` guards that print, nested scopes, moves, a drop in a function that took a value, an early return past a live guard, and a `#[track_caller]` panic the program must abort on. |

A seed with no `--family` picks one from `seed % 3`, so a run of N seeds covers all three.

## How the two sides are made to be comparable

* **One source.** The generated program declares the `extern "C"` print helpers
  `examples/core-tests/prelude.rs` declares — `js_log_i32`, `js_log_i64`, `js_log_f64`,
  `js_log_bool`, `js_log_str`, `js_abort` — which are exactly the members `runtime/shim.js`
  installs. The JS backend lowers a foreign call to `__rt.<name>(...)`; the LLVM build turns the
  same declarations into wasm imports (`-Clink-arg=--import-undefined`), which `shim.mjs` supplies.
* **One printer.** Both sides end in `console.log`, so `shim.mjs` converts each wasm value to the
  JavaScript value the JS backend would have passed: an `i64` is a `BigInt` printed with
  `toString`, a `bool` arrives as `0`/`1` and is printed as a boolean, and a `&str` is read out of
  the exported memory. The C ABI for `wasm32-unknown-unknown` passes `&str` *indirectly* — one
  pointer to a `{ptr, len}` pair — and `shim.mjs` accepts that and the two-argument form.
* **One set of paths.** Both compilations pass the same `--remap-path-prefix`, so the file name a
  `#[track_caller]` panic prints is identical.
* **Exit status as a class.** A panic reaches `js_abort`, which the JS shim raises as an exception
  (node exits 1) and `shim.mjs` reports as exit 101. Both are "aborted"; the number is not
  compared, because it says nothing about the compilers.

## What a failure looks like

A mismatching seed keeps `build/fuzz/<seed>/`: `prog.rs`, `prog.wasm`, `prog.js`, both outputs,
both compile logs and `stdout.diff`. The line-drop shrinker then removes one line at a time,
keeping a drop when both compilers still accept the program and the two still disagree, and writes
the result to `build/fuzz/<seed>/shrunk.rs`. `SHRINK_BUDGET=<n>` caps the number of attempts
(default 400).

A seed neither side can compile is reported as `COMPILE ERROR` and kept too. That is a generator
bug or an unimplemented construct rather than a miscompile, and it is counted separately for that
reason.

## Prerequisites

* `scripts/build.sh` — the backend dylib.
* `scripts/build_sysroot.sh` — the default (no extra options) sysroot the JS side links against.
* The `wasm32-unknown-unknown` target for the pinned nightly. `run.sh` installs it if it is
  missing.
