//! Per-function arena precondition enforcement: closed-world, monotone-`Vec`, no-subslice,
//! no-overwrite, no-raw-pointer, and constructor/aggregate flow checks.

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::repr::{
    CellId, Dominators, Func, Op, OpKind, Proj, RBlockId, RTerm, Root, ValueId,
};
use rcgjs_object::jir::semantic::{
    CallSemantic, Callee, CastKind, ConstTree, DefinitionId, DropGlue, ItemId, StandardOperation,
};
use rcgjs_object::jir::types::{TypeId, TypeKind, TypeTable};

use crate::analysis::heap::{is_direct_global_vec_ty, is_vec_len_field};
use crate::lower::js::value;

use super::fixpoint::ElemCtorSummary;
use super::{
    adt_def_of, func_mentions_def, slice_elem_def, ty_contains_vec_of_def, ty_reaches_def,
    vec_elem_def, vec_ref_elem_def,
};

/// Allowed `StandardOperation` calls on `Vec<Elem, Global>` for a monotone arena.
pub(super) fn is_monotone_arena_vec_op(op: StandardOperation) -> bool {
    use StandardOperation as SO;
    matches!(
        op,
        SO::VecNew
            | SO::NativeVecNew
            | SO::VecWithCapacity
            | SO::NativeVecWithCapacity
            | SO::VecPush
            | SO::NativeVecPush
            | SO::VecPushMut
            | SO::NativeVecPushMut
            | SO::VecLen
            | SO::VecIsEmpty
            | SO::VecCapacity
            | SO::NativeVecCapacity
            | SO::VecReserve
            | SO::VecReserveExact
            | SO::NativeVecReserve
            | SO::NativeVecReserveExact
            | SO::NativeVecTryReserve
            | SO::NativeVecTryReserveExact
            | SO::VecShrinkToFit
            | SO::NativeVecShrinkTo
            | SO::NativeVecTryShrinkTo
            | SO::VecAsSlice
            | SO::NativeVecAsSlice
            | SO::VecAsMutSlice
            | SO::NativeVecAsMutSlice
            | SO::VecIndex
            | SO::VecIndexMut
            | SO::VecIter
            | SO::VecIterMut
            | SO::VecDrop
    )
}

/// Allowed `StandardOperation` calls on an inner `Vec<Idx, Global>` field (like `Node.children`).
pub(super) fn is_allowed_inner_idx_vec_op(op: StandardOperation) -> bool {
    use StandardOperation as SO;
    matches!(
        op,
        SO::VecNew
            | SO::NativeVecNew
            | SO::VecWithCapacity
            | SO::NativeVecWithCapacity
            | SO::VecPush
            | SO::NativeVecPush
            | SO::VecPushMut
            | SO::NativeVecPushMut
            | SO::VecPop
            | SO::NativeVecPop
            | SO::VecClear
            | SO::VecTruncate
            | SO::VecRemove
            | SO::VecSwapRemove
            | SO::VecSwap
            | SO::VecLen
            | SO::VecIsEmpty
            | SO::VecCapacity
            | SO::NativeVecCapacity
            | SO::VecReserve
            | SO::VecReserveExact
            | SO::NativeVecReserve
            | SO::NativeVecReserveExact
            | SO::NativeVecTryReserve
            | SO::NativeVecTryReserveExact
            | SO::VecShrinkToFit
            | SO::NativeVecShrinkTo
            | SO::NativeVecTryShrinkTo
            | SO::VecAsSlice
            | SO::NativeVecAsSlice
            | SO::VecAsMutSlice
            | SO::NativeVecAsMutSlice
            | SO::VecIndex
            | SO::VecIndexMut
            | SO::VecIter
            | SO::VecIterMut
            | SO::VecDrop
            | SO::SliceSwap
            | SO::SliceSort
            | SO::SliceSortCmp
    )
}

