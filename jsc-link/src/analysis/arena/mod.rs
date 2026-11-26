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

mod context;
mod fixpoint;
mod param_bounds;
mod preconditions;
mod return_len;
mod static_arrays;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::jir::decline::{Decline, EncodeDecline};
use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::repr::{CellId, Dominators, Func, OpKind, Proj, RBlockId, Root, ValueId};
use rcgjs_object::jir::semantic::{
    BodyKind, BodySemantic, CallSemantic, Callee, ConstTree, DefinitionId, DropGlue,
    GlobalReference, ItemId, StandardOperation,
};
use rcgjs_object::jir::summary::Availability;
use rcgjs_object::jir::types::{NominalForm, TypeId, TypeKind, TypeTable};
use rcgjs_object::object::ItemKind;

use crate::analysis::heap::{self, global_vec_elem_ty, HeapPlan};
use crate::analysis::reach::{self, ReachPolicy};
use crate::item::{JsName, LinkItem};
use crate::lower::js::value;
use crate::opts::Opts;
use crate::plan::native_types::NativeBody;

pub(crate) use param_bounds::find_counted_push_loop_len;

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

pub(super) type LiveItem<'a> = (&'a JsName, &'a LinkItem, &'a Func, &'a Interns);

/// Whole-program metadata gathered once for Stage 1 (arenas) and Stage 2 (summaries).
pub(super) struct LiveProgram<'a> {
    pub live_items: Vec<LiveItem<'a>>,
    pub by_item_id: BTreeMap<ItemId, usize>,
    pub trusted_std_bodies: BTreeSet<ItemId>,
    pub drop_glue_items: BTreeSet<ItemId>,
    pub root_or_exported: BTreeSet<ItemId>,
    pub address_taken: BTreeSet<ItemId>,
    pub poisoned_defs: BTreeSet<DefinitionId>,
    pub all_arenas_poisoned: bool,
}

impl LiveProgram<'_> {
    pub(super) fn is_user_item(&self, item: ItemId) -> bool {
        !self.trusted_std_bodies.contains(&item) && !self.drop_glue_items.contains(&item)
    }

    pub(super) fn is_closed_internal(&self, item: ItemId) -> bool {
        self.is_user_item(item)
            && !self.root_or_exported.contains(&item)
            && !self.address_taken.contains(&item)
    }
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

    // 1. Filter out dead Tier-A / native-string / native-vec fallback bodies.
    let filtered_live = filter_live_items(table, funcs, live, opts, heap);

    // 2. Immutable static unsigned scalar array bounds & lengths.
    static_arrays::collect_static_array_bounds(table, &filtered_live, &mut facts);

    // 3. Gather live function metadata, trust boundaries, and completeness.
    let program = gather_live_program(table, funcs, native_bodies, &filtered_live, opts, heap);

    // 4. Stage 1: Whole-program monotone-arena discovery & verification.
    if !program.all_arenas_poisoned {
        fixpoint::analyze_arenas(&program, &mut facts);
    }

    // 5. Stage 2: Interprocedural return-length, return-elem-bound, param-arena,
    //    and equal-slice-length summaries (never fed back into Stage 1!).
    return_len::analyze_return_vec_lens(&program, &mut facts);
    return_len::analyze_return_vec_elem_bounds(&program, &mut facts);
    param_bounds::analyze_param_arena_indices(&program, &mut facts);
    param_bounds::analyze_param_slice_eq_len(&program, &mut facts);

    facts
}

fn filter_live_items(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    live: &BTreeSet<JsName>,
    opts: &Opts,
    heap: &HeapPlan,
) -> BTreeSet<JsName> {
    let mut filtered_live = heap::filter_live_for_repr(table, funcs, live, opts, heap);
    if heap.native_vec_enabled {
        filtered_live = prune_dead_native_vec_drop_helpers(table, funcs, &filtered_live, opts);
    }
    filtered_live
}

fn is_synthesized_item(item: &LinkItem) -> bool {
    item.jir.summary.is_none()
        || matches!(
            item.jir.summary.as_ref().map(|s| &s.availability),
            Some(Availability::Unavailable(Decline::Encode(
                EncodeDecline::Synthesized
            )))
        )
}

