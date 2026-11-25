//! Whole-type native lowering for `Box<T, Global>` drop-glue elision and order-unobserved
//! `BTreeMap<K, V, Global>` -> JS `Map` (replaces the Gen-1 B1 min-cut planner).
//!
//! Gen-2 universal `String` (`{ s, cap }`) and `Vec` (`{ buf, len }` / `{ buf, len, cap }`)
//! in `analysis/heap.rs` and `lower/js/call.rs` now serve all release profiles directly.
//! This module retains the two remaining whole-type standard-container optimizations:
//!
//! 1. **Native `Box<T, Global>` drop-glue elision (`Carrier::NativeBox`)**: when the program uses
//!    the default allocator and no reachable code derives raw pointers or observes allocation
//!    identity of `Box<T>`, replace `DropGlue(Box<T>)` with `drop::box_drop` (calling exact
//!    structural pointee glue if `T` needs drop and leaving deallocation to JS GC). Because
//!    `Box<T>` keeps its exact baseline `{ buf, off }` shape, direct and indirect callers agree
//!    without any private `$n1` version cloning.
//! 2. **Order-unobserved `BTreeMap<K, V, Global>` -> JS `Map` (`Carrier::NativeMap`)**: when
//!    every reachable use of a concrete `BTreeMap<K, V, Global>` `TypeId` only passes through the
//!    10 supported order-free operations (`new`, `len`, `is_empty`, `clear`, `contains_key`, `get`,
//!    `get_mut`, `insert`, `remove`, `drop`) plus plain moves/borrows/field containment, replace
//!    all reachable `BTreeMap*` and `DropGlue` bodies for that type with JS `Map` bodies.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use rcgjs_object::item::AllocatorProvider;
use rcgjs_object::jir::decline::Decline;
use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::repr::{Carrier, Func, OpKind, Proj, Root};
use rcgjs_object::jir::semantic::{
    BodySemantic, CallSemantic, Callee, ConstTree, StandardOperation,
};
use rcgjs_object::jir::types::{
    AllocatorIdentity, NominalAbi, PinnedBoundary, RepresentationCandidate, TypeId, TypeKind,
    TypeTable,
};
use rcgjs_object::jsast;

use crate::item::{JsName, LinkItem};
use crate::link::lane::{ReprBlock, ReprDecision};
use crate::lower::semantic_ops::{self, LowerCx, SemanticDecline};

/// What one lane pass hands the whole-type native-container planner.
pub(crate) struct ReprCx<'a> {
    pub opts: &'a crate::opts::Opts,
    pub drop_glue: &'a crate::lower::js::host::DropGlueIndex,
    pub heap: &'a crate::analysis::heap::HeapPlan,
}

/// A committed standard-operation item's replacement body, installed by phase C.
#[derive(Clone, Debug)]
pub(crate) struct NativeBody {
    pub parameters: Vec<String>,
    pub statements: Vec<jsast::Stmt>,
}

/// Everything the whole-type planner decided. `bodies` and `touched` are empty unless applied.
#[derive(Debug, Default)]
pub(crate) struct ReprOutcome {
    pub block: ReprBlock,
    /// Every item whose printed body this plan replaces.
    pub touched: BTreeSet<JsName>,
    /// The standard-operation / drop-glue items phase C prints instead of their `Func`.
    pub bodies: BTreeMap<JsName, NativeBody>,
    /// True when at least one `BTreeMap<K, V>` type committed to JS `Map`, so nested struct
    /// copies in `value::clone_expr_with` and `ptr::try_inline_overwrite` guard against JS `Map`.
    pub has_native_map: bool,
}

impl ReprOutcome {
    /// Keeps the proposed and validated accounting while applying no rewrite (`js-opt=report`).
    pub fn unapplied(&self) -> ReprOutcome {
        let mut block = self.block.clone();
        block.applied = 0;
        block.emits = false;
        ReprOutcome {
            block,
            touched: BTreeSet::new(),
            bodies: BTreeMap::new(),
            has_native_map: false,
        }
    }

    /// What a lane retry falls back to: accounting kept, commit disowned.
    pub fn emptied(&self) -> ReprOutcome {
        let mut block = self.block.clone();
        block.committed = 0;
        block.adapters = 0;
        block.versions = 0;
        block.helper_bytes = 0;
        block.applied = 0;
        block.emits = true;
        block.bytes_after = block.bytes_before;
        block.score_after = block.score_before;
        for decision in &mut block.decisions {
            if decision.decline.is_none() && decision.chosen != "baseline" {
                decision.chosen = "baseline".to_owned();
                decision.decline = Some(
                    "version.carrier-commit-rollback a phase-C decline emptied the plan".to_owned(),
                );
            }
        }
        for export in &mut block.exports {
            export.chosen = "baseline".to_owned();
        }
        ReprOutcome {
            block,
            touched: BTreeSet::new(),
            bodies: BTreeMap::new(),
            has_native_map: false,
        }
    }
}

