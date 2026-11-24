//! Whole-program link-time enum representation specialization (`EnumRepr::NullNiche` and
//! `EnumRepr::IntTag`) for `Profile::ReleaseSpeed`.
//!
//! Runs BEFORE Phase A (`ssa::lower` / `build_funcs`) so SSA cell boxing, `SetDiscriminant`,
//! `needs_clone`, `fresh_container`, and `PointerForm::Thin -> Slot` all observe one consistent
//! `TypeTable` across the entire program — without requiring a sysroot rebuild or bumping
//! `JIR_SCHEMA`.
//!
//! # Soundness Guarantees
//! - Every enum type crossing a JS/host/FFI/export/import/DTS/C-ABI boundary, a v1-emitted
//!   `static`, a non-offered (v1 fallback) item, or an explicit retry exclusion keeps its
//!   baseline `EnumRepr::Tag` representation (per-`TypeId`, never string-name poisoning).
//! - `rcgjs.map.*` (`mires-js` `HashMap` / `HashSet`) markers are exempted from boundary
//!   exclusion because `jsc-link/src/lower/js/host.rs` emits them through `value::enum_value`
//!   and `mires-js::HashMap` stores indirect `MapEntry<V>` cells. If a legacy prebuilt crate
//!   ever stores an enum directly inside a host `Map` (without `MapEntry`), that inner stored
//!   enum is excluded so its `PointerForm::Thin` assumption stays intact.
//! - `EnumRepr::NullNiche` (`None => null`, `Some(x) => x`) is selected bottom-up ONLY when:
//!   1. The enum has 2 inhabited variants: one all-ZST-field `none_variant` and one single-field
//!      non-ZST `some_variant` with payload `T`.
//!   2. `may_inhabit_null(T)` is false across every transparent / `Pattern` wrapper layer
//!      (rejecting `PinnedBoundary != None`, host-boundary transparent wrappers, `Opaque`/
//!      `Foreign`/`Never`, and any inner `NullNiche` enum such as `Option<Option<T>>`).
//! - Any non-excluded internal enum whose variants all carry only ZST fields (e.g. `Option<()>`,
//!   `Result<(), fmt::Error>`) is promoted to `EnumRepr::IntTag { fieldless: true }` (`0 | 1`).
//! - All remaining non-excluded internal `EnumRepr::Tag` enums become `EnumRepr::IntTag`
//!   (`0, 1, 2, ...`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::semantic::{
    CallSemantic, Callee, ConstTree, DropGlue, GlobalReference, HostPayload, ItemId, Operand,
    OperandKind, Rvalue, StatementKind, Terminator, TypeId,
};
use rcgjs_object::jir::types::{
    EnumRepr, NominalAbi, NominalForm, PinnedBoundary, PointerForm, TypeDescriptor, TypeKind,
    TypeTable,
};
use rcgjs_object::object::ItemKind;

use crate::item::{JsName, LinkItem};
use crate::lower::js::value;
use crate::opts::{OptimizerMode, Opts, Profile};

/// Outcome of link-time enum representation specialization.
#[derive(Clone, Debug, Default)]
pub struct EnumReprPlan {
    pub decisions: BTreeMap<TypeId, EnumRepr>,
    pub null_niche_count: usize,
    pub int_tag_fieldless_count: usize,
    pub int_tag_payload_count: usize,
}

impl EnumReprPlan {
    pub fn is_empty(&self) -> bool {
        self.decisions.is_empty()
    }
}

/// Collects all `TypeId`s reachable from an item's JIR signature (used to exclude declined items'
/// types on retry so v1 fallback items never disagree with JIR items).
pub fn collect_item_signature_types(
    table: &BTreeMap<JsName, LinkItem>,
    names: &[JsName],
    out: &mut BTreeSet<TypeId>,
) {
    let global_types = build_global_type_map(table);
    let mut visited = BTreeSet::new();
    for name in names {
        let Some(item) = table.get(name) else {
            continue;
        };
        let Some(body) = item.jir.body.as_ref() else {
            continue;
        };
        let Some(interns) = item.jir.interns() else {
            continue;
        };
        for &input in &body.signature.arguments {
            walk_type_id(input, interns, &global_types, out, &mut visited);
        }
        walk_type_id(
            body.signature.result,
            interns,
            &global_types,
            out,
            &mut visited,
        );
    }
}

