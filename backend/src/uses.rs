//! What the MIR body does with each of its locals.
//!
//! One pre-pass over the body counts, per local, how often it is assigned, how often it is read,
//! and whether a reference to it ever escapes. Three questions are answered from those counts, and
//! `queue.rs` asks all three:
//!
//! * **Inlinable** — assigned once, read once, never borrowed, not a parameter and not the return
//!   place. The expression the assignment computes can move to the one place that reads it.
//! * **Dead** — assigned but never read, never borrowed. The assignment can lose its target: what
//!   is left is the expression, kept only for its effects.
//! * **Immutable** — assigned at most once (a parameter counts as its own assignment) and never
//!   borrowed. Reading one is reading a value that cannot change, which is what lets the queue
//!   classify such a read as pure and move it freely.
//!
//! # Counting through projections
//!
//! `_1.x` reads `_1` and `_1.x = 3` writes through it, so both are recorded against `_1` — the
//! read as a use, the write as a *projection* def. The distinction matters: a local written
//! through a projection is not single-assignment even if its bare form is assigned once, so
//! neither inlining nor the immutability that makes a read pure applies to it.
//!
//! rustc's own visitor already collapses a projected place's context to
//! [`MutatingUseContext::Projection`] / [`NonMutatingUseContext::Projection`], so a context that
//! is *not* `Projection` is a use of the bare local.
//!
//! # Statements the backend does not lower
//!
//! A `view-abi` marker's `&root` argument is read at codegen time rather than lowered, and the
//! assignment that computed it is dropped with it (`abi.rs`). Such a borrow is not in the emitted
//! program at all, so it must not force its local into a box: `Uses::of` takes the locals whose
//! assignments are dropped and discounts the borrows those assignments take.
//!
//! Only the boxing question is discounted. The counts stay as MIR wrote them, so a local a dropped
//! statement mentions is still not inlinable, not dead and not immutable, which is the answer that
//! declines an optimization rather than the one that changes what runs.
//!
//! # Conservative in the right direction
//!
//! Counts come from MIR, but the substitution `queue.rs` performs is on the JavaScript the
//! lowering produced, and one MIR use does not always come out as exactly one mention of the
//! local (`&mut xs[i]` names its base twice, a `Subslice` three times). Over-counting is
//! therefore harmless — it only declines an optimization — and under-counting would not be, so
//! anything ambiguous counts as a use. Cleanup blocks are counted too, even though the backend
//! never lowers them, for the same reason.

use std::collections::HashSet;

use rustc_middle::mir::visit::{
    MutatingUseContext, NonMutatingUseContext, PlaceContext, Visitor,
};
use rustc_middle::mir::{
    Body, Local, Location, Place, PlaceElem, RETURN_PLACE, StatementKind,
};
use rustc_middle::ty::{self, Instance, Ty, TyCtxt, TypeFoldable};

use crate::value::{transparent_field, typing_env};

/// What one local is used for.
#[derive(Clone, Copy, Default)]
struct LocalUse {
    /// Assignments to the bare local: `_5 = ..`, and a call whose destination it is.
    defs: u32,
    /// Writes through a projection: `_5.x = ..`, `(*_5) = ..`, a `SetDiscriminant`.
    proj_defs: u32,
    /// Reads, including reads of the base of a projection.
    uses: u32,
    /// Whether a reference to the local, or into it, is ever taken.
    borrowed: bool,
    /// Whether a reference to the local *itself* is ever taken: `&_5`, not `&_5.x`.
    ///
    /// A subset of [`LocalUse::borrowed`], and the question boxing is decided by — see
    /// [`Uses::bare_borrowed`].
    bare_borrowed: bool,
}

/// Per-local use counts for one body.
pub(crate) struct Uses {
    locals: Vec<LocalUse>,
    arg_count: usize,
}

