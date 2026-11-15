//! Optimization witnesses: every class-named rewrite of the O/C pass groups records one, and
//! [`Witness::check`] re-derives its claim independently of the pass that made it.
//!
//! A witness is a structural re-derivation, not a semantic proof — semantics are carried by the
//! interpreter differentials, the fuzzer, and the browser differential. What a check buys is
//! independence: `ConstFold` re-evaluates the folded comparison with the `jir/interp` evaluator
//! rather than the pass's own arithmetic, `BranchFold` re-runs the case selection over the
//! recorded scrutinee, `BoundsRemoved` re-runs dominance and the implying fact against the final
//! [`Func`], and the escape-analysis classes re-run the carrier verifier obligations. A failed
//! check declines the item's rewrite with `proof.witness` — the item falls back to its baseline
//! text and the link goes on.
//!
//! Two lifetimes, because the pass groups renumber: a witness recorded mid-`dce` (`ConstFold`,
//! `BranchFold`) is self-contained — its payload carries the operand values and the case table,
//! so later rounds may renumber blocks and compact values without dangling it. A witness recorded
//! by the last pass of the group (`BoundsRemoved`, from `opt/range.rs`, which deletes asserts but
//! never renumbers) names blocks and values of the final `Func` and is checked structurally
//! against it.

use crate::jir::interns::{Interns, StrId};
use crate::jir::interp::{self, Hooks, Trap, Value};
use crate::jir::repr::{Dominators, Func, FuncBuilder, OpKind, RBlockId, RTerm, ValueId};
use crate::jir::semantic::{
    AbiSignature, BinaryOp, Callee, ConstTree, DefinitionId, DirectItemRef, ItemId, Signature,
    TypeId,
};
use crate::jir::types::IteratorIdentity;
use crate::jir::{OriginId, TypeTable};

/// Exact evidence for one standard-library slice-adapter loop replaced by a scalar cursor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SliceIteratorLoop {
    pub(crate) adapter: IteratorIdentity,
    pub(crate) header: RBlockId,
    pub(crate) preheader: RBlockId,
    pub(crate) backedges: Vec<RBlockId>,
    pub(crate) direct: RBlockId,
    pub(crate) body: RBlockId,
    pub(crate) exit: RBlockId,
    pub(crate) source: ValueId,
    pub(crate) length: ValueId,
    pub(crate) bound_length: ValueId,
    pub(crate) limit: ValueId,
    pub(crate) initial: ValueId,
    pub(crate) cursor: ValueId,
    pub(crate) length_guard: ValueId,
    pub(crate) limit_guard: Option<ValueId>,
    pub(crate) condition: ValueId,
    pub(crate) body_cursor: ValueId,
    pub(crate) enumerate_index: ValueId,
    pub(crate) item_ref: ValueId,
    pub(crate) byte: Option<ValueId>,
    pub(crate) one: ValueId,
    pub(crate) next: ValueId,
    pub(crate) body_args: Vec<ValueId>,
    pub(crate) item_ref_ty: TypeId,
    pub(crate) byte_ty: TypeId,
    pub(crate) cursor_ty: TypeId,
    pub(crate) usize_ty: TypeId,
    pub(crate) bool_ty: TypeId,
    pub(crate) cursor_bytes: u8,
    pub(crate) origin: OriginId,
}

