//! Whole-program zero-offset slice specialization (`off === 0`) under `Profile::ReleaseSpeed`.
//!
//! Runs a greatest fixpoint proving which flattened slice parameters (and direct slice leaves
//! inside `RecursiveShape` parameters) are provably called exclusively with `off === 0`, narrowing
//! them from 3 JS parameters `(buf, off, len)` to 2 JS parameters `(buf, len)`.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{JsName, LinkItem, Linkage};
use crate::jir::decline::Decline;
use crate::jir::repr::{Carrier, CellId, Func, OpKind, Proj, RBlockId, RTerm, Root, ValueId};
use crate::jir::semantic::{
    BinaryOp, BodySemantic, CallSemantic, Callee, CastKind, ConstTree, GlobalReference, ItemId,
    StandardOperation, TypeId,
};
use crate::jir::types::{PointerForm, TypeKind, TypeTable};

use super::plan::{refuses, type_table, AbiPlan};
use super::shapes::{FlatParamKind, LaneComponent, ProductNode};

/// Read-only program context passed into [`specialize_zero_offset_slices`].
pub(super) struct ZeroOffsetContext<'a> {
    pub table: &'a BTreeMap<JsName, LinkItem>,
    pub funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
    pub excluded: &'a BTreeSet<ItemId>,
    pub non_call_named: &'a BTreeSet<ItemId>,
    pub ineligible: &'a BTreeSet<ItemId>,
    pub pinned_by_tracked_caller: &'a BTreeSet<ItemId>,
    pub chunked: bool,
}

/// Identifies one slice position eligible for whole-program zero-offset (`off === 0`)
/// specialization (`[Buffer, Offset, Length]` -> `[Buffer, Length]`).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SliceSlot {
    /// A top-level `FlatParamKind::Slice` parameter.
    Top(usize),
    /// A direct slice leaf field `field_idx` inside a `FlatParamKind::Recursive` product.
    RecursiveField(usize, usize),
}

struct OffZeroBody<'a> {
    item: ItemId,
    trust_own_params: bool,
    func: &'a Func,
    types: &'a TypeTable,
    calls: Vec<(ItemId, Vec<ValueId>)>,
    pure_cell_stores: BTreeMap<CellId, Vec<ValueId>>,
    block_param_incomings: BTreeMap<ValueId, Vec<ValueId>>,
    value_defs: BTreeMap<ValueId, (usize, usize)>,
    param_indices: BTreeMap<ValueId, usize>,
    mutated_values: BTreeSet<ValueId>,
    tracked_slices: Vec<ValueId>,
    tracked_ptrs: Vec<ValueId>,
    ret_candidate: bool,
    returns: Vec<ValueId>,
}

struct CollectedBodies<'a> {
    bodies: Vec<OffZeroBody<'a>>,
    untrusted_callees: BTreeSet<ItemId>,
    entry_targeted_items: BTreeSet<ItemId>,
    callee_has_sites: BTreeSet<ItemId>,
}

/// Whole-program greatest fixpoint proving which flattened slice parameters (and direct slice
/// leaves inside `RecursiveShape` parameters) are provably called exclusively with `off === 0`.
pub(super) fn specialize_zero_offset_slices(ctx: ZeroOffsetContext<'_>, plan: &mut AbiPlan) {
    if plan.is_empty() {
        return;
    }

    let reachable = compute_reachable_items(&ctx, plan);
    let collected = collect_bodies(&ctx, plan, reachable.as_ref());
    let (mut param_zero, mut ret_zero) = seed_fixpoint_sets(&collected, plan);
    if param_zero.is_empty() {
        return;
    }

    solve_zero_offset_fixpoint(&collected.bodies, &mut param_zero, &mut ret_zero);
    apply_zero_offset_results(&param_zero, ctx.funcs, plan);
}

