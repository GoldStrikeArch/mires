//! Whole-program monotone-arena provenance propagation and cursor greatest-fixpoint solver.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::arena::{ArenaFieldKind, ArenaHandle, ProgramBoundsFacts, ReturnVecElemBound};
use crate::analysis::heap::{global_vec_elem_ty, is_direct_global_vec_ty, is_vec_len_field};
use crate::jir::interns::Interns;
use crate::jir::repr::{CellId, Func, Op, OpKind, Proj, RBlockId, Root, ValueId};
use crate::jir::semantic::{
    CallSemantic, Callee, CastKind, ConstTree, DefinitionId, ItemId, StandardOperation, TypeId,
    UnaryOp,
};
use crate::jir::types::TypeKind;
use crate::jir::TypeTable;

use super::super::cfg::{mask, Cfg};
use super::super::witness::ArenaIndexSource;
use super::{width_max, Analysis, ArenaValOrigin};

struct ArenaPropCx<'a> {
    func: &'a Func,
    table: &'a TypeTable,
    interns: Option<&'a Interns>,
    af: &'a ProgramBoundsFacts,
}

/// Unified provenance tag for SSA values referencing an arena, an element, a field, or an inner
/// `Vec` field slice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArenaRef {
    /// A `Vec<Elem>` or slice/reference to the arena itself.
    Arena {
        elem_def: DefinitionId,
        handle: ArenaHandle,
    },
    /// A reference to (or loaded copy of) an arena element `Elem`.
    Elem {
        elem_def: DefinitionId,
        handle: ArenaHandle,
    },
    /// A reference to field `field` of an arena element (or an element of its `Vec` field).
    Field {
        elem_def: DefinitionId,
        field: u32,
        handle: ArenaHandle,
    },
    /// A slice derived from a `VecOfValidIndices` field of an arena element.
    InnerVecSlice {
        elem_def: DefinitionId,
        field: u32,
        handle: ArenaHandle,
    },
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum HolderRoot {
    Param(usize),
    Cell(CellId),
}

impl HolderRoot {
    fn arena_handle(self, field: u32) -> ArenaHandle {
        match self {
            Self::Param(pi) => ArenaHandle::ParamHolder(pi, field),
            Self::Cell(c) => ArenaHandle::HolderCell(c, field),
        }
    }
}

struct ArenaOriginState {
    arena_refs: BTreeMap<ValueId, ArenaRef>,
    local_vec_cells: BTreeMap<CellId, DefinitionId>,
    holder_ref: BTreeMap<ValueId, HolderRoot>,
    ret_vec_bound_val: BTreeMap<ValueId, (ValueId, ItemId)>,
    ret_vec_bound_cell: BTreeMap<CellId, (ValueId, ItemId)>,
    ret_vec_bound_elem_ref: BTreeMap<ValueId, (ValueId, ItemId)>,
}

fn adt_def(table: &TypeTable, ty: TypeId) -> Option<DefinitionId> {
    match table.get(table.peeled(ty)).map(|d| &d.kind) {
        Some(TypeKind::Adt { def, .. }) => Some(*def),
        _ => None,
    }
}

fn slice_def(table: &TypeTable, ty: TypeId) -> Option<DefinitionId> {
    let pt = table.pointee(ty)?;
    match table.get(table.peeled(pt)).map(|d| &d.kind) {
        Some(TypeKind::Slice(e)) => adt_def(table, *e),
        _ => None,
    }
}

fn vec_def(table: &TypeTable, ty: TypeId) -> Option<DefinitionId> {
    let e = global_vec_elem_ty(table, ty)?;
    adt_def(table, e)
}

fn vec_ref_def(table: &TypeTable, ty: TypeId) -> Option<DefinitionId> {
    let pt = table.pointee(ty)?;
    vec_def(table, pt)
}

