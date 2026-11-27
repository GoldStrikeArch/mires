//! Optimization witnesses: genuine semantic rewrites record a [`Witness`], and
//! [`Witness::check_with_context`] re-derives the claim independently of the pass that made it.
//!
//! A failed check declines the item's rewrite with `proof.witness` — the item falls back to its
//! baseline text and the link goes on. Whole-program `prune_invoke` rewrites are additionally
//! validated by [`check_invoke_pruning`].

use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::num::NonZeroU128;
use std::sync::OnceLock;

use crate::analysis::arena::{ArenaFieldKind, ProgramBoundsFacts};
use crate::analysis::heap::is_global_vec_ty;
use crate::analysis::refs::{self, RefScan};
use crate::item::AllocatorProvider;
use crate::item::{ItemKind, JsName, LinkItem};
use crate::jir::decline::Decline;
use crate::jir::interns::{Interns, StrId};
use crate::jir::repr::{CellId, Func, Op, OpKind, RBlockId, RTerm, Root, ValueId};
use crate::jir::semantic::{
    BinaryOp, BodyKind, CallSemantic, Callee, CastKind, ConstTree, DefinitionId, DropGlue,
    HostPayload, ItemId, TypeId, UnaryOp, VtableEntry,
};
use crate::jir::types::TypeKind;
use crate::jir::TypeTable;

use super::cfg::{mask, mask_bits, sign_extend, Cfg, DefUse};
use super::prune_invoke::{
    check_noop_unwind_chain, host_payload_cannot_throw, is_non_throwing_sysroot_shim_member,
    is_panic_impl_item, type_contains_fn_ptr,
};

/// Raw scalar constant bits paired with their byte width for [`Witness::ConstFold`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScalarBits {
    pub bits: u128,
    pub bytes: u8,
}

/// Sentinel value paired with the dominating guard that ruled it out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SentinelGuard {
    pub value: u128,
    pub block: RBlockId,
    pub cond: ValueId,
}

/// Constant subtraction offset on a [`BoundsFact::LocalBoundedVec`] index.
///
/// Any non-zero offset structurally requires its dominating guard block and condition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubOffset {
    Zero,
    Guarded {
        offset: NonZeroU128,
        block: RBlockId,
        cond: ValueId,
    },
}

/// Failure returned by independent witness verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WitnessError {
    /// Top-level witness check failure naming the failing witness and its cause.
    Witness {
        witness: Box<Witness>,
        cause: Box<WitnessError>,
    },
    UnsupportedConstFoldType(TypeId),
    UnsupportedConstFoldOp(BinaryOp),
    ConstFoldMismatch {
        computed: bool,
        claimed: bool,
    },
    BranchFoldMismatch {
        selected: RBlockId,
        claimed: RBlockId,
    },
    MissingBlock(RBlockId),
    OperandsOutOfRange,
    NumericBoundInvalid {
        index_hi: u128,
        length_lo: u128,
    },
    DominatingOffsetNotImplied {
        offset_index: u128,
        offset_fact: u128,
    },
    BlockDoesNotDominate {
        role: &'static str,
        dominator: RBlockId,
        target: RBlockId,
    },
    GuardConditionMissing {
        role: &'static str,
        cond: ValueId,
    },
    BitmaskOperandOutOfRange,
    BitmaskIndexMismatch {
        index: ValueId,
        and_val: ValueId,
    },
    NotBitAnd(ValueId),
    BitmaskNotUnsigned,
    BitmaskMaskNotOperand {
        mask: ValueId,
        and_val: ValueId,
    },
    BitmaskLengthMismatch {
        len_val: ValueId,
        assert_len: ValueId,
    },
    BitmaskNotProvenBelow {
        mask_val: ValueId,
        len_val: ValueId,
    },
    ArenaFactsMissing,
    ArenaIndexNotUnsigned,
    ArenaNotProven(DefinitionId),
    ArenaSentinelMismatch {
        field: u32,
        expected: u128,
        actual: Option<u128>,
    },
    ArenaFieldNotProven {
        field: u32,
        kind: &'static str,
    },
    ArenaParamNotCertified {
        idx_param: Option<usize>,
        elem_def: DefinitionId,
    },
    ArenaCalleeMismatch {
        callee: ItemId,
        detail: &'static str,
    },
    NotBlockParam(ValueId),
    LocalVecRootInvalid {
        root: LocalVecRoot,
        detail: &'static str,
    },
    LoadValOutOfRange(ValueId),
    LoadValNotDeref(ValueId),
    LocalVecIndexMismatch {
        index: ValueId,
        load_val: ValueId,
        sub_offset: Option<NonZeroU128>,
    },
    AlignedPrefixMismatch {
        expected: Option<bool>,
    },
    InvokePruneFaultInjected,
    InvokePruneNoSites,
    InvokePruneMissingBlock(RBlockId),
    InvokePruneNotInvoke(RBlockId),
    InvokePruneNoopChain(String),
    InvokePruneMayThrow(String),
}

impl fmt::Display for WitnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Witness { witness, cause } => write!(f, "{witness:?}: {cause}"),
            Self::UnsupportedConstFoldType(ty) => {
                write!(f, "unsupported ConstFold operand type t{}", ty.0)
            }
            Self::UnsupportedConstFoldOp(op) => {
                write!(f, "unsupported ConstFold comparison {op:?}")
            }
            Self::ConstFoldMismatch { computed, claimed } => {
                write!(f, "the evaluator computed {computed}, not {claimed}")
            }
            Self::BranchFoldMismatch { selected, claimed } => {
                write!(
                    f,
                    "case selection lands on {}, not {}",
                    selected.0, claimed.0
                )
            }
            Self::MissingBlock(block) => write!(f, "block {} is gone", block.0),
            Self::OperandsOutOfRange => f.write_str("the assert's operands are gone"),
            Self::NumericBoundInvalid {
                index_hi,
                length_lo,
            } => write!(f, "{index_hi} is not below {length_lo}"),
            Self::DominatingOffsetNotImplied {
                offset_index,
                offset_fact,
            } => write!(
                f,
                "offset {offset_index} is not inside the fact's {offset_fact}"
            ),
            Self::BlockDoesNotDominate {
                role,
                dominator,
                target,
            } => write!(
                f,
                "{role} block {} does not dominate block {}",
                dominator.0, target.0
            ),
            Self::GuardConditionMissing { role, cond } => {
                write!(f, "{role} v{} is no longer present", cond.0)
            }
            Self::BitmaskOperandOutOfRange => {
                f.write_str("bitmask witness operand is out of range")
            }
            Self::BitmaskIndexMismatch { index, and_val } => {
                write!(
                    f,
                    "index v{} does not peel to BitAnd v{}",
                    index.0, and_val.0
                )
            }
            Self::NotBitAnd(v) => write!(f, "v{} is not a BinaryOp::BitAnd", v.0),
            Self::BitmaskNotUnsigned => f.write_str("bitmask BitAnd operand type is not unsigned"),
            Self::BitmaskMaskNotOperand { mask, and_val } => {
                write!(
                    f,
                    "mask v{} is not an operand of BitAnd v{}",
                    mask.0, and_val.0
                )
            }
            Self::BitmaskLengthMismatch {
                len_val,
                assert_len,
            } => write!(
                f,
                "bitmask len_val v{} does not match assert length v{}",
                len_val.0, assert_len.0
            ),
            Self::BitmaskNotProvenBelow { mask_val, len_val } => write!(
                f,
                "mask v{} is not structurally proven below nonzero length v{}",
                mask_val.0, len_val.0
            ),
            Self::ArenaFactsMissing => {
                f.write_str("MonotoneArena witness requires whole-program arena facts")
            }
            Self::ArenaIndexNotUnsigned => {
                f.write_str("MonotoneArena index is not an unsigned integer")
            }
            Self::ArenaNotProven(elem_def) => write!(f, "arena {elem_def:?} is not proven"),
            Self::ArenaSentinelMismatch {
                field,
                expected,
                actual,
            } => write!(
                f,
                "field {field} requires sentinel {expected} guard, got {actual:?}"
            ),
            Self::ArenaFieldNotProven { field, kind } => {
                write!(f, "field {field} is not a proven {kind} field")
            }
            Self::ArenaParamNotCertified {
                idx_param: None, ..
            } => f.write_str("function has no certified ParamArenaIndex facts"),
            Self::ArenaParamNotCertified {
                idx_param: Some(idx_param),
                elem_def,
            } => write!(
                f,
                "param {idx_param} is not certified for arena {elem_def:?}"
            ),
            Self::ArenaCalleeMismatch { callee, detail } => {
                write!(f, "callee {callee:?} {detail}")
            }
            Self::NotBlockParam(param) => write!(f, "v{} is not a block parameter", param.0),
            Self::LocalVecRootInvalid { root, detail } => match root {
                LocalVecRoot::Cell(cell) => write!(f, "local vec cell {} {detail}", cell.0),
                LocalVecRoot::Value(val) => write!(f, "local vec value v{} {detail}", val.0),
            },
            Self::LoadValOutOfRange(v) => write!(f, "load_val v{} is out of range", v.0),
            Self::LoadValNotDeref(v) => write!(f, "load_val v{} is not a Deref load", v.0),
            Self::LocalVecIndexMismatch {
                index,
                load_val,
                sub_offset: None,
            } => write!(
                f,
                "index v{} does not match load_val v{}",
                index.0, load_val.0
            ),
            Self::LocalVecIndexMismatch {
                index,
                load_val,
                sub_offset: Some(offset),
            } => write!(
                f,
                "index v{} is not load_val v{} - {offset}",
                index.0, load_val.0
            ),
            Self::AlignedPrefixMismatch {
                expected: Some(expected),
            } => write!(f, "the aligned-prefix relation evaluates to {expected}"),
            Self::AlignedPrefixMismatch { expected: None } => {
                f.write_str("the operands are not a proven aligned-prefix relation")
            }
            Self::InvokePruneFaultInjected => f.write_str("injected prune-invoke witness fault"),
            Self::InvokePruneNoSites => f.write_str("no invoke pruning sites recorded"),
            Self::InvokePruneMissingBlock(block) => {
                write!(f, "pruned block bb{} does not exist", block.0)
            }
            Self::InvokePruneNotInvoke(block) => {
                write!(
                    f,
                    "pruned block bb{} did not terminate in RTerm::Invoke",
                    block.0
                )
            }
            Self::InvokePruneNoopChain(msg) | Self::InvokePruneMayThrow(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for WitnessError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Witness { cause, .. } => Some(cause.as_ref()),
            _ => None,
        }
    }
}

