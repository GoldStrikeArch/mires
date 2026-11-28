//! `.d.ts` synthesis, tcx side: the TypeScript a Rust signature comes out as.
//!
//! The [`Dts`] value this produces and the writer that turns a module's worth of them into the
//! finished `.d.ts` text live in [`rcgjs_object::dts`]; the link step has no `TyCtxt` to compute a
//! signature with, and this half has nothing to print.
//!
//! A compiled island is an ES module with exported entry points and imports of its own, and nothing
//! about it says what those entries take or answer. This writes that down, beside the `.js`, from
//! the Rust types the entries were compiled from.
//!
//! # Why the representation makes this worth having
//!
//! The types are readable because the value model is nameable. A fieldless enum is its variant's
//! NAME, so it is a union of string literals and TypeScript narrows it; an enum with a payload is
//! an object under [`TAG`], so it is a discriminated union and TypeScript narrows that too. Before
//! the string-tag migration both were integers, and every one of them would have been `number`.
//!
//! # Where it is computed, and where it is written
//!
//! The signature is computed during codegen, where `TyCtxt` is, and travels in the object file
//! beside the item. The link step assembles the file: it is the only place that knows which items
//! survived and which module they ended up in, which is exactly what a `.d.ts` describes.
//!
//! `js-dts=on` turns it on. It is NOT emit affecting: nothing about the JavaScript changes, so it
//! does not have to agree with the sysroot's flags.
//!
//! # What is opaque, and why that is the honest answer
//!
//! A `Sig`, a `Node`, a `JsValue` and an `Owner` are one-word handles whose contents are the
//! runtime's: a `Sig<T>` is really the `[read, write]` pair `createSignal` answered. Spelling that
//! as `number`, which is what the handle's field says, would be a lie a caller could act on. They
//! are declared as distinct opaque types instead, so TypeScript refuses to swap one for another
//! and refuses to do arithmetic on either.
//!
//! [`TAG`]: crate::value::TAG

use std::collections::BTreeMap;

use rustc_abi::Size;
use rustc_data_structures::fx::FxHashMap;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::interpret::GlobalAlloc;
use rustc_middle::mir::ConstValue;
use rustc_middle::ty::{self, Instance, Ty, TyCtxt};

use crate::value::{enum_repr, EnumRepr, TAG};

pub(crate) use rcgjs_object::dts::Dts;

/// How deep a type is followed before it is called opaque. A type that nests further than this is
/// not one an island entry point takes.
const MAX_DEPTH: u32 = 8;

/// TypeScript spelling discovered from a `#[js_opaque]` associated constant (`__MIRES_TS_TYPE`).
enum OpaqueTs {
    /// Non-empty explicit TS type spelling (e.g. `"JSX.Element"`), emitted inline.
    Explicit(String),
    /// Empty `__MIRES_TS_TYPE`, emitted as a branded unique-symbol type alias.
    Branded(String),
}

/// Per-signature `.d.ts` synthesis state: tracks named type bodies and resolves clean short names
/// with automatic full-path disambiguation on intra-signature collisions.
struct DtsState<'tcx> {
    tcx: TyCtxt<'tcx>,
    types: BTreeMap<String, String>,
    name_owners: BTreeMap<String, Ty<'tcx>>,
    resolved_names: FxHashMap<Ty<'tcx>, String>,
}

impl<'tcx> DtsState<'tcx> {
    fn new(tcx: TyCtxt<'tcx>) -> Self {
        Self {
            tcx,
            types: BTreeMap::new(),
            name_owners: BTreeMap::new(),
            resolved_names: FxHashMap::default(),
        }
    }

    /// Resolves a TypeScript alias name for `ty`, returning `(name, already_registered)`.
    fn resolve_nominal_name(&mut self, ty: Ty<'tcx>, is_js_export: bool) -> (String, bool) {
        if let Some(existing) = self.resolved_names.get(&ty) {
            return (existing.clone(), true);
        }
        let preferred = if is_js_export {
            ts_short_name(self.tcx, ty)
        } else {
            ts_name(self.tcx, ty)
        };
        let chosen = self.claim_name(ty, preferred);
        self.resolved_names.insert(ty, chosen.clone());
        (chosen, false)
    }

