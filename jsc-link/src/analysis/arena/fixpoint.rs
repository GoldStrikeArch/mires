//! Part 2 (Stage 1): Whole-program monotone-arena discovery, constructor summarization,
//! precondition enforcement, and co-inductive greatest fixpoint solver.

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::jir::repr::{CellId, Func, OpKind, RBlockId, RTerm, ValueId};
use rcgjs_object::jir::semantic::{
    CallSemantic, CastKind, ConstTree, DefinitionId, DropGlue, ItemId, StandardOperation,
};
use rcgjs_object::jir::types::{NominalForm, TypeId, TypeKind, TypeTable};

use crate::analysis::heap::global_vec_elem_ty;
use crate::lower::js::value;

use super::context::{ArenaContextBuilder, ArenaScanTables, FuncArenaContext, ValueClassifier};
use super::preconditions::PreconditionChecker;
use super::{
    adt_def_of, func_mentions_def, holder_vec_fields, is_unsigned_index_ty, is_unsigned_vec_ty,
    mask_bits, slice_elem_def, ty_reaches_def, vec_elem_def, vec_ref_elem_def, width_max,
    ArenaFieldKind, ArenaHandle, ArenaInvariant, LiveProgram, ProgramBoundsFacts, ReturnArenaIndex,
};

#[derive(Clone, Debug)]
pub(super) enum CtorFieldSrc {
    Const(u128),
    Param(usize),
    EmptyVec,
    Unknown,
}

/// Summary of a pure constructor helper `fn(...) -> Elem` (e.g. `Node::new`, `Tidy::new`).
#[derive(Clone, Debug)]
pub(super) struct ElemCtorSummary {
    pub elem_def: DefinitionId,
    pub fields: Vec<CtorFieldSrc>,
}

/// Information about a single write to a candidate field `(elem_def, field)` inside one function.
#[derive(Clone, Debug)]
pub(super) struct FieldWriteSite {
    pub func_idx: usize,
    pub block: RBlockId,
    pub op_idx: usize,
    pub elem_def: DefinitionId,
    pub field: u32,
    pub arena_handle: ArenaHandle,
    pub value: ValueId,
    /// When a pure constructor initializes a scalar field to a constant (e.g. `Tidy::new` setting
    /// `default_ancestor: NONE, thread: NONE`), carries that exact constant directly.
    pub const_override: Option<u128>,
    /// When this write occurs inside an `Aggregate` pushed onto `arena_handle` at `(block, op_idx)`,
    /// records the pre-push length SSA value (if `arena_handle.len()` was read before push).
    pub pre_push_len: Option<ValueId>,
    /// Whether this is a push into an inner `Vec<Idx>` field vs scalar write.
    pub is_vec_push: bool,
    /// Whether this write happens to a local unescaped `Vec<Elem>` whose `Elem` has no custom
    /// `Drop` impl, allowing post-checked writes (`Assert(v < arena.len)` before any read/escape).
    pub local_post_check_ok: bool,
}

/// Candidate field type classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CandidateFieldKind {
    Scalar { bits: u32 },
    VecIdx { bits: u32 },
}

pub(super) type CandidateFieldsMap = BTreeMap<DefinitionId, BTreeMap<u32, CandidateFieldKind>>;
pub(super) type HolderFieldsMap = BTreeMap<DefinitionId, BTreeSet<(DefinitionId, u32)>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ValueArenaClass {
    /// Proven strictly `< arena.len`.
    AlwaysValid,
    /// Constant `c` (valid as a sentinel if consistent with field's sentinel).
    Const(u128),
    /// Either `== sentinel` or `< arena.len`.
    ValidOrSentinel(u128),
    /// Unproven.
    Unknown,
}

