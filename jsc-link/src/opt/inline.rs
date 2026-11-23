//! The cost-guided inliner: the O group's body-splicing pass (phase 5, D6).
//!
//! Bottom-up over the callgraph's components (callees before callers), so a caller inlines the
//! already-optimized form of its callee. A call site is a candidate when it is a plain direct
//! call — [`Callee::Direct`], [`CallSemantic::Ordinary`], no host payload, no `#[track_caller]`
//! site or callee (a `Location` substitution is not built; refusing is the sound reading of
//! "no inline without substitution") whose callee body the lane also built from a compatible
//! object. Provider metadata is imported transactionally, and one of the briefs' three
//! criteria holds: the call sits in a loop with a virtualizable argument or result
//! ([`candidates::variant_shape`] — the `next()`-in-a-loop shape whose `Some` return the escape
//! analysis then scalarizes), the caller is the callee's only live referrer, or the callee is at
//! most [`SHARED_CALLEE_FOOTPRINT`] compact bytes, because duplicating a shared body per site
//! can grow the compressed program even when the local minified count falls.
//!
//! Refused outright: a recursive callee ([`Callgraph::recursive`], which covers the caller's own
//! component), a registry-covered [`CallSemantic::Standard`] call (its registry lowering IS its
//! best form — such a call is not `Ordinary` and never matches), a coroutine body (detected by
//! its `ResumedAfter*` asserts), and a committed-carrier boundary: a callee carrying any
//! non-baseline value, or a site whose arguments or result do (labels agree only when both
//! sides are baseline).
//!
//! Budgets (D6, enforced as `cost.budget` declines, counted in [`InlineStats`]): the caller may
//! not exceed twice its size at group entry, a site may not grow the caller past its own size
//! plus the callee's (which keeps every recorded [`Witness::Inline`] checkable), a size-criterion
//! site must not grow the measured caller/callee sum under allocated item names, and the ledger —
//! every accepted site pays at least [`MIN_SITE_CHARGE`] — may not pass the program budget (+0.5%
//! of the measured bytes, floored for small programs). The ledger is what holds the compressed-size
//! exit gates: on the idiomatic FingerprintJS port unbounded inlining shrank the minified bundle
//! while growing its brotli form past the gate, so the pass is deliberately stingy and records what
//! it declined. A declined site is reverted whole — no partial inline. Every applied site records a
//! [`Witness::Inline`] and the func is re-verified under its observed carriers; a verify or
//! witness failure reverts the site too.
//!
//! Everything is deterministic: components in the graph's order, sites processed back-to-front
//! within a caller, all maps `BTreeMap`.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::callgraph::Callgraph;
use crate::item::JsName;
use crate::jir::decline::Decline;
use crate::jir::interns::Interns;
use crate::jir::origin::{OriginId, SourceFactId};
use crate::jir::repr::{
    CellId, Func, Op, OpKind, Proj, RAssertMessage, RBlock, RBlockId, RTerm, Root, ValueId,
};
use crate::jir::semantic::{
    BinaryOp, CallSemantic, Callee, ConstTree, DropGlue, GlobalReference, HostPayload, TypeId,
};
use crate::jir::types::{NominalForm, TypeKind, TypeTable};
use crate::lower::js::host::ItemIndex;

use super::witness::{self, Witness};
use super::{candidates, observed_carriers};

mod remap;
pub(crate) use remap::Metadata;

/// The program growth ceiling, as a fraction denominator of the measured per-item byte sum.
///
/// The briefs' number is +0.5% of the *minified program*; the denominator here is the sum of
/// per-item measurements, which overestimates the bundle roughly 2x (per-item printing repeats
/// what minification shares). ponytail: 1/2000 (0.05% of the sum, ~0.1% of the bundle) is the
/// calibration that provably holds the idiomatic brotli gate's ~100-byte headroom — the phase-5
/// stop rule chooses the gate over deeper in-loop wins; W5g re-opens the trade with the Murmur
/// closure if it needs to.
const PROGRAM_BUDGET_DENOMINATOR: u64 = 2000;

/// A shared helper must fit within this compact declaration footprint before duplication.
const SHARED_CALLEE_FOOTPRINT: u64 = 96;
/// A `#[track_caller]` helper with an unused location parameter drops an argument at every site.
const TRACK_CALLER_SHARED_FOOTPRINT: u64 = 84;

/// The least program-ledger charge per accepted site bounds duplication even for local wins.
/// Compression can lose sharing when a body replaces repeated calls.
const MIN_SITE_CHARGE: i64 = 24;
const BUDGET_FLOOR: u64 = 384;

/// Speed-first (`js-profile=release-speed`) post-representation inlining thresholds tuned for V8
/// TurboFan (inline small hot helpers without creating >12 KB mega-functions that bail out of JIT).
const SPEED_SHARED_CALLEE_FOOTPRINT: u64 = 220;
const SPEED_TRACK_CALLER_SHARED_FOOTPRINT: u64 = 160;
const SPEED_LOOP_CALLEE_FOOTPRINT: u64 = 384;
const SPEED_INLINE_HINT_FOOTPRINT: u64 = 384;
const SPEED_INLINE_ALWAYS_FOOTPRINT: u64 = 640;
const SPEED_CALLER_CAP: u64 = 10_240;
const SPEED_BUDGET_FLOOR: u64 = 8_192;

/// What the pass did, for the sidecar's `inline` block and the driver's accounting.
#[derive(Clone, Debug, Default)]
pub struct InlineStats {
    /// Call sites inlined.
    pub applied: u32,
    /// Accepted sites whose provider has a different object universe.
    pub cross_object_applied: u32,
    /// Candidate sites refused by a budget: the `cost.budget` declines.
    pub declined_budget: u32,
    /// Candidate refusals, including metadata, semantic, verification, and cost boundaries.
    pub declined: BTreeMap<&'static str, u32>,
    /// The summed entry bytes of every caller that took at least one inline.
    pub bytes_before: u64,
    /// The summed final bytes of those same callers.
    pub bytes_after: u64,
    /// Witnesses recorded, and the ones whose independent check failed (each failure reverted
    /// its site).
    pub witnesses_recorded: u32,
    pub witnesses_failed: u32,
    /// The callers that changed.
    pub items: BTreeSet<JsName>,
}

impl InlineStats {
    fn decline(&mut self, reason: &'static str) {
        *self.declined.entry(reason).or_default() += 1;
    }

    pub fn declines_json(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::from("{");
        for (index, (reason, count)) in self.declined.iter().enumerate() {
            if index != 0 {
                out.push(',');
            }
            crate::lane_report::push_string(&mut out, reason);
            let _ = write!(out, ":{count}");
        }
        out.push('}');
        out
    }
}

/// One candidate call site, located before any splice moves it.
struct Site {
    block: usize,
    op: usize,
    callee: JsName,
    has_track_caller: bool,
    host_boundary: bool,
}

type Measurement = (u64, BTreeSet<JsName>);

/// Runs the inliner over every `Ok` func, in place.
///
/// `measure` prices one func as the item's minified bytes (`plan::cost::measure_item` behind a
/// lane closure); `None` skips the caller. `sole_referrer(callee, caller)` answers the
/// single-caller criterion from the live reference counts. `program_bytes` is the measured
/// program total the +0.5% budget is taken of.
#[allow(dead_code)] // Called by inline unit tests; `lane.rs` calls `run_with_mode`.
#[allow(clippy::too_many_arguments)] // One call site (the lane driver); the arguments are the
                                     // pass's oracles.