/// Enforces all whole-program monotone-arena preconditions over a single function.
pub(super) struct PreconditionChecker<'a> {
    func: &'a Func,
    types: &'a TypeTable,
    interns: &'a Interns,
    by_item_id: &'a BTreeMap<ItemId, usize>,
    ctor_summaries: &'a BTreeMap<ItemId, ElemCtorSummary>,
    active: Vec<DefinitionId>,
    is_ctor: bool,
    in_loop: BTreeSet<RBlockId>,
    vec_cell_stores: BTreeMap<(DefinitionId, CellId), usize>,
    disqualified: &'a mut BTreeSet<DefinitionId>,
}

impl<'a> PreconditionChecker<'a> {
    pub(super) fn check_func(
        func: &'a Func,
        interns: &'a Interns,
        by_item_id: &'a BTreeMap<ItemId, usize>,
        ctor_summaries: &'a BTreeMap<ItemId, ElemCtorSummary>,
        candidate_defs: &[DefinitionId],
        disqualified: &'a mut BTreeSet<DefinitionId>,
    ) {
        let types = &interns.types;
        let active: Vec<DefinitionId> = candidate_defs
            .iter()
            .copied()
            .filter(|&def| !disqualified.contains(&def) && func_mentions_def(func, types, def))
            .collect();
        if active.is_empty() {
            return;
        }
        let doms = Dominators::of(func);
        let in_loop = blocks_in_loops(func, &doms);
        let is_ctor = ctor_summaries.contains_key(&func.item);
        let mut checker = Self {
            func,
            types,
            interns,
            by_item_id,
            ctor_summaries,
            active,
            is_ctor,
            in_loop,
            vec_cell_stores: BTreeMap::new(),
            disqualified,
        };
        checker.run();
    }

    fn run(&mut self) {
        self.check_fixed_arrays();
        for block in &self.func.blocks {
            for op in &block.ops {
                self.check_op(block.id, op);
            }
        }
    }

    /// Disqualifies every active candidate whose `DefinitionId` is transitively reachable from `ty`.
    fn disqualify_reaching(&mut self, ty: TypeId) {
        for &def in &self.active {
            if !self.disqualified.contains(&def) && ty_reaches_def(self.types, ty, def) {
                self.disqualified.insert(def);
            }
        }
    }

    /// Disqualifies every active candidate reachable from any value in `values` or `result`.
    fn disqualify_reaching_values(&mut self, values: &[ValueId], result: Option<ValueId>) {
        for &v in values {
            self.disqualify_reaching(self.func.values[v.0 as usize].ty);
        }
        if let Some(r) = result {
            self.disqualify_reaching(self.func.values[r.0 as usize].ty);
        }
    }

    /// Rejects `[Elem; N]` arrays anywhere in values/cells (a fixed array coerced to `&[Elem]`
    /// would not be backed by a monotone `Vec<Elem>`).
    fn check_fixed_arrays(&mut self) {
        for &def in &self.active {
            let has_fixed_arr = self
                .func
                .values
                .iter()
                .any(|v| ty_has_array_of_def(self.types, v.ty, def))
                || self
                    .func
                    .cells
                    .iter()
                    .any(|c| ty_has_array_of_def(self.types, c.ty, def));
            if has_fixed_arr {
                self.disqualified.insert(def);
            }
        }
    }

