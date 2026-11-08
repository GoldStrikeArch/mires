# Medium browser workload report

## Verdict

The reduced browser admission corpus passes its complete configured matrix:

- Natural `BTreeMap<String, Vec<u32>>` compilation succeeds.
- The MiniSearch-shaped structured-`Vec` stress succeeds through 240 documents.
- The explicit byte scanner and idiomatic `str::lines`/`split_once` IndexedDB paths agree.
- Marked-, fflate-, Ky-, uPlot-, and composed Explorer-shaped workloads pass.
- Direct-promise and Rust-async service workers each pass eight install/update/activate/online/offline/cleanup cycles.
- Chrome 152, Firefox 154, and Safari 26.6.2 agree on every ordinary browser path.

All ten artifacts use JIR with zero ordinary compatibility fallback. The only fallback entries are synthesized allocator/runtime bindings, which the build receipt classifies separately.

This is corpus admission, not production qualification. These are deliberately reduced workloads rather than five complete medium-size applications. The compiler source tree is clean in this receipt; the separate [complete browser application program](../browser-applications/REPORT.md) owns the five-application scope gate. This runtime receipt therefore records `admissionPassed: true`, `semanticParityPassed: true`, and `productionQualified: false`.

## Scope and authority

Each workload recreates a characteristic pressure from an established browser project while staying small enough to diagnose compiler output:

| Workload | Pressure exercised |
| --- | --- |
| MiniSearch-shaped | Tokenization, weighted index, sorting, nested vectors, structured growth |
| Marked-shaped | Block/inline parsing, escaping, borrowed slices, string construction |
| fflate-shaped | Byte buffers, CRC32, standards-compatible stored-DEFLATE gzip framing |
| idb-shaped | Upgrades, transactions, requests, callback-to-promise adapters |
| Ky-shaped | Fetch, headers, retries, timeout, `AbortController` |
| uPlot-shaped | Numeric arrays, scale calculation, Canvas2D calls |
| Explorer | Storage, search, Markdown, gzip, and charting in one export |
| Workbox-shaped | Install, activate, update, fetch, precache, offline use, cleanup |

All Rust is safe. Builds use `js-profile=release-size`, JIR, optimizer-on, MIR inlining, and the ES2018 parser gate. The build receipt records source hashes, compiler-tree identity before and after, minifier versions, artifact hashes/sizes, lane selection, fallback classes, and the BTree admission fixture. The runtime receipt hashes the exact build receipt and re-hashes every artifact before execution.

Authoritative receipts:

- [`results/build.json`](results/build.json)
- [`results/runtime.json`](results/runtime.json)

The runtime receipt links build SHA-256 `449e1a4ebbc3775ae585c001f6ba1a9c73321a87c913c7c629f6d2a26db72eb5`, and all artifact hashes match. The compiler-tree digest is stable before and after the build at `4d1a08fd77ef30f23eda8db8d5582fdfd2721db2d92dd771d8102f7381405b05`; `compilerDirty` is false.

## Final artifact sizes

| Artifact | Minified | gzip-9 | Brotli-11 | JIR items | Ordinary fallback |
| --- | ---: | ---: | ---: | ---: | ---: |
| MiniSearch-shaped | 57,650 | 13,419 | 11,905 | 134 | 0 |
| Marked-shaped | 30,437 | 8,720 | 7,698 | 64 | 0 |
| fflate-shaped | 14,711 | 4,542 | 4,098 | 35 | 0 |
| idb byte scanner | 26,311 | 7,461 | 6,756 | 91 | 0 |
| idb `str::lines` control | 51,054 | 14,216 | 12,761 | 160 | 0 |
| Ky-shaped | 29,042 | 8,372 | 7,478 | 97 | 0 |
| uPlot-shaped | 1,917 | 1,076 | 964 | 15 | 0 |
| Explorer | 111,943 | 28,500 | 24,627 | 304 | 0 |
| Rust-async worker | 20,599 | 5,915 | 5,352 | 65 | 0 |
| Direct-promise worker | 3,772 | 1,359 | 1,216 | 23 | 0 |

Every artifact parses as ES2018. The Rust-async worker is much larger than the direct-promise form, so semantic repair does not imply a size promotion.

## Correctness blockers repaired

