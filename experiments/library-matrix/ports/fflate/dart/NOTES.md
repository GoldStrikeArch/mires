# fflate 0.8.3: Dart port (dart2js)

A faithful port of the synchronous compression, decompression, framing, streaming, and text-codec
subset defined in `libraries/fflate/CONTRACT.md` in Dart 3 (`sdk: ^3.13.0`, compiled with Dart SDK
3.14.0 via `languages/dart/build.mjs`: `dart compile js -O2 --no-source-maps`, comparator facade,
common Terser). It implements `fflate@0.8.3`'s exact algorithms, lookup tables, tie-breaks, error
codes, and streaming chunk boundaries bit-for-bit, reaching **83/83** exact parity in Node, Chrome,
and Firefox for both `dist/index.min.js` and `dist/index.js` (`check.mjs --readable`).

| File | What it is |
| --- | --- |
| `src/errors.dart` | `FlateErrorCode` (`0..6`) and `FlateException` |
| `src/tables.dart` | Immutable `Tables` bundle initialized once (`fleb`, `fdeb`, `clim`, `fl`, `revfl`, `fd`, `revfd`, `rev`, `flt`, `fdt`, `flm`, `flrm`, `fdm`, `fdrm`, `deo`, `crct`, `emptyU8`) and `buildHMap` (`hMap`) |
| `src/huffman.dart` | `hMap`, `hTree` (flat typed-array nodes, stable insertion sorts, UZIP.js two-queue merge, reverse-topological depth computation, 15/7-bit Kraft debt clamping & rollback), `lc` (preserving empty-tree `{ c: [0], n: 1 }`), `clen` |
| `src/deflate.dart` | `DeflateOptions`, `DeflateState`, `wbits`, `wbits16`, `wfblk`, `wblk` (exact stored/fixed/dynamic tie-breaks), `dflt` (32 KiB sliding window, hash chains, rarest-2-byte jump, level-0 `i < s + lst` splitting), `dopt`, `deflateSync` |
| `src/inflate.dart` | `InflateOptions`, `InflateState`, `shft`, `subarray`, `slc`, `inflt` (zero-padded OOB bit reads, dynamic tree decoding, overlapping RLE copy, custom dictionary back-references, `!resize` truncation vs stored `RangeError`), `inflateSync` |
| `src/framing.dart` | `Crc32`, `Adler32`, `wbytes`, `gzhl`/`gzh`/`gzs`/`gzl`, `gzipSync`, `gunzipSync`, `zlh`/`zls`, `zlibSync`, `unzlibSync`, `decompressSync` |
| `src/stream.dart` | Synchronous streaming classes: `Deflate`, `Gzip`, `Zlib`, `Inflate`, `Gunzip` (recursive multi-member emission), `Unzlib`, `Decompress` |
| `src/codec.dart` | Pure-Dart `strToU8` (UTF-8 and Latin-1), WHATWG `strFromU8` (single leading BOM strip, maximal-subpart `U+FFFD` replacement, Latin-1), and `stringToUtf16Le` |
| `src/main.dart` | `dart:js_interop` boundary (port of `libraries/fflate/reference.mjs`); `main()` publishes `run` on `globalThis.__matrixExports` |

## Architecture and boundary decisions

- **Zero-copy `Uint8List` / `JSUint8Array` boundary.** On dart2js, `dart:typed_data` (`Uint8List`,
  `Uint16List`, `Int32List`) compiles directly to JS typed arrays (`Uint8Array`, `Uint16Array`,
  `Int32Array`), and `Uint8List.toJS` returns the underlying JS `Uint8Array` without copying.