/// One recorded semantic rewrite whose claim can be re-derived independently.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Witness {
    /// A comparison of two integer/bool constants was folded to a boolean constant.
    ConstFold {
        op: BinaryOp,
        ty: TypeId,
        l: ScalarBits,
        r: ScalarBits,
        result: bool,
    },
    /// A `Switch` on a known scrutinee took one edge: a constant fold, a statically fixed
    /// discriminant, or one threaded edge of a variant-per-path merge.
    BranchFold {
        scrut: u128,
        bytes: u8,
        cases: Vec<(u128, RBlockId)>,
        default: RBlockId,
        taken: RBlockId,
    },
    /// A bounds-check `Assert` was deleted because a dominating fact implies its condition.
    BoundsRemoved {
        block: RBlockId,
        index: ValueId,
        length: ValueId,
        fact: BoundsFact,
    },
    /// An unsigned aligned-prefix comparison was folded from
    /// `prefix = length - (length % nonzero)` and `0 <= prefix <= length`.
    AlignedPrefix {
        left: ValueId,
        right: ValueId,
        op: BinaryOp,
        result: bool,
    },
}

/// The evidence a [`Witness::BoundsRemoved`] carries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoundsFact {
    /// Intervals alone prove it: the index's upper bound is below the length's lower bound.
    Numeric { index_hi: u128, length_lo: u128 },
    /// A dominating comparison proves it: `base + offset_fact` relates to the same length the
    /// assert checks against, and the assert's `base + offset_index` sits inside it.
    Dominating {
        block: RBlockId,
        cond: ValueId,
        offset_index: u128,
        offset_fact: u128,
        strict: bool,
    },
    /// Symbolic bitmask bound: `index` is `x & mask`, where `mask == len - 1` (with `len >= 1`)
    /// or `mask` is a constant strictly below `len`.
    Bitmask {
        and_val: ValueId,
        mask_val: ValueId,
        len_val: ValueId,
        nonzero_site: Option<(RBlockId, ValueId)>,
    },
    /// Whole-program monotone-arena invariant proves `index < length` via [`ProgramBoundsFacts`].
    MonotoneArena {
        elem_def: DefinitionId,
        sentinel: Option<SentinelGuard>,
        source: ArenaIndexSource,
    },
    /// Local non-escaping bounded-element `Vec` invariant: `index` is `load_val - sub_offset`
    /// where every element stored into `vec_root` is bounded by `length + sub_offset - 1`.
    LocalBoundedVec {
        vec_root: LocalVecRoot,
        load_val: ValueId,
        sub_offset: SubOffset,
    },
}

/// Storage root of a local non-escaping `Vec` certified by [`BoundsFact::LocalBoundedVec`].
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LocalVecRoot {
    Cell(CellId),
    Value(ValueId),
}

/// Provenance of an arena index verified by [`BoundsFact::MonotoneArena`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArenaIndexSource {
    Field(u32),
    VecField(u32),
    Param { idx_param: usize },
    CallReturn { callee: ItemId },
    ReturnVecElem { callee: ItemId },
    BlockParam { param: ValueId },
}

impl Witness {
    /// Re-derives this witness's claim against `func`, independently of the pass that made it.
    #[cfg(test)]
    pub fn check(&self, func: &Func, table: &TypeTable) -> Result<(), WitnessError> {
        self.check_with_context(func, table, None)
    }

    /// Re-derives this witness's claim with optional whole-program [`ProgramBoundsFacts`].
    #[cfg(test)]
    pub fn check_with_context(
        &self,
        func: &Func,
        table: &TypeTable,
        arena_facts: Option<&ProgramBoundsFacts>,
    ) -> Result<(), WitnessError> {
        let cx = WitnessCheckCx::new(func);
        self.check_with_cx(&cx, table, arena_facts)
    }

    fn check_with_cx(
        &self,
        cx: &WitnessCheckCx<'_>,
        table: &TypeTable,
        arena_facts: Option<&ProgramBoundsFacts>,
    ) -> Result<(), WitnessError> {
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
            Self::BoundsRemoved {
                block,
                index,
                length,
                fact,
            } => check_bounds_removed(
                cx,
                table,
                arena_facts,
                BoundsSite {
                    block: *block,
                    index: *index,
                    length: *length,
                },
                fact,
            ),
            Self::AlignedPrefix {
                left,
                right,
                op,
                result,
            } => check_aligned_prefix(cx, table, *left, *right, *op, *result),
        }
    }
}

/// Checks every witness; the first failure names the witness and the reason.
pub fn check_all(
    witnesses: &[Witness],
    func: &Func,
    table: &TypeTable,
) -> Result<(), WitnessError> {
    check_all_with_context(witnesses, func, table, None)
}

/// Checks every witness with optional whole-program [`ProgramBoundsFacts`].
pub fn check_all_with_context(
    witnesses: &[Witness],
    func: &Func,
    table: &TypeTable,
    arena_facts: Option<&ProgramBoundsFacts>,
) -> Result<(), WitnessError> {
    let cx = WitnessCheckCx::new(func);
    for witness in witnesses {
        witness
            .check_with_cx(&cx, table, arena_facts)
            .map_err(|cause| WitnessError::Witness {
                witness: Box::new(witness.clone()),
                cause: Box::new(cause),
            })?;
    }
    Ok(())
}

/// Lazily computed per-function indices shared across a batch of witness checks.
struct WitnessCheckCx<'a> {
    func: &'a Func,
    cfg: OnceCell<Cfg>,
    def_use: OnceCell<DefUse>,
}

impl<'a> WitnessCheckCx<'a> {
    fn new(func: &'a Func) -> Self {
        Self {
            func,
            cfg: OnceCell::new(),
            def_use: OnceCell::new(),
        }
    }

    fn cfg(&self) -> &Cfg {
        self.cfg.get_or_init(|| Cfg::of(self.func))
    }

    fn def_use(&self) -> &DefUse {
        self.def_use.get_or_init(|| DefUse::of(self.func))
    }

    fn def_kind(&self, v: ValueId) -> Option<&'a OpKind> {
        self.def_use().def_kind(self.func, v)
    }

    fn cond_present(&self, cond: ValueId) -> bool {
        self.func.blocks.iter().any(|b| {
            matches!(&b.term, RTerm::Switch { scrut, .. } if *scrut == cond)
                || b.ops
                    .iter()
                    .any(|op| matches!(&op.kind, OpKind::Assert { cond: c, .. } if *c == cond))
        })
    }

    fn check_dominating_guard(
        &self,
        role: &'static str,
        guard_block: RBlockId,
        guard_cond: ValueId,
        target_block: RBlockId,
    ) -> Result<(), WitnessError> {
        if !self.cfg().doms.dominates(guard_block, target_block) {
            return Err(WitnessError::BlockDoesNotDominate {
                role,
                dominator: guard_block,
                target: target_block,
            });
        }
        if !self.cond_present(guard_cond) {
            return Err(WitnessError::GuardConditionMissing {
                role,
                cond: guard_cond,
            });
        }
        Ok(())
    }
}