    fn check_op(&mut self, block_id: RBlockId, op: &Op) {
        match &op.kind {
            OpKind::Call {
                callee,
                args,
                semantic,
                ..
            } => self.check_call(op, callee, args, semantic),
            OpKind::Host { args, .. } => {
                self.disqualify_reaching_values(args, op.result);
            }
            OpKind::Intrinsic {
                name,
                types: ity,
                args,
                ..
            } => {
                let benign_meta = matches!(
                    self.interns.str(*name),
                    Some("size_of" | "min_align_of" | "align_of" | "needs_drop")
                );
                if !benign_meta {
                    self.disqualify_reaching_values(args, op.result);
                    for &t in ity {
                        self.disqualify_reaching(t);
                    }
                }
            }
            OpKind::Cast { kind, from, to, .. } => {
                let allowed_cast = matches!(kind, CastKind::Subtype | CastKind::PointerCoercion)
                    && !value::is_raw(self.types, *from)
                    && !value::is_raw(self.types, *to)
                    && !matches!(
                        value::pointee_of(self.types, *to)
                            .and_then(|pt| self.types.get(self.types.peeled(pt)))
                            .map(|d| &d.kind),
                        Some(TypeKind::Dynamic { .. })
                    );
                if !allowed_cast {
                    self.disqualify_reaching(*from);
                    self.disqualify_reaching(*to);
                }
            }
            OpKind::RawView { root, path, .. } => {
                // Reject ANY raw pointer into `Elem`, `[Elem]`, or `Vec<Elem>`.
                if let Some(r) = op.result {
                    self.disqualify_reaching(self.func.values[r.0 as usize].ty);
                }
                for &def in &self.active {
                    if !self.disqualified.contains(&def)
                        && root_or_path_mentions_elem_or_vec(self.func, self.types, root, path, def)
                    {
                        self.disqualified.insert(def);
                    }
                }
            }
            OpKind::SliceView { .. } => {
                // Arbitrary `SliceView` on `[Elem]` could subslice or fabricate a shorter
                // slice length -> disqualify!
                if let Some(res) = op.result {
                    if let Some(def) =
                        slice_elem_def(self.types, self.func.values[res.0 as usize].ty)
                    {
                        if self.active.contains(&def) {
                            self.disqualified.insert(def);
                        }
                    }
                }
            }
            OpKind::BorrowView { root, path, .. } => {
                // Check for subslicing `[Elem]` (`Proj::Subslice`) or mutable borrow of
                // `Vec<Elem>` internals (`.len` / `.buf`).
                for &def in &self.active {
                    if !self.disqualified.contains(&def)
                        && borrow_subslice_or_vec_internal(
                            self.func,
                            self.types,
                            self.interns,
                            root,
                            path,
                            def,
                        )
                    {
                        self.disqualified.insert(def);
                    }
                }
            }
            OpKind::SetField { base, field, .. } => {
                let base_ty = self.func.values[base.0 as usize].ty;
                for &def in &self.active {
                    if self.disqualified.contains(&def) {
                        continue;
                    }
                    // Reject mutating `Vec<Elem>` fields directly or overwriting any field
                    // containing `Vec<Elem>`.
                    if vec_elem_def(self.types, base_ty) == Some(def) {
                        self.disqualified.insert(def);
                        continue;
                    }
                    if let Some(fd) = self.types.field(base_ty, None, *field) {
                        if ty_contains_vec_of_def(self.types, fd.ty, def) {
                            self.disqualified.insert(def);
                        }
                    }
                }
            }
            OpKind::StoreThrough { ptr, .. } => {
                if let Some(pt) = value::pointee_of(self.types, self.func.values[ptr.0 as usize].ty)
                {
                    for &def in &self.active {
                        if self.disqualified.contains(&def) {
                            continue;
                        }
                        // Overwriting a `Vec<Elem>` (or struct containing `Vec<Elem>`) through
                        // a pointer could shrink/replace the arena!
                        // Also fail closed for whole-`Elem` overwrite through `&mut Elem`.
                        if ty_contains_vec_of_def(self.types, pt, def)
                            || adt_def_of(self.types, pt) == Some(def)
                        {
                            self.disqualified.insert(def);
                        }
                    }
                }
            }
            OpKind::CellStore { cell, .. } => {
                let cty = self.func.cells[cell.0 as usize].ty;
                for &def in &self.active {
                    if self.disqualified.contains(&def) {
                        continue;
                    }
                    if ty_contains_vec_of_def(self.types, cty, def) {
                        let cnt = self.vec_cell_stores.entry((def, *cell)).or_insert(0);
                        *cnt += 1;
                        if *cnt > 1 || self.in_loop.contains(&block_id) {
                            self.disqualified.insert(def);
                        }
                    }
                }
            }
            OpKind::Aggregate { ty, .. } => {
                if let Some(def) = adt_def_of(self.types, *ty) {
                    if self.active.contains(&def)
                        && !self.disqualified.contains(&def)
                        && !self.is_ctor
                    {
                        // Fresh `Elem` aggregate outside a pure constructor MUST flow only
                        // directly into `Vec<Elem>::push`!
                        if let Some(res) = op.result {
                            if !value_flows_only_to_vec_push(self.func, res) {
                                self.disqualified.insert(def);
                            }
                        }
                    }
                }
            }
            OpKind::Const(tree) => {
                if !matches!(tree, ConstTree::Unit) {
                    if let Some(res) = op.result {
                        self.disqualify_reaching(self.func.values[res.0 as usize].ty);
                    }
                }
            }
            OpKind::Repeat { .. } => {
                if let Some(res) = op.result {
                    self.disqualify_reaching(self.func.values[res.0 as usize].ty);
                }
            }
            _ => {}
        }
    }

