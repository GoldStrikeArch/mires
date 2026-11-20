# `vec![0u8; n]` fails from 2^27 bytes: `alloc_zeroed` builds a boxed JS `Array` (runtime refusal, then process abort)

`port/src/lib.rs` allocates `vec![0u8; 129 << 20]` (129 MiB), reads its last byte and returns the
floor text. The build passes with no warning. Every call then throws:

```text
RangeError: Invalid array length
    at Array.fill (<anonymous>)
```

Native Rust allocates the buffer and returns the text, and so does JavaScript with
`new Uint8Array(129 << 20)`.

`vec![0u8; n]` lowers to `__rust_alloc_zeroed`, which the runtime implements as a boxed JS array
of `n` tagged elements (`runtime/shim.js`):

```js
globalThis.__rt.alloc_zeroed = (size, align) =>
  globalThis.__rt._register(new Array(size).fill(0), size, true);
```

V8 caps such arrays, so the size at which a zeroed byte buffer fails depends on the engine, not on
the program. Measured with the same `new Array(n).fill(0)`, one Node 26.8.1 process per size,
default heap:

| `n` (bytes) | Result |
| --- | --- |
| 2^26 (64 MiB) | succeeds after about 2 s, about 1.1 GB resident |
| 2^27 (128 MiB) | succeeds after about 6 s, about 3 GB resident |
| 2^27 + 1 to 192 MiB | `RangeError: Invalid array length` after 3 to 6 s |
| 224 MiB, 256 MiB | the process aborts: `FATAL ERROR: invalid table size Allocation failed - JavaScript heap out of memory` (SIGABRT, cannot be caught) |
| 2^31 and above | panics before the runtime is reached: on this 32-bit target the size exceeds `isize::MAX` |

Expected: `run({ text })` returns `text` for every floor case. Actual: every case fails with
`the port threw` (`RangeError: Invalid array length`).

Reproduce: `./run.sh`. The build passes and the check fails on all three floor cases with
`RangeError: Invalid array length`. Each call spends a few seconds and a few GB of memory inside
`fill(0)` before V8 refuses the array, so the script takes about 40 s.

Settings: `js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=off`, unwind mode
(`JSC_PANIC` unset), `JSC_RUST_OPT_LEVEL=z`, `JSC_RUST_MIR_INLINE=off`, `js-lane=jir`, through
`experiments/library-matrix/languages/rust/build.mjs`; compiler
`f525806d7907ab0f85c8c37a873419d3565da753` (the checkout was at `819b612`, which changes no compiler
source since `f525806`).

## Impact

Found by the 2026-11-20 review of the fflate Rust port. fflate's `gunzipSync` sizes its output
from the GZIP trailer (`new u8(gzl(data))`, ISIZE), and the port does the same with
`vec![0u8; ISIZE]` (`ports/fflate/rust/src/framing.rs`, `gunzip_sync`). A 37-byte GZIP of
`hello hello hello` whose trailer claims a larger ISIZE decodes to its 17 bytes in npm, because a
large `Uint8Array` is cheap. Under mires, an ISIZE of 128 MiB or more throws `RangeError`, from
about 224 MiB it kills the Node process, and from 2 GiB it panics. After building the fflate Rust
port (`node tools/build.mjs --library fflate --language rust`), from `experiments/library-matrix/`:

```sh
node --input-type=module -e "
import { run } from './ports/fflate/rust/dist/index.min.js'
import { gzipSync, strToU8 } from '../../node_modules/fflate/esm/browser.js'
const g = gzipSync(strToU8('hello hello hello'), { mtime: 0 }); g[g.length - 1] = 0x08
try { console.log(run({ op: 'gunzipSync', data: { bytes: Array.from(g) } }).length) } catch (e) { console.log(String(e)) }"
```

prints `RangeError: Invalid array length` (`0x20` aborts the process, `0x80` throws the port's
panic). The same representation is the performance cliff `PERF-002` (double zeroing, 2x to 4x
over-allocation for `u16`/`u32`). Any program that allocates a large zeroed buffer reaches the
limit, not only fflate.

Workaround in the port: none. fflate's `gunzipSync` writes into a non-resizable ISIZE buffer whose
length decides where the output is truncated, so allocating less would change the output. GZIP
inputs whose claimed ISIZE needs a buffer of 64 MiB or more are outside the fflate corpus
(`libraries/fflate/CONTRACT.md`, Parity domain), and the Rust port's `NOTES.md` lists the limit.
