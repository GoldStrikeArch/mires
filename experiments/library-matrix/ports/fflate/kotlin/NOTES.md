# fflate 0.8.3: Kotlin/JS port

A faithful port of the synchronous compression, decompression, framing, streaming, and text-codec
subset in `libraries/fflate/CONTRACT.md` to Kotlin/JS 2.4.10 (the harness's Gradle production lane:
whole-program ES module, ES2015, production DCE and member minimisation, followed by 2-pass Terser).
It reproduces `node_modules/fflate/esm/browser.js` algorithm by algorithm with zero `js("…")`, zero
`eval`, zero host JS, and zero module-state lint findings. It reaches exact bit-for-bit parity on
all 89 corpus cases in Node, Chrome, and Firefox (both `dist/index.min.js` and `dist/index.js`), and
passes 1,490+ randomized and targeted edge-case differential checks against `reference.mjs`.

| File | What it is |
| --- | --- |
| `src/Tables.kt` | Immutable module-level lookup tables (`FLEB`, `FDEB`, `CLIM`, `FL`, `REVFL`, `FD`, `REVFD`, `REV`, `FLT`, `FDT`, `FLM`, `FLRM`, `FDM`, `FDRM`, `DEO`, `CRCT`), `FlateException`, `hMap`, `shft`, `slc`, `byteAt`, `defaultPlvl` |
| `src/Huffman.kt` | `hTree` (parallel unboxed `IntArray` nodes, custom in-place stable insertion sort, UZIP.js two-queue linear merge with strict `<`, signed Kraft debt clamping & rollback), `lc` (including the empty-distance-tree `{ c: [0], n: 1 }` quirk), `clen` |
| `src/Deflate.kt` | `CompressOptions`, `DeflateState`, `wbits`, `wbits16`, `wfblk`, `wblk` (stored/fixed/dynamic cost comparison & tie-breaks), `dflt` (32 KiB sliding window, `ShortArray` `prev`/`head` hash chains, rarest-2-byte jump, level-0 `i < s + lst` splitting), `dopt`, `deflateSync` |
| `src/Inflate.kt` | `DecompressOptions`, `InflateState`, `bits`/`bits16` (zero-padded bounds-checked reads), `infltRaw` (zero-copy input slicing via `datStart`/`datEnd`, dynamic tree decoding, overlapping RLE copy, dictionary back-references, `outSize` non-resizing truncation vs stored `IndexOutOfBoundsException`), `inflateSync` |
| `src/Framing.kt` | `Crc32`, `Adler32`, `wbytes`, `doubleToU32Bits` (unsigned 32-bit `mtime` conversion without `Double.toInt()` saturation), `gzhl`/`gzh`/`gzs`/`gzl`, `gzipSync`, `gunzipSync`, `zlh`/`zls`, `zlibSync`, `unzlibSync`, `decompressSync` |
| `src/Streams.kt` | Sync streaming classes: `DeflateStream`, `GzipStream`, `ZlibStream`, `InflateStream`, `GunzipStream` (recursive multi-member emission sequence), `UnzlibStream`, `DecompressStream` |
| `src/Codec.kt` | Host `TextEncoder`/`TextDecoder` external bindings, zero-copy `ByteArray` ↔ `org.khronos.webgl.Uint8Array` views, `strToU8`, `strFromU8`, `stringToUtf16Le` |
| `src/Main.kt` | Boundary (`external interface`s, `RecordBuffer` `[u32le(len), final, ...chunk]` framing, `expectErrorCode` dispatch) and the single `@JsExport` entry `run(input: InputJson): Uint8Array` |

## Architecture and boundary decisions

- **Native JS typed arrays throughout.**
  - `ByteArray` compiles directly to JS `Int8Array` (signed bytes, masked with `and 0xFF` on read
    and `.toByte()` on write; `copyInto` lowers to `arrayCopy` over `TypedArray.prototype.set`).
  - `ShortArray` compiles to `Int16Array` for `dflt`'s `prev` (32,768 entries) and `head`
    (`1 shl plvl` entries) and `REV` (32,768 entries). Because every index stored in `prev`/`head`
    and every reversed 15-bit code in `REV` lies strictly in `0..32767`, signed 16-bit storage
    matches `fflate`'s `Uint16Array` memory footprint with zero sign-extension or masking overhead.
  - `IntArray` compiles to `Int32Array` for `syms` (25,000), Huffman encode/decode maps (`flm`,
    `flrm`, `fdm`, `fdrm`, `lm`, `dm`, `clm`), frequency tables (`lf`, `df`), `CRCT`, and `DEO`.
- **Zero-copy `Uint8Array` boundary.** `ByteArray.toUint8Array()` constructs
  `org.khronos.webgl.Uint8Array(i8.buffer, i8.byteOffset, i8.byteLength)` over the underlying
  `Int8Array`'s `ArrayBuffer`, and `Uint8Array.toByteArray()` creates the reverse `Int8Array` view.
  Every one-shot operation returns a raw JS `Uint8Array` with zero extra copy.
