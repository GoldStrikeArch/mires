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
use crate::jir::hash::Hash128;
use crate::jir::repr::Func;
use crate::jir::semantic::ItemId;
use crate::opts::{Opts, Profile};
use crate::plan::abi_flat::AbiPlan;

const COMPRESSED_GROWTH_BPS: u64 = 50;
const SPEED_GROWTH_BPS: u64 = 500;

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

impl Evaluation {
    pub fn final_emitted(&self) -> Option<ProgramMetrics> {
        self.final_metrics.or(if self.accepted && self.applied {
            self.validated
        } else {
            self.before
        })
    }

    /// The surviving program after a verification retry removed the physical plan.
    pub fn after_retry(&self) -> Evaluation {
        Evaluation {
            validated: self.before,
            final_metrics: self.before,
            accepted: false,
            reason: Some("validation.plan-retry".to_owned()),
            hotness_source: None,
            budget_consumed_bps: 0,
            ..self.clone()
        }
    }
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
    accepted: bool,
    reason: Option<String>,
    hotness_source: Option<&'static str>,
    budget_consumed_bps: u64,
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
            return *metrics;
        }
        let metrics = measure();
        self.entries.insert(key, metrics);
        metrics
    }
}

/// Prices each ABI-connected group and returns the exact economically selected union.
///
/// Standalone group wins are ordered by their protected policy benefit, then admitted
/// incrementally against the already selected program and the original global growth cap. This
/// keeps unrelated cold ABI candidates from masking a profitable hot graph while preserving one
/// transactional plan for every connected caller/callee region.
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
    let baseline = AbiPlan::default();
    let before = measure_cached(
        &mut cache,
        table,
        funcs,
        &baseline,
        profile,
        opts,
        &mut measure,
    );
    let full_after = measure_cached(
        &mut cache,
        table,
        funcs,
        candidate,
        profile,
        opts,
        &mut measure,
    );
    let mut trials = Vec::new();
    for items in candidate.economic_groups() {
        let plan = candidate.subset(&items);
        let after = measure_cached(&mut cache, table, funcs, &plan, profile, opts, &mut measure);
        let (accepted, reason, hotness_source, budget_consumed_bps) =
            decide(profile, before, after);
        trials.push(Trial {
            items,
            after,
            accepted,
            reason,
            hotness_source,
            budget_consumed_bps,
            benefit: policy_benefit(profile, before, after),
        });
    }
    trials.sort_by(|left, right| {
        right
            .accepted
            .cmp(&left.accepted)
            .then_with(|| right.benefit.cmp(&left.benefit))
            .then_with(|| left.items.cmp(&right.items))
    });

    let mut selected = BTreeSet::new();
    let mut reasons = BTreeMap::new();
    let mut decision_economics = BTreeMap::new();
    let mut current = before;
    let individually_selected: BTreeSet<ItemId> = trials
        .iter()
        .filter(|trial| trial.accepted)
        .flat_map(|trial| trial.items.iter().copied())
        .collect();
    let combined = candidate.subset(&individually_selected);
    let combined_metrics = (!individually_selected.is_empty()).then(|| {
        measure_cached(
            &mut cache,
            table,
            funcs,
            &combined,
            profile,
            opts,
            &mut measure,
        )
    });
    let combined_accepted = combined_metrics
        .flatten()
        .is_some_and(|after| decide(profile, before, Some(after)).0);

    if combined_accepted {
        selected = individually_selected;
        current = combined_metrics.flatten();
        for trial in trials {
            record_trial(
                candidate,
                &trial,
                trial.accepted,
                before,
                trial.after,
                trial.reason.clone(),
                trial.hotness_source,
                trial.budget_consumed_bps,
                &mut selected,
                &mut reasons,
                &mut decision_economics,
            );
        }
    } else {
        for trial in trials {
            let evidence_before = if trial.accepted { current } else { before };
            let (accepted, reason, hotness_source, budget_consumed_bps, next) = if trial.accepted {
                let mut proposed = selected.clone();
                proposed.extend(&trial.items);
                let proposed_plan = candidate.subset(&proposed);
                let next = measure_cached(
                    &mut cache,
                    table,
                    funcs,
                    &proposed_plan,
                    profile,
                    opts,
                    &mut measure,
                );
                let (accepted, reason, hotness_source, budget_consumed_bps) =
                    decide_incremental(profile, before, current, next);
                (accepted, reason, hotness_source, budget_consumed_bps, next)
            } else {
                (
                    false,
                    trial.reason.clone(),
                    trial.hotness_source,
                    trial.budget_consumed_bps,
                    trial.after,
                )
            };
            record_trial(
                candidate,
                &trial,
                accepted,
                evidence_before,
                next,
                reason,
                hotness_source,
                budget_consumed_bps,
                &mut selected,
                &mut reasons,
                &mut decision_economics,
            );
            if accepted {
                current = next;
            }
        }
    }

    // Replay the selected key through the cache. Besides guarding key completeness in every real
    // link, this makes the receipt prove that a hit returns the decision's exact final metrics.
    let replay_plan = candidate.subset(&selected);
    let replay = measure_cached(
        &mut cache,
        table,
        funcs,
        &replay_plan,
        profile,
        opts,
        &mut measure,
    );
    debug_assert_eq!(replay, current);
    current = replay;
    let selected_plan = candidate.with_economic_selection(&selected, &reasons);
    let (all_accepted, all_reason, all_hotness, all_budget) = decide(profile, before, full_after);
    let final_metrics = if applied { current } else { before };
    (
        selected_plan,
        Evaluation {
            profile: profile.as_str(),
            before,
            after: full_after,
            validated: current,
            final_metrics,
            accepted: !selected.is_empty(),
            applied,
            reason: (!all_accepted).then_some(all_reason).flatten(),
            hotness_source: all_hotness,
            budget_consumed_bps: all_budget,
            cache_hits: cache.hits,
            decisions: decision_economics,
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn record_trial(
    candidate: &AbiPlan,
    trial: &Trial,
    accepted: bool,
    before: Option<ProgramMetrics>,
    after: Option<ProgramMetrics>,
    reason: Option<String>,
    hotness_source: Option<&'static str>,
    budget_consumed_bps: u64,
    selected: &mut BTreeSet<ItemId>,
    reasons: &mut BTreeMap<ItemId, String>,
    decision_economics: &mut BTreeMap<JsName, DecisionEvaluation>,
) {
    let evidence = DecisionEvaluation {
        before,
        after,
        reason: reason.clone(),
        hotness_source,
        budget_consumed_bps,
    };
    for item in &trial.items {
        let source = candidate
            .source_name(*item)
            .expect("an economic item has a source name")
            .clone();
        decision_economics.insert(source, evidence.clone());
        if accepted {
            selected.insert(*item);
        } else {
            reasons.insert(
                *item,
                reason
                    .clone()
                    .unwrap_or_else(|| "validation.physical-measurement".to_owned()),
            );
        }
    }
}

fn measure_cached(
    cache: &mut Cache,
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, crate::jir::decline::Decline>>,
    plan: &AbiPlan,
    profile: Profile,
    opts: &Opts,
    measure: &mut impl FnMut(&AbiPlan) -> Option<ProgramMetrics>,
) -> Option<ProgramMetrics> {
    let key = cache_key(table, funcs, plan, profile, opts);
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
            .saturating_sub(after.loop_allocations),
    }
}

fn decide_incremental(
    profile: Profile,
    baseline: Option<ProgramMetrics>,
    current: Option<ProgramMetrics>,
    after: Option<ProgramMetrics>,
) -> (bool, Option<String>, Option<&'static str>, u64) {
    let decision = decide(profile, current, after);
    if !decision.0 {
        return decision;
    }
    let (Some(baseline), Some(after)) = (baseline, after) else {
        return (
            false,
            Some("validation.physical-measurement".to_owned()),
            None,
            0,
        );
    };
    let cap = match profile {
        Profile::ReleaseSpeed => SPEED_GROWTH_BPS,
        _ => COMPRESSED_GROWTH_BPS,
    };
    let global_ok = match profile {
        Profile::ReleaseSpeed => {
            within_growth(after.min, baseline.min, cap)
                && within_growth(after.gzip, baseline.gzip, cap)
                && within_growth(after.brotli, baseline.brotli, cap)
        }
        _ => {
            within_growth(after.gzip, baseline.gzip, cap)
                && within_growth(after.brotli, baseline.brotli, cap)
        }
    };
    if global_ok {
        decision
    } else {
        (
            false,
            Some("cost.global-growth-budget".to_owned()),
            None,
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
    let mut gzip = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::best());
    gzip.write_all(program.as_bytes())
        .expect("writing to a Vec cannot fail");
    let gzip = gzip.finish().expect("writing to a Vec cannot fail").len() as u64;

    let mut brotli = brotli::CompressorWriter::new(Vec::new(), 4096, 11, 22);
    brotli
        .write_all(program.as_bytes())
        .expect("writing to a Vec cannot fail");
    let brotli = brotli.into_inner().len() as u64;

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
) -> (bool, Option<String>, Option<&'static str>, u64) {
    let (Some(before), Some(after)) = (before, after) else {
        return (
            false,
            Some("validation.physical-measurement".to_owned()),
            None,
            0,
        );
    };
    let budget = growth_bps(after.min, before.min)
        .max(growth_bps(after.gzip, before.gzip))
        .max(growth_bps(after.brotli, before.brotli));
    let compressed_safe = within_growth(after.gzip, before.gzip, COMPRESSED_GROWTH_BPS)
        && within_growth(after.brotli, before.brotli, COMPRESSED_GROWTH_BPS);

    match profile {
        Profile::ReleaseSize => {
            let reason = if after.min >= before.min {
                Some("cost.minified-no-improvement".to_owned())
            } else if after.gzip >= before.gzip && after.brotli >= before.brotli {
                Some("cost.compressed-no-improvement".to_owned())
            } else if !within_growth(after.gzip, before.gzip, COMPRESSED_GROWTH_BPS) {
                Some("cost.gzip-regression".to_owned())
            } else if !within_growth(after.brotli, before.brotli, COMPRESSED_GROWTH_BPS) {
                Some("cost.brotli-regression".to_owned())
            } else {
                None
            };
            (reason.is_none(), reason, None, budget)
        }
        Profile::ReleaseBalanced | Profile::Dev | Profile::Report => {
            let reason = if after.balanced_score() >= before.balanced_score() {
                Some("cost.score-no-improvement".to_owned())
            } else if !compressed_safe {
                Some("cost.compressed-growth-budget".to_owned())
            } else if after.allocations > before.allocations
                || after.loop_allocations > before.loop_allocations
            {
                Some("cost.allocation-regression".to_owned())
            } else {
                None
            };
            (reason.is_none(), reason, None, budget)
        }
        Profile::ReleaseSpeed => {
            let hot = after.loop_allocations < before.loop_allocations;
            let bounded = within_growth(after.min, before.min, SPEED_GROWTH_BPS)
                && within_growth(after.gzip, before.gzip, SPEED_GROWTH_BPS)
                && within_growth(after.brotli, before.brotli, SPEED_GROWTH_BPS);
            let reason = if !hot {
                Some("cost.hotness-evidence-missing".to_owned())
            } else if !bounded {
                Some("cost.speed-growth-budget".to_owned())
            } else {
                None
            };
            (
                reason.is_none(),
                reason,
                hot.then_some("final-js-loop-allocation"),
                budget,
            )
        }
    }
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

fn cache_key(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, crate::jir::decline::Decline>>,
    abi: &AbiPlan,
    profile: Profile,
    opts: &Opts,
) -> Hash128 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(profile.as_str().as_bytes());
    bytes.extend_from_slice(
        format!(
            "|{:?}|{:?}|{}|{}",
            opts.names, opts.modules, opts.minify, opts.minify_items
        )
        .as_bytes(),
    );
    for (name, item) in table {
        bytes.extend_from_slice(name.as_str().as_bytes());
        bytes.push(0);
        if let Some(summary) = &item.jir.summary {
            bytes.extend_from_slice(
                format!("{:?}|{:?}", summary.key.item, summary.semantic_hash).as_bytes(),
            );
        }
        bytes.extend_from_slice(&Hash128::of(item.js.as_bytes()).lo.to_le_bytes());
    }
    for (name, func) in funcs {
        bytes.extend_from_slice(name.as_str().as_bytes());
        bytes.extend_from_slice(format!("{func:?}").as_bytes());
    }
    abi.extend_cost_key(&mut bytes);
    Hash128::of(&bytes)
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
            .0
        );
        assert_eq!(
            decide(
                Profile::ReleaseSize,
                Some(before),
                Some(metrics(1001, 490, 390, 0))
            )
            .1
            .as_deref(),
            Some("cost.minified-no-improvement")
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
        assert!(accepted.0);
        assert_eq!(accepted.2, Some("final-js-loop-allocation"));
        assert_eq!(
            decide(
                Profile::ReleaseSpeed,
                Some(before),
                Some(metrics(990, 490, 390, 2)),
            )
            .1
            .as_deref(),
            Some("cost.hotness-evidence-missing")
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
}
