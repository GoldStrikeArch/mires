//! Captures owned suspension facts before the runtime MIR passes erase yield terminators.
//!
//! The pinned rustc provider borrows promoted MIR, then delegates to rustc for checking. No MIR
//! reference escapes the query or remains borrowed while rustc steals the body. This backend, like
//! its global options, supports one compiler session per process. The cache is reset at session
//! init.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use rustc_data_structures::steal::Steal;
use rustc_middle::mir::{Body, TerminatorKind, UnwindAction};
use rustc_middle::ty::{CoroutineArgsExt, TyCtxt};
use rustc_middle::util::Providers;
use rustc_mir_dataflow::impls::{
    always_storage_live_locals, MaybeBorrowedLocals, MaybeLiveLocals, MaybeRequiresStorage,
    MaybeStorageLive,
};
use rustc_mir_dataflow::{Analysis, ResultsCursor};
use rustc_span::def_id::LocalDefId;

use rcgjs_object::jir::coroutine::{
    CoroutineSource, RuntimeFrame, SourceBlock, SourceEdge, SourceEdgeKind, SourceLiveness,
    SourceTerminator, SourceYield,
};
use rcgjs_object::jir::{DefinitionId, Interns, SourceFact};

mod initialization;
mod transport;
pub(crate) use transport::artifact;

type MirProvider = for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx Steal<Body<'tcx>>;

static ORIGINAL: OnceLock<MirProvider> = OnceLock::new();
static CAPTURED: Mutex<BTreeMap<DefinitionId, Captured>> = Mutex::new(BTreeMap::new());

struct Captured {
    mir: crate::source_mir::SourceMir,
    definition: LocalDefId,
    frame: Option<RuntimeFrame>,
}

pub(crate) fn init() {
    transport::init();
    CAPTURED
        .lock()
        .expect("coroutine source cache poisoned")
        .clear();
}

pub(crate) fn provide(providers: &mut Providers) {
    let original = providers.queries.mir_drops_elaborated_and_const_checked;
    ORIGINAL.get_or_init(|| original);
    providers.queries.mir_drops_elaborated_and_const_checked = capture;
}

/// Requests local generic coroutine bodies as well, before codegen units serialize their tables.
/// An incremental query hit may skip capture; absence remains explicit rather than fabricated.
pub(crate) fn prepare(tcx: TyCtxt<'_>) {
    for &definition in tcx.mir_keys(()) {
        if !tcx.is_constructor(definition.to_def_id())
            && (cleanup_probe() || tcx.is_coroutine(definition.to_def_id()))
        {
            tcx.ensure_done()
                .mir_drops_elaborated_and_const_checked(definition);
        }
    }
    transport::prepare(tcx);
}

fn capture<'tcx>(tcx: TyCtxt<'tcx>, definition: LocalDefId) -> &'tcx Steal<Body<'tcx>> {
    let coroutine = tcx.is_coroutine(definition.to_def_id());
    let captured = if cleanup_probe() || coroutine || crate::jir::encode::self_check() {
        let (body, _) = tcx.mir_promoted(definition);
        let body = body.borrow();
        Some((
            crate::source_mir::SourceMir::capture(&body),
            coroutine.then(|| body.var_debug_info.clone()),
        ))
    } else {
        None
    };
    let mut result = ORIGINAL.get().expect("rustc MIR provider was saved")(tcx, definition);
    if let Some((mir, debug)) = captured {
        if debug.is_none() && !cleanup_probe() {
            mir.body(tcx);
            return result;
        }
        if coroutine && cleanup_probe() {
            let mut body = mir.body(tcx);
            rustc_cleanup::coroutine(tcx, &mut body).expect("cleanup coroutine provider");
            // The extra initialized-state flags are part of the actual coroutine frame. The
            // query owns this body so rustc's layout, drop shims and metadata see the same fields.
            rustc_cleanup::runtime_unwinds(tcx, &mut body);
            result = tcx.alloc_steal_mir(body);
        }
        let frame = debug
            .as_ref()
            .and_then(|debug| runtime_frame(tcx, debug, &result.borrow()));
        let identity = crate::jir::types::definition_id(tcx, definition.to_def_id());
        CAPTURED
            .lock()
            .expect("coroutine source cache poisoned")
            .insert(
                identity,
                Captured {
                    mir,
                    definition,
                    frame,
                },
            );
    }
    result
}

