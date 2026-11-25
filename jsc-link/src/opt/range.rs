//! Bounds-check elision (D8): a forward interval analysis with symbolic length facts deletes an
//! `Assert` bounds check whose condition a dominating fact implies.
//!
//! Three ingredients, all per [`Func`]:
//!
//! * **Value numbering** over the pure ops (`Const` scalars, arithmetic, `PtrMetadata`, copies,
//!   whole-slice reborrows), so the length the guard compared against and the length the assert
//!   re-reads count as one term. The lens rules: `PtrMetadata` of a `SliceView` is that view's
//!   length operand, and a `BorrowView` reborrow of a whole slice is the slice itself.
//! * **Unsigned intervals** per value, to a fixpoint over the block parameters: constants are
//!   points, arithmetic transfers where it cannot wrap, and the metadata of a slice of non-ZST
//!   elements is bounded by `isize::MAX` — the certificate that lets an index's `+ k` peel without
//!   a wrap. Back-edge arguments are refined by the facts that hold at the edge, with a widening
//!   round so a counted loop converges.
//! * **Facts**: a `Switch` whose one edge into a block decides a comparison, and every earlier
//!   `Assert`, each normalized to `x < y` or `x <= y`.
//!
//! An assert `index < length` is deleted when either the intervals alone prove it, or a
//! dominating fact `base + k' </<= length` covers the assert's peeled `base + k` — and, per the
//! phase's ruling, no op that may observe or write memory sits on any fact-free path between the
//! fact and the assert (`Purity::Effect`, asserts themselves excepted). The pass only removes
//! `Assert` ops: no value or block is ever renumbered, so the recorded
//! [`Witness::BoundsRemoved`] evidence stays checkable against the final function.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use crate::analysis::arena::{
    find_counted_push_loop_len, ArenaFieldKind, ArenaHandle, ProgramBoundsFacts, ReturnVecLen,
};
use crate::analysis::heap::{
    global_vec_elem_ty, is_direct_global_vec_ty, is_global_vec_ty, is_vec_len_field,
};
use crate::jir::interns::Interns;
use crate::jir::repr::{
    CellId, Dominators, Func, Op, OpKind, Proj, RAssertMessage, RBlockId, RTerm, Root, ValueId,
};
use crate::jir::semantic::{
    BinaryOp, CallSemantic, Callee, CastKind, ConstTree, DefinitionId, DropGlue, GlobalReference,
    ItemId, StandardOperation, TypeId, UnaryOp,
};
use crate::jir::types::TypeKind;
use crate::jir::TypeTable;
use crate::lower::fuse::{op_purity, Purity};
use crate::lower::js::value;

use super::witness::{ArenaIndexSource, BoundsFact, LocalVecRoot, Witness};

thread_local! {
    static ACTIVE_PROGRAM_FACTS: RefCell<Vec<Arc<ProgramBoundsFacts>>> = const { RefCell::new(Vec::new()) };
}

/// RAII scope installing whole-program [`ProgramBoundsFacts`] for `opt/pipeline.rs` without
/// changing the signature of `opt::optimize`.
pub struct ProgramFactsGuard {
    _private: (),
}

impl Drop for ProgramFactsGuard {
    fn drop(&mut self) {
        ACTIVE_PROGRAM_FACTS.with(|stack| {
            let _ = stack.borrow_mut().pop();
        });
    }
}

/// Installs `facts` on the current thread until the returned guard is dropped.
pub fn enter_program_facts(facts: Arc<ProgramBoundsFacts>) -> ProgramFactsGuard {
    ACTIVE_PROGRAM_FACTS.with(|stack| {
        stack.borrow_mut().push(facts);
    });
    ProgramFactsGuard { _private: () }
}

/// Runs `f` with the active thread-local [`ProgramBoundsFacts`], if any.
pub fn with_current_program_facts<R>(f: impl FnOnce(Option<&ProgramBoundsFacts>) -> R) -> R {
    let active = ACTIVE_PROGRAM_FACTS.with(|stack| stack.borrow().last().cloned());
    f(active.as_deref())
}

/// What the narrow range pass proved in one function.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RangeStats {
    /// Bounds-check asserts removed.
    pub asserts_removed: u32,
    /// Slice-prefix comparisons folded from `len - len % chunk` facts.
    pub comparisons_folded: u32,
}

impl RangeStats {
    /// Whether the pass changed the function.
    pub fn changed(self) -> bool {
        self.asserts_removed > 0 || self.comparisons_folded > 0
    }
}

/// Runs the pass over `func` in place, without interned names: every sysroot runtime shim with the
/// right shape is trusted. Production uses [`run_with_interns`]; this form serves the unit and
/// corpus tests, whose functions carry no intern table.
#[cfg(test)]
pub fn run(func: &mut Func, table: &TypeTable, witnesses: &mut Vec<Witness>) -> RangeStats {
    run_impl(func, table, None, None, witnesses)
}

/// Test helper running the pass with explicit [`ProgramBoundsFacts`].
#[cfg(test)]
pub fn run_with_facts(
    func: &mut Func,
    table: &TypeTable,
    arena_facts: &ProgramBoundsFacts,
    witnesses: &mut Vec<Witness>,
) -> RangeStats {
    run_impl(func, table, None, Some(arena_facts), witnesses)
}

/// Runs the pass over `func` in place with `interns` for exact sysroot runtime shim matching.
#[allow(dead_code)]
pub fn run_with_interns(
    func: &mut Func,
    table: &TypeTable,
    interns: &Interns,
    witnesses: &mut Vec<Witness>,
) -> RangeStats {
    run_impl(func, table, Some(interns), None, witnesses)
}

/// Runs the pass over `func` in place with `interns` and optional whole-program
/// [`ProgramBoundsFacts`].
pub fn run_with_interns_and_facts(
    func: &mut Func,
    table: &TypeTable,
    interns: &Interns,
    arena_facts: Option<&ProgramBoundsFacts>,
    witnesses: &mut Vec<Witness>,
) -> RangeStats {
    run_impl(func, table, Some(interns), arena_facts, witnesses)
}

fn run_impl(
    func: &mut Func,
    table: &TypeTable,
    interns: Option<&Interns>,
    arena_facts: Option<&ProgramBoundsFacts>,
    witnesses: &mut Vec<Witness>,
) -> RangeStats {
    let any = func.blocks.iter().any(|block| {
        block.ops.iter().any(|op| {
            matches!(
                op.kind,
                OpKind::Assert {
                    expected: true,
                    msg: RAssertMessage::BoundsCheck { .. },
                    ..
                }
            )
        })
    });
    let aligned_candidate = {
        let mut remainder = false;
        let mut subtract = false;
        let mut compare = false;
        for op in func.blocks.iter().flat_map(|block| &block.ops) {
            match &op.kind {
                OpKind::Binary {
                    op: BinaryOp::Rem, ..
                } => remainder = true,
                OpKind::Binary {
                    op: BinaryOp::Sub | BinaryOp::SubUnchecked,
                    ..
                } => subtract = true,
                OpKind::Binary {
                    op: BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge,
                    ..
                } => compare = true,
                _ => {}
            }
        }
        remainder && subtract && compare
    };
    if !any && !aligned_candidate {
        return RangeStats::default();
    }
    let analysis = Analysis::of(func, table, interns, arena_facts);
    let mut stats = RangeStats::default();

    // A fixed-size chunk prefix is the largest multiple of `chunk` no greater than the slice
    // length. MIR spells the subslice legality as ordinary comparison branches rather than
    // `Assert`, so fold just those comparisons here and let the trailing DCE remove their dead
    // panic blocks and formatting dependencies.
    let mut folded = Vec::new();
    for block in &func.blocks {
        for (index, op) in block.ops.iter().enumerate() {
            let Some(result) = op.result else { continue };
            let Some((left, right, binary, value)) =
                analysis.aligned_prefix_comparison(func, table, &op.kind)
            else {
                continue;
            };
            folded.push((block.id.0 as usize, index, result, value));
            witnesses.push(Witness::AlignedPrefix {
                left,
                right,
                op: binary,
                result: value,
            });
        }
    }
    for (block, index, _result, value) in folded {
        func.blocks[block].ops[index].kind = OpKind::Const(ConstTree::Scalar {
            bits: u128::from(value),
            bytes: 1,
        });
        stats.comparisons_folded += 1;
    }

    if !any {
        return stats;
    }

    let mut deleted: BTreeSet<(u32, usize)> = BTreeSet::new();
    for &block in &analysis.doms.rpo {
        let b = block.0 as usize;
        for index in 0..func.blocks[b].ops.len() {
            let OpKind::Assert {
                cond,
                expected: true,
                msg: RAssertMessage::BoundsCheck { length, index: idx },
                ..
            } = &func.blocks[b].ops[index].kind
            else {
                continue;
            };
            let Some(fact) = analysis.prove(
                func,
                table,
                interns,
                arena_facts,
                *cond,
                block,
                index,
                &deleted,
            ) else {
                continue;
            };
            witnesses.push(Witness::BoundsRemoved {
                block,
                index: *idx,
                length: *length,
                fact,
            });
            deleted.insert((block.0, index));
        }
    }
    if deleted.is_empty() {
        return stats;
    }

    for block in &mut func.blocks {
        let b = block.id.0;
        if !deleted.iter().any(|(db, _)| *db == b) {
            continue;
        }
        block.ops = std::mem::take(&mut block.ops)
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !deleted.contains(&(b, *index)))
            .map(|(_, op)| op)
            .collect();
    }
    stats.asserts_removed = deleted.len() as u32;
    stats
}

/// An inclusive unsigned interval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Iv {
    lo: u128,
    hi: u128,
}

impl Iv {
    fn point(value: u128) -> Self {
        Self {
            lo: value,
            hi: value,
        }
    }

    fn full(max: u128) -> Self {
        Self { lo: 0, hi: max }
    }

    fn join(self, other: Self) -> Self {
        Self {
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }
}

/// The largest value of an unsigned integer of `bits` bits.
fn width_max(bits: u32) -> u128 {
    if bits >= 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    }
}

/// `bits` truncated to `bytes` bytes; 16 or more is the identity.
fn mask(bits: u128, bytes: u8) -> u128 {
    match bytes {
        0 => 0,
        1..=15 => bits & ((1u128 << (bytes as u32 * 8)) - 1),
        _ => bits,
    }
}

/// A comparison relation a fact carries, always spelled `x rel y`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Rel {
    Lt,
    Le,
}

/// Where a fact is established.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Site {
    /// Taking the one edge into `target` decided the comparison `cond`.
    Edge { target: RBlockId, cond: ValueId },
    /// The assert at `block.ops[index]` passed.
    Assert {
        block: RBlockId,
        index: usize,
        cond: ValueId,
    },
}

/// One side of a recorded fact: either a dynamic SSA value or a switch-matched integer constant.
#[derive(Clone, Copy, Debug)]
enum Bound {
    Value(ValueId),
    Const(u128),
}

/// One known-true comparison: `x rel y`.
#[derive(Clone, Copy, Debug)]
struct Fact {
    x: Bound,
    rel: Rel,
    y: Bound,
    site: Site,
}

/// The two flood-fill directions of [`Analysis::flood`].
#[derive(Clone, Copy)]
enum Direction {
    Forward(RBlockId),
    Backward(RBlockId),
}

/// One known-true disequality: `v != c`, established at `site`.
#[derive(Clone, Copy, Debug)]
struct NeFact {
    v: ValueId,
    c: u128,
    site: Site,
}

/// Downward-monotone loop parameter `param` bounded by `len_canon` (`strict`: `<`, else `<=`),
/// certified by dominating `(fact_block, fact_cond)`.
#[derive(Clone, Copy, Debug)]
struct DownwardBound {
    param: ValueId,
    header: RBlockId,
    len_canon: ValueId,
    strict: bool,
    fact_block: RBlockId,
    fact_cond: ValueId,
}

/// Element bound kind for a local non-escaping `Vec<u32|usize>` cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalElemBound {
    /// Every element `x < len_canon`.
    Lt,
    /// Every element `x <= len_canon` (so when `x != 0`, `x - 1 < len_canon`).
    Le,
    /// Every element `x <= len_canon + 1` (so when `x >= 2`, `x - 2 < len_canon`).
    LePlusOne,
}

#[derive(Clone, Copy, Debug)]
struct LocalBoundedVecFact {
    vec_root: LocalVecRoot,
    len_canon: ValueId,
    bound: LocalElemBound,
}

/// Provenance of an arena index value inside the current function.
#[derive(Clone, Copy, Debug)]
struct ArenaValOrigin {
    elem_def: DefinitionId,
    handle: ArenaHandle,
    sentinel: Option<u128>,
    source: ArenaIndexSource,
}

/// Identifies the storage root of a `Vec<T, Global>` for same-Vec length unification.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum VecRootKey {
    Cell(CellId),
    Value(ValueId),
    Param(usize),
    ParamField(usize, u32),
    CellField(CellId, u32),
}

/// Everything the proofs read, computed once per function.
struct Analysis {
    doms: Dominators,
    /// The value-numbering class representative per value.
    canon: Vec<ValueId>,
    /// The defining op per value, as (block index, op index).
    defs: Vec<Option<(u32, usize)>>,
    /// The interval per value; `None` for non-unsigned-integer values and unreachable code.
    intervals: Vec<Option<Iv>>,
    /// The unsigned bit width per value.
    widths: Vec<Option<u32>>,
    facts: Vec<Fact>,
    /// Fact indices valid on entry to each block (dominating edges and asserts).
    entry_facts: Vec<Vec<usize>>,
    /// Known-true `v != c` disequality facts.
    ne_facts: Vec<NeFact>,
    /// `ne_facts` indices valid on entry to each block.
    entry_ne_facts: Vec<Vec<usize>>,
    /// Predecessor blocks, deduplicated.
    preds: Vec<Vec<u32>>,
    /// Exact constant length for a canonical length/vec/slice term (e.g. `NativeVecFromArray`,
    /// `VecNew`, `ReturnVecLen::Const`, `StaticSlice`).
    vec_const_len: BTreeMap<ValueId, u128>,
    /// Exact `1 << shift_canon` length for a canonical length/vec/slice term (e.g. `fflate::h_map`).
    vec_one_shl_len: BTreeMap<ValueId, (ValueId, u32)>,
    /// Downward-monotone loop parameters bounded against a canonical length.
    downward_bounds: Vec<DownwardBound>,
    /// Local non-escaping bounded-element `Vec` facts and `Deref` loads referencing them.
    local_bounded_vecs: Vec<LocalBoundedVecFact>,
    load_to_local_vec: BTreeMap<ValueId, usize>,
    /// Mapping from canonical length value -> `(elem_def, ArenaHandle)` for proven arenas.
    arena_len_handle: BTreeMap<ValueId, Vec<(DefinitionId, ArenaHandle)>>,
    /// Mapping from canonical value -> proven arena index provenance.
    arena_val_origin: BTreeMap<ValueId, ArenaValOrigin>,
    /// Elements loaded from a `ReturnVecElemBound`-certified `Vec`: `load_canon -> (len_canon, callee)`.
    ret_vec_elem_len: BTreeMap<ValueId, (ValueId, ItemId)>,
}

impl Analysis {
    fn of(
        func: &Func,
        table: &TypeTable,
        interns: Option<&Interns>,
        arena_facts: Option<&ProgramBoundsFacts>,
    ) -> Self {
        let n = func.values.len();
        let doms = Dominators::of(func);

        let mut defs: Vec<Option<(u32, usize)>> = vec![None; n];
        for block in &func.blocks {
            for (index, op) in block.ops.iter().enumerate() {
                if let Some(result) = op.result {
                    defs[result.0 as usize] = Some((block.id.0, index));
                }
            }
        }

        let mut preds: Vec<Vec<u32>> = vec![Vec::new(); func.blocks.len()];
        for block in &func.blocks {
            for successor in block.term.successors() {
                let list = &mut preds[successor.0 as usize];
                if !list.contains(&block.id.0) {
                    list.push(block.id.0);
                }
            }
        }

        let widths: Vec<Option<u32>> = func
            .values
            .iter()
            .map(|def| match table.int_info(def.ty) {
                Some((false, bits)) => Some(bits),
                _ => None,
            })
            .collect();

        let mut analysis = Self {
            doms,
            canon: (0..n as u32).map(ValueId).collect(),
            defs,
            intervals: vec![None; n],
            widths,
            facts: Vec::new(),
            entry_facts: vec![Vec::new(); func.blocks.len()],
            ne_facts: Vec::new(),
            entry_ne_facts: vec![Vec::new(); func.blocks.len()],
            preds,
            vec_const_len: BTreeMap::new(),
            vec_one_shl_len: BTreeMap::new(),
            downward_bounds: Vec::new(),
            local_bounded_vecs: Vec::new(),
            load_to_local_vec: BTreeMap::new(),
            arena_len_handle: BTreeMap::new(),
            arena_val_origin: BTreeMap::new(),
            ret_vec_elem_len: BTreeMap::new(),
        };
        analysis.number_values(func, table, interns, arena_facts);
        analysis.collect_facts(func, table, interns);
        analysis.scope_facts(func);
        analysis.compute_intervals(func, table, interns, arena_facts);
        analysis.compute_downward_bounds(func, table);
        analysis.compute_local_bounded_vecs(func, table);
        if let Some(af) = arena_facts {
            analysis.compute_arena_origins(func, table, interns, af);
        }
        analysis
    }

    fn canon(&self, v: ValueId) -> ValueId {
        self.canon[v.0 as usize]
    }