impl Uses {
    /// Counts every mention of every local in `mir`.
    ///
    /// The instance is what makes the type questions below answerable: the body is the generic
    /// one, so a place's type has to be monomorphized before anything may be concluded from it.
    ///
    /// `dropped` flags the locals whose assignments the backend will not lower, indexed by local;
    /// pass `&[]` for a body that has none. See the module docs for what those discount and what
    /// they deliberately do not.
    pub(crate) fn of<'tcx>(
        tcx: TyCtxt<'tcx>,
        instance: Instance<'tcx>,
        mir: &Body<'tcx>,
        dropped: &[bool],
    ) -> Uses {
        let mut collector = Collector {
            tcx,
            instance,
            local_decls: mir.local_decls.iter().map(|decl| decl.ty).collect(),
            locals: vec![LocalUse::default(); mir.local_decls.len()],
            through_deref: false,
            dropped: dropped_locations(mir, dropped),
        };
        collector.visit_body(mir);
        Uses { locals: collector.locals, arg_count: mir.arg_count }
    }

    fn get(&self, local: Local) -> LocalUse {
        self.locals.get(local.as_usize()).copied().unwrap_or_default()
    }

    /// Whether the local is a parameter, which is bound by the function signature and so is
    /// neither declared nor assignable by the body's own prelude.
    fn is_arg(&self, local: Local) -> bool {
        local.as_usize() >= 1 && local.as_usize() <= self.arg_count
    }

    /// Whether the one expression assigned to this local may move to the one place that reads it.
    ///
    /// The return place is excluded because it is what a `return` reads, and a parameter because
    /// its "assignment" is the call.
    pub(crate) fn inlinable(&self, local: Local) -> bool {
        let use_ = self.get(local);
        use_.defs == 1
            && use_.uses == 1
            && use_.proj_defs == 0
            && !use_.borrowed
            && !self.is_arg(local)
            && local != RETURN_PLACE
    }

    /// Whether nothing ever reads this local, so an assignment to it only needs its right hand
    /// side.
    ///
    /// The return place qualifies only in a function whose return value is zero sized: `return;`
    /// says everything such a function can say, so what was stored in `_0` does not matter.
    pub(crate) fn dead(&self, local: Local, unit_return: bool) -> bool {
        let use_ = self.get(local);
        use_.defs >= 1
            && use_.uses == 0
            && use_.proj_defs == 0
            && !use_.borrowed
            && !self.is_arg(local)
            && (local != RETURN_PLACE || unit_return)
    }

    /// Whether the local can be a `const` declared where it is assigned.
    ///
    /// One assignment and no borrow: nothing rebinds the name later, and no accessor's setter can
    /// either. Writes *through* the binding are fine — `const p = {}; p.x = 3` is legal — so a
    /// projection def does not disqualify it.
    pub(crate) fn constant(&self, local: Local) -> bool {
        let use_ = self.get(local);
        use_.defs == 1 && !use_.borrowed && !self.is_arg(local) && local != RETURN_PLACE
    }

    /// Whether a pointer to the local *itself* is ever taken, rather than into it.
    ///
    /// This is what decides that a local is **boxed**: a pointer is a `{ buf, off }` slot, and a
    /// bare JavaScript `let` is not a place any slot can name, so a local of a type whose values
    /// are JS primitives is declared as a one element array (`let x = []`, used as `x[0]`,
    /// with a parameter re-bound in the function prelude) as soon as something points at it. A
    /// local nothing points at, or one whose value is an object with an identity of its own, is
    /// left alone. See `ptr.rs` and `base.rs`.
    ///
    /// A boxed local is by construction also [`LocalUse::borrowed`], so it is excluded from
    /// inlining, from the dead-store rule and from the immutability that makes a read pure —
    /// which is what keeps the queue from moving a read of `x[0]` across a write through a
    /// pointer to it.
    pub(crate) fn bare_borrowed(&self, local: Local) -> bool {
        self.get(local).bare_borrowed
    }

    /// Whether the local's value cannot change once it is there.
    ///
    /// A parameter counts as assigned by the call, so one that the body also assigns is mutable.
    /// Reading an immutable local is pure: there is no later write for the read to be moved past.
    pub(crate) fn immutable(&self, local: Local) -> bool {
        let use_ = self.get(local);
        let defs = use_.defs + u32::from(self.is_arg(local));
        defs <= 1 && use_.proj_defs == 0 && !use_.borrowed
    }
}