/// Makes the non-emitting table placeholder for a private clone (used by `plan/abi_flat.rs`).
pub(crate) fn clone_link_item(mut source: LinkItem, name: &JsName) -> LinkItem {
    source.name = name.clone();
    source.linkage = crate::item::Linkage::internal();
    source.order = None;
    source.dts = None;
    source.debug_path = format!("{} [native version]", source.debug_path);
    source
}

/// Final reachability for a hypothetical program, without diagnostics for missing extra roots.
pub(crate) fn measurement_reach(
    table: &BTreeMap<JsName, LinkItem>,
    extra_roots: &[String],
) -> Vec<JsName> {
    let mut queue: VecDeque<JsName> = table
        .iter()
        .filter(|(name, item)| {
            item.linkage.root || extra_roots.iter().any(|root| root == name.as_str())
        })
        .map(|(name, _)| name.clone())
        .collect();
    let mut seen = Vec::new();
    let mut in_set: BTreeSet<JsName> = queue.iter().cloned().collect();
    while let Some(name) = queue.pop_front() {
        seen.push(name.clone());
        let Some(item) = table.get(&name) else {
            continue;
        };
        for referenced in &item.refs {
            if table.contains_key(referenced) && in_set.insert(referenced.clone()) {
                queue.push_back(referenced.clone());
            }
        }
    }
    seen
}

/// Whole-type planner entry point for `Box<T, Global>` drop elision and order-unobserved
/// `BTreeMap<K, V, Global>` -> JS `Map`.
pub(crate) fn plan_and_commit(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    live: &BTreeSet<JsName>,
    cx: &ReprCx<'_>,
) -> ReprOutcome {
    let allocator_provider = table
        .values()
        .fold(AllocatorProvider::Unknown, |acc, item| {
            acc.merge(item.allocator_provider)
        });
    let default_allocator = allocator_provider == AllocatorProvider::Default;

    let mut outcome = ReprOutcome::default();
    outcome.block.emits = cx.opts.optimizer == crate::opts::OptimizerMode::On;
    outcome.block.families = 2;

    // 1. Index every reachable supported `BTreeMap*` / `DropGlue(BTreeMap)` item.
    let mut supported_map_items: BTreeMap<JsName, TypeId> = BTreeMap::new();
    let mut all_seen_maps: BTreeSet<TypeId> = BTreeSet::new();
    for name in live {
        let Some(item) = table.get(name) else {
            continue;
        };
        let (Some(body), Some(interns)) = (&item.jir.body, item.jir.interns()) else {
            continue;
        };
        if let Some(map_ty) =
            btree_map_ty_of_semantic(&interns.types, body.semantic, &body.signature)
        {
            all_seen_maps.insert(map_ty);
            supported_map_items.insert(name.clone(), map_ty);
        }
    }

    // 2. Fixpoint: compute the items reachable from program roots WITHOUT descending into the
    //    bodies of still-candidate `BTreeMap*` / `DropGlue(BTreeMap)` items (whose internal
    //    `alloc::collections::btree::*` helpers — `<BTreeMap as Drop>::drop`, `Entry`, node
    //    pointers, etc. — become dead once those items are replaced with JS `Map` bodies).
    //    If any candidate `BTreeMap` TypeId is disqualified by user code or fails `map.rs`
    //    legality, demote it and re-run so its unreplaced bodies' callees are also audited.
    let mut active_maps: BTreeSet<TypeId> = if default_allocator {
        all_seen_maps.clone()
    } else {
        BTreeSet::new()
    };
    let mut decline_reasons: BTreeMap<TypeId, String> = BTreeMap::new();
    if !default_allocator {
        for &map_ty in &all_seen_maps {
            decline_reasons.insert(
                map_ty,
                "legality.custom-allocator non-default global allocator".to_owned(),
            );
        }
    }
    let (final_audit, final_map_items, validated_rewrites) = loop {
        let reachable = user_reachable_excluding_maps(
            table,
            funcs,
            live,
            &cx.opts.roots,
            &supported_map_items,
            &active_maps,
        );
        let audit =
            audit_reachable_items(table, funcs, &reachable, &supported_map_items, &active_maps);

        let mut map_items: BTreeMap<TypeId, Vec<JsName>> = BTreeMap::new();
        for name in &reachable {
            if let Some(&map_ty) = supported_map_items.get(name) {
                map_items.entry(map_ty).or_default().push(name.clone());
            }
        }

        let mut demoted = false;
        let mut next_rewrites: BTreeMap<TypeId, Vec<(JsName, NativeBody)>> = BTreeMap::new();
        let candidates: Vec<TypeId> = active_maps.iter().copied().collect();
        for map_ty in candidates {
            if audit.disqualified_maps.contains(&map_ty) {
                active_maps.remove(&map_ty);
                decline_reasons.insert(
                    map_ty,
                    "legality.order-or-escape BTreeMap observed by unsupported operation or escape"
                        .to_owned(),
                );
                demoted = true;
                continue;
            }
            let Some(items_for_map) = map_items.get(&map_ty) else {
                active_maps.remove(&map_ty);
                demoted = true;
                continue;
            };
            match validate_btree_map_items(table, map_ty, items_for_map, cx) {
                Ok(rewrites) => {
                    next_rewrites.insert(map_ty, rewrites);
                }
                Err(reason) => {
                    active_maps.remove(&map_ty);
                    decline_reasons.insert(map_ty, reason);
                    demoted = true;
                }
            }
        }

        if !demoted {
            break (audit, map_items, next_rewrites);
        }
    };

    // 3. Plan `Box<T, Global>` drop-glue elision.
    plan_box_drops(
        table,
        live,
        cx,
        default_allocator,
        &final_audit,
        &mut outcome,
    );

    // 4. Commit surviving `BTreeMap<K, V, Global>` -> JS `Map` plans and record decisions.
    commit_btree_maps(
        &all_seen_maps,
        &final_map_items,
        &supported_map_items,
        &active_maps,
        &decline_reasons,
        validated_rewrites,
        &mut outcome,
    );

    outcome.block.applied = outcome.block.committed;
    outcome
}

