//! Whole-program linking: reachability, zombie reporting and emission.
//!
//! # What "linking" means here
//!
//! There is no linker. A program is the set of JavaScript items reachable from its roots, printed
//! in an order that respects module level initialization. So the link step is a graph walk:
//!
//! 1. collect every item from this crate's compiled modules and from each linked rlib;
//! 2. deduplicate — the same generic instantiation may be codegenned in several crates, which is
//!    fine as long as the two spellings agree, and an outright bug if they do not;
//! 3. mark the roots: the `#[no_mangle]`/`#[export_name]`/`#[used]` items **of the crate being
//!    linked** and any `--js-root=NAME`;
//! 4. breadth-first search over `refs` **intersected with the item table**, remembering which item
//!    first reached each one;
//! 5. report the zombies in reachable items only, each with the chain that made it reachable;
//! 6. print the reachable items, module level bindings first.
//!
//! Step 4 is where the naming invariants earn their keep: `refs` is every identifier the item's
//! JavaScript mentions, `Object` and `_3` and `bb` included, and the intersection is what turns
//! that into a call graph. See `naming.rs`.
//!
//! # Two entry points
//!
//! [`link`] is the real one, used for `--crate-type bin`/`cdylib`. Rlibs delegate to rustc's own
//! `link_binary`, which packs the objects into an archive for a later link to read back.
//!
//! [`prune_objects`] handles the single-crate `--emit=obj` path that `scripts/compile.sh` uses,
//! which never reaches [`link`] at all. It runs the same graph walk over the objects that were
//! just written and rewrites them in place, so a plain `rustc --emit=obj -o out.js` gets the same
//! dead-code elimination and the same zombie reporting as a real link.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use rustc_codegen_ssa::back::archive::ArArchiveBuilderBuilder;
use rustc_codegen_ssa::back::link::{each_linked_rlib, link_binary};
use rustc_codegen_ssa::diagnostics::LinkRlibError;
use rustc_codegen_ssa::{CompiledModules, CrateInfo};
use rustc_errors::DiagCtxtHandle;
use rustc_metadata::EncodedMetadata;
use rustc_session::Session;
use rustc_session::config::{CrateType, OutFileName, OutputFilenames, OutputType};

use crate::driver::JsModule;
use crate::item::{ItemKind, ItemMap, JsName, Linkage, LinkItem, read_object, write_program};
use crate::jsast;
use crate::opts::Modules;

/// The name the runtime shim is bound to. Emitted calls are `__rt.<symbol>(...)`, in either mode.
const SHIM_NAMESPACE: &str = "__rt";

/// The `CodegenBackend::link` hook.
pub(crate) fn link(
    sess: &Session,
    compiled_modules: CompiledModules,
    crate_info: CrateInfo,
    metadata: EncodedMetadata,
    outputs: &OutputFilenames,
) {
    // An rlib is not a program: it is a bag of objects for a later link to pick from, and rustc
    // already knows how to build one. Deleting unreachable items here would delete exactly the
    // items the downstream crate is about to ask for.
    if !sess.opts.crate_types.iter().any(is_program) {
        link_binary(
            sess,
            &ArArchiveBuilderBuilder,
            compiled_modules,
            crate_info,
            metadata,
            outputs,
            "js",
        );
        return;
    }

    let mut items = Vec::new();
    for module in &compiled_modules.modules {
        if let Some(path) = &module.object {
            read_into(sess.dcx(), path, &mut items);
        }
    }
    if let Some(module) = &compiled_modules.allocator_module {
        if let Some(path) = &module.object {
            read_into(sess.dcx(), path, &mut items);
        }
    }

    // Everything this crate depends on, in the archives rustc built for them.
    //
    // A dependency's exports are *not* this program's roots. That is how a static archive behaves
    // everywhere else — a member is pulled in when something needs it, not because it defines a
    // public symbol — and here it is the difference between a hello-world and all of `core`:
    // `compiler_builtins` marks a few hundred items `#[no_mangle]`, and rooting them would drag in
    // `libm`, `fmt` and the whole panic machinery for a program that never calls any of it.
    let crate_type = sess.opts.crate_types.iter().copied().find(is_program);
    if let Err(err) = each_linked_rlib(&crate_info, crate_type, &mut |_, path| {
        let first = items.len();
        read_archive_into(sess.dcx(), path, &mut items);
        for item in &mut items[first..] {
            item.linkage.demote();
        }
    }) {
        // The error type is not `Debug`; its variants all mean the same thing here, which is
        // that a dependency has no rlib to read items out of.
        let reason = match err {
            LinkRlibError::MissingFormat => "no dependency format was computed",
            LinkRlibError::IncompatibleDependencyFormats { .. } => {
                "the crate types disagree about how dependencies are linked"
            }
            LinkRlibError::NotFound { .. } => "a dependency's rlib is missing",
            LinkRlibError::OnlyRmetaFound { .. } => "a dependency has metadata but no rlib",
        };
        sess.dcx().err(format!("rustc_codegen_mires cannot read the linked rlibs: {reason}"));
    }

    let path = outputs.path(OutputType::Exe);
    match &path {
        OutFileName::Real(path) => {
            let program = resolve(sess, items, "program", Some(path.as_path()));
            if let Err(err) = fs::write(path, program.as_bytes()) {
                sess.dcx().err(format!("error writing `{}`: {err}", path.display()));
            }
        }
        // Nothing to hang a `.map` off: a program on stdout has no path to name one after.
        OutFileName::Stdout => {
            let program = resolve(sess, items, "program", None);
            print!("{program}");
        }
    }
}