- **Host `TextEncoder` and `TextDecoder`.** Declared via Kotlin `external class` bindings, matching
  `fflate`'s own module-level `TextEncoder`/`TextDecoder` usage (`strToU8` UTF-8 and WHATWG
  `strFromU8` with single leading BOM stripping and maximal-subpart `U+FFFD` replacement). Latin-1
  uses direct `String[i].code` (`charCodeAt`) and `CharArray.concatToString()`.
- **No `kotlin.collections` or callable references.** `hTree` uses parallel `IntArray`s and a
  15-line in-place stable insertion sort rather than `List<Node>` / `sortWith`. Stream record
  accumulation uses a small array-backed `RecordBuffer`, and stream callbacks use lambdas instead of
  `records::add` to avoid pulling in `ArrayList`, `HashMap`, or `KFunction` reflection wrappers.
- **Zero cross-call mutable state.** Only `fflate`'s immutable lookup tables and stateless
  `TextEncoder`/`TextDecoder` instances live at module scope (`moduleState: 0`).
- **Exact `fflate 0.8.3` tie-breaks and quirks.** Preserves `hTree` strict `<` merge and signed
  `dt >>= lft` debt rollback, `lc` empty distance tree `{ c: [0], n: 1 }`, `wblk` stored/fixed tie
  order and initial `bs = 0`, `dflt` rarest-2-byte jump and `i < s + lst` level-0 trailing empty
  block, `inflt` zero-padded OOB reads and `outSize` non-resizing truncation vs stored throw,
  `inflt` output stores that behave like `Uint8Array` stores (a literal past the end of the buffer
  is dropped while `bt` advances; the buffer grows only for a stored block, at Huffman block start
  and before a back-reference, so a run of more than 131,072 literals loses its tail as in fflate;
  before the 2026-11-20 review the port grew the buffer for literals and kept the whole run),
  `Gunzip` recursive single/multi-member emission sequence, `doubleToU32Bits` for GZIP `mtime`
  timestamps above `2^31 - 1`, and `gunzipSync` falling back to a dynamic `null` buffer when
  `gzl(data) < 0` (bit 31 set in the trailer's ISIZE) so corrupt or large trailers decode or throw
  `FlateError` instead of failing on `ByteArray` allocation.

## Kotlin/JS 2.4.10 findings

1. **`Double.toInt()` pulls in the boxed `Long` runtime (~11 KB pre-Terser).** Even when the
   receiver is statically typed as `Double` (`doubleToU32Bits` in `Framing.kt`), Kotlin/JS 2.4.10
   lowers `Double.toInt()` through `numberToInt(a)`, which branches on `a instanceof Long` and
   retains the entire emulated 64-bit `Long` class (`multiply`, `divide`, `compareTo_0`,
   `toStringImpl`, `PrimitiveKClassImpl`). No `Long` is used anywhere in the port.
2. **Gradle 9.4.1 macOS FSEvents deadlock.** On macOS under `--no-daemon`, Gradle's VFS file
   watcher can deadlock in `OsxFileEventFunctions$OsxFileWatcher.startWatching0`. Running with
   `GRADLE_OPTS="-Dorg.gradle.vfs.watch=false"` avoids the deadlock cleanly without altering any
   compiler flag or output (reproducible 9–10 s clean builds).
3. **Reproducibility.** Clean builds are byte-identical (`reproducibility.identical === true`) under
   the harness template's in-process `-XX:hashCode=2` JVM setting.

## Size

- `dist/index.min.js`: **35,764 B** raw, **13,303 B** gzip-9, **11,835 B** brotli-11 (JS reference
  for scale: 17,623 B raw / 7,340 B gzip-9 / 6,672 B brotli-11; Kotlin runtime floor: 46 B).
- Lints: `lines: 1684`, `embeddedJs: 0`, `evalRefs: 0`, `hostJs: 0`, `moduleState: 0`.
- Pre-Terser `dist/index.js` is **114,576 B**: ~83.7 KB (~73%) is the `fflate` port itself,
  ~11.0 KB (~10%) is the stdlib `Long` class pulled in by `Double.toInt()`, and ~19.4 KB (~17%) is
  Kotlin/JS core runtime (`initMetadataFor`, `arrayCopy`, `copyOf`, exceptions, `concatToString`).

## Quick Node performance check

Measured with `tools/bench.mjs --environments node --samples 6` (triplet design, bench variant;
isolated `dev-kotlin` receipts; not a performance claim — read ratios next to A/A and load):

| Case | Node port p50 | JS ref p50 | port/js | paired | A/A |
| --- | --- | --- | --- | --- | --- |
| `bench-compress` | ~0.78 ms | ~0.72 ms | ~1.09 | ~1.09 | ~0.99 |
| `bench-decompress` | ~0.40 ms | ~0.45 ms | ~0.88 | ~0.87 | ~0.99 |

Because Kotlin/JS lowers `ByteArray`/`ShortArray`/`IntArray` directly to JS typed arrays with
monomorphic 32-bit `| 0` integer arithmetic and zero-copy `Uint8Array` views at the boundary, both
compression and decompression run right alongside `fflate` in V8 (with `bench-decompress` slightly
faster than the reference thanks to zero-copy input range slicing in `infltRaw` and tight inner
RLE loops). Note: this port does **not** work around the Firefox Marionette CCW timing artifact
documented in `CONTRACT.md`.
