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

mod arena_origins;
mod bounds;
mod gvn;
mod local_vecs;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::arena::{ArenaHandle, ProgramBoundsFacts};
use crate::jir::interns::Interns;
use crate::jir::repr::{
    CellId, Dominators, Func, OpKind, Proj, RAssertMessage, RBlockId, RTerm, Root, ValueId,
};
use crate::jir::semantic::{
    BinaryOp, CallSemantic, Callee, CastKind, ConstTree, DefinitionId, GlobalReference, ItemId,
    StandardOperation, TypeId, UnaryOp,
};
use crate::jir::types::TypeKind;
use crate::jir::TypeTable;

use super::cfg::{mask, Cfg};
use super::witness::{ArenaIndexSource, LocalVecRoot, Witness};

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
/// right shape is trusted. Production uses [`run_with_interns_and_facts`]; this form serves the
/// unit and corpus tests, whose functions carry no intern table.
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

/// Immutable inputs shared across range analysis and proof passes.
#[derive(Clone, Copy)]
struct RangeInputs<'a> {
    func: &'a Func,
    table: &'a TypeTable,
    interns: Option<&'a Interns>,
    arena_facts: Option<&'a ProgramBoundsFacts>,
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

    let ctx = RangeInputs {
        func,
        table,
        interns,
        arena_facts,
    };
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
            let Some(fact) = analysis.prove(ctx, *cond, block, index, &deleted) else {
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
        if deleted.range((b, 0)..=(b, usize::MAX)).next().is_none() {
            continue;
        }
        let mut index = 0usize;
        block.ops.retain(|_| {
            let keep = !deleted.contains(&(b, index));
            index += 1;
            keep
        });
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

        let ctx = RangeInputs {
            func,
            table,
            interns,
            arena_facts,
        };
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
        analysis.number_values(ctx);
        analysis.collect_facts(func, table, interns);
        analysis.scope_facts(func);
        analysis.compute_intervals(ctx);
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
    fn compute_intervals(&mut self, ctx: RangeInputs<'_>) {
        const WIDEN: usize = 3;
        const CAP: usize = 24;
        let func = ctx.func;
        let table = ctx.table;

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
                    let next = self.transfer(ctx, block, result, &op.kind, bits);
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
                    self.intervals[result.0 as usize] =
                        self.transfer(ctx, block, result, &op.kind, bits);
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
    fn transfer(
        &self,
        ctx: RangeInputs<'_>,
        block: RBlockId,
        result: ValueId,
        kind: &OpKind,
        bits: u32,
    ) -> Option<Iv> {
        let func = ctx.func;
        let table = ctx.table;
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
            } => Some(self.metadata_interval(ctx, *v, bits)),
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
                .const_array_element_interval(func, ctx.arena_facts, *ptr, max)
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
        let Some(pointee) = table.pointee(func.values[pointer.0 as usize].ty) else {
            return full;
        };
        let pointee = table.peeled(pointee);
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

    /// The exact/bounded length exposed by a slice view or subslice.
    fn metadata_interval(&self, ctx: RangeInputs<'_>, pointer: ValueId, bits: u32) -> Iv {
        let func = ctx.func;
        let table = ctx.table;
        let interns = ctx.interns;
        let arena_facts = ctx.arena_facts;
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
        let incoming = Cfg::incoming_edges(func);
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
                    let is_back = self.doms.dominates(header, *pred);
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
                    if !self.doms.dominates(fact_block, header) {
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

/// Whether `ty` is a reference or raw pointer to a slice or `str`.
pub(super) fn is_slice_pointer(table: &TypeTable, ty: TypeId) -> bool {
    slice_pointee_element(table, ty).is_some()
}

fn slice_pointee_element(table: &TypeTable, ty: TypeId) -> Option<Option<TypeId>> {
    let pointee = table.pointee(ty)?;
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
