# `<` on slices refused: `transmute` of `ControlFlow<bool>` (compile-time)

core implements `PartialOrd::lt` for `[T]` by transmuting a `ControlFlow<bool>` to `u8`
(`core/src/slice/cmp.rs:52`, `as_underlying`). The backend refuses that transmute, so `a < b` on
slices does not compile, nor does anything that reaches it: `sort_unstable()` (and `sort()`) of a
`Vec<&[u16]>`, `Vec<Vec<u16>>` or `Vec<String>`-like keys compare with `lt`. `port/src/lib.rs`:

```rust
fn less(a: &[u16], b: &[u16]) -> bool {
    a < b
}
```

Exact error (`error.txt`):

```text
error: core/src/slice/cmp.rs:52:14: `transmute` between these types is not supported by rustc_codegen_js
  = note: in `core::slice::cmp::<impl core::cmp::PartialOrd for [T]>::lt`
  = note: required by `less`
  = note: required by `matrix_run`
error: rustc_codegen_js: the link step reported errors
```

Workaround in the MiniSearch port: `sort_by(|a, b| a.cmp(b))` (`Ord::cmp` goes through
`SliceOrd::compare`, which compiles).

Reproduce: `./run.sh` (build FAIL with the error above).
Settings: `js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=off`, unwind mode (`JSC_PANIC` unset), `JSC_RUST_OPT_LEVEL=z`, `JSC_RUST_MIR_INLINE=off`, `js-lane=jir`, through `experiments/library-matrix/languages/rust/build.mjs`; compiler `46383cb56f6262f99e3e729b6be35460059ee679`.
