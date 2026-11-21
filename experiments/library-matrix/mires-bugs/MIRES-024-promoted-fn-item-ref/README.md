# A promoted `&fn_item` passed to `F: Fn` is refused at link time (compile refusal)

`port/src/lib.rs` calls `apply_ref(&twice)`, where `twice` is a plain `fn(u32) -> u32` and
`apply_ref<F: Fn(u32) -> u32>(f: &F)` returns `f(7) + f(8)`. rustc promotes `&twice` to a constant
reference to the zero-sized fn item. The link step refuses it (`error.txt`):

```text
error: src/lib.rs:28:30: 28:36: constants of type `fn(u32) -> u32 {twice}` cannot be read out of an allocation
  = note: in `matrix_run`
```

Native Rust returns 30. The program is safe Rust with no unusual feature.

The MIRES-020 verifier found it (adjacent bug 2 of its verdict, probe `v3_fnitem_ref.rs`): the
lane report gives `ConstantUnencodable` in all six modes it tried (default, `-Oz` and `-Oz` with
MIR inlining, each in unwind and abort mode), on the MIRES-020 fix and on its base. It is
therefore not the `SpreadNotTuple` refusal of MIRES-020.

On this compiler the forms that avoid the promoted constant are refused too, by MIRES-020: in
scratch builds `apply_ref(&local)` with `let local = twice`, `apply_ref(&|x| twice(x))` and
`apply(twice)` with `f: F` by value all stop at `requires semantic cleanup lowering ...
SpreadNotTuple` in the generic function.

Expected: the build passes and `run` returns 30. Actual: the build fails.

Reproduce: `./run.sh`. The build fails, and the script prints the refusal from the build log.

Settings: `js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=off`, unwind mode
(`JSC_PANIC` unset), `JSC_RUST_OPT_LEVEL=z`, `JSC_RUST_MIR_INLINE=off`, `js-lane=jir`, through
`experiments/library-matrix/languages/rust/build.mjs`; compiler
`f525806d7907ab0f85c8c37a873419d3565da753` (the checkout was at `174279a`, which changes no compiler
source since `f525806`).

## Impact

No matrix port reaches it. A program that passes a reference to a function item to generic code
fails to build.

Workaround in the ports: none needed.