#[derive(Default)]
struct TypeAudit {
    /// `Box<T>` TypeIds observed through raw pointers, unsafe `Box` provenance, or host boundaries.
    disqualified_boxes: BTreeSet<TypeId>,
    /// `BTreeMap<K, V>` TypeIds touched by any unsupported op, raw view, cast, const, deref store,
    /// host boundary, export signature, or unlowered/declined item.
    disqualified_maps: BTreeSet<TypeId>,
}

fn is_supported_btree_map_op(op: StandardOperation) -> bool {
    matches!(
        op,
        StandardOperation::BTreeMapNew
            | StandardOperation::BTreeMapLen
            | StandardOperation::BTreeMapIsEmpty
            | StandardOperation::BTreeMapClear
            | StandardOperation::BTreeMapContainsKey
            | StandardOperation::BTreeMapGet
            | StandardOperation::BTreeMapGetMut
            | StandardOperation::BTreeMapInsert
            | StandardOperation::BTreeMapRemove
            | StandardOperation::BTreeMapDrop
    )
}

fn candidate_of(table: &TypeTable, ty: TypeId) -> Option<(RepresentationCandidate, TypeId)> {
    let peeled = table.peeled(ty);
    let desc = table.get(peeled)?;
    if desc.meta.candidate != RepresentationCandidate::None {
        Some((desc.meta.candidate, peeled))
    } else {
        None
    }
}

fn deref_candidate(table: &TypeTable, ty: TypeId) -> Option<(RepresentationCandidate, TypeId)> {
    let peeled = table.peeled(ty);
    let desc = table.get(peeled)?;
    match desc.kind {
        TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
            candidate_of(table, pointee)
        }
        _ => candidate_of(table, peeled),
    }
}

fn collect_contained_candidates(
    table: &TypeTable,
    ty: TypeId,
    target: RepresentationCandidate,
    out: &mut BTreeSet<TypeId>,
    visited: &mut BTreeSet<TypeId>,
) {
    let ty = table.peeled(ty);
    if !visited.insert(ty) {
        return;
    }
    let Some(desc) = table.get(ty) else {
        return;
    };
    if desc.meta.candidate == target {
        out.insert(ty);
    }
    match &desc.kind {
        TypeKind::Reference { pointee, .. }
        | TypeKind::RawPointer { pointee, .. }
        | TypeKind::Array {
            element: pointee, ..
        }
        | TypeKind::Slice(pointee) => {
            collect_contained_candidates(table, *pointee, target, out, visited);
        }
        TypeKind::Tuple(elems) => {
            for elem in elems {
                collect_contained_candidates(table, *elem, target, out, visited);
            }
        }
        TypeKind::Adt { variants, args, .. } => {
            for arg in args {
                collect_contained_candidates(table, *arg, target, out, visited);
            }
            for variant in variants {
                for field in &variant.fields {
                    collect_contained_candidates(table, field.ty, target, out, visited);
                }
            }
        }
        TypeKind::Closure { upvars, .. }
        | TypeKind::Coroutine { upvars, .. }
        | TypeKind::CoroutineClosure { upvars, .. } => {
            for upvar in upvars {
                collect_contained_candidates(table, *upvar, target, out, visited);
            }
        }
        _ => {}
    }
}

fn is_box_raw_provenance_item(item: &LinkItem, interns: &Interns) -> bool {
    let (crate_name, path) = semantic_ops::item_origin_info(item, interns);
    if crate_name != "alloc" && !path.starts_with("alloc::") {
        return false;
    }
    path.contains("boxed::Box")
        && (path.contains("into_raw")
            || path.contains("from_raw")
            || path.contains("into_non_null")
            || path.contains("from_non_null")
            || path.contains("leak"))
}

