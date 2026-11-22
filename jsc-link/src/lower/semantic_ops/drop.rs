//! Element and structural drop discipline for the native `Vec` carrier.
//!
//! Ports of v1 `native_std_lower.rs`'s element-glue rules and `native_structural_drop.rs`'s
//! order/can-throw discipline, re-keyed onto the type table. Two contracts:
//!
//! * **Element glue** ([`required_element_drop`], [`require_element_glue`]): an operation that
//!   destroys elements (`VecPush` proves, `VecClear`/`VecDrop` execute) needs the exact monomorphic
//!   drop-glue item of the vector's element. An element with its own destructor stays on baseline
//!   outright; one that merely needs drop requires the linker-resolved glue in
//!   [`LowerCx::element_glue`]. The dropful bodies call that glue element by element **in ascending
//!   index order**, clearing each slot after its destructor ran — v1's spellings.
//!
//! * **Structural chain** ([`structural_chain`], [`structural_drop_body`]): a compiler-generated
//!   drop shim over a plain one-variant struct or tuple drops its needy fields **in declaration
//!   order**. The chain here is resolved from the type table, and the caller (the representation
//!   planner) owes the can-throw proof v1's `prepared_nonthrowing` set carried: only glue that
//!   provably cannot throw may replace a body whose MIR cleanup edges are dropped.

use std::collections::BTreeSet;

use crate::item::JsName;
use crate::jir::interns::Interns;
use crate::jir::repr::Carrier;
use crate::jir::semantic::BodySemantic;
use crate::jir::types::{
    AllocatorIdentity, FieldKey, NominalAbi, NominalForm, PinnedBoundary, RepresentationCandidate,
    TypeDescriptor, TypeId, TypeKind, TypeTable,
};
use crate::jsast::{self, BinOp, Stmt};

use super::vec::vec_element;
use super::{slot_type, type_get, LowerCx, SemanticDecline, SignatureSlot};

/// The element type whose drop glue must be resolved for this vec operation, or `None` when the
/// element does not need drop (v1 `required_element_drop`, minus the operation filter — the
/// registry entries with [`super::Deps::ElementDropGlue`] are exactly the callers).
pub fn required_element_drop(cx: &LowerCx<'_>) -> Result<Option<TypeId>, SemanticDecline> {
    let element = vec_element(cx)?;
    if element.layout.has_own_drop {
        return Err(SemanticDecline::ElementHasOwnDrop);
    }
    Ok(element.layout.needs_drop.then_some(element.id))
}

/// The glue the dropful bodies call: `None` for a drop-free element, the resolved glue when the
/// element needs drop, and an exact decline when it is required but unresolved.
pub(super) fn require_element_glue<'a>(
    cx: &LowerCx<'a>,
) -> Result<Option<&'a JsName>, SemanticDecline> {
    match required_element_drop(cx)? {
        Some(_) => cx
            .element_glue
            .map(Some)
            .ok_or(SemanticDecline::ElementDropRequired),
        None => Ok(None),
    }
}

/// The pointee whose glue a native `Box` drop must call, or `None` when dropping the pointee is a
/// no-op. Custom pointee destructors stay baseline: this first Box carrier only admits structural
/// compiler glue, whose exact monomorphization the linker can resolve.
pub fn required_box_pointee_drop(cx: &LowerCx<'_>) -> Result<Option<TypeId>, SemanticDecline> {
    let pointee = box_pointee(cx)?;
    if pointee.layout.has_own_drop {
        return Err(SemanticDecline::ElementHasOwnDrop);
    }
    Ok(pointee.layout.needs_drop.then_some(pointee.id))
}

