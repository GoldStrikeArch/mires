//! The `BTreeMap` family emitters: `new`, `len`, `is_empty`, `clear`, `contains_key`, `get`,
//! `get_mut`, `insert`, `remove`, and `drop`.
//!
//! The carrier is [`Carrier::NativeMap`]: a plain JavaScript `Map` object. Only order-unobserved,
//! default-allocator `BTreeMap<K, V>` values whose key type is a Rust built-in primitive
//! (`bool`, `char`, integer up to 64 bits), `&str`, or `String`, and whose value type has no
//! user-defined `Drop` implementation anywhere inside it, are admitted. Any map that observes
//! `Ord` order (`iter`, `keys`, `values`, `range`, `first_*`, `last_*`, `pop_*`, `into_iter`,
//! `Debug`, `PartialEq`, `Ord`, `Clone`, `Entry`, etc.) is not in the registry and stays on
//! baseline `alloc::collections::BTreeMap`.
//!
//! Direct (non-indirect) values `V` are stored inside a 1-element slot array `[v]` so that
//! `get` / `get_mut` can return `{ buf: box, off: 0 }` with stable address identity and in-place
//! mutability (`*map.get_mut(&k).unwrap() = new_v`). Indirect values `V` are stored directly in
//! the `Map` since `&V` / `&mut V` is already the object itself.

use std::collections::BTreeSet;

use crate::item::JsName;
use crate::jir::repr::Carrier;
use crate::jir::semantic::{BodySemantic, StandardOperation};
use crate::jir::types::{
    AllocatorIdentity, EnumRepr, FieldKey, NominalAbi, NominalForm, PinnedBoundary,
    RepresentationCandidate, TypeDescriptor, TypeId, TypeKind, TypeTable,
};
use crate::jsast::{self, BinOp, Expr, Stmt, UnOp};
use crate::lower::js::adapters::StringLayout;
use crate::ptr;

use super::drop::element_drop_is_trivially_nonthrowing;
use super::string::receiver;
use super::{slot_type, type_get, unsigned_bits, LowerCx, SemanticDecline, SignatureSlot};

enum KeyKind {
    /// Primitive key (`bool`, `char`, integer <= 64 bits).
    Primitive,
    /// `&str` key (always a plain JS string in both UTF-16 and `js-str=bytes` modes).
    StrRef,
    /// Canonical global `String` key (may arrive as `HostString`, `StringBuilder`, or `Baseline`).
    OwnedString,
}

enum KeyLowering {
    /// Argument is already the JS Map key (`$native1`).
    Direct,
    /// Argument is a `{ buf, off }` slot reference to a primitive or `&str` (`$native1.buf[$native1.off]`).
    SlotDeref,
    /// Argument is a universal native `{ s, cap }` `String` or `&String` (`$native1.s`).
    NativeString,
    /// Argument is a `String` or `&String` whose carrier may be `HostString`, `StringBuilder`, or `Baseline`.
    StringCoerce(StringLayout),
}

fn classify_key(table: &TypeTable, desc: &TypeDescriptor) -> Option<KeyKind> {
    if !desc.layout.sized || desc.layout.zst {
        return None;
    }
    match desc.kind {
        TypeKind::Bool | TypeKind::Char => Some(KeyKind::Primitive),
        TypeKind::Integer { bits, .. } if bits <= 64 => Some(KeyKind::Primitive),
        TypeKind::Reference {
            pointee,
            mutable: false,
        } if table
            .get(table.peeled(pointee))
            .is_some_and(|p| p.kind == TypeKind::Str) =>
        {
            Some(KeyKind::StrRef)
        }
        _ if desc.meta.candidate == RepresentationCandidate::String
            && desc.meta.allocator == AllocatorIdentity::Global
            && !desc.layout.has_own_drop =>
        {
            Some(KeyKind::OwnedString)
        }
        _ => None,
    }
}

