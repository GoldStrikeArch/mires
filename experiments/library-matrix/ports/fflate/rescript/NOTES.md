# fflate 0.8.3: ReScript port

A faithful port of the synchronous compression, decompression, framing, streaming, and text-codec
subset defined in `libraries/fflate/CONTRACT.md` to ReScript 12.3.0: the same algorithms, lookup
tables, tie-breaks, error codes, and streaming chunk boundaries as
`node_modules/fflate/esm/browser.js`, with zero `%raw`/`%%raw`, zero `Obj.magic`, zero handwritten
JS, and zero cross-call mutable state. It reaches exact bit-for-bit parity on all 83 corpus cases
in Node, Chrome, and Firefox (both minified and readable artifacts), plus 586/586 cases in a
randomized differential check against `libraries/fflate/reference.mjs`.

| File | What it is |
| --- | --- |
| `src/Tables.res` | Zero-cost bitwise/typed-array externals, `FlateError` exception, bit I/O (`shft`, `bits`, `bits16`, `wbits`, `wbits16`, `wbytes`, `slc`, `maxU8`), canonical `hMap`, and module-load immutable tables (`fleb`, `fdeb`, `clim`, `fl`, `revfl`, `fd`, `revfd`, `rev`, `flt`, `fdt`, `flm`, `flrm`, `fdm`, `fdrm`, `deo`, `crct`, `et`) |
| `src/Huffman.res` | `hTree` (stable sort, UZIP.js linear merge with strict `<`, 15-bit/7-bit Kraft debt clamping and negative-debt rollback), `ln`, `lc` (RLE code-length encoding including the empty `ddt` quirk), and `clen` |
| `src/Deflate.res` | `wfblk`, `emitClct`, `wblk` (stored/dynamic/fixed selection and tie-breaks), `hsh`, `dflt` (32 KiB sliding-window hash chains, rarest-2-byte jump, level-0 65,535-byte stored splitting), `dopt`, and `deflateSync` |
| `src/Inflate.res` | `growBuf`, `inflt` (stored/fixed/dynamic state machine, zero-padded OOB bit reads, overlapping RLE copy, custom dictionary back-references, `outSize` non-resizing semantics), and `inflateSync` |
| `src/Framing.res` | `crc` (CRC-32), `adler` (Adler-32 with 2,655-byte reduction chunks), GZIP/Zlib header/footer codecs (`gzhl`, `gzh`, `gzs`, `gzl`, `zlh`, `zls`), `gzipSync`, `gunzipSync`, `zlibSync`, `unzlibSync`, `decompressSync`, and `strToU8`/`strFromU8` |
| `src/Stream.res` | All 7 synchronous streaming classes (`Deflate`, `Gzip`, `Zlib`, `Inflate`, `Gunzip` with recursive multi-member support, `Unzlib`, `Decompress`) |
| `src/Index.res`, `src/Index.resi` | The boundary matching `libraries/fflate/reference.mjs`; `run` is the module's only export |

## Decisions

