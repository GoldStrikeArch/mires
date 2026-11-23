//! Whole-program heap & box-tracking analysis for Phase 3 (Tier A typed native heap).
//!
//! Computes two conservative whole-program facts over items reachable from the link roots:
//! 1. **Tier A typed-heap legality and raw-alloc reachability** (`typed_heap_enabled`,
//!    `raw_alloc_reachable`, `non_rc_raw_alloc_reachable`):
//!    - `typed_heap_enabled` holds under `Profile::ReleaseSpeed` when the program uses the
//!      default allocator (`AllocatorProvider::Default`) and has zero wide-to-byte aliasing
//!      (`!byte_views.raw_byte_derefs && byte_views.elide_scale_scalar`).
//!    - When `typed_heap_enabled` holds, calls to the 5 typed-heap `Global` hooks
//!      (`rcgjs_typed_alloc`, `rcgjs_typed_realloc`, `rcgjs_typed_dealloc`, `rcgjs_box_new`,
//!      `rcgjs_typed_dealloc_raw`) are lowered directly at call sites without entering their
//!      fallback bodies. Reachability therefore excludes edges into those fallback bodies when
//!      determining whether any remaining live code calls raw `__rust_alloc`/`__rust_dealloc`
//!      (e.g. `Rc<[T]>` or direct user `alloc::alloc::alloc`).
//! 2. **Per-type `_boxes` tracking set** (`any_boxed`, `boxed_defs`, `boxed_non_adt`,
//!    `all_adts_boxed`):
//!    - Tracks which indirect aggregate types can ever have their bare JS object passed to
//!      `__rt.box`, unsized to `dyn Trait`, or inspected through `_boxes` (`unref`,
//!      `tail_ref`/`tail_slice`).
//!    - Every indirect type NOT in this set lowers writes through `__rt._put_plain` (zero
//!      `_blk_get`, zero `_boxes` WeakMap lookup/insert).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use rcgjs_object::jir::decline::Decline;
use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::repr::{Func, OpKind, ValueId};
use rcgjs_object::jir::semantic::{
    BinaryOp, BodySemantic, CallSemantic, Callee, CastKind, DefinitionId,
};
use rcgjs_object::jir::types::{TypeId, TypeKind, TypeTable};
use rcgjs_object::jsast;
use rcgjs_object::late_ir::AllocatorProvider;
use rcgjs_object::object::ItemKind;

use crate::analysis::byte_views::ByteViewPlan;
use crate::item::{JsName, LinkItem};
use crate::lower::js::value;
use crate::opts::{Opts, Profile};
use crate::plan::representation::NativeBody;

/// Whole-program heap and `_boxes` tracking plan passed into JIR-to-JS lowering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeapPlan {
    /// Whether `ReleaseSpeed` Tier A typed native heap lowering is active.
    pub typed_heap_enabled: bool,
    /// Whether any reachable non-hook item reaches raw `__rust_alloc` / `__rust_alloc_zeroed` /
    /// `__rust_realloc` / `__rust_dealloc` (including `alloc::rc` / `alloc::sync` unsized allocs).
    pub raw_alloc_reachable: bool,
    /// Whether any reachable non-hook item outside `alloc::rc` / `alloc::sync` reaches raw
    /// `__rust_alloc` / `__rust_alloc_zeroed` / `__rust_realloc` / `__rust_dealloc`.
    pub non_rc_raw_alloc_reachable: bool,
    /// Whether ANY indirect type in the program can be looked up in `_boxes`.
    pub any_boxed: bool,
    /// Specific ADT `DefinitionId`s whose bare JS objects can be looked up in `_boxes`.
    pub boxed_defs: BTreeSet<DefinitionId>,
    /// Conservative flag for non-ADT indirect types (tuples, arrays, closures).
    pub boxed_non_adt: bool,
    /// Conservative fallback when opaque/declined items may box arbitrary ADTs.
    pub all_adts_boxed: bool,
}

/// Conservative default heap plan (Tier A off, all indirect writes keep `_put` / `_boxes`).
pub static DEFAULT_HEAP_PLAN: HeapPlan = HeapPlan {
    typed_heap_enabled: false,
    raw_alloc_reachable: true,
    non_rc_raw_alloc_reachable: true,
    any_boxed: true,
    boxed_defs: BTreeSet::new(),
    boxed_non_adt: true,
    all_adts_boxed: true,
};

impl Default for HeapPlan {
    /// Conservative default used by standalone unit tests: Tier A off, all indirect writes keep
    /// standard `_put` / `_boxes` behavior.
    fn default() -> Self {
        DEFAULT_HEAP_PLAN.clone()
    }
}

