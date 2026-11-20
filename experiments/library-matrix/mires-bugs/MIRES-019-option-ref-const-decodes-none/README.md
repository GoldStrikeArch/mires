# Constant `Option<&u8>` (`Some(&b'.')`) silently decodes to `{TAG:"None"}` (silent miscompile)

`port/src/lib.rs` checks `s.as_bytes().last() == Some(&b'.')` on a non-empty `&str` ending with `'.'`.
The build passes with zero warnings, but every comparison against `Some(&b'.')` silently returns
`false` at runtime because `rustc` promotes `Some(&b'.')` to a constant niche-encoded `Option<&u8>`
carrying a static pointer provenance, and `mires` decodes that constant as `{TAG:"None"}`!

In `port/dist/index.js`, `ends_with_dot` emits:

```js
d="None",e=void 0;
```

(and in a non-flattened call, `$bo(v, {TAG:"None"})`), comparing the `Some(&u8)` from `slice::last()`
against `None` instead of `Some(&b'.')`.

Reproduce: `./run.sh` (build PASS, check FAIL on `ascii` case because `ends_with_dot("...dog.")`
returns `false`, then prints the emitted `{TAG:"None"}` comparison).
Settings: `js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=off`, unwind mode (`JSC_PANIC` unset), `JSC_RUST_OPT_LEVEL=z`, `JSC_RUST_MIR_INLINE=off`, `js-lane=jir`, through `experiments/library-matrix/languages/rust/build.mjs`; compiler `f525806d7907ab0f85c8c37a873419d3565da753`.
