//! Phase O (phase 5, D5): interprocedural summaries, cost-guided inlining, devirtualization,
//! projection cleanup, invoke pruning, and summary-driven pure-call DCE.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::callgraph::{self, Callgraph};
use crate::analysis::effects::{self, Summaries};
use crate::item::{write_program, JsName, LinkItem};
use crate::jir::decline::Decline;
use crate::jir::origin::OriginTable;
use crate::jir::repr::Func;
use crate::jir::summary::ItemSummary;
use crate::jir::Interns;
use crate::opt::inline::{InlineConfig, Metadata};
use crate::opt::pipeline::OPass;

use super::driver::{lower_input, Ctx};
use super::report::OGroup;

pub(super) fn merge_o_stats(dst: &mut OGroup, src: OGroup) {
    dst.inline.applied += src.inline.applied;
    dst.inline.cross_object_applied += src.inline.cross_object_applied;
    dst.inline.declined_budget += src.inline.declined_budget;
    for (reason, count) in src.inline.declined {
        *dst.inline.declined.entry(reason).or_default() += count;
    }
    dst.inline.bytes_before += src.inline.bytes_before;
    dst.inline.bytes_after += src.inline.bytes_after;
    dst.inline.items.extend(src.inline.items);
    dst.devirt.applied += src.devirt.applied;
    dst.devirt.witnesses_recorded += src.devirt.witnesses_recorded;
    dst.devirt.witnesses_failed += src.devirt.witnesses_failed;
    dst.devirt.items.extend(src.devirt.items);
    dst.dce_items.extend(src.dce_items);
    dst.witnesses_recorded += src.witnesses_recorded;
    dst.witnesses_failed += src.witnesses_failed;
}

/// Phase O (phase 5, D5): the interprocedural group over the whole `funcs` map.
///
/// Summaries first ([`crate::analysis::effects`]), then the inliner and the devirtualizer, then
/// `summary_check::enforce` demotes what the rewrites staled, then one dce run per func with the
/// pure-call oracle the surviving summaries answer (D7c). Every rewrite is verified and witness-
/// checked; a failure reverts that item's rewrite and the link goes on. `RCGJS_JIR_PASSES`
/// selects the group's members exactly as it does the per-item passes.
pub(super) fn o_group(
    table: &mut BTreeMap<JsName, LinkItem>,
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    ctx: &Ctx<'_>,
    live: &BTreeSet<JsName>,
) -> OGroup {
    o_group_with_mode(table, funcs, ctx, live, false)
}

/// Pass toggles and immutable configuration shared across one Phase O execution.
struct OGroupCx<'a> {
    ctx: &'a Ctx<'a>,
    live: &'a BTreeSet<JsName>,
    allow_std_post_repr: bool,
    run_inline: bool,
    run_devirt: bool,
    run_dce: bool,
}

/// Owned cross-object metadata edits produced by one inlining round.
struct MetadataUpdate {
    name: JsName,
    interns: Option<Interns>,
    origins: Option<Option<OriginTable>>,
    summary: Option<Option<ItemSummary>>,
}

/// Cached per-item measurements and rename map carried across O-group inlining rounds.
struct InlineRoundState {
    inline_renames: BTreeMap<String, String>,
    cached_sizes: BTreeMap<JsName, u64>,
    cached_refs: BTreeMap<JsName, BTreeSet<JsName>>,
    dirty_items: Option<BTreeSet<JsName>>,
}

impl InlineRoundState {
    fn new(table: &BTreeMap<JsName, LinkItem>, live: &BTreeSet<JsName>) -> Self {
        // Use the same reserved/public-name policy as final emission. One deterministic name map
        // is shared by every probe in this group, so symbol length cannot masquerade as a body win.
        let items: Vec<_> = live.iter().filter_map(|name| table.get(name)).collect();
        let (program, _) = write_program("", &items);
        let inline_renames = crate::minify::item_renames(&program, &items);
        Self {
            inline_renames,
            cached_sizes: BTreeMap::new(),
            cached_refs: BTreeMap::new(),
            dirty_items: None,
        }
    }

