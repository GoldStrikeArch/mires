//! Which types the escape analysis may scalarize, read off their [`TypeDescriptor`].
//!
//! One question, asked of a type and a variant: *if a value of this shape never escaped, what
//! would its fields be?* The answer is the field types in field order, which is everything
//! [`super::partial_escape`] needs — the JavaScript keys never come up, because a scalarized
//! aggregate is never printed.
//!
//! The filter is the plan's, tightened in one place. `needs_drop` is refused outright rather than
//! only an own `Drop` impl: a `Drop` op on a node escapes it anyway, and refusing here keeps drop
//! glue out of the pass.
//!
//! A field may itself be an aggregate — that is what `Option<(u32, u32)>` needs — so a node's
//! field may hold *another* node. The one thing that costs is [`Copy`]: copying a node whose field
//! holds a real object would alias that object rather than duplicate it, so
//! [`shape_survives_copy`] answers whether a node of this shape may be copied at all.
//!
//! [`Copy`]: crate::jir::repr::OpKind::Copy
//!
//! [`TypeDescriptor`]: crate::jir::types::TypeDescriptor

use crate::jir::semantic::TypeId;
use crate::jir::types::{EnumRepr, NominalAbi, NominalForm, PinnedBoundary, TypeKind, TypeTable};

/// The most fields a scalarized aggregate may have. Above it the per-field block parameters at a
/// merge cost more than the object they replace.
const MAX_FIELDS: usize = 12;

/// One variant of a multi-variant `EnumRepr::Tag { fieldless: false }` enum eligible for
/// scalarization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnumVariantShape {
    pub logical_discr: i128,
    pub fields: Vec<TypeId>,
}

/// Shape of a multi-variant `EnumRepr::Tag { fieldless: false }` enum eligible for scalarization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnumShape {
    pub variants: Vec<EnumVariantShape>,
}

/// The field types of `ty`, in field order, or `None` when a value of it may not be scalarized —
/// asked of a type alone, so a multi-variant enum has no answer.
///
/// This is the question a caller that has only a *type* may ask: the flat call ABI rebuilds a
/// parameter it took apart, and the fields of an enum depend on which variant the value it is
/// handed happens to be. `variant` is the variant such a caller means, which is always 0.
pub fn shape(table: &TypeTable, ty: TypeId, variant: u32) -> Option<Vec<TypeId>> {
    if let Some(TypeKind::Adt {
        form: NominalForm::Enum,
        variants,
        ..
    }) = table.get(ty).map(|d| &d.kind)
    {
        if variants.len() != 1 {
            return None;
        }
    }
    variant_shape(table, ty, variant)
}

/// The field types of `ty`'s `variant`, in field order, or `None` when a value of that shape may
/// not be scalarized.
pub fn variant_shape(table: &TypeTable, ty: TypeId, variant: u32) -> Option<Vec<TypeId>> {
    let descriptor = table.get(ty)?;
    let layout = descriptor.layout;
    if !layout.sized || layout.zst || layout.needs_drop {
        return None;
    }
    if descriptor.meta.abi == NominalAbi::C || descriptor.meta.boundary != PinnedBoundary::None {
        return None;
    }
    if descriptor.js.transparent.is_some() || descriptor.js.unsized_tail.is_some() {
        return None;
    }
    let own_fields = || -> Vec<TypeId> { descriptor.js.fields.iter().map(|f| f.ty).collect() };
    let fields: Vec<TypeId> = match &descriptor.kind {
        TypeKind::Tuple(_) => {
            if variant != 0 {
                return None;
            }
            own_fields()
        }
        TypeKind::Adt {
            form: NominalForm::Struct,
            ..
        } => {
            if variant != 0 {
                return None;
            }
            own_fields()
        }
        // The table lists one `Index(0)` entry for an array, so the fields are spelled here.
        TypeKind::Array { element, length } => {
            if variant != 0 || *length > MAX_FIELDS as u64 {
                return None;
            }
            vec![*element; *length as usize]
        }
        TypeKind::Closure { upvars, .. } => {
            if variant != 0 {
                return None;
            }
            upvars.clone()
        }
        TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            ..
        } => {
            if !matches!(
                descriptor.js.enum_repr,
                Some(EnumRepr::Tag { fieldless: false } | EnumRepr::IntTag { fieldless: false })
            ) || variants.iter().any(|v| v.fields.len() > MAX_FIELDS)
            {
                return None;
            }
            variants
                .get(variant as usize)?
                .fields
                .iter()
                .map(|f| f.ty)
                .collect()
        }
        _ => return None,
    };
    if fields.is_empty() || fields.len() > MAX_FIELDS {
        return None;
    }
    Some(fields)
}

