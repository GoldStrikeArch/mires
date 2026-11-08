//! Move-path initialization in promoted MIR. Normal-return edge effects are not call effects.

use rcgjs_object::jir::coroutine::{
    SourceInitPoint, SourceInitState, SourceInitialization, SourceMovePath, SourceProjection,
};
use rustc_middle::mir::{Body, Location, ProjectionElem};
use rustc_middle::ty::TyCtxt;
use rustc_mir_dataflow::impls::{MaybeInitializedPlaces, MaybeUninitializedPlaces};
use rustc_mir_dataflow::move_paths::MoveData;
use rustc_mir_dataflow::{Analysis, MaybeReachable};

pub(super) fn capture<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> SourceInitialization {
    // Track all places, including fields of partially moved values and generic types.
    // These are conservative analysis-MIR facts, not rustc's later drop-elaboration decisions.
    let moves = MoveData::gather_moves(body, tcx, |_| true);
    let paths = moves
        .move_paths
        .iter()
        .map(|path| SourceMovePath {
            parent: path.parent.map(|parent| parent.as_u32()),
            local: path.place.local.as_u32(),
            projection: path.place.projection.iter().map(projection).collect(),
        })
        .collect();
    let mut initialized = MaybeInitializedPlaces::new(tcx, body, &moves)
        .iterate_to_fixpoint(tcx, body, None)
        .into_results_cursor(body);
    let mut uninitialized = MaybeUninitializedPlaces::new(tcx, body, &moves)
        .iterate_to_fixpoint(tcx, body, None)
        .into_results_cursor(body);
    let mut points = Vec::with_capacity(body.basic_blocks.len());
    for (block, data) in body.basic_blocks.iter_enumerated() {
        let location = Location {
            block,
            statement_index: data.statements.len(),
        };
        let [before, after] = [false, true].map(|after| {
            if after {
                initialized.seek_after_primary_effect(location);
                uninitialized.seek_after_primary_effect(location);
            } else {
                initialized.seek_before_primary_effect(location);
                uninitialized.seek_before_primary_effect(location);
            }
            match initialized.get() {
                MaybeReachable::Unreachable => None,
                MaybeReachable::Reachable(initialized) => Some(SourceInitState {
                    maybe_initialized: initialized.iter().map(|path| path.as_u32()).collect(),
                    maybe_uninitialized: uninitialized
                        .get()
                        .iter()
                        .map(|path| path.as_u32())
                        .collect(),
                }),
            }
        });
        points.push(SourceInitPoint { before, after });
    }
    SourceInitialization { paths, points }
}

fn projection(element: rustc_middle::mir::PlaceElem<'_>) -> Option<SourceProjection> {
    Some(match element {
        ProjectionElem::Deref => SourceProjection::Deref,
        ProjectionElem::Field(field, _) => SourceProjection::Field(field.as_u32()),
        ProjectionElem::Downcast(_, variant) => SourceProjection::Downcast(variant.as_u32()),
        ProjectionElem::Index(local) => SourceProjection::Index(local.as_u32()),
        ProjectionElem::ConstantIndex {
            offset,
            min_length,
            from_end,
        } => SourceProjection::ConstantIndex {
            offset,
            min_length,
            from_end,
        },
        ProjectionElem::Subslice { from, to, from_end } => {
            SourceProjection::Subslice { from, to, from_end }
        }
        // A type-changing projection needs generic type identity, which this table does not carry.
        ProjectionElem::OpaqueCast(_) | ProjectionElem::UnwrapUnsafeBinder(_) => return None,
    })
}
