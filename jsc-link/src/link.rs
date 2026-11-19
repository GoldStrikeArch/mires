//! Whole-program linking: reachability, zombie reporting and emission.
//!
//! # What "linking" means here
//!
//! There is no linker. A program is the set of JavaScript items reachable from its roots, printed
//! in an order that respects module level initialization. So the link step is a graph walk:
//!
//! 1. collect every item from this crate's compiled modules and from each linked rlib — the summary
//!    index and the printed text only, never a JIR body;
//! 2. deduplicate — the same generic instantiation may be codegenned in several crates, which is
//!    fine as long as the two spellings agree, and an outright bug if they do not;
//! 3. mark the roots: the `#[no_mangle]`/`#[export_name]`/`#[used]` items **of the crate being
//!    linked** and any `--js-root=NAME`;
//! 4. breadth-first search over `refs` (the summary's `deps`) **intersected with the item table**,
//!    remembering which item first reached each one;
//! 5. decode and verify the JIR bodies of the reachable items, and only those;
//! 6. report the zombies in reachable items only, each with the chain that made it reachable;
//! 7. print the reachable items, module level bindings first.
//!
//! Step 4 is where the naming invariants earn their keep: `refs` is every identifier the item's
//! JavaScript mentions, `Object` and `_3` and `bb` included, and the intersection is what turns
//! that into a call graph. Step 5 is what keeps the link step's cost proportional to the program
//! rather than to the sysroot: an unreachable body is never inflated.
//!
//! # Two modes
//!
//! [`Mode::Program`] is the real one, used for `--crate-type bin`/`cdylib`. An rlib never gets
//! here at all: the dylib hands it to rustc's own `link_binary`, which packs the objects into an
//! archive for a later link to read back.
//!
//! [`Mode::Prune`] handles the single-crate `--emit=obj` path that `scripts/compile.sh` uses. It
//! runs the same graph walk over the objects that were just written and rewrites them in place, so
//! a plain `rustc --emit=obj -o out.js` gets the same dead-code elimination and the same zombie
//! reporting as a real link.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use rcgjs_object::artifact::{
    Artifact, ArtifactKind, ArtifactManifest, Entry, EntryRole, RuntimeRequirement, ARTIFACT_SCHEMA,
};

use crate::diag::Sink;
use crate::item::{
    prologue, read_object, write_program, ItemKind, ItemMap, JsName, LinkItem, Linkage,
    CLEANUP_LOWERING_REQUIRED, FORMAT_VERSION,
};
use crate::jir::conflict::{self, ConflictRecord};
use crate::jir::summary::{MergeDecision, Side};
use crate::jsast;
use crate::lane_report::{ItemLane, LaneAvailability, LaneItem};
use crate::manifest::{Manifest, Mode};
use crate::opts::{Lane, Modules};

// `pub(crate)`: the representation planner (`plan/representation.rs`, W4b) fills the sidecar's
// `ReprBlock`, which is defined beside the sidecar it is rendered into.
mod hoist_consts;
pub(crate) mod lane;
/// The initialization order of module level bindings, and the two-step form of a cycle of statics.
mod statics;

use statics::Placed;

/// The name the runtime shim is bound to. Emitted calls are `__rt.<symbol>(...)`, in either mode.
const SHIM_NAMESPACE: &str = "__rt";

/// Runs one link step. Returns the number of errors reported.
///
/// The whole of `jsc-link` hangs off this: the manifest names the objects, the rlibs, the output
/// and the options, and nothing else about the compilation is visible from here.
pub fn run(manifest: &Manifest) -> u32 {
    let total_scope = crate::profile::scope("link.total");
    let dcx = Sink::new();
    crate::opts::set(manifest.opts.clone());
    let read_scope = crate::profile::scope("link.read");

    let mut items = Read {
        source_map: manifest.opts.source_map,
        ..Read::default()
    };
    items.sources.extend(manifest.sources.iter().cloned());
    for path in &manifest.objects {
        read_into(&dcx, path, &mut items);
    }
    // Everything this crate depends on, in the archives rustc built for them.
    //
    // A dependency's exports are *not* this program's roots. That is how a static archive behaves
    // everywhere else — a member is pulled in when something needs it, not because it defines a
    // public symbol — and here it is the difference between a hello-world and all of `core`:
    // `compiler_builtins` marks a few hundred items `#[no_mangle]`, and rooting them would drag in
    // `libm`, `fmt` and the whole panic machinery for a program that never calls any of it.
    for path in &manifest.rlibs {
        let first = items.items.len();
        read_archive_into(&dcx, path, &mut items);
        for item in &mut items.items[first..] {
            item.linkage.demote();
        }
    }
    items.note_mixed_formats(&dcx);
    drop(read_scope);
    let sources = std::mem::take(&mut items.sources);

    match &manifest.mode {
        Mode::Program { output: Some(path) } => {
            let program = emit(
                &dcx,
                &sources,
                items.items,
                &items.coroutine_sources,
                Some(path.as_path()),
            );
            if let Err(err) = fs::write(path, program.as_bytes()) {
                dcx.err(format!("error writing `{}`: {err}", path.display()));
            }
        }
        // Nothing to hang a `.map` off: a program on stdout has no path to name one after.
        Mode::Program { output: None } => {
            let program = emit(&dcx, &sources, items.items, &items.coroutine_sources, None);
            print!("{program}");
        }
        Mode::Prune { first, rest } => {
            let program = emit(
                &dcx,
                &sources,
                items.items,
                &items.coroutine_sources,
                Some(first.as_path()),
            );
            if let Err(err) = fs::write(first, program.as_bytes()) {
                dcx.err(format!("error writing `{}`: {err}", first.display()));
            }
            for path in rest {
                let _ = fs::write(path, prologue("(empty)").as_bytes());
            }
        }
    }
    drop(total_scope);
    crate::profile::dump();
    dcx.errors()
}

/// One printing of the program, on one lane.
///
/// [`Lane::Shadow`] is the only setting that needs more than one: it prints the baseline program
/// to the output path and the JIR program beside it, so that a differential run has two files
/// that came out of one set of objects. The two passes differ in the lane they print from and in
/// which of the link step's side files they own, and in nothing else — the whole point of the
/// exercise is that everything downstream of the lane is the same code either way.
struct Pass<'a> {
    coroutine_sources: &'a crate::plan::async_graph::SourceCaptures,
    /// Never [`Lane::Shadow`]: a pass prints one lowering.
    lane: Lane,
    /// Whether this pass writes the optimizer report and the dedup conflict autopsy. They
    /// describe the baseline analyses, so the baseline pass owns them.
    reports: bool,
    /// Whether this pass writes `js-lane-report` and the `<out>.lane.json` sidecar. The pass that
    /// ran a lane owns them, because it is the one with something to say.
    lane_files: bool,
    /// Where the sidecar goes: beside the program the user asked for, never beside `<out>.jir.js`.
    /// `None` when no lane ran or there is no output path to hang it off.
    sidecar: Option<PathBuf>,
}

/// Resolves the program once per lane the build asked for, and returns the text for the output
/// path. The shadow lane's second program is written here, since only its own path names it.
///
/// The passes share one diagnostic sink, which is what lets a lowering that disagrees with the
/// baseline fail the build from the second pass. The cost is that a shadow build reports a
/// whole-program diagnostic — a zombie, a workspace miss — once per lane; a diagnostic is about
/// the objects, and the objects were read once.
fn emit(
    dcx: &Sink,
    sources: &BTreeMap<String, String>,
    items: Vec<LinkItem>,
    coroutine_sources: &crate::plan::async_graph::SourceCaptures,
    path: Option<&Path>,
) -> String {
    let opts = crate::opts::get();
    let sidecar = match opts.lane {
        Lane::Baseline => None,
        _ => path.map(Path::to_path_buf),
    };
    match (opts.lane, path) {
        (Lane::Shadow, Some(path)) => {
            let baseline = resolve(
                dcx,
                sources,
                items.clone(),
                "program",
                Some(path),
                &Pass {
                    coroutine_sources,
                    lane: Lane::Baseline,
                    reports: true,
                    lane_files: false,
                    sidecar: None,
                },
            );
            let jir_path = lane::shadow_path(path);
            let jir = resolve(
                dcx,
                sources,
                items,
                "program",
                Some(&jir_path),
                &Pass {
                    coroutine_sources,
                    lane: Lane::Jir,
                    reports: false,
                    lane_files: true,
                    sidecar,
                },
            );
            if let Err(err) = fs::write(&jir_path, jir.as_bytes()) {
                dcx.err(format!("error writing `{}`: {err}", jir_path.display()));
            }
            baseline
        }
        // A shadow build compares two files, and a program on stdout is not a file.
        (Lane::Shadow, None) => {
            dcx.err(
                "`-Cllvm-args=js-lane=shadow` needs an output path to write the JIR program \
                 beside",
            );
            String::new()
        }
        (lane, _) => resolve(
            dcx,
            sources,
            items,
            "program",
            path,
            &Pass {
                coroutine_sources,
                lane,
                reports: true,
                lane_files: true,
                sidecar,
            },
        ),
    }
}

// -------------------------------------------------------------------------------------------
// Reading
// -------------------------------------------------------------------------------------------

/// The item lists read so far and which object formats they came from, so that a program mixing
/// a format 20 crate with a format 19 sysroot can be told exactly once.
#[derive(Default)]
struct Read {
    items: Vec<LinkItem>,
    coroutine_sources: crate::plan::async_graph::SourceCaptures,
    sources: BTreeMap<String, String>,
    source_map: bool,
    formats: BTreeSet<u16>,
    /// The first format 20 header seen, and where it came from. Every later object is held to it:
    /// type ids, definition ids and printed text are only comparable within one JIR schema, one
    /// rustc, one data layout and one set of emit-affecting options.
    header: Option<(String, crate::item::Header)>,
}

impl Read {
    fn extend(&mut self, dcx: &Sink, label: &str, read: crate::item::ReadObject) {
        if self.source_map {
            match read.sources() {
                Ok(sources) => {
                    for (name, text) in sources {
                        match self.sources.entry(name) {
                            std::collections::btree_map::Entry::Vacant(entry) => {
                                entry.insert(text);
                            }
                            std::collections::btree_map::Entry::Occupied(entry)
                                if entry.get() != &text =>
                            {
                                dcx.err(format!("conflicting source contents for `{}` in `{label}`; rebuild dependencies with distinct source paths", entry.key()));
                            }
                            _ => {}
                        }
                    }
                }
                Err(error) => dcx.err(format!("`{label}`: {error}")),
            }
        }
        if let Some(interns) = read.interns() {
            self.coroutine_sources.extend(interns);
        }
        self.formats.insert(read.format);
        // Once per object, not once per item: the answer is a property of the object pair.
        if let Some(header) = read.header {
            match &self.header {
                None => self.header = Some((label.to_owned(), header)),
                Some((first_label, first)) => {
                    if let Err(decline) = crate::jir::summary::merge_headers(first, &header) {
                        let mut diag = dcx.struct_err(format!(
                            "rustc_codegen_js: `{label}` cannot be linked with `{first_label}`: \
                             {decline}"
                        ));
                        diag.note(
                            "every object in a program must be compiled with the same toolchain, \
                             target and `-Cllvm-args=js-*` options; rebuild the sysroot with the \
                             ones this crate uses (`JS_EXTRA_ARGS='...' scripts/build_sysroot.sh`)",
                        );
                        diag.emit();
                    }
                }
            }
        }
        self.items.extend(read.items);
    }

    /// Notes once that some objects predate the binary member. Their items link, and print the
    /// same baseline text as before; they only carry no JIR bodies.
    fn note_mixed_formats(&self, dcx: &Sink) {
        if self.formats.len() > 1 {
            let legacy: Vec<String> = self
                .formats
                .iter()
                .filter(|format| **format != FORMAT_VERSION)
                .map(u16::to_string)
                .collect();
            dcx.warn(format!(
                "this program links objects in format {FORMAT_VERSION} together with objects in \
                 format {}: the older ones carry no JIR bodies, so their items can only print \
                 from the baseline lane. Rebuild the sysroot and dependencies \
                 (`scripts/build_sysroot.sh --clean`) to carry JIR throughout",
                legacy.join(", ")
            ));
        }
    }
}

