# Round 3 review and current-main status evidence

Preserved on 2026-11-17. See [the current restoration assessment](../../../RESTORATION-STATUS.md) and [machine-readable main status](../../../docs/main-restoration-status-2026-11-17.json).

## Independent review records

The following files are exact copies of the original reviewers' JSON, not newly authored verdicts:

| File | Original scope |
|---|---|
| `correctness-initial.json` | First independent correctness review of the Round 3 candidate against `44161c2`. |
| `correctness-recheck.json` | Recheck of `933d5d0` after the runtime-shim fast paths were removed. |
| `gates-initial.json` | Gates, package measurements, environmental failures and initial attribution. |
| `gates-recheck.json` | Combined gate and package recheck for `933d5d0` / receipt `ba72c43`. |

`provenance.json` records original worktree paths and SHA-256 hashes. Their PASS classification is **scoped non-regression**, not a claim that every program was native-correct or that every test command exited successfully. Base-identical bugs, expected refusals, inherited/environment failures and untested configurations are retained explicitly. Referenced raw logs and generated artifacts are not all embedded in this small public archive.

## Fresh November 17 evidence

`remeasured-artifacts.json` binds 14 retained artifacts to raw/gzip/Brotli sizes and hashes. All matched the committed Round 3 receipt. It is not a fresh application rebuild.

`open-inventory.json` records newly compiled current-main runs of the preserved known-gap and Round 1/2 inventories: **5/6, 2/5, and 0/4**, respectively. Incorrect output is still a failure.

`capability-results.json` records three new probes with native expectations checked at O0 and O3. Each was compiled in optimizer-off and optimizer-on forms with the current JIR release-balanced unwind configuration. **None of the six JavaScript executions matched the complete native outcome.** These are useful restoration targets, not newly accepted limitations and not demonstrated Round 3 regressions.

| Probe | Native expectation | Current observed failure |
|---|---|---|
| `october_box_slice` | `3 / 6 / 2 / 9`, normal return | `3 / 0 / 2 / 9`, then invalid deallocation abort. |
| `october_rc_make_mut` | `9 / 7 / 1 / 10`, normal return | Byte-view size mismatch before output. |
| `october_const_closure` | `42`, normal return | `0`, normal return. |

The `october_` prefix names the historical capability under investigation. **These are not recovered byte-exact October fixture sources.** Original late fixtures are known by their historical records; a matched execution against the missing October compiler was not possible.

`capability-probes/` contains the current Rust sources, native companions and expected output. The sole adaptation from the audit's codegen source is a repository-relative prelude path; original and portable file hashes are in `capability-probes/provenance.json`.

Example commands from the repository root (these are failing regression targets, not a release gate expected to pass today):

```sh
mkdir -p build/restoration-capabilities
rustc +nightly-2026-07-27 --edition=2021 --crate-name capability_oracle \
  -Copt-level=3 research/v2-restoration/round3/capability-probes/october_rc_make_mut-native.rs \
  -o build/restoration-capabilities/native-rc
build/restoration-capabilities/native-rc

SKIP_BUILD=1 JSC_PANIC=unwind \
  JS_EXTRA_ARGS='js-profile=release-balanced js-lane=jir js-opt=on' \
  bash scripts/build_sysroot.sh
JSC_PANIC=unwind RCGJS_WITNESS_CHECK=1 \
  JS_EXTRA_ARGS='js-profile=release-balanced js-lane=jir js-opt=on' \
  bash scripts/compile_core.sh \
  research/v2-restoration/round3/capability-probes/october_rc_make_mut.rs \
  build/restoration-capabilities/rc.js
cat runtime/shim.js build/restoration-capabilities/rc.js scripts/entry.js | node
```

## Historical comparison inputs

`historical-receipt-boundaries.json` retains selected fields and hashes from the four original October dev13 receipts. It preserves their explicit production/timing limitations and names their local archive paths. It is an extract, not a replacement for the full original receipts.

The October dev12 OSS size numbers are a separate pre-loss waypoint preserved in the project narrative. Do not confuse them with dev13's qualification records, frozen v1 targets, or a new matched-input benchmark.

Raw logs, generated JavaScript, native binaries, the initial Git/source snapshot, and final verification are retained outside the repository under the audit directory identified in the main status receipt. No private Pro source or runtime package is added here. Same-device preservation is not off-device backup.