    /// Re-measures dirty (or all initial) functions, updating `cached_sizes`/`cached_refs`, and
    /// returns the total measured program byte budget denominator.
    fn measure_dirty(
        &mut self,
        table: &BTreeMap<JsName, LinkItem>,
        funcs: &BTreeMap<JsName, Result<Func, Decline>>,
        ctx: &Ctx<'_>,
    ) -> u64 {
        let mut program: u64 = 0;
        for (name, func) in funcs {
            let needs_measure = self
                .dirty_items
                .as_ref()
                .is_none_or(|dirty| dirty.contains(name));
            if needs_measure {
                let measured = func.as_ref().ok().and_then(|func| {
                    let item = table.get(name)?;
                    let input = lower_input(name, item, ctx, &[])?;
                    crate::plan::cost::measure_inline_item(&input, func, &self.inline_renames)
                });
                match measured {
                    Some((bytes, refs)) => {
                        self.cached_sizes.insert(name.clone(), bytes);
                        self.cached_refs.insert(name.clone(), refs);
                    }
                    None => {
                        self.cached_sizes.remove(name);
                        self.cached_refs.remove(name);
                    }
                }
            }
            if let Some(&bytes) = self.cached_sizes.get(name) {
                program += bytes;
            } else {
                program += table.get(name).map_or(0, |item| item.js.len() as u64);
            }
        }
        program
    }

    /// Recomputes live reachability and reference counts from post-O measured refs so the
    /// sole-referrer oracle does not stay anchored to stale pre-opt `item.refs`.
    fn compute_sole_referrers(
        &self,
        table: &BTreeMap<JsName, LinkItem>,
        live: &BTreeSet<JsName>,
        roots: &[String],
    ) -> BTreeMap<JsName, (u32, Option<JsName>)> {
        let mut effective_live = BTreeSet::new();
        let mut work: Vec<JsName> = table
            .iter()
            .filter(|(name, item)| {
                live.contains(*name)
                    && (item.linkage.root || roots.iter().any(|r| r == name.as_str()))
            })
            .map(|(name, _)| name.clone())
            .collect();
        while let Some(cur) = work.pop() {
            if !live.contains(&cur) || !effective_live.insert(cur.clone()) {
                continue;
            }
            if let Some(refs) = self.cached_refs.get(&cur) {
                work.extend(refs.iter().cloned());
            } else if let Some(item) = table.get(&cur) {
                work.extend(item.refs.iter().cloned());
            }
        }
        let mut referrers: BTreeMap<JsName, (u32, Option<JsName>)> = BTreeMap::new();
        for (name, item) in table {
            if !effective_live.contains(name) {
                continue;
            }
            let refs_set = self.cached_refs.get(name).unwrap_or(&item.refs);
            for target in refs_set {
                if target == name {
                    continue;
                }
                let entry = referrers.entry(target.clone()).or_insert((0, None));
                entry.0 += 1;
                entry.1 = Some(name.clone());
            }
        }
        referrers
    }

    /// Commits owned cross-object intern/origin/summary metadata updates back into `table`.
    fn apply_metadata_updates(
        table: &mut BTreeMap<JsName, LinkItem>,
        updates: Vec<MetadataUpdate>,
    ) {
        for update in updates {
            let item = table
                .get_mut(&update.name)
                .expect("inlined caller remains in the item table");
            if let Some(interns) = update.interns {
                item.jir.use_interns(interns);
                item.jir.origins = update.origins.flatten();
            }
            if let Some(Some(summary)) = update.summary {
                item.jir.summary = Some(summary);
            }
        }
    }

