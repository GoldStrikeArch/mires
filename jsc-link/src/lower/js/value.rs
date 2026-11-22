//! How Rust values are spelled in JavaScript, answered from the type table: the JS half of v1
//! `backend/src/value.rs`.
//!
//! Every question v1 asked `tcx` here is a stored fact on the [`TypeTable`] (`is_indirect`,
//! `needs_clone`, `enum_repr`, `int_info`, `peeled`, `field`); what is left is the spelling —
//! masks, literals, tags, the structural clone — and that is what this module holds. See the
//! baseline-carrier spec in `rcgjs-object/README.md` for the shapes.

use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::types::{
    EnumRepr, FieldKey, IntRepr, NominalForm, TypeId, TypeKind, TypeTable, VariantDescriptor,
};
use rcgjs_object::jsast::{self, BinOp, Expr};

use crate::plan::abi_flat::{LaneComponent, ProductNode, RecursiveShape, TaggedShape};

/// The key an enum object carries its variant name under (`value.rs:211`).
pub const TAG: &str = "TAG";

/// The number of enum variants past which a copy stops being rebuilt variant by variant
/// (`value.rs:592`).
const MAX_CLONED_VARIANTS: usize = 8;

/// Truncates `value` to an integer of the given width and signedness (`value.rs:902-916`).
pub fn mask(repr: IntRepr, signed: bool, bits: u32, value: Expr) -> Expr {
    match repr {
        IntRepr::BigInt => jsast::bigint_mask(signed, bits, value),
        IntRepr::Number => match (signed, bits) {
            (_, 33..) => value,
            (true, 32) => jsast::i32_wrap(value),
            (true, _) => jsast::sign_extend(bits, value),
            (false, 32) => jsast::u32_wrap(value),
            (false, _) => jsast::zero_extend(bits, value),
        },
    }
}

/// Truncates a value of type `ty` if `ty` is an integer, and leaves it alone otherwise.
pub fn mask_ty(table: &TypeTable, ty: TypeId, value: Expr) -> Expr {
    match table.int_info(ty) {
        Some((signed, bits)) => mask(table.int_repr(ty), signed, bits, value),
        None => value,
    }
}

/// Reinterprets the low `bits` bits of a raw bit pattern as an integer of that signedness
/// (`value.rs:923-929`).
pub fn resign(signed: bool, bits: u32, raw: u128) -> i128 {
    if signed && bits < 128 && bits > 0 && (raw >> (bits - 1)) & 1 == 1 {
        (raw as i128).wrapping_sub(1i128 << bits)
    } else {
        raw as i128
    }
}

/// A `BigInt` literal for a raw bit pattern of the given width and signedness.
pub fn wide_int_literal(signed: bool, bits: u32, raw: u128) -> Expr {
    if signed {
        jsast::bigint(resign(true, bits, raw))
    } else {
        jsast::bigint(raw)
    }
}

/// A literal for a raw integer bit pattern of type `ty` (`value.rs:948-955`); `None` when `ty`
/// is not an integer.
pub fn int_literal(table: &TypeTable, ty: TypeId, raw: u128) -> Option<Expr> {
    let (signed, bits) = table.int_info(ty)?;
    Some(match table.int_repr(ty) {
        IntRepr::BigInt => wide_int_literal(signed, bits, raw),
        IntRepr::Number => jsast::num(resign(signed, bits, raw) as f64),
    })
}

/// A literal for an already signed integer at width `bits`, in the representation `repr`.
pub fn signed_literal(repr: IntRepr, value: i128) -> Expr {
    match repr {
        IntRepr::BigInt => jsast::bigint(value),
        IntRepr::Number => jsast::num(value as f64),
    }
}

/// The name of one variant.
pub fn variant_name<'a>(interns: &'a Interns, variant: &VariantDescriptor) -> &'a str {
    interns.str(variant.name).unwrap_or("")
}

/// The JavaScript string standing for one variant of a tagged enum.
pub fn tag_literal(interns: &Interns, variant: &VariantDescriptor) -> Expr {
    jsast::string(variant_name(interns, variant))
}

/// The variants of the enum `ty` peels to (`TypeTable::variants` after peeling).
pub fn enum_variants(table: &TypeTable, ty: TypeId) -> &[VariantDescriptor] {
    table.variants(table.peeled(ty))
}

/// The discriminant of one variant, as the literal the value model spells it with
/// (`value.rs:340-352`). The representation follows the enum's own repr for a `Number` enum and
/// `discr_repr` (the discriminant type MIR reads) otherwise.
pub fn discriminant_literal(
    table: &TypeTable,
    ty: TypeId,
    variant: u32,
    discr_repr: IntRepr,
) -> Expr {
    let peeled = table.peeled(ty);
    let Some(descriptor) = table.variants(peeled).get(variant as usize) else {
        return jsast::num(0);
    };
    let repr = match table.enum_repr(peeled) {
        Some(EnumRepr::Number { bits, .. }) if bits > 32 => IntRepr::BigInt,
        Some(EnumRepr::Number { .. }) => IntRepr::Number,
        _ => discr_repr,
    };
    signed_literal(repr, descriptor.logical_discr)
}

