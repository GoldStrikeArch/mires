//! Exact compiler-link artifact economics for a coupled physical ABI plan.
//!
//! Legality only creates a candidate. This module compares the fully lowered canonical and
//! physical programs under the active release policy, including compact bytes, deterministic
//! gzip and Brotli bytes, and final JavaScript allocation sites. The lane owns construction of
//! those hypothetical programs; policy and caching live here so report and applying modes make
//! one decision.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;

use flate2::{Compression, GzBuilder};

use crate::item::{JsName, LinkItem};
use crate::jir::hash::{Hash128, Hasher128};
use crate::jir::repr::Func;
use crate::jir::semantic::{ItemId, TypeId};
use crate::opts::{MinifyMode, Modules, NameStyle, Opts, Profile};
use crate::plan::abi_flat::AbiPlan;

const COMPRESSED_GROWTH_BPS: u64 = 50;
const SPEED_GROWTH_BPS: u64 = 5_000;
const HOT_ALLOC_SMALL_PROGRAM_SLACK_BYTES: u64 = 256;
const SPEED_SMALL_PROGRAM_SLACK_BYTES: u64 = 4_096;

/// Streaming deterministic binary hash builder backed by [`Hasher128`].
///
/// Feeds structured tags, little-endian integers, length-prefixed strings, and `serde::Serialize`
/// values (via `postcard::to_io`) directly into SipHash-1-3-128 with zero `Debug` formatting and
/// zero intermediate heap buffers.
pub(crate) struct HashFeed {
    hasher: Hasher128,
    bytes: usize,
}

impl HashFeed {
    pub(crate) fn new() -> Self {
        Self {
            hasher: Hasher128::new(),
            bytes: 0,
        }
    }

    pub(crate) fn bytes(&mut self, data: &[u8]) {
        self.hasher.write(data);
        self.bytes = self.bytes.saturating_add(data.len());
    }

    pub(crate) fn u8(&mut self, value: u8) {
        self.bytes(&[value]);
    }

    pub(crate) fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn u128(&mut self, value: u128) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn usize(&mut self, value: usize) {
        self.u64(value as u64);
    }

    pub(crate) fn str(&mut self, value: &str) {
        self.usize(value.len());
        self.bytes(value.as_bytes());
    }

    pub(crate) fn item_id(&mut self, id: ItemId) {
        self.u64(id.definition.crate_id);
        self.u64(id.definition.local_hash);
        self.u128(id.instantiation);
    }

    pub(crate) fn type_id(&mut self, id: TypeId) {
        self.u128(id.0);
    }

    pub(crate) fn opt<T>(&mut self, value: Option<T>, f: impl FnOnce(&mut Self, T)) {
        match value {
            Some(inner) => {
                self.u8(1);
                f(self, inner);
            }
            None => self.u8(0),
        }
    }

    pub(crate) fn slice<T>(&mut self, items: &[T], mut f: impl FnMut(&mut Self, &T)) {
        self.usize(items.len());
        for item in items {
            f(self, item);
        }
    }

    pub(crate) fn func(&mut self, value: &Func) {
        postcard::to_io(value, self).expect("streaming Func into HashFeed is infallible");
    }

    pub(crate) fn decline(&mut self, value: &crate::jir::decline::Decline) {
        postcard::to_io(value, self).expect("streaming Decline into HashFeed is infallible");
    }

    pub(crate) fn bytes_written(&self) -> usize {
        self.bytes
    }

    pub(crate) fn finish(&self) -> Hash128 {
        self.hasher.finish()
    }
}

impl std::io::Write for HashFeed {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Exact metrics of one fully lowered, deduplicated, renamed program and its helper closure.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProgramMetrics {
    pub min: u64,
    pub gzip: u64,
    pub brotli: u64,
    pub allocations: u64,
    pub loop_allocations: u64,
    pub alloc_term: u64,
}

impl ProgramMetrics {
    fn balanced_score(self) -> u64 {
        self.min.saturating_add(self.alloc_term)
    }
}

/// Typed policy rejection reason mapped onto the physical report's stable string vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RejectReason {
    PhysicalMeasurement,
    MinifiedNoImprovement,
    CompressedNoImprovement,
    GzipRegression,
    BrotliRegression,
    ScoreNoImprovement,
    CompressedGrowthBudget,
    AllocationRegression,
    HotnessEvidenceMissing,
    SpeedGrowthBudget,
    GlobalGrowthBudget,
}

