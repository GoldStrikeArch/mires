//! Whole-program Structure-of-Arrays (SoA) planner for non-escaping `Vec<FlatStruct, Global>`.
//!
//! When `Profile::ReleaseSpeed`, `OptimizerMode::On`, and universal native vectors
//! (`HeapPlan::native_vec_enabled`) are active, this pass identifies flat primitive-scalar struct
//! and tuple element types `T` whose `Vec<T, Global>` allocations, derived `&[T]`/`&mut [T]`
//! slices, and element references `&T`/`&mut T`/`*const T`/`*mut T` never escape into host/FFI
//! boundaries, exports, statics, raw-memory casts, `Box`/`Rc`/`Arc`, non-SoA stack references, or
//! unsupported operations.
//!
//! Selected element types keep their canonical JS object/tuple shape for by-value `T`, while their
//! `Vec<T, Global>` becomes `{ buf: { f0: TA0, f1: TA1, ... }, len }`, slices stay `{ buf, off,
//! len }`, and all element references/pointers uniformly share `{ buf, off }`. This keeps
//! `abi_flat` slice flattening, `value::clone_expr_with`, and `loop_opt` column-array LICM working
//! seamlessly with zero per-element JS object allocations.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use crate::analysis::heap::{
    filter_live_for_repr, global_vec_elem_ty, is_direct_global_vec_ty, HeapPlan,
};
use crate::item::{ItemKind, JsName, LinkItem};
use crate::jir::decline::Decline;
use crate::jir::interns::{Interns, StrId};
use crate::jir::repr::{Carrier, Func, Op, OpKind, Proj, Root, ValueId};
use crate::jir::semantic::{
    Body, BodyKind, BodySemantic, CallSemantic, Callee, CastKind, ConstTree, DropGlue,
    GlobalReference, Operand, OperandKind, Place, Projection, Rvalue, StandardOperation,
    StatementKind, Terminator,
};
use crate::jir::types::{
    AllocatorIdentity, FieldKey, IteratorIdentity, NominalAbi, NominalForm, PinnedBoundary,
    RepresentationCandidate, TypeDescriptor, TypeId, TypeKind, TypeTable,
};
use crate::lower::js::value;
use crate::opts::{OptimizerMode, Opts, Profile};
use crate::plan::names::NamePool;

/// Minimum number of non-ZST scalar fields for an element type to benefit from SoA.
const MIN_SOA_FIELDS: usize = 2;
/// Maximum number of non-ZST scalar fields for an element type to use SoA.
const MAX_SOA_FIELDS: usize = 12;

fn soa_debug() -> bool {
    static DEBUG: OnceLock<bool> = OnceLock::new();
    *DEBUG.get_or_init(|| std::env::var_os("RCGJS_SOA_DEBUG").is_some())
}

/// TypedArray column storage kind for one scalar field of an SoA element type.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum SoaScalar {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    /// `u32` struct field in `HeapPlan::smi_u32_fields`, stored in signed `Int32Array` so reads
    /// and writes stay strictly aligned with `is_strict_i32_u32` (`-1` vs `=== -1`).
    SmiU32,
    F32,
    F64,
    /// `bool` stored in `Uint8Array` (`0` / `1`), decoded on read as `col[off] !== 0` and encoded
    /// on write as `val ? 1 : 0`.
    Bool,
}

impl SoaScalar {
    /// JavaScript `TypedArray` constructor name backing this column.
    pub const fn ta_ctor(self) -> &'static str {
        match self {
            SoaScalar::U8 | SoaScalar::Bool => "Uint8Array",
            SoaScalar::I8 => "Int8Array",
            SoaScalar::U16 => "Uint16Array",
            SoaScalar::I16 => "Int16Array",
            SoaScalar::U32 => "Uint32Array",
            SoaScalar::I32 | SoaScalar::SmiU32 => "Int32Array",
            SoaScalar::F32 => "Float32Array",
            SoaScalar::F64 => "Float64Array",
        }
    }

    pub const fn is_bool(self) -> bool {
        matches!(self, SoaScalar::Bool)
    }
}

/// One non-ZST scalar column of an SoA element type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SoaField {
    /// Declaration field index in the Rust struct or tuple.
    pub field_idx: u32,
    /// Property name on the SoA columns object (`"f0"`, `"f1"`, ...).
    pub col_name: String,
    /// Canonical JS property key used when materializing/destructuring by-value `T`.
    pub js_key: FieldKey,
    /// Resolved string key name for struct fields (`FieldKey::Name` or `FieldKey::Index`).
    pub js_key_name: Option<String>,
    /// Rust field type.
    pub ty: TypeId,
    /// Column storage representation.
    pub scalar: SoaScalar,
}

/// Selected SoA column layout and linker helper names for one element type `T`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SoaLayout {
    pub elem_ty: TypeId,
    pub is_tuple: bool,
    pub total_fields: usize,
    pub fields: Vec<SoaField>,
    pub field_to_col: BTreeMap<u32, usize>,
    pub elem_size: u64,
    pub min_cap: u64,
    pub max_elems: u64,
    pub id: u32,
    pub empty_name: JsName,
    pub alloc_name: JsName,
    pub zeroed_name: JsName,
    pub grow_name: JsName,
    pub reserve_name: JsName,
    pub try_reserve_name: JsName,
    pub shrink_name: JsName,
    pub copy_name: JsName,
}

impl SoaLayout {
    /// Looks up the SoA column for Rust declaration field `field_idx`.
    pub fn field(&self, field_idx: u32) -> Option<&SoaField> {
        let &col = self.field_to_col.get(&field_idx)?;
        self.fields.get(col)
    }

    /// All linker-synthesized helper names owned by this layout.
    pub fn helper_names(&self) -> [&JsName; 8] {
        [
            &self.empty_name,
            &self.alloc_name,
            &self.zeroed_name,
            &self.grow_name,
            &self.reserve_name,
            &self.try_reserve_name,
            &self.shrink_name,
            &self.copy_name,
        ]
    }
}

/// Whole-program SoA transformation plan.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SoaPlan {
    pub layouts: BTreeMap<TypeId, SoaLayout>,
}

impl SoaPlan {
    pub fn is_empty(&self) -> bool {
        self.layouts.is_empty()
    }

    pub fn get(&self, table: &TypeTable, ty: TypeId) -> Option<&SoaLayout> {
        if self.layouts.is_empty() {
            return None;
        }
        self.layouts.get(&table.peeled(ty))
    }

    pub fn all_helper_names(&self) -> Vec<JsName> {
        let mut out = Vec::with_capacity(self.layouts.len() * 8);
        for layout in self.layouts.values() {
            for name in layout.helper_names() {
                out.push(name.clone());
            }
        }
        out
    }

    /// Whether `item` may mention any selected SoA element type (or lacks metadata to prove
    /// otherwise).
    pub fn item_may_touch_soa(&self, item: &LinkItem) -> bool {
        if self.layouts.is_empty() {
            return false;
        }
        let Some(interns) = item.jir.interns() else {
            return true;
        };
        if let Some(body) = &item.jir.body {
            let mut touched = false;
            for ty in body_mentioned_types(body) {
                interns.types.visit_types(
                    ty,
                    rcgjs_object::jir::types::TypeWalk::WithAdtArgs,
                    &mut |current| {
                        if !touched && self.layouts.contains_key(&interns.types.peeled(current)) {
                            touched = true;
                        }
                    },
                );
                if touched {
                    break;
                }
            }
            return touched;
        }
        self.layouts
            .keys()
            .any(|ty| interns.types.get(*ty).is_some())
    }
}

/// Structural shape extracted from one `TypeTable` before helper-name assignment.
#[derive(Clone, Debug, Eq, PartialEq)]
struct CandidateShape {
    elem_ty: TypeId,
    is_tuple: bool,
    total_fields: usize,
    fields: Vec<SoaField>,
    elem_size: u64,
}

#[derive(Default)]
struct CandidateSet {
    candidates: BTreeMap<TypeId, CandidateShape>,
    disqualified: BTreeSet<TypeId>,
}

impl CandidateSet {
    fn disqualify(&mut self, ty: TypeId) {
        self.disqualified.insert(ty);
        self.candidates.remove(&ty);
    }

    fn disqualify_body_mentions(
        &mut self,
        name: &JsName,
        body: &Body,
        types: &TypeTable,
        reason: &str,
    ) {
        let mut mentioned = BTreeSet::new();
        for ty in body_mentioned_types(body) {
            collect_any_soa_mentions(types, ty, &self.candidates, &mut mentioned);
        }
        for ty in mentioned {
            if soa_debug() {
                eprintln!("[soa] disqualify {:?} ({reason} in {name})", ty);
            }
            self.disqualify(ty);
        }
    }

    fn disqualify_func_mentions(
        &mut self,
        name: &JsName,
        func: &Func,
        types: &TypeTable,
        reason: &str,
    ) {
        let mut mentioned = BTreeSet::new();
        for ty in func_referenced_types(func) {
            collect_any_soa_mentions(types, ty, &self.candidates, &mut mentioned);
        }
        for ty in mentioned {
            if soa_debug() {
                eprintln!("[soa] disqualify {:?} ({reason} in {name})", ty);
            }
            self.disqualify(ty);
        }
    }

    fn disqualify_non_jir_item(
        &mut self,
        name: &JsName,
        item: &LinkItem,
        funcs: &BTreeMap<JsName, Result<Func, Decline>>,
        reason: &str,
    ) {
        if let (Some(body), Some(interns)) = (&item.jir.body, item.jir.interns()) {
            self.disqualify_body_mentions(name, body, &interns.types, reason);
        } else if let (Some(Ok(func)), Some(interns)) = (funcs.get(name), item.jir.interns()) {
            self.disqualify_func_mentions(name, func, &interns.types, reason);
        } else {
            if soa_debug() {
                eprintln!("[soa] CLEAR ALL ({reason} {name} lacks body/func metadata)");
            }
            self.candidates.clear();
        }
    }
}

/// Computes the whole-program fail-closed SoA plan over `table` and post-rewrite `funcs`.
pub fn plan<V>(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    repr_bodies: &BTreeMap<JsName, V>,
    live: &BTreeSet<JsName>,
    opts: &Opts,
    heap: &HeapPlan,
) -> SoaPlan {
    let debug = soa_debug();
    if opts.optimizer != OptimizerMode::On
        || opts.profile != Some(Profile::ReleaseSpeed)
        || !heap.native_vec_enabled
    {
        if debug {
            eprintln!(
                "[soa] disabled: opt={:?} profile={:?} native_vec={}",
                opts.optimizer, opts.profile, heap.native_vec_enabled
            );
        }
        return SoaPlan::default();
    }

    let effective_live = filter_live_for_repr(table, funcs, live, opts, heap);

    let mut state = discover_candidates(table, funcs, repr_bodies, &effective_live, heap);
    if state.candidates.is_empty() {
        if debug {
            eprintln!(
                "[soa] no candidates discovered (effective_live={})",
                effective_live.len()
            );
        }
        return SoaPlan::default();
    }

    audit_live_items(
        table,
        funcs,
        repr_bodies,
        &effective_live,
        opts,
        heap,
        &mut state,
    );
    if state.candidates.is_empty() {
        if debug {
            eprintln!("[soa] 0 candidates survived Phase 2 audit");
        }
        return SoaPlan::default();
    }

    assign_soa_layouts(table, state.candidates)
}

/// Phase 1: discover every structurally eligible `T` appearing as `Vec<T, Global>` in an
/// effective-live JIR function.
fn discover_candidates<V>(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    repr_bodies: &BTreeMap<JsName, V>,
    effective_live: &BTreeSet<JsName>,
    heap: &HeapPlan,
) -> CandidateSet {
    let debug = soa_debug();
    let mut state = CandidateSet::default();

    for name in effective_live {
        let Some(item) = table.get(name) else {
            continue;
        };
        if repr_bodies.contains_key(name) || item.kind != ItemKind::Fn {
            continue;
        }
        let Some(Ok(func)) = funcs.get(name) else {
            continue;
        };
        let Some(interns) = item.jir.interns() else {
            continue;
        };
        if is_intercepted_native_vec_body(item) {
            continue;
        }
        let types = &interns.types;
        for ty in func_referenced_types(func) {
            if !is_direct_global_vec_ty(types, ty) {
                continue;
            }
            let Some(raw_elem) = global_vec_elem_ty(types, ty) else {
                continue;
            };
            let elem_ty = types.peeled(raw_elem);
            if state.disqualified.contains(&elem_ty) {
                continue;
            }
            match extract_candidate_shape(types, interns, elem_ty, heap) {
                Some(shape) => {
                    if let Some(existing) = state.candidates.get(&elem_ty) {
                        if existing != &shape {
                            if debug {
                                eprintln!(
                                    "[soa] disqualify {:?} (shape mismatch in {})",
                                    elem_ty, name
                                );
                            }
                            state.disqualify(elem_ty);
                        }
                    } else {
                        if debug {
                            let fields_dbg: Vec<String> = shape
                                .fields
                                .iter()
                                .map(|f| {
                                    format!(
                                        "{}:{:?}",
                                        f.js_key_name.as_deref().unwrap_or(&f.col_name),
                                        f.scalar
                                    )
                                })
                                .collect();
                            eprintln!(
                                "[soa] candidate {:?} from {} ({} fields: [{}])",
                                elem_ty,
                                name,
                                shape.fields.len(),
                                fields_dbg.join(", ")
                            );
                        }
                        state.candidates.insert(elem_ty, shape);
                    }
                }
                None => state.disqualify(elem_ty),
            }
        }
    }

    state
}

