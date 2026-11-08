//! The `Vec` family emitters: construction, length, growth, indexing, removal, extend, drop.
//!
//! Ports of v1 `native_std_lower.rs`'s vec arms, spelling for spelling. The carrier is D5's
//! [`Carrier::NativeVec`]: `{ buf, len, cap }` with `buf` a plain JavaScript array of
//! **capacity** length, elements initialized in `[0, len)` and released slots cleared. Capacity
//! grows only as much as an operation requires — Rust does not promise an allocation growth
//! policy — and every capacity or bounds throw precedes every mutation. Operations that destroy
//! elements go through `drop.rs`'s element-glue discipline.

use std::collections::BTreeSet;

use crate::item::JsName;
use crate::jir::repr::Carrier;
use crate::jir::semantic::{BodySemantic, StandardOperation};
use crate::jir::types::{TypeDescriptor, TypeId, TypeKind};
use crate::jsast::{self, BinOp, Stmt};
use crate::ptr;

use super::drop::require_element_glue;
use super::string::{member, receiver};
use super::{
    capacity_guard, pointer_limit, slot_type, type_get, unsigned_bits, LowerCx, SemanticDecline,
    SignatureSlot,
};

pub(super) fn vec_new(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    helpers.insert("native_vec_new");
    Ok(vec![jsast::ret(jsast::rt_call(
        "native_vec_new",
        vec![jsast::num(0)],
    ))])
}

pub(super) fn vec_with_capacity(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let maximum = vec_capacity_limit(cx)?;
    helpers.insert("native_vec_new");
    Ok(vec![
        capacity_guard(receiver(), maximum, "Vec"),
        jsast::ret(jsast::rt_call("native_vec_new", vec![receiver()])),
    ])
}

pub(super) fn vec_len(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![jsast::ret(member("len"))])
}

pub(super) fn vec_capacity(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![jsast::ret(member("cap"))])
}

pub(super) fn vec_is_empty(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![jsast::ret(jsast::binary(
        BinOp::StrictEq,
        member("len"),
        jsast::num(0),
    ))])
}

/// `as_slice`/`as_mut_slice`: a fat slice sharing the native buffer — exactly the
/// [`Carrier::SliceView`] shape.
pub(super) fn vec_as_slice(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![jsast::ret(ptr::fat_slice(
        member("buf"),
        jsast::num(0),
        member("len"),
    ))])
}

/// `Index`/`IndexMut`: bounds throw before anything else, then the element's reference form.
/// Primitive elements use a `{ buf, off }` slot; an indirect aggregate is its own reference.
pub(super) fn vec_index(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let index = jsast::id("$native1");
    let element = if vec_element(cx)?.js.is_indirect {
        jsast::index(member("buf"), index.clone())
    } else {
        ptr::slot(member("buf"), index.clone())
    };
    Ok(vec![
        jsast::if_(
            jsast::binary(BinOp::Ge, index, member("len")),
            vec![jsast::throw(jsast::new(
                jsast::id("RangeError"),
                vec![jsast::string(
                    "rustc_codegen_js: native Vec index out of bounds",
                )],
            ))],
        ),
        jsast::ret(element),
    ])
}

/// `Vec::remove`: bounds throw, one-element `splice`, the backing array restored to capacity
/// length (the removed slot's release IS the splice).
pub(super) fn vec_remove(
    _cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    Ok(vec![
        jsast::if_(
            jsast::binary(BinOp::Ge, jsast::id("$native1"), member("len")),
            vec![jsast::throw(jsast::new(
                jsast::id("RangeError"),
                vec![jsast::string(
                    "rustc_codegen_js: native Vec remove index out of bounds",
                )],
            ))],
        ),
        jsast::const_(
            "$nativeRemoved",
            jsast::index(
                jsast::method_call(
                    member("buf"),
                    "splice",
                    vec![jsast::id("$native1"), jsast::num(1)],
                ),
                jsast::num(0),
            ),
        ),
        jsast::assign_stmt(jsast::member(member("buf"), "length"), member("cap")),
        jsast::expr_stmt(jsast::compound_assign(
            BinOp::Sub,
            member("len"),
            jsast::num(1),
        )),
        jsast::ret(jsast::id("$nativeRemoved")),
    ])
}