impl HeapPlan {
    /// Whether storing a value of indirect type `ty` into a slot must register it in `_boxes`.
    pub fn needs_box_tracking(&self, table: &TypeTable, ty: TypeId) -> bool {
        if !self.any_boxed {
            return false;
        }
        let peeled = table.peeled(ty);
        match table.get(peeled).map(|d| &d.kind) {
            Some(TypeKind::Adt { def, .. }) => self.all_adts_boxed || self.boxed_defs.contains(def),
            Some(TypeKind::Array { .. }) => false,
            _ => self.boxed_non_adt,
        }
    }
}

fn is_host_delegated_body(item: &LinkItem) -> bool {
    matches!(
        item.jir.body.as_ref().map(|b| b.semantic),
        Some(BodySemantic::Standard(op)) if crate::lower::semantic_ops::num::is_float_from_str(op)
    )
}

/// Computes the conservative whole-program [`HeapPlan`] over items reachable from program roots.
pub fn analyze_reachable(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    native_bodies: &BTreeMap<JsName, NativeBody>,
    opts: &Opts,
    byte_views: ByteViewPlan,
) -> HeapPlan {
    let allocator_provider = table
        .values()
        .fold(AllocatorProvider::Unknown, |acc, item| {
            acc.merge(item.late_ir.artifact_allocator_provider())
        });
    let typed_heap_enabled = opts.profile == Some(Profile::ReleaseSpeed)
        && allocator_provider == AllocatorProvider::Default
        && !byte_views.raw_byte_derefs
        && byte_views.elide_scale_scalar;

    // Two-colour reachability: `(item, from_non_rc)` so we distinguish raw alloc calls that
    // originate solely inside `alloc::rc` / `alloc::sync` (`Rc<[T]>`) from user / general raw
    // allocations (`alloc::alloc::alloc`, `Vec::from_raw_parts`, etc.).
    let mut visited_rc: BTreeSet<JsName> = BTreeSet::new();
    let mut visited_non_rc: BTreeSet<JsName> = BTreeSet::new();
    let mut queue: VecDeque<(JsName, bool)> = VecDeque::new();

    for (name, item) in table {
        if (item.linkage.root || opts.roots.iter().any(|r| r == name.as_str()))
            && visited_non_rc.insert(name.clone())
        {
            queue.push_back((name.clone(), true));
        }
    }

    let mut raw_alloc_reachable = false;
    let mut non_rc_raw_alloc_reachable = false;
    let mut boxed_defs: BTreeSet<DefinitionId> = BTreeSet::new();
    let mut boxed_non_adt = false;
    let mut all_adts_boxed = false;
    let mut scanned_funcs: BTreeSet<JsName> = BTreeSet::new();

    while let Some((name, from_non_rc)) = queue.pop_front() {
        if let Some(body) = native_bodies.get(&name) {
            let mut idents = BTreeSet::new();
            for stmt in &body.statements {
                jsast::visit_idents(stmt, &mut |id| {
                    idents.insert(JsName::new(id.to_owned()));
                });
            }
            for r in idents {
                if table.contains_key(&r) {
                    enqueue(
                        &r,
                        from_non_rc,
                        &mut visited_rc,
                        &mut visited_non_rc,
                        &mut queue,
                    );
                }
            }
            continue;
        }
        let Some(item) = table.get(&name) else {
            continue;
        };
        if item.kind == ItemKind::Alloc {
            if is_raw_alloc_shim_name(name.as_str()) {
                raw_alloc_reachable = true;
                if from_non_rc {
                    non_rc_raw_alloc_reachable = true;
                }
            }
            continue;
        }
        if is_host_delegated_body(item) {
            continue;
        }

        let in_rc_or_sync = is_rc_or_sync_item(item);
        let in_alloc_global_impl = is_global_alloc_impl_item(item);
        let next_non_rc = if in_rc_or_sync {
            false
        } else if in_alloc_global_impl {
            from_non_rc
        } else {
            true
        };

        let func_opt = funcs.get(&name).and_then(|r| r.as_ref().ok());

        if let (Some(func), Some(interns)) = (func_opt, item.jir.interns()) {
            if scanned_funcs.insert(name.clone()) {
                scan_func_for_boxes(
                    func,
                    &interns.types,
                    interns,
                    &mut boxed_defs,
                    &mut boxed_non_adt,
                    &mut all_adts_boxed,
                );
            }
            // Traverse callees from JIR so we skip dead fallback bodies of intercepted
            // `StandardOperation::is_typed_heap()` calls when `typed_heap_enabled` is true.
            for callee in jir_callees(func, interns, typed_heap_enabled) {
                if is_raw_alloc_shim_name(callee.as_str()) {
                    raw_alloc_reachable = true;
                    if next_non_rc {
                        non_rc_raw_alloc_reachable = true;
                    }
                }
                if !table.contains_key(&callee) {
                    continue;
                }
                enqueue(
                    &callee,
                    next_non_rc,
                    &mut visited_rc,
                    &mut visited_non_rc,
                    &mut queue,
                );
            }
            // Also follow non-call references (vtables, statics, fn pointers, drop glue) recorded
            // on the item, excluding the typed-heap fallback hook symbols themselves when
            // `typed_heap_enabled` intercepts them at call sites.
            for r in &item.refs {
                if !table.contains_key(r) {
                    continue;
                }
                if typed_heap_enabled && is_typed_heap_hook_item(table.get(r)) {
                    continue;
                }
                if typed_heap_enabled
                    && is_global_alloc_impl_item_opt(table.get(r))
                    && !in_rc_or_sync
                    && !in_alloc_global_impl
                {
                    // Only follow `Global::alloc_impl_*` / `grow_impl_*` / `shrink_impl_*` if
                    // actually referenced in the JIR body (already covered by `jir_callees`).
                    continue;
                }
                if typed_heap_enabled && is_raw_alloc_shim_name(r.as_str()) {
                    // Only count raw alloc shims that actually appear in `jir_callees` (avoid
                    // stale item.refs from dead branches / pruned hooks).
                    continue;
                }
                enqueue(
                    r,
                    next_non_rc,
                    &mut visited_rc,
                    &mut visited_non_rc,
                    &mut queue,
                );
            }
        } else {
            // Non-JIR or declined item: scan JS conservatively.
            if scanned_funcs.insert(name.clone()) {
                if item.js.contains("__rt.alloc(")
                    || item.js.contains("__rt.alloc_zeroed(")
                    || item.js.contains("__rt.realloc(")
                    || item.js.contains("__rt.dealloc(")
                {
                    raw_alloc_reachable = true;
                    if next_non_rc {
                        non_rc_raw_alloc_reachable = true;
                    }
                }
                if item.js.contains("__rt.box(")
                    || item.js.contains("__rt.unref(")
                    || item.js.contains("__rt.tail_ref(")
                    || item.js.contains("__rt.tail_slice(")
                {
                    all_adts_boxed = true;
                    boxed_non_adt = true;
                }
            }
            for r in &item.refs {
                if !table.contains_key(r) {
                    continue;
                }
                enqueue(
                    r,
                    next_non_rc,
                    &mut visited_rc,
                    &mut visited_non_rc,
                    &mut queue,
                );
            }
        }
    }

    let any_boxed = all_adts_boxed || boxed_non_adt || !boxed_defs.is_empty();
    HeapPlan {
        typed_heap_enabled,
        raw_alloc_reachable,
        non_rc_raw_alloc_reachable,
        any_boxed,
        boxed_defs,
        boxed_non_adt,
        all_adts_boxed,
    }
}

