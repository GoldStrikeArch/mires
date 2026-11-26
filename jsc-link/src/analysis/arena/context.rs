//! Per-function arena handle tracking, local worklists, local bounds facts, write-site
//! collection, and unified arena-value classification.

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::index_vec::IndexVec;
use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::repr::{
    CellId, Dominators, Func, Op, OpKind, Proj, RAssertMessage, RBlockId, RTerm, Root, ValueId,
};
use rcgjs_object::jir::semantic::{
    BinaryOp, CallSemantic, Callee, CastKind, ConstTree, DefinitionId, DropGlue, ItemId,
    StandardOperation, UnaryOp,
};
use rcgjs_object::jir::types::{TypeId, TypeKind, TypeTable};

use crate::analysis::heap::{
    global_vec_elem_ty, is_direct_global_vec_ty, is_global_vec_ty, is_vec_len_field,
};
use crate::lower::js::value;

use super::fixpoint::{
    join_value_class, CandidateFieldKind, CandidateFieldsMap, CtorFieldSrc, ElemCtorSummary,
    FieldWriteSite, ValueArenaClass,
};
use super::param_bounds::find_counted_push_loop_details;
use super::preconditions::is_allowed_inner_idx_vec_op;
use super::{
    adt_def_of, holder_vec_fields, is_empty_vec_val, is_unsigned_index_ty, is_unsigned_vec_ty,
    mask_bits, peel_copy_move_local, propagate_cell_borrows, slice_elem_def, vec_elem_def,
    vec_ref_elem_def, width_max, ArenaFieldKind, ArenaHandle, ReturnArenaIndex,
};

pub(super) type CallRetArenaEntry = (ItemId, Vec<(usize, Option<u32>, DefinitionId, ArenaHandle)>);
pub(super) type WorklistPushes = BTreeMap<(CellId, Option<u32>), Vec<(RBlockId, usize, ValueId)>>;
pub(super) type ParamArenaKey = (ItemId, usize, usize, Option<u32>, DefinitionId);
pub(super) type ParamArenaCandidateMap = BTreeMap<ParamArenaKey, Option<u128>>;

/// Read-only arena candidate tables passed into [`ArenaContextBuilder::build`].
pub(super) struct ArenaScanTables<'a> {
    pub active_defs: &'a [DefinitionId],
    pub candidate_fields: &'a CandidateFieldsMap,
    pub ctor_summaries: &'a BTreeMap<ItemId, ElemCtorSummary>,
}

/// Per-function tracking of arena handles, slice/vec derivations, length SSA values, field loads,
/// non-arena `< arena.len` facts, and local worklists.
pub(super) struct FuncArenaContext {
    pub doms: Dominators,
    /// Defining `(block, op_idx)` per `ValueId`.
    pub defs: IndexVec<ValueId, Option<(RBlockId, usize)>>,
    /// Value numbering representative (peeling `Copy`/`Move` and same-width unsigned casts).
    pub canon: IndexVec<ValueId, ValueId>,
    /// Which `(elem_def, ArenaHandle)` a `Vec<Elem>`, `&Vec<Elem>`, `&mut Vec<Elem>`, `&[Elem]`,
    /// or `&mut [Elem]` SSA value refers to.
    pub val_arena: IndexVec<ValueId, Option<(DefinitionId, ArenaHandle)>>,
    /// Which `(elem_def, ArenaHandle)` a `usize`/`u32` SSA value is the exact `.len` of, along
    /// with its defining `(block, op_idx)`.
    pub len_of_arena_defs: IndexVec<ValueId, Option<(RBlockId, usize, DefinitionId, ArenaHandle)>>,
    /// All `VecPush` sites `(block, op_idx, elem_def, handle)` on monotone arenas in this function.
    pub push_sites: Vec<(RBlockId, usize, DefinitionId, ArenaHandle)>,
    /// Counted `0..N` single-push loop induction variables `(push_block, push_op_idx, iv_canon, elem_def, handle)`.
    pub counted_push_ivs: Vec<(RBlockId, usize, ValueId, DefinitionId, ArenaHandle)>,
    /// Which `(elem_def, field, ArenaHandle)` an SSA value was loaded from (scalar field or element
    /// of a `Vec<Idx>` field).
    pub loaded_from: IndexVec<ValueId, Option<(DefinitionId, u32, ArenaHandle, bool)>>,
    /// Which `(elem_def, ArenaHandle)` an SSA value is a pre-push length of (after push completes).
    pub pre_push_valid: IndexVec<ValueId, Option<(DefinitionId, ArenaHandle)>>,
    /// Direct calls returning a value that might be a `ReturnArenaIndex`.
    pub call_ret_arenas: IndexVec<ValueId, Option<CallRetArenaEntry>>,
    /// Local worklist `Vec<T>` cell (like `todo: Vec<(JsValue, usize)>` in `nested`) whose popped
    /// scalar/tuple-field values are bounded by the set of values pushed into that field.
    pub worklist_field_pushes: WorklistPushes,
    /// Values extracted from a local worklist `Vec` pop: `val -> (cell, tuple_field)`.
    pub worklist_popped: IndexVec<ValueId, Option<(CellId, Option<u32>)>>,
    /// All distinct `ArenaHandle`s seen per `elem_def` in this function.
    pub arenas_per_def: BTreeMap<DefinitionId, BTreeSet<ArenaHandle>>,
    /// Non-arena `< arena.len` facts: `(block, op_idx_or_none, value_canon, elem_def, handle)`.
    pub lt_arena_facts: Vec<(RBlockId, Option<usize>, ValueId, DefinitionId, ArenaHandle)>,
    /// `v != c` facts established on a single-predecessor switch edge into `block`.
    pub ne_const_facts: Vec<(RBlockId, ValueId, u128)>,
    /// `v == c` facts established on a single-predecessor switch edge into `block`.
    pub eq_const_facts: Vec<(RBlockId, ValueId, u128)>,
    /// Block-param incoming edge arguments `param -> Vec<(pred_block, arg_val)>`.
    pub param_incoming: IndexVec<ValueId, Vec<(RBlockId, ValueId)>>,
}

impl FuncArenaContext {
    pub(super) fn has_multiple_arenas_of(&self, def: DefinitionId) -> bool {
        self.arenas_per_def.get(&def).is_some_and(|s| s.len() > 1)
    }

    pub(super) fn canon(&self, v: ValueId) -> ValueId {
        self.canon[v]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HolderRoot {
    Param(usize),
    Cell(CellId),
}

/// Staged builder for [`FuncArenaContext`] and per-function [`FieldWriteSite`]s.
pub(super) struct ArenaContextBuilder<'a> {
    func_idx: usize,
    func: &'a Func,
    types: &'a TypeTable,
    interns: &'a Interns,
    tables: &'a ArenaScanTables<'a>,
    doms: Dominators,
    defs: IndexVec<ValueId, Option<(RBlockId, usize)>>,
    canon: IndexVec<ValueId, ValueId>,
    param_incoming: IndexVec<ValueId, Vec<(RBlockId, ValueId)>>,
    local_vec_cells: BTreeMap<CellId, DefinitionId>,
    holder_ref: IndexVec<ValueId, Option<HolderRoot>>,
    val_arena: IndexVec<ValueId, Option<(DefinitionId, ArenaHandle)>>,
    len_of_arena: IndexVec<ValueId, Option<(DefinitionId, ArenaHandle)>>,
    elem_ref_arena: IndexVec<ValueId, Option<(DefinitionId, ArenaHandle)>>,
    field_ref_arena: IndexVec<ValueId, Option<(DefinitionId, u32, ArenaHandle)>>,
    inner_vec_slice_arena: IndexVec<ValueId, Option<(DefinitionId, u32, ArenaHandle)>>,
    loaded_from: IndexVec<ValueId, Option<(DefinitionId, u32, ArenaHandle, bool)>>,
    pre_push_valid: IndexVec<ValueId, Option<(DefinitionId, ArenaHandle)>>,
    call_ret_arenas: IndexVec<ValueId, Option<CallRetArenaEntry>>,
    arenas_per_def: BTreeMap<DefinitionId, BTreeSet<ArenaHandle>>,
    len_of_arena_defs: IndexVec<ValueId, Option<(RBlockId, usize, DefinitionId, ArenaHandle)>>,
    push_sites: Vec<(RBlockId, usize, DefinitionId, ArenaHandle)>,
    counted_push_ivs: Vec<(RBlockId, usize, ValueId, DefinitionId, ArenaHandle)>,
    lt_arena_facts: Vec<(RBlockId, Option<usize>, ValueId, DefinitionId, ArenaHandle)>,
    ne_const_facts: Vec<(RBlockId, ValueId, u128)>,
    eq_const_facts: Vec<(RBlockId, ValueId, u128)>,
    worklist_field_pushes: WorklistPushes,
    worklist_popped: IndexVec<ValueId, Option<(CellId, Option<u32>)>>,
}

impl<'a> ArenaContextBuilder<'a> {
    pub(super) fn build(
        func_idx: usize,
        func: &'a Func,
        interns: &'a Interns,
        tables: &'a ArenaScanTables<'a>,
        disqualified: &mut BTreeSet<DefinitionId>,
        write_sites: &mut Vec<FieldWriteSite>,
    ) -> FuncArenaContext {
        let mut builder = Self::new(func_idx, func, interns, tables);
        builder.seed_arena_roots();
        builder.propagate_handles_rpo();
        builder.collect_worklists();
        builder.collect_local_arena_facts();
        builder.propagate_len_defs();
        builder.find_counted_push_ivs();
        builder.collect_write_sites(disqualified, write_sites);
        builder.finish()
    }

