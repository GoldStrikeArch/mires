//! Zero-macro JS↔Rust boundary marshaling for `#[js_export]` functions
//! (`BodySemantic::JsExport`, tagged by `link_section = "rcgjs.export.v1"`).
//!
//! Plain `#[unsafe(no_mangle)]` and `#[js_export(raw)]` functions remain `BodySemantic::Ordinary`
//! and never pass through this module. For `BodySemantic::JsExport` functions, identity types
//! (primitives, `()`, host handles `JsValue`/`JsString`/`JsArray`, `&str`, fieldless enums, and
//! plain structs/enums whose fields are all identity) emit **zero** wrapper code.
//!
//! Supported recursive conversions (up to depth [`MAX_DEPTH`]):
//! - `Option<T>`: JS `null`/`undefined` ↔ Rust `None` / `Some(T)` (matching exact `EnumRepr`,
//!   including `Tag`, `IntTag`, `NullNiche`, `SentinelNiche`, and `Number`).
//! - Top-level return `Result<T, E>`: `Ok(v)` returns converted `v` (or `undefined` for `()`);
//!   `Err(e)` converts `e` and throws a JavaScript `Error` (preserving `Error` instances and
//!   attaching `{ cause: e }` for structured non-string errors).
//! - `alloc::string::String`: JS UTF-16 string ↔ Rust `String` in all representation modes
//!   (`native_string_enabled`, `native_vec_enabled`, and `RawVec` fallback, under both `utf16` and
//!   `bytes` string modes).
//! - `alloc::vec::Vec<T>`: JS `Array` ↔ Rust `Vec<T>` in both `native_vec_enabled` (TypedArray /
//!   JS Array) and `RawVec` modes.
//! - `&[T]`: JS `Array` ↔ Rust fat slice `{ buf, off, len }`.
//! - `JsMapEntries<K, V>` / `JsBTreeMapEntries<K, V>`: JS plain object / `Map` → Rust `Vec<(K, V)>`.
//! - `std::collections::HashMap<K, V, S>` (`mires-std`) and `mires_js::collections::HashMap<K, V>`:
//!   Rust map → JS plain object `{}` (string keys) or `Map` (non-string keys).
//! - Plain Rust `struct`s, `enum`s, tuples, fixed arrays, and shared `Thin` references (`&T`):
//!   recursive field/element marshaling when any constituent requires conversion.
//! - `BodySemantic::JsExportOut` (`link_section = "rcgjs.export.out.v1"`): direct Rust→JS return
//!   marshaling helper for `#[js_export] async fn`.

use std::collections::BTreeSet;
#[cfg(test)]
use std::collections::HashSet;

use rcgjs_object::jir::interns::Interns;
#[cfg(test)]
use rcgjs_object::jir::repr::Func;
use rcgjs_object::jir::semantic::Signature;
use rcgjs_object::jir::types::{
    AllocatorIdentity, EnumRepr, FieldDescriptor, FieldKey, NominalForm, PinnedBoundary,
    PointerForm, RepresentationCandidate, TypeId, TypeKind, TypeTable, VariantDescriptor,
};
use rcgjs_object::jsast::{self, BinOp, Expr, Stmt, UnOp};

use crate::analysis::heap::HeapPlan;

use super::ast_util::TA_CTORS;
use super::value;

/// Maximum recursive type traversal depth before falling back to identity.
const MAX_DEPTH: u32 = 16;

/// Type and runtime-representation context for JS↔Rust boundary marshaling.
pub struct BoundaryCx<'a> {
    pub table: &'a TypeTable,
    pub interns: &'a Interns,
    pub heap: &'a HeapPlan,
    pub bstr: bool,
}

/// Whether an exported `#[js_export]` function with signature `sig` requires a separate external
/// JavaScript wrapper function to marshal arguments, return values, or `js-str=bytes` strings.
///
/// Returns `false` when all parameters and the return type are identity at the JS↔Rust boundary
/// (so the internal Rust function can be exported directly with zero wrapper bytes).
pub fn needs_js_export_wrapper(cx: &BoundaryCx<'_>, sig: &Signature) -> bool {
    if sig.abi.spread.is_some() {
        return false;
    }
    if result_info(cx.table, cx.interns, sig.result).is_some() {
        return true;
    }
    let convert_str_refs = cx.bstr;
    let arg_needs = sig.arguments.iter().any(|&arg_ty| {
        let mut seen = BTreeSet::new();
        needs_conversion(
            cx,
            arg_ty,
            ConvertMode {
                dir: Dir::JsToRust,
                convert_str_refs,
            },
            0,
            &mut seen,
        )
    });
    if arg_needs {
        return true;
    }
    let mut seen = BTreeSet::new();
    needs_conversion(
        cx,
        sig.result,
        ConvertMode {
            dir: Dir::RustToJs,
            convert_str_refs,
        },
        0,
        &mut seen,
    )
}

/// Builds a thin external JavaScript wrapper declaration `function <export_name>(...)` that
/// marshals JS arguments into Rust representations, invokes the unmodified internal Rust function
/// `<inner_name>(...)`, and marshals the Rust return value back to JS (unwrapping `Result<T, E>`
/// and throwing a JS `Error` on `Err`).
///
/// Returns `None` when [`needs_js_export_wrapper`] is `false` (zero wrapper overhead).
///
/// When `cx.bstr` is active, this single wrapper also handles `&str` ↔ byte-string conversions
/// (`__rt.bstr_in` / `__rt.bstr_out`), replacing any separate `byte_string_facades` wrapper.
/// `loc_arg` optionally appends the trailing `#[track_caller]` location parameter `(param_name, expr)`.
pub fn build_js_export_wrapper(
    cx: &BoundaryCx<'_>,
    export_name: &str,
    inner_name: &str,
    sig: &Signature,
    loc_arg: Option<(String, Expr)>,
) -> Option<Stmt> {
    if !needs_js_export_wrapper(cx, sig) {
        return None;
    }
    let mut emitter = MarshalEmitter::new_wrapper(cx);
    let mut params: Vec<String> = (0..sig.arguments.len()).map(|i| format!("a{i}")).collect();
    let mut call_args: Vec<Expr> = params
        .iter()
        .zip(&sig.arguments)
        .map(|(param, &arg_ty)| {
            let arg_expr = jsast::id(param.clone());
            if emitter.needs_js_to_rust_value(arg_ty) {
                emitter.js_to_rust(arg_expr, arg_ty, 0)
            } else {
                arg_expr
            }
        })
        .collect();
    if let Some((loc_param, loc_expr)) = loc_arg {
        params.push(loc_param);
        call_args.push(loc_expr);
    }
    let call = jsast::call(jsast::id(inner_name.to_owned()), call_args);
    let ret_ty = sig.result;
    let body = if let Some(res) = result_info(cx.table, cx.interns, ret_ty) {
        emitter.result_return_stmts(call, &res)
    } else if cx.table.is_zst(ret_ty) {
        vec![jsast::expr_stmt(call)]
    } else if emitter.needs_rust_to_js_value(ret_ty) {
        vec![jsast::ret(emitter.rust_to_js(call, ret_ty, 0))]
    } else {
        vec![jsast::ret(call)]
    };
    Some(jsast::function(export_name.to_owned(), params, body))
}

/// Applies JS↔Rust boundary marshaling to a lowered `BodySemantic::JsExport` function body in
/// place. Retained for unit tests covering direct body-rewriting semantics.
#[cfg(test)]
pub fn wrap_js_export(
    cx: &BoundaryCx<'_>,
    func: &Func,
    formal_params: &[String],
    body: &mut Vec<Stmt>,
) -> HashSet<String> {
    let mut emitter = MarshalEmitter::new(cx);
    let mut reassigned = HashSet::new();

    if func.sig.abi.spread.is_none() {
        let mut prelude = Vec::new();
        for (param_name, &arg_ty) in formal_params.iter().zip(&func.sig.arguments) {
            if needs_js_to_rust(cx, arg_ty) {
                let converted = emitter.js_to_rust(jsast::id(param_name), arg_ty, 0);
                prelude.push(jsast::assign_stmt(jsast::id(param_name), converted));
                reassigned.insert(param_name.clone());
            }
        }
        if !prelude.is_empty() {
            prelude.append(body);
            *body = prelude;
        }
    }

    let ret_ty = func.sig.result;
    if let Some(res) = result_info(cx.table, cx.interns, ret_ty) {
        emitter.rewrite_returns_result(body, &res);
    } else if needs_rust_to_js(cx, ret_ty) {
        emitter.rewrite_returns_value(body, ret_ty);
    }

    reassigned
}

/// Lowers the complete body of an internal `BodySemantic::JsExportOut` function
/// (`fn __mires_export_out(value: R) -> JsValue`) used by `#[js_export] async fn`.
///
/// - `()` / ZST returns `return undefined;`
/// - `Result<T, E>` unwraps `Ok(v)` via `rust_to_js` or throws a JS `Error` on `Err(e)`
///   (rejecting the returned `Promise` at the poll boundary)
/// - Any other type returns `rust_to_js($p0, R)` (or bare `return $p0;` when identity).
pub fn lower_export_out_body(cx: &BoundaryCx<'_>, param_name: &str, value_ty: TypeId) -> Vec<Stmt> {
    if cx.table.is_zst(value_ty) {
        return vec![jsast::ret(jsast::undefined())];
    }
    let mut emitter = MarshalEmitter::new_export_out(cx);
    if let Some(res) = result_info(cx.table, cx.interns, value_ty) {
        return emitter.result_return_stmts(jsast::id(param_name), &res);
    }
    if emitter.needs_rust_to_js_value(value_ty) {
        return vec![jsast::ret(emitter.rust_to_js(
            jsast::id(param_name),
            value_ty,
            0,
        ))];
    }
    vec![jsast::ret(jsast::id(param_name))]
}

/// Whether a JS caller value of Rust type `ty` requires marshaling into Rust's representation.
#[cfg(test)]
pub fn needs_js_to_rust(cx: &BoundaryCx<'_>, ty: TypeId) -> bool {
    let mut seen = BTreeSet::new();
    needs_conversion(
        cx,
        ty,
        ConvertMode {
            dir: Dir::JsToRust,
            convert_str_refs: false,
        },
        0,
        &mut seen,
    )
}

