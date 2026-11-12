# mires v2, round 2: beat v1 on size and speed without losing semantics

## Context

Round 1 promoted its verified wins to local `main` (`b1bcca4`, unpushed). v2 is still well behind
v1 on size, and four safe-Rust miscompiles are open. Every large remaining size lever is blocked on
a correctness item or on a decision that was escalated and is now answered.

**Targets (frozen v1 figures, v2 measured with cleanup on):**

| `fp.esm.min.js` | v1 target min / gzip / brotli | `main` today | Gap to close |
|---|---|---|---|
| Compact | 160,615 / 44,283 / 33,890 | 241,745 / 61,301 / 47,412 | −81.1 K / −17.0 K / −13.5 K |
| Idiomatic | 259,527 / 64,908 / 53,064 | 443,971 / 111,722 / 86,772 | −184.4 K / −46.8 K / −33.7 K |

- Source of the v1 row: `recovery/v2/snapshots/2026-09-07/compiler/checkpoints/phase-3.md:166-198`.
  It was compressed with system `gzip -9 -n`, ours with Node zlib level 9; expect a few bytes of
  difference, and v1 had no drop-on-unwind cleanup.
- Runtime: v1 was never timed on the three workloads and cannot be rebuilt (no checkout on disk).
  The runtime gate is the Rust-to-JavaScript ratio on `marked` (about 7×), `minisearch` (about 42×)
  and `fflate` (about 18–31×): each must improve on the checkpoint and never regress.

**What the known levers buy (compact, measured separately, not additive):** cleanup-edge removal
−33 K min / −5.4 K gzip; its no-throw extensions −9 K / −2.4 K; pooling up to −21 K / −4 K;
inlining −4 K / −0.1 K; abort helper, module-scoped runtime, enum tags and trailing arguments
−18 K / −1.6 K together. That reaches roughly 150 K minified but only about 47–49 K gzip, so the
compact gzip target and the whole idiomatic target also need the formatter/String work. Whether
that is enough for idiomatic is not known; the checkpoint attribution (Stage C) decides.

**Decisions taken (2026-11-11):**
1. v1 target: the frozen figures above.
2. Host values: checked at typed boundaries; cleanup-edge removal returns only after that and after
   its proof gate is fixed. A trusted binding stays an explicit opt-in.
3. Pooling: a rule that cannot leak by construction (type-proven immutable and never
   host-reachable); the multi-chunk load fix lands separately first.
4. `as_flattened`: full support, then make the runtime helper pay-per-use if possible.

Unchanged from round 1: cleanup stays on in release profiles; fail closed; pinned toolchain; no
push; nothing from `experiments-pro/`, `recovery/` or `build/` in git; no `Co-authored-by` lines.

## Execution model

- **Implementers:** `Agent`, model `opus`, effort `xhigh`, `isolation: "worktree"`, one item each.
- **Verifiers:** `Agent`, model `fable`, effort `high`, a fresh agent per check. The tool offers a
  single Fable model, so the 5.1 version cannot be pinned.
- A stage is committed to `main` only on `pass` or `pass-with-limits`, one commit per stage. A fail
  returns to the implementer once; a second fail is split into a minimal fix or escalated. A
  verifier that dies for infrastructure reasons is re-spawned with the same brief.
- Every brief states: expected `HEAD`; a reserved fixture range; the gate commands and their
  baseline tables from Stage 0; the package modes (`JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on`
  as well as release-balanced); and, for `runtime/shim.js` edits, a list of programs whose output
  changed and which decisions flipped.
- At most three implementers build at once. No timing claims from parallel waves; timing is taken
  only at checkpoints on a quiet machine.
- The 22 old worktrees, `integration` and the stash are read-only patch sources. Pending work is
  re-applied onto a fresh worktree from `main` (`git -C <old worktree> diff` plus its untracked
  fixtures), never merged, and nothing there is deleted.

## Stage 0: baseline and bookkeeping (main session, before any agent)

