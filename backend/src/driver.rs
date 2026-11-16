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
                    if item.jir.is_none() && requires_cleanup_lowering(&cx, instance) {
                        item.zombies.push(rcgjs_object::item::Zombie {
                            loc: tcx
                                .sess
                                .source_map()
                                .span_to_diagnostic_string(tcx.def_span(instance.def_id())),
                            message: format!(
                                "{} (encoding: {:?})",
                                rcgjs_object::item::CLEANUP_LOWERING_REQUIRED,
                                item.jir_decline
                            ),
                            span: None,
                            kind: ZombieKind::Unsupported,
                        });
                    }
                    if tcx.sess.panic_strategy() == rustc_target::spec::PanicStrategy::Unwind {
                        reject_unwind_crossing_calls(tcx, instance);
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

/// Whether the body an item was lowered from carries cleanup that only the JIR lane can keep.
///
/// Baseline text has no lowering for an unwind edge, so an item whose JIR encoding was declined
/// would print with its cleanup paths silently dropped. Its zombie is what refuses a program that
/// reaches it instead. Two configurations hand the backend such a body:
///
/// * `js-cleanup=on`: the cleanup body re-derived from unoptimized MIR, when there is one
///   ([`crate::coroutine_source::cleanup_body`]);
/// * `-Cpanic=unwind` (the unwind build mode, `JSC_PANIC=unwind`): otherwise rustc's own optimized
///   MIR, which keeps its unwind edges. Any cleanup block in it is an obligation.
///
/// Under `-Cpanic=abort` the second never applies (rustc removed every cleanup edge), so this is
/// exactly the earlier `js-cleanup=on` check, evaluated in the same order.
fn requires_cleanup_lowering<'tcx>(
    cx: &CguCx<'tcx>,
    instance: rustc_middle::ty::Instance<'tcx>,
) -> bool {
    let tcx = cx.tcx;
    if cx.opts.cleanup && crate::coroutine_source::cleanup_body(tcx, instance).is_some() {
        return true;
    }
    tcx.sess.panic_strategy() == rustc_target::spec::PanicStrategy::Unwind
        && tcx
            .instance_mir(instance.def)
            .basic_blocks
            .iter()
            .any(|block| block.is_cleanup)
}