/// Whether a Rust value of type `ty` requires marshaling into a JS value for JS callers.
#[cfg(test)]
pub fn needs_rust_to_js(cx: &BoundaryCx<'_>, ty: TypeId) -> bool {
    let mut seen = BTreeSet::new();
    needs_conversion(
        cx,
        ty,
        ConvertMode {
            dir: Dir::RustToJs,
            convert_str_refs: false,
        },
        0,
        &mut seen,
    )
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Dir {
    JsToRust,
    RustToJs,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct ConvertMode {
    dir: Dir,
    /// Convert `&str` (`PointerForm::Str`) directly via `__rt.bstr_out` when `bstr` is active.
    /// Enabled only for non-exported `BodySemantic::JsExportOut` helpers, which are not wrapped
    /// by `link.rs::byte_string_facades`.
    convert_str_refs: bool,
}

fn needs_conversion(
    cx: &BoundaryCx<'_>,
    ty: TypeId,
    mode: ConvertMode,
    depth: u32,
    seen: &mut BTreeSet<TypeId>,
) -> bool {
    if depth >= MAX_DEPTH || cx.table.is_zst(ty) {
        return false;
    }
    let Some(desc) = cx.table.get(ty) else {
        return false;
    };
    // Detect `mires_js` transparent map adapters/maps BEFORE `PinnedBoundary::Host` and before
    // peeling transparent wrappers.
    if map_entries_info(cx.table, cx.interns, ty).is_some() {
        return mode.dir == Dir::JsToRust;
    }
    if mires_js_hashmap_info(cx.table, cx.interns, ty).is_some() {
        return mode.dir == Dir::RustToJs;
    }
    if desc.meta.boundary == PinnedBoundary::Host {
        return false;
    }
    if is_string_ty(cx.table, cx.interns, ty) || is_marshal_vec_ty(cx.table, ty) {
        return true;
    }
    if std_hashmap_info(cx.table, cx.interns, ty).is_some() {
        return mode.dir == Dir::RustToJs;
    }
    if matches!(
        desc.meta.candidate,
        RepresentationCandidate::Box | RepresentationCandidate::BTreeMap
    ) {
        return false;
    }
    if desc.js.crosses_transparent_union {
        return false;
    }
    if let TypeKind::Pattern(inner) = desc.kind {
        return needs_conversion(cx, inner, mode, depth + 1, seen);
    }
    if let Some(transparent) = desc.js.transparent {
        return needs_conversion(cx, transparent.ty, mode, depth + 1, seen);
    }
    if !seen.insert(ty) {
        return false;
    }
    let result = match &desc.kind {
        TypeKind::Reference {
            pointee,
            mutable: false,
        } => match cx.table.pointer_form(ty) {
            Some(PointerForm::FatSlice) => matches!(
                cx.table.get(cx.table.peeled(*pointee)).map(|d| &d.kind),
                Some(TypeKind::Slice(_))
            ),
            Some(PointerForm::Str) => mode.convert_str_refs,
            Some(PointerForm::Thin) => needs_conversion(cx, *pointee, mode, depth + 1, seen),
            _ => false,
        },
        TypeKind::Tuple(elems) => elems
            .iter()
            .any(|&elem| needs_conversion(cx, elem, mode, depth + 1, seen)),
        TypeKind::Array { element, .. } => needs_conversion(cx, *element, mode, depth + 1, seen),
        TypeKind::Adt {
            form: NominalForm::Struct,
            variants,
            ..
        } => variants.first().is_some_and(|variant| {
            variant
                .fields
                .iter()
                .any(|f| needs_conversion(cx, f.ty, mode, depth + 1, seen))
        }),
        TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            ..
        } => {
            if option_info(cx.table, cx.interns, ty).is_some() {
                true
            } else {
                enum_needs_conversion(cx, ty, variants, mode, depth, seen)
            }
        }
        _ => false,
    };
    seen.remove(&ty);
    result
}

fn enum_needs_conversion(
    cx: &BoundaryCx<'_>,
    ty: TypeId,
    variants: &[VariantDescriptor],
    mode: ConvertMode,
    depth: u32,
    seen: &mut BTreeSet<TypeId>,
) -> bool {
    let Some(repr) = cx.table.enum_repr(ty) else {
        return false;
    };
    match repr {
        EnumRepr::Number { .. }
        | EnumRepr::State
        | EnumRepr::Tag { fieldless: true }
        | EnumRepr::IntTag { fieldless: true } => false,
        EnumRepr::Tag { fieldless: false } | EnumRepr::IntTag { fieldless: false } => {
            variants.iter().any(|v| {
                v.fields
                    .iter()
                    .any(|f| needs_conversion(cx, f.ty, mode, depth + 1, seen))
            })
        }
        EnumRepr::NullNiche { .. } | EnumRepr::SentinelNiche { .. } => {
            let Some((_, some_variant, some_field)) = repr.niche_variants() else {
                return false;
            };
            variants
                .get(some_variant as usize)
                .and_then(|v| v.fields.get(some_field as usize))
                .is_some_and(|f| needs_conversion(cx, f.ty, mode, depth + 1, seen))
        }
    }
}

/// Structural info extracted for `core::option::Option<T>`.
struct OptionInfo {
    ty: TypeId,
    none_idx: u32,
    some_idx: u32,
    some_field_key: FieldKey,
    inner_ty: TypeId,
    repr: EnumRepr,
}

/// Structural info extracted for `core::result::Result<T, E>`.
struct ResultInfo {
    ty: TypeId,
    ok_idx: u32,
    err_idx: u32,
    ok_field_key: FieldKey,
    ok_ty: TypeId,
    err_field_key: FieldKey,
    err_ty: TypeId,
    repr: EnumRepr,
}

/// Extracted `RawVec` field keys for non-native `Vec<T>` / `String`.
struct RawVecLayout {
    buf_key: String,
    len_key: String,
    vec_extra_keys: Vec<String>,
    inner_key: Option<String>,
    raw_extra_keys: Vec<String>,
    ptr_key: String,
    cap_key: String,
    inner_extra_keys: Vec<String>,
}

/// Structural info for `JsMapEntries<K, V>` / `JsBTreeMapEntries<K, V>` transparent adapters.
struct MapEntriesInfo {
    vec_ty: TypeId,
    pair_ty: TypeId,
    key_ty: TypeId,
    is_global: bool,
}

/// Structural info for `mires-std`'s open-addressing `HashMap<K, V, S>`.
struct StdHashMapInfo {
    slots_field_key: FieldKey,
    slots_vec_ty: TypeId,
    slots_is_global: bool,
    slot_ty: TypeId,
    slot_repr: EnumRepr,
    occupied_idx: u32,
    key_field_key: FieldKey,
    key_ty: TypeId,
    val_field_key: FieldKey,
    val_ty: TypeId,
}

/// Structural info for `mires_js::collections::HashMap<K, V>`.
struct MiresJsHashMapInfo {
    key_ty: TypeId,
    val_ty: TypeId,
}

/// Precomputed element layout facts for native `Vec<T, Global>` JS→Rust conversion.
struct NativeVecElem {
    elem_ty: TypeId,
    is_zst: bool,
    elem_needs: bool,
    ta_kind: Option<u8>,
    tracked: bool,
}

fn is_str_ref_ty(table: &TypeTable, ty: TypeId) -> bool {
    let peeled = table.peeled(ty);
    match table.get(peeled).map(|d| &d.kind) {
        Some(TypeKind::Str) => true,
        Some(TypeKind::Reference {
            pointee,
            mutable: false,
        }) => {
            table.pointer_form(peeled) == Some(PointerForm::Str)
                || matches!(
                    table.get(table.peeled(*pointee)).map(|d| &d.kind),
                    Some(TypeKind::Str)
                )
        }
        _ => false,
    }
}

fn is_string_key_ty(table: &TypeTable, interns: &Interns, ty: TypeId) -> bool {
    is_string_ty(table, interns, ty) || is_str_ref_ty(table, ty)
}

fn map_entries_info(table: &TypeTable, interns: &Interns, ty: TypeId) -> Option<MapEntriesInfo> {
    let desc = table.get(ty)?;
    let TypeKind::Adt {
        form: NominalForm::Struct,
        args,
        variants,
        ..
    } = &desc.kind
    else {
        return None;
    };
    if args.len() != 2 {
        return None;
    }
    let name_matches = desc
        .name
        .and_then(|id| interns.str(id))
        .is_some_and(|name| {
            let base = name.split('<').next().unwrap_or(name);
            base == "JsMapEntries"
                || base == "JsBTreeMapEntries"
                || base.ends_with("::JsMapEntries")
                || base.ends_with("::JsBTreeMapEntries")
        });
    let variant_matches = desc.meta.boundary == PinnedBoundary::Host
        && variants
            .first()
            .and_then(|v| interns.str(v.name))
            .is_some_and(|name| name == "JsMapEntries" || name == "JsBTreeMapEntries");
    if !name_matches && !variant_matches {
        return None;
    }
    let transparent = desc.js.transparent?;
    let (pair_ty, is_global) = vec_elem_and_global(table, transparent.ty)?;
    Some(MapEntriesInfo {
        vec_ty: transparent.ty,
        pair_ty,
        key_ty: args[0],
        is_global,
    })
}

fn mires_js_hashmap_info(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
) -> Option<MiresJsHashMapInfo> {
    let desc = table.get(ty)?;
    if desc.meta.boundary != PinnedBoundary::Host {
        return None;
    }
    let TypeKind::Adt {
        form: NominalForm::Struct,
        args,
        variants,
        ..
    } = &desc.kind
    else {
        return None;
    };
    if args.len() != 2 {
        return None;
    }
    let name_matches = desc
        .name
        .and_then(|id| interns.str(id))
        .is_some_and(|name| {
            let base = name.split('<').next().unwrap_or(name);
            base == "HashMap" || base.ends_with("::HashMap")
        });
    let variant_matches = variants
        .first()
        .and_then(|v| interns.str(v.name))
        .is_some_and(|name| name == "HashMap");
    if !name_matches && !variant_matches {
        return None;
    }
    let transparent = desc.js.transparent?;
    if vec_elem_and_global(table, transparent.ty).is_some() {
        return None;
    }
    Some(MiresJsHashMapInfo {
        key_ty: args[0],
        val_ty: args[1],
    })
}

fn std_hashmap_info(table: &TypeTable, interns: &Interns, ty: TypeId) -> Option<StdHashMapInfo> {
    let peeled = table.peeled(ty);
    let desc = table.get(peeled)?;
    if desc.meta.boundary == PinnedBoundary::Host {
        return None;
    }
    let TypeKind::Adt {
        form: NominalForm::Struct,
        variants,
        ..
    } = &desc.kind
    else {
        return None;
    };
    let [variant] = variants.as_slice() else {
        return None;
    };
    let name_matches = desc
        .name
        .and_then(|id| interns.str(id))
        .is_some_and(|name| {
            let base = name.split('<').next().unwrap_or(name);
            base == "HashMap" || base.ends_with("::HashMap")
        });
    let variant_matches = interns.str(variant.name) == Some("HashMap");
    if !name_matches && !variant_matches {
        return None;
    }
    let mut slots_field = None;
    let mut has_len = false;
    let mut has_tombstones = false;
    for field in &variant.fields {
        match field_key_str(interns, field.key).as_deref() {
            Some("slots") => slots_field = Some((field.key, field.ty)),
            Some("len") => has_len = true,
            Some("tombstones") => has_tombstones = true,
            _ => {}
        }
    }
    if !has_len || !has_tombstones {
        return None;
    }
    let (slots_field_key, slots_vec_ty) = slots_field?;
    let (slot_ty_raw, slots_is_global) = vec_elem_and_global(table, slots_vec_ty)?;
    let slot_ty = table.peeled(slot_ty_raw);
    let slot_desc = table.get(slot_ty)?;
    let TypeKind::Adt {
        form: NominalForm::Enum,
        variants: slot_variants,
        ..
    } = &slot_desc.kind
    else {
        return None;
    };
    let slot_repr = table.enum_repr(slot_ty)?;
    let (occupied_idx, occupied_variant) = slot_variants
        .iter()
        .enumerate()
        .find(|(_, v)| interns.str(v.name) == Some("Occupied"))?;
    let mut key_field = None;
    let mut val_field = None;
    for field in &occupied_variant.fields {
        match field_key_str(interns, field.key).as_deref() {
            Some("key") => key_field = Some((field.key, field.ty)),
            Some("value") => val_field = Some((field.key, field.ty)),
            _ => {}
        }
    }
    let (key_field_key, key_ty) = key_field?;
    let (val_field_key, val_ty) = val_field?;
    Some(StdHashMapInfo {
        slots_field_key,
        slots_vec_ty,
        slots_is_global,
        slot_ty,
        slot_repr,
        occupied_idx: occupied_idx as u32,
        key_field_key,
        key_ty,
        val_field_key,
        val_ty,
    })
}

/// Finds the exact `Option<val_ty>` descriptor used inside `mires_js::collections::map::MapEntry<V>`
/// when present in `table`.
fn find_map_entry_option(
    table: &TypeTable,
    interns: &Interns,
    val_ty: TypeId,
) -> Option<OptionInfo> {
    let peeled_val = table.peeled(val_ty);
    for desc in table.descriptors() {
        if desc.meta.boundary != PinnedBoundary::Host {
            continue;
        }
        let TypeKind::Adt {
            form: NominalForm::Struct,
            args,
            variants,
            ..
        } = &desc.kind
        else {
            continue;
        };
        let [variant] = variants.as_slice() else {
            continue;
        };
        if interns.str(variant.name) == Some("MapEntry")
            && args.first().copied().map(|t| table.peeled(t)) == Some(peeled_val)
        {
            if let Some(field) = variant
                .fields
                .iter()
                .find(|f| field_key_str(interns, f.key).as_deref() == Some("value"))
            {
                if let Some(opt) = option_info(table, interns, field.ty) {
                    return Some(opt);
                }
            }
        }
    }
    for desc in table.descriptors() {
        if let Some(opt) = option_info(table, interns, desc.id) {
            if opt.inner_ty == val_ty || table.peeled(opt.inner_ty) == peeled_val {
                return Some(opt);
            }
        }
    }
    None
}

fn is_string_ty(table: &TypeTable, interns: &Interns, ty: TypeId) -> bool {
    let Some(desc) = table.get(ty) else {
        return false;
    };
    if desc.meta.candidate == RepresentationCandidate::String {
        return true;
    }
    desc.name
        .and_then(|id| interns.str(id))
        .is_some_and(|name| {
            name == "alloc::string::String"
                || name == "std::string::String"
                || name.ends_with("::string::String")
        })
}

fn is_marshal_vec_ty(table: &TypeTable, ty: TypeId) -> bool {
    vec_elem_and_global(table, ty).is_some()
}

fn vec_elem_and_global(table: &TypeTable, ty: TypeId) -> Option<(TypeId, bool)> {
    let desc = table.get(ty)?;
    if desc.meta.candidate != RepresentationCandidate::Vec {
        return None;
    }
    if desc.meta.allocator == AllocatorIdentity::Custom {
        return None;
    }
    let TypeKind::Adt { args, .. } = &desc.kind else {
        return None;
    };
    let elem = *args.first()?;
    Some((elem, desc.meta.allocator == AllocatorIdentity::Global))
}

fn option_info(table: &TypeTable, interns: &Interns, ty: TypeId) -> Option<OptionInfo> {
    let peeled = table.peeled(ty);
    let desc = table.get(peeled)?;
    if desc.meta.boundary == PinnedBoundary::Host {
        return None;
    }
    let TypeKind::Adt {
        form: NominalForm::Enum,
        variants,
        args,
        ..
    } = &desc.kind
    else {
        return None;
    };
    if variants.len() != 2 || args.len() != 1 {
        return None;
    }
    let mut none_idx = None;
    let mut some_idx = None;
    for (idx, variant) in variants.iter().enumerate() {
        match interns.str(variant.name) {
            Some("None") if variant.fields.is_empty() => none_idx = Some(idx as u32),
            Some("Some") if variant.fields.len() == 1 => some_idx = Some(idx as u32),
            _ => return None,
        }
    }
    let (none_idx, some_idx) = (none_idx?, some_idx?);
    let some_field = variants[some_idx as usize].fields.first()?;
    let repr = table.enum_repr(peeled)?;
    Some(OptionInfo {
        ty: peeled,
        none_idx,
        some_idx,
        some_field_key: some_field.key,
        inner_ty: some_field.ty,
        repr,
    })
}

fn result_info(table: &TypeTable, interns: &Interns, ty: TypeId) -> Option<ResultInfo> {
    let peeled = table.peeled(ty);
    let desc = table.get(peeled)?;
    if desc.meta.boundary == PinnedBoundary::Host {
        return None;
    }
    let TypeKind::Adt {
        form: NominalForm::Enum,
        variants,
        args,
        ..
    } = &desc.kind
    else {
        return None;
    };
    if variants.len() != 2 {
        return None;
    }
    let (ok_idx, err_idx) = if variants[0].result_ok {
        (0u32, 1u32)
    } else if variants[1].result_ok {
        (1u32, 0u32)
    } else if args.len() == 2 {
        let n0 = interns.str(variants[0].name);
        let n1 = interns.str(variants[1].name);
        match (n0, n1) {
            (Some("Ok"), Some("Err")) => (0u32, 1u32),
            (Some("Err"), Some("Ok")) => (1u32, 0u32),
            _ => return None,
        }
    } else {
        return None;
    };
    let ok_field = variants[ok_idx as usize].fields.first()?;
    let err_field = variants[err_idx as usize].fields.first()?;
    let repr = table.enum_repr(peeled)?;
    Some(ResultInfo {
        ty: peeled,
        ok_idx,
        err_idx,
        ok_field_key: ok_field.key,
        ok_ty: ok_field.ty,
        err_field_key: err_field.key,
        err_ty: err_field.ty,
        repr,
    })
}

fn struct_fields(table: &TypeTable, ty: TypeId) -> Option<&[FieldDescriptor]> {
    let desc = table.get(table.peeled(ty))?;
    let TypeKind::Adt {
        form: NominalForm::Struct,
        variants,
        ..
    } = &desc.kind
    else {
        return None;
    };
    let [variant] = variants.as_slice() else {
        return None;
    };
    Some(&variant.fields)
}

fn field_key_str(interns: &Interns, key: FieldKey) -> Option<String> {
    match key {
        FieldKey::Name(id) => interns.str(id).map(str::to_owned),
        FieldKey::Index(i) => Some(format!("_{i}")),
        FieldKey::Transparent => None,
    }
}

/// Inspects a `Vec<T>` type descriptor to discover the exact field keys of `Vec`, `RawVec`, and
/// `RawVecInner`, falling back to standard `alloc` names when simplified descriptors are used.
fn raw_vec_layout(table: &TypeTable, interns: &Interns, vec_ty: TypeId) -> RawVecLayout {
    let default = || RawVecLayout {
        buf_key: "buf".to_owned(),
        len_key: "len".to_owned(),
        vec_extra_keys: Vec::new(),
        inner_key: Some("inner".to_owned()),
        raw_extra_keys: vec!["_marker".to_owned()],
        ptr_key: "ptr".to_owned(),
        cap_key: "cap".to_owned(),
        inner_extra_keys: vec!["alloc".to_owned()],
    };
    let Some(vec_fields) = struct_fields(table, vec_ty) else {
        return default();
    };
    let mut buf_field = None;
    let mut len_key = None;
    let mut vec_extra_keys = Vec::new();
    for field in vec_fields {
        let Some(name) = field_key_str(interns, field.key) else {
            continue;
        };
        if name == "len" {
            len_key = Some(name);
        } else if name == "buf" {
            buf_field = Some((name, field.ty));
        } else {
            vec_extra_keys.push(name);
        }
    }
    let (Some((buf_key, buf_ty)), Some(len_key)) = (buf_field, len_key) else {
        return default();
    };
    let Some(raw_fields) = struct_fields(table, buf_ty) else {
        return default();
    };
    // Check whether `RawVec` directly holds `{ ptr, cap }` or wraps `inner: RawVecInner`.
    let has_direct_ptr = raw_fields
        .iter()
        .any(|f| field_key_str(interns, f.key).as_deref() == Some("ptr"));
    if has_direct_ptr {
        let mut ptr_key = "ptr".to_owned();
        let mut cap_key = "cap".to_owned();
        let mut inner_extra_keys = Vec::new();
        for f in raw_fields {
            let Some(name) = field_key_str(interns, f.key) else {
                continue;
            };
            if name == "ptr" {
                ptr_key = name;
            } else if name == "cap" {
                cap_key = name;
            } else {
                inner_extra_keys.push(name);
            }
        }
        return RawVecLayout {
            buf_key,
            len_key,
            vec_extra_keys,
            inner_key: None,
            raw_extra_keys: Vec::new(),
            ptr_key,
            cap_key,
            inner_extra_keys,
        };
    }
    let mut inner_field = None;
    let mut raw_extra_keys = Vec::new();
    for f in raw_fields {
        let Some(name) = field_key_str(interns, f.key) else {
            continue;
        };
        if name == "inner" {
            inner_field = Some((name, f.ty));
        } else {
            raw_extra_keys.push(name);
        }
    }
    let Some((inner_key, inner_ty)) = inner_field else {
        return default();
    };
    let mut ptr_key = "ptr".to_owned();
    let mut cap_key = "cap".to_owned();
    let mut inner_extra_keys = Vec::new();
    if let Some(inner_fields) = struct_fields(table, inner_ty) {
        for f in inner_fields {
            let Some(name) = field_key_str(interns, f.key) else {
                continue;
            };
            if name == "ptr" {
                ptr_key = name;
            } else if name == "cap" {
                cap_key = name;
            } else {
                inner_extra_keys.push(name);
            }
        }
    }
    RawVecLayout {
        buf_key,
        len_key,
        vec_extra_keys,
        inner_key: Some(inner_key),
        raw_extra_keys,
        ptr_key,
        cap_key,
        inner_extra_keys,
    }
}

impl RawVecLayout {
    fn build_object(&self, ptr: Expr, cap: Expr, len: Expr) -> Expr {
        let mut inner_entries = vec![(self.ptr_key.clone(), ptr), (self.cap_key.clone(), cap)];
        for key in &self.inner_extra_keys {
            inner_entries.push((key.clone(), jsast::undefined()));
        }
        let inner_obj = jsast::object(inner_entries);
        let buf_obj = match &self.inner_key {
            Some(inner_key) => {
                let mut raw_entries = vec![(inner_key.clone(), inner_obj)];
                for key in &self.raw_extra_keys {
                    raw_entries.push((key.clone(), jsast::undefined()));
                }
                jsast::object(raw_entries)
            }
            None => inner_obj,
        };
        let mut vec_entries = vec![(self.buf_key.clone(), buf_obj), (self.len_key.clone(), len)];
        for key in &self.vec_extra_keys {
            vec_entries.push((key.clone(), jsast::undefined()));
        }
        jsast::object(vec_entries)
    }

    fn ptr_expr(&self, vec_expr: Expr) -> Expr {
        let buf = jsast::member(vec_expr, self.buf_key.clone());
        let inner = match &self.inner_key {
            Some(inner_key) => jsast::member(buf, inner_key.clone()),
            None => buf,
        };
        jsast::member(inner, self.ptr_key.clone())
    }

    fn len_expr(&self, vec_expr: Expr) -> Expr {
        jsast::member(vec_expr, self.len_key.clone())
    }
}

/// Stateful AST emitter keeping synthetic boundary variable names collision-free.
struct MarshalEmitter<'a> {
    cx: &'a BoundaryCx<'a>,
    next_id: usize,
    convert_str_refs: bool,
}

impl<'a> MarshalEmitter<'a> {
    #[cfg(test)]
    fn new(cx: &'a BoundaryCx<'a>) -> Self {
        Self {
            cx,
            next_id: 0,
            convert_str_refs: false,
        }
    }

    fn new_wrapper(cx: &'a BoundaryCx<'a>) -> Self {
        Self {
            cx,
            next_id: 0,
            convert_str_refs: cx.bstr,
        }
    }

    fn new_export_out(cx: &'a BoundaryCx<'a>) -> Self {
        Self {
            cx,
            next_id: 0,
            convert_str_refs: cx.bstr,
        }
    }

    fn needs_js_to_rust_value(&self, ty: TypeId) -> bool {
        let mut seen = BTreeSet::new();
        needs_conversion(
            self.cx,
            ty,
            ConvertMode {
                dir: Dir::JsToRust,
                convert_str_refs: self.convert_str_refs,
            },
            0,
            &mut seen,
        )
    }

    fn needs_rust_to_js_value(&self, ty: TypeId) -> bool {
        let mut seen = BTreeSet::new();
        needs_conversion(
            self.cx,
            ty,
            ConvertMode {
                dir: Dir::RustToJs,
                convert_str_refs: self.convert_str_refs,
            },
            0,
            &mut seen,
        )
    }

    fn fresh(&mut self, prefix: &str) -> String {
        let id = self.next_id;
        self.next_id += 1;
        format!("$eb_{prefix}{id}")
    }

    /// Evaluates `expr` once; if it is already an identifier, passes it straight through without
    /// introducing an IIFE.
    fn bind_once(
        &mut self,
        expr: Expr,
        prefix: &str,
        f: impl FnOnce(&mut Self, Expr) -> Expr,
    ) -> Expr {
        if matches!(expr, Expr::Ident(_)) {
            return f(self, expr);
        }
        let name = self.fresh(prefix);
        let body = f(self, jsast::id(name.clone()));
        jsast::call(jsast::arrow(vec![name], body), vec![expr])
    }

    fn js_to_rust(&mut self, expr: Expr, ty: TypeId, depth: u32) -> Expr {
        if depth >= MAX_DEPTH || !self.needs_js_to_rust_value(ty) {
            return expr;
        }
        let Some(desc) = self.cx.table.get(ty) else {
            return expr;
        };
        if let Some(info) = map_entries_info(self.cx.table, self.cx.interns, ty) {
            return self.map_entries_js_to_rust(expr, &info, depth);
        }
        if desc.meta.boundary == PinnedBoundary::Host {
            return expr;
        }
        if is_string_ty(self.cx.table, self.cx.interns, ty) {
            return self.string_js_to_rust(expr, ty);
        }
        if let Some((elem, is_global)) = vec_elem_and_global(self.cx.table, ty) {
            return self.vec_js_to_rust(expr, ty, elem, is_global, depth);
        }
        if let TypeKind::Pattern(inner) = desc.kind {
            return self.js_to_rust(expr, inner, depth + 1);
        }
        if let Some(transparent) = desc.js.transparent {
            return self.js_to_rust(expr, transparent.ty, depth + 1);
        }
        match &desc.kind {
            TypeKind::Reference {
                pointee,
                mutable: false,
            } => match self.cx.table.pointer_form(ty) {
                Some(PointerForm::FatSlice) => {
                    if let Some(TypeKind::Slice(elem)) = self
                        .cx
                        .table
                        .get(self.cx.table.peeled(*pointee))
                        .map(|d| &d.kind)
                    {
                        return self.slice_js_to_rust(expr, *elem, depth);
                    }
                    expr
                }
                Some(PointerForm::Str) if self.convert_str_refs => {
                    jsast::rt_call("bstr_in", vec![expr])
                }
                Some(PointerForm::Thin) => self.js_to_rust(expr, *pointee, depth + 1),
                _ => expr,
            },
            TypeKind::Tuple(elems) => self.tuple_convert(expr, elems, Dir::JsToRust, depth),
            TypeKind::Array { element, .. } => {
                self.array_convert(expr, *element, Dir::JsToRust, depth)
            }
            TypeKind::Adt {
                form: NominalForm::Struct,
                variants,
                ..
            } => match variants.first() {
                Some(variant) => self.struct_js_to_rust(expr, &variant.fields, depth),
                None => expr,
            },
            TypeKind::Adt {
                form: NominalForm::Enum,
                variants,
                ..
            } => {
                if let Some(opt) = option_info(self.cx.table, self.cx.interns, ty) {
                    return self.option_js_to_rust(expr, &opt, depth);
                }
                self.enum_convert(expr, ty, variants, Dir::JsToRust, depth)
            }
            _ => expr,
        }
    }

    fn rust_to_js(&mut self, expr: Expr, ty: TypeId, depth: u32) -> Expr {
        if depth >= MAX_DEPTH || !self.needs_rust_to_js_value(ty) {
            return expr;
        }
        let Some(desc) = self.cx.table.get(ty) else {
            return expr;
        };
        if let Some(info) = mires_js_hashmap_info(self.cx.table, self.cx.interns, ty) {
            return self.mires_js_hashmap_rust_to_js(expr, &info, depth);
        }
        if desc.meta.boundary == PinnedBoundary::Host {
            return expr;
        }
        if is_string_ty(self.cx.table, self.cx.interns, ty) {
            return self.string_rust_to_js(expr, ty);
        }
        if let Some((elem, is_global)) = vec_elem_and_global(self.cx.table, ty) {
            return self.vec_rust_to_js(expr, ty, elem, is_global, depth);
        }
        if let Some(info) = std_hashmap_info(self.cx.table, self.cx.interns, ty) {
            return self.std_hashmap_rust_to_js(expr, &info, depth);
        }
        if let TypeKind::Pattern(inner) = desc.kind {
            return self.rust_to_js(expr, inner, depth + 1);
        }
        if let Some(transparent) = desc.js.transparent {
            return self.rust_to_js(expr, transparent.ty, depth + 1);
        }
        match &desc.kind {
            TypeKind::Reference {
                pointee,
                mutable: false,
            } => match self.cx.table.pointer_form(ty) {
                Some(PointerForm::FatSlice) => {
                    if let Some(TypeKind::Slice(elem)) = self
                        .cx
                        .table
                        .get(self.cx.table.peeled(*pointee))
                        .map(|d| &d.kind)
                    {
                        return self.slice_rust_to_js(expr, *elem, depth);
                    }
                    expr
                }
                Some(PointerForm::Str) if self.convert_str_refs => {
                    jsast::rt_call("bstr_out", vec![expr])
                }
                Some(PointerForm::Thin) => self.rust_to_js(expr, *pointee, depth + 1),
                _ => expr,
            },
            TypeKind::Tuple(elems) => self.tuple_convert(expr, elems, Dir::RustToJs, depth),
            TypeKind::Array { element, .. } => {
                self.array_convert(expr, *element, Dir::RustToJs, depth)
            }
            TypeKind::Adt {
                form: NominalForm::Struct,
                variants,
                ..
            } => match variants.first() {
                Some(variant) => self.struct_rust_to_js(expr, &variant.fields, depth),
                None => expr,
            },
            TypeKind::Adt {
                form: NominalForm::Enum,
                variants,
                ..
            } => {
                if let Some(opt) = option_info(self.cx.table, self.cx.interns, ty) {
                    return self.option_rust_to_js(expr, &opt, depth);
                }
                self.enum_convert(expr, ty, variants, Dir::RustToJs, depth)
            }
            _ => expr,
        }
    }

