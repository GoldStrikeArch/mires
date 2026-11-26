//! The lane driver: link-time lowering and whole-program optimization of JIR-S bodies.
//!
//! Every object carries JIR-S bodies lowered at link time. For each reachable item that carries a
//! decoded body and passes the lane gates, [`run`] lowers its JIR-S through JIR-R and the
//! whole-program passes, replacing the item's printed text, refs, map, `.d.ts` and zombies; any
//! offered item that fails to lower is a hard link error. Everything downstream — `link_dedup`,
//! `order`, `print_units`, chunking, the source map, the `.d.ts` — consumes the resulting
//! [`crate::item::LinkItem`] table directly.
//!
//! # Reference validation
//!
//! The driver recomputes free identifier uses from each lowered declaration. Lexically bound
//! locals, properties and labels do not create dependencies, even when another item has the
//! same name. JavaScript builtins remain free names but do not normally name linked items.
//!
//! A lowering may remove dependencies or add references to prepared items. An unprepared item
//! reference or an undeclared compiler stub is a hard error. Final emitted references determine
//! reachability after lowering, so preparation alone does not retain an unused body.
//!
//! # Gates
//!
//! An item is offered to the lowerer when all of these hold:
//!
//! * it carries a JIR-S body that [`crate::link::decode_reachable_bodies`] decoded (a synthesized
//!   item — the allocator shim, a `global_asm` block, a link-step stub — has none, and so is never
//!   offered and never counted as a fallback);
//! * every [`crate::jir::summary::ItemClass`] flag it carries is in `js-lane-classes` (default:
//!   all of them). An item with no flags at all is a subset of every set and is therefore always
//!   offered;
//! * `js-lane-only`, when given, names it.
//!
//! # The sidecar
//!
//! `<out>.lane.json` is written beside the program: one row per offered item with the lane it
//! took, any decline that stopped it, and the bytes spent on it (`bytes_min` is that item printed
//! compact with its locals shortened, so a size claim about one item does not need `js-minify` on
//! the whole build). The run also carries `passes` (how many items each pass changed), `abi` (the
//! flat direct-call plan), `repr`, and `plan_retries`.

mod driver;
mod o_group;
mod report;
#[cfg(test)]
mod tests;

pub(super) use driver::refs_of;
pub(crate) use driver::run;
pub(crate) use report::{write_sidecar, LaneOutcome, LaneRun, ReprBlock, ReprDecision};