/// Whether a crate type produces a finished program rather than an intermediate archive.
fn is_program(crate_type: &CrateType) -> bool {
    matches!(crate_type, CrateType::Executable | CrateType::Cdylib | CrateType::Dylib)
}

/// Prunes the objects a single-crate `--emit=obj` run just wrote.
///
/// The whole crate's items are gathered, the graph walk runs once, and the result replaces the
/// first object's contents; any further object is emptied, because rustc only ever copies one of
/// them onto the `-o` path.
///
/// The final `-o` path comes first in the candidate list on purpose: `join()` has already run
/// `produce_final_output_artifacts`, which *renames* a single codegen unit's object onto it, so
/// the temporary path the module still points at is usually gone by now.
pub(crate) fn prune_objects(sess: &Session, outputs: &OutputFilenames, modules: &CompiledModules) {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let OutFileName::Real(path) = outputs.path(OutputType::Object) {
        candidates.push(path);
    }
    for module in modules.modules.iter().chain(&modules.allocator_module) {
        if let Some(path) = &module.object {
            if !candidates.contains(path) {
                candidates.push(path.clone());
            }
        }
    }
    candidates.retain(|path| path.exists());

    let Some((first, rest)) = candidates.split_first() else {
        return;
    };

    let mut items = Vec::new();
    for path in &candidates {
        read_into(sess.dcx(), path, &mut items);
    }

    let program = resolve(sess, items, "program", Some(first.as_path()));
    if let Err(err) = fs::write(first, program.as_bytes()) {
        sess.dcx().err(format!("error writing `{}`: {err}", first.display()));
    }
    for path in rest {
        let _ = fs::write(path, JsModule::prologue("(empty)").as_bytes());
    }
}

/// Whether this compilation writes a standalone object that nothing will link afterwards.
///
/// `--emit=obj` alone is `scripts/compile.sh`'s mode: rustc copies the single codegen unit's
/// object onto `-o` and stops. `--emit=link` (the default) means an rlib or a program is coming,
/// and pruning has to wait for [`link`], which can see the whole program.
pub(crate) fn is_standalone_object(sess: &Session) -> bool {
    let types = &sess.opts.output_types;
    types.contains_key(&OutputType::Object) && !types.contains_key(&OutputType::Exe)
}

// -------------------------------------------------------------------------------------------
// Reading
// -------------------------------------------------------------------------------------------

fn read_into(dcx: DiagCtxtHandle<'_>, path: &Path, out: &mut Vec<LinkItem>) {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            dcx.err(format!("error reading `{}`: {err}", path.display()));
            return;
        }
    };
    match read_object(&text) {
        Ok(Some(items)) => out.extend(items),
        // No footer: not one of ours, or already a finished program. Nothing to link.
        Ok(None) => {}
        Err(err) => {
            dcx.err(format!("`{}`: {err}", path.display()));
        }
    }
}

/// Reads every `rcgjs` object out of an `ar` archive.
fn read_archive_into(dcx: DiagCtxtHandle<'_>, path: &Path, out: &mut Vec<LinkItem>) {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            dcx.err(format!("error reading `{}`: {err}", path.display()));
            return;
        }
    };
    let archive = match object::read::archive::ArchiveFile::parse(&*bytes) {
        Ok(archive) => archive,
        Err(err) => {
            dcx.err(format!("`{}` is not a readable archive: {err}", path.display()));
            return;
        }
    };
    for member in archive.members() {
        let Ok(member) = member else { continue };
        let Ok(data) = member.data(&*bytes) else { continue };
        // Metadata and anything else that is not UTF-8 is not one of our objects.
        let Ok(text) = std::str::from_utf8(data) else { continue };
        match read_object(text) {
            Ok(Some(items)) => out.extend(items),
            Ok(None) => {}
            Err(err) => {
                dcx.err(format!(
                    "`{}`, member `{}`: {err}",
                    path.display(),
                    String::from_utf8_lossy(member.name())
                ));
            }
        }
    }
}

// -------------------------------------------------------------------------------------------
// The graph walk
// -------------------------------------------------------------------------------------------

