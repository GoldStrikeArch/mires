# Library contract: `fflate`

`fflate` is a pure-JavaScript DEFLATE, GZIP, and Zlib compression/decompression library. Ports
implement `fflate 0.8.3`'s exact algorithms — canonical Huffman tree construction and 15-bit
length clamping (`hTree`/`hMap`), RLE code-length encoding (`lc`), LZ77 32 KiB sliding-window hash
chains with rarest-2-byte-sequence jumping (`dflt`), level→`(nice, chain)` table (`deo`), dynamic
hash-table sizing (`mem`/`plvl`), stored/fixed/dynamic block selection (`wblk`), inflate state
machine (`inflt`), CRC-32, Adler-32, GZIP/Zlib framing, sync streaming classes, and UTF-8/Latin-1
codecs — and must match `fflate@0.8.3` **bit-for-bit** on every byte of output.

| Field | Value |
| --- | --- |
| Package | `fflate@0.8.3`, entry `esm/browser.js` (root `node_modules`) |
| Reference adapter | `reference.mjs`: dispatches `input.op` onto the published ESM entry; see "Input" and "Output" |
| API subset | One-shot sync: `deflateSync`, `inflateSync`, `gzipSync`, `gunzipSync`, `zlibSync`, `unzlibSync`, `decompressSync`, `strToU8`, `strFromU8` (options: `level`, `mem`, `dictionary`, `outSize`→`out`, `mtime`, `filename`, `latin1`). Sync streaming classes: `Deflate`, `Inflate`, `Gzip`, `Gunzip`, `Zlib`, `Unzlib`, `Decompress` (`push`, `flush`, missing-handler check). Errors: `FlateErrorCode` `0..6` (reachable in sync subset; `7` and `8` explained below). Left out: async / Web Worker APIs (`deflate`, `inflate`, `gzip`, `gunzip`, `zlib`, `unzlib`, `decompress`, `Async*`), ZIP archive APIs (`zipSync`, `unzipSync`, `zip`, `unzip`, `Zip*`, `Unzip*`), `DecodeUTF8`/`EncodeUTF8` stream classes, and `Gunzip.onmember`. |
| Input | `{ op, ... }` pure JSON object; see "Input" |
| Output form | `bytes` (`Uint8Array`), synchronous |
| Normalisation | none |
| Parity environments | node, chrome, firefox, safari (unavailable on this machine) |
| Corpus | `corpus.json`, 89 cases, written deterministically by `generate-corpus.mjs` (`--verify` validates all invariants) |
| Bench cases | `bench-compress`, `bench-decompress`; `baseIterations` 1; variant `bench-variant.mjs` (k-seeded ASCII letter bijection on plaintext; re-compressed at level 6 for `bench-decompress`) |
| Caching | Immutable lookup tables computed at module load (`fleb`, `fdeb`, `clim`, `fl`, `revfl`, `fd`, `revfd`, `rev`, `flt`, `fdt`, `flm`, `flrm`, `fdm`, `fdrm`, `deo`, `crct`); no cross-call mutable state or input memoisation |
| Files | none: neither the adapter nor a port reads files at run time |

## Why this subset

This subset covers the entire synchronous compression, decompression, framing, streaming, and
text-encoding core of `fflate`: raw DEFLATE, GZIP, Zlib, format auto-detection, custom
dictionaries, pre-allocated output buffers, all 10 compression levels (`0..9`), hash-table `mem`
sizing, incremental chunked streaming with both normal and sync flushes, multi-member GZIP
streaming, and all synchronous error paths. Left out:

- **Async / Web Worker APIs** (`deflate`, `inflate`, `gzip`, `gunzip`, `zlib`, `unzlib`,
  `decompress`, `AsyncDeflate`, `AsyncInflate`, `AsyncGzip`, `AsyncGunzip`, `AsyncZlib`,
  `AsyncUnzlib`, `AsyncDecompress`): `fflate` serialises functions into `blob:` URLs for
  `new Worker(...)` (`wcln`/`wrkr`), which is forbidden by the matrix runtime guard and relies on
  JS `Function.prototype.toString()`.
- **ZIP archive APIs** (`zipSync`, `unzipSync`, `zip`, `unzip`, `Zip`, `Unzip`, `ZipDeflate`,
  `ZipPassThrough`, `UnzipInflate`, `UnzipPassThrough`, `AsyncZipDeflate`, `AsyncUnzipInflate`):
  an archive container layer on top of `deflateSync`/`inflateSync` with DOS timestamps, ZIP64, and
  object-tree traversal; kept out so this contract focuses sharply on compression & streaming.
- **`DecodeUTF8` / `EncodeUTF8` stream classes**: thin wrappers around `TextDecoder({ stream: true })`
  and `strToU8`, redundant with the one-shot `strToU8`/`strFromU8` codec coverage.

## Input

Every corpus input is a pure JSON object. Binary payloads never rely on non-`fflate` encodings
such as base64 or hex; instead every `Payload` is decoded by `fflate`'s own `strToU8` or passed as
a small JSON byte array:

```text
Payload :=
  { "text": string }                       -> strToU8(text, false)  (UTF-8)
| { "text": string, "latin1": true }       -> strToU8(text, true)   (Latin-1: charCodeAt & 0xFF)
| { "bytes": [0..255, ...] }               -> new Uint8Array(bytes)

CompressOptions (all fields optional):
  { "level"?: 0..9, "mem"?: 0..12, "dictionary"?: Payload, "mtime"?: number, "filename"?: string }

DecompressOptions (all fields optional):
  { "dictionary"?: Payload, "outSize"?: non-negative integer }   (outSize -> out: new Uint8Array(outSize))

StreamStep :=
  { "action": "push",  "data": Payload, "final"?: boolean }
| { "action": "flush", "sync"?: boolean }

Input :=
  // One-shot compression / decompression (zero output framing; bench cases use these directly)
  { "op": "deflateSync" | "gzipSync" | "zlibSync",
    "data": Payload, "options"?: CompressOptions, "expectErrorCode"?: boolean }
| { "op": "inflateSync" | "gunzipSync" | "unzlibSync" | "decompressSync",
    "data": Payload, "options"?: DecompressOptions, "expectErrorCode"?: boolean }

  // String <-> Uint8Array codec
| { "op": "strToU8",   "text": string, "latin1"?: boolean }
| { "op": "strFromU8", "data": Payload, "latin1"?: boolean, "expectErrorCode"?: boolean }

  // Sync streaming classes
| { "op": "Deflate" | "Gzip" | "Zlib",
    "options"?: CompressOptions, "noHandler"?: boolean, "steps": [StreamStep, ...],
    "expectErrorCode"?: boolean }
| { "op": "Inflate" | "Gunzip" | "Unzlib" | "Decompress",
    "options"?: DecompressOptions, "noHandler"?: boolean, "steps": [StreamStep, ...],
    "expectErrorCode"?: boolean }
```

## Output

Every call returns a `Uint8Array`:

- **One-shot byte-returning ops** (`deflateSync`, `inflateSync`, `gzipSync`, `gunzipSync`,
  `zlibSync`, `unzlibSync`, `decompressSync`, `strToU8`): return the raw `Uint8Array` directly with
  **zero framing**. Both bench cases (`bench-compress` and `bench-decompress`) use this path.
- **`strFromU8`**: returns exact **UTF-16LE bytes** of the resulting JS string — length
  `2 * str.length`, byte `2*i = str.charCodeAt(i) & 0xFF`, byte `2*i + 1 = str.charCodeAt(i) >>> 8`
  — checking every UTF-16 code unit (including astral surrogate pairs) bit-for-bit.
- **Streaming ops** (`Deflate`, `Inflate`, `Gzip`, `Gunzip`, `Zlib`, `Unzlib`, `Decompress`):
  for every `ondata(chunk, final)` callback invocation in order, emit
  `[u32le(chunk.length), final ? 1 : 0, ...chunk]`, concatenated into one `Uint8Array`. Every
  emission's exact byte boundary, length (including empty chunks), and `final` flag are verified.
