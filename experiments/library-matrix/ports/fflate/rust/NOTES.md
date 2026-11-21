# fflate 0.8.3: Rust port (mires)

A faithful port of the synchronous compression, decompression, framing, streaming, and text-codec
subset defined in `libraries/fflate/CONTRACT.md`: the same algorithms, tables, tie-breaks, error
codes, and chunk boundaries as `node_modules/fflate/esm/browser.js`, in safe `no_std` Rust
(`#![deny(unsafe_code)]`, no `js!{}`). It reaches exact bit-for-bit parity on all 89 corpus cases
in Node, Chrome, and Firefox (both minified and readable artifacts). One input class is outside
what the compiler can run: a GZIP trailer claiming an ISIZE of 128 MiB or more (see "Known compiler
limit (MIRES-021)").

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
  semantics, typed-array output stores), and `Gunzip` (recursive multi-member emission sequence)
  matches `browser.js`.
- **`inflt` output stores behave like `Uint8Array` stores.** fflate writes `buf[bt++] = sym`, so a
  store past the end of the buffer is dropped while `bt` advances, and the buffer grows (`cbuf`)
  only for a stored block, at the start of a Huffman block and before a back-reference. The port
  bounds-checks every store the same way and never grows the buffer for a literal, so a run of more
  than 131,072 literals in one block loses its tail exactly as in fflate (corpus
  `inflate-literal-run-200k` and its three siblings). Before the 2026-11-20 review the port grew
  the buffer for literals too and returned the whole run.
- **Zero cross-call mutable state.** Only compile-time immutable `static` arrays live at module
  scope (`0` module-state lint findings). `rev15` (4 bitwise ops) and `revfd` (`leading_zeros`)
  are computed in O(1) to avoid emitting 32 KiB static table literals into JS.
- **No `core::slice::sort` and no `core::fmt`.** `h_tree` sorts at most 288 symbols with two
  in-place stable insertion sorts (`sort_by_freq_asc`, `sort_for_clamp`), and errors carry static
  `&'static str` messages / numeric codes. `sort_by` in place of the insertion sorts gives the same
  89/89 results but adds 16.9 KB to `index.min.js` for the first and 11.3 KB for the second
  (15.6 KB and 12.7 KB of core's driftsort by the optimizer report; measured on 2026-11-21).
- **Zero-copy UTF-8 input & bulk byte output.** UTF-8 `{ text }` payloads borrow
  `text.read::<&str>().as_bytes()` directly without copying; outputs return in one bulk host copy
  via `mires_js::JsUint8Array::from_slice`.
- **Performance accommodations for mires (code kept as is).** Three code shapes are there for the
  compiler, not for Rust; they answer the hot spots logged as performance cliffs
  [`PERF-002` and `PERF-004`](../../../MIRES-BUGS.md#performance-cliffs):
  - the `read_bits16!` macro in `inflt` (`src/inflate.rs`) and the `write_bits16!` macro in
    `emit_symbols` (`src/deflate.rs`) stand in for small `#[inline]` helper functions. The matrix
    builds with `JSC_RUST_MIR_INLINE=off`, so a helper would stay one JS call per decoded or
    emitted symbol;
  - slice references (`&[u8]`, `&mut [u8]`, `&mut [u16]`, `&mut [u32]`) are bound once outside the
    inner loops of `dflt`, `inflt`, `read_latin1_bytes` and `h_map` instead of indexing the `Vec`s
    (`PERF-004`, the slice-hoisting rewrite measured on earcut);
  - loops are written `while i < n { ...; i += 1 }` with explicit indices rather than
    `for i in 0..n` or iterator chains, so no `Range` or iterator adaptor has to be inlined by a
    compiler that runs with MIR inlining off. It is not the idiomatic Rust form; `PERF-002`
    records that the remaining gap to fflate is boxed `Array` storage plus the bounds checks these
    loops keep.

## mires bugs and performance cliffs

- **Compile refusals met:** none (`339/339` items compiled cleanly at the original port: `198` JIR,
  `141` non-ordinary fallback, `ordinaryFallback: 0`). Runtime defects met: one,
  [`MIRES-021`](../../../MIRES-BUGS.md) (below).
- **Known compiler limit (`MIRES-021`).** `gunzip_sync` allocates its output as
  `vec![0u8; ISIZE]` (`src/framing.rs`, `gunzip_sync`), as fflate allocates `new u8(gzl(data))`.
  Under mires a zeroed `Vec<u8>` is a boxed JS `Array` (`runtime/shim.js` `alloc_zeroed`:
  `new Array(size).fill(0)`), not a `Uint8Array`. A GZIP whose trailer claims an ISIZE far above
  the real output therefore fails where fflate returns the decoded bytes: from about 128 MiB the
  allocation throws `RangeError: Invalid array length`, around 512 MiB Node dies with an
  uncatchable heap out-of-memory, and from 2 GiB the `Layout` overflow panics. 64 MiB already
  costs about 1.2 GB of resident memory. The port does not work around it (a smaller or lazily
  grown buffer would change fflate's `!resize` truncation semantics); such inputs are outside the
  corpus (`CONTRACT.md`, Parity domain). Repro:
  [`mires-bugs/MIRES-021-alloc-zeroed-large-array/`](../../../mires-bugs/MIRES-021-alloc-zeroed-large-array/).
- **Performance cliff logged (`PERF-002` in `MIRES-BUGS.md`):** V8 CPU profiling on
  `bench-compress` + `bench-decompress` shows `16.4%` (`13.3% $rt_alloc_zeroed` + `3.1%
  $rt__retype`) spent double-zeroing `Vec<u8>`/`Vec<u16>`/`Vec<u32>` allocations.
  `$rt_alloc_zeroed` allocates a tagged JS `new Array(bytes).fill(0)` (2× over-allocated for
  `u16`, 4× for `u32`), and `$rt__retype` then shrinks `p.buf.length` and runs a second
  `for (let i = 0; i < length; i++) p.buf[i] = mk()` closure loop (even for `u8`), whereas `fflate`
  uses zeroed OS-backed `Uint8Array`/`Uint16Array`/`Int32Array`. Together with JS boxed `Array`
  storage and bounds checks on `while`-loop slice indexing, this accounts for the ~2.2–3.5× gap
  against JS in Node.

## Size

`dist/index.min.js` is 108,605 B (exact raw / gzip-9 / brotli-11 figures are in
`build/library-matrix/sizes.json` and `MATRIX.md`). By `optimizer-report.json` (`110,432 B`
linked `program.js` before Rollup/Terser): `96,912 B` (`96.9 KB`, 186 items) is `matrix_fflate`
itself (including `inflt` 10.5 KB, `dflt` 6.5 KB, `h_tree` 5.4 KB, `run_decompress_stream` 3.7 KB,
`wblk` 3.7 KB, `str_from_u8_utf16le` 3.5 KB, static tables `CRCT`/`FLRM`/`FLM`/`REVFL` ~7.2 KB),
`7,379 B` link-generated wrappers (135 items), `3,599 B` `alloc`, `1,411 B` `core`, `1,032 B`
`mires_js`, plus `26,773 B` (`26.8 KB`) minimal `runtime/shim.js` in Rollup attribution.