/// Deduplicates, walks, reports and prints. The whole link step, once the items are in hand.
///
/// `path` is where the program is about to be written, which is what a source map is named after
/// and what the `sourceMappingURL` comment points at. Writing the `.map` happens here rather than
/// at the call site because this is the only place that holds both the program text and the line
/// each item landed on.
fn resolve(sess: &Session, mut items: Vec<LinkItem>, label: &str, path: Option<&Path>) -> String {
    let opts = crate::opts::get();
    if opts.modules == Modules::Esm {
        items.push(shim_import());
    }

    let table = dedup(sess, items);
    let split = !opts.chunks.is_empty();

    let (reachable, parents) = reach(sess, &table);
    // With chunks the same zombie would otherwise be reported twice, once here and once for the
    // chunk that reaches it. The per-chunk report is the more useful of the two: its chain names
    // the entry point that pulled the item in.
    if !split {
        report_zombies(sess, &table, &reachable, &parents, None, &mut BTreeSet::new());
    }

    let stubs = stub_missing(&table, &reachable);
    let mut ordered = order(&table, &reachable);
    ordered.extend(stubs.iter());
    let unit = Unit {
        label: label.to_owned(),
        path: path.map(Path::to_path_buf),
        items: ordered,
        exports: BTreeSet::new(),
    };
    let mut printed = print_units(sess, std::slice::from_ref(&unit));

    if split {
        match path {
            Some(path) => write_chunks(sess, &table, path),
            None => {
                sess.dcx().err(
                    "`-Cllvm-args=js-chunk` needs an output path to write the chunk files beside",
                );
            }
        }
    }

    printed.pop().unwrap_or_default()
}

// -------------------------------------------------------------------------------------------
// Printing
// -------------------------------------------------------------------------------------------

/// One output file: the items it holds, the extra names it must export, and where it goes.
struct Unit<'a> {
    /// What the file's header comment calls it.
    label: String,
    /// Where the file is written, which is what its source map is named after. `None` for a
    /// program printed to stdout, which has no path to hang a `.map` off.
    path: Option<PathBuf>,
    /// The items in print order: imports, then module level bindings, then the rest.
    items: Vec<&'a LinkItem>,
    /// Names the export clause must carry beyond the items' own linkage, because another unit
    /// imports them from this one.
    exports: BTreeSet<&'a str>,
}

