//! Static unwind-safety checker for `-Cpanic=unwind` (`JSC_PANIC=unwind`).
//!
//! Verifies that no function holding live droppable values makes a non-unwinding call (or
//! executes an inlined non-unwinding callee) that could allow a JavaScript host exception to
//! escape and skip the caller's destructors.

use std::borrow::Cow;
use std::fmt;

use rustc_data_structures::fx::FxHashSet;
use rustc_index::IndexVec;
use rustc_middle::mir::{
    self, BasicBlock, Body, Local, SourceScope, TerminatorKind, UnwindAction, UnwindTerminateReason,
};
use rustc_middle::ty::layout::fn_can_unwind;
use rustc_middle::ty::{self, Instance, InstanceKind, ShimKind, TyCtxt};
use rustc_mir_dataflow::impls::{always_storage_live_locals, MaybeLiveLocals, MaybeStorageLive};
use rustc_mir_dataflow::{Analysis, ResultsCursor};
use rustc_span::def_id::{CrateNum, LOCAL_CRATE};
use rustc_span::Span;

use crate::value::typing_env;

/// Why a non-unwinding callee may allow a JavaScript host exception to escape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnwindHazard {
    /// The callee itself could not be resolved to a concrete MIR body.
    UnresolvedTarget,
    /// A host exception can leave directly (non-sysroot foreign import or `UnwindTerminate(Abi)`).
    HostExceptionEscape,
    /// Transitive dispatch through a `dyn` trait method.
    VirtualDispatch,
    /// Transitive drop of a `dyn` trait object.
    DynamicDrop,
    /// A non-sysroot callee's MIR body is unavailable.
    MissingBody,
    /// Transitive inline assembly.
    InlineAsm,
    /// Transitive tail call.
    TailCall,
    /// Transitive indirect call through a function pointer.
    IndirectCall,
    /// A transitive call inside the callee could not be resolved to a MIR body.
    UnresolvedInnerCall,
}

impl fmt::Display for UnwindHazard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnresolvedTarget => {
                "and it does not resolve to a body that shows no host exception leaves it"
            }
            Self::HostExceptionEscape => "and a host exception can leave it",
            Self::VirtualDispatch => {
                "and it dispatches through a `dyn` method that may pass a host exception"
            }
            Self::DynamicDrop => "and dropping a `dyn` value inside it may pass a host exception",
            Self::MissingBody => {
                "and a callee's body is not available to show that no host exception leaves it"
            }
            Self::InlineAsm => "and inline assembly inside it may pass a host exception",
            Self::TailCall => "and a tail call inside it may pass a host exception",
            Self::IndirectCall => {
                "and it calls through a function pointer that may pass a host exception"
            }
            Self::UnresolvedInnerCall => {
                "and a call inside it does not resolve to a body that shows no host exception \
                 leaves it"
            }
        })
    }
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
pub(crate) fn reject_unwind_crossing_calls<'tcx>(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) {
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
                let abi = callee.ty(tcx, typing_env()).fn_sig(tcx).abi();
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
            typing_env(),
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
            .map(|args| Instance::try_resolve(tcx, typing_env(), callee, args));
        let hazard = match resolved {
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
                    Some(hazard) => hazard,
                }
            }
            _ => UnwindHazard::UnresolvedTarget,
        };
        tcx.dcx()
            .struct_span_err(
                span,
                format!(
                    "rustc_codegen_js: `{path}` has the \"{abi}\" ABI, which cannot unwind, \
                     {hazard}: {consequence}. {fix}"
                ),
            )
            .with_span_note(tcx.def_span(callee), "the function is defined here")
            .emit();
    }
}