    fn convert(&mut self, expr: Expr, ty: TypeId, dir: Dir, depth: u32) -> Expr {
        match dir {
            Dir::JsToRust => self.js_to_rust(expr, ty, depth),
            Dir::RustToJs => self.rust_to_js(expr, ty, depth),
        }
    }

    fn option_js_to_rust(&mut self, expr: Expr, opt: &OptionInfo, depth: u32) -> Expr {
        let Some(rust_none) = value::enum_value(
            self.cx.table,
            self.cx.interns,
            opt.ty,
            opt.none_idx,
            Vec::new(),
        ) else {
            return expr;
        };
        self.bind_once(expr, "o", |this, v| {
            let inner = if this.cx.table.is_zst(opt.inner_ty) {
                jsast::undefined()
            } else {
                this.js_to_rust(v.clone(), opt.inner_ty, depth + 1)
            };
            let field_key = field_key_str(this.cx.interns, opt.some_field_key)
                .unwrap_or_else(|| "_0".to_owned());
            let Some(rust_some) = value::enum_value(
                this.cx.table,
                this.cx.interns,
                opt.ty,
                opt.some_idx,
                vec![(field_key, inner)],
            ) else {
                return v;
            };
            let is_nullish = jsast::binary(BinOp::Eq, v, jsast::null());
            jsast::cond(is_nullish, rust_none, rust_some)
        })
    }

    fn option_rust_to_js(&mut self, expr: Expr, opt: &OptionInfo, depth: u32) -> Expr {
        self.bind_once(expr, "o", |this, v| {
            let Some(is_none) = this.enum_variant_test(opt.ty, opt.repr, opt.none_idx, v.clone())
            else {
                return v;
            };
            let payload = this.extract_variant_field(
                opt.repr,
                opt.some_idx,
                opt.some_field_key,
                opt.inner_ty,
                v,
            );
            let converted = if this.cx.table.is_zst(opt.inner_ty) {
                jsast::undefined()
            } else {
                this.rust_to_js(payload, opt.inner_ty, depth + 1)
            };
            jsast::cond(is_none, jsast::undefined(), converted)
        })
    }

    fn enum_variant_test(
        &self,
        ty: TypeId,
        repr: EnumRepr,
        variant: u32,
        val: Expr,
    ) -> Option<Expr> {
        if let Some((none_variant, _some_variant, _)) = repr.niche_variants() {
            let none_expr = value::niche_none_expr(self.cx.table, repr)?;
            let op = if variant == none_variant {
                BinOp::StrictEq
            } else {
                BinOp::StrictNe
            };
            return Some(jsast::binary(op, val, none_expr));
        }
        let tag = value::tag_expr(repr, val);
        let expected = value::variant_tag(self.cx.table, self.cx.interns, ty, variant)?;
        Some(jsast::binary(BinOp::StrictEq, tag, expected))
    }

    fn extract_variant_field(
        &self,
        repr: EnumRepr,
        variant: u32,
        field_key: FieldKey,
        field_ty: TypeId,
        val: Expr,
    ) -> Expr {
        if self.cx.table.is_zst(field_ty) {
            return jsast::undefined();
        }
        if let Some((_, some_variant, _)) = repr.niche_variants() {
            if variant == some_variant {
                return val;
            }
            return jsast::undefined();
        }
        if matches!(
            repr,
            EnumRepr::Number { .. }
                | EnumRepr::Tag { fieldless: true }
                | EnumRepr::IntTag { fieldless: true }
        ) {
            return jsast::undefined();
        }
        value::field_expr(self.cx.interns, val, field_key)
    }

    fn string_vec_field(&self, str_ty: TypeId) -> (String, Option<TypeId>, Vec<String>) {
        if let Some(fields) = struct_fields(self.cx.table, str_ty) {
            let mut vec_field = None;
            let mut extras = Vec::new();
            for f in fields {
                let Some(name) = field_key_str(self.cx.interns, f.key) else {
                    continue;
                };
                if vec_field.is_none() && (name == "vec" || !self.cx.table.is_zst(f.ty)) {
                    vec_field = Some((name, f.ty));
                } else {
                    extras.push(name);
                }
            }
            if let Some((name, vec_ty)) = vec_field {
                return (name, Some(vec_ty), extras);
            }
        }
        ("vec".to_owned(), None, Vec::new())
    }

    fn string_js_to_rust(&mut self, expr: Expr, str_ty: TypeId) -> Expr {
        if self.cx.heap.native_string_enabled {
            let s_name = self.fresh("s");
            let s_id = jsast::id(s_name.clone());
            let encoded = jsast::rt_call("bstr_in", vec![expr]);
            let obj = jsast::object_of(vec![
                ("s", s_id.clone()),
                ("cap", jsast::member(s_id, "length")),
            ]);
            return jsast::call(jsast::arrow(vec![s_name], obj), vec![encoded]);
        }

        let (vec_key, vec_ty_opt, str_extras) = self.string_vec_field(str_ty);
        let wrap_string = |vec_expr: Expr| {
            let mut entries = vec![(vec_key.clone(), vec_expr)];
            for key in &str_extras {
                entries.push((key.clone(), jsast::undefined()));
            }
            jsast::object(entries)
        };

        let bytes_call = if self.cx.bstr {
            jsast::rt_call(
                "bstr_bytes_mut",
                vec![jsast::rt_call("bstr_in", vec![expr])],
            )
        } else {
            jsast::rt_call("str_bytes_mut", vec![expr])
        };

        let use_native_vec = self.cx.heap.native_vec_enabled
            && vec_ty_opt.is_none_or(|vt| {
                self.cx
                    .table
                    .get(self.cx.table.peeled(vt))
                    .is_none_or(|d| d.meta.allocator != AllocatorIdentity::Custom)
            });

        let b_name = self.fresh("b");
        let b_id = jsast::id(b_name.clone());
        let vec_expr = if use_native_vec {
            let buf_expr = if self.cx.bstr {
                // `bstr_bytes_mut` already returns a fresh `Uint8Array`.
                jsast::member(b_id.clone(), rcgjs_object::ptr::BUF)
            } else {
                // `str_bytes_mut` returns a fresh plain JS `Array`; native `Vec<u8>` uses `Uint8Array`.
                jsast::new(
                    jsast::id("Uint8Array"),
                    vec![jsast::member(b_id.clone(), rcgjs_object::ptr::BUF)],
                )
            };
            jsast::object_of(vec![
                ("buf", buf_expr),
                ("len", jsast::member(b_id, rcgjs_object::ptr::LEN)),
            ])
        } else {
            let layout = vec_ty_opt.map_or_else(
                || raw_vec_layout(self.cx.table, self.cx.interns, str_ty),
                |vt| raw_vec_layout(self.cx.table, self.cx.interns, vt),
            );
            let len_expr = jsast::member(b_id.clone(), rcgjs_object::ptr::LEN);
            let buf_expr = if self.cx.bstr && !self.cx.heap.typed_heap_enabled {
                // RawVec debug `realloc` sets `active.length`, which requires a plain JS `Array`.
                jsast::method_call(
                    jsast::id("Array"),
                    "from",
                    vec![jsast::member(b_id, rcgjs_object::ptr::BUF)],
                )
            } else {
                jsast::member(b_id, rcgjs_object::ptr::BUF)
            };
            let non_empty_ptr = if self.cx.heap.typed_heap_enabled {
                rcgjs_object::ptr::slot(buf_expr, jsast::num(0))
            } else {
                jsast::rt_call(
                    "_register",
                    vec![buf_expr, len_expr.clone(), jsast::boolean(false)],
                )
            };
            let ptr_expr = jsast::cond(
                jsast::binary(BinOp::StrictEq, len_expr.clone(), jsast::num(0)),
                jsast::num(1),
                non_empty_ptr,
            );
            layout.build_object(ptr_expr, len_expr.clone(), len_expr)
        };
        jsast::call(
            jsast::arrow(vec![b_name], wrap_string(vec_expr)),
            vec![bytes_call],
        )
    }

    fn string_rust_to_js(&mut self, expr: Expr, str_ty: TypeId) -> Expr {
        if self.cx.heap.native_string_enabled {
            return jsast::rt_call("bstr_out", vec![jsast::member(expr, "s")]);
        }

        let (vec_key, vec_ty_opt, _) = self.string_vec_field(str_ty);
        let vec_expr = jsast::member(expr, vec_key);
        let use_native_vec = self.cx.heap.native_vec_enabled
            && vec_ty_opt.is_none_or(|vt| {
                self.cx
                    .table
                    .get(self.cx.table.peeled(vt))
                    .is_none_or(|d| d.meta.allocator != AllocatorIdentity::Custom)
            });

        let decode = |buf: Expr, off: Expr, len: Expr| -> Expr {
            if self.cx.bstr {
                jsast::rt_call(
                    "bstr_out",
                    vec![jsast::rt_call("bstr_of", vec![buf, off, len])],
                )
            } else {
                jsast::rt_call("bytes_str", vec![buf, off, len])
            }
        };

        self.bind_once(vec_expr, "sv", |this, v| {
            if use_native_vec {
                let len = jsast::member(v.clone(), "len");
                jsast::cond(
                    jsast::binary(BinOp::StrictEq, len.clone(), jsast::num(0)),
                    jsast::string(""),
                    decode(jsast::member(v, "buf"), jsast::num(0), len),
                )
            } else {
                let layout = vec_ty_opt.map_or_else(
                    || raw_vec_layout(this.cx.table, this.cx.interns, str_ty),
                    |vt| raw_vec_layout(this.cx.table, this.cx.interns, vt),
                );
                let len = layout.len_expr(v.clone());
                let ptr = layout.ptr_expr(v);
                let p_name = this.fresh("p");
                let p_id = jsast::id(p_name.clone());
                let decoded = jsast::call(
                    jsast::arrow(
                        vec![p_name],
                        decode(
                            jsast::member(p_id.clone(), rcgjs_object::ptr::BUF),
                            jsast::member(p_id, rcgjs_object::ptr::OFF),
                            len.clone(),
                        ),
                    ),
                    vec![ptr],
                );
                jsast::cond(
                    jsast::binary(BinOp::StrictEq, len, jsast::num(0)),
                    jsast::string(""),
                    decoded,
                )
            }
        })
    }

    fn vec_js_to_rust(
        &mut self,
        expr: Expr,
        vec_ty: TypeId,
        elem_ty: TypeId,
        is_global: bool,
        depth: u32,
    ) -> Expr {
        let elem_desc = self.cx.table.get(elem_ty);
        let is_zst = elem_desc.is_some_and(|d| d.layout.zst);
        let elem_size = elem_desc.and_then(|d| d.layout.size).unwrap_or(1).max(1);
        let elem_align = elem_desc.and_then(|d| d.layout.align).unwrap_or(1).max(1);
        let elem_needs = !is_zst && self.needs_js_to_rust_value(elem_ty);
        let ta_kind = if is_zst {
            None
        } else {
            crate::analysis::heap::native_vec_ta_kind(self.cx.table, elem_ty)
        };
        let peeled_elem = self.cx.table.peeled(elem_ty);
        let tracked = !is_zst
            && ta_kind.is_none()
            && self.cx.heap.needs_box_tracking(self.cx.table, elem_ty)
            && self.cx.table.is_indirect(peeled_elem)
            && value::is_slot_pointee(self.cx.table, peeled_elem);

        let elem_info = NativeVecElem {
            elem_ty,
            is_zst,
            elem_needs,
            ta_kind,
            tracked,
        };
        if self.cx.heap.native_vec_enabled && is_global {
            return self.native_vec_js_to_rust(expr, elem_info, depth);
        }

        let layout = raw_vec_layout(self.cx.table, self.cx.interns, vec_ty);
        let a_name = self.fresh("a");
        let n_name = self.fresh("n");
        let a_id = jsast::id(a_name.clone());
        let n_id = jsast::id(n_name.clone());
        let mut stmts = vec![jsast::const_(n_name, jsast::member(a_id.clone(), "length"))];
        if is_zst {
            stmts.push(jsast::ret(layout.build_object(
                jsast::num(elem_align as f64),
                jsast::num(0),
                n_id,
            )));
            return jsast::call(jsast::arrow_block(vec![a_name], stmts), vec![expr]);
        }

        let p_name = self.fresh("p");
        let p_id = jsast::id(p_name.clone());
        stmts.push(jsast::let_(p_name, jsast::num(elem_align as f64)));

        let b_name = self.fresh("b");
        let b_id = jsast::id(b_name.clone());
        let mut init_stmts = Vec::new();
        let buf_init = if elem_needs {
            let e_name = self.fresh("e");
            let converted = self.js_to_rust(jsast::id(e_name.clone()), elem_ty, depth + 1);
            let mapped =
                jsast::method_call(a_id, "map", vec![jsast::arrow(vec![e_name], converted)]);
            if self.cx.heap.typed_heap_enabled {
                if let Some(k) = ta_kind {
                    jsast::new(jsast::id(TA_CTORS[k as usize]), vec![mapped])
                } else {
                    mapped
                }
            } else {
                mapped
            }
        } else if self.cx.heap.typed_heap_enabled {
            if let Some(k) = ta_kind {
                jsast::new(jsast::id(TA_CTORS[k as usize]), vec![a_id])
            } else {
                jsast::method_call(a_id, "slice", Vec::new())
            }
        } else {
            jsast::method_call(jsast::id("Array"), "from", vec![a_id])
        };
        init_stmts.push(jsast::const_(b_name, buf_init));
        if tracked {
            init_stmts.push(jsast::expr_stmt(jsast::rt_call(
                "nv_box_track_arr",
                vec![b_id.clone()],
            )));
        }
        if self.cx.heap.typed_heap_enabled {
            init_stmts.push(jsast::assign_stmt(
                p_id.clone(),
                rcgjs_object::ptr::slot(b_id, jsast::num(0)),
            ));
        } else {
            let total_bytes = if elem_size == 1 {
                n_id.clone()
            } else {
                jsast::binary(BinOp::Mul, n_id.clone(), jsast::num(elem_size as f64))
            };
            init_stmts.push(jsast::assign_stmt(
                p_id.clone(),
                jsast::rt_call(
                    "_register",
                    vec![b_id.clone(), total_bytes, jsast::boolean(false)],
                ),
            ));
            let m_name = self.fresh("m");
            let m_id = jsast::id(m_name.clone());
            init_stmts.push(jsast::const_(
                m_name,
                jsast::method_call(
                    jsast::member(jsast::id("__rt"), "_blocks"),
                    "get",
                    vec![b_id],
                ),
            ));
            if elem_size != 1 {
                init_stmts.push(jsast::assign_stmt(
                    jsast::member(m_id.clone(), "es"),
                    jsast::num(elem_size as f64),
                ));
            }
            init_stmts.push(jsast::assign_stmt(
                jsast::member(m_id, "fresh"),
                jsast::boolean(false),
            ));
        }
        stmts.push(jsast::if_(
            jsast::binary(BinOp::Gt, n_id.clone(), jsast::num(0)),
            init_stmts,
        ));
        stmts.push(jsast::ret(layout.build_object(p_id, n_id.clone(), n_id)));
        jsast::call(jsast::arrow_block(vec![a_name], stmts), vec![expr])
    }

    fn native_vec_js_to_rust(&mut self, expr: Expr, info: NativeVecElem, depth: u32) -> Expr {
        self.bind_once(expr, "a", |this, a| {
            if info.is_zst {
                return jsast::object_of(vec![
                    ("buf", jsast::undefined()),
                    ("len", jsast::member(a, "length")),
                ]);
            }
            let len = jsast::member(a.clone(), "length");
            let mapped = if info.elem_needs {
                let e_name = this.fresh("e");
                let converted = this.js_to_rust(jsast::id(e_name.clone()), info.elem_ty, depth + 1);
                jsast::method_call(a, "map", vec![jsast::arrow(vec![e_name], converted)])
            } else if info.ta_kind.is_some() {
                a
            } else {
                jsast::method_call(a, "slice", Vec::new())
            };
            let mut buf = match info.ta_kind {
                Some(k) => jsast::new(jsast::id(TA_CTORS[k as usize]), vec![mapped]),
                None => mapped,
            };
            if info.tracked {
                buf = jsast::rt_call("nv_box_track_arr", vec![buf]);
            }
            jsast::object_of(vec![("buf", buf), ("len", len)])
        })
    }

    fn vec_rust_to_js(
        &mut self,
        expr: Expr,
        vec_ty: TypeId,
        elem_ty: TypeId,
        is_global: bool,
        depth: u32,
    ) -> Expr {
        let is_zst = self.cx.table.is_zst(elem_ty);
        let elem_needs = !is_zst && self.needs_rust_to_js_value(elem_ty);
        let native = self.cx.heap.native_vec_enabled && is_global;
        let ta_kind = if is_zst {
            None
        } else {
            crate::analysis::heap::native_vec_ta_kind(self.cx.table, elem_ty)
        };

        let v_name = self.fresh("v");
        let v_id = jsast::id(v_name.clone());
        let n_name = self.fresh("n");
        let n_id = jsast::id(n_name.clone());

        let (len_expr, ptr_expr) = if native {
            (jsast::member(v_id.clone(), "len"), None)
        } else {
            let layout = raw_vec_layout(self.cx.table, self.cx.interns, vec_ty);
            (
                layout.len_expr(v_id.clone()),
                Some(layout.ptr_expr(v_id.clone())),
            )
        };

        if is_zst {
            return jsast::call(
                jsast::arrow(vec![v_name], jsast::new(jsast::id("Array"), vec![len_expr])),
                vec![expr],
            );
        }

        let mut stmts = vec![
            jsast::const_(n_name, len_expr),
            jsast::if_(
                jsast::binary(BinOp::StrictEq, n_id.clone(), jsast::num(0)),
                vec![jsast::ret(jsast::array(Vec::new()))],
            ),
        ];

        let (buf_expr, off_expr) = if let Some(ptr) = ptr_expr {
            let p_name = self.fresh("p");
            let p_id = jsast::id(p_name.clone());
            stmts.push(jsast::const_(p_name, ptr));
            (
                jsast::member(p_id.clone(), rcgjs_object::ptr::BUF),
                Some(jsast::member(p_id, rcgjs_object::ptr::OFF)),
            )
        } else {
            (jsast::member(v_id, "buf"), None)
        };

        if !elem_needs {
            let sliced =
                self.slice_identity_array(buf_expr, off_expr, n_id, native && ta_kind.is_some());
            stmts.push(jsast::ret(sliced));
            return jsast::call(jsast::arrow_block(vec![v_name], stmts), vec![expr]);
        }

        let b_name = self.fresh("b");
        let b_id = jsast::id(b_name.clone());
        stmts.push(jsast::const_(b_name, buf_expr));
        let off_id = off_expr.map(|off| {
            let o_name = self.fresh("off");
            stmts.push(jsast::const_(o_name.clone(), off));
            jsast::id(o_name)
        });
        let out_name = self.fresh("out");
        let out_id = jsast::id(out_name.clone());
        let i_name = self.fresh("i");
        let i_id = jsast::id(i_name.clone());
        stmts.push(jsast::const_(
            out_name,
            jsast::new(jsast::id("Array"), vec![n_id.clone()]),
        ));
        stmts.push(jsast::let_(i_name, jsast::num(0)));
        let idx_expr = match off_id {
            Some(off) => jsast::binary(BinOp::Add, off, i_id.clone()),
            None => i_id.clone(),
        };
        let converted = self.rust_to_js(jsast::index(b_id, idx_expr), elem_ty, depth + 1);
        stmts.push(jsast::while_(
            jsast::binary(BinOp::Lt, i_id.clone(), n_id),
            vec![
                jsast::assign_stmt(jsast::index(out_id.clone(), i_id.clone()), converted),
                jsast::expr_stmt(jsast::compound_assign(
                    jsast::CompoundOp::Add,
                    i_id,
                    jsast::num(1),
                )),
            ],
        ));
        stmts.push(jsast::ret(out_id));
        jsast::call(jsast::arrow_block(vec![v_name], stmts), vec![expr])
    }