/// Under `-Cpanic=unwind` (the unwind build mode), an error for each place in this function where
/// a host exception could cross a frame whose cleanup rustc removed.
///
/// rustc gives a call no cleanup edge when the callee's ABI cannot unwind (`fn_can_unwind`, so
/// every such ABI, not a list of names): the destructors this function holds across the call are
/// then gone from its MIR, and a JavaScript exception that comes out of the call would leave
/// without running them. Two shapes reach that point:
///
/// * a call whose callee has such an ABI and may let a host exception out: a foreign import, a
///   call through a function pointer or a `dyn` method, or a Rust function whose transitive call
///   and drop graph cannot be proven free of `UnwindTerminate(Abi)`, non-sysroot foreign imports,
///   indirect or `dyn` dispatch (including dropping a `dyn` value), inline assembly, tail calls,
///   or unresolved / bodyless non-sysroot callees;
/// * such a callee inlined by rustc's MIR inliner, which leaves its `UnwindTerminate(Abi)` edges
///   inside this body, where the cleanup of this frame is gone the same way.
///
/// Either is refused only where this frame holds a value with drop glue across it
/// ([`HeldDrops`]): with nothing to drop the exception passes this frame as it would with the
/// edge, and the next frame up decides for itself. Declared `extern "C-unwind"` the call keeps its
/// edge, which is the fix every message names. A Rust callee whose transitive call and drop graph
/// is available and free of those hazards (cycles included) cannot let anything out and is
/// accepted.
///
/// The sysroot's own crates are not checked where the sysroot build compiles them
/// ([`is_sysroot_build`]): they call no host function (the imports they declare are the runtime's,
/// which this compiler supplies), and `compiler_builtins`' intrinsics call one another through the
/// C ABI. Their generics are checked where a program instantiates them, and a program's call to an
/// import a sysroot crate declares is accepted ([`is_sysroot_crate`]). It is an error where the
/// call is compiled rather than a zombie: the JIR lane replaces an item's zombies with its
/// lowering's, and this is a property of the program, not of the lowering.
fn reject_unwind_crossing_calls<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: rustc_middle::ty::Instance<'tcx>,
) {
    use rustc_middle::mir::TerminatorKind;
    use rustc_middle::ty::{self, layout::fn_can_unwind, InstanceKind};
    if is_sysroot_build(tcx) {
        return;
    }
    let body = tcx.instance_mir(instance.def);
    let mut held = HeldDrops::new(tcx, instance, body);
    let fix = "Declare it `extern \"C-unwind\"`.";
    let consequence =
        "under -Cpanic=unwind (JSC_PANIC=unwind) rustc gives the call no cleanup, so \
                       a host exception thrown behind it would skip this function's destructors";
    // rustc deduplicates an identical diagnostic, so each instance of a generic reports once.
    for (block, data) in body.basic_blocks.iter_enumerated() {
        let terminator = data.terminator();
        let span = terminator.source_info.span;
        if ends_in_abi_terminate(&terminator.kind) {
            if let Some((callee, call)) =
                inlined_nounwind_frame(tcx, body, terminator.source_info.scope)
                    .filter(|_| held.across(block))
            {
                let abi = callee.ty(tcx, crate::value::typing_env()).fn_sig(tcx).abi();
                tcx.dcx()
                    .struct_span_err(
                        call,
                        format!(
                            "rustc_codegen_js: `{}` has the \"{}\" ABI, which cannot unwind, and \
                             rustc inlined it here: {consequence}. {fix}",
                            tcx.def_path_str(callee.def_id()),
                            abi.as_str()
                        ),
                    )
                    .with_span_note(
                        tcx.def_span(callee.def_id()),
                        "the function is defined here",
                    )
                    .emit();
            }
        }
        let TerminatorKind::Call { func, .. } = &terminator.kind else {
            continue;
        };
        let callee_ty = instance.instantiate_mir_and_normalize_erasing_regions(
            tcx,
            crate::value::typing_env(),
            ty::EarlyBinder::bind(tcx, func.ty(body, tcx)),
        );
        let abi = callee_ty.fn_sig(tcx).abi();
        if fn_can_unwind(tcx, None, abi) || !held.across(block) {
            continue;
        }
        let abi = abi.as_str();
        let ty::FnDef(callee, args) = *callee_ty.kind() else {
            tcx.dcx().span_err(
                span,
                format!(
                    "rustc_codegen_js: this call goes through a `{callee_ty}`, whose \"{abi}\" ABI \
                     cannot unwind: {consequence}. Use an `extern \"C-unwind\"` function pointer \
                     type."
                ),
            );
            continue;
        };
        let path = tcx.def_path_str(callee);
        if tcx.is_foreign_item(callee) {
            if is_sysroot_crate(tcx, callee.krate) {
                continue;
            }
            tcx.dcx()
                .struct_span_err(
                    span,
                    format!(
                        "rustc_codegen_js: `{path}` is a foreign import with the \"{abi}\" ABI, \
                         which cannot unwind: under -Cpanic=unwind (JSC_PANIC=unwind) a host \
                         exception thrown through it would skip its callers' cleanup. {fix}"
                    ),
                )
                .with_span_note(tcx.def_span(callee), "the import is declared here")
                .emit();
            continue;
        }
        let resolved = args
            .no_bound_vars()
            .map(|args| ty::Instance::try_resolve(tcx, crate::value::typing_env(), callee, args));
        let why = match resolved {
            Some(Ok(Some(target))) => {
                if matches!(target.def, InstanceKind::Virtual(..)) {
                    tcx.dcx().span_err(
                        span,
                        format!(
                            "rustc_codegen_js: this call of the `dyn` method `{path}` has the \
                             \"{abi}\" ABI, which cannot unwind: {consequence}. Declare the method \
                             `extern \"C-unwind\"`."
                        ),
                    );
                    continue;
                }
                match callee_may_pass_host_exception(tcx, target) {
                    None => continue,
                    Some(why) => why,
                }
            }
            _ => "and it does not resolve to a body that shows no host exception leaves it",
        };
        tcx.dcx()
            .struct_span_err(
                span,
                format!(
                    "rustc_codegen_js: `{path}` has the \"{abi}\" ABI, which cannot unwind, {why}: \
                     {consequence}. {fix}"
                ),
            )
            .with_span_note(tcx.def_span(callee), "the function is defined here")
            .emit();
    }
}

