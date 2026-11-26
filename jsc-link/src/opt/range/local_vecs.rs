//! Element upper-bound inference for local non-escaping `Vec`s (`LocalBoundedVec`).

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::heap::global_vec_elem_ty;
use crate::jir::repr::{CellId, Func, OpKind, Proj, RAssertMessage, RBlockId, Root, ValueId};
use crate::jir::semantic::{
    BinaryOp, CallSemantic, CastKind, ConstTree, StandardOperation, TypeId, UnaryOp,
};
use crate::jir::TypeTable;

use super::super::cfg::mask;
use super::super::witness::LocalVecRoot;
use super::{width_max, Analysis, Bound, LocalBoundedVecFact, LocalElemBound, Rel, Site};

/// Tracks borrows, slices, element references, writes, loads, and escape disqualification for
/// candidate local `Vec<u32|usize|u16|u8, Global>` roots.
#[derive(Default)]
struct LocalVecTracker {
    root_borrows: BTreeMap<ValueId, LocalVecRoot>,
    slice_to_root: BTreeMap<ValueId, LocalVecRoot>,
    elem_ref_to_root: BTreeMap<ValueId, LocalVecRoot>,
    init_zeroed_block: BTreeMap<LocalVecRoot, (RBlockId, bool)>,
    disq: BTreeSet<LocalVecRoot>,
    root_writes: BTreeMap<LocalVecRoot, Vec<LocalVecWrite>>,
    load_to_root: BTreeMap<ValueId, LocalVecRoot>,
}

#[derive(Clone, Copy)]
struct LocalVecWrite {
    block: RBlockId,
    op_idx: usize,
    val: ValueId,
}

impl LocalVecTracker {
    fn root_of(&self, v: ValueId) -> Option<LocalVecRoot> {
        self.root_borrows
            .get(&v)
            .or_else(|| self.slice_to_root.get(&v))
            .or_else(|| self.elem_ref_to_root.get(&v))
            .copied()
    }

    fn disqualify_value(&mut self, v: ValueId) {
        if self.init_zeroed_block.contains_key(&LocalVecRoot::Value(v)) {
            self.disq.insert(LocalVecRoot::Value(v));
        }
        if let Some(r) = self.root_of(v) {
            self.disq.insert(r);
        }
    }
}

struct LocalVecProveCx<'a> {
    analysis: &'a Analysis,
    func: &'a Func,
    table: &'a TypeTable,
    tracker: &'a LocalVecTracker,
    proven: &'a BTreeMap<LocalVecRoot, (ValueId, LocalElemBound)>,
}