- **Error handling**:
  - When `expectErrorCode` is omitted or `false`: exceptions propagate out of `run(input)` (checked
    by the harness's `both-threw` rule).
  - When `expectErrorCode === true`: a caught `FlateError` (an error carrying numeric `.code`)
    returns the 2-byte `Uint8Array([0xFF, error.code])`. If the call succeeds without throwing, the
    adapter throws an error.

## Behaviour

Everything below restates `fflate@0.8.3` (`node_modules/fflate/esm/browser.js`, which is the final
authority where this is silent). Because DEFLATE allows many valid compressed encodings of the same
input, **ports must implement `fflate@0.8.3`'s exact algorithm and tie-breaks** — substituting
another DEFLATE encoder (such as `flate2`, `miniz_oxide`, `zlib`, or `java.util.zip`) will fail
bit-exact parity immediately.

### Fixed tables

- `fleb` (32 × `u8`, length extra bits):
  `[0,0,0,0,0,0,0,0, 1,1,1,1, 2,2,2,2, 3,3,3,3, 4,4,4,4, 5,5,5,5, 0, 0, 0, 0]`
- `fdeb` (32 × `u8`, distance extra bits):
  `[0,0,0,0, 1,1, 2,2, 3,3, 4,4, 5,5, 6,6, 7,7, 8,8, 9,9, 10,10, 11,11, 12,12, 13,13, 0, 0]`
- `clim` (19 × `u8`, code-length alphabet order):
  `[16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15]`
- `freb(eb, start)`: builds `b` (31 × `u16`) with `b[0] = start + 1`, `b[i] = b[i-1] + (1 << eb[i-1])`
  for `i = 1..30`, and reverse table `r` (`i32[b[30]]`) with `r[j] = ((j - b[i]) << 5) | i` for
  `i = 1..29`, `j = b[i]..b[i+1]-1` (`r[b[0]] = 0`).
  - `fl, revfl = freb(fleb, 2)`, then override `fl[28] = 258`, `revfl[258] = 28`.
    (`fl = [3,4,5,6,7,8,9,10,11,13,15,17,19,23,27,31,35,43,51,59,67,83,99,115,131,163,195,227,258,260,261]`).
  - `fd, revfd = freb(fdeb, 0)`.
    (`fd = [1,2,3,4,5,7,9,13,17,25,33,49,65,97,129,193,257,385,513,769,1025,1537,2049,3073,4097,6145,8193,12289,16385,24577,32769]`).
- `rev` (15-bit reversal for `0..32767`): `rev[i]` is the 15-bit bit-reverse of `i`.
- Fixed Huffman lengths:
  - `flt` (288 × `u8`): `0..143 → 8`, `144..255 → 9`, `256..279 → 7`, `280..287 → 8`.
  - `fdt` (32 × `u8`): `0..31 → 5`.
  - `flm = hMap(flt, 9, 0)`, `flrm = hMap(flt, 9, 1)`, `fdm = hMap(fdt, 5, 0)`, `fdrm = hMap(fdt, 5, 1)`.
- `deo` (9 × `i32`, levels `1..9`, packed `(nice << 13) | chain`):

| Level | `deo[level - 1]` | `nice` (`>> 13`) | `chain` (`& 8191`) |
| --- | --- | --- | --- |
| 1 | `65540` | 8 | 4 |
| 2 | `131080` | 16 | 8 |
| 3 | `131088` | 16 | 16 |
| 4 | `131104` | 16 | 32 |
| 5 | `262176` | 32 | 32 |
| 6 | `1048704` | 128 | 128 |
| 7 | `1048832` | 128 | 256 |
| 8 | `2114560` | 258 | 1024 |
| 9 | `2117632` | 258 | 4096 |

- `crct` (256 × `u32`): standard reflected IEEE CRC-32 table (polynomial `0xEDB88320`, 8 rounds per byte).

### Bit I/O helpers

- `shft(p) = (p + 7) >> 3` (byte count rounded up).
- `bits(d, p, m)`: `o = p >> 3`, returns `((d[o] | (d[o + 1] << 8)) >> (p & 7)) & m`, treating any
  out-of-bounds read `d[k]` (`k >= d.length`) as `0`.
- `bits16(d, p)`: `o = p >> 3`, returns `(d[o] | (d[o + 1] << 8) | (d[o + 2] << 16)) >> (p & 7)`,
  treating out-of-bounds reads as `0`.
- `wbits(d, p, v)`: `v <<= p & 7; o = p >> 3; d[o] |= v & 0xFF; d[o + 1] |= (v >> 8) & 0xFF`.
- `wbits16(d, p, v)`: same, also `d[o + 2] |= (v >> 16) & 0xFF`.
- `wbytes(d, b, v)`: writes unsigned 32-bit `v` little-endian while `v != 0` (`d[b++] = v & 0xFF; v >>>= 8`).
  (Because output buffers are zero-initialised, leaving higher zero bytes untouched equals writing all 4 bytes LE.)
- `slc(v, s, e)`: clamps `s == null || s < 0` to `0` and `e == null || e > v.length` to `v.length`,
  returning a fresh copy `new Uint8Array(v.subarray(s, e))` (empty when `s >= e`). In streaming
  `Inflate`, `slc(dt, st.b - 32768)` relies on clamping negative `st.b - 32768` to `0`.

### Canonical Huffman maps: `hMap(cd, mb, r)`

Given code-length array `cd` of length `s`, max bit length `mb` (`0..15`), and mode `r` (`0` = forward
encoding table, `1` = reverse decoding table):

1. Count non-zero lengths: `l = u16[mb]`; for `i = 0..s-1`, if `cd[i] != 0`, `l[cd[i] - 1] += 1`.
2. Starting codes: `le = u16[mb]`; `le[0] = 0`; for `i = 1..mb-1`, `le[i] = (le[i - 1] + l[i - 1]) << 1` (mod 65536).
3. If `r == 1` (decode): allocate `co = u16[1 << mb]` (zeroed; length 1 when `mb == 0`). Let `rvb = 15 - mb`.
   For `i = 0..s-1` where `cd[i] != 0`:
   - `sv = (i << 4) | cd[i]`, `r1 = mb - cd[i]`, `v = (le[cd[i] - 1]++) << r1`, `m = v | ((1 << r1) - 1)`.
   - For `x = v..=m`: `co[rev[x] >> rvb] = sv` (if `x >= 32768` in a corrupt oversubscribed tree, index `0`).
4. If `r == 0` (encode): allocate `co = u16[s]`. For `i = 0..s-1` where `cd[i] != 0`:
   - `co[i] = rev[le[cd[i] - 1]++] >> (15 - cd[i])`.

### Huffman tree construction & length clamping: `hTree(d, mb)`

Exact tie-breaks here determine compressed output bytes bit-for-bit:

1. Collect non-zero entries in ascending symbol order: `t = [{ s: i, f: d[i] } for i in 0..d.length if d[i] != 0]`.
   Let `s = t.length`, and snapshot `t2 = t.slice()` (preserving ascending `s` order).
2. If `s == 0`: return `{ t: empty_u8, l: 0 }`.
3. If `s == 1`: return `{ t: u8[t[0].s + 1] with t[t[0].s] = 1, l: 1 }`.
4. **Stable sort** `t` by ascending `f` (ties keep ascending `s`, i.e., total order `(f, s)`).
5. Push sentinel `{ s: -1, f: 25001 }` at index `s`.
6. Two-queue linear merge (UZIP.js algorithm):
   - `l = t[0], r = t[1], i0 = 0, i1 = 1, i2 = 2`.
   - `t[0] = { s: -1, f: l.f + r.f, l, r }`.
   - While `i1 != s - 1`:
     - `l = t[t[i0].f < t[i2].f ? i0++ : i2++]` (**strict `<`**: equal frequency picks the leaf `t[i2]`, not the internal node `t[i0]`!).
     - `r = t[i0 != i1 && t[i0].f < t[i2].f ? i0++ : i2++]`.
     - `t[i1++] = { s: -1, f: l.f + r.f, l, r }`.
7. Compute depths `tr = u16[maxSym + 1]` (where `maxSym = t2[s - 1].s`) by walking `t[i1 - 1]` from
   depth `0`; `mbt` is the maximum leaf depth.
8. **Length clamping** when `mbt > mb`:
   - **Stable sort** `t2` by `(tr[b.s] - tr[a.s]) || (a.f - b.f)` — descending depth, then ascending
     frequency, then ascending symbol index `s`.
   - `lft = mbt - mb`, `cst = 1 << lft`, `i = 0`, `dt = 0` (signed 32-bit integer).
   - Clamp overly deep leaves: while `i < s && tr[t2[i].s] > mb`:
     `dt += cst - (1 << (mbt - tr[t2[i].s])); tr[t2[i].s] = mb; i++`.
   - `dt >>= lft`.
   - Pay off Kraft debt: while `dt > 0`:
     `sym = t2[i].s`; if `tr[sym] < mb`, `dt -= 1 << (mb - tr[sym] - 1); tr[sym]++` else `i++`.
   - Negative-debt rollback (note signed `i >= 0` starting at the final `i` from the previous loop):
     while `i >= 0 && dt != 0`: `sym = t2[i].s`; if `tr[sym] == mb`, `tr[sym]--; dt++`; `i--`.
   - `mbt = mb`.
9. Return `{ t: u8(tr), l: mbt }`.

### Code-length RLE encoding: `lc(c)`

1. Trim trailing zeros: `s = c.length; while (s && !c[--s]); s++`.
   **Important empty-input quirk**: when `c.length == 0` (empty distance tree `ddt` when a block has
   zero back-references), `s` becomes `1`, `cln = 0`, and `lc` returns `{ c: [0], n: 1 }`.
2. Scan runs of identical code lengths `cln` of length `cls` over `0..s-1`:
   - If `cln == 0 && cls > 2`:
     - While `cls > 138`: emit `32754` (`(127 << 5) | (7 << 12) | 18`), `cls -= 138`.
     - If `cls > 2`: emit `cls > 10 ? ((cls - 11) << 5) | 28690 : ((cls - 3) << 5) | 12305`, `cls = 0`.
   - Else if `cls > 3`:
     - Emit `cln`, `cls--`.
     - While `cls > 6`: emit `8304` (`(3 << 5) | (2 << 12) | 16`), `cls -= 6`.
     - If `cls > 2`: emit `((cls - 3) << 5) | 8208`, `cls = 0`.
   - Emit `cln` `cls` times.
3. Return `{ c: cl[0..cli], n: s }`.

### Block selection & emission: `wblk(dat, out, final, syms, lf, df, eb, li, bs, bl, p)`

1. Write 1-bit `final` at `p++`; increment `lf[256]`.
2. Build dynamic trees: `(dlt, mlb) = hTree(lf, 15)`, `(ddt, mdb) = hTree(df, 15)`,
   `(lclt, nlc) = lc(dlt)`, `(lcdt, ndc) = lc(ddt)`.
3. Count code-length symbol frequencies `lcfreq[0..19]` from `v & 31` in `lclt` and `lcdt`;
   `(lct, mlcb) = hTree(lcfreq, 7)`.
4. `nlcc = 19`; while `nlcc > 4 && (lct[clim[nlcc - 1]] || 0) == 0`, `nlcc--`.
5. Bit costs (using `clen(freq, lengths) = sum(freq[i] * lengths[i])`):
   - `flen = (bl + 5) << 3` (stored block)
   - `ftlen = clen(lf, flt) + clen(df, fdt) + eb` (fixed Huffman)
   - `dtlen = clen(lf, dlt) + clen(df, ddt) + eb + 14 + 3 * nlcc + clen(lcfreq, lct) + 2 * lcfreq[16] + 3 * lcfreq[17] + 7 * lcfreq[18]` (dynamic Huffman)
6. **Tie-breaking**:
   - If `bs >= 0 && flen <= ftlen && flen <= dtlen`: write stored block `wfblk(out, p, dat[bs..bs+bl])` and return new bit position.
     (`wfblk` sets `o = shft(p + 2)`, writes `LEN` (u16le), `LEN ^ 0xFFFF` (u16le), raw bytes, returns `(o + 4 + bl) * 8`.)
   - Else if `dtlen < ftlen`: write 2-bit BTYPE `2`, 14-bit header (`nlc - 257`, `ndc - 1`, `nlcc - 4`),
     `nlcc` × 3-bit code lengths `lct[clim[i]] || 0` in `clim` order (note: `lct.length = maxSym + 1 <= 19`,
     so bounds-checked languages must default out-of-bounds `clim[i] >= lct.length` to `0` here and in
     the `nlcc` trimming loop), encoded `lclt` then `lcdt` (for `len > 15`, write
     `(entry >> 5) & 127` in `entry >> 12` bits), then encode `syms[0..li]` + EOB (`256`) with `dlt`/`ddt`.
   - Else (`dtlen >= ftlen` — ties choose fixed!): write 2-bit BTYPE `1`, then encode `syms[0..li]` + EOB with `flt`/`fdt`.

### LZ77 compressor: `dopt` and `dflt`

- **`dopt(dat, opt, pre, post, st)`**:
  - When `st` is absent (one-shot): `st = { l: 1 }`; if `opt.dictionary` is present, let
    `dict = opt.dictionary.subarray(-32768)`, prepend `dict` to `dat`, and set `st.w = dict.length`.
  - `lvl = opt.level == null ? 6 : opt.level`.
  - `plvl`:
    - If `opt.mem != null`: `12 + opt.mem`.
    - Else if `st.l == 0` (non-final streaming chunk): `20`.
    - Else (`st.l == 1`): `Math.ceil(Math.max(8, Math.min(13, Math.log(dat.length))) * 1.5)`.
      This is an integer in `12..20` with exact integer cutoffs on `dat.length` (note: in streaming
      `Deflate`, `dat` is the full sliding buffer `this.b`, typically 98,304 bytes → `plvl = 18`,
      NOT `st.z`!):
      `≤ 2980 → 12`, `≤ 5806 → 13`, `≤ 11308 → 14`, `≤ 22026 → 15`, `≤ 42901 → 16`,
      `≤ 83561 → 17`, `≤ 162754 → 18`, `≤ 317003 → 19`, `≥ 317004 → 20`.
- **`dflt(dat, lvl, plvl, pre, post, st)`**:
  - `s = st.z || dat.length`, `lst = st.l` (`0` or `1`), `pos = (st.r || 0) & 7`.
  - Output allocation: `pre + s + 5 * (1 + Math.ceil(s / 7000)) + post`, writing into `w = o[pre .. o.length - post]`.
    (**Bounds-checked languages**: `wbits16` writes `w[o + 1]` and `w[o + 2]`, which JS `Uint8Array`
    silently drops if `o + 2 >= w.length` on the final EOB; allocate `+2` extra zero slack bytes inside
    `w` before slicing to `pre + shft(pos) + post` so `wbits16` never panics.)
  - **If `lvl == 0`**:
    - Loop `for (i = st.w || 0; i < s + lst; i += 65535)`:
      `e = i + 65535`; if `e >= s`, `w[pos >> 3] = lst; e = s`; `pos = wfblk(w, pos + 1, dat[i..e])`.
    - `st.i = s`.
    - **Quirk**: because the loop condition is `i < s + lst`, when `s - (st.w || 0)` is a positive
      multiple of `65535` (such as `s == 65535` with `st.w == 0`) and `lst == 1`, a second trailing
      empty stored block (`01 00 00 FF FF`) is emitted. (`deflate-level0-65535` tests this!)
  - **If `lvl != 0`**:
    - If `pos != 0`, `w[0] = st.r >> 3`.
    - `(n, c) = (deo[lvl - 1] >> 13, deo[lvl - 1] & 8191)`, `msk = (1 << plvl) - 1`,
      `bs1 = Math.ceil(plvl / 3)`, `bs2 = 2 * bs1`.
    - `prev = st.p || u16[32768]`, `head = st.h || u16[msk + 1]`.
    - `hsh(idx) = (dat[idx] ^ (dat[idx + 1] << bs1) ^ (dat[idx + 2] << bs2)) & msk`.
    - `syms = i32[25000]`, `lf = u16[288]`, `df = u16[32]`, `lc = 0, eb = 0, i = st.i || 0, li = 0, wi = st.w || 0, bs = 0`.
      (**Quirk**: `bs` always starts at `0`, even when `st.w > 0` from a dictionary or streaming lookback.
      Keeping `bs = 0` is required so `flen` in `wblk` matches `fflate` on the first block!)
    - For `i; i + 2 < s; ++i`:
      - `hv = hsh(i); imod = i & 32767; pimod = head[hv]; prev[imod] = pimod; head[hv] = imod`.
      - If `wi <= i`:
        - `rem = s - i`.
        - If `(lc > 7000 || li > 24576) && (rem > 423 || !lst)`:
          flush block via `wblk(dat, w, 0, syms, lf, df, eb, li, bs, i - bs, pos)`, reset `li = lc = eb = 0`,
          `bs = i`, clear `lf[0..286]` and `df[0..30]`.
        - Search hash chain: `l = 2, d = 0, ch = c, dif = (imod - pimod) & 32767`.
          If `rem > 2 && hv == hsh(i - dif)` (line 671; there is no `i >= dif` test: when `dif > i`,
          `hsh(i - dif)` reads `dat` at negative indices, `undefined`, which the XOR turns into `0`,
          and the chain loop below cannot run because `maxd = min(32767, i) < dif`, so a
          bounds-checked port may test `i >= dif` first with the same result):
          `maxn = min(n, rem) - 1`, `maxd = min(32767, i)`, `ml = min(258, rem)`.
          While `dif <= maxd && --ch != 0 && imod != pimod`:
          - If `dat[i + l] == dat[i + l - dif]`:
            count `nl` up to `ml` where `dat[i + nl] == dat[i + nl - dif]`.
            If `nl > l`:
            `l = nl; d = dif;`
            if `nl > maxn`, break;
            rarest-2-byte jump: `mmd = min(dif, nl - 2); md = 0`;
            for `j = 0..mmd-1`: `ti = (i - dif + j) & 32767; pti = prev[ti]; cd = (ti - pti) & 32767; if (cd > md) { md = cd; pimod = ti; }`.
          - Advance: `imod = pimod; pimod = prev[imod]; dif += (imod - pimod) & 32767`.
        - If `d != 0`:
          `syms[li++] = 268435456 | (revfl[l] << 18) | revfd[d]`;
          `lin = revfl[l] & 31; din = revfd[d] & 31`;
          `eb += fleb[lin] + fdeb[din]`; `lf[257 + lin]++`; `df[din]++`; `wi = i + l`; `lc++`.
        - Else: `syms[li++] = dat[i]; lf[dat[i]]++`.
    - Tail literals: `for (i = max(i, wi); i < s; ++i) { syms[li++] = dat[i]; lf[dat[i]]++; }`.
    - Emit final block of this call: `pos = wblk(dat, w, lst, syms, lf, df, eb, li, bs, i - bs, pos)`.
    - If `!lst`: `st.r = (pos & 7) | (w[pos >> 3] << 3); pos -= 7; st.h = head; st.p = prev; st.i = i; st.w = wi`.
  - Return slice `o[0 .. pre + shft(pos) + post]`.

### Inflate state machine: `inflt(dat, st, buf, dict)`

- `sl = dat.length`, `dl = dict ? dict.length : 0`.
- Early return: if `sl == 0 || (st.f != 0 && !st.l)`, return `buf || new u8(0)` (note: when `sl == 0`
  and `buf` was provided via `outSize`, the full zero-filled `buf` is returned!).
- `noBuf = !buf`, `resize = noBuf || st.i != 2`, `noSt = st.i` (`0` = non-final stream chunk, `1` = final stream chunk, `2` = one-shot sync).
- When `noBuf`, allocate `buf = new u8(sl * 3)`. Helper `cbuf(l)` grows `buf` to `max(buf.length * 2, l)` when `l > buf.length`
  (new bytes zero). `inflt` calls it only in resize mode, and only at the three places marked below.
- `tbts = sl * 8`. Loop `do { ... } while (!final)`:
  - If `!lm`:
    - `final = bits(dat, pos, 1)`, `type = bits(dat, pos + 1, 3)`, `pos += 3`.
    - `type == 0` (stored): `s = shft(pos) + 4`, `l = dat[s - 4] | (dat[s - 3] << 8)` (OOB reads are 0; NLEN is NOT checked), `t = s + l`.
      If `t > sl`: if `noSt != 0` throw `err(0)` else break.
      If `resize`, `cbuf(bt + l)` (line 261); else if `bt + l > buf.length` throw `RangeError` (matching JS `TypedArray.prototype.set`).
      Copy `dat[s..t]` into `buf[bt..bt+l]`; `st.b = bt += l; st.p = pos = t * 8; st.f = final; continue`.
    - `type == 1` (fixed): `lm = flrm, dm = fdrm, lbt = 9, dbt = 5`.
    - `type == 2` (dynamic): read `hLit = bits(dat, pos, 31) + 257`, `hDist = bits(dat, pos + 5, 31) + 1`,
      `hcLen = bits(dat, pos + 10, 15) + 4`, `pos += 14`. Decode `clt[clim[i]]` (`i < hcLen`, 3 bits each),
      build `clm = hMap(clt, max(clt), 1)`, decode `tl = hLit + hDist` lengths into `ldt` (codes 16/17/18;
      writes past `tl` ignored), split `lt = ldt[0..hLit]`, `dt = ldt[hLit..tl]`, build `lm = hMap(lt, max(lt), 1)`
      and `dm = hMap(dt, max(dt), 1)`.
    - `type == 3`: throw `err(1)` (`InvalidBlockType`).
    - If `pos > tbts`: if `noSt != 0` throw `err(0)` else break (leaving `st.l/p/b/f` untouched so the next chunk re-reads the block header).
  - Decode symbols: if `resize`, `cbuf(bt + 131072)` (line 331). `lms = (1 << lbt) - 1`, `dms = (1 << dbt) - 1`, `lpos = pos`.
    Loop (`lpos = pos` each iteration):
    - `c = lm[bits16(dat, pos) & lms]`, `sym = c >> 4`, `pos += c & 15`.
    - If `pos > tbts`: if `noSt != 0` throw `err(0)` else break.
    - If `c == 0`: throw `err(2)` (`InvalidLengthLiteral`).
    - If `sym < 256`: `buf[bt++] = sym` (line 346). `buf` is a `Uint8Array`, so a store at
      `bt >= buf.length` is dropped while `bt` still advances, **in both modes**: the buffer grows
      only through `cbuf`, which runs for a stored block (line 261), at the start of a Huffman block
      or of a block resumed from `st` (line 331), and before each back-reference (line 376), never
      for a literal. More than 131,072 literals after the last `cbuf` therefore lose their tail:
      `inflateSync` of one 200,000-literal block returns `max(2 * sl * 3, 131072)` bytes (150,084
      for `inflate-literal-run-200k`), and a stream emits at most the 131,072-byte buffer
      (`stream-inflate-literal-run-200k`). When a later `cbuf` grows the buffer past `bt`, the
      dropped positions read back as the new buffer's zeros (`inflate-literal-run-200k-backref`).
      `st.b` keeps the advanced `bt`, so `Inflate.c` clamps its `slc` calls to the returned buffer.
    - Else if `sym == 256`: `lpos = pos; lm = null; break`.
    - Else (`sym > 256`):
      - `add = sym - 254`; if `sym > 264`: `i = sym - 257; b = fleb[i]; add = bits(dat, pos, (1 << b) - 1) + fl[i]; pos += b`.
      - `d = dm[bits16(dat, pos) & dms]`, `dsym = d >> 4`; if `d == 0`, throw `err(3)` (`InvalidDistance`); `pos += d & 15`.
      - `dt = fd[dsym]`; if `dsym > 3`: `b = fdeb[dsym]; dt += bits16(dat, pos) & ((1 << b) - 1); pos += b`.
        (Note for bounds-checked languages on corrupt streams: `fd` has length `31`, whereas `fdt`/dynamic
        distance trees can emit `dsym == 31`, making `fd[31]` `undefined` → `dt = NaN`, so `bt < dt` is
        false and the copy loop fills `buf[bt..end]` with `0`.)
      - If `pos > tbts`: if `noSt != 0` throw `err(0)` else break.
      - If `resize`, `cbuf(bt + 131072)` (line 376).
      - `end = bt + add`.
      - If `bt < dt`: `shift = dl - dt` (signed), `dend = min(dt, end)`; if `shift + bt < 0`, throw `err(3)`;
        copy `dict[shift + bt]` into `buf[bt]` for `bt < dend` (ignoring OOB writes when `!resize`).
      - Byte-by-byte copy (for overlapping RLE `dt < add`): for `bt < end`, write `buf[bt] = buf[bt - dt]` (reading `0` if `bt - dt >= buf.length`, ignoring writes if `bt >= buf.length`).
  - Save state: `st.l = lm; st.p = lpos; st.b = bt; st.f = final`; if `lm != null`, set `final = 1; st.m = lbt; st.d = dm; st.n = dbt`.
- Return `buf[0 .. min(bt, buf.length)]`.

### Checksums and framing

- **CRC-32** (`crc`): initialised to `0xFFFFFFFF`; per byte `c = crct[(c & 0xFF) ^ b] ^ (c >>> 8)`;
  digest is `(~c) >>> 0`, written little-endian.
- **Adler-32** (`adler`): `a = 1, b = 0`; in blocks of at most 2655 bytes, `a += d[i]; b += a`, then
  `a = (a & 0xFFFF) + 15 * (a >> 16)`, `b = (b & 0xFFFF) + 15 * (b >> 16)`; at digest, `a %= 65521`,
  `b %= 65521`, written big-endian `[b >> 8, b & 0xFF, a >> 8, a & 0xFF]`.
- **GZIP**:
  - `gzhl(o) = 10 + (o.filename ? o.filename.length + 1 : 0)` (note: `""` is falsy → length 10, no FNAME flag!).
  - `gzh(c, o)`: `c[0..3] = [31, 139, 8]`; `c[8] = (o.level != null && o.level < 2) ? 4 : o.level == 9 ? 2 : 0`; `c[9] = 3`;
    if `o.mtime != 0`, write `Math.floor(o.mtime / 1000)` LE at `c[4..8]` (omitted `mtime` uses `Date.now()` in JS; all deterministic corpus cases pass `mtime` explicitly);
    if `o.filename` is non-empty, `c[3] = 8`, write `filename.charCodeAt(i) & 0xFF` at `c[10 + i]` followed by `0`.
  - `gzs(d)`: requires `d[0] == 31 && d[1] == 139 && d[2] == 8`, else `err(6)`. Reads `flg = d[3]`, `st = 10`.
    If `flg & 4` (FEXTRA), `st += (d[10] | (d[11] << 8)) + 2`. For `zs = ((flg >> 3) & 1) + ((flg >> 4) & 1)`
    (FNAME + FCOMMENT), scan NUL-terminated strings (`while (zs > 0) zs -= !d[st++]`, OOB treated as `0`).
    Returns `st + (flg & 2)` (FHCRC).
  - `gunzipSync(data, opts)`: `st = gzs(data)`; if `st + 8 > data.length`, `err(6)`;
    `inflt(data.subarray(st, -8), { i: 2 }, opts?.out || new u8(gzl(data)), opts?.dictionary)`.
    Note: CRC-32 and ISIZE are NOT validated on decompression (`gzl` only sizes the default output buffer).
- **Zlib**:
  - `zlh(c, o)`: `fl = o.level === 0 ? 0 : (o.level != null && o.level < 6) ? 1 : o.level === 9 ? 3 : 2`;
    `c[0] = 120`; `c[1] = (fl << 6) | (o.dictionary ? 32 : 0)`;
    `c[1] |= 31 - ((c[0] << 8) | c[1]) % 31` (note: for `level: 0` + dictionary, `30752 % 31 == 0`, so `31 - 0 = 31` → `c[1] = 0x3F`!).
    If `o.dictionary`, write Adler-32 of the **full** `o.dictionary` (not truncated to 32 KiB!) big-endian at `c[2..6]`.
  - `zls(d, hasDict)`: if `(d[0] & 15) != 8 || (d[0] >> 4) > 7 || ((d[0] << 8) | d[1]) % 31 != 0`, `err(6)`.
    If `((d[1] >> 5) & 1) == (hasDict ? 0 : 1)`, `err(6)` (need/unexpected dictionary).
    Returns `((d[1] >> 3) & 4) + 2` (6 if FDICT else 2).
  - `unzlibSync(data, opts)`: `inflt(data.subarray(zls(data, !!opts?.dictionary), -4), { i: 2 }, opts?.out, opts?.dictionary)`.
    Note: DICTID and trailing Adler-32 are NOT verified.
- **`decompressSync(data, opts)`**:
  - If `data[0] == 31 && data[1] == 139 && data[2] == 8`: `gunzipSync(data, opts)`.
  - Else if `(data[0] & 15) != 8 || (data[0] >> 4) > 7 || ((data[0] << 8) | data[1]) % 31 != 0`: `inflateSync(data, opts)`.
  - Else: `unzlibSync(data, opts)`.

### String codec: `strToU8` and `strFromU8`

- `strToU8(str, latin1)`:
  - If `latin1`: `out[i] = str.charCodeAt(i) & 0xFF` for each UTF-16 code unit.
  - Else: UTF-8 encoding (`TextEncoder.prototype.encode(str)`).
- `strFromU8(dat, latin1)`:
  - If `latin1`: each byte `b` becomes UTF-16 code unit `b` (`0x0000..0x00FF`).
  - Else: WHATWG `TextDecoder` semantics (`new TextDecoder()`, i.e. `fatal: false`, `ignoreBOM: false`):
    1. Strip a single leading UTF-8 BOM (`0xEF, 0xBB, 0xBF`) if present at index 0; later BOMs decode to `U+FEFF`.
    2. Decode valid UTF-8 to UTF-16 (4-byte astral sequences become surrogate pairs).
    3. Replace each maximal subpart of an ill-formed byte sequence with `U+FFFD` (matching Unicode Standard
       "Best Practices for Using U+FFFD", WHATWG Encoding Standard, and Rust's `String::from_utf8_lossy`).
    Because Node, Chrome, Firefox, and Safari all provide `TextDecoder`, `strFromU8` never throws
    `FlateErrorCode.InvalidUTF8 (8)` in these environments.

### Streaming classes

- **`Deflate`**:
  - Init: `s = { l: 0, i: 32768, w: 32768, z: 32768 }`, `b = u8[98304]`. If `opts.dictionary`, copy
    `dict = opts.dictionary.subarray(-32768)` to `b[32768 - dict.length .. 32768]`, set `s.i = 32768 - dict.length`.
  - `push(chunk, final)`: if `!ondata` → `err(5)`; if `s.l` → `err(4)`.
    `endLen = chunk.length + s.z`.
    If `endLen > b.length`:
    - If `endLen > 2 * b.length - 32768`: grow `b` to `endLen & ~32767`, copying `b[0..s.z]`.
    - `split = b.length - s.z`, copy `chunk[0..split]` into `b[s.z..]`, `s.z = b.length`, call `p(b, false)`.
    - Slide last 32768 bytes of `b` to `b[0..32768]`, copy `chunk[split..]` at `b[32768..]`,
      `s.z = chunk.length - split + 32768`, `s.i = 32766`, `s.w = 32768`.
    Else: copy `chunk` at `b[s.z..]`, `s.z += chunk.length`.
    Set `s.l = final ? 1 : 0`.
    If `s.z > s.w + 8191 || final`: call `p(b, Boolean(final))`; `s.w = s.i; s.i -= 2`.
    If `final`: reset `s = {}`, `o = {}`, `b = empty` (note: pushing again after `final` on `Deflate`/`Gzip`/`Zlib`
    does not hit `s.l` because `s` was cleared — see Parity domain).
  - `flush(sync)`: if `!ondata` → `err(5)`; if `s.l` → `err(4)`.
    Call `p(b, false)`; `s.w = s.i; s.i -= 2`.
    If `sync`: `c = u8[6]; c[0] = s.r >> 3; ep = wfblk(c, s.r, et); s.r = 0; ondata(c.subarray(0, ep >> 3), false)`.
- **`Gzip`**: wraps `Deflate` with `crc()`, `l = 0`, `v = 1`. `push` updates `crc` and `l` before
  `Deflate.push`. `p(c, f)` calls `dopt(c, o, v ? gzhl(o) : 0, f ? 8 : 0, s)`, writes `gzh` when `v`
  (then `v = 0`), writes CRC-32 and `l` at end when `f`, emits `ondata(raw, f)`.
- **`Zlib`**: wraps `Deflate` with `adler()`, `v = 1`. `push` updates `adler` before `Deflate.push`.
  `p(c, f)` calls `dopt(c, o, v ? (o.dictionary ? 6 : 2) : 0, f ? 4 : 0, s)`, writes `zlh` when `v`
  (then `v = 0`), writes Adler-32 when `f`, emits `ondata(raw, f)`.
- **`Inflate`**:
  - Init: `dict = opts?.dictionary?.subarray(-32768)`, `s = { i: 0, b: dict ? dict.length : 0 }`,
    `o = u8[32768]` (with `dict` copied at `0..dict.length`), `p = empty`.
  - `e(chunk)`: if `!ondata` → `err(5)`; if `d` → `err(4)`; append `chunk` to `p`.
  - `c(final)`: `d = Boolean(final); s.i = d ? 1 : 0; bts = s.b; dt = inflt(p, s, o);`
    `ondata(slc(dt, bts, s.b), d);`
    `o = slc(dt, s.b - 32768); s.b = o.length;`
    `p = slc(p, s.p >> 3); s.p &= 7`.
  - **Empty push before any output.** `e` keeps `p` empty for an empty chunk (line 1170), so `c`
    calls `inflt` with no input, whose early return hands back `o` itself: the whole 32,768-byte
    initial buffer. `c` emits an empty chunk, then `o = slc(dt, s.b - 32768)` keeps all 32,768 bytes
    and sets `s.b = 32768`. Decoding then starts 32,768 bytes into a zero window: a back-reference
    before the real output reads zeros instead of throwing `InvalidDistance (3)`, and a dictionary
    copied into `o` now sits 32,768 bytes back instead of right before the output, so references
    into it read the zero padding. Once `s.b == o.length` (after any non-empty push) an empty push
    changes nothing. `Gunzip`, `Unzlib` and `Decompress` reach the same path when their header
    parse leaves `p` empty. All ports reproduce it; `stream-inflate-1byte-chunks` starts with an
    empty push, but no corpus stream refers back before its output after one.
- **`Gunzip`**: wraps `Inflate` with `v = 1, r = 0`.
  - `push(chunk, final)`: `Inflate.e(chunk); r += chunk.length`.
    If `v != 0`: `p = this.p.subarray(v - 1); s = p.length > 3 ? gzs(p) : 4`.
    If `s > p.length`: if `!final` return.
    Set `this.p = p.subarray(s); v = 0`.
    Call `Inflate.c(false)`!
    If `this.s.f && !this.s.l` (member finished — enables concatenated multi-member GZIP):
    `v = shft(this.s.p) + 9; this.s = { i: 0 }; this.o = empty; this.push(empty, final)` (recursive,
    line 1345: one nested call per member, so the stack bounds the member count; see Parity domain).
    Else if `final`: call `Inflate.c(true)`.
    **Note**: even for a single-member GZIP pushed with `final = true`, `Gunzip` emits the member data
    with `final = false`, then recurses and emits `(empty, false)` and `(empty, true)`. Ports must
    reproduce this exact emission sequence.
- **`Unzlib`**: wraps `Inflate` with `v = opts?.dictionary ? 2 : 1`.
  - `push(chunk, final)`: `Inflate.e(chunk)`.
    If `v != 0`: if `p.length < 6 && !final` return; `p = p.subarray(zls(p, v - 1)); v = 0`.
    If `final`: if `p.length < 4` → `err(6)`; `p = p.subarray(0, -4)`.
    Call `Inflate.c(final)`.
- **`Decompress`**:
  - `push(chunk, final)`: if `!ondata` → `err(5)`.
    If no inner stream `s` yet: accumulate `chunk` into `p` (see Parity domain for `fflate 0.8.3`'s
    upstream bug on 1–2 byte first chunks); once `p.length > 2`, select `Gunzip` / `Inflate` / `Unzlib`
    using the same header test as `decompressSync`, wire `ondata`, call `s.push(p, final)`, clear `p`.
    Else delegate `s.push(chunk, final)`.

### Error codes (`FlateErrorCode`)

| Code | Name | Reachable in sync subset? | Trigger |
| --- | --- | --- | --- |
| `0` | `UnexpectedEOF` | Yes | `inflt`: truncated stored block (`t > sl`) or truncated Huffman bits (`pos > tbts`) with `noSt != 0` |
| `1` | `InvalidBlockType` | Yes | `inflt`: reserved DEFLATE block type `3` (`0b11`) |
| `2` | `InvalidLengthLiteral` | Yes | `inflt`: unmapped dynamic lit/len Huffman code (`lm[...] == 0`) |
| `3` | `InvalidDistance` | Yes | `inflt`: unmapped dynamic distance code (`dm[...] == 0`) or back-reference past `bt + dict.length` |
| `4` | `StreamFinished` | Yes | `Inflate` / `Gunzip` / `Unzlib` / `Decompress`: `push` after `final = true` |
| `5` | `NoStreamHandler` | Yes | Any stream class: `push` or `flush` when `ondata` is unset (`noHandler: true`) |
| `6` | `InvalidHeader` | Yes | `gzs`/`gunzipSync`: bad magic or `st + 8 > len`; `zls`/`Unzlib`: bad CMF/FLG/FCHECK, missing/unexpected dict, or `< 4` trailer bytes on final |
| `7` | `NoCallback` | No (async APIs excluded) | — |
| `8` | `InvalidUTF8` | No (`TextDecoder` exists in all 4 environments; `DecodeUTF8` excluded) | `strFromU8` uses WHATWG U+FFFD replacement instead |

## Parity domain

The following inputs are excluded from the contract (no corpus case or bench variant produces them):

- **Lone UTF-16 surrogates in JSON text strings**: all corpus `text` strings are well-formed UTF-16
  (required so UTF-8-native languages such as Rust, Go/GopherJS, and Elm decode identical bytes).
  Ill-formed UTF-8 byte sequences are tested via `strFromU8` with `{ "bytes": [...] }`.
- **Omitted `mtime` on `gzipSync`/`Gzip`**: `fflate` calls `Date.now()`, which is wall-clock
  non-deterministic; every `gzipSync`/`Gzip` case passes an explicit non-negative integer `mtime`
  with `0 <= Math.floor(mtime / 1000) <= 0xFFFFFFFF`.
- **`filename` with NUL bytes**: `gzipSync`/`Gzip` filenames are Latin-1/ASCII strings without `\u0000`.
- **More than 1,000 GZIP members in one `Gunzip` input** (also `Decompress` of GZIP). `Gunzip.push`
  recurses once per member (line 1345), so the member count one push can hold is a stack-depth
  limit that depends on the engine, its stack size and the caller's depth, not part of fflate's
  behaviour: npm itself overflows (`RangeError: Maximum call stack size exceeded`) between 6,500
  and 7,000 members in Node 26. Ports reproduce the recursion (Guidance 5), but their frames
  differ: the 2026-11-20 review saw Node overflows from about 2,150 members for Rust (mires frames
  are large), 4,000 for GopherJS and 6,000 for Dart, while Scala.js iterates and succeeds where npm
  throws. The domain stops at 1,000 members per input, below every port's and npm's limit with
  room for smaller browser stacks; `stream-gunzip-1000-members` checks the bound. Ports are not
  required to fail beyond it.
- **Upstream `fflate@0.8.3` quirks outside the parity domain**:
  1. `Decompress.prototype.push` (lines 1564–1567) forgets `this.p = n` when concatenating after a
     1- or 2-byte non-empty first chunk, dropping subsequent chunks. All `Decompress` cases supply a
     first non-empty chunk of at least 3 bytes (empty leading chunks `[]` are allowed and tested).
  2. `Deflate.prototype.flush(true)` (line 1107) calls `wfblk(c, this.s.r, et)` without masking
     `this.s.r & 7`, corrupting sync-flushed streams whenever the partial trailing byte is non-zero
     (`this.s.r >= 8`). Additionally, at `level: 0`, `dflt` never initializes `this.s.r` (leaving it
     `undefined`), so `flush(true)` at `level: 0` passes `undefined` to `wfblk` (`shft(NaN) = 0`).
     All corpus `flush(true)` calls occur at `level > 0` with `this.s.r < 8` (and `flush(true)` at
     `level: 0` is excluded from the parity domain).
  3. `Deflate`/`Gzip`/`Zlib` `push` after `final = true` clears `this.s = {}` and `this.b = et`
     (lines 1082–1083), so `this.s.l` no longer guards and nothing reports
     `FlateErrorCode.StreamFinished (4)`. A non-empty push then throws a JS `RangeError`
     (`this.b.set(chunk, ...)` on the empty `et`). An empty push does not throw: with `final` it
     emits a second final block (`03 00` for `Deflate`; `Gzip` follows it with its running CRC-32
     and length again, and no header), without `final` it emits nothing. `flush()` after `final`
     does not throw either: it emits a one-byte block (`02`), and `flush(true)` also the
     `00 00 00 FF FF` sync marker. All ports match npm on these (2026-11-20 review); they stay
     outside the corpus.
     Push-after-final `StreamFinished` is tested on `Inflate` (where `this.d` stays `true`).
  4. `deflateSync` with a non-empty `dictionary` on completely incompressible data where `bs = 0`
     causes `wblk` to emit dictionary bytes inside a stored block. All dictionary compression cases
     are compressible or use `level: 0` (where `st.w = dict.length` skips the dictionary cleanly).
  5. Corrupt `inflt` streams with distance symbol `dsym == 31` (JS `fd[31] === undefined` → `NaN`)
     or oversubscribed dynamic code-length trees with `v >= 32768`.

**In the domain, not in the corpus.** A GZIP trailer may claim an ISIZE above the decoded size:
`gunzipSync` allocates `new u8(ISIZE)` and returns `buf.subarray(0, bt)`, so npm returns the decoded
bytes whatever the claim, and ports must too. The corpus has no case whose ISIZE (or `outSize`)
needs a buffer of 64 MiB or more, because every port pays for that allocation on every call
(64 MiB already costs the Rust port about 1.2 GB of memory). The Rust port cannot run such inputs at
all from 128 MiB: a zeroed `Vec<u8>` is a boxed JS `Array` under mires, which V8 refuses past 2^27
elements and which aborts the Node process from about 224 MiB
([`MIRES-021`](../../MIRES-BUGS.md), a compiler limit, not a port deviation).

## Bench cases and variants

- **`bench-compress`**: `gzipSync` (`level: 6`, `mtime: 0`) on 65,536 bytes (64 KiB) of deterministic
  mixed-entropy text (JSON API events, HTML fragments, structured server logs, prose, and ~2%
  non-ASCII UTF-8; compresses to 21,926 bytes, 2.99× ratio; ~0.8–2 ms per call in JS).
- **`bench-decompress`**: `gunzipSync` on 42,992 bytes of level-6 GZIP (131,072 bytes / 128 KiB
  uncompressed, 3.05× ratio), stored in `corpus.json` as `{ "text": "...", "latin1": true }` (~0.8–1.5 ms
  per call in JS).

**Bench variant** (`bench-variant.mjs`, registry `bench.variant`):

- For `bench-compress`, applies a `k`-seeded bijection on ASCII letters (`a`–`z` and `A`–`Z`, same
  permutation for both cases) across the entire text. Content changes throughout while preserving
  exact byte length, UTF-8 validity, LZ77 match structure, symbol frequencies, and Huffman tree depths.
- For `bench-decompress`, decompresses the base stream once (cached inside the variant generator),
  applies the `k`-seeded ASCII letter bijection to a fresh copy of the 128 KiB UTF-8 plaintext,
  re-compresses with `gzipSync(..., { level: 6, mtime: 0 })`, and encodes to Latin-1 via
  `strFromU8(..., true)`. Every `k >= 1` yields a completely distinct valid GZIP stream (different
  blocks, Huffman tables, literals, and CRC-32) with equivalent decompression work (identical 131,072-byte
  output size and ~3.05× compressed ratio).

**Firefox WebDriver CCW timing note.** Under the current `tools/lib/browser.mjs` harness, Firefox
Marionette's `executeAsyncScript` wraps page inputs in cross-compartment wrappers (CCWs), which
causes `TextEncoder.prototype.encode()` inside `fflate`'s JS reference on `bench-compress` to slow
down dramatically (~14 ms per call instead of ~1–2 ms, making ports appear artificially fast on
Firefox `bench-compress`). Ports **must remain idiomatic and must NOT work around this**; the
harness's `browser.mjs` will be fixed cleanly before L4 once existing library receipts can be
re-recorded together.

## Corpus

89 deterministic cases generated by `generate-corpus.mjs`:

| Case | Op | What it exercises |
| --- | --- | --- |
| `deflate-empty` | `deflateSync` | empty input → single fixed-Huffman EOB block (`03 00`) |
| `deflate-empty-level0` | `deflateSync` | level 0 on empty input → empty stored block (`01 00 00 FF FF`) |
| `deflate-1byte` | `deflateSync` | 1-byte input skipping main LZ77 `i + 2 < s` loop |
| `gzip-empty` | `gzipSync` | empty GZIP frame with `mtime: 0` (20 bytes) |
| `zlib-empty` | `zlibSync` | empty Zlib frame (`78 9C 03 00 00 00 00 01`) |
| `deflate-level-0` .. `deflate-level-9` (10 cases) | `deflateSync` | all 10 compression levels on crafted 15.2 KB chain/nice probe (incl. L9 chain depth 2500 > 2048) — all 10 produce strictly distinct outputs |
| `deflate-mem-0` | `deflateSync` | level 1 with `mem: 0` (`plvl = 12`, 4096-entry hash table) — strictly distinct from default `mem` and `mem: 8` |
| `deflate-mem-8` | `deflateSync` | level 1 with `mem: 8` (`plvl = 20`, `bs1 = 7`, `bs2 = 14`) — strictly distinct from default `mem` and `mem: 0` |
| `deflate-stored-fallback` | `deflateSync` | level 6 on 1,500 random bytes triggering `wblk` stored-block fallback (`BTYPE=0`) |
| `deflate-fixed-huffman` | `deflateSync` | short text where fixed Huffman (`BTYPE=1`) beats dynamic and stored |
| `deflate-rle-overlap` | `deflateSync` | overlapping RLE runs hitting max match length 258 (`fl[28] = 258`) and distances 1..4 |
| `inflate-rle-overlap` | `inflateSync` | decoding overlapping back-references (`dist < len`, length 258) |
| `deflate-max-distance-32767` | `deflateSync` | match at exact `maxd = 32767` boundary (`fd[29]` + 13 extra bits) |
| `inflate-max-distance-32768` | `inflateSync` | hand-crafted stream decoding exact distance 32,768 (`fd[29] + 8191`) |
| `deflate-huffman-15bit-clamp` | `deflateSync` | 24,345-byte zero-repeat Fibonacci block forcing `hTree(lf, 15)` clamping (unclamped depth 19 → 15), debt rollback, and empty distance tree (`lc(et)`) |
| `deflate-split-lc` | `deflateSync` | 22,770-byte (< 24,576) 3-byte-match payload forcing multi-block split purely via `lc > 7000` |
| `deflate-multiblock-large` | `deflateSync` | 70 KB payload splitting across multiple blocks (`lc > 7000` / `li > 24576`, `plvl = 17`) |
| `deflate-level0-65536` | `deflateSync` | level 0 on 65,536 bytes splitting into 65,535 + 1 stored blocks |
| `deflate-level0-65535` | `deflateSync` | level 0 on exact 65,535 bytes exercising `i < s + lst` trailing empty stored block |
| `inflate-multiblock-mixed` | `inflateSync` | multi-block stream combining stored (`0`), fixed-Huffman (`1`), and genuine dynamic-Huffman (`2`) blocks |
| `deflate-with-dictionary` | `deflateSync` | 34 KiB custom dictionary verifying exact `-32768` truncation (marker before cutoff dropped, marker inside 32 KiB matched) |
| `inflate-with-dictionary` | `inflateSync` | back-references reaching into custom dictionary (`bt < dt`, `shift = dl - dt`) |
| `zlib-with-dictionary` | `zlibSync` | level 0 + 34 KiB dictionary (`FDICT=1`, `FCHECK=31` `78 3F`, Adler-32 DICTID over full untruncated dictionary) |
| `unzlib-with-dictionary` | `unzlibSync` | 6-byte FDICT Zlib header with matching dictionary |
| `gzip-with-dictionary` | `gzipSync` | GZIP compression with custom dictionary and `mtime: 0` |
| `gunzip-with-dictionary` | `gunzipSync` | GZIP decompression with custom dictionary |
| `gzip-mtime-filename-l9` | `gzipSync` | explicit non-zero `mtime`, `filename` (`FNAME`), and `level: 9` (`XFL = 2`) |
| `gzip-level1-empty-filename` | `gzipSync` | `level: 1` (`XFL = 4`) and empty `filename: ""` (falsy → omits `FNAME`) |
| `gunzip-all-header-flags` | `gunzipSync` | valid GZIP with `FTEXT \| FHCRC \| FEXTRA \| FNAME \| FCOMMENT` all set |
| `zlib-level-9-adler-wrap` | `zlibSync` | `level: 9` (`78 DA`) on 6,000 `0xFF` bytes stressing Adler-32 2655-byte reduction |
| `zlib-adler-mod-65521` | `zlibSync` | 257-byte payload where final Adler-32 `a = 65521` (`% 65521 = 0` vs `% 65536 = 65521`) |
| `decompress-gzip` | `decompressSync` | auto-detects GZIP magic `1F 8B 08` |
| `decompress-zlib` | `decompressSync` | auto-detects Zlib header (level 1 `78 5E`) |
| `decompress-raw-deflate` | `decompressSync` | auto-detects raw DEFLATE |
| `decompress-raw-0x78-ambiguous` | `decompressSync` | raw DEFLATE starting with `0x78` where `(b0 << 8 \| b1) % 31 != 0` falls through to `inflateSync` |
| `inflate-empty-outsize` | `inflateSync` | empty input with `outSize: 8` returns full zeroed 8-byte buffer |
| `inflate-outsize-exact` | `inflateSync` | pre-allocated `outSize` matching exact output length |
| `inflate-outsize-larger` | `inflateSync` | `outSize` larger than output returns trimmed subarray |
| `inflate-outsize-smaller` | `inflateSync` | Huffman stream with too-small `outSize` silently truncates to `outSize` |
| `inflate-outsize-smaller-stored-throws` | `inflateSync` | stored-block stream with too-small `outSize` throws (`both-threw`) |
| `str-to-u8-utf8` | `strToU8` | ASCII, NUL, 2-, 3-, and 4-byte astral UTF-8 encoding |
| `str-to-u8-latin1` | `strToU8` | all `0x00..0xFF` bytes plus `>0xFF` low-byte truncation |
| `str-from-u8-utf8` | `strFromU8` | leading BOM stripped, embedded BOM preserved, 2/3/4-byte astral surrogate pairs |
| `str-from-u8-utf8-invalid` | `strFromU8` | ill-formed UTF-8 bytes exercising WHATWG maximal-subpart `U+FFFD` replacement |
| `str-from-u8-latin1` | `strFromU8` | all 256 byte values `0x00..0xFF` decoded to UTF-16LE |
| `stream-deflate-flush-and-wrap` | `Deflate` | small pushes, `flush()`, `flush(true)`, >8 KiB auto-emit, >164 KiB buffer grow (`endLen > 2*b.len - 32768`) + window wrap, final |
| `stream-deflate-incomp-first` | `Deflate` | 200-byte incompressible first chunk + `flush()` exercising `bs = 0` start quirk (emits non-stored block) |
| `stream-deflate-level0` | `Deflate` | level 0 streaming with chunks and `flush()` |
| `stream-inflate-1byte-chunks` | `Inflate` | mixed stored/fixed/genuine-dynamic (`BTYPE=2`) stream pushed 1 byte at a time + empty chunk (splits dynamic header mid-parse) |
| `stream-inflate-dynamic-splits` | `Inflate` | 2.4 KB dynamic-Huffman stream pushed in 2/3/1/5/7-byte chunks splitting dynamic header and symbols |
| `stream-gzip-flush` | `Gzip` | streaming GZIP with `mtime`, `filename`, `flush()`, `flush(true)`, >8 KiB chunk |
| `stream-gunzip-multimember` | `Gunzip` | two concatenated GZIP members (`BTYPE=2` dynamic + level-0 stored) split across headers, FNAME, dynamic trees, and trailers |
| `stream-zlib-dict-flush` | `Zlib` | streaming Zlib with dictionary and `flush(true)` |
| `stream-unzlib-dict-chunks` | `Unzlib` | streaming Unzlib with dictionary, <6-byte initial chunk, and split 4-byte trailer |
| `stream-decompress-gzip` | `Decompress` | auto-detects GZIP (`BTYPE=2` dynamic body) after leading empty chunk |
| `stream-decompress-zlib` | `Decompress` | auto-detects Zlib |
| `stream-decompress-raw` | `Decompress` | auto-detects raw DEFLATE |
| `err-code-0-unexpected-eof` | `inflateSync` | `FlateErrorCode.UnexpectedEOF (0)` on truncated Huffman stream |
| `err-code-0-stored-eof` | `inflateSync` | `FlateErrorCode.UnexpectedEOF (0)` on truncated stored block |
| `err-code-1-invalid-block-type` | `inflateSync` | `FlateErrorCode.InvalidBlockType (1)` on `BTYPE = 3` |
| `err-code-2-invalid-litlen` | `inflateSync` | `FlateErrorCode.InvalidLengthLiteral (2)` on unmapped dynamic lit/len code |
| `err-code-3-invalid-dist-code` | `inflateSync` | `FlateErrorCode.InvalidDistance (3)` on unmapped dynamic distance code |
| `err-code-3-invalid-dist-backref` | `inflateSync` | `FlateErrorCode.InvalidDistance (3)` on back-reference past output + dict |
| `err-code-4-stream-finished` | `Inflate` | `FlateErrorCode.StreamFinished (4)` on push after `final = true` |
| `err-code-5-no-stream-handler` | `Deflate` | `FlateErrorCode.NoStreamHandler (5)` when `ondata` is absent |
| `err-code-6-invalid-gzip` | `gunzipSync` | `FlateErrorCode.InvalidHeader (6)` on bad GZIP magic |
| `err-code-6-invalid-zlib` | `unzlibSync` | `FlateErrorCode.InvalidHeader (6)` on bad Zlib FCHECK |
| `err-code-6-zlib-need-dict` | `unzlibSync` | `FlateErrorCode.InvalidHeader (6)` when `FDICT=1` without dictionary |
| `err-code-6-zlib-unexpected-dict` | `unzlibSync` | `FlateErrorCode.InvalidHeader (6)` when `FDICT=0` with dictionary |
| `err-code-6-unzlib-short-trailer` | `Unzlib` | `FlateErrorCode.InvalidHeader (6)` on final push with `< 4` trailer bytes |
| `err-uncaught-invalid-block` | `inflateSync` | uncaught `FlateError` verifying harness `both-threw` parity |
| `err-uncaught-invalid-gzip` | `gunzipSync` | uncaught `FlateError` verifying harness `both-threw` parity |
| `inflate-literal-run-200k` | `inflateSync` | one dynamic block of 200,000 one-bit literals (25,014 bytes): `buf = sl * 3` grows once at block start to 150,084 bytes and the literals past it are dropped |
| `decompress-literal-run-200k` | `decompressSync` | the same block through format detection (raw DEFLATE) |
| `stream-inflate-literal-run-200k` | `Inflate` | the same block in two pushes: the first overruns the 131,072-byte stream buffer mid-block (`st.b` past the buffer, clamped emit and window), the second resumes the block |
| `inflate-literal-run-200k-backref` | `inflateSync` | 200,000 literals then one length-3, distance-1 copy: the copy's `cbuf` grows the buffer past `bt`, so the 49,916 dropped literals read back as zeros and the copy repeats one (200,003 bytes) |
| `stream-gunzip-1000-members` | `Gunzip` | 1,000 concatenated one-line GZIP members in one final push: 1,000 nested `push` calls, the parity-domain bound |
| `zlib-1mb-l1-mem12` | `zlibSync` | 1 MiB of word-salad text (1,119,186 UTF-8 bytes) at `level: 1`, `mem: 12` (`plvl = 24`, the largest hash table): many blocks, window wraps, Adler-32 over 1 MB |
| `bench-compress` | `gzipSync` | benchmark: 64 KiB mixed-entropy text, `level: 6`, `mtime: 0` |
| `bench-decompress` | `gunzipSync` | benchmark: 128 KiB mixed-entropy text compressed at `level: 6` |

## Guidance for ports

What every port needs, then per-language notes. The Rust port (`ports/fflate/rust/`, `NOTES.md`) is
a worked reference for every point.

**Everyone.**

1. **Bit-exact `fflate@0.8.3` algorithm.** Follow `hTree` (strict `<` in merge, stable sorts,
   signed debt clamping & rollback), `lc` (empty `ddt` → `{ c: [0], n: 1 }`), `wblk` (exact `<=`
   vs `<` tie-breaks, `bs = 0` on first block), `dflt` (exact `deo` table, rarest-2-byte jump,
   `i < s + lst` at level 0), and `inflt` (zero-padded OOB reads in `bits`/`bits16`, byte-by-byte
   RLE copy, silent truncation vs stored `RangeError` when `!resize`, and typed-array stores in
   both modes: model the buffer's length, grow it only where `cbuf` runs, drop every store past
   its end) faithfully.
2. **Unsigned 32-bit shifts.** CRC-32 (`c >>> 8`) and `wbytes` (`v >>>= 8`) require **logical**
   unsigned right shifts; `hTree`'s `dt >>= lft` and `dflt`'s `endLen & -32768` require signed/bit-mask
   semantics. Watch `i32` overflow on `268435456 | (revfl[l] << 18) | revfd[d]` (fits in signed 31-bit
   positive `i32`: max is `0x1FFFFFFF`).
3. **Avoid 32 KiB static literals for `rev` and `revfd`.** Both can be computed cleanly at init or
   in O(1) (`rev15` in 4 bitwise ops; `revfd(d)` via `clz32`) to keep bundle size small.
4. **WHATWG `strFromU8`.** Strip one leading UTF-8 BOM (`EF BB BF`) and use WHATWG maximal-subpart
   `U+FFFD` replacement (or call host `TextDecoder` where idiomatic). Return exact UTF-16LE bytes.
5. **Streaming `Gunzip` recursion.** Reproduce `Gunzip.prototype.push` literally so single-member
   and multi-member GZIP emit the exact same sequence of `(chunk, false)` / `(empty, false)` /
   `(empty, true)` callbacks.
6. **Zero cross-call mutable state.** Only immutable lookup tables may live at module scope; never
   cache inputs, compressed buffers, or mutable scratch buffers across `run` calls.

**Rust (mires).** `no_std` + `alloc`, `#![deny(unsafe_code)]`, no `js!{}`. Return outputs via
`mires_js::JsUint8Array::from_slice(&out)` (one bulk host copy). Read UTF-8 strings via
`text.read::<&str>().as_bytes()` and Latin-1 strings via `charCodeAt` (or byte loop). Avoid
`core::slice::sort` (~18 KB per element type in mires): `hTree` sorts at most 288 elements, so a
small custom stable sort (insertion or merge sort) is ~20 lines and <400 bytes of JS. Keep hot
buffers (`prev`, `head`, `syms`, `buf`) in `Vec`s without per-element allocations, and check
`MIRES-BUGS.md` for known compiler pitfalls.

**ReScript / Melange.** Direct access to `Js.TypedArray2.Uint8Array` / `Uint16Array` /
`Int32Array` makes `dflt`, `inflt`, `hMap`, `wblk`, and `crc` map almost 1-to-1 onto JS typed
arrays with zero conversion overhead at the `Uint8Array` boundary. Watch OCaml 31-bit/32-bit
signed `int` (`lsr` vs `asr`, `land 0xFF`); `0xEDB88320` is negative in 32-bit signed int (`-306674912`),
so use logical shift `lsr 8` in CRC-32 and `wbytes`.

**js_of_ocaml.** Use `Typed_array.uint8Array` (or `Bytes.t` converted at the boundary with
`Typed_array.Bigstring` / bulk copy) and `Int32` or JS typed arrays for `syms`/`head`/`prev`.
Remember OCaml's `int` is 32-bit signed in jsoo: use `lsr` for unsigned shifts (`c lsr 8`), and
avoid `Bytes` copying per byte in hot loops. `TextDecoder` / `TextEncoder` or native UTF-8/UTF-16LE
codec both work cleanly.

**Scala.js.** `scala.scalajs.js.typedarray.{Uint8Array, Uint16Array, Int32Array}` (or Scala
`Array[Byte]`/`Array[Int]` with `>>>` unsigned shift; note Scala `Byte` is signed `-128..127`, so
always mask `& 0xFF` when reading bytes!). Return a `js.typedarray.Uint8Array` directly. Avoid
`Long` (emulated 64-bit); all `fflate` arithmetic fits in 32-bit `Int` (use `Double` only for
`Math.floor(mtime / 1000)`).

**Dart (dart2js).** `dart:typed_data` (`Uint8List`, `Uint16List`, `Int32List`) compiles directly
to JS typed arrays, and `Uint8List` crosses `dart:js_interop` (`JSUint8Array`) cleanly. Note that
Dart `int` is a JS double on web (`>>>` is 32-bit unsigned right shift, `~/` truncates toward zero,
`& 0xFFFFFFFF` keeps unsigned 32-bit values). `List.sort` is not guaranteed stable: break ties
explicitly by symbol index `s` in `hTree`.

**Kotlin/JS.** `ByteArray` (`Int8Array`) is signed (`b.toInt() and 0xFF`), or use `UByteArray` /
`org.khronos.webgl.Uint8Array` (`Int32Array`, `Uint16Array`) for zero-copy return. Watch Kotlin
`ushr` (`>>>`) vs `shr` (`>>`), and avoid boxing primitives in `hTree` (store parallel `IntArray`s
for symbol, freq, left, right instead of `List<Node>`). `Array.sortWith` is stable.

**Elm (+ JS host).** Elm has no mutable typed arrays; keep the host (`js/index.js`) strictly a
thin JSON/port boundary (passing byte arrays or strings through ports and converting the final
`List Int` to `Uint8Array` — keep host JS well under half the bundle so it avoids the "mostly JS"
label!). Inside Elm, use `Array Int` (HAMT) or `elm/bytes` + `Array` for sliding window and hash
tables, and `Bitwise.shiftRightZfBy` for unsigned shifts (`>>>`). Note `elm/core` `Bitwise`
operates on signed 32-bit ints, and `List.sortWith` is stable.

**GopherJS (Go).** Go's `[]byte` (`uint8`), `[]uint16`, `[]int32`, and unsigned `uint32` shifts
(`>>`) match `fflate` directly! Pass output back via `js.Global.Get("Uint8Array").New(...)` or
`js.InternalObject(out)` (`GopherJS` wraps `[]byte` as a `Uint8Array` slice — return a fresh
`Uint8Array` of the exact slice length, never the full backing `ArrayBuffer`!). Use
`sort.SliceStable` (or explicit `(f, s)` tie-break) in `hTree`.
