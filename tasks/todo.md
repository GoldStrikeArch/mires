# mires v2: smaller and faster output, cleanup on

> Current local-main promotion: [integration report](../round_1_integration_report.md) and the dated addendum at the end. Earlier entries remain the round-1 investigation history.

Plan: ~/.claude/plans/alright-look-at-the-breezy-hejlsberg.md (approved 2026-11-08).
Execution: implementer subagents (Opus 5.5, xhigh) + verifier subagent (Fable 5.1, high) per stage.

## Stage 0 — baseline commit + measurement loop
- [x] Baseline commit of the reconstruction on main (93c0785)
- [x] `experiments/v2-restoration/measure.mjs` + `RESULTS.md` (f7e0e10)
- [x] `RCGJS_COST_PROFILE=1` compile-cost counters in jsc-link (0111435)
- [x] R0 row recorded (compact 110.6 KB gz, idiomatic 186.5 KB gz, Pro optimized link completes in 654 s)

## Stage 1 — correctness (P02/P03)
- [x] 1a ZST aggregate storage/drop: 195_zst_array_drops, audit_zst_closure_drop, audit_zst_vec_drop (merged; fixtures 114, 115)
- [ ] 1b audit_aggregate_place identity — round 3 FAILED verification (2026-11-10): fixes round-1 fail reasons and known gaps 6/6, but regresses vs base: (1) `emit.rs:1231-1239` SetField skip drops a restoring store (`let saved = o.a; o.a = 7; …; o.a = saved`), (2) `owns_object` treats a coroutine held by value as a pointer → `ptr::write` of a future lost, (3) pointers to nested aggregate fields go stale after `ptr::swap/copy/replace` (shallow overwrite replaces the nested object), (4) two live places share one integer address (16-byte granule for an object first addressed through its first scalar field), (5) stores of >8-leaf aggregates 5–6× slower (`MAX_LEAVES = 8` falls back to `__rt.overwrite`), (6) `es = 4096` exhausts the address space 256× sooner. WIP preserved on branch worktree-agent-a1ab8f94827df1889; fixtures in scratchpad/verify1b-r3. ESCALATED TO USER: what to do with the full model. Meanwhile: "1b-min" extracts only the pieces that fix 201 lines 54–55 / ref_moves / m1_moves (shim element moves rebind pointer values), the v1 borrowed-field fix, and the baseline-lane cell case; `audit_aggregate_place` stays OPEN as on main.
- [ ] check-known-gaps 6/6; suites green; R1 row

## Stage 2 — compile cost (P04), equivalent output
- [x] 2a fuse.rs catch-name reservation O(1) (9a940b3)
- [x] 2b shim parsed once in cost.rs (9a940b3)
- [x] 2c physical_cost cache key hoisted (9a940b3)
- [x] 2d dependency-aware lowering reuse + RCGJS_COST_REUSE_VERIFY (compact physical lowerings 93,917 → 2,941; verified byte-identical)
- [x] Pro optimized cleanup-on link completes: 632 s → 146 s lane time (205 s wall, load ~20), compact 70 s → 32 s; outputs byte-identical (label S2-compile-cost). Remaining B2 cost is brotli-11 in trials (Pro 104 of 138 s) — a separate policy arm, see Stage 5.

## Stage 3 — remove provably unnecessary cleanup edges (P07)
- [ ] 3a RemoveNoopLandingPads after ElaborateDrops — full fix committed on branch worktree-agent-afeda83ff9fd9aa67 (9920693): selected-body set identical, compact gzip 110.6→60.7 KB, try 4,182→610, core suite 138/138. BLOCKED on 1b: with no-op pads gone, 201_reference_field_pointers and audit_named_reference fail (latent aggregate-place identity bug previously masked by all-box lowering). Merge after 1b passes 6/6 with 3a applied.
- [ ] 3b plan/cleanup.rs: EmptyChain + NoThrow (may_throw summary), witnesses, report — verified sound (whitelist, fixpoint, 10 protected-op classes, mutations) but BLOCKED like 3a on the fuse.rs place-view miscompile that opening fusion exposes (201, audit_named_reference). Agent a12941501626bbe56 fixing fuse.rs.
- [ ] 3c no empty try for dead protected op — covered by 3b (92→0) via no-throw proofs for markers/cold_path/nop calls
- [ ] cleanup-edges tests + CONTRACT.md note; R2a row

## Stage 4' — re-planned from the byte attribution of the integrated build (2026-11-10, scratchpad/attrib2)
Integrated compact: 208,988 min / 55,876 gzip / 43,548 brotli. App code 62.5% of min bytes, core+alloc 24.6%, shim 7.5%. Measured levers (re-minified with the build's terser settings):
- [ ] 4'a no-throw extensions in effects.rs — round 1 FAILED verification (2026-11-10) on one point, round 2 with the implementer: `compare_bytes` was audited as no-throw but `__rt.compare_bytes` raises `TypeError` on mixed BigInt/Number buffers (an `align_to::<u8>` view of `[u64]`, pointer casts) and on host values (`valueOf` runs host code) → the caller's guard destructor is skipped (repro: scratchpad/verify4a/repro/repro_compare_bytes.rs). Everything else held (foreign, every other intrinsic family at all widths in six builds, markers), all gates pass, numbers reproduce. Round 2: drop `compare_bytes` (verifier measured 200,050 min / 53,517 gzip / 41,922 brotli, try 99), regression tests, remove dead `fabsf32/64`, a test that the three literal lists agree (3 single-list mutants survive), doc for `u8arrayfrombytes`, and an investigation of the wider reinterpreting-view class. Round-1 delivery note: Implementer's numbers, not yet confirmed: compact 209,000 → 199,785 min / 55,869 → 53,475 gzip / 43,533 → 41,912 brotli, try 210 → 95 (945 edges removed / 117 retained: callee-may-throw 66, host-contract-untrusted 47, panic-path-unknown 4); step (i) alone changes nothing because every panic path also needs `caller_location`. `host_operation_raises_nothing` now delegates to `HostOperation::may_throw()`; closure markers stay a list in effects.rs (not `HostOperation` variants); cleanup-edges JSON rebaselined, new `abort_guard` fixture, matrix.sh gains `cleanup`/`host-args`. Original scope: (i) resolve `Callee::Foreign` to the in-program definition (panic handler `rust_begin_unwind` is `for(;;){}` yet counted as throwing), (ii) audited pure intrinsics (`caller_location`, `assert_inhabited`, `ctpop`, `size_of_val`, `align_of_val`, `compare_bytes`…), (iv) closure-creation markers `callback0/1/2` and `try`. (i)+(ii) −4.9K min / −1.09K gzip; with (iv) ≈ −8K; (iii) trusting declared non-fallible host contracts is a further −7.6K min but changes destructor semantics on a throwing host getter → stays opt-in (USER DECISION). All 210 tries = −19.6K min / −4.8K gzip upper bound. Also rebaselines the cleanup-edges expected JSON and adds `cleanup`/`host-args` to matrix.sh.
- [ ] 4'b inline budget fix — round 1 FAILED verification (2026-11-10), round 2 with the implementer; cannot land before base bug 1g is fixed. With 31 sites inlined instead of 16, `struct S { f: u8 }` assigned on two paths then updated through `fn set(s: &mut S, v)` prints 7 instead of 254 (right with the base linker only because the cap hid it; found twice in 239 generated programs). Everything claimed reproduced: compact 219,278 raw / 204,846 min / 55,786 gzip / 43,366 brotli, 634 sites, kernels 58/58; idiomatic 381,396 → 372,866 min / 97,735 → 96,979 gzip / 77,551 → 77,028 brotli, 949 sites; suite 142/142 with 0 larger / 82 smaller (−9.77%); no workload larger; deterministic; the `remap.rs`/`lane.rs` refactors byte-identical. Round 2: merge integration 2f36c3f, regressions, 3 surviving `lane.rs` mutants (twin credit; origin write-back; sole oracle) + a `splice` unit test, the probe cap starving sites (11 vs 15 in a >256-site caller), representation decisions that DID change (101_btree_map_nodes 1 → 0 regions, 29_rc 0 → 1, idb loses all committed regions), three loose doc statements. Round-1 delivery note: Implementer's numbers vs a base linker at c911157, not yet confirmed: compact 230,795 raw / 208,988 min / 55,876 gzip / 43,548 brotli → 219,278 / 204,846 / 55,786 / 43,366; inlined sites 16 → 634; release-balanced suite 142/142, 0 fixtures larger, 82 smaller (−9.77% total); corpus min: marked 34,608 → 31,410, minisearch 110,706 → 107,613, fflate 32,705 → 28,956; runtime (noisy, A/A 0.74–1.22): marked 0.98, minisearch 0.73, fflate 0.79 of base. New rules: per-caller signed ledger (refunds do not cross callers), MeasuredShrink for 33–64 B callees, `inline.representation-boundary` refusal (inlining `to_string` into an export grew 46_native_f64_string 1.6 → 105 KB), twin-aware sole-caller credit, probe cap 256; `remap.rs` no longer clones the caller's own table (O group 3.9 → 1.6 s). Follow-ups it names: (4'b2) transport `#[inline]` hints (backend `encode.rs` ~246 → `AbiSignature.inline`, JIR_SCHEMA 15 → 16) so `inline(always)` tuple helpers in the hashing cluster inline and `inline(never)` is honoured; batch evaluation of a sole callee's sites; representation conversion inside an item. Original note: budget max(program/2000, 384) with a 24 B floor charge per site = exactly 16 inlined sites per program; `inline.rs:146` comment promises shrinking sites refund, `:263` never does. 643 one-statement functions / 2,169 call sites raw; hashing `#[inline(always)]` helpers stay out of line (3,879 B vs 1,766 hand-written).
- [ ] 4'c pool repeated immutable constants — round 1 FAILED verification (2026-11-10), round 2 with the implementer. Numbers reproduced (187,664 / 51,802 / 43,385, deterministic, kernels 58/58) but: (A1) `let mut p = PS[4]; p.x = 99;` over a pooled const writes into the shared table (root cause is base bug 1c below); (A2) a two-chunk program fails to load, the binding is both imported and re-declared (`link.rs:1711`, `visit` `:1946-1953`; static-to-static fails the same way at base); (B1) a `mires_js` HashMap value reaches the host (`pool.rs:600,665`); (B2) a C-ABI `extern "C"` call argument reaches the host (`pool.rs:555,594-612`); a hand-written `rcgjs.host.erase` marker leaks; documented residual 1 (shared export parameter with a `Cell`) is reachable in safe Rust; three scripts still pin sidecar schema 17 (`repr-report-test.sh`, `carrier-pea-test.mjs`, `repr-sourcemap-test.sh`); `RCGJS_POOL_FAULT` is honoured by release linkers; four false CONTRACT sentences. Thirteen typed host routes were correctly declined; copy/ownership, async, init order, pointer comparisons all matched native. Evidence: scratchpad/verify4c. Round-1 delivery note: Implementer's numbers, not yet confirmed: compact 209,000 → 187,664 min / 55,869 → 51,802 gzip / 43,533 → 43,385 brotli; corpus byte-identical (no workload repeats an eligible constant). New link option `js-pool-consts` (on in release-size/balanced), `link/pool.rs`, manifest 5 → 6, sidecar schema 17 → 18, fixture 121. Only constants behind a shared reference, ≥256 B and ≥2 uses, not reachable by the host; by-value sharing not implemented (measured share 0). Open design note: the selection is passed through a scoped thread-local instead of a `JsCx` field. Original scope: `BLOCKER_FILTERS` is emitted 4× (7 KB each), `FONT_PREFERENCES` 2×: −20.9K min / −4.04K gzip (brotli −0.1K).
- [ ] 4'd shared unreachable-abort helper (155 identical `throw __rt.abort_error(new Error("…unreachable code"))`): −8.3K min / −0.32K gzip; printer-only (emit.rs — after 1b/5d1/5a2 merge, same file).
- [ ] 4'e module-scoped shim (no `globalThis.__rt.` properties): −5.5K min / −0.89K gzip; runtime structure change.
- [ ] 4'f numeric enum tags (−2.4K / −0.17K); dead trailing arguments (−2.1K / −0.25K, `analysis/param_use.rs` exists).
- Not worth doing (measured): Location literal hoisting (gzip gets worse), numeric spellings (0 after terser), `ub_checks` folding (0 after terser; raw only).
- Shape gaps vs hand-written JS on three functions: 53–64% (un-inlined helpers, `byte_slice_ref` re-evaluated per access, bounds checks with Location literals the loop guard already proves, iterator `{TAG,_0:[i,item]}` objects, temporaries).

## Stage 4 (original plan; superseded where 4' overlaps) — optimize cleanup-bearing bodies (P08/P09-lite)
- [ ] 4a shared try per protected region
- [ ] 4b fusion with fences
- [ ] 4c PEA in Invoke bodies
- [ ] 4d inlining at unprotected sites
- [ ] R2 row; re-plan from attribution

## Stage 5 — next wave (chosen from runtime attribution, perf-diag 2026-11-08)
Attribution (Node, per call): marked 79.8 ms = 92% `str_bytes` re-encoding the whole document per `&str` slice (4-entry memo always evicted; 4.39 MB encoded per 8 KB input; quadratic). minisearch 1.0 ms = 30% UTF-8 round-trips (`String::as_str`→`bytes_str`, `str==str`→re-encode) + 33% WeakMap/WeakSet heap registry (`_heap_block` on every `&[u8]` reborrow, `addr` minting in `NonNull::new`, `_put`, `dealloc`). fflate 5.5 ms = 31% bit-by-bit CRC (port) with compiler-added try/catch + `with_overflow` IIFE closures + `Range::next` Option objects, 32% per-byte `Reflect.set` export (binding), rest iterator pointer-record churn.
- [x] 5a P10 runtime strings (merged 2026-11-08: marked 88.4→3.72 ms, linear; minisearch 1.42→1.11 ms; shim +0.8 KB gzip): ASCII fast path in `str_bytes` (no `Array.prototype.slice` on typed arrays), owner/operation-lifetime byte-view reuse instead of the ring, `str::len` without full encode; measured A/B: marked 79.8→~3 ms, minisearch −24–31%
- [x] 5a' BUG (fixed with 5a: granule free-list allocator; 1.1M allocs tested): `__rt.addr` mints `4096*++_addrNext` and never reuses; wraps to 0 after 2^20 mints → `NonNull::new` reports null → "memory allocation failed" (minisearch dies after 3,705 calls). Fix with the string work.
- [ ] 5b P16 runtime heap registry off hot paths (`_heap_block`, `addr` in fresh-block NonNull, `_put` box set, `dealloc` freed set) — after 1b lands (same shim functions); minisearch −24%, marked residual −20%
- [ ] 5c structural: keep byte-derived `&str` as `{buf,off,len}` views, convert only at the host boundary (removes the quadratic structurally) — after 5a measured
- [x] 5d1 VERIFIED (pass-with-limits) and committed on its branch as 26f9ed4; merged into `integration` (6ec5422). Bit-exact vs native for 12 integer types × 11 configs; base bug was wider than reported (32-bit `pow`, `wrapping_pow`, `*`, `*=`, `product` wrong above 2^53). Follow-up VERIFIED (pass-with-limits, 12 types × 12 configs byte-equal to native, every listed mutant fails a committed test), committed as 45edc42 and merged into `integration` (b6ab45c): a pair moved/copied before projection keeps its carrier (alias family by fixpoint), a 32-bit product read through one half carries only that half; the verifier's out-of-block-order alias test was lifted in and kills the one-pass mutant. Limits: the new `Wrapped`/`Overflowed` carriers fire in no real program yet (corpus and compact byte-identical to 26f9ed4; 17 `Math.imul` + 4 array-pair sites remain), fixture 117 does not reach them end to end, the decline path is unreachable, fixture 117 uses `#![feature(core_intrinsics)]`. Original note: overflow pairs without IIFE; projected-only pairs never built; FOUND+FIXED exactness bug: 32-bit `overflowing_mul` wrapped value used a rounded double (`u32::MAX.overflowing_mul(u32::MAX)` gave 0, native 1) → `Math.imul`. fflate closures/call 490K→4; time neutral in steady state.
- [ ] 5d (remaining) lowering: `*_with_overflow` without IIFE closure+array (`lower/js/intrinsics.rs:619-633`); `Range::next`/`Option` result objects (SROA/inlining after 3b); hoist track_caller Location literals (P13) — fflate CRC −50% of compiler-added cost
- [x] 5e VERIFIED twice (pass-with-limits), committed on its branch as eec2d38 and merged into `integration` (f427f23); export can only abort, import sized by the built-in typed-array length and may-throw. Tests-only follow-up committed and merged (118 extern checks; the 4 surviving helper mutants now fail). Original note: one host op per byte-buffer crossing (`JsUint8Array::from_slice`/`copy_to_slice`/`to_vec`); fflate 33–36× → 17.5–19.6× vs JS in Node. Note: `01_chart.js.expected` extern golden was stale on main (rebaselined there).
- [ ] P10 strings / P12 formatter / P13–P16 bounded passes / browser timing matrix
- [ ] 2e (policy arm, not equivalence): cheaper trial compression (e.g. gzip-only or brotli q5 for trial ranking, brotli-11 only for the final candidate) — must be qualified as a decision-policy change with before/after decision diffs