    fn claim_name(&mut self, ty: Ty<'tcx>, preferred: String) -> String {
        if !self.name_owners.contains_key(&preferred) {
            self.name_owners.insert(preferred.clone(), ty);
            return preferred;
        }
        let full = ts_name(self.tcx, ty);
        if !self.name_owners.contains_key(&full) {
            self.name_owners.insert(full.clone(), ty);
            return full;
        }
        let mut suffix = 2u32;
        loop {
            let candidate = format!("{full}_{suffix}");
            if !self.name_owners.contains_key(&candidate) {
                self.name_owners.insert(candidate.clone(), ty);
                return candidate;
            }
            suffix += 1;
        }
    }
}

/// The signature of `instance`, whose emitted parameters are `params`.
///
/// When `is_js_export` is `true` (`#[js_export]` / `link_section = "rcgjs.export.v1"`), the
/// generated TypeScript describes the marshaled JavaScript boundary surface (`string` for
/// `String`, `T[]` for `Vec<T>`/`&[T]`, `T | undefined` / `field?: T` for `Option<T>`, unwrapped
/// `T` with `@throws` for top-level `Result<T, E>`, and native types for `JsValue`/`JsString`/
/// `JsArray`). When `false`, the signature reflects the raw internal value model unchanged.
///
/// Returns `None` when the emitted parameter list and the Rust signature disagree in length, which
/// is every shape that takes a hidden argument: a `#[track_caller]` callee takes a `Location`, and
/// a `"rust-call"` body takes its arguments splatted out of a tuple. Neither is an exported entry
/// point, and describing one wrongly would be worse than not describing it.
pub(crate) fn signature<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
    params: &[String],
    is_js_export: bool,
) -> Option<Dts> {
    let sig = tcx.normalize_erasing_late_bound_regions(
        crate::value::typing_env(),
        instance.ty(tcx, crate::value::typing_env()).fn_sig(tcx),
    );
    if sig.inputs().len() != params.len() {
        return None;
    }

    let mut state = DtsState::new(tcx);
    let optional_tail_start = if is_js_export {
        sig.inputs()
            .iter()
            .rposition(|ty| option_inner_ty(tcx, *ty).is_none())
            .map_or(0, |idx| idx + 1)
    } else {
        sig.inputs().len()
    };
    let named = params
        .iter()
        .zip(sig.inputs())
        .enumerate()
        .map(|(idx, (name, ty))| {
            if is_js_export && idx >= optional_tail_start {
                if let Some(inner) = option_inner_ty(tcx, *ty) {
                    return (format!("{name}?"), ts_type(&mut state, inner, 0, true));
                }
            }
            (name.clone(), ts_type(&mut state, *ty, 0, is_js_export))
        })
        .collect();
    let mut rust = format!("{sig}");
    let ret = if is_js_export {
        if let Some(inner_ty) = typed_promise_inner(tcx, sig.output()) {
            if let Some((ok_ty, err_ty)) = result_type_args(tcx, inner_ty) {
                let ok_ts = if ok_ty.is_unit() {
                    "void".to_owned()
                } else {
                    ts_type(&mut state, ok_ty, 0, true)
                };
                let err_ts = ts_type(&mut state, err_ty, 0, true);
                rust = format!("{sig}\n@throws {{{err_ts}}}");
                format!("Promise<{ok_ts}>")
            } else if inner_ty.is_unit() {
                "Promise<void>".to_owned()
            } else {
                format!("Promise<{}>", ts_type(&mut state, inner_ty, 0, true))
            }
        } else if let Some((ok_ty, err_ty)) = result_type_args(tcx, sig.output()) {
            let ok_ts = if ok_ty.is_unit() {
                "void".to_owned()
            } else {
                ts_type(&mut state, ok_ty, 0, true)
            };
            let err_ts = ts_type(&mut state, err_ty, 0, true);
            rust = format!("{sig}\n@throws {{{err_ts}}}");
            ok_ts
        } else if sig.output().is_unit() {
            "void".to_owned()
        } else {
            ts_type(&mut state, sig.output(), 0, true)
        }
    } else {
        match sig.output().is_unit() {
            true => "void".to_owned(),
            false => ts_type(&mut state, sig.output(), 0, false),
        }
    };

    Some(Dts {
        params: named,
        ret,
        types: state.types.into_iter().collect(),
        rust,
        doc: extract_doc(tcx, instance),
    })
}

