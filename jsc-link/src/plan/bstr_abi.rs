//! Phase 5A: Interprocedural read-only `js-str=bytes` specialization for `&[u8]` parameters.
//!
//! Under `Profile::ReleaseSpeed` + `js-str=bytes`, any immutable `&[u8]` parameter whose callee
//! body only reads length, indexes bytes, subslices, iterates, compares bytes, checks ASCII,
//! copies bytes out, converts back via `from_utf8_unchecked`, or forwards to other read-only
//! ByteStr callees can stay a JS byte string across calls — eliminating `$rt_bstr_bytes(s)`
//! `Uint8Array` allocations at parser/lexer boundaries (e.g. `marked`'s 25 block/inline helpers).
//!
//! # Two-Stage Soundness Architecture
//! 1. **Greatest-fixpoint callee safety** (`safe_full`, `safe_sub`): runs the exact
//!    `byte_view_analysis::analyze_with` engine (over scratch `opt::slice_iter`-canonicalized
//!    bodies, treating slice phis conservatively as `VarSlice`) to find the maximal closed set of
//!    `(callee, param_idx)` pairs whose parameters survive without escaping or mutating.
//! 2. **Least-fixpoint caller provenance**: runs `byte_view_analysis::analyze_with` across all
//!    lowered callers to propagate `&str`-backed `ByteView`s and lattice-join required modes
//!    (`Full ⊔ Full = Full`, `Full ⊔ Sub = Sub`).
//! 3. **In-place vs 1-clone (`$n*`) selection**:
//!    - Private non-addr-taken callees where 100% of direct call sites pass ByteViews specialize
//!      **in-place** (zero clones, zero code bloat).
//!    - Mixed-caller or exported/addr-taken callees receive at most **one** private clone with a
//!      fresh synthetic `ItemId` (`plan.synthetic_items`), and ONLY the exact ByteStr call sites
//!      are retargeted via `AbiPlan::add_site_retarget` (applied before `rewrite_callee_with` so
//!      block/op indices match 1-to-1).
//! 4. **Fail-closed Phase C verification**: `lower/js/function.rs` and `lower/js/call.rs` verify
//!    every `ByteStrParam` param and argument survived `byte_view_analysis` with the exact mode,
//!    declining cleanly into `physical_cost` rejection or lane retry if any mismatch ever occurs.

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::jir::decline::Decline;
use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::origin::OriginId;
use rcgjs_object::jir::repr::{Carrier, Func, Op, OpKind, RBlockId, ValueId};
use rcgjs_object::jir::semantic::{BodySemantic, Callee, DefinitionId, ItemId, TypeId};
use rcgjs_object::jir::types::{TypeKind, TypeTable};
use rcgjs_object::object::ItemKind;

use crate::item::{JsName, LinkItem, Linkage};
use crate::lower::js::byte_view_analysis::{analyze_with, is_immutable_byte_slice_pub};
use crate::lower::js::{ByteView, ByteViewMode};
use crate::opts::{OptimizerMode, Opts, Profile};
use crate::plan::abi_flat::{AbiPlan, ByteStrParam, FlatParam, FlatSig};
use crate::plan::names::NamePool;

/// Read-only program inputs handed to [`augment_plan`].
pub(crate) struct BstrAugmentCx<'a> {
    pub table: &'a BTreeMap<JsName, LinkItem>,
    pub funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
    pub opts: &'a Opts,
    pub excluded: &'a BTreeSet<ItemId>,
    pub ineligible: &'a BTreeSet<ItemId>,
    pub pinned_by_tracked_caller: &'a BTreeSet<ItemId>,
}

type ParamSiteSet = BTreeSet<(ItemId, usize)>;

struct Candidate<'a> {
    name: JsName,
    item: ItemId,
    func: &'a Func,
    interns: &'a Interns,
    usize_ty: TypeId,
    byte_slice_params: Vec<(usize, TypeId)>,
    abi_clone: Option<JsName>,
    in_place_eligible: bool,
}