The first corpus run found four generic compiler/runtime boundaries. The final run pins their repairs rather than hiding them with source workarounds.

### B-tree prefix provenance

`alloc::collections::btree` casts between a larger internal-node allocation and its leaf-node field at layout offset zero. A JavaScript field projection formerly lost the containing allocation, so the reverse cast could not recover the internal node.

The pointer model now has a prefix slot that retains the containing slot and pointee size. Addressing, equality, ordering, zero-byte round trips, and the checked reverse cast use that provenance. Same-size one-field wrappers are explicitly excluded from this path. The natural BTree fixture now compiles, and the parallel runtime matrix returns checksum 3,201,520 in all three engines.

### Structured `Vec` growth

The apparent 16-to-24-document ceiling was not a capacity policy. Stable sorting allocated `[MaybeUninit<u8>; 4096]` stack storage and crossed a transparent union while retyping it. That uninitialized array-backed block had no legal one-time retype path.

`unscale_uninit` now performs the transparent-union retype without inventing initialized values. The progressive MiniSearch stress passes at 2, 4, 8, 12, 16, 24, 32, 64, 120, and 240 documents; `firstFailingDocuments` is null.

### Idiomatic string iterators

`str::Lines` reaches `memchr`, which reads machine-sized scalar words through an aliasing byte pointer. The old byte-to-word cast accepted fresh heap retyping only and rejected genuine byte storage.

Aliasing little-endian scalar byte views now cover integers, floats, and `char`, preserving writes, pointer identity, and the original byte-buffer address. Both IndexedDB parsers return exactly:

```text
a=alpha=1
b=beta=2
c=gamma=3
```

in Chrome, Firefox, and Safari. The idiomatic artifact remains 24,743 raw and 6,005 Brotli bytes larger than the scanner, so compact iterator-state lowering remains a size opportunity rather than a correctness blocker.

### Phase 12L structural candidate

The current compiler tree has five generic custom-sysroot reductions for the forward string path. `char::encode_utf8` returns a slice of a four-byte buffer, so its length is stored with an exact `u8` cast instead of an impossible fallible conversion whose `expect` retained `TryFromIntError`, `Debug`, and `core::fmt`. Forward `memchr` uses `core`'s existing safe scalar implementation because the native-memory aligned-word path expands into pointer retyping, alignment, bounds, and panic machinery over JavaScript-array bytes before returning to scalar comparisons. Checked candidate and suffix views remove a redundant slice-index panic edge. Forward `Lines::next` drives its contained `SplitInclusive` state and trims fixed CR/LF bytes directly, while retaining the standard public representation and all non-forward behavior. Finally, once `CharSearcher` has found the encoded character's last byte, exact one- through four-byte checked slice patterns compare only the preceding bytes instead of invoking generic slice equality. Reverse `memrchr` remains unchanged. The `102_str_lines` core fixture covers every UTF-8 width, same-final-byte false candidates, U+A041's repeated-byte interior hit, suffix mismatches, the former word-scan threshold, and LF/CRLF/empty/unterminated lines. It rejects the four removed support graphs and passes in the default and release-size JIR lanes.

A sixth compiler tranche extends the existing finished-JIR SROA alias proof. Direct aliases of an unassigned parameter or a dominating compiler local written exactly once now canonicalize through the stable source value. The pass redirects reads to that already-evaluated slot rather than its initializer, so it neither duplicates nor moves evaluation and preserves JavaScript object identity. Reassigned locals, replaced-field snapshots, shadowed names, and verbatim JavaScript still decline. Focused unit tests pin both the transitive scalar chain and the mutable-source counterexample.

A local release-size JIR rebuild now puts the scanner at 26,174 raw / 7,384 gzip / 6,658 Brotli bytes and the idiomatic artifact at 28,542 / 8,128 / 7,316. Relative to the five-sysroot-patch candidate, stable alias forwarding removes 137 raw / 77 gzip / 98 Brotli bytes from the scanner and 194 / 112 / 114 from idiomatic lines. Their remaining gap is 2,368 raw / 744 gzip / 658 Brotli. The independent mixed `string-iterators` control falls from the prior 18,781 / 5,675 / 5,041 candidate to 18,715 / 5,625 / 4,966 and returns the same four Node oracle values.