struct Collector<'tcx> {
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
    /// The declared type of each local, in local order, so that a place's type can be walked
    /// without borrowing the body the visitor is already walking.
    local_decls: Vec<ty::Ty<'tcx>>,
    locals: Vec<LocalUse>,
    /// Whether the place being visited starts with a `Deref`, so that a write into it is a write
    /// *through* the local rather than to it. See [`Collector::visit_local`].
    through_deref: bool,
    /// The statements the backend will not lower, so that a borrow one of them takes forces no
    /// box. See the module docs.
    dropped: HashSet<Location>,
}

impl<'tcx> Collector<'tcx> {
    fn monomorphize<T>(&self, value: T) -> T
    where
        T: TypeFoldable<TyCtxt<'tcx>> + Copy,
    {
        self.instance.instantiate_mir_and_normalize_erasing_regions(
            self.tcx,
            typing_env(),
            ty::EarlyBinder::bind(self.tcx, value),
        )
    }

    /// Whether a place still names its local: every projection on it produces no JavaScript at
    /// all, so a reference to the place is a reference to the local itself.
    ///
    /// A downcast and an opaque cast are the obvious ones. The interesting one is a **field of a
    /// transparent aggregate** — `&t.0` on a `repr(transparent)` struct, `&mu.value` on a
    /// `MaybeUninit` — which reads as the whole value (`value.rs`), so the borrow is bare and the
    /// local has to be boxed for the slot to have somewhere to point. Without this the boxing pass
    /// and the lowering disagree, and the lowering is the one that has to report it.
    fn names_the_local(&self, place: &Place<'tcx>) -> bool {
        for (base, elem) in place.iter_projections() {
            match elem {
                PlaceElem::Downcast(..)
                | PlaceElem::OpaqueCast(_)
                | PlaceElem::UnwrapUnsafeBinder(_) => {}
                PlaceElem::Field(..) => {
                    let Some(base_ty) = self.projected_ty(base.local, base.projection) else {
                        return false;
                    };
                    if transparent_field(self.tcx, base_ty).is_none() {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        true
    }

    /// The monomorphized type a local reaches through a run of projections, or `None` if this pass
    /// cannot work it out.
    fn projected_ty(
        &self,
        local: Local,
        projection: &[PlaceElem<'tcx>],
    ) -> Option<Ty<'tcx>> {
        let &local_ty = self.local_decls.get(local.as_usize())?;
        let ty = projection.iter().fold(local_ty, |ty, elem| {
            rustc_middle::mir::PlaceTy::from_ty(ty).projection_ty(self.tcx, *elem).ty
        });
        Some(self.monomorphize(ty))
    }

    /// Whether this use is a `Drop` of a place that really has drop glue.
    ///
    /// Dropping takes a reference to the place, and where the place's JavaScript value is a
    /// primitive that reference is a slot, so the local has to be boxed for the slot to have
    /// somewhere to point. `Box` is what makes this reachable: a box *is* the pointer it owns
    /// (`value.rs`), so it is one of the few types that are direct *and* droppable.
    ///
    /// The glue is asked about rather than assumed. A `Drop` terminator survives monomorphization
    /// even where the type turns out to need no drop at all, and such a terminator produces no
    /// JavaScript, so it forces nothing into a box.
    fn drops_with_glue(&self, place: &Place<'tcx>, context: PlaceContext) -> bool {
        if !matches!(context, PlaceContext::MutatingUse(MutatingUseContext::Drop)) {
            return false;
        }
        self.projected_ty(place.local, place.projection)
            .is_some_and(|ty| ty.needs_drop(self.tcx, typing_env()))
    }
}

impl<'tcx> Visitor<'tcx> for Collector<'tcx> {
    /// Borrows are recorded here rather than in [`Collector::visit_local`], because that one can
    /// no longer tell `&_5` from `&_5.x`: `super_place` collapses a projected place's context to
    /// `Projection` before it gets there.
    ///
    /// A projection that produces no JavaScript at all — a downcast, an opaque cast — leaves the
    /// borrow bare: `&(_5 as Variant)` still points at the whole of `_5`.
    fn visit_place(&mut self, place: &Place<'tcx>, context: PlaceContext, location: Location) {
        if (is_borrow(context) || self.drops_with_glue(place, context))
            && self.names_the_local(place)
            && !self.dropped.contains(&location)
        {
            if let Some(slot) = self.locals.get_mut(place.local.as_usize()) {
                slot.bare_borrowed = true;
            }
        }
        let outer = self.through_deref;
        self.through_deref = matches!(place.projection.first(), Some(PlaceElem::Deref));
        self.super_place(place, context, location);
        self.through_deref = outer;
    }

    fn visit_local(&mut self, local: Local, context: PlaceContext, _location: Location) {
        let Some(slot) = self.locals.get_mut(local.as_usize()) else { return };
        match context {
            // Storage markers and debug info do not read or write anything.
            PlaceContext::NonUse(_) => {}
            PlaceContext::MutatingUse(context) => match context {
                MutatingUseContext::Store
                | MutatingUseContext::Call
                | MutatingUseContext::Yield
                | MutatingUseContext::AsmOutput => slot.defs += 1,
                // A write that reaches into the local rather than replacing it.
                MutatingUseContext::SetDiscriminant if !self.through_deref => slot.proj_defs += 1,
                MutatingUseContext::Projection | MutatingUseContext::SetDiscriminant => {
                    // `(*_5) = ..` writes through `_5`, and leaves `_5` itself alone: a pointer is
                    // an immutable record naming a place, so the only thing this does to the local
                    // is read it. A projection that does *not* start with a `Deref` — `_5.x = ..`
                    // — really does reach into the local's own value.
                    if !self.through_deref {
                        slot.proj_defs += 1;
                    }
                    slot.uses += 1;
                }
                // Dropping takes a reference to the place, exactly as a borrow does.
                MutatingUseContext::Borrow
                | MutatingUseContext::RawBorrow
                | MutatingUseContext::Drop => {
                    slot.borrowed = true;
                    slot.uses += 1;
                }
                MutatingUseContext::Retag => slot.uses += 1,
            },
            PlaceContext::NonMutatingUse(context) => match context {
                NonMutatingUseContext::SharedBorrow
                | NonMutatingUseContext::FakeBorrow
                | NonMutatingUseContext::RawBorrow => {
                    slot.borrowed = true;
                    slot.uses += 1;
                }
                _ => slot.uses += 1,
            },
        }
    }
}

/// Where the assignments to `dropped` locals are, so the visitor can tell a borrow that reaches the
/// emitted program from one that does not.
///
/// Only an `Assign` counts: those are the statements `base.rs` skips, and every other statement is
/// lowered whatever its target.
fn dropped_locations<'tcx>(mir: &Body<'tcx>, dropped: &[bool]) -> HashSet<Location> {
    let mut locations = HashSet::new();
    if dropped.iter().all(|flag| !flag) {
        return locations;
    }
    for (block, data) in mir.basic_blocks.iter_enumerated() {
        for (statement_index, statement) in data.statements.iter().enumerate() {
            let StatementKind::Assign(assignment) = &statement.kind else { continue };
            let Some(local) = assignment.0.as_local() else { continue };
            if dropped.get(local.as_usize()).copied().unwrap_or(false) {
                locations.insert(Location { block, statement_index });
            }
        }
    }
    locations
}

/// Whether this context takes a pointer to the place, of the kind that has to be able to name it.
///
/// A fake borrow takes one that produces no code, so it does not count here; it still sets
/// [`LocalUse::borrowed`], which is the conservative half of the pair. A `Drop` takes a reference
/// too, and whether *that* forces a box depends on the type -- see [`Collector::drops_with_glue`].
fn is_borrow(context: PlaceContext) -> bool {
    matches!(
        context,
        PlaceContext::MutatingUse(MutatingUseContext::Borrow | MutatingUseContext::RawBorrow)
            | PlaceContext::NonMutatingUse(
                NonMutatingUseContext::SharedBorrow | NonMutatingUseContext::RawBorrow
            )
    )
}