fn enqueue(
    callee: &JsName,
    from_non_rc: bool,
    visited_rc: &mut BTreeSet<JsName>,
    visited_non_rc: &mut BTreeSet<JsName>,
    queue: &mut VecDeque<(JsName, bool)>,
) {
    if from_non_rc {
        if visited_non_rc.insert(callee.clone()) {
            queue.push_back((callee.clone(), true));
        }
    } else if !visited_non_rc.contains(callee) && visited_rc.insert(callee.clone()) {
        queue.push_back((callee.clone(), false));
    }
}

fn is_raw_alloc_shim_name(name: &str) -> bool {
    (name.contains("__rust_alloc") && !name.contains("__rust_alloc_error_handler"))
        || name.contains("__rust_dealloc")
        || name.contains("__rust_realloc")
}

fn is_rc_or_sync_item(item: &LinkItem) -> bool {
    let dp = item.debug_path.as_str();
    dp.starts_with("alloc::rc::")
        || dp.starts_with("alloc::sync::")
        || item.name.as_str().starts_with("alloc$rc$")
        || item.name.as_str().starts_with("alloc$sync$")
}

fn is_global_alloc_impl_item(item: &LinkItem) -> bool {
    let dp = item.debug_path.as_str();
    dp.starts_with("alloc::alloc::") || item.name.as_str().starts_with("alloc$alloc$")
}

fn is_global_alloc_impl_item_opt(item: Option<&LinkItem>) -> bool {
    item.is_some_and(is_global_alloc_impl_item)
}

