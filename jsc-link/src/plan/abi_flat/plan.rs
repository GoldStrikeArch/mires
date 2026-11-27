//! Whole-program flat direct-call ABI planning ([`AbiPlan`], [`plan_excluding`]).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::analysis::effects::{Purity, Summaries};
use crate::analysis::param_use::{self, ParamKind, ParamUse};
use crate::analysis::refs::{self, RefScan};
use crate::item::{JsName, LinkItem, Linkage};
use crate::jir::decline::{BoundaryDecline, Decline, LowerDecline, OpClass};
use crate::jir::repr::{Carrier, Func, OpKind, RBlockId, RTerm};
use crate::jir::semantic::{BodySemantic, Callee, ItemId, TypeId};
use crate::jir::summary::{self, ItemClass};
use crate::jir::types::TypeTable;
use crate::lane_report::push_string;
use crate::lower::js::emit::spill_name;
use crate::opt::candidates;
use crate::opts::{Opts, Profile};
use crate::plan::bstr_abi::{self, BstrAugmentCx};
use crate::plan::names::{synthesized_static_item, NamePool};
use crate::plan::physical_cost::HashFeed;

use super::rewrite::{rewrite_callee_with, rewrite_calls, unbox_scalar_ref_cells};
use super::shapes::{
    flat_param_with, flat_result_with, recursive_shape, scalar_leaf_op, scalar_leaf_result,
    sentinel_result_shape, shapes_agree, signature_shape, slice_param, tagged_shape, FlatParam,
    FlatParamKind, FlatResult, FlatResultKind, FlatSig, LaneComponent, ProductNode,
    SentinelResultShape,
};
use super::zero_offset::{specialize_zero_offset_slices, ZeroOffsetContext};

/// One aggregate-bearing item the physical ABI planner selected or left canonical.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AbiDecision {
    pub item: JsName,
    pub shape: Option<String>,
    pub selected: bool,
    pub reason: Option<String>,
    pub cross_crate: CrossCrateEvidence,
}

/// Summary and caller evidence consumed by one physical ABI decision.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CrossCrateEvidence {
    pub summary_available: bool,
    pub summary_validated: bool,
    pub body_decoded: bool,
    pub caller_count: u32,
    pub crosses_crate: bool,
    pub producer_crate: Option<String>,
    pub caller_crates: Vec<String>,
}

/// The whole plan: the callees whose ABI changed, the ones considered and refused, and how many
/// spill registers the program needs.
#[derive(Clone, Debug, Default)]
pub struct AbiPlan {
    pub(super) flat: BTreeMap<ItemId, FlatSig>,
    /// The JavaScript name of every item in `flat`, for the sidecar.
    pub(super) names: BTreeMap<ItemId, JsName>,
    pub(super) declined: BTreeMap<JsName, Decline>,
    pub(super) decisions: BTreeMap<JsName, AbiDecision>,
    /// Every item whose `Func` the plan rewrites: the flat callees and all of their callers.
    pub(super) touched: BTreeSet<JsName>,
    pub(super) registers: u32,
    /// The name of each register, register 1 first. Allocated against the program's own names
    /// (see [`allocate_spill_names`]), so it is `$rN` only when nothing else is called that.
    pub(super) spill_names: Vec<JsName>,
    /// Collision-free module-scope sentinel object (`const $se = {};`) allocated when at least
    /// one selected signature uses [`SentinelResultShape`].
    pub(super) sentinel_err: Option<JsName>,
    /// Private flat bodies that retain a canonical exported or indirect entry point.
    pub(super) clones: BTreeMap<JsName, JsName>,
    /// Synthetic `ItemId` assigned to mixed-caller private clones (e.g. `ByteStrParam` `$bs`
    /// clones) so the canonical body keeps its original `ItemId` while specific call sites target
    /// the specialized clone.
    pub(crate) synthetic_items: BTreeMap<JsName, ItemId>,
    /// Per-call-site `caller -> (block, op_idx) -> target_item` retargets applied at the start of
    /// [`AbiPlan::rewrite`] before `rewrite_callee_with` prepends entry ops.
    pub(super) site_retargets: BTreeMap<JsName, BTreeMap<(RBlockId, usize), ItemId>>,
    /// Direct callers retargeted from a canonical body to its private flat clone.
    pub(super) call_retargets: BTreeMap<JsName, crate::lower::CallRetargets>,
    /// Proven direct callers of each selected semantic item, used to keep exact-cost groups
    /// coupled where two ABI changes meet in one body.
    pub(super) callers: BTreeMap<ItemId, BTreeSet<JsName>>,
}

impl AbiPlan {
    /// Whether the plan makes the program no promises, which is what a retry falls back to.
    pub fn is_empty(&self) -> bool {
        self.flat.is_empty()
    }

    /// Whether any flattened signature in this plan scalarizes a slice parameter (`buf`, `off`, `len`
    /// or zero-offset `buf`, `len`).
    pub(crate) fn has_slice_param(&self) -> bool {
        self.flat
            .values()
            .any(|sig| sig.params.iter().flatten().any(FlatParam::is_slice))
    }

    /// Whether any flattened signature in this plan specializes a slice parameter (top-level or
    /// recursive product leaf) to zero-offset `(buf, len)`.
    pub(crate) fn has_off_zero_slice(&self) -> bool {
        self.flat.values().any(|sig| {
            sig.params.iter().flatten().any(|p| {
                p.is_off_zero_slice()
                    || p.recursive()
                        .is_some_and(|r| r.root.has_off_zero_slice_leaf())
            })
        })
    }

    /// Whether any flattened signature in this plan specializes a read-only byte-string `&[u8]`.
    pub(crate) fn has_byte_str_abi(&self) -> bool {
        self.flat
            .values()
            .any(|sig| sig.params.iter().flatten().any(|p| p.byte_str().is_some()))
    }

    /// Whether any flattened signature in this plan passes or returns a 64-bit integer as `(lo, hi)`.
    pub(crate) fn has_i64_pair_abi(&self) -> bool {
        self.flat.values().any(|sig| {
            sig.params.iter().flatten().any(FlatParam::is_i64_pair)
                || sig.result.as_ref().is_some_and(FlatResult::is_i64_pair)
        })
    }

    /// Whether any flattened signature in this plan uses [`SentinelResultShape`].
    pub(crate) fn has_sentinel_result(&self) -> bool {
        self.flat
            .values()
            .any(|sig| sig.result.as_ref().is_some_and(|r| r.sentinel().is_some()))
    }

    /// The allocated module-scope sentinel binding (`$se`) for [`SentinelResultShape`], if active.
    pub fn sentinel_err_name(&self) -> Option<&JsName> {
        self.sentinel_err.as_ref()
    }

    /// Read-only access to the flat signature map.
    pub(crate) fn flat_map(&self) -> &BTreeMap<ItemId, FlatSig> {
        &self.flat
    }

    /// Target body JS name currently assigned to `item`, if flattened.
    pub(crate) fn target_name(&self, item: ItemId) -> Option<&JsName> {
        self.names.get(&item)
    }

    /// Per-call-site retargets for `caller`, if any.
    pub(crate) fn caller_site_retargets(
        &self,
        caller: &JsName,
    ) -> Option<&BTreeMap<(RBlockId, usize), ItemId>> {
        self.site_retargets.get(caller)
    }

    /// Transactionally registers an in-place (or pre-existing ABI-clone) ByteStr specialization
    /// for `callee`, updating `decisions`, `names`, `flat`, and `touched`. Returns the target body
    /// name receiving the specialized signature.
    pub(crate) fn register_in_place_bstr(
        &mut self,
        callee: ItemId,
        source_name: &JsName,
        sig: FlatSig,
    ) -> JsName {
        self.select(source_name, &sig);
        let target_name = self
            .names
            .entry(callee)
            .or_insert_with(|| source_name.clone())
            .clone();
        self.flat.insert(callee, sig);
        self.touched.insert(target_name.clone());
        target_name
    }

    /// Transactionally registers a newly cloned ByteStr specialization for `source_item`, updating
    /// `decisions` (if `source_item` wasn't already flat), `clones`, `synthetic_items`, `names`,
    /// `flat`, and `touched`.
    pub(crate) fn register_clone_bstr(
        &mut self,
        source_item: ItemId,
        source_name: &JsName,
        clone_name: JsName,
        synth_id: ItemId,
        sig: FlatSig,
    ) {
        if !self.flat.contains_key(&source_item) {
            self.select(source_name, &sig);
        }
        self.clones.insert(clone_name.clone(), source_name.clone());
        self.synthetic_items.insert(clone_name.clone(), synth_id);
        self.names.insert(synth_id, clone_name.clone());
        self.flat.insert(synth_id, sig);
        self.touched.insert(clone_name);
    }

    /// Records `caller` as a direct caller of an in-place flattened `callee`.
    pub(crate) fn add_in_place_caller(&mut self, callee: ItemId, caller: JsName) {
        self.callers
            .entry(callee)
            .or_default()
            .insert(caller.clone());
        self.touched.insert(caller);
    }

    /// Transactionally registers a per-call-site retarget from `caller` at `(block, op_idx)` to
    /// synthetic clone `(synth_id, clone_name)`.
    pub(crate) fn add_site_retarget(
        &mut self,
        caller: JsName,
        block: RBlockId,
        op_idx: usize,
        synth_id: ItemId,
        clone_name: JsName,
    ) {
        self.site_retargets
            .entry(caller.clone())
            .or_default()
            .insert((block, op_idx), synth_id);
        self.call_retargets
            .entry(caller.clone())
            .or_default()
            .insert(synth_id, clone_name);
        self.callers
            .entry(synth_id)
            .or_default()
            .insert(caller.clone());
        self.touched.insert(caller);
    }