impl LocalVecProveCx<'_> {
    fn write_satisfies(
        &self,
        self_root: LocalVecRoot,
        write: LocalVecWrite,
        len_c: ValueId,
        bound: LocalElemBound,
    ) -> bool {
        let cv = self.analysis.canon(write.val);
        if self.value_satisfies(self_root, write.block, write.op_idx, cv, len_c, bound) {
            return true;
        }
        if let Some(OpKind::Cast {
            kind: CastKind::IntToInt,
            v: src,
            from,
            to,
        }) = self.analysis.def_kind(self.func, cv)
        {
            if matches!(
                (self.table.int_info(*from), self.table.int_info(*to)),
                (Some((false, _)), Some((false, _)))
            ) {
                return self.value_satisfies(
                    self_root,
                    write.block,
                    write.op_idx,
                    *src,
                    len_c,
                    bound,
                );
            }
        }
        false
    }

    fn value_satisfies(
        &self,
        self_root: LocalVecRoot,
        block: RBlockId,
        op_idx: usize,
        inner: ValueId,
        len_c: ValueId,
        bound: LocalElemBound,
    ) -> bool {
        let ic = self.analysis.canon(inner);
        let load_to_root = &self.tracker.load_to_root;
        // Co-inductive: copying from the same vec preserves its bound!
        if load_to_root.get(&ic) == Some(&self_root) {
            return true;
        }
        // Loaded from another already-proven vec with the same `len_c`!
        if let Some(&other_r) = load_to_root.get(&ic) {
            if let Some(&(ol, ob)) = self.proven.get(&other_r) {
                if ol == len_c
                    && matches!(
                        (ob, bound),
                        (LocalElemBound::Lt, _)
                            | (
                                LocalElemBound::Le,
                                LocalElemBound::Le | LocalElemBound::LePlusOne
                            )
                            | (LocalElemBound::LePlusOne, LocalElemBound::LePlusOne)
                    )
                {
                    return true;
                }
            }
        }
        let active_facts: Vec<usize> = self.analysis.entry_facts[block.0 as usize]
            .iter()
            .copied()
            .chain(self.analysis.facts.iter().enumerate().filter_map(|(i, f)| {
                matches!(f.site, Site::Assert { block: fb, index: fi, .. } if fb == block && fi < op_idx)
                    .then_some(i)
            }))
            .collect();
        let active_ne: Vec<usize> = self.analysis.entry_ne_facts[block.0 as usize]
            .iter()
            .copied()
            .chain(self.analysis.ne_facts.iter().enumerate().filter_map(|(i, f)| {
                matches!(f.site, Site::Assert { block: fb, index: fi, .. } if fb == block && fi < op_idx)
                    .then_some(i)
            }))
            .collect();

        // Constant check:
        if let Some(OpKind::Const(ConstTree::Scalar { bits, bytes })) =
            self.analysis.def_kind(self.func, ic)
        {
            let max = self.analysis.widths[ic.0 as usize].map_or(u128::MAX, width_max);
            let c = mask(*bits, *bytes).min(max);
            let len_lo = self
                .analysis
                .interval_under(len_c, &active_facts, &active_ne)
                .map_or(0, |iv| iv.lo);
            return match bound {
                LocalElemBound::Lt => c < len_lo,
                LocalElemBound::Le => c <= len_lo,
                LocalElemBound::LePlusOne => c <= len_lo.saturating_add(1),
            };
        }

        // Helper: is `x` proven `< len_c` at `(block, op_idx)`?
        let is_lt_len = |x: ValueId| -> bool {
            let xc = self.analysis.canon(x);
            if let (Some(xiv), Some(liv)) = (
                self.analysis.interval_under(xc, &active_facts, &active_ne),
                self.analysis
                    .interval_under(len_c, &active_facts, &active_ne),
            ) {
                if xiv.hi < liv.lo {
                    return true;
                }
            }
            if active_facts.iter().any(|&fi| {
                let f = &self.analysis.facts[fi];
                matches!((f.x, f.y), (Bound::Value(fx), Bound::Value(fy))
                    if self.analysis.canon(fx) == xc && self.analysis.canon(fy) == len_c && f.rel == Rel::Lt)
            }) {
                return true;
            }
            // Or `x` is `load - k` from an already-proven `LocalBoundedVec` (e.g. `su = slot - 2`)!
            if let Some(OpKind::Binary {
                op: BinaryOp::Sub | BinaryOp::SubUnchecked,
                l,
                r,
                ty,
            }) = self.analysis.def_kind(self.func, xc)
            {
                if matches!(self.table.int_info(*ty), Some((false, _))) {
                    if let Some(OpKind::Const(ConstTree::Scalar { bits: kb, bytes })) =
                        self.analysis.def_kind(self.func, self.analysis.canon(*r))
                    {
                        let k = mask(*kb, *bytes);
                        let lc = self.analysis.canon(*l);
                        if let Some(&or) = load_to_root.get(&lc) {
                            if let Some(&(ol, ob)) = self.proven.get(&or) {
                                if ol == len_c {
                                    let liv =
                                        self.analysis.interval_under(lc, &active_facts, &active_ne);
                                    if (ob == LocalElemBound::Le
                                        && k == 1
                                        && liv.is_some_and(|iv| iv.lo >= 1))
                                        || (ob == LocalElemBound::LePlusOne
                                            && k == 2
                                            && liv.is_some_and(|iv| iv.lo >= 2))
                                    {
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Or `x` is directly loaded from an already-proven `Lt` vec!
            if let Some(&or) = load_to_root.get(&xc) {
                if self.proven.get(&or) == Some(&(len_c, LocalElemBound::Lt)) {
                    return true;
                }
            }
            false
        };

        let is_add_const_lt_len = |expected_k: u128| -> bool {
            let Some(OpKind::Binary {
                op: BinaryOp::Add | BinaryOp::AddUnchecked,
                l,
                r,
                ty,
            }) = self.analysis.def_kind(self.func, ic)
            else {
                return false;
            };
            if !matches!(self.table.int_info(*ty), Some((false, _))) {
                return false;
            }
            let is_k = |v: ValueId| {
                matches!(
                    self.analysis.def_kind(self.func, self.analysis.canon(v)),
                    Some(OpKind::Const(ConstTree::Scalar { bits, .. })) if *bits == expected_k
                )
            };
            (is_k(*r) && is_lt_len(*l)) || (is_k(*l) && is_lt_len(*r))
        };

        match bound {
            LocalElemBound::Lt => is_lt_len(ic),
            LocalElemBound::Le => is_lt_len(ic) || is_add_const_lt_len(1),
            LocalElemBound::LePlusOne => is_lt_len(ic) || is_add_const_lt_len(2),
        }
    }
}

impl Analysis {
    /// Computes sound element upper bounds (`< L`, `<= L`, or `<= L + 1`) for local non-escaping
    /// `Vec<u32|usize|u16|u8, Global>` roots — BOTH `CellId`s and PEA-unboxed SSA `ValueId`s
    /// (e.g. `earcut::refine::{ht_sl, he_sl, stack_sl}`, `fflate::h_tree::{node_left, node_right}`,
    /// `d3-hierarchy::circles::siblings::{next, prev}`, `minisearch::order`).
    pub(super) fn compute_local_bounded_vecs(&mut self, func: &Func, table: &TypeTable) {
        let mut tracker = self.discover_roots(func, table);
        self.collect_writes_and_escapes(func, &mut tracker);
        self.prove_element_bounds(func, table, &tracker);
    }

    /// First pass: collect unboxed SSA Vec roots, cell Vec roots, borrows, slices, element refs,
    /// and check direct escape via `RawView` / multi-store / `CellLoad`.
    fn discover_roots(&self, func: &Func, table: &TypeTable) -> LocalVecTracker {
        let mut tracker = LocalVecTracker::default();
        let mut cell_stores: BTreeMap<CellId, usize> = BTreeMap::new();

        let is_unsigned_vec = |ty: TypeId| {
            global_vec_elem_ty(table, ty)
                .and_then(|e| table.int_info(table.peeled(e)))
                .is_some_and(|(signed, _)| !signed)
        };
        let init_kind_of = |v: ValueId| -> Option<bool> {
            let cv = self.canon(v);
            match self.def_kind(func, cv) {
                Some(OpKind::Call {
                    semantic:
                        CallSemantic::Standard(
                            StandardOperation::VecNew
                            | StandardOperation::NativeVecNew
                            | StandardOperation::VecWithCapacity
                            | StandardOperation::NativeVecWithCapacity,
                        ),
                    ..
                }) => Some(true),
                Some(OpKind::Call {
                    semantic: CallSemantic::Standard(StandardOperation::NativeVecFromElemZeroed),
                    ..
                }) => Some(false),
                Some(OpKind::Call {
                    semantic: CallSemantic::Standard(StandardOperation::VecFromElem),
                    args,
                    ..
                }) if !args.is_empty()
                    && matches!(
                        self.def_kind(func, self.canon(args[0])),
                        Some(OpKind::Const(ConstTree::Scalar { bits: 0, .. }))
                    ) =>
                {
                    Some(false)
                }
                _ => None,
            }
        };

        for (ci, cell) in func.cells.iter().enumerate() {
            let cid = CellId(ci as u32);
            if !is_unsigned_vec(cell.ty) {
                tracker.disq.insert(LocalVecRoot::Cell(cid));
            }
        }

        for &b in &self.doms.rpo {
            for op in &func.blocks[b.0 as usize].ops {
                if let Some(res) = op.result {
                    if is_unsigned_vec(func.values[res.0 as usize].ty) {
                        if let Some(is_empty) = init_kind_of(res) {
                            tracker
                                .init_zeroed_block
                                .insert(LocalVecRoot::Value(res), (b, is_empty));
                        }
                    }
                }
                match &op.kind {
                    OpKind::RawView { root, .. } => match root {
                        Root::Cell(c) => {
                            tracker.disq.insert(LocalVecRoot::Cell(*c));
                        }
                        Root::Value(v) => {
                            tracker.disq.insert(LocalVecRoot::Value(*v));
                            if let Some(r) = tracker.root_of(*v) {
                                tracker.disq.insert(r);
                            }
                        }
                    },
                    OpKind::CellStore { cell, v }
                        if !tracker.disq.contains(&LocalVecRoot::Cell(*cell)) =>
                    {
                        // Moving a freshly constructed SSA Vec into a cell transfers tracking to the cell.
                        tracker.disq.insert(LocalVecRoot::Value(*v));
                        let cnt = cell_stores.entry(*cell).or_insert(0);
                        *cnt += 1;
                        if *cnt > 1 {
                            tracker.disq.insert(LocalVecRoot::Cell(*cell));
                        } else if let Some(is_empty) = init_kind_of(*v) {
                            tracker
                                .init_zeroed_block
                                .insert(LocalVecRoot::Cell(*cell), (b, is_empty));
                        } else {
                            tracker.disq.insert(LocalVecRoot::Cell(*cell));
                        }
                    }
                    OpKind::CellLoad(c) if !tracker.disq.contains(&LocalVecRoot::Cell(*c)) => {
                        tracker.disq.insert(LocalVecRoot::Cell(*c));
                    }
                    OpKind::BorrowView { root, path, .. } => {
                        if let Some(res) = op.result {
                            match (root, path.as_slice()) {
                                (Root::Cell(c), [])
                                    if !tracker.disq.contains(&LocalVecRoot::Cell(*c)) =>
                                {
                                    tracker.root_borrows.insert(res, LocalVecRoot::Cell(*c));
                                }
                                (Root::Cell(c), _) => {
                                    tracker.disq.insert(LocalVecRoot::Cell(*c));
                                }
                                (Root::Value(v), [])
                                    if tracker
                                        .init_zeroed_block
                                        .contains_key(&LocalVecRoot::Value(*v))
                                        && !tracker.disq.contains(&LocalVecRoot::Value(*v)) =>
                                {
                                    tracker.root_borrows.insert(res, LocalVecRoot::Value(*v));
                                }
                                (Root::Value(v), [] | [Proj::Deref]) => {
                                    if let Some(&r) = tracker.slice_to_root.get(v) {
                                        tracker.slice_to_root.insert(res, r);
                                    } else if let Some(&r) = tracker.root_borrows.get(v) {
                                        tracker.root_borrows.insert(res, r);
                                    } else if let Some(&r) = tracker.elem_ref_to_root.get(v) {
                                        tracker.elem_ref_to_root.insert(res, r);
                                    }
                                }
                                (
                                    Root::Value(v),
                                    [Proj::Deref, Proj::Index(_)]
                                    | [Proj::Deref, Proj::ConstantIndex { .. }],
                                ) => {
                                    if let Some(&r) = tracker.slice_to_root.get(v) {
                                        tracker.elem_ref_to_root.insert(res, r);
                                    }
                                }
                                (Root::Value(v), _) => {
                                    tracker.disq.insert(LocalVecRoot::Value(*v));
                                    if let Some(r) = tracker.root_of(*v) {
                                        tracker.disq.insert(r);
                                    }
                                }
                            }
                        }
                    }
                    OpKind::Copy(s) | OpKind::Move(s) => {
                        if let Some(res) = op.result {
                            if let Some(&r) = tracker.root_borrows.get(s) {
                                tracker.root_borrows.insert(res, r);
                            }
                            if let Some(&r) = tracker.slice_to_root.get(s) {
                                tracker.slice_to_root.insert(res, r);
                            }
                            if let Some(&r) = tracker.elem_ref_to_root.get(s) {
                                tracker.elem_ref_to_root.insert(res, r);
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
                    } => {
                        if let (Some(res), Some(&a0)) = (op.result, args.first()) {
                            if let Some(&r) = tracker.root_borrows.get(&a0) {
                                tracker.slice_to_root.insert(res, r);
                            }
                        }
                    }
                    OpKind::Call {
                        semantic:
                            CallSemantic::Standard(
                                StandardOperation::VecIndex | StandardOperation::VecIndexMut,
                            ),
                        args,
                        ..
                    } => {
                        if let (Some(res), Some(&a0)) = (op.result, args.first()) {
                            if let Some(&r) = tracker.root_borrows.get(&a0) {
                                tracker.elem_ref_to_root.insert(res, r);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        tracker
    }

    /// Second pass: collect all writes/pushes and element loads, disqualifying roots escaping to
    /// unknown ops or terminators.
    fn collect_writes_and_escapes(&self, func: &Func, tracker: &mut LocalVecTracker) {
        for &b in &self.doms.rpo {
            let blk = &func.blocks[b.0 as usize];
            for u in blk.term.uses() {
                tracker.disqualify_value(u);
            }
            for (oi, op) in blk.ops.iter().enumerate() {
                match &op.kind {
                    OpKind::Deref(ptr) => {
                        if let (Some(res), Some(&r)) =
                            (op.result, tracker.elem_ref_to_root.get(ptr))
                        {
                            tracker.load_to_root.insert(res, r);
                            tracker.load_to_root.insert(self.canon(res), r);
                        }
                    }
                    OpKind::StoreThrough { ptr, v } => {
                        if let Some(&r) = tracker.elem_ref_to_root.get(ptr) {
                            tracker
                                .root_writes
                                .entry(r)
                                .or_default()
                                .push(LocalVecWrite {
                                    block: b,
                                    op_idx: oi,
                                    val: *v,
                                });
                        } else if let Some(&r) = tracker
                            .root_borrows
                            .get(ptr)
                            .or_else(|| tracker.slice_to_root.get(ptr))
                        {
                            tracker.disq.insert(r);
                        }
                    }
                    OpKind::Call { semantic, args, .. } => {
                        let first_root = args
                            .first()
                            .and_then(|a| {
                                tracker
                                    .root_borrows
                                    .get(a)
                                    .or_else(|| tracker.slice_to_root.get(a))
                            })
                            .copied();
                        let allowed_on_first = matches!(
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
                                    | StandardOperation::VecPush
                                    | StandardOperation::NativeVecPush
                                    | StandardOperation::VecPop
                                    | StandardOperation::NativeVecPop
                                    | StandardOperation::VecClear
                                    | StandardOperation::VecTruncate
                                    | StandardOperation::VecReserve
                                    | StandardOperation::VecReserveExact
                                    | StandardOperation::NativeVecReserve
                                    | StandardOperation::NativeVecReserveExact
                                    | StandardOperation::SliceSwap
                                    | StandardOperation::SliceSort
                                    | StandardOperation::SliceSortCmp
                                    | StandardOperation::VecDrop
                            )
                        );
                        if let Some(r) = first_root {
                            if !allowed_on_first {
                                tracker.disq.insert(r);
                            } else if matches!(
                                semantic,
                                CallSemantic::Standard(
                                    StandardOperation::VecPush | StandardOperation::NativeVecPush
                                )
                            ) && args.len() >= 2
                            {
                                tracker
                                    .root_writes
                                    .entry(r)
                                    .or_default()
                                    .push(LocalVecWrite {
                                        block: b,
                                        op_idx: oi,
                                        val: args[1],
                                    });
                            }
                        }
                        for (ai, &a) in args.iter().enumerate() {
                            if ai == 0 && first_root.is_some() {
                                continue;
                            }
                            tracker.disqualify_value(a);
                        }
                    }
                    OpKind::Unary {
                        op: UnaryOp::PtrMetadata,
                        ..
                    }
                    | OpKind::Copy(_)
                    | OpKind::Move(_)
                    | OpKind::BorrowView { .. }
                    | OpKind::Drop { .. } => {}
                    _ => {
                        for u in op.kind.uses() {
                            tracker.disqualify_value(u);
                        }
                    }
                }
            }
        }
    }

    /// Third pass: fixpoint over surviving non-escaping roots checking whether every write obeys a
    /// candidate `(len_canon, LocalElemBound)` against any bounds-check length in `func`.
    fn prove_element_bounds(&mut self, func: &Func, table: &TypeTable, tracker: &LocalVecTracker) {
        let surviving_roots: Vec<LocalVecRoot> = tracker
            .init_zeroed_block
            .keys()
            .copied()
            .filter(|r| !tracker.disq.contains(r) && tracker.root_writes.contains_key(r))
            .collect();
        if surviving_roots.is_empty() {
            return;
        }

        // Collect candidate target lengths (all `PtrMetadata` / `VecLen` canonical values used in
        // BoundsCheck asserts in this function!).
        let mut target_lens: BTreeSet<ValueId> = BTreeSet::new();
        for b in &func.blocks {
            for op in &b.ops {
                if let OpKind::Assert {
                    expected: true,
                    msg: RAssertMessage::BoundsCheck { length, .. },
                    ..
                } = &op.kind
                {
                    target_lens.insert(self.canon(*length));
                }
            }
        }
        if target_lens.is_empty() {
            return;
        }

        // Fixpoint (up to 3 rounds so `ht_sl` -> `su < n` feeds into `he_sl` and `stack_sl`!).
        let mut proven: BTreeMap<LocalVecRoot, (ValueId, LocalElemBound)> = BTreeMap::new();
        for _round in 0..3 {
            let mut changed = false;
            for &root in &surviving_roots {
                if proven.contains_key(&root) {
                    continue;
                }
                let Some(&(init_b, is_empty)) = tracker.init_zeroed_block.get(&root) else {
                    continue;
                };
                let Some(writes) = tracker.root_writes.get(&root) else {
                    continue;
                };
                'len_loop: for &len_c in &target_lens {
                    for bound in [
                        LocalElemBound::Lt,
                        LocalElemBound::Le,
                        LocalElemBound::LePlusOne,
                    ] {
                        if !is_empty && bound == LocalElemBound::Lt {
                            let len_pos =
                                self.interval_in(len_c, init_b).is_some_and(|iv| iv.lo >= 1);
                            if !len_pos {
                                continue;
                            }
                        }
                        let len_iv_max_ok = match bound {
                            LocalElemBound::Lt => true,
                            LocalElemBound::Le => {
                                let w = self.widths[len_c.0 as usize].unwrap_or(32);
                                self.interval(len_c).is_some_and(|iv| iv.hi < width_max(w))
                            }
                            LocalElemBound::LePlusOne => {
                                let w = self.widths[len_c.0 as usize].unwrap_or(32);
                                self.interval(len_c)
                                    .is_some_and(|iv| iv.hi + 1 < width_max(w))
                            }
                        };
                        if !len_iv_max_ok {
                            continue;
                        }
                        let cx = LocalVecProveCx {
                            analysis: self,
                            func,
                            table,
                            tracker,
                            proven: &proven,
                        };
                        let all_writes_ok = writes
                            .iter()
                            .all(|&write| cx.write_satisfies(root, write, len_c, bound));
                        if all_writes_ok {
                            proven.insert(root, (len_c, bound));
                            changed = true;
                            break 'len_loop;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        for (vec_root, (len_canon, bound)) in proven {
            let idx = self.local_bounded_vecs.len();
            self.local_bounded_vecs.push(LocalBoundedVecFact {
                vec_root,
                len_canon,
                bound,
            });
            for (&lv, &lr) in &tracker.load_to_root {
                if lr == vec_root {
                    self.load_to_local_vec.insert(lv, idx);
                }
            }
        }
    }
}
