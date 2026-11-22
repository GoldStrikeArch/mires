//! The format-machinery emitters: `ToString` for the primitives, the shortest-`f64` writer, and
//! the three `fmt::Arguments` identities the native format pipeline is built from.
//!
//! Ports of v1 `native_std_lower.rs`'s format arms. A native `fmt::Arguments` is the concatenated
//! host string; call-site kernel inlining (wave 2 of the planner) supplies the static pieces, and
//! these bodies exist so a DCE'd constructor stays a proven standard operation.

use std::collections::BTreeSet;

use crate::item::JsName;
use crate::jir::repr::Carrier;
use crate::jir::semantic::StandardOperation;
use crate::jir::types::{RepresentationCandidate, TypeId, TypeKind};
use crate::jsast::{self, BinOp, Expr, Stmt};

use super::string::receiver;
use super::{slot_type, type_get, LowerCx, SemanticDecline, SignatureSlot};

/// `f64::to_string`: the exact Rust-compatible shortest decimal, already `{ text, cap, len }`.
pub(super) fn f64_spec_to_string(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    helpers.insert("native_f64_to_string");
    Ok(vec![jsast::ret(jsast::rt_call(
        "native_f64_to_string",
        vec![super::slot_load(receiver())],
    ))])
}

/// `ToString` for the fixed-width integers, `bool` and `char` (v1's one arm for all three).
pub(super) fn primitive_to_string(
    cx: &LowerCx<'_>,
    carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let operation = match cx.semantic {
        crate::jir::semantic::BodySemantic::Standard(operation) => operation,
        _ => return Err(SemanticDecline::NotAStandardOperation),
    };
    let value = super::slot_load(receiver());
    // `String` of a JavaScript number or BigInt is the plain decimal Rust `Display` writes, the
    // ternary spells `bool` without depending on the boolean's JavaScript value, and a `char` is
    // a code point in the value model rather than a string.
    let text = match operation {
        StandardOperation::IntToString => jsast::call(jsast::id("String"), vec![value]),
        StandardOperation::BoolToString => {
            jsast::cond(value, jsast::string("true"), jsast::string("false"))
        }
        _ => crate::lower::bstr::char_text(value),
    };
    match carrier {
        Some(Carrier::HostString) => Ok(vec![jsast::ret(text)]),
        Some(Carrier::StringBuilder) => {
            // A decimal integer and both boolean words are ASCII, so their UTF-16 length is their
            // UTF-8 byte length, which is both the builder's length and its exact-fit capacity.
            // A `char` needs the exact encoded width.
            let length = if operation == StandardOperation::CharToString {
                helpers.insert("str_len");
                jsast::rt_call("str_len", vec![jsast::id("$nativeText")])
            } else {
                jsast::member(jsast::id("$nativeText"), "length")
            };
            Ok(vec![
                jsast::const_("$nativeText", text),
                jsast::const_("$nativeLen", length),
                jsast::ret(super::string::exact_builder(
                    jsast::id("$nativeText"),
                    jsast::id("$nativeLen"),
                )),
            ])
        }
        _ => Err(SemanticDecline::WrongCarrier(SignatureSlot::Result)),
    }
}

/// `Argument::new_display` for a proven default-Display value: the argument's host text.
pub(super) fn format_argument_new_display(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![jsast::ret(format_display_text(cx, helpers)?)])
}

/// `Argument::new_lower_hex` / `Argument::new_upper_hex` for a proven integer value: the
/// argument's host hex text. Call-site kernel inlining supplies the placeholder flags/width;
/// this body exists so a DCE'd constructor stays a proven standard operation.
pub(super) fn format_argument_new_hex(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let upper = matches!(
        cx.semantic,
        crate::jir::semantic::BodySemantic::Standard(StandardOperation::FormatArgumentNewUpperHex)
    );
    let argument = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    let bits = format_hex_bits(cx, argument)?;
    helpers.insert("fmt_int_hex");
    Ok(vec![jsast::ret(jsast::rt_call(
        "fmt_int_hex",
        vec![
            super::slot_load(receiver()),
            jsast::num(f64::from(bits)),
            jsast::boolean(upper),
            jsast::num(0),
            jsast::boolean(false),
            jsast::boolean(false),
        ],
    ))])
}