struct ProbedFunc {
    scratch: Func,
    probe_plan: crate::lower::plan::Plan,
    /// Original `(CallSiteKey, callee_item)` indexed by stamped `op.origin.0`.
    orig_sites: Vec<(CallSiteKey, ItemId)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct CallSiteKey {
    block: RBlockId,
    op_idx: usize,
}

type SiteRecord = (JsName, CallSiteKey, bool);
type SelectedMap = BTreeMap<ItemId, BTreeMap<usize, ByteStrParam>>;

struct ConsistencySolution {
    selected: SelectedMap,
    in_place_set: BTreeSet<ItemId>,
    all_sites_by_callee: BTreeMap<ItemId, Vec<SiteRecord>>,
    canon_matches: BTreeMap<(JsName, CallSiteKey), ItemId>,
    clone_matches: BTreeMap<(ItemId, CallSiteKey), ItemId>,
}

/// Augments `plan` with interprocedural read-only `ByteStrParam` specializations under
/// `Profile::ReleaseSpeed` + `js-str=bytes`.
pub(crate) fn augment_plan(cx: BstrAugmentCx<'_>, pool: &mut NamePool, plan: &mut AbiPlan) {
    if cx.opts.profile != Some(Profile::ReleaseSpeed)
        || !crate::lower::bstr::enabled()
        || !cx.opts.chunks.is_empty()
    {
        return;
    }

    let mut planner = BstrPlanner::new(cx, plan);
    if planner.cands.is_empty() {
        return;
    }
    planner.probe_funcs();

    let (safe_sub, safe_full) = planner.solve_callee_safety_gfp();
    if safe_sub.is_empty() && safe_full.is_empty() {
        return;
    }

    let selected = planner.solve_demand_lfp(&safe_sub, &safe_full);
    if selected.is_empty() {
        return;
    }

    let solution = planner.solve_consistency_fixpoint(selected);
    if solution.selected.is_empty() {
        return;
    }

    planner.commit_to_plan(solution, pool, plan);
}

struct BstrPlanner<'a> {
    cx: BstrAugmentCx<'a>,
    name_to_item: BTreeMap<JsName, ItemId>,
    interns_map: BTreeMap<JsName, &'a Interns>,
    abi_clone_by_source: BTreeMap<JsName, JsName>,
    cands: BTreeMap<ItemId, Candidate<'a>>,
    probed: BTreeMap<JsName, ProbedFunc>,
}

impl<'a> BstrPlanner<'a> {
    fn new(cx: BstrAugmentCx<'a>, plan: &AbiPlan) -> Self {
        let mut planner = Self {
            cx,
            name_to_item: BTreeMap::new(),
            interns_map: BTreeMap::new(),
            abi_clone_by_source: BTreeMap::new(),
            cands: BTreeMap::new(),
            probed: BTreeMap::new(),
        };
        planner.collect_candidates(plan);
        planner
    }

    /// 1. Collect all lowered candidate callees that take at least one immutable `&[u8]` param.
    fn collect_candidates(&mut self, plan: &AbiPlan) {
        self.interns_map = self
            .cx
            .table
            .iter()
            .filter_map(|(name, item)| Some((name.clone(), item.jir.interns()?)))
            .collect();
        for (name, res) in self.cx.funcs {
            if let Ok(func) = res {
                self.name_to_item.insert(name.clone(), func.item);
            }
        }
        self.abi_clone_by_source = plan
            .clones()
            .iter()
            .map(|(clone, source)| (source.clone(), clone.clone()))
            .collect();

        for (name, res) in self.cx.funcs {
            let Ok(func) = res else { continue };
            if self.cx.excluded.contains(&func.item) || func.blocks.is_empty() {
                continue;
            }
            if func.sig.abi.spread.is_some() || func.sig.abi.caller_location {
                continue;
            }
            let Some(link_item) = self.cx.table.get(name) else {
                continue;
            };
            if link_item.kind == ItemKind::Import {
                continue;
            }
            let Some(interns) = link_item.jir.interns() else {
                continue;
            };
            let semantic = link_item.jir.body.as_ref().map(|b| b.semantic);
            if !bstr_semantic_allowed(semantic) {
                continue;
            }
            let types = &interns.types;
            let byte_slice_params: Vec<(usize, TypeId)> = func
                .params
                .iter()
                .enumerate()
                .filter_map(|(pos, &param)| {
                    let val = &func.values[param.0 as usize];
                    (val.carrier == Carrier::Baseline && is_immutable_byte_slice_pub(types, val.ty))
                        .then_some((pos, val.ty))
                })
                .collect();
            if byte_slice_params.is_empty() {
                continue;
            }
            let Some(usize_ty) = find_usize_ty(types) else {
                continue;
            };

            let abi_clone = self.abi_clone_by_source.get(name).cloned();
            let in_place_eligible = !self.cx.pinned_by_tracked_caller.contains(&func.item)
                && (abi_clone.is_some()
                    || (link_item.linkage == Linkage::internal()
                        && !self.cx.ineligible.contains(&func.item)));

            self.cands.insert(
                func.item,
                Candidate {
                    name: name.clone(),
                    item: func.item,
                    func,
                    interns,
                    usize_ty,
                    byte_slice_params,
                    abi_clone,
                    in_place_eligible,
                },
            );
        }
    }