fn is_box_constructor_or_drop(item: &LinkItem, interns: &Interns) -> bool {
    if matches!(
        item.jir.body.as_ref().map(|b| b.semantic),
        Some(BodySemantic::DropGlue(_))
    ) {
        return true;
    }
    let (crate_name, path) = semantic_ops::item_origin_info(item, interns);
    (crate_name == "alloc" || path.starts_with("alloc::"))
        && (path.contains("boxed::Box::new")
            || path.contains("boxed::Box::try_new")
            || path.contains("boxed::Box::pin"))
}

fn btree_map_ty_of_semantic(
    table: &TypeTable,
    semantic: BodySemantic,
    sig: &rcgjs_object::jir::semantic::Signature,
) -> Option<TypeId> {
    let raw = match semantic {
        BodySemantic::DropGlue(dropped) => dropped,
        BodySemantic::Standard(StandardOperation::BTreeMapNew) => sig.result,
        BodySemantic::Standard(op) if is_supported_btree_map_op(op) => *sig.arguments.first()?,
        _ => return None,
    };
    let (cand, id) = deref_candidate(table, raw)?;
    (cand == RepresentationCandidate::BTreeMap).then_some(id)
}

/// BFS from program roots within `live`, treating every `supported_map_items` entry whose
/// `BTreeMap` `TypeId` is still in `active_maps` as an opaque terminal (since replacing its body
/// with a JS `Map` operation drops its internal `alloc::collections::btree::*` callees).
fn user_reachable_excluding_maps(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    live: &BTreeSet<JsName>,
    extra_roots: &[String],
    supported_map_items: &BTreeMap<JsName, TypeId>,
    active_maps: &BTreeSet<TypeId>,
) -> BTreeSet<JsName> {
    let mut queue: VecDeque<JsName> = live
        .iter()
        .filter(|name| {
            table.get(*name).is_some_and(|item| {
                item.linkage.root || extra_roots.iter().any(|root| root == name.as_str())
            })
        })
        .cloned()
        .collect();
    if queue.is_empty() {
        // Synthetic test tables without explicit root linkage: seed with non-replaced items.
        queue.extend(
            live.iter()
                .filter(|name| {
                    !supported_map_items
                        .get(*name)
                        .is_some_and(|ty| active_maps.contains(ty))
                })
                .cloned(),
        );
        if queue.is_empty() {
            queue.extend(live.iter().cloned());
        }
    }

    let mut seen: BTreeSet<JsName> = queue.iter().cloned().collect();
    while let Some(name) = queue.pop_front() {
        // Replaced `BTreeMap*` / `DropGlue(BTreeMap)` bodies do not execute their Rust internals.
        if supported_map_items
            .get(&name)
            .is_some_and(|ty| active_maps.contains(ty))
        {
            continue;
        }
        let Some(item) = table.get(&name) else {
            continue;
        };
        for referenced in &item.refs {
            if live.contains(referenced) && seen.insert(referenced.clone()) {
                queue.push_back(referenced.clone());
            }
        }
        // Also follow direct calls / drop glues spliced in by pre-B1 inlining.
        if let (Some(Ok(func)), Some(interns)) = (funcs.get(&name), item.jir.interns()) {
            for block in &func.blocks {
                for op in &block.ops {
                    let direct_name = match &op.kind {
                        OpKind::Call {
                            callee: Callee::Direct { link_name, .. },
                            ..
                        }
                        | OpKind::Drop {
                            glue: rcgjs_object::jir::semantic::DropGlue::Direct { link_name, .. },
                            ..
                        } => interns.str(*link_name),
                        _ => None,
                    };
                    if let Some(target_str) = direct_name {
                        let target = JsName::new(target_str.to_owned());
                        if live.contains(&target) && seen.insert(target.clone()) {
                            queue.push_back(target);
                        }
                    }
                }
            }
        }
    }
    seen
}

