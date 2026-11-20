# fflate 0.8.3: Melange port

A faithful port of the synchronous compression, decompression, framing, streaming, and text-codec
subset in `libraries/fflate/CONTRACT.md`, written as **one OCaml program compiled by two
compilers**: this directory (Melange 7.0.1) and `../jsoo/` (js_of_ocaml 6.4.1) share the same
9-module OCaml core byte-for-byte, differing only in `src/host.ml` and the entry module
(`src/index.ml` vs `src/main.ml`). It reaches bit-exact parity on all 83 corpus cases in Node,
Chrome, and Firefox (both minified and readable artifacts), plus 760 randomized differential cases
against `libraries/fflate/reference.mjs`.

| File | Shared with `../jsoo/` | What it is |
| --- | --- | --- |
| `src/host.mli` | identical | JS host interface: `U8`/`U16`/`I32` typed arrays, `Text` (UTF-16 + `TextEncoder`/`TextDecoder`), in-place `Value` JSON reader, `date_now` |
| `src/tables.ml` | identical | `Flate_error`, `shft`/`slc`/`last_32768`, and module-load immutable tables (`fleb`, `fdeb`, `clim`, `fl`, `revfl`, `fd`, `revfd`, `rev`, `flt`, `fdt`, `deo`, `crct`) |
| `src/huffman.ml` | identical | `h_map`, precomputed `flm`/`flrm`/`fdm`/`fdrm`, `h_tree` (flat node arrays, custom stable bottom-up merge sort, UZIP.js linear merge with strict `<`, Kraft debt clamping & signed rollback), `lc`, `clen` |
| `src/deflate.ml` | identical | `wbits`/`wbits16`, `wfblk`, `wblk` (stored/fixed/dynamic cost comparison & tie-breaks), `default_plvl`, `dflt` (32 KiB sliding window, hash chains, rarest-2-byte jump, level-0 splitting), `dopt`, `deflate_sync` |
| `src/inflate.ml` | identical | `bits`/`bits16` (zero-padded OOB reads), `inflt` (dynamic trees, overlapping RLE copy, custom dictionary back-references, `outSize` non-resizing truncation vs stored `RangeError`), `inflate_sync` |
| `src/framing.ml` | identical | `crc` (CRC-32), `adler` (Adler-32), `wbytes`, `u32_of_float`, `gzhl`/`gzh`/`gzs`/`gzl`, `gzip_sync`/`gunzip_sync`, `zlh`/`zls`, `zlib_sync`/`unzlib_sync`, `decompress_sync` |
| `src/codec.ml` | identical | `str_to_u8`, `str_from_u8`, and `text_to_utf16le` |
| `src/stream.ml` | identical | All 7 sync streaming classes (`Deflate`, `Gzip`, `Zlib`, `Inflate`, `Gunzip` recursive multi-member, `Unzlib`, `Decompress`) and `[u32le(len), final, ...chunk]` framing |
| `src/script.ml` | identical | `run(input)`: decodes corpus inputs, dispatches all 16 ops, handles `expectErrorCode` |
| `src/host.ml` | Melange only | Melange externals (`[@@mel.new]`, `[@@mel.send]`, `[@@mel.get_index]`, `[@@mel.set_index]`, `[@@mel.scope]`), `Js.Json`, `Js.Typed_array` |
| `src/index.ml` | Melange only | ES module entry: `let run input = Script.run input` |

`shasum -a 256 src/{host.mli,tables.ml,huffman.ml,deflate.ml,inflate.ml,framing.ml,codec.ml,stream.ml,script.ml}`
matches `../jsoo/src/` identically.

## Interop and algorithm decisions

