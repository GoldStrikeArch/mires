# A1: copy aliasing (ledger id "1c", round 2)

Read `COMMON.md` in this directory first; its rules and gates apply. Scratch directory:
`SCRATCH/a1/`. Fixture numbers: **210–219 only**. Also read `tasks/todo.md` lines 120, 149 and 150.

## The bug
A by-value copy that is later written prints its write into the source, or into an operand.
Smallest form: `let mut p = PS[4]; p.x = 99;` changes `PS[4]` (static, local-array and
slice-parameter sources). Public reproduction: `examples/round1-open/copy_alias.rs`, native
expectation `copy_alias.expected`; it fails on main in both modes.

Two causes were identified last round: (A) `jsc-link/src/lower/fuse.rs` fuses a single-use value
even when it is then used as storage; (B) copy lowering in `jsc-link/src/lower/ssa.rs`.

## What already exists
A first-round fix is uncommitted in the old worktree
`/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-aa0340f7b4c30f8c0`: changes to
`lower/fuse.rs` (adds `uses_as_storage`), `lower/ssa.rs`, `lower/js/tests.rs`, plus untracked
fixtures `123_dead_copy_writes.*` and `124_transparent_copy_writes.*` (renumber into your range).

It was independently verified: it held on the 249 generated wrong cases with zero regressions, but
**failed** on one shape. A store through a shared view with interior mutability (`Cell::set`
inlined at the Rust level, `JSC_RUST_MIR_INLINE=on`, into a fresh struct) prints `99007` where
native prints `5007`. Also the `SetDiscriminant` arm of the fix is pinned by no test. The verifier's
evidence is in `OLD/verify1c/` (`cex/`, `setdisc/`, `extent/`, `rand/`, `probe.py`); use its
counterexamples and its generated-program corpus.

## Done when
1. `copy_alias` passes in `check-round1-open.py` in both modes.
2. The `Cell::set` shape is right under MIR inlining, with its neighbours: `Cell`, `RefCell`,
   `UnsafeCell::get` writes; shared-reference views of a copy; discriminant stores into a copied
   enum; copies of nested aggregates; copies out of statics, consts, promoteds, locals, slices and
   boxed values. Copies that are only read must still fuse: keep a positive control showing fusion
   still happens where it is legal.
3. The `SetDiscriminant` arm is pinned by a test that fails when the arm is removed.
4. The generated extent corpus from `OLD/verify1c` (or a regenerated equivalent if it cannot be
   run) matches native in release-balanced and in the `-Oz` + MIR-inline configuration. Report how
   many programs and functions ran and how many matched. The six heap-array cases that were wrong
   at base (`Box<[T; N]>` family) belong to another item: list them, do not fix them.

## Files
- Yours: `jsc-link/src/lower/fuse.rs`, `lower/ssa.rs`, `lower/js/tests.rs` and what they directly
  need; `examples/`; `OPTIMIZER.md` / `CONTRACT.md` only if documented behaviour changes.
- Leave alone: `runtime/shim.js`, `backend/src/ptr.rs`, `backend/src/constant.rs`,
  `jsc-link/src/lower/js/constant.rs`, `jsc-link/src/link.rs`.