/// The tag of one variant: the value that distinguishes it from the enum's others
/// (`value.rs:386-405`). `None` when `ty` is not an enum.
pub fn variant_tag(table: &TypeTable, interns: &Interns, ty: TypeId, variant: u32) -> Option<Expr> {
    let peeled = table.peeled(ty);
    Some(match table.enum_repr(peeled)? {
        EnumRepr::State => jsast::num(variant as f64),
        EnumRepr::Number { .. } => discriminant_literal(table, peeled, variant, IntRepr::Number),
        EnumRepr::Tag { .. } => tag_literal(interns, table.variants(peeled).get(variant as usize)?),
    })
}

/// How an enum value's tag is reached: the value itself where it *is* the tag, its `TAG`
/// otherwise (`value.rs:406-411`).
pub fn tag_expr(repr: EnumRepr, value: Expr) -> Expr {
    match repr {
        EnumRepr::Number { .. } | EnumRepr::Tag { fieldless: true } => value,
        EnumRepr::Tag { fieldless: false } | EnumRepr::State => jsast::member(value, TAG),
    }
}

/// The object keys read by a tagged physical product, tag first.
pub fn tagged_product_keys(
    table: &TypeTable,
    interns: &Interns,
    shape: &TaggedShape,
) -> Option<Vec<String>> {
    let mut keys = Vec::with_capacity(shape.fields() as usize);
    keys.push(TAG.to_owned());
    for lane in &shape.lanes {
        let field = table.field(shape.ty, Some(lane.variant), lane.field)?;
        let key = field_key_name(interns, field.key)?;
        keys.extend(std::iter::repeat_n(key, lane.node.fields() as usize));
    }
    Some(keys)
}

/// A canonical tagged enum value projected to `tag, payload...` expressions.
pub fn tagged_product_lanes(
    table: &TypeTable,
    interns: &Interns,
    shape: &TaggedShape,
    value: Expr,
) -> Option<Vec<Expr>> {
    let repr = table.enum_repr(table.peeled(shape.ty))?;
    let mut lanes = Vec::with_capacity(shape.fields() as usize);
    let tag = tag_expr(repr, value.clone());
    lanes.push(tag.clone());
    for lane in &shape.lanes {
        let field = table.field(shape.ty, Some(lane.variant), lane.field)?;
        let mut projected = Vec::with_capacity(lane.node.fields() as usize);
        project_product_node(
            table,
            interns,
            &lane.node,
            field_expr(interns, value.clone(), field.key),
            &mut projected,
        )?;
        // A fieldless variant has no payload property. Scalar reads of that absent property are
        // harmless `undefined`, but a nested payload would otherwise evaluate
        // `undefined.field` before its tag can be inspected. Guard the whole projection lazily;
        // keys present with the same shape in both variants need no guard.
        if lane.present_variants.len() < enum_variants(table, shape.ty).len() {
            let mut present = lane.present_variants.iter().map(|variant| {
                Some(jsast::binary(
                    BinOp::StrictEq,
                    tag.clone(),
                    variant_tag(table, interns, shape.ty, *variant)?,
                ))
            });
            let mut condition = present.next()??;
            for next in present {
                condition = jsast::binary(BinOp::Or, condition, next?);
            }
            lanes.extend(
                projected
                    .into_iter()
                    .map(|value| jsast::cond(condition.clone(), value, jsast::undefined())),
            );
        } else {
            lanes.extend(projected);
        }
    }
    Some(lanes)
}

/// A physical `tag, payload...` tuple rebuilt as the ordinary canonical enum object.
///
/// The expression is normally consumed immediately by field/discriminant reads, whose object
/// member folds erase it. A true whole-value use keeps the canonical materialization.
pub fn tagged_product_value(
    table: &TypeTable,
    interns: &Interns,
    shape: &TaggedShape,
    tag: Expr,
    payloads: Vec<Expr>,
) -> Option<Expr> {
    if payloads.len() + 1 != shape.fields() as usize {
        return None;
    }
    let mut payloads = payloads.into_iter();
    let mut entries = Vec::with_capacity(shape.lanes.len() + 1);
    entries.push((TAG.to_owned(), tag));
    for lane in &shape.lanes {
        let field = table.field(shape.ty, Some(lane.variant), lane.field)?;
        let key = field_key_name(interns, field.key)?;
        let value = rebuild_product_node(table, interns, &lane.node, &mut payloads)?;
        entries.push((key, value));
    }
    if payloads.next().is_some() {
        return None;
    }
    Some(jsast::object(entries))
}

/// The top-level keys a recursive product reads, one per physical lane.
pub fn recursive_product_keys(
    table: &TypeTable,
    interns: &Interns,
    shape: &RecursiveShape,
) -> Option<Vec<String>> {
    let ProductNode::Product { ty, fields } = &shape.root else {
        return None;
    };
    let mut keys = Vec::with_capacity(shape.fields() as usize);
    for (field, node) in fields.iter().enumerate() {
        let key = product_field_key(table, *ty, field as u32)?;
        let key = field_key_name(interns, key)?;
        keys.extend(std::iter::repeat_n(key, node.fields() as usize));
    }
    Some(keys)
}

