//! The `String` family emitters: construction, length, append, clear, borrow, drop.
//!
//! Ports of v1 `native_std_lower.rs`'s string arms. The carriers are D5's:
//! [`Carrier::HostString`] is an immutable JavaScript string; [`Carrier::StringBuilder`] is
//! `{ text, cap, len }` with the capacity and the length in UTF-8 bytes. Every capacity or bounds
//! throw precedes every mutation.
//!
//! `len` is the exact UTF-8 byte length of `text`, kept beside it so that no operation rescans the
//! accumulated text: an append costs the bytes it appends, not the bytes already there, and
//! `String::len` is a field read. Every writer of a builder keeps the two in step — a constructor
//! knows its text is empty, knows its width, or measures it once (`__rt.str_len`) rather than
//! guess; an append adds the appended width; `clear` and drop reset it. The writers are this file,
//! the format emitters (`fmt.rs`, `fmt_kernel.rs`), the `Convert` adapters
//! (`lower/js/adapters.rs`, `__rt.convert_host_sb`), the carrier copy (`lower/js/emit.rs`) and the
//! shim's `native_*` string helpers. `RCGJS_SB_VERIFY=1` routes every length read and every
//! mutation through `__rt._sb_check`, which re-measures the text and throws on drift.
//!
//! The text is always well formed, piece by piece. Text a host handed over as a `&str` may hold a
//! lone surrogate, which the baseline `Vec<u8>` `String` stores as the three bytes of U+FFFD;
//! every such piece — a `push_str` argument here, a format site's `&str` argument where it is
//! displayed (`fmt_kernel::display_text`, `fmt::format_display_text`) — therefore goes through
//! `__rt.native_str_wf` (lone surrogate to U+FFFD) first, and every other piece is well formed by
//! construction. So the builder hands back exactly the text the baseline decodes, its length is
//! the sum of its pieces' lengths, and two adjacent halves never pair into a scalar the baseline
//! never held.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use crate::item::{JsName, LinkItem};
use crate::jir::interns::{Interns, StrId};
use crate::jir::repr::{Carrier, CellId, Func, OpKind, Proj, RTerm, Root, ValueId};
use crate::jir::semantic::{
    BinaryOp, BodySemantic, CallSemantic, Callee, CastKind, ConstTree, ItemId, StandardOperation,
};
use crate::jir::types::{AllocatorIdentity, RepresentationCandidate, TypeId, TypeKind, TypeTable};
use crate::jsast::{self, BinOp, Expr, Stmt};
use crate::lower::js::adapters::{string_parts, StringParts};

use super::{
    capacity_guard, is_shared_str, pointer_limit, slot_type, string_capacity_limit, type_get,
    unsigned_bits, Effects, LowerCx, LowerFn, ReferenceRequirement, SemanticDecline, SignatureSlot,
    SlotRequirement,
};

pub(super) fn receiver() -> Expr {
    jsast::id("$native0")
}

pub(super) fn member(name: &str) -> Expr {
    jsast::member(receiver(), name)
}

/// Whether `RCGJS_SB_VERIFY=1` is set: every builder length read and mutation is then checked by
/// `__rt._sb_check` against a fresh measurement of the text. Read once per process; a debugging
/// aid for tests, off by default.
fn verify_builders() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("RCGJS_SB_VERIFY").is_ok_and(|value| value == "1"))
}

/// The cached UTF-8 byte length of the builder `value`: `value.len`. `value` must be a pure place
/// read, because it is evaluated once per use.
pub(super) fn builder_len(value: Expr, helpers: &mut BTreeSet<&'static str>) -> Expr {
    if verify_builders() {
        helpers.insert("_sb_check");
        jsast::member(jsast::rt_call("_sb_check", vec![value]), "len")
    } else {
        jsast::member(value, "len")
    }
}

/// What closes a mutation of the builder `value`: nothing, or the drift check under
/// `RCGJS_SB_VERIFY=1`.
pub(super) fn builder_checked(value: Expr, helpers: &mut BTreeSet<&'static str>) -> Vec<Stmt> {
    if verify_builders() {
        helpers.insert("_sb_check");
        vec![jsast::expr_stmt(jsast::rt_call("_sb_check", vec![value]))]
    } else {
        Vec::new()
    }
}

/// `__rt.native_str_wf(text)`: `text` as Rust holds it once it is bytes, every lone surrogate a
/// U+FFFD. What any text a host may have produced goes through before it joins a native `String`.
pub(super) fn well_formed(text: Expr, helpers: &mut BTreeSet<&'static str>) -> Expr {
    helpers.insert("native_str_wf");
    jsast::rt_call("native_str_wf", vec![text])
}

/// `{ text, cap, len }`: the builder literal, in the property order every constructor uses (the
/// carrier copy, `record_clone`, lists its fields sorted instead).
fn builder_literal(text: Expr, capacity: Expr, length: Expr) -> Expr {
    jsast::object_of(vec![("text", text), ("cap", capacity), ("len", length)])
}

/// An exact-fit builder over `text`, whose UTF-8 byte length is `length`: the capacity a `String`
/// freshly built from that text reports. `length` is spelled twice, so it must be a name or a pure
/// read.
pub(super) fn exact_builder(text: Expr, length: Expr) -> Expr {
    builder_literal(text, length.clone(), length)
}

/// `{ text: "", cap: <capacity>, len: 0 }`: the mutable carrier's empty value.
fn native_string(capacity: Expr) -> Expr {
    builder_literal(jsast::string(""), capacity, jsast::num(0))
}

pub(super) fn string_new(
    _cx: &LowerCx<'_>,
    carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![jsast::ret(match carrier {
        Some(Carrier::HostString) => jsast::string(""),
        Some(Carrier::StringBuilder) => native_string(jsast::num(0)),
        _ => return Err(SemanticDecline::WrongCarrier(SignatureSlot::Result)),
    })])
}

pub(super) fn string_with_capacity(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let argument = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    let bits = unsigned_bits(type_get(cx.table, argument)?)
        .map_err(|_| SemanticDecline::WrongArgumentType(0))?;
    Ok(vec![
        capacity_guard(receiver(), pointer_limit(bits)?, "String"),
        jsast::ret(native_string(receiver())),
    ])
}

/// `String::len`: a host string is measured, a builder answers from its cached length.
pub(super) fn string_len(
    _cx: &LowerCx<'_>,
    carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let length = match carrier {
        Some(Carrier::HostString) => {
            helpers.insert("str_len");
            jsast::rt_call("str_len", vec![receiver()])
        }
        Some(Carrier::StringBuilder) => builder_len(receiver(), helpers),
        _ => return Err(SemanticDecline::WrongCarrier(SignatureSlot::Argument(0))),
    };
    Ok(vec![jsast::ret(length)])
}

pub(super) fn string_capacity(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![jsast::ret(member("cap"))])
}

/// The append both `push` and `push_str` are: `setup`, then `$nativeRequired` (the cached length
/// plus the appended `width`), the capacity guard, the capacity growth, and `mutation`, which
/// appends the text and leaves the length at `$nativeRequired`.
fn append(
    cx: &LowerCx<'_>,
    setup: Vec<Stmt>,
    width: Expr,
    mutation: Vec<Stmt>,
    helpers: &mut BTreeSet<&'static str>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let maximum = string_capacity_limit(cx)?;
    let mut statements = setup;
    statements.extend([
        jsast::const_(
            "$nativeRequired",
            jsast::binary(BinOp::Add, builder_len(receiver(), helpers), width),
        ),
        capacity_guard(jsast::id("$nativeRequired"), maximum, "String"),
        jsast::if_(
            jsast::binary(BinOp::Lt, member("cap"), jsast::id("$nativeRequired")),
            vec![jsast::assign_stmt(
                member("cap"),
                jsast::id("$nativeRequired"),
            )],
        ),
    ]);
    statements.extend(mutation);
    statements.extend(builder_checked(receiver(), helpers));
    statements.push(append_return(cx)?);
    Ok(statements)
}

/// `String::push` and `fmt::Write::write_char`: byte width of the scalar, capacity guard, and
/// `__rt.native_str_push`, which appends the scalar exactly and advances the length by its width.
pub(super) fn string_push(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    helpers.insert("_utf8_width");
    helpers.insert("native_str_push");
    append(
        cx,
        Vec::new(),
        jsast::rt_call("_utf8_width", vec![jsast::id("$native1")]),
        vec![jsast::expr_stmt(jsast::rt_call(
            "native_str_push",
            vec![receiver(), jsast::id("$native1")],
        ))],
        helpers,
    )
}

/// `String::push_str` and `fmt::Write::write_str`: the piece as Rust holds it (a host's lone
/// surrogate is U+FFFD), its UTF-8 length (the only text measured), capacity guard, `+=`, and the
/// new length.
pub(super) fn string_push_str(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    helpers.insert("str_len");
    let piece = well_formed(jsast::id("$native1"), helpers);
    append(
        cx,
        vec![jsast::const_("$nativePiece", piece)],
        jsast::rt_call("str_len", vec![jsast::id("$nativePiece")]),
        vec![
            jsast::expr_stmt(jsast::compound_assign(
                BinOp::Add,
                member("text"),
                jsast::id("$nativePiece"),
            )),
            jsast::assign_stmt(member("len"), jsast::id("$nativeRequired")),
        ],
        helpers,
    )
}

/// `String::clear`: the text and its length go, the capacity stays.
pub(super) fn string_clear(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let mut statements = vec![
        jsast::assign_stmt(member("text"), jsast::string("")),
        jsast::assign_stmt(member("len"), jsast::num(0)),
    ];
    statements.extend(builder_checked(receiver(), helpers));
    statements.push(jsast::ret_void());
    Ok(statements)
}

pub(super) fn string_as_str(
    _cx: &LowerCx<'_>,
    carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![jsast::ret(match carrier {
        Some(Carrier::HostString) => receiver(),
        Some(Carrier::StringBuilder) => member("text"),
        _ => return Err(SemanticDecline::WrongCarrier(SignatureSlot::Argument(0))),
    })])
}

/// The mutable carrier's drop: release the text, zero the capacity and the length. A
/// `HostString` never has a drop body — an immutable string owns nothing.
pub(super) fn string_drop(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![
        jsast::assign_stmt(member("text"), jsast::string("")),
        jsast::assign_stmt(member("cap"), jsast::num(0)),
        jsast::assign_stmt(member("len"), jsast::num(0)),
        jsast::ret_void(),
    ])
}

/// A plain append returns nothing; a `fmt::Write` append returns the proven `Ok` variant value.
fn append_return(cx: &LowerCx<'_>) -> Result<Stmt, SemanticDecline> {
    match cx.semantic {
        crate::jir::semantic::BodySemantic::Standard(
            StandardOperation::StringFmtWriteStr | StandardOperation::StringFmtWriteChar,
        ) => Ok(jsast::ret(super::fmt::write_ok_value(cx)?)),
        _ => Ok(jsast::ret_void()),
    }
}

// The operation-specific legality arms (v1 `validate_argument_types` + `validate_return`).

pub(super) fn legality_with_capacity(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let argument = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    if !matches!(
        type_get(cx.table, argument)?.kind,
        TypeKind::Integer {
            signed: false,
            bits: 32,
        }
    ) {
        return Err(SemanticDecline::WrongArgumentType(0));
    }
    legality_string_candidate_return(cx)
}

