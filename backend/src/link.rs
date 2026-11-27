//! The link seam: what the dylib knows, handed to `jsc-link`.
//!
//! There is no linker, and there is no link step in this crate any more either. A program is the
//! set of JavaScript items reachable from its roots, and working that out needs nothing but the
//! object files — which is why the whole of it (the graph walk, deduplication, chunking, the source
//! map, the `.d.ts`, the report, and the JIR optimizer) lives in the rustc-free
//! `jsc-link` crate, with a plain `cargo test` and no `rustc_*` dependency at all.
//!
//! What is left here is everything that only rustc can answer:
//!
//! * an rlib is not a program. It is a bag of objects for a later link to pick from, and rustc
//!   already knows how to build one, so [`link`] hands it straight to `link_binary`.
//! * which objects this crate just wrote, and which rlibs rustc resolved for it, in order
//!   ([`each_linked_rlib`]).
//! * where the output goes, and whether this compilation is the single-crate `--emit=obj` case
//!   ([`is_standalone_object`]) whose object *is* the program.
//! * the text of this crate's own sources, which the source map embeds and which only lives in the
//!   session's source map.
//!
//! Those go into a [`Manifest`], written to the temp directory, and `jsc-link --manifest <path>`
//! runs. Its stderr is this process's stderr, so its diagnostics reach the user unchanged; a
//! non-zero exit becomes one rustc error, because the messages have already been printed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use rustc_codegen_ssa::back::archive::ArArchiveBuilderBuilder;
use rustc_codegen_ssa::back::link::{each_linked_rlib, link_binary};
use rustc_codegen_ssa::diagnostics::LinkRlibError;
use rustc_codegen_ssa::{CompiledModule, CompiledModules, CrateInfo, ModuleKind};
use rustc_metadata::EncodedMetadata;
use rustc_session::config::{CrateType, OutFileName, OutputFilenames, OutputType};
use rustc_session::output::out_filename;
use rustc_session::Session;

use rcgjs_object::manifest::{Manifest, Mode, MANIFEST_VERSION};

/// The environment variable that names the `jsc-link` binary, for a build tree that does not keep
/// it beside the dylib.
const BIN_ENV: &str = "JSC_LINK_BIN";

/// The `CodegenBackend::link` hook.
pub(crate) fn link(
    sess: &Session,
    mut compiled_modules: CompiledModules,
    mut crate_info: CrateInfo,
    metadata: EncodedMetadata,
    outputs: &OutputFilenames,
) {
    let program_types: Vec<CrateType> = crate_info
        .crate_types
        .iter()
        .copied()
        .filter(is_program)
        .collect();
    let has_non_program = crate_info.crate_types.iter().any(|ct| !is_program(ct));

    // Link every program crate type (`cdylib`, `bin`, `dylib`) via `jsc-link` BEFORE running
    // `link_binary` for any non-program crate types (e.g. `rlib` in a combined `["cdylib", "rlib"]`
    // build), because `link_binary` consumes `compiled_modules` and cleans up its temporary CGU
    // objects.
    if !program_types.is_empty() {
        let mut objects: Vec<PathBuf> = Vec::new();
        for module in compiled_modules
            .modules
            .iter()
            .chain(&compiled_modules.allocator_module)
        {
            if let Some(path) = &module.object {
                objects.push(path.clone());
            }
        }

        for crate_type in program_types {
            let mut rlibs: Vec<PathBuf> = Vec::new();
            if let Err(err) = each_linked_rlib(&crate_info, Some(crate_type), &mut |_, path| {
                rlibs.push(path.to_path_buf());
            }) {
                // The error type is not `Debug`; its variants all mean the same thing here, which
                // is that a dependency has no rlib to read items out of.
                let reason = match err {
                    LinkRlibError::MissingFormat => "no dependency format was computed",
                    LinkRlibError::IncompatibleDependencyFormats { .. } => {
                        "the crate types disagree about how dependencies are linked"
                    }
                    LinkRlibError::NotFound { .. } => "a dependency's rlib is missing",
                    LinkRlibError::OnlyRmetaFound { .. } => "a dependency has metadata but no rlib",
                };
                sess.dcx().err(format!(
                    "rustc_codegen_js cannot read the linked rlibs: {reason}"
                ));
            }

            let out = out_filename(sess, crate_type, outputs, crate_info.local_crate_name);
            let mode = match out {
                OutFileName::Real(path) => Mode::Program { output: Some(path) },
                OutFileName::Stdout => Mode::Program { output: None },
            };
            run(
                sess,
                outputs,
                manifest(
                    sess,
                    Some(format!("{crate_type}")),
                    mode,
                    objects.clone(),
                    rlibs,
                ),
            );
        }
    }

    if !has_non_program {
        return;
    }

    // An rlib is not a program: it is a bag of objects for a later link to pick from, and rustc
    // already knows how to build one. Deleting unreachable items here would delete exactly the
    // items the downstream crate is about to ask for. Refresh this member even when every codegen
    // unit came from the incremental cache.
    let crate_name = sess.opts.crate_name.as_deref().unwrap_or_default();
    let name = format!("{crate_name}.rcgjs-sources");
    let path = outputs.temp_path_for_cgu(OutputType::Object, &name);
    let bytes = crate::item::write_object(
        "",
        &[],
        false,
        crate_name,
        &crate::item::object_identity(sess),
        rcgjs_object::jir::interns::Interns::new(),
        &source_texts(sess),
    );
    if let Err(error) = std::fs::write(&path, bytes) {
        sess.dcx().err(format!(
            "cannot retain source text in `{}`: {error}",
            path.display()
        ));
        return;
    }
    compiled_modules.modules.push(CompiledModule {
        name,
        kind: ModuleKind::Regular,
        object: Some(path),
        global_asm_object: None,
        dwarf_object: None,
        bytecode: None,
        assembly: None,
        llvm_ir: None,
        links_from_incr_cache: Vec::new(),
    });
    // Strip already-linked program crate types so `link_binary` only archives non-program outputs
    // (such as `.rlib`) and never invokes a native linker on JavaScript CGU objects.
    crate_info.crate_types.retain(|ct| !is_program(ct));
    link_binary(
        sess,
        &ArArchiveBuilderBuilder,
        compiled_modules,
        crate_info,
        metadata,
        outputs,
        "js",
    );
}

