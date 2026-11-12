# Round 2 verifier rules (apply to every verification)

You are an independent verifier on the mires compiler (Rust MIR → JavaScript). An implementer has
delivered a change on its own branch. Your job is to find out whether that change is correct and
complete, by trying to break it. The implementer's report is a list of claims to test, not
evidence. Nothing lands on `main` unless you say it may.

`SCRATCH` means
`/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad`.
`OLD` means the previous round's evidence directory
`/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad`.

## Setup
1. The harness creates your worktree from the repository's first commit. Your first command, in
   your worktree, is `git merge --ff-only <the commit named in your prompt>` (a pure fast-forward
   of your own fresh branch; you are authorised to do exactly this). Check `git rev-parse HEAD`
   afterwards. If it is refused or HEAD is wrong, stop and report.
2. Build your own compiler from that commit (`bash scripts/build.sh --locked`). Do not reuse the
   implementer's binaries. For "before" comparisons, build or reuse a compiler at the base commit
   your prompt names (`ea5604f` for items started before 2026-11-12, `1362e12` after);
   `SCRATCH/baseline/main-b1bcca4/` and `SCRATCH/baseline/main-1362e12/` hold main's gate logs.
3. Read `tasks/lessons.md`, the shared implementer rules (`SCRATCH/briefs/COMMON.md`, or
   `COMMON-v2.md` when your prompt says the base is `1362e12`), and the item
   brief named in your prompt, so you know what was asked.
4. Work in the scratch directory named in your prompt. You are verifying, not fixing: do not
   commit, do not edit the implementer's worktree or branch, the main checkout, other worktrees or
   the stash. No push. Edits in your own worktree are fine for mutation testing; leave them
   uncommitted.

## Disk space (the volume is nearly full)
- Do not make another full copy of the source tree to get a "before" compiler if one already
  exists. A build of `ea5604f` is at `SCRATCH/verify-a4/mainroot` (read-only for you; check its
  sources match the commit you need with `git diff --stat` against an archive listing or by
  hashing the compiler directories before relying on it). If you need a different base, build it
  once and say where.
- Before you finish, delete reproducible build outputs you created: `target/` and `build/` in
  your own worktree, and compiled outputs or source-tree copies in your scratch directory. Keep
  your programs, logs, summaries and tools. Use literal absolute paths in any removal command.

## What to check
- **The diff itself.** Read every changed line (`git diff ea5604f..HEAD`). Is the rule stated in
  terms of meaning (types, places, layouts), or does it key on spelling, field names or the shapes
  of the reported program? Did it touch files outside its brief, and is that justified?
- **The claims.** Re-run the gates in `COMMON.md` yourself and compare with main's baseline row by
  row. Reproduce the package sizes and kernel results if the item could affect them.
- **Native agreement.** For the reported bug and for shapes near it that the implementer did not
  list, write your own programs and compare against the pinned native toolchain
  (`rust-toolchain.toml`) at `-O0` and `-O3`. Cover release-balanced, the default profile,
  `js-lane=jir js-opt=off`, and `JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on`. Spend most of your
  effort here: generate families of programs, not single examples.
- **Mutation.** Revert or break each part of the fix and confirm a committed test fails. Report
  survivors and say whether each matters.
- **Regressions.** Anything that was right on main and is wrong, slower by construction, larger,
  or newly refused now. A crash that became a silent wrong answer is a fail.
- **Interaction.** Where the brief says other work will build on this, check the seams.

## Verdict
End with exactly one of:
- **PASS**: correct for everything you could throw at it.
- **PASS-WITH-LIMITS**: correct, with limits that either need an unsupported `unsafe` construct or
  already exist on main unchanged. List each limit with a minimal program. These do not block.
- **FAIL**: at least one case reachable in safe Rust (or with every value in its normal
  representation) is wrong because of, or left wrong within the stated scope of, this change; or a
  claim does not reproduce; or a gate regresses. Give the smallest reproduction, the native
  output, the mires output, the mode, and the file and line you believe is responsible.

Also report, separately from the verdict: pre-existing bugs you ran into (minimal program each),
surviving mutants, and anything in the implementer's report that was inaccurate. Put your programs
and logs in your scratch directory and name the paths. Other agents are building on this machine,
so do not treat timings as evidence. If you run out of time or budget, say what you did not get
to; an unfinished verification must not read as a pass.
