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
//! 2. **Per-type `_boxes` tracking set** (`any_boxed`, `boxed_defs`, `boxed_non_adt_keys`,
//!    `all_non_adt_boxed`, `all_adts_boxed`):
//!    - Tracks which indirect aggregate types can ever have their bare JS object passed to
//!      `__rt.box`, unsized to `dyn Trait`, or inspected through `_boxes` (`unref`,
//!      `tail_ref`/`tail_slice`).
//!    - Every indirect type NOT in this set lowers writes through direct stores (zero
//!      `_blk_get`, zero `_boxes` WeakMap lookup/insert).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use rcgjs_object::jir::decline::Decline;
use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::repr::{Carrier, Func, OpKind, Proj, RTerm, Root, ValueId};
use rcgjs_object::jir::semantic::{
    BinaryOp, BodySemantic, CallSemantic, Callee, CastKind, ConstTree, DefinitionId, DropGlue,
};
use rcgjs_object::jir::types::{RepresentationCandidate, TypeId, TypeKind, TypeTable};
use rcgjs_object::jsast;
use rcgjs_object::late_ir::AllocatorProvider;
use rcgjs_object::object::ItemKind;

use crate::analysis::byte_views::ByteViewPlan;
use crate::item::{JsName, LinkItem};
use crate::lower::js::value;
use crate::opts::{Opts, Profile};
use crate::plan::representation::NativeBody;

/// Shallow, lifetime-independent structural key for elements of non-ADT indirect aggregates
/// (tuples) tracked in `_boxes`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NonAdtElemKey {
    Adt(DefinitionId),
    Int(bool, u32),
    Float(u32),
    Bool,
    Char,
    Str,
    Ref,
    Raw,
    Array(u64),
    Slice,
    Tuple(usize),
    Closure(DefinitionId),
    Coroutine(DefinitionId),
    CoroutineClosure(DefinitionId),
    Other,
}

/// Structural key for non-ADT indirect types tracked in `_boxes`, preventing a boxed tuple or
/// closure of one shape from conservatively poisoning unrelated tuple `Vec`s in the same crate.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NonAdtBoxKey {
    Tuple(Vec<NonAdtElemKey>),
    Closure(DefinitionId),
    Coroutine(DefinitionId),
    CoroutineClosure(DefinitionId),
    Other,
}

/// Whole-program heap and `_boxes` tracking plan passed into JIR-to-JS lowering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeapPlan {
    /// Whether `ReleaseSpeed` Tier A typed native heap lowering is active.
    pub typed_heap_enabled: bool,
    /// Whether `ReleaseSpeed` + `js-str=bytes` universal native `{ s, cap }` `String` is active
    /// (proven zero direct `.vec` / `Aggregate` / `Transmute` access to `String` internals).
    pub native_string_enabled: bool,
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
    /// Specific non-ADT structural shapes (tuples, closures, coroutines) tracked in `_boxes`.
    pub boxed_non_adt_keys: BTreeSet<NonAdtBoxKey>,
    /// Conservative fallback when opaque/declined items may box arbitrary non-ADTs.
    pub all_non_adt_boxed: bool,
    /// Conservative fallback when opaque/declined items may box arbitrary ADTs.
    pub all_adts_boxed: bool,
}

/// Conservative default heap plan (Tier A off, all indirect writes keep `_put` / `_boxes`).
pub static DEFAULT_HEAP_PLAN: HeapPlan = HeapPlan {
    typed_heap_enabled: false,
    native_string_enabled: false,
    raw_alloc_reachable: true,
    non_rc_raw_alloc_reachable: true,
    any_boxed: true,
    boxed_defs: BTreeSet::new(),
    boxed_non_adt_keys: BTreeSet::new(),
    all_non_adt_boxed: true,
    all_adts_boxed: true,
};

impl Default for HeapPlan {
    /// Conservative default used by standalone unit tests: Tier A off, all indirect writes keep
    /// standard `_put` / `_boxes` behavior.
    fn default() -> Self {
        DEFAULT_HEAP_PLAN.clone()
    }
}

fn non_adt_elem_key(table: &TypeTable, ty: TypeId) -> NonAdtElemKey {
    let peeled = table.peeled(ty);
    match table.get(peeled).map(|d| &d.kind) {
        Some(TypeKind::Adt { def, .. }) => NonAdtElemKey::Adt(*def),
        Some(TypeKind::Integer { signed, bits }) => NonAdtElemKey::Int(*signed, *bits),
        Some(TypeKind::Float { bits }) => NonAdtElemKey::Float(*bits),
        Some(TypeKind::Bool) => NonAdtElemKey::Bool,
        Some(TypeKind::Char) => NonAdtElemKey::Char,
        Some(TypeKind::Str) => NonAdtElemKey::Str,
        Some(TypeKind::Reference { .. }) => NonAdtElemKey::Ref,
        Some(TypeKind::RawPointer { .. }) => NonAdtElemKey::Raw,
        Some(TypeKind::Array { length, .. }) => NonAdtElemKey::Array(*length),
        Some(TypeKind::Slice(_)) => NonAdtElemKey::Slice,
        Some(TypeKind::Tuple(elems)) => NonAdtElemKey::Tuple(elems.len()),
        Some(TypeKind::Closure { def, .. }) => NonAdtElemKey::Closure(*def),
        Some(TypeKind::Coroutine { def, .. }) => NonAdtElemKey::Coroutine(*def),
        Some(TypeKind::CoroutineClosure { def, .. }) => NonAdtElemKey::CoroutineClosure(*def),
        _ => NonAdtElemKey::Other,
    }
}

