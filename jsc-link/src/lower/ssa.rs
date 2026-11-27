//! SSA construction: a JIR-S body becomes a JIR-R [`Func`].
//!
//! The algorithm is Braun et al.'s on-the-fly construction ("Simple and Efficient Construction of
//! Static Single Assignment Form", CC 2013) over the reachable blocks of the body: every MIR
//! local that stays a value has a current definition per block, a read that finds none in the
//! current block asks the predecessors, a block with several predecessors gets a block parameter
//! (JIR-R's phi), and a parameter whose operands all agree is removed again. Blocks are filled in
//! reverse postorder and sealed once every predecessor is filled, so a loop header keeps an
//! incomplete parameter until its back edge arrives.
//!
//! # Which locals are cells
//!
//! A local is a [`CellDef`] rather than a run of SSA values when it is:
//!
//! * **address-taken and direct** — a reference, raw pointer, or drop with glue is taken of the
//!   local itself (v1 `uses.rs` `bare_borrowed`: no projection, or only downcasts, opaque casts,
//!   and fields of transparent wrappers) and the type is not indirect. Such a cell is `boxed`
//!   (`[v]` in the baseline), the one place a `{ buf, off }` slot can point at. A borrow an
//!   argument on its way to a `view-abi` marker takes is discounted, as v1 does (`consumed`).
//! * **written through projections and never assigned whole** — the storage has to exist before the
//!   first field write, and a cell's does.
//! * **read where no definition reaches** — a drop guarded by a drop flag reads a local that one
//!   path never initialized; JIR-R has no undefined value to pass along that edge, and a cell is
//!   exactly storage that may be uninitialized. Found in the first pass and demoted for a second.
//!
//! Everything else is SSA values: aggregates included, written through projections with
//! `SetField`/`StoreThrough` on the reaching value's storage (JIR-R's value semantics say that is
//! the storage the borrow checker proved unique).
//!
//! # Moves
//!
//! A MIR `move` of a bare local whose type has an object identity (`needs_clone`) becomes a
//! `Move` op, so the verifier's use-after-move check has teeth; a local a `Drop` terminator names
//! is exempt, because a drop guarded by a drop flag legitimately follows a move on another path.
//! Should the verifier still reject a move, the function is rebuilt without any.
//!
//! # What declines
//!
//! Every shape this pass cannot express is a [`Decline::Lower`] naming the class: an
//! `async_drop` edge, a `dyn` pointer built from parts, a `copy_nonoverlapping` statement whose
//! name the interns lack, a `SetDiscriminant` of a non-enum, a spread argument that is not a
//! tuple, a JIR-R function the verifier rejects for anything but a move. Two shapes are carried
//! with a stand-in rather than declined, since the lowering never reads the type in question: a
//! `#[track_caller]` body whose `Location` type the table does not describe (the `$loc`
//! parameter takes the signature's result type) and a synthetic view of a pointer-typed place
//! with no reference type in the table (`plan.rs`, "Synthetic views").

use std::collections::{BTreeMap, BTreeSet};

use crate::jir::decline::{Decline, LowerDecline, OpClass};
use crate::jir::interns::{Interns, StrId};
use crate::jir::origin::{OriginId, OriginTable};
use crate::jir::repr::{
    CellId, Func, FuncBuilder, OpKind, Proj, RAssertMessage, RBlockId, RTerm, Root, ValueId,
};
use crate::jir::semantic::{
    AggregateKind, AssertMessage, BlockId, Body, BodyKind, BorrowKind, Call, Callee, CastKind,
    ConstTree, Constant, LocalId, LocalRole, Operand, OperandKind, Place, Projection, Rvalue,
    StatementKind, TerminateReason, Terminator, TypeId, Unwind,
};
use crate::jir::types::{EnumRepr, NominalForm, TypeKind, TypeTable};
use crate::jir::VerifyErrorKind;

use super::plan::{placeholder_view_ok, NO_ORIGIN};

/// Builds the JIR-R function of `body`.
///
/// `origins` is the body's origin table when the object carried one; without it every op and
/// value carries [`NO_ORIGIN`]. The result passes `Func::verify` or this returns a decline.
#[cfg(test)]
pub fn build(
    body: &Body,
    origins: Option<&OriginTable>,
    table: &TypeTable,
    interns: &Interns,
) -> Result<Func, Decline> {
    build_with(body, origins, table, interns, false)
}

/// [`build`], with the choice v1's per-item optimizer made: `fold_runtime_checks` takes the
/// decided edge of a `SwitchInt` on a `RuntimeCheck` constant (`ub_checks`) and leaves the other
/// arm — the `precondition_check` call — unreachable, exactly as v1's `ast-dce` folded
/// `if (false)` before an item's references were taken. Pass `opts.applies(OptimizerPass::AstDce)`
/// so the lane's references agree with the baseline's item for item; folding when the baseline
/// did not (or the reverse) makes the two lanes reach different items.
pub fn build_with(
    body: &Body,
    origins: Option<&OriginTable>,
    table: &TypeTable,
    interns: &Interns,
    fold_runtime_checks: bool,
) -> Result<Func, Decline> {
    if !matches!(body.kind, BodyKind::Function) {
        // Named by what the body IS, so a report groups the two non-function kinds apart: they
        // have their own lowerings (`js::function::lower_static`, `lower_vtable`) and reaching
        // here at all means the caller dispatched wrongly.
        let op = match body.kind {
            BodyKind::Static { .. } => OpClass::Static,
            BodyKind::Vtable { .. } => OpClass::Vtable,
            BodyKind::Function => OpClass::Other,
        };
        return Err(decline(
            op,
            format!("{} bodies are not functions", body.kind.as_str()),
        ));
    }
    if body.blocks.is_empty() {
        return Err(decline(OpClass::Region, "function body has no blocks"));
    }
    let analysis = Analysis::of(body, table, interns);
    let mut cells: BTreeSet<LocalId> = analysis.cells.clone();
    let mut moves = true;
    // Two demotion rounds at most: the first pass finds every local read where no definition
    // reaches (demoting one local never creates such a read for another), and a verifier
    // rejection of a move switches moves off for one more.
    for _ in 0..3 {
        let mut builder = Builder::new(
            body,
            origins,
            table,
            interns,
            &analysis,
            &cells,
            (moves, fold_runtime_checks),
        )?;
        builder.run()?;
        if !builder.undefined.is_empty() {
            cells.extend(builder.undefined.iter().copied());
            continue;
        }
        let func = builder.finish()?;
        match func.verify(table) {
            Ok(()) => return Ok(func),
            Err(err) if moves && err.kind == VerifyErrorKind::UseAfterMove => {
                moves = false;
                continue;
            }
            Err(err) => {
                return Err(decline(
                    OpClass::Region,
                    format!("JIR-R does not verify: {err}"),
                ));
            }
        }
    }
    Err(decline(
        OpClass::Region,
        "SSA construction did not converge in three passes",
    ))
}

fn decline(op: OpClass, detail: impl Into<String>) -> Decline {
    Decline::Lower(LowerDecline::Other {
        op,
        detail: detail.into(),
    })
}

// -------------------------------------------------------------------------------------------
// The pre-pass: what the body does with each local
// -------------------------------------------------------------------------------------------

/// What one local is used for (v1 `uses.rs` `LocalUse`, over JIR-S).
#[derive(Clone, Copy, Default, Debug)]
struct LocalUse {
    /// Assignments to the bare local, and calls whose destination it is.
    defs: u32,
    /// Writes through a projection that does not start with a `Deref`.
    proj_defs: u32,
    /// Whether a reference to the local itself is taken.
    bare_borrowed: bool,
    /// Whether a `Drop` terminator names the local (bare).
    dropped: bool,
}

/// The pre-pass results shared by every construction pass.
struct Analysis {
    uses: Vec<LocalUse>,
    /// The locals that are cells before any demotion, with whether each is boxed.
    cells: BTreeSet<LocalId>,
    boxed: BTreeSet<LocalId>,
    /// Indirect `(local, variant, field)` places that are both borrowed (or have a sub-place
    /// borrowed, or their enclosing local borrowed) and overwritten as a single field: must
    /// overwrite in place instead of rebinding the JS property so synthetic addresses stay stable.
    identity_preserved_fields: BTreeSet<(LocalId, Option<u32>, u32)>,
    /// Indirect bare locals that are borrowed and assigned more than once: subsequent writes
    /// overwrite in place so synthetic addresses stay stable.
    identity_preserved_locals: BTreeSet<LocalId>,
    /// Where the locals assembled field by field need a fresh container.
    assembly: super::assembly::Assembly,
    /// `(pointee, mutable)` to a reference type in the table, for synthetic views. Built on first
    /// use: it walks the whole crate-scope table and most bodies never need it.
    references: std::cell::OnceCell<BTreeMap<(TypeId, bool), TypeId>>,
    /// The `caller_location` parameter type, when the body takes one and the table has it.
    location_ty: Option<TypeId>,
    /// The id of `"copy_nonoverlapping"` in the interns, if interned (looked up on first use).
    copy_nonoverlapping: std::cell::OnceCell<Option<StrId>>,
}

/// First non-`Deref` field projection `(downcast, field, field_ty)` on `place`, if any.
fn leading_field(place: &Place) -> Option<(Option<u32>, u32, TypeId)> {
    let mut downcast = None;
    for projection in &place.projection {
        match projection {
            Projection::Downcast { variant } => downcast = Some(*variant),
            Projection::OpaqueCast(_) | Projection::UnwrapUnsafeBinder(_) => {}
            Projection::Field { field, ty } => return Some((downcast, *field, *ty)),
            _ => return None,
        }
    }
    None
}

impl Analysis {
    fn of(body: &Body, table: &TypeTable, interns: &Interns) -> Analysis {
        let mut uses = vec![LocalUse::default(); body.locals.len()];
        let mut borrowed_indirect_locals: BTreeSet<LocalId> = BTreeSet::new();
        let mut borrowed_indirect_fields: BTreeSet<(LocalId, Option<u32>, u32)> = BTreeSet::new();
        let mut written_single_indirect_fields: BTreeSet<(LocalId, Option<u32>, u32)> =
            BTreeSet::new();
        let mut record_place = |uses: &mut Vec<LocalUse>,
                                place: &Place,
                                borrow: bool,
                                write: bool| {
            let Some(slot) = uses.get_mut(place.local.0 as usize) else {
                return;
            };
            if write {
                if place.projection.is_empty() {
                    slot.defs += 1;
                } else if !matches!(place.projection.first(), Some(Projection::Deref)) {
                    slot.proj_defs += 1;
                    let (downcast, field) = single_field(&place.projection);
                    if let Some(field) = field {
                        if table.is_indirect(place.ty) {
                            written_single_indirect_fields.insert((place.local, downcast, field));
                        }
                    }
                }
            }
            if borrow {
                if names_the_local(place, body, table) {
                    slot.bare_borrowed = true;
                    if let Some(local) = body.locals.get(place.local.0 as usize) {
                        if table.is_indirect(local.ty) {
                            borrowed_indirect_locals.insert(place.local);
                        }
                    }
                }
                if let Some((downcast, field, field_ty)) = leading_field(place) {
                    if table.is_indirect(field_ty) {
                        borrowed_indirect_fields.insert((place.local, downcast, field));
                    }
                }
            }
        };
        for block in &body.blocks {
            for statement in &block.statements {
                match &statement.kind {
                    StatementKind::Assign { destination, value } => {
                        record_place(&mut uses, destination, false, true);
                        match value {
                            Rvalue::Reference { place, .. }
                            | Rvalue::RawPointer { place, .. }
                            | Rvalue::Reborrow { place, .. } => {
                                record_place(&mut uses, place, true, false);
                            }
                            _ => {}
                        }
                    }
                    StatementKind::SetDiscriminant { place, .. } => {
                        record_place(&mut uses, place, false, true);
                    }
                    _ => {}
                }
            }
            match &block.terminator {
                Terminator::Call(call) => record_place(&mut uses, &call.destination, false, true),
                Terminator::Drop { place, glue, .. } => {
                    if place.projection.is_empty() {
                        if let Some(slot) = uses.get_mut(place.local.0 as usize) {
                            slot.dropped = true;
                        }
                    }
                    if !matches!(glue, crate::jir::semantic::DropGlue::None) {
                        record_place(&mut uses, place, true, false);
                    }
                }
                _ => {}
            }
        }
        let identity_preserved_fields: BTreeSet<(LocalId, Option<u32>, u32)> =
            written_single_indirect_fields
                .into_iter()
                .filter(|key| {
                    borrowed_indirect_fields.contains(key)
                        || borrowed_indirect_locals.contains(&key.0)
                })
                .collect();
        let identity_preserved_locals: BTreeSet<LocalId> = borrowed_indirect_locals
            .into_iter()
            .filter(|local| {
                let Some(decl) = body.locals.get(local.0 as usize) else {
                    return false;
                };
                uses[local.0 as usize].defs > 1
                    && !table.is_zst(decl.ty)
                    && matches!(
                        table.get(table.peeled(decl.ty)).map(|d| &d.kind),
                        Some(
                            TypeKind::Adt {
                                form: NominalForm::Struct,
                                ..
                            } | TypeKind::Tuple(_)
                                | TypeKind::Array { .. }
                        )
                    )
            })
            .collect();
        let mut cells = BTreeSet::new();
        let mut boxed = BTreeSet::new();
        // A local assembled in place needs its container made at points SSA values cannot
        // express (`assembly.rs`), so it is storage: a cell, allocated again at each point.
        let assembly = super::assembly::Assembly::of(body, table);
        cells.extend(assembly.locals());
        for local in &body.locals {
            let use_ = uses[local.id.0 as usize];
            let is_arg = local.role == LocalRole::Argument;
            if use_.bare_borrowed && !table.is_indirect(local.ty) {
                cells.insert(local.id);
                boxed.insert(local.id);
            } else if use_.proj_defs > 0 && use_.defs == 0 && !is_arg {
                cells.insert(local.id);
            }
        }
        let location_ty = if body.signature.abi.caller_location {
            location_type(body, table, interns)
        } else {
            None
        };
        Analysis {
            uses,
            cells,
            boxed,
            identity_preserved_fields,
            identity_preserved_locals,
            assembly,
            references: std::cell::OnceCell::new(),
            location_ty,
            copy_nonoverlapping: std::cell::OnceCell::new(),
        }
    }

    fn references(&self, table: &TypeTable) -> &BTreeMap<(TypeId, bool), TypeId> {
        self.references.get_or_init(|| {
            let mut references = BTreeMap::new();
            for descriptor in table.descriptors() {
                if let TypeKind::Reference { pointee, mutable } = descriptor.kind {
                    references
                        .entry((pointee, mutable))
                        .or_insert(descriptor.id);
                }
            }
            references
        })
    }

    fn copy_nonoverlapping(&self, interns: &Interns) -> Option<StrId> {
        *self.copy_nonoverlapping.get_or_init(|| {
            interns
                .strings
                .iter()
                .position(|s| s == "copy_nonoverlapping")
                .map(|index| StrId(index as u32))
        })
    }
}

/// Whether a place still names its local: every projection on it produces no JavaScript at all
/// (v1 `uses.rs` `names_the_local`).
fn names_the_local(place: &Place, body: &Body, table: &TypeTable) -> bool {
    let Some(local) = body.locals.get(place.local.0 as usize) else {
        return false;
    };
    let mut ty = local.ty;
    let mut variant: Option<u32> = None;
    for projection in &place.projection {
        match projection {
            Projection::Downcast { variant: v, .. } => variant = Some(*v),
            Projection::OpaqueCast(next) | Projection::UnwrapUnsafeBinder(next) => {
                ty = *next;
                variant = None;
            }
            Projection::Field {
                field,
                ty: field_ty,
            } => {
                let transparent = table
                    .field(ty, variant, *field)
                    .is_some_and(|f| f.key == rcgjs_object::jir::types::FieldKey::Transparent);
                if !transparent && table.transparent_field(ty).is_none() && !table.is_zst(ty) {
                    return false;
                }
                ty = *field_ty;
                variant = None;
            }
            _ => return false,
        }
    }
    true
}

/// The type of the trailing caller `Location` parameter: the result type of a `caller_location`
/// intrinsic in the body, else the one reference-to-`Location` type the table describes.
fn location_type(body: &Body, table: &TypeTable, interns: &Interns) -> Option<TypeId> {
    for block in &body.blocks {
        let Terminator::Call(call) = &block.terminator else {
            continue;
        };
        if let Callee::Intrinsic { name, .. } = &call.callee {
            if interns.str(*name) == Some("caller_location") {
                return Some(call.destination.ty);
            }
        }
    }
    let is_location = |ty: TypeId| {
        let descriptor = table.get(ty)?;
        let TypeKind::Adt {
            form: NominalForm::Struct,
            variants,
            ..
        } = &descriptor.kind
        else {
            return None;
        };
        let variant = variants.first()?;
        if interns.str(variant.name) != Some("Location") || variant.fields.len() != 3 {
            return None;
        }
        let file = table.get(variant.fields[0].ty)?;
        let TypeKind::Reference { pointee, .. } = file.kind else {
            return None;
        };
        if !matches!(table.get(pointee)?.kind, TypeKind::Str) {
            return None;
        }
        for field in &variant.fields[1..] {
            if !matches!(
                table.get(field.ty)?.kind,
                TypeKind::Integer {
                    signed: false,
                    bits: 32
                }
            ) {
                return None;
            }
        }
        Some(())
    };
    table
        .descriptors()
        .iter()
        .find(|descriptor| match descriptor.kind {
            TypeKind::Reference {
                pointee,
                mutable: false,
            } => is_location(pointee).is_some(),
            _ => false,
        })
        .map(|descriptor| descriptor.id)
}

// -------------------------------------------------------------------------------------------
// The construction pass
// -------------------------------------------------------------------------------------------

/// How a MIR local is carried.
#[derive(Clone, Copy, Debug)]
enum LocalKind {
    Value,
    Cell(CellId),
}

/// A block parameter under construction (Braun's phi).
#[derive(Clone, Debug)]
struct Param {
    block: u32,
    local: LocalId,
    /// The operand per predecessor, in the predecessor list's order; `None` until sealed.
    operands: Vec<Option<ValueId>>,
    /// Whether the parameter was found trivial and replaced by [`Builder::replaced`].
    removed: bool,
}

/// The shape of a block's terminator before its edge arguments are known.
#[derive(Clone, Debug)]
enum TermShape {
    Goto(u32),
    Invoke {
        normal: u32,
        unwind: u32,
    },
    Switch {
        scrut: ValueId,
        scrut_ty: TypeId,
        cases: Vec<(u128, u32)>,
        default: u32,
    },
    Return(Option<ValueId>),
    Unreachable,
    Throw,
}

