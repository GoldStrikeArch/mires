//! The AOT driver: the `rustc_codegen_ssa` half that turns codegen units into "object files".
//!
//! Modeled on cg_clif's `src/driver/aot.rs`. The object file we write is the printed JavaScript
//! text followed by the binary member `link.rs` reads back (see `item.rs` and
//! `rcgjs_object::object` for the format); rustc's `produce_final_output_artifacts` then copies
//! the single CGU's object onto the `-o` path, which is how `-o out.js --emit=obj` ends up
//! containing our JS.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::convert::Infallible;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use rustc_ast::expand::allocator::AllocatorMethod;
use rustc_codegen_ssa::back::lto::ThinModule;
use rustc_codegen_ssa::back::write::{
    CodegenContext, FatLtoInput, ModuleConfig, SharedEmitter, TargetMachineFactoryFn, ThinLtoInput,
};
use rustc_codegen_ssa::traits::{ExtraBackendMethods, WriteBackendMethods};
use rustc_codegen_ssa::{CompiledModule, ModuleCodegen, ModuleKind};
use rustc_data_structures::profiling::SelfProfilerRef;
use rustc_errors::DiagCtxt;
use rustc_middle::dep_graph::WorkProduct;
use rustc_middle::mono::MonoItem;
use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_middle::ty::TyCtxt;
use rustc_session::config::{OptLevel, OutputFilenames, OutputType};
use rustc_session::Session;
use rustc_span::Symbol;

use crate::cgu::CguCx;
use crate::item::{
    object_identity, prologue, Decline, Interns, ItemKind, JsItem, JsItemJir, JsName, Linkage,
    ObjectIdentity, Zombie, ZombieKind, ZombieLog,
};
use crate::jsast;

/// A codegen unit's worth of JavaScript, as a list of independently linkable items.
pub(crate) struct JsModule {
    pub(crate) cgu_name: String,
    pub(crate) crate_name: String,
    pub(crate) items: Vec<JsItem>,
    /// What the object header records about this compilation.
    pub(crate) identity: ObjectIdentity,
    /// The crate-scope tables every JIR body in `items` refers into.
    pub(crate) interns: Interns,
}

impl JsModule {
    fn new(cgu_name: &str, crate_name: &str, identity: ObjectIdentity) -> JsModule {
        JsModule {
            cgu_name: cgu_name.to_owned(),
            crate_name: crate_name.to_owned(),
            items: Vec::new(),
            identity,
            interns: Interns::new(),
        }
    }
}

fn codegen_cgu(tcx: TyCtxt<'_>, cgu_name: Symbol) -> JsModule {
    let _timer = tcx
        .prof
        .generic_activity_with_arg("codegen cgu", cgu_name.as_str());

    // Every zombie message names a type or a def path, and rendering one of those *trimmed* runs
    // the `trimmed_def_paths` query, which asserts that a diagnostic was emitted before the session
    // ends. A zombie is not a diagnostic: it is recorded now and only reported if the link step
    // finds it reachable, which for an rlib never happens. Rendering untrimmed keeps the assertion
    // out of it — the paths in a zombie message should be unambiguous anyway.
    with_no_trimmed_paths!(codegen_cgu_inner(tcx, cgu_name))
}

