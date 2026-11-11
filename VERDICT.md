# Current verdict — selected round-1 main promotion

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


The packages were built on the selected staging source. Main's rebuild/smoke result is recorded separately after promotion; staging tests are not relabeled as a second main run. Current sizes and exact artifacts are in the comprehensive report. Package API checks use the explicit size-failure allowance; six historical size failures per Rust package remain failures.

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