/// Returns `None` iff `root`'s transitive call and drop graph is available and proven free of any
/// way for a host exception to leave `root`; otherwise returns `Some(hazard)` describing the cause.
fn callee_may_pass_host_exception<'tcx>(
    tcx: TyCtxt<'tcx>,
    root: Instance<'tcx>,
) -> Option<UnwindHazard> {
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
            InstanceKind::Virtual(..) => return Some(UnwindHazard::VirtualDispatch),
            InstanceKind::Shim(ShimKind::DropGlue(_, Some(drop_ty)))
                if matches!(drop_ty.kind(), ty::Dynamic(..)) =>
            {
                return Some(UnwindHazard::DynamicDrop);
            }
            InstanceKind::Item(def_id) => {
                if tcx.is_foreign_item(def_id) {
                    if is_sysroot_crate(tcx, def_id.krate) {
                        continue;
                    }
                    return Some(UnwindHazard::HostExceptionEscape);
                }
                if !tcx.is_mir_available(def_id) {
                    if is_sysroot_crate(tcx, def_id.krate) {
                        continue;
                    }
                    return Some(UnwindHazard::MissingBody);
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
                return Some(UnwindHazard::HostExceptionEscape);
            }
            match &term.kind {
                TerminatorKind::InlineAsm { .. } => return Some(UnwindHazard::InlineAsm),
                TerminatorKind::TailCall { .. } => return Some(UnwindHazard::TailCall),
                TerminatorKind::Drop { place, .. } => {
                    let drop_ty = inst.instantiate_mir_and_normalize_erasing_regions(
                        tcx,
                        typing_env(),
                        ty::EarlyBinder::bind(tcx, place.ty(&body.local_decls, tcx).ty),
                    );
                    if matches!(drop_ty.kind(), ty::Dynamic(..)) {
                        return Some(UnwindHazard::DynamicDrop);
                    }
                    let drop_inst = Instance::resolve_drop_glue(tcx, drop_ty);
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
                        typing_env(),
                        ty::EarlyBinder::bind(tcx, func.ty(body, tcx)),
                    );
                    let ty::FnDef(callee_def, callee_args) = *callee_ty.kind() else {
                        return Some(UnwindHazard::IndirectCall);
                    };
                    if tcx.is_foreign_item(callee_def) {
                        if is_sysroot_crate(tcx, callee_def.krate) {
                            continue;
                        }
                        return Some(UnwindHazard::HostExceptionEscape);
                    }
                    match callee_args
                        .no_bound_vars()
                        .map(|args| Instance::try_resolve(tcx, typing_env(), callee_def, args))
                    {
                        Some(Ok(Some(next))) => work.push(next),
                        _ => return Some(UnwindHazard::UnresolvedInnerCall),
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
    instance: Instance<'tcx>,
    body: &'a Body<'tcx>,
    /// Per local, whether its type has drop glue.
    droppable: Option<IndexVec<Local, bool>>,
    live: Option<ResultsCursor<'a, 'tcx, MaybeLiveLocals>>,
    storage: Option<ResultsCursor<'a, 'tcx, MaybeStorageLive<'a>>>,
}

impl<'a, 'tcx> HeldDrops<'a, 'tcx> {
    fn new(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>, body: &'a Body<'tcx>) -> Self {
        Self {
            tcx,
            instance,
            body,
            droppable: None,
            live: None,
            storage: None,
        }
    }

    fn across(&mut self, block: BasicBlock) -> bool {
        let (tcx, instance, body) = (self.tcx, self.instance, self.body);
        let droppable = self.droppable.get_or_insert_with(|| {
            body.local_decls
                .iter()
                .map(|decl| {
                    instance
                        .instantiate_mir_and_normalize_erasing_regions(
                            tcx,
                            typing_env(),
                            ty::EarlyBinder::bind(tcx, decl.ty),
                        )
                        .needs_drop(tcx, typing_env())
                })
                .collect()
        });
        if !droppable.iter().any(|&needs_drop| needs_drop) {
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
                    .any(|local| droppable[local] && Some(local) != destination)
            }
            None => {
                let storage = self.storage.get_or_insert_with(|| {
                    MaybeStorageLive::new(Cow::Owned(always_storage_live_locals(body)))
                        .iterate_to_fixpoint(tcx, body, None)
                        .into_results_cursor(body)
                });
                storage.seek_to_block_end(block);
                storage.get().iter().any(|local| droppable[local])
            }
        }
    }
}

/// Whether a terminator can end a cleanup path in `UnwindTerminate(Abi)`: rustc's mark of a frame
/// that cannot unwind. An `Assert` is left out: its unwind is a Rust panic, which aborts in this
/// backend and never carries a host exception.
fn ends_in_abi_terminate(kind: &mir::TerminatorKind<'_>) -> bool {
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
    body: &Body<'tcx>,
    mut scope: SourceScope,
) -> Option<(Instance<'tcx>, Span)> {
    let mut found = None;
    loop {
        let data = &body.source_scopes[scope];
        if let Some((callee, call)) = data.inlined {
            let abi = callee.ty(tcx, typing_env()).fn_sig(tcx).abi();
            if !fn_can_unwind(tcx, None, abi) {
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
pub(crate) fn is_sysroot_build(tcx: TyCtxt<'_>) -> bool {
    tcx.sess.opts.unstable_opts.force_unstable_if_unmarked
}

/// Whether `krate`, in a program's compilation, is one of the sysroot's own crates, whose foreign
/// imports are the runtime's: decided by where rustc loaded the crate from (the target's sysroot
/// library directory), never by its name. A crate passed with `--extern` from anywhere else is the
/// program's, whatever it is called, and so is the crate being compiled.
pub(crate) fn is_sysroot_crate(tcx: TyCtxt<'_>, krate: CrateNum) -> bool {
    if krate == LOCAL_CRATE {
        return false;
    }
    let sysroot = &tcx.sess.target_tlib_path.dir;
    let source = tcx.used_crate_source(krate);
    [&source.rlib, &source.rmeta, &source.dylib]
        .into_iter()
        .flatten()
        .any(|path| path.starts_with(sysroot))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwind_hazard_display_is_non_empty_and_distinct() {
        let all = [
            UnwindHazard::UnresolvedTarget,
            UnwindHazard::HostExceptionEscape,
            UnwindHazard::VirtualDispatch,
            UnwindHazard::DynamicDrop,
            UnwindHazard::MissingBody,
            UnwindHazard::InlineAsm,
            UnwindHazard::TailCall,
            UnwindHazard::IndirectCall,
            UnwindHazard::UnresolvedInnerCall,
        ];
        let rendered: Vec<String> = all.iter().map(ToString::to_string).collect();
        for text in &rendered {
            assert!(text.starts_with("and "), "{text}");
        }
        let unique: std::collections::BTreeSet<&str> =
            rendered.iter().map(String::as_str).collect();
        assert_eq!(unique.len(), all.len());
    }
}