The same compiler tranche removes 151 bytes from the seven-item Murmur graph, from 7,686 to 7,535, while keeping zero final loop allocation sites and 0.97x ReScript runtime. The MIR-inline FingerprintJS ESM package falls from 258,451 raw / 66,224 gzip / 53,560 Brotli to 256,664 / 65,296 / 52,933. It remains 1,191 bytes above the 2.0x ReScript threshold; the exact next cluster is the 5,252-byte `x64hash128_bytes` item, which owns every remaining raw/slice view site and both tail iterator state machines.

Those figures are the predecessor to the browser-qualified Phase 12F increment. Finished-JIR SROA
now also finds connected direct-copy locals with one stable external anchor. A component with a
non-copy write, an unknown or multiple anchor, shadowing, or opaque JavaScript declines. A 32-byte
compact-function floor rejects a marginal fflate substitution that lost 29 raw bytes but added 2
Brotli bytes.

The retained rule removes 83 bytes from the Murmur core and puts its seven-item graph at 7,452
bytes, with zero loop allocations and 0.96x ReScript runtime. The complete MIR-inline package is
255,765 raw / 64,519 gzip-9 -n / 52,546 Brotli. The ordinary package is 248,076 / 60,573 /
50,279. The 2.0x target remains unmet by 1,108 bytes. A reversible source oracle changes only the
two tail adapters to scalar cursor loops and reaches 6,686 bytes, identifying a 766-byte iterator
state opportunity and leaving a 342-byte gap. The production source is unchanged.

Against the immediate compiler predecessor, eight of ten browser artifacts are byte-identical.
Ky loses 74 raw / 81 gzip / 104 Brotli bytes, and Workbox loses 74 / 104 / 104. All ten artifacts
use JIR with zero ordinary fallback and pass in Chrome 152.0.7977.66, Firefox 155.0, and Safari
26.6.2. Both service-worker variants pass eight lifecycle cycles. A separate full FingerprintJS
differential runs twice per engine and reports zero normalized differences, including its four
Murmur vectors.

After the cross-cutting alias change, the next emitted string clusters remain
`CharSearcher::next_match` at 972 bytes, `Lines::next` at 805, `split_once` at 776,
`SplitInternal::next_inclusive` at 765, and `SplitInternal::get_end` at 576. Scalar
string-iterator lowering therefore remains open even though its protected browser rerun is now
complete. The earlier causal receipts remain in
[`results/string-iterator-candidate.json`](results/string-iterator-candidate.json) and
[`results/murmur-stable-alias-candidate.json`](results/murmur-stable-alias-candidate.json). The
current build, runtime, direct differential, and Phase 12F authority are:

- [`results/murmur-copy-cycle-baseline.json`](results/murmur-copy-cycle-baseline.json)
- [`results/murmur-copy-cycle-build.json`](results/murmur-copy-cycle-build.json)
- [`results/murmur-copy-cycle-runtime.json`](results/murmur-copy-cycle-runtime.json)
- [`results/murmur-copy-cycle-differential/`](results/murmur-copy-cycle-differential/)
- [`phase-12f.json`](../../spike/rustc-codegen-js-v2/checkpoints/phase-12f.json)

### Phase 12G tail-iterator cursor

The compiler now implements the tail-adapter opportunity Phase 12F measured. JIR schema 9 carries
exact rustc diagnostic-item identities for `SliceIter`, `Enumerate`, `Take`, and `Skip`. A
finished-JIR pass replaces a fresh, non-escaping
`Enumerate<Take|Skip<SliceIter<u8>>>` driver with a checked scalar cursor only when its complete
construction, source length, loop SCC, backedges, and discardable exits are proven. Other element
types, adapter orders, and observed iterator state decline. A separate per-loop witness rechecks
the exact adapter, cursor guards, indexed borrow, enumerate index, and edges and rolls the item back
on failure. Lane-report schema 15 separates opportunities from cursors retained in final output.

The two FingerprintJS tail loops lower. The seven-item Murmur graph falls from 7,452 to 6,632 bytes
and the core item from 5,169 to 4,349. The complete MIR-inline package falls from 255,765 raw /
64,519 gzip / 52,546 Brotli bytes to 255,304 / 64,263 / 52,331. Final loop allocations remain zero,
the committed best-of-three isolated kernel result is 0.9653x ReScript, and the fresh review run is
approximately 0.98x. The responsible local range is therefore approximately 0.97x to 0.98x, well
inside the 1.05 rejection boundary. The graph is 2.0908x ReScript and remains 288 bytes above the
2.0x target.