    fn check_call(&mut self, op: &Op, callee: &Callee, args: &[ValueId], semantic: &CallSemantic) {
        let is_trusted_std = matches!(semantic, CallSemantic::Standard(_));
        let is_known_direct = matches!(
            callee,
            Callee::Direct { item, .. } if self.by_item_id.contains_key(item)
        ) || matches!(callee, Callee::Nop);
        let opaque_call =
            !is_trusted_std && (!is_known_direct || !matches!(semantic, CallSemantic::Ordinary));

        if opaque_call {
            self.disqualify_reaching_values(args, op.result);
        }

        // If it's a `StandardOperation` on `Vec<Elem>`, enforce monotonicity!
        if let CallSemantic::Standard(std_op) = semantic {
            if !is_monotone_arena_vec_op(*std_op) {
                for &def in &self.active {
                    if self.disqualified.contains(&def) {
                        continue;
                    }
                    let touches_res = op.result.is_some_and(|res| {
                        vec_elem_def(self.types, self.func.values[res.0 as usize].ty) == Some(def)
                    });
                    let touches_arg0 = args.first().is_some_and(|&a0| {
                        let a0_ty = self.func.values[a0.0 as usize].ty;
                        vec_elem_def(self.types, a0_ty) == Some(def)
                            || vec_ref_elem_def(self.types, a0_ty) == Some(def)
                            || slice_elem_def(self.types, a0_ty) == Some(def)
                    });
                    if touches_res || touches_arg0 {
                        self.disqualified.insert(def);
                    }
                }
            }
        }

        // If calling a pure constructor helper, verify its return value is ONLY used
        // by `VecPush` / `NativeVecPush` (never stored elsewhere or returned).
        if let Callee::Direct { item, .. } = callee {
            if let Some(ctor) = self.ctor_summaries.get(item) {
                if self.active.contains(&ctor.elem_def)
                    && !self.disqualified.contains(&ctor.elem_def)
                {
                    if let Some(res) = op.result {
                        if !value_flows_only_to_vec_push(self.func, res) {
                            self.disqualified.insert(ctor.elem_def);
                        }
                    }
                }
            }
        }
    }
}

fn blocks_in_loops(func: &Func, doms: &Dominators) -> BTreeSet<RBlockId> {
    let mut in_loop = BTreeSet::new();
    let mut preds: Vec<Vec<RBlockId>> = vec![Vec::new(); func.blocks.len()];
    for b in &func.blocks {
        for s in b.term.successors() {
            preds[s.0 as usize].push(b.id);
        }
    }
    for b in &func.blocks {
        for succ in b.term.successors() {
            if doms.dominates(succ, b.id) {
                // Backedge `b.id -> succ`: mark all blocks that can reach `b.id` without passing
                // through `succ`.
                let mut stack = vec![b.id];
                in_loop.insert(succ);
                in_loop.insert(b.id);
                while let Some(cur) = stack.pop() {
                    if cur == succ {
                        continue;
                    }
                    for &p in &preds[cur.0 as usize] {
                        if in_loop.insert(p) {
                            stack.push(p);
                        }
                    }
                }
            }
        }
    }
    in_loop
}