/// Prints each unit, sharing one item rename across all of them.
///
/// The rename is shared because a name that crosses a file boundary is spelled in two places: the
/// definition in the unit that owns it, and the `import` clause of every unit that reaches it. One
/// map computed over the whole set is what keeps those spellings equal; computing a map per file
/// would rename the definition and the import to two different short names.
///
/// A single unit is the ordinary case, and then the corpus is that one program, so an unsplit
/// program is renamed exactly as it was before chunks existed.
fn print_units(sess: &Session, units: &[Unit<'_>]) -> Vec<String> {
    let opts = crate::opts::get();
    let bodies: Vec<(String, Vec<u32>)> = units
        .iter()
        .map(|unit| write_program(&JsModule::prologue(&unit.label), &unit.items))
        .collect();

    // The whole-program half of minification: an item name can only be shortened once every
    // reference to it is in one place, which is here and nowhere earlier. See `minify.rs`.
    let renames = match opts.minify && opts.minify_items {
        false => BTreeMap::new(),
        true => {
            let items: Vec<&LinkItem> =
                units.iter().flat_map(|unit| unit.items.iter().copied()).collect();
            warn_short_fixed_names(sess, &items);
            let corpus: String = bodies.iter().map(|(text, _)| text.as_str()).collect();
            crate::minify::item_renames(&corpus, &items)
        }
    };

    let mut printed = Vec::with_capacity(units.len());
    for (unit, (body, starts)) in units.iter().zip(&bodies) {
        let mut text = crate::minify::apply_renames(body, &renames);
        // After the rename, so that the clause is written once, out of the names that survived it,
        // and never scanned as if it were an item's code.
        text.push_str(&export_clause(unit, &renames));

        if let (true, Some(path)) = (opts.source_map, &unit.path) {
            let comment = write_source_map(sess, path, &text, &unit.items, starts);
            text.push_str(&comment);
        }
        // The `.d.ts` is written here because this is where a unit's surviving items and its
        // output path are both known, which is exactly what the file describes. It is synthesized
        // rather than emitted: nothing about `text` changes, which is why `js-dts` is not an
        // emit-affecting option.
        if let (true, Some(path)) = (opts.dts, &unit.path) {
            let path = path.with_extension("d.ts");
            let contents = crate::dts::module_dts(&unit.label, &unit.items);
            if let Err(err) = fs::write(&path, contents.as_bytes()) {
                sess.dcx().err(format!("error writing `{}`: {err}", path.display()));
            }
        }
        printed.push(text);
    }
    printed
}

/// The `import * as __rt from "..."` an ES module program calls the runtime shim through.
///
/// Synthesized here rather than by codegen, because it belongs to the program and not to any one
/// item: an item's own JavaScript is byte for byte the same in both module modes, which is what
/// keeps `js-modules` out of the options every object in a program must agree on.
///
/// It is deliberately not a root. `refs` already reports `__rt` for every `__rt.foo(...)` call in
/// the program — [`crate::jsast::visit_idents`] walks the object of a member expression — so the
/// reachability pass keeps this item exactly when something uses the shim, and drops it when
/// nothing does.
fn shim_import() -> LinkItem {
    let source = crate::opts::get().shim_module.clone();
    let decl = jsast::import_namespace(SHIM_NAMESPACE, source.clone());
    let style = match crate::opts::get().minify {
        true => jsast::Style::Compact,
        false => jsast::Style::Readable,
    };
    LinkItem {
        name: JsName::new(SHIM_NAMESPACE.to_owned()),
        kind: ItemKind::Import,
        js: jsast::program_to_string_styled(std::slice::from_ref(&decl), style),
        refs: Default::default(),
        zombies: Vec::new(),
        linkage: Linkage {
            fixed_name: Some(SHIM_NAMESPACE.to_owned()),
            root: false,
            exported: false,
        },
        debug_path: source,
        order: None,
        map: ItemMap::default(),
        dts: None,
    }
}

/// The trailing `export { .. };` naming the items the module exposes. Empty in script mode.
///
/// One clause at the end rather than an `export` keyword on each declaration, for the same reason
/// the import is synthesized here: an item's text must not depend on the module mode.
///
/// An item the program exposes to its host keeps its exact name, so minification never reaches it.
/// A name another chunk imports is a different case: it is internal to the program and gets
/// shortened like any other, so the clause is written out of `renames` rather than out of the
/// items, which is what makes the two ends of a cross-chunk import agree.
fn export_clause(unit: &Unit<'_>, renames: &BTreeMap<String, String>) -> String {
    let opts = crate::opts::get();
    if opts.modules != Modules::Esm {
        return String::new();
    }
    let mut names: BTreeSet<&str> = unit
        .items
        .iter()
        .filter(|item| item.linkage.exported)
        .map(|item| item.name.as_str())
        .collect();
    names.extend(unit.exports.iter().copied());
    if names.is_empty() {
        return String::new();
    }
    let names: Vec<&str> =
        names.into_iter().map(|name| renames.get(name).map_or(name, String::as_str)).collect();
    match opts.minify {
        true => format!("export{{{}}};\n", names.join(",")),
        false => format!("export {{ {} }};\n", names.join(", ")),
    }
}

/// Warns about a fixed name a minified local could shadow.
///
/// A fixed name keeps its exact spelling, so it is the one kind of name in a minified program that
/// the local renamer cannot see and therefore cannot avoid. It only avoids the identifiers a function
/// *mentions*: a function that calls `counter_clicked` will never name a local that, but a function
/// that does not call it could — if the name were short enough for the allocator to reach, which
/// for anything past three characters it never is (that needs more than 3445 distinct locals in one
/// function). So the hazard is real only for a very short `#[no_mangle]` name, and saying so is
/// better than silently narrowing what the flag may be used on. `names.rs` documents the same
/// hazard for readable local names, which have had it all along.
fn warn_short_fixed_names(sess: &Session, items: &[&LinkItem]) {
    let mut warned: BTreeSet<&str> = BTreeSet::new();
    for item in items {
        let Some(name) = &item.linkage.fixed_name else { continue };
        if name.len() <= 3 && !name.starts_with('$') && warned.insert(name.as_str()) {
            sess.dcx().warn(format!(
                "`{name}` keeps its exact name and is short enough that \
                 `-Cllvm-args=js-minify=on` could give a local the same one; rename it or drop \
                 the minification"
            ));
        }
    }
}

/// Writes the `.map` beside the program and returns the comment that points at it.
///
/// The comment goes last, as the format requires, and a finished program carries no `rcgjs` footer
/// for it to collide with — the footer belongs to object files, which never get a map because
/// nothing steps through an intermediate.
fn write_source_map(
    sess: &Session,
    path: &Path,
    program: &str,
    items: &[&LinkItem],
    starts: &[u32],
) -> String {
    let file = path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let map_name = format!("{file}.map");
    let map_path = path.with_file_name(&map_name);

    let json = crate::sourcemap::build(sess, program, items, starts, &file);
    if let Err(err) = fs::write(&map_path, json.as_bytes()) {
        sess.dcx().err(format!("error writing `{}`: {err}", map_path.display()));
        return String::new();
    }
    format!("//# sourceMappingURL={map_name}\n")
}

/// Stands in for every function a reachable item calls that no item defines.
///
/// rustc's monomorphization collector walks *mono-reachable* blocks: it folds the switch conditions
/// that are constant once the instance is known and never descends into a block the fold proves
/// dead. `core` is full of those — every `assert_unsafe_precondition!` compiles to
/// `if false { precondition_check(..) }` in a release build — so the collector deliberately does
/// not produce the callee, while this backend, which emits blocks rather than pruning them, still
/// prints the call.
///
/// The call is unreachable, so any body would do; a body that throws is the one that keeps the
/// property worth having, namely that a *reachable* call to a function the collector never produced
/// says so, loudly, at the point of the mistake instead of as a bare `ReferenceError`.
fn stub_missing(table: &BTreeMap<JsName, LinkItem>, reachable: &[JsName]) -> Vec<LinkItem> {
    let mut missing: BTreeMap<JsName, LinkItem> = BTreeMap::new();
    for name in reachable {
        let Some(item) = table.get(name) else { continue };
        for referenced in &item.refs {
            // The namer's `$h` infix is what makes an identifier certainly an item's name rather
            // than a local, a property or a JavaScript builtin. See `naming.rs`.
            if table.contains_key(referenced)
                || !referenced.as_str().contains("$h")
                || missing.contains_key(referenced)
            {
                continue;
            }
            let js = format!(
                "\nfunction {name}() {{\n  throw new Error(\"rustc_codegen_mires: `{name}` was \
                 never codegenned; the monomorphization collector proved every call to it \
                 unreachable\");\n}}\n",
                name = referenced.as_str(),
            );
            missing.insert(
                referenced.clone(),
                LinkItem {
                    name: referenced.clone(),
                    kind: ItemKind::Fn,
                    js,
                    refs: Default::default(),
                    zombies: Vec::new(),
                    linkage: Linkage::internal(),
                    debug_path: format!("stub for `{}`", referenced.as_str()),
                    order: None,
                    map: ItemMap::default(),
                    dts: None,
                },
            );
        }
    }
    missing.into_values().collect()
}

/// One entry per name. Identical duplicates are dropped; conflicting ones are an error.
fn dedup(sess: &Session, items: Vec<LinkItem>) -> BTreeMap<JsName, LinkItem> {
    let mut table: BTreeMap<JsName, LinkItem> = BTreeMap::new();
    for item in items {
        match table.get(&item.name) {
            None => {
                table.insert(item.name.clone(), item);
            }
            // The same instantiation codegenned in two crates: expected, and harmless. Only the
            // code is compared — see `LinkItem::code`.
            Some(existing) if existing.code() == item.code() => {}
            Some(existing) => {
                let mut diag = sess.dcx().struct_err(format!(
                    "rustc_codegen_mires: two different definitions of `{}` (`{}` and `{}`) \
                     reached the same program",
                    item.name, existing.debug_path, item.debug_path
                ));
                // By far the likeliest cause, and one whose message is otherwise baffling: the two
                // crates that codegenned this instantiation printed it under different backend
                // options. Every option that changes an item's *text* — `js-minify`,
                // `js-structure`, `js-queue` — has to be the same for the whole program, the
                // sysroot's `core` included.
                diag.note(
                    "every object in a program must be compiled with the same `-Cllvm-args=js-*` \
                     options; rebuild the sysroot with the ones this crate uses \
                     (`JS_EXTRA_ARGS='...' scripts/build_sysroot.sh`)",
                );
                diag.emit();
            }
        }
    }
    table
}

/// Breadth-first search from the roots. Returns the reachable names and, for each, the item that
/// first reached it.
fn reach(
    sess: &Session,
    table: &BTreeMap<JsName, LinkItem>,
) -> (Vec<JsName>, HashMap<JsName, JsName>) {
    let extra_roots = &crate::opts::get().roots;
    for root in extra_roots {
        if !table.contains_key(&JsName::new(root.clone())) {
            sess.dcx().warn(format!("`-Cllvm-args=js-root={root}` names no item in this program"));
        }
    }

    let roots: Vec<JsName> = table
        .iter()
        .filter(|(name, item)| {
            item.linkage.root || extra_roots.iter().any(|root| root == name.as_str())
        })
        .map(|(name, _)| name.clone())
        .collect();

    reach_from(table, &roots)
}

/// Breadth-first search from `roots`, which is what a chunk's contents are (see [`write_chunks`]).
fn reach_from(
    table: &BTreeMap<JsName, LinkItem>,
    roots: &[JsName],
) -> (Vec<JsName>, HashMap<JsName, JsName>) {
    let mut queue: VecDeque<JsName> = VecDeque::new();
    let mut seen: Vec<JsName> = Vec::new();
    let mut in_set: std::collections::HashSet<JsName> = std::collections::HashSet::new();
    let mut parents: HashMap<JsName, JsName> = HashMap::new();

    for root in roots {
        if table.contains_key(root) && in_set.insert(root.clone()) {
            queue.push_back(root.clone());
        }
    }

    while let Some(name) = queue.pop_front() {
        seen.push(name.clone());
        let Some(item) = table.get(&name) else { continue };
        for referenced in &item.refs {
            // The intersection: an identifier that names no item is a local, a JS builtin or a
            // property, and none of those keep anything alive. A namer-shaped name (its `$h` hash
            // infix) that no item defines is a call the monomorphization collector proved
            // unreachable and therefore never produced; `stub_missing` gives it a body that
            // throws. Either way there is nothing to walk into.
            if !table.contains_key(referenced) {
                continue;
            }
            if in_set.insert(referenced.clone()) {
                parents.insert(referenced.clone(), name.clone());
                queue.push_back(referenced.clone());
            }
        }
    }

    (seen, parents)
}

// -------------------------------------------------------------------------------------------
// Chunks
// -------------------------------------------------------------------------------------------

/// One requested chunk and what its entry point reaches.
struct ChunkPlan<'a> {
    /// The name the chunk is requested, written and served under.
    name: &'a str,
    /// The item the chunk exists to hold, which is never moved into the shared chunk.
    root: &'a JsName,
    /// Every item the entry point reaches, and which item first reached each one.
    reachable: Vec<JsName>,
    parents: HashMap<JsName, JsName>,
}