/// One recorded rewrite, by the class the phase-5 briefs name (D9).
///
/// `Inline` and `Devirt` are recorded by the wave-2 passes through this same type; their checks
/// are the budget arithmetic and the devirtualized callee's presence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Witness {
    /// A comparison of two integer constants was folded to a boolean constant.
    ConstFold {
        /// The comparison operator.
        op: BinaryOp,
        /// The operand type (decides signedness and width).
        ty: TypeId,
        /// Left operand: raw bits and byte width, as the `Const` op carried them.
        l: (u128, u8),
        /// Right operand.
        r: (u128, u8),
        /// The folded result.
        result: bool,
    },
    /// A `Switch` on a known scrutinee took one edge: a constant fold, a statically fixed
    /// discriminant, or one threaded edge of a variant-per-path merge.
    BranchFold {
        /// The scrutinee's raw bits.
        scrut: u128,
        /// The width the comparison masks to.
        bytes: u8,
        /// The case table as `(value, target block)` at record time.
        cases: Vec<(u128, u32)>,
        /// The default target at record time.
        default: u32,
        /// The target the fold or thread chose.
        taken: u32,
    },
    /// The escape analysis scalarized objects; the obligation is the carrier verifier.
    Scalarize {
        /// Objects replaced by their fields.
        scalarized: u32,
    },
    /// A representation adapter was priced and placed; same obligation as `Scalarize`.
    #[allow(dead_code)] // Recorded by the wave-2 passes; the check is live now.
    Adapter {
        /// Crossings adapted.
        crossings: u32,
    },
    /// A bounds-check `Assert` was deleted because a dominating fact implies its condition.
    BoundsRemoved {
        /// The block that held the assert (still present: the pass deletes ops only).
        block: RBlockId,
        /// The assert's index operand.
        index: ValueId,
        /// The assert's length operand.
        length: ValueId,
        /// The evidence.
        fact: BoundsFact,
    },
    /// An unsigned aligned-prefix comparison was folded from
    /// `prefix = length - (length % nonzero)` and `0 <= prefix <= length`.
    AlignedPrefix {
        /// The comparison's left operand.
        left: ValueId,
        /// The comparison's right operand.
        right: ValueId,
        /// The comparison relation.
        op: BinaryOp,
        /// The folded Boolean result.
        result: bool,
    },
    /// Drops were elided with an object's scalarization; same obligation as `Scalarize`.
    #[allow(dead_code)] // Recorded by the wave-2 passes; the check is live now.
    DropElided {
        /// Drops elided.
        count: u32,
    },
    /// A virtual or fn-pointer call was rewritten to a direct call of `target`.
    #[allow(dead_code)] // Recorded by the wave-2 passes; the check is live now.
    Devirt {
        /// The unique target's link name.
        target: StrId,
    },
    /// A callee body was inlined; the check is the budget arithmetic.
    #[allow(dead_code)] // Recorded by the wave-2 passes; the check is live now.
    Inline {
        /// The callee body's minified byte count.
        callee_bytes: u32,
        /// The per-item byte limit the inliner enforced.
        limit: u32,
        /// The item's minified bytes before.
        bytes_before: u32,
        /// The item's minified bytes after.
        bytes_after: u32,
    },
    /// An unsigned checked-add tuple was split into its wrapped result and overflow bit.
    UnsignedAddOverflowSplit {
        /// Width of the modular integer arithmetic.
        bits: u32,
    },
    /// A copied call result exposed fields proven to forward callee parameters.
    ReturnProjection {
        /// Forwarded fields exposed in the caller.
        fields: u32,
    },
    /// Immutable indexed borrows bypassed a locally built slice-view record.
    SliceViewForward {
        /// Indexed borrows forwarded.
        views: u32,
    },
    /// Fresh standard-library slice adapters were replaced by checked scalar cursors.
    SliceIteratorCursor {
        /// Exact generated blocks, values, edges, and adapter semantics for every loop.
        loops: Vec<SliceIteratorLoop>,
    },
}

/// The evidence a [`Witness::BoundsRemoved`] carries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoundsFact {
    /// Intervals alone prove it: the index's upper bound is below the length's lower bound.
    Numeric {
        /// The index interval's inclusive upper bound.
        index_hi: u128,
        /// The length interval's inclusive lower bound.
        length_lo: u128,
    },
    /// A dominating comparison proves it: `base + offset_fact` relates to the same length the
    /// assert checks against, and the assert's `base + offset_index` sits inside it.
    Dominating {
        /// The block the fact is established in (the branch target, or the earlier assert's
        /// block). Must dominate `Witness::BoundsRemoved::block` in the final `Func`.
        block: RBlockId,
        /// The comparison value the fact came from: the scrutinee of a `Switch`, or the
        /// condition of an `Assert`, still present in the final `Func`.
        cond: ValueId,
        /// The assert's peeled constant offset from the shared base.
        offset_index: u128,
        /// The fact's peeled constant offset from the shared base.
        offset_fact: u128,
        /// Whether the fact is strict (`<`): strict admits `offset_index <= offset_fact`,
        /// non-strict (`<=`) requires `offset_index < offset_fact`.
        strict: bool,
    },
}

