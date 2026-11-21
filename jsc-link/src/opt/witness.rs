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

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::analysis::refs::{self, RefScan};
use crate::item::{ItemKind, JsName, LinkItem};
use crate::jir::decline::Decline;
use crate::jir::interns::{Interns, StrId};
use crate::jir::interp::{self, Hooks, Trap, Value};
use crate::jir::repr::{Dominators, Func, FuncBuilder, Op, OpKind, RBlockId, RTerm, ValueId};
use crate::jir::semantic::{
    AbiSignature, BinaryOp, BodyKind, CallSemantic, Callee, ConstTree, DefinitionId, DirectItemRef,
    DropGlue, HostPayload, ItemId, Signature, TypeId, VtableEntry,
};
use crate::jir::types::{IteratorIdentity, TypeKind};
use crate::jir::{OriginId, TypeTable};
use crate::late_ir::AllocatorProvider;

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
    /// Non-throwing or noop-unwind `RTerm::Invoke` terminators were pruned to `RTerm::Goto`.
    InvokePruned {
        /// Number of pruned `RTerm::Invoke` sites in the function.
        count: u32,
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
            Self::InvokePruned { count } => {
                if *count == 0 {
                    Err("an invoke-pruned witness pruned no invokes".to_owned())
                } else {
                    check_carriers(func, table)
                }
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
                inline_never: false,
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

/// Reason an `RTerm::Invoke` terminator was proven safe to lower to `RTerm::Goto`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InvokePruneReason {
    NoopUnwindChain,
    ProtectedOpCannotThrow,
}

/// One pre-DCE `RTerm::Invoke` block pruned by `prune_invoke`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InvokePrunedSite {
    pub block: RBlockId,
    pub reason: InvokePruneReason,
}