fn compute_reachable_items(
    ctx: &ZeroOffsetContext<'_>,
    plan: &AbiPlan,
) -> Option<BTreeSet<ItemId>> {
    if ctx.chunked {
        return None;
    }
    let mut roots: BTreeSet<ItemId> = BTreeSet::new();
    let mut edges: BTreeMap<ItemId, Vec<ItemId>> = BTreeMap::new();
    for (name, entry) in ctx.funcs {
        let Ok(func) = entry else { continue };
        let link_item = ctx.table.get(name);
        let semantic = link_item
            .and_then(|it| it.jir.body.as_ref())
            .map(|b| b.semantic);
        let is_special_runtime = matches!(
            semantic,
            Some(BodySemantic::DropGlue(_) | BodySemantic::PanicImpl)
        ) || matches!(
            semantic,
            Some(BodySemantic::Standard(op))
                if crate::lower::semantic_ops::fmt_kernel::is_format_machinery(op)
                    || crate::lower::semantic_ops::num::is_float_from_str(op)
                    || matches!(
                        op,
                        StandardOperation::SliceSort | StandardOperation::SliceSortCmp
                    )
                    || op.is_typed_heap()
        );
        if link_item.is_none_or(|it| it.linkage != Linkage::internal())
            || ctx.non_call_named.contains(&func.item)
            || ctx.excluded.contains(&func.item)
            || is_special_runtime
        {
            roots.insert(func.item);
        }
        let caller_sites = plan.caller_site_retargets(name);
        let callee_list = edges.entry(func.item).or_default();
        for block in &func.blocks {
            for (oi, op) in block.ops.iter().enumerate() {
                if let OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    function: None,
                    host: None,
                    ..
                } = &op.kind
                {
                    callee_list.push(*item);
                    if let Some(&retarget) =
                        caller_sites.and_then(|sites| sites.get(&(block.id, oi)))
                    {
                        callee_list.push(retarget);
                    }
                }
            }
        }
    }
    for (clone_name, source_name) in plan.clones() {
        let Some(Ok(func)) = ctx.funcs.get(source_name) else {
            continue;
        };
        let clone_item = plan
            .synthetic_items
            .get(clone_name)
            .copied()
            .unwrap_or(func.item);
        if clone_item != func.item {
            edges.entry(func.item).or_default().push(clone_item);
            let src_edges = edges.get(&func.item).cloned().unwrap_or_default();
            edges.entry(clone_item).or_default().extend(src_edges);
        }
    }
    if roots.is_empty() {
        return None;
    }
    let mut seen = roots.clone();
    let mut work: Vec<ItemId> = roots.into_iter().collect();
    while let Some(cur) = work.pop() {
        if let Some(nexts) = edges.get(&cur) {
            for &next in nexts {
                if seen.insert(next) {
                    work.push(next);
                }
            }
        }
    }
    Some(seen)
}

fn collect_bodies<'a>(
    ctx: &ZeroOffsetContext<'a>,
    plan: &AbiPlan,
    reachable: Option<&BTreeSet<ItemId>>,
) -> CollectedBodies<'a> {
    let mut collected = CollectedBodies {
        bodies: Vec::new(),
        untrusted_callees: BTreeSet::new(),
        entry_targeted_items: BTreeSet::new(),
        callee_has_sites: BTreeSet::new(),
    };

    let push_body = |collected: &mut CollectedBodies<'a>,
                     name: &JsName,
                     item: ItemId,
                     trust_own_params: bool,
                     ret_allowed: bool,
                     func: &'a Func,
                     types: &'a TypeTable| {
        let body =
            build_off_zero_body(name, item, trust_own_params, ret_allowed, func, types, plan);
        for &(callee, _) in &body.calls {
            collected.callee_has_sites.insert(callee);
        }
        collected.bodies.push(body);
    };

    // Canonical / in-place bodies.
    for (name, entry) in ctx.funcs {
        let Ok(func) = entry else { continue };
        if reachable.is_some_and(|reach| !reach.contains(&func.item)) {
            continue;
        }
        let Some(types) = type_table(ctx.table, name) else {
            let caller_sites = plan.caller_site_retargets(name);
            for block in &func.blocks {
                for (oi, op) in block.ops.iter().enumerate() {
                    if let OpKind::Call {
                        callee: Callee::Direct { item, .. },
                        function: None,
                        host: None,
                        ..
                    } = &op.kind
                    {
                        let target = caller_sites
                            .and_then(|sites| sites.get(&(block.id, oi)))
                            .copied()
                            .unwrap_or(*item);
                        collected.untrusted_callees.insert(target);
                    }
                }
            }
            continue;
        };
        let entry_targeted = func
            .blocks
            .iter()
            .any(|b| b.term.successors().contains(&RBlockId(0)));
        if entry_targeted && plan.target_name(func.item) == Some(name) {
            collected.entry_targeted_items.insert(func.item);
        }
        let link_item = ctx.table.get(name);
        let semantic = link_item
            .and_then(|it| it.jir.body.as_ref())
            .map(|b| b.semantic);
        let in_place_flat =
            plan.flat_map().contains_key(&func.item) && plan.target_name(func.item) == Some(name);
        let internal_non_flat = !plan.flat_map().contains_key(&func.item)
            && !ctx.pinned_by_tracked_caller.contains(&func.item)
            && refuses(
                link_item.map(|it| &it.linkage),
                semantic,
                func,
                ctx.chunked,
                ctx.ineligible,
            )
            .is_none();
        let trust_own_params = !entry_targeted && (in_place_flat || internal_non_flat);
        let ret_allowed = !ctx.excluded.contains(&func.item)
            && matches!(semantic, Some(BodySemantic::Ordinary))
            && !plan.clones().values().any(|src| src == name);
        push_body(
            &mut collected,
            name,
            func.item,
            trust_own_params,
            ret_allowed,
            func,
            types,
        );
    }

    // Private clone bodies.
    for (clone_name, source_name) in plan.clones() {
        let Some(Ok(func)) = ctx.funcs.get(source_name) else {
            continue;
        };
        let Some(types) = type_table(ctx.table, source_name) else {
            continue;
        };
        let clone_item = plan
            .synthetic_items
            .get(clone_name)
            .copied()
            .unwrap_or(func.item);
        if reachable
            .is_some_and(|reach| !reach.contains(&clone_item) && !reach.contains(&func.item))
        {
            continue;
        }
        let entry_targeted = func
            .blocks
            .iter()
            .any(|b| b.term.successors().contains(&RBlockId(0)));
        if entry_targeted {
            collected.entry_targeted_items.insert(clone_item);
        }
        let trust_own_params = !entry_targeted && plan.target_name(clone_item) == Some(clone_name);
        push_body(
            &mut collected,
            clone_name,
            clone_item,
            trust_own_params,
            false,
            func,
            types,
        );
    }

    collected
}

