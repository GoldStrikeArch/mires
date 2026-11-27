//! The semantic-operation registry for surviving whole-type native lowerings (`BoxDrop` and
//! `BTreeMap*`) plus compact standard-operation effect summaries for `analysis/effects.rs` and
//! `analysis/callgraph.rs`.

#![allow(
    dead_code,
    reason = "semantic-op adapters expose helpers used selectively across lowering and tests"
)]

pub mod drop;
pub mod fmt_kernel;
pub mod map;
pub mod num;

use std::collections::BTreeSet;

use crate::item::{JsName, LinkItem};
use crate::jir::interns::Interns;
use crate::jir::repr::Carrier;
use crate::jir::semantic::{BodySemantic, Signature, StandardOperation};
use crate::jir::types::{
    AllocatorIdentity, RepresentationCandidate, TypeDescriptor, TypeId, TypeKind, TypeTable,
};
use crate::jsast::Stmt;

/// Everything an operation's validation and emitter read about the body it replaces.
pub struct LowerCx<'a> {
    pub semantic: BodySemantic,
    pub sig: &'a Signature,
    pub table: &'a TypeTable,
    pub interns: &'a Interns,
    /// The exact monomorphic child drop glue item for a boxed pointee. `None` when the child does
    /// not need drop, and in callers that have not resolved glue.
    pub element_glue: Option<&'a JsName>,
    /// Whether universal native `{ s, cap }` `String` representation
    /// (`HeapPlan::native_string_enabled`) is active for this program.
    pub native_string_enabled: bool,
}

/// One registry entry: the complete contract and lowering of one standard operation.
pub struct SemanticOp {
    pub id: StandardOperation,
    /// The JavaScript ABI parameter count the body must have.
    pub parameters: u16,
    /// The signature slots that must carry a native representation, with the carriers each allows.
    pub slots: &'static [SlotRequirement],
    /// The carrier family the operation participates in.
    pub carriers: &'static [Carrier],
    pub effects: Effects,
    /// The operation-specific half of validation: argument and return types beyond the slots.
    pub legality: fn(&LowerCx<'_>) -> Result<(), SemanticDecline>,
    pub lower: LowerFn,
    /// Every `__rt.*` member a lowering of this op may reach.
    pub helpers: &'static [&'static str],
    pub deps: Deps,
}

/// The emitter: statements of the replacement body, named `$native0..N-1`.
pub type LowerFn = fn(
    &LowerCx<'_>,
    Option<Carrier>,
    &mut BTreeSet<&'static str>,
    &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline>;

/// What an operation does, as `analysis/effects.rs` consumes it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Effects {
    pub can_throw: bool,
    pub mutates: bool,
}

/// Linker facts a lowering needs beyond the shim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Deps {
    None,
    /// The exact monomorphic drop glue for a native box's pointee.
    BoxPointeeDropGlue,
}

/// One Rust signature slot whose value must have a native carrier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlotRequirement {
    pub slot: SignatureSlot,
    pub reference: ReferenceRequirement,
    pub carriers: &'static [Carrier],
}

impl SlotRequirement {
    const fn result(carriers: &'static [Carrier]) -> Self {
        Self {
            slot: SignatureSlot::Result,
            reference: ReferenceRequirement::Value,
            carriers,
        }
    }

    const fn argument(
        index: u16,
        reference: ReferenceRequirement,
        carriers: &'static [Carrier],
    ) -> Self {
        Self {
            slot: SignatureSlot::Argument(index),
            reference,
            carriers,
        }
    }
}

/// A return place or zero-based Rust argument.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignatureSlot {
    Result,
    Argument(u16),
}

/// Whether the carrier is read at the slot or below a reference projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceRequirement {
    Value,
    Shared,
    Mutable,
}

