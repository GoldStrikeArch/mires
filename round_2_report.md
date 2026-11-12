# mires v2 optimization: round 2 report

Period: 2026-11-11 to 2026-11-12. State as written: paused at the user's request; no agent is
running. Nothing in this report was re-run to write it; every figure comes from the round's
implementer and verifier reports, and each is labelled with who measured it.

## 1. Summary

The goal of this iteration is a v2 compiler that beats v1 on bundle size (minified and compressed)
and on run time, with drop-on-unwind cleanup kept on and no semantics lost.

- **Round 2 so far is correctness work, not size work.** The approved plan puts the open
  miscompiles first, because every large size lever is blocked on one of them. No size lever has
  been started yet.
- **Three changes are verified and on local `main`:** the multi-chunk and static-cycle link fix,
  its hardening follow-up, and the copy-aliasing fix. `main` is at `1b582b8`, 11 commits past the
  round-1 head, unpushed.
- **Three more are delivered on branches with no verdict:** niche constants (third round),
  flattening and heap arrays (second round), `#[track_caller]` (second round). Their verifiers did
  not report before the session ended.
- **Sizes moved slightly the wrong way.** The copy-aliasing fix costs +527 minified / +298 gzip
  bytes on the compact package and +596 / +378 on idiomatic. Its verifier found a tighter rule
  that returns both packages to their previous bytes; that follow-up is not started.
- **Independent verification failed four of the seven changes it finished checking.** Each failure
  was a silent wrong answer or a regression that passed every gate. The verifiers also found
  about forty miscompiles that exist on `main` independently of this round's changes.
- **The three landed changes have never been gated together.** Each was verified alone. One
  integration gate run on `main` is owed before anything else lands.

## 2. Targets and where the packages stand

`fp.esm.min.js`, bytes, minified / gzip / brotli. v2 is measured with cleanup on.

| Package | v1 target | `main` at round start (`b1bcca4`) | `main` now (`1362e12`), expected | Gap to v1 now |
|---|---|---|---|---|
| Compact | 160,615 / 44,283 / 33,890 | 241,745 / 61,301 / 47,412 | 242,272 / 61,599 / 47,650 | 81,657 / 17,316 / 13,760 |
| Idiomatic | 259,527 / 64,908 / 53,064 | 443,971 / 111,722 / 86,772 | 444,567 / 112,100 / 87,101 | 185,040 / 47,192 / 34,037 |

- **v1 row:** historical, from `recovery/v2/snapshots/2026-09-07/compiler/checkpoints/phase-3.md:166-198`
  (the baseline lane with the frozen v1 optimizer; Node zlib level 9, brotli quality 11; cleanup
  off). There is no v1 checkout on this machine, so it cannot be re-measured. It is recorded in
  `experiments/v2-restoration/RESULTS.md`.
- **"Now" column:** measured by the copy-aliasing verifier on its branch (the round-start tree plus
  that fix). The two link changes were verified byte-identical for both packages, so `main` should
  produce these figures, but `main` itself has not been built since they landed.
- **Run time:** not measured this round. The Rust-to-JavaScript ratios from round 1 stand
  (`marked` about 7×, `minisearch` about 42×, `fflate` about 18–31×). Timing was unusable
  throughout: load averages of 15 to 40 while agents were building.

## 3. How the work was run

- **Plan:** `~/.claude/plans/look-at-this-recursive-firefly.md`, approved 2026-11-11, recorded as an
  addendum in `tasks/todo.md`. Wave A closes open miscompiles; Wave B makes cleanup-edge removal
  acceptable and restores it; Stage C is a measured checkpoint; Wave D is the size and speed
  levers.
- **Decisions you took at the start:**
  1. The v1 target is the frozen historical row above.
  2. Host values are checked at typed boundaries; cleanup-edge removal returns only after that
     and after its proof gate is fixed.
  3. Constant pooling gets a rule that cannot leak by construction; the multi-chunk fix lands
     separately first.
  4. `as_flattened` gets full support, then a pay-per-use runtime helper if possible.