/// `Vec::push`: minimal growth, capacity throw before any mutation. The element-glue proof is
/// required even though push destroys nothing — the pushed element's eventual drop must be
/// executable for the vector to be native at all (v1 discipline).
pub(super) fn vec_push(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    require_element_glue(cx)?;
    let maximum = vec_capacity_limit(cx)?;
    helpers.insert("native_vec_set_capacity");
    Ok(vec![
        jsast::const_(
            "$nativeRequired",
            jsast::binary(BinOp::Add, member("len"), jsast::num(1)),
        ),
        capacity_guard(jsast::id("$nativeRequired"), maximum, "Vec"),
        jsast::if_(
            jsast::binary(BinOp::StrictEq, member("len"), member("cap")),
            vec![jsast::expr_stmt(jsast::rt_call(
                "native_vec_set_capacity",
                vec![receiver(), jsast::id("$nativeRequired")],
            ))],
        ),
        jsast::assign_stmt(
            jsast::index(member("buf"), member("len")),
            jsast::id("$native1"),
        ),
        jsast::expr_stmt(jsast::compound_assign(
            BinOp::Add,
            member("len"),
            jsast::num(1),
        )),
        jsast::ret_void(),
    ])
}

/// `Extend<Vec<T>>` of an owned source vector: all throwing work first, then the source is
/// emptied *before* its elements move, so its eventual drop cannot destroy values whose
/// ownership transferred; the moved-out slots are cleared and the source's capacity released.
pub(super) fn vec_extend(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    _refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    let maximum = vec_capacity_limit(cx)?;
    helpers.insert("native_vec_set_capacity");
    Ok(vec![
        jsast::const_("$nativeTargetLen", member("len")),
        jsast::const_(
            "$nativeSourceLen",
            jsast::member(jsast::id("$native1"), "len"),
        ),
        jsast::const_(
            "$nativeRequired",
            jsast::binary(
                BinOp::Add,
                jsast::id("$nativeTargetLen"),
                jsast::id("$nativeSourceLen"),
            ),
        ),
        capacity_guard(jsast::id("$nativeRequired"), maximum, "Vec"),
        jsast::if_(
            jsast::binary(BinOp::Lt, member("cap"), jsast::id("$nativeRequired")),
            vec![jsast::expr_stmt(jsast::rt_call(
                "native_vec_set_capacity",
                vec![receiver(), jsast::id("$nativeRequired")],
            ))],
        ),
        // All potentially throwing work is complete. Emptying the source first makes its
        // eventual drop unable to destroy elements whose ownership moves to the target.
        jsast::assign_stmt(jsast::member(jsast::id("$native1"), "len"), jsast::num(0)),
        jsast::let_("$nativeIndex", jsast::num(0)),
        jsast::while_(
            jsast::binary(
                BinOp::Lt,
                jsast::id("$nativeIndex"),
                jsast::id("$nativeSourceLen"),
            ),
            vec![
                jsast::assign_stmt(
                    jsast::index(
                        member("buf"),
                        jsast::binary(
                            BinOp::Add,
                            jsast::id("$nativeTargetLen"),
                            jsast::id("$nativeIndex"),
                        ),
                    ),
                    jsast::index(
                        jsast::member(jsast::id("$native1"), "buf"),
                        jsast::id("$nativeIndex"),
                    ),
                ),
                jsast::assign_stmt(
                    jsast::index(
                        jsast::member(jsast::id("$native1"), "buf"),
                        jsast::id("$nativeIndex"),
                    ),
                    jsast::undefined(),
                ),
                jsast::expr_stmt(jsast::compound_assign(
                    BinOp::Add,
                    jsast::id("$nativeIndex"),
                    jsast::num(1),
                )),
            ],
        ),
        jsast::assign_stmt(member("len"), jsast::id("$nativeRequired")),
        jsast::expr_stmt(jsast::rt_call(
            "native_vec_set_capacity",
            vec![jsast::id("$native1"), jsast::num(0)],
        )),
        jsast::ret_void(),
    ])
}