fn is_typed_heap_hook_item(item: Option<&LinkItem>) -> bool {
    let Some(item) = item else {
        return false;
    };
    if matches!(
        item.jir.body.as_ref().map(|b| b.semantic),
        Some(BodySemantic::Standard(op)) if op.is_typed_heap()
    ) {
        return true;
    }
    let name = item.name.as_str();
    let dp = item.debug_path.as_str();
    name.contains("rcgjs_typed_alloc")
        || name.contains("rcgjs_typed_realloc")
        || name.contains("rcgjs_typed_dealloc")
        || name.contains("rcgjs_box_new")
        || dp.contains("rcgjs_typed_alloc")
        || dp.contains("rcgjs_typed_realloc")
        || dp.contains("rcgjs_typed_dealloc")
        || dp.contains("rcgjs_box_new")
}

fn jir_callees(func: &Func, interns: &Interns, skip_typed_heap_hooks: bool) -> Vec<JsName> {
    let mut out = Vec::new();
    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::Call {
                    callee, semantic, ..
                } => {
                    if skip_typed_heap_hooks
                        && matches!(semantic, CallSemantic::Standard(op) if op.is_typed_heap())
                    {
                        continue;
                    }
                    match callee {
                        Callee::Direct { link_name, .. } => {
                            if let Some(s) = interns.str(*link_name) {
                                out.push(JsName::new(s.to_owned()));
                            }
                        }
                        Callee::Foreign { symbol, .. } => {
                            if let Some(s) = interns.str(*symbol) {
                                out.push(JsName::new(s.to_owned()));
                            }
                        }
                        Callee::Intrinsic {
                            fallback: Some(d), ..
                        } => {
                            if let Some(s) = interns.str(d.link_name) {
                                out.push(JsName::new(s.to_owned()));
                            }
                        }
                        _ => {}
                    }
                }
                OpKind::Intrinsic {
                    fallback: Some(d), ..
                } => {
                    if let Some(s) = interns.str(d.link_name) {
                        out.push(JsName::new(s.to_owned()));
                    }
                }
                _ => {}
            }
        }
    }
    out
}

fn record_boxed_type(
    table: &TypeTable,
    ty: TypeId,
    boxed_defs: &mut BTreeSet<DefinitionId>,
    boxed_non_adt: &mut bool,
    all_adts_boxed: &mut bool,
) {
    let peeled = table.peeled(ty);
    if !table.is_indirect(peeled)
        || !value::is_slot_pointee(table, peeled)
        || matches!(
            table.get(peeled).map(|d| &d.kind),
            Some(TypeKind::Array { .. })
        )
    {
        return;
    }
    match table.get(peeled).map(|d| &d.kind) {
        Some(TypeKind::Adt { def, .. }) => {
            boxed_defs.insert(*def);
        }
        Some(TypeKind::Dynamic { .. }) => {
            // Dynamic pointee itself doesn't name a concrete ADT; concrete sources are recorded at
            // the `CastKind::PointerCoercion` (unsize) site.
        }
        Some(_) => {
            *boxed_non_adt = true;
        }
        None => {
            *all_adts_boxed = true;
            *boxed_non_adt = true;
        }
    }
}

/// Peels pointer/wrapper layers (`&T`, `&mut T`, `*const T`, `*mut T`, `Box<T>`, `Unique<T>`,
/// `NonNull<T>`, `Pin<P>`) down to the underlying concrete pointee type `T`.
fn peel_unsize_pointee(table: &TypeTable, mut ty: TypeId) -> Option<TypeId> {
    for _ in 0..8 {
        let peeled = table.peeled(ty);
        match table.get(peeled).map(|d| &d.kind)? {
            TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
                return Some(table.peeled(*pointee));
            }
            TypeKind::Adt { variants, .. } => {
                if variants.len() != 1 {
                    return Some(peeled);
                }
                let variant = variants.first()?;
                let mut non_zst = variant.fields.iter().filter(|f| !table.is_zst(f.ty));
                let first = non_zst.next()?;
                if non_zst.next().is_some() {
                    return Some(peeled);
                }
                ty = first.ty;
            }
            _ => return Some(peeled),
        }
    }
    None
}

fn target_is_dynamic(table: &TypeTable, ty: TypeId) -> bool {
    let Some(pointee) = peel_unsize_pointee(table, ty) else {
        return false;
    };
    if matches!(
        table.get(pointee).map(|d| &d.kind),
        Some(TypeKind::Dynamic { .. })
    ) {
        return true;
    }
    table.unsized_tail(pointee).is_some_and(|tail| {
        matches!(
            table.get(table.peeled(tail.ty)).map(|d| &d.kind),
            Some(TypeKind::Dynamic { .. })
        )
    })
}

fn val_ty(func: &Func, v: ValueId) -> Option<TypeId> {
    func.values.get(v.0 as usize).map(|d| d.ty)
}