struct Builder<'a> {
    body: &'a Body,
    origins: Option<&'a OriginTable>,
    table: &'a TypeTable,
    interns: &'a Interns,
    analysis: &'a Analysis,
    fb: FuncBuilder,
    kinds: Vec<LocalKind>,
    /// Whether `Move` ops are emitted this pass.
    moves: bool,
    /// Whether `RuntimeCheck` switches take their decided edge (see [`build_with`]).
    fold: bool,
    /// The locals whose read found no definition: demoted to cells for the next pass.
    undefined: BTreeSet<LocalId>,
    /// `current_def[local][block]`.
    current_def: Vec<Vec<Option<ValueId>>>,
    /// A private normal successor for each protected source terminator.
    normal_blocks: Vec<Option<u32>>,
    /// The invoking source block of each appended normal successor.
    normal_sources: Vec<u32>,
    /// Protected call results, assigned to their destination only in the normal successor.
    call_results: Vec<Option<ValueId>>,
    successors: Vec<Vec<BlockId>>,
    /// The reachable blocks in reverse postorder.
    order: Vec<u32>,
    reachable: Vec<bool>,
    preds: Vec<Vec<u32>>,
    sealed: Vec<bool>,
    filled: Vec<bool>,
    /// The incomplete parameters of an unsealed block.
    incomplete: Vec<Vec<ValueId>>,
    params: BTreeMap<ValueId, Param>,
    /// Trivial parameters and the value that replaces them.
    replaced: BTreeMap<ValueId, ValueId>,
    /// Parameters that read a given value as an operand (for the trivial-phi cascade).
    param_users: BTreeMap<ValueId, Vec<ValueId>>,
    /// The terminator shape of each filled block.
    terms: Vec<Option<TermShape>>,
    /// The block being filled and the origin of the statement being lowered.
    block: u32,
    origin: OriginId,
    /// Positional statement origin: the index of the first statement of each block.
    statement_base: Vec<usize>,
    /// The values that hold a whole local's definition, with the local (for `ValueDef::origin`):
    /// the first local a value defined, or the first with a user name.
    local_of_value: BTreeMap<ValueId, LocalId>,
    /// Values created by `OpKind::Uninit` (may lower to `undefined`, unsafe to overwrite in place).
    uninit_values: BTreeSet<ValueId>,
}

impl<'a> Builder<'a> {
    fn new(
        body: &'a Body,
        origins: Option<&'a OriginTable>,
        table: &'a TypeTable,
        interns: &'a Interns,
        analysis: &'a Analysis,
        cells: &BTreeSet<LocalId>,
        (moves, fold): (bool, bool),
    ) -> Result<Self, Decline> {
        let mut successors: Vec<_> = body
            .blocks
            .iter()
            .map(|block| block_successors(&block.terminator, fold))
            .collect();
        let mut normal_blocks = vec![None; body.blocks.len()];
        let mut normal_sources = Vec::new();
        for (index, block) in body.blocks.iter().enumerate() {
            if let Some(unwind) = cleanup_target(&block.terminator) {
                let normal = successors.len() as u32;
                normal_blocks[index] = Some(normal);
                normal_sources.push(index as u32);
                let normal_edges =
                    std::mem::replace(&mut successors[index], vec![BlockId(normal), unwind]);
                successors.push(normal_edges);
            }
        }
        let n = successors.len();
        let (order, reachable, preds) = cfg(&successors);
        let mut fb = FuncBuilder::new(body.item, body.signature.clone());
        // Source blocks keep their numbers. Appended normal blocks isolate call-result writes
        // from cleanup paths, even when the original normal target has other predecessors.
        for _ in 1..n {
            fb.block();
        }
        let mut kinds = Vec::with_capacity(body.locals.len());
        for local in &body.locals {
            if cells.contains(&local.id) {
                let origin = origins
                    .and_then(|origins| origins.locals.get(local.id.0 as usize).copied())
                    .unwrap_or(NO_ORIGIN);
                let boxed = analysis.boxed.contains(&local.id);
                kinds.push(LocalKind::Cell(fb.cell(local.ty, boxed, origin)));
            } else {
                kinds.push(LocalKind::Value);
            }
        }
        let mut statement_base = Vec::with_capacity(n);
        let mut base = 0;
        for block in &body.blocks {
            statement_base.push(base);
            base += block.statements.len();
        }
        Ok(Builder {
            body,
            origins,
            table,
            interns,
            analysis,
            fb,
            kinds,
            moves,
            fold,
            undefined: BTreeSet::new(),
            current_def: vec![vec![None; n]; body.locals.len()],
            normal_blocks,
            normal_sources,
            call_results: vec![None; body.blocks.len()],
            successors,
            order,
            reachable,
            preds,
            sealed: vec![false; n],
            filled: vec![false; n],
            incomplete: vec![Vec::new(); n],
            params: BTreeMap::new(),
            replaced: BTreeMap::new(),
            param_users: BTreeMap::new(),
            terms: vec![None; n],
            block: 0,
            origin: NO_ORIGIN,
            statement_base,
            local_of_value: BTreeMap::new(),
            uninit_values: BTreeSet::new(),
        })
    }

    // -- origins ------------------------------------------------------------------------------

    fn statement_origin(&self, block: u32, index: usize) -> OriginId {
        self.origins
            .and_then(|origins| {
                origins
                    .statements
                    .get(self.statement_base[block as usize] + index)
                    .copied()
            })
            .unwrap_or(NO_ORIGIN)
    }

    fn terminator_origin(&self, block: u32) -> OriginId {
        self.origins
            .and_then(|origins| origins.terminators.get(block as usize).copied())
            .unwrap_or(NO_ORIGIN)
    }

    fn local_origin(&self, local: LocalId) -> OriginId {
        self.origins
            .and_then(|origins| origins.locals.get(local.0 as usize).copied())
            .unwrap_or(NO_ORIGIN)
    }

    fn local_ty(&self, local: LocalId) -> TypeId {
        self.body.locals[local.0 as usize].ty
    }

    fn rblock(&self) -> RBlockId {
        RBlockId(self.block)
    }

    // -- values -------------------------------------------------------------------------------

    /// Pushes an op defining a value of `ty` in the current block.
    fn op(&mut self, kind: OpKind, ty: TypeId) -> ValueId {
        let is_unit = matches!(kind, OpKind::Const(ConstTree::Unit));
        let block = self.rblock();
        let v = self.fb.op(block, kind, ty, self.origin);
        if is_unit {
            self.uninit_values.insert(v);
        }
        v
    }

    fn effect(&mut self, kind: OpKind) {
        let block = self.rblock();
        self.fb.effect(block, kind, self.origin);
    }

    fn ty_of(&self, v: ValueId) -> TypeId {
        self.fb.func().values[v.0 as usize].ty
    }

    // -- Braun's algorithm --------------------------------------------------------------------

    fn write_local(&mut self, local: LocalId, block: u32, value: ValueId) {
        self.current_def[local.0 as usize][block as usize] = Some(value);
        match self.local_of_value.get(&value) {
            Some(existing) if *existing == local => {}
            Some(existing) if self.has_user_name(*existing) || !self.has_user_name(local) => {}
            _ => {
                self.local_of_value.insert(value, local);
            }
        }
    }

    fn has_user_name(&self, local: LocalId) -> bool {
        self.origins.is_some_and(|origins| {
            origins
                .locals
                .get(local.0 as usize)
                .and_then(|id| origins.origin(*id))
                .is_some_and(|origin| origin.local_name.is_some())
        })
    }

    /// The reaching definition of `local` at the end of the statements lowered so far in
    /// `block`, or `None` when no definition reaches (recorded for demotion by the caller).
    ///
    /// Braun's `readVariable`/`readVariableRecursive`, with the walk up a chain of single-
    /// predecessor blocks turned into a loop: a body can be hundreds of blocks deep, and the
    /// recursion is only needed where predecessors merge.
    fn read_variable(&mut self, local: LocalId, block: u32) -> Option<ValueId> {
        // The blocks passed on the way up, to memoize the answer on the way back down.
        let mut path: Vec<u32> = Vec::new();
        let mut current = block;
        let found: Option<ValueId> = loop {
            if let Some(value) = self.current_def[local.0 as usize][current as usize] {
                break Some(self.resolve(value));
            }
            if !self.sealed[current as usize] {
                // The block's predecessors are not all known yet: an incomplete parameter.
                let param = self.new_param(local, current);
                self.incomplete[current as usize].push(param);
                self.write_local(local, current, param);
                break Some(param);
            }
            match self.preds[current as usize].as_slice() {
                // The entry, or a block nothing reaches: no definition.
                [] => break None,
                [pred] => {
                    path.push(current);
                    current = *pred;
                }
                _ => {
                    let param = self.new_param(local, current);
                    self.write_local(local, current, param);
                    match self.add_param_operands(param) {
                        Some(value) => {
                            self.write_local(local, current, value);
                            break Some(value);
                        }
                        None => break None,
                    }
                }
            }
        };
        let value = found?;
        for block in path {
            self.write_local(local, block, value);
        }
        Some(value)
    }

    fn new_param(&mut self, local: LocalId, block: u32) -> ValueId {
        let ty = self.local_ty(local);
        let origin = self.local_origin(local);
        let param = self.fb.block_param(RBlockId(block), ty, origin);
        let count = self.preds[block as usize].len();
        self.params.insert(
            param,
            Param {
                block,
                local,
                operands: vec![None; count],
                removed: false,
            },
        );
        self.local_of_value.insert(param, local);
        param
    }

    /// Reads the parameter's local in every predecessor. `None` when one predecessor has no
    /// definition (the local gets demoted; the parameter stays as a placeholder in a pass whose
    /// result is discarded).
    fn add_param_operands(&mut self, param: ValueId) -> Option<ValueId> {
        let (local, block) = {
            let info = &self.params[&param];
            (info.local, info.block)
        };
        let preds = self.preds[block as usize].clone();
        let mut undefined = false;
        for (index, pred) in preds.iter().enumerate() {
            match self.read_variable(local, *pred) {
                Some(value) => {
                    // Reading a predecessor can recursively remove a parameter. Register
                    // this use on its surviving value so later simplifications notify us.
                    let value = self.resolve(value);
                    if let Some(info) = self.params.get_mut(&param) {
                        info.operands[index] = Some(value);
                    }
                    self.param_users.entry(value).or_default().push(param);
                }
                None => {
                    undefined = true;
                    self.undefined.insert(local);
                }
            }
        }
        if undefined {
            return None;
        }
        Some(self.try_remove_trivial(param))
    }

    /// Braun's `tryRemoveTrivialPhi`: a parameter whose operands are all itself or one other
    /// value is that value.
    fn try_remove_trivial(&mut self, param: ValueId) -> ValueId {
        let operands: Vec<ValueId> = self.params[&param]
            .operands
            .iter()
            .flatten()
            .map(|v| self.resolve(*v))
            .collect();
        let mut same: Option<ValueId> = None;
        for operand in operands {
            if Some(operand) == same || operand == param {
                continue;
            }
            if same.is_some() {
                // Two distinct operands: not trivial.
                return param;
            }
            same = Some(operand);
        }
        let Some(same) = same else {
            // Only itself: unreachable in a well-formed body; keep the parameter.
            return param;
        };
        if let Some(info) = self.params.get_mut(&param) {
            info.removed = true;
        }
        self.replaced.insert(param, same);
        // Every parameter that used this one may have become trivial.
        let users = self.param_users.remove(&param).unwrap_or_default();
        for user in users {
            if user == param {
                continue;
            }
            if self.params.get(&user).is_some_and(|info| !info.removed) {
                self.param_users.entry(same).or_default().push(user);
                self.try_remove_trivial(user);
            }
        }
        same
    }

    /// Follows the replacement chain of a removed parameter.
    fn resolve(&self, mut value: ValueId) -> ValueId {
        let mut steps = 0;
        while let Some(next) = self.replaced.get(&value) {
            value = *next;
            steps += 1;
            if steps > self.replaced.len() {
                break;
            }
        }
        value
    }

    fn seal(&mut self, block: u32) {
        if self.sealed[block as usize] {
            return;
        }
        let incomplete = std::mem::take(&mut self.incomplete[block as usize]);
        for param in incomplete {
            let _ = self.add_param_operands(param);
        }
        self.sealed[block as usize] = true;
    }

    // -- the pass -----------------------------------------------------------------------------

    fn run(&mut self) -> Result<(), Decline> {
        self.entry_prelude()?;
        // The entry has no predecessors: sealed from the start.
        self.seal(0);
        let order = self.order.clone();
        for block in order {
            self.fill(block)?;
            self.filled[block as usize] = true;
            // Seal every successor whose predecessors are now all filled.
            let successors = self.successors[block as usize].clone();
            for successor in successors {
                let successor = successor.0;
                if !self.reachable[successor as usize] || self.sealed[successor as usize] {
                    continue;
                }
                if self.preds[successor as usize]
                    .iter()
                    .all(|pred| self.filled[*pred as usize])
                {
                    self.seal(successor);
                }
            }
        }
        // Whatever is left unsealed (a loop whose back edge comes from an unreachable block, or
        // an ordering the successor scan missed) is sealed now that everything is filled.
        for block in 0..self.successors.len() as u32 {
            if self.reachable[block as usize] {
                self.seal(block);
            }
        }
        Ok(())
    }

    /// Parameters, cells, and the spread tuple: what the entry block runs before its statements.
    fn entry_prelude(&mut self) -> Result<(), Decline> {
        self.block = 0;
        self.origin = self
            .origins
            .and_then(|origins| origins.terminators.first().copied())
            .unwrap_or(NO_ORIGIN);
        let spread = self.body.signature.abi.spread;
        let mut param_values: Vec<(LocalId, ValueId)> = Vec::new();
        for local in &self.body.locals {
            if local.role != LocalRole::Argument {
                continue;
            }
            let origin = self.local_origin(local.id);
            if let Some(spread) = spread.filter(|spread| spread.local == local.id) {
                let TypeKind::Tuple(elements) = self
                    .table
                    .get(local.ty)
                    .map(|descriptor| descriptor.kind.clone())
                    .unwrap_or(TypeKind::Opaque)
                else {
                    return Err(decline(
                        OpClass::Call,
                        "rust-call spread argument is not a tuple",
                    ));
                };
                if elements.len() != usize::from(spread.arity) {
                    return Err(decline(
                        OpClass::Call,
                        "rust-call spread arity differs from the tuple",
                    ));
                }
                let fields: Vec<ValueId> = elements
                    .iter()
                    .map(|element| self.fb.param(*element, origin))
                    .collect();
                let tuple = self.op(
                    OpKind::Aggregate {
                        ty: local.ty,
                        variant: 0,
                        active_field: None,
                        fields,
                    },
                    local.ty,
                );
                param_values.push((local.id, tuple));
            } else {
                let value = self.fb.param(local.ty, origin);
                param_values.push((local.id, value));
            }
        }
        if self.body.signature.abi.caller_location {
            // The `Location` type is in the table only when some call site in the object spelled
            // a location; a body that merely forwards its own has nothing to name the parameter's
            // type with, and the lowering never reads it (the parameter is `$loc` by position),
            // so the signature's result type stands in.
            let location_ty = self
                .analysis
                .location_ty
                .unwrap_or(self.body.signature.result);
            self.fb.param(location_ty, NO_ORIGIN);
        }
        // Every cell exists from the entry on, so every use is dominated by its allocation.
        let cells: Vec<(LocalId, CellId)> = self
            .kinds
            .iter()
            .enumerate()
            .filter_map(|(index, kind)| match kind {
                LocalKind::Cell(cell) => Some((LocalId(index as u32), *cell)),
                LocalKind::Value => None,
            })
            .collect();
        for (local, cell) in &cells {
            self.origin = self.local_origin(*local);
            self.effect(OpKind::CellAlloc(*cell));
        }
        for (local, value) in param_values {
            match self.kinds[local.0 as usize] {
                LocalKind::Value => self.write_local(local, 0, value),
                LocalKind::Cell(cell) => {
                    self.origin = self.local_origin(local);
                    self.effect(OpKind::CellStore { cell, v: value });
                }
            }
        }
        Ok(())
    }

    fn fill(&mut self, block: u32) -> Result<(), Decline> {
        self.block = block;
        if block as usize >= self.body.blocks.len() {
            return self.fill_normal(block);
        }
        let data = &self.body.blocks[block as usize];
        for (index, statement) in data.statements.iter().enumerate() {
            self.origin = self.statement_origin(block, index);
            self.statement(&statement.kind)?;
            if let Some(locals) = self.analysis.assembly.after_statement.get(&(block, index)) {
                self.fresh_storage(locals);
            }
        }
        self.origin = self.terminator_origin(block);
        let shape = self.terminator(&data.terminator)?;
        self.terms[block as usize] = Some(shape);
        Ok(())
    }

    fn fill_normal(&mut self, block: u32) -> Result<(), Decline> {
        let source = self.normal_sources[block as usize - self.body.blocks.len()];
        self.origin = self.terminator_origin(source);
        let terminator = &self.body.blocks[source as usize].terminator;
        let target = match terminator {
            Terminator::Call(call) => {
                if let Some(value) = self.call_results[source as usize] {
                    self.write_place(&call.destination, value)?;
                } else if call.target.is_some()
                    && (!self.table.is_zst(call.destination.ty)
                        || call.destination.projection.is_empty())
                {
                    let unit = self.op(OpKind::Const(ConstTree::Unit), call.destination.ty);
                    self.write_place(&call.destination, unit)?;
                }
                if let Some(locals) = self.analysis.assembly.after_call.get(&source) {
                    self.fresh_storage(locals);
                }
                call.target
            }
            Terminator::Drop { target, .. } => Some(*target),
            _ => unreachable!("a private normal block belongs to a call or drop"),
        };
        self.terms[block as usize] = Some(match target {
            Some(target) => TermShape::Goto(target.0),
            None => TermShape::Unreachable,
        });
        Ok(())
    }

    /// Gives each local assembled in place fresh storage where `assembly.rs` found its storage
    /// starting over with a field write to follow: its `StorageLive`, or the statement or call that
    /// moved it out. The emitter spells the alloc as the local's empty container.
    fn fresh_storage(&mut self, locals: &[LocalId]) {
        let origin = self.origin;
        for local in locals {
            if let LocalKind::Cell(cell) = self.kinds[local.0 as usize] {
                self.origin = self.local_origin(*local);
                self.effect(OpKind::CellAlloc(cell));
            }
        }
        self.origin = origin;
    }

