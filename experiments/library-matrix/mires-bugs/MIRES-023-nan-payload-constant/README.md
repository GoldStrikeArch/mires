# A NaN-payload `f32`/`f64` constant is emitted as the literal `NaN` (silent miscompile)

`port/src/lib.rs` reads the bits of four quiet NaNs whose lowest payload bit is set
(`0x7fc0_0001` as `f32`, `0x7ff8_0000_0000_0001` as `f64`): two `const`s, and the same two values
built with `from_bits` at the call. The build passes with no warning. Under the matrix profile
every call returns:

```text
MISCOMPILED: NaN payload lost: const-f32 const-f64
```

Native Rust keeps the payload of all four, so `to_bits()` returns the bits they were made from.

The backend emits a float constant that is a NaN as the JS literal `NaN`, which is the canonical
quiet NaN (`0x7fc0_0000`, `0x7ff8_0000_0000_0000`). In `port/dist/index.js` the constant arguments
are `$l(NaN)` and `$m(NaN)`. A value built at run time keeps its payload: the `from_bits` calls
compile to `new Float32Array(new Uint32Array([2143289345]).buffer)[0]` and its `f64` counterpart,
whose bits come back unchanged.

`JSC_RUST_MIR_INLINE` decides which values are constants. The matrix profile has it off, so only
the `const`s lose their payload. With `JSC_RUST_MIR_INLINE=on`, MIR inlining folds
`f32::from_bits(0x7fc0_0001)` into a constant too, and all four are lost. The second half of
`run.sh` compiles the same crate with inlining on and prints
`MISCOMPILED: NaN payload lost: const-f32 const-f64 from_bits-f32 from_bits-f64`. The MIRES-020
verifier found the inline-on case with `from_bits` arguments (its probe `v4p_nan_plain.rs`: native
and inlining off give 2143289344 and 9221120237041090560, inlining on gives 2143289345 and
9221120237041090561, both on the MIRES-020 fix and on its base).

Expected: `run({ text })` returns `text` for every floor case. Actual: every case returns the
message above.

Reproduce: `./run.sh`. The build passes, the check fails on all three floor cases, and the script
prints the port's answer, the emitted `(NaN)` arguments and the answer with inlining on.

Settings: `js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=off`, unwind mode
(`JSC_PANIC` unset), `JSC_RUST_OPT_LEVEL=z`, `JSC_RUST_MIR_INLINE=off`, `js-lane=jir`, through
`experiments/library-matrix/languages/rust/build.mjs`; the second half changes only
`JSC_RUST_MIR_INLINE=on`. Compiler `f525806d7907ab0f85c8c37a873419d3565da753` (the checkout was at
`174279a`, which changes no compiler source since `f525806`).

## Impact

No matrix port reaches it: the Rust ports use NaN only as `f64::NAN`, the canonical NaN, and no
result depends on a NaN's payload. A program that compares or stores NaN bits (`to_bits`, a
`Float32Array` view, a binary format) reads the wrong bits from such a constant.

Workaround in the ports: none needed.
