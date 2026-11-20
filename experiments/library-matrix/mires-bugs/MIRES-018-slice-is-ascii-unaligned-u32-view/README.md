# `str::is_ascii()` / `[u8]::is_ascii()` throws `byte offset N cannot form a u32 scalar view` under `js-str=bytes`

`port/src/lib.rs` slices a non-empty `&str` at byte offset 1 (`&text[1..]`) and calls `.is_ascii()`.
The build passes; calling `run({ text: "The quick brown fox..." })` throws at runtime:

```text
Error: rustc_codegen_js: byte offset 39 cannot form a u32 scalar view
```

In `core::slice::ascii::is_ascii` (called by `str::is_ascii`), any slice of length ≥ 4 uses SWAR
`u32` reads (`align_offset(4)` + unaligned head/tail `u32` loads via `$rt_unscale_scalar(..., 4, ..., "u32")`).
Under `js-str=bytes`, any `&str` / `&[u8]` sliced at a non-4-byte-aligned offset (e.g. byte offset 1, 3, 9)
OR whose length ≥ 4 is not a multiple of 4 (e.g. length 43 → tail read at `43 - 4 = 39`) fails
`$rt__scalar_view` and throws a runtime error.

Reproduce: `./run.sh` (build PASS, check FAIL with `byte offset 39 cannot form a u32 scalar view`).
Settings: `js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=off`, unwind mode (`JSC_PANIC` unset), `JSC_RUST_OPT_LEVEL=z`, `JSC_RUST_MIR_INLINE=off`, `js-lane=jir`, through `experiments/library-matrix/languages/rust/build.mjs`; compiler `f525806d7907ab0f85c8c37a873419d3565da753`.