    // -- reading and writing locals -----------------------------------------------------------

    /// The value of a bare local at this point: the reaching definition, or a load of its cell.
    fn read_local(&mut self, local: LocalId) -> ValueId {
        match self.kinds[local.0 as usize] {
            LocalKind::Cell(cell) => {
                let ty = self.local_ty(local);
                self.op(OpKind::CellLoad(cell), ty)
            }
            LocalKind::Value => match self.read_variable(local, self.block) {
                Some(value) => value,
                None => {
                    self.undefined.insert(local);
                    // A placeholder so the pass can go on; its result is discarded.
                    let ty = self.local_ty(local);
                    let origin = self.local_origin(local);
                    self.fb.value(ty, origin)
                }
            },
        }
    }

    fn write_bare_local(&mut self, local: LocalId, value: ValueId) {
        match self.kinds[local.0 as usize] {
            LocalKind::Cell(cell) => self.effect(OpKind::CellStore { cell, v: value }),
            LocalKind::Value => {
                if self.analysis.identity_preserved_locals.contains(&local)
                    && !self.uninit_values.contains(&value)
                {
                    let prev = if self.sealed[self.block as usize] {
                        self.read_variable(local, self.block)
                    } else {
                        self.current_def[local.0 as usize][self.block as usize]
                    };
                    if let Some(prev) = prev {
                        if !self.uninit_values.contains(&prev) {
                            let ty = self.local_ty(local);
                            if let Ok(view_ty) = self.view_ty(ty, true) {
                                let view = self.op(
                                    OpKind::BorrowView {
                                        root: Root::Value(prev),
                                        path: Vec::new(),
                                        mutable: true,
                                    },
                                    view_ty,
                                );
                                self.effect(OpKind::StoreThrough {
                                    ptr: view,
                                    v: value,
                                });
                                return;
                            }
                        }
                    }
                }
                self.write_local(local, self.block, value);
            }
        }
    }

    // -- places -------------------------------------------------------------------------------

    /// Whether a value of `ty` can be read whole (its layout has a size).
    fn is_sized(&self, ty: TypeId) -> bool {
        self.table
            .get(ty)
            .is_some_and(|descriptor| descriptor.layout.size.is_some())
    }

    /// The type of a synthetic view of a place of type `ty`: a reference type from the table, or
    /// the place's own type when its peeled kind is not a pointer.
    fn view_ty(&self, ty: TypeId, mutable: bool) -> Result<TypeId, Decline> {
        let references = self.analysis.references(self.table);
        if let Some(reference) = references
            .get(&(ty, mutable))
            .or_else(|| references.get(&(ty, !mutable)))
        {
            return Ok(*reference);
        }
        if placeholder_view_ok(self.table, ty) {
            return Ok(ty);
        }
        // A pointer-typed place with no reference type in the table: the view is consumed by the
        // op right after it and its type is never inspected (`plan.rs`, "Synthetic views"), so
        // any non-pointer type of the table stands in. Deterministic: the first that qualifies.
        let stand_in = std::iter::once(self.body.signature.result)
            .chain(self.body.signature.arguments.iter().copied())
            .chain(self.table.ids())
            .find(|candidate| placeholder_view_ok(self.table, *candidate));
        stand_in.ok_or_else(|| {
            decline(
                OpClass::Place,
                "element access of a pointer-typed place with no non-pointer type to stand in",
            )
        })
    }

    fn proj_of(&mut self, projection: &Projection) -> Option<Proj> {
        Some(match projection {
            Projection::Deref => Proj::Deref,
            Projection::Field { field, ty } => Proj::Field {
                field: *field,
                ty: *ty,
            },
            Projection::Index(local) => Proj::Index(self.read_local(*local)),
            Projection::ConstantIndex {
                offset,
                min_length,
                from_end,
            } => Proj::ConstantIndex {
                offset: *offset,
                min_length: *min_length,
                from_end: *from_end,
            },
            Projection::Subslice { from, to, from_end } => Proj::Subslice {
                from: *from,
                to: *to,
                from_end: *from_end,
            },
            Projection::Downcast { variant } => Proj::Downcast { variant: *variant },
            Projection::OpaqueCast(_) | Projection::UnwrapUnsafeBinder(_) => return None,
        })
    }

    /// The root of a place: its local as a value or a cell.
    fn root_of(&mut self, local: LocalId) -> Root {
        match self.kinds[local.0 as usize] {
            LocalKind::Cell(cell) => Root::Cell(cell),
            LocalKind::Value => Root::Value(self.read_local(local)),
        }
    }

    /// The whole place path as JIR-R projections, for a borrow or a drop.
    fn full_path(&mut self, place: &Place) -> Vec<Proj> {
        let mut path = Vec::with_capacity(place.projection.len());
        for projection in &place.projection {
            if let Some(proj) = self.proj_of(projection) {
                path.push(proj);
            }
        }
        path
    }

    /// Reads a place as a value (no clone: the caller adds `Copy` for a copied operand).
    fn read_place(&mut self, place: &Place) -> Result<ValueId, Decline> {
        if place.projection.is_empty() {
            return Ok(self.read_local(place.local));
        }
        // The value reached so far, its type, and the downcast pending on it.
        let mut cur: Option<ValueId> = None;
        let mut cur_ty = self.local_ty(place.local);
        let mut variant: Option<u32> = None;
        // The root and path of a view under construction, once a step needs one.
        let root: Root;
        let mut pending: Vec<Proj> = Vec::new();
        match self.kinds[place.local.0 as usize] {
            LocalKind::Cell(cell) => root = Root::Cell(cell),
            LocalKind::Value => {
                let value = self.read_local(place.local);
                cur = Some(value);
                root = Root::Value(value);
            }
        }
        // A cell whose whole value is asked for through field projections is loaded first.
        let mut cell_root = matches!(root, Root::Cell(_));
        for (index, projection) in place.projection.iter().enumerate() {
            let simple = pending.is_empty();
            match projection {
                Projection::Field { field, ty } if simple => {
                    if cell_root {
                        let Root::Cell(cell) = root else {
                            unreachable!("cell root")
                        };
                        cur = Some(self.op(OpKind::CellLoad(cell), cur_ty));
                        cell_root = false;
                    }
                    let base = cur.expect("a value or a loaded cell");
                    cur = Some(self.op(
                        OpKind::Field {
                            base,
                            variant,
                            field: *field,
                        },
                        *ty,
                    ));
                    cur_ty = *ty;
                    variant = None;
                }
                Projection::Downcast { variant: v } if simple => variant = Some(*v),
                Projection::OpaqueCast(ty) | Projection::UnwrapUnsafeBinder(ty) => {
                    cur_ty = *ty;
                }
                Projection::Deref if simple && !cell_root => {
                    let Some(pointee) = self.table.pointee(cur_ty) else {
                        return Err(decline(OpClass::Place, "deref of a non-pointer type"));
                    };
                    let next_is_element = matches!(
                        place.projection.get(index + 1),
                        Some(
                            Projection::Index(_)
                                | Projection::ConstantIndex { .. }
                                | Projection::Subslice { .. }
                        )
                    );
                    if self.is_sized(pointee) && !next_is_element {
                        let base = cur.expect("a pointer value");
                        cur = Some(self.op(OpKind::Deref(base), pointee));
                        cur_ty = pointee;
                        variant = None;
                    } else {
                        pending.push(Proj::Deref);
                    }
                }
                other => {
                    // The view starts here: a pending downcast joins the path.
                    if pending.is_empty() {
                        if let Some(v) = variant.take() {
                            pending.push(Proj::Downcast { variant: v });
                        }
                    }
                    if let Some(proj) = self.proj_of(other) {
                        pending.push(proj);
                    }
                }
            }
        }
        if pending.is_empty() {
            return Ok(match cur {
                Some(value) => value,
                None => {
                    let Root::Cell(cell) = root else {
                        unreachable!("cell root")
                    };
                    self.op(OpKind::CellLoad(cell), cur_ty)
                }
            });
        }
        // The view's root is the value the simple prefix reached, or the cell.
        let view_root = match cur {
            Some(value) if !cell_root => Root::Value(value),
            _ => root,
        };
        let view_ty = self.view_ty(place.ty, false)?;
        let view = self.op(
            OpKind::BorrowView {
                root: view_root,
                path: pending,
                mutable: false,
            },
            view_ty,
        );
        Ok(self.op(OpKind::Deref(view), place.ty))
    }

    /// Writes `value` to a place.
    fn write_place(&mut self, place: &Place, value: ValueId) -> Result<(), Decline> {
        if place.projection.is_empty() {
            self.write_bare_local(place.local, value);
            return Ok(());
        }
        // A single field write on a value: `SetField` on the reaching value's storage.
        let value_root = matches!(self.kinds[place.local.0 as usize], LocalKind::Value);
        // That storage is the local's own only when its copies are values of their own. A copy of
        // a type that does not `needs_clone` is the source value itself (`operand`), so after
        // `let mut b = a` both locals reach one value, and writing `b`'s transparent field in
        // place would rebind `a` too. Such a local is given a value of its own first; a write
        // through a `Deref` lands in the pointee, not here.
        let local_ty = self.local_ty(place.local);
        if value_root
            && !self.table.needs_clone(local_ty)
            && !place
                .projection
                .iter()
                .any(|projection| matches!(projection, Projection::Deref))
        {
            let shared = self.read_local(place.local);
            let own = self.op(OpKind::Copy(shared), local_ty);
            self.write_bare_local(place.local, own);
        }
        let (downcast, field) = single_field(&place.projection);
        if let (true, Some(field)) = (value_root, field) {
            let preserve_identity = self.table.is_indirect(place.ty)
                && self.analysis.identity_preserved_fields.contains(&(
                    place.local,
                    downcast,
                    field,
                ));
            if !preserve_identity {
                let base = self.read_local(place.local);
                self.effect(OpKind::SetField {
                    base,
                    variant: downcast,
                    field,
                    v: value,
                });
                return Ok(());
            }
            if downcast.is_none()
                && !self.table.crosses_transparent_union(place.ty)
                && matches!(
                    self.table.get(self.table.peeled(local_ty)).map(|d| &d.kind),
                    Some(
                        TypeKind::Adt {
                            form: NominalForm::Struct,
                            ..
                        } | TypeKind::Tuple(_)
                    )
                )
            {
                let base = self.read_local(place.local);
                let cur = self.op(
                    OpKind::Field {
                        base,
                        variant: None,
                        field,
                    },
                    place.ty,
                );
                if let Ok(view_ty) = self.view_ty(place.ty, true) {
                    let view = self.op(
                        OpKind::BorrowView {
                            root: Root::Value(cur),
                            path: Vec::new(),
                            mutable: true,
                        },
                        view_ty,
                    );
                    self.effect(OpKind::StoreThrough {
                        ptr: view,
                        v: value,
                    });
                    return Ok(());
                }
            }
        }
        let root = self.root_of(place.local);
        let path = self.full_path(place);
        let view_ty = match self.view_ty(place.ty, true) {
            Ok(ty) => ty,
            Err(err) => {
                // Field-only paths have the load-modify-store form as a fallback.
                if value_root
                    && path
                        .iter()
                        .all(|proj| matches!(proj, Proj::Field { .. } | Proj::Downcast { .. }))
                {
                    return self.write_field_chain(place, value);
                }
                return Err(err);
            }
        };
        let view = self.op(
            OpKind::BorrowView {
                root,
                path,
                mutable: true,
            },
            view_ty,
        );
        self.effect(OpKind::StoreThrough {
            ptr: view,
            v: value,
        });
        Ok(())
    }

    /// `_3.a.b = v` as `t = Field(_3, a); SetField(t, b, v); SetField(_3, a, t)`: exact under
    /// value semantics without a pointer type.
    fn write_field_chain(&mut self, place: &Place, value: ValueId) -> Result<(), Decline> {
        let root = self.read_local(place.local);
        let mut chain: Vec<(ValueId, Option<u32>, u32)> = Vec::new();
        let mut cur = root;
        let mut variant = None;
        let mut steps: Vec<(Option<u32>, u32, TypeId)> = Vec::new();
        for projection in &place.projection {
            match projection {
                Projection::Downcast { variant: v } => variant = Some(*v),
                Projection::Field { field, ty } => {
                    steps.push((variant.take(), *field, *ty));
                }
                _ => return Err(decline(OpClass::Place, "field chain with a non-field step")),
            }
        }
        for (index, (variant, field, ty)) in steps.iter().enumerate() {
            if index + 1 == steps.len() {
                break;
            }
            let next = self.op(
                OpKind::Field {
                    base: cur,
                    variant: *variant,
                    field: *field,
                },
                *ty,
            );
            chain.push((cur, *variant, *field));
            cur = next;
        }
        let Some((variant, field, _)) = steps.last().copied() else {
            return Err(decline(OpClass::Place, "empty field chain"));
        };
        self.effect(OpKind::SetField {
            base: cur,
            variant,
            field,
            v: value,
        });
        // Write back up the chain.
        let mut written = cur;
        for (base, variant, field) in chain.into_iter().rev() {
            self.effect(OpKind::SetField {
                base,
                variant,
                field,
                v: written,
            });
            written = base;
        }
        Ok(())
    }

    // -- operands and rvalues -----------------------------------------------------------------

    fn constant(&mut self, constant: &Constant, ty: TypeId) -> ValueId {
        let tree = match constant {
            Constant::ZeroSized => ConstTree::Unit,
            Constant::Scalar { bits, bytes } => ConstTree::Scalar {
                bits: *bits,
                bytes: *bytes,
            },
            Constant::String(id) => ConstTree::Str(*id),
            Constant::Pointer { target, offset } => ConstTree::Global {
                target: *target,
                offset: *offset,
            },
            Constant::Memory(tree) => tree.clone(),
        };
        self.op(OpKind::Const(tree), ty)
    }

    fn operand(&mut self, operand: &Operand) -> Result<ValueId, Decline> {
        match &operand.kind {
            OperandKind::Constant(constant) => Ok(self.constant(constant, operand.ty)),
            OperandKind::RuntimeCheck(value) => Ok(self.op(
                OpKind::Const(ConstTree::Scalar {
                    bits: u128::from(*value),
                    bytes: 1,
                }),
                operand.ty,
            )),
            OperandKind::Copy(place) => {
                let value = self.read_place(place)?;
                if self.table.needs_clone(operand.ty) {
                    Ok(self.op(OpKind::Copy(value), operand.ty))
                } else {
                    Ok(value)
                }
            }
            OperandKind::Move(place) => {
                let value = self.read_place(place)?;
                let preserved = if place.projection.is_empty() {
                    self.analysis
                        .identity_preserved_locals
                        .contains(&place.local)
                } else {
                    let (downcast, field) = single_field(&place.projection);
                    field.is_some_and(|field| {
                        self.analysis.identity_preserved_fields.contains(&(
                            place.local,
                            downcast,
                            field,
                        ))
                    })
                };
                if preserved && self.table.needs_clone(operand.ty) {
                    return Ok(self.op(OpKind::Copy(value), operand.ty));
                }
                let bare_value = place.projection.is_empty()
                    && matches!(self.kinds[place.local.0 as usize], LocalKind::Value);
                let movable = self.moves
                    && bare_value
                    && self.table.needs_clone(operand.ty)
                    && !self.analysis.uses[place.local.0 as usize].dropped;
                if movable {
                    Ok(self.op(OpKind::Move(value), operand.ty))
                } else {
                    Ok(value)
                }
            }
        }
    }

    fn rvalue(&mut self, rvalue: &Rvalue, result_ty: TypeId) -> Result<ValueId, Decline> {
        Ok(match rvalue {
            Rvalue::Use(operand) | Rvalue::WrapUnsafeBinder(operand) => self.operand(operand)?,
            Rvalue::CopyForDeref(place) => self.read_place(place)?,
            Rvalue::Repeat { value, count } => {
                let v = self.operand(value)?;
                self.op(OpKind::Repeat { v, count: *count }, result_ty)
            }
            Rvalue::Reference { kind, place } => {
                let mutable = matches!(
                    kind,
                    BorrowKind::Mutable | BorrowKind::TwoPhase | BorrowKind::ClosureCapture
                );
                let root = self.root_of(place.local);
                let path = self.full_path(place);
                self.op(
                    OpKind::BorrowView {
                        root,
                        path,
                        mutable,
                    },
                    result_ty,
                )
            }
            Rvalue::Reborrow { mutable, place } => {
                let root = self.root_of(place.local);
                let path = self.full_path(place);
                self.op(
                    OpKind::BorrowView {
                        root,
                        path,
                        mutable: *mutable,
                    },
                    result_ty,
                )
            }
            Rvalue::RawPointer { kind, place } => {
                let mutable = matches!(kind, crate::jir::semantic::RawPointerKind::Mutable);
                let root = self.root_of(place.local);
                let path = self.full_path(place);
                self.op(
                    OpKind::RawView {
                        root,
                        path,
                        mutable,
                    },
                    result_ty,
                )
            }
            Rvalue::Cast {
                kind,
                value,
                result,
            } => {
                let v = self.operand(value)?;
                self.op(
                    OpKind::Cast {
                        kind: *kind,
                        v,
                        from: value.ty,
                        to: *result,
                    },
                    result_ty,
                )
            }
            Rvalue::Binary { op, left, right } => {
                let l = self.operand(left)?;
                let r = self.operand(right)?;
                self.op(
                    OpKind::Binary {
                        op: *op,
                        l,
                        r,
                        ty: left.ty,
                    },
                    result_ty,
                )
            }
            Rvalue::Unary { op, value } => {
                let v = self.operand(value)?;
                self.op(
                    OpKind::Unary {
                        op: *op,
                        v,
                        ty: value.ty,
                    },
                    result_ty,
                )
            }
            Rvalue::Discriminant(place) => {
                let base = self.read_place(place)?;
                self.op(OpKind::Discriminant(base), result_ty)
            }
            Rvalue::Aggregate {
                kind,
                result,
                operands,
            } => {
                let mut fields = Vec::with_capacity(operands.len());
                for operand in operands {
                    fields.push(self.operand(operand)?);
                }
                match kind {
                    AggregateKind::RawPointer { pointee, .. } => {
                        return self.pointer_from_parts(*pointee, fields, *result);
                    }
                    AggregateKind::Adt {
                        variant,
                        active_field,
                        ..
                    } => self.op(
                        OpKind::Aggregate {
                            ty: *result,
                            variant: *variant,
                            active_field: *active_field,
                            fields,
                        },
                        result_ty,
                    ),
                    AggregateKind::Array(_)
                    | AggregateKind::Tuple
                    | AggregateKind::Closure(_)
                    | AggregateKind::Coroutine(_)
                    | AggregateKind::CoroutineClosure(_) => self.op(
                        OpKind::Aggregate {
                            ty: *result,
                            variant: 0,
                            active_field: None,
                            fields,
                        },
                        result_ty,
                    ),
                }
            }
        })
    }

