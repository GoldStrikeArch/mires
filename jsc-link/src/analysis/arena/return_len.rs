//! Stage 2 interprocedural return-`Vec` length and return-`Vec` element upper-bound summaries.

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::index_vec::IndexVec;
use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::repr::{
    CellId, Dominators, Func, OpKind, Proj, RBlockId, RTerm, Root, ValueId,
};
use rcgjs_object::jir::semantic::{
    BinaryOp, CallSemantic, Callee, CastKind, ConstTree, DropGlue, StandardOperation, UnaryOp,
};
use rcgjs_object::jir::types::{TypeKind, TypeTable};

use crate::analysis::heap::is_global_vec_ty;
use crate::lower::js::value;

use super::{
    is_empty_vec_val, is_unsigned_vec_ty, mask_bits, peel_copy_move_local, propagate_cell_borrows,
    width_max, LiveProgram, ProgramBoundsFacts, ReturnVecElemBound, ReturnVecLen,
};

pub(super) fn analyze_return_vec_lens(program: &LiveProgram<'_>, facts: &mut ProgramBoundsFacts) {
    for &(_, _, func, interns) in &program.live_items {
        if !program.is_user_item(func.item) {
            continue;
        }
        let types = &interns.types;
        if !is_global_vec_ty(types, func.sig.result) {
            continue;
        }
        if let Some(summary) = summarize_return_vec_len(func, types) {
            facts.return_vec_len.insert(func.item, summary);
        }
    }
}

struct ReturnVecLenCx<'a> {
    func: &'a Func,
    types: &'a TypeTable,
    doms: Dominators,
    defs: IndexVec<ValueId, Option<(RBlockId, usize)>>,
    param_idx: IndexVec<ValueId, Option<usize>>,
}

impl<'a> ReturnVecLenCx<'a> {
    fn new(func: &'a Func, types: &'a TypeTable) -> Self {
        let n = func.values.len();
        let doms = Dominators::of(func);
        let mut defs = IndexVec::from_elem(None, n);
        for b in &func.blocks {
            for (i, op) in b.ops.iter().enumerate() {
                if let Some(res) = op.result {
                    defs[res] = Some((b.id, i));
                }
            }
        }
        let mut param_idx = IndexVec::from_elem(None, n);
        for (i, &p) in func.params.iter().enumerate() {
            param_idx[p] = Some(i);
        }
        Self {
            func,
            types,
            doms,
            defs,
            param_idx,
        }
    }

