# mires v2 optimization: round 1 report

Period: 2026-11-08 to 2026-11-11. State as of the pause on 2026-11-11: no work is running.

## Summary

The goal of this round was a v2 compiler whose output is smaller (raw and compressed) and faster at
run time, with drop-on-unwind cleanup kept on.

- The compact FingerprintJS package went from 515 KB minified / 110.6 KB gzip to 209 KB / 55.9 KB.
  The idiomatic port went from 822 KB / 186.5 KB to 381 KB / 97.7 KB.
- The optimized Pro link, which took about 11 minutes, now takes about 2.5.
- At run time, `marked` went from 146 times slower than the JavaScript original to about 7 times.
  `minisearch` and `fflate` are still roughly 42 and 18–20 times slower.
- These gains are verified, but most of them sit on the local `integration` branch. `main` only has
  the measurement harness, the compile-cost work, the zero-sized-type fix and the string runtime.
- Three further size changes are implemented and measured (together worth roughly 30 KB minified on
  the compact package, not additive) but have not passed verification.
- Independent verification found nine miscompiles that exist in the compiler today, most of them
  silent wrong answers in safe Rust. Three have fixes delivered, none of them landed yet.
- Nothing has been pushed.

The main finding of the round is about risk, not size: every optimization that was verified exposed
a latent bug that the old, more conservative output had been hiding. The correctness tail is now the
largest part of the remaining work.

## How the work was run

- **Plan:** `V2-OPTIMIZATION-RESTORATION-PLAN.md` was reduced to a focused compiler track with
  correctness first. The approved plan is in the ledger, `tasks/todo.md`.
- **Fixed decisions:** cleanup stays on in release profiles; fail closed (no destructor dropped, no
  exception swallowed, a crash never replaced by a silent wrong answer); pinned toolchain
  (nightly-2026-07-27); no push; nothing from `experiments-pro/`, `recovery/` or `build/` in git.
- **Execution model:** each item is implemented by a subagent (Opus 5.5, xhigh effort) in its own
  git worktree, then checked by an independent verifier (Fable 5.1 high at first, then Fable 5.2
  xhigh, and Fable 5.2 high from 2026-11-10). A change is committed only on `pass` or
  `pass-with-limits`. A fail goes back to the implementer once; a second fail goes to the user.
- **Measurement:** `experiments/v2-restoration/measure.mjs` builds the targets and records sizes
  (Node zlib only: gzip level 9, brotli quality 11), link cost and timings in
  `experiments/v2-restoration/RESULTS.md`. A paired Rust-versus-JavaScript timing harness with a
  same-build control lives in `experiments/browser-workloads/tools/`.
- **Targets:** compact OSS (`experiments/fingerprintjs-rust-compact`), idiomatic OSS
  (`experiments/fingerprintjs-rust`), the ten browser workloads, and the private Pro build.

## Results

### Size (`fp.esm.min.js`, bytes)

| Target | | Start (R0) | `integration` (2f36c3f) |
|---|---|---:|---:|
| Compact | minified | 515,481 | 209,000 |
| Compact | gzip | 110,636 | 55,869 |
| Compact | brotli | 80,491 | 43,533 |
| Idiomatic | minified | 822,496 | 381,395 |
| Idiomatic | gzip | 186,452 | 97,710 |
| Idiomatic | brotli | 135,211 | 77,454 |

The compact program went from 4,182 `try` blocks to 210. All 58 kernel checks pass on both targets.
`main` itself is still close to the starting sizes (compact 518,334 minified / 111,445 gzip), because
the size work is on `integration`.

### Compile cost

| | Before | After |
|---|---:|---:|
| Optimized Pro link (lane time) | 632 s | 146 s |
| Compact link | 70 s | 32 s |

Output is byte-identical before and after. Most of the remaining link time is brotli-11 compression
inside the optimizer's trials.

### Run time (Node, compiled Rust relative to the JavaScript original)

| Workload | Start | Now |
|---|---:|---:|
| marked | 146× | about 7× (8× Chrome, 12× Firefox) |
| minisearch | 68× | about 42× |
| fflate | 43× | about 18–20× (on `integration`) |

Timing on this machine was unreliable whenever several agents were building (differences of ±40% on
byte-identical code at high load). The figures above come from quieter runs. Later timing claims in
this report are marked as unconfirmed where that applies.

## What was done

### On `main`

