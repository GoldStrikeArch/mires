# Round 2 implementer rules (apply to every item)

You are an implementer on the mires compiler (Rust MIR → JavaScript). You work alone in your own git
worktree. An independent verifier checks your work afterwards, so everything you claim must be
reproducible from what you commit and what you leave in your scratch directory.

`SCRATCH` below means
`/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad`.
`OLD` means the previous round's evidence directory
`/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad`.

## Setup, first thing
1. The harness creates your worktree from the repository's first commit (`4ab4bd6`), not from local
   `main`. So your first command, in your worktree, is
   `git merge --ff-only ea5604f1d159f06bd8d9814e303659685642b959`.
   This is a pure fast-forward of your own fresh, clean branch and you are authorised to do exactly
   this. Then `git rev-parse HEAD` must print `ea5604f…` ("docs: round-2 plan, user decisions and
   frozen v1 size targets"). If the fast-forward is refused, the tree is not clean, or HEAD is
   anything else afterwards, stop and report.
2. Read `tasks/lessons.md` in full (short; it records mistakes already made on these items), then
   `round_1_integration_report.md` §6–§7. `CLAUDE.md` applies.
3. Create your scratch directory (named in your item brief). No temporary files in the repository.

## Rules
- Old worktrees under `/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-*` and
  `…/integration` are **read-only patch sources**. Their base (`2f36c3f`) contains a cleanup-edge
  pass that is deliberately not on main. Read them with `git -C <path> diff` / `status` and by
  copying untracked files out. No checkout, commit, stash, clean or edit there. Re-apply changes by
  hand onto your worktree; never merge or cherry-pick those branches.
- Do not touch the main checkout, the stash, or any branch other than your own. No push.
- Stay inside the files your item brief lists. Other agents are editing the files it tells you to
  leave alone. If your fix seems to need one of those, stop and report why.
- Fix by meaning (type, place and layout facts), never by the spelling of an expression or the name
  of a field. A fix that turns a wrong answer into a crash in a supported program, or a crash into a
  silent wrong answer, is not acceptable. Probe the neighbouring shapes, not only the reported one.
- Every `.expected` file comes from the pinned native toolchain (`rust-toolchain.toml`), checked at
  `-O0` and `-O3`. Use only the fixture numbers reserved for your item.
- Test in the modes the packages actually use: release-balanced, and
  `JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on`.
- If you find another pre-existing bug, record a minimal program in your scratch directory and
  report it; do not fix it.
- Other agents are building on this machine at the same time. Do not report timings as evidence.

## Gates (from your worktree root; run all before reporting)
```sh
bash scripts/build.sh --locked
bash scripts/check.sh
SKIP_BUILD=1 JS_EXTRA_ARGS='js-profile=release-balanced js-minify=on' bash scripts/test.sh --require-core
SKIP_BUILD=1 bash scripts/active-poll-cleanup-test.sh
SKIP_BUILD=1 bash scripts/extern-test.sh
node scripts/standalone-check.mjs
python3 scripts/check-known-gaps.py          # expected 5/6; exits nonzero by design
python3 scripts/check-round1-open.py         # your case plus lost_write must pass; others stay open
JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on python3 scripts/check-round1-open.py
```
A baseline run of these on unmodified main is recorded in `SCRATCH/baseline/main-b1bcca4/`
(`summary.tsv` plus one log per gate; it may still be filling in when you start). Compare your
`test.sh` table against it row by row: no fixture may go from pass to fail, and you report every
fixture whose output bytes changed. Do not assume any byte-budget row is waived.

Then build both packages (replace `ITEM` with your item id, e.g. `a1`):
```sh
node experiments/fingerprintjs-rust-compact/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --lane-report --output experiments/fingerprintjs-rust-compact/dist/ITEM
node experiments/tools/check-oss-kernels.mjs experiments/fingerprintjs-rust-compact/dist/ITEM/fp.esm.min.js
node experiments/fingerprintjs-rust/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --lane-report --output experiments/fingerprintjs-rust/dist/ITEM
node experiments/tools/check-oss-kernels.mjs experiments/fingerprintjs-rust/dist/ITEM/boundary/fp.esm.min.js
```
Kernels must be 58/58 on each. Report minified / gzip / brotli of `fp.esm.min.js` against main's
deterministic figures: compact 241,745 / 61,301 / 47,412; idiomatic 443,971 / 111,722 / 86,772.
If a package's bytes change, explain why.

## Disk space (the volume is nearly full)
Keep one base compiler and one fix compiler, not a copy per experiment. Before you finish, delete
reproducible outputs you created in your scratch directory (compiled programs, extra source-tree
copies, mutant build trees) and keep sources, logs, summaries and tools. Leave your worktree's own
`target/` and `build/` in place only if a follow-up round is likely; say in your report how much
space your worktree and scratch directory use. Use literal absolute paths in any removal command.

## Finish
Commit on your worktree's branch (one or a few clear commits). **Never** add a "Co-authored-by"
line. No push. Do not leave the deliverable as uncommitted files: an unchanged worktree is deleted
when you exit.

Report, as plain facts:
- worktree path, branch name, commit hash(es);
- root cause in two or three sentences and the rule the fix now enforces;
- files changed; fixtures and tests added and what each pins;
- each gate with its result, and every row that differs from the baseline;
- package sizes before/after and kernel results;
- what you did not do or could not verify, limits that remain, and pre-existing bugs you noticed.

State failures plainly. An unfinished item reported as unfinished is far more useful than an
optimistic summary.