pub(super) fn analyze_arenas(program: &LiveProgram<'_>, facts: &mut ProgramBoundsFacts) {
    // Step 1: Discover candidate `elem_def`s from any `Vec<Elem, Global>` in user functions.
    let mut disqualified = program.poisoned_defs.clone();
    let mut candidate_fields = discover_candidates(program, &mut disqualified);
    if candidate_fields.is_empty() {
        return;
    }

    // Step 2: Summarize pure constructor helpers `fn(...) -> Elem` (e.g. `Node::new`, `Tidy::new`).
    let ctor_summaries = summarize_ctors(program, &candidate_fields);

    // Step 3: Enforce whole-program closed-world, monotone-Vec, no-subslice, no-overwrite,
    // and no-escape preconditions for every candidate `elem_def`.
    let mut holder_fields = enforce_preconditions(
        program,
        &candidate_fields,
        &ctor_summaries,
        &mut disqualified,
    );
    candidate_fields.retain(|def, _| !disqualified.contains(def));
    if candidate_fields.is_empty() {
        return;
    }

    // Step 4: Collect all field writes and verify single-arena isolation per function.
    let (per_func_info, write_sites) = collect_arena_contexts_and_writes(
        program,
        &candidate_fields,
        &ctor_summaries,
        &mut disqualified,
    );
    candidate_fields.retain(|def, _| !disqualified.contains(def));
    if candidate_fields.is_empty() {
        return;
    }

    // Step 5: Solve non-circular greatest fixpoint over `(elem_def, field)` & `ReturnArenaIndex`.
    solve_arena_gfp(
        program,
        &candidate_fields,
        &ctor_summaries,
        &per_func_info,
        &write_sites,
        &mut holder_fields,
        facts,
    );
}

fn discover_candidates(
    program: &LiveProgram<'_>,
    disqualified: &mut BTreeSet<DefinitionId>,
) -> CandidateFieldsMap {
    let mut candidate_fields: CandidateFieldsMap = BTreeMap::new();
    for &(_, _, func, interns) in &program.live_items {
        if !program.is_user_item(func.item) {
            continue;
        }
        let types = &interns.types;
        let mut check_ty = |ty: TypeId| {
            let Some(elem_ty) = global_vec_elem_ty(types, ty) else {
                return;
            };
            let peeled = types.peeled(elem_ty);
            let Some(desc) = types.get(peeled) else {
                return;
            };
            if desc.layout.zst || types.unsized_tail(peeled).is_some() {
                return;
            }
            let TypeKind::Adt {
                def,
                form: NominalForm::Struct,
                variants,
                ..
            } = &desc.kind
            else {
                return;
            };
            if variants.len() != 1 || disqualified.contains(def) {
                return;
            }
            let mut fields = BTreeMap::new();
            for (fi, field) in variants[0].fields.iter().enumerate() {
                if let Some(bits) = is_unsigned_index_ty(types, field.ty) {
                    fields.insert(fi as u32, CandidateFieldKind::Scalar { bits });
                } else if let Some(bits) = is_unsigned_vec_ty(types, field.ty) {
                    fields.insert(fi as u32, CandidateFieldKind::VecIdx { bits });
                }
            }
            if fields.is_empty() {
                return;
            }
            match candidate_fields.get(def) {
                None => {
                    candidate_fields.insert(*def, fields);
                }
                Some(prev) if prev != &fields => {
                    // Different generic instantiations disagree on candidate field shapes.
                    disqualified.insert(*def);
                }
                Some(_) => {}
            }
        };
        for v in &func.values {
            check_ty(v.ty);
            if let Some(pt) = value::pointee_of(types, v.ty) {
                check_ty(pt);
            }
        }
        for c in &func.cells {
            check_ty(c.ty);
        }
    }
    candidate_fields.retain(|def, _| !disqualified.contains(def));
    candidate_fields
}

fn summarize_ctors(
    program: &LiveProgram<'_>,
    candidate_fields: &CandidateFieldsMap,
) -> BTreeMap<ItemId, ElemCtorSummary> {
    let mut ctor_summaries: BTreeMap<ItemId, ElemCtorSummary> = BTreeMap::new();
    for &(_, _, func, interns) in &program.live_items {
        if !program.is_user_item(func.item) {
            continue;
        }
        let types = &interns.types;
        if let Some(elem_def) = adt_def_of(types, func.sig.result) {
            if candidate_fields.contains_key(&elem_def) {
                if let Some(summary) = try_summarize_elem_ctor(func, types, elem_def) {
                    ctor_summaries.insert(func.item, summary);
                }
            }
        }
    }
    ctor_summaries
}