| Item | Commit | What it does |
|---|---|---|
| Baseline | 93c0785 | The reconstructed standalone compiler, committed as one baseline |
| Compile-cost counters | 0111435 | `RCGJS_COST_PROFILE=1` |
| Measurement harness | f7e0e10, 6388b8a | `measure.mjs`, `RESULTS.md`, the R0 row |
| Compile cost (stage 2) | 9a940b3, a82e341 | Removes repeated work from cost trials and reuses unchanged lowerings; equivalent output |
| Zero-sized aggregates (1a) | dda8ae3 | Elided storage with exact drops; fixes three of the six known-gap cases |
| Test gates | a8fde34, 3b8d768, 1b19124, 3b41a80 | Stale gates made real: cleanup test, standalone async test, lock release, DOM suites that counted a skip as a pass |
| Timing harness | 75d30a2 | Paired Rust-versus-JavaScript protocol with pinned reference libraries |
| Runtime strings (5a) | 7bdd932 | ASCII fast path, byte-view reuse, cheap `str::len`, address allocator that reuses addresses; `marked` 88 ms → 3.7 ms per call |

### On `integration` (verified, not yet on `main`)

| Item | Commit | What it does |
|---|---|---|
| Fusion fix | cadcfb0, 0b04bdd | A raw pointer to a reference's pointee was built from the expression's spelling; a base miscompile that the next two items exposed |
| No-op landing pads (3a) | 9920693 | Removes landing pads that drop elaboration emptied; compact gzip 110.6 KB → 60.7 KB on its own |
| Cross-crate cleanup transport | e3869c3 | No cleanup information crossed crate boundaries before this; a soundness fix |
| Cleanup-edge removal (3b) | 60eadac | Drops the `try` around a call proven unable to throw |
| Overflow arithmetic (5d1) | 26f9ed4, 45edc42 | No closure per overflow pair; fixes 32-bit products that were wrong above 2^53 |
| Byte buffers (5e) | eec2d38, 9c85898 | One host operation per buffer crossing; roughly halves `fflate`'s gap |
| String builder length (5a2) | 57375cf | Builder keeps its byte length, so repeated pushes are linear; every appended piece is sanitized |

The last full check of `integration`: build clean, 1,067 Rust tests, 67 runtime tests, core suite
143/143 under the release-balanced profile, known gaps 4 of 6, kernels 58/58.

## Delivered but not landed

Sizes are for the compact package and were measured separately, so they overlap.