/// Whether a crate type produces a finished program rather than an intermediate archive.
fn is_program(crate_type: &CrateType) -> bool {
    matches!(
        crate_type,
        CrateType::Executable | CrateType::Cdylib | CrateType::Dylib
    )
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
    let mode = Mode::Prune {
        first: first.clone(),
        rest: rest.to_vec(),
    };
    run(
        sess,
        outputs,
        manifest(sess, None, mode, candidates.clone(), Vec::new()),
    );
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

/// Everything the consumer needs and cannot work out for itself.
fn manifest(
    sess: &Session,
    crate_type: Option<String>,
    mode: Mode,
    objects: Vec<PathBuf>,
    rlibs: Vec<PathBuf>,
) -> Manifest {
    let opts = crate::opts::get().clone();
    let sources = if opts.source_map {
        source_texts(sess).into_iter().collect()
    } else {
        Vec::new()
    };
    Manifest {
        version: MANIFEST_VERSION,
        opts,
        crate_name: sess.opts.crate_name.clone().unwrap_or_default(),
        crate_type: crate_type.unwrap_or_default(),
        mode,
        objects,
        rlibs,
        sources,
    }
}

/// The local sources from this compilation, with rustc's diagnostic path remapping.
fn source_texts(sess: &Session) -> BTreeMap<String, String> {
    sess.source_map()
        .files()
        .iter()
        .filter_map(|file| {
            file.src.as_ref().map(|text| {
                (
                    file.name.prefer_remapped_unconditionally().to_string(),
                    text.to_string(),
                )
            })
        })
        .collect()
}

/// Writes the manifest and runs `jsc-link` over it.
fn run(sess: &Session, outputs: &OutputFilenames, manifest: Manifest) {
    let bytes = match postcard::to_allocvec(&manifest) {
        Ok(bytes) => bytes,
        Err(err) => {
            sess.dcx().err(format!(
                "rustc_codegen_js: cannot encode the link manifest: {err}"
            ));
            return;
        }
    };
    let path = outputs.temp_path_for_diagnostic("jsc-link-manifest");
    if let Err(err) = std::fs::write(&path, &bytes) {
        sess.dcx().err(format!(
            "rustc_codegen_js: cannot write `{}`: {err}",
            path.display()
        ));
        return;
    }

    let Some(binary) = binary(sess) else {
        sess.dcx().err(format!(
            "rustc_codegen_js: cannot find the `jsc-link` binary. Build it with \
             `cargo build --release -p jsc-link`, or name it with `{BIN_ENV}=<path>`"
        ));
        return;
    };
    // Stderr is inherited, so `jsc-link`'s diagnostics land on this process's stderr as it writes
    // them, in one piece and in order. Nothing is re-rendered here: the messages a user reads are
    // the ones `jsc-link` printed, and the error below only says that the link failed.
    let status = Command::new(&binary).arg("--manifest").arg(&path).status();
    let _ = std::fs::remove_file(&path);
    match status {
        Ok(status) if status.success() => {}
        Ok(_) => {
            sess.dcx()
                .err("rustc_codegen_js: the link step reported errors");
        }
        Err(err) => {
            sess.dcx().err(format!(
                "rustc_codegen_js: cannot run `{}`: {err}",
                binary.display()
            ));
        }
    }
}

/// Where the `jsc-link` binary is.
///
/// `JSC_LINK_BIN` wins, for a build tree that keeps the two apart. Otherwise it is the sibling of
/// this dylib, which is where `cargo build -p rustc_codegen_mires -p jsc-link` puts both of them —
/// and the dylib's own path is the one rustc was given on the command line, because a dylib loaded
/// into rustc cannot ask for it any other way (`current_exe` is rustc's).
fn binary(sess: &Session) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(BIN_ENV) {
        let path = PathBuf::from(path);
        return path.exists().then_some(path);
    }
    let dylib = sess.opts.unstable_opts.codegen_backend.as_ref()?;
    let sibling = Path::new(dylib).parent()?.join("jsc-link");
    sibling.exists().then_some(sibling)
}