/// Extracts `///` doc comments from an exported function item, if any.
fn extract_doc<'tcx>(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) -> Option<String> {
    let ty::InstanceKind::Item(def_id) = instance.def else {
        return None;
    };
    let mut lines = Vec::new();
    #[allow(
        deprecated,
        reason = "collects all doc attributes on a DefId in declaration order"
    )]
    let attrs = tcx.get_all_attrs(def_id);
    for attr in attrs {
        let Some(doc) = attr.doc_str() else {
            continue;
        };
        for line in doc.as_str().lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                lines.push(trimmed.replace("*/", "*\\/"));
            }
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// Formats `inner` as a TypeScript array type, parenthesizing unions (`(A | B)[]`).
fn ts_array_type(inner: String) -> String {
    if inner.contains(" | ") {
        format!("({inner})[]")
    } else {
        format!("{inner}[]")
    }
}

/// Whether `did` is `alloc::string::String` (`std::string::String`).
fn is_string_def(tcx: TyCtxt<'_>, did: rustc_hir::def_id::DefId) -> bool {
    tcx.lang_items().string() == Some(did)
        || tcx
            .get_diagnostic_name(did)
            .is_some_and(|name| name.as_str() == "String")
        || matches!(
            tcx.def_path_str(did).as_str(),
            "alloc::string::String" | "std::string::String"
        )
}

/// Whether `did` is `alloc::vec::Vec` (`std::vec::Vec`).
fn is_vec_def(tcx: TyCtxt<'_>, did: rustc_hir::def_id::DefId) -> bool {
    tcx.get_diagnostic_name(did)
        .is_some_and(|name| name.as_str() == "Vec")
        || matches!(
            tcx.def_path_str(did).as_str(),
            "alloc::vec::Vec" | "std::vec::Vec"
        )
}

/// Whether `did` is `core::option::Option` (`std::option::Option`).
fn is_option_def(tcx: TyCtxt<'_>, did: rustc_hir::def_id::DefId) -> bool {
    tcx.lang_items().option_type() == Some(did)
        || tcx
            .get_diagnostic_name(did)
            .is_some_and(|name| name.as_str() == "Option")
        || matches!(
            tcx.def_path_str(did).as_str(),
            "core::option::Option" | "std::option::Option"
        )
}

/// Whether `did` is `core::result::Result` (`std::result::Result`).
fn is_result_def(tcx: TyCtxt<'_>, did: rustc_hir::def_id::DefId) -> bool {
    tcx.get_diagnostic_name(did)
        .is_some_and(|name| name.as_str() == "Result")
        || matches!(
            tcx.def_path_str(did).as_str(),
            "core::result::Result" | "std::result::Result"
        )
}

/// If `ty` is `Option<T>`, returns `Some(T)`.
fn option_inner_ty<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
    let ty::Adt(def, args) = ty.kind() else {
        return None;
    };
    if !is_option_def(tcx, def.did()) {
        return None;
    }
    args.types().next()
}

/// If `ty` is `Result<T, E>`, returns `Some((T, E))`.
fn result_type_args<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<(Ty<'tcx>, Ty<'tcx>)> {
    let ty::Adt(def, args) = ty.kind() else {
        return None;
    };
    if !is_result_def(tcx, def.did()) {
        return None;
    }
    let mut types = args.types();
    Some((types.next()?, types.next()?))
}