/// A deterministic reason an otherwise reachable standard operation keeps its baseline body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticDecline {
    NotAFunction,
    NotAStandardOperation,
    UnsupportedOperation(StandardOperation),
    SpreadAbi,
    CallerLocationAbi,
    ParameterCount { expected: u16, actual: u16 },
    SignatureArgumentCount { expected: u16, actual: u16 },
    MissingSlot(SignatureSlot),
    MissingType(TypeId),
    WrongReferenceKind(SignatureSlot),
    WrongNativeType(SignatureSlot),
    WrongArgumentType(u16),
    WrongCarrier(SignatureSlot),
    WrongReturnType,
    UnknownLayout,
    UnknownElementLayout,
    ZeroSizedElement,
    ElementDropRequired,
    ElementHasOwnDrop,
    UnsupportedPointerWidth(u32),
}

impl SemanticDecline {
    pub const fn reason(self) -> &'static str {
        match self {
            Self::NotAFunction => "the item is not a function",
            Self::NotAStandardOperation => "the body has no canonical standard operation",
            Self::UnsupportedOperation(_) => {
                "the canonical standard operation has no exact native body lowering"
            }
            Self::SpreadAbi => "the standard operation uses an unsupported spread ABI",
            Self::CallerLocationAbi => {
                "the standard operation uses an unsupported caller-location ABI"
            }
            Self::ParameterCount { .. } => {
                "the JavaScript ABI parameter count does not match the canonical operation"
            }
            Self::SignatureArgumentCount { .. } => {
                "the Rust signature argument count does not match the canonical operation"
            }
            Self::MissingSlot(_) => "the signature has no slot where the operation requires one",
            Self::MissingType(_) => "the signature references a missing type descriptor",
            Self::WrongReferenceKind(_) => {
                "the standard operation receiver has the wrong reference mutability"
            }
            Self::WrongNativeType(_) => {
                "the standard operation signature slot has the wrong native Rust type"
            }
            Self::WrongArgumentType(_) => {
                "the standard operation argument has an unexpected Rust type"
            }
            Self::WrongCarrier(_) => {
                "the standard operation signature slot did not select a supported carrier"
            }
            Self::WrongReturnType => "the standard operation has an unexpected Rust return type",
            Self::UnknownLayout => "a layout the native lowering depends on is unavailable",
            Self::UnknownElementLayout => "the native element layout is unavailable",
            Self::ZeroSizedElement => "the native operation does not support zero-sized elements",
            Self::ElementDropRequired => {
                "the native operation requires executable element drop glue"
            }
            Self::ElementHasOwnDrop => {
                "the native element has a custom destructor and stays on baseline"
            }
            Self::UnsupportedPointerWidth(_) => {
                "the native lowering requires a JavaScript-number pointer width"
            }
        }
    }
}

/// A complete replacement body and the names it reaches.
#[derive(Clone, Debug, PartialEq)]
pub struct Rewrite {
    pub operation: StandardOperation,
    pub parameters: Vec<String>,
    pub statements: Vec<Stmt>,
    pub runtime_helpers: BTreeSet<&'static str>,
    pub item_references: BTreeSet<JsName>,
}

const NATIVE_BOX: &[Carrier] = &[Carrier::NativeBox];
const NATIVE_MAP: &[Carrier] = &[Carrier::NativeMap];
const RESULT_MAP: &[SlotRequirement] = &[SlotRequirement::result(NATIVE_MAP)];
const RECEIVER_MAP: &[SlotRequirement] = &[SlotRequirement::argument(
    0,
    ReferenceRequirement::Shared,
    NATIVE_MAP,
)];
const MUT_RECEIVER_MAP: &[SlotRequirement] = &[SlotRequirement::argument(
    0,
    ReferenceRequirement::Mutable,
    NATIVE_MAP,
)];
const VALUE_RECEIVER_MAP: &[SlotRequirement] = &[SlotRequirement::argument(
    0,
    ReferenceRequirement::Value,
    NATIVE_MAP,
)];
const MUT_RECEIVER_BOX: &[SlotRequirement] = &[SlotRequirement::argument(
    0,
    ReferenceRequirement::Mutable,
    NATIVE_BOX,
)];
const VALUE_RECEIVER_BOX: &[SlotRequirement] = &[SlotRequirement::argument(
    0,
    ReferenceRequirement::Value,
    NATIVE_BOX,
)];