fn read_into(dcx: &Sink, path: &Path, out: &mut Read) {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            dcx.err(format!("error reading `{}`: {err}", path.display()));
            return;
        }
    };
    match read_object(&bytes) {
        Ok(Some(read)) => out.extend(dcx, &path.display().to_string(), read),
        // Neither trailer nor footer: not one of ours, or already a finished program. Nothing to
        // link.
        Ok(None) => {}
        Err(err) => {
            dcx.err(format!("`{}`: {err}", path.display()));
        }
    }
}

/// Reads every `rcgjs` object out of an `ar` archive.
///
/// Members are read as bytes: a format 20 object is not UTF-8 past its text section, and rustc's
/// metadata member never was. `read_object` tells the three apart.
fn read_archive_into(dcx: &Sink, path: &Path, out: &mut Read) {
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
            dcx.err(format!(
                "`{}` is not a readable archive: {err}",
                path.display()
            ));
            return;
        }
    };
    for member in archive.members() {
        let Ok(member) = member else { continue };
        let Ok(data) = member.data(&*bytes) else {
            continue;
        };
        match read_object(data) {
            Ok(Some(read)) => out.extend(
                dcx,
                &format!(
                    "{}({})",
                    path.display(),
                    String::from_utf8_lossy(member.name())
                ),
                read,
            ),
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
fn resolve(
    dcx: &Sink,
    sources: &BTreeMap<String, String>,
    mut items: Vec<LinkItem>,
    label: &str,
    path: Option<&Path>,
    pass: &Pass,
) -> String {
    let opts = crate::opts::get();
    if opts.modules == Modules::Esm {
        items.push(shim_import());
    }

    let mut conflicts: Vec<ConflictRecord> = Vec::new();
    let mut table = dedup(dcx, items, &mut conflicts);
    let split = !opts.chunks.is_empty();
    if split
        && path.is_some_and(|path| {
            let name = path.file_name().and_then(|name| name.to_str());
            opts.chunks
                .iter()
                .any(|(chunk, _)| name == Some(format!("{chunk}.js").as_str()))
                || name == Some(format!("{}.js", opts.shared_chunk).as_str())
        })
    {
        dcx.err("the unsplit output path collides with an emitted chunk; choose a different output name");
        return String::new();
    }
    if split && opts.optimization_report.is_some() {
        dcx.err(
            "`js-opt-report` cannot be combined with `js-chunk` until reports account for each \
             emitted chunk",
        );
    }

    let (mut reachable, mut parents) = reach(dcx, &table);
    let baseline_count = reachable.len();
    decode_reachable_bodies(dcx, &mut table, &mut reachable, pass.lane != Lane::Baseline);
    // The lane, before anything reads an item's printed text. An item the lowerer takes is a
    // `LinkItem` like any other from here on; one it declines still holds v1's text.
    let mut lane_run = match pass.lane {
        Lane::Baseline => None,
        _ => Some(lane::run(
            dcx,
            &mut table,
            &reachable,
            pass.coroutine_sources,
        )),
    };
    // The v1 text lowerer that once rewrote printed baseline JavaScript here is gone; the JIR
    // lane above is the optimizer. On the baseline lane `js-opt=on` now means the producer-baked
    // passes plus link-dedup, with no link-time text rewriting.
    if table.values().any(|item| {
        item.jir
            .body
            .as_ref()
            .is_some_and(|body| body.blocks.iter().any(|block| block.cleanup))
    }) {
        let (live, _) = reach(dcx, &table);
        reject_cleanup_fallbacks(dcx, &table, &live, lane_run.as_ref());
    }
    let (mut dedup, references_changed) = if crate::lower::bstr::enabled() {
        let pre_changed = baseline_count != reachable.len()
            || lane_run
                .as_ref()
                .is_some_and(|run| run.added_items() || run.references_changed());
        if pre_changed {
            (reachable, parents) = reach(dcx, &table);
        }
        let facades = byte_string_facades(dcx, &mut table, &mut reachable);
        for (name, body) in &facades {
            if let Some(run) = &mut lane_run {
                run.rename(name, body);
            }
        }
        let pre_synth = reachable.len();
        crate::lower::bstr::synthesize_converters(&mut table, &mut reachable);
        let added_bstr = !facades.is_empty() || reachable.len() != pre_synth;
        let dedup = crate::profile::time("link.dedup", || {
            crate::link_dedup::merge_duplicates(
                &mut table,
                &reachable,
                &BTreeSet::new(),
                &mut conflicts,
            )
        });
        let references_changed = added_bstr || dedup.references_changed();
        (dedup, references_changed)
    } else {
        let dedup = crate::profile::time("link.dedup", || {
            crate::link_dedup::merge_duplicates(
                &mut table,
                &reachable,
                &BTreeSet::new(),
                &mut conflicts,
            )
        });
        let references_changed = baseline_count != reachable.len() || dedup.references_changed()
            // The flat-call ABI adds the spill registers to the table, and an item nothing has walked
            // to yet is an item `order` will not place and `print_units` will not print. A lowering
            // the lane accepted may also reach *fewer* items than the baseline did — a folded branch
            // is the ordinary case — and what nothing reaches any more must stop being printed.
            || lane_run
                .as_ref()
                .is_some_and(|run| run.added_items() || run.references_changed());
        (dedup, references_changed)
    };
    if references_changed {
        (reachable, parents) = reach(dcx, &table);
    }
    if crate::lower::bstr::enabled() {
        refuse_hybrid_strings(dcx, &table, &reachable);
    }
    dedup.retain_reachable(&reachable);
    if let Some(lane_run) = &mut lane_run {
        lane_run.retain_reachable(&reachable);
    }
    crate::report::warn_misses(dcx, &table, &reachable);
    // With chunks the same zombie would otherwise be reported twice, once here and once for the
    // chunk that reaches it. The per-chunk report is the more useful of the two: its chain names
    // the entry point that pulled the item in.
    if !split {
        report_zombies(
            dcx,
            &table,
            &reachable,
            &parents,
            None,
            &mut BTreeSet::new(),
        );
    }

    let stubs = stub_missing(&table, &reachable);
    let placed = layout(&table, &reachable);
    let cycles = statics::initialize_cycles(dcx, &placed);
    let mut ordered = statics::print_order(&placed, &cycles);
    ordered.extend(stubs.iter());
    let unit = Unit {
        label: label.to_owned(),
        path: path.map(Path::to_path_buf),
        items: ordered,
        exports: BTreeSet::new(),
        entry: None,
    };
    let mut printed = crate::profile::time("link.print", || {
        print_units(dcx, sources, std::slice::from_ref(&unit))
    });
    let mut deployment = ArtifactManifest {
        schema: ARTIFACT_SCHEMA,
        object_format: FORMAT_VERSION,
        jir_schema: crate::jir::JIR_SCHEMA,
        semantic_policy: "faithful".to_owned(),
        syntax_ceiling: match opts.ecma_target {
            crate::opts::EcmaTarget::Es2018 => "2018",
            crate::opts::EcmaTarget::Latest => "latest",
        }
        .to_owned(),
        entries: BTreeMap::new(),
        artifacts: printed
            .first()
            .map(|unit| unit.artifacts.clone())
            .unwrap_or_default(),
    };
    if !split {
        if let Some(file) = path
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
        {
            let mut roots = BTreeMap::new();
            for item in unit.items.iter().filter(|item| {
                item.linkage.root || opts.roots.iter().any(|name| name == item.name.as_str())
            }) {
                roots.insert(item.name.to_string(), root_identity(item));
            }
            if !roots.is_empty() {
                deployment.entries.insert(
                    label.to_owned(),
                    Entry {
                        role: opts.entry_role,
                        program: file.to_owned(),
                        roots,
                    },
                );
            }
        }
    }

    if pass.reports && !split {
        if let (Some(report_path), Some(program)) = (&opts.optimization_report, printed.first()) {
            crate::report::write(
                dcx,
                Path::new(report_path),
                label,
                program.text.len(),
                &program.item_bytes,
                program.synthesized_bytes,
                &table,
                &reachable,
                &parents,
                &dedup,
                lane_run.as_ref(),
            );
        }
    }

    if pass.lane_files {
        if let Some(report_path) = &opts.lane_report {
            write_lane_report(
                dcx,
                Path::new(report_path),
                &table,
                &reachable,
                lane_run.as_ref(),
            );
        }
        if let (Some(sidecar), Some(lane_run)) = (&pass.sidecar, &lane_run) {
            lane::write_sidecar(dcx, sidecar, lane_run, opts.lane);
        }
    }
    if pass.reports {
        if let Some(dump_path) = &opts.jir_dump_conflicts {
            if let Err(err) = crate::jir::conflict::write(Path::new(dump_path), &conflicts) {
                dcx.err(format!("error writing `{dump_path}`: {err}"));
            }
        }
    }

    if split {
        match path {
            Some(path) => write_chunks(dcx, sources, &table, path, &mut deployment),
            None => {
                dcx.err(
                    "`-Cllvm-args=js-chunk` needs an output path to write the chunk files beside",
                );
            }
        }
    }

    if let Some(path) = path.filter(|_| dcx.errors() == 0) {
        let names = deployment
            .artifacts
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        for artifact in deployment.artifacts.values_mut() {
            for (specifier, target) in &mut artifact.imports {
                if let Some(local) = specifier
                    .strip_prefix("./")
                    .filter(|local| names.contains(*local))
                {
                    *target = Some(local.to_owned());
                }
            }
        }
        let manifest_path = rcgjs_object::artifact::manifest_path(path);
        if let Err(error) = deployment.write(&manifest_path) {
            dcx.err(format!(
                "error writing `{}`: {error}",
                manifest_path.display()
            ));
        }
    }

    printed
        .pop()
        .map(|program| program.text)
        .unwrap_or_default()
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
    /// The entry a named chunk publishes for a module-script loader.
    entry: Option<&'a str>,
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
struct PrintedUnit {
    text: String,
    /// Exact contribution of each item after the link-time rename.
    item_bytes: BTreeMap<JsName, usize>,
    /// Prologue, exports, source map comment, and other link-synthesized text.
    synthesized_bytes: usize,
    artifacts: BTreeMap<String, Artifact>,
}

/// `js-str=bytes`: an exported function that takes or returns a `str` gets a **facade**.
///
/// An export is the item itself, called by the host under its fixed name, so the host's text
/// would reach a byte-string body unencoded and Rust's byte string would reach the host
/// undecoded. The body is renamed `<name>$bstr` — every reference to it in the program with it,
/// so a Rust caller keeps calling the body directly and converts nothing — and a facade takes
/// the exported name: `function name(a0, a1) { return <out>(name$bstr(<in>(a0), a1)); }`, each
/// argument and the result converted wherever they reach a `str` (`bstr::host_convert`: the
/// value itself, a field, an element, a variant payload, a shared pointee). A signature that
/// conversion cannot rebuild exactly is refused with the reason, and so is one whose JavaScript
/// parameters are not one per Rust argument.
pub(crate) fn item_caller_location_def(
    body: &crate::jir::Body,
    interns: &crate::jir::Interns,
) -> Option<crate::jir::DefinitionId> {
    use crate::jir::semantic::{Callee, Terminator};
    use crate::jir::types::TypeKind;
    use crate::lower::js::value;

    let types = &interns.types;
    for block in &body.blocks {
        let Terminator::Call(call) = &block.terminator else {
            continue;
        };
        if let Callee::Intrinsic { name, .. } = &call.callee {
            if interns.str(*name) == Some("caller_location") {
                let loc_ty =
                    value::pointee_of(types, call.destination.ty).unwrap_or(call.destination.ty);
                if let Some(TypeKind::Adt { def, .. }) =
                    types.get(types.peeled(loc_ty)).map(|d| &d.kind)
                {
                    return Some(*def);
                }
            }
        }
    }
    None
}

pub(crate) fn collect_panic_location_context(
    table: &BTreeMap<JsName, LinkItem>,
) -> (BTreeSet<crate::jir::DefinitionId>, BTreeSet<u64>) {
    use crate::jir::semantic::{CallSemantic, Callee, StandardOperation, Terminator};

    let mut global_loc_defs = BTreeSet::new();
    let mut std_core_crates = BTreeSet::new();
    let mut named_core_crates = BTreeSet::new();
    for item in table.values() {
        let (Some(body), Some(interns)) = (item.jir.body.as_ref(), item.jir.interns()) else {
            continue;
        };
        if let Some(def) = item_caller_location_def(body, interns) {
            global_loc_defs.insert(def);
        }
        for block in &body.blocks {
            let Terminator::Call(call) = &block.terminator else {
                continue;
            };
            if let Callee::Direct {
                item: callee_item, ..
            } = &call.callee
            {
                if matches!(
                    call.semantic,
                    CallSemantic::Standard(op)
                        if op.is_str() || matches!(op, StandardOperation::SliceSwap)
                ) {
                    std_core_crates.insert(callee_item.definition.crate_id);
                }
            }
        }
        if item.crate_name == "core" {
            let (origin_crate, _) =
                crate::lower::semantic_ops::string::item_origin_info(item, interns);
            if origin_crate == "core" {
                named_core_crates.insert(body.item.definition.crate_id);
            }
        }
    }
    let core_crate_ids = if !std_core_crates.is_empty() {
        std_core_crates
    } else {
        named_core_crates
    };
    (global_loc_defs, core_crate_ids)
}

fn panic_location_field_keys(
    types: &crate::jir::types::TypeTable,
    interns: &crate::jir::Interns,
    body: Option<&crate::jir::Body>,
    global_loc_defs: &BTreeSet<crate::jir::DefinitionId>,
    core_crate_ids: &BTreeSet<u64>,
) -> (String, String, String) {
    use crate::jir::semantic::{Callee, Terminator};
    use crate::lower::js::constant::{resolve_location_field_keys, LocationKeysOutcome};
    use crate::lower::js::value;

    let mut exact_tys = BTreeSet::new();
    let mut own_crate = None;
    if let Some(body) = body {
        own_crate = Some(body.item.definition.crate_id);
        for block in &body.blocks {
            if let Terminator::Call(call) = &block.terminator {
                if let Callee::Intrinsic { name, .. } = &call.callee {
                    if interns.str(*name) == Some("caller_location") {
                        let loc_ty = value::pointee_of(types, call.destination.ty)
                            .unwrap_or(call.destination.ty);
                        exact_tys.insert(types.peeled(loc_ty));
                    }
                }
            }
        }
    }
    match resolve_location_field_keys(
        types,
        interns,
        &exact_tys,
        global_loc_defs,
        core_crate_ids,
        own_crate,
        false,
    ) {
        Ok(LocationKeysOutcome::Found(keys)) => keys,
        _ => ("filename".to_owned(), "line".to_owned(), "col".to_owned()),
    }
}

fn byte_string_facades(
    dcx: &Sink,
    table: &mut BTreeMap<JsName, LinkItem>,
    reachable: &mut Vec<JsName>,
) -> Vec<(JsName, JsName)> {
    use crate::lower::bstr::{self, Crossing};
    let (global_loc_defs, core_crate_ids) = collect_panic_location_context(table);
    let mut facades = Vec::new();
    for name in reachable.iter() {
        let Some(item) = table.get(name) else {
            continue;
        };
        if !item.linkage.exported {
            continue;
        }
        let s = name.as_str();
        if s == "rust_begin_unwind" || s.ends_with("17rust_begin_unwind") {
            continue;
        }
        let (Some(body), Some(interns)) = (item.jir.body.as_ref(), item.jir.interns()) else {
            continue;
        };
        let types = &interns.types;
        let signature = &body.signature;
        let reaches = |ty: &crate::jir::types::TypeId| bstr::reaches_str(types, *ty);
        if !signature.arguments.iter().any(reaches)
            && !reaches(&signature.result)
            && !signature.abi.caller_location
        {
            continue;
        }
        let refuse = |why: &str| {
            dcx.err(format!(
                "rustc_codegen_js: `js-str=bytes` cannot give the exported `{}` a byte-string \
                 facade: {why}",
                name.as_str()
            ))
        };
        if signature.abi.spread.is_some()
            || usize::from(signature.abi.parameters)
                != signature.arguments.len() + usize::from(signature.abi.caller_location)
        {
            refuse("its JavaScript parameters are not one per Rust argument");
            continue;
        }
        let inner = JsName::new(format!("{}$bstr", name.as_str()));
        if table.contains_key(&inner) {
            refuse("its body's private name is taken");
            continue;
        }
        let mut params: Vec<String> = (0..signature.arguments.len())
            .map(|i| format!("a{i}"))
            .collect();
        let arguments: Result<Vec<_>, String> = signature
            .arguments
            .iter()
            .zip(&params)
            .map(|(ty, param)| {
                bstr::host_convert(types, interns, *ty, jsast::id(param.clone()), Crossing::In)
            })
            .collect();
        let mut arguments = match arguments {
            Ok(arguments) => arguments,
            Err(why) => {
                refuse(&why);
                continue;
            }
        };
        if signature.abi.caller_location {
            let loc_param = "a_loc".to_owned();
            let (file_key, line_key, col_key) = panic_location_field_keys(
                types,
                interns,
                Some(body),
                &global_loc_defs,
                &core_crate_ids,
            );
            let default_loc = jsast::object(vec![
                (file_key.clone(), bstr::literal("<host>")),
                (line_key, jsast::num(0)),
                (col_key, jsast::num(1)),
            ]);
            let loc_spec = jsast::object_of(vec![
                ("k", jsast::string("o")),
                ("f", jsast::object(vec![(file_key, jsast::string("s"))])),
            ]);
            let wrapped_loc =
                jsast::rt_call("bstr_ref_in", vec![jsast::id(loc_param.clone()), loc_spec]);
            let loc_expr = jsast::cond(
                jsast::binary(
                    jsast::BinOp::Eq,
                    jsast::id(loc_param.clone()),
                    jsast::null(),
                ),
                default_loc,
                wrapped_loc,
            );
            params.push(loc_param);
            arguments.push(loc_expr);
        }
        let call = jsast::call(jsast::id(inner.as_str().to_owned()), arguments);
        let result = match bstr::host_convert(types, interns, signature.result, call, Crossing::Out)
        {
            Ok(result) => result,
            Err(why) => {
                refuse(&why);
                continue;
            }
        };
        let decl = jsast::function(name.as_str().to_owned(), params, vec![jsast::ret(result)]);
        facades.push((name.clone(), inner, decl));
    }
    if facades.is_empty() {
        return Vec::new();
    }
    let moved: Vec<(JsName, JsName)> = facades
        .iter()
        .map(|(name, inner, _)| (name.clone(), inner.clone()))
        .collect();
    let renames: BTreeMap<String, String> = facades
        .iter()
        .map(|(name, inner, _)| (name.as_str().to_owned(), inner.as_str().to_owned()))
        .collect();
    for item in table.values_mut() {
        if !item.refs.iter().any(|r| renames.contains_key(r.as_str()))
            && !renames.contains_key(item.name.as_str())
        {
            continue;
        }
        let (js, map) = crate::minify::apply_renames_mapped(&item.js, &item.map, &renames);
        item.js = js;
        item.map = map;
        item.refs = std::mem::take(&mut item.refs)
            .into_iter()
            .map(|r| match renames.get(r.as_str()) {
                Some(to) => JsName::new(to.clone()),
                None => r,
            })
            .collect();
    }
    let style = match crate::opts::get().minify {
        true => jsast::Style::Compact,
        false => jsast::Style::Readable,
    };
    for (name, inner, decl) in facades {
        let Some(mut body) = table.remove(&name) else {
            continue;
        };
        let mut refs: BTreeSet<JsName> = BTreeSet::new();
        refs.insert(inner.clone());
        refs.insert(JsName::new(SHIM_NAMESPACE.to_owned()));
        jsast::visit_free_idents(&decl, &mut |ident| {
            if ident != name.as_str() {
                refs.insert(JsName::new(ident.to_owned()));
            }
        });
        let facade = LinkItem {
            name: name.clone(),
            kind: ItemKind::Fn,
            js: jsast::program_to_string_styled_for_target(
                std::slice::from_ref(&decl),
                style,
                crate::opts::get().ecma_target,
            ),
            refs,
            zombies: Vec::new(),
            linkage: body.linkage.clone(),
            debug_path: format!("{} (js-str=bytes facade)", body.debug_path),
            crate_name: body.crate_name.clone(),
            order: None,
            map: ItemMap::default(),
            dts: body.dts.take(),
            optimization: Default::default(),
            late_ir: Default::default(),
            baseline_shadow: Default::default(),
            jir: Default::default(),
        };
        body.name = inner.clone();
        body.linkage = Linkage::internal();
        table.insert(inner.clone(), body);
        table.insert(name, facade);
        reachable.push(inner);
    }
    moved
}

/// `js-str=bytes`: no item the program prints may still call a UTF-16 `str` helper — a verbatim
/// fragment the byte-string rewrite cannot see, or synthesized text that predates it.
fn refuse_hybrid_strings(dcx: &Sink, table: &BTreeMap<JsName, LinkItem>, reachable: &[JsName]) {
    for name in reachable {
        let Some(item) = table.get(name) else {
            continue;
        };
        if let Some(helper) = crate::lower::bstr::leftover_helper(&item.js) {
            dcx.err(format!(
                "rustc_codegen_js: `js-str=bytes`, but `{}` still calls the UTF-16 helper \
                 `__rt.{helper}`",
                name.as_str()
            ));
        }
    }
}

fn print_units(
    dcx: &Sink,
    sources: &BTreeMap<String, String>,
    units: &[Unit<'_>],
) -> Vec<PrintedUnit> {
    let opts = crate::opts::get();
    let bodies: Vec<String> = units
        .iter()
        .map(|unit| write_program(&prologue(&unit.label), &unit.items).0)
        .collect();

    // The whole-program half of minification: an item name can only be shortened once every
    // reference to it is in one place, which is here and nowhere earlier. See `minify.rs`.
    let renames = match opts.minify && opts.minify_items {
        false => BTreeMap::new(),
        true => {
            let items: Vec<&LinkItem> = units
                .iter()
                .flat_map(|unit| unit.items.iter().copied())
                .collect();
            warn_short_fixed_names(dcx, &items);
            let corpus: String = bodies.iter().map(String::as_str).collect();
            crate::minify::item_renames(&corpus, &items)
        }
    };

    let mut printed = Vec::with_capacity(units.len());
    for unit in units {
        let renamed: Vec<LinkItem> = unit
            .items
            .iter()
            .map(|item| {
                let mut item = (*item).clone();
                let (js, map) = crate::minify::apply_renames_mapped(&item.js, &item.map, &renames);
                item.js = js;
                item.map = map;
                item
            })
            .collect();
        let renamed_refs: Vec<&LinkItem> = renamed.iter().collect();
        let (mut text, starts) = write_program(&prologue(&unit.label), &renamed_refs);
        // After the rename, so that the clause is written once, out of the names that survived it,
        // and never scanned as if it were an item's code.
        text.push_str(&export_clause(unit, &renames));
        text.push_str(&chunk_registration(unit, &renames));

        if let (true, Some(path)) = (opts.source_map, &unit.path) {
            let comment = write_source_map(dcx, sources, path, &text, &renamed_refs, &starts);
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
                dcx.err(format!("error writing `{}`: {err}", path.display()));
            }
        }
        let item_bytes: BTreeMap<JsName, usize> = renamed
            .iter()
            .map(|item| (item.name.clone(), item.js.len()))
            .collect();
        let item_total: usize = item_bytes.values().sum();
        let synthesized_bytes = text
            .len()
            .checked_sub(item_total)
            .expect("renamed items are contained in their printed unit");
        let artifacts = artifact_records(dcx, unit, &text, &renames);
        printed.push(PrintedUnit {
            text,
            item_bytes,
            synthesized_bytes,
            artifacts,
        });
    }
    printed
}

fn root_identity(item: &LinkItem) -> String {
    match item
        .jir
        .summary
        .as_ref()
        .and_then(|summary| summary.key.item)
    {
        Some(item) => format!(
            "rust:{:016x}:{:016x}:{:032x}",
            item.definition.crate_id, item.definition.local_hash, item.instantiation
        ),
        None => format!("export:{}", item.name),
    }
}

fn artifact_records(
    dcx: &Sink,
    unit: &Unit<'_>,
    text: &str,
    renames: &BTreeMap<String, String>,
) -> BTreeMap<String, Artifact> {
    let mut artifacts = BTreeMap::new();
    let Some(path) = &unit.path else {
        return artifacts;
    };
    let Some(file) = path.file_name().and_then(|name| name.to_str()) else {
        dcx.err("artifact output needs a UTF-8 file name");
        return artifacts;
    };
    let opts = crate::opts::get();
    let mut program = Artifact::new(ArtifactKind::Program, text.as_bytes());
    // Every compiler-generated import stores its module specifier as the item's debug path.
    // This holds for extern imports, DOM imports, the runtime, and cross-chunk imports.
    for item in &unit.items {
        if item.kind == ItemKind::Import {
            program.imports.insert(item.debug_path.clone(), None);
        } else {
            program.items.insert(root_identity(item));
        }
        if item.linkage.exported || unit.exports.contains(item.name.as_str()) {
            program.exports.insert(
                renames
                    .get(item.name.as_str())
                    .cloned()
                    .unwrap_or_else(|| item.name.to_string()),
            );
        }
    }
    let members = crate::plan::cost::runtime_members(text);
    if !members.is_empty() {
        let helpers = crate::plan::cost::helper_bytes();
        let (direct, external): (BTreeSet<_>, BTreeSet<_>) = members
            .into_iter()
            .partition(|member| helpers.contains_key(member));
        program.runtime = Some(RuntimeRequirement {
            module: (opts.modules == Modules::Esm).then(|| opts.shim_module.clone()),
            source_sha256: crate::plan::cost::runtime_source_sha256(),
            closure: crate::plan::cost::helper_closure(&direct),
            direct,
            external,
            artifact: None,
        });
    }
    for (enabled, related, kind) in [
        (
            opts.source_map,
            path.with_file_name(format!("{file}.map")),
            ArtifactKind::SourceMap,
        ),
        (
            opts.dts,
            path.with_extension("d.ts"),
            ArtifactKind::Declarations,
        ),
    ] {
        if !enabled {
            continue;
        }
        match fs::read(&related) {
            Ok(bytes) => {
                let name = related.file_name().unwrap().to_string_lossy().into_owned();
                match kind {
                    ArtifactKind::SourceMap => program.source_map = Some(name.clone()),
                    ArtifactKind::Declarations => program.declarations = Some(name.clone()),
                    ArtifactKind::Program | ArtifactKind::Runtime => unreachable!(),
                }
                artifacts.insert(name, Artifact::new(kind, &bytes));
            }
            Err(error) => dcx.err(format!(
                "cannot bind artifact `{}`: {error}",
                related.display()
            )),
        }
    }
    artifacts.insert(file.to_owned(), program);
    artifacts
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
        js: jsast::program_to_string_styled_for_target(
            std::slice::from_ref(&decl),
            style,
            crate::opts::get().ecma_target,
        ),
        refs: Default::default(),
        zombies: Vec::new(),
        linkage: Linkage {
            fixed_name: Some(SHIM_NAMESPACE.to_owned()),
            root: false,
            exported: false,
        },
        debug_path: source,
        crate_name: "generated".to_owned(),
        order: None,
        map: ItemMap::default(),
        dts: None,
        optimization: Default::default(),
        late_ir: Default::default(),
        baseline_shadow: Default::default(),
        jir: Default::default(),
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
    let names: Vec<&str> = names
        .into_iter()
        .map(|name| renames.get(name).map_or(name, String::as_str))
        .collect();
    match opts.minify {
        true => format!("export{{{}}};\n", names.join(",")),
        false => format!("export {{ {} }};\n", names.join(", ")),
    }
}

/// Registers a chunk's entry so an ES2018 loader can fetch it with a module script element.
///
/// Dynamic `import()` is ES2020 syntax. A module script still executes ordinary ES2018 module
/// syntax, and this shared map gives its loader the entry value after the script's load event.
fn chunk_registration(unit: &Unit<'_>, renames: &BTreeMap<String, String>) -> String {
    let Some(entry) = unit.entry else {
        return String::new();
    };
    let entry = renames.get(entry).map_or(entry, String::as_str);
    let registry = || jsast::member(jsast::id("globalThis"), "__rcgjsChunks");
    let registry = jsast::binary(
        jsast::BinOp::Or,
        registry(),
        jsast::assign(registry(), jsast::new(jsast::id("Map"), Vec::new())),
    );
    let register = jsast::call(
        jsast::member(registry, "set"),
        vec![jsast::string(&unit.label), jsast::id(entry)],
    );
    let style = match crate::opts::get().minify {
        true => jsast::Style::Compact,
        false => jsast::Style::Readable,
    };
    jsast::program_to_string_styled_for_target(
        &[jsast::expr_stmt(register)],
        style,
        crate::opts::get().ecma_target,
    )
}

/// Warns about a fixed name a minified local could shadow.
///
/// A fixed name keeps its exact spelling, so it is the one kind of name in a minified program that
/// the local renamer cannot see and therefore cannot avoid. It only avoids the identifiers a
/// function *mentions*: a function that calls `counter_clicked` will never name a local that, but a
/// function that does not call it could — if the name were short enough for the allocator to reach,
/// which for anything past three characters it never is (that needs more than 3445 distinct locals
/// in one function). So the hazard is real only for a very short `#[no_mangle]` name, and saying so
/// is better than silently narrowing what the flag may be used on. `names.rs` documents the same
/// hazard for readable local names, which have had it all along.
fn warn_short_fixed_names(dcx: &Sink, items: &[&LinkItem]) {
    let mut warned: BTreeSet<&str> = BTreeSet::new();
    for item in items {
        let Some(name) = &item.linkage.fixed_name else {
            continue;
        };
        if name.len() <= 3 && !name.starts_with('$') && warned.insert(name.as_str()) {
            dcx.warn(format!(
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
    dcx: &Sink,
    sources: &BTreeMap<String, String>,
    path: &Path,
    program: &str,
    items: &[&LinkItem],
    starts: &[u32],
) -> String {
    let file = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let map_name = format!("{file}.map");
    let map_path = path.with_file_name(&map_name);

    let json = crate::sourcemap::build(sources, program, items, starts, &file);
    if let Err(err) = fs::write(&map_path, json.as_bytes()) {
        dcx.err(format!("error writing `{}`: {err}", map_path.display()));
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
pub(crate) fn stub_missing(
    table: &BTreeMap<JsName, LinkItem>,
    reachable: &[JsName],
) -> Vec<LinkItem> {
    let mut missing: BTreeMap<JsName, LinkItem> = BTreeMap::new();
    for name in reachable {
        let Some(item) = table.get(name) else {
            continue;
        };
        for referenced in &item.refs {
            // The namer's 16-hex `$h` suffix is what makes an identifier certainly an item's
            // name rather than a local, a property or a JavaScript builtin. See `naming.rs`.
            if table.contains_key(referenced)
                || !is_namer_name(referenced.as_str())
                || missing.contains_key(referenced)
            {
                continue;
            }
            let js = format!(
                "\nfunction {name}() {{\n  throw new Error(\"rustc_codegen_js: `{name}` was \
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
                    crate_name: "generated".to_owned(),
                    order: None,
                    map: ItemMap::default(),
                    dts: None,
                    optimization: Default::default(),
                    late_ir: Default::default(),
                    baseline_shadow: Default::default(),
                    jir: Default::default(),
                },
            );
        }
    }
    missing.into_values().collect()
}

/// Whether `name` has the exact readable-name hash shape produced by `backend::naming`.
///
/// Checking for the substring `$h` is insufficient: physicalized locals such as `$v4$high` and
/// `acc$high` contain it too. Native versions retain the source hash and append `$n1`, optionally
/// followed by the collision suffix selected by `versions`.
pub(crate) fn is_namer_name(name: &str) -> bool {
    const HASH_DIGITS: usize = 16;

    if name.starts_with('$') {
        return false;
    }
    let Some((stem, rest)) = name.rsplit_once("$h") else {
        return false;
    };
    if stem.is_empty() || rest.len() < HASH_DIGITS {
        return false;
    }
    let Some(hash) = rest.as_bytes().get(..HASH_DIGITS) else {
        return false;
    };
    if !hash.iter().all(u8::is_ascii_hexdigit) {
        return false;
    }

    let suffix = &rest[HASH_DIGITS..];
    if suffix.is_empty() {
        return true;
    }
    let Some(version_and_collision) = suffix.strip_prefix("$n") else {
        return false;
    };
    let mut parts = version_and_collision.split('$');
    let Some(version) = parts.next() else {
        return false;
    };
    if version.is_empty() || !version.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    parts.all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Decodes the baseline graph and, for JIR lanes, its potential semantic dependencies.
///
/// Each verified body can reveal items absent from its baseline declaration, including cleanup
/// glue. Those items and their baseline support graphs are prepared before lowering. Final
/// emitted references decide which prepared items survive. Unrelated bodies remain lazy.
/// An invalid body is a rebuild error, never a silent fallback. `JSC_JIR_SELFCHECK=1` reports
/// the number of prepared items and decoded bodies.
fn decode_reachable_bodies(
    dcx: &Sink,
    table: &mut BTreeMap<JsName, LinkItem>,
    reachable: &mut Vec<JsName>,
    semantic: bool,
) {
    let mut decoded = 0usize;
    let mut with_body = 0usize;
    let mut prepared: BTreeSet<_> = reachable.iter().cloned().collect();
    let mut cursor = 0;
    while cursor < reachable.len() {
        let name = reachable[cursor].clone();
        cursor += 1;
        let Some(item) = table.get_mut(&name) else {
            continue;
        };
        if item.jir.has_body() {
            with_body += 1;
        }
        match item.jir.decode() {
            Ok(true) => decoded += 1,
            Ok(false) => {}
            Err(err) => {
                dcx.err(format!(
                    "rustc_codegen_js: {err}; rebuild the sysroot and dependencies \
                     (`scripts/build_sysroot.sh --clean`)"
                ));
            }
        }
        if semantic {
            let mut dependencies = item.refs.clone();
            if let (Some(body), Some(interns)) = (&item.jir.body, item.jir.interns()) {
                // Lowering can introduce runtime operations absent from baseline text, such as
                // the abort classifier on a cleanup edge. Prepare the namespace when defined;
                // final emitted references still decide whether its import survives.
                dependencies.insert(JsName::new(SHIM_NAMESPACE.to_owned()));
                dependencies.extend(
                    body.link_dependencies().into_iter().filter_map(|name| {
                        interns.str(name).map(|name| JsName::new(name.to_owned()))
                    }),
                );
            }
            for dependency in dependencies {
                if table.contains_key(&dependency) && prepared.insert(dependency.clone()) {
                    reachable.push(dependency);
                }
            }
        }
    }
    if cfg!(debug_assertions) {
        let reachable: BTreeSet<&JsName> = reachable.iter().collect();
        assert!(
            table
                .iter()
                .filter(|(name, _)| !reachable.contains(name))
                .all(|(_, item)| item.jir.body.is_none()),
            "an unreachable body was decoded"
        );
    }
    if std::env::var_os("JSC_JIR_SELFCHECK").is_some_and(|value| value == "1") {
        eprintln!(
            "rcgjs jir selfcheck: reachable items {}, with JIR body {with_body}, decoded {decoded}",
            reachable.len()
        );
    }
}

/// Baseline text cannot satisfy a semantic cleanup body. Check final references before text
/// deduplication can merge that text with an item carrying a different cleanup obligation.
fn reject_cleanup_fallbacks(
    dcx: &Sink,
    table: &BTreeMap<JsName, LinkItem>,
    reachable: &[JsName],
    run: Option<&lane::LaneRun>,
) {
    for name in reachable {
        let Some(body) = table.get(name).and_then(|item| item.jir.body.as_ref()) else {
            continue;
        };
        if !body.blocks.iter().any(|block| block.cleanup) {
            continue;
        }
        let outcome = run.and_then(|run| run.outcome(name));
        if outcome.is_some_and(|outcome| outcome.lane == crate::lane_report::ItemLane::Jir) {
            continue;
        }
        let mut diagnostic = dcx.struct_err(format!(
            "rustc_codegen_js: `{name}` {CLEANUP_LOWERING_REQUIRED}"
        ));
        if let Some(decline) = outcome.and_then(|outcome| outcome.decline.as_ref()) {
            diagnostic.note(format!("JIR lowering declined: {decline}"));
        }
        diagnostic.emit();
    }
}

/// One entry per name. Identical duplicates are dropped; conflicting ones are an error.
/// Writes `-Cllvm-args=js-lane-report=<path>`: one row per reachable item saying what the JIR
/// lane could own and, when it could not, why.
///
/// Rows are built after dedup and reachability, so the file describes the program that was
/// actually printed rather than everything the objects happened to contain. `bytes` is the printed
/// item's own length, which is the measure the optimizer report and `scripts/jir-coverage.sh`
/// both attribute to an item.
///
/// An item that carries a decoded body is `semantic`, not `available`: Phase 1 still prints from
/// the baseline lane, so having a body is not having produced JavaScript from it. Phase 2's lowerer
/// is what moves a row to `available`.
fn write_lane_report(
    dcx: &Sink,
    path: &Path,
    table: &BTreeMap<JsName, LinkItem>,
    reachable: &[JsName],
    lane_run: Option<&lane::LaneRun>,
) {
    let mut rows = Vec::with_capacity(reachable.len());
    for name in reachable {
        let Some(item) = table.get(name) else {
            continue;
        };
        let mut row = match &item.jir.summary {
            Some(summary) => LaneItem::of(summary),
            // No summary at all: a format 19 object, or an item the link step synthesized.
            None => LaneItem {
                name: name.as_str().to_owned(),
                bytes: 0,
                availability: LaneAvailability::Unavailable,
                decline: Some(crate::jir::Decline::Encode(
                    crate::jir::decline::EncodeDecline::Synthesized,
                )),
                class: Default::default(),
                lane: ItemLane::Baseline,
            },
        };
        row.name = name.as_str().to_owned();
        row.bytes = item.js.len() as u64;
        // What the lane did with the item, when a lane ran. An item the driver never offered to
        // the lowerer keeps the row the summary produced: `semantic`, printed from the baseline.
        if let Some(outcome) = lane_run.and_then(|run| run.outcome(name)) {
            row.lane = outcome.lane;
            match outcome.lane {
                ItemLane::Jir => {
                    row.availability = LaneAvailability::Available;
                    row.decline = None;
                }
                ItemLane::Fallback | ItemLane::Baseline => {
                    row.availability = LaneAvailability::Semantic;
                    row.decline = outcome.decline.clone();
                }
            }
        }
        rows.push(row);
    }
    if let Err(err) = crate::lane_report::write(path, &rows) {
        dcx.err(format!("error writing `{}`: {err}", path.display()));
    }
}

/// One entry per name, under W4's identity rule.
///
/// v1 compared the two items' **printed text** and raised a hard error whenever it differed. That
/// asked the wrong question twice over: two objects compiled with different emit options differ in
/// every item's text (which [`Read::extend`] now catches once, against the header, with a message
/// that names the fix), and two crates that ran one item through different MIR pipelines print
/// bodies that mean the same thing.
///
/// So the identity is [`SemanticKey`] and the comparison is the normalized body hash:
///
/// * different Rust items under one JavaScript name — still a hard error, and now the only one;
/// * equal normalized bodies — merge, keeping either;
/// * a body on one side only — keep the side that has one;
/// * different normalized bodies — keep the one with cleanup edges when only one has them, else
///   the smaller hash, and record the choice for the autopsy.
///
/// Format 19 objects and link-synthesized items carry no summary; those fall back to v1's text
/// comparison, which is all there is to go on. That path disappears with format 19.
fn dedup(
    dcx: &Sink,
    items: Vec<LinkItem>,
    conflicts: &mut Vec<ConflictRecord>,
) -> BTreeMap<JsName, LinkItem> {
    let mut table: BTreeMap<JsName, LinkItem> = BTreeMap::new();
    for item in items {
        let Some(existing) = table.get_mut(&item.name) else {
            table.insert(item.name.clone(), item);
            continue;
        };
        match (existing.jir.summary.as_ref(), item.jir.summary.as_ref()) {
            (Some(left), Some(right)) => {
                let decision = crate::jir::summary::merge_duplicates(left, right);
                apply_duplicate(dcx, existing, item, decision, conflicts);
            }
            // No summary on one side: a format 19 object, or an item this link step made up.
            // v1's comparison is the only evidence there is.
            _ if existing.code() == item.code() => existing.merge_duplicate_metadata(&item),
            _ => report_collision(dcx, existing, &item),
        }
    }
    table
}

/// Whether a copy's producer recorded that it owes cleanup it could not lower: it has no JIR body,
/// and its baseline text carries the [`CLEANUP_LOWERING_REQUIRED`] zombie instead.
fn declined_cleanup(item: &LinkItem) -> bool {
    !item.jir.has_body()
        && item
            .zombies
            .iter()
            .any(|zombie| zombie.message.starts_with(CLEANUP_LOWERING_REQUIRED))
}

/// Whether a copy's JIR body has cleanup edges, by its summary.
fn cleanup_body(item: &LinkItem) -> bool {
    item.jir.summary.as_ref().is_some_and(|summary| {
        summary
            .class
            .contains(crate::jir::summary::ItemClass::UNWIND)
    })
}

/// Acts on one [`MergeDecision`], folding the loser's metadata into the winner either way.
///
/// A cleanup obligation survives deduplication. Between two bodies,
/// [`crate::jir::summary::merge_duplicates`] already keeps the one with cleanup edges. A copy that
/// owes cleanup but declined encoding ([`declined_cleanup`]) is kept, zombie and all, over a copy
/// whose body has no cleanup edges: the two were lowered from one Rust body, so the second came
/// from MIR that had lost its cleanup paths. If that copy is reachable its zombie fails the link,
/// which is the fail-closed outcome; nothing of the losing body is folded in, because
/// [`LinkItem::merge_duplicate_metadata`] would hand it the cleanup-free body.
fn apply_duplicate(
    dcx: &Sink,
    existing: &mut LinkItem,
    item: LinkItem,
    decision: MergeDecision,
    conflicts: &mut Vec<ConflictRecord>,
) {
    if !matches!(decision, MergeDecision::Error(_)) {
        let owes = |item: &LinkItem| declined_cleanup(item) || cleanup_body(item);
        if declined_cleanup(&item) && !owes(existing) {
            adopt_body(existing, item);
            return;
        }
        if declined_cleanup(existing) && !owes(&item) {
            return;
        }
    }
    match decision {
        MergeDecision::Merge => {
            existing.merge_physical_summary(&item);
            existing.merge_duplicate_metadata(&item);
        }
        MergeDecision::Available(Side::Left) => {
            existing.merge_duplicate_metadata(&item);
        }
        MergeDecision::Available(Side::Right) => {
            existing.merge_duplicate_metadata(&item);
            adopt_body(existing, item);
        }
        MergeDecision::Choose { keep, .. } => {
            conflicts.push(duplicate_conflict(existing, &item));
            existing.merge_duplicate_metadata(&item);
            if keep == Side::Right {
                adopt_body(existing, item);
            }
        }
        MergeDecision::Error(_) => report_collision(dcx, existing, &item),
    }
}

/// Replaces the item's body with the duplicate's, and its printed text with it.
///
/// An item's JavaScript and its JIR body are two spellings of one thing: taking one without the
/// other would print a body the summary does not describe. What does **not** move is everything
/// the *link step* decided about the name — linkage above all. Objects are read before rlibs and
/// an rlib's items are demoted as they are read, so adopting a whole `LinkItem` from an rlib would
/// quietly un-root the program's entry point.
fn adopt_body(existing: &mut LinkItem, winner: LinkItem) {
    existing.js = winner.js;
    existing.refs = winner.refs;
    existing.map = winner.map;
    existing.zombies = winner.zombies;
    existing.jir = winner.jir;
    if existing.dts.is_none() {
        existing.dts = winner.dts;
    }
}

/// The autopsy record for two duplicates whose bodies differ. Both are decoded here — the only
/// place the link step inflates a body it has not proved reachable, and only for the pairs the
/// rule actually had to choose between.
fn duplicate_conflict(existing: &LinkItem, item: &LinkItem) -> ConflictRecord {
    let entry = |item: &LinkItem| conflict::ConflictEntry {
        origin: item.crate_name.clone(),
        path: item.debug_path.clone(),
        blob: item
            .jir
            .summary
            .as_ref()
            .and_then(|summary| summary.body_hash),
        normalized: item
            .jir
            .summary
            .as_ref()
            .and_then(|summary| summary.semantic_hash),
    };
    let entries = vec![entry(existing), entry(item)];
    let name = existing.name.as_str().to_owned();
    let normalized = |item: &LinkItem| {
        let mut jir = item.jir.clone();
        jir.decode().ok()?;
        let body = jir.body.as_ref()?;
        let interns = jir.interns()?;
        Some((
            crate::jir::summary::NormalizedBody::of(body, interns),
            jir.origins.clone(),
        ))
    };
    match (normalized(existing), normalized(item)) {
        (Some((left, left_origins)), Some((right, right_origins))) => {
            let origins = left_origins.as_ref().zip(right_origins.as_ref());
            ConflictRecord::of(&name, "duplicate", &left, &right, origins).with_entries(entries)
        }
        _ => ConflictRecord {
            name,
            relation: "duplicate",
            verdict: conflict::Verdict::Different,
            cause: conflict::Cause::Undecodable,
            difference: Some("a body did not decode".to_owned()),
            entries,
        },
    }
}

/// v1's "two different definitions" error, which now fires only for a real name collision.
fn report_collision(dcx: &Sink, existing: &LinkItem, item: &LinkItem) {
    let mut diag = dcx.struct_err(format!(
        "rustc_codegen_js: two different definitions of `{}` (`{}` and `{}`) \
         reached the same program",
        item.name, existing.debug_path, item.debug_path
    ));
    // By far the likeliest cause, and one whose message is otherwise baffling: the two crates that
    // codegenned this instantiation printed it under different backend options. Every option that
    // changes an item's *text* — `js-minify`, `js-structure`, `js-queue` — has to be the same for
    // the whole program, the sysroot's `core` included.
    diag.note(
        "every object in a program must be compiled with the same `-Cllvm-args=js-*` \
         options; rebuild the sysroot with the ones this crate uses \
         (`JS_EXTRA_ARGS='...' scripts/build_sysroot.sh`)",
    );
    diag.emit();
}

/// Breadth-first search from the roots. Returns the reachable names and, for each, the item that
/// first reached it.
fn reach(dcx: &Sink, table: &BTreeMap<JsName, LinkItem>) -> (Vec<JsName>, HashMap<JsName, JsName>) {
    let extra_roots = &crate::opts::get().roots;
    for root in extra_roots {
        if !table.contains_key(&JsName::new(root.clone())) {
            dcx.warn(format!(
                "`-Cllvm-args=js-root={root}` names no item in this program"
            ));
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
        let Some(item) = table.get(&name) else {
            continue;
        };
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
fn write_chunks(
    dcx: &Sink,
    sources: &BTreeMap<String, String>,
    table: &BTreeMap<JsName, LinkItem>,
    path: &Path,
    deployment: &mut ArtifactManifest,
) {
    let opts = crate::opts::get();

    let mut plans: Vec<ChunkPlan<'_>> = Vec::with_capacity(opts.chunks.len());
    for (name, entry) in &opts.chunks {
        let Some(root) = entry_item(table, entry) else {
            dcx.err(format!(
                "`-Cllvm-args=js-chunk={name}:{entry}` names no item in this program; the entry \
                 point of a chunk is an exported item's name, spelled exactly"
            ));
            continue;
        };
        let (reachable, parents) = reach_from(table, std::slice::from_ref(root));
        plans.push(ChunkPlan {
            name,
            root,
            reachable,
            parents,
        });
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
            let Some((name, item)) = table.get_key_value(name) else {
                continue;
            };
            if item.kind == ItemKind::Import {
                continue;
            }
            owner
                .entry(name)
                .and_modify(|at| *at = if *at == index { index } else { shared })
                .or_insert(index);
        }
    }
    for (index, plan) in plans.iter().enumerate() {
        owner.insert(plan.root, index);
    }

    // Zombies once each, reported by the first chunk that reaches the item, so that the chain
    // walks back to an entry point the caller asked for.
    let mut reported = BTreeSet::new();
    for plan in &plans {
        report_zombies(
            dcx,
            table,
            &plan.reachable,
            &plan.parents,
            Some(plan.name),
            &mut reported,
        );
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
            let Some(item) = table.get(name) else {
                continue;
            };
            for referenced in &item.refs {
                let Some(&from) = owner.get(referenced) else {
                    continue;
                };
                if from == at {
                    continue;
                }
                let name = table
                    .get_key_value(referenced)
                    .map_or(referenced.as_str(), |(name, _)| name.as_str());
                imported[at].entry(from).or_default().insert(name);
                exported[from].insert(name);
            }
        }
    }

    let label = |at: usize| match at == shared {
        true => opts.shared_chunk.as_str(),
        false => plans[at].name,
    };
    // No module level initializer may read a binding around a cycle of chunk imports, which only
    // a `static` named as an entry point can create (`link/statics.rs`). A cycle of statics split
    // that way is reported first, in its own words.
    let errors = dcx.errors();
    statics::split_cycles(dcx, table, &owner, |at| label(at).to_owned());
    if dcx.errors() == errors {
        statics::evaluation_cycles(dcx, table, &names, &owner, &imported, |at| {
            label(at).to_owned()
        });
    }

    // Each unit's bindings in initialization order, over the names it owns and only those: a
    // binding another unit owns arrives through the `import` above it, and placing it here as well
    // would declare one name twice, a `SyntaxError` before the module runs. A cycle of statics is
    // always owned by one unit, and is initialized in two steps there (`link/statics.rs`).
    let placed: Vec<Vec<Placed<'_>>> = names.iter().map(|owned| layout(table, owned)).collect();
    let cycles: Vec<Vec<Vec<LinkItem>>> = placed
        .iter()
        .map(|placed| statics::initialize_cycles(dcx, placed))
        .collect();

    // The synthesized imports and the stubs are owned here, because a unit borrows its items.
    let crossings: Vec<Vec<LinkItem>> = imported
        .iter()
        .map(|from| {
            from.iter()
                .map(|(at, names)| cross_chunk_import(label(*at), names))
                .collect()
        })
        .collect();
    let stubs: Vec<Vec<LinkItem>> = names
        .iter()
        .map(|owned| stub_missing(table, owned))
        .collect();

    let mut units = Vec::with_capacity(shared + 1);
    for (at, owned) in names.iter().enumerate() {
        // The shared chunk is written only when there is something to share: its absence is how
        // mires-build reads "two chunks had nothing in common".
        if at == shared && owned.is_empty() {
            continue;
        }
        let mut items = statics::print_order(&placed[at], &cycles[at]);
        items.splice(0..0, crossings[at].iter());
        items.extend(stubs[at].iter());
        units.push(Unit {
            label: label(at).to_owned(),
            path: Some(path.with_file_name(format!("{}.js", label(at)))),
            items,
            exports: exported[at].clone(),
            entry: if opts.chunk_registry {
                plans.get(at).map(|plan| plan.root.as_str())
            } else {
                None
            },
        });
    }

    for (unit, printed) in units.iter().zip(print_units(dcx, sources, &units)) {
        let Some(path) = &unit.path else { continue };
        if let Err(err) = fs::write(path, printed.text.as_bytes()) {
            dcx.err(format!("error writing `{}`: {err}", path.display()));
        }
        deployment.artifacts.extend(printed.artifacts);
        let roots = plans
            .iter()
            .find(|plan| plan.name == unit.label)
            .and_then(|plan| table.get(plan.root))
            .map(|root| BTreeMap::from([(root.name.to_string(), root_identity(root))]))
            .unwrap_or_default();
        let role = if unit.label == opts.shared_chunk {
            EntryRole::Shared
        } else {
            opts.chunk_roles
                .iter()
                .find(|(name, _)| *name == unit.label)
                .map(|(_, role)| *role)
                .unwrap_or(EntryRole::Main)
        };
        deployment.entries.insert(
            unit.label.clone(),
            Entry {
                role,
                program: path.file_name().unwrap().to_string_lossy().into_owned(),
                roots,
            },
        );
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
    let named: Vec<(String, String)> = names
        .iter()
        .map(|name| ((*name).to_owned(), (*name).to_owned()))
        .collect();
    let decl = jsast::import_named(named, source.clone());
    let style = match crate::opts::get().minify {
        true => jsast::Style::Compact,
        false => jsast::Style::Readable,
    };
    LinkItem {
        name: JsName::new(format!("$chunk${chunk}")),
        kind: ItemKind::Import,
        js: jsast::program_to_string_styled_for_target(
            std::slice::from_ref(&decl),
            style,
            crate::opts::get().ecma_target,
        ),
        refs: Default::default(),
        zombies: Vec::new(),
        linkage: Linkage::internal(),
        debug_path: source,
        crate_name: "generated".to_owned(),
        order: None,
        map: ItemMap::default(),
        dts: None,
        optimization: Default::default(),
        late_ir: Default::default(),
        baseline_shadow: Default::default(),
        jir: Default::default(),
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
        let Some(item) = table.get(*name) else {
            continue;
        };
        for referenced in &item.refs {
            if table
                .get(referenced)
                .is_some_and(|item| item.kind == ItemKind::Import)
            {
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
    dcx: &Sink,
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
        let Some(item) = table.get(name) else {
            continue;
        };
        if item.zombies.is_empty() || !reported.insert(name.clone()) {
            continue;
        }
        for zombie in &item.zombies {
            let mut diag = dcx.struct_err(format!("{}: {}", zombie.loc, zombie.message));
            diag.note(format!("in `{}`", item.debug_path));
            if let Some(chunk) = chunk {
                diag.note(format!("in the chunk `{chunk}`"));
            }
            // Walk back to the root, so the report says why this item is in the program at all.
            let mut current = name;
            let mut steps = 0;
            while let Some(parent) = parents.get(current) {
                let Some(parent_item) = table.get(parent) else {
                    break;
                };
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
/// names another binding has to come after it. The dependency order among bindings is a
/// depth-first post-order over `refs`. Rust allows statics that name each other, and no order of
/// plain `let`s initializes those; [`layout`] keeps such a cycle together, and the printed program
/// initializes it in two steps (`link/statics.rs`). Here, for the measurements that only count
/// bytes, a cycle is listed member by member.
///
/// Only the items of `reachable` are placed. The walk that orders a binding after the bindings it
/// names follows `refs`, and in a chunk (see [`write_chunks`]) some of those belong to another file
/// and arrive through its `import` clause: following them into this file as well would declare one
/// name twice, which is a `SyntaxError` before the module runs.
///
/// A binding carrying a [`SourceOrder`] is placed before the rest and among its own kind in that
/// order. `view!` template cloners are what need it: a trace numbers them in declaration order, so
/// a program that declares the same cloners in another order is observably different from the
/// reference one. Nothing else is disturbed, because such a binding's initializer reads only
/// imports.
pub(crate) fn order<'a>(
    table: &'a BTreeMap<JsName, LinkItem>,
    reachable: &'a [JsName],
) -> Vec<&'a LinkItem> {
    let mut out = Vec::with_capacity(reachable.len());
    for step in layout(table, reachable) {
        match step {
            Placed::Item(item) => out.push(item),
            Placed::Cycle(members) => out.extend(members),
        }
    }
    out
}

/// [`order`], with each cycle among the bindings kept together as one [`Placed::Cycle`].
fn layout<'a>(table: &'a BTreeMap<JsName, LinkItem>, reachable: &'a [JsName]) -> Vec<Placed<'a>> {
    let members: std::collections::HashSet<&JsName> = reachable.iter().collect();
    let mut names: Vec<&JsName> = reachable.iter().collect();
    names.sort();
    // The ordered bindings first, in their own order; the rest keep the name order above. The sort
    // is stable, which is what leaves everything without an order where it already was.
    names.sort_by_key(|name| {
        let order = table.get(*name).and_then(|item| item.order.clone());
        (order.is_none(), order)
    });

    let mut out: Vec<Placed<'a>> = Vec::with_capacity(names.len());

    let mut imports: Vec<&LinkItem> = names
        .iter()
        .filter_map(|name| table.get(*name))
        .filter(|item| item.kind == ItemKind::Import)
        .collect();
    imports.sort_by(|a, b| (&a.debug_path, &a.name).cmp(&(&b.debug_path, &b.name)));
    out.extend(imports.into_iter().map(Placed::Item));

    out.extend(statics::layout(table, &members, &names));
    for name in &names {
        if let Some(item) = table.get(*name) {
            if !item.kind.is_binding() && item.kind != ItemKind::Import {
                out.push(Placed::Item(item));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_fallback_rejection_uses_final_references_before_text_deduplication() {
        use crate::jir::semantic::*;

        let mut entry = duplicate("entry", "function entry(){}", None, None);
        let mut cleanup = duplicate("cleanup", "function cleanup(){}", None, None);
        let body = Body {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash: 1,
                },
                instantiation: 0,
            },
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: vec![],
                result: TypeId(0),
                abi: Default::default(),
            },
            locals: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                cleanup: true,
                statements: vec![],
                terminator: Terminator::UnwindResume,
            }],
        };
        cleanup.jir.body = Some(body);
        let roots = vec![entry.name.clone()];
        let empty_run = lane::LaneRun::default();
        for run in [None, Some(&empty_run)] {
            for retained in [false, true] {
                entry.refs.clear();
                if retained {
                    entry.refs.insert(cleanup.name.clone());
                }
                let table = BTreeMap::from([
                    (entry.name.clone(), entry.clone()),
                    (cleanup.name.clone(), cleanup.clone()),
                ]);
                let (reachable, _) = reach_from(&table, &roots);
                let dcx = Sink::new();
                reject_cleanup_fallbacks(&dcx, &table, &reachable, run);
                assert_eq!(dcx.errors(), u32::from(retained));
            }
        }
    }

    #[test]
    fn semantic_preparation_follows_lazy_objects_without_retaining_unused_edges() {
        use crate::item::{JirItem, JsItem, ObjectIdentity};
        use crate::jir::interns::Interns;
        use crate::jir::origin::OriginTable;
        use crate::jir::semantic::*;
        use crate::jir::types::*;

        // Independent objects deliberately use different string ID orders.
        let object = |name: &str, edges: &[&str], baseline: &[&str]| {
            let mut interns = Interns::new();
            let item_path = interns.intern_str(name);
            let ty = TypeId(1);
            interns.intern_type(TypeDescriptor {
                id: ty,
                name: None,
                kind: TypeKind::Tuple(vec![]),
                layout: LayoutFacts {
                    size: Some(0),
                    unsized_prefix: None,
                    align: Some(1),
                    sized: true,
                    zst: true,
                    one_zst: true,
                    needs_drop: false,
                    has_own_drop: false,
                },
                meta: Default::default(),
                js: JsFacts {
                    int: None,
                    is_indirect: true,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: ty,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: vec![],
                },
            });
            let id = ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash: 0,
                },
                instantiation: 0,
            };
            let body = Body {
                item: id,
                kind: BodyKind::Vtable {
                    concrete: ty,
                    trait_ref: None,
                    entries: edges
                        .iter()
                        .map(|edge| VtableEntry::TraitVPtr(interns.intern_str(edge)))
                        .collect(),
                },
                semantic: BodySemantic::Ordinary,
                signature: Signature {
                    arguments: vec![],
                    result: ty,
                    abi: Default::default(),
                },
                locals: vec![],
                blocks: vec![],
            };
            let origins = OriginTable {
                origins: vec![],
                statements: vec![],
                terminators: vec![],
                locals: vec![],
                item_path,
                root: None,
            };
            let item = JsItem::new(
                JsName::new(name.to_owned()),
                ItemKind::Vtable,
                crate::jsast::const_(
                    name,
                    crate::jsast::array(
                        baseline
                            .iter()
                            .map(|name| crate::jsast::id(*name))
                            .collect(),
                    ),
                ),
                vec![],
                Linkage::internal(),
                name.to_owned(),
            )
            .with_jir(Some(JirItem { body, origins }), None);
            let bytes = crate::item::write_object(
                "",
                &[item],
                false,
                name,
                &ObjectIdentity::default(),
                interns,
                &BTreeMap::new(),
            );
            crate::item::read_object(&bytes)
                .unwrap()
                .unwrap()
                .items
                .pop()
                .unwrap()
        };
        let items = [
            object("entry", &["cleanup"], &[]),
            object("__rt", &[], &[]),
            object("cleanup", &["missing", "entry"], &["support"]),
            object("support", &[], &[]),
            object("unused", &["entry"], &[]),
        ];
        let original: BTreeMap<_, _> = items
            .into_iter()
            .map(|item| (item.name.clone(), item))
            .collect();
        let roots = vec![JsName::new("entry".to_owned())];
        for semantic in [false, true] {
            let dcx = Sink::new();
            let mut table = original.clone();
            assert!(table.values().all(|item| item.jir.body.is_none()));
            let (mut prepared, _) = reach_from(&table, &roots);
            decode_reachable_bodies(&dcx, &mut table, &mut prepared, semantic);
            assert_eq!(dcx.errors(), 0);
            let expected = if semantic {
                vec!["entry", "__rt", "cleanup", "support"]
            } else {
                vec!["entry"]
            };
            assert_eq!(
                prepared.iter().map(JsName::as_str).collect::<Vec<_>>(),
                expected
            );
            for (name, item) in &table {
                assert_eq!(item.jir.body.is_some(), prepared.contains(name));
                assert_eq!(
                    item.refs, original[name].refs,
                    "preparing cannot change printed references"
                );
            }
            assert_eq!(
                reach_from(&table, &roots).0,
                roots,
                "unused semantic candidates must be pruned"
            );
            if semantic {
                table
                    .get_mut(&roots[0])
                    .unwrap()
                    .refs
                    .insert(JsName::new("cleanup".to_owned()));
                assert_eq!(
                    reach_from(&table, &roots).0,
                    prepared.iter().filter(|name| name.as_str() != "__rt")
                        .cloned().collect::<Vec<_>>(),
                    "a retained cleanup keeps its support graph without retaining an unused runtime"
                );
            }
        }
    }

    #[test]
    fn conflicting_object_sources_are_rejected_only_for_maps() {
        let object = |text: &str| {
            let bytes = crate::item::write_object(
                "",
                &[],
                false,
                "dependency",
                &crate::item::ObjectIdentity::default(),
                crate::jir::interns::Interns::new(),
                &BTreeMap::from([("src/lib.rs".to_owned(), text.to_owned())]),
            );
            crate::item::read_object(&bytes).unwrap().unwrap()
        };
        for source_map in [false, true] {
            let dcx = Sink::new();
            let mut read = Read {
                source_map,
                ..Read::default()
            };
            read.extend(&dcx, "first", object("first source"));
            read.extend(&dcx, "same", object("first source"));
            assert_eq!(dcx.errors(), 0);
            read.extend(&dcx, "different", object("different source"));
            assert_eq!(dcx.errors(), u32::from(source_map));
            if source_map {
                assert_eq!(read.sources["src/lib.rs"], "first source");
            } else {
                assert!(read.sources.is_empty());
            }
        }
    }

    use crate::item::{Availability, ItemSummary, SemanticKey};
    use crate::jir::hash::Hash128;
    use crate::jir::semantic::{DefinitionId, ItemId};

    #[test]
    fn namer_shape_requires_the_exact_hash_suffix() {
        assert!(is_namer_name("crate$item$h0123456789abcdef"));
        assert!(is_namer_name("crate$item$h0123456789abcdef$n1"));
        assert!(is_namer_name("crate$item$h0123456789abcdef$n1$2"));
        assert!(!is_namer_name("$v4$high"));
        assert!(!is_namer_name("acc$high"));
        assert!(!is_namer_name("crate$item$h0123"));
        assert!(!is_namer_name("crate$item$h0123456789abcdef$other"));
    }

    fn duplicate(name: &str, js: &str, item: Option<u64>, hash: Option<u64>) -> LinkItem {
        let summary = ItemSummary {
            key: SemanticKey {
                link_name: name.to_owned(),
                item: item.map(|local_hash| ItemId {
                    definition: DefinitionId {
                        crate_id: 1,
                        local_hash,
                    },
                    instantiation: u128::from(local_hash),
                }),
            },
            body_hash: hash.map(|lo| Hash128 { lo, hi: 0 }),
            semantic_hash: hash.map(|lo| Hash128 { lo, hi: 0 }),
            availability: Availability::Available,
            deps: Vec::new(),
            abi: None,
            physical: None,
            kind: ItemKind::Fn,
            linkage: Linkage::internal(),
            order: None,
            dts: false,
            fallback: Default::default(),
            body: None,
            origin: None,
            estimated_bytes: js.len() as u32,
            class: Default::default(),
        };
        LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            js: js.to_owned(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: format!("path::{name}"),
            crate_name: "test".to_owned(),
            order: None,
            map: ItemMap::default(),
            dts: None,
            optimization: Default::default(),
            late_ir: Default::default(),
            baseline_shadow: Default::default(),
            jir: crate::item::LinkJir::from_summary(summary),
        }
    }

    /// The case v1 declined on and this rule merges: one item, two objects, bodies that normalize
    /// to the same thing even though their encoded bytes (and, here, their printed text) differ.
    #[test]
    fn one_item_from_two_objects_merges_on_the_normalized_hash() {
        let dcx = Sink::new();
        let mut conflicts = Vec::new();
        let table = dedup(
            &dcx,
            vec![
                duplicate("f", "function f(){/*a*/}", Some(1), Some(7)),
                duplicate("f", "function f(){/*b*/}", Some(1), Some(7)),
            ],
            &mut conflicts,
        );
        assert_eq!(dcx.errors(), 0, "equal bodies are never an error");
        assert_eq!(table.len(), 1);
        assert!(conflicts.is_empty(), "nothing had to be chosen");
    }

    /// Two bodies for one item: a choice recorded for the autopsy, never a lost item and never an
    /// error. The smaller normalized hash wins whichever order the objects were read in.
    #[test]
    fn two_bodies_for_one_item_choose_the_smaller_hash_either_way_round() {
        for (first, second) in [(9u64, 4u64), (4, 9)] {
            let dcx = Sink::new();
            let mut conflicts = Vec::new();
            let table = dedup(
                &dcx,
                vec![
                    duplicate("f", "function f(){/*a*/}", Some(1), Some(first)),
                    duplicate("f", "function f(){/*b*/}", Some(1), Some(second)),
                ],
                &mut conflicts,
            );
            assert_eq!(dcx.errors(), 0, "a body difference is not an error");
            let kept = table[&JsName::new("f".to_owned())]
                .jir
                .summary
                .as_ref()
                .and_then(|summary| summary.semantic_hash)
                .expect("a body survived");
            assert_eq!(kept, Hash128 { lo: 4, hi: 0 });
            assert_eq!(conflicts.len(), 1);
            assert_eq!(conflicts[0].relation, "duplicate");
        }
    }

    /// The one case that is still a hard error: one JavaScript name over two Rust items.
    #[test]
    fn one_name_over_two_items_is_still_an_error() {
        let dcx = Sink::new();
        let mut conflicts = Vec::new();
        dedup(
            &dcx,
            vec![
                duplicate("f", "function f(){}", Some(1), Some(1)),
                duplicate("f", "function f(){}", Some(2), Some(1)),
            ],
            &mut conflicts,
        );
        assert_eq!(dcx.errors(), 1);
    }

    /// A copy of `name` with a body (`hash`), with or without cleanup edges in its summary.
    fn body_copy(name: &str, hash: u64, cleanup: bool) -> LinkItem {
        let mut copy = duplicate(
            name,
            &format!("function {name}(){{/*{hash}*/}}"),
            Some(1),
            Some(hash),
        );
        let summary = copy.jir.summary.as_mut().expect("a summary");
        summary.body = Some(Default::default());
        if cleanup {
            summary.class.insert(crate::jir::summary::ItemClass::UNWIND);
        }
        copy
    }

    /// A copy of `name` that owes cleanup but declined encoding, as the producer writes it.
    fn declined_cleanup_copy(name: &str) -> LinkItem {
        let mut copy = duplicate(
            name,
            &format!("function {name}(){{/*declined*/}}"),
            Some(1),
            None,
        );
        copy.zombies.push(crate::item::FlatZombie {
            loc: "src/lib.rs:1:1".to_owned(),
            message: format!("{CLEANUP_LOWERING_REQUIRED} (encoding: None)"),
            kind: crate::item::ZombieKind::Unsupported,
        });
        copy
    }

    fn kept(table: &BTreeMap<JsName, LinkItem>, name: &str) -> LinkItem {
        table[&JsName::new(name.to_owned())].clone()
    }

    /// The copy lowered from MIR that kept its cleanup paths survives even when the copy lowered
    /// from MIR that lost them (rustc's optimized MIR under `-Cpanic=abort`) has the smaller hash,
    /// whichever order the objects were read in. With cleanup on both sides the smaller hash wins.
    #[test]
    fn a_copy_with_cleanup_edges_beats_a_smaller_hash_without_them() {
        for (cleanup_low, cleanup_high, expected) in [
            (false, true, 9u64),
            (true, false, 4),
            (true, true, 4),
            (false, false, 4),
        ] {
            for reversed in [false, true] {
                let dcx = Sink::new();
                let mut conflicts = Vec::new();
                let mut items = vec![
                    body_copy("f", 4, cleanup_low),
                    body_copy("f", 9, cleanup_high),
                ];
                if reversed {
                    items.reverse();
                }
                let table = dedup(&dcx, items, &mut conflicts);
                assert_eq!(dcx.errors(), 0);
                assert_eq!(conflicts.len(), 1);
                let summary = kept(&table, "f").jir.summary.expect("a summary");
                assert_eq!(
                    summary.semantic_hash,
                    Some(Hash128 {
                        lo: expected,
                        hi: 0
                    })
                );
            }
        }
    }

    /// A copy that owes cleanup but has no body is kept, zombie and all, over a body without
    /// cleanup edges, so a reachable use fails the link instead of losing its destructors. A body
    /// with cleanup edges is the faithful copy and is kept over both.
    #[test]
    fn a_declined_cleanup_copy_is_not_replaced_by_a_body_without_cleanup() {
        let orders = |items: Vec<LinkItem>| {
            let mut reversed = items.clone();
            reversed.reverse();
            [items, reversed]
        };
        for items in orders(vec![declined_cleanup_copy("f"), body_copy("f", 3, false)])
            .into_iter()
            .chain(orders(vec![
                declined_cleanup_copy("f"),
                duplicate("f", "function f(){}", Some(1), None),
            ]))
        {
            let dcx = Sink::new();
            let table = dedup(&dcx, items, &mut Vec::new());
            let f = kept(&table, "f");
            assert!(
                !f.jir.has_body(),
                "the cleanup-free body must not be adopted"
            );
            assert!(f.zombies[0].message.starts_with(CLEANUP_LOWERING_REQUIRED));
            let (reachable, parents) = reach_from(&table, std::slice::from_ref(&f.name));
            report_zombies(
                &dcx,
                &table,
                &reachable,
                &parents,
                None,
                &mut BTreeSet::new(),
            );
            assert_eq!(
                dcx.errors(),
                1,
                "a reachable copy that owes cleanup fails the link"
            );
        }
        for items in orders(vec![
            declined_cleanup_copy("f"),
            body_copy("f", 3, true),
            body_copy("f", 2, false),
        ]) {
            let dcx = Sink::new();
            let table = dedup(&dcx, items, &mut Vec::new());
            let f = kept(&table, "f");
            assert_eq!(dcx.errors(), 0);
            assert!(f.zombies.is_empty());
            assert_eq!(
                f.jir.summary.and_then(|summary| summary.semantic_hash),
                Some(Hash128 { lo: 3, hi: 0 })
            );
        }
    }

    /// An item encoded in one crate and declined in another keeps the body that exists, with no
    /// decline of its own: that is ordinary, not a conflict.
    #[test]
    fn a_body_on_one_side_only_is_kept_without_a_conflict() {
        for order in [false, true] {
            let dcx = Sink::new();
            let mut conflicts = Vec::new();
            let with = duplicate("f", "function f(){}", Some(1), Some(3));
            let without = duplicate("f", "function f(){}", Some(1), None);
            let items = match order {
                false => vec![with, without],
                true => vec![without, with],
            };
            let table = dedup(&dcx, items, &mut conflicts);
            assert_eq!(dcx.errors(), 0);
            assert!(conflicts.is_empty());
            assert_eq!(
                table[&JsName::new("f".to_owned())]
                    .jir
                    .summary
                    .as_ref()
                    .and_then(|summary| summary.semantic_hash),
                Some(Hash128 { lo: 3, hi: 0 })
            );
        }
    }

    /// A chunk's `order` places what the chunk owns and nothing it imports: a binding that names a
    /// binding another chunk owns comes after its `import`, and the import is not declared again.
    #[test]
    fn order_places_only_the_items_it_is_given() {
        let binding = |name: &str, refs: &[&str]| {
            let mut item = duplicate(name, &format!("let {name};\n"), None, None);
            item.kind = ItemKind::Static;
            item.refs = refs.iter().map(|r| JsName::new((*r).to_owned())).collect();
            item
        };
        let table: BTreeMap<JsName, LinkItem> = [
            binding("shared_table", &[]),
            binding("own_index", &["shared_table"]),
            binding("own_other", &["own_index"]),
        ]
        .into_iter()
        .map(|item| (item.name.clone(), item))
        .collect();
        let whole: Vec<JsName> = table.keys().cloned().collect();
        let placed: Vec<&str> = order(&table, &whole)
            .iter()
            .map(|item| item.name.as_str())
            .collect();
        assert_eq!(placed, ["shared_table", "own_index", "own_other"]);
        let chunk = [
            JsName::new("own_other".to_owned()),
            JsName::new("own_index".to_owned()),
        ];
        let placed: Vec<&str> = order(&table, &chunk)
            .iter()
            .map(|item| item.name.as_str())
            .collect();
        assert_eq!(placed, ["own_index", "own_other"]);
    }

    #[test]
    fn track_caller_export_gets_byte_string_facade_with_default_and_wrapped_location() {
        use crate::jir::interns::Interns;
        use crate::jir::semantic::*;
        use crate::jir::types::*;
        use rcgjs_object::item::LinkJir;

        let mut interns = Interns::new();
        let loc_name = interns.intern_str("Location");
        let filename_key = interns.intern_str("filename");
        let line_key = interns.intern_str("line");
        let col_key = interns.intern_str("col");

        let make_desc = |id: u128, kind: TypeKind| TypeDescriptor {
            id: TypeId(id),
            name: None,
            kind,
            layout: LayoutFacts::default(),
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: TypeId(id),
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        };

        let str_ty = TypeId(1);
        let str_ref_ty = TypeId(2);
        let u32_ty = TypeId(3);
        let loc_ty = TypeId(4);
        interns.types = TypeTable::from(vec![
            make_desc(str_ty.0, TypeKind::Str),
            make_desc(
                str_ref_ty.0,
                TypeKind::Reference {
                    pointee: str_ty,
                    mutable: false,
                },
            ),
            make_desc(
                u32_ty.0,
                TypeKind::Integer {
                    signed: false,
                    bits: 32,
                },
            ),
            make_desc(
                loc_ty.0,
                TypeKind::Adt {
                    def: DefinitionId {
                        crate_id: 1,
                        local_hash: 42,
                    },
                    args: Vec::new(),
                    form: NominalForm::Struct,
                    variants: vec![VariantDescriptor {
                        name: loc_name,
                        fields: vec![
                            FieldDescriptor {
                                key: FieldKey::Name(filename_key),
                                ty: str_ref_ty,
                                offset: 0,
                            },
                            FieldDescriptor {
                                key: FieldKey::Name(line_key),
                                ty: u32_ty,
                                offset: 4,
                            },
                            FieldDescriptor {
                                key: FieldKey::Name(col_key),
                                ty: u32_ty,
                                offset: 8,
                            },
                        ],
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    }],
                    niche: None,
                },
            ),
        ]);

        let mut tc_item = duplicate(
            "tc_exported",
            "function tc_exported(s, $loc) { return s.length + $loc.filename.length; }",
            Some(1),
            Some(1),
        );
        tc_item.linkage = Linkage::exported("tc_exported".to_owned());
        tc_item.jir = LinkJir::default();
        tc_item.jir.use_interns(interns);
        tc_item.jir.body = Some(Body {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash: 1,
                },
                instantiation: 0,
            },
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: vec![str_ref_ty],
                result: u32_ty,
                abi: AbiSignature {
                    parameters: 2,
                    spread: None,
                    caller_location: true,
                    inline_never: false,
                },
            },
            locals: Vec::new(),
            blocks: Vec::new(),
        });

        let mut table = BTreeMap::from([(JsName::new("tc_exported".to_owned()), tc_item)]);
        let mut reachable = vec![JsName::new("tc_exported".to_owned())];
        let dcx = Sink::new();
        let moved = byte_string_facades(&dcx, &mut table, &mut reachable);
        assert_eq!(dcx.errors(), 0);
        assert_eq!(
            moved,
            vec![(
                JsName::new("tc_exported".to_owned()),
                JsName::new("tc_exported$bstr".to_owned())
            )]
        );
        let facade_js = &table[&JsName::new("tc_exported".to_owned())].js;
        assert!(facade_js.contains("a_loc"), "{facade_js}");
        assert!(facade_js.contains("<host>"), "{facade_js}");
        assert!(facade_js.contains("filename"), "{facade_js}");
        assert!(facade_js.contains("bstr_ref_in"), "{facade_js}");
    }

    #[test]
    fn track_caller_export_disambiguates_core_location_from_user_location_struct() {
        use crate::jir::interns::{Interns, StrId};
        use crate::jir::semantic::*;
        use crate::jir::types::*;
        use rcgjs_object::item::LinkJir;

        let mut interns = Interns::new();
        let loc_name = interns.intern_str("Location");
        let name_key = interns.intern_str("name");
        let x_key = interns.intern_str("x");
        let y_key = interns.intern_str("y");
        let filename_key = interns.intern_str("filename");
        let line_key = interns.intern_str("line");
        let col_key = interns.intern_str("col");
        let intrinsic_name = interns.intern_str("caller_location");

        let make_desc = |id: u128, kind: TypeKind| TypeDescriptor {
            id: TypeId(id),
            name: None,
            kind,
            layout: LayoutFacts::default(),
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: TypeId(id),
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        };

        let str_ty = TypeId(1);
        let str_ref_ty = TypeId(2);
        let u32_ty = TypeId(3);
        let user_loc_ty = TypeId(4);
        let user_loc_ref_ty = TypeId(5);
        let core_loc_ty = TypeId(6);
        let core_loc_ref_ty = TypeId(7);

        let make_loc_struct =
            |ty: u128, crate_id: u64, local_hash: u64, keys: (StrId, StrId, StrId)| {
                make_desc(
                    ty,
                    TypeKind::Adt {
                        def: DefinitionId {
                            crate_id,
                            local_hash,
                        },
                        args: Vec::new(),
                        form: NominalForm::Struct,
                        variants: vec![VariantDescriptor {
                            name: loc_name,
                            fields: vec![
                                FieldDescriptor {
                                    key: FieldKey::Name(keys.0),
                                    ty: str_ref_ty,
                                    offset: 0,
                                },
                                FieldDescriptor {
                                    key: FieldKey::Name(keys.1),
                                    ty: u32_ty,
                                    offset: 4,
                                },
                                FieldDescriptor {
                                    key: FieldKey::Name(keys.2),
                                    ty: u32_ty,
                                    offset: 8,
                                },
                            ],
                            logical_discr: 0,
                            physical_tag: None,
                            result_ok: false,
                            saved_local_keys: Vec::new(),
                        }],
                        niche: None,
                    },
                )
            };

        let mut str_ref_desc = make_desc(
            str_ref_ty.0,
            TypeKind::Reference {
                pointee: str_ty,
                mutable: false,
            },
        );
        str_ref_desc.js.pointer_form = Some(PointerForm::Str);

        let mut user_loc_desc = make_loc_struct(user_loc_ty.0, 7, 100, (name_key, x_key, y_key));
        user_loc_desc.js.is_indirect = true;
        user_loc_desc.js.fields = vec![
            FieldDescriptor {
                key: FieldKey::Name(name_key),
                ty: str_ref_ty,
                offset: 0,
            },
            FieldDescriptor {
                key: FieldKey::Name(x_key),
                ty: u32_ty,
                offset: 4,
            },
            FieldDescriptor {
                key: FieldKey::Name(y_key),
                ty: u32_ty,
                offset: 8,
            },
        ];

        let mut user_loc_ref_desc = make_desc(
            user_loc_ref_ty.0,
            TypeKind::Reference {
                pointee: user_loc_ty,
                mutable: false,
            },
        );
        user_loc_ref_desc.js.pointer_form = Some(PointerForm::Thin);

        let mut core_loc_desc =
            make_loc_struct(core_loc_ty.0, 99, 200, (filename_key, line_key, col_key));
        core_loc_desc.js.is_indirect = true;

        let mut core_loc_ref_desc = make_desc(
            core_loc_ref_ty.0,
            TypeKind::Reference {
                pointee: core_loc_ty,
                mutable: false,
            },
        );
        core_loc_ref_desc.js.pointer_form = Some(PointerForm::Thin);

        interns.types = TypeTable::from(vec![
            make_desc(str_ty.0, TypeKind::Str),
            str_ref_desc,
            make_desc(
                u32_ty.0,
                TypeKind::Integer {
                    signed: false,
                    bits: 32,
                },
            ),
            user_loc_desc,
            user_loc_ref_desc,
            core_loc_desc,
            core_loc_ref_desc,
        ]);

        let mut tc_item = duplicate(
            "tc_ambig",
            "function tc_ambig(u, $loc) { return u.name.length + $loc.filename.length; }",
            Some(1),
            Some(1),
        );
        tc_item.linkage = Linkage::exported("tc_ambig".to_owned());
        tc_item.jir = LinkJir::default();
        tc_item.jir.use_interns(interns);
        tc_item.jir.body = Some(Body {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 7,
                    local_hash: 1,
                },
                instantiation: 0,
            },
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: vec![user_loc_ref_ty],
                result: u32_ty,
                abi: AbiSignature {
                    parameters: 2,
                    spread: None,
                    caller_location: true,
                    inline_never: false,
                },
            },
            locals: vec![
                Local {
                    id: LocalId(0),
                    role: LocalRole::Return,
                    ty: u32_ty,
                },
                Local {
                    id: LocalId(1),
                    role: LocalRole::Argument,
                    ty: user_loc_ref_ty,
                },
                Local {
                    id: LocalId(2),
                    role: LocalRole::Temporary,
                    ty: core_loc_ref_ty,
                },
            ],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Call(Call {
                    function: Operand {
                        ty: TypeId(1),
                        kind: OperandKind::Constant(Constant::ZeroSized),
                    },
                    callee: Callee::Intrinsic {
                        name: intrinsic_name,
                        types: Vec::new(),
                        fallback: None,
                    },
                    arguments: Vec::new(),
                    destination: Place {
                        local: LocalId(2),
                        projection: Vec::new(),
                        ty: core_loc_ref_ty,
                    },
                    target: None,
                    unwind: Unwind::Continue,
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                }),
            }],
        });

        let mut table = BTreeMap::from([(JsName::new("tc_ambig".to_owned()), tc_item)]);
        let mut reachable = vec![JsName::new("tc_ambig".to_owned())];
        let dcx = Sink::new();
        let moved = byte_string_facades(&dcx, &mut table, &mut reachable);
        assert_eq!(dcx.errors(), 0);
        assert_eq!(moved.len(), 1);
        let facade_js = &table[&JsName::new("tc_ambig".to_owned())].js;
        assert!(facade_js.contains("filename:"), "{facade_js}");
        assert!(facade_js.contains("col: 1"), "{facade_js}");
        assert!(!facade_js.contains("file:"), "{facade_js}");
        assert!(!facade_js.contains("{ name:"), "{facade_js}");
    }
}
