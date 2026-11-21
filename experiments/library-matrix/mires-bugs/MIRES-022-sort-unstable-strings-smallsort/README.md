# `Vec<String>::sort_unstable()` on 21 or more strings throws in core's small sort (runtime refusal)

`port/src/lib.rs` builds 21 two-letter `String`s in an unsorted order and sorts them with
`sort_unstable()`. The build passes with no warning. Every call then throws:

```text
Error: rustc_codegen_js: `copy_nonoverlapping` over 1 elements at 21 reaches past the end of a buffer of 1
```

Native Rust sorts the vector and returns the text.

The length decides it. A probe of the same function with the length read from the input sorted 0,
1, 2, 8, 16 and 20 strings and threw at 21, 32, 33, 39, 40 and 150 (the offset in the message
varies: 21, 32, 1, 1, 29, 1). Up to 20 elements `slice::sort_unstable` sorts by insertion. From
21 it runs ipnsort, whose small sort copies elements with `ptr::copy_nonoverlapping`. The A2
(MIRES-017) verifier found the refusal in core's small sort `sort4_stable`, on the base compiler
and with its fix, in unwind, abort and inline modes alike (its probe `v05_sort.rs` sorts 150
strings).

Expected: `run({ text })` returns `text` for every floor case. Actual: every case fails with
`the port threw`.

Reproduce: `./run.sh`. The build passes, the check fails on all three floor cases, and the script
prints the refusal from the check receipt.

Settings: `js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=off`, unwind mode
(`JSC_PANIC` unset), `JSC_RUST_OPT_LEVEL=z`, `JSC_RUST_MIR_INLINE=off`, `js-lane=jir`, through
`experiments/library-matrix/languages/rust/build.mjs`; compiler
`f525806d7907ab0f85c8c37a873419d3565da753` (the checkout was at `174279a`, which changes no compiler
source since `f525806`).

## Impact

Found by the 2026-11-20 verification of the MIRES-017 fix (adjacent bug 2 of its verdict). No
matrix port reaches it: no Rust port calls `sort_unstable` (minisearch, d3-hierarchy and the
pilot use the stable `sort_by`, fflate a hand-written insertion sort). Any program that sorts 21
or more owned strings with `sort_unstable` fails.

Workaround in the ports: none needed.