    /// `AggregateKind::RawPointer`: a pointer built from its data half and its metadata.
    fn pointer_from_parts(
        &mut self,
        pointee: TypeId,
        fields: Vec<ValueId>,
        result: TypeId,
    ) -> Result<ValueId, Decline> {
        let [data, meta] = fields.as_slice() else {
            return Err(decline(
                OpClass::Rvalue,
                "raw pointer aggregate without exactly two operands",
            ));
        };
        let kind = self
            .table
            .get(self.table.peeled(pointee))
            .map(|descriptor| descriptor.kind.clone());
        match kind {
            Some(TypeKind::Slice(_) | TypeKind::Str) => Ok(self.op(
                OpKind::SliceView {
                    ptr: *data,
                    len: *meta,
                },
                result,
            )),
            Some(TypeKind::Dynamic { .. }) => Err(decline(
                OpClass::Unsize,
                "raw `dyn` pointer built from parts",
            )),
            _ if self.is_sized(pointee) => {
                let from = self.ty_of(*data);
                Ok(self.op(
                    OpKind::Cast {
                        kind: CastKind::PtrToPtr,
                        v: *data,
                        from,
                        to: result,
                    },
                    result,
                ))
            }
            _ => Err(decline(
                OpClass::Unsize,
                "raw pointer to an unsized-tail struct built from parts",
            )),
        }
    }

    // -- statements ---------------------------------------------------------------------------

    fn statement(&mut self, statement: &StatementKind) -> Result<(), Decline> {
        match statement {
            StatementKind::Assign { destination, value } => {
                let v = self.rvalue(value, destination.ty)?;
                self.write_place(destination, v)
            }
            StatementKind::SetDiscriminant { place, variant } => {
                self.set_discriminant(place, *variant)
            }
            StatementKind::StorageLive(_)
            | StatementKind::StorageDead(_)
            | StatementKind::Nop
            | StatementKind::Assume(_) => Ok(()),
            StatementKind::CopyNonOverlapping {
                source,
                destination,
                count,
            } => {
                let Some(name) = self.analysis.copy_nonoverlapping(self.interns) else {
                    return Err(decline(
                        OpClass::Statement,
                        "copy_nonoverlapping statement without an interned intrinsic name",
                    ));
                };
                let Some(pointee) = self.table.pointee(source.ty) else {
                    return Err(decline(
                        OpClass::Statement,
                        "copy_nonoverlapping of a non-pointer",
                    ));
                };
                let src = self.operand(source)?;
                let dst = self.operand(destination)?;
                let count = self.operand(count)?;
                self.effect(OpKind::Intrinsic {
                    name,
                    types: vec![pointee],
                    fallback: None,
                    args: vec![src, dst, count],
                    track_caller: None,
                });
                Ok(())
            }
        }
    }

    fn set_discriminant(&mut self, place: &Place, variant: u32) -> Result<(), Decline> {
        let Some(repr) = self.table.enum_repr(place.ty) else {
            return Err(decline(
                OpClass::Statement,
                "SetDiscriminant of a place that is not an enum",
            ));
        };
        if let Some((none_variant, some_variant, _)) = repr.niche_variants() {
            if variant == some_variant {
                // Writing `((opt as Some).<some_field>) = payload` already wrote the whole place.
                return Ok(());
            }
            if variant == none_variant {
                let none_fields = self
                    .table
                    .variants(self.table.peeled(place.ty))
                    .get(none_variant as usize)
                    .map_or(0, |v| v.fields.len());
                let tag = self.op(
                    OpKind::Const(ConstTree::Variant {
                        variant: none_variant,
                        fields: vec![ConstTree::Unit; none_fields],
                    }),
                    place.ty,
                );
                return self.write_place(place, tag);
            }
        }
        let object_form = matches!(
            repr,
            EnumRepr::Tag { fieldless: false }
                | EnumRepr::IntTag { fieldless: false }
                | EnumRepr::State
        ) && self.table.is_indirect(place.ty);
        if !object_form {
            // A primitive enum is its tag: the whole place is written.
            let tag = self.op(
                OpKind::Const(ConstTree::Variant {
                    variant,
                    fields: Vec::new(),
                }),
                place.ty,
            );
            return self.write_place(place, tag);
        }
        if place.projection.is_empty() {
            match self.kinds[place.local.0 as usize] {
                LocalKind::Value => {
                    let base = self.read_local(place.local);
                    self.effect(OpKind::SetDiscriminant { base, variant });
                }
                LocalKind::Cell(cell) => {
                    let ty = self.local_ty(place.local);
                    // A variant with no fields has nothing in the storage to keep, so the write
                    // is the whole value. Loading the cell instead would read storage that may
                    // not hold a value yet: MIR can give a local its value by `SetDiscriminant`
                    // alone (`assembly.rs`), and a load of an uninitialized cell is undefined.
                    let fieldless = self
                        .table
                        .variants(self.table.peeled(ty))
                        .get(variant as usize)
                        .is_some_and(|descriptor| descriptor.fields.is_empty());
                    if fieldless && self.table.peeled(ty) == self.table.peeled(place.ty) {
                        let whole = self.op(
                            OpKind::Aggregate {
                                ty: place.ty,
                                variant,
                                active_field: None,
                                fields: Vec::new(),
                            },
                            place.ty,
                        );
                        self.effect(OpKind::CellStore { cell, v: whole });
                        return Ok(());
                    }
                    // Load, set, store back: exact under value semantics; the store is `x = x`.
                    let base = self.op(OpKind::CellLoad(cell), ty);
                    self.effect(OpKind::SetDiscriminant { base, variant });
                    self.effect(OpKind::CellStore { cell, v: base });
                }
            }
            return Ok(());
        }
        // Through a projection: the store-back idiom on a view.
        let root = self.root_of(place.local);
        let path = self.full_path(place);
        let view_ty = self.view_ty(place.ty, true)?;
        let view = self.op(
            OpKind::BorrowView {
                root,
                path,
                mutable: true,
            },
            view_ty,
        );
        // Under the baseline carrier a `Deref` of a pointer to an object-form enum is the object
        // itself, so the discriminant write lands on the pointee and a store-back would store the
        // object over itself. (The JIR-R interpreter copies on `Deref` and misses the write; the
        // dual-interpreter harness skips this shape.)
        let base = self.op(OpKind::Deref(view), place.ty);
        self.effect(OpKind::SetDiscriminant { base, variant });
        Ok(())
    }

    // -- terminators --------------------------------------------------------------------------

    fn terminator(&mut self, terminator: &Terminator) -> Result<TermShape, Decline> {
        Ok(match terminator {
            Terminator::Goto(target) => TermShape::Goto(target.0),
            Terminator::FalseEdge { real, .. } | Terminator::FalseUnwind { real, .. } => {
                TermShape::Goto(real.0)
            }
            Terminator::Switch { .. } if self.fold && folded_switch(terminator).is_some() => {
                TermShape::Goto(folded_switch(terminator).expect("checked").0)
            }
            Terminator::Switch {
                discriminant,
                targets,
                otherwise,
            } => {
                let scrut = self.operand(discriminant)?;
                TermShape::Switch {
                    scrut,
                    scrut_ty: discriminant.ty,
                    cases: targets
                        .iter()
                        .map(|(value, target)| (*value, target.0))
                        .collect(),
                    default: otherwise.0,
                }
            }
            Terminator::Return => {
                let ret = LocalId(0);
                // A zero-sized result returns nothing, assigned or not (v1 returns nothing for
                // `()`; the interpreters agree on the type's one value).
                if self.table.is_zst(self.body.signature.result) {
                    return Ok(TermShape::Return(None));
                }
                match self.kinds[0] {
                    LocalKind::Cell(cell) => {
                        let ty = self.local_ty(ret);
                        let value = self.op(OpKind::CellLoad(cell), ty);
                        TermShape::Return(Some(value))
                    }
                    LocalKind::Value => match self.read_variable(ret, self.block) {
                        Some(value) => TermShape::Return(Some(value)),
                        None => {
                            self.undefined.insert(ret);
                            TermShape::Return(None)
                        }
                    },
                }
            }
            Terminator::Unreachable => TermShape::Unreachable,
            Terminator::UnwindResume => TermShape::Throw,
            // An unwind leaving a function that cannot unwind: under `-Cpanic=unwind` rustc ends
            // the cleanup path of an `extern "C" fn` this way where a Rust one ends in
            // `UnwindResume`. A Rust panic is already an abort that every handler rethrows without
            // running cleanup (`__rt.is_abort`), so what reaches this block is a host exception,
            // and the contract is that it leaves the function with its identity once the drops
            // have run: the same rethrow of the caught exception as `UnwindResume`.
            Terminator::UnwindTerminate(TerminateReason::Abi) => TermShape::Throw,
            Terminator::UnwindTerminate(TerminateReason::InCleanup)
                if !self.normal_sources.is_empty() =>
            {
                return Err(decline(
                    OpClass::Terminator,
                    "exception termination requires a separate abort boundary",
                ));
            }
            Terminator::UnwindTerminate(TerminateReason::InCleanup) => TermShape::Throw,
            Terminator::Call(call) => self.call(call)?,
            Terminator::Drop {
                place,
                target,
                async_drop,
                glue,
                unwind,
                ..
            } => {
                if async_drop.is_some() {
                    return Err(decline(OpClass::Drop, "async drop"));
                }
                if !matches!(glue, crate::jir::semantic::DropGlue::None) {
                    let root = self.root_of(place.local);
                    let path = self.full_path(place);
                    self.effect(OpKind::Drop {
                        root,
                        path,
                        glue: *glue,
                    });
                }
                match self.normal_blocks[self.block as usize] {
                    Some(normal) => {
                        let Unwind::Cleanup(unwind) = unwind else {
                            unreachable!()
                        };
                        TermShape::Invoke {
                            normal,
                            unwind: unwind.0,
                        }
                    }
                    None => TermShape::Goto(target.0),
                }
            }
            Terminator::Assert {
                condition,
                expected,
                message,
                target,
                track_caller,
                ..
            } => {
                let cond = self.operand(condition)?;
                let msg = self.assert_message(message)?;
                self.effect(OpKind::Assert {
                    cond,
                    expected: *expected,
                    msg,
                    track_caller: *track_caller,
                });
                TermShape::Goto(target.0)
            }
        })
    }

    fn assert_message(&mut self, message: &AssertMessage) -> Result<RAssertMessage, Decline> {
        Ok(match message {
            AssertMessage::BoundsCheck { length, index } => RAssertMessage::BoundsCheck {
                length: self.operand(length)?,
                index: self.operand(index)?,
            },
            AssertMessage::Overflow { op, left, right } => RAssertMessage::Overflow {
                op: *op,
                left: self.operand(left)?,
                right: self.operand(right)?,
            },
            AssertMessage::OverflowNeg(v) => RAssertMessage::OverflowNeg(self.operand(v)?),
            AssertMessage::DivisionByZero(v) => RAssertMessage::DivisionByZero(self.operand(v)?),
            AssertMessage::RemainderByZero(v) => RAssertMessage::RemainderByZero(self.operand(v)?),
            AssertMessage::ResumedAfterReturn(kind) => RAssertMessage::ResumedAfterReturn(*kind),
            AssertMessage::ResumedAfterPanic(kind) => RAssertMessage::ResumedAfterPanic(*kind),
            AssertMessage::ResumedAfterDrop(kind) => RAssertMessage::ResumedAfterDrop(*kind),
            AssertMessage::MisalignedPointer { required, found } => {
                RAssertMessage::MisalignedPointer {
                    required: self.operand(required)?,
                    found: self.operand(found)?,
                }
            }
            AssertMessage::NullPointer => RAssertMessage::NullPointer,
            AssertMessage::NullReference => RAssertMessage::NullReference,
            AssertMessage::InvalidEnum(v) => RAssertMessage::InvalidEnum(self.operand(v)?),
        })
    }

    fn call(&mut self, call: &Call) -> Result<TermShape, Decline> {
        let function = match &call.callee {
            Callee::Indirect { .. } => Some(self.operand(&call.function)?),
            _ => None,
        };
        let mut args = Vec::with_capacity(call.arguments.len());
        for argument in &call.arguments {
            args.push(self.operand(argument)?);
        }
        let dest_ty = call.destination.ty;
        let diverges = call.target.is_none();
        let zst = self.table.is_zst(dest_ty);
        let produces = !diverges && !zst && !matches!(call.callee, Callee::Nop);
        let kind = match &call.callee {
            // Compile-time-false on this target: the printer already spells these intrinsics
            // `false` (`js/intrinsics.rs`), but only a `Const` lets `opt::dce` fold the
            // `if (false)` arm away. Folded under the same flag as the `RuntimeCheck` switches
            // ([`build_with`]), so the `off` lane's text is untouched.
            Callee::Intrinsic { name, .. }
                if self.fold
                    && self.normal_blocks[self.block as usize].is_none()
                    && matches!(
                        self.interns.str(*name),
                        Some("is_val_statically_known" | "contract_checks")
                    ) =>
            {
                OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 1 })
            }
            Callee::Intrinsic {
                name,
                types,
                fallback,
            } => OpKind::Intrinsic {
                name: *name,
                types: types.clone(),
                fallback: *fallback,
                args,
                track_caller: call.track_caller,
            },
            callee => OpKind::Call {
                callee: callee.clone(),
                function,
                args,
                track_caller: call.track_caller,
                semantic: call.semantic,
                host: call.host.clone(),
            },
        };
        if let Some(normal) = self.normal_blocks[self.block as usize] {
            if produces {
                let value = self.op(kind, dest_ty);
                self.call_results[self.block as usize] = Some(value);
            } else {
                self.effect(kind);
            }
            let Unwind::Cleanup(unwind) = call.unwind else {
                unreachable!()
            };
            return Ok(TermShape::Invoke {
                normal,
                unwind: unwind.0,
            });
        }
        if produces {
            let value = self.op(kind, dest_ty);
            self.write_place(&call.destination, value)?;
        } else {
            self.effect(kind);
            if !diverges && (!zst || call.destination.projection.is_empty()) {
                // A `()`-returning call still defines its destination.
                let unit = self.op(OpKind::Const(ConstTree::Unit), dest_ty);
                self.write_place(&call.destination, unit)?;
            }
        }
        if !diverges {
            if let Some(locals) = self.analysis.assembly.after_call.get(&self.block) {
                self.fresh_storage(locals);
            }
        }
        Ok(match call.target {
            Some(target) => TermShape::Goto(target.0),
            None => TermShape::Unreachable,
        })
    }

    // -- finishing ----------------------------------------------------------------------------

    /// Resolves parameter replacements, drops removed parameters, and installs the terminators
    /// with their edge arguments.
    fn finish(mut self) -> Result<Func, Decline> {
        let n = self.successors.len();
        // Edge arguments: for each (pred, succ), the operands of succ's surviving params.
        let mut edge_args: BTreeMap<(u32, u32), Vec<ValueId>> = BTreeMap::new();
        for block in 0..n as u32 {
            if !self.reachable[block as usize] {
                continue;
            }
            let params: Vec<ValueId> = self.fb.func().blocks[block as usize].params.clone();
            let kept: Vec<ValueId> = params
                .iter()
                .copied()
                .filter(|param| self.params.get(param).is_none_or(|info| !info.removed))
                .collect();
            for (index, pred) in self.preds[block as usize].iter().enumerate() {
                let mut args = Vec::with_capacity(kept.len());
                for param in &kept {
                    let operand = self
                        .params
                        .get(param)
                        .and_then(|info| info.operands.get(index).copied().flatten());
                    match operand {
                        Some(operand) => args.push(self.resolve(operand)),
                        None => {
                            return Err(decline(
                                OpClass::Terminator,
                                "block parameter without an operand on an incoming edge",
                            ));
                        }
                    }
                }
                edge_args.insert((*pred, block), args);
            }
        }
        // Rewrite every value use through the replacement map and drop the removed params.
        let replaced = std::mem::take(&mut self.replaced);
        let resolve = |v: ValueId| -> ValueId {
            let mut value = v;
            let mut steps = 0;
            while let Some(next) = replaced.get(&value) {
                value = *next;
                steps += 1;
                if steps > replaced.len() {
                    break;
                }
            }
            value
        };
        let removed: BTreeSet<ValueId> = self
            .params
            .iter()
            .filter(|(_, info)| info.removed)
            .map(|(param, _)| *param)
            .collect();
        let terms = std::mem::take(&mut self.terms);
        let local_of_value = std::mem::take(&mut self.local_of_value);
        let local_origins: Vec<OriginId> = (0..self.body.locals.len())
            .map(|index| self.local_origin(LocalId(index as u32)))
            .collect();
        let mut func = self.fb.finish();
        // A value that defines a MIR local carries the local's origin, whose `local_name` is
        // what the value is named after.
        for (value, local) in local_of_value {
            if let Some(def) = func.values.get_mut(value.0 as usize) {
                if let Some(origin) = local_origins.get(local.0 as usize) {
                    def.origin = *origin;
                }
            }
        }
        for block in &mut func.blocks {
            block.params.retain(|param| !removed.contains(param));
            for op in &mut block.ops {
                op.kind.visit_values_mut(|v| *v = resolve(*v));
            }
        }
        for (index, shape) in terms.into_iter().enumerate() {
            let Some(shape) = shape else {
                continue;
            };
            let block = index as u32;
            let args = |target: u32| -> Vec<ValueId> {
                edge_args.get(&(block, target)).cloned().unwrap_or_default()
            };
            let term = match shape {
                TermShape::Goto(target) => RTerm::Goto(RBlockId(target), args(target)),
                TermShape::Invoke { normal, unwind } => RTerm::Invoke {
                    normal: (RBlockId(normal), args(normal)),
                    unwind: (RBlockId(unwind), args(unwind)),
                },
                TermShape::Switch {
                    scrut,
                    scrut_ty,
                    cases,
                    default,
                } => RTerm::Switch {
                    scrut: resolve(scrut),
                    scrut_ty,
                    cases: cases
                        .into_iter()
                        .map(|(value, target)| (value, RBlockId(target), args(target)))
                        .collect(),
                    default: (RBlockId(default), args(default)),
                },
                TermShape::Return(v) => RTerm::Return(v.map(resolve)),
                TermShape::Unreachable => RTerm::Unreachable,
                TermShape::Throw => RTerm::Throw(None),
            };
            func.blocks[index].term = term;
        }
        // A removed parameter is referenced by nothing now and defined by nothing either, which
        // the verifier rejects: compact it out of the value table.
        compact_values(&mut func, &removed);
        Ok(func)
    }
}

