//! The AOT driver: the `rustc_codegen_ssa` half that turns codegen units into "object files".
//!
//! Modeled on cg_clif's `src/driver/aot.rs`. The object file we write is plain JavaScript text
//! plus the item table `link.rs` reads back (see `item.rs` for the format); rustc's
//! `produce_final_output_artifacts` then copies the single CGU's object onto the `-o` path, which
//! is how `-o out.js --emit=obj` ends up containing our JS.

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
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_session::Session;
use rustc_session::config::{OptLevel, OutputFilenames, OutputType};
use rustc_span::Symbol;

use crate::cgu::CguCx;
use crate::item::{ItemKind, JsItem, Linkage, ZombieKind, ZombieLog};
use crate::jsast;

/// A codegen unit's worth of JavaScript, as a list of independently linkable items.
pub(crate) struct JsModule {
    pub(crate) cgu_name: String,
    pub(crate) items: Vec<JsItem>,
}

impl JsModule {
    fn new(cgu_name: &str) -> JsModule {
        JsModule { cgu_name: cgu_name.to_owned(), items: Vec::new() }
    }

    /// The banner every object file and every finished program starts with.
    pub(crate) fn prologue(label: &str) -> String {
        format!("// rustc_codegen_mires — {label}\n")
    }
}

fn codegen_cgu(tcx: TyCtxt<'_>, cgu_name: Symbol) -> JsModule {
    let _timer = tcx.prof.generic_activity_with_arg("codegen cgu", cgu_name.as_str());

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

    let mut module = JsModule::new(cgu_name.as_str());
    let cx = CguCx::new(tcx, crate::opts::get());

    for (mono_item, _item_data) in mono_items {
        match mono_item {
            MonoItem::Fn(instance) => {
                let item = crate::base::codegen_fn(&cx, instance);
                if cx.claim(&item.name) {
                    module.items.push(item);
                }
            }
            MonoItem::Static(def_id) => {
                let item = crate::base::codegen_static(&cx, def_id);
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
                    "`global_asm!` is not supported by rustc_codegen_mires".to_string(),
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
    module
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

    let text = crate::item::write_object(
        &JsModule::prologue(&format!("codegen unit `{}`", module.cgu_name)),
        &module.items,
        crate::opts::get().comments,
    );

    if let Err(err) = fs::write(&tmp_file, text.as_bytes()) {
        return Err(format!("error writing js file `{}`: {err}", tmp_file.display()));
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
        let mut module = JsModule::new(module_name);
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
