# `into_iter().map(..).collect()` refused when collection happens in place (compile-time)

alloc reuses the source `Vec`'s allocation when the mapped element fits in the source element
(`in_place_collect::from_iter_in_place`). That casts the buffer pointer between the two element
types, which the backend refuses. `port/src/lib.rs`:

```rust
fn second_halves(pairs: Vec<(u32, usize)>) -> Vec<usize> {
    pairs.into_iter().map(|(_, position)| position).collect()
}
```

Exact error (`error.txt`, from `receipts/logs/floor--in-place-collect.build.log`):

```text
error: rustc_codegen_js: `alloc$vec$in_place_collect$from_iter_in_place$h9dd4528c867fde6c` requires semantic cleanup lowering; baseline fallback would discard its cleanup paths
  = note: JIR lowering declined: cast: a pointer cast from pointee 20505b461f6ac7096e589d7e33eefad9 to pointee d126e484f7fef0ebcd0edc871c1a88c1 has no row in the cast matrix
error: core/src/ptr/non_null.rs:506:18: 506:52: rustc_codegen_js cannot cast a pointer to `(u32, usize)` into a pointer to `usize`
  = note: in `alloc::vec::in_place_collect::from_iter_in_place`
  = note: required by `second_halves`
  = note: required by `matrix_run`
error: alloc/src/vec/in_place_collect.rs:261:13: 261:34: rustc_codegen_js cannot cast a pointer to `(u32, usize)` into a pointer to `usize`
  = note: in `alloc::vec::in_place_collect::from_iter_in_place`
  = note: required by `second_halves`
  = note: required by `matrix_run`
```

In the MiniSearch port it hit `Vec<(u32, usize)> -> Vec<usize>` and
`Vec<(Vec<u16>, f64, Vec<Vec<u16>>, u32)> -> Vec<Suggestion>`. Workaround: `iter()` with copies,
or a push loop. A common idiom, so every port of a real library is likely to meet it.

Reproduce: `./run.sh` (build FAIL with the error above).
Settings: `js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=off`, unwind mode (`JSC_PANIC` unset), `JSC_RUST_OPT_LEVEL=z`, `JSC_RUST_MIR_INLINE=off`, `js-lane=jir`, through `experiments/library-matrix/languages/rust/build.mjs`; compiler `46383cb56f6262f99e3e729b6be35460059ee679`.
