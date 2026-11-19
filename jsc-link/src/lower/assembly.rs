//! Aggregates MIR assembles in place: where a local written field by field needs a fresh
//! container. v1 `backend/src/assembly.rs` asks the same question of rustc's MIR; this module asks
//! it of the JIR-S body, and the answer has to agree, because either lane may print an item.
//!
//! rustc's clone shim for a tuple or a closure writes `_0.0`, `_0.1`, .. and never `_0` as a whole,
//! and its MIR inliner splices that body into callers. An aggregate is a JavaScript array or
//! object, so such a local's storage is a container that something has to create, and create
//! again whenever the storage starts over: at the entry of the function, at `StorageLive`, and
//! after the local has been moved out whole (the object that left belongs to the moved value).
//!
//! A container is needed at such a point when a field write can follow it before anything replaces
//! or ends the local's storage: a backward liveness in which a write through a projection of the
//! local's own storage needs the container, and a whole write, a `StorageLive`, a `StorageDead`
//! and a whole move end the question. A local assigned whole before every field write it gets --
//! everything rustc builds from source -- has no such point.
//!
//! `ssa.rs` makes every local with a point a cell and places a `CellAlloc` at each point after the
//! entry; `js/emit.rs` spells a `CellAlloc` of a container-typed cell as the empty container when
//! the cell's storage is written in place before it is next replaced (`fuse::fresh_cell_allocs`).

use std::collections::{BTreeMap, BTreeSet};

use crate::jir::semantic::{
    Body, LocalId, LocalRole, Operand, OperandKind, Place, Projection, Rvalue, StatementKind,
    Terminator, Unwind,
};
use crate::jir::types::TypeTable;

/// Where the locals of one body that are assembled in place need a fresh container.
#[derive(Debug, Default)]
pub struct Assembly {
    /// Locals that need one from the entry of the function.
    pub entry: BTreeSet<LocalId>,
    /// Locals that need one right after statement `(block, index)`: its `StorageLive`, or the
    /// statement that moved it out.
    pub after_statement: BTreeMap<(u32, usize), Vec<LocalId>>,
    /// Locals that need one on the return edge of the call that ends block `block`, which moved
    /// them out.
    pub after_call: BTreeMap<u32, Vec<LocalId>>,
}

impl Assembly {
    pub fn of(body: &Body, table: &TypeTable) -> Assembly {
        let written: BTreeSet<LocalId> = written_in_place(body)
            .into_iter()
            .filter(|local| {
                body.locals.get(local.0 as usize).is_some_and(|decl| {
                    crate::lower::js::value::fresh_container(table, decl.ty).is_some()
                })
            })
            .collect();
        if written.is_empty() {
            return Assembly::default();
        }
        let blocks = body.blocks.len();
        let mut need_in: Vec<BTreeSet<LocalId>> = vec![BTreeSet::new(); blocks];
        let mut changed = true;
        while changed {
            changed = false;
            for block in (0..blocks).rev() {
                let need = block_need(body, block, &need_in, &written, &mut |_, _| {});
                if need != need_in[block] {
                    need_in[block] = need;
                    changed = true;
                }
            }
        }
        let mut assembly = Assembly::default();
        for block in 0..blocks {
            block_need(
                body,
                block,
                &need_in,
                &written,
                &mut |point, local| match point {
                    Point::Statement(index) => assembly
                        .after_statement
                        .entry((block as u32, index))
                        .or_default()
                        .push(local),
                    Point::Call => assembly
                        .after_call
                        .entry(block as u32)
                        .or_default()
                        .push(local),
                },
            );
        }
        if let Some(entry) = need_in.first() {
            for local in entry {
                let is_arg = body
                    .locals
                    .get(local.0 as usize)
                    .is_some_and(|decl| decl.role == LocalRole::Argument);
                if !is_arg {
                    assembly.entry.insert(*local);
                }
            }
        }
        assembly
    }

    /// Every local that needs a container somewhere.
    pub fn locals(&self) -> BTreeSet<LocalId> {
        let mut locals = self.entry.clone();
        for list in self
            .after_statement
            .values()
            .chain(self.after_call.values())
        {
            locals.extend(list.iter().copied());
        }
        locals
    }
}

/// Where a point that needs a container is.
enum Point {
    Statement(usize),
    Call,
}

