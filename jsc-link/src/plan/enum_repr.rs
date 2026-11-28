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
    BlockId, Body, BodySemantic, Call, CallSemantic, Callee, ConstTree, Constant, DropGlue,
    GlobalReference, HostPayload, ItemId, LocalId, Operand, OperandKind, Projection, Rvalue,
    Statement, StatementKind, Terminator, TypeId, Unwind,
};
use rcgjs_object::jir::types::{
    EnumRepr, FieldDescriptor, NominalAbi, NominalForm, PinnedBoundary, PointerForm,
    TypeDescriptor, TypeKind, TypeTable,
};
use rcgjs_object::object::ItemKind;

use crate::item::{JsName, LinkItem};
use crate::lower::js::value;
use crate::opts::{OptimizerMode, OptimizerPass, Opts, Profile};

use super::names::{synthesized_static_item, NamePool};
use super::smi_fields::SmiScanState;

/// Whether `callee` is a pure representation-opaque sysroot runtime helper (memory/pointer/alloc/
/// string/byte/NativeVec primitive) that never inspects `.TAG` or struct field names and never
/// exposes Rust values across the JS host boundary.
///
/// Deliberately excluded:
/// - `host_*`, `host_try`, `host_spawn`, `host_future_*`: host/JS boundary or `.TAG` inspection.
/// - `bstr_boundary`, `bstr_ref`, `bstr_ref_in`, `bstr_str_in`, `bstr_utf8_bytes`: JS boundary
///   membranes (`bstr_ref` reads `t.TAG`).
/// - `map_*`: handled separately via `rcgjs.map.*` marker logic.
pub(crate) fn is_representation_opaque_runtime(callee: &Callee, interns: &Interns) -> bool {
    let Callee::Runtime {
        member,
        sysroot: true,
    } = callee
    else {
        return false;
    };
    matches!(
        interns.str(*member).unwrap_or(""),
        "alloc"
            | "dealloc"
            | "realloc"
            | "alloc_zeroed"
            | "alloc_error"
            | "panic"
            | "abort"
            | "abort_error"
            | "js_abort"
            | "unreachable"
            | "memcpy"
            | "memmove"
            | "memset"
            | "memcmp"
            | "bcmp"
            | "strlen"
            | "copy"
            | "copy_nonoverlapping"
            | "write_bytes"
            | "compare_bytes"
            | "overwrite"
            | "_put"
            | "_put_plain"
            | "_put_tracked"
            | "box"
            | "prefix"
            | "unproject"
            | "erase"
            | "unref"
            | "unscale"
            | "unscale_scalar"
            | "unscale_uninit"
            | "scale"
            | "scale_scalar"
            | "byte_sign"
            | "read_byte"
            | "write_byte"
            | "read_byte_at"
            | "write_byte_at"
            | "retype"
            | "retype_rc"
            | "ptr_is_null"
            | "offset"
            | "offset_from"
            | "ptr_eq"
            | "slice_ptr_eq"
            | "fat_ptr_eq"
            | "ptr_cmp"
            | "slice_ptr_cmp"
            | "fat_ptr_cmp"
            | "addr"
            | "slice_from_parts"
            | "sfp"
            | "chunk_slice"
            | "thin"
            | "tail_raw"
            | "tail_ref"
            | "tail_slice"
            | "tail_slice_raw"
            | "read_array"
            | "array_ref"
            | "write_array"
            | "unwindow"
            | "unwindow_slice"
            | "byte_slice_ref"
            | "str_bytes"
            | "str_bytes_mut"
            | "detach_str_memo"
            | "bytes_str"
            | "str_len"
            | "str_to_lowercase"
            | "str_to_uppercase"
            | "bytes_is_ascii"
            | "nstr_with_cap"
            | "nstr_reserve"
            | "nstr_into_vec"
            | "nstr_into_nvec"
            | "nstr_ascii_lower"
            | "nstr_ascii_upper"
            | "nstr_repeat"
            | "bstr_in"
            | "bstr_out"
            | "bstr_bytes"
            | "bstr_bytes_mut"
            | "bstr_of"
            | "bstr_get"
            | "bstr_get_from"
            | "bstr_get_to"
            | "bstr_sub"
            | "bstr_sub_from"
            | "bstr_sub_to"
            | "bstr_cmp"
            | "bstr_cmp_range"
            | "bstr_eq_arr"
            | "bstr_cmp_ptr"
            | "ptr_cmp_bstr"
            | "bstr_copy_to"
            | "bstr_char"
            | "bstr_find_char"
            | "bstr_rfind_char"
            | "bstr_find_str"
            | "bstr_rfind_str"
            | "bstr_trim"
            | "bstr_trim_start"
            | "bstr_trim_end"
            | "bstr_to_lowercase"
            | "bstr_to_uppercase"
            | "sb_new"
            | "sb_push"
            | "sb_push_char"
            | "sb_take"
            | "f2i"
            | "f2i_big"
            | "i2f32_big"
            | "fmod"
            | "fmodf"
            | "native_f64_to_string"
            | "native_f32_to_string"
            | "fmt_float_fixed"
            | "fmt_int_hex"
            | "fmt_int_dec"
            | "rcgjs_fmt_float_parts"
            | "parse_f64"
            | "parse_f32"
            | "rc_alloc_slice"
            | "arc_alloc_slice"
            | "typed_alloc"
            | "typed_alloc_arr"
            | "typed_realloc"
            | "typed_dealloc"
            | "typed_dealloc_raw"
            | "t_alloc_arr"
            | "t_alloc_z"
            | "t_realloc_ta"
            | "t_realloc_ta_checked"
            | "t_realloc_arr"
            | "t_realloc_arr_checked"
            | "t_dealloc_maybe"
            | "t_box_new_tracked"
            | "box_new"
            | "native_vec_new"
            | "native_vec_set_capacity"
            | "native_vec_set_len"
            | "typed_vec_push"
            | "nv_push_arr"
            | "nv_push_ta"
            | "nv_pop_arr"
            | "nv_pop_ta"
            | "nv_with_cap_arr"
            | "nv_with_cap_ta"
            | "nv_zeroed_arr"
            | "nv_zeroed_ta"
            | "nv_zeroed_arr_fn"
            | "nv_zeroed_arr_fn_tracked"
            | "nv_reserve_arr"
            | "nv_reserve_ta"
            | "nv_try_reserve_arr"
            | "nv_try_reserve_ta"
            | "nv_shrink_arr"
            | "nv_shrink_ta"
            | "nv_grow_one_arr"
            | "nv_grow_one_ta"
            | "nv_from_raw_arr"
            | "nv_from_raw_ta"
            | "nv_box_track_arr"
            | "nv_box_track_slot"
            | "nv_idx"
            | "nv_grow_u8"
            | "nv_grow_i8"
            | "nv_grow_u16"
            | "nv_grow_i16"
            | "nv_grow_u32"
            | "nv_grow_i32"
            | "nv_grow_f32"
            | "nv_grow_f64"
            | "nv_with_cap_u8"
            | "nv_with_cap_i8"
            | "nv_with_cap_u16"
            | "nv_with_cap_i16"
            | "nv_with_cap_u32"
            | "nv_with_cap_i32"
            | "nv_with_cap_f32"
            | "nv_with_cap_f64"
            | "nv_zeroed_u8"
            | "nv_zeroed_i8"
            | "nv_zeroed_u16"
            | "nv_zeroed_i16"
            | "nv_zeroed_u32"
            | "nv_zeroed_i32"
            | "nv_zeroed_f32"
            | "nv_zeroed_f64"
            | "nv_reserve_u8"
            | "nv_reserve_i8"
            | "nv_reserve_u16"
            | "nv_reserve_i16"
            | "nv_reserve_u32"
            | "nv_reserve_i32"
            | "nv_reserve_f32"
            | "nv_reserve_f64"
            | "nv_zeroed_arr_p"
            | "nv_zeroed_arr_fn_p"
            | "nv_zeroed_arr_fn_tracked_p"
            | "nv_resize_zero_ta"
            | "slice_sort"
    )
}

/// Outcome of link-time enum representation specialization and signed-`Int32` Smi `u32` struct
/// field analysis.
#[derive(Clone, Debug, Default)]
pub struct EnumReprPlan {
    pub decisions: BTreeMap<TypeId, EnumRepr>,
    pub null_niche_count: usize,
    pub sentinel_niche_count: usize,
    pub int_tag_fieldless_count: usize,
    pub int_tag_payload_count: usize,
    /// Collision-free module-scope sentinel binding (`const $sn = {};`) allocated when
    /// `opts.chunks.is_empty()` and at least one [`EnumRepr::SentinelNiche`] enum is selected.
    pub sentinel_none: Option<JsName>,
    /// `(peeled_struct_ty, field_idx)` pairs for internal multi-field structs whose plain `u32`
    /// fields never escape by reference/raw-pointer/transmute/static/JS-boundary and can therefore
    /// be stored as signed `Int32` (`-2^31..=2^31-1`) so V8 keeps them as Smi instead of
    /// boxing `u32::MAX` (`4294967295`) into a `HeapNumber` `Double` field.
    pub smi_u32_fields: BTreeSet<(TypeId, u32)>,
}

impl EnumReprPlan {
    pub fn is_empty(&self) -> bool {
        self.decisions.is_empty() && self.smi_u32_fields.is_empty()
    }

