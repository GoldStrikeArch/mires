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

use std::collections::BTreeSet;
use std::sync::OnceLock;

use crate::item::JsName;
use crate::jir::repr::Carrier;
use crate::jir::semantic::StandardOperation;
use crate::jir::types::TypeKind;
use crate::jsast::{self, BinOp, Expr, Stmt};

use super::{
    capacity_guard, is_shared_str, pointer_limit, slot_type, string_capacity_limit, type_get,
    unsigned_bits, LowerCx, SemanticDecline, SignatureSlot,
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