/// Formats a registered named type key for [`Dts::types`]. `#[js_export]` modules prefix named
/// types with `"export "` so `rcgjs_object::dts::module_dts` emits `export type Name = ...;`.
fn type_key(name: &str, is_js_export: bool) -> String {
    if is_js_export {
        format!("export {name}")
    } else {
        name.to_owned()
    }
}

/// Whether `did` names a known `mires_js` item.
fn is_mires_js_type(
    tcx: TyCtxt<'_>,
    did: rustc_hir::def_id::DefId,
    expected_item: &str,
    expected_suffix: &str,
) -> bool {
    let crate_name = tcx.crate_name(did.krate);
    let item_name = tcx.item_name(did);
    if crate_name.as_str() == "mires_js" && item_name.as_str() == expected_item {
        return true;
    }
    let path = tcx.def_path_str(did);
    path == expected_suffix || path.ends_with(&format!("::{expected_suffix}"))
}

/// If `ty` is `mires_js::promise::TypedPromise<R>`, returns `Some(R)`.
fn typed_promise_inner<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
    let ty::Adt(def, args) = ty.kind() else {
        return None;
    };
    if !is_mires_js_type(
        tcx,
        def.did(),
        "TypedPromise",
        "mires_js::promise::TypedPromise",
    ) {
        return None;
    }
    args.types().next()
}

/// Whether `did` is `std::collections::HashMap`, `mires_std::collections::HashMap`, or
/// `alloc::collections::BTreeMap` (`std::collections::BTreeMap`).
fn is_std_map_def(tcx: TyCtxt<'_>, did: rustc_hir::def_id::DefId) -> bool {
    if tcx
        .get_diagnostic_name(did)
        .is_some_and(|name| matches!(name.as_str(), "HashMap" | "BTreeMap"))
    {
        return true;
    }
    let crate_name = tcx.crate_name(did.krate);
    let item_name = tcx.item_name(did);
    if matches!(crate_name.as_str(), "std" | "mires_std") && item_name.as_str() == "HashMap" {
        return true;
    }
    if matches!(crate_name.as_str(), "alloc" | "std" | "mires_std")
        && item_name.as_str() == "BTreeMap"
    {
        return true;
    }
    matches!(
        tcx.def_path_str(did).as_str(),
        "std::collections::HashMap"
            | "std::collections::hash_map::HashMap"
            | "std::collections::hash::map::HashMap"
            | "mires_std::collections::HashMap"
            | "mires_std::collections::hash_map::HashMap"
            | "alloc::collections::BTreeMap"
            | "alloc::collections::btree::map::BTreeMap"
            | "std::collections::BTreeMap"
    )
}

/// Whether map key type `ty` is a string key (`String`, `&str`, or `str`), which marshals to
/// TypeScript `Record<string, V>` rather than `Map<K, V>`.
fn is_string_key_ty<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    match ty.kind() {
        ty::Str => true,
        ty::Ref(_, pointee, mutbl) if !mutbl.is_mut() => pointee.is_str(),
        ty::Adt(def, _) => is_string_def(tcx, def.did()),
        _ => false,
    }
}

/// Formats a map-like `(K, V)` generic type as `Record<string, V>` or `Map<K, V>`.
fn ts_map_type<'tcx>(
    state: &mut DtsState<'tcx>,
    args: ty::GenericArgsRef<'tcx>,
    depth: u32,
) -> String {
    let mut type_args = args.types();
    let Some(key_ty) = type_args.next() else {
        return "Map<unknown, unknown>".to_owned();
    };
    let val_ts = type_args
        .next()
        .map(|val_ty| ts_type(state, val_ty, depth + 1, true))
        .unwrap_or_else(|| "unknown".to_owned());
    if is_string_key_ty(state.tcx, key_ty) {
        format!("Record<string, {val_ts}>")
    } else {
        let key_ts = ts_type(state, key_ty, depth + 1, true);
        format!("Map<{key_ts}, {val_ts}>")
    }
}

