//! Closed byte-string view analysis (`js-str=bytes`) over JIR-R functions.
//!
//! Identifies connected components of immutable byte slices (`&[u8]`, `*const [u8]`, `*const str`)
//! and thin byte pointers (`*const u8`, `&u8`, `*const ()`, `&[u8; N]`) rooted in `&str` /
//! constant byte slices / specialized `ByteStrParam` parameters that never escape or mutate, so
//! lowering can keep them directly on JS strings rather than materializing `Uint8Array` views.

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::repr::{Dominators, Func, Op, OpKind, Proj, RBlockId, RTerm, Root, ValueId};
use rcgjs_object::jir::semantic::{
    BinaryOp, CallSemantic, Callee, CastKind, ConstTree, DropGlue, StandardOperation, UnaryOp,
};
use rcgjs_object::jir::types::{EnumRepr, NominalForm, TypeId, TypeKind, TypeTable};

use super::value;
use crate::lower::plan::Plan;
use crate::plan::abi_flat::ByteStrParam;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ByteViewMode {
    /// `&[u8]` / `*const [u8]` / `*const str` holding the full JS string (`off == 0`,
    /// `len == base.length`) directly as a plain JS `string`.
    FullSlice,
    /// `&[u8]` / `*const [u8]` holding a `(buf, off, len)` subslice.
    VarSlice,
    /// Thin byte pointer (`*const u8` / `&u8` / `*const ()`), held as a plain JS `number`
    /// byte offset into `base_str`.
    ThinPtr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteView {
    pub mode: ByteViewMode,
    /// Single dominating `&str` (or `&String` for StringBuilder/HostString `as_bytes`)
    /// when unique in the connected component; `None` for multi-root `FullSlice`.
    pub base_str: Option<ValueId>,
    /// True if base is `StringBuilder` (`{ text, cap, len }`) rather than `&str` / `HostString`.
    pub base_is_string_obj: bool,
    /// Slice `ValueId` from which a `[Deref, Index | ConstantIndex]` borrow originates.
    pub borrow_slice: Option<ValueId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ByteKind {
    Slice,
    ThinPtr,
}

#[derive(Clone, Copy, Debug)]
struct ByteViewRoot {
    base: ValueId,
    is_str_obj: bool,
    seed_op: Option<ValueId>,
    kind: ByteKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OriginLattice {
    None,
    Single(usize),
    Conflict,
}

impl OriginLattice {
    fn single(self) -> Option<usize> {
        match self {
            Self::Single(orig) => Some(orig),
            Self::None | Self::Conflict => None,
        }
    }

    fn join_opt(&mut self, incoming: Option<usize>) -> bool {
        match (*self, incoming) {
            (Self::Conflict, _) => false,
            (Self::None, Some(orig)) => {
                *self = Self::Single(orig);
                true
            }
            (Self::None, None) => false,
            (Self::Single(prev), Some(orig)) if prev == orig => false,
            (Self::Single(_), _) => {
                *self = Self::Conflict;
                true
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BaseSliceState {
    Unvisited,
    Unique(ValueId),
    Conflict,
}

impl BaseSliceState {
    fn join_value(&mut self, value: ValueId) -> bool {
        match *self {
            Self::Unvisited => {
                *self = Self::Unique(value);
                true
            }
            Self::Unique(prev) if prev == value => false,
            Self::Unique(_) => {
                *self = Self::Conflict;
                true
            }
            Self::Conflict => false,
        }
    }

    fn join(&mut self, incoming: Self) -> bool {
        match incoming {
            Self::Unvisited => false,
            Self::Unique(v) => self.join_value(v),
            Self::Conflict => {
                if *self != Self::Conflict {
                    *self = Self::Conflict;
                    true
                } else {
                    false
                }
            }
        }
    }
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            let p = self.parent[x];
            self.parent[x] = self.parent[p];
            x = p;
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

pub(super) fn is_immutable_str_ref_pub(table: &TypeTable, ty: TypeId) -> bool {
    is_immutable_str_ref(table, ty)
}

fn is_immutable_str_ref(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        table.get(table.peeled(ty)).map(|d| &d.kind),
        Some(TypeKind::Reference {
            pointee,
            mutable: false,
        }) if matches!(value::kind(table, *pointee), Some(TypeKind::Str))
    )
}

fn is_const_str_raw(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        table.get(table.peeled(ty)).map(|d| &d.kind),
        Some(TypeKind::RawPointer {
            pointee,
            mutable: false,
        }) if matches!(value::kind(table, *pointee), Some(TypeKind::Str))
    )
}

pub(crate) fn is_immutable_byte_slice_pub(table: &TypeTable, ty: TypeId) -> bool {
    is_immutable_byte_slice(table, ty)
}

fn is_immutable_byte_slice(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        table.get(table.peeled(ty)).map(|d| &d.kind),
        Some(
            TypeKind::Reference {
                pointee,
                mutable: false,
            } | TypeKind::RawPointer {
                pointee,
                mutable: false,
            }
        ) if matches!(
            value::kind(table, *pointee),
            Some(TypeKind::Slice(element)) if value::is_u8(table, *element)
        )
    )
}

fn is_readable_byte_slice(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        table.get(table.peeled(ty)).map(|d| &d.kind),
        Some(
            TypeKind::Reference {
                pointee,
                mutable: false,
            } | TypeKind::RawPointer { pointee, .. }
        ) if matches!(
            value::kind(table, *pointee),
            Some(TypeKind::Slice(element)) if value::is_u8(table, *element)
        )
    )
}

fn is_immutable_byte_ptr(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        table.get(table.peeled(ty)).map(|d| &d.kind),
        Some(
            TypeKind::Reference {
                pointee,
                mutable: false,
            } | TypeKind::RawPointer { pointee, .. }
        ) if value::is_u8(table, *pointee)
    )
}

pub(super) fn is_immutable_byte_array_ptr(table: &TypeTable, ty: TypeId) -> Option<u64> {
    let pointee = match table.get(table.peeled(ty)).map(|d| &d.kind) {
        Some(
            TypeKind::Reference {
                pointee,
                mutable: false,
            }
            | TypeKind::RawPointer {
                pointee,
                mutable: false,
            },
        ) => *pointee,
        _ => return None,
    };
    match value::kind(table, table.peeled(pointee)) {
        Some(TypeKind::Array { element, length }) if value::is_u8(table, *element) => Some(*length),
        _ => None,
    }
}

fn is_const_unit_ptr(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        table.get(table.peeled(ty)).map(|d| &d.kind),
        Some(TypeKind::RawPointer { pointee, .. }) if table.is_zst(*pointee)
    )
}

fn is_slice_ty(table: &TypeTable, ty: TypeId) -> bool {
    is_readable_byte_slice(table, ty) || is_const_str_raw(table, ty)
}

fn is_ptr_ty(table: &TypeTable, ty: TypeId) -> bool {
    is_immutable_byte_ptr(table, ty)
        || is_const_unit_ptr(table, ty)
        || is_immutable_byte_array_ptr(table, ty).is_some()
}

pub(super) fn null_niche_byte_payload(
    table: &TypeTable,
    ty: TypeId,
) -> Option<(u32, u32, u32, TypeId, bool)> {
    let peeled = table.peeled(ty);
    let descriptor = table.get(peeled)?;
    let EnumRepr::NullNiche {
        none_variant,
        some_variant,
    } = descriptor.js.enum_repr?
    else {
        return None;
    };
    let TypeKind::Adt {
        form: NominalForm::Enum,
        variants,
        ..
    } = &descriptor.kind
    else {
        return None;
    };
    let some = variants.get(some_variant as usize)?;
    let mut live = some
        .fields
        .iter()
        .enumerate()
        .filter(|(_, f)| !table.is_zst(f.ty));
    let (fidx, field) = live.next()?;
    if live.next().is_some() {
        return None;
    }
    let is_slice = if is_slice_ty(table, field.ty) {
        true
    } else if is_ptr_ty(table, field.ty) {
        false
    } else {
        return None;
    };
    Some((none_variant, some_variant, fidx as u32, field.ty, is_slice))
}

fn classify_ty(table: &TypeTable, ty: TypeId) -> Option<ByteKind> {
    if is_slice_ty(table, ty) {
        Some(ByteKind::Slice)
    } else if is_ptr_ty(table, ty) {
        Some(ByteKind::ThinPtr)
    } else if let Some((_, _, _, _, is_slice)) = null_niche_byte_payload(table, ty) {
        Some(if is_slice {
            ByteKind::Slice
        } else {
            ByteKind::ThinPtr
        })
    } else {
        None
    }
}

pub(super) fn const_u8_slice_str(tree: &ConstTree) -> Option<String> {
    match tree {
        ConstTree::SliceAddress { length: 0, .. } => Some(String::new()),
        ConstTree::Reference(inner) => match inner.as_ref() {
            ConstTree::Aggregate(elems) | ConstTree::Slice(elems) => {
                let mut out = String::with_capacity(elems.len());
                for elem in elems {
                    let ConstTree::Scalar { bits, bytes: 1 } = elem else {
                        return None;
                    };
                    if *bits > 255 {
                        return None;
                    }
                    out.push(char::from(*bits as u8));
                }
                Some(out)
            }
            ConstTree::SliceAddress { length: 0, .. } => Some(String::new()),
            _ => None,
        },
        _ => None,
    }
}

fn is_byte_view_candidate(func: &Func, table: &TypeTable, v: ValueId) -> bool {
    func.ty(v).and_then(|ty| classify_ty(table, ty)).is_some()
}

fn transparent_aggregate_field(
    table: &TypeTable,
    ty: TypeId,
    active_field: Option<u32>,
    fields: &[ValueId],
) -> Option<ValueId> {
    let d = table.get(ty)?;
    let inner = d.js.transparent?;
    let is_union = matches!(
        d.kind,
        TypeKind::Adt {
            form: NominalForm::Union,
            ..
        }
    );
    if is_union && active_field.is_some_and(|a| a != inner.field) {
        return None;
    }
    let idx = if is_union { 0 } else { inner.field as usize };
    fields.get(idx).copied()
}

fn is_reusable_base_str(func: &Func, plan: Option<&Plan>, mut base: ValueId) -> Option<ValueId> {
    let Some(plan) = plan else {
        return Some(base);
    };
    let mut seen = BTreeSet::new();
    while plan.inline.contains(&base) {
        if !seen.insert(base) {
            return None;
        }
        let mut next = None;
        for block in &func.blocks {
            for op in &block.ops {
                if op.result != Some(base) {
                    continue;
                }
                match &op.kind {
                    OpKind::Const(_) => return Some(base),
                    OpKind::Cast {
                        kind: CastKind::PointerCoercion,
                        v: src,
                        ..
                    } if func.blocks.iter().any(|b| {
                        b.ops.iter().any(|o| {
                            o.result == Some(*src)
                                && matches!(&o.kind, OpKind::Const(t) if const_u8_slice_str(t).is_some())
                        })
                    }) =>
                    {
                        return Some(base);
                    }
                    OpKind::Call {
                        semantic:
                            CallSemantic::Standard(
                                StandardOperation::StringAsStr | StandardOperation::NativeStrAsStr,
                            ),
                        args,
                        ..
                    } if args.len() == 1
                        && is_reusable_base_str(func, Some(plan), args[0]).is_some() =>
                    {
                        return Some(base);
                    }
                    OpKind::Move(src) | OpKind::Copy(src) => next = Some(*src),
                    OpKind::BorrowView {
                        root: Root::Value(src),
                        path,
                        mutable: false,
                    }
                    | OpKind::RawView {
                        root: Root::Value(src),
                        path,
                        mutable: false,
                    } if path.is_empty() => next = Some(*src),
                    OpKind::BorrowView {
                        root: Root::Cell(cell),
                        path,
                        mutable: false,
                    } if path.is_empty() => {
                        let single_write = func
                            .blocks
                            .iter()
                            .flat_map(|b| &b.ops)
                            .filter(|o| match &o.kind {
                                OpKind::CellStore { cell: c, .. }
                                | OpKind::BorrowView {
                                    root: Root::Cell(c),
                                    mutable: true,
                                    ..
                                }
                                | OpKind::RawView {
                                    root: Root::Cell(c),
                                    mutable: true,
                                    ..
                                } => *c == *cell,
                                _ => false,
                            })
                            .count()
                            <= 1;
                        return single_write.then_some(base);
                    }
                    _ => return None,
                }
            }
        }
        match next {
            Some(src) => base = src,
            None => return Some(base),
        }
    }
    Some(base)
}

fn join_param_origin(
    slot: &mut Option<usize>,
    conflict: &mut bool,
    incoming: Option<usize>,
) -> bool {
    if let Some(orig) = incoming {
        match *slot {
            None => {
                *slot = Some(orig);
                true
            }
            Some(prev) if prev != orig => {
                *slot = None;
                *conflict = true;
                true
            }
            _ => false,
        }
    } else if slot.is_some() {
        *slot = None;
        *conflict = true;
        true
    } else {
        false
    }
}

#[cfg(test)]
pub(super) fn analyze(
    func: &Func,
    plan: &Plan,
    table: &TypeTable,
    interns: &Interns,
) -> BTreeMap<ValueId, ByteView> {
    analyze_with(func, Some(plan), table, interns, &[], false, &|_, _| None)
}

pub(crate) fn analyze_with(
    func: &Func,
    plan: Option<&Plan>,
    table: &TypeTable,
    interns: &Interns,
    param_roots: &[(usize, ByteStrParam)],
    treat_slice_phi_as_var_slice: bool,
    call_accepts: &dyn Fn(&Op, usize) -> Option<ByteStrParam>,
) -> BTreeMap<ValueId, ByteView> {
    if func.values.is_empty() {
        return BTreeMap::new();
    }
    ByteViewAnalyzer::new(
        func,
        plan,
        table,
        interns,
        treat_slice_phi_as_var_slice,
        call_accepts,
    )
    .run(param_roots)
}

struct ByteViewAnalyzer<'a> {
    func: &'a Func,
    plan: Option<&'a Plan>,
    table: &'a TypeTable,
    interns: &'a Interns,
    treat_slice_phi_as_var_slice: bool,
    call_accepts: &'a dyn Fn(&Op, usize) -> Option<ByteStrParam>,
    block_index: BTreeMap<RBlockId, usize>,
    uf: UnionFind,
    slice_uf: UnionFind,
    roots: Vec<Option<ByteViewRoot>>,
    valid_op_def: Vec<bool>,
    poisoned: Vec<bool>,
    var_slice_seed: Vec<bool>,
    borrow_slice: Vec<Option<ValueId>>,
    def_block: Vec<Option<RBlockId>>,
    use_blocks: Vec<Vec<RBlockId>>,
    param_has_pred: Vec<bool>,
    is_block_param: Vec<bool>,
    const_u8_arr: Vec<bool>,
    cell_has_escaping_view: Vec<bool>,
    cell_has_store: Vec<bool>,
    cell_has_bad_store: Vec<bool>,
    cell_ok: Vec<bool>,
    cell_rep: Vec<Option<usize>>,
    ref_to_cell: Vec<Option<usize>>,
    agg_fields: Vec<Option<Vec<ValueId>>>,
    agg_origin: Vec<Option<usize>>,
    agg_ref: Vec<Option<usize>>,
    ref_to_val: Vec<Option<ValueId>>,
    agg_cell_origin: Vec<OriginLattice>,
    agg_param_conflict: Vec<bool>,
    agg_escaped: Vec<bool>,
    ref_to_val_escaped: Vec<bool>,
}

impl<'a> ByteViewAnalyzer<'a> {
    fn new(
        func: &'a Func,
        plan: Option<&'a Plan>,
        table: &'a TypeTable,
        interns: &'a Interns,
        treat_slice_phi_as_var_slice: bool,
        call_accepts: &'a dyn Fn(&Op, usize) -> Option<ByteStrParam>,
    ) -> Self {
        let n = func.values.len();
        let cells = func.cells.len();
        let block_index = func
            .blocks
            .iter()
            .enumerate()
            .map(|(idx, block)| (block.id, idx))
            .collect();
        Self {
            func,
            plan,
            table,
            interns,
            treat_slice_phi_as_var_slice,
            call_accepts,
            block_index,
            uf: UnionFind::new(n),
            slice_uf: UnionFind::new(n),
            roots: vec![None; n],
            valid_op_def: vec![false; n],
            poisoned: vec![false; n],
            var_slice_seed: vec![false; n],
            borrow_slice: vec![None; n],
            def_block: vec![None; n],
            use_blocks: vec![Vec::new(); n],
            param_has_pred: vec![false; n],
            is_block_param: vec![false; n],
            const_u8_arr: vec![false; n],
            cell_has_escaping_view: vec![false; cells],
            cell_has_store: vec![false; cells],
            cell_has_bad_store: vec![false; cells],
            cell_ok: Vec::new(),
            cell_rep: vec![None; cells],
            ref_to_cell: vec![None; n],
            agg_fields: vec![None; n],
            agg_origin: vec![None; n],
            agg_ref: vec![None; n],
            ref_to_val: vec![None; n],
            agg_cell_origin: vec![OriginLattice::None; cells],
            agg_param_conflict: vec![false; n],
            agg_escaped: vec![false; n],
            ref_to_val_escaped: vec![false; n],
        }
    }

    fn run(mut self, param_roots: &[(usize, ByteStrParam)]) -> BTreeMap<ValueId, ByteView> {
        self.scan_cells_and_consts();
        self.propagate_ref_to_cell();
        self.analyze_agg_containers();
        self.seed_params_and_roots(param_roots);
        self.union_byte_view_components();
        let comps = self.collect_components();
        if comps.is_empty() {
            return BTreeMap::new();
        }
        self.propagate_base_slices();
        self.materialize_byte_views(comps)
    }

    fn scan_cells_and_consts(&mut self) {
        for block in &self.func.blocks {
            for op in &block.ops {
                if let (Some(res), OpKind::Const(tree)) = (op.result, &op.kind) {
                    if const_u8_slice_str(tree).is_some() {
                        self.const_u8_arr[res.0 as usize] = true;
                    }
                }
                match &op.kind {
                    OpKind::BorrowView {
                        root: Root::Cell(c),
                        path,
                        mutable: false,
                    }
                    | OpKind::RawView {
                        root: Root::Cell(c),
                        path,
                        mutable: false,
                    } if path.is_empty() && op.result.is_some() => {
                        let ci = c.0 as usize;
                        if ci < self.func.cells.len() {
                            self.ref_to_cell[op.result.unwrap().0 as usize] = Some(ci);
                        }
                    }
                    OpKind::BorrowView {
                        root: Root::Cell(c),
                        ..
                    }
                    | OpKind::RawView {
                        root: Root::Cell(c),
                        ..
                    }
                    | OpKind::Drop {
                        root: Root::Cell(c),
                        ..
                    } => {
                        if let Some(slot) = self.cell_has_escaping_view.get_mut(c.0 as usize) {
                            *slot = true;
                        }
                    }
                    OpKind::CellStore { cell, v } => {
                        let ci = cell.0 as usize;
                        if let Some(c) = self.func.cells.get(ci) {
                            self.cell_has_store[ci] = true;
                            let ck = classify_ty(self.table, c.ty);
                            let vk = self.func.ty(*v).and_then(|ty| classify_ty(self.table, ty));
                            if ck.is_none() || ck != vk {
                                self.cell_has_bad_store[ci] = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Seeds `ref_to_cell` copies before scanning aggregates that might capture `ref_to_cell`.
    fn propagate_ref_to_cell(&mut self) {
        loop {
            let mut changed = false;
            for block in &self.func.blocks {
                for op in &block.ops {
                    if let (Some(res), OpKind::Copy(src) | OpKind::Move(src)) =
                        (op.result, &op.kind)
                    {
                        if let Some(ci) = self.ref_to_cell[src.0 as usize] {
                            if self.ref_to_cell[res.0 as usize].is_none() {
                                self.ref_to_cell[res.0 as usize] = Some(ci);
                                changed = true;
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Tracks non-escaping aggregate containers (closures, tuples, structs) holding ByteView
    /// fields or shared refs to ByteView cells, and computes final `cell_ok` eligibility.
    fn analyze_agg_containers(&mut self) {
        self.seed_agg_containers();
        self.propagate_agg_origins();
        self.check_agg_escapes();
        self.check_ref_to_cell_escapes();
        self.cell_ok = self
            .func
            .cells
            .iter()
            .enumerate()
            .map(|(ci, c)| {
                !self.cell_has_escaping_view[ci]
                    && self.cell_has_store[ci]
                    && !self.cell_has_bad_store[ci]
                    && classify_ty(self.table, c.ty).is_some()
                    && null_niche_byte_payload(self.table, c.ty).is_none()
            })
            .collect();
    }

    fn seed_agg_containers(&mut self) {
        for block in &self.func.blocks {
            for op in &block.ops {
                if let (
                    Some(res),
                    OpKind::Aggregate {
                        ty,
                        variant: 0,
                        active_field: None,
                        fields,
                    },
                ) = (op.result, &op.kind)
                {
                    if classify_ty(self.table, *ty).is_none()
                        && !self.table.is_zst(*ty)
                        && !self
                            .table
                            .get(*ty)
                            .is_some_and(|d| d.js.transparent.is_some())
                        && fields.iter().any(|&f| {
                            is_byte_view_candidate(self.func, self.table, f)
                                || self.ref_to_cell[f.0 as usize].is_some()
                        })
                    {
                        let ri = res.0 as usize;
                        self.agg_fields[ri] = Some(fields.clone());
                        self.agg_origin[ri] = Some(ri);
                    }
                }
            }
        }
    }

    fn propagate_agg_origins(&mut self) {
        loop {
            let mut changed = false;
            for block in &self.func.blocks {
                for op in &block.ops {
                    changed |= self.propagate_agg_op(op);
                }
                for (target, args) in block.term.edges() {
                    changed |= self.propagate_agg_edge(target, args);
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn propagate_agg_op(&mut self, op: &Op) -> bool {
        let mut changed = false;
        match &op.kind {
            OpKind::Copy(src) | OpKind::Move(src) => {
                if let Some(res) = op.result {
                    let (si, ri) = (src.0 as usize, res.0 as usize);
                    if let Some(orig) = self.agg_origin[si] {
                        if self.agg_origin[ri].is_none() {
                            self.agg_origin[ri] = Some(orig);
                            changed = true;
                        }
                    }
                    if let Some(orig) = self.agg_ref[si] {
                        if self.agg_ref[ri].is_none() {
                            self.agg_ref[ri] = Some(orig);
                            changed = true;
                        }
                    }
                    if let Some(ci) = self.ref_to_cell[si] {
                        if self.ref_to_cell[ri].is_none() {
                            self.ref_to_cell[ri] = Some(ci);
                            changed = true;
                        }
                    }
                    if let Some(v) = self.ref_to_val[si] {
                        if self.ref_to_val[ri].is_none() {
                            self.ref_to_val[ri] = Some(v);
                            changed = true;
                        }
                    }
                }
            }
            OpKind::CellStore { cell, v } => {
                if let Some(slot) = self.agg_cell_origin.get_mut(cell.0 as usize) {
                    changed |= slot.join_opt(self.agg_origin[v.0 as usize]);
                }
            }
            OpKind::CellLoad(cell) => {
                if let Some(res) = op.result {
                    if let Some(orig) = self
                        .agg_cell_origin
                        .get(cell.0 as usize)
                        .and_then(|l| l.single())
                    {
                        if self.agg_origin[res.0 as usize].is_none() {
                            self.agg_origin[res.0 as usize] = Some(orig);
                            changed = true;
                        }
                    }
                }
            }
            OpKind::BorrowView {
                root,
                path,
                mutable: false,
            }
            | OpKind::RawView {
                root,
                path,
                mutable: false,
            } => {
                if let Some(res) = op.result {
                    changed |= self.propagate_agg_view(res.0 as usize, root, path);
                }
            }
            OpKind::Deref(p) => {
                if let Some(res) = op.result {
                    if let Some(orig) = self.agg_ref[p.0 as usize] {
                        if self.agg_origin[res.0 as usize].is_none() {
                            self.agg_origin[res.0 as usize] = Some(orig);
                            changed = true;
                        }
                    }
                }
            }
            OpKind::Field {
                base,
                variant: None,
                field,
            } => {
                if let Some(res) = op.result {
                    let orig = self.agg_origin[base.0 as usize].or(self.agg_ref[base.0 as usize]);
                    if let Some(orig) = orig {
                        if let Some(&fv) = self.agg_fields[orig]
                            .as_ref()
                            .and_then(|fs| fs.get(*field as usize))
                        {
                            if let Some(ci) = self.ref_to_cell[fv.0 as usize] {
                                if self.ref_to_cell[res.0 as usize].is_none() {
                                    self.ref_to_cell[res.0 as usize] = Some(ci);
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        changed
    }

    fn propagate_agg_view(&mut self, ri: usize, root: &Root, path: &[Proj]) -> bool {
        match (root, path) {
            (Root::Value(src), []) => {
                if let Some(orig) = self.agg_origin[src.0 as usize] {
                    if self.agg_ref[ri].is_none() {
                        self.agg_ref[ri] = Some(orig);
                        return true;
                    }
                }
            }
            (Root::Cell(c), []) => {
                if let Some(orig) = self
                    .agg_cell_origin
                    .get(c.0 as usize)
                    .and_then(|l| l.single())
                {
                    if self.agg_ref[ri].is_none() {
                        self.agg_ref[ri] = Some(orig);
                        return true;
                    }
                }
            }
            (Root::Value(src), [Proj::Field { field: f, .. }]) => {
                if let Some(orig) = self.agg_origin[src.0 as usize] {
                    return self.record_agg_field_ref(ri, orig, *f);
                }
            }
            (Root::Value(src), [Proj::Deref, Proj::Field { field: f, .. }]) => {
                if let Some(orig) = self.agg_ref[src.0 as usize] {
                    return self.record_agg_field_ref(ri, orig, *f);
                }
            }
            (Root::Cell(c), [Proj::Field { field: f, .. }]) => {
                if let Some(orig) = self
                    .agg_cell_origin
                    .get(c.0 as usize)
                    .and_then(|l| l.single())
                {
                    return self.record_agg_field_ref(ri, orig, *f);
                }
            }
            _ => {}
        }
        false
    }

    fn record_agg_field_ref(&mut self, ri: usize, orig: usize, field: u32) -> bool {
        let Some(&fv) = self.agg_fields[orig]
            .as_ref()
            .and_then(|fs| fs.get(field as usize))
        else {
            return false;
        };
        if is_byte_view_candidate(self.func, self.table, fv) && self.ref_to_val[ri].is_none() {
            self.ref_to_val[ri] = Some(fv);
            return true;
        }
        false
    }

    fn propagate_agg_edge(&mut self, target: RBlockId, args: &[ValueId]) -> bool {
        let Some(&bi) = self.block_index.get(&target) else {
            return false;
        };
        let mut changed = false;
        for (pos, &arg) in args.iter().enumerate() {
            let Some(&param) = self.func.blocks[bi].params.get(pos) else {
                continue;
            };
            let (ai, pi) = (arg.0 as usize, param.0 as usize);
            if self.agg_param_conflict[pi] {
                continue;
            }
            let src_orig = self.agg_origin[ai];
            changed |= join_param_origin(
                &mut self.agg_origin[pi],
                &mut self.agg_param_conflict[pi],
                src_orig,
            );
            let src_ref = self.agg_ref[ai];
            changed |= join_param_origin(
                &mut self.agg_ref[pi],
                &mut self.agg_param_conflict[pi],
                src_ref,
            );
        }
        changed
    }

    fn check_agg_escapes(&mut self) {
        for block in &self.func.blocks {
            for op in &block.ops {
                self.check_agg_op_escapes(op);
            }
            self.check_agg_term_escapes(&block.term);
            for op in &block.ops {
                self.check_agg_cell_escapes(op);
            }
        }
    }

    fn check_agg_op_escapes(&mut self, op: &Op) {
        let is_agg_field_op = matches!(
            &op.kind,
            OpKind::Field { variant: None, .. }
                | OpKind::Copy(_)
                | OpKind::Move(_)
                | OpKind::Drop {
                    glue: DropGlue::None,
                    ..
                }
        );
        let is_safe_agg_borrow = matches!(
            &op.kind,
            OpKind::BorrowView {
                root: Root::Value(_),
                path,
                mutable: false,
            } | OpKind::RawView {
                root: Root::Value(_),
                path,
                mutable: false,
            } if path.is_empty()
                || matches!(
                    path.as_slice(),
                    [Proj::Field { .. }] | [Proj::Deref, Proj::Field { .. }]
                )
        );
        let is_safe_agg_store = matches!(
            &op.kind,
            OpKind::CellStore { cell, .. }
                if self
                    .agg_cell_origin
                    .get(cell.0 as usize)
                    .and_then(|l| l.single())
                    .is_some()
        );
        let is_safe_agg_deref =
            matches!(&op.kind, OpKind::Deref(p) if self.agg_ref[p.0 as usize].is_some());
        if !is_agg_field_op && !is_safe_agg_borrow && !is_safe_agg_store && !is_safe_agg_deref {
            for u in op.kind.uses() {
                if let Some(orig) = self.agg_origin[u.0 as usize].or(self.agg_ref[u.0 as usize]) {
                    self.agg_escaped[orig] = true;
                }
            }
        }
        if !matches!(
            &op.kind,
            OpKind::Deref(_) | OpKind::Copy(_) | OpKind::Move(_)
        ) {
            for u in op.kind.uses() {
                if self.ref_to_val[u.0 as usize].is_some() {
                    self.ref_to_val_escaped[u.0 as usize] = true;
                }
            }
        }
    }

    fn check_agg_term_escapes(&mut self, term: &RTerm) {
        for (target, args) in term.edges() {
            let bi = self.block_index.get(&target).copied();
            for (pos, &arg) in args.iter().enumerate() {
                let ai = arg.0 as usize;
                if self.ref_to_val[ai].is_some() {
                    self.ref_to_val_escaped[ai] = true;
                }
                if let Some(orig) = self.agg_origin[ai] {
                    let ok = bi
                        .and_then(|bi| self.func.blocks[bi].params.get(pos))
                        .is_some_and(|p| {
                            let pi = p.0 as usize;
                            !self.agg_param_conflict[pi] && self.agg_origin[pi] == Some(orig)
                        });
                    if !ok {
                        self.agg_escaped[orig] = true;
                    }
                }
                if let Some(orig) = self.agg_ref[ai] {
                    let ok = bi
                        .and_then(|bi| self.func.blocks[bi].params.get(pos))
                        .is_some_and(|p| {
                            let pi = p.0 as usize;
                            !self.agg_param_conflict[pi] && self.agg_ref[pi] == Some(orig)
                        });
                    if !ok {
                        self.agg_escaped[orig] = true;
                    }
                }
            }
        }
        match term {
            RTerm::Switch { scrut, .. } => {
                if let Some(orig) =
                    self.agg_origin[scrut.0 as usize].or(self.agg_ref[scrut.0 as usize])
                {
                    self.agg_escaped[orig] = true;
                }
                if self.ref_to_val[scrut.0 as usize].is_some() {
                    self.ref_to_val_escaped[scrut.0 as usize] = true;
                }
            }
            RTerm::Return(Some(u)) | RTerm::Throw(Some(u)) => {
                if let Some(orig) = self.agg_origin[u.0 as usize].or(self.agg_ref[u.0 as usize]) {
                    self.agg_escaped[orig] = true;
                }
                if self.ref_to_val[u.0 as usize].is_some() {
                    self.ref_to_val_escaped[u.0 as usize] = true;
                }
            }
            RTerm::Goto(..)
            | RTerm::Invoke { .. }
            | RTerm::Return(None)
            | RTerm::Throw(None)
            | RTerm::Unreachable => {}
        }
    }

    fn check_agg_cell_escapes(&mut self, op: &Op) {
        match &op.kind {
            OpKind::BorrowView {
                root: Root::Cell(c),
                path,
                mutable,
            }
            | OpKind::RawView {
                root: Root::Cell(c),
                path,
                mutable,
            } => {
                if let Some(orig) = self
                    .agg_cell_origin
                    .get(c.0 as usize)
                    .and_then(|l| l.single())
                {
                    let ok_path = !*mutable
                        && (path.is_empty() || matches!(path.as_slice(), [Proj::Field { .. }]));
                    if !ok_path {
                        self.agg_escaped[orig] = true;
                    }
                }
            }
            OpKind::Drop {
                root: Root::Cell(c),
                glue,
                ..
            } if !matches!(glue, DropGlue::None) => {
                if let Some(orig) = self
                    .agg_cell_origin
                    .get(c.0 as usize)
                    .and_then(|l| l.single())
                {
                    self.agg_escaped[orig] = true;
                }
            }
            _ => {}
        }
    }

    /// Checks `ref_to_cell` escapes: allowed in `Deref`, `Copy`/`Move`, `DropGlue::None`, or
    /// captured inside a non-escaping `agg_fields[r]`.
    fn check_ref_to_cell_escapes(&mut self) {
        for block in &self.func.blocks {
            for op in &block.ops {
                let safe_ref_use = match &op.kind {
                    OpKind::Deref(p) | OpKind::Copy(p) | OpKind::Move(p)
                        if self.ref_to_cell[p.0 as usize].is_some() =>
                    {
                        true
                    }
                    OpKind::Drop {
                        glue: DropGlue::None,
                        ..
                    } => true,
                    OpKind::Aggregate { .. }
                        if op.result.is_some_and(|r| {
                            self.agg_fields[r.0 as usize].is_some()
                                && !self.agg_escaped[r.0 as usize]
                        }) =>
                    {
                        true
                    }
                    _ => false,
                };
                if !safe_ref_use {
                    for u in op.kind.uses() {
                        if let Some(ci) = self.ref_to_cell[u.0 as usize] {
                            self.cell_has_escaping_view[ci] = true;
                        }
                    }
                }
            }
            for u in block.term.uses() {
                if let Some(ci) = self.ref_to_cell[u.0 as usize] {
                    self.cell_has_escaping_view[ci] = true;
                }
            }
        }
    }

    fn seed_params_and_roots(&mut self, param_roots: &[(usize, ByteStrParam)]) {
        for (bi, block) in self.func.blocks.iter().enumerate() {
            for &param in &block.params {
                let idx = param.0 as usize;
                self.def_block[idx] = Some(block.id);
                if bi > 0 && is_byte_view_candidate(self.func, self.table, param) {
                    self.is_block_param[idx] = true;
                    if self.treat_slice_phi_as_var_slice
                        && self
                            .func
                            .ty(param)
                            .and_then(|ty| classify_ty(self.table, ty))
                            == Some(ByteKind::Slice)
                    {
                        self.var_slice_seed[idx] = true;
                    }
                }
            }
            for op in &block.ops {
                if let Some(res) = op.result {
                    self.def_block[res.0 as usize] = Some(block.id);
                }
                for u in op.kind.uses() {
                    self.use_blocks[u.0 as usize].push(block.id);
                }
            }
            for u in block.term.uses() {
                self.use_blocks[u.0 as usize].push(block.id);
            }
        }

        for &(pos, bs_mode) in param_roots {
            if let Some(&param) = self.func.params.get(pos) {
                let idx = param.0 as usize;
                if self
                    .func
                    .ty(param)
                    .is_some_and(|ty| is_immutable_byte_slice(self.table, ty))
                {
                    self.roots[idx] = Some(ByteViewRoot {
                        base: param,
                        is_str_obj: false,
                        seed_op: None,
                        kind: ByteKind::Slice,
                    });
                    self.valid_op_def[idx] = true;
                    if bs_mode == ByteStrParam::Sub {
                        self.var_slice_seed[idx] = true;
                    }
                }
            }
        }
    }

    fn union_byte_view_components(&mut self) {
        for block in &self.func.blocks {
            for op in &block.ops {
                if let Some(res) = op.result {
                    self.union_valued_op(op, res);
                } else {
                    self.union_void_op(op);
                }
            }
            self.union_term_edges(&block.term);
        }

        if let Some(plan) = self.plan {
            for v in &plan.physical_slices {
                if let Some(slot) = self.var_slice_seed.get_mut(v.0 as usize) {
                    *slot = true;
                }
            }
        }
    }

    fn union_same_kind(&mut self, a: usize, b: usize, kind: ByteKind) {
        self.uf.union(a, b);
        if kind == ByteKind::Slice {
            self.slice_uf.union(a, b);
        }
    }

    fn union_cell_value(&mut self, ci: usize, vi: usize, kind: ByteKind) {
        if let Some(rep) = self.cell_rep[ci] {
            self.union_same_kind(rep, vi, kind);
        } else {
            self.cell_rep[ci] = Some(vi);
        }
    }

    fn poison_candidate_uses(&mut self, op: &Op) {
        for u in op.kind.uses() {
            if is_byte_view_candidate(self.func, self.table, u) {
                self.poisoned[u.0 as usize] = true;
            }
        }
    }

    fn union_void_op(&mut self, op: &Op) {
        if let OpKind::CellStore { cell, v } = &op.kind {
            let ci = cell.0 as usize;
            if self.cell_ok.get(ci).copied().unwrap_or(false) {
                let ck = classify_ty(self.table, self.func.cells[ci].ty).unwrap();
                self.union_cell_value(ci, v.0 as usize, ck);
                return;
            }
        }
        if let OpKind::Call {
            callee: Callee::Direct { .. },
            host: None,
            args,
            ..
        } = &op.kind
        {
            for (arg_idx, &arg) in args.iter().enumerate() {
                if is_byte_view_candidate(self.func, self.table, arg) {
                    let accepted = (self.call_accepts)(op, arg_idx).is_some()
                        && self
                            .func
                            .ty(arg)
                            .is_some_and(|ty| is_immutable_byte_slice(self.table, ty));
                    if !accepted {
                        self.poisoned[arg.0 as usize] = true;
                    }
                }
            }
            return;
        }
        if let OpKind::Intrinsic {
            name, types, args, ..
        } = &op.kind
        {
            let name_s = self.interns.str(*name).unwrap_or("");
            if matches!(name_s, "copy_nonoverlapping" | "copy")
                && args.len() == 3
                && types.first().is_some_and(|t| value::is_u8(self.table, *t))
                && self
                    .func
                    .ty(args[0])
                    .is_some_and(|lt| is_immutable_byte_ptr(self.table, lt))
            {
                if is_byte_view_candidate(self.func, self.table, args[1]) {
                    self.poisoned[args[1].0 as usize] = true;
                }
                return;
            }
        }
        self.poison_candidate_uses(op);
    }

    fn union_valued_op(&mut self, op: &Op, res: ValueId) {
        let r = res.0 as usize;
        let res_kind = self.func.ty(res).and_then(|ty| classify_ty(self.table, ty));

        let handled = match &op.kind {
            OpKind::Cast { .. } => self.union_cast_op(op, res, res_kind),
            OpKind::BorrowView { .. } | OpKind::RawView { .. } => {
                self.union_view_op(op, res, res_kind)
            }
            OpKind::Move(src) | OpKind::Copy(src) => {
                let sk = self
                    .func
                    .ty(*src)
                    .and_then(|ty| classify_ty(self.table, ty));
                if let (Some(sk), Some(rk)) = (sk, res_kind) {
                    if sk == rk {
                        self.union_same_kind(src.0 as usize, r, sk);
                        self.valid_op_def[r] = true;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            OpKind::Aggregate { .. } => self.union_aggregate_op(op, r, res_kind),
            OpKind::Field { .. } => self.union_field_op(op, res, res_kind),
            OpKind::Const(tree)
                if const_u8_slice_str(tree).is_some()
                    && self
                        .func
                        .ty(res)
                        .is_some_and(|ty| is_immutable_byte_slice(self.table, ty)) =>
            {
                self.roots[r] = Some(ByteViewRoot {
                    base: res,
                    is_str_obj: false,
                    seed_op: None,
                    kind: ByteKind::Slice,
                });
                self.valid_op_def[r] = true;
                true
            }
            OpKind::Const(ConstTree::Variant { variant, .. })
                if self.func.ty(res).is_some_and(|ty| {
                    null_niche_byte_payload(self.table, ty)
                        .is_some_and(|(none_v, _, _, _, _)| *variant == none_v)
                }) =>
            {
                self.valid_op_def[r] = true;
                true
            }
            OpKind::CellLoad(cell) => self.union_cell_load(cell.0 as usize, r, res_kind),
            OpKind::Discriminant(base)
                if self
                    .func
                    .ty(*base)
                    .and_then(|ty| null_niche_byte_payload(self.table, ty))
                    .is_some() =>
            {
                true
            }
            OpKind::SliceView { ptr, .. } => {
                let pk = self
                    .func
                    .ty(*ptr)
                    .filter(|ty| null_niche_byte_payload(self.table, *ty).is_none())
                    .and_then(|ty| classify_ty(self.table, ty));
                if pk.is_some()
                    && self
                        .func
                        .ty(res)
                        .is_some_and(|ty| is_slice_ty(self.table, ty))
                {
                    self.uf.union(ptr.0 as usize, r);
                    self.valid_op_def[r] = true;
                    self.var_slice_seed[r] = true;
                    true
                } else {
                    false
                }
            }
            OpKind::Binary {
                op: BinaryOp::Offset,
                l,
                ty,
                ..
            } => {
                if is_immutable_byte_ptr(self.table, *ty)
                    && self
                        .func
                        .ty(*l)
                        .is_some_and(|lt| is_immutable_byte_ptr(self.table, lt))
                    && self
                        .func
                        .ty(res)
                        .is_some_and(|rt| is_immutable_byte_ptr(self.table, rt))
                {
                    self.uf.union(l.0 as usize, r);
                    self.valid_op_def[r] = true;
                    true
                } else {
                    false
                }
            }
            OpKind::Binary {
                op:
                    BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge,
                l,
                r: rhs,
                ty,
            } => {
                if is_ptr_ty(self.table, *ty) {
                    self.uf.union(l.0 as usize, rhs.0 as usize);
                    true
                } else {
                    !is_slice_ty(self.table, *ty)
                        && !is_byte_view_candidate(self.func, self.table, *l)
                        && !is_byte_view_candidate(self.func, self.table, *rhs)
                }
            }
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v,
                ..
            } => self
                .func
                .ty(*v)
                .is_some_and(|ty| is_slice_ty(self.table, ty)),
            OpKind::Deref(ptr) => self.union_deref_op(*ptr, r, res_kind),
            OpKind::Intrinsic { .. } => self.union_intrinsic_op(op, res, res_kind),
            OpKind::Call { .. } => self.union_call_op(op, r, res_kind),
            _ => false,
        };

        if !handled {
            self.poison_candidate_uses(op);
            if res_kind.is_some() {
                self.poisoned[r] = true;
            }
        }
    }

    fn union_cast_op(&mut self, op: &Op, res: ValueId, res_kind: Option<ByteKind>) -> bool {
        let OpKind::Cast {
            kind: kind @ (CastKind::Transmute | CastKind::PtrToPtr | CastKind::PointerCoercion),
            v: src,
            from,
            to,
        } = &op.kind
        else {
            return false;
        };
        let r = res.0 as usize;
        if is_immutable_str_ref(self.table, *from) {
            if let Some(k) = classify_ty(self.table, *to) {
                self.roots[r] = Some(ByteViewRoot {
                    base: *src,
                    is_str_obj: false,
                    seed_op: None,
                    kind: k,
                });
                self.valid_op_def[r] = true;
                return true;
            }
        } else if is_slice_ty(self.table, *from) && is_immutable_str_ref(self.table, *to) {
            return true;
        } else if matches!(kind, CastKind::PointerCoercion)
            && self.const_u8_arr[src.0 as usize]
            && is_immutable_byte_array_ptr(self.table, *from).is_some()
            && is_immutable_byte_slice(self.table, *to)
        {
            self.roots[r] = Some(ByteViewRoot {
                base: res,
                is_str_obj: false,
                seed_op: None,
                kind: ByteKind::Slice,
            });
            self.valid_op_def[r] = true;
            return true;
        } else if null_niche_byte_payload(self.table, *from).is_none()
            && null_niche_byte_payload(self.table, *to).is_none()
        {
            if let (Some(sk), Some(rk)) = (classify_ty(self.table, *from), res_kind) {
                if sk == ByteKind::Slice || rk == ByteKind::ThinPtr {
                    let s = src.0 as usize;
                    self.uf.union(s, r);
                    self.valid_op_def[r] = true;
                    if sk == ByteKind::Slice && rk == ByteKind::Slice {
                        self.slice_uf.union(s, r);
                    }
                    return true;
                }
            }
        }
        false
    }

    fn union_view_op(&mut self, op: &Op, res: ValueId, res_kind: Option<ByteKind>) -> bool {
        let r = res.0 as usize;
        if let OpKind::RawView {
            root: Root::Value(src),
            path,
            mutable: false,
        } = &op.kind
        {
            if path.as_slice() == [Proj::Deref]
                && self
                    .func
                    .ty(*src)
                    .is_some_and(|ty| is_immutable_str_ref(self.table, ty))
                && self
                    .func
                    .ty(res)
                    .is_some_and(|ty| is_const_str_raw(self.table, ty))
            {
                self.roots[r] = Some(ByteViewRoot {
                    base: *src,
                    is_str_obj: false,
                    seed_op: None,
                    kind: ByteKind::Slice,
                });
                self.valid_op_def[r] = true;
                return true;
            }
        }

        let (OpKind::BorrowView {
            root: Root::Value(src),
            path,
            mutable: false,
        }
        | OpKind::RawView {
            root: Root::Value(src),
            path,
            mutable: false,
        }) = &op.kind
        else {
            return false;
        };

        let src_not_niche = self
            .func
            .ty(*src)
            .is_some_and(|ty| null_niche_byte_payload(self.table, ty).is_none());
        let res_not_niche = self
            .func
            .ty(res)
            .is_some_and(|ty| null_niche_byte_payload(self.table, ty).is_none());
        if let (Some(sk), Some(rk)) = (
            src_not_niche
                .then(|| {
                    self.func
                        .ty(*src)
                        .and_then(|ty| classify_ty(self.table, ty))
                })
                .flatten(),
            res_not_niche.then_some(res_kind).flatten(),
        ) {
            let s = src.0 as usize;
            match path.as_slice() {
                [Proj::Deref] if sk == rk => {
                    self.union_same_kind(s, r, sk);
                    self.valid_op_def[r] = true;
                    return true;
                }
                [Proj::Deref, Proj::Index(_) | Proj::ConstantIndex { .. }]
                    if (sk == ByteKind::Slice
                        || self.func.ty(*src).is_some_and(|ty| {
                            is_immutable_byte_array_ptr(self.table, ty).is_some()
                        }))
                        && self
                            .func
                            .ty(res)
                            .is_some_and(|ty| is_immutable_byte_ptr(self.table, ty)) =>
                {
                    self.uf.union(s, r);
                    self.valid_op_def[r] = true;
                    if sk == ByteKind::Slice {
                        self.borrow_slice[r] = Some(*src);
                    }
                    return true;
                }
                [Proj::Deref, Proj::Subslice { .. }]
                    if sk == ByteKind::Slice && rk == ByteKind::Slice =>
                {
                    self.uf.union(s, r);
                    self.valid_op_def[r] = true;
                    self.var_slice_seed[r] = true;
                    return true;
                }
                _ => {}
            }
        } else if matches!(&op.kind, OpKind::BorrowView { mutable: false, .. })
            && path.as_slice() == [Proj::Deref]
            && self
                .func
                .ty(*src)
                .is_some_and(|ty| is_slice_ty(self.table, ty))
            && self
                .func
                .ty(res)
                .is_some_and(|ty| is_immutable_str_ref(self.table, ty))
        {
            return true;
        }
        false
    }

    fn union_aggregate_op(&mut self, op: &Op, r: usize, res_kind: Option<ByteKind>) -> bool {
        let OpKind::Aggregate {
            ty,
            variant,
            active_field,
            fields,
        } = &op.kind
        else {
            return false;
        };
        if self.agg_fields[r].is_some() && !self.agg_escaped[r] {
            return true;
        }
        if let Some((none_v, some_v, fidx, _, is_slice)) = null_niche_byte_payload(self.table, *ty)
        {
            if *variant == none_v {
                self.valid_op_def[r] = true;
                return true;
            }
            if *variant == some_v {
                if let Some(&src) = fields.get(fidx as usize) {
                    let kind = if is_slice {
                        ByteKind::Slice
                    } else {
                        ByteKind::ThinPtr
                    };
                    self.union_same_kind(src.0 as usize, r, kind);
                    self.valid_op_def[r] = true;
                    return true;
                }
            }
            return false;
        }
        if fields.len() == 2
            && matches!(
                self.table.get(*ty).map(|d| &d.kind),
                Some(TypeKind::RawPointer { .. })
            )
            && res_kind == Some(ByteKind::Slice)
            && self
                .func
                .ty(fields[0])
                .and_then(|t| classify_ty(self.table, t))
                .is_some()
        {
            self.uf.union(fields[0].0 as usize, r);
            self.valid_op_def[r] = true;
            self.var_slice_seed[r] = true;
            return true;
        }
        if let Some(rk) = res_kind {
            if let Some(src) = transparent_aggregate_field(self.table, *ty, *active_field, fields) {
                if self.func.ty(src).and_then(|t| classify_ty(self.table, t)) == Some(rk) {
                    self.union_same_kind(src.0 as usize, r, rk);
                    self.valid_op_def[r] = true;
                    return true;
                }
            }
        }
        false
    }

    fn union_field_op(&mut self, op: &Op, res: ValueId, res_kind: Option<ByteKind>) -> bool {
        let OpKind::Field {
            base,
            variant,
            field,
        } = &op.kind
        else {
            return false;
        };
        let r = res.0 as usize;
        if variant.is_none() {
            if let Some(orig) = self.agg_origin[base.0 as usize].or(self.agg_ref[base.0 as usize]) {
                if !self.agg_escaped[orig] {
                    if let Some(&src) = self.agg_fields[orig]
                        .as_ref()
                        .and_then(|fs| fs.get(*field as usize))
                    {
                        let sk = self.func.ty(src).and_then(|ty| classify_ty(self.table, ty));
                        if let (Some(sk), Some(rk)) = (sk, res_kind) {
                            if sk == rk {
                                self.union_same_kind(src.0 as usize, r, sk);
                                self.valid_op_def[r] = true;
                                return true;
                            }
                        }
                    }
                    return false;
                }
            }
        }
        if let Some((_, some_v, fidx, _, is_slice)) = self
            .func
            .ty(*base)
            .and_then(|ty| null_niche_byte_payload(self.table, ty))
        {
            if *variant == Some(some_v) && *field == fidx {
                let kind = if is_slice {
                    ByteKind::Slice
                } else {
                    ByteKind::ThinPtr
                };
                self.union_same_kind(base.0 as usize, r, kind);
                self.valid_op_def[r] = true;
                return true;
            }
            if self.func.ty(res).is_some_and(|ty| self.table.is_zst(ty)) {
                return true;
            }
            return false;
        }
        if let Some(rk) = res_kind {
            let transparent_ok = self.func.ty(*base).is_some_and(|bty| {
                self.table
                    .get(bty)
                    .and_then(|d| d.js.transparent)
                    .is_some_and(|inner| inner.field == *field)
                    && classify_ty(self.table, bty) == Some(rk)
            });
            if transparent_ok {
                self.union_same_kind(base.0 as usize, r, rk);
                self.valid_op_def[r] = true;
                return true;
            }
        }
        false
    }

    fn union_cell_load(&mut self, ci: usize, r: usize, res_kind: Option<ByteKind>) -> bool {
        if !self.cell_ok.get(ci).copied().unwrap_or(false) {
            return false;
        }
        let ck = classify_ty(self.table, self.func.cells[ci].ty).unwrap();
        if res_kind != Some(ck) {
            return false;
        }
        self.union_cell_value(ci, r, ck);
        self.valid_op_def[r] = true;
        true
    }

    fn union_deref_op(&mut self, ptr: ValueId, r: usize, res_kind: Option<ByteKind>) -> bool {
        if self
            .func
            .ty(ptr)
            .is_some_and(|ty| is_immutable_byte_ptr(self.table, ty))
        {
            return true;
        }
        if let Some(ci) = self.ref_to_cell[ptr.0 as usize] {
            return self.union_cell_load(ci, r, res_kind);
        }
        if let Some(src) = self.ref_to_val[ptr.0 as usize] {
            if !self.ref_to_val_escaped[ptr.0 as usize] {
                let sk = self.func.ty(src).and_then(|ty| classify_ty(self.table, ty));
                if let (Some(sk), Some(rk)) = (sk, res_kind) {
                    if sk == rk {
                        self.union_same_kind(src.0 as usize, r, sk);
                        self.valid_op_def[r] = true;
                        return true;
                    }
                }
            }
        }
        false
    }

    fn union_intrinsic_op(&mut self, op: &Op, res: ValueId, res_kind: Option<ByteKind>) -> bool {
        let OpKind::Intrinsic {
            name, types, args, ..
        } = &op.kind
        else {
            return false;
        };
        let r = res.0 as usize;
        let name_s = self.interns.str(*name).unwrap_or("");
        match name_s {
            "transmute" | "transmute_unchecked" if types.len() == 2 && args.len() == 1 => {
                let (from, to, src) = (types[0], types[1], args[0]);
                if is_immutable_str_ref(self.table, from) {
                    if let Some(k) = classify_ty(self.table, to) {
                        self.roots[r] = Some(ByteViewRoot {
                            base: src,
                            is_str_obj: false,
                            seed_op: None,
                            kind: k,
                        });
                        self.valid_op_def[r] = true;
                        return true;
                    }
                } else if is_slice_ty(self.table, from) && is_immutable_str_ref(self.table, to) {
                    return true;
                } else if let (Some(sk), Some(rk)) = (classify_ty(self.table, from), res_kind) {
                    if sk == ByteKind::Slice || rk == ByteKind::ThinPtr {
                        let s = src.0 as usize;
                        self.uf.union(s, r);
                        self.valid_op_def[r] = true;
                        if sk == ByteKind::Slice && rk == ByteKind::Slice {
                            self.slice_uf.union(s, r);
                        }
                        return true;
                    }
                }
                false
            }
            "offset" | "arith_offset"
                if args.len() == 2
                    && types.first().is_some_and(|t| value::is_u8(self.table, *t))
                    && self
                        .func
                        .ty(args[0])
                        .is_some_and(|lt| is_immutable_byte_ptr(self.table, lt))
                    && self
                        .func
                        .ty(res)
                        .is_some_and(|rt| is_immutable_byte_ptr(self.table, rt)) =>
            {
                self.uf.union(args[0].0 as usize, r);
                self.valid_op_def[r] = true;
                true
            }
            "ptr_offset_from" | "ptr_offset_from_unsigned"
                if args.len() == 2
                    && types.first().is_some_and(|t| value::is_u8(self.table, *t))
                    && self
                        .func
                        .ty(args[0])
                        .is_some_and(|lt| is_immutable_byte_ptr(self.table, lt))
                    && self
                        .func
                        .ty(args[1])
                        .is_some_and(|rt| is_immutable_byte_ptr(self.table, rt)) =>
            {
                self.uf.union(args[0].0 as usize, args[1].0 as usize);
                true
            }
            "read_via_copy" | "volatile_load" | "unaligned_volatile_load"
                if args.len() == 1
                    && types.first().is_some_and(|t| value::is_u8(self.table, *t))
                    && self
                        .func
                        .ty(args[0])
                        .is_some_and(|lt| is_immutable_byte_ptr(self.table, lt)) =>
            {
                true
            }
            "ptr_metadata"
                if args.len() == 1
                    && self
                        .func
                        .ty(args[0])
                        .is_some_and(|ty| is_slice_ty(self.table, ty)) =>
            {
                true
            }
            "slice_get_unchecked"
                if args.len() == 2
                    && self
                        .func
                        .ty(args[0])
                        .is_some_and(|ty| is_immutable_byte_slice(self.table, ty))
                    && self
                        .func
                        .ty(res)
                        .is_some_and(|ty| is_immutable_byte_ptr(self.table, ty)) =>
            {
                self.uf.union(args[0].0 as usize, r);
                self.valid_op_def[r] = true;
                true
            }
            "compare_bytes" if args.len() == 3 => {
                let arg_ok = |arg| {
                    !is_byte_view_candidate(self.func, self.table, arg)
                        || self
                            .func
                            .ty(arg)
                            .is_some_and(|t| is_immutable_byte_ptr(self.table, t))
                };
                arg_ok(args[0]) && arg_ok(args[1])
            }
            "raw_eq"
                if args.len() == 2
                    && types.first().is_some_and(|pointee| {
                        matches!(
                            value::kind(self.table, self.table.peeled(*pointee)),
                            Some(TypeKind::Array { element, .. })
                                if value::is_u8(self.table, *element)
                        )
                    }) =>
            {
                let arg_ok = |arg| {
                    !is_byte_view_candidate(self.func, self.table, arg)
                        || self
                            .func
                            .ty(arg)
                            .is_some_and(|t| is_immutable_byte_array_ptr(self.table, t).is_some())
                };
                arg_ok(args[0]) && arg_ok(args[1])
            }
            "copy_nonoverlapping" | "copy"
                if args.len() == 3
                    && types.first().is_some_and(|t| value::is_u8(self.table, *t))
                    && self
                        .func
                        .ty(args[0])
                        .is_some_and(|lt| is_immutable_byte_ptr(self.table, lt)) =>
            {
                if is_byte_view_candidate(self.func, self.table, args[1]) {
                    self.poisoned[args[1].0 as usize] = true;
                }
                true
            }
            _ => false,
        }
    }

    fn union_call_op(&mut self, op: &Op, r: usize, res_kind: Option<ByteKind>) -> bool {
        if let OpKind::Call {
            semantic: CallSemantic::Standard(StandardOperation::SliceIsAscii),
            args,
            ..
        } = &op.kind
        {
            if args.len() == 1
                && self
                    .func
                    .ty(args[0])
                    .is_some_and(|ty| is_immutable_byte_slice(self.table, ty))
            {
                return true;
            }
        }
        let OpKind::Call {
            callee: Callee::Direct { .. },
            host: None,
            args,
            ..
        } = &op.kind
        else {
            return false;
        };
        let any_accepted = args.iter().enumerate().any(|(arg_idx, &arg)| {
            (self.call_accepts)(op, arg_idx).is_some()
                && self
                    .func
                    .ty(arg)
                    .is_some_and(|ty| is_immutable_byte_slice(self.table, ty))
        });
        if !any_accepted {
            return false;
        }
        for (arg_idx, &arg) in args.iter().enumerate() {
            if is_byte_view_candidate(self.func, self.table, arg) {
                let accepted = (self.call_accepts)(op, arg_idx).is_some()
                    && self
                        .func
                        .ty(arg)
                        .is_some_and(|ty| is_immutable_byte_slice(self.table, ty));
                if !accepted {
                    self.poisoned[arg.0 as usize] = true;
                }
            }
        }
        if res_kind.is_some() {
            self.poisoned[r] = true;
        }
        true
    }

    fn union_term_edges(&mut self, term: &RTerm) {
        for (target, args) in term.edges() {
            let Some(&bi) = self.block_index.get(&target) else {
                for &arg in args {
                    if is_byte_view_candidate(self.func, self.table, arg) {
                        self.poisoned[arg.0 as usize] = true;
                    }
                }
                continue;
            };
            for (pos, &arg) in args.iter().enumerate() {
                let Some(&param) = self.func.blocks[bi].params.get(pos) else {
                    if is_byte_view_candidate(self.func, self.table, arg) {
                        self.poisoned[arg.0 as usize] = true;
                    }
                    continue;
                };
                self.connect_edge(arg, param);
            }
        }
        match term {
            RTerm::Switch { scrut, .. } => {
                if is_byte_view_candidate(self.func, self.table, *scrut) {
                    self.poisoned[scrut.0 as usize] = true;
                }
            }
            RTerm::Return(Some(v)) | RTerm::Throw(Some(v)) => {
                if is_byte_view_candidate(self.func, self.table, *v) {
                    self.poisoned[v.0 as usize] = true;
                }
            }
            RTerm::Goto(..)
            | RTerm::Invoke { .. }
            | RTerm::Return(None)
            | RTerm::Throw(None)
            | RTerm::Unreachable => {}
        }
    }

    fn connect_edge(&mut self, arg: ValueId, param: ValueId) {
        let a = arg.0 as usize;
        let p = param.0 as usize;
        let ak = self.func.ty(arg).and_then(|ty| classify_ty(self.table, ty));
        let pk = self
            .func
            .ty(param)
            .and_then(|ty| classify_ty(self.table, ty));
        if ak.is_none() && pk.is_none() {
            return;
        }
        self.param_has_pred[p] = true;
        match (ak, pk) {
            (Some(ak), Some(pk)) if ak == pk => self.union_same_kind(a, p, ak),
            _ => {
                self.poisoned[a] = true;
                self.poisoned[p] = true;
            }
        }
    }

    fn collect_components(&mut self) -> BTreeMap<usize, Vec<usize>> {
        let mut comps: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (i, root) in self.roots.iter().enumerate() {
            if root.is_some() {
                comps.entry(self.uf.find(i)).or_default();
            }
        }
        if comps.is_empty() {
            return comps;
        }
        for i in 0..self.func.values.len() {
            let comp = self.uf.find(i);
            if let Some(members) = comps.get_mut(&comp) {
                members.push(i);
            }
        }
        comps
    }

    /// Propagates per-value base-slice provenance across both `Slice` and `ThinPtr` definitions
    /// so multi-root `VarSlice` components do not need a single component-wide base string when
    /// every `ThinPtr` traces back to a dominating non-inlined base slice or `&str`.
    fn propagate_base_slices(&mut self) {
        let n = self.func.values.len();
        let mut bs_state = vec![BaseSliceState::Unvisited; n];
        let orig_direct_bs = self.borrow_slice.clone();
        for block in &self.func.blocks {
            for &param in &block.params {
                if self
                    .func
                    .ty(param)
                    .and_then(|ty| classify_ty(self.table, ty))
                    == Some(ByteKind::Slice)
                {
                    bs_state[param.0 as usize] = BaseSliceState::Unique(param);
                }
            }
        }
        for (i, root) in self.roots.iter().enumerate() {
            match root {
                Some(ByteViewRoot {
                    base,
                    is_str_obj: false,
                    kind: ByteKind::Slice,
                    ..
                }) => {
                    let chosen = if *base == ValueId(i as u32) {
                        ValueId(i as u32)
                    } else {
                        *base
                    };
                    bs_state[i] = BaseSliceState::Unique(chosen);
                }
                Some(_) => {
                    bs_state[i] = BaseSliceState::Unique(ValueId(i as u32));
                }
                None => {}
            }
        }

        let mut cell_bs = vec![BaseSliceState::Unvisited; self.func.cells.len()];
        loop {
            let mut changed = false;
            for block in &self.func.blocks {
                for op in &block.ops {
                    changed |= self.propagate_base_slice_op(op, &mut bs_state, &mut cell_bs);
                }
                for (target, args) in block.term.edges() {
                    changed |= self.propagate_base_slice_edge(target, args, &mut bs_state);
                }
            }
            if !changed {
                break;
            }
        }

        for i in 0..n {
            if self
                .func
                .ty(ValueId(i as u32))
                .and_then(|ty| classify_ty(self.table, ty))
                != Some(ByteKind::ThinPtr)
            {
                continue;
            }
            // Preserve direct `BorrowView([Deref, Index])` slice when it is reusable, else
            // fall back to the peeled dominating ancestor slice `bs_state[i]`.
            let direct_ok = orig_direct_bs[i]
                .is_some_and(|bs| self.plan.is_none_or(|p| !p.inline.contains(&bs)));
            if !direct_ok {
                if let BaseSliceState::Unique(bs) = bs_state[i] {
                    self.borrow_slice[i] = Some(bs);
                }
            }
        }
    }

    fn propagate_base_slice_op(
        &self,
        op: &Op,
        bs_state: &mut [BaseSliceState],
        cell_bs: &mut [BaseSliceState],
    ) -> bool {
        if let OpKind::CellStore { cell, v } = &op.kind {
            let ci = cell.0 as usize;
            return self.cell_ok.get(ci).copied().unwrap_or(false)
                && cell_bs[ci].join(bs_state[v.0 as usize]);
        }
        let Some(res) = op.result else { return false };
        let r = res.0 as usize;
        let Some(rk) = self.func.ty(res).and_then(|ty| classify_ty(self.table, ty)) else {
            return false;
        };
        if self.roots[r].is_some() {
            return false;
        }
        match &op.kind {
            OpKind::Cast { v: src, from, .. } => {
                if is_immutable_str_ref(self.table, *from) {
                    bs_state[r].join_value(*src)
                } else if classify_ty(self.table, *from).is_some() {
                    let inc = bs_state[src.0 as usize];
                    bs_state[r].join(inc)
                } else {
                    false
                }
            }
            OpKind::CellLoad(cell) => {
                let ci = cell.0 as usize;
                self.cell_ok.get(ci).copied().unwrap_or(false) && bs_state[r].join(cell_bs[ci])
            }
            OpKind::Deref(ptr) => {
                if let Some(ci) = self.ref_to_cell[ptr.0 as usize] {
                    self.cell_ok.get(ci).copied().unwrap_or(false) && bs_state[r].join(cell_bs[ci])
                } else if let Some(src) = self.ref_to_val[ptr.0 as usize] {
                    if !self.ref_to_val_escaped[ptr.0 as usize] {
                        let inc = bs_state[src.0 as usize];
                        bs_state[r].join(inc)
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            OpKind::Copy(src)
            | OpKind::Move(src)
            | OpKind::SliceView { ptr: src, .. }
            | OpKind::Binary {
                op: BinaryOp::Offset,
                l: src,
                ..
            } => {
                let inc = bs_state[src.0 as usize];
                bs_state[r].join(inc)
            }
            OpKind::BorrowView {
                root: Root::Value(src),
                path,
                mutable: false,
            }
            | OpKind::RawView {
                root: Root::Value(src),
                path,
                mutable: false,
            } => {
                let valid_path = matches!(
                    path.as_slice(),
                    [Proj::Deref]
                        | [Proj::Deref, Proj::Index(_) | Proj::ConstantIndex { .. }]
                        | [Proj::Deref, Proj::Subslice { .. }]
                );
                if valid_path
                    && self
                        .func
                        .ty(*src)
                        .and_then(|ty| classify_ty(self.table, ty))
                        .is_some()
                {
                    let inc = bs_state[src.0 as usize];
                    bs_state[r].join(inc)
                } else {
                    false
                }
            }
            OpKind::Intrinsic { name, args, .. }
                if matches!(
                    self.interns.str(*name).unwrap_or(""),
                    "offset"
                        | "arith_offset"
                        | "slice_get_unchecked"
                        | "transmute"
                        | "transmute_unchecked"
                ) && !args.is_empty() =>
            {
                let src = args[0];
                if self
                    .func
                    .ty(src)
                    .and_then(|ty| classify_ty(self.table, ty))
                    .is_some()
                {
                    let inc = bs_state[src.0 as usize];
                    bs_state[r].join(inc)
                } else {
                    false
                }
            }
            OpKind::Aggregate {
                ty,
                variant,
                active_field,
                fields,
            } => {
                let src = if fields.len() == 2
                    && matches!(
                        self.table.get(*ty).map(|d| &d.kind),
                        Some(TypeKind::RawPointer { .. })
                    )
                    && rk == ByteKind::Slice
                {
                    Some(fields[0])
                } else if let Some((_, some_v, fidx, _, _)) =
                    null_niche_byte_payload(self.table, *ty)
                {
                    (*variant == some_v)
                        .then(|| fields.get(fidx as usize).copied())
                        .flatten()
                } else {
                    transparent_aggregate_field(self.table, *ty, *active_field, fields)
                };
                if let Some(src) = src {
                    let inc = bs_state[src.0 as usize];
                    bs_state[r].join(inc)
                } else {
                    false
                }
            }
            OpKind::Field {
                base,
                variant,
                field,
            } => {
                let bty = self.func.ty(*base);
                let is_niche_ok = bty
                    .and_then(|t| null_niche_byte_payload(self.table, t))
                    .is_some_and(|(_, some_v, fidx, _, _)| {
                        *variant == Some(some_v) && *field == fidx
                    });
                let is_trans_ok = bty
                    .and_then(|t| self.table.get(t))
                    .and_then(|d| d.js.transparent)
                    .is_some_and(|inner| inner.field == *field);
                if is_niche_ok || is_trans_ok {
                    let inc = bs_state[base.0 as usize];
                    return bs_state[r].join(inc);
                }
                if variant.is_none() {
                    if let Some(orig) =
                        self.agg_origin[base.0 as usize].or(self.agg_ref[base.0 as usize])
                    {
                        if !self.agg_escaped[orig] {
                            if let Some(&fval) = self.agg_fields[orig]
                                .as_ref()
                                .and_then(|fs| fs.get(*field as usize))
                            {
                                let inc = bs_state[fval.0 as usize];
                                return bs_state[r].join(inc);
                            }
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }

    fn propagate_base_slice_edge(
        &self,
        target: RBlockId,
        args: &[ValueId],
        bs_state: &mut [BaseSliceState],
    ) -> bool {
        let Some(&bi) = self.block_index.get(&target) else {
            return false;
        };
        let mut changed = false;
        for (pos, &arg) in args.iter().enumerate() {
            let Some(&param) = self.func.blocks[bi].params.get(pos) else {
                continue;
            };
            let Some(pk) = self
                .func
                .ty(param)
                .and_then(|ty| classify_ty(self.table, ty))
            else {
                continue;
            };
            if pk == ByteKind::Slice {
                // A slice phi merging multiple roots is itself a dynamic VarSlice carrying
                // `{ buf, off, len }`, so downstream ThinPtrs can read `.buf` directly from
                // `param` (which is never in `plan.inline`).
                changed |= bs_state[param.0 as usize].join_value(param);
            } else {
                let inc = bs_state[arg.0 as usize];
                changed |= bs_state[param.0 as usize].join(inc);
            }
        }
        changed
    }

    fn materialize_byte_views(
        &mut self,
        comps: BTreeMap<usize, Vec<usize>>,
    ) -> BTreeMap<ValueId, ByteView> {
        let mut var_slice_roots = BTreeSet::new();
        for (i, &is_seed) in self.var_slice_seed.iter().enumerate() {
            if is_seed {
                var_slice_roots.insert(self.slice_uf.find(i));
            }
        }

        let doms = Dominators::of(self.func);
        let mut out = BTreeMap::new();

        for members in comps.into_values() {
            let mut disqualified = false;
            let mut comp_roots: BTreeMap<(ValueId, bool), Option<ValueId>> = BTreeMap::new();
            let mut needs_single_base = false;
            let mut has_var_slice = false;

            for &v in &members {
                if self.poisoned[v] {
                    disqualified = true;
                    break;
                }
                let ok_def =
                    self.valid_op_def[v] || (self.is_block_param[v] && self.param_has_pred[v]);
                if !ok_def {
                    disqualified = true;
                    break;
                }
                let Some(kind) = self
                    .func
                    .ty(ValueId(v as u32))
                    .and_then(|ty| classify_ty(self.table, ty))
                else {
                    disqualified = true;
                    break;
                };
                if let Some(root) = self.roots[v] {
                    comp_roots
                        .entry((root.base, root.is_str_obj))
                        .and_modify(|prev| {
                            if *prev != root.seed_op {
                                *prev = None;
                            }
                        })
                        .or_insert(root.seed_op);
                }
                if kind == ByteKind::Slice && var_slice_roots.contains(&self.slice_uf.find(v)) {
                    has_var_slice = true;
                }
                if kind == ByteKind::ThinPtr && !self.thin_ptr_has_dominating_slice(&doms, v) {
                    needs_single_base = true;
                }
            }
            if disqualified {
                continue;
            }

            if has_var_slice && !needs_single_base {
                let all_roots_reusable = comp_roots.keys().all(|&(raw_base, _)| {
                    is_reusable_base_str(self.func, self.plan, raw_base).is_some()
                });
                if !all_roots_reusable {
                    continue;
                }
            }

            let single_base = self.find_single_base(&doms, &members, &comp_roots);
            if needs_single_base && single_base.is_none() {
                continue;
            }

            for &v in &members {
                let kind = self
                    .func
                    .ty(ValueId(v as u32))
                    .and_then(|ty| classify_ty(self.table, ty))
                    .unwrap();
                let mode = match kind {
                    ByteKind::Slice => {
                        if var_slice_roots.contains(&self.slice_uf.find(v)) {
                            ByteViewMode::VarSlice
                        } else {
                            ByteViewMode::FullSlice
                        }
                    }
                    ByteKind::ThinPtr => ByteViewMode::ThinPtr,
                };
                out.insert(
                    ValueId(v as u32),
                    ByteView {
                        mode,
                        base_str: single_base.map(|(b, _)| b),
                        base_is_string_obj: single_base.is_some_and(|(_, obj)| obj),
                        borrow_slice: self.borrow_slice[v],
                    },
                );
            }
        }

        out
    }

    fn thin_ptr_has_dominating_slice(&self, doms: &Dominators, v: usize) -> bool {
        self.borrow_slice[v].is_some_and(|bs| {
            is_reusable_base_str(self.func, self.plan, bs).is_some()
                && self.def_block[bs.0 as usize].is_some_and(|bs_blk| {
                    self.def_block[v].is_none_or(|d_blk| doms.dominates(bs_blk, d_blk))
                        && self.use_blocks[v]
                            .iter()
                            .all(|&u_blk| doms.dominates(bs_blk, u_blk))
                })
        })
    }

    fn find_single_base(
        &self,
        doms: &Dominators,
        members: &[usize],
        comp_roots: &BTreeMap<(ValueId, bool), Option<ValueId>>,
    ) -> Option<(ValueId, bool)> {
        if comp_roots.len() != 1 {
            return None;
        }
        let (&(raw_base, is_str_obj), _) = comp_roots.iter().next()?;
        let base = is_reusable_base_str(self.func, self.plan, raw_base)?;
        let base_blk = self.def_block[base.0 as usize]?;
        for &v in members {
            if let Some(d_blk) = self.def_block[v] {
                if !doms.dominates(base_blk, d_blk) {
                    return None;
                }
            }
            for &u_blk in &self.use_blocks[v] {
                if !doms.dominates(base_blk, u_blk) {
                    return None;
                }
            }
        }
        Some((base, is_str_obj))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::plan::NO_ORIGIN;
    use rcgjs_object::jir::interns::Interns;
    use rcgjs_object::jir::repr::{FuncBuilder, OpKind, Proj, RTerm, Root};
    use rcgjs_object::jir::semantic::{
        AbiSignature, BinaryOp, CallSemantic, Callee, CastKind, ConstTree, DefinitionId, ItemId,
        Signature, UnaryOp,
    };
    use rcgjs_object::jir::types::{
        IntFacts, IntRepr, JsFacts, LayoutFacts, PointerForm, TypeDescriptor, TypeId, TypeKind,
        TypeMeta,
    };
    use rcgjs_object::opts::Opts;

    const UNIT: TypeId = TypeId(1);
    const BOOL: TypeId = TypeId(2);
    const U8: TypeId = TypeId(3);
    const USIZE: TypeId = TypeId(4);
    const ISIZE: TypeId = TypeId(5);
    const STR: TypeId = TypeId(6);
    const STR_REF: TypeId = TypeId(7);
    const SLICE_U8: TypeId = TypeId(8);
    const REF_SLICE_U8: TypeId = TypeId(9);
    const PTR_U8: TypeId = TypeId(10);
    const REF_U8: TypeId = TypeId(11);
    const PAIR: TypeId = TypeId(12);
    const I32: TypeId = TypeId(13);
    const PTR_I32: TypeId = TypeId(14);

    fn desc(id: TypeId, kind: TypeKind, size: u64) -> TypeDescriptor {
        let sized = size > 0 || !matches!(kind, TypeKind::Str | TypeKind::Slice(_));
        let zst = size == 0 && !matches!(kind, TypeKind::Str | TypeKind::Slice(_));
        TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                size: Some(size),
                unsized_prefix: None,
                align: Some(size.clamp(1, 4)),
                sized,
                zst,
                one_zst: zst,
                needs_drop: false,
                has_own_drop: false,
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: id,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        }
    }

    fn int_desc(id: TypeId, signed: bool, bits: u32) -> TypeDescriptor {
        let mut d = desc(id, TypeKind::Integer { signed, bits }, u64::from(bits / 8));
        d.js.int = Some(IntFacts {
            signed,
            bits,
            repr: IntRepr::Number,
        });
        d.js.byte_element = bits == 8;
        d
    }

    fn ptr_desc(id: TypeId, pointee: TypeId, form: PointerForm, raw: bool) -> TypeDescriptor {
        let kind = if raw {
            TypeKind::RawPointer {
                pointee,
                mutable: false,
            }
        } else {
            TypeKind::Reference {
                pointee,
                mutable: false,
            }
        };
        let mut d = desc(id, kind, 4);
        d.js.pointer_form = Some(form);
        d
    }

    fn test_interns() -> Interns {
        let mut interns = Interns::new();
        interns.intern_type(desc(UNIT, TypeKind::Tuple(Vec::new()), 0));
        interns.intern_type(desc(BOOL, TypeKind::Bool, 1));
        interns.intern_type(int_desc(U8, false, 8));
        interns.intern_type(int_desc(USIZE, false, 32));
        interns.intern_type(int_desc(ISIZE, true, 32));
        interns.intern_type(int_desc(I32, true, 32));
        interns.intern_type(desc(STR, TypeKind::Str, 0));
        interns.intern_type(ptr_desc(STR_REF, STR, PointerForm::Str, false));
        interns.intern_type(desc(SLICE_U8, TypeKind::Slice(U8), 0));
        interns.intern_type(ptr_desc(
            REF_SLICE_U8,
            SLICE_U8,
            PointerForm::FatSlice,
            false,
        ));
        interns.intern_type(ptr_desc(PTR_U8, U8, PointerForm::Slot, true));
        interns.intern_type(ptr_desc(REF_U8, U8, PointerForm::Slot, false));
        interns.intern_type(ptr_desc(PTR_I32, I32, PointerForm::Slot, true));
        interns.intern_type(desc(PAIR, TypeKind::Tuple(vec![PTR_U8, USIZE]), 8));
        interns
    }

    fn sig(arguments: Vec<TypeId>, result: TypeId) -> Signature {
        Signature {
            abi: AbiSignature {
                parameters: arguments.len() as u16,
                spread: None,
                caller_location: false,
                inline_never: false,
                inline_hint: false,
                inline_always: false,
            },
            arguments,
            result,
        }
    }

    fn dummy_item(n: u64) -> ItemId {
        ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: n,
            },
            instantiation: n as u128,
        }
    }

    fn run_analyze(interns: &Interns, func: &Func) -> BTreeMap<ValueId, ByteView> {
        let plan = crate::lower::fuse::plan(func, None, &interns.types, interns, &Opts::default());
        analyze(func, &plan, &interns.types, interns)
    }

    #[test]
    fn byte_view_slice_across_goto_loop_and_subslice() {
        let interns = test_interns();
        let mut b = FuncBuilder::new(dummy_item(1), sig(vec![STR_REF, USIZE], U8));
        let s = b.param(STR_REF, NO_ORIGIN);
        let idx = b.param(USIZE, NO_ORIGIN);
        let e = FuncBuilder::ENTRY;
        let bytes = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: s,
                from: STR_REF,
                to: REF_SLICE_U8,
            },
            REF_SLICE_U8,
            NO_ORIGIN,
        );
        let sub = b.op(
            e,
            OpKind::BorrowView {
                root: Root::Value(bytes),
                path: vec![
                    Proj::Deref,
                    Proj::Subslice {
                        from: 1,
                        to: 0,
                        from_end: true,
                    },
                ],
                mutable: false,
            },
            REF_SLICE_U8,
            NO_ORIGIN,
        );
        let hdr = b.block();
        let lp = b.block_param(hdr, REF_SLICE_U8, NO_ORIGIN);
        b.terminate(e, RTerm::Goto(hdr, vec![sub]));
        let elem_ref = b.op(
            hdr,
            OpKind::BorrowView {
                root: Root::Value(lp),
                path: vec![Proj::Deref, Proj::Index(idx)],
                mutable: false,
            },
            REF_U8,
            NO_ORIGIN,
        );
        let ch = b.op(hdr, OpKind::Deref(elem_ref), U8, NO_ORIGIN);
        b.terminate(hdr, RTerm::Return(Some(ch)));
        let func = b.finish();

        let map = run_analyze(&interns, &func);
        assert_eq!(map[&bytes].mode, ByteViewMode::FullSlice);
        assert_eq!(map[&sub].mode, ByteViewMode::VarSlice);
        assert_eq!(map[&lp].mode, ByteViewMode::VarSlice);
        assert_eq!(map[&elem_ref].mode, ByteViewMode::ThinPtr);
        assert_eq!(map[&elem_ref].borrow_slice, Some(lp));
        assert_eq!(map[&bytes].base_str, Some(s));

        // Merging full-slice and subslice into the same block param promotes all to VarSlice.
        let mut bm = FuncBuilder::new(dummy_item(10), sig(vec![STR_REF, BOOL, USIZE], U8));
        let sm = bm.param(STR_REF, NO_ORIGIN);
        let flag = bm.param(BOOL, NO_ORIGIN);
        let idxm = bm.param(USIZE, NO_ORIGIN);
        let em = FuncBuilder::ENTRY;
        let fullm = bm.op(
            em,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: sm,
                from: STR_REF,
                to: REF_SLICE_U8,
            },
            REF_SLICE_U8,
            NO_ORIGIN,
        );
        let subm = bm.op(
            em,
            OpKind::BorrowView {
                root: Root::Value(fullm),
                path: vec![
                    Proj::Deref,
                    Proj::Subslice {
                        from: 1,
                        to: 0,
                        from_end: true,
                    },
                ],
                mutable: false,
            },
            REF_SLICE_U8,
            NO_ORIGIN,
        );
        let hdrm = bm.block();
        let lpm = bm.block_param(hdrm, REF_SLICE_U8, NO_ORIGIN);
        bm.terminate(
            em,
            RTerm::Switch {
                scrut: flag,
                scrut_ty: BOOL,
                cases: vec![(0, hdrm, vec![fullm])],
                default: (hdrm, vec![subm]),
            },
        );
        let eref = bm.op(
            hdrm,
            OpKind::BorrowView {
                root: Root::Value(lpm),
                path: vec![Proj::Deref, Proj::Index(idxm)],
                mutable: false,
            },
            REF_U8,
            NO_ORIGIN,
        );
        let chm = bm.op(hdrm, OpKind::Deref(eref), U8, NO_ORIGIN);
        bm.terminate(hdrm, RTerm::Return(Some(chm)));
        let mfunc = bm.finish();
        let mmap = run_analyze(&interns, &mfunc);
        assert_eq!(mmap[&fullm].mode, ByteViewMode::VarSlice);
        assert_eq!(mmap[&subm].mode, ByteViewMode::VarSlice);
        assert_eq!(mmap[&lpm].mode, ByteViewMode::VarSlice);
    }

    #[test]
    fn byte_view_raw_pointer_loop_compare_and_diff() {
        let mut interns = test_interns();
        let ptr_offset_from = interns.intern_str("ptr_offset_from");
        let mut b = FuncBuilder::new(dummy_item(2), sig(vec![STR_REF], ISIZE));
        let s = b.param(STR_REF, NO_ORIGIN);
        let e = FuncBuilder::ENTRY;
        let bytes = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: s,
                from: STR_REF,
                to: REF_SLICE_U8,
            },
            REF_SLICE_U8,
            NO_ORIGIN,
        );
        let len = b.op(
            e,
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v: bytes,
                ty: REF_SLICE_U8,
            },
            USIZE,
            NO_ORIGIN,
        );
        let start = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: bytes,
                from: REF_SLICE_U8,
                to: PTR_U8,
            },
            PTR_U8,
            NO_ORIGIN,
        );
        let end = b.op(
            e,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: start,
                r: len,
                ty: PTR_U8,
            },
            PTR_U8,
            NO_ORIGIN,
        );
        let hdr = b.block();
        let cur = b.block_param(hdr, PTR_U8, NO_ORIGIN);
        b.terminate(e, RTerm::Goto(hdr, vec![start]));
        let cond = b.op(
            hdr,
            OpKind::Binary {
                op: BinaryOp::Lt,
                l: cur,
                r: end,
                ty: PTR_U8,
            },
            BOOL,
            NO_ORIGIN,
        );
        let step_blk = b.block();
        let exit_blk = b.block();
        b.terminate(
            hdr,
            RTerm::Switch {
                scrut: cond,
                scrut_ty: BOOL,
                cases: vec![(0, exit_blk, Vec::new())],
                default: (step_blk, Vec::new()),
            },
        );
        let _val = b.op(step_blk, OpKind::Deref(cur), U8, NO_ORIGIN);
        let one = b.op(
            step_blk,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            ISIZE,
            NO_ORIGIN,
        );
        let next = b.op(
            step_blk,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: cur,
                r: one,
                ty: PTR_U8,
            },
            PTR_U8,
            NO_ORIGIN,
        );
        b.terminate(step_blk, RTerm::Goto(hdr, vec![next]));
        let diff = b.op(
            exit_blk,
            OpKind::Intrinsic {
                name: ptr_offset_from,
                types: vec![U8],
                fallback: None,
                args: vec![cur, start],
                track_caller: None,
            },
            ISIZE,
            NO_ORIGIN,
        );
        b.terminate(exit_blk, RTerm::Return(Some(diff)));
        let func = b.finish();

        let map = run_analyze(&interns, &func);
        for v in [bytes, start, end, cur, next] {
            assert_eq!(map[&v].base_str, Some(s), "missing base_str for {v:?}");
        }
        assert_eq!(map[&bytes].mode, ByteViewMode::FullSlice);
        assert_eq!(map[&cur].mode, ByteViewMode::ThinPtr);
    }

    #[test]
    fn byte_view_fails_closed_on_escapes_and_mixed_bases() {
        let mut interns = test_interns();
        let cmp_bytes = interns.intern_str("compare_bytes");
        let unknown_fn = interns.intern_str("other_fn");

        // 1. Escape to call poisons the whole component.
        {
            let mut b = FuncBuilder::new(dummy_item(3), sig(vec![STR_REF], PTR_U8));
            let s = b.param(STR_REF, NO_ORIGIN);
            let e = FuncBuilder::ENTRY;
            let bytes = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::Transmute,
                    v: s,
                    from: STR_REF,
                    to: REF_SLICE_U8,
                },
                REF_SLICE_U8,
                NO_ORIGIN,
            );
            let ptr = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::PtrToPtr,
                    v: bytes,
                    from: REF_SLICE_U8,
                    to: PTR_U8,
                },
                PTR_U8,
                NO_ORIGIN,
            );
            let _ = b.op(
                e,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: dummy_item(99),
                        link_name: unknown_fn,
                    },
                    function: None,
                    args: vec![bytes],
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                UNIT,
                NO_ORIGIN,
            );
            b.terminate(e, RTerm::Return(Some(ptr)));
            let func = b.finish();
            assert!(run_analyze(&interns, &func).is_empty());
        }

        // 2. Expose provenance (`as usize`) poisons the whole component.
        {
            let mut b = FuncBuilder::new(dummy_item(4), sig(vec![STR_REF], USIZE));
            let s = b.param(STR_REF, NO_ORIGIN);
            let e = FuncBuilder::ENTRY;
            let bytes = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::Transmute,
                    v: s,
                    from: STR_REF,
                    to: REF_SLICE_U8,
                },
                REF_SLICE_U8,
                NO_ORIGIN,
            );
            let ptr = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::PtrToPtr,
                    v: bytes,
                    from: REF_SLICE_U8,
                    to: PTR_U8,
                },
                PTR_U8,
                NO_ORIGIN,
            );
            let addr = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::PointerExposeProvenance,
                    v: ptr,
                    from: PTR_U8,
                    to: USIZE,
                },
                USIZE,
                NO_ORIGIN,
            );
            b.terminate(e, RTerm::Return(Some(addr)));
            let func = b.finish();
            assert!(run_analyze(&interns, &func).is_empty());
        }

        // 3. `compare_bytes` intrinsic on immutable byte-view pointers is admitted (lowers to
        // `bstr_cmp_range` / `bstr_cmp_ptr` / `ptr_cmp_bstr` without unioning operands).
        {
            let mut b = FuncBuilder::new(dummy_item(5), sig(vec![STR_REF], I32));
            let s = b.param(STR_REF, NO_ORIGIN);
            let e = FuncBuilder::ENTRY;
            let bytes = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::Transmute,
                    v: s,
                    from: STR_REF,
                    to: REF_SLICE_U8,
                },
                REF_SLICE_U8,
                NO_ORIGIN,
            );
            let ptr = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::PtrToPtr,
                    v: bytes,
                    from: REF_SLICE_U8,
                    to: PTR_U8,
                },
                PTR_U8,
                NO_ORIGIN,
            );
            let zero = b.op(
                e,
                OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
                USIZE,
                NO_ORIGIN,
            );
            let cmp = b.op(
                e,
                OpKind::Intrinsic {
                    name: cmp_bytes,
                    types: Vec::new(),
                    fallback: None,
                    args: vec![ptr, ptr, zero],
                    track_caller: None,
                },
                I32,
                NO_ORIGIN,
            );
            b.terminate(e, RTerm::Return(Some(cmp)));
            let func = b.finish();
            let views = run_analyze(&interns, &func);
            assert_eq!(
                views.get(&bytes).map(|v| v.mode),
                Some(ByteViewMode::FullSlice)
            );
            assert_eq!(views.get(&ptr).map(|v| v.mode), Some(ByteViewMode::ThinPtr));
        }

        // 4. Comparing thin pointers from different `base_str`s poisons both.
        {
            let mut b = FuncBuilder::new(dummy_item(6), sig(vec![STR_REF, STR_REF], BOOL));
            let s1 = b.param(STR_REF, NO_ORIGIN);
            let s2 = b.param(STR_REF, NO_ORIGIN);
            let e = FuncBuilder::ENTRY;
            let b1 = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::Transmute,
                    v: s1,
                    from: STR_REF,
                    to: PTR_U8,
                },
                PTR_U8,
                NO_ORIGIN,
            );
            let b2 = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::Transmute,
                    v: s2,
                    from: STR_REF,
                    to: PTR_U8,
                },
                PTR_U8,
                NO_ORIGIN,
            );
            let eq = b.op(
                e,
                OpKind::Binary {
                    op: BinaryOp::Eq,
                    l: b1,
                    r: b2,
                    ty: PTR_U8,
                },
                BOOL,
                NO_ORIGIN,
            );
            b.terminate(e, RTerm::Return(Some(eq)));
            let func = b.finish();
            assert!(run_analyze(&interns, &func).is_empty());
        }

        // 5. Casting `*const u8` -> `*const i32` and dereferencing poisons component.
        {
            let mut b = FuncBuilder::new(dummy_item(7), sig(vec![STR_REF], I32));
            let s = b.param(STR_REF, NO_ORIGIN);
            let e = FuncBuilder::ENTRY;
            let p_u8 = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::Transmute,
                    v: s,
                    from: STR_REF,
                    to: PTR_U8,
                },
                PTR_U8,
                NO_ORIGIN,
            );
            let p_i32 = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::PtrToPtr,
                    v: p_u8,
                    from: PTR_U8,
                    to: PTR_I32,
                },
                PTR_I32,
                NO_ORIGIN,
            );
            let val = b.op(e, OpKind::Deref(p_i32), I32, NO_ORIGIN);
            b.terminate(e, RTerm::Return(Some(val)));
            let func = b.finish();
            assert!(run_analyze(&interns, &func).is_empty());
        }
    }
}