## Known gaps discovered (not blocking the current stage)
- SOUNDNESS CONFIRMED and fixed (uncommitted on worktree agent-a1f081d8…, under verification): cargo pipelining passes `.rmeta`, so `transport::read` returned None and NO cleanup MIR crossed crate boundaries (0→1,257 imports); a 3-crate user fixture lost every destructor on a host throw with no diagnostic. Fix: sibling-rlib lookup that fails loudly, cleanup-aware duplicate resolution (UNWIND copy wins), fail-closed zombie. Originally: alloc imports no cleanup MIR from core (zero `cleanup-mir-import` in its sysroot build; likely .rmeta vs .rlib), so alloc's copies of core generics come from optimized MIR without cleanup paths; duplicate instances are resolved by smallest semantic hash regardless of cleanup edges. Fix: transport across sysroot crates + cleanup-aware duplicate resolution + fail-closed diagnostic.
- `Rc<[Z]>` built via `Rc::from(vec![Z;n])` fails on the JIR lane with `__rt.retype_rc: this pointer does not name a live heap block` (passes on the default lane). Pre-existing; found by the 1a verifier (fixture scratchpad/verify1a/fx/rc_vec.rs).
- [x] `scripts/active-poll-cleanup-test.sh` was stale (never requested js-cleanup=on); now a real gate (60/60 + baseline refusal). Open: same trap bug in scripts/compile.sh:116; wire the gate into check.sh/CI after 3a's check.sh edit lands.
- Runtime: on the current build, compiled Rust is 43–146x slower per call than the JS originals (marked 146x, minisearch 68x, fflate 43x; harness commit 75d30a2). Diagnosis running; this, not scaffolding, is the runtime problem.
- `scripts/matrix.sh:268` pass list lacks `cleanup` and `host-args` → no leave-one-out coverage for the new pass.
- `scripts/async-test.sh:127` cds to `$root/../..` and builds `topcoat-runtime-macro` (monorepo-era); cannot run standalone. Stale script.
- Baseline fusion miscompile fixed on branch worktree-agent-a12941501626bbe56 (cadcfb0 + follow-up 0b04bdd, both VERIFIED and merged into `integration`: gate-sensitive fixture 119 and clearing-rule unit tests; the transmute crash is a separate known gap). Limit that 1b must settle: an inline borrow of a place is now `box(read)` while `&raw const o.h`/pointer arithmetic build the slot `{buf:o,off:"h"}`, so place-identity cases that passed by accident in call-free leaf bodies (verify-fuse fx p1_place, p2_place_leaf, p3_offset) need 1b's canonicalization; `write_indirect` still branched on spelling for the write side (w1_write); baseline-lane cell case r8_cell. Routed to the 1b verifier. Original note: `raw_view` of an aggregate held as a value is `box(read)` regardless of spelling; fixture 116. Residual 201 lines 54–55 are a runtime `_put` bug (overwrites a moved `&P` element's pointee) — routed to 1b.
- Pre-existing cleanup semantics observed by the 3a verifier (identical at baseline): a destructor that throws during unwinding skips the remaining drops of that frame; a field destructor throwing on the normal path skips the sibling field's drop; a coroutine guard whose drop throws during poll cleanup is dropped again by the driver (poison discriminant set after the drops). Decide the contract (Rust aborts on panic-in-drop-during-unwind) and either document or fix.
- 3a verifier nits for merge time: `check.sh` and CI both run noop-landing-pads-test (redundant; keep one); `rustc-cleanup` `had_cleanup` deserves a comment about `Builder::call_deref`'s bare `Continue` edge (harmless).
- `scripts/build_sysroot.sh:150` has the same trap-replaces-lock-release bug as compile.sh/compile_core.sh.
- Async gate follow-ups: `scripts/lane-shadow-test.sh:457-503` drives async-test.sh with the old contract (JS_EXTRA_ARGS lane override, old log paths) — its async suite is broken; `scripts/jir-coverage.sh:322-385` still has monorepo scaffolding; `CONTRACT.md:2835` describes 06_procedure as running (now SKIP). `async-check.mjs` task-table check is vacuous when hand-run without a `.lane.json` (the gate guarantees it).
- Follow-up from the transport fix: alloc's core instances now come from pre-optimization cleanup MIR and keep `ub_checks` precondition calls (+12–16% on some core fixtures); fold `intrinsics::ub_checks()` to the session constant in cleanup bodies.
- Transport verifier follow-ups: record the producer digest in the object header so stale pre-fix objects (no cleanup marker) cannot link silently; document the pipelining-timing error ("build X completely before its dependents"); mixing stale rlibs with the new sysroot ICEs in the cleanup capture path (E0460 then "should never serialize an ErrorGuaranteed") — pre-existing robustness issue of the cleanup bridge.
- Structural gate notes (pre-existing): jir-coverage's AVAIL column is 0 unless `--sysroot-args js-lane=jir` is passed (base args never run the JIR lane); fixture compile failures in jir-coverage never fail the script; lane-shadow counts the dom-tests SKIP as agreement. DOM suites cannot run (view-dom-macro absent) — diagnosis running.
- DOM suites decision (user call): `examples/dom-tests` (25 fixtures) and `examples/keyed-tests` (26) are Topcoat framework fixtures needing `view_dom_macro` (+ ~17–21k LOC of framework crates); recommendation is to retire them (delete or move to `legacy/`) and, if the legacy template lowering in `backend/src/template.rs` / `jsc-link/src/lower/js/template.rs` (3.1k LOC) stays, freeze it via pre-expanded fixtures. Loud-skip + generic macro-free fixtures (loop-created host callbacks; event-style host access) are being implemented now.
- Found by 5a: native `String` builders (`{text,cap}`) call `str_len(text)` on every push → quadratic string building; needs a byte-length field on the StringBuilder carrier (P10 item 2). `&mut str` writes through `as_bytes_mut`/`make_ascii_uppercase` are lost (documented zombie), now contained by `str_bytes_mut`.
- DOM-skip verifier limits: the plain-mode loop-callback shape rule exempts an IIFE's inner thunk from the free-variable scan (`--snapshot` covers it in the inlined build); non-shadow matrix rows still PASS on exit 0 with no count line (pre-existing); `contract/fixtures/l2-status.json` `$howToRegenerate` text is stale until a `--record` run.
- Address model limit (pre-existing, confirmed by the 5a verifier): a heap block larger than 4096 bytes gets a 4096 granule, so synthetic addresses of its elements beyond byte 4096 alias the next live block's base. Only relations are observable, but `ptr_cmp` across such blocks can be wrong. Fix: size the granule to the block (or cap element addressing) — CONTRACT.md should state it until then.
- 5a2 delivered (uncommitted, worktree agent-a8bb8d77…, under verification): native String builder `{text,cap,len}` → linear building (64 KB of 1-byte pushes 5,589 → 18.8 ms). No real workload commits a builder today (marked's String stays baseline because of `is_empty`/`reserve`), so corpus/compact are byte-identical: value depends on widening builder commitment later.
- Granule address fix (blocks >4096 bytes) in progress (worktree agent-acd9d634…; implementer resumed after a network error).
- USER ACTION: `git stash list` shows `stash@{0}` "s1b-a1ab8f94-r4-premerge-fusion" left by the 1b implementer (its `git stash drop` was denied by the permission classifier). Already applied in its worktree; safe to drop, but that is the user's call.

## Accepted limits (documented, to revisit later — user decision 2026-11-10)
Rule: a limit that needs an unsupported `unsafe` construct, or that predates the change under review, does not block a stage. It is written down here and in CONTRACT.md, verifiers list such cases separately from their verdict, and the fix is decided later. A case reachable in safe Rust is still a fail.
- Representation assumption of the no-throw analysis (SCOPE REOPENED 2026-11-10, awaiting the user: the 4'a round-2 verifier showed it can be broken WITHOUT `unsafe` in user code — a typed host binding or extern returning a value of the wrong JS type, and the `as_flattened` base miscompile — see the 4'a round-2 entry at the end): a value is assumed to have the JS representation of its Rust type. A value read through an `unsafe` reinterpreting view over storage of another element type (`align_to`, pointer casts, `transmute_copy` of a pointer) can be a BigInt where a Number is expected; an operation counted as non-throwing then raises `TypeError` and destructors on removed edges do not run. Observed: `x + 1`, `x.wrapping_add(1)` (since 3b), `x.count_ones()` and the other bit operations (with 4'a). The crash stays a crash. CONTRACT sentence arrives with 4'a. Candidate fix: "retype-guard" (abort on such a view).
- The underlying unsupported construct: a byte view over wider storage reads whole elements, not bytes (`from_raw_parts(p.cast::<u8>(), n)` over `u32` storage; `align_to::<u8>` on `[u64]`) — silent wrong values today. Same candidate fix.
- Engine-level errors (stack overflow, out of memory) inside a call proven not to throw do not run destructors on removed edges; Rust aborts there too. In CONTRACT.md since 3b.
- A `char` returned by a host function is not checked to be a scalar value; a forged surrogate breaks the string-length guarantee in every representation. In CONTRACT.md since 5a2 (integration).
- Address model: a heap block larger than 4096 bytes gets a 4096-byte granule, so addresses of its elements beyond byte 4096 can coincide with a later block's range; only comparisons between such addresses are affected. CONTRACT sentence arrives with granule-min.
- Destructor edge cases seen by the 3a verifier (identical at baseline): a destructor that throws during unwinding skips the remaining drops of that frame; a field destructor throwing on the normal path skips the sibling field's drop. Not yet in CONTRACT.md.
- Constant pooling, if 4'c lands: host values reinterpreted through an `unsafe` cast between non-erased pointee types are not modelled by the host-exposure walk.
- To check, not yet confirmed: `__rt.host_try` is reported (4'a implementer, in passing) to turn a registered abort into `Err`.
- [ ] After 4'a, 4'c and granule-min merge (all three edit CONTRACT.md): one consolidation pass so CONTRACT.md has a single "limits of the guarantees" list pointing at each of the above.

## Integration branch (`integration`, scratch worktree .claude/worktrees/integration)
- 2026-11-10 c911157 = main + fusion (cadcfb0) + 3a (9920693) + transport (e3869c3) + 3b (60eadac); only conflict was one CONTRACT.md paragraph (kept both).
- Composition run (pre-1b): compact fp.esm.min.js 208,988 / gzip 55,876 / brotli 43,548 (R0: 515,481 / 110,636 / 80,491); raw program 230,795; try 210 (R0 4,182); removed edges empty-chain 630 + no-throw 188 on top of the producer's no-op-pad removal; kernels 58/58; core suite release-balanced 141/141; cargo test 1,051/0; cleanup-smoke, admission, noop-pads, transport, active-poll, async 28/28 all pass. Known gaps 4/6 (201 lines 54–55 and aggregate_place wait for 1b).
- 2026-11-10 (later) integration 7ebb785 = previous + 5d1 (exact overflow arithmetic) + 5e (bulk byte export, incl. test follow-up) + fusion follow-up (fixture 119) + rebaselined 03_bytes_bulk extern goldens. Quick composition run: compact 209,000 min / 55,869 gzip / 43,533 brotli (try 210); IDIOMATIC 381,395 min / 97,710 gzip / 77,454 brotli, raw program 433,804, try 523 (R0 idiomatic: 822,496 / 186,452 / 135,211); kernels 58/58 on both; cleanup-smoke, noop-pads, transport, async, extern (after rebaseline) pass; fixtures 117 and 119 pass; known gaps 4/6 (201 + aggregate_place wait for 1b).
- Merge-time follow-ups: rebaseline `scripts/fixtures/cleanup-edges/*.json` (recorded pre-3a; gate fails only on the expected-JSON diff: active_poll 74 removed / try 22, 02_result 2 removed / try 0); make 3b's `host_operation_raises_nothing` delegate to 5e's `HostOperation::may_throw()`; add `cleanup` + `host-args` to `matrix.sh` pass list; wire active-poll into check.sh/CI and dedupe the noop-pads run; `had_cleanup` comment.
- Retained cleanup edges on the integrated compact build (steers the next linker step): 110 via `foreign rust_begin_unwind` + `intrinsic caller_location` (the panic path: resolve the foreign symbol to the in-program handler summary and classify `caller_location`), 57 sole host-marker (`invoke`, `callback1/0`), 43 sole host-contract-untrusted (`reflect_get`, `object_constructor_with`, `global_this`).
- Pre-existing miscompiles found by the 5e verifier (probes in scratchpad/verify5e/probe.rs; none caused by 5e): (1) `slice::from_ref(&struct.field)` reads `undefined` in all lanes (Rust sees 0 instead of the field value) — a field slot used as a slice base; (2) `from_raw_parts(p.cast::<u8>(), n)` over `u32` storage silently reads elements as bytes: `slice_from_parts` (shim ~240) drops the `sc` scale marker, so the run-time error CONTRACT.md:650/:1120 promise never fires; (3) `[[u8;0];5].as_flattened()` under release-balanced throws an ordinary TypeError from `__rt.slice_from_parts`. Assign after 1b merges (same pointer-model files).
- Granule fix (exact-size granules + buddy allocator): FAILED verification twice; PARKED, WIP on branch worktree-agent-acd9d634a1811beab (ESCALATED TO USER with recommendation to park: it costs ~+960 B min / +0.3 KB gzip per bundle to fix a rare relation — element addresses beyond byte 4096 of a block overlapping the next block). Round-2 fail reasons: `addr` reads `sc`/`es` from a user struct passed as the key (shim :303; one-line fix validated in verify-granule/fixtest2); `convert_block_nv` release forgets the base so a dangling pointer changes address (fixtest3: release like `dealloc`, keep the entry); String/builder carrier drops still abandon the block (`adapters.rs:108-135` inline `bytes_str`).
- [ ] "granule-min" — round 1 FAILED verification (2026-11-10), round 2 with the implementer: the release also runs at CALL-ARGUMENT conversions, where the value lives on in the callee, so a second live allocation gets the first one's address (safe Rust: `make` returns a Vec and stores `as_ptr() as usize`, `keep(v, a)` allocates and compares → `true`, native/base `false`; scratchpad/verify-granule-min/evidence/repro_body.rs; 5 variants). The planner does not exclude it (`dead_unaliased` + every Call result is "fresh"). Allocator bookkeeping itself passed (model fuzzer 758,031 ops, 502 refusal cases); sizes reproduced. Round 2: release only at drop conversions (carry the fact on `Convert`), call-argument path leaks as at base (documented), 5 surviving mutants, fixture 122's string half, docs, optional native-box drop release. Round-1 delivery note: Implementer's results, not yet confirmed: Vec and String/builder loops complete 3.1M rounds (base aborts near 1.04M); refused bump leaves the free lists untouched (base hands out a duplicate at 4294963216 in `natural-dup.mjs`); new `__rt.convert_block_str`; compact 209,000 → 209,019 min / 55,869 → 55,884 gzip / 43,533 → 43,493 brotli (program identical); workloads +0…+177 B, some through planner flips because helpers are priced by source bytes; no timing regression vs the A/A control. Open risk it names (not observed): a call-argument conversion of an address-observed value the callee keeps alive now frees the old address. Host-string adapter never committed by today's planner (unit tests only). Launch note: on integration 2f36c3f (Opus 5.5 xhigh implementer; scope: release on conversion incl. a string helper, the refused-bump double hand-out, the CONTRACT limit; the parked buddy allocator must not be revived). Original definition: keep 5a's allocator; (1) release a heap block's granule when a conversion consumes it (`convert_block_nv`, and a `convert_block_str` for the host-string/builder paths), keeping the address entry as `dealloc` does — today on main every Vec/String dropped through a native carrier leaks 4096 bytes of address space forever (1,044,274 such drops abort); (2) fix the latent 5a double hand-out: a refused bump had already pushed its gap granules, so a retry could hand out one address twice; (3) CONTRACT: state the >4096-byte-block limit. Probes: verify-granule/probe (`heap_cycle`, `heap_cycle3`, `dangle`, `dangle2`, `str_cycle2`, `natural-dup.mjs`).
- 5e: merged into integration (see Stage 5). Residuals noted by the verifier: the abort path itself (`new Error`, `WeakSet.prototype.add`, `Symbol.for`) is not captured against later user patching — true of every runtime abort; typed-storage export path still consults `constructor[@@species]` (reachable only through a lying declaration).
- Extern gate on the JIR lane: `JS_EXTRA_ARGS=js-lane=jir bash scripts/extern-test.sh` fails the golden compare for 01_chart/02_globals (goldens last touched in the reconstruction commit; stale). `scripts/build_sysroot.sh` breaks when `CARGO_TARGET_DIR` is exported and the key is uncached (cargo writes outside the directory the script installs from).
- 5a2 (StringBuilder `len`): first verification FAILED on the surrogate junction (host pieces `"\\uD83D"` + `"\\uDE00"` join into one astral char: cached len 6, text encodes to 4 → unchecked reads past the byte view). Round 2: replace the junction units with U+FFFD so the builder matches the baseline representation. Everything else verified (48/48 fuzz programs vs native).
- P12 lead from the 5a2 verifier: format sites NEVER commit a native carrier today — `format!`→builder and inline `write!` are declined with "JIR-R Move reads vN as HostString but writes vM as Baseline; use Convert" (`plan/representation.rs:2690` marks only call results; check at `rcgjs-object/src/jir/repr.rs:1181`); `native-format-args` and `native-int-string` receipts fail on every linker. A scratch planner patch that lets the mark follow the `Move` rename makes them commit (scratchpad/verify5a2/saved/fmt-commit-scratch.patch). This is very likely the historical "formatter proof/commitment repair" (idiomatic OSS 65.5 → 48.8 KB gzip). Also: builders never commit under release-balanced (plan proposes them, exact measurement returns INF).
- Baseline-lane String bugs seen by the 5a2 verifier (identical on the base linker): `retain` wrong content, `replace_range` panic, `insert_str` compile refusal.
- Known gap (pre-existing, both lanes): `transmute::<&P, *const P>` yields the bare object and later crashes (JIR `unsize.rs::transmute_scalar` pointer arm; baseline `backend/src/intrinsics.rs:1871-1880`). The local rule (box on reference→raw transmute) was tried and reverted: coroutine resume functions transmute `&mut Context` into `ResumeTy` and read it back with a plain copy, so boxing breaks 26_async/31_host/111/112. Needs a paired raw→reference step at the `ResumeTy`/`get_context` read, or recognising the async-context transmute as identity; the reverse direction `transmute::<*const P, &P>` is untested. Rejected diff: scratchpad/fusefix/transmute-attempt.diff.
- P12 refinement (5a2 implementer's assessment, 2026-11-10): the format-commit planner bug is real (`commit_func` marks only call results while `prove_site` proves a whole Move/Copy spine, so the IR verifier rejects every committed format site), but fixing it changes NOTHING on the idiomatic FingerprintJS port (A/B byte-identical): every String region there is pinned to the baseline representation by unsupported operations (`as_bytes`, `reserve`, `from_utf8_unchecked`, `as_mut_str`, `spec_write_fmt`), must-equal-baseline boundaries and `format_inner` as an unknown call. So the formatter win needs String legality widened first (builder support for those operations / the fmt kernel recognising `format_inner`), then the spine-marking fix. A safe spine fix: carry the proven spine in `Site`, mark exactly those values, keep display marks `HostString`, re-assert single use.
- 5a2 round 2 delivered (each appended piece sanitized by `__rt.native_str_wf`, so builder text is always well formed and equals the baseline representation); under re-verification.
- 5a2 round 2 FAILED verification (2026-11-10, second fail; one narrow, latent reason — the round-1 reason is fixed): the format kernel sanitizes after concatenating, so two host `&str` arguments side by side in one `write!`/`format!` pair up (`fmt_kernel.rs:869`, `fmt.rs:205` emit the raw string; `native_str_wf` wraps the whole text at `fmt_kernel.rs:770,803`, `fmt.rs:122,127`): `"\uD83D"` + `"\uDE00"` give one emoji (4 bytes) where baseline gives two U+FFFD (6 bytes). Internally consistent, but representation-dependent. Reachable only with the scratch planner patch (format sites never commit today). No committed test covers the kernel (3 kernel mutants + the planner member line survive). Everything else passed (1,034 scenario lines, 409,653 strings vs `toWellFormed()`, 48-program fuzz, 19/19 shim mutants, gates). Handled as a split, not a loop: one tightly scoped final round ("5a2-fmt": sanitize each `Str` display argument, three kernel tests, pin the planner line, docs) sent to the same implementer; a further fail parks the item. USER INFORMED (second fail). Sequencing rule from the verifier: the P12 planner fix must not land before this.
- 2026-11-10 integration b6ab45c = 7ebb785 + overflow follow-up 45edc42. Composition check passed (scratchpad/integration/int3): build ok, cargo test 1063 passed / 0 failed, extern 5/5, fixtures 117 and 119 pass, compact unchanged at 209,000 min / 55,869 gzip / 43,533 brotli (try 210), kernels 58/58. Dry run: the 4'a and 4'b diffs apply cleanly on b6ab45c.
- 5a2 round 3 VERIFIED (pass-with-limits, 2026-11-10), committed on its branch as 57375cf and merged into `integration` (2f36c3f). The format kernel sanitizes each host `&str` argument where it is read; adjacent halves equal baseline on 730 adjacency lines + the round-2 repro; 6 kernel mutants killed; fuzz 48 programs match native; corpus and compact byte-identical to 5a (the builder still commits in no real workload). Limits: (1) the format kernel is still dead code until the planner spine fix (verified only through the verifier's scratch planner patch, scratchpad/verify5a2); (2) the planner member line is unpinned (only feeds a reported number; pin it with the planner fix); (3) kernel tests drive only `Str` and integer arguments; (4) release-balanced suite and cleanup gates not re-run on round 3 (running in the integration composition check, scratchpad/integration/int4).
- Known gap (5a2 verifier): a `char` returned by a host function is not validated as a scalar value; a forged surrogate (`0xD83D`) pushed twice gives a builder `len` 6 over a 4-byte text (verify aborts) and the baseline is incoherent too (`len()` 6, `as_str().len()` 18). Pre-existing, outside the value model; CONTRACT.md now says so. Fix would be a check on the extern `char` return path.
- [ ] "3b-gate" (from the 4'a verifier, pre-existing since 3b; do after 4'a merges, same files): the independent re-derivation does not actually gate edge removal. `cleanup::run` (`lane.rs:1363`) uses the summaries before `summary_check::enforce_with` (`lane.rs:1505`) validates them, and the witness accepts `ProtectedOp::Rust` and `Foreign{symbol: Some}` unconditionally (`witness.rs:354,357`); a release linker runs witnesses only under `RCGJS_WITNESS_CHECK`. So a wrong `may_throw = false` reaching an edge through a callee's summary is removed and never restored (three planted-lie linkers: edge removed, `summaries.validated` 6/7, 68/72, 25/27, destructor skipped; nothing reads `summaries.validated`). In the compact package all 127 new-class removals have a named Rust callee as the protected op, so the witness lists decide none of them. Fix: removal may use only a bit the independent derivation agrees with at that point (or restore on later disagreement); acceptance = the planted-lie linkers in scratchpad/verify4a/mut2 keep the destructor without the env var.
- 2026-11-10 integration 2f36c3f = b6ab45c + 5a2 (57375cf). Full composition check passed (scratchpad/integration/int4): build ok, cargo test 1067 passed / 0 failed, runtime node tests pass, extern pass, cleanup-smoke pass, release-balanced core suite 143/143, fixtures 117/119 pass, known gaps 4/6 as expected (201 + aggregate_place OPEN), compact unchanged 209,000 min / 55,869 gzip / 43,533 brotli (try 210), kernels 58/58.
- [ ] 1c BASE MISCOMPILE — DELIVERED 2026-11-10 (uncommitted on worktree agent-aa0340f7… at 2f36c3f), UNDER VERIFICATION (Fable 5.2 high, scratchpad/verify1c). Two causes: (A) `lower/fuse.rs` inlined any single-use value, including one that is WRITTEN (base of `SetField`/`SetDiscriminant`, root of a mutable view), so the write went to whatever the inline expression named (the source element, or an aggregate's operand: `q.x = 99` became `n = 99`); fix: `uses_as_storage()` keeps such a value declared. (B) `lower/ssa.rs` lowered a copy of a no-clone type (transparent scalar wrapper) to the source SSA value, so `let mut b = a; b.0 += 1` changed `a` too (9009 vs 8009, not only dead copies); fix: give the local its own value before a projected write. Extent, 667 generated cases: wrong at base 249 (release-balanced), 249 (jir opt on), 282 (jir opt off), 6 (default), → 6 everywhere with the fix; the 6 are heap arrays (`Vec<[u32;3]>` abort, `Box<[u32;3]>` reads undefined; all lanes, = 1f family). Cost: suite +990 B / 3.25 MB, compact 209,000 → 209,006 min, workloads +0…+127 B, no timing change. Fixtures 123, 124. Original entry (found by the 4'c verifier; launched on integration 2f36c3f): a by-value copy of a scalar-only struct out of an array/slice element that is then written and never read is emitted as an alias (`e=f[g+b];e.x=99;`), so the write lands in the source: `let mut p = PS[4]; p.x = 99;` makes `PS[4].x` read 99 (native 4). Wrong at base for a static, a local array and a slice parameter (scratchpad/verify4c/probe/src/r4.rs); MIR is correct; survives every JIR pass off. Not reproduced for tuples, arrays, nested aggregates, or when the copy is read after the write. Blocks 4'c (pooling extends it to constants).
- Chunk bug (pre-existing, found by the 4'c verifier; assigned to 4'c round 2): per-chunk `order` re-emits an item another chunk owns when a chunk-owned static refers to it → `SyntaxError: Identifier already declared` (scratchpad/verify4c/probe/src/c3.rs at base).
- 4'a round 2 delivered (2026-11-10), under narrow re-verification: `compare_bytes` and dead `fabs*` removed from all three lists, three new tests (mixed-buffer trace, forced removal refused, the three lists agree), docs; package 200,050 min / 53,517 gzip / 41,922 brotli, try 99, kernels 58/58 (matches the verifier's own build).
- USER DECISION — DECIDED 2026-11-10: accept and document now, think about the fix later ("we should document such cases (and similar to this one) and later we will think how to deal with them"). See "Accepted limits" below. Original entry (scoping of the fail-closed rule; surfaced 2026-11-10): the no-throw analysis assumes a value has the representation of its Rust type. A value read through an `unsafe` reinterpreting view over storage of another element type (already a recorded unsupported construct: the view reads whole elements) can be a BigInt where a Number is expected, and then an operation counted as non-throwing raises `TypeError` and the caller's destructors on removed edges do not run. Observed (scratchpad/s4a/item6): `x + 1` and `x.wrapping_add(1)` skip the destructor already at base (Stage 3b counts `Binary` ops as non-throwing; on `integration` since 60eadac, not on main); `x.count_ones()` only with 4'a's bit-operation audit. Orchestrator's working assumption for the 4'a re-verification: outside the guarantee, documented in CONTRACT.md, provided every instance needs `unsafe` reinterpretation. Alternatives: (strict) treat every arithmetic op on a value that may come from a load as may-throw, which gives back most of 3b; (root fix, recommended follow-up) make a reinterpreting view a registered abort at creation or read, like the existing `guard_zeroed_retype` guard, which also closes the recorded "byte view over wider storage silently reads elements" gap.
- [ ] Follow-up candidate "retype-guard": abort (registered) on a view whose element type differs from the storage's (`align_to`, `cast` + `from_raw_parts`, pointer-to-pointer casts, `transmute_copy` of a pointer), instead of silently reading elements.
- USER DECISION (2026-11-10): an ES2018/ES2020 language and API level is an acceptable baseline for compiled output ("most browsers support it out of the box"); it may or may not help bundle size; earlier experiments were mixed (some newer APIs were slower than the hand-written code), so any API swap is benchmarked before it is adopted. Not urgent.
- [ ] "ES2020 target" (queued, measurement first). State today: release profiles use `js-ecma-target=2018`, a SYNTAX ceiling only (BigInt constants printed as `BigInt("…")`; runtime already needs `BigInt`, `globalThis`, `queueMicrotask`, `TextEncoder`); the package tools accept `--ecma-target 2018|latest` but hard-code Terser `ecma: 2018`; `scripts/es2018-gate.mjs` enforces the ceiling. Measured on integration 2f36c3f compact with the existing `--ecma-target latest`: 209,000 → 208,964 min, 55,869 → 55,856 gzip, 43,533 → 43,412 brotli (only 4 of 15 BigInt constants become literals; no `??`, `?.` or `catch{}` appear), kernels pass. So today's switch buys almost nothing; a win needs the compiler and shim to use newer constructs. Candidates to measure on all four targets: Terser `ecma: 2020`; BigInt literals everywhere (64-bit-heavy code: hashing, idiomatic and Pro builds); `??`/`?.` in emitted option/null checks and in the shim; optional catch binding where the binding is unused; newer built-ins in the shim (each one benchmarked against the current hand-written path).
- Base bugs reported in passing by the granule-min implementer (to be confirmed by its verifier, then given their own fix): (1) `__rt.addr` reads user-struct fields named `buf`, `off`, `sc`, `es`, `pr`, `length` — a struct with such field names is mistaken for a pointer record; a `field_names` probe differs from native in 27 of 121 lines at base (safe Rust, so a real bug, not an accepted limit); (2) `String::as_ptr` reports a memoized byte view's address, so two live Strings with equal text share one address.
- Execution model change (user, 2026-11-10): new verifiers are Fable 5.2 HIGH. The three verifiers already running at xhigh (4'a round 2, 4'b, granule-min) finish as they are.
- 4'a round 2 FAILED re-verification (2026-11-10, SECOND FAIL → ESCALATED TO USER; 4'a stays uncommitted). The `compare_bytes` fix is complete, all 31 requested mutants are killed, gates pass, numbers reproduce (200,050 min / 53,517 gzip / 41,922 brotli, try 99). Fail reasons (my condition "every instance needs `unsafe`" is false):
  1. Host value in a number position, safe Rust: a typed `js_bindings!` getter (or a print-shim `extern "C"` declared to return `u32`/`u64`/`f64`/`usize`) returns whatever the host holds, unchecked (`CONTRACT.md:2685` "nothing is marshalled"; `JsValue::read::<T>()` is checked, typed bindings are not). A plain number where `u64` is declared → `count_ones`/`rotate_left` raise `TypeError` and, with 4'a, the destructor is skipped (base runs it). For a `+`/`>>` callee the destructor is ALREADY skipped at base (3b). With `-> u32`: a BigInt, a Symbol, or an object whose `valueOf` runs host code inside `count_ones`. Repro: scratchpad/verify4a/r2/repro/repro_host_u64.rs.
  2. `as_flattened` (safe Rust, BASE MISCOMPILE → item 1d): `__rt.unwindow` (shim ~1110) steps into one element, so a flattened `&[[u64; 2]]` covers only the first row; later elements are `undefined` (u64/i64/u128 → `TypeError`) or `0` (f64, `[[u8;4]]`, `[[u32;2]]` → SILENT WRONG VALUE). With 4'a's foreign rule the bounds-check panic path no longer keeps the edge, so the destructor is skipped where base ran it. Repro: scratchpad/verify4a/r2/repro/repro_as_flattened.rs.
  Also: the new CONTRACT sentence is true but incomplete (float math `sqrt`/`floor`/`powi` is newly affected too; it names only reinterpreting views). The three-lists test can still miss a name in 4 ways (a quiet rule placed in the publisher's transfer `throw_cause`; a stray `"` in a doc comment truncating the enumeration; a name arriving through a constant from a seventh file; a consistent wrong audit in all three lists is inherent).
  Root of the matter: the assumption is the foundation of 3b's `Binary` rule (on `integration`), not specific to 4'a; 4'a's foreign rule and intrinsic audit widen which functions and operations rely on it. Candidate root fix: "typed boundary checks" — coerce or validate every typed value where it enters from the host (binding results, extern results, callback arguments, export parameters), so a mismatch throws at the host call, which is always protected; cost not yet measured.
- [ ] 1d BASE MISCOMPILE `as_flattened` — DELIVERED 2026-11-10 (uncommitted on worktree agent-aeaee6e0… at 2f36c3f), NOT YET VERIFIED (pause: no verifier spawned). Root causes: (1) `__rt.unwindow` (shim:1110) turned a pointer into a buffer of rows into a pointer into row 0 only (arrays of arrays are always nested JS arrays) → reads past row 0 gave `undefined`/`0`/`TypeError`, writes through `as_flattened_mut` were lost, raw `ptr.add` across rows broke; (2) baseline lane `backend/src/ptr.rs:1095` `build_fat` did not peel transparent wrappers, so `vec![[9u32,8],[7,6]].into_boxed_slice()` gave a slice of scalars (silent wrong values in the DEFAULT lane without `as_flattened`). Fix: a `Proxy`-based flattened view (`_flat_view`) with address/`ptr_eq`/`ptr_cmp`/`chunk_slice` support, `build_fat` peels; fixtures 125, 126, 11 node tests; implementer reports every tested family correct in all six configurations and all gates passing. COST (decision needed): +2,166 min / +605 gzip / +475 brotli on the compact package (211,166 / 56,474 / 44,008) and +359…+2,369 B on eight workloads, all runtime text, although the three timed workloads never take the new paths; reads through the view cost ~0.7 µs per element. Timing inconclusive (load 137–172). Limits it names (base too): dropping a heap block of rows aborts (`vec![[..]]`, `Box<[[T;N]]>`); `Vec<[T;N]>` grown by `push` is stored flat; a row's own address differs from the flattened element address; `swap_with_slice`, array `into_iter()` of arrays (JIR), `MaybeUninit::write` on an array element fail. Original entry (launched on integration 2f36c3f): make it correct, or fail closed with a registered abort where a correct view cannot be built; includes the older `[[u8;0];5].as_flattened()` TypeError gap.
- "3b-gate" plan evaluated by the 4'a verifier (ran it in a copy): calling `summary_check::enforce_with` right after `compute_with` catches summary lies (`exact_div`, `try`) at zero cost on three probes (0 demotions, byte-identical output), but does NOT stop a wrong classification of the protected operation itself (the foreign lie; directly protected host operations, where both derivations read one table). To close: check the protected operation per edge against the enforced summaries, run the cleanup-edge witness in release builds, add early violations to `summaries.validated`, and have a gate assert it equals `items`.
- Base bugs CONFIRMED by the granule-min verifier (identical at base 2f36c3f; smallest programs in scratchpad/verify-granule-min/probe/src). Reachable in safe Rust, each needs its own fix:
  - [ ] 1e `__rt.addr` reads user-struct fields: `struct P { pr: u32 }` → `(&p as *const P).addr() == 0` is `true` (native `false`); `O { off: 5 }` gives a misaligned `addr()` that differs from `as usize` (`fld2_body.rs`; 27 of 121 probe lines differ). Fix after 1b-min reports (same shim function): pointer records must not be recognised by property names.
  - [ ] 1f `Box<[u32; 8]>` indexing prints `undefined` for `x[0]` in the DEFAULT profile (native 8) (`box1_body.rs`); found in passing, not investigated.
  - `String::as_ptr`: two live Strings with equal text share one address (`a.as_ptr() == b.as_ptr()` true), and `a.as_ptr() as usize != a.as_bytes().as_ptr() as usize` (`sas_body.rs`), both profiles.
  - A Vec's address changes across a conversion round trip (`var_ret`), release-balanced.
  - Native-box drop glue never releases the block's granule (`drop.rs:70-118` omits it on purpose): a loop of `Box<[u32; 2000]>` aborts near 1.05M rounds under release-balanced (offered to granule-min round 2 as an optional item).
  In `unsafe` Rust with defined behaviour: a raw pointer kept across a by-value move into a native-carrier callee no longer aliases the Vec (`alias`: prints 7099, native 99042) — the conversion legality ignores that the pointer escaped.
- Limit to document (base too): addresses of references to aggregates, stack arrays and string byte views come back only through the FinalizationRegistry, which cannot run inside a synchronous loop; a loop taking `&*b as *const _ as usize` on a large box aborts after 1,048,572 rounds in the default profile.
- Planner sensitivity (granule-min verifier): representation and flat-ABI decisions flip with the shim's byte count, because helpers are priced by their source text against compressed-size budgets. A 44-byte shim change declined idb's whole representation plan on the brotli budget (16,133 → 16,240) and un-flattened `<[T]>::get(Range)` in minisearch's `CharSearcher` loop (now builds `{start,end}` per iteration). No measurable timing change, but decisions that put allocations into loops should not hinge on unrelated shim text. Candidate for the "2e" policy work.
- [ ] 1g BASE MISCOMPILE — DELIVERED 2026-11-11 (uncommitted on worktree agent-ae6fe0b2… at 2f36c3f), NOT YET VERIFIED (pause: no verifier spawned). Root cause: the finished-JS scalar replacement (`rcgjs-object/src/sroa.rs`: `collect_whole_assignment` 1581–1587, `split_shapes` 1230–1294, `split_assignment` 1877–1886) rewrote a whole-object assignment `x = y` as per-field copies, but `x = y` makes `x` another NAME for the object (that is how a `&mut S` is spelled); in-place field stores were recorded like reads, so `s.f = v` went to `s$f` and `a$f` kept the old value. Not the inliner: REACHABLE WITHOUT ANY CALL (`let p = if c { &mut a } else { &mut b }; p.f = 254;`, `match`, a loop cursor, a reborrow on two paths). Fix (sroa.rs only): a component in which a local on a copy edge also stores a field is not split. Extent, 660 generated functions vs native: 511/660 right at base in every opt-on configuration (release-balanced/size/speed, jir opt on ± cleanup) → 660/660; opt off was always right. Wrong at base also for non-`Copy` structs, enums, u16…i128 newtypes, bool, char (wider than first read). Cost: every existing fixture, the compact package and all ten workloads BYTE-IDENTICAL; +0.4–0.8% only on probe programs dense with the pattern. Fixtures 130, 131 (fail at base under release-balanced). With the widened inliner (4'b) over the fix: all five verifier repros right, 570/660 → 660/660. Limit: the rule is conservative (a store anywhere in the component keeps the objects), which the wider inliner will hit often; a flow-sensitive refinement is possible. Same broken invariant as 1c in the opposite direction (`x = y` always means the same object). Original entry (found by the 4'b verifier; launched on integration 2f36c3f): a write through `&mut` is lost after a small helper is inlined. `#[derive(Clone, Copy)] struct S { f: u8 }` assigned on more than one path, then `set(&mut a, 254)` with `fn set(s: &mut S, v: u8) { s.f = v }` → prints 7 (native 254). The 12-line core is wrong with the base linker under release-balanced (scratchpad/verify4b/repro/inline_lost_write_core.rs); also a tuple newtype (10 vs 16), a `bool` flag, a `u64` wrapper. Lead: the inliner binds the `&mut S` parameter by an assignment (`_7 = a; s = _7`) and the finished-JS scalar replacement (`rcgjs-object/src/sroa.rs` `split_shapes`) treats it as a copy; with the `sroa` or the `inline` pass off it is right. Blocks 4'b.
- Other base bugs seen by the 4'b verifier (wrong with both linkers; scratchpad/verify4b/dt/hand): `#[track_caller]` function calling another one, or called through a function pointer, loses `$loc` on the JIR lane and throws (default lane right) → [ ] 1h; reading a struct element of `Box<[P; 2]>` throws `TypeError` in every configuration, default lane included (same family as 1f); the known alias miscompile (1c) reproduces on the JIR lane even with `js-opt=off`.
- 4'c round 2 delivered (2026-11-10), under verification by a fresh Fable 5.2 HIGH verifier (scratch: scratchpad/verify4c-r2). Implementer's claims: every `HostPayload` kind is a full host boundary; C-ABI `__rt.<symbol>` calls are host calls unless the symbol is a shim member; no marker signature is trusted; the walk is directional and treats non-`Freeze` data behind a shared reference as written (`TypeMeta::freeze`, JIR schema 15 → 16); `link::order` places only the items it is given (fixes the chunk double emission, which also failed without pooling); `RCGJS_POOL_FAULT` is debug-only; the manifest version is checked before decoding; all schema pins updated (sidecar 18, JIR 16, manifest 6). Compact unchanged from round 1: 187,664 min / 51,802 gzip / 43,385 brotli, kernels 58/58; corpus byte-identical on/off (five async workloads now have an unbounded footprint and decline everything — fails closed, costs nothing today). ONE KNOWN FAILING CHECK: the `r3` copy shape in fixture 121 (prints 99/371, native 4/276), waiting for base fix 1c; after 1c merges, tell the implementer to merge and the verifier to re-check that item. Not done: `PoolScope` thread-local stays (a `JsCx` field means touching ~17 `LowerInput` construction sites in 9 files).
- PAUSE (user, 2026-11-10): "let's pause and not spawn a new work after this batch is finished". The agents already running finish; no new agents are spawned after that: no queued items (1e, 1f, 1h, 3b-gate, 4'd–4'f, ES2020, runtime work), no further rounds, and no new verifiers unless the user says so. Reports that arrive are recorded here; a verdict of pass / pass-with-limits is still committed on its branch and merged into `integration` by the orchestrator (no agent needed). Running at the time: verifiers for 4'c round 2 and 1c; implementers for 4'b round 2, granule-min round 2, 1b-min, 1d, 1g.
- 1c round 1 FAILED verification (2026-11-11; NOT sent back: pause). The fix itself held: extent re-run 249/249/282 wrong at base → 6 everywhere (the heap-array cases, identical at base), zero regressions (145 fixtures × 3 configs, 900 random differential runs), Fix B and every mutable-write shape of Fix A hold, gates pass, costs as claimed (suite +1,058 B; compact 209,006 / 55,875 / 43,481). Fail reason, same cause as Cause A, wrong at base too: a store through a SHARED view into a single-use aggregate (interior mutability) once `Cell::set` is inlined — `let q = C { a: Cell::new(n), b: m }; q.a.set(99); n*1000+m` prints 99007 (native 5007), and with a slice element or a static as the operand it overwrites the caller's array / the static. Reproduces whenever `JSC_RUST_MIR_INLINE=on`, which the PACKAGE AND WORKLOAD BUILDERS turn on (the implementer's five configurations never did). Where: `lower/fuse.rs:1615-1630` `uses_as_storage` matches only `mutable: true` views ("a shared view only reads" is false for interior mutability). The verifier's diagnostic (any mutability) fixes the counterexample in all three failing configurations but changes the expected text of `a_raw_view_boxes_a_held_object_until_a_step_reaches_storage`, so it is not a drop-in. Also: the `SetDiscriminant` arm is pinned by no test (mutant survives; no reachable behaviour change found). Evidence: scratchpad/verify1c (`cex/cex_cell.rs`, `atk/src/r_cell.rs` 9 of 12 variants). Round 2 when work resumes: guard shared views whose pointee can be written (interior mutability), add MIR-inline configurations to the fixtures, pin the discriminant arm.
- Verifier side notes from 1c (base too): `&raw const` of a field cast to `*mut` and written reaches the source / the operand in every configuration incl. the default profile (definedness contested); gates and extent matrices should include `JSC_RUST_MIR_INLINE=on` and `JSC_RUST_OPT_LEVEL=z`, since that is what the package and workload builds actually use.
- 4'c round 2 FAILED re-verification (2026-11-11, SECOND FAIL → ESCALATED TO USER; not sent back; stays uncommitted). Every round-1 finding is fixed and re-checked (map, C-ABI, erase marker, `Cell` export, chunk double emission incl. three chunks, schema pins, fault knob absent from release binaries, version message); pooling-off output is byte-identical to base for all fixtures; compact 187,664 / 51,802 / 43,385 reproduced, kernels 58/58. Host isolation is still not fail-closed, four safe-Rust shapes with `host_exposed: 0` (evidence scratchpad/verify4c-r2/probe, `hc.sh <name>`):
  F1 a `&primitive` field inside pooled data (`Opt { default: &'static u32 }`; export returns `OPTS[i].default`; host writes `p.buf[p.off]`) — `pool.rs:513-517` `Footprint::reaches` compares host slots only with candidate objects/places (`f08`, `f09`, `f18`);
  F2 an array reference taken from a pooled slice (`TABLE.first_chunk::<4>()`, `try_into`, `array::from_ref`) is an aliasing view treated as an unrelated object — `pool.rs:931-933, 978-984` (`f04`–`f07`, `f14`, `f16`);
  F3 transparent newtypes: slots/places recorded unpeeled never meet the peeled objects — `pool.rs:914-917` vs `978-984`, `1008` (`f03`, `f15`);
  F4 shim members `js_log_*` hand their argument to `console.log`, but every shim member is classified internal — `pool.rs:759-765` (`s1`, `s3`; needs a mistyped declaration and a host hooking `console.log`).
  Also: `&Vec<Cell<Option<&'static [Rule]>>>` export parameter reaches the erased-pointer residual from safe code (no working exploit); 2 behaviour-changing mutants survive (closure-marker result / future-helper result not handed); the known 1c failure has FOUR shapes, not one (`match PS[11] { mut p => … }`, `split_first` then `*first`, `chunks(5)` loop) — re-check all after 1c; the round-1 probes were vacuous as first run (the shared prelude's panic handler makes the footprint unbounded). Design question for the user: keep refining the footprint walk (third round), or switch to a rule that cannot leak (e.g. pool only data the host provably never receives a pointer INTO at any depth by type, or deep-freeze pooled objects so a host write cannot change them).
- New base bugs from the 4'c round-2 verifier (same at base; scratchpad/verify4c-r2/probe/src2): [ ] 1i a niche `Some(..)` inside constant data reads as `None` (`o1.rs`: native `7 9 100 5 true 11`, mires `0 0 100 0 true 11`) — silent wrong answer in safe Rust; cyclic statics (`static A` ↔ `static B`) hit a TDZ `ReferenceError` (`c4`); the JIR schema number is mixed into the normalized body hash (`summary.rs:915`), so a schema bump changes which text-identical duplicate survives link dedup (no printed program changed).
- granule-min round 2 DELIVERED (2026-11-11; uncommitted on worktree agent-ac96e6c4… at 2f36c3f), NOT YET VERIFIED (pause). Release now happens only at the conversion in front of a drop (`OpKind::Convert` gains `at_drop: bool`; new shim members `drop_block_nv`/`drop_block_str`; call-argument conversions spell exactly what base spells). Implementer's results: reproducer + 5 variants match native and base again; drop-path loops complete 3.1M rounds; call-argument loops abort near 1.05M as at base (documented in CONTRACT.md; real fix = the address moves with the value into the native carrier); all requested mutants killed (incl. M2b, M2c, M2d, M5c, M6a); gates pass (suite 146/146, runtime tests 77/77, ES2018); `_addr_mint` back to base length (refusal fix is a reorder); compact 209,001 / 55,868 / 43,471 (program identical); timed workloads' programs identical to base; idb +124 B and idb_lines +125 B (helper rename; idb's plan no longer declined). Planner flips remain in 9 address-taking core fixtures (two put allocation sites into loops: 21_vec +2, 72_repr_format_min +5) although those programs see only a same-length reorder of `_addr_mint` — the compressed-size scoring moves with shim text. Not done: Box drop leak (needs native-box drop glue + helper pricing; root cause: `NonNull::new`'s null test and alignment ub-checks mint an address for nearly every allocation). MERGE NOTE: its new fixtures are numbered 123 and 124, which COLLIDE with 1c's fixtures 123/124 — renumber one set at merge time. `Convert` payload changed without a JIR schema bump (JIR-R is never stored in objects; decide at merge whether to bump anyway).
- 4'b round 2 DELIVERED (2026-11-11; uncommitted on worktree agent-ab531332…, now on 2f36c3f), NOT YET VERIFIED (pause); blocked on 1g (its four new lost-write fixtures fail until 1g merges; release-balanced suite 145/149, exactly those four). Implementer's results vs a base linker at 2f36c3f: compact 230,809 raw / 209,000 min / 55,869 gzip / 43,533 brotli → 219,292 / 204,858 / 55,781 / 43,291 (634 sites); idiomatic 433,804 / 381,395 / 97,710 / 77,454 → 413,987 / 372,861 / 96,957 / 77,062 (949 sites); kernels 58/58; corpus total min 566,068 → 546,283 (−3.5%), gzip −2.5% (fflate −12.0% min, marked −9.2%); suite: 0 fixtures larger, 85 smaller, −9.52%. The three surviving `lane.rs` mutants and the `splice` argument swap are now killed (oracle moved into `opt::inline::Declarations`; new fixture + script checks). Probe cap: phase 1 uncapped, `REPROBE_CAP = 256` only for phase-2 re-probes (the verifier's >256-site program now inlines 16 sites vs base 15). Representation flips re-examined against the new base: `29_rc` and `idb` identical to base; only `101_btree_map_nodes` flips (1 → 0 regions) on a ±11 B brotli sign in the solver's zero-growth gate — the same compressed-size sensitivity. Open, documented not fixed: the 384 B floor lets loop-virtualization sites grow a program with no benefit (`o_a` +303 B, in-loop allocation sites 59 → 65); proposed rule: a growing loop site must show no new in-loop allocation in its probe. MERGE NOTE: its fixtures are numbered 118 and 121–125, which COLLIDE with 4'c (121), granule-min (122–124), 1c (123–124) and 1d (125–126) — renumber at merge time.
- 1b-min DELIVERED (2026-11-11; uncommitted on worktree agent-ae104b5a… at 7ebb785 — one merge behind integration 2f36c3f), NOT YET VERIFIED (pause). Done: (item 1) `copy`/`copy_nonoverlapping` in the shim now rebind pointer-valued elements instead of overwriting the pointee in place (`ptr::swap` → `slice::swap`, sorts, `reverse` on arrays of `&P`, `*const P`, `Box<P>`, `Option<&P>` copied one pointee over the other); (item 3) baseline-lane `ref_to_raw` (`backend/src/ptr.rs:1022-1044`) keeps a slot only for `p.buf[p.off]`, every other reference becomes `__rt.box(reference)`. Not ported (need the full 1b model): the borrowed-field write-through (item 2) and `r5_dyn` (item 4). Implementer's results over 67 probe fixtures × 5 modes: wrong lines 2,493 → 2,001, 492 fixed, 0 regressions; known gaps 5/6 (`201_reference_field_pointers` now PASS, `audit_aggregate_place` stays OPEN); suite 144/144 release-balanced; pointer-element swaps 4× faster, integer bulk copy 6× faster in micro-benchmarks, corpus within noise; compact 209,000 → 209,289 min / 55,869 → 55,946 gzip (shim +399 B raw), kernels 58/58. Fixture 122 (COLLIDES with other pending fixtures numbered 122) + `scripts/runtime-moves.test.mjs`. The list of what is still wrong at base for the storage-identity model is in its report (scratchpad/1bmin/logs/table-base-vs-final3.txt).
- BATCH FINISHED 2026-11-11: no agent is running. State of every open item:
  - verified and merged on `integration` (2f36c3f): fusion fix, 3a, transport, 3b, 5d1 (+follow-up), 5e, 5a2. Not on main.
  - delivered, awaiting a verifier: 1d (as_flattened; +2.2 KB runtime decision), 1g (scalar-replacement alias; blocks 4'b), granule-min round 2, 4'b round 2, 1b-min.
  - needs another implementer round: 1c (shared-view stores under MIR inlining; blocks 4'c).
  - waiting on a user decision: 4'a (host values that do not match their declared type), 4'c (how to make host isolation unable to leak).
  - queued, not started: 1e (addr reads user field names), 1f (heap arrays / Box<[T;N]>), 1h (track_caller chains), 1i (niche Some in constants reads None), 3b-gate, retype-guard, typed boundary checks, String::as_ptr, Box address leak, 4'd–4'f, inline hints, ES2020 target, runtime work (5b, 5c, P12), CONTRACT consolidation, integration → main merge + R1.
  - merge-time housekeeping: fixture numbers 121–126 collide across 4'b, 4'c, granule-min, 1c, 1d, 1b-min; three branches bump schema versions (4'c: JIR 16, manifest 6, sidecar 18).


## Current promotion addendum — 2026-11-11 (supersedes older merge-status notes)

- [x] Preserve refs, pending patches, untracked fixtures and decisive verifier evidence before integration.
- [x] Select verified fusion, no-op landing-pad, cross-crate cleanup transport, overflow, bulk-byte and builder work; do not merge integration wholesale.
- [x] Review and execute the delivered minimal pointer-move fix; use fixture 202; original worktree unchanged.
- [x] Compare adversarial probes against freshly rebuilt old main. Treat the newly exposed lost write as an integration regression, not a waived base bug.
- [x] Review and execute the delivered 1g alias-store fix, native fixtures and bounded generated extent. Wider inlining remains unpromoted.
- [x] Run selected-candidate workspace/runtime/release fixtures, cleanup/transport/extern/standalone checks, and fresh OSS/corpus/optimized-Pro builds. Retain failures and negative controls.
- [x] Preserve five public round-1 reproducers and native expectations with a strict runner.
- [x] Write round_1_integration_report.md and docs/round1-promotion-verification.json.
- [ ] Repair the four remaining round-1 cases and aggregate-place identity; validate adjacent heap-array/track-caller/address cases.
- [ ] Repair 3b proof enforcement and representation/boundary prerequisites before promoting cleanup-edge removal or 4′a.
- [ ] Rebase and verify pending 1c, 1d, granule-min, wider inlining and pooling individually; preserve original branches and resolve fixture/schema collisions.
- [ ] Implement and freeze the public holdout; complete the original R1 gates rather than calling this scoped promotion R1.
- [ ] Complete browser parity, controlled runtime measurements and later distribution qualification.

The root integration report is the current status. Earlier under-verification/on-integration-only entries remain historical records. The user authorized local commits to main and prohibited pushing. This promotion does not claim a new broad implementation round or a new independent agent-verifier run.

- [x] Fast-forward main to the selected, documented candidate; rebuild both compiler binaries in main; replay standalone/open inventories and compact package kernels. Compact bytes match staging exactly. Evidence-only follow-up; no push.

## Round 2 plan — 2026-11-11 (approved; supersedes the "suggested order" notes above)

Plan: ~/.claude/plans/look-at-this-recursive-firefly.md. Base: main `b1bcca4`.
Goal: v2 with cleanup on beats the frozen v1 figures on minified, gzip and brotli for both ports, and improves the workload Rust/JS ratios, with no semantics lost.
Execution: implementers Opus 5.5 xhigh in their own worktree; a fresh Fable verifier at high effort per stage; commit to main only on pass / pass-with-limits; one commit per stage; no push.

Targets (`fp.esm.min.js`; v1 = baseline lane with the frozen v1 optimizer, `recovery/v2/snapshots/2026-09-07/compiler/checkpoints/phase-3.md:166-198`, Node zlib 9 / brotli 11, cleanup off):
- Compact: v1 160,615 / 44,283 / 33,890; main today 241,745 / 61,301 / 47,412.
- Idiomatic: v1 259,527 / 64,908 / 53,064; main today 443,971 / 111,722 / 86,772.

User decisions (2026-11-11):
- v1 target = the frozen figures above (no v1 checkout exists on this machine).
- Host values are checked at typed boundaries; 3b returns only after that and after the proof gate is fixed. A trusted binding stays an explicit opt-in.
- Pooling: a rule that cannot leak by construction; the multi-chunk load fix lands separately first.
- `as_flattened`: full support; then make the helper pay-per-use if possible.

Fixture ranges: A1 210–219, A2 220–229, A3 230–239, A4 240–249, A5 250–259, A6 260–269, B 270–289, D 300+.

### Stage 0
- [x] Baseline gate run on main `b1bcca4` (2026-11-11, load 5–11): build, check.sh, release fixtures 146/146, active-poll 60/60 with baseline refusals, cleanup transport and admission, extern 5/5, standalone all pass; known gaps 5/6 (aggregate place open); round1-open 1/5 in both modes (lost_write passes). Logs: session scratchpad `baseline/main-b1bcca4/`.
- [x] v1 targets recorded in `experiments/v2-restoration/RESULTS.md` (ea5604f)

### Wave A — open miscompiles
- [x] A1 copy aliasing (1c) — VERIFIED pass-with-limits 2026-11-12 (verified commit 35472d3 on branch worktree-agent-a4c08c1551e54383a; scratch verify-a1), squashed onto main. Rule A (`lower/fuse.rs` `uses_as_storage`): a single-use value is declared, not spelled inline, when its use writes it in place or is a view rooted at it that never dereferences, shared or mutable. Rule B (`lower/ssa.rs` `write_place`): a no-clone local gets its own copy before a projected write. Fixtures 210–214. Verifier: every claim reproduced; 797 own cases × up to 7 configurations all match native on the optimized lane; extent 661/667 (6 = heap arrays, A2); no regression in 128 fixtures × 7 configurations; `fallback_count` 0 in both packages and all ten workloads, so nothing reaches the baseline-lane bug below.
  - Cost landed with it: compact +527 min / +298 gzip (242,272 / 61,599 / 47,650), idiomatic +596 / +378; existing fixtures +5,051 B; flat-ABI sets flipped in 17 fixtures (e.g. `23_format` gains in-loop allocations); packages: no decision flipped.
- [ ] A1b recover A1's size cost (HIGH, cheap): the verifier's `tight5` variant (`verify-a1/tools/mutants.py`) exempts three classes the shared-borrow arm declares needlessly — a fresh object-typed value borrowed whole and handed to a callee (`f(&[a, b])`), call results, and views read by the adjacent `Deref` — and gives BOTH PACKAGES BYTE-IDENTICAL TO MAIN-BEFORE-A1, fixtures +14 B, same pass/fail on 210–214, 128 fixtures × 2 modes, 13 families × 3 modes and the extent corpus. It is an unreviewed experiment and fails one unit test whose expected text A1 edited. Implement properly, with the rule stated by meaning, and verify.
- [ ] A1c test gaps: no gate runs the fixture suite under `-Oz` + MIR inline (the shared-view and raw-view mutants pass 210–214 in the gate mode; only unit tests and 212 under -Oz catch them) → add that mode to the gate set; pin that a `needs_clone` local is written in place (mutant "rule B copies for every type" survives: it would clone an aggregate before every field write).
- [ ] A8 base miscompiles found by the A1 verifier (safe Rust, all lanes, same on main; `verify-a1/prebug/`): (L1) a written copy of an enum with MORE THAN 8 VARIANTS and an aggregate payload changes its source — the `Object.assign` shallow fallback above `MAX_CLONED_VARIANTS` (`jsc-link/src/lower/js/value.rs:557`, `backend/src/value.rs:717`); (L2) a write through `[_, rest @ ..]` of an array is lost (native `99 99 3`, mires `3 3 3`). Unsafe-only: copying a union of two aggregates throws; a write through a pointer cast to another struct type is lost (retype-guard family).
- [ ] Baseline-lane bug (default profile with `-Oz` + MIR inline only; `verify-a1/prebug/baseline_lane_oz.rs`, `a1/prebug/`): operand write through a shared/`&raw const` view of a fresh aggregate, lost write through `as_array_of_cells`, panic on the second of two futures built from one argument. Not reachable from the release profiles today (no item falls back). Also under that mode fixtures `25_array_ref` and `guard_use_after_free` are wrong and `15_ptr`, `115` are refused.
- [ ] A2 flattening + heap arrays (1d + 1f) — round 1 (branch worktree-agent-afbe5ab2cda06b284, 8d1f1de) FAILED verification 2026-11-11 (scratch verify-a2); round 2 with the implementer. Gates, sizes and most of the family reproduced. Fail reasons, all silent wrong answers: F1 (REGRESSION, safe, all modes) a one-byte heap value seen as `&[u8; 1]` is destroyed (`Box<u8>`, cap-1 `Vec<u8>`, 1-char `String`, `vec![true]`): `unscale_rows(p,1,1)`; F4 (REGRESSION, unsafe but normal representations) a zeroed/fresh `Vec<u8>` is retyped into rows by `p as *const [u8; 4]`; F3 (safe; main aborted) heap rows of width one; F2 (safe, rb/oz) a whole row stored through `&mut [T; N]` from the flattened view is lost — `__rt.overwrite` on an `_element_view` deletes its accessors, which is also the base lost write for flat buffers; F5 (safe, default + jir-off) `into_flattened` on a push-grown vector still wrong. Common cause: the runtime infers what a block is from sizes/freshness/JS shape instead of type facts from the lowering. Run-time costs added to programs that never flatten (must go): +2 WeakMap lookups per first address mint (minisearch +31,822), fmt width/fill reads now 2 calls + 5 weak lookups, `into_flattened` result Proxy-backed for life. The Proxy is not reached by any real workload (0 views in corpus and packages). Size: compact +2,612 min reproduces; ~910 B recoverable soundly when `__rt.unwindow` is not in the runtime closure (follow-up A2b), the other ~1,690 B needs a type fact.
  - Base bugs met by the A2 verifier (same on main; probes in verify-a2/probe/src): `Box<[u8]>::from(&[u8])` drop aborts in rb / reads 0 under jir-off; `concat` of rows of tuples and of zero rows aborts; `rotate_left/right` compile refusal; slice `<`/`==` on rows, `u8::from_le_bytes` on the JIR lane, `u128` in the default lane are compile refusals; hashing a `[u32]` crashes; `swap_with_slice`; `sort_by_cached_key` with a `String` key at n = 21 aborts in the default profile.
- [ ] A3 pointer records recognised by field name (1e) — after A2 (same files)
- [ ] A4 niche-optimised constants (1i) — round 1 (branch worktree-agent-a0a61a0aaa5ae144a, 0efa74d) FAILED verification 2026-11-11 (scratch verify-a4); round 2 with the implementer. Claims reproduced (fixtures 240–244, packages byte-identical). Fail reasons: (1) `niche_width` (`lower/js/value.rs:929`) takes `variants.first()` through a single-present-variant enum → `Some(Parsed::Char(..))`, `Some(Err(..))` over `Result<!, _>` and wrapping-niche variants read as the wrong variant on the JIR lane (same on main); (2) cycles through `Option<&'static T>` that never observe the link ran on main and now die at load (ReferenceError) — resolved by sequencing: A4 lands only on top of A6's static-cycle fix, cycle programs c1–c10 re-run on the combination. Also for round 2: two unpinned mutants (deep uninhabited walk), `static SL: Option<&[u32]>`/`&[u32]` to a static loses its length (base bug, newly reached). `Option<&dyn Trait>` `Some` constants are now a clean refusal (documented limit, same as plain `&dyn`). Base bug found: run-time `match` on `Result<(Infallible, ()), char>` throws "unreachable" (`opt/dce.rs:615`, `lower/js/rvalue.rs:1109`).
  - Round 2 (8586d19) FAILED a fresh verification 2026-11-12 (scratch verify-a4-r2) — SECOND FAIL. The niche-layout reconstruction held (600 generated enums, pattern types, 25 standard carriers byte-identical, both round-1 mutants killed, gates 156/156, packages byte-identical, combination with A6 40/40). Fail reasons, each a crash on main that became a silent wrong answer: (F1/F2) a one-element slice of an aggregate static — `static ONE: &[N] = from_ref(&B)` loses pointer identity, and a whole-element store through a `static mut` one-element slice never reaches the static (`constant.rs` `static_wide_pointer` wraps the object in a fresh array instead of the run-time slot); (F3) `const OF: Option<fn(u32) -> u32> = Some(|x| x + 7)` calls to 0 (native 8; the plain closure-fn constant reportedly prints 0 on main already). Also: `Option<&dyn Trait>` `Some` with a STATIC target compiles and throws at run time (refusal only covered promoteds); the interpreter refusal is too broad (property tests lose ~50 agreed argument sets each); the "intermittent" test failure is deterministic (245 compiled in the default profile or at opt-level 0); the array step and decline path of the niche walk are unpinned.
  - Decision (coordinator, per the split rule): F1/F2 came from the wide-pointer work I added to round 2, not from the niche fix. Round 3 is strictly narrow: fix-or-refuse each named defect locally, narrow the interpreter refusal, pin the two unpinned paths; no redesign. A fresh verifier re-checks only those points plus regressions. If round 3 fails, the wide-pointer commit is dropped from A4 and moves to A7.
  - Base bugs met by the round-2 verifier (same on main; verify-a4-r2/src): an enum with exactly one inhabited variant plus a kept dead variant throws "unreachable" under jir-opt-off (`min/m1b_dead_nonzst_rt.rs`); `slice.as_ptr()` vs `&array[i]` is false for aggregate elements (`W2/w16`); `slice::from_mut(&mut local)[0] = whole` loses the write (`W2/w18`, same `overwrite` family as A2 F2).
- [ ] A5 `#[track_caller]` chains (1h) — round 1 (branch worktree-agent-af471d6991ec28a6b, 14a45db) FAILED verification 2026-11-12 on one narrow point (scratch verify-a5); round 2 with the implementer. Root causes (both in the producer, both reproduced): a function-item value named the direct-call instance instead of the fn-pointer shim (`backend/src/jir/types.rs::fn_def_instance`), and a shim shared one `ItemId` with the function it wraps so the linker kept whichever it met last (`encode.rs::item_id`). Fixtures 250–254; existing fixtures, both packages and the ten-workload corpus byte-identical; locations equal native on 168 std panic programs; order-independent. Fail reason: a keyed-list callable that is a tracked function item compiled on main and is now a compile error (`lower/js/template.rs:923` compares against the old instance). Also: the `host.rs` change has no committed test.
- [ ] A5b (after A1 lands — needs `lower/ssa.rs`): a tracked body containing ANY virtual call (tracked callee or not, incl. a plain `&dyn Fn` callback) is declined on the optimized lane; when the body owes cleanup that is a compile error in release profiles. Same on main. Needs the producer to state "forward my location" explicitly (JIR-S/JIR-R schema). Minimal: verify-a5/p5/o6_dyn_fn_callback_cleanup.rs.
  - Base bugs met by the A5 verifier (same on main; verify-a5/pre, verify-a5/p3): a static `Option<fn()>` = `Some(f)` reads `None` (A4 family); `&fn_item` fails to compile; tuple/closure `CloneShim` writes `_0[0]` before `_0` exists and throws; a `self: Box<Self>` method on `Box<dyn Trait>` throws; out-of-bounds `Vec::remove` throws RangeError instead of reaching the panic handler on optimized lanes; with MIR inlining on and cleanup off a tracked call inside an inlined body reports a location inside the callee.
- [x] A6 multi-chunk ownership + statics that refer to each other — VERIFIED pass-with-limits 2026-11-11 (verified commit 4b3d379 on branch worktree-agent-a2ca43615f5f7cdb8; scratch verify-a6), squashed onto main. Verifier reproduced every claim: acyclic/unchunked programs byte-identical (fixtures in four modes, both packages, 40,000-table differential test of `order`), chunk suite 6/6 in six configurations, 264 generated-graph runs, and 40/40 on the ten cycle programs when combined with the A4 decoder.
  - Limits (all fail on main too): (1) a STATIC named as a chunk entry whose file and another file name each other's bindings throws ReferenceError at load (main: SyntaxError; `verify-a6/src/ck_static_entry.rs`; `link.rs` `entry_item` accepts any item; `mires-build` only passes function entries); (2) a static chunk entry inside a cycle is a link error; (3) size model undercounts cyclic programs ~20 B per member (ranking only).
  - Unmasked, NOT fixed here: in a cyclic program main threw at load; now `&STATIC.field` at offset 0 reads the base bug's wrong value (`verify-a6/src/in_first.rs`: native `1112 true …`, mires `0 false …`). The wrong value is base bug A7-a below and exists on main without any cycle. Landed anyway because the value bug predates A6; A7 is P0.
- [x] A6b chunk hardening — VERIFIED pass-with-limits 2026-11-12 (verified commit e7880e0 on branch a6b-hardening; scratch verify-a6b), squashed onto main. A split is refused when a module-level binding in one chunk file reads a binding owned by a file that imports it back (functions may cross); `__proto__` key in a cycle member refused; chunk suite runs from `check.sh` (13/13); fixtures 265–268. Verifier: 7,393 splits compared against the cf80163 linker in every load order — no false refusal (all 1,333 newly refused splits threw at load before), no accepted split throws; unchunked programs and both packages byte-identical.
  - Limit accepted: a `__proto__` field NESTED inside a cycle member is refused although it happened to run on cf80163 (`verify-a6b/src/proto_nested.rs`); fail-closed, conservative.
- [ ] A6c test gaps (bundle with the next linker item): a `refuse:` line with a three-file import cycle (mutant "import-back looked for directly only" survives and accepts programs that throw); a hoist-consts case (mutant "only a static counts as a read target" survives under `js-hoist-consts=on`); pin the nested `__proto__` refusal; `chunk-test.sh:127` accepts an empty `refuse:` text; `CONTRACT.md:1975` wording ("may name shared bindings when no shared binding names it" — a shared function naming it is enough to refuse).
- [ ] A7 constant pointers into statics (base miscompiles found by the A6 verifier; `backend/src/constant.rs`, after A4 lands — same files): (a) a constant pointer at offset 0 into a static whose pointee type differs from the static's is emitted as the static itself (`constant.rs:471` rejects only non-zero offsets): `&S.first_field`, `&ROWS[0]` read wrong, silently, safe Rust, every mode (`verify-a6/src/pre_nocycle.rs`); (b) a `&dyn Trait` constant pointing at a static loses its vtable → TypeError (`sh_dyn.rs`, `a6/preexisting/dyn_in_static_k2.rs`); (c) a struct field named `__proto__` is printed as a literal `__proto__:` key (`pre_proto_nc.rs`); (d) thin pointer into the middle of a static / one-past-the-end are compile-time refusals today.
- [ ] check-round1-open 5/5 in both modes

### Wave B — cleanup-edge removal
- [ ] B1 typed host-boundary checks
- [ ] B2 3b proof gate (validate before mutating; refuse on disagreement in release)
- [ ] B3 promote 3b (needs A2, B1, B2)
- [ ] B4 no-throw extensions (4′a) without `compare_bytes`

### Stage C — checkpoint
- [ ] One-tree measurement of compact, idiomatic, corpus, Pro on a quiet machine; row in RESULTS.md against the v1 targets; fresh byte and runtime attribution

### Wave D — ordered by the Stage C attribution
- [ ] Wider inlining (4′b) on A1 + 1g
- [ ] Emission wins: shared abort helper, module-scoped runtime, numeric enum tags, dead trailing arguments
- [ ] Pooling with the type-proven rule
- [ ] Formatter / String commitments
- [ ] Runtime: heap registry, string views, iterator/Option result objects, overflow IIFEs
- [ ] Cleanup-bearing body optimisation
- [ ] Verify granule-min address release

Deferred: aggregate-place identity (design review first), buddy allocator, seed-API adapter mismatch, public holdout, browser matrix.

### PAUSE — user, 2026-11-12: "let's pause all new work after finishing this one"
Read as: the four verifiers already running finish; nothing new is spawned or resumed, no send-backs, no merges, no gate runs until the user says so. The user also declined the integration gate run on main.
- On main (verified, unpushed): A6 (cf80163), A6b (7979702), A1 (1362e12). Main's compiler binaries are stale (built at b1bcca4).
- OWED before anything else lands: one integration gate run on main at 1362e12 (the three changes were each verified alone, never together). Script: session scratchpad `baseline/run-gates.sh <repo> <out>`; expected `copy_alias` + `lost_write` passing in round1-open, known gaps 5/6.
- Verifiers still running at the pause: A4 round 3 (branch worktree-agent-a0a61a0aaa5ae144a, e2245df), A2 round 2 (worktree-agent-afbe5ab2cda06b284, 96b89e3), A5 round 2 (worktree-agent-af471d6991ec28a6b, a420032). Their verdicts get recorded here; a pass is NOT merged during the pause.
- Not started: A1b (recover A1's +527/+596 B; brief not yet written, base would be 1362e12 with `briefs/COMMON-v2.md`), A1c, A2b (link-time lean runtime variants), A3 (after A2), A5b, A6c, A7 (after A4), A8, then Wave B.
- Briefs and evidence live in the session scratchpad (`briefs/`, `a*/`, `verify-*/`), a temporary location that is not backed up.


## Preservation and consolidation — 2026-11-12 (current user request)

- [x] Inventory all worktree heads, dirty changes, untracked sources and scratch evidence.
- [x] Preserve committed and unfinished work independently of `.claude/worktrees`, with hashes and restoration instructions; keep private/raw evidence out of public Git.
- [x] Bring public reports, representative regressions, generators and reproducible verification entry points into the main tree.
- [x] Run the combined-main verification; commit verified preservation/test/documentation changes, not unverified compiler branches.
- [x] Verify restoration from preserved data, commit locally, and leave all worktrees/stash intact. No push.

Verification: combined source at 1b582b8 rebuilt on main; 1,077 Rust tests, 75 runtime tests, 160/160 release fixtures, 13/13 chunks, 60/60 active-poll cleanup, transport/admission, extern and standalone passed. A1 package-mode subset 5/5. Strict open inventories remain known-gaps 5/6, round1-open 2/5 in both modes, round2-open 0/4 in each mode. No failing expected output was replaced.

Preservation: 43 source trees and all reconstruction patches verified; nine uncommitted source snapshots retained on archive refs; 812,662 raw file records and 146,007 unique contents checked, plus project sessions and opaque binary-fixture supplement. See WORKTREE-CLEANUP-HANDOFF.md. A2/A4/A5 and withheld optimization candidates are preserved, NOT promoted. No worktree/stash deletion and no push.

## Architecture research — 2026-11-12 (user request: is rustc→MIR→JS the right architecture; experiments; v3 candidates)

Goal: decide, with measurements, whether the current architecture (pinned rustc → MIR → semantic objects → jsc-link → JS) is the right level, and what a better one (v3) would be, judged on bundle size (min/gz/br) and runtime. No compiler source change on main; experiments live in the session scratchpad or ignored dirs.

- [x] R1 Byte attribution of the promoted compact package (by crate/module family, construct, generic-instance duplication, cleanup, runtime shim) (scratchpad/r1/REPORT.md)
- [x] R2 Value-model / contract digest (scratchpad/r2/REPORT.md): which observable semantics force each representation cost; which are self-imposed
- [x] R3 Control experiment (scratchpad/r3/REPORT.md): same pure kernels (fflate, marked, minisearch) through rustc→wasm32 (+wasm-opt) and wasm2js vs mires output: size and (noted) runtime
- [x] R4 Size levers (scratchpad/r4/REPORT.md) on the current compiler: cleanup on/off, baseline vs jir lane, minifier/target variants on the pure kernels and compact
- [x] R5 Landscape: how Kotlin/JS, Scala.js, Dart2JS, ReScript, J2CL, wasm2js, wasmGC toolchains get their size; any new Rust→JS work 2024–2026 (scratchpad/r5/REPORT.md)
- [x] R6 Hand-written "ideal managed lowering" oracle for the pure kernels: ceiling for a GC/erasure-based v3 (scratchpad/r6/REPORT.md)
- [ ] R7 Synthesis: verdict on current architecture; ranked experiment program; v3 candidate designs with go/no-go criteria; write-up

### Quick experiments — 2026-11-13 (user go-ahead; check of the external audit's claims)

Execution: three implementer subagents (Opus 5.5, xhigh), in the main checkout with the installed compiler, writing only under `build/arch-research/`; `CARGO_INCREMENTAL=1` for every cargo call. Verification is done by the coordinator in the main session, not delegated. Briefs: session scratchpad `briefs/` (common.md, q1-cleanup-tax.md, q2-unwind-target.md, q456-kernels.md). No compiler source change, no commit.

- [x] Q1 Cleanup on vs off on compact, idiomatic, marked/minisearch/fflate: sizes, lane counters, checks; like-for-like against frozen v1 (cleanup off)
- [x] Q2 Unwind-target trick with real `extern "C"` bindings: is host-exception cleanup kept? (probe arms A/B/C, one real fixture, optional `C-unwind` copy)
- [x] Q4 By-value `const` table: `static`, hoisted reference, scripted hoist ceiling
- [x] Q5 `__rt.overwrite`: profile attribution, same-key in-place variant with differential test, ceiling variant
- [x] Q6 64-bit kernel: profile attribution, `js-ecma-target=latest`, hoisted constants, Node + JavaScriptCore
- [x] Coordinator verification of Q1–Q6 (re-run sizes, checks, timings on a quiet machine; inspect emitted JS)

#### Review (2026-11-13, verified by the coordinator; compiler binaries unchanged: jsc-link 4e0d781e…, dylib 026c1823…)

Artifacts: `build/arch-research/{q1,q2,q456}/` (git-ignored; each has `results.json`). Coordinator re-runs: session scratchpad `verify-q1/`, `verify-q456/`. All timings Node v26.8.1, load 2.9–3.4, A/A within ±4%.

- Q1. `fp.esm.min.js` min/gzip/Brotli, cleanup on → off: compact 242,272/61,599/47,650 → 167,932/45,561/34,892; idiomatic 444,567/112,100/87,101 → 282,716/67,632/55,014. Workloads `index.min.js` gzip: marked 11,001 → 10,077; minisearch 32,343 → 15,354; fflate 9,751 → 6,097. Kernel checks 58/58 and corpus checks pass in all ten builds; compact-off rebuilt byte-identically by the coordinator. Against frozen v1 (cleanup off) the cleanup-off build is still larger: compact +2.9% gzip, idiomatic +4.2% gzip. Compact top-level functions 1,354 → 562. Rust/JS time ratio on → off: marked 5.2 → 2.4, minisearch 38 → 23, fflate 47 → 3.6.
- Q2. Under the unwind target (`panic=unwind`, `js-cleanup=off`) the `#[js_extern]`/`js_bindings!` bindings keep cleanup: the macro emits Rust-ABI marker functions, not foreign items. Repo checks on the candidate: noop-landing-pads 11/11, active-poll 20/20 (program 19,699 → 11,641 B, 149 → 42 functions). Cleanup IS lost for true `extern "C"` foreign items (`mires-js/src/collections/raw.rs` map_new/map_len/map_clear; `view-abi`): destructor skipped when `map_len` throws; a `"C-unwind"` copy restores it. An exported `extern "C" fn` holding a droppable value is refused at link ("exception termination requires a separate abort boundary"). Every final crate needs an `eh_personality` lang item. Whole suites and FingerprintJS were not built in this configuration.
- Q4. Confirmed. `const` table literal sits in the loop body: 1,011 µs; `static` 73 µs, `&CONST` before the loop 65 µs, emitted-JS hoist 73 µs (js_ref 26 µs). Size change ≤45 B.
- Q5. Partly true. `overwrite` is 79% of self time, but the same-key in-place variant (0 mismatches in the differential test, 75/75 runtime tests) gives 226 → 219 µs; the never-delete ceiling (not semantics-preserving) gives 125 µs; js_ref 2.0 µs. The cost is the representation, not the deletes.
- Q6. Partly true. `BigInt("…")` constants re-parsed at each use: 250 µs; `js-ecma-target=latest` 85 µs; hoisting the calls on ES2018 85 µs (JSC: 327 / 235 / 234 µs). BigInt itself stays slow: hand-written BigInt JS 13.6 µs Node / 89 µs JSC versus wasm2js 32-bit pairs 11.7 µs Node / 4.3 µs JSC.

Left on disk: `build/arch-research/` (~0.8 GB), one new shared sysroot `build/sysroots/cb2d60c0…` (`js-ecma-target=latest`).
- [x] R7 Runtime profile attribution of the three workloads (scratchpad/r7/REPORT.md)
- [ ] R8 Host-heavy control: fingerprint sources via wasm handle-table glue vs mires vs TS reference
- [ ] R9 Probe: JS-native containers (mires-js host collections) inside the current compiler, with and without cleanup

## Follow-ups from the quick experiments — 2026-11-14 (plan approved by the user; plan file `~/.claude/plans/harmonic-hatching-shore.md`)

Base `a868dcb`. Implementers Opus 5.5 xhigh (max three at once); each compiler change gets a separate verifier (Fable, high); the coordinator re-runs every gate number. Compiler changes live on their own branches in agent worktrees; nothing merges to `main` without a passed verification and the user's OK; no push. Briefs: `build/arch-research/briefs/`. Evidence: `build/arch-research/{u0,c1,r1,…}/`.

### Wave 1
- [x] U0 Probe: return the cleanup-preserving elaborated body to rustc's optimizer under today's target (branch `research/elaborated-to-rustc`, commit ee4957e). **No-go** (coordinator checked the ICE log and MIR dumps in `build/arch-research/u0/probes/`): under `panic=abort` rustc's validator rejects any unwind edge (`validate.rs:261`, `:502`, already while encoding `core`), and `InstSimplify` sets `unwind unreachable` on every call first. `js-cleanup=on`/`off` output unchanged by the probe commit. U1 proceeds.
- [x] C1 Hoist BigInt constants on ES2018, link-only (branch `opt/bigint-hoist`, commits `75973fd` linker + `45af4af` fixtures 270–274; 28 files +1230/−20; evidence `build/arch-research/c1/`). **Gate met on the coordinator's re-run:** `bench_murmur_u64(4096,42)` 253 → 87 µs in Node, same value (kernel rebuilt by me, identical to the agent's apart from file names); gzip deltas compact +2 B (+0.003%), idiomatic −56 B (−0.050%), marked +8, minisearch +15 (Brotli +104, +0.38%), fflate identical; kernel checks 58/58 on both ports. Agent reports no lane decision changed and `latest` output byte-identical on 160/160. Two implementer decisions under verification: statics and vtables keep `BigInt("…")` (hoisting them grew `19_float_fmt` by 15.8 KB); cost trials see inline constants.
- [x] C1 verifier (Fable, high; brief `c1-verify.md`; evidence `build/arch-research/c1-verify/`): **FAIL, one reason.** A hoisted binding evaluates `BigInt("…")` at module load, so a program whose 64-bit constants sit only in functions that never run throws at import on an engine without `BigInt` (control loads). Affects compact FingerprintJS (0 → 4 module-level references), marked (0 → 3), minisearch (0 → 4). Everything else agrees with control: all suite rows, five packages, 14,460 shift/rotate results against native Rust, name collisions, five chunk load orders, determinism, `latest` byte identity, cost-trial reuse.
  - Non-fail findings to schedule after the fix passes: (1) no default gate exercises the hoist (`test.sh` without a release profile, `check.sh`, `es2018-test.sh`, `emit-test.sh`, `dedup-test.sh` are byte-identical on both linkers); (2) three of nine mutations survive (bindings not registered in `defined`/`live`; hoist on `latest`; statics hoisted too) and a binding that is never inserted links with no diagnostic; (3) the lane sidecar's `physical.*` sizes no longer describe the emitted ES2018 program; (4) the constant shift mask and rotate subtraction still run per use; (5) a host module that imports the program back and calls an export during evaluation now hits the temporal dead zone at the first hoisted constant (control fails later, at the first static).
- [x] C1 send-back 1 (brief `c1-sendback-1.md`, sent 2026-11-14 with the fail reason only) — returned 2026-11-14: commits `36e5f8c`, `82aa859`, `8f38db4` on `opt/bigint-hoist` (8 files, +567/−112; jsc-link `5ed65412…`). Mechanism: `const X=<ok>&&BigInt("…")`, `<ok>=typeof BigInt==="function"`; the first constant on each path of a function is `(<ok>?X:<no>())`, later ones bare; fixture 275 + a `NN.no_bigint` second run in `scripts/test.sh` (ES2018 output only). **Coordinator re-run** (`c1/sendback-1/coord/`): my rebuild is byte-identical to the implementer's for kernel, compact, idiomatic, marked, minisearch, fflate; min/gzip/Brotli vs control: compact 242,356/61,658/47,833 (+117/+56 = +0.091%/+103), idiomatic 443,740/112,292/87,163 (−636/+64 = +0.057%/+29), marked 37,076/11,042/9,654 (+81/+42/+71), minisearch 124,324/32,390/27,439 (+86/+49/+148), fflate identical; 58/58 both ports; corpus checks equal. With `BigInt` deleted before load: compact, marked, minisearch, fflate load and behave as on control (old head threw at load for the first three); kernel `bench_murmur_u64` throws `ReferenceError` on control and new head alike. Timing under load 12–15 (three runs, fresh processes): control 367–409 µs, old head 129–151, new head 132–154 (ratio to control 0.34–0.38 for both heads); the absolute ≤ 100 µs was not re-shown at this load (implementer: 87.0 µs at load 3; old head 87 µs in my earlier run). I read the guard walk (`lower/bigint.rs`) against the AST (`rcgjs-object/src/jsast.rs`): exhaustive over a structured AST with no generator, `finally` or do-while; found no unsound case. Cost against the old head: compact +54 B gzip, marked +34 B, minisearch +34 B.
- [x] C1 verifier, second pass (same verifier resumed, brief `c1-verify-2.md`; evidence `.claude/worktrees/agent-a578ebeaef2acab48/build/c1-verify-2/`, `results.json` first) — returned 2026-11-14: **FAIL** on one narrow reason: `<no>` is `function(){return BigInt()}`, so when a program loads without `BigInt` and the host later defines a global `BigInt` whose zero-argument call returns (`globalThis.BigInt = Number`), every guarded constant is that one returned value; fixture 275 prints `true` where the control prints `false`. **Reproduced by me** on the implementer's fixture outputs. Everything else passed (verifier's evidence): the first fail reason is fixed (p4 probe, compact, marked, minisearch, fflate, all ten workloads load as on control without `BigInt`); its own audit of the parsed output finds 0 unprotected reads (161 + 158 fixtures, 30 chunk builds, 30 package files, kernel); 161 / 158 fixtures run with `BigInt` deleted behave as the control (125 / 122 equal output, 36 / 36 same error after equal output); 252 shape rows and five chunk load orders equal; `latest` byte-identical 160/160; suites equal row by row; gate met at load 5.5–6.2 (control 276.2 µs, old head 94.5, new head 94.7); no loop shape where the new head is slower than the control (one constant per iteration: 161 → 9.7 → 10.7 µs). Non-fail findings: a static-free program in a host import cycle runs on control and throws a temporal-dead-zone error at the first guard on both heads (round-1 finding, not worse); mutant n10 (fact passes between `case` bodies) survives every test and is not equivalent; six walk mutants are caught by one unit test only; no default gate runs fixture 275's second run (`test.sh` runs it only when `JS_EXTRA_ARGS` selects ES2018; `check.sh` does not run `test.sh`); n8 (support items unregistered) links silently; round-1 survivors m1, m3, m7 still survive; marked +0.38% and minisearch +0.15% gzip (not gated).
- [ ] C1 split after the second FAIL (lessons rule):
  - [x] C1a the guard's failing arm never returns (brief `c1a-guard-never-returns.md`) — returned 2026-11-14: commit `a4daa33` on `opt/bigint-hoist` (4 files, +77/−36; jsc-link `7ca11c51…`): `<no>` is `function(){throw BigInt()}`; `scripts/test.sh` gains a `NN.late_bigint` run and an empty `-- throws` line now means any failure. **Coordinator re-run** (`c1/c1a-coord/`): min/gzip/Brotli compact 242,355/61,657/47,781 (+55 B gzip, +0.089% vs control), idiomatic 443,739/112,293/87,191 (+65 B, +0.058%), marked 37,075/11,044/9,658, minisearch 124,323/32,391/27,463, fflate identical, all equal to the implementer's; 58/58 both ports; corpus checks equal; kernel at load 4–5, three runs: control 255–261 µs, previous head 88–89, new head 88–91 (gate ≤ 100 met), value 3914845223; fixture 275 passes with both absent-engine runs; late-`BigInt` case (`BigInt = Number` after load) now throws `0` after the three 32-bit lines; packages with `BigInt` deleted load and behave as on control.
  - [x] C1a narrow verification (same verifier, brief `c1a-verify.md`; evidence `.claude/worktrees/agent-a578ebeaef2acab48/build/c1a-verify/`) — returned 2026-11-14: **PASS**. Eight late stand-ins (none, `Number`, functions returning 7 / `undefined` / an object, a throwing function, a non-function value, the real `BigInt` restored): no guarded read yields a value on fixture 275, on 86 guard-reaching entry calls of its shape probe, or on compact's rounding helper; output differs from `8f38db4` only in the body of `<no>` (fixtures in both settings, chunk builds, packages, ten workloads); `latest` byte-identical to control 160/160; gate met (sizes equal to mine; kernel 271 / 93.5 / 93.7 µs at load 7). Non-fail findings: with the real `BigInt` restored after load the control computes correctly and the new head throws at the first guarded constant (documented in the module); the `late_bigint` run passes even if the stand-in is never installed (bare `-- throws`), so it shows the regression but not what was thrown; the unmodified control fails the new `late_bigint` row by design; the late run is skipped silently with the default target and under `js-modules=esm`. The existing `no_bigint` expectation is not weakened.
  - **C1 has passed verification** (hoist `75973fd` + fixtures `45af4af` + send-back 1 `36e5f8c`, `82aa859`, `8f38db4` + C1a `a4daa33`, branch `opt/bigint-hoist`). Merge needs the owner's OK.
  - [ ] C1b test gaps (needs the owner's go): a `switch` case in the walk's unit test and a fixture (n10); the support items' registration checked at link time (n8); the hoist and fixture 275's second run in a gate that runs by default; round-1 survivors m1, m3, m7; stale `physical.*` sidecar sizes; the host-import-cycle limit documented.
- [ ] C1 decision for the owner: new head (programs without 64-bit code paths keep loading on engines without `BigInt`; +54 B gzip on compact over the old head) or old head (`BigInt` at load, which `CONTRACT.md:302` already permits; +2 B gzip on compact).
- [x] R1 Census, no compiler change (`build/arch-research/r1/results.json`; coordinator re-counted enum shares with an independent instrumenter in scratchpad `verify-r1/` and re-measured the stub artifact):
  - By-value constant aggregates ≥ 48 B: 7 in compact, 7 in idiomatic, 0 in the four workloads; one is rebuilt in a loop and runs (42 times per load). The literals that run hot are `&'static` data, mainly the panic `Location` record (fflate 490,112 per call), and they disappear in the cleanup-off build. **C2 deferred as kernel-only.**
  - Enum objects, share of literal allocations, cleanup on / off: fflate 26.5% / 0.00%, marked 10.1% / 5.8%, minisearch (per call) 13.6% / 9.1%, idiomatic 17.4% / 14.7%, compact 1.9% / 0.2%. 87–100% of the hot ones are returned and switched on at once by the caller. **Enum prototype not started; re-measure on the unwind-mode output.**
  - 64-bit: no workload above 6.4% of time in BigInt functions. **32-bit pairs: no-go for now.** `flt2dec` is 11.9% of idiomatic gzip (444,567/112,100/87,101 → 391,777/98,790/77,367 when stubbed), 0% of kernel-check time. **Float-formatting prototype F1 started.**
- [x] Verifier for U0 (if go) and C1; coordinator re-run of gate numbers — U0 was a no-go (no verifier needed); C1 verification and re-runs are tracked in the C1 items above.

### Wave 2 (depends on Wave 1)
- [x] U1 Stage A + B: selectable unwind mode `JSC_PANIC=unwind` (branch `research/unwind-mode`, head `e277581`, 7 commits, 19 files +427/−47; brief `u1-unwind-mode.md`; evidence `build/arch-research/u1/`). Coordinator rebuilt all five programs in unwind mode (`u1/coord-unwind/`): SHA-256 identical to the agent's, kernels 58/58 ×2, three pure checks and the four host-throw suites pass.
  - Sizes min/gzip/Brotli, cleanup-on → unwind (cleanup-off): compact 242,239/61,602/47,730 → 214,674/59,860/45,498 (off 45,579 gzip; 10.9% of the gap); idiomatic 444,376/112,228/87,134 → 393,474/100,382/78,571 (off 67,695; 26.6%); marked 11,000 → **12,738** gzip (off 10,066; worse than on: native-representation pass commits nothing); minisearch 32,341 → 28,223 (off 15,326; 24.2%); fflate 9,751 → 7,531 (off 6,132; 61.3%).
  - Timing (coordinator, three interleaved rounds, load 6–8, rust/js ratio on / off / unwind): marked 5.4–6.2 / 2.4–2.5 / 2.5–3.0; minisearch 41–45 / 17–24 / 15–23; fflate 45–46 / 3.6–3.9 / 3.3–4.2. Unwind runs as fast as cleanup-off.
  - **Value gate: size half missed (56,300 / 97,300), timing alternative met (3 of 3 workloads ≥ 1.5×).** Invariant with `JSC_PANIC` unset: cache keys and 109/109 artifacts identical (agent's check; the 160 fixtures are compared in Stage C).
- [x] U1 Stage C (brief `u1-stage-c.md`; head `bba09a2`, 17 commits, 45 files +1058/−96 against `a868dcb`; evidence `build/arch-research/u1/stage-c/`). Coordinator re-ran in unwind mode (`stage-c/coord-unwind/`): `test.sh` release-balanced **160/160**; with MIR inline + opt z **150/160** (same ten rows as the agent); `unwind-mode-test.sh` 6/6; `async-test.sh` 28/28; `native-box-test.sh` fails as reported.
  - Agent's table (not all re-run by the coordinator): with `JSC_PANIC` unset every suite's output is byte-identical to control (test-rb 155/155 programs, test-rb-z 152/152, async 28/28, corpus 81/81); in unwind mode the cleanup suites pass, ten workloads and both FingerprintJS ports pass, 19 rows are skipped with a printed reason (baseline-lane refusals, abort-mode byte goldens, cleanup-transport rows that need a cleanup MIR member, the `cleanup === 'on'` receipt check).
  - **Correctness gate not met in package settings.** Seven rows fail only in unwind mode with MIR inline + opt z. Six are defects of the optimized-MIR path that exist on `main`: the coordinator reproduced all six on the unmodified main-checkout compiler with `js-cleanup=off` + MIR inline + opt z (`15_intrinsics` wrong output, `10_panic` wrong location, `25_array_ref` NUL bytes, `guard_use_after_free` missed abort, `115_zst_element_pointers` and `15_ptr` refused). The seventh, `103_boxed_fn_once`, is the fail-closed guard refusing an item whose JIR encoding declines (`SpreadNotTuple`).
  - **Contract question for the user:** `native-box-test.sh` expects a host throw to run no Box cleanup; in unwind mode both Boxes are dropped. Expectation untouched.
  - Diagnoses: (1) marked: the native-representation pass loses its candidates once rustc inlines `String`/`Vec` methods (15 of 17 declined for the same reason in cleanup-off); the two drop-glue candidates are declined by the zero-growth budget; no small fix. (2) Try blocks that protect nothing: 50 in compact unwind, −2,373 min / −254 gzip / −159 Brotli when removed; origin `lower/ssa.rs:679-690` + `lower/js/emit.rs:543-575`. (3) Remaining raw gap to cleanup-off (release-speed builds): compact 62,685 B, of which try/catch scaffolding 28,536; idiomatic 162,068, scaffolding 91,513. Costliest bail-outs (upper bounds, compact / idiomatic): operand fusion `lower/fuse.rs:156` ≈13.4 / 27.7 KB; SROA `rcgjs-object/src/sroa.rs:1448,:882` + `opt/partial_escape.rs:73` ≈11.8 / 24.1 KB; inlining `opt/inline.rs:546,:156` ≈7.1 / 3.9 KB.
- [x] U1 verifier (Fable, high; brief `u1-verify.md`; evidence `build/arch-research/u1-verify/`, `results.json` first) — returned 2026-11-14: **FAIL**, three reasons. (F1) A host exception crossing a Rust-defined `extern "C" fn` skips the destructors of the Rust frame that called it: rustc marks the call `unwind unreachable`, and the callee's `terminate(abi)` is now a rethrow; eight shapes (direct, callee with its own guard, fn pointer, `dyn` method, exported fn called from Rust, mid-chain, dependency crate). (F2) The foreign-import diagnostic names only `"C"` and `"system"`: a `cdecl` (or `unadjusted`) import builds and loses the destructor. (F3) An `extern "C"` import called only through a function pointer is never checked; the sysroot exemption is by crate name, so a crate named `compiler_builtins` is accepted. **Coordinator check**: `r01`, `r02`, `r03` compiled in the implementer's worktree at `bba09a2` in unwind mode and on the main checkout: control `[-1,1]` for all three; unwind `[-1]` for `r01` and `r02`, `[-1,1]` for `r03` (`"C-unwind"` middle). What held (verifier's evidence, not re-run by me): every other cleanup shape matches the control in both settings (30 `core`/`alloc` rows, 17 drop-path rows, 13 async, coroutines, re-entrant exports); the fail-closed guard (every lane option and declined encoding refused, mixed panic strategies refused by rustc); default mode byte-identical to control across 16 suites and both packages; the suite table and all five unwind package hashes reproduce; the 19 skipped rows hide no lost destructor; `native-box-test.sh` expects abort mode with cleanup off (events `[]`), unwind mode behaves like the control with `js-cleanup=on` (`[10,20]`). Non-fail findings: when a destructor itself throws, unwind mode runs more destructors than the control (`[-6,400]` → `[-6,410,420,400]`); `CONTRACT.md` overstates ("every live value", "rejects every call"); unwind mode is exercised by no default gate (`check.sh`, CI and `package.json` never set `JSC_PANIC=unwind` or run `unwind-mode-test.sh`); surviving mutants m5a, m5c (cache key), m8 (diagnostic without `"system"`), m9 (`mires.mjs` default); with the old guard condition `103_boxed_fn_once` links and `test.sh` reports PASS; `run.mjs --pure` checks behaviour of four of the ten workloads only; `check.sh` property tests read every object under `build/`. Control defects met: a foreign import used as a function value is a bare identifier; reading a foreign `static` ICEs with cleanup on; `BinaryHeap<u32>` push/pop wrong in every build; `slice::rotate` and `VecDeque::make_contiguous` refused in every build. Process: Write tool refused the main-checkout evidence path, evidence published with `rsync`; mutations and control ran in `git archive` exports; the worktree's `build/` was 133 GB, I deleted the four mutation/check export trees (93 GB).
- [x] U1 send-back 1 (brief `u1-sendback-1.md`) — returned 2026-11-14: commits `382db77`, `ebda208`, `e88a8f1`, `e899a4c`, `375524a` on `research/unwind-mode` (13 files, +780/−99; dylib `c029c293…`, jsc-link unchanged `e18bd8e3…`); evidence `.claude/worktrees/agent-af383844a6d287dcf/build/u1-sendback-1/`. Mechanism: nothing is made to run its destructors; in unwind mode the backend (`reject_unwind_crossing_calls`, `backend/src/driver.rs`) refuses every call whose callee's ABI cannot unwind by rustc's `fn_can_unwind` (foreign import, function pointer, `dyn` method, Rust fn whose cleanup ends in `UnwindTerminate(Abi)`, Rust fn without an available body, an inlined such edge), **only where the calling frame owes a destructor** (a maybe-live local with drop glue); sysroot exemption by load directory, not crate name. No emitted byte changed in either mode; fixtures 293–299; `unwind-mode-test.sh` runs two settings. Rejected: abort on `UnwindTerminate(Abi)` (control runs the destructors; breaks fixture 292), disabling rustc's `AbortUnwindingCalls` (MIR validator ICE), re-deriving cleanup per function. **Coordinator re-run** on the branch binary: `r01`, `r02` refused in unwind mode with a message naming `extern "C-unwind"`; `r03` gives `[-1,1]`; `r01`, `r02` in default mode byte-identical to my control builds; `unwind-mode-test.sh` exit 0 in both modes. Suite table and package sizes as in Stage C (agent's, not re-run). Agent's adjacent finding: the sysroot cache key does not record `CARGO_INCREMENTAL`, and a sysroot built without it changes crate hashes (broke byte comparisons in one of its runs). My slip: a bare `cargo build --release` in that worktree produced a different dylib (`12cc0072…`); `bash scripts/build.sh --locked` restored `c029c293…` before the runs above.
- [x] U1 verifier, second pass (same verifier, brief `u1-verify-2.md`; evidence `.claude/worktrees/agent-a3a5342a3902afde0/build/u1-verify-2/`, `results.json` first) — returned 2026-11-14: **FAIL**, five reasons, all in programs unwind mode accepts and where the control runs the destructors. (F1) Two `extern "C" fn`s in a row: the guard-holding frame's call to `outer` is accepted because `outer` has no `UnwindTerminate(Abi)` edge of its own, and the guard is lost; four forms (middle fn calls an import, another `extern "C" fn`, a function pointer, a `dyn` method). (F2) A guard living across an `await` is coroutine state, not a local with drop glue, so a non-unwinding call during a poll is accepted and the guard is never dropped. (F3) With inlining + `z`, a guard not used again on the normal path (`loop {}` after the call) is not counted live. (F4) With inlining + `z`, a throwing `#[global_allocator]` behind `__rust_alloc` (`#[rustc_nounwind]`, Rust ABI) is not looked at. (F5) A user rlib copied into the sysroot's library directory obtains the exemption. **Coordinator check**: `r04_two_extern_c.rs` in the implementer's worktree at `375524a` in unwind mode compiles and gives `[-1]`; on the main checkout `[-1,1]`. What held (verifier's evidence): the three first-round reasons are fixed (every shape refused with a diagnostic, `r03` correct); relay frames with the guard one or two callers up; seven "not a local" shapes match the control; eight liveness shapes refused where owed; nothing that built at `bba09a2` is newly refused; default mode byte-identical to control across 16 suites; unwind output byte-identical to `bba09a2`; suite table unchanged; seven mutations all caught, but only by `unwind-mode-test.sh` run with `JSC_PANIC=unwind`, which no default gate runs. `CONTRACT.md` has six untrue sentences about the mode. Dylib hash depends on the checkout path (`7f8d1c9e…` there, `c029c293…` in the implementer's worktree, same source).
- [ ] U1 split after the second FAIL (lessons rule) — **stopped for the owner's decision; nothing sent back**:
  - U1a (verified to hold twice, stays on `research/unwind-mode`, not mergeable alone): mode plumbing, sysroot keys, `eh_personality` patch, the fail-closed guard, default mode byte-identical.
  - [ ] U1b the host-exception boundary rule. The call-site refusal with a per-frame "owes a destructor" test is unsound by construction (transitive callees, coroutine state, liveness, attribute-based no-unwind). Options: (A, my recommendation) make rustc's assumption true: lower `UnwindTerminate(Abi)` as an abort, so a host exception can never leave a function rustc treats as non-unwinding; a function that must let one out (an export called from JavaScript, an import) is declared `extern "C-unwind"`; the call-site check and its liveness test go away; a non-unwinding import gets an aborting wrapper or is refused where it is declared. Cost: a contract change (today's release build drops and rethrows through an `extern "C" fn`; this aborts), fixture 292 and any export that should pass host exceptions must say `"C-unwind"`. The round-1 send-back brief (mine) ruled an abort out as an accepted outcome, which steered the implementer away from this. (B) keep refusing, with a whole-program "may let a host exception out" analysis at link time; conservative and large. (C) record unwind mode as no-go.
  - Unwind mode's correctness gate also needs D2 (the seven optimized-MIR root causes), whichever option is taken.
- [x] D3 Diagnose two more optimized-MIR defects, no fix (Opus 5.5 xhigh; brief `d3-cleanup-off-sort-heap.md`; evidence `.claude/worktrees/agent-a5553f06657fd8ced/build/d3/`, `results.json` first; branch `research/cleanup-off-sort-heap`, no commits) — returned 2026-11-14. Both are new root causes (not among D1's five), both fail in every cleanup-off configuration (so the fault is the backend / runtime, not a linker pass):
  6. **Slice built from a pointer of another type** (`sort_unstable_by` over 21–32 non-`Copy` elements): rustc's GVN strips the cast, leaving `*mut [MaybeUninit<T>] from (&raw mut <[T; 48] box>, 48)`; both lanes treat the data pointer as an element pointer (`backend/src/rvalue.rs:276-286` → `backend/src/ptr.rs:1111`; `jsc-link/src/lower/ssa.rs:1714-1725`, `lower/js/emit.rs:1135-1148`, `lower/js/ptr.rs:257-270`). 17 sysroot bodies have the shape (`Option::as_slice` is silently wrong with cleanup off and refused with cleanup on; `Rc`/`Arc` slice allocation). Fix ≈ 25–40 lines over 3–4 files.
  7. **Copy helpers index a field pointer as `off + i`** (`BinaryHeap::push` of a non-`Copy` element; `GapGuard::drop` for elements over 96 bytes): `{buf: obj, off: "elt"}` is read as `obj["elt0"]` (`runtime/shim.js:520-543`, `570-595`, `664-675`, `703-708`, `723-724`). **Also wrong in default release builds** when the source copies from or to a field through a raw pointer (stable Rust): `r3b_field_ptr_copy_scalar.rs` prints `undefined`, 9 with cleanup on (native 7, 5), `r3_field_ptr_copy.rs` throws; it is the cause of the known `BinaryHeap<u32>` defect. Fix ≈ 15–20 lines in the shim plus a linker rebuild.
  Neither shape is reachable in the cleanup-off builds of compact, marked, minisearch, fflate, uplot (agent's scan of 23,321 sysroot bodies and 1,089 package bodies; not re-run by me). Wide probe (76 cases, non-`Copy` and `Drop` elements): 46 right in both modes; differences listed in the evidence. Adjacent defects **in default release builds**, located but not diagnosed: `Weak::new()` then `upgrade()` or drop throws (`r6`, likely `backend/src/ptr.rs:611`); `.len()` on the iterator of an empty `Vec` panics, which breaks `Vec::splice` and `String::replace_range` when the replacement is at least as long as the range (`r5`, `r7`, likely `backend/src/ptr.rs:213-240`); with cleanup off + inlining + `z`, `.map(P)` with a tuple-struct constructor is refused (`r8`, `backend/src/constant.rs:304-309`); `BTreeMap` with `Drop` values is refused with cleanup on and right with cleanup off; `BTreeSet` refused in both. **Coordinator check**: reducers `r1`–`r8` compiled and run on the unmodified main-checkout compiler in release-balanced with cleanup on and off; every reported result reproduced (`r1`, `r2` right on / wrong off; `r3`, `r3b`, `r5`, `r6`, `r7` wrong in both; `r8` right in plain settings as reported).
- [x] D1 Diagnose the six optimized-MIR defects, no fix (Opus 5.5 xhigh; brief `d1-optimized-mir-defects.md`; evidence `build/arch-research/d1/`, `results.json` first; branch `research/optimized-mir-defects`, no commits) — returned 2026-11-14. Five root causes, all in the backend / lowering / runtime contract, none in the linker's optimizer (all six fail with `js-opt=off`, on the baseline lane and without a profile):
  1. **Caller location** (`15_intrinsics`, `10_panic`; needs MIR inlining only): `caller_location_argument` (`backend/src/intrinsics.rs:1656-1661`) ignores rustc's inlined `#[track_caller]` scopes (`Body::caller_location_span`); the JIR lane has the same gap (`backend/src/jir/encode.rs:687-693`, `jsc-link/src/lower/js/emit.rs:1487-1497`). Wrong panic locations only, no computed value changes. **Reaches the packages**: q1 cleanup-off outputs carry wrong core-internal locations (compact 7, marked 3, minisearch 6, fflate 0; cleanup-on 0). Fix ≈ 60–80 lines over 5–6 files plus an object-format field; a fixed build carries more distinct location literals (q1 compact: 30 distinct / 3,063 B with cleanup off against 145 / 8,571 B on; minisearch 27 / 1,895 B against 246 / 15,964 B), so part of the measured cleanup-off size advantage is this defect.
  2. **Window pointers** (`25_array_ref`; inlining + `z`): a place projected out of a raw pointer cast to a shorter array (`*mut [E; N]` over a longer buffer) indexes `p.buf[p.off][i]` instead of `buf[off+i]` (`backend/src/place.rs:177-197`, `jsc-link/src/lower/js/place.rs:121-125`). **Also wrong in default release builds** when the source writes `(*p)[i] = v` itself: reducer `d1/repro/r3_window_index.rs` prints 7, 0 with cleanup on (native 7, 9). Not found in the packages. Fix ≈ 10 lines per lane.
  3. **Freed-block check** (`guard_use_after_free`; `z` only): rustc's GVN removes the pointer cast the runtime's freed-block check lives on; the program is UB, the check is best-effort. Fix ≈ 10 lines in `runtime/shim.js` (a throwing prototype on a freed buffer) or a documented limit.
  4. **Wide constants without provenance** (`115_zst_element_pointers`; inlining + `z`): GVN folds `*const [Z]` / an empty `&[u32]` built from a dangling pointer into a constant the backend refuses (`backend/src/constant.rs:399-414`, `backend/src/jir/encode.rs:1708-1711`). Refusal, not a wrong value. Fix ≈ 10 + 30 lines.
  5. **Array-to-integer transmutes** (`15_ptr`; inlining only): InstSimplify turns `raw_eq::<[i32; 4]>` into a transmute to `u128` the backend does not support (`backend/src/intrinsics.rs:1905-1921`, `jsc-link/src/lower/js/unsize.rs:159-171`). Refusal. Fix ≈ 30–40 lines per lane.
  **Coordinator check**: all eight reducers (`d1/repro/r1`–`r8`) compiled and run on the unmodified main-checkout compiler in release-balanced (cleanup on) and with cleanup off + inlining + `z`: every reported result reproduced, including r3's `7, 0` with cleanup on; the cited code read; minisearch's q1 cleanup-off output contains `core/src/option.rs:1014:21` and `core/src/slice/index.rs:245:14`, its cleanup-on output neither. Other fixtures: no passing fixture prints different output between cleanup on and off (agent's comparison, not re-run). Adjacent: `08_closures`, `09_drop`, `10_dyn` fail under any `-Copt-level` above 0 because `mini_core` lacks the `unsafe_unpin` lang item. Process: the agent's Write tool refused the evidence path and it wrote there with shell copies; it briefly edited `runtime/shim.js` during a control run, reverted, and re-ran the suite.
- [ ] D2 (needs the owner's go; not in the approved plan; D3 adds causes 6 and 7 and three adjacent default-build defects — field-pointer copies (7) and the empty-iterator `len` / `Weak::new` defects are wrong in default release builds and would go first with window pointers) Fix the five root causes, one item each with its own verifier: caller location first (it is in the packages), window pointers second (wrong result in default release builds), then the two refusals; the freed-block check is a contract decision.
- [ ] U2 Full suites + compact/idiomatic/workload measurement in the surviving mode. Gate: all rows pass; compact gzip ≤ 56,300 and idiomatic ≤ 97,300, or two of three workloads ≥ 1.5× faster
- [x] C2 By-value read-only constant aggregates: **deferred** on the R1 census (one live site in six programs). The hot case is shared `&'static` data, which belongs to the pooling redesign; revisit on unwind-mode output.

### Wave 3 (conditional)
- [x] F1 Native format route commits in cleanup-on builds, so idiomatic no longer ships `flt2dec` (branch `opt/native-float-format`, commits `2de50cf` + `83446ad`; 10 files +687/−52, linker only; evidence `build/arch-research/f1/`). Diagnosis: two roots, both a recognised form missed at the cleanup-mode call shape (template arrives as `&*v`; `Move` renames unmarked; result conversion placed after an invoke). **Gate met on the coordinator's rebuild:** idiomatic 444,376/112,228/87,134 → 347,850/88,369/69,367 (−21.3% gzip, identical SHA to the agent's), compact identical, 58/58 on both ports, `native-f64-differential.sh` PASS (1,263,399 cases against real Rust), `native-format-args-test.sh` PASS (268,383 → 2,988).
  - About 13.5 of the 21.3 points are the float route; the rest is six other `format!` sites that now commit natively, after which the rest of `core::fmt` drops out. None of the eight sites is exercised by the kernel checks.
  - **Finding on `main`:** `native-format-args-test.sh` fails on the unmodified `a868dcb` build (268,383 → 268,383): the native format route committed nothing in cleanup-on builds. This change repairs it.
  - Not covered: with MIR inlining on + opt z, fixture 300's `to_string` region stays baseline (solver does not count what the commit frees).
- [x] F1 verifier (Fable, high; brief `f1-verify.md`; evidence `build/arch-research/f1-verify/`): **PASS.** 145 probe programs / 411 runs equal real Rust in formatted text; 408 host-throw runs have the same events and outcome as control; unconverted forms (`{:?}`, `{:e}`, precision, width, flags, `f32`, user `Display`) decline with byte-identical programs; ten workloads and compact byte-identical; the invoke-successor placement is guarded by `verify_carriers` at commit time ("invoke normal successor must be private"). Verifier's linkers are bit-identical to the implementer's.
  - **Decision for the user:** after a committed `format!`, `String::capacity()` is exact-fit (3 for `"0.5"`) where Rust and the control give 8. `CONTRACT.md:1036-1038` specifies exact-fit capacity for native Strings and the control already does this for `f64::to_string()`; `OPTIMIZER.md:11` says the optimizer must preserve Rust-visible capacity. Coordinator's recommendation: accept, and correct the `OPTIMIZER.md` sentence.
  - Non-fail findings: (1) the change also makes format sites commit in cleanup-off optimized builds (a `format!("{}", f64)` program 219,418 → 7,757 bytes under `js-lane=jir js-opt=on`); (2) fixture 300 passes on the control linker, so it cannot detect the route; (3) six of twelve mutations survive (one live: removing the use-once check makes a twice-read `Arguments` commit); (4) the rename marking has no commit-level unit test; (5) `native-format-args-test.sh` has failed since the commit that added it (`93c0785`); (6) identical on both linkers and unrelated: `native-int-string-test`, `native-capacity-test`, `flat-result-test` fail; `i32::MIN.abs()` aborts with overflow checks off; a constant `&[&dyn Trait]` is refused at link time.
  - Process: both verifiers had the Write tool refused for the main checkout's evidence directory and copied files in with shell commands; the F1 verifier ran its probes and extra suites with the repo's integration lock skipped.
- [ ] R2 At most one representation prototype, only if its R1 condition is met (enum objects ≥ 10% of allocations or time on two of four programs; `flt2dec` ≥ 3% of idiomatic gzip or time; 64-bit arithmetic ≥ 10% of a real workload)

#### Notes
- Worktree controls differ slightly from the main checkout's numbers on identical source (compact 242,239/61,602/47,730 vs 242,272/61,599/47,650). Cause per the C1 agent (not re-checked by the coordinator): `build_sysroot.sh` puts the backend dylib's path into `RUSTFLAGS` (`-Zcodegen-backend=<path>`), which changes every `$h` symbol hash, and size-scored linker decisions then flip. All three worktree controls agree with each other byte for byte; every comparison uses the worktree's own control.

#### Review (2026-11-14, every gate number below re-run by the coordinator unless marked "agent")

**Passed verification, on local branches, not merged (merge needs the owner's OK):**
- C1 BigInt constant hoist, `opt/bigint-hoist` at `a4daa33`: murmur kernel 255–261 → 88–91 µs (load 4–5); gzip compact +55 B (+0.089%), idiomatic +65 B (+0.058%), marked +44 B, minisearch +50 B, fflate identical; programs load without `BigInt` as before. Two failed verifications on the way (load-time `BigInt`; a returning late stand-in), split after the second, narrow pass passed. Limits: real `BigInt` restored after load throws; host import cycle TDZ. C1b test gaps open.
- F1 native format route in cleanup-on builds, `opt/native-float-format` at `83446ad`: idiomatic 444,376/112,228/87,134 → 347,850/88,369/69,367 (−21.3% gzip); compact and workloads identical. Decision item: exact-fit `String::capacity()` after a committed `format!`.

**Failed verification twice, split, stopped for a decision:**
- U1 unwind mode, `research/unwind-mode` at `375524a`. Value measured in Stage B (gzip cleanup-on → unwind: compact 61,602 → 59,860, idiomatic 112,228 → 100,382, minisearch 32,341 → 28,223, fflate 9,751 → 7,531, marked 11,000 → 12,738; rust/js time ratio marked 5.4–6.2 → 2.5–3.0, minisearch 41–45 → 15–23, fflate 45–46 → 3.3–4.2): size half of the gate missed, timing half met. Correctness: loses destructors on a host exception through non-unwinding ABIs (five reasons open); fixtures 160/160 in release-balanced, 150/160 with inlining + `z`.

**No-go / deferred:** U0 (rustc MIR validation ICE under `panic=abort`); C2 by-value constant hoist (kernel-only, deferred); R2 representation prototypes not started (conditions not met on cleanup-on output; to be re-measured on optimized-MIR output).

**Defects found on the way (diagnosed, not fixed; D2 needs the owner's go):**
- Optimized-MIR path (cleanup off / unwind mode), seven root causes: caller location ignores inlined `#[track_caller]` scopes (in the packages' cleanup-off output); window pointers; freed-block check removed by GVN; wide constants without provenance refused; array-to-integer transmutes refused; slice built from a pointer of another type; copy helpers index a field pointer as `off + i`.
- Wrong results in default release builds today: window-pointer writes `(*p)[i] = v` lost; copies through a raw pointer to a struct field (`undefined` / `TypeError`); `.len()` on the iterator of an empty `Vec` panics (breaks `Vec::splice`, `String::replace_range`); `Weak::new()` then `upgrade()` throws. Also `BinaryHeap<u32>` (same cause as field-pointer copies), `BTreeMap` with `Drop` values and `BTreeSet` refused.
- The cleanup-off size figures used as a ceiling (Q1) come from the defective path; a correct build carries more location literals (compact 30 distinct / 3,063 B off vs 145 / 8,571 B on).

**What this says about the architecture question:** consuming rustc's optimized MIR is where the measured runtime gain is (2–12× on three workloads) and part of the size gain, but that path was never exercised by the release gates and has at least seven backend defects plus an unsolved host-exception rule. The linker-only items (C1, F1) are the ones that reached a pass.

**Process:** verifiers' Write tool refuses main-checkout paths for worktree agents; from the second round on, evidence stays in each agent's own worktree. One verifier's worktree reached 133 GB; I deleted 93 GB of mutation exports. A bare `cargo build --release` in a worktree changes the backend dylib (lesson recorded). The sysroot cache key does not record `CARGO_INCREMENTAL` (agent's finding, not fixed).

## Fix wave — 2026-11-14 (owner's decisions after the review above)

Owner, 2026-11-14: (1) unwind mode takes option A (abort at a non-unwinding boundary, `extern "C-unwind"` for what must pass a host exception); (2) fix the diagnosed defects, default-build ones first, then caller location, then the rest, one item each with its own verifier; (3) merge `opt/bigint-hoist` and `opt/native-float-format` into `main` first, then make the fixes. No push.

### Merge
- [x] Combined check before `main` moves: branch `integrate/c1-f1` (`14e3d60`, `opt/bigint-hoist` + merge of `opt/native-float-format`, no conflicts) built in the C1 worktree; `scripts/test.sh` in two settings, ES2018, emit, chunk, dedup, runtime, check, kernel, the five packages with their checks, `native-f64-differential.sh`. Evidence `build/arch-research/c1/merge-c1-f1/`. Result: `test.sh` 167/167 (release-balanced) and 164/167 with inlining + `z` (the three `mini_core` rows that fail on the control too); row-by-row equal to the control plus the seven new fixtures; emit 17/21 as on the control; ES2018, chunk, dedup, runtime, check exit 0; float differential PASS (1,263,399 cases); 58/58 on both ports; workloads pass. Sizes min/gzip/Brotli: compact 242,355/61,657/47,781 (byte-identical to the verified C1 head), idiomatic 347,977/88,412/69,404, kernel 43,784/12,227/10,225; workload programs byte-identical to the C1 head.
- [x] `main` fast-forwarded to `14e3d60`, then `956edd2` (docs: `OPTIMIZER.md` states exact-fit capacity of native Strings). Main checkout rebuilt with `scripts/build.sh --locked`: jsc-link `d6b096e2…` (same as the checked build), dylib `c9467997…`. Not pushed. The C1 worktree stays on `integrate/c1-f1`.

### Fixes (implementers Opus 5.5 xhigh, one item each, at most three at once; each gets its own verifier; briefs in `build/arch-research/briefs/`)
- [x] X1 window pointers implemented (`x1-window-pointers.md`, branch `fix/window-pointer-index`, fixtures 310–314). Default build wrong today.
  - Implementer hand-back about 17:41: `fix/window-pointer-index` at `d01fdbd` (3 commits, 19 files +843/−33; worktree `.claude/worktrees/agent-aa470b163f9c519b4`, evidence `build/x1/`). Root cause in the place walk of both lanes: `Deref` of a pointer to an array followed by `Index` emitted `p.buf[p.off][i]`; now `__rt.array_buf(p)[__rt.array_off(p)+i]`, or the direct form when the slot literal shows its shape. Also fixes safe `Box<[T; N]>` indexing (read `undefined` on the base) and `copy_nonoverlapping::<[E; N]>` through a window. Agent's numbers: 162 other fixture programs and all package artifacts byte-identical to the base.
  - Coordinator re-run: reducer `7 9` on the head in four settings, `7 0` on an independent base build; fixtures 310–314 pass on the head, each differs from its expected file on the base; the head's compact and idiomatic artifacts have my base SHA-256.
  - Not fixed (agent's list, to schedule as X1b): `w.add(k)` on a window pointer moves by k elements instead of k·N; by-reference `rest @ ..` of an array place copies the elements; `Vec<[T; N]>` after `as_mut_ptr()` throws when sliced; DOM goldens `19_text_conversion` not regenerated.
- [x] X1 verifier (Fable, high), brief `x1-verify.md`
  - **FAIL (hand-back about 19:55), one reason;** evidence `.claude/worktrees/agent-a6d3eef11d849f4dd/build/x1-verify/`. A window of length 1 over a struct field: `(*w)[i] = 20` with `w = &raw mut s.b as *mut [u32; 1]` prints `2 20` (native `20 20`, base `2 undefined`); the new spelling adds the index to a property-name offset. One related row printed a value on the base and throws on the head. Reproduced by the coordinator.
  - Held: 155 probes / 444 cells, 0 right-on-base-wrong-on-head; window shapes over indexed buffers right; packages byte-identical because the shape does not occur in them; `check.sh` exit 0; 58/58; all suites' programs identical apart from shim copies.
  - Findings (not fail reasons; for the owner at merge and for follow-up items):
    - **Speed:** a raw `*mut [T; N]` or `Box<[T; N]>` that is never a window now pays six helper calls per loop iteration: 114.8 ms → 174 ms (about 1.5×, measured under load 24–36) on a loop through `*mut [u32; 64]` in a non-inlined function; `&mut [T; N]` references are unaffected (byte-identical). SROA declines one case it accepted (3,459 → 3,625 B in a probe). The direct spelling almost never fires.
    - **Tests:** five of nine mutations survive (plain-slot half of `array_buf`, `copy_arrays` counts above 1, `queue.rs` purity); the backend revert is caught only by bare `scripts/test.sh`.
    - **Changed for the worse:** by-reference `rest @ ..` on a window place threw on the base and now silently loses two writes.
    - Still wrong in the default build, same on the base: `ptr::replace` / `mem::replace` / `mem::take` / `mem::swap` / whole assignment through a window; `swap_nonoverlapping`, `write_bytes` on arrays; `as_chunks`; pointer arithmetic on a window; `Vec<[T; N]>` `pop` / `insert` / `remove` / `clone` / `drain` / growth and `VecDeque<[u32; 2]>` throw.
    - Safe-code defects found on the way, no raw pointer in the source, same on the base: `*m = [6, 7]` through `(&mut xs[1..3]).try_into()` is lost; `for r in [[1u8, 2], [3, 4]]` throws in release-balanced; `as_flattened_mut()[3] = 40` writes into row 0; `slice::from_mut(&mut s.b)[0] = 20` leaves the field unchanged; `sort_by` on a 60-element `[[u32; 2]]` throws.
- [ ] X1 send-back 1 (brief `x1-sendback-1.md`, sent about 20:00): the one fail reason.
- [ ] (implementer running since 15:30) X2 field-pointer copies (`x2-field-slot-copies.md`, branch `fix/field-slot-copy`, fixtures 315–319). Default build wrong today; also `BinaryHeap`.
  - Implementer hand-back about 17:36: `fix/field-slot-copy` at `51193ef` (two commits; worktree `.claude/worktrees/agent-acf19b702a41df035`, evidence `build/x2/`). Behaviour right in its runs (reducers native in six settings, fixtures 315–319 fail on the base, wide probe 46 → 54 of 76, runtime tests 86/86). Also found and fixed the same fault in the window helpers (`slice::from_ref(&s.f)` read `undefined` on the base). **Size line missed:** gzip compact +80 B (0.130%), idiomatic +78 B, marked +58 B (0.525%), minisearch +113 B (0.349%), fflate +54 B (0.554%); the larger helper text also flipped unrelated optimizer decisions (`72_repr_format_min` +2,365 B with inlining + `z`). Not sent to a verifier.
  - [x] X2 size pass (brief `x2-size-pass.md`, sent about 17:38): cheapest correct form by measurement, target ≤ +0.1% gzip per package, no unrelated program changed through pricing.
  - Size-pass hand-back about 19:35: head `529fe5b` (two more commits). The inserted shim lines are marked `// @unpriced`: shipped, but not priced by the optimizer, so every fixture program and every package `program.js` is byte-identical to the base again (`72_repr_format_min` included). Cost is the shipped runtime only. Coordinator measured the head's package artifacts against the base (min/gzip/Brotli deltas): compact +287/+73/+60 (0.118% gzip), idiomatic +287/+71/−35 (0.080%), marked +211/+58/+13 (0.525%), minisearch +368/+91/+21 (0.281%), fflate +211/+54/+39 (0.554%). The agent's floor argument: the `_run` branch (about 210 min bytes) must ship wherever a program has a copy helper and can construct a field pointer, which is all five packages. Runs fix alone (built): +56/+53/+58/+53/+54 gzip. Coordinator re-run: `r3b` `7 5` and `r3` `9 7 5` on the head, `undefined 9` and `TypeError` on the base; fixtures 315–319 pass on the head.
  - **Decision for the owner at merge:** accept +54–91 B gzip per package for this fix (the 0.1% line is missed on four of five).
  - [ ] (running since 19:47) X2 verifier (Fable, high), brief `x2-verify.md`
  - Adjacent defects it reported, not fixed (reducers in its `build/x2/repro/`): A1 `p.add(0)` on a field pointer reads `undefined` (handed to X3 as a possibly related shape); A2 `ptr::swap_nonoverlapping` on two field pointers aborts; A3 a cast from a pointer to a `repr(C)` struct with one aggregate field to a pointer to that field is lowered as the identity; A4 cleanup off + inlining + `z`: inlined `array::from_ref(x)[0]` reads the wrong place; A5 in-place `collect` from `Vec<P>` to `Vec<u32>` refused with cleanup on, NaN then abort with cleanup off.
- [ ] (implementer running since about 17:38) X3 `.len()` on an empty `Vec`'s iterator (`x3-empty-iter-len.md`, branch `fix/empty-iter-len`, fixtures 320–324). Default build wrong today; root cause not yet diagnosed.
- [ ] (implementer running since 17:44) X4 `Weak::new()` (`x4-weak-new.md`, branch `fix/weak-new-sentinel`, fixtures 325–329). Default build wrong today; root cause not yet diagnosed.
- [ ] X5 caller location in inlined `#[track_caller]` scopes (optimized-MIR path; in the packages). Brief to write.
- [ ] X6 slice built from a pointer of another type; X7 wide constants without provenance; X8 array-to-integer transmutes; X9 tuple-struct constructor as a function value. Optimized-MIR path. Briefs to write.
- Not scheduled, needs a decision: the freed-block check removed by GVN (a contract choice); `BTreeMap` with `Drop` values / `BTreeSet` refusals; the three `mini_core` fixtures above opt level 0.
- [x] U1b option A implemented (`u1b-abort-boundary.md`, branch `research/unwind-abort-boundary` from `research/unwind-mode` + new `main`, fixtures 330–339).
  - Implementer hand-back 17:30: branch `research/unwind-abort-boundary` at `4473e1d` (merge of `375524a` and `956edd2` without conflicts, then 7 commits; worktree `.claude/worktrees/agent-aa0342a7d625af46d`, evidence `build/u1b/`). Abort = `__rt.js_abort(...)` at the boundary, outer frames rethrow aborts without cleanup, nothing poisoned. Host imports and function-pointer calls marked `unwind unreachable` get a call-site wrapper (first site about +349 B min / +193 gzip, then +96 B / +15–20 gzip); `#[global_allocator]` methods abort at the definition; nothing refused; the call-site refusal is removed. Package exports are Rust-ABI (`#[js_export]`), so no package source changed; ten `extern "C"` exports of `examples/async-tests` became `"C-unwind"`. Fixtures 290–299 re-expected, 330–336 new, `scripts/check.sh` now runs the unwind tests in both modes. Agent's numbers: 0 lost rows in both settings; default mode byte-identical to the base; unwind gzip compact 59,948, idiomatic 97,738, marked 12,882, minisearch 28,441, fflate 7,531 (default: 61,657 / 88,412 / 11,044 / 32,391 / 9,751).
  - Coordinator re-run: `r01`, `r04` unwind `[-1]` abort in both settings, default `[-1,1]` marker; `r04` with `"C-unwind"` `[-1,1]` marker; `h03` (3 rows), `h05` (1), `p07b` (4) with inlining + `z`: every host-throw row an abort with a prefix of the default events; the head's saved default output of 162 fixtures, compact and idiomatic identical to my base outputs.
  - Note for U2: since the float-format route merged, idiomatic is larger in unwind mode than in default mode (97,738 vs 88,412 gzip); marked also (12,882 vs 11,044).
- [x] U1b verifier (Fable, high), brief `u1b-verify.md`. Coordinator also ran `scripts/unwind-mode-test.sh` on `4473e1d`: exit 0 in unwind mode and in default mode.
  - **FAIL (hand-back about 18:35)**, four reasons, all programs unwind mode accepts, both settings; evidence `.claude/worktrees/agent-a28e065b8789bd449/build/u1b-verify/`. Questions 3, 4, 7 (default-mode identity of the suites, unwind suites and packages, mutations) were unfinished at hand-back.
    - F1 `mires_js::try_host` swallows the boundary's abort: `__rt.host_try` has no `is_abort` test, so the closure's guard is skipped and the outer frame goes on (`[-2,210,220,200]` default, `[-2,220,200]` unwind, both return 2). `CONTRACT.md` documents that `try_host` answers an abort as `Err`; in default mode a Rust panic under `try_host` already behaves this way. **Owner's decision.**
    - F2 an `Assert` inside an `extern "C" fn` has no abort edge; a `#[panic_handler]` that throws a host exception lets it out (`[-999,200]` vs `[-999]`, marker). Reproduced by the coordinator.
    - F3 a `#[rustc_nounwind]` declaration with the Rust ABI gets no wrapper (`[-1,100]` vs `[-1]`, marker). Reproduced by the coordinator.
    - F4 the engine's stack overflow (`RangeError`) at a call to a Rust `extern "C" fn`: inner guard skipped, outer runs (`[310,300]` vs `[300]`). No host function throws it. **Owner's decision** whether it counts as a host exception.
    - Held: the implementer's 39 probe programs (0 lost), cost figures reproduced exactly, fixtures 290–299 expectations right, wrapper survives `js-opt=off` and `js-ecma-target=latest`. Findings: six rows where unwind mode runs more destructors than default when a destructor throws; three untrue `CONTRACT.md` sentences; tracked examples that still export `extern "C" fn` and would abort (`dts-tests/01_shapes`, `extern-tests/02_globals`, core 67/68/110, `bench/vanilla`); the jsc-link unit test that scans every `.o` under `build/`.
  - Verifier's suites helper, hand-back about 20:15 (the gates the verifier left unfinished, on `4473e1d`; record `…/agent-a28e065b8789bd449/build/u1b-verify/suites/summary.json`): default mode unchanged, 429 `.js` / 31 `.mjs` / 32 `.map` compared, 0 differing; both `test.sh` tables identical to the base; a control with the head's backend on the base's sources (the head edits many sources to `"C-unwind"`) is also identical for `test.sh` and the async suite. Unwind mode: `unwind-mode-test.sh` 68/68 in both modes, async 28/28, cleanup suites pass (three baseline-lane rows each of promise-cleanup and task-cleanup skipped by design), 58/58 both ports, workloads pass, package sizes and SHA-256 equal to the implementer's; `test.sh` 167/167 and 157/167 with inlining + `z` (the ten known optimized-MIR rows); `native-box-test.sh` exit 1 in unwind mode as before. Abort sites per package: compact 1, idiomatic 5, marked 1, minisearch 1, fflate 0. The mutation helper had not reported.
- [ ] U1b send-back 1 (brief `u1b-sendback-1.md`, sent about 18:45): F2 and F3 only. F1 and F4 wait for the owner.
- [ ] U2 after U1b passes and X5–X9 are in: the final unwind-mode measurement and gate.

#### Outage — 2026-11-14 about 20:00 to 2026-11-15 00:40
- A network outage ended every running agent mid-step: X1 implementer (send-back 1, 7 uncommitted files at `d01fdbd`), X3 implementer (`d25732d`, uncommitted work), X4 implementer (`73a924b`, uncommitted work, last step a temporary revert), U1b implementer (send-back 1, `d469dfd`, three commits, gate runs unfinished), X2 verifier and its mutation helper, the U1b verifier's mutation helper.
- 00:45, owner said to continue: all five resumed in their own worktrees with their briefs unchanged. The U1b verifier's mutation helper is not resumed: its head (`4473e1d`) is superseded by the send-back, and the next verification runs mutations again.

## Round 3 — verification and merge (2026-11-17; continued from the Antigravity swarm)

State found: Round 3 = four slices on `feat/round3-opt` (worktree `.claude/worktrees/round3-opt`): R3-A `&mut Enum` / `&Scalar` / `&mut Scalar` ABI flattening (`abi_flat.rs`, `param_use.rs`, `effects.rs`); R3-B slice/str loop scalarization, RawParts bounds peeling, DCE signed-overflow fold and copy→move elision (`slice_iter.rs`, `range.rs`, `dce.rs`); R3-C `#[track_caller]` substitution, trivial-wrapper and sole-caller batching, dead boxed-cell unboxing (`inline.rs`, `partial_escape.rs`, `pipeline.rs`); R3-D shim string/ptr/copy fast paths and i64 peepholes (`shim.js`, `i64_pair.rs`). Each slice was verified alone by a Gemini verifier; octopus merge `fccb8e6`, compiler commit `895ed1c` (adds `range::run_with_interns` wiring, harness and fixture edits), receipt `0a65083`. The final audit of the integrated candidate never ran (Gemini quota exhausted 2026-11-16 11:30). Not merged; `main` = `44161c2`.

Open on the candidate: (1) no independent audit of the combination or of the integration-only wiring; (2) the `js-lane=jir js-opt=on` `test.sh` gate (170/170, 0 skipped on base) was not run, the integrator's run was the default lane (76 passed / 98 skipped) plus 169/174 in abort mode; `cargo test` ran before the last edits; (3) receipt says `timingPassed` with minisearch A/A 1.19, and minisearch rust/js 10.6× → 12.7× (noisy); (4) gzip grew on marked +201 B, ky +319, workbox +141, workbox_direct +150, compact +49 / +37, minisearch +53 (idiomatic −449, fflate −484, idb −929 got smaller); (5) `895ed1c` edits fixture 332 after its verification.

Execution: at most three subagents (user). Verifiers Opus 5.5 high, implementers Opus 5.5 xhigh. Briefs: session scratchpad `briefs/` (`r3v-common.md`, `r3v-correctness.md`, `r3v-gates.md`). Each agent owns its own worktree (the test scripts repoint `build/sysroot`, so two agents in one worktree corrupt each other's runs). Nothing merges to `main` without a passed verification and the user's OK; no push.

- [x] V1 correctness verifier (Opus 5.5 high; worktrees `r3v-head` @ 895ed1c, `r3v-base` @ 44161c2; evidence `.claude/worktrees/r3v-head/build/r3v-correctness/`, `results.json` first; probes/generator in session scratchpad `v1/`) — returned 2026-11-17 ~11:05: **PASS**, no program wrong on the candidate and right on base. Coordinator checked the suite logs: `js-lane=jir js-opt=on` `test.sh --require-core` 174/174 in mires32-unwind and in `JSC_PANIC=abort`, 0 skipped; package setting (release-balanced + `JSC_RUST_MIR_INLINE=on JSC_RUST_OPT_LEVEL=z`) candidate 163/174 unwind and 171/174 abort, base 160/170 and 167/170, shared failures identical. Held: 45 targeted probes × 9 settings (405 runs; 331 native, 74 = base), 14 host-throw probes (84 runs; 81 native, 3 = base), 200 generated programs, 845 witness-checked links, fault rollback 182/184 (2 explained), ~545k shim operations against the base shim, fixture corpus 302/302 identical, committed r3 scripts and unwind-mode-test 56/56 per mode. Fixture 332's edit in 895ed1c only reorders match arms (does not weaken it).
  - Mutations: impure-operand (i64_pair), `needs_clone` (unbox_cells) and range provenance guards are each caught by one unit test; the R3-A `may_unwind` guard is caught only by a decline-reason check in `abi-flat-mut-test.sh` and is shadowed by `boundary.non-call-reference` in every shape tried (no behavioural reproducer). Follow-up: a test that reaches the guard, or a note that it is defence in depth.
  - Hashes: jsc-link and dylib differ from the receipt in both rebuilt worktrees (the dylib embeds the absolute worktree path, 121 occurrences; jsc-link cause not proven); shim matches; restored-source rebuild reproduces its own hashes. The receipt's binary hashes are path-bound, not a reproducibility check.
  - Pre-existing (identical on base), for the queue, not Round 3 blockers: P1 abort mode with opt on: failed bounds/arith assert throws `rust abort` without `#[panic_handler]` (location lost); P2 MIR inline + opt-level z (unwind, and sizez-a): `#[track_caller]` locations wrong (slice index → `core/src/slice/index.rs:414:13`; new fixture 332 fails in that setting); P3 `#[track_caller]` fn through a fn pointer → `TypeError … 'filename'` in every setting; P4 `make_ascii_uppercase/lowercase` lost (documented `&mut str` zombie, silent); P5 relz-a: host-throwing Vec element destructor skips the remaining drops; P6 closure as generic `FnMut` refused under inline-z unwind (fail-closed); P7 `iter_mut()` over `[u8; 0]` aborts with `ptr_eq … no provenance`.
- [x] V2 gates verifier (Opus 5.5 high; candidate `round3-opt`, control `r3v-ctl` @ 44161c2, `r3v-attr-{base,cand,a,b,c,d}`; evidence `.claude/worktrees/round3-opt/build/r3v-gates/results.json`, 57 gate rows) — returned 2026-11-17 ~11:10: **PASS**. Every gate that passes on base passes on the candidate in both modes; every candidate failure is identical on base; nothing base runs is skipped. Qualification gate 174/174 vs 170/170, 0 skipped, both modes (the 4 new fixtures are the only differing rows). Default lane 76/76 + 98 skipped (unwind) is the baseline lane refusing cleanup, base 74/74 + 96; abort 172/174 vs 168/170 (`102_str_lines`, `26_async` over budget with identical bytes on base). Package setting 163/172 vs 160/168 (unwind) and 171/174 vs 167/170 (abort), same failing rows; extra candidate row 332 is the pre-existing inline-z `#[track_caller]` location bug. cargo test, node, chunk, lane-shadow (168/168), unwind-mode, prune-invoke, active-poll (20/20), noop-landing-pads, sroa, slice-iterator, three native differentials, es2018, emit, dedup, check.sh, inventories (1 / 3 / 7–8 OPEN, identical rows): all equal to control. All 14 receipt artifacts reproduce byte-identically (non-incremental rebuild); base receipt 14/14 too.
  - Deviation (accepted, see lessons): gates and artifact builds ran with `CARGO_INCREMENTAL` unset to match the receipts.
  - Receipt: provenance OK; the A/A gate is not enforced on base or candidate (`timingPassed` = checksums agreed; minisearch kept 1.19 with `aaWithinGate=false`); step 5a compares what it says but reads the mutable main checkout and picks the newest sysroot without a key check.
  - Size attribution (gz9 vs base, each slice alone): all growth is R3-D's minimal shim (+150…+370 per bundle: copy / copy_nonoverlapping / compare_bytes / ptr_eq fast paths, str_bytes / str_len caches; program output unchanged). R3-C over-inlines `<Result as Try>::branch` in ky's async closures (+102 gz; the lane report's size measure predicted −161). B and C shrink everything else (idb/idb_lines through C's cell unboxing; fflate through B).
  - Coverage gap: `unwind-mode-test.sh` skips the R3-A fixtures 314–317 ("unknown fixture" in the harness, already so on 5b57f6d), so they run only in `abi-flat-mut-test.sh` (unwind, defaults, opt on).
- [x] Coordinator timing (2026-11-17 11:15–11:22, load 3.8–5.7; artifacts byte-identical to the receipts; drivers and data in session scratchpad `timing/`). Workloads, candidate vs base in one cell, 7 reps, A/A controls 0.99–1.01: fflate 0.947 (−5.3%), marked 1.016 (+1.6%), minisearch unusable (bimodal per instance; fast-mode values equal, ~0.27 ms). Per slice vs base: R3-B fflate 0.887; R3-C marked 0.968, fflate 0.952; R3-A ~1.0; **R3-D marked 1.048, fflate 1.020 (slower)**. Kernels (bench_kernels.rs, receipt settings and harness, 5 reps, Node / JSC, 250 runs all correct): murmur Node 16.0 → 12.8 µs (−20%, all from R3-C; JSC bimodal), enum_state −4% both engines, crc32 unchanged, string_vec_builder within noise (R3-D maybe −3% on JSC). R3-D alone shows no measurable kernel gain.
- [x] DECISION for the user (2026-11-17: user chose "drop R3-D's shim fast paths first, then merge"): merge as is, or first drop R3-D's shim fast paths (all in `de84420`'s `runtime/shim.js` +199/−15 plus `runtime/test/r3-fastpaths.test.mjs`; no compiler code references the new shim state; keep its verified i64 peepholes) — expected from V2's per-slice table: every bundle loses the D column (compact ≈ −320 gz vs base instead of +49, marked ≈ −150 instead of +201, minisearch ≈ −280 instead of +53, ky ≈ +60, workbox / workbox_direct ≈ 0), marked's 5% slowdown goes away. Needs a rebuild (jsc-link embeds the shim in its cost model), re-measurement and re-verification.
  - Follow-ups (not blockers): enforce the A/A gate and replace minisearch's timing protocol; `CARGO_INCREMENTAL` in the sysroot cache key; run 314–317 in `unwind-mode-test.sh`; a behavioural test for R3-A's `may_unwind` guard; ky over-inlining (inliner size measure for async closures); step 5a reads the main checkout; pre-existing P1–P7 above.
- [x] Coordinator re-run of the gate numbers and the timing on a quiet machine (after V1/V2). Superseded by the re-verification and the timing re-run below.
- [x] Implementer (third subagent, Opus 5.5 xhigh; brief `r3-noshim-impl.md`): in `round3-opt` on `feat/round3-opt` from `0a65083`: restore base `runtime/shim.js`, remove `runtime/test/r3-fastpaths.test.mjs`, guard the qualify script's write into the Gemini directory behind `QUALIFY_SCRATCH_RECEIPT`; rebuild non-incremental; gates; list of programs whose output flipped; compiler commit + receipt commit
  - Result: compiler commit `933d5d0`, receipt commit `ba72c43` (no trailers). Every gate passed, and no expectations were edited.
    - jir+opt core: 174/174 in both modes, 0 skipped.
    - cargo test: 1203 passed.
    - Other suites: unwind-mode 56/56, i64-pair differential 18068 cases, chunk 17/17, lane-shadow 168/168.
  - Outputs changed vs 895ed1c (cost-gated decisions scored against the shim text):
    - Core unwind: 7 of 150 programs (21_vec, 274, 29_rc, 300, 72, 94_repr_box_nested with native-box, 101).
    - Core abort: 1 of 150 (274).
    - Artifacts: only compact/release-size changed in its program; it is now equal to release-balanced, as on base.
  - Sizes, gzip-9, total of the 14 artifacts: 353,247 on base, 350,899 on 895ed1c, 347,506 on the new head.
    - Change from base: compact −316, marked −168, minisearch −288, ky +58 (the only artifact still above base), workbox −7, workbox_direct −11.
  - Coordinator checks:
    - The diff `0a65083..933d5d0` covers only the 3 agreed files.
    - `runtime/` is identical to base.
    - The receipt binds `933d5d0` with a clean tree.
    - The Gemini scratch copy is untouched (still Nov 16).
  - Evidence: `round3-opt/build/r3-noshim/`.
- [x] Re-verification: V2 (gates, sizes, receipt, diff review) and V1 (differential re-run of its probes and generated programs against the new head), same agents resumed (sent 2026-11-17 with brief `r3v-recheck.md`; V1 told to focus on the flipped programs, especially 94_repr_box_nested's native-box drop path)
  - [x] V1 recheck: **PASS**, 0 fail reasons. Both sides were rebuilt non-incremental, and jsc-link hashes match the reference and the main receipt.
    - Core suite: 174/174 in both modes. The r3 scripts and unwind-mode (56/56 per mode) pass in both modes.
    - Probes: 441 runs in 9 settings, 96 host-throw runs, 200 generated programs in 3 settings. 0 regressions; every program that differs from native is byte-identical on base.
    - The flipped fixtures are 99/99 identical to base in all 9 settings. The full corpus in sizez-a (where the release-size flat-ABI flip happens) is 151/151 identical to base.
    - The native-box / direct-drop path (fixture 94 variants, Box shapes, drop counts and order, host throws during drops) matches native everywhere. The only exception is a P5 case that is identical on base.
    - 250 decision flips seen in its own probes, 895ed1c → 933d5d0, all harmless.
    - Evidence: `r3v-head/build/r3v-correctness/results-recheck.json`.
    - New pre-existing defects (byte-identical on base; follow-ups, not blockers):
      - **P8:** cloning a tuple `(String, [u64; 2], P)` throws `TypeError: Cannot set properties of undefined`. Under MIR inlining plus opt-level z it is refused at compile time instead.
      - **P9:** `Rc::make_mut` on a shared `Rc<i32>` aborts with "byte view of 12-byte elements, not of 4-byte ones".
      - **P10:** in relz-u, latz-u and sizez-a, `==` on two length-1 `Vec<u32>` aborts in `compare_bytes`.
    - Not re-run: the mutations (the guards are unchanged since 895ed1c).
  - [x] V2 recheck: **PASS**, 0 fail reasons.
    - Gates: all 56 rows have the same exit status on 933d5d0, on 895ed1c and on control. Nothing is newly skipped.
      - jir+opt core: 174/174 in both modes.
      - Default lane, rb+inline+Oz and release-size are the same rows as before.
    - Independent rebuild in `r3v-attr-noshim` gives the same jsc-link `5d7ef812`.
    - The diff covers only the agreed change.
    - Its own flip list matches the implementer's: 7 programs changed in unwind, 1 in abort. Some fixtures change size under other settings without any row changing.
    - The 14 artifacts reproduce the `ba72c43` receipt exactly.
      - compact/release-size: a 4-item flat-ABI group is now selected, and the program equals release-balanced, as on base.
      - ky +58 gz over base is R3-C's `<Result as Try>::branch` over-inlining (follow-up).
    - The receipt binds 933d5d0 with a clean tree, and the scratch copy was not written.
    - The A/A band is still not enforced (pre-existing; follow-up).
    - Evidence: `round3-opt/build/r3v-gates/results-recheck.json`.
- [x] Coordinator timing re-run, new head vs base (2026-11-17 14:23–14:25, load average 3.2–4.4; the user's browser was the main load).
  - Drivers: `scratchpad/timing/ab2.mjs` and `kernels2.mjs`.
    - Bundles were snapshotted into `scratchpad/timing/newhead/` and checked against the receipt's hashes.
    - The new head's kernel program is byte-identical to the 895ed1c compile, and its shim is base's.
  - Workloads (7 reps; p50 ratio, below 1 is faster):

    | Workload | New head vs base | 895ed1c vs base | A/A band |
    |---|---|---|---|
    | marked | 0.982 | 1.016 | 0.96–1.03 |
    | fflate | 0.949 | 0.947 | 0.98–1.01 |
    | minisearch | unusable | unusable | 0.73–1.37 (bimodal) |

    - marked: new head vs 895ed1c is 0.978. The R3-D shim's slowdown is gone.
  - Kernels (5 reps; new head vs base):
    - murmur_u64_4096: 0.803 on node, 0.991 on jsc (895ed1c: 0.799).
    - enum_state_4096: 0.984 on node, 0.988 on jsc.
    - crc32 and string_vec: within ±1%.
  - Data: `scratchpad/timing/ab2-r7.json`, `kernels2-r5.json`.
- [x] Fast-forward `main` 44161c2 → ba72c43 (`git merge --ff-only feat/round3-opt`, 12 commits; the user's OK came with the decision). No push.
  - Before merging: `main` is an ancestor of the branch, Round 3 does not touch the uncommitted `tasks/` / `current_state.md` files, and no commit in the range has a trailer.
- [x] Rebuild the main checkout: `unset CARGO_INCREMENTAL; bash scripts/build.sh --locked`.
  - Only jsc-link recompiled, to `5d7ef812…`, the verified hash. The shim is `0ea2a34e…` (base's).
  - The dylib is unchanged (Round 3 does not touch the codegen crate).
  - The checkout had no keyed sysroot for its dylib in the default unwind mode (a gap from before the merge; see lessons). Built it with `build_sysroot.sh` for both modes.
  - Then ran `SKIP_BUILD=1 RCGJS_WITNESS_CHECK=1 JS_EXTRA_ARGS="js-lane=jir js-opt=on" scripts/test.sh --require-core`: **174/174 in mires32-unwind and 174/174 with `JSC_PANIC=abort`**, 0 skipped. The `build/sysroot` link is unchanged.

### Review (Round 3, 2026-11-17)
- Landed: R3-A (flat ABI for `&mut` enum / scalar), R3-B (loop and bounds scalarization), R3-C (cell unboxing, `#[track_caller]` substitution, small inlines) and R3-D's i64-pair peepholes.
- Not landed, by the user's decision: R3-D's runtime shim fast paths. They added +150…+370 B gzip per bundle for no measured gain.
- Verification: two independent verifier passes on 895ed1c and two more on 933d5d0, all PASS. 10 pre-existing defects (P1–P10) are recorded; none is a regression.
- Effect vs the old `main`:
  - Size, gzip-9, 14 artifacts: −5,741 B in total; only ky grew (+58).
  - Speed: fflate 5% faster, marked 2% faster, murmur 20% faster on node. Nothing measurably slower.
- Follow-ups:
  - Enforce the A/A gate.
  - Change minisearch's timing protocol.
  - Add `CARGO_INCREMENTAL` to the sysroot cache key.
  - Run 314–317 in `unwind-mode-test.sh`.
  - Add a behavioural test for the `may_unwind` guard.
  - Fix ky's `<Result as Try>::branch` over-inlining.
  - Stop step 5a of the qualify script reading the main checkout. That arm now compares against Round 3, not the old `main`.
  - Fix P1–P10.
  - Speed vs hand-written JS (2026-11-17, `scratchpad/timing/kernels3-r5.json`, `ab2-r7.json`): the biggest loss is enum_state, about 125× slower in Node.
    - Its `#[inline(never)] step_state(&mut State)` is exactly R3-A's flat-ABI shape (`mut-enum(6)+scalar-ref(1)`), but the size-scored cost model declines it as `cost.score-no-improvement`.
    - The receipt's enum matrix shows the same pattern with an inlinable helper runs at about 3 µs, on par with JS.
    - Candidate: let the cost model weigh hot-loop calls, not only bytes.
- Removable: the Round 3 worktrees, about 186 GB: `r3v-*`, `r3-*` and `round3-opt`. All their branches are merged. The verifiers' `results*.json` are in their `build/` folders; copy them out first if wanted.
- 2026-11-17 ~03:57: both verifiers ended on a network outage (API `ENOTFOUND`) during their first builds; resumed at ~04:00 with their context and briefs unchanged (same agents, not new spawns), told to confirm the builds completed and re-check any background command before using it. An unfinished verification is not a pass.


## Current-main restoration assessment — 2026-11-17

- [x] Commit the previously uncommitted narrative, task decisions and development-speed record without rewriting their historical content (`0f0407d`).
- [x] Preserve the independent Round 3 initial/recheck verdicts under `research/v2-restoration/round3/`, with original identities and limitations.
- [x] Re-run explicit JIR+optimizer core fixtures on main: 174/174 in unwind and abort; runtime subset 88/88; exact retained package/corpus hashes and sizes 14/14; OSS kernels 58/58 each.
- [x] Re-run strict known-gap inventories: 5/6, 2/5, 0/4; do not convert failures into accepted output.
- [x] Add native-controlled restoration probes: boxed-slice construction/drop, shared Rc::make_mut, constant closure-to-fn pointer. All six off/on compiler runs fail; no Round 3 attribution without a named baseline/bisect.
- [ ] Make semantic Invoke-pruning validation mandatory in ordinary release links (the current `witness::enabled()` conditional still bypasses it without the environment flag).
- [ ] Make missing/out-of-band A/A controls invalidate qualification. Current MiniSearch `aaWithinGate=false` coexists with successful outer receipt status; do not publish it as qualified timing.
- [ ] Bind CARGO_INCREMENTAL in sysroot identity and test mode switching; preserve the non-incremental baseline rather than borrowing incompatible caches.
- [ ] Pin immutable baseline compiler/source/runtime/sysroots for each qualification arm; the mutable `old_main` path is not a historical baseline after a merge.
- [ ] Restore the historically supported positive capability families and qualify current Pro/worker/browser/clean-consumer behavior separately from OSS kernel parity.

Current main and the lost-v2 comparison are consolidated in `RESTORATION-STATUS.md`; `current_state.md` retains its November 12 body under a dated historical heading. This assessment changes documentation/evidence only and pushes nothing.

## Strings: profile, then byte-string experiment (2026-11-17; user request)

Question: how much of marked's and minisearch's time goes to byte-memory string work, i.e. the ceiling for a better string representation? Then prototype js_of_ocaml-style byte strings (a JS string with one char per UTF-8 byte) and measure them against that ceiling.

- [x] P1 Profile (coordinator):
  - Results (2026-11-17, Node 26.8.1, new head `ba72c43` bundles, load average about 4):
    - Tools: `scratchpad/strings/prof.mjs`, `analyze.mjs`, `count.mjs`; profiles in `scratchpad/strings/prof/`.
    - The readable `index.js` runs within 5% of `index.min.js`, so its profiles are representative.

    | Share of time | marked (0.98 ms/call) | minisearch (0.33 ms/call) | fflate (control) |
    |---|---|---|---|
    | string conversion | **53.7%** (str_bytes 24.9, str_len 14.8, bytes_str 14.0) | 9.2% | 0% |
    | byte-memory ops | 13.9% | 22.4% | 11.8% |
    | allocator | 1.3% | **43.6%** (`$rt_dealloc` 32% self) | 1.9% |
    | GC | 1.2% | 9.1% | 0.9% |
    | program code | 27.4% | 14.0% | 85.4% |

  - Per call:
    - **marked** (7,658-char input): 45,069 `str_bytes` calls covering **9.0 M chars** (1,175× the input; 78% cache hits, but a hit still hashes or compares the whole string), 10,769 `bytes_str`, 7,499 `str_len`, 7,180 copies.
      - Cause: `inline_into` evaluates `source[index..].starts_with("**")` / `source[index + 1..].find("](")` at every byte. Each slice becomes a new JS string, then is converted back to bytes, so the work is quadratic. In native Rust those slices are free.
    - **minisearch**: 270 deallocs per call, 210 of them align-1 byte buffers; 1,415 `str_bytes` calls covering 78 K chars.
  - Probe on minisearch: removing `$rt__freed.add(buf)` from `$rt_dealloc` (a WeakSet used for use-after-free detection) takes 0.334 → 0.220 ms (−34%). Removing `buf.length = 0` changes nothing; removing the WeakMap delete is slower.
    - A separate, non-string finding (follow-up: find cheaper freed-block bookkeeping).
  - Upper bounds:
    - marked: eliminating conversions and byte ops would cut up to about 68% of its time (≈0.3 ms per call vs JS 0.47 ms; an upper bound, not a prediction).
    - minisearch: limited by the allocator, not by string conversion.
  - Node `--cpu-prof` on the readable `index.js` bundles of the new head, with the same inputs as the timing protocol.
  - Report self time and inclusive time per category:
    - string conversion: `$rt_str_bytes` / `$rt_bytes_str` / `$rt_str_len` / memo / TextEncoder / TextDecoder;
    - byte-memory ops: copy / compare / slices / alloc;
    - program code;
    - GC.
  - Instrumented counts: calls and bytes converted per run, memo hit rate.
  - fflate as a non-string control.
- [x] P2 Design (brief `scratchpad/briefs/bstr-exp.md`). Byte strings sit behind a link-time switch (`js-str=bytes`, default off and byte-identical to `main`).
  - Phase A is correctness: shim, emission, host boundary and literals, plus non-ASCII / ordering probes.
  - Phase B removes conversions: slicing, `starts_with` / `find` / `len` / `as_bytes()[i]` / `==` are lowered directly.
  - The minisearch allocator WeakSet is out of scope.
  - The coordinator does the timing.
- [x] P3 Experiment (implementer, Opus 5.5 xhigh): worktree `.claude/worktrees/bstr-exp`, branch `exp/byte-strings`, `ba72c43` → `91488e5` (Phase A) → `b0870cf` (Phase B). No trailers, not pushed, not for merge.
  - Switch: `-Cllvm-args=js-str=bytes`, link-only (not in the sysroot key), requires `js-lane=jir`.
  - Implementer's gates, all PASS: core 174/174 in both modes, rows identical on/off; off-mode core JS byte-identical to main; chunk, unwind-mode, native-format, row 55, cargo test, compact 58/58; probes 131/131 = native (on), 8/131 differ (off).
  - Coordinator checked: off-mode marked/minisearch/fflate bundles (`index.js`, `index.min.js`) sha256 = main's; on/off outputs equal on 60 non-ASCII / emoji / lone-surrogate inputs.
  - Results: `bstr-exp/build/bstr/results.json`.
- [x] P4 Verification (Opus 5.5 high): brief `scratchpad/briefs/bstr-verify.md`, verdict `bstr-exp/build/bstr/verify/verdict.json`. **FAIL**, with 2 blocking findings of one cause.
  - **B1:** `js_bindings!` params `Option<&str>` / `(&str, u32)` reach the host undecoded (mojibake, no error). The coordinator reproduced it.
  - **B2:** the same for `js!` captures of `Option<&str>`, tuples, arrays and `&&str`.
  - Cause: `host.rs` `host_argument` decodes only a top-level `&str`.
  - Confirmed:
    - switch-off output = main: all 169 core `.js` files byte-identical against a fresh main build, both modes;
    - Phase B lowerings exact: 3,844 probe lines = main = native, both modes;
    - top-level host boundary complete; aggregate exports hard-refused;
    - all 3 betterThanMain cases.
  - **Two bugs on main**, independent of this work:
    - `chars().count()` aborts on text of 16 bytes or more ("this buffer holds no [E; 4] elements, so it cannot be read at chunk granularity");
    - `String::from_utf8_lossy(b"a")` throws "bad block 1".
- [x] P5 Phase C (implementer): `6e88a2f`, no trailers.
  - **Fix:** a type-directed `host_convert` (`jsc-link/src/lower/bstr.rs`) on every host crossing, both directions. Nested-`str` exports now convert instead of being refused.
  - **New refusals:** `&mut` / raw pointer to `str`; union, closure or coroutine; depth over 12; a host-marker closure taking `str`.
  - **Gates:** all pass. Verifier repros: `bindopt` 6/6, `jscap` 14/14 = main. New repr probes 34/34.
  - **On bundles:** only a never-run `rust_begin_unwind` facade, +6 B.
  - **Coordinator checks:** phaseC-off `index.js` / `index.min.js` = main (marked, minisearch, fflate).
  - **Re-timed (5 reps, timing-cell), on / off / JS:**
    - marked 0.396 / 0.919 / 0.462, on/off 0.424, on/JS 0.871, A/A 0.994;
    - fflate on/off 1.042;
    - minisearch: the cell is unusable (A/A 1.22), as before.
- [x] P6 Re-verification of Phase C: **FAIL**, 3 new blocking findings. Verdict in `verify/verdict.json` `phaseC`; the Phase B verdict is in `verify-phaseB.json`.
  - **Passed:**
    - B1/B2 fixed, both modes; main in abort mode = main in unwind;
    - 59/61 adversarial lines = main;
    - refusals really are hard errors;
    - switch-off: 169/169 core JS = main, both modes;
    - repr probes 34/34 = real main.
  - **Blocking** (the coordinator checked C1–C3 from the run outputs):
    - **C1:** a `str` inside a `Vec` (also `&Vec<&str>`, a struct with a `Vec<&str>` field, an export returning `Vec<&str>`) crosses undecoded. `RawVecInner` erases the element type (`PhantomData`), so `reaches_str` says false. `VecDeque`, `Rc`, `Arc` and `BTreeMap` are probably the same.
    - **C2:** a closure capturing `&str` in a `js!` capture crosses undecoded; `reaches_str` says false for closures.
    - **C3:** object identity is lost. `same_pair(&p, &p)` → `a === b` is true on main, false under bytes, because the converter copies. This breaks CONTRACT.md:134/667 (a reference to an aggregate is its JS object).
  - **Notes:**
    - `[&str; 0]` param/field crashes at run time (`reading 'map'`);
    - newly refused programs: `&mut` / raw / union / `Box<[&str]>` / 14-deep nesting;
    - the nesting limit is really 13/14.
  - **Design point:** main hands the host its internal layout for arbitrary aggregates (`{"vec":{"buf":{"inner":…}}}`). Under bytes, a copy-convert of such a value cannot keep both text and identity, so the exact answer is a refusal by default for anything that is not a by-value value whose whole type is visible. Waiting on the user's call (Phase D vs stop).
- [x] P4 Timing (coordinator, Node 26.8.1, timing-cell protocol, 7 reps; plus steady-state alternating processes). Drivers: `scratchpad/bstr/timing/{ab-bstr,steady}.mjs`, `scratchpad/bstr/kb/kernels-bstr.mjs`.
  - Workloads, median ms per call (switch on / off / JS):
    | Workload | On | Off | JS | Result |
    |---|---|---|---|---|
    | marked | 0.400 | 0.932 | 0.468 | on/off 0.429 [0.423–0.443], A/A 1.011; on/JS 0.856 — **faster than the JS library** |
    | minisearch | 0.249 | 0.249 | 0.022 | steady state, 8 alternating reps: no change. The cell A/A is unusable (1.27). |
    | fflate | 0.564 | 0.542 | 0.178 | +3.7–3.9% (on/off 1.037, A/A 0.993): ASCII-scan of the 61K input at the boundary |
  - Kernels (Node / JSC), on vs off: murmur 0.997 / 1.022, crc32 1.000 / 0.998, enum_state 1.001 / 0.998, string_vec 0.989 / 1.029. All unchanged: owned `String` building still runs in byte memory.
  - Sizes (gzip-9), off → on: marked 9,825 → 7,238 (−26%); minisearch −96 B; fflate +45 B; ky −170 B; compact +90 B.
  - Left over (from the implementer): raw-pointer byte views (marked `escape_slice`, 5,900/call); `push_str` into a baseline `String` (7,180/call); untagged `strip_prefix` / `split` / `trim` / `get` / `&String` patterns; view-abi.

## Allocator: cheaper freed-block bookkeeping (2026-11-18; user chose this before byte strings Phase D)

Byte strings are paused at `exp/byte-strings` `6e88a2f` (verdict FAIL C1–C3; Phase D, refuse-by-default boundary, not started).

- [x] F1 Change: `__rt._freed` becomes a marker symbol written onto the emptied array (`buf[_freed] = true` in `dealloc`, `buf != null && buf[_freed] === true` in `_heap_block`), replacing the `WeakSet` add/has. The use-after-free refusal and its message stay as they are (`guard_use_after_free`, `runtime-shim-perf.test.mjs`, CONTRACT "Freeing"). Worktree `.claude/worktrees/alloc-freed`, branch `fix/freed-marker` from main.
  - Done: `runtime/shim.js` only, +9/−7 lines. A pre-change probe on main's minisearch bundle: WeakSet 0.271 ms, marker 0.180 ms, no tracking at all 0.183 ms (6 alternating fresh processes).
- [x] F2 Gates: build; core suite jir+opt in unwind and abort (abort enforces `.maxbytes`); `node --test scripts/runtime-shim-perf.test.mjs`; `cargo test --workspace`; emitted-JS diff against main (expect only shim text in programs that reach `dealloc`/`_heap_block`).
  - Core suite: 174/174 in unwind and abort (`guard_use_after_free` passes, 1985 B). `runtime-shim-perf.test.mjs` 6/6.
  - Emitted JS vs main: unwind 141/150 identical, abort 149/150. The control (same worktree, main's shim) = main 150/150 in both modes, so the 10 differences come from the shim's member bytes. The cost model prices `Helper(member)` by source bytes; +~44 B across `_freed` / `_heap_block` / `dealloc` flips marginal native-box size-gate decisions both ways (e.g. 107: native box now commits, −304 B; 101: +185 B). Net program bytes over the 10: −163. All pass.
  - `cargo test --workspace` (witness check on) 1203/0; `minimal-shim` + `runtime-shim-perf` node tests 16/0.
  - Workload bundles: `raw/program.js` byte-identical to main for marked/minisearch/fflate; only the 3 shim lines differ (+30 B min, +11–13 B gz).
- [x] F3 Timing: minisearch / marked / fflate new vs main (timing-cell + steady-state alternating), kernels.
  - Timing-cell (5 reps): marked new/main 0.992 (A/A 0.990), fflate 1.005 (A/A 0.995): unchanged. minisearch: the cell is unusable (A/A 0.81).
  - Steady state, 8 alternating fresh processes: minisearch **0.283 → 0.189 ms (−33%)**, every new run below every main run; now ~8.6× JS (was ~12.9×).
  - Kernels (`bench_kernels.rs`) not re-timed: skipped, the workloads cover the change.
  - Commit `a61fca8` on `fix/freed-marker`, no trailers.
- [x] F4 Verification (Opus 5.5 high): **PASS-WITH-NOTES**, nothing blocking. Verdict `alloc-freed/build/freed/verify/verdict.json`.
  - 126 shim probe cases (every `_heap_block` caller × 17 buffer kinds, double free, realloc after free, re-registration): main = branch.
  - The mark is written only onto fresh registered arrays (strict ESM probe too). The minimal shim keeps `_freed` in the closure.
  - Core 174/174 in both modes, emitted JS byte-identical to the coordinator's. The control = main 150/150.
  - Flip mechanism confirmed: the shim grows exactly 42 B (`_freed` +19, `_heap_block` +20, `dealloc` +3). `CarrierEconomics::accepts` refuses any gzip/brotli growth, and the flips sit on that edge (107 brotli 4887→4892, 101 15982→15998, 50 8136→8137).
    - Correction to my summary: not all flips are native-box. 50 is native-vec/string-builder; 110/21/23/30/46 are scalarization/spill.
  - Notes:
    1. The mark is an own enumerable symbol, so `Object.assign` / spread / `__rt.overwrite` of a freed backing array would carry it. This is unreachable in the pointer model (copies are of `buf[off]` elements) and already UB. A non-enumerable `defineProperty` mark closes it, but costs 0.173 → 0.191 ms on minisearch (coordinator probe: orig 0.258), so I kept the plain mark.
    2. Freed arrays now show the symbol to `Reflect.ownKeys` / `inspect`.
    3. `guard_use_after_free.expect_abort` checks only that the program aborts and prints `5`, not the message. Follow-up: an expected-message check in `test.sh`.
- [x] F5 Merge (the user's OK, plain mark): fast-forwarded main `5f9ef93` → `a61fca8`. Main was rebuilt (`target/release/jsc-link` sha256 `61ad1f17…`) and the keyed jir+opt sysroots rebuilt for both modes. Gate: 174/174 in unwind and in abort; main's abort-mode core JS = the branch build (150/150). Not pushed. The `alloc-freed` worktree and `fix/freed-marker` branch can now be removed (merged).

## Byte Strings: complete safe boundary + size & speed optimization (2026-11-19; Stage 5: fixing re-audit blockers + release-* default)

Cross-compiler survey (js_of_ocaml 6.4, Melange 7.0 / ReScript 12.3, Scala.js 1.22, Kotlin/JS 2.4, GopherJS, dart2js) and staged plan in `byte_strings_plan.md`. Branch `feat/byte-strings` (`5d69917`, integrated code SHA `41444a0`), worktree `.claude/worktrees/bstr-feat`, rebased onto `a61fca8` (`0` overlapping files with `46383cb`; ready for fast-forward/merge on user OK).

- [x] Stage 1 — Rebase `exp/byte-strings` onto `main` (`a61fca8`) + complete safe host boundary (`bstr/s1-boundary` `e23fea9`, verifier **PASS 32/32**, `build/bstr-verify/stage-s1/verdict.json`):
  - [x] Traverse `TypeKind::Adt { args }` (excluding `String`), `Closure`, `Coroutine`, `CoroutineClosure` in `reaches_str`; guard `descriptor.layout.zst` (`[&str; 0]`); depth cap 64.
  - [x] Synthesize deduplicated per-`(TypeId, Crossing)` `$bstr$out$*` / `$bstr$in$*` converter items using direct literals (`0` `Object.assign` across `46` helpers); skip `rust_begin_unwind` facade.
  - [x] Add `__rt.bstr_ref` / `__rt.bstr_ref_in` (`WeakMap`-cached live `Proxy` view) with distinct `{ k: "slice" }` vs `{ k: "slot" }` descriptors for `&T`, `&mut T`, `&Cell`, `Slot`, `FatSlice`, `Box<T>`, `Rc<T>`/`Arc<T>`, `RawVec<T>` (`Vec<&str>`, `VecDeque<&str>`), `BTreeMap`, and closure upvars + callback `str` thunks.
  - [x] Independent verification (round 1 FAIL on heterogeneous tuple `Slot` cache poisoning → fixed in `e23fea9`, round 2 **PASS 32/32**): C1–C3 + `[&str; 0]` fixed, core `174/174` unwind + abort, switch-off `169/169` SHA-256 identical to `a61fca8`.
- [x] Stage 2 — Codec speedups, interprocedural `is_ascii` elimination & slice outlining (`bstr/s2-codecs` `bd1e142`, verifier **PASS 27/27**, `build/bstr-verify/stage-s2/verdict.json`):
  - [x] Two-phase interprocedural `KnownAscii` fixpoint (`jsc-link/src/analysis/ascii.rs`) eliding `bstr_out` on ASCII-proven `str`, immutable `HostString`, and unmutated `Option<&str>`, plus `StringAsStr` post-repr inline guard (`jsc-link/src/opt/inline.rs`).
  - [x] `_bstr_is_ascii` (`< 24` char loop + `>= 24` regex) and `bstr_out` (`<= 48` pure-JS UTF-8 decode + `49..=1024` shared `Uint8Array(1024).subarray` scratch) in `runtime/shim.js`.
  - [x] Single-pass `__rt.bstr_utf8_bytes` fusion for `bstr_bytes(bstr_in(x))` with `typeof !== "string"` `TypeError` validation and dominance/purity guard.
  - [x] Outlined `__rt.bstr_sub*` / `__rt.bstr_get*` / exact 25-codepoint Unicode `White_Space` `__rt.bstr_trim*`, `StrStrip*`/`StrGet`/`StrPartialCmp` tags, and non-ASCII constant hoisting.
  - [x] Independent verification (round 1 FAIL on `StringAsStr` `HostString` inline & `bstr_utf8_bytes` validation/dominance → fixed in `bd1e142`, round 2 **PASS 27/27**).
- [x] Stage 3 — Raw-pointer byte views & byte-string-backed owned `String` (`bstr/s3-strings` `1d7f327`, verifier **PASS 29/29**, `build/bstr-verify/stage-s3/verdict.json`):
  - [x] Closed intra-`Func` `byte_view_analysis` (`as_ptr()`/`RawView` + offset + loop block params + `offset_from` + `Deref` → `s.charCodeAt(i)`), including MIR-inlined `String.vec`.
  - [x] Stdlib-whitelisted `BstrStringOp` (`IsEmpty`, `Reserve`, `ReserveExact`, `AsBytes`) + real `alloc` `String::reserve_exact` CFG/MIR-inline support so owned `String` builders commit `{ text, cap, len }`.
  - [x] Independent verification (round 1 FAIL on real `reserve_exact` CFG & stdlib path guard → fixed in `1d7f327`, round 2 **PASS 29/29**).
- [x] Stage 4 — Integrated qualification & `CONTRACT.md` update (`bstr/s4-integrated` `41444a0` + doc align `5d69917`, verifier **PASS 60/60**, `build/bstr-verify/stage-integrated/verdict.json`):
  - [x] Switch-off byte-identity vs `a61fca8`: `169/169` `.js` files in unwind and abort + all workload/`compact` bundles SHA-256 identical; `runtime/shim.js` strictly additive (`0` existing lines changed); `cargo test --workspace` `1,221/0`; core `174/174 × 4` (`RCGJS_WITNESS_CHECK=1`).
  - [x] Qualified `--str bytes` vs `off` metrics (`41444a0`):
    - **`marked`:** `min` `26,000 → 11,036 B` (**`−57.6%`**), `gzip-9` `9,838 → 4,208 B` (**`−57.2%`**), `brotli-11` `8,655 → 3,744 B` (**`−56.7%`**); Node `p50` **`0.9997 → 0.1354 ms` (`0.135×` = `7.38×` faster)**; `str_bytes 45,069 → 1`, `copy_nonoverlapping 7,180 → 0`, `alloc/dealloc/realloc 2/2/11 → 0/0/0`.
    - **`compact` (`58/58`):** `min` `155,711 → 153,271 B` (**`−2,440 B`**), `gzip-9` `47,866 → 46,899 B` (**`−967 B` / `−2.0%`**), `brotli-11` `36,806 → 36,015 B` (**`−791 B` / `−2.1%`**).
    - **`minisearch`:** `gzip-9` `14,325 → 14,290 B` (`−35 B`), `brotli-11` `12,765 → 12,695 B` (`−70 B`); `p50` **`0.2552 → 0.1684 ms` (`0.660×` = `1.52×` faster)**.
    - **`fflate`:** `gzip-9` `4,989 → 5,069 B` (`+80 B`), `p50` **`0.5695 → 0.5654 ms` (`0.993×`)**; `bstr_in`/`_bstr_units`/`_bstr_sub_reset`/`rust_begin_unwind$bstr` all tree-shaken (`0`).
    - **`ky`:** `gzip-9` `9,612 → 9,655 B` (`+43 B`).
  - [ ] Merge `feat/byte-strings` onto `main` and decide whether to flip `release-*` profiles to `js-str=bytes` by default (**awaiting user OK**).

## GopherJS comparator family (2026-11-18)

Goal: add GopherJS (Go → JS) as the eighth comparator family, OSS and Pro, built, checked and sized exactly like
rescript / melange / jsoo / scalajs / dart / kotlin / elm (`docs/COMPARATORS.md`). Work happens on main, in the main
checkout. Nothing is committed until verification passes and the user says OK.

- [x] G0 Toolchain. `experiments/tools/setup-go-comparators.sh` (`npm run setup:comparators:go`) installs Go 1.21.13
  (go.dev archive, SHA-256 pinned per host) and GopherJS v1.21.0 (latest, 2026-06-26; needs a Go 1.21 GOROOT) under
  `.toolchains/go`. macOS fix: Go 1.21's internal linker omits `LC_UUID`, which dyld refuses in a binary linking system frameworks
  (GopherJS links CoreFoundation/Security; a pure-Go binary runs), so GopherJS is linked with
  `-linkmode=external` and re-signed ad hoc. A hello-world compiles (`-m`: 52 KB) and runs under Node.
- [x] G2a Runner registration: `gopherjs` in `run-comparator-builds.py` (LANGUAGES, `.go`/`go.mod`/`go.sum` inventory,
  `MIRES_GO_ROOT`), `check-comparators.py`, `measure-comparators.mjs` (production variant `default`),
  `fingerprintjs-compare/ports.json`.
- [x] G1 OSS port `experiments/fingerprintjs-gopherjs` (implementer report; awaiting verification). One Go `package main`,
  3,639 lines; `uint64` Murmur; async on Promises/MessageChannel, no goroutines; no embedded JS. Runner PASS; package
  3/3, kernels 58/58, seeds 477/477; SSR-safe; differential 0 normalized diffs in Chrome and Firefox (also the min ESM
  and `run-feature-parity`). Safari not driven: needs Safari ▸ Develop ▸ "Allow Remote Automation" (user setting).
  `fp.min.js` 201,074 / gz 57,341 / br 47,845; runtime floor 42,619 / 14,111 / 12,707. GopherJS bugs found and
  worked around: `math.Trunc(4294967295) == 1` (32-bit int path), lone high surrogates mangled by string conversion,
  `throw null` passing through `defer` frames.
- [x] G2c Shared static audit fix (`fingerprintjs-compare/tools/check-feature-parity-static.mjs`): the IIFE sandbox
  injected host intrinsics (`Array`, `Object`, `Promise`, …) into a fresh V8 context, a split-realm global no browser
  has; GopherJS compares slice constructors against `Array` and broke. Now only host APIs the context lacks are
  injected. Before/after on all 17 rows: 16 identical (timestamps aside); gopherjs import error → contract PASS,
  strict PASS, 0 metadata diffs.
- [x] G3 Pro port `experiments-pro/fp-pro-gopherjs` (implementer report; awaiting verification). 6,611 Go lines in
  22 files; no goroutines; no embedded JS (`maxEmbeddedJsLines: 0`). Runner PASS; package 5/5; validation PASS; SSR
  ESM+CJS; ownership `PASS target=6633 js=13 embedded=0`; strict differential 0/0/0 on Chrome 158 and Firefox 160
  (144/144, 145/145 fields); Safari not driven. `fp.esm.min.js` 297,862 / gz 79,647 / br 63,908 (oracle 218,567 /
  85,504 / 70,741). Additive Go support in `experiments-pro/shared/` (ownership detector, language-agent, port lists),
  with a before/after identity proof. Open: `compile.mjs` rewrites the GopherJS prelude (global probe → `globalThis`,
  no `fs` shim, local `js.Module`) and renames labels for a Rollup 3.30 bug, while the OSS port ships the prelude
  untouched, so the two follow different policies. Process slip: the implementer spawned 3 sub-implementers (lesson
  recorded).
- [x] G2b Docs: `docs/COMPARATORS.md` (family rows, setup, emit policy, verified results, sizes, GopherJS defects),
  `docs/comparator-toolchains.json`, `fingerprintjs-compare/{CONTRACT,toolchains}.md`, `experiments/README.md`,
  private `experiments-pro/README.md`. `.gitignore` already covers the new `dist/` / `build/` / `.toolchains/`.
- [x] G4a OSS verification (2026-11-18): **FAIL** on 2 undocumented items; all 7 headline claims re-ran exact
  (package 3/3, kernels 58/58, seeds 477/477 on min *and* readable ESM; differential 0/0 Chrome/Firefox on both ESMs;
  feature parity 0/0; sizes exact; three GopherJS bugs reproduced; clean-room toolchain + 4 builds byte-identical;
  `check-comparators --suite oss` 51/51 across eight languages; static-audit fix confirmed, 16 rows identical).
  - B1 `withIframe` removes the iframe twice when `removeChild` throws after success (`src/dom.go:90-96` vs `fail`
    at 39-46); oracle removes once. Sent back; **fixed** (success path removes once and rejects directly on a throw); coordinator
    re-ran the probe on esm / esm-min / iife: oracle = port (1 attempt, `rm1`); package/kernels/seeds PASS. New sizes
    `fp.min.js` 201,163 / 57,372 / 47,821. Docs: deviations 1, 3, 5, 6, 7, 9 corrected or added.
  - B2 the facade binds `$global` to `globalThis`, so the prelude calls a page-global `require('fs')` /
    `require('util')`. Closed as documentation under the shared emit policy (NOTES deviation 1); `GOCACHE` moved
    in-repo; dist byte-identical.
  - Notes: TextDecoder needed on import; async exports constructible; BigInt seed accepted; surrogate bug wider; my
    LC_UUID wording overstated (fixed: dyld refuses only framework-linking binaries; setup script also retries
    downloads now).
- [x] G4b Pro verification (2026-11-18): **PASS-WITH-NOTES**, no blocking. Build deterministic; package 5/5;
  validation PASS; SSR; ownership PASS; Chrome 0/0/0 first run; one timing-sensitive Firefox signal flaky under host load (GopherJS 4/10,
  Scala control 3/5 in the same window); sizes exact; shared-tool changes additive, 10 ports identical; pro checks
  16/16. Emit rewrites: probe + `module` binding needed, the two `fs` rewrites not; label renaming forced by Rollup 3.30
  tree-shaking (minimal repro). The OSS wrapper works for Pro (+787 / +91 / +164 B).
  - **Decision (coordinator): one emit policy for both ports.** The GopherJS emit ships untouched plus loading-only
    packaging (wrapper function; Pro keeps the label renaming). GopherJS runtime side effects are documented, not
    patched out: `globalThis.fs`, `stackTraceLimit`, page-global `require('fs'|'util')`. This closes OSS B2 as
    documentation. Sent to both implementers along with: Pro real `typeof` (toStringTag spoof), the 62 dead lines,
    VERDICT notes (one signal stubbed on WebKit/Android as in Scala/Rust, deferred-start timer, null-arg errors, cache
    shape, the Firefox flake, detector false negatives 12/18; details only in the private VERDICT.md), and `GOCACHE` inside `.toolchains/go/cache`.
  - Privacy (local only): earlier Pro builds wrote 4 entries into `~/Library/Caches/go-build` (paths in the Pro
    verdict.json). Told the user; not deleted.
- [x] G4c Pro round 2 (PASS-WITH-NOTES, final): emit inside `compiled.js` = raw `gopherjs build -m` byte-for-byte
  apart from the wrapper and the pure label renaming (225 labels, 471 jumps); `$typeOf` / `$instanceOf` / `$id`
  compile to direct prelude calls, and both toStringTag spoof cases match the oracle; Go 6,467 lines, ownership
  `target=6489 embedded=0`; Chrome/Firefox 0/0/0, the flaky Firefox signal 4/4; `fp.esm.min.js` 298,020 / 79,506 / 63,897 (system gzip).
  Whole suite: `check-comparators --suite all` **67/67** (OSS 51, Pro 16); `measure-comparators` 25 rows.
- [ ] G5 Report with sizes next to the other families (done in COMPARATORS.md); **user OK before any commit**.

## Library matrix: faithful ports × all compilers (2026-11-18)

Goal: a matrix of real JS/TS libraries × compilers. Each cell is a **faithful port** of the library's algorithm for a
defined API subset, checked for exact output parity against the pinned npm package, then sized and timed against that
package (with an A/A noise control). User decisions (2026-11-18): faithful ports (not the reduced recreations); all
ten workloads; research further candidate libraries now, in parallel.

**Scope.** The ten `browser-workloads` entries map to 7 libraries plus a composite:
- minisearch 7.2.0, marked 18.1.1, fflate 0.8.3 (already pinned);
- idb, Ky, uPlot, Workbox (pins to choose: newest release ≥ 2 weeks old);
- Explorer, a composite rebuilt per language from that language's own ports.

`idb_lines` and `workbox_direct` are Rust-only diagnostic variants of idb / Workbox, so they stay in `browser-workloads`
and are not separate rows. The existing reduced corpus stays as it is: it is a compiler diagnostic, not a matrix row.

**Languages (9 compiled + the JS original as the baseline):**
- Rust via mires;
- ReScript, Melange, js_of_ocaml, Scala.js, Dart, Kotlin/JS, Elm, GopherJS.

Toolchains are the existing comparator setups. Melange and js_of_ocaml may share one OCaml source with per-compiler
interop modules: the same program through two compilers.

**Rules carried over from the FingerprintJS comparison:**
- idiomatic code in each language;
- each family's allowed-techniques row;
- no product JS, except Elm's allow-listed JS host and its "mostly JS" label;
- compiler emit untouched, plus loading-only packaging;
- the production profile per family;
- the ports change no compiler source;
- a Rust port that hits a mires bug reports it with a minimal repro, and does not work around it silently.

**Cost.** About 63 ports (7 × 9), each roughly FingerprintJS-sized (1–3k lines), each with independent verification. At
≤ 3 agents at a time, that is many waves. The harness and all toolchains are proven end to end first, so no expensive
port starts on broken plumbing.

- [x] L0 Harness, `experiments/library-matrix/` (implementer report; **verification running**).
  - Contents: registry `matrix.json`; per-language production builders; build runner with receipts; exact checker
    (string / bytes / strict JSON); N-arm timing (triplet port-a / JS / port-b in a Williams order, so port/js and A/A
    come from one process or page; `--design pairs` gives the old two-cell form); sizes; `MATRIX.md`; a `floor`
    identity library for uniform runtime floors.
  - Pilot results: 9/9 builds, and a clean rebuild is byte-identical; 9/9 exact parity in Node 26.8.1, Chrome 158 and
    Firefox 160 (Safari unavailable); bench 27/27 cells, A/A 0.95–1.05 at load 11–13; negative controls (wrong
    output, stale `dist/`, failed and timed-out builds) all refused.
  - Open, for the verifier: the Rust profile (the harness uses browser-workloads' `release-size`, MIR inline off;
    the FingerprintJS Rust uses `release-balanced`, and compact uses MIR inline on); the Elm port-sleep emit patch;
    Melange `emit_stdlib`; drift since the docs (Dart 3.14.0, scala-cli 1.19.0).
- [x] L0v Harness verification: round 1 FAIL (4 blocking). Sent back with exactly B1–B4. The implementer reports all
  fixed:
  - check protocol /3 runs first / repeat / rewritten-in-place, comparing outputs and input state;
  - bench /2 uses a fresh clone per call, plus random and last in-sample output checks;
  - bench and the matrix are gated on a current check pass;
  - `libraryBinding` covers corpus, reference, normalisation and output digests;
  - Kotlin uses `-XX:hashCode=2` in-process (8/8 identical).
  - Pilot after the fixes: 9/9 identical builds, 9/9 parity in 3 envs, 27/27 cells, A/A 0.90–1.07.
  - Round 2: **PASS-WITH-NOTES**. All four controls refused. Kotlin 6/6 identical; `hashCode=2` picks one of the
    compiler's natural outputs, a JVM setting, not an emit patch. Pilot check 9/9 × 10/10 in 3 envs; bench 18/18,
    A/A 0.93–1.05.
  - Residuals:
    - content-cache memoisation is black-box undetectable (benched 0.01);
    - in-sample checks catch rare divergence probabilistically (~1.3% of calls verified);
    - dynamic `import()`, normalisation imports and the harness code are not bound by digest.
- [x] L0h Harness hardening. Round-3 verification: **PASS-WITH-NOTES**.
  - Every mechanism held on the controls: the eval import is caught in 3 envs; the microtask hang is killed and the
    browsers recover; a real Rust panic throws; the image / prefetch / iframe-realm requests are caught; 24 samples
    give 8/8/8 Williams positions; warm calibration gives 17–21 ms samples; the variant is neutral between arms.
  - Medium follow-ups:
    - R3-1 a prefix cache evades the append-only variant;
    - R3-2 a globalThis cache shared across instances (A/A 0.01 or 97 still `ok`);
    - R3-3 the browser request log is time-attributed and blamed an innocent port.
  - Low: process.binding / data: import / wasm gaps; symlink and run-time file-read binding escape; HARNESS_CODE
    misses registry.mjs and cells.mjs.
- [ ] L0f Final harness round (sent): fix R3-1…R3-6 and write the threat model in the README (catches accidental
  divergence and common shortcuts; not a sandbox against adversarial ports; source review plus the lint are part of
  acceptance). After this the harness is frozen except for bugs.
- [ ] (Former L0h item list, kept for the record:) Protocols are now check /4 and
  bench /3; receipts are bound to the harness code by digest.
  - Caching: a bench-variant hook (the content-cache control goes from 0.018 to 1.39 with it) plus a module-state
    lint.
  - Binding: crypto-seeded picks; the module graph follows static and literal dynamic imports; package-dir,
    normalisation and variant digests; a same-major Node / browser policy.
  - Guards: the both-threw loophole is closed; the runtime guard blocks network APIs and Node module loads (browser
    gap documented); call timeouts kill hung cells (5 s); Rust panics throw.
  - Reporting: the lexer-based embedded-JS counter plus an eval-refs column; the comparator Elm formula; warm
    re-calibration; 24 samples.
  - Pilot 27/27 cells with the variant, A/A 0.98–1.15; selftest 15/15.
  - mires note: the runtime shim's `__rt._str_memo` (a content-keyed host-string memo) could favour Rust on fixed
    inputs; the variant neutralises it.
  - The items it was sent:
  - a no-memoisation rule plus a bench-variant hook and a module-level-cache lint;
  - a crypto-seeded random pick;
  - binding by digest for dynamic imports, normalisation imports and the harness code, plus Node and browser
    versions;
  - the both-threw loophole;
  - a runtime import guard;
  - embedded-JS counter fixes;
  - the Elm formula;
  - warm calibration and balanced sample counts;
  - `--receipts` relocating the work dir;
  - Rust panics failing fast, plus per-call hang timeouts.
- [x] K1 FingerprintJS Kotlin comparator (user OK 2026-11-19): `gradle-production/gradle.properties` now has
  in-process + `-XX:hashCode=2`.
  - The first rebuild was Gradle UP-TO-DATE (no recompile), so the compile caches were cleaned (toolchains kept).
  - 3 clean builds are byte-identical.
  - Against Nov-8, production differs only in 2 reordered interface lists; raw bytes are equal, gzip +1 (29,232).
    The Toolchain variants and Pro were unchanged.
  - Kotlin checks: 12/12 PASS. Sizes regenerated. Documented in COMPARATORS.md. Not committed yet.
- [ ] M1 mires fixes for the 5 matrix bugs (user OK 2026-11-19). Implementer **done**; **verification running**.
  - Branch `fix/matrix-minisearch-bugs`, HEAD `d93c692`, worktree `.claude/worktrees/mires-matrix-bugs` (0.82 GB).
  - Commits: `95ca59e` (1, aggregate container liveness in both lanes), `fed7d0f` (2, the SliceView shortcut only
    for real array refs), `3b84f6c` (3, sysroot patch 0014 disables in-place collect), `c153828` (4,
    `niche_to_scalar` encode), `d93c692` (5, `__rt._bytes` in compare_bytes).
  - Gate 174/174 → 180/180 in both modes; check.sh exit 0 (1,203 cargo tests); runtime 88/88; 0 of 150 core and 0 of
    19 example outputs changed; fixtures 340–345 fail before and pass after.
  - Gate gap: the MIR inliner is never on in `test.sh` (only side scripts).
  - Nearby, not fixed (M2 candidates):
    - a silent miscompile: the `NonZero::new(5)` const decodes to `{}` under inline + `-Oz`;
    - `(1..6).map(Wrapped)` refused;
    - `Box<str>::from` drop refused;
    - the rest of the bug-5 class (byte copy / write_bytes / add through the alloc pointer);
    - multi-niche transmutes;
    - coroutine clone shims;
    - side effect of the bug-3 patch: `capacity()` after collect.
  - **Verification (2026-11-19): APPROVE WITH CHANGES.**
    - Reproduced: gate 180/180 in both modes, 0 changed outputs; the inline-on gate is 176/180 on head vs 170/180 on
      base (4 pre-existing failures: 15_intrinsics, 10_panic, 15_ptr, 332_r3_assert_tc_panic); all 5 original repros
      pass on head; the MiniSearch port is 58/58 on head (+513 B).
    - Sent back:
      1. async-closure clone (the `CoroutineClosure` arm in both `fresh_container`s);
      2. narrow patch 0014 to different-type collects only (the blanket version costs +39–73% time on collect-heavy
         code);
      3. correct the false "round-trip" claim for niche transmutes (signed-niche decode is broken, pre-existing;
         queued).
  - Round 2 (implementer, HEAD `eb24b87`):
    - `34d7b4b`: async-closure container, plus fixture 346;
    - `92514de`: patch 0014 narrowed to same type + same size, with 343 proving reuse by capacity;
    - `eb24b87`: round-trip claim corrected.
    - Gate 181/181 in both modes; 0 changed outputs. **Re-verification running.**
- [ ] M2 mires follow-up queue (from M1 implementer and verifier; all pre-existing on 46383cb unless noted). New:
  dropping an async closure that captured a `Vec<T>` throws `native_vec_set_len … buf.fill is not a function` (not
  checked on base).
  - **Silent miscompiles:**
    - the `NonZero::new(5)` const decodes to `{}` under inline + `-Oz`;
    - `[u16]` read through a `u8` view returns elements (`Hash` for `Vec<u16>` is wrong in JIR, throws in v1);
    - byte arithmetic on a retyped allocation pointer reads whole elements (33 vs 22);
    - v1 transmute `NonNull<u8>` → `usize` returns 0;
    - signed-niche decode (`niche_from_scalar` compares 254, not -2).
  - **Refusals:**
    - `(1..6).map(Wrapped)` under inline + `-Oz`;
    - `Box<str>` drop (any source);
    - `Rc::make_mut` on a shared `Rc<(String, u32)>`;
    - `as_chunks` / slices of `Vec<[u8; 3]>`;
    - coroutine clone shims (unstable);
    - multi-niche transmutes.
  - **Infra:**
    - the gate never runs MIR-inline-on (4 inline-on core failures exist);
    - the cost model prices shim helpers by unminified source bytes, so a few bytes of shim text can flip flat-ABI
      choices (price by minified size).
- [x] L1f minisearch contract fixes: the verifier's letter-permutation bench variant is registered; 9 extra cases
  (58 total); 80/80 MiniSearch mutants caught (coordinator re-ran: 80 caught, only identity escapes); Math.log facts,
  `-0` ids, `removeAll(null)` and `fields: []` corrected.
  - Rust: 58/58 in 3 envs; 318,135 / 74,245 / 59,654 B.
  - Bench with the variant, 24 samples: Node index 8.99× / search 11.94×; Chrome 6.56 / 9.72; Firefox 6.19 / 8.45;
    A/A 0.92–1.09.
- [x] L1c minisearch, other 8 languages. **All 9 languages verified.** All registered in `matrix.json` by the coordinator, so agents never touch
  it. Generic brief: `scratchpad/briefs/matrix-port-template.md`.
  - ReScript (implementer report, verification pending):
    - 1,367 lines; 58/58 in 3 envs; build reproduces;
    - 19,642 / 6,476 / 5,930 B, about the JS reference (20,006 / 6,419 / 5,789);
    - bench port/js 0.90–1.19 at load 22–53 (rough), A/A 0.90–1.01; lint 0 / 0 / 0.
    - Finding: ReScript 12's `Iterator.find` (ES2025 iterator helpers) is slow in V8; plain loops fixed it.
  - OCaml (implementer report, verification pending): one core of 6 byte-identical files for Melange and jsoo; only
    `host.ml` and the entry differ.
    - Melange: 1,355 lines; 58/58 in 3 envs; 51,339 / 14,251 / 12,700 B; bench Node 1.62 / 1.36, Chrome 1.52 / 1.33,
      Firefox 0.69 / 0.86.
    - jsoo: 1,362 lines; 96,987 / 31,920 / 27,993 B (floor 67,208).
    - Findings: Melange `Hashtbl` pulls in Printf / CamlinternalFormat (one table ≈ 65 KB min; switching to
      `Map.Make` halved Melange's size); Melange's arity checks on stdlib higher-order calls cost ≈ 20% of
      bench-index.
  - **Harness bug fixed by the coordinator:** `languages/jsoo/build.mjs` failed the whole build when the
    diagnostic-only `--no-runtime` attribution compile hit js_of_ocaml 6.4.1's "Unresolved internal primitive
    %int_mul / %int_div / %int_mod". It is now caught and recorded as `runtime: null` plus `runtimeUnavailable`; the
    stderr is in the build log. Repro: `build/library-matrix/jsoo-bugs/no-runtime-int-arith/`.
    - Re-ran officially: jsoo minisearch / pilot / floor builds PASS (pilot and floor sizes unchanged); minisearch
      jsoo 58/58 in 3 envs; pilot 10/10 and floor 3/3 in 3 envs.
    - jsoo bench (load ≈ 10): Node 1.43 / 1.37, Chrome 1.41 / 1.21, Firefox 0.77 / 0.92; A/A 0.97–1.07.
  - **Wave 1 verified:** ReScript ACCEPT; Melange and jsoo ACCEPT-WITH-NOTES; the jsoo builder fix
    ACCEPT-WITH-NOTES.
    - Reproduced: byte-identical builds; 58/58 in 3 envs; bench within ~10%.
    - Fuzzer: 5,300 random operation scripts, 0 differences.
    - Deviations: only contract-allowed (the fuzzy walk stopping past the matrix costs the reference 0.8%) or
      equivalent; the perf rewrites are idiomatic.
    - Notes handled by the coordinator: jsoo NOTES.md rewritten for the official results; `runtimeUnavailable` no
      longer embeds absolute paths (rebuilt, re-checked 3 envs).
    - Noted: the jsoo runtime installs a global uncaughtException / error handler at import (a family trait).
  - Scala.js (implementer report, verification pending in wave 2):
    - 970 lines; 58/58 in 3 envs; reproducible build;
    - 306,336 / 77,766 / 62,192 B (floor 11,335 / 3,604; the Scala collections library dominates);
    - bench Node 2.06 / 1.68, Chrome 1.81 / 1.47, Firefox 1.55 / 2.13; A/A 0.93–1.04;
    - lint module-state 13, all class-field false positives (the lint can't tell class fields from `object`
      fields).
    - **Contract fix by the coordinator:** the Scala.js guidance said `java.util.regex` `\p{Z}` / `\p{P}`, which
      works only on ES2018 links; the builder's ES2015 (the comparator production default) throws
      PatternSyntaxException. Now points at `js.RegExp(..., "u")` and forbids raising the ES version per port.
    - Watch-out for future corpora: MiniSearch's timer-driven vacuum continuation can crash after `run` returns if
      later ops mutate the tree mid-batch.
  - Kotlin (implementer report, verification pending in wave 2):
    - 1,162 lines; 58/58 in 3 envs; `--clean` rebuild identical;
    - 71,755 / 23,521 / 20,518 B; bench Node 2.82 / 2.01, Chrome 2.40 / 2.16, Firefox 1.26 / 1.52 (A/A 0.95–1.10);
      lint 0. Its LinkedHashMap's string hashing dominates.
    - **Harness bug fixed by the coordinator:** the Kotlin builder's Gradle reported the compile UP-TO-DATE without
      recompiling, so "reproduced" proved nothing. Added `--rerun-tasks`. Two forced rebuilds give the same hashes
      for minisearch / pilot / floor; Node checks pass.
    - Kotlin guidance added to the contract (-0 vs 0 compare, `round` vs `roundToInt`,
      ConcurrentModificationException, callable-reference allocation, `Regex` passing `u` to the host).
  - Dart (implementer report, verification pending in wave 2):
    - 1,371 lines; 58/58 in 3 envs for min and readable; rebuild identical;
    - 99,208 / 33,799 / 29,377 B (floor 38,726 / 13,118);
    - bench Node 2.09 / 1.59, Chrome 1.86 / 1.57, Firefox 1.69 / 2.18; A/A 0.99–1.02;
    - module-state 1 (MiniSearch's own module RegExp constant).
    - Findings: dart2js `as` casts go through the general RTI check while `is` / type patterns are `instanceof` (≈ 11%
      of indexing); generic node classes cost reified checks; `Map.[]=` covariance checks remain at `-O2`.
    - Fuzz: 3,000 scripts, 0 diffs.
    - It hit an API timeout mid-run and was resumed.
  - Reference hazard confirmed by two ports: MiniSearch's async vacuum continuation can reject unhandled after `run`
    returns (about 340 / 1,500 fuzz scripts). The corpus does not trigger it; future corpus cases of that shape could
    kill a reference cell.
  - GopherJS (implementer report, verification in wave 2):
    - 2,079 lines; 58/58 in 3 envs for min and readable; `--clean` rebuild identical;
    - 134,177 / 35,615 / 30,804 B (floor 41,652 / 13,739);
    - bench Node 2.84 / 2.69, Chrome 2.32 / 2.70, Firefox 1.72 / 2.03 (from 7.6× / 7.2×); lint 0.
    - Text as `[]uint16`; native Go `unicode` tokenizer; host `toLowerCase` for non-ASCII.
    - GopherJS findings (repros in `build/library-matrix/gopherjs-bugs/`):
      - `-m` makes `&slice[i]` ≈ 20× slower (`$indexPtr` defineProperty) — an upstream candidate;
      - calls through function values or interfaces make callers blocking state machines, so it avoided closures,
        `sort.Interface`, `slices.*Func` and `strconv`. **Idiom judgement is with the wave-2 verifier.**
    - **Contract fix by the coordinator:** the tokenizer rule said native tables must equal `separators.json`
      (Unicode 17), while the GopherJS guidance allowed Go's Unicode 15 tables. Now: Unicode ≥ 15.0 standard tables
      qualify, and the 14 code points new in Unicode 16 / 17 are outside the parity domain (none is in the corpus).
      README GopherJS hazards updated.
  - **Wave 2 verified:** Kotlin ACCEPT; Scala.js, Dart and GopherJS ACCEPT-WITH-NOTES; no blocking.
    - Reproduced: identical in-place and `--clean` builds; 58/58 min and readable in 3 envs; 44 extra adversarial
      cases (90-way ties, 8,192-unit chunk edges, int32 ids, maxFuzzy 20) pass for all four; bench within 0.18;
      A/A 0.93–1.05.
    - GopherJS rewrites: idiomatic for GopherJS and faithful; **footnote in the matrix** that its receipts time the
      optimised port (the first idiomatic version was 7.6× / 7.2×).
    - Scala.js lint 13 = class fields. Dart's tie-break gives a strict total order.
    - My Unicode amendment confirmed (exactly 14 code points differ; all 398 corpus code points agree). Kotlin
      `--rerun-tasks` confirmed (4 tasks executed).
    - Parity-domain wording fixed (15.1 → 13.0, with the 14 excluded).
  - **Wave 3 verified:** Elm ACCEPT-WITH-NOTES, no blocking.
    - Reproduced: identical rebuilds; 58/58 in 3 envs, min and readable; bench within ~8%.
    - The JS host is I/O only (2.40%, not "mostly JS").
    - Snapshot walks are equivalent to MiniSearch's live iterator: 7 hand cases, 43,000 fuzz scripts with 0
      mismatches, 6 of 7 mutants caught (the survivor is provably equivalent).
    - The elm/core defect reproduced independently and is unreachable from the port.
    - `logBase e` is bit-identical on 720,606 arguments per engine.
    - Coordinator: the contract now puts fractional count options (`batchSize`, `minDirtCount`) outside the domain
      (Elm rejects them); Elm NOTES line count fixed; Elm rebuilt (same artifact) and the matrix regenerated.
  - Elm (implementer report):
    - 2,275 lines in 6 modules plus a 27-line JS host (2.4%, not "mostly JS"); 58/58 in 3 envs; rebuild identical;
    - 47,275 / 16,632 / 14,347 B (floor 9,679 / 3,886);
    - bench Node 3.03 / 2.82, Chrome 2.49 / 2.89, Firefox 1.92 / 2.71; A/A 0.98–1.01;
    - persistent index; insertion-ordered list nodes; native tokenizer from `separators.json`; `logBase e` checked
      equal to Math.log in 3 engines; fuzz 11,000 scripts with 0 diffs.
    - **elm/core 1.0.5 defect:** `String.toList "\u{DC00}"` never returns (`while (i--)` reaches -1);
      foldl / uncons / map / filter mis-decode lone surrogates; `Char.toCode` of a lone high surrogate is NaN.
      Repro in `scratchpad/elm-bugs/` (a write under `build/` was refused by permissions). Contract Elm guidance
      corrected.
- [x] **M1 merged (user OK 2026-11-19):** main fast-forwarded 46383cb → `eb24b87` (8 commits, no overlap with the
  dirty files).
  - Coordinator on main: rebuild OK; gate **181/181 unwind, 181/181 abort**.
  - Matrix Rust ports rebuilt with the fixed compiler: minisearch 318,772 B (+637), pilot 103,060, floor 416; all
    pass in 3 envs (58/58, 10/10, 3/3).
  - The original tuple-clone repro now passes.
  - Rust minisearch re-timed at load 6–19: Node 9.26 / 13.46, Chrome 6.46 / 10.93, Firefox 6.75 / 11.30.
  - Sizes and matrix regenerated: 27 rows, 9/9 minisearch ports at 58/58 in 3 envs.
- [x] L1-commit (user: "commit now"): `f72c452` holds the matrix plus 8 verified ports, the Kotlin fix, docs and notes.
  The Elm port follows in its own commit after wave 3.
- [x] L1v minisearch verification: Rust port **accepted** (faithful and safe; 49/49 exact in 3 envs under check/5;
  passes 9 adversarial extras). Contract had **2 blocking findings**, sent back:
  - no bench variant (the proposed one leaves texts unchanged); adopt the verifier's letter-permutation variant;
  - corpus gaps: 10 of 81 MiniSearch mutants escape (unstable autoSuggest sort, AND order, null vs absent, average
    field-length holes, wildcard + constructor boost, the vacuum boundary on the last term, the remove-term warning);
    add 9 extra cases.
  - Plus factual corrections: Math.log figures (Chrome 158 is correctly rounded; Node / Firefox differ on 41 args);
    `-0` ids; `removeAll(null)`; `fields: []`.
  - Fresh bench, bench/4, 24 samples, no variant: Node 8.18× / 13.62×; Chrome 6.25 / 9.81; Firefox 6.34 / 9.15;
    A/A 1.01–1.06.
  - **Rust profile decision: keep `release-size`, `-Oz`, MIR inline off.** `release-balanced` without inline is
    within noise and not smaller. MIR inline exposes a new mires miscompile, `[u16] == [u16]` reading past the
    buffer, sent to M1 as bug 5.
  - README Math.log figure corrected by the coordinator.
- [x] L0f Final harness round done, harness **frozen** (bugs only):
  - R3-3 requests attributed by page load (`/__load/N/`) and drained between loads;
  - R3-2 disjoint variant ranges per arm, a globalThis diff with per-family `runtimeGlobals`, and `--aa-range`
    [0.5, 2];
  - R3-1 a rotating pilot variant plus the variant rule;
  - R3-4 process.binding / dlopen blocked, a browser CSP refusing data: and blob:, wasm counted as eval refs;
  - R3-5 symlinks followed, no run-time file reads;
  - R3-6 HARNESS_CODE completed;
  - the threat model in the README;
  - bugs: the per-env reference digest for the bench gate, partial `sizes.mjs` merging, the undici and mires
    globals allow-listed.
  - Pilot 27/27 cells, A/A 0.90–1.02, selftest 18/18. Coordinator re-ran selftest (18/18) and the pilot Node check
    (9/9).
  - B1: timed calls are unchecked and share one input object (memo / hot-path-wrong controls benched "ok").
  - B2: bench and the matrix ignore parity (a port failing 9/10 still gets a ratio).
  - B3: receipts are not bound to the corpus / reference / normalisation digests.
  - B4: Kotlin output is not byte-reproducible (interface order flips; 4 hashes in 7 builds).
  - Reproduced fine: 9/9 builds, parity in Node / Chrome / Firefox, 27/27 cells, sizes, stale-dist refusal; the Elm
    patch is the comparator's verbatim. No same-process A/A bias found.
- [ ] L0q Queued after B1–B4, from the same verdict's notes:
  - the checker counts "reference throws + port returns a non-serialisable value" as both threw;
  - the shape gate is syntactic (`import()` hidden in eval passed in Node);
  - embedded-JS counter misses (Kotlin `js( ... )` with a space, multi-line triple-quoted `js()`, eval idioms) and
    double counts (`%%raw`);
  - Elm "mostly JS" formula differs from the comparator's (1.3% vs 5.6%);
  - calibration on cold code, so most samples are below `--min-sample-ms`; 9 samples is unbalanced for 3 arms (use
    12/18); with 9 samples "p95" is the max;
  - `--receipts` does not move the work dir;
  - the Rust pilot panic handler is `loop {}` (hangs a cell until timeout);
  - receipts lack the Node version and a harness digest.
- [x] L0r Rust profile: **decided, keep release-size / -Oz / MIR inline off** (see L1v). Original note: the verifier recommends `js-profile=release-balanced`, `-Oz`, `JSC_RUST_MIR_INLINE=on` (the
  repo's "real package settings"; the same sysroot as release-size). On the pilot it is within noise, ~−250 B gzip.
  Decide after measuring on minisearch with ≥3 seeds.
- [x] L1a minisearch contract + Rust port (implementer report; **verification pending until the harness is frozen**).
  - Contract:
    - broad API subset: add / remove / discard / replace / vacuum, search (prefix, fuzzy, boost, weights, bm25,
      AND / OR / AND_NOT, nested trees, wildcard, declarative filter and boost callbacks), autoSuggest, and a
      `toJSON` state probe;
    - 49 generated cases plus bench-index (1,200 docs) and bench-search (400 docs, 120 queries), with bench
      variants (id offset, query rotation);
    - UTF-16 code units everywhere; tokenizer via host `u`-RegExp or the native `separators.json`; full lowercasing
      with Final_Sigma;
    - radix-node insertion-order rules; host `Math.log`, which differs per engine, so parity is defined per
      environment.
  - Rust: 2,270 lines, safe, no `js!{}`; parity 49/49 in Node / Chrome / Firefox.
  - Sizes: 317,744 / 74,101 / 59,614 B (JS ref 20,006 / 6,419). Unicode lowercase tables ≈ 25 KB; one shared sort
    instantiation ≈ 14.6 KB (each core sort instantiation is ≈ 18 KB under mires).
  - Bench (old 9-sample run): Node index 7.9×, search 13.2×; Chrome index 6.4×; Firefox 6.1× / 8.9×. Time goes to
    mires heap side-table bookkeeping and GC.
  - **mires bugs (repros in `build/library-matrix/mires-bugs/`):**
    - `tuple-clone-uninitialized`, a miscompile: `(String, usize)::clone` emits `b[0]=…; return b` with `b`
      undefined. Coordinator reproduced it: TypeError on main 46383cb.
    - `boxed-slice-dealloc`: a run-time refusal on drop of `Box<[u16]>::from(&[u16])`.
    - `in-place-collect`: a compile refusal ("cannot cast a pointer to (u32, usize) into a pointer to usize").
    - `slice-partial-ord-lt`: a compile refusal on `<` for slices (`ControlFlow<bool>` transmute).
  - Harness bugs it found (sent to the final round): the per-engine reference digest for the bench gate; a
    per-library `sizes.mjs` run overwriting the shared table. Coordinator found: the globalThis guard flags undici
    and mires runtime globals.
- (M1 moved below, user OK.)
- [x] L-R Research → `research/library-matrix/candidates.{md,json}`: 39 ranked candidates with sources, plus 35
  rejected. Downloads (npm API, week 2026-11-11..17) spot-checked by the coordinator for earcut, long, lz-string,
  big.js and jmespath: exact. Recommended next wave: earcut, long, @noble/hashes (SHA-2/3/BLAKE2 subset), lz-string,
  diff, d3-hierarchy, big.js, jmespath. Alternates: entities, functional-red-black-tree; lru-cache is BlueOak-1.0.0.
  To decide later: js-yaml 4.x vs 5.x; time `long` with and without its WASM path.
- [ ] L1 Pure-compute libraries, one library at a time. Per library:
  - (a) a contract: API subset, corpus, canonical output, exact-parity rule, reference adapter;
  - (b) the Rust port, which proves the contract and the checker;
  - (c) the 8 other languages in waves of 3;
  - (d) independent verification per wave.
  - Order: minisearch → fflate → marked.
- [ ] L2 Host-API libraries: idb, Ky, uPlot (Canvas2D checked as a recorded call stream), Workbox (a service-worker
  subset; Chrome lifecycle). Browser checks are required.
- [ ] L3 Explorer composite per language.
- [ ] L4 Matrix report: parity, sizes and timing with A/A per cell. **Re-time every cell in one quiet window with no
  agents or builds running** (agent-time benches ran at load 5–53). User review before any commit.