/// Recognized `mires_js` host handle and boundary adapter types for `#[js_export]` `.d.ts` output.
fn mires_js_host_type<'tcx>(
    state: &mut DtsState<'tcx>,
    did: DefId,
    args: ty::GenericArgsRef<'tcx>,
    depth: u32,
) -> Option<String> {
    let tcx = state.tcx;
    if is_mires_js_type(tcx, did, "JsValue", "mires_js::host::JsValue") {
        return Some("unknown".to_owned());
    }
    if is_mires_js_type(tcx, did, "JsString", "mires_js::host_value::JsString") {
        return Some("string".to_owned());
    }
    if is_mires_js_type(tcx, did, "JsArray", "mires_js::host_value::JsArray") {
        let elem = args
            .types()
            .next()
            .map(|inner| ts_type(state, inner, depth + 1, true))
            .unwrap_or_else(|| "unknown".to_owned());
        return Some(ts_array_type(elem));
    }
    if is_mires_js_type(tcx, did, "Promise", "mires_js::promise::Promise") {
        return Some("Promise<unknown>".to_owned());
    }
    if is_mires_js_type(tcx, did, "TypedPromise", "mires_js::promise::TypedPromise") {
        let Some(inner_ty) = args.types().next() else {
            return Some("Promise<unknown>".to_owned());
        };
        if let Some((ok_ty, _)) = result_type_args(tcx, inner_ty) {
            let ok_ts = if ok_ty.is_unit() {
                "void".to_owned()
            } else {
                ts_type(state, ok_ty, depth + 1, true)
            };
            return Some(format!("Promise<{ok_ts}>"));
        }
        if inner_ty.is_unit() {
            return Some("Promise<void>".to_owned());
        }
        return Some(format!(
            "Promise<{}>",
            ts_type(state, inner_ty, depth + 1, true)
        ));
    }
    if is_mires_js_type(
        tcx,
        did,
        "JsMapEntries",
        "mires_js::map_entries::JsMapEntries",
    ) || is_mires_js_type(
        tcx,
        did,
        "JsBTreeMapEntries",
        "mires_js::map_entries::JsBTreeMapEntries",
    ) || is_mires_js_type(tcx, did, "HashMap", "mires_js::collections::map::HashMap")
    {
        return Some(ts_map_type(state, args, depth));
    }
    None
}