- **Direct JS typed arrays and zero-cost indexing.** `Uint8Array`, `Uint16Array`, and `Int32Array`
  are used directly with `@get_index`/`@set_index` externals (`getU8`/`setU8`, `getU16`/`setU16`,
  `getI32`/`setI32`) compiling to raw `arr[i]` / `arr[i] = v`. This preserves `fflate`'s exact
  out-of-bounds behavior on JS typed arrays (OOB reads coerce to `0` under bitwise operators in
  `bits`/`bits16`; OOB writes to a fixed `outSize` buffer silently truncate, while `TypedArray.set`
  in stored blocks throws `RangeError` as expected). Explicit bounds checks are used everywhere an
  equality test (`=== 0`) would otherwise compare `undefined === 0` (`lc`, `wblk`'s `lct` reads,
  `gzs`'s NUL scan).
- **Exact `fflate@0.8.3` algorithms and tie-breaks.** Every tie-break in `hTree`, `lc`, `wblk`
  (stored wins ties; fixed wins ties over dynamic; `bs = 0` on the first block), `dflt` (`deo`
  table, rarest-2-byte jump, `i < s + lst` at level 0), `inflt`, and `Gunzip` (recursive emission
  sequence for single- and multi-member streams) matches `browser.js` bit-for-bit.
- **Boundary.** `input` is typed as a `@tag("op")` variant with inline records and `streamStep` as
  a `@tag("action")` variant, so `run(input)` dispatches directly on the JSON object with zero input
  conversion. One-shot byte operations return the raw `Uint8Array` directly, `strFromU8` encodes
  UTF-16LE bytes, streaming operations frame `ondata` callbacks as `[u32le(len), final, ...chunk]`,
  and `expectErrorCode: true` catches `FlateError(code, _)` into `Uint8Array([0xff, code])`.
- **Host text codecs & zero cross-call mutable state.** `strToU8` and `strFromU8` bind host
  `TextEncoder` / `TextDecoder` (stateless module-level instances, matching `fflate` itself) and
  `String.fromCharCode.apply` in 16,384-byte chunks for Latin-1 decoding. Only `fflate`'s own
  immutable precomputed lookup tables and codec instances live at module scope (`module state` lint:
  `0`).

## ReScript 12.3.0 compiler and runtime findings

No compiler bugs were hit; inspecting the emitted JS (`dist/index.js`) caught several subtle
ReScript 12 behaviors worth documenting:

1. **`Int.shiftRightUnsigned(x, 0)` (`%lsrint` by `0`) is stripped by the compiler.** Because
   ReScript treats `int` as 32-bit signed, `ushr(x, 0)` constant-folded away and emitted a signed
   32-bit `int` in `gzl`. `gzl` instead computes the unsigned 32-bit GZIP `ISIZE` in `float`
   (`lo24 + hiByte * 16777216.0`) and allocates via `u8FromFloatLength`, preserving exact unsigned
   32-bit semantics up to `0xFFFFFFFF`.
2. **Preventing boxed `ref`s in hot loops.** ReScript unboxes local `ref`s into plain JS `let`
   variables only if no inner closure captures them. Hoisting `growBuf` out of `inflt` (instead of a
   nested `cbuf` closure over `buf`) and `emitClct` out of `wblk` (instead of closing over `p`) kept
   every hot-loop variable unboxed (`let outBuf = ...`, `let p = ...`).
3. **Block expressions in `while` conditions emit per-iteration IIFEs.** Writing `while cond && {
   ... }` caused ReScript to emit `while ((() => { ... })())`. Replacing those with a boolean loop
   flag (`trimming`) in `lc` and `wblk` eliminated all closure allocations in loops.
4. **Zero runtime bloat.** Using compiler-primitive externals (`%andint`, `%orint`, `%xorint`,
   `%lslint`, `%asrint`, `%lsrint`, `%bitnot_int`) avoided ReScript 12's deprecated `Int.Bitwise`
   warnings and zero-cost `=== None` / `!== None` checks avoided pulling in `Stdlib_Option` or
   `Primitive_object`. The entire bundle imports only a single `@rescript/runtime` module:
   `Primitive_exceptions.js` (492 B before Terser) for `FlateError`.
5. **Other minor notes.** `gunzipPush` compiled into a tail-call-optimized `while (true)` loop;
   `Index.resi` must repeat `@tag("op")` on the abstract `type input` to match `Index.res`; and
   `:=` binds tighter than `? :`, requiring parentheses around ternary RHS values.

## Size and quick Node performance

- **Bundle size (`dist/index.min.js`):** `20,165 B` raw, `8,016 B` gzip-9, `7,246 B` brotli-11
  (JS reference: `17,623 B` raw, `7,340 B` gzip-9, `6,672 B` brotli-11).
- **Rollup attribution (before Terser, `dist/meta.json`):** `48,115 B` port code across 7 modules
  (`Deflate` 10.1 KB, `Stream` 9.3 KB, `Inflate` 7.4 KB, `Index` 6.1 KB, `Framing` 5.7 KB,
  `Tables` 5.3 KB, `Huffman` 4.2 KB) plus `492 B` `@rescript/runtime` (`Primitive_exceptions.js`).
- **Source lints:** `0` embedded JS, `0` eval refs, `0` host JS, `0` module state.
- **Quick Node perf (`bench.mjs --environments node --samples 6`, load ~5.3–5.8):**
  - `bench-compress`: `0.785 ms` vs `0.729 ms` JS (`port/js` `1.08`, A/A `0.992`).
  - `bench-decompress`: `0.343 ms` vs `0.471 ms` JS (`port/js` `0.73`, A/A `0.996`; faster than
    the reference mainly because hoisting `growBuf` avoids `fflate`'s `cbuf` closure over `buf`
    and ReScript emits clean `| 0` integer ops and `>> 3` shifts).
  - As noted in `libraries/fflate/CONTRACT.md`, this port stays idiomatic (`TextEncoder`) and does
    not work around the Firefox Marionette CCW artifact.