This corpus is a neutral regression control for that narrow rule: all ten release-size ES2018
artifacts are byte-identical with `slice-iter` disabled, and all ten have zero ordinary fallback.
The final artifacts pass in Chrome 152.0.7977.66, Firefox 155.0, and Safari 26.6.2. Both worker
variants pass eight lifecycle cycles. The complete FingerprintJS differential also runs twice per
engine with zero normalized differences. These results accept the compiler tranche but do not
change this reduced corpus's production-qualification status. The clean checkpoint rerun records a
stable 1,475-file compiler digest
`d36ab104b4af830f4c7a7f58c285bea710904df8a0660cb808d89e3b9b7409df` before and after each build
and binds the evidence to committed revision `6f7ef213204220230d9e13d2063aa857d21b5b5a`.
Checkpoint files are outside that digest, avoiding circular provenance. This does not close plan
item 29's remaining 288-byte density gap or qualify production.

Phase 12G evidence:

- [`results/murmur-tail-cursor-baseline.json`](results/murmur-tail-cursor-baseline.json)
- [`results/murmur-tail-cursor-build.json`](results/murmur-tail-cursor-build.json)
- [`results/murmur-tail-cursor-runtime.json`](results/murmur-tail-cursor-runtime.json)
- [`results/murmur-tail-cursor-package.json`](results/murmur-tail-cursor-package.json)
- [`results/murmur-tail-cursor-differential/`](results/murmur-tail-cursor-differential/)
- [`phase-12g.json`](../../spike/rustc-codegen-js-v2/checkpoints/phase-12g.json)

### Boxed closures and service-worker lifetime

Owned `Box<dyn FnOnce>` virtual calls now pass the owned environment value, not the fat-pointer carrier. Separately, const-propagated `Fn` and `FnMut` environments decode from immutable allocation bytes capture by capture. The worker host boundary boxes values for `Reflect`, constructs cache names without numeric addition, keeps async install/activate work inside `waitUntil`, and reports lifecycle skips/failures explicitly.

Both worker variants now complete eight versioned cycles. Each cycle observes both precache entries, one current managed cache, a successful online fetch, and the same response while the server is forced offline. Final cleanup observes one registration before unregistering and zero managed caches afterward.

## Browser results

| Browser | Version | idb | idiomatic lines | Ky retry | uPlot | Explorer |
| --- | --- | --- | --- | --- | --- | --- |
| Chrome | 152.0.7977.65 | Pass | Pass | Pass | Pass | Pass |
| Firefox | 154.0.1 | Pass | Pass | Pass | Pass | Pass |
| Safari | 26.6.2 | Pass | Pass | Pass | Pass | Pass |

The Explorer result agrees across engines: `a:5,b:1` search hits, the expected heading, 132 stored bytes, a 155-byte gzip snapshot, 256 plotted points, and the same finite chart checksum.

The fflate-shaped pure check round-trips a 61,287-byte sample through Node `gunzipSync`. This validates the stored-block encoder and CRC path, not dynamic Huffman coding, streaming inflate, or dictionary support.

## Admission interpretation

This receipt separates four questions:

| Status | Result |
| --- | --- |
| Execution | Pass |
| Semantic parity | Pass |
| Reduced-corpus admission | Pass |
| Production qualification | Fail by scope |

The corpus is strong diagnostic and regression evidence. It does not itself satisfy the plan's requirement for five complete medium-size applications; that scope is now exercised separately by five final cross-crate bundles. This receipt also does not provide exact paired v1/v2 rows for the whole corpus, extracted-clean-checkout reproducibility, package/source-map/declaration closure, or the frozen North Star size/runtime gates.

The application program now covers streaming decompression, a larger search/index reference suite, IndexedDB cursor/abort behavior, fetch streaming/cancellation, and chart resize/event teardown. Full CommonMark conformance and broader compression formats remain open generality work.

## Reproduction

From the repository root:

```sh
npm --prefix experiments/browser-workloads run build
node experiments/browser-workloads/tools/run.mjs --pure
node experiments/browser-workloads/tools/run.mjs \
  --browsers chrome,firefox,safari --cycles 8
```
