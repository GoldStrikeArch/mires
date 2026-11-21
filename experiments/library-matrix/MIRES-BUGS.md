# `mires` bugs and performance cliffs found by the library matrix

This repository log records compiler refusals, runtime defects, incorrect generated code and
performance cliffs encountered by Rust ports. The standalone examples each have a `run.sh` that
writes its build and check receipts under `build/library-matrix/mires-bug-repros/`. [Reproduction cases](mires-bugs/)

## Policy during matrix porting

1. **Never work around a `mires` bug silently.** Whenever a Rust port hits a compile refusal,
   runtime refusal/crash, or silent miscompile:
   - Assign the next sequential ID (`MIRES-NNN`) in this file.
   - Extract a minimal standalone repro under `experiments/library-matrix/mires-bugs/MIRES-NNN-<slug>/`
     (containing `README.md`, `matrix.json` with a relative `dir`, `port/port.json`,
     `port/src/lib.rs`, `run.sh`, and `error.txt` when build-time).
   - Record the exact `git rev-parse HEAD` compiler SHA, profile/flags, expected vs. actual
     behaviour, and the workaround used in the port.
   - Tag the workaround in both the Rust port source (`// WORKAROUND(MIRES-NNN): ...`) and the
     port's `NOTES.md`, so it can be removed cleanly once fixed.
   - Ports never modify compiler source. If a bug blocks a faithful port without distorting the
     library's algorithm, pause and escalate rather than shipping an unfaithful port.