    /// Synthesized module-scope sentinel item (`const $sn = {};`) when [`EnumRepr::SentinelNiche`]
    /// is active; empty otherwise.
    pub fn sentinel_items(&self) -> Vec<LinkItem> {
        let Some(name) = self.sentinel_none.clone() else {
            return Vec::new();
        };
        let js = format!("\nconst {} = {{}};\n", name.as_str());
        vec![synthesized_static_item(
            name,
            js,
            "enum SentinelNiche none sentinel",
        )]
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

    let global_types = build_global_type_map(table);
    let (excluded_enums, host_boundary_types, smi_u32_fields) = {
        let mut scanner =
            EnumBoundaryScanner::new(table, offered, extra_excluded, &global_types, opts);
        let live_set: BTreeSet<&str> = reachable.iter().map(JsName::as_str).collect();
        scanner.scan_live_items(&live_set);
        let smi_u32_fields = scanner.smi.select(&global_types, &scanner.excluded_enums);
        (
            scanner.excluded_enums,
            scanner.host_boundary_types,
            smi_u32_fields,
        )
    };

    let mut plan = select_enum_reprs(
        table,
        &global_types,
        &excluded_enums,
        &host_boundary_types,
        opts,
    );
    plan.smi_u32_fields = smi_u32_fields;

    if !plan.decisions.is_empty() {
        rewrite_shared_interns(table, reachable, &plan);
    }
    plan
}

/// Scans every live item's descriptors, signatures, and reachable MIR blocks to record boundary
/// enum exclusions, host-boundary types, and `smi_u32_fields` disqualifications.
struct EnumBoundaryScanner<'a> {
    table: &'a BTreeMap<JsName, LinkItem>,
    offered_set: BTreeSet<&'a str>,
    items_by_id: BTreeMap<ItemId, &'a str>,
    drop_glue_by_ty: BTreeMap<TypeId, &'a str>,
    global_types: &'a BTreeMap<TypeId, TypeDescriptor>,
    opts: &'a Opts,
    excluded_enums: BTreeSet<TypeId>,
    host_boundary_types: BTreeSet<TypeId>,
    visited_excl: BTreeSet<TypeId>,
    visited_host: BTreeSet<TypeId>,
    smi: SmiScanState,
}

impl<'a> EnumBoundaryScanner<'a> {
    fn new(
        table: &'a BTreeMap<JsName, LinkItem>,
        offered: &'a [JsName],
        extra_excluded: &BTreeSet<TypeId>,
        global_types: &'a BTreeMap<TypeId, TypeDescriptor>,
        opts: &'a Opts,
    ) -> Self {
        let offered_set: BTreeSet<&'a str> = offered.iter().map(JsName::as_str).collect();
        let items_by_id: BTreeMap<ItemId, &'a str> = table
            .values()
            .filter_map(|item| {
                let id = item
                    .jir
                    .summary
                    .as_ref()
                    .and_then(|s| s.key.item)
                    .or_else(|| item.jir.body.as_ref().map(|b| b.item))?;
                Some((id, item.name.as_str()))
            })
            .collect();
        let drop_glue_by_ty: BTreeMap<TypeId, &'a str> = table
            .values()
            .filter_map(|item| {
                let body = item.jir.body.as_ref()?;
                let BodySemantic::DropGlue(ty) = body.semantic else {
                    return None;
                };
                Some((peel_global(ty, global_types), item.name.as_str()))
            })
            .collect();

        let mut excluded_enums = extra_excluded.clone();
        let mut visited_excl = BTreeSet::new();
        for &ty in extra_excluded {
            walk_type_id_global(ty, global_types, &mut excluded_enums, &mut visited_excl);
        }

        Self {
            table,
            offered_set,
            items_by_id,
            drop_glue_by_ty,
            global_types,
            opts,
            excluded_enums,
            host_boundary_types: BTreeSet::new(),
            visited_excl,
            visited_host: BTreeSet::new(),
            smi: SmiScanState::default(),
        }
    }

    fn walk_excluded(&mut self, ty: TypeId, interns: &Interns) {
        walk_type_id(
            ty,
            interns,
            self.global_types,
            &mut self.excluded_enums,
            &mut self.visited_excl,
        );
    }

    fn walk_both(&mut self, ty: TypeId, interns: &Interns) {
        walk_type_id(
            ty,
            interns,
            self.global_types,
            &mut self.excluded_enums,
            &mut self.visited_excl,
        );
        walk_type_id(
            ty,
            interns,
            self.global_types,
            &mut self.host_boundary_types,
            &mut self.visited_host,
        );
    }

    fn walk_signature_both(&mut self, body: &Body, interns: &Interns) {
        for &input in &body.signature.arguments {
            self.walk_both(input, interns);
        }
        self.walk_both(body.signature.result, interns);
    }

    fn walk_signature_excluded(&mut self, body: &Body, interns: &Interns) {
        for &input in &body.signature.arguments {
            self.walk_excluded(input, interns);
        }
        self.walk_excluded(body.signature.result, interns);
    }

    fn scan_live_items(&mut self, live_set: &BTreeSet<&str>) {
        for item in self.table.values() {
            if !live_set.contains(item.name.as_str()) {
                continue;
            }
            self.scan_item(item);
        }
    }

    fn scan_item(&mut self, item: &LinkItem) {
        let Some(interns) = item.jir.interns() else {
            return;
        };
        let local = &interns.types;

        // 1. Pinned C-ABI descriptors walk transitively; `PinnedBoundary::Host` pins only the
        // host descriptor itself so user types stored inside `mires_js::HashMap` are not
        // spuriously poisoned.
        for desc in local.descriptors() {
            if desc.meta.abi == NominalAbi::C {
                self.walk_both(desc.id, interns);
            } else if desc.meta.boundary == PinnedBoundary::Host {
                self.excluded_enums.insert(desc.id);
                self.host_boundary_types.insert(desc.id);
            }
        }

        let Some(body) = item.jir.body.as_ref() else {
            return;
        };

        if item.kind == ItemKind::Static {
            self.smi
                .record_static(body.signature.result, interns, self.global_types);
        }

        // 2. Exported / imported / DTS-bearing / async export-out items cross the JS boundary.
        let is_js_boundary_item = item.linkage.exported
            || item.kind == ItemKind::Import
            || item.dts.is_some()
            || body.semantic == BodySemantic::JsExportOut;
        let item_is_offered = self.offered_set.contains(item.name.as_str());
        if is_js_boundary_item {
            self.walk_signature_both(body, interns);
        } else if !item_is_offered {
            self.walk_signature_excluded(body, interns);
        }

        // 3. Scan reachable MIR blocks for calls, drops, and constant trees crossing boundaries.
        let reachable_blocks = reachable_mir_blocks(
            body,
            item_is_offered && self.opts.applies(OptimizerPass::AstDce),
        );
        for block in &body.blocks {
            if !reachable_blocks.contains(&block.id) {
                continue;
            }
            for stmt in &block.statements {
                self.scan_stmt(stmt, body, local, interns);
            }
            self.scan_terminator(&block.terminator, item_is_offered, local, interns);
        }
    }

    fn scan_stmt(&mut self, stmt: &Statement, body: &Body, local: &TypeTable, interns: &Interns) {
        match &stmt.kind {
            StatementKind::Assign {
                destination: place,
                value: rv,
            } => match rv {
                Rvalue::Cast {
                    kind,
                    value: op,
                    result,
                } => {
                    self.scan_operand_consts(op, local, interns);
                    self.smi
                        .record_cast(*kind, op, *result, local, interns, self.global_types);
                }
                Rvalue::Use(op) => {
                    self.scan_operand_consts(op, local, interns);
                    self.smi
                        .record_use_assign(place, op, &body.locals, local, self.global_types);
                }
                Rvalue::Repeat { value: op, .. }
                | Rvalue::Unary { value: op, .. }
                | Rvalue::WrapUnsafeBinder(op) => self.scan_operand_consts(op, local, interns),
                Rvalue::Binary { left, right, .. } => {
                    self.scan_operand_consts(left, local, interns);
                    self.scan_operand_consts(right, local, interns);
                }
                Rvalue::Aggregate {
                    result, operands, ..
                } => {
                    for op in operands {
                        self.scan_operand_consts(op, local, interns);
                        self.smi
                            .record_aggregate_operand(*result, op, local, self.global_types);
                    }
                }
                Rvalue::Reference { place, .. }
                | Rvalue::RawPointer { place, .. }
                | Rvalue::Reborrow { place, .. } => {
                    self.smi
                        .record_ref_place(place, &body.locals, local, self.global_types);
                }
                Rvalue::Discriminant(_) | Rvalue::CopyForDeref(_) => {}
            },
            StatementKind::Assume(op) => self.scan_operand_consts(op, local, interns),
            StatementKind::CopyNonOverlapping {
                source,
                destination,
                count,
            } => {
                for op in [source, destination, count] {
                    self.scan_operand_consts(op, local, interns);
                }
            }
            StatementKind::SetDiscriminant { .. }
            | StatementKind::StorageLive(_)
            | StatementKind::StorageDead(_)
            | StatementKind::Nop => {}
        }
    }

