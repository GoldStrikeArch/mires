//! Operator, cast, and constant evaluation shared by the two interpreters.

use crate::jir::interns::Interns;
use crate::jir::semantic::{BinaryOp, CastKind, ConstTree, GlobalReference, TypeId, UnaryOp};
use crate::jir::types::{EnumRepr, TypeKind, TypeTable};

use super::heap::{field_count, pointee_type, Heap};
use super::value::{int_range, normalize_int, truncate_bits, Value};
use super::Trap;

/// The `(signed, bits)` of an integer type, or of the discriminant type of a `Number` enum.
fn int_kind(ty: TypeId, table: &TypeTable) -> Option<(bool, u32)> {
    if let Some(info) = table.int_info(ty) {
        return Some(info);
    }
    match table.enum_repr(ty) {
        Some(EnumRepr::Number { bits, signed }) => Some((signed, bits)),
        _ => None,
    }
}

/// A scalar of type `ty` from raw bits.
pub fn scalar(bits: u128, ty: TypeId, table: &TypeTable) -> Result<Value, Trap> {
    let descriptor = table
        .get(ty)
        .ok_or(Trap::Malformed("scalar constant at an unknown type"))?;
    match &descriptor.kind {
        TypeKind::Bool => Ok(Value::Bool(bits != 0)),
        TypeKind::Char => char::from_u32(bits as u32)
            .map(Value::Char)
            .ok_or(Trap::Ub("invalid char constant")),
        TypeKind::Integer {
            signed,
            bits: width,
        } => Ok(Value::int(bits, *width, *signed)),
        TypeKind::Float { bits: 32 } => {
            Ok(Value::float(f64::from(f32::from_bits(bits as u32)), 32))
        }
        TypeKind::Float { bits: 64 } => Ok(Value::float(f64::from_bits(bits as u64), 64)),
        TypeKind::Float { .. } => Err(Trap::Unsupported("f16/f128 constant")),
        TypeKind::Pattern(base) => scalar(bits, *base, table),
        TypeKind::Adt { variants, .. } => {
            if let Some(inner) = descriptor.js.transparent {
                let field = scalar(bits, inner.ty, table)?;
                let count = variants.first().map_or(1, |variant| variant.fields.len());
                let mut fields = vec![Value::Unit; count.max(1)];
                fields[inner.field as usize] = field;
                return Ok(Value::Aggregate {
                    ty,
                    variant: 0,
                    fields,
                });
            }
            let variant = table
                .variant_with_discriminant(ty, bits)
                .ok_or(Trap::Ub("scalar constant names no variant of the enum"))?;
            Ok(Value::Aggregate {
                ty,
                variant,
                fields: Vec::new(),
            })
        }
        TypeKind::Reference { .. } | TypeKind::RawPointer { .. } | TypeKind::FnPtr { .. } => {
            Err(Trap::Unsupported("integer-valued pointer constant"))
        }
        _ => Err(Trap::Unsupported("scalar constant at a non-scalar type")),
    }
}

