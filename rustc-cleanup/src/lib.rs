//! Pinned rustc passes for JavaScript cleanup lowering.
#![feature(rustc_private, deref_patterns)]
#![allow(incomplete_features)]
extern crate itertools;
extern crate rustc_abi;
extern crate rustc_ast;
extern crate rustc_const_eval;
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_middle;
extern crate rustc_mir_dataflow;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_target;
extern crate rustc_trait_selection;
extern crate smallvec;
extern crate tracing;

use rustc_const_eval::util;
use rustc_middle::{mir::Body, ty::TyCtxt};

trait MirPass<'tcx> {
    fn run_pass(&self, tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>);
    fn is_required(&self) -> bool;
    fn is_enabled(&self, _: &rustc_session::Session) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}
mod pass_manager {
    pub(crate) use crate::MirPass;
}

include!(concat!(env!("OUT_DIR"), "/modules.rs"));

/// Elaborates a private copy through rustc's drop pass. The normal rustc provider must have
/// completed first. This does not lower coroutines or make this body executable runtime MIR.
pub fn elaborate<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) -> Result<(), &'static str> {
    use rustc_middle::mir::{AnalysisPhase, MirPhase};
    if body.should_skip() {
        return Err("rustc skips this body");
    }
    if body.injection_phase.is_some() {
        return Err("injected MIR is not supported");
    }
    if body.phase != MirPhase::Analysis(AnalysisPhase::Initial) {
        return Err("expected initial analysis MIR");
    }
    if !tcx.sess.opts.unstable_opts.mir_enable_passes.is_empty() {
        return Err("MIR pass overrides are not supported");
    }
    if body.tainted_by_errors.is_some() {
        return Err("MIR is tainted by an earlier error");
    }
    let analysis: &[&dyn MirPass<'tcx>] = &[
        &impossible_predicates::ImpossiblePredicates,
        &cleanup_post_borrowck::CleanupPostBorrowck,
        &remove_noop_landing_pads::RemoveNoopLandingPads,
        &simplify::SimplifyCfg::PostAnalysis,
        &deref_separator::Derefer,
    ];
    for pass in analysis {
        if pass.is_enabled(tcx.sess) {
            pass.run_pass(tcx, body);
            body.pass_count += 1;
        }
    }
    body.phase =
        rustc_middle::mir::MirPhase::Analysis(rustc_middle::mir::AnalysisPhase::PostCleanup);
    body.pass_count = 1;
    if rustc_const_eval::check_consts::post_drop_elaboration::checking_enabled(
        &rustc_const_eval::check_consts::ConstCx::new(tcx, body),
    ) {
        remove_uninit_drops::RemoveUninitDrops.run_pass(tcx, body);
        simplify::SimplifyCfg::RemoveFalseEdges.run_pass(tcx, body);
        // The original provider already ran CheckLiveDrops. It is a read-only lint.
        body.pass_count += 3;
    }
    let passes: &[&dyn MirPass<'tcx>] = &[
        &add_call_guards::CriticalCallEdges,
        &post_analysis_normalize::PostAnalysisNormalize,
        &add_subtyping_projections::Subtyper,
        &elaborate_drops::ElaborateDrops,
    ];
    for pass in passes {
        assert!(pass.is_required());
        let _ = pass.name();
        pass.run_pass(tcx, body);
        body.pass_count += 1;
    }
    Ok(())
}

/// What the runtime passes observed before removing no-op landing pads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prepared {
    /// The body had a cleanup block or an unwind edge that continues or enters cleanup. Such a
    /// body needs this MIR even when every pad was a no-op: its optimized MIR may inline a callee
    /// without the callee's cleanup. A body without either has no unwinding call to inline.
    pub had_cleanup: bool,
}

/// Reconstructs a private coroutine state machine while retaining host-exception cleanup.
/// The caller must retain this layout in the coroutine query. Rust panic policy is unchanged.
pub fn coroutine<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) -> Result<Prepared, &'static str> {
    if body.coroutine.is_none() {
        return Err("expected source coroutine");
    }
    elaborate(tcx, body)?;
    let passes: &[&dyn MirPass<'tcx>] = &[
        &add_moves_for_packed_drops::AddMovesForPackedDrops,
        &erase_deref_temps::EraseDerefTemps,
        &elaborate_box_derefs::ElaborateBoxDerefs,
        &coroutine::StateTransform,
    ];
    for pass in passes {
        pass.run_pass(tcx, body);
    }
    Ok(finish_runtime(tcx, body))
}