| Item | Gain or purpose | Status |
|---|---|---|
| No-throw extensions (4'a) | 209,000 → 200,050 minified, 55,869 → 53,517 gzip, `try` 210 → 99 | Failed twice. Waiting on a decision (see below) |
| Wider inlining (4'b) | 209,000 → 204,858 minified; idiomatic 381,395 → 372,861; workloads −3.5% minified; inlined sites 16 → 634 | Round 2 delivered, unverified. Blocked on fix 1g |
| Constant pooling (4'c) | 209,000 → 187,664 minified, 55,869 → 51,802 gzip | Failed twice on host isolation. Waiting on a decision. Also blocked on fix 1c. Includes the multi-chunk load fix |
| Address release (granule-min) | Loops that drop a `Vec`/`String` through a native form no longer abort after about a million rounds | Round 1 failed, round 2 delivered, unverified |
| Pointer element moves (1b-min) | Swapping or sorting arrays of references no longer overwrites the pointee; known gaps 5 of 6 | Delivered, unverified |
| Fix 1c (copy aliasing) | See the bug table | Failed once on one remaining shape; needs a second round |
| Fix 1d (`as_flattened`) | See the bug table | Delivered, unverified; costs about 2.2 KB of runtime per bundle |
| Fix 1g (lost write) | See the bug table | Delivered, unverified; no size cost on real code |

Parked: the full pointer storage-identity model (1b) failed verification three times, and the exact-size
address allocator failed twice. Both are kept as reference branches only.

## Miscompiles found on the base

All of these exist in the compiler today, independent of the changes above. Most are silent wrong
answers in safe Rust.

| Id | Bug | Example | State |
|---|---|---|---|
| 1c | A written value printed inline writes into its source, or into an operand | `let mut p = PS[4]; p.x = 99;` changes `PS[4]` | Fix delivered; 249 of 667 generated cases were wrong; one shape left (a `Cell` store into a fresh struct, under Rust-level inlining) |
| 1g | A store through a second name for an object is lost | `let p = if c { &mut a } else { &mut b }; p.f = 254;` | Fix delivered; 149 of 660 generated cases were wrong; does not need inlining |
| 1d | `as_flattened` covers only the first row | `[[1u32, 2], [3, 4]].as_flattened()[2]` reads wrong | Fix delivered |
| 1e | Address helper recognises pointers by property name | `struct P { pr: u32 }` → `(&p as *const P).addr() == 0` | Queued |
| 1f | Arrays stored on the heap | `Box<[u32; 8]>` reads `undefined`; `Vec<[u32; 3]>` aborts | Queued |
| 1h | `#[track_caller]` chains throw on the optimized lane | A `#[track_caller]` function calling another | Queued |
| 1i | `Some(..)` inside constant data reads as `None` | Niche-optimized `Option` in a `const` table | Queued |
| — | Multi-chunk builds can fail to load | A chunk both imports and re-declares an item | Fixed inside 4'c |
| — | Pointer elements overwritten when moved | `slice::swap` on `[&P]` | Fixed by 1b-min |

Smaller ones recorded in the ledger: two `String`s with equal text share an address; a `Vec`'s address
changes when it is converted and converted back; dropping boxes leaks address space; statics that refer
to each other fail at load.

Two structural findings:

- **The edge-removal safety net does not gate anything.** The second, independent analysis runs after
  edges are already removed, and nothing acts on its disagreement unless a debug flag is set. No
  miscompile is known from this, but a wrong rule would not be caught. Recorded as "3b-gate".
- **Optimizer decisions depend on unrelated runtime text.** Decisions are scored on compressed size,
  and the linker embeds the runtime source in that score. A 44-byte runtime change flipped decisions in
  other programs, in two cases moving allocations into loops.

## Decisions open

1. **Host values that do not match their declared type (blocks 4'a).** Typed bindings are not checked,
   so a host can hand a plain number where `u64` is declared; arithmetic then throws and, where a `try`
   was removed, destructors are skipped. Options: document it now and add boundary checks later
   (recommended); do the boundary checks first; or stop trusting arithmetic, which gives back most of
   the edge-removal gain.
2. **Host isolation for constant pooling (blocks 4'c).** Two rounds each found new ways for the host to
   reach pooled data. Options: a third round of the same approach; a rule that cannot leak by
   construction; or freezing pooled data at load.
3. **`as_flattened` fix cost (1d).** Full support costs about 2.2 KB of runtime in nearly every bundle;
   a clean abort instead would cost almost nothing but would not support multi-row flattening.
4. **The full pointer model (1b):** defer, retry, or declare unsupported.
5. **Trust declared host contracts by default:** about 7.6 KB minified, changes destructor behaviour on
   a throwing host getter. Currently opt-in.
6. **Housekeeping:** drop the leftover stash entry `s1b-a1ab8f94-r4-premerge-fusion`; retire the Topcoat
   DOM and keyed fixtures.

## Decisions taken

- Scope: compiler track with correctness first; four measured targets; one commit per stage; no push.
- Limits that need an unsupported `unsafe` construct, or that predate the change under review, are
  documented and do not block a stage. A case reachable in safe Rust still fails. The list is under
  "Accepted limits" in `tasks/todo.md`.
- ES2018/ES2020 is an acceptable baseline for compiled output. The existing `--ecma-target latest`
  switch was measured at −36 bytes minified on the compact package, so a real gain needs the compiler
  and runtime to emit newer constructs. Any API swap is benchmarked first.
- Verifiers run on Fable 5.2 at high effort from 2026-11-10.
- Pause from 2026-11-11: no new work until told otherwise.

## Not started

- **Correctness:** 1e, 1f, 1h, 1i, the edge-removal safety net, a clean abort for reinterpreting views,
  typed boundary checks.
- **Size:** shared unreachable-abort helper (about 8 KB minified), module-scoped runtime (about 5.5 KB),
  numeric enum tags and dead trailing arguments (about 4.5 KB), `#[inline]` hints passed from the Rust
  front end, the ES2020 target.
- **Run time:** heap-registry fast paths, string views that convert only at the host boundary, iterator
  and `Option` result objects, the formatter. A fresh profile should come first.
- **Process:** merge `integration` into `main`, take the R1 measurement (including Pro and browsers),
  consolidate the limits in `CONTRACT.md`.

## Lessons

The full list is in `tasks/lessons.md`. The ones that changed how the work is run:

- Test with the settings the package builds use. A fix passed five configurations and failed under
  Rust-level inlining, which the package and workload builds turn on.
- A fix that turns a crash into a silent wrong answer is not fail-closed; verifiers probe neighbouring
  cases, not only the reported one.
- After a second failed verification, split the work into a minimal fix instead of another full round.
- Timing taken while several agents are building is not evidence.
- Give each parallel agent its own fixture-number range. Six pending changes collide on numbers 121–126.

## Where things are

- **Ledger:** `tasks/todo.md` (status of every item, with file and line references) and
  `tasks/lessons.md`.
- **Measurements:** `experiments/v2-restoration/RESULTS.md`.
- **Branches:** `main` (a386c41 at the time of writing) and the local `integration` branch (2f36c3f),
  checked out at `.claude/worktrees/integration`.
- **Pending work:** uncommitted in worktrees under `.claude/worktrees/`. The ledger names the worktree
  for each item.
- **Evidence:** reproductions, probe programs and gate logs are under the session scratch directory
  `/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/`
  (one `verify*` directory per verification). This is a temporary location and is not backed up.

## Suggested order when work resumes

1. Verify fix 1g, then wider inlining (4'b) on top of it.
2. Second round for fix 1c, then constant pooling once its direction is chosen.
3. Verify the pointer-move fix, the address-release fix and the `as_flattened` fix.
4. Merge `integration` into `main` and take the R1 measurement.
5. Work through the queued miscompiles before starting new optimizations.