const PURE: Effects = Effects {
    can_throw: false,
    mutates: false,
};
const THROWS: Effects = Effects {
    can_throw: true,
    mutates: false,
};
const MUTATES: Effects = Effects {
    can_throw: false,
    mutates: true,
};
const THROWS_AND_MUTATES: Effects = Effects {
    can_throw: true,
    mutates: true,
};

macro_rules! op {
    ($name:ident, $id:ident, $parameters:expr, $slots:expr, $carriers:expr, $effects:expr,
     $legality:path, $lower:path, $helpers:expr) => {
        op!(
            $name,
            $id,
            $parameters,
            $slots,
            $carriers,
            $effects,
            $legality,
            $lower,
            $helpers,
            Deps::None
        );
    };
    ($name:ident, $id:ident, $parameters:expr, $slots:expr, $carriers:expr, $effects:expr,
     $legality:path, $lower:path, $helpers:expr, $deps:expr) => {
        pub(super) static $name: SemanticOp = SemanticOp {
            id: StandardOperation::$id,
            parameters: $parameters,
            slots: $slots,
            carriers: $carriers,
            effects: $effects,
            legality: $legality,
            lower: $lower,
            helpers: $helpers,
            deps: $deps,
        };
    };
}

#[rustfmt::skip]
mod ops {
    use super::*;

    op!(BOX_DROP, BoxDrop, 1, MUT_RECEIVER_BOX, NATIVE_BOX, MUTATES,
        drop::legality_box_drop, drop::box_drop, &[], Deps::BoxPointeeDropGlue);
    op!(BTREE_MAP_NEW, BTreeMapNew, 0, RESULT_MAP, NATIVE_MAP, PURE,
        map::legality_new, map::btree_map_new, &[]);
    op!(BTREE_MAP_LEN, BTreeMapLen, 1, RECEIVER_MAP, NATIVE_MAP, PURE,
        map::legality_len, map::btree_map_len, &[]);
    op!(BTREE_MAP_IS_EMPTY, BTreeMapIsEmpty, 1, RECEIVER_MAP, NATIVE_MAP, PURE,
        map::legality_is_empty, map::btree_map_is_empty, &[]);
    op!(BTREE_MAP_CLEAR, BTreeMapClear, 1, MUT_RECEIVER_MAP, NATIVE_MAP, MUTATES,
        map::legality_unit, map::btree_map_clear, &[]);
    op!(BTREE_MAP_CONTAINS_KEY, BTreeMapContainsKey, 2, RECEIVER_MAP, NATIVE_MAP, PURE,
        map::legality_contains_key, map::btree_map_contains_key, &[]);
    op!(BTREE_MAP_GET, BTreeMapGet, 2, RECEIVER_MAP, NATIVE_MAP, PURE,
        map::legality_get, map::btree_map_get, &[]);
    op!(BTREE_MAP_GET_MUT, BTreeMapGetMut, 2, MUT_RECEIVER_MAP, NATIVE_MAP, MUTATES,
        map::legality_get_mut, map::btree_map_get, &[]);
    op!(BTREE_MAP_INSERT, BTreeMapInsert, 3, MUT_RECEIVER_MAP, NATIVE_MAP, MUTATES,
        map::legality_insert, map::btree_map_insert, &[]);
    op!(BTREE_MAP_REMOVE, BTreeMapRemove, 2, MUT_RECEIVER_MAP, NATIVE_MAP, MUTATES,
        map::legality_remove, map::btree_map_remove, &[]);
    op!(BTREE_MAP_DROP, BTreeMapDrop, 1, MUT_RECEIVER_MAP, NATIVE_MAP, MUTATES,
        map::legality_unit, map::btree_map_clear, &[]);
}