/// Returns `None` iff `root`'s transitive call and drop graph is available and proven free of any
/// way for a host exception to leave `root`; otherwise returns `Some(why)` describing the hazard.
fn callee_may_pass_host_exception<'tcx>(
    tcx: TyCtxt<'tcx>,
    root: rustc_middle::ty::Instance<'tcx>,
) -> Option<&'static str> {
    use rustc_data_structures::fx::FxHashSet;
    use rustc_middle::mir::TerminatorKind;
    use rustc_middle::ty::{self, InstanceKind, ShimKind};

    let mut visited = FxHashSet::default();
    let mut work = vec![root];

    while let Some(inst) = work.pop() {
        if !visited.insert(inst) {
            continue;
        }
        match inst.def {
            InstanceKind::Intrinsic(_)
            | InstanceKind::LlvmIntrinsic(_)
            | InstanceKind::Shim(ShimKind::DropGlue(_, None) | ShimKind::ThreadLocal(..)) => {
                continue;
            }
            InstanceKind::Virtual(..) => {
                return Some(
                    "and it dispatches through a `dyn` method that may pass a host exception",
                );
            }
            InstanceKind::Shim(ShimKind::DropGlue(_, Some(drop_ty)))
                if matches!(drop_ty.kind(), ty::Dynamic(..)) =>
            {
                return Some("and dropping a `dyn` value inside it may pass a host exception");
            }
            InstanceKind::Item(def_id) => {
                if tcx.is_foreign_item(def_id) {
                    if is_sysroot_crate(tcx, def_id.krate) {
                        continue;
                    }
                    return Some("and a host exception can leave it");
                }
                if !tcx.is_mir_available(def_id) {
                    if is_sysroot_crate(tcx, def_id.krate) {
                        continue;
                    }
                    return Some(
                        "and a callee's body is not available to show that no host exception \
                         leaves it",
                    );
                }
            }
            // Other shims (`DropGlue(_, Some(_))`, `CloneShim`, `ClosureOnceShim`, `FnPtrShim`,
            // `ReifyShim`, `VTableShim`, etc.) provide compiler-synthesized MIR via `instance_mir`.
            InstanceKind::Shim(_) => {}
        }

        let body = tcx.instance_mir(inst.def);
        for data in body.basic_blocks.iter() {
            let term = data.terminator();
            if ends_in_abi_terminate(&term.kind) {
                return Some("and a host exception can leave it");
            }
            match &term.kind {
                TerminatorKind::InlineAsm { .. } => {
                    return Some("and inline assembly inside it may pass a host exception");
                }
                TerminatorKind::TailCall { .. } => {
                    return Some("and a tail call inside it may pass a host exception");
                }
                TerminatorKind::Drop { place, .. } => {
                    let drop_ty = inst.instantiate_mir_and_normalize_erasing_regions(
                        tcx,
                        crate::value::typing_env(),
                        ty::EarlyBinder::bind(tcx, place.ty(&body.local_decls, tcx).ty),
                    );
                    if matches!(drop_ty.kind(), ty::Dynamic(..)) {
                        return Some(
                            "and dropping a `dyn` value inside it may pass a host exception",
                        );
                    }
                    let drop_inst = ty::Instance::resolve_drop_glue(tcx, drop_ty);
                    if !matches!(
                        drop_inst.def,
                        InstanceKind::Shim(ShimKind::DropGlue(_, None))
                    ) {
                        work.push(drop_inst);
                    }
                }
                TerminatorKind::Call { func, .. } => {
                    let callee_ty = inst.instantiate_mir_and_normalize_erasing_regions(
                        tcx,
                        crate::value::typing_env(),
                        ty::EarlyBinder::bind(tcx, func.ty(body, tcx)),
                    );
                    let ty::FnDef(callee_def, callee_args) = *callee_ty.kind() else {
                        return Some(
                            "and it calls through a function pointer that may pass a host \
                             exception",
                        );
                    };
                    if tcx.is_foreign_item(callee_def) {
                        if is_sysroot_crate(tcx, callee_def.krate) {
                            continue;
                        }
                        return Some("and a host exception can leave it");
                    }
                    match callee_args.no_bound_vars().map(|args| {
                        ty::Instance::try_resolve(tcx, crate::value::typing_env(), callee_def, args)
                    }) {
                        Some(Ok(Some(next))) => work.push(next),
                        _ => {
                            return Some(
                                "and a call inside it does not resolve to a body that shows no \
                                 host exception leaves it",
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    None
}

/// Which values a frame holds across a call, asked only where a refusal is in question.
///
/// A value is held across the call ending `block` when its type has drop glue and it is used after
/// the call returns (dropped, moved or read: maybe-live at the call's return block), other than the
/// call's own destination, which is not yet written while the call runs. A terminator with no
/// block to return to (a call that does not return, or `UnwindTerminate` ending a cleanup path)
/// gives no such answer, so there every local of such a type whose storage is live counts.
struct HeldDrops<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    instance: rustc_middle::ty::Instance<'tcx>,
    body: &'a rustc_middle::mir::Body<'tcx>,
    /// Per local, whether its type has drop glue.
    droppable: Option<Vec<bool>>,
    live: Option<
        rustc_mir_dataflow::ResultsCursor<'a, 'tcx, rustc_mir_dataflow::impls::MaybeLiveLocals>,
    >,
    storage: Option<
        rustc_mir_dataflow::ResultsCursor<
            'a,
            'tcx,
            rustc_mir_dataflow::impls::MaybeStorageLive<'a>,
        >,
    >,
}

impl<'a, 'tcx> HeldDrops<'a, 'tcx> {
    fn new(
        tcx: TyCtxt<'tcx>,
        instance: rustc_middle::ty::Instance<'tcx>,
        body: &'a rustc_middle::mir::Body<'tcx>,
    ) -> Self {
        Self {
            tcx,
            instance,
            body,
            droppable: None,
            live: None,
            storage: None,
        }
    }

    fn across(&mut self, block: rustc_middle::mir::BasicBlock) -> bool {
        use rustc_middle::mir::TerminatorKind;
        use rustc_mir_dataflow::impls::{
            always_storage_live_locals, MaybeLiveLocals, MaybeStorageLive,
        };
        use rustc_mir_dataflow::Analysis;
        let (tcx, instance, body) = (self.tcx, self.instance, self.body);
        let droppable = self.droppable.get_or_insert_with(|| {
            body.local_decls
                .iter()
                .map(|decl| {
                    instance
                        .instantiate_mir_and_normalize_erasing_regions(
                            tcx,
                            crate::value::typing_env(),
                            rustc_middle::ty::EarlyBinder::bind(tcx, decl.ty),
                        )
                        .needs_drop(tcx, crate::value::typing_env())
                })
                .collect()
        });
        if !droppable.contains(&true) {
            return false;
        }
        let terminator = body.basic_blocks[block].terminator();
        let returns_to = match &terminator.kind {
            TerminatorKind::Call {
                target: Some(target),
                destination,
                ..
            } => Some((*target, Some(destination.local))),
            TerminatorKind::Drop { target, .. } => Some((*target, None)),
            _ => None,
        };
        match returns_to {
            Some((target, destination)) => {
                let live = self.live.get_or_insert_with(|| {
                    MaybeLiveLocals
                        .iterate_to_fixpoint(tcx, body, None)
                        .into_results_cursor(body)
                });
                live.seek_to_block_start(target);
                live.get()
                    .iter()
                    .any(|local| droppable[local.as_usize()] && Some(local) != destination)
            }
            None => {
                let storage = self.storage.get_or_insert_with(|| {
                    MaybeStorageLive::new(std::borrow::Cow::Owned(always_storage_live_locals(body)))
                        .iterate_to_fixpoint(tcx, body, None)
                        .into_results_cursor(body)
                });
                storage.seek_to_block_end(block);
                storage
                    .get()
                    .iter()
                    .any(|local| droppable[local.as_usize()])
            }
        }
    }
}

/// Whether a terminator can end a cleanup path in `UnwindTerminate(Abi)`: rustc's mark of a frame
/// that cannot unwind. An `Assert` is left out: its unwind is a Rust panic, which aborts in this
/// backend and never carries a host exception.
fn ends_in_abi_terminate(kind: &rustc_middle::mir::TerminatorKind<'_>) -> bool {
    use rustc_middle::mir::{TerminatorKind, UnwindAction, UnwindTerminateReason};
    match kind {
        TerminatorKind::UnwindTerminate(UnwindTerminateReason::Abi) => true,
        TerminatorKind::Assert { .. } => false,
        kind => matches!(
            kind.unwind(),
            Some(UnwindAction::Terminate(UnwindTerminateReason::Abi))
        ),
    }
}

/// The outermost function inlined at `scope` whose ABI cannot unwind, with the span of the call
/// rustc inlined it at: the frame an `UnwindTerminate(Abi)` at `scope` belongs to, when it is not
/// this body's own. `None` when every inlined frame there can unwind (the edge then ends this
/// body's own cleanup, or a `#[rustc_nounwind]` function's, whose promise is not about ABIs).
fn inlined_nounwind_frame<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &rustc_middle::mir::Body<'tcx>,
    mut scope: rustc_middle::mir::SourceScope,
) -> Option<(rustc_middle::ty::Instance<'tcx>, rustc_span::Span)> {
    let mut found = None;
    loop {
        let data = &body.source_scopes[scope];
        if let Some((callee, call)) = data.inlined {
            let abi = callee.ty(tcx, crate::value::typing_env()).fn_sig(tcx).abi();
            if !rustc_middle::ty::layout::fn_can_unwind(tcx, None, abi) {
                found = Some((callee, call));
            }
        }
        match data.parent_scope {
            Some(parent) => scope = parent,
            None => return found,
        }
    }
}

/// Whether this compilation is the sysroot build's: `scripts/build_sysroot.sh` compiles `core`,
/// `compiler_builtins` and `alloc` with `-Zforce-unstable-if-unmarked` (rustc's own rule for the
/// standard library's crates), which a program's compilation is never given.
fn is_sysroot_build(tcx: TyCtxt<'_>) -> bool {
    tcx.sess.opts.unstable_opts.force_unstable_if_unmarked
}

/// Whether `krate`, in a program's compilation, is one of the sysroot's own crates, whose foreign
/// imports are the runtime's: decided by where rustc loaded the crate from (the target's sysroot
/// library directory), never by its name. A crate passed with `--extern` from anywhere else is the
/// program's, whatever it is called, and so is the crate being compiled.
fn is_sysroot_crate(tcx: TyCtxt<'_>, krate: rustc_span::def_id::CrateNum) -> bool {
    if krate == rustc_span::def_id::LOCAL_CRATE {
        return false;
    }
    let sysroot = &tcx.sess.target_tlib_path.dir;
    let source = tcx.used_crate_source(krate);
    [&source.rlib, &source.rmeta, &source.dylib]
        .into_iter()
        .flatten()
        .next()
        .is_some_and(|path| path.starts_with(sysroot))
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