/// The TypeScript for `ty`, registering any named types it needs in `state`.
fn ts_type<'tcx>(
    state: &mut DtsState<'tcx>,
    ty: Ty<'tcx>,
    depth: u32,
    is_js_export: bool,
) -> String {
    if depth > MAX_DEPTH {
        return "unknown".to_owned();
    }
    let tcx = state.tcx;
    match ty.kind() {
        ty::Bool => "boolean".to_owned(),
        // A `char` is a code POINT in the value model, not a one-character string.
        ty::Char => "number".to_owned(),
        // Wider than 53 bits, so these are `BigInt`s and not numbers. Getting this wrong is the
        // one primitive mistake that survives testing: a small `u64` reads fine as a number.
        ty::Int(ty::IntTy::I64 | ty::IntTy::I128)
        | ty::Uint(ty::UintTy::U64 | ty::UintTy::U128) => "bigint".to_owned(),
        ty::Int(_) | ty::Uint(_) | ty::Float(_) => "number".to_owned(),
        ty::Str => "string".to_owned(),
        ty::Never => "never".to_owned(),
        ty::Ref(_, pointee, mutbl) => {
            if is_js_export && !mutbl.is_mut() {
                if let ty::Slice(elem) = pointee.kind() {
                    let elem_ts = ts_type(state, *elem, depth + 1, true);
                    return ts_array_type(elem_ts);
                }
            }
            match crate::value::is_slot(tcx, ty)
                || (mutbl.is_mut() && matches!(pointee.kind(), ty::Str))
            {
                // A reference the model keeps in a slot record (or a mutable `str` byte slice) is
                // a pointer into a buffer, which has no useful TypeScript type: it is the model's
                // own bookkeeping.
                true => "unknown".to_owned(),
                false => ts_type(state, *pointee, depth + 1, is_js_export),
            }
        }
        ty::Tuple(fields) if fields.is_empty() => "undefined".to_owned(),
        ty::Tuple(fields) => {
            let members: Vec<String> = fields
                .iter()
                .map(|field| ts_type(state, field, depth + 1, is_js_export))
                .collect();
            format!("[{}]", members.join(", "))
        }
        ty::Adt(def, args) => {
            if is_js_export {
                if let Some(host_ts) = mires_js_host_type(state, def.did(), args, depth) {
                    return host_ts;
                }
                if is_string_def(tcx, def.did()) {
                    return "string".to_owned();
                }
                if is_vec_def(tcx, def.did()) {
                    let elem = args
                        .types()
                        .next()
                        .map(|inner| ts_type(state, inner, depth + 1, true))
                        .unwrap_or_else(|| "unknown".to_owned());
                    return ts_array_type(elem);
                }
                if is_std_map_def(tcx, def.did()) {
                    return ts_map_type(state, args, depth);
                }
                if is_option_def(tcx, def.did()) {
                    let inner = args
                        .types()
                        .next()
                        .map(|inner| ts_type(state, inner, depth + 1, true))
                        .unwrap_or_else(|| "unknown".to_owned());
                    return format!("{inner} | undefined");
                }
                if is_result_def(tcx, def.did()) {
                    let mut type_args = args.types();
                    let ok_ts = type_args
                        .next()
                        .map(|inner| ts_type(state, inner, depth + 1, true))
                        .unwrap_or_else(|| "unknown".to_owned());
                    let err_ts = type_args
                        .next()
                        .map(|inner| ts_type(state, inner, depth + 1, true))
                        .unwrap_or_else(|| "unknown".to_owned());
                    return format!(
                        "{{ {TAG}: \"Ok\"; _0: {ok_ts} }} | {{ {TAG}: \"Err\"; _0: {err_ts} }}"
                    );
                }
            }
            if let Some(opaque) = opaque_handle(tcx, def.did()) {
                return match opaque {
                    OpaqueTs::Explicit(ts) => ts,
                    OpaqueTs::Branded(preferred) => {
                        let existing_same_def = state.name_owners.get(&preferred).is_some_and(
                            |owner| matches!(owner.kind(), ty::Adt(d, _) if d.did() == def.did()),
                        );
                        let name = if existing_same_def {
                            preferred
                        } else {
                            state.claim_name(ty, preferred)
                        };
                        state.types.insert(
                            type_key(&name, is_js_export),
                            format!("{{ readonly __{name}: unique symbol }}"),
                        );
                        name
                    }
                };
            }
            match def.is_enum() {
                true => ts_enum(state, ty, *def, args, depth, is_js_export),
                false => ts_struct(state, ty, *def, args, depth, is_js_export),
            }
        }
        _ => "unknown".to_owned(),
    }
}

/// Evaluates `#[js_opaque]`'s hidden `__MIRES_TS_TYPE: &'static str` associated constant on
/// `def_id`, if present.
///
/// Checked before [`ts_struct`] so a `#[js_opaque]` `#[repr(transparent)]` wrapper around
/// `HostHandle` never peels down to `number`.
fn opaque_handle(tcx: TyCtxt<'_>, def_id: DefId) -> Option<OpaqueTs> {
    for &impl_id in tcx.inherent_impls(def_id) {
        for item in tcx.associated_items(impl_id).in_definition_order() {
            if !matches!(item.kind, ty::AssocKind::Const { .. }) {
                continue;
            }
            if item
                .opt_name()
                .is_none_or(|name| name.as_str() != mires_marker_abi::export::OPAQUE_TS_CONST_NAME)
            {
                continue;
            }
            let Ok(value) = tcx.const_eval_poly(item.def_id) else {
                return None;
            };
            let text = const_value_str(tcx, value)?;
            return if text.is_empty() {
                let raw = pascal_ident(tcx.item_name(def_id).as_str());
                let name =
                    if raw.is_empty() || raw.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                        format!("T{raw}")
                    } else {
                        raw
                    };
                Some(OpaqueTs::Branded(name))
            } else {
                Some(OpaqueTs::Explicit(text))
            };
        }
    }
    None
}

