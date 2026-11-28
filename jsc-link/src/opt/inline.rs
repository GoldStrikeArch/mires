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
//! plus the callee's, a size-criterion
//! site must not grow the measured caller/callee sum under allocated item names, and the ledger —
//! every accepted site pays at least [`MIN_SITE_CHARGE`] — may not pass the program budget (+0.5%
//! of the measured bytes, floored for small programs). The ledger is what holds the compressed-size
//! exit gates: on the idiomatic FingerprintJS port unbounded inlining shrank the minified bundle
//! while growing its brotli form past the gate, so the pass is deliberately stingy and records what
//! it declined. A declined site is reverted whole — no partial inline. Every applied site is
//! re-verified under its observed carriers; a verify failure reverts the site too.
//!
//! Everything is deterministic: components in the graph's order, sites processed back-to-front
//! within a caller, all maps `BTreeMap`.

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use crate::analysis::callgraph::Callgraph;
use crate::item::JsName;
use crate::jir::decline::Decline;
use crate::jir::interns::Interns;
use crate::jir::origin::{OriginId, SourceFactId};
use crate::jir::repr::{
    CellId, Func, Op, OpKind, RAssertMessage, RBlock, RBlockId, RTerm, ValueId,
};
use crate::jir::semantic::{
    BinaryOp, CallSemantic, Callee, ConstTree, DropGlue, GlobalReference, HostPayload, TypeId,
};
use crate::jir::types::{NominalForm, TypeKind, TypeTable};
use crate::lower::js::host::ItemIndex;

use super::cfg::Cfg;
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
/// TurboFan (inline small hot helpers without creating giant functions that bail out of JIT).
const SPEED_SHARED_CALLEE_FOOTPRINT: u64 = 320;
const SPEED_TRACK_CALLER_SHARED_FOOTPRINT: u64 = 384;
const SPEED_LOOP_CALLEE_FOOTPRINT: u64 = 1_024;
const SPEED_INLINE_HINT_FOOTPRINT: u64 = 448;
const SPEED_INLINE_ALWAYS_FOOTPRINT: u64 = 640;
const SPEED_CALLER_CAP: u64 = 32_768;
const SPEED_BUDGET_FLOOR: u64 = 24_576;

/// Why a candidate call site was not inlined.
///
/// Variants are ordered alphabetically by [`Self::as_code`] so `BTreeMap` iteration
/// matches `str` ordering (byte-identical JSON) and [`Borrow<str>`] satisfies its `Ord` contract.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum InlineDecline {
    AbiUnsupported,
    BodyUnavailable,
    CarrierBoundary,
    CarrierConversionUnsupported,
    CleanupInvalid,
    CoroutineUnsupported,
    CostNoOpportunity,
    CostRejected,
    CostUnavailable,
    ExceptionalControlFlow,
    HostBoundary,
    MetadataUnavailable,
    Never,
    ObjectIncompatible,
    OriginBudget,
    OriginInvalid,
    OriginUnavailable,
    RecursiveBudget,
    ReprRetarget,
    SemanticBoundary,
    SignatureInvalid,
    SourceInvalid,
    SpliceInvalid,
    StringInvalid,
    TrackCallerUnavailable,
    TypeBudget,
    TypeConflict,
    TypeOpaque,
    TypeUnavailable,
    Zombie,
}

impl InlineDecline {
    pub const fn as_code(self) -> &'static str {
        match self {
            Self::AbiUnsupported => "inline.abi-unsupported",
            Self::BodyUnavailable => "inline.body-unavailable",
            Self::CarrierBoundary => "inline.carrier-boundary",
            Self::CarrierConversionUnsupported => "inline.carrier-conversion-unsupported",
            Self::CleanupInvalid => "inline.cleanup-invalid",
            Self::CoroutineUnsupported => "inline.coroutine-unsupported",
            Self::CostNoOpportunity => "inline.cost-no-opportunity",
            Self::CostRejected => "inline.cost-rejected",
            Self::CostUnavailable => "inline.cost-unavailable",
            Self::ExceptionalControlFlow => "inline.exceptional-control-flow",
            Self::HostBoundary => "inline.host-boundary",
            Self::MetadataUnavailable => "inline.metadata-unavailable",
            Self::Never => "inline.never",
            Self::ObjectIncompatible => "inline.object-incompatible",
            Self::OriginBudget => "inline.origin-budget",
            Self::OriginInvalid => "inline.origin-invalid",
            Self::OriginUnavailable => "inline.origin-unavailable",
            Self::RecursiveBudget => "inline.recursive-budget",
            Self::ReprRetarget => "inline.repr-retarget",
            Self::SemanticBoundary => "inline.semantic-boundary",
            Self::SignatureInvalid => "inline.signature-invalid",
            Self::SourceInvalid => "inline.source-invalid",
            Self::SpliceInvalid => "inline.splice-invalid",
            Self::StringInvalid => "inline.string-invalid",
            Self::TrackCallerUnavailable => "inline.track-caller-unavailable",
            Self::TypeBudget => "inline.type-budget",
            Self::TypeConflict => "inline.type-conflict",
            Self::TypeOpaque => "inline.type-opaque",
            Self::TypeUnavailable => "inline.type-unavailable",
            Self::Zombie => "inline.zombie",
        }
    }
}

impl Borrow<str> for InlineDecline {
    fn borrow(&self) -> &str {
        self.as_code()
    }
}

impl Hash for InlineDecline {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_code().hash(state);
    }
}

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
    pub declined: BTreeMap<InlineDecline, u32>,
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
    fn decline(&mut self, reason: InlineDecline) {
        *self.declined.entry(reason).or_default() += 1;
    }

    pub fn declines_json(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::from("{");
        for (index, (reason, count)) in self.declined.iter().enumerate() {
            if index != 0 {
                out.push(',');
            }
            crate::lane_report::push_string(&mut out, reason.as_code());
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

/// Cached `RCGJS_O_DEBUG_FUNC` filter, read once per process.
fn debug_func_filter() -> Option<&'static str> {
    static FILTER: OnceLock<Option<String>> = OnceLock::new();
    FILTER
        .get_or_init(|| std::env::var("RCGJS_O_DEBUG_FUNC").ok())
        .as_deref()
}

/// Inputs and policy configuration for one run of the cost-guided inliner.
pub(crate) struct InlineConfig<'a> {
    pub graph: &'a Callgraph,
    pub items: &'a ItemIndex,
    pub measure: &'a dyn Fn(&JsName, &Func, &Metadata<'_>) -> Option<Measurement>,
    pub sole_referrer: &'a dyn Fn(&JsName, &JsName) -> bool,
    pub zombie_free: &'a dyn Fn(&JsName) -> bool,
    pub program_bytes: u64,
    pub sizes: &'a BTreeMap<JsName, u64>,
    pub allow_std_post_repr: bool,
    pub profile: Option<crate::opts::Profile>,
    pub call_retargets: Option<&'a BTreeMap<JsName, crate::lower::CallRetargets>>,
}

/// Runs the inliner over every `Ok` func in place using `config`.
pub(crate) fn run_with_config(
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    metadata: &mut BTreeMap<JsName, Metadata<'_>>,
    config: InlineConfig<'_>,
) -> InlineStats {
    InlineCx::new(config).run(funcs, metadata)
}

/// Budget and growth-ledger evaluator for candidate inline sites.
struct CostModel {
    speed: bool,
    budget: u64,
    /// Growth spent so far, signed: a site that shrinks its caller gives headroom back.
    growth: i64,
}

struct SiteCostInput<'a> {
    caller: &'a JsName,
    site: &'a Site,
    criterion: Criterion,
    is_sole: bool,
    caller_recursive: bool,
    in_loop: bool,
    entry_bytes: u64,
    limit: u64,
    bytes: u64,
    after: u64,
    callee_bytes: u64,
    removed: u64,
    trivial_leaf: bool,
    iterator_step: bool,
    callee_func: &'a Func,
}

struct AcceptedCost {
    charge: i64,
    charge_growth: bool,
}

impl CostModel {
    fn new(speed: bool, program_bytes: u64) -> Self {
        let budget = if speed {
            (program_bytes / 2).max(SPEED_BUDGET_FLOOR)
        } else {
            (program_bytes / PROGRAM_BUDGET_DENOMINATOR).max(BUDGET_FLOOR)
        };
        Self {
            speed,
            budget,
            growth: 0,
        }
    }

    fn caller_limit(&self, entry_bytes: u64) -> u64 {
        if self.speed {
            entry_bytes
                .saturating_mul(2)
                .max(4_096)
                .min(entry_bytes.saturating_add(8_192).max(SPEED_CALLER_CAP))
        } else {
            entry_bytes.saturating_mul(2)
        }
    }

    fn evaluate_site(&self, input: &SiteCostInput<'_>) -> Result<AcceptedCost, InlineDecline> {
        let delta = input.after as i64 - input.bytes as i64;
        let net = delta.saturating_sub(input.removed.min(i64::MAX as u64) as i64);
        let tiny_accessor_slack = if matches!(input.criterion, Criterion::SmallCallee)
            && input.callee_bytes <= 36
            && net > 0
            && net <= 2
        {
            net
        } else {
            0
        };
        let trivial_leaf_slack = if !input.site.host_boundary
            && input.trivial_leaf
            && matches!(
                input.criterion,
                Criterion::SmallCallee | Criterion::SoleCaller
            )
            && net > 0
            && net <= 16
        {
            net
        } else {
            0
        };
        let charge = if self.speed {
            net.max(0)
        } else if net <= 0 || trivial_leaf_slack > 0 {
            0
        } else if tiny_accessor_slack > 0 {
            tiny_accessor_slack
        } else {
            net.max(MIN_SITE_CHARGE)
        };
        if debug_func_filter().is_some_and(|filter| input.caller.as_str().contains(filter)) {
            eprintln!(
                "inline {:?} <- {:?}: {:?}, before={}, after={}, callee={}, delta={delta}, removed={}, net={net}, limit={}, growth={}, charge={charge}, budget={}",
                input.caller,
                input.site.callee,
                input.criterion,
                input.bytes,
                input.after,
                input.callee_bytes,
                input.removed,
                input.limit,
                self.growth,
                self.budget,
            );
        }
        // Only loop virtualization (or <=2B tiny accessors, or <=16B trivial pure leaves) may spend
        // temporary net growth under size/balanced profiles. Speed-first mode permits bounded
        // growth for every qualifying criterion.
        let growth_allowed = self.speed
            || matches!(input.criterion, Criterion::LoopVirtualizable)
            || tiny_accessor_slack > 0
            || trivial_leaf_slack > 0;
        // A small loop helper may expose enough scalar replacement to repay a modest
        // pre-cleanup growth in the later per-item pipeline. Bound both the duplicated body
        // and that temporary growth; this is the iterator-state case, not a general license
        // to spend the whole-program compression budget.
        let bounded_loop_virtualization = matches!(input.criterion, Criterion::LoopVirtualizable)
            && input.iterator_step
            && input.callee_bytes <= 560
            && input.after <= input.bytes.saturating_add(360);
        // Under speed-first mode, never let earlier cold callees starve tiny leaf helpers
        // called inside loops (e.g., `Word::add` inside SHA-512's 80-round compression loop,
        // `Range::into_iter`, `Ord::min`, `Vec::len`, `Vec::index`, `compare$closure_1`).
        let callee_is_leaf = callee_is_inline_leaf(input.callee_func);
        let callee_total_ops: usize = input.callee_func.blocks.iter().map(|b| b.ops.len()).sum();
        let speed_tiny_exempt = self.speed
            && (input.trivial_leaf
                || (callee_is_leaf
                    && (input.callee_bytes <= 120
                        || (input.callee_func.blocks.len() <= 2
                            && callee_total_ops <= 8
                            && input.callee_bytes <= 256))));
        let speed_hot_loop = self.speed
            && input.in_loop
            && callee_is_leaf
            && input.callee_bytes <= SPEED_LOOP_CALLEE_FOOTPRINT;
        let speed_tiny_leaf = speed_tiny_exempt
            || (self.speed
                && callee_is_leaf
                && (input.callee_bytes <= 180
                    || ((input.callee_func.sig.abi.inline_hint
                        || input.callee_func.sig.abi.inline_always
                        || input.site.has_track_caller)
                        && input.callee_bytes <= 384)));
        const SPEED_SOLE_RECURSIVE_FOOTPRINT: u64 = 2_560;
        let speed_sole_loop = self.speed
            && input.is_sole
            && ((input.in_loop && input.callee_bytes <= SPEED_LOOP_CALLEE_FOOTPRINT)
                || (input.caller_recursive
                    && input.callee_bytes <= SPEED_SOLE_RECURSIVE_FOOTPRINT));
        let speed_budget_free = speed_hot_loop || speed_tiny_leaf || speed_sole_loop;
        // Inlining a `#[track_caller]` callee substitutes a `{filename,line,col}` literal
        // (~70-96 B pre-hoist) before `hoist_consts` shrinks it, and inlining a sole-caller,
        // loop-virtualizable, or tiny leaf helper into a large caller may add temporary
        // local-renaming / `try/catch` scaffolding before downstream cleanup.
        // Account for that pre-cleanup slack under `ReleaseSpeed`.
        let effective_callee_bytes = if speed_sole_loop {
            input.callee_bytes.saturating_add(256)
        } else if self.speed
            && (input.site.has_track_caller
                || input.is_sole
                || speed_tiny_leaf
                || matches!(input.criterion, Criterion::LoopVirtualizable))
        {
            input.callee_bytes.saturating_add(96)
        } else {
            input.callee_bytes
        };
        let size_limit = if speed_tiny_exempt {
            input
                .limit
                .max(input.entry_bytes.saturating_add(8_192))
                .max(SPEED_CALLER_CAP)
                .max(input.bytes.saturating_add(effective_callee_bytes))
        } else if speed_budget_free {
            input
                .limit
                .max(input.entry_bytes.saturating_add(8_192))
                .max(SPEED_CALLER_CAP)
        } else if input.is_sole && net <= 0 {
            input
                .limit
                .max(if self.speed { SPEED_CALLER_CAP } else { 16_384 })
        } else if self.speed && input.is_sole && input.callee_bytes <= SPEED_LOOP_CALLEE_FOOTPRINT {
            input.limit.max(SPEED_CALLER_CAP)
        } else {
            input.limit
        };
        if input.after > size_limit
            || input.after > input.bytes.saturating_add(effective_callee_bytes)
            || (!bounded_loop_virtualization
                && !speed_budget_free
                && self.growth + charge > self.budget as i64)
            || (!growth_allowed && net > 0)
        {
            return Err(InlineDecline::CostRejected);
        }
        Ok(AcceptedCost {
            charge,
            charge_growth: !bounded_loop_virtualization && !speed_tiny_leaf,
        })
    }