- **Execution model:** each item is implemented by a subagent (Opus, xhigh effort) in its own
  worktree and checked by a fresh verifier (the `fable` model, high effort). The Agent tool offers
  one Fable model, so the 5.1 version you named could not be pinned. A change is squash-merged to
  `main` only on `pass` or `pass-with-limits`.
- **Stage 0:** the full gate set was run once on `main` at `b1bcca4` and matched the round-1
  report: build, `check.sh`, 146/146 release fixtures, active-poll 60/60, cleanup transport and
  admission, extern 5/5 and standalone all pass; known gaps 5/6; `round1-open` 1/5 in both modes.

## 4. Verified and on `main`

| Item | Commit on `main` | Verified branch commit | Verdict |
|---|---|---|---|
| A6 multi-chunk ownership, static cycles | `cf80163` | `4b3d379` | pass-with-limits |
| A6b chunk hardening | `7979702` | `e7880e0` | pass-with-limits |
| A1 copy aliasing (ledger 1c) | `1362e12` | `35472d3` | pass-with-limits |

### A6: a chunk declares only what it owns; statics that refer to each other initialize

- **Bugs fixed:** a multi-chunk program re-declared an item it also imported (`SyntaxError` at
  load); statics that refer to each other, or to themselves, hit the temporal dead zone at load
  (`ReferenceError`). Both are valid Rust.
- **Rule:** a unit places only the names it owns. Bindings are ordered by strongly connected
  component; a cycle is declared empty and filled in a second step, so identity is preserved. A
  cycle member that is not one object or array literal, or a cycle that chunking would split, is a
  link error. No constant-pooling code came with the extracted fix.
- **Verifier's evidence:** the new ordering equals the old one on 40,000 random acyclic item
  tables; existing fixtures byte-identical in four modes; both packages byte-identical; chunk
  suite 6/6 in six configurations; 264 generated static-graph runs match native.
- **Files:** `jsc-link/src/link.rs`, new `jsc-link/src/link/statics.rs`, `scripts/chunk-test.sh`,
  `scripts/run-chunks.sh`, fixtures 260–264.
- **Cost:** about 22 minified bytes per cycle member; nothing for programs without cycles.

### A6b: refuse a chunk split whose initializers read around an import cycle

- **Rule:** a split is a link error when a module-level binding in one chunk file reads a binding
  owned by a file that imports the first back, directly or through other files. Functions may
  cross such a cycle. A cycle member with a `__proto__` key is refused.
- **Verifier's evidence:** 7,393 chunk splits relinked with the old and new linker and loaded with
  every chunk file first. All 1,333 newly refused splits threw at load before; no split that
  loaded in every order is refused; no accepted split throws.
- **Also:** `scripts/check.sh` now runs the chunk suite (13/13), so CI does too. Fixtures 265–268.
- **Accepted limit:** a `__proto__` field nested inside a cycle member is refused although it
  happened to run before.

### A1: a value used as storage is declared

- **Bug fixed:** a write to a by-value copy landed in the copy's source or in an operand
  (`let mut p = PS[4]; p.x = 99;` changed `PS[4]`). This is the `copy_alias` case in
  `examples/round1-open/`. The first attempt last round failed under Rust-level inlining on a
  `Cell::set` store; that shape is now right.
- **Rule A** (`jsc-link/src/lower/fuse.rs`): a single-use value is not spelled inline when its use
  writes it in place or takes a view rooted at it that never dereferences, shared or mutable.
  **Rule B** (`lower/ssa.rs`): a copy that shares its source's value gets its own before a
  projected write.
- **Verifier's evidence:** 797 of its own cases in up to seven configurations all match native on
  the optimized lane; the 667-case generated corpus matches on 661 (the six misses are heap-array
  cases belonging to A2); no regression across 128 fixtures in seven configurations; fixtures
  116, 119, 130, 131 and `lost_write` still pass.
