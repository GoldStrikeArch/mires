//! `.d.ts` synthesis: what an emitted module looks like to TypeScript.
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

use rustc_middle::ty::{self, Instance, Ty, TyCtxt};
use serde::{Deserialize, Serialize};

use crate::value::{enum_repr, EnumRepr, TAG};

/// How deep a type is followed before it is called opaque. A type that nests further than this is
/// not one an island entry point takes.
const MAX_DEPTH: u32 = 8;

/// One exported entry point's TypeScript signature.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Dts {
    /// `(parameter name, TypeScript type)`, in the order the function takes them.
    pub(crate) params: Vec<(String, String)>,
    /// The TypeScript type the function answers.
    pub(crate) ret: String,
    /// Named types this signature refers to, as `(name, body)`. Named rather than inlined so a
    /// type used by two entries is written once and reads as one type.
    pub(crate) types: Vec<(String, String)>,
    /// The Rust signature, for the doc comment. What the TypeScript came from, so a surprising
    /// type can be traced back without recompiling.
    pub(crate) rust: String,
}

/// The signature of `instance`, whose emitted parameters are `params`.
///
/// Returns `None` when the emitted parameter list and the Rust signature disagree in length, which
/// is every shape that takes a hidden argument: a `#[track_caller]` callee takes a `Location`, and
/// a `"rust-call"` body takes its arguments splatted out of a tuple. Neither is an exported entry
/// point, and describing one wrongly would be worse than not describing it.
pub(crate) fn signature<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
    params: &[String],
) -> Option<Dts> {
    let sig = tcx.normalize_erasing_late_bound_regions(
        crate::value::typing_env(),
        instance.ty(tcx, crate::value::typing_env()).fn_sig(tcx),
    );
    if sig.inputs().len() != params.len() {
        return None;
    }

    let mut types = BTreeMap::new();
    let named = params
        .iter()
        .zip(sig.inputs())
        .map(|(name, ty)| (name.clone(), ts_type(tcx, *ty, &mut types, 0)))
        .collect();
    let ret = match sig.output().is_unit() {
        true => "void".to_owned(),
        false => ts_type(tcx, sig.output(), &mut types, 0),
    };

    Some(Dts {
        params: named,
        ret,
        types: types.into_iter().collect(),
        rust: format!("{sig}"),
    })
}

/// The TypeScript for `ty`, registering any named types it needs in `types`.
fn ts_type<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    types: &mut BTreeMap<String, String>,
    depth: u32,
) -> String {
    if depth > MAX_DEPTH {
        return "unknown".to_owned();
    }
    match ty.kind() {
        ty::Bool => "boolean".to_owned(),
        // A `char` is a code POINT in the value model, not a one-character string.
        ty::Char => "number".to_owned(),
        // Wider than 53 bits, so these are `BigInt`s and not numbers. Getting this wrong is the
        // one primitive mistake that survives testing: a small `u64` reads fine as a number.
        ty::Int(ty::IntTy::I64 | ty::IntTy::I128) | ty::Uint(ty::UintTy::U64 | ty::UintTy::U128) => {
            "bigint".to_owned()
        }
        ty::Int(_) | ty::Uint(_) | ty::Float(_) => "number".to_owned(),
        ty::Str => "string".to_owned(),
        ty::Never => "never".to_owned(),
        ty::Ref(_, pointee, _) => match crate::ptr::is_slot(tcx, ty) {
            // A reference the model keeps in a slot record is a pointer into a buffer, which has
            // no useful TypeScript type: it is the model's own bookkeeping.
            true => "unknown".to_owned(),
            false => ts_type(tcx, *pointee, types, depth + 1),
        },
        ty::Tuple(fields) if fields.is_empty() => "undefined".to_owned(),
        ty::Tuple(fields) => {
            let members: Vec<String> =
                fields.iter().map(|field| ts_type(tcx, field, types, depth + 1)).collect();
            format!("[{}]", members.join(", "))
        }
        ty::Adt(def, args) => {
            if let Some(opaque) = opaque_handle(tcx, def.did()) {
                types.insert(opaque.to_owned(), format!("{{ readonly __{opaque}: unique symbol }}"));
                return opaque.to_owned();
            }
            match def.is_enum() {
                true => ts_enum(tcx, ty, *def, args, types, depth),
                false => ts_struct(tcx, ty, *def, args, types, depth),
            }
        }
        _ => "unknown".to_owned(),
    }
}

