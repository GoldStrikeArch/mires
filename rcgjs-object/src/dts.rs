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
//! an object under `TAG`, so it is a discriminated union and TypeScript narrows that too. Before
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
//! `TAG`: crate::value::TAG

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// One exported entry point's TypeScript signature.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dts {
    /// `(parameter name, TypeScript type)`, in the order the function takes them.
    pub params: Vec<(String, String)>,
    /// The TypeScript type the function answers.
    pub ret: String,
    /// Named types this signature refers to, as `(name, body)`. Named rather than inlined so a
    /// type used by two entries is written once and reads as one type.
    pub types: Vec<(String, String)>,
    /// The Rust signature, for the doc comment. What the TypeScript came from, so a surprising
    /// type can be traced back without recompiling.
    pub rust: String,
    /// Extracted Rust `///` doc comments, when present on the exported item.
    #[serde(default)]
    pub doc: Option<String>,
}
// -------------------------------------------------------------------------------------------

/// Prefix on exported symbol names synthesized by `#[js_export] impl ClassName`.
pub const CLASS_EXPORT_PREFIX: &str = "__mires_cls$";

/// Kind of exported helper symbol belonging to a `#[js_export] impl ClassName` block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClassExportOp<'a> {
    /// `__mires_cls$<Class>$__drop`: destructor freeing a raw handle.
    Drop,
    /// `__mires_cls$<Class>$new`: primary constructor.
    Ctor,
    /// `__mires_cls$<Class>$<method>`: `&self` / `&mut self` instance method.
    Method(&'a str),
    /// `__mires_cls$<Class>$__take$<method>`: consuming `self` instance method.
    ConsumingMethod(&'a str),
    /// `__mires_cls$<Class>$__static_ctor$<method>`: static method returning a new class instance.
    StaticCtor(&'a str),
    /// `__mires_cls$<Class>$__static$<method>`: plain static method.
    StaticMethod(&'a str),
}

/// Parses `__mires_cls$<ClassName>$<op>` into `(class_name, op)`.
pub fn parse_class_export(name: &str) -> Option<(&str, ClassExportOp<'_>)> {
    let rest = name.strip_prefix(CLASS_EXPORT_PREFIX)?;
    let (class_name, op) = rest.split_once('$')?;
    if class_name.is_empty() || op.is_empty() {
        return None;
    }
    let kind = if op == "__drop" {
        ClassExportOp::Drop
    } else if op == "new" {
        ClassExportOp::Ctor
    } else if let Some(method) = op.strip_prefix("__take$") {
        if method.is_empty() {
            return None;
        }
        ClassExportOp::ConsumingMethod(method)
    } else if let Some(method) = op.strip_prefix("__static_ctor$") {
        if method.is_empty() {
            return None;
        }
        ClassExportOp::StaticCtor(method)
    } else if let Some(method) = op.strip_prefix("__static$") {
        if method.is_empty() {
            return None;
        }
        ClassExportOp::StaticMethod(method)
    } else {
        ClassExportOp::Method(op)
    };
    Some((class_name, kind))
}

#[derive(Default)]
struct ClassDts<'a> {
    ctor: Option<(&'a Dts, &'a BTreeMap<String, String>)>,
    methods: Vec<(&'a str, &'a Dts, &'a BTreeMap<String, String>)>,
    static_members: Vec<(&'a str, &'a Dts, &'a BTreeMap<String, String>, bool)>,
    has_drop: bool,
}

fn format_params(params: &[(String, String)], renames: &BTreeMap<String, String>) -> String {
    params
        .iter()
        .map(|(param, ty)| {
            let ty = rename_ts_type_idents(ty, renames);
            if let Some(base) = param.strip_suffix('?') {
                format!("{}?: {ty}", crate::jsast::sanitize_ident(base))
            } else {
                format!("{}: {ty}", crate::jsast::sanitize_ident(param))
            }
        })
        .collect::<Vec<String>>()
        .join(", ")
}

fn push_jsdoc(
    out: &mut String,
    indent: &str,
    dts: &Dts,
    renames: &BTreeMap<String, String>,
    always_emit_sig: bool,
) {
    let (sig, extra) = match dts.rust.split_once('\n') {
        Some((sig, rest)) => (sig, Some(rest)),
        None => (dts.rust.as_str(), None),
    };
    let format_extra = |line: &str| -> String {
        if let Some(inner) = line
            .strip_prefix("@throws {")
            .and_then(|s| s.strip_suffix('}'))
        {
            format!("@throws {{{}}}", rename_ts_type_idents(inner, renames))
        } else {
            line.to_owned()
        }
    };
    match &dts.doc {
        Some(doc) if !doc.trim().is_empty() => {
            out.push_str(&format!("{indent}/**\n"));
            for line in doc.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    out.push_str(&format!("{indent} *\n"));
                } else {
                    let safe = trimmed.replace("*/", "*\\/");
                    out.push_str(&format!("{indent} * {safe}\n"));
                }
            }
            if !sig.is_empty() || extra.is_some() {
                out.push_str(&format!("{indent} *\n"));
            }
            if !sig.is_empty() {
                out.push_str(&format!("{indent} * `{sig}`\n"));
            }
            if let Some(rest) = extra {
                for line in rest.lines() {
                    out.push_str(&format!("{indent} * {}\n", format_extra(line)));
                }
            }
            out.push_str(&format!("{indent} */\n"));
        }
        _ => {
            if let Some(rest) = extra {
                out.push_str(&format!("{indent}/**\n{indent} * `{sig}`\n"));
                for line in rest.lines() {
                    out.push_str(&format!("{indent} * {}\n", format_extra(line)));
                }
                out.push_str(&format!("{indent} */\n"));
            } else if always_emit_sig {
                out.push_str(&format!("{indent}/** `{sig}` */\n"));
            }
        }
    }
}

/// Whether `rest` (immediately following an identifier token) marks that identifier as an object
/// type property name (`name: T` or `name?: T`).
fn is_property_key(rest: &str) -> bool {
    let trimmed = rest.trim_start();
    trimmed.starts_with(':')
        || trimmed
            .strip_prefix('?')
            .is_some_and(|after_q| after_q.trim_start().starts_with(':'))
}

fn is_ts_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$')
}

fn is_ts_ident_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$')
}