fn enforce_preconditions(
    program: &LiveProgram<'_>,
    candidate_fields: &CandidateFieldsMap,
    ctor_summaries: &BTreeMap<ItemId, ElemCtorSummary>,
    disqualified: &mut BTreeSet<DefinitionId>,
) -> HolderFieldsMap {
    let mut holder_fields: HolderFieldsMap = BTreeMap::new();
    let candidate_defs: Vec<DefinitionId> = candidate_fields.keys().copied().collect();

    for &(_, _, func, interns) in &program.live_items {
        let types = &interns.types;
        // Drop glues: only verify they do not mention custom user ops; their direct callees
        // (custom `Drop::drop` impls) are scanned separately as `BodySemantic::Ordinary`.
        if !program.is_user_item(func.item) {
            continue;
        }

        // Root / exported functions must NOT expose `Elem` in parameters or return type.
        if program.root_or_exported.contains(&func.item) {
            for &def in &candidate_defs {
                if disqualified.contains(&def) {
                    continue;
                }
                let exposed = func
                    .sig
                    .arguments
                    .iter()
                    .any(|&a| ty_reaches_def(types, a, def))
                    || ty_reaches_def(types, func.sig.result, def);
                if exposed {
                    disqualified.insert(def);
                }
            }
        }

        // Record holder struct fields (e.g. `EarcutState.nodes`).
        for &def in &candidate_defs {
            if disqualified.contains(&def) {
                continue;
            }
            for v in &func.values {
                for hf in holder_vec_fields(types, v.ty, def) {
                    holder_fields.entry(def).or_default().insert(hf);
                }
            }
            for c in &func.cells {
                for hf in holder_vec_fields(types, c.ty, def) {
                    holder_fields.entry(def).or_default().insert(hf);
                }
            }
        }

        // Check every op in `func` against all candidate defs.
        PreconditionChecker::check_func(
            func,
            interns,
            &program.by_item_id,
            ctor_summaries,
            &candidate_defs,
            disqualified,
        );
    }
    holder_fields
}

fn collect_arena_contexts_and_writes(
    program: &LiveProgram<'_>,
    candidate_fields: &CandidateFieldsMap,
    ctor_summaries: &BTreeMap<ItemId, ElemCtorSummary>,
    disqualified: &mut BTreeSet<DefinitionId>,
) -> (Vec<Option<FuncArenaContext>>, Vec<FieldWriteSite>) {
    let mut write_sites: Vec<FieldWriteSite> = Vec::new();
    let mut per_func_info: Vec<Option<FuncArenaContext>> =
        (0..program.live_items.len()).map(|_| None).collect();

    for (func_idx, &(_, _, func, interns)) in program.live_items.iter().enumerate() {
        if !program.is_user_item(func.item) || ctor_summaries.contains_key(&func.item) {
            continue;
        }
        let types = &interns.types;
        let active_defs: Vec<DefinitionId> = candidate_fields
            .keys()
            .copied()
            .filter(|&def| !disqualified.contains(&def) && func_mentions_def(func, types, def))
            .collect();
        if active_defs.is_empty() {
            continue;
        }
        let tables = ArenaScanTables {
            active_defs: &active_defs,
            candidate_fields,
            ctor_summaries,
        };
        let ctx = ArenaContextBuilder::build(
            func_idx,
            func,
            interns,
            &tables,
            disqualified,
            &mut write_sites,
        );
        per_func_info[func_idx] = Some(ctx);
    }

    (per_func_info, write_sites)
}

