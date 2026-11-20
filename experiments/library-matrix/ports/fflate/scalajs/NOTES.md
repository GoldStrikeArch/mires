# fflate 0.8.3: Scala.js port

A faithful port of the synchronous compression, decompression, framing, streaming, and text-codec
subset in `libraries/fflate/CONTRACT.md` in Scala 3.8.4 for Scala.js 1.22.0, built by
`languages/scalajs/build.mjs` (`fullLinkJS`, ES modules, common Terser). It implements
`fflate@0.8.3`'s exact algorithms, lookup tables, Huffman tie-breaks, LZ77 hash chains, block
selection, inflate state machine, error codes, and streaming chunk boundaries, reaching exact
bit-for-bit parity on all 83 corpus cases in Node, Chrome, and Firefox (both minified and
readable artifacts).

| File | What it is |
| --- | --- |
| `src/Tables.scala` | `FlateError`, module-level read-only lookup tables (`fleb`, `fdeb`, `clim`, `fl`, `revfl`, `fd`, `revfd`, `rev`, `flt`, `fdt`, `flm`, `flrm`, `fdm`, `fdrm`, `deo`, `crct`, `emptyBytes`), `hMap`/`hMapSlice`, `defaultPlvl`, `shft`, `maxSlice`, `wbytes`, `doubleToU32`, clamped `sliceBytes`, and zero-copy `bytesToU8`/`u8ToBytes` views |
| `src/Huffman.scala` | `hTree` (flat parallel `Array[Int]` node arrays, custom stable insertion sorts, UZIP.js two-queue linear merge with strict `<`, 15-bit/7-bit signed Kraft debt clamping & rollback), `lc` (RLE code-length encoding including empty-tree `[0]`), `clen` |
| `src/DeflateCore.scala` | `CompressOptions`, `DeflateState`, `inline` `wbits`/`wbits16`, `wfblk`, `wblk` (stored/fixed/dynamic cost comparison & tie-breaks), `dflt` (32 KiB sliding window, hash chains, rarest-2-byte jump, level-0 stored splitting with `i < s + lst`), `doptBytes`, `deflateSync` |
| `src/InflateCore.scala` | `DecompressOptions`, `InflateState`, `inline` `bits`/`bits16` (zero-padded OOB reads with fast 3-byte path), `infltRaw`/`inflt` (stored/fixed/dynamic decoding, overlapping RLE fast path, dictionary back-references, `outSize` non-resizing truncation vs stored `RangeError`), `inflateSync` |
| `src/Framing.scala` | `Crc32`, `Adler32`, `gzhl`/`gzh`/`gzs`/`gzl`, `gzipSync`, `gunzipSync`, `zlh`/`zls`, `zlibSync`, `unzlibSync`, `decompressSync` |
| `src/Streams.scala` | All 7 synchronous streaming classes (`Deflate`, `Gzip`, `Zlib`, `Inflate`, `Gunzip` with recursive multi-member emission, `Unzlib`, `Decompress`) |
| `src/Codec.scala` | Host `TextEncoder`/`TextDecoder` `@js.native` facades, `strToBytes`, `strToU8`, `strFromU8`, and `stringToUtf16Le` |
| `src/Main.scala` | `@js.native` facade traits for the JSON corpus input, payload/option decoding, streaming record framing, error-code handling, and `@JSExportTopLevel("run")` |

## Architecture & Interop Decisions

- **Unboxed Scala primitive arrays (`Array[Byte]` / `Array[Char]` / `Array[Int]`) with zero-copy JS typed-array views.**
  In Scala.js 1.22, `Array[Byte]` is backed by JS `Int8Array`, `Array[Char]` by JS `Uint16Array`
  (unsigned 16-bit `0..65535`, matching `fflate`'s `u16` tables `prev`, `head`, `rev`, `flm`,
  `flrm`, `fdm`, `fdrm`, `lf`, `df`, `lcfreq` with half the memory of `Int32Array`), and
  `Array[Int]` by JS `Int32Array`. Indexing Scala arrays compiles directly to raw `arr.a[i]` with
  zero `$uS`/`$uI` runtime unboxing checks, and `System.arraycopy` compiles to native
  `TypedArray.prototype.set`. At the boundary, `byteArray2Int8Array` / `int8Array2ByteArray`
  convert between `Array[Byte]` and `js.typedarray.Uint8Array` via zero-copy `ArrayBuffer` views.