fn codegen_cgu_inner(tcx: TyCtxt<'_>, cgu_name: Symbol) -> JsModule {
    let cgu = tcx.codegen_unit(cgu_name);
    let mono_items = cgu.items_in_deterministic_order(tcx);

    let crate_name = tcx.crate_name(rustc_hir::def_id::LOCAL_CRATE);
    let mut module = JsModule::new(
        cgu_name.as_str(),
        crate_name.as_str(),
        object_identity(tcx.sess),
    );
    let cx = CguCx::new(tcx, crate::opts::get());

    for (mono_item, _) in mono_items {
        match mono_item {
            MonoItem::Fn(instance) => {
                let item = codegen_jir_fn(&cx, instance);
                if tcx.sess.panic_strategy() == rustc_target::spec::PanicStrategy::Unwind {
                    crate::unwind_check::reject_unwind_crossing_calls(tcx, instance);
                }
                if cx.claim(&item.name) {
                    module.items.push(item);
                }
            }
            MonoItem::Static(def_id) => {
                let item = codegen_jir_static(&cx, def_id);
                if cx.claim(&item.name) {
                    module.items.push(item);
                }
            }
            MonoItem::GlobalAsm(item_id) => {
                let def_id = item_id.owner_id.to_def_id();
                let span = tcx.def_span(def_id);
                let zombies = ZombieLog::default();
                let value = zombies.record(
                    tcx,
                    span,
                    ZombieKind::Unsupported,
                    "`global_asm!` is not supported by rustc_codegen_js".to_string(),
                );
                let name = cx.namer.synthetic_name(
                    "global_asm",
                    &format!("{:?}\u{1}{span:?}", item_id.owner_id),
                );
                let debug_path = with_no_trimmed_paths!(tcx.def_path_str(def_id));
                // Global assembly is emitted whether or not anything names it, so it is its own
                // root: the zombie in it is always reported.
                module.items.push(JsItem::new(
                    name.clone(),
                    ItemKind::Static,
                    jsast::let_(name.as_str(), value),
                    zombies.take(),
                    Linkage::rooted(name.into_string()),
                    debug_path,
                ));
            }
        }
    }

    // Items discovered while lowering — interned vtables and the like — come last.
    module.items.extend(cx.take_extra());
    module.interns = cx.take_interns();
    module
}

/// Whether the body carries unwind cleanup that only the JIR lane can keep.
fn requires_cleanup_lowering<'tcx>(
    cx: &CguCx<'tcx>,
    instance: rustc_middle::ty::Instance<'tcx>,
) -> bool {
    let tcx = cx.tcx;
    tcx.sess.panic_strategy() == rustc_target::spec::PanicStrategy::Unwind
        && tcx
            .instance_mir(instance.def)
            .basic_blocks
            .iter()
            .any(|block| block.is_cleanup)
}

/// Collects link-level dependencies from a JIR encoding result, or records the decline as an
/// `Unsupported` zombie when encoding failed.
fn collect_jir_outcome<'tcx>(
    cx: &CguCx<'tcx>,
    name: &JsName,
    def_id: rustc_hir::def_id::DefId,
    encoded: Result<crate::jir::encode::EncodedJir, rcgjs_object::jir::decline::EncodeDecline>,
    on_decline: impl FnOnce(
        &rcgjs_object::jir::decline::EncodeDecline,
    ) -> (Option<rcgjs_object::jir::semantic::ItemId>, String),
) -> (JsItemJir, BTreeSet<JsName>, Vec<Zombie>) {
    match encoded {
        Ok(encoded) => {
            let mut refs = encoded.extra_refs;
            {
                let interns = cx.interns();
                for dep in encoded.item.body.link_dependencies() {
                    if let Some(dep_name) = interns.str(dep) {
                        refs.insert(JsName::new(dep_name.to_owned()));
                    }
                }
            }
            // Keep any self-reference out of `refs` so dependency ordering matches `JsItem::new`.
            refs.remove(name);
            (JsItemJir::from(encoded.item), refs, Vec::new())
        }
        Err(decline) => {
            let loc = cx
                .tcx
                .sess
                .source_map()
                .span_to_diagnostic_string(cx.tcx.def_span(def_id));
            let (item, message) = on_decline(&decline);
            let zombies = vec![Zombie {
                loc,
                message,
                span: None,
                kind: ZombieKind::Unsupported,
            }];
            (
                JsItemJir::Declined {
                    item,
                    decline: Decline::Encode(decline),
                },
                BTreeSet::new(),
                zombies,
            )
        }
    }
}