- **Native JS typed arrays throughout.** `Host.U8`, `Host.U16`, and `Host.I32` wrap JS `Uint8Array`,
  `Uint16Array`, and `Int32Array` directly (avoiding Melange's `Bytes.t`, which is backed by a
  manually zero-looped JS `Array`). Buffers are OS-zeroed on allocation, `subarray`/`set` map to
  native typed-array views and memcpy (including overlapping `Deflate` window slides and stored-block
  `RangeError` on too-small `outSize`), and `run` returns the output `Uint8Array` with zero copy.
- **Exact 32-bit signed `int` semantics.** Melange and js_of_ocaml both represent OCaml `int` as a
  32-bit signed integer. Unsigned shifts (`cr lsr 8` in CRC-32, `v lsr 8` in `wbytes`, `p lsr 3`
  for bit offsets) use `lsr` (`>>>`), while Kraft debt shifting (`dt asr lft`) uses `asr` (`>>`).
  `mtime` values up to `0xFFFFFFFF` seconds (`>= 2^31`) are converted from `float` via `u32_of_float`
  (wrapping `[2^31, 2^32 - 1]` into signed two's-complement 32-bit `int` cleanly). In `gunzip_sync`,
  a GZIP ISIZE trailer with bit 31 set (`>= 2 GiB`) wraps negative in 32-bit signed `int`, so
  `sz < 0` passes `None` to `inflt` (dynamic resizing) rather than a negative length to `Uint8Array`.
- **Text codecs.** `Host.Text.t` is `string` (native JS UTF-16). UTF-8 uses module-level stateless
  `TextEncoder` / `TextDecoder` instances (matching `fflate`'s `te`/`td` and WHATWG BOM + maximal-
  subpart `U+FFFD` replacement semantics); Latin-1 uses `charCodeAt` and chunked
  `String.fromCharCode.apply(null, ...)` (16,384-byte chunks). The port does not work around the
  Firefox Marionette CCW timing artifact noted in `CONTRACT.md`.
- **Zero cross-call mutable state.** Only immutable precomputed lookup tables matching `fflate` and
  the stateless `TextEncoder`/`TextDecoder` instances exist at module scope (`0` embedded JS, `0`
  eval refs, `0` host JS, `0` module-state lint findings).

## Melange 7.0.1 compiler findings

1. **Polymorphic comparisons silently pull in `caml_obj.js` (7.6 KB before Terser).** An initial
   build omitted `(tr : int array) (sym : int array) (freq : int array)` annotations on the Huffman
   clamping sort (`sort_by_depth_freq_sym`), causing OCaml to infer polymorphic `<>` / `>` and
   import `caml_obj.js` (7,615 B) + `caml.js` (544 B). Adding the monomorphic `int array`
   annotations and avoiding polymorphic `= None` / `<> None` dropped `dist/index.min.js` from
   34,662 B to 31,269 B (-3,393 B minified) and cut Rollup runtime attribution from 14,057 B to
   5,898 B.
2. **Stdlib discipline.** Using `Stdlib.floor` (primitive `Math.floor`), flat `int array`s with a
   custom 40-line stable merge sort in `h_tree`, and zero `Hashtbl`/`Printf`/`List`/`Float` module
   imports keeps Melange's emitted stdlib to 7 small helper modules (5.9 KB before Terser).
3. No Melange miscompilation was encountered; clean rebuilds are byte-identical.

## Size and quick Node timing observations

In isolated dev verification (`build/library-matrix/dev-ocaml/`):
- `dist/index.min.js`: **31,309 B** raw, **10,959 B** gzip-9, **9,749 B** brotli-11 (Rollup
  attribution before Terser: 69,969 B program + 5,898 B Melange stdlib runtime; JS reference is
  17,623 B raw / 7,340 B gzip-9).
- Quick Node check (6 samples, triplet design, load ~5.8): `bench-compress` 1.16× JS (0.91 ms vs
  0.79 ms, A/A 0.996), `bench-decompress` 0.77× JS (0.36 ms vs 0.46 ms, A/A 0.996). Official
  matrix receipts will be recorded under `build/library-matrix/`.
