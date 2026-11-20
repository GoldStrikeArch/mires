# Tuple `Clone::clone` writes into an uninitialized local (miscompile)

`port/src/lib.rs` clones a `(String, usize)` through `#[inline(never)] fn duplicate(pair: &(String, usize)) -> (String, usize) { pair.clone() }`.
The build passes; every call throws:

```text
TypeError: Cannot set properties of undefined (setting '0')
```

The emitted clone (`port/dist/index.js`, readable build) never creates the tuple it fills:

```js
function $i(a){let f,e,c,d,b;c=a[0].vec;d=$c($rt_byte_slice_ref($rt_slice_from_parts(c.buf.inner.ptr,c.len)),void 0);e={vec:d};b[0]=e;b[1]=a[1];f=b;return f}
```

`b` is declared and indexed but never assigned `[]` (or the aggregate). The same shape appeared
in the MiniSearch port for `(Vec<u16>, Vec<usize>)`, `(String, Value)`, `(DocId, f64)`,
`(String, f64)` and `(Vec<u16>, f64)`: any tuple with a non-`Copy` field, including every
`Vec<(..)>::clone`, which clones each tuple. Derived enum and struct clones in the same program (`DocId`, `Value`, `SearchOptions`) compiled correctly.

Reproduce: `./run.sh` (build PASS, check FAIL 0/3, then prints the emitted function).
Settings: `js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=off`, unwind mode (`JSC_PANIC` unset), `JSC_RUST_OPT_LEVEL=z`, `JSC_RUST_MIR_INLINE=off`, `js-lane=jir`, through `experiments/library-matrix/languages/rust/build.mjs`; compiler `46383cb56f6262f99e3e729b6be35460059ee679`.