/// Writes one file per requested chunk beside `path`, plus the shared chunk when two chunks reach
/// a common item.
///
/// # How the split is decided
///
/// Reachability is already a pure walk over the item table, so a chunk is one more walk from one
/// more root. Every name reached that way gets exactly one owner:
///
/// 1. a name that is some chunk's entry point belongs to that chunk, so `<name>.js` always exports
///    the entry the caller asked it for;
/// 2. a name only one chunk reaches belongs to that chunk;
/// 3. a name two or more chunks reach belongs to the shared chunk;
/// 4. an [`ItemKind::Import`] is the exception: it is replicated into every chunk that reaches it
///    rather than owned by one. An import *declares* a binding the module has to have, so moving
///    one into the shared chunk would leave a chunk calling `__rt.foo(..)` with no `__rt`.
///
/// A chunk then imports every name its own items mention that another chunk owns, and the owner
/// exports it. The shared chunk is closed under references, because everything an item two chunks
/// reach can reach is also reached by both, so it never imports from a chunk and the module graph
/// has no cycles.
///
/// The file the compiler was asked for is written as it always was: a split is an *extra*
/// emission, which is what keeps `--emit=obj` runnable and leaves an unsplit build untouched.
fn write_chunks(sess: &Session, table: &BTreeMap<JsName, LinkItem>, path: &Path) {
    let opts = crate::opts::get();

    let mut plans: Vec<ChunkPlan<'_>> = Vec::with_capacity(opts.chunks.len());
    for (name, entry) in &opts.chunks {
        let Some(root) = entry_item(table, entry) else {
            sess.dcx().err(format!(
                "`-Cllvm-args=js-chunk={name}:{entry}` names no item in this program; the entry \
                 point of a chunk is an exported item's name, spelled exactly"
            ));
            continue;
        };
        let (reachable, parents) = reach_from(table, std::slice::from_ref(root));
        plans.push(ChunkPlan { name, root, reachable, parents });
    }
    if plans.len() != opts.chunks.len() {
        return;
    }

    // Rule 1 comes last so that an entry another chunk also reaches stays in its own chunk; the
    // chunk that reaches it then imports it, like any other name it does not own.
    let shared = plans.len();
    let mut owner: BTreeMap<&JsName, usize> = BTreeMap::new();
    for (index, plan) in plans.iter().enumerate() {
        for name in &plan.reachable {
            let Some((name, item)) = table.get_key_value(name) else { continue };
            if item.kind == ItemKind::Import {
                continue;
            }
            owner.entry(name).and_modify(|at| *at = if *at == index { index } else { shared }).or_insert(index);
        }
    }
    for (index, plan) in plans.iter().enumerate() {
        owner.insert(plan.root, index);
    }

    // Zombies once each, reported by the first chunk that reaches the item, so that the chain
    // walks back to an entry point the caller asked for.
    let mut reported = BTreeSet::new();
    for plan in &plans {
        report_zombies(sess, table, &plan.reachable, &plan.parents, Some(plan.name), &mut reported);
    }

    let mut names: Vec<Vec<JsName>> = vec![Vec::new(); shared + 1];
    for (name, at) in &owner {
        names[*at].push((*name).clone());
    }
    // The owned names decide what the unit holds; the imports it reaches come with them, because
    // an import belongs to every unit that mentions what it binds. They carry no references of
    // their own, so nothing below has to tell them apart from what the unit owns.
    for (at, owned) in names.iter_mut().enumerate() {
        owned.extend(replicated_imports(table, &owner, at));
    }

    // Every unit's cross-file imports, and every unit's extra exports, in one pass over the
    // references of the items each unit owns.
    let mut imported: Vec<BTreeMap<usize, BTreeSet<&str>>> = vec![BTreeMap::new(); shared + 1];
    let mut exported: Vec<BTreeSet<&str>> = vec![BTreeSet::new(); shared + 1];
    for (at, owned) in names.iter().enumerate() {
        for name in owned {
            let Some(item) = table.get(name) else { continue };
            for referenced in &item.refs {
                let Some(&from) = owner.get(referenced) else { continue };
                if from == at {
                    continue;
                }
                let name = table.get_key_value(referenced).map_or(referenced.as_str(), |(name, _)| name.as_str());
                imported[at].entry(from).or_default().insert(name);
                exported[from].insert(name);
            }
        }
    }

    let label = |at: usize| match at == shared {
        true => opts.shared_chunk.as_str(),
        false => plans[at].name,
    };

    // The synthesized imports and the stubs are owned here, because a unit borrows its items.
    let crossings: Vec<Vec<LinkItem>> = imported
        .iter()
        .map(|from| {
            from.iter()
                .map(|(at, names)| cross_chunk_import(label(*at), names))
                .collect()
        })
        .collect();
    let stubs: Vec<Vec<LinkItem>> =
        names.iter().map(|owned| stub_missing(table, owned)).collect();

    let mut units = Vec::with_capacity(shared + 1);
    for (at, owned) in names.iter().enumerate() {
        // The shared chunk is written only when there is something to share: its absence is how
        // mires-build reads "two chunks had nothing in common".
        if at == shared && owned.is_empty() {
            continue;
        }
        let mut items = order(table, owned);
        items.splice(0..0, crossings[at].iter());
        items.extend(stubs[at].iter());
        units.push(Unit {
            label: label(at).to_owned(),
            path: Some(path.with_file_name(format!("{}.js", label(at)))),
            items,
            exports: exported[at].clone(),
        });
    }

    for (unit, text) in units.iter().zip(print_units(sess, &units)) {
        let Some(path) = &unit.path else { continue };
        if let Err(err) = fs::write(path, text.as_bytes()) {
            sess.dcx().err(format!("error writing `{}`: {err}", path.display()));
        }
    }
}