    fn commit(&mut self, accepted: AcceptedCost) {
        if accepted.charge_growth {
            self.growth += accepted.charge;
        }
    }
}

/// Read-only view of the function and metadata tables shared across site collection and splicing.
#[derive(Clone, Copy)]
struct SiteLookup<'a, 'm> {
    funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
    metadata: &'a BTreeMap<JsName, Metadata<'m>>,
}

/// ABI facts of one call site passed to `body_decline` and `prepare_callee_for_site`.
#[derive(Clone, Copy)]
struct CallSiteAbi {
    args_len: usize,
    caller_caller_location: bool,
    track_caller: Option<SourceFactId>,
}

/// Mutable state for the caller currently being processed.
struct CallerState<'a, 'm> {
    caller: &'a JsName,
    current: Func,
    metadata: Metadata<'m>,
    entry_bytes: u64,
    limit: u64,
    bytes: u64,
    looping: Vec<bool>,
    applied_here: u32,
    accepted: Option<Func>,
}

/// Verified result of atomically inlining every remaining call site to a sole-caller callee.
struct InlinedBatch<'m> {
    candidate: Func,
    metadata: Metadata<'m>,
    after_bytes: u64,
    sites_applied: u32,
    cross_object: u32,
}

/// Shared execution context across callers for one inliner pass.
struct InlineCx<'a> {
    config: InlineConfig<'a>,
    sizes: BTreeMap<JsName, u64>,
    cost: CostModel,
    /// Mutually-recursive sole-caller helpers already spliced into their sole caller (now dead).
    inlined_mutual_callees: BTreeSet<JsName>,
    stats: InlineStats,
}

impl<'a> InlineCx<'a> {
    fn new(config: InlineConfig<'a>) -> Self {
        // Run aggressive speed-first inlining only in the post-representation O pass so pre-repr
        // carrier analysis (`NativeVec`/`NativeMap`/`HostString`) always sees clean, un-poisoned
        // standard-library boundaries.
        let speed = config.allow_std_post_repr
            && matches!(config.profile, Some(crate::opts::Profile::ReleaseSpeed));
        let sizes = config.sizes.clone();
        let cost = CostModel::new(speed, config.program_bytes);
        Self {
            config,
            sizes,
            cost,
            inlined_mutual_callees: BTreeSet::new(),
            stats: InlineStats::default(),
        }
    }