/// The variant shapes of a multi-variant `EnumRepr::Tag { fieldless: false }` enum, when every
/// variant has at most [`MAX_FIELDS`] copy-safe fields and may be scalarized into `(tag, slots...)`.
pub fn enum_shape(table: &TypeTable, ty: TypeId) -> Option<EnumShape> {
    let descriptor = table.get(ty)?;
    let layout = descriptor.layout;
    if !layout.sized || layout.zst || layout.needs_drop {
        return None;
    }
    if descriptor.meta.abi == NominalAbi::C || descriptor.meta.boundary != PinnedBoundary::None {
        return None;
    }
    if descriptor.js.transparent.is_some() || descriptor.js.unsized_tail.is_some() {
        return None;
    }
    if !matches!(
        descriptor.js.enum_repr,
        Some(EnumRepr::Tag { fieldless: false } | EnumRepr::IntTag { fieldless: false })
    ) {
        return None;
    }
    let TypeKind::Adt {
        form: NominalForm::Enum,
        variants,
        ..
    } = &descriptor.kind
    else {
        return None;
    };
    if variants.len() <= 1 {
        return None;
    }
    let mut total_fields = 0usize;
    let mut out = Vec::with_capacity(variants.len());
    for variant in variants {
        if variant.fields.len() > MAX_FIELDS {
            return None;
        }
        let fields: Vec<TypeId> = variant.fields.iter().map(|f| f.ty).collect();
        if !shape_survives_copy(table, &fields) {
            return None;
        }
        total_fields = total_fields.saturating_add(fields.len());
        out.push(EnumVariantShape {
            logical_discr: variant.logical_discr,
            fields,
        });
    }
    if total_fields == 0 || total_fields > MAX_FIELDS * 4 {
        return None;
    }
    Some(EnumShape { variants: out })
}

/// Whether a node of this shape may be duplicated by sharing its field values.
///
/// A `Copy` of a scalarized node is spelled as a fresh node over the same values, which is Rust
/// `Copy` semantics exactly when no field is an object with an identity — an aggregate field would
/// be *aliased* by the copy, and writing through one of the two would show in the other.
pub fn shape_survives_copy(table: &TypeTable, fields: &[TypeId]) -> bool {
    !fields.iter().any(|field| table.needs_clone(*field))
}

/// Whether `OpKind::Field` can name every field of `ty`'s `variant`.
///
/// An array's fields are reached by index rather than by name — the type table lists one field
/// descriptor for the element — so a rewrite that would have to *read* an array's fields back out
/// of a real array has nothing to print, and the node escapes instead.
pub fn field_reads_resolve(table: &TypeTable, ty: TypeId, variant: u32, fields: usize) -> bool {
    let downcast = (variant != 0).then_some(variant);
    (0..fields as u32).all(|field| table.field(ty, downcast, field).is_some())
}

/// The width and signedness of the integer `[u8; N]` may be transmuted to, when `ty` is that array
/// type and `to` is that integer (`js/unsize.rs::bytes_to_int`).
pub fn byte_array_transmute(table: &TypeTable, ty: TypeId, to: TypeId) -> Option<(TypeId, u32)> {
    let Some(TypeKind::Array { element, length }) = table.get(ty).map(|d| &d.kind) else {
        return None;
    };
    if table.int_info(*element) != Some((false, 8)) {
        return None;
    }
    let (_, bits) = table.int_info(to)?;
    if !matches!(bits, 16 | 32 | 64) || *length != u64::from(bits / 8) {
        return None;
    }
    // Both ends have to be the plain types the emitter's cast path expects, with no transparent
    // wrapper to rebuild around the result.
    if !matches!(
        table.get(to).map(|d| &d.kind),
        Some(TypeKind::Integer { .. })
    ) {
        return None;
    }
    Some((*element, bits))
}