/// The locals written through a projection of their own storage.
fn written_in_place(body: &Body) -> BTreeSet<LocalId> {
    let mut written = BTreeSet::new();
    for block in &body.blocks {
        for statement in &block.statements {
            match &statement.kind {
                StatementKind::Assign { destination, .. } if into_place(destination) => {
                    written.insert(destination.local);
                }
                StatementKind::SetDiscriminant { place, .. } if !through_deref(place) => {
                    written.insert(place.local);
                }
                _ => {}
            }
        }
        if let Terminator::Call(call) = &block.terminator {
            if into_place(&call.destination) {
                written.insert(call.destination.local);
            }
        }
    }
    written
}

fn into_place(place: &Place) -> bool {
    !place.projection.is_empty() && !through_deref(place)
}

fn through_deref(place: &Place) -> bool {
    matches!(place.projection.first(), Some(Projection::Deref))
}

fn moved_whole(operand: &Operand) -> Option<LocalId> {
    match &operand.kind {
        OperandKind::Move(place) if place.projection.is_empty() => Some(place.local),
        _ => None,
    }
}

fn rvalue_moves(rvalue: &Rvalue, out: &mut Vec<LocalId>) {
    match rvalue {
        Rvalue::Use(operand)
        | Rvalue::Repeat { value: operand, .. }
        | Rvalue::Cast { value: operand, .. }
        | Rvalue::Unary { value: operand, .. }
        | Rvalue::WrapUnsafeBinder(operand) => out.extend(moved_whole(operand)),
        Rvalue::Binary { left, right, .. } => {
            out.extend(moved_whole(left));
            out.extend(moved_whole(right));
        }
        Rvalue::Aggregate { operands, .. } => out.extend(operands.iter().filter_map(moved_whole)),
        Rvalue::Reference { .. }
        | Rvalue::RawPointer { .. }
        | Rvalue::Discriminant(_)
        | Rvalue::CopyForDeref(_)
        | Rvalue::Reborrow { .. } => {}
    }
}

/// What writing `place` does to the question: a whole write replaces the container, a write into
/// one needs it.
fn write_effect(need: &mut BTreeSet<LocalId>, place: &Place, written: &BTreeSet<LocalId>) {
    if !written.contains(&place.local) {
        return;
    }
    if place.projection.is_empty() {
        need.remove(&place.local);
    } else if into_place(place) {
        need.insert(place.local);
    }
}

/// Runs block `block` backwards from its successors' `need_in` and returns its own, telling
/// `event` about every point where a local that needs a container starts its storage over.
fn block_need(
    body: &Body,
    block: usize,
    need_in: &[BTreeSet<LocalId>],
    written: &BTreeSet<LocalId>,
    event: &mut dyn FnMut(Point, LocalId),
) -> BTreeSet<LocalId> {
    let data = &body.blocks[block];
    let successor =
        |id: crate::jir::semantic::BlockId| need_in.get(id.0 as usize).cloned().unwrap_or_default();
    let mut need = BTreeSet::new();
    match &data.terminator {
        Terminator::Call(call) => {
            // The destination is written on the return edge only.
            if let Some(target) = call.target {
                need.extend(successor(target));
                write_effect(&mut need, &call.destination, written);
            }
            let moved: Vec<LocalId> = call.arguments.iter().filter_map(moved_whole).collect();
            for local in moved.into_iter().filter(|local| written.contains(local)) {
                if need.remove(&local) {
                    event(Point::Call, local);
                }
            }
            if let Unwind::Cleanup(cleanup) = call.unwind {
                need.extend(successor(cleanup));
            }
        }
        terminator => {
            for target in crate::jir::summary::successors(terminator) {
                need.extend(successor(target));
            }
        }
    }
    for (index, statement) in data.statements.iter().enumerate().rev() {
        match &statement.kind {
            StatementKind::Assign { destination, value } => {
                write_effect(&mut need, destination, written);
                let mut moved = Vec::new();
                rvalue_moves(value, &mut moved);
                for local in moved.into_iter().filter(|local| written.contains(local)) {
                    if need.remove(&local) {
                        event(Point::Statement(index), local);
                    }
                }
            }
            StatementKind::SetDiscriminant { place, .. } => {
                if !through_deref(place) && written.contains(&place.local) {
                    need.insert(place.local);
                }
            }
            StatementKind::StorageLive(local) => {
                if need.remove(local) {
                    event(Point::Statement(index), *local);
                }
            }
            StatementKind::StorageDead(local) => {
                need.remove(local);
            }
            StatementKind::Assume(_)
            | StatementKind::CopyNonOverlapping { .. }
            | StatementKind::Nop => {}
        }
    }
    need
}