    fn run<'m>(
        mut self,
        funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
        metadata: &mut BTreeMap<JsName, Metadata<'m>>,
    ) -> InlineStats {
        let order: Vec<JsName> = self.config.graph.sccs.iter().flatten().cloned().collect();
        for caller in &order {
            if self.inlined_mutual_callees.contains(caller) {
                continue;
            }
            let Some(current_metadata) = metadata.get(caller).cloned() else {
                continue;
            };
            let Some(Ok(func)) = funcs.get(caller) else {
                continue;
            };
            let lookup = SiteLookup { funcs, metadata };
            let sites = self.collect_sites(caller, func, &current_metadata, lookup, true);
            if sites.is_empty() {
                continue;
            }
            let entry_bytes = match self.sizes.get(caller).copied() {
                Some(bytes) => bytes,
                None => match (self.config.measure)(caller, func, &current_metadata) {
                    Some((bytes, _)) => bytes,
                    None => continue,
                },
            };
            let current = func.clone();
            let looping = Cfg::of(&current).looping_blocks();
            let mut state = CallerState {
                caller,
                current,
                metadata: current_metadata,
                entry_bytes,
                limit: self.cost.caller_limit(entry_bytes),
                bytes: entry_bytes,
                looping,
                applied_here: 0,
                accepted: None,
            };
            // Back to front within each block, so an earlier splice never moves a later site in
            // the same block (splicing only splits `site.block` and appends new blocks). Under
            // speed-first mode, prioritize sites inside loops before cold pre/post-loop sites so
            // cold callees cannot consume the caller size cap before inner-loop leaf helpers (e.g.
            // SHA-512's `Word::{ror,add}` inside the 80-round compression loop) get inlined for
            // PEA.
            let mut site_order: Vec<usize> = (0..sites.len()).rev().collect();
            if self.cost.speed {
                site_order.sort_by_key(|&idx| {
                    let s = &sites[idx];
                    (
                        !state.looping.get(s.block).copied().unwrap_or(false),
                        std::cmp::Reverse(s.block),
                        std::cmp::Reverse(s.op),
                    )
                });
            }
            for site_idx in site_order {
                self.try_inline_site(&mut state, &sites[site_idx], lookup);
            }

            // Multi-site sole-caller batching: when a callee has K > 1 sites in its sole referrer
            // and intermediate sites left `removed == 0` with `net > 0`, try splicing all
            // remaining sites to that callee as one atomic transaction.
            let mut sole_site_counts: BTreeMap<JsName, usize> = BTreeMap::new();
            for site in &sites {
                if !site.host_boundary && (self.config.sole_referrer)(&site.callee, caller) {
                    *sole_site_counts.entry(site.callee.clone()).or_default() += 1;
                }
            }
            let batch_callees: Vec<JsName> = sole_site_counts
                .into_iter()
                .filter_map(|(callee, count)| (count > 1).then_some(callee))
                .collect();
            if !batch_callees.is_empty() {
                if let Some(clean) = state.accepted.as_ref() {
                    state.current = clean.clone();
                }
                for callee in batch_callees {
                    let remaining: Vec<Site> = self
                        .collect_sites(state.caller, &state.current, &state.metadata, lookup, false)
                        .into_iter()
                        .filter(|site| site.callee == callee)
                        .collect();
                    if remaining.is_empty()
                        || remaining.iter().any(|site| site.host_boundary)
                        || count_item_refs(&state.current, &state.metadata.interns, callee.as_str())
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
                    match self.try_inline_sole_caller_batch(
                        &state,
                        &callee,
                        &remaining,
                        callee_func,
                        provider,
                    ) {
                        Ok(batch) => {
                            self.stats.witnesses_recorded += 1;
                            state.bytes = batch.after_bytes;
                            state.applied_here += batch.sites_applied;
                            self.stats.cross_object_applied += batch.cross_object;
                            state.metadata = batch.metadata;
                            state.current = batch.candidate.clone();
                            state.accepted = Some(batch.candidate);
                        }
                        Err(reason) => {
                            if reason == InlineDecline::CostRejected {
                                self.stats.declined_budget += 1;
                            }
                            self.stats.decline(reason);
                        }
                    }
                }
            }

            if let Some(current) = state.accepted {
                self.stats.applied += state.applied_here;
                self.stats.bytes_before += state.entry_bytes;
                self.stats.bytes_after += state.bytes;
                self.stats.items.insert(caller.clone());
                self.sizes.insert(caller.clone(), state.bytes);
                funcs.insert(caller.clone(), Ok(current));
                metadata.insert(caller.clone(), state.metadata);
            }
        }
        self.stats
    }

    /// Evaluates, splices, cleans, measures, and cost-checks one candidate call site.
    ///
    /// Leaves `state` untouched on any decline so no manual rollback is needed.
    fn try_inline_site<'m>(
        &mut self,
        state: &mut CallerState<'_, 'm>,
        site: &Site,
        lookup: SiteLookup<'_, 'm>,
    ) {
        let Some(criterion) = self.qualifies(state, site, lookup.funcs) else {
            self.stats.decline(InlineDecline::CostNoOpportunity);
            return;
        };
        let is_sole = (self.config.sole_referrer)(&site.callee, state.caller);
        if site.host_boundary
            && count_item_refs(
                &state.current,
                &state.metadata.interns,
                site.callee.as_str(),
            ) != 1
        {
            self.stats.decline(InlineDecline::HostBoundary);
            return;
        }
        let Some(Ok(callee_func)) = lookup.funcs.get(&site.callee) else {
            return;
        };
        let callee_bytes = self.sizes.get(&site.callee).copied().unwrap_or(u64::MAX);
        let trivial_leaf = trivial_pure_leaf(callee_func);
        let iterator_step = lookup
            .metadata
            .get(&site.callee)
            .is_some_and(|meta| bounded_iterator_step(callee_func, &meta.interns.types));
        let Some(provider) = lookup.metadata.get(&site.callee) else {
            return;
        };
        let origin = state.current.blocks[site.block].ops[site.op].origin;
        let (site_args_len, site_track_caller) =
            match &state.current.blocks[site.block].ops[site.op].kind {
                OpKind::Call {
                    args, track_caller, ..
                } => (args.len(), *track_caller),
                _ => return,
            };
        let site_abi = CallSiteAbi {
            args_len: site_args_len,
            caller_caller_location: state.current.sig.abi.caller_location,
            track_caller: site_track_caller,
        };
        let (callee_func, imported, sentinel_assert_origin) = match self.prepare_callee_for_site(
            &state.metadata,
            provider,
            callee_func,
            origin,
            site_abi,
        ) {
            Ok(value) => value,
            Err(reason) => {
                self.stats.decline(reason);
                return;
            }
        };
        let table = &imported.interns.types;
        let mut spliced = state.current.clone();
        splice(&mut spliced, site.block, site.op, &callee_func);
        if spliced
            .verify_carriers(table, observed_carriers(&spliced))
            .is_err()
        {
            self.stats.decline(InlineDecline::SpliceInvalid);
            return;
        }
        // Clean a clone so the remaining collected sites keep their block/op positions.
        // Price and retain the same verified shape, including merges, cell unboxing, and DCE cleanup.
        let mut candidate = spliced.clone();
        if let Err(reason) = clean_candidate(&mut candidate, table, sentinel_assert_origin) {
            self.stats.decline(reason);
            return;
        }
        let Some((after, refs)) = (self.config.measure)(state.caller, &candidate, &imported) else {
            self.stats.decline(InlineDecline::CostUnavailable);
            return;
        };
        if site.host_boundary && refs.contains(&site.callee) {
            self.stats.decline(InlineDecline::HostBoundary);
            return;
        }
        // The sole-referrer oracle excludes public and explicitly rooted items. Credit deletion
        // only after the emitted caller also lost every reference to the callee, including
        // function-pointer constants and retained indirect uses in this caller.
        let removed = if is_sole && !refs.contains(&site.callee) {
            self.sizes.get(&site.callee).copied().unwrap_or_default()
        } else {
            0
        };
        let accepted_cost = match self.cost.evaluate_site(&SiteCostInput {
            caller: state.caller,
            site,
            criterion,
            is_sole,
            caller_recursive: self.config.graph.recursive(state.caller),
            in_loop: state.looping.get(site.block).copied().unwrap_or(false),
            entry_bytes: state.entry_bytes,
            limit: state.limit,
            bytes: state.bytes,
            after,
            callee_bytes,
            removed,
            trivial_leaf,
            iterator_step,
            callee_func: &callee_func,
        }) {
            Ok(cost) => cost,
            Err(reason) => {
                // `cost.budget`: the whole site is reverted, never a partial inline.
                self.stats.declined_budget += 1;
                self.stats.decline(reason);
                return;
            }
        };
        self.stats.witnesses_recorded += 1;
        self.cost.commit(accepted_cost);
        if self.config.graph.recursive(&site.callee) {
            self.inlined_mutual_callees.insert(site.callee.clone());
        }
        self.stats.cross_object_applied +=
            u32::from(!std::ptr::eq(state.metadata.universe, provider.universe));
        state.current = spliced;
        state.metadata = imported;
        state.bytes = after;
        state.applied_here += 1;
        state.accepted = Some(candidate);
    }

    /// Atomically splices all `remaining` sites of sole-caller `callee` into a cloned trial copy
    /// of `state`, returning [`InlinedBatch`] only if verification, cleanup, and cost all pass.
    fn try_inline_sole_caller_batch<'m>(
        &self,
        state: &CallerState<'_, 'm>,
        callee: &JsName,
        remaining: &[Site],
        callee_func: &Func,
        provider: &Metadata<'m>,
    ) -> Result<InlinedBatch<'m>, InlineDecline> {
        let callee_bytes = self.sizes.get(callee).copied().unwrap_or(u64::MAX);
        let mut trial_func = state.current.clone();
        let mut trial_meta = state.metadata.clone();
        let mut sentinel_origin: Option<OriginId> = None;

        for site in remaining.iter().rev() {
            let origin = trial_func.blocks[site.block].ops[site.op].origin;
            let (site_args_len, site_track_caller) =
                match &trial_func.blocks[site.block].ops[site.op].kind {
                    OpKind::Call {
                        args, track_caller, ..
                    } => (args.len(), *track_caller),
                    _ => return Err(InlineDecline::SpliceInvalid),
                };
            let site_abi = CallSiteAbi {
                args_len: site_args_len,
                caller_caller_location: trial_func.sig.abi.caller_location,
                track_caller: site_track_caller,
            };
            let (prepared, next_meta, site_sentinel) =
                self.prepare_callee_for_site(&trial_meta, provider, callee_func, origin, site_abi)?;
            if sentinel_origin.is_none() {
                sentinel_origin = site_sentinel;
            }
            splice(&mut trial_func, site.block, site.op, &prepared);
            if trial_func
                .verify_carriers(&next_meta.interns.types, observed_carriers(&trial_func))
                .is_err()
            {
                return Err(InlineDecline::SpliceInvalid);
            }
            trial_meta = next_meta;
        }

        let mut candidate = trial_func;
        clean_candidate(&mut candidate, &trial_meta.interns.types, sentinel_origin)?;
        let (after, refs) = (self.config.measure)(state.caller, &candidate, &trial_meta)
            .ok_or(InlineDecline::CostUnavailable)?;
        let delta = after as i64 - state.bytes as i64;
        let removed = if !refs.contains(callee) {
            callee_bytes
        } else {
            0
        };
        let net = delta.saturating_sub(removed.min(i64::MAX as u64) as i64);
        let size_limit = state.limit.max(16_384);
        if debug_func_filter().is_some_and(|filter| state.caller.as_str().contains(filter)) {
            eprintln!(
                "inline-batch {:?} <- {callee:?} ({} sites): before={}, after={after}, callee={callee_bytes}, delta={delta}, removed={removed}, net={net}, limit={size_limit}",
                state.caller,
                remaining.len(),
                state.bytes,
            );
        }
        if net > 0 || after > size_limit || after > state.bytes.saturating_add(callee_bytes) {
            return Err(InlineDecline::CostRejected);
        }
        let cross_object = (remaining.len() as u32)
            * u32::from(!std::ptr::eq(state.metadata.universe, provider.universe));
        Ok(InlinedBatch {
            candidate,
            metadata: trial_meta,
            after_bytes: after,
            sites_applied: remaining.len() as u32,
            cross_object,
        })
    }

    /// Prepares an imported callee for splicing at a call site, resolving any `#[track_caller]`
    /// location substitution, importing panic lang-item dependencies for non-optional `Assert` ops
    /// into the caller's summary when unambiguous, and tagging any still-unresolved callee `Assert`
    /// ops with a sentinel origin so any assert surviving DCE rejects inlining.
    fn prepare_callee_for_site<'m>(
        &self,
        current_metadata: &Metadata<'m>,
        provider: &Metadata<'m>,
        callee_func: &Func,
        origin: OriginId,
        site_abi: CallSiteAbi,
    ) -> Result<(Func, Metadata<'m>, Option<OriginId>), InlineDecline> {
        let (mut callee_func, mut imported) =
            current_metadata.import(provider, callee_func, origin)?;
        let mut sentinel_assert_origin = None;
        if callee_func.sig.abi.caller_location && callee_func.params.len() == site_abi.args_len + 1
        {
            for block in &mut callee_func.blocks {
                for op in &mut block.ops {
                    if let OpKind::Assert { msg, .. } = &op.kind {
                        if is_optional_assert(&op.kind) {
                            continue;
                        }
                        if !try_import_assert_panic_dep(&mut imported, provider, msg) {
                            op.origin = SENTINEL_ASSERT_ORIGIN;
                            sentinel_assert_origin = Some(SENTINEL_ASSERT_ORIGIN);
                        }
                    }
                }
            }
            if let Some(site_fact) = site_abi.track_caller.filter(|f| !f.is_sentinel()) {
                rewrite_forwarded_track_caller(
                    &mut callee_func,
                    site_fact,
                    self.config.items,
                    &imported.interns,
                );
                if !unused_caller_location(&callee_func) {
                    bind_caller_location_param(
                        &mut callee_func,
                        site_fact,
                        &imported.interns,
                        origin,
                    )?;
                }
            }
            strip_unused_last_param(&mut callee_func);
        } else {
            for block in &callee_func.blocks {
                for op in &block.ops {
                    if let OpKind::Assert { msg, .. } = &op.kind {
                        if !is_optional_assert(&op.kind) {
                            let _ = try_import_assert_panic_dep(&mut imported, provider, msg);
                        }
                    }
                }
            }
        }
        Ok((callee_func, imported, sentinel_assert_origin))
    }

    /// The criterion one located site meets, if any (the checks that need the graph and interns
    /// were made when the site was collected). The loop shape is checked first: it is the one that
    /// may grow, so a site meeting it and a size criterion keeps the wider allowance.
    fn qualifies(
        &self,
        state: &CallerState<'_, '_>,
        site: &Site,
        funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    ) -> Option<Criterion> {
        let caller = state.caller;
        let callee = &site.callee;
        let func = &state.current;
        let table = &state.metadata.interns.types;
        let looping = &state.looping;
        let speed = self.cost.speed;
        if site.host_boundary {
            return (self.config.sole_referrer)(callee, caller).then_some(Criterion::SoleCaller);
        }
        let callee_bytes = self.sizes.get(callee).copied().unwrap_or(u64::MAX);
        if speed {
            // Check loop sites BEFORE `#[inline]` hints so hot-loop helpers (e.g. `Ord::min`,
            // `Vec::len`, `Vec::index`, `Range::into_iter`, `compare$closure_1`) receive the wider
            // `LoopVirtualizable` budget and slack instead of being shadowed by `InlineHint`.
            if looping.get(site.block).copied().unwrap_or(false)
                && callee_bytes <= SPEED_LOOP_CALLEE_FOOTPRINT
            {
                return Some(Criterion::LoopVirtualizable);
            }
            if let Some(Ok(callee_func)) = funcs.get(callee) {
                if (callee_func.sig.abi.inline_always
                    && callee_bytes <= SPEED_INLINE_ALWAYS_FOOTPRINT)
                    || (callee_func.sig.abi.inline_hint
                        && callee_bytes <= SPEED_INLINE_HINT_FOOTPRINT)
                {
                    return Some(Criterion::InlineHint);
                }
            }
        }
        let loop_shape = || -> bool {
            if !looping.get(site.block).copied().unwrap_or(false) {
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
        if (self.config.sole_referrer)(callee, caller) {
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
    fn collect_sites(
        &mut self,
        caller: &JsName,
        func: &Func,
        caller_metadata: &Metadata<'_>,
        lookup: SiteLookup<'_, '_>,
        record_declines: bool,
    ) -> Vec<Site> {
        let cx = CollectSiteCx {
            caller,
            func,
            caller_metadata,
            lookup,
            caller_retargets: self.config.call_retargets.and_then(|map| map.get(caller)),
            direct_site_counts: if self.cost.speed {
                count_direct_sites(func, &caller_metadata.interns)
            } else {
                BTreeMap::new()
            },
        };
        let mut sites = Vec::new();
        for (block_index, block) in func.blocks.iter().enumerate() {
            for op_index in 0..block.ops.len() {
                match self.evaluate_call_site(&cx, block_index, op_index) {
                    Some(Ok(site)) => sites.push(site),
                    Some(Err(reason)) if record_declines => self.stats.decline(reason),
                    Some(Err(_)) | None => {}
                }
            }
        }
        sites
    }

    /// Evaluates one op in `cx.func` as a candidate inline call site.
    ///
    /// Returns `None` for non-direct-call ops (or unresolved link names) that do not record an
    /// inline decline, `Some(Err(reason))` for declined direct call sites, and `Some(Ok(site))`
    /// for accepted candidates.
    fn evaluate_call_site(
        &self,
        cx: &CollectSiteCx<'_, '_>,
        block_index: usize,
        op_index: usize,
    ) -> Option<Result<Site, InlineDecline>> {
        let speed = self.cost.speed;
        let block = &cx.func.blocks[block_index];
        let op = &block.ops[op_index];
        let OpKind::Call {
            callee: Callee::Direct { item, link_name },
            function: None,
            args,
            track_caller,
            semantic,
            host,
        } = &op.kind
        else {
            return None;
        };
        if cx
            .caller_retargets
            .is_some_and(|retargets| retargets.contains_key(item))
        {
            return Some(Err(InlineDecline::ReprRetarget));
        }
        let is_invoke =
            matches!(block.term, RTerm::Invoke { .. }) && op_index + 1 == block.ops.len();
        if is_invoke {
            let unwind_uses_result = matches!(
                &block.term,
                RTerm::Invoke { unwind, .. }
                    if op.result.is_some_and(|r| unwind.1.contains(&r))
            );
            if !speed || unwind_uses_result {
                return Some(Err(InlineDecline::ExceptionalControlFlow));
            }
        }
        if !self.call_semantic_allowed(cx.func, cx.caller_metadata, semantic, args)
            || host.is_some()
        {
            return Some(Err(InlineDecline::SemanticBoundary));
        }
        let name = cx.caller_metadata.interns.str(*link_name)?;
        if self
            .config
            .call_retargets
            .and_then(|map| map.get(name))
            .is_some_and(|retargets| !retargets.is_empty())
        {
            return Some(Err(InlineDecline::ReprRetarget));
        }
        if name == cx.caller.as_str() {
            return Some(Err(InlineDecline::RecursiveBudget));
        }
        let callee = JsName::new(name.to_owned());
        if self.config.graph.recursive(&callee)
            && !self.can_inline_recursive_callee(cx, &callee, name)
        {
            return Some(Err(InlineDecline::RecursiveBudget));
        }
        // An item with recorded zombies owes the link step its compile-time diagnostic chain;
        // splicing its body would drop the item and silence the errors.
        if !(self.config.zombie_free)(&callee) {
            return Some(Err(InlineDecline::Zombie));
        }
        let Some(callee_metadata) = cx.lookup.metadata.get(&callee) else {
            return Some(Err(InlineDecline::MetadataUnavailable));
        };
        if !cx.caller_metadata.compatible(callee_metadata) {
            return Some(Err(InlineDecline::ObjectIncompatible));
        }
        let Some(Ok(callee_func)) = cx.lookup.funcs.get(&callee) else {
            return Some(Err(InlineDecline::BodyUnavailable));
        };
        if is_invoke && !callee_invoke_safe(callee_func, &callee_metadata.interns) {
            return Some(Err(InlineDecline::ExceptionalControlFlow));
        }
        if cx
            .caller_retargets
            .is_some_and(|retargets| callee_retargets_collide(callee_func, retargets))
        {
            return Some(Err(InlineDecline::ReprRetarget));
        }
        if callee_func.sig.abi.inline_never {
            return Some(Err(InlineDecline::Never));
        }
        if self.is_protected_callee(&callee, callee_func, callee_metadata) {
            return Some(Err(InlineDecline::SemanticBoundary));
        }
        let is_sole_caller = (self.config.sole_referrer)(&callee, cx.caller);
        let allow_host_boundary = is_sole_caller
            && count_item_refs(cx.func, &cx.caller_metadata.interns, callee.as_str()) == 1;
        let site_abi = CallSiteAbi {
            args_len: args.len(),
            caller_caller_location: cx.func.sig.abi.caller_location,
            track_caller: *track_caller,
        };
        if let Some(reason) = Self::body_decline(
            &self.config,
            speed,
            callee_func,
            &callee_metadata.interns,
            &cx.caller_metadata.interns,
            site_abi,
            allow_host_boundary,
        ) {
            return Some(Err(reason));
        }
        if has_non_baseline_carrier(cx.func, callee_func, args, op.result) {
            return Some(Err(InlineDecline::CarrierConversionUnsupported));
        }
        Some(Ok(Site {
            block: block_index,
            op: op_index,
            callee,
            has_track_caller: track_caller.is_some_and(|f| !f.is_not_tracked())
                || (callee_func.sig.abi.caller_location && unused_caller_location(callee_func)),
            host_boundary: callee_has_host_boundary(callee_func, &callee_metadata.interns, speed),
        }))
    }

    /// Whether a direct call's [`CallSemantic`] is eligible for inlining in this phase.
    fn call_semantic_allowed(
        &self,
        func: &Func,
        caller_metadata: &Metadata<'_>,
        semantic: &CallSemantic,
        args: &[ValueId],
    ) -> bool {
        let speed = self.cost.speed;
        match semantic {
            CallSemantic::Ordinary => true,
            CallSemantic::RawParts if self.config.allow_std_post_repr && speed => true,
            // A `str` identity (`js-str=bytes` only) stays a call: `lower/js/call.rs` spells it as
            // the string operation it is, which inlining its body would take away.
            CallSemantic::Standard(std_op)
                if std_op.is_str()
                    || std_op.is_typed_heap()
                    || std_op.is_native_string()
                    || std_op.is_native_vec()
                    || std_op.is_fast_str_op() =>
            {
                false
            }
            CallSemantic::Standard(std_op) if self.config.allow_std_post_repr => {
                !crate::lower::semantic_ops::fmt_kernel::is_format_machinery(*std_op)
                    && !matches!(
                        std_op,
                        crate::jir::semantic::StandardOperation::F64FromStr
                            | crate::jir::semantic::StandardOperation::F32FromStr
                            | crate::jir::semantic::StandardOperation::SliceSort
                            | crate::jir::semantic::StandardOperation::SliceSortCmp
                    )
                    && !(crate::lower::bstr::enabled()
                        && matches!(
                            std_op,
                            crate::jir::semantic::StandardOperation::StringAsStr
                                | crate::jir::semantic::StandardOperation::StrToLowercase
                                | crate::jir::semantic::StandardOperation::StrToUppercase
                        ))
                    && !(speed
                        && *std_op == crate::jir::semantic::StandardOperation::VecResize
                        && args.first().is_some_and(|&recv| {
                            is_ta_vec_resize_receiver(
                                &caller_metadata.interns.types,
                                func.values[recv.0 as usize].ty,
                            )
                        }))
            }
            _ => false,
        }
    }

    /// Under `ReleaseSpeed`, allows inlining a single-callsite sole-caller mutually recursive
    /// helper when `callee` is not directly self-recursive. Splicing eliminates `callee`, converts
    /// its back-edge into direct self-recursion on `caller`, and prevents downstream minifiers
    /// from turning `callee` into a hot closure IIFE.
    fn can_inline_recursive_callee(
        &self,
        cx: &CollectSiteCx<'_, '_>,
        callee: &JsName,
        name: &str,
    ) -> bool {
        self.cost.speed
            && (self.config.sole_referrer)(callee, cx.caller)
            && !self.inlined_mutual_callees.contains(callee)
            && cx.direct_site_counts.get(name).copied().unwrap_or(0) == 1
            && cx
                .lookup
                .funcs
                .get(callee)
                .and_then(|r| r.as_ref().ok())
                .is_some_and(|cf| {
                    cx.lookup
                        .metadata
                        .get(callee)
                        .is_some_and(|cm| !directly_calls_self(cf, &cm.interns, callee))
                })
    }

    /// Whether `callee` is a protected container drop glue or standard operation whose identity
    /// must remain intact as a direct call boundary.
    fn is_protected_callee(
        &self,
        callee: &JsName,
        callee_func: &Func,
        callee_metadata: &Metadata<'_>,
    ) -> bool {
        let speed = self.cost.speed;
        let is_nonempty_drop_glue = speed
            && callee.as_str().starts_with("core$ptr$drop_glue$")
            && callee_func.sig.arguments.len() == 1
            && !(callee_func.blocks.len() == 1
                && callee_func.blocks[0].ops.is_empty()
                && matches!(
                    callee_func.blocks[0].term,
                    rcgjs_object::jir::repr::RTerm::Return(None)
                ));
        let is_protected_drop_glue = is_nonempty_drop_glue
            && callee_func
                .sig
                .arguments
                .first()
                .and_then(|&arg_ty| callee_metadata.interns.types.pointee(arg_ty))
                .and_then(|pt| callee_metadata.interns.types.get(pt))
                .is_some_and(|d| {
                    (crate::lower::bstr::enabled()
                        && d.meta.candidate
                            == rcgjs_object::jir::types::RepresentationCandidate::String)
                        || (d.meta.candidate
                            == rcgjs_object::jir::types::RepresentationCandidate::Vec
                            && d.meta.allocator
                                == rcgjs_object::jir::types::AllocatorIdentity::Global)
                });
        is_protected_drop_glue
            || self
                .config
                .items
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
                        || op.is_typed_heap()
                        || op.is_native_string()
                        || op.is_native_vec()
                        || op.is_fast_str_op()
                        || (crate::lower::bstr::enabled()
                            && matches!(
                                op,
                                crate::jir::semantic::StandardOperation::StrToLowercase
                                    | crate::jir::semantic::StandardOperation::StrToUppercase
                            ))
                        || (speed
                            && op == crate::jir::semantic::StandardOperation::VecResize
                            && callee_func.sig.arguments.first().is_some_and(|&arg_ty| {
                                is_ta_vec_resize_receiver(&callee_metadata.interns.types, arg_ty)
                            }))
                })
    }

    /// The semantic boundary preventing a callee body from being spliced at this site.
    fn body_decline(
        config: &InlineConfig<'_>,
        speed: bool,
        callee: &Func,
        interns: &Interns,
        caller_interns: &Interns,
        site_abi: CallSiteAbi,
        allow_host_boundary: bool,
    ) -> Option<InlineDecline> {
        let items = config.items;
        let args = site_abi.args_len;
        let caller_caller_location = site_abi.caller_caller_location;
        let site_track_caller = site_abi.track_caller;

        let site_concrete = site_track_caller.filter(|f| !f.is_sentinel());
        let site_forwards = site_track_caller.is_some_and(SourceFactId::is_forward)
            || (caller_caller_location && site_track_caller.is_none());
        let site_is_not_tracked = site_track_caller.is_some_and(SourceFactId::is_not_tracked);

        if callee.has_invoke() && !(speed && callee_has_only_noop_invokes(callee)) {
            return Some(InlineDecline::ExceptionalControlFlow);
        }
        let expected_params = if callee.sig.abi.caller_location {
            if site_is_not_tracked {
                return Some(InlineDecline::TrackCallerUnavailable);
            }
            let can_bind_used_loc = site_concrete.is_some_and(|fact| {
                caller_interns.source_fact(fact).is_some()
                    && callee
                        .params
                        .last()
                        .and_then(|p| {
                            location_param_fields(&interns.types, callee.values[p.0 as usize].ty)
                        })
                        .is_some()
            });
            if callee.params.len() == args + 1
                && (unused_caller_location(callee) || can_bind_used_loc)
            {
                args + 1
            } else {
                return Some(InlineDecline::TrackCallerUnavailable);
            }
        } else if site_track_caller.is_some_and(|f| !f.is_not_tracked()) {
            return Some(InlineDecline::TrackCallerUnavailable);
        } else {
            args
        };
        if (!speed && callee.sig.abi.spread.is_some())
            || callee.params.len() != expected_params
            || callee
                .blocks
                .first()
                .is_none_or(|block| block.params != callee.params)
        {
            return Some(InlineDecline::AbiUnsupported);
        }
        let forwards_caller_location = site_forwards;
        callee
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .find_map(|op| match &op.kind {
                // Resume assertions identify coroutine bodies until JIR-R carries a dedicated flag.
                OpKind::Assert {
                    msg:
                        RAssertMessage::ResumedAfterReturn(_)
                        | RAssertMessage::ResumedAfterPanic(_)
                        | RAssertMessage::ResumedAfterDrop(_),
                    ..
                } => Some(InlineDecline::CoroutineUnsupported),
                OpKind::Assert { .. }
                    if callee.sig.abi.caller_location
                        && !forwards_caller_location
                        && site_concrete.is_none() =>
                {
                    Some(InlineDecline::TrackCallerUnavailable)
                }
                // The original item owes the linker its compile-time required-by diagnostic.
                OpKind::Zombie(_) => Some(InlineDecline::Zombie),
                OpKind::Intrinsic {
                    name,
                    track_caller: inner_tc,
                    ..
                } if interns.str(*name) == Some("caller_location") => {
                    let ok = speed
                        && (inner_tc.is_some_and(|f| !f.is_sentinel())
                            || (callee.sig.abi.caller_location
                                && (site_concrete.is_some() || forwards_caller_location)));
                    (!ok).then_some(InlineDecline::TrackCallerUnavailable)
                }
                // Host and template lowering may own item-local bindings and observable ordering.
                _ if !allow_host_boundary && is_host_boundary_op(&op.kind, interns, speed) => {
                    Some(InlineDecline::HostBoundary)
                }
                // Do not inline bodies that call the raw global allocator directly: splicing a
                // stdlib allocator call into a non-stdlib user function triggers `byte_views`'s
                // conservative `direct_alloc` fallback and forces the ES6 Proxy byte-view shim
                // across the whole program.
                OpKind::Call {
                    callee: inner_callee,
                    ..
                } if is_direct_alloc_call(inner_callee, interns) => {
                    Some(InlineDecline::SemanticBoundary)
                }
                OpKind::Call {
                    track_caller: Some(inner_tc),
                    ..
                } if inner_tc.is_not_tracked() => None,
                OpKind::Call {
                    track_caller: Some(inner_tc),
                    ..
                } if callee.sig.abi.caller_location && inner_tc.is_forward() => {
                    (site_concrete.is_none() && !forwards_caller_location)
                        .then_some(InlineDecline::TrackCallerUnavailable)
                }
                OpKind::Call {
                    callee: inner_callee,
                    args: inner_args,
                    track_caller: None,
                    ..
                } if callee.sig.abi.caller_location => match inner_callee {
                    Callee::Direct { item, .. } => {
                        let Some(entry) = items.get(item) else {
                            return Some(InlineDecline::TrackCallerUnavailable);
                        };
                        if !entry.abi_known
                            || entry.params != inner_args.len() + usize::from(entry.caller_location)
                            || (entry.caller_location
                                && site_concrete.is_none()
                                && !forwards_caller_location)
                        {
                            Some(InlineDecline::TrackCallerUnavailable)
                        } else {
                            None
                        }
                    }
                    Callee::Foreign { .. } | Callee::Runtime { .. } | Callee::Indirect { .. } => {
                        None
                    }
                    _ => Some(InlineDecline::TrackCallerUnavailable),
                },
                _ => None,
            })
    }
}

/// Shared caller-level inputs for [`InlineCx::collect_sites`] and its helper checks.
struct CollectSiteCx<'a, 'm> {
    caller: &'a JsName,
    func: &'a Func,
    caller_metadata: &'a Metadata<'m>,
    lookup: SiteLookup<'a, 'm>,
    caller_retargets: Option<&'a crate::lower::CallRetargets>,
    direct_site_counts: BTreeMap<&'a str, usize>,
}

/// Counts direct non-host call sites per callee link name inside `func`.
fn count_direct_sites<'a>(func: &Func, interns: &'a Interns) -> BTreeMap<&'a str, usize> {
    let mut counts: BTreeMap<&'a str, usize> = BTreeMap::new();
    for block in &func.blocks {
        for op in &block.ops {
            if let OpKind::Call {
                callee: Callee::Direct { link_name, .. },
                function: None,
                host: None,
                ..
            } = &op.kind
            {
                if let Some(name) = interns.str(*link_name) {
                    *counts.entry(name).or_default() += 1;
                }
            }
        }
    }
    counts
}