/// A canonical recursive product projected to its physical leaf lanes.
pub fn recursive_product_lanes(
    table: &TypeTable,
    interns: &Interns,
    shape: &RecursiveShape,
    value: Expr,
) -> Option<Vec<Expr>> {
    let mut lanes = Vec::with_capacity(shape.fields() as usize);
    project_product_node(table, interns, &shape.root, value, &mut lanes)?;
    Some(lanes)
}

/// Physical recursive-product lanes rebuilt as their virtual canonical value.
pub fn recursive_product_value(
    table: &TypeTable,
    interns: &Interns,
    shape: &RecursiveShape,
    lanes: Vec<Expr>,
) -> Option<Expr> {
    if lanes.len() != shape.fields() as usize {
        return None;
    }
    let mut lanes = lanes.into_iter();
    let value = rebuild_product_node(table, interns, &shape.root, &mut lanes)?;
    lanes.next().is_none().then_some(value)
}

fn project_product_node(
    table: &TypeTable,
    interns: &Interns,
    node: &ProductNode,
    value: Expr,
    out: &mut Vec<Expr>,
) -> Option<()> {
    match node {
        ProductNode::Zst { .. } => {}
        ProductNode::Leaf { components, .. } => {
            for component in components {
                out.push(match component {
                    LaneComponent::Value => value.clone(),
                    LaneComponent::Buffer => rcgjs_object::ptr::slot_buf(value.clone()),
                    LaneComponent::Offset => rcgjs_object::ptr::slot_off(value.clone()),
                    LaneComponent::Length => rcgjs_object::ptr::slot_len(value.clone()),
                });
            }
        }
        ProductNode::Product { ty, fields } => {
            for (field, child) in fields.iter().enumerate() {
                let key = product_field_key(table, *ty, field as u32)?;
                project_product_node(
                    table,
                    interns,
                    child,
                    field_expr(interns, value.clone(), key),
                    out,
                )?;
            }
        }
    }
    Some(())
}

fn rebuild_product_node(
    table: &TypeTable,
    interns: &Interns,
    node: &ProductNode,
    lanes: &mut impl Iterator<Item = Expr>,
) -> Option<Expr> {
    match node {
        ProductNode::Zst { .. } => Some(jsast::undefined()),
        ProductNode::Leaf { components, .. } => match components.as_slice() {
            [LaneComponent::Value] => lanes.next(),
            [LaneComponent::Buffer, LaneComponent::Offset] => {
                Some(rcgjs_object::ptr::slot(lanes.next()?, lanes.next()?))
            }
            [LaneComponent::Buffer, LaneComponent::Offset, LaneComponent::Length] => Some(
                rcgjs_object::ptr::fat_slice(lanes.next()?, lanes.next()?, lanes.next()?),
            ),
            _ => None,
        },
        ProductNode::Product { ty, fields } => {
            let values = fields
                .iter()
                .map(|field| rebuild_product_node(table, interns, field, lanes))
                .collect::<Option<Vec<_>>>()?;
            match table.get(*ty).map(|descriptor| &descriptor.kind) {
                Some(TypeKind::Tuple(_) | TypeKind::Array { .. } | TypeKind::Closure { .. }) => {
                    Some(jsast::array(values))
                }
                Some(TypeKind::Adt {
                    form: NominalForm::Struct,
                    ..
                }) => {
                    let entries = values
                        .into_iter()
                        .enumerate()
                        .map(|(field, value)| {
                            let key = product_field_key(table, *ty, field as u32)?;
                            Some((field_key_name(interns, key)?, value))
                        })
                        .collect::<Option<Vec<_>>>()?;
                    Some(jsast::object(entries))
                }
                _ => None,
            }
        }
    }
}

fn product_field_key(table: &TypeTable, ty: TypeId, field: u32) -> Option<FieldKey> {
    match table.get(ty).map(|descriptor| &descriptor.kind) {
        Some(TypeKind::Array { length, .. }) if u64::from(field) < *length => {
            Some(FieldKey::Index(field))
        }
        _ => table.field(ty, None, field).map(|field| field.key),
    }
}

fn field_key_name(interns: &Interns, key: FieldKey) -> Option<String> {
    match key {
        FieldKey::Name(name) => Some(interns.str(name)?.to_owned()),
        FieldKey::Index(index) => Some(index.to_string()),
        FieldKey::Transparent => None,
    }
}