fn audit_reachable_items(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    reachable: &BTreeSet<JsName>,
    supported_map_items: &BTreeMap<JsName, TypeId>,
    active_maps: &BTreeSet<TypeId>,
) -> TypeAudit {
    let mut audit = TypeAudit::default();

    for name in reachable {
        let Some(item) = table.get(name) else {
            continue;
        };
        let Some(interns) = item.jir.interns() else {
            continue;
        };
        let types = &interns.types;

        // Exported / root functions cannot change their ABI for `BTreeMap` or escape `Box` to host.
        if item.linkage.exported || item.linkage.root {
            if let Some(body) = &item.jir.body {
                for &ty in body
                    .signature
                    .arguments
                    .iter()
                    .chain(std::iter::once(&body.signature.result))
                {
                    let mut visited = BTreeSet::new();
                    collect_contained_candidates(
                        types,
                        ty,
                        RepresentationCandidate::BTreeMap,
                        &mut audit.disqualified_maps,
                        &mut visited,
                    );
                }
            }
        }

        // `Box::into_raw` / `Box::from_raw` / `Box::leak` disqualifies that `Box<T>`.
        if is_box_raw_provenance_item(item, interns) {
            if let Some(body) = &item.jir.body {
                for &ty in body
                    .signature
                    .arguments
                    .iter()
                    .chain(std::iter::once(&body.signature.result))
                {
                    let mut visited = BTreeSet::new();
                    collect_contained_candidates(
                        types,
                        ty,
                        RepresentationCandidate::Box,
                        &mut audit.disqualified_boxes,
                        &mut visited,
                    );
                }
            }
        }

        // Active `BTreeMap*` and `DropGlue(BTreeMap)` bodies will be replaced wholesale.
        if supported_map_items
            .get(name)
            .is_some_and(|ty| active_maps.contains(ty))
        {
            continue;
        }

        // Unlowered/declined functions, statics, and vtables disqualify any `BTreeMap` they touch.
        let func = match funcs.get(name) {
            Some(Ok(func)) => func,
            _ => {
                if let Some(body) = &item.jir.body {
                    for &ty in body
                        .signature
                        .arguments
                        .iter()
                        .chain(std::iter::once(&body.signature.result))
                        .chain(body.locals.iter().map(|l| &l.ty))
                    {
                        let mut visited = BTreeSet::new();
                        collect_contained_candidates(
                            types,
                            ty,
                            RepresentationCandidate::BTreeMap,
                            &mut audit.disqualified_maps,
                            &mut visited,
                        );
                    }
                }
                continue;
            }
        };

        let box_internal = is_box_constructor_or_drop(item, interns);
        audit_func(func, types, interns, box_internal, &mut audit);
    }

    audit
}