fn non_adt_box_key(table: &TypeTable, peeled: TypeId) -> Option<NonAdtBoxKey> {
    match table.get(peeled).map(|d| &d.kind)? {
        TypeKind::Tuple(elems) => Some(NonAdtBoxKey::Tuple(
            elems.iter().map(|e| non_adt_elem_key(table, *e)).collect(),
        )),
        TypeKind::Closure { def, .. } => Some(NonAdtBoxKey::Closure(*def)),
        TypeKind::Coroutine { def, .. } => Some(NonAdtBoxKey::Coroutine(*def)),
        TypeKind::CoroutineClosure { def, .. } => Some(NonAdtBoxKey::CoroutineClosure(*def)),
        _ => Some(NonAdtBoxKey::Other),
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
            _ => {
                self.all_non_adt_boxed
                    || non_adt_box_key(table, peeled)
                        .is_none_or(|key| self.boxed_non_adt_keys.contains(&key))
            }
        }
    }
}

fn is_host_delegated_body(item: &LinkItem) -> bool {
    matches!(
        item.jir.body.as_ref().map(|b| b.semantic),
        Some(BodySemantic::Standard(op)) if crate::lower::semantic_ops::num::is_float_from_str(op)
    )
}

/// Filters `live` down to items reachable when intercepted Tier A typed-heap / native-`String` /
/// fast-`str` fallback hooks are excluded, so dead hook bodies (e.g. `rcgjs_str_push` calling
/// `Vec<u8>::push`) do not pin `RawVec` alive in the representation planner's dependency graph.
pub fn filter_live_for_repr(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    live: &BTreeSet<JsName>,
    opts: &Opts,
    heap: &HeapPlan,
) -> BTreeSet<JsName> {
    let skip_typed = heap.typed_heap_enabled;
    let skip_nstr = heap.native_string_enabled;
    if !skip_typed && !skip_nstr {
        return live.clone();
    }
    let is_skipped_hook = |item: Option<&LinkItem>| {
        (skip_typed && is_typed_heap_hook_item(item))
            || (skip_nstr && (is_native_string_hook_item(item) || is_string_drop_glue(item)))
    };

    let mut visited: BTreeSet<JsName> = BTreeSet::new();
    let mut queue: VecDeque<JsName> = VecDeque::new();
    for name in live {
        let Some(item) = table.get(name) else {
            continue;
        };
        if (item.linkage.root
            || item.linkage.exported
            || opts.roots.iter().any(|r| r == name.as_str()))
            && !is_skipped_hook(Some(item))
            && visited.insert(name.clone())
        {
            queue.push_back(name.clone());
        }
    }
    if visited.is_empty() {
        // Standalone test graphs with no root/exported flags: strip only the hook items.
        return live
            .iter()
            .filter(|name| !is_skipped_hook(table.get(*name)))
            .cloned()
            .collect();
    }
    while let Some(name) = queue.pop_front() {
        let Some(item) = table.get(&name) else {
            continue;
        };
        if is_skipped_hook(Some(item)) || is_host_delegated_body(item) {
            continue;
        }
        let func_opt = funcs.get(&name).and_then(|r| r.as_ref().ok());
        if let (Some(func), Some(interns)) = (func_opt, item.jir.interns()) {
            for callee in jir_callees(func, interns, skip_typed, skip_nstr) {
                if live.contains(&callee)
                    && !is_skipped_hook(table.get(&callee))
                    && visited.insert(callee.clone())
                {
                    queue.push_back(callee);
                }
            }
            for r in &item.refs {
                if !live.contains(r) || is_skipped_hook(table.get(r)) {
                    continue;
                }
                if skip_typed
                    && (is_global_alloc_impl_item_opt(table.get(r))
                        || is_raw_alloc_shim_name(r.as_str()))
                    && !is_rc_or_sync_item(item)
                    && !is_global_alloc_impl_item(item)
                {
                    continue;
                }
                if visited.insert(r.clone()) {
                    queue.push_back(r.clone());
                }
            }
        } else {
            for r in &item.refs {
                if live.contains(r) && !is_skipped_hook(table.get(r)) && visited.insert(r.clone()) {
                    queue.push_back(r.clone());
                }
            }
        }
    }
    visited
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
    let try_native_string = typed_heap_enabled && crate::lower::bstr::enabled();

    let (plan, string_internally_accessed) = run_reachability(
        table,
        funcs,
        native_bodies,
        opts,
        typed_heap_enabled,
        try_native_string,
    );
    if try_native_string && !string_internally_accessed {
        plan
    } else if try_native_string {
        // Sound fallback: a reachable non-hook body accesses `String.vec` / `Aggregate` /
        // `Transmute`, so disable native String and re-run reachability over the fallback `Vec<u8>`
        // bodies of `rcgjs_str_*` and `DropGlue(String)`.
        run_reachability(table, funcs, native_bodies, opts, typed_heap_enabled, false).0
    } else {
        plan
    }
}

