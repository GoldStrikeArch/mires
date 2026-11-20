# A `Box<[u16]>` made by `From<&[u16]>` cannot be freed (run-time refusal)

`port/src/lib.rs` builds a `Box<[u16]>` from the text's UTF-16 three ways and drops it. The build
passes; constructions 0 (`units.into()`) and 1 (`units[1..].into()`) throw on the drop:

```text
rustc_codegen_js: this pointer does not name a live heap block, so it cannot be deallocated
    at $rt_dealloc (runtime/shim.js)
    at __rust_dealloc
    at <Box<[u16]> as Drop>
```

Construction 2 (`units.to_vec().into_boxed_slice()`) drops fine, which is the workaround the
MiniSearch port uses (`searchable_map::boxed`). In the MiniSearch build the failing drop glue was

```js
function $iu(a){let c,e,f,b,d;c=a.buf[a.off];b=c.len*2;d={size:b,align:2};if(b!==0)$aq(a.buf[a.off],$rt_scale(c,2),d);}
```

and `$rt_dealloc` found no live block for the pointer it was given.

Reproduce: `./run.sh` (build PASS; check FAIL; then case 0 and 1 throw, case 2 ok). The floor
corpus's empty string passes the first call (nothing is allocated for zero units).
Settings: `js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=off`, unwind mode (`JSC_PANIC` unset), `JSC_RUST_OPT_LEVEL=z`, `JSC_RUST_MIR_INLINE=off`, `js-lane=jir`, through `experiments/library-matrix/languages/rust/build.mjs`; compiler `46383cb56f6262f99e3e729b6be35460059ee679`.