    /// Marks top-level slice parameter `param_pos` of `item` as zero-offset (`off === 0`).
    pub(super) fn mark_zero_offset_slice(&mut self, item: ItemId, param_pos: usize) -> bool {
        self.flat
            .get_mut(&item)
            .and_then(|sig| sig.params.get_mut(param_pos))
            .and_then(Option::as_mut)
            .is_some_and(FlatParam::set_off_zero_slice)
    }

    /// Narrows a direct slice leaf `field_idx` inside recursive parameter `param_pos` of `item`
    /// from `[Buffer, Offset, Length]` to `[Buffer, Length]`.
    pub(super) fn mark_zero_offset_recursive_field(
        &mut self,
        item: ItemId,
        param_pos: usize,
        field_idx: usize,
    ) -> bool {
        let Some(rec) = self
            .flat
            .get_mut(&item)
            .and_then(|sig| sig.params.get_mut(param_pos))
            .and_then(Option::as_mut)
            .and_then(FlatParam::recursive_mut)
        else {
            return false;
        };
        let ProductNode::Product { fields, .. } = &mut rec.root else {
            return false;
        };
        let Some(ProductNode::Leaf { components, .. }) = fields.get_mut(field_idx) else {
            return false;
        };
        if components.as_slice()
            == [
                LaneComponent::Buffer,
                LaneComponent::Offset,
                LaneComponent::Length,
            ]
        {
            *components = vec![LaneComponent::Buffer, LaneComponent::Length];
            return true;
        }
        false
    }

    /// Refreshes the sidecar decision shape for `item` after a post-pass signature update (e.g.
    /// zero-offset slice narrowing).
    pub(super) fn refresh_decision_for_item(
        &mut self,
        item: ItemId,
        funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    ) {
        let Some(target) = self.names.get(&item) else {
            return;
        };
        let source = self.clones.get(target).unwrap_or(target).clone();
        let is_synth = self.synthetic_items.values().any(|&id| id == item);
        let skip_decision = is_synth
            && funcs
                .get(&source)
                .and_then(|r| r.as_ref().ok())
                .is_some_and(|f| self.flat.contains_key(&f.item));
        if skip_decision {
            return;
        }
        if let Some(sig) = self.flat.get(&item).cloned() {
            self.select(&source, &sig);
        }
    }

    /// The plan a retry falls back to: no callee flattened, nothing touched, but the refusals
    /// still recorded, so the sidecar says what was considered even on a run that kept none of it.
    pub fn emptied(&self) -> AbiPlan {
        let decisions = self
            .decisions
            .iter()
            .map(|(name, decision)| {
                let mut decision = decision.clone();
                if decision.selected {
                    decision.selected = false;
                    decision.reason = Some("validation.plan-retry".to_owned());
                }
                (name.clone(), decision)
            })
            .collect();
        AbiPlan {
            declined: self.declined.clone(),
            decisions,
            ..AbiPlan::default()
        }
    }

    /// Feeds the complete physical label vector and private ABI choices into a structured binary
    /// hash for exact-cost caching (zero `Debug` formatting).
    pub(crate) fn extend_cost_key(&self, feed: &mut HashFeed) {
        feed.u8(0xA1);
        feed.usize(self.flat.len());
        for (&item, sig) in &self.flat {
            feed.item_id(item);
            sig.hash_into(feed);
        }
        feed.u8(0xA2);
        feed.usize(self.clones.len());
        for (clone, source) in &self.clones {
            feed.str(clone.as_str());
            feed.str(source.as_str());
        }
        feed.u8(0xA3);
        feed.usize(self.synthetic_items.len());
        for (clone, &item) in &self.synthetic_items {
            feed.str(clone.as_str());
            feed.item_id(item);
        }
        feed.u8(0xA4);
        feed.usize(self.site_retargets.len());
        for (caller, sites) in &self.site_retargets {
            feed.str(caller.as_str());
            feed.usize(sites.len());
            for (&(blk, idx), &item) in sites {
                feed.u32(blk.0);
                feed.usize(idx);
                feed.item_id(item);
            }
        }
        feed.u8(0xA5);
        feed.slice(&self.spill_names, |f, name| f.str(name.as_str()));
        feed.u8(0xA6);
        feed.opt(self.sentinel_err.as_ref(), |f, name| f.str(name.as_str()));
        feed.u8(0xA7);
        feed.usize(self.call_retargets.len());
        for (caller, retargets) in &self.call_retargets {
            feed.str(caller.as_str());
            feed.usize(retargets.len());
            for (&item, target) in retargets {
                feed.item_id(item);
                feed.str(target.as_str());
            }
        }
    }

    /// ABI-connected groups for exact economics, computed in O((V + E) α(V)) via union-find.
    ///
    /// Two signatures stay together when one calls the other or when they share a caller. The
    /// latter matters because result lanes and copy-back lanes share spill bindings in that body;
    /// pricing either side alone would omit their interaction.
    pub(crate) fn economic_groups(&self) -> Vec<BTreeSet<ItemId>> {
        let items: Vec<ItemId> = self.flat.keys().copied().collect();
        let n = items.len();
        if n == 0 {
            return Vec::new();
        }

        let mut parent: Vec<usize> = (0..n).collect();
        let find = |parent: &mut [usize], mut x: usize| -> usize {
            while parent[x] != x {
                let p = parent[x];
                parent[x] = parent[p];
                x = p;
            }
            x
        };
        let union = |parent: &mut [usize], a: usize, b: usize| {
            let ra = find(parent, a);
            let rb = find(parent, b);
            if ra != rb {
                if ra < rb {
                    parent[rb] = ra;
                } else {
                    parent[ra] = rb;
                }
            }
        };

        // Map each flat item's source and target JS names to its index so caller -> callee edges
        // resolve in O(log V).
        let mut by_name: BTreeMap<&JsName, Vec<usize>> = BTreeMap::new();
        for (idx, &item) in items.iter().enumerate() {
            if let Some(target) = self.names.get(&item) {
                by_name.entry(target).or_default().push(idx);
                if let Some(source) = self.clones.get(target) {
                    by_name.entry(source).or_default().push(idx);
                }
            }
        }

        // Union items sharing a caller (`caller_owner`) and caller -> callee pairs (`by_name`).
        let mut caller_owner: BTreeMap<&JsName, usize> = BTreeMap::new();
        for (idx, &item) in items.iter().enumerate() {
            let Some(item_callers) = self.callers.get(&item) else {
                continue;
            };
            for caller in item_callers {
                if let Some(&prev) = caller_owner.get(caller) {
                    union(&mut parent, prev, idx);
                } else {
                    caller_owner.insert(caller, idx);
                }
                if let Some(owners) = by_name.get(caller) {
                    for &owner_idx in owners {
                        union(&mut parent, owner_idx, idx);
                    }
                }
            }
        }

        let mut components: BTreeMap<usize, BTreeSet<ItemId>> = BTreeMap::new();
        for (idx, &item) in items.iter().enumerate() {
            let root = find(&mut parent, idx);
            components.entry(root).or_default().insert(item);
        }
        components.into_values().collect()
    }

    /// The executable part of this plan restricted to `selected` semantic items.
    pub(crate) fn subset(&self, selected: &BTreeSet<ItemId>) -> AbiPlan {
        let flat: BTreeMap<ItemId, FlatSig> = self
            .flat
            .iter()
            .filter(|(item, _)| selected.contains(item))
            .map(|(item, sig)| (*item, sig.clone()))
            .collect();
        let names: BTreeMap<ItemId, JsName> = self
            .names
            .iter()
            .filter(|(item, _)| selected.contains(item))
            .map(|(item, name)| (*item, name.clone()))
            .collect();
        let targets: BTreeSet<JsName> = names.values().cloned().collect();
        let clones = self
            .clones
            .iter()
            .filter(|(clone, _)| targets.contains(*clone))
            .map(|(clone, source)| (clone.clone(), source.clone()))
            .collect();
        let synthetic_items = self
            .synthetic_items
            .iter()
            .filter(|(clone, _)| targets.contains(*clone))
            .map(|(clone, item)| (clone.clone(), *item))
            .collect();
        let site_retargets = self
            .site_retargets
            .iter()
            .filter_map(|(caller, sites)| {
                let retained: BTreeMap<(RBlockId, usize), ItemId> = sites
                    .iter()
                    .filter(|(_, item)| selected.contains(item))
                    .map(|(&site, &item)| (site, item))
                    .collect();
                (!retained.is_empty()).then(|| (caller.clone(), retained))
            })
            .collect();
        let call_retargets = self
            .call_retargets
            .iter()
            .filter_map(|(caller, retargets)| {
                let retained: crate::lower::CallRetargets = retargets
                    .iter()
                    .filter(|(item, _)| selected.contains(item))
                    .map(|(item, target)| (*item, target.clone()))
                    .collect();
                (!retained.is_empty()).then(|| (caller.clone(), retained))
            })
            .collect();
        let callers: BTreeMap<ItemId, BTreeSet<JsName>> = self
            .callers
            .iter()
            .filter(|(item, _)| selected.contains(item))
            .map(|(item, callers)| (*item, callers.clone()))
            .collect();
        let mut touched = targets;
        touched.extend(callers.values().flatten().cloned());
        let registers = flat.values().map(|sig| sig.spill).max().unwrap_or(0);
        let spill_names = self
            .spill_names
            .iter()
            .take(registers as usize)
            .cloned()
            .collect();
        let has_sentinel = flat
            .values()
            .any(|sig| sig.result.as_ref().is_some_and(|r| r.sentinel().is_some()));
        let sentinel_err = has_sentinel.then(|| self.sentinel_err.clone()).flatten();
        AbiPlan {
            flat,
            names,
            declined: self.declined.clone(),
            decisions: self.decisions.clone(),
            touched,
            registers,
            spill_names,
            sentinel_err,
            clones,
            synthetic_items,
            site_retargets,
            call_retargets,
            callers,
        }
    }

