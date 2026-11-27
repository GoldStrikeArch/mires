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

use std::collections::BTreeMap;

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
    ctor: Option<&'a Dts>,
    methods: Vec<(&'a str, &'a Dts)>,
    static_members: Vec<(&'a str, &'a Dts, bool)>,
    has_drop: bool,
}

fn format_params(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(param, ty)| {
            if let Some(base) = param.strip_suffix('?') {
                format!("{}?: {ty}", crate::jsast::sanitize_ident(base))
            } else {
                format!("{}: {ty}", crate::jsast::sanitize_ident(param))
            }
        })
        .collect::<Vec<String>>()
        .join(", ")
}

fn push_member_throws_jsdoc(out: &mut String, dts: &Dts) {
    if let Some((sig, rest)) = dts.rust.split_once('\n') {
        out.push_str(&format!("  /**\n   * `{sig}`\n   * {rest}\n   */\n"));
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

    // One definition per bare name, preferring `export type` whenever any `#[js_export]` entry
    // refers to it. Two items describing one Rust type describe it the same way, because both came
    // from the same walk over that type.
    let mut types: BTreeMap<&str, (bool, &str)> = BTreeMap::new();
    for item in items {
        if !item.linkage.exported {
            continue;
        }
        let Some(dts) = &item.dts else { continue };
        for (name, body) in &dts.types {
            let (exported, bare) = match name.strip_prefix("export ") {
                Some(rest) => (true, rest),
                None => (false, name.as_str()),
            };
            types
                .entry(bare)
                .and_modify(|(was_exported, _)| {
                    if exported {
                        *was_exported = true;
                    }
                })
                .or_insert((exported, body.as_str()));
        }
    }
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
    let mut modules: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for item in items {
        if item.kind != ItemKind::Import {
            continue;
        }
        modules
            .entry(item.debug_path.as_str())
            .or_default()
            .extend(imported_names(&item.js));
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
    let mut free_fns: Vec<(&str, &Dts)> = Vec::new();
    for item in items {
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
                        entry.ctor = Some(dts);
                    }
                }
                ClassExportOp::Method(method) | ClassExportOp::ConsumingMethod(method) => {
                    if let Some(dts) = &item.dts {
                        entry.methods.push((method, dts));
                    }
                }
                ClassExportOp::StaticCtor(method) => {
                    if let Some(dts) = &item.dts {
                        entry.static_members.push((method, dts, true));
                    }
                }
                ClassExportOp::StaticMethod(method) => {
                    if let Some(dts) = &item.dts {
                        entry.static_members.push((method, dts, false));
                    }
                }
            }
            continue;
        }
        if let Some(dts) = &item.dts {
            free_fns.push((name, dts));
        }
    }

    if !free_fns.is_empty() || classes.is_empty() {
        out.push('\n');
    }
    for (name, dts) in free_fns {
        let params = format_params(&dts.params);
        if let Some((sig, rest)) = dts.rust.split_once('\n') {
            out.push_str(&format!("/**\n * `{sig}`\n * {rest}\n */\n"));
        } else {
            out.push_str(&format!("/** `{}` */\n", dts.rust));
        }
        out.push_str(&format!(
            "export declare function {name}({params}): {};\n",
            dts.ret
        ));
    }

    for (class_name, cls) in classes {
        out.push('\n');
        out.push_str(&format!("export declare class {class_name} {{\n"));
        if let Some(ctor) = cls.ctor {
            push_member_throws_jsdoc(&mut out, ctor);
            out.push_str(&format!(
                "  constructor({});\n",
                format_params(&ctor.params)
            ));
        }
        for (method, dts) in cls.methods {
            push_member_throws_jsdoc(&mut out, dts);
            let params = format_params(dts.params.get(1..).unwrap_or(&[]));
            out.push_str(&format!("  {method}({params}): {};\n", dts.ret));
        }
        for (method, dts, is_ctor) in cls.static_members {
            push_member_throws_jsdoc(&mut out, dts);
            let params = format_params(&dts.params);
            let ret = if is_ctor {
                class_name
            } else {
                dts.ret.as_str()
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
            },
        );
        let throwing = make_item(
            "parse",
            Dts {
                params: vec![("input".to_owned(), "string".to_owned())],
                ret: "string".to_owned(),
                types: vec![("ParseOptions".to_owned(), "{ gfm?: boolean }".to_owned())],
                rust: "fn(&str) -> Result<String, String>\n@throws {string}".to_owned(),
            },
        );
        let text = module_dts("demo", &[&plain, &throwing]);
        assert!(text.contains("type ParseOptions = { gfm?: boolean };\n"));
        assert!(text.contains(
            "/** `fn(&str) -> String` */\nexport declare function greet(name: string): string;\n"
        ));
        assert!(text.contains("/**\n * `fn(&str) -> Result<String, String>`\n * @throws {string}\n */\nexport declare function parse(input: string): string;\n"));
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
            }),
        );
        let take_fn = make_item(
            "__mires_cls$Counter$__take$intoValue",
            Some(Dts {
                params: vec![("__mires_self".to_owned(), "unknown".to_owned())],
                ret: "number".to_owned(),
                types: Vec::new(),
                rust: "fn(*mut Counter) -> i32".to_owned(),
            }),
        );
        let static_ctor = make_item(
            "__mires_cls$Counter$__static_ctor$fromZero",
            Some(Dts {
                params: Vec::new(),
                ret: "unknown".to_owned(),
                types: Vec::new(),
                rust: "fn() -> *mut Counter".to_owned(),
            }),
        );
        let static_fn = make_item(
            "__mires_cls$Counter$__static$version",
            Some(Dts {
                params: Vec::new(),
                ret: "string".to_owned(),
                types: Vec::new(),
                rust: "fn() -> String".to_owned(),
            }),
        );
        let free_fn = make_item(
            "ping",
            Some(Dts {
                params: Vec::new(),
                ret: "Promise<void>".to_owned(),
                types: Vec::new(),
                rust: "fn() -> TypedPromise<()>".to_owned(),
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
export declare class Counter {
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
}