impl Witness {
    /// Re-derives this witness's claim against `func`, independently of the pass that made it.
    pub fn check(&self, func: &Func, table: &TypeTable) -> Result<(), String> {
        match self {
            Self::ConstFold {
                op,
                ty,
                l,
                r,
                result,
            } => check_const_fold(*op, *ty, *l, *r, *result, table),
            Self::BranchFold {
                scrut,
                bytes,
                cases,
                default,
                taken,
            } => check_branch_fold(*scrut, *bytes, cases, *default, *taken),
            Self::Scalarize { .. } | Self::Adapter { .. } | Self::DropElided { .. } => {
                check_carriers(func, table)
            }
            Self::BoundsRemoved {
                block,
                index,
                length,
                fact,
            } => check_bounds_removed(func, *block, *index, *length, fact),
            Self::AlignedPrefix {
                left,
                right,
                op,
                result,
            } => check_aligned_prefix(func, table, *left, *right, *op, *result),
            Self::Devirt { target } => check_devirt(func, *target),
            Self::Inline {
                callee_bytes,
                limit,
                bytes_before,
                bytes_after,
            } => check_inline(*callee_bytes, *limit, *bytes_before, *bytes_after),
            Self::UnsignedAddOverflowSplit { bits } => check_unsigned_add_split(*bits),
            Self::ReturnProjection { fields } => {
                if *fields == 0 {
                    Err("a return-projection witness exposed no fields".to_owned())
                } else {
                    check_carriers(func, table)
                }
            }
            Self::SliceViewForward { views } => {
                if *views == 0 {
                    Err("a slice-forward witness rewrote no views".to_owned())
                } else {
                    check_carriers(func, table)
                }
            }
            Self::SliceIteratorCursor { loops } => {
                super::slice_iter::check_lowered(func, table, loops)
            }
        }
    }
}

/// Checks every witness; the first failure names the witness and the reason.
pub fn check_all(witnesses: &[Witness], func: &Func, table: &TypeTable) -> Result<(), String> {
    for witness in witnesses {
        witness
            .check(func, table)
            .map_err(|why| format!("{witness:?}: {why}"))?;
    }
    Ok(())
}

/// Whether the independent checks run: always in debug builds, and wherever
/// `RCGJS_WITNESS_CHECK=1` asks for them (CI on release artifacts).
pub fn enabled() -> bool {
    cfg!(debug_assertions) || std::env::var_os("RCGJS_WITNESS_CHECK").is_some()
}

/// `ConstFold`: rebuilds the comparison as a three-op function and evaluates it with the
/// `jir/interp` interpreter, whose integer semantics are written independently of the pass.
fn check_const_fold(
    op: BinaryOp,
    ty: TypeId,
    l: (u128, u8),
    r: (u128, u8),
    result: bool,
    table: &TypeTable,
) -> Result<(), String> {
    let mut b = FuncBuilder::new(
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
            },
            arguments: Vec::new(),
            result: ty,
        },
    );
    let origin = OriginId(0);
    let entry = FuncBuilder::ENTRY;
    let scalar = |(bits, bytes)| OpKind::Const(ConstTree::Scalar { bits, bytes });
    let lv = b.op(entry, scalar(l), ty, origin);
    let rv = b.op(entry, scalar(r), ty, origin);
    let cmp = b.op(
        entry,
        OpKind::Binary {
            op,
            l: lv,
            r: rv,
            ty,
        },
        ty,
        origin,
    );
    b.terminate(entry, RTerm::Return(Some(cmp)));
    let func = b.finish();
    let hook = |_: &DirectItemRef, _: Vec<Value>| -> Result<Value, Trap> {
        Err(Trap::Hook("a constant fold calls nothing".to_owned()))
    };
    let interns = Interns::new();
    match interp::run_func(&func, table, &interns, Vec::new(), &Hooks::new(&hook)).ret {
        Ok(value) => match value {
            Value::Bool(value) if value == result => Ok(()),
            other => Err(format!("the evaluator computed {other:?}, not {result}")),
        },
        Err(trap) => Err(format!("the evaluator could not re-run the fold: {trap}")),
    }
}