- Append this plan as a dated addendum to `tasks/todo.md`; record the execution model and the four
  decisions in `tasks/lessons.md` / `CONTRACT.md` where they change the contract.
- Record the frozen v1 targets and the gap table in `experiments/v2-restoration/RESULTS.md`.
- Build `main` and run the full gate set once (see Verification); keep the tables as the baseline
  every brief refers to. Confirm the highest fixture number in use (202 expected) and reserve
  ranges: 210–219 (A1), 220–229 (A2), 230–239 (A3), 240–249 (A4), 250–259 (A5), 260–269 (A6),
  270–289 (B), 300+ (D).

## Wave A: close the open miscompiles (three lanes in parallel)

Lanes are split so no two running agents edit the same files.

| Lane | Item | Start from | Main files | Done when |
|---|---|---|---|---|
| Lowering | **A1** copy aliasing (1c), round 2 | worktree `agent-aa0340f7…` (`uses_as_storage`) | `jsc-link/src/lower/{fuse,ssa}.rs`, `lower/js/tests.rs` | `round1-open/copy_alias` passes in both modes; the `Cell::set` store under MIR inlining is right (native 5007); `SetDiscriminant` arm pinned by a test; generated extent corpus matches native |
| Runtime | **A2** flattening and heap arrays (1d + 1f) | worktree `agent-aeaee6e0…` | `runtime/shim.js` (`unwindow`), `backend/src/ptr.rs` (`build_fat`), `CONTRACT.md` | `flattened_safe` passes; `Box<[T; N]>`, `Vec<[T; N]>` read, grow and drop correctly on normal and unwind paths; size cost reported |
| Runtime, after A2 | **A3** pointer records recognised by field name (1e) | new | `runtime/shim.js` (`__rt.addr` :278-282, `"buf" in p` :147), `backend/src/ptr.rs`, `jsc-link/src/lower/js/ptr.rs` | `address_fields` passes; user structs with `buf/off/sc/es/pr/length` fields behave like native; no field-name whitelist |
| Constants | **A4** niche-optimised constants (1i) | new | `backend/src/constant.rs` (`read_enum`), `backend/src/intrinsics.rs` (`niche_from_scalar`), `jsc-link/src/lower/js/constant.rs` | `niche_constant` passes; nested enums, pointer niches, consts, statics and promoteds match native |
| Constants, parallel with A4 | **A6** multi-chunk ownership fix, extracted from pooling | worktree `agent-a12dd166…` (`link.rs` part only) | `jsc-link/src/link.rs` | two-chunk and static-to-static programs load; no pooling code comes with it |
| Lowering, after A1 | **A5** `#[track_caller]` chains (1h) | new | call/ABI lowering, `opt/inline.rs` | chained, direct and indirect calls keep `$loc` under release and MIR-inline modes |

Expected state after Wave A: `check-round1-open.py` 5/5 in both modes; `check-known-gaps.py` still
5/6 (aggregate-place identity stays open, see Deferred).

## Wave B: make cleanup-edge removal acceptable, then restore it

- **B1 typed host boundaries** (parallel with B2). Validate host values where they enter typed
  bindings, exports, callback arguments and results: `mires-js/src/{host,web}.rs`,
  `js-extern-macro/`, binding/export lowering in `jsc-link`. Controls: mistyped numbers and
  BigInts, symbols, objects with getters, chars. Report bytes and per-crossing cost. Keep the
  explicit trusted contract as an opt-in.
- **B2 proof gate** (parallel with B1). Cherry-pick `60eadac` onto a fresh worktree, then make
  validation gate mutation: summaries are revalidated before `cleanup::run` rewrites anything
  (`jsc-link/src/link/lane.rs`, `analysis/summary_check.rs`), witnesses rederive the no-throw
  premise for protected Rust and foreign calls (`opt/witness.rs`, `plan/cleanup.rs`), and
  disagreement refuses or reverts in an ordinary release linker with no environment variable.
  Acceptance: planted-false-summary and false-protected-operation linkers keep every destructor.
