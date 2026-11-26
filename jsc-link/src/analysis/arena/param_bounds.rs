//! Stage 2 parameter arena-index & equal-length slice summaries, plus counted `0..N` single-push
//! loop detection shared with `opt/range.rs`.

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::index_vec::IndexVec;
use rcgjs_object::jir::repr::{
    CellId, Dominators, Func, OpKind, Proj, RBlockId, RTerm, Root, ValueId,
};
use rcgjs_object::jir::semantic::{
    BinaryOp, CallSemantic, Callee, ConstTree, DefinitionId, DropGlue, ItemId, StandardOperation,
    UnaryOp,
};
use rcgjs_object::jir::types::{TypeKind, TypeTable};

use crate::analysis::heap::is_global_vec_ty;
use crate::lower::js::value;

use super::context::{
    resolve_holder_arg, value_ne_const_at, ArenaContextBuilder, ArenaScanTables, FuncArenaContext,
    ParamArenaCandidateMap, ValueClassifier,
};
use super::fixpoint::{merge_field_kind, CandidateFieldKind, CandidateFieldsMap, ValueArenaClass};
use super::{
    holder_vec_fields, is_empty_vec_val, is_unsigned_index_ty, propagate_cell_borrows,
    slice_elem_def, vec_elem_def, vec_ref_elem_def, ArenaFieldKind, LiveProgram, ParamArenaIndex,
    ParamSliceEqLen, ProgramBoundsFacts,
};

type ParamCallSiteMap = BTreeMap<ItemId, Vec<(usize, RBlockId, usize, Vec<ValueId>)>>;