/// Native `Arguments` is the concatenated host string. Call-site kernel inlining supplies the
/// static pieces; this body exists so a DCE'd `Arguments::new` stays a proven standard operation.
pub(super) fn format_args_new(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![jsast::ret(jsast::string(""))])
}

/// The exact `fn(Arguments) -> String`: `$native0` is already the native `Arguments` host string.
pub(super) fn format_args_to_string(
    _cx: &LowerCx<'_>,
    carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    owned_native_string(receiver(), carrier, helpers)
}

/// The native `String::new`/`with_capacity` layout the format sinks share: an immutable result is
/// the host string, a mutable one is the exact-fit `{ text, cap, len }`, its capacity and length
/// the text's one UTF-8 measurement. The native `Arguments` text is already the text the
/// baseline `String` holds: each of its pieces is well formed on its own (a host `&str` argument
/// is taken as Rust holds it where it is displayed, `format_display_text`).
pub(super) fn owned_native_string(
    text: Expr,
    carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    match carrier {
        Some(Carrier::HostString) => Ok(vec![jsast::ret(text)]),
        Some(Carrier::StringBuilder) => {
            helpers.insert("str_len");
            Ok(vec![
                jsast::const_("$nativeText", text),
                jsast::const_(
                    "$nativeLen",
                    jsast::rt_call("str_len", vec![jsast::id("$nativeText")]),
                ),
                jsast::ret(super::string::exact_builder(
                    jsast::id("$nativeText"),
                    jsast::id("$nativeLen"),
                )),
            ])
        }
        _ => Err(SemanticDecline::WrongCarrier(SignatureSlot::Result)),
    }
}

/// The default-Display argument shapes the native format pipeline recognizes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FormatDisplayKind {
    Integer,
    Bool,
    Char,
    Str,
    String,
    F64,
    F32,
}

fn format_display_kind(
    cx: &LowerCx<'_>,
    argument: TypeId,
) -> Result<FormatDisplayKind, SemanticDecline> {
    let mut ty = argument;
    let mut saw_reference = false;
    loop {
        let descriptor = type_get(cx.table, ty)?;
        match &descriptor.kind {
            TypeKind::Reference {
                pointee,
                mutable: false,
            } => {
                saw_reference = true;
                ty = *pointee;
            }
            TypeKind::Integer { bits, .. }
                if saw_reference && matches!(bits, 8 | 16 | 32 | 64 | 128) =>
            {
                return Ok(FormatDisplayKind::Integer);
            }
            TypeKind::Bool if saw_reference => return Ok(FormatDisplayKind::Bool),
            TypeKind::Char if saw_reference => return Ok(FormatDisplayKind::Char),
            TypeKind::Str if saw_reference => return Ok(FormatDisplayKind::Str),
            TypeKind::Float { bits: 64 } if saw_reference => return Ok(FormatDisplayKind::F64),
            TypeKind::Float { bits: 32 } if saw_reference => return Ok(FormatDisplayKind::F32),
            TypeKind::Adt { .. }
                if saw_reference
                    && descriptor.meta.candidate == RepresentationCandidate::String =>
            {
                return Ok(FormatDisplayKind::String);
            }
            _ => return Err(SemanticDecline::WrongArgumentType(0)),
        }
    }
}

fn format_hex_bits(cx: &LowerCx<'_>, argument: TypeId) -> Result<u32, SemanticDecline> {
    let mut ty = argument;
    let mut saw_reference = false;
    loop {
        let descriptor = type_get(cx.table, ty)?;
        match &descriptor.kind {
            TypeKind::Reference {
                pointee,
                mutable: false,
            } => {
                saw_reference = true;
                ty = *pointee;
            }
            TypeKind::Integer { bits, .. }
                if saw_reference && matches!(bits, 8 | 16 | 32 | 64 | 128) =>
            {
                return Ok(*bits);
            }
            _ => return Err(SemanticDecline::WrongArgumentType(0)),
        }
    }
}

