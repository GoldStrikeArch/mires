//! `ast-dce` over JIR-R: constant switches, unreachable blocks, dead ops, trivial block params.
//!
//! The JIR half of v1's `optimizer.rs`. The other half of the pass is already wired: the SSA
//! builder's `fold_runtime_checks` takes the decided edge of a `SwitchInt` on a `RuntimeCheck`
//! constant, which is what removes v1's 135 `if (false)` bodies from `core-tests/14_str`. This
//! file is what tidies up after that fold and after the escape analysis:
//!
//! 1. an integer comparison of two constants becomes a constant, an `Assert` whose constant
//!    condition says it can never fire is deleted, and a `Switch` on a constant scrutinee becomes a
//!    `Goto`;
//! 2. a block that immediately switches on the discriminant of its own parameter has every incoming
//!    edge whose argument's variant is statically known retargeted straight to the matching arm —
//!    the variant-per-path mechanism that lets the escape analysis scalarize an enum built under a
//!    condition and matched at the merge;
//! 3. blocks the entry no longer reaches are deleted and the rest renumbered densely;
//! 4. an op whose result is unused and whose [`Purity`] is `Const` or `Read` is deleted — never an
//!    `Effect` op, so a `Call`, a `Drop`, a store and a live `Assert` all stay, unless
//!    [`Options::pure_call`] vouches that a call's callee is pure;
//! 5. a block parameter whose incoming edges all pass one value (the parameter itself aside, so a
//!    loop-carried invariant counts) is replaced by that value; edges that pass the same constant
//!    *value* through different `Const` ops count as one value, materialized at the block's head.
//!
//! The rules run in that order and the whole thing iterates to a fixpoint, at most four times. Rule
//! 5 is what makes the escape analysis's optimistic per-field loop-header parameters affordable:
//! it adds one per field of every live object and this deletes the ones nothing varies.

use std::collections::{BTreeMap, BTreeSet};

use crate::jir::repr::{
    Carrier, CellId, Dominators, Func, Op, OpKind, RBlockId, RTerm, Root, ValueId,
};
use crate::jir::semantic::{BinaryOp, CallSemantic, Callee, CastKind, ConstTree, TypeId};
use crate::jir::types::{EnumRepr, NominalForm, TypeKind};
use crate::jir::TypeTable;
use crate::lower::fuse::{op_purity, Purity};

use super::witness::Witness;

/// The fixpoint bound. A round that changes nothing stops earlier.
const ROUNDS: usize = 4;

/// What the pass may do beyond its always-on floor.
///
/// Rules 1 and 3 — the constant folds, the settled asserts, and unreachable-block removal — run
/// unconditionally: they are the lane's reference-parity floor with the producer-baked baseline
/// text, which folded the same comparisons before its references were taken; a lane item that
/// kept them would reference panics the program no longer prepares. `RCGJS_JIR_PASSES` maps
/// `dce` to the deletions and the threading, and `constprop` to rule 5's constant-value merge.
#[derive(Clone, Copy, Default)]
pub struct Options<'a> {
    /// Disables rule 5's constant-value merge (leaving `constprop` out of `RCGJS_JIR_PASSES`).
    pub no_constant_params: bool,
    /// Disables the threading and the deletions (leaving `dce` out of `RCGJS_JIR_PASSES`).
    pub no_deletions: bool,
    /// Whether an unused-result `Call` may be deleted: the oracle says its callee never writes
    /// or observes anything (`Const`/`Read` purity from the effects summaries). `None` deletes
    /// no call, which is the pre-summary behavior.
    pub pure_call: Option<&'a dyn Fn(&Callee) -> bool>,
}

/// [`run_with`] under default options, discarding witnesses: the tests' spelling.
#[cfg(test)]
pub(crate) fn run(func: &mut Func, table: &TypeTable) -> bool {
    run_with(func, table, &Options::default(), &mut Vec::new())
}

/// [`run`] with pass options and a witness log for the rewrites that carry one.
pub fn run_with(
    func: &mut Func,
    table: &TypeTable,
    options: &Options<'_>,
    witnesses: &mut Vec<Witness>,
) -> bool {
    let mut changed = if options.no_deletions {
        false
    } else {
        split_unsigned_checked_adds(func, table, witnesses)
    };
    for _ in 0..ROUNDS {
        let mut round = fold_constant_comparisons(func, table, witnesses);
        if !options.no_deletions {
            round |= fold_signed_lt_step_overflow(func, table);
            round |= elide_redundant_copies(func, table);
            round |= cse_ptr_metadata(func, table);
        }
        round |= remove_settled_asserts(func);
        round |= fold_constant_switches(func, table, witnesses);
        if !options.no_deletions {
            round |= thread_variant_switches(func, table, witnesses);
            round |= split_payload_merges(func, table);
        }
        round |= remove_unreachable_blocks(func);
        if !options.no_deletions {
            round |= remove_dead_ops(func, options.pure_call);
            round |= remove_trivial_params(func, !options.no_constant_params);
        }
        changed |= round;
        if !round {
            break;
        }
    }
    if changed || has_orphaned_values(func) {
        compact_values(func);
    }
    changed
}

/// Splits a projected unsigned `AddWithOverflow` pair into two scalar values.
///
/// Rust commonly lowers `overflowing_add` to a tuple that is immediately projected as `.0` and
/// `.1`. Materializing that tuple in JavaScript costs an arrow closure and an array allocation.
/// For an unsigned modular integer the overflow bit is exactly `wrapped < lhs`, so the pair can
/// stay scalar whenever every use is one of those two projections:
///
/// ```text
/// pair = AddWithOverflow(lhs, rhs)  ->  wrapped = Add(lhs, rhs)
/// pair.0                            ->  wrapped
/// pair.1                            ->  Lt(wrapped, lhs)
/// ```
///
/// Refusing the rewrite on any other use keeps aggregate identity and storage semantics out of
/// this small lowering normalization.
fn split_unsigned_checked_adds(
    func: &mut Func,
    table: &TypeTable,
    witnesses: &mut Vec<Witness>,
) -> bool {
    #[derive(Clone, Copy)]
    struct Candidate {
        lhs: ValueId,
        ty: TypeId,
        origin: crate::jir::origin::OriginId,
        bits: u32,
        bool_ty: Option<TypeId>,
        projected: bool,
        valid: bool,
    }

    let mut candidates: BTreeMap<ValueId, Candidate> = BTreeMap::new();
    for block in &func.blocks {
        for op in &block.ops {
            let (
                Some(result),
                OpKind::Binary {
                    op: BinaryOp::AddWithOverflow,
                    l,
                    ty,
                    ..
                },
            ) = (op.result, &op.kind)
            else {
                continue;
            };
            let Some((false, bits)) = table.int_info(*ty) else {
                continue;
            };
            candidates.insert(
                result,
                Candidate {
                    lhs: *l,
                    ty: *ty,
                    origin: op.origin,
                    bits,
                    bool_ty: None,
                    projected: false,
                    valid: true,
                },
            );
        }
    }
    if candidates.is_empty() {
        return false;
    }

    for block in &func.blocks {
        for op in &block.ops {
            for used in op.kind.uses() {
                let Some(candidate) = candidates.get_mut(&used) else {
                    continue;
                };
                match (&op.kind, op.result) {
                    (
                        OpKind::Field {
                            base,
                            variant: None,
                            field: 0,
                        },
                        Some(result),
                    ) if *base == used && func.values[result.0 as usize].ty == candidate.ty => {
                        candidate.projected = true;
                    }
                    (
                        OpKind::Field {
                            base,
                            variant: None,
                            field: 1,
                        },
                        Some(result),
                    ) if *base == used => {
                        let ty = func.values[result.0 as usize].ty;
                        candidate.projected = true;
                        candidate.valid &= candidate.bool_ty.is_none_or(|known| known == ty);
                        candidate.bool_ty = Some(ty);
                    }
                    _ => candidate.valid = false,
                }
            }
        }
        for used in block.term.uses() {
            if let Some(candidate) = candidates.get_mut(&used) {
                candidate.valid = false;
            }
        }
    }
    candidates.retain(|_, candidate| candidate.valid && candidate.projected);
    if candidates.is_empty() {
        return false;
    }

    let mut overflow_values: BTreeMap<ValueId, ValueId> = BTreeMap::new();
    for (pair, candidate) in &candidates {
        func.values[pair.0 as usize].ty = candidate.ty;
        if let Some(bool_ty) = candidate.bool_ty {
            let overflow = ValueId(func.values.len() as u32);
            func.values.push(crate::jir::repr::ValueDef {
                ty: bool_ty,
                carrier: crate::jir::repr::Carrier::Baseline,
                origin: candidate.origin,
            });
            overflow_values.insert(*pair, overflow);
        }
        witnesses.push(Witness::UnsignedAddOverflowSplit {
            bits: candidate.bits,
        });
    }

    for block in &mut func.blocks {
        let mut rewritten = Vec::with_capacity(block.ops.len() + overflow_values.len());
        for mut op in std::mem::take(&mut block.ops) {
            let result = op.result;
            if let Some(pair) = result.filter(|pair| candidates.contains_key(pair)) {
                let candidate = candidates[&pair];
                op.kind = match op.kind {
                    OpKind::Binary {
                        op: BinaryOp::AddWithOverflow,
                        l,
                        r,
                        ty,
                    } => OpKind::Binary {
                        op: BinaryOp::Add,
                        l,
                        r,
                        ty,
                    },
                    other => other,
                };
                rewritten.push(op);
                if let Some(overflow) = overflow_values.get(&pair).copied() {
                    rewritten.push(Op {
                        result: Some(overflow),
                        origin: candidate.origin,
                        kind: OpKind::Binary {
                            op: BinaryOp::Lt,
                            l: pair,
                            r: candidate.lhs,
                            ty: candidate.ty,
                        },
                    });
                }
                continue;
            }

            if let OpKind::Field {
                base,
                variant: None,
                field,
            } = op.kind
            {
                if candidates.contains_key(&base) {
                    op.kind = match field {
                        0 => OpKind::Copy(base),
                        1 => OpKind::Copy(overflow_values[&base]),
                        _ => unreachable!("candidate validation accepted only fields 0 and 1"),
                    };
                } else {
                    op.kind = OpKind::Field {
                        base,
                        variant: None,
                        field,
                    };
                }
            }
            rewritten.push(op);
        }
        block.ops = rewritten;
    }
    true
}

/// Rewrites `AddWithOverflow(x, 1)` at signed `iN` (`N >= 2`) to `Add(x, 1)` + `overflow = false`
/// when dominated by a `Switch` edge proving `x < y` at the same signed type `iN`.
///
/// For any two's-complement signed integer `x < y` (`x, y: iN`), `x <= iN::MAX - 1` and
/// `x >= iN::MIN`, so `x + 1` is strictly within `[iN::MIN + 1, iN::MAX]` and cannot overflow.
/// This removes the `unreachable_unchecked` branch and panic payload inside `Range<iN>::next`.
fn fold_signed_lt_step_overflow(func: &mut Func, table: &TypeTable) -> bool {
    let n_values = func.values.len();
    let n_blocks = func.blocks.len();
    if n_blocks == 0 {
        return false;
    }

    let mut has_signed_add_ovf = false;
    let mut defs: Vec<Option<(usize, usize)>> = vec![None; n_values];
    for (b_idx, block) in func.blocks.iter().enumerate() {
        for (op_idx, op) in block.ops.iter().enumerate() {
            if let Some(result) = op.result {
                defs[result.0 as usize] = Some((b_idx, op_idx));
            }
            if let OpKind::Binary {
                op: BinaryOp::AddWithOverflow,
                ty,
                ..
            } = op.kind
            {
                if matches!(table.int_info(ty), Some((true, bits)) if bits >= 2) {
                    has_signed_add_ovf = true;
                }
            }
        }
    }
    if !has_signed_add_ovf {
        return false;
    }

    let doms = Dominators::of(func);
    let mut reachable = vec![false; n_blocks];
    reachable[0] = true;
    let mut stack = vec![0usize];
    while let Some(b) = stack.pop() {
        for succ in func.blocks[b].term.successors() {
            let s = succ.0 as usize;
            if s < n_blocks && !reachable[s] {
                reachable[s] = true;
                stack.push(s);
            }
        }
    }

    let mut preds: Vec<Vec<(usize, Vec<ValueId>)>> = vec![Vec::new(); n_blocks];
    for (pred_idx, block) in func.blocks.iter().enumerate() {
        if !reachable[pred_idx] {
            continue;
        }
        for (target, args) in block.term.edges() {
            let t = target.0 as usize;
            if t < n_blocks {
                preds[t].push((pred_idx, args.to_vec()));
            }
        }
    }

    let mut single_edge: Vec<Option<ValueId>> = vec![None; n_values];
    for (b_idx, block) in func.blocks.iter().enumerate() {
        if b_idx == 0 || !reachable[b_idx] {
            continue;
        }
        let mut unique_pred: Option<(usize, &[ValueId])> = None;
        let mut ok = true;
        for (pred_idx, args) in &preds[b_idx] {
            match unique_pred {
                None => unique_pred = Some((*pred_idx, args.as_slice())),
                Some((prev, _)) if prev == *pred_idx => ok = false,
                Some(_) => ok = false,
            }
        }
        if let (true, Some((pred_idx, args))) = (ok, unique_pred) {
            if pred_idx != b_idx && args.len() == block.params.len() {
                for (&param, &arg) in block.params.iter().zip(args.iter()) {
                    single_edge[param.0 as usize] = Some(arg);
                }
            }
        }
    }

    let def_op = |v: ValueId| -> Option<&Op> {
        let (b, i) = defs.get(v.0 as usize).copied().flatten()?;
        func.blocks.get(b)?.ops.get(i)
    };

    let strip_aliases = |mut v: ValueId| -> ValueId {
        for _ in 0..16 {
            if let Some(op) = def_op(v) {
                match op.kind {
                    OpKind::Copy(s) | OpKind::Move(s) => v = s,
                    _ => break,
                }
            } else if let Some(src) = single_edge.get(v.0 as usize).copied().flatten() {
                v = src;
            } else {
                break;
            }
        }
        v
    };

    let mask_bits = |x: u128, bits: u32| -> u128 {
        match bits {
            0 => 0,
            1..=127 => x & ((1u128 << bits) - 1),
            _ => x,
        }
    };

    let eval_const_int = |start: ValueId| -> Option<(u128, TypeId)> {
        let mut chain: Vec<(bool, u32, u32, TypeId)> = Vec::new();
        let mut cur = start;
        let mut base: Option<(u128, TypeId)> = None;
        for _ in 0..16 {
            if let Some(op) = def_op(cur) {
                match op.kind {
                    OpKind::Const(ConstTree::Scalar { bits, bytes }) => {
                        base = Some((mask(bits, bytes), func.values[cur.0 as usize].ty));
                        break;
                    }
                    OpKind::Copy(s) | OpKind::Move(s) => cur = s,
                    OpKind::Cast {
                        kind: CastKind::IntToInt,
                        v,
                        from,
                        to,
                    } => {
                        let (from_signed, from_bits) = table.int_info(from)?;
                        let (_, to_bits) = table.int_info(to)?;
                        chain.push((from_signed, from_bits, to_bits, to));
                        cur = v;
                    }
                    _ => return None,
                }
            } else {
                let src = single_edge.get(cur.0 as usize).copied().flatten()?;
                cur = src;
            }
        }
        let (mut val, mut val_ty) = base?;
        for (from_signed, from_bits, to_bits, to) in chain.into_iter().rev() {
            let extended = if from_signed {
                signed_value(val, from_bits) as u128
            } else {
                mask_bits(val, from_bits)
            };
            val = mask_bits(extended, to_bits);
            val_ty = to;
        }
        Some((val, val_ty))
    };

    let is_signed_one = |v: ValueId, target_ty: TypeId| -> bool {
        let Some((true, bits)) = table.int_info(target_ty) else {
            return false;
        };
        if bits < 2 {
            return false;
        }
        let Some((val, val_ty)) = eval_const_int(v) else {
            return false;
        };
        val_ty == target_ty && signed_value(val, bits) == 1
    };

    // Collect edge facts `(target_block, x_root, signed_ty)` where entering `target_block`
    // proves `x_root < some_y` at `signed_ty`.
    let mut edge_facts: Vec<(RBlockId, ValueId, TypeId)> = Vec::new();
    for t_idx in 1..n_blocks {
        if !reachable[t_idx] || preds[t_idx].len() != 1 {
            continue;
        }
        let pred_idx = preds[t_idx][0].0;
        if pred_idx == t_idx {
            continue;
        }
        let RTerm::Switch {
            scrut,
            scrut_ty,
            cases,
            default,
        } = &func.blocks[pred_idx].term
        else {
            continue;
        };
        let t_id = RBlockId(t_idx as u32);
        let width_bytes = table
            .int_info(*scrut_ty)
            .map_or(1, |(_, bits)| (bits / 8).max(1) as u8);
        let mut reaches_t = false;
        let mut all_nonzero = true;
        for (val, target, _) in cases {
            if *target == t_id {
                reaches_t = true;
                if mask(*val, width_bytes) == 0 {
                    all_nonzero = false;
                    break;
                }
            }
        }
        if default.0 == t_id {
            reaches_t = true;
            if !cases.iter().any(|(val, _, _)| mask(*val, width_bytes) == 0) {
                all_nonzero = false;
            }
        }
        if !reaches_t || !all_nonzero {
            continue;
        }
        let scrut_root = strip_aliases(*scrut);
        let Some(scrut_op) = def_op(scrut_root) else {
            continue;
        };
        let OpKind::Binary {
            op: cmp_op @ (BinaryOp::Lt | BinaryOp::Gt),
            l,
            r,
            ty: cmp_ty,
        } = scrut_op.kind
        else {
            continue;
        };
        if !matches!(table.int_info(cmp_ty), Some((true, bits)) if bits >= 2) {
            continue;
        }
        let x = if cmp_op == BinaryOp::Lt { l } else { r };
        edge_facts.push((t_id, strip_aliases(x), cmp_ty));
    }
    if edge_facts.is_empty() {
        return false;
    }

    let dominates = |a: RBlockId, b: RBlockId| -> bool {
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
    };

    #[derive(Clone, Copy)]
    struct Candidate {
        ty: TypeId,
        has_ovf_proj: bool,
        valid: bool,
    }

    let mut candidates: BTreeMap<ValueId, Candidate> = BTreeMap::new();
    for (b_idx, block) in func.blocks.iter().enumerate() {
        if !reachable[b_idx] {
            continue;
        }
        let b_id = RBlockId(b_idx as u32);
        for op in &block.ops {
            let (
                Some(result),
                OpKind::Binary {
                    op: BinaryOp::AddWithOverflow,
                    l,
                    r,
                    ty,
                },
            ) = (op.result, &op.kind)
            else {
                continue;
            };
            if !matches!(table.int_info(*ty), Some((true, bits)) if bits >= 2) {
                continue;
            }
            let l_root = strip_aliases(*l);
            let r_root = strip_aliases(*r);
            let proven = (is_signed_one(*r, *ty)
                && edge_facts.iter().any(|&(fact_b, fact_x, fact_ty)| {
                    fact_ty == *ty && fact_x == l_root && dominates(fact_b, b_id)
                }))
                || (is_signed_one(*l, *ty)
                    && edge_facts.iter().any(|&(fact_b, fact_x, fact_ty)| {
                        fact_ty == *ty && fact_x == r_root && dominates(fact_b, b_id)
                    }));
            if proven {
                candidates.insert(
                    result,
                    Candidate {
                        ty: *ty,
                        has_ovf_proj: false,
                        valid: true,
                    },
                );
            }
        }
    }
    if candidates.is_empty() {
        return false;
    }

    for block in &func.blocks {
        for op in &block.ops {
            for used in op.kind.uses() {
                let Some(cand) = candidates.get_mut(&used) else {
                    continue;
                };
                match (&op.kind, op.result) {
                    (
                        OpKind::Field {
                            base,
                            variant: None,
                            field: 0,
                        },
                        Some(result),
                    ) if *base == used && func.values[result.0 as usize].ty == cand.ty => {}
                    (
                        OpKind::Field {
                            base,
                            variant: None,
                            field: 1,
                        },
                        Some(result),
                    ) if *base == used
                        && matches!(
                            table
                                .get(func.values[result.0 as usize].ty)
                                .map(|d| &d.kind),
                            Some(TypeKind::Bool)
                        ) =>
                    {
                        cand.has_ovf_proj = true;
                    }
                    _ => cand.valid = false,
                }
            }
        }
        for used in block.term.uses() {
            if let Some(cand) = candidates.get_mut(&used) {
                cand.valid = false;
            }
        }
    }
    candidates.retain(|_, cand| cand.valid && cand.has_ovf_proj);
    if candidates.is_empty() {
        return false;
    }

    for (&pair, cand) in &candidates {
        func.values[pair.0 as usize].ty = cand.ty;
    }
    for block in &mut func.blocks {
        for op in &mut block.ops {
            if let Some(_pair) = op.result.filter(|r| candidates.contains_key(r)) {
                if let OpKind::Binary {
                    op: BinaryOp::AddWithOverflow,
                    l,
                    r,
                    ty,
                } = op.kind
                {
                    op.kind = OpKind::Binary {
                        op: BinaryOp::Add,
                        l,
                        r,
                        ty,
                    };
                }
                continue;
            }
            if let OpKind::Field {
                base,
                variant: None,
                field,
            } = op.kind
            {
                if candidates.contains_key(&base) {
                    op.kind = match field {
                        0 => OpKind::Copy(base),
                        1 => OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 1 }),
                        _ => unreachable!("candidate validation accepted only fields 0 and 1"),
                    };
                }
            }
        }
    }
    true
}