    /// Applies the accepted economic groups and records one terminal reason for every rejected
    /// legal signature while retaining all summary and boundary evidence.
    pub(crate) fn with_economic_selection(
        &self,
        selected: &BTreeSet<ItemId>,
        reasons: &BTreeMap<ItemId, String>,
    ) -> AbiPlan {
        let mut plan = self.subset(selected);
        let mut by_source: BTreeMap<JsName, Vec<ItemId>> = BTreeMap::new();
        for &item in self.flat.keys() {
            if let Some(source) = self.source_name(item) {
                by_source.entry(source.clone()).or_default().push(item);
            }
        }
        plan.decisions = self
            .decisions
            .iter()
            .map(|(name, decision)| {
                let mut decision = decision.clone();
                if decision.selected {
                    let items = by_source
                        .get(name)
                        .expect("every selected decision has a flat signature");
                    decision.selected = items.iter().any(|item| selected.contains(item));
                    decision.reason = (!decision.selected).then(|| {
                        let first = items[0];
                        terminal_reason(
                            reasons
                                .get(&first)
                                .map(String::as_str)
                                .unwrap_or("validation.physical-measurement"),
                        )
                    });
                }
                (name.clone(), decision)
            })
            .collect();
        plan
    }

    pub(crate) fn source_name(&self, item: ItemId) -> Option<&JsName> {
        let target = self.names.get(&item)?;
        Some(self.clones.get(target).unwrap_or(target))
    }

    fn consider_mut(
        &mut self,
        name: &JsName,
        default_reason: impl Into<String>,
    ) -> &mut AbiDecision {
        self.decisions
            .entry(name.clone())
            .or_insert_with(|| AbiDecision {
                item: name.clone(),
                shape: None,
                selected: false,
                reason: Some(default_reason.into()),
                cross_crate: CrossCrateEvidence::default(),
            })
    }

    fn record_evidence(&mut self, name: &JsName, evidence: CrossCrateEvidence) {
        self.consider_mut(name, "not-considered.no-physical-shape")
            .cross_crate = evidence;
    }

    fn record_callers(
        &mut self,
        table: &BTreeMap<JsName, LinkItem>,
        name: &JsName,
        callers: &BTreeSet<JsName>,
    ) {
        let Some(decision) = self.decisions.get_mut(name) else {
            return;
        };
        let producer = table.get(name).map(|item| item.crate_name.as_str());
        let caller_crates: BTreeSet<String> = callers
            .iter()
            .filter_map(|caller| table.get(caller))
            .map(|item| item.crate_name.clone())
            .collect();
        decision.cross_crate.caller_count = callers.len().min(u32::MAX as usize) as u32;
        decision.cross_crate.crosses_crate =
            producer.is_some_and(|producer| caller_crates.iter().any(|caller| caller != producer));
        decision.cross_crate.caller_crates = caller_crates.into_iter().collect();
    }

    fn summary_validated(&self, name: &JsName) -> bool {
        self.decisions
            .get(name)
            .is_some_and(|decision| decision.cross_crate.summary_validated)
    }

    fn record_reason(&mut self, name: &JsName, reason: impl Into<String>) {
        let reason = terminal_reason(&reason.into());
        let decision = self.consider_mut(name, reason.clone());
        decision.selected = false;
        decision.reason = Some(reason);
    }

    fn decline(&mut self, name: JsName, decline: Decline) {
        self.record_reason(&name, decline.code());
        self.declined.insert(name, decline);
    }

    pub(super) fn select(&mut self, name: &JsName, sig: &FlatSig) {
        let decision = self.consider_mut(name, "not-considered.no-physical-shape");
        decision.shape = Some(signature_shape(sig));
        decision.selected = true;
        decision.reason = None;
    }

    /// Whether the plan changes `name`'s own signature or a call it makes.
    pub fn touches(&self, name: &JsName) -> bool {
        self.touched.contains(name)
    }

    /// The names the registers were allocated, register 1 first.
    pub fn spill_names(&self) -> &[JsName] {
        &self.spill_names
    }

    /// The selected signature of a direct call.
    pub(crate) fn call_signature(&self, item: ItemId) -> Option<&FlatSig> {
        self.flat.get(&item)
    }

    /// The selected signature when `name` is the body whose physical declaration changes.
    pub(crate) fn target_signature(&self, name: &JsName, item: ItemId) -> Option<&FlatSig> {
        (self.names.get(&item) == Some(name))
            .then(|| self.flat.get(&item))
            .flatten()
    }

    #[cfg(test)]
    pub(crate) fn for_target_test(item: ItemId, name: JsName, sig: FlatSig) -> Self {
        let mut plan = Self::default();
        plan.flat.insert(item, sig);
        plan.names.insert(item, name);
        plan
    }

    #[cfg(test)]
    pub(crate) fn with_sentinel_err_test(mut self, name: JsName) -> Self {
        self.sentinel_err = Some(name);
        self
    }

    /// Direct-call retargets selected for `caller`.
    pub(crate) fn call_retargets(&self, caller: &JsName) -> &crate::lower::CallRetargets {
        self.call_retargets
            .get(caller)
            .unwrap_or(crate::lower::EMPTY_CALL_RETARGETS)
    }

    /// The private clone names and their canonical sources.
    pub(crate) fn clones(&self) -> &BTreeMap<JsName, JsName> {
        &self.clones
    }

    /// Adds the non-emitting placeholders and JIR functions for private flat clones.
    pub(crate) fn install_clones(
        &self,
        table: &mut BTreeMap<JsName, LinkItem>,
        funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    ) {
        for (clone, source) in &self.clones {
            let Some(item) = table.get(source).cloned() else {
                continue;
            };
            let mut cloned_item = crate::plan::native_types::clone_link_item(item, clone);
            if let Some(&synth_id) = self.synthetic_items.get(clone) {
                if let Some(body) = cloned_item.jir.body.as_mut() {
                    body.item = synth_id;
                }
            }
            table.insert(clone.clone(), cloned_item);
            let Some(mut func) = funcs.get(source).cloned() else {
                continue;
            };
            if let (Ok(f), Some(&synth_id)) = (&mut func, self.synthetic_items.get(clone)) {
                f.item = synth_id;
            }
            funcs.insert(clone.clone(), func);
        }
    }

    /// The synthesized spill-register items, one module-scope `let` each, plus the optional
    /// sentinel error token (`const $se = {};`).
    pub fn spill_items(&self) -> Vec<LinkItem> {
        let mut items: Vec<LinkItem> = self
            .spill_names
            .iter()
            .map(|name| {
                synthesized_static_item(
                    name.clone(),
                    format!("\nlet {};\n", name.as_str()),
                    "flat direct-call ABI spill register",
                )
            })
            .collect();
        if let Some(name) = self.sentinel_err.clone() {
            items.push(synthesized_static_item(
                name.clone(),
                format!("\nconst {} = {{}};\n", name.as_str()),
                "flat direct-call ABI sentinel error token",
            ));
        }
        items
    }

    /// The sidecar's `abi` block, or `None` when there was nothing to plan.
    pub fn json(&self, applied: bool) -> Option<String> {
        if self.flat.is_empty() && self.declined.is_empty() {
            return None;
        }
        let mut rows: Vec<(&JsName, &FlatSig)> = self
            .flat
            .iter()
            .filter_map(|(item, sig)| Some((self.names.get(item)?, sig)))
            .collect();
        rows.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        let mut out = String::new();
        let _ = write!(
            out,
            "{{\"applied\":{applied},\"registers\":{},\"flat\":[",
            self.registers
        );
        for (index, (name, sig)) in rows.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"item\":");
            push_string(&mut out, name.as_str());
            out.push_str(",\"params\":[");
            for (position, param) in sig.params.iter().enumerate() {
                if position > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{}", param.as_ref().map_or(0, FlatParam::fields));
            }
            let _ = write!(
                out,
                "],\"result\":{},\"spill\":{},\"canonical_wrapper\":{}}}",
                sig.result.as_ref().map_or(0, FlatResult::fields),
                sig.spill,
                self.clones.contains_key(*name),
            );
        }
        out.push_str("],\"declined\":[");
        for (index, (name, decline)) in self.declined.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"item\":");
            push_string(&mut out, name.as_str());
            out.push_str(",\"reason\":");
            push_string(&mut out, decline.code());
            out.push('}');
        }
        out.push_str("]}");
        Some(out)
    }

    /// Rewrites every flat callee's signature and then every caller's calls, in place.
    pub fn rewrite(
        &self,
        funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
        table: &BTreeMap<JsName, LinkItem>,
    ) {
        if self.flat.is_empty() {
            return;
        }
        // Apply per-call-site retargets (mixed-caller ByteStr clones) BEFORE `rewrite_callee_with`
        // prepends entry ops into block 0, so `(block_id, op_idx)` indices match `plan_excluding`
        // 1-to-1.
        if !self.site_retargets.is_empty() {
            for (name, entry) in funcs.iter_mut() {
                let Some(sites) = self.site_retargets.get(name) else {
                    continue;
                };
                let Ok(func) = entry else { continue };
                for block in &mut func.blocks {
                    for (op_idx, op) in block.ops.iter_mut().enumerate() {
                        if let Some(&target_item) = sites.get(&(block.id, op_idx)) {
                            if let OpKind::Call {
                                callee: Callee::Direct { item, .. },
                                ..
                            } = &mut op.kind
                            {
                                *item = target_item;
                            }
                        }
                    }
                }
            }
        }
        for (name, entry) in funcs.iter_mut() {
            let Ok(func) = entry else { continue };
            if self.names.get(&func.item) == Some(name) {
                if let Some(sig) = self.flat.get(&func.item) {
                    rewrite_callee_with(func, sig, type_table(table, name));
                }
            }
        }
        for (name, entry) in funcs.iter_mut() {
            if !self.touched.contains(name) {
                continue;
            }
            let Ok(func) = entry else { continue };
            let types = type_table(table, name);
            rewrite_calls(func, self, types);
            let Some(types) = types else {
                continue;
            };
            unbox_scalar_ref_cells(func, types);
            if let Err(err) = func.verify_carriers(types, crate::opt::observed_carriers(func)) {
                *entry = Err(Decline::Lower(LowerDecline::Other {
                    op: OpClass::Region,
                    detail: format!("plan/abi_flat: {err}"),
                }));
            }
        }
    }
}