fn solve_arena_gfp(
    program: &LiveProgram<'_>,
    candidate_fields: &CandidateFieldsMap,
    ctor_summaries: &BTreeMap<ItemId, ElemCtorSummary>,
    per_func_info: &[Option<FuncArenaContext>],
    write_sites: &[FieldWriteSite],
    holder_fields: &mut HolderFieldsMap,
    facts: &mut ProgramBoundsFacts,
) {
    // Non-circular greatest fixpoint over `(elem_def, field) -> ArenaFieldKind` and Stage-1
    // `ReturnArenaIndex` (only proven from pre-push len, non-arena checks, sentinel constants,
    // and co-inductive arena field loads).
    let mut field_state: BTreeMap<(DefinitionId, u32), ArenaFieldKind> = BTreeMap::new();
    for (&def, fields) in candidate_fields {
        for (&fi, &kind) in fields {
            match kind {
                CandidateFieldKind::Scalar { .. } => {
                    field_state.insert((def, fi), ArenaFieldKind::AlwaysValid);
                }
                CandidateFieldKind::VecIdx { .. } => {
                    field_state.insert((def, fi), ArenaFieldKind::VecOfValidIndices);
                }
            }
        }
    }

    let mut ret_idx_state =
        seed_return_arena_candidates(program, candidate_fields, ctor_summaries, per_func_info);

    // Iterate to greatest fixpoint: weaken `AlwaysValid -> ValidOrSentinel(s) -> removed`, and
    // `ReturnArenaIndex` `sentinel: None -> Some(s) -> removed`.
    loop {
        let mut changed = false;

        // First validate `ReturnArenaIndex` entries under current `field_state` + `ret_idx_state`.
        let ret_keys: Vec<ItemId> = ret_idx_state.keys().copied().collect();
        for item_id in ret_keys {
            let Some(&cur) = ret_idx_state.get(&item_id) else {
                continue;
            };
            let Some(&func_idx) = program.by_item_id.get(&item_id) else {
                ret_idx_state.remove(&item_id);
                changed = true;
                continue;
            };
            let (_, _, func, interns) = program.live_items[func_idx];
            let Some(ctx) = per_func_info[func_idx].as_ref() else {
                ret_idx_state.remove(&item_id);
                changed = true;
                continue;
            };
            let handle = match cur.holder_field {
                Some(hf) => ArenaHandle::ParamHolder(cur.arena_param, hf),
                None => ArenaHandle::Param(cur.arena_param),
            };
            let classifier = ValueClassifier {
                func,
                types: &interns.types,
                ctx,
                elem_def: cur.elem_def,
                handle,
                field_state: &field_state,
                ret_idx_state: &ret_idx_state,
                param_candidates: None,
            };
            let next = verify_return_arena_index(&classifier, cur);
            match next {
                None => {
                    ret_idx_state.remove(&item_id);
                    changed = true;
                }
                Some(updated) if updated != cur => {
                    ret_idx_state.insert(item_id, updated);
                    changed = true;
                }
                _ => {}
            }
        }

        // Next validate every `FieldWriteSite` under current `field_state` + `ret_idx_state`.
        for site in write_sites {
            let key = (site.elem_def, site.field);
            let Some(&cur_kind) = field_state.get(&key) else {
                continue;
            };
            let (_, _, func, interns) = program.live_items[site.func_idx];
            let Some(ctx) = per_func_info[site.func_idx].as_ref() else {
                field_state.remove(&key);
                changed = true;
                continue;
            };
            let class = if let Some(c) = site.const_override {
                ValueArenaClass::Const(c)
            } else {
                let classifier = ValueClassifier {
                    func,
                    types: &interns.types,
                    ctx,
                    elem_def: site.elem_def,
                    handle: site.arena_handle,
                    field_state: &field_state,
                    ret_idx_state: &ret_idx_state,
                    param_candidates: None,
                };
                classifier.classify(
                    site.block,
                    site.op_idx,
                    site.value,
                    site.pre_push_len,
                    site.local_post_check_ok,
                )
            };
            let next_kind = merge_field_kind(cur_kind, class, site.is_vec_push);
            match next_kind {
                None => {
                    field_state.remove(&key);
                    changed = true;
                }
                Some(updated) if updated != cur_kind => {
                    field_state.insert(key, updated);
                    changed = true;
                }
                _ => {}
            }
        }

        if !changed {
            break;
        }
    }

    // Collect surviving arenas that have at least one proven field.
    for &def in candidate_fields.keys() {
        let mut proven: BTreeMap<u32, ArenaFieldKind> = BTreeMap::new();
        for (&(d, f), &kind) in &field_state {
            if d == def {
                proven.insert(f, kind);
            }
        }
        if !proven.is_empty() {
            facts.arenas.insert(
                def,
                ArenaInvariant {
                    elem_def: def,
                    fields: proven,
                    holder_fields: holder_fields.remove(&def).unwrap_or_default(),
                },
            );
        }
    }

    // Keep only `ReturnArenaIndex` entries whose `elem_def` is a proven arena.
    for (item_id, ret) in ret_idx_state {
        if facts.arenas.contains_key(&ret.elem_def) {
            facts.return_arena_index.insert(item_id, ret);
        }
    }
}