/// The registry of whole-body standard operations (`BoxDrop` and `BTreeMap*`).
pub fn registry(operation: StandardOperation) -> Option<&'static SemanticOp> {
    use ops::*;
    Some(match operation {
        StandardOperation::BoxDrop => &BOX_DROP,
        StandardOperation::BTreeMapNew => &BTREE_MAP_NEW,
        StandardOperation::BTreeMapLen => &BTREE_MAP_LEN,
        StandardOperation::BTreeMapIsEmpty => &BTREE_MAP_IS_EMPTY,
        StandardOperation::BTreeMapClear => &BTREE_MAP_CLEAR,
        StandardOperation::BTreeMapContainsKey => &BTREE_MAP_CONTAINS_KEY,
        StandardOperation::BTreeMapGet => &BTREE_MAP_GET,
        StandardOperation::BTreeMapGetMut => &BTREE_MAP_GET_MUT,
        StandardOperation::BTreeMapInsert => &BTREE_MAP_INSERT,
        StandardOperation::BTreeMapRemove => &BTREE_MAP_REMOVE,
        StandardOperation::BTreeMapDrop => &BTREE_MAP_DROP,
        _ => return None,
    })
}

/// Effect contract for well-known standard library operations, used by `analysis/effects.rs`
/// and `analysis/callgraph.rs` so standard container/format calls do not require inspecting
/// allocator or formatting bodies.
fn standard_effects(operation: StandardOperation) -> Option<Effects> {
    Some(match operation {
        StandardOperation::StringNew
        | StandardOperation::StringLen
        | StandardOperation::StringCapacity
        | StandardOperation::StringAsStr
        | StandardOperation::F64SpecToString
        | StandardOperation::IntToString
        | StandardOperation::BoolToString
        | StandardOperation::CharToString
        | StandardOperation::StrToLowercase
        | StandardOperation::StrToUppercase
        | StandardOperation::FormatArgumentNewDisplay
        | StandardOperation::FormatArgumentNewLowerHex
        | StandardOperation::FormatArgumentNewUpperHex
        | StandardOperation::FormatArgsNew
        | StandardOperation::FormatArgsToString
        | StandardOperation::VecNew
        | StandardOperation::VecLen
        | StandardOperation::VecCapacity
        | StandardOperation::VecIsEmpty
        | StandardOperation::VecFromBoxArray
        | StandardOperation::VecAsSlice
        | StandardOperation::VecAsMutSlice
        | StandardOperation::VecIter
        | StandardOperation::VecIterMut
        | StandardOperation::VecClone
        | StandardOperation::SliceToVec
        | StandardOperation::BTreeMapNew
        | StandardOperation::BTreeMapLen
        | StandardOperation::BTreeMapIsEmpty
        | StandardOperation::BTreeMapContainsKey
        | StandardOperation::BTreeMapGet => PURE,

        StandardOperation::StringWithCapacity
        | StandardOperation::VecWithCapacity
        | StandardOperation::VecFromElem
        | StandardOperation::VecIndex
        | StandardOperation::VecIndexMut
        | StandardOperation::VecRangeIndex
        | StandardOperation::VecRangeIndexMut => THROWS,

        StandardOperation::StringClear
        | StandardOperation::StringDrop
        | StandardOperation::VecShrinkToFit
        | StandardOperation::VecTruncate
        | StandardOperation::VecPop
        | StandardOperation::VecClear
        | StandardOperation::VecReplace
        | StandardOperation::VecMemSwap
        | StandardOperation::VecDrop
        | StandardOperation::BoxDrop
        | StandardOperation::BTreeMapClear
        | StandardOperation::BTreeMapGetMut
        | StandardOperation::BTreeMapInsert
        | StandardOperation::BTreeMapRemove
        | StandardOperation::BTreeMapDrop => MUTATES,

        StandardOperation::StringPush
        | StandardOperation::StringPushStr
        | StandardOperation::StringFmtWriteStr
        | StandardOperation::StringFmtWriteChar
        | StandardOperation::VecPush
        | StandardOperation::VecPushMut
        | StandardOperation::VecInsert
        | StandardOperation::VecInsertMut
        | StandardOperation::VecReserve
        | StandardOperation::VecReserveExact
        | StandardOperation::VecResize
        | StandardOperation::VecRemove
        | StandardOperation::VecSwapRemove
        | StandardOperation::VecExtend
        | StandardOperation::VecExtendFromSlice
        | StandardOperation::SliceSwap => THROWS_AND_MUTATES,

        _ => return None,
    })
}