/// Whether `callee_func` directly invokes or drops any item retargeted by its caller.
fn callee_retargets_collide(
    callee_func: &Func,
    caller_retargets: &crate::lower::CallRetargets,
) -> bool {
    callee_func.blocks.iter().any(|b| {
        b.ops.iter().any(|op| match &op.kind {
            OpKind::Call {
                callee: Callee::Direct { item, .. },
                host: None,
                ..
            }
            | OpKind::Drop {
                glue: DropGlue::Direct { item, .. },
                ..
            } => caller_retargets.contains_key(item),
            _ => false,
        })
    })
}

/// Whether the callee body or call-site arguments/result cross a committed non-baseline carrier.
fn has_non_baseline_carrier(
    func: &Func,
    callee_func: &Func,
    args: &[ValueId],
    result: Option<ValueId>,
) -> bool {
    let baseline = crate::jir::repr::Carrier::Baseline;
    callee_func.values.iter().any(|def| def.carrier != baseline)
        || args
            .iter()
            .copied()
            .chain(result)
            .any(|v| func.values[v.0 as usize].carrier != baseline)
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

/// Merges the panic lang-item dependency required by a callee `Assert` into the caller's
/// `ItemSummary::deps` when both sides have summaries and the lang item resolves unambiguously.
fn try_import_assert_panic_dep(
    imported: &mut Metadata<'_>,
    provider: &Metadata<'_>,
    msg: &RAssertMessage,
) -> bool {
    let lang_fn = crate::lower::js::emit::panic_function(msg);
    let (Some(provider_summary), Some(caller_summary)) =
        (provider.summary.as_deref(), imported.summary.as_deref())
    else {
        return false;
    };
    let mut provider_matches = provider_summary
        .deps
        .iter()
        .filter(|dep| crate::lower::js::emit::names_lang_fn(dep, lang_fn));
    let (Some(provider_dep), None) = (provider_matches.next(), provider_matches.next()) else {
        return false;
    };
    let caller_matches: Vec<&String> = caller_summary
        .deps
        .iter()
        .filter(|dep| crate::lower::js::emit::names_lang_fn(dep, lang_fn))
        .collect();
    match caller_matches.as_slice() {
        [existing] => *existing == provider_dep,
        [] => {
            let dep = provider_dep.clone();
            let deps = &mut imported
                .summary
                .as_mut()
                .expect("checked Some")
                .to_mut()
                .deps;
            if !deps.contains(&dep) {
                deps.push(dep);
                deps.sort();
            }
            true
        }
        _ => false,
    }
}

fn clean_candidate(
    candidate: &mut Func,
    table: &TypeTable,
    sentinel_assert_origin: Option<OriginId>,
) -> Result<(), InlineDecline> {
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
            return Err(InlineDecline::TrackCallerUnavailable);
        }
    }
    candidate
        .verify_carriers(table, observed_carriers(candidate))
        .map_err(|_| InlineDecline::CleanupInvalid)
}

/// Whether a `StandardOperation` is lowered inline at the call site without emitting a JS call.
fn is_inline_lowered_std_op(op: crate::jir::semantic::StandardOperation) -> bool {
    use crate::jir::semantic::StandardOperation as Std;
    op.is_native_vec()
        || op.is_native_string()
        || op.is_fast_str_op()
        || op.is_str()
        || matches!(
            op,
            Std::StringAsStr | Std::VecAsSlice | Std::VecAsMutSlice | Std::VecLen | Std::VecIsEmpty
        )
}

