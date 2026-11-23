//! The semantic-operation registry (Phase 4, D7): one entry per producer-tagged
//! [`StandardOperation`] the jir lane can lower natively.
//!
//! There is **no matcher here**: the producer already established every identity
//! (`BodySemantic::Standard` on bodies, `CallSemantic::Standard` on calls) from rustc definitions,
//! so this module only answers "what does that operation need, and how is it spelled". One op =
//! one definition = one lowering. The slot tables and the emitter bodies are ports of the frozen
//! v1 text optimizer's `native_std_lower.rs` (same helper spellings, byte for byte, which is what
//! the `native-*-differential` receipts diff), re-keyed from `PlannedRepresentation` onto the
//! JIR-R [`Carrier`]s and from the late-IR type metadata onto the [`TypeTable`].
//!
//! The registry is data plus functions; nothing runs until the representation planner (W4b)
//! selects carriers and asks [`rewrite`] for a body. Until then the only caller is the test-only
//! `RCGJS_REPR_FORCE` harness in `lower/mod.rs` and the unit tests, which pin every emitter at
//! every legal carrier against the v1 spelling.
//!
//! Wave 1 covers the 18-op string/format cluster; wave 2 adds the 15-op vec/slice/drop cluster
//! (`vec.rs`, `slice.rs`, `drop.rs`) for the 33 operations v1 supported. The 5 operations v1
//! never supported (`VecPop`, `VecReserve`, `VecSwap`, `StringFmtWriteFmt`,
//! `StringFmtSpecWriteFmt`) stay exact [`SemanticDecline::UnsupportedOperation`] declines.

// The registry's data fields (`effects`, `carriers`, `cost_hint`, ...) and the proof gate are
// consumed by the representation planner (W4b), the legality analysis (W4a) and the cost model
// (W4e); until those land, the unit tests and the `RCGJS_REPR_FORCE` harness are the readers.
#![allow(dead_code)]

pub mod drop;
pub mod fmt;
pub mod fmt_kernel;
pub mod num;
pub mod slice;
pub mod string;
pub mod vec;

use std::collections::BTreeSet;

use crate::item::JsName;
use crate::jir::interns::Interns;
use crate::jir::repr::Carrier;
use crate::jir::semantic::{BodySemantic, Signature, StandardOperation};
use crate::jir::types::{
    AllocatorIdentity, RepresentationCandidate, TypeDescriptor, TypeId, TypeKind, TypeTable,
};
use crate::jsast::{self, BinOp, Expr, Stmt};

/// Everything an operation's validation and emitter read about the body it replaces.
///
/// Deliberately signature-level: a standard operation's body is replaced whole, so the CFG is
/// never consulted — the contract is the Rust signature, the ABI shape, and the type table.
pub struct LowerCx<'a> {
    pub semantic: BodySemantic,
    pub sig: &'a Signature,
    pub table: &'a TypeTable,
    pub interns: &'a Interns,
    /// The exact monomorphic child drop glue item for a vec element or boxed pointee. `None` when
    /// the child does not need drop, and in every caller that has not resolved glue.
    pub element_glue: Option<&'a JsName>,
    /// True only when `element_glue` is proven unable to unwind. Fail-closed (`false`) emits
    /// unwinding cleanup for remaining elements in `Vec::{clear,truncate,drop}`.
    pub element_glue_nonthrowing: bool,
    /// The program's `#[panic_handler]` (`BodySemantic::PanicImpl`), when resolved by the linker.
    pub panic_impl: Option<&'a JsName>,
}

/// One registry entry: the complete contract and lowering of one standard operation.
pub struct SemanticOp {
    pub id: StandardOperation,
    /// The JavaScript ABI parameter count the body must have.
    pub parameters: u16,
    /// The signature slots that must carry a native representation, with the carriers each allows.
    pub slots: &'static [SlotRequirement],
    /// The carrier family the operation participates in, deduplicated over its slots — what the
    /// solver's family staging (F3 strings, F2 vecs) classifies by.
    pub carriers: &'static [Carrier],
    pub effects: Effects,
    /// The operation-specific half of validation: argument and return types beyond the slots.
    pub legality: fn(&LowerCx<'_>) -> Result<(), SemanticDecline>,
    pub lower: LowerFn,
    /// Every `__rt.*` member a lowering of this op may reach, for the cost model's helper term.
    pub helpers: &'static [&'static str],
    /// Rough minified bytes of one lowered body, before measurement replaces the estimate.
    pub cost_hint: u16,
    pub deps: Deps,
    pub fixtures: &'static [&'static str],
}

/// The emitter: statements of the replacement body, named `$native0..N-1`.
pub type LowerFn = fn(
    &LowerCx<'_>,
    Option<Carrier>,
    &mut BTreeSet<&'static str>,
    &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline>;

/// What the operation does to its slots, as the legality analysis consumes it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Effects {
    /// The lowering can throw (a capacity or bounds `RangeError` under the target's panic
    /// policy). Panic timing is part of the carrier contract: every throw precedes any mutation.
    pub can_throw: bool,
    /// The lowering writes through its receiver slot.
    pub mutates: bool,
}