/// `BranchFold`: re-runs the case selection over the recorded scrutinee and case table.
fn check_branch_fold(
    scrut: u128,
    bytes: u8,
    cases: &[(u128, u32)],
    default: u32,
    taken: u32,
) -> Result<(), String> {
    let value = mask(scrut, bytes);
    let selected = cases
        .iter()
        .find(|(case, _)| mask(*case, bytes) == value)
        .map(|(_, target)| *target)
        .unwrap_or(default);
    if selected == taken {
        Ok(())
    } else {
        Err(format!("case selection lands on {selected}, not {taken}"))
    }
}

/// `UnsignedAddOverflowSplit`: independently checks the modular identity on its boundary cases.
fn check_unsigned_add_split(bits: u32) -> Result<(), String> {
    if !(1..=128).contains(&bits) {
        return Err(format!("invalid unsigned integer width {bits}"));
    }
    let max = if bits == 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    };
    let values = [0, 1, max / 2, max.saturating_sub(1), max];
    for lhs in values {
        for rhs in values {
            let (sum, machine_overflow) = lhs.overflowing_add(rhs);
            let wrapped = sum & max;
            let overflow = machine_overflow || sum > max;
            if (wrapped < lhs) != overflow {
                return Err(format!(
                    "wrapped {lhs} + {rhs} at {bits} bits gives an inconsistent overflow bit"
                ));
            }
        }
    }
    Ok(())
}

/// `bits` truncated to `bytes` bytes; 16 or more is the identity.
fn mask(bits: u128, bytes: u8) -> u128 {
    match bytes {
        0 => 0,
        1..=15 => bits & ((1u128 << (bytes as u32 * 8)) - 1),
        _ => bits,
    }
}

/// `Scalarize`/`Adapter`/`DropElided`: the obligation is that the function still verifies under
/// the carriers its values observably use.
fn check_carriers(func: &Func, table: &TypeTable) -> Result<(), String> {
    func.verify_carriers(table, super::observed_carriers(func))
        .map_err(|err| format!("the carrier verifier rejects the function: {err}"))
}

/// `BoundsRemoved`: re-runs dominance and the implying fact on the final `Func`.
fn check_bounds_removed(
    func: &Func,
    block: RBlockId,
    index: ValueId,
    length: ValueId,
    fact: &BoundsFact,
) -> Result<(), String> {
    if func.blocks.get(block.0 as usize).is_none() {
        return Err(format!("block {} is gone", block.0));
    }
    if (index.0 as usize) >= func.values.len() || (length.0 as usize) >= func.values.len() {
        return Err("the assert's operands are gone".to_owned());
    }
    match fact {
        BoundsFact::Numeric {
            index_hi,
            length_lo,
        } => {
            if index_hi < length_lo {
                Ok(())
            } else {
                Err(format!("{index_hi} is not below {length_lo}"))
            }
        }
        BoundsFact::Dominating {
            block: fact_block,
            cond,
            offset_index,
            offset_fact,
            strict,
        } => {
            let implied = match strict {
                true => offset_index <= offset_fact,
                false => offset_index < offset_fact,
            };
            if !implied {
                return Err(format!(
                    "offset {offset_index} is not inside the fact's {offset_fact}"
                ));
            }
            let doms = Dominators::of(func);
            if !dominates(&doms, *fact_block, block) {
                return Err(format!(
                    "block {} does not dominate block {}",
                    fact_block.0, block.0
                ));
            }
            let present = func.blocks.iter().any(|b| {
                matches!(&b.term, RTerm::Switch { scrut, .. } if scrut == cond)
                    || b.ops
                        .iter()
                        .any(|op| matches!(&op.kind, OpKind::Assert { cond: c, .. } if c == cond))
            });
            if present {
                Ok(())
            } else {
                Err(format!("no switch or assert on v{} remains", cond.0))
            }
        }
    }
}