/// Whether a `StandardOperation` is both lowered inline at the call site and provably non-throwing.
fn is_pure_inline_lowered_std_op(op: crate::jir::semantic::StandardOperation) -> bool {
    use crate::jir::semantic::StandardOperation as Std;
    op.is_fast_str_op()
        || matches!(
            op,
            Std::StringAsStr
                | Std::NativeStrAsStr
                | Std::NativeStrNew
                | Std::NativeStrFromStr
                | Std::NativeStrLen
                | Std::NativeStrCapacity
                | Std::NativeStrClear
                | Std::NativeVecNew
                | Std::NativeVecCapacity
                | Std::NativeVecAsPtr
                | Std::NativeVecAsMutPtr
                | Std::NativeVecAsSlice
                | Std::NativeVecAsMutSlice
                | Std::VecAsSlice
                | Std::VecAsMutSlice
                | Std::VecLen
                | Std::VecIsEmpty
                | Std::StrLen
                | Std::StrIsCharBoundary
                | Std::StrEq
                | Std::StrCmp
                | Std::StrPartialCmp
                | Std::StrIsAscii
                | Std::SliceIsAscii
        )
}

/// Whether `callee_func` is a leaf body (no non-cold JS calls outside inline-lowered std ops, and
/// no non-noop `Invoke` terminators).
fn callee_is_inline_leaf(callee_func: &Func) -> bool {
    callee_func.blocks.iter().all(|b| {
        let cold_diverging = matches!(b.term, RTerm::Unreachable);
        let ops_leaf = cold_diverging
            || b.ops.iter().all(|op| {
                !matches!(op.kind, OpKind::Call { .. })
                    || matches!(
                        op.kind,
                        OpKind::Call {
                            semantic: CallSemantic::Standard(std_op),
                            ..
                        } if is_inline_lowered_std_op(std_op)
                    )
            });
        let term_leaf = match &b.term {
            RTerm::Invoke {
                unwind: (ub, _), ..
            } => is_noop_rethrow_block(callee_func, *ub),
            _ => true,
        };
        ops_leaf && term_leaf
    })
}

/// Whether `func` is a trivial single-block pure scalar/field wrapper (`<= 4` pure ops, no calls,
/// no invokes, no aggregates, no asserts) whose inlining reduces to inline JS operators or field reads.
fn trivial_pure_leaf(func: &Func) -> bool {
    if func.blocks.len() != 1 || func.has_invoke() {
        return false;
    }
    let block = &func.blocks[0];
    if block.ops.len() > 4 || !matches!(block.term, RTerm::Return(_)) {
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
                | OpKind::Deref(_)
                | OpKind::Field { .. }
                | OpKind::Drop {
                    glue: DropGlue::None,
                    ..
                }
        )
    })
}

