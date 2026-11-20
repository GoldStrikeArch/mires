# fflate 0.8.3: Elm (+ JS host) port

A faithful port of the synchronous compression, decompression, framing, streaming, and text-codec
subset defined in `libraries/fflate/CONTRACT.md` to Elm 0.19.2: the exact algorithms, tables,
tie-breaks, error codes, and chunk boundaries of `node_modules/fflate/esm/browser.js`, using only
the default packages (`elm/core 1.0.5`, `elm/json 1.1.3`). The JS host (`js/index.js`, 27 non-blank
lines, 1,133 bytes) is host I/O only. It reaches exact bit-for-bit parity on all 89 corpus cases in
Node, Chrome, and Firefox for both minified (`dist/index.min.js`) and readable (`dist/index.js`)
artifacts.

| File | What it is |
| --- | --- |
| `src/ByteBuf.elm` | 8-way sparse persistent integer radix tree (`IntTree`: `get`, `set`, single-pass `swap`, `inc`) and 4-byte-packed `Bytes` / `ByteBuilder` (O(1) slice view, 32-byte positional tail buffer, fast unrolled tree folds, `getBits`/`getBits16`/`getU24Le`) |
| `src/Tables.elm` | Immutable top-level lookup tables (`fleb`, `fdeb`, `clim`, `fl`, `revfl`, `fd`, `flt`, `fdt`, `crct`), plus O(1) `rev15`, `revfd`, `deoNice`/`deoChain`, and `defaultPlvl` |
| `src/Huffman.elm` | Canonical Huffman `hMap`, fixed maps (`flm`, `flrm`, `fdm`, `fdrm`), length-limited `hTree` (stable sort, UZIP.js two-queue linear merge with strict `<`, signed Kraft debt clamping & rollback), `lc` RLE encoding, `clen` |
| `src/Deflate.elm` | Sequential `BitWriter`, `wfblk`, `wblk` (stored/fixed/dynamic cost comparison & tie-breaks), `dflt` (32 KiB sliding-window hash chains, rarest-2-byte jump, level-0 stored splitting), `dopt`, `deflateSync`, `syncFlushBytes` |
| `src/Inflate.elm` | `inflt` state machine (zero-padded OOB reads, dynamic tree decoding, overlapping RLE back-references, custom dictionary lookback, `outSize` non-resizing truncation vs stored `RangeError`), `inflateSync`, `FlateError` |
| `src/Framing.elm` | `crc32`, `adler32`, GZIP header/trailer (`gzhl`, `gzhBytes`, `gzs`, `gzl`, `gzipSync`, `gunzipSync`), Zlib header/trailer (`zlhBytes`, `zls`, `zlibSync`, `unzlibSync`), and `decompressSync` auto-detection |
| `src/Codec.elm` | `strToU8` (UTF-8 and Latin-1 via `String.foldl`) and `strFromU8Utf16Le` (WHATWG Encoding Standard UTF-8 decoder: single leading BOM strip, maximal-subpart `U+FFFD` replacement, surrogate pairs -> UTF-16LE bytes) |
| `src/Stream.elm` | All 7 sync streaming classes (`Deflate`, `Gzip`, `Zlib`, `Inflate`, `Gunzip` recursive multi-member, `Unzlib`, `Decompress`) and `[u32le(len), final, ...chunk]` framing |
| `src/Main.elm` | `Platform.worker` boundary (`()` model): decodes corpus inputs with `Json.Decode`, runs the op, handles `expectErrorCode`, encodes output bytes via `Json.Encode` |
| `js/index.js` | Minimal I/O-only JS host: sends `input` through `request`, receives synchronous `response`, throws on `error`, wraps `response.ok` in `new Uint8Array(...)` |

## Architecture & Decisions

- **Minimal I/O-only boundary (`js/index.js`):** Zero compression, decompression, framing, CRC,
  Adler, or UTF-8 logic lives in JS. `js/index.js` keeps only the two standard Elm host slots
  (`let app`, `let response` — the 2 `moduleState` lint findings, identical to `pilot` and
  `minisearch`). The `Platform.worker` model stays `()` so nothing persists across calls; all
  module-level tables in `Tables.elm` and `Huffman.elm` are pure immutable Elm values.