/// Builds a JIR-only function item: encodes MIR to JIR-S and populates `refs` from the JIR
/// body's dependencies plus auxiliary refs so link-time reachability needs no v1 JS emission.
fn codegen_jir_fn<'tcx>(cx: &CguCx<'tcx>, instance: rustc_middle::ty::Instance<'tcx>) -> JsItem {
    let tcx = cx.tcx;
    let mir = tcx.instance_mir(instance.def);
    let name = cx.namer.fn_name(instance);
    let linkage = cx.namer.linkage_of(instance);
    let debug_path = with_no_trimmed_paths!(tcx.def_path_str(instance.def_id()));
    let dts = if linkage.exported {
        let params = crate::jir::encode::dts_params(tcx, instance, mir);
        crate::dts::signature(tcx, instance, &params)
    } else {
        None
    };
    let names = vec![(name.as_str().to_owned(), debug_path.clone())];
    let loc = crate::jir::encode::location_of(tcx, mir.span);

    let (jir, refs, zombies) = collect_jir_outcome(
        cx,
        &name,
        instance.def_id(),
        crate::jir::encode::function(cx, instance, mir),
        |decline| {
            let message = if requires_cleanup_lowering(cx, instance) {
                format!(
                    "{} (encoding: {decline})",
                    rcgjs_object::item::CLEANUP_LOWERING_REQUIRED
                )
            } else {
                format!("JIR encoding declined: {decline}")
            };
            (Some(crate::jir::encode::item_id(cx, instance)), message)
        },
    );
    let decl = jsast::function(name.as_str(), Vec::new(), Vec::new());
    JsItem::new(name, ItemKind::Fn, decl, zombies, linkage, debug_path)
        .with_extra_refs(refs)
        .with_source(loc, names)
        .with_dts(dts)
        .with_jir(jir)
}

/// Builds a JIR-only static item. See [`codegen_jir_fn`].
fn codegen_jir_static<'tcx>(cx: &CguCx<'tcx>, def_id: rustc_hir::def_id::DefId) -> JsItem {
    let tcx = cx.tcx;
    let name = cx.namer.static_name(def_id);
    let linkage = cx
        .namer
        .linkage_of(rustc_middle::ty::Instance::mono(tcx, def_id));
    let debug_path = with_no_trimmed_paths!(tcx.def_path_str(def_id));
    let names = vec![(name.as_str().to_owned(), debug_path.clone())];
    let loc = crate::jir::encode::location_of(tcx, tcx.def_span(def_id));

    let (jir, refs, zombies) = collect_jir_outcome(
        cx,
        &name,
        def_id,
        crate::jir::encode::static_item(cx, def_id),
        |decline| (None, format!("JIR encoding declined: {decline}")),
    );
    let decl = jsast::let_(name.as_str(), jsast::undefined());
    JsItem::new(name, ItemKind::Static, decl, zombies, linkage, debug_path)
        .with_extra_refs(refs)
        .with_source(loc, names)
        .with_jir(jir)
}

/// Write a codegen unit's JavaScript out as its "object file" and describe it to rustc.
///
/// `output_filenames.temp_path_for_cgu(OutputType::Object, name)` is the path rustc expects the
/// object at; handing it back as `CompiledModule::object` makes rustc's
/// `produce_final_output_artifacts` copy it onto the user's `-o` path (single CGU only).
fn emit_module(
    output_filenames: &OutputFilenames,
    prof: &SelfProfilerRef,
    mut module: JsModule,
    kind: ModuleKind,
    name: String,
) -> Result<CompiledModule, String> {
    let tmp_file = output_filenames.temp_path_for_cgu(OutputType::Object, &name);

    // Only synthesized allocator shims carry non-empty JavaScript ASTs with local variables at
    // codegen time; regular CGU items carry empty JIR placeholder stubs (lowered and minified at
    // link time) or bindings without locals. A no-op unless `js-minify` is on.
    if kind == ModuleKind::Allocator {
        crate::minify::shorten_locals(&mut module.items);
    }

    let interns = std::mem::take(&mut module.interns);
    let text = crate::item::write_object(
        &prologue(&format!("codegen unit `{}`", module.cgu_name)),
        &module.items,
        crate::opts::get().comments,
        &module.crate_name,
        &module.identity,
        interns,
        &Default::default(),
    );

    if let Err(err) = fs::write(&tmp_file, &text) {
        return Err(format!(
            "error writing js file `{}`: {err}",
            tmp_file.display()
        ));
    }

    if prof.enabled() {
        let artifact_name = tmp_file.file_name().map_or_else(
            || Cow::Borrowed(name.as_str()),
            |file| file.to_string_lossy(),
        );
        prof.artifact_size("object_file", artifact_name, text.len() as u64);
    }

    Ok(CompiledModule {
        name,
        kind,
        object: Some(tmp_file),
        global_asm_object: None,
        dwarf_object: None,
        bytecode: None,
        assembly: None,
        llvm_ir: None,
        links_from_incr_cache: Vec::new(),
    })
}

