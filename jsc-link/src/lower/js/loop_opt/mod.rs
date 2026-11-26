//! Compute-loop AST optimizations for `ReleaseSpeed`:
//!
//! 1. [`fold_guarded_f2i`]: collapses nested `if (a) { if (b) { ... } }` and replaces guarded
//!    `f2i` `i32`-clamp ternaries inside `[-2147483648, 2147483647]` with `x | 0` (`earcut`).
//! 2. [`elide_dead_fresh_slice_clones`]: deletes redundant `.slice()` clones on single-use fresh
//!    array literals (`compress256` / `Array::into_iter`).
//! 3. [`unroll_small_const_loops`]: unrolls pure small constant-trip loops (`2..=10` iters) so
//!    index arithmetic, `%`, `Math.imul`, and bounds checks constant-fold away (`sha3::permute`).
//! 4. [`eliminate_dominated_guards`]: deletes redundant dominated panic guards (`if (!H)`,
//!    `if (bi)`, `if (!(x < len))`) and constant-bounded checks (bitmask, constant-table loads).
//! 5. [`hoist_loop_invariant_props`]: LICM of invariant `.buf`/`.off`/`.len` property loads across
//!    cold diverging `core$panicking$*` calls (`fflate/dflt`, `earcut/refine`, `earcut_linked`).
//! 6. [`version_checked_loops`]: hot/cold loop-versioned BCE emitting a single exact pre-loop
//!    invariant guard branching to an unchecked fast loop with panic-identical checked fallback
//!    (`compress512`, `compress256`, `fflate/dflt`, `earcut/refine`).
//! 7. [`smi_ize_counted_loop_steps`]: narrows bounded counted loop increments from `>>> 0` to
//!    `| 0` so V8 keeps induction variables as Smis.
//! 8. [`demote_bitexact_u32_wraps`]: demotes `>>> 0` wraps to `| 0` (or bare Int32) on locals
//!    whose reads transitively feed only bit-exact 32-bit operations (`compress256`).

mod guards;
mod hoist;
mod u32_demote;
mod unroll;
mod util;

#[cfg(test)]
mod tests;

pub(super) use guards::{eliminate_dominated_guards, version_checked_loops};
pub(super) use hoist::hoist_loop_invariant_props;
pub(super) use u32_demote::demote_bitexact_u32_wraps;
pub(super) use unroll::{
    elide_dead_fresh_slice_clones, fold_guarded_f2i, smi_ize_counted_loop_steps,
    unroll_small_const_loops,
};