    fn def_kind<'f>(&self, func: &'f Func, v: ValueId) -> Option<&'f OpKind> {
        let (block, index) = self.defs[v.0 as usize]?;
        Some(&func.blocks[block as usize].ops[index].kind)
    }

    fn interval(&self, v: ValueId) -> Option<Iv> {
        self.intervals[self.canon(v).0 as usize]
    }

    /// A comparison implied by `prefix = length - (length % nonzero)`, if this is one.
    fn aligned_prefix_comparison(
        &self,
        func: &Func,
        table: &TypeTable,
        kind: &OpKind,
    ) -> Option<(ValueId, ValueId, BinaryOp, bool)> {
        let OpKind::Binary { op, l, r, ty } = kind else {
            return None;
        };
        if !matches!(table.int_info(*ty), Some((false, _))) {
            return None;
        }
        let zero = |value: ValueId| {
            matches!(
                self.def_kind(func, self.canon(value)),
                Some(OpKind::Const(ConstTree::Scalar { bits: 0, .. }))
            )
        };
        let relation = if let Some(base) = self.aligned_prefix_base(func, table, *l) {
            if self.canon(*r) == base {
                match op {
                    BinaryOp::Le => Some(true),
                    BinaryOp::Gt => Some(false),
                    _ => None,
                }
            } else if zero(*r) {
                match op {
                    BinaryOp::Lt => Some(false),
                    BinaryOp::Ge => Some(true),
                    _ => None,
                }
            } else {
                None
            }
        } else if let Some(base) = self.aligned_prefix_base(func, table, *r) {
            if self.canon(*l) == base {
                match op {
                    BinaryOp::Ge => Some(true),
                    BinaryOp::Lt => Some(false),
                    _ => None,
                }
            } else if zero(*l) {
                match op {
                    BinaryOp::Gt => Some(false),
                    BinaryOp::Le => Some(true),
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        }?;
        Some((*l, *r, *op, relation))
    }

    /// The canonical length of `length - (length % nonzero)`.
    fn aligned_prefix_base(
        &self,
        func: &Func,
        table: &TypeTable,
        value: ValueId,
    ) -> Option<ValueId> {
        let OpKind::Binary {
            op: BinaryOp::Sub | BinaryOp::SubUnchecked,
            l: base,
            r: remainder,
            ty,
        } = self.def_kind(func, self.canon(value))?
        else {
            return None;
        };
        if !matches!(table.int_info(*ty), Some((false, _))) {
            return None;
        }
        let OpKind::Binary {
            op: BinaryOp::Rem,
            l: dividend,
            r: divisor,
            ty: remainder_ty,
        } = self.def_kind(func, self.canon(*remainder))?
        else {
            return None;
        };
        if remainder_ty != ty || self.canon(*base) != self.canon(*dividend) {
            return None;
        }
        match self.def_kind(func, self.canon(*divisor)) {
            Some(OpKind::Const(ConstTree::Scalar { bits, bytes })) if mask(*bits, *bytes) != 0 => {
                Some(self.canon(*base))
            }
            _ => None,
        }
    }

    /// Value numbering: congruent pure ops share one class representative, so equality of terms
    /// is equality of representatives. Assigned in reverse postorder, which is a dominance
    /// order, so an op's operands are numbered before it.
    fn number_values(
        &mut self,
        func: &Func,
        table: &TypeTable,
        interns: Option<&Interns>,
        arena_facts: Option<&ProgramBoundsFacts>,
    ) {
        #[derive(Clone, Ord, PartialOrd, Eq, PartialEq)]
        enum Key {
            Scalar(u128, u8, TypeId),
            Binary(u8, ValueId, ValueId, TypeId),
            Unary(u8, ValueId, TypeId),
            Cast(u8, ValueId, TypeId, TypeId),
        }
        let mut classes: BTreeMap<Key, ValueId> = BTreeMap::new();
        let mut incoming: Vec<Vec<Vec<ValueId>>> = vec![Vec::new(); func.blocks.len()];
        for block in &func.blocks {
            for (target, args) in block.term.edges() {
                incoming[target.0 as usize].push(args.to_vec());
            }
        }

        // Pre-scan local `Vec` cells: identify unmutated single-store cells (length fixed at init)
        // and cells populated by a counted `0..N` unconditional push loop.
        let mut cell_borrows: BTreeMap<ValueId, CellId> = BTreeMap::new();
        let mut cell_store_val: BTreeMap<CellId, Option<ValueId>> = BTreeMap::new();
        let mut cell_len_mutated: BTreeSet<CellId> = BTreeSet::new();
        for block in &func.blocks {
            for op in &block.ops {
                match &op.kind {
                    OpKind::BorrowView {
                        root: Root::Cell(c),
                        path,
                        ..
                    } if is_global_vec_ty(table, func.cells[c.0 as usize].ty) => {
                        if path.is_empty() {
                            if let Some(res) = op.result {
                                cell_borrows.insert(res, *c);
                            }
                        } else if !interns.is_some_and(|i| {
                            matches!(path.as_slice(), [Proj::Field { field, .. }] if is_vec_len_field(table, i, func.cells[c.0 as usize].ty, *field))
                        }) {
                            cell_len_mutated.insert(*c);
                        }
                    }
                    OpKind::RawView {
                        root: Root::Cell(c),
                        ..
                    } if is_global_vec_ty(table, func.cells[c.0 as usize].ty) => {
                        cell_len_mutated.insert(*c);
                    }
                    OpKind::CellStore { cell, v }
                        if is_global_vec_ty(table, func.cells[cell.0 as usize].ty) =>
                    {
                        cell_store_val
                            .entry(*cell)
                            .and_modify(|slot| {
                                *slot = None;
                                cell_len_mutated.insert(*cell);
                            })
                            .or_insert(Some(*v));
                    }
                    _ => {}
                }
            }
        }
        // Propagate `cell_borrows` through `Copy`/`Move`/whole-ref reborrows so copying `&v` or
        // `&mut v` doesn't falsely taint `v` as length-mutated.
        for &b in &self.doms.rpo {
            for op in &func.blocks[b.0 as usize].ops {
                let Some(res) = op.result else { continue };
                match &op.kind {
                    OpKind::Copy(s) | OpKind::Move(s) => {
                        if let Some(&cid) = cell_borrows.get(s) {
                            cell_borrows.insert(res, cid);
                        }
                    }
                    OpKind::BorrowView {
                        root: Root::Value(s),
                        path,
                        ..
                    } if matches!(path.as_slice(), [] | [Proj::Deref]) => {
                        if let Some(&cid) = cell_borrows.get(s) {
                            cell_borrows.insert(res, cid);
                        }
                    }
                    _ => {}
                }
            }
        }
        let is_immut_ref = |ty: TypeId| {
            matches!(
                table.get(table.peeled(ty)).map(|d| &d.kind),
                Some(TypeKind::Reference { mutable: false, .. })
            )
        };
        // Any call/op on a borrow of a `Vec` cell other than length-preserving ops or immutable
        // `&Vec<T>` calls marks it length-mutated.
        for block in &func.blocks {
            for op in &block.ops {
                match &op.kind {
                    OpKind::Call { semantic, args, .. } => {
                        for &a in args {
                            if let Some(&cid) = cell_borrows.get(&a) {
                                if !is_vec_len_preserving_call(*semantic)
                                    && !is_immut_ref(func.values[a.0 as usize].ty)
                                {
                                    cell_len_mutated.insert(cid);
                                }
                            }
                        }
                    }
                    OpKind::Copy(_)
                    | OpKind::Move(_)
                    | OpKind::Drop {
                        glue: DropGlue::None,
                        ..
                    } => {}
                    OpKind::BorrowView {
                        root: Root::Value(_),
                        path,
                        ..
                    } if matches!(path.as_slice(), [] | [Proj::Deref]) => {}
                    _ => {
                        for u in op.kind.uses() {
                            if let Some(&cid) = cell_borrows.get(&u) {
                                cell_len_mutated.insert(cid);
                            }
                        }
                    }
                }
            }
        }

        // Track `VecRootKey` per SSA value (for same-Vec length unification) and length-mutated
        // `VecRootKey`s across the whole function.
        let mut root_len_mutated: BTreeSet<VecRootKey> = BTreeSet::new();
        for &cid in &cell_len_mutated {
            root_len_mutated.insert(VecRootKey::Cell(cid));
        }
        let mut val_vec_root: BTreeMap<ValueId, VecRootKey> = BTreeMap::new();
        for (pi, &pv) in func.params.iter().enumerate() {
            let pty = func.values[pv.0 as usize].ty;
            if is_global_vec_ty(table, pty)
                || value::pointee_of(table, pty).is_some_and(|pt| is_global_vec_ty(table, pt))
            {
                val_vec_root.insert(pv, VecRootKey::Param(pi));
            }
        }
        for (&bv, &cid) in &cell_borrows {
            val_vec_root.insert(bv, VecRootKey::Cell(cid));
        }
        for &b in &self.doms.rpo {
            for &param in &func.blocks[b.0 as usize].params {
                if b.0 != 0 && is_global_vec_ty(table, func.values[param.0 as usize].ty) {
                    val_vec_root.insert(param, VecRootKey::Value(param));
                }
            }
            for op in &func.blocks[b.0 as usize].ops {
                if let Some(res) = op.result {
                    match &op.kind {
                        OpKind::Copy(s) | OpKind::Move(s) => {
                            if let Some(&rk) = val_vec_root.get(s) {
                                val_vec_root.insert(res, rk);
                            }
                        }
                        OpKind::CellLoad(c)
                            if is_global_vec_ty(table, func.cells[c.0 as usize].ty) =>
                        {
                            val_vec_root.insert(res, VecRootKey::Cell(*c));
                        }
                        OpKind::Call { .. }
                            if is_global_vec_ty(table, func.values[res.0 as usize].ty) =>
                        {
                            val_vec_root.insert(res, VecRootKey::Value(res));
                        }
                        OpKind::BorrowView { root, path, .. } => match (root, path.as_slice()) {
                            (Root::Value(v), [Proj::Deref] | []) => {
                                if let Some(&rk) = val_vec_root.get(v) {
                                    val_vec_root.insert(res, rk);
                                }
                            }
                            (
                                Root::Value(v),
                                [Proj::Deref, Proj::Field { field, ty }]
                                | [Proj::Field { field, ty }],
                            ) if is_global_vec_ty(table, *ty) => {
                                if let Some(pi) = func.params.iter().position(|&p| p == *v) {
                                    val_vec_root.insert(res, VecRootKey::ParamField(pi, *field));
                                }
                            }
                            (Root::Cell(c), [Proj::Field { field, ty }])
                                if is_global_vec_ty(table, *ty) =>
                            {
                                val_vec_root.insert(res, VecRootKey::CellField(*c, *field));
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
        }
        // Check if any `Param` / `ParamField` / `CellField` Vec root is ever passed to a
        // potentially length-mutating call or raw-viewed.
        for block in &func.blocks {
            for op in &block.ops {
                match &op.kind {
                    OpKind::Call { semantic, args, .. } => {
                        if !is_vec_len_preserving_call(*semantic) {
                            for &a in args {
                                if let Some(&rk) = val_vec_root.get(&a) {
                                    if !is_immut_ref(func.values[a.0 as usize].ty) {
                                        root_len_mutated.insert(rk);
                                    }
                                }
                            }
                            // Any opaque call taking `&mut Holder` could mutate `Holder`'s vec fields.
                            if !matches!(semantic, CallSemantic::Standard(_)) {
                                for &a in args {
                                    let aty = func.values[a.0 as usize].ty;
                                    if matches!(
                                        table.get(table.peeled(aty)).map(|d| &d.kind),
                                        Some(TypeKind::Reference { mutable: true, .. })
                                    ) {
                                        if let Some(pi) = func.params.iter().position(|&p| p == a) {
                                            for rk in val_vec_root.values() {
                                                if matches!(rk, VecRootKey::ParamField(p, _) if *p == pi)
                                                {
                                                    root_len_mutated.insert(*rk);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    OpKind::SetField { base, .. } => {
                        if let Some(&rk) = val_vec_root.get(base) {
                            root_len_mutated.insert(rk);
                        }
                    }
                    OpKind::StoreThrough { ptr, .. } => {
                        if let Some(&rk) = val_vec_root.get(ptr) {
                            root_len_mutated.insert(rk);
                        }
                    }
                    OpKind::RawView { root, .. } => match root {
                        Root::Cell(c) => {
                            root_len_mutated.insert(VecRootKey::Cell(*c));
                        }
                        Root::Value(v) => {
                            if let Some(&rk) = val_vec_root.get(v) {
                                root_len_mutated.insert(rk);
                            }
                        }
                    },
                    _ => {}
                }
            }
        }

        // Map of exact dynamic length `ValueId` per Vec SSA value, per unmutated `VecRootKey`, and
        // per slice derived from an exact-length Vec.
        let mut vec_dyn_len: BTreeMap<ValueId, ValueId> = BTreeMap::new();
        let mut cell_dyn_len: BTreeMap<CellId, ValueId> = BTreeMap::new();
        let mut cell_const_len: BTreeMap<CellId, u128> = BTreeMap::new();
        let mut cell_one_shl_len: BTreeMap<CellId, (ValueId, u32)> = BTreeMap::new();
        let mut unmutated_root_len: BTreeMap<VecRootKey, ValueId> = BTreeMap::new();
        // Same-block length map for mutable `VecRootKey`s (cleared on any length-mutating op!).
        let mut block_root_len: BTreeMap<VecRootKey, ValueId> = BTreeMap::new();
        // Map `ParamSliceEqLen` equivalence: canon slice param `pb` -> canon slice param `pa`.
        let mut param_eq_slice_len: BTreeMap<ValueId, ValueId> = BTreeMap::new();
        if let Some(af) = arena_facts {
            if let Some(eqs) = af.param_slice_eq_len.get(&func.item) {
                for eq in eqs {
                    if let (Some(&va), Some(&vb)) =
                        (func.params.get(eq.param_a), func.params.get(eq.param_b))
                    {
                        param_eq_slice_len.insert(vb, va);
                    }
                }
            }
        }

        // Precompute `defs` as `RBlockId` pairs for `find_counted_push_loop_len`.
        let rdefs: Vec<Option<(RBlockId, usize)>> = self
            .defs
            .iter()
            .map(|d| d.map(|(b, i)| (RBlockId(b), i)))
            .collect();

        for &block in &self.doms.rpo.clone() {
            block_root_len.clear();
            // A block parameter whose every incoming edge carries the same numbered value is
            // that value for analysis purposes. DCE may deliberately keep the parameter named
            // to prevent expression duplication in the printer; range proofs still need to see
            // through it (notably a `SliceView` handed to a bounds-check block).
            if block.0 != 0 {
                for (position, param) in func.blocks[block.0 as usize].params.iter().enumerate() {
                    let mut representative: Option<ValueId> = None;
                    let mut equal = true;
                    for args in &incoming[block.0 as usize] {
                        let Some(arg) = args.get(position).copied() else {
                            equal = false;
                            break;
                        };
                        if arg == *param {
                            continue;
                        }
                        let arg = self.canon(arg);
                        match representative {
                            None => representative = Some(arg),
                            Some(known) if known == arg => {}
                            Some(_) => {
                                equal = false;
                                break;
                            }
                        }
                    }
                    if equal {
                        if let Some(representative) = representative {
                            self.canon[param.0 as usize] = representative;
                        }
                    }
                }
            }
            for op in &func.blocks[block.0 as usize].ops {
                // Track `CellStore` exact lengths for unmutated local `Vec` cells, and invalidate
                // `block_root_len` on any length-mutating op!
                match &op.kind {
                    OpKind::CellStore { cell, v } => {
                        let cv = self.canon(*v);
                        if !cell_len_mutated.contains(cell) {
                            if let Some(&dl) = vec_dyn_len.get(v).or_else(|| vec_dyn_len.get(&cv)) {
                                cell_dyn_len.insert(*cell, dl);
                            }
                            if let Some(&cl) = self
                                .vec_const_len
                                .get(v)
                                .or_else(|| self.vec_const_len.get(&cv))
                            {
                                cell_const_len.insert(*cell, cl);
                            }
                            if let Some(&sl) = self
                                .vec_one_shl_len
                                .get(v)
                                .or_else(|| self.vec_one_shl_len.get(&cv))
                            {
                                cell_one_shl_len.insert(*cell, sl);
                            }
                        } else {
                            block_root_len.remove(&VecRootKey::Cell(*cell));
                        }
                    }
                    OpKind::Call { semantic, .. } if !is_vec_len_preserving_call(*semantic) => {
                        block_root_len.clear();
                    }
                    OpKind::SetField { .. } | OpKind::StoreThrough { .. }
                        if matches!(
                            &op.kind,
                            OpKind::SetField { base, .. }
                                if is_global_vec_ty(table, func.values[base.0 as usize].ty)
                        ) || matches!(
                            &op.kind,
                            OpKind::StoreThrough { ptr, .. }
                                if value::pointee_of(table, func.values[ptr.0 as usize].ty)
                                    .is_some_and(|pt| is_global_vec_ty(table, pt))
                        ) =>
                    {
                        block_root_len.clear();
                    }
                    _ => {}
                }

                let Some(result) = op.result else {
                    continue;
                };
                let ty = func.values[result.0 as usize].ty;
                let key = match &op.kind {
                    OpKind::Copy(v) | OpKind::Move(v) => {
                        let cv = self.canon(*v);
                        self.canon[result.0 as usize] = cv;
                        if let Some(&dl) = vec_dyn_len.get(v).or_else(|| vec_dyn_len.get(&cv)) {
                            vec_dyn_len.insert(result, dl);
                        }
                        if let Some(&cl) = self
                            .vec_const_len
                            .get(v)
                            .or_else(|| self.vec_const_len.get(&cv))
                        {
                            self.vec_const_len.insert(result, cl);
                        }
                        if let Some(&sl) = self
                            .vec_one_shl_len
                            .get(v)
                            .or_else(|| self.vec_one_shl_len.get(&cv))
                        {
                            self.vec_one_shl_len.insert(result, sl);
                        }
                        continue;
                    }
                    // Same-width unsigned `IntToInt` casts (e.g. `u32 as usize` / `usize as u32`
                    // on 32-bit targets) are value-preserving identities!
                    OpKind::Cast {
                        kind: CastKind::IntToInt,
                        v,
                        from,
                        to,
                    } if matches!(
                        (table.int_info(*from), table.int_info(*to)),
                        (Some((false, fb)), Some((false, tb))) if fb == tb
                    ) =>
                    {
                        self.canon[result.0 as usize] = self.canon(*v);
                        continue;
                    }
                    // A whole-slice reborrow (`&*s`) is the slice it reborrows; a borrow of an
                    // unmutated SSA `Vec` value (`&v` / `&mut v`) propagates its exact length!
                    OpKind::BorrowView {
                        root: Root::Value(v),
                        path,
                        ..
                    } => {
                        let cv = self.canon(*v);
                        if matches!(path.as_slice(), [Proj::Deref]) && is_slice_pointer(table, ty) {
                            self.canon[result.0 as usize] = cv;
                            if let Some(&dl) = vec_dyn_len.get(v).or_else(|| vec_dyn_len.get(&cv)) {
                                vec_dyn_len.insert(result, dl);
                            }
                            if let Some(&cl) = self
                                .vec_const_len
                                .get(v)
                                .or_else(|| self.vec_const_len.get(&cv))
                            {
                                self.vec_const_len.insert(result, cl);
                            }
                            if let Some(&sl) = self
                                .vec_one_shl_len
                                .get(v)
                                .or_else(|| self.vec_one_shl_len.get(&cv))
                            {
                                self.vec_one_shl_len.insert(result, sl);
                            }
                            continue;
                        }
                        if matches!(path.as_slice(), [] | [Proj::Deref])
                            && val_vec_root
                                .get(v)
                                .is_some_and(|rk| !root_len_mutated.contains(rk))
                        {
                            if let Some(&dl) = vec_dyn_len.get(v).or_else(|| vec_dyn_len.get(&cv)) {
                                vec_dyn_len.insert(result, dl);
                            }
                            if let Some(&cl) = self
                                .vec_const_len
                                .get(v)
                                .or_else(|| self.vec_const_len.get(&cv))
                            {
                                self.vec_const_len.insert(result, cl);
                            }
                            if let Some(&sl) = self
                                .vec_one_shl_len
                                .get(v)
                                .or_else(|| self.vec_one_shl_len.get(&cv))
                            {
                                self.vec_one_shl_len.insert(result, sl);
                            }
                        }
                        continue;
                    }
                    OpKind::CellLoad(c) if !cell_len_mutated.contains(c) => {
                        if let Some(&dl) = cell_dyn_len.get(c) {
                            vec_dyn_len.insert(result, dl);
                        }
                        if let Some(&cl) = cell_const_len.get(c) {
                            self.vec_const_len.insert(result, cl);
                        }
                        if let Some(&sl) = cell_one_shl_len.get(c) {
                            self.vec_one_shl_len.insert(result, sl);
                        }
                        continue;
                    }
                    OpKind::Field { base, field, .. }
                        if interns.is_some_and(|i| {
                            let bty = func.values[base.0 as usize].ty;
                            is_direct_global_vec_ty(table, bty)
                                && is_vec_len_field(table, i, bty, *field)
                        }) =>
                    {
                        if let Some(len_v) = self.resolve_vec_len_val(
                            *base,
                            &vec_dyn_len,
                            &cell_dyn_len,
                            &cell_const_len,
                            &cell_one_shl_len,
                            &val_vec_root,
                            &root_len_mutated,
                            &mut unmutated_root_len,
                            &mut block_root_len,
                            result,
                        ) {
                            self.canon[result.0 as usize] = len_v;
                        }
                        continue;
                    }
                    OpKind::Deref(ptr) => {
                        // Deref of `&v.len` on a Vec!
                        if let Some(i) = interns {
                            if let Some(OpKind::BorrowView { root, path, .. }) =
                                self.def_kind(func, self.canon(*ptr))
                            {
                                let vec_rk = match (root, path.as_slice()) {
                                    (Root::Cell(c), [Proj::Field { field, .. }])
                                        if is_direct_global_vec_ty(
                                            table,
                                            func.cells[c.0 as usize].ty,
                                        ) && is_vec_len_field(
                                            table,
                                            i,
                                            func.cells[c.0 as usize].ty,
                                            *field,
                                        ) =>
                                    {
                                        Some(VecRootKey::Cell(*c))
                                    }
                                    (Root::Value(v), [Proj::Deref, Proj::Field { field, .. }])
                                        if value::pointee_of(
                                            table,
                                            func.values[v.0 as usize].ty,
                                        )
                                        .is_some_and(|pt| {
                                            is_direct_global_vec_ty(table, pt)
                                                && is_vec_len_field(table, i, pt, *field)
                                        }) =>
                                    {
                                        val_vec_root.get(v).copied()
                                    }
                                    _ => None,
                                };
                                if let Some(rk) = vec_rk {
                                    if let Some(len_v) = self.unify_root_len(
                                        rk,
                                        result,
                                        &cell_dyn_len,
                                        &cell_const_len,
                                        &cell_one_shl_len,
                                        &root_len_mutated,
                                        &mut unmutated_root_len,
                                        &mut block_root_len,
                                    ) {
                                        self.canon[result.0 as usize] = len_v;
                                        continue;
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    OpKind::Call {
                        callee,
                        semantic,
                        args,
                        ..
                    } => {
                        match semantic {
                            CallSemantic::Standard(StandardOperation::NativeVecFromElemZeroed)
                                if !args.is_empty() =>
                            {
                                vec_dyn_len.insert(result, self.canon(args[0]));
                            }
                            CallSemantic::Standard(StandardOperation::VecFromElem)
                                if args.len() >= 2 =>
                            {
                                vec_dyn_len.insert(result, self.canon(args[1]));
                            }
                            CallSemantic::Standard(
                                StandardOperation::VecNew
                                | StandardOperation::NativeVecNew
                                | StandardOperation::VecWithCapacity
                                | StandardOperation::NativeVecWithCapacity,
                            ) => {
                                self.vec_const_len.insert(result, 0);
                            }
                            CallSemantic::Standard(StandardOperation::NativeVecFromArray)
                                if !args.is_empty() =>
                            {
                                let aty = table.peeled(func.values[args[0].0 as usize].ty);
                                if let Some(TypeKind::Array { length, .. }) =
                                    table.get(aty).map(|d| &d.kind)
                                {
                                    self.vec_const_len.insert(result, u128::from(*length));
                                }
                            }
                            CallSemantic::Standard(StandardOperation::VecClone)
                                if !args.is_empty() =>
                            {
                                let a0 = args[0];
                                if let Some(&dl) = vec_dyn_len.get(&a0) {
                                    vec_dyn_len.insert(result, dl);
                                } else if let Some(&VecRootKey::Cell(c)) = val_vec_root.get(&a0) {
                                    if !cell_len_mutated.contains(&c) {
                                        if let Some(&dl) = cell_dyn_len.get(&c) {
                                            vec_dyn_len.insert(result, dl);
                                        }
                                        if let Some(&cl) = cell_const_len.get(&c) {
                                            self.vec_const_len.insert(result, cl);
                                        }
                                    } else if let Some(&rl) =
                                        block_root_len.get(&VecRootKey::Cell(c))
                                    {
                                        vec_dyn_len.insert(result, rl);
                                    }
                                }
                            }
                            CallSemantic::Standard(StandardOperation::SliceToVec)
                                if !args.is_empty() =>
                            {
                                if let Ok(sl) =
                                    self.peel_slice_source(func, table, interns, args[0], None)
                                {
                                    vec_dyn_len.insert(result, sl);
                                }
                            }
                            CallSemantic::Standard(StandardOperation::VecLen)
                                if !args.is_empty() =>
                            {
                                if let Some(len_v) = self.resolve_vec_len_val(
                                    args[0],
                                    &vec_dyn_len,
                                    &cell_dyn_len,
                                    &cell_const_len,
                                    &cell_one_shl_len,
                                    &val_vec_root,
                                    &root_len_mutated,
                                    &mut unmutated_root_len,
                                    &mut block_root_len,
                                    result,
                                ) {
                                    self.canon[result.0 as usize] = len_v;
                                }
                            }
                            CallSemantic::Standard(
                                StandardOperation::VecAsSlice
                                | StandardOperation::NativeVecAsSlice
                                | StandardOperation::VecAsMutSlice
                                | StandardOperation::NativeVecAsMutSlice,
                            ) if !args.is_empty() => {
                                let a0 = args[0];
                                let ca0 = self.canon(a0);
                                if let Some(&dl) =
                                    vec_dyn_len.get(&a0).or_else(|| vec_dyn_len.get(&ca0))
                                {
                                    vec_dyn_len.insert(result, dl);
                                }
                                if let Some(&cl) = self
                                    .vec_const_len
                                    .get(&a0)
                                    .or_else(|| self.vec_const_len.get(&ca0))
                                {
                                    self.vec_const_len.insert(result, cl);
                                }
                                if let Some(&sl) = self
                                    .vec_one_shl_len
                                    .get(&a0)
                                    .or_else(|| self.vec_one_shl_len.get(&ca0))
                                {
                                    self.vec_one_shl_len.insert(result, sl);
                                }
                                if let Some(&rk) = val_vec_root.get(&a0) {
                                    val_vec_root.insert(result, rk);
                                    if let VecRootKey::Cell(c) = rk {
                                        if !cell_len_mutated.contains(&c) {
                                            if let Some(&dl) = cell_dyn_len.get(&c) {
                                                vec_dyn_len.insert(result, dl);
                                            }
                                            if let Some(&cl) = cell_const_len.get(&c) {
                                                self.vec_const_len.insert(result, cl);
                                            }
                                            if let Some(&sl) = cell_one_shl_len.get(&c) {
                                                self.vec_one_shl_len.insert(result, sl);
                                            }
                                        }
                                    }
                                    // Snapshot the slice's immutable fat-pointer length from the
                                    // same-block (no intervening resize) `Vec.len` read if present!
                                    if let std::collections::btree_map::Entry::Vacant(e) =
                                        vec_dyn_len.entry(result)
                                    {
                                        if let Some(&bl) = block_root_len.get(&rk) {
                                            e.insert(self.canon(bl));
                                        } else if !root_len_mutated.contains(&rk) {
                                            if let Some(&ul) = unmutated_root_len.get(&rk) {
                                                e.insert(self.canon(ul));
                                            }
                                        }
                                    }
                                }
                            }
                            CallSemantic::Ordinary => {
                                if let Callee::Direct { item, .. } = callee {
                                    if let Some(ret_len) =
                                        arena_facts.and_then(|af| af.return_vec_len.get(item))
                                    {
                                        self.apply_return_vec_len(
                                            func,
                                            result,
                                            args,
                                            ret_len,
                                            &mut vec_dyn_len,
                                        );
                                    }
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    OpKind::Unary {
                        op: UnaryOp::PtrMetadata,
                        v,
                        ty: operand_ty,
                    } => {
                        // Check exact Vec/allocation length first!
                        let meta_bits = self.widths[result.0 as usize];
                        let cv = self.canon(*v);
                        if let Some(&dl) = vec_dyn_len.get(v).or_else(|| vec_dyn_len.get(&cv)) {
                            if meta_bits.is_none_or(|b| self.widths[dl.0 as usize] == Some(b)) {
                                self.canon[result.0 as usize] = self.canon(dl);
                                continue;
                            }
                        }
                        if let Some(&cl) = self
                            .vec_const_len
                            .get(v)
                            .or_else(|| self.vec_const_len.get(&cv))
                        {
                            self.vec_const_len.insert(result, cl);
                        }
                        if let Some(&sl) = self
                            .vec_one_shl_len
                            .get(v)
                            .or_else(|| self.vec_one_shl_len.get(&cv))
                        {
                            self.vec_one_shl_len.insert(result, sl);
                        }
                        // Check counted `0..N` unconditional push loop length on a Vec cell!
                        if let Some(&rk) = val_vec_root.get(v).or_else(|| val_vec_root.get(&cv)) {
                            if let VecRootKey::Cell(c) = rk {
                                if cell_len_mutated.contains(&c) {
                                    if let Some(loop_n) = find_counted_push_loop_len(
                                        func,
                                        table,
                                        &self.doms,
                                        &rdefs,
                                        &self.canon,
                                        c,
                                        &cell_borrows,
                                    ) {
                                        let ln = self.canon(loop_n);
                                        if meta_bits
                                            .is_none_or(|b| self.widths[ln.0 as usize] == Some(b))
                                        {
                                            self.canon[result.0 as usize] = ln;
                                            continue;
                                        }
                                    }
                                }
                            }
                            if let Some(len_v) = self.unify_root_len(
                                rk,
                                result,
                                &cell_dyn_len,
                                &cell_const_len,
                                &cell_one_shl_len,
                                &root_len_mutated,
                                &mut unmutated_root_len,
                                &mut block_root_len,
                            ) {
                                if meta_bits
                                    .is_none_or(|b| self.widths[len_v.0 as usize] == Some(b))
                                {
                                    self.canon[result.0 as usize] = len_v;
                                    continue;
                                }
                            }
                        }
                        // Peel `SliceView`, sysroot `slice_from_parts` (`RawParts`), sysroot
                        // `byte_slice_ref`, whole-slice reborrows, and lossless slice pointer
                        // casts so `PtrMetadata` recovers the exact underlying length or root.
                        let mut base =
                            match self.peel_slice_source(func, table, interns, *v, meta_bits) {
                                Ok(len) => {
                                    self.canon[result.0 as usize] = len;
                                    continue;
                                }
                                Err(root) => root,
                            };
                        if let Some(&eq_base) = param_eq_slice_len.get(&base) {
                            base = self.canon(eq_base);
                        }
                        Key::Unary(UnaryOp::PtrMetadata as u8, base, *operand_ty)
                    }
                    OpKind::Const(ConstTree::Scalar { bits, bytes }) => {
                        Key::Scalar(*bits, *bytes, ty)
                    }
                    OpKind::Binary {
                        op: binary,
                        l,
                        r,
                        ty: operand_ty,
                    } if *binary != BinaryOp::Offset => {
                        Key::Binary(*binary as u8, self.canon(*l), self.canon(*r), *operand_ty)
                    }
                    OpKind::Unary {
                        op: unary,
                        v,
                        ty: operand_ty,
                    } => Key::Unary(*unary as u8, self.canon(*v), *operand_ty),
                    OpKind::Cast {
                        kind:
                            kind @ (CastKind::IntToInt
                            | CastKind::FloatToFloat
                            | CastKind::IntToFloat
                            | CastKind::Subtype),
                        v,
                        from,
                        to,
                    } => Key::Cast(*kind as u8, self.canon(*v), *from, *to),
                    _ => continue,
                };
                let entry = classes.entry(key).or_insert(result);
                self.canon[result.0 as usize] = *entry;
            }
        }

        // Lockstep `(len_param, slice_param)` block-parameter unification (e.g. `fflate::inflt`
        // carrying `(buf_len, out_sl)` across loop iterations where every incoming edge either
        // preserves the pair unchanged or assigns `buf_len = buf.len(); out_sl = &mut buf;`):
        // If every incoming edge to block `b` passes a slice whose known length equals the
        // paired integer argument (or passes `(p_len, p_slice)` itself), then `PtrMetadata(p_slice)`
        // unifies with `canon(p_len)`!
        let mut changed_lockstep = true;
        for _ in 0..3 {
            if !changed_lockstep {
                break;
            }
            changed_lockstep = false;
            for &block in &self.doms.rpo {
                if block.0 == 0 {
                    continue;
                }
                let params = &func.blocks[block.0 as usize].params;
                let edges = &incoming[block.0 as usize];
                if params.len() < 2 || edges.is_empty() {
                    continue;
                }
                for (sj, &p_slice) in params.iter().enumerate() {
                    if !is_slice_pointer(table, func.values[p_slice.0 as usize].ty)
                        || vec_dyn_len.contains_key(&p_slice)
                    {
                        continue;
                    }
                    let ps_c = self.canon(p_slice);
                    for (li, &p_len) in params.iter().enumerate() {
                        if li == sj || self.widths[p_len.0 as usize].is_none() {
                            continue;
                        }
                        let pl_c = self.canon(p_len);
                        let mut any_seed = false;
                        let mut all_ok = true;
                        for args in edges {
                            let (Some(&s_arg), Some(&l_arg)) = (args.get(sj), args.get(li)) else {
                                all_ok = false;
                                break;
                            };
                            let sa_c = self.canon(s_arg);
                            let la_c = self.canon(l_arg);
                            if sa_c == ps_c && la_c == pl_c {
                                continue;
                            }
                            let known_s_len = vec_dyn_len
                                .get(&s_arg)
                                .or_else(|| vec_dyn_len.get(&sa_c))
                                .copied()
                                .map(|v| self.canon(v));
                            if known_s_len == Some(la_c) {
                                any_seed = true;
                            } else {
                                all_ok = false;
                                break;
                            }
                        }
                        if all_ok && any_seed {
                            vec_dyn_len.insert(p_slice, pl_c);
                            vec_dyn_len.insert(ps_c, pl_c);
                            changed_lockstep = true;
                            break;
                        }
                    }
                }
            }
            if changed_lockstep {
                // Propagate newly discovered lockstep slice lengths to copies, reborrows, and
                // `PtrMetadata` ops in RPO.
                for &block in &self.doms.rpo.clone() {
                    for op in &func.blocks[block.0 as usize].ops {
                        let Some(res) = op.result else { continue };
                        match &op.kind {
                            OpKind::Copy(v) | OpKind::Move(v) => {
                                let cv = self.canon(*v);
                                if let Some(&dl) =
                                    vec_dyn_len.get(v).or_else(|| vec_dyn_len.get(&cv))
                                {
                                    vec_dyn_len.insert(res, dl);
                                }
                            }
                            OpKind::BorrowView {
                                root: Root::Value(v),
                                path,
                                ..
                            } if matches!(path.as_slice(), [Proj::Deref])
                                && is_slice_pointer(table, func.values[res.0 as usize].ty) =>
                            {
                                let cv = self.canon(*v);
                                if let Some(&dl) =
                                    vec_dyn_len.get(v).or_else(|| vec_dyn_len.get(&cv))
                                {
                                    vec_dyn_len.insert(res, dl);
                                }
                            }
                            OpKind::Unary {
                                op: UnaryOp::PtrMetadata,
                                v,
                                ..
                            } => {
                                let cv = self.canon(*v);
                                if let Some(&dl) =
                                    vec_dyn_len.get(v).or_else(|| vec_dyn_len.get(&cv))
                                {
                                    let meta_bits = self.widths[res.0 as usize];
                                    if meta_bits
                                        .is_none_or(|b| self.widths[dl.0 as usize] == Some(b))
                                    {
                                        self.canon[res.0 as usize] = self.canon(dl);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_vec_len_val(
        &mut self,
        vec_val: ValueId,
        vec_dyn_len: &BTreeMap<ValueId, ValueId>,
        cell_dyn_len: &BTreeMap<CellId, ValueId>,
        cell_const_len: &BTreeMap<CellId, u128>,
        cell_one_shl_len: &BTreeMap<CellId, (ValueId, u32)>,
        val_vec_root: &BTreeMap<ValueId, VecRootKey>,
        root_len_mutated: &BTreeSet<VecRootKey>,
        unmutated_root_len: &mut BTreeMap<VecRootKey, ValueId>,
        block_root_len: &mut BTreeMap<VecRootKey, ValueId>,
        result: ValueId,
    ) -> Option<ValueId> {
        let cv = self.canon(vec_val);
        if let Some(&dl) = vec_dyn_len.get(&vec_val).or_else(|| vec_dyn_len.get(&cv)) {
            return Some(self.canon(dl));
        }
        if let Some(&cl) = self
            .vec_const_len
            .get(&vec_val)
            .or_else(|| self.vec_const_len.get(&cv))
        {
            self.vec_const_len.insert(result, cl);
        }
        if let Some(&sl) = self
            .vec_one_shl_len
            .get(&vec_val)
            .or_else(|| self.vec_one_shl_len.get(&cv))
        {
            self.vec_one_shl_len.insert(result, sl);
        }
        if let Some(&rk) = val_vec_root.get(&vec_val).or_else(|| val_vec_root.get(&cv)) {
            return self.unify_root_len(
                rk,
                result,
                cell_dyn_len,
                cell_const_len,
                cell_one_shl_len,
                root_len_mutated,
                unmutated_root_len,
                block_root_len,
            );
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn unify_root_len(
        &mut self,
        rk: VecRootKey,
        result: ValueId,
        cell_dyn_len: &BTreeMap<CellId, ValueId>,
        cell_const_len: &BTreeMap<CellId, u128>,
        cell_one_shl_len: &BTreeMap<CellId, (ValueId, u32)>,
        root_len_mutated: &BTreeSet<VecRootKey>,
        unmutated_root_len: &mut BTreeMap<VecRootKey, ValueId>,
        block_root_len: &mut BTreeMap<VecRootKey, ValueId>,
    ) -> Option<ValueId> {
        if !root_len_mutated.contains(&rk) {
            if let VecRootKey::Cell(c) = rk {
                if let Some(&cl) = cell_const_len.get(&c) {
                    self.vec_const_len.insert(result, cl);
                }
                if let Some(&sl) = cell_one_shl_len.get(&c) {
                    self.vec_one_shl_len.insert(result, sl);
                }
                if let Some(&dl) = cell_dyn_len.get(&c) {
                    return Some(self.canon(dl));
                }
            }
            let entry = *unmutated_root_len.entry(rk).or_insert(result);
            return Some(self.canon(entry));
        }
        let entry = *block_root_len.entry(rk).or_insert(result);
        Some(self.canon(entry))
    }

    fn apply_return_vec_len(
        &mut self,
        func: &Func,
        result: ValueId,
        args: &[ValueId],
        summary: &ReturnVecLen,
        vec_dyn_len: &mut BTreeMap<ValueId, ValueId>,
    ) {
        match summary {
            ReturnVecLen::Const(c) => {
                self.vec_const_len.insert(result, *c);
            }
            ReturnVecLen::Param(k) => {
                if let Some(&ak) = args.get(*k) {
                    vec_dyn_len.insert(result, self.canon(ak));
                }
            }
            ReturnVecLen::ParamSliceLen(k) => {
                if let Some(&ak) = args.get(*k) {
                    if let Some(&dl) = vec_dyn_len.get(&ak) {
                        vec_dyn_len.insert(result, dl);
                    }
                }
            }
            ReturnVecLen::OneShlParam { shift_param, bits } => {
                if let Some(&ak) = args.get(*shift_param) {
                    self.vec_one_shl_len.insert(result, (self.canon(ak), *bits));
                }
            }
            ReturnVecLen::CondBoolParam {
                cond_param,
                when_true,
                when_false,
            } => {
                if let Some(&cv) = args.get(*cond_param) {
                    match self.def_kind(func, self.canon(cv)) {
                        Some(OpKind::Const(ConstTree::Scalar { bits: 1, .. })) => {
                            self.apply_return_vec_len(func, result, args, when_true, vec_dyn_len);
                        }
                        Some(OpKind::Const(ConstTree::Scalar { bits: 0, .. })) => {
                            self.apply_return_vec_len(func, result, args, when_false, vec_dyn_len);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Collects the known-true comparisons: decided switch edges and executed asserts, plus
    /// `!= const` disequalities for interval refinement and sentinel guards.
    fn collect_facts(&mut self, func: &Func, table: &TypeTable, _interns: Option<&Interns>) {
        let mut facts = Vec::new();
        let mut ne_facts = Vec::new();
        let const_val = |v: ValueId| -> Option<u128> {
            let cv = self.canon(v);
            let max = self.widths[cv.0 as usize].map_or(u128::MAX, width_max);
            match self.def_kind(func, cv) {
                Some(OpKind::Const(ConstTree::Scalar { bits, bytes })) => {
                    Some(mask(*bits, *bytes).min(max))
                }
                _ => None,
            }
        };
        let normalize = |cond: ValueId,
                         positive: bool,
                         site: Site,
                         facts: &mut Vec<Fact>,
                         ne_facts: &mut Vec<NeFact>| {
            let Some(OpKind::Binary { op, l, r, ty }) = self.def_kind(func, self.canon(cond))
            else {
                return;
            };
            if !matches!(table.int_info(*ty), Some((false, _))) {
                return;
            }
            let mut push = |x: ValueId, rel: Rel, y: ValueId| {
                facts.push(Fact {
                    x: Bound::Value(x),
                    rel,
                    y: Bound::Value(y),
                    site,
                });
            };
            match (op, positive) {
                (BinaryOp::Lt, true) => push(*l, Rel::Lt, *r),
                (BinaryOp::Lt, false) => push(*r, Rel::Le, *l),
                (BinaryOp::Le, true) => push(*l, Rel::Le, *r),
                (BinaryOp::Le, false) => push(*r, Rel::Lt, *l),
                (BinaryOp::Gt, true) => push(*r, Rel::Lt, *l),
                (BinaryOp::Gt, false) => push(*l, Rel::Le, *r),
                (BinaryOp::Ge, true) => push(*r, Rel::Le, *l),
                (BinaryOp::Ge, false) => push(*l, Rel::Lt, *r),
                (BinaryOp::Eq, true) | (BinaryOp::Ne, false) => {
                    push(*l, Rel::Le, *r);
                    push(*r, Rel::Le, *l);
                }
                (BinaryOp::Ne, true) | (BinaryOp::Eq, false) => {
                    if let Some(c) = const_val(*r) {
                        ne_facts.push(NeFact { v: *l, c, site });
                    } else if let Some(c) = const_val(*l) {
                        ne_facts.push(NeFact { v: *r, c, site });
                    }
                }
                _ => {}
            }
        };

        for block in &func.blocks {
            for (index, op) in block.ops.iter().enumerate() {
                if let OpKind::Assert { cond, expected, .. } = &op.kind {
                    normalize(
                        *cond,
                        *expected,
                        Site::Assert {
                            block: block.id,
                            index,
                            cond: *cond,
                        },
                        &mut facts,
                        &mut ne_facts,
                    );
                }
            }
        }

        for block in func.blocks.iter().skip(1) {
            // The edge fact needs every entry into the block to decide the comparison the same
            // way: exactly one predecessor, reaching it over exactly one switch edge.
            let [pred] = self.preds[block.id.0 as usize].as_slice() else {
                continue;
            };
            let RTerm::Switch {
                scrut,
                scrut_ty,
                cases,
                default,
            } = &func.blocks[*pred as usize].term
            else {
                continue;
            };
            let mut entering: Vec<Option<u128>> = cases
                .iter()
                .filter(|(_, target, _)| *target == block.id)
                .map(|(value, _, _)| Some(*value))
                .collect();
            if default.0 == block.id {
                entering.push(None);
            }
            let site = Site::Edge {
                target: block.id,
                cond: *scrut,
            };
            if let [Some(c)] = entering.as_slice() {
                if !matches!(
                    self.def_kind(func, self.canon(*scrut)),
                    Some(OpKind::Binary { .. })
                ) && matches!(table.int_info(*scrut_ty), Some((false, bits)) if *c <= width_max(bits))
                    && (*c > 1 || cases.iter().any(|(value, _, _)| *value > 1))
                {
                    facts.push(Fact {
                        x: Bound::Const(*c),
                        rel: Rel::Le,
                        y: Bound::Value(*scrut),
                        site,
                    });
                    facts.push(Fact {
                        x: Bound::Value(*scrut),
                        rel: Rel::Le,
                        y: Bound::Const(*c),
                        site,
                    });
                }
            }
            // Switch default edge excludes every case constant on an unsigned integer scrutinee!
            if let [None] = entering.as_slice() {
                if !matches!(
                    self.def_kind(func, self.canon(*scrut)),
                    Some(OpKind::Binary { .. })
                ) {
                    if let Some((false, bits)) = table.int_info(*scrut_ty) {
                        let max = width_max(bits);
                        for &(cv, _, _) in cases {
                            if cv <= max {
                                ne_facts.push(NeFact {
                                    v: *scrut,
                                    c: cv,
                                    site,
                                });
                            }
                        }
                    }
                }
            }
            let positive = match entering.as_slice() {
                // The one case edge with value 0: the condition is false.
                [Some(0)] => false,
                // The one case edge with value 1: true.
                [Some(1)] => true,
                // The default edge, with 0 the only case value: not zero, so true.
                [None] if cases.iter().all(|(value, _, _)| *value == 0) => true,
                _ => continue,
            };
            normalize(*scrut, positive, site, &mut facts, &mut ne_facts);
        }

        self.facts = facts;
        self.ne_facts = ne_facts;
    }

    /// Distributes fact indices to the blocks they hold in: a fact holds at a block's entry when
    /// its site is an edge into the block or a dominator, or an assert in a strict dominator
    /// (every op of a strict dominator has run by the time the block is entered).
    fn scope_facts(&mut self, func: &Func) {
        let by_edge_target: BTreeMap<u32, Vec<usize>> = {
            let mut map: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
            for (index, fact) in self.facts.iter().enumerate() {
                if let Site::Edge { target, .. } = fact.site {
                    map.entry(target.0).or_default().push(index);
                }
            }
            map
        };
        let by_assert_block: BTreeMap<u32, Vec<usize>> = {
            let mut map: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
            for (index, fact) in self.facts.iter().enumerate() {
                if let Site::Assert { block, .. } = fact.site {
                    map.entry(block.0).or_default().push(index);
                }
            }
            map
        };
        let ne_by_edge_target: BTreeMap<u32, Vec<usize>> = {
            let mut map: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
            for (index, fact) in self.ne_facts.iter().enumerate() {
                if let Site::Edge { target, .. } = fact.site {
                    map.entry(target.0).or_default().push(index);
                }
            }
            map
        };
        let ne_by_assert_block: BTreeMap<u32, Vec<usize>> = {
            let mut map: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
            for (index, fact) in self.ne_facts.iter().enumerate() {
                if let Site::Assert { block, .. } = fact.site {
                    map.entry(block.0).or_default().push(index);
                }
            }
            map
        };
        for &block in &self.doms.rpo.clone() {
            let b = block.0 as usize;
            let mut list = match self.doms.idom[b] {
                Some(idom) if idom != block => {
                    let mut inherited = self.entry_facts[idom.0 as usize].clone();
                    if let Some(asserts) = by_assert_block.get(&idom.0) {
                        inherited.extend_from_slice(asserts);
                    }
                    inherited
                }
                _ => Vec::new(),
            };
            if let Some(edges) = by_edge_target.get(&block.0) {
                list.extend_from_slice(edges);
            }
            self.entry_facts[b] = list;

            let mut ne_list = match self.doms.idom[b] {
                Some(idom) if idom != block => {
                    let mut inherited = self.entry_ne_facts[idom.0 as usize].clone();
                    if let Some(asserts) = ne_by_assert_block.get(&idom.0) {
                        inherited.extend_from_slice(asserts);
                    }
                    inherited
                }
                _ => Vec::new(),
            };
            if let Some(edges) = ne_by_edge_target.get(&block.0) {
                ne_list.extend_from_slice(edges);
            }
            self.entry_ne_facts[b] = ne_list;
        }
        let _ = func;
    }

    /// Fact indices that hold at `block`'s terminator: everything at entry plus its own asserts.
    fn term_facts(&self, block: RBlockId) -> Vec<usize> {
        let mut list = self.entry_facts[block.0 as usize].clone();
        for (index, fact) in self.facts.iter().enumerate() {
            if matches!(fact.site, Site::Assert { block: b, .. } if b == block) {
                list.push(index);
            }
        }
        list
    }

    fn term_ne_facts(&self, block: RBlockId) -> Vec<usize> {
        let mut list = self.entry_ne_facts[block.0 as usize].clone();
        for (index, fact) in self.ne_facts.iter().enumerate() {
            if matches!(fact.site, Site::Assert { block: b, .. } if b == block) {
                list.push(index);
            }
        }
        list
    }

    /// The interval fixpoint: block parameters join their edge contributions, refined by the
    /// facts at each edge; ops transfer in reverse postorder. A widening round keeps counted
    /// loops from crawling one trip at a time, and a failure to stabilize falls back to full
    /// ranges for every parameter, which one more transfer pass makes consistent.
    fn compute_intervals(
        &mut self,
        func: &Func,
        table: &TypeTable,
        interns: Option<&Interns>,
        arena_facts: Option<&ProgramBoundsFacts>,
    ) {
        const WIDEN: usize = 3;
        const CAP: usize = 24;

        for &param in &func.blocks[0].params {
            if let Some(bits) = self.widths[param.0 as usize] {
                self.intervals[param.0 as usize] = Some(Iv::full(width_max(bits)));
            }
        }
        let term_facts: Vec<Vec<usize>> = (0..func.blocks.len())
            .map(|b| self.term_facts(RBlockId(b as u32)))
            .collect();
        let term_ne_facts: Vec<Vec<usize>> = (0..func.blocks.len())
            .map(|b| self.term_ne_facts(RBlockId(b as u32)))
            .collect();

        let mut stable = false;
        for round in 0..CAP {
            let mut changed = false;

            // Parameters first: join every edge's refined contribution.
            let mut pending: BTreeMap<ValueId, (Iv, Option<u128>)> = BTreeMap::new();
            for &block in &self.doms.rpo {
                for (target, args) in func.blocks[block.0 as usize].term.edges() {
                    if target.0 == 0 {
                        continue;
                    }
                    let params = &func.blocks[target.0 as usize].params;
                    for (position, arg) in args.iter().enumerate() {
                        let Some(&param) = params.get(position) else {
                            continue;
                        };
                        let Some(_bits) = self.widths[param.0 as usize] else {
                            continue;
                        };
                        let Some((contribution, cap)) = self.refine(
                            func,
                            table,
                            *arg,
                            &term_facts[block.0 as usize],
                            &term_ne_facts[block.0 as usize],
                        ) else {
                            continue;
                        };
                        pending
                            .entry(param)
                            .and_modify(|(iv, caps)| {
                                *iv = iv.join(contribution);
                                *caps = (*caps).max(cap);
                            })
                            .or_insert((contribution, cap));
                    }
                }
            }
            for (param, (joined, cap)) in pending {
                let slot = &mut self.intervals[param.0 as usize];
                let mut next = match *slot {
                    Some(old) => old.join(joined),
                    None => joined,
                };
                // The widening round: a parameter still growing jumps to the tightest fact cap
                // among its edges, or its full type range when no fact bounds it, so a counted
                // loop converges instead of crawling one trip per round. Preserve `lo` for upward
                // loops (e.g. `16..64` or `size = 1; while size < n * 4 { size <<= 1; }`) so
                // non-wrapping subtractions (`p - 16`, `size - 1`) remain provable.
                if round >= WIDEN {
                    let bits = self.widths[param.0 as usize].expect("joined params are unsigned");
                    if slot.is_some_and(|old| old.hi < next.hi) {
                        next.hi = cap.unwrap_or(width_max(bits)).max(next.hi);
                    }
                    if slot.is_some_and(|old| next.lo < old.lo) {
                        next.lo = 0;
                    }
                }
                if *slot != Some(next) {
                    *slot = Some(next);
                    changed = true;
                }
            }

            // Then every op, in a dominance order, so operands are already current.
            for &block in &self.doms.rpo {
                for op in &func.blocks[block.0 as usize].ops {
                    let Some(result) = op.result else {
                        continue;
                    };
                    let Some(bits) = self.widths[result.0 as usize] else {
                        continue;
                    };
                    let next = self.transfer(
                        func,
                        table,
                        interns,
                        arena_facts,
                        block,
                        result,
                        &op.kind,
                        bits,
                    );
                    let slot = &mut self.intervals[result.0 as usize];
                    if *slot != next {
                        *slot = next;
                        changed = true;
                    }
                }
            }

            if !changed {
                stable = true;
                break;
            }
        }

        if !stable {
            // Not converged: give every parameter its full range and make the ops consistent
            // with one more pass, which is a fixpoint because parameters are the only cycles.
            for block in func.blocks.iter().skip(1) {
                for &param in &block.params {
                    if let Some(bits) = self.widths[param.0 as usize] {
                        self.intervals[param.0 as usize] = Some(Iv::full(width_max(bits)));
                    }
                }
            }
            for &block in &self.doms.rpo.clone() {
                for op in &func.blocks[block.0 as usize].ops {
                    let Some(result) = op.result else {
                        continue;
                    };
                    let Some(bits) = self.widths[result.0 as usize] else {
                        continue;
                    };
                    self.intervals[result.0 as usize] = self.transfer(
                        func,
                        table,
                        interns,
                        arena_facts,
                        block,
                        result,
                        &op.kind,
                        bits,
                    );
                }
            }
        }
    }

    /// Refines `v`'s interval against `facts` and `ne_facts` (both upper and lower bounds).
    fn interval_under(&self, v: ValueId, facts: &[usize], ne_facts: &[usize]) -> Option<Iv> {
        let mut iv = self.interval(v)?;
        let canon = self.canon(v);
        for &index in facts {
            let fact = &self.facts[index];
            if matches!(fact.x, Bound::Value(x) if self.canon(x) == canon) {
                let bound_hi = match fact.y {
                    Bound::Value(y) => self.interval(y).map(|b| b.hi),
                    Bound::Const(c) => Some(c),
                };
                if let Some(bound_hi) = bound_hi {
                    if fact.rel == Rel::Lt && bound_hi == 0 {
                        continue;
                    }
                    let cap = match fact.rel {
                        Rel::Le => bound_hi,
                        Rel::Lt => bound_hi - 1,
                    };
                    if cap < iv.hi {
                        iv.hi = cap;
                        iv.lo = iv.lo.min(iv.hi);
                    }
                }
            }
            if matches!(fact.y, Bound::Value(y) if self.canon(y) == canon) {
                let bound_lo = match fact.x {
                    Bound::Value(x) => self.interval(x).map(|b| b.lo),
                    Bound::Const(c) => Some(c),
                };
                if let Some(bound_lo) = bound_lo {
                    let floor = match fact.rel {
                        Rel::Le => Some(bound_lo),
                        Rel::Lt => bound_lo.checked_add(1),
                    };
                    if let Some(floor) = floor {
                        if floor > iv.lo && floor <= iv.hi {
                            iv.lo = floor;
                        }
                    }
                }
            }
        }
        // Short fixpoint over `v != c` disequalities: e.g. `s != 0 && s != 1 => s >= 2` and
        // `n != 0 && n != 1 && n != 2 => n >= 3`.
        for _ in 0..4 {
            let mut tightened = false;
            for &ni in ne_facts {
                let nf = &self.ne_facts[ni];
                if self.canon(nf.v) != canon {
                    continue;
                }
                if iv.lo == nf.c && iv.lo < iv.hi {
                    iv.lo += 1;
                    tightened = true;
                }
                if iv.hi == nf.c && iv.lo < iv.hi {
                    iv.hi -= 1;
                    tightened = true;
                }
            }
            if !tightened {
                break;
            }
        }
        Some(iv)
    }

    /// Interval of `v` refined by the facts that hold on entry to `block`.
    fn interval_in(&self, v: ValueId, block: RBlockId) -> Option<Iv> {
        self.interval_under(
            v,
            &self.entry_facts[block.0 as usize],
            &self.entry_ne_facts[block.0 as usize],
        )
    }

    /// The contribution of `arg` over an edge whose source block's terminator sees `facts`:
    /// the argument's interval, clamped by any fact bounding it from above (including stepped
    /// induction back-edges `base + k` and doubling back-edges `base << 1` where a fact bounds
    /// `base`). The clamp is also returned as the widening bound for the parameter it feeds.
    fn refine(
        &self,
        func: &Func,
        table: &TypeTable,
        arg: ValueId,
        facts: &[usize],
        ne_facts: &[usize],
    ) -> Option<(Iv, Option<u128>)> {
        let mut iv = self.interval_under(arg, facts, ne_facts)?;
        let mut cap: Option<u128> = None;
        for &index in facts {
            let fact = &self.facts[index];
            let Bound::Value(x) = fact.x else {
                continue;
            };
            if self.canon(x) != self.canon(arg) {
                continue;
            }
            let bound_hi = match fact.y {
                Bound::Value(y) => {
                    let Some(bound) = self.interval(y) else {
                        continue;
                    };
                    bound.hi
                }
                Bound::Const(c) => c,
            };
            if fact.rel == Rel::Lt && bound_hi == 0 {
                continue;
            }
            let bound = match fact.rel {
                Rel::Le => bound_hi,
                Rel::Lt => bound_hi - 1,
            };
            cap = Some(cap.map_or(bound, |current| current.min(bound)));
        }
        // Stepped loop back-edge: `arg = base + k` where `facts` caps `base <= B`, so `arg <= B + k`.
        if let Some(OpKind::Binary {
            op: BinaryOp::Add | BinaryOp::AddUnchecked,
            l,
            r,
            ty,
        }) = self.def_kind(func, self.canon(arg))
        {
            if let Some((false, bits)) = table.int_info(*ty) {
                let max = width_max(bits);
                let const_of = |side: ValueId| match self.def_kind(func, self.canon(side)) {
                    Some(OpKind::Const(ConstTree::Scalar { bits: v, bytes })) => {
                        Some(mask(*v, *bytes).min(max))
                    }
                    _ => None,
                };
                if let Some((base, k)) = match (const_of(*l), const_of(*r)) {
                    (None, Some(k)) => Some((*l, k)),
                    (Some(k), None) => Some((*r, k)),
                    _ => None,
                } {
                    let base_canon = self.canon(base);
                    let mut base_cap: Option<u128> = None;
                    for &index in facts {
                        let fact = &self.facts[index];
                        let Bound::Value(x) = fact.x else { continue };
                        if self.canon(x) != base_canon {
                            continue;
                        }
                        let bound_hi = match fact.y {
                            Bound::Value(y) => self.interval(y).map(|b| b.hi),
                            Bound::Const(c) => Some(c),
                        };
                        let Some(bound_hi) = bound_hi else { continue };
                        if fact.rel == Rel::Lt && bound_hi == 0 {
                            continue;
                        }
                        let b = match fact.rel {
                            Rel::Le => bound_hi,
                            Rel::Lt => bound_hi - 1,
                        };
                        base_cap = Some(base_cap.map_or(b, |c| c.min(b)));
                    }
                    if let Some(b_hi) = base_cap {
                        if let Some(stepped_cap) = b_hi.checked_add(k).filter(|&s| s <= max) {
                            cap = Some(cap.map_or(stepped_cap, |c| c.min(stepped_cap)));
                        }
                    }
                    if let Some(base_iv) = self.interval_under(base, facts, ne_facts) {
                        if let Some(sum_hi) = base_iv.hi.checked_add(k).filter(|&s| s <= max) {
                            let sum_lo = base_iv.lo + k;
                            iv.hi = iv.hi.min(sum_hi);
                            iv.lo = iv.lo.max(sum_lo).min(iv.hi);
                        }
                    }
                }
            }
        }
        // Doubling loop back-edge: `arg = base << 1` where `facts` caps `base < B <= 2^(bits-1)`.
        if let Some(OpKind::Binary {
            op: BinaryOp::Shl | BinaryOp::ShlUnchecked,
            l: base,
            r: shift,
            ty,
        }) = self.def_kind(func, self.canon(arg))
        {
            if let Some((false, bits)) = table.int_info(*ty) {
                let max = width_max(bits);
                let is_one = matches!(
                    self.def_kind(func, self.canon(*shift)),
                    Some(OpKind::Const(ConstTree::Scalar { bits: 1, .. }))
                );
                if is_one {
                    if let Some(base_iv) = self.interval_under(*base, facts, ne_facts) {
                        if base_iv.hi <= (max >> 1) {
                            let doubled_hi = base_iv.hi << 1;
                            let doubled_lo = base_iv.lo << 1;
                            iv.hi = iv.hi.min(doubled_hi);
                            iv.lo = iv.lo.max(doubled_lo).min(iv.hi);
                            cap = Some(cap.map_or(doubled_hi, |c| c.min(doubled_hi)));
                        }
                    }
                }
            }
        }
        if let Some(cap) = cap {
            iv.hi = iv.hi.min(cap);
            iv.lo = iv.lo.min(iv.hi);
        }
        Some((iv, cap))
    }

    /// The interval an op's result takes from its operands; `None` when an operand is still
    /// unknown (bottom), so optimism survives until the fixpoint fills it in.
    #[allow(clippy::too_many_arguments)]
    fn transfer(
        &self,
        func: &Func,
        table: &TypeTable,
        interns: Option<&Interns>,
        arena_facts: Option<&ProgramBoundsFacts>,
        block: RBlockId,
        result: ValueId,
        kind: &OpKind,
        bits: u32,
    ) -> Option<Iv> {
        let max = width_max(bits);
        let top = Some(Iv::full(max));
        // Exact constant or `1 << k` length attached during value numbering.
        if let Some(&c) = self
            .vec_const_len
            .get(&result)
            .or_else(|| self.vec_const_len.get(&self.canon(result)))
        {
            return Some(Iv::point(c.min(max)));
        }
        if let Some(&(_, shl_bits)) = self
            .vec_one_shl_len
            .get(&result)
            .or_else(|| self.vec_one_shl_len.get(&self.canon(result)))
        {
            let hi = if shl_bits >= 2 && shl_bits <= bits {
                1u128 << (shl_bits - 1)
            } else {
                max
            };
            return Some(Iv { lo: 1, hi });
        }
        match kind {
            OpKind::Const(ConstTree::Scalar { bits: value, bytes }) => {
                Some(Iv::point(mask(*value, *bytes).min(max)))
            }
            OpKind::Copy(v) | OpKind::Move(v) => match self.widths[v.0 as usize] {
                Some(w) if w == bits => self.interval_in(*v, block),
                _ => top,
            },
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v,
                ..
            } => Some(self.metadata_interval(func, table, interns, arena_facts, *v, bits)),
            OpKind::Binary { op, l, r, ty } => {
                if !matches!(table.int_info(*ty), Some((false, w)) if w == bits) {
                    return top;
                }
                // A mask bounds the result even while the other operand is still bottom: for
                // unsigned integers `x & m <= m`. This matters for shifts whose dynamic amount
                // the interval domain cannot model, followed by a constant array-index mask.
                if *op == BinaryOp::BitAnd {
                    return Some(bitand_transfer(
                        self.interval_in(*l, block),
                        self.interval_in(*r, block),
                        max,
                    ));
                }
                // Unsigned left shift:
                // - `1 << k` in Rust masks `k` modulo `bits`, so it is ALWAYS in `[1, 1 << (bits - 1)]`!
                // - `x << const_s` with `x.hi <= max >> s` is `[x.lo << s, x.hi << s]`.
                if matches!(op, BinaryOp::Shl | BinaryOp::ShlUnchecked) {
                    let l_is_one = matches!(
                        self.def_kind(func, self.canon(*l)),
                        Some(OpKind::Const(ConstTree::Scalar { bits: 1, .. }))
                    );
                    if let Some(OpKind::Const(ConstTree::Scalar {
                        bits: shift_raw,
                        bytes,
                    })) = self.def_kind(func, self.canon(*r))
                    {
                        let shift = (mask(*shift_raw, *bytes) as u32) & (bits - 1);
                        let l_iv = self.interval_in(*l, block)?;
                        if l_iv.hi <= (max >> shift) {
                            return Some(Iv {
                                lo: l_iv.lo << shift,
                                hi: l_iv.hi << shift,
                            });
                        }
                    }
                    if l_is_one && bits >= 2 {
                        return Some(Iv {
                            lo: 1,
                            hi: 1u128 << (bits - 1),
                        });
                    }
                    return top;
                }
                // Unsigned right shift by a constant (even when the shift-amount operand is a
                // signed `i32` or narrower integer type that `widths` does not track).
                if matches!(op, BinaryOp::Shr | BinaryOp::ShrUnchecked) {
                    if let Some(OpKind::Const(ConstTree::Scalar {
                        bits: shift_raw,
                        bytes,
                    })) = self.def_kind(func, self.canon(*r))
                    {
                        let shift = (mask(*shift_raw, *bytes) as u32) & (bits - 1);
                        let l_iv = self.interval_in(*l, block).unwrap_or_else(|| Iv::full(max));
                        return Some(Iv {
                            lo: l_iv.lo >> shift,
                            hi: l_iv.hi >> shift,
                        });
                    }
                }
                let (l_iv, r_iv) = (self.interval_in(*l, block)?, self.interval_in(*r, block)?);
                let mut out = binary_transfer(*op, l_iv, r_iv, max);
                // Even dividend modulo even constant `m >= 2` is always even, so `<= m - 2`.
                // Wrapping arithmetic modulo `2^bits` preserves parity (since `2^bits` is even).
                if *op == BinaryOp::Rem
                    && r_iv.lo == r_iv.hi
                    && r_iv.lo >= 2
                    && (r_iv.lo & 1) == 0
                    && self.is_even_unsigned(func, table, *l, 0)
                {
                    out.hi = out.hi.min(r_iv.lo - 2);
                    out.lo = out.lo.min(out.hi);
                }
                Some(out)
            }
            OpKind::Deref(ptr) => self
                .const_array_element_interval(func, arena_facts, *ptr, max)
                .or(top),
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v,
                from,
                ..
            } => match table.int_info(*from) {
                Some((false, _)) => {
                    let iv = self.interval_in(*v, block)?;
                    if iv.hi <= max {
                        Some(iv)
                    } else {
                        top
                    }
                }
                _ => top,
            },
            _ => top,
        }
    }

    /// Whether an unsigned integer value is provably even (low bit 0). Wrapping arithmetic modulo
    /// `2^bits` (`bits >= 1`) preserves parity, so no overflow precondition is required.
    fn is_even_unsigned(&self, func: &Func, table: &TypeTable, v: ValueId, depth: u8) -> bool {
        if depth >= 6 {
            return false;
        }
        let canon = self.canon(v);
        match self.def_kind(func, canon) {
            Some(OpKind::Const(ConstTree::Scalar { bits: val, bytes })) => {
                (mask(*val, *bytes) & 1) == 0
            }
            Some(OpKind::Copy(src) | OpKind::Move(src)) => {
                self.is_even_unsigned(func, table, *src, depth + 1)
            }
            Some(OpKind::Binary { op, l, r, ty }) if matches!(table.int_info(*ty), Some((false, bits)) if bits >= 1) =>
            {
                let bits = table.int_info(*ty).map_or(32, |(_, b)| b);
                match op {
                    BinaryOp::Add
                    | BinaryOp::AddUnchecked
                    | BinaryOp::Sub
                    | BinaryOp::SubUnchecked => {
                        self.is_even_unsigned(func, table, *l, depth + 1)
                            && self.is_even_unsigned(func, table, *r, depth + 1)
                    }
                    BinaryOp::Mul | BinaryOp::MulUnchecked => {
                        self.is_even_unsigned(func, table, *l, depth + 1)
                            || self.is_even_unsigned(func, table, *r, depth + 1)
                    }
                    BinaryOp::Shl | BinaryOp::ShlUnchecked => {
                        matches!(
                            self.def_kind(func, self.canon(*r)),
                            Some(OpKind::Const(ConstTree::Scalar { bits: s, bytes }))
                                if ((mask(*s, *bytes) as u32) & (bits - 1)) >= 1
                        )
                    }
                    BinaryOp::Rem => {
                        self.is_even_unsigned(func, table, *l, depth + 1)
                            && self.is_even_unsigned(func, table, *r, depth + 1)
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Computes `[min, max]` over all elements when `ptr` indexes a constant or immutable-static
    /// scalar array/slice.
    fn const_array_element_interval(
        &self,
        func: &Func,
        arena_facts: Option<&ProgramBoundsFacts>,
        ptr: ValueId,
        max: u128,
    ) -> Option<Iv> {
        let (root_val, path) = match self.def_kind(func, self.canon(ptr))? {
            OpKind::BorrowView {
                root: Root::Value(v),
                path,
                ..
            }
            | OpKind::RawView {
                root: Root::Value(v),
                path,
                ..
            } => (*v, path),
            _ => return None,
        };
        let valid_path = matches!(
            path.as_slice(),
            [Proj::Index(_)] | [Proj::Deref, Proj::Index(_)]
        );
        if !valid_path {
            return None;
        }
        let OpKind::Const(root_tree) = self.def_kind(func, self.canon(root_val))? else {
            return None;
        };
        let mut tree: &ConstTree = root_tree;
        while let ConstTree::Reference(inner) = tree {
            tree = inner.as_ref();
        }
        // Immutable static unsigned scalar arrays/slices (e.g. `fflate::tables::CLIM`).
        if let Some(af) = arena_facts {
            match tree {
                ConstTree::Global {
                    target: GlobalReference::Static { definition, .. },
                    offset: 0,
                }
                | ConstTree::StaticSlice {
                    target: GlobalReference::Static { definition, .. },
                    ..
                } => {
                    let is_root_slice = match tree {
                        ConstTree::StaticSlice { path, .. } => path.is_empty(),
                        _ => true,
                    };
                    if is_root_slice {
                        if let Some(&(lo, hi)) = af.static_array_bounds.get(definition) {
                            return Some(Iv {
                                lo: lo.min(max),
                                hi: hi.min(max),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        let items = match tree {
            ConstTree::Aggregate(items) | ConstTree::Slice(items) if !items.is_empty() => items,
            _ => return None,
        };
        let mut lo = u128::MAX;
        let mut hi = 0u128;
        for item in items {
            let ConstTree::Scalar { bits: val, bytes } = item else {
                return None;
            };
            let v = mask(*val, *bytes).min(max);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        Some(Iv { lo, hi })
    }

    /// The length bound of a pointer's metadata: a slice of elements of size `elem_size >= 1`,
    /// or `str`, cannot be longer than `isize::MAX / elem_size`, which certifies `+ k` and
    /// `* elem_size` arithmetic without overflow.
    fn metadata_bound(&self, func: &Func, table: &TypeTable, pointer: ValueId, bits: u32) -> Iv {
        let full = Iv::full(width_max(bits));
        let ty = table.peeled(func.values[pointer.0 as usize].ty);
        let pointee = match table.get(ty).map(|descriptor| &descriptor.kind) {
            Some(TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. }) => {
                table.peeled(*pointee)
            }
            _ => return full,
        };
        let elem_size: Option<u64> = match table.get(pointee).map(|descriptor| &descriptor.kind) {
            Some(TypeKind::Slice(element)) => {
                table.get(table.peeled(*element)).and_then(|descriptor| {
                    if descriptor.layout.zst {
                        None
                    } else {
                        Some(descriptor.layout.size.unwrap_or(1).max(1))
                    }
                })
            }
            Some(TypeKind::Str) => Some(1),
            _ => None,
        };
        if let Some(es) = elem_size {
            if bits >= 2 {
                return Iv {
                    lo: 0,
                    hi: width_max(bits - 1) / u128::from(es),
                };
            }
        }
        full
    }

    /// The exact/bounded length exposed by a slice view or subslice.
    ///
    /// This is intentionally a short structural lens, not symbolic algebra: fixed subslices have
    /// their literal width, end-relative subslices subtract their two fixed trims, and whole
    /// reborrows preserve the source. The construction's own legality ensures a reachable
    /// end-relative view has enough source elements; saturating the lower endpoint keeps the
    /// interval conservative even before that guard is visible here.
    /// Peels slice-length-preserving constructors and wrappers off `pointer`.
    ///
    /// Returns `Ok(len)` when a length-supplying op (`SliceView` or sysroot `slice_from_parts`)
    /// is reached, or `Err(root)` with the innermost slice pointer value after peeling copies,
    /// whole-slice reborrows, lossless slice casts, and sysroot `byte_slice_ref` calls.
    fn peel_slice_source(
        &self,
        func: &Func,
        table: &TypeTable,
        interns: Option<&Interns>,
        pointer: ValueId,
        expected_bits: Option<u32>,
    ) -> Result<ValueId, ValueId> {
        let mut pointer = self.canon(pointer);
        for _ in 0..8 {
            let ptr_ty = func.values[pointer.0 as usize].ty;
            match self.def_kind(func, pointer) {
                Some(OpKind::SliceView { len, .. }) => return Ok(self.canon(*len)),
                Some(OpKind::Aggregate { ty, fields, .. })
                    if fields.len() == 2 && is_slice_pointer(table, *ty) =>
                {
                    return Ok(self.canon(fields[1]));
                }
                Some(OpKind::Call {
                    callee:
                        Callee::Runtime {
                            member,
                            sysroot: true,
                        },
                    semantic: CallSemantic::RawParts,
                    args,
                    ..
                }) if args.len() == 2
                    && is_slice_pointer(table, ptr_ty)
                    && expected_bits.is_none_or(|b| self.widths[args[1].0 as usize] == Some(b))
                    && self.widths[args[1].0 as usize].is_some()
                    && interns.is_none_or(|i| i.str(*member) == Some("slice_from_parts")) =>
                {
                    return Ok(self.canon(args[1]));
                }
                Some(OpKind::Call {
                    callee:
                        Callee::Runtime {
                            member,
                            sysroot: true,
                        },
                    semantic: CallSemantic::Ordinary | CallSemantic::RawParts,
                    args,
                    ..
                }) if args.len() == 1
                    && same_slice_pointer(table, ptr_ty, func.values[args[0].0 as usize].ty)
                    && interns.is_none_or(|i| i.str(*member) == Some("byte_slice_ref")) =>
                {
                    pointer = self.canon(args[0]);
                }
                Some(
                    OpKind::BorrowView {
                        root: Root::Value(source),
                        path,
                        ..
                    }
                    | OpKind::RawView {
                        root: Root::Value(source),
                        path,
                        ..
                    },
                ) if path.as_slice() == [Proj::Deref]
                    && is_slice_pointer(table, ptr_ty)
                    && is_slice_pointer(table, func.values[source.0 as usize].ty) =>
                {
                    pointer = self.canon(*source);
                }
                Some(OpKind::Cast {
                    kind:
                        CastKind::PtrToPtr
                        | CastKind::PointerCoercion
                        | CastKind::Subtype
                        | CastKind::Transmute,
                    v: source,
                    from,
                    to,
                }) if same_slice_or_str_pointer(table, *to, *from) => {
                    pointer = self.canon(*source);
                }
                Some(OpKind::Copy(source) | OpKind::Move(source)) => {
                    pointer = self.canon(*source);
                }
                _ => break,
            }
        }
        Err(pointer)
    }

    fn metadata_interval(
        &self,
        func: &Func,
        table: &TypeTable,
        interns: Option<&Interns>,
        arena_facts: Option<&ProgramBoundsFacts>,
        pointer: ValueId,
        bits: u32,
    ) -> Iv {
        let max = width_max(bits);
        let mut pointer = self.canon(pointer);
        let mut trim = 0_u128;
        for _ in 0..8 {
            if let Some(&c) = self.vec_const_len.get(&pointer) {
                return Iv::point(c.saturating_sub(trim).min(max));
            }
            if let Some(&(_, shl_bits)) = self.vec_one_shl_len.get(&pointer) {
                let hi = if shl_bits >= 2 && shl_bits <= bits {
                    1u128 << (shl_bits - 1)
                } else {
                    max
                };
                return Iv {
                    lo: 1u128.saturating_sub(trim),
                    hi: hi.saturating_sub(trim),
                };
            }
            let ptr_ty = func.values[pointer.0 as usize].ty;
            match self.def_kind(func, pointer) {
                Some(OpKind::Const(ConstTree::StaticSlice { length, .. })) => {
                    return Iv::point(u128::from(*length).saturating_sub(trim).min(max));
                }
                Some(OpKind::Const(ConstTree::Slice(items))) => {
                    return Iv::point((items.len() as u128).saturating_sub(trim).min(max));
                }
                Some(OpKind::Const(ConstTree::Global {
                    target: GlobalReference::Static { definition, .. },
                    offset: 0,
                })) => {
                    if let Some(&len) =
                        arena_facts.and_then(|af| af.static_array_lens.get(definition))
                    {
                        return Iv::point(len.saturating_sub(trim).min(max));
                    }
                }
                Some(OpKind::SliceView { len, .. }) => {
                    let Some(mut interval) = self.interval(*len) else {
                        return self.metadata_bound(func, table, pointer, bits);
                    };
                    interval.lo = interval.lo.saturating_sub(trim);
                    interval.hi = interval.hi.saturating_sub(trim);
                    return interval;
                }
                Some(OpKind::Call {
                    callee:
                        Callee::Runtime {
                            member,
                            sysroot: true,
                        },
                    semantic: CallSemantic::RawParts,
                    args,
                    ..
                }) if args.len() == 2
                    && is_slice_pointer(table, ptr_ty)
                    && self.widths[args[1].0 as usize] == Some(bits)
                    && interns.is_none_or(|i| i.str(*member) == Some("slice_from_parts")) =>
                {
                    let Some(mut interval) = self.interval(args[1]) else {
                        return self.metadata_bound(func, table, pointer, bits);
                    };
                    interval.lo = interval.lo.saturating_sub(trim);
                    interval.hi = interval.hi.saturating_sub(trim);
                    return interval;
                }
                Some(OpKind::Call {
                    callee:
                        Callee::Runtime {
                            member,
                            sysroot: true,
                        },
                    semantic: CallSemantic::Ordinary | CallSemantic::RawParts,
                    args,
                    ..
                }) if args.len() == 1
                    && same_slice_pointer(table, ptr_ty, func.values[args[0].0 as usize].ty)
                    && interns.is_none_or(|i| i.str(*member) == Some("byte_slice_ref")) =>
                {
                    pointer = self.canon(args[0]);
                }
                Some(
                    OpKind::BorrowView {
                        root: Root::Value(source),
                        path,
                        ..
                    }
                    | OpKind::RawView {
                        root: Root::Value(source),
                        path,
                        ..
                    },
                ) => match path.as_slice() {
                    [Proj::Deref] => pointer = self.canon(*source),
                    [Proj::Deref, Proj::Subslice {
                        from,
                        to,
                        from_end: false,
                    }] => {
                        return Iv::point(u128::from(to.saturating_sub(*from)).min(max));
                    }
                    [Proj::Deref, Proj::Subslice {
                        from,
                        to,
                        from_end: true,
                    }] => {
                        let Some(next) = trim.checked_add(u128::from(*from) + u128::from(*to))
                        else {
                            return Iv::full(max);
                        };
                        trim = next;
                        pointer = self.canon(*source);
                    }
                    _ => break,
                },
                Some(OpKind::Cast {
                    kind:
                        CastKind::PtrToPtr
                        | CastKind::PointerCoercion
                        | CastKind::Subtype
                        | CastKind::Transmute,
                    v: source,
                    from,
                    to,
                }) if same_slice_or_str_pointer(table, *to, *from) => {
                    pointer = self.canon(*source);
                }
                Some(OpKind::Copy(source) | OpKind::Move(source)) => {
                    pointer = self.canon(*source);
                }
                _ => break,
            }
        }
        let mut interval = self.metadata_bound(func, table, pointer, bits);
        interval.lo = interval.lo.saturating_sub(trim);
        interval.hi = interval.hi.saturating_sub(trim);
        interval
    }

    /// Peels a chain of `+ constant` / `- constant` off `v` where the intervals at `block` certify
    /// no step can wrap: the result is `(base, offset)` with `v == base + offset` as mathematical
    /// integers.
    fn peel(&self, func: &Func, table: &TypeTable, block: RBlockId, v: ValueId) -> (ValueId, i128) {
        let mut at = self.canon(v);
        let mut offset = 0i128;
        for _ in 0..8 {
            let Some(OpKind::Binary { op, l, r, ty }) = self.def_kind(func, at) else {
                break;
            };
            let Some((false, bits)) = table.int_info(*ty) else {
                break;
            };
            let max = width_max(bits);
            let constant = |side: ValueId| match self.def_kind(func, self.canon(side)) {
                Some(OpKind::Const(ConstTree::Scalar { bits: value, bytes })) => {
                    Some(mask(*value, *bytes).min(max))
                }
                _ => None,
            };
            match op {
                BinaryOp::Add | BinaryOp::AddUnchecked => {
                    let (base, k) = match (constant(*l), constant(*r)) {
                        (None, Some(k)) => (*l, k),
                        (Some(k), None) => (*r, k),
                        _ => break,
                    };
                    let Some(iv) = self.interval_in(self.canon(base), block) else {
                        break;
                    };
                    let Some(sum) = iv.hi.checked_add(k) else {
                        break;
                    };
                    if sum > max {
                        break;
                    }
                    let Ok(k_i) = i128::try_from(k) else { break };
                    let Some(total) = offset.checked_add(k_i) else {
                        break;
                    };
                    offset = total;
                    at = self.canon(base);
                }
                BinaryOp::Sub | BinaryOp::SubUnchecked => {
                    let Some(k) = constant(*r) else { break };
                    if constant(*l).is_some() {
                        break;
                    }
                    let Some(iv) = self.interval_in(self.canon(*l), block) else {
                        break;
                    };
                    if iv.lo < k {
                        break;
                    }
                    let Ok(k_i) = i128::try_from(k) else { break };
                    let Some(total) = offset.checked_sub(k_i) else {
                        break;
                    };
                    offset = total;
                    at = self.canon(*l);
                }
                _ => break,
            }
        }
        (at, offset)
    }

    /// Infers downward-monotone loop parameters: a loop header param `p` whose non-backedge
    /// incoming arguments all satisfy a dominating fact `arg < L` (or `arg <= L`) and whose
    /// backedge incoming arguments decrement `p` by a non-underflowing constant (`p - k` with
    /// `p >= k` at the backedge) maintains `p < L` (or `p <= L`) inside the loop!
    fn compute_downward_bounds(&mut self, func: &Func, table: &TypeTable) {
        let mut incoming: Vec<Vec<(RBlockId, Vec<ValueId>)>> = vec![Vec::new(); func.blocks.len()];
        for b in &func.blocks {
            for (target, args) in b.term.edges() {
                incoming[target.0 as usize].push((b.id, args.to_vec()));
            }
        }
        let term_facts: Vec<Vec<usize>> = (0..func.blocks.len())
            .map(|b| self.term_facts(RBlockId(b as u32)))
            .collect();
        let term_ne_facts: Vec<Vec<usize>> = (0..func.blocks.len())
            .map(|b| self.term_ne_facts(RBlockId(b as u32)))
            .collect();

        for &header in &self.doms.rpo {
            if header.0 == 0 {
                continue;
            }
            let h = header.0 as usize;
            if incoming[h].len() < 2 {
                continue;
            }
            for (pos, &param) in func.blocks[h].params.iter().enumerate() {
                let Some(bits) = self.widths[param.0 as usize] else {
                    continue;
                };
                let max = width_max(bits);
                let param_c = self.canon(param);
                let mut entry_args: Vec<(RBlockId, ValueId)> = Vec::new();
                let mut has_back = false;
                let mut back_ok = true;

                for (pred, args) in &incoming[h] {
                    let Some(&arg) = args.get(pos) else {
                        back_ok = false;
                        break;
                    };
                    let is_back = dominates_block(&self.doms, header, *pred);
                    if !is_back {
                        entry_args.push((*pred, arg));
                        continue;
                    }
                    has_back = true;
                    let arg_c = self.canon(arg);
                    if arg_c == param_c {
                        continue;
                    }
                    // Backedge must be `param - k` with `param >= k` proven at `pred`!
                    let dec_ok = match self.def_kind(func, arg_c) {
                        Some(OpKind::Binary {
                            op: BinaryOp::Sub | BinaryOp::SubUnchecked,
                            l,
                            r,
                            ty,
                        }) if matches!(table.int_info(*ty), Some((false, _)))
                            && self.canon(*l) == param_c =>
                        {
                            match self.def_kind(func, self.canon(*r)) {
                                Some(OpKind::Const(ConstTree::Scalar { bits: kb, bytes })) => {
                                    let k = mask(*kb, *bytes).min(max);
                                    k >= 1
                                        && self
                                            .interval_under(
                                                param,
                                                &term_facts[pred.0 as usize],
                                                &term_ne_facts[pred.0 as usize],
                                            )
                                            .is_some_and(|iv| iv.lo >= k)
                                }
                                _ => false,
                            }
                        }
                        _ => false,
                    };
                    if !dec_ok {
                        back_ok = false;
                        break;
                    }
                }

                if !has_back || !back_ok || entry_args.is_empty() {
                    continue;
                }

                // Check if every entry edge shares a dominating upper bound `(len_canon, strict)`
                // (either via a dominating `Fact` on `entry_arg`, or `entry_arg == len_canon`!).
                let Some((first_pred, first_arg)) = entry_args.first().copied() else {
                    continue;
                };
                let first_c = self.canon(first_arg);
                // Candidate 1: `first_arg` is bounded by a dominating `Fact` at `first_pred`.
                for &fi in &term_facts[first_pred.0 as usize] {
                    let fact = &self.facts[fi];
                    let (Bound::Value(fx), Bound::Value(fy)) = (fact.x, fact.y) else {
                        continue;
                    };
                    if self.canon(fx) != first_c {
                        continue;
                    }
                    let (fact_block, fact_cond) = match fact.site {
                        Site::Edge { target, cond } => (target, cond),
                        Site::Assert { block, cond, .. } => (block, cond),
                    };
                    if !dominates_block(&self.doms, fact_block, header) {
                        continue;
                    }
                    let len_c = self.canon(fy);
                    let strict = fact.rel == Rel::Lt;
                    let all_match = entry_args.iter().skip(1).all(|&(ep, ea)| {
                        let eac = self.canon(ea);
                        eac == first_c
                            || term_facts[ep.0 as usize].iter().any(|&efi| {
                                let ef = &self.facts[efi];
                                matches!((ef.x, ef.y), (Bound::Value(x), Bound::Value(y))
                                    if self.canon(x) == eac
                                        && self.canon(y) == len_c
                                        && (!strict || ef.rel == Rel::Lt))
                            })
                    });
                    if all_match {
                        self.downward_bounds.push(DownwardBound {
                            param: param_c,
                            header,
                            len_canon: len_c,
                            strict,
                            fact_block,
                            fact_cond,
                        });
                    }
                }
                // Candidate 2: every entry edge passes the EXACT same canonical value `L` (e.g.
                // `let mut k = s; while k > 0 { k -= 1; a[k]; }`): then `param <= L` holds in
                // `header`, so in any `param >= 1` branch `param - 1 < L`!
                if entry_args.iter().all(|&(_, ea)| self.canon(ea) == first_c) {
                    // Find a dominating `param > 0` / `param != 0` switch edge in a child of `header`
                    // at prove-time; record `fact_block = header` as a sentinel marker and attach
                    // the real `Switch` cond from the `param >= 1` guard at prove-time.
                    self.downward_bounds.push(DownwardBound {
                        param: param_c,
                        header,
                        len_canon: first_c,
                        strict: false,
                        fact_block: header,
                        fact_cond: param,
                    });
                }
            }
        }
    }

    /// Computes sound element upper bounds (`< L`, `<= L`, or `<= L + 1`) for local non-escaping
    /// `Vec<u32|usize|u16|u8, Global>` roots — BOTH `CellId`s and PEA-unboxed SSA `ValueId`s
    /// (e.g. `earcut::refine::{ht_sl, he_sl, stack_sl}`, `fflate::h_tree::{node_left, node_right}`,
    /// `d3-hierarchy::circles::siblings::{next, prev}`, `minisearch::order`).
    fn compute_local_bounded_vecs(&mut self, func: &Func, table: &TypeTable) {
        let mut root_borrows: BTreeMap<ValueId, LocalVecRoot> = BTreeMap::new();
        let mut slice_to_root: BTreeMap<ValueId, LocalVecRoot> = BTreeMap::new();
        let mut elem_ref_to_root: BTreeMap<ValueId, LocalVecRoot> = BTreeMap::new();
        let mut init_zeroed_block: BTreeMap<LocalVecRoot, (RBlockId, bool)> = BTreeMap::new();
        let mut cell_stores: BTreeMap<CellId, usize> = BTreeMap::new();
        let mut disq: BTreeSet<LocalVecRoot> = BTreeSet::new();

        let is_unsigned_vec = |ty: TypeId| {
            global_vec_elem_ty(table, ty)
                .and_then(|e| table.int_info(table.peeled(e)))
                .is_some_and(|(signed, _)| !signed)
        };
        let init_kind_of = |v: ValueId| -> Option<bool> {
            let cv = self.canon(v);
            match self.def_kind(func, cv) {
                Some(OpKind::Call {
                    semantic:
                        CallSemantic::Standard(
                            StandardOperation::VecNew
                            | StandardOperation::NativeVecNew
                            | StandardOperation::VecWithCapacity
                            | StandardOperation::NativeVecWithCapacity,
                        ),
                    ..
                }) => Some(true),
                Some(OpKind::Call {
                    semantic: CallSemantic::Standard(StandardOperation::NativeVecFromElemZeroed),
                    ..
                }) => Some(false),
                Some(OpKind::Call {
                    semantic: CallSemantic::Standard(StandardOperation::VecFromElem),
                    args,
                    ..
                }) if !args.is_empty()
                    && matches!(
                        self.def_kind(func, self.canon(args[0])),
                        Some(OpKind::Const(ConstTree::Scalar { bits: 0, .. }))
                    ) =>
                {
                    Some(false)
                }
                _ => None,
            }
        };

        for (ci, cell) in func.cells.iter().enumerate() {
            let cid = CellId(ci as u32);
            if !is_unsigned_vec(cell.ty) {
                disq.insert(LocalVecRoot::Cell(cid));
            }
        }

        // First pass: collect unboxed SSA Vec roots, cell Vec roots, borrows, slices, element refs,
        // and check escape.
        for &b in &self.doms.rpo {
            for op in &func.blocks[b.0 as usize].ops {
                if let Some(res) = op.result {
                    if is_unsigned_vec(func.values[res.0 as usize].ty) {
                        if let Some(is_empty) = init_kind_of(res) {
                            init_zeroed_block.insert(LocalVecRoot::Value(res), (b, is_empty));
                        }
                    }
                }
                match &op.kind {
                    OpKind::RawView { root, .. } => match root {
                        Root::Cell(c) => {
                            disq.insert(LocalVecRoot::Cell(*c));
                        }
                        Root::Value(v) => {
                            disq.insert(LocalVecRoot::Value(*v));
                            if let Some(&r) = root_borrows
                                .get(v)
                                .or_else(|| slice_to_root.get(v))
                                .or_else(|| elem_ref_to_root.get(v))
                            {
                                disq.insert(r);
                            }
                        }
                    },
                    OpKind::CellStore { cell, v } if !disq.contains(&LocalVecRoot::Cell(*cell)) => {
                        // Moving a freshly constructed SSA Vec into a cell transfers tracking to the cell.
                        disq.insert(LocalVecRoot::Value(*v));
                        let cnt = cell_stores.entry(*cell).or_insert(0);
                        *cnt += 1;
                        if *cnt > 1 {
                            disq.insert(LocalVecRoot::Cell(*cell));
                        } else if let Some(is_empty) = init_kind_of(*v) {
                            init_zeroed_block.insert(LocalVecRoot::Cell(*cell), (b, is_empty));
                        } else {
                            disq.insert(LocalVecRoot::Cell(*cell));
                        }
                    }
                    OpKind::CellLoad(c) if !disq.contains(&LocalVecRoot::Cell(*c)) => {
                        disq.insert(LocalVecRoot::Cell(*c));
                    }
                    OpKind::BorrowView { root, path, .. } => {
                        if let Some(res) = op.result {
                            match (root, path.as_slice()) {
                                (Root::Cell(c), []) if !disq.contains(&LocalVecRoot::Cell(*c)) => {
                                    root_borrows.insert(res, LocalVecRoot::Cell(*c));
                                }
                                (Root::Cell(c), _) => {
                                    disq.insert(LocalVecRoot::Cell(*c));
                                }
                                (Root::Value(v), [])
                                    if init_zeroed_block.contains_key(&LocalVecRoot::Value(*v))
                                        && !disq.contains(&LocalVecRoot::Value(*v)) =>
                                {
                                    root_borrows.insert(res, LocalVecRoot::Value(*v));
                                }
                                (Root::Value(v), [] | [Proj::Deref]) => {
                                    if let Some(&r) = slice_to_root.get(v) {
                                        slice_to_root.insert(res, r);
                                    } else if let Some(&r) = root_borrows.get(v) {
                                        root_borrows.insert(res, r);
                                    } else if let Some(&r) = elem_ref_to_root.get(v) {
                                        elem_ref_to_root.insert(res, r);
                                    }
                                }
                                (
                                    Root::Value(v),
                                    [Proj::Deref, Proj::Index(_)]
                                    | [Proj::Deref, Proj::ConstantIndex { .. }],
                                ) => {
                                    if let Some(&r) = slice_to_root.get(v) {
                                        elem_ref_to_root.insert(res, r);
                                    }
                                }
                                (Root::Value(v), _) => {
                                    disq.insert(LocalVecRoot::Value(*v));
                                    if let Some(&r) = slice_to_root
                                        .get(v)
                                        .or_else(|| root_borrows.get(v))
                                        .or_else(|| elem_ref_to_root.get(v))
                                    {
                                        disq.insert(r);
                                    }
                                }
                            }
                        }
                    }
                    OpKind::Copy(s) | OpKind::Move(s) => {
                        if let Some(res) = op.result {
                            if let Some(&r) = root_borrows.get(s) {
                                root_borrows.insert(res, r);
                            }
                            if let Some(&r) = slice_to_root.get(s) {
                                slice_to_root.insert(res, r);
                            }
                            if let Some(&r) = elem_ref_to_root.get(s) {
                                elem_ref_to_root.insert(res, r);
                            }
                        }
                    }
                    OpKind::Call {
                        semantic:
                            CallSemantic::Standard(
                                StandardOperation::VecAsSlice
                                | StandardOperation::NativeVecAsSlice
                                | StandardOperation::VecAsMutSlice
                                | StandardOperation::NativeVecAsMutSlice,
                            ),
                        args,
                        ..
                    } => {
                        if let (Some(res), Some(&a0)) = (op.result, args.first()) {
                            if let Some(&r) = root_borrows.get(&a0) {
                                slice_to_root.insert(res, r);
                            }
                        }
                    }
                    OpKind::Call {
                        semantic:
                            CallSemantic::Standard(
                                StandardOperation::VecIndex | StandardOperation::VecIndexMut,
                            ),
                        args,
                        ..
                    } => {
                        if let (Some(res), Some(&a0)) = (op.result, args.first()) {
                            if let Some(&r) = root_borrows.get(&a0) {
                                elem_ref_to_root.insert(res, r);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Second pass: collect all writes/pushes and disqualify roots escaping to unknown ops.
        let mut root_writes: BTreeMap<LocalVecRoot, Vec<(RBlockId, usize, ValueId)>> =
            BTreeMap::new();
        let mut load_to_root: BTreeMap<ValueId, LocalVecRoot> = BTreeMap::new();
        for &b in &self.doms.rpo {
            let blk = &func.blocks[b.0 as usize];
            for u in blk.term.uses() {
                if init_zeroed_block.contains_key(&LocalVecRoot::Value(u)) {
                    disq.insert(LocalVecRoot::Value(u));
                }
                if let Some(&r) = root_borrows
                    .get(&u)
                    .or_else(|| slice_to_root.get(&u))
                    .or_else(|| elem_ref_to_root.get(&u))
                {
                    disq.insert(r);
                }
            }
            for (oi, op) in blk.ops.iter().enumerate() {
                match &op.kind {
                    OpKind::Deref(ptr) => {
                        if let (Some(res), Some(&r)) = (op.result, elem_ref_to_root.get(ptr)) {
                            load_to_root.insert(res, r);
                            load_to_root.insert(self.canon(res), r);
                        }
                    }
                    OpKind::StoreThrough { ptr, v } => {
                        if let Some(&r) = elem_ref_to_root.get(ptr) {
                            root_writes.entry(r).or_default().push((b, oi, *v));
                        } else if let Some(&r) =
                            root_borrows.get(ptr).or_else(|| slice_to_root.get(ptr))
                        {
                            disq.insert(r);
                        }
                    }
                    OpKind::Call { semantic, args, .. } => {
                        let first_root = args
                            .first()
                            .and_then(|a| root_borrows.get(a).or_else(|| slice_to_root.get(a)))
                            .copied();
                        let allowed_on_first = matches!(
                            semantic,
                            CallSemantic::Standard(
                                StandardOperation::VecAsSlice
                                    | StandardOperation::NativeVecAsSlice
                                    | StandardOperation::VecAsMutSlice
                                    | StandardOperation::NativeVecAsMutSlice
                                    | StandardOperation::VecLen
                                    | StandardOperation::VecIsEmpty
                                    | StandardOperation::VecIndex
                                    | StandardOperation::VecIndexMut
                                    | StandardOperation::VecPush
                                    | StandardOperation::NativeVecPush
                                    | StandardOperation::VecPop
                                    | StandardOperation::NativeVecPop
                                    | StandardOperation::VecClear
                                    | StandardOperation::VecTruncate
                                    | StandardOperation::VecReserve
                                    | StandardOperation::VecReserveExact
                                    | StandardOperation::NativeVecReserve
                                    | StandardOperation::NativeVecReserveExact
                                    | StandardOperation::SliceSwap
                                    | StandardOperation::SliceSort
                                    | StandardOperation::SliceSortCmp
                                    | StandardOperation::VecDrop
                            )
                        );
                        if let Some(r) = first_root {
                            if !allowed_on_first {
                                disq.insert(r);
                            } else if matches!(
                                semantic,
                                CallSemantic::Standard(
                                    StandardOperation::VecPush | StandardOperation::NativeVecPush
                                )
                            ) && args.len() >= 2
                            {
                                root_writes.entry(r).or_default().push((b, oi, args[1]));
                            }
                        }
                        for (ai, &a) in args.iter().enumerate() {
                            if ai == 0 && first_root.is_some() {
                                continue;
                            }
                            if init_zeroed_block.contains_key(&LocalVecRoot::Value(a)) {
                                disq.insert(LocalVecRoot::Value(a));
                            }
                            if let Some(&r) = root_borrows
                                .get(&a)
                                .or_else(|| slice_to_root.get(&a))
                                .or_else(|| elem_ref_to_root.get(&a))
                            {
                                disq.insert(r);
                            }
                        }
                    }
                    OpKind::Unary {
                        op: UnaryOp::PtrMetadata,
                        ..
                    }
                    | OpKind::Copy(_)
                    | OpKind::Move(_)
                    | OpKind::BorrowView { .. }
                    | OpKind::Drop { .. } => {}
                    _ => {
                        for u in op.kind.uses() {
                            if init_zeroed_block.contains_key(&LocalVecRoot::Value(u)) {
                                disq.insert(LocalVecRoot::Value(u));
                            }
                            if let Some(&r) = root_borrows
                                .get(&u)
                                .or_else(|| slice_to_root.get(&u))
                                .or_else(|| elem_ref_to_root.get(&u))
                            {
                                disq.insert(r);
                            }
                        }
                    }
                }
            }
        }

        let surviving_roots: Vec<LocalVecRoot> = init_zeroed_block
            .keys()
            .copied()
            .filter(|r| !disq.contains(r) && root_writes.contains_key(r))
            .collect();
        if surviving_roots.is_empty() {
            return;
        }

        // Collect candidate target lengths (all `PtrMetadata` / `VecLen` canonical values used in
        // BoundsCheck asserts in this function!).
        let mut target_lens: BTreeSet<ValueId> = BTreeSet::new();
        for b in &func.blocks {
            for op in &b.ops {
                if let OpKind::Assert {
                    expected: true,
                    msg: RAssertMessage::BoundsCheck { length, .. },
                    ..
                } = &op.kind
                {
                    target_lens.insert(self.canon(*length));
                }
            }
        }
        if target_lens.is_empty() {
            return;
        }

        // Fixpoint (up to 3 rounds so `ht_sl` -> `su < n` feeds into `he_sl` and `stack_sl`!).
        let mut proven: BTreeMap<LocalVecRoot, (ValueId, LocalElemBound)> = BTreeMap::new();
        for _round in 0..3 {
            let mut changed = false;
            for &root in &surviving_roots {
                if proven.contains_key(&root) {
                    continue;
                }
                let Some(&(init_b, is_empty)) = init_zeroed_block.get(&root) else {
                    continue;
                };
                let Some(writes) = root_writes.get(&root) else {
                    continue;
                };
                'len_loop: for &len_c in &target_lens {
                    for bound in [
                        LocalElemBound::Lt,
                        LocalElemBound::Le,
                        LocalElemBound::LePlusOne,
                    ] {
                        if !is_empty && bound == LocalElemBound::Lt {
                            let len_pos =
                                self.interval_in(len_c, init_b).is_some_and(|iv| iv.lo >= 1);
                            if !len_pos {
                                continue;
                            }
                        }
                        let len_iv_max_ok = match bound {
                            LocalElemBound::Lt => true,
                            LocalElemBound::Le => {
                                let w = self.widths[len_c.0 as usize].unwrap_or(32);
                                self.interval(len_c).is_some_and(|iv| iv.hi < width_max(w))
                            }
                            LocalElemBound::LePlusOne => {
                                let w = self.widths[len_c.0 as usize].unwrap_or(32);
                                self.interval(len_c)
                                    .is_some_and(|iv| iv.hi + 1 < width_max(w))
                            }
                        };
                        if !len_iv_max_ok {
                            continue;
                        }
                        let all_writes_ok = writes.iter().all(|&(wb, woi, wv)| {
                            self.local_vec_write_satisfies(
                                func,
                                table,
                                root,
                                wb,
                                woi,
                                wv,
                                len_c,
                                bound,
                                &load_to_root,
                                &proven,
                            )
                        });
                        if all_writes_ok {
                            proven.insert(root, (len_c, bound));
                            changed = true;
                            break 'len_loop;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        for (vec_root, (len_canon, bound)) in proven {
            let idx = self.local_bounded_vecs.len();
            self.local_bounded_vecs.push(LocalBoundedVecFact {
                vec_root,
                len_canon,
                bound,
            });
            for (&lv, &lr) in &load_to_root {
                if lr == vec_root {
                    self.load_to_local_vec.insert(lv, idx);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn local_vec_write_satisfies(
        &self,
        func: &Func,
        table: &TypeTable,
        self_root: LocalVecRoot,
        block: RBlockId,
        op_idx: usize,
        val: ValueId,
        len_c: ValueId,
        bound: LocalElemBound,
        load_to_root: &BTreeMap<ValueId, LocalVecRoot>,
        proven: &BTreeMap<LocalVecRoot, (ValueId, LocalElemBound)>,
    ) -> bool {
        let cv = self.canon(val);
        let check_inner = |inner: ValueId| -> bool {
            let ic = self.canon(inner);
            // Co-inductive: copying from the same vec preserves its bound!
            if load_to_root.get(&ic) == Some(&self_root) {
                return true;
            }
            // Loaded from another already-proven vec with the same `len_c`!
            if let Some(&other_r) = load_to_root.get(&ic) {
                if let Some(&(ol, ob)) = proven.get(&other_r) {
                    if ol == len_c
                        && matches!(
                            (ob, bound),
                            (LocalElemBound::Lt, _)
                                | (
                                    LocalElemBound::Le,
                                    LocalElemBound::Le | LocalElemBound::LePlusOne
                                )
                                | (LocalElemBound::LePlusOne, LocalElemBound::LePlusOne)
                        )
                    {
                        return true;
                    }
                }
            }
            let active_facts: Vec<usize> = self.entry_facts[block.0 as usize]
                .iter()
                .copied()
                .chain(self.facts.iter().enumerate().filter_map(|(i, f)| {
                    matches!(f.site, Site::Assert { block: fb, index: fi, .. } if fb == block && fi < op_idx)
                        .then_some(i)
                }))
                .collect();
            let active_ne: Vec<usize> = self.entry_ne_facts[block.0 as usize]
                .iter()
                .copied()
                .chain(self.ne_facts.iter().enumerate().filter_map(|(i, f)| {
                    matches!(f.site, Site::Assert { block: fb, index: fi, .. } if fb == block && fi < op_idx)
                        .then_some(i)
                }))
                .collect();

            // Constant check:
            if let Some(OpKind::Const(ConstTree::Scalar { bits, bytes })) = self.def_kind(func, ic)
            {
                let max = self.widths[ic.0 as usize].map_or(u128::MAX, width_max);
                let c = mask(*bits, *bytes).min(max);
                let len_lo = self
                    .interval_under(len_c, &active_facts, &active_ne)
                    .map_or(0, |iv| iv.lo);
                return match bound {
                    LocalElemBound::Lt => c < len_lo,
                    LocalElemBound::Le => c <= len_lo,
                    LocalElemBound::LePlusOne => c <= len_lo.saturating_add(1),
                };
            }

            // Helper: is `x` proven `< len_c` at `(block, op_idx)`?
            let is_lt_len = |x: ValueId| -> bool {
                let xc = self.canon(x);
                if let (Some(xiv), Some(liv)) = (
                    self.interval_under(xc, &active_facts, &active_ne),
                    self.interval_under(len_c, &active_facts, &active_ne),
                ) {
                    if xiv.hi < liv.lo {
                        return true;
                    }
                }
                if active_facts.iter().any(|&fi| {
                    let f = &self.facts[fi];
                    matches!((f.x, f.y), (Bound::Value(fx), Bound::Value(fy))
                        if self.canon(fx) == xc && self.canon(fy) == len_c && f.rel == Rel::Lt)
                }) {
                    return true;
                }
                // Or `x` is `load - k` from an already-proven `LocalBoundedVec` (e.g. `su = slot - 2`)!
                if let Some(OpKind::Binary {
                    op: BinaryOp::Sub | BinaryOp::SubUnchecked,
                    l,
                    r,
                    ty,
                }) = self.def_kind(func, xc)
                {
                    if matches!(table.int_info(*ty), Some((false, _))) {
                        if let Some(OpKind::Const(ConstTree::Scalar { bits: kb, bytes })) =
                            self.def_kind(func, self.canon(*r))
                        {
                            let k = mask(*kb, *bytes);
                            let lc = self.canon(*l);
                            if let Some(&or) = load_to_root.get(&lc) {
                                if let Some(&(ol, ob)) = proven.get(&or) {
                                    if ol == len_c {
                                        let liv =
                                            self.interval_under(lc, &active_facts, &active_ne);
                                        if (ob == LocalElemBound::Le
                                            && k == 1
                                            && liv.is_some_and(|iv| iv.lo >= 1))
                                            || (ob == LocalElemBound::LePlusOne
                                                && k == 2
                                                && liv.is_some_and(|iv| iv.lo >= 2))
                                        {
                                            return true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // Or `x` is directly loaded from an already-proven `Lt` vec!
                if let Some(&or) = load_to_root.get(&xc) {
                    if proven.get(&or) == Some(&(len_c, LocalElemBound::Lt)) {
                        return true;
                    }
                }
                false
            };

            match bound {
                LocalElemBound::Lt => is_lt_len(ic),
                LocalElemBound::Le => {
                    if is_lt_len(ic) {
                        return true;
                    }
                    if let Some(OpKind::Binary {
                        op: BinaryOp::Add | BinaryOp::AddUnchecked,
                        l,
                        r,
                        ty,
                    }) = self.def_kind(func, ic)
                    {
                        if matches!(table.int_info(*ty), Some((false, _))) {
                            let is_one = |v: ValueId| {
                                matches!(
                                    self.def_kind(func, self.canon(v)),
                                    Some(OpKind::Const(ConstTree::Scalar { bits: 1, .. }))
                                )
                            };
                            if (is_one(*r) && is_lt_len(*l)) || (is_one(*l) && is_lt_len(*r)) {
                                return true;
                            }
                        }
                    }
                    false
                }
                LocalElemBound::LePlusOne => {
                    if is_lt_len(ic) {
                        return true;
                    }
                    if let Some(OpKind::Binary {
                        op: BinaryOp::Add | BinaryOp::AddUnchecked,
                        l,
                        r,
                        ty,
                    }) = self.def_kind(func, ic)
                    {
                        if matches!(table.int_info(*ty), Some((false, _))) {
                            let is_two = |v: ValueId| {
                                matches!(
                                    self.def_kind(func, self.canon(v)),
                                    Some(OpKind::Const(ConstTree::Scalar { bits: 2, .. }))
                                )
                            };
                            if (is_two(*r) && is_lt_len(*l)) || (is_two(*l) && is_lt_len(*r)) {
                                return true;
                            }
                        }
                    }
                    false
                }
            }
        };

        if check_inner(cv) {
            return true;
        }
        if let Some(OpKind::Cast {
            kind: CastKind::IntToInt,
            v: src,
            from,
            to,
        }) = self.def_kind(func, cv)
        {
            if matches!(
                (table.int_info(*from), table.int_info(*to)),
                (Some((false, _)), Some((false, _)))
            ) {
                return check_inner(*src);
            }
        }
        false
    }

    /// Consumes whole-program [`ProgramBoundsFacts`] (`arenas`, `param_arena_indices`,
    /// `return_arena_index`, `return_vec_elem_bound`) to populate `arena_len_handle`,
    /// `arena_val_origin`, and `ret_vec_elem_len`.
    fn compute_arena_origins(
        &mut self,
        func: &Func,
        table: &TypeTable,
        interns: Option<&Interns>,
        af: &ProgramBoundsFacts,
    ) {
        let adt_def = |ty: TypeId| -> Option<DefinitionId> {
            match table.get(table.peeled(ty)).map(|d| &d.kind) {
                Some(TypeKind::Adt { def, .. }) => Some(*def),
                _ => None,
            }
        };
        let slice_def = |ty: TypeId| -> Option<DefinitionId> {
            let pt = value::pointee_of(table, ty)?;
            match table.get(table.peeled(pt)).map(|d| &d.kind) {
                Some(TypeKind::Slice(e)) => adt_def(*e),
                _ => None,
            }
        };
        let vec_def = |ty: TypeId| -> Option<DefinitionId> {
            let e = global_vec_elem_ty(table, ty)?;
            adt_def(e)
        };
        let vec_ref_def = |ty: TypeId| -> Option<DefinitionId> {
            let pt = value::pointee_of(table, ty)?;
            vec_def(pt)
        };

        let mut val_arena: BTreeMap<ValueId, (DefinitionId, ArenaHandle)> = BTreeMap::new();
        let mut elem_ref_arena: BTreeMap<ValueId, (DefinitionId, ArenaHandle)> = BTreeMap::new();
        let mut field_ref_arena: BTreeMap<ValueId, (DefinitionId, u32, ArenaHandle)> =
            BTreeMap::new();
        let mut inner_vec_slice_arena: BTreeMap<ValueId, (DefinitionId, u32, ArenaHandle)> =
            BTreeMap::new();
        let mut local_vec_cells: BTreeMap<CellId, DefinitionId> = BTreeMap::new();
        // Track `ReturnVecElemBound`-returned `Vec` values/cells/slices -> `(target_slice_len_c, callee)`.
        let mut ret_vec_bound_val: BTreeMap<ValueId, (ValueId, ItemId)> = BTreeMap::new();
        let mut ret_vec_bound_cell: BTreeMap<CellId, (ValueId, ItemId)> = BTreeMap::new();
        let mut ret_vec_bound_elem_ref: BTreeMap<ValueId, (ValueId, ItemId)> = BTreeMap::new();

        for (ci, cell) in func.cells.iter().enumerate() {
            let cid = CellId(ci as u32);
            if let Some(def) = vec_def(cell.ty) {
                if af.arenas.contains_key(&def) {
                    local_vec_cells.insert(cid, def);
                }
            }
        }
        for (pi, &pv) in func.params.iter().enumerate() {
            let pty = func.values[pv.0 as usize].ty;
            if let Some(def) = slice_def(pty)
                .or_else(|| vec_ref_def(pty))
                .or_else(|| vec_def(pty))
            {
                if af.arenas.contains_key(&def) {
                    val_arena.insert(pv, (def, ArenaHandle::Param(pi)));
                }
            }
        }
        // Seed certified `ParamArenaIndex` entry params.
        if let Some(plist) = af.param_arena_indices.get(&func.item) {
            for p in plist {
                if let Some(&pv) = func.params.get(p.idx_param) {
                    let handle = match p.holder_field {
                        Some(hf) => ArenaHandle::ParamHolder(p.arena_param, hf),
                        None => ArenaHandle::Param(p.arena_param),
                    };
                    self.arena_val_origin.insert(
                        self.canon(pv),
                        ArenaValOrigin {
                            elem_def: p.elem_def,
                            handle,
                            sentinel: p.sentinel,
                            source: ArenaIndexSource::Param {
                                idx_param: p.idx_param,
                            },
                        },
                    );
                }
            }
        }

        // Track holder struct references (`&Holder`, `&mut Holder`, `Holder`) through reborrows &
        // copies so inlined `&mut self` accesses like `(*reborrowed_self).nodes` resolve cleanly.
        #[derive(Clone, Copy, Eq, PartialEq)]
        enum HolderRoot {
            Param(usize),
            Cell(CellId),
        }
        let mut holder_ref: BTreeMap<ValueId, HolderRoot> = BTreeMap::new();
        for (pi, &pv) in func.params.iter().enumerate() {
            let pty = func.values[pv.0 as usize].ty;
            let base = value::pointee_of(table, pty).unwrap_or(pty);
            if matches!(
                table.get(table.peeled(base)).map(|d| &d.kind),
                Some(TypeKind::Adt {
                    form: rcgjs_object::jir::types::NominalForm::Struct,
                    ..
                })
            ) {
                holder_ref.insert(pv, HolderRoot::Param(pi));
            }
        }

        // Precompute incoming edges per block for propagating arena handles & cursor origins.
        let mut incoming: Vec<Vec<(RBlockId, Vec<ValueId>)>> = vec![Vec::new(); func.blocks.len()];
        for b in &func.blocks {
            for (target, args) in b.term.edges() {
                incoming[target.0 as usize].push((b.id, args.to_vec()));
            }
        }

        // Propagate arena handles, holder refs, slice lengths, element/field refs, and
        // `ReturnVecElemBound` in RPO (including across block parameters!).
        for _ in 0..3 {
            for &b in &self.doms.rpo {
                if b.0 != 0 {
                    let bi = b.0 as usize;
                    let edges = &incoming[bi];
                    if !edges.is_empty() {
                        for (pos, &param) in func.blocks[bi].params.iter().enumerate() {
                            let pc = self.canon(param);
                            // Propagate `holder_ref` across block params.
                            if !holder_ref.contains_key(&param) {
                                let mut first: Option<HolderRoot> = None;
                                let mut ok = true;
                                for (_, args) in edges {
                                    let Some(&arg) = args.get(pos) else {
                                        ok = false;
                                        break;
                                    };
                                    let ac = self.canon(arg);
                                    if ac == pc {
                                        continue;
                                    }
                                    if let Some(&hr) =
                                        holder_ref.get(&arg).or_else(|| holder_ref.get(&ac))
                                    {
                                        match first {
                                            None => first = Some(hr),
                                            Some(prev) if prev == hr => {}
                                            Some(_) => {
                                                ok = false;
                                                break;
                                            }
                                        }
                                    } else {
                                        ok = false;
                                        break;
                                    }
                                }
                                if ok {
                                    if let Some(hr) = first {
                                        holder_ref.insert(param, hr);
                                        holder_ref.insert(pc, hr);
                                    }
                                }
                            }
                            // Propagate `val_arena` across block params.
                            if !val_arena.contains_key(&param) {
                                let mut first: Option<(DefinitionId, ArenaHandle)> = None;
                                let mut ok = true;
                                for (_, args) in edges {
                                    let Some(&arg) = args.get(pos) else {
                                        ok = false;
                                        break;
                                    };
                                    let ac = self.canon(arg);
                                    if ac == pc {
                                        continue;
                                    }
                                    if let Some(&h) =
                                        val_arena.get(&arg).or_else(|| val_arena.get(&ac))
                                    {
                                        match first {
                                            None => first = Some(h),
                                            Some(prev) if prev == h => {}
                                            Some(_) => {
                                                ok = false;
                                                break;
                                            }
                                        }
                                    } else {
                                        ok = false;
                                        break;
                                    }
                                }
                                if ok {
                                    if let Some(h) = first {
                                        val_arena.insert(param, h);
                                        val_arena.insert(pc, h);
                                    }
                                }
                            }
                        }
                    }
                }
                for op in &func.blocks[b.0 as usize].ops {
                    if let OpKind::CellStore { cell, v } = &op.kind {
                        if let Some(&rb) = ret_vec_bound_val
                            .get(v)
                            .or_else(|| ret_vec_bound_val.get(&self.canon(*v)))
                        {
                            ret_vec_bound_cell.insert(*cell, rb);
                        }
                    }
                    let Some(res) = op.result else { continue };
                    let res_c = self.canon(res);
                    match &op.kind {
                        OpKind::Copy(s) | OpKind::Move(s) => {
                            if let Some(&hr) = holder_ref.get(s) {
                                holder_ref.insert(res, hr);
                            }
                            if let Some(&h) = val_arena.get(s) {
                                val_arena.insert(res, h);
                            }
                            if let Some(&h) = elem_ref_arena.get(s) {
                                elem_ref_arena.insert(res, h);
                            }
                            if let Some(&h) = field_ref_arena.get(s) {
                                field_ref_arena.insert(res, h);
                            }
                            if let Some(&h) = inner_vec_slice_arena.get(s) {
                                inner_vec_slice_arena.insert(res, h);
                            }
                            if let Some(&rb) = ret_vec_bound_val.get(s) {
                                ret_vec_bound_val.insert(res, rb);
                            }
                            if let Some(&rb) = ret_vec_bound_elem_ref.get(s) {
                                ret_vec_bound_elem_ref.insert(res, rb);
                            }
                        }
                        OpKind::Cast {
                            kind: CastKind::Subtype | CastKind::PointerCoercion,
                            v,
                            ..
                        } => {
                            if let Some(&hr) = holder_ref.get(v) {
                                holder_ref.insert(res, hr);
                            }
                            if let Some(&h) = val_arena.get(v) {
                                val_arena.insert(res, h);
                            }
                            if let Some(&h) = elem_ref_arena.get(v) {
                                elem_ref_arena.insert(res, h);
                            }
                            if let Some(&h) = field_ref_arena.get(v) {
                                field_ref_arena.insert(res, h);
                            }
                            if let Some(&rb) = ret_vec_bound_val.get(v) {
                                ret_vec_bound_val.insert(res, rb);
                            }
                        }
                        OpKind::CellLoad(c) => {
                            if let Some(&def) = local_vec_cells.get(c) {
                                val_arena.insert(res, (def, ArenaHandle::VecCell(*c)));
                            }
                            if let Some(&rb) = ret_vec_bound_cell.get(c) {
                                ret_vec_bound_val.insert(res, rb);
                            }
                        }
                        OpKind::BorrowView { root, path, .. } => match (root, path.as_slice()) {
                            (Root::Cell(c), []) => {
                                holder_ref.insert(res, HolderRoot::Cell(*c));
                                if let Some(&def) = local_vec_cells.get(c) {
                                    val_arena.insert(res, (def, ArenaHandle::VecCell(*c)));
                                }
                                if let Some(&rb) = ret_vec_bound_cell.get(c) {
                                    ret_vec_bound_val.insert(res, rb);
                                }
                            }
                            (Root::Cell(c), [Proj::Field { field, ty }]) => {
                                if let Some(def) = vec_def(*ty) {
                                    if af.arenas.contains_key(&def) {
                                        val_arena.insert(
                                            res,
                                            (def, ArenaHandle::HolderCell(*c, *field)),
                                        );
                                    }
                                }
                            }
                            (Root::Value(v), [] | [Proj::Deref]) => {
                                let vc = self.canon(*v);
                                if let Some(&hr) = holder_ref.get(v).or_else(|| holder_ref.get(&vc))
                                {
                                    holder_ref.insert(res, hr);
                                }
                                if let Some(&h) = val_arena.get(v).or_else(|| val_arena.get(&vc)) {
                                    val_arena.insert(res, h);
                                } else if let Some(&h) =
                                    elem_ref_arena.get(v).or_else(|| elem_ref_arena.get(&vc))
                                {
                                    elem_ref_arena.insert(res, h);
                                } else if let Some(&h) =
                                    field_ref_arena.get(v).or_else(|| field_ref_arena.get(&vc))
                                {
                                    field_ref_arena.insert(res, h);
                                } else if let Some(&h) = inner_vec_slice_arena
                                    .get(v)
                                    .or_else(|| inner_vec_slice_arena.get(&vc))
                                {
                                    inner_vec_slice_arena.insert(res, h);
                                } else if let Some(&rb) = ret_vec_bound_val
                                    .get(v)
                                    .or_else(|| ret_vec_bound_val.get(&vc))
                                {
                                    ret_vec_bound_val.insert(res, rb);
                                }
                            }
                            (
                                Root::Value(v),
                                [Proj::Deref, Proj::Field { field, ty }]
                                | [Proj::Field { field, ty }],
                            ) => {
                                let vc = self.canon(*v);
                                if let Some(def) = vec_def(*ty) {
                                    if af.arenas.contains_key(&def) {
                                        if let Some(&hr) =
                                            holder_ref.get(v).or_else(|| holder_ref.get(&vc))
                                        {
                                            let handle = match hr {
                                                HolderRoot::Param(pi) => {
                                                    ArenaHandle::ParamHolder(pi, *field)
                                                }
                                                HolderRoot::Cell(c) => {
                                                    ArenaHandle::HolderCell(c, *field)
                                                }
                                            };
                                            val_arena.insert(res, (def, handle));
                                        }
                                    }
                                }
                                if let Some(&(def, handle)) =
                                    elem_ref_arena.get(v).or_else(|| elem_ref_arena.get(&vc))
                                {
                                    field_ref_arena.insert(res, (def, *field, handle));
                                }
                            }
                            (
                                Root::Value(v),
                                [Proj::Deref, Proj::Index(_)]
                                | [Proj::Deref, Proj::ConstantIndex { .. }],
                            ) => {
                                let vc = self.canon(*v);
                                if let Some(&h) = val_arena.get(v).or_else(|| val_arena.get(&vc)) {
                                    elem_ref_arena.insert(res, h);
                                } else if let Some(&(def, field, handle)) = inner_vec_slice_arena
                                    .get(v)
                                    .or_else(|| inner_vec_slice_arena.get(&vc))
                                {
                                    field_ref_arena.insert(res, (def, field, handle));
                                } else if let Some(&rb) = ret_vec_bound_val
                                    .get(v)
                                    .or_else(|| ret_vec_bound_val.get(&vc))
                                {
                                    ret_vec_bound_elem_ref.insert(res, rb);
                                }
                            }
                            (
                                Root::Value(v),
                                [Proj::Deref, Proj::Index(_), Proj::Field { field, .. }]
                                | [Proj::Deref, Proj::ConstantIndex { .. }, Proj::Field { field, .. }],
                            ) => {
                                let vc = self.canon(*v);
                                if let Some(&(def, handle)) =
                                    val_arena.get(v).or_else(|| val_arena.get(&vc))
                                {
                                    field_ref_arena.insert(res, (def, *field, handle));
                                }
                            }
                            _ => {}
                        },
                        OpKind::Unary {
                            op: UnaryOp::PtrMetadata,
                            v,
                            ..
                        } => {
                            if let Some(&h) =
                                val_arena.get(v).or_else(|| val_arena.get(&self.canon(*v)))
                            {
                                let list = self.arena_len_handle.entry(res_c).or_default();
                                if !list.contains(&h) {
                                    list.push(h);
                                }
                            }
                        }
                        OpKind::Field { base, field, .. } => {
                            let bty = func.values[base.0 as usize].ty;
                            if interns.is_some_and(|i| {
                                is_direct_global_vec_ty(table, bty)
                                    && is_vec_len_field(table, i, bty, *field)
                            }) {
                                if let Some(&h) = val_arena.get(base) {
                                    let list = self.arena_len_handle.entry(res_c).or_default();
                                    if !list.contains(&h) {
                                        list.push(h);
                                    }
                                }
                            } else if let Some(&(def, handle)) = elem_ref_arena.get(base) {
                                if let Some(inv) = af.arenas.get(&def) {
                                    if let Some(&fk) = inv.fields.get(field) {
                                        match fk {
                                            ArenaFieldKind::AlwaysValid => {
                                                self.arena_val_origin.insert(
                                                    res_c,
                                                    ArenaValOrigin {
                                                        elem_def: def,
                                                        handle,
                                                        sentinel: None,
                                                        source: ArenaIndexSource::Field(*field),
                                                    },
                                                );
                                            }
                                            ArenaFieldKind::ValidOrSentinel { sentinel } => {
                                                self.arena_val_origin.insert(
                                                    res_c,
                                                    ArenaValOrigin {
                                                        elem_def: def,
                                                        handle,
                                                        sentinel: Some(sentinel),
                                                        source: ArenaIndexSource::Field(*field),
                                                    },
                                                );
                                            }
                                            ArenaFieldKind::VecOfValidIndices => {}
                                        }
                                    }
                                }
                            }
                        }
                        OpKind::Deref(ptr) => {
                            if let Some(&h) = elem_ref_arena.get(ptr) {
                                elem_ref_arena.insert(res, h);
                            } else if let Some(&(def, field, handle)) = field_ref_arena.get(ptr) {
                                if let Some(inv) = af.arenas.get(&def) {
                                    if let Some(&fk) = inv.fields.get(&field) {
                                        let (sentinel, source) = match fk {
                                            ArenaFieldKind::AlwaysValid => {
                                                (None, ArenaIndexSource::Field(field))
                                            }
                                            ArenaFieldKind::ValidOrSentinel { sentinel } => {
                                                (Some(sentinel), ArenaIndexSource::Field(field))
                                            }
                                            ArenaFieldKind::VecOfValidIndices => {
                                                (None, ArenaIndexSource::VecField(field))
                                            }
                                        };
                                        self.arena_val_origin.insert(
                                            res_c,
                                            ArenaValOrigin {
                                                elem_def: def,
                                                handle,
                                                sentinel,
                                                source,
                                            },
                                        );
                                    }
                                }
                            } else if let Some(&(len_c, callee)) = ret_vec_bound_elem_ref.get(ptr) {
                                self.ret_vec_elem_len.insert(res_c, (len_c, callee));
                            }
                        }
                        OpKind::Call {
                            callee,
                            semantic,
                            args,
                            ..
                        } => match semantic {
                            CallSemantic::Standard(
                                StandardOperation::VecAsSlice
                                | StandardOperation::NativeVecAsSlice
                                | StandardOperation::VecAsMutSlice
                                | StandardOperation::NativeVecAsMutSlice,
                            ) if !args.is_empty() => {
                                if let Some(&h) = val_arena.get(&args[0]) {
                                    val_arena.insert(res, h);
                                } else if let Some(&fh) = field_ref_arena.get(&args[0]) {
                                    inner_vec_slice_arena.insert(res, fh);
                                } else if let Some(&rb) = ret_vec_bound_val.get(&args[0]) {
                                    ret_vec_bound_val.insert(res, rb);
                                }
                            }
                            CallSemantic::Standard(StandardOperation::VecLen)
                                if !args.is_empty() =>
                            {
                                if let Some(&h) = val_arena.get(&args[0]) {
                                    let list = self.arena_len_handle.entry(res_c).or_default();
                                    if !list.contains(&h) {
                                        list.push(h);
                                    }
                                }
                            }
                            CallSemantic::Standard(
                                StandardOperation::VecIndex | StandardOperation::VecIndexMut,
                            ) if !args.is_empty() => {
                                if let Some(&h) = val_arena.get(&args[0]) {
                                    elem_ref_arena.insert(res, h);
                                } else if let Some(&fh) = field_ref_arena.get(&args[0]) {
                                    field_ref_arena.insert(res, fh);
                                } else if let Some(&rb) = ret_vec_bound_val.get(&args[0]) {
                                    ret_vec_bound_elem_ref.insert(res, rb);
                                }
                            }
                            CallSemantic::Ordinary => {
                                if let Callee::Direct { item, .. } = callee {
                                    if let Some(ret_idx) = af.return_arena_index.get(item) {
                                        if let Some(&arg_a) = args.get(ret_idx.arena_param) {
                                            let matched_handle = match ret_idx.holder_field {
                                                None => val_arena
                                                    .get(&arg_a)
                                                    .filter(|&&(d, _)| d == ret_idx.elem_def)
                                                    .map(|&(_, h)| h),
                                                Some(hf) => holder_ref
                                                    .get(&arg_a)
                                                    .or_else(|| holder_ref.get(&self.canon(arg_a)))
                                                    .map(|&hr| match hr {
                                                        HolderRoot::Param(pi) => {
                                                            ArenaHandle::ParamHolder(pi, hf)
                                                        }
                                                        HolderRoot::Cell(c) => {
                                                            ArenaHandle::HolderCell(c, hf)
                                                        }
                                                    }),
                                            };
                                            if let Some(handle) = matched_handle {
                                                self.arena_val_origin.insert(
                                                    res_c,
                                                    ArenaValOrigin {
                                                        elem_def: ret_idx.elem_def,
                                                        handle,
                                                        sentinel: ret_idx.sentinel,
                                                        source: ArenaIndexSource::CallReturn {
                                                            callee: *item,
                                                        },
                                                    },
                                                );
                                            }
                                        }
                                    }
                                    if let Some(ret_eb) = af.return_vec_elem_bound.get(item) {
                                        let crate::analysis::arena::ReturnVecElemBound::LtParamSliceLen {
                                            slice_param,
                                        } = *ret_eb;
                                        if let Some(&sa) = args.get(slice_param) {
                                            // Find any `PtrMetadata` of `sa` (canonical slice len)
                                            // in `func`.
                                            let sa_c = self.canon(sa);
                                            if let Some(len_c) =
                                                self.find_slice_len_canon(func, sa_c)
                                            {
                                                ret_vec_bound_val.insert(res, (len_c, *item));
                                            }
                                            // Also link to `val_arena` if `sa` is a known arena!
                                            if let Some(&(ed, eh)) = val_arena.get(&sa) {
                                                if let Some(len_c) =
                                                    self.find_slice_len_canon(func, sa_c)
                                                {
                                                    let list = self
                                                        .arena_len_handle
                                                        .entry(len_c)
                                                        .or_default();
                                                    if !list.contains(&(ed, eh)) {
                                                        list.push((ed, eh));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
        }

        // Propagate `arena_val_origin` across block parameters via a sound greatest fixpoint!
        // Handles arbitrary loop-carried `if/else` merge cursors (e.g. `earcut_linked`,
        // `eliminate_hole`, `split_earcut`, `filter_points_indexed`) where mutual cycles between
        // header params and merge-block params would deadlock a naive least fixpoint.
        let term_ne: Vec<Vec<usize>> = (0..func.blocks.len())
            .map(|b| self.term_ne_facts(RBlockId(b as u32)))
            .collect();
        // Step A: Collect all distinct `(elem_def, handle)` pairs active in this function.
        let mut active_arenas: Vec<(DefinitionId, ArenaHandle)> = Vec::new();
        for orig in self.arena_val_origin.values() {
            let pair = (orig.elem_def, orig.handle);
            if !active_arenas.contains(&pair) {
                active_arenas.push(pair);
            }
        }
        // Step B: For each active `(elem_def, handle)`, run a greatest fixpoint over all unsigned
        // non-entry block parameters not already in `arena_val_origin`!
        let base_origins = self.arena_val_origin.clone();
        for (target_def, target_handle) in active_arenas {
            // Seed every unsigned non-entry block param not in `base_origins` optimistically as
            // `AlwaysValid` (`sentinel = None`), then iteratively weaken/remove any param whose
            // incoming edges aren't ALL valid for `(target_def, target_handle)`.
            let mut param_state: BTreeMap<ValueId, Option<u128>> = BTreeMap::new();
            let mut param_loc: Vec<(RBlockId, usize, ValueId, ValueId)> = Vec::new();
            for &b in &self.doms.rpo {
                if b.0 == 0 || incoming[b.0 as usize].is_empty() {
                    continue;
                }
                for (pos, &param) in func.blocks[b.0 as usize].params.iter().enumerate() {
                    if self.widths[param.0 as usize].is_none() {
                        continue;
                    }
                    let pc = self.canon(param);
                    if base_origins.contains_key(&pc) || self.arena_val_origin.contains_key(&pc) {
                        continue;
                    }
                    param_state.insert(pc, None);
                    param_loc.push((b, pos, param, pc));
                }
            }
            if param_state.is_empty() {
                continue;
            }
            loop {
                let mut changed = false;
                for &(b, pos, _param, pc) in &param_loc {
                    let Some(&cur_sent) = param_state.get(&pc) else {
                        continue;
                    };
                    let mut merged_sent = cur_sent;
                    let mut ok = true;
                    for (pred, args) in &incoming[b.0 as usize] {
                        let Some(&arg) = args.get(pos) else {
                            ok = false;
                            break;
                        };
                        let ac = self.canon(arg);
                        if ac == pc {
                            continue;
                        }
                        let edge_sent = if let Some(&orig) = base_origins.get(&ac) {
                            if orig.elem_def != target_def || orig.handle != target_handle {
                                ok = false;
                                break;
                            }
                            orig.sentinel.filter(|&s| {
                                !term_ne[pred.0 as usize].iter().any(|&ni| {
                                    self.canon(self.ne_facts[ni].v) == ac
                                        && self.ne_facts[ni].c == s
                                })
                            })
                        } else if let Some(&ps) = param_state.get(&ac) {
                            ps.filter(|&s| {
                                !term_ne[pred.0 as usize].iter().any(|&ni| {
                                    self.canon(self.ne_facts[ni].v) == ac
                                        && self.ne_facts[ni].c == s
                                })
                            })
                        } else if let Some(OpKind::Const(ConstTree::Scalar { bits, bytes })) =
                            self.def_kind(func, ac)
                        {
                            let max = self.widths[ac.0 as usize].map_or(u128::MAX, width_max);
                            Some(mask(*bits, *bytes).min(max))
                        } else {
                            ok = false;
                            break;
                        };
                        match (merged_sent, edge_sent) {
                            (_, None) => {}
                            (None, Some(s)) => merged_sent = Some(s),
                            (Some(x), Some(y)) if x == y => {}
                            _ => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if !ok {
                        param_state.remove(&pc);
                        changed = true;
                    } else if merged_sent != cur_sent {
                        param_state.insert(pc, merged_sent);
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }
            // Also verify every surviving param is actually grounded (reachable from at least one
            // `base_origins` value — prevents pure constant cycles with no real arena origin!).
            let mut grounded: BTreeSet<ValueId> = base_origins
                .iter()
                .filter(|(_, o)| o.elem_def == target_def && o.handle == target_handle)
                .map(|(&k, _)| k)
                .collect();
            loop {
                let mut g_changed = false;
                for &(b, pos, _, pc) in &param_loc {
                    if !param_state.contains_key(&pc) || grounded.contains(&pc) {
                        continue;
                    }
                    let has_grounded = incoming[b.0 as usize].iter().any(|(_, args)| {
                        args.get(pos)
                            .is_some_and(|&a| grounded.contains(&self.canon(a)))
                    });
                    if has_grounded && grounded.insert(pc) {
                        g_changed = true;
                    }
                }
                if !g_changed {
                    break;
                }
            }
            for &(_, _, param, pc) in &param_loc {
                if grounded.contains(&pc) {
                    if let Some(&sent) = param_state.get(&pc) {
                        self.arena_val_origin.insert(
                            pc,
                            ArenaValOrigin {
                                elem_def: target_def,
                                handle: target_handle,
                                sentinel: sent,
                                source: ArenaIndexSource::BlockParam { param },
                            },
                        );
                    }
                }
            }
        }
    }

    fn find_slice_len_canon(&self, func: &Func, slice_canon: ValueId) -> Option<ValueId> {
        for b in &func.blocks {
            for op in &b.ops {
                let Some(res) = op.result else { continue };
                if let OpKind::Unary {
                    op: UnaryOp::PtrMetadata,
                    v,
                    ..
                } = op.kind
                {
                    if self.canon(v) == slice_canon {
                        return Some(self.canon(res));
                    }
                }
            }
        }
        None
    }

    /// Tries to prove the assert at `(block, index)` with condition `cond`; `deleted` names the
    /// asserts already removed, whose fact sites are no longer usable evidence.
    #[allow(clippy::too_many_arguments)]
    fn prove(
        &self,
        func: &Func,
        table: &TypeTable,
        interns: Option<&Interns>,
        arena_facts: Option<&ProgramBoundsFacts>,
        cond: ValueId,
        block: RBlockId,
        index: usize,
        deleted: &BTreeSet<(u32, usize)>,
    ) -> Option<BoundsFact> {
        let Some(OpKind::Binary {
            op: BinaryOp::Lt,
            l: idx,
            r: len,
            ty,
        }) = self.def_kind(func, self.canon(cond))
        else {
            return None;
        };
        if !matches!(table.int_info(*ty), Some((false, _))) {
            return None;
        }

        let b = block.0 as usize;
        let local: Vec<usize> = self
            .facts
            .iter()
            .enumerate()
            .filter_map(|(i, fact)| {
                matches!(
                    fact.site,
                    Site::Assert { block: fb, index: fi, .. }
                        if fb == block && fi < index && !deleted.contains(&(fb.0, fi))
                )
                .then_some(i)
            })
            .collect();
        let active_facts: Vec<usize> = self.entry_facts[b]
            .iter()
            .copied()
            .filter(|&i| match self.facts[i].site {
                Site::Assert {
                    block: fb,
                    index: fi,
                    ..
                } => !deleted.contains(&(fb.0, fi)),
                Site::Edge { .. } => true,
            })
            .chain(local.iter().copied())
            .collect();
        let active_ne: Vec<usize> = self.entry_ne_facts[b]
            .iter()
            .copied()
            .filter(|&i| match self.ne_facts[i].site {
                Site::Assert {
                    block: fb,
                    index: fi,
                    ..
                } => !deleted.contains(&(fb.0, fi)),
                Site::Edge { .. } => true,
            })
            .chain(self.ne_facts.iter().enumerate().filter_map(|(i, nf)| {
                matches!(
                    nf.site,
                    Site::Assert { block: fb, index: fi, .. }
                        if fb == block && fi < index && !deleted.contains(&(fb.0, fi))
                )
                .then_some(i)
            }))
            .collect();

        // 1. Numeric: the intervals (refined by dominating entry facts, `!=` disequalities, and
        // earlier non-deleted asserts in this block) alone imply it.
        if let (Some(iv_idx), Some(iv_len)) = (
            self.interval_under(*idx, &active_facts, &active_ne),
            self.interval_under(*len, &active_facts, &active_ne),
        ) {
            if iv_idx.hi < iv_len.lo {
                return Some(BoundsFact::Numeric {
                    index_hi: iv_idx.hi,
                    length_lo: iv_len.lo,
                });
            }
        }

        // 2. Symbolic: a dominating fact over the same base against the same length term.
        let (base, k_index) = self.peel(func, table, block, *idx);
        let length_term = self.canon(*len);
        for &fact_index in &active_facts {
            let fact = &self.facts[fact_index];
            if let Site::Assert {
                block: fb,
                index: fi,
                ..
            } = fact.site
            {
                if deleted.contains(&(fb.0, fi)) {
                    continue;
                }
            }
            let Bound::Value(fact_y) = fact.y else {
                continue;
            };
            if self.canon(fact_y) != length_term {
                continue;
            }
            let offsets = match fact.x {
                Bound::Value(vx) => {
                    let (fact_base, k_fact) = self.peel(func, table, block, vx);
                    if fact_base == base {
                        let implied = match fact.rel {
                            Rel::Lt => k_index <= k_fact,
                            Rel::Le => k_index < k_fact,
                        };
                        if implied {
                            let shift = k_index.min(k_fact);
                            let oi = k_index
                                .checked_sub(shift)
                                .and_then(|v| u128::try_from(v).ok());
                            let of = k_fact
                                .checked_sub(shift)
                                .and_then(|v| u128::try_from(v).ok());
                            oi.zip(of)
                        } else {
                            None
                        }
                    } else if let (Some(iv_idx), Some(iv_fact)) = (
                        self.interval_under(*idx, &active_facts, &active_ne),
                        self.interval_under(vx, &active_facts, &active_ne),
                    ) {
                        let implied = match fact.rel {
                            Rel::Lt => iv_idx.hi <= iv_fact.lo,
                            Rel::Le => iv_idx.hi < iv_fact.lo,
                        };
                        implied.then_some((iv_idx.hi, iv_fact.lo))
                    } else {
                        None
                    }
                }
                Bound::Const(c_fact) => self
                    .interval_under(*idx, &active_facts, &active_ne)
                    .and_then(|iv_idx| {
                        let implied = match fact.rel {
                            Rel::Lt => iv_idx.hi <= c_fact,
                            Rel::Le => iv_idx.hi < c_fact,
                        };
                        implied.then_some((iv_idx.hi, c_fact))
                    }),
            };
            let Some((offset_index, offset_fact)) = offsets else {
                continue;
            };
            if self.effect_between(func, table, interns, fact.site, block, index) {
                continue;
            }
            let (fact_block, fact_cond) = match fact.site {
                Site::Edge { target, cond } => (target, cond),
                Site::Assert { block, cond, .. } => (block, cond),
            };
            return Some(BoundsFact::Dominating {
                block: fact_block,
                cond: fact_cond,
                offset_index,
                offset_fact,
                strict: fact.rel == Rel::Lt,
            });
        }

        // 2b. Downward-monotone loop parameter bound (`base` is a downward-monotone param <= or <
        // `length_term`, e.g. `while j > 0 { a[j - 1]; a[j]; j -= 1; }` or `while k > 0 { k -= 1; a[k]; }`).
        for db in &self.downward_bounds {
            if db.param != base || db.len_canon != length_term {
                continue;
            }
            if !dominates_block(&self.doms, db.header, block) {
                continue;
            }
            let implied = if db.strict { k_index <= 0 } else { k_index < 0 };
            if !implied {
                continue;
            }
            // Pick a live dominating `(fact_block, fact_cond)` witness site.
            let witness_site = if db.fact_block != db.header {
                Some((db.fact_block, db.fact_cond))
            } else {
                // For `k <= len` with `k_index < 0`, find the dominating `k > 0` / `k != 0` guard!
                active_facts
                    .iter()
                    .find_map(|&fi| {
                        let f = &self.facts[fi];
                        matches!(f.y, Bound::Value(y) if self.canon(y) == base).then_some(
                            match f.site {
                                Site::Edge { target, cond } => (target, cond),
                                Site::Assert { block: b, cond, .. } => (b, cond),
                            },
                        )
                    })
                    .or_else(|| {
                        active_ne.iter().find_map(|&ni| {
                            let nf = &self.ne_facts[ni];
                            (self.canon(nf.v) == base && nf.c == 0).then_some(match nf.site {
                                Site::Edge { target, cond } => (target, cond),
                                Site::Assert { block: b, cond, .. } => (b, cond),
                            })
                        })
                    })
            };
            if let Some((fb, fc)) = witness_site {
                let shift = k_index.min(0);
                if let (Ok(oi), Ok(of)) = (u128::try_from(k_index - shift), u128::try_from(-shift))
                {
                    return Some(BoundsFact::Dominating {
                        block: fb,
                        cond: fc,
                        offset_index: oi,
                        offset_fact: of,
                        strict: db.strict,
                    });
                }
            }
        }

        // 3. Symbolic bitmask bound: `idx = x & mask`, where `mask == len - 1` (and `len >= 1`),
        // or `mask == (1 << k) - 1` and `len` has exact `vec_one_shl_len == (k, _)`.
        if let Some(bm) = self.prove_bitmask(func, table, *idx, *len, &active_facts, &active_ne) {
            return Some(bm);
        }

        // 4. Local non-escaping bounded-element `Vec` (`LocalBoundedVec`, e.g. `earcut::refine`,
        // `fflate::h_tree`, `d3-hierarchy::circles::siblings`, `minisearch::order`).
        if let Some(lbv) =
            self.prove_local_bounded_vec(func, table, *idx, length_term, &active_facts, &active_ne)
        {
            return Some(lbv);
        }

        // 5. Whole-program monotone-arena & `ReturnVecElemBound` invariant (`MonotoneArena`).
        if arena_facts.is_some() {
            if let Some(ma) = self.prove_monotone_arena(*idx, length_term, &active_ne) {
                return Some(ma);
            }
        }

        None
    }

    fn prove_bitmask(
        &self,
        func: &Func,
        table: &TypeTable,
        idx: ValueId,
        len: ValueId,
        active_facts: &[usize],
        active_ne: &[usize],
    ) -> Option<BoundsFact> {
        let idx_c = self.canon(idx);
        let len_c = self.canon(len);
        let OpKind::Binary {
            op: BinaryOp::BitAnd,
            l,
            r,
            ty,
        } = self.def_kind(func, idx_c)?
        else {
            return None;
        };
        if !matches!(table.int_info(*ty), Some((false, _))) {
            return None;
        }
        let one_shl_shift = |v: ValueId| -> Option<ValueId> {
            let vc = self.canon(v);
            if let Some(&(sh, _)) = self
                .vec_one_shl_len
                .get(&vc)
                .or_else(|| self.vec_one_shl_len.get(&v))
            {
                return Some(sh);
            }
            if let Some(OpKind::Binary {
                op: BinaryOp::Shl | BinaryOp::ShlUnchecked,
                l: sl,
                r: sr,
                ..
            }) = self.def_kind(func, vc)
            {
                if matches!(
                    self.def_kind(func, self.canon(*sl)),
                    Some(OpKind::Const(ConstTree::Scalar { bits: 1, .. }))
                ) {
                    return Some(self.canon(*sr));
                }
            }
            None
        };
        let len_shl = one_shl_shift(len).or_else(|| one_shl_shift(len_c));

        for mask_side in [*l, *r] {
            let mc = self.canon(mask_side);
            let Some(OpKind::Binary {
                op: BinaryOp::Sub | BinaryOp::SubUnchecked,
                l: sub_l,
                r: sub_r,
                ..
            }) = self.def_kind(func, mc)
            else {
                continue;
            };
            let is_one = matches!(
                self.def_kind(func, self.canon(*sub_r)),
                Some(OpKind::Const(ConstTree::Scalar { bits: 1, .. }))
            );
            if !is_one {
                continue;
            }
            let sub_l_c = self.canon(*sub_l);
            let matches_len = sub_l_c == len_c
                || matches!((one_shl_shift(sub_l_c), len_shl), (Some(a), Some(b)) if a == b);
            if !matches_len {
                continue;
            }
            // Verify `sub_l >= 1`!
            let iv_lo = self
                .interval_under(sub_l_c, active_facts, active_ne)
                .map_or(0, |iv| iv.lo);
            if iv_lo < 1 {
                continue;
            }
            // Find an optional dominating nonzero guard site if `sub_l` isn't statically `1 << k`,
            // constant `>= 1`, or a positive doubling loop.
            let nonzero_site = active_ne
                .iter()
                .find_map(|&ni| {
                    let nf = &self.ne_facts[ni];
                    (self.canon(nf.v) == sub_l_c && nf.c == 0).then_some(match nf.site {
                        Site::Edge { target, cond } => (target, cond),
                        Site::Assert { block: b, cond, .. } => (b, cond),
                    })
                })
                .or_else(|| {
                    active_facts.iter().find_map(|&fi| {
                        let f = &self.facts[fi];
                        let proves_pos = matches!(f.y, Bound::Value(y) if self.canon(y) == sub_l_c)
                            && match (f.x, f.rel) {
                                (Bound::Const(_), Rel::Lt) => true,
                                (Bound::Const(c), Rel::Le) => c >= 1,
                                (Bound::Value(x), Rel::Lt) => self.interval(x).is_some(),
                                (Bound::Value(x), Rel::Le) => {
                                    self.interval(x).is_some_and(|iv| iv.lo >= 1)
                                }
                            };
                        proves_pos.then_some(match f.site {
                            Site::Edge { target, cond } => (target, cond),
                            Site::Assert { block: b, cond, .. } => (b, cond),
                        })
                    })
                });
            return Some(BoundsFact::Bitmask {
                and_val: idx_c,
                mask_val: mc,
                len_val: sub_l_c,
                nonzero_site,
            });
        }
        None
    }

    fn prove_local_bounded_vec(
        &self,
        func: &Func,
        table: &TypeTable,
        idx: ValueId,
        length_term: ValueId,
        active_facts: &[usize],
        active_ne: &[usize],
    ) -> Option<BoundsFact> {
        let idx_c = self.canon(idx);
        // Case 0: direct `load` from a `LocalElemBound::Lt` local Vec (`sub_offset = 0`).
        if let Some(&lvi) = self
            .load_to_local_vec
            .get(&idx)
            .or_else(|| self.load_to_local_vec.get(&idx_c))
        {
            let info = self.local_bounded_vecs[lvi];
            if info.len_canon == length_term && info.bound == LocalElemBound::Lt {
                return Some(BoundsFact::LocalBoundedVec {
                    vec_root: info.vec_root,
                    load_val: idx_c,
                    sub_offset: 0,
                    guard: None,
                });
            }
        }
        // Case 1 & 2: `idx = load - k` (`k == 1` for `Le` with `load >= 1`, `k == 2` for `LePlusOne`
        // with `load >= 2`).
        let OpKind::Binary {
            op: BinaryOp::Sub | BinaryOp::SubUnchecked,
            l,
            r,
            ty,
        } = self.def_kind(func, idx_c)?
        else {
            return None;
        };
        if !matches!(table.int_info(*ty), Some((false, _))) {
            return None;
        }
        let OpKind::Const(ConstTree::Scalar { bits: kb, bytes }) =
            self.def_kind(func, self.canon(*r))?
        else {
            return None;
        };
        let k = mask(*kb, *bytes);
        let lc = self.canon(*l);
        let &lvi = self
            .load_to_local_vec
            .get(l)
            .or_else(|| self.load_to_local_vec.get(&lc))?;
        let info = self.local_bounded_vecs[lvi];
        if info.len_canon != length_term {
            return None;
        }
        let ok = (info.bound == LocalElemBound::Le && k == 1)
            || (info.bound == LocalElemBound::LePlusOne && k == 2);
        if !ok {
            return None;
        }
        if !self
            .interval_under(lc, active_facts, active_ne)
            .is_some_and(|iv| iv.lo >= k)
        {
            return None;
        }
        // Locate dominating guard ensuring `lc >= k` (either `!= (k - 1)` or `<`/`<=` fact).
        let guard = active_ne
            .iter()
            .find_map(|&ni| {
                let nf = &self.ne_facts[ni];
                (self.canon(nf.v) == lc && nf.c == k - 1).then_some(match nf.site {
                    Site::Edge { target, cond } => (target, cond),
                    Site::Assert { block: b, cond, .. } => (b, cond),
                })
            })
            .or_else(|| {
                active_facts.iter().find_map(|&fi| {
                    let f = &self.facts[fi];
                    matches!(f.y, Bound::Value(y) if self.canon(y) == lc).then_some(match f.site {
                        Site::Edge { target, cond } => (target, cond),
                        Site::Assert { block: b, cond, .. } => (b, cond),
                    })
                })
            })?;
        Some(BoundsFact::LocalBoundedVec {
            vec_root: info.vec_root,
            load_val: lc,
            sub_offset: k,
            guard: Some(guard),
        })
    }

    fn prove_monotone_arena(
        &self,
        idx: ValueId,
        length_term: ValueId,
        active_ne: &[usize],
    ) -> Option<BoundsFact> {
        let idx_c = self.canon(idx);
        // 1. `ReturnVecElemBound`: `v` loaded from a certified returned `Vec` (e.g. `walk(nodes)`).
        if let Some(&(target_len, callee)) = self
            .ret_vec_elem_len
            .get(&idx)
            .or_else(|| self.ret_vec_elem_len.get(&idx_c))
        {
            if target_len == length_term {
                return Some(BoundsFact::MonotoneArena {
                    elem_def: DefinitionId {
                        crate_id: 0,
                        local_hash: 0,
                    },
                    sentinel: None,
                    sentinel_guard: None,
                    source: ArenaIndexSource::ReturnVecElem { callee },
                });
            }
        }
        // 2. Proven `ArenaValOrigin` matching `arena_len_handle[length_term]`!
        let orig = self
            .arena_val_origin
            .get(&idx_c)
            .or_else(|| self.arena_val_origin.get(&idx))?;
        let handles = self.arena_len_handle.get(&length_term)?;
        if !handles.contains(&(orig.elem_def, orig.handle)) {
            return None;
        }
        match orig.sentinel {
            None => Some(BoundsFact::MonotoneArena {
                elem_def: orig.elem_def,
                sentinel: None,
                sentinel_guard: None,
                source: orig.source,
            }),
            Some(s) => {
                let guard = active_ne.iter().find_map(|&ni| {
                    let nf = &self.ne_facts[ni];
                    (self.canon(nf.v) == idx_c && nf.c == s).then_some(match nf.site {
                        Site::Edge { target, cond } => (target, cond),
                        Site::Assert { block: b, cond, .. } => (b, cond),
                    })
                })?;
                Some(BoundsFact::MonotoneArena {
                    elem_def: orig.elem_def,
                    sentinel: Some(s),
                    sentinel_guard: Some(guard),
                    source: orig.source,
                })
            }
        }
    }

    /// Whether an op that may observe or write memory sits on a path from the fact's
    /// establishment to the assert that does not re-establish the fact on the way.
    ///
    /// Keeps opaque calls (`CallSemantic::Ordinary`, `Foreign`, `Indirect`, `Virtual`, `Host`,
    /// `Runtime`) blocking, while allowing operations that provably cannot observe or invalidate
    /// SSA scalar/slice-length facts (`Drop(None)`, `CellAlloc`, non-Vec `CellStore`/`StoreThrough`/
    /// `SetField`/`SetDiscriminant`, `Materialize`, `Convert`, and length-preserving standard ops).
    fn effect_between(
        &self,
        func: &Func,
        table: &TypeTable,
        interns: Option<&Interns>,
        site: Site,
        block: RBlockId,
        index: usize,
    ) -> bool {
        let (start, from) = match site {
            Site::Edge { target, .. } => (target, 0usize),
            Site::Assert {
                block, index: at, ..
            } => (block, at + 1),
        };
        let is_pure_intrinsic = |kind: &OpKind| match kind {
            OpKind::Intrinsic { name, .. } => interns.and_then(|i| i.str(*name)).is_some_and(|n| {
                matches!(
                    n,
                    "rotate_left"
                        | "rotate_right"
                        | "wrapping_add"
                        | "wrapping_sub"
                        | "wrapping_mul"
                        | "saturating_add"
                        | "saturating_sub"
                        | "disjoint_bitor"
                        | "add_with_overflow"
                        | "sub_with_overflow"
                        | "mul_with_overflow"
                        | "ctlz"
                        | "cttz"
                        | "ctpop"
                        | "bswap"
                        | "bitreverse"
                )
            }),
            _ => false,
        };
        let is_len_safe_op = |kind: &OpKind| match kind {
            OpKind::Assert { .. }
            | OpKind::Drop {
                glue: DropGlue::None,
                ..
            }
            | OpKind::CellAlloc(_)
            | OpKind::SetDiscriminant { .. }
            | OpKind::Materialize(_)
            | OpKind::Convert { .. } => true,
            OpKind::CellStore { cell, .. } => {
                !is_global_vec_ty(table, func.cells[cell.0 as usize].ty)
            }
            OpKind::SetField { base, .. } => {
                !is_global_vec_ty(table, func.values[base.0 as usize].ty)
            }
            OpKind::StoreThrough { ptr, .. } => {
                value::pointee_of(table, func.values[ptr.0 as usize].ty)
                    .is_none_or(|pt| !is_global_vec_ty(table, pt))
            }
            OpKind::Call { semantic, .. } => is_vec_len_preserving_call(*semantic),
            _ => is_pure_intrinsic(kind),
        };
        let effectful =
            |op: &Op| op_purity(&op.kind) == Purity::Effect && !is_len_safe_op(&op.kind);
        let start_ops = &func.blocks[start.0 as usize].ops;
        if start == block {
            return start_ops[from..index].iter().any(effectful);
        }
        if start_ops[from..].iter().any(effectful) {
            return true;
        }
        let forward = self.flood(func, start, Direction::Forward(start));
        let backward = self.flood(func, start, Direction::Backward(block));
        for candidate in forward.intersection(&backward) {
            if *candidate == block.0 {
                continue;
            }
            if func.blocks[*candidate as usize].ops.iter().any(effectful) {
                return true;
            }
        }
        let assert_ops = &func.blocks[block.0 as usize].ops;
        if assert_ops[..index].iter().any(effectful) {
            return true;
        }
        // The assert block may cycle back to itself without re-establishing, and then its later
        // ops run between two executions of the assert.
        if self
            .flood(func, start, Direction::Forward(block))
            .contains(&block.0)
        {
            return assert_ops[index..].iter().any(effectful);
        }
        false
    }

    /// Flood fill that never traverses `avoid`: forward over successors starting after `from`'s
    /// terminator, or backward over predecessors starting at `from` itself.
    fn flood(&self, func: &Func, avoid: RBlockId, direction: Direction) -> BTreeSet<u32> {
        let mut seen: BTreeSet<u32> = BTreeSet::new();
        let mut queue: VecDeque<u32> = VecDeque::new();
        let push = |next: u32, seen: &mut BTreeSet<u32>, queue: &mut VecDeque<u32>| {
            if next != avoid.0 && seen.insert(next) {
                queue.push_back(next);
            }
        };
        match direction {
            Direction::Forward(from) => {
                for successor in func.blocks[from.0 as usize].term.successors() {
                    push(successor.0, &mut seen, &mut queue);
                }
                while let Some(at) = queue.pop_front() {
                    for successor in func.blocks[at as usize].term.successors() {
                        push(successor.0, &mut seen, &mut queue);
                    }
                }
            }
            Direction::Backward(from) => {
                if from != avoid {
                    seen.insert(from.0);
                    queue.push_back(from.0);
                }
                while let Some(at) = queue.pop_front() {
                    for &pred in &self.preds[at as usize] {
                        push(pred, &mut seen, &mut queue);
                    }
                }
            }
        }
        seen
    }
}

/// Whether a call semantic cannot change any `Vec` length.
fn is_vec_len_preserving_call(semantic: CallSemantic) -> bool {
    matches!(
        semantic,
        CallSemantic::Standard(
            StandardOperation::VecLen
                | StandardOperation::VecIsEmpty
                | StandardOperation::VecCapacity
                | StandardOperation::NativeVecCapacity
                | StandardOperation::VecAsSlice
                | StandardOperation::NativeVecAsSlice
                | StandardOperation::VecAsMutSlice
                | StandardOperation::NativeVecAsMutSlice
                | StandardOperation::VecIndex
                | StandardOperation::VecIndexMut
                | StandardOperation::VecIter
                | StandardOperation::VecIterMut
                | StandardOperation::VecClone
                | StandardOperation::VecDrop
                | StandardOperation::VecReserve
                | StandardOperation::VecReserveExact
                | StandardOperation::NativeVecReserve
                | StandardOperation::NativeVecReserveExact
                | StandardOperation::NativeVecTryReserve
                | StandardOperation::NativeVecTryReserveExact
                | StandardOperation::VecShrinkToFit
                | StandardOperation::NativeVecShrinkTo
                | StandardOperation::NativeVecTryShrinkTo
                | StandardOperation::SliceSwap
                | StandardOperation::SliceSort
                | StandardOperation::SliceSortCmp
        )
    )
}

fn dominates_block(doms: &Dominators, a: RBlockId, b: RBlockId) -> bool {
    let mut at = b;
    loop {
        if at == a {
            return true;
        }
        match doms.idom.get(at.0 as usize).copied().flatten() {
            Some(up) if up != at => at = up,
            _ => return false,
        }
    }
}

/// Whether `ty` is a reference or raw pointer to a slice or `str`.
pub(super) fn is_slice_pointer(table: &TypeTable, ty: TypeId) -> bool {
    slice_pointee_element(table, ty).is_some()
}

fn slice_pointee_element(table: &TypeTable, ty: TypeId) -> Option<Option<TypeId>> {
    let pointee = match table
        .get(table.peeled(ty))
        .map(|descriptor| &descriptor.kind)
    {
        Some(TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. }) => {
            *pointee
        }
        _ => return None,
    };
    match table.get(table.peeled(pointee)).map(|d| &d.kind) {
        Some(TypeKind::Slice(element)) => Some(Some(*element)),
        Some(TypeKind::Str) => Some(None),
        _ => None,
    }
}

fn same_slice_pointer(table: &TypeTable, a: TypeId, b: TypeId) -> bool {
    matches!(
        (slice_pointee_element(table, a), slice_pointee_element(table, b)),
        (Some(Some(ea)), Some(Some(eb))) if table.peeled(ea) == table.peeled(eb)
    )
}

fn same_slice_or_str_pointer(table: &TypeTable, a: TypeId, b: TypeId) -> bool {
    match (
        slice_pointee_element(table, a),
        slice_pointee_element(table, b),
    ) {
        (Some(Some(ea)), Some(Some(eb))) => table.peeled(ea) == table.peeled(eb),
        (Some(None), Some(None)) => true,
        (Some(None), Some(Some(elem))) | (Some(Some(elem)), Some(None)) => {
            table.int_info(table.peeled(elem)) == Some((false, 8))
        }
        _ => false,
    }
}

/// The interval of `l op r` at an unsigned type whose largest value is `max`; anything that
/// could wrap is the full range.
fn binary_transfer(op: BinaryOp, l: Iv, r: Iv, max: u128) -> Iv {
    let full = Iv::full(max);
    match op {
        BinaryOp::Add | BinaryOp::AddUnchecked => match l.hi.checked_add(r.hi) {
            Some(hi) if hi <= max => Iv {
                lo: l.lo + r.lo,
                hi,
            },
            _ => full,
        },
        BinaryOp::Sub | BinaryOp::SubUnchecked => {
            if l.lo >= r.hi {
                Iv {
                    lo: l.lo - r.hi,
                    hi: l.hi - r.lo,
                }
            } else {
                full
            }
        }
        BinaryOp::Mul | BinaryOp::MulUnchecked => match l.hi.checked_mul(r.hi) {
            Some(hi) if hi <= max => Iv {
                lo: l.lo.saturating_mul(r.lo),
                hi,
            },
            _ => full,
        },
        BinaryOp::Div => {
            if r.lo >= 1 {
                Iv {
                    lo: l.lo / r.hi,
                    hi: l.hi / r.lo,
                }
            } else {
                full
            }
        }
        BinaryOp::Rem => {
            if r.lo >= 1 {
                Iv {
                    lo: 0,
                    hi: (r.hi - 1).min(l.hi),
                }
            } else {
                full
            }
        }
        BinaryOp::BitAnd => Iv {
            lo: 0,
            hi: l.hi.min(r.hi),
        },
        BinaryOp::BitOr | BinaryOp::BitXor => {
            let cover = l.hi.max(r.hi);
            Iv {
                lo: 0,
                hi: if cover == 0 {
                    0
                } else {
                    u128::MAX >> cover.leading_zeros()
                },
            }
        }
        BinaryOp::Shr | BinaryOp::ShrUnchecked => {
            if r.lo == r.hi && r.lo < 128 {
                Iv {
                    lo: l.lo >> r.lo,
                    hi: l.hi >> r.lo,
                }
            } else {
                Iv { lo: 0, hi: l.hi }
            }
        }
        _ => full,
    }
}

fn bitand_transfer(left: Option<Iv>, right: Option<Iv>, max: u128) -> Iv {
    let left = left.unwrap_or_else(|| Iv::full(max));
    let right = right.unwrap_or_else(|| Iv::full(max));
    Iv {
        lo: 0,
        hi: left.hi.min(right.hi),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::repr::FuncBuilder;
    use crate::jir::semantic::{
        AbiSignature, CallSemantic, Callee, DefinitionId, ItemId, Signature,
    };
    use crate::jir::types::{
        IntFacts, IntRepr, JsFacts, LayoutFacts, PointerForm, TypeDescriptor, TypeMeta,
    };
    use crate::jir::{OriginId, StrId};

    const U32: TypeId = TypeId(1);
    const U8: TypeId = TypeId(2);
    const SLICE: TypeId = TypeId(3);
    const REF_SLICE: TypeId = TypeId(4);
    const NO_ORIGIN: OriginId = OriginId(0);

    /// `u32`, `u8`, `[u8]` and `&[u8]`: enough for a bounds check over a slice.
    fn table() -> TypeTable {
        let mut table = TypeTable::new();
        let js = |int: Option<IntFacts>, peeled: TypeId| JsFacts {
            int,
            is_indirect: false,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: false,
            fields: Vec::new(),
        };
        let unsigned = |bits| {
            Some(IntFacts {
                signed: false,
                bits,
                repr: IntRepr::Number,
            })
        };
        let sized = LayoutFacts {
            sized: true,
            ..LayoutFacts::default()
        };
        table.insert(TypeDescriptor {
            id: U32,
            name: None,
            kind: TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            layout: sized,
            meta: TypeMeta::default(),
            js: js(unsigned(32), U32),
        });
        table.insert(TypeDescriptor {
            id: U8,
            name: None,
            kind: TypeKind::Integer {
                signed: false,
                bits: 8,
            },
            layout: sized,
            meta: TypeMeta::default(),
            js: js(unsigned(8), U8),
        });
        table.insert(TypeDescriptor {
            id: SLICE,
            name: None,
            kind: TypeKind::Slice(U8),
            layout: LayoutFacts::default(),
            meta: TypeMeta::default(),
            js: js(None, SLICE),
        });
        let mut slice_ref = js(None, REF_SLICE);
        slice_ref.pointer_form = Some(PointerForm::FatSlice);
        table.insert(TypeDescriptor {
            id: REF_SLICE,
            name: None,
            kind: TypeKind::Reference {
                pointee: SLICE,
                mutable: false,
            },
            layout: sized,
            meta: TypeMeta::default(),
            js: slice_ref,
        });
        table
    }

    fn builder() -> FuncBuilder {
        FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 0,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 0,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    inline_hint: false,
                    inline_always: false,
                },
                arguments: Vec::new(),
                result: U32,
            },
        )
    }

    /// `FuncBuilder::finish` with the JavaScript parameter count synced.
    fn finished(b: FuncBuilder) -> Func {
        let mut func = b.finish();
        func.sig.abi.parameters = func.params.len() as u16;
        func
    }

    fn constant(b: &mut FuncBuilder, block: RBlockId, value: u128) -> ValueId {
        b.op(
            block,
            OpKind::Const(ConstTree::Scalar {
                bits: value,
                bytes: 4,
            }),
            U32,
            NO_ORIGIN,
        )
    }

    fn metadata(b: &mut FuncBuilder, block: RBlockId, slice: ValueId) -> ValueId {
        b.op(
            block,
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v: slice,
                ty: REF_SLICE,
            },
            U32,
            NO_ORIGIN,
        )
    }

    fn compare(
        b: &mut FuncBuilder,
        block: RBlockId,
        op: BinaryOp,
        l: ValueId,
        r: ValueId,
    ) -> ValueId {
        b.op(block, OpKind::Binary { op, l, r, ty: U32 }, U32, NO_ORIGIN)
    }

    fn bounds_assert(b: &mut FuncBuilder, block: RBlockId, index: ValueId, length: ValueId) {
        let cond = compare(b, block, BinaryOp::Lt, index, length);
        b.effect(
            block,
            OpKind::Assert {
                cond,
                expected: true,
                msg: RAssertMessage::BoundsCheck { length, index },
                track_caller: None,
            },
            NO_ORIGIN,
        );
    }

    fn asserts(func: &Func) -> usize {
        func.blocks
            .iter()
            .flat_map(|block| &block.ops)
            .filter(|op| matches!(op.kind, OpKind::Assert { .. }))
            .count()
    }

    fn effect_call(b: &mut FuncBuilder, block: RBlockId) {
        b.effect(
            block,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 0,
                            local_hash: 9,
                        },
                        instantiation: 0,
                    },
                    link_name: StrId(0),
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
    }

    /// A kept block parameter does not hide the exact length of the slice view passed to it.
    #[test]
    fn a_slice_view_length_flows_through_a_block_parameter() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let source = b.param(REF_SLICE, NO_ORIGIN);
        let body = b.block();
        let slice = b.block_param(body, REF_SLICE, NO_ORIGIN);
        let sixteen = constant(&mut b, entry, 16);
        let view = b.op(
            entry,
            OpKind::SliceView {
                ptr: source,
                len: sixteen,
            },
            REF_SLICE,
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Goto(body, vec![view]));
        let length = metadata(&mut b, body, slice);
        let fifteen = constant(&mut b, body, 15);
        bounds_assert(&mut b, body, fifteen, length);
        let zero = constant(&mut b, body, 0);
        b.terminate(body, RTerm::Return(Some(zero)));
        let mut func = finished(b);
        let mut witnesses = Vec::new();

        assert_eq!(run(&mut func, &table(), &mut witnesses).asserts_removed, 1);
        assert_eq!(asserts(&func), 0);
        assert!(matches!(
            witnesses.as_slice(),
            [Witness::BoundsRemoved {
                fact: BoundsFact::Numeric {
                    index_hi: 15,
                    length_lo: 16
                },
                ..
            }]
        ));
    }

    /// A fixed `[from..to]` projection exposes its literal width to the bounds proof.
    #[test]
    fn a_fixed_subslice_has_an_exact_length() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let source = b.param(REF_SLICE, NO_ORIGIN);
        let view = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(source),
                path: vec![
                    Proj::Deref,
                    Proj::Subslice {
                        from: 2,
                        to: 6,
                        from_end: false,
                    },
                ],
                mutable: false,
            },
            REF_SLICE,
            NO_ORIGIN,
        );
        let length = metadata(&mut b, entry, view);
        let three = constant(&mut b, entry, 3);
        bounds_assert(&mut b, entry, three, length);
        let zero = constant(&mut b, entry, 0);
        b.terminate(entry, RTerm::Return(Some(zero)));
        let mut func = finished(b);

        assert_eq!(run(&mut func, &table(), &mut Vec::new()).asserts_removed, 1);
        assert_eq!(asserts(&func), 0);
    }

    /// An end-relative subslice keeps the exact source length minus its two fixed trims.
    #[test]
    fn an_end_relative_subslice_propagates_its_trimmed_length() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let source = b.param(REF_SLICE, NO_ORIGIN);
        let sixteen = constant(&mut b, entry, 16);
        let source = b.op(
            entry,
            OpKind::SliceView {
                ptr: source,
                len: sixteen,
            },
            REF_SLICE,
            NO_ORIGIN,
        );
        let view = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(source),
                path: vec![
                    Proj::Deref,
                    Proj::Subslice {
                        from: 1,
                        to: 2,
                        from_end: true,
                    },
                ],
                mutable: false,
            },
            REF_SLICE,
            NO_ORIGIN,
        );
        let length = metadata(&mut b, entry, view);
        let twelve = constant(&mut b, entry, 12);
        bounds_assert(&mut b, entry, twelve, length);
        let zero = constant(&mut b, entry, 0);
        b.terminate(entry, RTerm::Return(Some(zero)));
        let mut func = finished(b);

        assert_eq!(run(&mut func, &table(), &mut Vec::new()).asserts_removed, 1);
        assert_eq!(asserts(&func), 0);
    }

    /// `i & 7` against a constant length 8: the intervals alone prove the check.
    #[test]
    fn a_masked_index_against_a_constant_length_is_numeric() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let i = b.param(U32, NO_ORIGIN);
        let seven = constant(&mut b, entry, 7);
        let masked = compare(&mut b, entry, BinaryOp::BitAnd, i, seven);
        let eight = constant(&mut b, entry, 8);
        bounds_assert(&mut b, entry, masked, eight);
        let ret = constant(&mut b, entry, 0);
        b.terminate(entry, RTerm::Return(Some(ret)));
        let mut func = finished(b);
        let mut witnesses = Vec::new();

        assert_eq!(run(&mut func, &table(), &mut witnesses).asserts_removed, 1);
        assert_eq!(asserts(&func), 0);
        assert!(matches!(
            witnesses.as_slice(),
            [Witness::BoundsRemoved {
                fact: BoundsFact::Numeric {
                    index_hi: 7,
                    length_lo: 8
                },
                ..
            }]
        ));
        super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
    }

    #[test]
    fn a_constant_mask_bounds_an_unknown_operand() {
        assert_eq!(
            bitand_transfer(None, Some(Iv::point(15)), width_max(32)),
            Iv { lo: 0, hi: 15 }
        );
        assert_eq!(
            bitand_transfer(Some(Iv::point(7)), None, width_max(32)),
            Iv { lo: 0, hi: 7 }
        );
    }

    /// The guarded-body shape: `if i < s.len() { s[i] }`, with the assert re-reading the
    /// length through a second `PtrMetadata`. Value numbering equates the two reads and the
    /// edge fact proves the assert.
    fn guarded(effectful: bool) -> (Func, Vec<Witness>, u32) {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let i = b.param(U32, NO_ORIGIN);
        let s = b.param(REF_SLICE, NO_ORIGIN);
        let body = b.block();
        let exit = b.block();
        let n = metadata(&mut b, entry, s);
        let guard = compare(&mut b, entry, BinaryOp::Lt, i, n);
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: guard,
                scrut_ty: U32,
                cases: vec![(0, exit, Vec::new())],
                default: (body, Vec::new()),
            },
        );
        if effectful {
            effect_call(&mut b, body);
        }
        let len2 = metadata(&mut b, body, s);
        bounds_assert(&mut b, body, i, len2);
        let one = constant(&mut b, body, 1);
        b.terminate(body, RTerm::Return(Some(one)));
        let zero = constant(&mut b, exit, 0);
        b.terminate(exit, RTerm::Return(Some(zero)));
        let mut func = finished(b);
        let mut witnesses = Vec::new();
        let removed = run(&mut func, &table(), &mut witnesses).asserts_removed;
        (func, witnesses, removed)
    }

    /// The dominating guard removes the assert, and the witness re-checks.
    #[test]
    fn a_dominating_guard_removes_the_assert() {
        let (func, witnesses, removed) = guarded(false);
        assert_eq!(removed, 1);
        assert_eq!(asserts(&func), 0);
        assert!(matches!(
            witnesses.as_slice(),
            [Witness::BoundsRemoved {
                fact: BoundsFact::Dominating {
                    offset_index: 0,
                    offset_fact: 0,
                    strict: true,
                    ..
                },
                ..
            }]
        ));
        super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
    }

    /// An observable op between the guard and the assert refuses the removal (the ruling's
    /// letter: a call may do anything, including changing what a length re-read would see).
    #[test]
    fn an_observable_op_between_keeps_the_assert() {
        let (func, witnesses, removed) = guarded(true);
        assert_eq!(removed, 0);
        assert_eq!(asserts(&func), 1);
        assert!(witnesses.is_empty());
    }

    /// A corrupted bounds witness fails its independent check.
    #[test]
    fn a_corrupted_bounds_witness_is_rejected() {
        let (func, mut witnesses, removed) = guarded(false);
        assert_eq!(removed, 1);
        for witness in &mut witnesses {
            if let Witness::BoundsRemoved { fact, .. } = witness {
                *fact = BoundsFact::Dominating {
                    block: RBlockId(0),
                    cond: ValueId(0),
                    offset_index: 5,
                    offset_fact: 1,
                    strict: false,
                };
            }
        }
        assert!(super::super::witness::check_all(&witnesses, &func, &table()).is_err());
    }

    /// The chunked-guard shape: `i` bounded, guard `i + 2 <= n`, asserts on `i`, `i + 1` and
    /// `i + 2`. The first two are covered (`k < 2`), the third is not.
    #[test]
    fn a_le_guard_covers_smaller_offsets_only() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let x = b.param(U32, NO_ORIGIN);
        let s = b.param(REF_SLICE, NO_ORIGIN);
        let body = b.block();
        let exit = b.block();
        // i = x & 0xffff bounds the base so `i + k` certifiably cannot wrap.
        let mask_c = constant(&mut b, entry, 0xffff);
        let i = compare(&mut b, entry, BinaryOp::BitAnd, x, mask_c);
        let n = metadata(&mut b, entry, s);
        let two = constant(&mut b, entry, 2);
        let i2 = b.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: i,
                r: two,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        let guard = compare(&mut b, entry, BinaryOp::Le, i2, n);
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: guard,
                scrut_ty: U32,
                cases: vec![(0, exit, Vec::new())],
                default: (body, Vec::new()),
            },
        );
        let one = constant(&mut b, body, 1);
        let i1 = b.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: i,
                r: one,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        let len2 = metadata(&mut b, body, s);
        bounds_assert(&mut b, body, i, len2); // k = 0 < 2: removed
        bounds_assert(&mut b, body, i1, len2); // k = 1 < 2: removed
        let two_b = constant(&mut b, body, 2);
        let i2b = b.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: i,
                r: two_b,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        bounds_assert(&mut b, body, i2b, len2); // k = 2, not < 2: stays
        let ret = constant(&mut b, body, 0);
        b.terminate(body, RTerm::Return(Some(ret)));
        let zero = constant(&mut b, exit, 0);
        b.terminate(exit, RTerm::Return(Some(zero)));
        let mut func = finished(b);
        let mut witnesses = Vec::new();

        assert_eq!(run(&mut func, &table(), &mut witnesses).asserts_removed, 2);
        assert_eq!(asserts(&func), 1, "the k = 2 assert stays");
        super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
    }

    /// A length read from a different slice proves nothing.
    #[test]
    fn a_different_slice_length_proves_nothing() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let i = b.param(U32, NO_ORIGIN);
        let s = b.param(REF_SLICE, NO_ORIGIN);
        let t = b.param(REF_SLICE, NO_ORIGIN);
        let body = b.block();
        let exit = b.block();
        let n = metadata(&mut b, entry, s);
        let guard = compare(&mut b, entry, BinaryOp::Lt, i, n);
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: guard,
                scrut_ty: U32,
                cases: vec![(0, exit, Vec::new())],
                default: (body, Vec::new()),
            },
        );
        let other = metadata(&mut b, body, t);
        bounds_assert(&mut b, body, i, other);
        let one = constant(&mut b, body, 1);
        b.terminate(body, RTerm::Return(Some(one)));
        let zero = constant(&mut b, exit, 0);
        b.terminate(exit, RTerm::Return(Some(zero)));
        let mut func = finished(b);

        assert_eq!(run(&mut func, &table(), &mut Vec::new()).asserts_removed, 0);
        assert_eq!(asserts(&func), 1);
    }

    /// The counted-loop shape the fixture compiles to: `while i + 4 <= n` over a slice, the
    /// body indexing `s[i]`. The back-edge refinement bounds `i` by the slice-length cap, the
    /// widening round converges, and the peel certificate holds.
    #[test]
    fn a_counted_loop_guard_removes_the_body_assert() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let s = b.param(REF_SLICE, NO_ORIGIN);
        let header = b.block();
        let i = b.block_param(header, U32, NO_ORIGIN);
        let body = b.block();
        let exit = b.block();
        let zero = constant(&mut b, entry, 0);
        let n = metadata(&mut b, entry, s);
        b.terminate(entry, RTerm::Goto(header, vec![zero]));
        let four = constant(&mut b, header, 4);
        let stepped = b.op(
            header,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: i,
                r: four,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        let guard = compare(&mut b, header, BinaryOp::Le, stepped, n);
        b.terminate(
            header,
            RTerm::Switch {
                scrut: guard,
                scrut_ty: U32,
                cases: vec![(0, exit, Vec::new())],
                default: (body, Vec::new()),
            },
        );
        let len2 = metadata(&mut b, body, s);
        bounds_assert(&mut b, body, i, len2);
        // The increment is a fresh `i + 4`, as MIR spells it; value numbering has to see it as
        // the guard's `stepped`.
        let four_b = constant(&mut b, body, 4);
        let next = b.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: i,
                r: four_b,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        b.terminate(body, RTerm::Goto(header, vec![next]));
        let ret = constant(&mut b, exit, 0);
        b.terminate(exit, RTerm::Return(Some(ret)));
        let mut func = finished(b);
        let mut witnesses = Vec::new();

        assert_eq!(run(&mut func, &table(), &mut witnesses).asserts_removed, 1);
        assert_eq!(asserts(&func), 0);
        super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
    }

    /// A length carried unchanged through a loop header remains the same numbered scalar fact.
    #[test]
    fn an_unchanged_loop_carried_length_keeps_its_exact_fact() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let header = b.block();
        let i = b.block_param(header, U32, NO_ORIGIN);
        let length = b.block_param(header, U32, NO_ORIGIN);
        let body = b.block();
        let exit = b.block();
        let zero = constant(&mut b, entry, 0);
        let sixteen = constant(&mut b, entry, 16);
        b.terminate(entry, RTerm::Goto(header, vec![zero, sixteen]));

        let fifteen = constant(&mut b, header, 15);
        bounds_assert(&mut b, header, fifteen, length);
        let one = constant(&mut b, header, 1);
        let keep_going = compare(&mut b, header, BinaryOp::Lt, i, one);
        b.terminate(
            header,
            RTerm::Switch {
                scrut: keep_going,
                scrut_ty: U32,
                cases: vec![(0, exit, Vec::new())],
                default: (body, Vec::new()),
            },
        );
        let next = b.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: i,
                r: one,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        b.terminate(body, RTerm::Goto(header, vec![next, length]));
        let ret = constant(&mut b, exit, 0);
        b.terminate(exit, RTerm::Return(Some(ret)));
        let mut func = finished(b);

        assert_eq!(run(&mut func, &table(), &mut Vec::new()).asserts_removed, 1);
        assert_eq!(asserts(&func), 0);
    }

    /// The same loop with a call in the body keeps its assert: the establishment-free cycle
    /// through the body carries an observable op.
    #[test]
    fn a_counted_loop_with_a_call_keeps_the_assert() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let s = b.param(REF_SLICE, NO_ORIGIN);
        let header = b.block();
        let i = b.block_param(header, U32, NO_ORIGIN);
        let body = b.block();
        let exit = b.block();
        let zero = constant(&mut b, entry, 0);
        let n = metadata(&mut b, entry, s);
        b.terminate(entry, RTerm::Goto(header, vec![zero]));
        let four = constant(&mut b, header, 4);
        let stepped = b.op(
            header,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: i,
                r: four,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        let guard = compare(&mut b, header, BinaryOp::Le, stepped, n);
        b.terminate(
            header,
            RTerm::Switch {
                scrut: guard,
                scrut_ty: U32,
                cases: vec![(0, exit, Vec::new())],
                default: (body, Vec::new()),
            },
        );
        effect_call(&mut b, body);
        let len2 = metadata(&mut b, body, s);
        bounds_assert(&mut b, body, i, len2);
        let four_b = constant(&mut b, body, 4);
        let next = b.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: i,
                r: four_b,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        b.terminate(body, RTerm::Goto(header, vec![next]));
        let ret = constant(&mut b, exit, 0);
        b.terminate(exit, RTerm::Return(Some(ret)));
        let mut func = finished(b);

        assert_eq!(run(&mut func, &table(), &mut Vec::new()).asserts_removed, 0);
        assert_eq!(asserts(&func), 1);
    }

    #[test]
    fn an_aligned_slice_prefix_folds_only_its_two_proven_relations() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let length = b.param(U32, NO_ORIGIN);
        let chunk = constant(&mut b, entry, 16);
        let remainder = b.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Rem,
                l: length,
                r: chunk,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        let prefix = b.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Sub,
                l: length,
                r: remainder,
                ty: U32,
            },
            U32,
            NO_ORIGIN,
        );
        let zero = constant(&mut b, entry, 0);
        let below_zero = compare(&mut b, entry, BinaryOp::Lt, prefix, zero);
        let within_length = compare(&mut b, entry, BinaryOp::Le, prefix, length);
        // Equality is not implied: when `length` is a multiple of 16 it holds, otherwise it does
        // not. Keeping this comparison is the guard against turning the rule into a theorem
        // prover by accident.
        let maybe_equal = compare(&mut b, entry, BinaryOp::Eq, prefix, length);
        b.terminate(entry, RTerm::Return(Some(within_length)));
        let mut func = finished(b);
        let mut witnesses = Vec::new();

        let stats = run(&mut func, &table(), &mut witnesses);
        assert_eq!(
            stats,
            RangeStats {
                asserts_removed: 0,
                comparisons_folded: 2,
            }
        );
        let kind = |value| {
            func.blocks[0]
                .ops
                .iter()
                .find(|op| op.result == Some(value))
                .map(|op| &op.kind)
                .expect("the value remains defined")
        };
        assert!(matches!(
            kind(below_zero),
            OpKind::Const(ConstTree::Scalar { bits: 0, .. })
        ));
        assert!(matches!(
            kind(within_length),
            OpKind::Const(ConstTree::Scalar { bits: 1, .. })
        ));
        assert!(matches!(
            kind(maybe_equal),
            OpKind::Binary {
                op: BinaryOp::Eq,
                ..
            }
        ));
        super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
    }

    #[test]
    fn a_dominating_constant_bound_removes_smaller_constant_asserts() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let s = b.param(REF_SLICE, NO_ORIGIN);
        let body = b.block();
        let exit = b.block();
        let len = metadata(&mut b, entry, s);
        let fifteen = constant(&mut b, entry, 15);
        let guard = compare(&mut b, entry, BinaryOp::Lt, fifteen, len);
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: guard,
                scrut_ty: U32,
                cases: vec![(0, exit, Vec::new())],
                default: (body, Vec::new()),
            },
        );
        let len2 = metadata(&mut b, body, s);
        for k in [0u128, 1, 7, 15, 16] {
            let idx = constant(&mut b, body, k);
            bounds_assert(&mut b, body, idx, len2);
        }
        let ret = constant(&mut b, body, 1);
        b.terminate(body, RTerm::Return(Some(ret)));
        let zero = constant(&mut b, exit, 0);
        b.terminate(exit, RTerm::Return(Some(zero)));
        let mut func = finished(b);
        let mut witnesses = Vec::new();

        assert_eq!(run(&mut func, &table(), &mut witnesses).asserts_removed, 4);
        assert_eq!(asserts(&func), 1, "16 < len is not implied by 15 < len");
        super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
    }

    #[test]
    fn a_dominating_ge_or_eq_constant_bound_removes_constant_asserts() {
        for (op, expected_case, len_bound) in [
            (BinaryOp::Ge, 1u128, 16u128),
            (BinaryOp::Lt, 0, 16), // !(len < 16)
            (BinaryOp::Eq, 1, 16), // len == 16
        ] {
            let mut b = builder();
            let entry = FuncBuilder::ENTRY;
            let s = b.param(REF_SLICE, NO_ORIGIN);
            let body = b.block();
            let exit = b.block();
            let len = metadata(&mut b, entry, s);
            let bound = constant(&mut b, entry, len_bound);
            let guard = compare(&mut b, entry, op, len, bound);
            let (cases, default) = if expected_case == 1 {
                (vec![(0, exit, Vec::new())], (body, Vec::new()))
            } else {
                (vec![(0, body, Vec::new())], (exit, Vec::new()))
            };
            b.terminate(
                entry,
                RTerm::Switch {
                    scrut: guard,
                    scrut_ty: U32,
                    cases,
                    default,
                },
            );
            for k in [0u128, 15, 16] {
                let idx = constant(&mut b, body, k);
                bounds_assert(&mut b, body, idx, len);
            }
            let ret = constant(&mut b, body, 1);
            b.terminate(body, RTerm::Return(Some(ret)));
            let zero = constant(&mut b, exit, 0);
            b.terminate(exit, RTerm::Return(Some(zero)));
            let mut func = finished(b);
            let mut witnesses = Vec::new();

            assert_eq!(run(&mut func, &table(), &mut witnesses).asserts_removed, 2);
            assert_eq!(asserts(&func), 1, "16 < len stays");
            super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
        }
    }

    #[test]
    fn a_switch_matched_constant_length_removes_constant_asserts() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let s = b.param(REF_SLICE, NO_ORIGIN);
        let body = b.block();
        let exit = b.block();
        let len = metadata(&mut b, entry, s);
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: len,
                scrut_ty: U32,
                cases: vec![(16, body, Vec::new())],
                default: (exit, Vec::new()),
            },
        );
        for k in [0u128, 15, 16] {
            let idx = constant(&mut b, body, k);
            bounds_assert(&mut b, body, idx, len);
        }
        let ret = constant(&mut b, body, 1);
        b.terminate(body, RTerm::Return(Some(ret)));
        let zero = constant(&mut b, exit, 0);
        b.terminate(exit, RTerm::Return(Some(zero)));
        let mut func = finished(b);
        let mut witnesses = Vec::new();

        assert_eq!(run(&mut func, &table(), &mut witnesses).asserts_removed, 2);
        assert_eq!(asserts(&func), 1);
        super::super::witness::check_all(&witnesses, &func, &table()).expect("witness checks");
    }

    fn str_and_raw_table() -> (TypeTable, TypeId, TypeId, TypeId) {
        const RAW_U8: TypeId = TypeId(5);
        const STR: TypeId = TypeId(6);
        const REF_STR: TypeId = TypeId(7);

        let mut tbl = table();
        let make = |id, kind, pointer_form| TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                sized: id != STR,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: id,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form,
                byte_element: false,
                fields: Vec::new(),
            },
        };
        tbl.insert(make(
            RAW_U8,
            TypeKind::RawPointer {
                pointee: U8,
                mutable: false,
            },
            None,
        ));
        tbl.insert(make(STR, TypeKind::Str, None));
        tbl.insert(make(
            REF_STR,
            TypeKind::Reference {
                pointee: STR,
                mutable: false,
            },
            Some(PointerForm::FatSlice),
        ));
        (tbl, RAW_U8, STR, REF_STR)
    }

    #[test]
    fn peels_raw_parts_byte_slice_ref_and_str_cast_with_interns() {
        let (tbl, raw_u8, _str_ty, ref_str) = str_and_raw_table();
        let mut interns = Interns::default();
        interns.strings.push("slice_from_parts".to_string()); // StrId(0)
        interns.strings.push("byte_slice_ref".to_string()); // StrId(1)
        interns.strings.push("unknown_runtime".to_string()); // StrId(2)

        // Positive: `slice_from_parts(ptr, 16)` -> `byte_slice_ref` -> `&[u8] -> &str` cast -> `PtrMetadata`.
        {
            let mut b = builder();
            let entry = FuncBuilder::ENTRY;
            let ptr = b.param(raw_u8, NO_ORIGIN);
            let sixteen = constant(&mut b, entry, 16);
            let raw_slice = b.op(
                entry,
                OpKind::Call {
                    callee: Callee::Runtime {
                        member: StrId(0),
                        sysroot: true,
                    },
                    function: None,
                    args: vec![ptr, sixteen],
                    track_caller: None,
                    semantic: CallSemantic::RawParts,
                    host: None,
                },
                REF_SLICE,
                NO_ORIGIN,
            );
            let byte_slice = b.op(
                entry,
                OpKind::Call {
                    callee: Callee::Runtime {
                        member: StrId(1),
                        sysroot: true,
                    },
                    function: None,
                    args: vec![raw_slice],
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                REF_SLICE,
                NO_ORIGIN,
            );
            let as_str = b.op(
                entry,
                OpKind::Cast {
                    kind: CastKind::PtrToPtr,
                    v: byte_slice,
                    from: REF_SLICE,
                    to: ref_str,
                },
                ref_str,
                NO_ORIGIN,
            );
            let meta = b.op(
                entry,
                OpKind::Unary {
                    op: UnaryOp::PtrMetadata,
                    v: as_str,
                    ty: ref_str,
                },
                U32,
                NO_ORIGIN,
            );
            let idx15 = constant(&mut b, entry, 15);
            bounds_assert(&mut b, entry, idx15, meta);
            let idx16 = constant(&mut b, entry, 16);
            bounds_assert(&mut b, entry, idx16, meta);
            b.terminate(entry, RTerm::Return(Some(meta)));

            let mut func = finished(b);
            let mut witnesses = Vec::new();
            let res = run_with_interns(&mut func, &tbl, &interns, &mut witnesses);
            assert_eq!(res.asserts_removed, 1, "15 < 16 eliminated, 16 < 16 kept");
            assert_eq!(asserts(&func), 1);
            super::super::witness::check_all(&witnesses, &func, &tbl).expect("witness checks");
        }

        // Negative controls: `sysroot: false`, `Callee::Foreign`, and unknown sysroot fn must NOT peel.
        for (label, callee) in [
            (
                "sysroot_false",
                Callee::Runtime {
                    member: StrId(0),
                    sysroot: false,
                },
            ),
            (
                "foreign",
                Callee::Foreign {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 0,
                    },
                    symbol: StrId(0),
                },
            ),
            (
                "unknown_sysroot",
                Callee::Runtime {
                    member: StrId(2),
                    sysroot: true,
                },
            ),
        ] {
            let mut b = builder();
            let entry = FuncBuilder::ENTRY;
            let ptr = b.param(raw_u8, NO_ORIGIN);
            let sixteen = constant(&mut b, entry, 16);
            let s = b.op(
                entry,
                OpKind::Call {
                    callee,
                    function: None,
                    args: vec![ptr, sixteen],
                    track_caller: None,
                    semantic: CallSemantic::RawParts,
                    host: None,
                },
                REF_SLICE,
                NO_ORIGIN,
            );
            let meta = metadata(&mut b, entry, s);
            let idx15 = constant(&mut b, entry, 15);
            bounds_assert(&mut b, entry, idx15, meta);
            b.terminate(entry, RTerm::Return(Some(meta)));
            let mut func = finished(b);
            let mut witnesses = Vec::new();
            let res = run_with_interns(&mut func, &tbl, &interns, &mut witnesses);
            assert_eq!(
                res.asserts_removed, 0,
                "{label} must not peel slice_from_parts"
            );
        }
    }

    #[test]
    fn loop_induction_signed_peel_and_local_assert_eliminate_bounds_checks() {
        let tbl = table();
        let interns = Interns::default();

        // 1. Stepped loop `for i in 0..10 { assert(i + 40 < 50); assert(i + 40 < 50); }`
        //    proves `i in [0, 9]` via stepped back-edge refinement and eliminates both asserts
        //    (including when a duplicate local assert follows in the same block).
        {
            let mut b = builder();
            let entry = FuncBuilder::ENTRY;
            let header = b.block();
            let body = b.block();
            let exit = b.block();
            let zero = constant(&mut b, entry, 0);
            let one = constant(&mut b, entry, 1);
            let ten = constant(&mut b, entry, 10);
            let forty = constant(&mut b, entry, 40);
            let fifty = constant(&mut b, entry, 50);
            b.terminate(entry, RTerm::Goto(header, vec![zero]));
            let i = b.block_param(header, U32, NO_ORIGIN);
            let cond = compare(&mut b, header, BinaryOp::Lt, i, ten);
            b.terminate(
                header,
                RTerm::Switch {
                    scrut: cond,
                    scrut_ty: U32,
                    cases: vec![(0, exit, Vec::new())],
                    default: (body, Vec::new()),
                },
            );
            let idx = b.op(
                body,
                OpKind::Binary {
                    op: BinaryOp::Add,
                    l: i,
                    r: forty,
                    ty: U32,
                },
                U32,
                NO_ORIGIN,
            );
            bounds_assert(&mut b, body, idx, fifty);
            bounds_assert(&mut b, body, idx, fifty);
            let next = b.op(
                body,
                OpKind::Binary {
                    op: BinaryOp::Add,
                    l: i,
                    r: one,
                    ty: U32,
                },
                U32,
                NO_ORIGIN,
            );
            b.terminate(body, RTerm::Goto(header, vec![next]));
            let ret = constant(&mut b, exit, 0);
            b.terminate(exit, RTerm::Return(Some(ret)));

            let mut func = finished(b);
            let mut witnesses = Vec::new();
            let res = run_with_interns(&mut func, &tbl, &interns, &mut witnesses);
            assert_eq!(
                res.asserts_removed, 2,
                "both i + 40 < 50 asserts should fold"
            );
            assert_eq!(asserts(&func), 0);
            super::super::witness::check_all(&witnesses, &func, &tbl).expect("witness checks");
        }

        // 2. Signed peel across loop `16..64`: `assert(p - 2 < len)` dominates `assert(p - 7 < len)`
        //    and `assert(p - 16 < len)` because widening preserves `lo = 16`, guaranteeing no underflow.
        {
            let mut b = builder();
            let entry = FuncBuilder::ENTRY;
            let header = b.block();
            let body = b.block();
            let exit = b.block();
            let slice = b.param(REF_SLICE, NO_ORIGIN);
            let len = metadata(&mut b, entry, slice);
            let c1 = constant(&mut b, entry, 1);
            let c2 = constant(&mut b, entry, 2);
            let c7 = constant(&mut b, entry, 7);
            let c16 = constant(&mut b, entry, 16);
            let c64 = constant(&mut b, entry, 64);
            b.terminate(entry, RTerm::Goto(header, vec![c16]));
            let p = b.block_param(header, U32, NO_ORIGIN);
            let cond = compare(&mut b, header, BinaryOp::Lt, p, c64);
            b.terminate(
                header,
                RTerm::Switch {
                    scrut: cond,
                    scrut_ty: U32,
                    cases: vec![(0, exit, Vec::new())],
                    default: (body, Vec::new()),
                },
            );
            let pm2 = b.op(
                body,
                OpKind::Binary {
                    op: BinaryOp::Sub,
                    l: p,
                    r: c2,
                    ty: U32,
                },
                U32,
                NO_ORIGIN,
            );
            let pm7 = b.op(
                body,
                OpKind::Binary {
                    op: BinaryOp::Sub,
                    l: p,
                    r: c7,
                    ty: U32,
                },
                U32,
                NO_ORIGIN,
            );
            let pm16 = b.op(
                body,
                OpKind::Binary {
                    op: BinaryOp::Sub,
                    l: p,
                    r: c16,
                    ty: U32,
                },
                U32,
                NO_ORIGIN,
            );
            bounds_assert(&mut b, body, pm2, len);
            bounds_assert(&mut b, body, pm7, len);
            bounds_assert(&mut b, body, pm16, len);
            let next = b.op(
                body,
                OpKind::Binary {
                    op: BinaryOp::Add,
                    l: p,
                    r: c1,
                    ty: U32,
                },
                U32,
                NO_ORIGIN,
            );
            b.terminate(body, RTerm::Goto(header, vec![next]));
            let ret = constant(&mut b, exit, 0);
            b.terminate(exit, RTerm::Return(Some(ret)));

            let mut func = finished(b);
            let mut witnesses = Vec::new();
            let res = run_with_interns(&mut func, &tbl, &interns, &mut witnesses);
            assert_eq!(
                res.asserts_removed, 2,
                "p - 7 < len and p - 16 < len must be eliminated from p - 2 < len"
            );
            assert_eq!(asserts(&func), 1);
            super::super::witness::check_all(&witnesses, &func, &tbl).expect("witness checks");
        }
    }

    #[test]
    fn cast_wrapped_duplicate_bounds_check_eliminated_across_field_store() {
        let mut tbl = table();
        const USIZE: TypeId = TypeId(20);
        const REF_MUT_U32: TypeId = TypeId(21);
        tbl.insert(TypeDescriptor {
            id: USIZE,
            name: None,
            kind: TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: Some(IntFacts {
                    signed: false,
                    bits: 32,
                    repr: IntRepr::Number,
                }),
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: USIZE,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });
        tbl.insert(TypeDescriptor {
            id: REF_MUT_U32,
            name: None,
            kind: TypeKind::Reference {
                pointee: U32,
                mutable: true,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: REF_MUT_U32,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: Some(PointerForm::Slot),
                byte_element: false,
                fields: Vec::new(),
            },
        });

        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let g = b.param(U32, NO_ORIGIN);
        let s = b.param(REF_SLICE, NO_ORIGIN);
        let field_ptr = b.param(REF_MUT_U32, NO_ORIGIN);
        let len1 = metadata(&mut b, entry, s);
        let t = b.op(
            entry,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: g,
                from: U32,
                to: USIZE,
            },
            USIZE,
            NO_ORIGIN,
        );
        bounds_assert(&mut b, entry, t, len1);
        // Non-Vec scalar StoreThrough cannot change slice length!
        b.effect(
            entry,
            OpKind::StoreThrough {
                ptr: field_ptr,
                v: g,
            },
            NO_ORIGIN,
        );
        let len2 = metadata(&mut b, entry, s);
        let j = b.op(
            entry,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: g,
                from: U32,
                to: USIZE,
            },
            USIZE,
            NO_ORIGIN,
        );
        bounds_assert(&mut b, entry, j, len2);
        let zero = constant(&mut b, entry, 0);
        b.terminate(entry, RTerm::Return(Some(zero)));
        let mut func = finished(b);
        let mut witnesses = Vec::new();

        assert_eq!(run(&mut func, &tbl, &mut witnesses).asserts_removed, 1);
        assert_eq!(asserts(&func), 1);
        super::super::witness::check_all(&witnesses, &func, &tbl).expect("witness checks");
    }

    #[test]
    fn bitmask_one_shl_exact_alloc_len_eliminates_and_adversarial_cases_keep() {
        use crate::jir::types::{AllocatorIdentity, RepresentationCandidate};

        let mut tbl = table();
        const VEC_U8: TypeId = TypeId(30);
        const REF_VEC_U8: TypeId = TypeId(31);
        tbl.insert(TypeDescriptor {
            id: VEC_U8,
            name: None,
            kind: TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 2,
                    local_hash: 77,
                },
                form: crate::jir::types::NominalForm::Struct,
                args: vec![U8],
                variants: Vec::new(),
                niche: None,
            },
            layout: LayoutFacts {
                size: Some(12),
                align: Some(4),
                sized: true,
                zst: false,
                needs_drop: false,
                ..LayoutFacts::default()
            },
            meta: TypeMeta {
                candidate: RepresentationCandidate::Vec,
                allocator: AllocatorIdentity::Global,
                ..TypeMeta::default()
            },
            js: JsFacts {
                int: None,
                is_indirect: true,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: VEC_U8,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });
        tbl.insert(TypeDescriptor {
            id: REF_VEC_U8,
            name: None,
            kind: TypeKind::Reference {
                pointee: VEC_U8,
                mutable: false,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: REF_VEC_U8,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: Some(PointerForm::Thin),
                byte_element: false,
                fields: Vec::new(),
            },
        });

        // Positive: `len = 1 << mb`, `mask = len - 1`, `v = nv_zeroed(len)`, `sl = v.as_slice()`, `x & mask < sl.len()`.
        {
            let mut b = builder();
            let entry = FuncBuilder::ENTRY;
            let x = b.param(U32, NO_ORIGIN);
            let mb = b.param(U32, NO_ORIGIN);
            let one = constant(&mut b, entry, 1);
            let len = compare(&mut b, entry, BinaryOp::Shl, one, mb);
            let mask_v = compare(&mut b, entry, BinaryOp::Sub, len, one);
            let cell = b.cell(VEC_U8, false, NO_ORIGIN);
            b.effect(entry, OpKind::CellAlloc(cell), NO_ORIGIN);
            let vec_v = b.op(
                entry,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: ItemId {
                            definition: DefinitionId {
                                crate_id: 2,
                                local_hash: 1,
                            },
                            instantiation: 0,
                        },
                        link_name: StrId(0),
                    },
                    function: None,
                    args: vec![len],
                    track_caller: None,
                    semantic: CallSemantic::Standard(StandardOperation::NativeVecFromElemZeroed),
                    host: None,
                },
                VEC_U8,
                NO_ORIGIN,
            );
            b.effect(entry, OpKind::CellStore { cell, v: vec_v }, NO_ORIGIN);
            let vref = b.op(
                entry,
                OpKind::BorrowView {
                    root: Root::Cell(cell),
                    path: Vec::new(),
                    mutable: false,
                },
                REF_VEC_U8,
                NO_ORIGIN,
            );
            let sl = b.op(
                entry,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: ItemId {
                            definition: DefinitionId {
                                crate_id: 2,
                                local_hash: 2,
                            },
                            instantiation: 0,
                        },
                        link_name: StrId(0),
                    },
                    function: None,
                    args: vec![vref],
                    track_caller: None,
                    semantic: CallSemantic::Standard(StandardOperation::VecAsSlice),
                    host: None,
                },
                REF_SLICE,
                NO_ORIGIN,
            );
            let meta = metadata(&mut b, entry, sl);
            let idx = compare(&mut b, entry, BinaryOp::BitAnd, x, mask_v);
            bounds_assert(&mut b, entry, idx, meta);
            let zero = constant(&mut b, entry, 0);
            b.terminate(entry, RTerm::Return(Some(zero)));
            let mut func = finished(b);
            let mut witnesses = Vec::new();
            assert_eq!(run(&mut func, &tbl, &mut witnesses).asserts_removed, 1);
            assert_eq!(asserts(&func), 0);
            assert!(matches!(
                witnesses.as_slice(),
                [Witness::BoundsRemoved {
                    fact: BoundsFact::Bitmask { .. },
                    ..
                }]
            ));
            super::super::witness::check_all(&witnesses, &func, &tbl).expect("witness checks");
        }

        // Adversarial A: `mask = sl.len() - 1` when `sl.len()` may be `0` -> MUST keep assert!
        {
            let mut b = builder();
            let entry = FuncBuilder::ENTRY;
            let x = b.param(U32, NO_ORIGIN);
            let s = b.param(REF_SLICE, NO_ORIGIN);
            let n = metadata(&mut b, entry, s);
            let one = constant(&mut b, entry, 1);
            let mask_v = compare(&mut b, entry, BinaryOp::Sub, n, one);
            let idx = compare(&mut b, entry, BinaryOp::BitAnd, x, mask_v);
            bounds_assert(&mut b, entry, idx, n);
            let zero = constant(&mut b, entry, 0);
            b.terminate(entry, RTerm::Return(Some(zero)));
            let mut func = finished(b);
            assert_eq!(run(&mut func, &tbl, &mut Vec::new()).asserts_removed, 0);
            assert_eq!(asserts(&func), 1);
        }

        // Adversarial B: wrong mask `x & len` (not `len - 1`) with `len = 1 << mb` -> MUST keep assert!
        {
            let mut b = builder();
            let entry = FuncBuilder::ENTRY;
            let x = b.param(U32, NO_ORIGIN);
            let mb = b.param(U32, NO_ORIGIN);
            let one = constant(&mut b, entry, 1);
            let len = compare(&mut b, entry, BinaryOp::Shl, one, mb);
            let idx = compare(&mut b, entry, BinaryOp::BitAnd, x, len);
            bounds_assert(&mut b, entry, idx, len);
            let zero = constant(&mut b, entry, 0);
            b.terminate(entry, RTerm::Return(Some(zero)));
            let mut func = finished(b);
            assert_eq!(run(&mut func, &tbl, &mut Vec::new()).asserts_removed, 0);
            assert_eq!(asserts(&func), 1);
        }
    }

    #[test]
    fn monotone_arena_eliminates_always_valid_and_requires_sentinel_guard() {
        use crate::analysis::arena::ArenaInvariant;
        use crate::jir::types::{FieldDescriptor, FieldKey, NominalForm, VariantDescriptor};

        let mut tbl = table();
        const NODE_TY: TypeId = TypeId(40);
        const SLICE_NODE: TypeId = TypeId(41);
        const REF_SLICE_NODE: TypeId = TypeId(42);
        const REF_U32: TypeId = TypeId(43);
        const NODE_DEF: DefinitionId = DefinitionId {
            crate_id: 42,
            local_hash: 1,
        };
        let mk_js = |peeled, pf| JsFacts {
            int: None,
            is_indirect: false,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: pf,
            byte_element: false,
            fields: Vec::new(),
        };
        tbl.insert(TypeDescriptor {
            id: NODE_TY,
            name: None,
            kind: TypeKind::Adt {
                def: NODE_DEF,
                form: NominalForm::Struct,
                args: Vec::new(),
                variants: vec![VariantDescriptor {
                    name: StrId(0),
                    fields: vec![
                        FieldDescriptor {
                            key: FieldKey::Index(0),
                            ty: U32,
                            offset: 0,
                        },
                        FieldDescriptor {
                            key: FieldKey::Index(1),
                            ty: U32,
                            offset: 4,
                        },
                    ],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            layout: LayoutFacts {
                size: Some(8),
                align: Some(4),
                sized: true,
                zst: false,
                needs_drop: false,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: mk_js(NODE_TY, None),
        });
        tbl.insert(TypeDescriptor {
            id: SLICE_NODE,
            name: None,
            kind: TypeKind::Slice(NODE_TY),
            layout: LayoutFacts::default(),
            meta: TypeMeta::default(),
            js: mk_js(SLICE_NODE, None),
        });
        tbl.insert(TypeDescriptor {
            id: REF_SLICE_NODE,
            name: None,
            kind: TypeKind::Reference {
                pointee: SLICE_NODE,
                mutable: false,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: mk_js(REF_SLICE_NODE, Some(PointerForm::FatSlice)),
        });
        tbl.insert(TypeDescriptor {
            id: REF_U32,
            name: None,
            kind: TypeKind::Reference {
                pointee: U32,
                mutable: false,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: mk_js(REF_U32, Some(PointerForm::Slot)),
        });

        let mut facts = ProgramBoundsFacts::default();
        let mut fields = BTreeMap::new();
        fields.insert(0, ArenaFieldKind::AlwaysValid);
        fields.insert(
            1,
            ArenaFieldKind::ValidOrSentinel {
                sentinel: u32::MAX as u128,
            },
        );
        facts.arenas.insert(
            NODE_DEF,
            ArenaInvariant {
                elem_def: NODE_DEF,
                fields,
                holder_fields: BTreeSet::new(),
            },
        );

        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let nodes = b.param(REF_SLICE_NODE, NO_ORIGIN);
        let idx = b.param(U32, NO_ORIGIN);
        let len = b.op(
            entry,
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v: nodes,
                ty: REF_SLICE_NODE,
            },
            U32,
            NO_ORIGIN,
        );
        let prev_ref = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(nodes),
                path: vec![
                    Proj::Deref,
                    Proj::Index(idx),
                    Proj::Field { field: 0, ty: U32 },
                ],
                mutable: false,
            },
            REF_U32,
            NO_ORIGIN,
        );
        let prev = b.op(entry, OpKind::Deref(prev_ref), U32, NO_ORIGIN);
        let next_z_ref = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(nodes),
                path: vec![
                    Proj::Deref,
                    Proj::Index(idx),
                    Proj::Field { field: 1, ty: U32 },
                ],
                mutable: false,
            },
            REF_U32,
            NO_ORIGIN,
        );
        let next_z = b.op(entry, OpKind::Deref(next_z_ref), U32, NO_ORIGIN);

        // 1. AlwaysValid field `prev` -> eliminated without guard.
        bounds_assert(&mut b, entry, prev, len);
        // 2. Adversarial: ValidOrSentinel field `next_z` WITHOUT `!= u32::MAX` guard -> MUST STAY!
        bounds_assert(&mut b, entry, next_z, len);

        let sent = constant(&mut b, entry, u32::MAX as u128);
        let ne = compare(&mut b, entry, BinaryOp::Ne, next_z, sent);
        let guarded_blk = b.block();
        let exit_blk = b.block();
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: ne,
                scrut_ty: U32,
                cases: vec![(0, exit_blk, Vec::new())],
                default: (guarded_blk, Vec::new()),
            },
        );
        // 3. ValidOrSentinel field `next_z` WITH dominating `!= u32::MAX` guard -> eliminated!
        bounds_assert(&mut b, guarded_blk, next_z, len);
        let one = constant(&mut b, guarded_blk, 1);
        b.terminate(guarded_blk, RTerm::Return(Some(one)));
        let zero = constant(&mut b, exit_blk, 0);
        b.terminate(exit_blk, RTerm::Return(Some(zero)));

        let mut func = finished(b);
        let mut witnesses = Vec::new();
        let res = run_with_facts(&mut func, &tbl, &facts, &mut witnesses);
        assert_eq!(
            res.asserts_removed, 2,
            "AlwaysValid and guarded ValidOrSentinel removed; unguarded ValidOrSentinel stays"
        );
        assert_eq!(asserts(&func), 1);
        super::super::witness::check_all_with_context(&witnesses, &func, &tbl, Some(&facts))
            .expect("MonotoneArena witnesses must verify");
    }
}

/// The corpus property test, exactly [`super::dce`]'s: every fixture function the pass changes
/// still verifies and still means the same thing under the interpreter — and a wrongly removed
/// bounds check is exactly the kind of change it sees, as a `Trap::Assert` on one side only.
#[cfg(test)]
mod corpus {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use rcgjs_object::object;

    use crate::jir::interns::StrId;
    use crate::jir::interp::Value;
    use crate::jir::repr::Func;
    use crate::jir::semantic::{BodyKind, TypeId};
    use crate::jir::types::TypeTable;
    use crate::lower::ssa;

    const SETS: usize = 4;
    const STEPS: u64 = 5_000;

    #[derive(Default)]
    struct Counts {
        functions: usize,
        changed: usize,
        removed: u32,
        agreed: usize,
        skipped: usize,
        verify_failures: Vec<String>,
        mismatches: Vec<String>,
    }

    #[test]
    fn range_preserves_fixture_semantics() {
        std::thread::Builder::new()
            .stack_size(512 << 20)
            .spawn(run_corpus)
            .expect("spawn")
            .join()
            .expect("fixture thread");
    }

    fn run_corpus() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../build");
        let mut paths = Vec::new();
        walk(&root, &mut paths);
        let mut counts = Counts::default();
        for path in &paths {
            check_object(&path.display().to_string(), path, &mut counts);
        }
        eprintln!(
            "range property: {} functions, {} changed, {} asserts removed, {} argument sets \
             agreed, {} skipped, {} verify failures, {} mismatches",
            counts.functions,
            counts.changed,
            counts.removed,
            counts.agreed,
            counts.skipped,
            counts.verify_failures.len(),
            counts.mismatches.len(),
        );
        for failure in counts.verify_failures.iter().take(10) {
            eprintln!("  VERIFY   {failure}");
        }
        for mismatch in counts.mismatches.iter().take(10) {
            eprintln!("  MISMATCH {mismatch}");
        }
        ssa::check::require_fixtures(counts.functions, "range property");
        assert!(counts.verify_failures.is_empty(), "verify failures");
        assert!(counts.mismatches.is_empty(), "functions changed meaning");
    }

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
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
                    Some("hostmacro-discover" | "sysroot-cache" | "sysroots" | "target")
                ) {
                    continue;
                }
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "o") {
                out.push(path);
            }
        }
    }

    fn check_object(label: &str, path: &Path, counts: &mut Counts) {
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        if !object::is_object(&bytes) {
            return;
        }
        let Ok(Some(object)) = object::read(&bytes) else {
            return;
        };
        let table = &object.interns.types;
        let interns = &object.interns;
        let mut decoded = Vec::new();
        let mut results: BTreeMap<StrId, TypeId> = BTreeMap::new();
        for item in &object.summaries {
            let Ok(Some((body, origins))) = object.decode_verified(item) else {
                continue;
            };
            if !matches!(body.kind, BodyKind::Function) {
                continue;
            }
            if let Some(index) = interns
                .strings
                .iter()
                .position(|string| string == &item.key.link_name)
            {
                results.insert(StrId(index as u32), body.signature.result);
            }
            decoded.push((item.key.link_name.clone(), body, origins));
        }
        for (name, body, origins) in decoded {
            let Ok(before) = ssa::build(&body, Some(&origins), table, interns) else {
                continue;
            };
            counts.functions += 1;
            let mut after = before.clone();
            let range = super::run(&mut after, table, &mut Vec::new());
            if !range.changed() {
                continue;
            }
            counts.changed += 1;
            counts.removed += range.asserts_removed + range.comparisons_folded;
            if let Err(error) = after.verify(table) {
                counts
                    .verify_failures
                    .push(format!("{label} {name}: {error}"));
                continue;
            }
            let Some(sets) = ssa::check::argument_sets(
                &before
                    .params
                    .iter()
                    .map(|param| before.values[param.0 as usize].ty)
                    .collect::<Vec<_>>(),
                table,
            ) else {
                continue;
            };
            for set in sets.iter().take(SETS) {
                match compare(&before, &after, table, interns, &results, set) {
                    None => counts.agreed += 1,
                    Some(None) => counts.skipped += 1,
                    Some(Some(why)) => counts.mismatches.push(format!("{label} {name}: {why}")),
                }
            }
        }
    }

    #[allow(clippy::option_option)]
    fn compare(
        before: &Func,
        after: &Func,
        table: &TypeTable,
        interns: &crate::jir::interns::Interns,
        results: &BTreeMap<StrId, TypeId>,
        set: &[(Value, Option<Value>)],
    ) -> Option<Option<String>> {
        match ssa::check::compare_funcs(before, after, table, interns, results, set, STEPS) {
            ssa::check::Verdict::Agree => None,
            ssa::check::Verdict::Skipped(_) => Some(None),
            ssa::check::Verdict::Mismatch(why) => Some(Some(why)),
        }
    }
}