/// Rewrites redundant same-block chained `OpKind::Copy(v1)` into `OpKind::Move(v1)`.
///
/// When `v1` is a non-primitive (`needs_clone`) value with total syntactic use count `1`, defined
/// in the exact same basic block before `v2 = Copy(v1)`, and `v1` is already a uniquely-owned
/// fresh value (`Copy(_)` or `Aggregate` whose `needs_clone` fields are themselves single-use,
/// same-block, uniquely-owned fresh values, or `Move` of one), cloning `v1` again is redundant and
/// can never create an observable alias.
fn elide_redundant_copies(func: &mut Func, table: &TypeTable) -> bool {
    let n_values = func.values.len();
    let mut total_uses = vec![0usize; n_values];
    for block in &func.blocks {
        for op in &block.ops {
            for v in op.kind.uses() {
                if let Some(slot) = total_uses.get_mut(v.0 as usize) {
                    *slot += 1;
                }
            }
        }
        for v in block.term.uses() {
            if let Some(slot) = total_uses.get_mut(v.0 as usize) {
                *slot += 1;
            }
        }
    }

    let mut changed = false;
    let mut fresh_owned = vec![false; n_values];
    let mut def_in_block: Vec<Option<usize>> = vec![None; n_values];
    for (b_idx, block) in func.blocks.iter_mut().enumerate() {
        for op in &mut block.ops {
            let Some(res) = op.result else {
                continue;
            };
            let res_def = func.values[res.0 as usize];
            def_in_block[res.0 as usize] = Some(b_idx);
            if res_def.carrier != Carrier::Baseline {
                continue;
            }
            match &op.kind {
                OpKind::Copy(v1) => {
                    let v1_val = *v1;
                    let v1_def = func.values[v1_val.0 as usize];
                    let can_elide = table.needs_clone(res_def.ty)
                        && v1_def.ty == res_def.ty
                        && v1_def.carrier == Carrier::Baseline
                        && total_uses.get(v1_val.0 as usize).copied() == Some(1)
                        && def_in_block.get(v1_val.0 as usize).copied().flatten() == Some(b_idx)
                        && fresh_owned.get(v1_val.0 as usize).copied().unwrap_or(false);
                    if can_elide {
                        op.kind = OpKind::Move(v1_val);
                        changed = true;
                    }
                    // Either as a fresh `Copy` or a `Move` of an unaliased fresh value, `res` is fresh.
                    fresh_owned[res.0 as usize] = true;
                }
                OpKind::Move(v1) => {
                    if total_uses.get(v1.0 as usize).copied() == Some(1)
                        && def_in_block.get(v1.0 as usize).copied().flatten() == Some(b_idx)
                        && fresh_owned.get(v1.0 as usize).copied().unwrap_or(false)
                    {
                        fresh_owned[res.0 as usize] = true;
                    }
                }
                OpKind::Cast {
                    kind: CastKind::Transmute,
                    v: v1,
                    ..
                } => {
                    if total_uses.get(v1.0 as usize).copied() == Some(1)
                        && def_in_block.get(v1.0 as usize).copied().flatten() == Some(b_idx)
                        && fresh_owned.get(v1.0 as usize).copied().unwrap_or(false)
                    {
                        fresh_owned[res.0 as usize] = true;
                    }
                }
                OpKind::Repeat { v: elem, .. } => {
                    let elem_ty = func.values[elem.0 as usize].ty;
                    if !table.needs_clone(elem_ty) || table.is_zst(elem_ty) {
                        fresh_owned[res.0 as usize] = true;
                    }
                }
                OpKind::Aggregate {
                    active_field: None,
                    fields,
                    ..
                } => {
                    let all_fields_fresh = fields.iter().all(|f| {
                        let f_ty = func.values[f.0 as usize].ty;
                        if !table.needs_clone(f_ty) || table.is_zst(f_ty) {
                            return true;
                        }
                        func.values[f.0 as usize].carrier == Carrier::Baseline
                            && total_uses.get(f.0 as usize).copied() == Some(1)
                            && def_in_block.get(f.0 as usize).copied().flatten() == Some(b_idx)
                            && fresh_owned.get(f.0 as usize).copied().unwrap_or(false)
                    });
                    if all_fields_fresh {
                        fresh_owned[res.0 as usize] = true;
                    }
                }
                _ => {}
            }
        }
    }

    // Second pass: read-only non-mutated same-block `Copy(src)` forwarding.
    // When `res = Copy(src)` is only ever inspected inside the same block by `Discriminant(res)`
    // and scalar/non-clone `Field(res)` reads before any `Effect` op can mutate `src` (and before
    // `src` is moved), forward those reads directly to `src` so DCE deletes the redundant clone
    // (`Object.assign({}, opt)`) without moving `src`.
    for block in &mut func.blocks {
        let len = block.ops.len();
        for i in 0..len {
            let (res, src) = match (block.ops[i].result, &block.ops[i].kind) {
                (Some(res), OpKind::Copy(src)) => (res, *src),
                _ => continue,
            };
            let res_def = func.values[res.0 as usize];
            let src_def = func.values[src.0 as usize];
            if res_def.carrier != Carrier::Baseline
                || src_def.carrier != Carrier::Baseline
                || res_def.ty != src_def.ty
                || !table.needs_clone(res_def.ty)
            {
                continue;
            }
            let expected_uses = total_uses.get(res.0 as usize).copied().unwrap_or(0);
            if expected_uses == 0 {
                continue;
            }
            let mut local_safe_uses = 0usize;
            let mut last_use_idx = i;
            let mut unsafe_found = false;
            for (offset, op) in block.ops[i + 1..].iter().enumerate() {
                let uses_res = op.kind.uses().contains(&res);
                if uses_res {
                    let safe_read = match &op.kind {
                        OpKind::Discriminant(v) if *v == res => true,
                        OpKind::Field { base, .. } if *base == res => op
                            .result
                            .is_some_and(|r| !table.needs_clone(func.values[r.0 as usize].ty)),
                        _ => false,
                    };
                    if !safe_read {
                        unsafe_found = true;
                        break;
                    }
                    local_safe_uses += 1;
                    last_use_idx = i + 1 + offset;
                    if local_safe_uses == expected_uses {
                        break;
                    }
                }
                if matches!(op.kind, OpKind::Move(v) if v == src) {
                    unsafe_found = true;
                    break;
                }
                if matches!(op_purity(&op.kind), Purity::Effect)
                    && !matches!(op.kind, OpKind::Assert { .. })
                {
                    unsafe_found = true;
                    break;
                }
            }
            if !unsafe_found && local_safe_uses == expected_uses {
                for op in &mut block.ops[i + 1..=last_use_idx] {
                    match &mut op.kind {
                        OpKind::Discriminant(v) if *v == res => *v = src,
                        OpKind::Field { base, .. } if *base == res => *base = src,
                        _ => {}
                    }
                }
                total_uses[res.0 as usize] = 0;
                total_uses[src.0 as usize] =
                    total_uses[src.0 as usize].saturating_add(local_safe_uses);
                changed = true;
            }
        }
    }
    changed
}

/// Drops the value-table entries of every definition the rounds above deleted.
///
/// Deleting an op, a block or a block parameter leaves its `ValueDef` behind with nothing defining
/// it, and `Func::verify` rejects that ("value vN is never defined"). Every one of them is
/// unreferenced by construction: a deleted op's result was unused, a removed parameter's uses were
/// substituted first, and only unreachable blocks can name a value an unreachable block defined.
/// Deduplicates dominated `UnaryOp::PtrMetadata` reads of the same fat-pointer SSA value.
///
/// Fat-pointer metadata (slice length) is an immutable field of the SSA value itself, never
/// mutable heap memory. Reusing a dominating `PtrMetadata(v)` eliminates redundant `.length` /
/// `.len` reloads after inlining (e.g. `FullSlice` byte-string helpers).
fn cse_ptr_metadata(func: &mut Func, table: &TypeTable) -> bool {
    let n = func.values.len();
    let mut copy_of: Vec<Option<ValueId>> = vec![None; n];
    for block in &func.blocks {
        for op in &block.ops {
            if let Some(res) = op.result {
                if let OpKind::Copy(src) | OpKind::Move(src) = op.kind {
                    let rdef = func.values[res.0 as usize];
                    let sdef = func.values[src.0 as usize];
                    if rdef.carrier == Carrier::Baseline
                        && sdef.carrier == Carrier::Baseline
                        && rdef.ty == sdef.ty
                    {
                        copy_of[res.0 as usize] = Some(src);
                    }
                }
            }
        }
    }
    let peel = |mut v: ValueId| -> ValueId {
        for _ in 0..16 {
            let Some(next) = copy_of.get(v.0 as usize).copied().flatten() else {
                break;
            };
            if next == v {
                break;
            }
            v = next;
        }
        v
    };

    type PtrMetaDef = (RBlockId, usize, ValueId);
    let mut defs: BTreeMap<(ValueId, TypeId), Vec<PtrMetaDef>> = BTreeMap::new();
    for block in &func.blocks {
        for (op_idx, op) in block.ops.iter().enumerate() {
            let Some(res) = op.result else {
                continue;
            };
            let OpKind::Unary {
                op: crate::jir::semantic::UnaryOp::PtrMetadata,
                v,
                ..
            } = &op.kind
            else {
                continue;
            };
            let rdef = func.values[res.0 as usize];
            if rdef.carrier != Carrier::Baseline {
                continue;
            }
            if !matches!(
                table.get(table.peeled(rdef.ty)).map(|d| &d.kind),
                Some(TypeKind::Integer { .. })
            ) {
                continue;
            }
            defs.entry((peel(*v), rdef.ty))
                .or_default()
                .push((block.id, op_idx, res));
        }
    }
    if defs.values().all(|list| list.len() <= 1) {
        return false;
    }

    let doms = Dominators::of(func);
    let mut repl: BTreeMap<ValueId, ValueId> = BTreeMap::new();
    for list in defs.values() {
        if list.len() <= 1 {
            continue;
        }
        for &(b2, idx2, v2) in list {
            if !doms.dominates(RBlockId(0), b2) {
                continue;
            }
            let mut best: Option<(RBlockId, usize, ValueId)> = None;
            for &(b1, idx1, v1) in list {
                if v1 == v2 {
                    continue;
                }
                let dominates = if b1 == b2 {
                    idx1 < idx2
                } else {
                    doms.dominates(b1, b2)
                };
                if !dominates {
                    continue;
                }
                let better = match best {
                    None => true,
                    Some((bb, bidx, _)) => {
                        if b1 == bb {
                            idx1 < bidx
                        } else {
                            doms.dominates(b1, bb)
                        }
                    }
                };
                if better {
                    best = Some((b1, idx1, v1));
                }
            }
            if let Some((_, _, dom_v)) = best {
                repl.insert(v2, dom_v);
            }
        }
    }
    if repl.is_empty() {
        return false;
    }
    // Resolve chains in case a dominating def is itself dominated by an earlier one.
    let keys: Vec<ValueId> = repl.keys().copied().collect();
    for k in keys {
        let mut cur = repl[&k];
        while let Some(&next) = repl.get(&cur) {
            if next == cur {
                break;
            }
            cur = next;
        }
        repl.insert(k, cur);
    }
    substitute(func, &repl);
    true
}

fn has_orphaned_values(func: &Func) -> bool {
    let mut defined = vec![false; func.values.len()];
    for block in &func.blocks {
        for param in &block.params {
            if let Some(slot) = defined.get_mut(param.0 as usize) {
                *slot = true;
            }
        }
        for op in &block.ops {
            if let Some(result) = op.result {
                if let Some(slot) = defined.get_mut(result.0 as usize) {
                    *slot = true;
                }
            }
        }
    }
    defined.into_iter().any(|is_defined| !is_defined)
}

fn compact_values(func: &mut Func) {
    let mut dead: BTreeSet<ValueId> = (0..func.values.len() as u32).map(ValueId).collect();
    for block in &func.blocks {
        for param in &block.params {
            dead.remove(param);
        }
        for op in &block.ops {
            if let Some(result) = op.result {
                dead.remove(&result);
            }
        }
    }
    crate::lower::ssa::compact_values(func, &dead);
}

