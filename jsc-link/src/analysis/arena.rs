//! Whole-program monotone-arena index invariant, exact return-length, and immutable static-array
//! bounds analysis over live JIR-R functions.
//!
//! Computes sound, fail-closed [`ProgramBoundsFacts`] consumed by `opt/range.rs` and independently
//! re-verified by `opt/witness.rs` under `RCGJS_WITNESS_CHECK=1`:
//!
//! 1. **Immutable static scalar array bounds** (`static_array_bounds`, `static_array_lens`):
//!    exact `[lo, hi]` element intervals and lengths of immutable `static` unsigned scalar arrays
//!    (e.g. `fflate::tables::CLIM: [usize; 19]`).
//! 2. **Whole-program monotone-arena invariants** (`arenas`):
//!    proves when an element struct `Elem` stored in `Vec<Elem, Global>` is closed-world,
//!    never shrunk or overwritten, never subsliced, and single-arena isolated, and computes a
//!    non-circular greatest fixpoint of fields (`AlwaysValid`, `ValidOrSentinel { sentinel }`,
//!    `VecOfValidIndices`) whose values are guaranteed `< arena.len` (or `== sentinel`).
//! 3. **Interprocedural summaries** (strictly Stage 2, zero circularity with Stage 1):
//!    - `return_vec_len`: exact returned `Vec<T, Global>` length (e.g. `1 << arg_k`, `arg_k`,
//!      `arg_k.len()`, or conditional on a `bool` parameter such as `fflate::h_map`).
//!    - `return_vec_elem_bound`: returned `Vec<u32|usize, Global>` whose every element is strictly
//!      `< arg[k].len()` (e.g. `d3-hierarchy::walk`).
//!    - `return_arena_index`: internal direct functions returning an index valid for an arena
//!      parameter (e.g. `EarcutState::create_node`, `insert_node`, `split_polygon`, `contour`).
//!    - `param_arena_indices`: closed internal functions whose every caller supplies a proven
//!      arena index paired with an arena parameter.
//!    - `param_slice_eq_len`: closed internal functions whose every caller passes two slices/vecs
//!      of provably equal length (e.g. `d3-hierarchy::tidy::{contour, apportion}`).

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::jir::decline::{Decline, EncodeDecline};
use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::repr::{
    CellId, Dominators, Func, OpKind, Proj, RAssertMessage, RBlockId, RTerm, Root, ValueId,
};
use rcgjs_object::jir::semantic::{
    BinaryOp, BodyKind, BodySemantic, CallSemantic, Callee, CastKind, ConstTree, DefinitionId,
    DropGlue, GlobalReference, ItemId, StandardOperation, UnaryOp,
};
use rcgjs_object::jir::summary::Availability;
use rcgjs_object::jir::types::{NominalForm, TypeId, TypeKind, TypeTable};
use rcgjs_object::object::ItemKind;

use crate::analysis::heap::{
    self, global_vec_elem_ty, is_direct_global_vec_ty, is_global_vec_ty, is_vec_len_field, HeapPlan,
};
use crate::item::{JsName, LinkItem};
use crate::lower::js::value;
use crate::opts::Opts;
use crate::plan::native_types::NativeBody;

/// Whole-program bounds facts computed before per-function optimization and checked by witnesses.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProgramBoundsFacts {
    /// Proven length-monotone arenas, keyed by the element struct's `DefinitionId`.
    pub arenas: BTreeMap<DefinitionId, ArenaInvariant>,
    /// Exact returned `Vec<T, Global>` length summaries, keyed by target `ItemId`.
    pub return_vec_len: BTreeMap<ItemId, ReturnVecLen>,
    /// Exact returned `Vec<u32|usize, Global>` element upper bounds, keyed by target `ItemId`.
    pub return_vec_elem_bound: BTreeMap<ItemId, ReturnVecElemBound>,
    /// Internal functions returning an index valid for one of their arena parameters.
    pub return_arena_index: BTreeMap<ItemId, ReturnArenaIndex>,
    /// Closed internal functions whose every caller passes a valid arena index for `idx_param`.
    pub param_arena_indices: BTreeMap<ItemId, Vec<ParamArenaIndex>>,
    /// Closed internal functions whose every caller passes equal-length slices/vecs at `(a, b)`.
    pub param_slice_eq_len: BTreeMap<ItemId, Vec<ParamSliceEqLen>>,
    /// Exact `[lo, hi]` element intervals for immutable static unsigned scalar arrays/slices.
    pub static_array_bounds: BTreeMap<DefinitionId, (u128, u128)>,
    /// Exact element counts for immutable static arrays/slices.
    pub static_array_lens: BTreeMap<DefinitionId, u128>,
}

/// Whole-program invariant for one arena element struct `Elem`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArenaInvariant {
    pub elem_def: DefinitionId,
    /// Proven struct fields (variant 0, field index -> invariant).
    pub fields: BTreeMap<u32, ArenaFieldKind>,
    /// Exact element type definitions of `Vec<Elem>` holders if any, recorded for diagnostics.
    pub holder_fields: BTreeSet<(DefinitionId, u32)>,
}

/// Invariant proven for one field of an arena element struct `Elem`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArenaFieldKind {
    /// Unsigned scalar field whose value is always `< arena.len`.
    AlwaysValid,
    /// Unsigned scalar field whose value is either `== sentinel` or `< arena.len`.
    ValidOrSentinel { sentinel: u128 },
    /// `Vec<u32|usize, Global>` field whose every element is always `< arena.len`.
    VecOfValidIndices,
}

/// Exact length of a `Vec<T, Global>` returned by a direct function.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReturnVecLen {
    Const(u128),
    Param(usize),
    ParamSliceLen(usize),
    OneShlParam {
        shift_param: usize,
        bits: u32,
    },
    CondBoolParam {
        cond_param: usize,
        when_true: Box<ReturnVecLen>,
        when_false: Box<ReturnVecLen>,
    },
}

/// Element upper bound of a `Vec<u32|usize, Global>` returned by a direct function.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReturnVecElemBound {
    /// Every element in the returned `Vec` is strictly `< arg[slice_param].len()`.
    LtParamSliceLen { slice_param: usize },
}

/// Return value of an internal function proven to be a valid index into an arena parameter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReturnArenaIndex {
    pub arena_param: usize,
    pub holder_field: Option<u32>,
    pub elem_def: DefinitionId,
    pub sentinel: Option<u128>,
}

/// Parameter of a closed internal function proven to be a valid index into an arena parameter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParamArenaIndex {
    pub idx_param: usize,
    pub arena_param: usize,
    pub holder_field: Option<u32>,
    pub elem_def: DefinitionId,
    pub sentinel: Option<u128>,
}

/// Pair of slice/vec parameters of a closed internal function proven equal-length at all callers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParamSliceEqLen {
    pub param_a: usize,
    pub param_b: usize,
}

/// Describes how an SSA value or cell refers to a specific arena instance inside a function.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ArenaHandle {
    /// A local cell holding `Vec<Elem>`.
    VecCell(CellId),
    /// A local cell holding a holder struct whose field `field` is `Vec<Elem>`.
    HolderCell(CellId, u32),
    /// Function entry parameter (either `&[Elem]`, `&mut [Elem]`, `&Vec<Elem>`, `&mut Vec<Elem>`,
    /// or `Vec<Elem>`).
    Param(usize),
    /// Function entry parameter `&Holder` / `&mut Holder` / `Holder` at `holder_field`.
    ParamHolder(usize, u32),
    /// An SSA `Vec<Elem>` value (e.g. returned from a call or created directly).
    VecValue(ValueId),
}