pub(crate) fn run(
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    metadata: &mut BTreeMap<JsName, Metadata<'_>>,
    graph: &Callgraph,
    measure: &dyn Fn(&JsName, &Func, &Metadata<'_>) -> Option<Measurement>,
    sole_referrer: &dyn Fn(&JsName, &JsName) -> bool,
    zombie_free: &dyn Fn(&JsName) -> bool,
    program_bytes: u64,
    sizes: &BTreeMap<JsName, u64>,
) -> InlineStats {
    run_with_profile(
        funcs,
        metadata,
        graph,
        &ItemIndex::new(),
        measure,
        sole_referrer,
        zombie_free,
        program_bytes,
        sizes,
        false,
        None,
    )
}

/// Runs the inliner, optionally admitting non-specialized `CallSemantic::Standard` calls after
/// representation specialization (`allow_std_post_repr`).
#[allow(dead_code)] // Called by inline unit tests; `lane.rs` calls `run_with_profile`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_with_mode(
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    metadata: &mut BTreeMap<JsName, Metadata<'_>>,
    graph: &Callgraph,
    items: &ItemIndex,
    measure: &dyn Fn(&JsName, &Func, &Metadata<'_>) -> Option<Measurement>,
    sole_referrer: &dyn Fn(&JsName, &JsName) -> bool,
    zombie_free: &dyn Fn(&JsName) -> bool,
    program_bytes: u64,
    sizes: &BTreeMap<JsName, u64>,
    allow_std_post_repr: bool,
) -> InlineStats {
    run_with_profile(
        funcs,
        metadata,
        graph,
        items,
        measure,
        sole_referrer,
        zombie_free,
        program_bytes,
        sizes,
        allow_std_post_repr,
        None,
    )
}

/// Runs the inliner under `profile` (`ReleaseSpeed` widens budgets and honors `#[inline]` hints).
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_with_profile(
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    metadata: &mut BTreeMap<JsName, Metadata<'_>>,
    graph: &Callgraph,
    items: &ItemIndex,
    measure: &dyn Fn(&JsName, &Func, &Metadata<'_>) -> Option<Measurement>,
    sole_referrer: &dyn Fn(&JsName, &JsName) -> bool,
    zombie_free: &dyn Fn(&JsName) -> bool,
    program_bytes: u64,
    sizes: &BTreeMap<JsName, u64>,
    allow_std_post_repr: bool,
    profile: Option<crate::opts::Profile>,
) -> InlineStats {
    let mut stats = InlineStats::default();
    let mut sizes = sizes.clone();
    // Run aggressive speed-first inlining only in the post-representation O pass so pre-repr
    // carrier analysis (`NativeVec`/`NativeMap`/`HostString`) always sees clean, un-poisoned
    // standard-library boundaries.
    let speed = allow_std_post_repr && matches!(profile, Some(crate::opts::Profile::ReleaseSpeed));
    let budget = if speed {
        (program_bytes / 4).max(SPEED_BUDGET_FLOOR)
    } else {
        (program_bytes / PROGRAM_BUDGET_DENOMINATOR).max(BUDGET_FLOOR)
    };
    // Growth spent so far, signed: a site that shrinks its caller gives headroom back.
    let mut growth: i64 = 0;
    let order: Vec<JsName> = graph.sccs.iter().flatten().cloned().collect();
    for caller in &order {
        let Some(mut current_metadata) = metadata.get(caller).cloned() else {
            continue;
        };
        let Some(Ok(func)) = funcs.get(caller) else {
            continue;
        };
        let sites = collect_sites(
            caller,
            func,
            &current_metadata,
            metadata,
            funcs,
            graph,
            items,
            sole_referrer,
            zombie_free,
            allow_std_post_repr,
            speed,
            &mut stats,
        );
        if sites.is_empty() {
            continue;
        }
        let entry_bytes = match sizes.get(caller).copied() {
            Some(bytes) => bytes,
            None => match measure(caller, func, &current_metadata) {
                Some((bytes, _)) => bytes,
                None => continue,
            },
        };
        let limit = if speed {
            entry_bytes
                .saturating_mul(2)
                .max(2_048)
                .min(entry_bytes.max(SPEED_CALLER_CAP))
        } else {
            entry_bytes.saturating_mul(2)
        };
        let mut current = funcs
            .get(caller)
            .and_then(|f| f.as_ref().ok())
            .cloned()
            .expect("checked Ok above");
        let looping = looping_blocks(&current);
        let mut bytes = entry_bytes;
        let mut applied_here = 0u32;
        let mut accepted = None;
        // Back to front, so an earlier splice never moves a later site.
        for site in sites.iter().rev() {
            let Some(criterion) = qualifies(
                caller,
                &site.callee,
                &current,
                site,
                funcs,
                &sizes,
                sole_referrer,
                &current_metadata.interns.types,
                &looping,
                speed,
            ) else {
                stats.decline("inline.cost-no-opportunity");
                continue;
            };
            let is_sole = sole_referrer(&site.callee, caller);
            if site.host_boundary
                && count_item_refs(&current, &current_metadata.interns, site.callee.as_str()) != 1
            {
                stats.decline("inline.host-boundary");
                continue;
            }
            let Some(Ok(callee_func)) = funcs.get(&site.callee) else {
                continue;
            };
            let callee_bytes = sizes.get(&site.callee).copied().unwrap_or(u64::MAX);
            let trivial_leaf = trivial_pure_leaf(callee_func);
            let iterator_step = metadata
                .get(&site.callee)
                .is_some_and(|meta| bounded_iterator_step(callee_func, &meta.interns.types));
            let Some(provider) = metadata.get(&site.callee) else {
                continue;
            };
            let origin = current.blocks[site.block].ops[site.op].origin;
            let (site_args_len, site_track_caller) =
                match &current.blocks[site.block].ops[site.op].kind {
                    OpKind::Call {
                        args, track_caller, ..
                    } => (args.len(), *track_caller),
                    _ => continue,
                };
            let (callee_func, imported, sentinel_assert_origin) = match prepare_callee_for_site(
                &current_metadata,
                provider,
                callee_func,
                origin,
                site_args_len,
                current.sig.abi.caller_location,
                site_track_caller,
                items,
            ) {
                Ok(value) => value,
                Err(reason) => {
                    stats.decline(reason);
                    continue;
                }
            };
            let table = &imported.interns.types;
            let backup = current.clone();
            splice(&mut current, site.block, site.op, &callee_func);
            if current
                .verify_carriers(table, observed_carriers(&current))
                .is_err()
            {
                current = backup;
                stats.decline("inline.splice-invalid");
                continue;
            }
            // Clean a clone so the remaining collected sites keep their block/op positions.
            // Price and retain the same verified shape, including the merges, cell unboxing, and DCE cleanup.
            let mut candidate = current.clone();
            if let Err(reason) = clean_candidate(&mut candidate, table, sentinel_assert_origin) {
                current = backup;
                stats.decline(reason);
                continue;
            }
            let Some((after, refs)) = measure(caller, &candidate, &imported) else {
                current = backup;
                stats.decline("inline.cost-unavailable");
                continue;
            };
            if site.host_boundary && refs.contains(&site.callee) {
                current = backup;
                stats.decline("inline.host-boundary");
                continue;
            }
            let delta = after as i64 - bytes as i64;
            // The sole-referrer oracle excludes public and explicitly rooted items. Credit
            // deletion only after the emitted caller also lost every reference to the callee,
            // including function-pointer constants and retained indirect uses in this caller.
            let removed = if is_sole && !refs.contains(&site.callee) {
                sizes.get(&site.callee).copied().unwrap_or_default()
            } else {
                0
            };
            let net = delta.saturating_sub(removed.min(i64::MAX as u64) as i64);
            let tiny_accessor_slack = if matches!(criterion, Criterion::SmallCallee)
                && callee_bytes <= 36
                && net > 0
                && net <= 2
            {
                net
            } else {
                0
            };
            let trivial_leaf_slack = if !site.host_boundary
                && trivial_leaf
                && matches!(criterion, Criterion::SmallCallee | Criterion::SoleCaller)
                && net > 0
                && net <= 16
            {
                net
            } else {
                0
            };
            let charge = if speed {
                net.max(0)
            } else if net <= 0 || trivial_leaf_slack > 0 {
                0
            } else if tiny_accessor_slack > 0 {
                tiny_accessor_slack
            } else {
                net.max(MIN_SITE_CHARGE)
            };
            if std::env::var("RCGJS_O_DEBUG_FUNC")
                .is_ok_and(|filter| caller.as_str().contains(&filter))
            {
                eprintln!(
                    "inline {caller:?} <- {:?}: {criterion:?}, before={bytes}, after={after}, callee={callee_bytes}, delta={delta}, removed={removed}, net={net}, limit={limit}, growth={growth}, charge={charge}, budget={budget}",
                    site.callee,
                );
            }
            // Only loop virtualization (or <=2B tiny accessors, or <=16B trivial pure leaves) may spend
            // temporary net growth under size/balanced profiles. Speed-first mode permits bounded
            // growth for every qualifying criterion.
            let growth_allowed = speed
                || matches!(criterion, Criterion::LoopVirtualizable)
                || tiny_accessor_slack > 0
                || trivial_leaf_slack > 0;
            // A small loop helper may expose enough scalar replacement to repay a modest
            // pre-cleanup growth in the later per-item pipeline. Bound both the duplicated body
            // and that temporary growth; this is the iterator-state case, not a general license
            // to spend the whole-program compression budget.
            let bounded_loop_virtualization = matches!(criterion, Criterion::LoopVirtualizable)
                && iterator_step
                && callee_bytes <= 560
                && after <= bytes.saturating_add(360);
            // Under speed-first mode, never let earlier cold callees starve tiny leaf helpers
            // called inside loops (e.g., `Word::add` inside SHA-512's 80-round compression loop).
            // Restrict to call-free leaves `<= 192` B and still charge `growth` so non-loop
            // callers (like top-level dispatchers) cannot consume freed budget and bloat.
            let callee_is_leaf = callee_func.blocks.iter().all(|b| {
                b.ops
                    .iter()
                    .all(|op| !matches!(op.kind, OpKind::Call { .. }))
                    && !matches!(b.term, RTerm::Invoke { .. })
            });
            let speed_hot_loop = speed
                && looping.contains(&(site.block as u32))
                && callee_is_leaf
                && callee_bytes <= 192;
            let size_limit = if is_sole && net <= 0 {
                limit.max(16_384)
            } else {
                limit
            };
            if after > size_limit
                || after > bytes.saturating_add(callee_bytes)
                || (!bounded_loop_virtualization
                    && !speed_hot_loop
                    && growth + charge > budget as i64)
                || (!growth_allowed && net > 0)
            {
                // `cost.budget`: the whole site is reverted, never a partial inline.
                current = backup;
                stats.declined_budget += 1;
                stats.decline("inline.cost-rejected");
                continue;
            }
            let record = Witness::Inline {
                callee_bytes: callee_bytes.min(u32::MAX as u64) as u32,
                limit: size_limit.min(u32::MAX as u64) as u32,
                bytes_before: bytes.min(u32::MAX as u64) as u32,
                bytes_after: after.min(u32::MAX as u64) as u32,
            };
            stats.witnesses_recorded += 1;
            if witness::enabled() && record.check(&candidate, table).is_err() {
                stats.witnesses_failed += 1;
                current = backup;
                stats.decline("inline.witness-invalid");
                continue;
            }
            if !bounded_loop_virtualization {
                growth += charge;
            }
            bytes = after;
            applied_here += 1;
            stats.cross_object_applied +=
                u32::from(!std::ptr::eq(current_metadata.universe, provider.universe));
            current_metadata = imported;
            accepted = Some(candidate);
        }

        // Multi-site sole-caller batching: when a callee has K > 1 sites in its sole referrer
        // and intermediate sites left `removed == 0` with `net > 0`, try splicing all remaining
        // sites to that callee as one atomic transaction.
        let mut sole_site_counts: BTreeMap<JsName, usize> = BTreeMap::new();
        for site in &sites {
            if !site.host_boundary && sole_referrer(&site.callee, caller) {
                *sole_site_counts.entry(site.callee.clone()).or_default() += 1;
            }
        }
        let batch_callees: Vec<JsName> = sole_site_counts
            .into_iter()
            .filter_map(|(callee, count)| (count > 1).then_some(callee))
            .collect();
        if !batch_callees.is_empty() {
            if let Some(clean) = accepted.as_ref() {
                current = clean.clone();
            }
            for callee in batch_callees {
                let mut throwaway_stats = InlineStats::default();
                let remaining: Vec<Site> = collect_sites(
                    caller,
                    &current,
                    &current_metadata,
                    metadata,
                    funcs,
                    graph,
                    items,
                    sole_referrer,
                    zombie_free,
                    allow_std_post_repr,
                    speed,
                    &mut throwaway_stats,
                )
                .into_iter()
                .filter(|site| site.callee == callee)
                .collect();
                if remaining.is_empty()
                    || remaining.iter().any(|site| site.host_boundary)
                    || count_item_refs(&current, &current_metadata.interns, callee.as_str())
                        != remaining.len()
                {
                    continue;
                }
                let Some(Ok(callee_func)) = funcs.get(&callee) else {
                    continue;
                };
                let Some(provider) = metadata.get(&callee) else {
                    continue;
                };
                let callee_bytes = sizes.get(&callee).copied().unwrap_or(u64::MAX);
                let batch_backup = current.clone();
                let batch_meta = current_metadata.clone();
                let batch_bytes = bytes;
                let mut sentinel_origin: Option<OriginId> = None;
                let mut batch_err: Option<&'static str> = None;

                for site in remaining.iter().rev() {
                    let origin = current.blocks[site.block].ops[site.op].origin;
                    let (site_args_len, site_track_caller) =
                        match &current.blocks[site.block].ops[site.op].kind {
                            OpKind::Call {
                                args, track_caller, ..
                            } => (args.len(), *track_caller),
                            _ => {
                                batch_err = Some("inline.splice-invalid");
                                break;
                            }
                        };
                    let (prepared, next_meta, site_sentinel) = match prepare_callee_for_site(
                        &current_metadata,
                        provider,
                        callee_func,
                        origin,
                        site_args_len,
                        current.sig.abi.caller_location,
                        site_track_caller,
                        items,
                    ) {
                        Ok(value) => value,
                        Err(reason) => {
                            batch_err = Some(reason);
                            break;
                        }
                    };
                    if sentinel_origin.is_none() {
                        sentinel_origin = site_sentinel;
                    }
                    splice(&mut current, site.block, site.op, &prepared);
                    if current
                        .verify_carriers(&next_meta.interns.types, observed_carriers(&current))
                        .is_err()
                    {
                        batch_err = Some("inline.splice-invalid");
                        break;
                    }
                    current_metadata = next_meta;
                }
                if let Some(reason) = batch_err {
                    current = batch_backup;
                    current_metadata = batch_meta;
                    bytes = batch_bytes;
                    stats.decline(reason);
                    continue;
                }

                let mut candidate = current.clone();
                let table = &current_metadata.interns.types;
                if let Err(reason) = clean_candidate(&mut candidate, table, sentinel_origin) {
                    current = batch_backup;
                    current_metadata = batch_meta;
                    bytes = batch_bytes;
                    stats.decline(reason);
                    continue;
                }
                let Some((after, refs)) = measure(caller, &candidate, &current_metadata) else {
                    current = batch_backup;
                    current_metadata = batch_meta;
                    bytes = batch_bytes;
                    stats.decline("inline.cost-unavailable");
                    continue;
                };
                let delta = after as i64 - batch_bytes as i64;
                let removed = if !refs.contains(&callee) {
                    callee_bytes
                } else {
                    0
                };
                let net = delta.saturating_sub(removed.min(i64::MAX as u64) as i64);
                let size_limit = limit.max(16_384);
                if std::env::var("RCGJS_O_DEBUG_FUNC")
                    .is_ok_and(|filter| caller.as_str().contains(&filter))
                {
                    eprintln!(
                        "inline-batch {caller:?} <- {callee:?} ({} sites): before={batch_bytes}, after={after}, callee={callee_bytes}, delta={delta}, removed={removed}, net={net}, limit={size_limit}",
                        remaining.len(),
                    );
                }
                if net > 0 || after > size_limit || after > batch_bytes.saturating_add(callee_bytes)
                {
                    current = batch_backup;
                    current_metadata = batch_meta;
                    bytes = batch_bytes;
                    stats.declined_budget += 1;
                    stats.decline("inline.cost-rejected");
                    continue;
                }
                let record = Witness::Inline {
                    callee_bytes: callee_bytes.min(u32::MAX as u64) as u32,
                    limit: size_limit.min(u32::MAX as u64) as u32,
                    bytes_before: batch_bytes.min(u32::MAX as u64) as u32,
                    bytes_after: after.min(u32::MAX as u64) as u32,
                };
                stats.witnesses_recorded += 1;
                if witness::enabled() && record.check(&candidate, table).is_err() {
                    stats.witnesses_failed += 1;
                    current = batch_backup;
                    current_metadata = batch_meta;
                    bytes = batch_bytes;
                    stats.decline("inline.witness-invalid");
                    continue;
                }
                bytes = after;
                applied_here += remaining.len() as u32;
                stats.cross_object_applied += (remaining.len() as u32)
                    * u32::from(!std::ptr::eq(batch_meta.universe, provider.universe));
                current = candidate.clone();
                accepted = Some(candidate);
            }
        }
        if let Some(current) = accepted {
            stats.applied += applied_here;
            stats.bytes_before += entry_bytes;
            stats.bytes_after += bytes;
            stats.items.insert(caller.clone());
            sizes.insert(caller.clone(), bytes);
            funcs.insert(caller.clone(), Ok(current));
            metadata.insert(caller.clone(), current_metadata);
        }
    }
    stats
}

/// Whether a callee is the narrow iterator step whose inlining exposes scalar pointer state.
///
/// This recognizes `nth`-shaped methods structurally: `&mut` to a small struct with two live
/// fields, one integer step, and a two-variant single-payload result. It deliberately excludes
/// general iterator adapters and arbitrary ADTs; those belong to the physical ABI rather than the
/// compression-budget exception above.
fn bounded_iterator_step(func: &Func, table: &TypeTable) -> bool {
    let self_param = match func.params.as_slice() {
        [self_param] | [self_param, _] => *self_param,
        _ => return false,
    };
    let self_ty = func.values[self_param.0 as usize].ty;
    let Some(TypeKind::Reference {
        pointee,
        mutable: true,
    }) = table
        .get(table.peeled(self_ty))
        .map(|descriptor| &descriptor.kind)
    else {
        return false;
    };
    let Some(TypeKind::Adt {
        form: NominalForm::Struct,
        variants,
        ..
    }) = table
        .get(table.peeled(*pointee))
        .map(|descriptor| &descriptor.kind)
    else {
        return false;
    };
    let [variant] = variants.as_slice() else {
        return false;
    };
    if variant
        .fields
        .iter()
        .filter(|field| !table.is_zst(field.ty))
        .count()
        != 2
    {
        return false;
    }
    let Some(TypeKind::Adt {
        form: NominalForm::Enum,
        variants,
        ..
    }) = table
        .get(table.peeled(func.sig.result))
        .map(|descriptor| &descriptor.kind)
    else {
        return false;
    };
    variants.len() == 2
        && variants.iter().all(|variant| {
            variant
                .fields
                .iter()
                .filter(|field| !table.is_zst(field.ty))
                .count()
                <= 1
        })
}

const SENTINEL_ASSERT_ORIGIN: OriginId = OriginId(u32::MAX - 1);

/// Overflow asserts for wrapping/checked integer ops are skipped by `lower::js::emit`, whereas
/// every other `OpKind::Assert` resolves its panic lang item from the caller's `summary.deps`.
fn is_optional_assert(kind: &OpKind) -> bool {
    matches!(
        kind,
        OpKind::Assert {
            msg: RAssertMessage::Overflow {
                op: BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Shl | BinaryOp::Shr,
                ..
            },
            ..
        }
    )
}

/// Prepares an imported callee for splicing at a call site, resolving any `#[track_caller]`
/// location substitution and tagging non-optional callee `Assert` ops with a sentinel origin so
/// any assert surviving DCE rejects inlining (because `lower::js::emit` resolves panic lang items
/// from the caller's `summary.deps` and would otherwise fall back to `js_abort`).
#[allow(clippy::too_many_arguments)]
fn prepare_callee_for_site<'a>(
    current_metadata: &Metadata<'a>,
    provider: &Metadata<'a>,
    callee_func: &Func,
    origin: OriginId,
    site_args_len: usize,
    _caller_caller_location: bool,
    site_track_caller: Option<SourceFactId>,
    items: &ItemIndex,
) -> Result<(Func, Metadata<'a>, Option<OriginId>), &'static str> {
    let (mut callee_func, imported) = current_metadata.import(provider, callee_func, origin)?;
    let mut sentinel_assert_origin = None;
    if callee_func.sig.abi.caller_location && callee_func.params.len() == site_args_len + 1 {
        for block in &mut callee_func.blocks {
            for op in &mut block.ops {
                if matches!(op.kind, OpKind::Assert { .. }) && !is_optional_assert(&op.kind) {
                    op.origin = SENTINEL_ASSERT_ORIGIN;
                    sentinel_assert_origin = Some(SENTINEL_ASSERT_ORIGIN);
                }
            }
        }
        if let Some(site_fact) = site_track_caller {
            rewrite_forwarded_track_caller(&mut callee_func, site_fact, items);
            if !unused_caller_location(&callee_func) {
                bind_caller_location_param(&mut callee_func, site_fact, &imported.interns, origin)?;
            }
        }
        strip_unused_last_param(&mut callee_func);
    }
    Ok((callee_func, imported, sentinel_assert_origin))
}

fn clean_candidate(
    candidate: &mut Func,
    table: &TypeTable,
    sentinel_assert_origin: Option<OriginId>,
) -> Result<(), &'static str> {
    collapse_forwarders(candidate);
    fuse_straight_lines(candidate);
    super::partial_escape::promote_cell_borrows(candidate, table);
    let dce_opts = crate::opt::dce::Options {
        no_constant_params: false,
        no_deletions: false,
        pure_call: None,
    };
    let mut candidate_dce = candidate.clone();
    if crate::opt::dce::run_with(&mut candidate_dce, table, &dce_opts, &mut Vec::new())
        && candidate_dce
            .verify_carriers(table, observed_carriers(&candidate_dce))
            .is_ok()
    {
        *candidate = candidate_dce;
    }
    super::partial_escape::unbox_cells(candidate, table);
    if let Some(sentinel) = sentinel_assert_origin {
        if candidate.blocks.iter().any(|block| {
            block
                .ops
                .iter()
                .any(|op| matches!(op.kind, OpKind::Assert { .. }) && op.origin == sentinel)
        }) {
            return Err("inline.track-caller-unavailable");
        }
    }
    candidate
        .verify_carriers(table, observed_carriers(candidate))
        .map_err(|_| "inline.cleanup-invalid")
}

/// Whether `func` is a trivial single-block pure scalar wrapper (`<= 3` pure ops, no calls,
/// no invokes, no aggregates, no asserts) whose inlining reduces to inline JS operators.
fn trivial_pure_leaf(func: &Func) -> bool {
    if func.blocks.len() != 1 || func.has_invoke() {
        return false;
    }
    let block = &func.blocks[0];
    if block.ops.len() > 3 || !matches!(block.term, RTerm::Return(_)) {
        return false;
    }
    block.ops.iter().all(|op| {
        matches!(
            op.kind,
            OpKind::Copy(_)
                | OpKind::Move(_)
                | OpKind::Const(ConstTree::Unit | ConstTree::Scalar { .. })
                | OpKind::Binary { .. }
                | OpKind::Unary { .. }
                | OpKind::Cast { .. }
        )
    })
}

/// Returns the number of fields if `ty` is `&Location` (`file: &str, line: u32, col: u32, ZST...`).
fn location_param_fields(table: &TypeTable, ty: TypeId) -> Option<usize> {
    let TypeKind::Reference { pointee, .. } = table.get(table.peeled(ty))?.kind else {
        return None;
    };
    let TypeKind::Adt {
        form: NominalForm::Struct,
        variants,
        ..
    } = &table.get(table.peeled(pointee))?.kind
    else {
        return None;
    };
    let [variant] = variants.as_slice() else {
        return None;
    };
    if variant.fields.len() < 3 {
        return None;
    }
    let TypeKind::Reference {
        pointee: file_pointee,
        ..
    } = table.get(table.peeled(variant.fields[0].ty))?.kind
    else {
        return None;
    };
    if !matches!(table.get(table.peeled(file_pointee))?.kind, TypeKind::Str) {
        return None;
    }
    for field in &variant.fields[1..3] {
        if table.int_info(field.ty) != Some((false, 32)) {
            return None;
        }
    }
    for field in &variant.fields[3..] {
        if !table.is_zst(field.ty) {
            return None;
        }
    }
    Some(variant.fields.len())
}

fn bind_caller_location_param(
    callee: &mut Func,
    site_fact: SourceFactId,
    interns: &Interns,
    origin: OriginId,
) -> Result<(), &'static str> {
    let Some(&last_param) = callee.params.last() else {
        return Err("inline.track-caller-unavailable");
    };
    let last_def = callee.values[last_param.0 as usize];
    let Some(field_count) = location_param_fields(&interns.types, last_def.ty) else {
        return Err("inline.track-caller-unavailable");
    };
    let Some(fact) = interns.source_fact(site_fact) else {
        return Err("inline.track-caller-unavailable");
    };
    let mut fields = Vec::with_capacity(field_count);
    fields.push(ConstTree::Str(fact.file));
    fields.push(ConstTree::Scalar {
        bits: u128::from(fact.line),
        bytes: 4,
    });
    fields.push(ConstTree::Scalar {
        bits: u128::from(fact.col + 1),
        bytes: 4,
    });
    fields.resize_with(field_count, || ConstTree::Unit);
    let tree = ConstTree::Reference(Box::new(ConstTree::Aggregate(fields)));
    let const_val = ValueId(callee.values.len() as u32);
    callee.values.push(last_def);
    let replace = |v: ValueId| if v == last_param { const_val } else { v };
    for block in &mut callee.blocks {
        for op in &mut block.ops {
            map_op(&mut op.kind, &replace, &|c| c);
        }
        block.term = map_term(&block.term, &replace, &|b| b);
    }
    if let Some(entry) = callee.blocks.first_mut() {
        entry.ops.insert(
            0,
            Op {
                result: Some(const_val),
                origin,
                kind: OpKind::Const(tree),
            },
        );
    }
    Ok(())
}