#[cfg(test)]
std::thread_local! {
    static TEST_WITNESS_FAULT: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct ScopedWitnessFault {
    prev: Option<String>,
}

#[cfg(test)]
impl ScopedWitnessFault {
    pub(crate) fn new(val: &str) -> Self {
        let prev = TEST_WITNESS_FAULT.with(|cell| cell.borrow_mut().replace(val.to_owned()));
        Self { prev }
    }
}

#[cfg(test)]
impl Drop for ScopedWitnessFault {
    fn drop(&mut self) {
        let prev = self.prev.take();
        TEST_WITNESS_FAULT.with(|cell| *cell.borrow_mut() = prev);
    }
}

fn current_witness_fault() -> Option<String> {
    #[cfg(test)]
    if let Some(val) = TEST_WITNESS_FAULT.with(|cell| cell.borrow().clone()) {
        return Some(val);
    }
    std::env::var("RCGJS_WITNESS_FAULT").ok()
}

/// Independent validator for `prune_invoke`: shares no helper code with `prune_invoke.rs` and
/// re-derives every pruned `Invoke` claim directly against pre-prune `pre_funcs`, `table`, and
/// `live`.
pub(crate) fn check_invoke_pruning(
    pre_func: &Func,
    interns: &Interns,
    sites: &[InvokePrunedSite],
    table: &BTreeMap<JsName, LinkItem>,
    pre_funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    live: &BTreeSet<JsName>,
) -> Result<(), String> {
    if let Some(fault) = current_witness_fault() {
        if fault
            .split(|c: char| c == ',' || c.is_whitespace())
            .any(|tok| matches!(tok, "prune-invoke" | "all" | "1"))
        {
            return Err("injected prune-invoke witness fault".to_owned());
        }
    }
    if sites.is_empty() {
        return Err("no invoke pruning sites recorded".to_owned());
    }

    let mut cx = InvokeCheckCx::new(table, pre_funcs, live);
    for site in sites {
        let block = pre_func
            .blocks
            .get(site.block.0 as usize)
            .ok_or_else(|| format!("pruned block bb{} does not exist", site.block.0))?;
        let RTerm::Invoke { unwind, .. } = &block.term else {
            return Err(format!(
                "pruned block bb{} did not terminate in RTerm::Invoke",
                site.block.0
            ));
        };
        match site.reason {
            InvokePruneReason::NoopUnwindChain => {
                verify_noop_unwind_chain(pre_func, interns, unwind.0, pre_funcs)?;
            }
            InvokePruneReason::ProtectedOpCannotThrow => {
                verify_protected_block_cannot_throw(pre_func, block, interns, &mut cx)?;
            }
        }
    }
    Ok(())
}

fn verify_noop_unwind_chain(
    func: &Func,
    interns: &Interns,
    start: RBlockId,
    pre_funcs: &BTreeMap<JsName, Result<Func, Decline>>,
) -> Result<(), String> {
    let mut cur = start;
    let mut visited = BTreeSet::new();
    for _ in 0..8 {
        if !visited.insert(cur) {
            return Err(format!("noop unwind chain cycles at bb{}", cur.0));
        }
        let block = func
            .blocks
            .get(cur.0 as usize)
            .ok_or_else(|| format!("missing unwind block bb{}", cur.0))?;
        for op in &block.ops {
            match &op.kind {
                OpKind::Copy(_) | OpKind::Move(_) => {}
                OpKind::Drop { glue, .. } => match glue {
                    DropGlue::None => {}
                    DropGlue::Direct { link_name, .. } => {
                        verify_empty_return_callee(interns, *link_name, pre_funcs)?;
                    }
                    DropGlue::Dynamic => {
                        return Err(format!("dynamic drop in unwind block bb{}", cur.0));
                    }
                },
                OpKind::Call {
                    callee: Callee::Direct { link_name, .. },
                    semantic: CallSemantic::Ordinary,
                    host: None,
                    function: None,
                    ..
                } => {
                    if let Some(res) = op.result {
                        if value_is_used_anywhere(func, res) {
                            return Err(format!("unwind call result v{} is used", res.0));
                        }
                    }
                    verify_empty_return_callee(interns, *link_name, pre_funcs)?;
                }
                other => {
                    return Err(format!("non-noop op {other:?} in unwind block bb{}", cur.0));
                }
            }
        }
        match &block.term {
            RTerm::Throw(None) => return Ok(()),
            RTerm::Goto(next, _) => cur = *next,
            other => {
                return Err(format!(
                    "unwind block bb{} ends in non-noop terminator {other:?}",
                    cur.0
                ));
            }
        }
    }
    Err("noop unwind chain exceeds 8-block bound".to_owned())
}

fn value_is_used_anywhere(func: &Func, val: ValueId) -> bool {
    func.blocks.iter().any(|b| {
        b.ops.iter().any(|op| op.kind.uses().contains(&val)) || b.term.uses().contains(&val)
    })
}

fn verify_empty_return_callee(
    interns: &Interns,
    link_name: StrId,
    pre_funcs: &BTreeMap<JsName, Result<Func, Decline>>,
) -> Result<(), String> {
    let name = interns
        .str(link_name)
        .ok_or_else(|| format!("unresolvable str#{} in unwind noop call", link_name.0))?;
    let js_name = JsName::new(name.to_owned());
    let callee = pre_funcs
        .get(&js_name)
        .and_then(|res| res.as_ref().ok())
        .ok_or_else(|| format!("unwind callee `{name}` has no compiled JIR body"))?;
    if callee.blocks.len() == 1
        && callee.blocks[0].ops.is_empty()
        && matches!(callee.blocks[0].term, RTerm::Return(None))
    {
        Ok(())
    } else {
        Err(format!("unwind callee `{name}` is not an empty return"))
    }
}

struct InvokeCheckCx<'a> {
    table: &'a BTreeMap<JsName, LinkItem>,
    pre_funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
    live: &'a BTreeSet<JsName>,
    has_custom_allocator: bool,
    vtable_scope: Option<Result<WitnessVtableScope, String>>,
    indirect_scope: Option<Result<WitnessIndirectScope, String>>,
}

struct WitnessVtableScope {
    drops: Vec<JsName>,
    methods: BTreeMap<u32, Vec<JsName>>,
}

struct WitnessIndirectScope {
    scan: RefScan,
    by_item: BTreeMap<ItemId, Vec<JsName>>,
    static_fn_refs: Vec<JsName>,
}

impl<'a> InvokeCheckCx<'a> {
    fn new(
        table: &'a BTreeMap<JsName, LinkItem>,
        pre_funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
        live: &'a BTreeSet<JsName>,
    ) -> Self {
        let has_custom_allocator = table.values().any(|item| {
            matches!(
                item.late_ir.artifact_allocator_provider(),
                AllocatorProvider::Custom | AllocatorProvider::Conflict
            )
        });
        Self {
            table,
            pre_funcs,
            live,
            has_custom_allocator,
            vtable_scope: None,
            indirect_scope: None,
        }
    }

