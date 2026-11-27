//! Drop legality and native `Box<T, Global>` drop-glue emitter.

use std::collections::BTreeSet;

use crate::item::JsName;
use crate::jir::repr::Carrier;
use crate::jir::semantic::BodySemantic;
use crate::jir::types::{
    AllocatorIdentity, NominalAbi, NominalForm, PinnedBoundary, RepresentationCandidate,
    TypeDescriptor, TypeId, TypeKind, TypeTable,
};
use crate::jsast::{self, Stmt};

use super::{slot_type, type_get, LowerCx, SemanticDecline, SignatureSlot};

/// True when `ty` transitively contains no user-defined `Drop` implementation under the global
/// allocator: only drop-free scalars/aggregates and canonical global `Vec`/`String`/`Box`/`BTreeMap`
/// containers over recursively trivially-nonthrowing elements.
pub fn element_drop_is_trivially_nonthrowing(table: &TypeTable, ty: TypeId) -> bool {
    let mut visited = BTreeSet::new();
    element_drop_is_trivially_nonthrowing_inner(table, ty, &mut visited)
}

fn element_drop_is_trivially_nonthrowing_inner(
    table: &TypeTable,
    ty: TypeId,
    visited: &mut BTreeSet<TypeId>,
) -> bool {
    let ty = table.peeled(ty);
    if !visited.insert(ty) {
        return true;
    }
    let Some(desc) = table.get(ty) else {
        return false;
    };
    if !desc.layout.needs_drop {
        return true;
    }
    if desc.layout.has_own_drop {
        let std_container = matches!(
            desc.meta.candidate,
            RepresentationCandidate::Vec
                | RepresentationCandidate::String
                | RepresentationCandidate::Box
                | RepresentationCandidate::BTreeMap
        ) && desc.meta.allocator == AllocatorIdentity::Global;
        if !std_container {
            return false;
        }
        return match &desc.kind {
            TypeKind::Adt { args, .. } => match desc.meta.candidate {
                RepresentationCandidate::String => true,
                RepresentationCandidate::Vec | RepresentationCandidate::Box => {
                    args.first().is_some_and(|inner| {
                        element_drop_is_trivially_nonthrowing_inner(table, *inner, visited)
                    })
                }
                RepresentationCandidate::BTreeMap => {
                    args.first().is_some_and(|k| {
                        element_drop_is_trivially_nonthrowing_inner(table, *k, visited)
                    }) && args.get(1).is_some_and(|v| {
                        element_drop_is_trivially_nonthrowing_inner(table, *v, visited)
                    })
                }
                _ => false,
            },
            _ => false,
        };
    }
    match &desc.kind {
        TypeKind::Tuple(fields) => fields
            .iter()
            .all(|field| element_drop_is_trivially_nonthrowing_inner(table, *field, visited)),
        TypeKind::Array { element, .. } | TypeKind::Slice(element) => {
            element_drop_is_trivially_nonthrowing_inner(table, *element, visited)
        }
        TypeKind::Adt {
            form: NominalForm::Struct | NominalForm::Enum,
            variants,
            ..
        } => variants.iter().all(|variant| {
            variant
                .fields
                .iter()
                .all(|field| element_drop_is_trivially_nonthrowing_inner(table, field.ty, visited))
        }),
        _ => false,
    }
}

/// The pointee whose glue a native `Box` drop must call, or `None` when dropping the pointee is a
/// no-op. Custom pointee destructors stay baseline: this Box lowering only admits structural
/// compiler glue, whose exact monomorphization the linker can resolve.
pub fn required_box_pointee_drop(cx: &LowerCx<'_>) -> Result<Option<TypeId>, SemanticDecline> {
    let pointee = box_pointee(cx)?;
    if pointee.layout.has_own_drop {
        return Err(SemanticDecline::ElementHasOwnDrop);
    }
    Ok(pointee.layout.needs_drop.then_some(pointee.id))
}

/// Proof gate for replacing compiler-generated `Box<T>` drop glue.
pub(super) fn legality_box_drop(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    super::legality_unit_return(cx)?;
    required_box_pointee_drop(cx).map(|_| ())
}

/// Compiler-generated `Box<T>` drop glue under [`Carrier::NativeBox`]. The carrier retains the
/// baseline pointer shape, calls exact structural pointee glue when needed, and leaves the now
/// unreachable allocation to JavaScript GC.
pub(super) fn box_drop(
    cx: &LowerCx<'_>,
    carrier: Option<Carrier>,
    _helpers: &mut BTreeSet<&'static str>,
    refs: &mut BTreeSet<JsName>,
) -> Result<Vec<Stmt>, SemanticDecline> {
    if carrier != Some(Carrier::NativeBox) {
        return Err(SemanticDecline::WrongCarrier(SignatureSlot::Argument(0)));
    }
    let Some(_) = required_box_pointee_drop(cx)? else {
        return Ok(vec![jsast::ret_void()]);
    };
    let glue = cx
        .element_glue
        .ok_or(SemanticDecline::ElementDropRequired)?;
    refs.insert(glue.clone());

    let argument = slot_type(cx.sig, SignatureSlot::Argument(0))?;
    let boxed = match type_get(cx.table, argument)?.kind {
        TypeKind::Reference { .. } => jsast::index(
            jsast::member(jsast::id("$native0"), rcgjs_object::ptr::BUF),
            jsast::member(jsast::id("$native0"), rcgjs_object::ptr::OFF),
        ),
        _ => jsast::id("$native0"),
    };
    Ok(vec![
        jsast::const_("$nativeBox", boxed),
        jsast::expr_stmt(jsast::call(
            jsast::id(glue.as_str()),
            vec![jsast::index(
                jsast::member(jsast::id("$nativeBox"), rcgjs_object::ptr::BUF),
                jsast::member(jsast::id("$nativeBox"), rcgjs_object::ptr::OFF),
            )],
        )),
        jsast::ret_void(),
    ])
}

fn box_pointee<'a>(cx: &LowerCx<'a>) -> Result<&'a TypeDescriptor, SemanticDecline> {
    let dropped = match cx.semantic {
        BodySemantic::DropGlue(dropped) => dropped,
        BodySemantic::Standard(operation) => {
            return Err(SemanticDecline::UnsupportedOperation(operation));
        }
        BodySemantic::Ordinary
        | BodySemantic::PanicImpl
        | BodySemantic::JsExport
        | BodySemantic::JsExportOut => {
            return Err(SemanticDecline::NotAStandardOperation);
        }
    };
    let descriptor = type_get(cx.table, dropped)?;
    if descriptor.meta.candidate != RepresentationCandidate::Box
        || descriptor.meta.abi != NominalAbi::Rust
        || descriptor.meta.allocator != AllocatorIdentity::Global
        || descriptor.meta.boundary != PinnedBoundary::None
        || !descriptor.layout.sized
        || descriptor.layout.zst
    {
        return Err(SemanticDecline::WrongNativeType(SignatureSlot::Argument(0)));
    }
    let pointee = match &descriptor.kind {
        TypeKind::Adt { args, .. } => args.first().copied(),
        _ => None,
    }
    .ok_or(SemanticDecline::UnknownElementLayout)?;
    let pointee = type_get(cx.table, pointee)?;
    if !pointee.layout.sized
        || pointee.layout.zst
        || pointee.layout.size.is_none()
        || pointee.layout.align.is_none()
    {
        return Err(SemanticDecline::UnknownElementLayout);
    }
    Ok(pointee)
}