/// The **number** an enum value's discriminant is (`value.rs:422-460`): a chain
/// `t === "A" ? 0 : t === "B" ? 1 : 2` for a tagged enum. `None` when `ty` is not an enum.
pub fn discriminant_number(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    value: Expr,
    discr_repr: IntRepr,
) -> Option<Expr> {
    let peeled = table.peeled(ty);
    let repr = table.enum_repr(peeled)?;
    if matches!(repr, EnumRepr::Number { .. }) {
        return Some(value);
    }
    if repr == EnumRepr::State {
        return Some(tag_expr(repr, value));
    }
    let variants = table.variants(peeled);
    let tag = tag_expr(repr, value);
    let last = variants.len().checked_sub(1)?;
    let mut number = discriminant_literal(table, peeled, last as u32, discr_repr);
    for index in (0..last).rev() {
        number = jsast::cond(
            jsast::binary(
                BinOp::StrictEq,
                tag.clone(),
                tag_literal(interns, &variants[index]),
            ),
            discriminant_literal(table, peeled, index as u32, discr_repr),
            number,
        );
    }
    Some(number)
}

/// The JavaScript value of one variant of an enum, built out of that variant's already lowered
/// fields (`value.rs:358-378`). `None` when `ty` is not an enum.
pub fn enum_value(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    variant: u32,
    fields: Vec<(String, Expr)>,
) -> Option<Expr> {
    let peeled = table.peeled(ty);
    Some(match table.enum_repr(peeled)? {
        EnumRepr::Number { .. } | EnumRepr::Tag { fieldless: true } => {
            variant_tag(table, interns, peeled, variant)?
        }
        EnumRepr::Tag { fieldless: false } | EnumRepr::State => {
            let mut entries = Vec::with_capacity(fields.len() + 1);
            entries.push((
                TAG.to_string(),
                variant_tag(table, interns, peeled, variant)?,
            ));
            entries.extend(fields);
            jsast::object(entries)
        }
    })
}

/// The JavaScript key of a field descriptor's `FieldKey::Name`.
pub fn key_name(interns: &Interns, key: FieldKey) -> Option<String> {
    match key {
        FieldKey::Name(id) => Some(interns.str(id).unwrap_or("").to_owned()),
        _ => None,
    }
}

/// `base.name`, `base[i]`, or `base` itself for the field of a transparent wrapper
/// (`value.rs:851-857`).
pub fn field_expr(interns: &Interns, base: Expr, key: FieldKey) -> Expr {
    match key {
        FieldKey::Name(id) => jsast::member(base, interns.str(id).unwrap_or("")),
        FieldKey::Index(i) => jsast::index(base, jsast::num(i as f64)),
        FieldKey::Transparent => base,
    }
}

/// `Object.assign({}, value)`, a one level copy for a shape this module cannot take apart.
fn shallow_clone(value: Expr) -> Expr {
    jsast::call(
        jsast::member(jsast::id("Object"), "assign"),
        vec![jsast::object(vec![]), value],
    )
}

/// A copy of an aggregate value that shares nothing with the original (`value.rs:603-689`).
pub fn clone_expr(table: &TypeTable, interns: &Interns, ty: TypeId, value: Expr) -> Expr {
    let peeled = table.peeled(ty);
    if peeled != ty {
        if !table.needs_clone(peeled) {
            return value;
        }
        let copy = clone_expr(table, interns, peeled, value.clone());
        if !table.crosses_transparent_union(ty) {
            return copy;
        }
        return jsast::cond(
            jsast::binary(BinOp::StrictNe, value, jsast::undefined()),
            copy,
            jsast::undefined(),
        );
    }
    let clone_field = |field_ty: TypeId, read: Expr| {
        if table.needs_clone(field_ty) {
            clone_expr(table, interns, field_ty, read)
        } else {
            read
        }
    };
    let Some(descriptor) = table.get(ty) else {
        return shallow_clone(value);
    };
    match &descriptor.kind {
        TypeKind::Adt {
            form: NominalForm::Struct | NominalForm::Union,
            variants,
            ..
        } => {
            let Some(variant) = variants.first() else {
                return shallow_clone(value);
            };
            let entries = variant
                .fields
                .iter()
                .map(|field| {
                    let key = key_name(interns, field.key).unwrap_or_default();
                    let read = clone_field(field.ty, jsast::member(value.clone(), key.clone()));
                    (key, read)
                })
                .collect();
            jsast::object(entries)
        }
        TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            ..
        } => clone_enum(table, interns, variants, value),
        TypeKind::Tuple(_) | TypeKind::Closure { .. } => jsast::array(
            descriptor
                .js
                .fields
                .iter()
                .enumerate()
                .map(|(i, field)| {
                    clone_field(field.ty, jsast::index(value.clone(), jsast::num(i as f64)))
                })
                .collect(),
        ),
        TypeKind::Array { element, .. } if table.needs_clone(*element) => jsast::method_call(
            value,
            "map",
            vec![jsast::arrow(
                vec![CLONE_PARAM.to_string()],
                clone_expr(table, interns, *element, jsast::id(CLONE_PARAM)),
            )],
        ),
        TypeKind::Array { .. } => jsast::method_call(value, "slice", vec![]),
        _ => shallow_clone(value),
    }
}

/// The parameter of the clone arrow (`value.rs`, `ptr.rs`): `$`-prefixed so it cannot shadow a
/// local.
pub const CLONE_PARAM: &str = "$e";