/// Extracts a UTF-8 string from an evaluated `&'static str` constant value.
fn const_value_str(tcx: TyCtxt<'_>, value: ConstValue) -> Option<String> {
    let ConstValue::Slice { alloc_id, meta } = value else {
        return None;
    };
    if meta == 0 {
        return Some(String::new());
    }
    let GlobalAlloc::Memory(allocation) = tcx.global_alloc(alloc_id) else {
        return None;
    };
    crate::const_raw::read_str(allocation.inner(), Size::ZERO, meta)
        .ok()
        .map(str::to_owned)
}

/// A struct, as an object type keyed by field name.
fn ts_struct<'tcx>(
    state: &mut DtsState<'tcx>,
    ty: Ty<'tcx>,
    def: ty::AdtDef<'tcx>,
    args: ty::GenericArgsRef<'tcx>,
    depth: u32,
    is_js_export: bool,
) -> String {
    let tcx = state.tcx;
    let variant = def.non_enum_variant();
    // A `repr(transparent)` struct IS its single non-ZST field in the value model.
    if def.repr().transparent() {
        if let Some(field_ty) = crate::value::transparent_field(tcx, ty) {
            return ts_type(state, field_ty, depth + 1, is_js_export);
        }
    }
    let (name, already_registered) = state.resolve_nominal_name(ty, is_js_export);
    if already_registered {
        return name;
    }
    let key = type_key(&name, is_js_export);
    // Registered before the fields are walked, so a struct that mentions itself terminates.
    state.types.insert(key.clone(), "unknown".to_owned());
    let members: Vec<String> = variant
        .fields
        .iter()
        .map(|field| {
            let rust_field_ty =
                tcx.normalize_erasing_regions(crate::value::typing_env(), field.ty(tcx, args));
            let field_name = ts_field_name(field.name.as_str(), is_js_export);
            if is_js_export {
                if let Some(inner) = option_inner_ty(tcx, rust_field_ty) {
                    let inner_ts = ts_type(state, inner, depth + 1, true);
                    return format!("{field_name}?: {inner_ts}");
                }
            }
            let field_ty = ts_type(state, rust_field_ty, depth + 1, is_js_export);
            format!("{field_name}: {field_ty}")
        })
        .collect();
    state.types.insert(key, object(&members));
    name
}

/// Formats a struct or variant field name for TypeScript (`_0` for positional fields on
/// `#[js_export]` boundaries to match the runtime object keys; raw Rust field names otherwise).
fn ts_field_name(name: &str, is_js_export: bool) -> String {
    if is_js_export && name.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{name}")
    } else if is_js_export && name == TAG {
        format!("{name}_")
    } else {
        name.to_owned()
    }
}

/// An object type, written `{}` when it has no members rather than `{  }`.
fn object(members: &[String]) -> String {
    match members.is_empty() {
        true => "{}".to_owned(),
        false => format!("{{ {} }}", members.join("; ")),
    }
}

