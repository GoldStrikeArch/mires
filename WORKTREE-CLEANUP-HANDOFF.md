# Worktree cleanup handoff — preserved rounds 1 and 2

Date: 2026-11-12. Scope: preserve implementation and evidence, validate the combined main compiler, commit useful research/tests/docs, and leave deletion to the user. **No push. No worktree, branch, or stash was deleted.**

## 1. What is safe to rely on

The three accepted Round 2 compiler changes were already on `main`: A6 static/chunk ownership (`cf80163`), A6b import-cycle hardening (`7979702`), and A1 copy/storage repair (`1362e12`). This consolidation does not replace them or merge an unfinished optimization branch. It rebuilds and tests their combined source at `1b582b84793842bd64698256d2811531f638e99b`.

The preservation has two deliberately different parts:

| Location | Role | In public Git? |
| --- | --- | --- |
| `research/v2-restoration/rounds-1-2/` | Original research sources, probes/generators, reports, branch identities, exact reconstruction patches, restore instructions | Yes |
| `examples/round2-open/` and new scripts | Portable strict regression checks and package-mode gate | Yes |
| `docs/round2-consolidation-verification.json` | Fresh combined-main verification, source/binary identities and limits | Yes |
| `recovery/worktree-preservation-20261112T013511Z/` | Complete preserved branch/stash bundle, private/raw evidence database, project sessions, dirty patches and checksums | **No; ignored and private** |
| `/Users/mikhail.pertsev/mires-recovery-backups/worktree-preservation-20261112T013511Z` | Second local copy, including the final source/history backup | Outside repository; same device |

**Keep the named `recovery/` directory and the external backup when deleting worktrees.** A clone of main contains the public source-reconstruction patches, but it does not contain the private evidence database or Pro material. A same-device second copy does not protect against device loss.

## 2. Source preservation, including unfinished work

All **43 registered worktrees** were inventoried. **Nine** held uncommitted source that now has a preservation-only snapshot commit. Original non-main indexes and working-tree files were left unchanged. Every resulting source tree was restored from the bundle and checked against its source. Independently, every saved patch was applied to its recorded base through an isolated index and produced the exact recorded Git tree.

Two end-to-end restore tests created new independent Git repositories: the pending A4 niche branch and the previously uncommitted no-throw extension. Both matched their snapshot tree hashes. A raw-evidence extraction also reproduced 194 records with verified content. These tests did not depend on the original worktree directories.

Durable local refs use `archive/rounds-1-2/<worktree-id>`. The private bundle also contains all original refs and the existing stash. The main tree contains patch-based reconstruction for the same source states, so deleting an old branch does not erase the only public source representation.

### Important unfinished implementations

| Work | Restore ID | Do not confuse preservation with acceptance |
| --- | --- | --- |
| Niche constants, third round | `agent-a0a61a0aaa5ae144a` | Delivered, verification incomplete |
| Flattening and heap arrays, second round | `agent-afbe5ab2cda06b284` | Delivered, verification incomplete |
| Caller-location/function-pointer repair, second round | `agent-af471d6991ec28a6b` | Delivered, verification incomplete |
| Wider inlining | `agent-ab531332a87a0b330` | Delivered older candidate; needs current-base qualification |
| Constant pooling | `agent-a12dd166f55ef46f8` | Known host-isolation failures; not accepted |
| No-throw extensions | `agent-aae8e384d9b94c2e6` | Boundary/proof defects remain; not accepted |
| Minimal address release | `agent-ac96e6c4714b6a992` | Unverified; keep distinct from full allocator redesign |
| Full address allocator | `agent-acd9d634a1811beab` | Parked after failed verification |
| Old integration branch | `integration` | Contains withheld cleanup removal; do not merge wholesale |

Other historical repairs and verifier trees are fully indexed in the research directory. Some have already been adapted onto main; some are superseded. The directory README records these distinctions.

## 3. Verification evidence preserved

The private content-addressed database retains **812,662 file records** and **146,007 unique contents**, representing approximately 28.3 GB before deduplication/compression, in approximately 2.5 GB. Every unique content was decompressed and checked against its original SHA-256. The database integrity check passed and no selected source file read failed.

Coverage includes both round scratchpads, worktree experiment source and delivered artifacts, private Pro material, generated JavaScript, counterexample sources, native expectations, mutation generators, diagnostic logs, receipts, and prior audits. Symbolic links are recorded rather than traversed. A separate private archive retains **224 project-specific session/transcript files**; it does not collect global account credential stores. Opaque `.bin` test inputs/controls are preserved in a separate supplement rather than discarded on the assumption that all are rebuildable.

Native executables, bulk `.o`/`.rlib`/`.rmeta` products, sysroot/dependency caches, and installed toolchains are not a required part of this source archive. The explicit exclusion index remains available. This is not a byte-for-byte machine backup or an offline mirror of every dependency.

The public archive contains **2,082 original historical source/report files**, plus branch patches and indexes. Redundant full compiler copies remain available in the private database rather than being duplicated in public research. Historical scripts may contain their original absolute paths; their identities are preserved, not silently rewritten. The new portable commands below are separate.

## 4. Fresh combined-main results

The installed compiler had been stale at the beginning. It was rebuilt from the accepted combined source before these checks. Compiler implementation files were not changed by this consolidation.