fn ty_has_array_of_def(table: &TypeTable, ty: TypeId, target: DefinitionId) -> bool {
    let mut found = false;
    table.visit_types(
        ty,
        rcgjs_object::jir::types::TypeWalk::WithAdtArgs,
        &mut |cur| {
            if found {
                return;
            }
            if let Some(TypeKind::Array { element, .. }) =
                table.get(table.peeled(cur)).map(|d| &d.kind)
            {
                if ty_reaches_def(table, *element, target) {
                    found = true;
                }
            }
        },
    );
    found
}

fn root_or_path_mentions_elem_or_vec(
    func: &Func,
    types: &TypeTable,
    root: &Root,
    path: &[Proj],
    def: DefinitionId,
) -> bool {
    let Some(mut cur) = func.root_ty(*root) else {
        return true;
    };
    let check_cur = |ty: TypeId| {
        adt_def_of(types, ty) == Some(def)
            || vec_elem_def(types, ty) == Some(def)
            || matches!(
                types.get(types.peeled(ty)).map(|d| &d.kind),
                Some(TypeKind::Slice(e)) if adt_def_of(types, *e) == Some(def)
            )
    };
    if check_cur(cur) {
        return true;
    }
    for proj in path {
        match proj {
            Proj::Deref => {
                let Some(pt) = value::pointee_of(types, cur) else {
                    return true;
                };
                cur = pt;
            }
            Proj::Field { ty, .. } => cur = *ty,
            Proj::Index(_) | Proj::ConstantIndex { .. } => {
                let Some(next) = (match types.get(types.peeled(cur)).map(|d| &d.kind) {
                    Some(TypeKind::Array { element, .. } | TypeKind::Slice(element)) => {
                        Some(*element)
                    }
                    _ => None,
                }) else {
                    return true;
                };
                cur = next;
            }
            Proj::Subslice { .. } | Proj::Downcast { .. } => {}
        }
        if check_cur(cur) {
            return true;
        }
    }
    false
}

fn borrow_subslice_or_vec_internal(
    func: &Func,
    types: &TypeTable,
    interns: &Interns,
    root: &Root,
    path: &[Proj],
    def: DefinitionId,
) -> bool {
    let Some(mut cur) = func.root_ty(*root) else {
        return false;
    };
    for proj in path {
        match proj {
            Proj::Deref => {
                let Some(pt) = value::pointee_of(types, cur) else {
                    return false;
                };
                cur = pt;
            }
            Proj::Field { field, ty } => {
                if is_direct_global_vec_ty(types, cur)
                    && vec_elem_def(types, cur) == Some(def)
                    && !is_vec_len_field(types, interns, cur, *field)
                {
                    return true;
                }
                cur = *ty;
            }
            Proj::Subslice { .. } => {
                if matches!(
                    types.get(types.peeled(cur)).map(|d| &d.kind),
                    Some(TypeKind::Slice(e) | TypeKind::Array { element: e, .. })
                        if adt_def_of(types, *e) == Some(def)
                ) {
                    return true;
                }
            }
            Proj::Index(_) | Proj::ConstantIndex { .. } => {
                let Some(next) = (match types.get(types.peeled(cur)).map(|d| &d.kind) {
                    Some(TypeKind::Array { element, .. } | TypeKind::Slice(element)) => {
                        Some(*element)
                    }
                    _ => None,
                }) else {
                    return false;
                };
                cur = next;
            }
            Proj::Downcast { .. } => {}
        }
    }
    false
}