- **Realm-safe `dart:js_interop` input reading.** Corpus inputs are read directly through
  `dart:js_interop` extension types (`InputJson`, `PayloadJson`, `CompressOptionsJson`,
  `DecompressOptionsJson`, `StreamStepJson`) without `dartify()` (which depends on the current
  realm's `Object.prototype`). Strings stay JS strings (Dart `String` on the web, UTF-16) and are
  encoded straight into `Uint8List` via `strToU8`.
- **Exact `fflate@0.8.3` algorithmic fidelity.**
  - `hTree`: flat `Uint16List`/`Int32List` node arrays avoid per-node heap allocation; two stable
    insertion sorts preserve `(f, s)` and `(-depth, f, s)` tie-breaks (Dart's `List.sort` is not
    guaranteed stable); the UZIP.js linear merge keeps strict `<`; depths are computed in one
    reverse-topological pass without recursion.
  - `dflt`: allocates `+2` slack bytes inside `w` before slicing to `pre + shft(pos) + post` so
    `wbits16` never triggers Dart bounds checks on the final EOB; guards `i >= dif` before hashing
    `i - dif`; keeps `bs = 0` on the first block and exact `i < s + lst` at level 0; computes
    default `plvl` via exact integer cutoffs matching `Math.ceil(Math.max(8, Math.min(13, Math.log(len))) * 1.5)`
    without importing `dart:math`.
  - `inflt`: zero-pads OOB reads in `_bits`, inlined `bits16`, and stored-block headers; splits
    back-reference copying into a bounds-checked slow path for `!resize` truncation and an unguarded
    fast path when `end <= bufLen`; lets `Uint8List.setRange` throw `RangeError` on stored-block
    overflow when `!resize`, matching JS `TypedArray.prototype.set`; uses a JS-compatible
    `subarray` helper everywhere `fflate` passes negative indices or expects clamping.
  - `codec`: implements pure-Dart UTF-8/Latin-1 `strToU8` and the normative WHATWG Encoding Standard
    UTF-8 state machine in `strFromU8` (stripping a single leading `EF BB BF` BOM and emitting exact
    maximal-subpart `U+FFFD` replacements), avoiding `dart:convert` bundle overhead and guaranteeing
    bit-exact parity with host `TextDecoder`.
  - `stream`: `Gunzip.push` reproduces `fflate`'s recursive multi-member progression literally so
    single- and multi-member streams emit identical `(chunk, false)` / `(empty, false)` /
    `(empty, true)` sequences.
- **No Firefox Marionette CCW workaround.** Per `CONTRACT.md`, the port stays strictly idiomatic and
  does not alter behavior for Firefox WebDriver cross-compartment wrappers.

## Lint columns

- **Embedded JS:** `0`
- **Eval references:** `0`
- **Host JS:** `0`
- **Module state:** `1` — `final tables = Tables._init();` (`src/tables.dart:7`). This single
  immutable object holds `fflate`'s own 16 module-load lookup tables (`fleb`, `fdeb`, `clim`, `fl`,
  `revfl`, `fd`, `revfd`, `rev`, `flt`, `fdt`, `flm`, `flrm`, `fdm`, `fdrm`, `deo`, `crct`) plus
  `emptyU8`, computed once and never mutated. Bundling them into one `final` instance also reduces
  dart2js lazy-initializer checks to one per top-level call. No inputs, compressed outputs, or
  scratch buffers are ever cached across calls.

## dart2js 3.14 findings (no compiler bugs)

No compiler bug or refusal was hit. Inspecting `dart compile js -O2 --no-minify` on hot paths
revealed four `-O2` codegen characteristics that directly guided idiomatic Dart 3 choices:

1. **Variable shift counts emit `B.JSInt_methods.$shl` unless masked into `0..31`.** In `dflt`,
   `dat[i + 1] << bs1` and `dat[i + 2] << bs2` originally compiled to `JSInt_methods.$shl` calls
   (2–4 calls per input byte) because dart2js did not prove `0 <= bs1, bs2 < 32` across
   `(plvl + 2) ~/ 3`. Masking `bs1 = ((plvl + 2) ~/ 3) & 31` and `bs2 = (2 * bs1) & 31` let dart2js
   emit native JS `<<`.