/// The type table of the object `name` was read from.
pub(super) fn type_table<'a>(
    table: &'a BTreeMap<JsName, LinkItem>,
    name: &JsName,
) -> Option<&'a TypeTable> {
    table
        .get(name)
        .and_then(|item| item.jir.interns())
        .map(|interns| &interns.types)
}

/// Validates the eager physical summary against the decoded body before a planner may consume it.
fn physical_evidence(
    table: &BTreeMap<JsName, LinkItem>,
    name: &JsName,
) -> (CrossCrateEvidence, Result<(), &'static str>) {
    let Some(item) = table.get(name) else {
        return (
            CrossCrateEvidence::default(),
            Err("validation.summary-item"),
        );
    };
    let mut evidence = CrossCrateEvidence {
        body_decoded: item.jir.body.is_some(),
        producer_crate: Some(item.crate_name.clone()),
        ..CrossCrateEvidence::default()
    };
    let Some(index) = item.jir.summary.as_ref() else {
        return (evidence, Err("validation.summary-missing"));
    };
    let Some(physical) = index.physical.as_ref() else {
        return (evidence, Err("validation.summary-missing"));
    };
    evidence.summary_available = true;
    let Some(body) = item.jir.body.as_ref() else {
        return (evidence, Err("validation.summary-body-unavailable"));
    };
    let Some(interns) = item.jir.interns() else {
        return (evidence, Err("validation.summary-interns-unavailable"));
    };
    let semantic_hash = summary::semantic_hash(body, interns);
    match physical.validate(body, &interns.types, semantic_hash) {
        Ok(()) => {
            evidence.summary_validated = true;
            (evidence, Ok(()))
        }
        Err(validation) => (evidence, Err(validation.code())),
    }
}

struct ProgramCallScan {
    scan: RefScan,
    callers: BTreeMap<ItemId, BTreeSet<JsName>>,
    result_used: BTreeSet<ItemId>,
    pinned_by_tracked_caller: BTreeSet<ItemId>,
    invoke_call_sites: BTreeSet<ItemId>,
    ineligible: BTreeSet<ItemId>,
}

/// Plans the flat ABI while retaining the baseline signature of `excluded` items.
pub fn plan_excluding(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    opts: &Opts,
    excluded: &BTreeSet<ItemId>,
) -> AbiPlan {
    let mut plan = AbiPlan::default();
    if !opts.flat_abi || !opts.analyzes(crate::opts::OptimizerPass::Sroa) {
        return plan;
    }
    let speed_profile = matches!(opts.profile, Some(Profile::ReleaseSpeed));
    let allow_sentinel = speed_profile && opts.chunks.is_empty();

    record_physical_summaries(table, funcs, allow_sentinel, &mut plan);
    let call_scan = scan_program_refs_and_callers(table, funcs, excluded, &mut plan);
    let potential_params = build_potential_params(table, funcs);
    let effects = compute_program_effects(table, funcs);

    let mut candidates = build_candidates(
        &CandidateBuildCx {
            table,
            funcs,
            opts,
            excluded,
            allow_sentinel,
            call_scan: &call_scan,
            potential_params: &potential_params,
            effects: &effects,
        },
        &mut plan,
    );
    prune_isolated_tagged(&mut candidates, speed_profile);

    let flat = close_over_forwards(&candidates);
    let mut pool = NamePool::from_table(table);
    materialize_flat_candidates(
        candidates,
        &flat,
        &call_scan.callers,
        &call_scan.result_used,
        &mut pool,
        &mut plan,
    );

    // Phase 5A: Interprocedural read-only ByteStr `&[u8]` physical specialization under
    // `ReleaseSpeed` + `js-str=bytes`. Runs before `inherit_clone_retargets` so ByteStr clones
    // inherit any regular `abi_flat` direct-call retargets automatically.
    bstr_abi::augment_plan(
        BstrAugmentCx {
            table,
            funcs,
            opts,
            excluded,
            ineligible: &call_scan.ineligible,
            pinned_by_tracked_caller: &call_scan.pinned_by_tracked_caller,
        },
        &mut pool,
        &mut plan,
    );

    // Phase 3C: Whole-program zero-offset slice specialization (`off === 0`) under `ReleaseSpeed`.
    if speed_profile {
        specialize_zero_offset_slices(
            ZeroOffsetContext {
                table,
                funcs,
                excluded,
                non_call_named: &call_scan.scan.named,
                ineligible: &call_scan.ineligible,
                pinned_by_tracked_caller: &call_scan.pinned_by_tracked_caller,
                chunked: !opts.chunks.is_empty(),
            },
            &mut plan,
        );
    }

    inherit_clone_retargets(&mut plan);
    finalize_generated_names(&mut pool, plan)
}

fn record_physical_summaries(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    allow_sentinel: bool,
    plan: &mut AbiPlan,
) {
    for (name, func) in funcs {
        let aggregate_body = table
            .get(name)
            .and_then(|item| item.jir.summary.as_ref())
            .is_some_and(|summary| summary.class.contains(ItemClass::AGGREGATE));
        let item_is_standard = matches!(
            table
                .get(name)
                .and_then(|item| item.jir.body.as_ref())
                .map(|body| body.semantic),
            Some(BodySemantic::Standard(_))
        );
        let aggregate_signature = func.as_ref().ok().is_some_and(|func| {
            type_table(table, name).is_some_and(|types| {
                func.params.iter().any(|param| {
                    let ty = func.values[param.0 as usize].ty;
                    slice_param(func, types, *param, ty).is_some()
                        || flat_param_with(Some(func), types, ty, true).is_some()
                }) || flat_result_with(types, func.sig.result, allow_sentinel && !item_is_standard)
                    .is_some()
            })
        });
        if !aggregate_body && !aggregate_signature {
            continue;
        }
        let (evidence, validation) = physical_evidence(table, name);
        plan.record_evidence(name, evidence);
        match func {
            Ok(_) => match validation {
                Ok(()) => {
                    plan.consider_mut(name, "not-considered.no-physical-shape");
                }
                Err(reason) => plan.record_reason(name, reason),
            },
            Err(decline) => plan.record_reason(name, decline.code()),
        }
    }
}

fn scan_program_refs_and_callers(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    excluded: &BTreeSet<ItemId>,
    plan: &mut AbiPlan,
) -> ProgramCallScan {
    let lowered: BTreeMap<ItemId, &JsName> = funcs
        .iter()
        .filter_map(|(name, func)| Some((func.as_ref().ok()?.item, name)))
        .collect();
    let by_name: BTreeMap<&str, ItemId> = lowered
        .iter()
        .map(|(item, name)| (name.as_str(), *item))
        .collect();

    let mut scan = RefScan::default();
    scan.named
        .extend(refs::named_by_unlowered(table, funcs, &by_name));
    let mut callers: BTreeMap<ItemId, BTreeSet<JsName>> = BTreeMap::new();
    let mut result_used: BTreeSet<ItemId> = BTreeSet::new();
    let mut pinned_by_tracked_caller: BTreeSet<ItemId> = BTreeSet::new();
    let mut invoke_call_sites: BTreeSet<ItemId> = BTreeSet::new();

    for (name, func) in funcs {
        let Ok(func) = func else { continue };
        let types = type_table(table, name);
        for block in &func.blocks {
            let last_op = block.ops.len().saturating_sub(1);
            for (op_index, op) in block.ops.iter().enumerate() {
                refs::scan_op(func, op, types, &mut scan);
                if let OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    host: None,
                    ..
                } = &op.kind
                {
                    callers.entry(*item).or_default().insert(name.clone());
                    if op.result.is_some() {
                        result_used.insert(*item);
                    }
                    if func.sig.abi.caller_location {
                        pinned_by_tracked_caller.insert(*item);
                    }
                    if matches!(block.term, RTerm::Invoke { .. }) && op_index == last_op {
                        invoke_call_sites.insert(*item);
                    }
                }
            }
        }
    }

    for (item, item_callers) in &callers {
        if let Some(name) = lowered.get(item) {
            plan.record_callers(table, name, item_callers);
        }
    }

    let mut ineligible = scan.ineligible();
    ineligible.extend(excluded.iter().copied());

    // A lowered item whose baseline text names a candidate without a modeled JIR direct call
    // keeps that candidate ineligible so the two lowerings cannot disagree.
    for (name, item) in table {
        if !funcs.get(name).is_some_and(Result::is_ok) {
            continue;
        }
        for referenced in &item.refs {
            let Some(id) = by_name.get(referenced.as_str()) else {
                continue;
            };
            if !callers.get(id).is_some_and(|set| set.contains(name)) {
                ineligible.insert(*id);
            }
        }
    }

    ProgramCallScan {
        scan,
        callers,
        result_used,
        pinned_by_tracked_caller,
        invoke_call_sites,
        ineligible,
    }
}