/// An integer comparison of two constants becomes the constant it evaluates to.
///
/// This is the JIR spelling of v1's `optimizer::fold_binary_literals`, and it is what makes rule 1
/// reach the checks `core` writes as `assert!(0 < size_of::<T>() && size_of::<T>() <= isize::MAX)`:
/// after monomorphization both sides are literals, v1 folded `0 < 4` to `true` before it took the
/// item's references, and a lane that did not would reference a `panic` the program no longer has.
/// Only comparisons fold — an arithmetic result would have to reproduce the wrapping and masking
/// the JavaScript side spells around it, and nothing needs that.
fn fold_constant_comparisons(
    func: &mut Func,
    table: &TypeTable,
    witnesses: &mut Vec<Witness>,
) -> bool {
    let mask_bits = |x: u128, bits: u32| -> u128 {
        match bits {
            0 => 0,
            1..=127 => x & ((1u128 << bits) - 1),
            _ => x,
        }
    };
    let mut constants: BTreeMap<ValueId, (u128, u8)> = BTreeMap::new();
    for _ in 0..4 {
        let before = constants.len();
        for block in &func.blocks {
            for op in &block.ops {
                let Some(result) = op.result else {
                    continue;
                };
                if constants.contains_key(&result) {
                    continue;
                }
                match &op.kind {
                    OpKind::Const(ConstTree::Scalar { bits, bytes }) => {
                        constants.insert(result, (*bits, *bytes));
                    }
                    OpKind::Copy(src) | OpKind::Move(src) => {
                        if let Some(c) = constants.get(src).copied() {
                            constants.insert(result, c);
                        }
                    }
                    OpKind::Cast {
                        kind: CastKind::IntToInt,
                        v,
                        from,
                        to,
                    } => {
                        if let (
                            Some((raw, raw_bytes)),
                            Some((from_signed, from_bits)),
                            Some((_, to_bits)),
                        ) = (
                            constants.get(v).copied(),
                            table.int_info(*from),
                            table.int_info(*to),
                        ) {
                            let masked = mask(raw, raw_bytes);
                            let extended = if from_signed {
                                signed_value(masked, from_bits) as u128
                            } else {
                                mask_bits(masked, from_bits)
                            };
                            let out_bits = mask_bits(extended, to_bits);
                            let out_bytes = (to_bits / 8).max(1) as u8;
                            constants.insert(result, (out_bits, out_bytes));
                        }
                    }
                    _ => {}
                }
            }
        }
        if constants.len() == before {
            break;
        }
    }
    let mut changed = false;
    for block in &mut func.blocks {
        for op in &mut block.ops {
            let OpKind::Binary {
                op: binary,
                l,
                r,
                ty,
            } = &op.kind
            else {
                continue;
            };
            let (Some(l), Some(r)) = (constants.get(l).copied(), constants.get(r).copied()) else {
                continue;
            };
            let (is_bool, ordering, l_witness, r_witness) =
                if let Some((signed, bits)) = table.int_info(*ty) {
                    let ord = match signed {
                        true => signed_value(l.0, bits).cmp(&signed_value(r.0, bits)),
                        false => mask(l.0, (bits / 8) as u8).cmp(&mask(r.0, (bits / 8) as u8)),
                    };
                    (false, ord, l, r)
                } else if matches!(
                    table.get(table.peeled(*ty)).map(|d| &d.kind),
                    Some(TypeKind::Bool)
                ) {
                    let lb = mask(l.0, l.1) != 0;
                    let rb = mask(r.0, r.1) != 0;
                    (true, lb.cmp(&rb), (u128::from(lb), 1), (u128::from(rb), 1))
                } else {
                    continue;
                };
            let (value, witness_op) = match binary {
                BinaryOp::Eq => (ordering.is_eq(), BinaryOp::Eq),
                BinaryOp::Ne => (ordering.is_ne(), BinaryOp::Ne),
                BinaryOp::Lt => (ordering.is_lt(), BinaryOp::Lt),
                BinaryOp::Le => (ordering.is_le(), BinaryOp::Le),
                BinaryOp::Gt => (ordering.is_gt(), BinaryOp::Gt),
                BinaryOp::Ge => (ordering.is_ge(), BinaryOp::Ge),
                // For `TypeKind::Bool`, `l ^ r` is `l != r`.
                BinaryOp::BitXor if is_bool => (ordering.is_ne(), BinaryOp::Ne),
                _ => continue,
            };
            witnesses.push(Witness::ConstFold {
                op: witness_op,
                ty: *ty,
                l: l_witness,
                r: r_witness,
                result: value,
            });
            op.kind = OpKind::Const(ConstTree::Scalar {
                bits: u128::from(value),
                bytes: 1,
            });
            changed = true;
        }
    }
    changed
}

/// An `Assert` whose condition is a constant that equals `expected` can never fire: deleted.
///
/// This is the JIR spelling of the two `if (false) __rt.js_abort(..)` residues row 25 of
/// phase-3.md counted in `as_chunks`: the divisor is a literal, the comparison folded (above or
/// in MIR), and the assert is settled at compile time. An assert whose constant condition
/// *disagrees* with `expected` always fires and is kept — deleting a throw would change what the
/// function does.
fn remove_settled_asserts(func: &mut Func) -> bool {
    let mut constants: BTreeMap<ValueId, (u128, u8)> = BTreeMap::new();
    for block in &func.blocks {
        for op in &block.ops {
            if let (Some(result), OpKind::Const(ConstTree::Scalar { bits, bytes })) =
                (op.result, &op.kind)
            {
                constants.insert(result, (*bits, *bytes));
            }
        }
    }
    let mut changed = false;
    for block in &mut func.blocks {
        let before = block.ops.len();
        block.ops.retain(|op| match &op.kind {
            OpKind::Assert { cond, expected, .. } => match constants.get(cond) {
                Some((bits, bytes)) => (mask(*bits, *bytes) != 0) != *expected,
                None => true,
            },
            _ => true,
        });
        changed |= block.ops.len() != before;
    }
    changed
}

/// `bits` read as a signed integer of `width` bits, sign-extended to `i128`.
fn signed_value(bits: u128, width: u32) -> i128 {
    if width == 0 || width >= 128 {
        return bits as i128;
    }
    let shift = 128 - width;
    ((bits << shift) as i128) >> shift
}

/// The values built as one statically known enum variant whose variant nothing can change behind
/// the ssa name: defined by an enum `Aggregate` or a variant/aggregate `Const` tree, on the
/// baseline carrier, and never used outside `Discriminant`/`Field`/`Copy`/edge arguments (a
/// borrow, a store, a call or a drop lets the storage be rewritten, so such a use pins the
/// value). A fieldless variant (`None`) is built as a `Const` tree rather than an `Aggregate`,
/// and the escape analysis's parameter rule already reads both forms.
///
/// Both the jump-threading and the constant-switch fold read a value's discriminant off this map.
fn pinned_values(func: &Func) -> Vec<bool> {
    use crate::jir::repr::Carrier;

    let mut pinned = vec![false; func.values.len()];
    for (index, def) in func.values.iter().enumerate() {
        if def.carrier != Carrier::Baseline {
            pinned[index] = true;
        }
    }
    for block in &func.blocks {
        for op in &block.ops {
            if matches!(
                op.kind,
                OpKind::Discriminant(_) | OpKind::Field { .. } | OpKind::Copy(_)
            ) {
                continue;
            }
            for v in op.kind.uses() {
                pinned[v.0 as usize] = true;
            }
        }
    }
    pinned
}

fn built_unpinned(func: &Func, table: &TypeTable) -> BTreeMap<ValueId, (TypeId, u32)> {
    let pinned = pinned_values(func);
    let mut built: BTreeMap<ValueId, (TypeId, u32)> = BTreeMap::new();
    for block in &func.blocks {
        for op in &block.ops {
            let Some(result) = op.result else {
                continue;
            };
            if pinned[result.0 as usize] {
                continue;
            }
            let ty = func.values[result.0 as usize].ty;
            let variant = match &op.kind {
                OpKind::Aggregate { variant, .. } => *variant,
                OpKind::Const(ConstTree::Variant { variant, .. }) => *variant,
                OpKind::Const(ConstTree::Aggregate(_)) => 0,
                // A niche-encoded fieldless constant — `None` of `Option<&T>` — is a scalar
                // whose bit pattern names the variant, the same decode the printer spells.
                OpKind::Const(ConstTree::Scalar { bits, .. }) => {
                    match scalar_variant(table, ty, *bits) {
                        Some(variant) => variant,
                        None => continue,
                    }
                }
                _ => continue,
            };
            built.insert(result, (ty, variant));
        }
    }
    built
}

/// The variant a scalar constant at enum type names: the variant whose direct tag is the bit
/// pattern, or the niche decode — the same rules `lower/js/constant.rs::scalar_expr` prints by.
/// `None` for a non-enum type or a bit pattern no variant owns.
fn scalar_variant(table: &TypeTable, ty: TypeId, bits: u128) -> Option<u32> {
    let peeled = table.peeled(ty);
    let Some(TypeKind::Adt {
        form: NominalForm::Enum,
        variants,
        niche,
        ..
    }) = table.get(peeled).map(|descriptor| &descriptor.kind)
    else {
        return None;
    };
    if let Some(niche) = niche {
        return Some(crate::lower::js::value::niche_variant(
            table, variants, niche, bits,
        ));
    }
    if variants
        .iter()
        .all(|variant| variant.physical_tag.is_some())
    {
        return variants
            .iter()
            .position(|variant| variant.physical_tag.is_some_and(|tag| tag.bits == bits))
            .map(|position| position as u32);
    }
    None
}