    fn expr_len(&self, v: ValueId) -> Option<ReturnVecLen> {
        let p = peel_copy_move_local(self.func, self.defs.as_slice(), v);
        if let Some(pi) = self.param_idx[p] {
            return Some(ReturnVecLen::Param(pi));
        }
        let (db, di) = self.defs[p]?;
        match &self.func.blocks[db.0 as usize].ops[di].kind {
            OpKind::Const(ConstTree::Scalar { bits, bytes }) => {
                let max = self
                    .types
                    .int_info(self.types.peeled(self.func.values[p.0 as usize].ty))
                    .map_or(u128::MAX, |(_, w)| width_max(w));
                Some(ReturnVecLen::Const(mask_bits(*bits, *bytes).min(max)))
            }
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v: slice_v,
                ..
            } => {
                let sp = peel_copy_move_local(self.func, self.defs.as_slice(), *slice_v);
                self.param_idx[sp].map(ReturnVecLen::ParamSliceLen)
            }
            OpKind::Binary {
                op: BinaryOp::Shl | BinaryOp::ShlUnchecked,
                l,
                r,
                ty,
            } => {
                let (_, bits) = self.types.int_info(*ty)?;
                let lp = peel_copy_move_local(self.func, self.defs.as_slice(), *l);
                let rp = peel_copy_move_local(self.func, self.defs.as_slice(), *r);
                let l_is_one = self.defs[lp].is_some_and(|(lb, li)| {
                    matches!(
                        self.func.blocks[lb.0 as usize].ops[li].kind,
                        OpKind::Const(ConstTree::Scalar { bits: 1, .. })
                    )
                });
                if l_is_one {
                    self.param_idx[rp].map(|pi| ReturnVecLen::OneShlParam {
                        shift_param: pi,
                        bits,
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn vec_val_len(
        &self,
        v: ValueId,
        cell_lens: &BTreeMap<CellId, ReturnVecLen>,
    ) -> Option<ReturnVecLen> {
        let p = peel_copy_move_local(self.func, self.defs.as_slice(), v);
        let (db, di) = self.defs[p]?;
        match &self.func.blocks[db.0 as usize].ops[di].kind {
            OpKind::CellLoad(c) => cell_lens.get(c).cloned(),
            OpKind::Call {
                semantic: CallSemantic::Standard(StandardOperation::NativeVecFromElemZeroed),
                args,
                ..
            } if !args.is_empty() => self.expr_len(args[0]),
            OpKind::Call {
                semantic: CallSemantic::Standard(StandardOperation::VecFromElem),
                args,
                ..
            } if args.len() >= 2 => self.expr_len(args[1]),
            OpKind::Call {
                semantic:
                    CallSemantic::Standard(
                        StandardOperation::VecNew
                        | StandardOperation::NativeVecNew
                        | StandardOperation::VecWithCapacity
                        | StandardOperation::NativeVecWithCapacity,
                    ),
                ..
            } => Some(ReturnVecLen::Const(0)),
            OpKind::Call {
                semantic: CallSemantic::Standard(StandardOperation::SliceToVec),
                args,
                ..
            } if !args.is_empty() => {
                let sp = peel_copy_move_local(self.func, self.defs.as_slice(), args[0]);
                self.param_idx[sp].map(ReturnVecLen::ParamSliceLen)
            }
            _ => None,
        }
    }

    /// Tracks single-store local `Vec` cells whose length is never mutated after initialization.
    fn collect_vec_cell_init_lens(&self) -> BTreeMap<CellId, ReturnVecLen> {
        let mut cell_init_len: BTreeMap<CellId, ReturnVecLen> = BTreeMap::new();
        let mut cell_disqualified: BTreeSet<CellId> = BTreeSet::new();
        let mut cell_borrows: BTreeMap<ValueId, CellId> = BTreeMap::new();
        let mut cell_stores: BTreeMap<CellId, usize> = BTreeMap::new();

        for b in &self.func.blocks {
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
                    if path.is_empty()
                        && is_global_vec_ty(self.types, self.func.cells[c.0 as usize].ty)
                    {
                        cell_borrows.insert(res, *c);
                    } else {
                        cell_disqualified.insert(*c);
                    }
                }
            }
        }
        propagate_cell_borrows(self.func, &self.doms, &mut cell_borrows);

        let empty_cells = BTreeMap::new();
        for b in &self.func.blocks {
            for op in &b.ops {
                match &op.kind {
                    OpKind::CellStore { cell, v }
                        if is_global_vec_ty(self.types, self.func.cells[cell.0 as usize].ty) =>
                    {
                        let cnt = cell_stores.entry(*cell).or_insert(0);
                        *cnt += 1;
                        if *cnt > 1 {
                            cell_disqualified.insert(*cell);
                        } else if let Some(len_s) = self.vec_val_len(*v, &empty_cells) {
                            cell_init_len.insert(*cell, len_s);
                        } else {
                            cell_disqualified.insert(*cell);
                        }
                    }
                    OpKind::Call { semantic, args, .. } => {
                        for &a in args {
                            if let Some(&cid) = cell_borrows.get(&a) {
                                let ok = matches!(
                                    semantic,
                                    CallSemantic::Standard(
                                        StandardOperation::VecAsSlice
                                            | StandardOperation::NativeVecAsSlice
                                            | StandardOperation::VecAsMutSlice
                                            | StandardOperation::NativeVecAsMutSlice
                                            | StandardOperation::VecLen
                                            | StandardOperation::VecIsEmpty
                                            | StandardOperation::VecIndex
                                            | StandardOperation::VecIndexMut
                                            | StandardOperation::VecIter
                                            | StandardOperation::VecIterMut
                                            | StandardOperation::VecDrop
                                    )
                                );
                                if !ok {
                                    cell_disqualified.insert(cid);
                                }
                            }
                        }
                    }
                    OpKind::Copy(_)
                    | OpKind::Move(_)
                    | OpKind::Drop {
                        glue: DropGlue::None,
                        ..
                    } => {}
                    OpKind::BorrowView {
                        root: Root::Value(_),
                        path,
                        ..
                    } if matches!(path.as_slice(), [] | [Proj::Deref]) => {}
                    _ => {
                        for u in op.kind.uses() {
                            if let Some(&cid) = cell_borrows.get(&u) {
                                cell_disqualified.insert(cid);
                            }
                        }
                    }
                }
            }
        }
        cell_init_len.retain(|c, _| !cell_disqualified.contains(c));
        cell_init_len
    }

    /// Collects `(origin_block, ReturnVecLen)` for every return, unfolding a merged block-param return.
    fn collect_returns(
        &self,
        cell_init_len: &BTreeMap<CellId, ReturnVecLen>,
    ) -> Option<Vec<(RBlockId, ReturnVecLen)>> {
        let mut returns: Vec<(RBlockId, ReturnVecLen)> = Vec::new();
        for b in &self.func.blocks {
            if let RTerm::Return(Some(ret_v)) = b.term {
                if let Some(len_s) = self.vec_val_len(ret_v, cell_init_len) {
                    returns.push((b.id, len_s));
                } else {
                    let pv = peel_copy_move_local(self.func, self.defs.as_slice(), ret_v);
                    let pos = b.params.iter().position(|&p| p == pv)?;
                    let mut edge_rets = Vec::new();
                    let mut ok = true;
                    for pred in &self.func.blocks {
                        for (target, args) in pred.term.edges() {
                            if target == b.id {
                                if let Some(len_s) = args
                                    .get(pos)
                                    .and_then(|&a| self.vec_val_len(a, cell_init_len))
                                {
                                    edge_rets.push((pred.id, len_s));
                                } else {
                                    ok = false;
                                }
                            }
                        }
                    }
                    if ok && !edge_rets.is_empty() {
                        returns.extend(edge_rets);
                    } else {
                        return None;
                    }
                }
            }
        }
        (!returns.is_empty()).then_some(returns)
    }

    /// Matches a two-way conditional return guarded by a `bool` entry parameter (`fflate::h_map`).
    fn match_cond_bool_return(&self, returns: &[(RBlockId, ReturnVecLen)]) -> Option<ReturnVecLen> {
        for b in &self.func.blocks {
            let RTerm::Switch {
                scrut,
                cases,
                default,
                ..
            } = &b.term
            else {
                continue;
            };
            let sp = peel_copy_move_local(self.func, self.defs.as_slice(), *scrut);
            let Some(cond_param) = self.param_idx[sp] else {
                continue;
            };
            if !matches!(
                self.types
                    .get(self.types.peeled(self.func.values[sp.0 as usize].ty))
                    .map(|d| &d.kind),
                Some(TypeKind::Bool)
            ) {
                continue;
            }
            let (false_blk, true_blk) = match cases.as_slice() {
                [(0, fb, _)] => (*fb, default.0),
                [(1, tb, _)] => (default.0, *tb),
                [(0, fb, _), (1, tb, _)] | [(1, tb, _), (0, fb, _)] => (*fb, *tb),
                _ => continue,
            };
            if false_blk == true_blk {
                continue;
            }
            let mut when_true = None;
            let mut when_false = None;
            for (rb, len_s) in returns {
                if self.doms.dominates(true_blk, *rb) {
                    when_true = Some(len_s.clone());
                } else if self.doms.dominates(false_blk, *rb) {
                    when_false = Some(len_s.clone());
                }
            }
            if let (Some(wt), Some(wf)) = (when_true, when_false) {
                return Some(ReturnVecLen::CondBoolParam {
                    cond_param,
                    when_true: Box::new(wt),
                    when_false: Box::new(wf),
                });
            }
        }
        None
    }
}

fn summarize_return_vec_len(func: &Func, types: &TypeTable) -> Option<ReturnVecLen> {
    let cx = ReturnVecLenCx::new(func, types);
    let cell_init_len = cx.collect_vec_cell_init_lens();
    let returns = cx.collect_returns(&cell_init_len)?;
    if returns.iter().all(|(_, l)| l == &returns[0].1) {
        return Some(returns[0].1.clone());
    }
    if returns.len() == 2 {
        return cx.match_cond_bool_return(&returns);
    }
    None
}

/// Proves `ReturnVecElemBound::LtParamSliceLen { slice_param }` for functions like
/// `d3-hierarchy::walk(nodes: &[Node], root: usize, post: bool) -> Vec<usize>` where every element
/// pushed into the returned `Vec` is checked `< nodes.len()`.
pub(super) fn analyze_return_vec_elem_bounds(
    program: &LiveProgram<'_>,
    facts: &mut ProgramBoundsFacts,
) {
    for &(_, _, func, interns) in &program.live_items {
        if !program.is_user_item(func.item) {
            continue;
        }
        let types = &interns.types;
        if is_unsigned_vec_ty(types, func.sig.result).is_none() {
            continue;
        }
        // Check each slice parameter as a potential bound target.
        for (p_idx, &p_val) in func.params.iter().enumerate() {
            let p_ty = func.values[p_val.0 as usize].ty;
            let Some(pt) = value::pointee_of(types, p_ty) else {
                continue;
            };
            if !matches!(
                types.get(types.peeled(pt)).map(|d| &d.kind),
                Some(TypeKind::Slice(_))
            ) {
                continue;
            }
            if func_returns_vec_lt_param_slice(func, types, interns, p_val) {
                facts.return_vec_elem_bound.insert(
                    func.item,
                    ReturnVecElemBound::LtParamSliceLen { slice_param: p_idx },
                );
                break;
            }
        }
    }
}

struct ReturnVecAuditCx<'a> {
    func: &'a Func,
    interns: &'a Interns,
    doms: Dominators,
    defs: IndexVec<ValueId, Option<(RBlockId, usize)>>,
    canon: IndexVec<ValueId, ValueId>,
    checked_lt: Vec<(RBlockId, Option<usize>, ValueId)>,
}

fn func_returns_vec_lt_param_slice(
    func: &Func,
    types: &TypeTable,
    interns: &Interns,
    slice_param_val: ValueId,
) -> bool {
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
                OpKind::Cast {
                    kind: CastKind::IntToInt,
                    v,
                    from,
                    to,
                } if matches!(
                    (types.int_info(*from), types.int_info(*to)),
                    (Some((false, a)), Some((false, b))) if a == b
                ) =>
                {
                    canon[res] = canon[*v];
                }
                _ => {}
            }
        }
    }

    let checked_lt = collect_slice_lt_asserts(func, types, &doms, &defs, &canon, slice_param_val);
    let Some(ret_cells) = find_returned_vec_cells(func, &defs) else {
        return false;
    };
    let cx = ReturnVecAuditCx {
        func,
        interns,
        doms,
        defs,
        canon,
        checked_lt,
    };
    ret_cells
        .into_iter()
        .all(|rc| audit_returned_vec_cell(&cx, rc))
}