2. **Record performance cliffs separately** in the [Performance cliffs](#performance-cliffs) table
   whenever profiling reveals a clear compiler/runtime hotspot (no need for a failing repro).

---

## Bug index

| ID | Status | Severity | Found by | Compiler SHA | Summary | Repro / Fixture | Workaround in port |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `MIRES-001` | fixed (`95ca59e` / `34d7b4b`) | silent miscompile (TypeError at runtime) | `minisearch` | `46383cb` | `<(T, U) as Clone>::clone` (and async-closure clones) write into an uninitialized container local (`b[0]=...` with `b === undefined`) | [`mires-bugs/MIRES-001-tuple-clone-uninitialized/`](mires-bugs/MIRES-001-tuple-clone-uninitialized/) (fixtures 340, 346) | Custom structs / manual element clone |
| `MIRES-002` | fixed (`fed7d0f`) | silent miscompile + runtime refusal | `minisearch` | `46383cb` | `Box::<[T]>::from(&[T])` reads wrong elements via SliceView shortcut and throws `not a live heap block` on drop | [`mires-bugs/MIRES-002-boxed-slice-dealloc/`](mires-bugs/MIRES-002-boxed-slice-dealloc/) (fixture 341) | `slice.to_vec().into_boxed_slice()` |
| `MIRES-003` | fixed (`92514de`) | compile refusal | `minisearch` | `46383cb` | `Vec<A>::into_iter().map(..).collect::<Vec<B>>()` refused in `in_place_collect::from_iter_in_place` (pointer cast across element types) | [`mires-bugs/MIRES-003-in-place-collect/`](mires-bugs/MIRES-003-in-place-collect/) (fixture 343) | `iter()` / explicit push loop |
| `MIRES-004` | fixed (`c153828`) | compile refusal | `minisearch` | `46383cb` | `a < b` on `&[T]` (and `sort`/`sort_unstable` of slice keys) refused on `ControlFlow<bool>` niche transmute | [`mires-bugs/MIRES-004-slice-partial-ord-lt/`](mires-bugs/MIRES-004-slice-partial-ord-lt/) (fixture 344) | `sort_by(\|a, b\| a.cmp(b))` |
| `MIRES-005` | fixed (`d93c692`) | silent miscompile (OOB read) | `minisearch` (MIR inline probe) | `46383cb` | `[u16] == [u16]` with MIR inlining on read past buffer end in `__rt.compare_bytes` | fixture 345 | Matrix kept `JSC_RUST_MIR_INLINE=off` |
| `MIRES-006` | open (M2 queue) | silent miscompile | M1 audit | `46383cb` | Constant `NonZero::new(5)` decodes to `{}` under MIR inlining + `-Oz` | (tracked in `tasks/todo.md` M2) | — |
| `MIRES-007` | open (M2 queue) | silent miscompile | M1 audit | `46383cb` | Reading `[u16]` through a `u8` view returns whole elements (`Hash` for `Vec<u16>` wrong in JIR, throws in v1) | (tracked in `tasks/todo.md` M2) | — |
| `MIRES-008` | open (M2 queue) | silent miscompile | M1 audit | `46383cb` | Byte arithmetic (`add`/`copy`/`write_bytes`) on a retyped allocation pointer operates on whole elements | (tracked in `tasks/todo.md` M2) | — |
| `MIRES-009` | open (M2 queue) | silent miscompile | M1 audit | `46383cb` | Older v1 lowering path `transmute::<NonNull<u8>, usize>` returns `0` | (tracked in `tasks/todo.md` M2) | — |
| `MIRES-010` | open (M2 queue) | silent miscompile | M1 audit | `eb24b87` | Signed-niche decode (`niche_from_scalar` compares `254`, not `-2`) | (tracked in `tasks/todo.md` M2) | — |
| `MIRES-011` | open (M2 queue) | compile/runtime refusal | M1 audit | `46383cb` | Dropping `Box<str>` (from any source) refused | (tracked in `tasks/todo.md` M2) | — |
| `MIRES-012` | open (M2 queue) | compile refusal | M1 audit | `46383cb` | `Rc::make_mut` on a shared `Rc<(String, u32)>` refused | (tracked in `tasks/todo.md` M2) | — |
| `MIRES-013` | open (M2 queue) | compile refusal | M1 audit | `46383cb` | `(1..6).map(Wrapped)` refused under MIR inlining + `-Oz` | (tracked in `tasks/todo.md` M2) | — |
| `MIRES-014` | open (M2 queue) | runtime crash | M1 audit | `eb24b87` | Dropping an async closure that captured a `Vec<T>` throws `native_vec_set_len ... buf.fill is not a function` | (tracked in `tasks/todo.md` M2) | — |
| `MIRES-015` | open (M2 queue) | compile refusal | M1 audit | `46383cb` | `as_chunks` / slices of `Vec<[u8; 3]>` refused | (tracked in `tasks/todo.md` M2) | — |
| `MIRES-016` | open (M2 queue) | compile refusal | M1 audit | `46383cb` | Coroutine clone shims (unstable) and multi-niche transmutes refused | (tracked in `tasks/todo.md` M2) | — |
| `MIRES-017` | open | silent miscompile (runtime crash) | `marked` | `f525806` | JIR structurizer `switch (d)` omits `break;` on its final case (`case 1:`) before `default: throw new Error("rustc_codegen_js: bad block "+d)`, crashing on every normal return from irreducible loops with owned drops | [`mires-bugs/MIRES-017-switch-last-case-missing-break/`](mires-bugs/MIRES-017-switch-last-case-missing-break/) | Extract loop body / simplify control flow in `tok_blockquote` so structurizer does not emit a fallthrough return case |
| `MIRES-018` | open | runtime refusal | `marked` | `f525806` | `str::is_ascii()` / `[u8]::is_ascii()` under `js-str=bytes` runs `core` SWAR unaligned `u32` reads and throws `rustc_codegen_js: byte offset N cannot form a u32 scalar view` on any slice of length ≥ 4 with misaligned offset or length % 4 ≠ 0 | [`mires-bugs/MIRES-018-slice-is-ascii-unaligned-u32-view/`](mires-bugs/MIRES-018-slice-is-ascii-unaligned-u32-view/) | Byte-by-byte loop `s.as_bytes().iter().all(\|&b\| b < 0x80)` |
| `MIRES-019` | open | silent miscompile | `marked` | `f525806` | Promoted constant `Option<&u8>` (e.g. `Some(&b'#')`, `Some(&b'.')`) silently decodes to `{TAG:"None"}` / `d="None",e=void 0`, causing `slice.last() == Some(&b'#')` to always return `false` | [`mires-bugs/MIRES-019-option-ref-const-decodes-none/`](mires-bugs/MIRES-019-option-ref-const-decodes-none/) | `slice.last().copied() == Some(b'#')` or explicit index check `bytes[len - 1] == b'#'` |
| `MIRES-020` | open | compile refusal | `@noble/hashes` SHA-512 | `f525806` | A caller with cleanup paths invokes a capturing closure that returns a pair of words. JIR encoding declines the call terminator with `SpreadNotTuple`, and semantic cleanup forbids baseline fallback. | [Runnable repro](mires-bugs/MIRES-020-closure-spread-cleanup/) | Pass the slice explicitly to a named function. |
| `MIRES-021` | open | runtime refusal, then process abort | `fflate` `gunzipSync` (ISIZE) | `f525806` | `vec![0u8; n]` lowers to `__rt.alloc_zeroed`, a boxed JS `new Array(n).fill(0)`. Above 2^27 bytes (128 MiB) it throws `RangeError: Invalid array length`; from about 224 MiB Node aborts with an uncatchable heap out-of-memory. Native Rust and a JS `Uint8Array` allocate the same buffer. | [Runnable repro](mires-bugs/MIRES-021-alloc-zeroed-large-array/) | None. Inputs whose claimed ISIZE needs 64 MiB or more are outside the fflate corpus. |
| `MIRES-022` | open | runtime refusal | MIRES-017 verification (probe `v05_sort.rs`) | `f525806` | `Vec<String>::sort_unstable()` on 21 or more elements throws `` `copy_nonoverlapping` over 1 elements at N reaches past the end of a buffer of 1`` in core's small sort (`sort4_stable`); up to 20 elements core sorts by insertion and succeeds | [Runnable repro](mires-bugs/MIRES-022-sort-unstable-strings-smallsort/) | None needed: no Rust port calls `sort_unstable`. |
| `MIRES-023` | open | silent miscompile | MIRES-020 verification (probe `v4p_nan_plain.rs`) | `f525806` | An `f32`/`f64` constant that is a NaN with a payload is emitted as the literal `NaN`, so `to_bits()` reads the canonical NaN. Under the matrix profile (`JSC_RUST_MIR_INLINE=off`) this hits a `const`; with `JSC_RUST_MIR_INLINE=on` and `-Oz`, MIR inlining also folds `f32::from_bits(0x7fc0_0001)` arguments into such constants | [Runnable repro](mires-bugs/MIRES-023-nan-payload-constant/) (both settings) | None needed: the ports use only the canonical `f64::NAN`. |
| `MIRES-024` | open | compile refusal | MIRES-020 verification (probe `v3_fnitem_ref.rs`) | `f525806` | A promoted `&fn_item` passed to `F: Fn` (`apply_ref(&twice)`) is refused in every mode: ``constants of type `fn(u32) -> u32 {twice}` cannot be read out of an allocation`` (`ConstantUnencodable`). On `f525806` the unpromoted forms reach `MIRES-020` instead | [Runnable repro](mires-bugs/MIRES-024-promoted-fn-item-ref/) | None needed: no port passes a function item by reference. |
| `MIRES-025` | open | silent miscompile | `ky` | `f525806` | `str::to_ascii_lowercase` / `to_ascii_uppercase` and `make_ascii_lowercase` / `make_ascii_uppercase`, which convert through `&mut str`, return or leave the text unchanged; `[u8]`, `Vec<u8>`, `u8` and `char` conversions and `str::to_lowercase` work | [Runnable repro](mires-bugs/MIRES-025-str-ascii-case-unchanged/) | `to_lowercase` / `to_uppercase` (JS's own mappings, which Ky uses) in `ports/ky/rust/src/ky.rs`, tagged `WORKAROUND(MIRES-025)`. |

The follow-up ports reproduced two existing compiler bugs. The diff patch decoder also reaches the missing switch break in `MIRES-017`. Sign and byte-token comparisons in long, big.js and diff reach the promoted byte-reference bug in `MIRES-019`. The ports use value comparisons or explicit control flow and include regression inputs. [long](ports/long/rust/NOTES.md) · [big.js](ports/big-js/rust/NOTES.md) · [diff](ports/diff/rust/NOTES.md)

### Infrastructure notes (from M2)
- `test.sh` never runs with `JSC_RUST_MIR_INLINE=on` (4 pre-existing inline-on failures: `15_intrinsics`, `10_panic`, `15_ptr`, `332_r3_assert_tc_panic`).
- The linker cost model prices runtime shim helpers by unminified source bytes, so minor shim edits can flip flat-ABI choices.

---

## Performance cliffs

| ID | Found by | Compiler SHA | Impact | Mechanism / profile evidence |
| --- | --- | --- | --- | --- |
| `PERF-001` | `minisearch` | `eb24b87` | 6.5–13.5× slower than JS reference; 318 KB min / 74 KB gz | Dominated by runtime heap side-table bookkeeping (`WeakMap`/`WeakSet` in `_heap_block`, `_put`, `dealloc`, `addr`) and GC pressure; each core `sort_by` (driftsort) instantiation adds 13–16 KB of unminified JavaScript (optimizer reports of 2026-11-21 at `f525806`: 14.9 KB in minisearch, 13.8 KB in d3-hierarchy, 15.6 and 12.7 KB for two in fflate); Unicode lowercase tables add ~25 KB. (Measured pre-`js-str=bytes` merge `3e2ffbb`.) |
| `PERF-002` | `fflate` | `f525806` | ~2.2–3.5× slower than JS reference on Node `bench-decompress` / `bench-compress`; 108 KB min | V8 CPU profile shows `16.4%` (`13.3% $rt_alloc_zeroed` + `3.1% $rt__retype`) double-zeroing typed-buffer `Vec`s: `$rt_alloc_zeroed` allocates a tagged JS `new Array(bytes).fill(0)` (2× over-allocated for `u16`, 4× for `u32`), then `$rt__retype` shrinks `p.buf.length` and runs a second per-element `mk()` closure fill loop (even for `u8`). Remaining gap comes from boxed JS `Array` backing stores vs JS `Uint8Array`/`Uint16Array`/`Int32Array` and uneliminated `while`-loop slice bounds checks under `JSC_RUST_MIR_INLINE=off`. |
| `PERF-003` | `marked` | `f525806` | ~25–28× slower than JS reference (~66–70 ms vs ~2.5 ms) on 46–52 KB documents when scanning via `&str` suffix reslicing (`src = &src[consume..]`) | Under `js-str=bytes`, V8 CPU profile shows `71.3%` of runtime (`45.4% $rt_bstr_bytes` + `20.9%` GC + `5.0% $rt__bstr_units`) spent materializing fresh JS `new Array(n)` byte arrays on every `src.as_bytes()` call after `&src[consume..]`: `$rt_bstr_sub_from` creates a new suffix JS string without sharing the parent's cached byte array, thrashing `memo.map` up to its 4 MB `memo.budget` (whereas `memo.huge` only triggers above 1 MB), turning linear parser scans into O(N²) copying and allocation (~15M array elements per 50 KB document). Switching the parser loop to an explicit `(bytes: &[u8], pos: usize)` cursor over a single `src.as_bytes()` call avoids the cliff. |
| `PERF-004` | `earcut` | `f525806` | ~4.8–5.3× slower than JS reference before hoisting slice borrows (11.6 ms vs 2.2 ms on `bench-earcut`, 7.6 ms vs 1.6 ms on `bench-refine`); ~2.7–3.3× on Node (1.07–1.73× Firefox, 2.30–2.73× Chrome) after hoisting | V8 CPU profile before hoisting shows `27.6%` of total runtime in `$rt_unscale` (`8.6%`) + `$rt__blk_get` (`6.6%`) + `Vec<Node>::index` (`6.2%`) + `$rt__retype` (`3.9%`) + `$rt_slice_from_parts` (`1.7%`) + `$rt_unscale_scalar` (`1.5%`) + `$rt__heap_block` (`1.2%`): because `RawVecInner` stores `*mut u8`, every `Vec::index` / `Vec::push` runs `$rt_unscale`, and `$rt__blk_get`'s 1-entry `_blkK` cache thrashes to `WeakMap.get` whenever a loop alternates between two `Vec`s (`nodes` + `triangles` / `block_bbox` / `sort_arr`). Additionally, non-zero `vec![val; n]` (`alloc::vec::from_elem`) allocates a fresh `{ buf, off }` pointer object per element in its fill loop (`2.0%` on `refine` before switching to zero-based sentinels). After hoisting `&[Node]` / `&mut [Node]` / `&[f64]` borrows outside hot loops and using zero-based sentinels in `refine`, `is_ear_hashed_sl` (`46.6%` of CPU) is dominated by JIR re-evaluating `a.buf[a.off + g]` 4× per `prev_z`/`next_z` step on a `Vec<Node>` whose backing JS `Array` was allocated as `new Array(cap * 48)` (48× over-allocated sparse array shrunk in `$rt__retype`), plus `6.1%` from `PERF-002` zeroed-buffer double-init in `refine`. |
| `PERF-005` | `marked` | `f525806` | Stack overflow (`RangeError: Maximum call stack size exceeded`) at ~200–280 nested list levels (196 in Node in the latest run) and ~540 nested blockquotes, against ~1,000 nested lists for six of the other ports (Melange ~620, Elm ~500) and ~2,490 for marked itself in Node. The marked parity domain is bounded at 100 nesting levels ([`CONTRACT.md`](libraries/marked/CONTRACT.md), "Parity domain"). | The port keeps marked's recursion: each list level is one `Lexer::block_tokens` → `Lexer::tok_list` → `block_tokens` round, two JS frames. The mires-generated functions are very large: in the readable artifact `block_tokens` declares 235 locals and `tok_list` 300 (10.8 KB and 14.2 KB in the optimizer report), and the stack alternates between exactly these two frames when it overflows. Each frame therefore takes far more stack than marked's `blockTokens` / `list`. Evidence in `build/library-matrix/review-20261120/fix-marked/`: `deep3.mjs` bisects the failing depth per port and shape, `deep4.mjs` probes Rust and Scala.js around the limits, and `deep5.mjs` prints the alternating two-frame stack at 400 levels. The latest `deep3.mjs` output is in `build/library-matrix/review-20261120/cleanup-a/deep3-marked.log`. |


### Reproducing the earcut timings for PERF-004

From `experiments/library-matrix/`, the following commands run the current Rust port against
the pinned npm package on the triangulation and refinement workloads. The port includes the
slice-borrowing workaround measured above. [Rust port notes](ports/earcut/rust/NOTES.md)

```sh
npm ci
node tools/build.mjs --library earcut --language rust --clean
node tools/check.mjs --library earcut --language rust --environments node --readable
node tools/bench.mjs --library earcut --language rust --environments node --samples 24
```

### D3 hierarchy measurements for PERF-001 and PERF-004

Both cliffs recur on the 10,000-node, six-child branching fixture in `d3-hierarchy` at compiler `f525806`. These measurements use the default squarify treemap and the front-chain circle pack with enclosing circles. Each benchmark includes hierarchy construction and result materialization.

| Layout | Rust / npm time, Node · Chrome · Firefox | V8 self time, readable Rust bundle | Runtime calls for the first input |
| --- | --- | --- | --- |
| Treemap | 9.91× · 8.91× · 5.78× | GC 21.9%, `$rt_overwrite` 10.1%, `$rt__blk_get` 8.0%, `$rt_unscale_scalar` 5.7%, `$rt_dealloc` 4.8%, `$rt__register` 4.3% | 486,741 block lookups, 11,680 registrations, 29,969 overwrites |
| Pack | 15.28× · 12.77× · 6.12× | `$rt__put` 16.7%, GC 16.0%, `$rt__register` 14.6%, `$rt__blk_get` 5.9%, `$rt_overwrite` 3.9% | 1,416,206 block lookups, 40,920 registrations, 123,324 aggregate puts |

Pack writes 103,324 three-field `{x,y,r}` circles through `$rt__put`, which maintains the object-address side table. Allocations of 24, 48 and 72 bytes account for 25,903 of its 40,920 backing-store registrations. Treemap overwrites 20,000 two-field buffers and 9,969 two-element tuples. The V8 samples attribute time to the emitted helpers themselves. GC is reported separately because the profile cannot attribute collections to individual allocations.

From `experiments/library-matrix/`, build the readable artifact, then run the profiler on each layout. The CPU sample used Node 26.8.1, a 250 µs interval, eight warmups and 65 calls cycling over eight input variants. A second in-memory copy of the same artifact counts helper calls for variant zero. The timing ratios come from the matrix benchmark using the production artifact.

```sh
node tools/build.mjs --library d3-hierarchy --language rust
node libraries/d3-hierarchy/profile.mjs treemap
node libraries/d3-hierarchy/profile.mjs pack
```