/// Seeds candidate `ReturnArenaIndex` summaries for internal functions returning an unsigned scalar
/// that take a single unambiguous arena parameter for one of the surviving `elem_def`s.
fn seed_return_arena_candidates(
    program: &LiveProgram<'_>,
    candidate_fields: &CandidateFieldsMap,
    ctor_summaries: &BTreeMap<ItemId, ElemCtorSummary>,
    per_func_info: &[Option<FuncArenaContext>],
) -> BTreeMap<ItemId, ReturnArenaIndex> {
    let mut ret_idx_state: BTreeMap<ItemId, ReturnArenaIndex> = BTreeMap::new();
    for (func_idx, &(_, _, func, interns)) in program.live_items.iter().enumerate() {
        if !program.is_user_item(func.item) || ctor_summaries.contains_key(&func.item) {
            continue;
        }
        let types = &interns.types;
        if is_unsigned_index_ty(types, func.sig.result).is_none() {
            continue;
        }
        let Some(ctx) = per_func_info[func_idx].as_ref() else {
            continue;
        };
        let mut param_arenas: BTreeMap<DefinitionId, Vec<(usize, Option<u32>)>> = BTreeMap::new();
        for (p_idx, &p_val) in func.params.iter().enumerate() {
            let p_ty = func.values[p_val.0 as usize].ty;
            if let Some(def) = slice_elem_def(types, p_ty)
                .or_else(|| vec_ref_elem_def(types, p_ty))
                .or_else(|| vec_elem_def(types, p_ty))
            {
                if candidate_fields.contains_key(&def) {
                    param_arenas.entry(def).or_default().push((p_idx, None));
                }
            }
            for &def in candidate_fields.keys() {
                for (_, hf) in holder_vec_fields(types, p_ty, def) {
                    param_arenas.entry(def).or_default().push((p_idx, Some(hf)));
                }
            }
        }
        if param_arenas.len() == 1 {
            if let Some((&elem_def, list)) = param_arenas.iter().next() {
                if list.len() == 1 && !ctx.has_multiple_arenas_of(elem_def) {
                    let (arena_param, holder_field) = list[0];
                    ret_idx_state.insert(
                        func.item,
                        ReturnArenaIndex {
                            arena_param,
                            holder_field,
                            elem_def,
                            sentinel: None,
                        },
                    );
                }
            }
        }
    }
    ret_idx_state
}

fn verify_return_arena_index(
    classifier: &ValueClassifier<'_>,
    cur: ReturnArenaIndex,
) -> Option<ReturnArenaIndex> {
    let mut acc: Option<ValueArenaClass> = None;
    for block in &classifier.func.blocks {
        let RTerm::Return(Some(ret_v)) = block.term else {
            continue;
        };
        let cls = classifier.classify(block.id, block.ops.len(), ret_v, None, false);
        acc = Some(match acc {
            None => cls,
            Some(prev) => join_value_class(prev, cls),
        });
    }
    let joined = acc?;
    match (cur.sentinel, joined) {
        (_, ValueArenaClass::Unknown) => None,
        (None, ValueArenaClass::AlwaysValid) => Some(cur),
        (None, ValueArenaClass::Const(s) | ValueArenaClass::ValidOrSentinel(s)) => {
            Some(ReturnArenaIndex {
                sentinel: Some(s),
                ..cur
            })
        }
        (Some(s), ValueArenaClass::AlwaysValid) => Some(ReturnArenaIndex {
            sentinel: Some(s),
            ..cur
        }),
        (Some(s), ValueArenaClass::Const(s2) | ValueArenaClass::ValidOrSentinel(s2)) if s == s2 => {
            Some(cur)
        }
        _ => None,
    }
}