/// A copy of an enum value: `TAG === "A" ? { TAG: "A", _0: ... } : ...` (`value.rs:700-758`).
fn clone_enum(
    table: &TypeTable,
    interns: &Interns,
    variants: &[VariantDescriptor],
    value: Expr,
) -> Expr {
    let payload_needs_clone = variants.iter().any(|variant| {
        variant
            .fields
            .iter()
            .any(|field| table.needs_clone(field.ty))
    });
    if !payload_needs_clone || variants.len() > MAX_CLONED_VARIANTS {
        return shallow_clone(value);
    }
    let variant_copy = |variant: &VariantDescriptor| {
        let mut entries = vec![(TAG.to_string(), tag_literal(interns, variant))];
        for field in &variant.fields {
            let key = key_name(interns, field.key).unwrap_or_default();
            let read = jsast::member(value.clone(), key.clone());
            let read = if table.needs_clone(field.ty) {
                clone_expr(table, interns, field.ty, read)
            } else {
                read
            };
            entries.push((key, read));
        }
        (tag_literal(interns, variant), jsast::object(entries))
    };
    let Some(last) = variants.last() else {
        return shallow_clone(value);
    };
    let mut copy = variant_copy(last).1;
    for variant in variants[..variants.len() - 1].iter().rev() {
        let (tag, rebuilt) = variant_copy(variant);
        copy = jsast::cond(
            jsast::binary(BinOp::StrictEq, jsast::member(value.clone(), TAG), tag),
            rebuilt,
            copy,
        );
    }
    copy
}

/// The empty container a value of `ty` is assembled in when MIR writes its fields one by one into a
/// local that was never assigned whole (`lower/assembly.rs`, v1 `value.rs::fresh_container`): `[]`
/// for a tuple, an array or the environment of a closure or an async closure, `{}` for a struct, a
/// union or an enum with a payload. `None` for every other value, including a zero-sized one and a
/// transparent wrapper around a scalar, whose field write is a write of the whole value.
pub fn fresh_container(table: &TypeTable, ty: TypeId) -> Option<Expr> {
    if table.is_zst(ty) {
        return None;
    }
    match kind(table, ty)? {
        TypeKind::Tuple(_)
        | TypeKind::Array { .. }
        | TypeKind::Closure { .. }
        | TypeKind::CoroutineClosure { .. } => Some(jsast::array(Vec::new())),
        TypeKind::Adt {
            form: NominalForm::Struct | NominalForm::Union,
            ..
        } => Some(jsast::object(Vec::new())),
        TypeKind::Adt {
            form: NominalForm::Enum,
            ..
        } if matches!(
            table.enum_repr(table.peeled(ty)),
            Some(EnumRepr::Tag { fieldless: false })
        ) =>
        {
            Some(jsast::object(Vec::new()))
        }
        _ => None,
    }
}

/// The value of an *uninitialized* place of type `ty` (`rvalue.rs:811-820`): `undefined`, or
/// `new Array(N)` for an array payload.
pub fn uninit(table: &TypeTable, ty: TypeId) -> Expr {
    match table.get(table.peeled(ty)).map(|d| &d.kind) {
        Some(TypeKind::Array { length, .. }) => {
            jsast::new(jsast::id("Array"), vec![jsast::num(*length as f64)])
        }
        _ => jsast::undefined(),
    }
}

/// The size of `ty` in bytes, or `None` when it has no layout or no statically known size.
pub fn size_of(table: &TypeTable, ty: TypeId) -> Option<u64> {
    let descriptor = table.get(ty)?;
    if !descriptor.layout.sized {
        return None;
    }
    descriptor.layout.size
}

/// The alignment of `ty` in bytes (a W8 seam: `min_align_of`).
#[allow(dead_code)]
pub fn align_of(table: &TypeTable, ty: TypeId) -> Option<u64> {
    table.get(ty)?.layout.align
}

/// Whether `ty` is a `Reference` or `RawPointer` after peeling.
pub fn is_pointer(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        table.get(table.peeled(ty)).map(|d| &d.kind),
        Some(TypeKind::Reference { .. } | TypeKind::RawPointer { .. })
    )
}

/// The pointee of a pointer type, after peeling.
pub fn pointee_of(table: &TypeTable, ty: TypeId) -> Option<TypeId> {
    match table.get(table.peeled(ty)).map(|d| &d.kind) {
        Some(TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. }) => {
            Some(*pointee)
        }
        _ => None,
    }
}

/// Whether the peeled type is a raw pointer (`ptr.rs::is_raw`).
pub fn is_raw(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        table.get(table.peeled(ty)).map(|d| &d.kind),
        Some(TypeKind::RawPointer { .. })
    )
}

/// The peeled kind of `ty`.
pub fn kind(table: &TypeTable, ty: TypeId) -> Option<&TypeKind> {
    table.get(table.peeled(ty)).map(|d| &d.kind)
}

/// Whether the type is `str` after peeling.
pub fn is_str(table: &TypeTable, ty: TypeId) -> bool {
    matches!(kind(table, ty), Some(TypeKind::Str))
}

/// Whether the peeled type is a mutable reference or mutable raw pointer (`&mut T` / `*mut T`).
pub fn is_mutable_ptr(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        kind(table, ty),
        Some(
            TypeKind::Reference { mutable: true, .. } | TypeKind::RawPointer { mutable: true, .. }
        )
    )
}