/// The bounds-check assert location and operands verified by [`check_bounds_removed`].
#[derive(Clone, Copy)]
struct BoundsSite {
    block: RBlockId,
    index: ValueId,
    length: ValueId,
}

/// Whether the independent checks run: always in debug builds, and wherever
/// `RCGJS_WITNESS_CHECK=1` asks for them (CI on release artifacts).
pub fn enabled() -> bool {
    if cfg!(debug_assertions) {
        return true;
    }
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("RCGJS_WITNESS_CHECK").is_some())
}

/// Cached process-wide `RCGJS_WITNESS_FAULT` value, if set.
pub(crate) fn env_witness_fault() -> Option<&'static str> {
    static FAULT: OnceLock<Option<String>> = OnceLock::new();
    FAULT
        .get_or_init(|| {
            std::env::var_os("RCGJS_WITNESS_FAULT").map(|v| v.to_string_lossy().into_owned())
        })
        .as_deref()
}

/// `ConstFold`: directly evaluates the folded integer/bool comparison independently of the pass.
fn check_const_fold(
    op: BinaryOp,
    ty: TypeId,
    l: ScalarBits,
    r: ScalarBits,
    result: bool,
    table: &TypeTable,
) -> Result<(), WitnessError> {
    let ordering = if let Some((signed, bits)) = table.int_info(ty) {
        let ml = mask_bits(l.bits, bits);
        let mr = mask_bits(r.bits, bits);
        if signed {
            sign_extend(ml, bits).cmp(&sign_extend(mr, bits))
        } else {
            ml.cmp(&mr)
        }
    } else if matches!(
        table.get(table.peeled(ty)).map(|d| &d.kind),
        Some(TypeKind::Bool)
    ) {
        (mask(l.bits, l.bytes) != 0).cmp(&(mask(r.bits, r.bytes) != 0))
    } else {
        return Err(WitnessError::UnsupportedConstFoldType(ty));
    };
    let computed = match op {
        BinaryOp::Eq => ordering.is_eq(),
        BinaryOp::Ne => ordering.is_ne(),
        BinaryOp::Lt => ordering.is_lt(),
        BinaryOp::Le => ordering.is_le(),
        BinaryOp::Gt => ordering.is_gt(),
        BinaryOp::Ge => ordering.is_ge(),
        other => return Err(WitnessError::UnsupportedConstFoldOp(other)),
    };
    if computed == result {
        Ok(())
    } else {
        Err(WitnessError::ConstFoldMismatch {
            computed,
            claimed: result,
        })
    }
}

/// `BranchFold`: re-runs the case selection over the recorded scrutinee and case table.
fn check_branch_fold(
    scrut: u128,
    bytes: u8,
    cases: &[(u128, RBlockId)],
    default: RBlockId,
    taken: RBlockId,
) -> Result<(), WitnessError> {
    let value = mask(scrut, bytes);
    let selected = cases
        .iter()
        .find(|(case, _)| mask(*case, bytes) == value)
        .map(|(_, target)| *target)
        .unwrap_or(default);
    if selected == taken {
        Ok(())
    } else {
        Err(WitnessError::BranchFoldMismatch {
            selected,
            claimed: taken,
        })
    }
}

fn peel_transparent(cx: &WitnessCheckCx<'_>, table: &TypeTable, mut v: ValueId) -> ValueId {
    for _ in 0..8 {
        match cx.def_kind(v) {
            Some(OpKind::Copy(src) | OpKind::Move(src)) => v = *src,
            Some(OpKind::Cast {
                kind: CastKind::IntToInt,
                v: src,
                from,
                to,
            }) if matches!(
                (table.int_info(*from), table.int_info(*to)),
                (Some((false, a)), Some((false, b))) if a == b
            ) =>
            {
                v = *src;
            }
            None => {
                let mut rep: Option<ValueId> = None;
                let mut same = true;
                if let Some((blk_id, pos)) = cx.def_use().param_site(v).filter(|(b, _)| b.0 != 0) {
                    for &pred_id in &cx.cfg().preds[blk_id.0 as usize] {
                        let pred = &cx.func.blocks[pred_id.0 as usize];
                        for (target, args) in pred.term.edges() {
                            if target != blk_id {
                                continue;
                            }
                            let Some(&arg) = args.get(pos) else {
                                same = false;
                                break;
                            };
                            if arg == v {
                                continue;
                            }
                            match rep {
                                None => rep = Some(arg),
                                Some(prev) if prev == arg => {}
                                Some(_) => {
                                    same = false;
                                    break;
                                }
                            }
                        }
                    }
                }
                if same {
                    if let Some(r) = rep {
                        v = r;
                        continue;
                    }
                }
                break;
            }
            _ => break,
        }
    }
    v
}

/// `BoundsRemoved`: re-runs dominance and the implying fact on the final `Func`.
fn check_bounds_removed(
    cx: &WitnessCheckCx<'_>,
    table: &TypeTable,
    arena_facts: Option<&ProgramBoundsFacts>,
    site: BoundsSite,
    fact: &BoundsFact,
) -> Result<(), WitnessError> {
    let func = cx.func;
    if func.blocks.get(site.block.0 as usize).is_none() {
        return Err(WitnessError::MissingBlock(site.block));
    }
    if (site.index.0 as usize) >= func.values.len() || (site.length.0 as usize) >= func.values.len()
    {
        return Err(WitnessError::OperandsOutOfRange);
    }

    match fact {
        BoundsFact::Numeric {
            index_hi,
            length_lo,
        } => check_bounds_numeric(*index_hi, *length_lo),
        BoundsFact::Dominating {
            block: fact_block,
            cond,
            offset_index,
            offset_fact,
            strict,
        } => check_bounds_dominating(
            cx,
            site.block,
            *fact_block,
            *cond,
            *offset_index,
            *offset_fact,
            *strict,
        ),
        BoundsFact::Bitmask {
            and_val,
            mask_val,
            len_val,
            nonzero_site,
        } => check_bounds_bitmask(
            cx,
            table,
            arena_facts,
            site,
            (*and_val, *mask_val, *len_val, *nonzero_site),
        ),
        BoundsFact::MonotoneArena {
            elem_def,
            sentinel,
            source,
        } => {
            check_bounds_monotone_arena(cx, table, arena_facts, site, *elem_def, *sentinel, *source)
        }
        BoundsFact::LocalBoundedVec {
            vec_root,
            load_val,
            sub_offset,
        } => check_bounds_local_bounded_vec(cx, table, site, *vec_root, *load_val, *sub_offset),
    }
}

fn check_bounds_numeric(index_hi: u128, length_lo: u128) -> Result<(), WitnessError> {
    if index_hi < length_lo {
        Ok(())
    } else {
        Err(WitnessError::NumericBoundInvalid {
            index_hi,
            length_lo,
        })
    }
}

fn check_bounds_dominating(
    cx: &WitnessCheckCx<'_>,
    block: RBlockId,
    fact_block: RBlockId,
    cond: ValueId,
    offset_index: u128,
    offset_fact: u128,
    strict: bool,
) -> Result<(), WitnessError> {
    let implied = if strict {
        offset_index <= offset_fact
    } else {
        offset_index < offset_fact
    };
    if !implied {
        return Err(WitnessError::DominatingOffsetNotImplied {
            offset_index,
            offset_fact,
        });
    }
    cx.check_dominating_guard("dominating fact", fact_block, cond, block)
}