fn build_potential_params(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
) -> BTreeMap<ItemId, Vec<Option<FlatParam>>> {
    funcs
        .iter()
        .filter_map(|(name, func)| {
            let func = func.as_ref().ok()?;
            let types = type_table(table, name)?;
            let shapes = func
                .params
                .iter()
                .map(|param| {
                    if func.values[param.0 as usize].carrier != Carrier::Baseline {
                        return None;
                    }
                    let ty = func.values[param.0 as usize].ty;
                    slice_param(func, types, *param, ty)
                        .or_else(|| flat_param_with(Some(func), types, ty, true))
                })
                .collect();
            Some((func.item, shapes))
        })
        .collect()
}

fn compute_program_effects(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
) -> Summaries {
    let interns = table
        .iter()
        .filter_map(|(name, item)| item.jir.interns().map(|interns| (name.clone(), interns)))
        .collect();
    let graph = crate::analysis::callgraph::build(funcs, &interns);
    let has_custom_allocator = table.values().any(|item| {
        matches!(
            item.allocator_provider,
            crate::item::AllocatorProvider::Custom | crate::item::AllocatorProvider::Conflict
        )
    });
    crate::analysis::effects::compute_with_allocator(funcs, &interns, &graph, has_custom_allocator)
}

struct EvaluatedResult {
    result: Option<FlatResult>,
    forwards: BTreeSet<(ItemId, usize)>,
    forward_sites: u32,
    hot_forward: bool,
    reason: Option<&'static str>,
}

struct CandidateBuildCx<'a> {
    table: &'a BTreeMap<JsName, LinkItem>,
    funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
    opts: &'a Opts,
    excluded: &'a BTreeSet<ItemId>,
    allow_sentinel: bool,
    call_scan: &'a ProgramCallScan,
    potential_params: &'a BTreeMap<ItemId, Vec<Option<FlatParam>>>,
    effects: &'a Summaries,
}

fn evaluate_result_candidate(
    cx: &CandidateBuildCx<'_>,
    func: &Func,
    types: &TypeTable,
    allow_sentinel: bool,
) -> EvaluatedResult {
    let mut out = EvaluatedResult {
        result: None,
        forwards: BTreeSet::new(),
        forward_sites: 0,
        hot_forward: false,
        reason: None,
    };
    let Some(candidate) = flat_result_with(types, func.sig.result, allow_sentinel) else {
        return out;
    };
    if !returns_baseline(func, candidate.ty) {
        out.reason = Some("legality.result-carrier");
        return out;
    }
    let Some(direct_callers) = cx
        .call_scan
        .callers
        .get(&func.item)
        .filter(|set| !set.is_empty())
    else {
        out.reason = Some("not-considered.no-direct-caller");
        return out;
    };

    let mut observed_field = false;
    let plain_speed_result =
        cx.opts.profile == Some(Profile::ReleaseSpeed) && candidate.is_aggregate();
    let mut accepted_callers = 0usize;
    let mut rejected_callers = 0usize;

    for caller in direct_callers {
        match caller_accepts_result(
            cx.table,
            cx.funcs,
            caller,
            func.item,
            &candidate,
            cx.potential_params,
        ) {
            Some(observed) => {
                accepted_callers += 1;
                observed_field |= observed.read || !observed.forwards.is_empty();
                out.forwards.extend(observed.forwards);
                out.forward_sites += observed.forward_sites;
                out.hot_forward |= observed.hot_forward;
            }
            None if plain_speed_result && cx.funcs.get(caller).is_some_and(Result::is_ok) => {
                rejected_callers += 1;
            }
            None => {
                out.reason = Some("boundary.result-use");
                return out;
            }
        }
    }
    if rejected_callers > 0 && accepted_callers < rejected_callers {
        out.reason = Some("boundary.result-use");
        return out;
    }
    if !observed_field {
        out.reason = Some("not-considered.result-unused");
        return out;
    }
    out.result = Some(candidate);
    out
}

fn build_candidates(cx: &CandidateBuildCx<'_>, plan: &mut AbiPlan) -> BTreeMap<ItemId, Candidate> {
    let CandidateBuildCx {
        table,
        funcs,
        opts,
        excluded,
        allow_sentinel,
        call_scan,
        potential_params,
        effects,
    } = *cx;
    let mut candidates: BTreeMap<ItemId, Candidate> = BTreeMap::new();
    for (name, func) in funcs {
        let Ok(func) = func else { continue };
        if !plan.summary_validated(name) {
            continue;
        }
        let Some(types) = type_table(table, name) else {
            continue;
        };
        let mut shapes = potential_params
            .get(&func.item)
            .cloned()
            .unwrap_or_else(|| vec![None; func.params.len()]);
        let item = table.get(name);
        let semantic = item
            .and_then(|item| item.jir.body.as_ref())
            .map(|body| body.semantic);
        let item_is_standard = matches!(semantic, Some(BodySemantic::Standard(_)));

        let eval_res =
            evaluate_result_candidate(cx, func, types, allow_sentinel && !item_is_standard);
        if shapes.iter().all(Option::is_none) && eval_res.result.is_none() {
            if let Some(reason) = eval_res.reason {
                plan.record_reason(name, reason);
            }
            continue;
        }

        let refusal = refuses(
            item.map(|item| &item.linkage),
            semantic,
            func,
            !opts.chunks.is_empty(),
            &call_scan.ineligible,
        )
        .or_else(|| {
            call_scan
                .pinned_by_tracked_caller
                .contains(&func.item)
                .then_some(BoundaryDecline::TrackedCaller)
        });
        let canonical = refusal.is_some_and(|reason| {
            matches!(
                reason,
                BoundaryDecline::Exported | BoundaryDecline::NonCallReference
            ) && canonical_clone_allowed(
                semantic,
                func,
                !opts.chunks.is_empty(),
                excluded.contains(&func.item),
                call_scan.pinned_by_tracked_caller.contains(&func.item),
            ) && call_scan
                .callers
                .get(&func.item)
                .is_some_and(|callers| !callers.is_empty())
        });
        if let Some(reason) = refusal.filter(|_| !canonical) {
            plan.decline(name.clone(), Decline::Boundary(reason));
            continue;
        }

        let kinds: Vec<Option<ParamKind>> = shapes
            .iter()
            .map(|shape| {
                shape.as_ref().and_then(|shape| match &shape.kind {
                    FlatParamKind::I64Pair | FlatParamKind::ByteStr { .. } => None,
                    FlatParamKind::Slice { .. } => Some(ParamKind::Slice),
                    FlatParamKind::MutEnum { .. } => Some(ParamKind::ByRefEnum),
                    FlatParamKind::ScalarRef { .. } => Some(ParamKind::ByRefScalar),
                    FlatParamKind::Aggregate { .. } if shape.by_ref() => Some(ParamKind::ByRef),
                    FlatParamKind::Aggregate { .. }
                    | FlatParamKind::Tagged(_)
                    | FlatParamKind::Recursive(_) => Some(ParamKind::ByValue),
                })
            })
            .collect();
        let uses = param_use::summarize(func, types, &kinds);

        let read_only_callee = effects
            .usable(name)
            .is_some_and(|summary| summary.purity <= Purity::Read);
        let entry_targeted = func
            .blocks
            .iter()
            .any(|block| block.term.successors().contains(&RBlockId(0)));
        for (shape, use_) in shapes.iter_mut().zip(&uses) {
            let unflattened_shared_scalar = shape
                .as_ref()
                .is_some_and(|shape| shape.is_scalar_ref() && !shape.mutable())
                && (!read_only_callee || use_.mutated);
            let unflattened_loop_entry_by_ref = entry_targeted
                && shape
                    .as_ref()
                    .is_some_and(|shape| shape.is_scalar_ref() || shape.is_mut_enum());
            if unflattened_shared_scalar || unflattened_loop_entry_by_ref {
                *shape = None;
            }
        }

        let copy_back_unsafe = !effects
            .usable(name)
            .is_some_and(|summary| !summary.may_unwind)
            || call_scan.invoke_call_sites.contains(&func.item);
        if copy_back_unsafe {
            let mut dropped_written_mut_ref = false;
            for (shape, use_) in shapes.iter_mut().zip(&uses) {
                if shape
                    .as_ref()
                    .is_some_and(|shape| shape.by_ref() && shape.mutable())
                    && use_.written
                {
                    *shape = None;
                    dropped_written_mut_ref = true;
                }
            }
            if dropped_written_mut_ref
                && shapes.iter().all(Option::is_none)
                && eval_res.result.is_none()
            {
                plan.decline(
                    name.clone(),
                    Decline::Boundary(BoundaryDecline::ExceptionalCopyBack),
                );
                continue;
            }
        }

        let has_mut_enum = shapes
            .iter()
            .any(|shape| shape.as_ref().is_some_and(FlatParam::is_mut_enum));
        let has_non_sentinel_result = eval_res
            .result
            .as_ref()
            .is_some_and(|result| result.sentinel().is_none());
        if (has_non_sentinel_result || has_mut_enum)
            && call_scan.invoke_call_sites.contains(&func.item)
        {
            plan.decline(
                name.clone(),
                Decline::Boundary(BoundaryDecline::NonCallReference),
            );
            continue;
        }

        let tagged_leaf_consumer = scalar_leaf_result(types, func.sig.result)
            && func
                .blocks
                .iter()
                .all(|block| block.ops.iter().all(|op| scalar_leaf_op(types, func, op)));
        for (shape, use_) in shapes.iter_mut().zip(&uses) {
            if shape
                .as_ref()
                .is_some_and(|shape| shape.tagged().is_some() || shape.recursive().is_some())
                && use_.mutated
            {
                *shape = None;
                continue;
            }
            if let Some(shape) = shape {
                shape.set_written(use_.written);
            }
        }

        if !call_scan
            .callers
            .get(&func.item)
            .into_iter()
            .flatten()
            .all(|caller| caller_agrees(table, caller, &shapes))
        {
            plan.decline(
                name.clone(),
                Decline::Boundary(BoundaryDecline::NonCallReference),
            );
            continue;
        }

        candidates.insert(
            func.item,
            Candidate {
                name: name.clone(),
                returns_zst: types.is_zst(func.sig.result),
                uses,
                shapes,
                result: eval_res.result,
                result_forwards: eval_res.forwards,
                result_forward_sites: eval_res.forward_sites,
                result_hot_forward: eval_res.hot_forward,
                tagged_leaf_consumer,
                canonical,
            },
        );
    }
    candidates
}