- **Persistent 8-way radix trees (`IntTree`) instead of `elm/core` `Array`:**
  - `elm/core`'s `Array.get` boxes every read in `Maybe` (`Just val`), and `Array.set` copies
    32-element JS arrays per level (96–128 copies for 32 KiB–128 KiB tables).
  - Our 8-way sparse `IntTree` returns unboxed `Int` directly (0 allocations on `get`, default `0`
    on `Empty` subtrees), allocates a 1,048,576-entry `plvl = 20` `head` table in O(1), and combines
    `head[hv]` read+write into a single-traversal `intTreeSwap`.
  - `Bytes` packs 4 bytes little-endian per 32-bit `Int` (32 bytes per `Leaf8`) with an O(1)
    `(offset, length)` slice view so `data.subarray(st, -8)` in `gunzipSync`, `slc` in streaming
    `Inflate`/`Gunzip`/`Unzlib`, and stored-block slices perform zero copying. `getBits16` and
    `getU24Le` read 3 consecutive bytes in 1 tree lookup (50% of alignments) or 2 lookups instead of
    3, and `foldl`/`foldr` walk leaves directly for `crc32`, `adler32`, and final JSON encoding.
  - `ByteBuilder` buffers up to 32 bytes in two positional 4-word structs (`Word4`) before flushing
    a complete `Leaf8` once every 32 bytes. Using positional constructors rather than record-update
    syntax (`{ b | ... }`, which compiles to `_Utils_update`'s `for..in` loop) eliminated 28.9% of
    CPU time in V8 profiling and more than doubled `bench-decompress` speed (33.8 ms → 14.5 ms).
- **`inflt` output models fflate's buffer, not just its bytes:** inflate output goes to an
  append-only `ByteBuilder`, while fflate writes into a `Uint8Array` `buf` whose length matters:
  `buf[bt++] = sym` drops a store past the end while `bt` advances, and the buffer grows (`cbuf`,
  to `max(2 * length, need)`) only for a stored block, at Huffman block start and before a
  back-reference. `Inflate.elm` threads that length (`cap`: `sl * 3`, the caller's `outSize`, or
  the stream window's length) through `decodeSymbols` and keeps the builder equal to
  `buf[0 .. min(bt, cap)]`; when the buffer grows past `bt`, the dropped positions are filled with
  the new buffer's zeros. So a run of more than 131,072 literals in one block loses its tail
  exactly as in fflate. Before the 2026-11-20 review only `outSize`/ISIZE buffers were capped, and
  resizable ones kept every literal.
- **Sequential `BitWriter` for deflate output:** Since `dflt`/`wblk`/`wfblk` emit bits strictly
  forward (saving only the 0..7-bit partial byte in `st.r` on non-final stream calls), output is
  built directly onto a reversed `List Int` without allocating or mutating an output byte array.
  GZIP/Zlib headers and footers are prepended/appended cleanly.
- **Stack safety & Elm 0.19.2 codegen constraints:**
  - Every loop over arbitrarily long inputs (`dfltLoop`, `searchChain`, `matchLen`, `findRarest`,
    `decodeSymbols`, `copyBackref`, `copyDict`, `runBlocks`, `gunzipPush`, `strFromU8Utf16Le`,
    `adlerUpdate`, `frameRecordsRev`) is strictly **self-tail-recursive** (compiled to JS `while`
    loops; no mutual recursion), so 136 KiB streaming chunks and 128 KiB decompressions never risk
    blowing the JS call stack.
  - Hot recursive loops stay within Elm's 9-parameter `F2..F9` fast-call ceiling (`dfltLoop` is
    exactly `F9`, packing static per-call config into one immutable record) and use literal-anchored
    difference comparisons `(a - b) < 0` / `(a - b) == 0` so Elm emits native JS `<` / `===`
    instead of generic `_Utils_cmp` / `_Utils_eq`.
  - `hTree` uses a functional two-list FIFO queue for the UZIP.js linear merge (strict `<` picks
    leaves on frequency ties) and an explicit `(depth desc, freq asc, sym asc)` comparator for
    clamping.
  - Unsigned 32-bit shifts (`Bitwise.shiftRightZfBy`) are used everywhere unsigned semantics are
    required (CRC-32, `wbytes`, `u32LeBytes`, packed byte extraction), while `Bitwise.shiftRightBy`
    is used for signed `dt >>= lft` in `hTree` and `Bitwise.and -32768 endLen` in `Deflate` buffer
    growth. GZIP `mtime` is decoded as `Float` and converted via `floor (m / 1000)` +
    `Bitwise.shiftRightZfBy 0`, handling the full `0..0xFFFFFFFF` timestamp range without 32-bit
    signed truncation.
  - String iteration uses `String.foldl` only (never `String.foldr` or `String.toList`, avoiding
    `elm/core 1.0.5`'s lone-surrogate infinite-loop defect). `strToU8` handles astral code points
    for both UTF-8 (4-byte sequences) and Latin-1 (emitting both surrogate low bytes).

## Size & Performance

Measured against `build/library-matrix/dev-elm/receipts/`:

- **Bundle size (`dist/index.min.js`):** 54,428 B raw / 18,218 B gzip-9 / 15,910 B brotli-11
  (readable `dist/index.js`: 224,196 B; 4,368 Elm code lines across 9 files). Reproducible across
  clean builds (`identical: true`).
- **Attribution & lints:** `elmRuntimeMinified` 6,484 B; `hostJs` 1,133 B (27 lines, `2.08%`
  comparator share, `0.49%` rendered share — `mostlyJs: false`); `embeddedJs: 0`, `evalRefs: 0`,
  `moduleState: 2` (`let app`, `let response` in `js/index.js`).
- **Quick Node benchmark (`tools/bench.mjs --environments node --samples 6`, load ~4.1):**
  - `bench-compress` (64 KiB `gzipSync` level 6): **33.50 ms** p50 vs JS 0.74 ms (`port/js` 45.37×,
    paired 45.16×, A/A 1.041 `ok`).
  - `bench-decompress` (128 KiB `gunzipSync`): **14.70 ms** p50 vs JS 0.46 ms (`port/js` 31.90×,
    paired 32.19×, A/A 1.002 `ok`).
  - Both finish smoothly with >900× margin below the harness's 30 s `--call-timeout`. Remaining time
    in Node CPU profiles is dominated by persistent radix-tree lookups and path copying
    (`intTreeGet` ~35%, `intTreeSwap`/`intTreeSet`/`Node8` ~26%, GC ~8%), the inherent cost of pure
    immutable data structures in Elm 0.19.2.

## Verification

- **Corpus parity:** `89/89` exact byte-for-byte parity in Node, Chrome, and Firefox for both
  `dist/index.min.js` and `dist/index.js` (`check.mjs --environments node,chrome,firefox --readable`).
- **Beyond the corpus:**
  - Exhaustively verified O(1) `revfd(1..32768)` and `defaultPlvl(0..400000)` against `browser.js`
    with 0 mismatches.
  - 820-case randomized differential fuzz in Node against `libraries/fflate/reference.mjs` (all
    levels `0..9`, `mem 0..8`, `<32K`/`=32K`/`>32K` dictionaries, full 32-bit unsigned GZIP `mtime`,
    1–3-member streaming `Gunzip`, awkward streaming `Inflate`/`Unzlib`/`Decompress` chunk splits,
    streaming `Deflate`/`Gzip`/`Zlib` with flushes, `outSize` edge cases, ill-formed WHATWG UTF-8
    sequences, and truncated/corrupt streams): `820/820` identical.
  - 386-case targeted regression & streaming fuzz covering `mem = 9..12` (including bits 21..23
    hash-bucket collisions up to `2^24` buckets in `shiftForSize`) and 2-/3-push streaming
    `Deflate`/`Gzip`/`Zlib` with omitted default `mem` across levels `1..9` (verifying `DeflateState`
    retains `hShift` when `plvl` drops from `20` on non-final chunks to `18` on the final chunk):
    `386/386` identical.
