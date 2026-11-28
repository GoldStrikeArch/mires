//! The one place that invents JavaScript identifiers.
//!
//! Every top level name the backend emits comes from a [`Namer`] method. Nothing else in the
//! backend builds an identifier out of a `DefId`, an `Instance` or a symbol name, because the
//! reachability pass in `link.rs` depends on a handful of invariants that only hold if names are
//! produced in exactly one place:
//!
//! * an item name never starts with `$` — that namespace belongs to compiler temporaries (`$v`,
//!   `$t0`, `$sel`), so a temporary can never be mistaken for a reference to an item;
//! * an item name never matches `^_[0-9]` — that is the shape of a MIR local (`_0`, `_1`);
//! * an item name is never `bb` — the trampoline's block variable;
//! * an item name is always a plain JavaScript identifier, never a reserved word.
//!
//! `refs` sets are collected mechanically from the emitted AST, so an item name that could collide
//! with a local would make a function look like it referenced something it does not.
//!
//! # Readable names
//!
//! The default ([`NameStyle::Readable`]) is a sanitized `def_path_str` plus `$h` and 16 hex digits
//! of a hash of the item's crate-independent identity ([`Namer::hash_key`]). The hash is not a
//! collision *repair* — it is unconditional, so a name depends on nothing but the item itself. A
//! per-codegen-unit disambiguation table would make the same item come out differently depending on
//! what else landed in its unit, which would break reproducible builds and cross-crate linking
//! alike.
//!
//! `#[no_mangle]` and `#[export_name]` items keep their exact name: `scripts/entry.js` calls
//! `rust_entry()`, and the demo's button calls `counter_clicked(n)`. So do
//! `#[rustc_std_internal_symbol]` items, whose symbol is the only spelling the crate that declares
//! one and the crate that defines it can both arrive at — see [`Namer::has_fixed_name`].
//!
//! [`NameStyle::Mangled`] (`-Cllvm-args=js-names=mangled`) restores the spike's raw v0 symbols,
//! which is useful for telling a naming bug apart from a lowering bug.

use rustc_abi::FieldIdx;
use rustc_data_structures::stable_hash::{StableHash, StableHasher};
use rustc_hashes::Hash64;
use rustc_hir::def::DefKind;
use rustc_hir::def_id::DefId;
use rustc_middle::middle::codegen_fn_attrs::CodegenFnAttrFlags;
use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_middle::ty::{self, ExistentialTraitRef, Instance, Ty, TyCtxt};

use crate::item::{JsName, Linkage};
use crate::jsast;
use crate::opts::NameStyle;

/// The separator a `::` becomes in a readable name.
///
/// `$` cannot appear in a Rust identifier, so `a$b` is unambiguous, and reading `foo$bar$baz` as
/// `foo::bar::baz` takes no effort.
const PATH_SEP: char = '$';

/// The marker introducing the hash suffix of a readable name.
const HASH_SEP: &str = "$h";

/// The number of hex digits [`readable`] appends after [`HASH_SEP`].
const HASH_DIGITS: usize = 16;

pub(crate) struct Namer<'tcx> {
    tcx: TyCtxt<'tcx>,
    style: NameStyle,
}