fn scan_func_for_boxes(
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
    boxed_defs: &mut BTreeSet<DefinitionId>,
    boxed_non_adt: &mut bool,
    all_adts_boxed: &mut bool,
) {
    macro_rules! mark {
        ($ty:expr) => {
            record_boxed_type(table, $ty, boxed_defs, boxed_non_adt, all_adts_boxed)
        };
    }

    // Any ADT with an unsized tail (e.g. `RcInner<[T]>`, custom slice-tail DST) may be queried
    // through `__rt.tail_ref` / `__rt.tail_slice` via `_boxes`.
    for v in &func.values {
        if let Some(pt) = value::pointee_of(table, v.ty) {
            if table.unsized_tail(pt).is_some() {
                mark!(pt);
            }
        }
    }

    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::Cast { kind, from, to, .. } => match kind {
                    CastKind::PointerCoercion => {
                        if target_is_dynamic(table, *to) {
                            if let Some(src_pt) = peel_unsize_pointee(table, *from) {
                                mark!(src_pt);
                            } else {
                                *all_adts_boxed = true;
                                *boxed_non_adt = true;
                            }
                        }
                        if let Some(from_pt) = value::pointee_of(table, *from) {
                            let from_is_ref = !value::is_raw(table, *from);
                            let to_is_ref = value::pointee_of(table, *to).is_some()
                                && !value::is_raw(table, *to);
                            if from_is_ref && !to_is_ref {
                                mark!(from_pt);
                            }
                        }
                    }
                    CastKind::PtrToPtr
                    | CastKind::PointerExposeProvenance
                    | CastKind::Transmute => {
                        if let Some(from_pt) = value::pointee_of(table, *from) {
                            let from_is_ref = !value::is_raw(table, *from);
                            let to_pt = value::pointee_of(table, *to);
                            let to_is_ref = to_pt.is_some() && !value::is_raw(table, *to);
                            if from_is_ref && !to_is_ref {
                                mark!(from_pt);
                            }
                            // Type-erased `&T as *const ()` cast back to `*const T` calls
                            // `ref_to_raw(to, value)` (`ptr.rs:896`).
                            if table.is_zst(from_pt) {
                                if let Some(tp) = to_pt {
                                    mark!(tp);
                                }
                            }
                        } else if *kind == CastKind::Transmute {
                            // `niche_to_scalar` can unwrap `Option<&T>` and recurse into
                            // `transmute_scalar(&T, *const T / usize)` (`unsize.rs:361`).
                            if let Some(TypeKind::Adt { variants, .. }) =
                                table.get(table.peeled(*from)).map(|d| &d.kind)
                            {
                                for variant in variants {
                                    for field in &variant.fields {
                                        if !value::is_raw(table, field.ty) {
                                            if let Some(pt) = value::pointee_of(table, field.ty) {
                                                mark!(pt);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                },
                OpKind::RawView { .. } => {
                    if let Some(res) = op.result {
                        if let Some(pt) =
                            val_ty(func, res).and_then(|t| value::pointee_of(table, t))
                        {
                            mark!(pt);
                        }
                    }
                }
                OpKind::SliceView { ptr, .. } => {
                    // `ptr.rs:386` calls `ref_to_raw` when `ptr` is a reference to an indirect
                    // non-array/slice/str pointee.
                    if let Some(dt) = val_ty(func, *ptr) {
                        if matches!(value::kind(table, dt), Some(TypeKind::Reference { .. })) {
                            if let Some(pt) = value::pointee_of(table, dt) {
                                mark!(pt);
                            }
                        }
                    }
                }
                OpKind::Binary {
                    op:
                        BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::Lt
                        | BinaryOp::Le
                        | BinaryOp::Gt
                        | BinaryOp::Ge
                        | BinaryOp::Offset,
                    ty,
                    ..
                } => {
                    if let Some(TypeKind::Reference { pointee, .. }) =
                        table.get(table.peeled(*ty)).map(|d| &d.kind)
                    {
                        mark!(*pointee);
                    }
                }
                OpKind::Intrinsic { name, args, .. } => {
                    if let Some(n) = interns.str(*name) {
                        if matches!(
                            n,
                            "transmute"
                                | "transmute_unchecked"
                                | "ptr_guaranteed_cmp"
                                | "offset_from"
                                | "sub_ptr"
                        ) {
                            for &arg in args {
                                if let Some(dt) = val_ty(func, arg) {
                                    if !value::is_raw(table, dt) {
                                        if let Some(pt) = value::pointee_of(table, dt) {
                                            mark!(pt);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                OpKind::Call {
                    callee: Callee::Runtime { member, .. },
                    ..
                } => {
                    if matches!(
                        interns.str(*member),
                        Some(
                            "box"
                                | "unref"
                                | "tail_ref"
                                | "tail_slice"
                                | "addr"
                                | "ptr_eq"
                                | "ptr_cmp"
                                | "offset_from"
                        )
                    ) {
                        *all_adts_boxed = true;
                        *boxed_non_adt = true;
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgjs_object::jir::origin::OriginId;
    use rcgjs_object::jir::repr::{Carrier, Func, Op, RBlock, RBlockId, RTerm, Root, ValueDef};
    use rcgjs_object::jir::semantic::{
        AbiSignature, Body, BodyKind, BodySemantic, DefinitionId, ItemId, Signature,
        StandardOperation,
    };
    use rcgjs_object::jir::types::{
        FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts, NominalForm,
        PointerForm, TypeDescriptor, TypeMeta, VariantDescriptor,
    };
    use rcgjs_object::late_ir::LateIrAnalysisSummary;
    use rcgjs_object::object::Linkage;

    use crate::item::LinkJir;

    fn make_desc(id: u128, kind: TypeKind, size: u64, is_indirect: bool) -> TypeDescriptor {
        let id = TypeId(id);
        let int = match kind {
            TypeKind::Integer { signed, bits } => Some(IntFacts {
                signed,
                bits,
                repr: if bits > 32 {
                    IntRepr::BigInt
                } else {
                    IntRepr::Number
                },
            }),
            _ => None,
        };
        TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                size: Some(size),
                align: Some(size.max(1)),
                sized: true,
                zst: size == 0,
                ..LayoutFacts::default()
            },
            js: JsFacts {
                int,
                is_indirect,
                needs_clone: is_indirect,
                peeled: id,
                transparent: None,
                crosses_transparent_union: false,
                enum_repr: None,
                is_primitive_enum: false,
                unsized_tail: None,
                byte_element: false,
                pointer_form: None,
                fields: Vec::new(),
            },
            meta: TypeMeta::default(),
        }
    }

    fn make_item(
        name: &str,
        debug_path: &str,
        kind: ItemKind,
        root: bool,
        provider: AllocatorProvider,
        interns: Option<Interns>,
        semantic: BodySemantic,
    ) -> LinkItem {
        let mut jir = LinkJir::default();
        if let Some(interns) = interns {
            jir.use_interns(interns);
            jir.body = Some(Body {
                item: ItemId {
                    definition: DefinitionId {
                        crate_id: 1,
                        local_hash: 1,
                    },
                    instantiation: 1,
                },
                kind: BodyKind::Function,
                semantic,
                signature: Signature {
                    arguments: Vec::new(),
                    result: TypeId(1),
                    abi: AbiSignature::default(),
                },
                locals: Vec::new(),
                blocks: Vec::new(),
            });
        }
        let mut linkage = Linkage::internal();
        linkage.root = root;
        LinkItem {
            name: JsName::new(name.to_owned()),
            kind,
            crate_name: "test".to_owned(),
            debug_path: debug_path.to_owned(),
            js: String::new(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage,
            order: None,
            map: Default::default(),
            dts: None,
            optimization: Default::default(),
            late_ir: LateIrAnalysisSummary::allocator_provider(provider),
            baseline_shadow: Default::default(),
            jir,
        }
    }

    fn empty_func(item_id: u64, ops: Vec<Op>, values: Vec<ValueDef>) -> Func {
        Func {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash: item_id,
                },
                instantiation: 1,
            },
            sig: Signature {
                arguments: Vec::new(),
                result: TypeId(1),
                abi: AbiSignature::default(),
            },
            params: Vec::new(),
            cells: Vec::new(),
            values,
            blocks: vec![RBlock {
                id: RBlockId(0),
                params: Vec::new(),
                ops,
                term: RTerm::Return(None),
            }],
        }
    }

    #[test]
    fn typed_heap_plan_gates_profile_allocator_and_two_colour_rc_reachability() {
        let mut interns = Interns::new();
        let hook_sym = interns.intern_str("alloc$alloc$rcgjs_typed_alloc$h1");
        let rc_sym = interns.intern_str("alloc$rc$allocate_for_layout$h2");
        let raw_alloc_sym = interns.intern_str("_RNvCs1_7___rustc12___rust_alloc");

        let unit_ty = TypeId(1);
        interns
            .types
            .insert(make_desc(1, TypeKind::Tuple(Vec::new()), 0, false));

        // Root calls typed-heap hook (skipped under Tier A) AND `alloc::rc::allocate_for_layout`
        // (RC-coloured), which calls `__rust_alloc`.
        let root_func = empty_func(
            1,
            vec![
                Op {
                    result: None,
                    origin: OriginId(0),
                    kind: OpKind::Call {
                        callee: Callee::Direct {
                            item: ItemId {
                                definition: DefinitionId {
                                    crate_id: 2,
                                    local_hash: 10,
                                },
                                instantiation: 1,
                            },
                            link_name: hook_sym,
                        },
                        function: None,
                        args: Vec::new(),
                        track_caller: None,
                        semantic: CallSemantic::Standard(StandardOperation::TypedAlloc),
                        host: None,
                    },
                },
                Op {
                    result: None,
                    origin: OriginId(0),
                    kind: OpKind::Call {
                        callee: Callee::Direct {
                            item: ItemId {
                                definition: DefinitionId {
                                    crate_id: 2,
                                    local_hash: 11,
                                },
                                instantiation: 1,
                            },
                            link_name: rc_sym,
                        },
                        function: None,
                        args: Vec::new(),
                        track_caller: None,
                        semantic: CallSemantic::Ordinary,
                        host: None,
                    },
                },
            ],
            vec![ValueDef {
                ty: unit_ty,
                origin: OriginId(0),
                carrier: Carrier::Baseline,
            }],
        );
        let rc_func = empty_func(
            2,
            vec![Op {
                result: None,
                origin: OriginId(0),
                kind: OpKind::Call {
                    callee: Callee::Foreign {
                        definition: DefinitionId {
                            crate_id: 3,
                            local_hash: 1,
                        },
                        symbol: raw_alloc_sym,
                    },
                    function: None,
                    args: Vec::new(),
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
            }],
            Vec::new(),
        );
        let hook_func = empty_func(
            3,
            vec![Op {
                result: None,
                origin: OriginId(0),
                kind: OpKind::Call {
                    callee: Callee::Foreign {
                        definition: DefinitionId {
                            crate_id: 3,
                            local_hash: 1,
                        },
                        symbol: raw_alloc_sym,
                    },
                    function: None,
                    args: Vec::new(),
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
            }],
            Vec::new(),
        );

        let mut table = BTreeMap::new();
        let root_name = JsName::new("entry".to_owned());
        let rc_name = JsName::new("alloc$rc$allocate_for_layout$h2".to_owned());
        let hook_name = JsName::new("alloc$alloc$rcgjs_typed_alloc$h1".to_owned());
        let alloc_name = JsName::new("_RNvCs1_7___rustc12___rust_alloc".to_owned());

        table.insert(
            root_name.clone(),
            make_item(
                "entry",
                "app::entry",
                ItemKind::Fn,
                true,
                AllocatorProvider::Default,
                Some(interns.clone()),
                BodySemantic::Ordinary,
            ),
        );
        table.insert(
            rc_name.clone(),
            make_item(
                "alloc$rc$allocate_for_layout$h2",
                "alloc::rc::Rc::allocate_for_layout",
                ItemKind::Fn,
                false,
                AllocatorProvider::Default,
                Some(interns.clone()),
                BodySemantic::Ordinary,
            ),
        );
        table.insert(
            hook_name.clone(),
            make_item(
                "alloc$alloc$rcgjs_typed_alloc$h1",
                "alloc::alloc::rcgjs_typed_alloc",
                ItemKind::Fn,
                false,
                AllocatorProvider::Default,
                Some(interns),
                BodySemantic::Standard(StandardOperation::TypedAlloc),
            ),
        );
        table.insert(
            alloc_name,
            make_item(
                "_RNvCs1_7___rustc12___rust_alloc",
                "__rustc::__rust_alloc",
                ItemKind::Alloc,
                false,
                AllocatorProvider::Default,
                None,
                BodySemantic::Ordinary,
            ),
        );

        let mut funcs = BTreeMap::new();
        funcs.insert(root_name, Ok(root_func));
        funcs.insert(rc_name, Ok(rc_func));
        funcs.insert(hook_name, Ok(hook_func));

        let native = BTreeMap::new();
        let bv_clean = ByteViewPlan {
            raw_byte_derefs: false,
            elide_scale_scalar: true,
        };
        let speed_opts = Opts {
            profile: Some(Profile::ReleaseSpeed),
            ..Opts::default()
        };
        let plan = analyze_reachable(&table, &funcs, &native, &speed_opts, bv_clean);
        assert!(plan.typed_heap_enabled);
        assert!(plan.raw_alloc_reachable);
        assert!(
            !plan.non_rc_raw_alloc_reachable,
            "RC-only raw alloc must leave non_rc_raw_alloc_reachable false"
        );
        assert!(!plan.any_boxed);

        // ReleaseSize disables Tier A and enters the hook fallback -> non_rc_raw_alloc_reachable = true.
        let size_opts = Opts {
            profile: Some(Profile::ReleaseSize),
            ..Opts::default()
        };
        let size_plan = analyze_reachable(&table, &funcs, &native, &size_opts, bv_clean);
        assert!(!size_plan.typed_heap_enabled);
        assert!(size_plan.non_rc_raw_alloc_reachable);

        // Raw byte derefs disable Tier A even under ReleaseSpeed.
        let bv_dirty = ByteViewPlan {
            raw_byte_derefs: true,
            elide_scale_scalar: false,
        };
        assert!(
            !analyze_reachable(&table, &funcs, &native, &speed_opts, bv_dirty).typed_heap_enabled
        );
    }

    #[test]
    fn box_tracking_distinguishes_raw_viewed_adt_from_plain_adt_and_array() {
        let mut interns = Interns::new();
        let x_key = interns.intern_str("x");
        let u32_ty = TypeId(10);
        interns.types.insert(make_desc(
            10,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            4,
            false,
        ));

        let def_boxed = DefinitionId {
            crate_id: 9,
            local_hash: 1,
        };
        let def_plain = DefinitionId {
            crate_id: 9,
            local_hash: 2,
        };
        let boxed_adt = TypeId(20);
        let plain_adt = TypeId(21);
        let arr_ty = TypeId(22);
        let raw_boxed_ty = TypeId(30);
        let raw_arr_ty = TypeId(31);

        let field = FieldDescriptor {
            key: FieldKey::Name(x_key),
            ty: u32_ty,
            offset: 0,
        };
        let variant = VariantDescriptor {
            name: x_key,
            fields: vec![field],
            logical_discr: 0,
            physical_tag: None,
            result_ok: false,
            saved_local_keys: Vec::new(),
        };
        interns.types.insert(make_desc(
            20,
            TypeKind::Adt {
                def: def_boxed,
                form: NominalForm::Struct,
                args: Vec::new(),
                variants: vec![variant.clone()],
                niche: None,
            },
            4,
            true,
        ));
        interns.types.insert(make_desc(
            21,
            TypeKind::Adt {
                def: def_plain,
                form: NominalForm::Struct,
                args: Vec::new(),
                variants: vec![variant],
                niche: None,
            },
            4,
            true,
        ));
        interns.types.insert(make_desc(
            22,
            TypeKind::Array {
                element: u32_ty,
                length: 2,
            },
            8,
            true,
        ));
        let mut rp_boxed = make_desc(
            30,
            TypeKind::RawPointer {
                pointee: boxed_adt,
                mutable: false,
            },
            4,
            false,
        );
        rp_boxed.js.pointer_form = Some(PointerForm::Slot);
        interns.types.insert(rp_boxed);
        let mut rp_arr = make_desc(
            31,
            TypeKind::RawPointer {
                pointee: arr_ty,
                mutable: false,
            },
            4,
            false,
        );
        rp_arr.js.pointer_form = Some(PointerForm::Slot);
        interns.types.insert(rp_arr);

        let func = empty_func(
            1,
            vec![
                Op {
                    result: Some(ValueId(0)),
                    origin: OriginId(0),
                    kind: OpKind::RawView {
                        root: Root::Value(ValueId(0)),
                        path: Vec::new(),
                        mutable: false,
                    },
                },
                Op {
                    result: Some(ValueId(1)),
                    origin: OriginId(0),
                    kind: OpKind::RawView {
                        root: Root::Value(ValueId(1)),
                        path: Vec::new(),
                        mutable: false,
                    },
                },
            ],
            vec![
                ValueDef {
                    ty: raw_boxed_ty,
                    origin: OriginId(0),
                    carrier: Carrier::Baseline,
                },
                ValueDef {
                    ty: raw_arr_ty,
                    origin: OriginId(0),
                    carrier: Carrier::Baseline,
                },
            ],
        );

        let mut boxed_defs = BTreeSet::new();
        let mut boxed_non_adt = false;
        let mut all_adts_boxed = false;
        scan_func_for_boxes(
            &func,
            &interns.types,
            &interns,
            &mut boxed_defs,
            &mut boxed_non_adt,
            &mut all_adts_boxed,
        );
        let plan = HeapPlan {
            typed_heap_enabled: true,
            raw_alloc_reachable: false,
            non_rc_raw_alloc_reachable: false,
            any_boxed: all_adts_boxed || boxed_non_adt || !boxed_defs.is_empty(),
            boxed_defs,
            boxed_non_adt,
            all_adts_boxed,
        };
        assert!(plan.needs_box_tracking(&interns.types, boxed_adt));
        assert!(!plan.needs_box_tracking(&interns.types, plain_adt));
        assert!(
            !plan.boxed_non_adt,
            "TypeKind::Array is not a _boxes slot pointee"
        );
    }
}