fn collect_slice_lt_asserts(
    func: &Func,
    types: &TypeTable,
    doms: &Dominators,
    defs: &IndexVec<ValueId, Option<(RBlockId, usize)>>,
    canon: &IndexVec<ValueId, ValueId>,
    slice_param_val: ValueId,
) -> Vec<(RBlockId, Option<usize>, ValueId)> {
    let n = func.values.len();
    let mut slice_vals = IndexVec::from_elem(false, n);
    slice_vals[slice_param_val] = true;
    let mut len_vals = IndexVec::from_elem(false, n);
    for &b in &doms.rpo {
        for op in &func.blocks[b.0 as usize].ops {
            let Some(res) = op.result else { continue };
            match &op.kind {
                OpKind::Copy(s) | OpKind::Move(s) => {
                    if slice_vals[*s] {
                        slice_vals[res] = true;
                    }
                    if len_vals[*s] {
                        len_vals[res] = true;
                    }
                }
                OpKind::BorrowView {
                    root: Root::Value(s),
                    path,
                    ..
                } if matches!(path.as_slice(), [Proj::Deref]) && slice_vals[*s] => {
                    slice_vals[res] = true;
                }
                OpKind::Unary {
                    op: UnaryOp::PtrMetadata,
                    v,
                    ..
                } if slice_vals[*v] => {
                    len_vals[canon[res]] = true;
                }
                _ => {}
            }
        }
    }

    let mut checked_lt: Vec<(RBlockId, Option<usize>, ValueId)> = Vec::new();
    for b in &func.blocks {
        for (oi, op) in b.ops.iter().enumerate() {
            if let OpKind::Assert {
                cond,
                expected: true,
                ..
            } = &op.kind
            {
                let cc = canon[*cond];
                if let Some((db, di)) = defs[cc] {
                    if let OpKind::Binary {
                        op: BinaryOp::Lt,
                        l,
                        r,
                        ty,
                    } = func.blocks[db.0 as usize].ops[di].kind
                    {
                        if matches!(types.int_info(ty), Some((false, _))) && len_vals[canon[r]] {
                            checked_lt.push((b.id, Some(oi), canon[l]));
                        }
                    }
                }
            }
        }
    }
    checked_lt
}

