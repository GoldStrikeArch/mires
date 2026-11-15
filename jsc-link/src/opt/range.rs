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

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::jir::repr::{
    Dominators, Func, Op, OpKind, Proj, RAssertMessage, RBlockId, RTerm, Root, ValueId,
};
use crate::jir::semantic::{BinaryOp, CastKind, ConstTree, TypeId, UnaryOp};
use crate::jir::types::TypeKind;
use crate::jir::TypeTable;
use crate::lower::fuse::{op_purity, Purity};

use super::witness::{BoundsFact, Witness};

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

/// Runs the pass over `func` in place.
pub fn run(func: &mut Func, table: &TypeTable, witnesses: &mut Vec<Witness>) -> RangeStats {
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
    let analysis = Analysis::of(func, table);
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
            } = &func.blocks[b].ops[index].kind
            else {
                continue;
            };
            let Some(fact) = analysis.prove(func, table, *cond, block, index, &deleted) else {
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
    /// Predecessor blocks, deduplicated.
    preds: Vec<Vec<u32>>,
}

impl Analysis {
    fn of(func: &Func, table: &TypeTable) -> Self {
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
            preds,
        };
        analysis.number_values(func, table);
        analysis.collect_facts(func, table);
        analysis.scope_facts(func);
        analysis.compute_intervals(func, table);
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
    fn number_values(&mut self, func: &Func, table: &TypeTable) {
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
        for &block in &self.doms.rpo.clone() {
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
                let Some(result) = op.result else {
                    continue;
                };
                let ty = func.values[result.0 as usize].ty;
                let key = match &op.kind {
                    OpKind::Copy(v) | OpKind::Move(v) => {
                        self.canon[result.0 as usize] = self.canon(*v);
                        continue;
                    }
                    // A whole-slice reborrow (`&*s`) is the slice it reborrows.
                    OpKind::BorrowView {
                        root: Root::Value(v),
                        path,
                        ..
                    } if matches!(path.as_slice(), [Proj::Deref])
                        && is_slice_pointer(table, ty) =>
                    {
                        self.canon[result.0 as usize] = self.canon(*v);
                        continue;
                    }
                    OpKind::Unary {
                        op: UnaryOp::PtrMetadata,
                        v,
                        ty: operand_ty,
                    } => {
                        // The `SliceView` lens: the metadata of a built view is its length
                        // operand.
                        let base = self.canon(*v);
                        if let Some(OpKind::SliceView { len, .. }) = self.def_kind(func, base) {
                            self.canon[result.0 as usize] = self.canon(*len);
                            continue;
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
    }

    /// Collects the known-true comparisons: decided switch edges and executed asserts.
    fn collect_facts(&mut self, func: &Func, table: &TypeTable) {
        let mut facts = Vec::new();
        let normalize = |cond: ValueId, positive: bool, site: Site, facts: &mut Vec<Fact>| {
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
            let positive = match entering.as_slice() {
                // The one case edge with value 0: the condition is false.
                [Some(0)] => false,
                // The one case edge with value 1: true.
                [Some(1)] => true,
                // The default edge, with 0 the only case value: not zero, so true.
                [None] if cases.iter().all(|(value, _, _)| *value == 0) => true,
                _ => continue,
            };
            normalize(*scrut, positive, site, &mut facts);
        }

        self.facts = facts;
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

    /// The interval fixpoint: block parameters join their edge contributions, refined by the
    /// facts at each edge; ops transfer in reverse postorder. A widening round keeps counted
    /// loops from crawling one trip at a time, and a failure to stabilize falls back to full
    /// ranges for every parameter, which one more transfer pass makes consistent.
    fn compute_intervals(&mut self, func: &Func, table: &TypeTable) {
        const WIDEN: usize = 4;
        const CAP: usize = 8;

        for &param in &func.blocks[0].params {
            if let Some(bits) = self.widths[param.0 as usize] {
                self.intervals[param.0 as usize] = Some(Iv::full(width_max(bits)));
            }
        }
        let term_facts: Vec<Vec<usize>> = (0..func.blocks.len())
            .map(|b| self.term_facts(RBlockId(b as u32)))
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
                        let Some((contribution, cap)) =
                            self.refine(*arg, &term_facts[block.0 as usize])
                        else {
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
                // loop converges instead of crawling one trip per round.
                if round == WIDEN && slot.is_some_and(|old| old.hi < next.hi) {
                    let bits = self.widths[param.0 as usize].expect("joined params are unsigned");
                    next.lo = 0;
                    next.hi = cap.unwrap_or(width_max(bits)).max(next.hi);
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
                    let next = self.transfer(func, table, &op.kind, bits);
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
                    self.intervals[result.0 as usize] = self.transfer(func, table, &op.kind, bits);
                }
            }
        }
    }

    /// The contribution of `arg` over an edge whose source block's terminator sees `facts`:
    /// the argument's interval, clamped by any fact bounding it from above. The clamp is also
    /// returned, as the widening bound for the parameter it feeds.
    fn refine(&self, arg: ValueId, facts: &[usize]) -> Option<(Iv, Option<u128>)> {
        let mut iv = self.interval(arg)?;
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
            let bound = match fact.rel {
                Rel::Le => bound_hi,
                Rel::Lt => bound_hi.saturating_sub(1),
            };
            cap = Some(cap.map_or(bound, |current| current.min(bound)));
        }
        if let Some(cap) = cap {
            iv.hi = iv.hi.min(cap);
            iv.lo = iv.lo.min(iv.hi);
        }
        Some((iv, cap))
    }

    /// The interval an op's result takes from its operands; `None` when an operand is still
    /// unknown (bottom), so optimism survives until the fixpoint fills it in.
    fn transfer(&self, func: &Func, table: &TypeTable, kind: &OpKind, bits: u32) -> Option<Iv> {
        let max = width_max(bits);
        let top = Some(Iv::full(max));
        match kind {
            OpKind::Const(ConstTree::Scalar { bits: value, bytes }) => {
                Some(Iv::point(mask(*value, *bytes).min(max)))
            }
            OpKind::Copy(v) | OpKind::Move(v) => match self.widths[v.0 as usize] {
                Some(w) if w == bits => self.interval(*v),
                _ => top,
            },
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v,
                ..
            } => Some(self.metadata_interval(func, table, *v, bits)),
            OpKind::Binary { op, l, r, ty } => {
                if !matches!(table.int_info(*ty), Some((false, w)) if w == bits) {
                    return top;
                }
                // A mask bounds the result even while the other operand is still bottom: for
                // unsigned integers `x & m <= m`. This matters for shifts whose dynamic amount
                // the interval domain cannot model, followed by a constant array-index mask.
                if *op == BinaryOp::BitAnd {
                    return Some(bitand_transfer(self.interval(*l), self.interval(*r), max));
                }
                let (l, r) = (self.interval(*l)?, self.interval(*r)?);
                Some(binary_transfer(*op, l, r, max))
            }
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v,
                from,
                ..
            } => match table.int_info(*from) {
                Some((false, _)) => {
                    let iv = self.interval(*v)?;
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

    /// The length bound of a pointer's metadata: a slice of elements that take space, or `str`,
    /// cannot be longer than `isize::MAX`, which is what certifies `+ k` index arithmetic.
    fn metadata_bound(&self, func: &Func, table: &TypeTable, pointer: ValueId, bits: u32) -> Iv {
        let full = Iv::full(width_max(bits));
        let ty = table.peeled(func.values[pointer.0 as usize].ty);
        let pointee = match table.get(ty).map(|descriptor| &descriptor.kind) {
            Some(TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. }) => {
                table.peeled(*pointee)
            }
            _ => return full,
        };
        let sized_elements = match table.get(pointee).map(|descriptor| &descriptor.kind) {
            Some(TypeKind::Slice(element)) => table
                .get(table.peeled(*element))
                .is_some_and(|descriptor| !descriptor.layout.zst),
            Some(TypeKind::Str) => true,
            _ => false,
        };
        if sized_elements && bits >= 2 {
            Iv {
                lo: 0,
                hi: width_max(bits - 1),
            }
        } else {
            full
        }
    }

    /// The exact/bounded length exposed by a slice view or subslice.
    ///
    /// This is intentionally a short structural lens, not symbolic algebra: fixed subslices have
    /// their literal width, end-relative subslices subtract their two fixed trims, and whole
    /// reborrows preserve the source. The construction's own legality ensures a reachable
    /// end-relative view has enough source elements; saturating the lower endpoint keeps the
    /// interval conservative even before that guard is visible here.
    fn metadata_interval(&self, func: &Func, table: &TypeTable, pointer: ValueId, bits: u32) -> Iv {
        let max = width_max(bits);
        let mut pointer = self.canon(pointer);
        let mut trim = 0_u128;
        for _ in 0..8 {
            match self.def_kind(func, pointer) {
                Some(OpKind::SliceView { len, .. }) => {
                    let Some(mut interval) = self.interval(*len) else {
                        return self.metadata_bound(func, table, pointer, bits);
                    };
                    interval.lo = interval.lo.saturating_sub(trim);
                    interval.hi = interval.hi.saturating_sub(trim);
                    return interval;
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

    /// Peels a chain of `+ constant` off `v` where the intervals certify no step can wrap:
    /// the result is `(base, k)` with `v == base + k` as mathematical integers.
    fn peel(&self, func: &Func, table: &TypeTable, v: ValueId) -> (ValueId, u128) {
        let mut at = self.canon(v);
        let mut offset = 0u128;
        for _ in 0..8 {
            let Some(OpKind::Binary {
                op: BinaryOp::Add | BinaryOp::AddUnchecked,
                l,
                r,
                ty,
            }) = self.def_kind(func, at)
            else {
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
            let (base, k) = match (constant(*l), constant(*r)) {
                (None, Some(k)) => (*l, k),
                (Some(k), None) => (*r, k),
                _ => break,
            };
            // The no-wrap certificate: the dynamic operand's upper bound plus the constant
            // stays inside the type.
            let Some(iv) = self.interval(self.canon(base)) else {
                break;
            };
            let Some(sum) = iv.hi.checked_add(k) else {
                break;
            };
            if sum > max {
                break;
            }
            let Some(total) = offset.checked_add(k) else {
                break;
            };
            offset = total;
            at = self.canon(base);
        }
        (at, offset)
    }

    /// Tries to prove the assert at `(block, index)` with condition `cond`; `deleted` names the
    /// asserts already removed, whose fact sites are no longer usable evidence.
    fn prove(
        &self,
        func: &Func,
        table: &TypeTable,
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

        // Numeric: the intervals alone imply it, over pure ssa derivations.
        if let (Some(iv_idx), Some(iv_len)) = (self.interval(*idx), self.interval(*len)) {
            if iv_idx.hi < iv_len.lo {
                return Some(BoundsFact::Numeric {
                    index_hi: iv_idx.hi,
                    length_lo: iv_len.lo,
                });
            }
        }

        // Symbolic: a dominating fact over the same base against the same length term.
        let (base, k_index) = self.peel(func, table, *idx);
        let length_term = self.canon(*len);
        let b = block.0 as usize;
        let local = self.facts.iter().enumerate().filter_map(|(i, fact)| {
            matches!(
                fact.site,
                Site::Assert { block: fb, index: fi, .. } if fb == block && fi < index
            )
            .then_some(i)
        });
        for fact_index in self.entry_facts[b].iter().copied().chain(local) {
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
                    let (fact_base, k_fact) = self.peel(func, table, vx);
                    if fact_base == base {
                        let implied = match fact.rel {
                            Rel::Lt => k_index <= k_fact,
                            Rel::Le => k_index < k_fact,
                        };
                        implied.then_some((k_index, k_fact))
                    } else if let (Some(iv_idx), Some(iv_fact)) =
                        (self.interval(*idx), self.interval(vx))
                    {
                        let implied = match fact.rel {
                            Rel::Lt => iv_idx.hi <= iv_fact.lo,
                            Rel::Le => iv_idx.hi < iv_fact.lo,
                        };
                        implied.then_some((iv_idx.hi, iv_fact.lo))
                    } else {
                        None
                    }
                }
                Bound::Const(c_fact) => self.interval(*idx).and_then(|iv_idx| {
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
            if self.effect_between(func, fact.site, block, index) {
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
        None
    }

    /// Whether an op that may observe or write memory sits on a path from the fact's
    /// establishment to the assert that does not re-establish the fact on the way.
    ///
    /// An edge fact is established on every entry to its target block (the target has exactly
    /// one incoming edge, which decides it); an assert fact at its own op. Either way,
    /// re-entering the establishment block re-establishes the fact, so the paths that matter
    /// never traverse it and the flood fills below simply refuse to.
    fn effect_between(&self, func: &Func, site: Site, block: RBlockId, index: usize) -> bool {
        let (start, from) = match site {
            Site::Edge { target, .. } => (target, 0usize),
            Site::Assert {
                block, index: at, ..
            } => (block, at + 1),
        };
        let effectful = |op: &Op| {
            op_purity(&op.kind) == Purity::Effect && !matches!(op.kind, OpKind::Assert { .. })
        };
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

/// Whether `ty` is a reference or raw pointer to a slice or `str`.
pub(super) fn is_slice_pointer(table: &TypeTable, ty: TypeId) -> bool {
    let pointee = match table
        .get(table.peeled(ty))
        .map(|descriptor| &descriptor.kind)
    {
        Some(TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. }) => {
            *pointee
        }
        _ => return false,
    };
    matches!(
        table.get(table.peeled(pointee)).map(|d| &d.kind),
        Some(TypeKind::Slice(_) | TypeKind::Str)
    )
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