/// Linker facts a lowering needs beyond the shim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Deps {
    None,
    /// Wave 2: the exact monomorphic element drop glue item (vec push/clear/drop).
    ElementDropGlue,
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
            Self::UnknownElementLayout => "the native Vec element layout is unavailable",
            Self::ZeroSizedElement => "the native Vec does not support zero-sized elements",
            Self::ElementDropRequired => {
                "the native Vec operation requires executable element drop glue"
            }
            Self::ElementHasOwnDrop => {
                "the native Vec element has a custom destructor and stays on baseline"
            }
            Self::UnsupportedPointerWidth(_) => {
                "the native capacity lowering requires a JavaScript-number pointer width"
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

const HOST_OR_BUILDER: &[Carrier] = &[Carrier::HostString, Carrier::StringBuilder];
const BUILDER_ONLY: &[Carrier] = &[Carrier::StringBuilder];
const RESULT_STRING: &[SlotRequirement] = &[SlotRequirement::result(HOST_OR_BUILDER)];
const RESULT_BUILDER: &[SlotRequirement] = &[SlotRequirement::result(BUILDER_ONLY)];
const RECEIVER_STRING: &[SlotRequirement] = &[SlotRequirement::argument(
    0,
    ReferenceRequirement::Shared,
    HOST_OR_BUILDER,
)];
const RECEIVER_BUILDER: &[SlotRequirement] = &[SlotRequirement::argument(
    0,
    ReferenceRequirement::Shared,
    BUILDER_ONLY,
)];
const MUT_RECEIVER_BUILDER: &[SlotRequirement] = &[SlotRequirement::argument(
    0,
    ReferenceRequirement::Mutable,
    BUILDER_ONLY,
)];
const VALUE_RECEIVER_BUILDER: &[SlotRequirement] = &[SlotRequirement::argument(
    0,
    ReferenceRequirement::Value,
    BUILDER_ONLY,
)];
const NO_SLOTS: &[SlotRequirement] = &[];

const NATIVE_VEC: &[Carrier] = &[Carrier::NativeVec];
const NATIVE_BOX: &[Carrier] = &[Carrier::NativeBox];
const SLICE_VIEW: &[Carrier] = &[Carrier::SliceView];
const RESULT_VEC: &[SlotRequirement] = &[SlotRequirement::result(NATIVE_VEC)];
const RECEIVER_VEC: &[SlotRequirement] = &[SlotRequirement::argument(
    0,
    ReferenceRequirement::Shared,
    NATIVE_VEC,
)];
const MUT_RECEIVER_VEC: &[SlotRequirement] = &[SlotRequirement::argument(
    0,
    ReferenceRequirement::Mutable,
    NATIVE_VEC,
)];
const MUT_RECEIVER_AND_VALUE_VEC: &[SlotRequirement] = &[
    SlotRequirement::argument(0, ReferenceRequirement::Mutable, NATIVE_VEC),
    SlotRequirement::argument(1, ReferenceRequirement::Value, NATIVE_VEC),
];
const RECEIVER_AND_RESULT_VEC: &[SlotRequirement] = &[
    SlotRequirement::argument(0, ReferenceRequirement::Shared, NATIVE_VEC),
    SlotRequirement::result(NATIVE_VEC),
];
const MUT_RECEIVER_VALUE_AND_RESULT_VEC: &[SlotRequirement] = &[
    SlotRequirement::argument(0, ReferenceRequirement::Mutable, NATIVE_VEC),
    SlotRequirement::argument(1, ReferenceRequirement::Value, NATIVE_VEC),
    SlotRequirement::result(NATIVE_VEC),
];
const TWO_MUT_VEC: &[SlotRequirement] = &[
    SlotRequirement::argument(0, ReferenceRequirement::Mutable, NATIVE_VEC),
    SlotRequirement::argument(1, ReferenceRequirement::Mutable, NATIVE_VEC),
];
const VALUE_RECEIVER_VEC: &[SlotRequirement] = &[SlotRequirement::argument(
    0,
    ReferenceRequirement::Value,
    NATIVE_VEC,
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

const FIXTURE_71: &[&str] = &["71_repr_string_builder"];
const FIXTURE_72: &[&str] = &["72_repr_format_min"];
const FIXTURE_75: &[&str] = &["75_repr_native_vec"];
const FIXTURE_76: &[&str] = &["76_repr_vec_drop_order"];
const FIXTURE_89: &[&str] = &["89_repr_box_drop"];

macro_rules! op {
    ($name:ident, $id:ident, $parameters:expr, $slots:expr, $carriers:expr, $effects:expr,
     $legality:path, $lower:path, $helpers:expr, $cost:expr, $fixtures:expr) => {
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
            $cost,
            $fixtures,
            Deps::None
        );
    };
    ($name:ident, $id:ident, $parameters:expr, $slots:expr, $carriers:expr, $effects:expr,
     $legality:path, $lower:path, $helpers:expr, $cost:expr, $fixtures:expr, $deps:expr) => {
        pub(super) static $name: SemanticOp = SemanticOp {
            id: StandardOperation::$id,
            parameters: $parameters,
            slots: $slots,
            carriers: $carriers,
            effects: $effects,
            legality: $legality,
            lower: $lower,
            helpers: $helpers,
            cost_hint: $cost,
            deps: $deps,
            fixtures: $fixtures,
        };
    };
}

#[rustfmt::skip]
mod ops {
    use super::*;

    op!(STRING_NEW, StringNew, 0, RESULT_STRING, HOST_OR_BUILDER, PURE,
        legality_none, string::string_new, &[], 24, FIXTURE_71);
    op!(STRING_WITH_CAPACITY, StringWithCapacity, 1, RESULT_BUILDER, BUILDER_ONLY, THROWS,
        string::legality_with_capacity, string::string_with_capacity, &[], 90, FIXTURE_71);
    op!(STRING_LEN, StringLen, 1, RECEIVER_STRING, HOST_OR_BUILDER, PURE,
        string::legality_unsigned_return, string::string_len, &["_sb_check", "str_len"], 34,
        FIXTURE_71);
    op!(STRING_CAPACITY, StringCapacity, 1, RECEIVER_BUILDER, BUILDER_ONLY, PURE,
        string::legality_unsigned_return, string::string_capacity, &[], 24, FIXTURE_71);
    op!(STRING_PUSH, StringPush, 2, MUT_RECEIVER_BUILDER, BUILDER_ONLY, THROWS_AND_MUTATES,
        string::legality_push, string::string_push,
        &["_sb_check", "_utf8_width", "native_str_push"], 200, FIXTURE_71);
    op!(STRING_PUSH_STR, StringPushStr, 2, MUT_RECEIVER_BUILDER, BUILDER_ONLY, THROWS_AND_MUTATES,
        string::legality_push_str, string::string_push_str,
        &["_sb_check", "native_str_wf", "str_len"], 170, FIXTURE_71);
    op!(STRING_CLEAR, StringClear, 1, MUT_RECEIVER_BUILDER, BUILDER_ONLY, MUTATES,
        string::legality_unit_return, string::string_clear, &["_sb_check"], 26, FIXTURE_71);
    op!(STRING_AS_STR, StringAsStr, 1, RECEIVER_STRING, HOST_OR_BUILDER, PURE,
        string::legality_as_str, string::string_as_str, &[], 22, FIXTURE_71);
    op!(STRING_DROP, StringDrop, 1, MUT_RECEIVER_BUILDER, BUILDER_ONLY, MUTATES,
        string::legality_unit_return, string::string_drop, &[], 40, FIXTURE_71);
    op!(FMT_WRITE_STR, StringFmtWriteStr, 2, MUT_RECEIVER_BUILDER, BUILDER_ONLY,
        THROWS_AND_MUTATES, fmt::legality_fmt_write_str, string::string_push_str,
        &["_sb_check", "native_str_wf", "str_len"], 190, FIXTURE_72);
    op!(FMT_WRITE_CHAR, StringFmtWriteChar, 2, MUT_RECEIVER_BUILDER, BUILDER_ONLY,
        THROWS_AND_MUTATES, fmt::legality_fmt_write_char, string::string_push,
        &["_sb_check", "_utf8_width", "native_str_push"], 220, FIXTURE_72);
    op!(F64_SPEC_TO_STRING, F64SpecToString, 1, RESULT_BUILDER, BUILDER_ONLY, PURE,
        fmt::legality_f64_to_string, fmt::f64_spec_to_string,
        &["native_f64_to_string"], 60, FIXTURE_72);
    op!(INT_TO_STRING, IntToString, 1, RESULT_STRING, HOST_OR_BUILDER, PURE,
        fmt::legality_int_to_string, fmt::primitive_to_string, &["str_len"], 60, FIXTURE_72);
    op!(BOOL_TO_STRING, BoolToString, 1, RESULT_STRING, HOST_OR_BUILDER, PURE,
        fmt::legality_bool_to_string, fmt::primitive_to_string, &[], 70, FIXTURE_72);
    op!(CHAR_TO_STRING, CharToString, 1, RESULT_STRING, HOST_OR_BUILDER, PURE,
        fmt::legality_char_to_string, fmt::primitive_to_string, &["str_len"], 80, FIXTURE_72);
    op!(STR_TO_LOWERCASE, StrToLowercase, 1, RESULT_STRING, HOST_OR_BUILDER, PURE,
        fmt::legality_str_case, fmt::str_case_to_string,
        &["str_to_lowercase", "str_len"], 80, FIXTURE_72);
    op!(STR_TO_UPPERCASE, StrToUppercase, 1, RESULT_STRING, HOST_OR_BUILDER, PURE,
        fmt::legality_str_case, fmt::str_case_to_string,
        &["str_to_uppercase", "str_len"], 80, FIXTURE_72);
    op!(FORMAT_ARGUMENT_NEW_DISPLAY, FormatArgumentNewDisplay, 1, NO_SLOTS, HOST_OR_BUILDER, PURE,
        fmt::legality_argument_new_display, fmt::format_argument_new_display,
        &["native_f64_to_string", "native_f32_to_string", "fmt_float_fixed", "fmt_int_dec", "native_str_wf"], 70, FIXTURE_72);
    op!(FORMAT_ARGUMENT_NEW_LOWER_HEX, FormatArgumentNewLowerHex, 1, NO_SLOTS, HOST_OR_BUILDER, PURE,
        fmt::legality_argument_new_hex, fmt::format_argument_new_hex,
        &["fmt_int_hex"], 70, FIXTURE_72);
    op!(FORMAT_ARGUMENT_NEW_UPPER_HEX, FormatArgumentNewUpperHex, 1, NO_SLOTS, HOST_OR_BUILDER, PURE,
        fmt::legality_argument_new_hex, fmt::format_argument_new_hex,
        &["fmt_int_hex"], 70, FIXTURE_72);
    op!(FORMAT_ARGS_NEW, FormatArgsNew, 2, NO_SLOTS, HOST_OR_BUILDER, PURE,
        fmt::legality_format_args_new, fmt::format_args_new, &[], 16, FIXTURE_72);
    op!(FORMAT_ARGS_TO_STRING, FormatArgsToString, 1, RESULT_STRING, HOST_OR_BUILDER, PURE,
        fmt::legality_string_return, fmt::format_args_to_string, &["str_len"], 60, FIXTURE_72);

    op!(VEC_NEW, VecNew, 0, RESULT_VEC, NATIVE_VEC, PURE,
        legality_none, vec::vec_new, &["tv_new"], 24, FIXTURE_75);
    op!(VEC_WITH_CAPACITY, VecWithCapacity, 1, RESULT_VEC, NATIVE_VEC, THROWS,
        vec::legality_with_capacity, vec::vec_with_capacity, &["nv_cap"], 34, FIXTURE_75);
    op!(VEC_LEN, VecLen, 1, RECEIVER_VEC, NATIVE_VEC, PURE,
        vec::legality_unsigned_return, vec::vec_len, &[], 24, FIXTURE_75);
    op!(VEC_CAPACITY, VecCapacity, 1, RECEIVER_VEC, NATIVE_VEC, PURE,
        vec::legality_unsigned_return, vec::vec_capacity, &[], 24, FIXTURE_75);
    op!(VEC_IS_EMPTY, VecIsEmpty, 1, RECEIVER_VEC, NATIVE_VEC, PURE,
        vec::legality_bool_return, vec::vec_is_empty, &[], 28, FIXTURE_75);
    op!(VEC_PUSH, VecPush, 2, MUT_RECEIVER_VEC, NATIVE_VEC, THROWS_AND_MUTATES,
        vec::legality_push, vec::vec_push,
        &["nv_push"], 34, FIXTURE_75, Deps::ElementDropGlue);
    op!(VEC_PUSH_MUT, VecPushMut, 2, MUT_RECEIVER_VEC, NATIVE_VEC, THROWS_AND_MUTATES,
        vec::legality_push_mut, vec::vec_push_mut,
        &["nv_push"], 54, FIXTURE_75, Deps::ElementDropGlue);
    op!(VEC_INSERT, VecInsert, 4, MUT_RECEIVER_VEC, NATIVE_VEC, THROWS_AND_MUTATES,
        vec::legality_insert, vec::vec_insert,
        &["nv_insert"], 40, FIXTURE_75, Deps::ElementDropGlue);
    op!(VEC_INSERT_MUT, VecInsertMut, 4, MUT_RECEIVER_VEC, NATIVE_VEC, THROWS_AND_MUTATES,
        vec::legality_insert_mut, vec::vec_insert_mut,
        &["nv_insert"], 56, FIXTURE_75, Deps::ElementDropGlue);
    op!(VEC_RESERVE, VecReserve, 2, MUT_RECEIVER_VEC, NATIVE_VEC, THROWS_AND_MUTATES,
        vec::legality_reserve, vec::vec_reserve,
        &["nv_reserve"], 38, FIXTURE_75);
    op!(VEC_RESERVE_EXACT, VecReserveExact, 2, MUT_RECEIVER_VEC, NATIVE_VEC, THROWS_AND_MUTATES,
        vec::legality_reserve, vec::vec_reserve,
        &["nv_reserve"], 38, FIXTURE_75);
    op!(VEC_SHRINK_TO_FIT, VecShrinkToFit, 1, MUT_RECEIVER_VEC, NATIVE_VEC, MUTATES,
        vec::legality_unit_return, vec::vec_shrink_to_fit,
        &["nv_shrink"], 28, FIXTURE_75);
    op!(VEC_TRUNCATE, VecTruncate, 2, MUT_RECEIVER_VEC, NATIVE_VEC, MUTATES,
        vec::legality_truncate, vec::vec_truncate,
        &[], 40, FIXTURE_76, Deps::ElementDropGlue);
    op!(VEC_POP, VecPop, 1, MUT_RECEIVER_VEC, NATIVE_VEC, MUTATES,
        vec::legality_pop, vec::vec_pop, &[], 70, FIXTURE_75);
    op!(VEC_RESIZE, VecResize, 3, MUT_RECEIVER_VEC, NATIVE_VEC, THROWS_AND_MUTATES,
        vec::legality_resize, vec::vec_resize,
        &["nv_resize"], 40, FIXTURE_75);
    op!(VEC_FROM_ELEM, VecFromElem, 2, RESULT_VEC, NATIVE_VEC, THROWS,
        vec::legality_from_elem, vec::vec_from_elem,
        &["nv_from_elem"], 40, FIXTURE_75);
    op!(VEC_FROM_BOX_ARRAY, VecFromBoxArray, 1, RESULT_VEC, NATIVE_VEC, PURE,
        vec::legality_from_box_array, vec::vec_from_box_array,
        &["nv_from_box"], 32, FIXTURE_75, Deps::ElementDropGlue);
    op!(VEC_CLEAR, VecClear, 1, MUT_RECEIVER_VEC, NATIVE_VEC, MUTATES,
        vec::legality_unit_return, vec::vec_clear,
        &[], 30, FIXTURE_76, Deps::ElementDropGlue);
    op!(VEC_AS_SLICE, VecAsSlice, 1, RECEIVER_VEC, NATIVE_VEC, PURE,
        vec::legality_as_slice, vec::vec_as_slice, &[], 40, FIXTURE_75);
    op!(VEC_AS_MUT_SLICE, VecAsMutSlice, 1, MUT_RECEIVER_VEC, NATIVE_VEC, PURE,
        vec::legality_as_slice, vec::vec_as_slice, &[], 40, FIXTURE_75);
    op!(VEC_ITER, VecIter, 1, RECEIVER_VEC, NATIVE_VEC, PURE,
        vec::legality_iter, vec::vec_iter, &[], 56, FIXTURE_75);
    op!(VEC_ITER_MUT, VecIterMut, 1, MUT_RECEIVER_VEC, NATIVE_VEC, PURE,
        vec::legality_iter, vec::vec_iter, &[], 56, FIXTURE_75);
    op!(VEC_REPLACE, VecReplace, 2, MUT_RECEIVER_VALUE_AND_RESULT_VEC, NATIVE_VEC, MUTATES,
        vec::legality_replace, vec::vec_replace, &[], 80, FIXTURE_75);
    op!(VEC_MEM_SWAP, VecMemSwap, 2, TWO_MUT_VEC, NATIVE_VEC, MUTATES,
        vec::legality_mem_swap, vec::vec_mem_swap, &[], 110, FIXTURE_75);
    op!(VEC_INDEX, VecIndex, 3, RECEIVER_VEC, NATIVE_VEC, THROWS,
        vec::legality_index, vec::vec_index, &["nv_idx"], 34, FIXTURE_75);
    op!(VEC_INDEX_MUT, VecIndexMut, 3, MUT_RECEIVER_VEC, NATIVE_VEC, THROWS,
        vec::legality_index, vec::vec_index, &["nv_idx"], 34, FIXTURE_75);
    op!(VEC_RANGE_INDEX, VecRangeIndex, 3, RECEIVER_VEC, NATIVE_VEC, THROWS,
        vec::legality_range_index, vec::vec_range_index, &["nv_range"], 40, FIXTURE_75);
    op!(VEC_RANGE_INDEX_MUT, VecRangeIndexMut, 3, MUT_RECEIVER_VEC, NATIVE_VEC, THROWS,
        vec::legality_range_index, vec::vec_range_index, &["nv_range"], 40, FIXTURE_75);
    op!(VEC_REMOVE, VecRemove, 3, MUT_RECEIVER_VEC, NATIVE_VEC, THROWS_AND_MUTATES,
        vec::legality_remove, vec::vec_remove, &["nv_remove"], 34, FIXTURE_75);
    op!(VEC_SWAP_REMOVE, VecSwapRemove, 2, MUT_RECEIVER_VEC, NATIVE_VEC, THROWS_AND_MUTATES,
        vec::legality_remove, vec::vec_swap_remove, &["nv_swap_remove"], 34, FIXTURE_75);
    op!(VEC_EXTEND, VecExtend, 2, MUT_RECEIVER_AND_VALUE_VEC, NATIVE_VEC, THROWS_AND_MUTATES,
        vec::legality_extend, vec::vec_extend,
        &["nv_extend"], 38, FIXTURE_75);
    op!(VEC_EXTEND_FROM_SLICE, VecExtendFromSlice, 2, MUT_RECEIVER_VEC, NATIVE_VEC, THROWS_AND_MUTATES,
        vec::legality_extend_from_slice, vec::vec_extend_from_slice,
        &["nv_extend_slice"], 38, FIXTURE_75);
    op!(VEC_CLONE, VecClone, 1, RECEIVER_AND_RESULT_VEC, NATIVE_VEC, PURE,
        vec::legality_clone, vec::vec_clone,
        &["nv_clone"], 28, FIXTURE_75);
    op!(SLICE_TO_VEC, SliceToVec, 1, RESULT_VEC, NATIVE_VEC, PURE,
        vec::legality_slice_to_vec, vec::slice_to_vec,
        &["nv_from_slice"], 32, FIXTURE_75);
    op!(VEC_DROP, VecDrop, 1, MUT_RECEIVER_VEC, NATIVE_VEC, MUTATES,
        vec::legality_unit_return, vec::vec_drop,
        &[], 40, FIXTURE_76,
        Deps::ElementDropGlue);
    op!(BOX_DROP, BoxDrop, 1, MUT_RECEIVER_BOX, NATIVE_BOX, MUTATES,
        drop::legality_box_drop, drop::box_drop, &[], 45, FIXTURE_89,
        Deps::BoxPointeeDropGlue);
    op!(SLICE_SWAP, SliceSwap, 4, NO_SLOTS, SLICE_VIEW, THROWS_AND_MUTATES,
        slice::legality_swap, slice::slice_swap, &[], 220, FIXTURE_75);
}

fn legality_none(_cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    Ok(())
}

/// The registry: the operations v1 supported plus v2's proven Box drop, covering the string,
/// format, vec, slice, and drop clusters. `None` is an exact decline: the operation and every
/// value it touches stay baseline.
pub fn registry(operation: StandardOperation) -> Option<&'static SemanticOp> {
    use ops::*;
    Some(match operation {
        StandardOperation::VecNew => &VEC_NEW,
        StandardOperation::VecWithCapacity => &VEC_WITH_CAPACITY,
        StandardOperation::VecLen => &VEC_LEN,
        StandardOperation::VecCapacity => &VEC_CAPACITY,
        StandardOperation::VecIsEmpty => &VEC_IS_EMPTY,
        StandardOperation::VecPush => &VEC_PUSH,
        StandardOperation::VecPushMut => &VEC_PUSH_MUT,
        StandardOperation::VecInsert => &VEC_INSERT,
        StandardOperation::VecInsertMut => &VEC_INSERT_MUT,
        StandardOperation::VecReserve => &VEC_RESERVE,
        StandardOperation::VecReserveExact => &VEC_RESERVE_EXACT,
        StandardOperation::VecShrinkToFit => &VEC_SHRINK_TO_FIT,
        StandardOperation::VecTruncate => &VEC_TRUNCATE,
        StandardOperation::VecPop => &VEC_POP,
        StandardOperation::VecResize => &VEC_RESIZE,
        StandardOperation::VecFromElem => &VEC_FROM_ELEM,
        StandardOperation::VecFromBoxArray => &VEC_FROM_BOX_ARRAY,
        StandardOperation::VecClear => &VEC_CLEAR,
        StandardOperation::VecAsSlice => &VEC_AS_SLICE,
        StandardOperation::VecAsMutSlice => &VEC_AS_MUT_SLICE,
        StandardOperation::VecIter => &VEC_ITER,
        StandardOperation::VecIterMut => &VEC_ITER_MUT,
        StandardOperation::VecReplace => &VEC_REPLACE,
        StandardOperation::VecMemSwap => &VEC_MEM_SWAP,
        StandardOperation::VecIndex => &VEC_INDEX,
        StandardOperation::VecIndexMut => &VEC_INDEX_MUT,
        StandardOperation::VecRangeIndex => &VEC_RANGE_INDEX,
        StandardOperation::VecRangeIndexMut => &VEC_RANGE_INDEX_MUT,
        StandardOperation::VecRemove => &VEC_REMOVE,
        StandardOperation::VecSwapRemove => &VEC_SWAP_REMOVE,
        StandardOperation::VecExtend => &VEC_EXTEND,
        StandardOperation::VecExtendFromSlice => &VEC_EXTEND_FROM_SLICE,
        StandardOperation::VecClone => &VEC_CLONE,
        StandardOperation::SliceToVec => &SLICE_TO_VEC,
        StandardOperation::VecDrop => &VEC_DROP,
        StandardOperation::BoxDrop => &BOX_DROP,
        StandardOperation::SliceSwap => &SLICE_SWAP,
        StandardOperation::StringNew => &STRING_NEW,
        StandardOperation::StringWithCapacity => &STRING_WITH_CAPACITY,
        StandardOperation::StringLen => &STRING_LEN,
        StandardOperation::StringCapacity => &STRING_CAPACITY,
        StandardOperation::StringPush => &STRING_PUSH,
        StandardOperation::StringPushStr => &STRING_PUSH_STR,
        StandardOperation::StringClear => &STRING_CLEAR,
        StandardOperation::StringAsStr => &STRING_AS_STR,
        StandardOperation::StringDrop => &STRING_DROP,
        StandardOperation::StringFmtWriteStr => &FMT_WRITE_STR,
        StandardOperation::StringFmtWriteChar => &FMT_WRITE_CHAR,
        StandardOperation::F64SpecToString => &F64_SPEC_TO_STRING,
        StandardOperation::IntToString => &INT_TO_STRING,
        StandardOperation::BoolToString => &BOOL_TO_STRING,
        StandardOperation::CharToString => &CHAR_TO_STRING,
        StandardOperation::StrToLowercase => &STR_TO_LOWERCASE,
        StandardOperation::StrToUppercase => &STR_TO_UPPERCASE,
        StandardOperation::FormatArgumentNewDisplay => &FORMAT_ARGUMENT_NEW_DISPLAY,
        StandardOperation::FormatArgumentNewLowerHex => &FORMAT_ARGUMENT_NEW_LOWER_HEX,
        StandardOperation::FormatArgumentNewUpperHex => &FORMAT_ARGUMENT_NEW_UPPER_HEX,
        StandardOperation::FormatArgsNew => &FORMAT_ARGS_NEW,
        StandardOperation::FormatArgsToString => &FORMAT_ARGS_TO_STRING,
        _ => return None,
    })
}

/// Returns the registry contract for a call site when that contract is sound for the call's
/// monomorphized types.
///
/// Operations that invoke `Clone::clone` (`VecClone`, `SliceToVec`, `VecFromElem`, `VecResize`,
/// `VecExtendFromSlice`) only satisfy the registry's effect summary when the element type is a
/// built-in primitive whose `Clone` is guaranteed pure and non-unwinding. For any other element
/// type, callers must consult the actual callee body so user-defined `Clone` side effects and
/// unwinds are preserved.
pub fn call_effects_entry(
    operation: StandardOperation,
    types: Option<&TypeTable>,
    func: &crate::jir::repr::Func,
    result: Option<crate::jir::repr::ValueId>,
    args: &[crate::jir::repr::ValueId],
) -> Option<&'static SemanticOp> {
    let entry = registry(operation)?;
    if !matches!(
        operation,
        StandardOperation::VecClone
            | StandardOperation::SliceToVec
            | StandardOperation::VecFromElem
            | StandardOperation::VecResize
            | StandardOperation::VecExtendFromSlice
    ) {
        return Some(entry);
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
    vec::is_trivial_clone_primitive(elem_desc).then_some(entry)
}

/// The registry entry of a body, through its semantic identity.
///
/// A compiler-generated drop glue shim for a canonical `String` is `StringDrop`, and one for a
/// canonical `Vec` is `VecDrop`, each by value or by mutable reference (v1 `drop_glue_support`).
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
                RepresentationCandidate::String => {
                    let op = registry(StandardOperation::StringDrop)
                        .ok_or(SemanticDecline::NotAStandardOperation)?;
                    Ok((
                        op,
                        if by_value {
                            VALUE_RECEIVER_BUILDER
                        } else {
                            MUT_RECEIVER_BUILDER
                        },
                    ))
                }
                RepresentationCandidate::Vec => {
                    let op = registry(StandardOperation::VecDrop)
                        .ok_or(SemanticDecline::NotAStandardOperation)?;
                    Ok((
                        op,
                        if by_value {
                            VALUE_RECEIVER_VEC
                        } else {
                            MUT_RECEIVER_VEC
                        },
                    ))
                }
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
                RepresentationCandidate::None => Err(SemanticDecline::NotAStandardOperation),
            }
        }
        BodySemantic::Ordinary | BodySemantic::PanicImpl => {
            Err(SemanticDecline::NotAStandardOperation)
        }
    }
}