    fn slice_identity_array(
        &mut self,
        buf_expr: Expr,
        off_expr: Option<Expr>,
        len_expr: Expr,
        known_typed_array: bool,
    ) -> Expr {
        let b_name = self.fresh("b");
        let b_id = jsast::id(b_name.clone());
        let start = off_expr.clone().unwrap_or_else(|| jsast::num(0));
        let end = match off_expr {
            Some(off) => jsast::binary(BinOp::Add, off, len_expr),
            None => len_expr,
        };
        let body = if known_typed_array {
            jsast::method_call(
                jsast::id("Array"),
                "from",
                vec![jsast::method_call(b_id, "subarray", vec![start, end])],
            )
        } else {
            let is_view =
                jsast::method_call(jsast::id("ArrayBuffer"), "isView", vec![b_id.clone()]);
            let from_view = jsast::method_call(
                jsast::id("Array"),
                "from",
                vec![jsast::method_call(
                    b_id.clone(),
                    "subarray",
                    vec![start.clone(), end.clone()],
                )],
            );
            let from_arr = jsast::method_call(b_id, "slice", vec![start, end]);
            jsast::cond(is_view, from_view, from_arr)
        };
        jsast::call(jsast::arrow(vec![b_name], body), vec![buf_expr])
    }

    fn map_entries_js_to_rust(&mut self, expr: Expr, info: &MapEntriesInfo, depth: u32) -> Expr {
        let string_key = is_string_key_ty(self.cx.table, self.cx.interns, info.key_ty);
        let key_is_bstr_str =
            self.cx.bstr && !self.convert_str_refs && is_str_ref_ty(self.cx.table, info.key_ty);
        let entries_expr = self.bind_once(expr, "m", |_, v| {
            let is_map = jsast::method_call(
                jsast::member(jsast::id("Map"), "prototype"),
                "isPrototypeOf",
                vec![v.clone()],
            );
            let from_map = jsast::method_call(
                jsast::id("Array"),
                "from",
                vec![jsast::method_call(v.clone(), "entries", Vec::new())],
            );
            let is_nullish = jsast::binary(BinOp::Eq, v.clone(), jsast::null());
            let fallback = if string_key {
                let safe_obj = jsast::cond(is_nullish, jsast::object(Vec::new()), v);
                jsast::method_call(jsast::id("Object"), "entries", vec![safe_obj])
            } else {
                let safe_iter = jsast::cond(is_nullish, jsast::array(Vec::new()), v);
                jsast::method_call(jsast::id("Array"), "from", vec![safe_iter])
            };
            jsast::cond(is_map, from_map, fallback)
        });
        let entries_expr = if key_is_bstr_str {
            let p_name = self.fresh("p");
            let p_id = jsast::id(p_name.clone());
            let k_conv = jsast::rt_call("bstr_in", vec![jsast::index(p_id.clone(), jsast::num(0))]);
            let pair = jsast::array(vec![k_conv, jsast::index(p_id, jsast::num(1))]);
            jsast::method_call(entries_expr, "map", vec![jsast::arrow(vec![p_name], pair)])
        } else {
            entries_expr
        };
        self.vec_js_to_rust(
            entries_expr,
            info.vec_ty,
            info.pair_ty,
            info.is_global,
            depth,
        )
    }

    fn std_hashmap_rust_to_js(&mut self, expr: Expr, info: &StdHashMapInfo, depth: u32) -> Expr {
        let string_key = is_string_key_ty(self.cx.table, self.cx.interns, info.key_ty);
        let key_is_bstr_str = self.cx.bstr && is_str_ref_ty(self.cx.table, info.key_ty);
        let native = self.cx.heap.native_vec_enabled && info.slots_is_global;

        let m_name = self.fresh("hm");
        let m_id = jsast::id(m_name.clone());
        let out_name = self.fresh("out");
        let out_id = jsast::id(out_name.clone());
        let v_name = self.fresh("sv");
        let v_id = jsast::id(v_name.clone());
        let n_name = self.fresh("n");
        let n_id = jsast::id(n_name.clone());

        let init_out = if string_key {
            jsast::object(Vec::new())
        } else {
            jsast::new(jsast::id("Map"), Vec::new())
        };
        let slots_expr = value::field_expr(self.cx.interns, m_id, info.slots_field_key);
        let (len_expr, ptr_expr) = if native {
            (jsast::member(v_id.clone(), "len"), None)
        } else {
            let layout = raw_vec_layout(self.cx.table, self.cx.interns, info.slots_vec_ty);
            (
                layout.len_expr(v_id.clone()),
                Some(layout.ptr_expr(v_id.clone())),
            )
        };

        let mut stmts = vec![
            jsast::const_(out_name, init_out),
            jsast::const_(v_name, slots_expr),
            jsast::const_(n_name, len_expr),
            jsast::if_(
                jsast::binary(BinOp::StrictEq, n_id.clone(), jsast::num(0)),
                vec![jsast::ret(out_id.clone())],
            ),
        ];

        let (buf_expr, off_expr) = if let Some(ptr) = ptr_expr {
            let p_name = self.fresh("p");
            let p_id = jsast::id(p_name.clone());
            stmts.push(jsast::const_(p_name, ptr));
            (
                jsast::member(p_id.clone(), rcgjs_object::ptr::BUF),
                Some(jsast::member(p_id, rcgjs_object::ptr::OFF)),
            )
        } else {
            (jsast::member(v_id, "buf"), None)
        };

        let b_name = self.fresh("b");
        let b_id = jsast::id(b_name.clone());
        stmts.push(jsast::const_(b_name, buf_expr));
        let off_id = off_expr.map(|off| {
            let o_name = self.fresh("off");
            stmts.push(jsast::const_(o_name.clone(), off));
            jsast::id(o_name)
        });
        let i_name = self.fresh("i");
        let i_id = jsast::id(i_name.clone());
        stmts.push(jsast::let_(i_name, jsast::num(0)));

        let idx_expr = match off_id {
            Some(off) => jsast::binary(BinOp::Add, off, i_id.clone()),
            None => i_id.clone(),
        };
        let s_name = self.fresh("sl");
        let s_id = jsast::id(s_name.clone());
        let mut loop_body = vec![jsast::const_(s_name, jsast::index(b_id, idx_expr))];

        if let Some(is_occupied) = self.enum_variant_test(
            info.slot_ty,
            info.slot_repr,
            info.occupied_idx,
            s_id.clone(),
        ) {
            let raw_key = self.extract_variant_field(
                info.slot_repr,
                info.occupied_idx,
                info.key_field_key,
                info.key_ty,
                s_id.clone(),
            );
            let raw_val = self.extract_variant_field(
                info.slot_repr,
                info.occupied_idx,
                info.val_field_key,
                info.val_ty,
                s_id,
            );
            let js_key = if key_is_bstr_str {
                jsast::rt_call("bstr_out", vec![raw_key])
            } else {
                self.rust_to_js(raw_key, info.key_ty, depth + 1)
            };
            let js_val = if self.cx.table.is_zst(info.val_ty) {
                jsast::undefined()
            } else {
                self.rust_to_js(raw_val, info.val_ty, depth + 1)
            };
            let store_stmt = if string_key {
                jsast::assign_stmt(jsast::index(out_id.clone(), js_key), js_val)
            } else {
                jsast::expr_stmt(jsast::method_call(
                    out_id.clone(),
                    "set",
                    vec![js_key, js_val],
                ))
            };
            loop_body.push(jsast::if_(is_occupied, vec![store_stmt]));
        }

        loop_body.push(jsast::expr_stmt(jsast::compound_assign(
            jsast::CompoundOp::Add,
            i_id.clone(),
            jsast::num(1),
        )));
        stmts.push(jsast::while_(
            jsast::binary(BinOp::Lt, i_id, n_id),
            loop_body,
        ));
        stmts.push(jsast::ret(out_id));
        jsast::call(jsast::arrow_block(vec![m_name], stmts), vec![expr])
    }

    fn mires_js_hashmap_rust_to_js(
        &mut self,
        expr: Expr,
        info: &MiresJsHashMapInfo,
        depth: u32,
    ) -> Expr {
        let string_key = is_string_key_ty(self.cx.table, self.cx.interns, info.key_ty);
        let val_is_zst = self.cx.table.is_zst(info.val_ty);
        let opt_info = find_map_entry_option(self.cx.table, self.cx.interns, info.val_ty);

        let m_name = self.fresh("hm");
        let m_id = jsast::id(m_name.clone());
        let out_name = self.fresh("out");
        let out_id = jsast::id(out_name.clone());
        let es_name = self.fresh("es");
        let es_id = jsast::id(es_name.clone());
        let i_name = self.fresh("i");
        let i_id = jsast::id(i_name.clone());
        let p_name = self.fresh("p");
        let p_id = jsast::id(p_name.clone());

        let init_out = if string_key {
            jsast::object(Vec::new())
        } else {
            jsast::new(jsast::id("Map"), Vec::new())
        };
        let mut stmts = vec![
            jsast::const_(out_name, init_out),
            jsast::const_(
                es_name,
                jsast::method_call(
                    jsast::id("Array"),
                    "from",
                    vec![jsast::method_call(m_id, "entries", Vec::new())],
                ),
            ),
            jsast::let_(i_name, jsast::num(0)),
        ];

        let raw_key = jsast::index(p_id.clone(), jsast::num(0));
        let js_key = if string_key && self.cx.bstr {
            jsast::rt_call("bstr_out", vec![raw_key])
        } else {
            raw_key
        };

        let raw_entry = jsast::index(p_id, jsast::num(1));
        let mut loop_body = vec![jsast::const_(
            p_name,
            jsast::index(es_id.clone(), i_id.clone()),
        )];

        let opt_name = self.fresh("ev");
        let opt_id = jsast::id(opt_name.clone());
        loop_body.push(jsast::const_(opt_name, jsast::member(raw_entry, "value")));

        let store_stmt = |this: &mut Self, raw_val: Expr| -> Stmt {
            let js_val = if val_is_zst {
                jsast::undefined()
            } else {
                this.rust_to_js(raw_val, info.val_ty, depth + 1)
            };
            if string_key {
                jsast::assign_stmt(jsast::index(out_id.clone(), js_key.clone()), js_val)
            } else {
                jsast::expr_stmt(jsast::method_call(
                    out_id.clone(),
                    "set",
                    vec![js_key.clone(), js_val],
                ))
            }
        };

        if let Some(opt) = opt_info {
            if let Some(is_none) =
                self.enum_variant_test(opt.ty, opt.repr, opt.none_idx, opt_id.clone())
            {
                let payload = self.extract_variant_field(
                    opt.repr,
                    opt.some_idx,
                    opt.some_field_key,
                    opt.inner_ty,
                    opt_id,
                );
                let stmt = store_stmt(self, payload);
                loop_body.push(jsast::if_(jsast::not(is_none), vec![stmt]));
            } else {
                let stmt = store_stmt(self, opt_id);
                loop_body.push(stmt);
            }
        } else {
            let (is_none, unwrapped) = self.dynamic_map_entry_unwrap(opt_id, info.val_ty);
            let stmt = store_stmt(self, unwrapped);
            loop_body.push(jsast::if_(jsast::not(is_none), vec![stmt]));
        }

        loop_body.push(jsast::expr_stmt(jsast::compound_assign(
            jsast::CompoundOp::Add,
            i_id.clone(),
            jsast::num(1),
        )));
        stmts.push(jsast::while_(
            jsast::binary(BinOp::Lt, i_id, jsast::member(es_id, "length")),
            loop_body,
        ));
        stmts.push(jsast::ret(out_id));
        jsast::call(jsast::arrow_block(vec![m_name], stmts), vec![expr])
    }

    fn dynamic_map_entry_unwrap(&self, opt_id: Expr, val_ty: TypeId) -> (Expr, Expr) {
        let val_is_zst = self.cx.table.is_zst(val_ty);
        let peeled_val = self.cx.table.peeled(val_ty);
        let val_is_tagged = matches!(
            self.cx.table.enum_repr(peeled_val),
            Some(
                EnumRepr::Tag { fieldless: false }
                    | EnumRepr::IntTag { fieldless: false }
                    | EnumRepr::State
            )
        );
        let is_obj = jsast::binary(
            BinOp::And,
            jsast::binary(
                BinOp::StrictEq,
                jsast::unary(UnOp::TypeOf, opt_id.clone()),
                jsast::string("object"),
            ),
            jsast::binary(BinOp::StrictNe, opt_id.clone(), jsast::null()),
        );
        let mut is_none = if val_is_zst {
            jsast::binary(
                BinOp::Or,
                jsast::binary(BinOp::StrictEq, opt_id.clone(), jsast::string("None")),
                jsast::binary(BinOp::StrictEq, opt_id.clone(), jsast::num(0)),
            )
        } else {
            jsast::binary(BinOp::StrictEq, opt_id.clone(), jsast::null())
        };
        if let Some(sn) = value::sentinel_none_expr(self.cx.table) {
            is_none = jsast::binary(
                BinOp::Or,
                is_none,
                jsast::binary(BinOp::StrictEq, opt_id.clone(), sn),
            );
        }
        if !val_is_tagged {
            let tag_is_none = jsast::binary(
                BinOp::And,
                is_obj.clone(),
                jsast::binary(
                    BinOp::Or,
                    jsast::binary(
                        BinOp::StrictEq,
                        jsast::member(opt_id.clone(), value::TAG),
                        jsast::string("None"),
                    ),
                    jsast::binary(
                        BinOp::StrictEq,
                        jsast::member(opt_id.clone(), value::TAG),
                        jsast::num(0),
                    ),
                ),
            );
            is_none = jsast::binary(BinOp::Or, is_none, tag_is_none);
        }
        let unwrapped = if val_is_tagged {
            let has_some_variant = self
                .cx
                .table
                .variants(peeled_val)
                .iter()
                .any(|v| self.cx.interns.str(v.name) == Some("Some"));
            if has_some_variant {
                opt_id
            } else {
                let tag_is_some = jsast::binary(
                    BinOp::And,
                    is_obj,
                    jsast::binary(
                        BinOp::StrictEq,
                        jsast::member(opt_id.clone(), value::TAG),
                        jsast::string("Some"),
                    ),
                );
                jsast::cond(tag_is_some, jsast::member(opt_id.clone(), "_0"), opt_id)
            }
        } else {
            let tag_is_some = jsast::binary(
                BinOp::And,
                is_obj,
                jsast::binary(
                    BinOp::Or,
                    jsast::binary(
                        BinOp::StrictEq,
                        jsast::member(opt_id.clone(), value::TAG),
                        jsast::string("Some"),
                    ),
                    jsast::binary(
                        BinOp::StrictEq,
                        jsast::member(opt_id.clone(), value::TAG),
                        jsast::num(1),
                    ),
                ),
            );
            jsast::cond(tag_is_some, jsast::member(opt_id.clone(), "_0"), opt_id)
        };
        (is_none, unwrapped)
    }

    fn slice_js_to_rust(&mut self, expr: Expr, elem_ty: TypeId, depth: u32) -> Expr {
        let is_zst = self.cx.table.is_zst(elem_ty);
        let elem_needs = !is_zst && self.needs_js_to_rust_value(elem_ty);
        self.bind_once(expr, "a", |this, a| {
            let len = jsast::member(a.clone(), "length");
            let buf = if is_zst {
                jsast::undefined()
            } else if elem_needs {
                let e_name = this.fresh("e");
                let converted = this.js_to_rust(jsast::id(e_name.clone()), elem_ty, depth + 1);
                jsast::method_call(a, "map", vec![jsast::arrow(vec![e_name], converted)])
            } else {
                a
            };
            rcgjs_object::ptr::fat_slice(buf, jsast::num(0), len)
        })
    }

    fn slice_rust_to_js(&mut self, expr: Expr, elem_ty: TypeId, depth: u32) -> Expr {
        let is_zst = self.cx.table.is_zst(elem_ty);
        let elem_needs = !is_zst && self.needs_rust_to_js_value(elem_ty);
        let s_name = self.fresh("sl");
        let s_id = jsast::id(s_name.clone());
        let n_name = self.fresh("n");
        let n_id = jsast::id(n_name.clone());

        if is_zst {
            return jsast::call(
                jsast::arrow(
                    vec![s_name],
                    jsast::new(
                        jsast::id("Array"),
                        vec![jsast::member(s_id, rcgjs_object::ptr::LEN)],
                    ),
                ),
                vec![expr],
            );
        }

        let mut stmts = vec![
            jsast::const_(n_name, jsast::member(s_id.clone(), rcgjs_object::ptr::LEN)),
            jsast::if_(
                jsast::binary(BinOp::StrictEq, n_id.clone(), jsast::num(0)),
                vec![jsast::ret(jsast::array(Vec::new()))],
            ),
        ];

        if !elem_needs {
            let sliced = self.slice_identity_array(
                jsast::member(s_id.clone(), rcgjs_object::ptr::BUF),
                Some(jsast::member(s_id, rcgjs_object::ptr::OFF)),
                n_id,
                false,
            );
            stmts.push(jsast::ret(sliced));
            return jsast::call(jsast::arrow_block(vec![s_name], stmts), vec![expr]);
        }

        let b_name = self.fresh("b");
        let b_id = jsast::id(b_name.clone());
        let off_name = self.fresh("off");
        let off_id = jsast::id(off_name.clone());
        let out_name = self.fresh("out");
        let out_id = jsast::id(out_name.clone());
        let i_name = self.fresh("i");
        let i_id = jsast::id(i_name.clone());
        stmts.push(jsast::const_(
            b_name,
            jsast::member(s_id.clone(), rcgjs_object::ptr::BUF),
        ));
        stmts.push(jsast::const_(
            off_name,
            jsast::member(s_id, rcgjs_object::ptr::OFF),
        ));
        stmts.push(jsast::const_(
            out_name,
            jsast::new(jsast::id("Array"), vec![n_id.clone()]),
        ));
        stmts.push(jsast::let_(i_name, jsast::num(0)));
        let converted = self.rust_to_js(
            jsast::index(b_id, jsast::binary(BinOp::Add, off_id, i_id.clone())),
            elem_ty,
            depth + 1,
        );
        stmts.push(jsast::while_(
            jsast::binary(BinOp::Lt, i_id.clone(), n_id),
            vec![
                jsast::assign_stmt(jsast::index(out_id.clone(), i_id.clone()), converted),
                jsast::expr_stmt(jsast::compound_assign(
                    jsast::CompoundOp::Add,
                    i_id,
                    jsast::num(1),
                )),
            ],
        ));
        stmts.push(jsast::ret(out_id));
        jsast::call(jsast::arrow_block(vec![s_name], stmts), vec![expr])
    }