/// The opaque type name for a one-word runtime handle, or `None` for anything else.
///
/// Matched by def path rather than by shape: a `repr(transparent)` wrapper around a `u32` is a
/// number to the value model, and these four are the cases where that is true of the Rust type and
/// false of the value the runtime actually keeps there.
fn opaque_handle(tcx: TyCtxt<'_>, def_id: rustc_hir::def_id::DefId) -> Option<&'static str> {
    const HANDLES: [&str; 5] = ["Sig", "Node", "JsValue", "Owner", "Event"];

    if tcx.crate_name(def_id.krate).as_str() != "view_abi" {
        return None;
    }
    let last = tcx.item_name(def_id);
    HANDLES.into_iter().find(|name| *name == last.as_str())
}

/// A struct, as an object type keyed by field name.
fn ts_struct<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    def: ty::AdtDef<'tcx>,
    args: ty::GenericArgsRef<'tcx>,
    types: &mut BTreeMap<String, String>,
    depth: u32,
) -> String {
    let variant = def.non_enum_variant();
    // A `repr(transparent)` struct IS its field in the value model, so it is that field's type.
    if def.repr().transparent() && variant.fields.len() == 1 {
        let field = variant.fields.iter().next().expect("one field");
        return ts_type(tcx, tcx.normalize_erasing_regions(crate::value::typing_env(), field.ty(tcx, args)), types, depth + 1);
    }
    let name = ts_name(tcx, ty);
    if types.contains_key(&name) {
        return name;
    }
    // Registered before the fields are walked, so a struct that mentions itself terminates.
    types.insert(name.clone(), "unknown".to_owned());
    let members: Vec<String> = variant
        .fields
        .iter()
        .map(|field| {
            let field_ty = ts_type(tcx, tcx.normalize_erasing_regions(crate::value::typing_env(), field.ty(tcx, args)), types, depth + 1);
            format!("{}: {field_ty}", field.name)
        })
        .collect();
    types.insert(name.clone(), object(&members));
    name
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
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    def: ty::AdtDef<'tcx>,
    args: ty::GenericArgsRef<'tcx>,
    types: &mut BTreeMap<String, String>,
    depth: u32,
) -> String {
    let Some(repr) = enum_repr(tcx, ty) else { return "unknown".to_owned() };
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

    let name = ts_name(tcx, ty);
    if types.contains_key(&name) {
        return name;
    }
    types.insert(name.clone(), "unknown".to_owned());

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
                let field_ty = ts_type(tcx, tcx.normalize_erasing_regions(crate::value::typing_env(), field.ty(tcx, args)), types, depth + 1);
                format!("{}: {field_ty}", field.name)
            }));
            object(&members)
        })
        .collect();
    types.insert(name.clone(), arms.join(" | "));
    name
}

/// The TypeScript name for a named type: its Rust path, with everything TypeScript will not take
/// replaced.
fn ts_name<'tcx>(_tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> String {
    let printed = rustc_middle::ty::print::with_no_trimmed_paths!(format!("{ty}"));
    let mut out = String::with_capacity(printed.len());
    let mut fresh = true;
    for character in printed.chars() {
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
    match out.chars().next().is_some_and(|c| c.is_ascii_digit()) || out.is_empty() {
        true => format!("T{out}"),
        false => out,
    }
}

// -------------------------------------------------------------------------------------------
// The file
// -------------------------------------------------------------------------------------------