- **B3 promote 3b** once A2, B1 and B2 have passed. Then **B4** re-apply the no-throw extensions
  from `agent-aae8e384…` without `compare_bytes`, verified against B1's boundary checks.

## Stage C: checkpoint

One source tree, one compiler build, quiet machine: `measure.mjs` on compact, idiomatic, the
ten-workload corpus and Pro; paired timing with the A/A control; all gates. Record the row in
`RESULTS.md` against the v1 targets. Redo the byte and runtime attribution on these artifacts and
use it to order Wave D. If the admitted cases are resolved, this is the R1 candidate.

## Wave D: size and speed levers (ordered by Stage C attribution; default order below)

1. **Wider inlining (4′b)** re-applied from `agent-ab531332…` on top of A1 and the 1g fix
   (`opt/inline.rs`, `inline/remap.rs`); the loop-allocation growth case must be resolved.
2. **Small emission wins** in `lower/js/emit.rs` and the runtime: shared unreachable-abort helper,
   module-scoped runtime, numeric enum tags, dead trailing arguments (`analysis/param_use.rs`).
3. **Constant pooling** redesigned around the type-proven rule (new work in `link/pool.rs`,
   `lower/js/constant.rs`); depends on A1.
4. **Formatter and String commitments** (`plan/representation.rs`, `analysis/legality.rs`,
   `lower/semantic_ops/{fmt,fmt_kernel,string}.rs`): commit the proved spine, then widen supported
   shapes. This is the main lever for idiomatic gzip.
5. **Runtime:** heap registry off hot paths, string views converted only at the host boundary,
   `Range::next` / `Option` result objects, remaining overflow IIFEs. Fresh profile first.
6. **Optimising cleanup-bearing bodies** (effect-fenced fusion, escape analysis inside protected
   regions).
7. Verify granule-min address release (`agent-ac96e6c4…`) when the runtime lane is free.

Each item is its own implement-then-verify stage with a positive selection test, an off switch,
package numbers on both ports and no new failures. A second checkpoint follows Wave D.

## Deferred, stated plainly

- Aggregate-place identity (the sixth known gap): needs a model design review before any patch;
  the failed redesign and the stash stay parked as reference.
- Exact-size buddy allocator: parked.
- Numeric-seed API mismatch (360/477): adapter work in the ports, not a compiler change.
- Public document-processing holdout, browser matrix, Pro signing: outside this round.

## Verification

Baseline and every stage run, from the repository root after `bash scripts/build.sh --locked`:

```sh
bash scripts/check.sh
SKIP_BUILD=1 JS_EXTRA_ARGS='js-profile=release-balanced js-minify=on' bash scripts/test.sh --require-core
SKIP_BUILD=1 bash scripts/active-poll-cleanup-test.sh
SKIP_BUILD=1 python3 scripts/cleanup-transport-test.py
SKIP_BUILD=1 bash scripts/cleanup-admission-test.sh
SKIP_BUILD=1 bash scripts/extern-test.sh
node scripts/standalone-check.mjs
python3 scripts/check-known-gaps.py
python3 scripts/check-round1-open.py
JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on python3 scripts/check-round1-open.py
```

- Packages: build both ports with `tools/build.mjs --profile release-balanced --optimizer on
  --lane jir --cleanup on`, then `node experiments/tools/check-oss-kernels.mjs` (58/58 each) and
  the corpus (`npm run build:corpus`, `run.mjs --pure`).
- Sizes and timing: `node experiments/v2-restoration/measure.mjs --label <stage>`; timing only at
  checkpoints.
- Correctness fixes are checked against the pinned native compiler at O0 and O3, with mutation or
  negative controls, and with the neighbouring shapes probed, not only the reported one.
- `test.sh` tables are compared behaviourally against the Stage 0 baseline; byte-budget rows are
  not treated as waived.
- Success for the round: both ports under the v1 targets on minified, gzip and brotli; workload
  ratios better than the checkpoint; `round1-open` 5/5; no gate weaker than at Stage 0.
