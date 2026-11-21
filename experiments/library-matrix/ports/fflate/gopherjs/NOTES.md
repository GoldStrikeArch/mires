# fflate 0.8.3: Go port (GopherJS)

A faithful port of the synchronous compression, decompression, framing, streaming, and text-codec
subset defined in `libraries/fflate/CONTRACT.md` to Go 1.21, compiled by GopherJS 1.21.0
(`gopherjs build -m --source_map=false`, the harness's production setting). It reproduces
`node_modules/fflate/esm/browser.js` bit-for-bit across all algorithms, lookup tables, Huffman
tie-breaks, error codes, and streaming chunk boundaries, using only Go and
`github.com/gopherjs/gopherjs/js` (no embedded JS, no eval refs, no host JS, no standard-library
packages beyond `runtime`, no goroutines, and no 64-bit integers). It reaches exact parity on all
83 corpus cases in Node 26.8.1, Chrome 158, and Firefox 160 for both `dist/index.min.js` and
`dist/index.js`, and passed 740/740 randomized differential cases against `reference.mjs` in Node.

| File | What it is |
| --- | --- |
| `src/tables.go` | Immutable module-load lookup tables (`fleb`, `fdeb`, `clim`, `deo`, `fl`, `revfl`, `fd`, `revfd`, `rev`, `flt`, `fdt`, `flm`, `flrm`, `fdm`, `fdrm`, `crct`) initialised once in `init()`, plus exact integer cutoffs for `defaultPlvl` |
| `src/huffman.go` | `hMap`, `hTree` (parallel primitive arrays, custom stable insertion sort, UZIP.js linear merge with strict `<`, iterative bottom-up depth pass, 15-bit/7-bit signed Kraft debt clamping and rollback), `lc` (including empty-distance-tree `[0], 1` quirk), `clen` |
| `src/deflate.go` | `wbits`, `wbits16`, `wfblk`, `wblk` (stored/fixed/dynamic cost tie-breaks, inlined bit emission), `dflt` (32 KiB sliding window, hash chains, rarest-2-byte jump, `bs = 0` start quirk, `i < s + lst` level-0 split), `dopt`, `deflateSync` |
| `src/inflate.go` | `readBits`, `readBits16`, `inflt` (zero-padded OOB reads, dynamic tree decoding, overlapping byte-by-byte RLE copy, custom dictionary back-references, non-resizing `outSize` truncation vs stored `RangeError`), `inflateSync` |
| `src/framing.go` | `crc32State`, `adler32State`, `wbytes`, `gzhl`/`gzh`/`gzs`/`gzl`, `gzipSync`, `gunzipSync`, `zlh`/`zls`, `zlibSync`, `unzlibSync`, `decompressSync`, `flateError` codes `0..6` |
| `src/stream.go` | `Deflate`, `Gzip`, `Zlib` (`deflateStream`), `Inflate`, `Gunzip` (recursive multi-member), `Unzlib`, `Decompress`, and concrete `streamSink` emitting `[u32le(len), final, ...chunk]` |
| `src/codec.go` | `strFromU8Utf16LE`: WHATWG Encoding Standard UTF-8 decoder (single leading BOM strip, maximal-subpart `U+FFFD` replacement, astral surrogate pairs) and Latin-1 decoder to UTF-16LE bytes |
| `src/boundary.go` | JS input parsing (`strToU8JS`, `readPayload`, options, stream steps), operation dispatch, `expectErrorCode` framing, non-nil `[]byte` return |
| `src/main.go` | Publishes `run` on `js.Module.exports` |

## Boundary and interop decisions

- **Zero-copy `Uint8Array` return.** `run(input *js.Object) []byte` returns a non-nil Go `[]byte`.
  GopherJS's `$externalize` returns `$sliceToNativeArray(r)` (`r.$array.subarray(r.$offset, r.$offset + r.$length)`),
  yielding a JS `Uint8Array` of the exact slice length with zero copy. Empty outputs explicitly
  return `make([]byte, 0)` (never `nil`, which GopherJS externalises as `null`).
- **UTF-8 payloads through the host `TextEncoder`.** UTF-8 `{ text }` payloads are encoded with
  `new TextEncoder().encode(textObj)`, as `fflate`'s `strToU8` does wherever the platform has a
  `TextEncoder`. GopherJS wraps the returned `Uint8Array` as a Go `[]byte` without copying
  (`.Interface().([]byte)`), and the string never passes through GopherJS's `.String()`
  (`$internalize(..., $String)`), which runs an `x += $encodeRune(I)` JS loop plus
  `$stringToBytes` on any non-ASCII string and has a known lone-surrogate defect.
- **Latin-1 payloads and GZIP `filename` strings via `charCodeAt`.** They are read with
  `textObj.Call("charCodeAt", i).Int()` (which GopherJS compiles directly to `e.charCodeAt(i)`
  with no argument-array wrapper), keeping the low byte of each code unit as `fflate` does.
- **Byte-array payloads.** `{ bytes: [...] }` is converted via
  `js.Global.Get("Uint8Array").New(bytesObj).Interface().([]byte)`, constructing a `Uint8Array`
  from the array and wrapping it zero-copy as a Go `[]byte`.
- **`strFromU8` in pure Go.** `strFromU8Utf16LE` implements the WHATWG Encoding Standard UTF-8
  state machine directly in Go into UTF-16LE bytes (stripping one leading UTF-8 BOM, encoding
  astral code points as surrogate pairs, and emitting maximal-subpart `U+FFFD` replacements),
  requiring no JS string round-trip.
- **GZIP `mtime`.** When `mtime != 0`, `Math.floor(mtime / 1000)` is computed via
  `js.Global.Get("Math").Call("floor", o.mtime/1000.0)` and converted to `uint32` (avoiding
  GopherJS's `math.Trunc` defect above 2^31 and avoiding importing Go's `math` package).
- **Module state lint (`moduleState: 1`).** `src/tables.go` line 8 (`var (`) is the sole
  package-level `var` declaration in the port. It holds only the 16 immutable lookup tables
  (`fleb`, `fdeb`, `clim`, `deo`, `fl`, `revfl`, `fd`, `revfd`, `rev`, `flt`, `fdt`, `flm`,
  `flrm`, `fdm`, `fdrm`, `crct`) initialised once in `init()` and never mutated afterwards,
  matching `fflate@0.8.3`'s module-load tables (`CONTRACT.md`, "Caching"). Zero cross-call
  mutable state or input memoisation exists.

## GopherJS 1.21 findings

Applying the MiniSearch GopherJS lessons up front kept the emitted package (`matrix/fflate/src`)
at **0 `$restore` state machines**, **0 `$indexPtr` calls**, **0 `$growSlice` calls**, and
**0 emulated 64-bit integer operations**:

1. **Zero closures / interface calls (`0` `$restore`).** `hTree` uses a custom in-place stable
   insertion sort on parallel primitive arrays (`[577]int32`) instead of `sort.SliceStable` or
   `slices.SortStableFunc`; `lc` and `inflt` avoid nested helper closures; all 7 streaming classes
   emit into a concrete `*streamSink` and dispatch via integer tags rather than `ondata` function
   values or interfaces. Errors return a concrete `*flateError` pointer rather than Go's `error`
   interface.
2. **Unsigned right shifts avoid `$min(..., 31)`.** In `inflt` and `readBits`/`readBits16`,
   shifting a signed `int` right by `uint(pos & 7)` caused GopherJS to emit
   `>> $min((7 & pos) >>> 0, 31)` on every symbol read (to match Go's signed shift-clamp rule).
   Keeping the bit-window accumulator as `uint32` avoids calling `Math.min` in the hot inflate
   loop.
3. **32-bit signed `int` and corrupt GZIP `ISIZE`.** Under GopherJS, Go `int` is signed 32-bit
   (`$makeSlice` throws `makeslice: len out of range` if `len < 0` or `len > 2147483647`). Found
   via randomized differential testing: a truncated or corrupted GZIP stream whose last 4 bytes
   have bit 31 set (`gzl >= 0x80000000`) overflows `int(gzl)` to a negative length before `inflt`
   can run and throw the expected `FlateError`. Returning `uint32` from `gzl` and leaving `out`
   `nil` when `sz > 0x7FFFFFFF` lets `inflt` allocate dynamically and match JS on both valid and
   corrupt streams.
4. **No stdlib bloat.** Using exact integer cutoffs for `defaultPlvl`, concrete errors instead of
   `fmt`/`errors`, and custom stable insertion sort instead of `sort` keeps the linked packages
   to just `runtime`, `github.com/gopherjs/gopherjs/js`, and `matrix/fflate/src`.

`gofmt` and `go vet` are clean.

## Size

From `tools/sizes.mjs` (`build/library-matrix/dev-gopherjs/receipts/sizes.json`):

- `dist/index.min.js`: **99,289 B** raw, **28,099 B** gzip-9, **24,541 B** brotli-11
  (against the GopherJS runtime floor of ~41.7 KB raw / ~13.7 KB gzip-9, and JS reference
  17,623 B raw / 7,340 B gzip-9).
- `dist/index.js`: **132,307 B** raw, **31,797 B** gzip-9, **27,393 B** brotli-11.
- Source lints: `2,134` Go code lines across `9` files, `embeddedJs: 0`, `evalRefs: 0`,
  `hostJs: 0`, `moduleState: 1` (`src/tables.go:8`, read-only lookup tables).
- `dist/meta.json` attribution (`132,141 B` `-m` emit before facade and Terser):

| Part | Bytes |
| --- | ---: |
| prelude | 41,828 |
| `matrix/fflate/src` | 80,877 |
| `runtime` | 4,766 |
| `github.com/gopherjs/gopherjs/js` | 4,437 |
| startup | 233 |

## Quick Node performance

Measured with `tools/bench.mjs --environments node --samples 6` (triplet design, bench variant,
1-minute load average ~4.6 on 10 cores; not a formal benchmark claim — read alongside A/A):

| Case | port p50 (ms) | js p50 (ms) | port/js | paired | A/A |
| --- | ---: | ---: | ---: | ---: | ---: |
| `bench-compress` | 1.96 | 0.78 | 2.52× | 2.52× | 1.020 |
| `bench-decompress` | 0.72 | 0.47 | 1.52× | 1.51× | 0.968 |

Node CPU profiling (`--cpu-prof`) shows nearly all runtime inside the core loops (`dflt`, `inflt`,
`crc32State.push`, `wblk`, `hMap`), with <1% GC overhead and ~4% in Latin-1 `charCodeAt` boundary
decoding; the remaining gap to JS is dominated by GopherJS's per-index slice/array bounds checks
(`i < 0 || i >= len`) and `$offset` arithmetic inside tight byte loops.