/// A `Switch` whose scrutinee is a constant integer takes its one live edge.
///
/// The scrutinee is constant three ways: a `Const` op, a `Discriminant` of a value whose layout
/// fixes its discriminant — the `ControlFlow<Infallible, T>` every `?` produces is that case, and
/// it is why this rule cannot wait for the printer, which folds it in the emitted text long after
/// an item's references have been taken — and a `Discriminant` of an unpinned value built as one
/// known variant, which is what a threaded merge leaves behind once its parameter is propagated
/// into the single remaining predecessor's `None`.
fn fold_constant_switches(
    func: &mut Func,
    table: &TypeTable,
    witnesses: &mut Vec<Witness>,
) -> bool {
    let built = built_unpinned(func, table);
    let mut constants: BTreeMap<ValueId, (u128, u8)> = BTreeMap::new();
    let mut discriminants: BTreeMap<ValueId, i128> = BTreeMap::new();
    for block in &func.blocks {
        for op in &block.ops {
            let Some(result) = op.result else {
                continue;
            };
            match &op.kind {
                OpKind::Const(ConstTree::Scalar { bits, bytes }) => {
                    constants.insert(result, (*bits, *bytes));
                }
                OpKind::Discriminant(base) => {
                    let ty = func.values[base.0 as usize].ty;
                    if let Some(discr) = static_discriminant(table, ty) {
                        discriminants.insert(result, discr);
                    } else if let Some(&(ty, variant)) = built.get(base) {
                        let variants = table.variants(table.peeled(ty));
                        if let Some(variant) = variants.get(variant as usize) {
                            discriminants.insert(result, variant.logical_discr);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut changed = false;
    for block in &mut func.blocks {
        let RTerm::Switch {
            scrut,
            scrut_ty,
            cases,
            default,
        } = &block.term
        else {
            continue;
        };
        let width = table
            .int_info(*scrut_ty)
            .map_or(16, |(_, bits)| (bits / 8) as u8);
        let Some((bits, bytes)) = constants
            .get(scrut)
            .copied()
            .or_else(|| Some((discriminants.get(scrut).copied()? as u128, width)))
        else {
            continue;
        };
        // Both sides are the raw bits MIR carries, so both are truncated to the constant's own
        // width before they meet: that is what makes `-1i32` and `0xffff_ffff` the same case.
        let value = mask(bits, bytes);
        let taken = cases
            .iter()
            .find(|(case, _, _)| mask(*case, bytes) == value)
            .map(|(_, target, args)| (*target, args.clone()))
            .unwrap_or_else(|| (default.0, default.1.clone()));
        witnesses.push(Witness::BranchFold {
            scrut: bits,
            bytes,
            cases: cases
                .iter()
                .map(|(case, target, _)| (*case, target.0))
                .collect(),
            default: default.0 .0,
            taken: taken.0 .0,
        });
        block.term = RTerm::Goto(taken.0, taken.1);
        changed = true;
    }
    changed
}

/// The discriminant of a value of type `ty`, when the layout fixes it at compile time.
///
/// The same rule `lower/js/rvalue.rs::discriminant` prints by: a non-enum has one variant whose
/// discriminant is zero, and an enum with at most one *inhabited* variant, or one whose layout
/// stores no tag at all, has its discriminant known here. `ControlFlow<Infallible, T>` is the case
/// that matters — `Infallible` is uninhabited, so every `?` switches on a constant.
fn static_discriminant(table: &TypeTable, ty: TypeId) -> Option<i128> {
    let peeled = table.peeled(ty);
    let Some(repr) = table.enum_repr(peeled) else {
        return Some(0);
    };
    let variants = table.variants(peeled);
    let inhabited: Vec<usize> = variants
        .iter()
        .enumerate()
        .filter(|(_, variant)| {
            !variant
                .fields
                .iter()
                .any(|field| uninhabited(table, field.ty))
        })
        .map(|(index, _)| index)
        .collect();
    let untagged = match table.get(peeled).map(|descriptor| &descriptor.kind) {
        Some(TypeKind::Adt { niche, .. }) => {
            niche.is_none()
                && variants
                    .iter()
                    .all(|variant| variant.physical_tag.is_none())
                && !matches!(repr, EnumRepr::Number { .. })
        }
        _ => false,
    };
    if inhabited.len() > 1 && !untagged {
        return None;
    }
    let index = inhabited.first().copied().unwrap_or(0);
    Some(
        variants
            .get(index)
            .map_or(0, |variant| variant.logical_discr),
    )
}

/// Whether a type is uninhabited (`!` or an enum with no variants), shallowly.
fn uninhabited(table: &TypeTable, ty: TypeId) -> bool {
    match table
        .get(table.peeled(ty))
        .map(|descriptor| &descriptor.kind)
    {
        Some(TypeKind::Never) => true,
        Some(TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            ..
        }) => variants.is_empty(),
        _ => false,
    }
}

/// `bits` truncated to `bytes` bytes; `bytes >= 16` is the identity.
fn mask(bits: u128, bytes: u8) -> u128 {
    match bytes {
        0 => 0,
        1..=15 => bits & ((1u128 << (bytes as u32 * 8)) - 1),
        _ => bits,
    }
}

/// Discriminant jump-threading: a block that immediately switches on the discriminant of its own
/// parameter has each incoming edge with a statically known variant retargeted to its arm.
///
/// This is the variant-per-path mechanism: an enum built as `Some(..)` on one edge and `None` on
/// another, merged into a parameter and matched at once, never actually merges — each creation
/// site jumps straight to its arm, and the escape analysis then scalarizes each site's aggregate,
/// which the merged parameter's unknown variant forbade.
///
/// Nothing is duplicated: the rule only retargets edges, and a merge block some edges still need
/// stays for them (a fully threaded one becomes unreachable and the next rule deletes it). The
/// conditions, each load-bearing:
///
/// * the merge block's ops are exactly one `Discriminant` of one of its own parameters, and the
///   terminator switches on its result — anything else in the block would run on the threaded edges
///   too, and is refused rather than duplicated;
/// * the incoming argument is defined by an enum `Aggregate` whose value never escapes: a use
///   outside `Discriminant`/`Field`/`Copy`/edge arguments (a borrow, a store, a call, a drop, a
///   non-baseline carrier) pins the value, because storage that escapes can have its variant
///   changed behind the ssa name;
/// * every use of a merge-defined value outside the merge is first rewritten to flow through an arm
///   parameter ([`localize_merge_values`]), so a retargeted edge leaves no use behind that the
///   merge no longer dominates — a merge whose values cannot be localized (the discriminant itself
///   used below, or a use outside a single-predecessor arm) is refused whole.
///
/// One merge is processed per iteration, because localizing rewrites the arms other candidates
/// may share; the scan restarts until nothing threads.
fn thread_variant_switches(
    func: &mut Func,
    table: &TypeTable,
    witnesses: &mut Vec<Witness>,
) -> bool {
    let mut changed = false;
    for _ in 0..8 {
        if !thread_one_merge(func, table, witnesses) {
            break;
        }
        changed = true;
    }
    changed
}

/// One threading step: finds the first merge block with a threadable edge, localizes its values,
/// and retargets every threadable edge. Reports whether it did anything.
fn thread_one_merge(func: &mut Func, table: &TypeTable, witnesses: &mut Vec<Witness>) -> bool {
    let built = built_unpinned(func, table);
    if built.is_empty() {
        return false;
    }

    for merge_index in 1..func.blocks.len() {
        let merge_id = RBlockId(merge_index as u32);
        let Some((position, width)) = merge_candidate(func, table, merge_id) else {
            continue;
        };
        // The threadable edges: (predecessor block, edge index, known logical discriminant).
        let mut threadable: Vec<(usize, usize, i128)> = Vec::new();
        for pred in &func.blocks {
            // Invoke's normal successor is private and dominates its result uses. Threading
            // either edge requires preserving that boundary as a separate rewrite.
            if matches!(pred.term, RTerm::Invoke { .. }) {
                continue;
            }
            for (edge, (target, args)) in pred.term.edges().into_iter().enumerate() {
                if target != merge_id {
                    continue;
                }
                let Some(arg) = args.get(position) else {
                    continue;
                };
                let Some(&(ty, variant)) = built.get(arg) else {
                    continue;
                };
                let variants = table.variants(table.peeled(ty));
                let Some(discr) = variants.get(variant as usize).map(|v| v.logical_discr) else {
                    continue;
                };
                threadable.push((pred.id.0 as usize, edge, discr));
            }
        }
        if threadable.is_empty() {
            continue;
        }
        if !localize_merge_values(func, merge_id) {
            continue;
        }

        // Everything below reads the merge as `localize_merge_values` left it.
        let mut retargets: Vec<(usize, usize, RBlockId, Vec<ValueId>)> = Vec::new();
        for (pred, edge, discr) in threadable {
            let args: Vec<ValueId> = func.blocks[pred].term.edges()[edge].1.to_vec();
            let merge = &func.blocks[merge_index];
            let RTerm::Switch { cases, default, .. } = &merge.term else {
                break;
            };
            let value = mask(discr as u128, width);
            let (taken, taken_args) = cases
                .iter()
                .find(|(case, _, _)| mask(*case, width) == value)
                .map(|(_, t, a)| (*t, a))
                .unwrap_or((default.0, &default.1));
            if taken == merge_id {
                continue;
            }
            let merge_discr = merge.ops.first().and_then(|op| op.result);
            let mapped: Option<Vec<ValueId>> = taken_args
                .iter()
                .map(|v| match merge.params.iter().position(|param| param == v) {
                    Some(at) => args.get(at).copied(),
                    // A value defined above the merge dominates every predecessor of the merge
                    // too, so the retargeted edge can pass it as it is; only the merge's own
                    // discriminant has no source on the new edge.
                    None => (Some(*v) != merge_discr).then_some(*v),
                })
                .collect();
            let Some(mapped) = mapped else {
                continue;
            };
            witnesses.push(Witness::BranchFold {
                scrut: discr as u128,
                bytes: width,
                cases: cases.iter().map(|(case, t, _)| (*case, t.0)).collect(),
                default: default.0 .0,
                taken: taken.0,
            });
            retargets.push((pred, edge, taken, mapped));
        }
        if retargets.is_empty() {
            continue;
        }
        for (pred, edge, target, args) in retargets {
            *edge_args_mut(&mut func.blocks[pred].term)[edge].1 = args;
            retarget_edge(&mut func.blocks[pred].term, edge, target);
        }
        return true;
    }
    false
}

/// Whether `block` is a merge-then-switch candidate: exactly one op, a `Discriminant` of one of
/// the block's own parameters, switched on. Returns the parameter position and the switch's
/// mask width.
fn merge_candidate(func: &Func, table: &TypeTable, block: RBlockId) -> Option<(usize, u8)> {
    let merge = &func.blocks[block.0 as usize];
    let [op] = merge.ops.as_slice() else {
        return None;
    };
    let (Some(discr), OpKind::Discriminant(base)) = (op.result, &op.kind) else {
        return None;
    };
    let position = merge.params.iter().position(|param| param == base)?;
    let RTerm::Switch {
        scrut, scrut_ty, ..
    } = &merge.term
    else {
        return None;
    };
    if scrut != &discr {
        return None;
    }
    let width = table
        .int_info(*scrut_ty)
        .map_or(16, |(_, bits)| (bits / 8) as u8);
    Some((position, width))
}

/// Rewrites every use of a merge-defined value outside the merge to flow through an arm
/// parameter, so an edge retargeted past the merge leaves no use the merge no longer dominates.
///
/// A use is coverable when its block sits under a switch arm whose only predecessor is the merge:
/// the arm gains a parameter, every edge from the merge into it passes the value, and the uses it
/// dominates read the parameter. A use of the discriminant itself, or one no arm covers, refuses
/// the merge (the caller skips it). Reports whether the merge is threadable afterwards.
fn localize_merge_values(func: &mut Func, merge_id: RBlockId) -> bool {
    use crate::jir::repr::Dominators;

    let merge_index = merge_id.0 as usize;
    let discr = func.blocks[merge_index]
        .ops
        .first()
        .and_then(|op| op.result)
        .expect("a merge candidate has the discriminant op");
    let defined: BTreeSet<ValueId> = func.blocks[merge_index]
        .params
        .iter()
        .copied()
        .chain(std::iter::once(discr))
        .collect();

    // Every outside use, by using block.
    let mut use_blocks: BTreeMap<u32, BTreeSet<ValueId>> = BTreeMap::new();
    for block in &func.blocks {
        if block.id == merge_id {
            continue;
        }
        let mut used: BTreeSet<ValueId> = BTreeSet::new();
        for op in &block.ops {
            for v in op.kind.uses() {
                if defined.contains(&v) {
                    used.insert(v);
                }
            }
        }
        for v in block.term.uses() {
            if defined.contains(&v) {
                used.insert(v);
            }
        }
        if !used.is_empty() {
            use_blocks.insert(block.id.0, used);
        }
    }
    if use_blocks.is_empty() {
        return true;
    }
    if use_blocks.values().any(|used| used.contains(&discr)) {
        // Threading an edge cannot supply the discriminant the arm reads.
        return false;
    }

    // Predecessor sets, to hold arms to "only the merge".
    let mut preds: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); func.blocks.len()];
    for block in &func.blocks {
        for successor in block.term.successors() {
            preds[successor.0 as usize].insert(block.id.0);
        }
    }
    let arm_targets: BTreeSet<u32> = func.blocks[merge_index]
        .term
        .successors()
        .into_iter()
        .map(|target| target.0)
        .filter(|target| {
            *target != merge_id.0 && preds[*target as usize] == BTreeSet::from([merge_id.0])
        })
        .collect();

    // The covering arm of each use block: the nearest dominator that is an arm.
    let doms = Dominators::of(func);
    let mut covering: BTreeMap<u32, u32> = BTreeMap::new();
    for &use_block in use_blocks.keys() {
        let mut at = use_block;
        let arm = loop {
            if arm_targets.contains(&at) {
                break Some(at);
            }
            match doms.idom.get(at as usize).copied().flatten() {
                Some(up) if up.0 != at => at = up.0,
                _ => break None,
            }
        };
        let Some(arm) = arm else {
            return false;
        };
        covering.insert(use_block, arm);
    }

    // One fresh parameter per (arm, value) pair, in deterministic order.
    let mut needed: BTreeMap<(u32, ValueId), ValueId> = BTreeMap::new();
    for (use_block, used) in &use_blocks {
        let arm = covering[use_block];
        for &v in used {
            needed.entry((arm, v)).or_insert_with(|| {
                let id = ValueId(func.values.len() as u32);
                let def = func.values[v.0 as usize];
                func.values.push(def);
                id
            });
        }
    }
    for (&(arm, v), &fresh) in &needed {
        func.blocks[arm as usize].params.push(fresh);
        for (target, args) in edge_args_mut(&mut func.blocks[merge_index].term) {
            if target.0 == arm {
                args.push(v);
            }
        }
    }
    for (use_block, used) in &use_blocks {
        let arm = covering[use_block];
        let map: BTreeMap<ValueId, ValueId> =
            used.iter().map(|&v| (v, needed[&(arm, v)])).collect();
        let rewrite = |v: ValueId| map.get(&v).copied().unwrap_or(v);
        let block = &mut func.blocks[*use_block as usize];
        for op in &mut block.ops {
            crate::lower::ssa::rewrite_op(&mut op.kind, &rewrite);
        }
        crate::lower::ssa::rewrite_term(&mut block.term, &rewrite);
    }
    true
}

/// Sets the target of edge `edge` (in [`edge_args_mut`] order) of `term`.
/// Payload merge splitting: an enum merge parameter used only as one variant's fields is
/// replaced by a parameter per field, each incoming edge passing the field values instead of
/// the whole object.
///
/// This is the residue the discriminant jump-threading leaves when one source of an `Option` is
/// opaque — a call kept by rustc's inline policy, `Skip::next`'s first-iteration `nth` in the
/// Murmur kernel: the threaded creation edges and the call's own `Some` arm merge into a
/// parameter, the merged parameter is not a node the escape analysis can virtualize (the call
/// result is real), and the virtual `Some` built in the loop would be materialized on its edge.
/// Splitting the parameter into its payload dissolves the merge instead: no object crosses it,
/// and the creation site dies with its last use.
///
/// The variant is proven per edge, or the parameter is refused whole:
///
/// * an argument built by an unpinned enum `Aggregate` of the projected variant passes its field
///   values straight from the creation site;
/// * an edge that is a `Switch` case over the argument's own `Discriminant`, naming the projected
///   variant, reads the fields in a new block on that edge alone — after the switch has proven them
///   readable, before anything else runs.
///
/// The rule fires only where at least one edge is a creation site (the shape whose edge
/// materialization it exists to remove), and only on a parameter every use of which is a
/// `Field` naming the same downcast variant.
fn split_payload_merges(func: &mut Func, table: &TypeTable) -> bool {
    let mut changed = false;
    for _ in 0..8 {
        if !split_one_payload_merge(func, table) {
            break;
        }
        changed = true;
    }
    changed
}

/// One splitting step: finds the first splittable merge parameter and rewrites it. Reports
/// whether it did anything.
fn split_one_payload_merge(func: &mut Func, table: &TypeTable) -> bool {
    use crate::jir::repr::{Carrier, RBlock, ValueDef};

    // The unpinned enum aggregates, with their field values: the direct-pass edges.
    let pinned = pinned_values(func);
    let built = built_unpinned(func, table);
    let mut agg_fields: BTreeMap<ValueId, (u32, Vec<ValueId>)> = BTreeMap::new();
    // Every `Discriminant`, by its result: the switch-proven edges.
    let mut discr_base: BTreeMap<ValueId, ValueId> = BTreeMap::new();
    for block in &func.blocks {
        for op in &block.ops {
            let Some(result) = op.result else {
                continue;
            };
            match &op.kind {
                OpKind::Aggregate {
                    variant,
                    active_field: None,
                    fields,
                    ..
                } if built.contains_key(&result) => {
                    agg_fields.insert(result, (*variant, fields.clone()));
                }
                OpKind::Discriminant(base) => {
                    discr_base.insert(result, *base);
                }
                _ => {}
            }
        }
    }

    if agg_fields.is_empty() {
        return false;
    }

    // Pre-index all merge-block parameters, their uses, and incoming block edges in one linear
    // pass over `func.blocks` instead of scanning every op in the function per parameter.
    let n_values = func.values.len();
    let mut is_merge_param = vec![false; n_values];
    for block in func.blocks.iter().skip(1) {
        for &param in &block.params {
            if func.values[param.0 as usize].carrier == Carrier::Baseline {
                is_merge_param[param.0 as usize] = true;
            }
        }
    }
    let mut invalid = vec![false; n_values];
    let mut param_variant: Vec<Option<u32>> = vec![None; n_values];
    type ProjList = Vec<(usize, usize, u32, ValueId, crate::jir::origin::OriginId)>;
    let mut param_projs: BTreeMap<ValueId, ProjList> = BTreeMap::new();
    let mut incoming: Vec<Vec<(usize, usize)>> = vec![Vec::new(); func.blocks.len()];
    for (block_index, block) in func.blocks.iter().enumerate() {
        for (op_index, op) in block.ops.iter().enumerate() {
            if let OpKind::Field {
                base,
                variant: Some(k),
                field,
            } = &op.kind
            {
                let idx = base.0 as usize;
                if is_merge_param[idx] {
                    if !invalid[idx] {
                        if *param_variant[idx].get_or_insert(*k) != *k {
                            invalid[idx] = true;
                        } else if let Some(result) = op.result {
                            if func.values[result.0 as usize].carrier != Carrier::Baseline {
                                invalid[idx] = true;
                            } else {
                                param_projs.entry(*base).or_default().push((
                                    block_index,
                                    op_index,
                                    *field,
                                    result,
                                    op.origin,
                                ));
                            }
                        } else {
                            invalid[idx] = true;
                        }
                    }
                    continue;
                }
            }
            for used in op.kind.uses() {
                let idx = used.0 as usize;
                if is_merge_param[idx] {
                    invalid[idx] = true;
                }
            }
        }
        for (edge, (target, args)) in block.term.edges().into_iter().enumerate() {
            incoming[target.0 as usize].push((block_index, edge));
            for &arg in args {
                let idx = arg.0 as usize;
                if is_merge_param[idx] {
                    invalid[idx] = true;
                }
            }
        }
        match &block.term {
            RTerm::Switch { scrut, .. } if is_merge_param[scrut.0 as usize] => {
                invalid[scrut.0 as usize] = true;
            }
            RTerm::Return(Some(v)) | RTerm::Throw(Some(v)) if is_merge_param[v.0 as usize] => {
                invalid[v.0 as usize] = true;
            }
            _ => {}
        }
    }

    for (merge_index, merge_incoming) in incoming.iter().enumerate().skip(1) {
        let merge_id = RBlockId(merge_index as u32);
        'params: for position in 0..func.blocks[merge_index].params.len() {
            let param = func.blocks[merge_index].params[position];
            let p_idx = param.0 as usize;
            if !is_merge_param[p_idx] || invalid[p_idx] {
                continue;
            }
            // Every use must be a `Field` of one same downcast variant; anything else — another
            // op, an edge argument, a scrutinee, a return — keeps the object whole.
            let Some(k) = param_variant[p_idx] else {
                continue;
            };
            let Some(projections) = param_projs.get(&param) else {
                continue;
            };

            // Every projection's result becomes a parameter of its own — two reads of one field
            // simply bind the same passed value twice, which spares the rewrite any use
            // tracking (and a `Move` it must not duplicate).

            // A plan per incoming edge, or the parameter is refused whole.
            enum Plan {
                Direct(Vec<ValueId>),
                Split { arg: ValueId },
            }
            let mut plans: Vec<(usize, usize, Plan)> = Vec::new();
            let mut direct = false;
            for &(pred_index, edge) in merge_incoming {
                let pred = &func.blocks[pred_index];
                let (_, args) = pred.term.edges()[edge];
                let Some(&arg) = args.get(position) else {
                    continue 'params;
                };
                if let Some((built_variant, fields)) = agg_fields.get(&arg) {
                    if *built_variant != k {
                        continue 'params;
                    }
                    let Some(values) = projections
                        .iter()
                        .map(|&(_, _, field, _, _)| fields.get(field as usize).copied())
                        .collect::<Option<Vec<ValueId>>>()
                    else {
                        continue 'params;
                    };
                    plans.push((pred_index, edge, Plan::Direct(values)));
                    direct = true;
                    continue;
                }
                // A switch proves only the discriminant. It does not prove that storage
                // reachable through the opaque value stayed unchanged before this edge.
                if pinned[arg.0 as usize] {
                    continue 'params;
                }
                // A `Switch` case over the argument's own discriminant proves the variant on
                // that edge; the default edge names no variant and refuses.
                let RTerm::Switch {
                    scrut,
                    scrut_ty,
                    cases,
                    ..
                } = &pred.term
                else {
                    continue 'params;
                };
                if discr_base.get(scrut) != Some(&arg) || edge >= cases.len() {
                    continue 'params;
                }
                let arg_ty = func.values[arg.0 as usize].ty;
                let variants = table.variants(table.peeled(arg_ty));
                let Some(discr) = variants.get(k as usize).map(|v| v.logical_discr) else {
                    continue 'params;
                };
                let width = table
                    .int_info(*scrut_ty)
                    .map_or(16, |(_, bits)| (bits / 8) as u8);
                if mask(cases[edge].0, width) != mask(discr as u128, width) {
                    continue 'params;
                }
                plans.push((pred_index, edge, Plan::Split { arg }));
            }
            if !direct || projections.is_empty() {
                continue;
            }

            // The rewrite. Params first: every projection result becomes a parameter, in
            // projection order, and the projection ops are deleted (descending op order keeps
            // the indices valid within a block).
            let new_params: Vec<ValueId> = projections
                .iter()
                .map(|&(_, _, _, result, _)| result)
                .collect();
            func.blocks[merge_index]
                .params
                .splice(position..=position, new_params);
            let mut by_block: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
            for &(block_index, op_index, _, _, _) in projections {
                by_block.entry(block_index).or_default().push(op_index);
            }
            for (block_index, mut ops) in by_block {
                ops.sort_unstable_by(|a, b| b.cmp(a));
                for op_index in ops {
                    func.blocks[block_index].ops.remove(op_index);
                }
            }
            // The edges.
            let origin = projections[0].4;
            for (pred_index, edge, plan) in plans {
                match plan {
                    Plan::Direct(values) => {
                        edge_args_mut(&mut func.blocks[pred_index].term)[edge]
                            .1
                            .splice(position..=position, values);
                    }
                    Plan::Split { arg } => {
                        // A new block on the edge alone: the field reads the switch proved,
                        // then the jump the edge was making, with the fields in the object's
                        // place. One read per distinct field; a repeated projection binds the
                        // same read twice.
                        let mut args: Vec<ValueId> =
                            func.blocks[pred_index].term.edges()[edge].1.to_vec();
                        let mut values = Vec::with_capacity(projections.len());
                        let mut ops = Vec::with_capacity(projections.len());
                        for &(_, _, field, result, _) in projections {
                            let value = ValueId(func.values.len() as u32);
                            func.values.push(ValueDef {
                                ty: func.values[result.0 as usize].ty,
                                carrier: Carrier::Baseline,
                                origin,
                            });
                            ops.push(Op {
                                result: Some(value),
                                origin,
                                kind: OpKind::Field {
                                    base: arg,
                                    variant: Some(k),
                                    field,
                                },
                            });
                            values.push(value);
                        }
                        args.splice(position..=position, values);
                        let split_id = RBlockId(func.blocks.len() as u32);
                        func.blocks.push(RBlock {
                            id: split_id,
                            params: Vec::new(),
                            ops,
                            term: RTerm::Goto(merge_id, args),
                        });
                        let term = &mut func.blocks[pred_index].term;
                        edge_args_mut(term)[edge].1.clear();
                        retarget_edge(term, edge, split_id);
                    }
                }
            }
            return true;
        }
    }
    false
}

fn retarget_edge(term: &mut RTerm, edge: usize, to: RBlockId) {
    match term {
        RTerm::Goto(target, _) => *target = to,
        RTerm::Invoke { normal, unwind } => {
            if edge == 0 {
                normal.0 = to;
            } else {
                unwind.0 = to;
            }
        }
        RTerm::Switch { cases, default, .. } => {
            if edge < cases.len() {
                cases[edge].1 = to;
            } else {
                default.0 = to;
            }
        }
        RTerm::Return(_) | RTerm::Unreachable | RTerm::Throw(_) => {}
    }
}

/// Deletes the blocks the entry does not reach and renumbers the rest densely.
///
/// Dense, index-equal block ids are a verifier rule, so the renumber is not optional. Values
/// defined only in a deleted block keep their slot in `Func::values`: an SSA value with no
/// definition site is only an error where it is used, and nothing uses these.
fn remove_unreachable_blocks(func: &mut Func) -> bool {
    let mut seen = vec![false; func.blocks.len()];
    seen[0] = true;
    let mut stack = vec![0usize];
    while let Some(block) = stack.pop() {
        for successor in func.blocks[block].term.successors() {
            let index = successor.0 as usize;
            if !seen[index] {
                seen[index] = true;
                stack.push(index);
            }
        }
    }
    if seen.iter().all(|reached| *reached) {
        return false;
    }
    let mut remap: Vec<Option<RBlockId>> = vec![None; func.blocks.len()];
    let mut next = 0u32;
    for (index, reached) in seen.iter().enumerate() {
        if *reached {
            remap[index] = Some(RBlockId(next));
            next += 1;
        }
    }
    let mut kept = Vec::with_capacity(next as usize);
    for (index, mut block) in std::mem::take(&mut func.blocks).into_iter().enumerate() {
        if !seen[index] {
            continue;
        }
        block.id = remap[index].expect("a kept block has a new id");
        for target in term_targets_mut(&mut block.term) {
            // A successor of a reachable block is reachable, so the remap always has it.
            *target = remap[target.0 as usize].expect("a successor of a reachable block is kept");
        }
        kept.push(block);
    }
    func.blocks = kept;
    true
}

/// Every successor id of a terminator, mutably.
fn term_targets_mut(term: &mut RTerm) -> Vec<&mut RBlockId> {
    match term {
        RTerm::Goto(target, _) => vec![target],
        RTerm::Invoke { normal, unwind } => vec![&mut normal.0, &mut unwind.0],
        RTerm::Switch { cases, default, .. } => {
            let mut targets: Vec<&mut RBlockId> =
                cases.iter_mut().map(|(_, target, _)| target).collect();
            targets.push(&mut default.0);
            targets
        }
        RTerm::Return(_) | RTerm::Unreachable | RTerm::Throw(_) => Vec::new(),
    }
}

/// Deletes ops whose result nothing reads and whose own work is `Const` or `Read`.
///
/// Iterated to a fixpoint here rather than left to the outer rounds, so that a chain of dead
/// definitions longer than [`ROUNDS`] goes in one call. `pure_call` is the D7 extension: a
/// `Call` with an unused result is `Effect` to [`op_purity`], but an oracle backed by the
/// effects summaries can vouch that a particular callee neither writes nor observes, and then
/// the dead call goes too.
fn remove_dead_ops(func: &mut Func, pure_call: Option<&dyn Fn(&Callee) -> bool>) -> bool {
    let mut changed = false;
    loop {
        let mut used = vec![false; func.values.len()];
        let mut used_cells = vec![false; func.cells.len()];
        let mark = |v: ValueId, used: &mut Vec<bool>| {
            if let Some(slot) = used.get_mut(v.0 as usize) {
                *slot = true;
            }
        };
        let mark_cell = |c: CellId, used_cells: &mut Vec<bool>| {
            if let Some(slot) = used_cells.get_mut(c.0 as usize) {
                *slot = true;
            }
        };
        for block in &func.blocks {
            // The final operation and Invoke form one exception boundary, even when a purity
            // oracle says the call has no ordinary effects and its result is otherwise dead.
            if matches!(block.term, RTerm::Invoke { .. }) {
                if let Some(result) = block.ops.last().and_then(|op| op.result) {
                    mark(result, &mut used);
                }
            }
            for op in &block.ops {
                match &op.kind {
                    OpKind::CellLoad(cell) | OpKind::CellStore { cell, .. } => {
                        mark_cell(*cell, &mut used_cells);
                    }
                    OpKind::BorrowView {
                        root: Root::Cell(cell),
                        ..
                    }
                    | OpKind::RawView {
                        root: Root::Cell(cell),
                        ..
                    }
                    | OpKind::Drop {
                        root: Root::Cell(cell),
                        ..
                    } => {
                        mark_cell(*cell, &mut used_cells);
                    }
                    _ => {}
                }
                for v in op.kind.uses() {
                    mark(v, &mut used);
                }
            }
            for v in block.term.uses() {
                mark(v, &mut used);
            }
        }
        let mut round = false;
        for block in &mut func.blocks {
            let before = block.ops.len();
            block.ops.retain(|op| match op.result {
                Some(result) if !used[result.0 as usize] => match &op.kind {
                    // Only a plain call is the oracle's to vouch for. A call the lowering
                    // intercepts — a host payload, a `rcgjs.tc.*`/`rcgjs.host.*` marker, raw
                    // JavaScript — is summarized by its Rust body, which is NOT what runs: the
                    // DOM lowering replaces it with template/effect emission whose deletion is
                    // observable (the trace regression this guard pins). The inliner's
                    // `collect_sites` spells the same refusal.
                    OpKind::Call {
                        callee,
                        host: None,
                        semantic:
                            CallSemantic::Ordinary
                            | CallSemantic::Standard(_)
                            | CallSemantic::GlobalAllocator,
                        ..
                    } => !pure_call.is_some_and(|oracle| oracle(callee)),
                    OpKind::Call { .. } => true,
                    kind => op_purity(kind) == Purity::Effect,
                },
                None => !matches!(
                    op.kind,
                    OpKind::CellAlloc(cell) if !used_cells.get(cell.0 as usize).copied().unwrap_or(true)
                ),
                _ => true,
            });
            round |= block.ops.len() != before;
        }
        changed |= round;
        if !round {
            return changed;
        }
    }
}

/// What a trivial parameter is replaced by.
enum Replacement {
    /// Every edge passes this one value.
    Value(ValueId),
    /// Every edge passes some `Const` op with this same tree at this same type; a fresh `Const`
    /// at the block's head stands in for all of them, since none of the originals need dominate
    /// the block.
    Constant(ConstTree, TypeId),
}

/// Replaces a block parameter whose incoming edges all pass one value by that value.
///
/// The entry block is never touched: its parameters are the function's, and `Func::verify` holds
/// them equal to `sig.abi.parameters`. A parameter that is passed itself on a back edge and one
/// value everywhere else is trivial — that is the loop-invariant case rule 5 exists for. Edges
/// that pass equal constant *values* through different `Const` ops (same tree, same type) are
/// one value too: the merge folds to a fresh constant at the block's head.
fn remove_trivial_params(func: &mut Func, constant_merge: bool) -> bool {
    // Incoming edge arguments per block, in (predecessor, edge) order. Only the values matter.
    let mut incoming: Vec<Vec<Vec<ValueId>>> = vec![Vec::new(); func.blocks.len()];
    for block in &func.blocks {
        for (target, args) in block.term.edges() {
            incoming[target.0 as usize].push(args.to_vec());
        }
    }

    // How many places name each value, and whether a value is a parameter or a constant. Both
    // decide whether the replacement below is worth making; see `stays_named`. `constants` is
    // what lets two different `Const` ops count as one incoming value.
    let mut uses = vec![0usize; func.values.len()];
    let mut leaf = vec![false; func.values.len()];
    let mut is_view = vec![false; func.values.len()];
    let mut constants: BTreeMap<ValueId, &ConstTree> = BTreeMap::new();
    for block in &func.blocks {
        for param in &block.params {
            leaf[param.0 as usize] = true;
        }
        for op in &block.ops {
            if let Some(result) = op.result {
                match &op.kind {
                    OpKind::Const(tree) => {
                        leaf[result.0 as usize] = true;
                        constants.insert(result, tree);
                    }
                    OpKind::BorrowView { .. } | OpKind::RawView { .. } => {
                        is_view[result.0 as usize] = true;
                    }
                    _ => {}
                }
            }
            for v in op.kind.uses() {
                uses[v.0 as usize] += 1;
            }
        }
        for v in block.term.uses() {
            uses[v.0 as usize] += 1;
        }
    }

    // An unused non-entry parameter needs no replacement at all. Drop its incoming argument
    // slot unconditionally; the ordinary dead-op sweep then removes the value chain that fed it.
    let dead_params: BTreeSet<(usize, usize)> = func
        .blocks
        .iter()
        .skip(1)
        .flat_map(|block| {
            block
                .params
                .iter()
                .enumerate()
                .filter(|(_, param)| uses[param.0 as usize] == 0)
                .map(|(position, _)| (block.id.0 as usize, position))
                .collect::<Vec<_>>()
        })
        .collect();

    // Every parameter that could go, and what would replace it.
    let mut candidates: Vec<(usize, usize, ValueId, Replacement)> = Vec::new();
    for block in func.blocks.iter().skip(1) {
        let index = block.id.0 as usize;
        if incoming[index].is_empty() {
            continue;
        }
        for (position, param) in block.params.iter().enumerate() {
            if dead_params.contains(&(index, position)) {
                continue;
            }
            // The incoming values other than the parameter itself, one per edge.
            let mut passed = Vec::with_capacity(incoming[index].len());
            let mut complete = true;
            for args in &incoming[index] {
                match args.get(position) {
                    None => {
                        complete = false;
                        break;
                    }
                    Some(arg) if arg == param => {}
                    Some(arg) => passed.push(*arg),
                }
            }
            let (true, Some(&first)) = (complete, passed.first()) else {
                continue;
            };
            if passed.iter().all(|arg| *arg == first) {
                if stays_named(&uses, &leaf, first, passed.len())
                    || (passed.len() == 1
                        && uses[first.0 as usize] == 1
                        && is_view[first.0 as usize])
                {
                    candidates.push((index, position, *param, Replacement::Value(first)));
                }
            } else if let (true, Some(&tree)) = (constant_merge, constants.get(&first)) {
                let ty = func.values[first.0 as usize].ty;
                let equal_constants = passed.iter().all(|arg| {
                    func.values[arg.0 as usize].ty == ty
                        && constants.get(arg).is_some_and(|other| *other == tree)
                });
                if equal_constants
                    && func.values[param.0 as usize].ty == ty
                    && func.values[param.0 as usize].carrier == crate::jir::repr::Carrier::Baseline
                {
                    candidates.push((
                        index,
                        position,
                        *param,
                        Replacement::Constant(tree.clone(), ty),
                    ));
                }
            }
        }
    }

    // A parameter replaced by another parameter that is itself going would end up standing for
    // whatever THAT one stands for, which is how the guard above gets bypassed — and a cycle of
    // two such parameters would leave both undefined. Neither is worth chasing: the round loop
    // above runs this pass again, and the chain resolves one link per round.
    let going: BTreeSet<ValueId> = candidates.iter().map(|(_, _, param, _)| *param).collect();
    let mut dropped: Vec<Vec<usize>> = vec![Vec::new(); func.blocks.len()];
    for (block, position) in &dead_params {
        dropped[*block].push(*position);
    }
    let mut inserted: Vec<Vec<Op>> = vec![Vec::new(); func.blocks.len()];
    let mut substitution: BTreeMap<ValueId, ValueId> = BTreeMap::new();
    for (index, position, param, replacement) in candidates {
        let value = match replacement {
            Replacement::Value(value) => {
                if going.contains(&value) {
                    continue;
                }
                value
            }
            Replacement::Constant(tree, ty) => {
                let fresh = ValueId(func.values.len() as u32);
                let def = func.values[param.0 as usize];
                func.values.push(crate::jir::repr::ValueDef { ty, ..def });
                inserted[index].push(Op {
                    result: Some(fresh),
                    origin: def.origin,
                    kind: OpKind::Const(tree),
                });
                fresh
            }
        };
        dropped[index].push(position);
        substitution.insert(param, value);
    }
    if substitution.is_empty() && dead_params.is_empty() {
        return false;
    }

    for positions in &mut dropped {
        positions.sort_unstable();
        positions.dedup();
    }
    for block in &mut func.blocks {
        let index = block.id.0 as usize;
        for position in dropped[index].iter().rev() {
            block.params.remove(*position);
        }
        for (offset, op) in std::mem::take(&mut inserted[index]).into_iter().enumerate() {
            block.ops.insert(offset, op);
        }
        for (target, args) in edge_args_mut(&mut block.term) {
            for position in dropped[target.0 as usize].iter().rev() {
                if *position < args.len() {
                    args.remove(*position);
                }
            }
        }
    }
    substitute(func, &substitution);
    true
}

/// Whether replacing a trivial parameter by `value` leaves `value` spelled as a name.
///
/// A block parameter is always a name, so removing one and putting an *expression* where it stood
/// hands that expression to `fuse`, which inlines a single-use value into its reader — and a reader
/// may spell its operand more than once (`js/value.rs` rebuilds an enum out of three reads of the
/// same operand). Nest that a few deep and one parameter's removal multiplies the printed text: on
/// `core-tests/39_native_vec_drop` it took `RawVec::grow_amortized` from 4.2 KB to 10.9 KB.
///
/// So the replacement is made only when it cannot start that: `value` is itself a parameter or a
/// constant (a name or a literal either way), or it has a use besides the edge arguments this
/// removes, which leaves it with two and `fuse` names anything used twice.
fn stays_named(uses: &[usize], leaf: &[bool], value: ValueId, edges: usize) -> bool {
    let index = value.0 as usize;
    leaf.get(index).copied().unwrap_or(false) || uses.get(index).copied().unwrap_or(0) > edges
}

/// Every edge of a terminator as its target and its argument list, mutably.
fn edge_args_mut(term: &mut RTerm) -> Vec<(RBlockId, &mut Vec<ValueId>)> {
    match term {
        RTerm::Goto(target, args) => vec![(*target, args)],
        RTerm::Invoke { normal, unwind } => {
            vec![(normal.0, &mut normal.1), (unwind.0, &mut unwind.1)]
        }
        RTerm::Switch { cases, default, .. } => {
            let mut edges: Vec<(RBlockId, &mut Vec<ValueId>)> = cases
                .iter_mut()
                .map(|(_, target, args)| (*target, args))
                .collect();
            edges.push((default.0, &mut default.1));
            edges
        }
        RTerm::Return(_) | RTerm::Unreachable | RTerm::Throw(_) => Vec::new(),
    }
}

/// Rewrites every use of a value in `map` to what it maps to.
///
/// The operand walk is `ssa::rewrite_op`/`rewrite_term`, which the SSA builder already keeps in
/// step with `OpKind`: a second copy of that match here is a second thing to forget to update.
fn substitute(func: &mut Func, map: &BTreeMap<ValueId, ValueId>) {
    let rewrite = |v: ValueId| map.get(&v).copied().unwrap_or(v);
    for block in &mut func.blocks {
        for op in &mut block.ops {
            crate::lower::ssa::rewrite_op(&mut op.kind, &rewrite);
        }
        crate::lower::ssa::rewrite_term(&mut block.term, &rewrite);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::repr::FuncBuilder;
    use crate::jir::semantic::{
        AbiSignature, BinaryOp, CallSemantic, Callee, DefinitionId, ItemId, Signature, TypeId,
    };
    use crate::jir::types::{
        IntFacts, IntRepr, JsFacts, LayoutFacts, TypeDescriptor, TypeKind, TypeMeta,
    };
    use crate::jir::{OriginId, StrId};

    const I32: TypeId = TypeId(1);
    const NO_ORIGIN: OriginId = OriginId(0);

    /// A table with one type: `i32`, which is what every value below is.
    fn table() -> TypeTable {
        let mut table = TypeTable::new();
        table.insert(TypeDescriptor {
            id: I32,
            name: None,
            kind: TypeKind::Integer {
                signed: true,
                bits: 32,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: Some(IntFacts {
                    signed: true,
                    bits: 32,
                    repr: IntRepr::Number,
                }),
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: I32,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
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
                result: I32,
            },
        )
    }

    fn constant(value: u128) -> OpKind {
        OpKind::Const(ConstTree::Scalar {
            bits: value,
            bytes: 4,
        })
    }

    /// A projected unsigned checked add stays entirely scalar.
    #[test]
    fn unsigned_checked_add_pair_is_split_before_lowering() {
        use crate::jir::types::{FieldDescriptor, FieldKey};

        const U32: TypeId = TypeId(3);
        const BOOL: TypeId = TypeId(4);
        const CHECKED: TypeId = TypeId(5);

        let scalar = |id, kind, int| TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int,
                is_indirect: false,
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
        };
        let mut types = TypeTable::new();
        types.insert(scalar(
            U32,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            Some(IntFacts {
                signed: false,
                bits: 32,
                repr: IntRepr::Number,
            }),
        ));
        types.insert(scalar(BOOL, TypeKind::Bool, None));
        let mut checked = scalar(CHECKED, TypeKind::Tuple(vec![U32, BOOL]), None);
        checked.js.is_indirect = true;
        checked.js.needs_clone = true;
        checked.js.fields = vec![
            FieldDescriptor {
                key: FieldKey::Index(0),
                ty: U32,
                offset: 0,
            },
            FieldDescriptor {
                key: FieldKey::Index(1),
                ty: BOOL,
                offset: 4,
            },
        ];
        types.insert(checked);

        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let lhs = b.op(entry, constant(u128::from(u32::MAX)), U32, NO_ORIGIN);
        let rhs = b.op(entry, constant(1), U32, NO_ORIGIN);
        let pair = b.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::AddWithOverflow,
                l: lhs,
                r: rhs,
                ty: U32,
            },
            CHECKED,
            NO_ORIGIN,
        );
        let wrapped = b.op(
            entry,
            OpKind::Field {
                base: pair,
                variant: None,
                field: 0,
            },
            U32,
            NO_ORIGIN,
        );
        let overflow = b.op(
            entry,
            OpKind::Field {
                base: pair,
                variant: None,
                field: 1,
            },
            BOOL,
            NO_ORIGIN,
        );
        let output = b.op(
            entry,
            OpKind::Aggregate {
                ty: CHECKED,
                variant: 0,
                active_field: None,
                fields: vec![wrapped, overflow],
            },
            CHECKED,
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(Some(output)));
        let mut func = b.finish();
        func.sig.result = CHECKED;
        let mut witnesses = Vec::new();

        assert!(run_with(
            &mut func,
            &types,
            &Options::default(),
            &mut witnesses
        ));
        func.verify(&types).expect("split checked add verifies");
        assert!(witnesses
            .iter()
            .any(|witness| matches!(witness, Witness::UnsignedAddOverflowSplit { bits: 32 })));
        assert!(!func.blocks[0].ops.iter().any(|op| matches!(
            op.kind,
            OpKind::Binary {
                op: BinaryOp::AddWithOverflow,
                ..
            } | OpKind::Field { .. }
        )));
        assert!(func.blocks[0].ops.iter().any(|op| matches!(
            op.kind,
            OpKind::Binary {
                op: BinaryOp::Lt,
                ..
            }
        )));
        super::super::witness::check_all(&witnesses, &func, &types).expect("split witness checks");
    }

    /// A `switch` on a constant takes one edge, and the arms it did not take go away with the
    /// blocks that held them.
    #[test]
    fn a_constant_switch_folds_and_its_dead_arm_goes() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let scrut = b.op(entry, constant(1), I32, NO_ORIGIN);
        let taken = b.block();
        let dead = b.block();
        b.terminate(
            entry,
            RTerm::Switch {
                scrut,
                scrut_ty: I32,
                cases: vec![(1, taken, Vec::new())],
                default: (dead, Vec::new()),
            },
        );
        let one = b.op(taken, constant(7), I32, NO_ORIGIN);
        b.terminate(taken, RTerm::Return(Some(one)));
        b.terminate(dead, RTerm::Unreachable);
        let mut func = b.finish();

        assert!(run(&mut func, &table()));
        assert_eq!(func.blocks.len(), 2, "the default arm is unreachable");
        assert!(matches!(func.blocks[0].term, RTerm::Goto(RBlockId(1), _)));
        // Block ids stay dense and equal to their index, which the verifier requires.
        for (index, block) in func.blocks.iter().enumerate() {
            assert_eq!(block.id.0 as usize, index);
        }
    }

    /// An assert whose constant condition equals `expected` can never fire and goes; one whose
    /// constant disagrees always fires and stays, and one on a non-constant is never touched.
    #[test]
    fn settled_asserts_go_and_live_ones_stay() {
        use crate::jir::repr::RAssertMessage;
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let truth = b.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 1 }),
            I32,
            NO_ORIGIN,
        );
        let falsity = b.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 1 }),
            I32,
            NO_ORIGIN,
        );
        // Not a `Const` op, so the pass has no verdict on it.
        let unknown = b.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: truth,
                r: falsity,
                ty: I32,
            },
            I32,
            NO_ORIGIN,
        );
        let assert = |cond, expected| OpKind::Assert {
            cond,
            expected,
            msg: RAssertMessage::NullPointer,
            track_caller: None,
        };
        b.effect(entry, assert(truth, true), NO_ORIGIN); // settled: goes
        b.effect(entry, assert(falsity, false), NO_ORIGIN); // settled: goes
        b.effect(entry, assert(truth, false), NO_ORIGIN); // always fires: stays
        b.effect(entry, assert(unknown, true), NO_ORIGIN); // not constant: stays
        let zero = b.op(entry, constant(0), I32, NO_ORIGIN);
        b.terminate(entry, RTerm::Return(Some(zero)));
        let mut func = b.finish();

        assert!(run(&mut func, &table()));
        let kept: Vec<bool> = func.blocks[0]
            .ops
            .iter()
            .filter_map(|op| match &op.kind {
                OpKind::Assert { expected, .. } => Some(*expected),
                _ => None,
            })
            .collect();
        assert_eq!(
            kept,
            vec![false, true],
            "the always-firing and the non-constant assert survive"
        );
    }

    /// An unused pure op goes; an effectful one stays however unused its result is.
    #[test]
    fn dead_pure_ops_go_and_effects_stay() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let unused = b.op(entry, constant(1), I32, NO_ORIGIN);
        // A chain: `dead` reads `unused`, and nothing reads `dead`. Both go, in one call.
        let _dead = b.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: unused,
                r: unused,
                ty: I32,
            },
            I32,
            NO_ORIGIN,
        );
        let _call = b.op(
            entry,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 0,
                            local_hash: 1,
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
            I32,
            NO_ORIGIN,
        );
        let kept = b.op(entry, constant(3), I32, NO_ORIGIN);
        b.terminate(entry, RTerm::Return(Some(kept)));
        let mut func = b.finish();

        assert!(run(&mut func, &table()));
        assert_eq!(func.blocks[0].ops.len(), 2, "the call and the return value");
        assert!(matches!(func.blocks[0].ops[0].kind, OpKind::Call { .. }));
    }

    /// A loop-carried parameter whose only other incoming value is one constant is trivial: the
    /// self reference on the back edge does not count against it.
    #[test]
    fn a_loop_invariant_parameter_is_replaced_by_its_one_value() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let seed = b.op(entry, constant(5), I32, NO_ORIGIN);
        let header = b.block();
        let param = b.block_param(header, I32, NO_ORIGIN);
        let exit = b.block();
        b.terminate(entry, RTerm::Goto(header, vec![seed]));
        // `header` loops back to itself passing the parameter, or leaves with it.
        b.terminate(
            header,
            RTerm::Switch {
                scrut: param,
                scrut_ty: I32,
                cases: vec![(0, header, vec![param])],
                default: (exit, Vec::new()),
            },
        );
        b.terminate(exit, RTerm::Return(Some(param)));
        let mut func = b.finish();

        assert!(run(&mut func, &table()));
        assert!(func.blocks[1].params.is_empty(), "the parameter is trivial");
        assert!(matches!(func.blocks[0].term, RTerm::Goto(_, ref args) if args.is_empty()));
        assert!(matches!(func.blocks[2].term, RTerm::Return(Some(v)) if v == seed));
    }

    /// A used trivial parameter and an unused neighbor are removed in one stable positional edit.
    #[test]
    fn a_dead_parameter_and_a_trivial_neighbor_drop_together() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let kept = b.op(entry, constant(5), I32, NO_ORIGIN);
        let unused = b.op(entry, constant(9), I32, NO_ORIGIN);
        let join = b.block();
        let kept_param = b.block_param(join, I32, NO_ORIGIN);
        let _dead_param = b.block_param(join, I32, NO_ORIGIN);
        b.terminate(entry, RTerm::Goto(join, vec![kept, unused]));
        b.terminate(join, RTerm::Return(Some(kept_param)));
        let mut func = b.finish();

        assert!(run(&mut func, &table()));
        assert!(func.blocks[1].params.is_empty());
        assert!(matches!(func.blocks[0].term, RTerm::Goto(_, ref args) if args.is_empty()));
        assert!(matches!(func.blocks[1].term, RTerm::Return(Some(value)) if value == kept));
        func.verify(&table()).unwrap();
    }

    /// `assert!(0 < size && size <= isize::MAX)` is two literal comparisons after
    /// monomorphization. v1 folded them before it took the item's references, so a lane that did
    /// not would reference a `panic` the optimized program no longer contains.
    #[test]
    fn a_literal_comparison_folds_and_takes_the_panic_arm_with_it() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let zero = b.op(entry, constant(0), I32, NO_ORIGIN);
        let four = b.op(entry, constant(4), I32, NO_ORIGIN);
        let less = b.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Lt,
                l: zero,
                r: four,
                ty: I32,
            },
            I32,
            NO_ORIGIN,
        );
        let ok = b.block();
        let panics = b.block();
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: less,
                scrut_ty: I32,
                // `0` is the false arm; a `bool` switch's default is the true one.
                cases: vec![(0, panics, Vec::new())],
                default: (ok, Vec::new()),
            },
        );
        let result = b.op(ok, constant(7), I32, NO_ORIGIN);
        b.terminate(ok, RTerm::Return(Some(result)));
        b.terminate(panics, RTerm::Unreachable);
        let mut func = b.finish();

        assert!(run(&mut func, &table()));
        assert_eq!(func.blocks.len(), 2, "the panic arm is unreachable");
        // A signed comparison reads its operands sign-extended: `-1 < 4`, not `0xffff_ffff < 4`.
        assert!(signed_value(u128::from(u32::MAX), 32) < 0);
    }

    /// `FuncBuilder::finish` with the JavaScript parameter count synced to the parameters the
    /// test actually added, which `Func::verify` checks.
    fn finished(b: FuncBuilder) -> Func {
        let mut func = b.finish();
        func.sig.abi.parameters = func.params.len() as u16;
        func
    }

    const OPTION: TypeId = TypeId(2);

    /// The [`table`] plus an `Option`-shaped tagged enum: variant 0 carries nothing at
    /// discriminant 0, variant 1 carries one `i32` at discriminant 1.
    fn option_table() -> TypeTable {
        use crate::jir::types::{
            FieldDescriptor, FieldKey, NominalDiscriminant, VariantDescriptor,
        };
        let mut table = table();
        let tag = |bits| Some(NominalDiscriminant { bits, bytes: 1 });
        table.insert(TypeDescriptor {
            id: OPTION,
            name: None,
            kind: TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 7,
                },
                args: Vec::new(),
                form: crate::jir::types::NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: StrId(0),
                        fields: Vec::new(),
                        logical_discr: 0,
                        physical_tag: tag(0),
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: StrId(0),
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Index(0),
                            ty: I32,
                            offset: 0,
                        }],
                        logical_discr: 1,
                        physical_tag: tag(1),
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
                niche: None,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: true,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: OPTION,
                unsized_tail: None,
                enum_repr: Some(crate::jir::types::EnumRepr::Tag { fieldless: false }),
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });
        table
    }

    /// The merge-then-switch shape: an option built as `Some` on one edge and `None` on the
    /// other, discriminated immediately at the merge. Returns the builder plus the ids of the
    /// creation blocks and the option values, with the merge at the returned block id.
    fn merge_shape(b: &mut FuncBuilder) -> (RBlockId, RBlockId, ValueId, ValueId, RBlockId) {
        let entry = FuncBuilder::ENTRY;
        let scrut = b.param(I32, NO_ORIGIN);
        let some_bb = b.block();
        let none_bb = b.block();
        let merge = b.block();
        let param = b.block_param(merge, OPTION, NO_ORIGIN);
        let some_arm = b.block();
        let arm_param = b.block_param(some_arm, OPTION, NO_ORIGIN);
        let none_arm = b.block();
        b.terminate(
            entry,
            RTerm::Switch {
                scrut,
                scrut_ty: I32,
                cases: vec![(0, none_bb, Vec::new())],
                default: (some_bb, Vec::new()),
            },
        );
        let payload = b.op(some_bb, constant(7), I32, NO_ORIGIN);
        let some_value = b.op(
            some_bb,
            OpKind::Aggregate {
                ty: OPTION,
                variant: 1,
                active_field: None,
                fields: vec![payload],
            },
            OPTION,
            NO_ORIGIN,
        );
        b.terminate(some_bb, RTerm::Goto(merge, vec![some_value]));
        let none_value = b.op(
            none_bb,
            OpKind::Aggregate {
                ty: OPTION,
                variant: 0,
                active_field: None,
                fields: Vec::new(),
            },
            OPTION,
            NO_ORIGIN,
        );
        b.terminate(none_bb, RTerm::Goto(merge, vec![none_value]));
        let discr = b.op(merge, OpKind::Discriminant(param), I32, NO_ORIGIN);
        b.terminate(
            merge,
            RTerm::Switch {
                scrut: discr,
                scrut_ty: I32,
                cases: vec![(0, none_arm, Vec::new())],
                default: (some_arm, vec![param]),
            },
        );
        let field = b.op(
            some_arm,
            OpKind::Field {
                base: arm_param,
                variant: Some(1),
                field: 0,
            },
            I32,
            NO_ORIGIN,
        );
        b.terminate(some_arm, RTerm::Return(Some(field)));
        let fallback = b.op(none_arm, constant(9), I32, NO_ORIGIN);
        b.terminate(none_arm, RTerm::Return(Some(fallback)));
        (some_bb, none_bb, some_value, none_value, merge)
    }

    /// How many `Discriminant` ops the function still holds.
    fn discriminants(func: &Func) -> usize {
        func.blocks
            .iter()
            .flat_map(|block| &block.ops)
            .filter(|op| matches!(op.kind, OpKind::Discriminant(_)))
            .count()
    }

    /// Both creation edges of a merge-then-switch jump straight to their arms, the merge dies,
    /// and the function still verifies.
    #[test]
    fn a_variant_merge_is_threaded_per_edge() {
        let mut b = builder();
        merge_shape(&mut b);
        let mut func = finished(b);
        let table = option_table();

        assert!(run(&mut func, &table));
        assert_eq!(discriminants(&func), 0, "the merge block is gone");
        func.verify(&table).expect("threaded function verifies");
        // The `Some` path returns the payload's constant, the `None` path the fallback: both
        // arms are reached directly now, so both return constants survive.
        let returned: Vec<ValueId> = func
            .blocks
            .iter()
            .filter_map(|block| match block.term {
                RTerm::Return(Some(v)) => Some(v),
                _ => None,
            })
            .collect();
        assert_eq!(returned.len(), 2, "both arms stay reachable");
    }

    #[test]
    fn threading_preserves_the_private_normal_successor() {
        let mut b = builder();
        let (some, other, some_value, other_value, normal) = merge_shape(&mut b);
        let cleanup = b.block();
        let payload = b.op(other, constant(9), I32, NO_ORIGIN);
        b.effect(
            some,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 0,
                            local_hash: 1,
                        },
                        instantiation: 0,
                    },
                    link_name: StrId(0),
                },
                function: None,
                args: vec![],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
        b.terminate(
            some,
            RTerm::Invoke {
                normal: (normal, vec![some_value]),
                unwind: (cleanup, vec![]),
            },
        );
        b.terminate(cleanup, RTerm::Throw(None));
        let mut func = finished(b);
        let RTerm::Switch {
            default: (shared, _),
            ..
        } = func.blocks[normal.0 as usize].term
        else {
            unreachable!()
        };
        let other = &mut func.blocks[other.0 as usize];
        other.ops.rotate_right(1);
        let OpKind::Aggregate {
            variant, fields, ..
        } = &mut other.ops[1].kind
        else {
            unreachable!()
        };
        *variant = 1;
        *fields = vec![payload];
        other.term = RTerm::Goto(shared, vec![other_value]);
        let table = option_table();
        func.verify(&table).unwrap();
        assert!(!thread_one_merge(&mut func, &table, &mut vec![]));
        run(&mut func, &table);
        func.verify(&table).unwrap();
    }

    /// A value whose storage escapes its ssa name (here through `SetDiscriminant`) is never
    /// threaded on, while the other edge still is.
    #[test]
    fn an_escaping_discriminant_is_not_threaded() {
        let mut b = builder();
        let (some_bb, _, some_value, _, _) = merge_shape(&mut b);
        b.effect(
            some_bb,
            OpKind::SetDiscriminant {
                base: some_value,
                variant: 1,
            },
            NO_ORIGIN,
        );
        let mut func = finished(b);
        let table = option_table();

        run(&mut func, &table);
        assert_eq!(
            discriminants(&func),
            1,
            "the merge survives for the pinned edge"
        );
        func.verify(&table).expect("still verifies");
    }

    /// A merge block with any op besides the one `Discriminant` is refused whole: retargeting
    /// would skip that op, and duplicating it is not this rule's business.
    #[test]
    fn a_merge_with_extra_work_is_not_threaded() {
        let mut b = builder();
        let (_, _, _, _, merge) = merge_shape(&mut b);
        // An extra op in the merge, kept alive by an arm's return below.
        let extra = b.op(merge, constant(5), I32, NO_ORIGIN);
        // Reroute the none arm to return the merge-defined value so nothing deletes it.
        let none_arm = RBlockId(func_none_arm(&b));
        b.terminate(none_arm, RTerm::Return(Some(extra)));
        let mut func = finished(b);
        let table = option_table();

        run(&mut func, &table);
        assert_eq!(discriminants(&func), 1, "the merge is untouched");
        func.verify(&table).expect("still verifies");
    }

    /// The none arm's block id in the [`merge_shape`] layout.
    fn func_none_arm(b: &FuncBuilder) -> u32 {
        (b.func().blocks.len() - 1) as u32
    }

    /// An arm edge that wants a value the merge defines (the discriminant itself) refuses the
    /// thread for that edge.
    #[test]
    fn an_arm_wanting_the_discriminant_is_not_threaded() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let scrut = b.param(I32, NO_ORIGIN);
        let some_bb = b.block();
        let merge = b.block();
        let param = b.block_param(merge, OPTION, NO_ORIGIN);
        let arm = b.block();
        let arm_param = b.block_param(arm, I32, NO_ORIGIN);
        b.terminate(
            entry,
            RTerm::Switch {
                scrut,
                scrut_ty: I32,
                cases: vec![(0, some_bb, Vec::new())],
                default: (some_bb, Vec::new()),
            },
        );
        let some_value = b.op(
            some_bb,
            OpKind::Aggregate {
                ty: OPTION,
                variant: 1,
                active_field: None,
                fields: Vec::new(),
            },
            OPTION,
            NO_ORIGIN,
        );
        b.terminate(some_bb, RTerm::Goto(merge, vec![some_value]));
        let discr = b.op(merge, OpKind::Discriminant(param), I32, NO_ORIGIN);
        b.terminate(
            merge,
            RTerm::Switch {
                scrut: discr,
                scrut_ty: I32,
                cases: Vec::new(),
                default: (arm, vec![discr]),
            },
        );
        b.terminate(arm, RTerm::Return(Some(arm_param)));
        let mut func = finished(b);
        let table = option_table();

        run(&mut func, &table);
        assert_eq!(discriminants(&func), 1, "the edge is refused");
        func.verify(&table).expect("still verifies");
    }

    /// Two edges passing the same constant value through two different `Const` ops fold like a
    /// single value: the parameter goes and a constant at the block's head stands in.
    #[test]
    fn a_parameter_fed_one_constant_value_twice_folds() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let scrut = b.param(I32, NO_ORIGIN);
        let left = b.block();
        let right = b.block();
        let join = b.block();
        let param = b.block_param(join, I32, NO_ORIGIN);
        b.terminate(
            entry,
            RTerm::Switch {
                scrut,
                scrut_ty: I32,
                cases: vec![(0, left, Vec::new())],
                default: (right, Vec::new()),
            },
        );
        let a = b.op(left, constant(7), I32, NO_ORIGIN);
        b.terminate(left, RTerm::Goto(join, vec![a]));
        let c = b.op(right, constant(7), I32, NO_ORIGIN);
        b.terminate(right, RTerm::Goto(join, vec![c]));
        b.terminate(join, RTerm::Return(Some(param)));
        let mut func = finished(b);

        assert!(run(&mut func, &table()));
        let join_block = &func.blocks[3];
        assert!(join_block.params.is_empty(), "the parameter folded");
        assert!(
            matches!(join_block.term, RTerm::Return(Some(v))
            if matches!(
                func.blocks.iter().flat_map(|b| &b.ops).find(|op| op.result == Some(v)),
                Some(op) if matches!(op.kind, OpKind::Const(_))
            )),
            "the return reads a constant"
        );
        func.verify(&table()).expect("still verifies");
    }

    /// Two different constant values do not fold, exactly as two different values never did.
    #[test]
    fn different_constant_values_do_not_fold() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let scrut = b.param(I32, NO_ORIGIN);
        let left = b.block();
        let right = b.block();
        let join = b.block();
        let param = b.block_param(join, I32, NO_ORIGIN);
        b.terminate(
            entry,
            RTerm::Switch {
                scrut,
                scrut_ty: I32,
                cases: vec![(0, left, Vec::new())],
                default: (right, Vec::new()),
            },
        );
        let a = b.op(left, constant(7), I32, NO_ORIGIN);
        b.terminate(left, RTerm::Goto(join, vec![a]));
        let c = b.op(right, constant(8), I32, NO_ORIGIN);
        b.terminate(right, RTerm::Goto(join, vec![c]));
        b.terminate(join, RTerm::Return(Some(param)));
        let mut func = finished(b);

        assert!(!run(&mut func, &table()), "nothing here folds");
        assert_eq!(func.blocks[3].params.len(), 1);
    }

    /// A dead call goes only when the oracle vouches for its callee, and stays without one.
    #[test]
    fn a_dead_call_needs_the_purity_oracle() {
        let call = || OpKind::Call {
            callee: Callee::Direct {
                item: ItemId {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 1,
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
        };
        let build = || {
            let mut b = builder();
            let entry = FuncBuilder::ENTRY;
            let _dead = b.op(entry, call(), I32, NO_ORIGIN);
            let kept = b.op(entry, constant(3), I32, NO_ORIGIN);
            b.terminate(entry, RTerm::Return(Some(kept)));
            b.finish()
        };

        let mut without = build();
        run(&mut without, &table());
        assert_eq!(without.blocks[0].ops.len(), 2, "no oracle, the call stays");

        let mut with = build();
        let oracle = |_: &Callee| true;
        let options = Options {
            pure_call: Some(&oracle),
            ..Options::default()
        };
        assert!(run_with(&mut with, &table(), &options, &mut Vec::new()));
        assert_eq!(with.blocks[0].ops.len(), 1, "the oracle deletes the call");
        assert!(matches!(with.blocks[0].ops[0].kind, OpKind::Const(_)));
    }

    #[test]
    fn a_dead_protected_call_keeps_its_exception_boundary() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let normal = b.block();
        let cleanup = b.block();
        let kept = b.op(entry, constant(3), I32, NO_ORIGIN);
        b.op(entry, constant(9), I32, NO_ORIGIN);
        b.op(
            entry,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 0,
                            local_hash: 1,
                        },
                        instantiation: 0,
                    },
                    link_name: StrId(0),
                },
                function: None,
                args: vec![],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            NO_ORIGIN,
        );
        b.terminate(
            entry,
            RTerm::Invoke {
                normal: (normal, vec![]),
                unwind: (cleanup, vec![]),
            },
        );
        b.terminate(normal, RTerm::Return(Some(kept)));
        b.terminate(cleanup, RTerm::Throw(None));
        let mut func = b.finish();
        func.verify(&table()).unwrap();
        let oracle = |_: &Callee| true;
        let options = Options {
            pure_call: Some(&oracle),
            ..Options::default()
        };
        assert!(run_with(&mut func, &table(), &options, &mut vec![]));
        func.verify(&table()).unwrap();
        assert_eq!(func.blocks[0].ops.len(), 2);
        assert!(matches!(
            func.blocks[0].ops.last().unwrap().kind,
            OpKind::Call { .. }
        ));
        assert!(matches!(func.blocks[0].term, RTerm::Invoke { .. }));
    }

    /// An intercepted call — a host-semantic callee here — is never the oracle's to vouch
    /// dead, purity notwithstanding: the summary describes the Rust body, and the lowering
    /// replaces that body with observable template/effect emission.
    #[test]
    fn the_pure_call_oracle_never_vouches_an_intercepted_call() {
        let call = || OpKind::Call {
            callee: Callee::Direct {
                item: ItemId {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 1,
                    },
                    instantiation: 0,
                },
                link_name: StrId(0),
            },
            function: None,
            args: Vec::new(),
            track_caller: None,
            semantic: CallSemantic::ViewAbi,
            host: None,
        };
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let _dead = b.op(entry, call(), I32, NO_ORIGIN);
        let kept = b.op(entry, constant(3), I32, NO_ORIGIN);
        b.terminate(entry, RTerm::Return(Some(kept)));
        let mut func = b.finish();

        let oracle = |_: &Callee| true;
        let options = Options {
            pure_call: Some(&oracle),
            ..Options::default()
        };
        run_with(&mut func, &table(), &options, &mut Vec::new());
        assert_eq!(
            func.blocks[0].ops.len(),
            2,
            "the intercepted call stays, oracle or no oracle"
        );
    }

    /// Every fold records a witness, the witnesses check against the final function, and a
    /// corrupted one is rejected.
    #[test]
    fn folds_record_checkable_witnesses() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let zero = b.op(entry, constant(0), I32, NO_ORIGIN);
        let four = b.op(entry, constant(4), I32, NO_ORIGIN);
        let less = b.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Lt,
                l: zero,
                r: four,
                ty: I32,
            },
            I32,
            NO_ORIGIN,
        );
        let ok = b.block();
        let panics = b.block();
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: less,
                scrut_ty: I32,
                cases: vec![(0, panics, Vec::new())],
                default: (ok, Vec::new()),
            },
        );
        let result = b.op(ok, constant(7), I32, NO_ORIGIN);
        b.terminate(ok, RTerm::Return(Some(result)));
        b.terminate(panics, RTerm::Unreachable);
        let mut func = b.finish();
        let table = table();
        let mut witnesses = Vec::new();

        assert!(run_with(
            &mut func,
            &table,
            &Options::default(),
            &mut witnesses
        ));
        assert!(
            witnesses
                .iter()
                .any(|w| matches!(w, super::super::witness::Witness::ConstFold { .. })),
            "the comparison fold is witnessed"
        );
        assert!(
            witnesses
                .iter()
                .any(|w| matches!(w, super::super::witness::Witness::BranchFold { .. })),
            "the switch fold is witnessed"
        );
        super::super::witness::check_all(&witnesses, &func, &table).expect("witnesses check");

        // Corrupt the fold's recorded result: the independent evaluator disagrees.
        for witness in &mut witnesses {
            if let super::super::witness::Witness::ConstFold { result, .. } = witness {
                *result = !*result;
            }
        }
        assert!(super::super::witness::check_all(&witnesses, &func, &table).is_err());
    }

    /// A parameter that genuinely merges two values stays, and so does its edge argument. The
    /// switch is on a function parameter, so nothing folds it.
    #[test]
    fn a_real_merge_parameter_stays() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let scrut = b.param(I32, NO_ORIGIN);
        let left = b.block();
        let right = b.block();
        let join = b.block();
        let param = b.block_param(join, I32, NO_ORIGIN);
        b.terminate(
            entry,
            RTerm::Switch {
                scrut,
                scrut_ty: I32,
                cases: vec![(0, left, Vec::new())],
                default: (right, Vec::new()),
            },
        );
        let a = b.op(left, constant(1), I32, NO_ORIGIN);
        b.terminate(left, RTerm::Goto(join, vec![a]));
        let c = b.op(right, constant(2), I32, NO_ORIGIN);
        b.terminate(right, RTerm::Goto(join, vec![c]));
        b.terminate(join, RTerm::Return(Some(param)));
        let mut func = b.finish();

        assert!(!run(&mut func, &table()), "nothing here is dead");
        assert_eq!(func.blocks.len(), 4);
        assert_eq!(func.blocks[3].params, vec![param]);
        assert!(matches!(func.blocks[3].term, RTerm::Return(Some(v)) if v == param));
    }

    /// A merge parameter read only as the `Some` payload is split per edge: the creation edge
    /// passes the payload straight from its `Aggregate`, the switch-proven opaque edge reads it
    /// in a block of its own, and no option value crosses the merge any more.
    #[test]
    fn a_payload_only_merge_is_split_per_edge() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let selector = b.param(I32, NO_ORIGIN);
        // A caller-supplied option: a real value no escape analysis can virtualize, which is
        // exactly the `nth`-call shape the rule exists for.
        let opaque = b.param(OPTION, NO_ORIGIN);
        let agg_bb = b.block();
        let switch_bb = b.block();
        let merge = b.block();
        let param = b.block_param(merge, OPTION, NO_ORIGIN);
        let none_arm = b.block();
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: selector,
                scrut_ty: I32,
                cases: vec![(0, agg_bb, Vec::new())],
                default: (switch_bb, Vec::new()),
            },
        );
        let payload = b.op(agg_bb, constant(7), I32, NO_ORIGIN);
        let some_value = b.op(
            agg_bb,
            OpKind::Aggregate {
                ty: OPTION,
                variant: 1,
                active_field: None,
                fields: vec![payload],
            },
            OPTION,
            NO_ORIGIN,
        );
        b.terminate(agg_bb, RTerm::Goto(merge, vec![some_value]));
        let discr = b.op(switch_bb, OpKind::Discriminant(opaque), I32, NO_ORIGIN);
        b.terminate(
            switch_bb,
            RTerm::Switch {
                scrut: discr,
                scrut_ty: I32,
                cases: vec![(1, merge, vec![opaque])],
                default: (none_arm, Vec::new()),
            },
        );
        let field = b.op(
            merge,
            OpKind::Field {
                base: param,
                variant: Some(1),
                field: 0,
            },
            I32,
            NO_ORIGIN,
        );
        b.terminate(merge, RTerm::Return(Some(field)));
        let fallback = b.op(none_arm, constant(9), I32, NO_ORIGIN);
        b.terminate(none_arm, RTerm::Return(Some(fallback)));
        let mut func = finished(b);
        let table = option_table();

        assert!(split_payload_merges(&mut func, &table), "the rule fires");
        // The merge takes the payload now: its parameter is the old projection result, and no
        // edge passes an option-typed value into it any more. (The ids are read before
        // `compact_values` renumbers them, exactly like the pipeline's witness checks.)
        let merge = func
            .blocks
            .iter()
            .find(|block| block.params.contains(&field))
            .expect("the projection result became a parameter");
        assert!(matches!(merge.term, RTerm::Return(Some(v)) if v == field));
        for block in &func.blocks {
            for (target, args) in block.term.edges() {
                if target == merge.id {
                    assert!(
                        !args.contains(&some_value) && !args.contains(&opaque),
                        "no option crosses the merge"
                    );
                }
            }
        }
        // The opaque edge got its own block: a `Field` of the opaque value, then the jump.
        assert!(
            func.blocks.iter().any(|block| {
                block
                    .ops
                    .iter()
                    .any(|op| matches!(op.kind, OpKind::Field { base, .. } if base == opaque))
                    && matches!(block.term, RTerm::Goto(target, _) if target == merge.id)
            }),
            "the switch-proven edge reads the payload in a split block"
        );
        compact_values(&mut func);
        func.verify(&table).expect("split function verifies");
    }

    /// A switch proves the opaque argument's variant, but not that another whole-value use left
    /// the reachable storage unchanged. Such an argument must not be projected on a split edge.
    #[test]
    fn a_pinned_switch_value_is_not_split() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let selector = b.param(I32, NO_ORIGIN);
        let opaque = b.param(OPTION, NO_ORIGIN);
        let agg_bb = b.block();
        let switch_bb = b.block();
        let merge = b.block();
        let param = b.block_param(merge, OPTION, NO_ORIGIN);
        let none_arm = b.block();
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: selector,
                scrut_ty: I32,
                cases: vec![(0, agg_bb, Vec::new())],
                default: (switch_bb, Vec::new()),
            },
        );
        let payload = b.op(agg_bb, constant(7), I32, NO_ORIGIN);
        let some_value = b.op(
            agg_bb,
            OpKind::Aggregate {
                ty: OPTION,
                variant: 1,
                active_field: None,
                fields: vec![payload],
            },
            OPTION,
            NO_ORIGIN,
        );
        b.terminate(agg_bb, RTerm::Goto(merge, vec![some_value]));
        let _materialized = b.op(switch_bb, OpKind::Materialize(opaque), OPTION, NO_ORIGIN);
        let discr = b.op(switch_bb, OpKind::Discriminant(opaque), I32, NO_ORIGIN);
        b.terminate(
            switch_bb,
            RTerm::Switch {
                scrut: discr,
                scrut_ty: I32,
                cases: vec![(1, merge, vec![opaque])],
                default: (none_arm, Vec::new()),
            },
        );
        let field = b.op(
            merge,
            OpKind::Field {
                base: param,
                variant: Some(1),
                field: 0,
            },
            I32,
            NO_ORIGIN,
        );
        b.terminate(merge, RTerm::Return(Some(field)));
        let fallback = b.op(none_arm, constant(9), I32, NO_ORIGIN);
        b.terminate(none_arm, RTerm::Return(Some(fallback)));
        let mut func = finished(b);
        let table = option_table();

        assert!(
            !split_payload_merges(&mut func, &table),
            "the whole-value use pins the switch argument"
        );
        func.verify(&table).expect("untouched function verifies");
    }

    /// A merge parameter with any use besides same-variant `Field` projections — here its own
    /// `Discriminant`, and an arm edge passing it on — is refused whole.
    #[test]
    fn a_discriminated_merge_is_not_split() {
        let mut b = builder();
        merge_shape(&mut b);
        let mut func = finished(b);
        let table = option_table();

        assert!(
            !split_payload_merges(&mut func, &table),
            "a discriminant use keeps the object whole"
        );
        func.verify(&table).expect("untouched function verifies");
    }

    #[test]
    fn orphaned_value_defs_compact_even_when_no_deletions_is_set() {
        let mut b = builder();
        let arg = b.param(I32, NO_ORIGIN);
        let orphan = b.op(FuncBuilder::ENTRY, constant(7), I32, NO_ORIGIN);
        let _ = orphan;
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(arg)));
        let mut func = finished(b);
        // Simulate an upstream pass (such as `slice_iter`) removing an op/param while leaving its
        // `ValueDef` in `func.values` for witness checking.
        func.blocks[0].ops.clear();
        let table = option_table();

        let opts = Options {
            no_deletions: true,
            ..Options::default()
        };
        let mut witnesses = Vec::new();
        let changed = run_with(&mut func, &table, &opts, &mut witnesses);
        assert!(!changed);
        func.verify(&table)
            .expect("compacts orphaned ValueDefs even when no_deletions = true");
    }

    #[test]
    fn single_use_borrow_view_trivial_param_eliminates_so_pea_scalarizes() {
        use crate::jir::repr::{OpKind, Root};
        use crate::jir::types::{FieldDescriptor, FieldKey, NominalForm, VariantDescriptor};

        const PAIR: TypeId = TypeId(3);
        const MUT_REF_PAIR: TypeId = TypeId(4);
        let mut tbl = table();
        let fields = vec![
            FieldDescriptor {
                key: FieldKey::Index(0),
                ty: I32,
                offset: 0,
            },
            FieldDescriptor {
                key: FieldKey::Index(1),
                ty: I32,
                offset: 4,
            },
        ];
        tbl.insert(TypeDescriptor {
            id: PAIR,
            name: None,
            kind: TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 99,
                },
                args: Vec::new(),
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: StrId(0),
                    fields: fields.clone(),
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: true,
                needs_clone: true,
                crosses_transparent_union: false,
                transparent: None,
                peeled: PAIR,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields,
            },
        });
        tbl.insert(TypeDescriptor {
            id: MUT_REF_PAIR,
            name: None,
            kind: TypeKind::Reference {
                pointee: PAIR,
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
                peeled: MUT_REF_PAIR,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });

        let mut b = builder();
        let callee_entry = b.block();
        let exit = b.block();
        let entry = FuncBuilder::ENTRY;

        let f0 = b.op(entry, constant(42), I32, NO_ORIGIN);
        let f1 = b.op(entry, constant(7), I32, NO_ORIGIN);
        let iter = b.op(
            entry,
            OpKind::Aggregate {
                ty: PAIR,
                variant: 0,
                active_field: None,
                fields: vec![f0, f1],
            },
            PAIR,
            NO_ORIGIN,
        );
        let view = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(iter),
                path: Vec::new(),
                mutable: true,
            },
            MUT_REF_PAIR,
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Goto(callee_entry, vec![view]));

        let self_param = b.block_param(callee_entry, MUT_REF_PAIR, NO_ORIGIN);
        let deref_val = b.op(callee_entry, OpKind::Deref(self_param), PAIR, NO_ORIGIN);
        let read0 = b.op(
            callee_entry,
            OpKind::Field {
                base: deref_val,
                variant: None,
                field: 0,
            },
            I32,
            NO_ORIGIN,
        );
        b.terminate(callee_entry, RTerm::Goto(exit, vec![]));
        let _ = (read0, exit);
        b.terminate(exit, RTerm::Return(Some(read0)));

        let mut func = finished(b);
        func.verify(&tbl).expect("pre-dce Func::verify");
        assert!(super::run(&mut func, &tbl));
        assert!(
            func.blocks[callee_entry.0 as usize].params.is_empty(),
            "single-use BorrowView trivial param should be eliminated"
        );
        func.verify(&tbl).expect("post-dce Func::verify");

        let pea = crate::opt::partial_escape::run(&mut func, &tbl);
        assert_eq!(
            pea.scalarized, 1,
            "iter should scalarize once view edge is removed"
        );
        assert_eq!(pea.escaped, 0);
        func.verify(&tbl).expect("post-pea Func::verify");
    }

    #[test]
    fn unused_cell_alloc_is_eliminated() {
        use crate::jir::repr::{OpKind, Root};

        const REF_I32: TypeId = TypeId(3);
        let mut tbl = table();
        tbl.insert(TypeDescriptor {
            id: REF_I32,
            name: None,
            kind: TypeKind::Reference {
                pointee: I32,
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
                peeled: REF_I32,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });

        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let zero = b.op(entry, constant(0), I32, NO_ORIGIN);
        let cell = b.cell(I32, false, NO_ORIGIN);
        b.effect(entry, OpKind::CellAlloc(cell), NO_ORIGIN);
        let dead_view = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Cell(cell),
                path: Vec::new(),
                mutable: false,
            },
            REF_I32,
            NO_ORIGIN,
        );
        let _ = dead_view;
        b.terminate(entry, RTerm::Return(Some(zero)));

        let mut func = finished(b);
        func.verify(&tbl).expect("pre-dce Func::verify");
        assert!(super::run(&mut func, &tbl));
        assert!(
            !func.blocks[entry.0 as usize]
                .ops
                .iter()
                .any(|op| matches!(op.kind, OpKind::CellAlloc(_))),
            "dead CellAlloc should be removed once dead BorrowView drops"
        );
        func.verify(&tbl).expect("post-dce Func::verify");
    }

    fn signed_and_checked_table() -> TypeTable {
        use crate::jir::types::{FieldDescriptor, FieldKey};

        const BOOL: TypeId = TypeId(2);
        const U32: TypeId = TypeId(3);
        const CHECKED_I32: TypeId = TypeId(4);
        const I64: TypeId = TypeId(5);
        const CHECKED_I64: TypeId = TypeId(6);
        const STRUCT: TypeId = TypeId(7);
        const CHECKED_U32: TypeId = TypeId(8);
        const FOUR_STRUCT: TypeId = TypeId(9);

        let mut tbl = table();
        let make = |id, kind, int| TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int,
                is_indirect: false,
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
        };
        tbl.insert(make(BOOL, TypeKind::Bool, None));
        tbl.insert(make(
            U32,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            Some(IntFacts {
                signed: false,
                bits: 32,
                repr: IntRepr::Number,
            }),
        ));
        tbl.insert(make(
            I64,
            TypeKind::Integer {
                signed: true,
                bits: 64,
            },
            Some(IntFacts {
                signed: true,
                bits: 64,
                repr: IntRepr::BigInt,
            }),
        ));
        for (pair_id, elem_ty, elem_bytes) in [
            (CHECKED_I32, I32, 4u64),
            (CHECKED_U32, U32, 4u64),
            (CHECKED_I64, I64, 8u64),
        ] {
            let mut d = make(pair_id, TypeKind::Tuple(vec![elem_ty, BOOL]), None);
            d.js.is_indirect = true;
            d.js.needs_clone = true;
            d.js.fields = vec![
                FieldDescriptor {
                    key: FieldKey::Index(0),
                    ty: elem_ty,
                    offset: 0,
                },
                FieldDescriptor {
                    key: FieldKey::Index(1),
                    ty: BOOL,
                    offset: elem_bytes,
                },
            ];
            tbl.insert(d);
        }
        let mut st = make(STRUCT, TypeKind::Tuple(vec![I32]), None);
        st.js.is_indirect = true;
        st.js.needs_clone = true;
        st.js.fields = vec![FieldDescriptor {
            key: FieldKey::Index(0),
            ty: I32,
            offset: 0,
        }];
        tbl.insert(st);
        let mut four = make(
            FOUR_STRUCT,
            TypeKind::Tuple(vec![STRUCT, STRUCT, STRUCT, STRUCT]),
            None,
        );
        four.js.is_indirect = true;
        four.js.needs_clone = true;
        four.js.fields = (0..4)
            .map(|i| FieldDescriptor {
                key: FieldKey::Index(i),
                ty: STRUCT,
                offset: u64::from(i) * 4,
            })
            .collect();
        tbl.insert(four);
        tbl
    }

    #[test]
    fn signed_lt_step_overflow_folds_i32_and_i64_and_rejects_unsafe_shapes() {
        use crate::jir::repr::RAssertMessage;

        const BOOL: TypeId = TypeId(2);
        const U32: TypeId = TypeId(3);
        const CHECKED_I32: TypeId = TypeId(4);
        const I64: TypeId = TypeId(5);
        const CHECKED_I64: TypeId = TypeId(6);
        const CHECKED_U32: TypeId = TypeId(8);
        let tbl = signed_and_checked_table();

        let run_case = |val_ty: TypeId,
                        pair_ty: TypeId,
                        cmp_op: BinaryOp,
                        cmp_left_is_x: bool,
                        step: u128,
                        join_from_both: bool| {
            let mut b = FuncBuilder::new(
                ItemId {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 201,
                    },
                    instantiation: 0,
                },
                Signature {
                    abi: AbiSignature {
                        parameters: 2,
                        ..AbiSignature::default()
                    },
                    arguments: vec![val_ty, val_ty],
                    result: val_ty,
                },
            );
            let entry = FuncBuilder::ENTRY;
            let body = b.block();
            let exit = b.block();
            let x = b.param(val_ty, NO_ORIGIN);
            let y = b.param(val_ty, NO_ORIGIN);
            let (l, r) = if cmp_left_is_x { (x, y) } else { (y, x) };
            let cmp = b.op(
                entry,
                OpKind::Binary {
                    op: cmp_op,
                    l,
                    r,
                    ty: val_ty,
                },
                BOOL,
                NO_ORIGIN,
            );
            let fallback = if join_from_both { body } else { exit };
            b.terminate(
                entry,
                RTerm::Switch {
                    scrut: cmp,
                    scrut_ty: BOOL,
                    cases: vec![(0, fallback, Vec::new())],
                    default: (body, Vec::new()),
                },
            );
            let bytes = if val_ty == I64 { 8 } else { 4 };
            let step_val = b.op(
                body,
                OpKind::Const(ConstTree::Scalar { bits: step, bytes }),
                val_ty,
                NO_ORIGIN,
            );
            let pair = b.op(
                body,
                OpKind::Binary {
                    op: BinaryOp::AddWithOverflow,
                    l: x,
                    r: step_val,
                    ty: val_ty,
                },
                pair_ty,
                NO_ORIGIN,
            );
            let sum = b.op(
                body,
                OpKind::Field {
                    base: pair,
                    variant: None,
                    field: 0,
                },
                val_ty,
                NO_ORIGIN,
            );
            let ovf = b.op(
                body,
                OpKind::Field {
                    base: pair,
                    variant: None,
                    field: 1,
                },
                BOOL,
                NO_ORIGIN,
            );
            b.effect(
                body,
                OpKind::Assert {
                    cond: ovf,
                    expected: false,
                    msg: RAssertMessage::Overflow {
                        op: BinaryOp::Add,
                        left: x,
                        right: step_val,
                    },
                    track_caller: None,
                },
                NO_ORIGIN,
            );
            b.terminate(body, RTerm::Return(Some(sum)));
            b.terminate(exit, RTerm::Return(Some(x)));

            let mut func = b.finish();
            func.verify(&tbl).expect("pre-dce Func::verify");
            let mut probe = func.clone();
            let folded_signed = fold_signed_lt_step_overflow(&mut probe, &tbl);
            let changed = super::run(&mut func, &tbl);
            func.verify(&tbl).expect("post-dce Func::verify");
            let has_ovf_add = func.blocks.iter().any(|blk| {
                blk.ops.iter().any(|op| {
                    matches!(
                        op.kind,
                        OpKind::Binary {
                            op: BinaryOp::AddWithOverflow,
                            ..
                        }
                    )
                })
            });
            let has_assert = func.blocks.iter().any(|blk| {
                blk.ops
                    .iter()
                    .any(|op| matches!(op.kind, OpKind::Assert { .. }))
            });
            (folded_signed, changed, has_ovf_add, has_assert)
        };

        // Positive cases: signed i32 Lt (`x < y`), signed i32 Gt (`y > x`), signed i64 Lt (`x < y`).
        for (ty, pair_ty, cmp_op, left_x) in [
            (I32, CHECKED_I32, BinaryOp::Lt, true),
            (I32, CHECKED_I32, BinaryOp::Gt, false),
            (I64, CHECKED_I64, BinaryOp::Lt, true),
        ] {
            let (folded_signed, changed, has_ovf_add, has_assert) =
                run_case(ty, pair_ty, cmp_op, left_x, 1, false);
            assert!(folded_signed);
            assert!(changed);
            assert!(
                !has_ovf_add,
                "signed x < y step + 1 should fold AddWithOverflow"
            );
            assert!(
                !has_assert,
                "signed x < y step + 1 overflow assert should fold away"
            );
        }

        // Negative cases: unsigned Lt, signed Le (`x <= y`), step + 2, non-dominated join block.
        for (label, ty, pair_ty, cmp_op, step, join) in [
            ("unsigned Lt", U32, CHECKED_U32, BinaryOp::Lt, 1u128, false),
            ("signed Le", I32, CHECKED_I32, BinaryOp::Le, 1u128, false),
            ("step + 2", I32, CHECKED_I32, BinaryOp::Lt, 2u128, false),
            ("non-dominated", I32, CHECKED_I32, BinaryOp::Lt, 1u128, true),
        ] {
            let (folded_signed, _, _, has_assert) = run_case(ty, pair_ty, cmp_op, true, step, join);
            assert!(
                !folded_signed,
                "{label} must not trigger fold_signed_lt_step_overflow"
            );
            assert!(has_assert, "{label} must preserve overflow Assert");
        }
    }

    #[test]
    fn elide_redundant_copies_turns_fresh_chain_to_move_and_preserves_aliases() {
        const STRUCT: TypeId = TypeId(7);
        const FOUR_STRUCT: TypeId = TypeId(9);
        let tbl = signed_and_checked_table();

        // Positive: `Aggregate` -> `Copy` -> `Copy` -> `Return` becomes `Move`.
        {
            let mut b = FuncBuilder::new(
                ItemId {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 202,
                    },
                    instantiation: 0,
                },
                Signature {
                    abi: AbiSignature {
                        parameters: 1,
                        ..AbiSignature::default()
                    },
                    arguments: vec![I32],
                    result: STRUCT,
                },
            );
            let entry = FuncBuilder::ENTRY;
            let x = b.param(I32, NO_ORIGIN);
            let agg = b.op(
                entry,
                OpKind::Aggregate {
                    ty: STRUCT,
                    variant: 0,
                    active_field: None,
                    fields: vec![x],
                },
                STRUCT,
                NO_ORIGIN,
            );
            let c1 = b.op(entry, OpKind::Copy(agg), STRUCT, NO_ORIGIN);
            let c2 = b.op(entry, OpKind::Copy(c1), STRUCT, NO_ORIGIN);
            b.terminate(entry, RTerm::Return(Some(c2)));
            let mut func = b.finish();
            func.verify(&tbl).expect("pre-dce Func::verify");
            assert!(super::run(&mut func, &tbl));
            let copy_count = func.blocks[entry.0 as usize]
                .ops
                .iter()
                .filter(|op| matches!(op.kind, OpKind::Copy(_)))
                .count();
            assert_eq!(
                copy_count, 0,
                "chained copies of fresh aggregate must become Move"
            );
        }

        // Negative: param copy, 2-use copy, cross-block copy must stay Copy.
        {
            let mut b = FuncBuilder::new(
                ItemId {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 203,
                    },
                    instantiation: 0,
                },
                Signature {
                    abi: AbiSignature {
                        parameters: 2,
                        ..AbiSignature::default()
                    },
                    arguments: vec![I32, STRUCT],
                    result: FOUR_STRUCT,
                },
            );
            let entry = FuncBuilder::ENTRY;
            let next = b.block();
            let x = b.param(I32, NO_ORIGIN);
            let p_struct = b.param(STRUCT, NO_ORIGIN);
            let copy_param = b.op(entry, OpKind::Copy(p_struct), STRUCT, NO_ORIGIN);
            let two_use_agg = b.op(
                entry,
                OpKind::Aggregate {
                    ty: STRUCT,
                    variant: 0,
                    active_field: None,
                    fields: vec![x],
                },
                STRUCT,
                NO_ORIGIN,
            );
            let u1 = b.op(entry, OpKind::Copy(two_use_agg), STRUCT, NO_ORIGIN);
            let u2 = b.op(entry, OpKind::Copy(two_use_agg), STRUCT, NO_ORIGIN);
            let cross_agg = b.op(
                entry,
                OpKind::Aggregate {
                    ty: STRUCT,
                    variant: 0,
                    active_field: None,
                    fields: vec![x],
                },
                STRUCT,
                NO_ORIGIN,
            );
            b.terminate(entry, RTerm::Goto(next, Vec::new()));
            let cross_copy = b.op(next, OpKind::Copy(cross_agg), STRUCT, NO_ORIGIN);
            let out = b.op(
                next,
                OpKind::Aggregate {
                    ty: FOUR_STRUCT,
                    variant: 0,
                    active_field: None,
                    fields: vec![copy_param, u1, u2, cross_copy],
                },
                FOUR_STRUCT,
                NO_ORIGIN,
            );
            b.terminate(next, RTerm::Return(Some(out)));

            let mut func = b.finish();
            func.verify(&tbl).expect("pre-dce Func::verify");
            assert!(!super::run(&mut func, &tbl));
            let remaining_copies: usize = func
                .blocks
                .iter()
                .flat_map(|blk| &blk.ops)
                .filter(|op| matches!(op.kind, OpKind::Copy(_)))
                .count();
            assert_eq!(
                remaining_copies, 4,
                "param, 2-use, and cross-block Copy must remain Copy"
            );
        }
    }
}

