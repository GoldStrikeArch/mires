//! The JIR-R optimizer: the passes `js-opt=on` runs on the `js-lane=jir` lane.
//!
//! v1's optimizer rewrites *printed* JavaScript, which is why `js-lane=jir` and `js-opt=on` were
//! refused together through Phase 2: two rewriters may not own one item. Phase 3 lifts the refusal
//! by re-landing the rewrites as passes over JIR-R, before anything is printed. The lane's pass
//! mapping keeps v1's five [`crate::opts::OptimizerPass`] names, because the diagnostics tests name
//! them:
//!
//! * `sroa` ⇒ [`partial_escape`], the partial escape analysis. It subsumes v1's local split,
//!   copy-of-literal and literal-into-`bytes_to_int` rewrites.
//! * `ast-dce` ⇒ `ssa::build_with`'s `fold_runtime_checks` (already wired in `lower/mod.rs`) plus
//!   [`dce`] (constant folds, discriminant jump-threading, deletions) and [`range`], the
//!   bounds-check elision that rides the same gate.
//! * `future-poller` ⇒ already the lane's: `lower/js/host.rs` intercepts the async markers when the
//!   pass applies, so the monomorphic poller turns on with `js-opt=on`.
//! * `native-representations` ⇒ declined on the lane; carriers are a later phase.
//! * `link-dedup` ⇒ unchanged, and runs on the printed text of both lanes.
//!
//! `js-opt=report` runs no rewrite at all: every `Opts::applies` is false, so [`optimize`] is a
//! no-op and the sidecar reports what the analyses would have found.

pub mod candidates;
pub mod dce;
pub mod devirtualize;
pub mod host_args;
pub mod inline;
pub mod partial_escape;
mod pipeline;
pub mod projection;
pub mod prune_invoke;
pub mod range;
pub mod slice_forward;
pub mod slice_iter;
pub mod witness;

pub use pipeline::{o_pass_enabled, observed_carriers, optimize, optimize_for_probe, OptStats};
