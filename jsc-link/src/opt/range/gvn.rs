//! Global value numbering and exact `Vec`/slice length unification for [`Analysis`].

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::arena::{find_counted_push_loop_len, ReturnVecLen};
use crate::analysis::heap::{is_direct_global_vec_ty, is_global_vec_ty, is_vec_len_field};
use crate::jir::repr::{CellId, Dominators, Func, OpKind, Proj, RBlockId, Root, ValueId};
use crate::jir::semantic::{
    BinaryOp, CallSemantic, Callee, CastKind, ConstTree, DropGlue, StandardOperation, TypeId,
    UnaryOp,
};
use crate::jir::types::TypeKind;
use crate::jir::TypeTable;

use super::{is_slice_pointer, is_vec_len_preserving_call, Analysis, RangeInputs, VecRootKey};

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq)]
enum GvnKey {
    Scalar(u128, u8, TypeId),
    Binary(u8, ValueId, ValueId, TypeId),
    Unary(u8, ValueId, TypeId),
    Cast(u8, ValueId, TypeId, TypeId),
}

/// Bundles exact dynamic, constant, and `1 << k` length tracking across `Vec` values, cells, and
/// storage roots during [`Analysis::number_values`].
struct VecLenTracker {
    vec_dyn_len: BTreeMap<ValueId, ValueId>,
    vec_const_len: BTreeMap<ValueId, u128>,
    vec_one_shl_len: BTreeMap<ValueId, (ValueId, u32)>,
    cell_dyn_len: BTreeMap<CellId, ValueId>,
    cell_const_len: BTreeMap<CellId, u128>,
    cell_one_shl_len: BTreeMap<CellId, (ValueId, u32)>,
    cell_borrows: BTreeMap<ValueId, CellId>,
    cell_len_mutated: BTreeSet<CellId>,
    val_vec_root: BTreeMap<ValueId, VecRootKey>,
    root_len_mutated: BTreeSet<VecRootKey>,
    unmutated_root_len: BTreeMap<VecRootKey, ValueId>,
    block_root_len: BTreeMap<VecRootKey, ValueId>,
    param_eq_slice_len: BTreeMap<ValueId, ValueId>,
}

fn is_immut_ref_ty(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        table.get(table.peeled(ty)).map(|d| &d.kind),
        Some(TypeKind::Reference { mutable: false, .. })
    )
}

impl VecLenTracker {
    /// Pre-scans local `Vec` cells, `VecRootKey` provenance, mutation sets, and `ParamSliceEqLen`
    /// equivalences before the RPO numbering pass.
    fn scan_vec_roots(doms: &Dominators, ctx: RangeInputs<'_>) -> Self {
        let func = ctx.func;
        let table = ctx.table;
        let (cell_borrows, cell_len_mutated) = Self::scan_vec_cells(doms, ctx);
        let val_vec_root = Self::scan_vec_root_keys(doms, func, table, &cell_borrows);
        let root_len_mutated =
            Self::collect_mutated_vec_roots(func, table, &cell_len_mutated, &val_vec_root);

        // Map `ParamSliceEqLen` equivalence: canon slice param `pb` -> canon slice param `pa`.
        let mut param_eq_slice_len: BTreeMap<ValueId, ValueId> = BTreeMap::new();
        if let Some(af) = ctx.arena_facts {
            if let Some(eqs) = af.param_slice_eq_len.get(&func.item) {
                for eq in eqs {
                    if let (Some(&va), Some(&vb)) =
                        (func.params.get(eq.param_a), func.params.get(eq.param_b))
                    {
                        param_eq_slice_len.insert(vb, va);
                    }
                }
            }
        }

        Self {
            vec_dyn_len: BTreeMap::new(),
            vec_const_len: BTreeMap::new(),
            vec_one_shl_len: BTreeMap::new(),
            cell_dyn_len: BTreeMap::new(),
            cell_const_len: BTreeMap::new(),
            cell_one_shl_len: BTreeMap::new(),
            cell_borrows,
            cell_len_mutated,
            val_vec_root,
            root_len_mutated,
            unmutated_root_len: BTreeMap::new(),
            block_root_len: BTreeMap::new(),
            param_eq_slice_len,
        }
    }

