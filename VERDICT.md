# Current verdict — standalone main, selected Round 3

## Assessment — 2026-11-17

**Working development compiler; near the old OSS size waypoint, not exact dev13 and not production-qualified.** Main contains compiler implementation `933d5d0` and its receipt `ba72c43`, including the repaired next-gen pipeline and selected Round 3 work. The rejected R3-D runtime fast paths remain out.

See [RESTORATION-STATUS.md](RESTORATION-STATUS.md), [the current machine-readable status](docs/main-restoration-status-2026-11-17.json), and [preserved independent reviews](research/v2-restoration/round3/README.md).

Fresh main checks: explicit JIR+optimizer fixture gate **174/174 in each panic mode**, runtime **88/88**, retained artifacts **14/14 exact hash/size matches**, and standard OSS kernels **58/58 each**. The fixture totals include six intended compile refusals per mode and do not certify every profile/MIR configuration. Open inventories remain **5/6, 2/5, 0/4**; six additional boxed-slice/Rc/constant-closure runs fail their native expectations.

Current compact is **155,681 min / 47,856 gzip / 36,762 Brotli**; default idiomatic is **267,044 / 78,583 / 63,111**. Historical October comparisons are not fully input/compressor matched. Current MiniSearch timing fails its A/A band despite the outer receipt's success fields, so no authoritative runtime qualification is claimed.

Outstanding promotion/release hardening: mandatory release exceptional-edge validation, timing-failure enforcement, incremental-sensitive sysroot identities, complete immutable per-arm inputs, and recovery of the supported semantic/lifecycle/consumer contracts. Inherited errors are not Round 3 regressions, but remain errors.

## Historical promotion and consolidation records

**The sections below retain the November 11–12 evidence. They are not current branch-status or test-count claims.**


## Current consolidation — 2026-11-12

The combined Round 2 main compiler has now been rebuilt and checked: 1,077 Rust tests, 160/160 release fixtures, 75/75 runtime checks and 13/13 chunk cases pass. Cleanup transport/admission and 60/60 active-poll cases pass. Known gaps remain 5/6, Round 1 open cases 2/5, and the preserved Round 2 counterexamples remain incorrect. No unfinished compiler branch was promoted. See [the cleanup handoff](WORKTREE-CLEANUP-HANDOFF.md) and [the fresh verification receipt](docs/round2-consolidation-verification.json). Earlier sections below are historical scoped promotion evidence, not a declaration that every later regression passes.


Date: 2026-11-11. **Working standalone development compiler; not production-qualified, not exact October dev13, and not R1.**

The current source combines the restored v2 architecture with selected round-1 wins and two freshly checked narrow correctness repairs. The full integration branch was deliberately not merged: its 3b cleanup-edge pass and pending no-throw extensions remain out until their proof gate and representation assumptions are repaired. Release cleanup stays on.

See [the comprehensive integration report](round_1_integration_report.md), [the exact verification receipt](docs/round1-promotion-verification.json), and [the historical round report](round_1_report.md). Original main was `a386c41d122f1f9e086119495b1d01285c18bc3f`; the tested implementation checkpoint is `045860d6bbcf689c0c05c9c9c06f623b47ea9922`. Documentation commits do not change that implementation.

## Fresh selected-candidate evidence

| Fresh check on selected staging source | Result |
|---|---|
| Format / Clippy warnings denied / Rust tests | Pass; **1055 passed, 0 failed, 7 ignored** |
| Generic and pointer runtime tests | **75/75 pass** |
| Release-balanced behavior/structural fixture suite | **146/146 pass**; default-mode byte budgets not applied |
| Pointer fixture native controls | 82 output lines agree at native O0/O3 |
| Pointer additional compiler settings | Default, JIR opt-off, release-balanced and compact-package MIR-inline pass |
| Alias-store native fixtures | Fixtures 130/131 agree at native O0/O3 |
| Alias-store generated extent | 22 programs / 132 functions × two settings; all 44 executions match fresh native output |
| No-op landing-pad controls | Optimization on/off × MIR inlining on/off pass |
| Active host cleanup | 40 smoke cases plus ZST controls pass; separate active-poll matrix **60/60**; baseline refuses as required |
| Cleanup transport and admission | Pass, including negative producer/checksum/version/metadata cases |
| Extern suite | **5/5**, including 118 byte-boundary and 17 globals assertions |
| Standalone module | Values, cleanup default, maps and manifest pass |
| Cleanup holdback control | 4/4 valid/mistyped host arithmetic cases preserve the destructor |
| October-derived known gaps | **5/6**; aggregate-place identity remains open |
| Round-1 adversarial inventory | **1/5**; alias-store repaired, four other failures remain |
| OSS standard kernel comparisons | **58/58 each** on compact and idiomatic packages |
| Expanded OSS seed API comparisons | **360/477 each**; failures identical to old main |
| Corpus | **10/10 build**, Node pure/reference checks pass; no browser matrix |
| Optimized cleanup-on Pro | Main/worker build and offline API/import/invalid-configuration checks pass |


The packages were built on the selected staging source. Both binaries were subsequently rebuilt in main; the standalone check, open-case inventories and compact package/kernel checks were replayed there. Main's compact artifact is byte-identical to staging, and all 431 source-inventory entries match. Full staging suites are not relabeled as a second main run. Current sizes and exact artifacts are in the comprehensive report. Package API checks use the explicit size-failure allowance; six historical size failures per Rust package remain failures.

## What remains

Aggregate-place identity and four preserved round-1 cases (copy aliasing, flattened rows, pointer field-name confusion and niche constants) remain open. Broader heap-array, track-caller, address-lifetime, mutable-string and host-boundary work is pending. The alias-store regression is repaired conservatively; wider inlining is not thereby qualified.

Next come correctness prerequisites, cleanup-proof enforcement and explicit host-boundary contracts, then formatter/String commitments and proven output improvements. The public holdout remains unimplemented. No new browser matrix, quiet-machine runtime comparison, clean external distribution or signing was performed. Other-language ports remain restored but were not rebuilt in this task.

## Reproduce the open inventory

```sh
bash scripts/build.sh --locked
python3 scripts/check-known-gaps.py
python3 scripts/check-round1-open.py
```

The strict diagnostic runners intentionally exit nonzero while defects remain. Native expectations are not replaced with incorrect compiler output. [The original recovery record](docs/V2-RESTORATION.md) is historical; its older counts are not the current candidate's counts.

No push was authorized or performed. Private Pro material, recovery inputs, binaries and raw build output stay out of Git. Local backup locations and preservation limits are recorded in the integration report.