- **Cost that landed with it:** compact +527 / +298 / +238, idiomatic +596 / +378 / +329; 82
  existing fixtures changed bytes (+5,051 B in total); flat-ABI choices flipped in 17 fixtures,
  and `23_format` gained in-loop allocations. No decision flipped in either package.
- **Recoverable:** the verifier built a variant that exempts three classes the rule declares
  needlessly (a fresh aggregate borrowed whole and handed to a callee, call results, views read by
  the adjacent dereference). It makes both packages byte-identical to before the fix and existing
  fixtures +14 B, with the same test results. It is an unreviewed experiment; implementing it
  properly is follow-up A1b.

## 5. Delivered on branches, no verdict

None of these is on `main`. The claims below are the implementers' own. The verifiers for the
flattening and `#[track_caller]` rounds are recorded as stopped when the session ended; the
niche-constants verifier had not reported and I have no completion record for it. Their partial
work is in the scratch directories named in section 10.

| Item | Branch | Head | Rounds so far |
|---|---|---|---|
| A4 niche constants (1i) | `worktree-agent-a0a61a0aaa5ae144a` | `e2245df` (16 commits) | failed twice, third round delivered |
| A2 flattening, heap arrays (1d, 1f) | `worktree-agent-afbe5ab2cda06b284` | `96b89e3` (10 commits) | failed once, second round delivered |
| A5 `#[track_caller]` (1h) | `worktree-agent-af471d6991ec28a6b` | `a420032` (2 commits) | failed once, second round delivered |

### A4: niche-optimised constants

- **Reported bug:** `Some(&x)` inside constant data read back as `None`.
- **Root causes found:** a niche tag was read by its raw bytes although a pointer's bytes are only
  an offset; a register-sized pointer constant at a non-pointer type was lowered as a pointer of
  that type; the linker rebuilt the compiler's niche numbering with the wrong rule.
- **Round 1 failed:** an enum laid out as a single present variant that is not variant 0 still
  decoded wrongly on the optimized lane; and static cycles through `Option<&'static T>` that ran
  on `main` died at load. The second point is resolved by A6, which landed first.
- **Round 2 failed:** the niche work held against 600 generated enums, but two things I had not
  asked the first round for were wrong. A one-element slice of a static lost pointer identity and
  whole-element writes; an `Option<fn>` constant holding a closure called to `0`.
- **Round 3 claims:** the one-element slice uses the static's own slot; closure-as-`fn`-pointer
  constants emit the wrapper the run-time coercion builds; `Option<&dyn Trait>` constants are a
  compile-time refusal for static and promoted targets; the test interpreter evaluates what it can
  and skips only what it got wrong before. Packages byte-identical to the round-start tree.
- **Things to weigh before landing:** five existing async and pointer fixtures grow by 104 bytes
  each because the standard library's no-op waker entries become wrappers; the linker finds
  closures by a linear scan per function-pointer constant; one mutant survives; sixteen commits
  for one reported bug.

### A2: flattened arrays and arrays on the heap

- **Reported bugs:** `as_flattened()` read `undefined` past the first row; `Box<[u32; 8]>` read
  `undefined` and `Vec<[u32; 3]>` aborted.
- **Round 1 failed on four silent wrong answers,** two of them regressions: a one-byte heap value
  viewed as `&[u8; 1]` was destroyed (`Box<u8>`, a one-character `String`, `vec![true]`); a fresh
  `Vec<u8>` viewed through a `[u8; 4]` pointer cast was retyped. The common cause was the runtime
  guessing what a block is from sizes and shape.
- **Round 2 claims:** a block is rows only if a cast whose types say so recorded it; every round-1
  finding matches native in all four modes; the added hot-path lookups are gone; the base
  lost-write through `&mut [T; N]` is fixed as well.