/// The item a `js-chunk=<name>:<entry>` names, by the exact spelling the caller used.
///
/// An exported item's JavaScript name *is* its symbol name (see `naming.rs`), so the usual case is
/// a direct hit. The fallback covers an item whose emitted name and fixed name differ.
fn entry_item<'a>(table: &'a BTreeMap<JsName, LinkItem>, entry: &str) -> Option<&'a JsName> {
    let name = JsName::new(entry.to_owned());
    if let Some((name, _)) = table.get_key_value(&name) {
        return Some(name);
    }
    table
        .iter()
        .find(|(_, item)| item.linkage.fixed_name.as_deref() == Some(entry))
        .map(|(name, _)| name)
}

/// The `import { .. } from "./<chunk>.js"` one chunk reaches another chunk's items through.
///
/// The specifier is **relative**, and deliberately: every chunk is published into one directory, so
/// a sibling path is correct wherever the files are served from and needs no import map entry to
/// resolve. The bare chunk name would need one, and the prefix that map is written with belongs to
/// the build tool rather than to the compiler, which never learns it.
///
/// The names are the items' own long ones. The item rename is a text pass that runs over this
/// declaration like any other, so both ends of the import come out spelled the same way.
fn cross_chunk_import(chunk: &str, names: &BTreeSet<&str>) -> LinkItem {
    let source = format!("./{chunk}.js");
    let named: Vec<(String, String)> =
        names.iter().map(|name| ((*name).to_owned(), (*name).to_owned())).collect();
    let decl = jsast::import_named(named, source.clone());
    let style = match crate::opts::get().minify {
        true => jsast::Style::Compact,
        false => jsast::Style::Readable,
    };
    LinkItem {
        name: JsName::new(format!("$chunk${chunk}")),
        kind: ItemKind::Import,
        js: jsast::program_to_string_styled(std::slice::from_ref(&decl), style),
        refs: Default::default(),
        zombies: Vec::new(),
        linkage: Linkage::internal(),
        debug_path: source,
        order: None,
        map: ItemMap::default(),
        dts: None,
    }
}

