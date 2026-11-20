# Structurizer switch dispatcher omits `break` on its last case before `default: throw ... bad block` (miscompile)

`port/src/lib.rs` runs a `while !lines.is_empty()` loop with owned locals (`String`, `Vec<String>`,
`Vec<Node>`), an early `?` check, multiple `break` exits, and a `continue` branch, returning
`Option<Node>`. The build passes; every non-empty call throws at runtime:

```text
Error: rustc_codegen_js: bad block 2
```

In `port/dist/index.js`, the JIR structurizer emits an irreducible-CFG dispatcher `switch (d)`
whose normal-return epilogue sits in the final case (`case 1:`) immediately before `default:`:

```js
case 4:case 3:case 2:...;case 1:...;default:throw new Error("rustc_codegen_js: bad block "+d)}break}
```

Because `case 1` has no `break;` before `default:`, every normal return from the function executes
the `case 1` return value construction and then falls straight through into `default:`, throwing
`Error: rustc_codegen_js: bad block <d>`.

Reproduce: `./run.sh` (build PASS, check FAIL with `rustc_codegen_js: bad block`, then prints the
emitted `case 1: ... default: throw new Error("rustc_codegen_js: bad block "` sequence).
Settings: `js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=off`, unwind mode (`JSC_PANIC` unset), `JSC_RUST_OPT_LEVEL=z`, `JSC_RUST_MIR_INLINE=off`, `js-lane=jir`, through `experiments/library-matrix/languages/rust/build.mjs`; compiler `f525806d7907ab0f85c8c37a873419d3565da753`.