- **Size, the main concern (implementer's figures, against the round-start tree):**

  | | Before | A2 round 2 | Change (min / gzip) |
  |---|---|---|---|
  | Compact | 241,745 / 61,301 | 246,344 / 62,514 | +4,599 / +1,213 |
  | Idiomatic | 443,971 / 111,722 | 450,115 / 113,362 | +6,144 / +1,640 |
  | Ten-workload corpus | 632,834 / 173,155 | 651,460 / 178,494 | +18,626 / +5,339 |

  All of it is shipped runtime. Nearly every program carries the row support because `vec![x]` of
  a non-byte element allocates a one-row heap block.
- **Other open points:** row addresses taken through a reference are still silently wrong; byte
  rows (`Vec<[u8; N]>`) are read through a `Proxy` view; two fixtures gain real in-loop
  allocations from re-priced decisions.

### A5: `#[track_caller]` chains and function pointers

- **Root causes (both reproduced by the first verifier):** a function-item value named the
  direct-call instance, which expects a location no pointer call passes; a shim and the function
  it wraps shared one item identity, so which function broke depended on hash order.
- **Round 1 held on everything except one regression:** locations matched native on chains,
  pointers, trait methods, other crates and 168 standard-library panic programs; fixtures,
  packages and the ten-workload corpus were byte-identical. It failed because a keyed-list
  callback that is a tracked function stopped compiling. Nothing in the repository uses that
  combination.
- **Round 2 claims:** the keyed callback compiles again and calls the shim; a committed fixture
  (255) covers the host-callback change. The baseline lane was touched this round.
- **Stated gap:** reverting the keyed-callback change is caught only by scratch programs, because
  no gate in this checkout can compile a keyed program.

## 6. Verification record

| # | Change | Verdict | Deciding finding |
|---|---|---|---|
| 1 | A4 round 1 | fail | wrong variant through a single-present-variant enum; cyclic statics died at load |
| 2 | A6 | pass-with-limits | — |
| 3 | A2 round 1 | fail | four silent wrong answers, two regressions |
| 4 | A4 round 2 | fail | one-element static slice; closure `fn` constant called to `0` |
| 5 | A5 round 1 | fail | tracked keyed-list callback no longer compiles |
| 6 | A6b | pass-with-limits | — |
| 7 | A1 | pass-with-limits | — |
| 8–10 | A4 round 3, A2 round 2, A5 round 2 | not finished | — |

Three implementer claims were shown to be wrong by control runs: "pre-existing" evidence that was
really a missing sysroot, a test failure reported as intermittent that is deterministic, and a
limit listed as inherited that the change had introduced.

## 7. Correctness inventory

| Inventory | Round start | `main` now (expected) | With the unverified branches |
|---|---|---|---|
| `round1-open` (5 cases, both modes) | 1/5 | 2/5: `copy_alias`, `lost_write` | 4/5: plus `niche_constant`, `flattened_safe` |
| October-derived known gaps (6 cases) | 5/6 | 5/6 | 5/6 |

`address_fields` (pointer records recognised by field name) is not started; it waits for A2
because it edits the same files. Aggregate-place identity stays deferred pending a design review.

## 8. Miscompiles and defects found on `main` this round

All of these exist independently of the round's changes. "Safe" means reachable in safe Rust.
Reproductions are in the verifier scratch directories (section 10).

**Constants and statics**

| Defect | Effect | Status |
|---|---|---|
| A constant pointer to the first field of a static (`&S.f0`, `&ROWS[0]`) is emitted as the static itself | silent wrong value, safe, all modes | queued A7. Newly reachable in cyclic programs since A6, which used to crash at load first |
| A `&dyn Trait` constant pointing at a static loses its vtable | `TypeError` | queued A7; a compile-time refusal on the A4 branch |
| A struct field named `__proto__` is printed as a literal key | wrong value | queued A7 |
| A slice or `&str` pointing into a static loses its length | wrong or panic | fixed on the A4 branch, unverified |
| A closure stored as a `fn` pointer in a constant calls to `0` | silent wrong value | fixed on the A4 branch, unverified |
| `const C: &[T] = &STATIC` used in a function body | crash, or refused in release | still refused in release on the A4 branch |
| A `const` struct whose only field is a `fn` pointer | `TypeError` | recorded |
| `&fn_item`, or a function item coerced to `&dyn Fn` | compile refusal | recorded |

**Enums, copies, arrays**

| Defect | Effect | Status |
|---|---|---|
| A written copy of an enum with more than 8 variants and an aggregate payload changes its source | silent, safe, all lanes | queued A8 |
| A write through `[_, rest @ ..]` of an array is lost | silent, safe, all lanes | queued A8 |
| A whole-array store through `&mut [T; N]` from a subslice is lost | silent, safe | fixed on the A2 branch, unverified |
| An enum with one inhabited variant plus a kept dead variant throws "unreachable" with the optimizer off | crash | recorded |
| `slice.as_ptr()` against `&array[i]` is false for aggregate elements | wrong comparison | recorded |
| `Box<[u8]>::from(&[u8])`, `concat` of tuple rows, hashing a `[u32]`, `swap_with_slice`, `rotate_*`, `sort_by_cached_key` at n = 21 | aborts or compile refusals | recorded |

**Calls and locations**

| Defect | Effect | Status |
|---|---|---|
| A tracked function containing any virtual call, when the body owes cleanup | compile error in release profiles | queued A5b; needs a format change |
| Tuple and closure clone shims write `_0[0]` before `_0` exists | throws | recorded |
| A `self: Box<Self>` method on `Box<dyn Trait>` | throws | recorded |
| Out-of-bounds `Vec::remove` on optimized lanes | `RangeError` instead of the panic handler | recorded |
| A tracked call inside a Rust-inlined body, cleanup off | reports a location inside the callee | recorded |

**Baseline lane and harness**

- Under the default profile with `-Oz` and Rust-level inlining: an operand write through a shared
  view, a lost write through `as_array_of_cells`, and a panic on the second of two futures. Not
  reachable from release profiles today, since no item falls back to that text.
- No gate runs the fixture suite under `-Oz` with Rust-level inlining, the mode the compact
  package uses. Two mutants of the copy-aliasing rule are caught only there.
- Stale or failing on `main`: emit goldens `04_loop_break` and `18_enum_shapes`, two extern
  goldens on the optimized lane, `26_async` over budget in the default profile,
  `flat-result-test.sh`.

The full list, with paths, is in `tasks/todo.md` under the round-2 section.

## 9. Not started

| Item | What | Depends on |
|---|---|---|
| Integration gate run | all gates on `main` at `1362e12` | nothing; owed first |
| A1b | recover the copy-aliasing size cost with the tighter rule | — |
| A1c | fixture gate under `-Oz` with inlining; pin one untested path | — |
| A2b | drop runtime members a program cannot reach, at link time | A2 landing |
| A3 | pointer records recognised by field name (1e) | A2 landing |
| A5b | tracked body with a virtual call | A5 landing |
| A6c | three test gaps and one doc sentence from the A6b verification | — |
| A7 | constant pointers into statics | A4 landing |
| A8 | large-enum copies; array rest-pattern writes | — |
| Wave B | typed host-boundary checks, proof gate, then cleanup-edge removal and its extensions | Wave A |
| Stage C | one-tree measurement against the v1 targets, fresh attribution | Wave B |
| Wave D | inlining, emission wins, pooling, formatter and String work, runtime | Stage C |

Wave B holds the largest known size lever: about 33 K minified and 5.4 K gzip on the compact
package from cleanup-edge removal, plus about 9 K from its extensions.

## 10. Repository, branches and evidence

- **`main`:** `1b582b8`, clean, 105 commits ahead of `origin/main`, nothing pushed. Its compiler
  binaries were last built at `b1bcca4`.
- **Round-2 commits on `main`:** three code commits (`cf80163`, `7979702`, `1362e12`) and eight
  ledger commits.
- **Branches kept as provenance:** the three verified branches and `a6b-hardening`, plus the three
  unverified branches in section 5.
- **Worktrees:** 43 in total: the 23 preserved from round 1, untouched, and 19 created this round.
  The round-1 stash entry is untouched.
- **Disk:** 166 GB free (82% used) when checked. It reached 94% mid-round. I deleted the
  git-ignored `target/` and `build/` directories in six finished verifier worktrees; nothing else
  was removed.
- **Evidence:** the session scratchpad,
  `/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/`.
  It holds `briefs/` (every implementer and verifier brief), `baseline/` (Stage 0 gate logs and
  the gate script), one directory per implementer (`a1`, `a2`, `a4`, `a5`, `a6`, `a6b`) and per
  verification (`verify-a1`, `verify-a2`, `verify-a2-r2`, `verify-a4`, `verify-a4-r2`,
  `verify-a4-r3`, `verify-a5`, `verify-a5-r2`, `verify-a6`, `verify-a6b`). **This is a temporary
  location and is not backed up.**
- **This report** is written to the repository root and is not committed.

## 11. What the round taught

The full entries are in `tasks/lessons.md`.

- **Agent worktrees start from the remote-tracking ref, not local `main`.** The first launch of
  three implementers produced nothing. Every brief now begins with a fast-forward of the agent's
  own branch.
- **A send-back carries only the verifier's fail reasons.** The second niche-constants failure was
  on work I added to its second round, not on the reported bug.
- **"Pre-existing" and "intermittent" need a control run on `main` in every mode** before they are
  repeated.
- **Disk is a real limit.** Each agent builds its own compiler and sysroots, 10 to 57 GB per
  worktree.
- **Passing gates is weak evidence here.** Every failed verification passed all gates, both
  packages and the kernels.

## 12. Decisions for you

1. **Whether to resume, and how to finish the three open verifications.** Their transcripts are
   saved, so each can be resumed or replaced with a fresh verifier.
2. **A2's size cost.** As delivered it adds about 4.6 K minified to compact. Either land it once
   verified and recover the bytes in A2b, or hold it until A2b exists. I would land it once
   verified: it fixes silent wrong answers, and the verifier of round 1 judged about 910 bytes
   soundly recoverable with a fact the linker already computes.
3. **A4's scope.** If its third verification fails, I would drop the slice-into-static and closure
   work from it and move both to A7, keeping the niche decoding, which has held twice.
4. **Evidence preservation.** The scratchpad is temporary. Copying the briefs and the verifier
   programs into the recovery backup directory would protect them.
5. **Worktree cleanup.** The round created 19 worktrees. Removing the finished verifiers' ones is
   safe; I have not done it without your say-so.
6. **Still open from round 1:** dropping the leftover stash entry, retiring the Topcoat DOM and
   keyed fixtures, and what to do with the full pointer model.

## 13. Suggested order when work resumes

1. Run the integration gates on `main` and rebuild its binaries.
2. Finish the three open verifications; land what passes, on top of the integration result.
3. A1b, to return the packages to their round-start bytes.
4. A7 and A3 once A4 and A2 land; A2b alongside A3.
5. Add the `-Oz` fixture gate (A1c) before more lowering changes.
6. Wave B, which is where the first real size reduction of this iteration comes from.

## 14. Reproducing the checks

From the repository root, after `bash scripts/build.sh --locked`:

```sh
bash scripts/check.sh
SKIP_BUILD=1 JS_EXTRA_ARGS='js-profile=release-balanced js-minify=on' bash scripts/test.sh --require-core
SKIP_BUILD=1 bash scripts/active-poll-cleanup-test.sh
SKIP_BUILD=1 python3 scripts/cleanup-transport-test.py
SKIP_BUILD=1 bash scripts/cleanup-admission-test.sh
SKIP_BUILD=1 bash scripts/extern-test.sh
node scripts/standalone-check.mjs
python3 scripts/check-known-gaps.py          # expected 5/6; exits nonzero by design
python3 scripts/check-round1-open.py         # expected on main: copy_alias and lost_write pass
JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on python3 scripts/check-round1-open.py
```

Package builds and kernel checks use the commands in `round_1_integration_report.md` §15 and the
verification receipt `docs/round1-promotion-verification.json`. The gate script used for the
Stage 0 baseline is `baseline/run-gates.sh` in the scratchpad.