/// Specializes internal enums across all live items' `TypeTable`s under `ReleaseSpeed`.
pub fn specialize_table(
    table: &mut BTreeMap<JsName, LinkItem>,
    reachable: &[JsName],
    offered: &[JsName],
    extra_excluded: &BTreeSet<TypeId>,
    opts: &Opts,
) -> EnumReprPlan {
    if opts.optimizer != OptimizerMode::On || opts.profile != Some(Profile::ReleaseSpeed) {
        return EnumReprPlan::default();
    }

    let live_set: BTreeSet<&JsName> = reachable.iter().collect();
    let offered_set: BTreeSet<&JsName> = offered.iter().collect();
    let items_by_id: BTreeMap<ItemId, JsName> = table
        .values()
        .filter_map(|item| Some((item.jir.summary.as_ref()?.key.item?, item.name.clone())))
        .collect();
    let global_types = build_global_type_map(table);

    let mut excluded_enums: BTreeSet<TypeId> = extra_excluded.clone();
    let mut host_boundary_types: BTreeSet<TypeId> = BTreeSet::new();
    let mut visited_excl = BTreeSet::new();
    let mut visited_host = BTreeSet::new();

    // Seed any `extra_excluded` types transitively so nested enum fields stay aligned with v1.
    for &ty in extra_excluded {
        walk_type_id_global(ty, &global_types, &mut excluded_enums, &mut visited_excl);
    }

    let walk_both = |ty: TypeId,
                     interns: &Interns,
                     excluded_enums: &mut BTreeSet<TypeId>,
                     host_boundary_types: &mut BTreeSet<TypeId>,
                     visited_excl: &mut BTreeSet<TypeId>,
                     visited_host: &mut BTreeSet<TypeId>| {
        walk_type_id(ty, interns, &global_types, excluded_enums, visited_excl);
        walk_type_id(
            ty,
            interns,
            &global_types,
            host_boundary_types,
            visited_host,
        );
    };

    for item in table.values() {
        if !live_set.contains(&item.name) {
            continue;
        }
        let Some(interns) = item.jir.interns() else {
            continue;
        };
        let local = &interns.types;

        // 1. Pinned view-abi or C-ABI descriptors walk transitively; `PinnedBoundary::Host`
        // (`mires_js` crate types like `JsValue`, `HashMap<K, V>`, `MapEntry<V>`) pins only the
        // host descriptor itself so user types stored inside `mires_js::HashMap` are not
        // spuriously poisoned (actual host crossings are walked at `Call`/`Export`/`Import`).
        for desc in local.descriptors() {
            if desc.meta.boundary == PinnedBoundary::ViewAbi || desc.meta.abi == NominalAbi::C {
                walk_both(
                    desc.id,
                    interns,
                    &mut excluded_enums,
                    &mut host_boundary_types,
                    &mut visited_excl,
                    &mut visited_host,
                );
            } else if desc.meta.boundary == PinnedBoundary::Host {
                excluded_enums.insert(desc.id);
                host_boundary_types.insert(desc.id);
            }
        }

        let Some(body) = item.jir.body.as_ref() else {
            continue;
        };

        // 2. Exported / imported / DTS-bearing items cross the JS boundary.
        let is_js_boundary_item =
            item.linkage.exported || item.kind == ItemKind::Import || item.dts.is_some();
        if is_js_boundary_item {
            for &input in &body.signature.arguments {
                walk_both(
                    input,
                    interns,
                    &mut excluded_enums,
                    &mut host_boundary_types,
                    &mut visited_excl,
                    &mut visited_host,
                );
            }
            walk_both(
                body.signature.result,
                interns,
                &mut excluded_enums,
                &mut host_boundary_types,
                &mut visited_excl,
                &mut visited_host,
            );
        } else if !offered_set.contains(&item.name) {
            // Non-offered item runs v1 JS; keep its signature enums baseline.
            for &input in &body.signature.arguments {
                walk_type_id(
                    input,
                    interns,
                    &global_types,
                    &mut excluded_enums,
                    &mut visited_excl,
                );
            }
            walk_type_id(
                body.signature.result,
                interns,
                &global_types,
                &mut excluded_enums,
                &mut visited_excl,
            );
        }

        // 3. Scan calls, drops, and constant trees for boundary/static/non-offered edges.
        for block in &body.blocks {
            for stmt in &block.statements {
                match &stmt.kind {
                    StatementKind::Assign { value: rv, .. } => match rv {
                        Rvalue::Use(op)
                        | Rvalue::Repeat { value: op, .. }
                        | Rvalue::Cast { value: op, .. }
                        | Rvalue::Unary { value: op, .. }
                        | Rvalue::WrapUnsafeBinder(op) => scan_operand_consts(
                            op,
                            local,
                            interns,
                            table,
                            &offered_set,
                            &global_types,
                            &mut excluded_enums,
                            &mut visited_excl,
                        ),
                        Rvalue::Binary { left, right, .. } => {
                            scan_operand_consts(
                                left,
                                local,
                                interns,
                                table,
                                &offered_set,
                                &global_types,
                                &mut excluded_enums,
                                &mut visited_excl,
                            );
                            scan_operand_consts(
                                right,
                                local,
                                interns,
                                table,
                                &offered_set,
                                &global_types,
                                &mut excluded_enums,
                                &mut visited_excl,
                            );
                        }
                        Rvalue::Aggregate { operands, .. } => {
                            for op in operands {
                                scan_operand_consts(
                                    op,
                                    local,
                                    interns,
                                    table,
                                    &offered_set,
                                    &global_types,
                                    &mut excluded_enums,
                                    &mut visited_excl,
                                );
                            }
                        }
                        Rvalue::Reference { .. }
                        | Rvalue::RawPointer { .. }
                        | Rvalue::Discriminant(_)
                        | Rvalue::CopyForDeref(_)
                        | Rvalue::Reborrow { .. } => {}
                    },
                    StatementKind::Assume(op) => scan_operand_consts(
                        op,
                        local,
                        interns,
                        table,
                        &offered_set,
                        &global_types,
                        &mut excluded_enums,
                        &mut visited_excl,
                    ),
                    StatementKind::CopyNonOverlapping {
                        source,
                        destination,
                        count,
                    } => {
                        for op in [source, destination, count] {
                            scan_operand_consts(
                                op,
                                local,
                                interns,
                                table,
                                &offered_set,
                                &global_types,
                                &mut excluded_enums,
                                &mut visited_excl,
                            );
                        }
                    }
                    StatementKind::SetDiscriminant { .. }
                    | StatementKind::StorageLive(_)
                    | StatementKind::StorageDead(_)
                    | StatementKind::Nop => {}
                }
            }

            match &block.terminator {
                Terminator::Switch { discriminant, .. } => scan_operand_consts(
                    discriminant,
                    local,
                    interns,
                    table,
                    &offered_set,
                    &global_types,
                    &mut excluded_enums,
                    &mut visited_excl,
                ),
                Terminator::Assert { condition, .. } => scan_operand_consts(
                    condition,
                    local,
                    interns,
                    table,
                    &offered_set,
                    &global_types,
                    &mut excluded_enums,
                    &mut visited_excl,
                ),
                Terminator::Drop {
                    place,
                    glue:
                        DropGlue::Direct {
                            item: tid,
                            link_name,
                        },
                    ..
                } => {
                    let target_name = items_by_id
                        .get(tid)
                        .cloned()
                        .or_else(|| interns.str(*link_name).map(|s| JsName::new(s.to_owned())));
                    if !target_name
                        .as_ref()
                        .is_some_and(|n| offered_set.contains(n))
                    {
                        walk_type_id(
                            place.ty,
                            interns,
                            &global_types,
                            &mut excluded_enums,
                            &mut visited_excl,
                        );
                    }
                }
                Terminator::Call(call) => {
                    scan_operand_consts(
                        &call.function,
                        local,
                        interns,
                        table,
                        &offered_set,
                        &global_types,
                        &mut excluded_enums,
                        &mut visited_excl,
                    );
                    for arg in &call.arguments {
                        scan_operand_consts(
                            arg,
                            local,
                            interns,
                            table,
                            &offered_set,
                            &global_types,
                            &mut excluded_enums,
                            &mut visited_excl,
                        );
                    }

                    let map_marker = match &call.host {
                        Some(HostPayload::Map { marker }) => interns.str(*marker),
                        Some(HostPayload::Marker { section, .. }) => interns
                            .str(*section)
                            .and_then(|s| s.strip_prefix("rcgjs.map.")),
                        _ => None,
                    };
                    if let Some(marker) = map_marker {
                        // `rcgjs.map.*` markers lower via `value::enum_value` for their `Option`
                        // return type, so do NOT exclude the return `Option`. However, if a
                        // legacy crate stores an enum directly inside the JS `Map` (without
                        // `MapEntry<V>`), keep that stored enum indirect (`PointerForm::Thin`).
                        if matches!(marker, "try_get" | "try_getmut") {
                            if let Some(stored_ty) =
                                option_ref_inner_pointee(local, &global_types, call.destination.ty)
                            {
                                let peeled = peel_global(stored_ty, &global_types);
                                if global_types
                                    .get(&peeled)
                                    .is_some_and(|d| d.js.enum_repr.is_some())
                                {
                                    excluded_enums.insert(peeled);
                                }
                            }
                        } else if marker == "values" {
                            if let Some(stored_ty) =
                                slice_of_ref_pointee(local, &global_types, call.destination.ty)
                            {
                                let peeled = peel_global(stored_ty, &global_types);
                                if global_types
                                    .get(&peeled)
                                    .is_some_and(|d| d.js.enum_repr.is_some())
                                {
                                    excluded_enums.insert(peeled);
                                }
                            }
                        }
                        continue;
                    }

                    let is_host_boundary = call.host.is_some()
                        || matches!(
                            call.semantic,
                            CallSemantic::Host
                                | CallSemantic::ViewAbi
                                | CallSemantic::RawJavaScript
                        )
                        || matches!(call.callee, Callee::Foreign { .. } | Callee::Runtime { .. });

                    let is_non_offered_direct = match &call.callee {
                        Callee::Direct {
                            item: tid,
                            link_name,
                        } => {
                            let target_name = items_by_id.get(tid).cloned().or_else(|| {
                                interns.str(*link_name).map(|s| JsName::new(s.to_owned()))
                            });
                            !target_name
                                .as_ref()
                                .is_some_and(|n| offered_set.contains(n))
                        }
                        _ => false,
                    };

                    if is_host_boundary {
                        for arg in &call.arguments {
                            walk_both(
                                arg.ty,
                                interns,
                                &mut excluded_enums,
                                &mut host_boundary_types,
                                &mut visited_excl,
                                &mut visited_host,
                            );
                        }
                        walk_both(
                            call.destination.ty,
                            interns,
                            &mut excluded_enums,
                            &mut host_boundary_types,
                            &mut visited_excl,
                            &mut visited_host,
                        );
                        if let Some(host) = &call.host {
                            for (_, helper) in host.helpers() {
                                if let Some(helper_name) = items_by_id.get(&helper.item) {
                                    if let Some(helper_item) = table.get(helper_name) {
                                        if let (Some(h_body), Some(h_interns)) =
                                            (&helper_item.jir.body, helper_item.jir.interns())
                                        {
                                            for &inp in &h_body.signature.arguments {
                                                walk_both(
                                                    inp,
                                                    h_interns,
                                                    &mut excluded_enums,
                                                    &mut host_boundary_types,
                                                    &mut visited_excl,
                                                    &mut visited_host,
                                                );
                                            }
                                            walk_both(
                                                h_body.signature.result,
                                                h_interns,
                                                &mut excluded_enums,
                                                &mut host_boundary_types,
                                                &mut visited_excl,
                                                &mut visited_host,
                                            );
                                        }
                                    }
                                }
                            }
                            if let HostPayload::KeyedList(kl) = host {
                                for ty in [
                                    kl.collection_ty,
                                    kl.item_ty,
                                    kl.key_ty,
                                    kl.row_ty,
                                    kl.collection.ty,
                                    kl.key.ty,
                                    kl.row.ty,
                                ] {
                                    walk_both(
                                        ty,
                                        interns,
                                        &mut excluded_enums,
                                        &mut host_boundary_types,
                                        &mut visited_excl,
                                        &mut visited_host,
                                    );
                                }
                                for body_ref in [kl.collection.body, kl.key.body, kl.row.body]
                                    .into_iter()
                                    .flatten()
                                {
                                    if let Some(h_name) = items_by_id.get(&body_ref.item) {
                                        if let Some(h_item) = table.get(h_name) {
                                            if let (Some(h_body), Some(h_interns)) =
                                                (&h_item.jir.body, h_item.jir.interns())
                                            {
                                                for &inp in &h_body.signature.arguments {
                                                    walk_both(
                                                        inp,
                                                        h_interns,
                                                        &mut excluded_enums,
                                                        &mut host_boundary_types,
                                                        &mut visited_excl,
                                                        &mut visited_host,
                                                    );
                                                }
                                                walk_both(
                                                    h_body.signature.result,
                                                    h_interns,
                                                    &mut excluded_enums,
                                                    &mut host_boundary_types,
                                                    &mut visited_excl,
                                                    &mut visited_host,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if is_non_offered_direct {
                        for arg in &call.arguments {
                            walk_type_id(
                                arg.ty,
                                interns,
                                &global_types,
                                &mut excluded_enums,
                                &mut visited_excl,
                            );
                        }
                        walk_type_id(
                            call.destination.ty,
                            interns,
                            &global_types,
                            &mut excluded_enums,
                            &mut visited_excl,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    // 4. Compute canonical bottom-up decisions over all `TypeId`s in `global_types`.
    let mut plan = EnumReprPlan::default();

    // Collect eligible internal `EnumRepr::Tag` enums (excluding untagged single-inhabited enums,
    // C-ABI, pinned boundary, and any `excluded_enums`).
    let mut candidates: Vec<TypeId> = Vec::new();
    for (&id, desc) in &global_types {
        if excluded_enums.contains(&id)
            || desc.meta.abi == NominalAbi::C
            || desc.meta.boundary != PinnedBoundary::None
            || desc.js.transparent.is_some()
        {
            continue;
        }
        let Some(EnumRepr::Tag { .. }) = desc.js.enum_repr else {
            continue;
        };
        let TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            niche,
            ..
        } = &desc.kind
        else {
            continue;
        };
        let untagged = niche.is_none() && variants.iter().all(|v| v.physical_tag.is_none());
        if untagged || variants.len() < 2 {
            continue;
        }
        let inhabited_count = variants
            .iter()
            .filter(|v| {
                !v.fields
                    .iter()
                    .any(|f| is_uninhabited_global(f.ty, &global_types))
            })
            .count();
        if inhabited_count < 2 {
            continue;
        }
        candidates.push(id);
    }

    // Pass A: All-ZST-field enums (`Option<()>`, `Result<(), fmt::Error>`, etc.) -> `IntTag { fieldless: true }`.
    let mut null_niche_candidates: Vec<(u32, TypeId, u32, u32, TypeId)> = Vec::new();
    let mut remaining_tag_enums: Vec<(TypeId, bool)> = Vec::new();
    for id in candidates {
        let desc = &global_types[&id];
        let Some(EnumRepr::Tag { fieldless }) = desc.js.enum_repr else {
            continue;
        };
        let TypeKind::Adt { variants, .. } = &desc.kind else {
            continue;
        };
        if !fieldless
            && variants
                .iter()
                .all(|v| v.fields.iter().all(|f| is_zst_global(f.ty, &global_types)))
        {
            plan.decisions
                .insert(id, EnumRepr::IntTag { fieldless: true });
            plan.int_tag_fieldless_count += 1;
            continue;
        }
        if !fieldless && variants.len() == 2 {
            let v0_zst = variants[0]
                .fields
                .iter()
                .all(|f| is_zst_global(f.ty, &global_types));
            let v1_zst = variants[1]
                .fields
                .iter()
                .all(|f| is_zst_global(f.ty, &global_types));
            let pair = if v0_zst && variants[1].fields.len() == 1 {
                Some((0u32, 1u32, variants[1].fields[0].ty))
            } else if v1_zst && variants[0].fields.len() == 1 {
                Some((1u32, 0u32, variants[0].fields[0].ty))
            } else {
                None
            };
            if let Some((none_v, some_v, payload_ty)) = pair {
                if !is_zst_global(payload_ty, &global_types)
                    && !is_uninhabited_global(payload_ty, &global_types)
                {
                    let depth = transparent_enum_depth(payload_ty, &global_types);
                    null_niche_candidates.push((depth, id, none_v, some_v, payload_ty));
                    continue;
                }
            }
        }
        remaining_tag_enums.push((id, fieldless));
    }

    // Pass B: NullNiche in ascending nesting depth so inner `Option<T>` is decided before
    // `Option<Option<T>>` / `Option<ManuallyDrop<Option<T>>>`.
    null_niche_candidates.sort_by_key(|&(depth, id, _, _, _)| (depth, id));
    for (_, id, none_variant, some_variant, payload_ty) in null_niche_candidates {
        if !may_inhabit_null(
            payload_ty,
            &global_types,
            &host_boundary_types,
            &plan.decisions,
        ) {
            plan.decisions.insert(
                id,
                EnumRepr::NullNiche {
                    none_variant,
                    some_variant,
                },
            );
            plan.null_niche_count += 1;
        } else {
            remaining_tag_enums.push((id, false));
        }
    }

    // Pass C: All remaining internal `EnumRepr::Tag` enums -> `EnumRepr::IntTag`.
    for (id, fieldless) in remaining_tag_enums {
        plan.decisions.insert(id, EnumRepr::IntTag { fieldless });
        if fieldless {
            plan.int_tag_fieldless_count += 1;
        } else {
            plan.int_tag_payload_count += 1;
        }
    }

    if plan.decisions.is_empty() {
        return plan;
    }

    // 5. Apply `plan.decisions` to every live item's `TypeTable`, deduplicating mutated `Arc<Interns>`
    // by original pointer identity so items from the same crate share one table.
    let mut mutated_interns: BTreeMap<usize, Arc<Interns>> = BTreeMap::new();
    let item_names: Vec<JsName> = reachable.to_vec();
    for name in item_names {
        let Some(item) = table.get_mut(&name) else {
            continue;
        };
        let Some(orig_interns) = item.jir.interns() else {
            continue;
        };
        let key = orig_interns as *const Interns as usize;
        let shared = if let Some(cached) = mutated_interns.get(&key) {
            cached.clone()
        } else {
            let mut cloned = orig_interns.clone();
            apply_decisions_to_table(&mut cloned.types, &plan.decisions);
            let arc = Arc::new(cloned);
            mutated_interns.insert(key, arc.clone());
            arc
        };
        item.jir.use_shared_interns(shared);
    }

    plan
}

fn build_global_type_map(table: &BTreeMap<JsName, LinkItem>) -> BTreeMap<TypeId, TypeDescriptor> {
    let mut out = BTreeMap::new();
    let mut seen_interns: BTreeSet<usize> = BTreeSet::new();
    for item in table.values() {
        let Some(interns) = item.jir.interns() else {
            continue;
        };
        let key = interns as *const Interns as usize;
        if !seen_interns.insert(key) {
            continue;
        }
        for desc in interns.types.descriptors() {
            out.entry(desc.id).or_insert_with(|| desc.clone());
        }
    }
    out
}

fn lookup_desc<'a>(
    ty: TypeId,
    local: &'a TypeTable,
    global: &'a BTreeMap<TypeId, TypeDescriptor>,
) -> Option<&'a TypeDescriptor> {
    local.get(ty).or_else(|| global.get(&ty))
}

fn peel_global(mut ty: TypeId, global: &BTreeMap<TypeId, TypeDescriptor>) -> TypeId {
    for _ in 0..16 {
        let Some(desc) = global.get(&ty) else {
            break;
        };
        if desc.js.peeled != ty {
            ty = desc.js.peeled;
        } else {
            break;
        }
    }
    ty
}

fn is_zst_global(ty: TypeId, global: &BTreeMap<TypeId, TypeDescriptor>) -> bool {
    global.get(&ty).is_some_and(|d| d.layout.zst)
}

fn is_uninhabited_global(ty: TypeId, global: &BTreeMap<TypeId, TypeDescriptor>) -> bool {
    let peeled = peel_global(ty, global);
    match global.get(&peeled).map(|d| &d.kind) {
        Some(TypeKind::Never) => true,
        Some(TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            ..
        }) => variants.is_empty(),
        _ => false,
    }
}

/// Counts transparent + single-field-enum nesting depth so `Option<T>` is decided before
/// `Option<Option<T>>` / `Option<ManuallyDrop<Option<T>>>`.
fn transparent_enum_depth(mut ty: TypeId, global: &BTreeMap<TypeId, TypeDescriptor>) -> u32 {
    let mut depth = 0u32;
    for _ in 0..16 {
        let Some(desc) = global.get(&ty) else {
            break;
        };
        if let Some(inner) = desc.js.transparent {
            depth += 1;
            ty = inner.ty;
            continue;
        }
        if let TypeKind::Pattern(base) = desc.kind {
            depth += 1;
            ty = base;
            continue;
        }
        if let TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            ..
        } = &desc.kind
        {
            if variants.len() == 2 {
                let next = if variants[0].fields.len() == 1 {
                    Some(variants[0].fields[0].ty)
                } else if variants[1].fields.len() == 1 {
                    Some(variants[1].fields[0].ty)
                } else {
                    None
                };
                if let Some(inner) = next {
                    depth += 1;
                    ty = inner;
                    continue;
                }
            }
        }
        break;
    }
    depth
}

/// Whether JS `null` could ever be a valid runtime value of `ty`.
fn may_inhabit_null(
    mut ty: TypeId,
    global: &BTreeMap<TypeId, TypeDescriptor>,
    host_boundary_types: &BTreeSet<TypeId>,
    decisions: &BTreeMap<TypeId, EnumRepr>,
) -> bool {
    for _ in 0..16 {
        let Some(desc) = global.get(&ty) else {
            return true;
        };
        if desc.meta.boundary != PinnedBoundary::None {
            return true;
        }
        if desc.js.transparent.is_some() && host_boundary_types.contains(&ty) {
            return true;
        }
        if matches!(
            desc.kind,
            TypeKind::Opaque | TypeKind::Foreign(_) | TypeKind::Never
        ) {
            return true;
        }
        if matches!(decisions.get(&ty), Some(EnumRepr::NullNiche { .. }))
            || matches!(desc.js.enum_repr, Some(EnumRepr::NullNiche { .. }))
        {
            return true;
        }
        if let Some(inner) = desc.js.transparent {
            ty = inner.ty;
            continue;
        }
        if let TypeKind::Pattern(base) = desc.kind {
            ty = base;
            continue;
        }
        return false;
    }
    true
}

fn option_ref_inner_pointee(
    local: &TypeTable,
    global: &BTreeMap<TypeId, TypeDescriptor>,
    ty: TypeId,
) -> Option<TypeId> {
    let desc = lookup_desc(peel_global(ty, global), local, global)?;
    let TypeKind::Adt {
        form: NominalForm::Enum,
        variants,
        ..
    } = &desc.kind
    else {
        return None;
    };
    let some_field = variants.iter().find(|v| v.fields.len() == 1)?.fields[0].ty;
    let ref_desc = lookup_desc(peel_global(some_field, global), local, global)?;
    match ref_desc.kind {
        TypeKind::Reference { pointee, .. } => Some(pointee),
        _ => None,
    }
}

fn slice_of_ref_pointee(
    local: &TypeTable,
    global: &BTreeMap<TypeId, TypeDescriptor>,
    ty: TypeId,
) -> Option<TypeId> {
    let ref_desc = lookup_desc(peel_global(ty, global), local, global)?;
    let TypeKind::Reference { pointee, .. } = ref_desc.kind else {
        return None;
    };
    let slice_desc = lookup_desc(peel_global(pointee, global), local, global)?;
    let TypeKind::Slice(elem) = slice_desc.kind else {
        return None;
    };
    let elem_desc = lookup_desc(peel_global(elem, global), local, global)?;
    match elem_desc.kind {
        TypeKind::Reference { pointee, .. } => Some(pointee),
        _ => None,
    }
}

fn walk_type_id(
    ty: TypeId,
    interns: &Interns,
    global: &BTreeMap<TypeId, TypeDescriptor>,
    out: &mut BTreeSet<TypeId>,
    visited: &mut BTreeSet<TypeId>,
) {
    if !visited.insert(ty) {
        return;
    }
    out.insert(ty);
    let Some(desc) = lookup_desc(ty, &interns.types, global) else {
        return;
    };
    if desc.js.peeled != ty {
        walk_type_id(desc.js.peeled, interns, global, out, visited);
    }
    if let Some(inner) = desc.js.transparent {
        walk_type_id(inner.ty, interns, global, out, visited);
    }
    if let Some(tail) = desc.js.unsized_tail {
        walk_type_id(tail.ty, interns, global, out, visited);
    }
    for field in &desc.js.fields {
        walk_type_id(field.ty, interns, global, out, visited);
    }
    match &desc.kind {
        TypeKind::Reference { pointee, .. }
        | TypeKind::RawPointer { pointee, .. }
        | TypeKind::Slice(pointee)
        | TypeKind::Array {
            element: pointee, ..
        }
        | TypeKind::Pattern(pointee) => {
            walk_type_id(*pointee, interns, global, out, visited);
        }
        TypeKind::Tuple(elems) => {
            for &e in elems {
                walk_type_id(e, interns, global, out, visited);
            }
        }
        TypeKind::Adt { args, variants, .. } => {
            for &a in args {
                walk_type_id(a, interns, global, out, visited);
            }
            for v in variants {
                for f in &v.fields {
                    walk_type_id(f.ty, interns, global, out, visited);
                }
            }
        }
        TypeKind::Closure { upvars, .. } | TypeKind::CoroutineClosure { upvars, .. } => {
            for &u in upvars {
                walk_type_id(u, interns, global, out, visited);
            }
        }
        TypeKind::Coroutine {
            upvars,
            saved_locals,
            variants,
            ..
        } => {
            for &u in upvars {
                walk_type_id(u, interns, global, out, visited);
            }
            for &(_, t) in saved_locals {
                walk_type_id(t, interns, global, out, visited);
            }
            for v in variants {
                for f in &v.fields {
                    walk_type_id(f.ty, interns, global, out, visited);
                }
            }
        }
        TypeKind::FnDef { sig, .. } | TypeKind::FnPtr { sig } => {
            if let Some(sig_desc) = interns.sig(*sig) {
                for &i in &sig_desc.inputs {
                    walk_type_id(i, interns, global, out, visited);
                }
                walk_type_id(sig_desc.output, interns, global, out, visited);
            }
        }
        _ => {}
    }
}

fn walk_type_id_global(
    ty: TypeId,
    global: &BTreeMap<TypeId, TypeDescriptor>,
    out: &mut BTreeSet<TypeId>,
    visited: &mut BTreeSet<TypeId>,
) {
    let empty = Interns::default();
    walk_type_id(ty, &empty, global, out, visited);
}

#[allow(clippy::too_many_arguments)]
fn scan_operand_consts(
    op: &Operand,
    local: &TypeTable,
    interns: &Interns,
    table: &BTreeMap<JsName, LinkItem>,
    offered_set: &BTreeSet<&JsName>,
    global: &BTreeMap<TypeId, TypeDescriptor>,
    excluded: &mut BTreeSet<TypeId>,
    visited: &mut BTreeSet<TypeId>,
) {
    if let OperandKind::Constant(constant) = &op.kind {
        match constant {
            rcgjs_object::jir::semantic::Constant::Pointer {
                target: GlobalReference::Static { .. },
                ..
            } => {
                walk_type_id(op.ty, interns, global, excluded, visited);
            }
            rcgjs_object::jir::semantic::Constant::Pointer {
                target: GlobalReference::Function { link_name, .. },
                ..
            } => {
                let offered = interns
                    .str(*link_name)
                    .is_some_and(|s| offered_set.contains(&JsName::new(s.to_owned())));
                if !offered {
                    walk_type_id(op.ty, interns, global, excluded, visited);
                }
            }
            rcgjs_object::jir::semantic::Constant::Memory(tree) => {
                scan_const_tree(
                    tree,
                    op.ty,
                    local,
                    interns,
                    table,
                    offered_set,
                    global,
                    excluded,
                    visited,
                    0,
                );
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_const_tree(
    tree: &ConstTree,
    ty: TypeId,
    local: &TypeTable,
    interns: &Interns,
    table: &BTreeMap<JsName, LinkItem>,
    offered_set: &BTreeSet<&JsName>,
    global: &BTreeMap<TypeId, TypeDescriptor>,
    excluded: &mut BTreeSet<TypeId>,
    visited: &mut BTreeSet<TypeId>,
    depth: u32,
) {
    if depth > 32 {
        return;
    }
    match tree {
        ConstTree::Global {
            target: GlobalReference::Static { .. },
            ..
        }
        | ConstTree::StaticField { .. }
        | ConstTree::StaticSlice { .. } => {
            walk_type_id(ty, interns, global, excluded, visited);
        }
        ConstTree::Global {
            target: GlobalReference::Function { link_name, .. },
            ..
        } => {
            let offered = interns
                .str(*link_name)
                .is_some_and(|s| offered_set.contains(&JsName::new(s.to_owned())));
            if !offered {
                walk_type_id(ty, interns, global, excluded, visited);
            }
        }
        ConstTree::Reference(inner) => {
            let peeled = peel_global(ty, global);
            if let Some(desc) = lookup_desc(peeled, local, global) {
                if let TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } =
                    desc.kind
                {
                    scan_const_tree(
                        inner,
                        pointee,
                        local,
                        interns,
                        table,
                        offered_set,
                        global,
                        excluded,
                        visited,
                        depth + 1,
                    );
                }
            }
        }
        ConstTree::Slice(elems) => {
            let peeled = peel_global(ty, global);
            if let Some(desc) = lookup_desc(peeled, local, global) {
                let elem_ty = match desc.kind {
                    TypeKind::Slice(e) | TypeKind::Array { element: e, .. } => Some(e),
                    TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
                        match lookup_desc(peel_global(pointee, global), local, global)
                            .map(|d| &d.kind)
                        {
                            Some(TypeKind::Slice(e) | TypeKind::Array { element: e, .. }) => {
                                Some(*e)
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some(e) = elem_ty {
                    for elem in elems {
                        scan_const_tree(
                            elem,
                            e,
                            local,
                            interns,
                            table,
                            offered_set,
                            global,
                            excluded,
                            visited,
                            depth + 1,
                        );
                    }
                }
            }
        }
        ConstTree::Aggregate(fields) => {
            let peeled = peel_global(ty, global);
            if let Some(desc) = lookup_desc(peeled, local, global) {
                match &desc.kind {
                    TypeKind::Array { element, .. } => {
                        for f in fields {
                            scan_const_tree(
                                f,
                                *element,
                                local,
                                interns,
                                table,
                                offered_set,
                                global,
                                excluded,
                                visited,
                                depth + 1,
                            );
                        }
                    }
                    TypeKind::Tuple(elems) => {
                        for (f, &e_ty) in fields.iter().zip(elems) {
                            scan_const_tree(
                                f,
                                e_ty,
                                local,
                                interns,
                                table,
                                offered_set,
                                global,
                                excluded,
                                visited,
                                depth + 1,
                            );
                        }
                    }
                    TypeKind::Adt { variants, .. } => {
                        if let Some(v) = variants.first() {
                            for (f, fd) in fields.iter().zip(&v.fields) {
                                scan_const_tree(
                                    f,
                                    fd.ty,
                                    local,
                                    interns,
                                    table,
                                    offered_set,
                                    global,
                                    excluded,
                                    visited,
                                    depth + 1,
                                );
                            }
                        }
                    }
                    TypeKind::Closure { upvars, .. }
                    | TypeKind::CoroutineClosure { upvars, .. } => {
                        for (f, &u_ty) in fields.iter().zip(upvars) {
                            scan_const_tree(
                                f,
                                u_ty,
                                local,
                                interns,
                                table,
                                offered_set,
                                global,
                                excluded,
                                visited,
                                depth + 1,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        ConstTree::Variant { variant, fields } => {
            let peeled = peel_global(ty, global);
            if let Some(desc) = lookup_desc(peeled, local, global) {
                if let TypeKind::Adt { variants, .. } = &desc.kind {
                    if let Some(v) = variants.get(*variant as usize) {
                        for (f, fd) in fields.iter().zip(&v.fields) {
                            scan_const_tree(
                                f,
                                fd.ty,
                                local,
                                interns,
                                table,
                                offered_set,
                                global,
                                excluded,
                                visited,
                                depth + 1,
                            );
                        }
                    }
                }
            }
        }
        ConstTree::Dyn {
            data,
            concrete,
            vtable,
        } => {
            scan_const_tree(
                data,
                *concrete,
                local,
                interns,
                table,
                offered_set,
                global,
                excluded,
                visited,
                depth + 1,
            );
            if let Some(vt_name) = interns.str(*vtable) {
                if let Some(vt_item) = table.get(&JsName::new(vt_name.to_owned())) {
                    for r in &vt_item.refs {
                        if let Some(ref_item) = table.get(r) {
                            if ref_item.kind == ItemKind::Fn && !offered_set.contains(r) {
                                walk_type_id(*concrete, interns, global, excluded, visited);
                                if let (Some(rb), Some(ri)) =
                                    (&ref_item.jir.body, ref_item.jir.interns())
                                {
                                    for &inp in &rb.signature.arguments {
                                        walk_type_id(inp, ri, global, excluded, visited);
                                    }
                                    walk_type_id(
                                        rb.signature.result,
                                        ri,
                                        global,
                                        excluded,
                                        visited,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        ConstTree::Unit
        | ConstTree::Scalar { .. }
        | ConstTree::Str(_)
        | ConstTree::Address(_)
        | ConstTree::SliceAddress { .. }
        | ConstTree::Global {
            target: GlobalReference::Type(_),
            ..
        } => {}
    }
}

/// Applies `decisions` to one `TypeTable`, cascading to transparent/pattern wrappers and
/// references whose pointee transitioned from `is_indirect = true` to `false`.
fn apply_decisions_to_table(table: &mut TypeTable, decisions: &BTreeMap<TypeId, EnumRepr>) {
    // Pass 1a: `IntTag { fieldless: true }` first so `needs_clone` is false when `NullNiche`
    // queries its payload (e.g. `Option<Option<()>>` or `Result<Option<()>, ()>`).
    for (&id, &repr) in decisions {
        if let EnumRepr::IntTag { fieldless: true } = repr {
            if let Some(desc) = table.get_mut(id) {
                desc.js.enum_repr = Some(repr);
                desc.js.is_indirect = false;
                desc.js.needs_clone = false;
                desc.js.is_primitive_enum = true;
            }
        }
    }

    // Pass 1b: `NullNiche` and `IntTag { fieldless: false }`.
    let null_niche_updates: Vec<(TypeId, EnumRepr, bool)> = decisions
        .iter()
        .filter_map(|(&id, &repr)| {
            let EnumRepr::NullNiche { some_variant, .. } = repr else {
                return None;
            };
            let desc = table.get(id)?;
            let TypeKind::Adt { variants, .. } = &desc.kind else {
                return None;
            };
            let payload_ty = variants.get(some_variant as usize)?.fields.first()?.ty;
            let payload_needs_clone = table.needs_clone(payload_ty);
            Some((id, repr, payload_needs_clone))
        })
        .collect();
    for (id, repr, payload_needs_clone) in null_niche_updates {
        if let Some(desc) = table.get_mut(id) {
            desc.js.enum_repr = Some(repr);
            desc.js.is_indirect = false;
            desc.js.needs_clone = payload_needs_clone;
            desc.js.is_primitive_enum = false;
        }
    }
    for (&id, &repr) in decisions {
        if let EnumRepr::IntTag { fieldless: false } = repr {
            if let Some(desc) = table.get_mut(id) {
                desc.js.enum_repr = Some(repr);
            }
        }
    }

    // Pass 2: Cascade enum facts to transparent and `Pattern` wrappers whose `peeled` changed.
    let wrapper_updates: Vec<(TypeId, Option<EnumRepr>, bool, bool, bool)> = table
        .descriptors()
        .iter()
        .filter_map(|desc| {
            let peeled = desc.js.peeled;
            if peeled == desc.id || !decisions.contains_key(&peeled) {
                return None;
            }
            let target = table.get(peeled)?;
            Some((
                desc.id,
                target.js.enum_repr,
                target.js.is_indirect,
                target.js.needs_clone,
                target.js.is_primitive_enum,
            ))
        })
        .collect();
    for (id, enum_repr, is_indirect, needs_clone, is_primitive_enum) in wrapper_updates {
        if let Some(desc) = table.get_mut(id) {
            desc.js.enum_repr = enum_repr;
            desc.js.is_indirect = is_indirect;
            desc.js.needs_clone = needs_clone;
            desc.js.is_primitive_enum = is_primitive_enum;
        }
    }

    // Pass 3: Cascade `PointerForm::Thin -> PointerForm::Slot` on references whose pointee became
    // direct (`!is_indirect`), then propagate updated `pointer_form` to transparent wrappers like
    // `Pin<&mut Option<T>>`.
    let ref_updates: Vec<TypeId> = table
        .descriptors()
        .iter()
        .filter_map(|desc| {
            if desc.js.pointer_form != Some(PointerForm::Thin) {
                return None;
            }
            let TypeKind::Reference { pointee, .. } = desc.kind else {
                return None;
            };
            if table.is_indirect(pointee) || !value::is_slot_pointee(table, pointee) {
                return None;
            }
            if value::is_str(table, pointee)
                || table.unsized_tail(pointee).is_some()
                || matches!(
                    value::kind(table, pointee),
                    Some(TypeKind::Slice(_) | TypeKind::Dynamic { .. })
                )
            {
                return None;
            }
            Some(desc.id)
        })
        .collect();
    for id in ref_updates {
        if let Some(desc) = table.get_mut(id) {
            desc.js.pointer_form = Some(PointerForm::Slot);
        }
    }

    let ptr_wrapper_updates: Vec<(TypeId, Option<PointerForm>)> = table
        .descriptors()
        .iter()
        .filter_map(|desc| {
            let peeled = desc.js.peeled;
            if peeled == desc.id {
                return None;
            }
            let target = table.get(peeled)?;
            if target.js.pointer_form != desc.js.pointer_form {
                Some((desc.id, target.js.pointer_form))
            } else {
                None
            }
        })
        .collect();
    for (id, pointer_form) in ptr_wrapper_updates {
        if let Some(desc) = table.get_mut(id) {
            desc.js.pointer_form = pointer_form;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{ItemMap, LinkJir, Linkage};
    use rcgjs_object::jir::interns::StrId;
    use rcgjs_object::jir::semantic::{
        AbiSignature, BasicBlock, BlockId, Body, BodyKind, BodySemantic, DefinitionId, Signature,
        Terminator,
    };
    use rcgjs_object::jir::types::{
        FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts, NominalDiscriminant,
        TypeMeta, VariantDescriptor,
    };

    fn base_facts(id: TypeId) -> JsFacts {
        JsFacts {
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
        }
    }

    fn unit_desc(id: TypeId) -> TypeDescriptor {
        TypeDescriptor {
            id,
            name: None,
            kind: TypeKind::Tuple(Vec::new()),
            layout: LayoutFacts {
                size: Some(0),
                align: Some(1),
                sized: true,
                zst: true,
                one_zst: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(id),
        }
    }

    fn u32_desc(id: TypeId) -> TypeDescriptor {
        let mut js = base_facts(id);
        js.int = Some(IntFacts {
            signed: false,
            bits: 32,
            repr: IntRepr::Number,
        });
        TypeDescriptor {
            id,
            name: None,
            kind: TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            layout: LayoutFacts {
                size: Some(4),
                align: Some(4),
                sized: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js,
        }
    }

    fn option_desc(
        id: TypeId,
        payload_ty: TypeId,
        none_name: StrId,
        some_name: StrId,
        f0_name: StrId,
    ) -> TypeDescriptor {
        let mut js = base_facts(id);
        js.is_indirect = true;
        js.needs_clone = true;
        js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
        TypeDescriptor {
            id,
            name: None,
            kind: TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 100,
                },
                args: vec![payload_ty],
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: none_name,
                        fields: Vec::new(),
                        logical_discr: 0,
                        physical_tag: Some(NominalDiscriminant { bits: 0, bytes: 1 }),
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: some_name,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(f0_name),
                            ty: payload_ty,
                            offset: 4,
                        }],
                        logical_discr: 1,
                        physical_tag: Some(NominalDiscriminant { bits: 1, bytes: 1 }),
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
                niche: None,
            },
            layout: LayoutFacts {
                size: Some(8),
                align: Some(4),
                sized: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js,
        }
    }

    fn ref_desc(id: TypeId, pointee: TypeId, form: PointerForm) -> TypeDescriptor {
        let mut js = base_facts(id);
        js.pointer_form = Some(form);
        TypeDescriptor {
            id,
            name: None,
            kind: TypeKind::Reference {
                pointee,
                mutable: false,
            },
            layout: LayoutFacts {
                size: Some(4),
                align: Some(4),
                sized: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js,
        }
    }

    fn dummy_item(name: &str, interns: Arc<Interns>, arg_ty: TypeId, exported: bool) -> LinkItem {
        let js_name = JsName::new(name.to_owned());
        let body = Body {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash: if exported { 2 } else { 1 },
                },
                instantiation: 0,
            },
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: vec![arg_ty],
                result: TypeId(1),
                abi: AbiSignature::default(),
            },
            locals: Vec::new(),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Return,
            }],
        };
        let mut jir = LinkJir::default();
        jir.body = Some(body);
        jir.use_shared_interns(interns);
        LinkItem {
            name: js_name,
            kind: ItemKind::Fn,
            js: String::new(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: if exported {
                Linkage {
                    exported: true,
                    ..Linkage::internal()
                }
            } else {
                Linkage::internal()
            },
            debug_path: name.to_owned(),
            crate_name: "test".to_owned(),
            order: None,
            map: ItemMap::default(),
            dts: None,
            optimization: Default::default(),
            late_ir: Default::default(),
            baseline_shadow: Default::default(),
            jir,
        }
    }

    #[test]
    fn specializes_null_niche_int_tag_nested_option_and_protects_boundary() {
        let mut interns = Interns::default();
        let s_none = interns.intern_str("None");
        let s_some = interns.intern_str("Some");
        let s_f0 = interns.intern_str("_0");

        let t_unit = TypeId(1);
        let t_u32 = TypeId(2);
        let t_opt_u32 = TypeId(3);
        let t_opt_opt_u32 = TypeId(4);
        let t_opt_unit = TypeId(5);
        let t_ref_opt_u32 = TypeId(6);
        let t_exported_opt = TypeId(7);

        let mut exported_opt = option_desc(t_exported_opt, t_u32, s_none, s_some, s_f0);
        exported_opt.id = t_exported_opt;
        exported_opt.js.peeled = t_exported_opt;

        for desc in [
            unit_desc(t_unit),
            u32_desc(t_u32),
            option_desc(t_opt_u32, t_u32, s_none, s_some, s_f0),
            option_desc(t_opt_opt_u32, t_opt_u32, s_none, s_some, s_f0),
            option_desc(t_opt_unit, t_unit, s_none, s_some, s_f0),
            ref_desc(t_ref_opt_u32, t_opt_u32, PointerForm::Thin),
            exported_opt,
        ] {
            interns.types.insert(desc);
        }
        let shared = Arc::new(interns);

        let f_int = JsName::new("internal_fn".to_owned());
        let f_exp = JsName::new("exported_fn".to_owned());
        let mut table = BTreeMap::new();
        table.insert(
            f_int.clone(),
            dummy_item("internal_fn", shared.clone(), t_opt_opt_u32, false),
        );
        table.insert(
            f_exp.clone(),
            dummy_item("exported_fn", shared, t_exported_opt, true),
        );
        let reachable = vec![f_int.clone(), f_exp.clone()];

        // Non-ReleaseSpeed makes zero decisions.
        let bal_opts = Opts {
            optimizer: OptimizerMode::On,
            profile: Some(Profile::ReleaseBalanced),
            ..Opts::default()
        };
        let bal_plan = specialize_table(
            &mut table.clone(),
            &reachable,
            &reachable,
            &BTreeSet::new(),
            &bal_opts,
        );
        assert!(bal_plan.is_empty());

        // ReleaseSpeed specializes bottom-up and protects exported boundary enum.
        let spd_opts = Opts {
            optimizer: OptimizerMode::On,
            profile: Some(Profile::ReleaseSpeed),
            ..Opts::default()
        };
        let plan = specialize_table(
            &mut table,
            &reachable,
            &reachable,
            &BTreeSet::new(),
            &spd_opts,
        );
        assert_eq!(
            plan.decisions.get(&t_opt_u32),
            Some(&EnumRepr::NullNiche {
                none_variant: 0,
                some_variant: 1
            })
        );
        assert_eq!(
            plan.decisions.get(&t_opt_opt_u32),
            Some(&EnumRepr::IntTag { fieldless: false })
        );
        assert_eq!(
            plan.decisions.get(&t_opt_unit),
            Some(&EnumRepr::IntTag { fieldless: true })
        );
        assert!(
            !plan.decisions.contains_key(&t_exported_opt),
            "exported enum must keep baseline EnumRepr::Tag"
        );

        let updated = &table[&f_int].jir.interns().unwrap().types;
        assert!(!updated.is_indirect(t_opt_u32));
        assert!(!updated.is_indirect(t_opt_unit));
        assert!(updated.is_primitive_enum(t_opt_unit));
        assert_eq!(
            updated.get(t_ref_opt_u32).unwrap().js.pointer_form,
            Some(PointerForm::Slot),
            "&Option<u32> must cascade Thin -> Slot when Option<u32> becomes NullNiche"
        );
        assert_eq!(
            updated.enum_repr(t_exported_opt),
            Some(EnumRepr::Tag { fieldless: false })
        );
    }
}