fn map_kv<'a>(
    cx: &LowerCx<'a>,
) -> Result<(&'a TypeDescriptor, &'a TypeDescriptor), SemanticDecline> {
    let mut map_ty = match cx.semantic {
        BodySemantic::DropGlue(dropped) => dropped,
        BodySemantic::Standard(StandardOperation::BTreeMapNew) => cx.sig.result,
        BodySemantic::Standard(
            StandardOperation::BTreeMapLen
            | StandardOperation::BTreeMapIsEmpty
            | StandardOperation::BTreeMapClear
            | StandardOperation::BTreeMapContainsKey
            | StandardOperation::BTreeMapGet
            | StandardOperation::BTreeMapGetMut
            | StandardOperation::BTreeMapInsert
            | StandardOperation::BTreeMapRemove
            | StandardOperation::BTreeMapDrop,
        ) => cx
            .sig
            .arguments
            .first()
            .copied()
            .ok_or(SemanticDecline::WrongArgumentType(0))?,
        BodySemantic::Standard(op) => return Err(SemanticDecline::UnsupportedOperation(op)),
        BodySemantic::Ordinary | BodySemantic::PanicImpl => {
            return Err(SemanticDecline::NotAStandardOperation);
        }
    };
    if let TypeKind::Reference { pointee, .. } = type_get(cx.table, map_ty)?.kind {
        map_ty = pointee;
    }
    let map_desc = type_get(cx.table, map_ty)?;
    if map_desc.meta.candidate != RepresentationCandidate::BTreeMap
        || map_desc.meta.abi != NominalAbi::Rust
        || map_desc.meta.allocator != AllocatorIdentity::Global
        || map_desc.meta.boundary != PinnedBoundary::None
        || !map_desc.layout.sized
        || map_desc.layout.zst
    {
        return Err(SemanticDecline::WrongNativeType(SignatureSlot::Argument(0)));
    }
    let TypeKind::Adt { args, .. } = &map_desc.kind else {
        return Err(SemanticDecline::WrongNativeType(SignatureSlot::Argument(0)));
    };
    let (Some(&k_ty), Some(&v_ty)) = (args.first(), args.get(1)) else {
        return Err(SemanticDecline::UnknownElementLayout);
    };
    let k_desc = type_get(cx.table, cx.table.peeled(k_ty))?;
    if classify_key(cx.table, k_desc).is_none() {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    let v_desc = type_get(cx.table, cx.table.peeled(v_ty))?;
    if !v_desc.layout.sized || v_desc.layout.zst {
        return Err(SemanticDecline::ZeroSizedElement);
    }
    if v_desc.layout.size.is_none() || v_desc.layout.align.is_none() {
        return Err(SemanticDecline::UnknownElementLayout);
    }
    if !element_drop_is_trivially_nonthrowing(cx.table, v_desc.id) {
        return Err(SemanticDecline::ElementHasOwnDrop);
    }
    Ok((k_desc, v_desc))
}

fn owned_key_lowering(
    cx: &LowerCx<'_>,
    k_desc: &TypeDescriptor,
) -> Result<KeyLowering, SemanticDecline> {
    let arg_ty = cx
        .table
        .peeled(slot_type(cx.sig, SignatureSlot::Argument(1))?);
    if arg_ty != k_desc.id {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    match classify_key(cx.table, k_desc) {
        Some(KeyKind::Primitive | KeyKind::StrRef) => Ok(KeyLowering::Direct),
        Some(KeyKind::OwnedString) => {
            if cx.native_string_enabled {
                Ok(KeyLowering::NativeString)
            } else {
                let layout = StringLayout::of(cx.table, cx.interns, k_desc.id)
                    .map_err(|_| SemanticDecline::UnknownLayout)?;
                Ok(KeyLowering::StringCoerce(layout))
            }
        }
        None => Err(SemanticDecline::WrongArgumentType(1)),
    }
}

fn lookup_key_lowering(
    cx: &LowerCx<'_>,
    k_desc: &TypeDescriptor,
) -> Result<KeyLowering, SemanticDecline> {
    let arg_ty = cx
        .table
        .peeled(slot_type(cx.sig, SignatureSlot::Argument(1))?);
    let TypeKind::Reference {
        pointee,
        mutable: false,
    } = type_get(cx.table, arg_ty)?.kind
    else {
        return Err(SemanticDecline::WrongArgumentType(1));
    };
    let q_ty = cx.table.peeled(pointee);
    let q_desc = type_get(cx.table, q_ty)?;
    match classify_key(cx.table, k_desc) {
        Some(KeyKind::Primitive) => {
            if q_ty != k_desc.id {
                return Err(SemanticDecline::WrongArgumentType(1));
            }
            Ok(KeyLowering::SlotDeref)
        }
        Some(KeyKind::StrRef) => {
            if q_desc.kind == TypeKind::Str {
                Ok(KeyLowering::Direct)
            } else if matches!(
                q_desc.kind,
                TypeKind::Reference { pointee: p, mutable: false }
                    if cx.table.get(cx.table.peeled(p)).is_some_and(|d| d.kind == TypeKind::Str)
            ) {
                Ok(KeyLowering::SlotDeref)
            } else {
                Err(SemanticDecline::WrongArgumentType(1))
            }
        }
        Some(KeyKind::OwnedString) => {
            if q_desc.kind == TypeKind::Str {
                Ok(KeyLowering::Direct)
            } else if q_ty == k_desc.id {
                if cx.native_string_enabled {
                    Ok(KeyLowering::NativeString)
                } else {
                    let layout = StringLayout::of(cx.table, cx.interns, k_desc.id)
                        .map_err(|_| SemanticDecline::UnknownLayout)?;
                    Ok(KeyLowering::StringCoerce(layout))
                }
            } else {
                Err(SemanticDecline::WrongArgumentType(1))
            }
        }
        None => Err(SemanticDecline::WrongArgumentType(1)),
    }
}

fn emit_key_expr(lowering: &KeyLowering, helpers: &mut BTreeSet<&'static str>) -> Expr {
    let arg = jsast::id("$native1");
    match lowering {
        KeyLowering::Direct => arg,
        KeyLowering::SlotDeref => {
            jsast::index(jsast::member(arg.clone(), "buf"), jsast::member(arg, "off"))
        }
        KeyLowering::NativeString => jsast::member(arg, "s"),
        KeyLowering::StringCoerce(layout) => {
            helpers.insert("bytes_str");
            let ptr_expr = layout.member(arg.clone(), &layout.ptr);
            let len_expr = layout.member(arg.clone(), &layout.len);
            let baseline = jsast::rt_call(
                "bytes_str",
                vec![
                    jsast::member(ptr_expr.clone(), "buf"),
                    jsast::member(ptr_expr, "off"),
                    len_expr,
                ],
            );
            let builder_or_base = jsast::cond(
                jsast::binary(
                    BinOp::StrictNe,
                    jsast::member(arg.clone(), "text"),
                    jsast::undefined(),
                ),
                jsast::member(arg.clone(), "text"),
                baseline,
            );
            jsast::cond(
                jsast::binary(
                    BinOp::StrictEq,
                    jsast::unary(UnOp::TypeOf, arg.clone()),
                    jsast::string("string"),
                ),
                arg,
                builder_or_base,
            )
        }
    }
}

enum ExpectedOptionPayload {
    Value(TypeId),
    Ref { pointee: TypeId, mutable: bool },
}

fn option_layout(
    cx: &LowerCx<'_>,
    expected: ExpectedOptionPayload,
) -> Result<(u32, u32, String), SemanticDecline> {
    let result = type_get(cx.table, cx.table.peeled(cx.sig.result))?;
    if !matches!(
        result.js.enum_repr,
        Some(
            EnumRepr::Tag { fieldless: false }
                | EnumRepr::IntTag { fieldless: false }
                | EnumRepr::NullNiche { .. }
                | EnumRepr::SentinelNiche { .. }
        )
    ) {
        return Err(SemanticDecline::WrongReturnType);
    }
    let TypeKind::Adt {
        form: NominalForm::Enum,
        variants,
        ..
    } = &result.kind
    else {
        return Err(SemanticDecline::WrongReturnType);
    };
    if variants.len() != 2 {
        return Err(SemanticDecline::WrongReturnType);
    }
    let mut none_idx = None;
    let mut some_info = None;
    for (index, variant) in variants.iter().enumerate() {
        match variant.fields.as_slice() {
            [] if none_idx.is_none() => none_idx = Some(index as u32),
            [field] if some_info.is_none() => {
                let field_ty = cx.table.peeled(field.ty);
                let matches = match expected {
                    ExpectedOptionPayload::Value(v_ty) => field_ty == v_ty,
                    ExpectedOptionPayload::Ref { pointee, mutable } => type_get(cx.table, field_ty)
                        .is_ok_and(|d| {
                            matches!(
                                d.kind,
                                TypeKind::Reference {
                                    pointee: p,
                                    mutable: m,
                                } if cx.table.peeled(p) == pointee && m == mutable
                            )
                        }),
                };
                if !matches {
                    return Err(SemanticDecline::WrongReturnType);
                }
                let FieldKey::Name(key_id) = field.key else {
                    return Err(SemanticDecline::WrongReturnType);
                };
                let Some(field_name) = cx.interns.str(key_id) else {
                    return Err(SemanticDecline::WrongReturnType);
                };
                some_info = Some((index as u32, field_name.to_owned()));
            }
            _ => return Err(SemanticDecline::WrongReturnType),
        }
    }
    let (Some(none_idx), Some((some_idx, some_field))) = (none_idx, some_info) else {
        return Err(SemanticDecline::WrongReturnType);
    };
    Ok((none_idx, some_idx, some_field))
}

fn none_expr(cx: &LowerCx<'_>, none_idx: u32) -> Result<Expr, SemanticDecline> {
    crate::lower::js::value::enum_value(cx.table, cx.interns, cx.sig.result, none_idx, vec![])
        .ok_or(SemanticDecline::WrongReturnType)
}

fn some_expr(
    cx: &LowerCx<'_>,
    some_idx: u32,
    some_field: String,
    payload: Expr,
) -> Result<Expr, SemanticDecline> {
    crate::lower::js::value::enum_value(
        cx.table,
        cx.interns,
        cx.sig.result,
        some_idx,
        vec![(some_field, payload)],
    )
    .ok_or(SemanticDecline::WrongReturnType)
}

// -- Legality checks -------------------------------------------------------------------------

pub(super) fn legality_new(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    map_kv(cx)?;
    Ok(())
}

pub(super) fn legality_len(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    map_kv(cx)?;
    let ret = type_get(cx.table, cx.sig.result)?;
    let bits = unsigned_bits(ret)?;
    if bits == 0 || bits > 32 {
        return Err(SemanticDecline::UnsupportedPointerWidth(bits));
    }
    Ok(())
}

pub(super) fn legality_is_empty(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    map_kv(cx)?;
    if type_get(cx.table, cx.sig.result)?.kind != TypeKind::Bool {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

pub(super) fn legality_unit(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    map_kv(cx)?;
    super::vec::legality_unit_return(cx)
}

pub(super) fn legality_contains_key(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let (k_desc, _) = map_kv(cx)?;
    lookup_key_lowering(cx, k_desc)?;
    if type_get(cx.table, cx.sig.result)?.kind != TypeKind::Bool {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

pub(super) fn legality_get(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let (k_desc, v_desc) = map_kv(cx)?;
    lookup_key_lowering(cx, k_desc)?;
    option_layout(
        cx,
        ExpectedOptionPayload::Ref {
            pointee: v_desc.id,
            mutable: false,
        },
    )?;
    Ok(())
}

pub(super) fn legality_get_mut(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let (k_desc, v_desc) = map_kv(cx)?;
    lookup_key_lowering(cx, k_desc)?;
    option_layout(
        cx,
        ExpectedOptionPayload::Ref {
            pointee: v_desc.id,
            mutable: true,
        },
    )?;
    Ok(())
}

pub(super) fn legality_insert(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let (k_desc, v_desc) = map_kv(cx)?;
    owned_key_lowering(cx, k_desc)?;
    let val_ty = cx
        .table
        .peeled(slot_type(cx.sig, SignatureSlot::Argument(2))?);
    if val_ty != v_desc.id {
        return Err(SemanticDecline::WrongArgumentType(2));
    }
    option_layout(cx, ExpectedOptionPayload::Value(v_desc.id))?;
    Ok(())
}

pub(super) fn legality_remove(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let (k_desc, v_desc) = map_kv(cx)?;
    lookup_key_lowering(cx, k_desc)?;
    option_layout(cx, ExpectedOptionPayload::Value(v_desc.id))?;
    Ok(())
}

// -- Emitters --------------------------------------------------------------------------------

pub(super) fn btree_map_new(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    map_kv(cx)?;
    Ok(vec![jsast::ret(jsast::new(jsast::id("Map"), vec![]))])
}

pub(super) fn btree_map_len(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    map_kv(cx)?;
    Ok(vec![jsast::ret(jsast::member(receiver(), "size"))])
}

pub(super) fn btree_map_is_empty(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    map_kv(cx)?;
    Ok(vec![jsast::ret(jsast::binary(
        BinOp::StrictEq,
        jsast::member(receiver(), "size"),
        jsast::num(0),
    ))])
}

pub(super) fn btree_map_clear(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    map_kv(cx)?;
    Ok(vec![
        jsast::expr_stmt(jsast::method_call(receiver(), "clear", vec![])),
        jsast::ret_void(),
    ])
}

pub(super) fn btree_map_contains_key(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let (k_desc, _) = map_kv(cx)?;
    let key = emit_key_expr(&lookup_key_lowering(cx, k_desc)?, helpers);
    Ok(vec![jsast::ret(jsast::method_call(
        receiver(),
        "has",
        vec![key],
    ))])
}

pub(super) fn btree_map_get(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let (k_desc, v_desc) = map_kv(cx)?;
    let mutable = matches!(
        cx.semantic,
        BodySemantic::Standard(StandardOperation::BTreeMapGetMut)
    );
    let (none_idx, some_idx, some_field) = option_layout(
        cx,
        ExpectedOptionPayload::Ref {
            pointee: v_desc.id,
            mutable,
        },
    )?;
    let key = emit_key_expr(&lookup_key_lowering(cx, k_desc)?, helpers);
    let payload = if v_desc.js.is_indirect {
        jsast::id("$nmv")
    } else {
        ptr::slot(jsast::id("$nmv"), jsast::num(0))
    };
    Ok(vec![
        jsast::const_("$nmv", jsast::method_call(receiver(), "get", vec![key])),
        jsast::ret(jsast::cond(
            jsast::binary(BinOp::StrictEq, jsast::id("$nmv"), jsast::undefined()),
            none_expr(cx, none_idx)?,
            some_expr(cx, some_idx, some_field, payload)?,
        )),
    ])
}

pub(super) fn btree_map_insert(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let (k_desc, v_desc) = map_kv(cx)?;
    let (none_idx, some_idx, some_field) =
        option_layout(cx, ExpectedOptionPayload::Value(v_desc.id))?;
    let key = emit_key_expr(&owned_key_lowering(cx, k_desc)?, helpers);
    if v_desc.js.is_indirect {
        Ok(vec![
            jsast::const_("$nmk", key),
            jsast::const_(
                "$nmp",
                jsast::method_call(receiver(), "get", vec![jsast::id("$nmk")]),
            ),
            jsast::expr_stmt(jsast::method_call(
                receiver(),
                "set",
                vec![jsast::id("$nmk"), jsast::id("$native2")],
            )),
            jsast::ret(jsast::cond(
                jsast::binary(BinOp::StrictEq, jsast::id("$nmp"), jsast::undefined()),
                none_expr(cx, none_idx)?,
                some_expr(cx, some_idx, some_field, jsast::id("$nmp"))?,
            )),
        ])
    } else {
        Ok(vec![
            jsast::const_("$nmk", key),
            jsast::const_(
                "$nmp",
                jsast::method_call(receiver(), "get", vec![jsast::id("$nmk")]),
            ),
            jsast::if_(
                jsast::binary(BinOp::StrictEq, jsast::id("$nmp"), jsast::undefined()),
                vec![
                    jsast::expr_stmt(jsast::method_call(
                        receiver(),
                        "set",
                        vec![jsast::id("$nmk"), jsast::array(vec![jsast::id("$native2")])],
                    )),
                    jsast::ret(none_expr(cx, none_idx)?),
                ],
            ),
            jsast::const_("$nmo", jsast::index(jsast::id("$nmp"), jsast::num(0))),
            jsast::assign_stmt(
                jsast::index(jsast::id("$nmp"), jsast::num(0)),
                jsast::id("$native2"),
            ),
            jsast::ret(some_expr(cx, some_idx, some_field, jsast::id("$nmo"))?),
        ])
    }
}

pub(super) fn btree_map_remove(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let (k_desc, v_desc) = map_kv(cx)?;
    let (none_idx, some_idx, some_field) =
        option_layout(cx, ExpectedOptionPayload::Value(v_desc.id))?;
    let key = emit_key_expr(&lookup_key_lowering(cx, k_desc)?, helpers);
    let payload = if v_desc.js.is_indirect {
        jsast::id("$nmp")
    } else {
        jsast::index(jsast::id("$nmp"), jsast::num(0))
    };
    Ok(vec![
        jsast::const_("$nmk", key),
        jsast::const_(
            "$nmp",
            jsast::method_call(receiver(), "get", vec![jsast::id("$nmk")]),
        ),
        jsast::if_(
            jsast::binary(BinOp::StrictEq, jsast::id("$nmp"), jsast::undefined()),
            vec![jsast::ret(none_expr(cx, none_idx)?)],
        ),
        jsast::expr_stmt(jsast::method_call(
            receiver(),
            "delete",
            vec![jsast::id("$nmk")],
        )),
        jsast::ret(some_expr(cx, some_idx, some_field, payload)?),
    ])
}