fn check_bounds_bitmask(
    cx: &WitnessCheckCx<'_>,
    table: &TypeTable,
    arena_facts: Option<&ProgramBoundsFacts>,
    site: BoundsSite,
    (and_val, mask_val, len_val, nonzero_site): (
        ValueId,
        ValueId,
        ValueId,
        Option<(RBlockId, ValueId)>,
    ),
) -> Result<(), WitnessError> {
    let func = cx.func;
    if (and_val.0 as usize) >= func.values.len()
        || (mask_val.0 as usize) >= func.values.len()
        || (len_val.0 as usize) >= func.values.len()
    {
        return Err(WitnessError::BitmaskOperandOutOfRange);
    }
    let p_idx = peel_transparent(cx, table, site.index);
    let p_and = peel_transparent(cx, table, and_val);
    if p_idx != p_and {
        return Err(WitnessError::BitmaskIndexMismatch {
            index: site.index,
            and_val,
        });
    }
    let Some(OpKind::Binary {
        op: BinaryOp::BitAnd,
        l,
        r,
        ty,
    }) = cx.def_kind(p_and)
    else {
        return Err(WitnessError::NotBitAnd(p_and));
    };
    if !matches!(table.int_info(*ty), Some((false, _))) {
        return Err(WitnessError::BitmaskNotUnsigned);
    }
    let p_mask = peel_transparent(cx, table, mask_val);
    if peel_transparent(cx, table, *l) != p_mask && peel_transparent(cx, table, *r) != p_mask {
        return Err(WitnessError::BitmaskMaskNotOperand {
            mask: mask_val,
            and_val,
        });
    }
    if let Some((nz_blk, nz_cond)) = nonzero_site {
        cx.check_dominating_guard("bitmask nonzero", nz_blk, nz_cond, site.block)?;
    }
    let p_len = peel_transparent(cx, table, len_val);
    let p_assert_len = peel_transparent(cx, table, site.length);
    let len_matches_assert = p_len == p_assert_len
        || same_one_shl_term(cx, table, p_len, p_assert_len)
        || assert_len_matches_target(cx, table, arena_facts, p_assert_len, p_len);
    if !len_matches_assert {
        return Err(WitnessError::BitmaskLengthMismatch {
            len_val,
            assert_len: site.length,
        });
    }
    let len_proven_nonzero = nonzero_site.is_some()
        || is_one_shl(cx, table, p_len)
        || matches!(
            cx.def_kind(p_len),
            Some(OpKind::Const(ConstTree::Scalar { bits, bytes })) if mask(*bits, *bytes) >= 1
        )
        || is_positive_doubling_param(cx, table, p_len);
    let sub_one_match = len_proven_nonzero
        && match cx.def_kind(p_mask) {
            Some(OpKind::Binary {
                op: BinaryOp::Sub | BinaryOp::SubUnchecked,
                l: sub_l,
                r: sub_r,
                ..
            }) => {
                let is_one = matches!(
                    cx.def_kind(peel_transparent(cx, table, *sub_r)),
                    Some(OpKind::Const(ConstTree::Scalar { bits: 1, .. }))
                );
                if !is_one {
                    false
                } else {
                    let p_sub_l = peel_transparent(cx, table, *sub_l);
                    p_sub_l == p_len
                        || same_one_shl_term(cx, table, p_sub_l, p_len)
                        || p_sub_l == p_assert_len
                }
            }
            _ => false,
        };
    let const_match = match (cx.def_kind(p_mask), cx.def_kind(p_len)) {
        (
            Some(OpKind::Const(ConstTree::Scalar {
                bits: mb,
                bytes: mby,
            })),
            Some(OpKind::Const(ConstTree::Scalar {
                bits: lb,
                bytes: lby,
            })),
        ) => mask(*mb, *mby) < mask(*lb, *lby),
        (
            Some(OpKind::Const(ConstTree::Scalar { bits: 0, .. })),
            Some(OpKind::Binary {
                op: BinaryOp::Shl | BinaryOp::ShlUnchecked,
                l: shl_l,
                ..
            }),
        ) => matches!(
            cx.def_kind(peel_transparent(cx, table, *shl_l)),
            Some(OpKind::Const(ConstTree::Scalar { bits: 1, .. }))
        ),
        _ => false,
    };
    if sub_one_match || const_match {
        Ok(())
    } else {
        Err(WitnessError::BitmaskNotProvenBelow { mask_val, len_val })
    }
}

fn check_bounds_monotone_arena(
    cx: &WitnessCheckCx<'_>,
    table: &TypeTable,
    arena_facts: Option<&ProgramBoundsFacts>,
    site: BoundsSite,
    elem_def: DefinitionId,
    sentinel: Option<SentinelGuard>,
    source: ArenaIndexSource,
) -> Result<(), WitnessError> {
    let func = cx.func;
    let Some(facts) = arena_facts else {
        return Err(WitnessError::ArenaFactsMissing);
    };
    if !matches!(
        table.int_info(table.peeled(func.values[site.index.0 as usize].ty)),
        Some((false, _))
    ) {
        return Err(WitnessError::ArenaIndexNotUnsigned);
    }
    if let Some(guard) = sentinel {
        cx.check_dominating_guard("sentinel guard", guard.block, guard.cond, site.block)?;
    }
    let sentinel_val = sentinel.map(|g| g.value);
    match source {
        ArenaIndexSource::Field(f) => {
            let inv = facts
                .arenas
                .get(&elem_def)
                .ok_or(WitnessError::ArenaNotProven(elem_def))?;
            match inv.fields.get(&f) {
                Some(ArenaFieldKind::AlwaysValid) => Ok(()),
                Some(ArenaFieldKind::ValidOrSentinel { sentinel: s }) => {
                    if sentinel_val == Some(*s) {
                        Ok(())
                    } else {
                        Err(WitnessError::ArenaSentinelMismatch {
                            field: f,
                            expected: *s,
                            actual: sentinel_val,
                        })
                    }
                }
                _ => Err(WitnessError::ArenaFieldNotProven {
                    field: f,
                    kind: "scalar arena",
                }),
            }
        }
        ArenaIndexSource::VecField(f) => {
            let inv = facts
                .arenas
                .get(&elem_def)
                .ok_or(WitnessError::ArenaNotProven(elem_def))?;
            if matches!(inv.fields.get(&f), Some(ArenaFieldKind::VecOfValidIndices)) {
                Ok(())
            } else {
                Err(WitnessError::ArenaFieldNotProven {
                    field: f,
                    kind: "VecOfValidIndices",
                })
            }
        }
        ArenaIndexSource::Param { idx_param } => {
            let Some(list) = facts.param_arena_indices.get(&func.item) else {
                return Err(WitnessError::ArenaParamNotCertified {
                    idx_param: None,
                    elem_def,
                });
            };
            let ok = list.iter().any(|p| {
                p.idx_param == idx_param
                    && p.elem_def == elem_def
                    && (p.sentinel.is_none() || p.sentinel == sentinel_val)
            });
            if ok {
                Ok(())
            } else {
                Err(WitnessError::ArenaParamNotCertified {
                    idx_param: Some(idx_param),
                    elem_def,
                })
            }
        }
        ArenaIndexSource::CallReturn { callee } => {
            let Some(ret) = facts.return_arena_index.get(&callee) else {
                return Err(WitnessError::ArenaCalleeMismatch {
                    callee,
                    detail: "has no certified ReturnArenaIndex",
                });
            };
            if ret.elem_def == elem_def && (ret.sentinel.is_none() || ret.sentinel == sentinel_val)
            {
                Ok(())
            } else {
                Err(WitnessError::ArenaCalleeMismatch {
                    callee,
                    detail: "return arena summary does not match witness",
                })
            }
        }
        ArenaIndexSource::ReturnVecElem { callee } => {
            if facts.return_vec_elem_bound.contains_key(&callee) {
                Ok(())
            } else {
                Err(WitnessError::ArenaCalleeMismatch {
                    callee,
                    detail: "has no certified ReturnVecElemBound",
                })
            }
        }
        ArenaIndexSource::BlockParam { param } => {
            if !facts.arenas.contains_key(&elem_def) {
                return Err(WitnessError::ArenaNotProven(elem_def));
            }
            if cx
                .def_use()
                .param_site(param)
                .is_some_and(|(blk, _)| blk.0 != 0)
            {
                Ok(())
            } else {
                Err(WitnessError::NotBlockParam(param))
            }
        }
    }
}