/// Checks that an `Elem` SSA value flows ONLY (through `Move`/`Copy` or a single-store,
/// non-borrowed local `Cell`) into the element argument of a `VecPush` / `NativeVecPush` /
/// `VecPushMut` / `NativeVecPushMut` call.
fn value_flows_only_to_vec_push(func: &Func, root_v: ValueId) -> bool {
    // Identify single-store, non-borrowed local cells that can serve as transparent temporaries
    // for `let tmp = Elem { ... }; vec.push(tmp);` prior to PEA unboxing.
    let mut cell_store_count: Vec<usize> = vec![0; func.cells.len()];
    let mut cell_borrowed: Vec<bool> = vec![false; func.cells.len()];
    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::CellStore { cell, .. } => {
                    cell_store_count[cell.0 as usize] += 1;
                }
                OpKind::BorrowView {
                    root: Root::Cell(c),
                    ..
                }
                | OpKind::RawView {
                    root: Root::Cell(c),
                    ..
                } => {
                    cell_borrowed[c.0 as usize] = true;
                }
                _ => {}
            }
        }
    }

    let mut tracked: Vec<bool> = vec![false; func.values.len()];
    let mut tracked_cells: Vec<bool> = vec![false; func.cells.len()];
    tracked[root_v.0 as usize] = true;
    let mut changed = true;
    while changed {
        changed = false;
        for block in &func.blocks {
            for op in &block.ops {
                match (&op.kind, op.result) {
                    (OpKind::Move(src) | OpKind::Copy(src), Some(res)) => {
                        if tracked[src.0 as usize] && !tracked[res.0 as usize] {
                            tracked[res.0 as usize] = true;
                            changed = true;
                        }
                    }
                    (OpKind::CellStore { cell, v }, _) => {
                        let ci = cell.0 as usize;
                        if tracked[v.0 as usize]
                            && cell_store_count[ci] == 1
                            && !cell_borrowed[ci]
                            && !tracked_cells[ci]
                        {
                            tracked_cells[ci] = true;
                            changed = true;
                        }
                    }
                    (OpKind::CellLoad(cell), Some(res))
                        if tracked_cells[cell.0 as usize] && !tracked[res.0 as usize] =>
                    {
                        tracked[res.0 as usize] = true;
                        changed = true;
                    }
                    _ => {}
                }
            }
            for (target, args) in block.term.edges() {
                if target.0 == 0 {
                    continue;
                }
                for (pos, &arg) in args.iter().enumerate() {
                    if tracked[arg.0 as usize] {
                        if let Some(&param) = func.blocks[target.0 as usize].params.get(pos) {
                            if !tracked[param.0 as usize] {
                                tracked[param.0 as usize] = true;
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
    }
    let mut saw_push = false;
    for block in &func.blocks {
        match &block.term {
            RTerm::Goto { .. } => {}
            RTerm::Switch { scrut, .. } => {
                if tracked[scrut.0 as usize] {
                    return false;
                }
            }
            _ => {
                for u in block.term.uses() {
                    if tracked[u.0 as usize] {
                        return false;
                    }
                }
            }
        }
        for op in &block.ops {
            for u in op.kind.uses() {
                if !tracked[u.0 as usize] {
                    continue;
                }
                match &op.kind {
                    OpKind::Move(_) | OpKind::Copy(_) => {}
                    OpKind::CellStore { cell, .. } if tracked_cells[cell.0 as usize] => {}
                    OpKind::Call {
                        semantic:
                            CallSemantic::Standard(
                                StandardOperation::VecPush
                                | StandardOperation::NativeVecPush
                                | StandardOperation::VecPushMut
                                | StandardOperation::NativeVecPushMut,
                            ),
                        args,
                        ..
                    } if args.len() >= 2 && args[1] == u => {
                        saw_push = true;
                    }
                    OpKind::Drop {
                        glue: DropGlue::None,
                        ..
                    } => {}
                    _ => return false,
                }
            }
        }
    }
    saw_push
}