/// Phase 2: whole-program fail-closed audit across every effective-live item.
fn audit_live_items<V>(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    repr_bodies: &BTreeMap<JsName, V>,
    effective_live: &BTreeSet<JsName>,
    opts: &Opts,
    heap: &HeapPlan,
    state: &mut CandidateSet,
) {
    let debug = soa_debug();
    for name in effective_live {
        if state.candidates.is_empty() {
            break;
        }
        let Some(item) = table.get(name) else {
            continue;
        };

        if repr_bodies.contains_key(name) {
            state.disqualify_non_jir_item(name, item, funcs, "repr_body");
            continue;
        }

        match item.kind {
            ItemKind::Alloc | ItemKind::Import | ItemKind::Const => {
                if let (Some(body), Some(interns)) = (&item.jir.body, item.jir.interns()) {
                    state.disqualify_body_mentions(name, body, &interns.types, "non_fn_body");
                }
                continue;
            }
            ItemKind::Static | ItemKind::Vtable => {
                if let (Some(body), Some(interns)) = (&item.jir.body, item.jir.interns()) {
                    let reason = if item.kind == ItemKind::Static {
                        "static"
                    } else {
                        "vtable"
                    };
                    state.disqualify_body_mentions(name, body, &interns.types, reason);
                } else if !is_known_synthesized_static(name.as_str(), item) {
                    if debug {
                        eprintln!(
                            "[soa] CLEAR ALL: {:?} item {} has no JIR body",
                            item.kind, name
                        );
                    }
                    state.candidates.clear();
                    break;
                }
                continue;
            }
            ItemKind::Fn => {}
        }

        let Some(Ok(func)) = funcs.get(name) else {
            if let (Some(body), Some(interns)) = (&item.jir.body, item.jir.interns()) {
                state.disqualify_body_mentions(name, body, &interns.types, "declined_fn");
            } else if !is_known_synthesized_fn(name.as_str()) {
                if debug {
                    eprintln!("[soa] CLEAR ALL: non-JIR fn {} has no JIR body", name);
                }
                state.candidates.clear();
                break;
            }
            continue;
        };

        let Some(interns) = item.jir.interns() else {
            if debug {
                eprintln!("[soa] CLEAR ALL: fn {} missing interns", name);
            }
            state.candidates.clear();
            break;
        };
        let types = &interns.types;

        // Consistency check across every live item whose TypeTable defines a candidate `elem_ty`.
        let mismatch: Vec<TypeId> = state
            .candidates
            .iter()
            .filter_map(|(&elem_ty, expected)| {
                types.get(elem_ty)?;
                let got = extract_candidate_shape(types, interns, elem_ty, heap);
                (got.as_ref() != Some(expected)).then_some(elem_ty)
            })
            .collect();
        for ty in mismatch {
            if debug {
                eprintln!(
                    "[soa] disqualify {:?} (cross-crate shape mismatch in {})",
                    ty, name
                );
            }
            state.disqualify(ty);
        }
        if state.candidates.is_empty() {
            break;
        }

        // Export / root / async export-out boundary check: no SoA type or container may cross.
        if item.linkage.exported
            || opts.roots.iter().any(|r| r == name.as_str())
            || matches!(
                item.jir.body.as_ref().map(|b| b.semantic),
                Some(BodySemantic::JsExportOut)
            )
        {
            let mut mentioned = BTreeSet::new();
            for &arg_ty in &func.sig.arguments {
                collect_any_soa_mentions(types, arg_ty, &state.candidates, &mut mentioned);
            }
            collect_any_soa_mentions(types, func.sig.result, &state.candidates, &mut mentioned);
            for ty in mentioned {
                if debug {
                    eprintln!("[soa] disqualify {:?} (export/root ABI of {})", ty, name);
                }
                state.disqualify(ty);
            }
        }

        if let Some(BodySemantic::Standard(
            StandardOperation::SliceSort | StandardOperation::SliceSortCmp,
        )) = item.jir.body.as_ref().map(|b| b.semantic)
        {
            state.disqualify_func_mentions(name, func, types, "slice_sort_body");
            continue;
        }

        SoaFuncAuditor {
            fn_name: name,
            func,
            types,
            interns,
            table,
            funcs,
            repr_bodies,
            state,
        }
        .audit();
    }
}

/// Phase 3: allocate collision-free helper names via [`NamePool`] and build [`SoaPlan`].
fn assign_soa_layouts(
    table: &BTreeMap<JsName, LinkItem>,
    candidates: BTreeMap<TypeId, CandidateShape>,
) -> SoaPlan {
    if soa_debug() {
        for (ty, shape) in &candidates {
            let fields_dbg: Vec<String> = shape
                .fields
                .iter()
                .map(|f| {
                    format!(
                        "{}:{:?}",
                        f.js_key_name.as_deref().unwrap_or(&f.col_name),
                        f.scalar
                    )
                })
                .collect();
            eprintln!("[soa] SELECTED {:?}: [{}]", ty, fields_dbg.join(", "));
        }
    }

    let prefix = NamePool::from_table(table).alloc_unused_prefix("$__mires_soa_");
    let mut layouts = BTreeMap::new();
    for (idx, (elem_ty, shape)) in candidates.into_iter().enumerate() {
        let id = idx as u32;
        let min_cap: u64 = if shape.elem_size == 1 {
            8
        } else if shape.elem_size <= 1024 {
            4
        } else {
            1
        };
        let max_elems: u64 = 2_147_483_647u64
            .checked_div(shape.elem_size.max(1))
            .unwrap_or(4_294_967_295);
        let mut field_to_col = BTreeMap::new();
        for (col_idx, field) in shape.fields.iter().enumerate() {
            field_to_col.insert(field.field_idx, col_idx);
        }
        let layout = SoaLayout {
            elem_ty,
            is_tuple: shape.is_tuple,
            total_fields: shape.total_fields,
            fields: shape.fields,
            field_to_col,
            elem_size: shape.elem_size,
            min_cap,
            max_elems,
            id,
            empty_name: JsName::new(format!("{prefix}e{id}")),
            alloc_name: JsName::new(format!("{prefix}alloc_{id}")),
            zeroed_name: JsName::new(format!("{prefix}zeroed_{id}")),
            grow_name: JsName::new(format!("{prefix}grow_{id}")),
            reserve_name: JsName::new(format!("{prefix}res_{id}")),
            try_reserve_name: JsName::new(format!("{prefix}try_res_{id}")),
            shrink_name: JsName::new(format!("{prefix}shrink_{id}")),
            copy_name: JsName::new(format!("{prefix}copy_{id}")),
        };
        layouts.insert(elem_ty, layout);
    }

    SoaPlan { layouts }
}

fn is_known_synthesized_fn(name: &str) -> bool {
    crate::lower::js::i64_pair::HELPER_NAMES.contains(&name)
}

fn is_known_synthesized_static(name: &str, item: &LinkItem) -> bool {
    crate::lower::js::i64_pair::HELPER_NAMES.contains(&name)
        || (item.jir.summary.is_none() && item.jir.interns().is_none())
}

fn is_intercepted_native_vec_body(item: &LinkItem) -> bool {
    matches!(
        item.jir.body.as_ref().map(|b| b.semantic),
        Some(BodySemantic::Standard(op)) if op.is_native_vec()
    )
}

fn extract_candidate_shape(
    types: &TypeTable,
    interns: &Interns,
    elem_ty: TypeId,
    heap: &HeapPlan,
) -> Option<CandidateShape> {
    let desc = types.get(elem_ty)?;
    if !desc.layout.sized
        || desc.layout.zst
        || desc.layout.needs_drop
        || desc.layout.has_own_drop
        || !desc.js.is_indirect
        || desc.js.unsized_tail.is_some()
        || desc.js.transparent.is_some()
        || desc.js.crosses_transparent_union
        || desc.meta.candidate != RepresentationCandidate::None
        || desc.meta.iterator != IteratorIdentity::None
        || desc.meta.boundary != PinnedBoundary::None
        || desc.meta.abi != NominalAbi::Rust
    {
        return None;
    }
    let elem_size = desc.layout.size?;
    if elem_size == 0 {
        return None;
    }

    let (is_tuple, raw_fields, total_fields) = match &desc.kind {
        TypeKind::Adt {
            form: NominalForm::Struct,
            variants,
            ..
        } => {
            let [variant] = variants.as_slice() else {
                return None;
            };
            (false, variant.fields.as_slice(), variant.fields.len())
        }
        TypeKind::Tuple(elems) => (true, desc.js.fields.as_slice(), elems.len()),
        _ => return None,
    };

    if is_tuple && raw_fields.len() != total_fields {
        return None;
    }

    let mut fields = Vec::new();
    for (fi, field) in raw_fields.iter().enumerate() {
        if types.is_zst(field.ty) {
            continue;
        }
        let f_desc = types.get(field.ty)?;
        let scalar = classify_raw_scalar(f_desc, elem_ty, fi as u32, heap)?;
        let js_key_name = if is_tuple {
            None
        } else {
            let key_name = match field.key {
                FieldKey::Name(id) => interns.str(id)?.to_owned(),
                FieldKey::Index(i) => format!("_{i}"),
                FieldKey::Transparent => return None,
            };
            Some(key_name)
        };
        let col_idx = fields.len();
        fields.push(SoaField {
            field_idx: fi as u32,
            col_name: format!("f{col_idx}"),
            js_key: field.key,
            js_key_name,
            ty: field.ty,
            scalar,
        });
    }

    if !(MIN_SOA_FIELDS..=MAX_SOA_FIELDS).contains(&fields.len()) {
        return None;
    }

    Some(CandidateShape {
        elem_ty,
        is_tuple,
        total_fields,
        fields,
        elem_size,
    })
}

fn classify_raw_scalar(
    desc: &TypeDescriptor,
    owner_elem_ty: TypeId,
    field_idx: u32,
    heap: &HeapPlan,
) -> Option<SoaScalar> {
    if !desc.layout.sized
        || desc.layout.zst
        || desc.layout.needs_drop
        || desc.layout.has_own_drop
        || desc.js.transparent.is_some()
        || desc.js.crosses_transparent_union
        || desc.meta.boundary != PinnedBoundary::None
        || desc.meta.candidate != RepresentationCandidate::None
    {
        return None;
    }
    match desc.kind {
        TypeKind::Float { bits: 64 } => Some(SoaScalar::F64),
        TypeKind::Float { bits: 32 } => Some(SoaScalar::F32),
        TypeKind::Integer {
            signed: true,
            bits: 32,
        } => Some(SoaScalar::I32),
        TypeKind::Integer {
            signed: false,
            bits: 32,
        } => {
            if heap.smi_u32_fields.contains(&(owner_elem_ty, field_idx)) {
                Some(SoaScalar::SmiU32)
            } else {
                Some(SoaScalar::U32)
            }
        }
        TypeKind::Integer {
            signed: true,
            bits: 16,
        } => Some(SoaScalar::I16),
        TypeKind::Integer {
            signed: false,
            bits: 16,
        } => Some(SoaScalar::U16),
        TypeKind::Integer {
            signed: true,
            bits: 8,
        } => Some(SoaScalar::I8),
        TypeKind::Integer {
            signed: false,
            bits: 8,
        } => Some(SoaScalar::U8),
        TypeKind::Bool => Some(SoaScalar::Bool),
        _ => None,
    }
}

fn collect_operand_types(op: &Operand, out: &mut BTreeSet<TypeId>) {
    out.insert(op.ty);
    match &op.kind {
        OperandKind::Copy(place) | OperandKind::Move(place) => collect_place_types(place, out),
        OperandKind::Constant(_) | OperandKind::RuntimeCheck(_) => {}
    }
}

fn collect_place_types(place: &Place, out: &mut BTreeSet<TypeId>) {
    out.insert(place.ty);
    for proj in &place.projection {
        match proj {
            Projection::Field { ty, .. }
            | Projection::OpaqueCast(ty)
            | Projection::UnwrapUnsafeBinder(ty) => {
                out.insert(*ty);
            }
            _ => {}
        }
    }
}

/// Collects every [`TypeId`] referenced directly by a single JIR-S [`Body`].
fn body_mentioned_types(body: &Body) -> BTreeSet<TypeId> {
    let mut out = BTreeSet::new();
    for &arg in &body.signature.arguments {
        out.insert(arg);
    }
    out.insert(body.signature.result);
    if let BodyKind::Vtable { concrete, .. } = &body.kind {
        out.insert(*concrete);
    }
    for local in &body.locals {
        out.insert(local.ty);
    }
    for block in &body.blocks {
        for stmt in &block.statements {
            match &stmt.kind {
                StatementKind::Assign { destination, value } => {
                    collect_place_types(destination, &mut out);
                    match value {
                        Rvalue::Use(op)
                        | Rvalue::Unary { value: op, .. }
                        | Rvalue::WrapUnsafeBinder(op) => {
                            collect_operand_types(op, &mut out);
                        }
                        Rvalue::Repeat { value, .. } => collect_operand_types(value, &mut out),
                        Rvalue::Reference { place, .. }
                        | Rvalue::RawPointer { place, .. }
                        | Rvalue::Discriminant(place)
                        | Rvalue::CopyForDeref(place)
                        | Rvalue::Reborrow { place, .. } => collect_place_types(place, &mut out),
                        Rvalue::Cast { value, result, .. } => {
                            collect_operand_types(value, &mut out);
                            out.insert(*result);
                        }
                        Rvalue::Binary { left, right, .. } => {
                            collect_operand_types(left, &mut out);
                            collect_operand_types(right, &mut out);
                        }
                        Rvalue::Aggregate {
                            result, operands, ..
                        } => {
                            out.insert(*result);
                            for op in operands {
                                collect_operand_types(op, &mut out);
                            }
                        }
                    }
                }
                StatementKind::SetDiscriminant { place, .. } => {
                    collect_place_types(place, &mut out)
                }
                StatementKind::Assume(op) => collect_operand_types(op, &mut out),
                StatementKind::CopyNonOverlapping {
                    source,
                    destination,
                    count,
                } => {
                    collect_operand_types(source, &mut out);
                    collect_operand_types(destination, &mut out);
                    collect_operand_types(count, &mut out);
                }
                StatementKind::StorageLive(_)
                | StatementKind::StorageDead(_)
                | StatementKind::Nop => {}
            }
        }
        match &block.terminator {
            Terminator::Call(call) => {
                collect_operand_types(&call.function, &mut out);
                for arg in &call.arguments {
                    collect_operand_types(arg, &mut out);
                }
                collect_place_types(&call.destination, &mut out);
                if let Callee::Intrinsic { types, .. } = &call.callee {
                    out.extend(types.iter().copied());
                }
            }
            Terminator::Drop { place, .. } => collect_place_types(place, &mut out),
            Terminator::Switch { discriminant, .. } => {
                collect_operand_types(discriminant, &mut out)
            }
            Terminator::Assert { condition, .. } => collect_operand_types(condition, &mut out),
            _ => {}
        }
    }
    out
}