    /// Runs one inlining round, updating `group.inline` and `table` metadata, and returning the
    /// number of inlined sites plus the set of modified callers.
    fn run_inline_round(
        &mut self,
        table: &mut BTreeMap<JsName, LinkItem>,
        funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
        ocx: &OGroupCx<'_>,
        graph: &Callgraph,
        group: &mut OGroup,
    ) -> (u32, BTreeSet<JsName>) {
        let program = self.measure_dirty(table, funcs, ocx.ctx);
        let referrers = self.compute_sole_referrers(table, ocx.live, &ocx.ctx.opts.roots);
        let (applied, dirty, updates) = {
            let sole = |callee: &JsName, caller: &JsName| -> bool {
                table.get(callee).is_some_and(|item| !item.linkage.root)
                    && !ocx
                        .ctx
                        .opts
                        .roots
                        .iter()
                        .any(|root| root == callee.as_str())
                    && referrers
                        .get(callee)
                        .is_some_and(|(count, who)| *count == 1 && who.as_ref() == Some(caller))
            };
            let zombie_free =
                |name: &JsName| table.get(name).is_some_and(|item| item.zombies.is_empty());
            let mut metadata: BTreeMap<_, _> = table
                .iter()
                .filter_map(|(name, item)| {
                    let interns = item.jir.interns()?;
                    Some((
                        name.clone(),
                        Metadata {
                            universe: interns,
                            interns: Cow::Borrowed(interns),
                            origins: item.jir.origins.as_ref().map(Cow::Borrowed),
                            header: item.jir.header(),
                            summary: item.jir.summary.as_ref().map(Cow::Borrowed),
                        },
                    ))
                })
                .collect();
            let measure_imported = |name: &JsName, func: &Func, metadata: &Metadata<'_>| {
                let item = table.get(name)?;
                let mut input = lower_input(name, item, ocx.ctx, &[])?;
                input.interns = &metadata.interns;
                input.table = &metadata.interns.types;
                input.origins = metadata.origins.as_deref();
                if let Some(summary) = metadata.summary.as_deref() {
                    input.summary = summary;
                }
                crate::plan::cost::measure_inline_item(&input, func, &self.inline_renames)
            };
            let round_inline = crate::opt::inline::run_with_config(
                funcs,
                &mut metadata,
                InlineConfig {
                    graph,
                    items: ocx.ctx.items,
                    measure: &measure_imported,
                    sole_referrer: &sole,
                    zombie_free: &zombie_free,
                    program_bytes: program,
                    sizes: &self.cached_sizes,
                    allow_std_post_repr: ocx.allow_std_post_repr,
                    profile: ocx.ctx.opts.profile,
                    call_retargets: None,
                },
            );
            let applied = round_inline.applied;
            let dirty: BTreeSet<JsName> = round_inline.items.iter().cloned().collect();
            group.inline.applied += round_inline.applied;
            group.inline.cross_object_applied += round_inline.cross_object_applied;
            group.inline.declined_budget += round_inline.declined_budget;
            for (reason, count) in round_inline.declined {
                *group.inline.declined.entry(reason).or_default() += count;
            }
            group.inline.bytes_before += round_inline.bytes_before;
            group.inline.bytes_after += round_inline.bytes_after;
            group.inline.witnesses_recorded += round_inline.witnesses_recorded;
            group.inline.witnesses_failed += round_inline.witnesses_failed;
            group.inline.items.extend(round_inline.items);
            let updates = extract_metadata_updates(metadata);
            (applied, dirty, updates)
        };
        Self::apply_metadata_updates(table, updates);
        (applied, dirty)
    }
}

fn extract_metadata_updates(metadata: BTreeMap<JsName, Metadata<'_>>) -> Vec<MetadataUpdate> {
    metadata
        .into_iter()
        .filter_map(|(name, metadata)| {
            let interns_owned = matches!(metadata.interns, Cow::Owned(_));
            let summary_owned = matches!(metadata.summary, Some(Cow::Owned(_)));
            if !interns_owned && !summary_owned {
                return None;
            }
            Some(MetadataUpdate {
                name,
                interns: interns_owned.then(|| metadata.interns.into_owned()),
                origins: interns_owned.then(|| metadata.origins.map(Cow::into_owned)),
                summary: summary_owned.then(|| metadata.summary.map(Cow::into_owned)),
            })
        })
        .collect()
}