    fn vtables(&mut self) -> Result<&WitnessVtableScope, String> {
        if self.vtable_scope.is_none() {
            self.vtable_scope = Some(build_witness_vtable_scope(self.table, self.live));
        }
        match self.vtable_scope.as_ref().expect("initialized above") {
            Ok(scope) => Ok(scope),
            Err(err) => Err(err.clone()),
        }
    }

    fn indirect(&mut self) -> Result<&WitnessIndirectScope, String> {
        if self.indirect_scope.is_none() {
            self.indirect_scope = Some(build_witness_indirect_scope(
                self.table,
                self.pre_funcs,
                self.live,
            ));
        }
        match self.indirect_scope.as_ref().expect("initialized above") {
            Ok(scope) => Ok(scope),
            Err(err) => Err(err.clone()),
        }
    }
}

fn build_witness_vtable_scope(
    table: &BTreeMap<JsName, LinkItem>,
    live: &BTreeSet<JsName>,
) -> Result<WitnessVtableScope, String> {
    let mut drops = Vec::new();
    let mut methods: BTreeMap<u32, Vec<JsName>> = BTreeMap::new();
    for name in live {
        let Some(item) = table.get(name) else {
            continue;
        };
        if item.kind != ItemKind::Vtable {
            continue;
        }
        let (Some(body), Some(interns)) = (item.jir.body.as_ref(), item.jir.interns()) else {
            return Err(format!("live vtable `{name}` lacks JIR body or interns"));
        };
        let BodyKind::Vtable { entries, .. } = &body.kind else {
            return Err(format!("live vtable `{name}` has non-vtable body kind"));
        };
        for (slot, entry) in entries.iter().enumerate() {
            match entry {
                VtableEntry::Drop(Some(d)) => {
                    let s = interns.str(d.link_name).ok_or_else(|| {
                        format!("live vtable `{name}` drop entry has invalid str")
                    })?;
                    drops.push(JsName::new(s.to_owned()));
                }
                VtableEntry::Method(m) => {
                    let s = interns.str(m.link_name).ok_or_else(|| {
                        format!("live vtable `{name}` slot {slot} has invalid str")
                    })?;
                    methods
                        .entry(slot as u32)
                        .or_default()
                        .push(JsName::new(s.to_owned()));
                }
                _ => {}
            }
        }
    }
    Ok(WitnessVtableScope { drops, methods })
}

fn witness_type_contains_fn_ptr(
    ty: TypeId,
    types: &TypeTable,
    seen: &mut BTreeSet<TypeId>,
) -> bool {
    if !seen.insert(ty) {
        return false;
    }
    let Some(desc) = types.get(ty) else {
        return true;
    };
    match &desc.kind {
        TypeKind::FnPtr { .. } => true,
        TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
            witness_type_contains_fn_ptr(*pointee, types, seen)
        }
        TypeKind::Slice(elem) | TypeKind::Array { element: elem, .. } => {
            witness_type_contains_fn_ptr(*elem, types, seen)
        }
        TypeKind::Tuple(fields) => fields
            .iter()
            .any(|f| witness_type_contains_fn_ptr(*f, types, seen)),
        TypeKind::Adt { variants, .. } => variants.iter().any(|v| {
            v.fields
                .iter()
                .any(|f| witness_type_contains_fn_ptr(f.ty, types, seen))
        }),
        _ => false,
    }
}