fn prune_isolated_tagged(candidates: &mut BTreeMap<ItemId, Candidate>, speed_profile: bool) {
    let tagged = tagged_chain_members(candidates);
    for (item, candidate) in candidates.iter_mut() {
        let keep_speed_isolated_result = speed_profile && candidate.result_forwards.is_empty();
        if candidate
            .result
            .as_ref()
            .is_some_and(|result| result.tagged().is_some())
            && !tagged.results.contains(item)
            && !keep_speed_isolated_result
        {
            candidate.result = None;
            candidate.result_forwards.clear();
        }
        for (position, shape) in candidate.shapes.iter_mut().enumerate() {
            if shape.as_ref().is_some_and(|shape| shape.tagged().is_some())
                && !tagged.params.contains(&(*item, position))
            {
                *shape = None;
            }
        }
    }
}

fn materialize_flat_candidates(
    candidates: BTreeMap<ItemId, Candidate>,
    flat: &BTreeMap<ItemId, Vec<bool>>,
    callers: &BTreeMap<ItemId, BTreeSet<JsName>>,
    result_used: &BTreeSet<ItemId>,
    pool: &mut NamePool,
    plan: &mut AbiPlan,
) {
    for (item, mut candidate) in candidates {
        if !result_forwards_survive(&candidate.result_forwards, flat) {
            candidate.result = None;
        }
        let params: Vec<Option<FlatParam>> = candidate
            .shapes
            .into_iter()
            .zip(&flat[&item])
            .map(|(shape, keep)| shape.filter(|_| *keep))
            .collect();
        if params.iter().all(Option::is_none) && candidate.result.is_none() {
            plan.decline(
                candidate.name,
                Decline::Boundary(BoundaryDecline::ParamEscapes),
            );
            continue;
        }
        let mut sig = FlatSig {
            params,
            result: candidate.result,
            spill: 0,
            returns_out_field: false,
        };
        let outs = sig.out_fields().len() as u32;
        sig.returns_out_field = sig.result.is_none()
            && outs > 0
            && candidate.returns_zst
            && !result_used.contains(&item);
        sig.spill = sig.result_spills() + outs - u32::from(sig.returns_out_field);
        plan.registers = plan.registers.max(sig.spill);
        let target = if candidate.canonical {
            if let Err(decline) = crate::plan::versions::budget(1) {
                plan.decline(candidate.name, decline);
                continue;
            }
            let clone = match pool.alloc_native_version(candidate.name.as_str()) {
                Ok(clone) => clone,
                Err(decline) => {
                    plan.decline(candidate.name, decline);
                    continue;
                }
            };
            plan.clones.insert(clone.clone(), candidate.name.clone());
            for caller in callers.get(&item).into_iter().flatten() {
                plan.call_retargets
                    .entry(caller.clone())
                    .or_default()
                    .insert(item, clone.clone());
            }
            clone
        } else {
            candidate.name.clone()
        };
        plan.touched.insert(target.clone());
        plan.touched
            .extend(callers.get(&item).into_iter().flatten().cloned());
        if let Some(item_callers) = callers.get(&item) {
            plan.callers.insert(item, item_callers.clone());
        }
        plan.select(&candidate.name, &sig);
        plan.names.insert(item, target);
        plan.flat.insert(item, sig);
    }
}

fn finalize_generated_names(pool: &mut NamePool, mut plan: AbiPlan) -> AbiPlan {
    let mut name_collision = false;
    match allocate_spill_names_from_pool(plan.registers, pool) {
        Some(names) => plan.spill_names = names,
        None => name_collision = true,
    }
    if !name_collision && plan.has_sentinel_result() {
        match pool.alloc_suffixed("$se") {
            Some(name) => plan.sentinel_err = Some(name),
            None => name_collision = true,
        }
    }
    if !name_collision {
        return plan;
    }
    let mut declined = plan.declined;
    let mut decisions = plan.decisions;
    for name in plan.names.into_values() {
        let decline = Decline::Boundary(BoundaryDecline::GeneratedNameCollision);
        if let Some(decision) = decisions.get_mut(&name) {
            decision.selected = false;
            decision.reason = Some(terminal_reason(decline.code()));
        }
        declined.insert(name, decline);
    }
    for decision in decisions.values_mut().filter(|decision| decision.selected) {
        decision.selected = false;
        decision.reason = Some(terminal_reason(
            Decline::Boundary(BoundaryDecline::GeneratedNameCollision).code(),
        ));
    }
    AbiPlan {
        declined,
        decisions,
        ..AbiPlan::default()
    }
}

/// Maps the compiler-wide decline vocabulary onto the physical report's terminal-state contract.
pub(super) fn terminal_reason(reason: &str) -> String {
    let category = reason.split('.').next().unwrap_or_default();
    let terminal = match category {
        "selected" | "not-considered" | "legality" | "boundary" | "cost" | "lower"
        | "validation" => return reason.to_owned(),
        "body" | "type" | "observe" | "op" | "kernel" | "std" | "drop" | "format" | "iterator"
        | "async" => "legality",
        "encode" => "lower",
        _ => "validation",
    };
    format!("{terminal}.{reason}")
}

/// Copies a canonical source's already selected direct-call targets to its private body.
pub(super) fn inherit_clone_retargets(plan: &mut AbiPlan) {
    for (clone, source) in plan.clones.clone() {
        let Some(retargets) = plan.call_retargets.get(&source).cloned() else {
            continue;
        };
        let entry = plan.call_retargets.entry(clone).or_default();
        for (item, target) in retargets {
            entry.entry(item).or_insert(target);
        }
    }
}

fn allocate_spill_names_from_pool(registers: u32, pool: &mut NamePool) -> Option<Vec<JsName>> {
    let mut names = Vec::with_capacity(registers as usize);
    for register in 1..=registers {
        names.push(pool.alloc_suffixed(&spill_name(register))?);
    }
    Some(names)
}

#[cfg(test)]
pub(super) fn allocate_spill_names(
    registers: u32,
    table: &BTreeMap<JsName, LinkItem>,
) -> Option<Vec<JsName>> {
    if registers == 0 {
        return Some(Vec::new());
    }
    let mut pool = NamePool::from_table(table);
    allocate_spill_names_from_pool(registers, &mut pool)
}

/// One callee the plan is still considering.
pub(super) struct Candidate {
    pub(super) name: JsName,
    pub(super) returns_zst: bool,
    pub(super) shapes: Vec<Option<FlatParam>>,
    pub(super) result: Option<FlatResult>,
    /// Direct-call parameter lanes that must survive for the result to remain field-only.
    pub(super) result_forwards: BTreeSet<(ItemId, usize)>,
    /// Number of static producer-call sites whose result crosses one of those edges.
    pub(super) result_forward_sites: u32,
    /// Whether any such producer call is in a cyclic CFG region.
    pub(super) result_hot_forward: bool,
    /// Whether this function consumes a tagged parameter without another call/product boundary.
    pub(super) tagged_leaf_consumer: bool,
    pub(super) uses: Vec<ParamUse>,
    pub(super) canonical: bool,
}

/// Tagged result and leaf-parameter sites belonging to a whole-value direct-call edge.
#[derive(Default)]
pub(super) struct TaggedChains {
    pub(super) results: BTreeSet<ItemId>,
    pub(super) params: BTreeSet<(ItemId, usize)>,
}

