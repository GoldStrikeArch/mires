# fflate 0.8.3: Rust port (mires)

A faithful port of the synchronous compression, decompression, framing, streaming, and text-codec
subset defined in `libraries/fflate/CONTRACT.md`: the same algorithms, tables, tie-breaks, error
codes, and chunk boundaries as `node_modules/fflate/esm/browser.js`, in safe `no_std` Rust
(`#![deny(unsafe_code)]`, no `js!{}`). It reaches exact bit-for-bit parity on all 83 corpus cases
in Node, Chrome, and Firefox (both minified and readable artifacts).

| File | What it is |
| --- | --- |
| `src/tables.rs` | Compile-time (`const fn`) immutable tables (`FLEB`, `FDEB`, `CLIM`, `FL`, `REVFL`, `FD`, `FLT`, `FDT`, `FLM`, `FLRM`, `FDM`, `FDRM`, `DEO`, `CRCT`), plus O(1) `rev15`, `revfd`, and `default_plvl` |
| `src/huffman.rs` | `h_map`, `h_tree` (flat node arrays + custom stable insertion sort, UZIP.js linear merge, 15-bit/7-bit Kraft debt clamping & rollback), `lc`, `clen` |
| `src/deflate.rs` | `wbits`, `wfblk`, `wblk` (stored/fixed/dynamic cost comparison & tie-breaks), `dflt` (32 KiB sliding window, hash chains, rarest-2-byte jump, level-0 stored splitting), `dopt`, `deflate_sync` |
| `src/inflate.rs` | `inflt` (zero-padded OOB bit reads, dynamic tree decoding, overlapping RLE copy, custom dictionary back-references, `outSize` non-resizing truncation vs stored `RangeError`), `inflate_sync` |
| `src/framing.rs` | `Crc32`, `Adler32`, `wbytes`, `gzhl`/`gzh`/`gzs`/`gzl`, `gzip_sync`, `gunzip_sync`, `zlh`/`zls`, `zlib_sync`, `unzlib_sync`, `decompress_sync`, `FlateError` |
| `src/stream.rs` | `Deflate`, `Gzip`, `Zlib` (`CompressStream`), `Inflate`, `Gunzip` (recursive multi-member), `Unzlib`, `Decompress`, and `[u32le(len), final, ...chunk]` framing (`StreamSink`) |
| `src/codec.rs` | `strFromU8` WHATWG UTF-8 decoder (single leading BOM strip, maximal-subpart `U+FFFD` replacement, UTF-16LE output) and Latin-1 decoder |
| `src/boundary.rs` | `JsValue` input parsing (zero-copy UTF-8 `&str` borrow, `charCodeAt` Latin-1, byte arrays), error-code framing, and `JsUint8Array::from_slice` bulk output |

## Decisions

- **Exact `fflate 0.8.3` algorithms, not an alternative DEFLATE crate.** Every tie-break in
  `hTree` (strict `<` in queue merge, stable sorting by frequency then symbol), `lc` (empty
  distance tree returning `{ c: [0], n: 1 }`), `wblk` (stored wins ties, fixed wins ties over
  dynamic, `bs = 0` on the first block), `dflt` (exact `deo` table, rarest-2-byte jump,
  `i < s + lst` at level 0), `inflt` (zero-padded OOB reads, byte-by-byte RLE, `outSize`
  semantics), and `Gunzip` (recursive multi-member emission sequence) matches `browser.js`.
- **Zero cross-call mutable state.** Only compile-time immutable `static` arrays live at module
  scope (`0` module-state lint findings). `rev15` (4 bitwise ops) and `revfd` (`leading_zeros`)
  are computed in O(1) to avoid emitting 32 KiB static table literals into JS.
- **No `core::slice::sort` and no `core::fmt`.** `h_tree` sorts at most 288 symbols using a
  20-line in-place stable insertion sort (avoiding ~18 KB per `core::slice::sort` instantiation
  under mires), and errors carry static `&'static str` messages / numeric codes.
- **Zero-copy UTF-8 input & bulk byte output.** UTF-8 `{ text }` payloads borrow
  `text.read::<&str>().as_bytes()` directly without copying; outputs return in one bulk host copy
  via `mires_js::JsUint8Array::from_slice`.
- **Hot-loop slice binding and macro inlining.** Because the matrix builds Rust with
  `JSC_RUST_MIR_INLINE=off`, slice references (`&[u8]`, `&mut [u8]`, `&mut [u16]`, `&mut [u32]`)
  are bound outside inner loops (`dflt`, `inflt`, `read_latin1_bytes`, `h_map`) to avoid repeated
  `Vec` derefs, and the tightest inner-loop bit helpers (`read_bits16!` in `inflt`,
  `write_bits16!` in `emit_symbols`) are expanded locally.

## mires bugs and performance cliffs

- **Compiler bugs / refusals met:** none (`339/339` items compiled cleanly: `198` JIR, `141`
  non-ordinary fallback, `ordinaryFallback: 0`; `83/83` parity across Node, Chrome, and Firefox).
- **Performance cliff logged (`PERF-002` in `MIRES-BUGS.md`):** V8 CPU profiling on
  `bench-compress` + `bench-decompress` shows `16.4%` (`13.3% $rt_alloc_zeroed` + `3.1%
  $rt__retype`) spent double-zeroing `Vec<u8>`/`Vec<u16>`/`Vec<u32>` allocations.
  `$rt_alloc_zeroed` allocates a tagged JS `new Array(bytes).fill(0)` (2× over-allocated for
  `u16`, 4× for `u32`), and `$rt__retype` then shrinks `p.buf.length` and runs a second
  `for (let i = 0; i < length; i++) p.buf[i] = mk()` closure loop (even for `u8`), whereas `fflate`
  uses zeroed OS-backed `Uint8Array`/`Uint16Array`/`Int32Array`. Together with JS boxed `Array`
  storage and bounds checks on `while`-loop slice indexing, this accounts for the ~2.2–3.5× gap
  against JS in Node.
- **Firefox Marionette CCW timing note:** Under the current `browser.mjs` WebDriver harness,
  Firefox's `executeAsyncScript` wraps `{ text }` inputs in cross-compartment wrappers (CCWs),
  making `TextEncoder.encode()` in the JS reference `bench-compress` path take ~14 ms under
  WebDriver (producing an artificially low `port/js` ratio on Firefox `bench-compress`). This port
  does not work around that harness artifact; see `libraries/fflate/CONTRACT.md`.

## Size

`dist/index.min.js` is 108,605 B (exact raw / gzip-9 / brotli-11 figures are in
`build/library-matrix/sizes.json` and `MATRIX.md`). By `optimizer-report.json` (`110,432 B`
linked `program.js` before Rollup/Terser): `96,912 B` (`96.9 KB`, 186 items) is `matrix_fflate`
itself (including `inflt` 10.5 KB, `dflt` 6.5 KB, `h_tree` 5.4 KB, `run_decompress_stream` 3.7 KB,
`wblk` 3.7 KB, `str_from_u8_utf16le` 3.5 KB, static tables `CRCT`/`FLRM`/`FLM`/`REVFL` ~7.2 KB),
`7,379 B` link-generated wrappers (135 items), `3,599 B` `alloc`, `1,411 B` `core`, `1,032 B`
`mires_js`, plus `26,773 B` (`26.8 KB`) minimal `runtime/shim.js` in Rollup attribution.