/// Independently checks the one modular-arithmetic identity the range pass folds.
fn check_aligned_prefix(
    func: &Func,
    table: &TypeTable,
    left: ValueId,
    right: ValueId,
    op: BinaryOp,
    result: bool,
) -> Result<(), String> {
    fn definition(func: &Func, value: ValueId) -> Option<&OpKind> {
        func.blocks
            .iter()
            .flat_map(|block| &block.ops)
            .find(|op| op.result == Some(value))
            .map(|op| &op.kind)
    }
    fn copy_root(func: &Func, mut value: ValueId) -> ValueId {
        for _ in 0..8 {
            match definition(func, value) {
                Some(OpKind::Copy(source) | OpKind::Move(source)) => value = *source,
                _ => break,
            }
        }
        value
    }
    fn same_term(func: &Func, left: ValueId, right: ValueId) -> bool {
        let (left, right) = (copy_root(func, left), copy_root(func, right));
        left == right
            || matches!(
                (definition(func, left), definition(func, right)),
                (
                    Some(OpKind::Unary {
                        op: crate::jir::semantic::UnaryOp::PtrMetadata,
                        v: left,
                        ..
                    }),
                    Some(OpKind::Unary {
                        op: crate::jir::semantic::UnaryOp::PtrMetadata,
                        v: right,
                        ..
                    })
                ) if copy_root(func, *left) == copy_root(func, *right)
            )
    }
    fn zero(func: &Func, value: ValueId) -> bool {
        matches!(
            definition(func, copy_root(func, value)),
            Some(OpKind::Const(ConstTree::Scalar { bits: 0, .. }))
        )
    }
    fn prefix_base(func: &Func, table: &TypeTable, value: ValueId) -> Option<ValueId> {
        let OpKind::Binary {
            op: BinaryOp::Sub | BinaryOp::SubUnchecked,
            l: base,
            r: remainder,
            ty,
        } = definition(func, copy_root(func, value))?
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
        } = definition(func, copy_root(func, *remainder))?
        else {
            return None;
        };
        if remainder_ty != ty || !same_term(func, *base, *dividend) {
            return None;
        }
        match definition(func, copy_root(func, *divisor)) {
            Some(OpKind::Const(ConstTree::Scalar { bits, bytes })) if mask(*bits, *bytes) != 0 => {
                Some(*base)
            }
            _ => None,
        }
    }

    let expected = if let Some(base) = prefix_base(func, table, left) {
        if same_term(func, right, base) {
            match op {
                BinaryOp::Le => Some(true),
                BinaryOp::Gt => Some(false),
                _ => None,
            }
        } else if zero(func, right) {
            match op {
                BinaryOp::Lt => Some(false),
                BinaryOp::Ge => Some(true),
                _ => None,
            }
        } else {
            None
        }
    } else if let Some(base) = prefix_base(func, table, right) {
        if same_term(func, left, base) {
            match op {
                BinaryOp::Ge => Some(true),
                BinaryOp::Lt => Some(false),
                _ => None,
            }
        } else if zero(func, left) {
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
    };
    match expected {
        Some(expected) if expected == result => Ok(()),
        Some(expected) => Err(format!(
            "the aligned-prefix relation evaluates to {expected}"
        )),
        None => Err("the operands are not a proven aligned-prefix relation".to_owned()),
    }
}

