# mires v2: smaller and faster output, cleanup on

Plan: ~/.claude/plans/alright-look-at-the-breezy-hejlsberg.md (approved 2026-11-08).
Execution: implementer subagents (Opus 5.5, xhigh) + verifier subagent (Fable 5.1, high) per stage.

## Stage 0 — baseline commit + measurement loop
- [ ] Baseline commit of the reconstruction on main
- [ ] `experiments/v2-restoration/measure.mjs` + `RESULTS.md`
- [ ] `RCGJS_COST_PROFILE=1` compile-cost counters in jsc-link
- [ ] R0 row recorded

## Stage 1 — correctness (P02/P03)
- [ ] 1a ZST aggregate storage/drop: 195_zst_array_drops, audit_zst_closure_drop, audit_zst_vec_drop
- [ ] 1b audit_aggregate_place identity
- [ ] check-known-gaps 6/6; suites green; R1 row

## Stage 2 — compile cost (P04), equivalent output
- [ ] 2a fuse.rs catch-name reservation O(1)
- [ ] 2b shim parsed once in cost.rs
- [ ] 2c physical_cost cache key hoisted
- [ ] 2d dependency-aware lowering reuse + RCGJS_COST_REUSE_VERIFY
- [ ] Pro optimized cleanup-on link completes; R1.1 row

## Stage 3 — remove provably unnecessary cleanup edges (P07)
- [ ] 3a RemoveNoopLandingPads after ElaborateDrops
- [ ] 3b plan/cleanup.rs: EmptyChain + NoThrow (may_throw summary), witnesses, report
- [ ] 3c no empty try for dead protected op
- [ ] cleanup-edges tests + CONTRACT.md note; R2a row

## Stage 4 — optimize cleanup-bearing bodies (P08/P09-lite)
- [ ] 4a shared try per protected region
- [ ] 4b fusion with fences
- [ ] 4c PEA in Invoke bodies
- [ ] 4d inlining at unprotected sites
- [ ] R2 row; re-plan from attribution

## Stage 5 — next wave (chosen after Stage 4 attribution)
- [ ] P10 strings / P12 formatter / P13–P16 bounded passes / browser timing matrix