/// `len`/`capacity` return a `usize`-shaped unsigned number.
pub(super) fn legality_unsigned_return(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let result = type_get(cx.table, cx.sig.result)?;
    if !matches!(
        result.kind,
        TypeKind::Integer { signed: false, bits } if bits <= 32
    ) {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

pub(super) fn legality_unit_return(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let result = type_get(cx.table, cx.sig.result)?;
    if !matches!(&result.kind, TypeKind::Tuple(fields) if fields.is_empty()) {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

pub(super) fn legality_push(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let argument = slot_type(cx.sig, SignatureSlot::Argument(1))?;
    if !matches!(type_get(cx.table, argument)?.kind, TypeKind::Char) {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    legality_unit_return(cx)
}

pub(super) fn legality_push_str(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let argument = slot_type(cx.sig, SignatureSlot::Argument(1))?;
    if !is_shared_str(cx.table, argument) {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    legality_unit_return(cx)
}

pub(super) fn legality_as_str(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let result = type_get(cx.table, cx.sig.result)?;
    if !matches!(
        result.kind,
        TypeKind::Reference {
            pointee,
            mutable: false,
        } if type_get(cx.table, pointee)
            .is_ok_and(|pointee| matches!(pointee.kind, TypeKind::Str))
    ) {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

/// The result is the canonical global-allocator `String`.
pub(super) fn legality_string_candidate_return(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    use crate::jir::types::{AllocatorIdentity, RepresentationCandidate};
    let result = type_get(cx.table, cx.sig.result)?;
    if result.meta.candidate != RepresentationCandidate::String
        || result.meta.allocator != AllocatorIdentity::Global
    {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

// ============================================================================================
// Stage 3 (`js-str=bytes`): Byte-String Widened `String` Operations & Inlined `String.vec`
// ============================================================================================

/// Additional `alloc::string::String` operations recognized and lowered ONLY when
/// `crate::lower::bstr::enabled()`. Never activates under `js-str=utf16`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum BstrStringOp {
    IsEmpty,
    Reserve,
    ReserveExact,
    AsBytes,
}

/// Lowering contract for a [`BstrStringOp`], mirroring [`super::SemanticOp`] without touching
/// `rcgjs-object`'s wire `StandardOperation` enum or `semantic_ops/mod.rs`.
pub(crate) struct BstrOpSpec {
    pub(crate) op: BstrStringOp,
    pub(crate) parameters: u16,
    pub(crate) slots: &'static [SlotRequirement],
    pub(crate) carriers: &'static [Carrier],
    pub(crate) effects: Effects,
    pub(crate) legality: fn(&LowerCx<'_>) -> Result<(), SemanticDecline>,
    pub(crate) lower: LowerFn,
    pub(crate) helpers: &'static [&'static str],
    pub(crate) cost_hint: u32,
}

pub(crate) struct BstrRewrite {
    pub(crate) parameters: Vec<String>,
    pub(crate) statements: Vec<Stmt>,
    pub(crate) runtime_helpers: BTreeSet<&'static str>,
    pub(crate) item_references: BTreeSet<JsName>,
}

const BSTR_HOST_OR_BUILDER: &[Carrier] = &[Carrier::HostString, Carrier::StringBuilder];
const BSTR_BUILDER_ONLY: &[Carrier] = &[Carrier::StringBuilder];
const BSTR_RECEIVER_STRING: &[SlotRequirement] = &[SlotRequirement {
    slot: SignatureSlot::Argument(0),
    reference: ReferenceRequirement::Shared,
    carriers: BSTR_HOST_OR_BUILDER,
}];
const BSTR_MUT_RECEIVER_BUILDER: &[SlotRequirement] = &[SlotRequirement {
    slot: SignatureSlot::Argument(0),
    reference: ReferenceRequirement::Mutable,
    carriers: BSTR_BUILDER_ONLY,
}];
const BSTR_PURE: Effects = Effects {
    can_throw: false,
    mutates: false,
};
const BSTR_THROWS_AND_MUTATES: Effects = Effects {
    can_throw: true,
    mutates: true,
};

static SPEC_IS_EMPTY: BstrOpSpec = BstrOpSpec {
    op: BstrStringOp::IsEmpty,
    parameters: 1,
    slots: BSTR_RECEIVER_STRING,
    carriers: BSTR_HOST_OR_BUILDER,
    effects: BSTR_PURE,
    legality: legality_is_empty,
    lower: string_is_empty,
    helpers: &[],
    cost_hint: 24,
};

static SPEC_RESERVE: BstrOpSpec = BstrOpSpec {
    op: BstrStringOp::Reserve,
    parameters: 2,
    slots: BSTR_MUT_RECEIVER_BUILDER,
    carriers: BSTR_BUILDER_ONLY,
    effects: BSTR_THROWS_AND_MUTATES,
    legality: legality_reserve,
    lower: string_reserve,
    helpers: &[],
    cost_hint: 88,
};

static SPEC_RESERVE_EXACT: BstrOpSpec = BstrOpSpec {
    op: BstrStringOp::ReserveExact,
    parameters: 2,
    slots: BSTR_MUT_RECEIVER_BUILDER,
    carriers: BSTR_BUILDER_ONLY,
    effects: BSTR_THROWS_AND_MUTATES,
    legality: legality_reserve,
    lower: string_reserve,
    helpers: &[],
    cost_hint: 88,
};

static SPEC_AS_BYTES: BstrOpSpec = BstrOpSpec {
    op: BstrStringOp::AsBytes,
    parameters: 1,
    slots: BSTR_RECEIVER_STRING,
    carriers: BSTR_HOST_OR_BUILDER,
    effects: BSTR_PURE,
    legality: legality_as_bytes,
    lower: string_as_bytes,
    helpers: &["bstr_bytes"],
    cost_hint: 30,
};

pub(crate) fn bstr_spec(op: BstrStringOp) -> &'static BstrOpSpec {
    match op {
        BstrStringOp::IsEmpty => &SPEC_IS_EMPTY,
        BstrStringOp::Reserve => &SPEC_RESERVE,
        BstrStringOp::ReserveExact => &SPEC_RESERVE_EXACT,
        BstrStringOp::AsBytes => &SPEC_AS_BYTES,
    }
}

fn is_global_string_ty(table: &TypeTable, ty: TypeId) -> bool {
    table.get(table.peeled(ty)).is_some_and(|d| {
        d.meta.candidate == RepresentationCandidate::String
            && d.meta.allocator == AllocatorIdentity::Global
    })
}

fn is_u8_slice_ref(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        type_get(table, ty).map(|d| &d.kind),
        Ok(TypeKind::Reference {
            pointee,
            mutable: false,
        }) if matches!(
            type_get(table, *pointee).map(|d| &d.kind),
            Ok(TypeKind::Slice(el)) if table.byte_element(*el)
        )
    )
}

pub(crate) fn is_immutable_str_ref(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        table.get(table.peeled(ty)).map(|d| &d.kind),
        Some(TypeKind::Reference {
            pointee,
            mutable: false,
        }) if table
            .get(table.peeled(*pointee))
            .is_some_and(|d| matches!(d.kind, TypeKind::Str))
    )
}

pub(crate) fn is_immutable_u8_slice_ref(table: &TypeTable, ty: TypeId) -> bool {
    is_u8_slice_ref(table, ty)
}

pub(crate) fn item_origin_info<'a>(item: &'a LinkItem, interns: &'a Interns) -> (&'a str, &'a str) {
    let path = item
        .jir
        .origins
        .as_ref()
        .and_then(|o| interns.str(o.item_path))
        .unwrap_or("");
    (item.crate_name.as_str(), path)
}

pub(crate) fn is_stdlib_string_op_path(crate_name: &str, item_path: &str) -> Option<BstrStringOp> {
    match (crate_name, item_path) {
        ("alloc", "string::String::is_empty") | (_, "alloc::string::String::is_empty") => {
            Some(BstrStringOp::IsEmpty)
        }
        ("alloc", "string::String::reserve") | (_, "alloc::string::String::reserve") => {
            Some(BstrStringOp::Reserve)
        }
        ("alloc", "string::String::reserve_exact")
        | (_, "alloc::string::String::reserve_exact") => Some(BstrStringOp::ReserveExact),
        ("alloc", "string::String::as_bytes") | (_, "alloc::string::String::as_bytes") => {
            Some(BstrStringOp::AsBytes)
        }
        _ => None,
    }
}

pub(crate) fn is_stdlib_str_as_bytes_path(crate_name: &str, item_path: &str) -> bool {
    (crate_name == "core" && item_path == "str::<impl str>::as_bytes")
        || item_path == "core::str::<impl str>::as_bytes"
}

pub(crate) fn is_stdlib_raw_vec_reserve_path(crate_name: &str, item_path: &str) -> bool {
    let in_raw_vec = (crate_name == "alloc" && item_path.starts_with("raw_vec::RawVecInner::"))
        || item_path.starts_with("alloc::raw_vec::RawVecInner::");
    in_raw_vec && (item_path.ends_with("::reserve_exact") || item_path.ends_with("::reserve"))
}

pub(crate) fn is_raw_vec_reserve_in_table(
    table: &BTreeMap<JsName, LinkItem>,
    interns_map: &BTreeMap<JsName, &Interns>,
    lowered: &BTreeMap<ItemId, JsName>,
    item_id: ItemId,
) -> bool {
    let Some(name) = lowered.get(&item_id) else {
        return false;
    };
    let (Some(item), Some(callee_interns)) = (table.get(name), interns_map.get(name).copied())
    else {
        return false;
    };
    let (crate_name, item_path) = item_origin_info(item, callee_interns);
    is_stdlib_raw_vec_reserve_path(crate_name, item_path)
}

fn has_terminal_method(sym: &str, needle: &str) -> bool {
    let mut idx = 0;
    while let Some(pos) = sym[idx..].find(needle) {
        let after = &sym[idx + pos + needle.len()..];
        if after.is_empty() || after.starts_with("$h") {
            return true;
        }
        idx += pos + needle.len();
    }
    false
}

pub(crate) fn is_raw_vec_reserve_by_link_name(
    interns: &Interns,
    items: Option<&crate::lower::js::host::ItemIndex>,
    item_id: ItemId,
    link_name: StrId,
) -> bool {
    let sym = items
        .and_then(|m| m.get(&item_id))
        .map(|e| e.name.as_str())
        .or_else(|| interns.str(link_name))
        .unwrap_or("");
    sym.starts_with("alloc$raw_vec$")
        && (has_terminal_method(sym, "_reserve_exact") || has_terminal_method(sym, "_reserve"))
}

fn is_stdlib_alloc_helper_path(crate_name: &str, item_path: &str) -> bool {
    (crate_name == "alloc"
        && (item_path.starts_with("raw_vec::") || item_path.starts_with("alloc::")))
        || item_path.starts_with("alloc::raw_vec::")
        || item_path.starts_with("alloc::alloc::")
        || (crate_name == "core"
            && (item_path.starts_with("panicking::") || item_path.starts_with("hint::")))
        || item_path.starts_with("core::panicking::")
        || item_path.starts_with("core::hint::")
}

/// Returns `true` for a trivial `&str -> &[u8]` wrapper (`core::str::<impl str>::as_bytes`),
/// strictly under `crate::lower::bstr::enabled()`.
pub(crate) fn is_trivial_str_as_bytes(
    semantic: BodySemantic,
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
    crate_name: &str,
    item_path: &str,
) -> bool {
    if !crate::lower::bstr::enabled()
        || semantic != BodySemantic::Ordinary
        || !is_stdlib_str_as_bytes_path(crate_name, item_path)
    {
        return false;
    }
    let abi = func.sig.abi;
    if abi.spread.is_some() || abi.caller_location || abi.parameters != 1 || func.params.len() != 1
    {
        return false;
    }
    let [arg0] = func.sig.arguments.as_slice() else {
        return false;
    };
    if !is_immutable_str_ref(table, *arg0) || !is_u8_slice_ref(table, func.sig.result) {
        return false;
    }
    if func.blocks.len() != 1 {
        return false;
    }
    let entry = &func.blocks[0];
    let RTerm::Return(Some(ret_v)) = entry.term else {
        return false;
    };
    let mut vals = BTreeSet::from([func.params[0]]);
    for op in &entry.ops {
        let Some(res) = op.result else {
            return false;
        };
        let passthrough = match &op.kind {
            OpKind::Copy(v) | OpKind::Move(v) => vals.contains(v),
            OpKind::BorrowView {
                root: Root::Value(v),
                path,
                mutable: false,
            } => (path.is_empty() || path.as_slice() == [Proj::Deref]) && vals.contains(v),
            OpKind::Cast {
                kind: CastKind::Transmute | CastKind::PtrToPtr | CastKind::PointerCoercion,
                v,
                ..
            } => vals.contains(v),
            OpKind::Intrinsic { name, args, .. }
                if args.len() == 1
                    && matches!(
                        interns.str(*name),
                        Some("transmute" | "transmute_unchecked")
                    ) =>
            {
                vals.contains(&args[0])
            }
            _ => false,
        };
        if !passthrough {
            return false;
        }
        vals.insert(res);
    }
    vals.contains(&ret_v)
}

fn legality_is_empty(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    if cx.sig.arguments.len() != 1 {
        return Err(SemanticDecline::SignatureArgumentCount {
            expected: 1,
            actual: u16::try_from(cx.sig.arguments.len()).unwrap_or(u16::MAX),
        });
    }
    let result = type_get(cx.table, cx.sig.result)?;
    if !matches!(result.kind, TypeKind::Bool) {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

fn legality_reserve(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let additional = slot_type(cx.sig, SignatureSlot::Argument(1))?;
    if unsigned_bits(type_get(cx.table, additional)?).is_err() {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    legality_unit_return(cx)
}

fn legality_as_bytes(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    if cx.sig.arguments.len() != 1 {
        return Err(SemanticDecline::SignatureArgumentCount {
            expected: 1,
            actual: u16::try_from(cx.sig.arguments.len()).unwrap_or(u16::MAX),
        });
    }
    if !is_u8_slice_ref(cx.table, cx.sig.result) {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

pub(super) fn string_is_empty(
    _cx: &LowerCx<'_>,
    carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let length = match carrier {
        Some(Carrier::HostString) => jsast::member(receiver(), "length"),
        Some(Carrier::StringBuilder) => {
            if verify_builders() {
                builder_len(receiver(), helpers)
            } else {
                jsast::member(member("text"), "length")
            }
        }
        _ => return Err(SemanticDecline::WrongCarrier(SignatureSlot::Argument(0))),
    };
    Ok(vec![jsast::ret(jsast::binary(
        BinOp::StrictEq,
        length,
        jsast::num(0),
    ))])
}

/// Emits the shared JS statements updating `cap` and checking 32-bit overflow for
/// `String::reserve` / `String::reserve_exact` on a [`Carrier::StringBuilder`] receiver `sb_expr`.
pub(crate) fn reserve_builder_stmts_with(
    sb_init: Stmt,
    sb_id: Expr,
    req_name: &str,
    req_init: impl FnOnce(Expr) -> Stmt,
    req_id: Expr,
    maximum: u64,
    helpers: &mut BTreeSet<&'static str>,
) -> Vec<Stmt> {
    let len_expr = if verify_builders() {
        helpers.insert("_sb_check");
        jsast::member(jsast::rt_call("_sb_check", vec![sb_id.clone()]), "len")
    } else {
        jsast::member(sb_id.clone(), "len")
    };
    let _ = req_name;
    let mut statements = vec![
        sb_init,
        req_init(len_expr),
        capacity_guard(req_id.clone(), maximum, "String"),
        jsast::if_(
            jsast::binary(
                BinOp::Lt,
                jsast::member(sb_id.clone(), "cap"),
                req_id.clone(),
            ),
            vec![jsast::assign_stmt(
                jsast::member(sb_id.clone(), "cap"),
                req_id,
            )],
        ),
    ];
    statements.extend(builder_checked(sb_id, helpers));
    statements
}

pub(crate) fn reserve_builder_stmts(
    sb_expr: Expr,
    additional: Expr,
    maximum: u64,
    helpers: &mut BTreeSet<&'static str>,
) -> Vec<Stmt> {
    reserve_builder_stmts_with(
        jsast::const_("$nativeSb", sb_expr),
        jsast::id("$nativeSb"),
        "$nativeRequired",
        |len_expr| {
            jsast::const_(
                "$nativeRequired",
                jsast::binary(BinOp::Add, len_expr, additional),
            )
        },
        jsast::id("$nativeRequired"),
        maximum,
        helpers,
    )
}

pub(super) fn string_reserve(
    cx: &LowerCx<'_>,
    carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    if carrier != Some(Carrier::StringBuilder) {
        return Err(SemanticDecline::WrongCarrier(SignatureSlot::Argument(0)));
    }
    let maximum = string_capacity_limit(cx)?;
    let mut statements = reserve_builder_stmts(receiver(), jsast::id("$native1"), maximum, helpers);
    statements.push(jsast::ret_void());
    Ok(statements)
}

pub(super) fn string_as_bytes(
    _cx: &LowerCx<'_>,
    carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    helpers.insert("bstr_bytes");
    let text = match carrier {
        Some(Carrier::HostString) => receiver(),
        Some(Carrier::StringBuilder) => member("text"),
        _ => return Err(SemanticDecline::WrongCarrier(SignatureSlot::Argument(0))),
    };
    Ok(vec![jsast::ret(jsast::rt_call("bstr_bytes", vec![text]))])
}

/// Validates and lowers one [`BstrStringOp`], matching `semantic_ops::lower`'s slot and ABI
/// checks strictly under `crate::lower::bstr::enabled()`.
pub(crate) fn lower_bstr_op(
    op: BstrStringOp,
    cx: &LowerCx<'_>,
    selected: &[Carrier],
) -> Result<BstrRewrite, SemanticDecline> {
    if !crate::lower::bstr::enabled() {
        return Err(SemanticDecline::WrongCarrier(SignatureSlot::Argument(0)));
    }
    let spec = bstr_spec(op);
    let abi = cx.sig.abi;
    if abi.spread.is_some() {
        return Err(SemanticDecline::SpreadAbi);
    }
    if abi.caller_location {
        return Err(SemanticDecline::CallerLocationAbi);
    }
    if abi.parameters != spec.parameters {
        return Err(SemanticDecline::ParameterCount {
            expected: spec.parameters,
            actual: abi.parameters,
        });
    }
    let actual_args = u16::try_from(cx.sig.arguments.len()).unwrap_or(u16::MAX);
    if actual_args != spec.parameters {
        return Err(SemanticDecline::SignatureArgumentCount {
            expected: spec.parameters,
            actual: actual_args,
        });
    }
    if selected.len() != spec.slots.len() {
        return Err(SemanticDecline::WrongCarrier(SignatureSlot::Argument(0)));
    }
    for (req, carrier) in spec.slots.iter().zip(selected) {
        let arg_ty = slot_type(cx.sig, req.slot)?;
        let expected_mut = match req.reference {
            ReferenceRequirement::Shared => false,
            ReferenceRequirement::Mutable => true,
            ReferenceRequirement::Value => {
                if !is_global_string_ty(cx.table, arg_ty) {
                    return Err(SemanticDecline::WrongNativeType(req.slot));
                }
                if !req.carriers.contains(carrier) {
                    return Err(SemanticDecline::WrongCarrier(req.slot));
                }
                continue;
            }
        };
        let TypeKind::Reference { pointee, mutable } = type_get(cx.table, arg_ty)?.kind else {
            return Err(SemanticDecline::WrongReferenceKind(req.slot));
        };
        if mutable != expected_mut {
            return Err(SemanticDecline::WrongReferenceKind(req.slot));
        }
        if !is_global_string_ty(cx.table, pointee) {
            return Err(SemanticDecline::WrongNativeType(req.slot));
        }
        if !req.carriers.contains(carrier) {
            return Err(SemanticDecline::WrongCarrier(req.slot));
        }
    }
    (spec.legality)(cx)?;
    let parameters = (0..spec.parameters)
        .map(|i| format!("$native{i}"))
        .collect();
    let mut runtime_helpers = BTreeSet::new();
    let mut item_references = BTreeSet::new();
    let statements = (spec.lower)(
        cx,
        selected.first().copied(),
        &mut runtime_helpers,
        &mut item_references,
    )?;
    Ok(BstrRewrite {
        parameters,
        statements,
        runtime_helpers,
        item_references,
    })
}

/// Structurally classifies a stdlib `alloc::string::String` `BodySemantic::Ordinary` function into
/// [`BstrStringOp`]. Restricted strictly to canonical `alloc::string::String::*` item paths and
/// returns `None` whenever `bstr` is disabled.
#[allow(clippy::too_many_arguments)] // Classifier entry taking IR context, item path, and resolver closures.
pub(crate) fn classify_bstr_op<'a, F, P>(
    semantic: BodySemantic,
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
    crate_name: &str,
    item_path: &str,
    mut resolve_callee: F,
    mut resolve_path: P,
) -> Option<BstrStringOp>
where
    F: FnMut(ItemId) -> Option<&'a Func>,
    P: FnMut(ItemId) -> Option<(&'a str, &'a str)>,
{
    if !crate::lower::bstr::enabled() || semantic != BodySemantic::Ordinary {
        return None;
    }
    let expected = is_stdlib_string_op_path(crate_name, item_path)?;
    let abi = func.sig.abi;
    if abi.spread.is_some() || abi.caller_location {
        return None;
    }
    let args = func.sig.arguments.as_slice();
    match (expected, args) {
        (BstrStringOp::IsEmpty, [arg0]) if abi.parameters == 1 && func.params.len() == 1 => {
            let TypeKind::Reference {
                pointee,
                mutable: false,
            } = table.get(table.peeled(*arg0)).map(|d| &d.kind)?
            else {
                return None;
            };
            let parts = string_parts(table, interns, *pointee)?;
            let res_desc = table.get(table.peeled(func.sig.result))?;
            (matches!(res_desc.kind, TypeKind::Bool) && classify_is_empty_body(func, &parts))
                .then_some(BstrStringOp::IsEmpty)
        }
        (BstrStringOp::AsBytes, [arg0]) if abi.parameters == 1 && func.params.len() == 1 => {
            let TypeKind::Reference {
                pointee,
                mutable: false,
            } = table.get(table.peeled(*arg0)).map(|d| &d.kind)?
            else {
                return None;
            };
            let parts = string_parts(table, interns, *pointee)?;
            (is_u8_slice_ref(table, func.sig.result)
                && classify_as_bytes_body(func, table, interns, &parts))
            .then_some(BstrStringOp::AsBytes)
        }
        (BstrStringOp::Reserve | BstrStringOp::ReserveExact, [arg0, arg1])
            if abi.parameters == 2 && func.params.len() == 2 =>
        {
            let TypeKind::Reference {
                pointee,
                mutable: true,
            } = table.get(table.peeled(*arg0)).map(|d| &d.kind)?
            else {
                return None;
            };
            let parts = string_parts(table, interns, *pointee)?;
            let arg1_desc = table.get(table.peeled(*arg1))?;
            if unsigned_bits(arg1_desc).is_err() {
                return None;
            }
            let res_desc = table.get(table.peeled(func.sig.result))?;
            if !res_desc.layout.zst || !matches!(&res_desc.kind, TypeKind::Tuple(f) if f.is_empty())
            {
                return None;
            }
            let classified = classify_reserve_body(
                func,
                table,
                interns,
                &parts,
                &mut resolve_callee,
                &mut resolve_path,
            )?;
            match (expected, classified) {
                (BstrStringOp::Reserve, BstrStringOp::Reserve | BstrStringOp::ReserveExact) => {
                    Some(BstrStringOp::Reserve)
                }
                (BstrStringOp::ReserveExact, BstrStringOp::ReserveExact) => {
                    Some(BstrStringOp::ReserveExact)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn is_zero_const(tree: &ConstTree) -> bool {
    matches!(tree, ConstTree::Scalar { bits: 0, .. })
}

fn classify_is_empty_body(func: &Func, parts: &StringParts) -> bool {
    if func.blocks.len() != 1 || func.params.is_empty() {
        return false;
    }
    let entry = &func.blocks[0];
    let RTerm::Return(Some(ret_val)) = entry.term else {
        return false;
    };
    let v0 = func.params[0];
    let mut str_refs = BTreeSet::from([v0]);
    let mut vec_vals = BTreeSet::new();
    let mut len_refs = BTreeSet::new();
    let mut len_vals = BTreeSet::new();
    let mut zero_consts = BTreeSet::new();
    let mut bool_res = BTreeSet::new();

    for op in &entry.ops {
        let Some(res) = op.result else {
            return false;
        };
        match &op.kind {
            OpKind::Const(tree) if is_zero_const(tree) => {
                zero_consts.insert(res);
            }
            OpKind::Copy(src) | OpKind::Move(src) => {
                if str_refs.contains(src) {
                    str_refs.insert(res);
                } else if vec_vals.contains(src) {
                    vec_vals.insert(res);
                } else if len_refs.contains(src) {
                    len_refs.insert(res);
                } else if len_vals.contains(src) {
                    len_vals.insert(res);
                } else if zero_consts.contains(src) {
                    zero_consts.insert(res);
                } else if bool_res.contains(src) {
                    bool_res.insert(res);
                } else {
                    return false;
                }
            }
            OpKind::BorrowView {
                root: Root::Value(src),
                path,
                mutable: false,
            }
            | OpKind::RawView {
                root: Root::Value(src),
                path,
                mutable: false,
            } if str_refs.contains(src) => match path.as_slice() {
                [] | [Proj::Deref] => {
                    str_refs.insert(res);
                }
                [Proj::Deref, Proj::Field { field: f, .. }] if *f == parts.vec_field => {
                    vec_vals.insert(res);
                }
                [Proj::Deref, Proj::Field { field: f1, .. }, Proj::Field { field: f2, .. }]
                    if *f1 == parts.vec_field && *f2 == parts.len_field =>
                {
                    len_refs.insert(res);
                }
                _ => return false,
            },
            OpKind::BorrowView {
                root: Root::Value(src),
                path,
                mutable: false,
            }
            | OpKind::RawView {
                root: Root::Value(src),
                path,
                mutable: false,
            } if vec_vals.contains(src) => match path.as_slice() {
                [] | [Proj::Deref] => {
                    vec_vals.insert(res);
                }
                [Proj::Field { field: f, .. }] | [Proj::Deref, Proj::Field { field: f, .. }]
                    if *f == parts.len_field =>
                {
                    len_refs.insert(res);
                }
                _ => return false,
            },
            OpKind::Field { base: v, field, .. }
                if vec_vals.contains(v) && *field == parts.len_field =>
            {
                len_vals.insert(res);
            }
            OpKind::Deref(ptr) if len_refs.contains(ptr) => {
                len_vals.insert(res);
            }
            OpKind::Deref(ptr) if str_refs.contains(ptr) => {
                str_refs.insert(res);
            }
            OpKind::Call {
                args,
                semantic: CallSemantic::Standard(StandardOperation::StringLen),
                host: None,
                ..
            } if args.len() == 1 && str_refs.contains(&args[0]) => {
                len_vals.insert(res);
            }
            OpKind::Call {
                args,
                semantic: CallSemantic::Standard(StandardOperation::VecLen),
                host: None,
                ..
            } if args.len() == 1 && vec_vals.contains(&args[0]) => {
                len_vals.insert(res);
            }
            OpKind::Call {
                args,
                semantic: CallSemantic::Standard(StandardOperation::VecIsEmpty),
                host: None,
                ..
            } if args.len() == 1 && vec_vals.contains(&args[0]) => {
                bool_res.insert(res);
            }
            OpKind::Binary {
                op: BinaryOp::Eq,
                l,
                r,
                ..
            } if (len_vals.contains(l) && zero_consts.contains(r))
                || (len_vals.contains(r) && zero_consts.contains(l)) =>
            {
                bool_res.insert(res);
            }
            _ => return false,
        }
    }
    bool_res.contains(&ret_val)
}

fn classify_as_bytes_body(
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
    parts: &StringParts,
) -> bool {
    if func.blocks.len() != 1 || func.params.is_empty() {
        return false;
    }
    let entry = &func.blocks[0];
    let RTerm::Return(Some(ret_val)) = entry.term else {
        return false;
    };
    let v0 = func.params[0];
    let mut str_refs = BTreeSet::from([v0]);
    let mut vec_vals = BTreeSet::new();
    let mut len_refs = BTreeSet::new();
    let mut len_vals = BTreeSet::new();
    let mut buf_chain = BTreeSet::new();
    let mut data_ptrs = BTreeSet::new();
    let mut slice_vals = BTreeSet::new();

    for op in &entry.ops {
        let Some(res) = op.result else {
            return false;
        };
        let is_u8_ptr = func.ty(res).is_some_and(|ty| {
            matches!(
                table.get(table.peeled(ty)).map(|d| &d.kind),
                Some(TypeKind::RawPointer {
                    pointee,
                    mutable: false,
                }) if table.byte_element(*pointee)
            )
        });
        match &op.kind {
            OpKind::Copy(src) | OpKind::Move(src) => {
                if str_refs.contains(src) {
                    str_refs.insert(res);
                } else if vec_vals.contains(src) {
                    vec_vals.insert(res);
                } else if len_refs.contains(src) {
                    len_refs.insert(res);
                } else if len_vals.contains(src) {
                    len_vals.insert(res);
                } else if buf_chain.contains(src) {
                    buf_chain.insert(res);
                } else if data_ptrs.contains(src) {
                    data_ptrs.insert(res);
                } else if slice_vals.contains(src) {
                    slice_vals.insert(res);
                } else {
                    return false;
                }
            }
            OpKind::BorrowView {
                root: Root::Value(src),
                path,
                mutable: false,
            }
            | OpKind::RawView {
                root: Root::Value(src),
                path,
                mutable: false,
            } => {
                if slice_vals.contains(src) && path.as_slice() == [Proj::Deref] {
                    slice_vals.insert(res);
                } else if let Some(kind) =
                    classify_vec_proj_from_value(src, path, parts, &str_refs, &vec_vals, &buf_chain)
                {
                    match kind {
                        VecProjKind::Str => str_refs.insert(res),
                        VecProjKind::Vec => vec_vals.insert(res),
                        VecProjKind::Len => len_refs.insert(res),
                        VecProjKind::Buf => buf_chain.insert(res),
                    };
                } else {
                    return false;
                }
            }
            OpKind::Field { base: v, field, .. } => {
                if vec_vals.contains(v) && *field == parts.len_field {
                    len_vals.insert(res);
                } else if (vec_vals.contains(v) && *field == parts.buf_field)
                    || buf_chain.contains(v)
                {
                    buf_chain.insert(res);
                } else if str_refs.contains(v) && *field == parts.vec_field {
                    vec_vals.insert(res);
                } else {
                    return false;
                }
            }
            OpKind::Deref(ptr) => {
                if str_refs.contains(ptr) {
                    str_refs.insert(res);
                } else if vec_vals.contains(ptr) {
                    vec_vals.insert(res);
                } else if len_refs.contains(ptr) {
                    len_vals.insert(res);
                } else if buf_chain.contains(ptr) {
                    if is_u8_ptr {
                        data_ptrs.insert(res);
                    } else {
                        buf_chain.insert(res);
                    }
                } else {
                    return false;
                }
            }
            OpKind::Cast {
                kind: CastKind::Transmute | CastKind::PtrToPtr | CastKind::PointerCoercion,
                v,
                ..
            } if is_u8_ptr && (buf_chain.contains(v) || data_ptrs.contains(v)) => {
                data_ptrs.insert(res);
            }
            OpKind::Cast {
                kind: CastKind::Transmute | CastKind::PtrToPtr | CastKind::PointerCoercion,
                v,
                ..
            } if buf_chain.contains(v) => {
                buf_chain.insert(res);
            }
            OpKind::Intrinsic { name, args, .. }
                if args.len() == 1
                    && matches!(
                        interns.str(*name),
                        Some("transmute" | "transmute_unchecked")
                    ) =>
            {
                let v = &args[0];
                if buf_chain.contains(v) || data_ptrs.contains(v) {
                    if is_u8_ptr {
                        data_ptrs.insert(res);
                    } else {
                        buf_chain.insert(res);
                    }
                } else {
                    return false;
                }
            }
            OpKind::SliceView { ptr, len }
                if (data_ptrs.contains(ptr) || buf_chain.contains(ptr))
                    && len_vals.contains(len) =>
            {
                slice_vals.insert(res);
            }
            OpKind::Call {
                args,
                semantic: CallSemantic::Standard(StandardOperation::VecAsSlice),
                host: None,
                ..
            } if args.len() == 1 && vec_vals.contains(&args[0]) => {
                slice_vals.insert(res);
            }
            _ => return false,
        }
    }
    slice_vals.contains(&ret_val)
}

#[derive(Clone, Copy)]
enum VecProjKind {
    Str,
    Vec,
    Len,
    Buf,
}

fn classify_vec_proj_from_value(
    src: &ValueId,
    path: &[Proj],
    parts: &StringParts,
    str_refs: &BTreeSet<ValueId>,
    vec_vals: &BTreeSet<ValueId>,
    buf_chain: &BTreeSet<ValueId>,
) -> Option<VecProjKind> {
    if str_refs.contains(src) {
        let rest = match path {
            [] | [Proj::Deref] => return Some(VecProjKind::Str),
            [Proj::Deref, rest @ ..] => rest,
            rest => rest,
        };
        return classify_vec_subpath(rest, parts);
    }
    if vec_vals.contains(src) {
        let rest = match path {
            [] | [Proj::Deref] => return Some(VecProjKind::Vec),
            [Proj::Deref, rest @ ..] => rest,
            rest => rest,
        };
        return match rest {
            [Proj::Field { field: f, .. }] if *f == parts.len_field => Some(VecProjKind::Len),
            [Proj::Field { field: f, .. }, tail @ ..]
                if *f == parts.buf_field
                    && tail
                        .iter()
                        .all(|p| matches!(p, Proj::Field { .. } | Proj::Deref)) =>
            {
                Some(VecProjKind::Buf)
            }
            _ => None,
        };
    }
    if buf_chain.contains(src)
        && path
            .iter()
            .all(|p| matches!(p, Proj::Field { .. } | Proj::Deref))
    {
        return Some(VecProjKind::Buf);
    }
    None
}

fn classify_vec_subpath(rest: &[Proj], parts: &StringParts) -> Option<VecProjKind> {
    match rest {
        [Proj::Field { field: f, .. }] if *f == parts.vec_field => Some(VecProjKind::Vec),
        [Proj::Field { field: f1, .. }, Proj::Field { field: f2, .. }]
            if *f1 == parts.vec_field && *f2 == parts.len_field =>
        {
            Some(VecProjKind::Len)
        }
        [Proj::Field { field: f1, .. }, Proj::Field { field: f2, .. }, tail @ ..]
            if *f1 == parts.vec_field
                && *f2 == parts.buf_field
                && tail
                    .iter()
                    .all(|p| matches!(p, Proj::Field { .. } | Proj::Deref)) =>
        {
            Some(VecProjKind::Buf)
        }
        _ => None,
    }
}

fn classify_reserve_body<'a, F, P>(
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
    parts: &StringParts,
    resolve_callee: &mut F,
    resolve_path: &mut P,
) -> Option<BstrStringOp>
where
    F: FnMut(ItemId) -> Option<&'a Func>,
    P: FnMut(ItemId) -> Option<(&'a str, &'a str)>,
{
    let (v0, v1) = (func.params[0], func.params[1]);
    // Form A: single-block wrapper `Vec::reserve(&mut self.vec, additional)` or 1-hop `Vec::reserve_exact`.
    if func.blocks.len() == 1 {
        let entry = &func.blocks[0];
        if let RTerm::Return(None | Some(_)) = entry.term {
            let mut str_mut = BTreeSet::from([v0]);
            let mut add_vals = BTreeSet::from([v1]);
            let mut vec_mut = BTreeSet::new();
            let mut saw_reserve = false;
            let mut forwarded_vec_exact: Option<ItemId> = None;
            let mut form_a_ok = true;
            for op in &entry.ops {
                match &op.kind {
                    OpKind::Copy(s) | OpKind::Move(s) => {
                        let Some(r) = op.result else {
                            form_a_ok = false;
                            break;
                        };
                        if str_mut.contains(s) {
                            str_mut.insert(r);
                        } else if add_vals.contains(s) {
                            add_vals.insert(r);
                        } else if vec_mut.contains(s) {
                            vec_mut.insert(r);
                        } else {
                            form_a_ok = false;
                            break;
                        }
                    }
                    OpKind::BorrowView {
                        root: Root::Value(s),
                        path,
                        mutable: true,
                    } if str_mut.contains(s)
                        && matches!(
                            path.as_slice(),
                            [Proj::Deref, Proj::Field { field, .. }] if *field == parts.vec_field
                        ) =>
                    {
                        if let Some(r) = op.result {
                            vec_mut.insert(r);
                        } else {
                            form_a_ok = false;
                            break;
                        }
                    }
                    OpKind::Call {
                        args,
                        semantic: CallSemantic::Standard(StandardOperation::VecReserve),
                        host: None,
                        ..
                    } if args.len() == 2
                        && vec_mut.contains(&args[0])
                        && add_vals.contains(&args[1]) =>
                    {
                        saw_reserve = true;
                    }
                    OpKind::Call {
                        callee: Callee::Direct { item, .. },
                        args,
                        semantic: CallSemantic::Ordinary,
                        host: None,
                        ..
                    } if !saw_reserve
                        && forwarded_vec_exact.is_none()
                        && args.len() == 2
                        && vec_mut.contains(&args[0])
                        && add_vals.contains(&args[1]) =>
                    {
                        let stdlib_vec_reserve = resolve_path(*item).is_some_and(|(c, p)| {
                            ((c == "alloc" && p.starts_with("vec::Vec::"))
                                || p.starts_with("alloc::vec::Vec::"))
                                && (p.ends_with("::reserve_exact") || p.ends_with("::reserve"))
                        });
                        if stdlib_vec_reserve {
                            forwarded_vec_exact = Some(*item);
                        } else {
                            form_a_ok = false;
                            break;
                        }
                    }
                    OpKind::Const(ConstTree::Unit) => {}
                    _ => {
                        form_a_ok = false;
                        break;
                    }
                }
            }
            if form_a_ok && saw_reserve {
                return Some(BstrStringOp::Reserve);
            }
            if form_a_ok {
                if let Some(vec_item) = forwarded_vec_exact {
                    if let Some(vec_func) = resolve_callee(vec_item) {
                        if vec_func.params.len() == 2
                            && vec_func.sig.abi.parameters == 2
                            && !vec_func.sig.abi.caller_location
                            && vec_func.sig.abi.spread.is_none()
                            && classify_reserve_raw_inner(
                                vec_func,
                                table,
                                interns,
                                parts,
                                false,
                                resolve_path,
                            )
                        {
                            return Some(BstrStringOp::ReserveExact);
                        }
                    }
                }
            }
        }
    }

    if classify_reserve_raw_inner(func, table, interns, parts, true, resolve_path) {
        Some(BstrStringOp::ReserveExact)
    } else {
        None
    }
}

fn classify_reserve_raw_inner<'a, P>(
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
    parts: &StringParts,
    receiver_is_string: bool,
    resolve_path: &mut P,
) -> bool
where
    P: FnMut(ItemId) -> Option<(&'a str, &'a str)>,
{
    // Form B: MIR-inlined `RawVec::reserve_exact` / `reserve` on `(&mut String | &mut Vec<u8>, additional: usize)`.
    // Soundness invariants:
    // 1. Fixed-point tracking across ops AND CFG block-param edges (`Deref(v0) -> Field(vec) -> Field(len)`
    //    and `BorrowView` paths). Receiver/vec/len refs never escape or get written.
    // 2. `additional` (`v1`) enters capacity arithmetic (`Add*` with `len`, or `cap - len`).
    // 3. Strict op/call whitelist: ONLY pure ops, `cold_path`/`likely`/`unlikely`/`assume` intrinsics,
    //    `StoreThrough` strictly into `buf_refs`, and direct calls to `alloc::raw_vec`/`alloc::alloc`/
    //    `core::panicking`/`core::hint` helpers (with at least one grow call taking a `buf_ref`/`buf_val`).
    let (v0, v1) = (func.params[0], func.params[1]);
    let block_map: BTreeMap<crate::jir::repr::RBlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();
    let mut recv_refs = BTreeSet::from([v0]);
    let mut recv_vals = BTreeSet::new();
    let mut vec_refs = BTreeSet::new();
    let mut vec_vals = BTreeSet::new();
    let mut len_refs = BTreeSet::new();
    let mut len_vals = BTreeSet::new();
    let mut buf_refs = BTreeSet::new();
    let mut buf_vals = BTreeSet::new();
    let mut cap_vals = BTreeSet::new();
    let mut add_vals = BTreeSet::from([v1]);

    fn prop(set: &mut BTreeSet<ValueId>, from: ValueId, to: ValueId) -> bool {
        set.contains(&from) && set.insert(to)
    }

    let max_iters = func.values.len().saturating_add(4);
    for _ in 0..max_iters {
        let mut changed = false;
        for block in &func.blocks {
            for op in &block.ops {
                let Some(r) = op.result else { continue };
                match &op.kind {
                    OpKind::Copy(s) | OpKind::Move(s) => {
                        changed |= prop(&mut recv_refs, *s, r);
                        changed |= prop(&mut recv_vals, *s, r);
                        changed |= prop(&mut vec_refs, *s, r);
                        changed |= prop(&mut vec_vals, *s, r);
                        changed |= prop(&mut len_refs, *s, r);
                        changed |= prop(&mut len_vals, *s, r);
                        changed |= prop(&mut buf_refs, *s, r);
                        changed |= prop(&mut buf_vals, *s, r);
                        changed |= prop(&mut cap_vals, *s, r);
                        changed |= prop(&mut add_vals, *s, r);
                    }
                    OpKind::Cast { v, .. } => {
                        changed |= prop(&mut len_vals, *v, r);
                        changed |= prop(&mut add_vals, *v, r);
                        changed |= prop(&mut cap_vals, *v, r);
                        changed |= prop(&mut buf_refs, *v, r);
                        if buf_vals.contains(v) && unsigned_bits_opt(table, func.ty(r)) {
                            changed |= cap_vals.insert(r);
                        }
                    }
                    OpKind::BorrowView {
                        root: Root::Value(s),
                        path,
                        mutable,
                    }
                    | OpKind::RawView {
                        root: Root::Value(s),
                        path,
                        mutable,
                    } => {
                        if recv_refs.contains(s) {
                            let rest = match path.as_slice() {
                                [] | [Proj::Deref] => {
                                    changed |= recv_refs.insert(r);
                                    continue;
                                }
                                [Proj::Deref, rest @ ..] => rest,
                                rest => rest,
                            };
                            let vec_tail = if receiver_is_string {
                                match rest {
                                    [Proj::Field { field: f, .. }, tail @ ..]
                                        if *f == parts.vec_field =>
                                    {
                                        if tail.is_empty() {
                                            changed |= vec_refs.insert(r);
                                            continue;
                                        }
                                        tail
                                    }
                                    _ => return false,
                                }
                            } else {
                                rest
                            };
                            match vec_tail {
                                [Proj::Field { field: f2, .. }]
                                    if *f2 == parts.len_field && !*mutable =>
                                {
                                    changed |= len_refs.insert(r);
                                }
                                [Proj::Field { field: f2, .. }, tail @ ..]
                                    if *f2 == parts.buf_field
                                        && tail.iter().all(|p| {
                                            matches!(p, Proj::Field { .. } | Proj::Deref)
                                        }) =>
                                {
                                    changed |= buf_refs.insert(r);
                                }
                                _ => return false,
                            }
                        } else if vec_refs.contains(s) {
                            let rest = match path.as_slice() {
                                [] | [Proj::Deref] => {
                                    changed |= vec_refs.insert(r);
                                    continue;
                                }
                                [Proj::Deref, rest @ ..] => rest,
                                rest => rest,
                            };
                            match rest {
                                [Proj::Field { field: f2, .. }]
                                    if *f2 == parts.len_field && !*mutable =>
                                {
                                    changed |= len_refs.insert(r);
                                }
                                [Proj::Field { field: f2, .. }, tail @ ..]
                                    if *f2 == parts.buf_field
                                        && tail.iter().all(|p| {
                                            matches!(p, Proj::Field { .. } | Proj::Deref)
                                        }) =>
                                {
                                    changed |= buf_refs.insert(r);
                                }
                                _ => return false,
                            }
                        } else if buf_refs.contains(s) {
                            if !path
                                .iter()
                                .all(|p| matches!(p, Proj::Field { .. } | Proj::Deref))
                            {
                                return false;
                            }
                            changed |= buf_refs.insert(r);
                        }
                    }
                    OpKind::Deref(ptr) => {
                        if recv_refs.contains(ptr) {
                            changed |= recv_vals.insert(r);
                            if !receiver_is_string {
                                changed |= vec_vals.insert(r);
                            }
                        } else if vec_refs.contains(ptr) {
                            changed |= vec_vals.insert(r);
                        } else if len_refs.contains(ptr) {
                            changed |= len_vals.insert(r);
                        } else if buf_refs.contains(ptr) {
                            if unsigned_bits_opt(table, func.ty(r)) {
                                changed |= cap_vals.insert(r);
                            } else {
                                changed |= buf_vals.insert(r);
                            }
                        }
                    }
                    OpKind::Field { base, field, .. } => {
                        if recv_vals.contains(base) && receiver_is_string {
                            if *field == parts.vec_field {
                                changed |= vec_vals.insert(r);
                            } else {
                                return false;
                            }
                        } else if vec_vals.contains(base) {
                            if *field == parts.len_field {
                                changed |= len_vals.insert(r);
                            } else if *field == parts.buf_field {
                                changed |= buf_vals.insert(r);
                            } else {
                                return false;
                            }
                        } else if buf_vals.contains(base) {
                            if unsigned_bits_opt(table, func.ty(r)) {
                                changed |= cap_vals.insert(r);
                            } else {
                                changed |= buf_vals.insert(r);
                            }
                        }
                    }
                    _ => {}
                }
            }

            let mut propagate_edge =
                |target: crate::jir::repr::RBlockId, args: &[ValueId], changed: &mut bool| {
                    let Some(&bi) = block_map.get(&target) else {
                        return;
                    };
                    let params = &func.blocks[bi].params;
                    for (&arg, &param) in args.iter().zip(params.iter()) {
                        *changed |= prop(&mut recv_refs, arg, param);
                        *changed |= prop(&mut recv_vals, arg, param);
                        *changed |= prop(&mut vec_refs, arg, param);
                        *changed |= prop(&mut vec_vals, arg, param);
                        *changed |= prop(&mut len_refs, arg, param);
                        *changed |= prop(&mut len_vals, arg, param);
                        *changed |= prop(&mut buf_refs, arg, param);
                        *changed |= prop(&mut buf_vals, arg, param);
                        *changed |= prop(&mut cap_vals, arg, param);
                        *changed |= prop(&mut add_vals, arg, param);
                    }
                };
            match &block.term {
                RTerm::Goto(target, args) => propagate_edge(*target, args, &mut changed),
                RTerm::Switch { cases, default, .. } => {
                    for (_, target, args) in cases {
                        propagate_edge(*target, args, &mut changed);
                    }
                    propagate_edge(default.0, &default.1, &mut changed);
                }
                _ => {}
            }
        }
        if !changed {
            break;
        }
    }

    let protected = |u: &ValueId| {
        recv_refs.contains(u)
            || recv_vals.contains(u)
            || vec_refs.contains(u)
            || vec_vals.contains(u)
            || len_refs.contains(u)
    };
    let mut tracked_defs = BTreeSet::new();
    tracked_defs.extend(recv_refs.iter().copied());
    tracked_defs.extend(recv_vals.iter().copied());
    tracked_defs.extend(vec_refs.iter().copied());
    tracked_defs.extend(vec_vals.iter().copied());
    tracked_defs.extend(len_refs.iter().copied());
    tracked_defs.extend(len_vals.iter().copied());
    tracked_defs.extend(buf_refs.iter().copied());
    tracked_defs.extend(buf_vals.iter().copied());
    tracked_defs.extend(cap_vals.iter().copied());

    let mut saw_add_or_sub = false;
    let mut saw_raw_grow_call = false;
    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::Const(_)
                | OpKind::Copy(_)
                | OpKind::Move(_)
                | OpKind::Cast { .. }
                | OpKind::Unary { .. }
                | OpKind::Aggregate { .. }
                | OpKind::Discriminant(_)
                | OpKind::Assert { .. } => {
                    let is_tracked_prop = op.result.is_some_and(|r| tracked_defs.contains(&r));
                    if !is_tracked_prop && op.kind.uses().iter().any(protected) {
                        return false;
                    }
                }
                OpKind::BorrowView { .. }
                | OpKind::RawView { .. }
                | OpKind::Deref(_)
                | OpKind::Field { .. } => {
                    let is_tracked_prop = op.result.is_some_and(|r| tracked_defs.contains(&r));
                    if !is_tracked_prop && op.kind.uses().iter().any(protected) {
                        return false;
                    }
                }
                OpKind::Binary {
                    op: bin_op, l, r, ..
                } => {
                    if protected(l) || protected(r) {
                        return false;
                    }
                    match bin_op {
                        BinaryOp::Add | BinaryOp::AddWithOverflow | BinaryOp::AddUnchecked => {
                            if (len_vals.contains(l) && add_vals.contains(r))
                                || (len_vals.contains(r) && add_vals.contains(l))
                            {
                                saw_add_or_sub = true;
                            }
                        }
                        BinaryOp::Sub | BinaryOp::SubWithOverflow | BinaryOp::SubUnchecked
                            if cap_vals.contains(l) && len_vals.contains(r) =>
                        {
                            saw_add_or_sub = true;
                        }
                        _ => {}
                    }
                }
                OpKind::StoreThrough { ptr, v } => {
                    if !buf_refs.contains(ptr) || protected(v) {
                        return false;
                    }
                }
                OpKind::Intrinsic { name, args, .. } => {
                    if !matches!(
                        interns.str(*name),
                        Some("cold_path" | "likely" | "unlikely" | "assume")
                    ) {
                        return false;
                    }
                    if args.iter().any(protected) {
                        return false;
                    }
                }
                OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    args,
                    semantic,
                    host: None,
                    ..
                } => {
                    if !matches!(
                        semantic,
                        CallSemantic::Ordinary | CallSemantic::GlobalAllocator
                    ) {
                        return false;
                    }
                    if args.iter().any(protected) {
                        return false;
                    }
                    let Some((c, p)) = resolve_path(*item) else {
                        return false;
                    };
                    if !is_stdlib_alloc_helper_path(c, p) {
                        return false;
                    }
                    if args
                        .iter()
                        .any(|a| buf_refs.contains(a) || buf_vals.contains(a))
                    {
                        saw_raw_grow_call = true;
                    }
                }
                _ => return false,
            }
        }
        match &block.term {
            RTerm::Goto(_, _) | RTerm::Switch { .. } | RTerm::Unreachable => {}
            RTerm::Return(None) => {}
            RTerm::Return(Some(v))
                if !protected(v) && !buf_refs.contains(v) && !buf_vals.contains(v) =>
            {
                let is_unit = func
                    .ty(*v)
                    .and_then(|t| table.get(table.peeled(t)))
                    .is_some_and(|d| {
                        d.layout.zst && matches!(&d.kind, TypeKind::Tuple(f) if f.is_empty())
                    });
                if !is_unit {
                    return false;
                }
            }
            _ => return false,
        }
    }
    saw_add_or_sub && saw_raw_grow_call
}

fn unsigned_bits_opt(table: &TypeTable, ty: Option<TypeId>) -> bool {
    ty.and_then(|t| table.get(table.peeled(t)))
        .is_some_and(|d| unsigned_bits(d).is_ok())
}

// ============================================================================================
// Closed Inlined `String.vec` Analysis (`valueflow` + `representation` + `emit` + `mod`)
// ============================================================================================

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum StringRoot {
    Value(ValueId),
    Cell(CellId),
}

#[derive(Clone, Debug)]
pub(crate) struct InlinedVecCluster {
    pub(crate) root: StringRoot,
    pub(crate) root_is_ref: bool,
    pub(crate) str_ty: TypeId,
    /// ONLY internal intermediates (`VecRef`, `LenRef`, `BufChain`, `DataPtr`) — never `usize` or
    /// `&[u8]` leaves — so `commit_func` never leaks non-Baseline carriers across edges.
    pub(crate) witness_values: Vec<ValueId>,
    /// Internal intermediate value results whose baseline `.vec`/`.buf` ops are bypassed in `emit`.
    pub(crate) internal_values: BTreeSet<ValueId>,
    /// `usize` SSA values producing `(*root).vec.len`.
    pub(crate) len_values: BTreeSet<ValueId>,
    /// `Call(Standard(VecReserve), [vec_mut, additional])` `(block_idx, op_idx)` -> `additional`.
    pub(crate) reserve_calls: BTreeMap<(usize, usize), ValueId>,
    /// `SliceView` / `Call(Standard(VecDeref))` SSA values producing `&[u8]` (`as_bytes`).
    pub(crate) slice_values: BTreeSet<ValueId>,
    pub(crate) is_mut: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InlinedStringVecAnalysis {
    pub(crate) clusters: Vec<InlinedVecCluster>,
    pub(crate) op_cluster: BTreeMap<(usize, usize), usize>,
    pub(crate) value_cluster: BTreeMap<ValueId, usize>,
}

struct VecUf {
    parent: Vec<usize>,
}

impl VecUf {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }
    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InlNodeKind {
    StringVal,
    VecRef,
    LenRef,
    LenVal,
    BufChain,
    DataPtr,
    SliceVal,
}

fn const_align_is_one(tree: &ConstTree) -> bool {
    match tree {
        ConstTree::Scalar { bits, .. } => *bits == 1,
        ConstTree::Variant { variant, fields } => *variant == 0 && fields.is_empty(),
        ConstTree::Aggregate(fields) => fields.len() == 1 && const_align_is_one(&fields[0]),
        _ => false,
    }
}

fn is_byte_layout_const(tree: &ConstTree) -> bool {
    match tree {
        ConstTree::Aggregate(fields) if fields.len() == 2 => {
            matches!(fields[0], ConstTree::Scalar { bits: 1, .. }) && const_align_is_one(&fields[1])
        }
        _ => false,
    }
}

fn is_byte_layout_operand(func: &Func, op_of_res: &[Option<(usize, usize)>], v: ValueId) -> bool {
    let mut cur = v;
    for _ in 0..16 {
        let Some(&(bi, oi)) = op_of_res.get(cur.0 as usize).and_then(|x| x.as_ref()) else {
            return false;
        };
        let Some(op) = func.blocks.get(bi).and_then(|b| b.ops.get(oi)) else {
            return false;
        };
        match &op.kind {
            OpKind::Const(tree) => return is_byte_layout_const(tree),
            OpKind::Copy(src) | OpKind::Move(src) => cur = *src,
            _ => return false,
        }
    }
    false
}

fn root_is_direct_ref(func: &Func, table: &TypeTable, root: Root) -> bool {
    let ty = match root {
        Root::Cell(c) => func.cells.get(c.0 as usize).map(|cd| cd.ty),
        Root::Value(v) => func.ty(v),
    };
    ty.is_some_and(|t| {
        matches!(
            table.get(table.peeled(t)).map(|d| &d.kind),
            Some(TypeKind::Reference { .. } | TypeKind::RawPointer { .. })
        )
    })
}

fn canonical_string_root(
    func: &Func,
    table: &TypeTable,
    start: Root,
) -> Option<(StringRoot, bool, TypeId)> {
    let mut cur = start;
    let mut seen = BTreeSet::new();
    for _ in 0..=func.values.len() {
        match cur {
            Root::Cell(c) => {
                let ty = func.cells.get(c.0 as usize)?.ty;
                return is_global_string_ty(table, ty).then_some((StringRoot::Cell(c), false, ty));
            }
            Root::Value(v) => {
                if !seen.insert(v) {
                    return None;
                }
                let ty = func.ty(v)?;
                let peeled = table.get(table.peeled(ty))?;
                match &peeled.kind {
                    _ if is_global_string_ty(table, ty) => {
                        return Some((StringRoot::Value(v), false, ty));
                    }
                    TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. }
                        if is_global_string_ty(table, *pointee) =>
                    {
                        let mut next = None;
                        for block in &func.blocks {
                            for op in &block.ops {
                                if op.result == Some(v) {
                                    match &op.kind {
                                        OpKind::Copy(s) | OpKind::Move(s) => {
                                            next = Some(Root::Value(*s));
                                        }
                                        OpKind::BorrowView { root, path, .. }
                                        | OpKind::RawView { root, path, .. }
                                            if path.is_empty()
                                                || path.as_slice() == [Proj::Deref] =>
                                        {
                                            next = Some(*root);
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        if let Some(n) = next {
                            cur = n;
                            continue;
                        }
                        return Some((StringRoot::Value(v), true, *pointee));
                    }
                    _ => return None,
                }
            }
        }
    }
    None
}

/// Identifies closed MIR-inlined `String.vec` operations (`len`/`is_empty`, `reserve`, `as_bytes`)
/// inside any function, strictly gated behind `crate::lower::bstr::enabled()`.
/// Disqualifies any component whose intermediate values escape outside the whitelist.
pub(crate) fn analyze_inlined_string_vec<P>(
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
    is_raw_vec_reserve: P,
) -> Option<InlinedStringVecAnalysis>
where
    P: FnMut(ItemId, StrId) -> bool,
{
    if !crate::lower::bstr::enabled() {
        return None;
    }
    analyze_inlined_string_vec_inner(func, table, interns, is_raw_vec_reserve)
}

fn analyze_inlined_string_vec_inner<P>(
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
    mut is_raw_vec_reserve: P,
) -> Option<InlinedStringVecAnalysis>
where
    P: FnMut(ItemId, StrId) -> bool,
{
    if func.values.is_empty() {
        return None;
    }
    let n = func.values.len();
    let mut uf = VecUf::new(n);
    let mut node_kind: Vec<Option<InlNodeKind>> = vec![None; n];
    let mut node_root: Vec<Option<(StringRoot, bool, TypeId)>> = vec![None; n];
    let mut poisoned = vec![false; n];
    let mut is_mut_node = vec![false; n];
    let mut valid_def = vec![false; n];
    let mut is_deref_def = vec![false; n];
    let mut op_of_res: Vec<Option<(usize, usize)>> = vec![None; n];
    let mut reserve_site: Vec<Option<(usize, usize, ValueId)>> = vec![None; n];

    for (bi, block) in func.blocks.iter().enumerate() {
        for (oi, op) in block.ops.iter().enumerate() {
            if let Some(res) = op.result {
                op_of_res[res.0 as usize] = Some((bi, oi));
                if matches!(op.kind, OpKind::Deref(_)) {
                    is_deref_def[res.0 as usize] = true;
                }
            }
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        for block in &func.blocks {
            for op in &block.ops {
                let Some(res) = op.result else { continue };
                let r = res.0 as usize;
                if node_kind[r].is_some() {
                    continue;
                }
                match &op.kind {
                    OpKind::BorrowView {
                        root,
                        path,
                        mutable,
                    }
                    | OpKind::RawView {
                        root,
                        path,
                        mutable,
                    } => {
                        if let Some((s_root, is_ref, str_ty)) =
                            canonical_string_root(func, table, *root)
                        {
                            if let Some(parts) = string_parts(table, interns, str_ty) {
                                let rest = if root_is_direct_ref(func, table, *root) {
                                    match path.as_slice() {
                                        [Proj::Deref, tail @ ..] => tail,
                                        tail => tail,
                                    }
                                } else {
                                    path.as_slice()
                                };
                                if let Some(pk) = classify_vec_subpath(rest, &parts) {
                                    let nk = match pk {
                                        VecProjKind::Str => continue,
                                        VecProjKind::Vec => InlNodeKind::VecRef,
                                        VecProjKind::Len => InlNodeKind::LenRef,
                                        VecProjKind::Buf => InlNodeKind::BufChain,
                                    };
                                    node_kind[r] = Some(nk);
                                    node_root[r] = Some((s_root, is_ref, str_ty));
                                    valid_def[r] = true;
                                    if *mutable {
                                        is_mut_node[r] = true;
                                    }
                                    changed = true;
                                    continue;
                                }
                            }
                        }
                        if let Root::Value(src) = root {
                            if let Some(src_k) = node_kind[src.0 as usize] {
                                let s = src.0 as usize;
                                let (_, _, str_ty) = node_root[s].unwrap();
                                let parts = string_parts(table, interns, str_ty).unwrap();
                                let nk = match (src_k, path.as_slice()) {
                                    (InlNodeKind::StringVal, p) => {
                                        match classify_vec_subpath(p, &parts) {
                                            Some(VecProjKind::Vec) => Some(InlNodeKind::VecRef),
                                            Some(VecProjKind::Len) => Some(InlNodeKind::LenRef),
                                            Some(VecProjKind::Buf) => Some(InlNodeKind::BufChain),
                                            _ => None,
                                        }
                                    }
                                    (InlNodeKind::VecRef, [] | [Proj::Deref]) => {
                                        Some(InlNodeKind::VecRef)
                                    }
                                    (
                                        InlNodeKind::VecRef,
                                        [Proj::Field { field: f, .. }]
                                        | [Proj::Deref, Proj::Field { field: f, .. }],
                                    ) if *f == parts.len_field => Some(InlNodeKind::LenRef),
                                    (
                                        InlNodeKind::VecRef,
                                        [Proj::Field { field: f, .. }, tail @ ..]
                                        | [Proj::Deref, Proj::Field { field: f, .. }, tail @ ..],
                                    ) if *f == parts.buf_field
                                        && tail.iter().all(|p| {
                                            matches!(p, Proj::Field { .. } | Proj::Deref)
                                        }) =>
                                    {
                                        Some(InlNodeKind::BufChain)
                                    }
                                    (InlNodeKind::BufChain, p)
                                        if p.iter().all(|p| {
                                            matches!(p, Proj::Field { .. } | Proj::Deref)
                                        }) =>
                                    {
                                        Some(InlNodeKind::BufChain)
                                    }
                                    (InlNodeKind::SliceVal, [Proj::Deref]) => {
                                        Some(InlNodeKind::SliceVal)
                                    }
                                    _ => None,
                                };
                                if let Some(nk) = nk {
                                    uf.union(s, r);
                                    node_kind[r] = Some(nk);
                                    node_root[r] = node_root[s];
                                    valid_def[r] = true;
                                    if *mutable || is_mut_node[s] {
                                        is_mut_node[r] = true;
                                    }
                                    changed = true;
                                }
                            }
                        }
                    }
                    OpKind::Field { base: v, field, .. } => {
                        let s = v.0 as usize;
                        if let Some(src_k) = node_kind[s] {
                            let (_, _, str_ty) = node_root[s].unwrap();
                            let parts = string_parts(table, interns, str_ty).unwrap();
                            let nk = match src_k {
                                InlNodeKind::StringVal if *field == parts.vec_field => {
                                    Some(InlNodeKind::VecRef)
                                }
                                InlNodeKind::VecRef if *field == parts.len_field => {
                                    Some(InlNodeKind::LenVal)
                                }
                                InlNodeKind::VecRef if *field == parts.buf_field => {
                                    Some(InlNodeKind::BufChain)
                                }
                                InlNodeKind::BufChain => Some(InlNodeKind::BufChain),
                                _ => None,
                            };
                            if let Some(nk) = nk {
                                uf.union(s, r);
                                node_kind[r] = Some(nk);
                                node_root[r] = node_root[s];
                                valid_def[r] = true;
                                if is_mut_node[s] {
                                    is_mut_node[r] = true;
                                }
                                changed = true;
                                continue;
                            }
                        }
                        if !is_deref_def[s] {
                            if let Some((s_root, is_ref, str_ty)) =
                                canonical_string_root(func, table, Root::Value(*v))
                            {
                                if !is_ref
                                    && string_parts(table, interns, str_ty)
                                        .is_some_and(|p| p.vec_field == *field)
                                {
                                    node_kind[r] = Some(InlNodeKind::VecRef);
                                    node_root[r] = Some((s_root, false, str_ty));
                                    valid_def[r] = true;
                                    changed = true;
                                }
                            }
                        }
                    }
                    OpKind::Deref(ptr) => {
                        let s = ptr.0 as usize;
                        if let Some(src_k) = node_kind[s] {
                            let nk = match src_k {
                                InlNodeKind::LenRef => Some(InlNodeKind::LenVal),
                                InlNodeKind::VecRef => Some(InlNodeKind::VecRef),
                                InlNodeKind::BufChain => {
                                    let is_u8_ptr = func.ty(res).is_some_and(|ty| {
                                        matches!(
                                            table.get(table.peeled(ty)).map(|d| &d.kind),
                                            Some(TypeKind::RawPointer {
                                                pointee,
                                                mutable: false,
                                            }) if table.byte_element(*pointee)
                                        )
                                    });
                                    Some(if is_u8_ptr {
                                        InlNodeKind::DataPtr
                                    } else {
                                        InlNodeKind::BufChain
                                    })
                                }
                                _ => None,
                            };
                            if let Some(nk) = nk {
                                uf.union(s, r);
                                node_kind[r] = Some(nk);
                                node_root[r] = node_root[s];
                                valid_def[r] = true;
                                if is_mut_node[s] {
                                    is_mut_node[r] = true;
                                }
                                changed = true;
                            }
                        } else if root_is_direct_ref(func, table, Root::Value(*ptr)) {
                            if let Some((s_root, is_ref, str_ty)) =
                                canonical_string_root(func, table, Root::Value(*ptr))
                            {
                                node_kind[r] = Some(InlNodeKind::StringVal);
                                node_root[r] = Some((s_root, is_ref, str_ty));
                                valid_def[r] = true;
                                changed = true;
                            }
                        }
                    }
                    OpKind::Copy(src) | OpKind::Move(src) => {
                        let s = src.0 as usize;
                        if let Some(src_k) = node_kind[s] {
                            if src_k != InlNodeKind::LenVal && src_k != InlNodeKind::SliceVal {
                                uf.union(s, r);
                                node_kind[r] = Some(src_k);
                                node_root[r] = node_root[s];
                                valid_def[r] = true;
                                if is_mut_node[s] {
                                    is_mut_node[r] = true;
                                }
                                changed = true;
                            }
                        }
                    }
                    OpKind::Cast {
                        kind: CastKind::Transmute | CastKind::PtrToPtr | CastKind::PointerCoercion,
                        v,
                        ..
                    } => {
                        let s = v.0 as usize;
                        if matches!(
                            node_kind[s],
                            Some(InlNodeKind::BufChain | InlNodeKind::DataPtr)
                        ) {
                            let is_u8_ptr = func.ty(res).is_some_and(|ty| {
                                matches!(
                                    table.get(table.peeled(ty)).map(|d| &d.kind),
                                    Some(TypeKind::RawPointer {
                                        pointee,
                                        mutable: false,
                                    }) if table.byte_element(*pointee)
                                )
                            });
                            let nk = if is_u8_ptr {
                                InlNodeKind::DataPtr
                            } else {
                                InlNodeKind::BufChain
                            };
                            uf.union(s, r);
                            node_kind[r] = Some(nk);
                            node_root[r] = node_root[s];
                            valid_def[r] = true;
                            if is_mut_node[s] {
                                is_mut_node[r] = true;
                            }
                            changed = true;
                        }
                    }
                    OpKind::Intrinsic { name, args, .. }
                        if args.len() == 1
                            && matches!(
                                interns.str(*name),
                                Some("transmute" | "transmute_unchecked")
                            ) =>
                    {
                        let s = args[0].0 as usize;
                        if matches!(
                            node_kind[s],
                            Some(InlNodeKind::BufChain | InlNodeKind::DataPtr)
                        ) {
                            let is_u8_ptr = func.ty(res).is_some_and(|ty| {
                                matches!(
                                    table.get(table.peeled(ty)).map(|d| &d.kind),
                                    Some(TypeKind::RawPointer {
                                        pointee,
                                        mutable: false,
                                    }) if table.byte_element(*pointee)
                                )
                            });
                            let nk = if is_u8_ptr {
                                InlNodeKind::DataPtr
                            } else {
                                InlNodeKind::BufChain
                            };
                            uf.union(s, r);
                            node_kind[r] = Some(nk);
                            node_root[r] = node_root[s];
                            valid_def[r] = true;
                            if is_mut_node[s] {
                                is_mut_node[r] = true;
                            }
                            changed = true;
                        }
                    }
                    OpKind::SliceView { ptr, len } => {
                        let p = ptr.0 as usize;
                        let l = len.0 as usize;
                        let is_immut_slice = func.ty(res).is_some_and(|ty| {
                            matches!(
                                table.get(table.peeled(ty)).map(|d| &d.kind),
                                Some(
                                    TypeKind::Reference { mutable: false, .. }
                                        | TypeKind::RawPointer { mutable: false, .. }
                                )
                            )
                        });
                        if is_immut_slice
                            && !is_mut_node[p]
                            && matches!(
                                node_kind[p],
                                Some(InlNodeKind::DataPtr | InlNodeKind::BufChain)
                            )
                            && node_kind[l] == Some(InlNodeKind::LenVal)
                            && node_root[p].map(|x| x.0) == node_root[l].map(|x| x.0)
                        {
                            uf.union(p, r);
                            uf.union(l, r);
                            node_kind[r] = Some(InlNodeKind::SliceVal);
                            node_root[r] = node_root[p];
                            valid_def[r] = true;
                            changed = true;
                        }
                    }
                    OpKind::Call {
                        args,
                        semantic: CallSemantic::Standard(StandardOperation::VecLen),
                        host: None,
                        ..
                    } if args.len() == 1
                        && node_kind[args[0].0 as usize] == Some(InlNodeKind::VecRef) =>
                    {
                        let s = args[0].0 as usize;
                        uf.union(s, r);
                        node_kind[r] = Some(InlNodeKind::LenVal);
                        node_root[r] = node_root[s];
                        valid_def[r] = true;
                        changed = true;
                    }
                    OpKind::Call {
                        args,
                        semantic: CallSemantic::Standard(StandardOperation::VecAsSlice),
                        host: None,
                        ..
                    } if args.len() == 1
                        && node_kind[args[0].0 as usize] == Some(InlNodeKind::VecRef) =>
                    {
                        let s = args[0].0 as usize;
                        uf.union(s, r);
                        node_kind[r] = Some(InlNodeKind::SliceVal);
                        node_root[r] = node_root[s];
                        valid_def[r] = true;
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
    }

    if node_kind.iter().all(Option::is_none) {
        return None;
    }

    // Pass 2: verify every use of every internal node strictly stays in the whitelist.
    for (bi, block) in func.blocks.iter().enumerate() {
        for (oi, op) in block.ops.iter().enumerate() {
            if let OpKind::Call {
                args,
                semantic: CallSemantic::Standard(StandardOperation::VecReserve),
                host: None,
                ..
            } = &op.kind
            {
                if args.len() == 2
                    && node_kind[args[0].0 as usize] == Some(InlNodeKind::VecRef)
                    && node_kind[args[1].0 as usize].is_none()
                {
                    let v0 = args[0].0 as usize;
                    reserve_site[v0] = Some((bi, oi, args[1]));
                    is_mut_node[v0] = true;
                    continue;
                }
            }
            if let OpKind::Call {
                callee: Callee::Direct { item, link_name },
                args,
                semantic: CallSemantic::Ordinary,
                host: None,
                ..
            } = &op.kind
            {
                if args.len() == 4
                    && is_raw_vec_reserve(*item, *link_name)
                    && node_kind[args[0].0 as usize] == Some(InlNodeKind::BufChain)
                    && node_kind[args[1].0 as usize] == Some(InlNodeKind::LenVal)
                    && node_kind[args[2].0 as usize].is_none()
                    && node_root[args[0].0 as usize].map(|x| x.0)
                        == node_root[args[1].0 as usize].map(|x| x.0)
                    && is_byte_layout_operand(func, &op_of_res, args[3])
                {
                    let v0 = args[0].0 as usize;
                    let v1 = args[1].0 as usize;
                    uf.union(v0, v1);
                    reserve_site[v0] = Some((bi, oi, args[2]));
                    is_mut_node[v0] = true;
                    continue;
                }
            }

            let op_is_valid_consumer = op.result.is_some_and(|r| valid_def[r.0 as usize]);
            for u in op.kind.uses() {
                let ui = u.0 as usize;
                let Some(nk) = node_kind[ui] else { continue };
                if matches!(nk, InlNodeKind::LenVal | InlNodeKind::SliceVal) {
                    continue;
                }
                if !op_is_valid_consumer {
                    poisoned[ui] = true;
                }
            }
        }
        for u in block.term.uses() {
            let ui = u.0 as usize;
            if let Some(nk) = node_kind[ui] {
                if !matches!(nk, InlNodeKind::LenVal | InlNodeKind::SliceVal) {
                    poisoned[ui] = true;
                }
            }
        }
    }

    let mut comp_poisoned: BTreeMap<usize, bool> = BTreeMap::new();
    let mut comp_members: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for v in 0..n {
        if node_kind[v].is_none() {
            continue;
        }
        let root_id = uf.find(v);
        comp_members.entry(root_id).or_default().push(v);
        if poisoned[v] {
            comp_poisoned.insert(root_id, true);
        }
    }

    let mut out = InlinedStringVecAnalysis::default();
    for (root_id, members) in comp_members {
        if *comp_poisoned.get(&root_id).unwrap_or(&false) {
            continue;
        }
        let mut s_root = None;
        let mut root_is_ref = false;
        let mut str_ty = None;
        let mut mixed = false;
        let mut has_mut = false;
        for &m in &members {
            if let Some((sr, ir, ty)) = node_root[m] {
                if s_root.is_none() {
                    s_root = Some(sr);
                    root_is_ref = ir;
                    str_ty = Some(ty);
                } else if s_root != Some(sr) {
                    mixed = true;
                }
                if ir {
                    root_is_ref = true;
                }
            }
            if is_mut_node[m] {
                has_mut = true;
            }
        }
        if mixed {
            continue;
        }
        let (Some(root), Some(str_ty)) = (s_root, str_ty) else {
            continue;
        };
        let mut witness_values = Vec::new();
        let mut internal_values = BTreeSet::new();
        let mut len_values = BTreeSet::new();
        let mut slice_values = BTreeSet::new();
        let mut reserve_calls = BTreeMap::new();
        let cid = out.clusters.len();

        for &m in &members {
            let vid = ValueId(m as u32);
            out.value_cluster.insert(vid, cid);
            if let Some(pos) = op_of_res[m] {
                out.op_cluster.insert(pos, cid);
            }
            if let Some((bi, oi, add)) = reserve_site[m] {
                reserve_calls.insert((bi, oi), add);
                out.op_cluster.insert((bi, oi), cid);
            }
            match node_kind[m].unwrap() {
                InlNodeKind::LenVal => {
                    len_values.insert(vid);
                }
                InlNodeKind::SliceVal => {
                    slice_values.insert(vid);
                }
                _ => {
                    internal_values.insert(vid);
                    witness_values.push(vid);
                }
            }
        }
        out.clusters.push(InlinedVecCluster {
            root,
            root_is_ref,
            str_ty,
            witness_values,
            internal_values,
            len_values,
            reserve_calls,
            slice_values,
            is_mut: has_mut,
        });
    }

    (!out.clusters.is_empty()).then_some(out)
}

#[cfg(test)]
mod bstr_tests {
    use super::*;
    use crate::jir::interns::Interns;
    use crate::jir::repr::{FuncBuilder, OpKind, Proj, RTerm, Root};
    use crate::jir::semantic::{
        AbiSignature, BinaryOp, BodySemantic, CallSemantic, Callee, CastKind, ConstTree,
        DefinitionId, ItemId, Signature,
    };
    use crate::jir::types::{
        AllocatorIdentity, FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts,
        NominalAbi, NominalForm, PinnedBoundary, PointerForm, RepresentationCandidate,
        TypeDescriptor, TypeId, TypeKind, TypeMeta, TypeTable, VariantDescriptor,
    };
    use crate::lower::plan::NO_ORIGIN;

    const UNIT: TypeId = TypeId(1);
    const BOOL: TypeId = TypeId(2);
    const U8: TypeId = TypeId(3);
    const USIZE: TypeId = TypeId(4);
    const STR: TypeId = TypeId(5);
    const STR_REF: TypeId = TypeId(6);
    const SLICE_U8: TypeId = TypeId(7);
    const REF_SLICE_U8: TypeId = TypeId(8);
    const RAW_U8: TypeId = TypeId(9);
    const UNIQUE_U8: TypeId = TypeId(10);
    const CAP: TypeId = TypeId(11);
    const ALLOC_ZST: TypeId = TypeId(12);
    const MARKER_ZST: TypeId = TypeId(13);
    const RAW_VEC_INNER: TypeId = TypeId(14);
    const RAW_VEC: TypeId = TypeId(15);
    const VEC_U8: TypeId = TypeId(16);
    const VEC_U8_REF: TypeId = TypeId(17);
    const VEC_U8_MUT: TypeId = TypeId(18);
    const STRING: TypeId = TypeId(19);
    const STRING_REF: TypeId = TypeId(20);
    const STRING_MUT: TypeId = TypeId(21);
    const LEN_REF: TypeId = TypeId(22);
    const BUF_REF: TypeId = TypeId(23);
    const PTR_U8_REF: TypeId = TypeId(24);
    const PTR_U8: TypeId = TypeId(25);
    const STR_REF_MUT_NONE: TypeId = SLICE_U8;

    fn def(n: u64) -> DefinitionId {
        DefinitionId {
            crate_id: 1,
            local_hash: n,
        }
    }

    fn item(n: u64) -> ItemId {
        ItemId {
            definition: def(n),
            instantiation: n as u128,
        }
    }

    fn desc(id: TypeId, kind: TypeKind, size: u64) -> TypeDescriptor {
        let sized = !matches!(kind, TypeKind::Str | TypeKind::Slice(_));
        let zst = sized && size == 0;
        TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                size: sized.then_some(size),
                unsized_prefix: None,
                align: Some(size.clamp(1, 4)),
                sized,
                zst,
                one_zst: zst,
                needs_drop: false,
                has_own_drop: false,
            },
            meta: TypeMeta {
                candidate: RepresentationCandidate::None,
                abi: NominalAbi::Rust,
                boundary: PinnedBoundary::None,
                allocator: AllocatorIdentity::NotApplicable,
                iterator: crate::jir::types::IteratorIdentity::None,
            },
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
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        }
    }

    fn int_desc(id: TypeId, signed: bool, bits: u32) -> TypeDescriptor {
        let mut d = desc(id, TypeKind::Integer { signed, bits }, u64::from(bits / 8));
        d.js.int = Some(IntFacts {
            signed,
            bits,
            repr: IntRepr::Number,
        });
        d.js.byte_element = bits == 8 && !signed;
        d
    }

    fn ref_desc(id: TypeId, pointee: TypeId, mutable: bool, form: PointerForm) -> TypeDescriptor {
        let mut d = desc(id, TypeKind::Reference { pointee, mutable }, 4);
        d.js.pointer_form = Some(form);
        d
    }

    fn field(interns: &mut Interns, name: &str, ty: TypeId) -> FieldDescriptor {
        FieldDescriptor {
            key: FieldKey::Name(interns.intern_str(name)),
            ty,
            offset: 0,
        }
    }

    fn strukt(
        interns: &mut Interns,
        id: TypeId,
        name: &str,
        fields: Vec<FieldDescriptor>,
        args: Vec<TypeId>,
        size: u64,
    ) -> TypeDescriptor {
        let variant = VariantDescriptor {
            name: interns.intern_str(name),
            fields,
            logical_discr: 0,
            physical_tag: None,
            result_ok: false,
            saved_local_keys: Vec::new(),
        };
        let mut d = desc(
            id,
            TypeKind::Adt {
                def: def(id.0 as u64),
                args,
                form: NominalForm::Struct,
                variants: vec![variant],
                niche: None,
            },
            size,
        );
        d.js.is_indirect = true;
        d
    }

    fn build_table() -> (TypeTable, Interns) {
        let mut interns = Interns::new();
        let mut table = TypeTable::new();
        let put = |d: TypeDescriptor, t: &mut TypeTable| {
            interns.intern_type(d.clone());
            assert!(t.insert(d));
        };
        let _ = STR_REF_MUT_NONE;

        // Helper closure without capturing mutable `interns` twice:
        let add = |t: &mut TypeTable, i: &mut Interns, d: TypeDescriptor| {
            i.intern_type(d.clone());
            assert!(t.insert(d));
        };
        let _ = put;

        add(
            &mut table,
            &mut interns,
            desc(UNIT, TypeKind::Tuple(Vec::new()), 0),
        );
        add(&mut table, &mut interns, desc(BOOL, TypeKind::Bool, 1));
        add(&mut table, &mut interns, int_desc(U8, false, 8));
        add(&mut table, &mut interns, int_desc(USIZE, false, 32));
        add(&mut table, &mut interns, desc(STR, TypeKind::Str, 0));
        add(
            &mut table,
            &mut interns,
            ref_desc(STR_REF, STR, false, PointerForm::Str),
        );
        add(
            &mut table,
            &mut interns,
            desc(SLICE_U8, TypeKind::Slice(U8), 0),
        );
        add(
            &mut table,
            &mut interns,
            ref_desc(REF_SLICE_U8, SLICE_U8, false, PointerForm::FatSlice),
        );
        let mut raw_ptr = desc(
            RAW_U8,
            TypeKind::RawPointer {
                pointee: U8,
                mutable: false,
            },
            4,
        );
        raw_ptr.js.pointer_form = Some(PointerForm::Slot);
        add(&mut table, &mut interns, raw_ptr);
        let mut ptr_u8 = desc(
            PTR_U8,
            TypeKind::RawPointer {
                pointee: U8,
                mutable: false,
            },
            4,
        );
        ptr_u8.js.pointer_form = Some(PointerForm::Slot);
        add(&mut table, &mut interns, ptr_u8);
        add(
            &mut table,
            &mut interns,
            ref_desc(LEN_REF, USIZE, false, PointerForm::Slot),
        );
        add(
            &mut table,
            &mut interns,
            ref_desc(PTR_U8_REF, U8, false, PointerForm::Slot),
        );

        let mut unique = strukt(&mut interns, UNIQUE_U8, "Unique", Vec::new(), Vec::new(), 4);
        unique.js.peeled = RAW_U8;
        add(&mut table, &mut interns, unique);
        let mut cap = strukt(&mut interns, CAP, "Cap", Vec::new(), Vec::new(), 4);
        cap.js.peeled = USIZE;
        add(&mut table, &mut interns, cap);
        let d = strukt(&mut interns, ALLOC_ZST, "Global", Vec::new(), Vec::new(), 0);
        add(&mut table, &mut interns, d);
        let d = strukt(
            &mut interns,
            MARKER_ZST,
            "PhantomData",
            Vec::new(),
            Vec::new(),
            0,
        );
        add(&mut table, &mut interns, d);

        let inner_fields = vec![
            field(&mut interns, "ptr", UNIQUE_U8),
            field(&mut interns, "cap", CAP),
            field(&mut interns, "alloc", ALLOC_ZST),
        ];
        let d = strukt(
            &mut interns,
            RAW_VEC_INNER,
            "RawVecInner",
            inner_fields,
            Vec::new(),
            8,
        );
        add(&mut table, &mut interns, d);
        let raw_fields = vec![
            field(&mut interns, "inner", RAW_VEC_INNER),
            field(&mut interns, "_marker", MARKER_ZST),
        ];
        let d = strukt(&mut interns, RAW_VEC, "RawVec", raw_fields, Vec::new(), 8);
        add(&mut table, &mut interns, d);
        add(
            &mut table,
            &mut interns,
            ref_desc(BUF_REF, RAW_VEC, true, PointerForm::Slot),
        );

        let vec_fields = vec![
            field(&mut interns, "buf", RAW_VEC),
            field(&mut interns, "len", USIZE),
        ];
        let mut vec_u8 = strukt(
            &mut interns,
            VEC_U8,
            "Vec",
            vec_fields,
            vec![U8, ALLOC_ZST],
            12,
        );
        vec_u8.meta.candidate = RepresentationCandidate::Vec;
        vec_u8.meta.allocator = AllocatorIdentity::Global;
        add(&mut table, &mut interns, vec_u8);
        add(
            &mut table,
            &mut interns,
            ref_desc(VEC_U8_REF, VEC_U8, false, PointerForm::Slot),
        );
        add(
            &mut table,
            &mut interns,
            ref_desc(VEC_U8_MUT, VEC_U8, true, PointerForm::Slot),
        );

        let str_fields = vec![field(&mut interns, "vec", VEC_U8)];
        let mut string = strukt(&mut interns, STRING, "String", str_fields, Vec::new(), 12);
        string.meta.candidate = RepresentationCandidate::String;
        string.meta.allocator = AllocatorIdentity::Global;
        add(&mut table, &mut interns, string);
        add(
            &mut table,
            &mut interns,
            ref_desc(STRING_REF, STRING, false, PointerForm::Slot),
        );
        add(
            &mut table,
            &mut interns,
            ref_desc(STRING_MUT, STRING, true, PointerForm::Slot),
        );

        (table, interns)
    }

    fn sig(arguments: Vec<TypeId>, result: TypeId) -> Signature {
        Signature {
            abi: AbiSignature {
                parameters: u16::try_from(arguments.len()).unwrap(),
                spread: None,
                caller_location: false,
                inline_never: false,
                inline_hint: false,
                inline_always: false,
            },
            arguments,
            result,
        }
    }

    fn bstr_cx<'a>(sig: &'a Signature, table: &'a TypeTable, interns: &'a Interns) -> LowerCx<'a> {
        LowerCx {
            semantic: BodySemantic::Ordinary,
            sig,
            table,
            interns,
            element_glue: None,
            element_glue_nonthrowing: false,
            panic_impl: None,
            panic_reads_info: false,
            native_string_enabled: false,
        }
    }

    #[test]
    fn bstr_disabled_gate_blocks_all_entry_points() {
        let (table, interns) = build_table();
        let sig_is_empty = sig(vec![STRING_REF], BOOL);
        let mut b = FuncBuilder::new(item(1), sig_is_empty.clone());
        let _p = b.param(STRING_REF, NO_ORIGIN);
        let zero = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            USIZE,
            NO_ORIGIN,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(zero)));
        let f_empty = b.finish();

        let mut bs = FuncBuilder::new(item(2), sig(vec![STR_REF], REF_SLICE_U8));
        let ps = bs.param(STR_REF, NO_ORIGIN);
        let cast = bs.op(
            FuncBuilder::ENTRY,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: ps,
                from: STR_REF,
                to: REF_SLICE_U8,
            },
            REF_SLICE_U8,
            NO_ORIGIN,
        );
        bs.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(cast)));
        let f_str_as_bytes = bs.finish();

        assert!(!is_trivial_str_as_bytes(
            BodySemantic::Ordinary,
            &f_str_as_bytes,
            &table,
            &interns,
            "core",
            "core::str::<impl str>::as_bytes",
        ));
        assert_eq!(
            classify_bstr_op(
                BodySemantic::Ordinary,
                &f_empty,
                &table,
                &interns,
                "alloc",
                "alloc::string::String::is_empty",
                |_| None,
                |_| None,
            ),
            None
        );
        let cx = bstr_cx(&sig_is_empty, &table, &interns);
        assert!(lower_bstr_op(BstrStringOp::IsEmpty, &cx, &[Carrier::HostString]).is_err());
        assert!(analyze_inlined_string_vec(&f_empty, &table, &interns, |_, _| false).is_none());
    }

    #[test]
    fn bstr_classify_and_lower_is_empty() {
        let (table, interns) = build_table();
        let parts = string_parts(&table, &interns, STRING).expect("canonical String parts");

        let s = sig(vec![STRING_REF], BOOL);
        let mut b = FuncBuilder::new(item(10), s.clone());
        let p = b.param(STRING_REF, NO_ORIGIN);
        let e = FuncBuilder::ENTRY;
        let len_ref = b.op(
            e,
            OpKind::BorrowView {
                root: Root::Value(p),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.vec_field,
                        ty: VEC_U8,
                    },
                    Proj::Field {
                        field: parts.len_field,
                        ty: USIZE,
                    },
                ],
                mutable: false,
            },
            LEN_REF,
            NO_ORIGIN,
        );
        let len_val = b.op(e, OpKind::Deref(len_ref), USIZE, NO_ORIGIN);
        let zero = b.op(
            e,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            USIZE,
            NO_ORIGIN,
        );
        let is_zero = b.op(
            e,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: len_val,
                r: zero,
                ty: BOOL,
            },
            BOOL,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(Some(is_zero)));
        let func = b.finish();
        assert!(classify_is_empty_body(&func, &parts));

        // Negative: Ne is not `is_empty`.
        let mut bad = FuncBuilder::new(item(11), s.clone());
        let p2 = bad.param(STRING_REF, NO_ORIGIN);
        let e2 = FuncBuilder::ENTRY;
        let lr2 = bad.op(
            e2,
            OpKind::BorrowView {
                root: Root::Value(p2),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.vec_field,
                        ty: VEC_U8,
                    },
                    Proj::Field {
                        field: parts.len_field,
                        ty: USIZE,
                    },
                ],
                mutable: false,
            },
            LEN_REF,
            NO_ORIGIN,
        );
        let lv2 = bad.op(e2, OpKind::Deref(lr2), USIZE, NO_ORIGIN);
        let z2 = bad.op(
            e2,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            USIZE,
            NO_ORIGIN,
        );
        let ne = bad.op(
            e2,
            OpKind::Binary {
                op: BinaryOp::Ne,
                l: lv2,
                r: z2,
                ty: BOOL,
            },
            BOOL,
            NO_ORIGIN,
        );
        bad.terminate(e2, RTerm::Return(Some(ne)));
        assert!(!classify_is_empty_body(&bad.finish(), &parts));

        let cx = bstr_cx(&s, &table, &interns);
        assert!(legality_is_empty(&cx).is_ok());

        for (carrier, expected_substr) in [
            (Carrier::HostString, "$native0.length === 0"),
            (Carrier::StringBuilder, "$native0.text.length === 0"),
        ] {
            let mut helpers = BTreeSet::new();
            let mut refs = BTreeSet::new();
            let stmts = string_is_empty(&cx, Some(carrier), &mut helpers, &mut refs)
                .expect("is_empty lowers");
            let printed =
                jsast::stmt_to_string(&jsast::function("f", vec!["$native0".to_owned()], stmts));
            assert!(printed.contains(expected_substr), "{printed}");
            assert!(helpers.is_empty(), "{helpers:?}");
        }
    }

    #[test]
    fn bstr_classify_and_lower_reserve_and_exact() {
        let (table, interns) = build_table();
        let parts = string_parts(&table, &interns, STRING).expect("canonical String parts");
        // Form A (`VecReserve` semantic call) AND inlined `RawVec` reserve on `&mut self`.
        let s_res = sig(vec![STRING_MUT, USIZE], UNIT);
        let mut b_vec = FuncBuilder::new(item(19), s_res.clone());
        let pv = b_vec.param(STRING_MUT, NO_ORIGIN);
        let addv = b_vec.param(USIZE, NO_ORIGIN);
        let ev = FuncBuilder::ENTRY;
        let vec_ref_v = b_vec.op(
            ev,
            OpKind::BorrowView {
                root: Root::Value(pv),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.vec_field,
                        ty: VEC_U8,
                    },
                ],
                mutable: true,
            },
            VEC_U8_MUT,
            NO_ORIGIN,
        );
        b_vec.effect(
            ev,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(899),
                    link_name: crate::jir::interns::StrId(0),
                },
                function: None,
                args: vec![vec_ref_v, addv],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::VecReserve),
                host: None,
            },
            NO_ORIGIN,
        );
        b_vec.terminate(ev, RTerm::Return(None));
        assert_eq!(
            classify_reserve_body(
                &b_vec.finish(),
                &table,
                &interns,
                &parts,
                &mut |_| None,
                &mut |_| None,
            ),
            Some(BstrStringOp::Reserve)
        );

        // Inlined `RawVec` grow inside `String::reserve_exact` (`Form B` direct).
        let mut ba = FuncBuilder::new(item(20), s_res.clone());
        let pa = ba.param(STRING_MUT, NO_ORIGIN);
        let adda = ba.param(USIZE, NO_ORIGIN);
        let ea = FuncBuilder::ENTRY;
        let len_ref = ba.op(
            ea,
            OpKind::BorrowView {
                root: Root::Value(pa),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.vec_field,
                        ty: VEC_U8,
                    },
                    Proj::Field {
                        field: parts.len_field,
                        ty: USIZE,
                    },
                ],
                mutable: false,
            },
            LEN_REF,
            NO_ORIGIN,
        );
        let len_val = ba.op(ea, OpKind::Deref(len_ref), USIZE, NO_ORIGIN);
        let _sum = ba.op(
            ea,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: len_val,
                r: adda,
                ty: USIZE,
            },
            USIZE,
            NO_ORIGIN,
        );
        let buf_ref = ba.op(
            ea,
            OpKind::BorrowView {
                root: Root::Value(pa),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.vec_field,
                        ty: VEC_U8,
                    },
                    Proj::Field {
                        field: parts.buf_field,
                        ty: RAW_VEC,
                    },
                ],
                mutable: true,
            },
            BUF_REF,
            NO_ORIGIN,
        );
        ba.effect(
            ea,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(900),
                    link_name: crate::jir::interns::StrId(0),
                },
                function: None,
                args: vec![buf_ref, len_val, adda],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
        ba.terminate(ea, RTerm::Return(None));
        let fa = ba.finish();
        assert_eq!(
            classify_reserve_body(&fa, &table, &interns, &parts, &mut |_| None, &mut |id| (id
                == item(900))
            .then_some((
                "alloc",
                "alloc::raw_vec::RawVecInner::<Global>::finish_grow"
            )),),
            Some(BstrStringOp::ReserveExact)
        );

        // Form B: 1-hop `String::reserve_exact` delegating `&mut self.vec` to `Vec::reserve_exact`.
        let mut bi = FuncBuilder::new(item(21), sig(vec![VEC_U8_MUT, USIZE], UNIT));
        let pi = bi.param(VEC_U8_MUT, NO_ORIGIN);
        let addi = bi.param(USIZE, NO_ORIGIN);
        let ei = FuncBuilder::ENTRY;
        let li_ref = bi.op(
            ei,
            OpKind::BorrowView {
                root: Root::Value(pi),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.len_field,
                        ty: USIZE,
                    },
                ],
                mutable: false,
            },
            LEN_REF,
            NO_ORIGIN,
        );
        let li_val = bi.op(ei, OpKind::Deref(li_ref), USIZE, NO_ORIGIN);
        let _sumi = bi.op(
            ei,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: li_val,
                r: addi,
                ty: USIZE,
            },
            USIZE,
            NO_ORIGIN,
        );
        let bi_ref = bi.op(
            ei,
            OpKind::BorrowView {
                root: Root::Value(pi),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.buf_field,
                        ty: RAW_VEC,
                    },
                ],
                mutable: true,
            },
            BUF_REF,
            NO_ORIGIN,
        );
        bi.effect(
            ei,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(901),
                    link_name: crate::jir::interns::StrId(0),
                },
                function: None,
                args: vec![bi_ref, li_val, addi],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
        bi.terminate(ei, RTerm::Return(None));
        let f_inner = bi.finish();

        let mut bo = FuncBuilder::new(item(22), s_res.clone());
        let po = bo.param(STRING_MUT, NO_ORIGIN);
        let addo = bo.param(USIZE, NO_ORIGIN);
        let eo = FuncBuilder::ENTRY;
        let vec_mut = bo.op(
            eo,
            OpKind::BorrowView {
                root: Root::Value(po),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.vec_field,
                        ty: VEC_U8,
                    },
                ],
                mutable: true,
            },
            VEC_U8_MUT,
            NO_ORIGIN,
        );
        bo.effect(
            eo,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(21),
                    link_name: crate::jir::interns::StrId(0),
                },
                function: None,
                args: vec![vec_mut, addo],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
        bo.terminate(eo, RTerm::Return(None));
        let f_outer = bo.finish();
        assert_eq!(
            classify_reserve_body(
                &f_outer,
                &table,
                &interns,
                &parts,
                &mut |id| (id == item(21)).then_some(&f_inner),
                &mut |id| match id.definition.local_hash {
                    21 => Some(("alloc", "alloc::vec::Vec::<u8>::reserve_exact")),
                    901 => Some((
                        "alloc",
                        "alloc::raw_vec::RawVecInner::<Global>::finish_grow"
                    )),
                    _ => None,
                },
            ),
            Some(BstrStringOp::ReserveExact)
        );

        // Negative: writing through `len_ref` is rejected.
        let mut bbad = FuncBuilder::new(item(23), s_res.clone());
        let pb = bbad.param(STRING_MUT, NO_ORIGIN);
        let addb = bbad.param(USIZE, NO_ORIGIN);
        let eb = FuncBuilder::ENTRY;
        let lr_bad = bbad.op(
            eb,
            OpKind::BorrowView {
                root: Root::Value(pb),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.vec_field,
                        ty: VEC_U8,
                    },
                    Proj::Field {
                        field: parts.len_field,
                        ty: USIZE,
                    },
                ],
                mutable: false,
            },
            LEN_REF,
            NO_ORIGIN,
        );
        bbad.effect(
            eb,
            OpKind::StoreThrough {
                ptr: lr_bad,
                v: addb,
            },
            NO_ORIGIN,
        );
        bbad.terminate(eb, RTerm::Return(None));
        assert_eq!(
            classify_reserve_body(
                &bbad.finish(),
                &table,
                &interns,
                &parts,
                &mut |_| None,
                &mut |_| None,
            ),
            None
        );

        // Lowering check: 32-bit overflow RangeError precedes `.cap = Math.max(...)` mutation.
        let cx = bstr_cx(&s_res, &table, &interns);
        assert!(legality_reserve(&cx).is_ok());
        let mut helpers = BTreeSet::new();
        let mut refs = BTreeSet::new();
        assert!(string_reserve(&cx, Some(Carrier::HostString), &mut helpers, &mut refs).is_err());
        let stmts = string_reserve(&cx, Some(Carrier::StringBuilder), &mut helpers, &mut refs)
            .expect("reserve lowers on StringBuilder");
        let printed = jsast::stmt_to_string(&jsast::function(
            "f",
            vec!["$native0".to_owned(), "$native1".to_owned()],
            stmts,
        ));
        let range_pos = printed
            .find("RangeError")
            .expect("overflow throws RangeError");
        let cap_pos = printed.find(".cap =").expect("updates .cap");
        assert!(
            range_pos < cap_pos,
            "overflow guard must precede .cap write:\n{printed}"
        );
        assert!(printed.contains("2147483647"), "{printed}");
        assert!(
            printed.contains("$nativeSb.cap < $nativeRequired"),
            "{printed}"
        );
    }

    #[test]
    fn bstr_classify_and_lower_as_bytes() {
        let (table, interns) = build_table();
        let parts = string_parts(&table, &interns, STRING).expect("canonical String parts");

        let s = sig(vec![STRING_REF], REF_SLICE_U8);
        let mut b = FuncBuilder::new(item(30), s.clone());
        let p = b.param(STRING_REF, NO_ORIGIN);
        let e = FuncBuilder::ENTRY;
        let len_ref = b.op(
            e,
            OpKind::BorrowView {
                root: Root::Value(p),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.vec_field,
                        ty: VEC_U8,
                    },
                    Proj::Field {
                        field: parts.len_field,
                        ty: USIZE,
                    },
                ],
                mutable: false,
            },
            LEN_REF,
            NO_ORIGIN,
        );
        let len_val = b.op(e, OpKind::Deref(len_ref), USIZE, NO_ORIGIN);
        let buf_chain = b.op(
            e,
            OpKind::BorrowView {
                root: Root::Value(p),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.vec_field,
                        ty: VEC_U8,
                    },
                    Proj::Field {
                        field: parts.buf_field,
                        ty: RAW_VEC,
                    },
                ],
                mutable: false,
            },
            PTR_U8_REF,
            NO_ORIGIN,
        );
        let ptr_val = b.op(e, OpKind::Deref(buf_chain), PTR_U8, NO_ORIGIN);
        let slice = b.op(
            e,
            OpKind::SliceView {
                ptr: ptr_val,
                len: len_val,
            },
            REF_SLICE_U8,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(Some(slice)));
        let func = b.finish();
        assert!(classify_as_bytes_body(&func, &table, &interns, &parts));

        let cx = bstr_cx(&s, &table, &interns);
        assert!(legality_as_bytes(&cx).is_ok());

        for (carrier, expected_substr) in [
            (Carrier::HostString, "__rt.bstr_bytes($native0)"),
            (Carrier::StringBuilder, "__rt.bstr_bytes($native0.text)"),
        ] {
            let mut helpers = BTreeSet::new();
            let mut refs = BTreeSet::new();
            let stmts = string_as_bytes(&cx, Some(carrier), &mut helpers, &mut refs)
                .expect("as_bytes lowers");
            let printed =
                jsast::stmt_to_string(&jsast::function("f", vec!["$native0".to_owned()], stmts));
            assert!(printed.contains(expected_substr), "{printed}");
            assert!(helpers.contains("bstr_bytes"), "{helpers:?}");
        }
    }

    #[test]
    fn bstr_stdlib_path_whitelist_and_side_effect_rejection() {
        let (table, interns) = build_table();
        let parts = string_parts(&table, &interns, STRING).expect("canonical String parts");

        // Exact user-defined `reserve_and_count` shape MUST NOT match stdlib path allowlist.
        assert_eq!(
            is_stdlib_string_op_path("my_crate", "my_crate::reserve_and_count"),
            None
        );
        assert_eq!(
            is_stdlib_string_op_path("my_crate", "alloc::string::String::reserve_exact"),
            Some(BstrStringOp::ReserveExact)
        );
        assert_eq!(
            is_stdlib_string_op_path("alloc", "string::String::reserve"),
            Some(BstrStringOp::Reserve)
        );
        assert!(!is_stdlib_str_as_bytes_path(
            "my_crate",
            "my_crate::as_bytes"
        ));
        assert!(is_stdlib_str_as_bytes_path(
            "core",
            "str::<impl str>::as_bytes"
        ));
        assert!(is_stdlib_str_as_bytes_path(
            "my_crate",
            "core::str::<impl str>::as_bytes"
        ));
        assert!(is_stdlib_raw_vec_reserve_path(
            "alloc",
            "raw_vec::RawVecInner::<A>::reserve_exact"
        ));
        assert!(is_stdlib_raw_vec_reserve_path(
            "my_crate",
            "alloc::raw_vec::RawVecInner::<A>::reserve_exact"
        ));
        assert!(!is_stdlib_raw_vec_reserve_path(
            "my_crate",
            "my_crate::RawVecInner::reserve_exact"
        ));

        // Even inside `String::reserve_exact`, any arbitrary non-stdlib call (e.g. atomic counter / panic)
        // or extra store MUST reject reserve classification.
        let s_res = sig(vec![STRING_MUT, USIZE], UNIT);
        let mut b = FuncBuilder::new(item(31), s_res);
        let p = b.param(STRING_MUT, NO_ORIGIN);
        let add = b.param(USIZE, NO_ORIGIN);
        let e = FuncBuilder::ENTRY;
        let len_ref = b.op(
            e,
            OpKind::BorrowView {
                root: Root::Value(p),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.vec_field,
                        ty: VEC_U8,
                    },
                    Proj::Field {
                        field: parts.len_field,
                        ty: USIZE,
                    },
                ],
                mutable: false,
            },
            LEN_REF,
            NO_ORIGIN,
        );
        let len_val = b.op(e, OpKind::Deref(len_ref), USIZE, NO_ORIGIN);
        let _sum = b.op(
            e,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: len_val,
                r: add,
                ty: USIZE,
            },
            USIZE,
            NO_ORIGIN,
        );
        b.effect(
            e,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(902),
                    link_name: crate::jir::interns::StrId(0),
                },
                function: None,
                args: vec![add],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(None));
        let func = b.finish();
        assert_eq!(
            classify_reserve_body(&func, &table, &interns, &parts, &mut |_| None, &mut |id| {
                (id == item(902)).then_some(("my_crate", "my_crate::tick"))
            },),
            None
        );
    }

    #[test]
    fn bstr_classify_real_stdlib_reserve_exact_cfg() {
        let (table, interns) = build_table();
        let parts = string_parts(&table, &interns, STRING).expect("canonical String parts");

        // Multi-block CFG mirroring real `alloc::string::String::reserve_exact`:
        // Entry: `Deref(self) -> Field(vec) -> Field(len)` across `Goto(b1, [len])`
        // b1: `BorrowView(&mut self.vec.buf.inner)` -> `Deref -> Field(cap) -> Cast` across `Goto(b2, [cap])`
        // b2: calls `alloc::raw_vec::RawVecInner::<Global>::finish_grow`, returns `()`.
        let s_res = sig(vec![STRING_MUT, USIZE], UNIT);
        let mut b = FuncBuilder::new(item(32), s_res);
        let p = b.param(STRING_MUT, NO_ORIGIN);
        let add = b.param(USIZE, NO_ORIGIN);
        let e = FuncBuilder::ENTRY;
        let b1 = b.block();
        let b2 = b.block();
        let len_p = b.block_param(b1, USIZE, NO_ORIGIN);
        let cap_p = b.block_param(b2, USIZE, NO_ORIGIN);

        let str_val = b.op(e, OpKind::Deref(p), STRING, NO_ORIGIN);
        let vec_val = b.op(
            e,
            OpKind::Field {
                base: str_val,
                variant: None,
                field: parts.vec_field,
            },
            VEC_U8,
            NO_ORIGIN,
        );
        let len_val = b.op(
            e,
            OpKind::Field {
                base: vec_val,
                variant: None,
                field: parts.len_field,
            },
            USIZE,
            NO_ORIGIN,
        );
        let inner_ref = b.op(
            e,
            OpKind::BorrowView {
                root: Root::Value(p),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.vec_field,
                        ty: VEC_U8,
                    },
                    Proj::Field {
                        field: parts.buf_field,
                        ty: RAW_VEC,
                    },
                ],
                mutable: true,
            },
            BUF_REF,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Goto(b1, vec![len_val]));

        let raw_val = b.op(b1, OpKind::Deref(inner_ref), RAW_VEC, NO_ORIGIN);
        let cap_val = b.op(
            b1,
            OpKind::Field {
                base: raw_val,
                variant: None,
                field: 0,
            },
            USIZE,
            NO_ORIGIN,
        );
        let cap_cast = b.op(
            b1,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: cap_val,
                from: USIZE,
                to: USIZE,
            },
            USIZE,
            NO_ORIGIN,
        );
        b.terminate(b1, RTerm::Goto(b2, vec![cap_cast]));

        let _need = b.op(
            b2,
            OpKind::Binary {
                op: BinaryOp::Sub,
                l: cap_p,
                r: len_p,
                ty: USIZE,
            },
            USIZE,
            NO_ORIGIN,
        );
        b.effect(
            b2,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(903),
                    link_name: crate::jir::interns::StrId(0),
                },
                function: None,
                args: vec![inner_ref, len_p, add],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
        b.terminate(b2, RTerm::Return(None));

        let func = b.finish();
        assert_eq!(
            classify_reserve_body(&func, &table, &interns, &parts, &mut |_| None, &mut |id| {
                (id == item(903)).then_some((
                    "alloc",
                    "alloc::raw_vec::RawVecInner::<Global>::finish_grow",
                ))
            },),
            Some(BstrStringOp::ReserveExact)
        );
    }

    #[test]
    fn bstr_analyze_inlined_raw_vec_reserve_exact() {
        let (table, interns) = build_table();
        let parts = string_parts(&table, &interns, STRING).expect("canonical String parts");

        let mut b = FuncBuilder::new(item(33), sig(vec![USIZE], UNIT));
        let add = b.param(USIZE, NO_ORIGIN);
        let s_cell = b.cell(STRING, false, NO_ORIGIN);
        let e = FuncBuilder::ENTRY;

        let s_mut = b.op(
            e,
            OpKind::BorrowView {
                root: Root::Cell(s_cell),
                path: Vec::new(),
                mutable: true,
            },
            STRING_MUT,
            NO_ORIGIN,
        );
        let str_val = b.op(e, OpKind::Deref(s_mut), STRING, NO_ORIGIN);
        let vec_val = b.op(
            e,
            OpKind::Field {
                base: str_val,
                variant: None,
                field: parts.vec_field,
            },
            VEC_U8,
            NO_ORIGIN,
        );
        let len_val = b.op(
            e,
            OpKind::Field {
                base: vec_val,
                variant: None,
                field: parts.len_field,
            },
            USIZE,
            NO_ORIGIN,
        );
        let buf_ref = b.op(
            e,
            OpKind::BorrowView {
                root: Root::Value(s_mut),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.vec_field,
                        ty: VEC_U8,
                    },
                    Proj::Field {
                        field: parts.buf_field,
                        ty: RAW_VEC,
                    },
                ],
                mutable: true,
            },
            BUF_REF,
            NO_ORIGIN,
        );
        let layout_const = b.op(
            e,
            OpKind::Const(ConstTree::Aggregate(vec![
                ConstTree::Scalar { bits: 1, bytes: 4 },
                ConstTree::Variant {
                    variant: 0,
                    fields: Vec::new(),
                },
            ])),
            USIZE,
            NO_ORIGIN,
        );
        b.effect(
            e,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(904),
                    link_name: crate::jir::interns::StrId(0),
                },
                function: None,
                args: vec![buf_ref, len_val, add, layout_const],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(None));

        let func = b.finish();
        let analysis =
            analyze_inlined_string_vec_inner(&func, &table, &interns, |id, _| id == item(904))
                .expect("recognizes MIR-inlined RawVecInner::reserve_exact on String Cell");
        assert_eq!(analysis.clusters.len(), 1);
        let c = &analysis.clusters[0];
        assert_eq!(c.root, StringRoot::Cell(s_cell));
        assert!(!c.root_is_ref);
        assert!(c.is_mut);
        assert!(c.len_values.contains(&len_val));
        assert!(c.internal_values.contains(&buf_ref));
        assert!(c.internal_values.contains(&str_val));
        assert_eq!(c.reserve_calls.get(&(0, 6)), Some(&add));
    }

    #[test]
    fn inlined_string_vec_rejects_mutable_slice_view() {
        let (mut table, interns) = build_table();
        let mut_slice_u8 = TypeId(99);
        table.insert(ref_desc(
            mut_slice_u8,
            SLICE_U8,
            true,
            PointerForm::FatSlice,
        ));
        let parts = string_parts(&table, &interns, STRING).expect("parts");

        let mut b = FuncBuilder::new(item(34), sig(vec![STRING_MUT], mut_slice_u8));
        let s_mut = b.param(STRING_MUT, NO_ORIGIN);
        let e = FuncBuilder::ENTRY;
        let vec_mut = b.op(
            e,
            OpKind::BorrowView {
                root: Root::Value(s_mut),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.vec_field,
                        ty: VEC_U8,
                    },
                ],
                mutable: true,
            },
            VEC_U8_MUT,
            NO_ORIGIN,
        );
        let len_ref = b.op(
            e,
            OpKind::BorrowView {
                root: Root::Value(vec_mut),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.len_field,
                        ty: USIZE,
                    },
                ],
                mutable: false,
            },
            LEN_REF,
            NO_ORIGIN,
        );
        let len_val = b.op(e, OpKind::Deref(len_ref), USIZE, NO_ORIGIN);
        let buf_ref = b.op(
            e,
            OpKind::BorrowView {
                root: Root::Value(vec_mut),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: parts.buf_field,
                        ty: RAW_VEC,
                    },
                ],
                mutable: true,
            },
            BUF_REF,
            NO_ORIGIN,
        );
        let slice = b.op(
            e,
            OpKind::SliceView {
                ptr: buf_ref,
                len: len_val,
            },
            mut_slice_u8,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(Some(slice)));

        let func = b.finish();
        assert!(
            analyze_inlined_string_vec_inner(&func, &table, &interns, |_, _| false).is_none(),
            "inlined mutable SliceView on String.vec must disqualify StringBuilder/HostString"
        );
    }
}