/// Returns the sound effect summary for a standard call site, guarding `Clone`-invoking `Vec`
/// operations so only primitive elements with trivial pure `Clone` bypass body inspection.
pub fn call_effects_entry(
    operation: StandardOperation,
    types: Option<&TypeTable>,
    func: &crate::jir::repr::Func,
    result: Option<crate::jir::repr::ValueId>,
    args: &[crate::jir::repr::ValueId],
) -> Option<Effects> {
    let effects = standard_effects(operation)?;
    if !matches!(
        operation,
        StandardOperation::VecClone
            | StandardOperation::SliceToVec
            | StandardOperation::VecFromElem
            | StandardOperation::VecResize
            | StandardOperation::VecExtendFromSlice
    ) {
        return Some(effects);
    }
    let types = types?;
    let vec_val = match operation {
        StandardOperation::SliceToVec | StandardOperation::VecFromElem => result?,
        StandardOperation::VecClone
        | StandardOperation::VecResize
        | StandardOperation::VecExtendFromSlice => *args.first()?,
        _ => unreachable!(),
    };
    let mut vec_ty = types.peeled(func.ty(vec_val)?);
    if let Some(TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. }) =
        types.get(vec_ty).map(|descriptor| &descriptor.kind)
    {
        vec_ty = types.peeled(*pointee);
    }
    let vec_desc = types.get(vec_ty)?;
    if vec_desc.meta.candidate != RepresentationCandidate::Vec
        || vec_desc.meta.allocator != AllocatorIdentity::Global
    {
        return None;
    }
    let TypeKind::Adt { args: ty_args, .. } = &vec_desc.kind else {
        return None;
    };
    let elem_ty = *ty_args.first()?;
    let elem_desc = types.get(types.peeled(elem_ty))?;
    is_trivial_clone_primitive(elem_desc).then_some(effects)
}

/// ES2018 TypedArray constructor kind tag (`__rt._tv_ctors[k]`) for built-in primitive numeric
/// elements (`u8`, `i8`, `u16`, `i16`, `u32`, `i32`, `f32`, `f64`).
pub(crate) fn typed_array_kind(element: &TypeDescriptor) -> Option<u8> {
    match element.kind {
        TypeKind::Integer {
            signed: false,
            bits: 8,
        } => Some(0),
        TypeKind::Integer {
            signed: true,
            bits: 8,
        } => Some(1),
        TypeKind::Integer {
            signed: false,
            bits: 16,
        } => Some(2),
        TypeKind::Integer {
            signed: true,
            bits: 16,
        } => Some(3),
        TypeKind::Integer {
            signed: false,
            bits: 32,
        } => Some(4),
        TypeKind::Integer {
            signed: true,
            bits: 32,
        } => Some(5),
        TypeKind::Float { bits: 32 } => Some(6),
        TypeKind::Float { bits: 64 } => Some(7),
        _ => None,
    }
}

/// Built-in primitive types whose `Clone` impl is guaranteed pure, non-panicking, and drop-free.
pub(crate) fn is_trivial_clone_primitive(element: &TypeDescriptor) -> bool {
    matches!(
        element.kind,
        TypeKind::Bool | TypeKind::Char | TypeKind::Integer { .. } | TypeKind::Float { .. }
    ) && !element.layout.needs_drop
        && !element.layout.zst
        && element.layout.sized
}

/// Returns `(crate_name, item_path)` for an item's JIR origin metadata.
pub(crate) fn item_origin_info<'a>(item: &'a LinkItem, interns: &'a Interns) -> (&'a str, &'a str) {
    let path = item
        .jir
        .origins
        .as_ref()
        .and_then(|o| interns.str(o.item_path))
        .unwrap_or("");
    (item.crate_name.as_str(), path)
}