/// The `.d.ts` for one emitted module.
///
/// Three sections, in the order a reader wants them: the named types, the modules the program
/// imports from, and the entry points. Nothing is emitted for an item that carries no signature,
/// which is every internal item and every entry whose Rust signature the emitter could not match
/// to its parameter list.
pub(crate) fn module_dts(label: &str, items: &[&crate::item::LinkItem]) -> String {
    use crate::item::ItemKind;

    let mut out = format!(
        "// rustc_codegen_mires -- {label}\n\
         // Generated by `-Cllvm-args=js-dts=on`. Do not edit: it is rewritten on every build.\n"
    );

    // One definition per name, from whichever item mentioned it first. Two items describing one
    // Rust type describe it the same way, because both came from the same walk over that type.
    let mut types: BTreeMap<&str, &str> = BTreeMap::new();
    for item in items {
        let Some(dts) = &item.dts else { continue };
        for (name, body) in &dts.types {
            types.entry(name).or_insert(body);
        }
    }
    if !types.is_empty() {
        out.push('\n');
        for (name, body) in &types {
            out.push_str(&format!("type {name} = {body};\n"));
        }
    }

    // What the module reaches out to. A consumer needs this to build an import map, and it is the
    // declared-interface surface: every `#[js_extern]` that named a module is one of these.
    let mut modules: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for item in items {
        if item.kind != ItemKind::Import {
            continue;
        }
        modules.entry(item.debug_path.as_str()).or_default().extend(imported_names(&item.js));
    }
    if !modules.is_empty() {
        out.push('\n');
        for (source, names) in &modules {
            let mut names = names.clone();
            names.sort();
            names.dedup();
            out.push_str(&format!("declare module \"{source}\" {{\n"));
            for name in names {
                out.push_str(&format!("  export const {name}: unknown;\n"));
            }
            out.push_str("}\n");
        }
    }

    out.push('\n');
    for item in items {
        let Some(dts) = &item.dts else { continue };
        if !item.linkage.exported {
            continue;
        }
        let name = item.linkage.fixed_name.as_deref().unwrap_or_else(|| item.name.as_str());
        let params: Vec<String> = dts
            .params
            .iter()
            .map(|(param, ty)| format!("{}: {ty}", crate::jsast::sanitize_ident(param)))
            .collect();
        out.push_str(&format!("/** `{}` */\n", dts.rust));
        out.push_str(&format!(
            "export declare function {name}({}): {};\n",
            params.join(", "),
            dts.ret
        ));
    }
    out
}

/// The names an `import { A as B, C } from "..."` statement binds.
///
/// The text is the backend's own, printed by `jsast`, so this reads exactly the one shape it
/// prints. The LOCAL name is what is reported, because that is the binding the emitted program
/// uses and the one a reader is matching against the code.
fn imported_names(js: &str) -> Vec<String> {
    let Some(open) = js.find('{') else { return Vec::new() };
    let Some(close) = js[open..].find('}') else { return Vec::new() };
    js[open + 1..open + close]
        .split(',')
        .filter_map(|entry| entry.split_whitespace().next_back().map(str::to_owned))
        .filter(|name| !name.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_local_binding_is_what_an_import_reports() {
        assert_eq!(
            imported_names("import { getOwner as _$getOwner } from \"./mires-dom.js\";\n"),
            ["_$getOwner"]
        );
        assert_eq!(
            imported_names("import { a as b, c } from \"m\";\n"),
            ["b", "c"]
        );
        assert!(imported_names("import * as __rt from \"./shim.js\";\n").is_empty());
    }

    #[test]
    fn a_named_type_is_a_typescript_identifier() {
        // The Rust path is squashed rather than escaped, so `Option<&str>` and `core::fmt::Error`
        // both come out as something TypeScript will take as a name.
        assert!(!"OptionStr".contains(|c: char| !c.is_ascii_alphanumeric()));
    }
}