impl RejectReason {
    const fn code(self) -> &'static str {
        match self {
            Self::PhysicalMeasurement => "validation.physical-measurement",
            Self::MinifiedNoImprovement => "cost.minified-no-improvement",
            Self::CompressedNoImprovement => "cost.compressed-no-improvement",
            Self::GzipRegression => "cost.gzip-regression",
            Self::BrotliRegression => "cost.brotli-regression",
            Self::ScoreNoImprovement => "cost.score-no-improvement",
            Self::CompressedGrowthBudget => "cost.compressed-growth-budget",
            Self::AllocationRegression => "cost.allocation-regression",
            Self::HotnessEvidenceMissing => "cost.hotness-evidence-missing",
            Self::SpeedGrowthBudget => "cost.speed-growth-budget",
            Self::GlobalGrowthBudget => "cost.global-growth-budget",
        }
    }
}

/// Structured outcome of evaluating one physical ABI candidate against a policy profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PolicyOutcome {
    accepted: bool,
    reason: Option<RejectReason>,
    hotness_source: Option<&'static str>,
    budget_consumed_bps: u64,
}

impl PolicyOutcome {
    fn rejected(reason: RejectReason, budget_consumed_bps: u64) -> Self {
        Self {
            accepted: false,
            reason: Some(reason),
            hotness_source: None,
            budget_consumed_bps,
        }
    }

    fn reason_string(self) -> Option<String> {
        self.reason.map(|r| r.code().to_owned())
    }
}

/// The measured policy decision shared by emission and the physical sidecar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evaluation {
    pub profile: &'static str,
    pub before: Option<ProgramMetrics>,
    /// The complete legal label vector before group economics.
    pub after: Option<ProgramMetrics>,
    /// The union of groups that survived exact economics.
    pub validated: Option<ProgramMetrics>,
    /// What the applying or non-emitting mode actually writes.
    pub final_metrics: Option<ProgramMetrics>,
    pub accepted: bool,
    pub applied: bool,
    pub reason: Option<String>,
    pub hotness_source: Option<&'static str>,
    pub budget_consumed_bps: u64,
    pub cache_hits: u32,
    pub decisions: BTreeMap<JsName, DecisionEvaluation>,
}

/// Exact marginal evidence for one ABI-connected group, copied onto each decision in the group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionEvaluation {
    pub before: Option<ProgramMetrics>,
    pub after: Option<ProgramMetrics>,
    pub reason: Option<String>,
    pub hotness_source: Option<&'static str>,
    pub budget_consumed_bps: u64,
}

/// One-link cache. A complete input digest owns each measurement, so a hit cannot cross programs,
/// physical label vectors, profiles, minifier settings, or helper-affecting item bodies.
#[derive(Default)]
struct Cache {
    entries: BTreeMap<Hash128, Option<ProgramMetrics>>,
    hits: u32,
}

#[derive(Clone)]
struct Trial {
    items: BTreeSet<ItemId>,
    after: Option<ProgramMetrics>,
    outcome: PolicyOutcome,
    benefit: u64,
}

impl Cache {
    fn measure(
        &mut self,
        key: Hash128,
        measure: impl FnOnce() -> Option<ProgramMetrics>,
    ) -> Option<ProgramMetrics> {
        if let Some(metrics) = self.entries.get(&key) {
            self.hits += 1;
            crate::profile::bump(crate::profile::Counter::CacheHits);
            return *metrics;
        }
        crate::profile::bump(crate::profile::Counter::CacheMisses);
        let metrics = measure();
        self.entries.insert(key, metrics);
        metrics
    }
}

struct SelectionRecorder<'a> {
    candidate: &'a AbiPlan,
    selected: BTreeSet<ItemId>,
    reasons: BTreeMap<ItemId, String>,
    decision_economics: BTreeMap<JsName, DecisionEvaluation>,
}

impl<'a> SelectionRecorder<'a> {
    fn new(candidate: &'a AbiPlan) -> Self {
        Self {
            candidate,
            selected: BTreeSet::new(),
            reasons: BTreeMap::new(),
            decision_economics: BTreeMap::new(),
        }
    }