/// The import declarations the unit at `at` needs, which belong to every unit that reaches them.
///
/// An import is not owned by one chunk (see [`write_chunks`]), so it is picked up here from the
/// references of the items the unit does own.
fn replicated_imports(
    table: &BTreeMap<JsName, LinkItem>,
    owner: &BTreeMap<&JsName, usize>,
    at: usize,
) -> BTreeSet<JsName> {
    let mut wanted: BTreeSet<JsName> = BTreeSet::new();
    for (name, _) in owner.iter().filter(|(_, owned)| **owned == at) {
        let Some(item) = table.get(*name) else { continue };
        for referenced in &item.refs {
            if table.get(referenced).is_some_and(|item| item.kind == ItemKind::Import) {
                wanted.insert(referenced.clone());
            }
        }
    }
    wanted
}

/// Reports every zombie the program can reach, with the chain of items that reach it.
///
/// `chunk` names the chunk the walk started from, so a split program says which entry point is the
/// one that cannot be built. `reported` carries across the chunks of one program: an item two
/// chunks reach is one mistake, and is reported by the first chunk that reaches it.
fn report_zombies(
    sess: &Session,
    table: &BTreeMap<JsName, LinkItem>,
    reachable: &[JsName],
    parents: &HashMap<JsName, JsName>,
    chunk: Option<&str>,
    reported: &mut BTreeSet<JsName>,
) {
    // Deterministic order: `reachable` is discovery order, which depends on the BFS; sorting by
    // name makes two runs of the same compilation report the same way.
    let mut named: Vec<&JsName> = reachable.iter().collect();
    named.sort();

    for name in named {
        let Some(item) = table.get(name) else { continue };
        if item.zombies.is_empty() || !reported.insert(name.clone()) {
            continue;
        }
        for zombie in &item.zombies {
            let mut diag = sess.dcx().struct_err(format!("{}: {}", zombie.loc, zombie.message));
            diag.note(format!("in `{}`", item.debug_path));
            if let Some(chunk) = chunk {
                diag.note(format!("in the chunk `{chunk}`"));
            }
            // Walk back to the root, so the report says why this item is in the program at all.
            let mut current = name;
            let mut steps = 0;
            while let Some(parent) = parents.get(current) {
                let Some(parent_item) = table.get(parent) else { break };
                diag.note(format!("required by `{}`", parent_item.debug_path));
                current = parent;
                steps += 1;
                if steps == 16 {
                    diag.note("...");
                    break;
                }
            }
            diag.emit();
        }
    }
}