    fn new(
        func_idx: usize,
        func: &'a Func,
        interns: &'a Interns,
        tables: &'a ArenaScanTables<'a>,
    ) -> Self {
        let n = func.values.len();
        let types = &interns.types;
        let doms = Dominators::of(func);
        let mut defs = IndexVec::from_elem(None, n);
        let mut canon = IndexVec::from_fn(n, |v| v);
        for block in &func.blocks {
            for (i, op) in block.ops.iter().enumerate() {
                if let Some(res) = op.result {
                    defs[res] = Some((block.id, i));
                }
            }
        }
        // Canonicalize `Copy`/`Move` and same-width unsigned `Cast(IntToInt)` in RPO.
        for &b in &doms.rpo {
            for op in &func.blocks[b.0 as usize].ops {
                let Some(res) = op.result else { continue };
                match &op.kind {
                    OpKind::Copy(src) | OpKind::Move(src) => {
                        canon[res] = canon[*src];
                    }
                    OpKind::Cast {
                        kind: CastKind::IntToInt,
                        v,
                        from,
                        to,
                    } => {
                        if let (Some((false, fb)), Some((false, tb))) =
                            (types.int_info(*from), types.int_info(*to))
                        {
                            if fb == tb {
                                canon[res] = canon[*v];
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let mut param_incoming: IndexVec<ValueId, Vec<(RBlockId, ValueId)>> =
            IndexVec::from_fn(n, |_| Vec::new());
        for block in &func.blocks {
            for (target, args) in block.term.edges() {
                if target.0 == 0 {
                    continue;
                }
                for (pos, &arg) in args.iter().enumerate() {
                    if let Some(&param) = func.blocks[target.0 as usize].params.get(pos) {
                        param_incoming[param].push((block.id, arg));
                    }
                }
            }
        }

        Self {
            func_idx,
            func,
            types,
            interns,
            tables,
            doms,
            defs,
            canon,
            param_incoming,
            local_vec_cells: BTreeMap::new(),
            holder_ref: IndexVec::from_elem(None, n),
            val_arena: IndexVec::from_elem(None, n),
            len_of_arena: IndexVec::from_elem(None, n),
            elem_ref_arena: IndexVec::from_elem(None, n),
            field_ref_arena: IndexVec::from_elem(None, n),
            inner_vec_slice_arena: IndexVec::from_elem(None, n),
            loaded_from: IndexVec::from_elem(None, n),
            pre_push_valid: IndexVec::from_elem(None, n),
            call_ret_arenas: IndexVec::from_fn(n, |_| None),
            arenas_per_def: BTreeMap::new(),
            len_of_arena_defs: IndexVec::from_elem(None, n),
            push_sites: Vec::new(),
            counted_push_ivs: Vec::new(),
            lt_arena_facts: Vec::new(),
            ne_const_facts: Vec::new(),
            eq_const_facts: Vec::new(),
            worklist_field_pushes: BTreeMap::new(),
            worklist_popped: IndexVec::from_elem(None, n),
        }
    }

    fn seed_arena_roots(&mut self) {
        // Seed entry params.
        for (p_idx, &p_val) in self.func.params.iter().enumerate() {
            let p_ty = self.func.values[p_val.0 as usize].ty;
            if let Some(def) = slice_elem_def(self.types, p_ty)
                .or_else(|| vec_ref_elem_def(self.types, p_ty))
                .or_else(|| vec_elem_def(self.types, p_ty))
            {
                if self.tables.active_defs.contains(&def) {
                    let h = ArenaHandle::Param(p_idx);
                    self.val_arena[p_val] = Some((def, h));
                    self.arenas_per_def.entry(def).or_default().insert(h);
                }
            }
            let mut is_holder = false;
            for &def in self.tables.active_defs {
                let hfs = holder_vec_fields(self.types, p_ty, def);
                if !hfs.is_empty() {
                    is_holder = true;
                }
                for (_, hf) in hfs {
                    let h = ArenaHandle::ParamHolder(p_idx, hf);
                    self.arenas_per_def.entry(def).or_default().insert(h);
                }
            }
            if is_holder {
                self.holder_ref[p_val] = Some(HolderRoot::Param(p_idx));
            }
        }
        // Seed local cells holding `Vec<Elem>` or holder structs.
        for (c_idx, cell) in self.func.cells.iter().enumerate() {
            let cid = CellId(c_idx as u32);
            if let Some(def) = vec_elem_def(self.types, cell.ty) {
                if self.tables.active_defs.contains(&def) {
                    self.local_vec_cells.insert(cid, def);
                    self.arenas_per_def
                        .entry(def)
                        .or_default()
                        .insert(ArenaHandle::VecCell(cid));
                }
            }
            for &def in self.tables.active_defs {
                for (_, hf) in holder_vec_fields(self.types, cell.ty, def) {
                    self.arenas_per_def
                        .entry(def)
                        .or_default()
                        .insert(ArenaHandle::HolderCell(cid, hf));
                }
            }
        }
    }

    /// Propagates arena handles, lengths, element refs, and field refs in RPO (3 rounds for block params).
    fn propagate_handles_rpo(&mut self) {
        for _ in 0..3 {
            for rpo_idx in 0..self.doms.rpo.len() {
                let b = self.doms.rpo[rpo_idx];
                let block = &self.func.blocks[b.0 as usize];
                if b.0 != 0 {
                    for &param in &block.params {
                        self.propagate_block_param_handles(param);
                    }
                }
                for op in &block.ops {
                    if let Some(res) = op.result {
                        self.propagate_op_handle(res, op);
                    }
                }
            }
        }
    }

    fn propagate_block_param_handles(&mut self, param: ValueId) {
        let inc = &self.param_incoming[param];
        if inc.is_empty() {
            return;
        }
        let mut first: Option<(DefinitionId, ArenaHandle)> = None;
        let mut same = true;
        let mut first_hr: Option<HolderRoot> = None;
        let mut same_hr = true;
        for &(_, arg) in inc {
            if arg == param {
                continue;
            }
            if let Some(h) = self.val_arena[arg] {
                match first {
                    None => first = Some(h),
                    Some(prev) if prev == h => {}
                    Some(_) => same = false,
                }
            } else {
                same = false;
            }
            if let Some(hr) = self.holder_ref[arg] {
                match first_hr {
                    None => first_hr = Some(hr),
                    Some(prev) if prev == hr => {}
                    Some(_) => same_hr = false,
                }
            } else {
                same_hr = false;
            }
        }
        if same {
            if let Some(h) = first {
                self.val_arena[param] = Some(h);
            }
        }
        if same_hr {
            if let Some(hr) = first_hr {
                self.holder_ref[param] = Some(hr);
            }
        }
    }

    fn propagate_op_handle(&mut self, res: ValueId, op: &Op) {
        let rty = self.func.values[res.0 as usize].ty;
        match &op.kind {
            OpKind::Copy(src) | OpKind::Move(src) => {
                if let Some(hr) = self.holder_ref[*src] {
                    self.holder_ref[res] = Some(hr);
                }
                if let Some(h) = self.val_arena[*src] {
                    self.val_arena[res] = Some(h);
                }
                if let Some(h) = self.len_of_arena[*src] {
                    self.len_of_arena[res] = Some(h);
                }
                if let Some(h) = self.elem_ref_arena[*src] {
                    self.elem_ref_arena[res] = Some(h);
                }
                if let Some(h) = self.field_ref_arena[*src] {
                    self.field_ref_arena[res] = Some(h);
                }
                if let Some(h) = self.inner_vec_slice_arena[*src] {
                    self.inner_vec_slice_arena[res] = Some(h);
                }
                if let Some(h) = self.loaded_from[*src] {
                    self.loaded_from[res] = Some(h);
                }
            }
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v,
                from,
                to,
            } => {
                if let (Some((false, fb)), Some((false, tb))) =
                    (self.types.int_info(*from), self.types.int_info(*to))
                {
                    if fb == tb {
                        if let Some(h) = self.len_of_arena[*v] {
                            self.len_of_arena[res] = Some(h);
                        }
                        if let Some(h) = self.loaded_from[*v] {
                            self.loaded_from[res] = Some(h);
                        }
                    }
                }
            }
            OpKind::Cast {
                kind: CastKind::Subtype | CastKind::PointerCoercion,
                v,
                ..
            } => {
                if let Some(hr) = self.holder_ref[*v] {
                    self.holder_ref[res] = Some(hr);
                }
                if let Some(h) = self.val_arena[*v] {
                    self.val_arena[res] = Some(h);
                }
                if let Some(h) = self.elem_ref_arena[*v] {
                    self.elem_ref_arena[res] = Some(h);
                }
                if let Some(h) = self.field_ref_arena[*v] {
                    self.field_ref_arena[res] = Some(h);
                }
            }
            OpKind::CellLoad(c) => {
                if let Some(&def) = self.local_vec_cells.get(c) {
                    self.val_arena[res] = Some((def, ArenaHandle::VecCell(*c)));
                }
            }
            OpKind::Field { base, field, .. } => {
                let bty = self.func.values[base.0 as usize].ty;
                if is_direct_global_vec_ty(self.types, bty)
                    && is_vec_len_field(self.types, self.interns, bty, *field)
                {
                    if let Some(h) = self.val_arena[*base] {
                        self.len_of_arena[res] = Some(h);
                    }
                } else if let Some(def) = adt_def_of(self.types, bty) {
                    if let Some(h) = self.elem_ref_arena[*base] {
                        if h.0 == def {
                            self.loaded_from[res] = Some((def, *field, h.1, false));
                        }
                    }
                }
                // Holder struct value field `holder.nodes`.
                if let Some(def) = vec_elem_def(self.types, rty) {
                    if self.tables.active_defs.contains(&def) {
                        if let Some(hr) = self.holder_ref[*base] {
                            let h = match hr {
                                HolderRoot::Param(pi) => ArenaHandle::ParamHolder(pi, *field),
                                HolderRoot::Cell(c) => ArenaHandle::HolderCell(c, *field),
                            };
                            self.val_arena[res] = Some((def, h));
                        }
                    }
                }
            }
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v,
                ..
            } => {
                if let Some(h) = self.val_arena[*v] {
                    self.len_of_arena[res] = Some(h);
                }
            }
            OpKind::Deref(ptr) => {
                if let Some(h) = self.val_arena[*ptr] {
                    self.val_arena[res] = Some(h);
                } else if let Some(h) = self.elem_ref_arena[*ptr] {
                    self.elem_ref_arena[res] = Some(h);
                } else if let Some((def, field, handle)) = self.field_ref_arena[*ptr] {
                    let is_vec_elem = self.is_vec_idx_field(def, field);
                    self.loaded_from[res] = Some((def, field, handle, is_vec_elem));
                } else if value::pointee_of(self.types, self.func.values[ptr.0 as usize].ty)
                    .is_some()
                {
                    if let Some((def, handle)) = self.deref_vec_len_borrow(*ptr) {
                        self.len_of_arena[res] = Some((def, handle));
                    } else if let Some((def, field, handle)) =
                        self.deref_elem_field_borrow(*ptr, true)
                    {
                        let is_vec_elem = self.is_vec_idx_field(def, field);
                        self.loaded_from[res] = Some((def, field, handle, is_vec_elem));
                    }
                }
            }
            OpKind::BorrowView { root, path, .. } => {
                self.classify_borrow_view(res, root, path);
            }
            OpKind::Call {
                callee,
                args,
                semantic,
                ..
            } => match semantic {
                CallSemantic::Standard(
                    StandardOperation::VecNew
                    | StandardOperation::NativeVecNew
                    | StandardOperation::VecWithCapacity
                    | StandardOperation::NativeVecWithCapacity,
                ) => {
                    if let Some(def) = vec_elem_def(self.types, rty) {
                        if self.tables.active_defs.contains(&def) {
                            let h = ArenaHandle::VecValue(res);
                            self.val_arena[res] = Some((def, h));
                            self.arenas_per_def.entry(def).or_default().insert(h);
                        }
                    }
                }
                CallSemantic::Standard(
                    StandardOperation::VecAsSlice
                    | StandardOperation::NativeVecAsSlice
                    | StandardOperation::VecAsMutSlice
                    | StandardOperation::NativeVecAsMutSlice,
                ) => {
                    if let Some(&a0) = args.first() {
                        if let Some(h) = self.val_arena[a0] {
                            self.val_arena[res] = Some(h);
                        } else if let Some((def, field, handle)) = self.field_ref_arena[a0] {
                            self.inner_vec_slice_arena[res] = Some((def, field, handle));
                        }
                    }
                }
                CallSemantic::Standard(StandardOperation::VecLen) => {
                    if let Some(&a0) = args.first() {
                        if let Some(h) = self.val_arena[a0] {
                            self.len_of_arena[res] = Some(h);
                        }
                    }
                }
                CallSemantic::Standard(
                    StandardOperation::VecIndex | StandardOperation::VecIndexMut,
                ) => {
                    if let Some(&a0) = args.first() {
                        if let Some(h) = self.val_arena[a0] {
                            self.elem_ref_arena[res] = Some(h);
                        } else if let Some((def, field, handle)) = self.field_ref_arena[a0] {
                            self.field_ref_arena[res] = Some((def, field, handle));
                        }
                    }
                }
                CallSemantic::Standard(
                    StandardOperation::VecIter | StandardOperation::VecIterMut,
                ) => {
                    if let Some(&a0) = args.first() {
                        if let Some((def, field, handle)) = self.field_ref_arena[a0] {
                            self.inner_vec_slice_arena[res] = Some((def, field, handle));
                        }
                    }
                }
                CallSemantic::Ordinary => {
                    if let Callee::Direct { item, .. } = callee {
                        if is_unsigned_index_ty(self.types, rty).is_some() {
                            let mut arg_arenas = Vec::new();
                            for (ai, &av) in args.iter().enumerate() {
                                if let Some((def, handle)) = self.val_arena[av] {
                                    arg_arenas.push((ai, None, def, handle));
                                }
                                if let Some((def, hf, handle)) = resolve_holder_arg(
                                    self.func,
                                    self.types,
                                    self.defs.as_slice(),
                                    av,
                                    self.tables.active_defs,
                                ) {
                                    arg_arenas.push((ai, Some(hf), def, handle));
                                }
                            }
                            if !arg_arenas.is_empty() {
                                self.call_ret_arenas[res] = Some((*item, arg_arenas));
                            }
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn is_vec_idx_field(&self, def: DefinitionId, field: u32) -> bool {
        self.tables
            .candidate_fields
            .get(&def)
            .and_then(|m| m.get(&field))
            .is_some_and(|k| matches!(k, CandidateFieldKind::VecIdx { .. }))
    }

    fn classify_borrow_view(&mut self, res: ValueId, root: &Root, path: &[Proj]) {
        // Case 1: Borrow of a local `Vec<Elem>` cell (`&nodes`, `&mut nodes`) or holder struct cell.
        if let Root::Cell(c) = root {
            let cty = self.func.cells[c.0 as usize].ty;
            if path.is_empty() {
                if let Some(&def) = self.local_vec_cells.get(c) {
                    self.val_arena[res] = Some((def, ArenaHandle::VecCell(*c)));
                    return;
                }
                if self
                    .tables
                    .active_defs
                    .iter()
                    .any(|&def| !holder_vec_fields(self.types, cty, def).is_empty())
                {
                    self.holder_ref[res] = Some(HolderRoot::Cell(*c));
                    return;
                }
            }
            if let [Proj::Field { field, ty }] = path {
                if let Some(def) = vec_elem_def(self.types, *ty) {
                    if self.tables.active_defs.contains(&def) {
                        let h = ArenaHandle::HolderCell(*c, *field);
                        self.val_arena[res] = Some((def, h));
                        self.arenas_per_def.entry(def).or_default().insert(h);
                        return;
                    }
                }
            }
        }

        // Case 2: Borrow from an SSA `ValueId`.
        if let Root::Value(v) = root {
            if matches!(path, [] | [Proj::Deref]) {
                if let Some(hr) = self.holder_ref[*v] {
                    self.holder_ref[res] = Some(hr);
                }
            }
            if let Some(h) = self.val_arena[*v] {
                match path {
                    [] | [Proj::Deref] => {
                        self.val_arena[res] = Some(h);
                        return;
                    }
                    [Proj::Deref, Proj::Index(_)] | [Proj::Deref, Proj::ConstantIndex { .. }] => {
                        self.elem_ref_arena[res] = Some(h);
                        return;
                    }
                    [Proj::Deref, Proj::Index(_), Proj::Field { field, .. }]
                    | [Proj::Deref, Proj::ConstantIndex { .. }, Proj::Field { field, .. }] => {
                        self.field_ref_arena[res] = Some((h.0, *field, h.1));
                        return;
                    }
                    _ => {}
                }
            }
            if let [Proj::Deref, Proj::Field { field, ty }] | [Proj::Field { field, ty }] = path {
                if let Some(def) = vec_elem_def(self.types, *ty) {
                    if self.tables.active_defs.contains(&def) {
                        if let Some(hr) = self.holder_ref[*v] {
                            let h = match hr {
                                HolderRoot::Param(pi) => ArenaHandle::ParamHolder(pi, *field),
                                HolderRoot::Cell(c) => ArenaHandle::HolderCell(c, *field),
                            };
                            self.val_arena[res] = Some((def, h));
                            self.arenas_per_def.entry(def).or_default().insert(h);
                            return;
                        }
                    }
                }
            }
            if let Some((def, handle)) = self.elem_ref_arena[*v] {
                match path {
                    [] | [Proj::Deref] => {
                        self.elem_ref_arena[res] = Some((def, handle));
                        return;
                    }
                    [Proj::Deref, Proj::Field { field, .. }] | [Proj::Field { field, .. }] => {
                        self.field_ref_arena[res] = Some((def, *field, handle));
                        return;
                    }
                    _ => {}
                }
            }
            if let Some((def, field, handle)) = self.field_ref_arena[*v] {
                if matches!(path, [] | [Proj::Deref]) {
                    self.field_ref_arena[res] = Some((def, field, handle));
                    return;
                }
            }
            if let Some((def, field, handle)) = self.inner_vec_slice_arena[*v] {
                match path {
                    [] | [Proj::Deref] => {
                        self.inner_vec_slice_arena[res] = Some((def, field, handle));
                    }
                    [Proj::Deref, Proj::Index(_)] | [Proj::Deref, Proj::ConstantIndex { .. }] => {
                        self.field_ref_arena[res] = Some((def, field, handle));
                    }
                    _ => {}
                }
            }
        }
    }

    fn deref_vec_len_borrow(&self, ptr: ValueId) -> Option<(DefinitionId, ArenaHandle)> {
        let (db, di) = self.defs[ptr]?;
        if let OpKind::BorrowView { root, path, .. } = &self.func.blocks[db.0 as usize].ops[di].kind
        {
            match (root, path.as_slice()) {
                (Root::Value(v), [Proj::Deref, Proj::Field { field, .. }]) => {
                    let vty = self.func.values[v.0 as usize].ty;
                    if let Some(pt) = value::pointee_of(self.types, vty) {
                        if is_direct_global_vec_ty(self.types, pt)
                            && is_vec_len_field(self.types, self.interns, pt, *field)
                        {
                            return self.val_arena[*v];
                        }
                    }
                }
                (
                    Root::Value(v),
                    [Proj::Deref, Proj::Field { field: hf, ty: hty }, Proj::Field { field: lf, .. }],
                ) => {
                    if is_direct_global_vec_ty(self.types, *hty)
                        && is_vec_len_field(self.types, self.interns, *hty, *lf)
                    {
                        if let Some(def) = vec_elem_def(self.types, *hty) {
                            if let Some(hr) = self.holder_ref[*v] {
                                let h = match hr {
                                    HolderRoot::Param(pi) => ArenaHandle::ParamHolder(pi, *hf),
                                    HolderRoot::Cell(c) => ArenaHandle::HolderCell(c, *hf),
                                };
                                return Some((def, h));
                            }
                        }
                    }
                }
                (Root::Cell(c), [Proj::Field { field: lf, .. }]) => {
                    let cty = self.func.cells[c.0 as usize].ty;
                    if is_direct_global_vec_ty(self.types, cty)
                        && is_vec_len_field(self.types, self.interns, cty, *lf)
                    {
                        if let Some(def) = vec_elem_def(self.types, cty) {
                            return Some((def, ArenaHandle::VecCell(*c)));
                        }
                    }
                }
                (
                    Root::Cell(c),
                    [Proj::Field { field: hf, ty: hty }, Proj::Field { field: lf, .. }],
                ) if is_direct_global_vec_ty(self.types, *hty)
                    && is_vec_len_field(self.types, self.interns, *hty, *lf) =>
                {
                    if let Some(def) = vec_elem_def(self.types, *hty) {
                        return Some((def, ArenaHandle::HolderCell(*c, *hf)));
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn deref_elem_field_borrow(
        &self,
        ptr: ValueId,
        include_inner_slice: bool,
    ) -> Option<(DefinitionId, u32, ArenaHandle)> {
        let (db, di) = self.defs[ptr]?;
        if let OpKind::BorrowView {
            root: Root::Value(v),
            path,
            ..
        } = &self.func.blocks[db.0 as usize].ops[di].kind
        {
            if let Some((def, handle)) = self.val_arena[*v] {
                if let [Proj::Deref, Proj::Index(_), Proj::Field { field, .. }]
                | [Proj::Deref, Proj::ConstantIndex { .. }, Proj::Field { field, .. }] =
                    path.as_slice()
                {
                    return Some((def, *field, handle));
                }
            }
            if let Some((def, handle)) = self.elem_ref_arena[*v] {
                if let [Proj::Deref, Proj::Field { field, .. }] = path.as_slice() {
                    return Some((def, *field, handle));
                }
            }
            if include_inner_slice {
                if let Some((def, field, handle)) = self.inner_vec_slice_arena[*v] {
                    if let [Proj::Deref, Proj::Index(_)]
                    | [Proj::Deref, Proj::ConstantIndex { .. }] = path.as_slice()
                    {
                        return Some((def, field, handle));
                    }
                }
            }
        }
        None
    }

    fn collect_worklists(&mut self) {
        let (pushes, popped) = WorklistCollector::collect(
            self.func,
            self.types,
            &self.doms,
            self.defs.as_slice(),
            self.canon.as_slice(),
        );
        self.worklist_field_pushes = pushes;
        self.worklist_popped = popped;
    }

    fn collect_local_arena_facts(&mut self) {
        let arena_of_len = |v: ValueId| self.len_of_arena[v].or(self.len_of_arena[self.canon[v]]);
        let const_of = |v: ValueId| -> Option<u128> {
            let c = self.canon[v];
            let (db, di) = self.defs[c]?;
            match self.func.blocks[db.0 as usize].ops[di].kind {
                OpKind::Const(ConstTree::Scalar { bits, bytes }) => {
                    let max = self
                        .types
                        .int_info(self.types.peeled(self.func.values[c.0 as usize].ty))
                        .map_or(u128::MAX, |(_, w)| width_max(w));
                    Some(mask_bits(bits, bytes).min(max))
                }
                _ => None,
            }
        };

        let mut preds: Vec<Vec<u32>> = vec![Vec::new(); self.func.blocks.len()];
        for b in &self.func.blocks {
            for s in b.term.successors() {
                if !preds[s.0 as usize].contains(&b.id.0) {
                    preds[s.0 as usize].push(b.id.0);
                }
            }
        }

        // 1. Executed `Assert` ops and `VecIndex`/`VecIndexMut` calls (which assert `idx < vec.len`).
        for block in &self.func.blocks {
            for (oi, op) in block.ops.iter().enumerate() {
                match &op.kind {
                    OpKind::Assert {
                        cond,
                        expected: true,
                        ..
                    } => {
                        let c = self.canon[*cond];
                        if let Some((db, di)) = self.defs[c] {
                            if let OpKind::Binary {
                                op: BinaryOp::Lt,
                                l,
                                r,
                                ty,
                            } = self.func.blocks[db.0 as usize].ops[di].kind
                            {
                                if matches!(self.types.int_info(ty), Some((false, _))) {
                                    if let Some((def, handle)) = arena_of_len(r) {
                                        self.lt_arena_facts.push((
                                            block.id,
                                            Some(oi),
                                            self.canon[l],
                                            def,
                                            handle,
                                        ));
                                    }
                                }
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
                    } if args.len() >= 2
                        && is_unsigned_index_ty(
                            self.types,
                            self.func.values[args[1].0 as usize].ty,
                        )
                        .is_some() =>
                    {
                        if let Some((def, handle)) = self.val_arena[args[0]] {
                            self.lt_arena_facts.push((
                                block.id,
                                Some(oi),
                                self.canon[args[1]],
                                def,
                                handle,
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }

        // 2. Single-predecessor `Switch` edges.
        for block in self.func.blocks.iter().skip(1) {
            let [pred] = preds[block.id.0 as usize].as_slice() else {
                continue;
            };
            let RTerm::Switch {
                scrut,
                cases,
                default,
                ..
            } = &self.func.blocks[*pred as usize].term
            else {
                continue;
            };
            let mut entering: Vec<Option<u128>> = cases
                .iter()
                .filter(|(_, t, _)| *t == block.id)
                .map(|(v, _, _)| Some(*v))
                .collect();
            if default.0 == block.id {
                entering.push(None);
            }
            if let [None] = entering.as_slice() {
                for &(cv, _, _) in cases {
                    self.ne_const_facts.push((block.id, self.canon[*scrut], cv));
                }
            } else if let [Some(cv)] = entering.as_slice() {
                self.eq_const_facts
                    .push((block.id, self.canon[*scrut], *cv));
            }

            let positive = match entering.as_slice() {
                [Some(0)] => false,
                [Some(1)] => true,
                [None] if cases.iter().all(|(v, _, _)| *v == 0) => true,
                _ => continue,
            };
            let sc = self.canon[*scrut];
            let Some((db, di)) = self.defs[sc] else {
                continue;
            };
            if let OpKind::Binary { op, l, r, ty } = self.func.blocks[db.0 as usize].ops[di].kind {
                if !matches!(self.types.int_info(ty), Some((false, _))) {
                    continue;
                }
                match (op, positive) {
                    (BinaryOp::Lt, true) => {
                        if let Some((def, handle)) = arena_of_len(r) {
                            self.lt_arena_facts
                                .push((block.id, None, self.canon[l], def, handle));
                        }
                    }
                    (BinaryOp::Gt, true) => {
                        if let Some((def, handle)) = arena_of_len(l) {
                            self.lt_arena_facts
                                .push((block.id, None, self.canon[r], def, handle));
                        }
                    }
                    (BinaryOp::Ne, true) | (BinaryOp::Eq, false) => {
                        if let Some(c) = const_of(r) {
                            self.ne_const_facts.push((block.id, self.canon[l], c));
                        } else if let Some(c) = const_of(l) {
                            self.ne_const_facts.push((block.id, self.canon[r], c));
                        }
                    }
                    (BinaryOp::Eq, true) | (BinaryOp::Ne, false) => {
                        if let Some(c) = const_of(r) {
                            self.eq_const_facts.push((block.id, self.canon[l], c));
                        } else if let Some(c) = const_of(l) {
                            self.eq_const_facts.push((block.id, self.canon[r], c));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn propagate_len_defs(&mut self) {
        for (lv, &entry) in self.len_of_arena.iter_enumerated() {
            let Some((def, handle)) = entry else {
                continue;
            };
            if let Some((lb, li)) = self.defs[lv] {
                let clv = self.canon[lv];
                if self.len_of_arena_defs[clv].is_none() {
                    self.len_of_arena_defs[clv] = Some((lb, li, def, handle));
                }
            }
        }
        // Propagate `len_of_arena_defs` across block parameters whose every incoming edge carries
        // a `.len()` read of the SAME monotone arena `(def, handle)` (e.g. merged push blocks in
        // `earcut::linked_list`): on every path entering `b`, `param <= arena.len()@(b, 0)`.
        for _ in 0..2 {
            for &b in &self.doms.rpo {
                if b.0 == 0 {
                    continue;
                }
                for &param in &self.func.blocks[b.0 as usize].params {
                    let pc = self.canon[param];
                    if self.len_of_arena_defs[pc].is_some() {
                        continue;
                    }
                    let inc = &self.param_incoming[param];
                    if inc.is_empty() {
                        continue;
                    }
                    let mut target: Option<(DefinitionId, ArenaHandle)> = None;
                    let mut all_ok = true;
                    for &(pred_b, arg) in inc {
                        let ac = self.canon[arg];
                        if ac == pc {
                            continue;
                        }
                        if let Some((lb, _li, def, handle)) = self.len_of_arena_defs[ac] {
                            if self.doms.dominates(lb, pred_b) {
                                match target {
                                    None => target = Some((def, handle)),
                                    Some(prev) if prev == (def, handle) => {}
                                    Some(_) => {
                                        all_ok = false;
                                        break;
                                    }
                                }
                            } else {
                                all_ok = false;
                                break;
                            }
                        } else {
                            all_ok = false;
                            break;
                        }
                    }
                    if all_ok {
                        if let Some((def, handle)) = target {
                            self.len_of_arena_defs[pc] = Some((b, 0, def, handle));
                        }
                    }
                }
            }
        }
    }

    /// Detects counted `0..N` push loops on local `Vec<Elem>` cells (e.g.
    /// `d3-hierarchy::tidy::layout` `for i in 0..nodes.len() { t.push(Tidy::new(i)); }`).
    fn find_counted_push_ivs(&mut self) {
        if self.local_vec_cells.is_empty() {
            return;
        }
        let mut cell_borrows_local: BTreeMap<ValueId, CellId> = BTreeMap::new();
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
                    if path.is_empty() && self.local_vec_cells.contains_key(c) {
                        cell_borrows_local.insert(res, *c);
                    }
                }
            }
        }
        propagate_cell_borrows(self.func, &self.doms, &mut cell_borrows_local);
        for (&cid, &def) in &self.local_vec_cells {
            if let Some((push_b, push_oi, iv_c, _bound)) = find_counted_push_loop_details(
                self.func,
                self.types,
                &self.doms,
                self.defs.as_slice(),
                self.canon.as_slice(),
                cid,
                &cell_borrows_local,
            ) {
                self.counted_push_ivs
                    .push((push_b, push_oi, iv_c, def, ArenaHandle::VecCell(cid)));
            }
        }
    }

    fn collect_write_sites(
        &mut self,
        disqualified: &mut BTreeSet<DefinitionId>,
        write_sites: &mut Vec<FieldWriteSite>,
    ) {
        let local_post_check_cells: BTreeSet<CellId> = self
            .local_vec_cells
            .keys()
            .copied()
            .filter(|&cid| {
                global_vec_elem_ty(self.types, self.func.cells[cid.0 as usize].ty)
                    .is_some_and(|elem_ty| elem_drop_cannot_read_fields(self.types, elem_ty))
            })
            .collect();

        for rpo_idx in 0..self.doms.rpo.len() {
            let b = self.doms.rpo[rpo_idx];
            let block = &self.func.blocks[b.0 as usize];
            // Track the most recent `len_of_arena` read per `(elem_def, ArenaHandle)` in this block
            // before a `VecPush` (with no intervening push on the same handle!).
            let mut last_len_read: BTreeMap<(DefinitionId, ArenaHandle), ValueId> = BTreeMap::new();
            for (op_idx, op) in block.ops.iter().enumerate() {
                if let Some(res) = op.result {
                    if let Some((def, handle)) = self.len_of_arena[res] {
                        last_len_read.insert((def, handle), res);
                    }
                }
                match &op.kind {
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
                    } if args.len() >= 2 => {
                        self.record_vec_push_writes(
                            (b, op_idx),
                            args,
                            &mut last_len_read,
                            &local_post_check_cells,
                            disqualified,
                            write_sites,
                        );
                    }
                    OpKind::Call {
                        semantic: CallSemantic::Standard(std_op),
                        args,
                        ..
                    } => {
                        if let Some(&a0) = args.first() {
                            if let Some((def, fi, handle)) = self.field_ref_arena[a0] {
                                if !is_allowed_inner_idx_vec_op(*std_op) {
                                    write_sites.push(FieldWriteSite {
                                        func_idx: self.func_idx,
                                        block: b,
                                        op_idx,
                                        elem_def: def,
                                        field: fi,
                                        arena_handle: handle,
                                        value: a0,
                                        const_override: None,
                                        pre_push_len: None,
                                        is_vec_push: true,
                                        local_post_check_ok: false,
                                    });
                                }
                            }
                        }
                    }
                    OpKind::Call { args, .. } => {
                        for &arg in args {
                            let arg_ty = self.func.values[arg.0 as usize].ty;
                            let is_mut_ref = matches!(
                                self.types.get(self.types.peeled(arg_ty)).map(|d| &d.kind),
                                Some(TypeKind::Reference { mutable: true, .. })
                            );
                            if !is_mut_ref {
                                continue;
                            }
                            if let Some((def, fi, handle)) = self.field_ref_arena[arg] {
                                write_sites.push(FieldWriteSite {
                                    func_idx: self.func_idx,
                                    block: b,
                                    op_idx,
                                    elem_def: def,
                                    field: fi,
                                    arena_handle: handle,
                                    value: arg,
                                    const_override: None,
                                    pre_push_len: None,
                                    is_vec_push: self.is_vec_idx_field(def, fi),
                                    local_post_check_ok: false,
                                });
                            }
                        }
                    }
                    OpKind::StoreThrough { ptr, v } => {
                        self.record_store_writes(
                            (b, op_idx),
                            *ptr,
                            *v,
                            &local_post_check_cells,
                            write_sites,
                        );
                    }
                    OpKind::SetField { base, field, v, .. } => {
                        let bty = self.func.values[base.0 as usize].ty;
                        if let Some(def) = adt_def_of(self.types, bty) {
                            if let Some(cfields) = self.tables.candidate_fields.get(&def) {
                                if let Some(&ckind) = cfields.get(field) {
                                    if let Some((_, handle)) = self.elem_ref_arena[*base] {
                                        write_sites.push(FieldWriteSite {
                                            func_idx: self.func_idx,
                                            block: b,
                                            op_idx,
                                            elem_def: def,
                                            field: *field,
                                            arena_handle: handle,
                                            value: *v,
                                            const_override: None,
                                            pre_push_len: None,
                                            is_vec_push: matches!(
                                                ckind,
                                                CandidateFieldKind::VecIdx { .. }
                                            ),
                                            local_post_check_ok: false,
                                        });
                                    } else {
                                        disqualified.insert(def);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn record_vec_push_writes(
        &mut self,
        pos: (RBlockId, usize),
        args: &[ValueId],
        last_len_read: &mut BTreeMap<(DefinitionId, ArenaHandle), ValueId>,
        local_post_check_cells: &BTreeSet<CellId>,
        disqualified: &mut BTreeSet<DefinitionId>,
        write_sites: &mut Vec<FieldWriteSite>,
    ) {
        let (b, op_idx) = pos;
        let recv = args[0];
        let val = args[1];
        if let Some((def, handle)) = self.val_arena[recv] {
            self.push_sites.push((b, op_idx, def, handle));
            let Some(cfields) = self.tables.candidate_fields.get(&def) else {
                return;
            };
            let pre_len = last_len_read.remove(&(def, handle));
            if let Some(pl) = pre_len {
                let cpl = self.canon[pl];
                self.pre_push_valid[cpl] = Some((def, handle));
            }
            let post_ok =
                matches!(handle, ArenaHandle::VecCell(c) if local_post_check_cells.contains(&c));
            // Inspect the pushed `Elem` value: must be either `Aggregate` or a pure `ctor_summaries` call!
            let peeled_val = peel_copy_move_local(self.func, self.defs.as_slice(), val);
            let Some((db, di)) = self.defs[peeled_val] else {
                disqualified.insert(def);
                return;
            };
            match &self.func.blocks[db.0 as usize].ops[di].kind {
                OpKind::Aggregate {
                    ty,
                    variant: 0,
                    fields,
                    ..
                } if adt_def_of(self.types, *ty) == Some(def) => {
                    for (&fi, &ckind) in cfields {
                        let Some(&fv) = fields.get(fi as usize) else {
                            disqualified.insert(def);
                            break;
                        };
                        match ckind {
                            CandidateFieldKind::Scalar { .. } => {
                                write_sites.push(FieldWriteSite {
                                    func_idx: self.func_idx,
                                    block: b,
                                    op_idx,
                                    elem_def: def,
                                    field: fi,
                                    arena_handle: handle,
                                    value: fv,
                                    const_override: None,
                                    pre_push_len: pre_len,
                                    is_vec_push: false,
                                    local_post_check_ok: post_ok,
                                });
                            }
                            CandidateFieldKind::VecIdx { .. } => {
                                if !is_empty_vec_val(self.func, self.defs.as_slice(), fv) {
                                    write_sites.push(FieldWriteSite {
                                        func_idx: self.func_idx,
                                        block: b,
                                        op_idx,
                                        elem_def: def,
                                        field: fi,
                                        arena_handle: handle,
                                        value: fv,
                                        const_override: None,
                                        pre_push_len: None,
                                        is_vec_push: true,
                                        local_post_check_ok: false,
                                    });
                                }
                            }
                        }
                    }
                }
                OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    args: ctor_args,
                    ..
                } if self
                    .tables
                    .ctor_summaries
                    .get(item)
                    .is_some_and(|s| s.elem_def == def) =>
                {
                    let ctor = &self.tables.ctor_summaries[item];
                    for (&fi, &ckind) in cfields {
                        match (ckind, ctor.fields.get(fi as usize)) {
                            (CandidateFieldKind::Scalar { .. }, Some(CtorFieldSrc::Param(pi))) => {
                                if let Some(&arg_v) = ctor_args.get(*pi) {
                                    write_sites.push(FieldWriteSite {
                                        func_idx: self.func_idx,
                                        block: b,
                                        op_idx,
                                        elem_def: def,
                                        field: fi,
                                        arena_handle: handle,
                                        value: arg_v,
                                        const_override: None,
                                        pre_push_len: pre_len,
                                        is_vec_push: false,
                                        local_post_check_ok: post_ok,
                                    });
                                } else {
                                    disqualified.insert(def);
                                }
                            }
                            (
                                CandidateFieldKind::Scalar { .. },
                                Some(CtorFieldSrc::Const(const_v)),
                            ) => {
                                write_sites.push(FieldWriteSite {
                                    func_idx: self.func_idx,
                                    block: b,
                                    op_idx,
                                    elem_def: def,
                                    field: fi,
                                    arena_handle: handle,
                                    value: peeled_val,
                                    const_override: Some(*const_v),
                                    pre_push_len: None,
                                    is_vec_push: false,
                                    local_post_check_ok: false,
                                });
                            }
                            (CandidateFieldKind::VecIdx { .. }, Some(CtorFieldSrc::EmptyVec)) => {}
                            _ => {
                                write_sites.push(FieldWriteSite {
                                    func_idx: self.func_idx,
                                    block: b,
                                    op_idx,
                                    elem_def: def,
                                    field: fi,
                                    arena_handle: handle,
                                    value: peeled_val,
                                    const_override: None,
                                    pre_push_len: None,
                                    is_vec_push: matches!(ckind, CandidateFieldKind::VecIdx { .. }),
                                    local_post_check_ok: false,
                                });
                            }
                        }
                    }
                }
                _ => {
                    disqualified.insert(def);
                }
            }
        } else if let Some((def, fi, handle)) = self.field_ref_arena[recv] {
            let post_ok =
                matches!(handle, ArenaHandle::VecCell(c) if local_post_check_cells.contains(&c));
            write_sites.push(FieldWriteSite {
                func_idx: self.func_idx,
                block: b,
                op_idx,
                elem_def: def,
                field: fi,
                arena_handle: handle,
                value: val,
                const_override: None,
                pre_push_len: None,
                is_vec_push: true,
                local_post_check_ok: post_ok,
            });
        }
    }

    fn record_store_writes(
        &self,
        pos: (RBlockId, usize),
        ptr: ValueId,
        v: ValueId,
        local_post_check_cells: &BTreeSet<CellId>,
        write_sites: &mut Vec<FieldWriteSite>,
    ) {
        let (b, op_idx) = pos;
        if let Some((def, fi, handle)) =
            self.field_ref_arena[ptr].or_else(|| self.deref_elem_field_borrow(ptr, true))
        {
            let is_vec = self.is_vec_idx_field(def, fi);
            let post_ok =
                matches!(handle, ArenaHandle::VecCell(c) if local_post_check_cells.contains(&c));
            write_sites.push(FieldWriteSite {
                func_idx: self.func_idx,
                block: b,
                op_idx,
                elem_def: def,
                field: fi,
                arena_handle: handle,
                value: v,
                const_override: None,
                pre_push_len: None,
                is_vec_push: is_vec,
                local_post_check_ok: post_ok,
            });
        } else if let Some(pt) = value::pointee_of(self.types, self.func.values[ptr.0 as usize].ty)
        {
            if let Some((def, fi, handle)) = self.deref_elem_field_borrow(ptr, false) {
                write_sites.push(FieldWriteSite {
                    func_idx: self.func_idx,
                    block: b,
                    op_idx,
                    elem_def: def,
                    field: fi,
                    arena_handle: handle,
                    value: v,
                    const_override: None,
                    pre_push_len: None,
                    is_vec_push: is_unsigned_vec_ty(self.types, pt).is_some(),
                    local_post_check_ok: false,
                });
            }
        }
    }

    fn finish(self) -> FuncArenaContext {
        FuncArenaContext {
            doms: self.doms,
            defs: self.defs,
            canon: self.canon,
            val_arena: self.val_arena,
            len_of_arena_defs: self.len_of_arena_defs,
            push_sites: self.push_sites,
            counted_push_ivs: self.counted_push_ivs,
            loaded_from: self.loaded_from,
            pre_push_valid: self.pre_push_valid,
            call_ret_arenas: self.call_ret_arenas,
            worklist_field_pushes: self.worklist_field_pushes,
            worklist_popped: self.worklist_popped,
            arenas_per_def: self.arenas_per_def,
            lt_arena_facts: self.lt_arena_facts,
            ne_const_facts: self.ne_const_facts,
            eq_const_facts: self.eq_const_facts,
            param_incoming: self.param_incoming,
        }
    }
}

pub(super) fn resolve_holder_arg(
    func: &Func,
    types: &TypeTable,
    defs: &[Option<(RBlockId, usize)>],
    av: ValueId,
    active_defs: &[DefinitionId],
) -> Option<(DefinitionId, u32, ArenaHandle)> {
    // Entry parameter `&mut Holder` passed directly.
    if let Some((p_idx, _)) = func.params.iter().enumerate().find(|(_, &pv)| pv == av) {
        let p_ty = func.values[av.0 as usize].ty;
        for &def in active_defs {
            let hfs = holder_vec_fields(types, p_ty, def);
            if hfs.len() == 1 {
                return Some((def, hfs[0].1, ArenaHandle::ParamHolder(p_idx, hfs[0].1)));
            }
        }
    }
    // `BorrowView` of a local holder cell `&mut state` (O(1) lookup via `defs`).
    let (db, di) = defs[av.0 as usize]?;
    if let OpKind::BorrowView {
        root: Root::Cell(c),
        path,
        ..
    } = &func.blocks[db.0 as usize].ops[di].kind
    {
        if path.is_empty() {
            let cty = func.cells[c.0 as usize].ty;
            for &def in active_defs {
                let hfs = holder_vec_fields(types, cty, def);
                if hfs.len() == 1 {
                    return Some((def, hfs[0].1, ArenaHandle::HolderCell(*c, hfs[0].1)));
                }
            }
        }
    }
    None
}

fn elem_drop_cannot_read_fields(types: &TypeTable, elem_ty: TypeId) -> bool {
    fn safe_ty(types: &TypeTable, ty: TypeId, seen: &mut BTreeSet<TypeId>) -> bool {
        let peeled = types.peeled(ty);
        let Some(desc) = types.get(peeled) else {
            return false;
        };
        if !desc.layout.needs_drop {
            return true;
        }
        if !seen.insert(peeled) {
            return false;
        }
        if desc.meta.candidate == rcgjs_object::jir::types::RepresentationCandidate::String {
            return true;
        }
        if is_global_vec_ty(types, peeled) {
            return global_vec_elem_ty(types, peeled).is_some_and(|e| safe_ty(types, e, seen));
        }
        match &desc.kind {
            TypeKind::Adt { variants, .. } => variants
                .iter()
                .all(|v| v.fields.iter().all(|f| safe_ty(types, f.ty, seen))),
            TypeKind::Tuple(elems) => elems.iter().all(|&e| safe_ty(types, e, seen)),
            TypeKind::Array { element, .. } => safe_ty(types, *element, seen),
            _ => false,
        }
    }
    let mut seen = BTreeSet::new();
    safe_ty(types, elem_ty, &mut seen)
}

/// Tracks local non-escaping worklist `Vec<T>` cells (e.g. `todo: Vec<(JsValue, usize)>`) whose
/// ONLY mutations are `VecNew`/`WithCapacity`/`VecFromArray`/`VecPush`/`VecPop`.
struct WorklistCollector<'a> {
    func: &'a Func,
    defs: &'a [Option<(RBlockId, usize)>],
    pushes: WorklistPushes,
    disqualified: BTreeSet<CellId>,
}

type WorklistCellMap = IndexVec<ValueId, Option<(CellId, Option<u32>)>>;

impl<'a> WorklistCollector<'a> {
    fn collect(
        func: &'a Func,
        types: &TypeTable,
        doms: &Dominators,
        defs: &'a [Option<(RBlockId, usize)>],
        canon: &[ValueId],
    ) -> (WorklistPushes, WorklistCellMap) {
        let n = func.values.len();
        let mut collector = Self {
            func,
            defs,
            pushes: BTreeMap::new(),
            disqualified: BTreeSet::new(),
        };
        for (ci, cell) in func.cells.iter().enumerate() {
            let cid = CellId(ci as u32);
            if !is_global_vec_ty(types, cell.ty) {
                collector.disqualified.insert(cid);
            }
        }

        let mut cell_ref_map: BTreeMap<ValueId, CellId> = BTreeMap::new();
        for block in &func.blocks {
            for op in &block.ops {
                if let (
                    Some(res),
                    OpKind::BorrowView {
                        root: Root::Cell(c),
                        path,
                        ..
                    },
                ) = (op.result, &op.kind)
                {
                    if path.is_empty() && !collector.disqualified.contains(c) {
                        cell_ref_map.insert(res, *c);
                    } else {
                        collector.disqualified.insert(*c);
                    }
                }
            }
        }
        propagate_cell_borrows(func, doms, &mut cell_ref_map);

        let mut opt_pop_cell: IndexVec<ValueId, Option<CellId>> = IndexVec::from_elem(None, n);
        for block in &func.blocks {
            for (oi, op) in block.ops.iter().enumerate() {
                match &op.kind {
                    OpKind::CellStore { cell, v } if !collector.disqualified.contains(cell) => {
                        let pv = peel_copy_move_local(func, defs, *v);
                        if let Some((db, di)) = defs[pv.0 as usize] {
                            match &func.blocks[db.0 as usize].ops[di].kind {
                                OpKind::Call {
                                    semantic:
                                        CallSemantic::Standard(
                                            StandardOperation::VecNew
                                            | StandardOperation::NativeVecNew
                                            | StandardOperation::VecWithCapacity
                                            | StandardOperation::NativeVecWithCapacity,
                                        ),
                                    ..
                                } => {}
                                OpKind::Call {
                                    semantic:
                                        CallSemantic::Standard(
                                            StandardOperation::NativeVecFromArray
                                            | StandardOperation::VecFromBoxArray,
                                        ),
                                    args,
                                    ..
                                } if !args.is_empty() => {
                                    collector.record_init_array(*cell, (block.id, oi), args[0]);
                                }
                                _ => {
                                    collector.disqualified.insert(*cell);
                                }
                            }
                        } else {
                            collector.disqualified.insert(*cell);
                        }
                    }
                    OpKind::Call {
                        semantic: CallSemantic::Standard(std_op),
                        args,
                        ..
                    } => {
                        if let Some(&a0) = args.first() {
                            if let Some(&cid) = cell_ref_map.get(&a0) {
                                match std_op {
                                    StandardOperation::VecPush
                                    | StandardOperation::NativeVecPush
                                        if args.len() >= 2 =>
                                    {
                                        collector.record_push(cid, (block.id, oi), args[1]);
                                    }
                                    StandardOperation::VecPop | StandardOperation::NativeVecPop => {
                                        if let Some(res) = op.result {
                                            opt_pop_cell[res] = Some(cid);
                                        }
                                    }
                                    StandardOperation::VecLen
                                    | StandardOperation::VecIsEmpty
                                    | StandardOperation::VecReserve
                                    | StandardOperation::NativeVecReserve
                                    | StandardOperation::VecDrop => {}
                                    _ => {
                                        collector.disqualified.insert(cid);
                                    }
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
                            if let Some(&cid) = cell_ref_map.get(&u) {
                                collector.disqualified.insert(cid);
                            }
                        }
                    }
                }
            }
        }

        // Track extracted payload from `Option<T>` returned by `Vec::pop`.
        let mut popped: IndexVec<ValueId, Option<(CellId, Option<u32>)>> =
            IndexVec::from_elem(None, n);
        let mut popped_elem_cell: IndexVec<ValueId, Option<CellId>> = IndexVec::from_elem(None, n);
        let mut opt_cell_src: BTreeMap<CellId, CellId> = BTreeMap::new();
        let mut popped_ref: IndexVec<ValueId, Option<(CellId, Option<u32>)>> =
            IndexVec::from_elem(None, n);
        for &bid in &doms.rpo {
            let block = &func.blocks[bid.0 as usize];
            for op in &block.ops {
                if let OpKind::CellStore { cell, v } = &op.kind {
                    let cv = canon[v.0 as usize];
                    if let Some(cid) = opt_pop_cell[*v].or(opt_pop_cell[cv]) {
                        if !collector.disqualified.contains(&cid) {
                            opt_cell_src.insert(*cell, cid);
                        }
                    }
                }
                let Some(res) = op.result else { continue };
                match &op.kind {
                    OpKind::CellLoad(c) => {
                        if let Some(&cid) = opt_cell_src.get(c) {
                            opt_pop_cell[res] = Some(cid);
                        }
                    }
                    OpKind::BorrowView { root, path, .. } => {
                        let src_cid = match root {
                            Root::Cell(c) => opt_cell_src.get(c).copied(),
                            Root::Value(v) => opt_pop_cell[*v],
                        };
                        if let Some(cid) = src_cid {
                            match path.as_slice() {
                                [Proj::Downcast { .. }, Proj::Field { field: 0, .. }] => {
                                    popped_ref[res] = Some((cid, None));
                                }
                                [Proj::Downcast { .. }, Proj::Field { field: 0, .. }, Proj::Field { field: fi, .. }] =>
                                {
                                    popped_ref[res] = Some((cid, Some(*fi)));
                                }
                                _ => {}
                            }
                        } else if let Root::Value(v) = root {
                            if let Some((cid, None)) = popped_ref[*v] {
                                if let [Proj::Deref, Proj::Field { field: fi, .. }] =
                                    path.as_slice()
                                {
                                    popped_ref[res] = Some((cid, Some(*fi)));
                                }
                            }
                        }
                    }
                    OpKind::Deref(ptr) => {
                        if let Some(p) = popped_ref[*ptr] {
                            popped[res] = Some(p);
                        }
                    }
                    OpKind::Field { base, field, .. } => {
                        let cb = canon[base.0 as usize];
                        if let Some(cid) = opt_pop_cell[*base].or(opt_pop_cell[cb]) {
                            if !collector.disqualified.contains(&cid) {
                                popped_elem_cell[res] = Some(cid);
                                popped[res] = Some((cid, None));
                            }
                        } else if let Some(cid) = popped_elem_cell[*base].or(popped_elem_cell[cb]) {
                            if !collector.disqualified.contains(&cid) {
                                popped[res] = Some((cid, Some(*field)));
                            }
                        }
                    }
                    OpKind::Copy(s) | OpKind::Move(s) => {
                        if let Some(c) = opt_pop_cell[*s] {
                            opt_pop_cell[res] = Some(c);
                        }
                        if let Some(c) = popped_elem_cell[*s] {
                            popped_elem_cell[res] = Some(c);
                        }
                        if let Some(p) = popped_ref[*s] {
                            popped_ref[res] = Some(p);
                        }
                        if let Some(p) = popped[*s] {
                            popped[res] = Some(p);
                        }
                    }
                    _ => {}
                }
            }
        }

        collector
            .pushes
            .retain(|(cid, _), _| !collector.disqualified.contains(cid));
        for slot in popped.iter_mut() {
            if let Some((cid, _)) = *slot {
                if collector.disqualified.contains(&cid) {
                    *slot = None;
                }
            }
        }
        (collector.pushes, popped)
    }

    fn record_init_array(&mut self, cid: CellId, at: (RBlockId, usize), arr_val: ValueId) {
        let p = peel_copy_move_local(self.func, self.defs, arr_val);
        let Some((db, di)) = self.defs[p.0 as usize] else {
            self.disqualified.insert(cid);
            return;
        };
        if let OpKind::Aggregate { fields, .. } = &self.func.blocks[db.0 as usize].ops[di].kind {
            for &elem in fields {
                self.record_push(cid, at, elem);
            }
        } else {
            self.disqualified.insert(cid);
        }
    }

    fn record_push(&mut self, cid: CellId, at: (RBlockId, usize), val: ValueId) {
        let (block, op_idx) = at;
        self.pushes
            .entry((cid, None))
            .or_default()
            .push((block, op_idx, val));
        let p = peel_copy_move_local(self.func, self.defs, val);
        if let Some((db, di)) = self.defs[p.0 as usize] {
            if let OpKind::Aggregate { fields, .. } = &self.func.blocks[db.0 as usize].ops[di].kind
            {
                for (fi, &fv) in fields.iter().enumerate() {
                    self.pushes
                        .entry((cid, Some(fi as u32)))
                        .or_default()
                        .push((block, op_idx, fv));
                }
            }
        }
    }
}

/// Unified arena-value classifier shared by Stage 1 (`param_candidates = None`) and Stage 2
/// (`param_candidates = Some(&candidates)`).
pub(super) struct ValueClassifier<'a> {
    pub func: &'a Func,
    pub types: &'a TypeTable,
    pub ctx: &'a FuncArenaContext,
    pub elem_def: DefinitionId,
    pub handle: ArenaHandle,
    pub field_state: &'a BTreeMap<(DefinitionId, u32), ArenaFieldKind>,
    pub ret_idx_state: &'a BTreeMap<ItemId, ReturnArenaIndex>,
    pub param_candidates: Option<&'a ParamArenaCandidateMap>,
}

impl ValueClassifier<'_> {
    pub(super) fn classify(
        &self,
        block: RBlockId,
        op_idx: usize,
        value: ValueId,
        pre_push_len: Option<ValueId>,
        local_post_check_ok: bool,
    ) -> ValueArenaClass {
        let mut visited = BTreeSet::new();
        let mut cls = self.classify_rec(block, op_idx, value, pre_push_len, &mut visited);
        // Strip sentinel if `value != sentinel` dominates `(block, op_idx)`.
        if let ValueArenaClass::ValidOrSentinel(s) = cls {
            if value_ne_const_at(self.ctx, block, value, s) {
                cls = ValueArenaClass::AlwaysValid;
            }
        }
        if cls != ValueArenaClass::Unknown {
            return cls;
        }
        // Check local post-checked write: for a local unescaped `Vec<Elem>`, if the value is
        // checked `Assert(value < arena.len)` (either unconditionally in the same straight-line
        // block, or in the `value != sentinel` branch right after the write) before any read of
        // `Elem` or escape!
        if local_post_check_ok {
            if let Some(post_cls) = self.check_local_post_assert(block, op_idx, value) {
                return post_cls;
            }
        }
        ValueArenaClass::Unknown
    }

    fn classify_rec(
        &self,
        block: RBlockId,
        op_idx: usize,
        value: ValueId,
        pre_push_len: Option<ValueId>,
        visited: &mut BTreeSet<ValueId>,
    ) -> ValueArenaClass {
        let cv = self.ctx.canon(value);

        // 1. Pre-push length on the SAME arena handle!
        if let Some(pl) = pre_push_len {
            if self.ctx.canon(pl) == cv {
                return ValueArenaClass::AlwaysValid;
            }
        }
        if let Some((pd, ph)) = self.ctx.pre_push_valid[cv] {
            if pd == self.elem_def && ph == self.handle {
                return ValueArenaClass::AlwaysValid;
            }
        }
        if let Some((lb, li, ld, lh)) = self.ctx.len_of_arena_defs[cv] {
            if ld == self.elem_def && lh == self.handle {
                let dom_lt = |a: (RBlockId, usize), b: (RBlockId, usize)| {
                    (a.0 == b.0 && a.1 < b.1) || (a.0 != b.0 && self.ctx.doms.dominates(a.0, b.0))
                };
                let dom_le = |a: (RBlockId, usize), b: (RBlockId, usize)| {
                    (a.0 == b.0 && a.1 <= b.1) || (a.0 != b.0 && self.ctx.doms.dominates(a.0, b.0))
                };
                if self.ctx.push_sites.iter().any(|&(pb, pi, pd, ph)| {
                    pd == self.elem_def
                        && ph == self.handle
                        && dom_lt((lb, li), (pb, pi))
                        && dom_le((pb, pi), (block, op_idx))
                }) {
                    return ValueArenaClass::AlwaysValid;
                }
            }
        }
        // Counted `0..N` unconditional single-push loop induction var.
        if self
            .ctx
            .counted_push_ivs
            .iter()
            .any(|&(pb, pi, iv_c, pd, ph)| {
                iv_c == cv
                    && pd == self.elem_def
                    && ph == self.handle
                    && ((pb == block && pi <= op_idx)
                        || (pb != block && self.ctx.doms.dominates(pb, block)))
            })
        {
            return ValueArenaClass::AlwaysValid;
        }

        // 2. Dominating non-arena `< arena.len` fact on the SAME arena handle!
        for &(fb, foi, fv, fd, fh) in &self.ctx.lt_arena_facts {
            if fd == self.elem_def && fh == self.handle && fv == cv {
                let dom = if fb == block {
                    foi.is_none_or(|i| i < op_idx)
                } else {
                    self.ctx.doms.dominates(fb, block)
                };
                if dom {
                    return ValueArenaClass::AlwaysValid;
                }
            }
        }

        // 3. Constant scalar?
        if let Some((db, di)) = self.ctx.defs[cv] {
            if let OpKind::Const(ConstTree::Scalar { bits, bytes }) =
                self.func.blocks[db.0 as usize].ops[di].kind
            {
                let max = self
                    .types
                    .int_info(self.types.peeled(self.func.values[cv.0 as usize].ty))
                    .map_or(u128::MAX, |(_, w)| width_max(w));
                return ValueArenaClass::Const(mask_bits(bits, bytes).min(max));
            }
        }

        // 4. Loaded from a proven field (or `Vec<Idx>` field) of the SAME arena handle!
        if let Some((ld, lf, lh, _is_vec)) =
            self.ctx.loaded_from[value].or(self.ctx.loaded_from[cv])
        {
            if ld == self.elem_def && lh == self.handle {
                if let Some(&kind) = self.field_state.get(&(ld, lf)) {
                    return match kind {
                        ArenaFieldKind::AlwaysValid | ArenaFieldKind::VecOfValidIndices => {
                            ValueArenaClass::AlwaysValid
                        }
                        ArenaFieldKind::ValidOrSentinel { sentinel } => {
                            if value_ne_const_at(self.ctx, block, value, sentinel) {
                                ValueArenaClass::AlwaysValid
                            } else {
                                ValueArenaClass::ValidOrSentinel(sentinel)
                            }
                        }
                    };
                }
            }
        }

        // 5. Returned from a `ReturnArenaIndex` direct call on the SAME arena handle!
        if let Some((callee_item, arg_arenas)) = self.ctx.call_ret_arenas[value]
            .as_ref()
            .or(self.ctx.call_ret_arenas[cv].as_ref())
        {
            if let Some(ret) = self.ret_idx_state.get(callee_item) {
                if ret.elem_def == self.elem_def
                    && arg_arenas.iter().any(|&(ai, hf, d, h)| {
                        ai == ret.arena_param
                            && hf == ret.holder_field
                            && d == self.elem_def
                            && h == self.handle
                    })
                {
                    return match ret.sentinel {
                        None => ValueArenaClass::AlwaysValid,
                        Some(s) => {
                            if value_ne_const_at(self.ctx, block, value, s) {
                                ValueArenaClass::AlwaysValid
                            } else {
                                ValueArenaClass::ValidOrSentinel(s)
                            }
                        }
                    };
                }
            }
        }

        // 6. Stage-2 only: entry parameter of `func` covered by `param_candidates`.
        if let Some(param_candidates) = self.param_candidates {
            if let Some((pi, _)) = self
                .func
                .params
                .iter()
                .enumerate()
                .find(|(_, &pv)| self.ctx.canon(pv) == cv)
            {
                let (ap, hf) = match self.handle {
                    ArenaHandle::Param(a) => (Some(a), None),
                    ArenaHandle::ParamHolder(a, f) => (Some(a), Some(f)),
                    _ => (None, None),
                };
                if let Some(ap) = ap {
                    if let Some(&sent) =
                        param_candidates.get(&(self.func.item, pi, ap, hf, self.elem_def))
                    {
                        return match sent {
                            None => ValueArenaClass::AlwaysValid,
                            Some(s) => {
                                if value_ne_const_at(self.ctx, block, value, s) {
                                    ValueArenaClass::AlwaysValid
                                } else {
                                    ValueArenaClass::ValidOrSentinel(s)
                                }
                            }
                        };
                    }
                }
            }
        }

        // Cycle guard for block params and worklists (co-inductive: revisiting a cycle node
        // contributes `AlwaysValid` so self-loops like `p = next` inherit the non-cycle edges).
        if !visited.insert(cv) {
            return ValueArenaClass::AlwaysValid;
        }

        // 7. Popped from a local worklist `Vec` cell (e.g. `todo.pop()` in `d3-hierarchy::nested`).
        if let Some((cid, field_opt)) =
            self.ctx.worklist_popped[value].or(self.ctx.worklist_popped[cv])
        {
            if let Some(push_list) = self.ctx.worklist_field_pushes.get(&(cid, field_opt)) {
                if !push_list.is_empty() {
                    let mut acc: Option<ValueArenaClass> = None;
                    for &(pb, poi, pv) in push_list {
                        let c = self.classify_rec(pb, poi, pv, None, visited);
                        acc = Some(match acc {
                            None => c,
                            Some(prev) => join_value_class(prev, c),
                        });
                    }
                    if let Some(res) = acc {
                        return res;
                    }
                }
            }
        }

        // 8. Block parameter: join across all incoming edges!
        let incoming = &self.ctx.param_incoming[cv];
        if !incoming.is_empty() {
            let mut acc: Option<ValueArenaClass> = None;
            for &(pred_b, arg) in incoming {
                let pred_len = self.func.blocks[pred_b.0 as usize].ops.len();
                let mut c = self.classify_rec(pred_b, pred_len, arg, None, visited);
                if let ValueArenaClass::ValidOrSentinel(s) = c {
                    if value_ne_const_at(self.ctx, pred_b, arg, s) {
                        c = ValueArenaClass::AlwaysValid;
                    }
                }
                acc = Some(match acc {
                    None => c,
                    Some(prev) => join_value_class(prev, c),
                });
            }
            if let Some(res) = acc {
                return res;
            }
        }

        ValueArenaClass::Unknown
    }

    /// Checks if a write of `value` at `(block, op_idx)` into a local unescaped `Vec<Elem>` is
    /// immediately post-checked by `Assert(value < arena.len)` (either in the same block after
    /// `op_idx` with no intervening reads of `Elem` or calls, OR in the `value != sentinel` branch
    /// of `block`'s terminator!).
    fn check_local_post_assert(
        &self,
        block: RBlockId,
        op_idx: usize,
        value: ValueId,
    ) -> Option<ValueArenaClass> {
        let cv = self.ctx.canon(value);
        let b = &self.func.blocks[block.0 as usize];
        let has_assert_in = |start_blk: &rcgjs_object::jir::repr::RBlock, start: usize| -> bool {
            let mut cur_b = start_blk;
            let mut idx0 = start;
            for _ in 0..3 {
                for (i, op) in cur_b.ops.iter().enumerate().skip(idx0) {
                    let is_bounds_check = matches!(
                        &op.kind,
                        OpKind::Assert {
                            expected: true,
                            msg: RAssertMessage::BoundsCheck { .. },
                            ..
                        } | OpKind::Call {
                            semantic: CallSemantic::Standard(
                                StandardOperation::VecIndex | StandardOperation::VecIndexMut,
                            ),
                            ..
                        }
                    );
                    if is_bounds_check
                        && self
                            .ctx
                            .lt_arena_facts
                            .iter()
                            .any(|&(fb, foi, fv, fd, fh)| {
                                fb == cur_b.id
                                    && foi == Some(i)
                                    && fv == cv
                                    && fd == self.elem_def
                                    && fh == self.handle
                            })
                    {
                        return true;
                    }
                    match &op.kind {
                        OpKind::Call {
                            semantic:
                                CallSemantic::Standard(
                                    StandardOperation::VecAsSlice
                                    | StandardOperation::NativeVecAsSlice
                                    | StandardOperation::VecAsMutSlice
                                    | StandardOperation::NativeVecAsMutSlice
                                    | StandardOperation::VecLen
                                    | StandardOperation::VecIndex
                                    | StandardOperation::VecIndexMut,
                                ),
                            ..
                        }
                        | OpKind::Copy(_)
                        | OpKind::Move(_)
                        | OpKind::Const(_)
                        | OpKind::Field { .. }
                        | OpKind::Unary { .. }
                        | OpKind::Binary { .. }
                        | OpKind::Cast { .. }
                        | OpKind::BorrowView { .. }
                        | OpKind::Assert { .. }
                        | OpKind::Drop {
                            glue: DropGlue::None,
                            ..
                        } => {}
                        _ => return false,
                    }
                }
                match &cur_b.term {
                    RTerm::Goto(next, args) if args.is_empty() && next != &cur_b.id => {
                        cur_b = &self.func.blocks[next.0 as usize];
                        idx0 = 0;
                    }
                    _ => break,
                }
            }
            false
        };

        if has_assert_in(b, op_idx + 1) {
            return Some(ValueArenaClass::AlwaysValid);
        }

        let mut cur_blk = b;
        let mut start_i = op_idx + 1;
        let switch_blk = loop {
            for op in cur_blk.ops.iter().skip(start_i) {
                match &op.kind {
                    OpKind::Copy(_)
                    | OpKind::Move(_)
                    | OpKind::Const(_)
                    | OpKind::Binary { .. }
                    | OpKind::Cast { .. }
                    | OpKind::Drop {
                        glue: DropGlue::None,
                        ..
                    } => {}
                    _ => return None,
                }
            }
            match &cur_blk.term {
                RTerm::Switch { .. } => break cur_blk,
                RTerm::Goto(next, args) if args.is_empty() && next != &cur_blk.id => {
                    cur_blk = &self.func.blocks[next.0 as usize];
                    start_i = 0;
                }
                _ => return None,
            }
        };
        if let RTerm::Switch { cases, default, .. } = &switch_blk.term {
            let mut targets = Vec::new();
            for &(_, t, _) in cases {
                targets.push(t);
            }
            targets.push(default.0);
            for &(_, fv, sentinel) in &self.ctx.eq_const_facts {
                if fv != cv {
                    continue;
                }
                let eq_ok = targets.iter().any(|&t| {
                    self.ctx
                        .eq_const_facts
                        .iter()
                        .any(|&(fb, f, c)| fb == t && f == cv && c == sentinel)
                });
                if !eq_ok {
                    continue;
                }
                let all_other_assert = targets.iter().all(|&t| {
                    self.ctx
                        .eq_const_facts
                        .iter()
                        .any(|&(fb, f, c)| fb == t && f == cv && c == sentinel)
                        || (self
                            .ctx
                            .ne_const_facts
                            .iter()
                            .any(|&(fb, f, c)| fb == t && f == cv && c == sentinel)
                            && has_assert_in(&self.func.blocks[t.0 as usize], 0))
                });
                if all_other_assert {
                    return Some(ValueArenaClass::ValidOrSentinel(sentinel));
                }
            }
        }
        None
    }
}

pub(super) fn value_ne_const_at(
    ctx: &FuncArenaContext,
    block: RBlockId,
    v: ValueId,
    c: u128,
) -> bool {
    let cv = ctx.canon(v);
    ctx.ne_const_facts
        .iter()
        .any(|&(fb, fv, fc)| fv == cv && fc == c && ctx.doms.dominates(fb, block))
}