/// Whether the peeled type is a `str` pointer represented as a live `{ buf, off, len }` UTF-8 byte
/// slice (`&mut str`, `*const str`, `*mut str`, and transparent wrappers such as `NonNull<str>`,
/// `Unique<str>`, `Box<str>`) rather than an immutable JavaScript string (`&str`).
pub fn is_mut_str_ptr(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        kind(table, ty),
        Some(
            TypeKind::Reference {
                pointee,
                mutable: true,
            } | TypeKind::RawPointer { pointee, .. }
        ) if is_str(table, *pointee)
    )
}

/// Whether the peeled type is an immutable shared `&str` reference, represented as a JavaScript
/// string (`PointerForm::Str`).
pub fn is_const_str_ptr(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        kind(table, ty),
        Some(TypeKind::Reference {
            pointee,
            mutable: false,
        }) if is_str(table, *pointee)
    )
}

/// Whether the type is `bool` after peeling.
pub fn is_bool(table: &TypeTable, ty: TypeId) -> bool {
    matches!(kind(table, ty), Some(TypeKind::Bool))
}

/// Whether the type is `char` after peeling.
pub fn is_char(table: &TypeTable, ty: TypeId) -> bool {
    matches!(kind(table, ty), Some(TypeKind::Char))
}

/// The float width, if the type is a float after peeling.
pub fn float_bits(table: &TypeTable, ty: TypeId) -> Option<u32> {
    match kind(table, ty) {
        Some(TypeKind::Float { bits }) => Some(*bits),
        _ => None,
    }
}

/// Whether the type is `u8` after peeling.
pub fn is_u8(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        kind(table, ty),
        Some(TypeKind::Integer {
            signed: false,
            bits: 8
        })
    )
}

/// Whether `===` on two values of this type answers the question Rust's `==` asks
/// (`rvalue.rs:775-789`).
pub fn compares_by_value(table: &TypeTable, ty: TypeId) -> bool {
    if table.is_primitive_enum(ty) {
        return true;
    }
    matches!(
        table.get(ty).map(|d| &d.kind),
        Some(
            TypeKind::Integer { .. }
                | TypeKind::Float { .. }
                | TypeKind::Bool
                | TypeKind::Char
                | TypeKind::FnPtr { .. }
                | TypeKind::FnDef { .. }
        )
    )
}

/// Whether a *raw* pointer to `pointee` is a slot record (`ptr.rs::is_slot_pointee`): every
/// sized non-zero-sized pointee except `str`.
pub fn is_slot_pointee(table: &TypeTable, pointee: TypeId) -> bool {
    if is_str(table, pointee) {
        return false;
    }
    match table.get(pointee) {
        Some(descriptor) => descriptor.layout.sized && !descriptor.layout.zst,
        None => false,
    }
}

/// Whether a value of pointer type `pointer_ty` is a slot record (`ptr.rs::is_slot`).
pub fn is_slot(table: &TypeTable, pointer_ty: TypeId) -> bool {
    match kind(table, pointer_ty) {
        Some(TypeKind::Reference { pointee, .. }) => {
            !table.is_indirect(*pointee) && is_slot_pointee(table, *pointee)
        }
        Some(TypeKind::RawPointer { pointee, .. }) => is_slot_pointee(table, *pointee),
        _ => false,
    }
}

/// The JavaScript value a `ty` whose bytes are all zero has, if this lowering can spell one
/// (`alloc_support.rs::zero_value`, over the type table instead of a scratch allocation).
pub fn zero_value(table: &TypeTable, interns: &Interns, ty: TypeId) -> Option<Expr> {
    zero_value_at(table, interns, ty, 0)
}

fn zero_value_at(table: &TypeTable, interns: &Interns, ty: TypeId, depth: u32) -> Option<Expr> {
    if depth > 16 {
        return None;
    }
    let descriptor = table.get(ty)?;
    if !descriptor.layout.sized {
        return None;
    }
    if descriptor.layout.zst {
        return Some(jsast::undefined());
    }
    if let Some(inner) = descriptor.js.transparent {
        return zero_value_at(table, interns, inner.ty, depth + 1);
    }
    match &descriptor.kind {
        TypeKind::Bool => Some(jsast::boolean(false)),
        TypeKind::Char | TypeKind::Float { .. } => Some(jsast::num(0)),
        TypeKind::Integer { .. } => int_literal(table, ty, 0),
        // A zeroed pointer is the null address, which is the number zero.
        TypeKind::RawPointer { .. } | TypeKind::FnPtr { .. } => Some(jsast::num(0)),
        TypeKind::Pattern(base) => zero_value_at(table, interns, *base, depth + 1),
        TypeKind::Adt {
            form: NominalForm::Struct,
            variants,
            ..
        } => {
            let variant = variants.first()?;
            let mut entries = Vec::with_capacity(variant.fields.len());
            for field in &variant.fields {
                let key = key_name(interns, field.key)?;
                entries.push((key, zero_value_at(table, interns, field.ty, depth + 1)?));
            }
            Some(jsast::object(entries))
        }
        TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            niche,
            ..
        } => {
            let index = zero_variant(table, variants, niche.as_ref())?;
            let variant = variants.get(index as usize)?;
            let mut fields = Vec::with_capacity(variant.fields.len());
            for field in &variant.fields {
                let key = key_name(interns, field.key)?;
                fields.push((key, zero_value_at(table, interns, field.ty, depth + 1)?));
            }
            enum_value(table, interns, ty, index, fields)
        }
        TypeKind::Tuple(elements) => Some(jsast::array(
            elements
                .iter()
                .map(|element| zero_value_at(table, interns, *element, depth + 1))
                .collect::<Option<Vec<_>>>()?,
        )),
        TypeKind::Array { element, length } => {
            let zero = zero_value_at(table, interns, *element, depth + 1)?;
            Some(jsast::array((0..*length).map(|_| zero.clone()).collect()))
        }
        _ => None,
    }
}