/// Removes the values in `dead` (unreferenced by construction) from the function's value table,
/// renumbering every remaining reference.
///
/// `pub(crate)` because `opt/dce.rs` deletes definitions too, and a value with no definition site
/// is what `Func::verify` rejects with "value vN is never defined".
pub(crate) fn compact_values(func: &mut Func, dead: &BTreeSet<ValueId>) {
    if dead.is_empty() {
        return;
    }
    let mut map: Vec<Option<ValueId>> = Vec::with_capacity(func.values.len());
    let mut values = Vec::with_capacity(func.values.len());
    for (index, def) in func.values.iter().enumerate() {
        if dead.contains(&ValueId(index as u32)) {
            map.push(None);
        } else {
            map.push(Some(ValueId(values.len() as u32)));
            values.push(*def);
        }
    }
    let renumber = |v: ValueId| -> ValueId { map[v.0 as usize].unwrap_or(v) };
    func.values = values;
    func.params = func.params.iter().map(|v| renumber(*v)).collect();
    for block in &mut func.blocks {
        block.params = block.params.iter().map(|v| renumber(*v)).collect();
        for op in &mut block.ops {
            if let Some(result) = op.result.as_mut() {
                *result = renumber(*result);
            }
            op.kind.visit_values_mut(|v| *v = renumber(*v));
        }
        block.term.visit_values_mut(|v| *v = renumber(*v));
    }
}

/// The single field write `[Downcast?, Field]` of a projection, if that is its whole shape.
fn single_field(projection: &[Projection]) -> (Option<u32>, Option<u32>) {
    match projection {
        [Projection::Field { field, .. }] => (None, Some(*field)),
        [Projection::Downcast { variant }, Projection::Field { field, .. }] => {
            (Some(*variant), Some(*field))
        }
        _ => (None, None),
    }
}

/// The successors of a terminator along non-unwind edges.
/// The one target of a `Switch` on a `RuntimeCheck` constant (`ub_checks`), which is decided
/// at compile time: the case matching the constant, else `otherwise`. The other arm — the
/// `precondition_check` call — is dead code v1's per-item optimizer folded away before it
/// counted references, and it never enters the graph here.
fn folded_switch(terminator: &Terminator) -> Option<BlockId> {
    let Terminator::Switch {
        discriminant:
            Operand {
                kind: OperandKind::RuntimeCheck(value),
                ..
            },
        targets,
        otherwise,
    } = terminator
    else {
        return None;
    };
    let wanted = u128::from(*value);
    Some(
        targets
            .iter()
            .find(|(case, _)| *case == wanted)
            .map_or(*otherwise, |(_, target)| *target),
    )
}

/// Assertions follow the abort contract; imaginary edges do not execute. A no-glue drop
/// has no operation that could throw. Calls and observable drops retain their cleanup edge.
fn cleanup_target(terminator: &Terminator) -> Option<BlockId> {
    match terminator {
        Terminator::Call(Call {
            unwind: Unwind::Cleanup(target),
            ..
        }) => Some(*target),
        Terminator::Drop {
            unwind: Unwind::Cleanup(target),
            glue,
            ..
        } if !matches!(glue, crate::jir::semantic::DropGlue::None) => Some(*target),
        _ => None,
    }
}

fn block_successors(terminator: &Terminator, fold: bool) -> Vec<BlockId> {
    if fold {
        if let Some(target) = folded_switch(terminator) {
            return vec![target];
        }
    }
    match terminator {
        Terminator::Goto(target) => vec![*target],
        Terminator::Switch {
            targets, otherwise, ..
        } => {
            let mut out: Vec<BlockId> = targets.iter().map(|(_, target)| *target).collect();
            out.push(*otherwise);
            out
        }
        Terminator::Call(call) => call.target.into_iter().collect(),
        Terminator::Drop { target, .. } | Terminator::Assert { target, .. } => vec![*target],
        Terminator::FalseEdge { real, .. } | Terminator::FalseUnwind { real, .. } => vec![*real],
        Terminator::Return
        | Terminator::Unreachable
        | Terminator::UnwindResume
        | Terminator::UnwindTerminate(_) => Vec::new(),
    }
}

/// The reachable blocks in reverse postorder, the reachability flags, and the
/// predecessor lists (deduplicated, in discovery order) over those blocks.
fn cfg(successors: &[Vec<BlockId>]) -> (Vec<u32>, Vec<bool>, Vec<Vec<u32>>) {
    let n = successors.len();
    let mut reachable = vec![false; n];
    let mut postorder = Vec::new();
    let mut stack: Vec<(u32, usize)> = Vec::new();
    if n > 0 {
        reachable[0] = true;
        stack.push((0, 0));
    }
    while let Some((block, next)) = stack.last_mut() {
        let edges = &successors[*block as usize];
        if *next < edges.len() {
            let successor = edges[*next].0 as usize;
            *next += 1;
            if successor < n && !reachable[successor] {
                reachable[successor] = true;
                stack.push((successor as u32, 0));
            }
        } else {
            postorder.push(*block);
            stack.pop();
        }
    }
    postorder.reverse();
    let mut preds: Vec<Vec<u32>> = vec![Vec::new(); n];
    for &block in &postorder {
        for successor in &successors[block as usize] {
            let index = successor.0 as usize;
            if index < n && reachable[index] && !preds[index].contains(&block) {
                preds[index].push(block);
            }
        }
    }
    (postorder, reachable, preds)
}