/// Runs post-inline invoke pruning, devirtualization, and projection cleanup for one O-group
/// round, refreshing `graph` when edges change.
fn run_post_inline_passes(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    ocx: &OGroupCx<'_>,
    round_inline_applied: u32,
    graph: &mut Callgraph,
    group: &mut OGroup,
) -> (bool, u32, BTreeSet<JsName>) {
    let mut dirty = BTreeSet::new();
    let had_invoke_before: BTreeSet<JsName> = if ocx.run_dce && round_inline_applied > 0 {
        funcs
            .iter()
            .filter_map(|(name, res)| {
                res.as_ref()
                    .ok()
                    .filter(|f| f.has_invoke())
                    .map(|_| name.clone())
            })
            .collect()
    } else {
        BTreeSet::new()
    };
    let pruned_post_inline = if ocx.run_dce && round_inline_applied > 0 {
        let pruned = crate::opt::prune_invoke::run_checked(table, funcs, ocx.live);
        group.witnesses_recorded += pruned.witnesses_recorded;
        group.witnesses_failed += pruned.witnesses_failed;
        if pruned.modified > 0 {
            for name in &had_invoke_before {
                if funcs
                    .get(name)
                    .and_then(|res| res.as_ref().ok())
                    .is_some_and(|f| !f.has_invoke())
                {
                    dirty.insert(name.clone());
                }
            }
            true
        } else {
            false
        }
    } else {
        false
    };
    let interns_map = collect_interns_map(table);
    if pruned_post_inline {
        *graph = callgraph::build(funcs, &interns_map);
        let summaries = effects::compute(funcs, &interns_map, graph);
        group.summaries_items = summaries.len() as u32;
    }
    let mut round_devirt_applied = 0u32;
    if ocx.run_devirt {
        let post_devirt =
            crate::opt::devirtualize::run(funcs, &interns_map, table, ocx.ctx.vtables);
        round_devirt_applied = post_devirt.applied;
        dirty.extend(post_devirt.items.iter().cloned());
        group.devirt.applied += post_devirt.applied;
        group.devirt.witnesses_recorded += post_devirt.witnesses_recorded;
        group.devirt.witnesses_failed += post_devirt.witnesses_failed;
        group.devirt.items.extend(post_devirt.items);
    }
    if ocx.run_dce {
        let projection = crate::opt::projection::run(funcs, &interns_map);
        dirty.extend(projection.items.iter().cloned());
        group.dce_items.extend(projection.items);
        group.witnesses_recorded += projection.witnesses_recorded;
        group.witnesses_failed += projection.witnesses_failed;
    }
    if round_inline_applied > 0 || round_devirt_applied > 0 {
        *graph = callgraph::build(funcs, &interns_map);
    }
    (pruned_post_inline, round_devirt_applied, dirty)
}

/// D7(c): runs whole-map DCE with the summary-driven pure-call oracle (`Const`/`Read`,
/// non-unwinding, zombie-free callees with unused results are deletable).
fn run_summary_pure_call_dce(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    interns_map: &BTreeMap<JsName, &Interns>,
    summaries: &Summaries,
    group: &mut OGroup,
) {
    let names: Vec<JsName> = funcs.keys().cloned().collect();
    for name in names {
        let Some(interns) = interns_map.get(&name).copied() else {
            continue;
        };
        let Some(Ok(func)) = funcs.get(&name) else {
            continue;
        };
        let oracle = |callee: &crate::jir::semantic::Callee| match callee {
            crate::jir::semantic::Callee::Direct { link_name, .. } => {
                // An item with recorded zombies owes its compile-time diagnostics: deleting the
                // call would drop the item and silence them, purity notwithstanding.
                let diagnosed = interns
                    .str(*link_name)
                    .and_then(|name| table.get(name))
                    .is_some_and(|item| !item.zombies.is_empty());
                !diagnosed
                    && summaries
                        .callee(interns, *link_name)
                        .is_some_and(|summary| {
                            summary.purity != effects::Purity::Effect && !summary.may_unwind
                        })
            }
            _ => false,
        };
        let options = crate::opt::dce::Options {
            no_constant_params: !OPass::Constprop.enabled(),
            no_deletions: false,
            pure_call: Some(&oracle),
        };
        let mut rewritten = func.clone();
        let mut witnesses = Vec::new();
        if !crate::opt::dce::run_with(&mut rewritten, &interns.types, &options, &mut witnesses) {
            continue;
        }
        group.witnesses_recorded += witnesses.len() as u32;
        let verified = rewritten
            .verify_carriers(&interns.types, crate::opt::observed_carriers(&rewritten))
            .is_ok();
        let checked = !crate::opt::witness::enabled()
            || crate::opt::witness::check_all(&witnesses, &rewritten, &interns.types).is_ok();
        if !verified || !checked {
            if verified {
                group.witnesses_failed += 1;
            }
            continue;
        }
        group.dce_items.insert(name.clone());
        funcs.insert(name, Ok(rewritten));
    }
}