fn seed_fixpoint_sets(
    collected: &CollectedBodies<'_>,
    plan: &AbiPlan,
) -> (BTreeSet<(ItemId, SliceSlot)>, BTreeSet<ItemId>) {
    let mut param_zero: BTreeSet<(ItemId, SliceSlot)> = BTreeSet::new();
    for (&item, sig) in plan.flat_map() {
        if !collected.callee_has_sites.contains(&item)
            || collected.untrusted_callees.contains(&item)
            || collected.entry_targeted_items.contains(&item)
        {
            continue;
        }
        for (pos, param_opt) in sig.params.iter().enumerate() {
            let Some(param) = param_opt else { continue };
            match &param.kind {
                FlatParamKind::Slice {
                    zero_offset: false, ..
                } => {
                    param_zero.insert((item, SliceSlot::Top(pos)));
                }
                FlatParamKind::Recursive(rec) => {
                    if let ProductNode::Product { fields, .. } = &rec.root {
                        for (fi, node) in fields.iter().enumerate() {
                            if matches!(
                                node,
                                ProductNode::Leaf { components, .. }
                                    if components.as_slice()
                                        == [
                                            LaneComponent::Buffer,
                                            LaneComponent::Offset,
                                            LaneComponent::Length,
                                        ]
                            ) {
                                param_zero.insert((item, SliceSlot::RecursiveField(pos, fi)));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if param_zero.is_empty() {
        return (param_zero, BTreeSet::new());
    }

    // Also seed trusted internal non-flat forwarders so `entry -> forwarder -> flat_callee`
    // propagates `off === 0` across unflattened internal helpers.
    for body in &collected.bodies {
        if plan.flat_map().contains_key(&body.item)
            || !body.trust_own_params
            || !collected.callee_has_sites.contains(&body.item)
            || collected.untrusted_callees.contains(&body.item)
        {
            continue;
        }
        for (pos, &p) in body.func.params.iter().enumerate() {
            let def = body.func.values[p.0 as usize];
            if def.carrier == Carrier::Baseline && slice_element_ty(body.types, def.ty).is_some() {
                param_zero.insert((body.item, SliceSlot::Top(pos)));
            }
        }
    }

    let ret_zero: BTreeSet<ItemId> = collected
        .bodies
        .iter()
        .filter(|b| b.ret_candidate)
        .map(|b| b.item)
        .collect();
    (param_zero, ret_zero)
}

fn solve_zero_offset_fixpoint(
    bodies: &[OffZeroBody<'_>],
    param_zero: &mut BTreeSet<(ItemId, SliceSlot)>,
    ret_zero: &mut BTreeSet<ItemId>,
) {
    let mut item_dependents: BTreeMap<ItemId, Vec<usize>> = BTreeMap::new();
    let mut dirty: Vec<bool> = vec![false; bodies.len()];
    for (bi, body) in bodies.iter().enumerate() {
        if body.trust_own_params || body.ret_candidate {
            item_dependents.entry(body.item).or_default().push(bi);
            dirty[bi] = true;
        }
        for &(callee, _) in &body.calls {
            if param_zero
                .range((callee, SliceSlot::Top(0))..)
                .next()
                .is_some_and(|(id, _)| *id == callee)
                || ret_zero.contains(&callee)
            {
                item_dependents.entry(callee).or_default().push(bi);
                dirty[bi] = true;
            }
        }
    }
    for deps in item_dependents.values_mut() {
        deps.sort_unstable();
        deps.dedup();
    }

    loop {
        let mut next_dirty = vec![false; bodies.len()];
        let mut any_changed = false;

        for (bi, body) in bodies.iter().enumerate() {
            if !dirty[bi] {
                continue;
            }
            let mut slice_zero: BTreeSet<ValueId> = body.tracked_slices.iter().copied().collect();
            let mut ptr_zero: BTreeSet<ValueId> = body.tracked_ptrs.iter().copied().collect();
            loop {
                let before_s = slice_zero.len();
                let before_p = ptr_zero.len();
                let cur_s = slice_zero.clone();
                let cur_p = ptr_zero.clone();
                slice_zero.retain(|&v| {
                    value_is_zero_offset_slice(body, v, &cur_s, &cur_p, param_zero, ret_zero)
                });
                ptr_zero.retain(|&v| value_is_zero_offset_ptr(body, v, &slice_zero, &cur_p));
                if slice_zero.len() == before_s && ptr_zero.len() == before_p {
                    break;
                }
            }

            let mut changed_items: Vec<ItemId> = Vec::new();
            if body.ret_candidate
                && ret_zero.contains(&body.item)
                && !body.returns.iter().all(|v| slice_zero.contains(v))
            {
                ret_zero.remove(&body.item);
                changed_items.push(body.item);
            }

            for (callee, args) in &body.calls {
                let slots: Vec<SliceSlot> = param_zero
                    .range((*callee, SliceSlot::Top(0))..)
                    .take_while(|(id, _)| id == callee)
                    .map(|(_, slot)| *slot)
                    .collect();
                for slot in slots {
                    let ok = match slot {
                        SliceSlot::Top(pos) => {
                            args.get(pos).is_some_and(|arg| slice_zero.contains(arg))
                        }
                        SliceSlot::RecursiveField(pos, field_idx) => {
                            args.get(pos).is_some_and(|arg| {
                                arg_recursive_field_is_zero(
                                    body,
                                    *arg,
                                    field_idx,
                                    &slice_zero,
                                    param_zero,
                                )
                            })
                        }
                    };
                    if !ok {
                        param_zero.remove(&(*callee, slot));
                        changed_items.push(*callee);
                    }
                }
            }

            if !changed_items.is_empty() {
                any_changed = true;
                for item in changed_items {
                    if let Some(deps) = item_dependents.get(&item) {
                        for &dep_bi in deps {
                            next_dirty[dep_bi] = true;
                        }
                    }
                }
            }
        }

        if !any_changed {
            break;
        }
        dirty = next_dirty;
    }
}

fn apply_zero_offset_results(
    param_zero: &BTreeSet<(ItemId, SliceSlot)>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    plan: &mut AbiPlan,
) {
    let mut updated_items: BTreeSet<ItemId> = BTreeSet::new();
    for &(item, slot) in param_zero {
        let changed = match slot {
            SliceSlot::Top(pos) => plan.mark_zero_offset_slice(item, pos),
            SliceSlot::RecursiveField(pos, field_idx) => {
                plan.mark_zero_offset_recursive_field(item, pos, field_idx)
            }
        };
        if changed {
            updated_items.insert(item);
        }
    }
    for item in updated_items {
        plan.refresh_decision_for_item(item, funcs);
    }
}

fn slice_element_ty(types: &TypeTable, ty: TypeId) -> Option<TypeId> {
    let pointee = match types.get(types.peeled(ty)).map(|d| &d.kind) {
        Some(TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. }) => {
            *pointee
        }
        _ => return None,
    };
    match types.get(types.peeled(pointee)).map(|d| &d.kind) {
        Some(TypeKind::Slice(element)) => Some(*element),
        _ => None,
    }
}

fn array_ref_element_ty(types: &TypeTable, ty: TypeId) -> Option<(TypeId, bool)> {
    let (pointee, is_raw) = match types.get(types.peeled(ty)).map(|d| &d.kind) {
        Some(TypeKind::Reference { pointee, .. }) => (*pointee, false),
        Some(TypeKind::RawPointer { pointee, .. }) => (*pointee, true),
        _ => return None,
    };
    match types.get(types.peeled(pointee)).map(|d| &d.kind) {
        Some(TypeKind::Array { element, .. }) => Some((*element, is_raw)),
        _ => None,
    }
}

fn ptr_pointee_ty(types: &TypeTable, ty: TypeId) -> Option<(TypeId, bool)> {
    match types.get(types.peeled(ty)).map(|d| &d.kind) {
        Some(TypeKind::Reference { pointee, .. }) => Some((*pointee, false)),
        Some(TypeKind::RawPointer { pointee, .. }) => Some((*pointee, true)),
        _ => None,
    }
}

fn build_off_zero_body<'a>(
    name: &JsName,
    item: ItemId,
    trust_own_params: bool,
    ret_allowed: bool,
    func: &'a Func,
    types: &'a TypeTable,
    plan: &AbiPlan,
) -> OffZeroBody<'a> {
    let caller_sites = plan.caller_site_retargets(name);
    let mut calls = Vec::new();
    let mut value_defs = BTreeMap::new();
    let mut mutated_values = BTreeSet::new();
    let mut cell_stores: BTreeMap<CellId, Vec<ValueId>> = BTreeMap::new();
    let mut bad_cells: BTreeSet<CellId> = BTreeSet::new();
    let mut block_index: BTreeMap<RBlockId, usize> = BTreeMap::new();
    let mut returns = Vec::new();
    let mut has_void_return = false;

    for (bi, block) in func.blocks.iter().enumerate() {
        block_index.insert(block.id, bi);
        for (oi, op) in block.ops.iter().enumerate() {
            if let Some(res) = op.result {
                value_defs.insert(res, (bi, oi));
            }
            match &op.kind {
                OpKind::Call {
                    callee: Callee::Direct { item: callee, .. },
                    function: None,
                    args,
                    host: None,
                    ..
                } => {
                    let target = caller_sites
                        .and_then(|sites| sites.get(&(block.id, oi)))
                        .copied()
                        .unwrap_or(*callee);
                    calls.push((target, args.clone()));
                }
                OpKind::CellStore { cell, v } => {
                    cell_stores.entry(*cell).or_default().push(*v);
                }
                OpKind::BorrowView {
                    root: Root::Cell(cell),
                    path,
                    mutable: true,
                } if path.is_empty() => {
                    bad_cells.insert(*cell);
                }
                OpKind::RawView {
                    root: Root::Cell(cell),
                    path,
                    ..
                } if path.is_empty() => {
                    bad_cells.insert(*cell);
                }
                OpKind::SetField { base, .. } | OpKind::SetDiscriminant { base, .. } => {
                    mutated_values.insert(*base);
                }
                OpKind::BorrowView {
                    root: Root::Value(v),
                    mutable: true,
                    ..
                }
                | OpKind::RawView {
                    root: Root::Value(v),
                    ..
                } => {
                    mutated_values.insert(*v);
                }
                _ => {}
            }
        }
        match block.term {
            RTerm::Return(Some(v)) => returns.push(v),
            RTerm::Return(None) => has_void_return = true,
            _ => {}
        }
    }

    let pure_cell_stores: BTreeMap<CellId, Vec<ValueId>> = cell_stores
        .into_iter()
        .filter(|(cell, stores)| !bad_cells.contains(cell) && !stores.is_empty())
        .collect();

    let mut block_param_incomings: BTreeMap<ValueId, Vec<ValueId>> = BTreeMap::new();
    for block in &func.blocks {
        if block.id != RBlockId(0) {
            for &param in &block.params {
                block_param_incomings.entry(param).or_default();
            }
        }
    }
    for block in &func.blocks {
        for (target, args) in block.term.edges() {
            if target == RBlockId(0) {
                continue;
            }
            let Some(&bi) = block_index.get(&target) else {
                continue;
            };
            for (pos, &param) in func.blocks[bi].params.iter().enumerate() {
                if let Some(&arg) = args.get(pos) {
                    block_param_incomings.entry(param).or_default().push(arg);
                }
            }
        }
    }

    let param_indices: BTreeMap<ValueId, usize> = func
        .params
        .iter()
        .enumerate()
        .map(|(i, &v)| (v, i))
        .collect();

    let mut tracked_slices = Vec::new();
    let mut tracked_ptrs = Vec::new();
    for (idx, def) in func.values.iter().enumerate() {
        if def.carrier != Carrier::Baseline {
            continue;
        }
        let v = ValueId(idx as u32);
        if slice_element_ty(types, def.ty).is_some() {
            tracked_slices.push(v);
        } else if ptr_pointee_ty(types, def.ty).is_some() {
            tracked_ptrs.push(v);
        }
    }

    let ret_candidate = ret_allowed
        && !has_void_return
        && !returns.is_empty()
        && slice_element_ty(types, func.sig.result).is_some();

    OffZeroBody {
        item,
        trust_own_params,
        func,
        types,
        calls,
        pure_cell_stores,
        block_param_incomings,
        value_defs,
        param_indices,
        mutated_values,
        tracked_slices,
        tracked_ptrs,
        ret_candidate,
        returns,
    }
}

fn borrowed_vec_is_slice_view(body: &OffZeroBody<'_>, mut v: ValueId) -> bool {
    for _ in 0..=body.func.values.len().min(64) {
        if body
            .func
            .values
            .get(v.0 as usize)
            .is_some_and(|d| d.carrier == Carrier::SliceView)
        {
            return true;
        }
        let Some(&(bi, oi)) = body.value_defs.get(&v) else {
            return false;
        };
        match &body.func.blocks[bi].ops[oi].kind {
            OpKind::Copy(src) | OpKind::Move(src) => v = *src,
            OpKind::BorrowView {
                root: Root::Value(src),
                path,
                ..
            } if path.is_empty() || path.as_slice() == [Proj::Deref] => v = *src,
            _ => return false,
        }
    }
    true
}

fn is_const_zero_scalar(body: &OffZeroBody<'_>, mut v: ValueId) -> bool {
    for _ in 0..8 {
        let Some(&(bi, oi)) = body.value_defs.get(&v) else {
            return false;
        };
        match &body.func.blocks[bi].ops[oi].kind {
            OpKind::Const(ConstTree::Scalar { bits: 0, .. }) => return true,
            OpKind::Copy(src) | OpKind::Move(src) => v = *src,
            _ => return false,
        }
    }
    false
}

fn arg_recursive_field_is_zero(
    body: &OffZeroBody<'_>,
    mut v: ValueId,
    field_idx: usize,
    slice_zero: &BTreeSet<ValueId>,
    param_zero: &BTreeSet<(ItemId, SliceSlot)>,
) -> bool {
    for _ in 0..16 {
        if body.mutated_values.contains(&v) {
            return false;
        }
        if let Some(&p) = body.param_indices.get(&v) {
            return body.trust_own_params
                && param_zero.contains(&(body.item, SliceSlot::RecursiveField(p, field_idx)));
        }
        let Some(&(bi, oi)) = body.value_defs.get(&v) else {
            return false;
        };
        match &body.func.blocks[bi].ops[oi].kind {
            OpKind::Copy(src) | OpKind::Move(src) => v = *src,
            OpKind::Aggregate {
                variant: 0, fields, ..
            } => {
                return fields
                    .get(field_idx)
                    .is_some_and(|f| slice_zero.contains(f));
            }
            _ => return false,
        }
    }
    false
}

fn value_is_zero_offset_slice(
    body: &OffZeroBody<'_>,
    v: ValueId,
    slice_zero: &BTreeSet<ValueId>,
    ptr_zero: &BTreeSet<ValueId>,
    param_zero: &BTreeSet<(ItemId, SliceSlot)>,
    ret_zero: &BTreeSet<ItemId>,
) -> bool {
    if let Some(&p) = body.param_indices.get(&v) {
        return body.trust_own_params && param_zero.contains(&(body.item, SliceSlot::Top(p)));
    }
    if let Some(incomings) = body.block_param_incomings.get(&v) {
        return !incomings.is_empty() && incomings.iter().all(|inc| slice_zero.contains(inc));
    }
    let Some(&(bi, oi)) = body.value_defs.get(&v) else {
        return false;
    };
    let op = &body.func.blocks[bi].ops[oi];
    let types = body.types;
    let v_ty = body.func.values[v.0 as usize].ty;

    match &op.kind {
        OpKind::Copy(src) | OpKind::Move(src) => slice_zero.contains(src),
        OpKind::Const(tree) => match tree {
            ConstTree::Slice(_) | ConstTree::SliceAddress { .. } => true,
            ConstTree::Reference(inner) => matches!(inner.as_ref(), ConstTree::Slice(_)),
            ConstTree::StaticSlice { start: 0, .. } => true,
            _ => false,
        },
        OpKind::CellLoad(cell) => body
            .pure_cell_stores
            .get(cell)
            .is_some_and(|stores| stores.iter().all(|s| slice_zero.contains(s))),
        OpKind::BorrowView { root, path, .. } | OpKind::RawView { root, path, .. } => {
            let whole_or_prefix0 = matches!(
                path.as_slice(),
                [Proj::Deref] | [Proj::Deref, Proj::Subslice { from: 0, .. }]
            );
            if !whole_or_prefix0 {
                return false;
            }
            match root {
                Root::Value(src) => {
                    body.func
                        .ty(*src)
                        .is_some_and(|t| slice_element_ty(types, t).is_some())
                        && slice_zero.contains(src)
                }
                Root::Cell(cell) => {
                    body.func
                        .cells
                        .get(cell.0 as usize)
                        .is_some_and(|c| slice_element_ty(types, c.ty).is_some())
                        && body
                            .pure_cell_stores
                            .get(cell)
                            .is_some_and(|stores| stores.iter().all(|s| slice_zero.contains(s)))
                }
            }
        }
        OpKind::Cast {
            kind,
            v: src,
            from,
            to,
        } => match kind {
            CastKind::Subtype => {
                slice_element_ty(types, *from).is_some() && slice_zero.contains(src)
            }
            CastKind::PointerCoercion => {
                if let (Some((arr_elem, is_raw)), Some(slice_elem)) = (
                    array_ref_element_ty(types, *from),
                    slice_element_ty(types, *to),
                ) {
                    if types.peeled(arr_elem) == types.peeled(slice_elem) {
                        return !is_raw || types.is_zst(arr_elem) || ptr_zero.contains(src);
                    }
                }
                if let (Some(fe), Some(te)) =
                    (slice_element_ty(types, *from), slice_element_ty(types, *to))
                {
                    return types.peeled(fe) == types.peeled(te) && slice_zero.contains(src);
                }
                false
            }
            CastKind::PtrToPtr => {
                if let (Some(fe), Some(te)) =
                    (slice_element_ty(types, *from), slice_element_ty(types, *to))
                {
                    return types.peeled(fe) == types.peeled(te) && slice_zero.contains(src);
                }
                false
            }
            _ => false,
        },
        OpKind::SliceView { ptr, .. } => {
            let Some(slice_elem) = slice_element_ty(types, v_ty) else {
                return false;
            };
            if types.is_zst(slice_elem) {
                return true;
            }
            let Some(ptr_ty) = body.func.ty(*ptr) else {
                return false;
            };
            if types.pointer_form(ptr_ty) == Some(PointerForm::Thin)
                && array_ref_element_ty(types, ptr_ty)
                    .is_some_and(|(ae, _)| types.peeled(ae) == types.peeled(slice_elem))
            {
                return true;
            }
            if matches!(
                types.get(types.peeled(slice_elem)).map(|d| &d.kind),
                Some(TypeKind::Array { .. })
            ) {
                return false;
            }
            let same_elem_ptr = ptr_pointee_ty(types, ptr_ty)
                .is_some_and(|(tp, _)| types.peeled(tp) == types.peeled(slice_elem))
                && ptr_zero.contains(ptr);
            let same_elem_slice = slice_element_ty(types, ptr_ty)
                .is_some_and(|se| types.peeled(se) == types.peeled(slice_elem))
                && slice_zero.contains(ptr);
            same_elem_ptr || same_elem_slice
        }
        OpKind::Call {
            callee,
            function: None,
            args,
            semantic,
            host: None,
            ..
        } => match semantic {
            CallSemantic::Standard(
                StandardOperation::NativeVecAsSlice | StandardOperation::NativeVecAsMutSlice,
            ) => true,
            CallSemantic::Standard(
                StandardOperation::VecAsSlice | StandardOperation::VecAsMutSlice,
            ) => args
                .first()
                .is_some_and(|a| !borrowed_vec_is_slice_view(body, *a)),
            CallSemantic::Ordinary => match callee {
                Callee::Direct { item, .. } => ret_zero.contains(item),
                _ => false,
            },
            _ => false,
        },
        OpKind::Field {
            base,
            variant: None,
            field,
        } => arg_recursive_field_is_zero(body, *base, *field as usize, slice_zero, param_zero),
        _ => false,
    }
}

fn value_is_zero_offset_ptr(
    body: &OffZeroBody<'_>,
    v: ValueId,
    slice_zero: &BTreeSet<ValueId>,
    ptr_zero: &BTreeSet<ValueId>,
) -> bool {
    if let Some(incomings) = body.block_param_incomings.get(&v) {
        return !incomings.is_empty() && incomings.iter().all(|inc| ptr_zero.contains(inc));
    }
    let Some(&(bi, oi)) = body.value_defs.get(&v) else {
        return false;
    };
    let op = &body.func.blocks[bi].ops[oi];
    let types = body.types;

    match &op.kind {
        OpKind::Copy(src) | OpKind::Move(src) => ptr_zero.contains(src),
        OpKind::Const(ConstTree::Address(_)) => true,
        OpKind::Const(ConstTree::Global {
            target: GlobalReference::Static { .. },
            offset: 0,
        }) => true,
        OpKind::Call {
            semantic:
                CallSemantic::Standard(
                    StandardOperation::NativeVecAsPtr | StandardOperation::NativeVecAsMutPtr,
                ),
            host: None,
            ..
        } => true,
        OpKind::Cast {
            kind,
            v: src,
            from,
            to,
        } => match kind {
            CastKind::Subtype => ptr_zero.contains(src),
            CastKind::PtrToPtr | CastKind::PointerCoercion => {
                if let (Some(se), Some((tp, _))) =
                    (slice_element_ty(types, *from), ptr_pointee_ty(types, *to))
                {
                    if types.peeled(se) == types.peeled(tp) {
                        return slice_zero.contains(src);
                    }
                }
                if matches!(kind, CastKind::PointerCoercion) {
                    if let (Some((ae, true)), Some((tp, true))) = (
                        array_ref_element_ty(types, *from),
                        ptr_pointee_ty(types, *to),
                    ) {
                        if types.peeled(ae) == types.peeled(tp) {
                            return true;
                        }
                    }
                }
                if let (Some((fp, _)), Some((tp, _))) =
                    (ptr_pointee_ty(types, *from), ptr_pointee_ty(types, *to))
                {
                    let fp_p = types.peeled(fp);
                    let tp_p = types.peeled(tp);
                    if fp_p == tp_p
                        && !matches!(
                            types.get(fp_p).map(|d| &d.kind),
                            Some(TypeKind::Str | TypeKind::Slice(_))
                        )
                    {
                        return ptr_zero.contains(src);
                    }
                }
                false
            }
            _ => false,
        },
        OpKind::BorrowView { root, path, .. } | OpKind::RawView { root, path, .. } => {
            match (root, path.as_slice()) {
                (Root::Cell(_), []) => true,
                (Root::Value(val), []) => body.func.ty(*val).is_some_and(|t| {
                    matches!(
                        types.get(types.peeled(t)).map(|d| &d.kind),
                        Some(TypeKind::Array { .. })
                    )
                }),
                (
                    Root::Cell(c),
                    [Proj::ConstantIndex {
                        offset: 0,
                        from_end: false,
                        ..
                    }],
                ) => body.func.cells.get(c.0 as usize).is_some_and(|cell| {
                    matches!(
                        types.get(types.peeled(cell.ty)).map(|d| &d.kind),
                        Some(TypeKind::Array { .. })
                    )
                }),
                (
                    Root::Value(src),
                    [Proj::Deref, Proj::ConstantIndex {
                        offset: 0,
                        from_end: false,
                        ..
                    }],
                ) => body.func.ty(*src).is_some_and(|t| {
                    (types.pointer_form(t) == Some(PointerForm::Thin)
                        && array_ref_element_ty(types, t).is_some())
                        || (slice_element_ty(types, t).is_some() && slice_zero.contains(src))
                }),
                (Root::Value(src), [Proj::Deref]) => {
                    body.func.ty(*src).is_some_and(|t| {
                        ptr_pointee_ty(types, t).is_some() && slice_element_ty(types, t).is_none()
                    }) && ptr_zero.contains(src)
                }
                _ => false,
            }
        }
        OpKind::Binary {
            op: BinaryOp::Offset,
            l,
            r,
            ..
        } => is_const_zero_scalar(body, *r) && ptr_zero.contains(l),
        _ => false,
    }
}