    fn record(
        &mut self,
        trial: &Trial,
        outcome: PolicyOutcome,
        before: Option<ProgramMetrics>,
        after: Option<ProgramMetrics>,
    ) {
        let reason = outcome.reason_string();
        let evidence = DecisionEvaluation {
            before,
            after,
            reason: reason.clone(),
            hotness_source: outcome.hotness_source,
            budget_consumed_bps: outcome.budget_consumed_bps,
        };
        for item in &trial.items {
            let source = self
                .candidate
                .source_name(*item)
                .expect("an economic item has a source name")
                .clone();
            self.decision_economics.insert(source, evidence.clone());
            if outcome.accepted {
                self.selected.insert(*item);
            } else {
                self.reasons.insert(
                    *item,
                    reason
                        .clone()
                        .unwrap_or_else(|| RejectReason::PhysicalMeasurement.code().to_owned()),
                );
            }
        }
    }
}

/// Prices each ABI-connected group and returns the exact economically selected union.
pub fn evaluate_groups(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, crate::jir::decline::Decline>>,
    candidate: &AbiPlan,
    opts: &Opts,
    applied: bool,
    mut measure: impl FnMut(&AbiPlan) -> Option<ProgramMetrics>,
) -> (AbiPlan, Evaluation) {
    let profile = release_profile(opts);
    let mut cache = Cache::default();
    let program = program_key(table, funcs, profile, opts);
    let baseline = AbiPlan::default();
    let before = measure_cached(&mut cache, program, &baseline, &mut measure);
    let full_after = measure_cached(&mut cache, program, candidate, &mut measure);

    let mut trials = Vec::new();
    for items in candidate.economic_groups() {
        let plan = candidate.subset(&items);
        let after = measure_cached(&mut cache, program, &plan, &mut measure);
        let outcome = decide_for_plan(profile, &plan, before, after);
        trials.push(Trial {
            items,
            after,
            outcome,
            benefit: policy_benefit(profile, before, after),
        });
    }
    trials.sort_by(|left, right| {
        right
            .outcome
            .accepted
            .cmp(&left.outcome.accepted)
            .then_with(|| right.benefit.cmp(&left.benefit))
            .then_with(|| left.items.cmp(&right.items))
    });

    let mut recorder = SelectionRecorder::new(candidate);
    let mut current = before;
    let individually_selected: BTreeSet<ItemId> = trials
        .iter()
        .filter(|trial| trial.outcome.accepted)
        .flat_map(|trial| trial.items.iter().copied())
        .collect();
    let combined = candidate.subset(&individually_selected);
    let combined_metrics = (!individually_selected.is_empty())
        .then(|| measure_cached(&mut cache, program, &combined, &mut measure));
    let combined_accepted = combined_metrics
        .flatten()
        .is_some_and(|after| decide_for_plan(profile, &combined, before, Some(after)).accepted);

    if combined_accepted {
        recorder.selected = individually_selected;
        current = combined_metrics.flatten();
        for trial in trials {
            recorder.record(&trial, trial.outcome, before, trial.after);
        }
    } else {
        for trial in trials {
            let evidence_before = if trial.outcome.accepted {
                current
            } else {
                before
            };
            let (outcome, next) = if trial.outcome.accepted {
                let mut proposed = recorder.selected.clone();
                proposed.extend(&trial.items);
                let proposed_plan = candidate.subset(&proposed);
                let step_plan = candidate.subset(&trial.items);
                let next = measure_cached(&mut cache, program, &proposed_plan, &mut measure);
                let outcome = decide_incremental(profile, &step_plan, before, current, next);
                (outcome, next)
            } else {
                (trial.outcome, trial.after)
            };
            recorder.record(&trial, outcome, evidence_before, next);
            if outcome.accepted {
                current = next;
            }
        }
    }

    // Replay the selected key through the cache to guard key completeness.
    let replay_plan = candidate.subset(&recorder.selected);
    let replay = measure_cached(&mut cache, program, &replay_plan, &mut measure);
    debug_assert_eq!(replay, current);
    current = replay;
    let selected_plan = candidate.with_economic_selection(&recorder.selected, &recorder.reasons);
    let all_outcome = decide_for_plan(profile, candidate, before, full_after);
    let final_metrics = if applied { current } else { before };
    (
        selected_plan,
        Evaluation {
            profile: profile.as_str(),
            before,
            after: full_after,
            validated: current,
            final_metrics,
            accepted: !recorder.selected.is_empty(),
            applied,
            reason: (!all_outcome.accepted)
                .then(|| all_outcome.reason_string())
                .flatten(),
            hotness_source: all_outcome.hotness_source,
            budget_consumed_bps: all_outcome.budget_consumed_bps,
            cache_hits: cache.hits,
            decisions: recorder.decision_economics,
        },
    )
}

