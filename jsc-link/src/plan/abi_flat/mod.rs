//! The flat direct-call ABI: an aggregate argument passed as its fields.
//!
//! The baseline calling convention gives a Rust argument one JavaScript argument, so
//! `X64::add(&mut h1, h2)` hands the callee two objects — and the callee, which does nothing but
//! read and write `high` and `low`, has to keep them objects because the caller built them that
//! way. Nothing local can fix that: the shape of a call is a fact about *both* ends, so it is a
//! plan over the whole program rather than a pass over one function.
//!
//! [`plan_excluding`] picks the callees whose calling convention this program may change and
//! rewrites both ends of them:
//!
//! * the **callee** takes `k` scalar parameters instead of the aggregate and rebuilds the aggregate
//!   in its entry block, which the partial escape analysis then takes apart again;
//! * the **caller** passes `Field(a, i)` for each field instead of `a`;
//! * a field-only aggregate **result** returns its first field normally and its remaining fields
//!   through the same spill registers used for mutable-reference copy-back, then the caller
//!   rebuilds the semantic result for partial escape analysis to remove.
//!
//! A `&mut T` parameter is copied in the same way and copied *back* at every `Return`: the first
//! out-field rides the callee's return value when the callee returned `()`, and the rest go
//! through **spill registers** — module-scope `let $r1; let $r2; ...` bindings the link step
//! synthesizes as items of their own ([`AbiPlan::spill_items`]), that the callee writes
//! ([`OpKind::SpillWrite`]) right before it returns and the caller reads
//! ([`OpKind::SpillRead`]) right after the call. Flat aggregate results use the same protocol:
//! lane zero is the ordinary return, and lanes one onward precede copy-back fields in the spill
//! bindings.
//!
//! # What makes a callee eligible
//!
//! The rewrite changes what the item's JavaScript declaration accepts. An internal item may change
//! in place only when every reference is a direct call the plan also rewrites. An exported item or
//! one named by a function constant, static, vtable, or function-pointer coercion instead keeps its
//! canonical body and may receive one private flat clone; only proven direct callers retarget to
//! that clone. This is proved, not assumed: candidates still need a plain parameter list (no spread
//! tuple or `#[track_caller]`) and a body the lane lowered. Intrinsic fallbacks, host-marker
//! helpers, drop glue whose names are resolved by type, tracked callers, and other unmodeled
//! boundaries remain refusals. A program split into chunks refuses the plan whole: a spill register
//! is one module-scope binding and may not cross a chunk boundary.
//!
//! Which *parameters* go flat is [`crate::analysis::param_use`]'s question, closed over a fixed
//! point here: a parameter forwarded to another callee's parameter is field-only exactly when that
//! parameter is itself flat.

mod plan;
mod rewrite;
mod shapes;
mod zero_offset;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use plan::{plan_excluding, AbiDecision, AbiPlan, CrossCrateEvidence};
#[cfg(test)]
pub use rewrite::rewrite_callee;
#[allow(unused_imports)]
pub use rewrite::rewrite_calls;
#[allow(unused_imports)]
pub(crate) use shapes::sentinel_result_shape;
#[allow(unused_imports)]
pub use shapes::{
    ByteStrParam, FlatParam, FlatParamKind, FlatResult, FlatResultKind, FlatSig, LaneComponent,
    ParamAccess, ProductNode, RecursiveShape, SentinelResultShape, TaggedLane, TaggedShape,
};
