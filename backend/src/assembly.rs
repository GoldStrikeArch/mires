//! Aggregates MIR assembles in place, and where their JavaScript container is made.
//!
//! MIR can give a local its value one field at a time, through projections, with no assignment
//! of the whole: rustc's clone shim for a tuple or a closure is
//!
//! ```text
//! (_0.0: String) = <String as Clone>::clone(move _2) -> ..;
//! (_0.1: usize) = copy ((*_1).1: usize);
//! return;
//! ```
//!
//! and its MIR inliner splices that body into callers, where the return place becomes the call's
//! destination local. In native code the local's storage is there before any field is written.
//! Here an aggregate is a JavaScript array or object, and nothing in that body creates one: `_0`
//! was declared `let _0;` and the first field write threw `Cannot set properties of undefined`.
//!
//! So such a local is given an empty container (`[]` for a tuple, an array or a closure, `{}` for
//! a struct or an object-form enum, `value::fresh_container`) wherever its storage starts over and
//! a field write can follow before the whole local is assigned:
//!
//! * at the top of the function, in its declaration, for a local that is not a parameter;
//! * at its `StorageLive`;
//! * after a statement or a call that moves it out whole. The object that left is the moved
//!   value's now, and assembling the next value in it would write into that value. MIR built
//!   without storage markers (`-Copt-level=0`) has nothing else between two iterations of a loop
//!   that assemble and move one local.
//!
//! "Can follow" is a backward liveness: a field write (an assignment, a call destination or a
//! `SetDiscriminant` through a projection that does not start with a deref) needs the container,
//! and a whole assignment, a `StorageLive`, a `StorageDead` and a whole move each end the question
//! for what comes before them. A local that is assigned whole before every field write it gets,
//! which is all of the MIR rustc builds from source, needs nothing here and nothing changes for it.
//!
//! The JIR lane answers the same question over JIR-S in `jsc-link/src/lower/assembly.rs`.

use std::collections::{HashMap, HashSet};

use rustc_middle::mir::{BasicBlock, Local, UnwindAction};
use rustc_middle::mir::{
    Body, Location, Operand, Place, PlaceElem, Rvalue, StatementKind, TerminatorKind,
};

use crate::base::FnCx;
use crate::jsast::Expr;

/// Where the locals of one body that are assembled in place get a fresh container.
#[derive(Default)]
pub(crate) struct Assembly {
    /// The container each such local is given, by local.
    containers: HashMap<Local, Expr>,
    /// Locals whose declaration creates the container.
    entry: HashSet<Local>,
    /// Locals given a new container right after the statement (or terminator) at a location.
    after: HashMap<Location, Vec<Local>>,
}

impl Assembly {
    /// The container `local`'s declaration starts with, if any.
    pub(crate) fn at_entry(&self, local: Local) -> Option<&Expr> {
        if self.entry.contains(&local) {
            self.containers.get(&local)
        } else {
            None
        }
    }

    /// Whether `local` is given a container anywhere: its declaration or a later assignment is
    /// then not the only thing that binds it.
    pub(crate) fn assembles(&self, local: Local) -> bool {
        self.entry.contains(&local) || self.after.values().any(|locals| locals.contains(&local))
    }

    /// The locals given a fresh container right after `location`, with the container each gets.
    pub(crate) fn after(&self, location: Location) -> impl Iterator<Item = (Local, &Expr)> {
        self.after
            .get(&location)
            .into_iter()
            .flatten()
            .filter_map(|local| Some((*local, self.containers.get(local)?)))
    }

    pub(crate) fn of<'tcx>(fx: &FnCx<'_, 'tcx>, mir: &Body<'tcx>) -> Assembly {
        let mut containers = HashMap::new();
        for local in mir.local_decls.indices() {
            let ty = fx.monomorphize(mir.local_decls[local].ty);
            if let Some(container) = crate::value::fresh_container(fx.tcx, ty) {
                containers.insert(local, container);
            }
        }
        // Only a local that some field write reaches into can need one.
        let written: HashSet<Local> = written_in_place(mir)
            .into_iter()
            .filter(|local| containers.contains_key(local))
            .collect();
        if written.is_empty() {
            return Assembly::default();
        }
        containers.retain(|local, _| written.contains(local));

        // `need_in[b]`: the locals whose container block `b` may write into before anything
        // replaces or ends it.
        let blocks = mir.basic_blocks.len();
        let mut need_in: Vec<HashSet<Local>> = vec![HashSet::new(); blocks];
        let mut changed = true;
        while changed {
            changed = false;
            for (bb, _) in mir.basic_blocks.iter_enumerated().rev() {
                let need = block_need(mir, bb, &need_in, &written, &mut |_, _, _| {});
                if need != need_in[bb.as_usize()] {
                    need_in[bb.as_usize()] = need;
                    changed = true;
                }
            }
        }

        let mut assembly = Assembly {
            containers,
            ..Assembly::default()
        };
        for (bb, _) in mir.basic_blocks.iter_enumerated() {
            block_need(
                mir,
                bb,
                &need_in,
                &written,
                &mut |location, local, needed| {
                    if needed {
                        assembly.after.entry(location).or_default().push(local);
                    }
                },
            );
        }
        for local in &written {
            let is_arg = local.as_usize() >= 1 && local.as_usize() <= mir.arg_count;
            if !is_arg && need_in[0].contains(local) {
                assembly.entry.insert(*local);
            }
        }
        assembly
    }
}