fn measure_cached(
    cache: &mut Cache,
    program: Hash128,
    plan: &AbiPlan,
    measure: &mut impl FnMut(&AbiPlan) -> Option<ProgramMetrics>,
) -> Option<ProgramMetrics> {
    let key = cache_key(program, plan);
    cache.measure(key, || measure(plan))
}

fn policy_benefit(
    profile: Profile,
    before: Option<ProgramMetrics>,
    after: Option<ProgramMetrics>,
) -> u64 {
    let (Some(before), Some(after)) = (before, after) else {
        return 0;
    };
    match profile {
        Profile::ReleaseSize => before.min.saturating_sub(after.min),
        Profile::ReleaseBalanced | Profile::Dev | Profile::Report => before
            .balanced_score()
            .saturating_sub(after.balanced_score()),
        Profile::ReleaseSpeed => before
            .loop_allocations
            .saturating_sub(after.loop_allocations)
            .saturating_mul(16)
            .saturating_add(before.allocations.saturating_sub(after.allocations)),
    }
}

fn decide_for_plan(
    profile: Profile,
    plan: &AbiPlan,
    before: Option<ProgramMetrics>,
    after: Option<ProgramMetrics>,
) -> PolicyOutcome {
    let mut outcome = decide(profile, before, after);
    // Under ReleaseSpeed, scalarizing a slice parameter, flattening `i64`/`u64`, specializing
    // read-only byte-strings, or using `SentinelResult` removes property loads or wrapper
    // allocations across tight loops/recursive call trees. Accept whenever it does not regress
    // object allocations and stays within the speed budget.
    if !outcome.accepted
        && profile == Profile::ReleaseSpeed
        && (plan.has_slice_param()
            || plan.has_off_zero_slice()
            || plan.has_i64_pair_abi()
            || plan.has_byte_str_abi()
            || plan.has_sentinel_result())
        && (outcome.reason == Some(RejectReason::HotnessEvidenceMissing) || plan.has_byte_str_abi())
    {
        if let (Some(b), Some(a)) = (before, after) {
            let allocs_ok = plan.has_byte_str_abi()
                || (a.loop_allocations <= b.loop_allocations && a.allocations <= b.allocations);
            if allocs_ok && speed_within_budget(b, a) {
                outcome.accepted = true;
                outcome.reason = None;
                outcome.hotness_source = Some(if plan.has_byte_str_abi() {
                    "bstr-slice-specialization"
                } else if plan.has_sentinel_result() {
                    "sentinel-result-abi"
                } else if plan.has_i64_pair_abi() {
                    "i64-pair-abi-scalarization"
                } else {
                    "slice-param-scalarization"
                });
            }
        }
    }
    outcome
}

fn decide_incremental(
    profile: Profile,
    step_plan: &AbiPlan,
    baseline: Option<ProgramMetrics>,
    current: Option<ProgramMetrics>,
    after: Option<ProgramMetrics>,
) -> PolicyOutcome {
    let decision = decide_for_plan(profile, step_plan, current, after);
    if !decision.accepted {
        return decision;
    }
    let (Some(baseline), Some(after)) = (baseline, after) else {
        return PolicyOutcome::rejected(RejectReason::PhysicalMeasurement, 0);
    };
    let global_ok = match profile {
        Profile::ReleaseSpeed => speed_within_budget(baseline, after),
        Profile::ReleaseBalanced | Profile::Dev | Profile::Report => {
            balanced_compressed_within_budget(baseline, after)
        }
        Profile::ReleaseSize => {
            within_growth(after.gzip, baseline.gzip, COMPRESSED_GROWTH_BPS)
                && within_growth(after.brotli, baseline.brotli, COMPRESSED_GROWTH_BPS)
        }
    };
    if global_ok {
        decision
    } else {
        PolicyOutcome::rejected(
            RejectReason::GlobalGrowthBudget,
            growth_bps(after.min, baseline.min)
                .max(growth_bps(after.gzip, baseline.gzip))
                .max(growth_bps(after.brotli, baseline.brotli)),
        )
    }
}