/// The variant an all-zero enum value holds: the one whose direct tag is zero, or under a niche
/// the variant the bit pattern `0` names ([`niche_variant`]).
fn zero_variant(
    table: &TypeTable,
    variants: &[VariantDescriptor],
    niche: Option<&rcgjs_object::jir::types::Niche>,
) -> Option<u32> {
    if let Some(niche) = niche {
        return Some(niche_variant(table, variants, niche, 0));
    }
    if variants
        .iter()
        .all(|variant| variant.physical_tag.is_some())
    {
        return variants
            .iter()
            .position(|variant| variant.physical_tag.is_some_and(|tag| tag.bits == 0))
            .map(|position| position as u32);
    }
    // A single variant layout: the one variant.
    (variants.len() == 1).then_some(0)
}

/// Whether a type is uninhabited (`!`, a 0-variant enum, or an aggregate containing one).
pub fn is_uninhabited(table: &TypeTable, ty: TypeId) -> bool {
    is_uninhabited_at(table, ty, 0)
}

fn is_uninhabited_at(table: &TypeTable, ty: TypeId, depth: usize) -> bool {
    if depth > 16 {
        return false;
    }
    match kind(table, ty) {
        Some(TypeKind::Never) => true,
        Some(TypeKind::Pattern(base)) => is_uninhabited_at(table, *base, depth + 1),
        Some(TypeKind::Tuple(elements)) => elements
            .iter()
            .any(|element| is_uninhabited_at(table, *element, depth + 1)),
        Some(TypeKind::Array { element, length }) => {
            *length > 0 && is_uninhabited_at(table, *element, depth + 1)
        }
        Some(TypeKind::Adt {
            form: NominalForm::Struct,
            variants,
            ..
        }) => variants.first().is_some_and(|variant| {
            variant
                .fields
                .iter()
                .any(|field| is_uninhabited_at(table, field.ty, depth + 1))
        }),
        Some(TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            ..
        }) => variants.iter().all(|variant| {
            variant
                .fields
                .iter()
                .any(|field| is_uninhabited_at(table, field.ty, depth + 1))
        }),
        _ => false,
    }
}

/// The inhabited niche variants of a niche-encoded enum and the raw tag bit pattern each one
/// occupies at width `bits` (`(variant_index, raw_bits)`).
///
/// Matches rustc's `TagEncoding::Niche` numbering (`niche_start.wrapping_add(v - start) & mask`,
/// skipping `untagged_variant` and uninhabited variants while preserving their slot offsets).
pub fn niche_variant_tags(
    table: &TypeTable,
    variants: &[VariantDescriptor],
    niche: &rcgjs_object::jir::types::Niche,
    bits: u32,
) -> Option<Vec<(u32, u128)>> {
    let present = |index: usize| {
        !variants[index]
            .fields
            .iter()
            .any(|field| is_uninhabited(table, field.ty))
    };
    let start = (0..variants.len())
        .find(|index| *index as u32 != niche.untagged_variant && present(*index))?;
    let end = (0..variants.len())
        .rev()
        .find(|index| *index as u32 != niche.untagged_variant && present(*index))?;
    let mask = if bits >= 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    };
    let mut out = Vec::new();
    for index in start..=end {
        if index as u32 == niche.untagged_variant || !present(index) {
            continue;
        }
        let offset = (index - start) as u128;
        let raw_bits = niche.niche_start.wrapping_add(offset) & mask;
        out.push((index as u32, raw_bits));
    }
    (!out.is_empty()).then_some(out)
}