/// Validates every fact that is independent of the carrier choice: the proof gate the planner
/// runs before it offers an operation to the solver at all (v1 `preflight`).
pub fn validate(cx: &LowerCx<'_>) -> Result<&'static SemanticOp, SemanticDecline> {
    let (op, slots) = support_of(cx)?;
    validate_abi(cx, op)?;
    validate_slot_types(cx, slots)?;
    (op.legality)(cx)?;

    // Prove emission at every carrier the first slot admits, so a later `rewrite` cannot fail
    // for a reason validation accepted (v1 `preflight` did exactly this — including a stand-in
    // element glue name, so the proof is independent of whether the linker resolved glue yet;
    // `rewrite` still requires the real glue).
    let proof_glue = JsName::new("$nativeElementDrop".to_owned());
    let cx = LowerCx {
        semantic: cx.semantic,
        sig: cx.sig,
        table: cx.table,
        interns: cx.interns,
        element_glue: cx.element_glue.or(Some(&proof_glue)),
        element_glue_nonthrowing: cx.element_glue_nonthrowing,
        panic_impl: cx.panic_impl,
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

/// Builds the exact native body at the selected carriers (one per slot, in slot order) or
/// returns a stable decline. Slots and carriers must agree in length.
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
    // The native bounds operations map the panic to the target's `RangeError` policy: their
    // trailing track-caller parameter stays in the JavaScript ABI, but its source record is
    // never inspected (v1 `validate_abi`). No other operation takes a caller `Location`.
    let accepts_caller_location = matches!(
        op.id,
        StandardOperation::VecIndex
            | StandardOperation::VecIndexMut
            | StandardOperation::VecRangeIndex
            | StandardOperation::VecRangeIndexMut
            | StandardOperation::VecRemove
            | StandardOperation::VecInsert
            | StandardOperation::VecInsertMut
            | StandardOperation::SliceSwap
    );
    if abi.caller_location != accepts_caller_location {
        return Err(SemanticDecline::CallerLocationAbi);
    }
    if abi.parameters != op.parameters {
        return Err(SemanticDecline::ParameterCount {
            expected: op.parameters,
            actual: abi.parameters,
        });
    }
    let expected = op.parameters.saturating_sub(u16::from(abi.caller_location));
    let actual = u16::try_from(cx.sig.arguments.len()).unwrap_or(u16::MAX);
    if actual != expected {
        return Err(SemanticDecline::SignatureArgumentCount { expected, actual });
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

/// The type at a signature slot.
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

/// The slot must be the canonical global-allocator type of the family its carriers name — a
/// `Vec` for a [`Carrier::NativeVec`] slot, a `String` otherwise (v1 `validate_native_type`).
fn require_native_type(
    table: &TypeTable,
    ty: TypeId,
    requirement: &SlotRequirement,
) -> Result<(), SemanticDecline> {
    let candidate = if requirement.carriers.contains(&Carrier::NativeVec) {
        RepresentationCandidate::Vec
    } else if requirement.carriers.contains(&Carrier::NativeBox) {
        RepresentationCandidate::Box
    } else {
        RepresentationCandidate::String
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

/// The largest capacity a `bits`-wide length can carry as an exact JavaScript number
/// (`isize::MAX` of the producer's 32-bit pointer target).
pub(crate) fn pointer_limit(bits: u32) -> Result<u64, SemanticDecline> {
    if bits == 0 || bits > 32 {
        return Err(SemanticDecline::UnsupportedPointerWidth(bits));
    }
    Ok((1_u64 << (bits - 1)) - 1)
}

/// The capacity ceiling of a `String` operation, from the receiver reference's pointer width.
pub(crate) fn string_capacity_limit(cx: &LowerCx<'_>) -> Result<u64, SemanticDecline> {
    let receiver = cx
        .sig
        .arguments
        .first()
        .copied()
        .ok_or(SemanticDecline::WrongArgumentType(0))?;
    let receiver = type_get(cx.table, receiver)?;
    let TypeKind::Reference { .. } = receiver.kind else {
        return Err(SemanticDecline::WrongArgumentType(0));
    };
    let bits = receiver
        .layout
        .size
        .and_then(|bytes| bytes.checked_mul(8))
        .and_then(|bits| u32::try_from(bits).ok())
        .ok_or(SemanticDecline::UnknownLayout)?;
    pointer_limit(bits)
}

/// `if (capacity > maximum) throw new RangeError(...)` — the panic that must precede any
/// mutation (v1 `capacity_guard`, byte for byte).
pub(crate) fn capacity_guard(capacity: Expr, maximum: u64, family: &str) -> Stmt {
    jsast::if_(
        jsast::binary(BinOp::Gt, capacity, jsast::raw_num(maximum.to_string())),
        vec![jsast::throw(jsast::new(
            jsast::id("RangeError"),
            vec![jsast::string(format!(
                "rustc_codegen_js: native {family} capacity overflow"
            ))],
        ))],
    )
}

/// `&T` receiver of a shown-by-value primitive: the baseline slot read
/// `$native0.buf[$native0.off]`.
pub(crate) fn slot_load(receiver: Expr) -> Expr {
    jsast::index(
        jsast::member(receiver.clone(), "buf"),
        jsast::member(receiver, "off"),
    )
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