/// An enum, as whatever its runtime representation is.
fn ts_enum<'tcx>(
    state: &mut DtsState<'tcx>,
    ty: Ty<'tcx>,
    def: ty::AdtDef<'tcx>,
    args: ty::GenericArgsRef<'tcx>,
    depth: u32,
    is_js_export: bool,
) -> String {
    let tcx = state.tcx;
    let Some(repr) = enum_repr(tcx, ty) else {
        return "unknown".to_owned();
    };
    match repr {
        // A `repr(int)` fieldless enum IS its discriminant, which is what makes a transmute to
        // that integer a move with no shape change. There is no narrower type for it.
        EnumRepr::Number => return "number".to_owned(),
        // A coroutine's states have no names to spell, so its tag is an index.
        EnumRepr::State => return format!("{{ {TAG}: number }}"),
        EnumRepr::Tag { .. } => {}
    }
    if def.variants().is_empty() {
        return "never".to_owned();
    }

    let (name, already_registered) = state.resolve_nominal_name(ty, is_js_export);
    if already_registered {
        return name;
    }
    let key = type_key(&name, is_js_export);
    state.types.insert(key.clone(), "unknown".to_owned());

    let fieldless = matches!(repr, EnumRepr::Tag { fieldless: true });
    let arms: Vec<String> = def
        .variants()
        .iter()
        .map(|variant| {
            // A wholly fieldless enum is the bare string; a mixed one keeps the object form even
            // for its empty variants, because such a value has to have an identity to write
            // through.
            if fieldless {
                return format!("\"{}\"", variant.name);
            }
            let mut members = vec![format!("{TAG}: \"{}\"", variant.name)];
            members.extend(variant.fields.iter().map(|field| {
                let rust_field_ty =
                    tcx.normalize_erasing_regions(crate::value::typing_env(), field.ty(tcx, args));
                let field_name = ts_field_name(field.name.as_str(), is_js_export);
                if is_js_export {
                    if let Some(inner) = option_inner_ty(tcx, rust_field_ty) {
                        let inner_ts = ts_type(state, inner, depth + 1, true);
                        return format!("{field_name}?: {inner_ts}");
                    }
                }
                let field_ty = ts_type(state, rust_field_ty, depth + 1, is_js_export);
                format!("{field_name}: {field_ty}")
            }));
            object(&members)
        })
        .collect();
    state.types.insert(key, arms.join(" | "));
    name
}

/// Converts an arbitrary Rust identifier or path string into a PascalCase TypeScript fragment.
fn pascal_ident(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut fresh = true;
    for character in raw.chars() {
        match character.is_ascii_alphanumeric() {
            true => {
                out.push(match fresh {
                    true => character.to_ascii_uppercase(),
                    false => character,
                });
                fresh = false;
            }
            false => fresh = true,
        }
    }
    out
}

/// Clean short TypeScript name for a `#[js_export]` type: the item's own ident plus PascalCase
/// generic type arguments (e.g. `shared::TaskItem` -> `TaskItem`, `Page<User>` -> `PageUser`).
fn ts_short_name<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> String {
    let out = ts_short_fragment(tcx, ty, 0);
    match out.chars().next().is_some_and(|c| c.is_ascii_digit()) || out.is_empty() {
        true => format!("T{out}"),
        false => out,
    }
}

fn ts_short_fragment<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, depth: u32) -> String {
    if depth > MAX_DEPTH {
        return "Unknown".to_owned();
    }
    match ty.kind() {
        ty::Adt(def, args) => {
            let mut out = pascal_ident(tcx.item_name(def.did()).as_str());
            for arg in args.types() {
                out.push_str(&ts_short_fragment(tcx, arg, depth + 1));
            }
            out
        }
        ty::Ref(_, pointee, _) => ts_short_fragment(tcx, *pointee, depth + 1),
        ty::Slice(elem) => format!("Slice{}", ts_short_fragment(tcx, *elem, depth + 1)),
        ty::Array(elem, _) => format!("Array{}", ts_short_fragment(tcx, *elem, depth + 1)),
        ty::Tuple(fields) if fields.is_empty() => "Unit".to_owned(),
        ty::Tuple(fields) => {
            let mut out = String::from("Tuple");
            for field in fields.iter() {
                out.push_str(&ts_short_fragment(tcx, field, depth + 1));
            }
            out
        }
        _ => pascal_ident(&rustc_middle::ty::print::with_no_trimmed_paths!(format!(
            "{ty}"
        ))),
    }
}

/// The TypeScript name for a named type: its full Rust path, with everything TypeScript will not
/// take replaced. Used for raw exports and as the collision fallback for short names.
fn ts_name<'tcx>(_tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> String {
    let printed = rustc_middle::ty::print::with_no_trimmed_paths!(format!("{ty}"));
    let out = pascal_ident(&printed);
    match out.chars().next().is_some_and(|c| c.is_ascii_digit()) || out.is_empty() {
        true => format!("T{out}"),
        false => out,
    }
}

// -------------------------------------------------------------------------------------------
// The file
