# Dev-loop speed (plan: ~/.claude/plans/replicated-leaping-cascade.md, approved 2026-11-14)

Kept apart from `tasks/todo.md` because another session is editing that file. Fold it in later.
Execution: implementer Opus 5.5 at xhigh, then verifier Fable at high, one item at a time. Every merge into main needs the user's OK.
Order (user, 2026-11-14): Item 1 now. Phase 0 waits until the F1 verifier is done and the 1-minute load is below 8.

- [ ] Item 1: reuse node in the Rust tests. Branch `devspeed/node-pool`, worktree `../mires-devspeed-node`, base a868dcb.
  - [x] implementer: commit 90e08bb, new dev-only crate `test-node`. 420 of 427 programs pooled; 12 node processes instead of 427; 2.3 s instead of 13.3 s (a note, the machine was shared). Release dylib byte-identical (dbe5ec4f…).
    - Not pooled: the 7 sites in `rcgjs-object/src`. **Finding:** any byte edit in `rcgjs-object/src`, even a comment in `#[cfg(test)]` code, changes the dylib's metadata (crate hash, UUID, signature), so every sysroot is invalidated. Likely cause: rustc's crate hash covers all source files it loads. If so, tests in an out-of-line `#[cfg(test)] mod tests;` file wouldn't have this cost. That is unverified; follow up.
  - [x] verifier round 1: **FAIL**. The vm contexts give false passes: strict-mode undeclared function assignment on node 24; `process` and exit codes; async work that isn't tracked. None of the 427 real programs is affected. Pass sets, the dylib, orphans and clippy are all fine.
  - [x] round 2 (user, 2026-11-14): fresh `worker_threads` per program instead of vm contexts, at about 10.7 ms per program measured on node 26. The send-back holds only the verifier's 3 fail reasons, plus dropping the unused Eval, File and Check modes.
    - implementer: commit 97e4696. It reports 0 false passes on node 24 and 26. Two extra fixes were needed: the thread stack is capped at 1.125 MB so recursion depth matches node, and node's stdin teardown is reproduced. `cargo test -p rcgjs-object -p jsc-link` takes 4.4 s pooled against 9.6 s with spawning. The dylib is still byte-identical.
  - [x] verifier round 2: **FAIL, one cause.** A thread can emit `error` and still exit 0, so the pool reports a pass (`worker.js:133-147`). Seen with exit listeners that throw, and with a throwing microtask followed by `process.exit(0)`. Every other check passed, including stack-depth parity over 86 shapes. Second fail, so per the lessons it is split: round 3 is only this fix plus its regression tests.
  - [x] round 3: `error` event ⇒ status 1 when the exit code is 0. Commit ceaf089 adds 2 regression cases (each fails when the fix is reverted). On both node versions the only remaining false passes are hostile or by construction: `abort`, the thread-identity checks, a forged frame. Pass sets match; dylib `dbe5ec4f…` is unchanged.
  - [x] verifier round 3: **PASS**. Reverting the fix brings 17 false passes back and makes the regression cases fail. At HEAD the only false passes left are hostile or by construction (527, 700, 701, 703, 817) on node 24 and node 26. The verifier's 96 new cases give none. Pass sets are identical (1084 ok, 7 ignored, plus 7 `test_node` tests) in pool and spawn mode on both node versions. The dylib is byte-identical. clippy and fmt are clean.
    - Follow-up: the comment at `worker.js:145-147` should say that node itself exits 0 in cases 503, 504 and 505; the pool fails those now, which is the safe direction.
  - Follow-ups queued (not merge blockers):
    - a nonce and surplus-reply check against forged frames;
    - a process-wide V8 flag set by one program breaks the later programs on that worker;
    - the worker's stderr log is never cleared between programs;
    - `RCGJS_TEST_NODE_TIMEOUT` overflows `setTimeout`;
    - exit codes are not reduced to 8 bits;
    - SIGILL on a node 24 regexp depth case with a 1.125 MB stack (node's own behaviour);
    - the RSS figure (actually ~12 KB/program drift).
  - Leftover from the round-2 verifier: an empty FIFO at `/var/folders/h3/__nb11g92yn7xr9cryz2xls80000gn/T/tn-verify-r2-fifo-60603-1794660405907`. A safety check blocked the agent's removal; it's the user's to delete.
  - [x] merge (user, 2026-11-14): main had moved to 956edd2 (the other session's 10 commits), so a fast-forward wasn't possible. The test merge 4b57b1d had no conflicts. On the merged tree, both pool and spawn modes ran 1094 tests with 0 failures; clippy and fmt were clean; and no test from main was lost (only the 7 `test_node` tests are new). The user chose a merge commit, and main was fast-forwarded to 4b57b1d. The worktrees `mires-devspeed-node` and `mires-devspeed-merge` are removed; the branches are kept.
- [ ] Phase 0: serial baselines in 3 modes (reference `.js` saved), per-fixture split (rustc / jsc-link / node), cargo test ranking
- [ ] Item 2: incremental release build for jsc-link only
- [ ] Item 3: test.sh does less work (3a brotli size cache, or 3b reuse the rustc half; Phase 0 decides)
- [ ] Item 4: bounded parallel fixtures in test.sh (the user decides the default)
- [ ] Item 1b, pool follow-ups (user, 2026-11-14: "go ahead"). Branch `devspeed/node-pool-followups`, worktree `../mires-devspeed-pool2`, base 4b57b1d. One commit per fix:
  1. a nonce and a surplus check on replies;
  2. V8-flag taint retires the worker;
  3. stderr is attributed to the program that wrote it;
  4. timeout bounds;
  5. 8-bit exit codes;
  6. if the worker crashes or breaks the protocol, the program is run once with a spawned node;
  7. wording fixes and the temp-file leak.
  - [x] implementer: 7 commits, bc8b09a..f7271d8. Every regression test fails on its parent commit. On node 24 and 26 the only false passes left are 527, 700, 701 and 703; 817 is now "same". The crash fallback fixed the false fails 810, 812, 137, 710 and 688 (n24). 1111 tests in pool and spawn mode on both versions.
    - Residual risk I asked the verifier to treat as top priority: the fallback spawn has **no timeout**, which may be a new hang path.
  - [x] verifier round 1: **FAIL**.
    - (1) The fallback spawn has no bound (`lib.rs:288` → `:144`). Programs that break the protocol or kill their worker *and* never finish now hang `cargo test`; 4b57b1d failed them in about 3 s. Plausible non-hostile triggers: an inherited-stdio child, the node 24 deep regexp (688), and a leftover child echoing into the next program.
    - (2) The forge test frame is in the old layout, so with the nonce check removed every test still passes.
    - Everything else passed.
  - [ ] round 2: bound the fallback by the request deadline, don't fall back after a timeout reply, and fix the forge-test frame. Only the 2 fail reasons go into the send-back.
  - Follow-ups queued:
    - the surplus check has no test that fails without it;
    - the fallback runs a program twice (2070/2071), and the docs don't say so;
    - an inspector `connectToMainThread` taint;
    - nested-Worker `setFlagsFromString`;
    - late stderr from a child that outlives its program;
    - microtask `exit(-1)` after an error returns 255 where node returns 1.
  - [ ] merge (ask the user)
- [ ] Item C: decode the jsc-link test corpus once and share it across the 5 corpus tests. User, 2026-11-15: "Decode once, share", with no coverage change. Branch `devspeed/shared-corpus`, worktree `../mires-devspeed-corpus`, base 16aafc8.
  - [ ] implementer
  - [ ] verifier
  - [ ] merge (ask the user)
- [x] Item 5: optimise third-party deps in the dev profile (user, 2026-11-14: "go ahead"). Branch `devspeed/dev-deps-opt`, worktree `../mires-devspeed-deps`, base 4b57b1d. Invariants: the release dylib and the debug proc-macro dylibs (`libjs_extern_macro`) stay byte-identical, and there is no `.cargo/config.toml`.
  - [x] implementer: commit 16aafc8, `Cargo.toml` only. `"*"` is set to opt-level 2, with 6 proc-macro-closure packages pinned to 0.
    - Release dylib: byte-identical.
    - Macro dylibs: identical under `CARGO_INCREMENTAL=0`. Incremental dev builds aren't reproducible even at the baseline.
    - Pass sets: identical.
    - Benefit, only when a corpus is present: `cargo test -p jsc-link --lib` CPU −28% (106.8 → 77.3 s). No change otherwise. A cold build costs about +10% CPU.
  - [x] verifier: **PASS**. The pin list is complete, and the release dylib and the non-incremental macro dylibs are identical. The root profiles provably don't reach the sysroot builds. With a corpus, CPU is −29%; a cold build is +7%.
  - [x] merge (user, 2026-11-15): main fast-forwarded to 16aafc8, and the worktree was removed.
- **Finding (from the Item 5 measurements):** with a fixture corpus in `build/`, as in the main checkout, `cargo test -p jsc-link --lib` takes **28–37 s** rather than about 8 s. Five non-hermetic corpus tests take 25–37 s each: `fixture_bodies_build_verify…`, `effects summaries_admit…`, `dce_preserves…`, `pea_preserves…` and `range_preserves…`. Dedup and gating were deferred to the user (plan Item C).

## Review
(pending)