/// Prepares a private ordinary body for runtime execution while preserving its unwind edges.
pub fn ordinary<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) -> Result<Prepared, &'static str> {
    elaborate(tcx, body)?;
    for pass in [
        &add_moves_for_packed_drops::AddMovesForPackedDrops as &dyn MirPass<'tcx>,
        &erase_deref_temps::EraseDerefTemps,
        &elaborate_box_derefs::ElaborateBoxDerefs,
    ] {
        pass.run_pass(tcx, body);
    }
    Ok(finish_runtime(tcx, body))
}

fn finish_runtime<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) -> Prepared {
    body.phase = rustc_middle::mir::MirPhase::Runtime(rustc_middle::mir::RuntimePhase::Initial);
    for pass in [
        &lower_intrinsics::LowerIntrinsics as &dyn MirPass<'tcx>,
        &remove_place_mention::RemovePlaceMention,
        &simplify::SimplifyCfg::PreOptimizations,
    ] {
        if pass.is_enabled(tcx.sess) {
            pass.run_pass(tcx, body);
        }
    }
    body.phase = rustc_middle::mir::MirPhase::Runtime(rustc_middle::mir::RuntimePhase::PostCleanup);
    for decl in &mut body.local_decls {
        decl.local_info = rustc_middle::mir::ClearCrossCrate::Clear;
    }
    use rustc_middle::mir::UnwindAction;
    let prepared = Prepared {
        had_cleanup: body.basic_blocks.iter().any(|block| {
            block.is_cleanup
                || matches!(
                    block.terminator().unwind(),
                    Some(UnwindAction::Continue | UnwindAction::Cleanup(_))
                )
        }),
    };
    remove_noop_pads(tcx, body);
    prepared
}

/// Repeats rustc's post-elaboration `RemoveNoopLandingPads` and `SimplifyCfg::Final` from
/// `run_optimization_passes`. rustc gates that pair by MIR opt level only, so it applies under
/// `-Cpanic=abort`, which disables the analysis-phase run in `elaborate`. A no-op pad holds only
/// markers, writes to bare locals and jumps to no-op pads before resuming, never a drop or call.
/// Unwind edges into one become `Continue`, which propagates the same exception.
fn remove_noop_pads<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) {
    if tcx.sess.mir_opt_level() == 0
        || remove_noop_landing_pads::find_noop_landing_pads(body, None).is_empty()
    {
        return;
    }
    remove_noop_landing_pads::RemoveNoopLandingPads.run_pass(tcx, body);
    simplify::SimplifyCfg::Final.run_pass(tcx, body);
}

/// Selects the runtime half of const-evaluation branches with the pinned rustc transform.
pub fn select_runtime<'tcx>(tcx: TyCtxt<'tcx>, body: Body<'tcx>) -> Body<'tcx> {
    runtime_select::remap_mir_for_const_eval_select(tcx, body, rustc_hir::Constness::NotConst)
}

fn take_array<T, const N: usize>(b: &mut Box<[T]>) -> Result<[T; N], Box<[T]>> {
    let b: Box<[T; N]> = std::mem::take(b).try_into()?;
    Ok(*b)
}

/// Applies rustc's unchanged no-unwind policy to the body returned to its queries. The frame
/// layout remains the one computed with cleanup flags. JavaScript emission uses a separate body.
pub fn runtime_unwinds<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) {
    if let Some(coroutine) = &mut body.coroutine {
        for child in [
            &mut coroutine.coroutine_drop,
            &mut coroutine.coroutine_drop_async,
            &mut coroutine.coroutine_drop_proxy_async,
        ]
        .into_iter()
        .flatten()
        {
            runtime_unwinds(tcx, child);
        }
    }
    abort_unwinding_calls::AbortUnwindingCalls.run_pass(tcx, body);
}