/// Whether `a` dominates `b` (reflexively) under `doms`.
fn dominates(doms: &Dominators, a: RBlockId, b: RBlockId) -> bool {
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

/// `Devirt`: the direct call the rewrite produced is present.
fn check_devirt(func: &Func, target: StrId) -> Result<(), String> {
    let called = func.blocks.iter().any(|block| {
        block.ops.iter().any(|op| {
            matches!(
                &op.kind,
                OpKind::Call {
                    callee: Callee::Direct { link_name, .. },
                    ..
                } if *link_name == target
            )
        })
    });
    if called {
        Ok(())
    } else {
        Err(format!("no direct call of str#{} remains", target.0))
    }
}

/// `Inline`: the budget arithmetic holds.
fn check_inline(
    callee_bytes: u32,
    limit: u32,
    bytes_before: u32,
    bytes_after: u32,
) -> Result<(), String> {
    if bytes_after > limit {
        return Err(format!("{bytes_after} bytes exceeds the {limit} limit"));
    }
    if u64::from(bytes_after) > u64::from(bytes_before) + u64::from(callee_bytes) {
        return Err(format!(
            "{bytes_after} bytes exceeds before ({bytes_before}) plus the callee ({callee_bytes})"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::types::{
        IntFacts, IntRepr, JsFacts, LayoutFacts, TypeDescriptor, TypeKind, TypeMeta,
    };

    const I32: TypeId = TypeId(1);

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

    /// A function with nothing in it, for the payload-only checks.
    fn empty() -> Func {
        let mut b = FuncBuilder::new(
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
                },
                arguments: Vec::new(),
                result: I32,
            },
        );
        let zero = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            I32,
            OriginId(0),
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(zero)));
        b.finish()
    }

    /// The signed comparison the evaluator has to get right independently: `-1 < 4` at `i32`,
    /// where the raw bits of `-1` are numerically huge.
    #[test]
    fn const_fold_reevaluates_signed_comparisons() {
        let (func, table) = (empty(), table());
        let fold = |result| Witness::ConstFold {
            op: BinaryOp::Lt,
            ty: I32,
            l: (u32::MAX as u128, 4),
            r: (4, 4),
            result,
        };
        assert!(fold(true).check(&func, &table).is_ok());
        assert!(fold(false).check(&func, &table).is_err());
    }

    /// The case selection re-runs over the recorded table, masked to the recorded width.
    #[test]
    fn branch_fold_reselects_the_case() {
        let (func, table) = (empty(), table());
        let fold = |taken| Witness::BranchFold {
            scrut: 0x1_0000_0001,
            bytes: 4,
            cases: vec![(1, 7), (2, 8)],
            default: 9,
            taken,
        };
        // Masked to four bytes the scrutinee is 1: case (1, 7).
        assert!(fold(7).check(&func, &table).is_ok());
        assert!(fold(9).check(&func, &table).is_err());
    }

    /// The inline budget arithmetic, and the devirt target presence, both fail loudly when the
    /// recorded numbers or callee are wrong.
    #[test]
    fn inline_and_devirt_payloads_check() {
        let (func, table) = (empty(), table());
        let inline = Witness::Inline {
            callee_bytes: 10,
            limit: 100,
            bytes_before: 50,
            bytes_after: 60,
        };
        assert!(inline.check(&func, &table).is_ok());
        let over = Witness::Inline {
            callee_bytes: 10,
            limit: 55,
            bytes_before: 50,
            bytes_after: 60,
        };
        assert!(over.check(&func, &table).is_err());
        let devirt = Witness::Devirt { target: StrId(0) };
        assert!(devirt.check(&func, &table).is_err(), "no such call remains");
    }

    /// The carrier-obligation classes re-run the verifier, which an empty function passes.
    #[test]
    fn carrier_classes_rerun_the_verifier() {
        let (func, table) = (empty(), table());
        for witness in [
            Witness::Scalarize { scalarized: 1 },
            Witness::Adapter { crossings: 1 },
            Witness::DropElided { count: 1 },
        ] {
            witness.check(&func, &table).expect("the verifier accepts");
        }
    }
}