/// Rewrites TypeScript type references according to `renames`, preserving string literals,
/// object property keys, and member segments (such as `JSX.Element`).
fn rename_ts_type_idents(ty: &str, renames: &BTreeMap<String, String>) -> String {
    if renames.is_empty() {
        return ty.to_owned();
    }
    let bytes = ty.as_bytes();
    let mut out = String::with_capacity(ty.len());
    let mut i = 0;
    let mut last_non_ws: Option<u8> = None;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'"' {
            let start = i;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push_str(&ty[start..i]);
            last_non_ws = Some(b'"');
            continue;
        }
        if is_ts_ident_start(byte) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ts_ident_continue(bytes[i]) {
                i += 1;
            }
            let ident = &ty[start..i];
            if last_non_ws != Some(b'.') && !is_property_key(&ty[i..]) {
                if let Some(renamed) = renames.get(ident) {
                    out.push_str(renamed);
                } else {
                    out.push_str(ident);
                }
            } else {
                out.push_str(ident);
            }
            last_non_ws = Some(bytes[i - 1]);
            continue;
        }
        let ch = ty[i..].chars().next().expect("valid utf-8 boundary");
        out.push(ch);
        if !ch.is_ascii_whitespace() {
            last_non_ws = Some(byte);
        }
        i += ch.len_utf8();
    }
    out
}