/// Validates that the signature returns `()`.
pub(crate) fn legality_unit_return(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let result = type_get(cx.table, cx.sig.result)?;
    if !matches!(&result.kind, TypeKind::Tuple(fields) if fields.is_empty()) || !result.layout.zst {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

/// The registry entry of a body, through its semantic identity.
pub fn support_of(
    cx: &LowerCx<'_>,
) -> Result<(&'static SemanticOp, &'static [SlotRequirement]), SemanticDecline> {
    match cx.semantic {
        BodySemantic::Standard(operation) => {
            let op = registry(operation).ok_or(SemanticDecline::UnsupportedOperation(operation))?;
            Ok((op, op.slots))
        }
        BodySemantic::DropGlue(dropped) => {
            let argument = cx
                .sig
                .arguments
                .first()
                .copied()
                .ok_or(SemanticDecline::WrongArgumentType(0))?;
            let (value, by_value) = match type_get(cx.table, argument)?.kind {
                TypeKind::Reference { pointee, mutable } => {
                    if !mutable {
                        return Err(SemanticDecline::WrongReferenceKind(
                            SignatureSlot::Argument(0),
                        ));
                    }
                    (pointee, false)
                }
                _ => (argument, true),
            };
            if value != dropped {
                return Err(SemanticDecline::WrongArgumentType(0));
            }
            match type_get(cx.table, dropped)?.meta.candidate {
                RepresentationCandidate::Box => {
                    let op = registry(StandardOperation::BoxDrop)
                        .ok_or(SemanticDecline::NotAStandardOperation)?;
                    Ok((
                        op,
                        if by_value {
                            VALUE_RECEIVER_BOX
                        } else {
                            MUT_RECEIVER_BOX
                        },
                    ))
                }
                RepresentationCandidate::BTreeMap => {
                    let op = registry(StandardOperation::BTreeMapDrop)
                        .ok_or(SemanticDecline::NotAStandardOperation)?;
                    Ok((
                        op,
                        if by_value {
                            VALUE_RECEIVER_MAP
                        } else {
                            MUT_RECEIVER_MAP
                        },
                    ))
                }
                _ => Err(SemanticDecline::NotAStandardOperation),
            }
        }
        BodySemantic::Ordinary | BodySemantic::PanicImpl | BodySemantic::JsExport => {
            Err(SemanticDecline::NotAStandardOperation)
        }
    }
}

/// Validates every fact that is independent of the carrier choice.
pub fn validate(cx: &LowerCx<'_>) -> Result<&'static SemanticOp, SemanticDecline> {
    let (op, slots) = support_of(cx)?;
    validate_abi(cx, op)?;
    validate_slot_types(cx, slots)?;
    (op.legality)(cx)?;

    let proof_glue = JsName::new("$nativeElementDrop".to_owned());
    let cx = LowerCx {
        semantic: cx.semantic,
        sig: cx.sig,
        table: cx.table,
        interns: cx.interns,
        element_glue: cx.element_glue.or(Some(&proof_glue)),
        native_string_enabled: cx.native_string_enabled,
    };
    match slots.first() {
        None => {
            (op.lower)(&cx, None, &mut BTreeSet::new(), &mut BTreeSet::new())?;
        }
        Some(requirement) => {
            for carrier in requirement.carriers {
                (op.lower)(
                    &cx,
                    Some(*carrier),
                    &mut BTreeSet::new(),
                    &mut BTreeSet::new(),
                )?;
            }
        }
    }
    Ok(op)
}

/// Builds the exact native body at the selected carriers (one per slot, in slot order).
pub fn rewrite(cx: &LowerCx<'_>, selected: &[Carrier]) -> Result<Rewrite, SemanticDecline> {
    let (op, slots) = support_of(cx)?;
    validate_abi(cx, op)?;
    validate_slot_types(cx, slots)?;
    (op.legality)(cx)?;
    if selected.len() != slots.len() {
        return Err(SemanticDecline::WrongCarrier(
            slots
                .get(selected.len())
                .map_or(SignatureSlot::Result, |requirement| requirement.slot),
        ));
    }
    for (requirement, carrier) in slots.iter().zip(selected) {
        if !requirement.carriers.contains(carrier) {
            return Err(SemanticDecline::WrongCarrier(requirement.slot));
        }
    }

    let parameters = (0..op.parameters)
        .map(|index| format!("$native{index}"))
        .collect();
    let mut runtime_helpers = BTreeSet::new();
    let mut item_references = BTreeSet::new();
    let statements = (op.lower)(
        cx,
        selected.first().copied(),
        &mut runtime_helpers,
        &mut item_references,
    )?;
    Ok(Rewrite {
        operation: op.id,
        parameters,
        statements,
        runtime_helpers,
        item_references,
    })
}

fn validate_abi(cx: &LowerCx<'_>, op: &SemanticOp) -> Result<(), SemanticDecline> {
    let abi = cx.sig.abi;
    if abi.spread.is_some() {
        return Err(SemanticDecline::SpreadAbi);
    }
    if abi.caller_location {
        return Err(SemanticDecline::CallerLocationAbi);
    }
    if abi.parameters != op.parameters {
        return Err(SemanticDecline::ParameterCount {
            expected: op.parameters,
            actual: abi.parameters,
        });
    }
    let actual = u16::try_from(cx.sig.arguments.len()).unwrap_or(u16::MAX);
    if actual != op.parameters {
        return Err(SemanticDecline::SignatureArgumentCount {
            expected: op.parameters,
            actual,
        });
    }
    Ok(())
}

fn validate_slot_types(cx: &LowerCx<'_>, slots: &[SlotRequirement]) -> Result<(), SemanticDecline> {
    for requirement in slots {
        let mut ty = slot_type(cx.sig, requirement.slot)?;
        if requirement.reference != ReferenceRequirement::Value {
            let descriptor = type_get(cx.table, ty)?;
            let TypeKind::Reference { pointee, mutable } = descriptor.kind else {
                return Err(SemanticDecline::WrongReferenceKind(requirement.slot));
            };
            if mutable != (requirement.reference == ReferenceRequirement::Mutable) {
                return Err(SemanticDecline::WrongReferenceKind(requirement.slot));
            }
            ty = pointee;
        }
        require_native_type(cx.table, ty, requirement)?;
    }
    Ok(())
}

pub(crate) fn slot_type(sig: &Signature, slot: SignatureSlot) -> Result<TypeId, SemanticDecline> {
    match slot {
        SignatureSlot::Result => Ok(sig.result),
        SignatureSlot::Argument(index) => sig
            .arguments
            .get(usize::from(index))
            .copied()
            .ok_or(SemanticDecline::MissingSlot(slot)),
    }
}

pub(crate) fn type_get(table: &TypeTable, ty: TypeId) -> Result<&TypeDescriptor, SemanticDecline> {
    table.get(ty).ok_or(SemanticDecline::MissingType(ty))
}

fn require_native_type(
    table: &TypeTable,
    ty: TypeId,
    requirement: &SlotRequirement,
) -> Result<(), SemanticDecline> {
    let candidate = if requirement.carriers.contains(&Carrier::NativeBox) {
        RepresentationCandidate::Box
    } else if requirement.carriers.contains(&Carrier::NativeMap) {
        RepresentationCandidate::BTreeMap
    } else {
        return Err(SemanticDecline::WrongNativeType(requirement.slot));
    };
    let descriptor = type_get(table, ty)?;
    if descriptor.meta.candidate != candidate
        || descriptor.meta.allocator != AllocatorIdentity::Global
        || !descriptor.layout.sized
        || descriptor.layout.zst
    {
        return Err(SemanticDecline::WrongNativeType(requirement.slot));
    }
    Ok(())
}

pub(crate) fn unsigned_bits(descriptor: &TypeDescriptor) -> Result<u32, SemanticDecline> {
    match descriptor.kind {
        TypeKind::Integer {
            signed: false,
            bits,
        } => Ok(bits),
        _ => Err(SemanticDecline::WrongReturnType),
    }
}

/// Whether `ty` is `&str`.
pub(crate) fn is_shared_str(table: &TypeTable, ty: TypeId) -> bool {
    type_get(table, ty).is_ok_and(|descriptor| {
        matches!(
            descriptor.kind,
            TypeKind::Reference {
                pointee,
                mutable: false,
            } if type_get(table, pointee)
                .is_ok_and(|pointee| matches!(pointee.kind, TypeKind::Str))
        )
    })
}

#[cfg(test)]
mod tests;