    fn scan_terminator(
        &mut self,
        term: &Terminator,
        item_is_offered: bool,
        local: &TypeTable,
        interns: &Interns,
    ) {
        match term {
            Terminator::Switch { discriminant, .. } => {
                self.scan_operand_consts(discriminant, local, interns);
            }
            Terminator::Assert { condition, .. } => {
                self.scan_operand_consts(condition, local, interns);
            }
            Terminator::Drop {
                place,
                glue:
                    DropGlue::Direct {
                        item: tid,
                        link_name,
                    },
                ..
            } => {
                let target_name = self
                    .items_by_id
                    .get(tid)
                    .copied()
                    .or_else(|| interns.str(*link_name));
                if !target_name.is_some_and(|n| self.offered_set.contains(n)) {
                    self.walk_excluded(place.ty, interns);
                }
            }
            Terminator::Call(call) => {
                self.scan_call(call, item_is_offered, local, interns);
            }
            _ => {}
        }
    }

    fn scan_call(
        &mut self,
        call: &Call,
        item_is_offered: bool,
        local: &TypeTable,
        interns: &Interns,
    ) {
        self.scan_operand_consts(&call.function, local, interns);
        for arg in &call.arguments {
            self.scan_operand_consts(arg, local, interns);
        }

        let map_marker = match &call.host {
            Some(HostPayload::Map { marker }) => interns.str(*marker),
            Some(HostPayload::Marker { section, .. }) => interns
                .str(*section)
                .and_then(|s| s.strip_prefix("rcgjs.map.")),
            _ => None,
        };
        if let Some(marker) = map_marker {
            let stored_ty = if matches!(marker, "try_get" | "try_getmut") {
                option_ref_inner_pointee(local, self.global_types, call.destination.ty)
            } else if marker == "values" {
                slice_of_ref_pointee(local, self.global_types, call.destination.ty)
            } else {
                None
            };
            if let Some(stored_ty) = stored_ty {
                let peeled = peel_global(stored_ty, self.global_types);
                if self
                    .global_types
                    .get(&peeled)
                    .is_some_and(|d| d.js.enum_repr.is_some())
                {
                    self.excluded_enums.insert(peeled);
                }
            }
            return;
        }

        self.smi
            .record_call(call, local, interns, self.global_types);

        if item_is_offered {
            if let Some(HostPayload::Marker { section, .. }) = &call.host {
                if let Some(marker) = interns
                    .str(*section)
                    .and_then(|s| s.strip_prefix("rcgjs.host."))
                {
                    if self.handle_opaque_host_marker(marker, call, interns) {
                        return;
                    }
                }
            }
        }

        let is_internal_foreign = match &call.callee {
            Callee::Foreign { symbol, .. } => interns.str(*symbol).is_some_and(|sym| {
                self.table.get(sym).is_some_and(|target| {
                    target.kind == ItemKind::Alloc
                        || (target.kind == ItemKind::Fn
                            && self.offered_set.contains(sym)
                            && !target.linkage.exported
                            && target.dts.is_none())
                })
            }),
            _ => false,
        };

        let is_host_boundary = call.host.is_some()
            || matches!(
                call.semantic,
                CallSemantic::Host | CallSemantic::RawJavaScript
            )
            || (matches!(call.callee, Callee::Foreign { .. }) && !is_internal_foreign)
            || (matches!(call.callee, Callee::Runtime { .. })
                && !is_representation_opaque_runtime(&call.callee, interns));

        let is_non_offered_direct = match &call.callee {
            Callee::Direct {
                item: tid,
                link_name,
            } => {
                let target_name = self
                    .items_by_id
                    .get(tid)
                    .copied()
                    .or_else(|| interns.str(*link_name));
                !target_name.is_some_and(|n| {
                    self.offered_set.contains(n)
                        || self.table.get(n).is_some_and(|t| t.kind == ItemKind::Alloc)
                })
            }
            _ => false,
        };

        if is_host_boundary {
            for arg in &call.arguments {
                self.walk_both(arg.ty, interns);
            }
            self.walk_both(call.destination.ty, interns);
            if let Some(host) = &call.host {
                for (_, helper) in host.helpers() {
                    if let Some(&helper_name) = self.items_by_id.get(&helper.item) {
                        if let Some(helper_item) = self.table.get(helper_name) {
                            if let (Some(h_body), Some(h_interns)) =
                                (&helper_item.jir.body, helper_item.jir.interns())
                            {
                                self.walk_signature_both(h_body, h_interns);
                            }
                        }
                    }
                }
            }
        } else if is_non_offered_direct {
            for arg in &call.arguments {
                self.walk_excluded(arg.ty, interns);
            }
            self.walk_excluded(call.destination.ty, interns);
        }
    }

    /// Handles closure-taking and async-driver `rcgjs.host.*` markers whose JS host wrappers store
    /// a Rust closure environment or coroutine future opaquely without inspecting its captured
    /// fields or saved locals.
    fn handle_opaque_host_marker(&mut self, marker: &str, call: &Call, interns: &Interns) -> bool {
        // 1. Closure-taking markers: JS receives `(...$a) => body(env, ...$a)`.
        let closure_idx = match marker {
            "callback0" | "callback1" | "callback2" | "callback3" | "callback4" | "promise"
            | "localwaker" | "microtask" | "task" | "try"
                if call.arguments.len() == 1 =>
            {
                Some(0usize)
            }
            "settled" if call.arguments.len() == 2 => Some(1usize),
            _ => None,
        };
        if let Some(c_idx) = closure_idx {
            let closure_peeled = peel_global(call.arguments[c_idx].ty, self.global_types);
            let Some(closure_desc) = self.global_types.get(&closure_peeled) else {
                return false;
            };
            let TypeKind::Closure { def, .. } = closure_desc.kind else {
                return false;
            };
            let closure_item_id = ItemId {
                definition: def,
                instantiation: closure_peeled.0,
            };
            let Some(&closure_name) = self.items_by_id.get(&closure_item_id) else {
                return false;
            };
            if !self.offered_set.contains(closure_name) {
                return false;
            }
            let Some(closure_item) = self.table.get(closure_name) else {
                return false;
            };
            let (Some(c_body), Some(c_interns)) =
                (closure_item.jir.body.as_ref(), closure_item.jir.interns())
            else {
                return false;
            };
            if marker == "try" && closure_desc.layout.needs_drop {
                let Some(&glue_name) = self.drop_glue_by_ty.get(&closure_peeled) else {
                    return false;
                };
                if !self.offered_set.contains(glue_name) {
                    return false;
                }
            }

            for (i, arg) in call.arguments.iter().enumerate() {
                if i != c_idx {
                    self.walk_both(arg.ty, interns);
                }
            }
            self.walk_both(call.destination.ty, interns);
            for &inp in c_body.signature.arguments.iter().skip(1) {
                self.walk_both(inp, c_interns);
            }
            self.walk_both(c_body.signature.result, c_interns);
            return true;
        }

        // 2. Async-driver markers: JS stores `state.future` opaquely.
        let (is_promise, expected_args) = match marker {
            "futurepromise" | "futurenativepromise" => (true, 1usize),
            "spawn" => (false, 1usize),
            "spawnowner" | "spawnowner.v2" => (false, 2usize),
            _ => return false,
        };
        if !self.opts.applies(OptimizerPass::FuturePoller) || call.arguments.len() != expected_args
        {
            return false;
        }
        let Some(host) = &call.host else {
            return false;
        };
        let helper_key = if is_promise {
            "poll_result"
        } else {
            "poll_once"
        };
        let Some(helper_ref) = host.helper(interns, helper_key) else {
            return false;
        };
        let Some(helper_name) = self
            .items_by_id
            .get(&helper_ref.item)
            .copied()
            .or_else(|| interns.str(helper_ref.link_name))
        else {
            return false;
        };
        if !self.offered_set.contains(helper_name) {
            return false;
        }
        let Some(helper_item) = self.table.get(helper_name) else {
            return false;
        };
        let (Some(h_body), Some(h_interns)) =
            (helper_item.jir.body.as_ref(), helper_item.jir.interns())
        else {
            return false;
        };
        let future_peeled = peel_global(call.arguments[0].ty, self.global_types);
        let Some(future_desc) = self.global_types.get(&future_peeled) else {
            return false;
        };
        if future_desc.layout.needs_drop {
            let Some(&glue_name) = self.drop_glue_by_ty.get(&future_peeled) else {
                return false;
            };
            if !self.offered_set.contains(glue_name) {
                return false;
            }
        }

        for arg in call.arguments.iter().skip(1) {
            self.walk_both(arg.ty, interns);
        }
        self.walk_both(call.destination.ty, interns);
        for &inp in h_body.signature.arguments.iter().skip(1) {
            self.walk_both(inp, h_interns);
        }
        self.walk_both(h_body.signature.result, h_interns);
        true
    }

    fn is_offered_internal_static(&self, target: &GlobalReference, interns: &Interns) -> bool {
        let GlobalReference::Static { link_name, .. } = target else {
            return false;
        };
        let Some(name_str) = interns.str(*link_name) else {
            return false;
        };
        self.offered_set.contains(name_str)
            && self
                .table
                .get(name_str)
                .is_some_and(|item| !item.linkage.exported && item.dts.is_none())
    }

    fn scan_operand_consts(&mut self, op: &Operand, local: &TypeTable, interns: &Interns) {
        let OperandKind::Constant(constant) = &op.kind else {
            return;
        };
        match constant {
            Constant::Pointer {
                target: target @ GlobalReference::Static { .. },
                ..
            } => {
                if !self.is_offered_internal_static(target, interns) {
                    self.walk_excluded(op.ty, interns);
                }
            }
            Constant::Pointer {
                target: GlobalReference::Function { link_name, .. },
                ..
            } => {
                let offered = interns
                    .str(*link_name)
                    .is_some_and(|s| self.offered_set.contains(s));
                if !offered {
                    self.walk_excluded(op.ty, interns);
                }
            }
            Constant::Memory(tree) => {
                self.scan_const_tree(tree, op.ty, local, interns, 0);
            }
            _ => {}
        }
    }