    /// Pre-run the lane's intra-procedural probe optimizer on every candidate and direct caller,
    /// stamping each direct call to a candidate with its original `(block, op_idx)` in `op.origin`.
    fn probe_funcs(&mut self) {
        let mut probe_opts = self.cx.opts.clone();
        probe_opts.optimizer = OptimizerMode::On;
        for (name, res) in self.cx.funcs {
            let Ok(func) = res else { continue };
            let Some(&interns) = self.interns_map.get(name) else {
                continue;
            };
            let is_cand = self.cands.contains_key(&func.item);
            let calls_cand = func.blocks.iter().any(|b| {
                b.ops.iter().any(|op| {
                    matches!(
                        &op.kind,
                        OpKind::Call {
                            callee: Callee::Direct { item, .. },
                            host: None,
                            ..
                        } if self.cands.contains_key(item)
                    )
                })
            });
            if !is_cand && !calls_cand {
                continue;
            }
            let mut scratch = func.clone();
            let orig_sites = stamp_candidate_call_sites(&mut scratch, &self.cands);
            let mut stats = crate::opt::OptStats::default();
            if crate::opt::optimize_for_probe(
                &mut scratch,
                &interns.types,
                interns,
                &probe_opts,
                &mut stats,
            )
            .is_err()
            {
                // Fall back to stamped un-optimized copy + slice_iter if probe opt declined.
                scratch = func.clone();
                let _ = stamp_candidate_call_sites(&mut scratch, &self.cands);
                let _ = crate::opt::slice_iter::run(&mut scratch, &interns.types);
            }
            let probe_plan =
                crate::lower::fuse::plan(&scratch, None, &interns.types, interns, &probe_opts);
            self.probed.insert(
                name.clone(),
                ProbedFunc {
                    scratch,
                    probe_plan,
                    orig_sites,
                },
            );
        }
    }