/// Stage 2: Propagates `ParamArenaIndex` for closed, non-root, non-exported, non-address-taken
/// internal functions (e.g. `earcut::{filter_points_sl, is_valid_diagonal_sl, is_ear_hashed_sl,
/// intersects_sl, earcut_linked, eliminate_hole, ...}` and `d3-hierarchy::tidy::{contour, apportion}`).
pub(super) fn analyze_param_arena_indices(
    program: &LiveProgram<'_>,
    facts: &mut ProgramBoundsFacts,
) {
    if facts.arenas.is_empty() {
        return;
    }

    let mut candidates: ParamArenaCandidateMap = BTreeMap::new();
    let mut call_sites: ParamCallSiteMap = BTreeMap::new();

    for &(_, _, func, interns) in &program.live_items {
        if !program.is_closed_internal(func.item) {
            continue;
        }
        let types = &interns.types;
        let mut arena_params: Vec<(usize, Option<u32>, DefinitionId)> = Vec::new();
        for (pi, &pv) in func.params.iter().enumerate() {
            let pty = func.values[pv.0 as usize].ty;
            if let Some(def) = slice_elem_def(types, pty)
                .or_else(|| vec_ref_elem_def(types, pty))
                .or_else(|| vec_elem_def(types, pty))
            {
                if facts.arenas.contains_key(&def) {
                    arena_params.push((pi, None, def));
                }
            }
            for &def in facts.arenas.keys() {
                for (_, hf) in holder_vec_fields(types, pty, def) {
                    arena_params.push((pi, Some(hf), def));
                }
            }
        }
        if arena_params.is_empty() {
            continue;
        }
        for (pi, &pv) in func.params.iter().enumerate() {
            if is_unsigned_index_ty(types, func.values[pv.0 as usize].ty).is_none() {
                continue;
            }
            for &(ap, hf, def) in &arena_params {
                if ap != pi {
                    candidates.insert((func.item, pi, ap, hf, def), None);
                }
            }
        }
    }
    if candidates.is_empty() {
        return;
    }

    let candidate_callees: BTreeSet<ItemId> = candidates.keys().map(|k| k.0).collect();
    for (func_idx, &(_, _, func, _)) in program.live_items.iter().enumerate() {
        if !program.is_user_item(func.item) {
            continue;
        }
        for block in &func.blocks {
            for (oi, op) in block.ops.iter().enumerate() {
                if let OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    args,
                    ..
                } = &op.kind
                {
                    if candidate_callees.contains(item) {
                        call_sites.entry(*item).or_default().push((
                            func_idx,
                            block.id,
                            oi,
                            args.clone(),
                        ));
                    }
                }
            }
        }
    }

    candidates.retain(|k, _| call_sites.get(&k.0).is_some_and(|v| !v.is_empty()));
    if candidates.is_empty() {
        return;
    }

    let caller_indices: BTreeSet<usize> = call_sites
        .values()
        .flat_map(|v| v.iter().map(|(fi, _, _, _)| *fi))
        .collect();
    let proven_fields: BTreeMap<(DefinitionId, u32), ArenaFieldKind> = facts
        .arenas
        .iter()
        .flat_map(|(&d, inv)| inv.fields.iter().map(move |(&f, &k)| ((d, f), k)))
        .collect();
    let candidate_field_shapes: CandidateFieldsMap = facts
        .arenas
        .iter()
        .map(|(&d, inv)| {
            let m = inv
                .fields
                .iter()
                .map(|(&f, &k)| {
                    (
                        f,
                        match k {
                            ArenaFieldKind::VecOfValidIndices => {
                                CandidateFieldKind::VecIdx { bits: 32 }
                            }
                            _ => CandidateFieldKind::Scalar { bits: 32 },
                        },
                    )
                })
                .collect();
            (d, m)
        })
        .collect();

    let active_defs: Vec<DefinitionId> = facts.arenas.keys().copied().collect();
    let empty_ctors = BTreeMap::new();
    let tables = ArenaScanTables {
        active_defs: &active_defs,
        candidate_fields: &candidate_field_shapes,
        ctor_summaries: &empty_ctors,
    };
    let mut caller_ctxs: BTreeMap<usize, FuncArenaContext> = BTreeMap::new();
    let mut dummy_disq = BTreeSet::new();
    let mut dummy_writes = Vec::new();
    for &fi in &caller_indices {
        let (_, _, func, interns) = program.live_items[fi];
        let ctx = ArenaContextBuilder::build(
            fi,
            func,
            interns,
            &tables,
            &mut dummy_disq,
            &mut dummy_writes,
        );
        caller_ctxs.insert(fi, ctx);
    }

    // Greatest fixpoint: weaken/remove any `(callee, idx_param, ...)` whose callers don't all pass
    // a proven arena index for the matching arena handle.
    loop {
        let mut changed = false;
        let keys: Vec<_> = candidates.keys().copied().collect();
        for key in keys {
            let Some(&cur_sentinel) = candidates.get(&key) else {
                continue;
            };
            let (callee_id, idx_param, arena_param, holder_field, elem_def) = key;
            let Some(sites) = call_sites.get(&callee_id) else {
                candidates.remove(&key);
                changed = true;
                continue;
            };
            let mut next_kind = match cur_sentinel {
                None => ArenaFieldKind::AlwaysValid,
                Some(s) => ArenaFieldKind::ValidOrSentinel { sentinel: s },
            };
            let mut ok = true;
            for &(caller_idx, blk, oi, ref args) in sites {
                let (Some(&arg_idx), Some(&arg_arena)) =
                    (args.get(idx_param), args.get(arena_param))
                else {
                    ok = false;
                    break;
                };
                let (_, _, caller_func, caller_interns) = program.live_items[caller_idx];
                let Some(ctx) = caller_ctxs.get(&caller_idx) else {
                    ok = false;
                    break;
                };
                let caller_handle = match holder_field {
                    None => ctx.val_arena[arg_arena]
                        .filter(|&(d, _)| d == elem_def)
                        .map(|(_, h)| h),
                    Some(hf) => resolve_holder_arg(
                        caller_func,
                        &caller_interns.types,
                        ctx.defs.as_slice(),
                        arg_arena,
                        &active_defs,
                    )
                    .filter(|&(d, f, _)| d == elem_def && f == hf)
                    .map(|(_, _, h)| h),
                };
                let Some(handle) = caller_handle else {
                    ok = false;
                    break;
                };

                // Unified Stage-2 value classifier using proven fields, `return_arena_index`,
                // AND caller's own surviving `candidates` (`ParamArenaIndex`)!
                let classifier = ValueClassifier {
                    func: caller_func,
                    types: &caller_interns.types,
                    ctx,
                    elem_def,
                    handle,
                    field_state: &proven_fields,
                    ret_idx_state: &facts.return_arena_index,
                    param_candidates: Some(&candidates),
                };
                let mut cls = classifier.classify(blk, oi, arg_idx, None, false);
                if let ValueArenaClass::ValidOrSentinel(s) = cls {
                    if value_ne_const_at(ctx, blk, arg_idx, s) {
                        cls = ValueArenaClass::AlwaysValid;
                    }
                }
                match merge_field_kind(next_kind, cls, false) {
                    Some(nk) => next_kind = nk,
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                candidates.remove(&key);
                changed = true;
            } else {
                let updated_sentinel = match next_kind {
                    ArenaFieldKind::AlwaysValid | ArenaFieldKind::VecOfValidIndices => None,
                    ArenaFieldKind::ValidOrSentinel { sentinel } => Some(sentinel),
                };
                if updated_sentinel != cur_sentinel {
                    candidates.insert(key, updated_sentinel);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    for ((callee_id, idx_param, arena_param, holder_field, elem_def), sentinel) in candidates {
        facts
            .param_arena_indices
            .entry(callee_id)
            .or_default()
            .push(ParamArenaIndex {
                idx_param,
                arena_param,
                holder_field,
                elem_def,
                sentinel,
            });
    }
}

/// Stage 2: Proves `ParamSliceEqLen` for closed internal functions whose every caller passes two
/// slices/vecs of provably equal length (e.g. `d3-hierarchy::tidy::{contour, apportion}` where
/// `t` was created by a `0..nodes.len()` push loop).
pub(super) fn analyze_param_slice_eq_len(
    program: &LiveProgram<'_>,
    facts: &mut ProgramBoundsFacts,
) {
    let mut candidates: BTreeSet<(ItemId, usize, usize)> = BTreeSet::new();
    for &(_, _, func, interns) in &program.live_items {
        if !program.is_closed_internal(func.item) {
            continue;
        }
        let types = &interns.types;
        let slice_params: Vec<usize> = func
            .params
            .iter()
            .enumerate()
            .filter_map(|(pi, &pv)| {
                let pty = func.values[pv.0 as usize].ty;
                let pt = value::pointee_of(types, pty)?;
                matches!(
                    types.get(types.peeled(pt)).map(|d| &d.kind),
                    Some(TypeKind::Slice(_))
                )
                .then_some(pi)
            })
            .collect();
        for i in 0..slice_params.len() {
            for j in (i + 1)..slice_params.len() {
                candidates.insert((func.item, slice_params[i], slice_params[j]));
            }
        }
    }
    if candidates.is_empty() {
        return;
    }

    let callees: BTreeSet<ItemId> = candidates.iter().map(|c| c.0).collect();
    let mut call_sites: BTreeMap<ItemId, Vec<(usize, Vec<ValueId>)>> = BTreeMap::new();
    for (fi, &(_, _, func, _)) in program.live_items.iter().enumerate() {
        if !program.is_user_item(func.item) {
            continue;
        }
        for block in &func.blocks {
            for op in &block.ops {
                if let OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    args,
                    ..
                } = &op.kind
                {
                    if callees.contains(item) {
                        call_sites
                            .entry(*item)
                            .or_default()
                            .push((fi, args.clone()));
                    }
                }
            }
        }
    }
    candidates.retain(|c| call_sites.get(&c.0).is_some_and(|s| !s.is_empty()));
    if candidates.is_empty() {
        return;
    }

    let mut caller_eq_pairs: BTreeMap<usize, BTreeSet<(ValueId, ValueId)>> = BTreeMap::new();
    for sites in call_sites.values() {
        for &(fi, _) in sites {
            caller_eq_pairs.entry(fi).or_insert_with(|| {
                let (_, _, func, interns) = program.live_items[fi];
                local_equal_len_slices(func, &interns.types)
            });
        }
    }

    loop {
        let mut changed = false;
        let snapshot: Vec<_> = candidates.iter().copied().collect();
        for (callee_id, pa, pb) in snapshot {
            let Some(sites) = call_sites.get(&callee_id) else {
                candidates.remove(&(callee_id, pa, pb));
                changed = true;
                continue;
            };
            let ok = sites.iter().all(|(fi, args)| {
                let (Some(&va), Some(&vb)) = (args.get(pa), args.get(pb)) else {
                    return false;
                };
                if va == vb {
                    return true;
                }
                if caller_eq_pairs
                    .get(fi)
                    .is_some_and(|set| set.contains(&(va, vb)) || set.contains(&(vb, va)))
                {
                    return true;
                }
                let (_, _, cfunc, _) = program.live_items[*fi];
                let idx_a = cfunc.params.iter().position(|&p| p == va);
                let idx_b = cfunc.params.iter().position(|&p| p == vb);
                if let (Some(ia), Some(ib)) = (idx_a, idx_b) {
                    let (lo, hi) = if ia < ib { (ia, ib) } else { (ib, ia) };
                    return candidates.contains(&(cfunc.item, lo, hi));
                }
                false
            });
            if !ok {
                candidates.remove(&(callee_id, pa, pb));
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    for (callee_id, pa, pb) in candidates {
        facts
            .param_slice_eq_len
            .entry(callee_id)
            .or_default()
            .push(ParamSliceEqLen {
                param_a: pa,
                param_b: pb,
            });
    }
}

/// Computes pairs `(s1, s2)` of slice values in `func` proven to have identical `.len()`
/// (from `vec![0; s.len()]`, `Vec::from_elem(x, s.len())`, or an exact `0..s.len()` unconditional
/// push loop on a local non-escaping `Vec` cell).
fn local_equal_len_slices(func: &Func, types: &TypeTable) -> BTreeSet<(ValueId, ValueId)> {
    let n = func.values.len();
    let doms = Dominators::of(func);
    let mut defs = IndexVec::from_elem(None, n);
    let mut canon = IndexVec::from_fn(n, |v| v);
    for b in &func.blocks {
        for (i, op) in b.ops.iter().enumerate() {
            if let Some(res) = op.result {
                defs[res] = Some((b.id, i));
            }
        }
    }
    for &b in &doms.rpo {
        for op in &func.blocks[b.0 as usize].ops {
            let Some(res) = op.result else { continue };
            match &op.kind {
                OpKind::Copy(s) | OpKind::Move(s) => canon[res] = canon[*s],
                OpKind::BorrowView {
                    root: Root::Value(s),
                    path,
                    ..
                } if matches!(path.as_slice(), [Proj::Deref]) => {
                    canon[res] = canon[*s];
                }
                _ => {}
            }
        }
    }

    // Map `len_val -> slice_canon`.
    let mut len_of_slice: IndexVec<ValueId, Option<ValueId>> = IndexVec::from_elem(None, n);
    for &b in &doms.rpo {
        for op in &func.blocks[b.0 as usize].ops {
            let Some(res) = op.result else { continue };
            if let OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v,
                ..
            } = op.kind
            {
                len_of_slice[canon[res]] = Some(canon[v]);
            }
        }
    }

    let mut cell_eq_slice: BTreeMap<CellId, ValueId> = BTreeMap::new();
    let mut cell_borrows: BTreeMap<ValueId, CellId> = BTreeMap::new();
    for b in &func.blocks {
        for op in &b.ops {
            if let (
                Some(res),
                OpKind::BorrowView {
                    root: Root::Cell(c),
                    path,
                    ..
                },
            ) = (op.result, &op.kind)
            {
                if path.is_empty() && is_global_vec_ty(types, func.cells[c.0 as usize].ty) {
                    cell_borrows.insert(res, *c);
                }
            }
        }
    }
    propagate_cell_borrows(func, &doms, &mut cell_borrows);

    for (ci, cell) in func.cells.iter().enumerate() {
        if !is_global_vec_ty(types, cell.ty) {
            continue;
        }
        let cid = CellId(ci as u32);
        if let Some(bound_len) = find_counted_push_loop_len(
            func,
            types,
            &doms,
            defs.as_slice(),
            canon.as_slice(),
            cid,
            &cell_borrows,
        ) {
            if let Some(src_slice) = len_of_slice[canon[bound_len]] {
                cell_eq_slice.insert(cid, src_slice);
            }
        }
    }

    let mut out = BTreeSet::new();
    let mut slice_group: IndexVec<ValueId, Option<ValueId>> = IndexVec::from_elem(None, n);
    for &b in &doms.rpo {
        for op in &func.blocks[b.0 as usize].ops {
            let Some(res) = op.result else { continue };
            match &op.kind {
                OpKind::Copy(s) | OpKind::Move(s) => {
                    if let Some(g) = slice_group[*s] {
                        slice_group[res] = Some(g);
                    }
                }
                OpKind::BorrowView {
                    root: Root::Value(s),
                    path,
                    ..
                } if matches!(path.as_slice(), [Proj::Deref]) => {
                    if let Some(g) = slice_group[*s] {
                        slice_group[res] = Some(g);
                    }
                }
                OpKind::Call {
                    semantic:
                        CallSemantic::Standard(
                            StandardOperation::VecAsSlice
                            | StandardOperation::NativeVecAsSlice
                            | StandardOperation::VecAsMutSlice
                            | StandardOperation::NativeVecAsMutSlice,
                        ),
                    args,
                    ..
                } => {
                    if let Some(&a0) = args.first() {
                        if let Some(&cid) = cell_borrows.get(&a0) {
                            if let Some(&src_s) = cell_eq_slice.get(&cid) {
                                slice_group[res] = Some(src_s);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    for (derived, &src_canon_opt) in slice_group.iter_enumerated() {
        let Some(src_canon) = src_canon_opt else {
            continue;
        };
        for (vi, &c) in canon.iter_enumerated() {
            if c == src_canon {
                out.insert((derived, vi));
            }
        }
    }
    out
}

/// Detects the exact `let mut v = Vec::with_capacity(N); for i in 0..N { v.push(...); }` pattern
/// on a local `Vec` cell `cid`:
/// - `cid` is stored ONCE with an empty `Vec` (`VecNew`/`VecWithCapacity`) before the loop header.
/// - `cid` has EXACTLY ONE `VecPush`/`NativeVecPush` op in the entire function, sitting inside a
///   single-header, single-backedge counted loop whose induction var starts at `0`, guards `i < N`,
///   and increments `i + 1` on the backedge, with the push dominating the backedge (and no other
///   length-changing op on `cid` anywhere in the function!).
///
/// Returns `Some(N)` if proven!
pub(crate) fn find_counted_push_loop_len(
    func: &Func,
    types: &TypeTable,
    doms: &Dominators,
    defs: &[Option<(RBlockId, usize)>],
    canon: &[ValueId],
    cid: CellId,
    cell_borrows: &BTreeMap<ValueId, CellId>,
) -> Option<ValueId> {
    find_counted_push_loop_details(func, types, doms, defs, canon, cid, cell_borrows)
        .map(|(_, _, _, bound)| bound)
}

struct CountedPushLoopCx<'a> {
    func: &'a Func,
    types: &'a TypeTable,
    doms: &'a Dominators,
    defs: &'a [Option<(RBlockId, usize)>],
    canon: &'a [ValueId],
    cid: CellId,
    cell_borrows: &'a BTreeMap<ValueId, CellId>,
}

impl CountedPushLoopCx<'_> {
    fn find_single_vec_push_site(&self) -> Option<(RBlockId, RBlockId, usize)> {
        let mut store_block = None;
        let mut push_site: Option<(RBlockId, usize)> = None;
        for b in &self.func.blocks {
            for (oi, op) in b.ops.iter().enumerate() {
                match &op.kind {
                    OpKind::CellStore { cell, v } if *cell == self.cid => {
                        if store_block.is_some() || !is_empty_vec_val(self.func, self.defs, *v) {
                            return None;
                        }
                        store_block = Some(b.id);
                    }
                    OpKind::BorrowView {
                        root: Root::Cell(c),
                        path,
                        ..
                    } if *c == self.cid && !path.is_empty() => {
                        return None;
                    }
                    OpKind::RawView {
                        root: Root::Cell(c),
                        ..
                    } if *c == self.cid => return None,
                    OpKind::Copy(_) | OpKind::Move(_) => {}
                    OpKind::BorrowView {
                        root: Root::Value(_),
                        path,
                        ..
                    } if matches!(path.as_slice(), [] | [Proj::Deref]) => {}
                    OpKind::Call { semantic, args, .. }
                        if args
                            .first()
                            .is_some_and(|a| self.cell_borrows.get(a) == Some(&self.cid)) =>
                    {
                        match semantic {
                            CallSemantic::Standard(
                                StandardOperation::VecPush | StandardOperation::NativeVecPush,
                            ) => {
                                if push_site.is_some() {
                                    return None;
                                }
                                push_site = Some((b.id, oi));
                            }
                            CallSemantic::Standard(
                                StandardOperation::VecAsSlice
                                | StandardOperation::NativeVecAsSlice
                                | StandardOperation::VecAsMutSlice
                                | StandardOperation::NativeVecAsMutSlice
                                | StandardOperation::VecLen
                                | StandardOperation::VecIsEmpty
                                | StandardOperation::VecIndex
                                | StandardOperation::VecIndexMut
                                | StandardOperation::VecReserve
                                | StandardOperation::NativeVecReserve
                                | StandardOperation::VecDrop,
                            ) => {}
                            _ => return None,
                        }
                    }
                    _ => {
                        for u in op.kind.uses() {
                            if self.cell_borrows.get(&u) == Some(&self.cid)
                                && !matches!(
                                    op.kind,
                                    OpKind::Drop {
                                        glue: DropGlue::None,
                                        ..
                                    }
                                )
                            {
                                return None;
                            }
                        }
                    }
                }
            }
        }
        let init_blk = store_block?;
        let (push_blk, push_oi) = push_site?;
        Some((init_blk, push_blk, push_oi))
    }

    fn match_counted_loop_header(
        &self,
        header: RBlockId,
        init_blk: RBlockId,
        push_blk: RBlockId,
        preds: &[(RBlockId, Vec<ValueId>)],
    ) -> Option<(ValueId, ValueId, RBlockId, RBlockId)> {
        let [(p0, a0), (p1, a1)] = preds else {
            return None;
        };
        let (pre, pre_args, back, back_args) =
            if self.doms.dominates(header, *p1) && !self.doms.dominates(header, *p0) {
                (*p0, a0, *p1, a1)
            } else if self.doms.dominates(header, *p0) && !self.doms.dominates(header, *p1) {
                (*p1, a1, *p0, a0)
            } else {
                return None;
            };
        if !self.doms.dominates(init_blk, pre) || !self.doms.dominates(push_blk, back) {
            return None;
        }
        let h_idx = header.0 as usize;
        let RTerm::Switch {
            scrut,
            cases,
            default,
            ..
        } = &self.func.blocks[h_idx].term
        else {
            return None;
        };
        let sc = self.canon[scrut.0 as usize];
        let (cb, ci) = self.defs[sc.0 as usize]?;
        let OpKind::Binary {
            op: BinaryOp::Lt,
            l: iv_val,
            r: bound_val,
            ty,
        } = self.func.blocks[cb.0 as usize].ops[ci].kind
        else {
            return None;
        };
        if !matches!(self.types.int_info(ty), Some((false, _))) {
            return None;
        }
        let (false_blk, true_blk) = match cases.as_slice() {
            [(0, fb, _)] => (*fb, default.0),
            [(1, tb, _)] => (default.0, *tb),
            [(0, fb, _), (1, tb, _)] | [(1, tb, _), (0, fb, _)] => (*fb, *tb),
            _ => return None,
        };
        if !self.doms.dominates(true_blk, push_blk) || self.doms.dominates(false_blk, push_blk) {
            return None;
        }

        let iv_c = self.canon[iv_val.0 as usize];
        let pos = self.func.blocks[h_idx]
            .params
            .iter()
            .position(|&p| self.canon[p.0 as usize] == iv_c)?;
        let init_v = self.canon[pre_args.get(pos)?.0 as usize];
        let back_v = self.canon[back_args.get(pos)?.0 as usize];
        let init_is_zero = self.defs[init_v.0 as usize].is_some_and(|(ib, ii)| {
            matches!(
                self.func.blocks[ib.0 as usize].ops[ii].kind,
                OpKind::Const(ConstTree::Scalar { bits: 0, .. })
            )
        });
        if !init_is_zero {
            return None;
        }
        let back_is_inc = self.defs[back_v.0 as usize].is_some_and(|(bb, bi)| {
            match self.func.blocks[bb.0 as usize].ops[bi].kind {
                OpKind::Binary {
                    op: BinaryOp::Add | BinaryOp::AddUnchecked,
                    l,
                    r,
                    ..
                } => {
                    let is_one = |v: ValueId| {
                        let cv = self.canon[v.0 as usize];
                        self.defs[cv.0 as usize].is_some_and(|(ob, oi)| {
                            matches!(
                                self.func.blocks[ob.0 as usize].ops[oi].kind,
                                OpKind::Const(ConstTree::Scalar { bits: 1, .. })
                            )
                        })
                    };
                    (self.canon[l.0 as usize] == iv_c && is_one(r))
                        || (self.canon[r.0 as usize] == iv_c && is_one(l))
                }
                _ => false,
            }
        });
        if !back_is_inc {
            return None;
        }
        let bound_dom = match self.defs[self.canon[bound_val.0 as usize].0 as usize] {
            Some((bb, _)) => self.doms.dominates(bb, header),
            None => true,
        };
        if !bound_dom {
            return None;
        }
        Some((iv_c, bound_val, true_blk, false_blk))
    }

    fn verify_single_exit_and_post_uses(
        &self,
        header: RBlockId,
        true_blk: RBlockId,
        false_blk: RBlockId,
    ) -> bool {
        let mut visited = BTreeSet::new();
        let mut stack = vec![true_blk];
        while let Some(lb) = stack.pop() {
            if lb == header || !visited.insert(lb) {
                continue;
            }
            if lb == false_blk {
                return false;
            }
            match self.func.blocks[lb.0 as usize].term {
                RTerm::Return(_) => return false,
                RTerm::Unreachable | RTerm::Throw(_) => continue,
                _ => {
                    for s in self.func.blocks[lb.0 as usize].term.successors() {
                        if s != header && !visited.contains(&s) {
                            stack.push(s);
                        }
                    }
                }
            }
        }
        for b in &self.func.blocks {
            for op in &b.ops {
                if let OpKind::Call {
                    semantic:
                        CallSemantic::Standard(
                            StandardOperation::VecAsSlice
                            | StandardOperation::NativeVecAsSlice
                            | StandardOperation::VecAsMutSlice
                            | StandardOperation::NativeVecAsMutSlice
                            | StandardOperation::VecLen,
                        ),
                    args,
                    ..
                } = &op.kind
                {
                    if args
                        .first()
                        .is_some_and(|a| self.cell_borrows.get(a) == Some(&self.cid))
                        && !self.doms.dominates(false_blk, b.id)
                    {
                        return false;
                    }
                }
            }
        }
        true
    }
}

pub(super) fn find_counted_push_loop_details(
    func: &Func,
    types: &TypeTable,
    doms: &Dominators,
    defs: &[Option<(RBlockId, usize)>],
    canon: &[ValueId],
    cid: CellId,
    cell_borrows: &BTreeMap<ValueId, CellId>,
) -> Option<(RBlockId, usize, ValueId, ValueId)> {
    let cx = CountedPushLoopCx {
        func,
        types,
        doms,
        defs,
        canon,
        cid,
        cell_borrows,
    };
    let (init_blk, push_blk, push_oi) = cx.find_single_vec_push_site()?;

    let mut preds: Vec<Vec<(RBlockId, Vec<ValueId>)>> = vec![Vec::new(); func.blocks.len()];
    for b in &func.blocks {
        for (target, args) in b.term.edges() {
            preds[target.0 as usize].push((b.id, args.to_vec()));
        }
    }

    let mut cur = push_blk;
    for _ in 0..16 {
        let h_idx = cur.0 as usize;
        if preds[h_idx].len() == 2 {
            let (p0, _) = &preds[h_idx][0];
            let (p1, _) = &preds[h_idx][1];
            let has_backedge = (doms.dominates(cur, *p1) && !doms.dominates(cur, *p0))
                || (doms.dominates(cur, *p0) && !doms.dominates(cur, *p1));
            if has_backedge {
                let (iv_c, bound_val, true_blk, false_blk) =
                    cx.match_counted_loop_header(cur, init_blk, push_blk, &preds[h_idx])?;
                if !cx.verify_single_exit_and_post_uses(cur, true_blk, false_blk) {
                    return None;
                }
                return Some((push_blk, push_oi, iv_c, bound_val));
            }
        }
        match doms.idom[h_idx] {
            Some(up) if up != cur => cur = up,
            _ => return None,
        }
    }
    None
}
