//! The AOT driver: the `rustc_codegen_ssa` half that turns codegen units into "object files".
//!
//! Modeled on cg_clif's `src/driver/aot.rs`. The object file we write is the printed JavaScript
//! text followed by the binary member `link.rs` reads back (see `item.rs` and
//! `rcgjs_object::object` for the format); rustc's `produce_final_output_artifacts` then copies
//! the single CGU's object onto the `-o` path, which is how `-o out.js --emit=obj` ends up
//! containing our JS.

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
    object_identity, Decline, Interns, ItemKind, JirItem, JsItem, Linkage, ObjectIdentity,
    ZombieKind, ZombieLog,
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

    /// The banner every object file and every finished program starts with.
    pub(crate) fn prologue(label: &str) -> String {
        format!("// rustc_codegen_js — {label}\n")
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
    let shadow_cx = (cx.opts.optimizer != crate::opts::OptimizerMode::Off)
        .then(|| CguCx::new(tcx, crate::opts::get()));

    let mut work: Vec<_> = mono_items.into_iter().map(|(item, _)| item).collect();
    let mut depth = 0;
    loop {
        for mono_item in work {
            match mono_item {
                MonoItem::Fn(instance) => {
                    let replay = shadow_cx.as_ref().map(|shadow| {
                        let item = crate::base::codegen_fn(shadow, instance);
                        let _ = shadow.claim(&item.name);
                        item
                    });
                    let item = crate::base::codegen_fn(&cx, instance);
                    // The JIR half travels in every mode: the object carries a body whether or not
                    // the frozen text optimizer below runs, which is what lets the link step choose
                    // a lane without a rebuild. In the modes where the v1
                    // analysis runs, its summary already carries the JIR
                    // encoding and it is moved out of there rather than encoded
                    // twice. The identity travels whether or not the body
                    // encoded: an item the encoder declined is still what
                    // another item's `Callee::Direct` and `fn` pointers name.
                    let item = item.with_jir_item(Some(crate::jir::encode::item_id(&cx, instance)));
                    let item = if cx.opts.optimizer == crate::opts::OptimizerMode::Off {
                        let (jir, decline) = encode_jir_fn(&cx, instance);
                        item.with_jir(jir, decline)
                    } else {
                        let cleanup = crate::coroutine_source::cleanup_body(tcx, instance);
                        let mir = cleanup
                            .as_ref()
                            .unwrap_or_else(|| tcx.instance_mir(instance.def));
                        let mut late_ir = crate::late_ir::function(&cx, instance, mir);
                        let (jir, decline) = split_jir(&cx, &mut late_ir);
                        let shadow = crate::baseline_shadow::BaselineShadowSummary::check(
                            &item,
                            replay.as_ref().expect("optimizer replay context exists"),
                            &late_ir,
                        );
                        item.with_jir(jir, decline)
                            .with_late_ir(late_ir)
                            .with_baseline_shadow(shadow)
                    };
                    let mut item = item;
                    if cx.opts.cleanup
                        && item.jir.is_none()
                        && crate::coroutine_source::cleanup_body(tcx, instance).is_some_and(
                            |body| body.basic_blocks.iter().any(|block| block.is_cleanup),
                        )
                    {
                        item.zombies.push(rcgjs_object::item::Zombie {
                            loc: tcx.sess.source_map().span_to_diagnostic_string(tcx.def_span(instance.def_id())),
                            message: format!(
                                "requires semantic cleanup lowering; baseline fallback would discard its cleanup paths (encoding: {:?})",
                                item.jir_decline
                            ),
                            span: None,
                            kind: ZombieKind::Unsupported,
                        });
                    }
                    if cx.claim(&item.name) {
                        module.items.push(item);
                    }
                }
                MonoItem::Static(def_id) => {
                    let replay = shadow_cx.as_ref().map(|shadow| {
                        let item = crate::base::codegen_static(shadow, def_id);
                        let _ = shadow.claim(&item.name);
                        item
                    });
                    let item = crate::base::codegen_static(&cx, def_id);
                    let item = if cx.opts.optimizer == crate::opts::OptimizerMode::Off {
                        let (jir, decline) = encode_jir_static(&cx, def_id);
                        item.with_jir(jir, decline)
                    } else {
                        let mut late_ir = crate::late_ir::static_item(&cx, def_id);
                        let (jir, decline) = split_jir(&cx, &mut late_ir);
                        let shadow = crate::baseline_shadow::BaselineShadowSummary::check(
                            &item,
                            replay.as_ref().expect("optimizer replay context exists"),
                            &late_ir,
                        );
                        item.with_jir(jir, decline)
                            .with_late_ir(late_ir)
                            .with_baseline_shadow(shadow)
                    };
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

        work = cx
            .take_required()
            .into_iter()
            .map(|(name, item)| {
                eprintln!("cleanup-dependency: {}", name.as_str());
                item
            })
            .collect();
        if work.is_empty() {
            break;
        }
        depth += 1;
        if depth > tcx.recursion_limit().0 {
            tcx.dcx()
                .fatal("semantic dependency collection exceeded rustc's recursion limit");
        }
    }

    // Items discovered while lowering — interned vtables and the like — come last. A vtable's
    // JIR arrives inside its late-IR summary (`vtable.rs`); it moves to the item's own JIR half
    // here so the object stores every body once, in S3.
    module
        .items
        .extend(cx.take_extra().into_iter().map(|mut item| {
            let (jir, decline) = split_jir(&cx, &mut item.late_ir);
            item.with_jir(jir, decline)
        }));
    if let Some(shadow) = shadow_cx {
        let _ = shadow.take_extra();
    }
    module.interns = cx.take_interns();
    crate::coroutine_source::append(tcx, &mut module.interns);
    module
}

/// Moves the JIR body out of a late-IR summary, so that the object stores it once (S3) and the
/// summary itself (S6) carries none. Under `js-jir=off` the body is dropped instead: the encoder
/// ran because the v1 analysis drives it, but the object must not carry what the option turned
/// off.
fn split_jir(
    cx: &CguCx<'_>,
    late_ir: &mut crate::late_ir::LateIrAnalysisSummary,
) -> (Option<JirItem>, Option<Decline>) {
    let jir = late_ir.jir.take();
    let decline = late_ir.jir_decline.take();
    match cx.opts.jir {
        true => (jir, decline),
        false => (None, Some(jir_disabled())),
    }
}

/// The reason an object carries no JIR body when the lane was switched off.
///
/// Not `EncodeDecline::Synthesized`, which is what the writer records for a missing reason and
/// which means "this item has no typed MIR body". `js-jir=off` is a property of the build, not of
/// the item, and a coverage report has to be able to tell the two apart — otherwise turning the
/// lane off reads as every item in the program suddenly being unencodable.
fn jir_disabled() -> Decline {
    Decline::Encode(rcgjs_object::jir::decline::EncodeDecline::Disabled)
}

/// The JIR-S body of a function, in every `js-opt` mode, or why there is none.
///
/// The seam between the encoder (`crate::jir::encode`) and the object writer. `js-jir=off` writes
/// no body and no reason: the summary then reports the item as unavailable for want of a body,
/// which is the bisecting step that option exists for.
fn encode_jir_fn<'tcx>(
    cx: &CguCx<'tcx>,
    instance: rustc_middle::ty::Instance<'tcx>,
) -> (Option<JirItem>, Option<Decline>) {
    if !cx.opts.jir {
        return (None, Some(jir_disabled()));
    }
    let cleanup = crate::coroutine_source::cleanup_body(cx.tcx, instance);
    let mir = cleanup
        .as_ref()
        .unwrap_or_else(|| cx.tcx.instance_mir(instance.def));
    match crate::jir::encode::function(cx, instance, mir) {
        Ok(item) => (Some(item), None),
        Err(decline) => (None, Some(Decline::Encode(decline))),
    }
}

/// The JIR-S body of a static. See [`encode_jir_fn`].
fn encode_jir_static<'tcx>(
    cx: &CguCx<'tcx>,
    def_id: rustc_hir::def_id::DefId,
) -> (Option<JirItem>, Option<Decline>) {
    if !cx.opts.jir {
        return (None, Some(jir_disabled()));
    }
    match crate::jir::encode::static_item(cx, def_id) {
        Ok(item) => (Some(item), None),
        Err(decline) => (None, Some(Decline::Encode(decline))),
    }
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

    // The last thing that happens to an item before it becomes text. Local renaming is per item and
    // needs the AST, so this is as late as it can run; item renaming needs the whole program and so
    // waits for `link.rs`. A no-op unless `js-minify` is on.
    crate::minify::shorten_locals(&mut module.items);

    let interns = std::mem::take(&mut module.interns);
    let text = crate::item::write_object(
        &JsModule::prologue(&format!("codegen unit `{}`", module.cgu_name)),
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
        prof.artifact_size(
            "object_file",
            tmp_file.file_name().unwrap().to_string_lossy(),
            text.len() as u64,
        );
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
