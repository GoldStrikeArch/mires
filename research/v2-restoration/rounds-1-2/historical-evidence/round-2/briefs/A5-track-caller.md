# A5: `#[track_caller]` chains and function pointers (ledger id "1h")

Read `COMMON.md` in this directory first; its rules and gates apply. Scratch directory:
`SCRATCH/a5/`. Fixture numbers: **250–259 only**. Also read `tasks/todo.md` line 146.

## The bug
On the JIR lane (every release profile), a `#[track_caller]` function that calls another
`#[track_caller]` function, or one that is called through a function pointer, loses its implicit
location argument (`$loc`) and throws at run time. The default profile (baseline lane) is right.
This is safe Rust and it crashes a valid program; it is also how every panic message gets its
file and line, so a wrong location is a wrong observable answer even when nothing throws. The
earlier verifier's hand-written programs are in `OLD/verify4b/dt/hand/`.

Nobody has found the root cause yet. Establish it before changing code. Places to start:
how the producer encodes the caller-location argument for `#[track_caller]` instances and for
their function-pointer shims (`backend/`), how call lowering passes it on the JIR lane
(`jsc-link/src/lower/js/`, call and ABI lowering, the flat direct-call ABI in
`jsc-link/src/plan/abi_flat.rs`), and what the inliner and devirtualizer do with it
(`jsc-link/src/opt/inline.rs`, `opt/inline/remap.rs`).

## Done when
Compared against native, in release-balanced, `js-lane=jir js-opt=off`, the default profile, and
`JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on`:
1. Chains: a tracked function calling a tracked function two, three and four deep; each level
   reads `Location::caller()` and the values (file, line, column) match native at every level.
2. Indirect calls: a tracked function taken as `fn()` pointer, stored in a struct, a table and a
   `static`, passed as a generic `F: Fn()`, boxed as `dyn Fn()`, and called through each. Native
   reports the location of the pointer's creation shim; match it.
3. Tracked trait methods (inherent, trait impl, default method, through `dyn Trait`), tracked
   closures are not a thing so do not invent them, tracked `const fn`, tracked generic functions
   instantiated several ways, and tracked functions that are inlined, devirtualized, or given the
   flat ABI by the optimizer.
4. The standard library's tracked paths: `Option::unwrap` / `expect`, `Result::unwrap`,
   slice indexing out of bounds, `RefCell::borrow_mut` twice, integer overflow in a debug-assert
   build if the tree supports it, `unreachable!`, `assert!` — the panic message's location must be
   the user's call site, as native prints it. Use the tree's existing way of observing panic
   messages in fixtures (look at `10_panic` and the `guard_*` fixtures).
5. Evaluation order and effects are unchanged: the location argument must not be evaluated
   earlier or later than before relative to the other arguments' side effects, and no call gains
   or loses an argument it should not.
6. A mutation control: the new tests fail with the fix reverted.

The location literals are a known size cost (hoisting them was measured last round and made gzip
worse). Do not add a size optimization here. If your fix changes how many location literals a
program carries, report the count and the package sizes; correct locations come first.

## Files
- Yours: call and ABI lowering under `jsc-link/src/lower/js/` (not `ptr.rs`, `place.rs`,
  `unsize.rs`, `constant.rs`, `value.rs`), `jsc-link/src/plan/abi_flat.rs`,
  `jsc-link/src/opt/inline.rs` and `opt/inline/remap.rs`, the producer's call/shim encoding in
  `backend/` where the location argument is concerned, `examples/`, docs for this item.
- Leave alone (changes to them are under verification): `runtime/shim.js`,
  `backend/src/ptr.rs`, `backend/src/place.rs`, `backend/src/constant.rs` and
  `backend/src/constant/`, `backend/src/jir/encode.rs` except the lines your fix must touch (say
  exactly which), `jsc-link/src/lower/fuse.rs`, `lower/ssa.rs`, `lower/js/tests.rs` except for
  adding tests at the end of the file, `jsc-link/src/link.rs` and `jsc-link/src/link/`.
- The inliner has an unlanded widening patch that will be re-applied on top of main later. Keep
  your change to `opt/inline.rs` minimal and local so that re-application stays simple, and say
  in your report exactly what you changed there.