/// Which of D6's criteria admitted a site: the in-loop virtualizable shape may pay for growth
/// (the escape analysis cashes it), the size plays may not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Criterion {
    SmallCallee,
    SoleCaller,
    LoopVirtualizable,
    InlineHint,
}

/// The criterion one located site meets, if any (the checks that need the graph and interns
/// were made when the site was collected). The loop shape is checked first: it is the one that
/// may grow, so a site meeting it and a size criterion keeps the wider allowance.
#[allow(clippy::too_many_arguments)] // One call site; the arguments are the criteria's inputs.
fn qualifies(
    caller: &JsName,
    callee: &JsName,
    func: &Func,
    site: &Site,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    sizes: &BTreeMap<JsName, u64>,
    sole_referrer: &dyn Fn(&JsName, &JsName) -> bool,
    table: &crate::jir::TypeTable,
    looping: &BTreeSet<u32>,
    speed: bool,
) -> Option<Criterion> {
    if site.host_boundary {
        return sole_referrer(callee, caller).then_some(Criterion::SoleCaller);
    }
    let callee_bytes = sizes.get(callee).copied().unwrap_or(u64::MAX);
    if speed {
        if let Some(Ok(callee_func)) = funcs.get(callee) {
            if (callee_func.sig.abi.inline_always && callee_bytes <= SPEED_INLINE_ALWAYS_FOOTPRINT)
                || (callee_func.sig.abi.inline_hint && callee_bytes <= SPEED_INLINE_HINT_FOOTPRINT)
            {
                return Some(Criterion::InlineHint);
            }
        }
        if looping.contains(&(site.block as u32)) && callee_bytes <= SPEED_LOOP_CALLEE_FOOTPRINT {
            return Some(Criterion::LoopVirtualizable);
        }
    }
    let loop_shape = || -> bool {
        if !looping.contains(&(site.block as u32)) {
            return false;
        }
        let ty_of = |v: ValueId| func.values[v.0 as usize].ty;
        let Some(op) = func
            .blocks
            .get(site.block)
            .and_then(|block| block.ops.get(site.op))
        else {
            return false;
        };
        let OpKind::Call { args, .. } = &op.kind else {
            return false;
        };
        args.iter()
            .map(|arg| ty_of(*arg))
            .chain(op.result.map(ty_of))
            .any(|ty| virtualizable(table, ty))
    };
    if loop_shape() {
        return Some(Criterion::LoopVirtualizable);
    }
    if sole_referrer(callee, caller) {
        return Some(Criterion::SoleCaller);
    }
    // A small callee at a shared site duplicates its body per site. The raw bytes can still
    // shrink, but the *compressed* program grows: N identical call sites are nearly free under
    // brotli, N copies of a body are not — measured on the idiomatic FingerprintJS bundle,
    // where accepting these sites cost ~500 brotli bytes while the minified count fell. So the
    // small-callee criterion only stands where the duplication is degenerate: a body at or
    // under the call's own post-minify footprint, or a trivial 1..=3 op pure scalar wrapper.
    let footprint_limit = match (speed, site.has_track_caller) {
        (true, true) => SPEED_TRACK_CALLER_SHARED_FOOTPRINT,
        (true, false) => SPEED_SHARED_CALLEE_FOOTPRINT,
        (false, true) => TRACK_CALLER_SHARED_FOOTPRINT,
        (false, false) => SHARED_CALLEE_FOOTPRINT,
    };
    if callee_bytes <= footprint_limit
        || funcs
            .get(callee)
            .and_then(|res| res.as_ref().ok())
            .is_some_and(trivial_pure_leaf)
    {
        return Some(Criterion::SmallCallee);
    }
    None
}

/// The candidate call sites of one caller, in block-and-op order.
#[allow(clippy::too_many_arguments)] // One call site; the arguments are the refusal oracles.
fn collect_sites(
    caller: &JsName,
    func: &Func,
    caller_metadata: &Metadata<'_>,
    metadata: &BTreeMap<JsName, Metadata<'_>>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    graph: &Callgraph,
    items: &ItemIndex,
    sole_referrer: &dyn Fn(&JsName, &JsName) -> bool,
    zombie_free: &dyn Fn(&JsName) -> bool,
    allow_std_post_repr: bool,
    speed: bool,
    stats: &mut InlineStats,
) -> Vec<Site> {
    let mut sites = Vec::new();
    for (block_index, block) in func.blocks.iter().enumerate() {
        for (op_index, op) in block.ops.iter().enumerate() {
            let OpKind::Call {
                callee: Callee::Direct { link_name, .. },
                function: None,
                args,
                track_caller,
                semantic,
                host,
            } = &op.kind
            else {
                continue;
            };
            if matches!(block.term, RTerm::Invoke { .. }) && op_index + 1 == block.ops.len() {
                stats.decline("inline.exceptional-control-flow");
                continue;
            }
            let semantic_ok = match semantic {
                CallSemantic::Ordinary => true,
                // A `str` identity (`js-str=bytes` only) stays a call: `lower/js/call.rs` spells
                // it as the string operation it is, which inlining its body would take away.
                CallSemantic::Standard(std_op) if std_op.is_str() => false,
                CallSemantic::Standard(std_op) if allow_std_post_repr => {
                    !matches!(
                        std_op,
                        crate::jir::semantic::StandardOperation::FormatArgumentNewDisplay
                            | crate::jir::semantic::StandardOperation::FormatArgumentNewLowerHex
                            | crate::jir::semantic::StandardOperation::FormatArgumentNewUpperHex
                            | crate::jir::semantic::StandardOperation::FormatArgsNew
                            | crate::jir::semantic::StandardOperation::FormatArgsToString
                            | crate::jir::semantic::StandardOperation::F64FromStr
                            | crate::jir::semantic::StandardOperation::F32FromStr
                            | crate::jir::semantic::StandardOperation::SliceSort
                            | crate::jir::semantic::StandardOperation::SliceSortCmp
                    ) && !(crate::lower::bstr::enabled()
                        && matches!(std_op, crate::jir::semantic::StandardOperation::StringAsStr))
                }
                _ => false,
            };
            if !semantic_ok || host.is_some() {
                stats.decline("inline.semantic-boundary");
                continue;
            }
            let Some(name) = caller_metadata.interns.str(*link_name) else {
                continue;
            };
            let callee = JsName::new(name.to_owned());
            if &callee == caller || graph.recursive(&callee) {
                stats.decline("inline.recursive-budget");
                continue;
            }
            // An item with recorded zombies owes the link step its compile-time diagnostic
            // chain; splicing its body would drop the item and silence the errors.
            if !zombie_free(&callee) {
                stats.decline("inline.zombie");
                continue;
            }
            let Some(callee_metadata) = metadata.get(&callee) else {
                stats.decline("inline.metadata-unavailable");
                continue;
            };
            if !caller_metadata.compatible(callee_metadata) {
                stats.decline("inline.object-incompatible");
                continue;
            }
            let Some(Ok(callee_func)) = funcs.get(&callee) else {
                stats.decline("inline.body-unavailable");
                continue;
            };
            if callee_func.sig.abi.inline_never {
                stats.decline("inline.never");
                continue;
            }
            if items
                .get(&callee_func.item)
                .and_then(|entry| entry.standard)
                .is_some_and(|op| {
                    crate::lower::semantic_ops::fmt_kernel::is_format_machinery(op)
                        || crate::lower::semantic_ops::num::is_float_from_str(op)
                        || matches!(
                            op,
                            crate::jir::semantic::StandardOperation::SliceSort
                                | crate::jir::semantic::StandardOperation::SliceSortCmp
                        )
                })
            {
                stats.decline("inline.semantic-boundary");
                continue;
            }
            let is_sole_caller = sole_referrer(&callee, caller);
            let allow_host_boundary = is_sole_caller
                && count_item_refs(func, &caller_metadata.interns, callee.as_str()) == 1;
            if let Some(reason) = body_decline(
                callee_func,
                &callee_metadata.interns,
                &caller_metadata.interns,
                args.len(),
                func.sig.abi.caller_location,
                *track_caller,
                items,
                allow_host_boundary,
                speed,
            ) {
                stats.decline(reason);
                continue;
            }
            // A committed-carrier boundary: labels agree only when both sides are baseline.
            let baseline = crate::jir::repr::Carrier::Baseline;
            if callee_func.values.iter().any(|def| def.carrier != baseline) {
                stats.decline("inline.carrier-conversion-unsupported");
                continue;
            }
            let boundary = args
                .iter()
                .copied()
                .chain(op.result)
                .any(|v| func.values[v.0 as usize].carrier != baseline);
            if boundary {
                stats.decline("inline.carrier-conversion-unsupported");
                continue;
            }
            sites.push(Site {
                block: block_index,
                op: op_index,
                callee,
                has_track_caller: track_caller.is_some()
                    || (callee_func.sig.abi.caller_location && unused_caller_location(callee_func)),
                host_boundary: callee_has_host_boundary(
                    callee_func,
                    &callee_metadata.interns,
                    speed,
                ),
            });
        }
    }
    sites
}

/// Whether `callee`'s implicit trailing `Location` parameter has zero uses in its body.
fn unused_caller_location(callee: &Func) -> bool {
    let Some(&last_param) = callee.params.last() else {
        return false;
    };
    !callee.blocks.iter().any(|block| {
        block
            .ops
            .iter()
            .any(|op| op.kind.uses().contains(&last_param))
            || block.term.uses().contains(&last_param)
    })
}

/// Strips the dead trailing `Location` parameter from an imported `#[track_caller]` callee before splicing.
fn strip_unused_last_param(callee: &mut Func) {
    let Some(removed) = callee.params.pop() else {
        return;
    };
    if let Some(entry) = callee.blocks.first_mut() {
        entry.params.retain(|&p| p != removed);
    }
    callee.sig.abi.parameters = callee.sig.abi.parameters.saturating_sub(1);
    callee.sig.abi.caller_location = false;
    callee.values.remove(removed.0 as usize);
    let remap_val = |v: ValueId| {
        if v.0 > removed.0 {
            ValueId(v.0 - 1)
        } else {
            v
        }
    };
    for p in &mut callee.params {
        *p = remap_val(*p);
    }
    for block in &mut callee.blocks {
        for p in &mut block.params {
            *p = remap_val(*p);
        }
        for op in &mut block.ops {
            if let Some(res) = &mut op.result {
                *res = remap_val(*res);
            }
            map_op(&mut op.kind, &remap_val, &|c| c);
        }
        block.term = map_term(&block.term, &remap_val, &|b| b);
    }
}

/// Rewrites implicit `#[track_caller]` forwarding ops in `callee` to the call-site
/// `SourceFactId`.
fn rewrite_forwarded_track_caller(callee: &mut Func, site_fact: SourceFactId, items: &ItemIndex) {
    for block in &mut callee.blocks {
        for op in &mut block.ops {
            match &mut op.kind {
                OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    track_caller: tc @ None,
                    ..
                } => {
                    if items.get(item).is_some_and(|entry| entry.caller_location) {
                        *tc = Some(site_fact);
                    }
                }
                OpKind::Assert {
                    track_caller: tc @ None,
                    ..
                } => {
                    *tc = Some(site_fact);
                }
                _ => {}
            }
        }
    }
}

fn const_item_refs(tree: &ConstTree, interns: &Interns, target: &str) -> usize {
    match tree {
        ConstTree::Aggregate(fields)
        | ConstTree::Variant { fields, .. }
        | ConstTree::Slice(fields) => fields
            .iter()
            .map(|field| const_item_refs(field, interns, target))
            .sum(),
        ConstTree::Reference(inner) => const_item_refs(inner, interns, target),
        ConstTree::Dyn { data, vtable, .. } => {
            const_item_refs(data, interns, target)
                + usize::from(interns.str(*vtable) == Some(target))
        }
        ConstTree::Global {
            target:
                GlobalReference::Function { link_name, .. } | GlobalReference::Static { link_name, .. },
            ..
        }
        | ConstTree::StaticField {
            target: GlobalReference::Static { link_name, .. },
            ..
        }
        | ConstTree::StaticSlice {
            target: GlobalReference::Static { link_name, .. },
            ..
        } => usize::from(interns.str(*link_name) == Some(target)),
        ConstTree::Unit
        | ConstTree::Scalar { .. }
        | ConstTree::Str(_)
        | ConstTree::Address(_)
        | ConstTree::SliceAddress { .. }
        | ConstTree::Global {
            target: GlobalReference::Type(_),
            ..
        }
        | ConstTree::StaticField { .. }
        | ConstTree::StaticSlice { .. } => 0,
    }
}

fn count_item_refs(func: &Func, interns: &Interns, target: &str) -> usize {
    func.blocks
        .iter()
        .flat_map(|block| &block.ops)
        .map(|op| match &op.kind {
            OpKind::Const(tree) => const_item_refs(tree, interns, target),
            OpKind::Drop {
                glue: DropGlue::Direct { link_name, .. },
                ..
            } => usize::from(interns.str(*link_name) == Some(target)),
            OpKind::Intrinsic {
                fallback: Some(d), ..
            } => usize::from(interns.str(d.link_name) == Some(target)),
            OpKind::Call { callee, .. } => match callee {
                Callee::Direct { link_name, .. } => {
                    usize::from(interns.str(*link_name) == Some(target))
                }
                Callee::Intrinsic {
                    fallback: Some(d), ..
                } => usize::from(interns.str(d.link_name) == Some(target)),
                _ => 0,
            },
            _ => 0,
        })
        .sum()
}

fn is_host_boundary_op(kind: &OpKind, interns: &Interns, speed: bool) -> bool {
    match kind {
        OpKind::Host { payload, .. } => !independent_host(payload, interns, speed),
        OpKind::Call { host, semantic, .. } => {
            !host
                .as_ref()
                .is_some_and(|payload| independent_host(payload, interns, speed))
                && (host.is_some()
                    || matches!(
                        semantic,
                        CallSemantic::Host | CallSemantic::ViewAbi | CallSemantic::RawJavaScript
                    ))
        }
        _ => false,
    }
}

fn callee_has_host_boundary(callee: &Func, interns: &Interns, speed: bool) -> bool {
    callee
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .any(|op| is_host_boundary_op(&op.kind, interns, speed))
}

/// The semantic boundary preventing a callee body from being spliced at this site.
#[allow(clippy::too_many_arguments)]
fn body_decline(
    callee: &Func,
    interns: &Interns,
    caller_interns: &Interns,
    args: usize,
    caller_caller_location: bool,
    site_track_caller: Option<SourceFactId>,
    items: &ItemIndex,
    allow_host_boundary: bool,
    speed: bool,
) -> Option<&'static str> {
    if callee.has_invoke() {
        return Some("inline.exceptional-control-flow");
    }
    let expected_params = if callee.sig.abi.caller_location {
        let can_bind_used_loc = site_track_caller.is_some_and(|fact| {
            caller_interns.source_fact(fact).is_some()
                && callee
                    .params
                    .last()
                    .and_then(|p| {
                        location_param_fields(&interns.types, callee.values[p.0 as usize].ty)
                    })
                    .is_some()
        });
        if callee.params.len() == args + 1 && (unused_caller_location(callee) || can_bind_used_loc)
        {
            args + 1
        } else {
            return Some("inline.track-caller-unavailable");
        }
    } else if site_track_caller.is_some() {
        return Some("inline.track-caller-unavailable");
    } else {
        args
    };
    if callee.sig.abi.spread.is_some()
        || callee.params.len() != expected_params
        || callee
            .blocks
            .first()
            .is_none_or(|block| block.params != callee.params)
    {
        return Some("inline.abi-unsupported");
    }
    let forwards_caller_location = caller_caller_location && site_track_caller.is_none();
    callee
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .find_map(|op| {
            match &op.kind {
                // Resume assertions identify coroutine bodies until JIR-R carries a dedicated flag.
                OpKind::Assert {
                    msg:
                        RAssertMessage::ResumedAfterReturn(_)
                        | RAssertMessage::ResumedAfterPanic(_)
                        | RAssertMessage::ResumedAfterDrop(_),
                    ..
                } => Some("inline.coroutine-unsupported"),
                OpKind::Assert { .. }
                    if callee.sig.abi.caller_location
                        && !forwards_caller_location
                        && site_track_caller.is_none() =>
                {
                    Some("inline.track-caller-unavailable")
                }
                // The original item owes the linker its compile-time required-by diagnostic.
                OpKind::Zombie(_) => Some("inline.zombie"),
                OpKind::Intrinsic { name, .. } if interns.str(*name) == Some("caller_location") => {
                    Some("inline.track-caller-unavailable")
                }
                // Host and template lowering may own item-local bindings and observable ordering.
                _ if !allow_host_boundary && is_host_boundary_op(&op.kind, interns, speed) => {
                    Some("inline.host-boundary")
                }
                // Do not inline bodies that call the raw global allocator directly: splicing a
                // stdlib allocator call into a non-stdlib user function triggers `byte_views`'s
                // conservative `direct_alloc` fallback and forces the ES6 Proxy byte-view shim
                // across the whole program.
                OpKind::Call {
                    callee: inner_callee,
                    ..
                } if is_direct_alloc_call(inner_callee, interns) => {
                    Some("inline.semantic-boundary")
                }
                OpKind::Call {
                    callee: inner_callee,
                    args: inner_args,
                    track_caller: None,
                    ..
                } if callee.sig.abi.caller_location => match inner_callee {
                    Callee::Direct { item, .. } => {
                        let Some(entry) = items.get(item) else {
                            return Some("inline.track-caller-unavailable");
                        };
                        if !entry.abi_known
                            || entry.params != inner_args.len() + usize::from(entry.caller_location)
                            || (entry.caller_location
                                && site_track_caller.is_none()
                                && !caller_caller_location)
                        {
                            Some("inline.track-caller-unavailable")
                        } else {
                            None
                        }
                    }
                    Callee::Foreign { .. } | Callee::Runtime { .. } | Callee::Indirect { .. } => {
                        None
                    }
                    _ => Some("inline.track-caller-unavailable"),
                },
                _ => None,
            }
        })
}

