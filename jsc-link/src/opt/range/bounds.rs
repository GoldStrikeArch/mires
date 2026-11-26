//! Bounds-check proof obligations and memory-effect reachability checks for [`Analysis`].

use std::collections::{BTreeSet, VecDeque};
use std::num::NonZeroU128;

use crate::analysis::heap::is_global_vec_ty;
use crate::jir::interns::Interns;
use crate::jir::repr::{Func, Op, OpKind, RBlockId, ValueId};
use crate::jir::semantic::{BinaryOp, ConstTree, DefinitionId, DropGlue};
use crate::jir::TypeTable;
use crate::lower::fuse::{op_purity, Purity};

use super::super::cfg::mask;
use super::super::witness::{ArenaIndexSource, BoundsFact, SentinelGuard, SubOffset};
use super::{
    is_vec_len_preserving_call, Analysis, Bound, Direction, LocalElemBound, RangeInputs, Rel, Site,
};

impl Analysis {
    /// Tries to prove the assert at `(block, index)` with condition `cond`; `deleted` names the
    /// asserts already removed, whose fact sites are no longer usable evidence.
    pub(super) fn prove(
        &self,
        ctx: RangeInputs<'_>,
        cond: ValueId,
        block: RBlockId,
        index: usize,
        deleted: &BTreeSet<(u32, usize)>,
    ) -> Option<BoundsFact> {
        let func = ctx.func;
        let table = ctx.table;
        let interns = ctx.interns;
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
            if !self.doms.dominates(db.header, block) {
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
        if ctx.arena_facts.is_some() {
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
        // Case 0: direct `load` from a `LocalElemBound::Lt` local Vec (`SubOffset::Zero`).
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
                    sub_offset: SubOffset::Zero,
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
        let offset = NonZeroU128::new(k)?;
        // Locate dominating guard ensuring `lc >= k` (either `!= (k - 1)` or `<`/`<=` fact).
        let (guard_block, guard_cond) = active_ne
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
            sub_offset: SubOffset::Guarded {
                offset,
                block: guard_block,
                cond: guard_cond,
            },
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
                source: orig.source,
            }),
            Some(value) => {
                let (block, cond) = active_ne.iter().find_map(|&ni| {
                    let nf = &self.ne_facts[ni];
                    (self.canon(nf.v) == idx_c && nf.c == value).then_some(match nf.site {
                        Site::Edge { target, cond } => (target, cond),
                        Site::Assert { block: b, cond, .. } => (b, cond),
                    })
                })?;
                Some(BoundsFact::MonotoneArena {
                    elem_def: orig.elem_def,
                    sentinel: Some(SentinelGuard { value, block, cond }),
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
            OpKind::StoreThrough { ptr, .. } => table
                .pointee(func.values[ptr.0 as usize].ty)
                .is_none_or(|pt| !is_global_vec_ty(table, pt)),
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