/// The reachable items: imports, then module level bindings, then everything else.
///
/// Imports come first because a module's bindings are its header, and because an imported name may
/// be read by a binding's initializer. They are ordered by `(module, binding)` — an import item's
/// `debug_path` is the module it imports from — which depends on nothing but the items themselves.
///
/// Function declarations hoist, so their order never matters. A `let` does not: a binding that
/// reads another binding has to come after it. The dependency order among bindings is a
/// depth-first post-order over `refs`; a cycle (which Rust's `static` rules make impossible) would
/// simply keep the discovery order.
///
/// A binding carrying a [`SourceOrder`] is placed before the rest and among its own kind in that
/// order. `view!` template cloners are what need it: a trace numbers them in declaration order, so
/// a program that declares the same cloners in another order is observably different from the
/// reference one. Nothing else is disturbed, because such a binding's initializer reads only
/// imports.
fn order<'a>(
    table: &'a BTreeMap<JsName, LinkItem>,
    reachable: &'a [JsName],
) -> Vec<&'a LinkItem> {
    let mut names: Vec<&JsName> = reachable.iter().collect();
    names.sort();
    // The ordered bindings first, in their own order; the rest keep the name order above. The sort
    // is stable, which is what leaves everything without an order where it already was.
    names.sort_by_key(|name| {
        let order = table.get(*name).and_then(|item| item.order.clone());
        (order.is_none(), order)
    });

    let mut out: Vec<&LinkItem> = Vec::with_capacity(names.len());
    let mut placed: std::collections::HashSet<&JsName> = std::collections::HashSet::new();

    let mut imports: Vec<&LinkItem> = names
        .iter()
        .filter_map(|name| table.get(*name))
        .filter(|item| item.kind == ItemKind::Import)
        .collect();
    imports.sort_by(|a, b| (&a.debug_path, &a.name).cmp(&(&b.debug_path, &b.name)));
    out.extend(imports);

    fn visit<'a>(
        name: &'a JsName,
        table: &'a BTreeMap<JsName, LinkItem>,
        placed: &mut std::collections::HashSet<&'a JsName>,
        out: &mut Vec<&'a LinkItem>,
    ) {
        let Some((key, item)) = table.get_key_value(name) else { return };
        if !item.kind.is_binding() || !placed.insert(key) {
            return;
        }
        for referenced in &item.refs {
            if table.get(referenced).is_some_and(|item| item.kind.is_binding()) {
                visit(referenced, table, placed, out);
            }
        }
        out.push(item);
    }

    for name in &names {
        visit(name, table, &mut placed, &mut out);
    }
    for name in &names {
        if let Some(item) = table.get(*name) {
            if !item.kind.is_binding() && item.kind != ItemKind::Import {
                out.push(item);
            }
        }
    }
    out
}