// -------------------------------------------------------------------------------------------
// Tests: hand-built bodies, and every fixture object's bodies, through build → verify → plan →
// structure and the two interpreters.
// -------------------------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod check {
    //! The dual-interpreter harness: evaluates a JIR-S body and the JIR-R function built from
    //! it on generated arguments and compares outcomes.

    use std::collections::BTreeMap;

    use crate::jir::interns::Interns;
    use crate::jir::interp::{
        Canon, Heap, Hooks, Location, Outcome, ReprInterpreter, SemanticInterpreter, Trap, Value,
    };
    use crate::jir::repr::Func;
    use crate::jir::semantic::{Body, DirectItemRef, LocalRole, TypeId};
    use crate::jir::types::{TypeKind, TypeTable};

    /// One argument vector: each argument as the value to pass, or the pointee to allocate for a
    /// pointer argument.
    pub type ArgumentSet = Vec<(Value, Option<Value>)>;

    /// What the comparison of one argument vector found.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Verdict {
        /// Both sides agree on the result or trap, ordered trace and final pointees.
        Agree,
        /// One side hit something outside the interpreters' subset: not comparable.
        Skipped(String),
        /// The two sides disagree.
        Mismatch(String),
    }

    /// The zero of `ty`, for stubbing call results.
    pub fn zero_value(ty: TypeId, table: &TypeTable) -> Result<Value, Trap> {
        let descriptor = table.get(ty).ok_or(Trap::Hook("unknown type".to_owned()))?;
        Ok(match &descriptor.kind {
            TypeKind::Pattern(inner) => zero_value(*inner, table)?,
            TypeKind::Bool => Value::Bool(false),
            TypeKind::Char => Value::Char('\0'),
            TypeKind::Integer { signed, bits } => Value::int(0, *bits, *signed),
            TypeKind::Float { bits } => Value::float(0.0, *bits),
            TypeKind::Str => Value::Str(String::new()),
            TypeKind::Tuple(elements) => {
                if elements.is_empty() {
                    Value::Unit
                } else {
                    Value::Aggregate {
                        ty,
                        variant: 0,
                        fields: elements
                            .iter()
                            .map(|element| zero_value(*element, table))
                            .collect::<Result<_, _>>()?,
                    }
                }
            }
            TypeKind::Array { element, length } => Value::Aggregate {
                ty,
                variant: 0,
                fields: vec![zero_value(*element, table)?; *length as usize],
            },
            TypeKind::Adt { variants, .. } => {
                let variant = variants
                    .first()
                    .ok_or(Trap::Hook("uninhabited".to_owned()))?;
                Value::Aggregate {
                    ty,
                    variant: 0,
                    fields: variant
                        .fields
                        .iter()
                        .map(|field| zero_value(field.ty, table))
                        .collect::<Result<_, _>>()?,
                }
            }
            TypeKind::Closure { upvars, .. } => Value::Aggregate {
                ty,
                variant: 0,
                fields: upvars
                    .iter()
                    .map(|upvar| zero_value(*upvar, table))
                    .collect::<Result<_, _>>()?,
            },
            TypeKind::FnDef { .. } => Value::Opaque("function item value"),
            _ => return Err(Trap::Hook(format!("no zero for {:?}", descriptor.kind))),
        })
    }

    /// Fails when a run loaded nothing and the environment asked for a non-empty corpus.
    ///
    /// The corpus is whatever `build/` holds, so a checkout that has not run the suites finds
    /// nothing and every property test passes without checking a thing. Setting
    /// `RCGJS_SSA_FIXTURES_REQUIRED=1` — which `scripts/check.sh` does whenever the fixtures are
    /// there — turns that vacuous pass into a failure.
    pub fn require_fixtures(loaded: usize, what: &str) {
        assert!(
            loaded > 0 || !std::env::var("RCGJS_SSA_FIXTURES_REQUIRED").is_ok_and(|v| v == "1"),
            "{what}: RCGJS_SSA_FIXTURES_REQUIRED=1 but the corpus loaded no bodies",
        );
    }

    /// The result a stubbed call returns: derived from the callee and its arguments.
    ///
    /// The zero of the result type would hide every transformation error that is only observable
    /// when a callee returns something other than the zero — a different enum variant, a nonzero
    /// field, a pointer-free payload the caller then reads back. The value here is a hash of the
    /// callee's link name and the scalar content of its arguments, spread over the result type's
    /// scalar fields and used to pick the variant of an enum, so it is stable for one call site
    /// and different between two. Heap identity is deliberately left out of the hash: it is
    /// run-local, and the two sides of a comparison allocate independently. A result the harness
    /// cannot represent still traps, as the zero did.
    pub fn stub_result(
        callee: &DirectItemRef,
        args: &[Value],
        ty: TypeId,
        table: &TypeTable,
    ) -> Result<Value, Trap> {
        let mut seed = 0xcbf2_9ce4_8422_2325_u64;
        mix(&mut seed, u64::from(callee.link_name.0));
        for arg in args {
            fold(arg, &mut seed);
        }
        stub_value(ty, seed, table)
    }

    /// One FNV-1a round.
    fn mix(seed: &mut u64, value: u64) {
        *seed = (*seed ^ value).wrapping_mul(0x0100_0000_01b3);
    }

    /// Folds the scalar content of `value` into `seed`, skipping heap identity.
    fn fold(value: &Value, seed: &mut u64) {
        match value {
            Value::Undef => mix(seed, 1),
            Value::Unit => mix(seed, 2),
            Value::Bool(bit) => mix(seed, 3 + u64::from(*bit)),
            Value::Int { value, bits, .. } => {
                mix(seed, *value as u64);
                mix(seed, u64::from(*bits));
            }
            // Every `NaN` folds the same way, as it normalizes the same way.
            Value::Float { value, .. } => {
                mix(seed, if value.is_nan() { 5 } else { value.to_bits() })
            }
            Value::Char(c) => mix(seed, u64::from(u32::from(*c))),
            Value::Str(text) => {
                for byte in text.as_bytes() {
                    mix(seed, u64::from(*byte));
                }
            }
            Value::Aggregate {
                variant, fields, ..
            } => {
                mix(seed, u64::from(*variant));
                for field in fields {
                    fold(field, seed);
                }
            }
            Value::Ptr(_) => mix(seed, 6),
            Value::Slice { off, len, .. } => {
                mix(seed, *off);
                mix(seed, *len);
            }
            Value::Fn(name) => mix(seed, u64::from(name.0)),
            Value::Opaque(_) => mix(seed, 7),
        }
    }

    /// A value of `ty` derived from `seed`. Integers stay small and nonzero so a stubbed result
    /// used as a length or an index does not blow the step budget.
    fn stub_value(ty: TypeId, seed: u64, table: &TypeTable) -> Result<Value, Trap> {
        let descriptor = table.get(ty).ok_or(Trap::Hook("unknown type".to_owned()))?;
        Ok(match &descriptor.kind {
            TypeKind::Pattern(inner) => stub_value(*inner, seed, table)?,
            TypeKind::Bool => Value::Bool(seed & 1 == 1),
            TypeKind::Char => Value::Char(char::from(b'a' + (seed % 26) as u8)),
            TypeKind::Integer { signed, bits } => {
                Value::int(u128::from(seed % 251 + 1), *bits, *signed)
            }
            TypeKind::Float { bits } => Value::float((seed % 251) as f64 + 0.5, *bits),
            TypeKind::Str => Value::Str(format!("s{}", seed % 251)),
            TypeKind::Tuple(elements) if elements.is_empty() => Value::Unit,
            TypeKind::Tuple(elements) => {
                stub_aggregate(ty, 0, elements.iter().copied(), seed, table)?
            }
            TypeKind::Array { element, length } => {
                stub_aggregate(ty, 0, (0..*length as usize).map(|_| *element), seed, table)?
            }
            TypeKind::Closure { upvars, .. } => {
                stub_aggregate(ty, 0, upvars.iter().copied(), seed, table)?
            }
            TypeKind::Adt { variants, .. } => {
                if variants.is_empty() {
                    return Err(Trap::Hook("uninhabited".to_owned()));
                }
                // Cycle from the variant the seed picks. A payload the harness cannot build (a
                // recursive variant behind a pointer) falls through to the next one, so an enum
                // still lands on a variant rather than declining the whole call.
                let start = (seed % variants.len() as u64) as usize;
                (0..variants.len())
                    .map(|offset| (start + offset) % variants.len())
                    .find_map(|index| {
                        stub_aggregate(
                            ty,
                            index as u32,
                            variants[index].fields.iter().map(|field| field.ty),
                            seed,
                            table,
                        )
                        .ok()
                    })
                    .ok_or(Trap::Hook("no representable variant".to_owned()))?
            }
            TypeKind::FnDef { .. } => Value::Opaque("function item value"),
            _ => return Err(Trap::Hook(format!("no stub for {:?}", descriptor.kind))),
        })
    }

    fn stub_aggregate(
        ty: TypeId,
        variant: u32,
        fields: impl Iterator<Item = TypeId>,
        seed: u64,
        table: &TypeTable,
    ) -> Result<Value, Trap> {
        Ok(Value::Aggregate {
            ty,
            variant,
            fields: fields
                .enumerate()
                .map(|(index, field)| {
                    let mut seed = seed;
                    mix(&mut seed, index as u64);
                    stub_value(field, seed, table)
                })
                .collect::<Result<_, _>>()?,
        })
    }

    /// A few small values of `ty` to try, or `None` when the type has no in-heap sample.
    fn samples(ty: TypeId, table: &TypeTable) -> Option<Vec<Value>> {
        let descriptor = table.get(ty)?;
        Some(match &descriptor.kind {
            TypeKind::Bool => vec![Value::Bool(false), Value::Bool(true)],
            TypeKind::Char => vec![Value::Char('a'), Value::Char('\u{1F600}')],
            TypeKind::Integer { signed, bits } => {
                let mut out = vec![Value::int(0, *bits, *signed), Value::int(1, *bits, *signed)];
                out.push(Value::int(3, *bits, *signed));
                if *signed {
                    out.push(Value::int((-2i128) as u128, *bits, *signed));
                }
                out.push(Value::int(u128::MAX, *bits, *signed));
                out
            }
            TypeKind::Float { bits } => vec![Value::float(0.0, *bits), Value::float(2.5, *bits)],
            TypeKind::Str => vec![Value::Str(String::new()), Value::Str("hi".to_owned())],
            TypeKind::Tuple(elements) if elements.is_empty() => vec![Value::Unit],
            TypeKind::Tuple(_)
            | TypeKind::Array { .. }
            | TypeKind::Adt { .. }
            | TypeKind::Closure { .. } => {
                // The zero, and one with every scalar field bumped.
                let zero = zero_value(ty, table).ok()?;
                vec![zero.clone(), bump(&zero)]
            }
            _ => return None,
        })
    }

    fn bump(value: &Value) -> Value {
        match value {
            Value::Int {
                value,
                bits,
                signed,
            } => Value::int((*value + 1) as u128, *bits, *signed),
            Value::Bool(_) => Value::Bool(true),
            Value::Aggregate {
                ty,
                variant,
                fields,
            } => Value::Aggregate {
                ty: *ty,
                variant: *variant,
                fields: fields.iter().map(bump).collect(),
            },
            other => other.clone(),
        }
    }

    /// One argument vector per parameter combination (bounded), each with the heap objects
    /// pointer arguments need. `None` when a parameter type has no sample.
    pub fn argument_sets(params: &[TypeId], table: &TypeTable) -> Option<Vec<ArgumentSet>> {
        // Per parameter: the direct samples, or `(pointer, pointee)` pairs.
        let mut per_param: Vec<ArgumentSet> = Vec::new();
        for ty in params {
            let descriptor = table.get(*ty)?;
            let entries: Vec<(Value, Option<Value>)> = match &descriptor.kind {
                TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
                    let pointee_desc = table.get(*pointee)?;
                    match &pointee_desc.kind {
                        TypeKind::Str => vec![
                            (Value::Str(String::new()), None),
                            (Value::Str("hi".to_owned()), None),
                        ],
                        TypeKind::Slice(element) => {
                            let zero = zero_value(*element, table).ok()?;
                            let three = Value::Aggregate {
                                ty: *pointee,
                                variant: 0,
                                fields: vec![zero.clone(), bump(&zero), bump(&bump(&zero))],
                            };
                            let empty = Value::Aggregate {
                                ty: *pointee,
                                variant: 0,
                                fields: Vec::new(),
                            };
                            // The pointer half is filled in once the pointee is allocated.
                            vec![(Value::Undef, Some(three)), (Value::Undef, Some(empty))]
                        }
                        _ => samples(*pointee, table)?
                            .into_iter()
                            .map(|pointee| (Value::Undef, Some(pointee)))
                            .collect(),
                    }
                }
                _ => samples(*ty, table)?
                    .into_iter()
                    .map(|value| (value, None))
                    .collect(),
            };
            per_param.push(entries);
        }
        // The cartesian product, capped.
        let mut sets: Vec<ArgumentSet> = vec![Vec::new()];
        for entries in per_param {
            let mut next = Vec::new();
            for set in &sets {
                for entry in &entries {
                    let mut extended = set.clone();
                    extended.push(entry.clone());
                    next.push(extended);
                }
            }
            sets = next;
            if sets.len() > 64 {
                sets.truncate(64);
            }
        }
        Some(sets)
    }

    /// Allocates the pointees of one argument set and returns the values to pass.
    fn materialize(
        set: &[(Value, Option<Value>)],
        heap: &mut Heap,
        table: &TypeTable,
    ) -> Vec<Value> {
        set.iter()
            .map(|(value, pointee)| match pointee {
                None => value.clone(),
                Some(pointee) => {
                    let id = heap.alloc(pointee.clone());
                    let _ = table;
                    match pointee {
                        Value::Aggregate { fields, ty, .. }
                            if matches!(
                                table.get(*ty).map(|d| &d.kind),
                                Some(TypeKind::Slice(_))
                            ) =>
                        {
                            Value::Slice {
                                buf: Location::root(id),
                                off: 0,
                                len: fields.len() as u64,
                            }
                        }
                        _ => Value::Ptr(Location::root(id)),
                    }
                }
            })
            .collect()
    }

    /// Canonical heap identity for this comparison: per observation, not per run.
    ///
    /// The two sides model storage differently. JIR-S keeps one slot per local for the whole
    /// body; JIR-R makes a fresh object per evaluation, so a loop that re-creates a local passes
    /// the same address every iteration on one side and a new one on the other. Both are correct
    /// models of the same program, so identity *across* observations is not comparable here — but
    /// identity *within* one is: two arguments of one call alias or they do not, and two slice
    /// arguments share a backing buffer or they do not. So each event, and the result, gets its
    /// own numbering. (The single-model harnesses — PEA, DCE — number per run instead, and see
    /// the whole alias graph.)
    fn canonical(outcome: &Outcome) -> Outcome {
        Outcome {
            ret: Canon::new().result(&outcome.ret),
            trace: outcome
                .trace
                .iter()
                .map(|event| Canon::new().event(event))
                .collect(),
            pointees: outcome
                .pointees
                .iter()
                .map(|pointee| Canon::new().value(pointee))
                .collect(),
        }
    }

    fn compare_outcomes(before: Outcome, after: Outcome) -> Verdict {
        match (&before.ret, &after.ret) {
            (Err(Trap::Malformed(_)), _) | (_, Err(Trap::Malformed(_))) => Verdict::Mismatch(
                format!("malformed execution: before {before:?} vs after {after:?}"),
            ),
            (Err(Trap::Unsupported(what)), _) | (_, Err(Trap::Unsupported(what))) => {
                Verdict::Skipped(format!("unsupported: {what}"))
            }
            (Err(Trap::Hook(what)), _) | (_, Err(Trap::Hook(what))) => {
                Verdict::Skipped(format!("hook: {what}"))
            }
            // The interpreters and optimized bodies count steps differently.
            (Err(Trap::Steps), _) | (_, Err(Trap::Steps)) => {
                Verdict::Skipped("step budget".to_owned())
            }
            // A transformation owes only defined executions of the original program.
            (Err(Trap::Ub(what)), _) => Verdict::Skipped(format!("original UB: {what}")),
            _ if before == after => Verdict::Agree,
            _ => Verdict::Mismatch(format!("before {before:?} vs after {after:?}")),
        }
    }

    /// Compares full JIR-R executions, including effects before a modeled failure.
    pub fn compare_funcs(
        before: &Func,
        after: &Func,
        table: &TypeTable,
        interns: &Interns,
        results: &BTreeMap<crate::jir::interns::StrId, TypeId>,
        set: &[(Value, Option<Value>)],
        step_limit: u64,
    ) -> Verdict {
        let hook = |callee: &DirectItemRef, args: Vec<Value>| -> Result<Value, Trap> {
            match results.get(&callee.link_name) {
                Some(ty) => stub_result(callee, &args, *ty, table),
                None => Err(Trap::Hook("callee outside the object".to_owned())),
            }
        };
        let mut hooks = Hooks::new(&hook);
        hooks.step_limit = step_limit;
        let run = |func: &Func| {
            let mut heap = Heap::new();
            let args = materialize(set, &mut heap, table);
            let tracked = args.clone();
            let (ret, trace) = {
                let mut interpreter = ReprInterpreter::new(func, table, interns, &hooks, &mut heap);
                (interpreter.run(args), interpreter.trace)
            };
            Canon::new().outcome(&Outcome {
                ret,
                trace,
                pointees: crate::jir::interp::pointee_snapshots(&tracked, &heap),
            })
        };
        compare_outcomes(run(before), run(after))
    }

    /// Runs both interpreters on `body`/`func` for one argument set.
    pub fn compare(
        body: &Body,
        func: &Func,
        table: &TypeTable,
        interns: &Interns,
        results: &BTreeMap<crate::jir::interns::StrId, TypeId>,
        set: &[(Value, Option<Value>)],
        step_limit: u64,
    ) -> Verdict {
        // A `SetDiscriminant` through a pointer relies on the baseline carrier's aliasing
        // `Deref` (`ssa.rs::set_discriminant`), which the JIR-R interpreter's value semantics do
        // not model.
        if body.blocks.iter().any(|block| {
            block.statements.iter().any(|statement| {
                matches!(
                    &statement.kind,
                    crate::jir::semantic::StatementKind::SetDiscriminant { place, .. }
                        if !place.projection.is_empty() && table.is_indirect(place.ty)
                )
            })
        }) {
            return Verdict::Skipped("set_discriminant through a pointer".to_owned());
        }
        // A host marker call reads its arguments at lowering time (`ssa.rs`, consumed
        // arguments): the two sides pass them differently on purpose.
        if body.blocks.iter().any(|block| {
            matches!(&block.terminator, crate::jir::semantic::Terminator::Call(call) if call.host.is_some())
        }) {
            return Verdict::Skipped("host marker call".to_owned());
        }
        let hook = |callee: &DirectItemRef, args: Vec<Value>| -> Result<Value, Trap> {
            match results.get(&callee.link_name) {
                Some(ty) => stub_result(callee, &args, *ty, table),
                None => Err(Trap::Hook("callee outside the object".to_owned())),
            }
        };
        let mut hooks = Hooks::new(&hook);
        hooks.step_limit = step_limit;

        let mut heap_s = Heap::new();
        let args_s = materialize(set, &mut heap_s, table);
        // The pointer arguments, kept aside so the final pointees are compared after each run:
        // a store through a `&mut` parameter that is never read back is invisible in the result
        // and the trace. Captured before the spread expansion below so both sides track the same
        // list (`materialize` builds them from the same `set`).
        let tracked: Vec<Value> = args_s
            .iter()
            .filter(|arg| matches!(arg, Value::Ptr(_) | Value::Slice { .. }))
            .cloned()
            .collect();
        let semantic = {
            let (ret, trace) =
                match SemanticInterpreter::new(body, table, interns, &hooks, &mut heap_s, args_s) {
                    Ok(mut interpreter) => (interpreter.run(), interpreter.trace),
                    Err(trap) => (Err(trap), Vec::new()),
                };
            Outcome {
                ret,
                trace,
                pointees: crate::jir::interp::pointee_snapshots(&tracked, &heap_s),
            }
        };

        let mut heap_r = Heap::new();
        let mut args_r = materialize(set, &mut heap_r, table);
        // This side's copies of the tracked pointers: same `set`, this heap's locations.
        let tracked: Vec<Value> = args_r
            .iter()
            .filter(|arg| matches!(arg, Value::Ptr(_) | Value::Slice { .. }))
            .cloned()
            .collect();
        // A spread tuple arrives at the function as its elements.
        if let Some(spread) = body.signature.abi.spread {
            let position = body
                .locals
                .iter()
                .filter(|local| local.role == LocalRole::Argument)
                .position(|local| local.id == spread.local);
            if let Some(position) = position {
                if let Some(Value::Aggregate { fields, .. }) = args_r.get(position).cloned() {
                    args_r.remove(position);
                    for (offset, field) in fields.into_iter().enumerate() {
                        args_r.insert(position + offset, field);
                    }
                }
            }
        }
        if body.signature.abi.caller_location {
            args_r.push(Value::Opaque("caller location"));
        }
        let repr = {
            let (ret, trace) = {
                let mut interpreter =
                    ReprInterpreter::new(func, table, interns, &hooks, &mut heap_r);
                (interpreter.run(args_r), interpreter.trace)
            };
            Outcome {
                ret,
                trace,
                pointees: crate::jir::interp::pointee_snapshots(&tracked, &heap_r),
            }
        };
        compare_outcomes(canonical(&semantic), canonical(&repr))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::check::{argument_sets, compare, Verdict};
    use super::*;
    use crate::jir::interp::Value;
    use crate::jir::semantic::{
        AbiSignature, BasicBlock, BinaryOp, BodySemantic, DirectItemRef, DropGlue, Local,
        Signature, Statement, Unwind,
    };
    use crate::jir::types::*;
    use crate::lower::fuse;
    use crate::lower::structure;
    use crate::opts::Opts;

    const UNIT: TypeId = TypeId(1);
    const BOOL: TypeId = TypeId(2);
    const I32: TypeId = TypeId(3);
    const USIZE: TypeId = TypeId(5);
    const OPTION: TypeId = TypeId(7);
    const POINT: TypeId = TypeId(8);
    const ARRAY: TypeId = TypeId(9);
    const SLICE: TypeId = TypeId(10);
    const REF_POINT: TypeId = TypeId(11);
    const REF_I32: TypeId = TypeId(12);
    const REF_SLICE: TypeId = TypeId(13);
    const WRAP: TypeId = TypeId(14);

    fn descriptor(id: TypeId, kind: TypeKind, size: u64) -> TypeDescriptor {
        TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                size: Some(size),
                unsized_prefix: None,
                align: Some(size.clamp(1, 4)),
                sized: true,
                zst: size == 0,
                one_zst: size == 0,
                needs_drop: false,
                has_own_drop: false,
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: size == 0,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: id,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        }
    }

    fn int(id: TypeId, signed: bool, bits: u32) -> TypeDescriptor {
        let mut descriptor =
            descriptor(id, TypeKind::Integer { signed, bits }, u64::from(bits / 8));
        descriptor.js.int = Some(IntFacts {
            signed,
            bits,
            repr: IntRepr::Number,
        });
        descriptor
    }

    fn variant(name: StrId, fields: Vec<(FieldKey, TypeId)>, discr: i128) -> VariantDescriptor {
        VariantDescriptor {
            name,
            fields: fields
                .into_iter()
                .map(|(key, ty)| FieldDescriptor { key, ty, offset: 0 })
                .collect(),
            logical_discr: discr,
            physical_tag: Some(NominalDiscriminant {
                bits: discr as u128,
                bytes: 4,
            }),
            result_ok: false,
            saved_local_keys: Vec::new(),
        }
    }

    fn reference(id: TypeId, pointee: TypeId, form: PointerForm) -> TypeDescriptor {
        let mut descriptor = descriptor(
            id,
            TypeKind::Reference {
                pointee,
                mutable: true,
            },
            4,
        );
        descriptor.js.pointer_form = Some(form);
        descriptor
    }

    /// `()`, `bool`, `i32`, `usize`, `Option<i32>`, `Point { x, y }`, `[i32; 4]`, `[i32]`,
    /// `Wrap(i32)` (`repr(transparent)`), `&mut Point`, `&mut i32`, `&mut [i32]`. `with_refs`
    /// leaves the reference types out.
    fn world(with_refs: bool) -> Interns {
        let mut interns = Interns::new();
        let def = |n: u64| crate::jir::semantic::DefinitionId {
            crate_id: 1,
            local_hash: n,
        };
        let none = interns.intern_str("None");
        let some = interns.intern_str("Some");
        let field_0 = interns.intern_str("_0");
        let x = interns.intern_str("x");
        let y = interns.intern_str("y");
        let point = interns.intern_str("Point");
        interns.intern_type(descriptor(UNIT, TypeKind::Tuple(Vec::new()), 0));
        interns.intern_type(descriptor(BOOL, TypeKind::Bool, 1));
        interns.intern_type(int(I32, true, 32));
        interns.intern_type(int(USIZE, false, 32));
        let mut option = descriptor(
            OPTION,
            TypeKind::Adt {
                def: def(10),
                args: vec![I32],
                form: NominalForm::Enum,
                variants: vec![
                    variant(none, Vec::new(), 0),
                    variant(some, vec![(FieldKey::Name(field_0), I32)], 1),
                ],
                niche: None,
            },
            8,
        );
        option.js.is_indirect = true;
        option.js.needs_clone = true;
        option.js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
        interns.intern_type(option);
        let mut point_ty = descriptor(
            POINT,
            TypeKind::Adt {
                def: def(11),
                args: Vec::new(),
                form: NominalForm::Struct,
                variants: vec![{
                    let mut v = variant(
                        point,
                        vec![(FieldKey::Name(x), I32), (FieldKey::Name(y), I32)],
                        0,
                    );
                    v.physical_tag = None;
                    v
                }],
                niche: None,
            },
            8,
        );
        point_ty.js.is_indirect = true;
        point_ty.js.needs_clone = true;
        point_ty.js.fields = vec![
            FieldDescriptor {
                key: FieldKey::Name(x),
                ty: I32,
                offset: 0,
            },
            FieldDescriptor {
                key: FieldKey::Name(y),
                ty: I32,
                offset: 4,
            },
        ];
        interns.intern_type(point_ty);
        let mut array = descriptor(
            ARRAY,
            TypeKind::Array {
                element: I32,
                length: 4,
            },
            16,
        );
        array.js.is_indirect = true;
        array.js.needs_clone = true;
        interns.intern_type(array);
        let mut slice = descriptor(SLICE, TypeKind::Slice(I32), 0);
        slice.layout.sized = false;
        slice.layout.size = None;
        slice.layout.zst = false;
        slice.js.is_indirect = true;
        interns.intern_type(slice);
        let wrap = interns.intern_str("Wrap");
        let mut wrap_ty = descriptor(
            WRAP,
            TypeKind::Adt {
                def: def(12),
                args: Vec::new(),
                form: NominalForm::Struct,
                variants: vec![{
                    let mut v = variant(wrap, vec![(FieldKey::Transparent, I32)], 0);
                    v.physical_tag = None;
                    v
                }],
                niche: None,
            },
            4,
        );
        wrap_ty.js.transparent = Some(TransparentField { field: 0, ty: I32 });
        wrap_ty.js.peeled = I32;
        interns.intern_type(wrap_ty);
        if with_refs {
            interns.intern_type(reference(REF_POINT, POINT, PointerForm::Thin));
            interns.intern_type(reference(REF_I32, I32, PointerForm::Slot));
            interns.intern_type(reference(REF_SLICE, SLICE, PointerForm::FatSlice));
        }
        interns
    }

    fn signature(arguments: Vec<TypeId>, result: TypeId) -> Signature {
        Signature {
            abi: AbiSignature {
                parameters: arguments.len() as u16,
                spread: None,
                caller_location: false,
                inline_never: false,
                inline_hint: false,
                inline_always: false,
            },
            arguments,
            result,
        }
    }

    fn local(id: u32, role: LocalRole, ty: TypeId) -> Local {
        Local {
            id: LocalId(id),
            role,
            ty,
        }
    }

    fn place(local: u32, projection: Vec<Projection>, ty: TypeId) -> Place {
        Place {
            local: LocalId(local),
            projection,
            ty,
        }
    }

    fn copy(local: u32, ty: TypeId) -> Operand {
        Operand {
            ty,
            kind: OperandKind::Copy(place(local, Vec::new(), ty)),
        }
    }

    fn mv(local: u32, ty: TypeId) -> Operand {
        Operand {
            ty,
            kind: OperandKind::Move(place(local, Vec::new(), ty)),
        }
    }

    fn copy_place(place: Place) -> Operand {
        Operand {
            ty: place.ty,
            kind: OperandKind::Copy(place),
        }
    }

    fn i32_const(value: i32) -> Operand {
        Operand {
            ty: I32,
            kind: OperandKind::Constant(Constant::Scalar {
                bits: value as u32 as u128,
                bytes: 4,
            }),
        }
    }

    fn assign(destination: Place, value: Rvalue) -> Statement {
        Statement {
            kind: StatementKind::Assign { destination, value },
        }
    }

    fn block(statements: Vec<Statement>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            id: BlockId(0),
            cleanup: false,
            statements,
            terminator,
        }
    }

    fn body(signature_of: Signature, locals: Vec<Local>, mut blocks: Vec<BasicBlock>) -> Body {
        for (index, block) in blocks.iter_mut().enumerate() {
            block.id = BlockId(index as u32);
        }
        Body {
            item: crate::jir::semantic::ItemId {
                definition: crate::jir::semantic::DefinitionId {
                    crate_id: 1,
                    local_hash: 1,
                },
                instantiation: 1,
            },
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: signature_of,
            locals,
            blocks,
        }
    }

    fn switch_bool(local: u32, then_: u32, else_: u32) -> Terminator {
        Terminator::Switch {
            discriminant: copy(local, BOOL),
            targets: vec![(0, BlockId(else_))],
            otherwise: BlockId(then_),
        }
    }

    /// Builds, verifies, plans, structures, and compares on generated arguments. Returns the
    /// function and the plan.
    fn check(interns: &Interns, body: &Body) -> (Func, super::super::plan::Plan) {
        body.verify(&interns.types, interns).expect("body verifies");
        let func = build(body, None, &interns.types, interns).expect("builds");
        func.verify(&interns.types).expect("verifies");
        let plan = fuse::plan(&func, None, &interns.types, interns, &Opts::default());
        let structured =
            structure::structure(&func, &plan, &interns.types, false).expect("structures");
        let _ = structured.stats;
        let sets =
            argument_sets(&body.signature.arguments, &interns.types).expect("argument samples");
        let mut agreed = 0;
        for set in &sets {
            match compare(
                body,
                &func,
                &interns.types,
                interns,
                &BTreeMap::new(),
                set,
                10_000,
            ) {
                Verdict::Agree => agreed += 1,
                Verdict::Skipped(why) => panic!("not comparable: {why}"),
                Verdict::Mismatch(why) => panic!("mismatch on {set:?}: {why}"),
            }
        }
        assert!(agreed > 0);
        (func, plan)
    }

    #[test]
    fn failed_execution_preserves_effects_in_differentials() {
        let interns = world(true);
        let source = body(
            signature(vec![REF_I32, I32], I32),
            vec![
                local(0, LocalRole::Return, I32),
                local(1, LocalRole::Argument, REF_I32),
                local(2, LocalRole::Argument, I32),
            ],
            vec![block(
                vec![assign(
                    place(1, vec![Projection::Deref], I32),
                    Rvalue::Use(copy(2, I32)),
                )],
                Terminator::UnwindResume,
            )],
        );
        source.verify(&interns.types, &interns).unwrap();
        let original = build(&source, None, &interns.types, &interns).unwrap();
        original.verify(&interns.types).unwrap();
        let mut faulty = original.clone();
        let mut removed = 0;
        for block in &mut faulty.blocks {
            block.ops.retain(|op| {
                if matches!(op.kind, OpKind::StoreThrough { .. }) {
                    removed += 1;
                    false
                } else {
                    true
                }
            });
        }
        assert_eq!(removed, 1);
        faulty.verify(&interns.types).unwrap();
        let set = [
            (Value::Undef, Some(Value::int(0, 32, true))),
            (Value::int(7, 32, true), None),
        ];
        let compare = |func| {
            super::check::compare(
                &source,
                func,
                &interns.types,
                &interns,
                &BTreeMap::new(),
                &set,
                100,
            )
        };
        assert_eq!(compare(&original), Verdict::Agree);
        assert!(matches!(compare(&faulty), Verdict::Mismatch(_)));
        assert!(matches!(
            super::check::compare_funcs(
                &original,
                &faulty,
                &interns.types,
                &interns,
                &BTreeMap::new(),
                &set,
                100,
            ),
            Verdict::Mismatch(_)
        ));
        let undefined = [(Value::Undef, None), (Value::int(7, 32, true), None)];
        assert!(matches!(
            super::check::compare_funcs(
                &original,
                &faulty,
                &interns.types,
                &interns,
                &BTreeMap::new(),
                &undefined,
                100,
            ),
            Verdict::Skipped(_)
        ));
        assert!(matches!(
            super::check::compare_funcs(
                &original,
                &original,
                &interns.types,
                &interns,
                &BTreeMap::new(),
                &[],
                100,
            ),
            Verdict::Mismatch(_)
        ));
    }

    #[test]
    fn failed_execution_keeps_call_order_and_multiplicity() {
        use crate::jir::semantic::CallSemantic;

        let mut interns = world(false);
        let name = interns.intern_str("observed-call");
        let item = crate::jir::semantic::ItemId {
            definition: crate::jir::semantic::DefinitionId {
                crate_id: 1,
                local_hash: 1,
            },
            instantiation: 1,
        };
        let origin = crate::jir::origin::OriginId(0);
        let mut b = FuncBuilder::new(item, signature(vec![I32, I32], I32));
        let first = b.param(I32, origin);
        let second = b.param(I32, origin);
        for arg in [first, second] {
            b.effect(
                FuncBuilder::ENTRY,
                OpKind::Call {
                    callee: Callee::Direct {
                        item,
                        link_name: name,
                    },
                    function: None,
                    args: vec![arg],
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                origin,
            );
        }
        b.terminate(FuncBuilder::ENTRY, RTerm::Throw(None));
        let original = b.finish();
        original.verify(&interns.types).unwrap();
        let results = BTreeMap::from([(name, I32)]);
        let set = [
            (Value::int(1, 32, true), None),
            (Value::int(2, 32, true), None),
        ];
        let compare = |func| {
            super::check::compare_funcs(
                &original,
                func,
                &interns.types,
                &interns,
                &results,
                &set,
                100,
            )
        };
        assert_eq!(compare(&original), Verdict::Agree);
        let mut omitted = original.clone();
        omitted.blocks[0].ops.remove(0);
        let mut duplicated = original.clone();
        duplicated.blocks[0]
            .ops
            .insert(0, original.blocks[0].ops[0].clone());
        let mut reversed = original.clone();
        reversed.blocks[0].ops.swap(0, 1);
        for faulty in &[omitted, duplicated, reversed] {
            faulty.verify(&interns.types).unwrap();
            assert!(matches!(compare(faulty), Verdict::Mismatch(_)));
        }
    }

    /// `fn sum_to(n: i32) -> i32 { let mut i = 0; let mut acc = 0; while i < n { acc += i;
    /// i += 1 } acc }` — a loop with two carried locals.
    fn sum_body() -> Body {
        // _0 ret, _1 n, _2 i, _3 acc, _4 bool
        let locals = vec![
            local(0, LocalRole::Return, I32),
            local(1, LocalRole::Argument, I32),
            local(2, LocalRole::Temporary, I32),
            local(3, LocalRole::Temporary, I32),
            local(4, LocalRole::Temporary, BOOL),
        ];
        let entry = block(
            vec![
                assign(place(2, vec![], I32), Rvalue::Use(i32_const(0))),
                assign(place(3, vec![], I32), Rvalue::Use(i32_const(0))),
            ],
            Terminator::Goto(BlockId(1)),
        );
        let header = block(
            vec![assign(
                place(4, vec![], BOOL),
                Rvalue::Binary {
                    op: BinaryOp::Lt,
                    left: copy(2, I32),
                    right: copy(1, I32),
                },
            )],
            switch_bool(4, 2, 3),
        );
        let latch = block(
            vec![
                assign(
                    place(3, vec![], I32),
                    Rvalue::Binary {
                        op: BinaryOp::Add,
                        left: copy(3, I32),
                        right: copy(2, I32),
                    },
                ),
                assign(
                    place(2, vec![], I32),
                    Rvalue::Binary {
                        op: BinaryOp::Add,
                        left: copy(2, I32),
                        right: i32_const(1),
                    },
                ),
            ],
            Terminator::Goto(BlockId(1)),
        );
        let exit = block(
            vec![assign(place(0, vec![], I32), Rvalue::Use(copy(3, I32)))],
            Terminator::Return,
        );
        body(
            signature(vec![I32], I32),
            locals,
            vec![entry, header, latch, exit],
        )
    }

    #[test]
    fn a_loop_becomes_block_parameters_and_edge_copies() {
        let interns = world(true);
        let (func, plan) = check(&interns, &sum_body());
        // The header carries `i` and `acc` as parameters; the entry and the latch pass them.
        assert_eq!(func.blocks[1].params.len(), 2);
        assert!(plan.copies.contains_key(&(RBlockId(0), RBlockId(1))));
        assert!(plan.copies.contains_key(&(RBlockId(2), RBlockId(1))));
        // The condition is inline into the switch; the increments are inline into the copies.
        let cond = func.blocks[1].ops.last().and_then(|op| op.result).unwrap();
        assert!(plan.inline.contains(&cond));
        assert!(func.cells.is_empty());
    }

    /// `fn abs(x: i32) -> i32 { if x < 0 { -x } else { x } }`
    fn abs_body() -> Body {
        let locals = vec![
            local(0, LocalRole::Return, I32),
            local(1, LocalRole::Argument, I32),
            local(2, LocalRole::Temporary, BOOL),
        ];
        let entry = block(
            vec![assign(
                place(2, vec![], BOOL),
                Rvalue::Binary {
                    op: BinaryOp::Lt,
                    left: copy(1, I32),
                    right: i32_const(0),
                },
            )],
            switch_bool(2, 1, 2),
        );
        let neg = block(
            vec![assign(
                place(0, vec![], I32),
                Rvalue::Unary {
                    op: crate::jir::semantic::UnaryOp::Neg,
                    value: copy(1, I32),
                },
            )],
            Terminator::Goto(BlockId(3)),
        );
        let pos = block(
            vec![assign(place(0, vec![], I32), Rvalue::Use(copy(1, I32)))],
            Terminator::Goto(BlockId(3)),
        );
        let exit = block(vec![], Terminator::Return);
        body(
            signature(vec![I32], I32),
            locals,
            vec![entry, neg, pos, exit],
        )
    }

    #[test]
    fn a_merge_becomes_one_block_parameter() {
        let interns = world(true);
        let (func, _) = check(&interns, &abs_body());
        assert_eq!(func.blocks[3].params.len(), 1);
        assert!(matches!(func.blocks[3].term, RTerm::Return(Some(_))));
    }

    /// `fn f(a: i32) -> i32 { let mut p = Point { x: 1, y: 2 }; p.x = a; p.x + p.y }`
    fn field_write_body() -> Body {
        let x = |ty| Projection::Field { field: 0, ty };
        let y = |ty| Projection::Field { field: 1, ty };
        let locals = vec![
            local(0, LocalRole::Return, I32),
            local(1, LocalRole::Argument, I32),
            local(2, LocalRole::Temporary, POINT),
        ];
        let entry = block(
            vec![
                assign(
                    place(2, vec![], POINT),
                    Rvalue::Aggregate {
                        kind: AggregateKind::Adt {
                            definition: crate::jir::semantic::DefinitionId {
                                crate_id: 1,
                                local_hash: 11,
                            },
                            variant: 0,
                            active_field: None,
                        },
                        result: POINT,
                        operands: vec![i32_const(1), i32_const(2)],
                    },
                ),
                assign(place(2, vec![x(I32)], I32), Rvalue::Use(copy(1, I32))),
                assign(
                    place(0, vec![], I32),
                    Rvalue::Binary {
                        op: BinaryOp::Add,
                        left: copy_place(place(2, vec![x(I32)], I32)),
                        right: copy_place(place(2, vec![y(I32)], I32)),
                    },
                ),
            ],
            Terminator::Return,
        );
        body(signature(vec![I32], I32), locals, vec![entry])
    }

    #[test]
    fn a_field_write_mutates_the_reaching_value() {
        let interns = world(true);
        let (func, _) = check(&interns, &field_write_body());
        assert!(func.blocks[0]
            .ops
            .iter()
            .any(|op| matches!(op.kind, OpKind::SetField { .. })));
        assert!(func.cells.is_empty());
    }

    /// `fn f(a: Wrap) -> i32 { let mut b = a; b.0 = 99; a.0 }`, `Wrap` a transparent `i32`.
    fn transparent_write_body() -> Body {
        let inner = Projection::Field { field: 0, ty: I32 };
        let locals = vec![
            local(0, LocalRole::Return, I32),
            local(1, LocalRole::Argument, WRAP),
            local(2, LocalRole::Temporary, WRAP),
        ];
        let entry = block(
            vec![
                assign(place(2, vec![], WRAP), Rvalue::Use(copy(1, WRAP))),
                assign(place(2, vec![inner], I32), Rvalue::Use(i32_const(99))),
                assign(
                    place(0, vec![], I32),
                    Rvalue::Use(copy_place(place(1, vec![inner], I32))),
                ),
            ],
            Terminator::Return,
        );
        body(signature(vec![WRAP], I32), locals, vec![entry])
    }

    #[test]
    fn a_write_into_a_copy_that_shares_its_source_value_writes_a_value_of_its_own() {
        // A copy of a type that does not `needs_clone` is the source value itself, so `b` and
        // `a` reach one value; the write into `b.0` must not land in it, or `a.0` reads 99.
        let interns = world(true);
        let (func, _) = check(&interns, &transparent_write_body());
        let a = func.params[0];
        let written: Vec<ValueId> = func.blocks[0]
            .ops
            .iter()
            .filter_map(|op| match op.kind {
                OpKind::SetField { base, .. } => Some(base),
                _ => None,
            })
            .collect();
        assert_eq!(written.len(), 1);
        assert_ne!(written[0], a);
    }

    /// `fn f(s: &[i32], i: usize) -> i32 { s[i] }` with the bounds check.
    fn slice_body() -> Body {
        let locals = vec![
            local(0, LocalRole::Return, I32),
            local(1, LocalRole::Argument, REF_SLICE),
            local(2, LocalRole::Argument, USIZE),
            local(3, LocalRole::Temporary, USIZE),
            local(4, LocalRole::Temporary, BOOL),
        ];
        let entry = block(
            vec![
                assign(
                    place(3, vec![], USIZE),
                    Rvalue::Unary {
                        op: crate::jir::semantic::UnaryOp::PtrMetadata,
                        value: copy(1, REF_SLICE),
                    },
                ),
                assign(
                    place(4, vec![], BOOL),
                    Rvalue::Binary {
                        op: BinaryOp::Lt,
                        left: copy(2, USIZE),
                        right: copy(3, USIZE),
                    },
                ),
            ],
            Terminator::Assert {
                condition: copy(4, BOOL),
                expected: true,
                message: AssertMessage::BoundsCheck {
                    length: copy(3, USIZE),
                    index: copy(2, USIZE),
                },
                target: BlockId(1),
                unwind: Unwind::Continue,
                track_caller: None,
            },
        );
        let read = block(
            vec![assign(
                place(0, vec![], I32),
                Rvalue::Use(copy_place(place(
                    1,
                    vec![Projection::Deref, Projection::Index(LocalId(2))],
                    I32,
                ))),
            )],
            Terminator::Return,
        );
        body(
            signature(vec![REF_SLICE, USIZE], I32),
            locals,
            vec![entry, read],
        )
    }

    #[test]
    fn an_element_read_is_a_view_and_a_deref() {
        let interns = world(true);
        let (func, plan) = check(&interns, &slice_body());
        let view = func.blocks[1]
            .ops
            .iter()
            .find(|op| matches!(op.kind, OpKind::BorrowView { .. }))
            .expect("a view");
        // The table has `&mut i32`, so the view is typed with it and inline.
        assert_eq!(func.ty(view.result.unwrap()), Some(REF_I32));
        assert!(plan.inline.contains(&view.result.unwrap()));
    }

    #[test]
    fn a_view_without_a_reference_type_is_typed_with_the_pointee() {
        // Without `&mut i32` in the table the view carries the element type itself.
        let mut interns = world(false);
        interns.intern_type(reference(REF_SLICE, SLICE, PointerForm::FatSlice));
        let (func, _) = check(&interns, &slice_body());
        let view = func.blocks[1]
            .ops
            .iter()
            .find(|op| matches!(op.kind, OpKind::BorrowView { .. }))
            .expect("a view");
        assert_eq!(func.ty(view.result.unwrap()), Some(I32));
        assert_eq!(super::super::plan::view_pointee(&interns.types, I32), I32);
    }

    /// `fn f(a: i32) -> i32 { let mut x = a; let r = &mut x; *r = 5; x }`
    fn boxed_body() -> Body {
        let locals = vec![
            local(0, LocalRole::Return, I32),
            local(1, LocalRole::Argument, I32),
            local(2, LocalRole::Temporary, I32),
            local(3, LocalRole::Temporary, REF_I32),
        ];
        let entry = block(
            vec![
                assign(place(2, vec![], I32), Rvalue::Use(copy(1, I32))),
                assign(
                    place(3, vec![], REF_I32),
                    Rvalue::Reference {
                        kind: BorrowKind::Mutable,
                        place: place(2, vec![], I32),
                    },
                ),
                assign(
                    place(3, vec![Projection::Deref], I32),
                    Rvalue::Use(i32_const(5)),
                ),
                assign(place(0, vec![], I32), Rvalue::Use(copy(2, I32))),
            ],
            Terminator::Return,
        );
        body(signature(vec![I32], I32), locals, vec![entry])
    }

    #[test]
    fn an_address_taken_primitive_is_a_boxed_cell() {
        let interns = world(true);
        let (func, plan) = check(&interns, &boxed_body());
        assert_eq!(func.cells.len(), 1);
        assert!(func.cells[0].boxed);
        // Without origins the cell cannot be told apart from the parameter's own, and the
        // parameter is read nowhere else: the cell shares its name (`a = [a]`).
        assert_eq!(plan.param_cells.get(&CellId(0)), Some(&func.params[0]));
        assert_eq!(plan.cell_names[&CellId(0)], plan.names[&func.params[0]]);
        // The write through the reference is a `StoreThrough` on the borrowed cell.
        assert!(func.blocks[0]
            .ops
            .iter()
            .any(|op| matches!(op.kind, OpKind::StoreThrough { .. })));
    }

    /// A drop guarded by a drop flag: `let p; if c { p = Point{..}; flag = true }; if flag {
    /// drop(p) }` — `p` has no definition on one path into the drop.
    fn drop_flag_body() -> Body {
        let locals = vec![
            local(0, LocalRole::Return, I32),
            local(1, LocalRole::Argument, BOOL),
            local(2, LocalRole::Temporary, POINT),
            local(3, LocalRole::Temporary, BOOL),
        ];
        let entry = block(
            vec![assign(
                place(3, vec![], BOOL),
                Rvalue::Use(Operand {
                    ty: BOOL,
                    kind: OperandKind::Constant(Constant::Scalar { bits: 0, bytes: 1 }),
                }),
            )],
            switch_bool(1, 1, 2),
        );
        let init = block(
            vec![
                assign(
                    place(2, vec![], POINT),
                    Rvalue::Aggregate {
                        kind: AggregateKind::Adt {
                            definition: crate::jir::semantic::DefinitionId {
                                crate_id: 1,
                                local_hash: 11,
                            },
                            variant: 0,
                            active_field: None,
                        },
                        result: POINT,
                        operands: vec![i32_const(1), i32_const(2)],
                    },
                ),
                assign(
                    place(3, vec![], BOOL),
                    Rvalue::Use(Operand {
                        ty: BOOL,
                        kind: OperandKind::Constant(Constant::Scalar { bits: 1, bytes: 1 }),
                    }),
                ),
            ],
            Terminator::Goto(BlockId(2)),
        );
        let merge = block(
            vec![assign(place(0, vec![], I32), Rvalue::Use(i32_const(7)))],
            switch_bool(3, 3, 4),
        );
        let drop = block(
            vec![],
            Terminator::Drop {
                place: place(2, vec![], POINT),
                target: BlockId(4),
                unwind: Unwind::Continue,
                replace: false,
                async_drop: None,
                glue: DropGlue::Direct {
                    item: crate::jir::semantic::ItemId {
                        definition: crate::jir::semantic::DefinitionId {
                            crate_id: 1,
                            local_hash: 99,
                        },
                        instantiation: 99,
                    },
                    link_name: StrId(0),
                },
            },
        );
        let exit = block(vec![], Terminator::Return);
        body(
            signature(vec![BOOL], I32),
            locals,
            vec![entry, init, merge, drop, exit],
        )
    }

    #[test]
    fn a_local_read_without_a_definition_is_demoted_to_a_cell() {
        let interns = world(true);
        let (func, _) = check(&interns, &drop_flag_body());
        assert_eq!(func.cells.len(), 1);
        assert!(!func.cells[0].boxed);
        assert!(func.blocks[3].ops.iter().any(|op| matches!(
            op.kind,
            OpKind::Drop {
                root: Root::Cell(_),
                ..
            }
        )));
    }

    /// `fn f(o: Option<i32>) -> i32 { match o { Some(v) => v, None => 0 } }`: a tag switch.
    fn option_body() -> Body {
        let locals = vec![
            local(0, LocalRole::Return, I32),
            local(1, LocalRole::Argument, OPTION),
            local(2, LocalRole::Temporary, I32),
        ];
        let entry = block(
            vec![assign(
                place(2, vec![], I32),
                Rvalue::Discriminant(place(1, vec![], OPTION)),
            )],
            Terminator::Switch {
                discriminant: mv(2, I32),
                targets: vec![(1, BlockId(1))],
                otherwise: BlockId(2),
            },
        );
        let some = block(
            vec![assign(
                place(0, vec![], I32),
                Rvalue::Use(copy_place(place(
                    1,
                    vec![
                        Projection::Downcast { variant: 1 },
                        Projection::Field { field: 0, ty: I32 },
                    ],
                    I32,
                ))),
            )],
            Terminator::Return,
        );
        let none = block(
            vec![assign(place(0, vec![], I32), Rvalue::Use(i32_const(0)))],
            Terminator::Return,
        );
        body(
            signature(vec![OPTION], I32),
            locals,
            vec![entry, some, none],
        )
    }

    #[test]
    fn a_discriminant_only_switched_on_reads_the_tag() {
        let interns = world(true);
        let (func, plan) = check(&interns, &option_body());
        let discr = func.blocks[0].ops[0].result.unwrap();
        assert!(plan.tag_values.contains(&discr));
        assert!(matches!(
            plan.scrutinees[&RBlockId(0)],
            super::super::plan::RScrut::Value {
                tag_enum: Some(OPTION),
                ..
            }
        ));
        assert!(plan.inline.contains(&discr));
    }

    #[test]
    fn a_static_body_declines() {
        let interns = world(true);
        let mut static_body = abs_body();
        static_body.kind = BodyKind::Static {
            init: ConstTree::Unit,
            mutable: false,
        };
        static_body.locals.clear();
        static_body.blocks.clear();
        assert!(build(&static_body, None, &interns.types, &interns).is_err());
    }

    /// `fn take(p: &mut Point) -> i32 { let v = (*p).x; (*p).x = 0; v }`: the read must not
    /// move past the store that follows it (`Option::take`'s shape).
    #[test]
    fn a_read_is_not_inlined_past_a_later_store() {
        let interns = world(true);
        let x = |ty| Projection::Field { field: 0, ty };
        let locals = vec![
            local(0, LocalRole::Return, I32),
            local(1, LocalRole::Argument, REF_POINT),
            local(2, LocalRole::Temporary, I32),
        ];
        let entry = block(
            vec![
                assign(
                    place(2, vec![], I32),
                    Rvalue::Use(copy_place(place(1, vec![Projection::Deref, x(I32)], I32))),
                ),
                assign(
                    place(1, vec![Projection::Deref, x(I32)], I32),
                    Rvalue::Use(i32_const(0)),
                ),
                assign(place(0, vec![], I32), Rvalue::Use(copy(2, I32))),
            ],
            Terminator::Return,
        );
        let body = body(signature(vec![REF_POINT], I32), locals, vec![entry]);
        let (func, plan) = check(&interns, &body);
        let RTerm::Return(Some(returned)) = func.blocks[0].term else {
            panic!("returns a value");
        };
        assert!(!plan.inline.contains(&returned));
        assert!(plan.declared_order.contains(&returned));
    }

    /// `fn g(p: &mut Point) -> i32 { let v = (*p).x; h(); v }`: a read queued before a call is
    /// flushed before the call (v1 `queue.rs`), so `v` is declared, not inlined into the return.
    #[test]
    fn a_read_is_not_inlined_past_a_call() {
        let interns = world(true);
        let x = |ty| Projection::Field { field: 0, ty };
        let locals = vec![
            local(0, LocalRole::Return, I32),
            local(1, LocalRole::Argument, REF_POINT),
            local(2, LocalRole::Temporary, I32),
            local(3, LocalRole::Temporary, UNIT),
        ];
        let entry = block(
            vec![assign(
                place(2, vec![], I32),
                Rvalue::Use(copy_place(place(1, vec![Projection::Deref, x(I32)], I32))),
            )],
            Terminator::Call(Call {
                function: Operand {
                    ty: I32,
                    kind: OperandKind::Constant(Constant::ZeroSized),
                },
                callee: Callee::Direct {
                    item: crate::jir::semantic::ItemId {
                        definition: crate::jir::semantic::DefinitionId {
                            crate_id: 1,
                            local_hash: 51,
                        },
                        instantiation: 51,
                    },
                    link_name: StrId(0),
                },
                semantic: crate::jir::semantic::CallSemantic::Ordinary,
                arguments: vec![],
                destination: place(3, vec![], UNIT),
                target: Some(BlockId(1)),
                unwind: Unwind::Continue,
                track_caller: None,
                host: None,
            }),
        );
        let rest = block(
            vec![assign(place(0, vec![], I32), Rvalue::Use(copy(2, I32)))],
            Terminator::Return,
        );
        let body = body(signature(vec![REF_POINT], I32), locals, vec![entry, rest]);
        let func = build(&body, None, &interns.types, &interns).expect("builds");
        func.verify(&interns.types).expect("verifies");
        let plan = fuse::plan(&func, None, &interns.types, &interns, &Opts::default());
        let RTerm::Return(Some(returned)) = func.blocks[1].term else {
            panic!("returns a value");
        };
        assert!(!plan.inline.contains(&returned));
        assert!(plan.declared_order.contains(&returned));
        let mut results = BTreeMap::new();
        results.insert(StrId(0), UNIT);
        for set in &argument_sets(&[REF_POINT], &interns.types).unwrap() {
            assert_eq!(
                compare(&body, &func, &interns.types, &interns, &results, set, 1000),
                Verdict::Agree
            );
        }
    }

    /// A `SwitchInt` on `ub_checks` (a `RuntimeCheck(false)` operand): folded, the check's arm
    /// is unreachable and its block stays empty; unfolded, it is an ordinary switch.
    #[test]
    fn a_runtime_check_switch_folds_when_asked() {
        let interns = world(true);
        let locals = vec![
            local(0, LocalRole::Return, I32),
            local(1, LocalRole::Argument, I32),
        ];
        let entry = block(
            vec![],
            Terminator::Switch {
                discriminant: Operand {
                    ty: BOOL,
                    kind: OperandKind::RuntimeCheck(false),
                },
                targets: vec![(0, BlockId(2))],
                otherwise: BlockId(1),
            },
        );
        let checked = block(
            vec![assign(place(0, vec![], I32), Rvalue::Use(i32_const(-1)))],
            Terminator::Return,
        );
        let fast = block(
            vec![assign(place(0, vec![], I32), Rvalue::Use(copy(1, I32)))],
            Terminator::Return,
        );
        let body = body(
            signature(vec![I32], I32),
            locals,
            vec![entry, checked, fast],
        );
        let folded = build_with(&body, None, &interns.types, &interns, true).expect("builds");
        folded.verify(&interns.types).expect("verifies");
        assert!(matches!(folded.blocks[0].term, RTerm::Goto(RBlockId(2), _)));
        assert!(folded.blocks[1].ops.is_empty());
        assert!(matches!(folded.blocks[1].term, RTerm::Unreachable));
        let (unfolded, _) = check(&interns, &body);
        assert!(matches!(unfolded.blocks[0].term, RTerm::Switch { .. }));
        for set in &argument_sets(&[I32], &interns.types).unwrap() {
            assert_eq!(
                compare(
                    &body,
                    &folded,
                    &interns.types,
                    &interns,
                    &BTreeMap::new(),
                    set,
                    1000
                ),
                Verdict::Agree
            );
        }
    }

    #[test]
    fn a_direct_call_result_flows_to_its_destination() {
        // `fn f(a: i32) -> i32 { g(a) + 1 }` with `g` stubbed to return zero.
        let interns = world(true);
        let locals = vec![
            local(0, LocalRole::Return, I32),
            local(1, LocalRole::Argument, I32),
            local(2, LocalRole::Temporary, I32),
        ];
        let callee = DirectItemRef {
            item: crate::jir::semantic::ItemId {
                definition: crate::jir::semantic::DefinitionId {
                    crate_id: 1,
                    local_hash: 50,
                },
                instantiation: 50,
            },
            link_name: StrId(0),
        };
        let entry = block(
            vec![],
            Terminator::Call(Call {
                function: Operand {
                    ty: I32,
                    kind: OperandKind::Constant(Constant::ZeroSized),
                },
                callee: Callee::Direct {
                    item: callee.item,
                    link_name: callee.link_name,
                },
                semantic: crate::jir::semantic::CallSemantic::Ordinary,
                arguments: vec![copy(1, I32)],
                destination: place(2, vec![], I32),
                target: Some(BlockId(1)),
                unwind: Unwind::Continue,
                track_caller: None,
                host: None,
            }),
        );
        let rest = block(
            vec![assign(
                place(0, vec![], I32),
                Rvalue::Binary {
                    op: BinaryOp::Add,
                    left: mv(2, I32),
                    right: i32_const(1),
                },
            )],
            Terminator::Return,
        );
        let body = body(signature(vec![I32], I32), locals, vec![entry, rest]);
        let func = build(&body, None, &interns.types, &interns).expect("builds");
        func.verify(&interns.types).expect("verifies");
        // The call's result is defined at the end of the entry and used in the next block, a
        // straight-line chain: inline, as v1's queue carried it into the next statement.
        let plan = fuse::plan(&func, None, &interns.types, &interns, &Opts::default());
        let call = func.blocks[0]
            .ops
            .iter()
            .find(|op| matches!(op.kind, OpKind::Call { .. }))
            .and_then(|op| op.result)
            .expect("a call result");
        assert!(plan.inline.contains(&call));
        let mut results = BTreeMap::new();
        results.insert(StrId(0), I32);
        let sets = argument_sets(&[I32], &interns.types).unwrap();
        for set in &sets {
            assert_eq!(
                compare(&body, &func, &interns.types, &interns, &results, set, 1000),
                Verdict::Agree
            );
        }
        let _ = Value::Unit;
    }
}

