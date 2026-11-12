# A2: flattened arrays and arrays stored on the heap (ledger ids "1d" and "1f")

Read `COMMON.md` in this directory first; its rules and gates apply. Scratch directory:
`SCRATCH/a2/`. Fixture numbers: **220–229 only**. Also read `round_1_integration_report.md` §9 and
`tasks/todo.md` lines 131, 134, 136–138 and 149.

## The bugs (safe Rust; silent or crashing today)
- **1d:** `as_flattened()` covers only the first row: `[[1u32, 2], [3, 4]].as_flattened()[2]`
  reads `undefined`. `__rt.unwindow` in `runtime/shim.js` (around line 1143) steps into one
  element. Public reproduction: `examples/round1-open/flattened_safe.rs` (prints
  `1 2 undefined undefined`; native expectation in `flattened_safe.expected`).
- **1f:** arrays stored on the heap: `Box<[u32; 8]>` indexing prints `undefined` for `x[0]` in the
  default profile (native 8); `Vec<[u32; 3]>` aborts. Smallest programs:
  `OLD/verify-granule-min/probe/src/box1_body.rs` and its neighbours. Six more heap-array cases are
  in the generated corpus under `OLD/verify1c/extent/`.

## What already exists
A fix for 1d is uncommitted and **never verified** in the old worktree
`/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-aeaee6e0ef235816f`:
`runtime/shim.js` (+172), `backend/src/ptr.rs` (`build_fat` peels wrappers), `CONTRACT.md`, plus
untracked `125_as_flattened.*`, `126_flattened_pointers.*` and `scripts/runtime-flatten.test.mjs`
(renumber fixtures into your range). Review it critically, since nobody has, and extend it to 1f.
Its own notes list limits it left: dropping heap blocks of rows aborts; a `Vec<[T; N]>` grown by
`push` is stored flat; row addresses differ from flattened element addresses. Those are in scope.

## Decision already taken by the user
Full support, not a clean abort. The earlier fix cost about +2,166 minified / +605 gzip / +475
brotli of runtime in nearly every bundle. Keep the cost low, measure it, and say whether the helper
could be made pay-per-use (emitted only when a program can reach flattening) and what that would
take. Do not build a pay-per-use mechanism unless it is small and obviously safe; a clear proposal
is enough.

## Done when
1. `flattened_safe` passes in `check-round1-open.py` in both modes.
2. Reads **and writes** across row boundaries work for stack arrays, `Box<[[T; N]; M]>`,
   `Box<[T; N]>`, `Vec<[T; N]>` (built by `vec!`, `push`, `extend`, and grown past capacity),
   slices of rows, `as_flattened` / `as_flattened_mut`, `iter().flatten()`, `concat`,
   `chunks_exact` / `as_chunks` round trips, zero-length rows, zero rows and zero-sized elements.
   Element types: `u8`, `u32`, `u64` / `i64` (BigInt-backed), `f64`, a small struct, a reference.
3. Destruction is right on the normal path and the unwind path: elements with destructors inside
   heap blocks of rows are dropped exactly once.
4. Aliasing is coherent: a write through the flattened view is visible through the row view and
   the reverse; row addresses and flattened element addresses agree with native where a program
   can observe them.
5. Your new fixtures also pass in the default profile, where 1f was seen.

## Specific to `runtime/shim.js`
- `jsc-link` embeds the shim (`include_str!` in `jsc-link/src/plan/cost.rs`): rebuild the linker
  after every shim change before compiling on the optimized lane.
- Optimizer decisions are priced against the shim's text, so a shim edit can flip representation
  and flat-ABI choices in unrelated programs. **Report the list of programs whose output changed**
  (core fixtures, the ten-workload corpus, both packages) and what flipped in each (lane reports).
  Call out any flip that moves an allocation into a loop.
- Extra gates: `npm run test:runtime` (add your runtime tests there), and
  `npm run build:corpus && node experiments/browser-workloads/tools/run.mjs --pure`.
- Report the shim's size delta separately from the package deltas.

## Files
- Yours: `runtime/shim.js`, `backend/src/ptr.rs`, pointer/slice lowering under
  `jsc-link/src/lower/js/` that this needs (`ptr.rs` and neighbours), `examples/`,
  `scripts/runtime-*.test.mjs`, `CONTRACT.md` for this item.
- Do **not** change how `__rt.addr` recognises pointer records (`p.pr !== undefined`,
  `"buf" in p` and similar field-name tests). That is the next item in this lane and will be built
  on your commit. Do not add new field-name tests either.
- Leave alone: `jsc-link/src/lower/fuse.rs`, `lower/ssa.rs`, `backend/src/constant.rs`,
  `jsc-link/src/lower/js/constant.rs`, `jsc-link/src/link.rs`.