| Check | Fresh result |
| --- | --- |
| `scripts/check.sh`: format, Clippy, Rust tests | **1,077 passed, 0 failed, 7 ignored** |
| Generic runtime checks | **75/75 passed** |
| Cleanup-on release fixture suite | **160/160 passed**; historical default-mode byte budgets not enforced |
| Chunk suite inside check.sh | **13/13 passed** |
| Active-poll cleanup | **60/60 passed**, plus required baseline refusals |
| Cleanup transport and admission | Passed |
| Extern suite and standalone artifact/map check | Passed |
| A1 fixtures 210–214 under Rust `-Oz` plus MIR inlining | **5/5 passed** |
| Original October-derived known-gap inventory | **5/6**; aggregate-place remains open |
| Round 1 adversarial inventory | **2/5** in both modes; copy alias and lost write pass |
| Newly portable Round 2 inventory | **0/4** in each of two modes; incorrect output remains a failure |

The new Round 2 native expectations agree at native optimization levels 0 and 3. Unknown-case selection fails before compilation. Restore tooling refuses an existing destination and traversal prefixes. No empty or incomplete test run is accepted as a pass.

The full fixture suite was run in release-balanced; only the focused A1 subset was run through the new package-mode command during this consolidation. The command supports the full suite, but this record does not claim that full additional run. No new package-size or runtime-performance claims, browser parity, distribution/signing qualification, or pending-branch verifier verdict were produced.

## 5. Portable continuation commands

From the repository root:

```sh
# Exact source-state inventory; none of these commands push.
python3 scripts/preserved-work.py list-work

# Restore a pending branch into a NEW independent local repository.
python3 scripts/preserved-work.py restore-work agent-a0a61a0aaa5ae144a \
  --destination /absolute/path/to/restored-a4

# Verify the private source/evidence bundle before removing old worktrees.
python3 scripts/preserved-work.py verify \
  --archive recovery/worktree-preservation-20261112T013511Z

# Find/extract original verifier evidence without recreating /private/tmp.
python3 scripts/preserved-work.py list-evidence \
  --archive recovery/worktree-preservation-20261112T013511Z
python3 scripts/preserved-work.py extract-evidence \
  --archive recovery/worktree-preservation-20261112T013511Z \
  --root scratch-bf1d1602-d3a2-4fa9-88f8-ca711756aab5 \
  --prefix verify-a1/prebug \
  --destination /absolute/path/to/restored-counterexamples

# Strict targets: currently exits nonzero because bugs remain.
python3 scripts/check-round2-open.py --mode both

# Actual package compilation settings; no names means the entire fixture suite.
SKIP_BUILD=1 bash scripts/package-mode-test.sh 210_dead_copy_writes \
  211_transparent_copy_writes 212_interior_mutable_copy_writes \
  213_copy_sources_and_variants 214_copy_writes_through_borrows
```

The source restorer verifies the resulting Git tree, not merely filenames or patch exit status. It leaves reconstructed changes staged for review, rather than committing or merging them silently. Raw evidence extraction creates regular files and a symlink-target manifest, never external symlinks that could lead back into soon-deleted worktrees.

## 6. Remaining work and acceptance boundaries

**Immediate correctness priorities:** A7 static projections (including the newly executing cycle cases), large-enum copy semantics, array-rest writes, then completion of the A2/A4/A5 verification on the combined base. Safe programs that now return wrong answers must not be waived simply because the underlying decoder was wrong before the program could load.

**Cleanup optimization prerequisites:** checked typed host boundaries and mandatory independent proof validation before edge removal. Release cleanup remains on. The old no-throw pass is preserved but not promoted.

**Test gaps:** the package-mode gate now has a portable entry point, but CI/full-suite scheduling and the remaining A1 mutation control are still follow-up work. A6c's three-file/hoisted-constant/nested-prototype controls and refusal-parser hardening remain open. This consolidation does not claim to implement those repairs.

**Optimization after correctness:** formatter/String representation commitments; widening inlining with preserved alias/cleanup behavior; a host-isolated pooling rule; dead trailing arguments; runtime-closure and allocation improvements. A1b's few hundred compressed bytes are not prioritized above silent wrong answers. Historical October and v1 byte targets remain distinct labeled comparisons, not proof of current equality.

**Qualification:** keep the public non-fingerprinting holdout, fresh exact-artifact package/browser parity, quiet-machine runtime measurements, and distribution integrity in the roadmap. They have not become complete by preserving their plans.

This remains a working development compiler, not R1 or a production-qualified replacement for all admitted Rust programs.

## 7. Cleanup boundary

Use Git's worktree inventory to identify the old checkouts; the actual directory is `.claude/worktrees/` (with the leading dot). Prefer removing registered worktrees through Git rather than removing arbitrary directories and leaving stale metadata. No removal was performed here.

Do **not** remove `.git`, `research/`, the named `recovery/` archive, the separate backup, or original experiment/reference directories as part of worktree cleanup. Keep the archive refs or bundle if exact historical commits matter; the public patches reconstruct source but do not preserve every commit's narrative. The existing stash remains untouched and included in the bundle.

Before discarding all local copies, copy the verified backup off this Mac. No off-device copy or remote push was performed.
