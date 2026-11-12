# Preserved v2 restoration work: rounds 1 and 2

This directory is the **public, committed research archive**, not a directory of optimizer patches accepted into the compiler. It was assembled before worktree cleanup. The original task branches and working trees were left intact. The private/raw archive is under `recovery/` and is deliberately ignored by Git.

## What survives a Git clone of main

- `worktree-snapshots.json`: 43 source-tree identities, original heads, durable archive refs, bases, and reconstruction patches. Nine working trees contained uncommitted source; those were snapshotted on `archive/rounds-1-2/*` branches without modifying their original index or files.
- `pending-patches/`: complete source-tree differences from a base reachable from main. These preserve unfinished implementation work **without enabling it**. Some are superseded or known to fail. Never apply all of them to main.
- `historical-evidence/`: original counterexamples, generators, mutation scripts and briefs. Source provenance and hashes are in `source-provenance.json`. These historical scripts may retain original absolute paths; they are evidence/specifications, not a portable command suite.
- `reports/`: historical audit reports and the approved Round 2 plan, copied unchanged. `round_1_report.md`, `round_1_integration_report.md`, and `round_2_report.md` remain in the repository root.
- `private-only-duplicates.json`: index of redundant historical compiler copies not duplicated in public Git; their content is in the private evidence database.

**Historical documents are not live qualification receipts.** In particular, the approved Round 2 plan incorrectly describes the v1 compression row as system gzip; the authoritative recovered `phase-3.md` specifies Node zlib level 9. The root round-2 report records the correct qualification. Earlier timing ratios are diagnostic and are not current-main timing claims.

## Recover a source tree without its original worktree

List all saved trees:

```sh
python3 scripts/preserved-work.py list-work
```

Restore the pending niche-constants implementation into a **new, independent local Git repository**:

```sh
python3 scripts/preserved-work.py restore-work agent-a0a61a0aaa5ae144a \
  --destination /absolute/path/to/restored-niche-work
```

The command fetches only from this local repository, checks out the preserved base, applies the hash-checked patch to the index, and verifies the resulting Git tree against the recorded snapshot tree. It does not merge into main, push, or infer that tests passed. The restored diff is staged for review. The destination must not exist.

This patch-based route works even after the original worktree and branch are removed, because the preserved base is an ancestor reachable from main. Exact original commits, branch history and stash are additionally retained by the private Git bundle.

## Important snapshots

| Work | Snapshot ID | Status at preservation |
| --- | --- | --- |
| A4 niche constants, round 3 | `agent-a0a61a0aaa5ae144a` | Delivered, verification unfinished; **not merged** |
| A2 arrays/flattening, round 2 | `agent-afbe5ab2cda06b284` | Delivered, verification unfinished; **not merged** |
| A5 caller-location, round 2 | `agent-af471d6991ec28a6b` | Delivered, verification unfinished; **not merged** |
| Wider inlining | `agent-ab531332a87a0b330` | Uncommitted Round 1 work preserved; not accepted |
| Constant pooling | `agent-a12dd166f55ef46f8` | Known host-isolation failures; not accepted |
| Expanded no-throw analysis | `agent-aae8e384d9b94c2e6` | Known proof/boundary defects; not accepted |
| Minimal drop-address release | `agent-ac96e6c4714b6a992` | Unverified; preserve separately from allocator redesign |
| Full/buddy address allocator | `agent-acd9d634a1811beab` | Parked after failed verification |
| Earlier copy-alias repair | `agent-aa0340f7b4c30f8c0` | Superseded by landed A1 |
| Earlier pointer moves | `agent-ae104b5a611c870ef` | Adapted and verified on main in Round 1 |
| Earlier alias-store repair | `agent-ae6fe0b2bcf6f29f7` | Adapted and verified on main in Round 1 |
| Earlier flattening repair | `agent-aeaee6e0ef235816f` | Superseded by A2; not accepted independently |
| Old integration branch | `integration` | Includes withheld cleanup-edge pass; **do not merge wholesale** |

All other IDs, including independent verifier states, are indexed. A preservation snapshot is never a pass verdict.

## Private evidence: retain the recovery directory

The exact directory is named in `WORKTREE-CLEANUP-HANDOFF.md`. Its `evidence.sqlite3` stores content-addressed, zlib-compressed file contents with original paths, root identity, modes and hashes. It contains private Pro material and unsanitized logs and must not be committed or published.

```sh
python3 scripts/preserved-work.py verify --archive recovery/worktree-preservation-TIMESTAMP
python3 scripts/preserved-work.py list-evidence --archive recovery/worktree-preservation-TIMESTAMP
python3 scripts/preserved-work.py extract-evidence \
  --archive recovery/worktree-preservation-TIMESTAMP \
  --root scratch-bf1d1602-d3a2-4fa9-88f8-ca711756aab5 \
  --prefix verify-a1/prebug \
  --destination /absolute/path/to/restored-evidence
```

Extraction verifies content hashes, refuses an existing destination, and records symlink targets rather than following/recreating external links. The original paths identify provenance, not a requirement to recreate the old `/private/tmp` layout.

The private archive also contains a verified `all-refs-preserved.bundle`, staged and unstaged patches, worktree manifests, and `private-project-sessions.tar.gz` with this project's sessions and final prose reports. No global account credential store was collected. Do not publish the session archive.

## Portable tests now in the active repository

```sh
# Original native expectations; remains nonzero while wrong answers remain.
python3 scripts/check-round2-open.py --mode both

# The settings used by real compact-package compilation.
# With no fixture names, runs the full fixture suite in this mode.
SKIP_BUILD=1 bash scripts/package-mode-test.sh 210_dead_copy_writes \
  211_transparent_copy_writes 212_interior_mutable_copy_writes \
  213_copy_sources_and_variants 214_copy_writes_through_borrows
```

`examples/round2-open` contains four safe-Rust counterexamples with fresh, agreeing native O0/O3 expectations. These are not accepted wrong-answer goldens. The runner fails on missing/empty inputs and records compile failures and timeouts rather than inferring a pass.

## What is intentionally not preserved as required rebuild input

Native executables, object files, rlibs/rmeta, compiler/dependency caches, installed language toolchains and bulk sysroot caches are excluded from the evidence database. A separately verified `binary-fixtures-supplement.tar.gz` retains all 16,735 `.bin` records (including opaque test inputs and some native controls) rather than assuming every binary-looking fixture is disposable. Source, native expectations, generators, diagnostics, JavaScript text, manifests, receipts and branch identities are retained. Symlinks are indexed but not traversed. The exclusion table in the database lists the explicit reasons. This is a verified source/evidence preservation, not a bit-for-bit disk image or a hermetic toolchain mirror.

No worktree, old branch or stash was deleted during preservation. Removing worktrees is a separate user action. Keep `research/`, the named `recovery/` archive, the root experiment inputs and the separate backup.