    fn struct_js_to_rust(&mut self, expr: Expr, fields: &[FieldDescriptor], depth: u32) -> Expr {
        let v_name = self.fresh("st");
        let v_id = jsast::id(v_name.clone());
        let mut entries = Vec::with_capacity(fields.len());
        for field in fields {
            let Some(key) = field_key_str(self.cx.interns, field.key) else {
                continue;
            };
            let val = if self.cx.table.is_zst(field.ty) {
                jsast::undefined()
            } else {
                let raw = jsast::member(v_id.clone(), key.clone());
                self.js_to_rust(raw, field.ty, depth + 1)
            };
            entries.push((key, val));
        }
        // Default `null`/`undefined` options objects to `{}` so omitted optional properties become
        // `None` rather than throwing a `TypeError` on property read.
        let safe_arg = self.bind_once(expr, "in", |_, raw| {
            jsast::cond(
                jsast::binary(BinOp::Eq, raw.clone(), jsast::null()),
                jsast::object(Vec::new()),
                raw,
            )
        });
        jsast::call(
            jsast::arrow(vec![v_name], jsast::object(entries)),
            vec![safe_arg],
        )
    }

    fn struct_rust_to_js(&mut self, expr: Expr, fields: &[FieldDescriptor], depth: u32) -> Expr {
        self.bind_once(expr, "st", |this, v| {
            let mut entries = Vec::with_capacity(fields.len());
            for field in fields {
                let Some(key) = field_key_str(this.cx.interns, field.key) else {
                    continue;
                };
                let val = if this.cx.table.is_zst(field.ty) {
                    jsast::undefined()
                } else {
                    let raw = jsast::member(v.clone(), key.clone());
                    this.rust_to_js(raw, field.ty, depth + 1)
                };
                entries.push((key, val));
            }
            jsast::object(entries)
        })
    }

    fn tuple_convert(&mut self, expr: Expr, elems: &[TypeId], dir: Dir, depth: u32) -> Expr {
        self.bind_once(expr, "tup", |this, v| {
            let items = elems
                .iter()
                .enumerate()
                .map(|(i, &elem_ty)| {
                    if this.cx.table.is_zst(elem_ty) {
                        jsast::undefined()
                    } else {
                        let raw = jsast::index(v.clone(), jsast::num(i as f64));
                        this.convert(raw, elem_ty, dir, depth + 1)
                    }
                })
                .collect();
            jsast::array(items)
        })
    }

    fn array_convert(&mut self, expr: Expr, elem_ty: TypeId, dir: Dir, depth: u32) -> Expr {
        let e_name = self.fresh("e");
        let converted = self.convert(jsast::id(e_name.clone()), elem_ty, dir, depth + 1);
        jsast::method_call(expr, "map", vec![jsast::arrow(vec![e_name], converted)])
    }

    fn enum_convert(
        &mut self,
        expr: Expr,
        ty: TypeId,
        variants: &[VariantDescriptor],
        dir: Dir,
        depth: u32,
    ) -> Expr {
        let peeled = self.cx.table.peeled(ty);
        let Some(repr) = self.cx.table.enum_repr(peeled) else {
            return expr;
        };
        if let Some((_, some_variant, some_field)) = repr.niche_variants() {
            let Some(none_expr) = value::niche_none_expr(self.cx.table, repr) else {
                return expr;
            };
            let Some(payload_ty) = variants
                .get(some_variant as usize)
                .and_then(|v| v.fields.get(some_field as usize))
                .map(|f| f.ty)
            else {
                return expr;
            };
            return self.bind_once(expr, "en", |this, v| {
                let converted = this.convert(v.clone(), payload_ty, dir, depth + 1);
                jsast::cond(
                    jsast::binary(BinOp::StrictEq, v, none_expr.clone()),
                    none_expr,
                    converted,
                )
            });
        }
        if !matches!(
            repr,
            EnumRepr::Tag { fieldless: false } | EnumRepr::IntTag { fieldless: false }
        ) {
            return expr;
        }
        let v_name = self.fresh("en");
        let v_id = jsast::id(v_name.clone());
        let mut cases = Vec::new();
        for (idx, variant) in variants.iter().enumerate() {
            let variant_needs = variant.fields.iter().any(|f| match dir {
                Dir::JsToRust => self.needs_js_to_rust_value(f.ty),
                Dir::RustToJs => self.needs_rust_to_js_value(f.ty),
            });
            if !variant_needs {
                continue;
            }
            let Some(tag) = value::variant_tag(self.cx.table, self.cx.interns, peeled, idx as u32)
            else {
                continue;
            };
            let mut entries = vec![(value::TAG.to_owned(), tag.clone())];
            for field in &variant.fields {
                let Some(key) = field_key_str(self.cx.interns, field.key) else {
                    continue;
                };
                let val = if self.cx.table.is_zst(field.ty) {
                    jsast::undefined()
                } else {
                    let raw = jsast::member(v_id.clone(), key.clone());
                    self.convert(raw, field.ty, dir, depth + 1)
                };
                entries.push((key, val));
            }
            cases.push(jsast::case(
                vec![tag],
                vec![jsast::ret(jsast::object(entries))],
            ));
        }
        if cases.is_empty() {
            return expr;
        }
        cases.push(jsast::default_case(vec![jsast::ret(v_id.clone())]));
        let sw = jsast::switch(value::tag_expr(repr, v_id), cases);
        jsast::call(jsast::arrow_block(vec![v_name], vec![sw]), vec![expr])
    }