fn run_reachability(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    native_bodies: &BTreeMap<JsName, NativeBody>,
    opts: &Opts,
    typed_heap_enabled: bool,
    skip_native_string: bool,
) -> (HeapPlan, bool) {
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
    let mut boxed = BoxedSet::default();
    let mut copy_one_boxed = BoxedSet::default();
    let mut string_internally_accessed = false;
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
        if skip_native_string
            && (is_native_string_hook_item(Some(item)) || is_string_drop_glue(Some(item)))
        {
            continue;
        }
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
        let is_drop_glue = matches!(
            item.jir.body.as_ref().map(|b| b.semantic),
            Some(BodySemantic::DropGlue(_))
        );
        let next_non_rc = if in_rc_or_sync {
            false
        } else if in_alloc_global_impl || is_drop_glue {
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
                    &mut boxed,
                    &mut copy_one_boxed,
                );
                if skip_native_string && func_uses_string_internals(func, &interns.types, interns) {
                    string_internally_accessed = true;
                }
            }
            // Traverse callees from JIR so we skip dead fallback bodies of intercepted
            // `StandardOperation::is_typed_heap()` / `is_native_string()` calls when enabled.
            for callee in jir_callees(func, interns, typed_heap_enabled, skip_native_string) {
                if skip_native_string && is_string_drop_glue(table.get(&callee)) {
                    continue;
                }
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
            // on the item, excluding intercepted fallback hook symbols.
            for r in &item.refs {
                if !table.contains_key(r) {
                    continue;
                }
                if typed_heap_enabled && is_typed_heap_hook_item(table.get(r)) {
                    continue;
                }
                if skip_native_string
                    && (is_native_string_hook_item(table.get(r))
                        || is_string_drop_glue(table.get(r)))
                {
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
                    boxed.all_adts_boxed = true;
                    boxed.all_non_adt_boxed = true;
                }
                if skip_native_string
                    && item.kind == ItemKind::Fn
                    && (item.debug_path.contains("alloc::string")
                        || item.name.as_str().contains("alloc$string$")
                        || item.js.contains(".vec"))
                {
                    string_internally_accessed = true;
                }
            }
            for r in &item.refs {
                if !table.contains_key(r) {
                    continue;
                }
                if skip_native_string
                    && (is_native_string_hook_item(table.get(r))
                        || is_string_drop_glue(table.get(r)))
                {
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

    if !typed_heap_enabled || non_rc_raw_alloc_reachable {
        boxed.merge(copy_one_boxed);
    }
    let any_boxed = boxed.all_adts_boxed
        || boxed.all_non_adt_boxed
        || !boxed.boxed_defs.is_empty()
        || !boxed.boxed_non_adt_keys.is_empty();
    (
        HeapPlan {
            typed_heap_enabled,
            native_string_enabled: skip_native_string && !string_internally_accessed,
            raw_alloc_reachable,
            non_rc_raw_alloc_reachable,
            any_boxed,
            boxed_defs: boxed.boxed_defs,
            boxed_non_adt_keys: boxed.boxed_non_adt_keys,
            all_non_adt_boxed: boxed.all_non_adt_boxed,
            all_adts_boxed: boxed.all_adts_boxed,
        },
        string_internally_accessed,
    )
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

fn is_native_string_hook_item(item: Option<&LinkItem>) -> bool {
    let Some(item) = item else {
        return false;
    };
    if matches!(
        item.jir.body.as_ref().map(|b| b.semantic),
        Some(BodySemantic::Standard(op)) if op.is_native_string()
    ) {
        return true;
    }
    let name = item.name.as_str();
    let dp = item.debug_path.as_str();
    let matches_any = |s: &str| {
        s.contains("rcgjs_str_new")
            || s.contains("rcgjs_str_with_capacity")
            || s.contains("rcgjs_str_from_str")
            || s.contains("rcgjs_str_as_str")
            || s.contains("rcgjs_str_len")
            || s.contains("rcgjs_str_capacity")
            || s.contains("rcgjs_str_push")
            || s.contains("rcgjs_str_clear")
            || s.contains("rcgjs_str_truncate_unchecked")
            || s.contains("rcgjs_str_reserve")
            || s.contains("rcgjs_str_shrink_to")
            || s.contains("rcgjs_str_from_vec_unchecked")
            || s.contains("rcgjs_str_into_bytes")
            || s.contains("rcgjs_str_replace_slice")
            || s.contains("rcgjs_str_extend_within")
            || s.contains("rcgjs_str_clone_from")
            || s.contains("rcgjs_str_to_ascii_")
            || s.contains("rcgjs_str_repeat")
    };
    matches_any(name) || matches_any(dp)
}

fn is_string_ty(table: &TypeTable, ty: TypeId) -> bool {
    table
        .get(table.peeled(ty))
        .is_some_and(|d| d.meta.candidate == RepresentationCandidate::String)
}

/// Whether `ty` is *directly* `alloc::string::String` without peeling `#[repr(transparent)]`
/// wrappers (`MaybeUninit<String>`, `ManuallyDrop<String>`, newtype structs). Used when
/// detecting direct `.vec` field access, `String` aggregate construction, or `String` drop glue
/// so wrapper operations (like `MaybeUninit::<String>::write`) do not falsely block native String.
fn is_direct_string_ty(table: &TypeTable, ty: TypeId) -> bool {
    table
        .get(ty)
        .is_some_and(|d| d.meta.candidate == RepresentationCandidate::String)
}

fn is_string_drop_glue(item: Option<&LinkItem>) -> bool {
    let Some(item) = item else {
        return false;
    };
    match item.jir.body.as_ref().map(|b| b.semantic) {
        Some(BodySemantic::Standard(
            rcgjs_object::jir::semantic::StandardOperation::StringDrop,
        )) => true,
        Some(BodySemantic::DropGlue(ty)) => item
            .jir
            .interns()
            .is_some_and(|interns| is_direct_string_ty(&interns.types, ty)),
        _ => false,
    }
}

fn root_ty(func: &Func, root: &Root) -> Option<TypeId> {
    match root {
        Root::Value(v) => func.values.get(v.0 as usize).map(|d| d.ty),
        Root::Cell(c) => func.cells.get(c.0 as usize).map(|d| d.ty),
    }
}

fn place_ty_and_string_field_proj(
    func: &Func,
    table: &TypeTable,
    root: &Root,
    path: &[Proj],
) -> (Option<TypeId>, bool) {
    let Some(mut cur) = root_ty(func, root) else {
        return (None, false);
    };
    for proj in path {
        let peeled = table.peeled(cur);
        match proj {
            Proj::Deref => {
                let Some(pt) = value::pointee_of(table, peeled) else {
                    return (None, false);
                };
                cur = pt;
            }
            Proj::Downcast { .. } => {}
            Proj::Field { ty, .. } => {
                if is_direct_string_ty(table, cur) {
                    return (None, true);
                }
                cur = *ty;
            }
            Proj::Index(_) | Proj::ConstantIndex { .. } => {
                let next = match table.get(peeled).map(|d| &d.kind) {
                    Some(TypeKind::Array { element, .. } | TypeKind::Slice(element)) => {
                        Some(*element)
                    }
                    _ => None,
                };
                let Some(next) = next else {
                    return (None, false);
                };
                cur = next;
            }
            Proj::Subslice { .. } => {}
        }
    }
    (Some(cur), false)
}

/// Whether `ty` contains `String` directly by value (not behind a reference or raw pointer).
fn ty_contains_string_by_value(table: &TypeTable, ty: TypeId, seen: &mut BTreeSet<TypeId>) -> bool {
    let peeled = table.peeled(ty);
    if !seen.insert(peeled) {
        return false;
    }
    if is_string_ty(table, peeled) {
        return true;
    }
    let Some(desc) = table.get(peeled) else {
        return false;
    };
    match &desc.kind {
        TypeKind::Array { element, .. } | TypeKind::Slice(element) => {
            ty_contains_string_by_value(table, *element, seen)
        }
        TypeKind::Tuple(elems) => elems
            .iter()
            .any(|e| ty_contains_string_by_value(table, *e, seen)),
        TypeKind::Adt { variants, .. } => variants.iter().any(|v| {
            v.fields
                .iter()
                .any(|f| ty_contains_string_by_value(table, f.ty, seen))
        }),
        TypeKind::Closure { upvars, .. } | TypeKind::CoroutineClosure { upvars, .. } => upvars
            .iter()
            .any(|u| ty_contains_string_by_value(table, *u, seen)),
        _ => false,
    }
}

/// Peels pointer/ref/single-field-wrapper layers (`&T`, `&mut T`, `*const T`, `*mut T`,
/// `Box<T>`, `Unique<T>`, `NonNull<T>`, `Pin<P>`) down to the pointee `T`. Returns `None` if
/// `ty` is not a reference, raw pointer, or single-field wrapper around one (e.g. `usize` in
/// `NonNull::dangling()`).
fn peel_ptr_pointee(table: &TypeTable, mut ty: TypeId) -> Option<TypeId> {
    for _ in 0..8 {
        let peeled = table.peeled(ty);
        match table.get(peeled).map(|d| &d.kind)? {
            TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
                return Some(table.peeled(*pointee));
            }
            TypeKind::Adt { variants, .. } => {
                if variants.len() != 1 {
                    return None;
                }
                let variant = variants.first()?;
                let mut non_zst = variant.fields.iter().filter(|f| !table.is_zst(f.ty));
                let first = non_zst.next()?;
                if non_zst.next().is_some() {
                    return None;
                }
                ty = first.ty;
            }
            _ => return None,
        }
    }
    None
}

/// Whether transmuting `from -> to` could observe or forge `String`'s private `.vec` layout.
fn transmute_exposes_string_layout(table: &TypeTable, from: TypeId, to: TypeId) -> bool {
    let f = table.peeled(from);
    let t = table.peeled(to);
    if f == t {
        return false;
    }
    let mut seen = BTreeSet::new();
    if ty_contains_string_by_value(table, f, &mut seen) {
        return true;
    }
    seen.clear();
    if ty_contains_string_by_value(table, t, &mut seen) {
        return true;
    }
    // Check pointer/ref transmute between `&String`/`*const String` and a different concrete
    // non-ZST pointee. Integer <-> pointer transmutes (e.g. `NonNull::<String>::dangling()`) have
    // `None` on the integer side and do not expose `String`'s private `.vec` fields.
    let pt_f = peel_ptr_pointee(table, f);
    let pt_t = peel_ptr_pointee(table, t);
    match (pt_f, pt_t) {
        (Some(a), Some(b)) if a != b => {
            let a_str = is_string_ty(table, a);
            let b_str = is_string_ty(table, b);
            (a_str && !table.is_zst(b)) || (b_str && !table.is_zst(a))
        }
        _ => false,
    }
}

fn const_is_zero_vec_string(tree: &ConstTree) -> bool {
    match tree {
        ConstTree::Unit => true,
        ConstTree::Scalar { bits, .. } => *bits == 0,
        ConstTree::Address(_) => true,
        ConstTree::Aggregate(fields) => fields.iter().all(const_is_zero_vec_string),
        _ => false,
    }
}

fn const_has_nonempty_string(table: &TypeTable, tree: &ConstTree, ty: TypeId) -> bool {
    let peeled = table.peeled(ty);
    if is_string_ty(table, peeled) {
        return !const_is_zero_vec_string(tree);
    }
    match (tree, table.get(peeled).map(|d| &d.kind)) {
        (ConstTree::Aggregate(fields), Some(TypeKind::Adt { variants, .. })) => {
            variants.first().is_some_and(|v| {
                fields
                    .iter()
                    .zip(&v.fields)
                    .any(|(f, fd)| const_has_nonempty_string(table, f, fd.ty))
            })
        }
        (ConstTree::Aggregate(fields), Some(TypeKind::Tuple(elems))) => fields
            .iter()
            .zip(elems)
            .any(|(f, et)| const_has_nonempty_string(table, f, *et)),
        (ConstTree::Aggregate(fields), Some(TypeKind::Array { element, .. })) => fields
            .iter()
            .any(|f| const_has_nonempty_string(table, f, *element)),
        (ConstTree::Variant { variant, fields }, Some(TypeKind::Adt { variants, .. })) => {
            variants.get(*variant as usize).is_some_and(|v| {
                fields
                    .iter()
                    .zip(&v.fields)
                    .any(|(f, fd)| const_has_nonempty_string(table, f, fd.ty))
            })
        }
        (
            ConstTree::Reference(inner),
            Some(TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. }),
        ) => const_has_nonempty_string(table, inner, *pointee),
        _ => false,
    }
}

/// Whether `OpKind::Drop { root, path, .. }` drops a native `String` directly or drops its
/// rustc-elaborated `.vec` field (both are no-ops under universal native `{ s, cap }` `String`).
pub fn drop_targets_native_string(
    func: &Func,
    table: &TypeTable,
    root: &Root,
    path: &[Proj],
) -> bool {
    let (ty, crosses_string_field) = place_ty_and_string_field_proj(func, table, root, path);
    crosses_string_field || ty.is_some_and(|t| is_direct_string_ty(table, t))
}

/// Returns `true` if `func` directly accesses `String`'s private `.vec` field, constructs a
/// `String` aggregate literal, transmutes a type mentioning `String`, or embeds a non-empty
/// constant `String`.
fn func_uses_string_internals(func: &Func, table: &TypeTable, interns: &Interns) -> bool {
    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::Field { base, .. } | OpKind::SetField { base, .. } => {
                    if val_ty(func, *base).is_some_and(|ty| is_direct_string_ty(table, ty)) {
                        return true;
                    }
                }
                OpKind::BorrowView { root, path, .. } | OpKind::RawView { root, path, .. } => {
                    if place_ty_and_string_field_proj(func, table, root, path).1 {
                        return true;
                    }
                }
                OpKind::Aggregate { ty, .. } => {
                    if is_direct_string_ty(table, *ty) {
                        return true;
                    }
                }
                OpKind::Cast {
                    kind: CastKind::Transmute,
                    from,
                    to,
                    ..
                } => {
                    if transmute_exposes_string_layout(table, *from, *to) {
                        return true;
                    }
                }
                OpKind::Intrinsic { name, types, .. } => {
                    if matches!(
                        interns.str(*name),
                        Some("transmute" | "transmute_unchecked")
                    ) {
                        if let [from, to] = types.as_slice() {
                            if transmute_exposes_string_layout(table, *from, *to) {
                                return true;
                            }
                        } else {
                            for &t in types {
                                let mut seen = BTreeSet::new();
                                if ty_contains_string_by_value(table, t, &mut seen) {
                                    return true;
                                }
                            }
                        }
                    }
                }
                OpKind::Const(tree) => {
                    if let Some(res) = op.result {
                        if let Some(ty) = val_ty(func, res) {
                            if const_has_nonempty_string(table, tree, ty) {
                                return true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    false
}

fn jir_callees(
    func: &Func,
    interns: &Interns,
    skip_typed_heap_hooks: bool,
    skip_native_string: bool,
) -> Vec<JsName> {
    let mut out = Vec::new();
    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::Call {
                    callee, semantic, ..
                } => {
                    if skip_typed_heap_hooks
                        && matches!(
                            semantic,
                            CallSemantic::Standard(op)
                                if op.is_typed_heap() || op.is_fast_str_op()
                        )
                    {
                        continue;
                    }
                    if skip_native_string
                        && matches!(semantic, CallSemantic::Standard(op) if op.is_native_string())
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
                OpKind::Drop {
                    root,
                    path,
                    glue: DropGlue::Direct { link_name, .. },
                } => {
                    if skip_native_string
                        && drop_targets_native_string(func, &interns.types, root, path)
                    {
                        continue;
                    }
                    if let Some(s) = interns.str(*link_name) {
                        out.push(JsName::new(s.to_owned()));
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

#[derive(Default)]
struct BoxedSet {
    boxed_defs: BTreeSet<DefinitionId>,
    boxed_non_adt_keys: BTreeSet<NonAdtBoxKey>,
    all_non_adt_boxed: bool,
    all_adts_boxed: bool,
}

impl BoxedSet {
    fn merge(&mut self, other: BoxedSet) {
        self.boxed_defs.extend(other.boxed_defs);
        self.boxed_non_adt_keys.extend(other.boxed_non_adt_keys);
        self.all_non_adt_boxed |= other.all_non_adt_boxed;
        self.all_adts_boxed |= other.all_adts_boxed;
    }

    fn record(&mut self, table: &TypeTable, ty: TypeId) {
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
                self.boxed_defs.insert(*def);
            }
            Some(TypeKind::Dynamic { .. }) => {
                // Dynamic pointee itself doesn't name a concrete ADT; concrete sources are recorded
                // at the `CastKind::PointerCoercion` (unsize) site.
            }
            Some(_) => match non_adt_box_key(table, peeled) {
                Some(key) => {
                    self.boxed_non_adt_keys.insert(key);
                }
                None => {
                    self.all_non_adt_boxed = true;
                }
            },
            None => {
                self.all_adts_boxed = true;
                self.all_non_adt_boxed = true;
            }
        }
    }
}

/// Returns the set of `OpKind::Cast` result `ValueId`s (from `&T`/`&mut T` to `*const T`/`*mut T`
/// with the exact same indirect non-array non-DST pointee `T`) whose sole use in `func` is as the
/// `src` operand of a constant-1 `copy`/`copy_nonoverlapping` intrinsic.
///
/// Under Tier A without non-RC raw allocs, those casts can pass the bare reference directly into
/// the single-element copy without calling `__rt.box(o)` (avoiding `_boxes` WeakMap tracking for
/// `T` across the entire program).
pub fn direct_ref_copy_one_casts(
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
) -> BTreeSet<ValueId> {
    let mut use_counts: BTreeMap<ValueId, u32> = BTreeMap::new();
    let mut defs: BTreeMap<ValueId, &OpKind> = BTreeMap::new();
    for block in &func.blocks {
        for op in &block.ops {
            if let Some(res) = op.result {
                defs.insert(res, &op.kind);
            }
            for u in op.kind.uses() {
                *use_counts.entry(u).or_insert(0) += 1;
            }
        }
        match &block.term {
            RTerm::Goto(_, args) => {
                for &a in args {
                    *use_counts.entry(a).or_insert(0) += 1;
                }
            }
            RTerm::Switch {
                scrut,
                cases,
                default,
                ..
            } => {
                *use_counts.entry(*scrut).or_insert(0) += 1;
                for (_, _, args) in cases {
                    for &a in args {
                        *use_counts.entry(a).or_insert(0) += 1;
                    }
                }
                for &a in &default.1 {
                    *use_counts.entry(a).or_insert(0) += 1;
                }
            }
            RTerm::Return(Some(v)) | RTerm::Throw(Some(v)) => {
                *use_counts.entry(*v).or_insert(0) += 1;
            }
            RTerm::Invoke { normal, unwind } => {
                for &a in normal.1.iter().chain(&unwind.1) {
                    *use_counts.entry(a).or_insert(0) += 1;
                }
            }
            RTerm::Return(None) | RTerm::Throw(None) | RTerm::Unreachable => {}
        }
    }

    let mut out = BTreeSet::new();
    for block in &func.blocks {
        for op in &block.ops {
            let OpKind::Intrinsic { name, args, .. } = &op.kind else {
                continue;
            };
            if !matches!(interns.str(*name), Some("copy_nonoverlapping" | "copy")) || args.len() < 3
            {
                continue;
            }
            let src = args[0];
            let count = args[2];
            if use_counts.get(&src).copied() != Some(1) {
                continue;
            }
            if !matches!(
                defs.get(&count),
                Some(OpKind::Const(ConstTree::Scalar { bits: 1, .. }))
            ) {
                continue;
            }
            let (ref_val, from_ty, to_ty) = match defs.get(&src) {
                Some(OpKind::Cast {
                    kind: CastKind::PtrToPtr | CastKind::PointerCoercion,
                    v,
                    from,
                    to,
                }) => (*v, *from, *to),
                Some(OpKind::RawView {
                    root: Root::Value(v),
                    path,
                    ..
                }) if matches!(path.as_slice(), [Proj::Deref]) => {
                    let (Some(from), Some(to)) = (val_ty(func, *v), val_ty(func, src)) else {
                        continue;
                    };
                    (*v, from, to)
                }
                _ => continue,
            };
            if func
                .values
                .get(src.0 as usize)
                .is_none_or(|d| d.carrier != Carrier::Baseline)
                || func
                    .values
                    .get(ref_val.0 as usize)
                    .is_none_or(|d| d.carrier != Carrier::Baseline)
            {
                continue;
            }
            let (Some(from_pt), Some(to_pt)) = (
                value::pointee_of(table, from_ty),
                value::pointee_of(table, to_ty),
            ) else {
                continue;
            };
            if value::is_raw(table, from_ty) || !value::is_raw(table, to_ty) {
                continue;
            }
            let peeled = table.peeled(from_pt);
            if peeled != table.peeled(to_pt)
                || !table.is_indirect(peeled)
                || !value::is_slot_pointee(table, peeled)
                || table.unsized_tail(peeled).is_some()
                || value::is_str(table, peeled)
                || matches!(
                    table.get(peeled).map(|d| &d.kind),
                    Some(TypeKind::Array { .. })
                )
            {
                continue;
            }
            out.insert(src);
        }
    }
    out
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

/// Exact abstract mirror of `lower/js/place.rs::{walk_place, raw_view}` to determine whether an
/// `OpKind::RawView { root, path, .. }` can emit `__rt.box(...)` or `ref_to_raw(...)` on a bare
/// indirect object (requiring `_boxes` registration), as opposed to constructing a `{ buf, off }`
/// slot directly from a boxed cell, raw pointer, slice element, or non-transparent struct field.
fn raw_view_may_box(func: &Func, table: &TypeTable, root: &Root, path: &[Proj]) -> bool {
    let is_object_ty = |t: TypeId| {
        table.is_indirect(t) && value::is_slot_pointee(table, t) && table.unsized_tail(t).is_none()
    };
    let (mut ty, mut has_storage, mut object) = match root {
        Root::Cell(c) => {
            let Some(cell) = func.cells.get(c.0 as usize) else {
                return true;
            };
            (cell.ty, cell.boxed, false)
        }
        Root::Value(v) => {
            let Some(t) = val_ty(func, *v) else {
                return true;
            };
            (t, false, is_object_ty(t))
        }
    };
    let mut variant: Option<u32> = None;
    let mut identity = false;

    for proj in path {
        identity = false;
        object &= matches!(proj, Proj::Downcast { .. } | Proj::Field { .. });
        match proj {
            Proj::Downcast { variant: v } => {
                variant = Some(*v);
            }
            Proj::Deref => {
                let Some(pointee) = value::pointee_of(table, ty) else {
                    return true;
                };
                let shape = table.peeled(pointee);
                match value::kind(table, shape) {
                    Some(TypeKind::Dynamic { .. } | TypeKind::Slice(_) | TypeKind::Str) => {
                        identity = true;
                        has_storage = false;
                    }
                    _ if table.unsized_tail(shape).is_some() => {
                        identity = true;
                        has_storage = false;
                    }
                    _ if value::is_slot(table, ty) => {
                        has_storage = true;
                    }
                    _ => {
                        object = is_object_ty(pointee);
                        has_storage = false;
                    }
                }
                ty = pointee;
                variant = None;
            }
            Proj::Field {
                field,
                ty: field_ty,
            } => {
                let Some(desc) = table.field(ty, variant, *field) else {
                    return true;
                };
                let transparent =
                    matches!(desc.key, rcgjs_object::jir::types::FieldKey::Transparent);
                object &= transparent;
                if !transparent {
                    has_storage = table.get(*field_ty).is_some_and(|d| d.layout.sized);
                }
                ty = *field_ty;
                variant = None;
            }
            Proj::Index(_) | Proj::ConstantIndex { .. } => {
                let element = match value::kind(table, ty) {
                    Some(TypeKind::Array { element, .. } | TypeKind::Slice(element)) => *element,
                    _ => return true,
                };
                has_storage = true;
                ty = element;
                variant = None;
            }
            Proj::Subslice { .. } => {
                // Conservative: fixed-size subslice patterns may yield an array place.
                has_storage = false;
                variant = None;
            }
        }
    }

    if value::is_str(table, ty) {
        return false;
    }
    if identity {
        return table.unsized_tail(ty).is_some()
            || (value::is_slot_pointee(table, ty) && table.is_indirect(ty));
    }
    if !value::is_slot_pointee(table, ty) {
        return false;
    }
    object || (!has_storage && table.is_indirect(ty))
}

fn scan_func_for_boxes(
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
    boxed: &mut BoxedSet,
    copy_one_boxed: &mut BoxedSet,
) {
    let direct_copy_casts = direct_ref_copy_one_casts(func, table, interns);
    macro_rules! mark {
        ($ty:expr) => {
            boxed.record(table, $ty)
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
                                boxed.all_adts_boxed = true;
                                boxed.all_non_adt_boxed = true;
                            }
                        }
                        if let Some(from_pt) = value::pointee_of(table, *from) {
                            let from_is_ref = !value::is_raw(table, *from);
                            let to_is_ref = value::pointee_of(table, *to).is_some()
                                && !value::is_raw(table, *to);
                            if from_is_ref
                                && !to_is_ref
                                && table.is_indirect(from_pt)
                                && value::is_slot_pointee(table, from_pt)
                            {
                                if op.result.is_some_and(|r| direct_copy_casts.contains(&r)) {
                                    copy_one_boxed.record(table, from_pt);
                                } else {
                                    mark!(from_pt);
                                }
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
                            if from_is_ref
                                && !to_is_ref
                                && table.is_indirect(from_pt)
                                && value::is_slot_pointee(table, from_pt)
                            {
                                if op.result.is_some_and(|r| direct_copy_casts.contains(&r)) {
                                    copy_one_boxed.record(table, from_pt);
                                } else {
                                    mark!(from_pt);
                                }
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
                OpKind::RawView { root, path, .. } => {
                    if let Some(res) = op.result {
                        if raw_view_may_box(func, table, root, path) {
                            if let Some(pt) =
                                val_ty(func, res).and_then(|t| value::pointee_of(table, t))
                            {
                                if direct_copy_casts.contains(&res) {
                                    copy_one_boxed.record(table, pt);
                                } else {
                                    mark!(pt);
                                }
                            }
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
                        boxed.all_adts_boxed = true;
                        boxed.all_non_adt_boxed = true;
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
                        root: Root::Value(ValueId(2)),
                        path: Vec::new(),
                        mutable: false,
                    },
                },
                Op {
                    result: Some(ValueId(1)),
                    origin: OriginId(0),
                    kind: OpKind::RawView {
                        root: Root::Value(ValueId(3)),
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
                ValueDef {
                    ty: boxed_adt,
                    origin: OriginId(0),
                    carrier: Carrier::Baseline,
                },
                ValueDef {
                    ty: arr_ty,
                    origin: OriginId(0),
                    carrier: Carrier::Baseline,
                },
            ],
        );

        let mut boxed = BoxedSet::default();
        let mut copy_one_boxed = BoxedSet::default();
        scan_func_for_boxes(
            &func,
            &interns.types,
            &interns,
            &mut boxed,
            &mut copy_one_boxed,
        );
        let plan = HeapPlan {
            typed_heap_enabled: true,
            native_string_enabled: false,
            raw_alloc_reachable: false,
            non_rc_raw_alloc_reachable: false,
            any_boxed: boxed.all_adts_boxed
                || boxed.all_non_adt_boxed
                || !boxed.boxed_defs.is_empty()
                || !boxed.boxed_non_adt_keys.is_empty(),
            boxed_defs: boxed.boxed_defs,
            boxed_non_adt_keys: boxed.boxed_non_adt_keys,
            all_non_adt_boxed: boxed.all_non_adt_boxed,
            all_adts_boxed: boxed.all_adts_boxed,
        };
        assert!(plan.needs_box_tracking(&interns.types, boxed_adt));
        assert!(!plan.needs_box_tracking(&interns.types, plain_adt));
        assert!(
            !plan.all_non_adt_boxed && plan.boxed_non_adt_keys.is_empty(),
            "TypeKind::Array is not a _boxes slot pointee"
        );
    }

    #[test]
    fn raw_view_on_boxed_cell_or_raw_ptr_does_not_mark_boxed_and_tuple_keys_isolate() {
        let mut interns = Interns::default();
        let u32_ty = TypeId(10);
        let u64_ty = TypeId(11);
        interns.types.insert(make_desc(
            10,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            4,
            false,
        ));
        interns.types.insert(make_desc(
            11,
            TypeKind::Integer {
                signed: false,
                bits: 64,
            },
            8,
            false,
        ));
        let x_key = interns.intern_str("x");
        let adt_def = DefinitionId {
            crate_id: 9,
            local_hash: 77,
        };
        let adt_ty = TypeId(20);
        let variant = VariantDescriptor {
            name: x_key,
            fields: vec![FieldDescriptor {
                key: FieldKey::Name(x_key),
                ty: u32_ty,
                offset: 0,
            }],
            logical_discr: 0,
            physical_tag: None,
            result_ok: false,
            saved_local_keys: Vec::new(),
        };
        interns.types.insert(make_desc(
            20,
            TypeKind::Adt {
                def: adt_def,
                form: NominalForm::Struct,
                args: Vec::new(),
                variants: vec![variant],
                niche: None,
            },
            4,
            true,
        ));
        let mut raw_adt = make_desc(
            21,
            TypeKind::RawPointer {
                pointee: adt_ty,
                mutable: true,
            },
            4,
            false,
        );
        raw_adt.js.pointer_form = Some(PointerForm::Slot);
        interns.types.insert(raw_adt);
        let raw_adt_ty = TypeId(21);

        let tup_a = TypeId(30);
        let tup_b = TypeId(31);
        interns.types.insert(make_desc(
            30,
            TypeKind::Tuple(vec![u32_ty, u32_ty]),
            8,
            true,
        ));
        interns.types.insert(make_desc(
            31,
            TypeKind::Tuple(vec![u64_ty, u32_ty]),
            16,
            true,
        ));
        let mut raw_tup_a = make_desc(
            32,
            TypeKind::RawPointer {
                pointee: tup_a,
                mutable: false,
            },
            4,
            false,
        );
        raw_tup_a.js.pointer_form = Some(PointerForm::Slot);
        interns.types.insert(raw_tup_a);

        // 1. RawView on a boxed cell or deref of a raw slot pointer must NOT mark `adt_ty`.
        let mut func = empty_func(
            2,
            vec![
                Op {
                    result: Some(ValueId(0)),
                    origin: OriginId(0),
                    kind: OpKind::RawView {
                        root: Root::Cell(rcgjs_object::jir::repr::CellId(0)),
                        path: Vec::new(),
                        mutable: true,
                    },
                },
                Op {
                    result: Some(ValueId(1)),
                    origin: OriginId(0),
                    kind: OpKind::RawView {
                        root: Root::Value(ValueId(0)),
                        path: vec![Proj::Deref],
                        mutable: true,
                    },
                },
                // 2. RawView on bare SSA `tup_a` marks ONLY `tup_a`'s structural key, not `tup_b`.
                Op {
                    result: Some(ValueId(2)),
                    origin: OriginId(0),
                    kind: OpKind::RawView {
                        root: Root::Value(ValueId(3)),
                        path: Vec::new(),
                        mutable: false,
                    },
                },
            ],
            vec![
                ValueDef {
                    ty: raw_adt_ty,
                    origin: OriginId(0),
                    carrier: Carrier::Baseline,
                },
                ValueDef {
                    ty: raw_adt_ty,
                    origin: OriginId(0),
                    carrier: Carrier::Baseline,
                },
                ValueDef {
                    ty: TypeId(32),
                    origin: OriginId(0),
                    carrier: Carrier::Baseline,
                },
                ValueDef {
                    ty: tup_a,
                    origin: OriginId(0),
                    carrier: Carrier::Baseline,
                },
            ],
        );
        func.cells.push(rcgjs_object::jir::repr::CellDef {
            ty: adt_ty,
            boxed: true,
            origin: OriginId(0),
        });

        let mut boxed = BoxedSet::default();
        let mut copy_one_boxed = BoxedSet::default();
        scan_func_for_boxes(
            &func,
            &interns.types,
            &interns,
            &mut boxed,
            &mut copy_one_boxed,
        );
        let plan = HeapPlan {
            typed_heap_enabled: true,
            native_string_enabled: false,
            raw_alloc_reachable: false,
            non_rc_raw_alloc_reachable: false,
            any_boxed: true,
            boxed_defs: boxed.boxed_defs,
            boxed_non_adt_keys: boxed.boxed_non_adt_keys,
            all_non_adt_boxed: boxed.all_non_adt_boxed,
            all_adts_boxed: boxed.all_adts_boxed,
        };
        assert!(
            !plan.needs_box_tracking(&interns.types, adt_ty),
            "boxed cell and raw pointer deref RawViews never call __rt.box"
        );
        assert!(plan.needs_box_tracking(&interns.types, tup_a));
        assert!(
            !plan.needs_box_tracking(&interns.types, tup_b),
            "distinct tuple shapes must stay isolated in boxed_non_adt_keys"
        );
    }
}