    fn scan_const_tree(
        &mut self,
        tree: &ConstTree,
        ty: TypeId,
        local: &TypeTable,
        interns: &Interns,
        depth: u32,
    ) {
        if depth > 32 {
            return;
        }
        match tree {
            ConstTree::Global {
                target: target @ GlobalReference::Static { .. },
                ..
            }
            | ConstTree::StaticField { target, .. }
            | ConstTree::StaticSlice { target, .. } => {
                if !self.is_offered_internal_static(target, interns) {
                    self.walk_excluded(ty, interns);
                }
            }
            ConstTree::Global {
                target: GlobalReference::Function { link_name, .. },
                ..
            } => {
                let offered = interns
                    .str(*link_name)
                    .is_some_and(|s| self.offered_set.contains(s));
                if !offered {
                    self.walk_excluded(ty, interns);
                }
            }
            ConstTree::Reference(inner) => {
                let peeled = peel_global(ty, self.global_types);
                if let Some(desc) = lookup_desc(peeled, local, self.global_types) {
                    if let TypeKind::Reference { pointee, .. }
                    | TypeKind::RawPointer { pointee, .. } = desc.kind
                    {
                        self.scan_const_tree(inner, pointee, local, interns, depth + 1);
                    }
                }
            }
            ConstTree::Slice(elems) => {
                let peeled = peel_global(ty, self.global_types);
                if let Some(desc) = lookup_desc(peeled, local, self.global_types) {
                    let elem_ty = match desc.kind {
                        TypeKind::Slice(e) | TypeKind::Array { element: e, .. } => Some(e),
                        TypeKind::Reference { pointee, .. }
                        | TypeKind::RawPointer { pointee, .. } => match lookup_desc(
                            peel_global(pointee, self.global_types),
                            local,
                            self.global_types,
                        )
                        .map(|d| &d.kind)
                        {
                            Some(TypeKind::Slice(e) | TypeKind::Array { element: e, .. }) => {
                                Some(*e)
                            }
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(e) = elem_ty {
                        for elem in elems {
                            self.scan_const_tree(elem, e, local, interns, depth + 1);
                        }
                    }
                }
            }
            ConstTree::Aggregate(fields) => {
                let peeled = peel_global(ty, self.global_types);
                if let Some(desc) = lookup_desc(peeled, local, self.global_types) {
                    match &desc.kind {
                        TypeKind::Array { element, .. } => {
                            let elem = *element;
                            for f in fields {
                                self.scan_const_tree(f, elem, local, interns, depth + 1);
                            }
                        }
                        TypeKind::Tuple(elems) => {
                            for (f, &e_ty) in fields.iter().zip(elems) {
                                self.scan_const_tree(f, e_ty, local, interns, depth + 1);
                            }
                        }
                        TypeKind::Adt { variants, .. } => {
                            if let Some(v) = variants.first() {
                                for (f, fd) in fields.iter().zip(&v.fields) {
                                    self.scan_const_tree(f, fd.ty, local, interns, depth + 1);
                                }
                            }
                        }
                        TypeKind::Closure { upvars, .. }
                        | TypeKind::CoroutineClosure { upvars, .. } => {
                            for (f, &u_ty) in fields.iter().zip(upvars) {
                                self.scan_const_tree(f, u_ty, local, interns, depth + 1);
                            }
                        }
                        _ => {}
                    }
                }
            }
            ConstTree::Variant { variant, fields } => {
                let peeled = peel_global(ty, self.global_types);
                if let Some(desc) = lookup_desc(peeled, local, self.global_types) {
                    if let TypeKind::Adt { variants, .. } = &desc.kind {
                        if let Some(v) = variants.get(*variant as usize) {
                            for (f, fd) in fields.iter().zip(&v.fields) {
                                self.scan_const_tree(f, fd.ty, local, interns, depth + 1);
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
                self.scan_const_tree(data, *concrete, local, interns, depth + 1);
                if let Some(vt_name) = interns.str(*vtable) {
                    if let Some(vt_item) = self.table.get(vt_name) {
                        for r in &vt_item.refs {
                            if let Some(ref_item) = self.table.get(r) {
                                if ref_item.kind == ItemKind::Fn
                                    && !self.offered_set.contains(r.as_str())
                                {
                                    self.walk_excluded(*concrete, interns);
                                    if let (Some(rb), Some(ri)) =
                                        (&ref_item.jir.body, ref_item.jir.interns())
                                    {
                                        self.walk_signature_excluded(rb, ri);
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
}

/// Candidate 2-variant enum eligible for bottom-up `NullNiche` / `SentinelNiche` selection.
struct NicheCandidate {
    depth: u32,
    id: TypeId,
    none_variant: u32,
    some_variant: u32,
    some_field: u32,
    single_field: bool,
    payload_ty: TypeId,
}

/// Computes canonical bottom-up enum representation decisions over `global_types`.
fn select_enum_reprs(
    table: &BTreeMap<JsName, LinkItem>,
    global_types: &BTreeMap<TypeId, TypeDescriptor>,
    excluded_enums: &BTreeSet<TypeId>,
    host_boundary_types: &BTreeSet<TypeId>,
    opts: &Opts,
) -> EnumReprPlan {
    let mut plan = EnumReprPlan::default();

    // Collect eligible internal `EnumRepr::Tag` enums.
    let mut candidates: Vec<TypeId> = Vec::new();
    for (&id, desc) in global_types {
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
                    .any(|f| is_uninhabited_global(f.ty, global_types))
            })
            .count();
        if inhabited_count < 2 {
            continue;
        }
        candidates.push(id);
    }

    // Pass A: All-ZST-field enums -> `IntTag { fieldless: true }`; 2-variant option-like enums ->
    // Pass B; everything else -> Pass C.
    let mut niche_candidates: Vec<NicheCandidate> = Vec::new();
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
                .all(|v| v.fields.iter().all(|f| is_zst_global(f.ty, global_types)))
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
                .all(|f| is_zst_global(f.ty, global_types));
            let v1_zst = variants[1]
                .fields
                .iter()
                .all(|f| is_zst_global(f.ty, global_types));
            let niche = if v0_zst {
                single_non_zst_field(&variants[1].fields, global_types).map(
                    |(some_field, payload_ty)| NicheCandidate {
                        depth: transparent_enum_depth(payload_ty, global_types),
                        id,
                        none_variant: 0,
                        some_variant: 1,
                        some_field,
                        single_field: variants[1].fields.len() == 1,
                        payload_ty,
                    },
                )
            } else if v1_zst {
                single_non_zst_field(&variants[0].fields, global_types).map(
                    |(some_field, payload_ty)| NicheCandidate {
                        depth: transparent_enum_depth(payload_ty, global_types),
                        id,
                        none_variant: 1,
                        some_variant: 0,
                        some_field,
                        single_field: variants[0].fields.len() == 1,
                        payload_ty,
                    },
                )
            } else {
                None
            };
            if let Some(cand) = niche {
                if !is_zst_global(cand.payload_ty, global_types)
                    && !is_uninhabited_global(cand.payload_ty, global_types)
                {
                    niche_candidates.push(cand);
                    continue;
                }
            }
        }
        remaining_tag_enums.push((id, fieldless));
    }

    // Pass B: NullNiche / SentinelNiche in ascending nesting depth so inner `Option<T>` is decided
    // before `Option<Option<T>>` / `Option<ManuallyDrop<Option<T>>>`.
    let sentinel_candidate = (!crate::link::is_chunked(opts, table))
        .then(|| NamePool::from_table(table).alloc_suffixed("$sn"))
        .flatten();
    niche_candidates.sort_by_key(|c| (c.depth, c.id));
    for cand in niche_candidates {
        if cand.single_field
            && cand.some_field == 0
            && !may_inhabit_null(
                cand.payload_ty,
                global_types,
                host_boundary_types,
                &plan.decisions,
            )
        {
            plan.decisions.insert(
                cand.id,
                EnumRepr::NullNiche {
                    none_variant: cand.none_variant,
                    some_variant: cand.some_variant,
                },
            );
            plan.null_niche_count += 1;
        } else if sentinel_candidate.is_some()
            && !may_inhabit_sentinel_none(cand.payload_ty, global_types, &plan.decisions)
        {
            plan.decisions.insert(
                cand.id,
                EnumRepr::SentinelNiche {
                    none_variant: cand.none_variant,
                    some_variant: cand.some_variant,
                    some_field: cand.some_field,
                },
            );
            plan.sentinel_niche_count += 1;
        } else {
            remaining_tag_enums.push((cand.id, false));
        }
    }
    if plan.sentinel_niche_count > 0 {
        plan.sentinel_none = sentinel_candidate;
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

    plan
}

/// Applies `plan.decisions` to every live item's `TypeTable`, deduplicating mutated `Arc<Interns>`
/// by original pointer identity so items from the same crate share one table.
fn rewrite_shared_interns(
    table: &mut BTreeMap<JsName, LinkItem>,
    reachable: &[JsName],
    plan: &EnumReprPlan,
) {
    let mut mutated_interns: BTreeMap<usize, Arc<Interns>> = BTreeMap::new();
    let sentinel_str = plan.sentinel_none.as_ref().map(JsName::as_str);
    for name in reachable {
        let Some(item) = table.get_mut(name) else {
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
            apply_decisions_to_table(&mut cloned.types, &plan.decisions, sentinel_str);
            let arc = Arc::new(cloned);
            mutated_interns.insert(key, arc.clone());
            arc
        };
        item.jir.use_shared_interns(shared);
    }
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

pub(crate) fn lookup_desc<'a>(
    ty: TypeId,
    local: &'a TypeTable,
    global: &'a BTreeMap<TypeId, TypeDescriptor>,
) -> Option<&'a TypeDescriptor> {
    local.get(ty).or_else(|| global.get(&ty))
}

pub(crate) fn peel_global(mut ty: TypeId, global: &BTreeMap<TypeId, TypeDescriptor>) -> TypeId {
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

fn single_non_zst_field(
    fields: &[FieldDescriptor],
    global: &BTreeMap<TypeId, TypeDescriptor>,
) -> Option<(u32, TypeId)> {
    let mut found = None;
    for (idx, field) in fields.iter().enumerate() {
        if is_zst_global(field.ty, global) {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some((idx as u32, field.ty));
    }
    found
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
                let next = single_non_zst_field(&variants[0].fields, global)
                    .or_else(|| single_non_zst_field(&variants[1].fields, global))
                    .map(|(_, inner)| inner);
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
        if matches!(
            decisions.get(&ty),
            Some(EnumRepr::NullNiche { .. } | EnumRepr::SentinelNiche { .. })
        ) || matches!(
            desc.js.enum_repr,
            Some(EnumRepr::NullNiche { .. } | EnumRepr::SentinelNiche { .. })
        ) {
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

/// Whether our private module-scope sentinel (`const $sn = {};`) could ever be a valid runtime
/// value of `ty`.
fn may_inhabit_sentinel_none(
    mut ty: TypeId,
    global: &BTreeMap<TypeId, TypeDescriptor>,
    decisions: &BTreeMap<TypeId, EnumRepr>,
) -> bool {
    for _ in 0..16 {
        let Some(desc) = global.get(&ty) else {
            return true;
        };
        if matches!(
            desc.kind,
            TypeKind::Opaque | TypeKind::Foreign(_) | TypeKind::Never
        ) {
            return true;
        }
        let repr = decisions.get(&ty).copied().or(desc.js.enum_repr);
        match repr {
            Some(EnumRepr::SentinelNiche { .. }) => return true,
            Some(EnumRepr::NullNiche { some_variant, .. }) => {
                let TypeKind::Adt { variants, .. } = &desc.kind else {
                    return true;
                };
                let Some(payload_ty) = variants
                    .get(some_variant as usize)
                    .and_then(|v| v.fields.first())
                    .map(|f| f.ty)
                else {
                    return true;
                };
                ty = payload_ty;
                continue;
            }
            _ => {}
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

pub(crate) fn walk_type_id(
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

fn reachable_mir_blocks(body: &Body, fold_runtime_checks: bool) -> BTreeSet<BlockId> {
    let mut visited = BTreeSet::new();
    let Some(entry) = body.blocks.first().map(|b| b.id) else {
        return visited;
    };
    let by_id: BTreeMap<BlockId, usize> = body
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();
    let mut work = vec![entry];
    while let Some(bid) = work.pop() {
        if !visited.insert(bid) {
            continue;
        }
        let Some(&idx) = by_id.get(&bid) else {
            continue;
        };
        let block = &body.blocks[idx];
        let mut const_locals: BTreeMap<LocalId, u128> = BTreeMap::new();
        if fold_runtime_checks {
            for stmt in &block.statements {
                match &stmt.kind {
                    StatementKind::Assign { destination, value } => {
                        if destination.projection.contains(&Projection::Deref) {
                            const_locals.clear();
                        } else if destination.projection.is_empty() {
                            match value {
                                Rvalue::Use(op) => match &op.kind {
                                    OperandKind::RuntimeCheck(v) => {
                                        const_locals.insert(destination.local, u128::from(*v));
                                    }
                                    OperandKind::Constant(Constant::Scalar { bits, .. }) => {
                                        const_locals.insert(destination.local, *bits);
                                    }
                                    _ => {
                                        const_locals.remove(&destination.local);
                                    }
                                },
                                Rvalue::Reference { place, .. }
                                | Rvalue::RawPointer { place, .. }
                                | Rvalue::Reborrow { place, .. } => {
                                    const_locals.remove(&destination.local);
                                    const_locals.remove(&place.local);
                                }
                                _ => {
                                    const_locals.remove(&destination.local);
                                }
                            }
                        } else {
                            const_locals.remove(&destination.local);
                        }
                    }
                    StatementKind::CopyNonOverlapping { .. } => {
                        const_locals.clear();
                    }
                    _ => {}
                }
            }
        }
        let push_unwind = |u: Unwind, work: &mut Vec<BlockId>| {
            if let Unwind::Cleanup(b) = u {
                work.push(b);
            }
        };
        match &block.terminator {
            Terminator::Goto(b)
            | Terminator::FalseEdge { real: b, .. }
            | Terminator::FalseUnwind { real: b, .. } => {
                work.push(*b);
            }
            Terminator::Switch {
                discriminant,
                targets,
                otherwise,
            } => {
                let folded = if fold_runtime_checks {
                    match &discriminant.kind {
                        OperandKind::RuntimeCheck(v) => Some(u128::from(*v)),
                        OperandKind::Constant(Constant::Scalar { bits, .. }) => Some(*bits),
                        OperandKind::Copy(p) | OperandKind::Move(p) if p.projection.is_empty() => {
                            const_locals.get(&p.local).copied()
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                if let Some(val) = folded {
                    let target = targets
                        .iter()
                        .find_map(|&(bits, b)| (bits == val).then_some(b))
                        .unwrap_or(*otherwise);
                    work.push(target);
                } else {
                    work.push(*otherwise);
                    for &(_, b) in targets {
                        work.push(b);
                    }
                }
            }
            Terminator::Call(call) => {
                if let Some(t) = call.target {
                    work.push(t);
                }
                push_unwind(call.unwind, &mut work);
            }
            Terminator::Drop {
                target,
                unwind,
                async_drop,
                ..
            } => {
                work.push(*target);
                push_unwind(*unwind, &mut work);
                if let Some(b) = async_drop {
                    work.push(*b);
                }
            }
            Terminator::Assert {
                condition,
                expected,
                target,
                unwind,
                ..
            } => {
                work.push(*target);
                let proven_ok = fold_runtime_checks
                    && match &condition.kind {
                        OperandKind::RuntimeCheck(v) => *v == *expected,
                        OperandKind::Copy(p) | OperandKind::Move(p) if p.projection.is_empty() => {
                            const_locals
                                .get(&p.local)
                                .is_some_and(|&bits| (bits != 0) == *expected)
                        }
                        _ => false,
                    };
                if !proven_ok {
                    push_unwind(*unwind, &mut work);
                }
            }
            Terminator::Return
            | Terminator::Unreachable
            | Terminator::UnwindResume
            | Terminator::UnwindTerminate(_) => {}
        }
    }
    visited
}

/// Applies `decisions` to one `TypeTable`, cascading to transparent/pattern wrappers and
/// references whose pointee transitioned from `is_indirect = true` to `false`.
fn apply_decisions_to_table(
    table: &mut TypeTable,
    decisions: &BTreeMap<TypeId, EnumRepr>,
    sentinel_none: Option<&str>,
) {
    // Pass 1a: `IntTag { fieldless: true }` first so `needs_clone` is false when `NullNiche` /
    // `SentinelNiche` queries its payload.
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

    // Pass 1b: `NullNiche` before `SentinelNiche`, so `Option<Option<T>>` (`SentinelNiche`)
    // reads the updated `needs_clone` of its inner `NullNiche` `Option<T>`.
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
            let payload_needs_clone = table.needs_clone(table.peeled(payload_ty));
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
    let sentinel_niche_updates: Vec<(TypeId, EnumRepr, bool)> = decisions
        .iter()
        .filter_map(|(&id, &repr)| {
            let EnumRepr::SentinelNiche {
                some_variant,
                some_field,
                ..
            } = repr
            else {
                return None;
            };
            let desc = table.get(id)?;
            let TypeKind::Adt { variants, .. } = &desc.kind else {
                return None;
            };
            let payload_ty = variants
                .get(some_variant as usize)?
                .fields
                .get(some_field as usize)?
                .ty;
            let payload_needs_clone = table.needs_clone(table.peeled(payload_ty));
            Some((id, repr, payload_needs_clone))
        })
        .collect();
    for (id, repr, payload_needs_clone) in sentinel_niche_updates {
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
    table.set_sentinel_none(sentinel_none.map(str::to_owned));

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
    // direct (`!is_indirect`), then propagate updated `pointer_form` to transparent wrappers.
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
#[allow(
    clippy::too_many_lines,
    reason = "unit tests build multi-block JIR fixtures inline"
)]
mod tests {
    use super::*;
    use crate::item::{ItemMap, LinkJir, Linkage};
    use rcgjs_object::jir::interns::StrId;
    use rcgjs_object::jir::semantic::{
        AbiSignature, BasicBlock, BlockId, Body, BodyKind, BodySemantic, DefinitionId, Place,
        Signature, Terminator,
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
            allocator_provider: Default::default(),
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
        let reachable = vec![f_int.clone(), f_exp];

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
            Some(&EnumRepr::SentinelNiche {
                none_variant: 0,
                some_variant: 1,
                some_field: 0,
            })
        );
        assert_eq!(plan.sentinel_none.as_ref().map(JsName::as_str), Some("$sn"));
        assert_eq!(plan.sentinel_items().len(), 1);
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

    fn payload_enum_desc(
        id: TypeId,
        def_hash: u64,
        v0_name: StrId,
        v0_ty: TypeId,
        v1_name: StrId,
        v1_ty: TypeId,
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
                    local_hash: def_hash,
                },
                args: Vec::new(),
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: v0_name,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(f0_name),
                            ty: v0_ty,
                            offset: 4,
                        }],
                        logical_discr: 0,
                        physical_tag: Some(NominalDiscriminant { bits: 0, bytes: 1 }),
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: v1_name,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(f0_name),
                            ty: v1_ty,
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

    fn js_value_desc(id: TypeId) -> TypeDescriptor {
        TypeDescriptor {
            id,
            name: None,
            kind: TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 2,
                    local_hash: 900,
                },
                args: Vec::new(),
                form: NominalForm::Opaque,
                variants: Vec::new(),
                niche: None,
            },
            layout: LayoutFacts {
                size: Some(4),
                align: Some(4),
                sized: true,
                ..Default::default()
            },
            meta: TypeMeta {
                boundary: PinnedBoundary::Host,
                ..Default::default()
            },
            js: base_facts(id),
        }
    }

    fn custom_item(
        name: &str,
        item_id: ItemId,
        semantic: BodySemantic,
        interns: Arc<Interns>,
        arguments: Vec<TypeId>,
        result: TypeId,
        blocks: Vec<BasicBlock>,
    ) -> LinkItem {
        let mut jir = LinkJir::default();
        jir.body = Some(Body {
            item: item_id,
            kind: BodyKind::Function,
            semantic,
            signature: Signature {
                arguments,
                result,
                abi: AbiSignature::default(),
            },
            locals: Vec::new(),
            blocks,
        });
        jir.use_shared_interns(interns);
        LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            js: String::new(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: name.to_owned(),
            crate_name: "test".to_owned(),
            order: None,
            map: ItemMap::default(),
            dts: None,
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir,
        }
    }

    fn dummy_place(ty: TypeId) -> Place {
        Place {
            local: LocalId(0),
            projection: Vec::new(),
            ty,
        }
    }

    fn dummy_operand(ty: TypeId) -> Operand {
        Operand {
            ty,
            kind: OperandKind::Copy(dummy_place(ty)),
        }
    }

    #[test]
    fn opaque_runtime_allowlist_covers_audited_helpers_and_rejects_boundary_helpers() {
        let mut interns = Interns::default();
        for sym in [
            "write_bytes",
            "compare_bytes",
            "t_alloc_arr",
            "t_alloc_z",
            "t_realloc_arr",
            "t_realloc_arr_checked",
            "t_realloc_ta",
            "t_realloc_ta_checked",
            "t_dealloc_maybe",
            "t_box_new_tracked",
            "_put_plain",
            "_put_tracked",
            "rc_alloc_slice",
            "arc_alloc_slice",
            "typed_alloc_arr",
            "nstr_into_vec",
            "nstr_ascii_lower",
            "bstr_in",
            "bstr_out",
            "bstr_bytes",
            "bstr_of",
            "bstr_sub",
            "bstr_sub_from",
            "bstr_sub_to",
            "bstr_cmp",
            "bstr_cmp_range",
            "bstr_char",
            "bstr_find_char",
            "bstr_rfind_char",
            "bstr_find_str",
            "bstr_rfind_str",
            "bstr_trim",
            "bstr_trim_start",
            "bstr_trim_end",
            "js_abort",
            "abort_error",
            "panic",
            "unreachable",
            "nv_grow_u8",
            "nv_grow_i8",
            "nv_grow_u16",
            "nv_grow_i16",
            "nv_grow_u32",
            "nv_grow_i32",
            "nv_grow_f32",
            "nv_grow_f64",
            "nv_with_cap_u8",
            "nv_with_cap_i8",
            "nv_with_cap_u16",
            "nv_with_cap_i16",
            "nv_with_cap_u32",
            "nv_with_cap_i32",
            "nv_with_cap_f32",
            "nv_with_cap_f64",
            "nv_zeroed_u8",
            "nv_zeroed_i8",
            "nv_zeroed_u16",
            "nv_zeroed_i16",
            "nv_zeroed_u32",
            "nv_zeroed_i32",
            "nv_zeroed_f32",
            "nv_zeroed_f64",
            "nv_resize_zero_ta",
        ] {
            let id = interns.intern_str(sym);
            assert!(
                is_representation_opaque_runtime(
                    &Callee::Runtime {
                        member: id,
                        sysroot: true,
                    },
                    &interns,
                ),
                "sysroot helper `{sym}` must be representation-opaque"
            );
            assert!(
                !is_representation_opaque_runtime(
                    &Callee::Runtime {
                        member: id,
                        sysroot: false,
                    },
                    &interns,
                ),
                "non-sysroot helper `{sym}` must never be treated as opaque"
            );
        }
        for boundary in [
            "host_try",
            "host_spawn",
            "host_future_promise",
            "host_str_to_lowercase",
            "bstr_boundary",
            "bstr_ref",
            "bstr_ref_in",
            "bstr_str_in",
            "bstr_utf8_bytes",
            "map_new",
            "map_get",
            "map_set",
        ] {
            let id = interns.intern_str(boundary);
            assert!(
                !is_representation_opaque_runtime(
                    &Callee::Runtime {
                        member: id,
                        sysroot: true,
                    },
                    &interns,
                ),
                "boundary helper `{boundary}` must NOT be in is_representation_opaque_runtime"
            );
        }
    }

    #[test]
    fn opaque_runtime_host_try_async_foreign_and_dead_branch_specialize_internal_enums() {
        use rcgjs_object::jir::semantic::{Call, DirectItemRef};

        let mut interns = Interns::default();
        let s_none = interns.intern_str("None");
        let s_some = interns.intern_str("Some");
        let s_leaf = interns.intern_str("Leaf");
        let s_edge = interns.intern_str("Edge");
        let s_ok = interns.intern_str("Ok");
        let s_err = interns.intern_str("Err");
        let s_pending = interns.intern_str("Pending");
        let s_ready = interns.intern_str("Ready");
        let s_f0 = interns.intern_str("_0");
        let s_empty = interns.intern_str("");
        let s_t_alloc_arr = interns.intern_str("t_alloc_arr");
        let s_write_bytes = interns.intern_str("write_bytes");
        let s_nv_grow_u8 = interns.intern_str("nv_grow_u8");
        let s_bstr_boundary = interns.intern_str("bstr_boundary");
        let s_host_try = interns.intern_str("rcgjs.host.try");
        let s_host_fp = interns.intern_str("rcgjs.host.futurepromise");
        let s_poll_result_key = interns.intern_str("poll_result");
        let s_poll_result_name = interns.intern_str("poll_result_fn");
        let s_panic_sym = interns.intern_str("rust_begin_unwind");
        let s_ext_sym = interns.intern_str("external_c_fn");
        let s_v1_name = interns.intern_str("v1_only_fn");

        let t_unit = TypeId(1);
        let t_u32 = TypeId(2);
        let t_js = TypeId(3);
        let t_entry = TypeId(4);
        let t_opt_u32 = TypeId(5);
        let t_try_res = TypeId(6);
        let t_poll_res = TypeId(7);
        let t_boundary_enum = TypeId(8);
        let t_foreign_enum = TypeId(9);
        let t_closure = TypeId(10);
        let t_coro = TypeId(11);
        let t_ref_coro = TypeId(12);

        let closure_def = DefinitionId {
            crate_id: 1,
            local_hash: 500,
        };
        let closure_item_id = ItemId {
            definition: closure_def,
            instantiation: t_closure.0,
        };
        let poll_item_id = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 501,
            },
            instantiation: 0,
        };
        let v1_item_id = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 502,
            },
            instantiation: 0,
        };

        let mut closure_desc = TypeDescriptor {
            id: t_closure,
            name: None,
            kind: TypeKind::Closure {
                def: closure_def,
                upvars: vec![t_entry, t_opt_u32],
                fn_pointer_shim: None,
            },
            layout: LayoutFacts {
                size: Some(16),
                align: Some(4),
                sized: true,
                needs_drop: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(t_closure),
        };
        closure_desc.js.is_indirect = true;

        let mut coro_desc = TypeDescriptor {
            id: t_coro,
            name: None,
            kind: TypeKind::Coroutine {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 503,
                },
                upvars: vec![t_opt_u32],
                saved_locals: vec![(0, t_entry)],
                states: vec![vec![], vec![0]],
                variants: vec![
                    VariantDescriptor {
                        name: s_empty,
                        fields: Vec::new(),
                        logical_discr: 0,
                        physical_tag: Some(NominalDiscriminant { bits: 0, bytes: 1 }),
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: s_empty,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(s_f0),
                            ty: t_entry,
                            offset: 4,
                        }],
                        logical_discr: 1,
                        physical_tag: Some(NominalDiscriminant { bits: 1, bytes: 1 }),
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
            },
            layout: LayoutFacts {
                size: Some(16),
                align: Some(4),
                sized: true,
                needs_drop: false,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(t_coro),
        };
        coro_desc.js.is_indirect = true;

        for desc in [
            unit_desc(t_unit),
            u32_desc(t_u32),
            js_value_desc(t_js),
            payload_enum_desc(t_entry, 200, s_leaf, t_u32, s_edge, t_u32, s_f0),
            option_desc(t_opt_u32, t_u32, s_none, s_some, s_f0),
            payload_enum_desc(t_try_res, 201, s_ok, t_u32, s_err, t_js, s_f0),
            option_desc(t_poll_res, t_try_res, s_pending, s_ready, s_f0),
            payload_enum_desc(t_boundary_enum, 202, s_leaf, t_u32, s_edge, t_u32, s_f0),
            payload_enum_desc(t_foreign_enum, 203, s_leaf, t_u32, s_edge, t_u32, s_f0),
            closure_desc,
            coro_desc,
            ref_desc(t_ref_coro, t_coro, PointerForm::Thin),
        ] {
            interns.types.insert(desc);
        }
        let shared = Arc::new(interns);

        let mk_call = |id: u32,
                       callee: Callee,
                       semantic: CallSemantic,
                       host: Option<HostPayload>,
                       arguments: Vec<Operand>,
                       dest_ty: TypeId,
                       next: BlockId| BasicBlock {
            id: BlockId(id),
            cleanup: false,
            statements: Vec::new(),
            terminator: Terminator::Call(Call {
                function: dummy_operand(t_unit),
                callee,
                semantic,
                arguments,
                destination: dummy_place(dest_ty),
                target: Some(next),
                unwind: Unwind::Continue,
                track_caller: None,
                host,
            }),
        };

        // Main offered function exercises:
        // bb0: RuntimeCheck(false) switch -> dead bb1 (calls non-offered v1_only_fn(Entry)), live bb2
        // bb2: opaque sysroot Runtime `t_alloc_arr(Entry)`
        // bb3: opaque sysroot Runtime `write_bytes(Option<u32>)`
        // bb4: opaque sysroot Runtime `nv_grow_u8(Entry)`
        // bb5: `rcgjs.host.try(closure_capturing_Entry_and_Option)` -> Result<u32, JsValue>
        // bb6: `rcgjs.host.futurepromise(coro_saving_Entry)` with `poll_result` -> JsValue
        // bb7: internal offered `Callee::Foreign(rust_begin_unwind)` with `Option<u32>`
        // bb8: non-opaque sysroot `bstr_boundary(BoundaryEnum)` -> MUST protect BoundaryEnum
        // bb9: unresolved `Callee::Foreign(external_c_fn)` with `ForeignEnum` -> MUST protect ForeignEnum
        // bb10: Return
        let blocks = vec![
            BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Switch {
                    discriminant: Operand {
                        ty: t_u32,
                        kind: OperandKind::RuntimeCheck(false),
                    },
                    targets: vec![(1, BlockId(1))],
                    otherwise: BlockId(2),
                },
            },
            mk_call(
                1,
                Callee::Direct {
                    item: v1_item_id,
                    link_name: s_v1_name,
                },
                CallSemantic::Ordinary,
                None,
                vec![dummy_operand(t_entry)],
                t_unit,
                BlockId(10),
            ),
            mk_call(
                2,
                Callee::Runtime {
                    member: s_t_alloc_arr,
                    sysroot: true,
                },
                CallSemantic::Ordinary,
                None,
                vec![dummy_operand(t_entry)],
                t_entry,
                BlockId(3),
            ),
            mk_call(
                3,
                Callee::Runtime {
                    member: s_write_bytes,
                    sysroot: true,
                },
                CallSemantic::Ordinary,
                None,
                vec![dummy_operand(t_opt_u32)],
                t_unit,
                BlockId(4),
            ),
            mk_call(
                4,
                Callee::Runtime {
                    member: s_nv_grow_u8,
                    sysroot: true,
                },
                CallSemantic::Ordinary,
                None,
                vec![dummy_operand(t_entry)],
                t_unit,
                BlockId(5),
            ),
            mk_call(
                5,
                Callee::Direct {
                    item: closure_item_id,
                    link_name: s_host_try,
                },
                CallSemantic::Host,
                Some(HostPayload::Marker {
                    section: s_host_try,
                    def: DefinitionId {
                        crate_id: 2,
                        local_hash: 1,
                    },
                    helpers: Vec::new(),
                }),
                vec![dummy_operand(t_closure)],
                t_try_res,
                BlockId(6),
            ),
            mk_call(
                6,
                Callee::Direct {
                    item: poll_item_id,
                    link_name: s_host_fp,
                },
                CallSemantic::Host,
                Some(HostPayload::Marker {
                    section: s_host_fp,
                    def: DefinitionId {
                        crate_id: 2,
                        local_hash: 2,
                    },
                    helpers: vec![(
                        s_poll_result_key,
                        DirectItemRef {
                            item: poll_item_id,
                            link_name: s_poll_result_name,
                        },
                    )],
                }),
                vec![dummy_operand(t_coro)],
                t_js,
                BlockId(7),
            ),
            mk_call(
                7,
                Callee::Foreign {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 99,
                    },
                    symbol: s_panic_sym,
                },
                CallSemantic::Ordinary,
                None,
                vec![dummy_operand(t_opt_u32)],
                t_unit,
                BlockId(8),
            ),
            mk_call(
                8,
                Callee::Runtime {
                    member: s_bstr_boundary,
                    sysroot: true,
                },
                CallSemantic::Ordinary,
                None,
                vec![dummy_operand(t_boundary_enum)],
                t_unit,
                BlockId(9),
            ),
            mk_call(
                9,
                Callee::Foreign {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 100,
                    },
                    symbol: s_ext_sym,
                },
                CallSemantic::Ordinary,
                None,
                vec![dummy_operand(t_foreign_enum)],
                t_unit,
                BlockId(10),
            ),
            BasicBlock {
                id: BlockId(10),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Return,
            },
        ];

        let ret_block = vec![BasicBlock {
            id: BlockId(0),
            cleanup: false,
            statements: Vec::new(),
            terminator: Terminator::Return,
        }];

        let f_main = JsName::new("main_fn".to_owned());
        let f_closure = JsName::new("closure_body".to_owned());
        let f_closure_drop = JsName::new("closure_drop".to_owned());
        let f_poll = JsName::new("poll_result_fn".to_owned());
        let f_panic = JsName::new("rust_begin_unwind".to_owned());

        let mut table = BTreeMap::new();
        table.insert(
            f_main.clone(),
            custom_item(
                "main_fn",
                ItemId {
                    definition: DefinitionId {
                        crate_id: 1,
                        local_hash: 1,
                    },
                    instantiation: 0,
                },
                BodySemantic::Ordinary,
                shared.clone(),
                Vec::new(),
                t_unit,
                blocks,
            ),
        );
        table.insert(
            f_closure.clone(),
            custom_item(
                "closure_body",
                closure_item_id,
                BodySemantic::Ordinary,
                shared.clone(),
                vec![t_closure],
                t_u32,
                ret_block.clone(),
            ),
        );
        table.insert(
            f_closure_drop.clone(),
            custom_item(
                "closure_drop",
                ItemId {
                    definition: DefinitionId {
                        crate_id: 1,
                        local_hash: 504,
                    },
                    instantiation: 0,
                },
                BodySemantic::DropGlue(t_closure),
                shared.clone(),
                vec![t_closure],
                t_unit,
                ret_block.clone(),
            ),
        );
        table.insert(
            f_poll.clone(),
            custom_item(
                "poll_result_fn",
                poll_item_id,
                BodySemantic::Ordinary,
                shared.clone(),
                vec![t_ref_coro, t_js],
                t_poll_res,
                ret_block.clone(),
            ),
        );
        table.insert(
            f_panic.clone(),
            custom_item(
                "rust_begin_unwind",
                ItemId {
                    definition: DefinitionId {
                        crate_id: 1,
                        local_hash: 505,
                    },
                    instantiation: 0,
                },
                BodySemantic::Ordinary,
                shared,
                vec![t_opt_u32],
                t_unit,
                ret_block,
            ),
        );

        let reachable = vec![
            f_main.clone(),
            f_closure,
            f_closure_drop.clone(),
            f_poll.clone(),
            f_panic.clone(),
        ];
        let opts = Opts {
            optimizer: OptimizerMode::On,
            profile: Some(Profile::ReleaseSpeed),
            ..Opts::default()
        };
        let mut table_unspec = table.clone();
        let plan = specialize_table(&mut table, &reachable, &reachable, &BTreeSet::new(), &opts);

        assert_eq!(
            plan.decisions.get(&t_entry),
            Some(&EnumRepr::IntTag { fieldless: false }),
            "internal payload enum behind opaque runtime/host_try/futurepromise/dead-branch must get IntTag"
        );
        assert_eq!(
            plan.decisions.get(&t_opt_u32),
            Some(&EnumRepr::NullNiche {
                none_variant: 0,
                some_variant: 1
            }),
            "internal Option<u32> must get NullNiche"
        );
        assert!(
            !plan.decisions.contains_key(&t_try_res),
            "Result<u32, JsValue> returned by host_try must keep EnumRepr::Tag"
        );
        assert!(
            !plan.decisions.contains_key(&t_poll_res),
            "Poll<Result<u32, JsValue>> returned by poll_result must keep EnumRepr::Tag"
        );
        assert!(
            !plan.decisions.contains_key(&t_boundary_enum),
            "enum crossing bstr_boundary must keep EnumRepr::Tag"
        );
        assert!(
            !plan.decisions.contains_key(&t_foreign_enum),
            "enum crossing unresolved Callee::Foreign must keep EnumRepr::Tag"
        );

        // Negative check: if the closure body is NOT offered (v1 fallback), `rcgjs.host.try`
        // must conservatively exclude the closure's captured enums (`Entry` and `Option<u32>`).
        let offered_without_closure = vec![f_main, f_closure_drop, f_poll, f_panic];
        let fallback_plan = specialize_table(
            &mut table_unspec,
            &reachable,
            &offered_without_closure,
            &BTreeSet::new(),
            &opts,
        );
        assert!(
            !fallback_plan.decisions.contains_key(&t_entry),
            "non-offered closure body in host_try must conservatively protect captured Entry"
        );
        assert!(
            !fallback_plan.decisions.contains_key(&t_opt_u32),
            "non-offered closure body in host_try must conservatively protect captured Option<u32>"
        );
    }

    #[test]
    fn sentinel_niche_covers_option_jsvalue_nested_options_triple_fallback_and_chunks() {
        let mut interns = Interns::default();
        let s_none = interns.intern_str("None");
        let s_some = interns.intern_str("Some");
        let s_f0 = interns.intern_str("_0");
        let s_f1 = interns.intern_str("_1");

        let t_unit = TypeId(1);
        let t_u32 = TypeId(2);
        let t_js = TypeId(3);
        let t_opt_js = TypeId(4);
        let t_opt_opt_js = TypeId(5);
        let t_opt_u32 = TypeId(6);
        let t_opt_opt_u32 = TypeId(7);
        let t_opt_opt_opt_u32 = TypeId(8);
        let t_ref_opt_js = TypeId(9);
        let t_exported_opt_js = TypeId(10);
        let t_zst_prefix_opt = TypeId(11);

        let mut zst_prefix_opt = option_desc(t_zst_prefix_opt, t_u32, s_none, s_some, s_f1);
        if let TypeKind::Adt { variants, .. } = &mut zst_prefix_opt.kind {
            variants[1].fields = vec![
                FieldDescriptor {
                    key: FieldKey::Name(s_f0),
                    ty: t_unit,
                    offset: 0,
                },
                FieldDescriptor {
                    key: FieldKey::Name(s_f1),
                    ty: t_u32,
                    offset: 4,
                },
            ];
        }

        for desc in [
            unit_desc(t_unit),
            u32_desc(t_u32),
            js_value_desc(t_js),
            option_desc(t_opt_js, t_js, s_none, s_some, s_f0),
            option_desc(t_opt_opt_js, t_opt_js, s_none, s_some, s_f0),
            option_desc(t_opt_u32, t_u32, s_none, s_some, s_f0),
            option_desc(t_opt_opt_u32, t_opt_u32, s_none, s_some, s_f0),
            option_desc(t_opt_opt_opt_u32, t_opt_opt_u32, s_none, s_some, s_f0),
            ref_desc(t_ref_opt_js, t_opt_js, PointerForm::Thin),
            option_desc(t_exported_opt_js, t_js, s_none, s_some, s_f0),
            zst_prefix_opt,
        ] {
            interns.types.insert(desc);
        }
        let shared = Arc::new(interns);

        let f_int = JsName::new("internal_fn".to_owned());
        let f_exp = JsName::new("exported_fn".to_owned());
        let f_clash = JsName::new("$sn".to_owned());
        let mut table = BTreeMap::new();
        table.insert(
            f_int.clone(),
            dummy_item("internal_fn", shared.clone(), t_opt_opt_opt_u32, false),
        );
        table.insert(
            f_exp.clone(),
            dummy_item("exported_fn", shared.clone(), t_exported_opt_js, true),
        );
        let reachable = vec![f_int.clone(), f_exp.clone()];

        let spd_opts = Opts {
            optimizer: OptimizerMode::On,
            profile: Some(Profile::ReleaseSpeed),
            ..Opts::default()
        };
        let mut t1 = table.clone();
        let plan = specialize_table(&mut t1, &reachable, &reachable, &BTreeSet::new(), &spd_opts);

        assert_eq!(
            plan.decisions.get(&t_opt_js),
            Some(&EnumRepr::SentinelNiche {
                none_variant: 0,
                some_variant: 1,
                some_field: 0,
            }),
            "Option<JsValue> must use SentinelNiche (host JsValue cannot equal private $sn)"
        );
        assert_eq!(
            plan.decisions.get(&t_opt_opt_js),
            Some(&EnumRepr::IntTag { fieldless: false }),
            "Option<Option<JsValue>> must fall back to IntTag (inner already inhabits $sn)"
        );
        assert_eq!(
            plan.decisions.get(&t_opt_u32),
            Some(&EnumRepr::NullNiche {
                none_variant: 0,
                some_variant: 1,
            })
        );
        assert_eq!(
            plan.decisions.get(&t_opt_opt_u32),
            Some(&EnumRepr::SentinelNiche {
                none_variant: 0,
                some_variant: 1,
                some_field: 0,
            }),
            "Option<Option<u32>> must use SentinelNiche (inner NullNiche inhabits null, not $sn)"
        );
        assert_eq!(
            plan.decisions.get(&t_opt_opt_opt_u32),
            Some(&EnumRepr::IntTag { fieldless: false }),
            "Option<Option<Option<u32>>> must fall back to IntTag (inner inhabits both null and $sn)"
        );
        assert_eq!(
            plan.decisions.get(&t_zst_prefix_opt),
            Some(&EnumRepr::SentinelNiche {
                none_variant: 0,
                some_variant: 1,
                some_field: 1,
            }),
            "2-variant enum with leading ZST field must use SentinelNiche at some_field=1"
        );
        assert!(
            !plan.decisions.contains_key(&t_exported_opt_js),
            "exported Option<JsValue> must stay baseline Tag"
        );
        assert_eq!(plan.sentinel_none.as_ref().map(JsName::as_str), Some("$sn"));
        let sent_items = plan.sentinel_items();
        assert_eq!(sent_items.len(), 1);
        assert_eq!(sent_items[0].name.as_str(), "$sn");
        assert!(sent_items[0].js.contains("const $sn = {};"));

        let updated = &t1[&f_int].jir.interns().unwrap().types;
        assert_eq!(updated.sentinel_none(), Some("$sn"));
        assert!(!updated.is_indirect(t_opt_js));
        assert!(!updated.needs_clone(t_opt_js));
        assert_eq!(
            updated.get(t_ref_opt_js).unwrap().js.pointer_form,
            Some(PointerForm::Slot),
            "&Option<JsValue> must cascade Thin -> Slot"
        );
        assert_eq!(
            updated.field(t_zst_prefix_opt, Some(1), 1).unwrap().key,
            FieldKey::Transparent,
            "SentinelNiche some_field projection must be FieldKey::Transparent"
        );

        // Collision avoidance when `$sn` is already taken in the program.
        let mut t_clash = table.clone();
        t_clash.insert(f_clash.clone(), dummy_item("$sn", shared, t_unit, false));
        let plan_clash = specialize_table(
            &mut t_clash,
            &[f_int.clone(), f_exp, f_clash],
            &reachable,
            &BTreeSet::new(),
            &spd_opts,
        );
        assert_eq!(
            plan_clash.sentinel_none.as_ref().map(JsName::as_str),
            Some("$sn$1")
        );

        // Chunked mode must disable SentinelNiche cleanly (falls back to IntTag, no sentinel item).
        let mut chunked_opts = spd_opts.clone();
        chunked_opts
            .chunks
            .push(("extra".to_owned(), "exported_fn".to_owned()));
        let mut t_chunked = table.clone();
        let plan_chunked = specialize_table(
            &mut t_chunked,
            &reachable,
            &reachable,
            &BTreeSet::new(),
            &chunked_opts,
        );
        assert_eq!(
            plan_chunked.decisions.get(&t_opt_js),
            Some(&EnumRepr::IntTag { fieldless: false })
        );
        assert_eq!(
            plan_chunked.decisions.get(&t_opt_opt_u32),
            Some(&EnumRepr::IntTag { fieldless: false })
        );
        assert!(plan_chunked.sentinel_none.is_none());
        assert!(plan_chunked.sentinel_items().is_empty());

        // Attribute-discovered chunks in ESM mode must also disable SentinelNiche.
        let mut discovered_opts = spd_opts;
        discovered_opts.modules = crate::opts::Modules::Esm;
        let mut t_discovered = table;
        t_discovered
            .get_mut(&JsName::new("exported_fn".to_owned()))
            .unwrap()
            .linkage = crate::item::Linkage::exported("exported_fn".to_owned()).with_chunk(
            "island".to_owned(),
            rcgjs_object::artifact::EntryRole::Island,
        );
        let plan_discovered = specialize_table(
            &mut t_discovered,
            &reachable,
            &reachable,
            &BTreeSet::new(),
            &discovered_opts,
        );
        assert_eq!(
            plan_discovered.decisions.get(&t_opt_js),
            Some(&EnumRepr::IntTag { fieldless: false })
        );
        assert!(plan_discovered.sentinel_none.is_none());
        assert!(plan_discovered.sentinel_items().is_empty());
    }
}