fn check_bounds_local_bounded_vec(
    cx: &WitnessCheckCx<'_>,
    table: &TypeTable,
    site: BoundsSite,
    vec_root: LocalVecRoot,
    load_val: ValueId,
    sub_offset: SubOffset,
) -> Result<(), WitnessError> {
    let func = cx.func;
    match vec_root {
        LocalVecRoot::Cell(cell) => {
            let Some(cdef) = func.cells.get(cell.0 as usize) else {
                return Err(WitnessError::LocalVecRootInvalid {
                    root: vec_root,
                    detail: "is gone",
                });
            };
            if !is_global_vec_ty(table, cdef.ty) {
                return Err(WitnessError::LocalVecRootInvalid {
                    root: vec_root,
                    detail: "is not a Vec<T, Global>",
                });
            }
            if func.blocks.iter().any(|b| {
                b.ops.iter().any(|op| {
                    matches!(&op.kind, OpKind::RawView { root: Root::Cell(c), .. } if *c == cell)
                })
            }) {
                return Err(WitnessError::LocalVecRootInvalid {
                    root: vec_root,
                    detail: "has an unsafe RawView",
                });
            }
        }
        LocalVecRoot::Value(val) => {
            let Some(vdef) = func.values.get(val.0 as usize) else {
                return Err(WitnessError::LocalVecRootInvalid {
                    root: vec_root,
                    detail: "is gone",
                });
            };
            if !is_global_vec_ty(table, vdef.ty) {
                return Err(WitnessError::LocalVecRootInvalid {
                    root: vec_root,
                    detail: "is not a Vec<T, Global>",
                });
            }
            if func.blocks.iter().any(|b| {
                b.ops.iter().any(|op| {
                    matches!(&op.kind, OpKind::RawView { root: Root::Value(v), .. } if *v == val)
                })
            }) {
                return Err(WitnessError::LocalVecRootInvalid {
                    root: vec_root,
                    detail: "has an unsafe RawView",
                });
            }
        }
    }
    if (load_val.0 as usize) >= func.values.len() {
        return Err(WitnessError::LoadValOutOfRange(load_val));
    }
    let p_load = peel_transparent(cx, table, load_val);
    if !matches!(cx.def_kind(p_load), Some(OpKind::Deref(_))) {
        return Err(WitnessError::LoadValNotDeref(load_val));
    }
    let p_idx = peel_transparent(cx, table, site.index);
    match sub_offset {
        SubOffset::Zero => {
            if p_idx != p_load {
                return Err(WitnessError::LocalVecIndexMismatch {
                    index: site.index,
                    load_val,
                    sub_offset: None,
                });
            }
        }
        SubOffset::Guarded {
            offset,
            block: guard_block,
            cond: guard_cond,
        } => {
            let sub_ok = matches!(
                cx.def_kind(p_idx),
                Some(OpKind::Binary {
                    op: BinaryOp::Sub | BinaryOp::SubUnchecked,
                    l,
                    r,
                    ..
                }) if peel_transparent(cx, table, *l) == p_load
                    && matches!(
                        cx.def_kind(peel_transparent(cx, table, *r)),
                        Some(OpKind::Const(ConstTree::Scalar { bits, bytes }))
                            if mask(*bits, *bytes) == offset.get()
                    )
            );
            if !sub_ok {
                return Err(WitnessError::LocalVecIndexMismatch {
                    index: site.index,
                    load_val,
                    sub_offset: Some(offset),
                });
            }
            cx.check_dominating_guard(
                "LocalBoundedVec guard",
                guard_block,
                guard_cond,
                site.block,
            )?;
        }
    }
    Ok(())
}

fn is_one_shl(cx: &WitnessCheckCx<'_>, table: &TypeTable, v: ValueId) -> bool {
    matches!(
        cx.def_kind(peel_transparent(cx, table, v)),
        Some(OpKind::Binary {
            op: BinaryOp::Shl | BinaryOp::ShlUnchecked,
            l,
            ty,
            ..
        }) if matches!(table.int_info(*ty), Some((false, _)))
            && matches!(
                cx.def_kind(peel_transparent(cx, table, *l)),
                Some(OpKind::Const(ConstTree::Scalar { bits: 1, .. }))
            )
    )
}

fn is_positive_doubling_param(cx: &WitnessCheckCx<'_>, table: &TypeTable, v: ValueId) -> bool {
    let Some((header_id, pos)) = cx.def_use().param_site(v).filter(|(b, _)| b.0 != 0) else {
        return false;
    };
    let mut has_pos_init = false;
    let mut all_ok = true;
    for &pred_id in &cx.cfg().preds[header_id.0 as usize] {
        let b = &cx.func.blocks[pred_id.0 as usize];
        for (target, args) in b.term.edges() {
            if target != header_id {
                continue;
            }
            let Some(&arg) = args.get(pos) else {
                return false;
            };
            if arg == v {
                continue;
            }
            match cx.def_kind(arg) {
                Some(OpKind::Const(ConstTree::Scalar { bits, bytes }))
                    if mask(*bits, *bytes) >= 1 =>
                {
                    has_pos_init = true;
                }
                Some(OpKind::Binary {
                    op: BinaryOp::Shl | BinaryOp::ShlUnchecked,
                    l,
                    r,
                    ty,
                }) if matches!(table.int_info(*ty), Some((false, _)))
                    && *l == v
                    && matches!(
                        cx.def_kind(*r),
                        Some(OpKind::Const(ConstTree::Scalar { bits: 1, .. }))
                    ) => {}
                _ => {
                    all_ok = false;
                }
            }
        }
    }
    has_pos_init && all_ok
}

fn assert_len_matches_target(
    cx: &WitnessCheckCx<'_>,
    table: &TypeTable,
    arena_facts: Option<&ProgramBoundsFacts>,
    assert_len: ValueId,
    target_len: ValueId,
) -> bool {
    use crate::analysis::arena::ReturnVecLen;
    use crate::jir::semantic::StandardOperation;

    let func = cx.func;
    let same_val = |a: ValueId, b: ValueId| -> bool {
        let pa = peel_transparent(cx, table, a);
        let pb = peel_transparent(cx, table, b);
        if pa == pb {
            return true;
        }
        match (cx.def_kind(pa), cx.def_kind(pb)) {
            (
                Some(OpKind::Cast {
                    kind: CastKind::IntToInt,
                    v: va,
                    from: fa,
                    to: ta,
                }),
                Some(OpKind::Cast {
                    kind: CastKind::IntToInt,
                    v: vb,
                    from: fb,
                    to: tb,
                }),
            ) => {
                peel_transparent(cx, table, *va) == peel_transparent(cx, table, *vb)
                    && table.int_info(*fa) == table.int_info(*fb)
                    && table.int_info(*ta) == table.int_info(*tb)
            }
            _ => false,
        }
    };
    let unique_cell_store = |c: CellId| -> Option<ValueId> {
        let mut stores = func.blocks.iter().flat_map(|b| &b.ops).filter_map(|op| {
            if let OpKind::CellStore { cell, v } = op.kind {
                (cell == c).then_some(v)
            } else {
                None
            }
        });
        let init_v = stores.next()?;
        stores.next().is_none().then_some(init_v)
    };

    let mut cur = peel_transparent(cx, table, assert_len);
    for _ in 0..8 {
        if cur == target_len || same_one_shl_term(cx, table, cur, target_len) {
            return true;
        }
        match cx.def_kind(cur) {
            Some(OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v,
                ..
            }) => cur = peel_transparent(cx, table, *v),
            Some(OpKind::SliceView { len, .. }) => cur = peel_transparent(cx, table, *len),
            Some(OpKind::BorrowView {
                root: Root::Value(v),
                ..
            }) => cur = peel_transparent(cx, table, *v),
            Some(OpKind::BorrowView {
                root: Root::Cell(c),
                path,
                ..
            }) if path.is_empty() => {
                let Some(init_v) = unique_cell_store(*c) else {
                    return false;
                };
                cur = peel_transparent(cx, table, init_v);
            }
            Some(OpKind::CellLoad(c)) => {
                let Some(init_v) = unique_cell_store(*c) else {
                    return false;
                };
                cur = peel_transparent(cx, table, init_v);
            }
            Some(OpKind::Field { base, .. }) => cur = peel_transparent(cx, table, *base),
            Some(OpKind::Call {
                callee,
                semantic,
                args,
                ..
            }) => match semantic {
                CallSemantic::Standard(
                    StandardOperation::VecAsSlice
                    | StandardOperation::NativeVecAsSlice
                    | StandardOperation::VecAsMutSlice
                    | StandardOperation::NativeVecAsMutSlice
                    | StandardOperation::VecLen
                    | StandardOperation::VecClone
                    | StandardOperation::NativeVecFromElemZeroed,
                ) if !args.is_empty() => cur = peel_transparent(cx, table, args[0]),
                CallSemantic::Standard(StandardOperation::VecFromElem) if args.len() >= 2 => {
                    cur = peel_transparent(cx, table, args[1]);
                }
                CallSemantic::Ordinary => {
                    let Callee::Direct { item, .. } = callee else {
                        return false;
                    };
                    let Some(summary) = arena_facts.and_then(|f| f.return_vec_len.get(item)) else {
                        return false;
                    };
                    let resolved: &ReturnVecLen = match summary {
                        ReturnVecLen::CondBoolParam {
                            cond_param,
                            when_true,
                            when_false,
                        } => {
                            let Some(&av) = args.get(*cond_param) else {
                                return false;
                            };
                            match cx.def_kind(peel_transparent(cx, table, av)) {
                                Some(OpKind::Const(ConstTree::Scalar { bits: 1, .. })) => {
                                    when_true.as_ref()
                                }
                                Some(OpKind::Const(ConstTree::Scalar { bits: 0, .. })) => {
                                    when_false.as_ref()
                                }
                                _ => return false,
                            }
                        }
                        other => other,
                    };
                    match resolved {
                        ReturnVecLen::Param(k) | ReturnVecLen::ParamSliceLen(k) => {
                            let Some(&ak) = args.get(*k) else {
                                return false;
                            };
                            cur = peel_transparent(cx, table, ak);
                        }
                        ReturnVecLen::OneShlParam { shift_param, .. } => {
                            let Some(&ak) = args.get(*shift_param) else {
                                return false;
                            };
                            let shift_p = peel_transparent(cx, table, ak);
                            return match cx.def_kind(target_len) {
                                Some(OpKind::Binary {
                                    op: BinaryOp::Shl | BinaryOp::ShlUnchecked,
                                    l,
                                    r,
                                    ..
                                }) => {
                                    matches!(
                                        cx.def_kind(peel_transparent(cx, table, *l)),
                                        Some(OpKind::Const(ConstTree::Scalar { bits: 1, .. }))
                                    ) && same_val(*r, shift_p)
                                }
                                _ => false,
                            };
                        }
                        ReturnVecLen::Const(c) => {
                            return matches!(
                                cx.def_kind(target_len),
                                Some(OpKind::Const(ConstTree::Scalar { bits, bytes }))
                                    if mask(*bits, *bytes) == *c
                            );
                        }
                        ReturnVecLen::CondBoolParam { .. } => return false,
                    }
                }
                _ => return false,
            },
            _ => return false,
        }
    }
    cur == target_len || same_one_shl_term(cx, table, cur, target_len)
}