    /// 2. Phase 1: Greatest fixpoint for callee safety (`safe_sub` and `safe_full`).
    fn solve_callee_safety_gfp(&self) -> (ParamSiteSet, ParamSiteSet) {
        let mut safe_sub: BTreeSet<(ItemId, usize)> = BTreeSet::new();
        let mut safe_full: BTreeSet<(ItemId, usize)> = BTreeSet::new();
        for cand in self.cands.values() {
            for &(pos, _) in &cand.byte_slice_params {
                safe_sub.insert((cand.item, pos));
                safe_full.insert((cand.item, pos));
            }
        }

        loop {
            let mut changed = false;
            let snap_sub = safe_sub.clone();
            let snap_full = safe_full.clone();
            let accepts = |op: &Op, arg_idx: usize| -> Option<ByteStrParam> {
                let OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    host: None,
                    ..
                } = &op.kind
                else {
                    return None;
                };
                if snap_sub.contains(&(*item, arg_idx)) {
                    Some(ByteStrParam::Sub)
                } else if snap_full.contains(&(*item, arg_idx)) {
                    Some(ByteStrParam::Full)
                } else {
                    None
                }
            };

            for cand in self.cands.values() {
                let cand_probed = &self.probed[&cand.name];
                let cand_scratch = &cand_probed.scratch;
                let cand_plan = Some(&cand_probed.probe_plan);

                for (mode, expected, snap, target) in [
                    (
                        ByteStrParam::Sub,
                        ByteViewMode::VarSlice,
                        &snap_sub,
                        &mut safe_sub,
                    ),
                    (
                        ByteStrParam::Full,
                        ByteViewMode::FullSlice,
                        &snap_full,
                        &mut safe_full,
                    ),
                ] {
                    let seeds: Vec<(usize, ByteStrParam)> = cand
                        .byte_slice_params
                        .iter()
                        .filter(|&&(pos, _)| snap.contains(&(cand.item, pos)))
                        .map(|&(pos, _)| (pos, mode))
                        .collect();
                    if seeds.is_empty() {
                        continue;
                    }
                    let views = analyze_with(
                        cand_scratch,
                        cand_plan,
                        &cand.interns.types,
                        cand.interns,
                        &seeds,
                        false,
                        &accepts,
                    );
                    for &(pos, _) in &seeds {
                        let param = cand_scratch.params[pos];
                        if views.get(&param).map(|v| v.mode) != Some(expected) {
                            target.remove(&(cand.item, pos));
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        (safe_sub, safe_full)
    }

    /// 3. Phase 2: Least fixpoint for caller provenance & mode (`Full` vs `Sub`).
    fn solve_demand_lfp(
        &self,
        safe_sub: &BTreeSet<(ItemId, usize)>,
        safe_full: &BTreeSet<(ItemId, usize)>,
    ) -> SelectedMap {
        let mut demand: BTreeMap<(ItemId, usize), ByteStrParam> = BTreeMap::new();
        let accepts_safe = |op: &Op, arg_idx: usize| -> Option<ByteStrParam> {
            let OpKind::Call {
                callee: Callee::Direct { item, .. },
                host: None,
                ..
            } = &op.kind
            else {
                return None;
            };
            if safe_sub.contains(&(*item, arg_idx)) {
                Some(ByteStrParam::Sub)
            } else if safe_full.contains(&(*item, arg_idx)) {
                Some(ByteStrParam::Full)
            } else {
                None
            }
        };

        loop {
            let mut changed = false;
            for (caller_name, probed_caller) in &self.probed {
                let caller_func = &probed_caller.scratch;
                if caller_func.sig.abi.caller_location {
                    continue;
                }
                let Some(&caller_interns) = self.interns_map.get(caller_name) else {
                    continue;
                };
                let caller_seeds: Vec<(usize, ByteStrParam)> = (0..caller_func.params.len())
                    .filter_map(|pos| {
                        demand
                            .get(&(caller_func.item, pos))
                            .map(|&mode| (pos, mode))
                    })
                    .collect();
                let views = analyze_with(
                    caller_func,
                    Some(&probed_caller.probe_plan),
                    &caller_interns.types,
                    caller_interns,
                    &caller_seeds,
                    false,
                    &accepts_safe,
                );
                if views.is_empty() {
                    continue;
                }
                for block in &caller_func.blocks {
                    for op in &block.ops {
                        let OpKind::Call {
                            callee: Callee::Direct { item: callee, .. },
                            host: None,
                            args,
                            ..
                        } = &op.kind
                        else {
                            continue;
                        };
                        if !self.cands.contains_key(callee) {
                            continue;
                        }
                        for (arg_idx, &arg) in args.iter().enumerate() {
                            let Some(bv) = views.get(&arg) else {
                                continue;
                            };
                            let key = (*callee, arg_idx);
                            let desired = match bv.mode {
                                ByteViewMode::FullSlice => {
                                    if safe_full.contains(&key) {
                                        ByteStrParam::Full
                                    } else if safe_sub.contains(&key) {
                                        ByteStrParam::Sub
                                    } else {
                                        continue;
                                    }
                                }
                                ByteViewMode::VarSlice => {
                                    if safe_sub.contains(&key) {
                                        ByteStrParam::Sub
                                    } else if safe_full.contains(&key) {
                                        ByteStrParam::Full
                                    } else {
                                        continue;
                                    }
                                }
                                ByteViewMode::ThinPtr => continue,
                            };
                            match demand.get_mut(&key) {
                                Some(cur) => {
                                    let joined = cur.join(desired);
                                    let next = if joined == ByteStrParam::Sub
                                        && !safe_sub.contains(&key)
                                    {
                                        ByteStrParam::Full
                                    } else {
                                        joined
                                    };
                                    if next != *cur {
                                        *cur = next;
                                        changed = true;
                                    }
                                }
                                None => {
                                    demand.insert(key, desired);
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        let mut selected: SelectedMap = BTreeMap::new();
        for ((item, pos), mode) in demand {
            selected.entry(item).or_default().insert(pos, mode);
        }
        selected
    }

    /// 4. Phase 3: Consistency fixpoint over selected callees, in-place vs clone decisions, and
    ///    matched call sites.
    fn solve_consistency_fixpoint(&self, mut selected: SelectedMap) -> ConsistencySolution {
        let all_sites_by_callee = self.collect_all_direct_sites();
        let mut in_place_set: BTreeSet<ItemId>;
        let mut canon_matches: BTreeMap<(JsName, CallSiteKey), ItemId> = BTreeMap::new();
        let mut clone_matches: BTreeMap<(ItemId, CallSiteKey), ItemId> = BTreeMap::new();

        loop {
            // 4a. Re-verify every selected callee's body under ONLY `selected` callees.
            let sel_snap = selected.clone();
            let accepts_sel = |op: &Op, arg_idx: usize| -> Option<ByteStrParam> {
                let OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    host: None,
                    ..
                } = &op.kind
                else {
                    return None;
                };
                sel_snap.get(item)?.get(&arg_idx).copied()
            };

            let mut to_remove_params: Vec<(ItemId, usize)> = Vec::new();
            for (&item, params) in &sel_snap {
                let cand = &self.cands[&item];
                let cand_probed = &self.probed[&cand.name];
                let cand_scratch = &cand_probed.scratch;
                let seeds: Vec<(usize, ByteStrParam)> =
                    params.iter().map(|(&p, &m)| (p, m)).collect();
                let views = analyze_with(
                    cand_scratch,
                    Some(&cand_probed.probe_plan),
                    &cand.interns.types,
                    cand.interns,
                    &seeds,
                    false,
                    &accepts_sel,
                );
                for &(pos, mode) in &seeds {
                    let param = cand_scratch.params[pos];
                    let expected = match mode {
                        ByteStrParam::Full => ByteViewMode::FullSlice,
                        ByteStrParam::Sub => ByteViewMode::VarSlice,
                    };
                    if views.get(&param).map(|v| v.mode) != Some(expected) {
                        to_remove_params.push((item, pos));
                    }
                }
            }
            if !to_remove_params.is_empty() {
                for (item, pos) in to_remove_params {
                    if let Some(map) = selected.get_mut(&item) {
                        map.remove(&pos);
                        if map.is_empty() {
                            selected.remove(&item);
                        }
                    }
                }
                continue;
            }

            // 4b. Compute matched call sites in canonical bodies (no seeded params) AND in
            // specialized bodies (with `selected[caller]` seeded).
            canon_matches.clear();
            clone_matches.clear();
            let mut matched_by_callee_canon: BTreeMap<ItemId, BTreeSet<(JsName, CallSiteKey)>> =
                BTreeMap::new();
            let mut matched_by_callee_spec: BTreeMap<ItemId, BTreeSet<(ItemId, CallSiteKey)>> =
                BTreeMap::new();

            for (caller_name, probed_caller) in &self.probed {
                let caller_func = &probed_caller.scratch;
                if caller_func.sig.abi.caller_location {
                    continue;
                }
                let Some(&caller_interns) = self.interns_map.get(caller_name) else {
                    continue;
                };
                let canon_views = analyze_with(
                    caller_func,
                    Some(&probed_caller.probe_plan),
                    &caller_interns.types,
                    caller_interns,
                    &[],
                    false,
                    &accepts_sel,
                );
                for (site, callee) in collect_matched_sites(probed_caller, &canon_views, &selected)
                {
                    canon_matches.insert((caller_name.clone(), site), callee);
                    matched_by_callee_canon
                        .entry(callee)
                        .or_default()
                        .insert((caller_name.clone(), site));
                }

                if let Some(caller_params) = selected.get(&caller_func.item) {
                    let seeds: Vec<(usize, ByteStrParam)> =
                        caller_params.iter().map(|(&p, &m)| (p, m)).collect();
                    let spec_views = analyze_with(
                        caller_func,
                        Some(&probed_caller.probe_plan),
                        &caller_interns.types,
                        caller_interns,
                        &seeds,
                        false,
                        &accepts_sel,
                    );
                    for (site, callee) in
                        collect_matched_sites(probed_caller, &spec_views, &selected)
                    {
                        clone_matches.insert((caller_func.item, site), callee);
                        matched_by_callee_spec
                            .entry(callee)
                            .or_default()
                            .insert((caller_func.item, site));
                    }
                }
            }

            // 4c. Maximal in-place set (greatest fixpoint).
            in_place_set = self.solve_in_place_gfp(
                &selected,
                &all_sites_by_callee,
                &matched_by_callee_canon,
                &matched_by_callee_spec,
            );

            // 4d. Prune any selected callee that is neither in-place nor has >= 1 matched site.
            let mut drop_callees = Vec::new();
            for &callee in selected.keys() {
                if in_place_set.contains(&callee) {
                    continue;
                }
                let has_canon_match = matched_by_callee_canon
                    .get(&callee)
                    .is_some_and(|s| !s.is_empty());
                let has_spec_match = matched_by_callee_spec
                    .get(&callee)
                    .is_some_and(|s| !s.is_empty());
                if !has_canon_match && !has_spec_match {
                    drop_callees.push(callee);
                }
            }
            if !drop_callees.is_empty() {
                for callee in drop_callees {
                    selected.remove(&callee);
                }
                continue;
            }
            break;
        }

        ConsistencySolution {
            selected,
            in_place_set,
            all_sites_by_callee,
            canon_matches,
            clone_matches,
        }
    }

    fn collect_all_direct_sites(&self) -> BTreeMap<ItemId, Vec<SiteRecord>> {
        let mut all_sites_by_callee: BTreeMap<ItemId, Vec<SiteRecord>> = BTreeMap::new();
        for (caller_name, res) in self.cx.funcs {
            let Ok(caller_func) = res else { continue };
            let tracked = caller_func.sig.abi.caller_location;
            for block in &caller_func.blocks {
                for (op_idx, op) in block.ops.iter().enumerate() {
                    if let OpKind::Call {
                        callee: Callee::Direct { item, .. },
                        host: None,
                        ..
                    } = &op.kind
                    {
                        if self.cands.contains_key(item) {
                            all_sites_by_callee.entry(*item).or_default().push((
                                caller_name.clone(),
                                CallSiteKey {
                                    block: block.id,
                                    op_idx,
                                },
                                tracked,
                            ));
                        }
                    }
                }
            }
        }
        all_sites_by_callee
    }

    fn solve_in_place_gfp(
        &self,
        selected: &SelectedMap,
        all_sites_by_callee: &BTreeMap<ItemId, Vec<SiteRecord>>,
        matched_by_callee_canon: &BTreeMap<ItemId, BTreeSet<(JsName, CallSiteKey)>>,
        matched_by_callee_spec: &BTreeMap<ItemId, BTreeSet<(ItemId, CallSiteKey)>>,
    ) -> BTreeSet<ItemId> {
        let mut in_place_set: BTreeSet<ItemId> = selected
            .keys()
            .copied()
            .filter(|id| self.cands[id].in_place_eligible)
            .collect();
        loop {
            let before = in_place_set.len();
            let snap_in_place = in_place_set.clone();
            in_place_set.retain(|&callee| {
                let Some(sites) = all_sites_by_callee.get(&callee) else {
                    return false;
                };
                if sites.is_empty() {
                    return false;
                }
                sites.iter().all(|(caller_name, site, tracked)| {
                    if *tracked {
                        return false;
                    }
                    let caller_item = self.name_to_item.get(caller_name).copied();
                    // A caller replaces its canonical body iff it is in-place AND did NOT keep a
                    // canonical copy for `abi_flat`.
                    if let Some(cid) = caller_item.filter(|id| {
                        snap_in_place.contains(id) && self.cands[id].abi_clone.is_none()
                    }) {
                        matched_by_callee_spec
                            .get(&callee)
                            .is_some_and(|s| s.contains(&(cid, *site)))
                    } else {
                        let canon_ok = matched_by_callee_canon
                            .get(&callee)
                            .is_some_and(|s| s.contains(&(caller_name.clone(), *site)));
                        let spec_ok = match caller_item {
                            Some(cid) if selected.contains_key(&cid) => matched_by_callee_spec
                                .get(&callee)
                                .is_some_and(|s| s.contains(&(cid, *site))),
                            _ => true,
                        };
                        canon_ok && spec_ok
                    }
                })
            });
            if in_place_set.len() == before {
                break;
            }
        }
        in_place_set
    }

    /// 5. Materialize selected in-place and cloned ByteStr specializations into `plan` via its
    ///    transactional mutation API.
    fn commit_to_plan(
        &self,
        solution: ConsistencySolution,
        pool: &mut NamePool,
        plan: &mut AbiPlan,
    ) {
        let ConsistencySolution {
            selected,
            in_place_set,
            all_sites_by_callee,
            canon_matches,
            clone_matches,
        } = solution;

        let mut clone_info: BTreeMap<ItemId, (ItemId, JsName)> = BTreeMap::new();
        let mut next_synth_hash: u64 = 1;
        let existing_items: BTreeSet<ItemId> = self
            .cx
            .funcs
            .values()
            .filter_map(|r| r.as_ref().ok().map(|f| f.item))
            .chain(plan.flat_map().keys().copied())
            .collect();

        // Register in-place and cloned callees.
        for (&callee, params) in &selected {
            let cand = &self.cands[&callee];
            let mut sig = plan
                .flat_map()
                .get(&callee)
                .cloned()
                .unwrap_or_else(|| FlatSig {
                    params: vec![None; cand.func.params.len()],
                    result: None,
                    spill: 0,
                    returns_out_field: false,
                });
            for (&pos, &mode) in params {
                let param_ty = cand.func.values[cand.func.params[pos].0 as usize].ty;
                sig.params[pos] = Some(FlatParam::byte_str_param(param_ty, cand.usize_ty, mode));
            }

            if in_place_set.contains(&callee) {
                plan.register_in_place_bstr(callee, &cand.name, sig);
            } else {
                let Ok(clone_name) = pool.alloc_native_version(cand.name.as_str()) else {
                    continue;
                };
                let synth_id = loop {
                    let id = ItemId {
                        definition: DefinitionId {
                            crate_id: u64::MAX - 1,
                            local_hash: next_synth_hash,
                        },
                        instantiation: 0,
                    };
                    next_synth_hash += 1;
                    if !existing_items.contains(&id) && !plan.flat_map().contains_key(&id) {
                        break id;
                    }
                };
                plan.register_clone_bstr(callee, &cand.name, clone_name.clone(), synth_id, sig);
                clone_info.insert(callee, (synth_id, clone_name));
            }
        }

        // 5a. Callers of in-place callees (canonical callers + pre-existing `abi_flat` clones +
        // ByteStr clones).
        for &callee in &in_place_set {
            if let Some(sites) = all_sites_by_callee.get(&callee) {
                for (caller_name, _, _) in sites {
                    plan.add_in_place_caller(callee, caller_name.clone());
                    if let Some(abi_clone) = self.abi_clone_by_source.get(caller_name) {
                        plan.add_in_place_caller(callee, abi_clone.clone());
                    }
                    if let Some(&caller_item) = self.name_to_item.get(caller_name) {
                        if let Some((_, caller_clone)) = clone_info.get(&caller_item) {
                            plan.add_in_place_caller(callee, caller_clone.clone());
                        }
                    }
                }
            }
        }

        // 5b. Canonical callers calling a cloned ByteStr callee at matched sites.
        for ((caller_name, site), callee) in &canon_matches {
            let Some(&(synth_id, ref clone_name)) = clone_info.get(callee) else {
                continue;
            };
            let caller_item = self.name_to_item.get(caller_name).copied();
            let replaced_canonical = caller_item.is_some_and(|id| {
                in_place_set.contains(&id) && self.cands[&id].abi_clone.is_none()
            });
            if replaced_canonical {
                continue;
            }
            plan.add_site_retarget(
                caller_name.clone(),
                site.block,
                site.op_idx,
                synth_id,
                clone_name.clone(),
            );

            // Mirror canonical site retargets onto a pre-existing `abi_flat` clone that is NOT
            // itself specialized by `bstr_abi`.
            if let Some(abi_clone) = self.abi_clone_by_source.get(caller_name) {
                let caller_is_bstr_in_place =
                    caller_item.is_some_and(|id| in_place_set.contains(&id));
                if !caller_is_bstr_in_place {
                    plan.add_site_retarget(
                        abi_clone.clone(),
                        site.block,
                        site.op_idx,
                        synth_id,
                        clone_name.clone(),
                    );
                }
            }
        }

        // 5c. Specialized callers calling a cloned ByteStr callee.
        for ((caller_item, site), callee) in &clone_matches {
            let Some(&(synth_id, ref clone_name)) = clone_info.get(callee) else {
                continue;
            };
            let caller_target_name = if in_place_set.contains(caller_item) {
                plan.target_name(*caller_item)
                    .cloned()
                    .unwrap_or_else(|| self.cands[caller_item].name.clone())
            } else if let Some((_, caller_clone)) = clone_info.get(caller_item) {
                caller_clone.clone()
            } else {
                continue;
            };
            plan.add_site_retarget(
                caller_target_name,
                site.block,
                site.op_idx,
                synth_id,
                clone_name.clone(),
            );
        }
    }
}

/// Stamps every direct call to a candidate in `scratch` with its sequential index in `op.origin`
/// (and `u32::MAX` on all other ops), returning the indexed `(CallSiteKey, callee_item)` table.
fn stamp_candidate_call_sites(
    scratch: &mut Func,
    cands: &BTreeMap<ItemId, Candidate<'_>>,
) -> Vec<(CallSiteKey, ItemId)> {
    let mut orig_sites = Vec::new();
    for block in &mut scratch.blocks {
        for (op_idx, op) in block.ops.iter_mut().enumerate() {
            if let OpKind::Call {
                callee: Callee::Direct { item, .. },
                host: None,
                ..
            } = &op.kind
            {
                if cands.contains_key(item) {
                    let site_idx = orig_sites.len() as u32;
                    orig_sites.push((
                        CallSiteKey {
                            block: block.id,
                            op_idx,
                        },
                        *item,
                    ));
                    op.origin = OriginId(site_idx);
                    continue;
                }
            }
            op.origin = OriginId(u32::MAX);
        }
    }
    orig_sites
}

/// Collects every original call site `(CallSiteKey, callee)` in `probed_caller` whose arguments
/// satisfy every ByteStr parameter required by `selected[callee]` under `views`.
fn collect_matched_sites(
    probed_caller: &ProbedFunc,
    views: &BTreeMap<ValueId, ByteView>,
    selected: &SelectedMap,
) -> Vec<(CallSiteKey, ItemId)> {
    if views.is_empty() {
        return Vec::new();
    }
    let mut site_ok: Vec<Option<bool>> = vec![None; probed_caller.orig_sites.len()];
    for block in &probed_caller.scratch.blocks {
        for op in &block.ops {
            let OpKind::Call {
                callee: Callee::Direct { item: callee, .. },
                host: None,
                args,
                ..
            } = &op.kind
            else {
                continue;
            };
            let Some(req_params) = selected.get(callee) else {
                continue;
            };
            let sidx = op.origin.0 as usize;
            let Some(&(_, orig_callee)) = probed_caller.orig_sites.get(sidx) else {
                continue;
            };
            if orig_callee != *callee {
                continue;
            }
            let all_match = req_params.keys().all(|&pos| {
                args.get(pos)
                    .and_then(|arg| views.get(arg))
                    .is_some_and(|bv| {
                        matches!(bv.mode, ByteViewMode::FullSlice | ByteViewMode::VarSlice)
                    })
            });
            site_ok[sidx] = Some(site_ok[sidx].unwrap_or(true) && all_match);
        }
    }
    site_ok
        .into_iter()
        .enumerate()
        .filter(|&(_, ok)| ok == Some(true))
        .map(|(sidx, _)| probed_caller.orig_sites[sidx])
        .collect()
}

fn bstr_semantic_allowed(semantic: Option<BodySemantic>) -> bool {
    !matches!(
        semantic,
        Some(
            BodySemantic::DropGlue(_)
                | BodySemantic::PanicImpl
                | BodySemantic::JsExport
                | BodySemantic::JsExportOut
        )
    ) && !matches!(
        semantic,
        Some(BodySemantic::Standard(operation))
            if crate::lower::semantic_ops::fmt_kernel::is_format_machinery(operation)
                || crate::lower::semantic_ops::num::is_float_from_str(operation)
                || matches!(
                    operation,
                    rcgjs_object::jir::semantic::StandardOperation::SliceSort
                        | rcgjs_object::jir::semantic::StandardOperation::SliceSortCmp
                )
                || operation.is_typed_heap()
                || operation.is_str()
    )
}

fn find_usize_ty(table: &TypeTable) -> Option<TypeId> {
    table.descriptors().iter().find_map(|d| {
        matches!(
            d.kind,
            TypeKind::Integer {
                signed: false,
                bits: 32
            }
        )
        .then_some(d.id)
    })
}