/// The host text of a display argument loaded through its `&T` slot.
fn format_display_text(
    cx: &LowerCx<'_>,
    helpers: &mut BTreeSet<&'static str>,
) -> Result<Expr, SemanticDecline> {
    let argument = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    let kind = format_display_kind(cx, argument)?;
    let loaded = super::slot_load(receiver());
    Ok(match kind {
        FormatDisplayKind::Integer => jsast::call(jsast::id("String"), vec![loaded]),
        FormatDisplayKind::Bool => {
            jsast::cond(loaded, jsast::string("true"), jsast::string("false"))
        }
        FormatDisplayKind::Char => crate::lower::bstr::char_text(loaded),
        // The one display text a host may have made: taken as Rust holds it (lone surrogate to
        // U+FFFD), so the concatenated `Arguments` text is well formed piece by piece.
        FormatDisplayKind::Str => super::string::well_formed(receiver(), helpers),
        // A committed `String` argument may arrive as either string carrier; the typeof test
        // reads whichever it is without a plan-time distinction (v1's exact ternary).
        FormatDisplayKind::String => jsast::cond(
            jsast::binary(
                BinOp::StrictEq,
                jsast::type_of(loaded.clone()),
                jsast::string("string"),
            ),
            loaded.clone(),
            jsast::member(loaded, "text"),
        ),
        FormatDisplayKind::F64 => {
            helpers.insert("native_f64_to_string");
            jsast::member(jsast::rt_call("native_f64_to_string", vec![loaded]), "text")
        }
        FormatDisplayKind::F32 => {
            helpers.insert("native_f32_to_string");
            jsast::member(jsast::rt_call("native_f32_to_string", vec![loaded]), "text")
        }
    })
}

/// Concatenate static template pieces with already-lowered argument texts (the call-site kernel's
/// runtime, shared here so wave 2 has one spelling).
pub fn concat_format_pieces(pieces: &[String], texts: &[Expr]) -> Expr {
    let mut parts = Vec::new();
    for (index, piece) in pieces.iter().enumerate() {
        if !piece.is_empty() {
            parts.push(crate::lower::bstr::literal(piece));
        }
        if let Some(text) = texts.get(index) {
            parts.push(text.clone());
        }
    }
    parts
        .into_iter()
        .reduce(|left, right| jsast::binary(BinOp::Add, left, right))
        .unwrap_or_else(|| jsast::string(""))
}

/// Proves the return type is the canonical `Result<(), fmt::Error>` and spells its `Ok` value
/// from the descriptor (`{ TAG: "Ok", _0: undefined }` in the baseline tag encoding).
pub(super) fn write_ok_value(cx: &LowerCx<'_>) -> Result<Expr, SemanticDecline> {
    write_ok_expr(cx.table, cx.interns, cx.sig.result)
}

/// [`write_ok_value`] against a bare type: what the call-site format kernel spells a proven
/// `write_fmt` site's result with (`fmt_kernel.rs`).
pub(crate) fn write_ok_expr(
    table: &crate::jir::types::TypeTable,
    interns: &crate::jir::interns::Interns,
    result: TypeId,
) -> Result<Expr, SemanticDecline> {
    let (tag, key) = write_ok_variant(table, interns, result)?;
    Ok(jsast::object(vec![
        ("TAG".to_owned(), jsast::string(tag)),
        (key, jsast::undefined()),
    ]))
}

/// The canonical `Result<(), fmt::Error>` shape check (v1 `fmt_write_ok_variant`), returning the
/// `Ok` variant's tag literal and its one field's JavaScript key.
fn fmt_write_ok_variant(cx: &LowerCx<'_>) -> Result<(String, String), SemanticDecline> {
    write_ok_variant(cx.table, cx.interns, cx.sig.result)
}