/// Proof gate for replacing compiler-generated `Box<T>` drop glue. Allocation release is
/// intentionally omitted: value-flow has already ruled out custom allocators, raw provenance,
/// address observation, unsized pointees, and zero-sized pointees, so JavaScript GC is the only
/// observable lifetime mechanism left.
pub(super) fn legality_box_drop(cx: &LowerCx<'_>) -> Result<(), SemanticDecline> {
    super::vec::legality_unit_return(cx)?;
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
            jsast::member(jsast::id("$native0"), "buf"),
            jsast::member(jsast::id("$native0"), "off"),
        ),
        _ => jsast::id("$native0"),
    };
    Ok(vec![
        jsast::const_("$nativeBox", boxed),
        jsast::expr_stmt(jsast::call(
            jsast::id(glue.as_str()),
            vec![jsast::index(
                jsast::member(jsast::id("$nativeBox"), "buf"),
                jsast::member(jsast::id("$nativeBox"), "off"),
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
        BodySemantic::Ordinary | BodySemantic::PanicImpl => {
            return Err(SemanticDecline::NotAStandardOperation)
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

/// `Vec::clear` over a needy element: the vector is emptied *before* the destructors run
/// (`Vec::clear`'s order), then each element is dropped and its slot cleared, ascending. Under
/// the target's panic=abort policy a throwing destructor does not unwind into Rust code.
pub(super) fn dropful_vec_clear(element_glue: &JsName) -> Vec<Stmt> {
    let receiver = || jsast::id("$native0");
    let buffer = || jsast::member(receiver(), "buf");
    vec![
        jsast::const_("$nativeOldLen", jsast::member(receiver(), "len")),
        jsast::assign_stmt(jsast::member(receiver(), "len"), jsast::num(0)),
        jsast::let_("$nativeIndex", jsast::num(0)),
        jsast::while_(
            jsast::binary(
                BinOp::Lt,
                jsast::id("$nativeIndex"),
                jsast::id("$nativeOldLen"),
            ),
            vec![
                jsast::const_(
                    "$nativeElement",
                    jsast::index(buffer(), jsast::id("$nativeIndex")),
                ),
                jsast::expr_stmt(jsast::call(
                    jsast::id(element_glue.as_str()),
                    vec![jsast::id("$nativeElement")],
                )),
                jsast::assign_stmt(
                    jsast::index(buffer(), jsast::id("$nativeIndex")),
                    jsast::undefined(),
                ),
                jsast::expr_stmt(jsast::compound_assign(
                    BinOp::Add,
                    jsast::id("$nativeIndex"),
                    jsast::num(1),
                )),
            ],
        ),
        jsast::ret_void(),
    ]
}

/// `Vec`'s drop over a needy element: ascending element drops with slot clearing, then the
/// length and capacity released.
pub(super) fn dropful_vec_drop(element_glue: &JsName) -> Vec<Stmt> {
    let receiver = || jsast::id("$native0");
    let buffer = || jsast::member(receiver(), "buf");
    vec![
        jsast::const_("$nativeOldLen", jsast::member(receiver(), "len")),
        jsast::let_("$nativeIndex", jsast::num(0)),
        jsast::while_(
            jsast::binary(
                BinOp::Lt,
                jsast::id("$nativeIndex"),
                jsast::id("$nativeOldLen"),
            ),
            vec![
                jsast::const_(
                    "$nativeElement",
                    jsast::index(buffer(), jsast::id("$nativeIndex")),
                ),
                jsast::expr_stmt(jsast::call(
                    jsast::id(element_glue.as_str()),
                    vec![jsast::id("$nativeElement")],
                )),
                jsast::assign_stmt(
                    jsast::index(buffer(), jsast::id("$nativeIndex")),
                    jsast::undefined(),
                ),
                jsast::expr_stmt(jsast::compound_assign(
                    BinOp::Add,
                    jsast::id("$nativeIndex"),
                    jsast::num(1),
                )),
            ],
        ),
        jsast::assign_stmt(jsast::member(receiver(), "len"), jsast::num(0)),
        jsast::expr_stmt(jsast::rt_call(
            "native_vec_set_capacity",
            vec![receiver(), jsast::num(0)],
        )),
        jsast::ret_void(),
    ]
}

/// One entry of a structural drop chain: the member spelling and type of a field whose drop glue
/// must run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChainField {
    pub member: String,
    pub ty: TypeId,
}

/// The needy fields of a compiler-generated structural drop shim's type, **in declaration
/// order** — the order Rust drops struct fields and the order the replacement body must call
/// their glue (v1 `native_structural_drop.rs`'s validated chain, resolved from the type table).
///
/// Declines exactly when the type is not the plain shape v1 accepted: a one-variant struct or
/// tuple on the Rust ABI, no pinned boundary, sized and not zero-sized, not itself a native
/// standard candidate, and without its own destructor.
pub fn structural_chain(
    table: &TypeTable,
    interns: &Interns,
    dropped: TypeId,
) -> Result<Vec<ChainField>, SemanticDecline> {
    let descriptor = type_get(table, dropped)?;
    if !descriptor.layout.needs_drop || descriptor.layout.has_own_drop {
        return Err(SemanticDecline::NotAStandardOperation);
    }
    if descriptor.meta.candidate != RepresentationCandidate::None
        || descriptor.meta.abi != NominalAbi::Rust
        || descriptor.meta.boundary != PinnedBoundary::None
        || !descriptor.layout.sized
        || descriptor.layout.zst
    {
        return Err(SemanticDecline::NotAStandardOperation);
    }
    let fields: Vec<(FieldKey, TypeId)> = match &descriptor.kind {
        TypeKind::Tuple(fields) => fields
            .iter()
            .enumerate()
            .map(|(index, ty)| (FieldKey::Index(index as u32), *ty))
            .collect(),
        TypeKind::Adt {
            form: NominalForm::Struct,
            variants,
            ..
        } if variants.len() == 1 => variants[0]
            .fields
            .iter()
            .map(|field| (field.key, field.ty))
            .collect(),
        _ => return Err(SemanticDecline::NotAStandardOperation),
    };
    let mut chain = Vec::new();
    for (key, ty) in fields {
        if !type_get(table, ty)?.layout.needs_drop {
            continue;
        }
        let member = match key {
            FieldKey::Name(id) => interns
                .str(id)
                .map(str::to_owned)
                .ok_or(SemanticDecline::NotAStandardOperation)?,
            FieldKey::Index(index) => format!("_{index}"),
            FieldKey::Transparent => return Err(SemanticDecline::NotAStandardOperation),
        };
        chain.push(ChainField { member, ty });
    }
    Ok(chain)
}

/// The replacement body of a structural drop shim: each field's glue called in chain order over
/// `$native0.<member>`, then `return`. The caller owes the nonthrowing proof for every glue —
/// the body has no cleanup edges to unwind into.
pub fn structural_drop_body(chain: &[(String, JsName)]) -> Vec<Stmt> {
    let mut body: Vec<Stmt> = chain
        .iter()
        .map(|(member, glue)| {
            jsast::expr_stmt(jsast::call(
                jsast::id(glue.as_str()),
                vec![jsast::member(jsast::id("$native0"), member.clone())],
            ))
        })
        .collect();
    body.push(jsast::ret_void());
    body
}