impl<'tcx> Namer<'tcx> {
    pub(crate) fn new(tcx: TyCtxt<'tcx>, style: NameStyle) -> Namer<'tcx> {
        Namer { tcx, style }
    }

    /// The name of a monomorphized function.
    pub(crate) fn fn_name(&self, instance: Instance<'tcx>) -> JsName {
        let def_id = instance.def_id();
        let symbol = self.tcx.symbol_name(instance).name;
        if let Some(exact) = self.exact_name(def_id, symbol) {
            return exact;
        }
        match self.style {
            NameStyle::Mangled => {
                debug_assert!(
                    jsast::is_plain_ident(symbol),
                    "`{symbol}` is not a JS identifier"
                );
                JsName::new(symbol.to_owned())
            }
            NameStyle::Readable => {
                JsName::new(readable(&self.stem(def_id), &self.hash_key(instance)))
            }
        }
    }

    /// The key a readable function name's hash is taken over: the instance's **stable hash**.
    ///
    /// Deliberately not `tcx.symbol_name`, and the difference is the whole point. rustc appends the
    /// *instantiating crate* to the symbol of an instance it considers cross-crate, so that two
    /// crates which both codegen `next_code_point::<slice::Iter<u8>>` do not collide at the linker.
    /// Here that is exactly backwards: two crates codegenning one instantiation have to spell it
    /// *identically*, or `link.rs` sees two different definitions of one item. Worse, whether the
    /// suffix is there at all depends on the instantiation mode, so the crate the item is defined
    /// in spells it one way and every other crate spells it another.
    ///
    /// A stable hash has neither problem. It is the identity rustc's own cross-crate machinery uses
    /// -- a `DefId` hashes as its crate-independent `DefPathHash`, and a type hashes structurally
    /// through those -- so it is the same number in every crate and in every compilation.
    ///
    /// It matters once a program has two dependencies that instantiate the same `#[inline]` item.
    /// `core` and `alloc` are that pair.
    fn hash_key(&self, instance: Instance<'tcx>) -> String {
        let hash = self.tcx.with_stable_hashing_context(|mut hcx| {
            let mut hasher = StableHasher::new();
            instance.stable_hash(&mut hcx, &mut hasher);
            hasher.finish::<Hash64>()
        });
        format!("{:016x}", hash.as_u64())
    }

    /// The name of a `static` item.
    pub(crate) fn static_name(&self, def_id: DefId) -> JsName {
        let symbol = self.tcx.symbol_name(Instance::mono(self.tcx, def_id)).name;
        if let Some(exact) = self.exact_name(def_id, symbol) {
            return exact;
        }
        self.derived(def_id, symbol)
    }

    /// The name of the interned vtable for `ty as dyn Trait`.
    ///
    /// Vtables have no `DefId` of their own, so the name is built out of the two types it is for.
    /// They are spelled by [`Namer::stable_ty`] rather than printed, because a printed path is
    /// relative to the crate printing it and two crates that intern one vtable have to name it
    /// identically -- see [`Namer::hash_key`] for the same rule on functions. The hash covers both
    /// types, which is what makes the name unique; the readable half is a hint.
    pub(crate) fn vtable_name(
        &self,
        ty: Ty<'tcx>,
        trait_ref: Option<ExistentialTraitRef<'tcx>>,
    ) -> JsName {
        let normalized = self
            .tcx
            .normalize_erasing_regions(crate::value::typing_env(), ty::Unnormalized::new_wip(ty));
        let type_hash = self.tcx.type_id_hash(normalized).as_u128();
        let ty = self.stable_ty(normalized);
        let (path, key) = match trait_ref {
            Some(trait_ref) => {
                let trait_ = self.stable_trait(trait_ref);
                (
                    format!("vtable::{ty}::{trait_}"),
                    format!("vtable\u{1}{type_hash:032x}\u{1}{ty}\u{1}{trait_}"),
                )
            }
            None => (
                format!("vtable::{ty}"),
                format!("vtable\u{1}{type_hash:032x}\u{1}{ty}"),
            ),
        };
        match self.style {
            // Even in mangled mode a vtable has no symbol to fall back on.
            NameStyle::Readable | NameStyle::Mangled => JsName::new(readable(&path, &key)),
        }
    }

    fn stable_const(&self, c: ty::Const<'tcx>) -> String {
        if let Some(n) = c.try_to_target_usize(self.tcx) {
            return n.to_string();
        }
        with_no_trimmed_paths!(format!("{c}"))
    }

    fn stable_generic_args(&self, args: &[ty::GenericArg<'tcx>]) -> String {
        let parts: Vec<String> = args
            .iter()
            .filter_map(|arg| match arg.kind() {
                ty::GenericArgKind::Type(t) => Some(self.stable_ty(t)),
                ty::GenericArgKind::Const(c) => Some(self.stable_const(c)),
                ty::GenericArgKind::Lifetime(_) => None,
            })
            .collect();
        if parts.is_empty() {
            String::new()
        } else {
            format!("<{}>", parts.join(","))
        }
    }

    /// A spelling of `ty` that every crate in a program agrees on.
    ///
    /// `format!("{ty}")` will not do. rustc prints a path relative to the crate printing it, and it
    /// prefers a *re-export* path for a foreign item: `core` calls its own type
    /// `num::error::IntErrorKind` and everybody else calls it `core::num::IntErrorKind`. Neither is
    /// wrong, and they are not the same string, so a name built out of one would make the two
    /// crates disagree about which item they are talking about.
    ///
    /// So a type that has a `DefId` is spelled by its def path, exactly as [`Namer::stem`] spells
    /// an item, and everything else is spelled structurally out of parts that are themselves
    /// spelled this way. What is left over -- the primitives, `str`, `!`, a function pointer -- has
    /// no def path in it at all and prints the same from anywhere.
    fn stable_ty(&self, ty: Ty<'tcx>) -> String {
        match ty.kind() {
            ty::Adt(def, generics) => {
                format!(
                    "{}{}",
                    self.stem(def.did()),
                    self.stable_generic_args(generics)
                )
            }
            ty::Foreign(def_id) => self.stem(*def_id),
            ty::Closure(def_id, generics) => {
                format!(
                    "{}{}",
                    self.stem(*def_id),
                    self.stable_generic_args(generics)
                )
            }
            ty::FnDef(def_id, generics) => format!(
                "{}{}",
                self.stem(*def_id),
                self.stable_generic_args(generics.skip_binder())
            ),
            ty::Coroutine(def_id, generics) => {
                format!(
                    "{}{}",
                    self.stem(*def_id),
                    self.stable_generic_args(generics)
                )
            }
            ty::CoroutineClosure(def_id, generics) => {
                format!(
                    "{}{}",
                    self.stem(*def_id),
                    self.stable_generic_args(generics)
                )
            }
            ty::Ref(_, inner, mutability) => {
                format!("&{}{}", mutability.prefix_str(), self.stable_ty(*inner))
            }
            ty::RawPtr(inner, mutability) => {
                format!("*{} {}", mutability.ptr_str(), self.stable_ty(*inner))
            }
            ty::Slice(element) => format!("[{}]", self.stable_ty(*element)),
            ty::Array(element, len) => {
                format!("[{};{}]", self.stable_ty(*element), self.stable_const(*len))
            }
            ty::Pat(base, _) => self.stable_ty(*base),
            ty::Tuple(elements) => {
                let parts: Vec<String> = elements
                    .iter()
                    .map(|element| self.stable_ty(element))
                    .collect();
                format!("({})", parts.join(","))
            }
            ty::Dynamic(predicates, ..) => match predicates.principal() {
                Some(principal) => format!("dyn {}", self.stem(principal.skip_binder().def_id)),
                None => "dyn".to_string(),
            },
            // No def path anywhere in it: the primitives, `str`, `!`, a function pointer.
            _ => with_no_trimmed_paths!(format!("{ty}")),
        }
    }

    /// The [stable spelling](Namer::stable_ty) of the trait half of a vtable.
    fn stable_trait(&self, trait_ref: ExistentialTraitRef<'tcx>) -> String {
        format!(
            "{}{}",
            self.stem(trait_ref.def_id),
            self.stable_generic_args(trait_ref.args)
        )
    }

    /// The name of a synthetic item that has no `DefId` and no type to describe it.
    pub(crate) fn synthetic_name(&self, prefix: &str, key: &str) -> JsName {
        JsName::new(readable(prefix, key))
    }

    /// The property name of one field of a struct or enum variant.
    ///
    /// Numeric field names (tuple structs, tuple variants) become `_0`, `_1`, ... so that they
    /// print as plain identifiers and match the contract's enum shape.
    ///
    /// A field literally named `TAG` gets a trailing underscore, because that is the key an enum
    /// object carries its variant name under (`value::TAG`). A struct variant with such a field
    /// would otherwise overwrite the tag with its own value, which is a silent wrong answer rather
    /// than a rejection. Structs are escaped the same way so that one field name has one spelling
    /// wherever it appears.
    pub(crate) fn field_name(variant: &ty::VariantDef, field: FieldIdx) -> String {
        let name = variant.fields[field].name;
        let name = name.as_str();
        if name.starts_with(|c: char| c.is_ascii_digit()) {
            format!("_{name}")
        } else if name == crate::value::TAG {
            format!("{name}_")
        } else {
            jsast::sanitize_ident(name)
        }
    }

    /// How an item joins the program: exported under its exact name, or internal.
    ///
    /// An exported item is also a DCE root, and is reachable by definition: something outside this
    /// program may call it. When its `link_section` declares a code-split chunk via
    /// `#[js_export(chunk = "...")]`, that target rides along in the linkage metadata.
    pub(crate) fn linkage_of(&self, instance: Instance<'tcx>) -> Linkage {
        if !self.is_exported(instance.def_id()) {
            return Linkage::internal();
        }
        let mut linkage = Linkage::exported(self.tcx.symbol_name(instance).name.to_owned());
        if let ty::InstanceKind::Item(def_id) = instance.def {
            if let Some(chunk) = self.export_chunk(def_id) {
                linkage = linkage.with_chunk(chunk.name, map_chunk_role(chunk.role));
            }
        }
        linkage
    }

    fn export_chunk(&self, def_id: DefId) -> Option<mires_marker_abi::export::ChunkSpec> {
        let section = self.tcx.codegen_fn_attrs(def_id).link_section?;
        mires_marker_abi::export::parse_export_section(section.as_str())?.chunk
    }

    /// Whether the item's JavaScript name is its symbol name, spelled exactly.
    ///
    /// Two shapes qualify, and only the first is part of the program's interface.
    ///
    /// * an **exported** item, which external JavaScript calls by that name;
    /// * a **`#[rustc_std_internal_symbol]`** item, whose symbol rustc builds out of the compiler
    ///   version alone (`mangle_internal_symbol`) rather than out of a def path. Such an item is
    ///   declared in one crate and defined in another — or, for the allocator shims, defined by the
    ///   backend with no `DefId` to derive a name from — so the symbol is the only spelling both
    ///   sides can arrive at. `rust_begin_unwind` is the case that already worked, because
    ///   `#[panic_handler]` sets an explicit symbol name; `__rust_alloc` is the case that needs
    ///   this arm.
    fn has_fixed_name(&self, def_id: DefId) -> bool {
        self.is_exported(def_id) || self.is_std_internal_symbol(def_id)
    }

    /// Whether the item carries `#[rustc_std_internal_symbol]`.
    fn is_std_internal_symbol(&self, def_id: DefId) -> bool {
        if !matches!(
            self.tcx.def_kind(def_id),
            DefKind::Fn | DefKind::AssocFn | DefKind::Static { .. }
        ) {
            return false;
        }
        self.tcx
            .codegen_fn_attrs(def_id)
            .flags
            .contains(CodegenFnAttrFlags::RUSTC_STD_INTERNAL_SYMBOL)
    }

    /// Whether the item is visible from outside the program.
    fn is_exported(&self, def_id: DefId) -> bool {
        if !matches!(
            self.tcx.def_kind(def_id),
            DefKind::Fn | DefKind::AssocFn | DefKind::Static { .. }
        ) {
            return false;
        }
        let attrs = self.tcx.codegen_fn_attrs(def_id);
        attrs.symbol_name.is_some()
            || attrs.flags.intersects(
                CodegenFnAttrFlags::NO_MANGLE
                    | CodegenFnAttrFlags::USED_COMPILER
                    | CodegenFnAttrFlags::USED_LINKER,
            )
    }

    /// The exact name of an item that [keeps its symbol](Namer::has_fixed_name), validated as a
    /// JavaScript identifier.
    fn exact_name(&self, def_id: DefId, symbol: &str) -> Option<JsName> {
        if !self.has_fixed_name(def_id) {
            return None;
        }
        if !jsast::is_plain_ident(symbol) {
            self.tcx.dcx().span_err(
                self.tcx.def_span(def_id),
                format!(
                    "`{symbol}` is not a valid JavaScript identifier, so it cannot be used as an \
                     exported name by rustc_codegen_js"
                ),
            );
            return Some(JsName::new(jsast::sanitize_ident(symbol)));
        }
        Some(JsName::new(symbol.to_owned()))
    }

    /// The name of a non-exported item, in whichever style this compilation asked for.
    fn derived(&self, def_id: DefId, symbol: &str) -> JsName {
        match self.style {
            NameStyle::Mangled => {
                // v0 and legacy symbols are already plain identifiers, and already unique.
                debug_assert!(
                    jsast::is_plain_ident(symbol),
                    "`{symbol}` is not a JS identifier"
                );
                JsName::new(symbol.to_owned())
            }
            NameStyle::Readable => JsName::new(readable(&self.stem(def_id), symbol)),
        }
    }
}

fn map_chunk_role(role: mires_marker_abi::export::ChunkRole) -> rcgjs_object::artifact::EntryRole {
    use mires_marker_abi::export::ChunkRole;
    use rcgjs_object::artifact::EntryRole;
    match role {
        ChunkRole::Main => EntryRole::Main,
        ChunkRole::Island => EntryRole::Island,
        ChunkRole::Worker => EntryRole::Worker,
        ChunkRole::Procedure => EntryRole::Procedure,
    }
}

/// The readable half of a name: the item's path, spelled the same from every crate.
///
/// `def_path_str` is deliberately *not* used here. It renders a path relative to where it is
/// being printed from — `print_i32` inside `mini_core`, `mini_core::print_i32` from a crate that
/// depends on it — so two crates would name the same item differently and the reference from one
/// would not resolve to the definition in the other. `DefPath` is structural and crate-relative,
/// so prefixing it with the crate name gives a spelling that only depends on the item.
impl<'tcx> Namer<'tcx> {
    fn stem(&self, def_id: DefId) -> String {
        let mut out = self.tcx.crate_name(def_id.krate).to_string();
        out.push_str(&self.tcx.def_path(def_id).to_string_no_crate_verbose());
        out
    }
}

/// Sanitizes a Rust path into an identifier and appends a hash of `key`.
///
/// `::` becomes `$`; everything outside `[A-Za-z0-9_]` becomes `_`, with runs collapsed. The
/// result is guaranteed to satisfy the invariants documented at the top of this module — the
/// assertions at the end are not decoration, `link.rs` is unsound without them.
fn readable(path: &str, key: &str) -> String {
    let mut out = String::with_capacity(path.len() + HASH_SEP.len() + 16);
    let mut chars = path.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == ':' && chars.peek() == Some(&':') {
            chars.next();
            push_sep(&mut out, PATH_SEP);
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            push_sep(&mut out, '_');
        }
    }
    while out.ends_with('_') || out.ends_with(PATH_SEP) {
        out.pop();
    }

    // Guards, in order: an empty stem, a leading `$` (reserved for temporaries), a leading digit,
    // and the `_0` shape of a MIR local. `x` is a boring, always-legal prefix.
    if out.is_empty() {
        out.push_str("item");
    }
    let head: Vec<char> = out.chars().take(2).collect();
    let needs_prefix = head[0] == PATH_SEP
        || head[0].is_ascii_digit()
        || (head[0] == '_' && head.get(1).is_some_and(|c| c.is_ascii_digit()));
    if needs_prefix {
        out.insert(0, 'x');
    }

    out.push_str(HASH_SEP);
    out.push_str(&format!("{:0width$x}", hash64(key), width = HASH_DIGITS));

    debug_assert!(
        jsast::is_plain_ident(&out),
        "`{out}` is not a JavaScript identifier"
    );
    debug_assert!(
        !out.starts_with(PATH_SEP),
        "`{out}` invades the temporary namespace"
    );
    debug_assert!(
        out != "bb",
        "`{out}` collides with the trampoline block variable"
    );
    debug_assert!(
        !(out.starts_with('_') && out[1..].starts_with(|c: char| c.is_ascii_digit())),
        "`{out}` looks like a MIR local"
    );
    out
}

/// The local a `#[js_extern]` import binds, stable across crates and unique per (module, name).
///
/// The module specifier is part of the hashed key, so two modules exporting the same name bind two
/// different locals, while two crates importing the same name from the same module produce the
/// same local and the same import text, which `link.rs` collapses like any other duplicate.
pub(crate) fn extern_import(module: &str, name: &str) -> String {
    // A NUL cannot appear in either half, so the key is unambiguous however they are spelled.
    readable(&format!("ext::{name}"), &format!("{module}\u{0}{name}"))
}

/// Appends a separator, collapsing a run of them into one and never leading with one.
fn push_sep(out: &mut String, sep: char) {
    if out.is_empty() || out.ends_with('_') || out.ends_with(PATH_SEP) {
        return;
    }
    out.push(sep);
}

/// FNV-1a, 64 bit.
///
/// Hand rolled rather than borrowed from `std`, because the value has to stay the same across
/// compiler versions: it ends up in every emitted identifier, and two crates linked together must
/// agree on the name of a shared item.
fn hash64(bytes: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in bytes.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_names_satisfy_the_link_invariants() {
        for path in [
            "mini_core::print_i32",
            "<i32 as mini_core::Add>::add",
            "0weird",
            "_0",
            "bb",
            "",
            "::",
            "a::::b",
            "let",
        ] {
            let name = readable(path, path);
            assert!(jsast::is_plain_ident(&name), "{path} -> {name}");
            assert!(!name.starts_with('$'), "{path} -> {name}");
            assert_ne!(name, "bb");
            assert!(
                !(name.starts_with('_') && name[1..].starts_with(|c: char| c.is_ascii_digit()))
            );
        }
    }

    #[test]
    fn readable_names_are_deterministic_and_distinct() {
        assert_eq!(readable("a::b", "sym"), readable("a::b", "sym"));
        assert_ne!(readable("a::b", "sym1"), readable("a::b", "sym2"));
        assert_eq!(readable("a::b", "k"), format!("a$b$h{:016x}", hash64("k")));
    }
}