fn build_witness_indirect_scope(
    table: &BTreeMap<JsName, LinkItem>,
    pre_funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    live: &BTreeSet<JsName>,
) -> Result<WitnessIndirectScope, String> {
    let extra_roots = &crate::opts::get().roots;
    let mut static_fn_refs = Vec::new();
    for name in live {
        let Some(item) = table.get(name) else {
            continue;
        };
        match item.kind {
            ItemKind::Fn => {
                if !pre_funcs.get(name).is_some_and(Result::is_ok) {
                    return Err(format!(
                        "unbounded indirect call: live function `{name}` has no JIR body"
                    ));
                }
            }
            ItemKind::Vtable | ItemKind::Alloc | ItemKind::Import => {}
            _ => {
                for r in &item.refs {
                    if pre_funcs.get(r).is_some_and(Result::is_ok)
                        || table
                            .get(r)
                            .is_some_and(|it| matches!(it.kind, ItemKind::Fn | ItemKind::Import))
                    {
                        static_fn_refs.push(r.clone());
                    }
                }
            }
        }
    }

    let mut scan = RefScan::default();
    let mut by_item: BTreeMap<ItemId, Vec<JsName>> = BTreeMap::new();
    for (name, res) in pre_funcs {
        let Ok(func) = res else { continue };
        by_item.entry(func.item).or_default().push(name.clone());
        let types = table
            .get(name)
            .and_then(|it| it.jir.interns())
            .map(|i| &i.types);
        if let (Some(item), Some(types)) = (table.get(name), types) {
            let is_root = item.linkage.root || extra_roots.iter().any(|r| r == name.as_str());
            if is_root
                && func.sig.arguments.iter().any(|&arg_ty| {
                    witness_type_contains_fn_ptr(arg_ty, types, &mut BTreeSet::new())
                })
            {
                return Err(format!(
                    "unbounded indirect call: root `{name}` accepts a function pointer"
                ));
            }
        }
        for block in &func.blocks {
            for op in &block.ops {
                refs::scan_op(func, op, types, &mut scan);
                let fn_ptr_boundary = match &op.kind {
                    OpKind::Host { args, .. }
                    | OpKind::Call {
                        host: Some(_),
                        args,
                        ..
                    } => Some(args.as_slice()),
                    OpKind::Call {
                        callee: Callee::Runtime { sysroot: false, .. },
                        args,
                        ..
                    } => Some(args.as_slice()),
                    OpKind::Call {
                        callee: Callee::Foreign { symbol, .. },
                        args,
                        ..
                    } => {
                        let known_ok = table
                            .get(name)
                            .and_then(|it| it.jir.interns())
                            .and_then(|i| i.str(*symbol))
                            .is_some_and(|s| {
                                pre_funcs
                                    .get(&JsName::new(s.to_owned()))
                                    .is_some_and(Result::is_ok)
                            });
                        (!known_ok).then_some(args.as_slice())
                    }
                    _ => None,
                };
                if let Some(args) = fn_ptr_boundary {
                    let Some(types) = types else {
                        return Err(format!(
                            "unbounded indirect call: missing types for `{name}`"
                        ));
                    };
                    if let Some(res_val) = op.result {
                        if let Some(vdef) = func.values.get(res_val.0 as usize) {
                            if witness_type_contains_fn_ptr(vdef.ty, types, &mut BTreeSet::new()) {
                                return Err(format!(
                                    "unbounded indirect call: external/host op in `{name}` returns a fn ptr"
                                ));
                            }
                        }
                    }
                    if args.iter().any(|arg| {
                        func.values.get(arg.0 as usize).is_some_and(|vdef| {
                            witness_type_contains_fn_ptr(vdef.ty, types, &mut BTreeSet::new())
                        })
                    }) {
                        return Err(format!(
                            "unbounded indirect call: external/host op in `{name}` receives a fn ptr"
                        ));
                    }
                }
            }
        }
    }
    Ok(WitnessIndirectScope {
        scan,
        by_item,
        static_fn_refs,
    })
}

fn verify_protected_block_cannot_throw(
    pre_func: &Func,
    block: &crate::jir::repr::RBlock,
    interns: &Interns,
    cx: &mut InvokeCheckCx<'_>,
) -> Result<(), String> {
    let mut queue = VecDeque::new();
    let mut visited_callees = BTreeSet::new();
    if let Some((last, prefix)) = block.ops.split_last() {
        for op in prefix {
            check_op_no_throw(pre_func, op, interns, cx, &mut visited_callees, &mut queue)?;
        }
        check_op_no_throw(
            pre_func,
            last,
            interns,
            cx,
            &mut visited_callees,
            &mut queue,
        )?;
    }
    drain_callee_queue(cx, &mut visited_callees, &mut queue)
}