impl Analysis {
    /// Consumes whole-program [`ProgramBoundsFacts`] (`arenas`, `param_arena_indices`,
    /// `return_arena_index`, `return_vec_elem_bound`) to populate `arena_len_handle`,
    /// `arena_val_origin`, and `ret_vec_elem_len`.
    pub(super) fn compute_arena_origins(
        &mut self,
        func: &Func,
        table: &TypeTable,
        interns: Option<&Interns>,
        af: &ProgramBoundsFacts,
    ) {
        let mut state = self.seed_arena_roots(func, table, af);
        let incoming = Cfg::incoming_edges(func);
        self.propagate_arena_refs(func, table, interns, af, &incoming, &mut state);
        self.solve_arena_cursor_fixpoint(func, &incoming);
    }

    fn seed_arena_roots(
        &mut self,
        func: &Func,
        table: &TypeTable,
        af: &ProgramBoundsFacts,
    ) -> ArenaOriginState {
        let mut arena_refs: BTreeMap<ValueId, ArenaRef> = BTreeMap::new();
        let mut local_vec_cells: BTreeMap<CellId, DefinitionId> = BTreeMap::new();
        let mut holder_ref: BTreeMap<ValueId, HolderRoot> = BTreeMap::new();

        for (ci, cell) in func.cells.iter().enumerate() {
            let cid = CellId(ci as u32);
            if let Some(def) = vec_def(table, cell.ty) {
                if af.arenas.contains_key(&def) {
                    local_vec_cells.insert(cid, def);
                }
            }
        }
        for (pi, &pv) in func.params.iter().enumerate() {
            let pty = func.values[pv.0 as usize].ty;
            if let Some(def) = slice_def(table, pty)
                .or_else(|| vec_ref_def(table, pty))
                .or_else(|| vec_def(table, pty))
            {
                if af.arenas.contains_key(&def) {
                    arena_refs.insert(
                        pv,
                        ArenaRef::Arena {
                            elem_def: def,
                            handle: ArenaHandle::Param(pi),
                        },
                    );
                }
            }
            // Track holder struct references (`&Holder`, `&mut Holder`, `Holder`) through reborrows
            // & copies so inlined `&mut self` accesses like `(*reborrowed_self).nodes` resolve.
            let base = table.pointee(pty).unwrap_or(pty);
            if matches!(
                table.get(table.peeled(base)).map(|d| &d.kind),
                Some(TypeKind::Adt {
                    form: rcgjs_object::jir::types::NominalForm::Struct,
                    ..
                })
            ) {
                holder_ref.insert(pv, HolderRoot::Param(pi));
            }
        }
        // Seed certified `ParamArenaIndex` entry params.
        if let Some(plist) = af.param_arena_indices.get(&func.item) {
            for p in plist {
                if let Some(&pv) = func.params.get(p.idx_param) {
                    let handle = match p.holder_field {
                        Some(hf) => ArenaHandle::ParamHolder(p.arena_param, hf),
                        None => ArenaHandle::Param(p.arena_param),
                    };
                    self.arena_val_origin.insert(
                        self.canon(pv),
                        ArenaValOrigin {
                            elem_def: p.elem_def,
                            handle,
                            sentinel: p.sentinel,
                            source: ArenaIndexSource::Param {
                                idx_param: p.idx_param,
                            },
                        },
                    );
                }
            }
        }

        ArenaOriginState {
            arena_refs,
            local_vec_cells,
            holder_ref,
            ret_vec_bound_val: BTreeMap::new(),
            ret_vec_bound_cell: BTreeMap::new(),
            ret_vec_bound_elem_ref: BTreeMap::new(),
        }
    }

    fn propagate_block_param_map<T: Copy + Eq>(
        &self,
        params: &[ValueId],
        edges: &[(RBlockId, &[ValueId])],
        map: &mut BTreeMap<ValueId, T>,
        allow: impl Fn(T) -> bool,
    ) {
        for (pos, &param) in params.iter().enumerate() {
            if map.contains_key(&param) {
                continue;
            }
            let pc = self.canon(param);
            let mut first: Option<T> = None;
            let mut ok = true;
            for (_, args) in edges {
                let Some(&arg) = args.get(pos) else {
                    ok = false;
                    break;
                };
                let ac = self.canon(arg);
                if ac == pc {
                    continue;
                }
                if let Some(&val) = map
                    .get(&arg)
                    .or_else(|| map.get(&ac))
                    .filter(|&&val| allow(val))
                {
                    match first {
                        None => first = Some(val),
                        Some(prev) if prev == val => {}
                        Some(_) => {
                            ok = false;
                            break;
                        }
                    }
                } else {
                    ok = false;
                    break;
                }
            }
            if ok {
                if let Some(val) = first {
                    map.insert(param, val);
                    map.insert(pc, val);
                }
            }
        }
    }