/// Temporary isolated execution probe. Rust panic policy remains unchanged.
pub(crate) fn cleanup_probe() -> bool {
    crate::opts::get().cleanup
}

pub(crate) fn cleanup_body<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: rustc_middle::ty::Instance<'tcx>,
) -> Option<Body<'tcx>> {
    if !cleanup_probe() {
        return None;
    }
    let rustc_middle::ty::InstanceKind::Item(def) = instance.def else {
        return None;
    };
    if !def.is_local() {
        return transport::body(tcx, def);
    }
    let identity = crate::jir::types::definition_id(tcx, def);
    let mut body = {
        let captured = CAPTURED.lock().expect("coroutine source cache poisoned");
        captured.get(&identity)?.mir.body(tcx)
    };
    if tcx.is_coroutine(def) {
        rustc_cleanup::coroutine(tcx, &mut body).expect("cleanup coroutine transformation");
        let original = tcx.instance_mir(instance.def);
        let original_layout = original
            .coroutine
            .as_ref()
            .and_then(|c| c.coroutine_layout.as_ref());
        let layout = body
            .coroutine
            .as_ref()
            .and_then(|c| c.coroutine_layout.as_ref());
        assert_eq!(
            format!("{layout:?}"),
            format!("{original_layout:?}"),
            "cleanup coroutine layout differs"
        );
        eprintln!("cleanup-coroutine-layout-match: {instance:?}");
    } else {
        rustc_cleanup::ordinary(tcx, &mut body)
            .expect("cleanup execution probe requires elaboration");
    }
    body = rustc_cleanup::select_runtime(tcx, body);
    if std::env::var_os("JSC_MIR_TRANSPORT_SELFCHECK").is_some() {
        body = crate::source_mir::PortableMir::roundtrip(tcx, &body);
    }
    if !body.basic_blocks.iter().any(|block| block.is_cleanup) {
        return None;
    }
    eprintln!(
        "cleanup-execution {:?}: {} blocks",
        instance,
        body.basic_blocks.len()
    );
    Some(body)
}

fn source_facts<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: LocalDefId,
    body: &Body<'tcx>,
    frame: Option<RuntimeFrame>,
) -> (CoroutineSource, Vec<Option<crate::jsast::Loc>>) {
    let liveness = source_liveness(tcx, definition, body);
    let mut source = CoroutineSource {
        initialization: Some(initialization::capture(tcx, body)),
        runtime_frame: frame,
        block_count: body.basic_blocks.len() as u32,
        local_count: body.local_decls.len() as u32,
        blocks: body
            .basic_blocks
            .iter()
            .map(|block| source_block(tcx, body, &block.terminator().kind))
            .collect(),
        yields: Vec::new(),
    };
    let mut locations = Vec::new();
    for (block, data) in body.basic_blocks.iter_enumerated() {
        let terminator = data.terminator();
        if let TerminatorKind::Yield {
            resume,
            resume_arg,
            drop,
            ..
        } = terminator.kind
        {
            source.yields.push(SourceYield {
                block: block.as_u32(),
                resume: resume.as_u32(),
                drop: drop.map(|block| block.as_u32()),
                resume_local: resume_arg
                    .projection
                    .is_empty()
                    .then(|| resume_arg.local.as_u32()),
                source: None,
                liveness: liveness
                    .as_ref()
                    .map(|points| points[&block.as_u32()].clone()),
            });
            let span =
                rustc_span::hygiene::walk_chain_collapsed(terminator.source_info.span, body.span);
            locations.push(crate::names::location_of(tcx, span));
        }
    }
    (source, locations)
}

