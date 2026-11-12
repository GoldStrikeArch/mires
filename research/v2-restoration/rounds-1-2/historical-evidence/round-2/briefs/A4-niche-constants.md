# A4: niche-optimised constants (ledger id "1i")

Read `COMMON.md` in this directory first; its rules and gates apply. Scratch directory:
`SCRATCH/a4/`. Fixture numbers: **240–249 only**. Also read `tasks/todo.md` line 157.

## The bug
A niche-optimised `Some(..)` inside constant data reads back as `None`: a silent wrong answer in
safe Rust. Public reproduction: `examples/round1-open/niche_constant.rs`; native prints
`7 9 100 5 true 11`, mires prints `0 0 100 0 true 11` (expectation in `niche_constant.expected`).
It fails on main in both modes. Smallest earlier program: `OLD/verify4c-r2/probe/src2/o1.rs`.
Nobody has found the root cause yet; this is new work.

Leads to verify yourself (not conclusions):
- `backend/src/constant.rs`, `read_enum` (around lines 313–345): decoding an enum out of constant
  memory.
- `backend/src/intrinsics.rs`, `niche_from_scalar` (around lines 2059–2144).
- `jsc-link/src/lower/js/constant.rs`: constant emission in the linker.

The producer writes both the baseline-lane text and the JIR body for every item. Find out whether
the bug is in one lane or both, fix it wherever it is, and say which lanes' output changes.

## Done when
1. `niche_constant` passes in `check-round1-open.py` in both modes.
2. Decoding matches native for every way a niche can appear in constant data:
   - where the data lives: `const`, `static`, `static mut` read back, promoted constants
     (`&Some(..)`), associated and generic-position constants, const tables (arrays and slices of
     structs), and, as a control, the same values built at run time in locals;
   - what carries the niche: `Option<&T>`, `Option<&[T]>` / `Option<&str>` (fat pointers),
     `Option<Box<T>>` where const-constructible, `Option<NonNull<T>>`,
     `Option<NonZeroU8/U32/U64>`, `Option<fn()>`, `Option<char>`, `Option<bool>`, `Option` of a
     fieldless enum, `Option<Option<bool>>` and deeper nesting, an enum with one data-carrying
     variant and several niche-encoded unit variants, a niche in a field at a non-zero offset
     inside a struct payload, and a niche range that wraps;
   - both `Some` and `None`, and every variant of multi-variant enums.
3. Static initialisation order and identity are unchanged: a static that refers to another static
   still sees the same object, and nothing is initialised earlier or later than before.
4. If a layout genuinely cannot be decoded, it is a clear compile-time refusal, never a silently
   wrong value, and your report lists it. Decode from rustc's layout and variant facts, not by
   matching the shapes in the reproduction.
5. A mutation control: show the new tests fail with your fix reverted.
6. Your new fixtures also pass in the default profile (baseline lane) and with
   `js-lane=jir js-opt=off`.

If an existing fixture's or a package's bytes change, that program was affected by the bug: say
which constants were being decoded wrongly and whether its behaviour was wrong before.

## Files
- Yours: `backend/src/constant.rs`, `backend/src/intrinsics.rs` (niche handling only),
  `jsc-link/src/lower/js/constant.rs`, `examples/`, docs for this item.
- Leave alone: `runtime/shim.js`, `backend/src/ptr.rs`, `jsc-link/src/lower/fuse.rs`,
  `lower/ssa.rs`, `jsc-link/src/link.rs`.
- Do not add constant pooling or any constant-sharing optimisation. That is later, separate work
  that will build on a correct decoder.