pub(super) fn merge_field_kind(
    cur: ArenaFieldKind,
    written: ValueArenaClass,
    is_vec_push: bool,
) -> Option<ArenaFieldKind> {
    if is_vec_push {
        return match (cur, written) {
            (ArenaFieldKind::VecOfValidIndices, ValueArenaClass::AlwaysValid) => {
                Some(ArenaFieldKind::VecOfValidIndices)
            }
            _ => None,
        };
    }
    match (cur, written) {
        (_, ValueArenaClass::Unknown) => None,
        (ArenaFieldKind::AlwaysValid, ValueArenaClass::AlwaysValid) => {
            Some(ArenaFieldKind::AlwaysValid)
        }
        (
            ArenaFieldKind::AlwaysValid,
            ValueArenaClass::Const(s) | ValueArenaClass::ValidOrSentinel(s),
        ) => Some(ArenaFieldKind::ValidOrSentinel { sentinel: s }),
        (ArenaFieldKind::ValidOrSentinel { sentinel }, ValueArenaClass::AlwaysValid) => {
            Some(ArenaFieldKind::ValidOrSentinel { sentinel })
        }
        (
            ArenaFieldKind::ValidOrSentinel { sentinel },
            ValueArenaClass::Const(s) | ValueArenaClass::ValidOrSentinel(s),
        ) if s == sentinel => Some(ArenaFieldKind::ValidOrSentinel { sentinel }),
        _ => None,
    }
}

pub(super) fn join_value_class(a: ValueArenaClass, b: ValueArenaClass) -> ValueArenaClass {
    match (a, b) {
        (ValueArenaClass::Unknown, _) | (_, ValueArenaClass::Unknown) => ValueArenaClass::Unknown,
        (ValueArenaClass::AlwaysValid, ValueArenaClass::AlwaysValid) => {
            ValueArenaClass::AlwaysValid
        }
        (ValueArenaClass::Const(x), ValueArenaClass::Const(y)) if x == y => {
            ValueArenaClass::Const(x)
        }
        (ValueArenaClass::AlwaysValid, ValueArenaClass::Const(s))
        | (ValueArenaClass::Const(s), ValueArenaClass::AlwaysValid)
        | (ValueArenaClass::AlwaysValid, ValueArenaClass::ValidOrSentinel(s))
        | (ValueArenaClass::ValidOrSentinel(s), ValueArenaClass::AlwaysValid) => {
            ValueArenaClass::ValidOrSentinel(s)
        }
        (ValueArenaClass::Const(x), ValueArenaClass::ValidOrSentinel(y))
        | (ValueArenaClass::ValidOrSentinel(x), ValueArenaClass::Const(y))
        | (ValueArenaClass::ValidOrSentinel(x), ValueArenaClass::ValidOrSentinel(y))
            if x == y =>
        {
            ValueArenaClass::ValidOrSentinel(x)
        }
        _ => ValueArenaClass::Unknown,
    }
}