/// The pinned analysis-to-runtime passes visit debug places in their original record order.
/// Compare the record identity before accepting its transformed place; never match by name/span.
fn runtime_frame<'tcx>(
    tcx: TyCtxt<'tcx>,
    source: &[rustc_middle::mir::VarDebugInfo<'tcx>],
    body: &Body<'tcx>,
) -> Option<RuntimeFrame> {
    use rustc_middle::mir::{PlaceElem, VarDebugInfoContents};
    let layout = body.coroutine.as_ref()?.coroutine_layout.as_ref()?;
    if source.len() != body.var_debug_info.len() {
        return None;
    }
    let mut saved_local_sources = vec![Vec::new(); layout.field_tys.len()];
    for (before, after) in source.iter().zip(&body.var_debug_info) {
        if before.name != after.name || before.source_info != after.source_info {
            return None;
        }
        if before.composite.is_some() || after.composite.is_some() {
            continue;
        }
        let VarDebugInfoContents::Place(before) = before.value else {
            continue;
        };
        let Some(local) = before.as_local() else {
            continue;
        };
        let VarDebugInfoContents::Place(after) = after.value else {
            continue;
        };
        if !frame_self_local(body, after.local) {
            continue;
        }
        let Some((base, PlaceElem::Field(field, ty))) = after.iter_projections().last() else {
            continue;
        };
        let base = base.ty(&body.local_decls, tcx);
        let rustc_middle::ty::Coroutine(def, _) = base.ty.kind() else {
            continue;
        };
        if *def != body.source.def_id() {
            continue;
        }
        let Some(variant) = base.variant_index else {
            continue;
        };
        let saved = *layout.variant_fields.get(variant)?.get(field)?;
        if ty != layout.field_tys[saved].ty {
            return None;
        }
        saved_local_sources[saved.as_usize()].push(local.as_u32());
    }
    for sources in &mut saved_local_sources {
        sources.sort_unstable();
        sources.dedup();
    }
    Some(RuntimeFrame {
        saved_local_sources,
        states: layout
            .variant_fields
            .iter()
            .map(|fields| fields.iter().map(|field| field.as_u32()).collect())
            .collect(),
    })
}

/// StateTransform either uses SELF directly or unpacks Pin's field into a fresh local.
/// Accept that local only when it has exactly one write with that direct argument origin.
fn frame_self_local(body: &Body<'_>, local: rustc_middle::mir::Local) -> bool {
    use rustc_middle::mir::{Operand, PlaceElem, Rvalue, StatementKind};
    if local.as_u32() == 1 {
        return true;
    }
    let mut writes = 0;
    let mut unpacked = false;
    for block in body.basic_blocks.iter() {
        for statement in &block.statements {
            if let StatementKind::Assign(assign) = &statement.kind {
                let (place, value) = &**assign;
                if place.local != local || !place.projection.is_empty() {
                    continue;
                }
                writes += 1;
                unpacked = place.projection.is_empty()
                    && matches!(value,
                    Rvalue::Use(Operand::Move(source) | Operand::Copy(source), _)
                    if source.local.as_u32() == 1 && matches!(source.projection.as_ref(),
                        [PlaceElem::Field(field, _)] if field.as_u32() == 0));
            }
        }
        if let TerminatorKind::Call { destination, .. } = &block.terminator().kind {
            if destination.local == local {
                return false;
            }
        }
    }
    writes == 1 && unpacked
}