    fn record_arena_len_handle(&mut self, len_c: ValueId, pair: (DefinitionId, ArenaHandle)) {
        let list = self.arena_len_handle.entry(len_c).or_default();
        if !list.contains(&pair) {
            list.push(pair);
        }
    }

    fn lookup_holder_ref_canon(&self, state: &ArenaOriginState, v: ValueId) -> Option<HolderRoot> {
        state
            .holder_ref
            .get(&v)
            .or_else(|| state.holder_ref.get(&self.canon(v)))
            .copied()
    }

    fn lookup_arena_ref_canon(&self, state: &ArenaOriginState, v: ValueId) -> Option<ArenaRef> {
        state
            .arena_refs
            .get(&v)
            .or_else(|| state.arena_refs.get(&self.canon(v)))
            .copied()
    }

    fn lookup_ret_vec_bound_canon(
        &self,
        state: &ArenaOriginState,
        v: ValueId,
    ) -> Option<(ValueId, ItemId)> {
        state
            .ret_vec_bound_val
            .get(&v)
            .or_else(|| state.ret_vec_bound_val.get(&self.canon(v)))
            .copied()
    }

    fn propagate_block_params(
        &self,
        func: &Func,
        b: RBlockId,
        incoming: &[Vec<(RBlockId, &[ValueId])>],
        state: &mut ArenaOriginState,
    ) {
        if b.0 == 0 {
            return;
        }
        let bi = b.0 as usize;
        let edges = &incoming[bi];
        if edges.is_empty() {
            return;
        }
        let params = &func.blocks[bi].params;
        self.propagate_block_param_map(params, edges, &mut state.holder_ref, |_| true);
        self.propagate_block_param_map(params, edges, &mut state.arena_refs, |r| {
            matches!(r, ArenaRef::Arena { .. })
        });
    }

    fn propagate_arena_refs(
        &mut self,
        func: &Func,
        table: &TypeTable,
        interns: Option<&Interns>,
        af: &ProgramBoundsFacts,
        incoming: &[Vec<(RBlockId, &[ValueId])>],
        state: &mut ArenaOriginState,
    ) {
        // Propagate arena handles, holder refs, slice lengths, element/field refs, and
        // `ReturnVecElemBound` in RPO (including across block parameters!).
        for _ in 0..3 {
            for &b in &self.doms.rpo.clone() {
                self.propagate_block_params(func, b, incoming, state);
                for op in &func.blocks[b.0 as usize].ops {
                    self.propagate_op_arena_ref(func, table, interns, af, op, state);
                }
            }
        }
    }