/// Compresses the exact compact program with fixed in-process encoders.
pub fn program_metrics(
    program: &str,
    allocations: u64,
    loop_allocations: u64,
    alloc_term: u64,
) -> ProgramMetrics {
    crate::profile::bump(crate::profile::Counter::ProgramMetrics);
    crate::profile::add(crate::profile::Counter::CompressInputBytes, program.len());
    let gzip_scope = crate::profile::scope("compress.gzip");
    let mut gzip = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::best());
    gzip.write_all(program.as_bytes())
        .expect("writing to a Vec cannot fail");
    let gzip = gzip.finish().expect("writing to a Vec cannot fail").len() as u64;
    drop(gzip_scope);

    let brotli_scope = crate::profile::scope("compress.brotli");
    let mut brotli = brotli::CompressorWriter::new(Vec::new(), 4096, 11, 22);
    brotli
        .write_all(program.as_bytes())
        .expect("writing to a Vec cannot fail");
    let brotli = brotli.into_inner().len() as u64;
    drop(brotli_scope);

    ProgramMetrics {
        min: program.len() as u64,
        gzip,
        brotli,
        allocations,
        loop_allocations,
        alloc_term,
    }
}

fn release_profile(opts: &Opts) -> Profile {
    match opts.profile {
        Some(Profile::ReleaseSize) => Profile::ReleaseSize,
        Some(Profile::ReleaseSpeed) => Profile::ReleaseSpeed,
        Some(Profile::ReleaseBalanced) | None => Profile::ReleaseBalanced,
        Some(Profile::Dev | Profile::Report) => Profile::ReleaseBalanced,
    }
}

fn decide(
    profile: Profile,
    before: Option<ProgramMetrics>,
    after: Option<ProgramMetrics>,
) -> PolicyOutcome {
    let (Some(before), Some(after)) = (before, after) else {
        return PolicyOutcome::rejected(RejectReason::PhysicalMeasurement, 0);
    };
    let budget = growth_bps(after.min, before.min)
        .max(growth_bps(after.gzip, before.gzip))
        .max(growth_bps(after.brotli, before.brotli));

    match profile {
        Profile::ReleaseSize => {
            let reason = if after.min >= before.min {
                Some(RejectReason::MinifiedNoImprovement)
            } else if after.gzip >= before.gzip && after.brotli >= before.brotli {
                Some(RejectReason::CompressedNoImprovement)
            } else if !within_growth(after.gzip, before.gzip, COMPRESSED_GROWTH_BPS) {
                Some(RejectReason::GzipRegression)
            } else if !within_growth(after.brotli, before.brotli, COMPRESSED_GROWTH_BPS) {
                Some(RejectReason::BrotliRegression)
            } else {
                None
            };
            PolicyOutcome {
                accepted: reason.is_none(),
                reason,
                hotness_source: None,
                budget_consumed_bps: budget,
            }
        }
        Profile::ReleaseBalanced | Profile::Dev | Profile::Report => {
            let reason = if after.balanced_score() >= before.balanced_score() {
                Some(RejectReason::ScoreNoImprovement)
            } else if !balanced_compressed_within_budget(before, after) {
                Some(RejectReason::CompressedGrowthBudget)
            } else if after.allocations > before.allocations
                || after.loop_allocations > before.loop_allocations
            {
                Some(RejectReason::AllocationRegression)
            } else {
                None
            };
            PolicyOutcome {
                accepted: reason.is_none(),
                reason,
                hotness_source: None,
                budget_consumed_bps: budget,
            }
        }
        Profile::ReleaseSpeed => {
            let hot = after.loop_allocations < before.loop_allocations;
            let alloc_win = after.allocations < before.allocations;
            let bounded = speed_within_budget(before, after);
            let reason = if !hot && !alloc_win {
                Some(RejectReason::HotnessEvidenceMissing)
            } else if !bounded {
                Some(RejectReason::SpeedGrowthBudget)
            } else {
                None
            };
            PolicyOutcome {
                accepted: reason.is_none(),
                reason,
                hotness_source: hot
                    .then_some("final-js-loop-allocation")
                    .or_else(|| alloc_win.then_some("final-js-allocation")),
                budget_consumed_bps: budget,
            }
        }
    }
}