/// `Vec::clear`: the ordered element-drop body when the element needs drop, else the length
/// release helper.
pub(super) fn vec_clear(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    if let Some(glue) = require_element_glue(cx)? {
        refs.insert(glue.clone());
        Ok(super::drop::dropful_vec_clear(glue))
    } else {
        helpers.insert("native_vec_set_len");
        Ok(vec![
            jsast::expr_stmt(jsast::rt_call(
                "native_vec_set_len",
                vec![receiver(), jsast::num(0)],
            )),
            jsast::ret_void(),
        ])
    }
}

/// `Vec`'s own drop: ordered element drops when required, then everything released.
pub(super) fn vec_drop(
    cx: &LowerCx<'_>,
    _carrier: Option<Carrier>,
    helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    if let Some(glue) = require_element_glue(cx)? {
        refs.insert(glue.clone());
        helpers.insert("native_vec_set_capacity");
        Ok(super::drop::dropful_vec_drop(glue))
    } else {
        helpers.insert("native_vec_set_capacity");
        helpers.insert("native_vec_set_len");
        Ok(vec![
            jsast::expr_stmt(jsast::rt_call(
                "native_vec_set_len",
                vec![receiver(), jsast::num(0)],
            )),
            jsast::expr_stmt(jsast::rt_call(
                "native_vec_set_capacity",
                vec![receiver(), jsast::num(0)],
            )),
            jsast::ret_void(),
        ])
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
    vec_capacity_limit(cx)?;
    Ok(())
}

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

pub(super) fn legality_bool_return(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    if !matches!(type_get(cx.table, cx.sig.result)?.kind, TypeKind::Bool) {
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

/// `push`'s value argument is exactly the vector's element type.
pub(super) fn legality_push(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let receiver = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    let TypeKind::Reference { pointee, .. } = type_get(cx.table, receiver)?.kind else {
        return Err(SemanticDecline::WrongArgumentType(0));
    };
    let vector = type_get(cx.table, pointee)?;
    let TypeKind::Adt { args, .. } = &vector.kind else {
        return Err(SemanticDecline::WrongArgumentType(0));
    };
    if args.first().copied() != Some(slot_type(cx.sig, SignatureSlot::Argument(1))?) {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    legality_unit_return(cx)
}

/// `extend`'s owned source is exactly the receiver's own vector type.
pub(super) fn legality_extend(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let receiver = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    let TypeKind::Reference { pointee, .. } = type_get(cx.table, receiver)?.kind else {
        return Err(SemanticDecline::WrongArgumentType(0));
    };
    if pointee != slot_type(cx.sig, SignatureSlot::Argument(1))? {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    vec_capacity_limit(cx)?;
    legality_unit_return(cx)
}

/// `as_slice`/`as_mut_slice` return a slice reference over the exact element, at the operation's
/// mutability.
pub(super) fn legality_as_slice(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let mutable = matches!(
        cx.semantic,
        BodySemantic::Standard(StandardOperation::VecAsMutSlice)
    );
    let result = type_get(cx.table, cx.sig.result)?;
    let correct = matches!(
        result.kind,
        TypeKind::Reference {
            pointee,
            mutable: result_mutable,
        } if result_mutable == mutable
            && type_get(cx.table, pointee).is_ok_and(|pointee| {
                matches!(
                    pointee.kind,
                    TypeKind::Slice(element)
                        if vec_element(cx).is_ok_and(|expected| expected.id == element)
                )
            })
    );
    if !correct {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

/// `Index`/`IndexMut` take a `usize` and return an element reference at the operation's
/// mutability.
pub(super) fn legality_index(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    legality_usize_argument_1(cx)?;
    let mutable = matches!(
        cx.semantic,
        BodySemantic::Standard(StandardOperation::VecIndexMut)
    );
    let result = type_get(cx.table, cx.sig.result)?;
    let correct = matches!(
        result.kind,
        TypeKind::Reference {
            pointee,
            mutable: result_mutable,
        } if result_mutable == mutable
            && vec_element(cx).is_ok_and(|element| element.id == pointee)
    );
    if !correct {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

/// `remove` takes a `usize` and returns the element by value.
pub(super) fn legality_remove(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    legality_usize_argument_1(cx)?;
    let correct = vec_element(cx).is_ok_and(|element| {
        element.id == cx.sig.result && element.layout.sized && !element.layout.zst
    });
    if !correct {
        return Err(SemanticDecline::WrongReturnType);
    }
    Ok(())
}

fn legality_usize_argument_1(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    let argument = slot_type(cx.sig, SignatureSlot::Argument(1))?;
    if !matches!(
        type_get(cx.table, argument)?.kind,
        TypeKind::Integer {
            signed: false,
            bits: 32,
        }
    ) {
        return Err(SemanticDecline::WrongArgumentType(1));
    }
    Ok(())
}

/// The element type of the operation's canonical `Vec<T>` — the monomorphic type argument, never
/// a name inference (v1 `vec_element`).
pub(super) fn vec_element<'a>(cx: &LowerCx<'a>) -> Result<&'a TypeDescriptor, SemanticDecline> {
    let vector_ty = match cx.semantic {
        BodySemantic::DropGlue(dropped) => dropped,
        BodySemantic::Standard(StandardOperation::VecNew | StandardOperation::VecWithCapacity) => {
            cx.sig.result
        }
        BodySemantic::Standard(
            StandardOperation::VecLen
            | StandardOperation::VecCapacity
            | StandardOperation::VecPush
            | StandardOperation::VecClear
            | StandardOperation::VecAsSlice
            | StandardOperation::VecAsMutSlice
            | StandardOperation::VecIsEmpty
            | StandardOperation::VecIndex
            | StandardOperation::VecIndexMut
            | StandardOperation::VecRemove
            | StandardOperation::VecExtend
            | StandardOperation::VecDrop,
        ) => cx
            .sig
            .arguments
            .first()
            .copied()
            .ok_or(SemanticDecline::WrongArgumentType(0))?,
        BodySemantic::Standard(operation) => {
            return Err(SemanticDecline::UnsupportedOperation(operation));
        }
        BodySemantic::Ordinary => return Err(SemanticDecline::NotAStandardOperation),
    };
    element_of_vec_type(cx, vector_ty)
}

fn element_of_vec_type<'a>(
    cx: &LowerCx<'a>,
    mut vector_ty: TypeId,
) -> Result<&'a TypeDescriptor, SemanticDecline> {
    if let TypeKind::Reference { pointee, .. } = type_get(cx.table, vector_ty)?.kind {
        vector_ty = pointee;
    }
    let vector = type_get(cx.table, vector_ty)?;
    if vector.meta.candidate != crate::jir::types::RepresentationCandidate::Vec {
        return Err(SemanticDecline::WrongNativeType(SignatureSlot::Result));
    }
    let TypeKind::Adt { args, .. } = &vector.kind else {
        return Err(SemanticDecline::WrongNativeType(SignatureSlot::Result));
    };
    let element = args
        .first()
        .copied()
        .ok_or(SemanticDecline::UnknownElementLayout)?;
    type_get(cx.table, element)
}

/// The capacity ceiling of a `Vec` operation: how many elements of this size and alignment fit
/// the producer's 32-bit address space (v1 `vec_capacity_limit`).
pub(super) fn vec_capacity_limit(cx: &LowerCx<'_>) -> Result<u64, SemanticDecline> {
    let element = vec_element(cx)?;
    if !element.layout.sized {
        return Err(SemanticDecline::UnknownElementLayout);
    }
    if element.layout.zst {
        return Err(SemanticDecline::ZeroSizedElement);
    }
    let (Some(size), Some(align)) = (element.layout.size, element.layout.align) else {
        return Err(SemanticDecline::UnknownElementLayout);
    };
    if size == 0 || align == 0 {
        return Err(SemanticDecline::UnknownElementLayout);
    }
    let argument = cx
        .sig
        .arguments
        .first()
        .copied()
        .map(|ty| type_get(cx.table, ty))
        .transpose()?
        .ok_or(SemanticDecline::WrongArgumentType(0))?;
    let bits = match argument.kind {
        TypeKind::Reference { .. } => argument
            .layout
            .size
            .and_then(|bytes| bytes.checked_mul(8))
            .and_then(|bits| u32::try_from(bits).ok())
            .ok_or(SemanticDecline::UnknownElementLayout)?,
        _ => unsigned_bits(argument).map_err(|_| SemanticDecline::WrongArgumentType(0))?,
    };
    let bytes = pointer_limit(bits)?
        .checked_add(1)
        .and_then(|limit| limit.checked_sub(align))
        .ok_or(SemanticDecline::UnknownElementLayout)?;
    Ok(bytes / size)
}