/// Reuses the pinned compiler's dataflow analyses and checks the source-to-witness field order.
/// This is the promoted body's witness layout, not the separately optimized runtime layout.
fn source_liveness<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: LocalDefId,
    body: &Body<'tcx>,
) -> Option<BTreeMap<u32, SourceLiveness>> {
    let witness = tcx.mir_coroutine_witnesses(definition);
    let witness = witness.as_ref()?;
    let mut storage =
        MaybeStorageLive::new(std::borrow::Cow::Owned(always_storage_live_locals(body)))
            .iterate_to_fixpoint(tcx, body, None)
            .into_results_cursor(body);
    let borrowed = MaybeBorrowedLocals.iterate_to_fixpoint(tcx, body, None);
    let requires = MaybeRequiresStorage::new(ResultsCursor::new_borrowing(body, &borrowed))
        .iterate_to_fixpoint(tcx, body, None);
    let mut requires = ResultsCursor::new_borrowing(body, &requires);
    let mut borrowed = ResultsCursor::new_borrowing(body, &borrowed);
    let mut live = MaybeLiveLocals
        .iterate_to_fixpoint(tcx, body, None)
        .into_results_cursor(body);
    let movable =
        tcx.coroutine_movability(definition.to_def_id()) == rustc_hir::Movability::Movable;
    let mut points = BTreeMap::new();
    for (block, data) in body.basic_blocks.iter_enumerated() {
        if !matches!(data.terminator().kind, TerminatorKind::Yield { .. }) {
            continue;
        }
        let location = rustc_middle::mir::Location {
            block,
            statement_index: data.statements.len(),
        };
        live.seek_to_block_end(block);
        borrowed.seek_before_primary_effect(location);
        requires.seek_before_primary_effect(location);
        storage.seek_before_primary_effect(location);
        let mut retained = live.get().clone();
        if !movable {
            retained.union(borrowed.get());
        }
        retained.intersect(requires.get());
        retained.remove(rustc_middle::mir::Local::from_u32(1));
        points.insert(
            block.as_u32(),
            SourceLiveness {
                live: retained.iter().map(|local| local.as_u32()).collect(),
                borrowed: borrowed
                    .get()
                    .iter()
                    .filter(|local| retained.contains(*local))
                    .map(|local| local.as_u32())
                    .collect(),
                storage: storage.get().iter().map(|local| local.as_u32()).collect(),
            },
        );
    }
    let saved: std::collections::BTreeSet<_> = points
        .values()
        .flat_map(|point| point.live.iter().copied())
        .collect();
    assert_eq!(
        saved.len(),
        witness.field_tys.len(),
        "source saved-local count disagrees with rustc"
    );
    for (local, field) in saved.iter().zip(&witness.field_tys) {
        let decl = &body.local_decls[rustc_middle::mir::Local::from_u32(*local)];
        assert_eq!(
            decl.ty, field.ty,
            "source saved-local type disagrees with rustc"
        );
        assert_eq!(
            decl.source_info, field.source_info,
            "source saved-local identity disagrees with rustc"
        );
    }
    assert_eq!(
        points.len() + rustc_middle::ty::CoroutineArgs::RESERVED_VARIANTS,
        witness.variant_fields.len()
    );
    let saved: Vec<_> = saved.into_iter().collect();
    for ((block, point), (fields, info)) in points.iter().zip(
        witness
            .variant_fields
            .iter()
            .zip(&witness.variant_source_info)
            .skip(rustc_middle::ty::CoroutineArgs::RESERVED_VARIANTS),
    ) {
        assert!(
            point
                .live
                .iter()
                .copied()
                .eq(fields.iter().map(|field| saved[field.as_usize()])),
            "source yield live fields disagree with rustc"
        );
        assert_eq!(
            *info,
            body.basic_blocks[rustc_middle::mir::BasicBlock::from_u32(*block)]
                .terminator()
                .source_info
        );
    }
    Some(points)
}