fn find_returned_vec_cells(
    func: &Func,
    defs: &IndexVec<ValueId, Option<(RBlockId, usize)>>,
) -> Option<BTreeSet<CellId>> {
    let mut ret_cells = BTreeSet::new();
    for b in &func.blocks {
        if let RTerm::Return(Some(rv)) = b.term {
            let pv = peel_copy_move_local(func, defs.as_slice(), rv);
            let (db, di) = defs[pv]?;
            match func.blocks[db.0 as usize].ops[di].kind {
                OpKind::CellLoad(c) => {
                    ret_cells.insert(c);
                }
                _ => return None,
            }
        }
    }
    (!ret_cells.is_empty()).then_some(ret_cells)
}

/// Verifies `rc` is initialized empty, only mutated via allowed ops (`VecPush`, `SliceSwap`,
/// `SliceSort`, `reverse`, `VecReserve*`), and every pushed value is proven `< slice_param.len()`.
fn audit_returned_vec_cell(cx: &ReturnVecAuditCx<'_>, rc: CellId) -> bool {
    let n = cx.func.values.len();
    let mut borrows = IndexVec::from_elem(false, n);
    let mut mut_slices = IndexVec::from_elem(false, n);
    let mut stores = 0usize;
    let mut pushed: Vec<(RBlockId, usize, ValueId)> = Vec::new();

    for &bid in &cx.doms.rpo {
        let b = &cx.func.blocks[bid.0 as usize];
        for op in &b.ops {
            match &op.kind {
                OpKind::BorrowView {
                    root: Root::Cell(c),
                    path,
                    ..
                } if *c == rc => {
                    if !path.is_empty() {
                        return false;
                    }
                    if let Some(res) = op.result {
                        borrows[res] = true;
                    }
                }
                OpKind::Copy(s) | OpKind::Move(s) => {
                    if let Some(res) = op.result {
                        if borrows[*s] {
                            borrows[res] = true;
                        }
                        if mut_slices[*s] {
                            mut_slices[res] = true;
                        }
                    }
                }
                OpKind::BorrowView {
                    root: Root::Value(s),
                    path,
                    ..
                } if matches!(path.as_slice(), [] | [Proj::Deref]) => {
                    if let Some(res) = op.result {
                        if borrows[*s] {
                            borrows[res] = true;
                        }
                        if mut_slices[*s] {
                            mut_slices[res] = true;
                        }
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
                } if args.first().is_some_and(|&a| borrows[a]) => {
                    if let Some(res) = op.result {
                        mut_slices[res] = true;
                    }
                }
                _ => {}
            }
        }
    }

    for b in &cx.func.blocks {
        for (oi, op) in b.ops.iter().enumerate() {
            match &op.kind {
                OpKind::CellStore { cell, v } if *cell == rc => {
                    stores += 1;
                    if stores > 1 || !is_empty_vec_val(cx.func, cx.defs.as_slice(), *v) {
                        return false;
                    }
                }
                OpKind::BorrowView {
                    root: Root::Cell(c),
                    ..
                } if *c == rc => {}
                OpKind::RawView {
                    root: Root::Cell(c),
                    ..
                } if *c == rc => return false,
                OpKind::Copy(_) | OpKind::Move(_) => {}
                OpKind::BorrowView {
                    root: Root::Value(_),
                    path,
                    ..
                } if matches!(path.as_slice(), [] | [Proj::Deref]) => {}
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
                } if args.first().is_some_and(|&a| borrows[a]) => {}
                OpKind::Call {
                    semantic:
                        CallSemantic::Standard(
                            StandardOperation::VecPush | StandardOperation::NativeVecPush,
                        ),
                    args,
                    ..
                } if args.len() >= 2 && borrows[args[0]] => {
                    pushed.push((b.id, oi, args[1]));
                }
                OpKind::Call {
                    callee,
                    semantic,
                    args,
                    ..
                } if args.iter().any(|&a| borrows[a] || mut_slices[a]) => {
                    let ok_std = matches!(
                        semantic,
                        CallSemantic::Standard(
                            StandardOperation::VecLen
                                | StandardOperation::VecIsEmpty
                                | StandardOperation::VecReserve
                                | StandardOperation::NativeVecReserve
                                | StandardOperation::SliceSwap
                                | StandardOperation::SliceSort
                                | StandardOperation::SliceSortCmp
                                | StandardOperation::VecDrop
                        )
                    );
                    let ok_rev = matches!(callee, Callee::Direct { .. })
                        && args.len() == 1
                        && mut_slices[args[0]]
                        && is_slice_reverse_call(cx.interns, callee);
                    if !ok_std && !ok_rev {
                        return false;
                    }
                }
                OpKind::Drop {
                    glue: DropGlue::None,
                    ..
                } => {}
                _ => {
                    for u in op.kind.uses() {
                        if borrows[u] || mut_slices[u] {
                            return false;
                        }
                    }
                }
            }
        }
    }

    if pushed.is_empty() {
        return false;
    }

    for (pb, poi, pv) in pushed {
        let cpv = cx.canon[pv];
        let dom_ok = cx.checked_lt.iter().any(|&(cb, coi, cv)| {
            cv == cpv
                && if cb == pb {
                    coi.is_none_or(|i| i < poi)
                } else {
                    cx.doms.dominates(cb, pb)
                }
        });
        if dom_ok {
            continue;
        }
        if !value_asserted_before_return(cx.func, pb, poi, cpv, &cx.checked_lt) {
            return false;
        }
    }
    true
}