fn drain_callee_queue(
    cx: &mut InvokeCheckCx<'_>,
    visited_callees: &mut BTreeSet<JsName>,
    queue: &mut VecDeque<JsName>,
) -> Result<(), String> {
    while let Some(callee_name) = queue.pop_front() {
        if let Some(Ok(callee_func)) = cx.pre_funcs.get(&callee_name) {
            let callee_interns = cx
                .table
                .get(&callee_name)
                .and_then(|item| item.jir.interns())
                .ok_or_else(|| format!("callee `{callee_name}` missing JIR interns"))?;
            if callee_func.blocks.is_empty() {
                continue;
            }
            let mut visited_blocks = vec![false; callee_func.blocks.len()];
            let mut work = vec![0usize];
            while let Some(bi) = work.pop() {
                if visited_blocks.get(bi).copied().unwrap_or(true) {
                    continue;
                }
                visited_blocks[bi] = true;
                let cb = &callee_func.blocks[bi];
                if matches!(cb.term, RTerm::Throw(_)) {
                    return Err(format!(
                        "callee `{callee_name}` reaches RTerm::Throw at bb{bi}"
                    ));
                }
                match &cb.term {
                    RTerm::Invoke { normal, unwind } => {
                        work.push(normal.0 .0 as usize);
                        if let Some((last, prefix)) = cb.ops.split_last() {
                            for op in prefix {
                                check_op_no_throw(
                                    callee_func,
                                    op,
                                    callee_interns,
                                    cx,
                                    visited_callees,
                                    queue,
                                )?;
                            }
                            if op_no_throw_isolated(
                                callee_func,
                                last,
                                callee_interns,
                                cx,
                                visited_callees,
                            )
                            .is_err()
                            {
                                work.push(unwind.0 .0 as usize);
                            }
                        }
                    }
                    other => {
                        for op in &cb.ops {
                            check_op_no_throw(
                                callee_func,
                                op,
                                callee_interns,
                                cx,
                                visited_callees,
                                queue,
                            )?;
                        }
                        for succ in other.successors() {
                            work.push(succ.0 as usize);
                        }
                    }
                }
            }
        } else {
            let item = cx
                .table
                .get(&callee_name)
                .ok_or_else(|| format!("unresolved callee `{callee_name}` may throw"))?;
            match item.kind {
                ItemKind::Alloc => {
                    if cx.has_custom_allocator {
                        return Err(format!(
                            "allocator shim `{callee_name}` with custom allocator may throw"
                        ));
                    }
                }
                ItemKind::Import => {
                    return Err(format!("foreign import `{callee_name}` may throw"));
                }
                ItemKind::Fn => {
                    return Err(format!(
                        "JIR-declined or opaque function `{callee_name}` may throw"
                    ));
                }
                other => {
                    return Err(format!(
                        "callee `{callee_name}` of kind {other:?} may throw"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn op_no_throw_isolated(
    func: &Func,
    op: &Op,
    interns: &Interns,
    cx: &mut InvokeCheckCx<'_>,
    outer_visited: &BTreeSet<JsName>,
) -> Result<(), String> {
    let mut sub_visited = outer_visited.clone();
    let mut sub_queue = VecDeque::new();
    check_op_no_throw(func, op, interns, cx, &mut sub_visited, &mut sub_queue)?;
    drain_callee_queue(cx, &mut sub_visited, &mut sub_queue)
}

fn enqueue_callee(name: JsName, visited: &mut BTreeSet<JsName>, queue: &mut VecDeque<JsName>) {
    if visited.insert(name.clone()) {
        queue.push_back(name);
    }
}

fn witness_is_marker_name(name: &str) -> bool {
    name.starts_with("rcgjs.host.")
        || name.starts_with("rcgjs.tc.")
        || name.starts_with("rcgjs.map.")
}

fn check_intrinsic_without_fallback(
    func: &Func,
    name: StrId,
    args: &[ValueId],
    interns: &Interns,
    visited: &mut BTreeSet<JsName>,
    queue: &mut VecDeque<JsName>,
) -> Result<(), String> {
    let sym = interns
        .str(name)
        .ok_or_else(|| format!("unresolvable intrinsic str#{}", name.0))?;
    match sym {
        "catch_unwind" => Err("intrinsic `catch_unwind` calls closure uncaught".to_owned()),
        "const_eval_select" => {
            let runtime_arg = args
                .get(2)
                .ok_or_else(|| "const_eval_select missing runtime arm".to_owned())?;
            let vdef = func
                .values
                .get(runtime_arg.0 as usize)
                .ok_or_else(|| "const_eval_select runtime arm value missing".to_owned())?;
            let peeled = interns.types.peeled(vdef.ty);
            let desc = interns
                .types
                .get(peeled)
                .ok_or_else(|| "const_eval_select runtime arm type missing".to_owned())?;
            let TypeKind::FnDef {
                instance: Some(instance),
                ..
            } = &desc.kind
            else {
                return Err("const_eval_select runtime arm is not a resolved FnDef".to_owned());
            };
            let target = interns
                .str(instance.link_name)
                .ok_or_else(|| "const_eval_select target link_name unresolvable".to_owned())?;
            if witness_is_marker_name(target) {
                return Err(format!(
                    "const_eval_select target `{target}` is a host marker"
                ));
            }
            enqueue_callee(JsName::new(target.to_owned()), visited, queue);
            Ok(())
        }
        _ => Ok(()),
    }
}

fn check_op_no_throw(
    func: &Func,
    op: &Op,
    interns: &Interns,
    cx: &mut InvokeCheckCx<'_>,
    visited: &mut BTreeSet<JsName>,
    queue: &mut VecDeque<JsName>,
) -> Result<(), String> {
    let resolve = |id: StrId, label: &str| -> Result<JsName, String> {
        interns
            .str(id)
            .map(|s| JsName::new(s.to_owned()))
            .ok_or_else(|| format!("unresolvable {label} str#{}", id.0))
    };
    match &op.kind {
        OpKind::Host { payload, .. } => verify_host_payload_no_throw(payload, interns),
        OpKind::Intrinsic {
            fallback: Some(d), ..
        } => {
            let name = resolve(d.link_name, "intrinsic fallback")?;
            enqueue_callee(name, visited, queue);
            Ok(())
        }
        OpKind::Intrinsic {
            name,
            fallback: None,
            args,
            ..
        } => check_intrinsic_without_fallback(func, *name, args, interns, visited, queue),
        OpKind::Drop { glue, .. } => match glue {
            DropGlue::None => Ok(()),
            DropGlue::Direct { link_name, .. } => {
                let name = resolve(*link_name, "direct drop")?;
                enqueue_callee(name, visited, queue);
                Ok(())
            }
            DropGlue::Dynamic => {
                let drops = cx.vtables()?.drops.clone();
                for name in drops {
                    enqueue_callee(name, visited, queue);
                }
                Ok(())
            }
        },
        OpKind::Call {
            callee,
            args,
            semantic,
            host,
            ..
        } => {
            if let Some(hp) = host {
                return verify_host_payload_no_throw(hp, interns);
            }
            if matches!(
                semantic,
                CallSemantic::Host | CallSemantic::ViewAbi | CallSemantic::RawJavaScript
            ) {
                return Err(format!("call semantic {semantic:?} may throw"));
            }
            match callee {
                Callee::Nop => Ok(()),
                Callee::Direct { link_name, .. } => {
                    let s = interns
                        .str(*link_name)
                        .ok_or_else(|| format!("unresolvable direct call str#{}", link_name.0))?;
                    if witness_is_marker_name(s) {
                        return Err(format!("marker-named direct call `{s}` may throw"));
                    }
                    enqueue_callee(JsName::new(s.to_owned()), visited, queue);
                    Ok(())
                }
                Callee::Intrinsic {
                    fallback: Some(d), ..
                } => {
                    let name = resolve(d.link_name, "intrinsic fallback")?;
                    enqueue_callee(name, visited, queue);
                    Ok(())
                }
                Callee::Intrinsic {
                    name,
                    fallback: None,
                    ..
                } => check_intrinsic_without_fallback(func, *name, args, interns, visited, queue),
                Callee::Runtime { member, sysroot } => {
                    if !*sysroot {
                        let m = interns.str(*member).unwrap_or("<invalid>");
                        return Err(format!("non-sysroot Callee::Runtime `{m}` may throw"));
                    }
                    let m = interns
                        .str(*member)
                        .ok_or_else(|| format!("unresolvable runtime member str#{}", member.0))?;
                    if !witness_sysroot_shim_is_non_throwing(m) {
                        return Err(format!("sysroot runtime member `{m}` may throw"));
                    }
                    Ok(())
                }
                Callee::Foreign { symbol, .. } => {
                    let sym = interns
                        .str(*symbol)
                        .ok_or_else(|| format!("unresolvable foreign symbol str#{}", symbol.0))?;
                    let js_name = JsName::new(sym.to_owned());
                    if !cx.table.contains_key(&js_name) {
                        return Err(format!("unresolved foreign symbol `{sym}` may throw"));
                    }
                    enqueue_callee(js_name, visited, queue);
                    Ok(())
                }
                Callee::Virtual { slot, .. } => {
                    let targets = cx
                        .vtables()?
                        .methods
                        .get(slot)
                        .ok_or_else(|| format!("unbounded virtual call at slot {slot}"))?
                        .clone();
                    for name in targets {
                        enqueue_callee(name, visited, queue);
                    }
                    Ok(())
                }
                Callee::Indirect { signature } => {
                    let indirect = cx.indirect()?;
                    let targets =
                        indirect
                            .scan
                            .fn_pointer_targets
                            .get(signature)
                            .ok_or_else(|| {
                                format!("unbounded indirect call for signature ty#{}", signature.0)
                            })?;
                    let mut names = indirect.static_fn_refs.clone();
                    for item_id in indirect.scan.named.iter().chain(targets.iter()) {
                        let item_names = indirect.by_item.get(item_id).ok_or_else(|| {
                            format!("indirect target {item_id:?} has no JIR item")
                        })?;
                        names.extend(item_names.iter().cloned());
                    }
                    for name in names {
                        enqueue_callee(name, visited, queue);
                    }
                    Ok(())
                }
            }
        }
        _ => Ok(()),
    }
}

fn verify_host_payload_no_throw(payload: &HostPayload, interns: &Interns) -> Result<(), String> {
    let HostPayload::Marker {
        section,
        template: None,
        helpers,
        ..
    } = payload
    else {
        return Err("non-marker or templated host payload may throw".to_owned());
    };
    if !helpers.is_empty() {
        return Err("host payload with helpers may throw".to_owned());
    }
    let sec = interns
        .str(*section)
        .ok_or_else(|| "unresolvable host payload section".to_owned())?;
    let op_name = sec
        .strip_prefix("rcgjs.host.")
        .ok_or_else(|| format!("unknown host marker `{sec}` may throw"))?;
    let op = rcgjs_object::host::HostOperation::from_marker(op_name)
        .ok_or_else(|| format!("unrecognized host operation `{op_name}` may throw"))?;
    if op.may_throw() {
        Err(format!("host operation `{op_name}` may throw"))
    } else {
        Ok(())
    }
}

fn witness_sysroot_shim_is_non_throwing(sym: &str) -> bool {
    matches!(
        sym,
        "alloc"
            | "dealloc"
            | "realloc"
            | "alloc_zeroed"
            | "alloc_error"
            | "panic"
            | "abort"
            | "abort_error"
            | "js_abort"
            | "unreachable"
            | "memcpy"
            | "memmove"
            | "memset"
            | "memcmp"
            | "bcmp"
            | "strlen"
            | "copy"
            | "copy_nonoverlapping"
            | "write_bytes"
            | "overwrite"
            | "_put"
            | "box"
            | "unscale"
            | "scale"
            | "scale_scalar"
            | "byte_sign"
            | "retype"
            | "retype_rc"
            | "offset"
            | "offset_from"
            | "ptr_eq"
            | "slice_ptr_eq"
            | "fat_ptr_eq"
            | "ptr_cmp"
            | "addr"
            | "slice_from_parts"
            | "chunk_slice"
            | "thin"
            | "tail_raw"
            | "tail_ref"
            | "read_array"
            | "write_array"
            | "byte_slice_ref"
            | "str_bytes"
            | "str_bytes_mut"
            | "bytes_str"
            | "str_len"
            | "f2i"
            | "fmod"
            | "fmodf"
            | "clone"
            | "u8array_from_bytes"
            | "native_str_wf"
            | "native_str_utf16_index"
            | "native_str_byte_index"
            | "native_str_is_char_boundary"
            | "native_f64_to_string"
            | "native_str_push"
            | "native_str_pop"
            | "native_str_truncate"
            | "_sb_check"
            | "native_vec_new"
            | "typed_vec_push"
            | "native_vec_set_capacity"
            | "native_vec_set_len"
            | "convert_host_sb"
            | "convert_str_block"
            | "convert_sb_block"
            | "convert_nv_block"
            | "convert_block_nv"
            | "convert_block_sv"
            | "convert_sv_nv"
            | "ptr_is_null"
    )
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
                    inline_never: false,
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