fn collect_interns_map(table: &BTreeMap<JsName, LinkItem>) -> BTreeMap<JsName, &Interns> {
    table
        .iter()
        .filter_map(|(name, item)| Some((name.clone(), item.jir.interns()?)))
        .collect()
}

pub(super) fn o_group_with_mode(
    table: &mut BTreeMap<JsName, LinkItem>,
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    ctx: &Ctx<'_>,
    live: &BTreeSet<JsName>,
    allow_std_post_repr: bool,
) -> OGroup {
    let mut group = OGroup::default();
    let ocx = OGroupCx {
        ctx,
        live,
        allow_std_post_repr,
        run_inline: OPass::Inline.enabled(),
        run_devirt: OPass::Devirt.enabled(),
        run_dce: OPass::Dce.enabled(),
    };
    if !(ocx.run_inline || ocx.run_devirt || ocx.run_dce) {
        return group;
    }
    if ocx.run_dce {
        let pruned = crate::opt::prune_invoke::run_checked(table, funcs, live);
        group.witnesses_recorded += pruned.witnesses_recorded;
        group.witnesses_failed += pruned.witnesses_failed;
    }
    let mut graph = {
        let interns_map = collect_interns_map(table);
        if allow_std_post_repr
            && ocx.run_devirt
            && matches!(ctx.opts.profile, Some(crate::opts::Profile::ReleaseSpeed))
        {
            let pre_devirt = crate::opt::devirtualize::run(funcs, &interns_map, table, ctx.vtables);
            group.devirt.applied += pre_devirt.applied;
            group.devirt.witnesses_recorded += pre_devirt.witnesses_recorded;
            group.devirt.witnesses_failed += pre_devirt.witnesses_failed;
            group.devirt.items.extend(pre_devirt.items);
        }
        let graph = callgraph::build(funcs, &interns_map);
        let summaries = effects::compute(funcs, &interns_map, &graph);
        group.summaries_items = summaries.len() as u32;
        graph
    };

    let speed_post_repr =
        allow_std_post_repr && matches!(ctx.opts.profile, Some(crate::opts::Profile::ReleaseSpeed));
    let mut round_state = InlineRoundState::new(table, live);
    let max_rounds = if speed_post_repr && ocx.run_inline {
        2
    } else {
        1
    };
    for round in 0..max_rounds {
        let (round_inline_applied, mut round_dirty) = if ocx.run_inline {
            round_state.run_inline_round(table, funcs, &ocx, &graph, &mut group)
        } else {
            (0, BTreeSet::new())
        };
        let (pruned_post_inline, round_devirt_applied, post_dirty) = run_post_inline_passes(
            table,
            funcs,
            &ocx,
            round_inline_applied,
            &mut graph,
            &mut group,
        );
        round_dirty.extend(post_dirty);
        if round + 1 < max_rounds
            && (round_inline_applied > 0 || round_devirt_applied > 0 || pruned_post_inline)
        {
            round_state.dirty_items = Some(round_dirty);
            continue;
        }
        break;
    }

    let interns_map = collect_interns_map(table);
    // Recompute summaries directly after inline/devirt rewrites.
    let graph = callgraph::build(funcs, &interns_map);
    let summaries = effects::compute(funcs, &interns_map, &graph);
    group.summaries_validated = summaries.len() as u32;
    group.summaries_top = summaries.iter().filter(|(_, s)| s.is_top()).count() as u32;

    if ocx.run_dce {
        run_summary_pure_call_dce(table, funcs, &interns_map, &summaries, &mut group);
    }
    // The `RCGJS_REPR_DEBUG_FUNC` precedent, for the group's rewrites.
    if let Ok(filter) = std::env::var("RCGJS_O_DEBUG_FUNC") {
        for (name, func) in funcs.iter() {
            if name.as_str().contains(&filter) {
                if let Ok(func) = func {
                    eprintln!("o-func: {} = {func:#?}", name.as_str());
                }
            }
        }
    }
    group
}
