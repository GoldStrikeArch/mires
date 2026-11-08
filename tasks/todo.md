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

## Stage 5 — next wave (chosen after Stage 4 attribution)
- [ ] P10 strings / P12 formatter / P13–P16 bounded passes / browser timing matrix
- [ ] 2e (policy arm, not equivalence): cheaper trial compression (e.g. gzip-only or brotli q5 for trial ranking, brotli-11 only for the final candidate) — must be qualified as a decision-policy change with before/after decision diffs

## Known gaps discovered (not blocking the current stage)
- `Rc<[Z]>` built via `Rc::from(vec![Z;n])` fails on the JIR lane with `__rt.retype_rc: this pointer does not name a live heap block` (passes on the default lane). Pre-existing; found by the 1a verifier (fixture scratchpad/verify1a/fx/rc_vec.rs).
- [x] `scripts/active-poll-cleanup-test.sh` was stale (never requested js-cleanup=on); now a real gate (60/60 + baseline refusal). Open: same trap bug in scripts/compile.sh:116; wire the gate into check.sh/CI after 3a's check.sh edit lands.
- Runtime: on the current build, compiled Rust is 43–146x slower per call than the JS originals (marked 146x, minisearch 68x, fflate 43x; harness commit 75d30a2). Diagnosis running; this, not scaffolding, is the runtime problem.