    #[cfg(test)]
    fn rewrite_returns_value(&mut self, stmts: &mut [Stmt], ret_ty: TypeId) {
        for stmt in stmts.iter_mut() {
            match stmt {
                Stmt::Return(Some(expr)) => {
                    let taken = std::mem::replace(expr, Expr::Undefined);
                    *expr = self.rust_to_js(taken, ret_ty, 0);
                }
                Stmt::Block(inner) | Stmt::LoopForever(_, inner) | Stmt::While(_, inner) => {
                    self.rewrite_returns_value(inner, ret_ty);
                }
                Stmt::If(_, then_body, else_body) => {
                    self.rewrite_returns_value(then_body, ret_ty);
                    if let Some(els) = else_body {
                        self.rewrite_returns_value(els, ret_ty);
                    }
                }
                Stmt::Switch(_, cases) => {
                    for case in cases {
                        self.rewrite_returns_value(&mut case.body, ret_ty);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    self.rewrite_returns_value(std::slice::from_mut(inner.as_mut()), ret_ty);
                }
                Stmt::TryCatch { body, handler, .. } => {
                    self.rewrite_returns_value(body, ret_ty);
                    self.rewrite_returns_value(handler, ret_ty);
                }
                Stmt::FunctionDecl { .. } => {}
                _ => {}
            }
        }
    }

    #[cfg(test)]
    fn rewrite_returns_result(&mut self, stmts: &mut Vec<Stmt>, res: &ResultInfo) {
        let mut out = Vec::with_capacity(stmts.len());
        for stmt in std::mem::take(stmts) {
            match stmt {
                Stmt::Return(Some(expr)) => {
                    out.extend(self.result_return_stmts(expr, res));
                }
                Stmt::Block(mut inner) => {
                    self.rewrite_returns_result(&mut inner, res);
                    out.push(Stmt::Block(inner));
                }
                Stmt::LoopForever(label, mut inner) => {
                    self.rewrite_returns_result(&mut inner, res);
                    out.push(Stmt::LoopForever(label, inner));
                }
                Stmt::While(cond, mut inner) => {
                    self.rewrite_returns_result(&mut inner, res);
                    out.push(Stmt::While(cond, inner));
                }
                Stmt::If(cond, mut then_body, else_body) => {
                    self.rewrite_returns_result(&mut then_body, res);
                    let else_body = else_body.map(|mut els| {
                        self.rewrite_returns_result(&mut els, res);
                        els
                    });
                    out.push(Stmt::If(cond, then_body, else_body));
                }
                Stmt::Switch(disc, mut cases) => {
                    for case in &mut cases {
                        self.rewrite_returns_result(&mut case.body, res);
                    }
                    out.push(Stmt::Switch(disc, cases));
                }
                Stmt::Labeled(label, inner) => {
                    let mut single = vec![*inner];
                    self.rewrite_returns_result(&mut single, res);
                    let wrapped = if single.len() == 1 {
                        single.pop().expect("one statement")
                    } else {
                        Stmt::Block(single)
                    };
                    out.push(Stmt::Labeled(label, Box::new(wrapped)));
                }
                Stmt::TryCatch {
                    mut body,
                    binding,
                    mut handler,
                } => {
                    self.rewrite_returns_result(&mut body, res);
                    self.rewrite_returns_result(&mut handler, res);
                    out.push(Stmt::TryCatch {
                        body,
                        binding,
                        handler,
                    });
                }
                Stmt::FunctionDecl { .. } => out.push(stmt),
                other => out.push(other),
            }
        }
        *stmts = out;
    }

    fn result_return_stmts(&mut self, expr: Expr, res: &ResultInfo) -> Vec<Stmt> {
        let r_name = self.fresh("r");
        let r_id = jsast::id(r_name.clone());
        let Some(is_ok) = self.enum_variant_test(res.ty, res.repr, res.ok_idx, r_id.clone()) else {
            return vec![jsast::ret(expr)];
        };
        let ok_ret = if self.cx.table.is_zst(res.ok_ty) {
            jsast::ret_void()
        } else {
            let ok_val = self.extract_variant_field(
                res.repr,
                res.ok_idx,
                res.ok_field_key,
                res.ok_ty,
                r_id.clone(),
            );
            jsast::ret(self.rust_to_js(ok_val, res.ok_ty, 0))
        };
        let err_val =
            self.extract_variant_field(res.repr, res.err_idx, res.err_field_key, res.err_ty, r_id);
        let err_js = self.rust_to_js(err_val, res.err_ty, 0);
        let e_name = self.fresh("e");
        let e_id = jsast::id(e_name.clone());
        let thrown = build_throw_error_expr(e_id);
        vec![
            jsast::const_(r_name, expr),
            jsast::if_(is_ok, vec![ok_ret]),
            jsast::const_(e_name, err_js),
            jsast::throw(thrown),
        ]
    }
}

/// Converts a marshaled JavaScript error value `e` into a throwable `Error` instance:
/// - string -> `new Error(e)`
/// - `Error` instance -> `e` unchanged
/// - object with string `.message` -> `Object.assign(new Error(e.message), { cause: e })`
/// - anything else -> `Object.assign(new Error(String(e)), { cause: e })`
fn build_throw_error_expr(e: Expr) -> Expr {
    let is_string = jsast::binary(
        BinOp::StrictEq,
        jsast::unary(UnOp::TypeOf, e.clone()),
        jsast::string("string"),
    );
    let is_error = jsast::method_call(
        jsast::member(jsast::id("Error"), "prototype"),
        "isPrototypeOf",
        vec![e.clone()],
    );
    let has_message = jsast::binary(
        BinOp::And,
        jsast::binary(
            BinOp::And,
            jsast::binary(
                BinOp::StrictEq,
                jsast::unary(UnOp::TypeOf, e.clone()),
                jsast::string("object"),
            ),
            jsast::binary(BinOp::StrictNe, e.clone(), jsast::null()),
        ),
        jsast::binary(
            BinOp::StrictEq,
            jsast::unary(UnOp::TypeOf, jsast::member(e.clone(), "message")),
            jsast::string("string"),
        ),
    );
    let msg = jsast::cond(
        has_message,
        jsast::member(e.clone(), "message"),
        jsast::call(jsast::id("String"), vec![e.clone()]),
    );
    let wrapped = jsast::method_call(
        jsast::id("Object"),
        "assign",
        vec![
            jsast::new(jsast::id("Error"), vec![msg]),
            jsast::object_of(vec![("cause", e.clone())]),
        ],
    );
    jsast::cond(
        is_string,
        jsast::new(jsast::id("Error"), vec![e.clone()]),
        jsast::cond(is_error, e, wrapped),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;

    use rcgjs_object::jir::interns::Interns;
    use rcgjs_object::jir::repr::Func;
    use rcgjs_object::jir::semantic::{AbiSignature, DefinitionId, ItemId, Signature};
    use rcgjs_object::jir::types::{
        AllocatorIdentity, EnumRepr, FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts,
        LayoutFacts, NominalAbi, NominalForm, PinnedBoundary, PointerForm, RepresentationCandidate,
        TransparentField, TypeDescriptor, TypeId, TypeKind, TypeMeta, VariantDescriptor,
    };
    use rcgjs_object::jsast::{self, Stmt};

    use crate::analysis::heap::HeapPlan;

    use super::{
        build_js_export_wrapper, lower_export_out_body, needs_js_export_wrapper, needs_js_to_rust,
        needs_rust_to_js, wrap_js_export, BoundaryCx,
    };

    struct TestTypes {
        interns: Interns,
        next_ty: u128,
        next_def: u64,
    }

    impl TestTypes {
        fn new() -> Self {
            Self {
                interns: Interns::new(),
                next_ty: 1,
                next_def: 1,
            }
        }

        fn id(&mut self) -> TypeId {
            let id = TypeId(self.next_ty);
            self.next_ty += 1;
            id
        }

        fn def(&mut self) -> DefinitionId {
            let def = DefinitionId {
                crate_id: 1,
                local_hash: self.next_def,
            };
            self.next_def += 1;
            def
        }

        fn insert(&mut self, desc: TypeDescriptor) -> TypeId {
            let id = desc.id;
            self.interns.intern_type(desc);
            id
        }

        fn unit(&mut self) -> TypeId {
            let id = self.id();
            self.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Tuple(Vec::new()),
                layout: LayoutFacts {
                    size: Some(0),
                    align: Some(1),
                    sized: true,
                    zst: true,
                    one_zst: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
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
                },
            })
        }

        fn bool_ty(&mut self) -> TypeId {
            let id = self.id();
            self.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Bool,
                layout: LayoutFacts {
                    size: Some(1),
                    align: Some(1),
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
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
                },
            })
        }

        fn int(&mut self, signed: bool, bits: u32) -> TypeId {
            let id = self.id();
            let bytes = u64::from(bits / 8);
            self.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Integer { signed, bits },
                layout: LayoutFacts {
                    size: Some(bytes),
                    align: Some(bytes),
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: Some(IntFacts {
                        signed,
                        bits,
                        repr: if bits > 32 {
                            IntRepr::BigInt
                        } else {
                            IntRepr::Number
                        },
                    }),
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: id,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: bits == 8,
                    fields: Vec::new(),
                },
            })
        }

        fn host_value(&mut self, u32_ty: TypeId) -> TypeId {
            let id = self.id();
            let def = self.def();
            let name = self.interns.intern_str("JsValue");
            let handle = self.interns.intern_str("handle");
            let field = FieldDescriptor {
                key: FieldKey::Name(handle),
                ty: u32_ty,
                offset: 0,
            };
            self.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Adt {
                    def,
                    args: Vec::new(),
                    form: NominalForm::Struct,
                    variants: vec![VariantDescriptor {
                        name,
                        fields: vec![field.clone()],
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    }],
                    niche: None,
                },
                layout: LayoutFacts {
                    size: Some(4),
                    align: Some(4),
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta {
                    abi: NominalAbi::Transparent,
                    boundary: PinnedBoundary::Host,
                    ..TypeMeta::default()
                },
                js: JsFacts {
                    int: None,
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: Some(TransparentField {
                        field: 0,
                        ty: u32_ty,
                    }),
                    peeled: u32_ty,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: vec![field],
                },
            })
        }

        fn str_ref(&mut self) -> TypeId {
            let str_id = self.id();
            self.insert(TypeDescriptor {
                id: str_id,
                name: None,
                kind: TypeKind::Str,
                layout: LayoutFacts::default(),
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: None,
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: str_id,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: Vec::new(),
                },
            });
            let ref_id = self.id();
            self.insert(TypeDescriptor {
                id: ref_id,
                name: None,
                kind: TypeKind::Reference {
                    pointee: str_id,
                    mutable: false,
                },
                layout: LayoutFacts {
                    size: Some(8),
                    align: Some(4),
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: None,
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: ref_id,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: Some(PointerForm::Str),
                    byte_element: false,
                    fields: Vec::new(),
                },
            })
        }

        fn slice_ref(&mut self, elem: TypeId) -> TypeId {
            let slice_id = self.id();
            self.insert(TypeDescriptor {
                id: slice_id,
                name: None,
                kind: TypeKind::Slice(elem),
                layout: LayoutFacts::default(),
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: None,
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: slice_id,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: Vec::new(),
                },
            });
            let ref_id = self.id();
            self.insert(TypeDescriptor {
                id: ref_id,
                name: None,
                kind: TypeKind::Reference {
                    pointee: slice_id,
                    mutable: false,
                },
                layout: LayoutFacts {
                    size: Some(8),
                    align: Some(4),
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: None,
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: ref_id,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: Some(PointerForm::FatSlice),
                    byte_element: false,
                    fields: Vec::new(),
                },
            })
        }

        fn option(&mut self, inner: TypeId, repr: EnumRepr) -> TypeId {
            let id = self.id();
            let def = self.def();
            let s_none = self.interns.intern_str("None");
            let s_some = self.interns.intern_str("Some");
            let s_0 = self.interns.intern_str("_0");
            self.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Adt {
                    def,
                    args: vec![inner],
                    form: NominalForm::Enum,
                    variants: vec![
                        VariantDescriptor {
                            name: s_none,
                            fields: Vec::new(),
                            logical_discr: 0,
                            physical_tag: None,
                            result_ok: false,
                            saved_local_keys: Vec::new(),
                        },
                        VariantDescriptor {
                            name: s_some,
                            fields: vec![FieldDescriptor {
                                key: FieldKey::Name(s_0),
                                ty: inner,
                                offset: 0,
                            }],
                            logical_discr: 1,
                            physical_tag: None,
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
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: None,
                    is_indirect: !repr.is_null_or_sentinel_niche(),
                    needs_clone: !repr.is_null_or_sentinel_niche(),
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: id,
                    unsized_tail: None,
                    enum_repr: Some(repr),
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: Vec::new(),
                },
            })
        }

        fn result(&mut self, ok: TypeId, err: TypeId) -> TypeId {
            let id = self.id();
            let def = self.def();
            let s_ok = self.interns.intern_str("Ok");
            let s_err = self.interns.intern_str("Err");
            let s_0 = self.interns.intern_str("_0");
            self.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Adt {
                    def,
                    args: vec![ok, err],
                    form: NominalForm::Enum,
                    variants: vec![
                        VariantDescriptor {
                            name: s_ok,
                            fields: vec![FieldDescriptor {
                                key: FieldKey::Name(s_0),
                                ty: ok,
                                offset: 0,
                            }],
                            logical_discr: 0,
                            physical_tag: None,
                            result_ok: true,
                            saved_local_keys: Vec::new(),
                        },
                        VariantDescriptor {
                            name: s_err,
                            fields: vec![FieldDescriptor {
                                key: FieldKey::Name(s_0),
                                ty: err,
                                offset: 0,
                            }],
                            logical_discr: 1,
                            physical_tag: None,
                            result_ok: false,
                            saved_local_keys: Vec::new(),
                        },
                    ],
                    niche: None,
                },
                layout: LayoutFacts {
                    size: Some(16),
                    align: Some(4),
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: None,
                    is_indirect: true,
                    needs_clone: true,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: id,
                    unsized_tail: None,
                    enum_repr: Some(EnumRepr::Tag { fieldless: false }),
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: Vec::new(),
                },
            })
        }

        fn vec(&mut self, elem: TypeId, usize_ty: TypeId, unit_ty: TypeId) -> TypeId {
            let ptr_ty = self.id();
            self.insert(TypeDescriptor {
                id: ptr_ty,
                name: None,
                kind: TypeKind::RawPointer {
                    pointee: elem,
                    mutable: true,
                },
                layout: LayoutFacts {
                    size: Some(4),
                    align: Some(4),
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: None,
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: ptr_ty,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: Some(PointerForm::Slot),
                    byte_element: false,
                    fields: Vec::new(),
                },
            });
            let inner_ty = self.plain_struct(
                "RawVecInner",
                &[("ptr", ptr_ty), ("cap", usize_ty), ("alloc", unit_ty)],
            );
            let raw_ty = self.plain_struct("RawVec", &[("inner", inner_ty), ("_marker", unit_ty)]);
            let id = self.id();
            let def = self.def();
            let s_vec = self.interns.intern_str("Vec");
            let s_buf = self.interns.intern_str("buf");
            let s_len = self.interns.intern_str("len");
            let fields = vec![
                FieldDescriptor {
                    key: FieldKey::Name(s_buf),
                    ty: raw_ty,
                    offset: 0,
                },
                FieldDescriptor {
                    key: FieldKey::Name(s_len),
                    ty: usize_ty,
                    offset: 8,
                },
            ];
            self.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Adt {
                    def,
                    args: vec![elem],
                    form: NominalForm::Struct,
                    variants: vec![VariantDescriptor {
                        name: s_vec,
                        fields: fields.clone(),
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
                    ..LayoutFacts::default()
                },
                meta: TypeMeta {
                    candidate: RepresentationCandidate::Vec,
                    allocator: AllocatorIdentity::Global,
                    ..TypeMeta::default()
                },
                js: JsFacts {
                    int: None,
                    is_indirect: true,
                    needs_clone: true,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: id,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields,
                },
            })
        }

        fn string(&mut self, u8_ty: TypeId, usize_ty: TypeId, unit_ty: TypeId) -> TypeId {
            let vec_u8 = self.vec(u8_ty, usize_ty, unit_ty);
            let id = self.id();
            let def = self.def();
            let s_string = self.interns.intern_str("String");
            let s_vec = self.interns.intern_str("vec");
            let fields = vec![FieldDescriptor {
                key: FieldKey::Name(s_vec),
                ty: vec_u8,
                offset: 0,
            }];
            self.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Adt {
                    def,
                    args: Vec::new(),
                    form: NominalForm::Struct,
                    variants: vec![VariantDescriptor {
                        name: s_string,
                        fields: fields.clone(),
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
                    ..LayoutFacts::default()
                },
                meta: TypeMeta {
                    candidate: RepresentationCandidate::String,
                    allocator: AllocatorIdentity::Global,
                    ..TypeMeta::default()
                },
                js: JsFacts {
                    int: None,
                    is_indirect: true,
                    needs_clone: true,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: id,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields,
                },
            })
        }

        fn plain_struct(&mut self, name: &str, fields: &[(&str, TypeId)]) -> TypeId {
            let id = self.id();
            let def = self.def();
            let s_name = self.interns.intern_str(name);
            let fds: Vec<FieldDescriptor> = fields
                .iter()
                .enumerate()
                .map(|(i, (fname, fty))| FieldDescriptor {
                    key: FieldKey::Name(self.interns.intern_str(fname)),
                    ty: *fty,
                    offset: (i * 4) as u64,
                })
                .collect();
            self.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Adt {
                    def,
                    args: Vec::new(),
                    form: NominalForm::Struct,
                    variants: vec![VariantDescriptor {
                        name: s_name,
                        fields: fds.clone(),
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    }],
                    niche: None,
                },
                layout: LayoutFacts {
                    size: Some((fields.len().max(1) * 4) as u64),
                    align: Some(4),
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: None,
                    is_indirect: true,
                    needs_clone: true,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: id,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: fds,
                },
            })
        }

        fn tuple(&mut self, elems: Vec<TypeId>) -> TypeId {
            let id = self.id();
            let fields: Vec<FieldDescriptor> = elems
                .iter()
                .enumerate()
                .map(|(i, &ty)| FieldDescriptor {
                    key: FieldKey::Index(i as u32),
                    ty,
                    offset: (i * 4) as u64,
                })
                .collect();
            self.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Tuple(elems.clone()),
                layout: LayoutFacts {
                    size: Some((elems.len().max(1) * 4) as u64),
                    align: Some(4),
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: None,
                    is_indirect: true,
                    needs_clone: true,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: id,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields,
                },
            })
        }

        fn raw_mut_ptr(&mut self, pointee: TypeId) -> TypeId {
            let id = self.id();
            self.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::RawPointer {
                    pointee,
                    mutable: true,
                },
                layout: LayoutFacts {
                    size: Some(4),
                    align: Some(4),
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: None,
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: id,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: Some(PointerForm::Slot),
                    byte_element: false,
                    fields: Vec::new(),
                },
            })
        }

        fn typed_promise(&mut self, inner: TypeId, u32_ty: TypeId) -> TypeId {
            let promise_ty = self.host_value(u32_ty);
            let id = self.id();
            let def = self.def();
            let name = self.interns.intern_str("TypedPromise");
            let inner_key = self.interns.intern_str("inner");
            let field = FieldDescriptor {
                key: FieldKey::Name(inner_key),
                ty: promise_ty,
                offset: 0,
            };
            self.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Adt {
                    def,
                    args: vec![inner],
                    form: NominalForm::Struct,
                    variants: vec![VariantDescriptor {
                        name,
                        fields: vec![field.clone()],
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    }],
                    niche: None,
                },
                layout: LayoutFacts {
                    size: Some(4),
                    align: Some(4),
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta {
                    abi: NominalAbi::Transparent,
                    boundary: PinnedBoundary::Host,
                    ..TypeMeta::default()
                },
                js: JsFacts {
                    int: None,
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: Some(TransparentField {
                        field: 0,
                        ty: promise_ty,
                    }),
                    peeled: u32_ty,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: vec![field],
                },
            })
        }

        fn map_entries(
            &mut self,
            variant_name: &str,
            key_ty: TypeId,
            val_ty: TypeId,
            usize_ty: TypeId,
            unit_ty: TypeId,
        ) -> TypeId {
            let pair_ty = self.tuple(vec![key_ty, val_ty]);
            let vec_ty = self.vec(pair_ty, usize_ty, unit_ty);
            let id = self.id();
            let def = self.def();
            let v_name = self.interns.intern_str(variant_name);
            let field = FieldDescriptor {
                key: FieldKey::Transparent,
                ty: vec_ty,
                offset: 0,
            };
            self.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Adt {
                    def,
                    args: vec![key_ty, val_ty],
                    form: NominalForm::Struct,
                    variants: vec![VariantDescriptor {
                        name: v_name,
                        fields: vec![field.clone()],
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
                    ..LayoutFacts::default()
                },
                meta: TypeMeta {
                    abi: NominalAbi::Transparent,
                    boundary: PinnedBoundary::Host,
                    ..TypeMeta::default()
                },
                js: JsFacts {
                    int: None,
                    is_indirect: true,
                    needs_clone: true,
                    crosses_transparent_union: false,
                    transparent: Some(TransparentField {
                        field: 0,
                        ty: vec_ty,
                    }),
                    peeled: vec_ty,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: vec![field],
                },
            })
        }

        fn std_hash_map(
            &mut self,
            key_ty: TypeId,
            val_ty: TypeId,
            u64_ty: TypeId,
            usize_ty: TypeId,
            unit_ty: TypeId,
            slot_repr: EnumRepr,
        ) -> TypeId {
            let slot_id = self.id();
            let slot_def = self.def();
            let s_empty = self.interns.intern_str("Empty");
            let s_tomb = self.interns.intern_str("Tombstone");
            let s_occ = self.interns.intern_str("Occupied");
            let s_hash = self.interns.intern_str("hash");
            let s_key = self.interns.intern_str("key");
            let s_val = self.interns.intern_str("value");
            self.insert(TypeDescriptor {
                id: slot_id,
                name: None,
                kind: TypeKind::Adt {
                    def: slot_def,
                    args: vec![key_ty, val_ty],
                    form: NominalForm::Enum,
                    variants: vec![
                        VariantDescriptor {
                            name: s_empty,
                            fields: Vec::new(),
                            logical_discr: 0,
                            physical_tag: None,
                            result_ok: false,
                            saved_local_keys: Vec::new(),
                        },
                        VariantDescriptor {
                            name: s_tomb,
                            fields: Vec::new(),
                            logical_discr: 1,
                            physical_tag: None,
                            result_ok: false,
                            saved_local_keys: Vec::new(),
                        },
                        VariantDescriptor {
                            name: s_occ,
                            fields: vec![
                                FieldDescriptor {
                                    key: FieldKey::Name(s_hash),
                                    ty: u64_ty,
                                    offset: 0,
                                },
                                FieldDescriptor {
                                    key: FieldKey::Name(s_key),
                                    ty: key_ty,
                                    offset: 8,
                                },
                                FieldDescriptor {
                                    key: FieldKey::Name(s_val),
                                    ty: val_ty,
                                    offset: 16,
                                },
                            ],
                            logical_discr: 2,
                            physical_tag: None,
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
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: None,
                    is_indirect: true,
                    needs_clone: true,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: slot_id,
                    unsized_tail: None,
                    enum_repr: Some(slot_repr),
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: Vec::new(),
                },
            });
            let slots_vec = self.vec(slot_id, usize_ty, unit_ty);
            let id = self.id();
            let def = self.def();
            let s_hm = self.interns.intern_str("HashMap");
            let s_slots = self.interns.intern_str("slots");
            let s_len = self.interns.intern_str("len");
            let s_tombs = self.interns.intern_str("tombstones");
            let s_hb = self.interns.intern_str("hash_builder");
            let fields = vec![
                FieldDescriptor {
                    key: FieldKey::Name(s_slots),
                    ty: slots_vec,
                    offset: 0,
                },
                FieldDescriptor {
                    key: FieldKey::Name(s_len),
                    ty: usize_ty,
                    offset: 12,
                },
                FieldDescriptor {
                    key: FieldKey::Name(s_tombs),
                    ty: usize_ty,
                    offset: 16,
                },
                FieldDescriptor {
                    key: FieldKey::Name(s_hb),
                    ty: unit_ty,
                    offset: 20,
                },
            ];
            self.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Adt {
                    def,
                    args: vec![key_ty, val_ty, unit_ty],
                    form: NominalForm::Struct,
                    variants: vec![VariantDescriptor {
                        name: s_hm,
                        fields: fields.clone(),
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    }],
                    niche: None,
                },
                layout: LayoutFacts {
                    size: Some(24),
                    align: Some(4),
                    sized: true,
                    needs_drop: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: None,
                    is_indirect: true,
                    needs_clone: true,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: id,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields,
                },
            })
        }

        fn mires_js_hash_map(&mut self, key_ty: TypeId, val_ty: TypeId, u32_ty: TypeId) -> TypeId {
            let raw_map = self.host_value(u32_ty);
            let id = self.id();
            let def = self.def();
            let s_hm = self.interns.intern_str("HashMap");
            let s_raw = self.interns.intern_str("raw");
            let field = FieldDescriptor {
                key: FieldKey::Name(s_raw),
                ty: raw_map,
                offset: 0,
            };
            self.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Adt {
                    def,
                    args: vec![key_ty, val_ty],
                    form: NominalForm::Struct,
                    variants: vec![VariantDescriptor {
                        name: s_hm,
                        fields: vec![field.clone()],
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    }],
                    niche: None,
                },
                layout: LayoutFacts {
                    size: Some(4),
                    align: Some(4),
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta {
                    abi: NominalAbi::Transparent,
                    boundary: PinnedBoundary::Host,
                    ..TypeMeta::default()
                },
                js: JsFacts {
                    int: None,
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: Some(TransparentField {
                        field: 0,
                        ty: raw_map,
                    }),
                    peeled: u32_ty,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: vec![field],
                },
            })
        }
    }

    fn dummy_func(args: Vec<TypeId>, result: TypeId) -> Func {
        Func {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash: 1,
                },
                instantiation: 1,
            },
            sig: Signature {
                arguments: args.clone(),
                result,
                abi: AbiSignature {
                    parameters: args.len() as u16,
                    ..AbiSignature::default()
                },
            },
            params: Vec::new(),
            values: Vec::new(),
            cells: Vec::new(),
            blocks: Vec::new(),
        }
    }

    fn shim_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("runtime")
            .join("shim.js")
    }

    fn run_node_script(script: &str) {
        let full = format!(
            "const fs = require('fs');\n\
             eval(fs.readFileSync({}, 'utf8'));\n\
             const assert = require('assert');\n\
             {script}\n",
            serde_json::to_string(&shim_path().display().to_string()).expect("path json"),
        );
        let out = Command::new("node")
            .arg("-e")
            .arg(&full)
            .output()
            .expect("node executable must be available");
        assert!(
            out.status.success(),
            "node failed:\n--- script ---\n{script}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }

    #[test]
    fn identity_types_emit_zero_wrapper_overhead() {
        let mut tt = TestTypes::new();
        let unit = tt.unit();
        let b = tt.bool_ty();
        let i32_ty = tt.int(true, 32);
        let u32_ty = tt.int(false, 32);
        let js_val = tt.host_value(u32_ty);
        let str_ref = tt.str_ref();
        let point = tt.plain_struct("Point", &[("x", i32_ty), ("y", i32_ty)]);
        let heap = HeapPlan::default();
        let cx = BoundaryCx {
            table: &tt.interns.types,
            interns: &tt.interns,
            heap: &heap,
            bstr: false,
        };
        for ty in [unit, b, i32_ty, js_val, str_ref, point] {
            assert!(!needs_js_to_rust(&cx, ty));
            assert!(!needs_rust_to_js(&cx, ty));
        }

        let func = dummy_func(vec![i32_ty, js_val, str_ref, point], i32_ty);
        let orig_body = vec![jsast::ret(jsast::id("a"))];
        let mut body = orig_body.clone();
        let params = vec![
            "a".to_owned(),
            "b".to_owned(),
            "c".to_owned(),
            "d".to_owned(),
        ];
        let reassigned = wrap_js_export(&cx, &func, &params, &mut body);
        assert!(reassigned.is_empty());
        assert_eq!(body, orig_body);
    }

    #[test]
    fn option_tagged_and_niche_roundtrip_in_node() {
        let mut tt = TestTypes::new();
        let i32_ty = tt.int(true, 32);
        let opt_tagged = tt.option(i32_ty, EnumRepr::Tag { fieldless: false });
        let opt_niche = tt.option(
            i32_ty,
            EnumRepr::NullNiche {
                none_variant: 0,
                some_variant: 1,
            },
        );
        let heap = HeapPlan::default();
        let cx = BoundaryCx {
            table: &tt.interns.types,
            interns: &tt.interns,
            heap: &heap,
            bstr: false,
        };

        // Tagged Option<i32>: JS undefined/null <-> { TAG: "None" }, 42 <-> { TAG: "Some", _0: 42 }
        let f_tagged = dummy_func(vec![opt_tagged], opt_tagged);
        let mut body_tagged = vec![
            jsast::if_(
                jsast::binary(
                    jsast::BinOp::StrictEq,
                    jsast::member(jsast::id("x"), "TAG"),
                    jsast::string("Some"),
                ),
                vec![jsast::ret(jsast::object_of(vec![
                    ("TAG", jsast::string("Some")),
                    (
                        "_0",
                        jsast::binary(
                            jsast::BinOp::Add,
                            jsast::member(jsast::id("x"), "_0"),
                            jsast::num(1),
                        ),
                    ),
                ]))],
            ),
            jsast::ret(jsast::object_of(vec![("TAG", jsast::string("None"))])),
        ];
        wrap_js_export(&cx, &f_tagged, &["x".to_owned()], &mut body_tagged);
        let decl_tagged = jsast::function("inc_tagged", vec!["x".to_owned()], body_tagged);

        // NullNiche Option<i32>: JS undefined/null <-> null, 42 <-> 42
        let f_niche = dummy_func(vec![opt_niche], opt_niche);
        let mut body_niche = vec![
            jsast::if_(
                jsast::binary(jsast::BinOp::StrictEq, jsast::id("x"), jsast::null()),
                vec![jsast::ret(jsast::null())],
            ),
            jsast::ret(jsast::binary(
                jsast::BinOp::Add,
                jsast::id("x"),
                jsast::num(10),
            )),
        ];
        wrap_js_export(&cx, &f_niche, &["x".to_owned()], &mut body_niche);
        let decl_niche = jsast::function("inc_niche", vec!["x".to_owned()], body_niche);

        let js = format!(
            "{}\n{}\n\
             assert.strictEqual(inc_tagged(undefined), undefined);\n\
             assert.strictEqual(inc_tagged(null), undefined);\n\
             assert.strictEqual(inc_tagged(41), 42);\n\
             assert.strictEqual(inc_tagged(0), 1);\n\
             assert.strictEqual(inc_niche(undefined), undefined);\n\
             assert.strictEqual(inc_niche(null), undefined);\n\
             assert.strictEqual(inc_niche(5), 15);\n",
            jsast::stmt_to_string(&decl_tagged),
            jsast::stmt_to_string(&decl_niche),
        );
        run_node_script(&js);
    }

    #[test]
    fn result_ok_and_err_throw_in_node() {
        let mut tt = TestTypes::new();
        let u8_ty = tt.int(false, 8);
        let usize_ty = tt.int(false, 32);
        let unit = tt.unit();
        let bool_ty = tt.bool_ty();
        let str_ty = tt.string(u8_ty, usize_ty, unit);
        let res_ty = tt.result(str_ty, str_ty);

        let heap = HeapPlan {
            native_string_enabled: true,
            native_vec_enabled: true,
            typed_heap_enabled: true,
            ..HeapPlan::default()
        };
        let cx = BoundaryCx {
            table: &tt.interns.types,
            interns: &tt.interns,
            heap: &heap,
            bstr: true,
        };

        let func = dummy_func(vec![bool_ty, str_ty], res_ty);
        let mut body = vec![
            jsast::if_(
                jsast::id("ok"),
                vec![jsast::ret(jsast::object_of(vec![
                    ("TAG", jsast::string("Ok")),
                    ("_0", jsast::id("msg")),
                ]))],
            ),
            jsast::ret(jsast::object_of(vec![
                ("TAG", jsast::string("Err")),
                ("_0", jsast::id("msg")),
            ])),
        ];
        wrap_js_export(&cx, &func, &["ok".to_owned(), "msg".to_owned()], &mut body);
        let decl = jsast::function(
            "check_result",
            vec!["ok".to_owned(), "msg".to_owned()],
            body,
        );

        let js = format!(
            "{}\n\
             assert.strictEqual(check_result(true, 'héllo 🚀'), 'héllo 🚀');\n\
             assert.throws(() => check_result(false, 'boom 💥'), (err) => {{\n\
               assert.ok(err instanceof Error);\n\
               assert.strictEqual(err.message, 'boom 💥');\n\
               return true;\n\
             }});\n",
            jsast::stmt_to_string(&decl),
        );
        run_node_script(&js);
    }

    #[test]
    fn string_native_and_rawvec_and_bstr_roundtrip_in_node() {
        let mut tt = TestTypes::new();
        let u8_ty = tt.int(false, 8);
        let usize_ty = tt.int(false, 32);
        let unit = tt.unit();
        let str_ty = tt.string(u8_ty, usize_ty, unit);

        let configs = [
            (
                "echo_native",
                HeapPlan {
                    typed_heap_enabled: true,
                    native_string_enabled: true,
                    native_vec_enabled: true,
                    ..HeapPlan::default()
                },
                true,
            ),
            (
                "echo_nvec_utf16",
                HeapPlan {
                    typed_heap_enabled: true,
                    native_string_enabled: false,
                    native_vec_enabled: true,
                    ..HeapPlan::default()
                },
                false,
            ),
            (
                "echo_rawvec_utf16",
                HeapPlan {
                    typed_heap_enabled: false,
                    native_string_enabled: false,
                    native_vec_enabled: false,
                    ..HeapPlan::default()
                },
                false,
            ),
            (
                "echo_rawvec_bstr",
                HeapPlan {
                    typed_heap_enabled: false,
                    native_string_enabled: false,
                    native_vec_enabled: false,
                    ..HeapPlan::default()
                },
                true,
            ),
        ];

        let mut decls = String::new();
        let mut checks = String::new();
        for (name, heap, bstr) in configs {
            let cx = BoundaryCx {
                table: &tt.interns.types,
                interns: &tt.interns,
                heap: &heap,
                bstr,
            };
            let func = dummy_func(vec![str_ty], str_ty);
            let mut body = vec![jsast::ret(jsast::id("s"))];
            wrap_js_export(&cx, &func, &["s".to_owned()], &mut body);
            let decl = jsast::function(name, vec!["s".to_owned()], body);
            decls.push_str(&jsast::stmt_to_string(&decl));
            decls.push('\n');
            checks.push_str(&format!(
                "for (const t of ['', 'ascii', 'café', 'こんにちは🌍']) {{\n\
                   assert.strictEqual({name}(t), t, '{name}: ' + t);\n\
                 }}\n"
            ));
        }

        // Also test RawVec dealloc on an incoming String to prove `_register` keeps `_blocks` valid.
        let raw_heap = HeapPlan::default();
        let cx_raw = BoundaryCx {
            table: &tt.interns.types,
            interns: &tt.interns,
            heap: &raw_heap,
            bstr: false,
        };
        let f_drop = dummy_func(vec![str_ty], unit);
        let mut b_drop = vec![
            jsast::expr_stmt(jsast::rt_call(
                "dealloc",
                vec![
                    jsast::member(
                        jsast::member(
                            jsast::member(jsast::member(jsast::id("s"), "vec"), "buf"),
                            "inner",
                        ),
                        "ptr",
                    ),
                    jsast::member(jsast::member(jsast::id("s"), "vec"), "len"),
                    jsast::num(1),
                ],
            )),
            jsast::ret_void(),
        ];
        wrap_js_export(&cx_raw, &f_drop, &["s".to_owned()], &mut b_drop);
        decls.push_str(&jsast::stmt_to_string(&jsast::function(
            "drop_raw_string",
            vec!["s".to_owned()],
            b_drop,
        )));
        checks.push_str("drop_raw_string('hello world');\n");

        run_node_script(&format!("{decls}\n{checks}"));
    }

    #[test]
    fn vec_i32_and_vec_string_native_and_rawvec_in_node() {
        let mut tt = TestTypes::new();
        let u8_ty = tt.int(false, 8);
        let i32_ty = tt.int(true, 32);
        let usize_ty = tt.int(false, 32);
        let unit = tt.unit();
        let str_ty = tt.string(u8_ty, usize_ty, unit);
        let vec_i32 = tt.vec(i32_ty, usize_ty, unit);
        let vec_str = tt.vec(str_ty, usize_ty, unit);

        let native_heap = HeapPlan {
            typed_heap_enabled: true,
            native_string_enabled: true,
            native_vec_enabled: true,
            ..HeapPlan::default()
        };
        let raw_heap = HeapPlan::default();

        let make_echo = |name: &str, ty: TypeId, heap: &HeapPlan, bstr: bool| -> Stmt {
            let cx = BoundaryCx {
                table: &tt.interns.types,
                interns: &tt.interns,
                heap,
                bstr,
            };
            let func = dummy_func(vec![ty], ty);
            let mut body = vec![jsast::ret(jsast::id("v"))];
            wrap_js_export(&cx, &func, &["v".to_owned()], &mut body);
            jsast::function(name, vec!["v".to_owned()], body)
        };

        let d1 = make_echo("vec_i32_native", vec_i32, &native_heap, true);
        let d2 = make_echo("vec_i32_raw", vec_i32, &raw_heap, false);
        let d3 = make_echo("vec_str_native", vec_str, &native_heap, true);
        let d4 = make_echo("vec_str_raw", vec_str, &raw_heap, false);

        // Also test `&[i32]` slice param conversion.
        let slice_i32 = tt.slice_ref(i32_ty);
        let cx_slice = BoundaryCx {
            table: &tt.interns.types,
            interns: &tt.interns,
            heap: &native_heap,
            bstr: false,
        };
        let f_slice = dummy_func(vec![slice_i32], i32_ty);
        let mut b_slice = vec![jsast::ret(jsast::binary(
            jsast::BinOp::Add,
            jsast::member(jsast::id("xs"), "$l"),
            jsast::index(
                jsast::member(jsast::id("xs"), "$b"),
                jsast::member(jsast::id("xs"), "$o"),
            ),
        ))];
        let reassigned = wrap_js_export(&cx_slice, &f_slice, &["xs".to_owned()], &mut b_slice);
        assert!(reassigned.contains("xs"));
        let d5 = jsast::function("slice_check", vec!["xs".to_owned()], b_slice);

        let js = format!(
            "{}\n{}\n{}\n{}\n{}\n\
             assert.deepStrictEqual(vec_i32_native([]), []);\n\
             assert.deepStrictEqual(vec_i32_native([1, -2, 3]), [1, -2, 3]);\n\
             assert.ok(Array.isArray(vec_i32_native([1, 2])));\n\
             assert.deepStrictEqual(vec_i32_raw([]), []);\n\
             assert.deepStrictEqual(vec_i32_raw([10, 20, 30]), [10, 20, 30]);\n\
             assert.deepStrictEqual(vec_str_native(['a', 'héllo', '🦀']), ['a', 'héllo', '🦀']);\n\
             assert.deepStrictEqual(vec_str_raw(['x', 'über', '']), ['x', 'über', '']);\n\
             assert.strictEqual(slice_check([100, 200, 300]), 103);\n",
            jsast::stmt_to_string(&d1),
            jsast::stmt_to_string(&d2),
            jsast::stmt_to_string(&d3),
            jsast::stmt_to_string(&d4),
            jsast::stmt_to_string(&d5),
        );
        run_node_script(&js);
    }

    #[test]
    fn parse_options_struct_with_optional_bools_roundtrip_in_node() {
        let mut tt = TestTypes::new();
        let bool_ty = tt.bool_ty();
        let i32_ty = tt.int(true, 32);
        let opt_bool = tt.option(bool_ty, EnumRepr::Tag { fieldless: false });
        let opts_struct =
            tt.plain_struct("ParseOptions", &[("gfm", opt_bool), ("breaks", opt_bool)]);

        let heap = HeapPlan::default();
        let cx = BoundaryCx {
            table: &tt.interns.types,
            interns: &tt.interns,
            heap: &heap,
            bstr: false,
        };
        assert!(needs_js_to_rust(&cx, opts_struct));
        assert!(needs_rust_to_js(&cx, opts_struct));

        let func = dummy_func(vec![opts_struct], opts_struct);
        let mut body = vec![jsast::ret(jsast::id("opts"))];
        wrap_js_export(&cx, &func, &["opts".to_owned()], &mut body);
        let decl = jsast::function("echo_opts", vec!["opts".to_owned()], body);

        // Also inspect the marshaled Rust shape directly via a helper that reads `.gfm.TAG`.
        let f_inspect = dummy_func(vec![opts_struct], i32_ty);
        let mut b_inspect = vec![jsast::ret(jsast::binary(
            jsast::BinOp::Add,
            jsast::cond(
                jsast::binary(
                    jsast::BinOp::And,
                    jsast::binary(
                        jsast::BinOp::StrictEq,
                        jsast::member(jsast::member(jsast::id("opts"), "gfm"), "TAG"),
                        jsast::string("Some"),
                    ),
                    jsast::member(jsast::member(jsast::id("opts"), "gfm"), "_0"),
                ),
                jsast::num(1),
                jsast::num(0),
            ),
            jsast::cond(
                jsast::binary(
                    jsast::BinOp::StrictEq,
                    jsast::member(jsast::member(jsast::id("opts"), "breaks"), "TAG"),
                    jsast::string("None"),
                ),
                jsast::num(10),
                jsast::num(0),
            ),
        ))];
        wrap_js_export(&cx, &f_inspect, &["opts".to_owned()], &mut b_inspect);
        let d_inspect = jsast::function("inspect_opts", vec!["opts".to_owned()], b_inspect);

        let js = format!(
            "{}\n{}\n\
             assert.deepStrictEqual(echo_opts({{ gfm: true }}), {{ gfm: true, breaks: undefined }});\n\
             assert.deepStrictEqual(echo_opts({{ gfm: false, breaks: true }}), {{ gfm: false, breaks: true }});\n\
             assert.deepStrictEqual(echo_opts(undefined), {{ gfm: undefined, breaks: undefined }});\n\
             assert.strictEqual(inspect_opts({{ gfm: true }}), 11);\n\
             assert.strictEqual(inspect_opts({{ gfm: false, breaks: false }}), 0);\n",
            jsast::stmt_to_string(&decl),
            jsast::stmt_to_string(&d_inspect),
        );
        run_node_script(&js);
    }

    #[test]
    fn structured_result_error_and_payload_enum_roundtrip_in_node() {
        let mut tt = TestTypes::new();
        let u8_ty = tt.int(false, 8);
        let i32_ty = tt.int(true, 32);
        let usize_ty = tt.int(false, 32);
        let bool_ty = tt.bool_ty();
        let unit = tt.unit();
        let str_ty = tt.string(u8_ty, usize_ty, unit);
        let opt_bool = tt.option(bool_ty, EnumRepr::Tag { fieldless: false });

        // Structured error `{ message: String, line: i32 }`, plus `Result<(), i32>`.
        let err_struct = tt.plain_struct("ParseError", &[("message", str_ty), ("line", i32_ty)]);
        let res_struct_err = tt.result(unit, err_struct);
        let res_int_err = tt.result(unit, i32_ty);

        // Tagged payload enum `Action { Empty, Rename(String), SetFlag { flag: Option<bool> } }`.
        let action_id = tt.id();
        let action_def = tt.def();
        let s_empty = tt.interns.intern_str("Empty");
        let s_rename = tt.interns.intern_str("Rename");
        let s_set_flag = tt.interns.intern_str("SetFlag");
        let s_0 = tt.interns.intern_str("_0");
        let s_flag = tt.interns.intern_str("flag");
        tt.insert(TypeDescriptor {
            id: action_id,
            name: None,
            kind: TypeKind::Adt {
                def: action_def,
                args: Vec::new(),
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: s_empty,
                        fields: Vec::new(),
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: s_rename,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(s_0),
                            ty: str_ty,
                            offset: 0,
                        }],
                        logical_discr: 1,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: s_set_flag,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(s_flag),
                            ty: opt_bool,
                            offset: 0,
                        }],
                        logical_discr: 2,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
                niche: None,
            },
            layout: LayoutFacts {
                size: Some(16),
                align: Some(4),
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: true,
                needs_clone: true,
                crosses_transparent_union: false,
                transparent: None,
                peeled: action_id,
                unsized_tail: None,
                enum_repr: Some(EnumRepr::Tag { fieldless: false }),
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });

        // SentinelNiche Option<i32>
        tt.interns.types.set_sentinel_none(Some("$sn".to_owned()));
        let opt_sentinel = tt.option(
            i32_ty,
            EnumRepr::SentinelNiche {
                none_variant: 0,
                some_variant: 1,
                some_field: 0,
            },
        );

        let heap = HeapPlan {
            typed_heap_enabled: true,
            native_string_enabled: true,
            native_vec_enabled: true,
            ..HeapPlan::default()
        };
        let cx = BoundaryCx {
            table: &tt.interns.types,
            interns: &tt.interns,
            heap: &heap,
            bstr: true,
        };

        let f_struct_err = dummy_func(vec![bool_ty, err_struct], res_struct_err);
        let mut b_struct_err = vec![
            jsast::if_(
                jsast::id("ok"),
                vec![jsast::ret(jsast::object_of(vec![
                    ("TAG", jsast::string("Ok")),
                    ("_0", jsast::undefined()),
                ]))],
            ),
            jsast::ret(jsast::object_of(vec![
                ("TAG", jsast::string("Err")),
                ("_0", jsast::id("err")),
            ])),
        ];
        wrap_js_export(
            &cx,
            &f_struct_err,
            &["ok".to_owned(), "err".to_owned()],
            &mut b_struct_err,
        );
        let d_struct_err = jsast::function(
            "check_struct_err",
            vec!["ok".to_owned(), "err".to_owned()],
            b_struct_err,
        );

        let f_int_err = dummy_func(vec![bool_ty, i32_ty], res_int_err);
        let mut b_int_err = vec![
            jsast::if_(
                jsast::id("ok"),
                vec![jsast::ret(jsast::object_of(vec![
                    ("TAG", jsast::string("Ok")),
                    ("_0", jsast::undefined()),
                ]))],
            ),
            jsast::ret(jsast::object_of(vec![
                ("TAG", jsast::string("Err")),
                ("_0", jsast::id("code")),
            ])),
        ];
        wrap_js_export(
            &cx,
            &f_int_err,
            &["ok".to_owned(), "code".to_owned()],
            &mut b_int_err,
        );
        let d_int_err = jsast::function(
            "check_int_err",
            vec!["ok".to_owned(), "code".to_owned()],
            b_int_err,
        );

        let f_action = dummy_func(vec![action_id], action_id);
        let mut b_action = vec![jsast::ret(jsast::id("a"))];
        wrap_js_export(&cx, &f_action, &["a".to_owned()], &mut b_action);
        let d_action = jsast::function("echo_action", vec!["a".to_owned()], b_action);

        let f_sentinel = dummy_func(vec![opt_sentinel], opt_sentinel);
        let mut b_sentinel = vec![
            jsast::if_(
                jsast::binary(jsast::BinOp::StrictEq, jsast::id("x"), jsast::id("$sn")),
                vec![jsast::ret(jsast::id("$sn"))],
            ),
            jsast::ret(jsast::binary(
                jsast::BinOp::Add,
                jsast::id("x"),
                jsast::num(1),
            )),
        ];
        wrap_js_export(&cx, &f_sentinel, &["x".to_owned()], &mut b_sentinel);
        let d_sentinel = jsast::function("inc_sentinel", vec!["x".to_owned()], b_sentinel);

        let js = format!(
            "const $sn = {{}};\n{}\n{}\n{}\n{}\n\
             assert.strictEqual(check_struct_err(true, {{ message: 'unused', line: 0 }}), undefined);\n\
             assert.throws(() => check_struct_err(false, {{ message: 'bad token', line: 7 }}), (e) => {{\n\
               assert.ok(e instanceof Error);\n\
               assert.strictEqual(e.message, 'bad token');\n\
               assert.deepStrictEqual(e.cause, {{ message: 'bad token', line: 7 }});\n\
               return true;\n\
             }});\n\
             assert.strictEqual(check_int_err(true, 0), undefined);\n\
             assert.throws(() => check_int_err(false, 404), (e) => {{\n\
               assert.ok(e instanceof Error);\n\
               assert.strictEqual(e.message, '404');\n\
               assert.strictEqual(e.cause, 404);\n\
               return true;\n\
             }});\n\
             assert.deepStrictEqual(echo_action({{ TAG: 'Empty' }}), {{ TAG: 'Empty' }});\n\
             assert.deepStrictEqual(echo_action({{ TAG: 'Rename', _0: 'café' }}), {{ TAG: 'Rename', _0: 'café' }});\n\
             assert.deepStrictEqual(echo_action({{ TAG: 'SetFlag', flag: true }}), {{ TAG: 'SetFlag', flag: true }});\n\
             assert.deepStrictEqual(echo_action({{ TAG: 'SetFlag' }}), {{ TAG: 'SetFlag', flag: undefined }});\n\
             assert.strictEqual(inc_sentinel(undefined), undefined);\n\
             assert.strictEqual(inc_sentinel(null), undefined);\n\
             assert.strictEqual(inc_sentinel(9), 10);\n",
            jsast::stmt_to_string(&d_struct_err),
            jsast::stmt_to_string(&d_int_err),
            jsast::stmt_to_string(&d_action),
            jsast::stmt_to_string(&d_sentinel),
        );
        run_node_script(&js);
    }

    #[test]
    fn export_out_body_handles_unit_plain_and_result_in_node() {
        let mut tt = TestTypes::new();
        let u8_ty = tt.int(false, 8);
        let u32_ty = tt.int(false, 32);
        let i32_ty = tt.int(true, 32);
        let usize_ty = tt.int(false, 32);
        let unit = tt.unit();
        let str_ty = tt.string(u8_ty, usize_ty, unit);
        let res_str = tt.result(str_ty, str_ty);
        let promise_str = tt.typed_promise(str_ty, u32_ty);
        let raw_ptr = tt.raw_mut_ptr(i32_ty);

        let heap = HeapPlan {
            typed_heap_enabled: true,
            native_string_enabled: true,
            native_vec_enabled: true,
            ..HeapPlan::default()
        };
        let cx = BoundaryCx {
            table: &tt.interns.types,
            interns: &tt.interns,
            heap: &heap,
            bstr: true,
        };

        // TypedPromise and raw *mut T are identity at the JS export boundary.
        assert!(!needs_rust_to_js(&cx, promise_str));
        assert!(!needs_js_to_rust(&cx, raw_ptr));
        assert!(!needs_rust_to_js(&cx, raw_ptr));

        let unit_fn = jsast::function(
            "out_unit",
            vec!["$p0".to_owned()],
            lower_export_out_body(&cx, "$p0", unit),
        );
        let str_fn = jsast::function(
            "out_str",
            vec!["$p0".to_owned()],
            lower_export_out_body(&cx, "$p0", str_ty),
        );
        let res_fn = jsast::function(
            "out_res",
            vec!["$p0".to_owned()],
            lower_export_out_body(&cx, "$p0", res_str),
        );
        let id_fn = jsast::function(
            "out_i32",
            vec!["$p0".to_owned()],
            lower_export_out_body(&cx, "$p0", i32_ty),
        );

        let js = format!(
            "{}\n{}\n{}\n{}\n\
             const mkStr = (s) => {{ const b = __rt.bstr_in(s); return {{ s: b, cap: b.length }}; }};\n\
             assert.strictEqual(out_unit(123), undefined);\n\
             assert.strictEqual(out_i32(42), 42);\n\
             assert.strictEqual(out_str(mkStr('async 🦀')), 'async 🦀');\n\
             assert.strictEqual(out_res({{ TAG: 'Ok', _0: mkStr('done') }}), 'done');\n\
             assert.throws(() => out_res({{ TAG: 'Err', _0: mkStr('async fail') }}), (e) => {{\n\
               assert.ok(e instanceof Error);\n\
               assert.strictEqual(e.message, 'async fail');\n\
               return true;\n\
             }});\n",
            jsast::stmt_to_string(&unit_fn),
            jsast::stmt_to_string(&str_fn),
            jsast::stmt_to_string(&res_fn),
            jsast::stmt_to_string(&id_fn),
        );
        run_node_script(&js);
    }

    #[test]
    fn map_entries_and_hashmap_marshaling_in_node() {
        let mut tt = TestTypes::new();
        let u8_ty = tt.int(false, 8);
        let i32_ty = tt.int(true, 32);
        let u32_ty = tt.int(false, 32);
        let u64_ty = tt.int(false, 64);
        let usize_ty = tt.int(false, 32);
        let unit = tt.unit();
        let str_ty = tt.string(u8_ty, usize_ty, unit);

        let entries_str_i32 = tt.map_entries("JsMapEntries", str_ty, i32_ty, usize_ty, unit);
        let entries_i32_str = tt.map_entries("JsBTreeMapEntries", i32_ty, str_ty, usize_ty, unit);
        let hm_str_u32_tag = tt.std_hash_map(
            str_ty,
            u32_ty,
            u64_ty,
            usize_ty,
            unit,
            EnumRepr::Tag { fieldless: false },
        );
        let hm_i32_str_int = tt.std_hash_map(
            i32_ty,
            str_ty,
            u64_ty,
            usize_ty,
            unit,
            EnumRepr::IntTag { fieldless: false },
        );
        let report_struct =
            tt.plain_struct("Report", &[("counts", hm_str_u32_tag), ("total", u32_ty)]);
        let host_hm_str = tt.mires_js_hash_map(str_ty, str_ty, u32_ty);
        let host_hm_i32 = tt.mires_js_hash_map(i32_ty, i32_ty, u32_ty);
        let _opt_str = tt.option(str_ty, EnumRepr::Tag { fieldless: false });

        let native_heap = HeapPlan {
            typed_heap_enabled: true,
            native_string_enabled: true,
            native_vec_enabled: true,
            ..HeapPlan::default()
        };
        let raw_heap = HeapPlan::default();

        let cx_native = BoundaryCx {
            table: &tt.interns.types,
            interns: &tt.interns,
            heap: &native_heap,
            bstr: true,
        };
        let cx_raw = BoundaryCx {
            table: &tt.interns.types,
            interns: &tt.interns,
            heap: &raw_heap,
            bstr: false,
        };

        assert!(needs_js_to_rust(&cx_native, entries_str_i32));
        assert!(needs_js_to_rust(&cx_native, entries_i32_str));
        assert!(needs_rust_to_js(&cx_native, hm_str_u32_tag));
        assert!(!needs_js_to_rust(&cx_native, hm_str_u32_tag));
        assert!(needs_rust_to_js(&cx_native, report_struct));
        assert!(needs_rust_to_js(&cx_native, host_hm_str));
        assert!(needs_rust_to_js(&cx_native, host_hm_i32));

        // 1. JsMapEntries<String, i32> input (native Vec + native String) -> returns raw Vec for inspection
        let f_in_str = dummy_func(vec![entries_str_i32], unit);
        let mut b_in_str = vec![jsast::ret(jsast::id("m"))];
        wrap_js_export(&cx_native, &f_in_str, &["m".to_owned()], &mut b_in_str);
        let d_in_str = jsast::function("in_str_map_native", vec!["m".to_owned()], b_in_str);

        // 2. JsMapEntries<String, i32> input (RawVec mode)
        let mut b_in_raw = vec![jsast::ret(jsast::id("m"))];
        wrap_js_export(&cx_raw, &f_in_str, &["m".to_owned()], &mut b_in_raw);
        let d_in_raw = jsast::function("in_str_map_raw", vec!["m".to_owned()], b_in_raw);

        // 3. JsBTreeMapEntries<i32, String> input (native Vec)
        let f_in_i32 = dummy_func(vec![entries_i32_str], unit);
        let mut b_in_i32 = vec![jsast::ret(jsast::id("m"))];
        wrap_js_export(&cx_native, &f_in_i32, &["m".to_owned()], &mut b_in_i32);
        let d_in_i32 = jsast::function("in_i32_map", vec!["m".to_owned()], b_in_i32);

        // 4. mires-std HashMap<String, u32> nested inside Report struct (native Vec + Tag)
        let f_rep = dummy_func(Vec::new(), report_struct);
        let mut b_rep = vec![jsast::ret(jsast::id("r"))];
        wrap_js_export(&cx_native, &f_rep, &[], &mut b_rep);
        let d_rep = jsast::function("out_report_native", vec!["r".to_owned()], b_rep);

        // 5. mires-std HashMap<i32, String> (RawVec + IntTag)
        let f_hm_i32 = dummy_func(Vec::new(), hm_i32_str_int);
        let mut b_hm_i32 = vec![jsast::ret(jsast::id("m"))];
        wrap_js_export(&cx_raw, &f_hm_i32, &[], &mut b_hm_i32);
        let d_hm_i32 = jsast::function("out_hm_i32_raw", vec!["m".to_owned()], b_hm_i32);

        // 6. mires_js::collections::HashMap<String, String> & <i32, i32>
        let f_host_str = dummy_func(Vec::new(), host_hm_str);
        let mut b_host_str = vec![jsast::ret(jsast::id("m"))];
        wrap_js_export(&cx_native, &f_host_str, &[], &mut b_host_str);
        let d_host_str = jsast::function("out_host_hm_str", vec!["m".to_owned()], b_host_str);

        let f_host_i32 = dummy_func(Vec::new(), host_hm_i32);
        let mut b_host_i32 = vec![jsast::ret(jsast::id("m"))];
        wrap_js_export(&cx_native, &f_host_i32, &[], &mut b_host_i32);
        let d_host_i32 = jsast::function("out_host_hm_i32", vec!["m".to_owned()], b_host_i32);

        let js = format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n\
             const mkNStr = (s) => {{ const b = __rt.bstr_in(s); return {{ s: b, cap: b.length }}; }};\n\
             const mkRawStr = (s) => {{\n\
               const b = __rt.str_bytes_mut(s);\n\
               const ptr = b.$l === 0 ? 1 : __rt._register(b.$b, b.$l, false);\n\
               return {{ vec: {{ buf: {{ inner: {{ ptr, cap: b.$l, alloc: undefined }}, _marker: undefined }}, len: b.$l }} }};\n\
             }};\n\
             // JsMapEntries<String, i32> from plain object, JS Map, and null\n\
             const vObj = in_str_map_native({{ a: 1, 'café': 2 }});\n\
             assert.strictEqual(vObj.len, 2);\n\
             assert.strictEqual(__rt.bstr_out(vObj.buf[0][0].s), 'a');\n\
             assert.strictEqual(vObj.buf[0][1], 1);\n\
             assert.strictEqual(__rt.bstr_out(vObj.buf[1][0].s), 'café');\n\
             assert.strictEqual(vObj.buf[1][1], 2);\n\
             const vMap = in_str_map_native(new Map([['k', 99]]));\n\
             assert.strictEqual(vMap.len, 1);\n\
             assert.strictEqual(__rt.bstr_out(vMap.buf[0][0].s), 'k');\n\
             assert.strictEqual(vMap.buf[0][1], 99);\n\
             assert.strictEqual(in_str_map_native(undefined).len, 0);\n\
             const vRaw = in_str_map_raw({{ hi: 7 }});\n\
             assert.strictEqual(vRaw.len, 1);\n\
             const rEntry = vRaw.buf.inner.ptr.$b[vRaw.buf.inner.ptr.$o];\n\
             assert.strictEqual(rEntry[1], 7);\n\
             // JsBTreeMapEntries<i32, String> from JS Map and entries array\n\
             const bMap = in_i32_map(new Map([[10, 'ten'], [20, 'twenty']]));\n\
             assert.strictEqual(bMap.len, 2);\n\
             assert.strictEqual(bMap.buf[0][0], 10);\n\
             assert.strictEqual(__rt.bstr_out(bMap.buf[0][1].s), 'ten');\n\
             const bArr = in_i32_map([[3, 'three']]);\n\
             assert.strictEqual(bArr.len, 1);\n\
             assert.strictEqual(bArr.buf[0][0], 3);\n\
             // Report {{ counts: HashMap<String, u32>, total: u32 }} (native Vec + Tag)\n\
             const rep = out_report_native({{\n\
               counts: {{\n\
                 slots: {{ buf: [\n\
                   {{ TAG: 'Empty' }},\n\
                   {{ TAG: 'Occupied', hash: 1n, key: mkNStr('rust'), value: 3 }},\n\
                   {{ TAG: 'Tombstone' }},\n\
                   {{ TAG: 'Occupied', hash: 2n, key: mkNStr('js'), value: 5 }},\n\
                 ], len: 4 }},\n\
                 len: 2, tombstones: 1, hash_builder: undefined\n\
               }},\n\
               total: 8\n\
             }});\n\
             assert.deepStrictEqual(rep, {{ counts: {{ rust: 3, js: 5 }}, total: 8 }});\n\
             // Empty HashMap\n\
             assert.deepStrictEqual(out_report_native({{\n\
               counts: {{ slots: {{ buf: [], len: 0 }}, len: 0, tombstones: 0, hash_builder: undefined }},\n\
               total: 0\n\
             }}), {{ counts: {{}}, total: 0 }});\n\
             // mires-std HashMap<i32, String> (RawVec + IntTag where Empty=0, Tombstone=1, Occupied=2)\n\
             const rawSlots = [\n\
               {{ TAG: 0 }},\n\
               {{ TAG: 2, hash: 1n, key: 7, value: mkRawStr('seven') }},\n\
               {{ TAG: 1 }},\n\
               {{ TAG: 2, hash: 2n, key: 9, value: mkRawStr('nine') }},\n\
             ];\n\
             const outMap = out_hm_i32_raw({{\n\
               slots: {{ buf: {{ inner: {{ ptr: {{ $b: rawSlots, $o: 0 }}, cap: 4, alloc: undefined }}, _marker: undefined }}, len: 4 }},\n\
               len: 2, tombstones: 1, hash_builder: undefined\n\
             }});\n\
             assert.ok(outMap instanceof Map);\n\
             assert.strictEqual(outMap.size, 2);\n\
             assert.strictEqual(outMap.get(7), 'seven');\n\
             assert.strictEqual(outMap.get(9), 'nine');\n\
             const emptyRawMap = out_hm_i32_raw({{\n\
               slots: {{ buf: {{ inner: {{ ptr: 8, cap: 0, alloc: undefined }}, _marker: undefined }}, len: 0 }},\n\
               len: 0, tombstones: 0, hash_builder: undefined\n\
             }});\n\
             assert.ok(emptyRawMap instanceof Map && emptyRawMap.size === 0);\n\
             // mires_js::collections::HashMap<String, String> (with Option<String> Tag in TypeTable)\n\
             const hostStr = new Map([\n\
               [__rt.bstr_in('greeting'), {{ value: {{ TAG: 'Some', _0: mkNStr('héllo') }} }}],\n\
               [__rt.bstr_in('dead'), {{ value: {{ TAG: 'None' }} }}],\n\
             ]);\n\
             assert.deepStrictEqual(out_host_hm_str(hostStr), {{ greeting: 'héllo' }});\n\
             // mires_js::collections::HashMap<i32, i32> (dynamic MapEntry unwrap: Tag + NullNiche)\n\
             const hostI32 = new Map([\n\
               [1, {{ value: {{ TAG: 'Some', _0: 100 }} }}],\n\
               [2, {{ value: 200 }}],\n\
               [3, {{ value: null }}],\n\
             ]);\n\
             const outHostI32 = out_host_hm_i32(hostI32);\n\
             assert.ok(outHostI32 instanceof Map);\n\
             assert.strictEqual(outHostI32.size, 2);\n\
             assert.strictEqual(outHostI32.get(1), 100);\n\
             assert.strictEqual(outHostI32.get(2), 200);\n",
            jsast::stmt_to_string(&d_in_str),
            jsast::stmt_to_string(&d_in_raw),
            jsast::stmt_to_string(&d_in_i32),
            jsast::stmt_to_string(&d_rep),
            jsast::stmt_to_string(&d_hm_i32),
            jsast::stmt_to_string(&d_host_str),
            jsast::stmt_to_string(&d_host_i32),
        );
        run_node_script(&js);
    }

    #[test]
    fn lower_item_short_circuits_js_export_out_and_collects_rt_refs() {
        use rcgjs_object::item::JsName;
        use rcgjs_object::jir::semantic::{
            BasicBlock, BlockId, Body, BodyKind, BodySemantic, Call, CallSemantic, Callee,
            Constant, Local, LocalId, LocalRole, Operand, OperandKind, Place, Terminator, Unwind,
        };

        let mut tt = TestTypes::new();
        let u8_ty = tt.int(false, 8);
        let u32_ty = tt.int(false, 32);
        let usize_ty = tt.int(false, 32);
        let unit = tt.unit();
        let str_ty = tt.string(u8_ty, usize_ty, unit);
        let js_val = tt.host_value(u32_ty);
        let dummy_callee = tt.interns.intern_str("never_emit_dummy_callee");

        // Dummy MIR body calls `never_emit_dummy_callee` (simulating `ManuallyDrop::new` +
        // `JsValue::undefined()`). `BodySemantic::JsExportOut` MUST short-circuit before lowering
        // this MIR so `never_emit_dummy_callee` is absent from both JS and `refs`, while `__rt`
        // (needed for `bstr_out` / `bytes_str`) IS recorded in `refs`.
        let body = Body {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash: 99,
                },
                instantiation: 1,
            },
            kind: BodyKind::Function,
            semantic: BodySemantic::JsExportOut,
            signature: Signature {
                arguments: vec![str_ty],
                result: js_val,
                abi: AbiSignature {
                    parameters: 1,
                    inline_never: true,
                    ..AbiSignature::default()
                },
            },
            locals: vec![
                Local {
                    id: LocalId(0),
                    role: LocalRole::Return,
                    ty: js_val,
                },
                Local {
                    id: LocalId(1),
                    role: LocalRole::Argument,
                    ty: str_ty,
                },
            ],
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    cleanup: false,
                    statements: Vec::new(),
                    terminator: Terminator::Call(Call {
                        function: Operand {
                            ty: unit,
                            kind: OperandKind::Constant(Constant::ZeroSized),
                        },
                        callee: Callee::Direct {
                            item: ItemId {
                                definition: DefinitionId {
                                    crate_id: 1,
                                    local_hash: 100,
                                },
                                instantiation: 1,
                            },
                            link_name: dummy_callee,
                        },
                        semantic: CallSemantic::Ordinary,
                        arguments: Vec::new(),
                        destination: Place {
                            local: LocalId(0),
                            projection: Vec::new(),
                            ty: js_val,
                        },
                        target: Some(BlockId(1)),
                        unwind: Unwind::Continue,
                        track_caller: None,
                        host: None,
                    }),
                },
                BasicBlock {
                    id: BlockId(1),
                    cleanup: false,
                    statements: Vec::new(),
                    terminator: Terminator::Return,
                },
            ],
        };

        let heap = HeapPlan {
            typed_heap_enabled: true,
            native_string_enabled: true,
            native_vec_enabled: true,
            ..HeapPlan::default()
        };
        let opts = crate::opts::Opts::default();
        let name = JsName::new("__mires_export_out$h0123456789abcdef".to_owned());
        let summary = rcgjs_object::jir::summary::ItemSummary {
            key: rcgjs_object::jir::summary::SemanticKey {
                link_name: name.as_str().to_owned(),
                item: Some(body.item),
            },
            body_hash: None,
            semantic_hash: None,
            availability: rcgjs_object::jir::summary::Availability::Available,
            deps: Vec::new(),
            abi: Some(body.signature.abi),
            physical: None,
            kind: rcgjs_object::object::ItemKind::Fn,
            linkage: rcgjs_object::object::Linkage::internal(),
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
            class: rcgjs_object::jir::summary::ItemClass::empty(),
        };
        let input = crate::lower::LowerInput {
            name: &name,
            body: &body,
            origins: None,
            table: &tt.interns.types,
            interns: &tt.interns,
            opts: &opts,
            summary: &summary,
            items: crate::lower::js::host::EMPTY_ITEMS,
            drop_glue: crate::lower::js::host::EMPTY_DROP_GLUE,
            vtables: &crate::lower::js::unsize::EMPTY_VTABLES,
            spills: &[],
            abi: None,
            call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
            bigints: None,
            byte_views: crate::analysis::byte_views::ByteViewPlan::default(),
            heap: Some(&heap),
            nested_clone: crate::lower::NestedClone::default(),
            final_lowering: true,
        };
        let lowered = crate::lower::lower_item(&input).expect("JsExportOut lowers cleanly");
        let printed = jsast::stmt_to_string(&lowered.decl);
        assert!(
            !printed.contains("never_emit_dummy_callee"),
            "dummy MIR body must be bypassed:\n{printed}"
        );
        assert!(
            !lowered.refs.contains("never_emit_dummy_callee"),
            "dummy callee must not appear in refs: {:?}",
            lowered.refs
        );
        assert!(
            lowered.refs.contains("__rt"),
            "marshaled return must record `__rt` in refs: {:?}",
            lowered.refs
        );
        assert!(
            printed.contains("__rt.bstr_out($p0.s)"),
            "expected native String conversion in body:\n{printed}"
        );
    }

    #[test]
    fn separate_js_export_wrapper_skips_identity_and_wraps_marshaled_signatures() {
        let mut tt = TestTypes::new();
        let unit = tt.unit();
        let u8_ty = tt.int(false, 8);
        let usize_ty = tt.int(false, 32);
        let i32_ty = tt.int(true, 32);
        let bool_ty = tt.bool_ty();
        let str_ref = tt.str_ref();
        let string_ty = tt.string(u8_ty, usize_ty, unit);
        let vec_i32 = tt.vec(i32_ty, usize_ty, unit);
        let opt_string = tt.option(string_ty, EnumRepr::Tag { fieldless: false });
        let res_vec_str = tt.result(vec_i32, string_ty);

        let heap_default = HeapPlan::default();
        let heap_native = HeapPlan {
            typed_heap_enabled: true,
            native_string_enabled: true,
            native_vec_enabled: true,
            ..HeapPlan::default()
        };
        let cx_utf16 = BoundaryCx {
            table: &tt.interns.types,
            interns: &tt.interns,
            heap: &heap_default,
            bstr: false,
        };
        let cx_bstr = BoundaryCx {
            table: &tt.interns.types,
            interns: &tt.interns,
            heap: &heap_native,
            bstr: true,
        };

        // Identity-only signature under UTF-16 emits zero wrapper (None).
        let id_sig = Signature {
            arguments: vec![i32_ty, str_ref],
            result: bool_ty,
            abi: AbiSignature {
                parameters: 2,
                ..AbiSignature::default()
            },
        };
        assert!(!needs_js_export_wrapper(&cx_utf16, &id_sig));
        assert!(build_js_export_wrapper(&cx_utf16, "id_fn", "id_fn$rs", &id_sig, None).is_none());

        // Under `js-str=bytes`, the `&str` parameter requires one thin wrapper with `__rt.bstr_in`.
        assert!(needs_js_export_wrapper(&cx_bstr, &id_sig));
        let bstr_id_decl = build_js_export_wrapper(&cx_bstr, "id_fn", "id_fn$rs", &id_sig, None)
            .expect("bstr wrapper emitted");
        let bstr_id_js = jsast::stmt_to_string(&bstr_id_decl);
        assert!(
            bstr_id_js.contains("function id_fn(a0, a1)"),
            "{bstr_id_js}"
        );
        assert!(
            bstr_id_js.contains("id_fn$rs(a0, __rt.bstr_in(a1))"),
            "{bstr_id_js}"
        );

        // `fn render_markdown(&str) -> String` (npm-markdown shape):
        // UTF-16 leaves `&str` untouched and converts return `String` via `__rt.bytes_str`.
        let md_sig = Signature {
            arguments: vec![str_ref],
            result: string_ty,
            abi: AbiSignature {
                parameters: 1,
                ..AbiSignature::default()
            },
        };
        let md_utf16 = jsast::stmt_to_string(
            &build_js_export_wrapper(
                &cx_utf16,
                "render_markdown",
                "render_markdown$rs",
                &md_sig,
                None,
            )
            .expect("utf16 wrapper"),
        );
        assert!(md_utf16.contains("render_markdown$rs(a0)"), "{md_utf16}");
        assert!(md_utf16.contains("__rt.bytes_str"), "{md_utf16}");

        // Under `bstr + native_string`, one wrapper does BOTH `bstr_in` and `bstr_out` with zero UTF-16 helpers.
        let md_bstr = jsast::stmt_to_string(
            &build_js_export_wrapper(
                &cx_bstr,
                "render_markdown",
                "render_markdown$rs",
                &md_sig,
                None,
            )
            .expect("bstr wrapper"),
        );
        assert!(
            md_bstr.contains("__rt.bstr_out(render_markdown$rs(__rt.bstr_in(a0)).s)"),
            "{md_bstr}"
        );
        assert!(!md_bstr.contains("str_bytes"), "{md_bstr}");
        assert!(!md_bstr.contains("bytes_str"), "{md_bstr}");

        // Complex `fn(Option<String>) -> Result<Vec<i32>, String>` wrapper unwraps Ok / throws Err.
        let complex_sig = Signature {
            arguments: vec![opt_string],
            result: res_vec_str,
            abi: AbiSignature {
                parameters: 1,
                ..AbiSignature::default()
            },
        };
        let complex_js = jsast::stmt_to_string(
            &build_js_export_wrapper(
                &cx_bstr,
                "parse_items",
                "parse_items$rs",
                &complex_sig,
                None,
            )
            .expect("complex wrapper"),
        );
        assert!(complex_js.contains("parse_items$rs("), "{complex_js}");
        assert!(complex_js.contains("throw "), "{complex_js}");
    }
}