fn gather_live_program<'a>(
    table: &'a BTreeMap<JsName, LinkItem>,
    funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
    native_bodies: &BTreeMap<JsName, NativeBody>,
    filtered_live: &BTreeSet<JsName>,
    opts: &Opts,
    heap: &HeapPlan,
) -> LiveProgram<'a> {
    let mut live_items: Vec<LiveItem<'a>> = Vec::new();
    let mut by_item_id: BTreeMap<ItemId, usize> = BTreeMap::new();
    let mut std_body_items: BTreeMap<ItemId, StandardOperation> = BTreeMap::new();
    let mut drop_glue_items: BTreeSet<ItemId> = BTreeSet::new();
    let mut root_or_exported: BTreeSet<ItemId> = BTreeSet::new();
    let mut address_taken: BTreeSet<ItemId> = BTreeSet::new();
    let mut all_arenas_poisoned = false;
    let mut poisoned_defs: BTreeSet<DefinitionId> = BTreeSet::new();

    for name in filtered_live {
        let Some((key, item)) = table.get_key_value(name) else {
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
                );
                if let BodyKind::Static { init, .. } = &body.kind {
                    collect_const_fn_refs(init, &mut address_taken);
                }
            } else if !is_synthesized_item(item) || item.js.contains(".buf") {
                all_arenas_poisoned = true;
            }
            continue;
        }
        if item.kind != ItemKind::Fn || is_skipped_stdlib_hook(item, native_bodies, heap) {
            continue;
        }
        let body_sem = item.jir.body.as_ref().map(|b| b.semantic);
        let Some(interns) = item.jir.interns() else {
            if !is_synthesized_item(item) {
                all_arenas_poisoned = true;
            }
            continue;
        };
        let Some(Ok(func)) = funcs.get(name) else {
            // Declined or unlowered live function: conservatively poison any ADT its crate
            // mentions, or everything if no body metadata is available.
            if let Some(body) = item.jir.body.as_ref() {
                if !matches!(body.semantic, BodySemantic::DropGlue(_)) {
                    for &arg_ty in &body.signature.arguments {
                        collect_all_adt_defs_in_ty(&interns.types, arg_ty, &mut poisoned_defs);
                    }
                    collect_all_adt_defs_in_ty(
                        &interns.types,
                        body.signature.result,
                        &mut poisoned_defs,
                    );
                    for local in &body.locals {
                        collect_all_adt_defs_in_ty(&interns.types, local.ty, &mut poisoned_defs);
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
        live_items.push((key, item, func, interns));
    }

    let trusted_std_bodies = compute_trusted_std_bodies(
        &live_items,
        &std_body_items,
        &root_or_exported,
        &mut address_taken,
    );

    prune_trusted_std_only_helpers(
        &mut live_items,
        &mut by_item_id,
        &root_or_exported,
        &trusted_std_bodies,
    );

    LiveProgram {
        live_items,
        by_item_id,
        trusted_std_bodies,
        drop_glue_items,
        root_or_exported,
        address_taken,
        poisoned_defs,
        all_arenas_poisoned,
    }
}

/// Track address-taken functions and verify every call to a `BodySemantic::Standard` body
/// uses the matching `CallSemantic::Standard(op)`.
fn compute_trusted_std_bodies(
    live_items: &[LiveItem<'_>],
    std_body_items: &BTreeMap<ItemId, StandardOperation>,
    root_or_exported: &BTreeSet<ItemId>,
    address_taken: &mut BTreeSet<ItemId>,
) -> BTreeSet<ItemId> {
    let mut non_standard_called_std: BTreeSet<ItemId> = BTreeSet::new();
    for &(_, _, func, _) in live_items {
        for block in &func.blocks {
            for op in &block.ops {
                match &op.kind {
                    OpKind::Const(tree) => collect_const_fn_refs(tree, address_taken),
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
    std_body_items
        .keys()
        .copied()
        .filter(|id| {
            !root_or_exported.contains(id)
                && !address_taken.contains(id)
                && !non_standard_called_std.contains(id)
        })
        .collect()
}

struct UserItemReachPolicy<'a> {
    live_items: &'a [LiveItem<'a>],
    by_item_id: &'a BTreeMap<ItemId, usize>,
    trusted_std_bodies: &'a BTreeSet<ItemId>,
}

impl ReachPolicy<usize> for UserItemReachPolicy<'_> {
    fn expand(&mut self, &idx: &usize, enqueue: &mut dyn FnMut(usize)) {
        let (_, _, func, _) = self.live_items[idx];
        if self.trusted_std_bodies.contains(&func.item) {
            return;
        }
        for block in &func.blocks {
            for op in &block.ops {
                match &op.kind {
                    OpKind::Call {
                        callee: Callee::Direct { item, .. },
                        ..
                    }
                    | OpKind::Drop {
                        glue: DropGlue::Direct { item, .. },
                        ..
                    } => {
                        if let Some(&ci) = self.by_item_id.get(item) {
                            enqueue(ci);
                        }
                    }
                    OpKind::Const(tree) => {
                        let mut refs = BTreeSet::new();
                        collect_const_fn_refs(tree, &mut refs);
                        for r in refs {
                            if let Some(&ci) = self.by_item_id.get(&r) {
                                enqueue(ci);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Excludes private stdlib helpers reachable ONLY from inside `trusted_std_bodies` (e.g.
/// `alloc$vec$impl_3_vec_push` called solely inside `Vec::push`'s `Standard(VecPush)` body).
/// Any helper called directly by user code remains reachable and is fully checked.
fn prune_trusted_std_only_helpers<'a>(
    live_items: &mut Vec<LiveItem<'a>>,
    by_item_id: &mut BTreeMap<ItemId, usize>,
    root_or_exported: &BTreeSet<ItemId>,
    trusted_std_bodies: &BTreeSet<ItemId>,
) {
    if root_or_exported.is_empty() {
        return;
    }
    let seeds: Vec<usize> = live_items
        .iter()
        .enumerate()
        .filter_map(|(idx, &(_, _, func, _))| root_or_exported.contains(&func.item).then_some(idx))
        .collect();
    let reachable_idx = {
        let mut policy = UserItemReachPolicy {
            live_items,
            by_item_id,
            trusted_std_bodies,
        };
        reach::walk_reachable(seeds, &mut policy)
    };
    if reachable_idx.is_empty() {
        return;
    }
    let old = std::mem::take(live_items);
    by_item_id.clear();
    for (idx, entry) in old.into_iter().enumerate() {
        if reachable_idx.contains(&idx) {
            let new_idx = live_items.len();
            by_item_id.insert(entry.2.item, new_idx);
            live_items.push(entry);
        }
    }
}

struct PruneVecDropPolicy<'a> {
    table: &'a BTreeMap<JsName, LinkItem>,
    funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
    live: &'a BTreeSet<JsName>,
}

impl ReachPolicy<JsName> for PruneVecDropPolicy<'_> {
    fn expand(&mut self, name: &JsName, enqueue: &mut dyn FnMut(JsName)) {
        let Some(item) = self.table.get(name) else {
            return;
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
                if !self.live.contains(r) {
                    continue;
                }
                let is_drop_glue = self.table.get(r).is_some_and(|ri| {
                    matches!(
                        ri.jir.body.as_ref().map(|b| b.semantic),
                        Some(BodySemantic::DropGlue(_))
                    ) && !ri.debug_path.contains("raw_vec::RawVec")
                        && !ri.name.as_str().contains("raw_vec$RawVec")
                });
                if is_drop_glue {
                    enqueue(r.clone());
                }
            }
            return;
        }
        if let (Some(Ok(func)), Some(interns)) = (self.funcs.get(name), item.jir.interns()) {
            for block in &func.blocks {
                for op in &block.ops {
                    if let OpKind::Call {
                        callee: Callee::Direct { link_name, .. },
                        ..
                    }
                    | OpKind::Drop {
                        glue: DropGlue::Direct { link_name, .. },
                        ..
                    } = &op.kind
                    {
                        if let Some(jn) = interns.str(*link_name).and_then(|s| self.live.get(s)) {
                            enqueue(jn.clone());
                        }
                    }
                }
            }
        }
        for r in &item.refs {
            if !self.live.contains(r) {
                continue;
            }
            if heap::is_global_vec_drop_glue(Some(item))
                && self.table.get(r).is_some_and(|ri| {
                    ri.debug_path.contains("raw_vec::RawVec")
                        || ri.name.as_str().contains("raw_vec$RawVec")
                })
            {
                continue;
            }
            enqueue(r.clone());
        }
    }
}

fn prune_dead_native_vec_drop_helpers(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    live: &BTreeSet<JsName>,
    opts: &Opts,
) -> BTreeSet<JsName> {
    let seeds: Vec<JsName> = live
        .iter()
        .filter(|name| {
            table.get(*name).is_some_and(|item| {
                item.linkage.root
                    || item.linkage.exported
                    || opts.roots.iter().any(|r| r == name.as_str())
            })
        })
        .cloned()
        .collect();
    if seeds.is_empty() {
        return live.clone();
    }
    let mut policy = PruneVecDropPolicy { table, funcs, live };
    reach::walk_reachable(seeds, &mut policy)
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

pub(super) fn collect_const_fn_refs(tree: &ConstTree, out: &mut BTreeSet<ItemId>) {
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

pub(super) fn collect_all_adt_defs_in_ty(
    table: &TypeTable,
    ty: TypeId,
    out: &mut BTreeSet<DefinitionId>,
) {
    table.visit_types(
        ty,
        rcgjs_object::jir::types::TypeWalk::WithAdtArgs,
        &mut |cur| {
            if let Some(def) = adt_def_of(table, cur) {
                out.insert(def);
            }
        },
    );
}

pub(super) fn ty_reaches_def(table: &TypeTable, ty: TypeId, target: DefinitionId) -> bool {
    let mut found = false;
    table.visit_types(
        ty,
        rcgjs_object::jir::types::TypeWalk::WithAdtArgs,
        &mut |cur| {
            if !found && adt_def_of(table, cur) == Some(target) {
                found = true;
            }
        },
    );
    found
}

pub(super) fn ty_contains_vec_of_def(table: &TypeTable, ty: TypeId, target: DefinitionId) -> bool {
    let mut found = false;
    table.visit_types(
        ty,
        rcgjs_object::jir::types::TypeWalk::ByValue,
        &mut |cur| {
            if !found && vec_elem_def(table, cur) == Some(target) {
                found = true;
            }
        },
    );
    found
}

pub(super) fn adt_def_of(table: &TypeTable, ty: TypeId) -> Option<DefinitionId> {
    match table.get(table.peeled(ty)).map(|d| &d.kind) {
        Some(TypeKind::Adt { def, .. }) => Some(*def),
        _ => None,
    }
}

pub(super) fn slice_elem_def(table: &TypeTable, ty: TypeId) -> Option<DefinitionId> {
    let pt = value::pointee_of(table, ty)?;
    match table.get(table.peeled(pt)).map(|d| &d.kind) {
        Some(TypeKind::Slice(elem)) => adt_def_of(table, *elem),
        _ => None,
    }
}

pub(super) fn vec_elem_def(table: &TypeTable, ty: TypeId) -> Option<DefinitionId> {
    let elem = global_vec_elem_ty(table, ty)?;
    adt_def_of(table, elem)
}

pub(super) fn vec_ref_elem_def(table: &TypeTable, ty: TypeId) -> Option<DefinitionId> {
    let pt = value::pointee_of(table, ty)?;
    vec_elem_def(table, pt)
}

/// If `ty` is a struct (or `&Struct`/`&mut Struct`) having a field of type `Vec<Elem, Global>`,
/// returns `(struct_def, field_idx)` for all such fields.
pub(super) fn holder_vec_fields(
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

pub(super) fn is_unsigned_index_ty(table: &TypeTable, ty: TypeId) -> Option<u32> {
    match table.int_info(table.peeled(ty)) {
        Some((false, bits)) if bits >= 16 => Some(bits),
        _ => None,
    }
}

pub(super) fn is_unsigned_vec_ty(table: &TypeTable, ty: TypeId) -> Option<u32> {
    let elem = global_vec_elem_ty(table, ty)?;
    is_unsigned_index_ty(table, elem)
}

pub(super) fn width_max(bits: u32) -> u128 {
    if bits >= 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    }
}

pub(super) fn mask_bits(bits: u128, bytes: u8) -> u128 {
    match bytes {
        0 => 0,
        1..=15 => bits & ((1u128 << (bytes as u32 * 8)) - 1),
        _ => bits,
    }
}

pub(super) fn func_mentions_def(func: &Func, types: &TypeTable, def: DefinitionId) -> bool {
    func.values.iter().any(|v| ty_reaches_def(types, v.ty, def))
        || func.cells.iter().any(|c| ty_reaches_def(types, c.ty, def))
}

pub(super) fn peel_copy_move_local(
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

pub(super) fn is_empty_vec_val(
    func: &Func,
    defs: &[Option<(RBlockId, usize)>],
    v: ValueId,
) -> bool {
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

pub(super) fn propagate_cell_borrows(
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