    /// Pre-scans local `Vec` cells to identify whole-cell borrows and length-mutated cells.
    fn scan_vec_cells(
        doms: &Dominators,
        ctx: RangeInputs<'_>,
    ) -> (BTreeMap<ValueId, CellId>, BTreeSet<CellId>) {
        let func = ctx.func;
        let table = ctx.table;
        let interns = ctx.interns;

        let mut cell_borrows: BTreeMap<ValueId, CellId> = BTreeMap::new();
        let mut cell_store_val: BTreeMap<CellId, Option<ValueId>> = BTreeMap::new();
        let mut cell_len_mutated: BTreeSet<CellId> = BTreeSet::new();
        for block in &func.blocks {
            for op in &block.ops {
                match &op.kind {
                    OpKind::BorrowView {
                        root: Root::Cell(c),
                        path,
                        ..
                    } if is_global_vec_ty(table, func.cells[c.0 as usize].ty) => {
                        if path.is_empty() {
                            if let Some(res) = op.result {
                                cell_borrows.insert(res, *c);
                            }
                        } else if !interns.is_some_and(|i| {
                            matches!(path.as_slice(), [Proj::Field { field, .. }] if is_vec_len_field(table, i, func.cells[c.0 as usize].ty, *field))
                        }) {
                            cell_len_mutated.insert(*c);
                        }
                    }
                    OpKind::RawView {
                        root: Root::Cell(c),
                        ..
                    } if is_global_vec_ty(table, func.cells[c.0 as usize].ty) => {
                        cell_len_mutated.insert(*c);
                    }
                    OpKind::CellStore { cell, v }
                        if is_global_vec_ty(table, func.cells[cell.0 as usize].ty) =>
                    {
                        cell_store_val
                            .entry(*cell)
                            .and_modify(|slot| {
                                *slot = None;
                                cell_len_mutated.insert(*cell);
                            })
                            .or_insert(Some(*v));
                    }
                    _ => {}
                }
            }
        }
        // Propagate `cell_borrows` through `Copy`/`Move`/whole-ref reborrows so copying `&v` or
        // `&mut v` doesn't falsely taint `v` as length-mutated.
        for &b in &doms.rpo {
            for op in &func.blocks[b.0 as usize].ops {
                let Some(res) = op.result else { continue };
                match &op.kind {
                    OpKind::Copy(s) | OpKind::Move(s) => {
                        if let Some(&cid) = cell_borrows.get(s) {
                            cell_borrows.insert(res, cid);
                        }
                    }
                    OpKind::BorrowView {
                        root: Root::Value(s),
                        path,
                        ..
                    } if matches!(path.as_slice(), [] | [Proj::Deref]) => {
                        if let Some(&cid) = cell_borrows.get(s) {
                            cell_borrows.insert(res, cid);
                        }
                    }
                    _ => {}
                }
            }
        }
        // Any call/op on a borrow of a `Vec` cell other than length-preserving ops or immutable
        // `&Vec<T>` calls marks it length-mutated.
        for block in &func.blocks {
            for op in &block.ops {
                match &op.kind {
                    OpKind::Call { semantic, args, .. } => {
                        for &a in args {
                            if let Some(&cid) = cell_borrows.get(&a) {
                                if !is_vec_len_preserving_call(*semantic)
                                    && !is_immut_ref_ty(table, func.values[a.0 as usize].ty)
                                {
                                    cell_len_mutated.insert(cid);
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
                                cell_len_mutated.insert(cid);
                            }
                        }
                    }
                }
            }
        }
        (cell_borrows, cell_len_mutated)
    }

    /// Maps each SSA value to its canonical `VecRootKey` for same-`Vec` length unification.
    fn scan_vec_root_keys(
        doms: &Dominators,
        func: &Func,
        table: &TypeTable,
        cell_borrows: &BTreeMap<ValueId, CellId>,
    ) -> BTreeMap<ValueId, VecRootKey> {
        let mut val_vec_root: BTreeMap<ValueId, VecRootKey> = BTreeMap::new();
        for (pi, &pv) in func.params.iter().enumerate() {
            let pty = func.values[pv.0 as usize].ty;
            if is_global_vec_ty(table, pty)
                || table
                    .pointee(pty)
                    .is_some_and(|pt| is_global_vec_ty(table, pt))
            {
                val_vec_root.insert(pv, VecRootKey::Param(pi));
            }
        }
        for (&bv, &cid) in cell_borrows {
            val_vec_root.insert(bv, VecRootKey::Cell(cid));
        }
        for &b in &doms.rpo {
            for &param in &func.blocks[b.0 as usize].params {
                if b.0 != 0 && is_global_vec_ty(table, func.values[param.0 as usize].ty) {
                    val_vec_root.insert(param, VecRootKey::Value(param));
                }
            }
            for op in &func.blocks[b.0 as usize].ops {
                let Some(res) = op.result else { continue };
                match &op.kind {
                    OpKind::Copy(s) | OpKind::Move(s) => {
                        if let Some(&rk) = val_vec_root.get(s) {
                            val_vec_root.insert(res, rk);
                        }
                    }
                    OpKind::CellLoad(c) if is_global_vec_ty(table, func.cells[c.0 as usize].ty) => {
                        val_vec_root.insert(res, VecRootKey::Cell(*c));
                    }
                    OpKind::Call { .. }
                        if is_global_vec_ty(table, func.values[res.0 as usize].ty) =>
                    {
                        val_vec_root.insert(res, VecRootKey::Value(res));
                    }
                    OpKind::BorrowView { root, path, .. } => match (root, path.as_slice()) {
                        (Root::Value(v), [Proj::Deref] | []) => {
                            if let Some(&rk) = val_vec_root.get(v) {
                                val_vec_root.insert(res, rk);
                            }
                        }
                        (
                            Root::Value(v),
                            [Proj::Deref, Proj::Field { field, ty }] | [Proj::Field { field, ty }],
                        ) if is_global_vec_ty(table, *ty) => {
                            if let Some(pi) = func.params.iter().position(|&p| p == *v) {
                                val_vec_root.insert(res, VecRootKey::ParamField(pi, *field));
                            }
                        }
                        (Root::Cell(c), [Proj::Field { field, ty }])
                            if is_global_vec_ty(table, *ty) =>
                        {
                            val_vec_root.insert(res, VecRootKey::CellField(*c, *field));
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        val_vec_root
    }

    /// Collects every `VecRootKey` whose length may be mutated anywhere in `func`.
    fn collect_mutated_vec_roots(
        func: &Func,
        table: &TypeTable,
        cell_len_mutated: &BTreeSet<CellId>,
        val_vec_root: &BTreeMap<ValueId, VecRootKey>,
    ) -> BTreeSet<VecRootKey> {
        let mut root_len_mutated: BTreeSet<VecRootKey> = cell_len_mutated
            .iter()
            .copied()
            .map(VecRootKey::Cell)
            .collect();
        for block in &func.blocks {
            for op in &block.ops {
                match &op.kind {
                    OpKind::Call { semantic, args, .. } => {
                        if !is_vec_len_preserving_call(*semantic) {
                            for &a in args {
                                if let Some(&rk) = val_vec_root.get(&a) {
                                    if !is_immut_ref_ty(table, func.values[a.0 as usize].ty) {
                                        root_len_mutated.insert(rk);
                                    }
                                }
                            }
                            // Any opaque call taking `&mut Holder` could mutate `Holder`'s vec fields.
                            if !matches!(semantic, CallSemantic::Standard(_)) {
                                for &a in args {
                                    let aty = func.values[a.0 as usize].ty;
                                    if matches!(
                                        table.get(table.peeled(aty)).map(|d| &d.kind),
                                        Some(TypeKind::Reference { mutable: true, .. })
                                    ) {
                                        if let Some(pi) = func.params.iter().position(|&p| p == a) {
                                            for rk in val_vec_root.values() {
                                                if matches!(rk, VecRootKey::ParamField(p, _) if *p == pi)
                                                {
                                                    root_len_mutated.insert(*rk);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    OpKind::SetField { base, .. } => {
                        if let Some(&rk) = val_vec_root.get(base) {
                            root_len_mutated.insert(rk);
                        }
                    }
                    OpKind::StoreThrough { ptr, .. } => {
                        if let Some(&rk) = val_vec_root.get(ptr) {
                            root_len_mutated.insert(rk);
                        }
                    }
                    OpKind::RawView { root, .. } => match root {
                        Root::Cell(c) => {
                            root_len_mutated.insert(VecRootKey::Cell(*c));
                        }
                        Root::Value(v) => {
                            if let Some(&rk) = val_vec_root.get(v) {
                                root_len_mutated.insert(rk);
                            }
                        }
                    },
                    _ => {}
                }
            }
        }
        root_len_mutated
    }

    fn dyn_len(&self, v: ValueId, cv: ValueId) -> Option<ValueId> {
        self.vec_dyn_len
            .get(&v)
            .or_else(|| self.vec_dyn_len.get(&cv))
            .copied()
    }

    fn propagate_val(&mut self, from: ValueId, canon_from: ValueId, to: ValueId) {
        if let Some(dl) = self.dyn_len(from, canon_from) {
            self.vec_dyn_len.insert(to, dl);
        }
        if let Some(&cl) = self
            .vec_const_len
            .get(&from)
            .or_else(|| self.vec_const_len.get(&canon_from))
        {
            self.vec_const_len.insert(to, cl);
        }
        if let Some(&sl) = self
            .vec_one_shl_len
            .get(&from)
            .or_else(|| self.vec_one_shl_len.get(&canon_from))
        {
            self.vec_one_shl_len.insert(to, sl);
        }
    }

    fn store_to_cell(&mut self, cell: CellId, v: ValueId, cv: ValueId) {
        if !self.cell_len_mutated.contains(&cell) {
            if let Some(dl) = self.dyn_len(v, cv) {
                self.cell_dyn_len.insert(cell, dl);
            }
            if let Some(&cl) = self
                .vec_const_len
                .get(&v)
                .or_else(|| self.vec_const_len.get(&cv))
            {
                self.cell_const_len.insert(cell, cl);
            }
            if let Some(&sl) = self
                .vec_one_shl_len
                .get(&v)
                .or_else(|| self.vec_one_shl_len.get(&cv))
            {
                self.cell_one_shl_len.insert(cell, sl);
            }
        } else {
            self.block_root_len.remove(&VecRootKey::Cell(cell));
        }
    }

    fn load_from_cell(&mut self, cell: CellId, to: ValueId) {
        if let Some(&dl) = self.cell_dyn_len.get(&cell) {
            self.vec_dyn_len.insert(to, dl);
        }
        if let Some(&cl) = self.cell_const_len.get(&cell) {
            self.vec_const_len.insert(to, cl);
        }
        if let Some(&sl) = self.cell_one_shl_len.get(&cell) {
            self.vec_one_shl_len.insert(to, sl);
        }
    }

    fn resolve_vec_len_val(
        &mut self,
        canon: &[ValueId],
        vec_val: ValueId,
        result: ValueId,
    ) -> Option<ValueId> {
        let cv = canon[vec_val.0 as usize];
        if let Some(dl) = self.dyn_len(vec_val, cv) {
            return Some(canon[dl.0 as usize]);
        }
        if let Some(&cl) = self
            .vec_const_len
            .get(&vec_val)
            .or_else(|| self.vec_const_len.get(&cv))
        {
            self.vec_const_len.insert(result, cl);
        }
        if let Some(&sl) = self
            .vec_one_shl_len
            .get(&vec_val)
            .or_else(|| self.vec_one_shl_len.get(&cv))
        {
            self.vec_one_shl_len.insert(result, sl);
        }
        if let Some(&rk) = self
            .val_vec_root
            .get(&vec_val)
            .or_else(|| self.val_vec_root.get(&cv))
        {
            return self.unify_root_len(canon, rk, result);
        }
        None
    }

    fn unify_root_len(
        &mut self,
        canon: &[ValueId],
        rk: VecRootKey,
        result: ValueId,
    ) -> Option<ValueId> {
        if !self.root_len_mutated.contains(&rk) {
            if let VecRootKey::Cell(c) = rk {
                if let Some(&cl) = self.cell_const_len.get(&c) {
                    self.vec_const_len.insert(result, cl);
                }
                if let Some(&sl) = self.cell_one_shl_len.get(&c) {
                    self.vec_one_shl_len.insert(result, sl);
                }
                if let Some(&dl) = self.cell_dyn_len.get(&c) {
                    return Some(canon[dl.0 as usize]);
                }
            }
            let entry = *self.unmutated_root_len.entry(rk).or_insert(result);
            return Some(canon[entry.0 as usize]);
        }
        let entry = *self.block_root_len.entry(rk).or_insert(result);
        Some(canon[entry.0 as usize])
    }

    fn apply_return_vec_len(
        &mut self,
        analysis: &Analysis,
        func: &Func,
        result: ValueId,
        args: &[ValueId],
        summary: &ReturnVecLen,
    ) {
        match summary {
            ReturnVecLen::Const(c) => {
                self.vec_const_len.insert(result, *c);
            }
            ReturnVecLen::Param(k) => {
                if let Some(&ak) = args.get(*k) {
                    self.vec_dyn_len.insert(result, analysis.canon(ak));
                }
            }
            ReturnVecLen::ParamSliceLen(k) => {
                if let Some(&ak) = args.get(*k) {
                    if let Some(&dl) = self.vec_dyn_len.get(&ak) {
                        self.vec_dyn_len.insert(result, dl);
                    }
                }
            }
            ReturnVecLen::OneShlParam { shift_param, bits } => {
                if let Some(&ak) = args.get(*shift_param) {
                    self.vec_one_shl_len
                        .insert(result, (analysis.canon(ak), *bits));
                }
            }
            ReturnVecLen::CondBoolParam {
                cond_param,
                when_true,
                when_false,
            } => {
                if let Some(&cv) = args.get(*cond_param) {
                    match analysis.def_kind(func, analysis.canon(cv)) {
                        Some(OpKind::Const(ConstTree::Scalar { bits: 1, .. })) => {
                            self.apply_return_vec_len(analysis, func, result, args, when_true);
                        }
                        Some(OpKind::Const(ConstTree::Scalar { bits: 0, .. })) => {
                            self.apply_return_vec_len(analysis, func, result, args, when_false);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

impl Analysis {
    /// Value numbering: congruent pure ops share one class representative, so equality of terms
    /// is equality of representatives. Assigned in reverse postorder, which is a dominance
    /// order, so an op's operands are numbered before it.
    pub(super) fn number_values(&mut self, ctx: RangeInputs<'_>) {
        let mut incoming: Vec<Vec<Vec<ValueId>>> = vec![Vec::new(); ctx.func.blocks.len()];
        for block in &ctx.func.blocks {
            for (target, args) in block.term.edges() {
                incoming[target.0 as usize].push(args.to_vec());
            }
        }

        let mut tracker = VecLenTracker::scan_vec_roots(&self.doms, ctx);
        self.number_rpo(ctx, &incoming, &mut tracker);
        self.unify_lockstep_slice_params(ctx.func, ctx.table, &incoming, &mut tracker);
        self.vec_const_len = tracker.vec_const_len;
        self.vec_one_shl_len = tracker.vec_one_shl_len;
    }

    fn number_rpo(
        &mut self,
        ctx: RangeInputs<'_>,
        incoming: &[Vec<Vec<ValueId>>],
        tracker: &mut VecLenTracker,
    ) {
        let func = ctx.func;
        let table = ctx.table;
        let interns = ctx.interns;
        let mut classes: BTreeMap<GvnKey, ValueId> = BTreeMap::new();
        // Precompute `defs` as `RBlockId` pairs for `find_counted_push_loop_len`.
        let rdefs: Vec<Option<(RBlockId, usize)>> = self
            .defs
            .iter()
            .map(|d| d.map(|(b, i)| (RBlockId(b), i)))
            .collect();

        for &block in &self.doms.rpo.clone() {
            tracker.block_root_len.clear();
            // A block parameter whose every incoming edge carries the same numbered value is
            // that value for analysis purposes. DCE may deliberately keep the parameter named
            // to prevent expression duplication in the printer; range proofs still need to see
            // through it (notably a `SliceView` handed to a bounds-check block).
            if block.0 != 0 {
                for (position, param) in func.blocks[block.0 as usize].params.iter().enumerate() {
                    let mut representative: Option<ValueId> = None;
                    let mut equal = true;
                    for args in &incoming[block.0 as usize] {
                        let Some(arg) = args.get(position).copied() else {
                            equal = false;
                            break;
                        };
                        if arg == *param {
                            continue;
                        }
                        let arg = self.canon(arg);
                        match representative {
                            None => representative = Some(arg),
                            Some(known) if known == arg => {}
                            Some(_) => {
                                equal = false;
                                break;
                            }
                        }
                    }
                    if equal {
                        if let Some(representative) = representative {
                            self.canon[param.0 as usize] = representative;
                        }
                    }
                }
            }

            for op in &func.blocks[block.0 as usize].ops {
                // Track `CellStore` exact lengths for unmutated local `Vec` cells, and invalidate
                // `block_root_len` on any length-mutating op!
                match &op.kind {
                    OpKind::CellStore { cell, v } => {
                        let cv = self.canon(*v);
                        tracker.store_to_cell(*cell, *v, cv);
                    }
                    OpKind::Call { semantic, .. } if !is_vec_len_preserving_call(*semantic) => {
                        tracker.block_root_len.clear();
                    }
                    OpKind::SetField { .. } | OpKind::StoreThrough { .. }
                        if matches!(
                            &op.kind,
                            OpKind::SetField { base, .. }
                                if is_global_vec_ty(table, func.values[base.0 as usize].ty)
                        ) || matches!(
                            &op.kind,
                            OpKind::StoreThrough { ptr, .. }
                                if table
                                    .pointee(func.values[ptr.0 as usize].ty)
                                    .is_some_and(|pt| is_global_vec_ty(table, pt))
                        ) =>
                    {
                        tracker.block_root_len.clear();
                    }
                    _ => {}
                }

                let Some(result) = op.result else {
                    continue;
                };
                let ty = func.values[result.0 as usize].ty;
                let key = match &op.kind {
                    OpKind::Copy(v) | OpKind::Move(v) => {
                        let cv = self.canon(*v);
                        self.canon[result.0 as usize] = cv;
                        tracker.propagate_val(*v, cv, result);
                        continue;
                    }
                    // Same-width unsigned `IntToInt` casts (e.g. `u32 as usize` / `usize as u32`
                    // on 32-bit targets) are value-preserving identities!
                    OpKind::Cast {
                        kind: CastKind::IntToInt,
                        v,
                        from,
                        to,
                    } if matches!(
                        (table.int_info(*from), table.int_info(*to)),
                        (Some((false, fb)), Some((false, tb))) if fb == tb
                    ) =>
                    {
                        self.canon[result.0 as usize] = self.canon(*v);
                        continue;
                    }
                    // A whole-slice reborrow (`&*s`) is the slice it reborrows; a borrow of an
                    // unmutated SSA `Vec` value (`&v` / `&mut v`) propagates its exact length!
                    OpKind::BorrowView {
                        root: Root::Value(v),
                        path,
                        ..
                    } => {
                        let cv = self.canon(*v);
                        if matches!(path.as_slice(), [Proj::Deref]) && is_slice_pointer(table, ty) {
                            self.canon[result.0 as usize] = cv;
                            tracker.propagate_val(*v, cv, result);
                            continue;
                        }
                        if matches!(path.as_slice(), [] | [Proj::Deref])
                            && tracker
                                .val_vec_root
                                .get(v)
                                .is_some_and(|rk| !tracker.root_len_mutated.contains(rk))
                        {
                            tracker.propagate_val(*v, cv, result);
                        }
                        continue;
                    }
                    OpKind::CellLoad(c) if !tracker.cell_len_mutated.contains(c) => {
                        tracker.load_from_cell(*c, result);
                        continue;
                    }
                    OpKind::Field { base, field, .. }
                        if interns.is_some_and(|i| {
                            let bty = func.values[base.0 as usize].ty;
                            is_direct_global_vec_ty(table, bty)
                                && is_vec_len_field(table, i, bty, *field)
                        }) =>
                    {
                        if let Some(len_v) = tracker.resolve_vec_len_val(&self.canon, *base, result)
                        {
                            self.canon[result.0 as usize] = len_v;
                        }
                        continue;
                    }
                    OpKind::Deref(ptr) => {
                        // Deref of `&v.len` on a Vec!
                        if let Some(i) = interns {
                            if let Some(OpKind::BorrowView { root, path, .. }) =
                                self.def_kind(func, self.canon(*ptr))
                            {
                                let vec_rk = match (root, path.as_slice()) {
                                    (Root::Cell(c), [Proj::Field { field, .. }])
                                        if is_direct_global_vec_ty(
                                            table,
                                            func.cells[c.0 as usize].ty,
                                        ) && is_vec_len_field(
                                            table,
                                            i,
                                            func.cells[c.0 as usize].ty,
                                            *field,
                                        ) =>
                                    {
                                        Some(VecRootKey::Cell(*c))
                                    }
                                    (Root::Value(v), [Proj::Deref, Proj::Field { field, .. }])
                                        if table
                                            .pointee(func.values[v.0 as usize].ty)
                                            .is_some_and(|pt| {
                                                is_direct_global_vec_ty(table, pt)
                                                    && is_vec_len_field(table, i, pt, *field)
                                            }) =>
                                    {
                                        tracker.val_vec_root.get(v).copied()
                                    }
                                    _ => None,
                                };
                                if let Some(rk) = vec_rk {
                                    if let Some(len_v) =
                                        tracker.unify_root_len(&self.canon, rk, result)
                                    {
                                        self.canon[result.0 as usize] = len_v;
                                        continue;
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    OpKind::Call { .. } => {
                        self.number_vec_call(ctx, tracker, result, &op.kind);
                        continue;
                    }
                    OpKind::Unary {
                        op: UnaryOp::PtrMetadata,
                        v,
                        ty: operand_ty,
                    } => {
                        let Some(base) = self.number_ptr_metadata(ctx, tracker, &rdefs, result, *v)
                        else {
                            continue;
                        };
                        GvnKey::Unary(UnaryOp::PtrMetadata as u8, base, *operand_ty)
                    }
                    OpKind::Const(ConstTree::Scalar { bits, bytes }) => {
                        GvnKey::Scalar(*bits, *bytes, ty)
                    }
                    OpKind::Binary {
                        op: binary,
                        l,
                        r,
                        ty: operand_ty,
                    } if *binary != BinaryOp::Offset => {
                        GvnKey::Binary(*binary as u8, self.canon(*l), self.canon(*r), *operand_ty)
                    }
                    OpKind::Unary {
                        op: unary,
                        v,
                        ty: operand_ty,
                    } => GvnKey::Unary(*unary as u8, self.canon(*v), *operand_ty),
                    OpKind::Cast {
                        kind:
                            kind @ (CastKind::IntToInt
                            | CastKind::FloatToFloat
                            | CastKind::IntToFloat
                            | CastKind::Subtype),
                        v,
                        from,
                        to,
                    } => {
                        let tag = match kind {
                            CastKind::IntToInt => 0,
                            CastKind::FloatToFloat => 1,
                            CastKind::IntToFloat => 2,
                            CastKind::Subtype => 3,
                            _ => unreachable!(),
                        };
                        GvnKey::Cast(tag, self.canon(*v), *from, *to)
                    }
                    _ => continue,
                };
                let entry = classes.entry(key).or_insert(result);
                self.canon[result.0 as usize] = *entry;
            }
        }
    }

    fn number_vec_call(
        &mut self,
        ctx: RangeInputs<'_>,
        tracker: &mut VecLenTracker,
        result: ValueId,
        kind: &OpKind,
    ) {
        let OpKind::Call {
            callee,
            semantic,
            args,
            ..
        } = kind
        else {
            return;
        };
        let func = ctx.func;
        let table = ctx.table;
        let interns = ctx.interns;
        match semantic {
            CallSemantic::Standard(StandardOperation::NativeVecFromElemZeroed)
                if !args.is_empty() =>
            {
                tracker.vec_dyn_len.insert(result, self.canon(args[0]));
            }
            CallSemantic::Standard(StandardOperation::VecFromElem) if args.len() >= 2 => {
                tracker.vec_dyn_len.insert(result, self.canon(args[1]));
            }
            CallSemantic::Standard(
                StandardOperation::VecNew
                | StandardOperation::NativeVecNew
                | StandardOperation::VecWithCapacity
                | StandardOperation::NativeVecWithCapacity,
            ) => {
                tracker.vec_const_len.insert(result, 0);
            }
            CallSemantic::Standard(StandardOperation::NativeVecFromArray) if !args.is_empty() => {
                let aty = table.peeled(func.values[args[0].0 as usize].ty);
                if let Some(TypeKind::Array { length, .. }) = table.get(aty).map(|d| &d.kind) {
                    tracker.vec_const_len.insert(result, u128::from(*length));
                }
            }
            CallSemantic::Standard(StandardOperation::VecClone) if !args.is_empty() => {
                let a0 = args[0];
                if let Some(&dl) = tracker.vec_dyn_len.get(&a0) {
                    tracker.vec_dyn_len.insert(result, dl);
                } else if let Some(&VecRootKey::Cell(c)) = tracker.val_vec_root.get(&a0) {
                    if !tracker.cell_len_mutated.contains(&c) {
                        if let Some(&dl) = tracker.cell_dyn_len.get(&c) {
                            tracker.vec_dyn_len.insert(result, dl);
                        }
                        if let Some(&cl) = tracker.cell_const_len.get(&c) {
                            tracker.vec_const_len.insert(result, cl);
                        }
                    } else if let Some(&rl) = tracker.block_root_len.get(&VecRootKey::Cell(c)) {
                        tracker.vec_dyn_len.insert(result, rl);
                    }
                }
            }
            CallSemantic::Standard(StandardOperation::SliceToVec) if !args.is_empty() => {
                if let Ok(sl) = self.peel_slice_source(func, table, interns, args[0], None) {
                    tracker.vec_dyn_len.insert(result, sl);
                }
            }
            CallSemantic::Standard(StandardOperation::VecLen) if !args.is_empty() => {
                if let Some(len_v) = tracker.resolve_vec_len_val(&self.canon, args[0], result) {
                    self.canon[result.0 as usize] = len_v;
                }
            }
            CallSemantic::Standard(
                StandardOperation::VecAsSlice
                | StandardOperation::NativeVecAsSlice
                | StandardOperation::VecAsMutSlice
                | StandardOperation::NativeVecAsMutSlice,
            ) if !args.is_empty() => {
                let a0 = args[0];
                let ca0 = self.canon(a0);
                tracker.propagate_val(a0, ca0, result);
                if let Some(&rk) = tracker.val_vec_root.get(&a0) {
                    tracker.val_vec_root.insert(result, rk);
                    if let VecRootKey::Cell(c) = rk {
                        if !tracker.cell_len_mutated.contains(&c) {
                            tracker.load_from_cell(c, result);
                        }
                    }
                    // Snapshot the slice's immutable fat-pointer length from the same-block (no
                    // intervening resize) `Vec.len` read if present!
                    if let std::collections::btree_map::Entry::Vacant(e) =
                        tracker.vec_dyn_len.entry(result)
                    {
                        if let Some(&bl) = tracker.block_root_len.get(&rk) {
                            e.insert(self.canon(bl));
                        } else if !tracker.root_len_mutated.contains(&rk) {
                            if let Some(&ul) = tracker.unmutated_root_len.get(&rk) {
                                e.insert(self.canon(ul));
                            }
                        }
                    }
                }
            }
            CallSemantic::Ordinary => {
                if let Callee::Direct { item, .. } = callee {
                    if let Some(ret_len) =
                        ctx.arena_facts.and_then(|af| af.return_vec_len.get(item))
                    {
                        tracker.apply_return_vec_len(self, func, result, args, ret_len);
                    }
                }
            }
            _ => {}
        }
    }

    /// Numbers a `PtrMetadata` op. Returns `None` when resolved directly to an existing canonical
    /// length value, or `Some(base)` when the caller should value-number `PtrMetadata(base)`.
    fn number_ptr_metadata(
        &mut self,
        ctx: RangeInputs<'_>,
        tracker: &mut VecLenTracker,
        rdefs: &[Option<(RBlockId, usize)>],
        result: ValueId,
        v: ValueId,
    ) -> Option<ValueId> {
        let meta_bits = self.widths[result.0 as usize];
        let cv = self.canon(v);
        if let Some(dl) = tracker.dyn_len(v, cv) {
            if meta_bits.is_none_or(|b| self.widths[dl.0 as usize] == Some(b)) {
                self.canon[result.0 as usize] = self.canon(dl);
                return None;
            }
        }
        if let Some(&cl) = tracker
            .vec_const_len
            .get(&v)
            .or_else(|| tracker.vec_const_len.get(&cv))
        {
            tracker.vec_const_len.insert(result, cl);
        }
        if let Some(&sl) = tracker
            .vec_one_shl_len
            .get(&v)
            .or_else(|| tracker.vec_one_shl_len.get(&cv))
        {
            tracker.vec_one_shl_len.insert(result, sl);
        }
        // Check counted `0..N` unconditional push loop length on a Vec cell!
        if let Some(&rk) = tracker
            .val_vec_root
            .get(&v)
            .or_else(|| tracker.val_vec_root.get(&cv))
        {
            if let VecRootKey::Cell(c) = rk {
                if tracker.cell_len_mutated.contains(&c) {
                    if let Some(loop_n) = find_counted_push_loop_len(
                        ctx.func,
                        ctx.table,
                        &self.doms,
                        rdefs,
                        &self.canon,
                        c,
                        &tracker.cell_borrows,
                    ) {
                        let ln = self.canon(loop_n);
                        if meta_bits.is_none_or(|b| self.widths[ln.0 as usize] == Some(b)) {
                            self.canon[result.0 as usize] = ln;
                            return None;
                        }
                    }
                }
            }
            if let Some(len_v) = tracker.unify_root_len(&self.canon, rk, result) {
                if meta_bits.is_none_or(|b| self.widths[len_v.0 as usize] == Some(b)) {
                    self.canon[result.0 as usize] = len_v;
                    return None;
                }
            }
        }
        // Peel `SliceView`, sysroot `slice_from_parts` (`RawParts`), sysroot `byte_slice_ref`,
        // whole-slice reborrows, and lossless slice pointer casts so `PtrMetadata` recovers the
        // exact underlying length or root.
        let mut base = match self.peel_slice_source(ctx.func, ctx.table, ctx.interns, v, meta_bits)
        {
            Ok(len) => {
                self.canon[result.0 as usize] = len;
                return None;
            }
            Err(root) => root,
        };
        if let Some(&eq_base) = tracker.param_eq_slice_len.get(&base) {
            base = self.canon(eq_base);
        }
        Some(base)
    }

    /// Lockstep `(len_param, slice_param)` block-parameter unification (e.g. `fflate::inflt`
    /// carrying `(buf_len, out_sl)` across loop iterations where every incoming edge either
    /// preserves the pair unchanged or assigns `buf_len = buf.len(); out_sl = &mut buf;`):
    /// if every incoming edge to block `b` passes a slice whose known length equals the paired
    /// integer argument (or passes `(p_len, p_slice)` itself), then `PtrMetadata(p_slice)` unifies
    /// with `canon(p_len)`.
    fn unify_lockstep_slice_params(
        &mut self,
        func: &Func,
        table: &TypeTable,
        incoming: &[Vec<Vec<ValueId>>],
        tracker: &mut VecLenTracker,
    ) {
        let mut changed_lockstep = true;
        for _ in 0..3 {
            if !changed_lockstep {
                break;
            }
            changed_lockstep = false;
            for &block in &self.doms.rpo {
                if block.0 == 0 {
                    continue;
                }
                let params = &func.blocks[block.0 as usize].params;
                let edges = &incoming[block.0 as usize];
                if params.len() < 2 || edges.is_empty() {
                    continue;
                }
                for (sj, &p_slice) in params.iter().enumerate() {
                    if !is_slice_pointer(table, func.values[p_slice.0 as usize].ty)
                        || tracker.vec_dyn_len.contains_key(&p_slice)
                    {
                        continue;
                    }
                    let ps_c = self.canon(p_slice);
                    for (li, &p_len) in params.iter().enumerate() {
                        if li == sj || self.widths[p_len.0 as usize].is_none() {
                            continue;
                        }
                        let pl_c = self.canon(p_len);
                        let mut any_seed = false;
                        let mut all_ok = true;
                        for args in edges {
                            let (Some(&s_arg), Some(&l_arg)) = (args.get(sj), args.get(li)) else {
                                all_ok = false;
                                break;
                            };
                            let sa_c = self.canon(s_arg);
                            let la_c = self.canon(l_arg);
                            if sa_c == ps_c && la_c == pl_c {
                                continue;
                            }
                            let known_s_len = tracker.dyn_len(s_arg, sa_c).map(|v| self.canon(v));
                            if known_s_len == Some(la_c) {
                                any_seed = true;
                            } else {
                                all_ok = false;
                                break;
                            }
                        }
                        if all_ok && any_seed {
                            tracker.vec_dyn_len.insert(p_slice, pl_c);
                            tracker.vec_dyn_len.insert(ps_c, pl_c);
                            changed_lockstep = true;
                            break;
                        }
                    }
                }
            }
            if changed_lockstep {
                // Propagate newly discovered lockstep slice lengths to copies, reborrows, and
                // `PtrMetadata` ops in RPO.
                for &block in &self.doms.rpo.clone() {
                    for op in &func.blocks[block.0 as usize].ops {
                        let Some(res) = op.result else { continue };
                        match &op.kind {
                            OpKind::Copy(v) | OpKind::Move(v) => {
                                let cv = self.canon(*v);
                                if let Some(dl) = tracker.dyn_len(*v, cv) {
                                    tracker.vec_dyn_len.insert(res, dl);
                                }
                            }
                            OpKind::BorrowView {
                                root: Root::Value(v),
                                path,
                                ..
                            } if matches!(path.as_slice(), [Proj::Deref])
                                && is_slice_pointer(table, func.values[res.0 as usize].ty) =>
                            {
                                let cv = self.canon(*v);
                                if let Some(dl) = tracker.dyn_len(*v, cv) {
                                    tracker.vec_dyn_len.insert(res, dl);
                                }
                            }
                            OpKind::Unary {
                                op: UnaryOp::PtrMetadata,
                                v,
                                ..
                            } => {
                                let cv = self.canon(*v);
                                if let Some(dl) = tracker.dyn_len(*v, cv) {
                                    let meta_bits = self.widths[res.0 as usize];
                                    if meta_bits
                                        .is_none_or(|b| self.widths[dl.0 as usize] == Some(b))
                                    {
                                        self.canon[res.0 as usize] = self.canon(dl);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}