/// Returns the number of fields if `ty` is `&Location` (`file: &str | NonNull<str>, line: u32, col: u32, ZST...`).
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
    let file_pointee = table.pointee(variant.fields[0].ty)?;
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
) -> Result<(), InlineDecline> {
    let Some(&last_param) = callee.params.last() else {
        return Err(InlineDecline::TrackCallerUnavailable);
    };
    let last_def = callee.values[last_param.0 as usize];
    let Some(field_count) = location_param_fields(&interns.types, last_def.ty) else {
        return Err(InlineDecline::TrackCallerUnavailable);
    };
    let Some(fact) = interns.source_fact(site_fact) else {
        return Err(InlineDecline::TrackCallerUnavailable);
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
    let const_val = callee.push_value(last_def.ty, last_def.carrier, last_def.origin);
    for block in &mut callee.blocks {
        for op in &mut block.ops {
            op.kind.visit_values_mut(|v| {
                if *v == last_param {
                    *v = const_val;
                }
            });
        }
        block.term.visit_values_mut(|v| {
            if *v == last_param {
                *v = const_val;
            }
        });
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

fn is_noop_rethrow_block(callee: &Func, mut blk_id: RBlockId) -> bool {
    for _ in 0..4 {
        let Some(ub) = callee.blocks.get(blk_id.0 as usize) else {
            return false;
        };
        let ops_noop = ub.ops.iter().all(|op| {
            matches!(
                op.kind,
                OpKind::Drop {
                    glue: crate::jir::semantic::DropGlue::None,
                    ..
                } | OpKind::BorrowView { .. }
                    | OpKind::RawView { .. }
                    | OpKind::Copy(_)
                    | OpKind::Move(_)
                    | OpKind::CellLoad(_)
                    | OpKind::Const(_)
                    | OpKind::Field { .. }
                    | OpKind::Cast { .. }
            )
        });
        if !ops_noop {
            return false;
        }
        match &ub.term {
            RTerm::Throw(None) => return true,
            RTerm::Goto(next, _) if *next != blk_id => blk_id = *next,
            _ => return false,
        }
    }
    false
}

/// True when every `Invoke`/`Throw` inside `callee` unwinds only through a no-op cleanup pad
/// (`DropGlue::None` + `Throw(None)`), so splicing can safely redirect unwind edges to the caller
/// (at an `Invoke` site) or strip them to `Goto(normal)` (at a non-`Invoke` site).
fn callee_has_only_noop_invokes(callee: &Func) -> bool {
    callee.blocks.iter().all(|b| match &b.term {
        RTerm::Throw(Some(_)) => false,
        RTerm::Throw(None) => b.id != RBlockId(0) && is_noop_rethrow_block(callee, b.id),
        RTerm::Invoke {
            unwind: (ub, _), ..
        } => is_noop_rethrow_block(callee, *ub),
        _ => true,
    })
}

/// Whether `callee` can be safely spliced at an `RTerm::Invoke` call site: no `Assert`, `Zombie`,
/// value-throwing `Throw(Some(_))`, or non-trivial `Drop`/`Host` outside cold diverging blocks;
/// at most one non-cold `Call` (so splicing 1 `Invoke` yields at most 1 non-cold `Invoke`, e.g.
/// `Option::get_or_insert_with` / `Option::map` / `Option::and_then`), and any inner `Invoke`
/// unwinds only through a no-op cleanup pad (`DropGlue::None` + `Throw(None)`) that can redirect
/// straight to the caller's unwind edge.
fn callee_invoke_safe(callee: &Func, interns: &Interns) -> bool {
    if !callee_has_only_noop_invokes(callee) {
        return false;
    }
    let mut non_cold_calls = 0usize;
    for b in &callee.blocks {
        let cold_diverging = matches!(b.term, RTerm::Unreachable);
        for op in &b.ops {
            let ok = match &op.kind {
                OpKind::Assert { .. } | OpKind::Zombie(_) => false,
                OpKind::Drop {
                    glue: crate::jir::semantic::DropGlue::None,
                    ..
                } => true,
                OpKind::Drop { .. } | OpKind::Host { .. } => cold_diverging,
                OpKind::Call {
                    semantic: CallSemantic::Standard(std_op),
                    ..
                } if is_pure_inline_lowered_std_op(*std_op) => true,
                OpKind::Call { .. } => {
                    if cold_diverging {
                        true
                    } else {
                        non_cold_calls += 1;
                        non_cold_calls <= 1
                    }
                }
                OpKind::Intrinsic { name, .. } => {
                    cold_diverging
                        || matches!(
                            interns.str(*name).unwrap_or(""),
                            "compare_bytes"
                                | "raw_eq"
                                | "ptr_metadata"
                                | "offset"
                                | "arith_offset"
                                | "slice_get_unchecked"
                                | "read_via_copy"
                                | "transmute"
                                | "transmute_unchecked"
                                | "cttz"
                                | "cttz_nonzero"
                                | "ctlz"
                                | "ctlz_nonzero"
                                | "ctpop"
                                | "bswap"
                                | "bitreverse"
                                | "rotate_left"
                                | "rotate_right"
                                | "likely"
                                | "unlikely"
                                | "select_unpredictable"
                                | "wrapping_add"
                                | "wrapping_sub"
                                | "wrapping_mul"
                                | "saturating_add"
                                | "saturating_sub"
                                | "add_with_overflow"
                                | "sub_with_overflow"
                                | "mul_with_overflow"
                                | "unchecked_add"
                                | "unchecked_sub"
                                | "unchecked_mul"
                                | "black_box"
                                | "size_of_val"
                                | "align_of_val"
                        )
                }
                _ => true,
            };
            if !ok {
                return false;
            }
        }
    }
    true
}

fn directly_calls_self(func: &Func, interns: &Interns, self_name: &JsName) -> bool {
    func.blocks.iter().any(|block| {
        block.ops.iter().any(|op| {
            matches!(
                &op.kind,
                OpKind::Call {
                    callee: Callee::Direct { link_name, .. },
                    ..
                } if interns.str(*link_name) == Some(self_name.as_str())
            )
        })
    })
}

/// Whether `callee`'s implicit trailing `Location` parameter has zero uses in its body.
fn unused_caller_location(callee: &Func) -> bool {
    let Some(&last_param) = callee.params.last() else {
        return false;
    };
    !callee.blocks.iter().any(|block| {
        let mut used = false;
        for op in &block.ops {
            op.kind.for_each_use(|v| {
                if v == last_param {
                    used = true;
                }
            });
            if used {
                return true;
            }
        }
        block.term.for_each_use(|v| {
            if v == last_param {
                used = true;
            }
        });
        used
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
            op.kind.visit_values_mut(|v| *v = remap_val(*v));
        }
        block.term.visit_values_mut(|v| *v = remap_val(*v));
    }
}

/// Whether `recv_ty` (`&mut Vec<T, Global>`) is a `TypedArray`-eligible numeric/scalar vector
/// whose `Vec::resize` is intercepted directly by `call.rs` (Phase 1B `TypedArray.fill`).
fn is_ta_vec_resize_receiver(table: &TypeTable, recv_ty: TypeId) -> bool {
    let Some(vec_ty) = table.pointee(recv_ty) else {
        return false;
    };
    let Some(elem_ty) = crate::analysis::heap::global_vec_elem_ty(table, vec_ty) else {
        return false;
    };
    crate::analysis::heap::native_vec_ta_kind(table, elem_ty).is_some()
        || matches!(
            table.get(table.peeled(elem_ty)).map(|d| &d.kind),
            Some(TypeKind::Bool | TypeKind::Char)
        )
}

/// Rewrites implicit `#[track_caller]` forwarding ops in `callee` to the call-site
/// `SourceFactId`.
fn rewrite_forwarded_track_caller(
    callee: &mut Func,
    site_fact: SourceFactId,
    items: &ItemIndex,
    interns: &Interns,
) {
    for block in &mut callee.blocks {
        for op in &mut block.ops {
            match &mut op.kind {
                OpKind::Call {
                    track_caller: Some(f),
                    ..
                } if f.is_forward() => {
                    *f = site_fact;
                }
                OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    track_caller: tc @ None,
                    ..
                } => {
                    if items.get(item).is_some_and(|entry| entry.caller_location) {
                        *tc = Some(site_fact);
                    }
                }
                OpKind::Intrinsic {
                    name,
                    track_caller: tc @ None,
                    ..
                } if interns.str(*name) == Some("caller_location") => {
                    *tc = Some(site_fact);
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
                    || matches!(semantic, CallSemantic::Host | CallSemantic::RawJavaScript))
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
            section, helpers, ..
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
                            | "add"
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
            kind.visit_values_mut(|v| *v = map_value(*v));
            kind.visit_cells_mut(|c| *c = map_cell(*c));
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
            other => {
                let mut term = other.clone();
                term.visit_values_mut(|v| *v = map_value(*v));
                term.targets_mut(|b| *b = map_block(*b));
                term
            }
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
    let old_term = std::mem::replace(
        &mut func.blocks[block].term,
        RTerm::Goto(map_block(RBlockId(0)), call_args),
    );
    let (cont_term, invoke_unwind) = match old_term {
        RTerm::Invoke { normal, unwind } if rest.is_empty() => {
            (RTerm::Goto(normal.0, normal.1), Some(unwind))
        }
        other => (other, None),
    };
    func.blocks.push(RBlock {
        id: continuation,
        params: call.result.into_iter().collect(),
        ops: rest,
        term: cont_term,
    });
    // When splicing a callee at an `RTerm::Invoke` site, redirect any potentially-throwing op
    // inside the spliced callee blocks to the caller's original `unwind` cleanup edge.
    if let Some(unwind) = invoke_unwind {
        let mut idx = block_offset as usize;
        while idx < func.blocks.len() {
            if idx == continuation.0 as usize {
                idx += 1;
                continue;
            }
            match &mut func.blocks[idx].term {
                RTerm::Invoke {
                    unwind: inner_unwind,
                    ..
                } => {
                    *inner_unwind = unwind.clone();
                    idx += 1;
                    continue;
                }
                RTerm::Throw(None) => {
                    func.blocks[idx].term = RTerm::Goto(unwind.0, unwind.1.clone());
                    idx += 1;
                    continue;
                }
                _ => {}
            }
            let throwing_pos = func.blocks[idx].ops.iter().position(|op| {
                matches!(
                    op.kind,
                    OpKind::Call { .. }
                        | OpKind::Drop {
                            glue: crate::jir::semantic::DropGlue::Direct { .. }
                                | crate::jir::semantic::DropGlue::Dynamic,
                            ..
                        }
                        | OpKind::Host { .. }
                        | OpKind::Intrinsic { .. }
                )
            });
            if let Some(pos) = throwing_pos {
                let fresh_next = RBlockId(func.blocks.len() as u32);
                let tail_ops = func.blocks[idx].ops.split_off(pos + 1);
                let tail_term = std::mem::replace(
                    &mut func.blocks[idx].term,
                    RTerm::Invoke {
                        normal: (fresh_next, Vec::new()),
                        unwind: unwind.clone(),
                    },
                );
                func.blocks.push(RBlock {
                    id: fresh_next,
                    params: Vec::new(),
                    ops: tail_ops,
                    term: tail_term,
                });
            }
            idx += 1;
        }
    } else {
        // Non-Invoke call site: any no-op `Invoke` admitted under speed mode unwinds straight out
        // of the caller, so replace it with `Goto(normal)`.
        for idx in (block_offset as usize)..func.blocks.len() {
            if idx == continuation.0 as usize {
                continue;
            }
            if let RTerm::Invoke { normal, .. } = &func.blocks[idx].term {
                let normal = normal.clone();
                func.blocks[idx].term = RTerm::Goto(normal.0, normal.1);
            }
        }
    }
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

#[cfg(test)]
#[allow(
    clippy::too_many_lines,
    reason = "unit tests build multi-block JIR fixtures inline"
)]
mod tests {
    use super::*;
    use crate::analysis::callgraph;
    use crate::jir::interns::StrId;
    use crate::jir::repr::{Carrier, FuncBuilder, Root};
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
                            summary: None,
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
                            is_zst_closure_receiver: false,
                            standard: None,
                        },
                    );
                }
            }
            run_with_config(
                funcs,
                &mut metadata,
                InlineConfig {
                    graph: &graph,
                    items: &items,
                    measure: &measure,
                    sole_referrer: &sole_referrer,
                    zombie_free: &|_: &JsName| true,
                    program_bytes,
                    sizes: &sizes,
                    allow_std_post_repr: false,
                    profile: None,
                    call_retargets: None,
                },
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
                semantic: CallSemantic::Host,
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
            let payload = HostPayload::Marker {
                section,
                def: item(1).definition,
                helpers: Vec::new(),
            };
            assert_eq!(independent_host(&payload, &interns, false), supported);
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
                        summary: None,
                    },
                ),
                (
                    g_name.clone(),
                    Metadata {
                        universe: &provider.interns,
                        interns: Cow::Borrowed(&provider.interns),
                        origins: Some(Cow::Borrowed(&provider_origins)),
                        header: Some(&provider_header),
                        summary: None,
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
            let stats = run_with_config(
                &mut funcs,
                &mut metadata,
                InlineConfig {
                    graph: &graph,
                    items: &ItemIndex::new(),
                    measure: &measure,
                    sole_referrer: &|_, _| true,
                    zombie_free: &|_| true,
                    program_bytes: 100000,
                    sizes: &sizes,
                    allow_std_post_repr: false,
                    profile: None,
                    call_retargets: None,
                },
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
                    summary: None,
                },
            ),
            (
                g_name.clone(),
                Metadata {
                    universe: &provider,
                    interns: Cow::Borrowed(&provider),
                    origins: Some(Cow::Borrowed(&provider_origins)),
                    header: Some(&provider_header),
                    summary: None,
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
            (g_name, measured(&g)),
        ]);
        let measure = |_: &JsName, func: &Func, meta: &Metadata<'_>| Some(measurement(func, meta));
        let stats = run_with_config(
            &mut funcs,
            &mut metadata,
            InlineConfig {
                graph: &graph,
                items: &ItemIndex::new(),
                measure: &measure,
                sole_referrer: &|_, _| true,
                zombie_free: &|_| true,
                program_bytes: 100_000,
                sizes: &sizes,
                allow_std_post_repr: false,
                profile: None,
                call_retargets: None,
            },
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
                        summary: None,
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
        let stats = run_with_config(
            funcs,
            &mut metadata,
            InlineConfig {
                graph: &graph,
                items: &items,
                measure: &|_name, func, _meta| {
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
                sole_referrer: &|callee, caller| callee == &g_name && caller == &f_name,
                zombie_free: &|_| true,
                program_bytes: 10_000,
                sizes: &sizes,
                allow_std_post_repr: false,
                profile: None,
                call_retargets: None,
            },
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
                        summary: None,
                    },
                )
            })
            .collect();
        let items = ItemIndex::new();
        // `entry_bytes = 50` (2x limit = 100) and `after = 150` (> 100), while `g` is 200 B so
        // sole-referrer removal yields `net = 100 - 200 = -100 <= 0` and lifts `size_limit`.
        let sizes = BTreeMap::from([(f_name.clone(), 50), (g_name.clone(), 200)]);
        let stats = run_with_config(
            funcs,
            &mut metadata,
            InlineConfig {
                graph: &graph,
                items: &items,
                measure: &|_name, func, _meta| {
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
                sole_referrer: &|callee, caller| callee == &g_name && caller == &f_name,
                zombie_free: &|_| true,
                program_bytes: 10_000,
                sizes: &sizes,
                allow_std_post_repr: false,
                profile: None,
                call_retargets: None,
            },
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
                        summary: None,
                    },
                )
            })
            .collect();
        let items = ItemIndex::new();
        let sizes = BTreeMap::from([(f_name, 1_000), (g_name.clone(), 455)]);
        let stats = run_with_config(
            funcs,
            &mut metadata,
            InlineConfig {
                graph: &graph,
                items: &items,
                measure: &|_name, func, _meta| {
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
                sole_referrer: &|_, _| false,
                zombie_free: &|_| true,
                program_bytes: 1_000,
                sizes: &sizes,
                allow_std_post_repr: false,
                profile: None,
                call_retargets: None,
            },
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
                        summary: None,
                    },
                )
            })
            .collect();
        let items = ItemIndex::new();
        // Shared callee at 88 B (in 65..=96): admitted when caller shrinks (`120 -> 116`).
        let sizes = BTreeMap::from([(f_name, 120), (g_name.clone(), 88)]);
        let stats = run_with_config(
            funcs,
            &mut metadata,
            InlineConfig {
                graph: &graph,
                items: &items,
                measure: &|_name, func, _meta| {
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
                sole_referrer: &|_, _| false,
                zombie_free: &|_| true,
                program_bytes: 10_000,
                sizes: &sizes,
                allow_std_post_repr: false,
                profile: None,
                call_retargets: None,
            },
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
                        summary: None,
                    },
                ),
                (
                    g_name.clone(),
                    Metadata {
                        universe: interns,
                        interns: Cow::Borrowed(interns),
                        origins: Some(Cow::Owned(build_origins(core_fact))),
                        header: None,
                        summary: None,
                    },
                ),
            ]);
            let sizes = BTreeMap::from([(f_name, 200), (g_name, 120)]);
            let stats = run_with_config(
                funcs,
                &mut metadata,
                InlineConfig {
                    graph: &graph,
                    items: &ItemIndex::new(),
                    measure: &|_, func, _| {
                        Some(measurement(func, &metadata_dummy_refs(func, interns)))
                    },
                    sole_referrer: &|_, _| true,
                    zombie_free: &|_| true,
                    program_bytes: 100_000,
                    sizes: &sizes,
                    allow_std_post_repr: false,
                    profile: None,
                    call_retargets: None,
                },
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
                        summary: None,
                    },
                ),
                (
                    g_name.clone(),
                    Metadata {
                        universe: interns,
                        interns: Cow::Borrowed(interns),
                        origins: Some(Cow::Owned(build_origins(site_fact))),
                        header: None,
                        summary: None,
                    },
                ),
            ]);
            let sizes = BTreeMap::from([(f_name, 200), (g_name, 120)]);
            let stats = run_with_config(
                funcs,
                &mut metadata,
                InlineConfig {
                    graph: &graph,
                    items: &ItemIndex::new(),
                    measure: &|_, func, _| {
                        Some(measurement(func, &metadata_dummy_refs(func, interns)))
                    },
                    sole_referrer: &|_, _| true,
                    zombie_free: &|_| true,
                    program_bytes: 100_000,
                    sizes: &sizes,
                    allow_std_post_repr: false,
                    profile: None,
                    call_retargets: None,
                },
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
            summary: None,
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
                        summary: None,
                    },
                )
            })
            .collect();
        // Shared callee (`sole_referrer = false`) priced at 120 bytes (> 96B SHARED_CALLEE_FOOTPRINT),
        // with +12B temporary caller growth (within <=16B trivial leaf slack) and 0 budget charge.
        let sizes = BTreeMap::from([(f_name, 100), (g_name.clone(), 120)]);
        let stats = run_with_config(
            funcs,
            &mut metadata,
            InlineConfig {
                graph: &graph,
                items: &ItemIndex::new(),
                measure: &|_, func, _| {
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
                sole_referrer: &|_, _| false,
                zombie_free: &|_| true,
                program_bytes: 10_000,
                sizes: &sizes,
                allow_std_post_repr: false,
                profile: None,
                call_retargets: None,
            },
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
                        summary: None,
                    },
                )
            })
            .collect();
        // Make each intermediate site grow by +25B (>16B slack so single-site rejects when refs still contain g),
        // while batching all 3 sites yields 150 -> 180 (+30B) - 80B removed = -50B net!
        let sizes = BTreeMap::from([(f_name, 150), (g_name.clone(), 80)]);
        let stats = run_with_config(
            funcs,
            &mut metadata,
            InlineConfig {
                graph: &graph,
                items: &ItemIndex::new(),
                measure: &|_, func, _| {
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
                sole_referrer: &|_, _| true,
                zombie_free: &|_| true,
                program_bytes: 10_000,
                sizes: &sizes,
                allow_std_post_repr: false,
                profile: None,
                call_retargets: None,
            },
        );
        assert_eq!(
            stats.applied, 3,
            "all 3 sole-caller sites should batch-inline"
        );
        assert_eq!(prog.calls_in("f"), 0);
    }

    #[test]
    fn track_caller_live_assert_inlines_and_imports_panic_dep_from_summary() {
        use crate::jir::origin::{Origin, OriginKind, OriginTable, SourceFact};
        use crate::jir::summary::{Availability, ItemClass, ItemSummary, SemanticKey};
        use rcgjs_object::item::{ItemKind, Linkage};
        use rcgjs_object::object::BlobRef;
        use std::borrow::Cow;

        let make_summary = |deps: Vec<&str>| ItemSummary {
            key: SemanticKey {
                link_name: "f".to_owned(),
                item: None,
            },
            body_hash: None,
            semantic_hash: None,
            availability: Availability::Available,
            deps: deps.into_iter().map(str::to_owned).collect(),
            abi: None,
            physical: None,
            kind: ItemKind::Fn,
            linkage: Linkage::internal(),
            order: None,
            dts: false,
            fallback: BlobRef {
                offset: 0,
                clen: 0,
                ulen: 0,
            },
            body: None,
            origin: None,
            estimated_bytes: 0,
            class: ItemClass::empty(),
        };
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
        let g_str = prog.strid("g_bounds");

        let mut g_sig = sig(2);
        g_sig.abi.caller_location = true;
        let mut g_b = FuncBuilder::new(item(2), g_sig);
        let idx = g_b.param(I32, OriginId(0));
        let _loc = g_b.param(I32, OriginId(0));
        let len = g_b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 4, bytes: 4 }),
            I32,
            OriginId(0),
        );
        let cmp = g_b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: BinaryOp::Lt,
                l: idx,
                r: len,
                ty: I32,
            },
            I32,
            OriginId(0),
        );
        g_b.effect(
            FuncBuilder::ENTRY,
            OpKind::Assert {
                cond: cmp,
                expected: true,
                msg: RAssertMessage::BoundsCheck {
                    length: len,
                    index: idx,
                },
                track_caller: None,
            },
            OriginId(0),
        );
        g_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(idx)));
        let g = g_b.finish();

        let mut f_b = FuncBuilder::new(item(1), sig(1));
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
                track_caller: Some(site_fact),
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            OriginId(0),
        );
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(call)));
        let f = f_b.finish();

        prog.add("g_bounds", g);
        prog.add("f", f);

        let panic_dep = "core$panicking$panic_bounds_check$ha80078aa06207de3";
        let interns = &prog.interns;
        let funcs = &mut prog.funcs;
        let interns_map: BTreeMap<JsName, &Interns> =
            funcs.keys().map(|name| (name.clone(), interns)).collect();
        let graph = callgraph::build(funcs, &interns_map);
        let f_name = JsName::new("f".to_owned());
        let g_name = JsName::new("g_bounds".to_owned());
        let mut metadata = BTreeMap::from([
            (
                f_name.clone(),
                Metadata {
                    universe: interns,
                    interns: Cow::Borrowed(interns),
                    origins: Some(Cow::Owned(build_origins(site_fact))),
                    header: None,
                    summary: Some(Cow::Owned(make_summary(vec![]))),
                },
            ),
            (
                g_name.clone(),
                Metadata {
                    universe: interns,
                    interns: Cow::Borrowed(interns),
                    origins: Some(Cow::Owned(build_origins(core_fact))),
                    header: None,
                    summary: Some(Cow::Owned(make_summary(vec![panic_dep]))),
                },
            ),
        ]);
        let sizes = BTreeMap::from([(f_name.clone(), 200), (g_name, 120)]);
        let stats = run_with_config(
            funcs,
            &mut metadata,
            InlineConfig {
                graph: &graph,
                items: &ItemIndex::new(),
                measure: &|_, func, _| Some(measurement(func, &metadata_dummy_refs(func, interns))),
                sole_referrer: &|_, _| true,
                zombie_free: &|_| true,
                program_bytes: 100_000,
                sizes: &sizes,
                allow_std_post_repr: false,
                profile: None,
                call_retargets: None,
            },
        );
        assert_eq!(
            stats.applied, 1,
            "live Assert in #[track_caller] callee should inline when panic dep resolves from summary"
        );
        assert_eq!(prog.calls_in("f"), 0);
        let f_deps = &metadata
            .get(&f_name)
            .and_then(|m| m.summary.as_deref())
            .expect("caller summary preserved")
            .deps;
        assert!(
            f_deps.iter().any(|d| d == panic_dep),
            "panic lang dep should be imported into caller summary: {f_deps:?}"
        );
    }

    #[test]
    fn speed_tiny_leaves_inline_past_caller_cap_and_measurement_noise() {
        use crate::jir::semantic::StandardOperation;
        use crate::jir::types::{
            FieldDescriptor, FieldKey, NominalForm, PointerForm, VariantDescriptor,
        };

        let mut prog = Prog::new();
        let vec_ty = TypeId(20);
        let ref_vec_ty = TypeId(21);
        let vec_name = prog.strid("Vec");
        let f_len = prog.strid("len");
        let js_base = |peeled: TypeId| JsFacts {
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
        let vec_fields = vec![FieldDescriptor {
            key: FieldKey::Name(f_len),
            ty: I32,
            offset: 0,
        }];
        prog.interns.intern_type(TypeDescriptor {
            id: vec_ty,
            name: Some(vec_name),
            kind: TypeKind::Adt {
                def: item(80).definition,
                form: NominalForm::Struct,
                args: Vec::new(),
                variants: vec![VariantDescriptor {
                    name: vec_name,
                    logical_discr: 0,
                    physical_tag: None,
                    fields: vec_fields.clone(),
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            layout: LayoutFacts {
                sized: true,
                size: Some(4),
                align: Some(4),
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                is_indirect: true,
                fields: vec_fields,
                ..js_base(vec_ty)
            },
        });
        prog.interns.intern_type(TypeDescriptor {
            id: ref_vec_ty,
            name: None,
            kind: TypeKind::Reference {
                pointee: vec_ty,
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
                ..js_base(ref_vec_ty)
            },
        });

        // 1) Identity helper (`Range::into_iter`: `function(a){return a}`, 23 B, `#[inline]`).
        let id_str = prog.strid("into_iter");
        let mut id_sig = sig(1);
        id_sig.abi.inline_hint = true;
        let mut id_b = FuncBuilder::new(item(2), id_sig);
        let p = id_b.param(I32, O);
        id_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(p)));
        let into_iter = id_b.finish();

        // 2) `Ord::min`-shaped 3-block leaf (`b < a ? b : a`, 35 B, `#[inline]`).
        let min_str = prog.strid("ord_min");
        let mut min_sig = sig(2);
        min_sig.abi.inline_hint = true;
        let mut min_b = FuncBuilder::new(item(3), min_sig);
        let a = min_b.param(I32, O);
        let b = min_b.param(I32, O);
        let cmp = min_b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: BinaryOp::Lt,
                l: b,
                r: a,
                ty: I32,
            },
            I32,
            O,
        );
        let then_bb = min_b.block();
        let else_bb = min_b.block();
        min_b.terminate(
            FuncBuilder::ENTRY,
            RTerm::Switch {
                scrut: cmp,
                scrut_ty: I32,
                cases: vec![(1, then_bb, Vec::new())],
                default: (else_bb, Vec::new()),
            },
        );
        min_b.terminate(then_bb, RTerm::Return(Some(b)));
        min_b.terminate(else_bb, RTerm::Return(Some(a)));
        let ord_min = min_b.finish();

        // 3) `Vec::len` (`CallSemantic::Standard(VecLen)`, `Deref` + `Field`, 27 B, `#[inline]`).
        let len_str = prog.strid("vec_len");
        let mut len_sig = Signature {
            arguments: vec![ref_vec_ty],
            result: I32,
            abi: AbiSignature {
                parameters: 1,
                inline_hint: true,
                ..AbiSignature::default()
            },
        };
        len_sig.abi.inline_hint = true;
        let mut len_b = FuncBuilder::new(item(4), len_sig);
        let vref = len_b.param(ref_vec_ty, O);
        let vval = len_b.op(FuncBuilder::ENTRY, OpKind::Deref(vref), vec_ty, O);
        let vlen = len_b.op(
            FuncBuilder::ENTRY,
            OpKind::Field {
                base: vval,
                variant: None,
                field: 0,
            },
            I32,
            O,
        );
        len_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(vlen)));
        let vec_len = len_b.finish();

        // Caller calls all three helpers.
        let caller_sig = Signature {
            arguments: vec![ref_vec_ty, I32],
            result: I32,
            abi: AbiSignature {
                parameters: 2,
                ..AbiSignature::default()
            },
        };
        let mut f_b = FuncBuilder::new(item(1), caller_sig);
        let pv = f_b.param(ref_vec_ty, O);
        let px = f_b.param(I32, O);
        let c1 = f_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: id_str,
                },
                function: None,
                args: vec![px],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        let c2 = f_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(4),
                    link_name: len_str,
                },
                function: None,
                args: vec![pv],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::VecLen),
                host: None,
            },
            I32,
            O,
        );
        let c3 = f_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(3),
                    link_name: min_str,
                },
                function: None,
                args: vec![c1, c2],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(c3)));
        let caller = f_b.finish();

        prog.add("into_iter", into_iter);
        prog.add("ord_min", ord_min);
        prog.add("vec_len", vec_len);
        prog.add("caller", caller);

        let f_name = JsName::new("caller".to_owned());
        let id_name = JsName::new("into_iter".to_owned());
        let min_name = JsName::new("ord_min".to_owned());
        let len_name = JsName::new("vec_len".to_owned());
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
                    metadata_dummy_refs(funcs[name].as_ref().unwrap(), interns),
                )
            })
            .collect();
        // Huge caller already past `SPEED_CALLER_CAP` (40_000 B), and each splice adds +45 B of
        // local-renaming noise (exceeding raw 23..35 B callee sizes, within the +96 B slack).
        let sizes = BTreeMap::from([
            (f_name.clone(), 40_000),
            (id_name, 23),
            (min_name, 35),
            (len_name, 27),
        ]);
        let stats = run_with_config(
            funcs,
            &mut metadata,
            InlineConfig {
                graph: &graph,
                items: &ItemIndex::new(),
                measure: &|name, func, _| {
                    if name != &f_name {
                        return Some((sizes[name], BTreeSet::new()));
                    }
                    let mut remaining = 0u64;
                    let mut refs = BTreeSet::new();
                    for op in func.blocks.iter().flat_map(|b| &b.ops) {
                        if let OpKind::Call {
                            callee: Callee::Direct { link_name, .. },
                            ..
                        } = &op.kind
                        {
                            remaining += 1;
                            if let Some(s) = interns.str(*link_name) {
                                refs.insert(JsName::new(s.to_owned()));
                            }
                        }
                    }
                    let inlined = 3u64.saturating_sub(remaining);
                    Some((40_000 + inlined * 45, refs))
                },
                sole_referrer: &|_, _| false,
                zombie_free: &|_| true,
                program_bytes: 50_000,
                sizes: &sizes,
                allow_std_post_repr: true,
                profile: Some(crate::opts::Profile::ReleaseSpeed),
                call_retargets: None,
            },
        );
        assert_eq!(
            stats.applied, 3,
            "all 3 tiny leaves should inline past cap/noise (declined={:?})",
            stats.declined
        );
        assert_eq!(stats.witnesses_failed, 0);
        assert_eq!(prog.calls_in("caller"), 0);
    }

    #[test]
    fn speed_vec_index_nonnull_location_inlines_while_ta_vec_resize_stays_outline() {
        use crate::jir::origin::{Origin, OriginKind, OriginTable, SourceFact};
        use crate::jir::semantic::StandardOperation;
        use crate::jir::summary::{Availability, ItemClass, ItemSummary, SemanticKey};
        use crate::jir::types::{
            AllocatorIdentity, FieldDescriptor, FieldKey, NominalForm, PointerForm,
            RepresentationCandidate, VariantDescriptor,
        };
        use rcgjs_object::item::{ItemKind, Linkage};
        use rcgjs_object::object::BlobRef;
        use std::borrow::Cow;

        let mut prog = Prog::new();
        let str_ty = TypeId(30);
        let raw_str_ty = TypeId(31);
        let u32_ty = TypeId(32);
        let loc_ty = TypeId(33);
        let ref_loc_ty = TypeId(34);
        let u8_ty = TypeId(35);
        let vec_u8_ty = TypeId(36);
        let ref_vec_u8_ty = TypeId(37);
        let mut_vec_u8_ty = TypeId(38);
        let unit_ty = TypeId(39);

        let js_base = |peeled: TypeId| JsFacts {
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
            js: js_base(str_ty),
        });
        // Real `core::panic::Location` stores `file: NonNull<str>` (peels to `RawPointer { pointee: Str }`).
        prog.interns.intern_type(TypeDescriptor {
            id: raw_str_ty,
            name: None,
            kind: TypeKind::RawPointer {
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
                pointer_form: Some(PointerForm::FatSlice),
                ..js_base(raw_str_ty)
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
                ..js_base(u32_ty)
            },
        });
        prog.interns.intern_type(TypeDescriptor {
            id: u8_ty,
            name: None,
            kind: TypeKind::Integer {
                signed: false,
                bits: 8,
            },
            layout: LayoutFacts {
                sized: true,
                size: Some(1),
                align: Some(1),
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: Some(IntFacts {
                    signed: false,
                    bits: 8,
                    repr: IntRepr::Number,
                }),
                byte_element: true,
                ..js_base(u8_ty)
            },
        });
        prog.interns.intern_type(TypeDescriptor {
            id: unit_ty,
            name: None,
            kind: TypeKind::Tuple(Vec::new()),
            layout: LayoutFacts {
                sized: true,
                size: Some(0),
                align: Some(1),
                zst: true,
                one_zst: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: js_base(unit_ty),
        });
        let loc_name = prog.strid("Location");
        let f_file = prog.strid("file");
        let f_line = prog.strid("line");
        let f_col = prog.strid("col");
        let loc_fields = vec![
            FieldDescriptor {
                key: FieldKey::Name(f_file),
                ty: raw_str_ty,
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
                def: item(91).definition,
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
                fields: loc_fields,
                ..js_base(loc_ty)
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
                ..js_base(ref_loc_ty)
            },
        });
        let vec_name = prog.strid("Vec");
        prog.interns.intern_type(TypeDescriptor {
            id: vec_u8_ty,
            name: Some(vec_name),
            kind: TypeKind::Adt {
                def: item(92).definition,
                form: NominalForm::Struct,
                args: vec![u8_ty],
                variants: vec![VariantDescriptor {
                    name: vec_name,
                    logical_discr: 0,
                    physical_tag: None,
                    fields: Vec::new(),
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            layout: LayoutFacts {
                sized: true,
                size: Some(12),
                align: Some(4),
                ..LayoutFacts::default()
            },
            meta: TypeMeta {
                candidate: RepresentationCandidate::Vec,
                allocator: AllocatorIdentity::Global,
                ..TypeMeta::default()
            },
            js: JsFacts {
                is_indirect: true,
                ..js_base(vec_u8_ty)
            },
        });
        prog.interns.intern_type(TypeDescriptor {
            id: ref_vec_u8_ty,
            name: None,
            kind: TypeKind::Reference {
                pointee: vec_u8_ty,
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
                ..js_base(ref_vec_u8_ty)
            },
        });
        prog.interns.intern_type(TypeDescriptor {
            id: mut_vec_u8_ty,
            name: None,
            kind: TypeKind::Reference {
                pointee: vec_u8_ty,
                mutable: true,
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
                ..js_base(mut_vec_u8_ty)
            },
        });

        let file_s = prog.strid("enclose.rs");
        let site_fact = prog.interns.intern_source_fact(SourceFact {
            file: file_s,
            line: 42,
            col: 3,
        });
        let vec_index_s = prog.strid("vec_index");
        let vec_resize_s = prog.strid("vec_resize");
        let native_as_slice_s = prog.strid("rcgjs_vec_as_slice");

        // `Vec::index` (`#[track_caller]`, uses `&Location` with `NonNull<str>` and calls `NativeVecAsSlice`).
        let idx_sig = Signature {
            arguments: vec![ref_vec_u8_ty, u32_ty],
            result: u32_ty,
            abi: AbiSignature {
                parameters: 3,
                caller_location: true,
                inline_hint: true,
                ..AbiSignature::default()
            },
        };
        let mut idx_b = FuncBuilder::new(item(2), idx_sig);
        let p_vec = idx_b.param(ref_vec_u8_ty, O);
        let p_i = idx_b.param(u32_ty, O);
        let p_loc = idx_b.param(ref_loc_ty, O);
        let _slice = idx_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(9),
                    link_name: native_as_slice_s,
                },
                function: None,
                args: vec![p_vec],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeVecAsSlice),
                host: None,
            },
            u32_ty,
            O,
        );
        let loc_v = idx_b.op(FuncBuilder::ENTRY, OpKind::Deref(p_loc), loc_ty, O);
        let line_v = idx_b.op(
            FuncBuilder::ENTRY,
            OpKind::Field {
                base: loc_v,
                variant: None,
                field: 1,
            },
            u32_ty,
            O,
        );
        let out_v = idx_b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: p_i,
                r: line_v,
                ty: u32_ty,
            },
            u32_ty,
            O,
        );
        idx_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(out_v)));
        let vec_index = idx_b.finish();

        // `Vec::resize` on `Vec<u8, Global>` (must NOT inline under speed so Phase 1B `call.rs` intercepts it).
        let resize_sig = Signature {
            arguments: vec![mut_vec_u8_ty, u32_ty, u8_ty],
            result: unit_ty,
            abi: AbiSignature {
                parameters: 3,
                inline_hint: true,
                ..AbiSignature::default()
            },
        };
        let mut res_b = FuncBuilder::new(item(3), resize_sig);
        let _rv = res_b.param(mut_vec_u8_ty, O);
        let _rn = res_b.param(u32_ty, O);
        let _rx = res_b.param(u8_ty, O);
        res_b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let vec_resize = res_b.finish();

        let caller_sig = Signature {
            arguments: vec![ref_vec_u8_ty, mut_vec_u8_ty, u32_ty, u8_ty],
            result: u32_ty,
            abi: AbiSignature {
                parameters: 4,
                ..AbiSignature::default()
            },
        };
        let mut f_b = FuncBuilder::new(item(1), caller_sig);
        let pv = f_b.param(ref_vec_u8_ty, O);
        let pmv = f_b.param(mut_vec_u8_ty, O);
        let pi = f_b.param(u32_ty, O);
        let px = f_b.param(u8_ty, O);
        f_b.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(3),
                    link_name: vec_resize_s,
                },
                function: None,
                args: vec![pmv, pi, px],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::VecResize),
                host: None,
            },
            O,
        );
        let got = f_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: vec_index_s,
                },
                function: None,
                args: vec![pv, pi],
                track_caller: Some(site_fact),
                semantic: CallSemantic::Standard(StandardOperation::VecIndex),
                host: None,
            },
            u32_ty,
            O,
        );
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(got)));
        let caller = f_b.finish();

        prog.add("vec_index", vec_index);
        prog.add("vec_resize", vec_resize);
        prog.add("caller", caller);

        let mut items = ItemIndex::new();
        items.insert(
            item(9),
            crate::lower::js::host::ItemEntry {
                name: "rcgjs_vec_as_slice".to_owned(),
                params: 1,
                abi_known: true,
                result: Some(u32_ty),
                arguments: vec![ref_vec_u8_ty],
                caller_location: false,
                caller_location_def: None,
                is_core: false,
                is_zst_closure_receiver: false,
                standard: Some(StandardOperation::NativeVecAsSlice),
            },
        );
        let make_origins = |fact| OriginTable {
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
        let make_summary = || ItemSummary {
            key: SemanticKey {
                link_name: "f".to_owned(),
                item: None,
            },
            body_hash: None,
            semantic_hash: None,
            availability: Availability::Available,
            deps: Vec::new(),
            abi: None,
            physical: None,
            kind: ItemKind::Fn,
            linkage: Linkage::internal(),
            order: None,
            dts: false,
            fallback: BlobRef {
                offset: 0,
                clen: 0,
                ulen: 0,
            },
            body: None,
            origin: None,
            estimated_bytes: 0,
            class: ItemClass::empty(),
        };

        let f_name = JsName::new("caller".to_owned());
        let idx_name = JsName::new("vec_index".to_owned());
        let res_name = JsName::new("vec_resize".to_owned());
        let interns = &prog.interns;
        let funcs = &mut prog.funcs;
        let interns_map: BTreeMap<JsName, &Interns> =
            funcs.keys().map(|name| (name.clone(), interns)).collect();
        let graph = callgraph::build(funcs, &interns_map);
        let mut metadata = BTreeMap::from([
            (
                f_name.clone(),
                Metadata {
                    universe: interns,
                    interns: Cow::Borrowed(interns),
                    origins: Some(Cow::Owned(make_origins(site_fact))),
                    header: None,
                    summary: Some(Cow::Owned(make_summary())),
                },
            ),
            (
                idx_name.clone(),
                Metadata {
                    universe: interns,
                    interns: Cow::Borrowed(interns),
                    origins: Some(Cow::Owned(make_origins(site_fact))),
                    header: None,
                    summary: Some(Cow::Owned(make_summary())),
                },
            ),
            (
                res_name.clone(),
                Metadata {
                    universe: interns,
                    interns: Cow::Borrowed(interns),
                    origins: Some(Cow::Owned(make_origins(site_fact))),
                    header: None,
                    summary: Some(Cow::Owned(make_summary())),
                },
            ),
        ]);
        let sizes = BTreeMap::from([(f_name.clone(), 400), (idx_name, 90), (res_name, 50)]);
        let stats = run_with_config(
            funcs,
            &mut metadata,
            InlineConfig {
                graph: &graph,
                items: &items,
                measure: &|name, func, _| {
                    if name != &f_name {
                        return Some((sizes[name], BTreeSet::new()));
                    }
                    let has_idx = func.blocks.iter().flat_map(|b| &b.ops).any(|op| {
                        matches!(
                            &op.kind,
                            OpKind::Call {
                                callee: Callee::Direct { item: id, .. },
                                ..
                            } if *id == item(2)
                        )
                    });
                    let bytes = if has_idx { 400 } else { 440 };
                    Some((bytes, BTreeSet::new()))
                },
                sole_referrer: &|_, _| false,
                zombie_free: &|_| true,
                program_bytes: 10_000,
                sizes: &sizes,
                allow_std_post_repr: true,
                profile: Some(crate::opts::Profile::ReleaseSpeed),
                call_retargets: None,
            },
        );
        assert_eq!(
            stats.applied, 1,
            "Vec::index with NonNull<str> Location should inline, Vec::resize must stay outline (declined={:?})",
            stats.declined
        );
        // 1 semantic-boundary decline for `rcgjs_vec_as_slice` inside `vec_index`, plus 1 for
        // TA-eligible `Vec::resize` inside `caller`.
        assert_eq!(
            stats
                .declined
                .get("inline.semantic-boundary")
                .copied()
                .unwrap_or(0),
            2,
            "NativeVecAsSlice and TA-eligible Vec::resize should both be declined at semantic boundary"
        );
        let remaining_callees: Vec<ItemId> = prog
            .func("caller")
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter_map(|op| match &op.kind {
                OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    ..
                } => Some(*item),
                _ => None,
            })
            .collect();
        assert!(
            remaining_callees.contains(&item(3))
                && remaining_callees.contains(&item(9))
                && !remaining_callees.contains(&item(2)),
            "expected Vec::resize (3) + inlined NativeVecAsSlice (9), without Vec::index (2): {remaining_callees:?}"
        );
    }

    #[test]
    fn speed_sole_caller_loop_closure_above_640b_inlines_both_sites() {
        let mut prog = Prog::new();
        let helper_str = prog.strid("external_op");
        let closure_str = prog.strid("analyse_closure_0");

        // Callee closure: non-leaf (calls `external_op`), priced at 750 B (> 640 B, <= 1024 B).
        let mut c_b = FuncBuilder::new(item(2), sig(1));
        let cx = c_b.param(I32, O);
        let cres = c_b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(99),
                    link_name: helper_str,
                },
                function: None,
                args: vec![cx],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        c_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(cres)));
        let closure = c_b.finish();

        // Caller calls `analyse_closure_0` twice: once inside a loop block, once after the loop.
        let mut f_b = FuncBuilder::new(item(1), sig(1));
        let x0 = f_b.param(I32, O);
        let loop_bb = f_b.block();
        let exit_bb = f_b.block();
        let lp = f_b.block_param(loop_bb, I32, O);
        f_b.terminate(FuncBuilder::ENTRY, RTerm::Goto(loop_bb, vec![x0]));
        let in_loop = f_b.op(
            loop_bb,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: closure_str,
                },
                function: None,
                args: vec![lp],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        f_b.terminate(
            loop_bb,
            RTerm::Switch {
                scrut: in_loop,
                scrut_ty: I32,
                cases: vec![(0, exit_bb, vec![in_loop])],
                default: (loop_bb, vec![in_loop]),
            },
        );
        let ep = f_b.block_param(exit_bb, I32, O);
        let after_loop = f_b.op(
            exit_bb,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: closure_str,
                },
                function: None,
                args: vec![ep],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        f_b.terminate(exit_bb, RTerm::Return(Some(after_loop)));
        let caller = f_b.finish();

        prog.add("analyse_closure_0", closure);
        prog.add("analyse", caller);

        let f_name = JsName::new("analyse".to_owned());
        let c_name = JsName::new("analyse_closure_0".to_owned());
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
                    metadata_dummy_refs(funcs[name].as_ref().unwrap(), interns),
                )
            })
            .collect();
        let sizes = BTreeMap::from([(f_name.clone(), 1_200), (c_name.clone(), 750)]);
        let stats = run_with_config(
            funcs,
            &mut metadata,
            InlineConfig {
                graph: &graph,
                items: &ItemIndex::new(),
                measure: &|name, func, _| {
                    if name != &f_name {
                        return Some((750, BTreeSet::new()));
                    }
                    let remaining = func
                        .blocks
                        .iter()
                        .flat_map(|b| &b.ops)
                        .filter(|op| {
                            matches!(
                                &op.kind,
                                OpKind::Call {
                                    callee: Callee::Direct { item: id, .. },
                                    ..
                                } if *id == item(2)
                            )
                        })
                        .count() as u64;
                    let mut refs = BTreeSet::new();
                    if remaining > 0 {
                        refs.insert(c_name.clone());
                    }
                    let inlined = 2u64.saturating_sub(remaining);
                    Some((1_200 + inlined * 700, refs))
                },
                sole_referrer: &|callee, caller| callee == &c_name && caller == &f_name,
                zombie_free: &|_| true,
                program_bytes: 50_000,
                sizes: &sizes,
                allow_std_post_repr: true,
                profile: Some(crate::opts::Profile::ReleaseSpeed),
                call_retargets: None,
            },
        );
        assert_eq!(
            stats.applied, 2,
            "both loop and post-loop sites of 750B sole-caller closure should inline (declined={:?})",
            stats.declined
        );
        let remaining_closure_calls = prog
            .func("analyse")
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter(|op| {
                matches!(
                    &op.kind,
                    OpKind::Call {
                        callee: Callee::Direct { item: id, .. },
                        ..
                    } if *id == item(2)
                )
            })
            .count();
        assert_eq!(remaining_closure_calls, 0);
    }

    #[test]
    fn sole_caller_mutually_recursive_helper_inlines_and_terminates() {
        // `eval` calls `helper` (1 site, sole referrer), and `helper` calls `eval` (2-node SCC).
        // Under ReleaseSpeed post-repr, `helper` should inline into `eval`, turning the mutual
        // recursion into direct self-recursion on `eval` and stopping cleanly.
        let mut prog = Prog::new();
        let eval_link = prog.strid("eval");
        let helper_link = prog.strid("helper");

        let mut hb = FuncBuilder::new(item(2), sig(1));
        let hp = hb.param(I32, O);
        let hcall = hb.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(1),
                    link_name: eval_link,
                },
                function: None,
                args: vec![hp],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        hb.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(hcall)));
        let helper_func = hb.finish();

        let mut eb = FuncBuilder::new(item(1), sig(1));
        let ep = eb.param(I32, O);
        let b_base = eb.block();
        let b_rec = eb.block();
        eb.terminate(
            FuncBuilder::ENTRY,
            RTerm::Switch {
                scrut: ep,
                scrut_ty: I32,
                cases: vec![(0, b_base, Vec::new())],
                default: (b_rec, Vec::new()),
            },
        );
        eb.terminate(b_base, RTerm::Return(Some(ep)));
        let ecall = eb.op(
            b_rec,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: helper_link,
                },
                function: None,
                args: vec![ep],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        eb.terminate(b_rec, RTerm::Return(Some(ecall)));
        let eval_func = eb.finish();

        prog.add("eval", eval_func);
        prog.add("helper", helper_func);

        let e_name = JsName::new("eval".to_owned());
        let h_name = JsName::new("helper".to_owned());
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
                    metadata_dummy_refs(funcs[name].as_ref().unwrap(), interns),
                )
            })
            .collect();
        assert!(graph.recursive(&e_name) && graph.recursive(&h_name));
        let sizes = BTreeMap::from([(e_name.clone(), 1_600), (h_name.clone(), 1_400)]);
        let stats = run_with_config(
            funcs,
            &mut metadata,
            InlineConfig {
        graph: &graph,
        items: &ItemIndex::new(),
        measure: &|name, func, _| {
                if name != &e_name {
                    return Some((1_400, BTreeSet::new()));
                }
                let calls_helper = func.blocks.iter().flat_map(|b| &b.ops).any(|op| {
                    matches!(
                        &op.kind,
                        OpKind::Call {
                            callee: Callee::Direct { item: id, .. },
                            ..
                        } if *id == item(2)
                    )
                });
                let mut refs = BTreeSet::new();
                if calls_helper {
                    refs.insert(h_name.clone());
                }
                Some((if calls_helper { 1_600 } else { 2_900 }, refs))
            },
        sole_referrer: // Only `helper` has sole referrer `eval`; `eval` is an external entry point.
            &|callee, caller| callee == &h_name && caller == &e_name,
        zombie_free: &|_| true,
        program_bytes: 50_000,
        sizes: &sizes,
        allow_std_post_repr: true,
        profile: Some(crate::opts::Profile::ReleaseSpeed),
        call_retargets: None,
    },
        );
        assert_eq!(
            stats.applied, 1,
            "sole-caller mutually-recursive helper should inline once into eval (declined={:?})",
            stats.declined
        );
        let calls_helper = prog
            .func("eval")
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .any(|op| {
                matches!(
                    &op.kind,
                    OpKind::Call {
                        callee: Callee::Direct { item: id, .. },
                        ..
                    } if *id == item(2)
                )
            });
        let calls_self = prog
            .func("eval")
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .any(|op| {
                matches!(
                    &op.kind,
                    OpKind::Call {
                        callee: Callee::Direct { item: id, .. },
                        ..
                    } if *id == item(1)
                )
            });
        assert!(!calls_helper, "eval must no longer call helper");
        assert!(
            calls_self,
            "eval should now directly self-recurse after splicing helper"
        );
    }
}