fn alloc_disambiguated(reserved: &mut BTreeSet<String>, base: &str) -> String {
    let mut suffix = 2u32;
    loop {
        let candidate = format!("{base}_{suffix}");
        if reserved.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

struct CollectedTypes {
    types: BTreeMap<String, (bool, String)>,
    per_item_renames: Vec<BTreeMap<String, String>>,
}

/// Collects and deduplicates named types across all exported items in one module, disambiguating
/// any short-name collisions whose definitions differ (`Item`, `Item_2`, ...).
fn collect_module_types(items: &[&crate::item::LinkItem]) -> CollectedTypes {
    let mut types: BTreeMap<String, (bool, String)> = BTreeMap::new();
    let mut by_bare: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut reserved: BTreeSet<String> = BTreeSet::new();

    for item in items {
        if !item.linkage.exported {
            continue;
        }
        let Some(dts) = &item.dts else { continue };
        for (name, _) in &dts.types {
            let bare = name.strip_prefix("export ").unwrap_or(name.as_str());
            reserved.insert(bare.to_owned());
        }
    }

    let mut per_item_renames = Vec::with_capacity(items.len());
    for item in items {
        if !item.linkage.exported {
            per_item_renames.push(BTreeMap::new());
            continue;
        }
        let Some(dts) = &item.dts else {
            per_item_renames.push(BTreeMap::new());
            continue;
        };
        let entries: Vec<(bool, &str, &str)> = dts
            .types
            .iter()
            .map(|(name, body)| match name.strip_prefix("export ") {
                Some(rest) => (true, rest, body.as_str()),
                None => (false, name.as_str(), body.as_str()),
            })
            .collect();

        let mut item_renames: BTreeMap<String, String> = BTreeMap::new();
        let mut allocated: BTreeSet<String> = BTreeSet::new();

        for _ in 0..=entries.len() {
            let mut changed = false;
            for &(_, bare, body) in &entries {
                if allocated.contains(bare) {
                    continue;
                }
                let Some(existing_list) = by_bare.get(bare) else {
                    continue;
                };
                let rewritten = rename_ts_type_idents(body, &item_renames);
                let matched = existing_list
                    .iter()
                    .find_map(|(existing_name, existing_body)| {
                        if *existing_body == rewritten {
                            return Some(existing_name.clone());
                        }
                        if existing_name != bare {
                            let mut probe = item_renames.clone();
                            probe.insert(bare.to_owned(), existing_name.clone());
                            if rename_ts_type_idents(body, &probe) == *existing_body {
                                return Some(existing_name.clone());
                            }
                        }
                        None
                    });
                match matched {
                    Some(existing_name) => {
                        if existing_name == bare {
                            if item_renames.remove(bare).is_some() {
                                changed = true;
                            }
                        } else if item_renames.get(bare) != Some(&existing_name) {
                            item_renames.insert(bare.to_owned(), existing_name);
                            changed = true;
                        }
                    }
                    None => {
                        let fresh = alloc_disambiguated(&mut reserved, bare);
                        allocated.insert(bare.to_owned());
                        item_renames.insert(bare.to_owned(), fresh);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }

        for &(exported, bare, body) in &entries {
            let final_name = item_renames
                .get(bare)
                .cloned()
                .unwrap_or_else(|| bare.to_owned());
            let final_body = rename_ts_type_idents(body, &item_renames);
            types
                .entry(final_name.clone())
                .and_modify(|(was_exported, _)| {
                    if exported {
                        *was_exported = true;
                    }
                })
                .or_insert_with(|| (exported, final_body.clone()));
            let list = by_bare.entry(bare.to_owned()).or_default();
            if !list.iter().any(|(name, _)| *name == final_name) {
                list.push((final_name, final_body));
            }
        }

        per_item_renames.push(item_renames);
    }

    CollectedTypes {
        types,
        per_item_renames,
    }
}

/// The `.d.ts` for one emitted module.
///
/// Three sections, in the order a reader wants them: the named types, the modules the program
/// imports from, and the entry points (functions and synthesized classes). Nothing is emitted for
/// an item that carries no signature, which is every internal item and every entry whose Rust
/// signature the emitter could not match to its parameter list.
pub fn module_dts(label: &str, items: &[&crate::item::LinkItem]) -> String {
    use crate::item::ItemKind;

    let mut out = format!(
        "// rustc_codegen_js -- {label}\n\
         // Generated by `-Cllvm-args=js-dts=on`. Do not edit: it is rewritten on every build.\n"
    );

    let CollectedTypes {
        types,
        per_item_renames,
    } = collect_module_types(items);
    if !types.is_empty() {
        out.push('\n');
        for (name, (exported, body)) in &types {
            if *exported {
                out.push_str(&format!("export type {name} = {body};\n"));
            } else {
                out.push_str(&format!("type {name} = {body};\n"));
            }
        }
    }

    // What the module reaches out to. A consumer needs this to build an import map, and it is the
    // declared-interface surface: every `#[js_extern]` that named a module is one of these.
    // Linker-synthesized imports (runtime shim namespace and cross-chunk wiring, `crate_name ==
    // "generated"`) are internal plumbing rather than declared external modules; emitting
    // `declare module "./shim.mjs"` in a `.d.mts` file also fails TypeScript module augmentation
    // checks (`TS2665`).
    let mut modules: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for item in items {
        if item.kind != ItemKind::Import || item.crate_name == "generated" {
            continue;
        }
        let names = imported_names(&item.js);
        if names.is_empty() {
            continue;
        }
        modules
            .entry(item.debug_path.as_str())
            .or_default()
            .extend(names);
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

    let mut classes: BTreeMap<&str, ClassDts<'_>> = BTreeMap::new();
    let mut free_fns: Vec<(&str, &Dts, &BTreeMap<String, String>)> = Vec::new();
    for (item, renames) in items.iter().zip(&per_item_renames) {
        if !item.linkage.exported {
            continue;
        }
        let name = item
            .linkage
            .fixed_name
            .as_deref()
            .unwrap_or_else(|| item.name.as_str());
        if let Some((class_name, op)) = parse_class_export(name) {
            let entry = classes.entry(class_name).or_default();
            match op {
                ClassExportOp::Drop => entry.has_drop = true,
                ClassExportOp::Ctor => {
                    if let Some(dts) = &item.dts {
                        entry.ctor = Some((dts, renames));
                    }
                }
                ClassExportOp::Method(method) | ClassExportOp::ConsumingMethod(method) => {
                    if let Some(dts) = &item.dts {
                        entry.methods.push((method, dts, renames));
                    }
                }
                ClassExportOp::StaticCtor(method) => {
                    if let Some(dts) = &item.dts {
                        entry.static_members.push((method, dts, renames, true));
                    }
                }
                ClassExportOp::StaticMethod(method) => {
                    if let Some(dts) = &item.dts {
                        entry.static_members.push((method, dts, renames, false));
                    }
                }
            }
            continue;
        }
        if let Some(dts) = &item.dts {
            free_fns.push((name, dts, renames));
        }
    }

    if !free_fns.is_empty() || classes.is_empty() {
        out.push('\n');
    }
    for (name, dts, renames) in free_fns {
        let params = format_params(&dts.params, renames);
        let ret = rename_ts_type_idents(&dts.ret, renames);
        push_jsdoc(&mut out, "", dts, renames, true);
        out.push_str(&format!(
            "export declare function {name}({params}): {ret};\n"
        ));
    }

    if classes.values().any(|cls| cls.has_drop) {
        out.push_str(
            "\ndeclare const __MiresDisposableBase: {\n  \
             new (...args: never[]): SymbolConstructor extends { readonly dispose: infer S extends symbol }\n    \
             ? { [K in S]: () => void }\n    \
             : {};\n\
             };\n",
        );
    }

    for (class_name, cls) in classes {
        out.push('\n');
        if cls.has_drop {
            out.push_str(&format!(
                "export declare class {class_name} extends __MiresDisposableBase {{\n"
            ));
        } else {
            out.push_str(&format!("export declare class {class_name} {{\n"));
        }
        if let Some((ctor, renames)) = cls.ctor {
            push_jsdoc(&mut out, "  ", ctor, renames, false);
            out.push_str(&format!(
                "  constructor({});\n",
                format_params(&ctor.params, renames)
            ));
        }
        for (method, dts, renames) in cls.methods {
            push_jsdoc(&mut out, "  ", dts, renames, false);
            let params = format_params(dts.params.get(1..).unwrap_or(&[]), renames);
            let ret = rename_ts_type_idents(&dts.ret, renames);
            out.push_str(&format!("  {method}({params}): {ret};\n"));
        }
        for (method, dts, renames, is_ctor) in cls.static_members {
            push_jsdoc(&mut out, "  ", dts, renames, false);
            let params = format_params(&dts.params, renames);
            let ret = if is_ctor {
                class_name.to_owned()
            } else {
                rename_ts_type_idents(&dts.ret, renames)
            };
            out.push_str(&format!("  static {method}({params}): {ret};\n"));
        }
        if cls.has_drop {
            out.push_str("  free(): void;\n");
        }
        out.push_str("}\n");
    }
    out
}

/// The names an `import { A as B, C } from "..."` statement binds.
///
/// The text is the backend's own, printed by `jsast`, so this reads exactly the one shape it
/// prints. The LOCAL name is what is reported, because that is the binding the emitted program
/// uses and the one a reader is matching against the code.
fn imported_names(js: &str) -> Vec<String> {
    let Some(open) = js.find('{') else {
        return Vec::new();
    };
    let Some(close) = js[open..].find('}') else {
        return Vec::new();
    };
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
            imported_names("import { getOwner as _$getOwner } from \"./topcoat-dom.js\";\n"),
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

    #[test]
    fn module_dts_formats_single_line_and_throws_jsdoc() {
        use crate::item::{ItemKind, JsName, LinkItem, Linkage};

        let make_item = |name: &str, dts: Dts| LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            js: String::new(),
            refs: Default::default(),
            zombies: Vec::new(),
            linkage: Linkage::exported(name.to_owned()),
            debug_path: name.to_owned(),
            crate_name: "demo".to_owned(),
            order: None,
            map: Default::default(),
            dts: Some(dts),
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
        };

        let plain = make_item(
            "greet",
            Dts {
                params: vec![("name".to_owned(), "string".to_owned())],
                ret: "string".to_owned(),
                types: Vec::new(),
                rust: "fn(&str) -> String".to_owned(),
                doc: None,
            },
        );
        let throwing = make_item(
            "parse",
            Dts {
                params: vec![("input".to_owned(), "string".to_owned())],
                ret: "string".to_owned(),
                types: vec![("ParseOptions".to_owned(), "{ gfm?: boolean }".to_owned())],
                rust: "fn(&str) -> Result<String, String>\n@throws {string}".to_owned(),
                doc: None,
            },
        );
        let documented = make_item(
            "render",
            Dts {
                params: vec![("src".to_owned(), "string".to_owned())],
                ret: "string".to_owned(),
                types: Vec::new(),
                rust: "fn(&str) -> Result<String, String>\n@throws {string}".to_owned(),
                doc: Some("Renders markdown to HTML.\nSupports /* block */ notes.".to_owned()),
            },
        );
        let text = module_dts("demo", &[&plain, &throwing, &documented]);
        assert!(text.contains("type ParseOptions = { gfm?: boolean };\n"));
        assert!(text.contains(
            "/** `fn(&str) -> String` */\nexport declare function greet(name: string): string;\n"
        ));
        assert!(text.contains("/**\n * `fn(&str) -> Result<String, String>`\n * @throws {string}\n */\nexport declare function parse(input: string): string;\n"));
        assert!(
            text.contains(
                "/**\n * Renders markdown to HTML.\n * Supports /* block *\\/ notes.\n *\n * `fn(&str) -> Result<String, String>`\n * @throws {string}\n */\nexport declare function render(src: string): string;\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn module_dts_exports_named_types_and_deduplicates_preferring_export() {
        use crate::item::{ItemKind, JsName, LinkItem, Linkage};

        let make_item = |name: &str, dts: Dts| LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            js: String::new(),
            refs: Default::default(),
            zombies: Vec::new(),
            linkage: Linkage::exported(name.to_owned()),
            debug_path: name.to_owned(),
            crate_name: "demo".to_owned(),
            order: None,
            map: Default::default(),
            dts: Some(dts),
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
        };

        let raw_item = make_item(
            "raw_fn",
            Dts {
                params: vec![("o".to_owned(), "SharedOpts".to_owned())],
                ret: "InternalOnly".to_owned(),
                types: vec![
                    ("SharedOpts".to_owned(), "{ flag?: boolean }".to_owned()),
                    ("InternalOnly".to_owned(), "{ x: number }".to_owned()),
                ],
                rust: "fn(SharedOpts) -> InternalOnly".to_owned(),
                doc: None,
            },
        );
        let export_item = make_item(
            "render",
            Dts {
                params: vec![("o".to_owned(), "SharedOpts".to_owned())],
                ret: "RenderResult".to_owned(),
                types: vec![
                    (
                        "export SharedOpts".to_owned(),
                        "{ flag?: boolean }".to_owned(),
                    ),
                    (
                        "export RenderResult".to_owned(),
                        "{ html: string }".to_owned(),
                    ),
                ],
                rust: "fn(SharedOpts) -> RenderResult".to_owned(),
                doc: None,
            },
        );

        let text = module_dts("demo", &[&raw_item, &export_item]);
        assert!(
            text.contains("type InternalOnly = { x: number };\n"),
            "{text}"
        );
        assert!(
            text.contains("export type RenderResult = { html: string };\n"),
            "{text}"
        );
        assert!(
            text.contains("export type SharedOpts = { flag?: boolean };\n"),
            "{text}"
        );
        assert!(
            !text.contains("\ntype SharedOpts ="),
            "duplicate bare type must be upgraded to export type:\n{text}"
        );
    }

    #[test]
    fn module_dts_disambiguates_colliding_short_names_across_functions() {
        use crate::item::{ItemKind, JsName, LinkItem, Linkage};

        let make_item = |name: &str, dts: Dts| LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            js: String::new(),
            refs: Default::default(),
            zombies: Vec::new(),
            linkage: Linkage::exported(name.to_owned()),
            debug_path: name.to_owned(),
            crate_name: "demo".to_owned(),
            order: None,
            map: Default::default(),
            dts: Some(dts),
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
        };

        let fn_a = make_item(
            "get_a",
            Dts {
                params: Vec::new(),
                ret: "Outer".to_owned(),
                types: vec![
                    ("export Item".to_owned(), "{ a: number }".to_owned()),
                    (
                        "export Outer".to_owned(),
                        "{ TAG: \"Item\"; Item?: Item }".to_owned(),
                    ),
                ],
                rust: "fn() -> mod_a::Outer".to_owned(),
                doc: None,
            },
        );
        let fn_b = make_item(
            "get_b",
            Dts {
                params: vec![("item".to_owned(), "Item".to_owned())],
                ret: "Outer".to_owned(),
                types: vec![
                    ("export Item".to_owned(), "{ b: string }".to_owned()),
                    (
                        "export Outer".to_owned(),
                        "{ TAG: \"Item\"; Item?: Item }".to_owned(),
                    ),
                ],
                rust: "fn(mod_b::Item) -> Result<mod_b::Outer, mod_b::Item>\n@throws {Item}"
                    .to_owned(),
                doc: None,
            },
        );
        let fn_b2 = make_item(
            "get_b_again",
            Dts {
                params: Vec::new(),
                ret: "Item".to_owned(),
                types: vec![("export Item".to_owned(), "{ b: string }".to_owned())],
                rust: "fn() -> mod_b::Item".to_owned(),
                doc: None,
            },
        );

        let text = module_dts("demo", &[&fn_a, &fn_b, &fn_b2]);
        assert!(
            text.contains("export type Item = { a: number };\n"),
            "{text}"
        );
        assert!(
            text.contains("export type Item_2 = { b: string };\n"),
            "{text}"
        );
        assert!(
            text.contains("export type Outer = { TAG: \"Item\"; Item?: Item };\n"),
            "{text}"
        );
        assert!(
            text.contains("export type Outer_2 = { TAG: \"Item\"; Item?: Item_2 };\n"),
            "property key and string tag must stay untouched while field type renames:\n{text}"
        );
        assert!(
            text.contains("export declare function get_a(): Outer;\n"),
            "{text}"
        );
        assert!(
            text.contains(
                "* @throws {Item_2}\n */\nexport declare function get_b(item: Item_2): Outer_2;\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("export declare function get_b_again(): Item_2;\n"),
            "third function sharing mod_b::Item should reuse Item_2:\n{text}"
        );
        assert!(!text.contains("Item_3"), "{text}");
    }

    #[test]
    fn module_dts_synthesizes_export_declare_class_and_skips_raw_helpers() {
        use crate::item::{ItemKind, JsName, LinkItem, Linkage};

        let make_item = |name: &str, dts: Option<Dts>| LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            js: String::new(),
            refs: Default::default(),
            zombies: Vec::new(),
            linkage: Linkage::exported(name.to_owned()),
            debug_path: name.to_owned(),
            crate_name: "demo".to_owned(),
            order: None,
            map: Default::default(),
            dts,
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
        };

        let drop_fn = make_item("__mires_cls$Counter$__drop", None);
        let ctor_fn = make_item(
            "__mires_cls$Counter$new",
            Some(Dts {
                params: vec![("initial".to_owned(), "number".to_owned())],
                ret: "unknown".to_owned(),
                types: vec![(
                    "export CounterOpts".to_owned(),
                    "{ step?: number }".to_owned(),
                )],
                rust: "fn(i32) -> Result<*mut Counter, String>\n@throws {string}".to_owned(),
                doc: None,
            }),
        );
        let inc_fn = make_item(
            "__mires_cls$Counter$increment",
            Some(Dts {
                params: vec![
                    ("__mires_self".to_owned(), "unknown".to_owned()),
                    ("by?".to_owned(), "number".to_owned()),
                ],
                ret: "number".to_owned(),
                types: Vec::new(),
                rust: "fn(*mut Counter, Option<i32>) -> i32".to_owned(),
                doc: None,
            }),
        );
        let take_fn = make_item(
            "__mires_cls$Counter$__take$intoValue",
            Some(Dts {
                params: vec![("__mires_self".to_owned(), "unknown".to_owned())],
                ret: "number".to_owned(),
                types: Vec::new(),
                rust: "fn(*mut Counter) -> i32".to_owned(),
                doc: None,
            }),
        );
        let static_ctor = make_item(
            "__mires_cls$Counter$__static_ctor$fromZero",
            Some(Dts {
                params: Vec::new(),
                ret: "unknown".to_owned(),
                types: Vec::new(),
                rust: "fn() -> *mut Counter".to_owned(),
                doc: None,
            }),
        );
        let static_fn = make_item(
            "__mires_cls$Counter$__static$version",
            Some(Dts {
                params: Vec::new(),
                ret: "string".to_owned(),
                types: Vec::new(),
                rust: "fn() -> String".to_owned(),
                doc: None,
            }),
        );
        let free_fn = make_item(
            "ping",
            Some(Dts {
                params: Vec::new(),
                ret: "Promise<void>".to_owned(),
                types: Vec::new(),
                rust: "fn() -> TypedPromise<()>".to_owned(),
                doc: None,
            }),
        );

        let text = module_dts(
            "demo",
            &[
                &drop_fn,
                &ctor_fn,
                &inc_fn,
                &take_fn,
                &static_ctor,
                &static_fn,
                &free_fn,
            ],
        );
        assert!(
            !text.contains("__mires_cls$"),
            "raw class helpers must not appear in .d.ts:\n{text}"
        );
        assert!(
            text.contains("export type CounterOpts = { step?: number };\n"),
            "{text}"
        );
        assert!(
            text.contains("export declare function ping(): Promise<void>;\n"),
            "{text}"
        );
        let expected_class = "\
declare const __MiresDisposableBase: {
  new (...args: never[]): SymbolConstructor extends { readonly dispose: infer S extends symbol }
    ? { [K in S]: () => void }
    : {};
};

export declare class Counter extends __MiresDisposableBase {
  /**
   * `fn(i32) -> Result<*mut Counter, String>`
   * @throws {string}
   */
  constructor(initial: number);
  increment(by?: number): number;
  intoValue(): number;
  static fromZero(): Counter;
  static version(): string;
  free(): void;
}
";
        assert!(
            text.contains(expected_class),
            "expected class declaration not found:\n{text}"
        );
    }

    #[test]
    fn module_dts_skips_linker_generated_imports_and_keeps_extern_modules() {
        use crate::item::{ItemKind, JsName, LinkItem, Linkage};

        let shim = LinkItem {
            name: JsName::new("__rt".to_owned()),
            kind: ItemKind::Import,
            js: "import * as __rt from \"./shim.mjs\";\n".to_owned(),
            refs: Default::default(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: "./shim.mjs".to_owned(),
            crate_name: "generated".to_owned(),
            order: None,
            map: Default::default(),
            dts: None,
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
        };
        let chunk = LinkItem {
            name: JsName::new("$chunk$shared".to_owned()),
            kind: ItemKind::Import,
            js: "import { a } from \"./shared.js\";\n".to_owned(),
            refs: Default::default(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: "./shared.js".to_owned(),
            crate_name: "generated".to_owned(),
            order: None,
            map: Default::default(),
            dts: None,
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
        };
        let ext = LinkItem {
            name: JsName::new("ext$Chart$h1".to_owned()),
            kind: ItemKind::Import,
            js: "import { Chart as ext$Chart$h1 } from \"chart.js\";\n".to_owned(),
            refs: Default::default(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: "chart.js".to_owned(),
            crate_name: "demo".to_owned(),
            order: None,
            map: Default::default(),
            dts: None,
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
        };

        let text = module_dts("demo", &[&shim, &chunk, &ext]);
        assert!(!text.contains("./shim.mjs"), "{text}");
        assert!(!text.contains("./shared.js"), "{text}");
        assert!(text.contains("declare module \"chart.js\""), "{text}");
        assert!(
            text.contains("export const ext$Chart$h1: unknown;"),
            "{text}"
        );
    }
}
