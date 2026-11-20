# fflate 0.8.3: js_of_ocaml port

A faithful port of the synchronous compression, decompression, framing, streaming, and text-codec
subset in `libraries/fflate/CONTRACT.md`, written as **one OCaml program compiled by two
compilers**: this directory (js_of_ocaml 6.4.1) and `../melange/` (Melange 7.0.1) share the same
9-module OCaml core byte-for-byte, differing only in `src/host.ml` and the entry module
(`src/main.ml` vs `src/index.ml`). It reaches bit-exact parity on all 83 corpus cases in Node,
Chrome, and Firefox (both minified and readable artifacts), plus 760 randomized differential cases
against `libraries/fflate/reference.mjs`.

| File | Shared with `../melange/` | What it is |
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
| `src/host.ml` | jsoo only | `Js_of_ocaml` host over `Typed_array.uint8Array`/`uint16Array`/`int32Array`, `Js.js_string`, `Js.Unsafe.get`/`set`, `TextEncoder`/`TextDecoder` |
| `src/main.ml` | jsoo only | Classic-script export: `Js.export "run" (Js.wrap_callback Script.run)` |

`shasum -a 256 src/{host.mli,tables.ml,huffman.ml,deflate.ml,inflate.ml,framing.ml,codec.ml,stream.ml,script.ml}`
matches `../melange/src/` identically. Shared core design notes are in `../melange/NOTES.md`; what
follows is specific to js_of_ocaml.

## Interop decisions

- **Direct `Typed_array` indexing without boxing.** `Host.U8.t`, `Host.U16.t`, and `Host.I32.t`
  wrap `Typed_array.uint8Array Js.t`, `Typed_array.uint16Array Js.t`, and
  `Typed_array.int32Array Js.t`. Because js_of_ocaml's `int` is a 32-bit signed JS number,
  `Js.Unsafe.get` / `Js.Unsafe.set` on all three typed arrays compile under `--opt=3` directly to
  `a[i]` / `a[i] = v` with zero boxing, and whole-program inlining collapses the `Host` wrapper
  calls directly into `dflt`, `inflt`, `h_map`, `wblk`, and `crc`.
- **UTF-16 JS strings stay JS strings.** `Host.Text.t` is `Js.js_string Js.t` (never converted to
  OCaml UTF-8 `string` via `Js.to_string` except for ASCII op/action names in `script.ml`).
  `charCodeAt` is typed to return `int` directly via a one-method class type (`code_units`), and
  UTF-8 uses module-level `TextEncoder` / `TextDecoder` instances.
- **In-place input reading.** `Host.Value.t` is `Js.Unsafe.top Js.t`, type-checked via
  `Js.strict_equals (Js.typeof v)` against pre-allocated JS type-name strings and `Array.isArray`,
  with JSON byte arrays converted directly via `new%js Typed_array.uint8Array_fromArray`.
- **Zero cross-call mutable state.** `0` embedded JS, `0` eval refs, `0` host JS, `0` module-state
  lint findings.

## js_of_ocaml 6.4.1 compiler findings

1. **Zero-argument JS constructors (`Js.constr`).** Typing `TextDecoder` as
   `(unit -> text_decoder Js.t) Js.constr` and invoking `new%js td_constr ()` passes OCaml's
   `unit` representation (`0`) as `new TextDecoder(0)`, which Node rejects (`The "0" encoding is not
   supported`). A zero-argument JS constructor must be typed as `text_decoder Js.t Js.constr` and
   invoked as `new%js td_constr` without `()`.
2. **`--no-runtime` attribution fails on integer multiply/divide/modulo.** As previously observed on
   `minisearch/jsoo`, `js_of_ocaml compile --no-runtime` fails with `Unresolved internal primitive:
   %int_mul` / `%int_div` / `%int_mod`. `languages/jsoo/build.mjs` already catches this diagnostic
   step and records `runtime: null` (`runtimeUnavailable`) in `meta.json`; the production artifact
   is unaffected and clean rebuilds are byte-identical.
3. **Whole-program inlining makes `Host` abstraction free.** Under `--opt=3`, the shared-core
   `Host.U8`/`U16`/`I32` getters and setters inline away, leaving `jsoo` slightly faster than
   Melange on `bench-compress` and effectively tied on `bench-decompress`.
4. No js_of_ocaml miscompilation was encountered.

## Size and quick Node timing observations

In isolated dev verification (`build/library-matrix/dev-ocaml/`):
- `dist/index.min.js`: **90,730 B** raw, **31,167 B** gzip-9, **27,472 B** brotli-11 (emit before
  facade/Terser: 96,082 B; compare the js_of_ocaml `floor` of ~67 KB, so ~23.5 KB above the runtime
  floor; JS reference is 17,623 B raw / 7,340 B gzip-9).
- Quick Node check (6 samples, triplet design, load ~5.8): `bench-compress` 1.09× JS (0.81 ms vs
  0.75 ms, A/A 0.992), `bench-decompress` 0.76× JS (0.36 ms vs 0.48 ms, A/A 1.000). Official
  matrix receipts will be recorded under `build/library-matrix/`.
