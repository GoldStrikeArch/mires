//! Which of a body's locals hold an enum **tag** rather than a discriminant number.
//!
//! A tagged enum tells its variants apart by name (`value::EnumRepr::Tag`), so reading the
//! discriminant of one gives a string. MIR does not know that: `_5 = discriminant(_1)` types `_5`
//! as an integer, and every use of `_5` is written as a use of an integer. Two of those uses are
//! fine with a string and the rest are not:
//!
//! * a `SwitchInt` on it is a match, and a match over names is a comparison of names;
//! * anything else -- `_5 as i32`, arithmetic, a call argument -- means the program asked for the
//!   number, and a string would be a silently wrong answer.
//!
//! So one pre-pass per body finds the locals whose single definition is a tagged enum's
//! discriminant and whose *every* use is a `SwitchInt` scrutinee. Those read the tag; every other
//! discriminant read of a tagged enum is lowered to the number instead, by a chain that maps each
//! name to its discriminant (`rvalue.rs`). The classification is conservative in that direction: a
//! local this pass is unsure about gets the number, which is correct everywhere and merely longer.
//!
//! # Why the switch has to be asked here and not later
//!
//! By the time the switch is lowered, its scrutinee is an expression and its type is the integer
//! MIR gave it; the enum it came from is gone. `cfg_adapter::Scrutinee` carries the enum type
//! forward so that `emit.rs` can spell a case value as a name, and this is where that type is
//! found.

use rustc_abi::Variants;
use rustc_data_structures::fx::{FxHashMap, FxHashSet};
use rustc_middle::mir::visit::{PlaceContext, Visitor};
use rustc_middle::mir::{
    Body, Local, Location, Operand, RETURN_PLACE, Rvalue, StatementKind, TerminatorKind,
};
use rustc_middle::ty::{Instance, Ty, TyCtxt};

use crate::value::{self, EnumRepr, typing_env};

/// The tag-reading locals of one body, with the enum each one's tag belongs to.
#[derive(Default)]
pub(crate) struct TagDiscrs<'tcx> {
    locals: FxHashMap<Local, Ty<'tcx>>,
}

impl<'tcx> TagDiscrs<'tcx> {
    /// Classifies every local of `mir`.
    ///
    /// The instance is what makes the type questions answerable: the body is the generic one, so a
    /// place's type has to be monomorphized before anything may be concluded from it.
    pub(crate) fn of(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>, mir: &Body<'tcx>) -> Self {
        let mut candidates = FxHashMap::default();
        let mut rejected = FxHashSet::default();

        let monomorphize = |value: Ty<'tcx>| {
            instance.instantiate_mir_and_normalize_erasing_regions(
                tcx,
                typing_env(),
                rustc_middle::ty::EarlyBinder::bind(tcx, value),
            )
        };

        for (block, data) in mir.basic_blocks.iter_enumerated() {
            for (index, statement) in data.statements.iter().enumerate() {
                let StatementKind::Assign(assignment) = &statement.kind else { continue };
                let (Some(local), Rvalue::Discriminant(source)) =
                    (assignment.0.as_local(), &assignment.1)
                else {
                    continue;
                };
                // A parameter and the return place are bound outside the body, so a definition
                // here is not the only one they have.
                if local == RETURN_PLACE || local.as_usize() <= mir.arg_count {
                    continue;
                }
                let source_ty = monomorphize(source.ty(&mir.local_decls, tcx).ty);
                if !matches!(value::enum_repr(tcx, source_ty), Some(EnumRepr::Tag { .. })) {
                    continue;
                }
                // A layout with one possible variant has no tag in the value at all: the
                // discriminant is a compile time constant, and `rvalue.rs` answers with the number.
                // `ControlFlow<Infallible, T>` is the case that matters -- one variant is
                // uninhabited, so the enum has two variants and one layout -- and it is reached by
                // every `?` in the program. Claiming a tag here would leave a `switch` comparing a
                // constant number against variant names, which is a silent wrong answer.
                let multiple = tcx
                    .layout_of(typing_env().as_query_input(source_ty))
                    .is_ok_and(|layout| matches!(layout.variants, Variants::Multiple { .. }));
                if !multiple {
                    continue;
                }
                // A second definition of the same local means its value is not this one read.
                let at = Location { block, statement_index: index };
                if candidates.insert(local, (source_ty, at)).is_some() {
                    rejected.insert(local);
                }
            }
        }

        if candidates.is_empty() {
            return TagDiscrs::default();
        }

        // The uses a `SwitchInt` makes of its bare scrutinee local, which are the only ones a tag
        // can serve.
        let mut switched = FxHashSet::default();
        for (block, data) in mir.basic_blocks.iter_enumerated() {
            let Some(terminator) = &data.terminator else { continue };
            let TerminatorKind::SwitchInt { discr, .. } = &terminator.kind else { continue };
            let Some(local) = discr.place().and_then(|place| place.as_local()) else { continue };
            switched.insert((local, Location { block, statement_index: data.statements.len() }));
        }

        let mut collector = Collector { candidates: &candidates, switched, rejected };
        collector.visit_body(mir);
        let rejected = collector.rejected;

        TagDiscrs {
            locals: candidates
                .into_iter()
                .filter(|(local, _)| !rejected.contains(local))
                .map(|(local, (ty, _))| (local, ty))
                .collect(),
        }
    }

    /// The enum whose tag this local holds, if it holds one.
    pub(crate) fn get(&self, local: Local) -> Option<Ty<'tcx>> {
        self.locals.get(&local).copied()
    }

    /// The enum whose tag this operand reads, if the operand is such a local read whole.
    pub(crate) fn of_operand(&self, operand: &Operand<'tcx>) -> Option<Ty<'tcx>> {
        self.get(operand.place()?.as_local()?)
    }
}

/// Rejects every candidate that is mentioned anywhere but its own definition and the switches that
/// read it.
struct Collector<'a, 'tcx> {
    candidates: &'a FxHashMap<Local, (Ty<'tcx>, Location)>,
    switched: FxHashSet<(Local, Location)>,
    rejected: FxHashSet<Local>,
}

impl<'tcx> Visitor<'tcx> for Collector<'_, 'tcx> {
    fn visit_local(&mut self, local: Local, context: PlaceContext, location: Location) {
        let Some((_, definition)) = self.candidates.get(&local) else { return };
        // `StorageLive`, `StorageDead` and the other non-uses do not read the value.
        if matches!(context, PlaceContext::NonUse(_)) {
            return;
        }
        if location == *definition || self.switched.contains(&(local, location)) {
            return;
        }
        self.rejected.insert(local);
    }
}