fn write_ok_variant(
    table: &crate::jir::types::TypeTable,
    interns: &crate::jir::interns::Interns,
    result_ty: TypeId,
) -> Result<(String, String), SemanticDecline> {
    let result = type_get(table, result_ty)?;
    let TypeKind::Adt {
        args,
        form: crate::jir::types::NominalForm::Enum,
        variants,
        ..
    } = &result.kind
    else {
        return Err(SemanticDecline::WrongReturnType);
    };
    if args.len() != 2 || variants.len() != 2 {
        return Err(SemanticDecline::WrongReturnType);
    }
    let [ok, error] = variants.as_slice() else {
        return Err(SemanticDecline::WrongReturnType);
    };
    let (Some(ok_tag), Some(error_tag)) = (ok.physical_tag, error.physical_tag) else {
        return Err(SemanticDecline::WrongReturnType);
    };
    let unit = type_get(table, args[0])?;
    let fmt_error = type_get(table, args[1])?;
    let field_tys = |variant: &crate::jir::types::VariantDescriptor| -> Vec<TypeId> {
        variant.fields.iter().map(|field| field.ty).collect()
    };
    if !ok.result_ok
        || error.result_ok
        || ok_tag.bits != 0
        || error_tag.bits != 1
        || ok_tag.bytes != error_tag.bytes
        || field_tys(ok) != [args[0]]
        || field_tys(error) != [args[1]]
        || !matches!(&unit.kind, TypeKind::Tuple(fields) if fields.is_empty())
        || !unit.layout.zst
        || !matches!(
            &fmt_error.kind,
            TypeKind::Adt {
                form: crate::jir::types::NominalForm::Struct,
                variants,
                ..
            } if variants.len() == 1 && variants[0].fields.is_empty()
        )
        || !fmt_error.layout.zst
        || !result.layout.sized
        || result.layout.zst
    {
        return Err(SemanticDecline::WrongReturnType);
    }
    let tag = interns
        .str(ok.name)
        .ok_or(SemanticDecline::WrongReturnType)?
        .to_owned();
    let key = match ok.fields[0].key {
        crate::jir::types::FieldKey::Name(id) => interns
            .str(id)
            .ok_or(SemanticDecline::WrongReturnType)?
            .to_owned(),
        crate::jir::types::FieldKey::Index(index) => format!("_{index}"),
        crate::jir::types::FieldKey::Transparent => {
            return Err(SemanticDecline::WrongReturnType);
        }
    };
    Ok((tag, key))
}

// The operation-specific legality arms.

pub(super) fn legality_fmt_write_str(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let argument = slot_type(cx.sig, SignatureSlot::Argument(1))?;
    if !super::is_shared_str(cx.table, argument) {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    fmt_write_ok_variant(cx).map(|_| ())
}

pub(super) fn legality_fmt_write_char(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let argument = slot_type(cx.sig, SignatureSlot::Argument(1))?;
    if !matches!(type_get(cx.table, argument)?.kind, TypeKind::Char) {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    fmt_write_ok_variant(cx).map(|_| ())
}

/// `&f64 -> String`, with the receiver behind a shared reference.
pub(super) fn legality_f64_to_string(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    shared_pointee_matches(cx, |kind| matches!(kind, TypeKind::Float { bits: 64 }))?;
    legality_string_return(cx)
}

pub(super) fn legality_int_to_string(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    shared_pointee_matches(
        cx,
        |kind| matches!(kind, TypeKind::Integer { bits, .. } if matches!(bits, 8 | 16 | 32 | 64 | 128)),
    )?;
    legality_string_return(cx)
}

pub(super) fn legality_bool_to_string(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    shared_pointee_matches(cx, |kind| matches!(kind, TypeKind::Bool))?;
    legality_string_return(cx)
}

pub(super) fn legality_char_to_string(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    shared_pointee_matches(cx, |kind| matches!(kind, TypeKind::Char))?;
    legality_string_return(cx)
}

pub(super) fn legality_argument_new_display(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let argument = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    format_display_kind(cx, argument)?;
    let result = type_get(cx.table, cx.sig.result)?;
    if !result.layout.sized || result.layout.zst {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

pub(super) fn legality_argument_new_hex(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let argument = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    format_hex_bits(cx, argument)?;
    let result = type_get(cx.table, cx.sig.result)?;
    if !result.layout.sized || result.layout.zst {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

pub(super) fn legality_format_args_new(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    for index in 0..2u16 {
        let argument = slot_type(cx.sig, SignatureSlot::Argument(index))?;
        if !matches!(
            type_get(cx.table, argument)?.kind,
            TypeKind::Reference { mutable: false, .. }
        ) {
            return Err(SemanticDecline::WrongArgumentType(index));
        }
    }
    let result = type_get(cx.table, cx.sig.result)?;
    if !result.layout.sized || result.layout.zst {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

/// The result is the canonical global-allocator `String`.
pub(super) fn legality_string_return(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    super::string::legality_string_candidate_return(cx)
}

fn shared_pointee_matches(
    cx: &LowerCx<'_>,
    expected: impl Fn(&TypeKind) -> bool,
) -> Result<(), SemanticDecline> {
    let argument = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    let TypeKind::Reference {
        pointee,
        mutable: false,
    } = type_get(cx.table, argument)?.kind
    else {
        return Err(SemanticDecline::WrongArgumentType(0));
    };
    if !expected(&type_get(cx.table, pointee)?.kind) {
        return Err(SemanticDecline::WrongArgumentType(0));
    }
    Ok(())
}