fn source_block<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &TerminatorKind<'tcx>,
) -> SourceBlock {
    use SourceEdgeKind::{Cancel, Imaginary, Normal, Resume, Unwind};
    use TerminatorKind as Mir;
    let mut successors = Vec::new();
    let mut edge = |kind, target: rustc_middle::mir::BasicBlock| {
        successors.push(SourceEdge {
            kind,
            target: target.as_u32(),
        });
    };
    let kind = match term {
        Mir::Goto { target } => {
            edge(Normal, *target);
            SourceTerminator::Goto
        }
        Mir::SwitchInt { targets, .. } => {
            for target in targets.all_targets() {
                edge(Normal, *target);
            }
            SourceTerminator::Switch
        }
        Mir::Call { target, unwind, .. } => {
            if let Some(target) = target {
                edge(Normal, *target);
            }
            if let UnwindAction::Cleanup(target) = unwind {
                edge(Unwind, *target);
            }
            SourceTerminator::Call
        }
        Mir::Drop {
            target,
            unwind,
            drop,
            ..
        } => {
            edge(Normal, *target);
            if let UnwindAction::Cleanup(target) = unwind {
                edge(Unwind, *target);
            }
            if let Some(target) = drop {
                edge(Cancel, *target);
            }
            SourceTerminator::Drop
        }
        Mir::Assert { target, unwind, .. } => {
            edge(Normal, *target);
            if let UnwindAction::Cleanup(target) = unwind {
                edge(Unwind, *target);
            }
            SourceTerminator::Assert
        }
        Mir::Yield { resume, drop, .. } => {
            edge(Resume, *resume);
            if let Some(target) = drop {
                edge(Cancel, *target);
            }
            SourceTerminator::Yield
        }
        Mir::FalseEdge {
            real_target,
            imaginary_target,
        } => {
            edge(Normal, *real_target);
            edge(Imaginary, *imaginary_target);
            SourceTerminator::FalseEdge
        }
        Mir::FalseUnwind {
            real_target,
            unwind,
        } => {
            edge(Normal, *real_target);
            if let UnwindAction::Cleanup(target) = unwind {
                edge(Imaginary, *target);
            }
            SourceTerminator::FalseUnwind
        }
        Mir::InlineAsm {
            targets, unwind, ..
        } => {
            for target in targets {
                edge(Normal, *target);
            }
            if let UnwindAction::Cleanup(target) = unwind {
                edge(Unwind, *target);
            }
            SourceTerminator::InlineAsm
        }
        Mir::TailCall { .. } => SourceTerminator::TailCall,
        Mir::Return => SourceTerminator::Return,
        Mir::Unreachable => SourceTerminator::Unreachable,
        Mir::CoroutineDrop => SourceTerminator::CoroutineDrop,
        Mir::UnwindResume => SourceTerminator::UnwindResume,
        Mir::UnwindTerminate(_) => SourceTerminator::UnwindTerminate,
    };
    assert!(
        successors
            .iter()
            .map(|edge| edge.target)
            .eq(term.successors().map(|block| block.as_u32())),
        "source coroutine capture omitted or reordered a MIR successor"
    );
    let drop_local = match term {
        Mir::Drop { place, .. } if place.projection.is_empty() => Some(place.local.as_u32()),
        _ => None,
    };
    let callee = match term {
        Mir::Call { func, .. } | Mir::TailCall { func, .. } => {
            match *func.ty(&body.local_decls, tcx).kind() {
                rustc_middle::ty::FnDef(def, _) => Some(crate::jir::types::definition_id(tcx, def)),
                _ => None,
            }
        }
        _ => None,
    };
    SourceBlock {
        kind,
        successors,
        drop_local,
        callee,
    }
}

/// Stores generic source facts once per object, independent of monomorphic type encoding.
pub(crate) fn append(tcx: TyCtxt<'_>, interns: &mut Interns) {
    // This must also run inside the codegen dep node. A generic-only crate's empty CGU has no
    // monomorphic body dependency that would otherwise invalidate its cached source facts.
    prepare(tcx);
    for (definition, captured) in CAPTURED
        .lock()
        .expect("coroutine source cache poisoned")
        .iter()
    {
        if !tcx.is_coroutine(captured.definition.to_def_id()) {
            continue;
        }
        let body = captured.mir.body(tcx);
        let (mut source, locations) =
            source_facts(tcx, captured.definition, &body, captured.frame.clone());
        for (point, location) in source.yields.iter_mut().zip(&locations) {
            point.source = location.as_ref().map(|location| {
                let file = interns.intern_str(&location.file);
                interns.intern_source_fact(SourceFact {
                    file,
                    line: location.line,
                    col: location.col,
                })
            });
        }
        interns.coroutine_sources.insert(*definition, source);
    }
}
