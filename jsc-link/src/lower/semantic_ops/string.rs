//! The `String` family emitters: construction, length, append, clear, borrow, drop.
//!
//! Ports of v1 `native_std_lower.rs`'s string arms, spelling for spelling. The carriers are D5's:
//! [`Carrier::HostString`] is an immutable JavaScript string; [`Carrier::StringBuilder`] is
//! `{ text, cap }` with the capacity in UTF-8 bytes. Every capacity or bounds throw precedes
//! every mutation.

use std::collections::BTreeSet;

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

/// `{ text: "", cap: <capacity> }` — the mutable carrier's empty value.
fn native_string(capacity: Expr) -> Expr {
    jsast::object_of(vec![("text", jsast::string("")), ("cap", capacity)])
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

pub(super) fn string_len(
    _cx: &LowerCx<'_>,
    carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    helpers.insert("str_len");
    let text = match carrier {
        Some(Carrier::HostString) => receiver(),
        Some(Carrier::StringBuilder) => member("text"),
        _ => return Err(SemanticDecline::WrongCarrier(SignatureSlot::Argument(0))),
    };
    Ok(vec![jsast::ret(jsast::rt_call("str_len", vec![text]))])
}

pub(super) fn string_capacity(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![jsast::ret(member("cap"))])
}

/// `String::push` and `fmt::Write::write_char`: byte width of the scalar, capacity guard,
/// character-boundary-exact append through `__rt.native_str_push`.
pub(super) fn string_push(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let maximum = string_capacity_limit(cx)?;
    helpers.insert("_utf8_width");
    helpers.insert("native_str_push");
    helpers.insert("str_len");
    Ok(vec![
        jsast::const_(
            "$nativeRequired",
            jsast::binary(
                BinOp::Add,
                jsast::rt_call("str_len", vec![member("text")]),
                jsast::rt_call("_utf8_width", vec![jsast::id("$native1")]),
            ),
        ),
        capacity_guard(jsast::id("$nativeRequired"), maximum, "String"),
        jsast::if_(
            jsast::binary(BinOp::Lt, member("cap"), jsast::id("$nativeRequired")),
            vec![jsast::assign_stmt(
                member("cap"),
                jsast::id("$nativeRequired"),
            )],
        ),
        jsast::expr_stmt(jsast::rt_call(
            "native_str_push",
            vec![receiver(), jsast::id("$native1")],
        )),
        append_return(cx)?,
    ])
}

/// `String::push_str` and `fmt::Write::write_str`: two UTF-8 lengths, capacity guard, `+=`.
pub(super) fn string_push_str(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let maximum = string_capacity_limit(cx)?;
    helpers.insert("str_len");
    Ok(vec![
        jsast::const_(
            "$nativeRequired",
            jsast::binary(
                BinOp::Add,
                jsast::rt_call("str_len", vec![member("text")]),
                jsast::rt_call("str_len", vec![jsast::id("$native1")]),
            ),
        ),
        capacity_guard(jsast::id("$nativeRequired"), maximum, "String"),
        jsast::if_(
            jsast::binary(BinOp::Lt, member("cap"), jsast::id("$nativeRequired")),
            vec![jsast::assign_stmt(
                member("cap"),
                jsast::id("$nativeRequired"),
            )],
        ),
        jsast::expr_stmt(jsast::compound_assign(
            BinOp::Add,
            member("text"),
            jsast::id("$native1"),
        )),
        append_return(cx)?,
    ])
}

pub(super) fn string_clear(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![
        jsast::assign_stmt(member("text"), jsast::string("")),
        jsast::ret_void(),
    ])
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

/// The mutable carrier's drop: release the text, zero the capacity. A `HostString` never has a
/// drop body — an immutable string owns nothing.
pub(super) fn string_drop(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![
        jsast::assign_stmt(member("text"), jsast::string("")),
        jsast::assign_stmt(member("cap"), jsast::num(0)),
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