2. **Signed `>>` emits `B.JSInt_methods._shrOtherPositive$1` when non-negativity isn't proven;
   `>>>` compiles directly to native JS `>>>`.** In `wblk`, `p >> 3` and `sym >> 18` called
   `_shrOtherPositive$1` because `p` is a parameter and `sym` comes from `Int32List`. Switching
   guaranteed non-negative shifts in `wblk`, `wbits`, `wbits16`, `wfblk`, `shft`, and `dflt` to
   Dart 3's `>>>` eliminated every helper call in bit emission.
3. **Reassigning a `Uint8List` local inside a loop introduces per-write `$flags & 2` unmodifiable-view
   checks.** In the initial `strToU8`, growing `out` via reassignment lost dart2js's freshly-allocated
   fact and emitted `out.$flags & 2 && A.throwUnsupportedOperation(out)` (a slow missing-property
   lookup on JS `Uint8Array`) on every byte. Pre-allocating a `final out = Uint8List(len * 3)`
   removed the check and cut `strToU8(64 KiB)` from ~0.245 ms to ~0.147 ms.
4. **`Int32List` vs `Uint32List` and `((cr ^ d[i]) & 255)` in CRC-32.** Using `Int32List(256)` for
   `crct` (matching `fflate`'s `Int32Array(256)`) and writing `crct[(cr ^ d[i]) & 255]` instead of
   `crct[(cr & 255) ^ d[i]]` eliminated the per-byte bounds check in dart2js and cut 64 KiB CRC-32
   from ~0.279 ms to ~0.061 ms in V8.
5. **Unboxed loops without capturing closures.** Avoiding `fflate`'s nested mutating closures
   (`cbuf` in `inflt`, `hsh` in `dflt`, `w` in `lc`) prevents dart2js from boxing local variables
   (`buf`, `cli`) into heap cells.

## Size and speed

- **Size.** `dist/index.min.js` is **83,913 B** (**29,481 B** gzip-9, **25,728 B** brotli-11);
  `dist/index.js` is **95,831 B** (`dart2js` emits **95,509 B** before the facade and Terser).
  The Dart runtime floor (`ports/floor/dart/dist/index.min.js`) is **38,726 B**, leaving ~45.2 KB
  for the full `fflate` port plus typed-data, string, and error runtime support. For scale, the
  bundled JS reference (`reference.mjs` + `fflate@0.8.3`) is **17,623 B** (**7,340 B** gzip-9).
- **Quick Node perf check** (`bench.mjs --environments node --samples 6`, triplet design, bench
  variants enabled; load average ~4–6):
  - `bench-compress` (64 KiB `gzipSync` L6): **~0.91 ms** vs JS **~0.74 ms** (**~1.22×** JS, A/A
    ~1.00). The first version measured 1.52× before the shift, `strToU8`, and `Crc32` improvements
    above; most of the remaining ~0.17 ms gap is pure-Dart UTF-8 encoding (~0.15 ms) vs V8's C++
    `TextEncoder.encode()` (~0.015 ms).
  - `bench-decompress` (128 KiB `gunzipSync`): **~0.43 ms** vs JS **~0.48 ms** (**~0.89×** JS, A/A
    ~0.97) — slightly faster than the JS reference in Node thanks to unboxed `inflt` bit reads and
    avoiding the `cbuf` closure cell.

## Verification

- **Corpus parity:** `83/83` exact byte-for-bit parity in Node 26, Chrome 158, and Firefox 160 for
  both `dist/index.min.js` and `dist/index.js` (`check.mjs --readable`).
- **Differential fuzzing:** A 1,423-case randomized differential suite in Node against
  `libraries/fflate/reference.mjs` passed `1,423/1,423` (0 failures, 46 `both-threw`), covering
  levels `0..9`, `mem 0..12`, dictionaries `0..40 KiB`, streaming inflate/gunzip/unzlib/decompress
  dynamic splits (including multi-member GZIP), 100–190 KiB streaming deflate buffer growth and
  window wraps, `outSize` edge cases, corrupt/bit-flipped streams, all error codes `0..6`, and 200+
  random ill-formed UTF-8 byte sequences.
- **Reproducibility:** Two consecutive `--clean` builds are byte-identical
  (`reproducibility.identical === true`, SHA-256 `0de698926b91`).