#[derive(Copy, Clone)]
pub(crate) struct JsDriver;

impl ExtraBackendMethods for JsDriver {
    type Module = JsModule;

    fn codegen_allocator<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        module_name: &str,
        methods: &[AllocatorMethod],
    ) -> Self::Module {
        // The bodies live in `alloc_support.rs`, which is also where the heap they call into is
        // described. Nothing here is a dead code elimination root: a program that never allocates
        // drops the whole module.
        let mut module = JsModule::new(
            module_name,
            tcx.crate_name(rustc_hir::def_id::LOCAL_CRATE).as_str(),
            object_identity(tcx.sess),
        );
        module.items = crate::alloc_support::shim_items(tcx, methods);
        module
    }

    fn compile_codegen_unit(
        &self,
        tcx: TyCtxt<'_>,
        cgu_name: Symbol,
    ) -> (ModuleCodegen<Self::Module>, u64) {
        let start_time = Instant::now();

        let dep_node = tcx.codegen_unit(cgu_name).codegen_dep_node(tcx);
        let (module, _) = tcx.dep_graph.with_task(
            dep_node,
            tcx,
            || {
                let js_module = codegen_cgu(tcx, cgu_name);
                ModuleCodegen::new_regular(cgu_name.as_str().to_owned(), js_module)
            },
            Some(rustc_middle::dep_graph::hash_result),
        );

        (module, start_time.elapsed().as_nanos() as u64)
    }
}

impl WriteBackendMethods for JsDriver {
    type Module = JsModule;
    type ModuleBuffer = Infallible;
    type TargetMachine = ();
    type ThinData = Infallible;

    fn target_machine_factory(
        &self,
        _sess: &Session,
        _opt_level: OptLevel,
        _target_features: &[String],
    ) -> TargetMachineFactoryFn<Self> {
        Arc::new(|_, _| ())
    }

    fn optimize_and_codegen_fat_lto(
        _sess: &Session,
        _cgcx: &CodegenContext,
        _shared_emitter: &SharedEmitter,
        _tm_factory: TargetMachineFactoryFn<Self>,
        _exported_symbols_for_lto: &[String],
        _each_linked_rlib_for_lto: &[PathBuf],
        _modules: Vec<FatLtoInput<Self>>,
    ) -> CompiledModule {
        unreachable!("LTO is rejected in CodegenBackend::init")
    }

    fn run_thin_lto(
        _cgcx: &CodegenContext,
        _prof: &SelfProfilerRef,
        _dcx: rustc_errors::DiagCtxtHandle<'_>,
        _exported_symbols_for_lto: &[String],
        _each_linked_rlib_for_lto: &[PathBuf],
        _modules: Vec<ThinLtoInput<Self>>,
    ) -> (Vec<ThinModule<Self>>, Vec<WorkProduct>) {
        unreachable!("LTO is rejected in CodegenBackend::init")
    }

    fn optimize(
        _cgcx: &CodegenContext,
        _prof: &SelfProfilerRef,
        _shared_emitter: &SharedEmitter,
        _module: &mut ModuleCodegen<Self::Module>,
        _config: &ModuleConfig,
    ) {
    }

    fn optimize_and_codegen_thin(
        _cgcx: &CodegenContext,
        _prof: &SelfProfilerRef,
        _shared_emitter: &SharedEmitter,
        _tm_factory: TargetMachineFactoryFn<Self>,
        _thin: ThinModule<Self>,
    ) -> CompiledModule {
        unreachable!("LTO is rejected in CodegenBackend::init")
    }

    fn codegen(
        cgcx: &CodegenContext,
        prof: &SelfProfilerRef,
        shared_emitter: &SharedEmitter,
        module: ModuleCodegen<Self::Module>,
        _config: &ModuleConfig,
    ) -> CompiledModule {
        let dcx = DiagCtxt::new(Box::new(shared_emitter.clone()));
        emit_module(
            &cgcx.output_filenames,
            prof,
            module.module_llvm,
            module.kind,
            module.name,
        )
        .unwrap_or_else(|err| dcx.handle().fatal(err))
    }

    fn serialize_module(_module: Self::Module, _is_thin: bool) -> Self::ModuleBuffer {
        unreachable!("LTO is rejected in CodegenBackend::init")
    }
}