/// Whether `callee` invokes the raw allocator directly (`__rust_alloc`/`__rust_realloc`,
/// `__rt.{alloc,alloc_zeroed,realloc}`, or `alloc::alloc::{alloc,alloc_zeroed,realloc,Global::*}`).
fn is_direct_alloc_call(callee: &Callee, interns: &Interns) -> bool {
    match callee {
        Callee::Runtime { member, .. } => matches!(
            interns.str(*member).unwrap_or(""),
            "alloc" | "alloc_zeroed" | "realloc"
        ),
        Callee::Foreign { symbol, .. } => {
            let sym = interns.str(*symbol).unwrap_or("");
            (sym.contains("__rust_alloc") && !sym.contains("__rust_alloc_error_handler"))
                || sym.contains("__rust_realloc")
        }
        Callee::Direct { link_name, .. } => {
            let name = interns.str(*link_name).unwrap_or("");
            (name.contains("__rust_alloc") && !name.contains("__rust_alloc_error_handler"))
                || name.contains("__rust_realloc")
                || name.starts_with("alloc$alloc$alloc$")
                || name.starts_with("alloc$alloc$alloc_zeroed$")
                || name.starts_with("alloc$alloc$realloc$")
                || (name.starts_with("alloc$alloc$")
                    && (name.contains("alloc_impl")
                        || name.contains("grow_impl")
                        || name.contains("_allocate$")
                        || name.contains("_grow$")))
        }
        _ => false,
    }
}

/// These dynamic markers have no item-owned template, helper bindings, or scheduling state.
fn independent_host(payload: &HostPayload, interns: &Interns, speed: bool) -> bool {
    match payload {
        HostPayload::Marker {
            section,
            template: None,
            helpers,
            ..
        } if helpers.is_empty() => {
            let Some(marker) = interns
                .str(*section)
                .and_then(|section| section.strip_prefix("rcgjs.host."))
            else {
                return false;
            };
            rcgjs_object::host::HostOperation::from_marker(marker).is_some()
                || (speed
                    && matches!(
                        marker,
                        "read"
                            | "iserror"
                            | "throw"
                            | "badread"
                            | "base64url"
                            | "base64urlnative"
                            | "base64urlsupported"
                    ))
        }
        HostPayload::Block(_) | HostPayload::Map { .. } if speed => true,
        HostPayload::Extern(bytes_id) if speed => interns
            .bytes(*bytes_id)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .and_then(|text| crate::descriptor::Descriptor::decode(text).ok())
            .is_some_and(|desc| desc.root() != crate::descriptor::Root::Module),
        _ => false,
    }
}

/// Whether a value of `ty` is something the escape analysis could scalarize at some variant —
/// the briefs' "virtualizable" (an `Option` return counts through its `Some`), or a reference
/// `&T`/`&mut T` to a scalar or virtualizable target.
fn virtualizable(table: &crate::jir::TypeTable, ty: crate::jir::semantic::TypeId) -> bool {
    let mut cur = table.peeled(ty);
    let mut is_ref = false;
    for _ in 0..8 {
        if let Some(TypeKind::Reference { pointee, .. }) =
            table.get(cur).map(|descriptor| &descriptor.kind)
        {
            is_ref = true;
            cur = table.peeled(*pointee);
        } else {
            break;
        }
    }
    if candidates::variant_shape(table, cur, 0).is_some()
        || candidates::variant_shape(table, cur, 1).is_some()
    {
        return true;
    }
    if is_ref {
        if let Some(descriptor) = table.get(cur) {
            return descriptor.layout.sized
                && !descriptor.layout.zst
                && matches!(
                    descriptor.kind,
                    TypeKind::Integer { .. }
                        | TypeKind::Bool
                        | TypeKind::Float { .. }
                        | TypeKind::Char
                        | TypeKind::RawPointer { .. }
                );
        }
    }
    false
}

/// The blocks that sit on a cycle of the CFG: reachable from themselves.
///
/// ponytail: O(blocks²) DFS; fine at JIR function sizes, replace with one Tarjan pass if a
/// profile ever names it.
fn looping_blocks(func: &Func) -> BTreeSet<u32> {
    let successors: Vec<Vec<u32>> = func
        .blocks
        .iter()
        .map(|block| block.term.successors().iter().map(|b| b.0).collect())
        .collect();
    let mut looping = BTreeSet::new();
    for start in 0..successors.len() as u32 {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<u32> = successors[start as usize].clone();
        while let Some(next) = stack.pop() {
            if next == start {
                looping.insert(start);
                break;
            }
            if seen.insert(next) {
                stack.extend(successors[next as usize].iter().copied());
            }
        }
    }
    looping
}

/// Splices `callee`'s body over the call at `func.blocks[block].ops[at]`.
///
/// The call's block keeps the ops before the call and jumps into the mapped callee entry with
/// the call's arguments; the ops after the call and the old terminator move to a fresh
/// continuation block, whose parameter is the call's result value; every callee `Return`
/// becomes an edge into it. Callee values, cells and blocks are appended with their ids offset;
/// spliced definitions retain the provider origins imported into the caller's metadata.
fn splice(func: &mut Func, block: usize, at: usize, callee: &Func) {
    let value_offset = func.values.len() as u32;
    let cell_offset = func.cells.len() as u32;
    let block_offset = func.blocks.len() as u32;
    let continuation = RBlockId(block_offset + callee.blocks.len() as u32);

    let call = func.blocks[block].ops[at].clone();
    let OpKind::Call { args, .. } = &call.kind else {
        unreachable!("splice is only called on a collected call site");
    };
    let call_args = args.clone();
    for def in &callee.values {
        func.values.push(*def);
    }
    for cell in &callee.cells {
        func.cells.push(*cell);
    }

    let map_value = |v: ValueId| ValueId(v.0 + value_offset);
    let map_cell = |c: CellId| CellId(c.0 + cell_offset);
    let map_block = |b: RBlockId| RBlockId(b.0 + block_offset);

    // The callee's blocks, remapped, with every `Return` turned into the continuation edge.
    let returns_value = call.result.is_some();
    for source in &callee.blocks {
        let mut ops: Vec<Op> = Vec::with_capacity(source.ops.len());
        for op in &source.ops {
            let mut kind = op.kind.clone();
            map_op(&mut kind, &map_value, &map_cell);
            ops.push(Op {
                result: op.result.map(map_value),
                origin: op.origin,
                kind,
            });
        }
        let term = match &source.term {
            RTerm::Return(v) => {
                let args = match (returns_value, v) {
                    (true, Some(v)) => vec![map_value(*v)],
                    // An unused result: the callee's value is dropped on the edge.
                    _ => Vec::new(),
                };
                RTerm::Goto(continuation, args)
            }
            other => map_term(other, &map_value, &map_block),
        };
        func.blocks.push(RBlock {
            id: map_block(source.id),
            params: source.params.iter().copied().map(map_value).collect(),
            ops,
            term,
        });
    }

    // The continuation: the rest of the split block, parameterized on the call's result.
    let rest = func.blocks[block].ops.split_off(at + 1);
    func.blocks[block].ops.pop();
    let term = std::mem::replace(
        &mut func.blocks[block].term,
        RTerm::Goto(map_block(RBlockId(0)), call_args),
    );
    func.blocks.push(RBlock {
        id: continuation,
        params: call.result.into_iter().collect(),
        ops: rest,
        term,
    });
}

/// Retargets edges around empty pass-through blocks: a block with no ops whose `Goto` passes
/// only its own parameters, each used nowhere else, adds nothing but a merge point — and the
/// merge point is exactly what stops the variant-per-path threading downstream. The callee's
/// `_0`-return merge takes this shape after a splice: both arms fed the return slot, and the
/// collapse hands each arm's value straight to the continuation, where the discriminant
/// threading can see the arm's `Aggregate`. The emptied block goes unreachable and the next dce
/// deletes it.
fn collapse_forwarders(func: &mut Func) {
    // Bounded: each round moves edges one step down an acyclic forwarder chain, and the bound
    // keeps a degenerate empty-forwarder cycle (an empty `loop`) from bouncing edges forever.
    for _ in 0..func.blocks.len() {
        let mut collapsed = false;
        for index in 1..func.blocks.len() {
            let forwarder = &func.blocks[index];
            if !forwarder.ops.is_empty() || forwarder.params.is_empty() {
                continue;
            }
            let RTerm::Goto(target, args) = &forwarder.term else {
                continue;
            };
            if target.0 as usize == index {
                continue;
            }
            // Pass-through only: every argument is one of the block's own parameters, so the
            // retargeted edges' arguments all come from the predecessor's own edge (no dominance
            // question), and those parameters are used nowhere else.
            let params: Vec<ValueId> = forwarder.params.clone();
            let positions: Option<Vec<usize>> = args
                .iter()
                .map(|arg| params.iter().position(|param| param == arg))
                .collect();
            let Some(positions) = positions else {
                continue;
            };
            let used_elsewhere = func.blocks.iter().enumerate().any(|(at, block)| {
                let in_ops = block
                    .ops
                    .iter()
                    .any(|op| op.kind.uses().iter().any(|v| params.contains(v)));
                in_ops || (at != index && block.term.uses().iter().any(|v| params.contains(v)))
            });
            if used_elsewhere {
                continue;
            }
            let (target, arity) = (*target, params.len());
            for pred in 0..func.blocks.len() {
                if pred == index {
                    continue;
                }
                let term = &mut func.blocks[pred].term;
                let retarget = |t: &mut RBlockId, edge_args: &mut Vec<ValueId>| {
                    if t.0 as usize != index || edge_args.len() != arity {
                        return false;
                    }
                    let new_args: Vec<ValueId> =
                        positions.iter().map(|at| edge_args[*at]).collect();
                    *t = target;
                    *edge_args = new_args;
                    true
                };
                match term {
                    RTerm::Goto(t, a) => collapsed |= retarget(t, a),
                    RTerm::Switch { cases, default, .. } => {
                        for (_, t, a) in cases.iter_mut() {
                            collapsed |= retarget(t, a);
                        }
                        collapsed |= retarget(&mut default.0, &mut default.1);
                    }
                    // Keep the private normal entry where the call result is defined.
                    RTerm::Invoke { .. } => {}
                    RTerm::Return(_) | RTerm::Unreachable | RTerm::Throw(_) => {}
                }
            }
        }
        if !collapsed {
            break;
        }
    }
    fuse_straight_lines(func);
}

/// Fuses a block into its parameterless `Goto` successor when that successor has no other
/// predecessor: the pair is one straight line the split only exists for. The continuation of a
/// splice takes this shape when the caller kept its discriminant switch in a successor block —
/// fusing gives the switch the continuation's parameter as its own, which is the exact shape
/// the variant-per-path threading recognizes.
fn fuse_straight_lines(func: &mut Func) {
    for _ in 0..func.blocks.len() {
        let mut preds: BTreeMap<u32, u32> = BTreeMap::new();
        for block in &func.blocks {
            for target in block.term.successors() {
                *preds.entry(target.0).or_default() += 1;
            }
        }
        let mut fused = false;
        for index in 0..func.blocks.len() {
            let RTerm::Goto(target, args) = &func.blocks[index].term else {
                continue;
            };
            let target = target.0 as usize;
            if target == index || target == 0 || !args.is_empty() {
                continue;
            }
            if !func.blocks[target].params.is_empty() || preds.get(&(target as u32)) != Some(&1) {
                continue;
            }
            let ops = std::mem::take(&mut func.blocks[target].ops);
            let term = std::mem::replace(&mut func.blocks[target].term, RTerm::Unreachable);
            func.blocks[index].ops.extend(ops);
            func.blocks[index].term = term;
            fused = true;
            break; // The pred counts are stale; recount before the next fusion.
        }
        if !fused {
            break;
        }
    }
}

/// Remaps a terminator's values and successors. `Return` is the caller's own and never remapped
/// here — the callee's returns were rewritten before this runs.
fn map_term(
    term: &RTerm,
    value: &dyn Fn(ValueId) -> ValueId,
    block: &dyn Fn(RBlockId) -> RBlockId,
) -> RTerm {
    let map_args = |args: &[ValueId]| args.iter().copied().map(value).collect::<Vec<_>>();
    match term {
        RTerm::Goto(target, args) => RTerm::Goto(block(*target), map_args(args)),
        RTerm::Switch {
            scrut,
            scrut_ty,
            cases,
            default,
        } => RTerm::Switch {
            scrut: value(*scrut),
            scrut_ty: *scrut_ty,
            cases: cases
                .iter()
                .map(|(bits, target, args)| (*bits, block(*target), map_args(args)))
                .collect(),
            default: (block(default.0), map_args(&default.1)),
        },
        RTerm::Return(v) => RTerm::Return(v.map(value)),
        RTerm::Unreachable => RTerm::Unreachable,
        RTerm::Throw(v) => RTerm::Throw(v.map(value)),
        RTerm::Invoke { normal, unwind } => RTerm::Invoke {
            normal: (block(normal.0), map_args(&normal.1)),
            unwind: (block(unwind.0), map_args(&unwind.1)),
        },
    }
}

/// Remaps every value and cell an op names. Mirrors [`OpKind::for_each_use`], plus the cell
/// operands and the assert message.
fn map_op(kind: &mut OpKind, value: &dyn Fn(ValueId) -> ValueId, cell: &dyn Fn(CellId) -> CellId) {
    let mut map = |v: &mut ValueId| *v = value(*v);
    let path = |path: &mut Vec<Proj>, f: &mut dyn FnMut(&mut ValueId)| {
        for proj in path {
            if let Proj::Index(v) = proj {
                f(v);
            }
        }
    };
    let root = |root: &mut Root, f: &mut dyn FnMut(&mut ValueId)| match root {
        Root::Value(v) => f(v),
        Root::Cell(c) => *c = cell(*c),
    };
    match kind {
        OpKind::Const(_) | OpKind::Zombie(_) | OpKind::SpillRead(_) => {}
        OpKind::CellAlloc(c) | OpKind::CellLoad(c) => *c = cell(*c),
        OpKind::CellStore { cell: c, v } => {
            *c = cell(*c);
            map(v);
        }
        OpKind::Copy(v)
        | OpKind::Move(v)
        | OpKind::Repeat { v, .. }
        | OpKind::Field { base: v, .. }
        | OpKind::Discriminant(v)
        | OpKind::SetDiscriminant { base: v, .. }
        | OpKind::SpillWrite { v, .. }
        | OpKind::Deref(v)
        | OpKind::Unary { v, .. }
        | OpKind::Cast { v, .. }
        | OpKind::Materialize(v)
        | OpKind::Convert { v, .. } => map(v),
        OpKind::Aggregate { fields, .. } => fields.iter_mut().for_each(&mut map),
        OpKind::SetField { base, v, .. } => {
            map(base);
            map(v);
        }
        OpKind::BorrowView {
            root: r, path: p, ..
        }
        | OpKind::RawView {
            root: r, path: p, ..
        }
        | OpKind::Drop {
            root: r, path: p, ..
        } => {
            root(r, &mut map);
            path(p, &mut map);
        }
        OpKind::SliceView { ptr, len } => {
            map(ptr);
            map(len);
        }
        OpKind::StoreThrough { ptr, v } => {
            map(ptr);
            map(v);
        }
        OpKind::Binary { l, r, .. } => {
            map(l);
            map(r);
        }
        OpKind::Call { function, args, .. } => {
            if let Some(function) = function {
                map(function);
            }
            args.iter_mut().for_each(&mut map);
        }
        OpKind::Intrinsic { args, .. } | OpKind::Host { args, .. } => {
            args.iter_mut().for_each(&mut map)
        }
        OpKind::Assert { cond, msg, .. } => {
            map(cond);
            map_message(msg, &mut map);
        }
    }
}

