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
use crate::jir::repr::{
    CellId, Func, Op, OpKind, Proj, RAssertMessage, RBlock, RBlockId, RTerm, Root, ValueId,
};
use crate::jir::semantic::{CallSemantic, Callee, HostPayload};
use crate::jir::types::{NominalForm, TypeKind, TypeTable};

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
const SHARED_CALLEE_FOOTPRINT: u64 = 44;
/// A `#[track_caller]` helper with an unused location parameter drops an argument at every site.
const TRACK_CALLER_SHARED_FOOTPRINT: u64 = 84;

/// The least program-ledger charge per accepted site bounds duplication even for local wins.
/// Compression can lose sharing when a body replaces repeated calls.
const MIN_SITE_CHARGE: i64 = 24;
const BUDGET_FLOOR: u64 = 384;

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
    run_with_mode(
        funcs,
        metadata,
        graph,
        measure,
        sole_referrer,
        zombie_free,
        program_bytes,
        sizes,
        false,
    )
}

/// Runs the inliner, optionally admitting non-specialized `CallSemantic::Standard` calls after
/// representation specialization (`allow_std_post_repr`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_with_mode(
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    metadata: &mut BTreeMap<JsName, Metadata<'_>>,
    graph: &Callgraph,
    measure: &dyn Fn(&JsName, &Func, &Metadata<'_>) -> Option<Measurement>,
    sole_referrer: &dyn Fn(&JsName, &JsName) -> bool,
    zombie_free: &dyn Fn(&JsName) -> bool,
    program_bytes: u64,
    sizes: &BTreeMap<JsName, u64>,
    allow_std_post_repr: bool,
) -> InlineStats {
    let mut stats = InlineStats::default();
    let mut sizes = sizes.clone();
    let budget = (program_bytes / PROGRAM_BUDGET_DENOMINATOR).max(BUDGET_FLOOR);
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
            zombie_free,
            allow_std_post_repr,
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
        let limit = entry_bytes.saturating_mul(2);
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
                &sizes,
                sole_referrer,
                &current_metadata.interns.types,
                &looping,
            ) else {
                stats.decline("inline.cost-no-opportunity");
                continue;
            };
            let Some(Ok(callee_func)) = funcs.get(&site.callee) else {
                continue;
            };
            let callee_bytes = sizes.get(&site.callee).copied().unwrap_or(u64::MAX);
            let iterator_step = metadata
                .get(&site.callee)
                .is_some_and(|meta| bounded_iterator_step(callee_func, &meta.interns.types));
            let Some(provider) = metadata.get(&site.callee) else {
                continue;
            };
            let origin = current.blocks[site.block].ops[site.op].origin;
            let site_args_len = match &current.blocks[site.block].ops[site.op].kind {
                OpKind::Call { args, .. } => args.len(),
                _ => continue,
            };
            let (mut callee_func, imported) =
                match current_metadata.import(provider, callee_func, origin) {
                    Ok(value) => value,
                    Err(reason) => {
                        stats.decline(reason);
                        continue;
                    }
                };
            if callee_func.sig.abi.caller_location && callee_func.params.len() == site_args_len + 1
            {
                strip_unused_last_param(&mut callee_func);
            }
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
            // Price and retain the same verified shape, including the merges and DCE cleanup.
            let mut candidate = current.clone();
            collapse_forwarders(&mut candidate);
            fuse_straight_lines(&mut candidate);
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
                candidate = candidate_dce;
            }
            if candidate
                .verify_carriers(table, observed_carriers(&candidate))
                .is_err()
            {
                current = backup;
                stats.decline("inline.cleanup-invalid");
                continue;
            }
            let Some((after, refs)) = measure(caller, &candidate, &imported) else {
                current = backup;
                stats.decline("inline.cost-unavailable");
                continue;
            };
            let delta = after as i64 - bytes as i64;
            // The sole-referrer oracle excludes public and explicitly rooted items. Credit
            // deletion only after the emitted caller also lost every reference to the callee,
            // including function-pointer constants and retained indirect uses in this caller.
            let removed = if criterion == Criterion::SoleCaller && !refs.contains(&site.callee) {
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
            let charge = if net <= 0 {
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
            // Only loop virtualization (or <=2B tiny accessors) may spend net growth. Other sites
            // must repay their caller growth through local cleanup or sole-callee removal.
            let growth_allowed =
                matches!(criterion, Criterion::LoopVirtualizable) || tiny_accessor_slack > 0;
            // A small loop helper may expose enough scalar replacement to repay a modest
            // pre-cleanup growth in the later per-item pipeline. Bound both the duplicated body
            // and that temporary growth; this is the iterator-state case, not a general license
            // to spend the whole-program compression budget.
            let bounded_loop_virtualization = matches!(criterion, Criterion::LoopVirtualizable)
                && iterator_step
                && callee_bytes <= 320
                && after <= bytes.saturating_add(96);
            if after > limit
                || after > bytes.saturating_add(callee_bytes)
                || (!bounded_loop_virtualization && growth + charge > budget as i64)
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
                limit: limit.min(u32::MAX as u64) as u32,
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
            growth += charge;
            bytes = after;
            applied_here += 1;
            stats.cross_object_applied +=
                u32::from(!std::ptr::eq(current_metadata.universe, provider.universe));
            current_metadata = imported;
            accepted = Some(candidate);
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
    let [self_param, _step] = func.params.as_slice() else {
        return false;
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

/// Which of D6's criteria admitted a site: the in-loop virtualizable shape may pay for growth
/// (the escape analysis cashes it), the size plays may not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Criterion {
    SmallCallee,
    SoleCaller,
    LoopVirtualizable,
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
    sizes: &BTreeMap<JsName, u64>,
    sole_referrer: &dyn Fn(&JsName, &JsName) -> bool,
    table: &crate::jir::TypeTable,
    looping: &BTreeSet<u32>,
) -> Option<Criterion> {
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
    // under the call's own post-minify footprint.
    let footprint_limit = if site.has_track_caller {
        TRACK_CALLER_SHARED_FOOTPRINT
    } else {
        SHARED_CALLEE_FOOTPRINT
    };
    if sizes
        .get(callee)
        .is_some_and(|bytes| *bytes <= footprint_limit)
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
    zombie_free: &dyn Fn(&JsName) -> bool,
    allow_std_post_repr: bool,
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
                CallSemantic::Standard(std_op) if allow_std_post_repr => !matches!(
                    std_op,
                    crate::jir::semantic::StandardOperation::VecNew
                        | crate::jir::semantic::StandardOperation::VecWithCapacity
                        | crate::jir::semantic::StandardOperation::VecAsSlice
                        | crate::jir::semantic::StandardOperation::VecPush
                        | crate::jir::semantic::StandardOperation::FormatArgumentNewDisplay
                        | crate::jir::semantic::StandardOperation::FormatArgsNew
                        | crate::jir::semantic::StandardOperation::FormatArgsToString
                ),
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
            if let Some(reason) = body_decline(
                callee_func,
                &callee_metadata.interns,
                args.len(),
                track_caller.is_some(),
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

/// The semantic boundary preventing a callee body from being spliced at this site.
fn body_decline(
    callee: &Func,
    interns: &Interns,
    args: usize,
    site_has_track_caller: bool,
) -> Option<&'static str> {
    if callee.has_invoke() {
        return Some("inline.exceptional-control-flow");
    }
    let expected_params = if callee.sig.abi.caller_location {
        if unused_caller_location(callee) && callee.params.len() == args + 1 {
            args + 1
        } else {
            return Some("inline.track-caller-unavailable");
        }
    } else if site_has_track_caller {
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
                // The original item owes the linker its compile-time required-by diagnostic.
                OpKind::Zombie(_) => Some("inline.zombie"),
                OpKind::Intrinsic { name, .. } if interns.str(*name) == Some("caller_location") => {
                    Some("inline.track-caller-unavailable")
                }
                // Host and template lowering may own item-local bindings and observable ordering.
                OpKind::Host { payload, .. } if !independent_host(payload, interns) => {
                    Some("inline.host-boundary")
                }
                OpKind::Call { host, semantic, .. }
                    if !host
                        .as_ref()
                        .is_some_and(|payload| independent_host(payload, interns))
                        && (host.is_some()
                            || matches!(
                                semantic,
                                CallSemantic::Host
                                    | CallSemantic::ViewAbi
                                    | CallSemantic::RawJavaScript
                            )) =>
                {
                    Some("inline.host-boundary")
                }
                _ => None,
            }
        })
}

/// These dynamic markers have no item-owned template, helper bindings, or scheduling state.
fn independent_host(payload: &HostPayload, interns: &Interns) -> bool {
    let HostPayload::Marker {
        section,
        template: None,
        helpers,
        ..
    } = payload
    else {
        return false;
    };
    helpers.is_empty()
        && interns
            .str(*section)
            .and_then(|section| section.strip_prefix("rcgjs.host."))
            .and_then(rcgjs_object::host::HostOperation::from_marker)
            .is_some()
}

/// Whether a value of `ty` is something the escape analysis could scalarize at some variant —
/// the briefs' "virtualizable" (an `Option` return counts through its `Some`).
fn virtualizable(table: &crate::jir::TypeTable, ty: crate::jir::semantic::TypeId) -> bool {
    candidates::variant_shape(table, ty, 0).is_some()
        || candidates::variant_shape(table, ty, 1).is_some()
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
            for (name, func) in funcs.iter() {
                if let Ok(func) = func {
                    sizes.insert(name.clone(), measured(func));
                }
            }
            run(
                funcs,
                &mut metadata,
                &graph,
                &measure,
                &sole_referrer,
                &|_: &JsName| true,
                program_bytes,
                &sizes,
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
        // Removing a sole callee still cannot spend more than the caller's own growth limit.
        let mut b = FuncBuilder::new(item(2), sig(1));
        let x = b.param(I32, O);
        let mut acc = x;
        for bits in 0..70 {
            let c = b.op(
                FuncBuilder::ENTRY,
                OpKind::Const(ConstTree::Scalar { bits, bytes: 4 }),
                I32,
                O,
            );
            acc = b.op(
                FuncBuilder::ENTRY,
                OpKind::Binary {
                    op: BinaryOp::Add,
                    l: acc,
                    r: c,
                    ty: I32,
                },
                I32,
                O,
            );
        }
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(acc)));
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
        let stats = prog.run(100_000, true);
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
            assert_eq!(independent_host(&payload, &interns), supported);
            let HostPayload::Marker { template, .. } = &mut payload else {
                unreachable!()
            };
            *template = Some(ConstTree::Unit);
            assert!(!independent_host(&payload, &interns));
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
}