- **No `Long` on hot paths and no `scala.collection` dependency.** All DEFLATE, Inflate, CRC-32
  (`>>> 8`), Adler-32, and `wbytes` (`>>>= 8`) arithmetic uses 32-bit `Int`. `Double` is used only
  for `mtime` (`Math.floor(mtime / 1000.0)`), converted to unsigned 32-bit bits via `doubleToU32`
  (explicitly wrapping values `>= 2^31` because Scala `Double.toInt` clamps at `Int.MaxValue`).
  Avoiding `scala.collection` (`Seq`, `Map`, `Vector`, `ArrayBuffer`, `StringContext`) keeps the
  bundle dramatically smaller than collection-heavy Scala.js ports (51.7 KB vs MiniSearch's
  306 KB), using flat `Array[Int]`s and 15-line stable insertion sorts in `hTree`, plus `js.Array`
  only for collecting streaming callbacks in `Main.scala`.
- **Host `TextEncoder` / `TextDecoder` facades (`Codec.scala`).** Matching `fflate/esm/browser.js`
  and `CONTRACT.md`, UTF-8 `strToU8` and `strFromU8` delegate to module-level stateless
  `TextEncoder` / `TextDecoder` instances through `@js.native @JSGlobal` facades (providing exact
  WHATWG single-BOM stripping and maximal-subpart `U+FFFD` replacement, and preserving parity with
  `fflate` without working around the Firefox Marionette CCW artifact noted in `CONTRACT.md`).
- **Scala 3 `inline def` on inner-loop bit I/O.** Marking `shft`, `wbits`, `wbits16`, `bits`, and
  `bits16` as `inline def` expands them directly inside `wblk` and `infltRaw`, eliminating JS
  closure call overhead on the hot path.
- **Safe bounds handling for JS-idiomatic OOB reads/writes.** All reads that `fflate` relies on
  coercing `undefined` to `0` (`bits`/`bits16`, `gzs`, `zls`, `lc` on empty `ddt`, `lct[clim[i]]`,
  `ldt[i - 1]` on truncated dynamic headers, `i >= dif` before `hsh(i - dif)`) are explicitly
  bounds-checked. Output buffers in `dflt` include `+2` slack bytes so `wbits16` never writes out
  of bounds. In `gunzipSync`, if a corrupt GZIP trailer produces a negative signed `gzl(data)`
  (unsigned ISIZE `>= 2^31`), the port passes `null` (auto-resizing buffer) rather than attempting
  a negative array allocation.
- **No cross-call mutable state.** `Tables` and `Codec` hold only read-only module-level tables
  and stateless `TextEncoder`/`TextDecoder` instances matching `fflate` itself. Every `run` call
  allocates fresh per-call state.

## Lint Columns

- **Embedded JS:** `0`. **Eval references:** `0`. **Host JS:** `0`.
- **Module state:** `18` findings, all false positives on 2-space-indented instance `var` fields
  inside per-call classes (`Framing.scala`: 3 fields on `Crc32` and `Adler32`; `Streams.scala`:
  15 fields on `Deflate`, `Gzip`, `Zlib`, `Inflate`, `Gunzip`, `Unzlib`, `Decompress`), because
  `tools/lib/source-stats.mjs` matches any `var` indented `<= 2` spaces and does not distinguish
  `class` bodies from `object` bodies. Zero `var`s or mutable collections exist on any `object`.

## Size Breakdown