fn func_referenced_types(func: &Func) -> BTreeSet<TypeId> {
    let mut out = BTreeSet::new();
    for &arg in &func.sig.arguments {
        out.insert(arg);
    }
    out.insert(func.sig.result);
    for v in &func.values {
        out.insert(v.ty);
    }
    for c in &func.cells {
        out.insert(c.ty);
    }
    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::Aggregate { ty, .. } => {
                    out.insert(*ty);
                }
                OpKind::Cast { from, to, .. } => {
                    out.insert(*from);
                    out.insert(*to);
                }
                OpKind::Intrinsic { types, .. } => {
                    out.extend(types.iter().copied());
                }
                _ => {}
            }
        }
    }
    out
}

fn is_soa_elem(
    types: &TypeTable,
    ty: TypeId,
    candidates: &BTreeMap<TypeId, CandidateShape>,
) -> Option<TypeId> {
    let p = types.peeled(ty);
    candidates.contains_key(&p).then_some(p)
}

fn is_soa_vec(
    types: &TypeTable,
    ty: TypeId,
    candidates: &BTreeMap<TypeId, CandidateShape>,
) -> Option<TypeId> {
    if !is_direct_global_vec_ty(types, ty) {
        return None;
    }
    let elem = global_vec_elem_ty(types, ty)?;
    is_soa_elem(types, elem, candidates)
}

fn is_soa_slice(
    types: &TypeTable,
    ty: TypeId,
    candidates: &BTreeMap<TypeId, CandidateShape>,
) -> Option<TypeId> {
    match value::kind(types, types.peeled(ty)) {
        Some(TypeKind::Slice(elem)) => is_soa_elem(types, *elem, candidates),
        _ => None,
    }
}

fn is_soa_elem_ptr(
    types: &TypeTable,
    ty: TypeId,
    candidates: &BTreeMap<TypeId, CandidateShape>,
) -> Option<TypeId> {
    let pointee = value::pointee_of(types, ty)?;
    is_soa_elem(types, pointee, candidates)
}

fn is_soa_slice_ptr(
    types: &TypeTable,
    ty: TypeId,
    candidates: &BTreeMap<TypeId, CandidateShape>,
) -> Option<TypeId> {
    let pointee = value::pointee_of(types, ty)?;
    is_soa_slice(types, pointee, candidates)
}

/// Collects every candidate `T` reachable anywhere in `ty` (by value, pointer, slice, or container).
fn collect_any_soa_mentions(
    types: &TypeTable,
    ty: TypeId,
    candidates: &BTreeMap<TypeId, CandidateShape>,
    out: &mut BTreeSet<TypeId>,
) {
    types.visit_types(
        ty,
        rcgjs_object::jir::types::TypeWalk::WithAdtArgs,
        &mut |current| {
            let peeled = types.peeled(current);
            if candidates.contains_key(&peeled) {
                out.insert(peeled);
            }
        },
    );
}

/// Collects every candidate `T` appearing in `ty` through an SoA-represented form (`Vec<T>`,
/// `[T]`, `&T`, `&mut T`, `*const T`, `*mut T`), excluding standalone by-value `T`.
fn collect_soa_container_or_ptr_mentions(
    types: &TypeTable,
    ty: TypeId,
    candidates: &BTreeMap<TypeId, CandidateShape>,
    out: &mut BTreeSet<TypeId>,
) {
    let mut visited = BTreeSet::new();
    collect_soa_container_or_ptr_rec(types, ty, candidates, &mut visited, out);
}

fn collect_soa_container_or_ptr_rec(
    types: &TypeTable,
    ty: TypeId,
    candidates: &BTreeMap<TypeId, CandidateShape>,
    visited: &mut BTreeSet<TypeId>,
    out: &mut BTreeSet<TypeId>,
) {
    if !visited.insert(ty) {
        return;
    }
    if let Some(t) = is_soa_vec(types, ty, candidates)
        .or_else(|| is_soa_slice(types, ty, candidates))
        .or_else(|| is_soa_elem_ptr(types, ty, candidates))
        .or_else(|| is_soa_slice_ptr(types, ty, candidates))
    {
        out.insert(t);
        return;
    }
    let Some(desc) = types.get(ty) else {
        return;
    };
    if let Some(inner) = desc.js.transparent {
        collect_soa_container_or_ptr_rec(types, inner.ty, candidates, visited, out);
    }
    match &desc.kind {
        TypeKind::Reference { pointee, .. }
        | TypeKind::RawPointer { pointee, .. }
        | TypeKind::Slice(pointee)
        | TypeKind::Array {
            element: pointee, ..
        }
        | TypeKind::Pattern(pointee) => {
            collect_soa_container_or_ptr_rec(types, *pointee, candidates, visited, out);
        }
        TypeKind::Tuple(elems) => {
            for &e in elems {
                collect_soa_container_or_ptr_rec(types, e, candidates, visited, out);
            }
        }
        TypeKind::Adt { args, variants, .. } => {
            if desc.meta.candidate == RepresentationCandidate::Vec
                || desc.meta.candidate == RepresentationCandidate::Box
            {
                if let Some(&first) = args.first() {
                    collect_any_soa_mentions(types, first, candidates, out);
                }
            }
            for v in variants {
                for f in &v.fields {
                    collect_soa_container_or_ptr_rec(types, f.ty, candidates, visited, out);
                }
            }
        }
        TypeKind::Closure { upvars, .. } | TypeKind::CoroutineClosure { upvars, .. } => {
            for &u in upvars {
                collect_soa_container_or_ptr_rec(types, u, candidates, visited, out);
            }
        }
        TypeKind::Coroutine {
            upvars,
            saved_locals,
            ..
        } => {
            for &t in upvars {
                collect_soa_container_or_ptr_rec(types, t, candidates, visited, out);
            }
            for &(_, t) in saved_locals {
                collect_soa_container_or_ptr_rec(types, t, candidates, visited, out);
            }
        }
        _ => {}
    }
}

/// Resolves the `Vec<T, Global>` type targeted by a `StandardOperation` call.
fn call_vec_ty(
    func: &Func,
    types: &TypeTable,
    op: &Op,
    operation: StandardOperation,
    args: &[ValueId],
) -> Option<TypeId> {
    match operation {
        StandardOperation::NativeVecNew
        | StandardOperation::NativeVecWithCapacity
        | StandardOperation::NativeVecFromElemZeroed
        | StandardOperation::NativeVecFromArray
        | StandardOperation::NativeVecFromRawParts => {
            let res = op.result?;
            Some(func.values.get(res.0 as usize)?.ty)
        }
        StandardOperation::NativeVecIntoBoxedSlice => {
            let &v0 = args.first()?;
            Some(func.values.get(v0.0 as usize)?.ty)
        }
        _ => {
            let &v0 = args.first()?;
            let arg_ty = func.values.get(v0.0 as usize)?.ty;
            value::pointee_of(types, arg_ty)
        }
    }
}

fn place_final_ty(func: &Func, types: &TypeTable, root: Root, path: &[Proj]) -> Option<TypeId> {
    let mut cur = func.root_ty(root)?;
    for step in path {
        match step {
            Proj::Deref => cur = value::pointee_of(types, cur)?,
            Proj::Field { ty, .. } => cur = *ty,
            Proj::Index(_) | Proj::ConstantIndex { .. } => {
                cur = match value::kind(types, types.peeled(cur)) {
                    Some(TypeKind::Array { element, .. } | TypeKind::Slice(element)) => *element,
                    _ => return None,
                };
            }
            Proj::Subslice { .. } | Proj::Downcast { .. } => {}
        }
    }
    Some(cur)
}

/// Fail-closed intra-function SoA auditor for one JIR-R [`Func`].
struct SoaFuncAuditor<'a, V> {
    fn_name: &'a JsName,
    func: &'a Func,
    types: &'a TypeTable,
    interns: &'a Interns,
    table: &'a BTreeMap<JsName, LinkItem>,
    funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
    repr_bodies: &'a BTreeMap<JsName, V>,
    state: &'a mut CandidateSet,
}