/// The value of a zero-sized type: `()` for the unit tuple, an aggregate of zero-sized fields
/// otherwise.
pub fn zero_sized(ty: TypeId, table: &TypeTable) -> Result<Value, Trap> {
    let descriptor = table
        .get(ty)
        .ok_or(Trap::Malformed("zero-sized constant at an unknown type"))?;
    match &descriptor.kind {
        TypeKind::Tuple(elements) if elements.is_empty() => Ok(Value::Unit),
        TypeKind::FnDef { .. } => Ok(Value::Opaque("function item value")),
        TypeKind::Never => Err(Trap::Ub("value of the never type")),
        TypeKind::Adt { variants, .. } => {
            let variant = 0;
            let fields = variants
                .first()
                .map(|variant| {
                    variant
                        .fields
                        .iter()
                        .map(|field| field.ty)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let fields = fields
                .into_iter()
                .map(|field| zero_sized(field, table))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::Aggregate {
                ty,
                variant,
                fields,
            })
        }
        TypeKind::Tuple(elements) => Ok(Value::Aggregate {
            ty,
            variant: 0,
            fields: elements
                .iter()
                .map(|element| zero_sized(*element, table))
                .collect::<Result<Vec<_>, _>>()?,
        }),
        TypeKind::Array { element, length } => Ok(Value::Aggregate {
            ty,
            variant: 0,
            fields: vec![zero_sized(*element, table)?; *length as usize],
        }),
        TypeKind::Closure { upvars, .. } => Ok(Value::Aggregate {
            ty,
            variant: 0,
            fields: upvars
                .iter()
                .map(|upvar| zero_sized(*upvar, table))
                .collect::<Result<Vec<_>, _>>()?,
        }),
        _ if descriptor.layout.zst => Ok(Value::Unit),
        _ => Err(Trap::Unsupported("zero-sized constant at a sized type")),
    }
}

/// The value of a constant tree at type `ty`.
pub fn const_tree(
    tree: &ConstTree,
    ty: TypeId,
    table: &TypeTable,
    interns: &Interns,
) -> Result<Value, Trap> {
    let descriptor = table
        .get(ty)
        .ok_or(Trap::Malformed("constant at an unknown type"))?;
    // The producer encodes transparent constants at the field's type. The interpreter
    // keeps semantic aggregates, so reconstruct the wrapper around that field value.
    if let Some(inner) = descriptor.js.transparent {
        let value = const_tree(tree, inner.ty, table, interns)?;
        let count = field_count(ty, 0, table)
            .ok_or(Trap::Malformed("transparent constant without fields"))?;
        if inner.field as usize >= count {
            return Err(Trap::Malformed(
                "transparent constant names a missing field",
            ));
        }
        let mut fields = Vec::with_capacity(count);
        for index in 0..count {
            fields.push(if index == inner.field as usize {
                value.clone()
            } else {
                let field_ty = super::heap::field_type(ty, Some(0), index as u32, table)
                    .ok_or(Trap::Malformed("transparent constant field type missing"))?;
                if table.get(field_ty).is_some_and(|field| field.layout.zst) {
                    zero_sized(field_ty, table)?
                } else {
                    Value::Undef
                }
            });
        }
        return Ok(Value::Aggregate {
            ty,
            variant: 0,
            fields,
        });
    }
    if let TypeKind::Pattern(base) = descriptor.kind {
        return const_tree(tree, base, table, interns);
    }
    match tree {
        ConstTree::Unit => zero_sized(ty, table),
        ConstTree::Scalar { bits, .. } => scalar(*bits, ty, table),
        ConstTree::Str(id) => interns
            .str(*id)
            .map(|s| Value::Str(s.to_owned()))
            .ok_or(Trap::Malformed("string constant names a missing string")),
        ConstTree::Aggregate(fields) => {
            let descriptor = table
                .get(ty)
                .ok_or(Trap::Malformed("aggregate constant at an unknown type"))?;
            let field_types: Vec<TypeId> = match &descriptor.kind {
                TypeKind::Adt { variants, .. } => variants
                    .first()
                    .map(|variant| variant.fields.iter().map(|field| field.ty).collect())
                    .unwrap_or_default(),
                TypeKind::Tuple(elements) => elements.clone(),
                TypeKind::Array { element, .. } => vec![*element; fields.len()],
                TypeKind::Closure { upvars, .. } => upvars.clone(),
                _ => {
                    return Err(Trap::Unsupported(
                        "aggregate constant at a non-aggregate type",
                    ))
                }
            };
            if field_types.len() != fields.len() {
                return Err(Trap::Malformed(
                    "aggregate constant with the wrong field count",
                ));
            }
            Ok(Value::Aggregate {
                ty,
                variant: 0,
                fields: fields
                    .iter()
                    .zip(field_types)
                    .map(|(field, field_ty)| const_tree(field, field_ty, table, interns))
                    .collect::<Result<Vec<_>, _>>()?,
            })
        }
        ConstTree::Variant { variant, fields } => {
            let variants = table.variants(ty);
            let field_types: Vec<TypeId> = variants
                .get(*variant as usize)
                .ok_or(Trap::Malformed("variant constant names a missing variant"))?
                .fields
                .iter()
                .map(|field| field.ty)
                .collect();
            if field_types.len() != fields.len() {
                return Err(Trap::Malformed(
                    "variant constant with the wrong field count",
                ));
            }
            Ok(Value::Aggregate {
                ty,
                variant: *variant,
                fields: fields
                    .iter()
                    .zip(field_types)
                    .map(|(field, field_ty)| const_tree(field, field_ty, table, interns))
                    .collect::<Result<Vec<_>, _>>()?,
            })
        }
        ConstTree::Global {
            target: GlobalReference::Function { link_name, .. },
            ..
        } => Ok(Value::Fn(*link_name)),
        ConstTree::Reference(_)
        | ConstTree::Slice(_)
        | ConstTree::Global { .. }
        | ConstTree::Address(_) => Err(Trap::Unsupported("constant memory reference")),
    }
}

/// `left op right` where the operands have type `ty` and the result type `result_ty`.
pub fn binary(
    op: BinaryOp,
    left: &Value,
    right: &Value,
    ty: TypeId,
    result_ty: TypeId,
    table: &TypeTable,
) -> Result<Value, Trap> {
    if let (Some(l), Some(r)) = (int_of(left), int_of(right)) {
        let (signed, bits) = int_kind(ty, table)
            .or(match left {
                Value::Int { bits, signed, .. } => Some((*signed, *bits)),
                Value::Bool(_) => Some((false, 8)),
                Value::Char(_) => Some((false, 32)),
                _ => None,
            })
            .ok_or(Trap::Unsupported("integer operator at a non-integer type"))?;
        return binary_int(op, l, r, signed, bits, left, right, ty, result_ty, table);
    }
    match (op, left, right) {
        (_, Value::Float { value: l, bits }, Value::Float { value: r, .. }) => {
            let (l, r) = (*l, *r);
            let bits = *bits;
            Ok(match op {
                BinaryOp::Add => Value::float(l + r, bits),
                BinaryOp::Sub => Value::float(l - r, bits),
                BinaryOp::Mul => Value::float(l * r, bits),
                BinaryOp::Div => Value::float(l / r, bits),
                BinaryOp::Rem => Value::float(l % r, bits),
                BinaryOp::Eq => Value::Bool(l == r),
                BinaryOp::Ne => Value::Bool(l != r),
                BinaryOp::Lt => Value::Bool(l < r),
                BinaryOp::Le => Value::Bool(l <= r),
                BinaryOp::Gt => Value::Bool(l > r),
                BinaryOp::Ge => Value::Bool(l >= r),
                _ => return Err(Trap::Unsupported("float operator")),
            })
        }
        (BinaryOp::Eq, Value::Ptr(l), Value::Ptr(r)) => Ok(Value::Bool(l == r)),
        (BinaryOp::Ne, Value::Ptr(l), Value::Ptr(r)) => Ok(Value::Bool(l != r)),
        (BinaryOp::Eq, Value::Str(l), Value::Str(r)) => Ok(Value::Bool(l == r)),
        (BinaryOp::Ne, Value::Str(l), Value::Str(r)) => Ok(Value::Bool(l != r)),
        (BinaryOp::Offset, Value::Ptr(location), Value::Int { value, .. }) => {
            let mut location = location.clone();
            match location.path.last_mut() {
                Some(super::value::Step::Index(index)) => {
                    let moved = i128::from(*index) + value;
                    if moved < 0 {
                        return Err(Trap::Ub("pointer offset before the array"));
                    }
                    *index = moved as u64;
                    Ok(Value::Ptr(location))
                }
                _ if *value == 0 => Ok(Value::Ptr(location)),
                _ => Err(Trap::Ub("pointer offset outside an array")),
            }
        }
        _ => Err(Trap::Unsupported("binary operator on these operand kinds")),
    }
}

fn int_of(value: &Value) -> Option<i128> {
    match value {
        Value::Int { value, .. } => Some(*value),
        Value::Bool(b) => Some(i128::from(*b)),
        Value::Char(c) => Some(i128::from(u32::from(*c))),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn binary_int(
    op: BinaryOp,
    l: i128,
    r: i128,
    signed: bool,
    bits: u32,
    left: &Value,
    right: &Value,
    ty: TypeId,
    result_ty: TypeId,
    table: &TypeTable,
) -> Result<Value, Trap> {
    let wrap = |v: i128| Value::Int {
        value: normalize_int(v, bits, signed),
        bits,
        signed,
    };
    let (min, max) = int_range(bits, signed);
    let overflowing = |v: i128, wide_overflow: bool| -> Value {
        let overflow = wide_overflow || v < min || v > max;
        Value::Aggregate {
            ty: result_ty,
            variant: 0,
            fields: vec![wrap(v), Value::Bool(overflow)],
        }
    };
    let shift_amount = || {
        // MIR `Shl`/`Shr` mask the amount to the width; the unchecked forms are UB past it.
        ((r as u128) % u128::from(bits)) as u32
    };
    let bool_result = |b: bool| Ok(Value::Bool(b));
    match op {
        BinaryOp::Add | BinaryOp::AddUnchecked => Ok(wrap(l.wrapping_add(r))),
        BinaryOp::Sub | BinaryOp::SubUnchecked => Ok(wrap(l.wrapping_sub(r))),
        BinaryOp::Mul | BinaryOp::MulUnchecked => {
            let (v, wide) = l.overflowing_mul(r);
            if wide && bits == 128 {
                return Ok(wrap(v));
            }
            Ok(wrap(v))
        }
        BinaryOp::AddWithOverflow => {
            let (v, wide) = l.overflowing_add(r);
            Ok(overflowing(v, wide))
        }
        BinaryOp::SubWithOverflow => {
            let (v, wide) = l.overflowing_sub(r);
            Ok(overflowing(v, wide))
        }
        BinaryOp::MulWithOverflow => {
            let (v, wide) = l.overflowing_mul(r);
            if bits == 128 && !signed {
                // Unsigned 128-bit products need more than an i128 to detect overflow exactly.
                let (v, wide) = (l as u128).overflowing_mul(r as u128);
                return Ok(overflowing(v as i128, wide));
            }
            Ok(overflowing(v, wide))
        }
        BinaryOp::Div => {
            if r == 0 {
                return Err(Trap::Ub("division by zero"));
            }
            if bits == 128 && !signed {
                return Ok(wrap(((l as u128) / (r as u128)) as i128));
            }
            Ok(wrap(l.wrapping_div(r)))
        }
        BinaryOp::Rem => {
            if r == 0 {
                return Err(Trap::Ub("remainder by zero"));
            }
            if bits == 128 && !signed {
                return Ok(wrap(((l as u128) % (r as u128)) as i128));
            }
            Ok(wrap(l.wrapping_rem(r)))
        }
        BinaryOp::BitXor => Ok(wrap(l ^ r)),
        BinaryOp::BitAnd => Ok(wrap(l & r)),
        BinaryOp::BitOr => Ok(wrap(l | r)),
        BinaryOp::Shl | BinaryOp::ShlUnchecked => {
            let amount = shift_amount();
            Ok(wrap(if amount >= 128 {
                0
            } else {
                l.wrapping_shl(amount)
            }))
        }
        BinaryOp::Shr | BinaryOp::ShrUnchecked => {
            let amount = shift_amount();
            if signed {
                Ok(wrap(l.wrapping_shr(amount)))
            } else {
                Ok(wrap(((truncate_bits(l as u128, bits)) >> amount) as i128))
            }
        }
        BinaryOp::Eq => bool_result(l == r),
        BinaryOp::Ne => bool_result(l != r),
        BinaryOp::Lt => bool_result(l < r),
        BinaryOp::Le => bool_result(l <= r),
        BinaryOp::Gt => bool_result(l > r),
        BinaryOp::Ge => bool_result(l >= r),
        BinaryOp::Cmp => {
            let ordering: i128 = match l.cmp(&r) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            };
            let variant = table
                .variant_with_discriminant(result_ty, ordering as u128)
                .ok_or(Trap::Unsupported(
                    "Cmp without an Ordering type in the table",
                ))?;
            Ok(Value::Aggregate {
                ty: result_ty,
                variant,
                fields: Vec::new(),
            })
        }
        BinaryOp::Offset => {
            let _ = (left, right, ty);
            Err(Trap::Unsupported("integer Offset"))
        }
    }
}

/// `op v` at operand type `ty`.
pub fn unary(op: UnaryOp, v: &Value) -> Result<Value, Trap> {
    match (op, v) {
        (UnaryOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
        (
            UnaryOp::Not,
            Value::Int {
                value,
                bits,
                signed,
            },
        ) => Ok(Value::Int {
            value: normalize_int(!value, *bits, *signed),
            bits: *bits,
            signed: *signed,
        }),
        (
            UnaryOp::Neg,
            Value::Int {
                value,
                bits,
                signed,
            },
        ) => Ok(Value::Int {
            value: normalize_int(value.wrapping_neg(), *bits, *signed),
            bits: *bits,
            signed: *signed,
        }),
        (UnaryOp::Neg, Value::Float { value, bits }) => Ok(Value::float(-value, *bits)),
        (UnaryOp::PtrMetadata, Value::Slice { len, .. }) => Ok(Value::usize(*len)),
        (UnaryOp::PtrMetadata, Value::Str(s)) => Ok(Value::usize(s.len() as u64)),
        (UnaryOp::PtrMetadata, Value::Ptr(_)) => Ok(Value::Unit),
        (UnaryOp::PtrMetadata, Value::Aggregate { fields, .. }) if fields.len() == 1 => {
            unary(op, &fields[0])
        }
        _ => Err(Trap::Unsupported("unary operator on this operand kind")),
    }
}

/// `v as to`.
pub fn cast(
    kind: CastKind,
    v: &Value,
    to: TypeId,
    table: &TypeTable,
    heap: &Heap,
) -> Result<Value, Trap> {
    let target = table
        .get(to)
        .ok_or(Trap::Malformed("cast to an unknown type"))?;
    match kind {
        CastKind::IntToInt => {
            let bits_value: u128 = match v {
                Value::Int { value, bits, .. } => truncate_bits(*value as u128, *bits),
                Value::Bool(b) => u128::from(*b),
                Value::Char(c) => u128::from(u32::from(*c)),
                Value::Aggregate { ty, variant, .. } => {
                    match table.variants(*ty).get(*variant as usize) {
                        Some(descriptor) => descriptor.logical_discr as u128,
                        None => return Err(Trap::Ub("cast of an enum with a missing variant")),
                    }
                }
                _ => return Err(Trap::Unsupported("IntToInt from a non-integer")),
            };
            // A signed source sign-extends into a wider target.
            let source_signed_value: i128 = match v {
                Value::Int { value, .. } => *value,
                _ => bits_value as i128,
            };
            match &target.kind {
                TypeKind::Integer { signed, bits } => {
                    Ok(Value::int(source_signed_value as u128, *bits, *signed))
                }
                TypeKind::Char => char::from_u32(bits_value as u32)
                    .map(Value::Char)
                    .ok_or(Trap::Ub("cast to an invalid char")),
                TypeKind::Bool => Ok(Value::Bool(bits_value != 0)),
                _ => Err(Trap::Unsupported("IntToInt to a non-integer type")),
            }
        }
        CastKind::FloatToInt => {
            let Value::Float { value, .. } = v else {
                return Err(Trap::Unsupported("FloatToInt from a non-float"));
            };
            let TypeKind::Integer { signed, bits } = target.kind else {
                return Err(Trap::Unsupported("FloatToInt to a non-integer"));
            };
            let (min, max) = int_range(bits, signed);
            let truncated = value.trunc();
            let clamped: i128 = if truncated.is_nan() {
                0
            } else if truncated >= max as f64 {
                max
            } else if truncated <= min as f64 {
                min
            } else {
                truncated as i128
            };
            Ok(Value::Int {
                value: clamped,
                bits,
                signed,
            })
        }
        CastKind::IntToFloat => {
            let TypeKind::Float { bits } = target.kind else {
                return Err(Trap::Unsupported("IntToFloat to a non-float"));
            };
            let value = match v {
                Value::Int {
                    value,
                    signed,
                    bits: from_bits,
                    ..
                } => {
                    if *signed || *from_bits < 128 {
                        *value as f64
                    } else {
                        (*value as u128) as f64
                    }
                }
                _ => return Err(Trap::Unsupported("IntToFloat from a non-integer")),
            };
            Ok(Value::float(value, bits))
        }
        CastKind::FloatToFloat => {
            let (Value::Float { value, .. }, TypeKind::Float { bits }) = (v, &target.kind) else {
                return Err(Trap::Unsupported("FloatToFloat on non-floats"));
            };
            Ok(Value::float(*value, *bits))
        }
        CastKind::PointerCoercion
        | CastKind::PtrToPtr
        | CastKind::Subtype
        | CastKind::FnPtrToPtr => unsize_or_identity(v, to, table, heap),
        CastKind::Transmute => match (v, &target.kind) {
            (
                Value::Int { value, bits, .. },
                TypeKind::Integer {
                    signed,
                    bits: to_bits,
                },
            ) if bits == to_bits => Ok(Value::int(*value as u128, *to_bits, *signed)),
            (
                Value::Int {
                    value, bits: 32, ..
                },
                TypeKind::Float { bits: 32 },
            ) => Ok(Value::float(f64::from(f32::from_bits(*value as u32)), 32)),
            (
                Value::Int {
                    value, bits: 64, ..
                },
                TypeKind::Float { bits: 64 },
            ) => Ok(Value::float(f64::from_bits(*value as u64), 64)),
            (Value::Float { value, bits: 32 }, TypeKind::Integer { signed, bits: 32 }) => Ok(
                Value::int(u128::from((*value as f32).to_bits()), 32, *signed),
            ),
            (Value::Float { value, bits: 64 }, TypeKind::Integer { signed, bits: 64 }) => {
                Ok(Value::int(u128::from(value.to_bits()), 64, *signed))
            }
            _ => Err(Trap::Unsupported("transmute between these types")),
        },
        CastKind::PointerExposeProvenance | CastKind::PointerWithExposedProvenance => {
            Err(Trap::Unsupported("pointer/integer provenance cast"))
        }
    }
}

/// An array pointer to slice pointer unsizing (the array length read from the heap), or the
/// pointer itself for every other pointer cast the abstract model does not distinguish.
fn unsize_or_identity(
    v: &Value,
    to: TypeId,
    table: &TypeTable,
    heap: &Heap,
) -> Result<Value, Trap> {
    let Some(pointee) = pointee_type(to, table) else {
        return Ok(v.clone());
    };
    let Some(pointee_descriptor) = table.get(pointee) else {
        return Ok(v.clone());
    };
    match (&pointee_descriptor.kind, v) {
        (TypeKind::Slice(_), Value::Ptr(location)) => {
            let len = match heap.get(location)? {
                Value::Aggregate { fields, .. } => fields.len() as u64,
                _ => return Err(Trap::Ub("unsizing a pointer to a non-array")),
            };
            Ok(Value::Slice {
                buf: location.clone(),
                off: 0,
                len,
            })
        }
        (TypeKind::Slice(_), Value::Slice { .. }) => Ok(v.clone()),
        (TypeKind::Dynamic { .. }, _) => Err(Trap::Unsupported("unsize to dyn")),
        _ => Ok(v.clone()),
    }
}

/// The logical discriminant of an aggregate, as an integer of `result_ty`.
pub fn discriminant(v: &Value, result_ty: TypeId, table: &TypeTable) -> Result<Value, Trap> {
    let Value::Aggregate { ty, variant, .. } = v else {
        return Err(Trap::Unsupported("discriminant of a non-aggregate"));
    };
    let discr = table
        .variants(*ty)
        .get(*variant as usize)
        .map_or(0, |descriptor| descriptor.logical_discr);
    let (signed, bits) = table
        .int_info(result_ty)
        .ok_or(Trap::Unsupported("discriminant at a non-integer type"))?;
    Ok(Value::int(discr as u128, bits, signed))
}

/// Sets the active variant of an aggregate in place, keeping the fields already written and
/// resizing to the variant's field count.
pub fn set_discriminant(
    slot: &mut Value,
    variant: u32,
    ty: TypeId,
    table: &TypeTable,
) -> Result<(), Trap> {
    let count = field_count(ty, variant, table).unwrap_or(0);
    match slot {
        Value::Aggregate {
            variant: active,
            fields,
            ..
        } => {
            *active = variant;
            fields.resize(count, Value::Undef);
        }
        _ => {
            *slot = Value::Aggregate {
                ty,
                variant,
                fields: vec![Value::Undef; count],
            };
        }
    }
    Ok(())
}