fn audit_func(
    func: &Func,
    types: &TypeTable,
    interns: &Interns,
    box_internal: bool,
    audit: &mut TypeAudit,
) {
    let val_ty = |v: rcgjs_object::jir::repr::ValueId| func.values.get(v.0 as usize).map(|d| d.ty);
    let root_ty = |root: &Root| match root {
        Root::Cell(c) => func.cells.get(c.0 as usize).map(|d| d.ty),
        Root::Value(v) => val_ty(*v),
    };
    let mut def_map: BTreeMap<rcgjs_object::jir::repr::ValueId, &OpKind> = BTreeMap::new();
    for block in &func.blocks {
        for op in &block.ops {
            if let Some(res) = op.result {
                def_map.insert(res, &op.kind);
            }
        }
    }
    let disqualify_all_maps = |ty: TypeId, audit: &mut TypeAudit| {
        let mut visited = BTreeSet::new();
        collect_contained_candidates(
            types,
            ty,
            RepresentationCandidate::BTreeMap,
            &mut audit.disqualified_maps,
            &mut visited,
        );
    };
    let disqualify_all_boxes = |ty: TypeId, audit: &mut TypeAudit| {
        let mut visited = BTreeSet::new();
        collect_contained_candidates(
            types,
            ty,
            RepresentationCandidate::Box,
            &mut audit.disqualified_boxes,
            &mut visited,
        );
    };

    for block in &func.blocks {
        for op in &block.ops {
            let res_ty = op.result.and_then(val_ty);
            match &op.kind {
                OpKind::Const(tree) => {
                    if let Some(ty) = res_ty {
                        disqualify_all_maps(ty, audit);
                    }
                    // Static globals inside constants might also hold a BTreeMap.
                    if matches!(tree, ConstTree::Global { .. }) {
                        if let Some(ty) = res_ty {
                            disqualify_all_maps(ty, audit);
                        }
                    }
                }
                OpKind::Aggregate { ty, .. } => {
                    // Directly constructing a `BTreeMap` struct literal (not enclosing it).
                    if let Some((RepresentationCandidate::BTreeMap, map_ty)) =
                        candidate_of(types, *ty)
                    {
                        audit.disqualified_maps.insert(map_ty);
                    }
                }
                OpKind::Field { base, .. } | OpKind::SetField { base, .. } => {
                    if let Some(ty) = val_ty(*base) {
                        if let Some((RepresentationCandidate::BTreeMap, map_ty)) =
                            deref_candidate(types, ty)
                        {
                            audit.disqualified_maps.insert(map_ty);
                        }
                    }
                }
                OpKind::BorrowView { root, path, .. } | OpKind::Drop { root, path, .. } => {
                    if let Some(r_ty) = root_ty(root) {
                        check_path_field_into_map(types, r_ty, path, &mut audit.disqualified_maps);
                    }
                }
                OpKind::StoreThrough { ptr, v } => {
                    // Writing through `&mut BTreeMap` (`*m = other_map`) invokes `__rt.overwrite`
                    // which cannot copy JS `Map` entries. Storing into a struct field or array
                    // slot (`s.map = m`, `arr[i] = m`) is safe because it assigns the property/index.
                    if let Some(v_ty) = val_ty(*v) {
                        if let Some((RepresentationCandidate::BTreeMap, map_ty)) =
                            candidate_of(types, v_ty)
                        {
                            let is_member_store = matches!(
                                def_map.get(ptr),
                                Some(OpKind::BorrowView { path, .. })
                                    if matches!(
                                        path.last(),
                                        Some(Proj::Field { .. } | Proj::Index(_) | Proj::ConstantIndex { .. })
                                    )
                            );
                            if !is_member_store {
                                audit.disqualified_maps.insert(map_ty);
                            }
                        }
                    }
                }
                OpKind::RawView { root, path, .. } => {
                    if let Some(r_ty) = root_ty(root) {
                        disqualify_all_maps(r_ty, audit);
                        if !box_internal {
                            disqualify_all_boxes(r_ty, audit);
                        }
                    }
                    if let Some(ty) = res_ty {
                        disqualify_all_maps(ty, audit);
                        if !box_internal {
                            disqualify_all_boxes(ty, audit);
                        }
                    }
                    let _ = path;
                }
                OpKind::Cast { v, kind, .. } => {
                    if let Some(s_ty) = val_ty(*v) {
                        disqualify_all_maps(s_ty, audit);
                        if !box_internal
                            && !matches!(
                                kind,
                                rcgjs_object::jir::semantic::CastKind::PointerCoercion
                            )
                        {
                            if let Some((RepresentationCandidate::Box, box_ty)) =
                                deref_candidate(types, s_ty)
                            {
                                audit.disqualified_boxes.insert(box_ty);
                            }
                        }
                    }
                    if let Some(d_ty) = res_ty {
                        disqualify_all_maps(d_ty, audit);
                    }
                }
                OpKind::Intrinsic {
                    args, types: targs, ..
                } => {
                    for &arg in args {
                        if let Some(ty) = val_ty(arg) {
                            disqualify_all_maps(ty, audit);
                        }
                    }
                    for &ty in targs {
                        disqualify_all_maps(ty, audit);
                    }
                    if let Some(ty) = res_ty {
                        disqualify_all_maps(ty, audit);
                    }
                }
                OpKind::Host { args, .. } => {
                    for &arg in args {
                        if let Some(ty) = val_ty(arg) {
                            disqualify_all_maps(ty, audit);
                            disqualify_all_boxes(ty, audit);
                        }
                    }
                    if let Some(ty) = res_ty {
                        disqualify_all_maps(ty, audit);
                        disqualify_all_boxes(ty, audit);
                    }
                }
                OpKind::Call {
                    callee,
                    args,
                    semantic,
                    host,
                    ..
                } => {
                    if host.is_some() {
                        for &arg in args {
                            if let Some(ty) = val_ty(arg) {
                                disqualify_all_maps(ty, audit);
                                disqualify_all_boxes(ty, audit);
                            }
                        }
                        if let Some(ty) = res_ty {
                            disqualify_all_maps(ty, audit);
                            disqualify_all_boxes(ty, audit);
                        }
                        continue;
                    }
                    match callee {
                        Callee::Direct { link_name, .. } => {
                            let supported = matches!(
                                semantic,
                                CallSemantic::Standard(op) if is_supported_btree_map_op(*op)
                            );
                            if !supported {
                                // An ordinary function call is audited when we visit its own body;
                                // only unknown/runtime/alloc internal BTreeMap methods (which are
                                // also audited via their bodies) could touch internals. Still, if
                                // the link name is unresolvable, conservatively disqualify.
                                if interns.str(*link_name).is_none() {
                                    for &arg in args {
                                        if let Some(ty) = val_ty(arg) {
                                            disqualify_all_maps(ty, audit);
                                        }
                                    }
                                    if let Some(ty) = res_ty {
                                        disqualify_all_maps(ty, audit);
                                    }
                                }
                            }
                        }
                        Callee::Nop => {}
                        Callee::Indirect { .. }
                        | Callee::Virtual { .. }
                        | Callee::Intrinsic { .. }
                        | Callee::Runtime { .. }
                        | Callee::Foreign { .. } => {
                            for &arg in args {
                                if let Some(ty) = val_ty(arg) {
                                    disqualify_all_maps(ty, audit);
                                }
                            }
                            if let Some(ty) = res_ty {
                                disqualify_all_maps(ty, audit);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Checks if a projection path steps `Proj::Field` directly into a `BTreeMap` struct.
fn check_path_field_into_map(
    types: &TypeTable,
    mut ty: TypeId,
    path: &[Proj],
    disqualified: &mut BTreeSet<TypeId>,
) {
    for step in path {
        ty = types.peeled(ty);
        let Some(desc) = types.get(ty) else {
            return;
        };
        match step {
            Proj::Deref => match desc.kind {
                TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
                    ty = pointee;
                }
                TypeKind::Adt { ref args, .. }
                    if desc.meta.candidate == RepresentationCandidate::Box =>
                {
                    let Some(&pointee) = args.first() else {
                        return;
                    };
                    ty = pointee;
                }
                _ => return,
            },
            Proj::Field { ty: field_ty, .. } => {
                if desc.meta.candidate == RepresentationCandidate::BTreeMap {
                    disqualified.insert(ty);
                    return;
                }
                ty = *field_ty;
            }
            Proj::Index(_) | Proj::ConstantIndex { .. } | Proj::Subslice { .. } => {
                match desc.kind {
                    TypeKind::Array { element, .. } | TypeKind::Slice(element) => ty = element,
                    _ => return,
                }
            }
            Proj::Downcast { .. } => {}
        }
    }
}

fn decline_label(decline: SemanticDecline) -> String {
    let code = match decline {
        SemanticDecline::ElementHasOwnDrop => "legality.element-has-own-drop",
        SemanticDecline::ElementDropRequired => "legality.element-drop-required",
        SemanticDecline::ZeroSizedElement => "legality.zero-sized",
        SemanticDecline::UnknownElementLayout | SemanticDecline::UnknownLayout => {
            "legality.unknown-layout"
        }
        _ => "legality.unsupported-shape",
    };
    format!("{code} {}", decline.reason())
}

fn plan_box_drops(
    table: &BTreeMap<JsName, LinkItem>,
    live: &BTreeSet<JsName>,
    cx: &ReprCx<'_>,
    default_allocator: bool,
    audit: &TypeAudit,
    outcome: &mut ReprOutcome,
) {
    for name in live {
        let Some(item) = table.get(name) else {
            continue;
        };
        let (Some(body), Some(interns)) = (&item.jir.body, item.jir.interns()) else {
            continue;
        };
        let BodySemantic::DropGlue(dropped) = body.semantic else {
            continue;
        };
        let Some(desc) = interns.types.get(dropped) else {
            continue;
        };
        if desc.meta.candidate != RepresentationCandidate::Box {
            continue;
        }
        outcome.block.candidates += 1;
        let subject = format!("{}/box-drop", name.as_str());
        let alternatives = vec!["baseline".to_owned(), "native-box".to_owned()];
        let items = BTreeSet::from([name.clone()]);

        if !default_allocator {
            outcome.block.decisions.push(ReprDecision {
                subject,
                family: "box".to_owned(),
                chosen: "baseline".to_owned(),
                alternatives,
                decline: Some("legality.custom-allocator non-default global allocator".to_owned()),
                items,
            });
            continue;
        }
        if audit
            .disqualified_boxes
            .contains(&interns.types.peeled(dropped))
        {
            outcome.block.decisions.push(ReprDecision {
                subject,
                family: "box".to_owned(),
                chosen: "baseline".to_owned(),
                alternatives,
                decline: Some(
                    "legality.raw-provenance Box observed via raw pointer or host boundary"
                        .to_owned(),
                ),
                items,
            });
            continue;
        }

        let probe_cx = LowerCx {
            semantic: body.semantic,
            sig: &body.signature,
            table: &interns.types,
            interns,
            element_glue: None,
            native_string_enabled: cx.heap.native_string_enabled,
        };
        let pointee_drop = match semantic_ops::drop::required_box_pointee_drop(&probe_cx) {
            Ok(drop_ty) => drop_ty,
            Err(decline) => {
                outcome.block.decisions.push(ReprDecision {
                    subject,
                    family: "box".to_owned(),
                    chosen: "baseline".to_owned(),
                    alternatives,
                    decline: Some(decline_label(decline)),
                    items,
                });
                continue;
            }
        };
        let element_glue: Option<JsName> = match pointee_drop {
            Some(ty) => match cx.drop_glue.get(&ty) {
                Some(glue) => Some(JsName::new(glue.clone())),
                None => {
                    outcome.block.decisions.push(ReprDecision {
                        subject,
                        family: "box".to_owned(),
                        chosen: "baseline".to_owned(),
                        alternatives,
                        decline: Some(decline_label(SemanticDecline::ElementDropRequired)),
                        items,
                    });
                    continue;
                }
            },
            None => None,
        };

        let lcx = LowerCx {
            semantic: body.semantic,
            sig: &body.signature,
            table: &interns.types,
            interns,
            element_glue: element_glue.as_ref(),
            native_string_enabled: cx.heap.native_string_enabled,
        };
        let rewrite = match semantic_ops::rewrite(&lcx, &[Carrier::NativeBox]) {
            Ok(rewrite) => rewrite,
            Err(decline) => {
                outcome.block.decisions.push(ReprDecision {
                    subject,
                    family: "box".to_owned(),
                    chosen: "baseline".to_owned(),
                    alternatives,
                    decline: Some(decline_label(decline)),
                    items,
                });
                continue;
            }
        };

        outcome.block.legal += 1;
        outcome.block.planned += 1;
        outcome.block.cost_positive += 1;
        outcome.block.committed += 1;
        outcome.block.decisions.push(ReprDecision {
            subject,
            family: "box".to_owned(),
            chosen: "native-box".to_owned(),
            alternatives,
            decline: None,
            items,
        });
        outcome.touched.insert(name.clone());
        outcome.bodies.insert(
            name.clone(),
            NativeBody {
                parameters: rewrite.parameters,
                statements: rewrite.statements,
            },
        );
    }
}

fn validate_btree_map_items(
    table: &BTreeMap<JsName, LinkItem>,
    map_ty: TypeId,
    items_for_map: &[JsName],
    cx: &ReprCx<'_>,
) -> Result<Vec<(JsName, NativeBody)>, String> {
    let mut rewrites: Vec<(JsName, NativeBody)> = Vec::with_capacity(items_for_map.len());
    for name in items_for_map {
        let Some(item) = table.get(name) else {
            return Err("legality.missing-item missing BTreeMap item".to_owned());
        };
        let (Some(body), Some(interns)) = (&item.jir.body, item.jir.interns()) else {
            return Err("legality.missing-jir missing JIR body".to_owned());
        };
        // Double-check the map descriptor itself meets canonical Global/Rust/sized constraints.
        if let Some(desc) = interns.types.get(map_ty) {
            if desc.meta.abi != NominalAbi::Rust
                || desc.meta.allocator != AllocatorIdentity::Global
                || desc.meta.boundary != PinnedBoundary::None
                || !desc.layout.sized
                || desc.layout.zst
            {
                return Err(decline_label(SemanticDecline::WrongNativeType(
                    semantic_ops::SignatureSlot::Argument(0),
                )));
            }
        }
        let lcx = LowerCx {
            semantic: body.semantic,
            sig: &body.signature,
            table: &interns.types,
            interns,
            element_glue: None,
            native_string_enabled: cx.heap.native_string_enabled,
        };
        match semantic_ops::rewrite(&lcx, &[Carrier::NativeMap]) {
            Ok(rewrite) => {
                rewrites.push((
                    name.clone(),
                    NativeBody {
                        parameters: rewrite.parameters,
                        statements: rewrite.statements,
                    },
                ));
            }
            Err(decline) => return Err(decline_label(decline)),
        }
    }
    Ok(rewrites)
}

fn commit_btree_maps(
    all_seen_maps: &BTreeSet<TypeId>,
    final_map_items: &BTreeMap<TypeId, Vec<JsName>>,
    supported_map_items: &BTreeMap<JsName, TypeId>,
    active_maps: &BTreeSet<TypeId>,
    decline_reasons: &BTreeMap<TypeId, String>,
    mut validated_rewrites: BTreeMap<TypeId, Vec<(JsName, NativeBody)>>,
    outcome: &mut ReprOutcome,
) {
    for &map_ty in all_seen_maps {
        outcome.block.candidates += 1;
        let subject = format!("btreemap:{:032x}", map_ty.0);
        let alternatives = vec!["baseline".to_owned(), "native-map".to_owned()];
        let item_set: BTreeSet<JsName> = final_map_items
            .get(&map_ty)
            .map(|items| items.iter().cloned().collect())
            .unwrap_or_else(|| {
                supported_map_items
                    .iter()
                    .filter(|(_, ty)| **ty == map_ty)
                    .map(|(name, _)| name.clone())
                    .collect()
            });

        if active_maps.contains(&map_ty) {
            if let Some(rewrites) = validated_rewrites.remove(&map_ty) {
                outcome.block.legal += 1;
                outcome.block.planned += 1;
                outcome.block.cost_positive += 1;
                outcome.block.committed += 1;
                outcome.has_native_map = true;
                outcome.block.decisions.push(ReprDecision {
                    subject,
                    family: "map".to_owned(),
                    chosen: "native-map".to_owned(),
                    alternatives,
                    decline: None,
                    items: item_set,
                });
                for (name, body) in rewrites {
                    outcome.touched.insert(name.clone());
                    outcome.bodies.insert(name, body);
                }
                continue;
            }
        }

        let reason = decline_reasons
            .get(&map_ty)
            .cloned()
            .unwrap_or_else(|| "legality.unreachable-map no reachable BTreeMap ops".to_owned());
        outcome.block.decisions.push(ReprDecision {
            subject,
            family: "map".to_owned(),
            chosen: "baseline".to_owned(),
            alternatives,
            decline: Some(reason),
            items: item_set,
        });
    }
}