fn same_one_shl_term(cx: &WitnessCheckCx<'_>, table: &TypeTable, a: ValueId, b: ValueId) -> bool {
    let one_shl_arg = |v: ValueId| -> Option<ValueId> {
        let OpKind::Binary {
            op: BinaryOp::Shl | BinaryOp::ShlUnchecked,
            l,
            r,
            ..
        } = cx.def_kind(peel_transparent(cx, table, v))?
        else {
            return None;
        };
        matches!(
            cx.def_kind(peel_transparent(cx, table, *l)),
            Some(OpKind::Const(ConstTree::Scalar { bits: 1, .. }))
        )
        .then(|| peel_transparent(cx, table, *r))
    };
    matches!((one_shl_arg(a), one_shl_arg(b)), (Some(ka), Some(kb)) if ka == kb)
}

/// Independently checks the modular-arithmetic identity the range pass folds.
fn check_aligned_prefix(
    cx: &WitnessCheckCx<'_>,
    table: &TypeTable,
    left: ValueId,
    right: ValueId,
    op: BinaryOp,
    result: bool,
) -> Result<(), WitnessError> {
    fn copy_root(cx: &WitnessCheckCx<'_>, mut value: ValueId) -> ValueId {
        for _ in 0..8 {
            match cx.def_kind(value) {
                Some(OpKind::Copy(source) | OpKind::Move(source)) => value = *source,
                _ => break,
            }
        }
        value
    }
    fn same_term(cx: &WitnessCheckCx<'_>, left: ValueId, right: ValueId) -> bool {
        let (left, right) = (copy_root(cx, left), copy_root(cx, right));
        left == right
            || matches!(
                (cx.def_kind(left), cx.def_kind(right)),
                (
                    Some(OpKind::Unary {
                        op: UnaryOp::PtrMetadata,
                        v: left,
                        ..
                    }),
                    Some(OpKind::Unary {
                        op: UnaryOp::PtrMetadata,
                        v: right,
                        ..
                    })
                ) if copy_root(cx, *left) == copy_root(cx, *right)
            )
    }
    fn zero(cx: &WitnessCheckCx<'_>, value: ValueId) -> bool {
        matches!(
            cx.def_kind(copy_root(cx, value)),
            Some(OpKind::Const(ConstTree::Scalar { bits: 0, .. }))
        )
    }
    fn prefix_base(cx: &WitnessCheckCx<'_>, table: &TypeTable, value: ValueId) -> Option<ValueId> {
        let OpKind::Binary {
            op: BinaryOp::Sub | BinaryOp::SubUnchecked,
            l: base,
            r: remainder,
            ty,
        } = cx.def_kind(copy_root(cx, value))?
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
        } = cx.def_kind(copy_root(cx, *remainder))?
        else {
            return None;
        };
        if remainder_ty != ty || !same_term(cx, *base, *dividend) {
            return None;
        }
        match cx.def_kind(copy_root(cx, *divisor)) {
            Some(OpKind::Const(ConstTree::Scalar { bits, bytes })) if mask(*bits, *bytes) != 0 => {
                Some(*base)
            }
            _ => None,
        }
    }

    let expected = if let Some(base) = prefix_base(cx, table, left) {
        if same_term(cx, right, base) {
            match op {
                BinaryOp::Le => Some(true),
                BinaryOp::Gt => Some(false),
                _ => None,
            }
        } else if zero(cx, right) {
            match op {
                BinaryOp::Lt => Some(false),
                BinaryOp::Ge => Some(true),
                _ => None,
            }
        } else {
            None
        }
    } else if let Some(base) = prefix_base(cx, table, right) {
        if same_term(cx, left, base) {
            match op {
                BinaryOp::Ge => Some(true),
                BinaryOp::Lt => Some(false),
                _ => None,
            }
        } else if zero(cx, left) {
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
        other => Err(WitnessError::AlignedPrefixMismatch { expected: other }),
    }
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
    env_witness_fault().map(str::to_owned)
}

/// Independent validator for `prune_invoke`: re-derives every pruned `Invoke` claim against
/// pre-prune `pre_funcs`, `table`, and `live`.
pub(crate) fn check_invoke_pruning(
    pre_func: &Func,
    interns: &Interns,
    sites: &[InvokePrunedSite],
    table: &BTreeMap<JsName, LinkItem>,
    pre_funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    live: &BTreeSet<JsName>,
) -> Result<(), WitnessError> {
    if let Some(fault) = current_witness_fault() {
        if fault
            .split(|c: char| c == ',' || c.is_whitespace())
            .any(|tok| matches!(tok, "prune-invoke" | "all" | "1"))
        {
            return Err(WitnessError::InvokePruneFaultInjected);
        }
    }
    if sites.is_empty() {
        return Err(WitnessError::InvokePruneNoSites);
    }

    let mut cx = InvokeCheckCx::new(table, pre_funcs, live);
    for site in sites {
        let block = pre_func
            .blocks
            .get(site.block.0 as usize)
            .ok_or(WitnessError::InvokePruneMissingBlock(site.block))?;
        let RTerm::Invoke { unwind, .. } = &block.term else {
            return Err(WitnessError::InvokePruneNotInvoke(site.block));
        };
        match site.reason {
            InvokePruneReason::NoopUnwindChain => {
                check_noop_unwind_chain(pre_func, interns, unwind.0, |link_name| {
                    verify_empty_return_callee(interns, link_name, pre_funcs)
                })
                .map_err(WitnessError::InvokePruneNoopChain)?;
            }
            InvokePruneReason::ProtectedOpCannotThrow => {
                verify_protected_block_cannot_throw(pre_func, block, interns, &mut cx)
                    .map_err(WitnessError::InvokePruneMayThrow)?;
            }
        }
    }
    Ok(())
}