fn balanced_compressed_within_budget(before: ProgramMetrics, after: ProgramMetrics) -> bool {
    let strict = within_growth(after.gzip, before.gzip, COMPRESSED_GROWTH_BPS)
        && within_growth(after.brotli, before.brotli, COMPRESSED_GROWTH_BPS);
    if strict {
        return true;
    }
    let major_hot_alloc_win = after.loop_allocations < before.loop_allocations
        && after.alloc_term.saturating_mul(4) <= before.alloc_term.saturating_mul(3);
    major_hot_alloc_win
        && within_growth_or_bytes(
            after.gzip,
            before.gzip,
            COMPRESSED_GROWTH_BPS,
            HOT_ALLOC_SMALL_PROGRAM_SLACK_BYTES,
        )
        && within_growth_or_bytes(
            after.brotli,
            before.brotli,
            COMPRESSED_GROWTH_BPS,
            HOT_ALLOC_SMALL_PROGRAM_SLACK_BYTES,
        )
}

fn speed_within_budget(before: ProgramMetrics, after: ProgramMetrics) -> bool {
    within_growth_or_bytes(
        after.min,
        before.min,
        SPEED_GROWTH_BPS,
        SPEED_SMALL_PROGRAM_SLACK_BYTES,
    ) && within_growth_or_bytes(
        after.gzip,
        before.gzip,
        SPEED_GROWTH_BPS,
        SPEED_SMALL_PROGRAM_SLACK_BYTES,
    ) && within_growth_or_bytes(
        after.brotli,
        before.brotli,
        SPEED_GROWTH_BPS,
        SPEED_SMALL_PROGRAM_SLACK_BYTES,
    )
}

fn within_growth_or_bytes(after: u64, before: u64, basis_points: u64, max_bytes: u64) -> bool {
    within_growth(after, before, basis_points) || after <= before.saturating_add(max_bytes)
}

fn within_growth(after: u64, before: u64, basis_points: u64) -> bool {
    after.saturating_mul(10_000) <= before.saturating_mul(10_000u64.saturating_add(basis_points))
}

fn growth_bps(after: u64, before: u64) -> u64 {
    if after <= before || before == 0 {
        return 0;
    }
    after
        .saturating_sub(before)
        .saturating_mul(10_000)
        .div_ceil(before)
}

/// The digest of every key input that one [`evaluate_groups`] holds fixed: the release profile,
/// the naming and minifier settings, every item, and every function body. Uses deterministic
/// binary hashing with zero `Debug` formatting.
fn program_key(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, crate::jir::decline::Decline>>,
    profile: Profile,
    opts: &Opts,
) -> Hash128 {
    let _scope = crate::profile::scope("physical_cache.program_key");
    let mut feed = HashFeed::new();
    feed.u8(0x50); // domain tag
    feed.str(profile.as_str());
    feed.u8(match opts.names {
        NameStyle::Readable => 1,
        NameStyle::Mangled => 2,
    });
    feed.u8(match opts.modules {
        Modules::Esm => 1,
        Modules::Script => 2,
    });
    feed.u8(match opts.minify {
        MinifyMode::Off => 0,
        MinifyMode::Locals => 1,
        MinifyMode::All => 2,
    });

    feed.u8(0x01);
    feed.usize(table.len());
    for (name, item) in table {
        feed.str(name.as_str());
        feed.opt(item.jir.summary.as_ref(), |f, summary| {
            f.opt(summary.key.item, |f, item_id| f.item_id(item_id));
            f.opt(summary.semantic_hash, |f, h| {
                f.u64(h.lo);
                f.u64(h.hi);
            });
        });
        feed.u64(Hash128::of(item.js.as_bytes()).lo);
    }

    feed.u8(0x02);
    feed.usize(funcs.len());
    for (name, func) in funcs {
        feed.str(name.as_str());
        match func {
            Ok(func) => {
                feed.u8(1);
                feed.func(func);
            }
            Err(decline) => {
                feed.u8(0);
                feed.decline(decline);
            }
        }
    }

    crate::profile::add(crate::profile::Counter::CacheKeyBytes, feed.bytes_written());
    feed.finish()
}

