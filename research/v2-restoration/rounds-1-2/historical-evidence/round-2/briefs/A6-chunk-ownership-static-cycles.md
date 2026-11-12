# A6: multi-chunk ownership and statics that refer to each other

Read `COMMON.md` in this directory first; its rules and gates apply. Scratch directory:
`SCRATCH/a6/`. Fixture numbers: **260–269 only**. Also read `round_1_integration_report.md` §8.4
and §11, and `tasks/todo.md` lines 121 and 151.

These are link-time correctness bugs. They were found while verifying constant pooling, and the fix
for the first one is bundled inside the pooling patch. The job is to land the correctness part on
its own, with **no pooling code**.

## Bug 1: a multi-chunk program fails to load
A program split into more than one chunk can both import an item and re-declare it: per-chunk
`order` re-emits an item another chunk owns when a chunk-owned static refers to it, giving
`SyntaxError: Identifier already declared` at load. Ledger line 121 names the place
(`jsc-link/src/link.rs`, around `:1711` and `visit` `:1946-1953` in the old numbering). The earlier
verifiers' two-chunk reproductions are under `OLD/verify4c/` and `OLD/verify4c-r2/`.

A fix exists, uncommitted, inside the pooling worktree
`/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-a12dd166f55ef46f8` (its `link.rs`
changes; the verifier re-checked "chunk double emission" as fixed there). Extract only the
ownership/ordering change. Leave behind everything about pools (`link/pool.rs`, `link/pool/`,
`lower/js/constant.rs` pooling, schema bumps, fixture `121_pooled_constants`).

## Bug 2: statics that refer to each other fail at load
`static A: Node = Node { next: &B, .. }; static B: Node = Node { next: &A, .. };` is valid Rust.
Today it stops at load with a temporal-dead-zone `ReferenceError`; a static-to-static reference
across chunks fails the same way. Minimal programs from another agent this round:
`SCRATCH/a4/src/p6_cycle.rs` (cycle through `Option<&'static T>`) and `p7_plaincycle.rs` (plain
references). On main the `Option` form decodes wrongly instead (a separate fix for that is under
verification), so use the plain-reference form as your reproduction and keep the `Option` form as
a probe to report on.

Give statics a declared, native-matching initialisation behaviour, including cycles and
self-reference (`static S: Node = Node { me: &S }`), with identity preserved (`ptr::eq` of the two
ways to reach a static is true) and no observable change to programs without cycles. If a full fix
for cycles turns out to need a design larger than this item (for example a change to how every
static is emitted), deliver Bug 1 on its own, and report a concrete design for Bug 2 with its cost
in bytes on programs that have no cycles, instead of forcing it in.

## Done when
1. Two-chunk and many-chunk programs load and run: chunk-owned statics referring to items owned by
   another chunk, both directions, functions and statics, with and without cleanup. Find how
   chunked output is requested in this tree (build tools and `jsc-link` options) and say so in the
   report.
2. Static cycles: two-node and three-node cycles, self-reference, a cycle through a `const` table,
   through a slice of references, through `Option<&'static T>` (probe; may depend on the pending
   decoder fix, say what you see), with interior-mutable statics (`AtomicU32`, `Cell` inside a
   `static mut`) in the cycle, in one chunk and across chunks.
3. Programs without cycles or chunks are byte-identical to main: both packages and every existing
   fixture. If any byte changes, explain why it must.
4. A mutation control: the new tests fail with the fix reverted.

## Files
- Yours: `jsc-link/src/link.rs` and what it directly needs under `jsc-link/src/link/`,
  `examples/`, test scripts for chunked builds, docs for this item.
- Leave alone: `runtime/shim.js`, `backend/src/ptr.rs`, `backend/src/constant.rs` and
  `backend/src/constant/`, `backend/src/jir/encode.rs`, `jsc-link/src/lower/` (all of it). If
  static emission order can only be fixed in `jsc-link/src/lower/js/constant.rs`, stop and report
  the design instead; another change to that file is under verification.
