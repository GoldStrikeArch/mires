//! The slice family: `<[T]>::swap`, over the shared `{ buf, off, len }` view shape.
//!
//! A port of v1 `native_std_lower.rs`'s `SliceSwap` arm, spelling for spelling. The receiver is a
//! fat slice — the baseline spelling and D5's [`Carrier::SliceView`] are the same record, which
//! is why the operation carries no slot requirement. Both bounds throw before either element
//! moves, so a panicking swap mutates nothing.

use std::collections::BTreeSet;

use crate::item::JsName;
use crate::jir::repr::Carrier;
use crate::jir::types::{TypeDescriptor, TypeKind};
use crate::jsast::{self, BinOp, Stmt};

use super::string::member;
use super::{slot_type, type_get, LowerCx, SemanticDecline, SignatureSlot};

pub(super) fn slice_swap(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    slice_element(cx)?;
    let buffer = member("buf");
    let offset = member("off");
    Ok(vec![
        bounds_guard("$native1"),
        bounds_guard("$native2"),
        jsast::const_(
            "$nativeLeft",
            jsast::binary(BinOp::Add, offset.clone(), jsast::id("$native1")),
        ),
        jsast::const_(
            "$nativeRight",
            jsast::binary(BinOp::Add, offset, jsast::id("$native2")),
        ),
        jsast::const_(
            "$nativeValue",
            jsast::index(buffer.clone(), jsast::id("$nativeLeft")),
        ),
        jsast::assign_stmt(
            jsast::index(buffer.clone(), jsast::id("$nativeLeft")),
            jsast::index(buffer.clone(), jsast::id("$nativeRight")),
        ),
        jsast::assign_stmt(
            jsast::index(buffer, jsast::id("$nativeRight")),
            jsast::id("$nativeValue"),
        ),
        jsast::ret_void(),
    ])
}

fn bounds_guard(index: &str) -> Stmt {
    jsast::if_(
        jsast::binary(
            BinOp::Ge,
            jsast::id(index),
            jsast::member(jsast::id("$native0"), "len"),
        ),
        vec![jsast::throw(jsast::new(
            jsast::id("RangeError"),
            vec![jsast::string(
                "rustc_codegen_js: native slice swap index out of bounds",
            )],
        ))],
    )
}

/// Both indices are `usize`, the receiver is `&mut [T]`, and the return is unit (v1
/// `validate_argument_types` + `validate_return` for `SliceSwap`).
pub(super) fn legality_swap(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    slice_element(cx)?;
    for index in 1..=2 {
        let argument = slot_type(cx.sig, SignatureSlot::Argument(index))?;
        if !matches!(
            type_get(cx.table, argument)?.kind,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            }
        ) {
            return Err(SemanticDecline::WrongArgumentType(index));
        }
    }
    let result = type_get(cx.table, cx.sig.result)?;
    if !matches!(&result.kind, TypeKind::Tuple(fields) if fields.is_empty()) {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

/// The receiver's sized, non-zero-sized element (v1 `slice_element`).
fn slice_element<'a>(cx: &LowerCx<'a>) -> Result<&'a TypeDescriptor, SemanticDecline> {
    let receiver = cx
        .sig
        .arguments
        .first()
        .copied()
        .ok_or(SemanticDecline::WrongArgumentType(0))?;
    let TypeKind::Reference {
        pointee,
        mutable: true,
    } = type_get(cx.table, receiver)?.kind
    else {
        return Err(SemanticDecline::WrongReferenceKind(
            SignatureSlot::Argument(0),
        ));
    };
    let TypeKind::Slice(element) = type_get(cx.table, pointee)?.kind else {
        return Err(SemanticDecline::WrongArgumentType(0));
    };
    let element = type_get(cx.table, element)?;
    if !element.layout.sized {
        return Err(SemanticDecline::UnknownElementLayout);
    }
    if element.layout.zst {
        return Err(SemanticDecline::ZeroSizedElement);
    }
    Ok(element)
}