/// The fixture corpus: every format 20 object under `build/` (the suites' `.o` files), and,
/// when `RCGJS_SSA_FIXTURES=core`, the `libcore` rlibs of the built sysroots too. Every body is
/// built, verified, planned, structured, and — where the interpreters can run it — evaluated
/// against the JIR-S interpreter on generated arguments.
#[cfg(test)]
mod fixtures {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use super::check::{argument_sets, compare, Verdict};
    use crate::jir::interns::StrId;
    use crate::jir::semantic::{BodyKind, TypeId};
    use crate::lower::{fuse, structure};
    use crate::opts::Opts;
    use rcgjs_object::object;

    #[derive(Default)]
    struct Counts {
        objects: usize,
        bodies: usize,
        built: usize,
        declined: BTreeMap<String, usize>,
        planned: usize,
        structured: usize,
        structure_declined: usize,
        interpreted_bodies: usize,
        agreed_sets: usize,
        skipped: BTreeMap<String, usize>,
        no_arguments: usize,
        mismatches: Vec<String>,
        cells: usize,
        boxed: usize,
        params: usize,
        values: usize,
        inline: usize,
    }

    fn walk(dir: &Path, out: &mut Vec<PathBuf>, core: bool) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        entries.sort();
        for path in entries {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            if path.is_dir() {
                if matches!(
                    name.as_deref(),
                    Some("hostmacro-discover" | "sysroot-cache" | "target")
                ) {
                    continue;
                }
                if name.as_deref() == Some("sysroots") && !core {
                    continue;
                }
                walk(&path, out, core);
            } else if path.extension().is_some_and(|ext| ext == "o")
                || (core
                    && name.as_deref().is_some_and(|name| {
                        name.starts_with("libcore-") && name.ends_with(".rlib")
                    }))
            {
                out.push(path);
            }
        }
    }

    fn members(path: &Path) -> Vec<(String, Vec<u8>)> {
        let Ok(bytes) = std::fs::read(path) else {
            return Vec::new();
        };
        if !bytes.starts_with(b"!<arch>\n") {
            return vec![(path.display().to_string(), bytes)];
        }
        let Ok(archive) = ::object::read::archive::ArchiveFile::parse(&*bytes) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for member in archive.members().flatten() {
            if let Ok(data) = member.data(&*bytes) {
                out.push((
                    format!(
                        "{}({})",
                        path.display(),
                        String::from_utf8_lossy(member.name())
                    ),
                    data.to_vec(),
                ));
            }
        }
        out
    }

    fn check_object(label: &str, bytes: &[u8], counts: &mut Counts, seen: &mut BTreeMap<u128, ()>) {
        if !object::is_object(bytes) {
            return;
        }
        let Ok(Some(object)) = object::read(bytes) else {
            return;
        };
        counts.objects += 1;
        let table = &object.interns.types;
        let interns = &object.interns;
        // Decode every body once: the result types of the object's own items stub the calls.
        let mut decoded = Vec::new();
        let mut results: BTreeMap<StrId, TypeId> = BTreeMap::new();
        for item in &object.summaries {
            let Ok(Some((body, origins))) = object.decode_verified(item) else {
                continue;
            };
            if !matches!(body.kind, BodyKind::Function) {
                continue;
            }
            let link_name = interns
                .strings
                .iter()
                .position(|s| s == &item.key.link_name)
                .map(|index| StrId(index as u32));
            if let Some(link_name) = link_name {
                results.insert(link_name, body.signature.result);
            }
            decoded.push((item.key.link_name.clone(), body, origins));
        }
        for (name, body, origins) in decoded {
            // The same item shows up in several objects; count it once.
            let key = crate::jir::summary::semantic_hash(&body, interns);
            let key = u128::from(key.lo) ^ (u128::from(key.hi) << 64);
            if seen.insert(key, ()).is_some() {
                continue;
            }
            counts.bodies += 1;
            let func = match super::build(&body, Some(&origins), table, interns) {
                Ok(func) => func,
                Err(decline) => {
                    let detail = match &decline {
                        crate::jir::decline::Decline::Lower(
                            crate::jir::decline::LowerDecline::Other { detail, .. },
                        ) => detail.clone(),
                        other => other.to_string(),
                    };
                    // Group verifier messages by their leading words.
                    let key: String = detail
                        .split(':')
                        .next()
                        .unwrap_or("")
                        .split_whitespace()
                        .take(8)
                        .collect::<Vec<_>>()
                        .join(" ");
                    *counts.declined.entry(key).or_insert(0) += 1;
                    if std::env::var("RCGJS_SSA_VERBOSE").is_ok() {
                        eprintln!("declined {label} {name}: {detail}");
                        if detail.contains("takes the address") {
                            for block in &body.blocks {
                                for statement in &block.statements {
                                    if let crate::jir::semantic::StatementKind::Assign {
                                        destination,
                                        value:
                                            crate::jir::semantic::Rvalue::Reference { place, .. }
                                            | crate::jir::semantic::Rvalue::RawPointer { place, .. },
                                    } = &statement.kind
                                    {
                                        if !table.is_indirect(place.ty) {
                                            eprintln!(
                                                "    {:?} = &{:?} : {:?} (indirect {})",
                                                destination.local,
                                                place,
                                                table.get(place.ty).map(|d| &d.kind),
                                                table.is_indirect(place.ty),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }
            };
            counts.built += 1;
            counts.cells += func.cells.len();
            counts.boxed += func.cells.iter().filter(|cell| cell.boxed).count();
            counts.values += func.values.len();
            counts.params += func
                .blocks
                .iter()
                .skip(1)
                .map(|block| block.params.len())
                .sum::<usize>();
            let plan = fuse::plan(&func, Some(&origins), table, interns, &Opts::default());
            counts.planned += 1;
            counts.inline += plan.inline.len();
            match structure::structure(&func, &plan, table, false) {
                Ok(_) => counts.structured += 1,
                Err(_) => counts.structure_declined += 1,
            }
            let Some(sets) = argument_sets(&body.signature.arguments, table) else {
                counts.no_arguments += 1;
                continue;
            };
            let mut ran = false;
            for set in &sets {
                match compare(&body, &func, table, interns, &results, set, 20_000) {
                    Verdict::Agree => {
                        ran = true;
                        counts.agreed_sets += 1;
                    }
                    Verdict::Skipped(why) => {
                        *counts.skipped.entry(why).or_insert(0) += 1;
                    }
                    Verdict::Mismatch(why) => {
                        counts.mismatches.push(format!("{label} {name}: {why}"));
                    }
                }
            }
            if ran {
                counts.interpreted_bodies += 1;
            }
        }
    }

    #[test]
    fn fixture_bodies_build_verify_and_agree_with_the_semantic_interpreter() {
        // A deep body recurses through the merge points on its way up; the harness thread's
        // default stack is small.
        std::thread::Builder::new()
            .stack_size(512 << 20)
            .spawn(run_fixtures)
            .expect("spawn")
            .join()
            .expect("fixture thread");
    }

    fn run_fixtures() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../build");
        let core = std::env::var("RCGJS_SSA_FIXTURES").is_ok_and(|v| v == "core");
        let mut paths = Vec::new();
        walk(&root, &mut paths, core);
        let mut counts = Counts::default();
        let mut seen = BTreeMap::new();
        for path in &paths {
            for (label, bytes) in members(path) {
                check_object(&label, &bytes, &mut counts, &mut seen);
            }
        }
        eprintln!(
            "ssa fixtures: {} objects, {} distinct bodies, {} built ({} declined), {} planned, \
             {} structured ({} declined), {} bodies interpreted, {} argument sets agreed, \
             {} bodies without argument samples, {} mismatches; {} values, {} block params, \
             {} inline, {} cells ({} boxed)",
            counts.objects,
            counts.bodies,
            counts.built,
            counts.bodies - counts.built,
            counts.planned,
            counts.structured,
            counts.structure_declined,
            counts.interpreted_bodies,
            counts.agreed_sets,
            counts.no_arguments,
            counts.mismatches.len(),
            counts.values,
            counts.params,
            counts.inline,
            counts.cells,
            counts.boxed,
        );
        let mut declined: Vec<_> = counts.declined.iter().collect();
        declined.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (reason, count) in declined {
            eprintln!("  declined {count:>5}  {reason}");
        }
        let mut skipped: Vec<_> = counts.skipped.iter().collect();
        skipped.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (reason, count) in skipped.iter().take(20) {
            eprintln!("  skipped  {count:>5}  {reason}");
        }
        for mismatch in counts.mismatches.iter().take(20) {
            eprintln!("  MISMATCH {mismatch}");
        }
        super::check::require_fixtures(counts.bodies, "ssa fixtures");
        assert!(
            counts.mismatches.is_empty(),
            "{} dual-interpreter mismatches",
            counts.mismatches.len()
        );
    }
}