fn verify_empty_return_callee(
    interns: &Interns,
    link_name: StrId,
    pre_funcs: &BTreeMap<JsName, Result<Func, Decline>>,
) -> Result<(), String> {
    let name = interns
        .str(link_name)
        .ok_or_else(|| format!("unresolvable str#{} in unwind noop call", link_name.0))?;
    let callee = pre_funcs
        .get(name)
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
                item.allocator_provider,
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
                && func
                    .sig
                    .arguments
                    .iter()
                    .any(|&arg_ty| type_contains_fn_ptr(arg_ty, types, &mut BTreeSet::new()))
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
                            .is_some_and(|s| pre_funcs.get(s).is_some_and(Result::is_ok));
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
                            if type_contains_fn_ptr(vdef.ty, types, &mut BTreeSet::new()) {
                                return Err(format!(
                                    "unbounded indirect call: external/host op in `{name}` returns a fn ptr"
                                ));
                            }
                        }
                    }
                    if args.iter().any(|arg| {
                        func.values.get(arg.0 as usize).is_some_and(|vdef| {
                            type_contains_fn_ptr(vdef.ty, types, &mut BTreeSet::new())
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
    for op in &block.ops {
        check_op_no_throw(pre_func, op, interns, cx, &mut visited_callees, &mut queue)?;
    }
    drain_callee_queue(cx, &mut visited_callees, &mut queue)
}

fn drain_callee_queue(
    cx: &mut InvokeCheckCx<'_>,
    visited_callees: &mut BTreeSet<JsName>,
    queue: &mut VecDeque<JsName>,
) -> Result<(), String> {
    while let Some(callee_name) = queue.pop_front() {
        if is_panic_impl_item(cx.table, &callee_name) {
            continue;
        }
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
                        if let Some(protected_idx) = cb.protected_op_index() {
                            for op in &cb.ops[..protected_idx] {
                                check_op_no_throw(
                                    callee_func,
                                    op,
                                    callee_interns,
                                    cx,
                                    visited_callees,
                                    queue,
                                )?;
                            }
                            if let Some(protected) = cb.protected_op() {
                                if op_no_throw_isolated(
                                    callee_func,
                                    protected,
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

fn enqueue_callee_str(name: &str, visited: &mut BTreeSet<JsName>, queue: &mut VecDeque<JsName>) {
    if !visited.contains(name) {
        let owned = JsName::new(name.to_owned());
        visited.insert(owned.clone());
        queue.push_back(owned);
    }
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
            if crate::lower::js::host::is_marker_name(target) {
                return Err(format!(
                    "const_eval_select target `{target}` is a host marker"
                ));
            }
            enqueue_callee_str(target, visited, queue);
            Ok(())
        }
        _ => Ok(()),
    }
}

fn verify_host_payload_no_throw(payload: &HostPayload, interns: &Interns) -> Result<(), String> {
    if host_payload_cannot_throw(payload, interns) {
        Ok(())
    } else {
        Err("host payload may throw".to_owned())
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
    let resolve = |id: StrId, label: &str| -> Result<&str, String> {
        interns
            .str(id)
            .ok_or_else(|| format!("unresolvable {label} str#{}", id.0))
    };
    match &op.kind {
        OpKind::Host { payload, .. } => verify_host_payload_no_throw(payload, interns),
        OpKind::Intrinsic {
            fallback: Some(d), ..
        } => {
            enqueue_callee_str(resolve(d.link_name, "intrinsic fallback")?, visited, queue);
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
                enqueue_callee_str(resolve(*link_name, "direct drop")?, visited, queue);
                Ok(())
            }
            DropGlue::Dynamic => {
                for name in cx.vtables()?.drops.clone() {
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
            if matches!(semantic, CallSemantic::Host | CallSemantic::RawJavaScript) {
                return Err(format!("call semantic {semantic:?} may throw"));
            }
            match callee {
                Callee::Nop => Ok(()),
                Callee::Direct { link_name, .. } => {
                    let s = interns
                        .str(*link_name)
                        .ok_or_else(|| format!("unresolvable direct call str#{}", link_name.0))?;
                    if crate::lower::js::host::is_marker_name(s) {
                        return Err(format!("marker-named direct call `{s}` may throw"));
                    }
                    enqueue_callee_str(s, visited, queue);
                    Ok(())
                }
                Callee::Intrinsic {
                    fallback: Some(d), ..
                } => {
                    enqueue_callee_str(resolve(d.link_name, "intrinsic fallback")?, visited, queue);
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
                    if !is_non_throwing_sysroot_shim_member(m) {
                        return Err(format!("sysroot runtime member `{m}` may throw"));
                    }
                    Ok(())
                }
                Callee::Foreign { symbol, .. } => {
                    let sym = interns
                        .str(*symbol)
                        .ok_or_else(|| format!("unresolvable foreign symbol str#{}", symbol.0))?;
                    if !cx.table.contains_key(sym) {
                        return Err(format!("unresolved foreign symbol `{sym}` may throw"));
                    }
                    enqueue_callee_str(sym, visited, queue);
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

#[cfg(test)]
#[allow(
    clippy::too_many_lines,
    reason = "unit tests build multi-block witness fixtures inline"
)]
mod tests {
    use super::*;
    use crate::jir::repr::FuncBuilder;
    use crate::jir::semantic::{AbiSignature, Signature};
    use crate::jir::types::{
        IntFacts, IntRepr, JsFacts, LayoutFacts, TypeDescriptor, TypeKind, TypeMeta,
    };
    use crate::jir::OriginId;

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
                abi: AbiSignature::default(),
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

    #[test]
    fn const_fold_reevaluates_signed_comparisons() {
        let (func, table) = (empty(), table());
        let fold = |result| Witness::ConstFold {
            op: BinaryOp::Lt,
            ty: I32,
            l: ScalarBits {
                bits: u32::MAX as u128,
                bytes: 4,
            },
            r: ScalarBits { bits: 4, bytes: 4 },
            result,
        };
        assert!(fold(true).check(&func, &table).is_ok());
        assert!(fold(false).check(&func, &table).is_err());
    }

    #[test]
    fn branch_fold_reselects_the_case() {
        let (func, table) = (empty(), table());
        let fold = |taken| Witness::BranchFold {
            scrut: 0x1_0000_0001,
            bytes: 4,
            cases: vec![(1, RBlockId(7)), (2, RBlockId(8))],
            default: RBlockId(9),
            taken: RBlockId(taken),
        };
        assert!(fold(7).check(&func, &table).is_ok());
        assert!(fold(9).check(&func, &table).is_err());
    }

    #[test]
    fn invoke_pruning_witness_accepts_branching_dag_and_rejects_unsound_dags() {
        let mut interns = Interns::new();
        let bool_ty = TypeId(2);
        let raw_i32_ty = TypeId(3);
        interns.intern_type(table().get(I32).unwrap().clone());
        interns.intern_type(TypeDescriptor {
            id: bool_ty,
            name: None,
            kind: TypeKind::Bool,
            layout: LayoutFacts {
                size: Some(1),
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
                peeled: bool_ty,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });
        interns.intern_type(TypeDescriptor {
            id: raw_i32_ty,
            name: None,
            kind: TypeKind::RawPointer {
                pointee: I32,
                mutable: true,
            },
            layout: LayoutFacts {
                size: Some(4),
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
                peeled: raw_i32_ty,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });

        let sig = Signature {
            abi: AbiSignature::default(),
            arguments: Vec::new(),
            result: I32,
        };
        let item = ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash: 7,
            },
            instantiation: 0,
        };
        let host = HostPayload::Marker {
            section: interns.intern_str("rcgjs.host.invoke"),
            def: item.definition,
            helpers: Vec::new(),
        };

        // 1. Valid branching DAG with diamond merge -> Throw(None).
        let mut b = FuncBuilder::new(item, sig.clone());
        let flag_cell = b.cell(bool_ty, false, OriginId(0));
        let zero = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            I32,
            OriginId(0),
        );
        let t_bool = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 1 }),
            bool_ty,
            OriginId(0),
        );
        b.effect(
            FuncBuilder::ENTRY,
            OpKind::CellAlloc(flag_cell),
            OriginId(0),
        );
        b.effect(
            FuncBuilder::ENTRY,
            OpKind::Host {
                payload: host.clone(),
                args: Vec::new(),
            },
            OriginId(0),
        );
        let normal = b.block();
        let u_head = b.block();
        let u_a = b.block();
        let u_b = b.block();
        let u_tail = b.block();
        b.terminate(
            FuncBuilder::ENTRY,
            RTerm::Invoke {
                normal: (normal, Vec::new()),
                unwind: (u_head, Vec::new()),
            },
        );
        b.terminate(normal, RTerm::Return(Some(zero)));
        let loaded = b.op(u_head, OpKind::CellLoad(flag_cell), bool_ty, OriginId(0));
        b.effect(
            u_head,
            OpKind::CellStore {
                cell: flag_cell,
                v: t_bool,
            },
            OriginId(0),
        );
        let eq = b.op(
            u_head,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: loaded,
                r: t_bool,
                ty: bool_ty,
            },
            bool_ty,
            OriginId(0),
        );
        b.terminate(
            u_head,
            RTerm::Switch {
                scrut: eq,
                scrut_ty: bool_ty,
                cases: vec![(1, u_a, Vec::new())],
                default: (u_b, Vec::new()),
            },
        );
        b.effect(
            u_a,
            OpKind::Drop {
                root: Root::Value(zero),
                path: Vec::new(),
                glue: DropGlue::None,
            },
            OriginId(0),
        );
        b.terminate(u_a, RTerm::Goto(u_tail, Vec::new()));
        b.terminate(u_b, RTerm::Goto(u_tail, Vec::new()));
        b.terminate(u_tail, RTerm::Throw(None));
        let valid_func = b.finish();

        let site = [InvokePrunedSite {
            block: FuncBuilder::ENTRY,
            reason: InvokePruneReason::NoopUnwindChain,
        }];
        let empty_table = BTreeMap::new();
        let empty_funcs = BTreeMap::new();
        let empty_live = BTreeSet::new();
        check_invoke_pruning(
            &valid_func,
            &interns,
            &site,
            &empty_table,
            &empty_funcs,
            &empty_live,
        )
        .expect("witness must accept sound branching cleanup DAG");

        // 2. Cycle in cleanup DAG must be rejected.
        let mut cyc = valid_func;
        cyc.blocks[u_a.0 as usize].term = RTerm::Goto(u_head, Vec::new());
        let err = check_invoke_pruning(
            &cyc,
            &interns,
            &site,
            &empty_table,
            &empty_funcs,
            &empty_live,
        )
        .expect_err("witness must reject cycle in cleanup DAG");
        assert!(err.to_string().contains("cycle"), "unexpected error: {err}");

        // 3. CellStore to an escaped cell (via RawView) must be rejected.
        let mut esc = FuncBuilder::new(item, sig.clone());
        let esc_cell = esc.cell(I32, true, OriginId(0));
        let val = esc.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 7, bytes: 4 }),
            I32,
            OriginId(0),
        );
        esc.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(esc_cell), OriginId(0));
        let _raw = esc.op(
            FuncBuilder::ENTRY,
            OpKind::RawView {
                root: Root::Cell(esc_cell),
                path: Vec::new(),
                mutable: true,
            },
            raw_i32_ty,
            OriginId(0),
        );
        esc.effect(
            FuncBuilder::ENTRY,
            OpKind::Host {
                payload: host.clone(),
                args: Vec::new(),
            },
            OriginId(0),
        );
        let en = esc.block();
        let eu = esc.block();
        esc.terminate(
            FuncBuilder::ENTRY,
            RTerm::Invoke {
                normal: (en, Vec::new()),
                unwind: (eu, Vec::new()),
            },
        );
        esc.terminate(en, RTerm::Return(Some(val)));
        esc.effect(
            eu,
            OpKind::CellStore {
                cell: esc_cell,
                v: val,
            },
            OriginId(0),
        );
        esc.terminate(eu, RTerm::Throw(None));
        let esc_func = esc.finish();
        let err = check_invoke_pruning(
            &esc_func,
            &interns,
            &site,
            &empty_table,
            &empty_funcs,
            &empty_live,
        )
        .expect_err("witness must reject escaped cell store");
        assert!(
            err.to_string().contains("escaped cell"),
            "unexpected error: {err}"
        );

        // 4. Exceeding 64-block bound (65 blocks) must be rejected.
        let mut big = FuncBuilder::new(item, sig);
        let z = big.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            I32,
            OriginId(0),
        );
        big.effect(
            FuncBuilder::ENTRY,
            OpKind::Host {
                payload: host,
                args: Vec::new(),
            },
            OriginId(0),
        );
        let bn = big.block();
        let chain: Vec<RBlockId> = (0..65).map(|_| big.block()).collect();
        big.terminate(
            FuncBuilder::ENTRY,
            RTerm::Invoke {
                normal: (bn, Vec::new()),
                unwind: (chain[0], Vec::new()),
            },
        );
        big.terminate(bn, RTerm::Return(Some(z)));
        for w in chain.windows(2) {
            big.terminate(w[0], RTerm::Goto(w[1], Vec::new()));
        }
        big.terminate(chain[64], RTerm::Throw(None));
        let big_func = big.finish();
        let err = check_invoke_pruning(
            &big_func,
            &interns,
            &site,
            &empty_table,
            &empty_funcs,
            &empty_live,
        )
        .expect_err("witness must reject 65-block cleanup DAG");
        assert!(
            err.to_string().contains("64-block"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn witness_sysroot_allowlist_covers_monomorphic_nv_helpers() {
        for sym in [
            "nv_grow_u8",
            "nv_grow_i8",
            "nv_grow_u16",
            "nv_grow_i16",
            "nv_grow_u32",
            "nv_grow_i32",
            "nv_grow_f32",
            "nv_grow_f64",
            "nv_with_cap_u8",
            "nv_with_cap_i8",
            "nv_with_cap_u16",
            "nv_with_cap_i16",
            "nv_with_cap_u32",
            "nv_with_cap_i32",
            "nv_with_cap_f32",
            "nv_with_cap_f64",
            "nv_zeroed_u8",
            "nv_zeroed_i8",
            "nv_zeroed_u16",
            "nv_zeroed_i16",
            "nv_zeroed_u32",
            "nv_zeroed_i32",
            "nv_zeroed_f32",
            "nv_zeroed_f64",
            "nv_resize_zero_ta",
        ] {
            assert!(
                is_non_throwing_sysroot_shim_member(sym),
                "sysroot allowlist missing `{sym}`"
            );
        }
    }

    #[test]
    fn bitmask_monotone_arena_and_local_bounded_vec_witnesses_fail_closed() {
        use crate::analysis::arena::{ArenaFieldKind, ArenaInvariant, ProgramBoundsFacts};

        let mut tbl = TypeTable::new();
        const U32: TypeId = TypeId(100);
        tbl.insert(TypeDescriptor {
            id: U32,
            name: None,
            kind: TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            layout: LayoutFacts {
                size: Some(4),
                align: Some(4),
                sized: true,
                zst: false,
                needs_drop: false,
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
                peeled: U32,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });

        let item = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 55,
            },
            instantiation: 0,
        };
        let mut b = FuncBuilder::new(
            item,
            Signature {
                abi: AbiSignature::default(),
                arguments: vec![U32, U32],
                result: U32,
            },
        );
        let x = b.param(U32, OriginId(0));
        let len = b.param(U32, OriginId(0));
        let one = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            U32,
            OriginId(0),
        );
        let mask = b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: BinaryOp::Sub,
                l: len,
                r: one,
                ty: U32,
            },
            U32,
            OriginId(0),
        );
        let masked = b.op(
            FuncBuilder::ENTRY,
            OpKind::Binary {
                op: BinaryOp::BitAnd,
                l: x,
                r: mask,
                ty: U32,
            },
            U32,
            OriginId(0),
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(masked)));
        let func = b.finish();

        // 1. Bitmask with possibly-zero `len` and `nonzero_site: None` MUST fail closed.
        let bad_bitmask = [Witness::BoundsRemoved {
            block: FuncBuilder::ENTRY,
            index: masked,
            length: len,
            fact: BoundsFact::Bitmask {
                and_val: masked,
                mask_val: mask,
                len_val: len,
                nonzero_site: None,
            },
        }];
        assert!(check_all_with_context(&bad_bitmask, &func, &tbl, None).is_err());

        // 2. MonotoneArena without whole-program facts MUST fail closed.
        let node_def = DefinitionId {
            crate_id: 7,
            local_hash: 99,
        };
        let arena_w = [Witness::BoundsRemoved {
            block: FuncBuilder::ENTRY,
            index: x,
            length: len,
            fact: BoundsFact::MonotoneArena {
                elem_def: node_def,
                sentinel: None,
                source: ArenaIndexSource::Field(0),
            },
        }];
        assert!(check_all_with_context(&arena_w, &func, &tbl, None).is_err());

        // 3. MonotoneArena ValidOrSentinel without sentinel/guard or with bogus guard MUST fail closed.
        let mut facts = ProgramBoundsFacts::default();
        let mut fields = BTreeMap::new();
        fields.insert(
            0,
            ArenaFieldKind::ValidOrSentinel {
                sentinel: u32::MAX as u128,
            },
        );
        facts.arenas.insert(
            node_def,
            ArenaInvariant {
                elem_def: node_def,
                fields,
                holder_fields: BTreeSet::new(),
            },
        );
        assert!(check_all_with_context(&arena_w, &func, &tbl, Some(&facts)).is_err());
        let arena_bogus_guard = [Witness::BoundsRemoved {
            block: FuncBuilder::ENTRY,
            index: x,
            length: len,
            fact: BoundsFact::MonotoneArena {
                elem_def: node_def,
                sentinel: Some(SentinelGuard {
                    value: u32::MAX as u128,
                    block: FuncBuilder::ENTRY,
                    cond: x,
                }),
                source: ArenaIndexSource::Field(0),
            },
        }];
        assert!(check_all_with_context(&arena_bogus_guard, &func, &tbl, Some(&facts)).is_err());

        // 4. LocalBoundedVec with non-Vec root / bogus guard MUST fail closed.
        let bad_local_vec = [Witness::BoundsRemoved {
            block: FuncBuilder::ENTRY,
            index: x,
            length: len,
            fact: BoundsFact::LocalBoundedVec {
                vec_root: LocalVecRoot::Value(x),
                load_val: x,
                sub_offset: SubOffset::Guarded {
                    offset: NonZeroU128::new(1).unwrap(),
                    block: FuncBuilder::ENTRY,
                    cond: x,
                },
            },
        }];
        assert!(check_all_with_context(&bad_local_vec, &func, &tbl, None).is_err());
    }
}