    fn propagate_op_arena_ref(
        &mut self,
        func: &Func,
        table: &TypeTable,
        interns: Option<&Interns>,
        af: &ProgramBoundsFacts,
        op: &Op,
        state: &mut ArenaOriginState,
    ) {
        if let OpKind::CellStore { cell, v } = &op.kind {
            if let Some(rb) = self.lookup_ret_vec_bound_canon(state, *v) {
                state.ret_vec_bound_cell.insert(*cell, rb);
            }
        }
        let Some(res) = op.result else { return };
        let res_c = self.canon(res);
        match &op.kind {
            OpKind::Copy(s) | OpKind::Move(s) => {
                if let Some(&hr) = state.holder_ref.get(s) {
                    state.holder_ref.insert(res, hr);
                }
                if let Some(&ar) = state.arena_refs.get(s) {
                    state.arena_refs.insert(res, ar);
                }
                if let Some(&rb) = state.ret_vec_bound_val.get(s) {
                    state.ret_vec_bound_val.insert(res, rb);
                }
                if let Some(&rb) = state.ret_vec_bound_elem_ref.get(s) {
                    state.ret_vec_bound_elem_ref.insert(res, rb);
                }
            }
            OpKind::Cast {
                kind: CastKind::Subtype | CastKind::PointerCoercion,
                v,
                ..
            } => {
                if let Some(&hr) = state.holder_ref.get(v) {
                    state.holder_ref.insert(res, hr);
                }
                if let Some(&ar) = state
                    .arena_refs
                    .get(v)
                    .filter(|r| !matches!(r, ArenaRef::InnerVecSlice { .. }))
                {
                    state.arena_refs.insert(res, ar);
                }
                if let Some(&rb) = state.ret_vec_bound_val.get(v) {
                    state.ret_vec_bound_val.insert(res, rb);
                }
            }
            OpKind::CellLoad(c) => {
                if let Some(&def) = state.local_vec_cells.get(c) {
                    state.arena_refs.insert(
                        res,
                        ArenaRef::Arena {
                            elem_def: def,
                            handle: ArenaHandle::VecCell(*c),
                        },
                    );
                }
                if let Some(&rb) = state.ret_vec_bound_cell.get(c) {
                    state.ret_vec_bound_val.insert(res, rb);
                }
            }
            OpKind::BorrowView { root, path, .. } => {
                self.propagate_borrow_view_ref(table, af, res, root, path, state);
            }
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v,
                ..
            } => {
                if let Some(ArenaRef::Arena { elem_def, handle }) =
                    self.lookup_arena_ref_canon(state, *v)
                {
                    self.record_arena_len_handle(res_c, (elem_def, handle));
                }
            }
            OpKind::Field { base, field, .. } => {
                let pcx = ArenaPropCx {
                    func,
                    table,
                    interns,
                    af,
                };
                self.propagate_field_arena_ref(&pcx, res_c, *base, *field, state);
            }
            OpKind::Deref(ptr) => {
                self.propagate_deref_arena_ref(af, res, res_c, *ptr, state);
            }
            OpKind::Call {
                callee,
                semantic,
                args,
                ..
            } => {
                let pcx = ArenaPropCx {
                    func,
                    table,
                    interns,
                    af,
                };
                self.propagate_call_arena_ref(&pcx, res, callee, semantic, args, state);
            }
            _ => {}
        }
    }

    fn propagate_borrow_view_ref(
        &self,
        table: &TypeTable,
        af: &ProgramBoundsFacts,
        res: ValueId,
        root: &Root,
        path: &[Proj],
        state: &mut ArenaOriginState,
    ) {
        match (root, path) {
            (Root::Cell(c), []) => {
                state.holder_ref.insert(res, HolderRoot::Cell(*c));
                if let Some(&def) = state.local_vec_cells.get(c) {
                    state.arena_refs.insert(
                        res,
                        ArenaRef::Arena {
                            elem_def: def,
                            handle: ArenaHandle::VecCell(*c),
                        },
                    );
                }
                if let Some(&rb) = state.ret_vec_bound_cell.get(c) {
                    state.ret_vec_bound_val.insert(res, rb);
                }
            }
            (Root::Cell(c), [Proj::Field { field, ty }]) => {
                if let Some(def) = vec_def(table, *ty) {
                    if af.arenas.contains_key(&def) {
                        state.arena_refs.insert(
                            res,
                            ArenaRef::Arena {
                                elem_def: def,
                                handle: ArenaHandle::HolderCell(*c, *field),
                            },
                        );
                    }
                }
            }
            (Root::Value(v), [] | [Proj::Deref]) => {
                if let Some(hr) = self.lookup_holder_ref_canon(state, *v) {
                    state.holder_ref.insert(res, hr);
                }
                if let Some(ar) = self.lookup_arena_ref_canon(state, *v) {
                    state.arena_refs.insert(res, ar);
                } else if let Some(rb) = self.lookup_ret_vec_bound_canon(state, *v) {
                    state.ret_vec_bound_val.insert(res, rb);
                }
            }
            (
                Root::Value(v),
                [Proj::Deref, Proj::Field { field, ty }] | [Proj::Field { field, ty }],
            ) => {
                if let Some(def) = vec_def(table, *ty) {
                    if af.arenas.contains_key(&def) {
                        if let Some(hr) = self.lookup_holder_ref_canon(state, *v) {
                            state.arena_refs.insert(
                                res,
                                ArenaRef::Arena {
                                    elem_def: def,
                                    handle: hr.arena_handle(*field),
                                },
                            );
                        }
                    }
                }
                if let Some(ArenaRef::Elem { elem_def, handle }) =
                    self.lookup_arena_ref_canon(state, *v)
                {
                    state.arena_refs.insert(
                        res,
                        ArenaRef::Field {
                            elem_def,
                            field: *field,
                            handle,
                        },
                    );
                }
            }
            (
                Root::Value(v),
                [Proj::Deref, Proj::Index(_)] | [Proj::Deref, Proj::ConstantIndex { .. }],
            ) => match self.lookup_arena_ref_canon(state, *v) {
                Some(ArenaRef::Arena { elem_def, handle }) => {
                    state
                        .arena_refs
                        .insert(res, ArenaRef::Elem { elem_def, handle });
                }
                Some(ArenaRef::InnerVecSlice {
                    elem_def,
                    field,
                    handle,
                }) => {
                    state.arena_refs.insert(
                        res,
                        ArenaRef::Field {
                            elem_def,
                            field,
                            handle,
                        },
                    );
                }
                _ => {
                    if let Some(rb) = self.lookup_ret_vec_bound_canon(state, *v) {
                        state.ret_vec_bound_elem_ref.insert(res, rb);
                    }
                }
            },
            (
                Root::Value(v),
                [Proj::Deref, Proj::Index(_), Proj::Field { field, .. }]
                | [Proj::Deref, Proj::ConstantIndex { .. }, Proj::Field { field, .. }],
            ) => {
                if let Some(ArenaRef::Arena { elem_def, handle }) =
                    self.lookup_arena_ref_canon(state, *v)
                {
                    state.arena_refs.insert(
                        res,
                        ArenaRef::Field {
                            elem_def,
                            field: *field,
                            handle,
                        },
                    );
                }
            }
            _ => {}
        }
    }

    fn propagate_field_arena_ref(
        &mut self,
        pcx: &ArenaPropCx<'_>,
        res_c: ValueId,
        base: ValueId,
        field: u32,
        state: &ArenaOriginState,
    ) {
        let bty = pcx.func.values[base.0 as usize].ty;
        if pcx.interns.is_some_and(|i| {
            is_direct_global_vec_ty(pcx.table, bty) && is_vec_len_field(pcx.table, i, bty, field)
        }) {
            if let Some(&ArenaRef::Arena { elem_def, handle }) = state.arena_refs.get(&base) {
                self.record_arena_len_handle(res_c, (elem_def, handle));
            }
        } else if let Some(&ArenaRef::Elem { elem_def, handle }) = state.arena_refs.get(&base) {
            if let Some(inv) = pcx.af.arenas.get(&elem_def) {
                if let Some(&fk) = inv.fields.get(&field) {
                    match fk {
                        ArenaFieldKind::AlwaysValid => {
                            self.arena_val_origin.insert(
                                res_c,
                                ArenaValOrigin {
                                    elem_def,
                                    handle,
                                    sentinel: None,
                                    source: ArenaIndexSource::Field(field),
                                },
                            );
                        }
                        ArenaFieldKind::ValidOrSentinel { sentinel } => {
                            self.arena_val_origin.insert(
                                res_c,
                                ArenaValOrigin {
                                    elem_def,
                                    handle,
                                    sentinel: Some(sentinel),
                                    source: ArenaIndexSource::Field(field),
                                },
                            );
                        }
                        ArenaFieldKind::VecOfValidIndices => {}
                    }
                }
            }
        }
    }

    fn propagate_deref_arena_ref(
        &mut self,
        af: &ProgramBoundsFacts,
        res: ValueId,
        res_c: ValueId,
        ptr: ValueId,
        state: &mut ArenaOriginState,
    ) {
        match state.arena_refs.get(&ptr) {
            Some(&ar @ ArenaRef::Elem { .. }) => {
                state.arena_refs.insert(res, ar);
            }
            Some(&ArenaRef::Field {
                elem_def,
                field,
                handle,
            }) => {
                if let Some(inv) = af.arenas.get(&elem_def) {
                    if let Some(&fk) = inv.fields.get(&field) {
                        let (sentinel, source) = match fk {
                            ArenaFieldKind::AlwaysValid => (None, ArenaIndexSource::Field(field)),
                            ArenaFieldKind::ValidOrSentinel { sentinel } => {
                                (Some(sentinel), ArenaIndexSource::Field(field))
                            }
                            ArenaFieldKind::VecOfValidIndices => {
                                (None, ArenaIndexSource::VecField(field))
                            }
                        };
                        self.arena_val_origin.insert(
                            res_c,
                            ArenaValOrigin {
                                elem_def,
                                handle,
                                sentinel,
                                source,
                            },
                        );
                    }
                }
            }
            _ => {
                if let Some(&(len_c, callee)) = state.ret_vec_bound_elem_ref.get(&ptr) {
                    self.ret_vec_elem_len.insert(res_c, (len_c, callee));
                }
            }
        }
    }

    fn propagate_call_arena_ref(
        &mut self,
        pcx: &ArenaPropCx<'_>,
        res: ValueId,
        callee: &Callee,
        semantic: &CallSemantic,
        args: &[ValueId],
        state: &mut ArenaOriginState,
    ) {
        let res_c = self.canon(res);
        match semantic {
            CallSemantic::Standard(
                StandardOperation::VecAsSlice
                | StandardOperation::NativeVecAsSlice
                | StandardOperation::VecAsMutSlice
                | StandardOperation::NativeVecAsMutSlice,
            ) if !args.is_empty() => match state.arena_refs.get(&args[0]) {
                Some(&ar @ ArenaRef::Arena { .. }) => {
                    state.arena_refs.insert(res, ar);
                }
                Some(&ArenaRef::Field {
                    elem_def,
                    field,
                    handle,
                }) => {
                    state.arena_refs.insert(
                        res,
                        ArenaRef::InnerVecSlice {
                            elem_def,
                            field,
                            handle,
                        },
                    );
                }
                _ => {
                    if let Some(&rb) = state.ret_vec_bound_val.get(&args[0]) {
                        state.ret_vec_bound_val.insert(res, rb);
                    }
                }
            },
            CallSemantic::Standard(StandardOperation::VecLen) if !args.is_empty() => {
                if let Some(&ArenaRef::Arena { elem_def, handle }) = state.arena_refs.get(&args[0])
                {
                    self.record_arena_len_handle(res_c, (elem_def, handle));
                }
            }
            CallSemantic::Standard(
                StandardOperation::VecIndex | StandardOperation::VecIndexMut,
            ) if !args.is_empty() => match state.arena_refs.get(&args[0]) {
                Some(&ArenaRef::Arena { elem_def, handle }) => {
                    state
                        .arena_refs
                        .insert(res, ArenaRef::Elem { elem_def, handle });
                }
                Some(&ar @ ArenaRef::Field { .. }) => {
                    state.arena_refs.insert(res, ar);
                }
                _ => {
                    if let Some(&rb) = state.ret_vec_bound_val.get(&args[0]) {
                        state.ret_vec_bound_elem_ref.insert(res, rb);
                    }
                }
            },
            CallSemantic::Ordinary => {
                if let Callee::Direct { item, .. } = callee {
                    self.propagate_ordinary_call_arena_ref(pcx, res, res_c, *item, args, state);
                }
            }
            _ => {}
        }
    }

    fn propagate_ordinary_call_arena_ref(
        &mut self,
        pcx: &ArenaPropCx<'_>,
        res: ValueId,
        res_c: ValueId,
        item: ItemId,
        args: &[ValueId],
        state: &mut ArenaOriginState,
    ) {
        let func = pcx.func;
        let af = pcx.af;
        if let Some(ret_idx) = af.return_arena_index.get(&item) {
            if let Some(&arg_a) = args.get(ret_idx.arena_param) {
                let matched_handle = match ret_idx.holder_field {
                    None => match state.arena_refs.get(&arg_a) {
                        Some(&ArenaRef::Arena { elem_def, handle })
                            if elem_def == ret_idx.elem_def =>
                        {
                            Some(handle)
                        }
                        _ => None,
                    },
                    Some(hf) => self
                        .lookup_holder_ref_canon(state, arg_a)
                        .map(|hr| hr.arena_handle(hf)),
                };
                if let Some(handle) = matched_handle {
                    self.arena_val_origin.insert(
                        res_c,
                        ArenaValOrigin {
                            elem_def: ret_idx.elem_def,
                            handle,
                            sentinel: ret_idx.sentinel,
                            source: ArenaIndexSource::CallReturn { callee: item },
                        },
                    );
                }
            }
        }
        if let Some(ret_eb) = af.return_vec_elem_bound.get(&item) {
            let ReturnVecElemBound::LtParamSliceLen { slice_param } = *ret_eb;
            if let Some(&sa) = args.get(slice_param) {
                let sa_c = self.canon(sa);
                if let Some(len_c) = self.find_slice_len_canon(func, sa_c) {
                    state.ret_vec_bound_val.insert(res, (len_c, item));
                    if let Some(&ArenaRef::Arena { elem_def, handle }) = state.arena_refs.get(&sa) {
                        self.record_arena_len_handle(len_c, (elem_def, handle));
                    }
                }
            }
        }
    }

    /// Propagates `arena_val_origin` across block parameters via a sound greatest fixpoint!
    /// Handles arbitrary loop-carried `if/else` merge cursors (e.g. `earcut_linked`,
    /// `eliminate_hole`, `split_earcut`, `filter_points_indexed`) where mutual cycles between
    /// header params and merge-block params would deadlock a naive least fixpoint.
    fn solve_arena_cursor_fixpoint(
        &mut self,
        func: &Func,
        incoming: &[Vec<(RBlockId, &[ValueId])>],
    ) {
        let term_ne: Vec<Vec<usize>> = (0..func.blocks.len())
            .map(|b| self.term_ne_facts(RBlockId(b as u32)))
            .collect();
        // Step A: Collect all distinct `(elem_def, handle)` pairs active in this function.
        let mut active_arenas: Vec<(DefinitionId, ArenaHandle)> = Vec::new();
        for orig in self.arena_val_origin.values() {
            let pair = (orig.elem_def, orig.handle);
            if !active_arenas.contains(&pair) {
                active_arenas.push(pair);
            }
        }
        // Step B: For each active `(elem_def, handle)`, run a greatest fixpoint over all unsigned
        // non-entry block parameters not already in `arena_val_origin`!
        let base_origins = self.arena_val_origin.clone();
        for (target_def, target_handle) in active_arenas {
            self.solve_arena_cursor_for_target(
                func,
                incoming,
                &term_ne,
                &base_origins,
                target_def,
                target_handle,
            );
        }
    }

    fn solve_arena_cursor_for_target(
        &mut self,
        func: &Func,
        incoming: &[Vec<(RBlockId, &[ValueId])>],
        term_ne: &[Vec<usize>],
        base_origins: &BTreeMap<ValueId, ArenaValOrigin>,
        target_def: DefinitionId,
        target_handle: ArenaHandle,
    ) {
        // Seed every unsigned non-entry block param not in `base_origins` optimistically as
        // `AlwaysValid` (`sentinel = None`), then iteratively weaken/remove any param whose
        // incoming edges aren't ALL valid for `(target_def, target_handle)`.
        let mut param_state: BTreeMap<ValueId, Option<u128>> = BTreeMap::new();
        let mut param_loc: Vec<(RBlockId, usize, ValueId, ValueId)> = Vec::new();
        for &b in &self.doms.rpo {
            if b.0 == 0 || incoming[b.0 as usize].is_empty() {
                continue;
            }
            for (pos, &param) in func.blocks[b.0 as usize].params.iter().enumerate() {
                if self.widths[param.0 as usize].is_none() {
                    continue;
                }
                let pc = self.canon(param);
                if base_origins.contains_key(&pc) || self.arena_val_origin.contains_key(&pc) {
                    continue;
                }
                param_state.insert(pc, None);
                param_loc.push((b, pos, param, pc));
            }
        }
        if param_state.is_empty() {
            return;
        }
        loop {
            let mut changed = false;
            for &(b, pos, _param, pc) in &param_loc {
                let Some(&cur_sent) = param_state.get(&pc) else {
                    continue;
                };
                let mut merged_sent = cur_sent;
                let mut ok = true;
                for (pred, args) in &incoming[b.0 as usize] {
                    let Some(&arg) = args.get(pos) else {
                        ok = false;
                        break;
                    };
                    let ac = self.canon(arg);
                    if ac == pc {
                        continue;
                    }
                    let edge_sent = if let Some(&orig) = base_origins.get(&ac) {
                        if orig.elem_def != target_def || orig.handle != target_handle {
                            ok = false;
                            break;
                        }
                        orig.sentinel.filter(|&s| {
                            !term_ne[pred.0 as usize].iter().any(|&ni| {
                                self.canon(self.ne_facts[ni].v) == ac && self.ne_facts[ni].c == s
                            })
                        })
                    } else if let Some(&ps) = param_state.get(&ac) {
                        ps.filter(|&s| {
                            !term_ne[pred.0 as usize].iter().any(|&ni| {
                                self.canon(self.ne_facts[ni].v) == ac && self.ne_facts[ni].c == s
                            })
                        })
                    } else if let Some(OpKind::Const(ConstTree::Scalar { bits, bytes })) =
                        self.def_kind(func, ac)
                    {
                        let max = self.widths[ac.0 as usize].map_or(u128::MAX, width_max);
                        Some(mask(*bits, *bytes).min(max))
                    } else {
                        ok = false;
                        break;
                    };
                    match (merged_sent, edge_sent) {
                        (_, None) => {}
                        (None, Some(s)) => merged_sent = Some(s),
                        (Some(x), Some(y)) if x == y => {}
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    param_state.remove(&pc);
                    changed = true;
                } else if merged_sent != cur_sent {
                    param_state.insert(pc, merged_sent);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // Also verify every surviving param is actually grounded (reachable from at least one
        // `base_origins` value — prevents pure constant cycles with no real arena origin!).
        let mut grounded: BTreeSet<ValueId> = base_origins
            .iter()
            .filter(|(_, o)| o.elem_def == target_def && o.handle == target_handle)
            .map(|(&k, _)| k)
            .collect();
        loop {
            let mut g_changed = false;
            for &(b, pos, _, pc) in &param_loc {
                if !param_state.contains_key(&pc) || grounded.contains(&pc) {
                    continue;
                }
                let has_grounded = incoming[b.0 as usize].iter().any(|(_, args)| {
                    args.get(pos)
                        .is_some_and(|&a| grounded.contains(&self.canon(a)))
                });
                if has_grounded && grounded.insert(pc) {
                    g_changed = true;
                }
            }
            if !g_changed {
                break;
            }
        }
        for &(_, _, param, pc) in &param_loc {
            if grounded.contains(&pc) {
                if let Some(&sent) = param_state.get(&pc) {
                    self.arena_val_origin.insert(
                        pc,
                        ArenaValOrigin {
                            elem_def: target_def,
                            handle: target_handle,
                            sentinel: sent,
                            source: ArenaIndexSource::BlockParam { param },
                        },
                    );
                }
            }
        }
    }

    fn find_slice_len_canon(&self, func: &Func, slice_canon: ValueId) -> Option<ValueId> {
        for b in &func.blocks {
            for op in &b.ops {
                let Some(res) = op.result else { continue };
                if let OpKind::Unary {
                    op: UnaryOp::PtrMetadata,
                    v,
                    ..
                } = op.kind
                {
                    if self.canon(v) == slice_canon {
                        return Some(self.canon(res));
                    }
                }
            }
        }
        None
    }
}