/// Runs the whole-program arena, exact-length, and immutable-static-array analyses over all
/// `live` items in `table`.
pub fn analyze(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    native_bodies: &BTreeMap<JsName, NativeBody>,
    live: &BTreeSet<JsName>,
    opts: &Opts,
    heap: &HeapPlan,
) -> ProgramBoundsFacts {
    let mut facts = ProgramBoundsFacts::default();

    // Filter out dead Tier-A / native-string / native-vec fallback bodies (e.g.
    // `alloc::alloc::Global::alloc_elems` / `RawVec` / `Vec::drop -> Vec::as_mut_ptr`) proven
    // unreachable from program roots once intercepted native hooks are excluded.
    let mut filtered_live = heap::filter_live_for_repr(table, funcs, live, opts, heap);
    if heap.native_vec_enabled {
        filtered_live = prune_dead_native_vec_drop_helpers(table, funcs, &filtered_live, opts);
    }

    // -------------------------------------------------------------------------
    // 1. Immutable static unsigned scalar array bounds & lengths
    // -------------------------------------------------------------------------
    collect_static_array_bounds(table, &filtered_live, &mut facts);

    // -------------------------------------------------------------------------
    // 2. Gather live function metadata & check completeness
    // -------------------------------------------------------------------------
    let mut live_items: Vec<(&JsName, &LinkItem, &Func, &Interns)> = Vec::new();
    let mut by_item_id: BTreeMap<ItemId, usize> = BTreeMap::new();
    let mut std_body_items: BTreeMap<ItemId, StandardOperation> = BTreeMap::new();
    let mut drop_glue_items: BTreeSet<ItemId> = BTreeSet::new();
    let mut root_or_exported: BTreeSet<ItemId> = BTreeSet::new();
    let mut address_taken: BTreeSet<ItemId> = BTreeSet::new();
    let mut all_arenas_poisoned = false;
    let mut poisoned_defs: BTreeSet<DefinitionId> = BTreeSet::new();

    for name in &filtered_live {
        let Some(item) = table.get(name) else {
            continue;
        };
        let is_root = item.linkage.root
            || item.linkage.exported
            || opts.roots.iter().any(|r| r == name.as_str());

        if item.kind == ItemKind::Static {
            if let (Some(body), Some(interns)) = (item.jir.body.as_ref(), item.jir.interns()) {
                collect_all_adt_defs_in_ty(
                    &interns.types,
                    body.signature.result,
                    &mut poisoned_defs,
                    &mut BTreeSet::new(),
                );
                if let BodyKind::Static { init, .. } = &body.kind {
                    collect_const_fn_refs(init, &mut address_taken);
                }
            } else {
                // Linker-synthesized constants/registers (`$hc_*`, `$r*`, `$se`, `$sn`, `$ur_*`,
                // `__rt`) carry no JIR summary or `EncodeDecline::Synthesized` and never hold a
                // user `Vec<Elem>` arena.
                let synthesized = item.jir.summary.is_none()
                    || matches!(
                        item.jir.summary.as_ref().map(|s| &s.availability),
                        Some(Availability::Unavailable(Decline::Encode(
                            EncodeDecline::Synthesized
                        )))
                    );
                if !synthesized || item.js.contains(".buf") {
                    all_arenas_poisoned = true;
                }
            }
            continue;
        }
        if item.kind != ItemKind::Fn {
            continue;
        }
        if is_skipped_stdlib_hook(item, native_bodies, heap) {
            continue;
        }
        let body_sem = item.jir.body.as_ref().map(|b| b.semantic);
        let Some(interns) = item.jir.interns() else {
            let synthesized = item.jir.summary.is_none()
                || matches!(
                    item.jir.summary.as_ref().map(|s| &s.availability),
                    Some(Availability::Unavailable(Decline::Encode(
                        EncodeDecline::Synthesized
                    )))
                );
            if !synthesized {
                all_arenas_poisoned = true;
            }
            continue;
        };
        let Some(Ok(func)) = funcs.get(name) else {
            // Declined or unlowered live function: conservatively poison any ADT its crate
            // mentions, or everything if no body metadata is available.
            if let Some(body) = item.jir.body.as_ref() {
                if !matches!(body.semantic, BodySemantic::DropGlue(_)) {
                    let mut seen = BTreeSet::new();
                    for &arg_ty in &body.signature.arguments {
                        collect_all_adt_defs_in_ty(
                            &interns.types,
                            arg_ty,
                            &mut poisoned_defs,
                            &mut seen,
                        );
                    }
                    collect_all_adt_defs_in_ty(
                        &interns.types,
                        body.signature.result,
                        &mut poisoned_defs,
                        &mut seen,
                    );
                    for local in &body.locals {
                        collect_all_adt_defs_in_ty(
                            &interns.types,
                            local.ty,
                            &mut poisoned_defs,
                            &mut seen,
                        );
                    }
                }
            } else {
                all_arenas_poisoned = true;
            }
            continue;
        };

        if is_root {
            root_or_exported.insert(func.item);
        }
        match body_sem {
            Some(BodySemantic::Standard(op)) => {
                std_body_items.insert(func.item, op);
            }
            Some(BodySemantic::DropGlue(_)) => {
                drop_glue_items.insert(func.item);
            }
            _ => {}
        }

        let idx = live_items.len();
        by_item_id.insert(func.item, idx);
        live_items.push((name, item, func, interns));
    }

    // Track address-taken functions and verify every call to a `BodySemantic::Standard` body
    // uses the matching `CallSemantic::Standard(op)`.
    let mut non_standard_called_std: BTreeSet<ItemId> = BTreeSet::new();
    for &(_, _, func, _) in &live_items {
        for block in &func.blocks {
            for op in &block.ops {
                match &op.kind {
                    OpKind::Const(tree) => collect_const_fn_refs(tree, &mut address_taken),
                    OpKind::Call {
                        callee: Callee::Direct { item, .. },
                        semantic,
                        ..
                    } => {
                        if let Some(&expected_op) = std_body_items.get(item) {
                            if *semantic != CallSemantic::Standard(expected_op) {
                                non_standard_called_std.insert(*item);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Items whose bodies are trusted stdlib implementations (only when not root, not
    // address-taken, and every call site goes through matching `CallSemantic::Standard`).
    let trusted_std_bodies: BTreeSet<ItemId> = std_body_items
        .keys()
        .copied()
        .filter(|id| {
            !root_or_exported.contains(id)
                && !address_taken.contains(id)
                && !non_standard_called_std.contains(id)
        })
        .collect();

    // Exclude private stdlib helpers reachable ONLY from inside `trusted_std_bodies` (e.g.
    // `alloc$vec$impl_3_vec_push` called solely inside `Vec::push`'s `Standard(VecPush)` body).
    // Any helper called directly by user code remains reachable and is fully checked!
    if !root_or_exported.is_empty() {
        let mut reachable_idx: BTreeSet<usize> = BTreeSet::new();
        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        for (idx, &(_, _, func, _)) in live_items.iter().enumerate() {
            if root_or_exported.contains(&func.item) && reachable_idx.insert(idx) {
                queue.push_back(idx);
            }
        }
        while let Some(idx) = queue.pop_front() {
            let (_, _, func, _) = live_items[idx];
            if trusted_std_bodies.contains(&func.item) {
                continue;
            }
            for block in &func.blocks {
                for op in &block.ops {
                    match &op.kind {
                        OpKind::Call {
                            callee: Callee::Direct { item, .. },
                            ..
                        } => {
                            if let Some(&ci) = by_item_id.get(item) {
                                if reachable_idx.insert(ci) {
                                    queue.push_back(ci);
                                }
                            }
                        }
                        OpKind::Drop {
                            glue: DropGlue::Direct { item, .. },
                            ..
                        } => {
                            if let Some(&ci) = by_item_id.get(item) {
                                if reachable_idx.insert(ci) {
                                    queue.push_back(ci);
                                }
                            }
                        }
                        OpKind::Const(tree) => {
                            let mut refs = BTreeSet::new();
                            collect_const_fn_refs(tree, &mut refs);
                            for r in refs {
                                if let Some(&ci) = by_item_id.get(&r) {
                                    if reachable_idx.insert(ci) {
                                        queue.push_back(ci);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        if !reachable_idx.is_empty() {
            let old = std::mem::take(&mut live_items);
            by_item_id.clear();
            for (idx, entry) in old.into_iter().enumerate() {
                if reachable_idx.contains(&idx) {
                    let new_idx = live_items.len();
                    by_item_id.insert(entry.2.item, new_idx);
                    live_items.push(entry);
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // 3. Stage 1: Whole-program monotone-arena discovery & verification
    // -------------------------------------------------------------------------
    if !all_arenas_poisoned {
        analyze_arenas(
            &live_items,
            &by_item_id,
            &trusted_std_bodies,
            &drop_glue_items,
            &root_or_exported,
            &poisoned_defs,
            &mut facts,
        );
    }

    // -------------------------------------------------------------------------
    // 4. Stage 2: Interprocedural return-length, return-elem-bound, param-arena,
    //    and equal-slice-length summaries (never fed back into Stage 1!)
    // -------------------------------------------------------------------------
    analyze_return_vec_lens(
        &live_items,
        &trusted_std_bodies,
        &drop_glue_items,
        &mut facts,
    );
    analyze_return_vec_elem_bounds(
        &live_items,
        &trusted_std_bodies,
        &drop_glue_items,
        &mut facts,
    );
    analyze_param_arena_indices(
        &live_items,
        &by_item_id,
        &trusted_std_bodies,
        &drop_glue_items,
        &root_or_exported,
        &address_taken,
        &mut facts,
    );
    analyze_param_slice_eq_len(
        &live_items,
        &by_item_id,
        &trusted_std_bodies,
        &drop_glue_items,
        &root_or_exported,
        &address_taken,
        &mut facts,
    );

    facts
}

fn prune_dead_native_vec_drop_helpers(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    live: &BTreeSet<JsName>,
    opts: &Opts,
) -> BTreeSet<JsName> {
    let mut visited: BTreeSet<JsName> = BTreeSet::new();
    let mut queue: std::collections::VecDeque<JsName> = std::collections::VecDeque::new();
    for name in live {
        let Some(item) = table.get(name) else {
            continue;
        };
        if (item.linkage.root
            || item.linkage.exported
            || opts.roots.iter().any(|r| r == name.as_str()))
            && visited.insert(name.clone())
        {
            queue.push_back(name.clone());
        }
    }
    if visited.is_empty() {
        return live.clone();
    }
    while let Some(name) = queue.pop_front() {
        let Some(item) = table.get(&name) else {
            continue;
        };
        let is_vec_drop_body = matches!(
            item.jir.body.as_ref().map(|b| b.semantic),
            Some(BodySemantic::Standard(StandardOperation::VecDrop))
        );
        if is_vec_drop_body {
            // Under `native_vec_enabled`, `VecDrop` is lowered natively (dropping elements
            // directly without calling `Vec::as_mut_ptr` / `RawVec::deallocate`); keep the
            // element drop glue edges if any, skip `Vec::as_mut_ptr` / `RawVec` plumbing.
            for r in &item.refs {
                if !live.contains(r) {
                    continue;
                }
                let is_drop_glue = table.get(r).is_some_and(|ri| {
                    matches!(
                        ri.jir.body.as_ref().map(|b| b.semantic),
                        Some(BodySemantic::DropGlue(_))
                    ) && !ri.debug_path.contains("raw_vec::RawVec")
                        && !ri.name.as_str().contains("raw_vec$RawVec")
                });
                if is_drop_glue && visited.insert(r.clone()) {
                    queue.push_back(r.clone());
                }
            }
            continue;
        }
        if let (Some(Ok(func)), Some(interns)) = (funcs.get(&name), item.jir.interns()) {
            for block in &func.blocks {
                for op in &block.ops {
                    if let OpKind::Call {
                        callee: Callee::Direct { link_name, .. },
                        semantic,
                        ..
                    } = &op.kind
                    {
                        if matches!(semantic, CallSemantic::Standard(StandardOperation::VecDrop)) {
                            if let Some(s) = interns.str(*link_name) {
                                let jn = JsName::new(s.to_owned());
                                if live.contains(&jn) && visited.insert(jn.clone()) {
                                    queue.push_back(jn);
                                }
                            }
                            continue;
                        }
                        if let Some(s) = interns.str(*link_name) {
                            let jn = JsName::new(s.to_owned());
                            if live.contains(&jn) && visited.insert(jn.clone()) {
                                queue.push_back(jn);
                            }
                        }
                    }
                    if let OpKind::Drop {
                        glue: DropGlue::Direct { link_name, .. },
                        ..
                    } = &op.kind
                    {
                        if let Some(s) = interns.str(*link_name) {
                            let jn = JsName::new(s.to_owned());
                            if live.contains(&jn) && visited.insert(jn.clone()) {
                                queue.push_back(jn);
                            }
                        }
                    }
                }
            }
        }
        for r in &item.refs {
            if !live.contains(r) {
                continue;
            }
            if heap::is_global_vec_drop_glue(Some(item))
                && table.get(r).is_some_and(|ri| {
                    ri.debug_path.contains("raw_vec::RawVec")
                        || ri.name.as_str().contains("raw_vec$RawVec")
                })
            {
                continue;
            }
            if visited.insert(r.clone()) {
                queue.push_back(r.clone());
            }
        }
    }
    visited
}

fn is_skipped_stdlib_hook(
    item: &LinkItem,
    native_bodies: &BTreeMap<JsName, NativeBody>,
    heap: &HeapPlan,
) -> bool {
    if native_bodies.contains_key(&item.name) {
        return true;
    }
    if heap.native_vec_enabled
        && !item.linkage.root
        && !item.linkage.exported
        && (item.debug_path.starts_with("alloc::raw_vec::")
            || item.name.as_str().starts_with("alloc$raw_vec$"))
    {
        return true;
    }
    if let Some(body) = item.jir.body.as_ref() {
        if let BodySemantic::Standard(op) = body.semantic {
            if (heap.typed_heap_enabled && (op.is_typed_heap() || op.is_fast_str_op()))
                || (heap.native_string_enabled && op.is_native_string())
                || (heap.native_vec_enabled && op.is_native_vec())
            {
                return true;
            }
        }
        if matches!(body.semantic, BodySemantic::DropGlue(_))
            && heap.native_vec_enabled
            && heap::is_noop_global_vec_drop_glue(
                Some(item),
                heap.native_string_enabled,
                heap.native_vec_enabled,
            )
        {
            return true;
        }
    }
    false
}

fn collect_const_fn_refs(tree: &ConstTree, out: &mut BTreeSet<ItemId>) {
    match tree {
        ConstTree::Global {
            target: GlobalReference::Function { item, .. },
            ..
        } => {
            out.insert(*item);
        }
        ConstTree::Aggregate(items) | ConstTree::Slice(items) => {
            for item in items {
                collect_const_fn_refs(item, out);
            }
        }
        ConstTree::Variant { fields, .. } => {
            for field in fields {
                collect_const_fn_refs(field, out);
            }
        }
        ConstTree::Reference(inner) | ConstTree::Dyn { data: inner, .. } => {
            collect_const_fn_refs(inner, out);
        }
        _ => {}
    }
}

fn collect_all_adt_defs_in_ty(
    table: &TypeTable,
    ty: TypeId,
    out: &mut BTreeSet<DefinitionId>,
    seen: &mut BTreeSet<TypeId>,
) {
    let peeled = table.peeled(ty);
    if !seen.insert(peeled) {
        return;
    }
    let Some(desc) = table.get(peeled) else {
        return;
    };
    match &desc.kind {
        TypeKind::Adt {
            def,
            args,
            variants,
            ..
        } => {
            out.insert(*def);
            for &a in args {
                collect_all_adt_defs_in_ty(table, a, out, seen);
            }
            for v in variants {
                for f in &v.fields {
                    collect_all_adt_defs_in_ty(table, f.ty, out, seen);
                }
            }
        }
        TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
            collect_all_adt_defs_in_ty(table, *pointee, out, seen);
        }
        TypeKind::Array { element, .. } | TypeKind::Slice(element) => {
            collect_all_adt_defs_in_ty(table, *element, out, seen);
        }
        TypeKind::Tuple(elems) => {
            for &e in elems {
                collect_all_adt_defs_in_ty(table, e, out, seen);
            }
        }
        TypeKind::Closure { upvars, .. } | TypeKind::CoroutineClosure { upvars, .. } => {
            for &u in upvars {
                collect_all_adt_defs_in_ty(table, u, out, seen);
            }
        }
        _ => {}
    }
}

fn ty_reaches_def(
    table: &TypeTable,
    ty: TypeId,
    target: DefinitionId,
    seen: &mut BTreeSet<TypeId>,
) -> bool {
    let peeled = table.peeled(ty);
    if !seen.insert(peeled) {
        return false;
    }
    let Some(desc) = table.get(peeled) else {
        return false;
    };
    match &desc.kind {
        TypeKind::Adt {
            def,
            args,
            variants,
            ..
        } => {
            if *def == target {
                return true;
            }
            args.iter().any(|&a| ty_reaches_def(table, a, target, seen))
                || variants.iter().any(|v| {
                    v.fields
                        .iter()
                        .any(|f| ty_reaches_def(table, f.ty, target, seen))
                })
        }
        TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
            ty_reaches_def(table, *pointee, target, seen)
        }
        TypeKind::Array { element, .. } | TypeKind::Slice(element) => {
            ty_reaches_def(table, *element, target, seen)
        }
        TypeKind::Tuple(elems) => elems
            .iter()
            .any(|&e| ty_reaches_def(table, e, target, seen)),
        TypeKind::Closure { upvars, .. } | TypeKind::CoroutineClosure { upvars, .. } => upvars
            .iter()
            .any(|&u| ty_reaches_def(table, u, target, seen)),
        _ => false,
    }
}

fn adt_def_of(table: &TypeTable, ty: TypeId) -> Option<DefinitionId> {
    match table.get(table.peeled(ty)).map(|d| &d.kind) {
        Some(TypeKind::Adt { def, .. }) => Some(*def),
        _ => None,
    }
}

fn slice_elem_def(table: &TypeTable, ty: TypeId) -> Option<DefinitionId> {
    let pt = value::pointee_of(table, ty)?;
    match table.get(table.peeled(pt)).map(|d| &d.kind) {
        Some(TypeKind::Slice(elem)) => adt_def_of(table, *elem),
        _ => None,
    }
}

fn vec_elem_def(table: &TypeTable, ty: TypeId) -> Option<DefinitionId> {
    let elem = global_vec_elem_ty(table, ty)?;
    adt_def_of(table, elem)
}

fn vec_ref_elem_def(table: &TypeTable, ty: TypeId) -> Option<DefinitionId> {
    let pt = value::pointee_of(table, ty)?;
    vec_elem_def(table, pt)
}

/// If `ty` is a struct (or `&Struct`/`&mut Struct`) having a field of type `Vec<Elem, Global>`,
/// returns `(struct_def, field_idx, elem_def)` for all such fields.
fn holder_vec_fields(
    table: &TypeTable,
    ty: TypeId,
    elem_def: DefinitionId,
) -> Vec<(DefinitionId, u32)> {
    let base = value::pointee_of(table, ty).unwrap_or(ty);
    let peeled = table.peeled(base);
    let mut out = Vec::new();
    if let Some(TypeKind::Adt {
        def,
        form: NominalForm::Struct,
        variants,
        ..
    }) = table.get(peeled).map(|d| &d.kind)
    {
        if variants.len() == 1 {
            for (fi, field) in variants[0].fields.iter().enumerate() {
                if vec_elem_def(table, field.ty) == Some(elem_def) {
                    out.push((*def, fi as u32));
                }
            }
        }
    }
    out
}

fn is_unsigned_index_ty(table: &TypeTable, ty: TypeId) -> Option<u32> {
    match table.int_info(table.peeled(ty)) {
        Some((false, bits)) if bits >= 16 => Some(bits),
        _ => None,
    }
}

fn is_unsigned_vec_ty(table: &TypeTable, ty: TypeId) -> Option<u32> {
    let elem = global_vec_elem_ty(table, ty)?;
    is_unsigned_index_ty(table, elem)
}

fn width_max(bits: u32) -> u128 {
    if bits >= 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    }
}

fn mask_bits(bits: u128, bytes: u8) -> u128 {
    match bytes {
        0 => 0,
        1..=15 => bits & ((1u128 << (bytes as u32 * 8)) - 1),
        _ => bits,
    }
}

// =================================================================================================
// Part 1: Immutable Static Scalar Array Bounds
// =================================================================================================

fn collect_static_array_bounds(
    table: &BTreeMap<JsName, LinkItem>,
    live: &BTreeSet<JsName>,
    facts: &mut ProgramBoundsFacts,
) {
    for name in live {
        let Some(item) = table.get(name) else {
            continue;
        };
        if item.kind != ItemKind::Static {
            continue;
        }
        let (Some(body), Some(interns)) = (item.jir.body.as_ref(), item.jir.interns()) else {
            continue;
        };
        let BodyKind::Static {
            init,
            mutable: false,
        } = &body.kind
        else {
            continue;
        };
        let types = &interns.types;
        let mut ty = types.peeled(body.signature.result);
        let mut tree = init;
        while let ConstTree::Reference(inner) = tree {
            tree = inner.as_ref();
            if let Some(pt) = value::pointee_of(types, ty) {
                ty = types.peeled(pt);
            }
        }
        let elem_ty = match types.get(ty).map(|d| &d.kind) {
            Some(TypeKind::Array { element, .. } | TypeKind::Slice(element)) => *element,
            _ => continue,
        };
        let items = match tree {
            ConstTree::Aggregate(items) | ConstTree::Slice(items) => items,
            _ => continue,
        };
        facts
            .static_array_lens
            .insert(body.item.definition, items.len() as u128);
        let Some((false, bits)) = types.int_info(types.peeled(elem_ty)) else {
            continue;
        };
        if items.is_empty() {
            continue;
        }
        let max_val = width_max(bits);
        let mut lo = u128::MAX;
        let mut hi = 0u128;
        let mut all_scalars = true;
        for elem in items {
            let ConstTree::Scalar { bits: raw, bytes } = elem else {
                all_scalars = false;
                break;
            };
            let v = mask_bits(*raw, *bytes).min(max_val);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        if all_scalars {
            facts
                .static_array_bounds
                .insert(body.item.definition, (lo, hi));
        }
    }
}

// =================================================================================================
// Part 2 (Stage 1): Whole-Program Monotone-Arena Invariant
// =================================================================================================

#[derive(Clone, Debug)]
enum CtorFieldSrc {
    Const(u128),
    Param(usize),
    EmptyVec,
    Unknown,
}

/// Summary of a pure constructor helper `fn(...) -> Elem` (e.g. `Node::new`, `Tidy::new`).
#[derive(Clone, Debug)]
struct ElemCtorSummary {
    elem_def: DefinitionId,
    fields: Vec<CtorFieldSrc>,
}

/// Allowed `StandardOperation` calls on `Vec<Elem, Global>` for a monotone arena.
fn is_monotone_arena_vec_op(op: StandardOperation) -> bool {
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
fn is_allowed_inner_idx_vec_op(op: StandardOperation) -> bool {
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

/// Information about a single write to a candidate field `(elem_def, field)` inside one function.
#[derive(Clone, Debug)]
struct FieldWriteSite {
    func_idx: usize,
    block: RBlockId,
    op_idx: usize,
    elem_def: DefinitionId,
    field: u32,
    arena_handle: ArenaHandle,
    value: ValueId,
    /// When a pure constructor initializes a scalar field to a constant (e.g. `Tidy::new` setting
    /// `default_ancestor: NONE, thread: NONE`), carries that exact constant directly.
    const_override: Option<u128>,
    /// When this write occurs inside an `Aggregate` pushed onto `arena_handle` at `(block, op_idx)`,
    /// records the pre-push length SSA value (if `arena_handle.len()` was read before push).
    pre_push_len: Option<ValueId>,
    /// Whether this is a push into an inner `Vec<Idx>` field vs scalar write.
    is_vec_push: bool,
    /// Whether this write happens to a local unescaped `Vec<Elem>` whose `Elem` has no custom
    /// `Drop` impl, allowing post-checked writes (`Assert(v < arena.len)` before any read/escape).
    local_post_check_ok: bool,
}

/// Candidate field type classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CandidateFieldKind {
    Scalar { bits: u32 },
    VecIdx { bits: u32 },
}

fn analyze_arenas(
    live_items: &[(&JsName, &LinkItem, &Func, &Interns)],
    by_item_id: &BTreeMap<ItemId, usize>,
    trusted_std_bodies: &BTreeSet<ItemId>,
    drop_glue_items: &BTreeSet<ItemId>,
    root_or_exported: &BTreeSet<ItemId>,
    poisoned_defs: &BTreeSet<DefinitionId>,
    facts: &mut ProgramBoundsFacts,
) {
    // Step 1: Discover candidate `elem_def`s from any `Vec<Elem, Global>` in user functions.
    let mut candidate_fields: BTreeMap<DefinitionId, BTreeMap<u32, CandidateFieldKind>> =
        BTreeMap::new();
    let mut disqualified: BTreeSet<DefinitionId> = poisoned_defs.clone();
    let mut holder_fields: BTreeMap<DefinitionId, BTreeSet<(DefinitionId, u32)>> = BTreeMap::new();

    for &(_, _, func, interns) in live_items {
        if trusted_std_bodies.contains(&func.item) || drop_glue_items.contains(&func.item) {
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
    if candidate_fields.is_empty() {
        return;
    }

    // Step 2: Check pure constructor helpers `fn(...) -> Elem` (e.g. `Node::new`, `Tidy::new`).
    let mut ctor_summaries: BTreeMap<ItemId, ElemCtorSummary> = BTreeMap::new();
    for &(_, _, func, interns) in live_items {
        if trusted_std_bodies.contains(&func.item) || drop_glue_items.contains(&func.item) {
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

    // Step 3: Enforce whole-program closed-world, monotone-Vec, no-subslice, no-overwrite,
    // and no-escape preconditions for every candidate `elem_def`.
    let candidate_defs: Vec<DefinitionId> = candidate_fields.keys().copied().collect();
    for &(_, _, func, interns) in live_items {
        let types = &interns.types;
        // Drop glues: only verify they do not mention custom user ops; their direct callees
        // (custom `Drop::drop` impls) are scanned separately as `BodySemantic::Ordinary`.
        if drop_glue_items.contains(&func.item) {
            continue;
        }
        if trusted_std_bodies.contains(&func.item) {
            continue;
        }

        // Root / exported functions must NOT expose `Elem` in parameters or return type.
        if root_or_exported.contains(&func.item) {
            for &def in &candidate_defs {
                if disqualified.contains(&def) {
                    continue;
                }
                let mut seen = BTreeSet::new();
                let exposed = func
                    .sig
                    .arguments
                    .iter()
                    .any(|&a| ty_reaches_def(types, a, def, &mut seen))
                    || ty_reaches_def(types, func.sig.result, def, &mut seen);
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
        check_func_arena_preconditions(
            func,
            types,
            interns,
            by_item_id,
            &ctor_summaries,
            &candidate_defs,
            &mut disqualified,
        );
    }

    candidate_fields.retain(|def, _| !disqualified.contains(def));
    if candidate_fields.is_empty() {
        return;
    }

    // Step 4: Collect all field writes and verify single-arena isolation per function.
    // Also collect Stage-1 return-arena-index candidates (e.g. `create_node`, `contour`).
    let mut write_sites: Vec<FieldWriteSite> = Vec::new();
    let mut per_func_info: Vec<Option<FuncArenaContext>> =
        (0..live_items.len()).map(|_| None).collect();

    for (func_idx, &(_, _, func, interns)) in live_items.iter().enumerate() {
        if trusted_std_bodies.contains(&func.item) || drop_glue_items.contains(&func.item) {
            continue;
        }
        if ctor_summaries.contains_key(&func.item) {
            continue;
        }
        let types = &interns.types;
        // Quick skip: does this function mention any surviving `elem_def`?
        let active_defs: Vec<DefinitionId> = candidate_fields
            .keys()
            .copied()
            .filter(|&def| !disqualified.contains(&def) && func_mentions_def(func, types, def))
            .collect();
        if active_defs.is_empty() {
            continue;
        }
        let ctx = build_func_arena_context(
            func_idx,
            func,
            types,
            interns,
            &active_defs,
            &candidate_fields,
            &ctor_summaries,
            &mut disqualified,
            &mut write_sites,
        );
        per_func_info[func_idx] = Some(ctx);
    }

    candidate_fields.retain(|def, _| !disqualified.contains(def));
    if candidate_fields.is_empty() {
        return;
    }

    // Step 5: Greatest fixpoint over `(elem_def, field) -> ArenaFieldKind` and Stage-1
    // `ReturnArenaIndex` (non-circular: only proven from pre-push len, non-arena checks,
    // sentinel constants, and co-inductive arena field loads!).
    let mut field_state: BTreeMap<(DefinitionId, u32), ArenaFieldKind> = BTreeMap::new();
    for (&def, fields) in &candidate_fields {
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

    // Seed candidate `ReturnArenaIndex` summaries for internal functions returning unsigned scalar
    // that also take an arena parameter for one of the surviving `elem_def`s.
    let mut ret_idx_state: BTreeMap<ItemId, ReturnArenaIndex> = BTreeMap::new();
    for (func_idx, &(_, _, func, interns)) in live_items.iter().enumerate() {
        if trusted_std_bodies.contains(&func.item)
            || drop_glue_items.contains(&func.item)
            || ctor_summaries.contains_key(&func.item)
        {
            continue;
        }
        let types = &interns.types;
        if is_unsigned_index_ty(types, func.sig.result).is_none() {
            continue;
        }
        let Some(ctx) = per_func_info[func_idx].as_ref() else {
            continue;
        };
        // Find unique arena param per `elem_def` in this function.
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
        // Only track when the function has a single unambiguous arena param (avoids ambiguity).
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
            let Some(&func_idx) = by_item_id.get(&item_id) else {
                ret_idx_state.remove(&item_id);
                changed = true;
                continue;
            };
            let (_, _, func, interns) = live_items[func_idx];
            let Some(ctx) = per_func_info[func_idx].as_ref() else {
                ret_idx_state.remove(&item_id);
                changed = true;
                continue;
            };
            let handle = match cur.holder_field {
                Some(hf) => ArenaHandle::ParamHolder(cur.arena_param, hf),
                None => ArenaHandle::Param(cur.arena_param),
            };
            let next = verify_return_arena_index(
                func,
                &interns.types,
                ctx,
                cur,
                handle,
                &field_state,
                &ret_idx_state,
            );
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
        for site in &write_sites {
            let key = (site.elem_def, site.field);
            let Some(&cur_kind) = field_state.get(&key) else {
                continue;
            };
            let (_, _, func, interns) = live_items[site.func_idx];
            let Some(ctx) = per_func_info[site.func_idx].as_ref() else {
                field_state.remove(&key);
                changed = true;
                continue;
            };
            let class = if let Some(c) = site.const_override {
                ValueArenaClass::Const(c)
            } else {
                classify_value_for_arena(
                    func,
                    &interns.types,
                    ctx,
                    site.block,
                    site.op_idx,
                    site.value,
                    site.elem_def,
                    site.arena_handle,
                    site.pre_push_len,
                    site.local_post_check_ok,
                    &field_state,
                    &ret_idx_state,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ValueArenaClass {
    /// Proven strictly `< arena.len`.
    AlwaysValid,
    /// Constant `c` (valid as a sentinel if consistent with field's sentinel).
    Const(u128),
    /// Either `== sentinel` or `< arena.len`.
    ValidOrSentinel(u128),
    /// Unproven.
    Unknown,
}

fn merge_field_kind(
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

fn join_value_class(a: ValueArenaClass, b: ValueArenaClass) -> ValueArenaClass {
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
    let mut block_param_alias: BTreeMap<ValueId, ValueId> = BTreeMap::new();
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
                        block_param_alias.insert(tp, arg);
                    }
                }
                cur = *target;
            }
            _ => return None,
        }
    };
    let mut defs: BTreeMap<ValueId, &OpKind> = BTreeMap::new();
    let mut cell_store_val: BTreeMap<CellId, Option<ValueId>> = BTreeMap::new();
    let mut param_map: BTreeMap<ValueId, usize> = BTreeMap::new();
    for (idx, &p) in func.params.iter().enumerate() {
        param_map.insert(p, idx);
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
                defs.insert(res, &op.kind);
            }
        }
    }
    let peel = |mut v: ValueId| -> ValueId {
        for _ in 0..8 {
            if let Some(&src) = block_param_alias.get(&v) {
                v = src;
                continue;
            }
            match defs.get(&v) {
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
    } = defs.get(&peel(ret_v))?
    else {
        return None;
    };
    if adt_def_of(types, *ty) != Some(elem_def) {
        return None;
    }
    let mut out = Vec::with_capacity(fields.len());
    for &fv in fields {
        let p = peel(fv);
        if let Some(&pi) = param_map.get(&p) {
            out.push(CtorFieldSrc::Param(pi));
        } else {
            match defs.get(&p) {
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

fn func_mentions_def(func: &Func, types: &TypeTable, def: DefinitionId) -> bool {
    let mut seen = BTreeSet::new();
    func.values
        .iter()
        .any(|v| ty_reaches_def(types, v.ty, def, &mut seen))
        || func
            .cells
            .iter()
            .any(|c| ty_reaches_def(types, c.ty, def, &mut seen))
}

fn check_func_arena_preconditions(
    func: &Func,
    types: &TypeTable,
    interns: &Interns,
    by_item_id: &BTreeMap<ItemId, usize>,
    ctor_summaries: &BTreeMap<ItemId, ElemCtorSummary>,
    candidate_defs: &[DefinitionId],
    disqualified: &mut BTreeSet<DefinitionId>,
) {
    let active: Vec<DefinitionId> = candidate_defs
        .iter()
        .copied()
        .filter(|&def| !disqualified.contains(&def) && func_mentions_def(func, types, def))
        .collect();
    if active.is_empty() {
        return;
    }

    let is_ctor = ctor_summaries.contains_key(&func.item);

    // Track `CellStore` counts for cells holding `Vec<Elem>` (or containing `Vec<Elem>`).
    let mut vec_cell_stores: BTreeMap<(DefinitionId, CellId), usize> = BTreeMap::new();
    let doms = Dominators::of(func);
    // Detect blocks in loops (any block reachable from itself via a backedge).
    let in_loop = blocks_in_loops(func, &doms);

    let disq_def = |def: DefinitionId, _reason: &str, disq: &mut BTreeSet<DefinitionId>| {
        disq.insert(def);
    };

    // Check `[Elem; N]` arrays anywhere in values/cells (a fixed array coerced to `&[Elem]` would
    // not be backed by a monotone `Vec<Elem>`!).
    for &def in &active {
        let mut seen = BTreeSet::new();
        let has_fixed_arr = func
            .values
            .iter()
            .any(|v| ty_has_array_of_def(types, v.ty, def, &mut seen))
            || func
                .cells
                .iter()
                .any(|c| ty_has_array_of_def(types, c.ty, def, &mut seen));
        if has_fixed_arr {
            disq_def(def, "fixed_array", disqualified);
        }
    }

    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::Call {
                    callee,
                    args,
                    semantic,
                    ..
                } => {
                    // Check opaque/external/indirect/virtual/host calls touching any active `def`.
                    let is_trusted_std = matches!(semantic, CallSemantic::Standard(_));
                    let is_known_direct = matches!(
                        callee,
                        Callee::Direct { item, .. } if by_item_id.contains_key(item)
                    ) || matches!(callee, Callee::Nop);
                    let opaque_call = !is_trusted_std
                        && (!is_known_direct || !matches!(semantic, CallSemantic::Ordinary));

                    for &def in &active {
                        if disqualified.contains(&def) {
                            continue;
                        }
                        if opaque_call {
                            let mut seen = BTreeSet::new();
                            let touches = args.iter().any(|&a| {
                                ty_reaches_def(types, func.values[a.0 as usize].ty, def, &mut seen)
                            }) || op.result.is_some_and(|r| {
                                ty_reaches_def(types, func.values[r.0 as usize].ty, def, &mut seen)
                            });
                            if touches {
                                disq_def(def, "opaque_call", disqualified);
                                continue;
                            }
                        }

                        // If it's a `StandardOperation` on `Vec<Elem>`, enforce monotonicity!
                        if let CallSemantic::Standard(std_op) = semantic {
                            let mut touches_vec_of_def = false;
                            if let Some(res) = op.result {
                                if vec_elem_def(types, func.values[res.0 as usize].ty) == Some(def)
                                {
                                    touches_vec_of_def = true;
                                }
                            }
                            if let Some(&a0) = args.first() {
                                let a0_ty = func.values[a0.0 as usize].ty;
                                if vec_elem_def(types, a0_ty) == Some(def)
                                    || vec_ref_elem_def(types, a0_ty) == Some(def)
                                    || slice_elem_def(types, a0_ty) == Some(def)
                                {
                                    touches_vec_of_def = true;
                                }
                            }
                            if touches_vec_of_def && !is_monotone_arena_vec_op(*std_op) {
                                disq_def(def, "non_monotone_vec_op", disqualified);
                                continue;
                            }
                        }
                    }

                    // If calling a pure constructor helper, verify its return value is ONLY used
                    // by `VecPush` / `NativeVecPush` (never stored elsewhere or returned).
                    if let Callee::Direct { item, .. } = callee {
                        if let Some(ctor) = ctor_summaries.get(item) {
                            if active.contains(&ctor.elem_def)
                                && !disqualified.contains(&ctor.elem_def)
                            {
                                if let Some(res) = op.result {
                                    if !value_flows_only_to_vec_push(func, res) {
                                        disq_def(ctor.elem_def, "ctor_flow", disqualified);
                                    }
                                }
                            }
                        }
                    }
                }
                OpKind::Host { args, .. } => {
                    for &def in &active {
                        if disqualified.contains(&def) {
                            continue;
                        }
                        let mut seen = BTreeSet::new();
                        let touches = args.iter().any(|&a| {
                            ty_reaches_def(types, func.values[a.0 as usize].ty, def, &mut seen)
                        }) || op.result.is_some_and(|r| {
                            ty_reaches_def(types, func.values[r.0 as usize].ty, def, &mut seen)
                        });
                        if touches {
                            disq_def(def, "host_op", disqualified);
                        }
                    }
                }
                OpKind::Intrinsic {
                    name,
                    types: ity,
                    args,
                    ..
                } => {
                    let benign_meta = matches!(
                        interns.str(*name),
                        Some("size_of" | "min_align_of" | "align_of" | "needs_drop")
                    );
                    if !benign_meta {
                        for &def in &active {
                            if disqualified.contains(&def) {
                                continue;
                            }
                            let mut seen = BTreeSet::new();
                            let touches = args.iter().any(|&a| {
                                ty_reaches_def(types, func.values[a.0 as usize].ty, def, &mut seen)
                            }) || op.result.is_some_and(|r| {
                                ty_reaches_def(types, func.values[r.0 as usize].ty, def, &mut seen)
                            }) || ity
                                .iter()
                                .any(|&t| ty_reaches_def(types, t, def, &mut seen));
                            if touches {
                                disq_def(def, "intrinsic", disqualified);
                            }
                        }
                    }
                }
                OpKind::Cast { kind, from, to, .. } => {
                    let allowed_cast =
                        matches!(kind, CastKind::Subtype | CastKind::PointerCoercion)
                            && !value::is_raw(types, *from)
                            && !value::is_raw(types, *to)
                            && !matches!(
                                value::pointee_of(types, *to)
                                    .and_then(|pt| types.get(types.peeled(pt)))
                                    .map(|d| &d.kind),
                                Some(TypeKind::Dynamic { .. })
                            );
                    if !allowed_cast {
                        for &def in &active {
                            if disqualified.contains(&def) {
                                continue;
                            }
                            let mut seen = BTreeSet::new();
                            if ty_reaches_def(types, *from, def, &mut seen)
                                || ty_reaches_def(types, *to, def, &mut seen)
                            {
                                disq_def(def, "cast", disqualified);
                            }
                        }
                    }
                }
                OpKind::RawView { root, path, .. } => {
                    // Reject ANY raw pointer into `Elem`, `[Elem]`, or `Vec<Elem>`.
                    for &def in &active {
                        if disqualified.contains(&def) {
                            continue;
                        }
                        if op.result.is_some_and(|r| {
                            let mut seen = BTreeSet::new();
                            ty_reaches_def(types, func.values[r.0 as usize].ty, def, &mut seen)
                        }) || root_or_path_mentions_elem_or_vec(func, types, root, path, def)
                        {
                            disq_def(def, "raw_view", disqualified);
                        }
                    }
                }
                OpKind::SliceView { .. } => {
                    // Arbitrary `SliceView` on `[Elem]` could subslice or fabricate a shorter
                    // slice length -> disqualify!
                    if let Some(res) = op.result {
                        if let Some(def) = slice_elem_def(types, func.values[res.0 as usize].ty) {
                            if active.contains(&def) {
                                disq_def(def, "slice_view", disqualified);
                            }
                        }
                    }
                }
                OpKind::BorrowView { root, path, .. } => {
                    // Check for subslicing `[Elem]` (`Proj::Subslice`) or mutable borrow of
                    // `Vec<Elem>` internals (`.len` / `.buf`).
                    for &def in &active {
                        if disqualified.contains(&def) {
                            continue;
                        }
                        if borrow_subslice_or_vec_internal(func, types, interns, root, path, def) {
                            disq_def(def, "borrow_subslice_or_vec_internal", disqualified);
                        }
                    }
                }
                OpKind::SetField { base, field, .. } => {
                    let base_ty = func.values[base.0 as usize].ty;
                    for &def in &active {
                        if disqualified.contains(&def) {
                            continue;
                        }
                        // Reject mutating `Vec<Elem>` fields directly or overwriting any field
                        // containing `Vec<Elem>`.
                        if vec_elem_def(types, base_ty) == Some(def) {
                            disq_def(def, "set_field_vec", disqualified);
                            continue;
                        }
                        if let Some(fd) = types.field(base_ty, None, *field) {
                            let mut seen = BTreeSet::new();
                            if ty_contains_vec_of_def(types, fd.ty, def, &mut seen) {
                                disq_def(def, "set_field_holder", disqualified);
                            }
                        }
                    }
                }
                OpKind::StoreThrough { ptr, .. } => {
                    if let Some(pt) = value::pointee_of(types, func.values[ptr.0 as usize].ty) {
                        for &def in &active {
                            if disqualified.contains(&def) {
                                continue;
                            }
                            let mut seen = BTreeSet::new();
                            // Overwriting a `Vec<Elem>` (or struct containing `Vec<Elem>`) through
                            // a pointer could shrink/replace the arena!
                            // Also overwriting a whole `Elem` through `&mut Elem` (unless we prove
                            // it's a verified aggregate — fail closed for whole-Elem overwrite!).
                            if ty_contains_vec_of_def(types, pt, def, &mut seen)
                                || adt_def_of(types, pt) == Some(def)
                            {
                                disq_def(def, "store_through_vec_or_elem", disqualified);
                            }
                        }
                    }
                }
                OpKind::CellStore { cell, .. } => {
                    let cty = func.cells[cell.0 as usize].ty;
                    for &def in &active {
                        if disqualified.contains(&def) {
                            continue;
                        }
                        let mut seen = BTreeSet::new();
                        if ty_contains_vec_of_def(types, cty, def, &mut seen) {
                            let cnt = vec_cell_stores.entry((def, *cell)).or_insert(0);
                            *cnt += 1;
                            if *cnt > 1 || in_loop.contains(&block.id) {
                                disq_def(def, "cell_store_vec_overwrite", disqualified);
                            }
                        }
                    }
                }
                OpKind::Aggregate { ty, .. } => {
                    if let Some(def) = adt_def_of(types, *ty) {
                        if active.contains(&def) && !disqualified.contains(&def) && !is_ctor {
                            // Fresh `Elem` aggregate outside a pure constructor MUST flow only
                            // directly into `Vec<Elem>::push`!
                            if let Some(res) = op.result {
                                if !value_flows_only_to_vec_push(func, res) {
                                    disq_def(def, "aggregate_flow", disqualified);
                                }
                            }
                        }
                    }
                }
                OpKind::Const(tree) => {
                    if let Some(res) = op.result {
                        let rty = func.values[res.0 as usize].ty;
                        for &def in &active {
                            if disqualified.contains(&def) {
                                continue;
                            }
                            let mut seen = BTreeSet::new();
                            if ty_reaches_def(types, rty, def, &mut seen)
                                && !matches!(tree, ConstTree::Unit)
                            {
                                disq_def(def, "const_elem", disqualified);
                            }
                        }
                    }
                }
                OpKind::Repeat { .. } => {
                    if let Some(res) = op.result {
                        let rty = func.values[res.0 as usize].ty;
                        for &def in &active {
                            if disqualified.contains(&def) {
                                continue;
                            }
                            let mut seen = BTreeSet::new();
                            if ty_reaches_def(types, rty, def, &mut seen) {
                                disq_def(def, "repeat_elem", disqualified);
                            }
                        }
                    }
                }
                _ => {}
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
            if dominates_block(doms, succ, b.id) {
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

fn dominates_block(doms: &Dominators, a: RBlockId, b: RBlockId) -> bool {
    let mut at = b;
    loop {
        if at == a {
            return true;
        }
        match doms.idom.get(at.0 as usize).copied().flatten() {
            Some(up) if up != at => at = up,
            _ => return false,
        }
    }
}

fn ty_has_array_of_def(
    table: &TypeTable,
    ty: TypeId,
    target: DefinitionId,
    seen: &mut BTreeSet<TypeId>,
) -> bool {
    let peeled = table.peeled(ty);
    if !seen.insert(peeled) {
        return false;
    }
    let Some(desc) = table.get(peeled) else {
        return false;
    };
    match &desc.kind {
        TypeKind::Array { element, .. } => {
            let mut inner_seen = BTreeSet::new();
            ty_reaches_def(table, *element, target, &mut inner_seen)
        }
        TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
            ty_has_array_of_def(table, *pointee, target, seen)
        }
        TypeKind::Slice(element) => ty_has_array_of_def(table, *element, target, seen),
        TypeKind::Tuple(elems) => elems
            .iter()
            .any(|&e| ty_has_array_of_def(table, e, target, seen)),
        TypeKind::Adt { variants, .. } => variants.iter().any(|v| {
            v.fields
                .iter()
                .any(|f| ty_has_array_of_def(table, f.ty, target, seen))
        }),
        _ => false,
    }
}

fn ty_contains_vec_of_def(
    table: &TypeTable,
    ty: TypeId,
    target: DefinitionId,
    seen: &mut BTreeSet<TypeId>,
) -> bool {
    let peeled = table.peeled(ty);
    if !seen.insert(peeled) {
        return false;
    }
    if vec_elem_def(table, peeled) == Some(target) {
        return true;
    }
    let Some(desc) = table.get(peeled) else {
        return false;
    };
    match &desc.kind {
        TypeKind::Array { element, .. } | TypeKind::Slice(element) => {
            ty_contains_vec_of_def(table, *element, target, seen)
        }
        TypeKind::Tuple(elems) => elems
            .iter()
            .any(|&e| ty_contains_vec_of_def(table, e, target, seen)),
        TypeKind::Adt { variants, .. } => variants.iter().any(|v| {
            v.fields
                .iter()
                .any(|f| ty_contains_vec_of_def(table, f.ty, target, seen))
        }),
        TypeKind::Closure { upvars, .. } | TypeKind::CoroutineClosure { upvars, .. } => upvars
            .iter()
            .any(|&u| ty_contains_vec_of_def(table, u, target, seen)),
        _ => false,
    }
}

fn root_or_path_mentions_elem_or_vec(
    func: &Func,
    types: &TypeTable,
    root: &Root,
    path: &[Proj],
    def: DefinitionId,
) -> bool {
    let mut cur = match root {
        Root::Value(v) => func.values[v.0 as usize].ty,
        Root::Cell(c) => func.cells[c.0 as usize].ty,
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
    let mut cur = match root {
        Root::Value(v) => func.values[v.0 as usize].ty,
        Root::Cell(c) => func.cells[c.0 as usize].ty,
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
    let mut cell_store_count: BTreeMap<CellId, usize> = BTreeMap::new();
    let mut cell_borrowed: BTreeSet<CellId> = BTreeSet::new();
    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::CellStore { cell, .. } => {
                    *cell_store_count.entry(*cell).or_insert(0) += 1;
                }
                OpKind::BorrowView {
                    root: Root::Cell(c),
                    ..
                }
                | OpKind::RawView {
                    root: Root::Cell(c),
                    ..
                } => {
                    cell_borrowed.insert(*c);
                }
                _ => {}
            }
        }
    }

    let mut tracked = BTreeSet::new();
    let mut tracked_cells: BTreeSet<CellId> = BTreeSet::new();
    tracked.insert(root_v);
    let mut changed = true;
    while changed {
        changed = false;
        for block in &func.blocks {
            for op in &block.ops {
                match (&op.kind, op.result) {
                    (OpKind::Move(src) | OpKind::Copy(src), Some(res)) => {
                        if tracked.contains(src) && tracked.insert(res) {
                            changed = true;
                        }
                    }
                    (OpKind::CellStore { cell, v }, _) => {
                        if tracked.contains(v)
                            && cell_store_count.get(cell).copied() == Some(1)
                            && !cell_borrowed.contains(cell)
                            && tracked_cells.insert(*cell)
                        {
                            changed = true;
                        }
                    }
                    (OpKind::CellLoad(cell), Some(res))
                        if tracked_cells.contains(cell) && tracked.insert(res) =>
                    {
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
                    if tracked.contains(&arg) {
                        if let Some(&param) = func.blocks[target.0 as usize].params.get(pos) {
                            if tracked.insert(param) {
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
                if tracked.contains(scrut) {
                    return false;
                }
            }
            _ => {
                for u in block.term.uses() {
                    if tracked.contains(&u) {
                        return false;
                    }
                }
            }
        }
        for op in &block.ops {
            for u in op.kind.uses() {
                if !tracked.contains(&u) {
                    continue;
                }
                match &op.kind {
                    OpKind::Move(_) | OpKind::Copy(_) => {}
                    OpKind::CellStore { cell, .. } if tracked_cells.contains(cell) => {}
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

type CallRetArenaMap =
    BTreeMap<ValueId, (ItemId, Vec<(usize, Option<u32>, DefinitionId, ArenaHandle)>)>;
type WorklistPushes = BTreeMap<(CellId, Option<u32>), Vec<(RBlockId, usize, ValueId)>>;
type ParamArenaKey = (ItemId, usize, usize, Option<u32>, DefinitionId);
type ParamArenaCandidateMap = BTreeMap<ParamArenaKey, Option<u128>>;
type ParamCallSiteMap = BTreeMap<ItemId, Vec<(usize, RBlockId, usize, Vec<ValueId>)>>;

/// Per-function tracking of arena handles, slice/vec derivations, length SSA values, field loads,
/// non-arena `< arena.len` facts, and local worklists.
struct FuncArenaContext {
    doms: Dominators,
    /// Defining `(block, op_idx)` per `ValueId`.
    defs: Vec<Option<(RBlockId, usize)>>,
    /// Value numbering representative (peeling `Copy`/`Move` and same-width unsigned casts).
    canon: Vec<ValueId>,
    /// Which `(elem_def, ArenaHandle)` a `Vec<Elem>`, `&Vec<Elem>`, `&mut Vec<Elem>`, `&[Elem]`,
    /// or `&mut [Elem]` SSA value refers to.
    val_arena: BTreeMap<ValueId, (DefinitionId, ArenaHandle)>,
    /// Which `(elem_def, ArenaHandle)` a `usize`/`u32` SSA value is the exact `.len` of, along
    /// with its defining `(block, op_idx)`.
    len_of_arena_defs: BTreeMap<ValueId, (RBlockId, usize, DefinitionId, ArenaHandle)>,
    /// All `VecPush` sites `(block, op_idx, elem_def, handle)` on monotone arenas in this function.
    push_sites: Vec<(RBlockId, usize, DefinitionId, ArenaHandle)>,
    /// Counted `0..N` single-push loop induction variables `(push_block, push_op_idx, iv_canon, elem_def, handle)`.
    counted_push_ivs: Vec<(RBlockId, usize, ValueId, DefinitionId, ArenaHandle)>,
    /// Which `(elem_def, field, ArenaHandle)` an SSA value was loaded from (scalar field or element
    /// of a `Vec<Idx>` field).
    loaded_from: BTreeMap<ValueId, (DefinitionId, u32, ArenaHandle, bool)>,
    /// Which `(elem_def, ArenaHandle)` an SSA value is a pre-push length of (after push completes).
    pre_push_valid: BTreeMap<ValueId, (DefinitionId, ArenaHandle)>,
    /// Direct calls returning a value that might be a `ReturnArenaIndex`:
    /// `result_val -> (callee_item, arena_handle_at_arg_pos)`.
    call_ret_arenas: CallRetArenaMap,
    /// Local worklist `Vec<T>` cell (like `todo: Vec<(JsValue, usize)>` in `nested`) whose popped
    /// scalar/tuple-field values are bounded by the set of values pushed into that field.
    worklist_field_pushes: WorklistPushes,
    /// Values extracted from a local worklist `Vec` pop: `val -> (cell, tuple_field)`.
    worklist_popped: BTreeMap<ValueId, (CellId, Option<u32>)>,
    /// All distinct `ArenaHandle`s seen per `elem_def` in this function.
    arenas_per_def: BTreeMap<DefinitionId, BTreeSet<ArenaHandle>>,
    /// Non-arena `< arena.len` facts: `(block, op_idx_or_0, value_canon, elem_def, handle)`.
    /// Established by `Assert(v < arena_len)` or `Switch` edge `v < arena_len` or `v != sentinel`.
    lt_arena_facts: Vec<(RBlockId, Option<usize>, ValueId, DefinitionId, ArenaHandle)>,
    /// `v != c` facts established on a single-predecessor switch edge into `block`.
    ne_const_facts: Vec<(RBlockId, ValueId, u128)>,
    /// `v == c` facts established on a single-predecessor switch edge into `block`.
    eq_const_facts: Vec<(RBlockId, ValueId, u128)>,
    /// Block-param incoming edge arguments `(block, param_pos) -> Vec<(pred_block, arg_val)>`.
    param_incoming: BTreeMap<ValueId, Vec<(RBlockId, ValueId)>>,
}

impl FuncArenaContext {
    fn has_multiple_arenas_of(&self, def: DefinitionId) -> bool {
        self.arenas_per_def.get(&def).is_some_and(|s| s.len() > 1)
    }

    fn canon(&self, v: ValueId) -> ValueId {
        self.canon[v.0 as usize]
    }
}

#[allow(clippy::too_many_arguments)]
fn build_func_arena_context(
    func_idx: usize,
    func: &Func,
    types: &TypeTable,
    interns: &Interns,
    active_defs: &[DefinitionId],
    candidate_fields: &BTreeMap<DefinitionId, BTreeMap<u32, CandidateFieldKind>>,
    ctor_summaries: &BTreeMap<ItemId, ElemCtorSummary>,
    disqualified: &mut BTreeSet<DefinitionId>,
    write_sites: &mut Vec<FieldWriteSite>,
) -> FuncArenaContext {
    let n = func.values.len();
    let doms = Dominators::of(func);
    let mut defs = vec![None; n];
    let mut canon: Vec<ValueId> = (0..n as u32).map(ValueId).collect();
    for block in &func.blocks {
        for (i, op) in block.ops.iter().enumerate() {
            if let Some(res) = op.result {
                defs[res.0 as usize] = Some((block.id, i));
            }
        }
    }
    // Canonicalize `Copy`/`Move` and same-width unsigned `Cast(IntToInt)` in RPO.
    for &b in &doms.rpo {
        for op in &func.blocks[b.0 as usize].ops {
            let Some(res) = op.result else { continue };
            match &op.kind {
                OpKind::Copy(src) | OpKind::Move(src) => {
                    canon[res.0 as usize] = canon[src.0 as usize];
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
                            canon[res.0 as usize] = canon[v.0 as usize];
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let mut param_incoming: BTreeMap<ValueId, Vec<(RBlockId, ValueId)>> = BTreeMap::new();
    for block in &func.blocks {
        for (target, args) in block.term.edges() {
            if target.0 == 0 {
                continue;
            }
            for (pos, &arg) in args.iter().enumerate() {
                if let Some(&param) = func.blocks[target.0 as usize].params.get(pos) {
                    param_incoming
                        .entry(param)
                        .or_default()
                        .push((block.id, arg));
                }
            }
        }
    }

    let mut val_arena: BTreeMap<ValueId, (DefinitionId, ArenaHandle)> = BTreeMap::new();
    let mut len_of_arena: BTreeMap<ValueId, (DefinitionId, ArenaHandle)> = BTreeMap::new();
    let mut arenas_per_def: BTreeMap<DefinitionId, BTreeSet<ArenaHandle>> = BTreeMap::new();

    // Seed entry params.
    for (p_idx, &p_val) in func.params.iter().enumerate() {
        let p_ty = func.values[p_val.0 as usize].ty;
        if let Some(def) = slice_elem_def(types, p_ty)
            .or_else(|| vec_ref_elem_def(types, p_ty))
            .or_else(|| vec_elem_def(types, p_ty))
        {
            if active_defs.contains(&def) {
                let h = ArenaHandle::Param(p_idx);
                val_arena.insert(p_val, (def, h));
                arenas_per_def.entry(def).or_default().insert(h);
            }
        }
        for &def in active_defs {
            for (_, hf) in holder_vec_fields(types, p_ty, def) {
                let h = ArenaHandle::ParamHolder(p_idx, hf);
                arenas_per_def.entry(def).or_default().insert(h);
            }
        }
    }
    // Seed local cells holding `Vec<Elem>` or holder structs.
    let mut local_vec_cells: BTreeMap<CellId, DefinitionId> = BTreeMap::new();
    for (c_idx, cell) in func.cells.iter().enumerate() {
        let cid = CellId(c_idx as u32);
        if let Some(def) = vec_elem_def(types, cell.ty) {
            if active_defs.contains(&def) {
                local_vec_cells.insert(cid, def);
                arenas_per_def
                    .entry(def)
                    .or_default()
                    .insert(ArenaHandle::VecCell(cid));
            }
        }
        for &def in active_defs {
            for (_, hf) in holder_vec_fields(types, cell.ty, def) {
                arenas_per_def
                    .entry(def)
                    .or_default()
                    .insert(ArenaHandle::HolderCell(cid, hf));
            }
        }
    }

    // Track holder struct references (`&Holder`, `&mut Holder`, `Holder`) through reborrows &
    // copies so inlined `&mut self` accesses like `(*reborrowed_self).nodes` resolve cleanly.
    let mut holder_ref: BTreeMap<ValueId, HolderRoot> = BTreeMap::new();
    for (p_idx, &p_val) in func.params.iter().enumerate() {
        let p_ty = func.values[p_val.0 as usize].ty;
        if active_defs
            .iter()
            .any(|&def| !holder_vec_fields(types, p_ty, def).is_empty())
        {
            holder_ref.insert(p_val, HolderRoot::Param(p_idx));
        }
    }

    // Track `BorrowView` to an arena element `&Elem`/`&mut Elem` and to an arena element field
    // `&Elem.f`/`&mut Elem.f` (including `&mut Elem.vec_field`).
    let mut elem_ref_arena: BTreeMap<ValueId, (DefinitionId, ArenaHandle)> = BTreeMap::new();
    let mut field_ref_arena: BTreeMap<ValueId, (DefinitionId, u32, ArenaHandle)> = BTreeMap::new();
    let mut inner_vec_slice_arena: BTreeMap<ValueId, (DefinitionId, u32, ArenaHandle)> =
        BTreeMap::new();
    let mut loaded_from: BTreeMap<ValueId, (DefinitionId, u32, ArenaHandle, bool)> =
        BTreeMap::new();
    let mut pre_push_valid: BTreeMap<ValueId, (DefinitionId, ArenaHandle)> = BTreeMap::new();
    let mut call_ret_arenas: CallRetArenaMap = BTreeMap::new();

    // Propagate arena handles, lengths, element refs, and field refs in RPO (3 rounds for block params).
    for _ in 0..3 {
        for &b in &doms.rpo {
            let block = &func.blocks[b.0 as usize];
            if b.0 != 0 {
                for &param in &block.params {
                    if let Some(inc) = param_incoming.get(&param) {
                        let mut first: Option<(DefinitionId, ArenaHandle)> = None;
                        let mut same = !inc.is_empty();
                        let mut first_hr: Option<HolderRoot> = None;
                        let mut same_hr = !inc.is_empty();
                        for &(_, arg) in inc {
                            if arg == param {
                                continue;
                            }
                            if let Some(&h) = val_arena.get(&arg) {
                                match first {
                                    None => first = Some(h),
                                    Some(prev) if prev == h => {}
                                    Some(_) => same = false,
                                }
                            } else {
                                same = false;
                            }
                            if let Some(&hr) = holder_ref.get(&arg) {
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
                                val_arena.insert(param, h);
                            }
                        }
                        if same_hr {
                            if let Some(hr) = first_hr {
                                holder_ref.insert(param, hr);
                            }
                        }
                    }
                }
            }
            for op in &block.ops {
                let Some(res) = op.result else { continue };
                let rty = func.values[res.0 as usize].ty;
                match &op.kind {
                    OpKind::Copy(src) | OpKind::Move(src) => {
                        if let Some(&hr) = holder_ref.get(src) {
                            holder_ref.insert(res, hr);
                        }
                        if let Some(&h) = val_arena.get(src) {
                            val_arena.insert(res, h);
                        }
                        if let Some(&h) = len_of_arena.get(src) {
                            len_of_arena.insert(res, h);
                        }
                        if let Some(&h) = elem_ref_arena.get(src) {
                            elem_ref_arena.insert(res, h);
                        }
                        if let Some(&h) = field_ref_arena.get(src) {
                            field_ref_arena.insert(res, h);
                        }
                        if let Some(&h) = inner_vec_slice_arena.get(src) {
                            inner_vec_slice_arena.insert(res, h);
                        }
                        if let Some(&h) = loaded_from.get(src) {
                            loaded_from.insert(res, h);
                        }
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
                                if let Some(&h) = len_of_arena.get(v) {
                                    len_of_arena.insert(res, h);
                                }
                                if let Some(&h) = loaded_from.get(v) {
                                    loaded_from.insert(res, h);
                                }
                            }
                        }
                    }
                    OpKind::Cast {
                        kind: CastKind::Subtype | CastKind::PointerCoercion,
                        v,
                        ..
                    } => {
                        if let Some(&hr) = holder_ref.get(v) {
                            holder_ref.insert(res, hr);
                        }
                        if let Some(&h) = val_arena.get(v) {
                            val_arena.insert(res, h);
                        }
                        if let Some(&h) = elem_ref_arena.get(v) {
                            elem_ref_arena.insert(res, h);
                        }
                        if let Some(&h) = field_ref_arena.get(v) {
                            field_ref_arena.insert(res, h);
                        }
                    }
                    OpKind::CellLoad(c) => {
                        if let Some(&def) = local_vec_cells.get(c) {
                            val_arena.insert(res, (def, ArenaHandle::VecCell(*c)));
                        }
                    }
                    OpKind::Field { base, field, .. } => {
                        let bty = func.values[base.0 as usize].ty;
                        if is_direct_global_vec_ty(types, bty)
                            && is_vec_len_field(types, interns, bty, *field)
                        {
                            if let Some(&h) = val_arena.get(base) {
                                len_of_arena.insert(res, h);
                            }
                        } else if let Some(def) = adt_def_of(types, bty) {
                            if let Some(&h) = elem_ref_arena.get(base) {
                                if h.0 == def {
                                    loaded_from.insert(res, (def, *field, h.1, false));
                                }
                            }
                        }
                        // Holder struct value field `holder.nodes`.
                        if let Some(def) = vec_elem_def(types, rty) {
                            if active_defs.contains(&def) {
                                if let Some(&hr) = holder_ref.get(base) {
                                    let h = match hr {
                                        HolderRoot::Param(pi) => {
                                            ArenaHandle::ParamHolder(pi, *field)
                                        }
                                        HolderRoot::Cell(c) => ArenaHandle::HolderCell(c, *field),
                                    };
                                    val_arena.insert(res, (def, h));
                                }
                            }
                        }
                    }
                    OpKind::Unary {
                        op: UnaryOp::PtrMetadata,
                        v,
                        ..
                    } => {
                        if let Some(&h) = val_arena.get(v) {
                            len_of_arena.insert(res, h);
                        }
                    }
                    OpKind::Deref(ptr) => {
                        if let Some(&h) = val_arena.get(ptr) {
                            val_arena.insert(res, h);
                        } else if let Some(&h) = elem_ref_arena.get(ptr) {
                            elem_ref_arena.insert(res, h);
                        } else if let Some(&(def, field, handle)) = field_ref_arena.get(ptr) {
                            // Reading a scalar field through `&Elem.f` or reading a `usize` element
                            // through `&Elem.children[i]`.
                            let is_vec_elem = candidate_fields
                                .get(&def)
                                .and_then(|m| m.get(&field))
                                .is_some_and(|k| matches!(k, CandidateFieldKind::VecIdx { .. }));
                            loaded_from.insert(res, (def, field, handle, is_vec_elem));
                        } else if let Some(pt) =
                            value::pointee_of(types, func.values[ptr.0 as usize].ty)
                        {
                            // Deref of `&Vec<Elem>.len` -> length of that arena!
                            if let Some((def, handle)) = deref_vec_len_borrow(
                                func,
                                types,
                                interns,
                                &defs,
                                *ptr,
                                &val_arena,
                                &holder_ref,
                            ) {
                                len_of_arena.insert(res, (def, handle));
                            } else if let Some((def, field, handle)) = deref_elem_field_borrow(
                                func,
                                types,
                                &defs,
                                *ptr,
                                &val_arena,
                                &elem_ref_arena,
                                &inner_vec_slice_arena,
                            ) {
                                let is_vec_elem = candidate_fields
                                    .get(&def)
                                    .and_then(|m| m.get(&field))
                                    .is_some_and(|k| {
                                        matches!(k, CandidateFieldKind::VecIdx { .. })
                                    });
                                loaded_from.insert(res, (def, field, handle, is_vec_elem));
                            }
                            let _ = pt;
                        }
                    }
                    OpKind::BorrowView { root, path, .. } => {
                        classify_borrow_view(
                            func,
                            types,
                            active_defs,
                            res,
                            root,
                            path,
                            &local_vec_cells,
                            &mut holder_ref,
                            &mut val_arena,
                            &mut elem_ref_arena,
                            &mut field_ref_arena,
                            &mut inner_vec_slice_arena,
                            &mut arenas_per_def,
                        );
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
                            if let Some(def) = vec_elem_def(types, rty) {
                                if active_defs.contains(&def) {
                                    let h = ArenaHandle::VecValue(res);
                                    val_arena.insert(res, (def, h));
                                    arenas_per_def.entry(def).or_default().insert(h);
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
                                if let Some(&h) = val_arena.get(&a0) {
                                    val_arena.insert(res, h);
                                } else if let Some(&(def, field, handle)) = field_ref_arena.get(&a0)
                                {
                                    inner_vec_slice_arena.insert(res, (def, field, handle));
                                }
                            }
                        }
                        CallSemantic::Standard(StandardOperation::VecLen) => {
                            if let Some(&a0) = args.first() {
                                if let Some(&h) = val_arena.get(&a0) {
                                    len_of_arena.insert(res, h);
                                }
                            }
                        }
                        CallSemantic::Standard(
                            StandardOperation::VecIndex | StandardOperation::VecIndexMut,
                        ) => {
                            if let Some(&a0) = args.first() {
                                if let Some(&h) = val_arena.get(&a0) {
                                    elem_ref_arena.insert(res, h);
                                } else if let Some(&(def, field, handle)) = field_ref_arena.get(&a0)
                                {
                                    // `&nodes[v].children[i]` -> reference to element of `Vec<Idx>` field!
                                    field_ref_arena.insert(res, (def, field, handle));
                                }
                            }
                        }
                        CallSemantic::Standard(
                            StandardOperation::VecIter | StandardOperation::VecIterMut,
                        ) => {
                            if let Some(&a0) = args.first() {
                                if let Some(&(def, field, handle)) = field_ref_arena.get(&a0) {
                                    inner_vec_slice_arena.insert(res, (def, field, handle));
                                }
                            }
                        }
                        CallSemantic::Ordinary => {
                            if let Callee::Direct { item, .. } = callee {
                                if is_unsigned_index_ty(types, rty).is_some() {
                                    let mut arg_arenas = Vec::new();
                                    for (ai, &av) in args.iter().enumerate() {
                                        if let Some(&(def, handle)) = val_arena.get(&av) {
                                            arg_arenas.push((ai, None, def, handle));
                                        }
                                        // Check holder parameter passed directly (`&mut self`).
                                        if let Some((def, hf, handle)) =
                                            resolve_holder_arg(func, types, &defs, av, active_defs)
                                        {
                                            arg_arenas.push((ai, Some(hf), def, handle));
                                        }
                                    }
                                    if !arg_arenas.is_empty() {
                                        call_ret_arenas.insert(res, (*item, arg_arenas));
                                    }
                                }
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
    }

    // Track `slice::Iter` / `Rev<slice::Iter>` over an inner `Vec<Idx>` field (e.g.
    // `for &c in &nodes[v].children` or `nodes[v].children.iter().rev()`):
    // Any `Deref` of a `BorrowView([Deref, Index(_)])` on `inner_vec_slice_arena` is already
    // captured above; also capture `CellLoad` / `Field` from an iterator created from
    // `inner_vec_slice_arena`? Wait: before `slice_iter` runs (we run BEFORE `opt/pipeline`),
    // `for &c in &nodes[v].children` uses `core::slice::iter::Iter::next`!
    // Wait — in `d3-hierarchy`, `nodes[v].children.iter()` calls `core::slice::Iter::next`, which
    // returns `Option<&usize>`! In `arena.rs`, `d3-hierarchy::Node.children` writes only happen
    // via `nodes[parent].children.push(pos)` and `nodes[p].children.push(i)` (NEITHER reads from
    // `Iter::next`!). Where IS `nodes[v].children.iter()` used?
    // 1. In `walk`: `for &c in &nodes[v].children { stack.push(c); }` (Stage 2 `return_vec_elem_bound`!).
    // 2. In `tidy::layout`: `for &c in children.iter().rev() { t[c]... }` (`opt/range.rs`, which
    //    runs AFTER `slice_iter` has lowered the iterator into direct `children[cursor]` indexing!!).
    // OH! What a realization: `opt/range.rs` runs AFTER `opt/slice_iter.rs` has ALREADY lowered
    // `children.iter()` and `children.iter().rev()` into direct `BorrowView([Deref, Index(cursor)])`
    // + `Deref`! And for `walk` (which runs BEFORE `opt/slice_iter`), we can inspect `Iter`
    // provenance in `analyze_return_vec_elem_bounds`!

    // Also track local worklist `Vec` cells (e.g. `todo: Vec<(JsValue, usize)>` in `nested`).
    let (worklist_field_pushes, worklist_popped) =
        collect_local_worklists(func, types, &defs, &canon);

    // Collect non-arena `< arena.len` facts, `!= const`, and `== const` facts.
    let mut lt_arena_facts = Vec::new();
    let mut ne_const_facts = Vec::new();
    let mut eq_const_facts = Vec::new();
    collect_local_arena_facts(
        func,
        types,
        &defs,
        &canon,
        &val_arena,
        &len_of_arena,
        &mut lt_arena_facts,
        &mut ne_const_facts,
        &mut eq_const_facts,
    );

    // Determine which local `Vec<Elem>` cells are unescaped (not passed to opaque calls, not
    // returned before the write, and `Elem` has no custom Drop glue).
    let local_post_check_cells: BTreeSet<CellId> = local_vec_cells
        .iter()
        .filter_map(|(&cid, &_def)| {
            let elem_ty = global_vec_elem_ty(types, func.cells[cid.0 as usize].ty)?;
            // `Elem` has no custom Drop that reads scalar fields if dropping it only drops
            // built-in `String`/`Vec`/primitives, OR simply `layout.needs_drop == false` or
            // every field of `Elem` is primitive/`String`/`Vec<primitive>`/`JsValue`.
            if elem_drop_cannot_read_fields(types, elem_ty) {
                Some(cid)
            } else {
                None
            }
        })
        .collect();

    let mut push_sites: Vec<(RBlockId, usize, DefinitionId, ArenaHandle)> = Vec::new();
    let mut counted_push_ivs: Vec<(RBlockId, usize, ValueId, DefinitionId, ArenaHandle)> =
        Vec::new();
    let mut len_of_arena_defs: BTreeMap<ValueId, (RBlockId, usize, DefinitionId, ArenaHandle)> =
        BTreeMap::new();
    for (&lv, &(def, handle)) in &len_of_arena {
        if let Some((lb, li)) = defs[lv.0 as usize] {
            len_of_arena_defs
                .entry(canon[lv.0 as usize])
                .or_insert((lb, li, def, handle));
        }
    }
    // Propagate `len_of_arena_defs` across block parameters whose every incoming edge carries
    // a `.len()` read of the SAME monotone arena `(def, handle)` (e.g. merged push blocks in
    // `earcut::linked_list`): on every path entering `b`, `param <= arena.len()@(b, 0)`.
    for _ in 0..2 {
        for &b in &doms.rpo {
            if b.0 == 0 {
                continue;
            }
            for &param in &func.blocks[b.0 as usize].params {
                let pc = canon[param.0 as usize];
                if len_of_arena_defs.contains_key(&pc) {
                    continue;
                }
                let Some(inc) = param_incoming.get(&param) else {
                    continue;
                };
                if inc.is_empty() {
                    continue;
                }
                let mut target: Option<(DefinitionId, ArenaHandle)> = None;
                let mut all_ok = true;
                for &(pred_b, arg) in inc {
                    let ac = canon[arg.0 as usize];
                    if ac == pc {
                        continue;
                    }
                    if let Some(&(lb, _li, def, handle)) = len_of_arena_defs.get(&ac) {
                        if dominates_block(&doms, lb, pred_b) {
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
                        len_of_arena_defs.insert(pc, (b, 0, def, handle));
                    }
                }
            }
        }
    }
    // Check counted `0..N` push loops on local `Vec<Elem>` cells (e.g. `d3-hierarchy::tidy::layout`
    // `for i in 0..nodes.len() { t.push(Tidy::new(i)); }`).
    {
        let mut cell_borrows_local: BTreeMap<ValueId, CellId> = BTreeMap::new();
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
                    if path.is_empty() && local_vec_cells.contains_key(c) {
                        cell_borrows_local.insert(res, *c);
                    }
                }
            }
        }
        propagate_cell_borrows(func, &doms, &mut cell_borrows_local);
        for (&cid, &def) in &local_vec_cells {
            if let Some((push_b, push_oi, iv_c, _bound)) = find_counted_push_loop_details(
                func,
                types,
                &doms,
                &defs,
                &canon,
                cid,
                &cell_borrows_local,
            ) {
                counted_push_ivs.push((push_b, push_oi, iv_c, def, ArenaHandle::VecCell(cid)));
            }
        }
    }

    // Scan all writes to candidate fields (inside `VecPush` aggregates/ctor calls, `SetField`,
    // `StoreThrough`, and inner `Vec<Idx>` mutations).
    for &b in &doms.rpo {
        let block = &func.blocks[b.0 as usize];
        // Track the most recent `len_of_arena` read per `(elem_def, ArenaHandle)` in this block
        // before a `VecPush` (with no intervening push on the same handle!).
        let mut last_len_read: BTreeMap<(DefinitionId, ArenaHandle), ValueId> = BTreeMap::new();
        for (op_idx, op) in block.ops.iter().enumerate() {
            if let Some(res) = op.result {
                if let Some(&(def, handle)) = len_of_arena.get(&res) {
                    last_len_read.insert((def, handle), res);
                }
            }
            match &op.kind {
                OpKind::Call {
                    semantic:
                        CallSemantic::Standard(
                            std_op @ (StandardOperation::VecPush
                            | StandardOperation::NativeVecPush
                            | StandardOperation::VecPushMut
                            | StandardOperation::NativeVecPushMut),
                        ),
                    args,
                    ..
                } if args.len() >= 2 => {
                    let recv = args[0];
                    let val = args[1];
                    if let Some(&(def, handle)) = val_arena.get(&recv) {
                        push_sites.push((b, op_idx, def, handle));
                        if let Some(cfields) = candidate_fields.get(&def) {
                            let pre_len = last_len_read.remove(&(def, handle));
                            if let Some(pl) = pre_len {
                                pre_push_valid.insert(canon[pl.0 as usize], (def, handle));
                            }
                            let post_ok = matches!(handle, ArenaHandle::VecCell(c) if local_post_check_cells.contains(&c));
                            // Inspect the pushed `Elem` value: must be either `Aggregate` or a
                            // pure `ctor_summaries` call!
                            let peeled_val = peel_copy_move_local(func, &defs, val);
                            if let Some((db, di)) = defs[peeled_val.0 as usize] {
                                match &func.blocks[db.0 as usize].ops[di].kind {
                                    OpKind::Aggregate {
                                        ty,
                                        variant: 0,
                                        fields,
                                        ..
                                    } if adt_def_of(types, *ty) == Some(def) => {
                                        for (&fi, &ckind) in cfields {
                                            let Some(&fv) = fields.get(fi as usize) else {
                                                disqualified.insert(def);
                                                break;
                                            };
                                            match ckind {
                                                CandidateFieldKind::Scalar { .. } => {
                                                    write_sites.push(FieldWriteSite {
                                                        func_idx,
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
                                                    if !is_empty_vec_val(func, &defs, fv) {
                                                        write_sites.push(FieldWriteSite {
                                                            func_idx,
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
                                    } if ctor_summaries
                                        .get(item)
                                        .is_some_and(|s| s.elem_def == def) =>
                                    {
                                        let ctor = &ctor_summaries[item];
                                        for (&fi, &ckind) in cfields {
                                            match (ckind, ctor.fields.get(fi as usize)) {
                                                (
                                                    CandidateFieldKind::Scalar { .. },
                                                    Some(CtorFieldSrc::Param(pi)),
                                                ) => {
                                                    if let Some(&arg_v) = ctor_args.get(*pi) {
                                                        write_sites.push(FieldWriteSite {
                                                            func_idx,
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
                                                        func_idx,
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
                                                (
                                                    CandidateFieldKind::VecIdx { .. },
                                                    Some(CtorFieldSrc::EmptyVec),
                                                ) => {}
                                                _ => {
                                                    write_sites.push(FieldWriteSite {
                                                        func_idx,
                                                        block: b,
                                                        op_idx,
                                                        elem_def: def,
                                                        field: fi,
                                                        arena_handle: handle,
                                                        value: peeled_val,
                                                        const_override: None,
                                                        pre_push_len: None,
                                                        is_vec_push: matches!(
                                                            ckind,
                                                            CandidateFieldKind::VecIdx { .. }
                                                        ),
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
                            } else {
                                disqualified.insert(def);
                            }
                        }
                    } else if let Some(&(def, fi, handle)) = field_ref_arena.get(&recv) {
                        let post_ok = matches!(handle, ArenaHandle::VecCell(c) if local_post_check_cells.contains(&c));
                        write_sites.push(FieldWriteSite {
                            func_idx,
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
                    let _ = std_op;
                }
                OpKind::Call {
                    semantic: CallSemantic::Standard(std_op),
                    args,
                    ..
                } => {
                    if let Some(&a0) = args.first() {
                        if let Some(&(def, fi, handle)) = field_ref_arena.get(&a0) {
                            if !is_allowed_inner_idx_vec_op(*std_op) {
                                write_sites.push(FieldWriteSite {
                                    func_idx,
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
                        let arg_ty = func.values[arg.0 as usize].ty;
                        let is_mut_ref = matches!(
                            types.get(types.peeled(arg_ty)).map(|d| &d.kind),
                            Some(TypeKind::Reference { mutable: true, .. })
                        );
                        if !is_mut_ref {
                            continue;
                        }
                        if let Some(&(def, fi, handle)) = field_ref_arena.get(&arg) {
                            write_sites.push(FieldWriteSite {
                                func_idx,
                                block: b,
                                op_idx,
                                elem_def: def,
                                field: fi,
                                arena_handle: handle,
                                value: arg,
                                const_override: None,
                                pre_push_len: None,
                                is_vec_push: candidate_fields
                                    .get(&def)
                                    .and_then(|m| m.get(&fi))
                                    .is_some_and(|k| {
                                        matches!(k, CandidateFieldKind::VecIdx { .. })
                                    }),
                                local_post_check_ok: false,
                            });
                        }
                    }
                }
                OpKind::StoreThrough { ptr, v } => {
                    if let Some(&(def, fi, handle)) = field_ref_arena.get(ptr) {
                        let is_vec = candidate_fields
                            .get(&def)
                            .and_then(|m| m.get(&fi))
                            .is_some_and(|k| matches!(k, CandidateFieldKind::VecIdx { .. }));
                        let post_ok = matches!(handle, ArenaHandle::VecCell(c) if local_post_check_cells.contains(&c));
                        write_sites.push(FieldWriteSite {
                            func_idx,
                            block: b,
                            op_idx,
                            elem_def: def,
                            field: fi,
                            arena_handle: handle,
                            value: *v,
                            const_override: None,
                            pre_push_len: None,
                            is_vec_push: is_vec,
                            local_post_check_ok: post_ok,
                        });
                    } else if let Some((def, fi, handle)) = store_target_elem_field(
                        func,
                        types,
                        &defs,
                        *ptr,
                        &val_arena,
                        &elem_ref_arena,
                        &inner_vec_slice_arena,
                    ) {
                        let is_vec = candidate_fields
                            .get(&def)
                            .and_then(|m| m.get(&fi))
                            .is_some_and(|k| matches!(k, CandidateFieldKind::VecIdx { .. }));
                        let post_ok = matches!(handle, ArenaHandle::VecCell(c) if local_post_check_cells.contains(&c));
                        write_sites.push(FieldWriteSite {
                            func_idx,
                            block: b,
                            op_idx,
                            elem_def: def,
                            field: fi,
                            arena_handle: handle,
                            value: *v,
                            const_override: None,
                            pre_push_len: None,
                            is_vec_push: is_vec,
                            local_post_check_ok: post_ok,
                        });
                    } else if let Some(pt) =
                        value::pointee_of(types, func.values[ptr.0 as usize].ty)
                    {
                        if let Some((def, fi, handle)) = find_borrow_to_elem_field(
                            func,
                            types,
                            &defs,
                            *ptr,
                            &val_arena,
                            &elem_ref_arena,
                        ) {
                            write_sites.push(FieldWriteSite {
                                func_idx,
                                block: b,
                                op_idx,
                                elem_def: def,
                                field: fi,
                                arena_handle: handle,
                                value: *v,
                                const_override: None,
                                pre_push_len: None,
                                is_vec_push: is_unsigned_vec_ty(types, pt).is_some(),
                                local_post_check_ok: false,
                            });
                        }
                    }
                }
                OpKind::SetField { base, field, v, .. } => {
                    let bty = func.values[base.0 as usize].ty;
                    if let Some(def) = adt_def_of(types, bty) {
                        if let Some(cfields) = candidate_fields.get(&def) {
                            if let Some(&ckind) = cfields.get(field) {
                                if let Some(&(_, handle)) = elem_ref_arena.get(base) {
                                    write_sites.push(FieldWriteSite {
                                        func_idx,
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

    FuncArenaContext {
        doms,
        defs,
        canon,
        val_arena,
        len_of_arena_defs,
        push_sites,
        counted_push_ivs,
        loaded_from,
        pre_push_valid,
        call_ret_arenas,
        worklist_field_pushes,
        worklist_popped,
        arenas_per_def,
        lt_arena_facts,
        ne_const_facts,
        eq_const_facts,
        param_incoming,
    }
}

fn peel_copy_move_local(
    func: &Func,
    defs: &[Option<(RBlockId, usize)>],
    mut v: ValueId,
) -> ValueId {
    for _ in 0..8 {
        let Some((b, i)) = defs[v.0 as usize] else {
            // Single-predecessor block parameter passed from an edge arg (e.g. `Goto(b, [agg])`).
            let mut inc: Option<ValueId> = None;
            let mut unique = true;
            for blk in &func.blocks {
                for (target, args) in blk.term.edges() {
                    if let Some(pos) = func.blocks[target.0 as usize]
                        .params
                        .iter()
                        .position(|&p| p == v)
                    {
                        if let Some(&arg) = args.get(pos) {
                            if inc.is_some_and(|prev| prev != arg) {
                                unique = false;
                            }
                            inc = Some(arg);
                        }
                    }
                }
            }
            if unique {
                if let Some(arg) = inc {
                    if arg != v {
                        v = arg;
                        continue;
                    }
                }
            }
            break;
        };
        match func.blocks[b.0 as usize].ops[i].kind {
            OpKind::Copy(s) | OpKind::Move(s) => v = s,
            OpKind::CellLoad(c) => {
                // Single-store, non-borrowed temporary cell (`let tmp = Elem { ... }; vec.push(tmp)`).
                let mut stored: Option<ValueId> = None;
                let mut ok = true;
                for blk in &func.blocks {
                    for op in &blk.ops {
                        match &op.kind {
                            OpKind::CellStore { cell, v: sv } if *cell == c => {
                                if stored.is_some() {
                                    ok = false;
                                }
                                stored = Some(*sv);
                            }
                            OpKind::BorrowView {
                                root: Root::Cell(bc),
                                ..
                            }
                            | OpKind::RawView {
                                root: Root::Cell(bc),
                                ..
                            } if *bc == c => {
                                ok = false;
                            }
                            _ => {}
                        }
                    }
                }
                if ok {
                    if let Some(sv) = stored {
                        v = sv;
                        continue;
                    }
                }
                break;
            }
            _ => break,
        }
    }
    v
}

fn is_empty_vec_val(func: &Func, defs: &[Option<(RBlockId, usize)>], v: ValueId) -> bool {
    let p = peel_copy_move_local(func, defs, v);
    let Some((b, i)) = defs[p.0 as usize] else {
        return false;
    };
    matches!(
        func.blocks[b.0 as usize].ops[i].kind,
        OpKind::Call {
            semantic: CallSemantic::Standard(
                StandardOperation::VecNew
                    | StandardOperation::NativeVecNew
                    | StandardOperation::VecWithCapacity
                    | StandardOperation::NativeVecWithCapacity
            ),
            ..
        }
    )
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
    // Check that `elem_ty` itself has no custom `Drop` impl: wait — in `TypeDescriptor`,
    // `layout.needs_drop` is true if any field needs drop OR if the struct itself implements `Drop`.
    // We cannot distinguish a custom `Drop` impl from field drops via `layout.needs_drop` alone
    // unless every non-`Copy` field is `String`/`Vec` AND we don't have a custom `Drop::drop` in
    // `live`! Wait: let's be even safer — `elem_drop_cannot_read_fields` is checked against
    // `drop_glue_items` or simply: in Rust, a struct `Node` with `String` + `Vec<usize>` has
    // `needs_drop == true`, and `DropGlue(Node)` ONLY calls field drop glues (`String`, `Vec`),
    // NEVER a custom `Drop::drop`! We verify that in `analyze_arenas` by checking that NO
    // `BodySemantic::Ordinary` item in `live` is a `Drop::drop` taking `&mut Elem` (Wait: if a
    // `Drop::drop(&mut Elem)` existed in `live`, our `check_func_arena_preconditions` ALREADY scans
    // it!). Even better: if a local `Vec<Elem>` is dropped on panic before any read of `Elem.f`,
    // could a custom `Drop::drop(&mut Elem)` read `Elem.f`? YES, if `Drop::drop(&mut Elem)` exists
    // AND reads `Elem.f`! So checking `no_custom_drop_reads` is simply: check `safe_ty` AND check
    // in `local_post_check` that the assert IMMEDIATELY follows with no intervening calls or reads!
    // Wait: if `Elem` has a custom `Drop::drop(&mut Elem)` that reads `Elem.f` and indexes `nodes`,
    // `Drop::drop(&mut Elem)` has signature `(&mut Elem) -> ()` — it doesn't even have access to
    // `nodes` (a local variable!) because static arenas are ALREADY poisoned! So even if `Drop`
    // ran on unwind, it could NEVER index `nodes` with the invalid `Elem.f`!
    // STILL, requiring `safe_ty` is extra defense-in-depth!
    let mut seen = BTreeSet::new();
    safe_ty(types, elem_ty, &mut seen)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HolderRoot {
    Param(usize),
    Cell(CellId),
}

#[allow(clippy::too_many_arguments)]
fn classify_borrow_view(
    func: &Func,
    types: &TypeTable,
    active_defs: &[DefinitionId],
    res: ValueId,
    root: &Root,
    path: &[Proj],
    local_vec_cells: &BTreeMap<CellId, DefinitionId>,
    holder_ref: &mut BTreeMap<ValueId, HolderRoot>,
    val_arena: &mut BTreeMap<ValueId, (DefinitionId, ArenaHandle)>,
    elem_ref_arena: &mut BTreeMap<ValueId, (DefinitionId, ArenaHandle)>,
    field_ref_arena: &mut BTreeMap<ValueId, (DefinitionId, u32, ArenaHandle)>,
    inner_vec_slice_arena: &mut BTreeMap<ValueId, (DefinitionId, u32, ArenaHandle)>,
    arenas_per_def: &mut BTreeMap<DefinitionId, BTreeSet<ArenaHandle>>,
) {
    // Case 1: Borrow of a local `Vec<Elem>` cell (`&nodes`, `&mut nodes`) or holder struct cell.
    if let Root::Cell(c) = root {
        let cty = func.cells[c.0 as usize].ty;
        if path.is_empty() {
            if let Some(&def) = local_vec_cells.get(c) {
                val_arena.insert(res, (def, ArenaHandle::VecCell(*c)));
                return;
            }
            if active_defs
                .iter()
                .any(|&def| !holder_vec_fields(types, cty, def).is_empty())
            {
                holder_ref.insert(res, HolderRoot::Cell(*c));
                return;
            }
        }
        // Borrow of a holder struct cell's `Vec<Elem>` field (`&self.nodes`, `&mut self.nodes`).
        if let [Proj::Field { field, ty }] = path {
            if let Some(def) = vec_elem_def(types, *ty) {
                if active_defs.contains(&def) {
                    let h = ArenaHandle::HolderCell(*c, *field);
                    val_arena.insert(res, (def, h));
                    arenas_per_def.entry(def).or_default().insert(h);
                    return;
                }
            }
        }
    }

    // Case 2: Borrow from an SSA `ValueId`.
    if let Root::Value(v) = root {
        // Reborrow of a holder struct `&*self` / `&mut *self`.
        if matches!(path, [] | [Proj::Deref]) {
            if let Some(&hr) = holder_ref.get(v) {
                holder_ref.insert(res, hr);
            }
        }
        // Whole-slice reborrow `&*s` / `&mut *s`.
        if let Some(&h) = val_arena.get(v) {
            match path {
                [] | [Proj::Deref] => {
                    val_arena.insert(res, h);
                    return;
                }
                [Proj::Deref, Proj::Index(_)] | [Proj::Deref, Proj::ConstantIndex { .. }] => {
                    elem_ref_arena.insert(res, h);
                    return;
                }
                [Proj::Deref, Proj::Index(_), Proj::Field { field, .. }]
                | [Proj::Deref, Proj::ConstantIndex { .. }, Proj::Field { field, .. }] => {
                    field_ref_arena.insert(res, (h.0, *field, h.1));
                    return;
                }
                _ => {}
            }
        }
        // Borrow of `&(*holder_ref).nodes_field` (e.g. `&self.nodes` in `EarcutState`, including
        // through copied/reborrowed `&mut self` after inlining!).
        if let [Proj::Deref, Proj::Field { field, ty }] | [Proj::Field { field, ty }] = path {
            if let Some(def) = vec_elem_def(types, *ty) {
                if active_defs.contains(&def) {
                    if let Some(&hr) = holder_ref.get(v) {
                        let h = match hr {
                            HolderRoot::Param(pi) => ArenaHandle::ParamHolder(pi, *field),
                            HolderRoot::Cell(c) => ArenaHandle::HolderCell(c, *field),
                        };
                        val_arena.insert(res, (def, h));
                        arenas_per_def.entry(def).or_default().insert(h);
                        return;
                    }
                }
            }
        }
        // Borrow of `&(*elem_ref).field` or `&mut (*elem_ref).field`.
        if let Some(&(def, handle)) = elem_ref_arena.get(v) {
            match path {
                [] | [Proj::Deref] => {
                    elem_ref_arena.insert(res, (def, handle));
                    return;
                }
                [Proj::Deref, Proj::Field { field, .. }] | [Proj::Field { field, .. }] => {
                    field_ref_arena.insert(res, (def, *field, handle));
                    return;
                }
                _ => {}
            }
        }
        // Reborrow of `&*field_ref` or `&(*inner_slice)[i]`.
        if let Some(&(def, field, handle)) = field_ref_arena.get(v) {
            if matches!(path, [] | [Proj::Deref]) {
                field_ref_arena.insert(res, (def, field, handle));
                return;
            }
        }
        if let Some(&(def, field, handle)) = inner_vec_slice_arena.get(v) {
            match path {
                [] | [Proj::Deref] => {
                    inner_vec_slice_arena.insert(res, (def, field, handle));
                }
                [Proj::Deref, Proj::Index(_)] | [Proj::Deref, Proj::ConstantIndex { .. }] => {
                    field_ref_arena.insert(res, (def, field, handle));
                }
                _ => {}
            }
        }
    }
}

fn resolve_holder_arg(
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

fn deref_vec_len_borrow(
    func: &Func,
    types: &TypeTable,
    interns: &Interns,
    defs: &[Option<(RBlockId, usize)>],
    ptr: ValueId,
    val_arena: &BTreeMap<ValueId, (DefinitionId, ArenaHandle)>,
    holder_ref: &BTreeMap<ValueId, HolderRoot>,
) -> Option<(DefinitionId, ArenaHandle)> {
    let (db, di) = defs[ptr.0 as usize]?;
    if let OpKind::BorrowView { root, path, .. } = &func.blocks[db.0 as usize].ops[di].kind {
        match (root, path.as_slice()) {
            (Root::Value(v), [Proj::Deref, Proj::Field { field, .. }]) => {
                let vty = func.values[v.0 as usize].ty;
                if let Some(pt) = value::pointee_of(types, vty) {
                    if is_direct_global_vec_ty(types, pt)
                        && is_vec_len_field(types, interns, pt, *field)
                    {
                        return val_arena.get(v).copied();
                    }
                }
            }
            (
                Root::Value(v),
                [Proj::Deref, Proj::Field { field: hf, ty: hty }, Proj::Field { field: lf, .. }],
            ) => {
                if is_direct_global_vec_ty(types, *hty)
                    && is_vec_len_field(types, interns, *hty, *lf)
                {
                    if let Some(def) = vec_elem_def(types, *hty) {
                        if let Some(&hr) = holder_ref.get(v) {
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
                let cty = func.cells[c.0 as usize].ty;
                if is_direct_global_vec_ty(types, cty) && is_vec_len_field(types, interns, cty, *lf)
                {
                    if let Some(def) = vec_elem_def(types, cty) {
                        return Some((def, ArenaHandle::VecCell(*c)));
                    }
                }
            }
            (
                Root::Cell(c),
                [Proj::Field { field: hf, ty: hty }, Proj::Field { field: lf, .. }],
            ) if is_direct_global_vec_ty(types, *hty)
                && is_vec_len_field(types, interns, *hty, *lf) =>
            {
                if let Some(def) = vec_elem_def(types, *hty) {
                    return Some((def, ArenaHandle::HolderCell(*c, *hf)));
                }
            }
            _ => {}
        }
    }
    None
}

fn deref_elem_field_borrow(
    func: &Func,
    _types: &TypeTable,
    defs: &[Option<(RBlockId, usize)>],
    ptr: ValueId,
    val_arena: &BTreeMap<ValueId, (DefinitionId, ArenaHandle)>,
    elem_ref_arena: &BTreeMap<ValueId, (DefinitionId, ArenaHandle)>,
    inner_vec_slice_arena: &BTreeMap<ValueId, (DefinitionId, u32, ArenaHandle)>,
) -> Option<(DefinitionId, u32, ArenaHandle)> {
    let (db, di) = defs[ptr.0 as usize]?;
    if let OpKind::BorrowView {
        root: Root::Value(v),
        path,
        ..
    } = &func.blocks[db.0 as usize].ops[di].kind
    {
        if let Some(&(def, handle)) = val_arena.get(v) {
            if let [Proj::Deref, Proj::Index(_), Proj::Field { field, .. }]
            | [Proj::Deref, Proj::ConstantIndex { .. }, Proj::Field { field, .. }] =
                path.as_slice()
            {
                return Some((def, *field, handle));
            }
        }
        if let Some(&(def, handle)) = elem_ref_arena.get(v) {
            if let [Proj::Deref, Proj::Field { field, .. }] = path.as_slice() {
                return Some((def, *field, handle));
            }
        }
        if let Some(&(def, field, handle)) = inner_vec_slice_arena.get(v) {
            if let [Proj::Deref, Proj::Index(_)] | [Proj::Deref, Proj::ConstantIndex { .. }] =
                path.as_slice()
            {
                return Some((def, field, handle));
            }
        }
    }
    None
}

fn store_target_elem_field(
    func: &Func,
    types: &TypeTable,
    defs: &[Option<(RBlockId, usize)>],
    ptr: ValueId,
    val_arena: &BTreeMap<ValueId, (DefinitionId, ArenaHandle)>,
    elem_ref_arena: &BTreeMap<ValueId, (DefinitionId, ArenaHandle)>,
    inner_vec_slice_arena: &BTreeMap<ValueId, (DefinitionId, u32, ArenaHandle)>,
) -> Option<(DefinitionId, u32, ArenaHandle)> {
    deref_elem_field_borrow(
        func,
        types,
        defs,
        ptr,
        val_arena,
        elem_ref_arena,
        inner_vec_slice_arena,
    )
}

fn find_borrow_to_elem_field(
    func: &Func,
    types: &TypeTable,
    defs: &[Option<(RBlockId, usize)>],
    ptr: ValueId,
    val_arena: &BTreeMap<ValueId, (DefinitionId, ArenaHandle)>,
    elem_ref_arena: &BTreeMap<ValueId, (DefinitionId, ArenaHandle)>,
) -> Option<(DefinitionId, u32, ArenaHandle)> {
    deref_elem_field_borrow(
        func,
        types,
        defs,
        ptr,
        val_arena,
        elem_ref_arena,
        &BTreeMap::new(),
    )
}

/// Tracks local non-escaping worklist `Vec<T>` cells (e.g. `todo: Vec<(JsValue, usize)>`) whose
/// ONLY mutations are `VecNew`/`WithCapacity`/`VecFromArray`/`VecPush`/`VecPop`.
#[allow(clippy::type_complexity)]
fn collect_local_worklists(
    func: &Func,
    types: &TypeTable,
    defs: &[Option<(RBlockId, usize)>],
    canon: &[ValueId],
) -> (
    BTreeMap<(CellId, Option<u32>), Vec<(RBlockId, usize, ValueId)>>,
    BTreeMap<ValueId, (CellId, Option<u32>)>,
) {
    let mut pushes: BTreeMap<(CellId, Option<u32>), Vec<(RBlockId, usize, ValueId)>> =
        BTreeMap::new();
    let mut popped: BTreeMap<ValueId, (CellId, Option<u32>)> = BTreeMap::new();
    let mut cell_ref_map: BTreeMap<ValueId, CellId> = BTreeMap::new();
    let mut disqualified_cells: BTreeSet<CellId> = BTreeSet::new();

    for (ci, cell) in func.cells.iter().enumerate() {
        let cid = CellId(ci as u32);
        if !is_global_vec_ty(types, cell.ty) {
            disqualified_cells.insert(cid);
        }
    }

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
                if path.is_empty() && !disqualified_cells.contains(c) {
                    cell_ref_map.insert(res, *c);
                } else {
                    disqualified_cells.insert(*c);
                }
            }
        }
    }
    let doms_wl = Dominators::of(func);
    propagate_cell_borrows(func, &doms_wl, &mut cell_ref_map);

    // Verify every use of `cell_ref_map` is strictly `VecPush` / `VecPop` / `VecLen` / `VecIsEmpty`
    // / `VecReserve` / `VecDrop`, and track every pushed value (including initial `VecFromArray`!).
    let mut opt_pop_cell: BTreeMap<ValueId, CellId> = BTreeMap::new();
    for block in &func.blocks {
        for (oi, op) in block.ops.iter().enumerate() {
            match &op.kind {
                OpKind::CellStore { cell, v } if !disqualified_cells.contains(cell) => {
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
                                record_worklist_init_array(
                                    func,
                                    defs,
                                    *cell,
                                    b_oi(block.id, oi),
                                    args[0],
                                    &mut pushes,
                                    &mut disqualified_cells,
                                );
                            }
                            _ => {
                                disqualified_cells.insert(*cell);
                            }
                        }
                    } else {
                        disqualified_cells.insert(*cell);
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
                                StandardOperation::VecPush | StandardOperation::NativeVecPush
                                    if args.len() >= 2 =>
                                {
                                    record_worklist_push(
                                        func,
                                        defs,
                                        cid,
                                        block.id,
                                        oi,
                                        args[1],
                                        &mut pushes,
                                        &mut disqualified_cells,
                                    );
                                }
                                StandardOperation::VecPop | StandardOperation::NativeVecPop => {
                                    if let Some(res) = op.result {
                                        opt_pop_cell.insert(res, cid);
                                    }
                                }
                                StandardOperation::VecLen
                                | StandardOperation::VecIsEmpty
                                | StandardOperation::VecReserve
                                | StandardOperation::NativeVecReserve
                                | StandardOperation::VecDrop => {}
                                _ => {
                                    disqualified_cells.insert(cid);
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
                            disqualified_cells.insert(cid);
                        }
                    }
                }
            }
        }
    }

    // Track extracted payload from `Option<T>` returned by `Vec::pop` -> both SSA `Field` and
    // pre-PEA `CellStore(opt_cell, pop_res)` -> `BorrowView([Downcast, Field(0), Field(fi)])` + `Deref`!
    let mut popped_elem_cell: BTreeMap<ValueId, CellId> = BTreeMap::new();
    let mut opt_cell_src: BTreeMap<CellId, CellId> = BTreeMap::new();
    let mut popped_ref: BTreeMap<ValueId, (CellId, Option<u32>)> = BTreeMap::new();
    for &bid in &doms_wl.rpo {
        let block = &func.blocks[bid.0 as usize];
        for op in &block.ops {
            if let OpKind::CellStore { cell, v } = &op.kind {
                let cv = canon[v.0 as usize];
                if let Some(&cid) = opt_pop_cell.get(v).or_else(|| opt_pop_cell.get(&cv)) {
                    if !disqualified_cells.contains(&cid) {
                        opt_cell_src.insert(*cell, cid);
                    }
                }
            }
            let Some(res) = op.result else { continue };
            match &op.kind {
                OpKind::CellLoad(c) => {
                    if let Some(&cid) = opt_cell_src.get(c) {
                        opt_pop_cell.insert(res, cid);
                    }
                }
                OpKind::BorrowView { root, path, .. } => {
                    let src_cid = match root {
                        Root::Cell(c) => opt_cell_src.get(c).copied(),
                        Root::Value(v) => opt_pop_cell.get(v).copied(),
                    };
                    if let Some(cid) = src_cid {
                        match path.as_slice() {
                            [Proj::Downcast { .. }, Proj::Field { field: 0, .. }] => {
                                popped_ref.insert(res, (cid, None));
                            }
                            [Proj::Downcast { .. }, Proj::Field { field: 0, .. }, Proj::Field { field: fi, .. }] =>
                            {
                                popped_ref.insert(res, (cid, Some(*fi)));
                            }
                            _ => {}
                        }
                    } else if let Root::Value(v) = root {
                        if let Some(&(cid, None)) = popped_ref.get(v) {
                            if let [Proj::Deref, Proj::Field { field: fi, .. }] = path.as_slice() {
                                popped_ref.insert(res, (cid, Some(*fi)));
                            }
                        }
                    }
                }
                OpKind::Deref(ptr) => {
                    if let Some(&p) = popped_ref.get(ptr) {
                        popped.insert(res, p);
                    }
                }
                OpKind::Field { base, field, .. } => {
                    let cb = canon[base.0 as usize];
                    if let Some(&cid) = opt_pop_cell.get(base).or_else(|| opt_pop_cell.get(&cb)) {
                        if !disqualified_cells.contains(&cid) {
                            popped_elem_cell.insert(res, cid);
                            popped.insert(res, (cid, None));
                        }
                    } else if let Some(&cid) = popped_elem_cell
                        .get(base)
                        .or_else(|| popped_elem_cell.get(&cb))
                    {
                        if !disqualified_cells.contains(&cid) {
                            popped.insert(res, (cid, Some(*field)));
                        }
                    }
                }
                OpKind::Copy(s) | OpKind::Move(s) => {
                    if let Some(&c) = opt_pop_cell.get(s) {
                        opt_pop_cell.insert(res, c);
                    }
                    if let Some(&c) = popped_elem_cell.get(s) {
                        popped_elem_cell.insert(res, c);
                    }
                    if let Some(&p) = popped_ref.get(s) {
                        popped_ref.insert(res, p);
                    }
                    if let Some(&p) = popped.get(s) {
                        popped.insert(res, p);
                    }
                }
                _ => {}
            }
        }
    }

    pushes.retain(|(cid, _), _| !disqualified_cells.contains(cid));
    popped.retain(|_, (cid, _)| !disqualified_cells.contains(cid));
    (pushes, popped)
}

fn b_oi(b: RBlockId, oi: usize) -> (RBlockId, usize) {
    (b, oi)
}

fn record_worklist_init_array(
    func: &Func,
    defs: &[Option<(RBlockId, usize)>],
    cid: CellId,
    at: (RBlockId, usize),
    arr_val: ValueId,
    pushes: &mut WorklistPushes,
    disqualified: &mut BTreeSet<CellId>,
) {
    let p = peel_copy_move_local(func, defs, arr_val);
    let Some((db, di)) = defs[p.0 as usize] else {
        disqualified.insert(cid);
        return;
    };
    if let OpKind::Aggregate { fields, .. } = &func.blocks[db.0 as usize].ops[di].kind {
        for &elem in fields {
            record_worklist_push(func, defs, cid, at.0, at.1, elem, pushes, disqualified);
        }
    } else {
        disqualified.insert(cid);
    }
}

#[allow(clippy::too_many_arguments)]
fn record_worklist_push(
    func: &Func,
    defs: &[Option<(RBlockId, usize)>],
    cid: CellId,
    block: RBlockId,
    op_idx: usize,
    val: ValueId,
    pushes: &mut WorklistPushes,
    _disqualified: &mut BTreeSet<CellId>,
) {
    pushes
        .entry((cid, None))
        .or_default()
        .push((block, op_idx, val));
    let p = peel_copy_move_local(func, defs, val);
    if let Some((db, di)) = defs[p.0 as usize] {
        if let OpKind::Aggregate { fields, .. } = &func.blocks[db.0 as usize].ops[di].kind {
            for (fi, &fv) in fields.iter().enumerate() {
                pushes
                    .entry((cid, Some(fi as u32)))
                    .or_default()
                    .push((block, op_idx, fv));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_local_arena_facts(
    func: &Func,
    types: &TypeTable,
    defs: &[Option<(RBlockId, usize)>],
    canon: &[ValueId],
    val_arena: &BTreeMap<ValueId, (DefinitionId, ArenaHandle)>,
    len_of_arena: &BTreeMap<ValueId, (DefinitionId, ArenaHandle)>,
    lt_arena_facts: &mut Vec<(RBlockId, Option<usize>, ValueId, DefinitionId, ArenaHandle)>,
    ne_const_facts: &mut Vec<(RBlockId, ValueId, u128)>,
    eq_const_facts: &mut Vec<(RBlockId, ValueId, u128)>,
) {
    // Helper to look up `len_of_arena` on either `v` or `canon[v]`.
    let arena_of_len = |v: ValueId| {
        len_of_arena
            .get(&v)
            .or_else(|| len_of_arena.get(&canon[v.0 as usize]))
            .copied()
    };
    let const_of = |v: ValueId| -> Option<u128> {
        let c = canon[v.0 as usize];
        let (db, di) = defs[c.0 as usize]?;
        match func.blocks[db.0 as usize].ops[di].kind {
            OpKind::Const(ConstTree::Scalar { bits, bytes }) => {
                let max = types
                    .int_info(types.peeled(func.values[c.0 as usize].ty))
                    .map_or(u128::MAX, |(_, w)| width_max(w));
                Some(mask_bits(bits, bytes).min(max))
            }
            _ => None,
        }
    };

    let mut preds: Vec<Vec<u32>> = vec![Vec::new(); func.blocks.len()];
    for b in &func.blocks {
        for s in b.term.successors() {
            if !preds[s.0 as usize].contains(&b.id.0) {
                preds[s.0 as usize].push(b.id.0);
            }
        }
    }

    // 1. Executed `Assert` ops and `VecIndex`/`VecIndexMut` calls (which assert `idx < vec.len`).
    for block in &func.blocks {
        for (oi, op) in block.ops.iter().enumerate() {
            match &op.kind {
                OpKind::Assert {
                    cond,
                    expected: true,
                    ..
                } => {
                    let c = canon[cond.0 as usize];
                    if let Some((db, di)) = defs[c.0 as usize] {
                        if let OpKind::Binary {
                            op: BinaryOp::Lt,
                            l,
                            r,
                            ty,
                        } = func.blocks[db.0 as usize].ops[di].kind
                        {
                            if matches!(types.int_info(ty), Some((false, _))) {
                                if let Some((def, handle)) = arena_of_len(r) {
                                    lt_arena_facts.push((
                                        block.id,
                                        Some(oi),
                                        canon[l.0 as usize],
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
                    && is_unsigned_index_ty(types, func.values[args[1].0 as usize].ty)
                        .is_some() =>
                {
                    if let Some(&(def, handle)) = val_arena.get(&args[0]) {
                        lt_arena_facts.push((
                            block.id,
                            Some(oi),
                            canon[args[1].0 as usize],
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
    for block in func.blocks.iter().skip(1) {
        let [pred] = preds[block.id.0 as usize].as_slice() else {
            continue;
        };
        let RTerm::Switch {
            scrut,
            cases,
            default,
            ..
        } = &func.blocks[*pred as usize].term
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
        // Direct switch on an integer value (e.g. `match x { NONE => ..., _ => ... }`).
        if let [None] = entering.as_slice() {
            for &(cv, _, _) in cases {
                ne_const_facts.push((block.id, canon[scrut.0 as usize], cv));
            }
        } else if let [Some(cv)] = entering.as_slice() {
            eq_const_facts.push((block.id, canon[scrut.0 as usize], *cv));
        }

        let positive = match entering.as_slice() {
            [Some(0)] => false,
            [Some(1)] => true,
            [None] if cases.iter().all(|(v, _, _)| *v == 0) => true,
            _ => continue,
        };
        let sc = canon[scrut.0 as usize];
        let Some((db, di)) = defs[sc.0 as usize] else {
            continue;
        };
        if let OpKind::Binary { op, l, r, ty } = func.blocks[db.0 as usize].ops[di].kind {
            if !matches!(types.int_info(ty), Some((false, _))) {
                continue;
            }
            match (op, positive) {
                (BinaryOp::Lt, true) => {
                    if let Some((def, handle)) = arena_of_len(r) {
                        lt_arena_facts.push((block.id, None, canon[l.0 as usize], def, handle));
                    }
                }
                (BinaryOp::Gt, true) => {
                    if let Some((def, handle)) = arena_of_len(l) {
                        lt_arena_facts.push((block.id, None, canon[r.0 as usize], def, handle));
                    }
                }
                (BinaryOp::Ne, true) | (BinaryOp::Eq, false) => {
                    if let Some(c) = const_of(r) {
                        ne_const_facts.push((block.id, canon[l.0 as usize], c));
                    } else if let Some(c) = const_of(l) {
                        ne_const_facts.push((block.id, canon[r.0 as usize], c));
                    }
                }
                (BinaryOp::Eq, true) | (BinaryOp::Ne, false) => {
                    if let Some(c) = const_of(r) {
                        eq_const_facts.push((block.id, canon[l.0 as usize], c));
                    } else if let Some(c) = const_of(l) {
                        eq_const_facts.push((block.id, canon[r.0 as usize], c));
                    }
                }
                _ => {}
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn classify_value_for_arena(
    func: &Func,
    types: &TypeTable,
    ctx: &FuncArenaContext,
    block: RBlockId,
    op_idx: usize,
    value: ValueId,
    elem_def: DefinitionId,
    handle: ArenaHandle,
    pre_push_len: Option<ValueId>,
    local_post_check_ok: bool,
    field_state: &BTreeMap<(DefinitionId, u32), ArenaFieldKind>,
    ret_idx_state: &BTreeMap<ItemId, ReturnArenaIndex>,
) -> ValueArenaClass {
    let mut visited = BTreeSet::new();
    let mut cls = classify_value_rec(
        func,
        types,
        ctx,
        block,
        op_idx,
        value,
        elem_def,
        handle,
        pre_push_len,
        field_state,
        ret_idx_state,
        &mut visited,
    );
    // Strip sentinel if `value != sentinel` dominates `(block, op_idx)`.
    if let ValueArenaClass::ValidOrSentinel(s) = cls {
        if value_ne_const_at(ctx, block, value, s) {
            cls = ValueArenaClass::AlwaysValid;
        }
    }
    if cls != ValueArenaClass::Unknown {
        return cls;
    }
    // Check local post-checked write: for a local unescaped `Vec<Elem>`, if the value is checked
    // `Assert(value < arena.len)` (either unconditionally in the same straight-line block, or in
    // the `value != sentinel` branch right after the write) before any read of `Elem` or escape!
    if local_post_check_ok {
        if let Some(post_cls) =
            check_local_post_assert(func, ctx, block, op_idx, value, elem_def, handle)
        {
            return post_cls;
        }
    }
    ValueArenaClass::Unknown
}

#[allow(clippy::too_many_arguments)]
fn classify_value_rec(
    func: &Func,
    types: &TypeTable,
    ctx: &FuncArenaContext,
    block: RBlockId,
    op_idx: usize,
    value: ValueId,
    elem_def: DefinitionId,
    handle: ArenaHandle,
    pre_push_len: Option<ValueId>,
    field_state: &BTreeMap<(DefinitionId, u32), ArenaFieldKind>,
    ret_idx_state: &BTreeMap<ItemId, ReturnArenaIndex>,
    visited: &mut BTreeSet<ValueId>,
) -> ValueArenaClass {
    let cv = ctx.canon(value);

    // 1. Pre-push length on the SAME arena handle!
    // Because the arena is length-monotone, ANY `len = arena.len()` whose definition dominates a
    // `VecPush` `(pb, pi)` on the SAME `(elem_def, handle)` that dominates (or equals) `(block, op_idx)`
    // satisfies `len <= len_before_push < len_after_push`!
    if let Some(pl) = pre_push_len {
        if ctx.canon(pl) == cv {
            return ValueArenaClass::AlwaysValid;
        }
    }
    if let Some(&(pd, ph)) = ctx.pre_push_valid.get(&cv) {
        if pd == elem_def && ph == handle {
            return ValueArenaClass::AlwaysValid;
        }
    }
    if let Some(&(lb, li, ld, lh)) = ctx.len_of_arena_defs.get(&cv) {
        if ld == elem_def && lh == handle {
            let dom_lt = |a: (RBlockId, usize), b: (RBlockId, usize)| {
                (a.0 == b.0 && a.1 < b.1) || (a.0 != b.0 && dominates_block(&ctx.doms, a.0, b.0))
            };
            let dom_le = |a: (RBlockId, usize), b: (RBlockId, usize)| {
                (a.0 == b.0 && a.1 <= b.1) || (a.0 != b.0 && dominates_block(&ctx.doms, a.0, b.0))
            };
            if ctx.push_sites.iter().any(|&(pb, pi, pd, ph)| {
                pd == elem_def
                    && ph == handle
                    && dom_lt((lb, li), (pb, pi))
                    && dom_le((pb, pi), (block, op_idx))
            }) {
                return ValueArenaClass::AlwaysValid;
            }
        }
    }
    // Counted `0..N` unconditional single-push loop induction var (`iv == pre_push_len < post_push_len`
    // at and after the push in each iteration!).
    if ctx.counted_push_ivs.iter().any(|&(pb, pi, iv_c, pd, ph)| {
        iv_c == cv
            && pd == elem_def
            && ph == handle
            && ((pb == block && pi <= op_idx)
                || (pb != block && dominates_block(&ctx.doms, pb, block)))
    }) {
        return ValueArenaClass::AlwaysValid;
    }

    // 2. Dominating non-arena `< arena.len` fact on the SAME arena handle!
    for &(fb, foi, fv, fd, fh) in &ctx.lt_arena_facts {
        if fd == elem_def && fh == handle && fv == cv {
            let dom = if fb == block {
                foi.is_none_or(|i| i < op_idx)
            } else {
                dominates_block(&ctx.doms, fb, block)
            };
            if dom {
                return ValueArenaClass::AlwaysValid;
            }
        }
    }

    // 3. Constant scalar?
    if let Some((db, di)) = ctx.defs[cv.0 as usize] {
        if let OpKind::Const(ConstTree::Scalar { bits, bytes }) =
            func.blocks[db.0 as usize].ops[di].kind
        {
            let max = types
                .int_info(types.peeled(func.values[cv.0 as usize].ty))
                .map_or(u128::MAX, |(_, w)| width_max(w));
            return ValueArenaClass::Const(mask_bits(bits, bytes).min(max));
        }
    }

    // 4. Loaded from a proven field (or `Vec<Idx>` field) of the SAME arena handle!
    if let Some(&(ld, lf, lh, _is_vec)) = ctx
        .loaded_from
        .get(&value)
        .or_else(|| ctx.loaded_from.get(&cv))
    {
        if ld == elem_def && lh == handle {
            if let Some(&kind) = field_state.get(&(ld, lf)) {
                return match kind {
                    ArenaFieldKind::AlwaysValid | ArenaFieldKind::VecOfValidIndices => {
                        ValueArenaClass::AlwaysValid
                    }
                    ArenaFieldKind::ValidOrSentinel { sentinel } => {
                        if value_ne_const_at(ctx, block, value, sentinel) {
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
    if let Some((callee_item, arg_arenas)) = ctx
        .call_ret_arenas
        .get(&value)
        .or_else(|| ctx.call_ret_arenas.get(&cv))
    {
        if let Some(ret) = ret_idx_state.get(callee_item) {
            if ret.elem_def == elem_def
                && arg_arenas.iter().any(|&(ai, hf, d, h)| {
                    ai == ret.arena_param && hf == ret.holder_field && d == elem_def && h == handle
                })
            {
                return match ret.sentinel {
                    None => ValueArenaClass::AlwaysValid,
                    Some(s) => {
                        if value_ne_const_at(ctx, block, value, s) {
                            ValueArenaClass::AlwaysValid
                        } else {
                            ValueArenaClass::ValidOrSentinel(s)
                        }
                    }
                };
            }
        }
    }

    // Cycle guard for block params and worklists (co-inductive: revisiting a cycle node contributes
    // `AlwaysValid` so self-loops like `p = next` inherit the non-cycle incoming edges!).
    if !visited.insert(cv) {
        return ValueArenaClass::AlwaysValid;
    }

    // 6. Popped from a local worklist `Vec` cell (e.g. `todo.pop()` in `d3-hierarchy::nested`).
    if let Some(&(cid, field_opt)) = ctx
        .worklist_popped
        .get(&value)
        .or_else(|| ctx.worklist_popped.get(&cv))
    {
        if let Some(push_list) = ctx.worklist_field_pushes.get(&(cid, field_opt)) {
            if !push_list.is_empty() {
                let mut acc: Option<ValueArenaClass> = None;
                for &(pb, poi, pv) in push_list {
                    let c = classify_value_rec(
                        func,
                        types,
                        ctx,
                        pb,
                        poi,
                        pv,
                        elem_def,
                        handle,
                        None,
                        field_state,
                        ret_idx_state,
                        visited,
                    );
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

    // 7. Block parameter: join across all incoming edges!
    if let Some(incoming) = ctx.param_incoming.get(&cv) {
        if !incoming.is_empty() {
            let mut acc: Option<ValueArenaClass> = None;
            for &(pred_b, arg) in incoming {
                let pred_len = func.blocks[pred_b.0 as usize].ops.len();
                let mut c = classify_value_rec(
                    func,
                    types,
                    ctx,
                    pred_b,
                    pred_len,
                    arg,
                    elem_def,
                    handle,
                    None,
                    field_state,
                    ret_idx_state,
                    visited,
                );
                if let ValueArenaClass::ValidOrSentinel(s) = c {
                    if value_ne_const_at(ctx, pred_b, arg, s) {
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
    }

    ValueArenaClass::Unknown
}

fn value_ne_const_at(ctx: &FuncArenaContext, block: RBlockId, v: ValueId, c: u128) -> bool {
    let cv = ctx.canon(v);
    ctx.ne_const_facts
        .iter()
        .any(|&(fb, fv, fc)| fv == cv && fc == c && dominates_block(&ctx.doms, fb, block))
}

/// Checks if a write of `value` at `(block, op_idx)` into a local unescaped `Vec<Elem>` is
/// immediately post-checked by `Assert(value < arena.len)` (either in the same block after `op_idx`
/// with no intervening reads of `Elem` or calls, OR in the `value != sentinel` branch of `block`'s
/// terminator!).
fn check_local_post_assert(
    func: &Func,
    ctx: &FuncArenaContext,
    block: RBlockId,
    op_idx: usize,
    value: ValueId,
    elem_def: DefinitionId,
    handle: ArenaHandle,
) -> Option<ValueArenaClass> {
    let cv = ctx.canon(value);
    let b = &func.blocks[block.0 as usize];
    // Straight-line check starting at `(blk, start)`, following at most 2 pure `Goto` hops.
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
                    && ctx.lt_arena_facts.iter().any(|&(fb, foi, fv, fd, fh)| {
                        fb == cur_b.id
                            && foi == Some(i)
                            && fv == cv
                            && fd == elem_def
                            && fh == handle
                    })
                {
                    return true;
                }
                // Stop before any read of an arena element or any call (other than `VecAsSlice`/
                // `VecAsMutSlice`/`VecLen`/`VecIndex`/`VecIndexMut` which precede the bounds check).
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
                    cur_b = &func.blocks[next.0 as usize];
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

    // Check `if value != sentinel { assert(value < arena.len); ... }` right at `b.term` (or after
    // at most 2 pure `Goto` hops, e.g. `d3-hierarchy::nested` lines 143-145!).
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
                cur_blk = &func.blocks[next.0 as usize];
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
        // Look for a target where `value == s` and the other target(s) have `value != s` AND
        // immediately assert `value < arena.len`!
        for &(_, fv, sentinel) in &ctx.eq_const_facts {
            if fv != cv {
                continue;
            }
            let eq_ok = targets.iter().any(|&t| {
                ctx.eq_const_facts
                    .iter()
                    .any(|&(fb, f, c)| fb == t && f == cv && c == sentinel)
            });
            if !eq_ok {
                continue;
            }
            let all_other_assert = targets.iter().all(|&t| {
                ctx.eq_const_facts
                    .iter()
                    .any(|&(fb, f, c)| fb == t && f == cv && c == sentinel)
                    || (ctx
                        .ne_const_facts
                        .iter()
                        .any(|&(fb, f, c)| fb == t && f == cv && c == sentinel)
                        && has_assert_in(&func.blocks[t.0 as usize], 0))
            });
            if all_other_assert {
                return Some(ValueArenaClass::ValidOrSentinel(sentinel));
            }
        }
    }
    None
}

fn verify_return_arena_index(
    func: &Func,
    types: &TypeTable,
    ctx: &FuncArenaContext,
    cur: ReturnArenaIndex,
    handle: ArenaHandle,
    field_state: &BTreeMap<(DefinitionId, u32), ArenaFieldKind>,
    ret_idx_state: &BTreeMap<ItemId, ReturnArenaIndex>,
) -> Option<ReturnArenaIndex> {
    let mut acc: Option<ValueArenaClass> = None;
    for block in &func.blocks {
        let RTerm::Return(Some(ret_v)) = block.term else {
            continue;
        };
        let cls = classify_value_for_arena(
            func,
            types,
            ctx,
            block.id,
            block.ops.len(),
            ret_v,
            cur.elem_def,
            handle,
            None,
            false,
            field_state,
            ret_idx_state,
        );
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

// =================================================================================================
// Part 3 (Stage 2): Interprocedural Return-Vec-Len, Return-Vec-Elem-Bound,
//                   Param-Arena-Index, and Equal-Slice-Len Summaries
// =================================================================================================

fn analyze_return_vec_lens(
    live_items: &[(&JsName, &LinkItem, &Func, &Interns)],
    trusted_std_bodies: &BTreeSet<ItemId>,
    drop_glue_items: &BTreeSet<ItemId>,
    facts: &mut ProgramBoundsFacts,
) {
    for &(_, _, func, interns) in live_items {
        if trusted_std_bodies.contains(&func.item) || drop_glue_items.contains(&func.item) {
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

fn summarize_return_vec_len(func: &Func, types: &TypeTable) -> Option<ReturnVecLen> {
    let doms = Dominators::of(func);
    let mut defs: Vec<Option<(RBlockId, usize)>> = vec![None; func.values.len()];
    for b in &func.blocks {
        for (i, op) in b.ops.iter().enumerate() {
            if let Some(res) = op.result {
                defs[res.0 as usize] = Some((b.id, i));
            }
        }
    }
    let mut param_idx: BTreeMap<ValueId, usize> = BTreeMap::new();
    for (i, &p) in func.params.iter().enumerate() {
        param_idx.insert(p, i);
    }

    // Track single-store local `Vec` cells whose length is never mutated after initialization
    // (only `VecAsSlice`/`VecAsMutSlice`/`VecLen`/`VecIndex`/`VecIndexMut`/`CellLoad`).
    let mut cell_init_len: BTreeMap<CellId, ReturnVecLen> = BTreeMap::new();
    let mut cell_disqualified: BTreeSet<CellId> = BTreeSet::new();
    let mut cell_borrows: BTreeMap<ValueId, CellId> = BTreeMap::new();
    let mut cell_stores: BTreeMap<CellId, usize> = BTreeMap::new();

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
                } else {
                    cell_disqualified.insert(*c);
                }
            }
        }
    }
    propagate_cell_borrows(func, &doms, &mut cell_borrows);

    let expr_len = |v: ValueId| -> Option<ReturnVecLen> {
        let p = peel_copy_move_local(func, &defs, v);
        if let Some(&pi) = param_idx.get(&p) {
            return Some(ReturnVecLen::Param(pi));
        }
        let (db, di) = defs[p.0 as usize]?;
        match &func.blocks[db.0 as usize].ops[di].kind {
            OpKind::Const(ConstTree::Scalar { bits, bytes }) => {
                let max = types
                    .int_info(types.peeled(func.values[p.0 as usize].ty))
                    .map_or(u128::MAX, |(_, w)| width_max(w));
                Some(ReturnVecLen::Const(mask_bits(*bits, *bytes).min(max)))
            }
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v: slice_v,
                ..
            } => {
                let sp = peel_copy_move_local(func, &defs, *slice_v);
                param_idx
                    .get(&sp)
                    .map(|&pi| ReturnVecLen::ParamSliceLen(pi))
            }
            OpKind::Binary {
                op: BinaryOp::Shl | BinaryOp::ShlUnchecked,
                l,
                r,
                ty,
            } => {
                let (_, bits) = types.int_info(*ty)?;
                let lp = peel_copy_move_local(func, &defs, *l);
                let rp = peel_copy_move_local(func, &defs, *r);
                let l_is_one = defs[lp.0 as usize].is_some_and(|(lb, li)| {
                    matches!(
                        func.blocks[lb.0 as usize].ops[li].kind,
                        OpKind::Const(ConstTree::Scalar { bits: 1, .. })
                    )
                });
                if l_is_one {
                    param_idx.get(&rp).map(|&pi| ReturnVecLen::OneShlParam {
                        shift_param: pi,
                        bits,
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    };

    let vec_val_len =
        |v: ValueId, cell_lens: &BTreeMap<CellId, ReturnVecLen>| -> Option<ReturnVecLen> {
            let p = peel_copy_move_local(func, &defs, v);
            let (db, di) = defs[p.0 as usize]?;
            match &func.blocks[db.0 as usize].ops[di].kind {
                OpKind::CellLoad(c) => cell_lens.get(c).cloned(),
                OpKind::Call {
                    semantic: CallSemantic::Standard(StandardOperation::NativeVecFromElemZeroed),
                    args,
                    ..
                } if !args.is_empty() => expr_len(args[0]),
                OpKind::Call {
                    semantic: CallSemantic::Standard(StandardOperation::VecFromElem),
                    args,
                    ..
                } if args.len() >= 2 => expr_len(args[1]),
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
                    let sp = peel_copy_move_local(func, &defs, args[0]);
                    param_idx
                        .get(&sp)
                        .map(|&pi| ReturnVecLen::ParamSliceLen(pi))
                }
                _ => None,
            }
        };

    for b in &func.blocks {
        for op in &b.ops {
            match &op.kind {
                OpKind::CellStore { cell, v }
                    if is_global_vec_ty(types, func.cells[cell.0 as usize].ty) =>
                {
                    let cnt = cell_stores.entry(*cell).or_insert(0);
                    *cnt += 1;
                    if *cnt > 1 {
                        cell_disqualified.insert(*cell);
                    } else if let Some(len_s) = vec_val_len(*v, &BTreeMap::new()) {
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

    // Check every `Return(Some(v))` block; also unfold a single merged `Return(Some(block_param))`
    // whose 2 incoming edges come from two distinct branches (e.g. `fflate::h_map`'s `if reverse`).
    let mut returns: Vec<(RBlockId, ReturnVecLen)> = Vec::new();
    for b in &func.blocks {
        if let RTerm::Return(Some(ret_v)) = b.term {
            if let Some(len_s) = vec_val_len(ret_v, &cell_init_len) {
                returns.push((b.id, len_s));
            } else {
                // Check if `peel(ret_v)` is a block param of `b` with 2 incoming edges!
                let pv = peel_copy_move_local(func, &defs, ret_v);
                let pos = b.params.iter().position(|&p| p == pv)?;
                let mut edge_rets = Vec::new();
                let mut ok = true;
                for pred in &func.blocks {
                    for (target, args) in pred.term.edges() {
                        if target == b.id {
                            if let Some(len_s) =
                                args.get(pos).and_then(|&a| vec_val_len(a, &cell_init_len))
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
    if returns.is_empty() {
        return None;
    }
    // All returns agree on one exact length summary?
    if returns.iter().all(|(_, l)| l == &returns[0].1) {
        return Some(returns[0].1.clone());
    }
    // Two-way conditional on a `bool` entry parameter (e.g. `fflate::h_map(cd, mb, reverse)`).
    if returns.len() == 2 {
        // Find a `Switch` block on a `bool` entry parameter whose true/false edges dominate the
        // two return blocks respectively.
        for b in &func.blocks {
            let RTerm::Switch {
                scrut,
                cases,
                default,
                ..
            } = &b.term
            else {
                continue;
            };
            let sp = peel_copy_move_local(func, &defs, *scrut);
            let Some(&cond_param) = param_idx.get(&sp) else {
                continue;
            };
            if !matches!(
                types
                    .get(types.peeled(func.values[sp.0 as usize].ty))
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
            for (rb, len_s) in &returns {
                if dominates_block(&doms, true_blk, *rb) {
                    when_true = Some(len_s.clone());
                } else if dominates_block(&doms, false_blk, *rb) {
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
    }
    None
}

/// Proves `ReturnVecElemBound::LtParamSliceLen { slice_param }` for functions like
/// `d3-hierarchy::walk(nodes: &[Node], root: usize, post: bool) -> Vec<usize>` where every element
/// pushed into the returned `Vec` is either:
/// - checked `< nodes.len()` (e.g. `nodes[v]`!), or
/// - loaded from a proven `VecOfValidIndices` / `AlwaysValid` arena field of `nodes`!
fn analyze_return_vec_elem_bounds(
    live_items: &[(&JsName, &LinkItem, &Func, &Interns)],
    trusted_std_bodies: &BTreeSet<ItemId>,
    drop_glue_items: &BTreeSet<ItemId>,
    facts: &mut ProgramBoundsFacts,
) {
    for &(_, _, func, interns) in live_items {
        if trusted_std_bodies.contains(&func.item) || drop_glue_items.contains(&func.item) {
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
            if func_returns_vec_lt_param_slice(func, types, interns, p_idx, p_val, facts) {
                facts.return_vec_elem_bound.insert(
                    func.item,
                    ReturnVecElemBound::LtParamSliceLen { slice_param: p_idx },
                );
                break;
            }
        }
    }
}

fn func_returns_vec_lt_param_slice(
    func: &Func,
    types: &TypeTable,
    interns: &Interns,
    slice_param_idx: usize,
    slice_param_val: ValueId,
    facts: &ProgramBoundsFacts,
) -> bool {
    let doms = Dominators::of(func);
    let mut defs: Vec<Option<(RBlockId, usize)>> = vec![None; func.values.len()];
    let mut canon: Vec<ValueId> = (0..func.values.len() as u32).map(ValueId).collect();
    for b in &func.blocks {
        for (i, op) in b.ops.iter().enumerate() {
            if let Some(res) = op.result {
                defs[res.0 as usize] = Some((b.id, i));
            }
        }
    }
    for &b in &doms.rpo {
        for op in &func.blocks[b.0 as usize].ops {
            let Some(res) = op.result else { continue };
            match &op.kind {
                OpKind::Copy(s) | OpKind::Move(s) => canon[res.0 as usize] = canon[s.0 as usize],
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
                    canon[res.0 as usize] = canon[v.0 as usize];
                }
                _ => {}
            }
        }
    }

    // Track `slice_param_val` aliases and its `PtrMetadata` length values.
    let mut slice_vals = BTreeSet::new();
    slice_vals.insert(slice_param_val);
    let mut len_vals = BTreeSet::new();
    for &b in &doms.rpo {
        for op in &func.blocks[b.0 as usize].ops {
            let Some(res) = op.result else { continue };
            match &op.kind {
                OpKind::Copy(s) | OpKind::Move(s) => {
                    if slice_vals.contains(s) {
                        slice_vals.insert(res);
                    }
                    if len_vals.contains(s) {
                        len_vals.insert(res);
                    }
                }
                OpKind::BorrowView {
                    root: Root::Value(s),
                    path,
                    ..
                } if matches!(path.as_slice(), [Proj::Deref]) && slice_vals.contains(s) => {
                    slice_vals.insert(res);
                }
                OpKind::Unary {
                    op: UnaryOp::PtrMetadata,
                    v,
                    ..
                } if slice_vals.contains(v) => {
                    len_vals.insert(canon[res.0 as usize]);
                }
                _ => {}
            }
        }
    }

    // Find all `Assert(x < slice_len)` (and single-pred switch edges `x < slice_len`).
    let mut checked_lt: Vec<(RBlockId, Option<usize>, ValueId)> = Vec::new();
    for b in &func.blocks {
        for (oi, op) in b.ops.iter().enumerate() {
            if let OpKind::Assert {
                cond,
                expected: true,
                ..
            } = &op.kind
            {
                let cc = canon[cond.0 as usize];
                if let Some((db, di)) = defs[cc.0 as usize] {
                    if let OpKind::Binary {
                        op: BinaryOp::Lt,
                        l,
                        r,
                        ty,
                    } = func.blocks[db.0 as usize].ops[di].kind
                    {
                        if matches!(types.int_info(ty), Some((false, _)))
                            && len_vals.contains(&canon[r.0 as usize])
                        {
                            checked_lt.push((b.id, Some(oi), canon[l.0 as usize]));
                        }
                    }
                }
            }
        }
    }

    // Identify the returned local `Vec<Idx>` cell(s).
    let mut ret_cells = BTreeSet::new();
    for b in &func.blocks {
        if let RTerm::Return(Some(rv)) = b.term {
            let pv = peel_copy_move_local(func, &defs, rv);
            let Some((db, di)) = defs[pv.0 as usize] else {
                return false;
            };
            match func.blocks[db.0 as usize].ops[di].kind {
                OpKind::CellLoad(c) => {
                    ret_cells.insert(c);
                }
                _ => return false,
            }
        }
    }
    if ret_cells.is_empty() {
        return false;
    }

    // Verify every `ret_cell` is initialized empty, only mutated via `VecPush`, `SliceSwap`,
    // `SliceSort`, `reverse` (via `&mut [T]`), `VecReserve*`, and EVERY pushed value `v` is proven
    // `< slice_param.len()` (either dominated by `checked_lt`, OR post-checked in the same loop
    // body by `Assert(v < slice_len)` with no intervening return/escape!).
    for &rc in &ret_cells {
        let mut borrows = BTreeSet::new();
        let mut mut_slices = BTreeSet::new();
        let mut stores = 0usize;
        let mut pushed: Vec<(RBlockId, usize, ValueId)> = Vec::new();
        for &bid in &doms.rpo {
            let b = &func.blocks[bid.0 as usize];
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
                            borrows.insert(res);
                        }
                    }
                    OpKind::Copy(s) | OpKind::Move(s) => {
                        if let Some(res) = op.result {
                            if borrows.contains(s) {
                                borrows.insert(res);
                            }
                            if mut_slices.contains(s) {
                                mut_slices.insert(res);
                            }
                        }
                    }
                    OpKind::BorrowView {
                        root: Root::Value(s),
                        path,
                        ..
                    } if matches!(path.as_slice(), [] | [Proj::Deref]) => {
                        if let Some(res) = op.result {
                            if borrows.contains(s) {
                                borrows.insert(res);
                            }
                            if mut_slices.contains(s) {
                                mut_slices.insert(res);
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
                    } if args.first().is_some_and(|a| borrows.contains(a)) => {
                        if let Some(res) = op.result {
                            mut_slices.insert(res);
                        }
                    }
                    _ => {}
                }
            }
        }
        for b in &func.blocks {
            for (oi, op) in b.ops.iter().enumerate() {
                match &op.kind {
                    OpKind::CellStore { cell, v } if *cell == rc => {
                        stores += 1;
                        if stores > 1 || !is_empty_vec_val(func, &defs, *v) {
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
                    } if args.first().is_some_and(|a| borrows.contains(a)) => {}
                    OpKind::Call {
                        semantic:
                            CallSemantic::Standard(
                                StandardOperation::VecPush | StandardOperation::NativeVecPush,
                            ),
                        args,
                        ..
                    } if args.len() >= 2 && borrows.contains(&args[0]) => {
                        pushed.push((b.id, oi, args[1]));
                    }
                    OpKind::Call {
                        callee,
                        semantic,
                        args,
                        ..
                    } if args
                        .iter()
                        .any(|a| borrows.contains(a) || mut_slices.contains(a)) =>
                    {
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
                        // Also allow `core::slice::<impl [T]>::reverse` (permutation only!).
                        let ok_rev = matches!(callee, Callee::Direct { .. })
                            && args.len() == 1
                            && mut_slices.contains(&args[0])
                            && is_slice_reverse_call(interns, callee);
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
                            if borrows.contains(&u) || mut_slices.contains(&u) {
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
        // Check every pushed value `pv`: must be proven `< slice_len` either before push OR
        // immediately after push in the same straight-line block (like `walk`: `out.push(v); ...
        // &nodes[v].children` — if `v >= nodes.len()`, `walk` panics on `nodes[v]` and NEVER
        // returns `out`!).
        for (pb, poi, pv) in pushed {
            let cpv = canon[pv.0 as usize];
            let dom_ok = checked_lt.iter().any(|&(cb, coi, cv)| {
                cv == cpv
                    && if cb == pb {
                        coi.is_none_or(|i| i < poi)
                    } else {
                        dominates_block(&doms, cb, pb)
                    }
            });
            if dom_ok {
                continue;
            }
            // Check post-dominating assert on EVERY path from `(pb, poi)` before any `Return`:
            // specifically, if `pb` itself (or EVERY successor path within 2 hops before any
            // `Return`) executes `Assert(cpv < slice_len)`!
            let post_ok = value_asserted_before_return(func, pb, poi, cpv, &checked_lt);
            if !post_ok {
                return false;
            }
        }
    }
    let _ = (slice_param_idx, facts);
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

/// Checks that along ALL paths starting after `(start_block, start_op)` before any `Return`
/// (bounded BFS, no `Return` reachable without passing a `checked_lt` assert for `target_v`),
/// `target_v < slice_len` is asserted!
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

/// Stage 2: Propagates `ParamArenaIndex` for closed, non-root, non-exported, non-address-taken
/// internal functions (e.g. `earcut::{filter_points_sl, is_valid_diagonal_sl, is_ear_hashed_sl,
/// intersects_sl, earcut_linked, eliminate_hole, ...}` and `d3-hierarchy::tidy::{contour, apportion}`).
#[allow(clippy::too_many_arguments)]
fn analyze_param_arena_indices(
    live_items: &[(&JsName, &LinkItem, &Func, &Interns)],
    by_item_id: &BTreeMap<ItemId, usize>,
    trusted_std_bodies: &BTreeSet<ItemId>,
    drop_glue_items: &BTreeSet<ItemId>,
    root_or_exported: &BTreeSet<ItemId>,
    address_taken: &BTreeSet<ItemId>,
    facts: &mut ProgramBoundsFacts,
) {
    if facts.arenas.is_empty() {
        return;
    }

    // Collect all closed candidate `(callee_item, idx_param, arena_param, holder_field, elem_def)`
    // and all direct call sites to them.
    let mut candidates: ParamArenaCandidateMap = BTreeMap::new();
    let mut call_sites: ParamCallSiteMap = BTreeMap::new();

    for &(_, _, func, interns) in live_items {
        if trusted_std_bodies.contains(&func.item)
            || drop_glue_items.contains(&func.item)
            || root_or_exported.contains(&func.item)
            || address_taken.contains(&func.item)
        {
            continue;
        }
        let types = &interns.types;
        // Find arena params of `func`.
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
    for (func_idx, &(_, _, func, _)) in live_items.iter().enumerate() {
        if trusted_std_bodies.contains(&func.item) || drop_glue_items.contains(&func.item) {
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

    // Remove candidates with zero call sites.
    candidates.retain(|k, _| call_sites.get(&k.0).is_some_and(|v| !v.is_empty()));
    if candidates.is_empty() {
        return;
    }

    // Build lightweight per-caller arena context for callers of candidate callees.
    let caller_indices: BTreeSet<usize> = call_sites
        .values()
        .flat_map(|v| v.iter().map(|(fi, _, _, _)| *fi))
        .collect();
    let proven_fields: BTreeMap<(DefinitionId, u32), ArenaFieldKind> = facts
        .arenas
        .iter()
        .flat_map(|(&d, inv)| inv.fields.iter().map(move |(&f, &k)| ((d, f), k)))
        .collect();
    let candidate_field_shapes: BTreeMap<DefinitionId, BTreeMap<u32, CandidateFieldKind>> = facts
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
    let mut caller_ctxs: BTreeMap<usize, FuncArenaContext> = BTreeMap::new();
    let mut dummy_disq = BTreeSet::new();
    let mut dummy_writes = Vec::new();
    let empty_ctors = BTreeMap::new();
    for &fi in &caller_indices {
        let (_, _, func, interns) = live_items[fi];
        let ctx = build_func_arena_context(
            fi,
            func,
            &interns.types,
            interns,
            &active_defs,
            &candidate_field_shapes,
            &empty_ctors,
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
                let (_, _, caller_func, caller_interns) = live_items[caller_idx];
                let Some(ctx) = caller_ctxs.get(&caller_idx) else {
                    ok = false;
                    break;
                };
                // Resolve caller's `ArenaHandle` for `arg_arena`.
                let caller_handle = match holder_field {
                    None => ctx
                        .val_arena
                        .get(&arg_arena)
                        .filter(|&&(d, _)| d == elem_def)
                        .map(|&(_, h)| h),
                    Some(hf) => resolve_holder_arg(
                        caller_func,
                        &caller_interns.types,
                        &ctx.defs,
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

                // Classify `arg_idx` in `caller_func` using proven fields, `return_arena_index`,
                // AND caller's own surviving `candidates` (`ParamArenaIndex`)!
                let mut cls = classify_value_for_arena_with_params(
                    caller_func,
                    &caller_interns.types,
                    ctx,
                    blk,
                    oi,
                    arg_idx,
                    elem_def,
                    handle,
                    &proven_fields,
                    &facts.return_arena_index,
                    &candidates,
                    &facts.return_vec_elem_bound,
                );
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
                    ArenaFieldKind::AlwaysValid => None,
                    ArenaFieldKind::ValidOrSentinel { sentinel } => Some(sentinel),
                    ArenaFieldKind::VecOfValidIndices => None,
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
    let _ = by_item_id;
}

#[allow(clippy::too_many_arguments)]
fn classify_value_for_arena_with_params(
    func: &Func,
    types: &TypeTable,
    ctx: &FuncArenaContext,
    block: RBlockId,
    op_idx: usize,
    value: ValueId,
    elem_def: DefinitionId,
    handle: ArenaHandle,
    field_state: &BTreeMap<(DefinitionId, u32), ArenaFieldKind>,
    ret_idx_state: &BTreeMap<ItemId, ReturnArenaIndex>,
    param_candidates: &ParamArenaCandidateMap,
    ret_vec_elem_bound: &BTreeMap<ItemId, ReturnVecElemBound>,
) -> ValueArenaClass {
    let base = classify_value_for_arena(
        func,
        types,
        ctx,
        block,
        op_idx,
        value,
        elem_def,
        handle,
        None,
        false,
        field_state,
        ret_idx_state,
    );
    if base != ValueArenaClass::Unknown {
        return base;
    }
    // Check if `value` is a parameter (or block param derived from params/arena values) covered by
    // `param_candidates`!
    let mut visited = BTreeSet::new();
    classify_with_params_rec(
        func,
        types,
        ctx,
        block,
        op_idx,
        value,
        elem_def,
        handle,
        field_state,
        ret_idx_state,
        param_candidates,
        ret_vec_elem_bound,
        &mut visited,
    )
}

#[allow(clippy::too_many_arguments)]
fn classify_with_params_rec(
    func: &Func,
    types: &TypeTable,
    ctx: &FuncArenaContext,
    block: RBlockId,
    op_idx: usize,
    value: ValueId,
    elem_def: DefinitionId,
    handle: ArenaHandle,
    field_state: &BTreeMap<(DefinitionId, u32), ArenaFieldKind>,
    ret_idx_state: &BTreeMap<ItemId, ReturnArenaIndex>,
    param_candidates: &ParamArenaCandidateMap,
    ret_vec_elem_bound: &BTreeMap<ItemId, ReturnVecElemBound>,
    visited: &mut BTreeSet<ValueId>,
) -> ValueArenaClass {
    let base = classify_value_for_arena(
        func,
        types,
        ctx,
        block,
        op_idx,
        value,
        elem_def,
        handle,
        None,
        false,
        field_state,
        ret_idx_state,
    );
    if base != ValueArenaClass::Unknown {
        return base;
    }
    let cv = ctx.canon(value);
    // Entry parameter of `func` in `param_candidates`?
    if let Some((pi, _)) = func
        .params
        .iter()
        .enumerate()
        .find(|(_, &pv)| ctx.canon(pv) == cv)
    {
        let (ap, hf) = match handle {
            ArenaHandle::Param(a) => (Some(a), None),
            ArenaHandle::ParamHolder(a, f) => (Some(a), Some(f)),
            _ => (None, None),
        };
        if let Some(ap) = ap {
            if let Some(&sent) = param_candidates.get(&(func.item, pi, ap, hf, elem_def)) {
                return match sent {
                    None => ValueArenaClass::AlwaysValid,
                    Some(s) => {
                        if value_ne_const_at(ctx, block, value, s) {
                            ValueArenaClass::AlwaysValid
                        } else {
                            ValueArenaClass::ValidOrSentinel(s)
                        }
                    }
                };
            }
        }
    }
    if !visited.insert(cv) {
        return ValueArenaClass::AlwaysValid;
    }
    if let Some(incoming) = ctx.param_incoming.get(&cv) {
        if !incoming.is_empty() {
            let mut acc: Option<ValueArenaClass> = None;
            for &(pred_b, arg) in incoming {
                let pred_len = func.blocks[pred_b.0 as usize].ops.len();
                let mut c = classify_with_params_rec(
                    func,
                    types,
                    ctx,
                    pred_b,
                    pred_len,
                    arg,
                    elem_def,
                    handle,
                    field_state,
                    ret_idx_state,
                    param_candidates,
                    ret_vec_elem_bound,
                    visited,
                );
                if let ValueArenaClass::ValidOrSentinel(s) = c {
                    if value_ne_const_at(ctx, pred_b, arg, s) {
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
    }
    let _ = ret_vec_elem_bound;
    ValueArenaClass::Unknown
}

/// Stage 2: Proves `ParamSliceEqLen` for closed internal functions whose every caller passes two
/// slices/vecs of provably equal length (e.g. `d3-hierarchy::tidy::{contour, apportion}` where
/// `t` was created by a `0..nodes.len()` push loop).
#[allow(clippy::too_many_arguments)]
fn analyze_param_slice_eq_len(
    live_items: &[(&JsName, &LinkItem, &Func, &Interns)],
    _by_item_id: &BTreeMap<ItemId, usize>,
    trusted_std_bodies: &BTreeSet<ItemId>,
    drop_glue_items: &BTreeSet<ItemId>,
    root_or_exported: &BTreeSet<ItemId>,
    address_taken: &BTreeSet<ItemId>,
    facts: &mut ProgramBoundsFacts,
) {
    let mut candidates: BTreeSet<(ItemId, usize, usize)> = BTreeSet::new();
    for &(_, _, func, interns) in live_items {
        if trusted_std_bodies.contains(&func.item)
            || drop_glue_items.contains(&func.item)
            || root_or_exported.contains(&func.item)
            || address_taken.contains(&func.item)
        {
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
    for (fi, &(_, _, func, _)) in live_items.iter().enumerate() {
        if trusted_std_bodies.contains(&func.item) || drop_glue_items.contains(&func.item) {
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

    // For each caller, compute simple local equal-length slice/vec pairs (including `0..s.len()`
    // push loops and `vec![0; s.len()]`!).
    let mut caller_eq_pairs: BTreeMap<usize, BTreeSet<(ValueId, ValueId)>> = BTreeMap::new();
    for sites in call_sites.values() {
        for &(fi, _) in sites {
            caller_eq_pairs.entry(fi).or_insert_with(|| {
                let (_, _, func, interns) = live_items[fi];
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
                // Transitive through caller's own `candidates` params!
                let (_, _, cfunc, _) = live_items[*fi];
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
    let doms = Dominators::of(func);
    let mut defs: Vec<Option<(RBlockId, usize)>> = vec![None; func.values.len()];
    let mut canon: Vec<ValueId> = (0..func.values.len() as u32).map(ValueId).collect();
    for b in &func.blocks {
        for (i, op) in b.ops.iter().enumerate() {
            if let Some(res) = op.result {
                defs[res.0 as usize] = Some((b.id, i));
            }
        }
    }
    for &b in &doms.rpo {
        for op in &func.blocks[b.0 as usize].ops {
            let Some(res) = op.result else { continue };
            match &op.kind {
                OpKind::Copy(s) | OpKind::Move(s) => canon[res.0 as usize] = canon[s.0 as usize],
                OpKind::BorrowView {
                    root: Root::Value(s),
                    path,
                    ..
                } if matches!(path.as_slice(), [Proj::Deref]) => {
                    canon[res.0 as usize] = canon[s.0 as usize];
                }
                _ => {}
            }
        }
    }

    // Map `len_val -> slice_canon`.
    let mut len_of_slice: BTreeMap<ValueId, ValueId> = BTreeMap::new();
    for &b in &doms.rpo {
        for op in &func.blocks[b.0 as usize].ops {
            let Some(res) = op.result else { continue };
            if let OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v,
                ..
            } = op.kind
            {
                len_of_slice.insert(canon[res.0 as usize], canon[v.0 as usize]);
            }
        }
    }

    // Map local `Vec` cells whose length equals `len_val_canon`:
    // 1. Initialized by `NativeVecFromElemZeroed(len)` / `VecFromElem(_, len)` / `SliceToVec(s)`
    //    and never resized/pushed/popped.
    // 2. Initialized empty (`VecNew` / `VecWithCapacity`), pushed ONCE unconditionally per
    //    iteration of a `0..N` counted loop, and never mutated again.
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

    // Find counted `0..N` push loops per local `Vec` cell.
    for (ci, cell) in func.cells.iter().enumerate() {
        if !is_global_vec_ty(types, cell.ty) {
            continue;
        }
        let cid = CellId(ci as u32);
        if let Some(bound_len) =
            find_counted_push_loop_len(func, types, &doms, &defs, &canon, cid, &cell_borrows)
        {
            if let Some(&src_slice) = len_of_slice.get(&canon[bound_len.0 as usize]) {
                cell_eq_slice.insert(cid, src_slice);
            }
        }
    }

    // Map every slice derived from `cell_eq_slice` via `VecAsSlice`/`VecAsMutSlice` to `src_slice`.
    let mut out = BTreeSet::new();
    let mut slice_group: BTreeMap<ValueId, ValueId> = BTreeMap::new();
    for &b in &doms.rpo {
        for op in &func.blocks[b.0 as usize].ops {
            let Some(res) = op.result else { continue };
            match &op.kind {
                OpKind::Copy(s) | OpKind::Move(s) => {
                    if let Some(&g) = slice_group.get(s) {
                        slice_group.insert(res, g);
                    }
                }
                OpKind::BorrowView {
                    root: Root::Value(s),
                    path,
                    ..
                } if matches!(path.as_slice(), [Proj::Deref]) => {
                    if let Some(&g) = slice_group.get(s) {
                        slice_group.insert(res, g);
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
                                slice_group.insert(res, src_s);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Emit `(res, src_slice)` pairs (including any alias of `src_slice`).
    for (&derived, &src_canon) in &slice_group {
        for (vi, &c) in canon.iter().enumerate() {
            if c == src_canon {
                out.insert((derived, ValueId(vi as u32)));
            }
        }
    }
    out
}

fn propagate_cell_borrows(
    func: &Func,
    doms: &Dominators,
    cell_borrows: &mut BTreeMap<ValueId, CellId>,
) {
    for &bid in &doms.rpo {
        for op in &func.blocks[bid.0 as usize].ops {
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

fn find_counted_push_loop_details(
    func: &Func,
    types: &TypeTable,
    doms: &Dominators,
    defs: &[Option<(RBlockId, usize)>],
    canon: &[ValueId],
    cid: CellId,
    cell_borrows: &BTreeMap<ValueId, CellId>,
) -> Option<(RBlockId, usize, ValueId, ValueId)> {
    let mut store_block = None;
    let mut push_site: Option<(RBlockId, usize)> = None;
    for b in &func.blocks {
        for (oi, op) in b.ops.iter().enumerate() {
            match &op.kind {
                OpKind::CellStore { cell, v } if *cell == cid => {
                    if store_block.is_some() || !is_empty_vec_val(func, defs, *v) {
                        return None;
                    }
                    store_block = Some(b.id);
                }
                OpKind::BorrowView {
                    root: Root::Cell(c),
                    path,
                    ..
                } if *c == cid && !path.is_empty() => {
                    return None;
                }
                OpKind::RawView {
                    root: Root::Cell(c),
                    ..
                } if *c == cid => return None,
                OpKind::Copy(_) | OpKind::Move(_) => {}
                OpKind::BorrowView {
                    root: Root::Value(_),
                    path,
                    ..
                } if matches!(path.as_slice(), [] | [Proj::Deref]) => {}
                OpKind::Call { semantic, args, .. }
                    if args
                        .first()
                        .is_some_and(|a| cell_borrows.get(a) == Some(&cid)) =>
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
                        if cell_borrows.get(&u) == Some(&cid)
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

    // Find the natural loop header dominating `push_blk` that has:
    // - one entry edge (dominated by `init_blk`) passing `0` to `iv`
    // - one backedge (dominated by `push_blk`, so every iteration executes the push!) passing `iv + 1`
    // - a `Switch` on `iv < N` whose true branch dominates `push_blk` and whose false branch exits!
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
            let (p0, a0) = &preds[h_idx][0];
            let (p1, a1) = &preds[h_idx][1];
            let (pre, pre_args, back, back_args) =
                if dominates_block(doms, cur, *p1) && !dominates_block(doms, cur, *p0) {
                    (*p0, a0, *p1, a1)
                } else if dominates_block(doms, cur, *p0) && !dominates_block(doms, cur, *p1) {
                    (*p1, a1, *p0, a0)
                } else {
                    match doms.idom[h_idx] {
                        Some(up) if up != cur => {
                            cur = up;
                            continue;
                        }
                        _ => return None,
                    }
                };
            if !dominates_block(doms, init_blk, pre) || !dominates_block(doms, push_blk, back) {
                return None;
            }
            // Check header `Switch` on `iv < N`.
            let RTerm::Switch {
                scrut,
                cases,
                default,
                ..
            } = &func.blocks[h_idx].term
            else {
                return None;
            };
            let sc = canon[scrut.0 as usize];
            let (cb, ci) = defs[sc.0 as usize]?;
            let OpKind::Binary {
                op: BinaryOp::Lt,
                l: iv_val,
                r: bound_val,
                ty,
            } = func.blocks[cb.0 as usize].ops[ci].kind
            else {
                return None;
            };
            if !matches!(types.int_info(ty), Some((false, _))) {
                return None;
            }
            let (false_blk, true_blk) = match cases.as_slice() {
                [(0, fb, _)] => (*fb, default.0),
                [(1, tb, _)] => (default.0, *tb),
                [(0, fb, _), (1, tb, _)] | [(1, tb, _), (0, fb, _)] => (*fb, *tb),
                _ => return None,
            };
            if !dominates_block(doms, true_blk, push_blk)
                || dominates_block(doms, false_blk, push_blk)
            {
                return None;
            }
            // Ensure `iv_val` is a block param of `cur` initialized to `0` on `pre` and `iv + 1` on `back`.
            let iv_c = canon[iv_val.0 as usize];
            let pos = func.blocks[h_idx]
                .params
                .iter()
                .position(|&p| canon[p.0 as usize] == iv_c)?;
            let init_v = canon[pre_args.get(pos)?.0 as usize];
            let back_v = canon[back_args.get(pos)?.0 as usize];
            let init_is_zero = defs[init_v.0 as usize].is_some_and(|(ib, ii)| {
                matches!(
                    func.blocks[ib.0 as usize].ops[ii].kind,
                    OpKind::Const(ConstTree::Scalar { bits: 0, .. })
                )
            });
            if !init_is_zero {
                return None;
            }
            let back_is_inc = defs[back_v.0 as usize].is_some_and(|(bb, bi)| {
                match func.blocks[bb.0 as usize].ops[bi].kind {
                    OpKind::Binary {
                        op: BinaryOp::Add | BinaryOp::AddUnchecked,
                        l,
                        r,
                        ..
                    } => {
                        let is_one = |v: ValueId| {
                            let cv = canon[v.0 as usize];
                            defs[cv.0 as usize].is_some_and(|(ob, oi)| {
                                matches!(
                                    func.blocks[ob.0 as usize].ops[oi].kind,
                                    OpKind::Const(ConstTree::Scalar { bits: 1, .. })
                                )
                            })
                        };
                        (canon[l.0 as usize] == iv_c && is_one(r))
                            || (canon[r.0 as usize] == iv_c && is_one(l))
                    }
                    _ => false,
                }
            });
            if !back_is_inc {
                return None;
            }
            // Ensure no other exit from the loop body bypasses `push_blk` or exits early, and `bound_val`
            // dominates `cur` (loop-invariant).
            let bound_dom = match defs[canon[bound_val.0 as usize].0 as usize] {
                Some((bb, _)) => dominates_block(doms, bb, cur),
                None => true, // entry param
            };
            if !bound_dom {
                return None;
            }
            // Check every block inside the loop (reachable from `true_blk` without leaving via `false_blk`):
            // none may jump to `false_blk` directly or return early without hitting `cur`!
            let mut visited = BTreeSet::new();
            let mut stack = vec![true_blk];
            while let Some(lb) = stack.pop() {
                if lb == cur || !visited.insert(lb) {
                    continue;
                }
                if lb == false_blk {
                    return None;
                }
                match func.blocks[lb.0 as usize].term {
                    RTerm::Return(_) => return None,
                    RTerm::Unreachable | RTerm::Throw(_) => continue,
                    _ => {
                        for s in func.blocks[lb.0 as usize].term.successors() {
                            if s != cur && !visited.contains(&s) {
                                stack.push(s);
                            }
                        }
                    }
                }
            }
            // Verify every `VecAsSlice`/`VecAsMutSlice`/`VecLen` use of `cid` is dominated by
            // `false_blk` (post-loop)!
            for b in &func.blocks {
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
                            .is_some_and(|a| cell_borrows.get(a) == Some(&cid))
                            && !dominates_block(doms, false_blk, b.id)
                        {
                            return None;
                        }
                    }
                }
            }
            return Some((push_blk, push_oi, iv_c, bound_val));
        }
        match doms.idom[h_idx] {
            Some(up) if up != cur => cur = up,
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::heap::HeapPlan;
    use crate::item::LinkJir;
    use crate::jir::repr::FuncBuilder;
    use crate::jir::semantic::{AbiSignature, Body, BodyKind, Signature};
    use crate::jir::types::{
        AllocatorIdentity, FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts,
        PointerForm, RepresentationCandidate, TypeDescriptor, TypeMeta, VariantDescriptor,
    };
    use crate::jir::OriginId;
    use rcgjs_object::item::AllocatorProvider;
    use rcgjs_object::object::Linkage;

    const UNIT: TypeId = TypeId(1);
    const U32: TypeId = TypeId(2);
    const BOOL: TypeId = TypeId(3);
    const NODE: TypeId = TypeId(10);
    const VEC_NODE: TypeId = TypeId(11);
    const REF_MUT_VEC_NODE: TypeId = TypeId(12);
    const SLICE_NODE: TypeId = TypeId(13);
    const REF_SLICE_NODE: TypeId = TypeId(14);
    const REF_MUT_SLICE_NODE: TypeId = TypeId(15);
    const REF_MUT_U32: TypeId = TypeId(16);
    const RAW_NODE: TypeId = TypeId(17);
    const VEC_U32: TypeId = TypeId(18);
    const REF_MUT_VEC_U32: TypeId = TypeId(19);
    const NO_ORIGIN: OriginId = OriginId(0);

    const NODE_DEF: DefinitionId = DefinitionId {
        crate_id: 99,
        local_hash: 1001,
    };
    const VEC_DEF: DefinitionId = DefinitionId {
        crate_id: 2,
        local_hash: 2002,
    };

    fn make_desc(id: TypeId, kind: TypeKind, size: u64, is_indirect: bool) -> TypeDescriptor {
        let int = match kind {
            TypeKind::Integer { signed, bits } => Some(IntFacts {
                signed,
                bits,
                repr: IntRepr::Number,
            }),
            _ => None,
        };
        let sized = !matches!(kind, TypeKind::Slice(_));
        TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                size: Some(size),
                align: Some(size.clamp(1, 8)),
                sized,
                zst: size == 0,
                needs_drop: false,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int,
                is_indirect,
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

    fn make_arena_interns() -> Interns {
        let mut interns = Interns::new();
        let prev_k = interns.intern_str("prev");
        let next_z_k = interns.intern_str("next_z");
        let len_k = interns.intern_str("len");
        let node_k = interns.intern_str("Node");
        let vec_k = interns.intern_str("Vec");
        let _hook = interns.intern_str("std_hook");

        interns
            .types
            .insert(make_desc(UNIT, TypeKind::Tuple(Vec::new()), 0, false));
        interns.types.insert(make_desc(
            U32,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            4,
            false,
        ));
        interns
            .types
            .insert(make_desc(BOOL, TypeKind::Bool, 1, false));
        interns.types.insert(make_desc(
            NODE,
            TypeKind::Adt {
                def: NODE_DEF,
                form: NominalForm::Struct,
                args: Vec::new(),
                variants: vec![VariantDescriptor {
                    name: node_k,
                    fields: vec![
                        FieldDescriptor {
                            key: FieldKey::Name(prev_k),
                            ty: U32,
                            offset: 0,
                        },
                        FieldDescriptor {
                            key: FieldKey::Name(next_z_k),
                            ty: U32,
                            offset: 4,
                        },
                    ],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            8,
            true,
        ));

        let mut vec_node = make_desc(
            VEC_NODE,
            TypeKind::Adt {
                def: VEC_DEF,
                form: NominalForm::Struct,
                args: vec![NODE],
                variants: vec![VariantDescriptor {
                    name: vec_k,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(len_k),
                        ty: U32,
                        offset: 0,
                    }],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            12,
            true,
        );
        vec_node.meta.candidate = RepresentationCandidate::Vec;
        vec_node.meta.allocator = AllocatorIdentity::Global;
        interns.types.insert(vec_node);

        let mut ref_mut_vec_node = make_desc(
            REF_MUT_VEC_NODE,
            TypeKind::Reference {
                pointee: VEC_NODE,
                mutable: true,
            },
            4,
            false,
        );
        ref_mut_vec_node.js.pointer_form = Some(PointerForm::Thin);
        interns.types.insert(ref_mut_vec_node);

        interns
            .types
            .insert(make_desc(SLICE_NODE, TypeKind::Slice(NODE), 0, false));
        let mut ref_sl = make_desc(
            REF_SLICE_NODE,
            TypeKind::Reference {
                pointee: SLICE_NODE,
                mutable: false,
            },
            8,
            false,
        );
        ref_sl.js.pointer_form = Some(PointerForm::FatSlice);
        interns.types.insert(ref_sl);

        let mut ref_mut_sl = make_desc(
            REF_MUT_SLICE_NODE,
            TypeKind::Reference {
                pointee: SLICE_NODE,
                mutable: true,
            },
            8,
            false,
        );
        ref_mut_sl.js.pointer_form = Some(PointerForm::FatSlice);
        interns.types.insert(ref_mut_sl);

        let mut ref_mut_u32 = make_desc(
            REF_MUT_U32,
            TypeKind::Reference {
                pointee: U32,
                mutable: true,
            },
            4,
            false,
        );
        ref_mut_u32.js.pointer_form = Some(PointerForm::Slot);
        interns.types.insert(ref_mut_u32);

        let mut raw_node = make_desc(
            RAW_NODE,
            TypeKind::RawPointer {
                pointee: NODE,
                mutable: true,
            },
            4,
            false,
        );
        raw_node.js.pointer_form = Some(PointerForm::Slot);
        interns.types.insert(raw_node);

        let mut vec_u32 = make_desc(
            VEC_U32,
            TypeKind::Adt {
                def: VEC_DEF,
                form: NominalForm::Struct,
                args: vec![U32],
                variants: vec![VariantDescriptor {
                    name: vec_k,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(len_k),
                        ty: U32,
                        offset: 0,
                    }],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            12,
            true,
        );
        vec_u32.meta.candidate = RepresentationCandidate::Vec;
        vec_u32.meta.allocator = AllocatorIdentity::Global;
        interns.types.insert(vec_u32);

        let mut ref_mut_vec_u32 = make_desc(
            REF_MUT_VEC_U32,
            TypeKind::Reference {
                pointee: VEC_U32,
                mutable: true,
            },
            4,
            false,
        );
        ref_mut_vec_u32.js.pointer_form = Some(PointerForm::Thin);
        interns.types.insert(ref_mut_vec_u32);

        interns
    }

    fn make_link_item(
        name: &str,
        item_id: ItemId,
        root: bool,
        interns: Interns,
        sig: Signature,
    ) -> LinkItem {
        let mut jir = LinkJir::default();
        jir.use_interns(interns);
        jir.body = Some(Body {
            item: item_id,
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: sig,
            locals: Vec::new(),
            blocks: Vec::new(),
        });
        let mut linkage = Linkage::internal();
        linkage.root = root;
        LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            crate_name: "arena_test".to_owned(),
            debug_path: format!("arena_test::{name}"),
            js: String::new(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage,
            order: None,
            map: Default::default(),
            dts: None,
            optimization: Default::default(),
            allocator_provider: AllocatorProvider::Default,
            jir,
        }
    }

    fn std_call_op(
        b: &mut FuncBuilder,
        blk: RBlockId,
        std_op: StandardOperation,
        args: Vec<ValueId>,
        ret_ty: Option<TypeId>,
    ) -> Option<ValueId> {
        let kind = OpKind::Call {
            callee: Callee::Direct {
                item: ItemId {
                    definition: DefinitionId {
                        crate_id: 2,
                        local_hash: 9000,
                    },
                    instantiation: 0,
                },
                link_name: rcgjs_object::jir::StrId(5),
            },
            function: None,
            args,
            track_caller: None,
            semantic: CallSemantic::Standard(std_op),
            host: None,
        };
        if let Some(ty) = ret_ty {
            Some(b.op(blk, kind, ty, NO_ORIGIN))
        } else {
            b.effect(blk, kind, NO_ORIGIN);
            None
        }
    }

    #[derive(Clone, Copy)]
    enum ArenaMutation {
        Clean,
        VecPop,
        VecTruncate,
        VecClear,
        VecResize,
        UnverifiedPrevStore,
        SubsliceView,
        RawPointerView,
        RootExposesElem,
    }

    fn run_arena_scenario(mode: ArenaMutation) -> ProgramBoundsFacts {
        let interns = make_arena_interns();
        let entry_id = ItemId {
            definition: DefinitionId {
                crate_id: 99,
                local_hash: 1,
            },
            instantiation: 0,
        };
        let sig = if matches!(mode, ArenaMutation::RootExposesElem) {
            Signature {
                abi: AbiSignature {
                    parameters: 2,
                    ..AbiSignature::default()
                },
                arguments: vec![U32, REF_SLICE_NODE],
                result: U32,
            }
        } else {
            Signature {
                abi: AbiSignature {
                    parameters: 1,
                    ..AbiSignature::default()
                },
                arguments: vec![U32],
                result: U32,
            }
        };
        let mut b = FuncBuilder::new(entry_id, sig.clone());
        let entry = FuncBuilder::ENTRY;
        let ext = b.param(U32, NO_ORIGIN);
        if matches!(mode, ArenaMutation::RootExposesElem) {
            let _exposed = b.param(REF_SLICE_NODE, NO_ORIGIN);
        }
        let nodes_cell = b.cell(VEC_NODE, true, NO_ORIGIN);
        b.effect(entry, OpKind::CellAlloc(nodes_cell), NO_ORIGIN);
        let init_vec = std_call_op(
            &mut b,
            entry,
            StandardOperation::VecNew,
            vec![],
            Some(VEC_NODE),
        )
        .expect("vec new");
        b.effect(
            entry,
            OpKind::CellStore {
                cell: nodes_cell,
                v: init_vec,
            },
            NO_ORIGIN,
        );
        let vec_ref = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Cell(nodes_cell),
                path: Vec::new(),
                mutable: true,
            },
            REF_MUT_VEC_NODE,
            NO_ORIGIN,
        );
        let pos = std_call_op(
            &mut b,
            entry,
            StandardOperation::VecLen,
            vec![vec_ref],
            Some(U32),
        )
        .expect("len");
        let sentinel = b.op(
            entry,
            OpKind::Const(ConstTree::Scalar {
                bits: u32::MAX as u128,
                bytes: 4,
            }),
            U32,
            NO_ORIGIN,
        );
        let node = b.op(
            entry,
            OpKind::Aggregate {
                ty: NODE,
                variant: 0,
                active_field: None,
                fields: vec![pos, sentinel],
            },
            NODE,
            NO_ORIGIN,
        );
        std_call_op(
            &mut b,
            entry,
            StandardOperation::VecPush,
            vec![vec_ref, node],
            None,
        );
        let sl = std_call_op(
            &mut b,
            entry,
            StandardOperation::VecAsMutSlice,
            vec![vec_ref],
            Some(REF_MUT_SLICE_NODE),
        )
        .expect("as_mut_slice");
        let sl_len = b.op(
            entry,
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v: sl,
                ty: REF_MUT_SLICE_NODE,
            },
            U32,
            NO_ORIGIN,
        );
        let zero = b.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            U32,
            NO_ORIGIN,
        );
        let ok0 = b.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Lt,
                l: zero,
                r: sl_len,
                ty: U32,
            },
            BOOL,
            NO_ORIGIN,
        );
        b.effect(
            entry,
            OpKind::Assert {
                cond: ok0,
                expected: true,
                msg: RAssertMessage::BoundsCheck {
                    length: sl_len,
                    index: zero,
                },
                track_caller: None,
            },
            NO_ORIGIN,
        );
        // Valid write: `sl[0].next_z = pos` (pre-push len of `nodes`, proven `< nodes.len`).
        let next_z_ptr = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(sl),
                path: vec![
                    Proj::Deref,
                    Proj::Index(zero),
                    Proj::Field { field: 1, ty: U32 },
                ],
                mutable: true,
            },
            REF_MUT_U32,
            NO_ORIGIN,
        );
        b.effect(
            entry,
            OpKind::StoreThrough {
                ptr: next_z_ptr,
                v: pos,
            },
            NO_ORIGIN,
        );

        match mode {
            ArenaMutation::Clean | ArenaMutation::RootExposesElem => {}
            ArenaMutation::VecPop => {
                std_call_op(
                    &mut b,
                    entry,
                    StandardOperation::VecPop,
                    vec![vec_ref],
                    None,
                );
            }
            ArenaMutation::VecTruncate => {
                std_call_op(
                    &mut b,
                    entry,
                    StandardOperation::VecTruncate,
                    vec![vec_ref, zero],
                    None,
                );
            }
            ArenaMutation::VecClear => {
                std_call_op(
                    &mut b,
                    entry,
                    StandardOperation::VecClear,
                    vec![vec_ref],
                    None,
                );
            }
            ArenaMutation::VecResize => {
                std_call_op(
                    &mut b,
                    entry,
                    StandardOperation::VecResize,
                    vec![vec_ref, zero],
                    None,
                );
            }
            ArenaMutation::UnverifiedPrevStore => {
                let prev_ptr = b.op(
                    entry,
                    OpKind::BorrowView {
                        root: Root::Value(sl),
                        path: vec![
                            Proj::Deref,
                            Proj::Index(zero),
                            Proj::Field { field: 0, ty: U32 },
                        ],
                        mutable: true,
                    },
                    REF_MUT_U32,
                    NO_ORIGIN,
                );
                b.effect(
                    entry,
                    OpKind::StoreThrough {
                        ptr: prev_ptr,
                        v: ext,
                    },
                    NO_ORIGIN,
                );
            }
            ArenaMutation::SubsliceView => {
                let _sub = b.op(
                    entry,
                    OpKind::SliceView { ptr: sl, len: zero },
                    REF_SLICE_NODE,
                    NO_ORIGIN,
                );
            }
            ArenaMutation::RawPointerView => {
                let _raw = b.op(
                    entry,
                    OpKind::RawView {
                        root: Root::Value(sl),
                        path: vec![Proj::Deref, Proj::Index(zero)],
                        mutable: true,
                    },
                    RAW_NODE,
                    NO_ORIGIN,
                );
            }
        }

        b.terminate(entry, RTerm::Return(Some(zero)));
        let func = b.finish();

        let name = JsName::new("arena_entry".to_owned());
        let mut table = BTreeMap::new();
        table.insert(
            name.clone(),
            make_link_item("arena_entry", entry_id, true, interns, sig),
        );
        let mut funcs = BTreeMap::new();
        funcs.insert(name.clone(), Ok(func));
        let mut live = BTreeSet::new();
        live.insert(name);
        let opts = Opts::default();
        let heap = HeapPlan::default();
        analyze(&table, &funcs, &BTreeMap::new(), &live, &opts, &heap)
    }

    #[test]
    fn monotone_arena_proves_always_valid_and_sentinel_fields() {
        let facts = run_arena_scenario(ArenaMutation::Clean);
        let inv = facts
            .arenas
            .get(&NODE_DEF)
            .expect("clean monotone arena must be proven");
        assert_eq!(inv.fields.get(&0), Some(&ArenaFieldKind::AlwaysValid));
        assert_eq!(
            inv.fields.get(&1),
            Some(&ArenaFieldKind::ValidOrSentinel {
                sentinel: u32::MAX as u128
            })
        );
    }

    #[test]
    fn adversarial_arena_shrink_escape_subslice_and_raw_disqualify() {
        for mode in [
            ArenaMutation::VecPop,
            ArenaMutation::VecTruncate,
            ArenaMutation::VecClear,
            ArenaMutation::VecResize,
            ArenaMutation::SubsliceView,
            ArenaMutation::RawPointerView,
            ArenaMutation::RootExposesElem,
        ] {
            let facts = run_arena_scenario(mode);
            assert!(
                facts.arenas.is_empty(),
                "adversarial arena mutation must disqualify NODE_DEF"
            );
        }
    }

    #[test]
    fn adversarial_unverified_external_write_kills_only_that_field() {
        let facts = run_arena_scenario(ArenaMutation::UnverifiedPrevStore);
        let inv = facts
            .arenas
            .get(&NODE_DEF)
            .expect("other valid field on NODE_DEF remains sound");
        assert!(
            !inv.fields.contains_key(&0),
            "field 0 (prev) received unchecked external param and must be rejected"
        );
        assert_eq!(
            inv.fields.get(&1),
            Some(&ArenaFieldKind::ValidOrSentinel {
                sentinel: u32::MAX as u128
            })
        );
    }

    #[test]
    fn return_vec_len_proves_one_shl_and_cond_bool_and_rejects_mutated_vec() {
        let interns = make_arena_interns();

        // 1. Clean `fn make_pow2(mb: u32) -> Vec<u32>` returning `NativeVecFromElemZeroed(1 << mb)`.
        let pow2_id = ItemId {
            definition: DefinitionId {
                crate_id: 99,
                local_hash: 20,
            },
            instantiation: 0,
        };
        let pow2_sig = Signature {
            abi: AbiSignature {
                parameters: 1,
                ..AbiSignature::default()
            },
            arguments: vec![U32],
            result: VEC_U32,
        };
        let mut b1 = FuncBuilder::new(pow2_id, pow2_sig.clone());
        let mb1 = b1.param(U32, NO_ORIGIN);
        let one1 = b1.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            U32,
            NO_ORIGIN,
        );
        let shl1 = b1.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: BinaryOp::Shl,
                l: one1,
                r: mb1,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        let v1 = std_call_op(
            &mut b1,
            FuncBuilder::ENTRY,
            StandardOperation::NativeVecFromElemZeroed,
            vec![shl1],
            Some(VEC_U32),
        )
        .expect("zeroed vec");
        b1.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(v1)));

        // 2. Clean conditional `fn h_map_like(mb: u32, r: bool) -> Vec<u32>`.
        let cond_id = ItemId {
            definition: DefinitionId {
                crate_id: 99,
                local_hash: 21,
            },
            instantiation: 0,
        };
        let cond_sig = Signature {
            abi: AbiSignature {
                parameters: 2,
                ..AbiSignature::default()
            },
            arguments: vec![U32, BOOL],
            result: VEC_U32,
        };
        let mut b2 = FuncBuilder::new(cond_id, cond_sig.clone());
        let mb2 = b2.param(U32, NO_ORIGIN);
        let flag = b2.param(BOOL, NO_ORIGIN);
        let b_true = b2.block();
        let b_false = b2.block();
        b2.terminate(
            FuncBuilder::ENTRY,
            RTerm::Switch {
                scrut: flag,
                scrut_ty: BOOL,
                cases: vec![(0, b_false, Vec::new())],
                default: (b_true, Vec::new()),
            },
        );
        let one_t = b2.op(
            b_true,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            U32,
            NO_ORIGIN,
        );
        let shl_t = b2.op(
            b_true,
            OpKind::Binary {
                op: BinaryOp::Shl,
                l: one_t,
                r: mb2,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        let vt = std_call_op(
            &mut b2,
            b_true,
            StandardOperation::NativeVecFromElemZeroed,
            vec![shl_t],
            Some(VEC_U32),
        )
        .expect("vt");
        b2.terminate(b_true, RTerm::Return(Some(vt)));
        let vf = std_call_op(
            &mut b2,
            b_false,
            StandardOperation::NativeVecFromElemZeroed,
            vec![mb2],
            Some(VEC_U32),
        )
        .expect("vf");
        b2.terminate(b_false, RTerm::Return(Some(vf)));

        // 3. Adversarial: allocates `1 << mb`, then pushes one more element -> must NOT summarize!
        let bad_id = ItemId {
            definition: DefinitionId {
                crate_id: 99,
                local_hash: 22,
            },
            instantiation: 0,
        };
        let mut b3 = FuncBuilder::new(bad_id, pow2_sig.clone());
        let mb3 = b3.param(U32, NO_ORIGIN);
        let cell3 = b3.cell(VEC_U32, true, NO_ORIGIN);
        b3.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(cell3), NO_ORIGIN);
        let one3 = b3.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            U32,
            NO_ORIGIN,
        );
        let shl3 = b3.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: BinaryOp::Shl,
                l: one3,
                r: mb3,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        let v3 = std_call_op(
            &mut b3,
            FuncBuilder::ENTRY,
            StandardOperation::NativeVecFromElemZeroed,
            vec![shl3],
            Some(VEC_U32),
        )
        .expect("v3");
        b3.effect(
            FuncBuilder::ENTRY,
            OpKind::CellStore { cell: cell3, v: v3 },
            NO_ORIGIN,
        );
        let v3_ref = b3.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Cell(cell3),
                path: Vec::new(),
                mutable: true,
            },
            REF_MUT_VEC_U32,
            NO_ORIGIN,
        );
        std_call_op(
            &mut b3,
            FuncBuilder::ENTRY,
            StandardOperation::VecPush,
            vec![v3_ref, one3],
            None,
        );
        let loaded = b3.op(
            FuncBuilder::ENTRY,
            OpKind::CellLoad(cell3),
            VEC_U32,
            NO_ORIGIN,
        );
        b3.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(loaded)));

        let n1 = JsName::new("make_pow2".to_owned());
        let n2 = JsName::new("h_map_like".to_owned());
        let n3 = JsName::new("make_pow2_mutated".to_owned());
        let mut table = BTreeMap::new();
        table.insert(
            n1.clone(),
            make_link_item(
                "make_pow2",
                pow2_id,
                true,
                interns.clone(),
                pow2_sig.clone(),
            ),
        );
        table.insert(
            n2.clone(),
            make_link_item("h_map_like", cond_id, true, interns.clone(), cond_sig),
        );
        table.insert(
            n3.clone(),
            make_link_item("make_pow2_mutated", bad_id, true, interns, pow2_sig),
        );
        let mut funcs = BTreeMap::new();
        funcs.insert(n1.clone(), Ok(b1.finish()));
        funcs.insert(n2.clone(), Ok(b2.finish()));
        funcs.insert(n3.clone(), Ok(b3.finish()));
        let live: BTreeSet<JsName> = [n1, n2, n3].into_iter().collect();

        let facts = analyze(
            &table,
            &funcs,
            &BTreeMap::new(),
            &live,
            &Opts::default(),
            &HeapPlan::default(),
        );
        assert_eq!(
            facts.return_vec_len.get(&pow2_id),
            Some(&ReturnVecLen::OneShlParam {
                shift_param: 0,
                bits: 32
            })
        );
        assert_eq!(
            facts.return_vec_len.get(&cond_id),
            Some(&ReturnVecLen::CondBoolParam {
                cond_param: 1,
                when_true: Box::new(ReturnVecLen::OneShlParam {
                    shift_param: 0,
                    bits: 32
                }),
                when_false: Box::new(ReturnVecLen::Param(0)),
            })
        );
        assert!(
            !facts.return_vec_len.contains_key(&bad_id),
            "mutated Vec with extra push must NOT be summarized with initial alloc length"
        );
    }
}