fn try_summarize_elem_ctor(
    func: &Func,
    types: &TypeTable,
    elem_def: DefinitionId,
) -> Option<ElemCtorSummary> {
    // Walk the single normal linear path from entry block 0 (skipping unwind-only blocks;
    // rejects any branching `Switch` or cycles) to the unique `Return(Some(ret_v))`.
    let mut path_blocks: Vec<RBlockId> = Vec::new();
    let mut visited: BTreeSet<RBlockId> = BTreeSet::new();
    let mut block_param_alias: Vec<Option<ValueId>> = vec![None; func.values.len()];
    let mut cur = RBlockId(0);
    let ret_v = loop {
        if !visited.insert(cur) || path_blocks.len() > 16 {
            return None;
        }
        path_blocks.push(cur);
        match &func.blocks[cur.0 as usize].term {
            RTerm::Return(Some(rv)) => break *rv,
            RTerm::Goto(target, args) => {
                let tparams = &func.blocks[target.0 as usize].params;
                for (pos, &arg) in args.iter().enumerate() {
                    if let Some(&tp) = tparams.get(pos) {
                        block_param_alias[tp.0 as usize] = Some(arg);
                    }
                }
                cur = *target;
            }
            _ => return None,
        }
    };
    let mut defs: Vec<Option<&OpKind>> = vec![None; func.values.len()];
    let mut cell_store_val: BTreeMap<CellId, Option<ValueId>> = BTreeMap::new();
    let mut param_map: Vec<Option<usize>> = vec![None; func.values.len()];
    for (idx, &p) in func.params.iter().enumerate() {
        param_map[p.0 as usize] = Some(idx);
    }
    for b in &path_blocks {
        for op in &func.blocks[b.0 as usize].ops {
            match &op.kind {
                OpKind::Const(_)
                | OpKind::Copy(_)
                | OpKind::Move(_)
                | OpKind::Aggregate { .. }
                | OpKind::CellAlloc(_)
                | OpKind::CellLoad(_)
                | OpKind::Cast {
                    kind: CastKind::IntToInt | CastKind::Subtype,
                    ..
                } => {}
                OpKind::CellStore { cell, v } => {
                    cell_store_val
                        .entry(*cell)
                        .and_modify(|s| *s = None)
                        .or_insert(Some(*v));
                }
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
                OpKind::Drop {
                    glue: DropGlue::None,
                    ..
                } => {}
                _ => return None,
            }
            if let Some(res) = op.result {
                defs[res.0 as usize] = Some(&op.kind);
            }
        }
    }
    let peel = |mut v: ValueId| -> ValueId {
        for _ in 0..8 {
            if let Some(src) = block_param_alias[v.0 as usize] {
                v = src;
                continue;
            }
            match defs[v.0 as usize] {
                Some(OpKind::Copy(src) | OpKind::Move(src)) => v = *src,
                Some(OpKind::CellLoad(c)) => {
                    if let Some(Some(sv)) = cell_store_val.get(c) {
                        v = *sv;
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        v
    };
    let OpKind::Aggregate {
        ty,
        variant: 0,
        fields,
        ..
    } = defs[peel(ret_v).0 as usize]?
    else {
        return None;
    };
    if adt_def_of(types, *ty) != Some(elem_def) {
        return None;
    }
    let mut out = Vec::with_capacity(fields.len());
    for &fv in fields {
        let p = peel(fv);
        if let Some(pi) = param_map[p.0 as usize] {
            out.push(CtorFieldSrc::Param(pi));
        } else {
            match defs[p.0 as usize] {
                Some(OpKind::Const(ConstTree::Scalar { bits, bytes })) => {
                    let max = types
                        .int_info(types.peeled(func.values[p.0 as usize].ty))
                        .map_or(u128::MAX, |(_, w)| width_max(w));
                    out.push(CtorFieldSrc::Const(mask_bits(*bits, *bytes).min(max)));
                }
                Some(OpKind::Call {
                    semantic:
                        CallSemantic::Standard(
                            StandardOperation::VecNew
                            | StandardOperation::NativeVecNew
                            | StandardOperation::VecWithCapacity
                            | StandardOperation::NativeVecWithCapacity,
                        ),
                    ..
                }) => {
                    out.push(CtorFieldSrc::EmptyVec);
                }
                _ => out.push(CtorFieldSrc::Unknown),
            }
        }
    }
    Some(ElemCtorSummary {
        elem_def,
        fields: out,
    })
}