fn is_slice_reverse_call(interns: &Interns, callee: &Callee) -> bool {
    let Callee::Direct { link_name, .. } = callee else {
        return false;
    };
    let Some(name) = interns.str(*link_name) else {
        return false;
    };
    name.starts_with("core$slice$") && (name.contains("_reverse$") || name.contains("$reverse$"))
}

/// Checks that along ALL paths starting after `(start_block, start_op)` before any `Return`,
/// `target_v < slice_len` is asserted.
fn value_asserted_before_return(
    func: &Func,
    start_block: RBlockId,
    start_op: usize,
    target_v: ValueId,
    checked_lt: &[(RBlockId, Option<usize>, ValueId)],
) -> bool {
    if checked_lt.iter().any(|&(cb, coi, cv)| {
        cb == start_block && cv == target_v && coi.is_some_and(|i| i > start_op)
    }) {
        return true;
    }
    if matches!(func.blocks[start_block.0 as usize].term, RTerm::Return(_)) {
        return false;
    }
    let mut visited = BTreeSet::new();
    let mut stack = func.blocks[start_block.0 as usize].term.successors();
    while let Some(b) = stack.pop() {
        if !visited.insert(b) {
            continue;
        }
        if checked_lt
            .iter()
            .any(|&(cb, _, cv)| cb == b && cv == target_v)
        {
            continue;
        }
        match func.blocks[b.0 as usize].term {
            RTerm::Return(_) => return false,
            RTerm::Unreachable | RTerm::Throw(_) => continue,
            _ => {
                for s in func.blocks[b.0 as usize].term.successors() {
                    if !visited.contains(&s) {
                        stack.push(s);
                    }
                }
            }
        }
    }
    true
}