impl<V> SoaFuncAuditor<'_, V> {
    fn audit(&mut self) {
        self.audit_forbidden_containers();
        if self.state.candidates.is_empty() {
            return;
        }
        self.audit_non_baseline_carriers();
        if self.state.candidates.is_empty() {
            return;
        }

        // Pre-index value uses and terminator uses in dense Vecs indexed by `ValueId.0`.
        let n_vals = self.func.values.len();
        let mut value_uses: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n_vals];
        let mut term_uses: Vec<bool> = vec![false; n_vals];
        let mut const_scalars: Vec<bool> = vec![false; n_vals];
        for (bi, block) in self.func.blocks.iter().enumerate() {
            for v in block.term.uses() {
                if let Some(slot) = term_uses.get_mut(v.0 as usize) {
                    *slot = true;
                }
            }
            for (oi, op) in block.ops.iter().enumerate() {
                if let (
                    Some(res),
                    OpKind::Const(ConstTree::Scalar { .. } | ConstTree::Address(_)),
                ) = (op.result, &op.kind)
                {
                    if let Some(slot) = const_scalars.get_mut(res.0 as usize) {
                        *slot = true;
                    }
                }
                for u in op.kind.uses() {
                    if let Some(slot) = value_uses.get_mut(u.0 as usize) {
                        slot.push((bi, oi));
                    }
                }
            }
        }

        // 3. Audit every op in every block (fail-closed).
        let mut bool_views: Vec<(TypeId, Option<ValueId>)> = Vec::new();
        let mut to_disqualify: BTreeSet<TypeId> = BTreeSet::new();
        let debug = soa_debug();

        for block in &self.func.blocks {
            for op in &block.ops {
                if self.state.candidates.is_empty() {
                    return;
                }
                let before_len = to_disqualify.len();
                match &op.kind {
                    OpKind::BorrowView { root, path, .. } | OpKind::RawView { root, path, .. } => {
                        self.audit_place_view(
                            *root,
                            path,
                            op.result,
                            &mut to_disqualify,
                            &mut bool_views,
                        );
                    }
                    OpKind::Field { base, field, .. } | OpKind::SetField { base, field, .. } => {
                        if *field == 0 {
                            if let Some(base_def) = self.func.values.get(base.0 as usize) {
                                if let Some(t) =
                                    is_soa_vec(self.types, base_def.ty, &self.state.candidates)
                                {
                                    to_disqualify.insert(t);
                                }
                            }
                        }
                    }
                    OpKind::Aggregate { ty, .. } => {
                        if let Some(t) = is_soa_vec(self.types, *ty, &self.state.candidates) {
                            to_disqualify.insert(t);
                        }
                    }
                    OpKind::Const(tree) => {
                        if let Some(res) = op.result {
                            if let Some(res_def) = self.func.values.get(res.0 as usize) {
                                self.audit_const_tree(tree, res_def.ty, 0, &mut to_disqualify);
                            }
                        }
                    }
                    OpKind::SliceView { ptr, .. } => {
                        if let Some(res) = op.result {
                            if let Some(res_def) = self.func.values.get(res.0 as usize) {
                                if let Some(t) =
                                    is_soa_slice_ptr(self.types, res_def.ty, &self.state.candidates)
                                {
                                    let ptr_ok =
                                        self.func.values.get(ptr.0 as usize).and_then(|d| {
                                            is_soa_elem_ptr(
                                                self.types,
                                                d.ty,
                                                &self.state.candidates,
                                            )
                                        }) == Some(t);
                                    if !ptr_ok {
                                        to_disqualify.insert(t);
                                    }
                                }
                            }
                        }
                    }
                    OpKind::Cast { kind, v, from, to } => {
                        self.audit_cast(*kind, *v, *from, *to, &const_scalars, &mut to_disqualify);
                    }
                    OpKind::Call {
                        callee,
                        args,
                        semantic,
                        host,
                        ..
                    } => {
                        self.audit_call(
                            op,
                            callee,
                            args,
                            *semantic,
                            host.is_some(),
                            &mut to_disqualify,
                        );
                    }
                    OpKind::Intrinsic {
                        name,
                        types: inst_tys,
                        args,
                        ..
                    } => {
                        self.audit_intrinsic(op, *name, inst_tys, args, &mut to_disqualify);
                    }
                    OpKind::Host { args, .. } => {
                        for &arg in args {
                            if let Some(d) = self.func.values.get(arg.0 as usize) {
                                collect_any_soa_mentions(
                                    self.types,
                                    d.ty,
                                    &self.state.candidates,
                                    &mut to_disqualify,
                                );
                            }
                        }
                        if let Some(res) = op.result {
                            if let Some(d) = self.func.values.get(res.0 as usize) {
                                collect_any_soa_mentions(
                                    self.types,
                                    d.ty,
                                    &self.state.candidates,
                                    &mut to_disqualify,
                                );
                            }
                        }
                    }
                    OpKind::Drop {
                        root,
                        path,
                        glue: DropGlue::Dynamic,
                    } => {
                        if let Some(final_ty) = place_final_ty(self.func, self.types, *root, path) {
                            collect_soa_container_or_ptr_mentions(
                                self.types,
                                final_ty,
                                &self.state.candidates,
                                &mut to_disqualify,
                            );
                        }
                    }
                    OpKind::Convert { v, .. } => {
                        if let Some(d) = self.func.values.get(v.0 as usize) {
                            collect_soa_container_or_ptr_mentions(
                                self.types,
                                d.ty,
                                &self.state.candidates,
                                &mut to_disqualify,
                            );
                        }
                        if let Some(res) = op.result {
                            if let Some(d) = self.func.values.get(res.0 as usize) {
                                collect_soa_container_or_ptr_mentions(
                                    self.types,
                                    d.ty,
                                    &self.state.candidates,
                                    &mut to_disqualify,
                                );
                            }
                        }
                    }
                    _ => {}
                }
                if debug && to_disqualify.len() > before_len {
                    eprintln!(
                        "[soa] op in {} flagged {:?}: {:?}",
                        self.fn_name, to_disqualify, op.kind
                    );
                }
            }
        }

        // 4. Validate every SoA `bool` field borrow.
        for (elem_ty, view_opt) in bool_views {
            if to_disqualify.contains(&elem_ty) {
                continue;
            }
            let Some(view_v) = view_opt else {
                to_disqualify.insert(elem_ty);
                continue;
            };
            if term_uses.get(view_v.0 as usize).copied().unwrap_or(false) {
                to_disqualify.insert(elem_ty);
                continue;
            }
            let Some(uses) = value_uses.get(view_v.0 as usize) else {
                continue;
            };
            let all_direct = uses.iter().all(|&(bi, oi)| {
                let use_op = &self.func.blocks[bi].ops[oi];
                match &use_op.kind {
                    OpKind::Deref(p) if *p == view_v => true,
                    OpKind::StoreThrough { ptr, v } if *ptr == view_v && *v != view_v => true,
                    OpKind::Drop {
                        glue: DropGlue::None,
                        ..
                    } => true,
                    _ => false,
                }
            });
            if !all_direct {
                if debug {
                    eprintln!(
                        "[soa] bool field view escaped in {} for {:?}",
                        self.fn_name, elem_ty
                    );
                }
                to_disqualify.insert(elem_ty);
            }
        }

        for t in to_disqualify {
            self.state.disqualify(t);
        }
    }

    fn audit_forbidden_containers(&mut self) {
        let mut bad_types = BTreeSet::new();
        for ty in func_referenced_types(self.func) {
            let Some(desc) = self.types.get(ty) else {
                continue;
            };
            if desc.meta.candidate == RepresentationCandidate::Vec
                && desc.meta.allocator != AllocatorIdentity::Global
            {
                if let TypeKind::Adt { args, .. } = &desc.kind {
                    if let Some(&elem) = args.first() {
                        if let Some(t) = is_soa_elem(self.types, elem, &self.state.candidates) {
                            bad_types.insert(t);
                        }
                    }
                }
            }
            if desc.meta.candidate == RepresentationCandidate::Box {
                if let TypeKind::Adt { args, .. } = &desc.kind {
                    if let Some(&inner) = args.first() {
                        if let Some(t) = is_soa_elem(self.types, inner, &self.state.candidates)
                            .or_else(|| is_soa_slice(self.types, inner, &self.state.candidates))
                        {
                            bad_types.insert(t);
                        }
                    }
                }
            }
            if let Some(tail) = desc.js.unsized_tail {
                if let Some(t) = is_soa_slice(self.types, tail.ty, &self.state.candidates) {
                    bad_types.insert(t);
                }
            }
        }
        for t in bad_types {
            if soa_debug() {
                eprintln!(
                    "[soa] disqualify {:?} (forbidden container in {})",
                    t, self.fn_name
                );
            }
            self.state.disqualify(t);
        }
    }

    fn audit_non_baseline_carriers(&mut self) {
        let mut non_baseline = BTreeSet::new();
        for v in &self.func.values {
            if v.carrier != Carrier::Baseline {
                collect_soa_container_or_ptr_mentions(
                    self.types,
                    v.ty,
                    &self.state.candidates,
                    &mut non_baseline,
                );
            }
        }
        for t in non_baseline {
            if soa_debug() {
                eprintln!(
                    "[soa] disqualify {:?} (non-baseline carrier in {})",
                    t, self.fn_name
                );
            }
            self.state.disqualify(t);
        }
    }

    fn audit_place_view(
        &self,
        root: Root,
        path: &[Proj],
        result: Option<ValueId>,
        to_disqualify: &mut BTreeSet<TypeId>,
        bool_views: &mut Vec<(TypeId, Option<ValueId>)>,
    ) {
        let Some(mut cur_ty) = self.func.root_ty(root) else {
            return;
        };
        let mut from_soa_elem: Option<TypeId> = None;
        let mut from_soa_slice: Option<TypeId> = None;

        for step in path {
            match step {
                Proj::Deref => {
                    let Some(pointee) = value::pointee_of(self.types, cur_ty) else {
                        return;
                    };
                    if let Some(t) = is_soa_elem(self.types, pointee, &self.state.candidates) {
                        from_soa_elem = Some(t);
                        from_soa_slice = None;
                    } else if let Some(t) =
                        is_soa_slice(self.types, pointee, &self.state.candidates)
                    {
                        from_soa_slice = Some(t);
                        from_soa_elem = None;
                    } else {
                        from_soa_elem = None;
                        from_soa_slice = None;
                    }
                    cur_ty = pointee;
                }
                Proj::Index(_) | Proj::ConstantIndex { .. } => {
                    if let Some(t) = is_soa_slice(self.types, cur_ty, &self.state.candidates) {
                        from_soa_elem = Some(t);
                        from_soa_slice = None;
                        cur_ty = t;
                    } else {
                        from_soa_elem = None;
                        from_soa_slice = None;
                        cur_ty = match value::kind(self.types, self.types.peeled(cur_ty)) {
                            Some(TypeKind::Array { element, .. } | TypeKind::Slice(element)) => {
                                *element
                            }
                            _ => return,
                        };
                    }
                }
                Proj::Subslice { .. } | Proj::Downcast { .. } => {}
                Proj::Field {
                    field,
                    ty: field_ty,
                } => {
                    if *field == 0 {
                        if let Some(t) = is_soa_vec(self.types, cur_ty, &self.state.candidates) {
                            to_disqualify.insert(t);
                        }
                    }
                    if let Some(elem_t) = from_soa_elem {
                        if let Some(shape) = self.state.candidates.get(&elem_t) {
                            if shape
                                .fields
                                .iter()
                                .any(|f| f.field_idx == *field && f.scalar.is_bool())
                            {
                                bool_views.push((elem_t, result));
                            }
                        }
                    }
                    from_soa_elem = None;
                    from_soa_slice = None;
                    cur_ty = *field_ty;
                }
            }
        }

        if let Some(t) = is_soa_elem(self.types, cur_ty, &self.state.candidates) {
            if from_soa_elem != Some(t) {
                to_disqualify.insert(t);
            }
        }
        if let Some(t) = is_soa_slice(self.types, cur_ty, &self.state.candidates) {
            if from_soa_slice != Some(t) {
                to_disqualify.insert(t);
            }
        }
    }

    fn audit_const_tree(
        &self,
        tree: &ConstTree,
        ty: TypeId,
        depth: u32,
        to_disqualify: &mut BTreeSet<TypeId>,
    ) {
        if depth > 64 {
            return;
        }
        let peeled = self.types.peeled(ty);

        if let Some(t) = is_soa_vec(self.types, peeled, &self.state.candidates) {
            if depth > 0 {
                to_disqualify.insert(t);
            }
            return;
        }

        if let Some(t) = is_soa_slice_ptr(self.types, peeled, &self.state.candidates) {
            let is_top_empty = depth == 0
                && match tree {
                    ConstTree::Slice(elems) => elems.is_empty(),
                    ConstTree::SliceAddress { length, .. } => *length == 0,
                    _ => false,
                };
            if !is_top_empty {
                to_disqualify.insert(t);
            }
            return;
        }

        if let Some(t) = is_soa_elem_ptr(self.types, peeled, &self.state.candidates) {
            let is_dangling_raw = value::is_raw(self.types, ty)
                && matches!(tree, ConstTree::Address(_) | ConstTree::Scalar { .. });
            if !is_dangling_raw {
                to_disqualify.insert(t);
            }
            return;
        }

        let Some(desc) = self.types.get(peeled) else {
            return;
        };

        match tree {
            ConstTree::Aggregate(fields) => match &desc.kind {
                TypeKind::Adt {
                    form: NominalForm::Struct,
                    variants,
                    ..
                } => {
                    if let Some(variant) = variants.first() {
                        for (field, sub) in variant.fields.iter().zip(fields) {
                            self.audit_const_tree(sub, field.ty, depth + 1, to_disqualify);
                        }
                    }
                }
                TypeKind::Tuple(elems) => {
                    for (&elem, sub) in elems.iter().zip(fields) {
                        self.audit_const_tree(sub, elem, depth + 1, to_disqualify);
                    }
                }
                TypeKind::Array { element, .. } => {
                    for sub in fields {
                        self.audit_const_tree(sub, *element, depth + 1, to_disqualify);
                    }
                }
                TypeKind::Closure { upvars, .. } => {
                    for (&upvar, sub) in upvars.iter().zip(fields) {
                        self.audit_const_tree(sub, upvar, depth + 1, to_disqualify);
                    }
                }
                _ => {
                    collect_soa_container_or_ptr_mentions(
                        self.types,
                        ty,
                        &self.state.candidates,
                        to_disqualify,
                    );
                }
            },
            ConstTree::Variant { variant, fields } => {
                if let Some(vdesc) = self.types.variants(peeled).get(*variant as usize) {
                    for (field, sub) in vdesc.fields.iter().zip(fields) {
                        self.audit_const_tree(sub, field.ty, depth + 1, to_disqualify);
                    }
                }
            }
            ConstTree::Reference(inner) => {
                if let Some(pointee) = value::pointee_of(self.types, peeled) {
                    self.audit_const_tree(inner, pointee, depth + 1, to_disqualify);
                }
            }
            ConstTree::Slice(elems) => {
                if let Some(pointee) = value::pointee_of(self.types, peeled) {
                    if let Some(TypeKind::Slice(elem)) =
                        value::kind(self.types, self.types.peeled(pointee))
                    {
                        for sub in elems {
                            self.audit_const_tree(sub, *elem, depth + 1, to_disqualify);
                        }
                    }
                }
            }
            ConstTree::Global { target, .. } => {
                if let GlobalReference::Function { link_name, .. } = target {
                    if let Some(s) = self.interns.str(*link_name) {
                        if let Some(ti) = self.table.get(s) {
                            if is_intercepted_native_vec_body(ti) {
                                if let (Some(ti_body), Some(ti_interns)) =
                                    (&ti.jir.body, ti.jir.interns())
                                {
                                    for mty in body_mentioned_types(ti_body) {
                                        collect_any_soa_mentions(
                                            &ti_interns.types,
                                            mty,
                                            &self.state.candidates,
                                            to_disqualify,
                                        );
                                    }
                                } else {
                                    to_disqualify.extend(self.state.candidates.keys().copied());
                                }
                            }
                        }
                    }
                }
                collect_soa_container_or_ptr_mentions(
                    self.types,
                    ty,
                    &self.state.candidates,
                    to_disqualify,
                );
            }
            ConstTree::StaticField { .. }
            | ConstTree::StaticSlice { .. }
            | ConstTree::Dyn { .. } => {
                collect_soa_container_or_ptr_mentions(
                    self.types,
                    ty,
                    &self.state.candidates,
                    to_disqualify,
                );
            }
            ConstTree::Unit
            | ConstTree::Scalar { .. }
            | ConstTree::Str(_)
            | ConstTree::Address(_)
            | ConstTree::SliceAddress { .. } => {}
        }
    }

    fn audit_cast(
        &self,
        kind: CastKind,
        v: ValueId,
        from: TypeId,
        to: TypeId,
        const_scalars: &[bool],
        to_disqualify: &mut BTreeSet<TypeId>,
    ) {
        let mut from_m = BTreeSet::new();
        let mut to_m = BTreeSet::new();
        collect_soa_container_or_ptr_mentions(
            self.types,
            from,
            &self.state.candidates,
            &mut from_m,
        );
        collect_soa_container_or_ptr_mentions(self.types, to, &self.state.candidates, &mut to_m);
        if from_m.is_empty() && to_m.is_empty() {
            return;
        }

        match kind {
            CastKind::PointerCoercion => {
                if let (Some(t1), Some(t2)) = (
                    is_soa_elem_ptr(self.types, from, &self.state.candidates),
                    is_soa_elem_ptr(self.types, to, &self.state.candidates),
                ) {
                    if t1 == t2 {
                        return;
                    }
                }
                if let (Some(t1), Some(t2)) = (
                    is_soa_slice_ptr(self.types, from, &self.state.candidates),
                    is_soa_slice_ptr(self.types, to, &self.state.candidates),
                ) {
                    if t1 == t2 {
                        return;
                    }
                }
                to_disqualify.extend(from_m);
                to_disqualify.extend(to_m);
            }
            CastKind::PtrToPtr => {
                let from_soa = is_soa_elem_ptr(self.types, from, &self.state.candidates)
                    .map(|t| (t, false))
                    .or_else(|| {
                        is_soa_slice_ptr(self.types, from, &self.state.candidates)
                            .map(|t| (t, true))
                    });
                let to_soa = is_soa_elem_ptr(self.types, to, &self.state.candidates)
                    .map(|t| (t, false))
                    .or_else(|| {
                        is_soa_slice_ptr(self.types, to, &self.state.candidates).map(|t| (t, true))
                    });
                let allowed = match (from_soa, to_soa) {
                    (Some((t1, false)), Some((t2, false))) => t1 == t2,
                    (Some((t1, true)), Some((t2, true))) => t1 == t2,
                    (Some((t1, true)), Some((t2, false))) => t1 == t2,
                    _ => false,
                };
                if !allowed {
                    to_disqualify.extend(from_m);
                    to_disqualify.extend(to_m);
                }
            }
            CastKind::Transmute => {
                let allowed = match (
                    is_soa_elem_ptr(self.types, from, &self.state.candidates),
                    is_soa_elem_ptr(self.types, to, &self.state.candidates),
                ) {
                    (Some(t1), Some(t2)) if t1 == t2 => true,
                    _ => matches!(
                        (
                            is_soa_slice_ptr(self.types, from, &self.state.candidates),
                            is_soa_slice_ptr(self.types, to, &self.state.candidates),
                        ),
                        (Some(t1), Some(t2)) if t1 == t2
                    ),
                };
                if !allowed {
                    to_disqualify.extend(from_m);
                    to_disqualify.extend(to_m);
                }
            }
            CastKind::PointerWithExposedProvenance => {
                if from_m.is_empty()
                    && is_soa_elem_ptr(self.types, to, &self.state.candidates).is_some()
                    && value::is_raw(self.types, to)
                    && const_scalars.get(v.0 as usize).copied().unwrap_or(false)
                {
                    return;
                }
                to_disqualify.extend(from_m);
                to_disqualify.extend(to_m);
            }
            CastKind::PointerExposeProvenance => {
                if to_m.is_empty()
                    && is_soa_elem_ptr(self.types, from, &self.state.candidates).is_some()
                {
                    return;
                }
                to_disqualify.extend(from_m);
                to_disqualify.extend(to_m);
            }
            CastKind::Subtype => {
                if self.types.peeled(from) == self.types.peeled(to) {
                    return;
                }
                to_disqualify.extend(from_m);
                to_disqualify.extend(to_m);
            }
            _ => {
                to_disqualify.extend(from_m);
                to_disqualify.extend(to_m);
            }
        }
    }

    fn audit_call(
        &self,
        op: &Op,
        callee: &Callee,
        args: &[ValueId],
        semantic: CallSemantic,
        has_host: bool,
        to_disqualify: &mut BTreeSet<TypeId>,
    ) {
        if has_host {
            for &arg in args {
                if let Some(d) = self.func.values.get(arg.0 as usize) {
                    collect_any_soa_mentions(
                        self.types,
                        d.ty,
                        &self.state.candidates,
                        to_disqualify,
                    );
                }
            }
            if let Some(res) = op.result {
                if let Some(d) = self.func.values.get(res.0 as usize) {
                    collect_any_soa_mentions(
                        self.types,
                        d.ty,
                        &self.state.candidates,
                        to_disqualify,
                    );
                }
            }
            return;
        }

        if let CallSemantic::Standard(std_op) = semantic {
            match std_op {
                StandardOperation::NativeVecNew
                | StandardOperation::NativeVecWithCapacity
                | StandardOperation::NativeVecFromElemZeroed
                | StandardOperation::NativeVecFromArray
                | StandardOperation::NativeVecCapacity
                | StandardOperation::NativeVecAsPtr
                | StandardOperation::NativeVecAsMutPtr
                | StandardOperation::NativeVecAsSlice
                | StandardOperation::NativeVecAsMutSlice
                | StandardOperation::NativeVecReserve
                | StandardOperation::NativeVecReserveExact
                | StandardOperation::NativeVecTryReserve
                | StandardOperation::NativeVecTryReserveExact
                | StandardOperation::NativeVecShrinkTo
                | StandardOperation::NativeVecTryShrinkTo
                | StandardOperation::NativeVecGrowOne
                | StandardOperation::NativeVecPush
                | StandardOperation::NativeVecPushMut
                | StandardOperation::NativeVecPop
                | StandardOperation::VecAsSlice
                | StandardOperation::VecResize => {
                    if let Some(vt) = call_vec_ty(self.func, self.types, op, std_op, args) {
                        if is_soa_vec(self.types, vt, &self.state.candidates).is_some() {
                            return;
                        }
                    }
                }
                StandardOperation::NativeVecFromRawParts
                | StandardOperation::NativeVecIntoBoxedSlice
                | StandardOperation::SliceSort
                | StandardOperation::SliceSortCmp
                | StandardOperation::TypedAlloc
                | StandardOperation::TypedRealloc
                | StandardOperation::TypedDealloc
                | StandardOperation::TypedDeallocRaw
                | StandardOperation::BoxNew
                | StandardOperation::BoxDrop
                | StandardOperation::RcAllocSlice
                | StandardOperation::ArcAllocSlice => {
                    for &arg in args {
                        if let Some(d) = self.func.values.get(arg.0 as usize) {
                            collect_any_soa_mentions(
                                self.types,
                                d.ty,
                                &self.state.candidates,
                                to_disqualify,
                            );
                        }
                    }
                    if let Some(res) = op.result {
                        if let Some(d) = self.func.values.get(res.0 as usize) {
                            collect_any_soa_mentions(
                                self.types,
                                d.ty,
                                &self.state.candidates,
                                to_disqualify,
                            );
                        }
                    }
                    return;
                }
                _ => {
                    let mut touched = BTreeSet::new();
                    for &arg in args {
                        if let Some(d) = self.func.values.get(arg.0 as usize) {
                            collect_soa_container_or_ptr_mentions(
                                self.types,
                                d.ty,
                                &self.state.candidates,
                                &mut touched,
                            );
                        }
                    }
                    if let Some(res) = op.result {
                        if let Some(d) = self.func.values.get(res.0 as usize) {
                            collect_soa_container_or_ptr_mentions(
                                self.types,
                                d.ty,
                                &self.state.candidates,
                                &mut touched,
                            );
                        }
                    }
                    if !touched.is_empty() {
                        to_disqualify.extend(touched);
                        return;
                    }
                }
            }
        }

        let mut touched = BTreeSet::new();
        for &arg in args {
            if let Some(d) = self.func.values.get(arg.0 as usize) {
                collect_soa_container_or_ptr_mentions(
                    self.types,
                    d.ty,
                    &self.state.candidates,
                    &mut touched,
                );
            }
        }
        if let Some(res) = op.result {
            if let Some(d) = self.func.values.get(res.0 as usize) {
                collect_soa_container_or_ptr_mentions(
                    self.types,
                    d.ty,
                    &self.state.candidates,
                    &mut touched,
                );
            }
        }
        if touched.is_empty() {
            return;
        }

        match callee {
            Callee::Direct { link_name, .. } => {
                let Some(name_str) = self.interns.str(*link_name) else {
                    to_disqualify.extend(touched);
                    return;
                };
                if self.repr_bodies.contains_key(name_str)
                    || !self.table.contains_key(name_str)
                    || !matches!(self.funcs.get(name_str), Some(Ok(_)))
                {
                    to_disqualify.extend(touched);
                }
            }
            Callee::Nop => {}
            Callee::Runtime { .. }
            | Callee::Foreign { .. }
            | Callee::Indirect { .. }
            | Callee::Virtual { .. }
            | Callee::Intrinsic { .. } => {
                to_disqualify.extend(touched);
            }
        }
    }

    fn audit_intrinsic(
        &self,
        op: &Op,
        name: StrId,
        inst_tys: &[TypeId],
        args: &[ValueId],
        to_disqualify: &mut BTreeSet<TypeId>,
    ) {
        let mut touched = BTreeSet::new();
        for &arg in args {
            if let Some(d) = self.func.values.get(arg.0 as usize) {
                collect_soa_container_or_ptr_mentions(
                    self.types,
                    d.ty,
                    &self.state.candidates,
                    &mut touched,
                );
            }
        }
        if let Some(res) = op.result {
            if let Some(d) = self.func.values.get(res.0 as usize) {
                collect_soa_container_or_ptr_mentions(
                    self.types,
                    d.ty,
                    &self.state.candidates,
                    &mut touched,
                );
            }
        }
        for &ty in inst_tys {
            collect_soa_container_or_ptr_mentions(
                self.types,
                ty,
                &self.state.candidates,
                &mut touched,
            );
            if let Some(t) = is_soa_elem(self.types, ty, &self.state.candidates) {
                touched.insert(t);
            }
        }
        if touched.is_empty() {
            return;
        }

        let Some(iname) = self.interns.str(name) else {
            to_disqualify.extend(touched);
            return;
        };
        match iname {
            "slice_get_unchecked"
            | "typed_swap_nonoverlapping"
            | "copy"
            | "copy_nonoverlapping"
            | "volatile_copy_memory"
            | "volatile_copy_nonoverlapping_memory"
            | "read_via_copy"
            | "volatile_load"
            | "unaligned_volatile_load"
            | "write_via_move"
            | "volatile_store"
            | "unaligned_volatile_store"
            | "offset"
            | "arith_offset"
            | "ptr_offset_from"
            | "ptr_offset_from_unsigned"
            | "ptr_metadata"
            | "size_of"
            | "min_align_of"
            | "align_of"
            | "needs_drop"
            | "type_id"
            | "type_name"
            | "forget"
            | "assume"
            | "black_box"
            | "likely"
            | "unlikely"
            | "const_eval_select" => {}
            "aggregate_raw_ptr" => {
                if let Some(res) = op.result {
                    if let Some(res_def) = self.func.values.get(res.0 as usize) {
                        if let Some(t) =
                            is_soa_slice_ptr(self.types, res_def.ty, &self.state.candidates)
                        {
                            let data_ok = args
                                .first()
                                .and_then(|a| self.func.values.get(a.0 as usize))
                                .and_then(|d| {
                                    is_soa_elem_ptr(self.types, d.ty, &self.state.candidates)
                                })
                                == Some(t);
                            if !data_ok {
                                to_disqualify.insert(t);
                            }
                        }
                    }
                }
            }
            "transmute" | "transmute_unchecked" => {
                let from_ty = args
                    .first()
                    .and_then(|a| self.func.values.get(a.0 as usize))
                    .map(|d| d.ty);
                let to_ty = op
                    .result
                    .and_then(|r| self.func.values.get(r.0 as usize))
                    .map(|d| d.ty);
                let allowed = match (from_ty, to_ty) {
                    (Some(f), Some(t)) => {
                        let same_elem_ptr = matches!(
                            (
                                is_soa_elem_ptr(self.types, f, &self.state.candidates),
                                is_soa_elem_ptr(self.types, t, &self.state.candidates),
                            ),
                            (Some(a), Some(b)) if a == b
                        );
                        let same_slice_ptr = matches!(
                            (
                                is_soa_slice_ptr(self.types, f, &self.state.candidates),
                                is_soa_slice_ptr(self.types, t, &self.state.candidates),
                            ),
                            (Some(a), Some(b)) if a == b
                        );
                        same_elem_ptr || same_slice_ptr
                    }
                    _ => false,
                };
                if !allowed {
                    to_disqualify.extend(touched);
                }
            }
            _ => {
                to_disqualify.extend(touched);
            }
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
    use crate::item::{ItemMap, Linkage};
    use crate::jir::origin::OriginId;
    use crate::jir::repr::{FuncBuilder, RTerm};
    use crate::jir::semantic::{AbiSignature, DefinitionId, ItemId, Signature};
    use crate::jir::summary::ItemSummary;
    use crate::jir::types::{
        EnumRepr, FieldDescriptor, IntFacts, IntRepr, JsFacts, LayoutFacts, NominalDiscriminant,
        PointerForm, TransparentField, TypeMeta, VariantDescriptor,
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

    #[allow(
        dead_code,
        reason = "test fixture fields retained for scenario variations"
    )]
    struct Fixture {
        interns: Interns,
        t_unit: TypeId,
        t_bool: TypeId,
        t_u32: TypeId,
        t_i32: TypeId,
        t_f64: TypeId,
        t_node: TypeId,
        t_vec: TypeId,
        t_ref_vec: TypeId,
        t_mut_vec: TypeId,
        t_slice: TypeId,
        t_ref_slice: TypeId,
        t_mut_slice: TypeId,
        t_ref_node: TypeId,
        t_mut_node: TypeId,
        t_ptr_node: TypeId,
        t_mut_ptr_node: TypeId,
        t_ref_bool: TypeId,
        t_mut_bool: TypeId,
        t_opt_node: TypeId,
        t_arr2_node: TypeId,
        t_ref_arr2_node: TypeId,
    }

    fn build_node_fixture() -> Fixture {
        let mut interns = Interns::new();
        let s_x = interns.intern_str("x");
        let s_prev = interns.intern_str("prev");
        let s_z = interns.intern_str("z");
        let s_steiner = interns.intern_str("steiner");
        let s_node = interns.intern_str("Node");
        let s_vec = interns.intern_str("Vec");
        let s_buf = interns.intern_str("buf");
        let s_len = interns.intern_str("len");
        let s_none = interns.intern_str("None");
        let s_some = interns.intern_str("Some");
        let s_f0 = interns.intern_str("_0");

        let t_unit = TypeId(1);
        let t_bool = TypeId(2);
        let t_u32 = TypeId(3);
        let t_i32 = TypeId(4);
        let t_f64 = TypeId(5);
        let t_node = TypeId(6);
        let t_vec = TypeId(7);
        let t_ref_vec = TypeId(8);
        let t_mut_vec = TypeId(9);
        let t_slice = TypeId(10);
        let t_ref_slice = TypeId(11);
        let t_mut_slice = TypeId(12);
        let t_ref_node = TypeId(13);
        let t_mut_node = TypeId(14);
        let t_ptr_node = TypeId(15);
        let t_mut_ptr_node = TypeId(16);
        let t_ref_bool = TypeId(17);
        let t_mut_bool = TypeId(18);
        let t_opt_node = TypeId(19);
        let t_arr2_node = TypeId(20);
        let t_ref_arr2_node = TypeId(21);

        // ()
        interns.types.insert(TypeDescriptor {
            id: t_unit,
            name: None,
            kind: TypeKind::Tuple(Vec::new()),
            layout: LayoutFacts {
                size: Some(0),
                align: Some(1),
                sized: true,
                zst: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(t_unit),
        });
        // bool
        interns.types.insert(TypeDescriptor {
            id: t_bool,
            name: None,
            kind: TypeKind::Bool,
            layout: LayoutFacts {
                size: Some(1),
                align: Some(1),
                sized: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(t_bool),
        });
        // u32 / usize
        let mut u32_d = TypeDescriptor {
            id: t_u32,
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
            js: base_facts(t_u32),
        };
        u32_d.js.int = Some(IntFacts {
            signed: false,
            bits: 32,
            repr: IntRepr::Number,
        });
        interns.types.insert(u32_d);
        // i32
        let mut i32_d = TypeDescriptor {
            id: t_i32,
            name: None,
            kind: TypeKind::Integer {
                signed: true,
                bits: 32,
            },
            layout: LayoutFacts {
                size: Some(4),
                align: Some(4),
                sized: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(t_i32),
        };
        i32_d.js.int = Some(IntFacts {
            signed: true,
            bits: 32,
            repr: IntRepr::Number,
        });
        interns.types.insert(i32_d);
        // f64
        interns.types.insert(TypeDescriptor {
            id: t_f64,
            name: None,
            kind: TypeKind::Float { bits: 64 },
            layout: LayoutFacts {
                size: Some(8),
                align: Some(8),
                sized: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(t_f64),
        });
        // Node { x: f64, prev: u32, z: i32, steiner: bool }
        let node_fields = vec![
            FieldDescriptor {
                key: FieldKey::Name(s_x),
                ty: t_f64,
                offset: 0,
            },
            FieldDescriptor {
                key: FieldKey::Name(s_prev),
                ty: t_u32,
                offset: 8,
            },
            FieldDescriptor {
                key: FieldKey::Name(s_z),
                ty: t_i32,
                offset: 12,
            },
            FieldDescriptor {
                key: FieldKey::Name(s_steiner),
                ty: t_bool,
                offset: 16,
            },
        ];
        let mut node_d = TypeDescriptor {
            id: t_node,
            name: None,
            kind: TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 100,
                },
                args: Vec::new(),
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: s_node,
                    fields: node_fields.clone(),
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            layout: LayoutFacts {
                size: Some(24),
                align: Some(8),
                sized: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(t_node),
        };
        node_d.js.is_indirect = true;
        node_d.js.needs_clone = true;
        node_d.js.fields = node_fields;
        interns.types.insert(node_d);

        // Vec<Node, Global>
        let vec_fields = vec![
            FieldDescriptor {
                key: FieldKey::Name(s_buf),
                ty: t_u32,
                offset: 0,
            },
            FieldDescriptor {
                key: FieldKey::Name(s_len),
                ty: t_u32,
                offset: 4,
            },
        ];
        let mut vec_d = TypeDescriptor {
            id: t_vec,
            name: None,
            kind: TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 2,
                    local_hash: 200,
                },
                args: vec![t_node],
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: s_vec,
                    fields: vec_fields.clone(),
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            layout: LayoutFacts {
                size: Some(12),
                align: Some(4),
                sized: true,
                needs_drop: true,
                ..Default::default()
            },
            meta: TypeMeta {
                candidate: RepresentationCandidate::Vec,
                abi: NominalAbi::Rust,
                boundary: PinnedBoundary::None,
                allocator: AllocatorIdentity::Global,
                iterator: IteratorIdentity::None,
            },
            js: base_facts(t_vec),
        };
        vec_d.js.is_indirect = true;
        vec_d.js.needs_clone = true;
        vec_d.js.fields = vec_fields;
        interns.types.insert(vec_d);

        // &Vec<Node> / &mut Vec<Node>
        for (id, mutable) in [(t_ref_vec, false), (t_mut_vec, true)] {
            let mut d = TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Reference {
                    pointee: t_vec,
                    mutable,
                },
                layout: LayoutFacts {
                    size: Some(4),
                    align: Some(4),
                    sized: true,
                    ..Default::default()
                },
                meta: TypeMeta::default(),
                js: base_facts(id),
            };
            d.js.pointer_form = Some(PointerForm::Thin);
            interns.types.insert(d);
        }

        // [Node], &[Node], &mut [Node]
        let mut slice_d = TypeDescriptor {
            id: t_slice,
            name: None,
            kind: TypeKind::Slice(t_node),
            layout: LayoutFacts {
                size: None,
                align: Some(8),
                sized: false,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(t_slice),
        };
        slice_d.js.is_indirect = true;
        interns.types.insert(slice_d);
        for (id, mutable) in [(t_ref_slice, false), (t_mut_slice, true)] {
            let mut d = TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Reference {
                    pointee: t_slice,
                    mutable,
                },
                layout: LayoutFacts {
                    size: Some(8),
                    align: Some(4),
                    sized: true,
                    ..Default::default()
                },
                meta: TypeMeta::default(),
                js: base_facts(id),
            };
            d.js.pointer_form = Some(PointerForm::FatSlice);
            interns.types.insert(d);
        }

        // &Node, &mut Node, *const Node, *mut Node
        for (id, raw, mutable, form) in [
            (t_ref_node, false, false, PointerForm::Thin),
            (t_mut_node, false, true, PointerForm::Thin),
            (t_ptr_node, true, false, PointerForm::Slot),
            (t_mut_ptr_node, true, true, PointerForm::Slot),
        ] {
            let mut d = TypeDescriptor {
                id,
                name: None,
                kind: if raw {
                    TypeKind::RawPointer {
                        pointee: t_node,
                        mutable,
                    }
                } else {
                    TypeKind::Reference {
                        pointee: t_node,
                        mutable,
                    }
                },
                layout: LayoutFacts {
                    size: Some(4),
                    align: Some(4),
                    sized: true,
                    ..Default::default()
                },
                meta: TypeMeta::default(),
                js: base_facts(id),
            };
            d.js.pointer_form = Some(form);
            interns.types.insert(d);
        }

        // &bool, &mut bool
        for (id, mutable) in [(t_ref_bool, false), (t_mut_bool, true)] {
            let mut d = TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Reference {
                    pointee: t_bool,
                    mutable,
                },
                layout: LayoutFacts {
                    size: Some(4),
                    align: Some(4),
                    sized: true,
                    ..Default::default()
                },
                meta: TypeMeta::default(),
                js: base_facts(id),
            };
            d.js.pointer_form = Some(PointerForm::Slot);
            interns.types.insert(d);
        }

        // Option<Node>
        let mut opt_d = TypeDescriptor {
            id: t_opt_node,
            name: None,
            kind: TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 2,
                    local_hash: 201,
                },
                args: vec![t_node],
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: s_none,
                        fields: Vec::new(),
                        logical_discr: 0,
                        physical_tag: Some(NominalDiscriminant { bits: 0, bytes: 1 }),
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: s_some,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(s_f0),
                            ty: t_node,
                            offset: 0,
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
                size: Some(24),
                align: Some(8),
                sized: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(t_opt_node),
        };
        opt_d.js.is_indirect = true;
        opt_d.js.needs_clone = true;
        opt_d.js.enum_repr = Some(EnumRepr::NullNiche {
            none_variant: 0,
            some_variant: 1,
        });
        interns.types.insert(opt_d);

        // [Node; 2], &[Node; 2]
        let mut arr_d = TypeDescriptor {
            id: t_arr2_node,
            name: None,
            kind: TypeKind::Array {
                element: t_node,
                length: 2,
            },
            layout: LayoutFacts {
                size: Some(48),
                align: Some(8),
                sized: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(t_arr2_node),
        };
        arr_d.js.is_indirect = true;
        arr_d.js.needs_clone = true;
        interns.types.insert(arr_d);

        let mut ref_arr_d = TypeDescriptor {
            id: t_ref_arr2_node,
            name: None,
            kind: TypeKind::Reference {
                pointee: t_arr2_node,
                mutable: false,
            },
            layout: LayoutFacts {
                size: Some(4),
                align: Some(4),
                sized: true,
                ..Default::default()
            },
            meta: TypeMeta::default(),
            js: base_facts(t_ref_arr2_node),
        };
        ref_arr_d.js.pointer_form = Some(PointerForm::Thin);
        interns.types.insert(ref_arr_d);

        Fixture {
            interns,
            t_unit,
            t_bool,
            t_u32,
            t_i32,
            t_f64,
            t_node,
            t_vec,
            t_ref_vec,
            t_mut_vec,
            t_slice,
            t_ref_slice,
            t_mut_slice,
            t_ref_node,
            t_mut_node,
            t_ptr_node,
            t_mut_ptr_node,
            t_ref_bool,
            t_mut_bool,
            t_opt_node,
            t_arr2_node,
            t_ref_arr2_node,
        }
    }

    fn speed_opts() -> Opts {
        Opts {
            optimizer: OptimizerMode::On,
            profile: Some(Profile::ReleaseSpeed),
            ..Opts::default()
        }
    }

    fn enabled_heap(smi_node_prev: Option<TypeId>) -> HeapPlan {
        let mut heap = HeapPlan {
            typed_heap_enabled: true,
            native_vec_enabled: true,
            ..HeapPlan::default()
        };
        if let Some(ty) = smi_node_prev {
            heap.smi_u32_fields.insert((ty, 1));
        }
        heap
    }

    fn make_link_item(name: &str, interns: Interns, exported: bool) -> LinkItem {
        let mut item = LinkItem {
            js: String::new(),
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: if exported {
                Linkage {
                    root: true,
                    exported: true,
                    fixed_name: None,
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
            jir: Default::default(),
        };
        item.jir.use_interns(interns);
        item
    }

    fn build_clean_func(f: &mut Fixture) -> Func {
        let s_vec_new = f.interns.intern_str("rcgjs_vec_new");
        let s_vec_push = f.interns.intern_str("rcgjs_vec_push");
        let s_vec_slice = f.interns.intern_str("rcgjs_vec_as_mut_slice");

        let mut b = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash: 1,
                },
                instantiation: 0,
            },
            Signature {
                arguments: vec![f.t_f64],
                result: f.t_f64,
                abi: AbiSignature {
                    parameters: 1,
                    ..AbiSignature::default()
                },
            },
        );
        let x_in = b.param(f.t_f64, OriginId(0));
        let v_cell = b.cell(f.t_vec, false, OriginId(0));
        b.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(v_cell), OriginId(0));
        let v_new = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 2,
                            local_hash: 1,
                        },
                        instantiation: 0,
                    },
                    link_name: s_vec_new,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeVecNew),
                host: None,
            },
            f.t_vec,
            OriginId(0),
        );
        b.effect(
            FuncBuilder::ENTRY,
            OpKind::CellStore {
                cell: v_cell,
                v: v_new,
            },
            OriginId(0),
        );
        let c_max = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar {
                bits: u128::from(u32::MAX),
                bytes: 4,
            }),
            f.t_u32,
            OriginId(0),
        );
        let c_z = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 7, bytes: 4 }),
            f.t_i32,
            OriginId(0),
        );
        let c_true = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 1 }),
            f.t_bool,
            OriginId(0),
        );
        let node_val = b.op(
            FuncBuilder::ENTRY,
            OpKind::Aggregate {
                ty: f.t_node,
                variant: 0,
                active_field: None,
                fields: vec![x_in, c_max, c_z, c_true],
            },
            f.t_node,
            OriginId(0),
        );
        let v_mut = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Cell(v_cell),
                path: Vec::new(),
                mutable: true,
            },
            f.t_mut_vec,
            OriginId(0),
        );
        b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 2,
                            local_hash: 2,
                        },
                        instantiation: 0,
                    },
                    link_name: s_vec_push,
                },
                function: None,
                args: vec![v_mut, node_val],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeVecPush),
                host: None,
            },
            f.t_unit,
            OriginId(0),
        );
        let v_mut2 = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Cell(v_cell),
                path: Vec::new(),
                mutable: true,
            },
            f.t_mut_vec,
            OriginId(0),
        );
        let slice = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 2,
                            local_hash: 3,
                        },
                        instantiation: 0,
                    },
                    link_name: s_vec_slice,
                },
                function: None,
                args: vec![v_mut2],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeVecAsMutSlice),
                host: None,
            },
            f.t_mut_slice,
            OriginId(0),
        );
        let c0 = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            f.t_u32,
            OriginId(0),
        );
        // Direct bool field read (allowed!)
        let steiner_ref = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(slice),
                path: vec![
                    Proj::Deref,
                    Proj::Index(c0),
                    Proj::Field {
                        field: 3,
                        ty: f.t_bool,
                    },
                ],
                mutable: false,
            },
            f.t_ref_bool,
            OriginId(0),
        );
        let _steiner_v = b.op(
            FuncBuilder::ENTRY,
            OpKind::Deref(steiner_ref),
            f.t_bool,
            OriginId(0),
        );
        // Direct f64 field read via local &Node borrow
        let node_ref = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(slice),
                path: vec![Proj::Deref, Proj::Index(c0)],
                mutable: false,
            },
            f.t_ref_node,
            OriginId(0),
        );
        let x_ref = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(node_ref),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: 0,
                        ty: f.t_f64,
                    },
                ],
                mutable: false,
            },
            f.t_f64,
            OriginId(0),
        );
        let x_out = b.op(
            FuncBuilder::ENTRY,
            OpKind::Deref(x_ref),
            f.t_f64,
            OriginId(0),
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(x_out)));
        b.finish()
    }

    fn run_plan(
        func: Func,
        interns: Interns,
        opts: &Opts,
        heap: &HeapPlan,
        exported: bool,
    ) -> SoaPlan {
        let name = JsName::new("main".to_owned());
        let mut table = BTreeMap::new();
        table.insert(name.clone(), make_link_item("main", interns, exported));
        let mut funcs = BTreeMap::new();
        funcs.insert(name.clone(), Ok(func));
        let repr_bodies: BTreeMap<JsName, ()> = BTreeMap::new();
        let live = BTreeSet::from([name]);
        plan(&table, &funcs, &repr_bodies, &live, opts, heap)
    }

    #[test]
    fn selects_eligible_flat_struct_with_smi_u32_and_bool() {
        let mut f = build_node_fixture();
        let func = build_clean_func(&mut f);
        let heap = enabled_heap(Some(f.t_node));
        let soa = run_plan(func, f.interns, &speed_opts(), &heap, true);
        let layout = soa.layouts.get(&f.t_node).expect("Node must be selected");
        assert_eq!(layout.fields.len(), 4);
        assert_eq!(layout.fields[0].scalar, SoaScalar::F64);
        assert_eq!(layout.fields[1].scalar, SoaScalar::SmiU32);
        assert_eq!(layout.fields[1].scalar.ta_ctor(), "Int32Array");
        assert_eq!(layout.fields[2].scalar, SoaScalar::I32);
        assert_eq!(layout.fields[3].scalar, SoaScalar::Bool);
        assert_eq!(layout.fields[3].scalar.ta_ctor(), "Uint8Array");
    }

    #[test]
    fn respects_optimizer_profile_and_native_vec_gates() {
        let mut f = build_node_fixture();
        let func = build_clean_func(&mut f);
        let heap = enabled_heap(Some(f.t_node));

        let mut report_opts = speed_opts();
        report_opts.optimizer = OptimizerMode::Report;
        assert!(run_plan(func.clone(), f.interns.clone(), &report_opts, &heap, true).is_empty());

        let mut off_opts = speed_opts();
        off_opts.optimizer = OptimizerMode::Off;
        assert!(run_plan(func.clone(), f.interns.clone(), &off_opts, &heap, true).is_empty());

        let mut size_opts = speed_opts();
        size_opts.profile = Some(Profile::ReleaseSize);
        assert!(run_plan(func.clone(), f.interns.clone(), &size_opts, &heap, true).is_empty());

        let mut disabled_heap = heap;
        disabled_heap.native_vec_enabled = false;
        assert!(run_plan(func, f.interns, &speed_opts(), &disabled_heap, true).is_empty());
    }

    fn replace_type(types: &mut TypeTable, updated: TypeDescriptor) {
        let mut rebuilt = TypeTable::new();
        for d in types.descriptors() {
            if d.id == updated.id {
                rebuilt.insert(updated.clone());
            } else {
                rebuilt.insert(d.clone());
            }
        }
        *types = rebuilt;
    }

    #[test]
    fn rejects_needs_drop_and_interior_mutability_and_pinned_boundary() {
        // 1. needs_drop == true
        {
            let mut f = build_node_fixture();
            let func = build_clean_func(&mut f);
            let mut d = f.interns.types.get(f.t_node).unwrap().clone();
            d.layout.needs_drop = true;
            replace_type(&mut f.interns.types, d);
            let heap = enabled_heap(None);
            assert!(run_plan(func, f.interns, &speed_opts(), &heap, true).is_empty());
        }
        // 2. PinnedBoundary::Host on element struct
        {
            let mut f = build_node_fixture();
            let func = build_clean_func(&mut f);
            let mut d = f.interns.types.get(f.t_node).unwrap().clone();
            d.meta.boundary = PinnedBoundary::Host;
            replace_type(&mut f.interns.types, d);
            let heap = enabled_heap(None);
            assert!(run_plan(func, f.interns, &speed_opts(), &heap, true).is_empty());
        }
        // 3. Interior mutability / transparent wrapper field (`UnsafeCell<u32>`-shaped)
        {
            let mut f = build_node_fixture();
            let t_cell_u32 = TypeId(99);
            let mut cell_d = TypeDescriptor {
                id: t_cell_u32,
                name: None,
                kind: TypeKind::Adt {
                    def: DefinitionId {
                        crate_id: 9,
                        local_hash: 9,
                    },
                    args: vec![f.t_u32],
                    form: NominalForm::Struct,
                    variants: Vec::new(),
                    niche: None,
                },
                layout: LayoutFacts {
                    size: Some(4),
                    align: Some(4),
                    sized: true,
                    ..Default::default()
                },
                meta: TypeMeta::default(),
                js: base_facts(t_cell_u32),
            };
            cell_d.js.transparent = Some(TransparentField {
                field: 0,
                ty: f.t_u32,
            });
            cell_d.js.peeled = f.t_u32;
            f.interns.types.insert(cell_d);
            let mut node_d = f.interns.types.get(f.t_node).unwrap().clone();
            if let TypeKind::Adt {
                ref mut variants, ..
            } = node_d.kind
            {
                variants[0].fields[1].ty = t_cell_u32;
            }
            node_d.js.fields[1].ty = t_cell_u32;
            replace_type(&mut f.interns.types, node_d);
            let func = build_clean_func(&mut f);
            let heap = enabled_heap(None);
            assert!(run_plan(func, f.interns, &speed_opts(), &heap, true).is_empty());
        }
    }

    #[test]
    fn rejects_stack_ref_escape_and_unsizing_and_raw_parts_and_export_and_escaping_bool() {
        let heap = enabled_heap(None);

        // 1. Stack `&Node` borrow (non-SoA `&T`)
        {
            let mut f = build_node_fixture();
            let mut func = build_clean_func(&mut f);
            let c = crate::jir::repr::CellId(func.cells.len() as u32);
            func.cells.push(crate::jir::repr::CellDef {
                ty: f.t_node,
                boxed: false,
                origin: OriginId(0),
            });
            let res = ValueId(func.values.len() as u32);
            func.values.push(crate::jir::repr::ValueDef {
                ty: f.t_ref_node,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            });
            func.blocks[0].ops.push(Op {
                result: Some(res),
                kind: OpKind::BorrowView {
                    root: Root::Cell(c),
                    path: Vec::new(),
                    mutable: false,
                },
                origin: OriginId(0),
            });
            assert!(run_plan(func, f.interns, &speed_opts(), &heap, true).is_empty());
        }

        // 2. Array-to-slice unsizing `[Node; 2] -> &[Node]`
        {
            let mut f = build_node_fixture();
            let mut func = build_clean_func(&mut f);
            let arr_v = ValueId(func.values.len() as u32);
            func.values.push(crate::jir::repr::ValueDef {
                ty: f.t_ref_arr2_node,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            });
            let res = ValueId(func.values.len() as u32);
            func.values.push(crate::jir::repr::ValueDef {
                ty: f.t_ref_slice,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            });
            func.blocks[0].ops.push(Op {
                result: Some(res),
                kind: OpKind::Cast {
                    kind: CastKind::PointerCoercion,
                    v: arr_v,
                    from: f.t_ref_arr2_node,
                    to: f.t_ref_slice,
                },
                origin: OriginId(0),
            });
            assert!(run_plan(func, f.interns, &speed_opts(), &heap, true).is_empty());
        }

        // 3. `NativeVecFromRawParts` / `SliceSort` escape
        {
            let mut f = build_node_fixture();
            let mut func = build_clean_func(&mut f);
            let s_frp = f.interns.intern_str("from_raw_parts");
            let res = ValueId(func.values.len() as u32);
            func.values.push(crate::jir::repr::ValueDef {
                ty: f.t_vec,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            });
            func.blocks[0].ops.push(Op {
                result: Some(res),
                kind: OpKind::Call {
                    callee: Callee::Direct {
                        item: ItemId {
                            definition: DefinitionId {
                                crate_id: 2,
                                local_hash: 99,
                            },
                            instantiation: 0,
                        },
                        link_name: s_frp,
                    },
                    function: None,
                    args: Vec::new(),
                    track_caller: None,
                    semantic: CallSemantic::Standard(StandardOperation::NativeVecFromRawParts),
                    host: None,
                },
                origin: OriginId(0),
            });
            assert!(run_plan(func, f.interns, &speed_opts(), &heap, true).is_empty());
        }

        // 4. Exported signature carrying `Vec<Node>`
        {
            let mut f = build_node_fixture();
            let mut func = build_clean_func(&mut f);
            func.sig.arguments.push(f.t_vec);
            assert!(run_plan(func, f.interns, &speed_opts(), &heap, true).is_empty());
        }

        // 5. Escaping `&bool` field borrow into an external call
        {
            let mut f = build_node_fixture();
            let mut func = build_clean_func(&mut f);
            let s_ext = f.interns.intern_str("takes_bool_ref");
            let bool_view = func.blocks[0]
                .ops
                .iter()
                .find_map(|op| match &op.kind {
                    OpKind::BorrowView { path, .. }
                        if matches!(path.last(), Some(Proj::Field { field: 3, .. })) =>
                    {
                        op.result
                    }
                    _ => None,
                })
                .expect("steiner borrow");
            func.blocks[0].ops.push(Op {
                result: None,
                kind: OpKind::Call {
                    callee: Callee::Runtime {
                        member: s_ext,
                        sysroot: false,
                    },
                    function: None,
                    args: vec![bool_view],
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                origin: OriginId(0),
            });
            assert!(run_plan(func, f.interns, &speed_opts(), &heap, true).is_empty());
        }
    }

    #[test]
    fn end_to_end_soa_lowering_executes_in_node_with_zero_node_allocs() {
        use crate::lower::js::soa::needed_helper_items;
        use crate::lower::js::JsCx;

        let mut f = build_node_fixture();
        let s_vec_new = f.interns.intern_str("rcgjs_vec_new");
        let s_vec_push = f.interns.intern_str("rcgjs_vec_push");
        let s_vec_slice = f.interns.intern_str("rcgjs_vec_as_mut_slice");
        let s_vec_pop = f.interns.intern_str("rcgjs_vec_pop");
        let s_vec_cap = f.interns.intern_str("rcgjs_vec_capacity");
        let s_vec_as_mut_ptr = f.interns.intern_str("rcgjs_vec_as_mut_ptr");
        let s_swap = f.interns.intern_str("typed_swap_nonoverlapping");

        let mut b = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash: 50,
                },
                instantiation: 0,
            },
            Signature {
                arguments: vec![f.t_f64],
                result: f.t_f64,
                abi: AbiSignature {
                    parameters: 1,
                    ..AbiSignature::default()
                },
            },
        );
        let x0 = b.param(f.t_f64, OriginId(0));
        let v_cell = b.cell(f.t_vec, false, OriginId(0));
        b.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(v_cell), OriginId(0));
        let v_new = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 2,
                            local_hash: 1,
                        },
                        instantiation: 0,
                    },
                    link_name: s_vec_new,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeVecNew),
                host: None,
            },
            f.t_vec,
            OriginId(0),
        );
        b.effect(
            FuncBuilder::ENTRY,
            OpKind::CellStore {
                cell: v_cell,
                v: v_new,
            },
            OriginId(0),
        );

        // Push 5 nodes (triggers growth past min_cap = 4!)
        for idx in 0..5u32 {
            let cx = if idx == 0 {
                x0
            } else {
                b.op(
                    FuncBuilder::ENTRY,
                    OpKind::Const(ConstTree::Scalar {
                        bits: (f64::from(idx) * 10.0).to_bits() as u128,
                        bytes: 8,
                    }),
                    f.t_f64,
                    OriginId(0),
                )
            };
            let prev_bits = if idx == 0 {
                u128::from(u32::MAX)
            } else {
                u128::from(idx - 1)
            };
            let c_prev = b.op(
                FuncBuilder::ENTRY,
                OpKind::Const(ConstTree::Scalar {
                    bits: prev_bits,
                    bytes: 4,
                }),
                f.t_u32,
                OriginId(0),
            );
            let c_z = b.op(
                FuncBuilder::ENTRY,
                OpKind::Const(ConstTree::Scalar {
                    bits: u128::from(idx),
                    bytes: 4,
                }),
                f.t_i32,
                OriginId(0),
            );
            let c_b = b.op(
                FuncBuilder::ENTRY,
                OpKind::Const(ConstTree::Scalar {
                    bits: u128::from(idx % 2 == 0),
                    bytes: 1,
                }),
                f.t_bool,
                OriginId(0),
            );
            let node = b.op(
                FuncBuilder::ENTRY,
                OpKind::Aggregate {
                    ty: f.t_node,
                    variant: 0,
                    active_field: None,
                    fields: vec![cx, c_prev, c_z, c_b],
                },
                f.t_node,
                OriginId(0),
            );
            let vm = b.op(
                FuncBuilder::ENTRY,
                OpKind::BorrowView {
                    root: Root::Cell(v_cell),
                    path: Vec::new(),
                    mutable: true,
                },
                f.t_mut_vec,
                OriginId(0),
            );
            b.op(
                FuncBuilder::ENTRY,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: ItemId {
                            definition: DefinitionId {
                                crate_id: 2,
                                local_hash: 2,
                            },
                            instantiation: 0,
                        },
                        link_name: s_vec_push,
                    },
                    function: None,
                    args: vec![vm, node],
                    track_caller: None,
                    semantic: CallSemantic::Standard(StandardOperation::NativeVecPush),
                    host: None,
                },
                f.t_unit,
                OriginId(0),
            );
        }

        // Swap nodes[0] and nodes[1] via typed_swap_nonoverlapping
        let vm_ptr = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Cell(v_cell),
                path: Vec::new(),
                mutable: true,
            },
            f.t_mut_vec,
            OriginId(0),
        );
        let p0 = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 2,
                            local_hash: 4,
                        },
                        instantiation: 0,
                    },
                    link_name: s_vec_as_mut_ptr,
                },
                function: None,
                args: vec![vm_ptr],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeVecAsMutPtr),
                host: None,
            },
            f.t_mut_ptr_node,
            OriginId(0),
        );
        let c1 = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            f.t_u32,
            OriginId(0),
        );
        let p1 = b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: crate::jir::semantic::BinaryOp::Offset,
                l: p0,
                r: c1,
                ty: f.t_mut_ptr_node,
            },
            f.t_mut_ptr_node,
            OriginId(0),
        );
        b.op(
            FuncBuilder::ENTRY,
            OpKind::Intrinsic {
                name: s_swap,
                types: vec![f.t_node],
                fallback: None,
                args: vec![p0, p1],
                track_caller: None,
            },
            f.t_unit,
            OriginId(0),
        );

        // Read back through scalarized local `&Node` (`let np = &slice[1]`) + capacity + pop
        let vm_s = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Cell(v_cell),
                path: Vec::new(),
                mutable: true,
            },
            f.t_mut_vec,
            OriginId(0),
        );
        let slice = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 2,
                            local_hash: 3,
                        },
                        instantiation: 0,
                    },
                    link_name: s_vec_slice,
                },
                function: None,
                args: vec![vm_s],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeVecAsMutSlice),
                host: None,
            },
            f.t_mut_slice,
            OriginId(0),
        );
        // Multi-use local borrow `np = &slice[1]` (original node 0 after swap: x=x0, prev=u32::MAX, steiner=true)
        let np = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(slice),
                path: vec![Proj::Deref, Proj::Index(c1)],
                mutable: false,
            },
            f.t_ref_node,
            OriginId(0),
        );
        let rx_p = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(np),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: 0,
                        ty: f.t_f64,
                    },
                ],
                mutable: false,
            },
            f.t_f64,
            OriginId(0),
        );
        let rx = b.op(
            FuncBuilder::ENTRY,
            OpKind::Deref(rx_p),
            f.t_f64,
            OriginId(0),
        );
        let rprev_p = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(np),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: 1,
                        ty: f.t_u32,
                    },
                ],
                mutable: false,
            },
            f.t_u32,
            OriginId(0),
        );
        let rprev = b.op(
            FuncBuilder::ENTRY,
            OpKind::Deref(rprev_p),
            f.t_u32,
            OriginId(0),
        );
        let rbool_p = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(np),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: 3,
                        ty: f.t_bool,
                    },
                ],
                mutable: false,
            },
            f.t_ref_bool,
            OriginId(0),
        );
        let rbool = b.op(
            FuncBuilder::ENTRY,
            OpKind::Deref(rbool_p),
            f.t_bool,
            OriginId(0),
        );
        let rbool_f = b.op(
            FuncBuilder::ENTRY,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: rbool,
                from: f.t_bool,
                to: f.t_u32,
            },
            f.t_u32,
            OriginId(0),
        );
        let rprev_f = b.op(
            FuncBuilder::ENTRY,
            OpKind::Cast {
                kind: CastKind::IntToFloat,
                v: rprev,
                from: f.t_u32,
                to: f.t_f64,
            },
            f.t_f64,
            OriginId(0),
        );
        let rbool_d = b.op(
            FuncBuilder::ENTRY,
            OpKind::Cast {
                kind: CastKind::IntToFloat,
                v: rbool_f,
                from: f.t_u32,
                to: f.t_f64,
            },
            f.t_f64,
            OriginId(0),
        );
        // Check capacity (should be 8 after pushing 5 items with min_cap=4)
        let vr_cap = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Cell(v_cell),
                path: Vec::new(),
                mutable: false,
            },
            f.t_ref_vec,
            OriginId(0),
        );
        let cap_u = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 2,
                            local_hash: 5,
                        },
                        instantiation: 0,
                    },
                    link_name: s_vec_cap,
                },
                function: None,
                args: vec![vr_cap],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeVecCapacity),
                host: None,
            },
            f.t_u32,
            OriginId(0),
        );
        let cap_f = b.op(
            FuncBuilder::ENTRY,
            OpKind::Cast {
                kind: CastKind::IntToFloat,
                v: cap_u,
                from: f.t_u32,
                to: f.t_f64,
            },
            f.t_f64,
            OriginId(0),
        );
        // Pop last node (idx 4: x = 40.0)
        let vm_pop = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Cell(v_cell),
                path: Vec::new(),
                mutable: true,
            },
            f.t_mut_vec,
            OriginId(0),
        );
        let popped = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 2,
                            local_hash: 6,
                        },
                        instantiation: 0,
                    },
                    link_name: s_vec_pop,
                },
                function: None,
                args: vec![vm_pop],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeVecPop),
                host: None,
            },
            f.t_opt_node,
            OriginId(0),
        );
        let popped_node = b.op(
            FuncBuilder::ENTRY,
            OpKind::Field {
                base: popped,
                variant: Some(1),
                field: 0,
            },
            f.t_node,
            OriginId(0),
        );
        let pop_x = b.op(
            FuncBuilder::ENTRY,
            OpKind::Field {
                base: popped_node,
                variant: None,
                field: 0,
            },
            f.t_f64,
            OriginId(0),
        );
        let s1 = b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: crate::jir::semantic::BinaryOp::Add,
                l: rx,
                r: cap_f,
                ty: f.t_f64,
            },
            f.t_f64,
            OriginId(0),
        );
        let s2 = b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: crate::jir::semantic::BinaryOp::Add,
                l: s1,
                r: pop_x,
                ty: f.t_f64,
            },
            f.t_f64,
            OriginId(0),
        );
        let s3 = b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: crate::jir::semantic::BinaryOp::Add,
                l: s2,
                r: rbool_d,
                ty: f.t_f64,
            },
            f.t_f64,
            OriginId(0),
        );
        // Check rprev == u32::MAX (4294967295.0)
        let s4 = b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: crate::jir::semantic::BinaryOp::Add,
                l: s3,
                r: rprev_f,
                ty: f.t_f64,
            },
            f.t_f64,
            OriginId(0),
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(s4)));
        let func = b.finish();

        let opts = speed_opts();
        let heap = enabled_heap(Some(f.t_node));
        let soa = run_plan(func.clone(), f.interns.clone(), &opts, &heap, true);
        assert!(soa.layouts.contains_key(&f.t_node));

        let plan_fn = crate::lower::fuse::plan(&func, None, &f.interns.types, &f.interns, &opts);
        let structured =
            crate::lower::structure::structure(&func, &plan_fn, &f.interns.types, false)
                .expect("structure");
        let summary = ItemSummary {
            key: crate::jir::summary::SemanticKey {
                link_name: "run_soa".to_owned(),
                item: None,
            },
            body_hash: None,
            semantic_hash: None,
            availability: crate::jir::summary::Availability::Available,
            deps: Vec::new(),
            abi: None,
            physical: None,
            kind: ItemKind::Fn,
            linkage: Linkage::internal(),
            order: None,
            dts: false,
            fallback: rcgjs_object::object::BlobRef {
                offset: 0,
                clen: 0,
                ulen: 0,
            },
            body: None,
            origin: None,
            estimated_bytes: 0,
            class: crate::jir::summary::ItemClass::default(),
        };
        let cx = JsCx::new(
            &func,
            &plan_fn,
            &f.interns.types,
            &f.interns,
            &opts,
            &summary,
            None,
        )
        .with_heap(Some(&heap))
        .with_soa(Some(&soa));
        let (body, _tmps) = crate::lower::js::emit::emit(&cx, structured.region).expect("emit");
        let mut prelude = Vec::new();
        for v in &plan_fn.declared_order {
            if let Some(n) = plan_fn.names.get(v) {
                prelude.push(rcgjs_object::jsast::Stmt::Let(n.clone(), None));
            }
        }
        for n in plan_fn.cell_names.values() {
            prelude.push(rcgjs_object::jsast::Stmt::Let(n.clone(), None));
        }
        for n in cx.temps() {
            prelude.push(rcgjs_object::jsast::Stmt::Let(n, None));
        }
        prelude.extend(body);
        let params = func
            .params
            .iter()
            .map(|v| plan_fn.names.get(v).cloned().unwrap_or_default())
            .collect();
        let decl = rcgjs_object::jsast::function("run_soa".to_owned(), params, prelude);
        let js_fn = rcgjs_object::jsast::stmt_to_string(&decl);

        // Verify SoA columns and zero per-node object literals on push!
        assert!(
            js_fn.contains("$__mires_soa_e0") && js_fn.contains("$__mires_soa_grow_0"),
            "expected SoA empty & grow helpers in:\n{js_fn}"
        );
        // Only `NativeVecPop` materializes a `Node` object (1 occurrence); all 5 pushes write
        // directly to SoA TypedArray columns with zero intermediate Node object allocations, and
        // the multi-use local `&Node` borrow (`np`) is scalarized with zero `{ buf, off }` object.
        assert_eq!(
            js_fn.matches("steiner:").count(),
            1,
            "expected only pop to materialize a Node object:\n{js_fn}"
        );

        let refs: BTreeSet<JsName> = soa.all_helper_names().into_iter().collect();
        let helpers = needed_helper_items(&soa, &refs, &BTreeSet::new());
        let mut program = String::from("const __rt = { js_abort(m) { throw new Error(m); } };\n");
        for h in &helpers {
            program.push_str(&h.js);
        }
        program.push_str(&js_fn);
        program.push_str(
            "\nconst got = run_soa(3.5);\n\
             const expected = 3.5 + 8 + 40 + 1 + 4294967295;\n\
             if (got !== expected) { throw new Error(`mismatch: ${got} !== ${expected}`); }\n\
             console.log('SOA_OK:' + got);\n",
        );
        let output = std::process::Command::new("node")
            .arg("-e")
            .arg(&program)
            .output()
            .expect("run node");
        assert!(
            output.status.success(),
            "node execution failed:\nstdout: {}\nstderr: {}\nprogram:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            program
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("SOA_OK:"),
            "unexpected output: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    #[test]
    fn unrelated_static_vtable_const_and_declined_fn_in_same_crate_do_not_disqualify() {
        let mut f = build_node_fixture();
        let main_func = build_clean_func(&mut f);

        let make_body = |kind: BodyKind, semantic: BodySemantic, result: TypeId| Body {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash: 77,
                },
                instantiation: 1,
            },
            kind,
            semantic,
            signature: Signature {
                arguments: vec![],
                result,
                abi: AbiSignature::default(),
            },
            locals: vec![],
            blocks: vec![],
        };

        let make_extra = |name: &str, kind: ItemKind, body: Option<Body>| {
            let mut item = make_link_item(name, f.interns.clone(), false);
            item.kind = kind;
            item.jir.body = body;
            item
        };

        let mut table = BTreeMap::new();
        let mut funcs: BTreeMap<JsName, Result<Func, Decline>> = BTreeMap::new();
        let mut repr_bodies: BTreeMap<JsName, ()> = BTreeMap::new();
        let mut live = BTreeSet::new();

        let main_name = JsName::new("main".to_owned());
        table.insert(
            main_name.clone(),
            make_link_item("main", f.interns.clone(), false),
        );
        funcs.insert(main_name.clone(), Ok(main_func));
        live.insert(main_name);

        // Unrelated items sharing the exact same crate-wide `Interns` (which contains `Node`):
        for (nm, kind, body) in [
            (
                "unrelated_static",
                ItemKind::Static,
                Some(make_body(
                    BodyKind::Static {
                        init: ConstTree::Unit,
                        mutable: false,
                    },
                    BodySemantic::Ordinary,
                    f.t_u32,
                )),
            ),
            (
                "unrelated_vtable",
                ItemKind::Vtable,
                Some(make_body(
                    BodyKind::Vtable {
                        concrete: f.t_u32,
                        trait_ref: None,
                        entries: vec![],
                    },
                    BodySemantic::Ordinary,
                    f.t_unit,
                )),
            ),
            ("unrelated_const", ItemKind::Const, None),
            ("unrelated_alloc", ItemKind::Alloc, None),
            ("unrelated_import", ItemKind::Import, None),
            (
                "unrelated_declined_fn",
                ItemKind::Fn,
                Some(make_body(
                    BodyKind::Function,
                    BodySemantic::Ordinary,
                    f.t_u32,
                )),
            ),
            (
                "unrelated_repr_body",
                ItemKind::Fn,
                Some(make_body(
                    BodyKind::Function,
                    BodySemantic::Ordinary,
                    f.t_u32,
                )),
            ),
        ] {
            let jn = JsName::new(nm.to_owned());
            table.insert(jn.clone(), make_extra(nm, kind, body));
            if nm == "unrelated_repr_body" {
                repr_bodies.insert(jn.clone(), ());
            }
            live.insert(jn);
        }

        let opts = speed_opts();
        let heap = enabled_heap(Some(f.t_node));
        let p = plan(&table, &funcs, &repr_bodies, &live, &opts, &heap);
        assert!(
            p.layouts.contains_key(&f.t_node),
            "unrelated crate-mates sharing Interns must not disqualify clean Vec<Node>"
        );

        // Now add a `Static` that ACTUALLY holds `Node` -> MUST disqualify!
        let bad_static = JsName::new("node_static".to_owned());
        table.insert(
            bad_static.clone(),
            make_extra(
                "node_static",
                ItemKind::Static,
                Some(make_body(
                    BodyKind::Static {
                        init: ConstTree::Unit,
                        mutable: false,
                    },
                    BodySemantic::Ordinary,
                    f.t_node,
                )),
            ),
        );
        live.insert(bad_static);
        let p_bad = plan(&table, &funcs, &repr_bodies, &live, &opts, &heap);
        assert!(
            !p_bad.layouts.contains_key(&f.t_node),
            "static holding Node must disqualify SoA"
        );
    }
}