Measured via `tools/sizes.mjs` (`fullLinkJS` + common Terser, `1,557` Scala code lines across 8 files):

- **`dist/index.min.js`:** `51,737` B raw / `17,636` B gzip-9 / `15,493` B brotli-11
  (JS reference for scale: `17,623` B raw / `7,340` B gzip-9 / `6,672` B brotli-11)
- **`dist/index.js` (readable linked output):** `147,097` B raw / `28,660` B gzip-9 / `24,169` B brotli-11
  - Port code (`fflate.*`): `82,403` B (`56.0%`), led by `DeflateCore` (`16.7 KB`), `InflateCore`
    (`14.3 KB`), `Main` (`10.4 KB`), `Huffman` (`7.1 KB`), `Tables` (`6.6 KB`), `Framing` (`5.3 KB`),
    and the 7 streaming classes (`12.5 KB` combined).
  - Scala.js runtime/stdlib: `64,695` B (`44.0%`), including `$TypeData`, `Throwable`/exceptions,
    typed-array wrappers, and minimal stdlib stubs (`RTLong`, `MurmurHash3`, `AbstractSeq` pulled
    indirectly through `Array(...)` literal / `StringBuilder` / `Throwable` linkages).

## Verification & Quick Node Perf Observations

- **Reproducibility:** 2 consecutive `--clean` builds produced byte-identical outputs
  (`reproducibility.identical === true`).
- **Corpus parity:** `83/83` exact byte-for-byte parity in Node 26, Chrome 158, and Firefox 160
  for both `dist/index.min.js` and `dist/index.js`.
- **Differential fuzzing:** `938/938` randomized differential cases against `reference.mjs` in
  Node passed bit-for-bit (one-shot levels `0..9`, `mem` `0..8`, dictionaries up to 34 KiB,
  `mtime` > 2^31-1 s, `outSize`, streaming `Deflate`/`Gzip`/`Zlib` with `flush()`/`flush(true)`
  and >164 KiB wraparound, streaming `Inflate`/`Gunzip` multi-member/`Unzlib`/`Decompress` with
  1..64 B dynamic-header splits, UTF-8/BOM/ill-formed `strFromU8`, and corrupt streams/error codes).
- **Quick Node perf check** (`tools/bench.mjs --environments node --samples 6`, triplet design,
  load average ~5.9):
  - `bench-compress`: `0.873 ms` port vs `0.814 ms` JS — **`port/js = 1.07×`** (`A/A = 1.002`)
  - `bench-decompress`: `0.363 ms` port vs `0.491 ms` JS — **`port/js = 0.74×`** (`A/A = 0.974`)
  Scala 3's `inline def` expansion of `bits`/`bits16` and the unboxed `Array[Byte]`/`Array[Char]`
  RLE copy fast path let `InflateCore.infltRaw` outperform the hand-written JS reference (which
  calls non-inlined `bits`/`bits16` helper closures inside its inner loop) while `DeflateCore.dflt`
  tracks native JS within ~7%.

## Scala.js 1.22 Findings

- No compiler bugs or linker issues encountered; all Scala 3.8.4 union types (`T | Null`) and
  `inline def`s compiled cleanly without warnings and reproduced byte-identically.
- Scala `Double.toInt` follows JVM saturation semantics (clamping values `>= 2^31` to
  `Int.MaxValue`), which diverges from JS `ToUint32` on GZIP `mtime` timestamps past year 2038;
  handled explicitly in `Tables.doubleToU32`.
- Inspecting `dist/index.js` confirmed Scala.js `fullLinkJS` compiles `Array[Byte]`, `Array[Char]`,
  and `Array[Int]` reads/writes directly to raw `Int8Array`/`Uint16Array`/`Int32Array` indexing
  (`arr.a[i]`) without per-access boxing or type-check helpers, making Scala primitive arrays +
  zero-copy typed-array boundary views faster and safer than raw `js.typedarray.Uint8Array`
  indexing in hot loops.