/// Finds the minimal tagged ABI subgraph rooted at a returned value forwarded into a call.
pub(super) fn tagged_chain_members(candidates: &BTreeMap<ItemId, Candidate>) -> TaggedChains {
    let mut eligible: BTreeSet<(ItemId, usize)> = candidates
        .iter()
        .flat_map(|(item, candidate)| {
            candidate
                .shapes
                .iter()
                .enumerate()
                .filter_map(move |(position, shape)| {
                    let use_ = candidate.uses.get(position)?;
                    (shape.as_ref().is_some_and(|shape| shape.tagged().is_some())
                        && !use_.escapes
                        && !use_.mutated
                        && (candidate.tagged_leaf_consumer || !use_.forwards.is_empty()))
                    .then_some((*item, position))
                })
        })
        .collect();
    loop {
        let before = eligible.len();
        let snapshot = eligible.clone();
        eligible.retain(|(item, position)| {
            let candidate = &candidates[item];
            let use_ = &candidate.uses[*position];
            candidate.tagged_leaf_consumer
                || (!use_.forwards.is_empty()
                    && use_.forwards.iter().all(|target| snapshot.contains(target)))
        });
        if eligible.len() == before {
            break;
        }
    }

    let mut reaches_leaf: BTreeSet<(ItemId, usize)> = eligible
        .iter()
        .copied()
        .filter(|(item, _)| candidates[item].tagged_leaf_consumer)
        .collect();
    loop {
        let before = reaches_leaf.len();
        for key in &eligible {
            let use_ = &candidates[&key.0].uses[key.1];
            if !use_.forwards.is_empty()
                && use_.forwards.iter().all(|target| eligible.contains(target))
                && use_
                    .forwards
                    .iter()
                    .any(|target| reaches_leaf.contains(target))
            {
                reaches_leaf.insert(*key);
            }
        }
        if reaches_leaf.len() == before {
            break;
        }
    }

    let mut chains = TaggedChains::default();
    let mut pending = Vec::new();
    for (item, candidate) in candidates {
        if candidate
            .result
            .as_ref()
            .is_some_and(|result| result.tagged().is_some())
            && !candidate.result_forwards.is_empty()
            && (candidate.result_forward_sites >= 2 || candidate.result_hot_forward)
            && candidate
                .result_forwards
                .iter()
                .all(|target| reaches_leaf.contains(target))
        {
            chains.results.insert(*item);
            pending.extend(candidate.result_forwards.iter().copied());
        }
    }

    while let Some((item, position)) = pending.pop() {
        let Some(candidate) = candidates.get(&item) else {
            continue;
        };
        if !reaches_leaf.contains(&(item, position)) || !chains.params.insert((item, position)) {
            continue;
        }
        if !candidate.tagged_leaf_consumer {
            pending.extend(candidate.uses[position].forwards.iter().copied());
        }
    }
    chains
}

/// Whether an exported or indirect canonical entry point may coexist with one private flat body.
fn canonical_clone_allowed(
    semantic: Option<BodySemantic>,
    func: &Func,
    chunked: bool,
    excluded: bool,
    tracked_caller: bool,
) -> bool {
    !chunked
        && !excluded
        && !tracked_caller
        && func.sig.abi.spread.is_none()
        && !func.sig.abi.caller_location
        && !matches!(
            semantic,
            Some(BodySemantic::DropGlue(_) | BodySemantic::JsExport)
        )
        && !matches!(
            semantic,
            Some(BodySemantic::Standard(operation))
                if crate::lower::semantic_ops::fmt_kernel::is_format_machinery(operation)
                    || crate::lower::semantic_ops::num::is_float_from_str(operation)
                    || matches!(
                        operation,
                        crate::jir::semantic::StandardOperation::SliceSort
                            | crate::jir::semantic::StandardOperation::SliceSortCmp
                    )
                    || operation.is_typed_heap()
        )
}

/// Why the callee may not have its calling convention changed, or `None`.
pub(super) fn refuses(
    linkage: Option<&Linkage>,
    semantic: Option<BodySemantic>,
    func: &Func,
    chunked: bool,
    ineligible: &BTreeSet<ItemId>,
) -> Option<BoundaryDecline> {
    if chunked {
        return Some(BoundaryDecline::Chunks);
    }
    if linkage? != &Linkage::internal() {
        return Some(BoundaryDecline::Exported);
    }
    if func.sig.abi.spread.is_some() || func.sig.abi.caller_location {
        return Some(BoundaryDecline::SpreadAbi);
    }
    if matches!(
        semantic,
        Some(BodySemantic::DropGlue(_) | BodySemantic::JsExport)
    ) {
        return Some(BoundaryDecline::NonCallReference);
    }
    if matches!(
        semantic,
        Some(BodySemantic::Standard(operation))
            if crate::lower::semantic_ops::fmt_kernel::is_format_machinery(operation)
                || crate::lower::semantic_ops::num::is_float_from_str(operation)
                || matches!(
                    operation,
                    crate::jir::semantic::StandardOperation::SliceSort
                        | crate::jir::semantic::StandardOperation::SliceSortCmp
                )
                || operation.is_typed_heap()
    ) {
        return Some(BoundaryDecline::NonCallReference);
    }
    if ineligible.contains(&func.item) {
        return Some(BoundaryDecline::NonCallReference);
    }
    None
}

/// Whether `caller`'s own type table spells the same fields the plan chose.
fn caller_agrees(
    table: &BTreeMap<JsName, LinkItem>,
    caller: &JsName,
    shapes: &[Option<FlatParam>],
) -> bool {
    type_table(table, caller).is_some_and(|types| shapes_agree(types, shapes))
}

/// Whether `caller` can rebuild this result and every use of a result from `callee` is field-only.
fn caller_accepts_result(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    caller: &JsName,
    callee: ItemId,
    result: &FlatResult,
    potential_params: &BTreeMap<ItemId, Vec<Option<FlatParam>>>,
) -> Option<ResultUse> {
    let types = type_table(table, caller)?;
    match &result.kind {
        FlatResultKind::I64Pair => {
            if !crate::lower::js::i64_pair::is_i64(types, result.ty) {
                return None;
            }
            let func = funcs.get(caller)?.as_ref().ok()?;
            let mut saw_call = false;
            for block in &func.blocks {
                for op in &block.ops {
                    let OpKind::Call {
                        callee: Callee::Direct { item, .. },
                        host: None,
                        ..
                    } = &op.kind
                    else {
                        continue;
                    };
                    if *item != callee {
                        continue;
                    }
                    saw_call = true;
                    if let Some(value) = op.result {
                        let definition = func.values.get(value.0 as usize)?;
                        if definition.ty != result.ty || definition.carrier != Carrier::Baseline {
                            return None;
                        }
                    }
                }
            }
            return saw_call.then_some(ResultUse {
                read: true,
                ..ResultUse::default()
            });
        }
        FlatResultKind::Sentinel(sentinel) => {
            if sentinel_result_shape(types, result.ty).as_ref() != Some(sentinel) {
                return None;
            }
            let func = funcs.get(caller)?.as_ref().ok()?;
            let caller_returns_same_sentinel = func.sig.result == result.ty
                && sentinel_result_shape(types, func.sig.result).as_ref() == Some(sentinel)
                && !table.get(caller).is_some_and(|item| item.linkage.exported);
            let mut use_ = ResultUse::default();
            for block in &func.blocks {
                for op in &block.ops {
                    let OpKind::Call {
                        callee: Callee::Direct { item, .. },
                        host: None,
                        ..
                    } = &op.kind
                    else {
                        continue;
                    };
                    if *item != callee {
                        continue;
                    }
                    let Some(value) = op.result else {
                        use_.read = true;
                        continue;
                    };
                    let definition = func.values.get(value.0 as usize)?;
                    if definition.ty != result.ty || definition.carrier != Carrier::Baseline {
                        return None;
                    }
                    if sentinel_result_site_unboxed(
                        func,
                        types,
                        value,
                        sentinel,
                        caller_returns_same_sentinel,
                    ) {
                        use_.read = true;
                    }
                }
            }
            return Some(use_);
        }
        FlatResultKind::Tagged(tagged) => {
            if tagged_shape(types, result.ty).as_ref() != Some(tagged) {
                return None;
            }
        }
        FlatResultKind::Recursive(recursive) => {
            if recursive_shape(types, result.ty).as_ref() != Some(recursive) {
                return None;
            }
        }
        FlatResultKind::Aggregate { field_tys, .. } => {
            let agrees = candidates::shape(types, result.ty, 0).as_deref()
                == Some(field_tys.as_slice())
                && candidates::field_reads_resolve(types, result.ty, 0, field_tys.len());
            if !agrees {
                return None;
            }
        }
    }

    let func = funcs.get(caller)?.as_ref().ok()?;
    let loop_depths = crate::analysis::alloc_sites::loop_depths(func);
    let mut use_ = ResultUse::default();
    for block in &func.blocks {
        for op in &block.ops {
            let OpKind::Call {
                callee: Callee::Direct { item, .. },
                host: None,
                ..
            } = &op.kind
            else {
                continue;
            };
            if *item != callee {
                continue;
            }
            let Some(value) = op.result else { continue };
            let definition = func.values.get(value.0 as usize)?;
            if definition.ty != result.ty || definition.carrier != Carrier::Baseline {
                return None;
            }
            let observed = field_only_value(func, value, result, potential_params)?;
            use_.read |= observed.read;
            if !observed.forwards.is_empty() {
                use_.forward_sites += 1;
                use_.hot_forward |= loop_depths
                    .get(block.id.0 as usize)
                    .is_some_and(|depth| *depth > 0);
            }
            use_.forwards.extend(observed.forwards);
        }
    }
    Some(use_)
}

