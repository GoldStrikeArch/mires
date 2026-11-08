# mires v2: smaller and faster output, cleanup on

Plan: ~/.claude/plans/alright-look-at-the-breezy-hejlsberg.md (approved 2026-11-08).
Execution: implementer subagents (Opus 5.5, xhigh) + verifier subagent (Fable 5.1, high) per stage.

## Stage 0 — baseline commit + measurement loop
- [x] Baseline commit of the reconstruction on main (93c0785)
- [x] `experiments/v2-restoration/measure.mjs` + `RESULTS.md` (f7e0e10)
- [x] `RCGJS_COST_PROFILE=1` compile-cost counters in jsc-link (0111435)
- [x] R0 row recorded (compact 110.6 KB gz, idiomatic 186.5 KB gz, Pro optimized link completes in 654 s)

## Stage 1 — correctness (P02/P03)
- [x] 1a ZST aggregate storage/drop: 195_zst_array_drops, audit_zst_closure_drop, audit_zst_vec_drop (merged; fixtures 114, 115)
- [ ] 1b audit_aggregate_place identity — first verification FAILED on runtime regression (struct field store 5.7–8.4×, ptr_eq 2.9×, ptr_cmp 6.3×, fflate 1.26×) and a v1-lane borrowed-local bug; sent back for rework
- [ ] check-known-gaps 6/6; suites green; R1 row

## Stage 2 — compile cost (P04), equivalent output
- [x] 2a fuse.rs catch-name reservation O(1) (9a940b3)
- [x] 2b shim parsed once in cost.rs (9a940b3)
- [x] 2c physical_cost cache key hoisted (9a940b3)
- [x] 2d dependency-aware lowering reuse + RCGJS_COST_REUSE_VERIFY (compact physical lowerings 93,917 → 2,941; verified byte-identical)
- [x] Pro optimized cleanup-on link completes: 632 s → 146 s lane time (205 s wall, load ~20), compact 70 s → 32 s; outputs byte-identical (label S2-compile-cost). Remaining B2 cost is brotli-11 in trials (Pro 104 of 138 s) — a separate policy arm, see Stage 5.

## Stage 3 — remove provably unnecessary cleanup edges (P07)
- [ ] 3a RemoveNoopLandingPads after ElaborateDrops — finding: under panic=abort the vendored pass never ran; 91% of cleanup bodies had only bare `resume` pads. Guarded run: compact gzip 110.6→106.1 KB. Full fix (body selection keyed on pre-removal state) measured 60.7 KB gzip / 610 tries; being delivered with backend/coroutine_source.rs + driver.rs changes.
- [ ] 3b plan/cleanup.rs: EmptyChain + NoThrow (may_throw summary), witnesses, report — implemented (pre-3a baseline): compact gzip 110.6→54.9 KB, try 4,182→196, core suite 138/138; under verification
- [ ] 3c no empty try for dead protected op — covered by 3b (92→0) via no-throw proofs for markers/cold_path/nop calls
- [ ] cleanup-edges tests + CONTRACT.md note; R2a row

## Stage 4 — optimize cleanup-bearing bodies (P08/P09-lite)
- [ ] 4a shared try per protected region
- [ ] 4b fusion with fences
- [ ] 4c PEA in Invoke bodies
- [ ] 4d inlining at unprotected sites
- [ ] R2 row; re-plan from attribution

## Stage 5 — next wave (chosen from runtime attribution, perf-diag 2026-11-08)
Attribution (Node, per call): marked 79.8 ms = 92% `str_bytes` re-encoding the whole document per `&str` slice (4-entry memo always evicted; 4.39 MB encoded per 8 KB input; quadratic). minisearch 1.0 ms = 30% UTF-8 round-trips (`String::as_str`→`bytes_str`, `str==str`→re-encode) + 33% WeakMap/WeakSet heap registry (`_heap_block` on every `&[u8]` reborrow, `addr` minting in `NonNull::new`, `_put`, `dealloc`). fflate 5.5 ms = 31% bit-by-bit CRC (port) with compiler-added try/catch + `with_overflow` IIFE closures + `Range::next` Option objects, 32% per-byte `Reflect.set` export (binding), rest iterator pointer-record churn.
- [ ] 5a P10 runtime strings: ASCII fast path in `str_bytes` (no `Array.prototype.slice` on typed arrays), owner/operation-lifetime byte-view reuse instead of the ring, `str::len` without full encode; measured A/B: marked 79.8→~3 ms, minisearch −24–31%
- [ ] 5a' BUG: `__rt.addr` mints `4096*++_addrNext` and never reuses; wraps to 0 after 2^20 mints → `NonNull::new` reports null → "memory allocation failed" (minisearch dies after 3,705 calls). Fix with the string work.
- [ ] 5b P16 runtime heap registry off hot paths (`_heap_block`, `addr` in fresh-block NonNull, `_put` box set, `dealloc` freed set) — after 1b lands (same shim functions); minisearch −24%, marked residual −20%
- [ ] 5c structural: keep byte-derived `&str` as `{buf,off,len}` views, convert only at the host boundary (removes the quadratic structurally) — after 5a measured
- [ ] 5d lowering: `*_with_overflow` without IIFE closure+array (`lower/js/intrinsics.rs:619-633`); `Range::next`/`Option` result objects (SROA/inlining after 3b); hoist track_caller Location literals (P13) — fflate CRC −50% of compiler-added cost
- [ ] 5e binding: bulk `Vec<u8>`→`Uint8Array` export path (fflate −39%)
- [ ] P10 strings / P12 formatter / P13–P16 bounded passes / browser timing matrix
- [ ] 2e (policy arm, not equivalence): cheaper trial compression (e.g. gzip-only or brotli q5 for trial ranking, brotli-11 only for the final candidate) — must be qualified as a decision-policy change with before/after decision diffs

## Known gaps discovered (not blocking the current stage)
- `Rc<[Z]>` built via `Rc::from(vec![Z;n])` fails on the JIR lane with `__rt.retype_rc: this pointer does not name a live heap block` (passes on the default lane). Pre-existing; found by the 1a verifier (fixture scratchpad/verify1a/fx/rc_vec.rs).
- [x] `scripts/active-poll-cleanup-test.sh` was stale (never requested js-cleanup=on); now a real gate (60/60 + baseline refusal). Open: same trap bug in scripts/compile.sh:116; wire the gate into check.sh/CI after 3a's check.sh edit lands.
- Runtime: on the current build, compiled Rust is 43–146x slower per call than the JS originals (marked 146x, minisearch 68x, fflate 43x; harness commit 75d30a2). Diagnosis running; this, not scaffolding, is the runtime problem.