/// The locals written through a projection of their own storage: a field, an element or a
/// variant of the local itself, not of something it points to.
fn written_in_place(mir: &Body<'_>) -> HashSet<Local> {
    let mut written = HashSet::new();
    for data in mir.basic_blocks.iter() {
        for statement in &data.statements {
            match &statement.kind {
                StatementKind::Assign(assignment) if into_place(&assignment.0) => {
                    written.insert(assignment.0.local);
                }
                StatementKind::SetDiscriminant { place, .. } if !through_deref(place) => {
                    written.insert(place.local);
                }
                _ => {}
            }
        }
        if let TerminatorKind::Call { destination, .. } = &data.terminator().kind {
            if into_place(destination) {
                written.insert(destination.local);
            }
        }
    }
    written
}

/// A write into the local's own storage rather than of the whole local or through a pointer.
fn into_place(place: &Place<'_>) -> bool {
    !place.projection.is_empty() && !through_deref(place)
}

fn through_deref(place: &Place<'_>) -> bool {
    matches!(place.projection.first(), Some(PlaceElem::Deref))
}

/// The local a whole `move` operand takes, if it is one.
fn moved_whole(operand: &Operand<'_>) -> Option<Local> {
    match operand {
        Operand::Move(place) if place.projection.is_empty() => Some(place.local),
        _ => None,
    }
}

fn rvalue_moves(rvalue: &Rvalue<'_>, out: &mut Vec<Local>) {
    let mut operand = |operand: &Operand<'_>| out.extend(moved_whole(operand));
    match rvalue {
        Rvalue::Use(value, _)
        | Rvalue::Repeat(value, _)
        | Rvalue::Cast(_, value, _)
        | Rvalue::UnaryOp(_, value)
        | Rvalue::WrapUnsafeBinder(value, _) => operand(value),
        Rvalue::BinaryOp(_, operands) => {
            operand(&operands.0);
            operand(&operands.1);
        }
        Rvalue::Aggregate(_, operands) => operands.iter().for_each(operand),
        _ => {}
    }
}

/// Runs block `bb` backwards from its successors' `need_in` and returns its own. `event` is told,
/// at every point a local's storage starts over (a `StorageLive`, a whole move), whether the local
/// needs a container right after it: the location is the statement's, or the terminator's for a
/// call, whose container belongs on its return edge.
fn block_need(
    mir: &Body<'_>,
    bb: BasicBlock,
    need_in: &[HashSet<Local>],
    written: &HashSet<Local>,
    event: &mut dyn FnMut(Location, Local, bool),
) -> HashSet<Local> {
    let data = &mir.basic_blocks[bb];
    let terminator = data.terminator();
    let at_terminator = Location {
        block: bb,
        statement_index: data.statements.len(),
    };
    let mut need: HashSet<Local> = HashSet::new();
    match &terminator.kind {
        TerminatorKind::Call {
            args,
            destination,
            target,
            ..
        } => {
            // The destination is written on the return edge only.
            if let Some(target) = target {
                need.extend(need_in[target.as_usize()].iter().copied());
                write_effect(&mut need, destination, written);
            }
            let mut moved = Vec::new();
            for arg in args.iter() {
                moved.extend(moved_whole(&arg.node));
            }
            for local in moved.into_iter().filter(|local| written.contains(local)) {
                event(at_terminator, local, need.contains(&local));
                need.remove(&local);
            }
            if let Some(UnwindAction::Cleanup(cleanup)) = terminator.unwind() {
                need.extend(need_in[cleanup.as_usize()].iter().copied());
            }
        }
        _ => {
            for successor in terminator.successors() {
                need.extend(need_in[successor.as_usize()].iter().copied());
            }
        }
    }
    for (index, statement) in data.statements.iter().enumerate().rev() {
        let location = Location {
            block: bb,
            statement_index: index,
        };
        match &statement.kind {
            StatementKind::Assign(assignment) => {
                write_effect(&mut need, &assignment.0, written);
                let mut moved = Vec::new();
                rvalue_moves(&assignment.1, &mut moved);
                for local in moved.into_iter().filter(|local| written.contains(local)) {
                    event(location, local, need.contains(&local));
                    need.remove(&local);
                }
            }
            StatementKind::SetDiscriminant { place, .. } => {
                if !through_deref(place) && written.contains(&place.local) {
                    need.insert(place.local);
                }
            }
            StatementKind::StorageLive(local) if written.contains(local) => {
                event(location, *local, need.contains(local));
                need.remove(local);
            }
            StatementKind::StorageDead(local) => {
                need.remove(local);
            }
            _ => {}
        }
    }
    need
}

/// What writing `place` does to the question: a whole write replaces the container, a write into
/// one needs it.
fn write_effect(need: &mut HashSet<Local>, place: &Place<'_>, written: &HashSet<Local>) {
    if !written.contains(&place.local) {
        return;
    }
    if place.projection.is_empty() {
        need.remove(&place.local);
    } else if into_place(place) {
        need.insert(place.local);
    }
}