/// Field-sensitive uses of one prospective result-lane value.
#[derive(Default)]
pub(super) struct ResultUse {
    pub(super) read: bool,
    pub(super) forwards: BTreeSet<(ItemId, usize)>,
    pub(super) forward_sites: u32,
    pub(super) hot_forward: bool,
}

/// Whether `value` (returned by a `SentinelResult` direct call) is consumed without escaping.
fn sentinel_result_site_unboxed(
    func: &Func,
    types: &TypeTable,
    value: crate::jir::repr::ValueId,
    sentinel: &SentinelResultShape,
    caller_returns_same_sentinel: bool,
) -> bool {
    let payload_needs_clone = sentinel
        .ok_field
        .is_some_and(|(_, ty)| types.needs_clone(ty))
        || sentinel
            .err_field
            .is_some_and(|(_, ty)| types.needs_clone(ty));
    let mut aliases = BTreeSet::from([value]);
    let block_index: BTreeMap<RBlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();
    loop {
        let before = aliases.len();
        for block in &func.blocks {
            for op in &block.ops {
                let source = match &op.kind {
                    OpKind::Move(src) => *src,
                    OpKind::Copy(src) if !payload_needs_clone => *src,
                    _ => continue,
                };
                if aliases.contains(&source) {
                    if let Some(res) = op.result {
                        aliases.insert(res);
                    }
                }
            }
            for (target, args) in block.term.edges() {
                if target == RBlockId(0) {
                    continue;
                }
                let Some(&bi) = block_index.get(&target) else {
                    continue;
                };
                for (pos, arg) in args.iter().enumerate() {
                    if aliases.contains(arg) {
                        if let Some(&param) = func.blocks[bi].params.get(pos) {
                            aliases.insert(param);
                        }
                    }
                }
            }
        }
        if aliases.len() == before {
            break;
        }
    }
    if aliases.iter().any(|alias| {
        func.values
            .get(alias.0 as usize)
            .is_none_or(|d| d.ty != sentinel.ty || d.carrier != Carrier::Baseline)
    }) {
        return false;
    }
    let mut saw_use = false;
    for block in &func.blocks {
        for op in &block.ops {
            if !op.kind.uses().iter().any(|u| aliases.contains(u)) {
                continue;
            }
            match &op.kind {
                OpKind::Discriminant(base) if aliases.contains(base) => {
                    saw_use = true;
                }
                OpKind::Field {
                    base,
                    variant,
                    field,
                } if aliases.contains(base) => {
                    let v = variant.unwrap_or(0);
                    let res_zst = op
                        .result
                        .and_then(|r| func.ty(r))
                        .is_some_and(|t| types.is_zst(t));
                    let valid = (v == sentinel.ok_variant
                        && (sentinel.ok_field.is_some_and(|(f, _)| f == *field) || res_zst))
                        || (v == sentinel.err_variant
                            && (sentinel.err_field.is_some_and(|(f, _)| f == *field) || res_zst));
                    if !valid {
                        return false;
                    }
                    saw_use = true;
                }
                OpKind::Move(src) if aliases.contains(src) => {}
                OpKind::Copy(src) if !payload_needs_clone && aliases.contains(src) => {}
                OpKind::Drop {
                    root: crate::jir::repr::Root::Value(r),
                    path,
                    glue: crate::jir::semantic::DropGlue::None,
                } if aliases.contains(r) && path.is_empty() => {}
                _ => return false,
            }
        }
        match &block.term {
            RTerm::Return(Some(v)) if aliases.contains(v) => {
                if !caller_returns_same_sentinel {
                    return false;
                }
                saw_use = true;
            }
            RTerm::Goto(_, _) | RTerm::Switch { .. } | RTerm::Invoke { .. } => {
                if let RTerm::Switch { scrut, .. } = &block.term {
                    if aliases.contains(scrut) {
                        return false;
                    }
                }
                for (target, args) in block.term.edges() {
                    let Some(&bi) = block_index.get(&target) else {
                        if args.iter().any(|a| aliases.contains(a)) {
                            return false;
                        }
                        continue;
                    };
                    for (pos, arg) in args.iter().enumerate() {
                        if aliases.contains(arg)
                            && (target == RBlockId(0)
                                || func.blocks[bi]
                                    .params
                                    .get(pos)
                                    .is_none_or(|p| !aliases.contains(p)))
                        {
                            return false;
                        }
                    }
                }
            }
            RTerm::Throw(Some(v)) if aliases.contains(v) => return false,
            _ => {}
        }
    }
    saw_use || aliases.len() == 1
}

/// Whether `value` and aliases made only by `Move`/`Copy` are consumed exclusively by field reads
/// or by-value forwards into compatible prospective flat parameters.
pub(super) fn field_only_value(
    func: &Func,
    value: crate::jir::repr::ValueId,
    result: &FlatResult,
    potential_params: &BTreeMap<ItemId, Vec<Option<FlatParam>>>,
) -> Option<ResultUse> {
    let mut aliases = BTreeSet::from([value]);
    loop {
        let before = aliases.len();
        for block in &func.blocks {
            for op in &block.ops {
                let source = match &op.kind {
                    OpKind::Move(source) | OpKind::Copy(source) => *source,
                    _ => continue,
                };
                if aliases.contains(&source) {
                    aliases.insert(op.result?);
                }
            }
        }
        if aliases.len() == before {
            break;
        }
    }
    if aliases.iter().any(|alias| {
        func.values
            .get(alias.0 as usize)
            .is_none_or(|definition| definition.carrier != Carrier::Baseline)
    }) {
        return None;
    }

    let mut use_ = ResultUse::default();
    for block in &func.blocks {
        for op in &block.ops {
            let used_alias = op
                .kind
                .uses()
                .into_iter()
                .any(|used| aliases.contains(&used));
            if !used_alias {
                continue;
            }
            match &op.kind {
                OpKind::Field {
                    base,
                    variant,
                    field,
                } if aliases.contains(base) && result_accepts_field(result, *variant, *field) => {
                    use_.read = true;
                }
                OpKind::Discriminant(base)
                    if aliases.contains(base) && result.tagged().is_some() =>
                {
                    use_.read = true;
                }
                OpKind::Move(source) | OpKind::Copy(source) if aliases.contains(source) => {}
                OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    args,
                    host: None,
                    ..
                } => {
                    let mut forwarded = false;
                    for (position, argument) in args.iter().enumerate() {
                        if !aliases.contains(argument) {
                            continue;
                        }
                        let shape = potential_params.get(item)?.get(position)?.as_ref()?;
                        if !result.matches_by_value_param(shape) {
                            return None;
                        }
                        use_.forwards.insert((*item, position));
                        forwarded = true;
                    }
                    if !forwarded {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        if block
            .term
            .uses()
            .into_iter()
            .any(|used| aliases.contains(&used))
        {
            return None;
        }
    }
    Some(use_)
}

/// Whether a tagged/fixed result lane plan can answer this semantic field projection.
fn result_accepts_field(result: &FlatResult, variant: Option<u32>, field: u32) -> bool {
    match &result.kind {
        FlatResultKind::Tagged(tagged) => tagged.fields.contains(&(variant.unwrap_or(0), field)),
        FlatResultKind::Recursive(recursive) => {
            variant.is_none() && recursive.fields.contains(&field)
        }
        FlatResultKind::Aggregate { field_tys, .. } => {
            variant.is_none() && (field as usize) < field_tys.len()
        }
        FlatResultKind::I64Pair | FlatResultKind::Sentinel(_) => false,
    }
}

/// Whether every whole-result forward still targets a parameter selected by the parameter SCC
/// fixed point.
pub(super) fn result_forwards_survive(
    forwards: &BTreeSet<(ItemId, usize)>,
    flat: &BTreeMap<ItemId, Vec<bool>>,
) -> bool {
    forwards.iter().all(|(callee, argument)| {
        flat.get(callee)
            .and_then(|params| params.get(*argument))
            .copied()
            .unwrap_or(false)
    })
}

/// Every returned aggregate still has its baseline carrier before the ABI rewrite.
fn returns_baseline(func: &Func, ty: TypeId) -> bool {
    func.blocks.iter().all(|block| match block.term {
        RTerm::Return(Some(value)) => func.values.get(value.0 as usize).is_some_and(|definition| {
            definition.ty == ty && definition.carrier == Carrier::Baseline
        }),
        RTerm::Return(None) => false,
        _ => true,
    })
}

/// Closes the per-parameter summaries over the calls between them.
pub(super) fn close_over_forwards(
    candidates: &BTreeMap<ItemId, Candidate>,
) -> BTreeMap<ItemId, Vec<bool>> {
    let mut flat: BTreeMap<ItemId, Vec<bool>> = candidates
        .iter()
        .map(|(item, candidate)| {
            (
                *item,
                candidate.shapes.iter().map(Option::is_some).collect(),
            )
        })
        .collect();
    loop {
        let mut changed = false;
        for (item, candidate) in candidates {
            for (position, use_) in candidate.uses.iter().enumerate() {
                if !flat[item][position] {
                    continue;
                }
                if candidate.shapes[position]
                    .as_ref()
                    .is_some_and(FlatParam::is_i64_pair)
                {
                    continue;
                }
                let keeps = !use_.escapes
                    && use_.forwards.iter().all(|(callee, argument)| {
                        flat.get(callee)
                            .and_then(|params| params.get(*argument))
                            .copied()
                            .unwrap_or(false)
                    });
                if !keeps {
                    flat.get_mut(item).expect("a candidate")[position] = false;
                    changed = true;
                }
            }
        }
        if !changed {
            return flat;
        }
    }
}