/// The variant a niche-encoded enum holds when its niche scalar reads `bits`
/// (`rustc_abi::TagEncoding::Niche`): the variant `niche_variants.start + (bits - niche_start)`
/// when that lands on a niche variant, the untagged variant otherwise.
///
/// The table records the untagged variant and `niche_start`; `niche_variants.start` is the first
/// present variant that is not the untagged one (rustc numbers the niche values from it, the
/// untagged variant's own slot in that numbering unused).
pub fn niche_variant(
    table: &TypeTable,
    variants: &[VariantDescriptor],
    niche: &rcgjs_object::jir::types::Niche,
    bits: u128,
) -> u32 {
    let present = |index: usize| {
        !variants[index]
            .fields
            .iter()
            .any(|field| is_uninhabited(table, field.ty))
    };
    let Some(start) = (0..variants.len())
        .find(|index| *index as u32 != niche.untagged_variant && present(*index))
    else {
        return niche.untagged_variant;
    };
    let Some(end) = (0..variants.len())
        .rev()
        .find(|index| *index as u32 != niche.untagged_variant && present(*index))
    else {
        return niche.untagged_variant;
    };
    // The width of the niche scalar decides how the subtraction wraps: the niche field's own
    // width when the path reaches an integer, the pointer width otherwise.
    let width = niche_width(table, variants, niche)
        .filter(|&w| w > 0)
        .unwrap_or(128);
    let mask = if width >= 128 {
        u128::MAX
    } else {
        (1u128 << width) - 1
    };
    let relative = bits.wrapping_sub(niche.niche_start) & mask;
    let count = (end - start + 1) as u128;
    if relative < count {
        let variant = start as u128 + relative;
        if variant as u32 != niche.untagged_variant && present(variant as usize) {
            return variant as u32;
        }
    }
    niche.untagged_variant
}

/// The width in bits of the scalar a niche lives in, following the niche field path down the
/// untagged variant.
pub fn niche_width(
    table: &TypeTable,
    variants: &[VariantDescriptor],
    niche: &rcgjs_object::jir::types::Niche,
) -> Option<u32> {
    niche_width_at(table, variants, niche, 0)
}

fn niche_width_at(
    table: &TypeTable,
    variants: &[VariantDescriptor],
    niche: &rcgjs_object::jir::types::Niche,
    depth: usize,
) -> Option<u32> {
    enum StepFields<'a> {
        Fields(&'a [rcgjs_object::jir::types::FieldDescriptor]),
        Single(TypeId),
        Array(TypeId, u64),
    }

    if depth > 16 {
        return None;
    }
    let mut step_fields =
        StepFields::Fields(&variants.get(niche.untagged_variant as usize)?.fields);
    let mut ty = None;
    for step in &niche.niche_field {
        // Follow the unpeeled field chain (`niche_facts` records rustc's unpeeled field indices,
        // including transparent wrappers like `NonZero<T>`, single-inhabited-variant enums, array
        // elements, and `ty::Pat` single-field steps).
        let next_ty = match step_fields {
            StepFields::Single(base) => {
                if *step != 0 {
                    return None;
                }
                base
            }
            StepFields::Array(element, length) => {
                if u64::from(*step) >= length {
                    return None;
                }
                element
            }
            StepFields::Fields(fields) => fields.get(*step as usize)?.ty,
        };
        ty = Some(next_ty);
        let descriptor = table.get(next_ty)?;
        step_fields = match &descriptor.kind {
            TypeKind::Pattern(base) => StepFields::Single(*base),
            TypeKind::Array { element, length } => StepFields::Array(*element, *length),
            TypeKind::Adt {
                form: NominalForm::Enum,
                variants,
                ..
            } => {
                // Only a single-inhabited-variant enum (`Variants::Single`) has an unambiguous
                // field list to descend into; 0 or >1 inhabited variants stop the path.
                let mut inhabited = variants.iter().filter(|variant| {
                    !variant
                        .fields
                        .iter()
                        .any(|field| is_uninhabited(table, field.ty))
                });
                match (inhabited.next(), inhabited.next()) {
                    (Some(sole), None) => StepFields::Fields(sole.fields.as_slice()),
                    _ => StepFields::Fields(&[]),
                }
            }
            TypeKind::Adt { variants, .. } => {
                StepFields::Fields(variants.first().map_or(&[], |v| v.fields.as_slice()))
            }
            _ => StepFields::Fields(&descriptor.js.fields),
        };
    }
    let ty = ty?;
    let peeled = table.peeled(ty);
    if let Some((_, bits)) = table.int_info(peeled) {
        return (bits > 0).then_some(bits);
    }
    match kind(table, peeled) {
        Some(TypeKind::Bool) => Some(8),
        Some(TypeKind::Char) => Some(32),
        Some(TypeKind::Reference { .. } | TypeKind::RawPointer { .. } | TypeKind::FnPtr { .. }) => {
            Some(32)
        }
        Some(TypeKind::Adt {
            form: NominalForm::Enum,
            variants: inner_variants,
            niche: Some(inner_niche),
            ..
        }) => niche_width_at(table, inner_variants, inner_niche, depth + 1),
        _ => size_of(table, peeled)
            .filter(|&bytes| bytes > 0)
            .map(|bytes| (bytes * 8) as u32),
    }
}

/// The variant of a `Number` enum whose logical discriminant is `discr`.
pub fn variant_with_logical_discr(table: &TypeTable, ty: TypeId, discr: i128) -> Option<u32> {
    enum_variants(table, ty)
        .iter()
        .position(|variant| variant.logical_discr == discr)
        .map(|position| position as u32)
}