/// The cache key of one measurement: the evaluation's fixed [`program_key`] and the ABI plan.
fn cache_key(program: Hash128, abi: &AbiPlan) -> Hash128 {
    crate::profile::bump(crate::profile::Counter::CacheKeyCalls);
    let _scope = crate::profile::scope("physical_cache.key");
    let mut feed = HashFeed::new();
    feed.bytes(&program.to_bytes());
    abi.extend_cost_key(&mut feed);
    crate::profile::add(crate::profile::Counter::CacheKeyBytes, feed.bytes_written());
    feed.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(min: u64, gzip: u64, brotli: u64, loop_allocations: u64) -> ProgramMetrics {
        ProgramMetrics {
            min,
            gzip,
            brotli,
            allocations: loop_allocations,
            loop_allocations,
            alloc_term: loop_allocations * 96,
        }
    }

    #[test]
    fn release_size_requires_raw_and_compressed_improvement() {
        let before = metrics(1000, 500, 400, 0);
        assert!(
            decide(
                Profile::ReleaseSize,
                Some(before),
                Some(metrics(990, 499, 401, 0))
            )
            .accepted
        );
        assert_eq!(
            decide(
                Profile::ReleaseSize,
                Some(before),
                Some(metrics(1001, 490, 390, 0))
            )
            .reason,
            Some(RejectReason::MinifiedNoImprovement)
        );
    }

    #[test]
    fn speed_needs_a_bounded_loop_allocation_win() {
        let before = metrics(1000, 500, 400, 2);
        let accepted = decide(
            Profile::ReleaseSpeed,
            Some(before),
            Some(metrics(1040, 520, 415, 1)),
        );
        assert!(accepted.accepted);
        assert_eq!(accepted.hotness_source, Some("final-js-loop-allocation"));
        assert_eq!(
            decide(
                Profile::ReleaseSpeed,
                Some(before),
                Some(metrics(990, 490, 390, 2)),
            )
            .reason,
            Some(RejectReason::HotnessEvidenceMissing)
        );
    }

    #[test]
    fn compression_and_cache_are_deterministic() {
        let first = program_metrics("function f(){return 1}\n", 0, 0, 0);
        assert_eq!(first, program_metrics("function f(){return 1}\n", 0, 0, 0));
        let mut cache = Cache::default();
        let key = Hash128::of(b"same");
        assert_eq!(cache.measure(key, || Some(first)), Some(first));
        assert_eq!(
            cache.measure(key, || panic!("a hit must not measure")),
            Some(first)
        );
        assert_eq!(cache.hits, 1);
    }

    #[test]
    fn balanced_allows_small_slack_only_for_major_hot_alloc_reduction() {
        let before = ProgramMetrics {
            min: 6652,
            gzip: 2294,
            brotli: 2080,
            allocations: 15,
            loop_allocations: 13,
            alloc_term: 1296,
        };
        let major_win = ProgramMetrics {
            min: 6850,
            gzip: 2429,
            brotli: 2175,
            allocations: 8,
            loop_allocations: 8,
            alloc_term: 768,
        };
        assert!(decide(Profile::ReleaseBalanced, Some(before), Some(major_win)).accepted);

        let minor_before = ProgramMetrics {
            min: 15983,
            gzip: 4444,
            brotli: 4003,
            allocations: 95,
            loop_allocations: 54,
            alloc_term: 6168,
        };
        let minor_after = ProgramMetrics {
            min: 15976,
            gzip: 4481,
            brotli: 4034,
            allocations: 91,
            loop_allocations: 50,
            alloc_term: 5784,
        };
        assert_eq!(
            decide(
                Profile::ReleaseBalanced,
                Some(minor_before),
                Some(minor_after)
            )
            .reason,
            Some(RejectReason::CompressedGrowthBudget)
        );
    }
}