fn map_message(msg: &mut RAssertMessage, f: &mut dyn FnMut(&mut ValueId)) {
    match msg {
        RAssertMessage::BoundsCheck {
            length: a,
            index: b,
        }
        | RAssertMessage::Overflow {
            left: a, right: b, ..
        }
        | RAssertMessage::MisalignedPointer {
            required: a,
            found: b,
        } => {
            f(a);
            f(b);
        }
        RAssertMessage::OverflowNeg(v)
        | RAssertMessage::DivisionByZero(v)
        | RAssertMessage::RemainderByZero(v)
        | RAssertMessage::InvalidEnum(v) => f(v),
        RAssertMessage::ResumedAfterReturn(_)
        | RAssertMessage::ResumedAfterPanic(_)
        | RAssertMessage::ResumedAfterDrop(_)
        | RAssertMessage::NullPointer
        | RAssertMessage::NullReference => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::callgraph;
    use crate::jir::interns::StrId;
    use crate::jir::repr::{Carrier, FuncBuilder};
    use crate::jir::semantic::{
        AbiSignature, BinaryOp, ConstTree, DefinitionId, ItemId, Signature, TypeId,
    };
    use crate::jir::types::{
        IntFacts, IntRepr, JsFacts, LayoutFacts, TypeDescriptor, TypeKind, TypeMeta,
    };
    use crate::jir::OriginId;

    const O: OriginId = OriginId(0);
    const I32: TypeId = TypeId(1);

    fn item(n: u64) -> ItemId {
        ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash: n,
            },
            instantiation: 0,
        }
    }

    fn sig(args: usize) -> Signature {
        Signature {
            arguments: vec![I32; args],
            result: I32,
            abi: AbiSignature {
                parameters: args as u16,
                ..AbiSignature::default()
            },
        }
    }

    /// A one-object program: one interns table, an `i32` descriptor, and the funcs.
    struct Prog {
        interns: Interns,
        funcs: BTreeMap<JsName, Result<Func, Decline>>,
    }

    impl Prog {
        fn new() -> Prog {
            let mut interns = Interns::new();
            interns.intern_type(TypeDescriptor {
                id: I32,
                name: None,
                kind: TypeKind::Integer {
                    signed: true,
                    bits: 32,
                },
                layout: LayoutFacts {
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: Some(IntFacts {
                        signed: true,
                        bits: 32,
                        repr: IntRepr::Number,
                    }),
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: I32,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: Vec::new(),
                },
            });
            Prog {
                interns,
                funcs: BTreeMap::new(),
            }
        }

        fn strid(&mut self, name: &str) -> StrId {
            self.interns.intern_str(name)
        }

        fn add(&mut self, name: &str, func: Func) {
            self.funcs.insert(JsName::new(name.to_owned()), Ok(func));
        }

        /// Runs the inliner with an op-count size measure and the given budget/referrer knobs.
        fn run(&mut self, program_bytes: u64, sole: bool) -> InlineStats {
            let interns = &self.interns;
            let funcs = &mut self.funcs;
            let interns_map: BTreeMap<JsName, &Interns> =
                funcs.keys().map(|name| (name.clone(), interns)).collect();
            let graph = callgraph::build(funcs, &interns_map);
            let measure = |_: &JsName, func: &Func, metadata: &Metadata<'_>| {
                Some(measurement(func, metadata))
            };
            let mut metadata = funcs
                .keys()
                .map(|name| {
                    (
                        name.clone(),
                        Metadata {
                            universe: interns,
                            interns: std::borrow::Cow::Borrowed(interns),
                            origins: None,
                            header: None,
                        },
                    )
                })
                .collect();
            let sole_referrer = move |_: &JsName, _: &JsName| sole;
            let mut sizes = BTreeMap::new();
            let mut items = ItemIndex::new();
            for (name, func) in funcs.iter() {
                if let Ok(func) = func {
                    sizes.insert(name.clone(), measured(func));
                    items.insert(
                        func.item,
                        crate::lower::js::host::ItemEntry {
                            name: name.as_str().to_owned(),
                            params: func.sig.abi.parameters as usize,
                            abi_known: true,
                            result: Some(func.sig.result),
                            arguments: func.sig.arguments.clone(),
                            caller_location: func.sig.abi.caller_location,
                            caller_location_def: None,
                            is_core: false,
                            used_params: BTreeSet::new(),
                            standard: None,
                            keyed_purity: None,
                            keyed_read_params: None,
                            keyed_may_unwind: None,
                            keyed_row_capture: None,
                            keyed_proxy_safe_params: None,
                        },
                    );
                }
            }
            run_with_mode(
                funcs,
                &mut metadata,
                &graph,
                &items,
                &measure,
                &sole_referrer,
                &|_: &JsName| true,
                program_bytes,
                &sizes,
                false,
            )
        }

        fn func(&self, name: &str) -> &Func {
            self.funcs
                .get(&JsName::new(name.to_owned()))
                .and_then(|f| f.as_ref().ok())
                .expect("func")
        }

        fn calls_in(&self, name: &str) -> usize {
            self.func(name)
                .blocks
                .iter()
                .flat_map(|block| &block.ops)
                .filter(|op| matches!(op.kind, OpKind::Call { .. }))
                .count()
        }
    }

    /// A toy size: ops cost one, calls cost fifty more, plus a constant.
    /// Inlining a small callee therefore shrinks its
    /// caller, the shape the pass pays for, while the budget tests can still starve it.
    fn measured(func: &Func) -> u64 {
        let ops: u64 = func.blocks.iter().map(|b| b.ops.len() as u64).sum();
        let calls: u64 = func
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter(|op| matches!(op.kind, OpKind::Call { .. }))
            .count() as u64;
        ops + 50 * calls + 10
    }

    fn measurement(func: &Func, metadata: &Metadata<'_>) -> Measurement {
        let refs = func
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .filter_map(|op| {
                let OpKind::Call {
                    callee: Callee::Direct { link_name, .. },
                    ..
                } = op.kind
                else {
                    return None;
                };
                metadata
                    .interns
                    .str(link_name)
                    .map(|name| JsName::new(name.to_owned()))
            })
            .collect();
        (measured(func), refs)
    }

    /// `fn g(x) { return x + 1; }`
    fn add_one(prog: &mut Prog) -> Func {
        let _ = prog;
        let mut b = FuncBuilder::new(item(2), sig(1));
        let x = b.param(I32, O);
        let one = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            I32,
            O,
        );
        let sum = b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: x,
                r: one,
                ty: I32,
            },
            I32,
            O,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(sum)));
        b.finish()
    }

    /// `fn f(p) { return g(p); }`
    fn calls_g(prog: &mut Prog) -> Func {
        let g = prog.strid("g");
        let mut b = FuncBuilder::new(item(1), sig(1));
        let p = b.param(I32, O);
        let r = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: g,
                },
                function: None,
                args: vec![p],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(r)));
        b.finish()
    }

    #[test]
    fn a_small_callee_is_spliced_and_the_call_disappears() {
        let mut prog = Prog::new();
        let g = add_one(&mut prog);
        let f = calls_g(&mut prog);
        prog.add("g", g);
        prog.add("f", f);
        let stats = prog.run(100_000, false);
        assert_eq!(stats.applied, 1);
        assert_eq!(stats.declined_budget, 0);
        assert_eq!(prog.calls_in("f"), 0, "the call was replaced by the body");
        assert!(stats.items.contains(&JsName::new("f".to_owned())));
        // The spliced body carries the callee's add.
        let has_add = prog.func("f").blocks.iter().any(|block| {
            block.ops.iter().any(|op| {
                matches!(
                    op.kind,
                    OpKind::Binary {
                        op: BinaryOp::Add,
                        ..
                    }
                )
            })
        });
        assert!(has_add);
        // The witness held and the byte accounting is the measure's: the splice deleted the
        // call, so the caller shrank.
        assert_eq!(stats.witnesses_recorded, 1);
        assert_eq!(stats.witnesses_failed, 0);
        assert!(stats.bytes_after <= stats.bytes_before);
    }

    #[test]
    fn recursion_is_refused() {
        let mut prog = Prog::new();
        // `g` calls itself; `f` calls `g`: neither site is inlined.
        let g_name = prog.strid("g");
        let mut b = FuncBuilder::new(item(2), sig(1));
        let x = b.param(I32, O);
        let r = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: g_name,
                },
                function: None,
                args: vec![x],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(r)));
        prog.add("g", b.finish());
        let f = calls_g(&mut prog);
        prog.add("f", f);
        let stats = prog.run(100_000, true);
        assert_eq!(stats.applied, 0);
        assert_eq!(prog.calls_in("f"), 1);
        assert_eq!(prog.calls_in("g"), 1);
    }

    #[test]
    fn a_budget_overrun_declines_the_whole_site() {
        let mut prog = Prog::new();
        // Removing a sole callee still cannot spend more than the caller's lifted 16 KiB growth cap.
        let ext = prog.strid("ext_step");
        let mut b = FuncBuilder::new(item(2), sig(1));
        let x = b.param(I32, O);
        for _ in 0..400 {
            let _ = b.op(
                FuncBuilder::ENTRY,
                OpKind::Call {
                    callee: Callee::Foreign {
                        definition: DefinitionId {
                            crate_id: 0,
                            local_hash: 99,
                        },
                        symbol: ext,
                    },
                    function: None,
                    args: vec![x],
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                I32,
                O,
            );
        }
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(x)));
        let f = calls_g(&mut prog);
        prog.add("g", b.finish());
        prog.add("f", f);
        let before = prog.func("f").clone();
        let stats = prog.run(100_000, true);
        assert_eq!(stats.applied, 0);
        assert_eq!(stats.declined_budget, 1, "the decline is `cost.budget`");
        assert_eq!(prog.func("f"), &before, "no partial inline");
    }

    #[test]
    fn a_committed_carrier_boundary_is_refused() {
        let mut prog = Prog::new();
        let mut g = add_one(&mut prog);
        g.values[0].carrier = Carrier::NativeVec;
        let f = calls_g(&mut prog);
        prog.add("g", g);
        prog.add("f", f);
        let stats = prog.run(100_000, true);
        assert_eq!(stats.applied, 0);
        assert_eq!(prog.calls_in("f"), 1);
    }

    #[test]
    fn a_caller_location_callee_is_refused() {
        let mut prog = Prog::new();
        let mut g = add_one(&mut prog);
        g.sig.abi.caller_location = true;
        let f = calls_g(&mut prog);
        prog.add("g", g);
        prog.add("f", f);
        let stats = prog.run(100_000, true);
        assert_eq!(stats.applied, 0);
        assert_eq!(prog.calls_in("f"), 1);
    }

    #[test]
    fn a_coroutine_shaped_callee_is_refused() {
        let mut prog = Prog::new();
        let mut b = FuncBuilder::new(item(2), sig(1));
        let x = b.param(I32, O);
        let t = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 1 }),
            I32,
            O,
        );
        b.effect(
            FuncBuilder::ENTRY,
            OpKind::Assert {
                cond: t,
                expected: true,
                msg: RAssertMessage::ResumedAfterReturn(
                    crate::jir::semantic::CoroutineKind::Coroutine { movable: false },
                ),
                track_caller: None,
            },
            O,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(x)));
        let f = calls_g(&mut prog);
        prog.add("g", b.finish());
        prog.add("f", f);
        let stats = prog.run(100_000, true);
        assert_eq!(stats.applied, 0);
        assert_eq!(prog.calls_in("f"), 1);
    }

    /// Template lowering owns item-local bindings, so its caller cannot be spliced.
    #[test]
    fn a_host_calling_callee_is_refused() {
        let mut prog = Prog::new();
        let host = prog.strid("h");
        let mut b = FuncBuilder::new(item(2), sig(1));
        let x = b.param(I32, O);
        let r = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(3),
                    link_name: host,
                },
                function: None,
                args: vec![x],
                track_caller: None,
                semantic: CallSemantic::ViewAbi,
                host: None,
            },
            I32,
            O,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(r)));
        let f = calls_g(&mut prog);
        prog.add("g", b.finish());
        prog.add("f", f);
        let stats = prog.run(100_000, false);
        assert_eq!(stats.applied, 0);
        assert_eq!(prog.calls_in("f"), 1);
    }

    #[test]
    fn dynamic_host_import_refuses_item_owned_state() {
        let mut interns = Interns::new();
        for (marker, supported) in [
            ("rcgjs.host.isnull", true),
            ("rcgjs.host.invoke", true),
            ("rcgjs.host.futurepromise", false),
            ("rcgjs.tc.text", false),
        ] {
            let section = interns.intern_str(marker);
            let mut payload = HostPayload::Marker {
                section,
                def: item(1).definition,
                template: None,
                helpers: Vec::new(),
            };
            assert_eq!(independent_host(&payload, &interns, false), supported);
            let HostPayload::Marker { template, .. } = &mut payload else {
                unreachable!()
            };
            *template = Some(ConstTree::Unit);
            assert!(!independent_host(&payload, &interns, false));
        }
    }

    #[test]
    fn cross_object_import_preserves_metadata_and_rolls_back_as_one_transaction() {
        use crate::jir::origin::{Origin, OriginKind, OriginTable, SourceFact};
        use rcgjs_object::object::Header;
        use std::borrow::Cow;
        fn origins(interns: &mut Interns, file: &str) -> OriginTable {
            let file = interns.intern_str(file);
            let source = interns.intern_source_fact(SourceFact {
                file,
                line: 17,
                col: 4,
            });
            OriginTable {
                origins: vec![Origin {
                    kind: OriginKind::Statement,
                    source: Some(source),
                    local_name: None,
                    inline_stack: Vec::new(),
                }],
                statements: Vec::new(),
                terminators: Vec::new(),
                locals: Vec::new(),
                item_path: file,
                root: Some(source),
            }
        }
        for failure in [None, Some("type"), Some("object"), Some("cost")] {
            let mut caller = Prog::new();
            caller.strid("caller-only");
            let f = calls_g(&mut caller);
            let caller_origins = origins(&mut caller.interns, "caller.rs");
            let mut provider = Prog::new();
            let intrinsic = provider.strid("black_box");
            assert_eq!(intrinsic, StrId(0));
            let provider_origins = origins(&mut provider.interns, "provider.rs");
            let array = TypeId(2);
            let mut array_type = provider.interns.ty(I32).unwrap().clone();
            array_type.id = array;
            array_type.kind = TypeKind::Array {
                element: I32,
                length: 2,
            };
            array_type.js.peeled = array;
            array_type.js.int = None;
            array_type.js.is_indirect = true;
            array_type.js.needs_clone = true;
            provider.interns.intern_type(array_type);
            let mut b = FuncBuilder::new(item(2), sig(1));
            let x = b.param(I32, O);
            let pack = b.op(
                FuncBuilder::ENTRY,
                OpKind::Aggregate {
                    ty: array,
                    variant: 0,
                    active_field: None,
                    fields: vec![x, x],
                },
                array,
                O,
            );
            let field = b.op(
                FuncBuilder::ENTRY,
                OpKind::Field {
                    base: pack,
                    variant: None,
                    field: 1,
                },
                I32,
                O,
            );
            let result = b.op(
                FuncBuilder::ENTRY,
                OpKind::Intrinsic {
                    name: intrinsic,
                    types: vec![I32],
                    fallback: None,
                    args: vec![field],
                    track_caller: None,
                },
                I32,
                O,
            );
            b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(result)));
            let g = b.finish();
            g.verify(&provider.interns.types).unwrap();
            let header = Header {
                jir_schema: crate::jir::JIR_SCHEMA,
                rustc_version_hash: 7,
                data_layout: "wasm32".to_owned(),
                emit_args: "faithful".to_owned(),
                crate_name: "caller".to_owned(),
                codec: crate::jir::codec::Codec::Deflate,
                hash: crate::jir::hash::HashAlgorithm::SipHash13x128,
            };
            let mut provider_header = header.clone();
            provider_header.crate_name = "provider".to_owned();
            if failure == Some("object") {
                provider_header.rustc_version_hash += 1;
            }
            if failure == Some("type") {
                let mut ty = provider.interns.ty(I32).unwrap().clone();
                ty.layout.needs_drop = true;
                provider.interns.types.replace(ty);
            }
            let f_name = JsName::new("f".to_owned());
            let g_name = JsName::new("g".to_owned());
            let mut funcs = BTreeMap::from([
                (f_name.clone(), Ok(f.clone())),
                (g_name.clone(), Ok(g.clone())),
            ]);
            let interns_map = BTreeMap::from([
                (f_name.clone(), &caller.interns),
                (g_name.clone(), &provider.interns),
            ]);
            let graph = callgraph::build(&funcs, &interns_map);
            let mut metadata = BTreeMap::from([
                (
                    f_name.clone(),
                    Metadata {
                        universe: &caller.interns,
                        interns: Cow::Borrowed(&caller.interns),
                        origins: Some(Cow::Borrowed(&caller_origins)),
                        header: Some(&header),
                    },
                ),
                (
                    g_name.clone(),
                    Metadata {
                        universe: &provider.interns,
                        interns: Cow::Borrowed(&provider.interns),
                        origins: Some(Cow::Borrowed(&provider_origins)),
                        header: Some(&provider_header),
                    },
                ),
            ]);
            let sizes = BTreeMap::from([
                (f_name.clone(), measured(&f)),
                (g_name.clone(), measured(&g)),
            ]);
            let measure = |_: &JsName, func: &Func, metadata: &Metadata<'_>| {
                let (bytes, refs) = measurement(func, metadata);
                Some((
                    if failure == Some("cost") {
                        10000
                    } else {
                        bytes
                    },
                    refs,
                ))
            };
            let stats = run(
                &mut funcs,
                &mut metadata,
                &graph,
                &measure,
                &|_, _| true,
                &|_| true,
                100000,
                &sizes,
            );
            assert_eq!(funcs[&g_name].as_ref().unwrap(), &g);
            assert!(matches!(metadata[&g_name].interns, Cow::Borrowed(_)));
            let updated = &metadata[&f_name];
            if let Some(reason) = failure {
                assert_eq!(stats.applied, 0);
                assert_eq!(funcs[&f_name].as_ref().unwrap(), &f);
                assert!(matches!(updated.interns, Cow::Borrowed(_)));
                assert_eq!(updated.origins.as_deref(), Some(&caller_origins));
                match reason {
                    "type" => assert_eq!(stats.declined["inline.type-conflict"], 1),
                    "object" => assert_eq!(stats.declined["inline.object-incompatible"], 1),
                    "cost" => assert_eq!(stats.declined_budget, 1),
                    _ => unreachable!(),
                }
            } else {
                assert_eq!(stats.applied, 1);
                assert_eq!(stats.cross_object_applied, 1);
                let f = funcs[&f_name].as_ref().unwrap();
                f.verify(&updated.interns.types).unwrap();
                assert!(updated.interns.ty(array).is_some());
                let op = f
                    .blocks
                    .iter()
                    .flat_map(|b| &b.ops)
                    .find(|op| matches!(op.kind, OpKind::Intrinsic { .. }))
                    .unwrap();
                let OpKind::Intrinsic { name, .. } = op.kind else {
                    unreachable!()
                };
                assert_eq!(updated.interns.str(name), Some("black_box"));
                assert_ne!(name, intrinsic);
                let origin = updated.origins.as_ref().unwrap().origin(op.origin).unwrap();
                let fact = updated.interns.source_fact(origin.source.unwrap()).unwrap();
                assert_eq!(updated.interns.str(fact.file), Some("provider.rs"));
                assert_eq!(fact.line, 17);
                assert_eq!(origin.inline_stack, vec![O]);
            }
        }
    }

    #[test]
    fn net_nonpositive_inlines_charge_zero_budget() {
        // Two callers share a small callee soCriterion::SmallCallee applies (not SoleCaller).
        // Previously each site paid at least MIN_SITE_CHARGE (24), exhausting BUDGET_FLOOR (384)
        // after 16 calls even when each inline had net <= 0.
        let mut prog = Prog::new();
        let g = add_one(&mut prog);
        prog.add("g", g);
        for idx in 0..20 {
            let caller = calls_g(&mut prog);
            prog.add(&format!("f_{idx}"), caller);
        }
        let stats = prog.run(1, false);
        assert_eq!(
            stats.applied, 20,
            "all 20 zero-net calls must fit in BUDGET_FLOOR"
        );
        assert_eq!(stats.declined_budget, 0);
    }

    #[test]
    fn origin_invalid_uses_fallback_origin_on_import() {
        use crate::jir::origin::{Origin, OriginKind, OriginTable};
        use rcgjs_object::object::Header;
        use std::borrow::Cow;

        let mut provider = Interns::new();
        provider.intern_type(TypeDescriptor {
            id: I32,
            name: None,
            kind: TypeKind::Integer {
                signed: true,
                bits: 32,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: Some(IntFacts {
                    signed: true,
                    bits: 32,
                    repr: IntRepr::Number,
                }),
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: I32,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });
        let provider_origins = OriginTable {
            origins: Vec::new(),
            statements: Vec::new(),
            terminators: Vec::new(),
            locals: Vec::new(),
            item_path: StrId(0),
            root: None,
        };
        let provider_header = Header {
            jir_schema: crate::jir::JIR_SCHEMA,
            rustc_version_hash: 7,
            data_layout: "wasm32".to_owned(),
            emit_args: "faithful".to_owned(),
            crate_name: "provider".to_owned(),
            codec: crate::jir::codec::Codec::Deflate,
            hash: crate::jir::hash::HashAlgorithm::SipHash13x128,
        };

        let mut caller_interns = provider.clone();
        let g_str = caller_interns.intern_str("g");
        let caller_origins = OriginTable {
            origins: vec![Origin {
                kind: OriginKind::Statement,
                source: None,
                local_name: None,
                inline_stack: Vec::new(),
            }],
            statements: Vec::new(),
            terminators: Vec::new(),
            locals: vec![O],
            item_path: g_str,
            root: None,
        };

        let invalid_origin = OriginId(u32::MAX);
        // Callee uses OriginId(u32::MAX) while provider has empty origin table.
        let mut g_b = FuncBuilder::new(item(2), sig(1));
        let x = g_b.param(I32, invalid_origin);
        let one = g_b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            I32,
            invalid_origin,
        );
        let sum = g_b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: x,
                r: one,
                ty: I32,
            },
            I32,
            invalid_origin,
        );
        g_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(sum)));
        let g = g_b.finish();

        let mut f_b = FuncBuilder::new(item(1), sig(1));
        let p = f_b.param(I32, O);
        let call = f_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: g_str,
                },
                function: None,
                args: vec![p],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(call)));
        let f = f_b.finish();

        let f_name = JsName::new("f".to_owned());
        let g_name = JsName::new("g".to_owned());
        let mut funcs = BTreeMap::from([(f_name.clone(), Ok(f)), (g_name.clone(), Ok(g.clone()))]);
        let mut metadata = BTreeMap::from([
            (
                f_name.clone(),
                Metadata {
                    universe: &caller_interns,
                    interns: Cow::Borrowed(&caller_interns),
                    origins: Some(Cow::Borrowed(&caller_origins)),
                    header: Some(&provider_header),
                },
            ),
            (
                g_name.clone(),
                Metadata {
                    universe: &provider,
                    interns: Cow::Borrowed(&provider),
                    origins: Some(Cow::Borrowed(&provider_origins)),
                    header: Some(&provider_header),
                },
            ),
        ]);
        let interns_map = BTreeMap::from([
            (f_name.clone(), &caller_interns),
            (g_name.clone(), &provider),
        ]);
        let graph = crate::analysis::callgraph::build(&funcs, &interns_map);
        let sizes = BTreeMap::from([
            (f_name.clone(), measured(funcs[&f_name].as_ref().unwrap())),
            (g_name.clone(), measured(&g)),
        ]);
        let measure = |_: &JsName, func: &Func, meta: &Metadata<'_>| Some(measurement(func, meta));
        let stats = run(
            &mut funcs,
            &mut metadata,
            &graph,
            &measure,
            &|_, _| true,
            &|_| true,
            100_000,
            &sizes,
        );
        assert_eq!(
            stats.applied, 1,
            "OriginId::INVALID should use fallback origin"
        );
        assert!(!stats.declined.contains_key("inline.origin-invalid"));
    }

    #[test]
    fn unused_track_caller_param_is_stripped_and_inlined() {
        let mut prog = Prog::new();
        let file = prog.strid("caller.rs");
        let fact = prog
            .interns
            .intern_source_fact(crate::jir::origin::SourceFact {
                file,
                line: 10,
                col: 1,
            });
        // `fn g(x, _loc) { return x + 1; }` with caller_location = true
        let mut g_sig = sig(2);
        g_sig.abi.caller_location = true;
        let mut g_b = FuncBuilder::new(item(2), g_sig);
        let x = g_b.param(I32, O);
        let _unused_loc = g_b.param(I32, O);
        let one = g_b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            I32,
            O,
        );
        let sum = g_b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: x,
                r: one,
                ty: I32,
            },
            I32,
            O,
        );
        g_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(sum)));
        let g = g_b.finish();

        // Call `g(p)` with 1 explicit arg and `track_caller: Some(fact)`.
        let mut f_b = FuncBuilder::new(item(1), sig(1));
        let p = f_b.param(I32, O);
        let g_str = prog.strid("g");
        let call = f_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: g_str,
                },
                function: None,
                args: vec![p],
                track_caller: Some(fact),
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(call)));
        let f = f_b.finish();

        prog.add("g", g);
        prog.add("f", f);
        let stats = prog.run(100_000, true);
        assert_eq!(
            stats.applied, 1,
            "unused #[track_caller] callee should inline"
        );
        assert_eq!(prog.calls_in("f"), 0);
    }

    #[test]
    fn dead_constant_branch_is_cleaned_before_pricing() {
        let mut prog = Prog::new();
        // `fn g(flag, x) { if flag == 0 { return x + 1; } else { return x + 1 + 2 + ...; } }`
        // When called as `g(0, p)`, pre-pricing DCE eliminates the huge dead arm so `after <= limit`.
        let mut g_b = FuncBuilder::new(item(2), sig(2));
        let flag = g_b.param(I32, O);
        let x = g_b.param(I32, O);
        let b_dead = g_b.block();
        let b_live = g_b.block();
        g_b.terminate(
            FuncBuilder::ENTRY,
            RTerm::Switch {
                scrut: flag,
                scrut_ty: I32,
                cases: vec![(0, b_live, Vec::new())],
                default: (b_dead, Vec::new()),
            },
        );
        let mut dead_acc = x;
        for bits in 1..40 {
            let c = g_b.op(
                b_dead,
                OpKind::Const(ConstTree::Scalar { bits, bytes: 4 }),
                I32,
                O,
            );
            dead_acc = g_b.op(
                b_dead,
                OpKind::Binary {
                    op: BinaryOp::Add,
                    l: dead_acc,
                    r: c,
                    ty: I32,
                },
                I32,
                O,
            );
        }
        g_b.terminate(b_dead, RTerm::Return(Some(dead_acc)));
        let one = g_b.op(
            b_live,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            I32,
            O,
        );
        let live_sum = g_b.op(
            b_live,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: x,
                r: one,
                ty: I32,
            },
            I32,
            O,
        );
        g_b.terminate(b_live, RTerm::Return(Some(live_sum)));
        let g = g_b.finish();

        let mut f_b = FuncBuilder::new(item(1), sig(1));
        let p = f_b.param(I32, O);
        let zero = f_b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            I32,
            O,
        );
        let g_str = prog.strid("g");
        let call = f_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: g_str,
                },
                function: None,
                args: vec![zero, p],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(call)));
        let f = f_b.finish();

        prog.add("g", g);
        prog.add("f", f);
        let stats = prog.run(100_000, true);
        assert_eq!(
            stats.applied, 1,
            "DCE before pricing removes constant-folded dead arm"
        );
        assert_eq!(prog.calls_in("f"), 0);
    }

    #[test]
    fn forwarded_track_caller_call_receives_site_fact_when_inlined() {
        let mut prog = Prog::new();
        let file = prog.strid("caller.rs");
        let site_fact = prog
            .interns
            .intern_source_fact(crate::jir::origin::SourceFact {
                file,
                line: 42,
                col: 5,
            });
        let caller_loc_name = prog.strid("caller_location");
        let line_of_caller_str = prog.strid("line_of_caller");
        let through_str = prog.strid("through");

        // `#[track_caller] fn line_of_caller() -> u32 { intrinsic("caller_location") }`
        let mut loc_sig = sig(1);
        loc_sig.abi.caller_location = true;
        let mut loc_b = FuncBuilder::new(item(3), loc_sig);
        let _loc_param = loc_b.param(I32, O);
        let loc_val = loc_b.op(
            FuncBuilder::ENTRY,
            OpKind::Intrinsic {
                name: caller_loc_name,
                types: Vec::new(),
                args: Vec::new(),
                fallback: None,
                track_caller: None,
            },
            I32,
            O,
        );
        loc_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(loc_val)));
        let line_of_caller = loc_b.finish();

        // `#[track_caller] fn through() -> u32 { line_of_caller() }` (track_caller: None)
        let mut through_sig = sig(1);
        through_sig.abi.caller_location = true;
        let mut through_b = FuncBuilder::new(item(2), through_sig);
        let _unused_loc = through_b.param(I32, O);
        let forwarded = through_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(3),
                    link_name: line_of_caller_str,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        through_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(forwarded)));
        let through = through_b.finish();

        // `fn caller() -> u32 { through() }` with `track_caller: Some(site_fact)`
        let mut caller_b = FuncBuilder::new(item(1), sig(0));
        let res = caller_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: through_str,
                },
                function: None,
                args: Vec::new(),
                track_caller: Some(site_fact),
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        caller_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(res)));
        let caller = caller_b.finish();

        prog.add("line_of_caller", line_of_caller);
        prog.add("through", through);
        prog.add("caller", caller);
        let stats = prog.run(100_000, true);
        assert_eq!(stats.applied, 1, "through should inline into caller");

        let calls: Vec<_> = prog
            .func("caller")
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter_map(|op| match &op.kind {
                OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    track_caller,
                    ..
                } => Some((*item, *track_caller)),
                _ => None,
            })
            .collect();
        assert_eq!(calls, vec![(item(3), Some(site_fact))]);
    }

    #[test]
    fn track_caller_callee_with_assert_is_refused_in_non_track_caller_site() {
        let mut prog = Prog::new();
        let file = prog.strid("caller.rs");
        let site_fact = prog
            .interns
            .intern_source_fact(crate::jir::origin::SourceFact {
                file,
                line: 7,
                col: 1,
            });
        let g_str = prog.strid("g");

        let mut g_sig = sig(2);
        g_sig.abi.caller_location = true;
        let mut g_b = FuncBuilder::new(item(2), g_sig);
        let x = g_b.param(I32, O);
        let _unused_loc = g_b.param(I32, O);
        g_b.effect(
            FuncBuilder::ENTRY,
            OpKind::Assert {
                cond: x,
                expected: true,
                msg: RAssertMessage::BoundsCheck {
                    length: x,
                    index: x,
                },
                track_caller: None,
            },
            O,
        );
        g_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(x)));
        let g = g_b.finish();

        let mut f_b = FuncBuilder::new(item(1), sig(1));
        let p = f_b.param(I32, O);
        let call = f_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: g_str,
                },
                function: None,
                args: vec![p],
                track_caller: Some(site_fact),
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(call)));
        let f = f_b.finish();

        prog.add("g", g);
        prog.add("f", f);
        let stats = prog.run(100_000, true);
        assert_eq!(stats.applied, 0);
        assert_eq!(
            stats.declined.get("inline.track-caller-unavailable"),
            Some(&1)
        );
    }

    #[test]
    fn sole_caller_negative_net_lifts_two_x_caller_limit() {
        let mut prog = Prog::new();
        let g_str = prog.strid("g");

        let mut g_b = FuncBuilder::new(item(2), sig(1));
        let gx = g_b.param(I32, O);
        g_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(gx)));
        let g = g_b.finish();

        let mut f_b = FuncBuilder::new(item(1), sig(1));
        let fx = f_b.param(I32, O);
        let call = f_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: g_str,
                },
                function: None,
                args: vec![fx],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(call)));
        let f = f_b.finish();

        prog.add("g", g);
        prog.add("f", f);
        let f_name = JsName::new("f".to_owned());
        let g_name = JsName::new("g".to_owned());
        let interns = &prog.interns;
        let funcs = &mut prog.funcs;
        let interns_map: BTreeMap<JsName, &Interns> =
            funcs.keys().map(|name| (name.clone(), interns)).collect();
        let graph = callgraph::build(funcs, &interns_map);
        let mut metadata = funcs
            .keys()
            .map(|name| {
                (
                    name.clone(),
                    Metadata {
                        universe: interns,
                        interns: std::borrow::Cow::Borrowed(interns),
                        origins: None,
                        header: None,
                    },
                )
            })
            .collect();
        let items = ItemIndex::new();
        let mut sizes = BTreeMap::new();
        // Tiny caller (40 B) would fail `after > entry_bytes * 2` (= 80 B) when `after = 120 B`,
        // yet sole-caller removal shrinks total program (120 - 40 - 200 = -120 <= 0).
        sizes.insert(f_name.clone(), 40);
        sizes.insert(g_name.clone(), 200);
        let stats = run_with_mode(
            funcs,
            &mut metadata,
            &graph,
            &items,
            &|_name, func, _meta| {
                let mut refs = BTreeSet::new();
                for op in func.blocks.iter().flat_map(|b| &b.ops) {
                    if let OpKind::Call {
                        callee: Callee::Direct { .. },
                        ..
                    } = &op.kind
                    {
                        refs.insert(g_name.clone());
                    }
                }
                let bytes = if func.item == item(1) && refs.is_empty() {
                    120
                } else {
                    40
                };
                Some((bytes, refs))
            },
            &|callee, caller| callee == &g_name && caller == &f_name,
            &|_| true,
            10_000,
            &sizes,
            false,
        );
        assert_eq!(stats.applied, 1, "sole-caller net <= 0 lifts 2x entry cap");
        assert_eq!(prog.calls_in("f"), 0);
    }

    #[test]
    fn sole_caller_inlines_host_boundary_while_multi_caller_declines() {
        let make_prog = |sole: bool| {
            let mut prog = Prog::new();
            let h_str = prog.strid("h");
            let bytes_id = prog.interns.intern_bytes(&[1, 2, 3]);

            let mut h_b = FuncBuilder::new(item(2), sig(1));
            let hx = h_b.param(I32, O);
            h_b.effect(
                FuncBuilder::ENTRY,
                OpKind::Host {
                    payload: HostPayload::Block(bytes_id),
                    args: vec![hx],
                },
                O,
            );
            h_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(hx)));
            let h = h_b.finish();

            let mut f_b = FuncBuilder::new(item(1), sig(1));
            let fx = f_b.param(I32, O);
            let call = f_b.op(
                FuncBuilder::ENTRY,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: item(2),
                        link_name: h_str,
                    },
                    function: None,
                    args: vec![fx],
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                I32,
                O,
            );
            f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(call)));
            let f = f_b.finish();

            prog.add("h", h);
            prog.add("f", f);
            let stats = prog.run(10_000, sole);
            (prog, stats)
        };

        let (sole_prog, sole_stats) = make_prog(true);
        assert_eq!(sole_stats.applied, 1, "sole caller should inline Host op");
        assert_eq!(sole_prog.calls_in("f"), 0);

        let (multi_prog, multi_stats) = make_prog(false);
        assert_eq!(multi_stats.applied, 0);
        assert_eq!(multi_stats.declined.get("inline.host-boundary"), Some(&1));
        assert_eq!(multi_prog.calls_in("f"), 1);
    }

    #[test]
    fn sole_caller_calling_host_boundary_twice_declines() {
        let mut prog = Prog::new();
        let h_str = prog.strid("h");
        let section = prog.strid("rcgjs.host.custom");

        let mut h_b = FuncBuilder::new(item(2), sig(1));
        let hx = h_b.param(I32, O);
        h_b.effect(
            FuncBuilder::ENTRY,
            OpKind::Host {
                payload: HostPayload::Marker {
                    section,
                    def: item(2).definition,
                    template: None,
                    helpers: Vec::new(),
                },
                args: vec![hx],
            },
            O,
        );
        h_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(hx)));
        let h = h_b.finish();

        let mut f_b = FuncBuilder::new(item(1), sig(1));
        let fx = f_b.param(I32, O);
        let first = f_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: h_str,
                },
                function: None,
                args: vec![fx],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        let second = f_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: h_str,
                },
                function: None,
                args: vec![first],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(second)));
        let f = f_b.finish();

        prog.add("h", h);
        prog.add("f", f);
        let stats = prog.run(10_000, true);
        assert_eq!(stats.applied, 0);
        assert_eq!(stats.declined.get("inline.host-boundary"), Some(&2));
        assert_eq!(prog.calls_in("f"), 2);
    }

    #[test]
    fn loop_virtualizable_sole_caller_retains_sole_credit_and_size_limit() {
        use crate::jir::semantic::AggregateKind;
        const REF_I32: TypeId = TypeId(2);
        let mut prog = Prog::new();
        prog.interns.intern_type(TypeDescriptor {
            id: REF_I32,
            name: None,
            kind: TypeKind::Reference {
                pointee: I32,
                mutable: false,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: REF_I32,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });
        let g_str = prog.strid("g");

        let g_sig = Signature {
            arguments: vec![REF_I32],
            result: I32,
            abi: AbiSignature {
                parameters: 1,
                ..AbiSignature::default()
            },
        };
        let mut g_b = FuncBuilder::new(item(2), g_sig);
        let gp = g_b.param(REF_I32, O);
        let gv = g_b.op(FuncBuilder::ENTRY, OpKind::Deref(gp), I32, O);
        g_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(gv)));
        let g = g_b.finish();

        // Call `g(&x)` inside a self-looping block so `qualifies()` returns `LoopVirtualizable`,
        // while `f` is also `g`'s sole referrer. Before the fix, `LoopVirtualizable` shadowed
        // `SoleCaller`, zeroing `removed` and capping `size_limit = 2 * entry_bytes`.
        let mut f_b = FuncBuilder::new(item(1), sig(1));
        let fx = f_b.param(I32, O);
        let cell = f_b.cell(I32, false, O);
        let header = f_b.block();
        let exit = f_b.block();
        f_b.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(cell), O);
        f_b.effect(FuncBuilder::ENTRY, OpKind::CellStore { cell, v: fx }, O);
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Goto(header, Vec::new()));
        let rx = f_b.op(
            header,
            OpKind::BorrowView {
                root: Root::Cell(cell),
                path: Vec::new(),
                mutable: false,
            },
            REF_I32,
            O,
        );
        let call = f_b.op(
            header,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: g_str,
                },
                function: None,
                args: vec![rx],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        let cond = f_b.op(
            header,
            OpKind::Binary {
                op: BinaryOp::Lt,
                ty: I32,
                l: call,
                r: fx,
            },
            I32,
            O,
        );
        f_b.terminate(
            header,
            RTerm::Switch {
                scrut: cond,
                scrut_ty: I32,
                cases: vec![(0, exit, Vec::new())],
                default: (header, Vec::new()),
            },
        );
        f_b.terminate(exit, RTerm::Return(Some(call)));
        let _keep = AggregateKind::Tuple;
        let f = f_b.finish();

        prog.add("g", g);
        prog.add("f", f);
        let f_name = JsName::new("f".to_owned());
        let g_name = JsName::new("g".to_owned());
        let interns = &prog.interns;
        let funcs = &mut prog.funcs;
        let interns_map: BTreeMap<JsName, &Interns> =
            funcs.keys().map(|name| (name.clone(), interns)).collect();
        let graph = callgraph::build(funcs, &interns_map);
        let mut metadata = funcs
            .keys()
            .map(|name| {
                (
                    name.clone(),
                    Metadata {
                        universe: interns,
                        interns: std::borrow::Cow::Borrowed(interns),
                        origins: None,
                        header: None,
                    },
                )
            })
            .collect();
        let items = ItemIndex::new();
        // `entry_bytes = 50` (2x limit = 100) and `after = 150` (> 100), while `g` is 200 B so
        // sole-referrer removal yields `net = 100 - 200 = -100 <= 0` and lifts `size_limit`.
        let sizes = BTreeMap::from([(f_name.clone(), 50), (g_name.clone(), 200)]);
        let stats = run_with_mode(
            funcs,
            &mut metadata,
            &graph,
            &items,
            &|_name, func, _meta| {
                let mut refs = BTreeSet::new();
                for op in func.blocks.iter().flat_map(|b| &b.ops) {
                    if let OpKind::Call {
                        callee: Callee::Direct { .. },
                        ..
                    } = &op.kind
                    {
                        refs.insert(g_name.clone());
                    }
                }
                let bytes = if func.item == item(1) && refs.is_empty() {
                    150
                } else {
                    50
                };
                Some((bytes, refs))
            },
            &|callee, caller| callee == &g_name && caller == &f_name,
            &|_| true,
            10_000,
            &sizes,
            false,
        );
        assert_eq!(
            stats.applied, 1,
            "sole caller inside loop must receive sole credit and lifted size limit: {:?}",
            stats.declined
        );
    }

    #[test]
    fn bounded_loop_virtualization_admits_up_to_560b_iterator_step() {
        use crate::jir::types::{FieldDescriptor, FieldKey, VariantDescriptor};
        const PAIR: TypeId = TypeId(2);
        const REF_MUT_PAIR: TypeId = TypeId(3);
        const OPT_I32: TypeId = TypeId(4);

        let mut prog = Prog::new();
        let pair_name = prog.strid("Pair");
        let none_name = prog.strid("None");
        let some_name = prog.strid("Some");
        let g_str = prog.strid("iter_next");
        let empty_js_facts = |peeled: TypeId| JsFacts {
            int: None,
            is_indirect: false,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: false,
            fields: Vec::new(),
        };
        let pair_fields = vec![
            FieldDescriptor {
                key: FieldKey::Index(0),
                ty: I32,
                offset: 0,
            },
            FieldDescriptor {
                key: FieldKey::Index(1),
                ty: I32,
                offset: 4,
            },
        ];
        prog.interns.intern_type(TypeDescriptor {
            id: PAIR,
            name: None,
            kind: TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: 10,
                },
                args: Vec::new(),
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: pair_name,
                    fields: pair_fields.clone(),
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                is_indirect: true,
                needs_clone: true,
                fields: pair_fields,
                ..empty_js_facts(PAIR)
            },
        });
        prog.interns.intern_type(TypeDescriptor {
            id: REF_MUT_PAIR,
            name: None,
            kind: TypeKind::Reference {
                pointee: PAIR,
                mutable: true,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: empty_js_facts(REF_MUT_PAIR),
        });
        prog.interns.intern_type(TypeDescriptor {
            id: OPT_I32,
            name: None,
            kind: TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: 11,
                },
                args: Vec::new(),
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: none_name,
                        fields: Vec::new(),
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: some_name,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Index(0),
                            ty: I32,
                            offset: 4,
                        }],
                        logical_discr: 1,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
                niche: None,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: empty_js_facts(OPT_I32),
        });

        let g_sig = Signature {
            arguments: vec![REF_MUT_PAIR],
            result: OPT_I32,
            abi: AbiSignature {
                parameters: 1,
                ..AbiSignature::default()
            },
        };
        let mut g_b = FuncBuilder::new(item(2), g_sig);
        let _self_p = g_b.param(REF_MUT_PAIR, O);
        let ret = g_b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Unit),
            OPT_I32,
            O,
        );
        g_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(ret)));
        let g = g_b.finish();

        // Caller calls `iter_next` twice inside a loop; the first site pushes `growth` up, so
        // the second site (`callee_bytes = 455`, `delta = 300`) exceeds `BUDGET_FLOOR` (384) and
        // only passes via widened `bounded_loop_virtualization` (`<= 560` and `+360`).
        let mut f_b = FuncBuilder::new(item(1), sig(1));
        let fx = f_b.param(I32, O);
        let header = f_b.block();
        let exit = f_b.block();
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Goto(header, Vec::new()));
        let pair_v = f_b.op(header, OpKind::Const(ConstTree::Unit), PAIR, O);
        let r_pair = f_b.op(
            header,
            OpKind::BorrowView {
                root: Root::Value(pair_v),
                path: Vec::new(),
                mutable: true,
            },
            REF_MUT_PAIR,
            O,
        );
        for _ in 0..2 {
            let _ = f_b.op(
                header,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: item(2),
                        link_name: g_str,
                    },
                    function: None,
                    args: vec![r_pair],
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                OPT_I32,
                O,
            );
        }
        f_b.terminate(
            header,
            RTerm::Switch {
                scrut: fx,
                scrut_ty: I32,
                cases: vec![(0, exit, Vec::new())],
                default: (header, Vec::new()),
            },
        );
        f_b.terminate(exit, RTerm::Return(Some(fx)));
        let f = f_b.finish();

        prog.add("iter_next", g);
        prog.add("f", f);
        let f_name = JsName::new("f".to_owned());
        let g_name = JsName::new("iter_next".to_owned());
        let interns = &prog.interns;
        let funcs = &mut prog.funcs;
        let interns_map: BTreeMap<JsName, &Interns> =
            funcs.keys().map(|name| (name.clone(), interns)).collect();
        let graph = callgraph::build(funcs, &interns_map);
        let mut metadata = funcs
            .keys()
            .map(|name| {
                (
                    name.clone(),
                    Metadata {
                        universe: interns,
                        interns: std::borrow::Cow::Borrowed(interns),
                        origins: None,
                        header: None,
                    },
                )
            })
            .collect();
        let items = ItemIndex::new();
        let sizes = BTreeMap::from([(f_name.clone(), 1_000), (g_name.clone(), 455)]);
        let stats = run_with_mode(
            funcs,
            &mut metadata,
            &graph,
            &items,
            &|_name, func, _meta| {
                let calls = func
                    .blocks
                    .iter()
                    .flat_map(|b| &b.ops)
                    .filter(|op| matches!(op.kind, OpKind::Call { .. }))
                    .count() as u64;
                let refs = if calls > 0 {
                    BTreeSet::from([g_name.clone()])
                } else {
                    BTreeSet::new()
                };
                // 2 calls -> 1000 B; 1 call -> 1300 B (+300); 0 calls -> 1600 B (+300).
                let bytes = if func.item == item(1) {
                    1_000 + (2 - calls) * 300
                } else {
                    455
                };
                Some((bytes, refs))
            },
            &|_, _| false,
            &|_| true,
            1_000,
            &sizes,
            false,
        );
        assert_eq!(
            stats.applied, 2,
            "widened bounded_loop_virtualization must admit 455 B / +300 B steps past budget: {:?}",
            stats.declined
        );
    }

    #[test]
    fn shared_callee_footprint_up_to_96_bytes_inlines_when_shrinking() {
        let mut prog = Prog::new();
        let g_str = prog.strid("shared_small");

        let mut g_b = FuncBuilder::new(item(2), sig(1));
        let gx = g_b.param(I32, O);
        let res = g_b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: BinaryOp::Add,
                ty: I32,
                l: gx,
                r: gx,
            },
            I32,
            O,
        );
        g_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(res)));
        let g = g_b.finish();

        let mut f_b = FuncBuilder::new(item(1), sig(1));
        let fx = f_b.param(I32, O);
        let call = f_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: g_str,
                },
                function: None,
                args: vec![fx],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(call)));
        let f = f_b.finish();

        prog.add("shared_small", g);
        prog.add("f", f);
        let f_name = JsName::new("f".to_owned());
        let g_name = JsName::new("shared_small".to_owned());
        let interns = &prog.interns;
        let funcs = &mut prog.funcs;
        let interns_map: BTreeMap<JsName, &Interns> =
            funcs.keys().map(|name| (name.clone(), interns)).collect();
        let graph = callgraph::build(funcs, &interns_map);
        let mut metadata = funcs
            .keys()
            .map(|name| {
                (
                    name.clone(),
                    Metadata {
                        universe: interns,
                        interns: std::borrow::Cow::Borrowed(interns),
                        origins: None,
                        header: None,
                    },
                )
            })
            .collect();
        let items = ItemIndex::new();
        // Shared callee at 88 B (in 65..=96): admitted when caller shrinks (`120 -> 116`).
        let sizes = BTreeMap::from([(f_name.clone(), 120), (g_name.clone(), 88)]);
        let stats = run_with_mode(
            funcs,
            &mut metadata,
            &graph,
            &items,
            &|_name, func, _meta| {
                let mut refs = BTreeSet::new();
                for op in func.blocks.iter().flat_map(|b| &b.ops) {
                    if matches!(op.kind, OpKind::Call { .. }) {
                        refs.insert(g_name.clone());
                    }
                }
                let bytes = if func.item == item(1) && refs.is_empty() {
                    116
                } else if func.item == item(1) {
                    120
                } else {
                    88
                };
                Some((bytes, refs))
            },
            &|_, _| false,
            &|_| true,
            10_000,
            &sizes,
            false,
        );
        assert_eq!(stats.applied, 1);
    }

    #[test]
    fn declines_inline_never_even_for_tiny_sole_caller() {
        let mut prog = Prog::new();
        let g_str = prog.strid("g");
        let mut g_sig = sig(1);
        g_sig.abi.inline_never = true;
        let mut g_b = FuncBuilder::new(item(2), g_sig);
        let gp = g_b.param(I32, O);
        g_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(gp)));
        let g = g_b.finish();

        let mut f_b = FuncBuilder::new(item(1), sig(1));
        let fx = f_b.param(I32, O);
        let call = f_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: g_str,
                },
                function: None,
                args: vec![fx],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(call)));
        let f = f_b.finish();

        prog.add("g", g);
        prog.add("f", f);
        let stats = prog.run(10_000, true);
        assert_eq!(stats.applied, 0);
        assert_eq!(stats.declined.get("inline.never"), Some(&1));
        assert_eq!(prog.calls_in("f"), 1);
    }

    #[test]
    fn used_track_caller_location_param_is_substituted_when_site_fact_known() {
        use crate::jir::origin::SourceFact;
        use crate::jir::types::{
            FieldDescriptor, FieldKey, NominalForm, PointerForm, VariantDescriptor,
        };

        let mut prog = Prog::new();
        let file = prog.strid("caller_site.rs");
        let site_fact = prog.interns.intern_source_fact(SourceFact {
            file,
            line: 77,
            col: 8,
        });
        let str_ty = TypeId(10);
        let ref_str_ty = TypeId(11);
        let u32_ty = TypeId(12);
        let loc_ty = TypeId(13);
        let ref_loc_ty = TypeId(14);

        let loc_name = prog.strid("Location");
        let f_file = prog.strid("file");
        let f_line = prog.strid("line");
        let f_col = prog.strid("col");
        let g_str = prog.strid("used_loc_callee");

        let js_facts = |peeled: TypeId| JsFacts {
            int: None,
            is_indirect: false,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: false,
            fields: Vec::new(),
        };

        prog.interns.intern_type(TypeDescriptor {
            id: str_ty,
            name: None,
            kind: TypeKind::Str,
            layout: LayoutFacts {
                sized: false,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: js_facts(str_ty),
        });
        prog.interns.intern_type(TypeDescriptor {
            id: ref_str_ty,
            name: None,
            kind: TypeKind::Reference {
                pointee: str_ty,
                mutable: false,
            },
            layout: LayoutFacts {
                sized: true,
                size: Some(8),
                align: Some(4),
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                pointer_form: Some(PointerForm::Str),
                ..js_facts(ref_str_ty)
            },
        });
        prog.interns.intern_type(TypeDescriptor {
            id: u32_ty,
            name: None,
            kind: TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            layout: LayoutFacts {
                sized: true,
                size: Some(4),
                align: Some(4),
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: Some(IntFacts {
                    signed: false,
                    bits: 32,
                    repr: IntRepr::Number,
                }),
                ..js_facts(u32_ty)
            },
        });
        let loc_fields = vec![
            FieldDescriptor {
                key: FieldKey::Name(f_file),
                ty: ref_str_ty,
                offset: 0,
            },
            FieldDescriptor {
                key: FieldKey::Name(f_line),
                ty: u32_ty,
                offset: 8,
            },
            FieldDescriptor {
                key: FieldKey::Name(f_col),
                ty: u32_ty,
                offset: 12,
            },
        ];
        prog.interns.intern_type(TypeDescriptor {
            id: loc_ty,
            name: Some(loc_name),
            kind: TypeKind::Adt {
                def: item(99).definition,
                form: NominalForm::Struct,
                args: Vec::new(),
                variants: vec![VariantDescriptor {
                    name: loc_name,
                    logical_discr: 0,
                    physical_tag: None,
                    fields: loc_fields.clone(),
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            layout: LayoutFacts {
                sized: true,
                size: Some(16),
                align: Some(4),
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                is_indirect: true,
                needs_clone: true,
                fields: loc_fields,
                ..js_facts(loc_ty)
            },
        });
        prog.interns.intern_type(TypeDescriptor {
            id: ref_loc_ty,
            name: None,
            kind: TypeKind::Reference {
                pointee: loc_ty,
                mutable: false,
            },
            layout: LayoutFacts {
                sized: true,
                size: Some(4),
                align: Some(4),
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                pointer_form: Some(PointerForm::Thin),
                ..js_facts(ref_loc_ty)
            },
        });

        let mut g_sig = Signature {
            arguments: vec![u32_ty],
            result: u32_ty,
            abi: AbiSignature {
                parameters: 2,
                caller_location: true,
                ..AbiSignature::default()
            },
        };
        g_sig.abi.caller_location = true;
        let mut g_b = FuncBuilder::new(item(2), g_sig);
        let x = g_b.param(u32_ty, O);
        let loc_param = g_b.param(ref_loc_ty, O);
        let loc_val = g_b.op(FuncBuilder::ENTRY, OpKind::Deref(loc_param), loc_ty, O);
        let line_val = g_b.op(
            FuncBuilder::ENTRY,
            OpKind::Field {
                base: loc_val,
                variant: None,
                field: 1,
            },
            u32_ty,
            O,
        );
        let sum = g_b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: x,
                r: line_val,
                ty: u32_ty,
            },
            u32_ty,
            O,
        );
        g_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(sum)));
        let g = g_b.finish();

        let f_sig = Signature {
            arguments: vec![u32_ty],
            result: u32_ty,
            abi: AbiSignature {
                parameters: 1,
                ..AbiSignature::default()
            },
        };
        let mut f_b = FuncBuilder::new(item(1), f_sig);
        let p = f_b.param(u32_ty, O);
        let call = f_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: g_str,
                },
                function: None,
                args: vec![p],
                track_caller: Some(site_fact),
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            u32_ty,
            O,
        );
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(call)));
        let f = f_b.finish();

        prog.add("used_loc_callee", g);
        prog.add("f", f);
        let stats = prog.run(100_000, true);
        assert_eq!(
            stats.applied, 1,
            "used #[track_caller] callee should inline"
        );
        assert_eq!(prog.calls_in("f"), 0);
        let f_after = prog.func("f");
        f_after.verify(&prog.interns.types).expect("verified");
        let has_loc_const = f_after.blocks.iter().flat_map(|b| &b.ops).any(|op| {
            matches!(
                &op.kind,
                OpKind::Const(ConstTree::Reference(inner))
                    if matches!(
                        inner.as_ref(),
                        ConstTree::Aggregate(fields)
                            if fields.len() == 3
                                && fields[0] == ConstTree::Str(file)
                                && fields[1] == ConstTree::Scalar { bits: 77, bytes: 4 }
                                && fields[2] == ConstTree::Scalar { bits: 9, bytes: 4 }
                    )
            )
        });
        assert!(
            has_loc_const,
            "expected bound Location constant for site_fact"
        );
    }

    #[test]
    fn track_caller_dead_assert_folds_and_live_assert_declines() {
        use crate::jir::origin::{Origin, OriginKind, OriginTable, SourceFact};
        use std::borrow::Cow;

        let build_origins = |fact| OriginTable {
            origins: vec![Origin {
                source: Some(fact),
                local_name: None,
                kind: OriginKind::Statement,
                inline_stack: Vec::new(),
            }],
            statements: Vec::new(),
            terminators: Vec::new(),
            locals: Vec::new(),
            item_path: StrId(0),
            root: None,
        };

        let make_assert_callee = || {
            let mut g_sig = sig(2);
            g_sig.abi.caller_location = true;
            let mut g_b = FuncBuilder::new(item(2), g_sig);
            let x = g_b.param(I32, OriginId(0));
            let _loc = g_b.param(I32, OriginId(0));
            let zero = g_b.op(
                FuncBuilder::ENTRY,
                OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
                I32,
                OriginId(0),
            );
            let cmp = g_b.op(
                FuncBuilder::ENTRY,
                OpKind::Binary {
                    op: BinaryOp::Eq,
                    l: x,
                    r: zero,
                    ty: I32,
                },
                I32,
                OriginId(0),
            );
            g_b.effect(
                FuncBuilder::ENTRY,
                OpKind::Assert {
                    cond: cmp,
                    expected: false,
                    msg: RAssertMessage::RemainderByZero(x),
                    track_caller: None,
                },
                OriginId(0),
            );
            g_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(x)));
            g_b.finish()
        };

        // Case 1: Live non-optional Assert inside #[track_caller] callee MUST decline even when
        // OriginTable is present (both direct site_track_caller = Some(...) and forwarded None).
        for forward_in_track_caller in [false, true] {
            let mut prog = Prog::new();
            let caller_file = prog.strid("caller.rs");
            let core_file = prog.strid("core/slice.rs");
            let site_fact = prog.interns.intern_source_fact(SourceFact {
                file: caller_file,
                line: 55,
                col: 12,
            });
            let core_fact = prog.interns.intern_source_fact(SourceFact {
                file: core_file,
                line: 999,
                col: 1,
            });
            let g_str = prog.strid("g_assert");
            let g = make_assert_callee();

            let mut f_sig = sig(1);
            f_sig.abi.caller_location = forward_in_track_caller;
            let mut f_b = FuncBuilder::new(item(1), f_sig);
            let p = f_b.param(I32, OriginId(0));
            let call = f_b.op(
                FuncBuilder::ENTRY,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: item(2),
                        link_name: g_str,
                    },
                    function: None,
                    args: vec![p],
                    track_caller: if forward_in_track_caller {
                        None
                    } else {
                        Some(site_fact)
                    },
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                I32,
                OriginId(0),
            );
            f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(call)));
            let f = f_b.finish();

            prog.add("g_assert", g);
            prog.add("f", f);

            let interns = &prog.interns;
            let funcs = &mut prog.funcs;
            let interns_map: BTreeMap<JsName, &Interns> =
                funcs.keys().map(|name| (name.clone(), interns)).collect();
            let graph = callgraph::build(funcs, &interns_map);
            let f_name = JsName::new("f".to_owned());
            let g_name = JsName::new("g_assert".to_owned());
            let mut metadata = BTreeMap::from([
                (
                    f_name.clone(),
                    Metadata {
                        universe: interns,
                        interns: Cow::Borrowed(interns),
                        origins: Some(Cow::Owned(build_origins(site_fact))),
                        header: None,
                    },
                ),
                (
                    g_name.clone(),
                    Metadata {
                        universe: interns,
                        interns: Cow::Borrowed(interns),
                        origins: Some(Cow::Owned(build_origins(core_fact))),
                        header: None,
                    },
                ),
            ]);
            let sizes = BTreeMap::from([(f_name.clone(), 200), (g_name.clone(), 120)]);
            let stats = run_with_mode(
                funcs,
                &mut metadata,
                &graph,
                &ItemIndex::new(),
                &|_, func, _| Some(measurement(func, &metadata_dummy_refs(func, interns))),
                &|_, _| true,
                &|_| true,
                100_000,
                &sizes,
                false,
            );
            assert_eq!(
                stats.applied, 0,
                "live Assert in #[track_caller] callee must decline (forward={forward_in_track_caller})"
            );
            assert_eq!(
                stats
                    .declined
                    .get("inline.track-caller-unavailable")
                    .copied()
                    .unwrap_or(0),
                1
            );
        }

        // Case 2: Dead constant-folded Assert (e.g. ChunksExact::new(_, 16)) folds in DCE and inlines.
        {
            let mut prog = Prog::new();
            let caller_file = prog.strid("caller.rs");
            let site_fact = prog.interns.intern_source_fact(SourceFact {
                file: caller_file,
                line: 77,
                col: 5,
            });
            let g_str = prog.strid("g_assert");
            let g = make_assert_callee();

            let mut f_b = FuncBuilder::new(item(1), sig(0));
            let sixteen = f_b.op(
                FuncBuilder::ENTRY,
                OpKind::Const(ConstTree::Scalar { bits: 16, bytes: 4 }),
                I32,
                OriginId(0),
            );
            let call = f_b.op(
                FuncBuilder::ENTRY,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: item(2),
                        link_name: g_str,
                    },
                    function: None,
                    args: vec![sixteen],
                    track_caller: Some(site_fact),
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                I32,
                OriginId(0),
            );
            f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(call)));
            let f = f_b.finish();

            prog.add("g_assert", g);
            prog.add("f", f);

            let interns = &prog.interns;
            let funcs = &mut prog.funcs;
            let interns_map: BTreeMap<JsName, &Interns> =
                funcs.keys().map(|name| (name.clone(), interns)).collect();
            let graph = callgraph::build(funcs, &interns_map);
            let f_name = JsName::new("f".to_owned());
            let g_name = JsName::new("g_assert".to_owned());
            let mut metadata = BTreeMap::from([
                (
                    f_name.clone(),
                    Metadata {
                        universe: interns,
                        interns: Cow::Borrowed(interns),
                        origins: Some(Cow::Owned(build_origins(site_fact))),
                        header: None,
                    },
                ),
                (
                    g_name.clone(),
                    Metadata {
                        universe: interns,
                        interns: Cow::Borrowed(interns),
                        origins: Some(Cow::Owned(build_origins(site_fact))),
                        header: None,
                    },
                ),
            ]);
            let sizes = BTreeMap::from([(f_name.clone(), 200), (g_name.clone(), 120)]);
            let stats = run_with_mode(
                funcs,
                &mut metadata,
                &graph,
                &ItemIndex::new(),
                &|_, func, _| Some(measurement(func, &metadata_dummy_refs(func, interns))),
                &|_, _| true,
                &|_| true,
                100_000,
                &sizes,
                false,
            );
            assert_eq!(
                stats.applied, 1,
                "constant-folded dead Assert in #[track_caller] callee should inline (declined={:?})",
                stats.declined
            );
            assert_eq!(prog.calls_in("f"), 0);
        }
    }

    fn metadata_dummy_refs<'a>(func: &Func, interns: &'a Interns) -> Metadata<'a> {
        let _ = func;
        Metadata {
            universe: interns,
            interns: std::borrow::Cow::Borrowed(interns),
            origins: None,
            header: None,
        }
    }

    #[test]
    fn trivial_pure_leaf_wrapper_inlines_even_above_96_bytes() {
        let mut prog = Prog::new();
        let g = add_one(&mut prog);
        let f = calls_g(&mut prog);
        prog.add("g", g);
        prog.add("f", f);

        let f_name = JsName::new("f".to_owned());
        let g_name = JsName::new("g".to_owned());
        let interns = &prog.interns;
        let funcs = &mut prog.funcs;
        let interns_map: BTreeMap<JsName, &Interns> =
            funcs.keys().map(|name| (name.clone(), interns)).collect();
        let graph = callgraph::build(funcs, &interns_map);
        let mut metadata = funcs
            .keys()
            .map(|name| {
                (
                    name.clone(),
                    Metadata {
                        universe: interns,
                        interns: std::borrow::Cow::Borrowed(interns),
                        origins: None,
                        header: None,
                    },
                )
            })
            .collect();
        // Shared callee (`sole_referrer = false`) priced at 120 bytes (> 96B SHARED_CALLEE_FOOTPRINT),
        // with +12B temporary caller growth (within <=16B trivial leaf slack) and 0 budget charge.
        let sizes = BTreeMap::from([(f_name.clone(), 100), (g_name.clone(), 120)]);
        let stats = run_with_mode(
            funcs,
            &mut metadata,
            &graph,
            &ItemIndex::new(),
            &|_, func, _| {
                let mut refs = BTreeSet::new();
                for op in func.blocks.iter().flat_map(|b| &b.ops) {
                    if matches!(op.kind, OpKind::Call { .. }) {
                        refs.insert(g_name.clone());
                    }
                }
                let bytes = if func.item == item(1) && refs.is_empty() {
                    112
                } else if func.item == item(1) {
                    100
                } else {
                    120
                };
                Some((bytes, refs))
            },
            &|_, _| false,
            &|_| true,
            10_000,
            &sizes,
            false,
        );
        assert_eq!(
            stats.applied, 1,
            "trivial pure leaf above 96B should inline"
        );
        assert_eq!(prog.calls_in("f"), 0);
    }

    #[test]
    fn multi_site_sole_caller_batching_inlines_when_single_sites_have_positive_net() {
        let mut prog = Prog::new();
        let g = add_one(&mut prog);
        let g_str = prog.strid("g");

        let mut f_b = FuncBuilder::new(item(1), sig(1));
        let mut cur = f_b.param(I32, O);
        for _ in 0..3 {
            cur = f_b.op(
                FuncBuilder::ENTRY,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: item(2),
                        link_name: g_str,
                    },
                    function: None,
                    args: vec![cur],
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                I32,
                O,
            );
        }
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(cur)));
        let f = f_b.finish();

        prog.add("g", g);
        prog.add("f", f);

        let f_name = JsName::new("f".to_owned());
        let g_name = JsName::new("g".to_owned());
        let interns = &prog.interns;
        let funcs = &mut prog.funcs;
        let interns_map: BTreeMap<JsName, &Interns> =
            funcs.keys().map(|name| (name.clone(), interns)).collect();
        let graph = callgraph::build(funcs, &interns_map);
        let mut metadata = funcs
            .keys()
            .map(|name| {
                (
                    name.clone(),
                    Metadata {
                        universe: interns,
                        interns: std::borrow::Cow::Borrowed(interns),
                        origins: None,
                        header: None,
                    },
                )
            })
            .collect();
        // Make each intermediate site grow by +25B (>16B slack so single-site rejects when refs still contain g),
        // while batching all 3 sites yields 150 -> 180 (+30B) - 80B removed = -50B net!
        let sizes = BTreeMap::from([(f_name.clone(), 150), (g_name.clone(), 80)]);
        let stats = run_with_mode(
            funcs,
            &mut metadata,
            &graph,
            &ItemIndex::new(),
            &|_, func, _| {
                let mut call_count = 0u64;
                let mut refs = BTreeSet::new();
                for op in func.blocks.iter().flat_map(|b| &b.ops) {
                    if matches!(op.kind, OpKind::Call { .. }) {
                        call_count += 1;
                        refs.insert(g_name.clone());
                    }
                }
                let bytes = if func.item == item(1) {
                    match call_count {
                        3 => 150,
                        2 => 175,
                        1 => 178,
                        _ => 180,
                    }
                } else {
                    80
                };
                Some((bytes, refs))
            },
            &|_, _| true,
            &|_| true,
            10_000,
            &sizes,
            false,
        );
        assert_eq!(
            stats.applied, 3,
            "all 3 sole-caller sites should batch-inline"
        );
        assert_eq!(prog.calls_in("f"), 0);
    }
}