/// The corpus property test: every fixture function the pass changes still verifies and still
/// means the same thing.
///
/// The verify half is the one that earns its keep — deleting a definition and leaving its
/// `ValueDef` behind is what `Func::verify` rejects with "value vN is never defined", and it
/// declines the item rather than failing the build, so nothing else would notice. The corpus is
/// whatever `build/` holds; on a checkout that has not run the suites the test finds nothing and
/// passes, and says so — unless `RCGJS_SSA_FIXTURES_REQUIRED=1` says the corpus must be there.
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

    /// Argument vectors and interpreter steps per function: the interpreter runs twice per vector.
    const SETS: usize = 4;
    const STEPS: u64 = 5_000;

    #[derive(Default)]
    struct Counts {
        functions: usize,
        changed: usize,
        agreed: usize,
        skipped: usize,
        verify_failures: Vec<String>,
        mismatches: Vec<String>,
    }

    #[test]
    fn dce_preserves_fixture_semantics() {
        // A deep body recurses through its blocks on the way up.
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
            "dce property: {} functions, {} changed, {} argument sets agreed, {} skipped, \
             {} verify failures, {} mismatches",
            counts.functions,
            counts.changed,
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
        ssa::check::require_fixtures(counts.functions, "dce property");
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
        // The result type of every function in the object, so a stubbed call can return a value
        // of the type the caller reads back rather than one opaque token for all of them.
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
            if !super::run(&mut after, table) {
                continue;
            }
            counts.changed += 1;
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

    /// `None` when the two runs agree, `Some(None)` when the comparison says nothing.
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
