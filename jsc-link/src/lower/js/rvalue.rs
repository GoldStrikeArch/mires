//! Operators, casts, aggregates and discriminants (v1 `backend/src/rvalue.rs` and the coercion
//! rows of `unsize.rs` that need no vtable name).

use std::collections::BTreeSet;

use rcgjs_object::evaluation::OperandEvaluation;
use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::repr::{Func, OpKind, ValueId};
use rcgjs_object::jir::semantic::{BinaryOp, CastKind, UnaryOp};
use rcgjs_object::jir::types::{
    EnumRepr, FieldKey, IntRepr, NominalForm, TypeId, TypeKind, TypeTable,
};
use rcgjs_object::jsast::{self, BinOp as JsBinOp, Expr};
use rcgjs_object::ptr;

use super::call::Lowered;
use super::value::{self, mask, mask_ty, TAG};
use super::{decline, JsCx, Res};

/// The JavaScript operator of an overflow-reporting operator, applied at full precision.
fn overflow_operator(op: BinaryOp) -> Option<JsBinOp> {
    match op {
        BinaryOp::AddWithOverflow => Some(JsBinOp::Add),
        BinaryOp::SubWithOverflow => Some(JsBinOp::Sub),
        BinaryOp::MulWithOverflow => Some(JsBinOp::Mul),
        _ => None,
    }
}

/// Whether `op` at `ty` can carry its `(T, bool)` pair as the exact JavaScript result `x` alone:
/// the wrapped half is then `mask(x)` and the flag `mask(x) !== x`.
///
/// Every sum and difference can: two integers of at most 32 bits add exactly in a double, and a
/// `BigInt` result is exact by construction. So can a product of at most 16 bits (below 2^32) or
/// in `BigInt`. A 32 bit product cannot — the double rounds away the low bits a wrap keeps — and
/// keeps the array form of [`JsCx::binary`].
fn exact_overflow(table: &TypeTable, op: BinaryOp, ty: TypeId) -> bool {
    let Some((_, bits)) = table.int_info(ty) else {
        return false;
    };
    let limit = if op == BinaryOp::MulWithOverflow {
        16
    } else {
        32
    };
    overflow_operator(op).is_some() && (table.int_repr(ty) == IntRepr::BigInt || bits <= limit)
}

/// The overflow pairs of `func` that are never built: results of `AddWithOverflow` and its
/// siblings that [`exact_overflow`] admits and that every use takes apart with a `Field`
/// projection, the `let (r, o) = a.overflowing_add(b)` and `checked_*` shape.
///
/// Such a value is carried as its exact result (`JsCx::op_expr` spells only `l op r`), and each
/// projection reads its half off that ([`JsCx::overflow_field`]). Any other use — a block
/// argument, a copy, a store, a call argument, a return — needs the real pair and keeps the
/// array. Unsigned sums usually never get here: `opt::dce` already splits a projected one into a
/// wrapped add and a comparison.
pub(super) fn exact_overflow_pairs(func: &Func, table: &TypeTable) -> BTreeSet<ValueId> {
    let mut pairs: BTreeSet<ValueId> = func
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter_map(|op| match (op.result, &op.kind) {
            (Some(result), OpKind::Binary { op, ty, .. }) if exact_overflow(table, *op, *ty) => {
                Some(result)
            }
            _ => None,
        })
        .collect();
    if pairs.is_empty() {
        return pairs;
    }
    for block in &func.blocks {
        for op in &block.ops {
            let projection = matches!(
                op.kind,
                OpKind::Field {
                    variant: None,
                    field: 0 | 1,
                    ..
                }
            );
            if !projection {
                for used in op.kind.uses() {
                    pairs.remove(&used);
                }
            }
        }
        for used in block.term.uses() {
            pairs.remove(&used);
        }
    }
    pairs
}

impl<'a> JsCx<'a> {
    /// `l op r` at operand type `ty` (`rvalue.rs::codegen_rvalue`, `BinaryOp` arm).
    pub fn binary(
        &self,
        op: BinaryOp,
        ty: TypeId,
        rhs_ty: TypeId,
        result_ty: TypeId,
        lhs: Expr,
        rhs: Expr,
    ) -> Res<Expr> {
        match op {
            BinaryOp::Cmp => self.three_way(ty, result_ty, lhs, rhs),
            BinaryOp::AddWithOverflow | BinaryOp::SubWithOverflow | BinaryOp::MulWithOverflow => {
                self.with_overflow(op, ty, lhs, rhs)
            }
            _ => self.binop(op, ty, rhs_ty, lhs, rhs),
        }
    }

    /// `BinOp::Cmp` (`rvalue.rs::codegen_three_way`).
    fn three_way(
        &self,
        operand_ty: TypeId,
        ordering_ty: TypeId,
        lhs: Expr,
        rhs: Expr,
    ) -> Res<Expr> {
        let table = self.table;
        if table.int_info(operand_ty).is_none()
            && !value::is_bool(table, operand_ty)
            && !value::is_char(table, operand_ty)
        {
            return Err(decline(OpClass::Rvalue, "Cmp on a non-integer operand"));
        }
        let variants = value::enum_variants(table, ordering_ty);
        let variant = |wanted: &str| -> Option<Expr> {
            let index = variants
                .iter()
                .position(|variant| value::variant_name(self.interns, variant) == wanted)?;
            value::enum_value(table, self.interns, ordering_ty, index as u32, Vec::new())
        };
        let (Some(less), Some(equal), Some(greater)) =
            (variant("Less"), variant("Equal"), variant("Greater"))
        else {
            return Err(decline(
                OpClass::Rvalue,
                "Cmp returned a type that is not `Ordering`",
            ));
        };
        Ok(rcgjs_object::evaluation::three_way(
            lhs, rhs, less, equal, greater,
        ))
    }

    /// `AddWithOverflow` and its siblings as a real pair (`rvalue.rs::codegen_with_overflow`): the
    /// array `[$u = mask($t = l op r), $u !== $t]`.
    ///
    /// An rvalue has no statement position, so the exact result is bound by an assignment to a
    /// hoisted temporary inside the first element (`ptr.rs::reread`'s idiom) and read back by the
    /// second; array elements evaluate left to right, and that is all the sequencing it needs —
    /// no closure. A 32 bit product is not exact in a double, so its wrapped half is
    /// `Math.imul` over the operands, and only the flag compares against the rounded product: a
    /// product outside the type's range rounds to a double outside it too, never to a value
    /// inside.
    fn with_overflow(&self, op: BinaryOp, ty: TypeId, lhs: Expr, rhs: Expr) -> Res<Expr> {
        let (Some((signed, bits)), Some(js_op)) = (self.table.int_info(ty), overflow_operator(op))
        else {
            return Err(decline(
                OpClass::Rvalue,
                "overflowing arithmetic on a non-integer",
            ));
        };
        let repr = self.table.int_repr(ty);
        let (wrapped, exact) =
            if op == BinaryOp::MulWithOverflow && repr == IntRepr::Number && bits == 32 {
                let (lhs, lhs_again) = self.read_twice(lhs);
                let (rhs, rhs_again) = self.read_twice(rhs);
                (
                    jsast::imul(lhs, rhs),
                    jsast::binary(JsBinOp::Mul, lhs_again, rhs_again),
                )
            } else {
                let exact = self.temp_name();
                (
                    jsast::assign(exact.clone(), jsast::binary(js_op, lhs, rhs)),
                    exact,
                )
            };
        let bound = self.temp_name();
        Ok(jsast::array(vec![
            jsast::assign(bound.clone(), mask(repr, signed, bits, wrapped)),
            jsast::binary(JsBinOp::StrictNe, bound, exact),
        ]))
    }

    /// The value of an overflow pair [`exact_overflow_pairs`] admitted: the exact `l op r`, with
    /// no wrap and no pair around it.
    pub(super) fn overflow_exact(&self, op: BinaryOp, lhs: Expr, rhs: Expr) -> Res<Expr> {
        let Some(js_op) = overflow_operator(op) else {
            return Err(decline(
                OpClass::Rvalue,
                format!("`{op:?}` carried as an exact overflow result"),
            ));
        };
        Ok(jsast::binary(js_op, lhs, rhs))
    }

    /// Field `field` of an overflow pair [`exact_overflow_pairs`] admitted, read off its exact
    /// result `x`: the wrapped `mask(x)`, or the flag `mask(x) !== x`.
    pub(super) fn overflow_field(&self, pair: ValueId, field: u32) -> Res<Expr> {
        let ty = match self.def_of(pair).map(|def| &def.kind) {
            Some(OpKind::Binary { ty, .. }) => *ty,
            _ => {
                return Err(decline(
                    OpClass::Rvalue,
                    "an exact overflow pair that no `Binary` op defines",
                ))
            }
        };
        let Some((signed, bits)) = self.table.int_info(ty) else {
            return Err(decline(
                OpClass::Rvalue,
                "overflowing arithmetic on a non-integer",
            ));
        };
        let repr = self.table.int_repr(ty);
        let exact = self.expr_of(pair)?;
        Ok(match field {
            0 => mask(repr, signed, bits, exact),
            _ => {
                let (exact, again) = self.read_twice(exact);
                jsast::binary(JsBinOp::StrictNe, mask(repr, signed, bits, exact), again)
            }
        })
    }

    /// `value` read twice, evaluated once: itself when it is a name or a literal, otherwise an
    /// assignment to a hoisted temporary for the first read and the temporary for the second.
    fn read_twice(&self, value: Expr) -> (Expr, Expr) {
        if matches!(
            value,
            Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_) | Expr::BigInt(_)
        ) {
            return (value.clone(), value);
        }
        let name = self.temp_name();
        (jsast::assign(name.clone(), value), name)
    }

    /// A fresh hoisted temporary, for an assignment spelled inside an expression.
    fn temp_name(&self) -> Expr {
        self.temp(jsast::undefined()).1
    }

    /// Every other binary operator (`rvalue.rs::codegen_binop`).
    fn binop(&self, op: BinaryOp, ty: TypeId, rhs_ty: TypeId, lhs: Expr, rhs: Expr) -> Res<Expr> {
        let table = self.table;
        let compare = |js_op: JsBinOp| jsast::binary(js_op, lhs.clone(), rhs.clone());

        if value::is_pointer(table, ty) {
            let pointee = value::pointee_of(table, ty).unwrap_or(ty);
            let ptr_op = |js_op: JsBinOp| self.ptr_cmp(ty, js_op, lhs.clone(), rhs.clone());
            return match op {
                BinaryOp::Offset => Ok(self.ptr_add(pointee, lhs, rhs)),
                BinaryOp::Eq => Ok(self.ptr_eq(ty, lhs, rhs)),
                BinaryOp::Ne => Ok(jsast::not(self.ptr_eq(ty, lhs, rhs))),
                BinaryOp::Lt => Ok(ptr_op(JsBinOp::Lt)),
                BinaryOp::Le => Ok(ptr_op(JsBinOp::Le)),
                BinaryOp::Gt => Ok(ptr_op(JsBinOp::Gt)),
                BinaryOp::Ge => Ok(ptr_op(JsBinOp::Ge)),
                _ => Err(decline(
                    OpClass::Rvalue,
                    format!("`{op:?}` on a pointer operand"),
                )),
            };
        }

        match op {
            BinaryOp::Eq | BinaryOp::Ne if !value::compares_by_value(table, ty) => {
                return Err(decline(
                    OpClass::Rvalue,
                    format!("`{op:?}` on an operand that does not compare by value"),
                ));
            }
            BinaryOp::Eq => return Ok(compare(JsBinOp::StrictEq)),
            BinaryOp::Ne => return Ok(compare(JsBinOp::StrictNe)),
            BinaryOp::Lt => return Ok(compare(JsBinOp::Lt)),
            BinaryOp::Le => return Ok(compare(JsBinOp::Le)),
            BinaryOp::Gt => return Ok(compare(JsBinOp::Gt)),
            BinaryOp::Ge => return Ok(compare(JsBinOp::Ge)),
            _ => {}
        }

        if value::is_bool(table, ty) {
            return match op {
                BinaryOp::BitAnd => Ok(rcgjs_object::evaluation::eager_boolean(
                    JsBinOp::BitAnd,
                    lhs,
                    rhs,
                )),
                BinaryOp::BitOr => Ok(rcgjs_object::evaluation::eager_boolean(
                    JsBinOp::BitOr,
                    lhs,
                    rhs,
                )),
                BinaryOp::BitXor => Ok(jsast::binary(JsBinOp::StrictNe, lhs, rhs)),
                _ => Err(decline(OpClass::Rvalue, format!("`{op:?}` on `bool`"))),
            };
        }

        if let Some(bits) = value::float_bits(table, ty) {
            let round = |value: Expr| {
                if bits == 32 {
                    jsast::fround(value)
                } else {
                    value
                }
            };
            return match op {
                BinaryOp::Add => Ok(round(jsast::binary(JsBinOp::Add, lhs, rhs))),
                BinaryOp::Sub => Ok(round(jsast::binary(JsBinOp::Sub, lhs, rhs))),
                BinaryOp::Mul => Ok(round(jsast::binary(JsBinOp::Mul, lhs, rhs))),
                BinaryOp::Div => Ok(round(jsast::binary(JsBinOp::Div, lhs, rhs))),
                BinaryOp::Rem => Ok(round(jsast::binary(JsBinOp::Rem, lhs, rhs))),
                _ => Err(decline(OpClass::Rvalue, format!("`{op:?}` on a float"))),
            };
        }

        let Some((signed, bits)) = table.int_info(ty) else {
            return Err(decline(
                OpClass::Rvalue,
                format!("`{op:?}` on an operand that is not an integer, float, bool or pointer"),
            ));
        };
        let repr = table.int_repr(ty);
        let mask = |value: Expr| mask(repr, signed, bits, value);
        let shift_amount = |rhs: Expr| {
            let amount = match (repr, table.int_repr(rhs_ty)) {
                (IntRepr::BigInt, IntRepr::Number) => jsast::to_bigint(rhs),
                (IntRepr::Number, IntRepr::BigInt) => {
                    jsast::to_number(jsast::bigint_mask(false, 32, rhs))
                }
                _ => rhs,
            };
            match repr {
                IntRepr::BigInt => jsast::binary(JsBinOp::BitAnd, amount, jsast::bigint(bits - 1)),
                IntRepr::Number if bits == 32 => amount,
                IntRepr::Number => {
                    jsast::binary(JsBinOp::BitAnd, amount, jsast::num((bits - 1) as f64))
                }
            }
        };

        Ok(match op {
            BinaryOp::Add | BinaryOp::AddUnchecked => mask(jsast::binary(JsBinOp::Add, lhs, rhs)),
            BinaryOp::Sub | BinaryOp::SubUnchecked => mask(jsast::binary(JsBinOp::Sub, lhs, rhs)),
            BinaryOp::Mul | BinaryOp::MulUnchecked => {
                let product = if repr == IntRepr::Number && bits == 32 {
                    jsast::imul(lhs, rhs)
                } else {
                    jsast::binary(JsBinOp::Mul, lhs, rhs)
                };
                mask(product)
            }
            BinaryOp::Div => mask(jsast::binary(JsBinOp::Div, lhs, rhs)),
            BinaryOp::Rem => mask(jsast::binary(JsBinOp::Rem, lhs, rhs)),
            BinaryOp::BitAnd => mask(jsast::binary(JsBinOp::BitAnd, lhs, rhs)),
            BinaryOp::BitOr => mask(jsast::binary(JsBinOp::BitOr, lhs, rhs)),
            BinaryOp::BitXor => mask(jsast::binary(JsBinOp::BitXor, lhs, rhs)),
            BinaryOp::Shl | BinaryOp::ShlUnchecked => {
                mask(jsast::binary(JsBinOp::Shl, lhs, shift_amount(rhs)))
            }
            BinaryOp::Shr | BinaryOp::ShrUnchecked => {
                let js_op = if signed || repr == IntRepr::BigInt {
                    JsBinOp::Shr
                } else {
                    JsBinOp::UShr
                };
                mask(jsast::binary(js_op, lhs, shift_amount(rhs)))
            }
            _ => {
                return Err(decline(
                    OpClass::Rvalue,
                    format!("`{op:?}` on an integer operand"),
                ))
            }
        })
    }

    /// `op v` (`rvalue.rs::codegen_unop`).
    pub fn unary(&self, op: UnaryOp, ty: TypeId, value: Expr) -> Res<Expr> {
        let table = self.table;
        Ok(match op {
            UnaryOp::Not => {
                if value::is_bool(table, ty) {
                    jsast::not(value)
                } else {
                    mask_ty(table, ty, jsast::bit_not(value))
                }
            }
            UnaryOp::Neg => {
                if value::float_bits(table, ty).is_some() {
                    jsast::neg(value)
                } else {
                    mask_ty(table, ty, jsast::neg(value))
                }
            }
            UnaryOp::PtrMetadata => self.ptr_metadata(ty, value)?,
        })
    }

    /// `v as to` (`rvalue.rs::codegen_cast`).
    pub fn cast(&self, kind: CastKind, from_ty: TypeId, to_ty: TypeId, value: Expr) -> Res<Expr> {
        let table = self.table;
        match kind {
            CastKind::IntToInt => self.int_to_int(from_ty, to_ty, value),
            CastKind::FloatToInt => self.float_to_int(to_ty, value),
            CastKind::IntToFloat => {
                let value = match table.int_repr(from_ty) {
                    IntRepr::BigInt => jsast::to_number(value),
                    IntRepr::Number => value,
                };
                Ok(float_round(table, to_ty, value))
            }
            CastKind::FloatToFloat => Ok(float_round(table, to_ty, value)),
            CastKind::PtrToPtr | CastKind::FnPtrToPtr => {
                let (Some(from_pointee), Some(to_pointee)) = (
                    value::pointee_of(table, from_ty),
                    value::pointee_of(table, to_ty),
                ) else {
                    return Ok(value);
                };
                let value = if !value::is_raw(table, from_ty) && value::is_raw(table, to_ty) {
                    self.ref_to_raw(from_pointee, value)
                } else {
                    value
                };
                // A `str` arm asks whether the target is mutable (`as_bytes_mut`), which only the
                // pointer types say.
                if let Some(converted) = self.cast_str_ptr(from_ty, to_ty, value.clone()) {
                    return converted;
                }
                self.cast_pointer(from_pointee, to_pointee, value)
            }
            CastKind::PointerExposeProvenance => {
                let address = match value::pointee_of(table, from_ty) {
                    Some(pointee) => self.ptr_addr(pointee, value),
                    None => value,
                };
                Ok(mask_ty(table, to_ty, address))
            }
            CastKind::PointerWithExposedProvenance | CastKind::Subtype => {
                match self.cast_str_ptr(from_ty, to_ty, value.clone()) {
                    Some(converted) => converted,
                    None => Ok(value),
                }
            }
            CastKind::PointerCoercion => self.pointer_coercion(from_ty, to_ty, value),
            CastKind::Transmute => self.transmute(from_ty, to_ty, value),
        }
    }

    /// The pointer coercions, told apart by their two types (`rvalue.rs:601-668`).
    fn pointer_coercion(&self, from_ty: TypeId, to_ty: TypeId, value: Expr) -> Res<Expr> {
        let table = self.table;
        let from_kind = value::kind(table, from_ty);
        let to_kind = value::kind(table, to_ty);
        match (from_kind, to_kind) {
            // `ReifyFnPointer`: the function item itself, by name. The *operand* cannot say which
            // one — a `FnDef` is zero-sized, so its value is `undefined` — and neither can `def`,
            // which may be a trait method the impl resolves. The type table carries the resolved
            // instance for exactly this (v1 asked `Instance::resolve` at the cast).
            (
                Some(TypeKind::FnDef {
                    instance: Some(instance),
                    ..
                }),
                Some(TypeKind::FnPtr { .. }),
            ) => Ok(jsast::id(
                self.interns
                    .str(instance.link_name)
                    .unwrap_or("")
                    .to_owned(),
            )),
            (Some(TypeKind::FnDef { .. }), Some(TypeKind::FnPtr { .. })) => Err(decline(
                OpClass::Cast,
                "a `fn` pointer to a function item that does not resolve to one instance",
            )),
            // `ClosureFnPointer`: the `FnOnce::call_once` shim of the closure, which takes the
            // environment as its first parameter. Only a non-capturing closure coerces, so that
            // environment is zero-sized and `undefined` is the whole of it — but it still occupies
            // a parameter slot, so the coercion is the wrapper that puts it back
            // (v1 `rvalue.rs::ClosureFnPointer`).
            (
                Some(TypeKind::Closure {
                    fn_pointer_shim: Some(shim),
                    ..
                }),
                Some(TypeKind::FnPtr { sig }),
            ) => {
                let target = jsast::id(self.interns.str(shim.link_name).unwrap_or("").to_owned());
                let arity = self
                    .interns
                    .sig(*sig)
                    .map(|descriptor| descriptor.inputs.len())
                    .unwrap_or_default();
                let params: Vec<String> = (0..arity).map(|index| format!("$a{index}")).collect();
                let mut arguments = vec![jsast::undefined()];
                arguments.extend(params.iter().cloned().map(jsast::id));
                Ok(jsast::arrow(params, jsast::call(target, arguments)))
            }
            (Some(TypeKind::Closure { .. }), Some(TypeKind::FnPtr { .. })) => Err(decline(
                OpClass::Cast,
                "a closure coerced to a `fn` pointer whose `call_once` shim the type table does \
                 not record",
            )),
            (
                Some(TypeKind::RawPointer {
                    pointee: from_pointee,
                    ..
                }),
                Some(TypeKind::RawPointer {
                    pointee: to_pointee,
                    ..
                }),
            ) if from_pointee == to_pointee => Ok(value),
            (
                Some(TypeKind::RawPointer {
                    pointee: from_pointee,
                    ..
                }),
                Some(TypeKind::RawPointer {
                    pointee: to_pointee,
                    ..
                }),
            ) if matches!(
                (value::kind(table, *from_pointee), value::kind(table, *to_pointee)),
                (Some(TypeKind::Array { element, .. }), Some(_)) if table.peeled(*element) == table.peeled(*to_pointee)
            ) =>
            {
                // `ArrayToPointer`.
                Ok(ptr::slot(ptr::slot_element(value), jsast::num(0)))
            }
            _ => self.coerce_unsized(value, from_ty, to_ty),
        }
    }

    /// `Unsize` (`unsize.rs::coerce_unsized`), without the rows that need a vtable name.
    fn coerce_unsized(&self, value: Expr, from_ty: TypeId, to_ty: TypeId) -> Res<Expr> {
        let table = self.table;
        let from_kind = value::kind(table, from_ty);
        let to_kind = value::kind(table, to_ty);
        match (from_kind, to_kind) {
            (
                Some(
                    TypeKind::Reference {
                        pointee: source, ..
                    }
                    | TypeKind::RawPointer {
                        pointee: source, ..
                    },
                ),
                Some(
                    TypeKind::Reference {
                        pointee: target, ..
                    }
                    | TypeKind::RawPointer {
                        pointee: target, ..
                    },
                ),
            ) => {
                let raw = value::is_raw(table, from_ty);
                let (source, target) = (*source, *target);
                if table.unsized_tail(target).is_some() {
                    return self.fatten_tail(value, source, target);
                }
                let (source_tail, target_tail) = self.lockstep_tails(source, target)?;
                self.fatten(value, source_tail, target_tail, raw)
            }
            (Some(TypeKind::Adt { .. }), Some(TypeKind::Adt { .. })) => {
                let (Some(from_field), Some(to_field)) = (
                    table.transparent_field(from_ty),
                    table.transparent_field(to_ty),
                ) else {
                    return self.coerce_unsized_struct(value, from_ty, to_ty);
                };
                self.coerce_unsized(value, from_field.ty, to_field.ty)
            }
            _ => Err(decline(
                OpClass::Unsize,
                "an unsizing coercion between types that are neither pointers nor structs",
            )),
        }
    }

    /// The struct row of `coerce_unsized`: `Adt<..>` to the same `Adt<..>` with one field
    /// coerced.
    fn coerce_unsized_struct(&self, value: Expr, from_ty: TypeId, to_ty: TypeId) -> Res<Expr> {
        let table = self.table;
        let (Some(from), Some(to)) = (
            table.get(table.peeled(from_ty)),
            table.get(table.peeled(to_ty)),
        ) else {
            return Err(decline(OpClass::Unsize, "unsize of unknown types"));
        };
        let (
            TypeKind::Adt {
                def: def_a,
                variants: variants_a,
                ..
            },
            TypeKind::Adt {
                def: def_b,
                variants: variants_b,
                ..
            },
        ) = (&from.kind, &to.kind)
        else {
            return Err(decline(OpClass::Unsize, "unsize between non-structs"));
        };
        if def_a != def_b {
            return Err(decline(
                OpClass::Unsize,
                "unsize between two different structs",
            ));
        }
        let (Some(variant_a), Some(variant_b)) = (variants_a.first(), variants_b.first()) else {
            return Err(decline(
                OpClass::Unsize,
                "unsize of a struct without a variant",
            ));
        };
        let (setup, mut operands) =
            OperandEvaluation::Materialize.capture(vec![value], |value| self.temp(value));
        let value = operands.pop().expect("one unsizing operand");
        let mut entries = Vec::with_capacity(variant_a.fields.len());
        let mut coerced = false;
        for (field_a, field_b) in variant_a.fields.iter().zip(&variant_b.fields) {
            let key = value::key_name(self.interns, field_a.key).unwrap_or_default();
            let read = jsast::member(value.clone(), key.clone());
            if field_a.ty == field_b.ty || table.is_zst(field_a.ty) {
                entries.push((key, read));
                continue;
            }
            if coerced {
                return Err(decline(
                    OpClass::Unsize,
                    "a struct with more than one field to unsize",
                ));
            }
            coerced = true;
            entries.push((key, self.coerce_unsized(read, field_a.ty, field_b.ty)?));
        }
        if coerced {
            Ok(Lowered::Value(setup, jsast::object(entries))
                .into_expr()
                .expect("operand captures are assignments"))
        } else {
            Err(decline(
                OpClass::Unsize,
                "cannot unsize a struct: no field of it changes type",
            ))
        }
    }

    /// `struct_lockstep_tails_for_codegen` over the table: walk both types down their last
    /// field until they differ.
    fn lockstep_tails(&self, source: TypeId, target: TypeId) -> Res<(TypeId, TypeId)> {
        let table = self.table;
        let (mut source, mut target) = (source, target);
        for _ in 0..16 {
            let (Some(a), Some(b)) = (
                table.get(table.peeled(source)),
                table.get(table.peeled(target)),
            ) else {
                break;
            };
            match (&a.kind, &b.kind) {
                (
                    TypeKind::Adt {
                        def: def_a,
                        variants: va,
                        form: NominalForm::Struct,
                        ..
                    },
                    TypeKind::Adt {
                        def: def_b,
                        variants: vb,
                        form: NominalForm::Struct,
                        ..
                    },
                ) if def_a == def_b => {
                    let (Some(fa), Some(fb)) = (
                        va.first().and_then(|v| v.fields.last()),
                        vb.first().and_then(|v| v.fields.last()),
                    ) else {
                        break;
                    };
                    source = fa.ty;
                    target = fb.ty;
                }
                (TypeKind::Tuple(ta), TypeKind::Tuple(tb)) if ta.len() == tb.len() => {
                    let (Some(fa), Some(fb)) = (ta.last(), tb.last()) else {
                        break;
                    };
                    source = *fa;
                    target = *fb;
                }
                _ => break,
            }
        }
        Ok((source, target))
    }

    /// `unsize.rs::fatten_tail`.
    fn fatten_tail(&self, value: Expr, source: TypeId, target: TypeId) -> Res<Expr> {
        let table = self.table;
        let (source_tail, target_tail) = self.lockstep_tails(source, target)?;
        let meta = match (
            value::kind(table, source_tail),
            value::kind(table, target_tail),
        ) {
            (Some(TypeKind::Array { length, .. }), Some(TypeKind::Slice(_))) => {
                jsast::num(*length as f64)
            }
            _ => {
                return Err(decline(
                    OpClass::Unsize,
                    "a struct tail unsizing that is not array to slice",
                ))
            }
        };

        Ok(ptr::fat_dyn(value, meta))
    }

    /// `unsize.rs::fatten`, without the `dyn` rows that need a vtable name.
    fn fatten(&self, value: Expr, source: TypeId, target: TypeId, raw: bool) -> Res<Expr> {
        let table = self.table;
        match (value::kind(table, source), value::kind(table, target)) {
            (Some(TypeKind::Array { length, .. }), Some(TypeKind::Slice(_))) => {
                let len = *length;
                if !raw {
                    return Ok(ptr::fat_slice(value, jsast::num(0), jsast::num(len as f64)));
                }
                Ok(jsast::rt_call(
                    "unwindow_slice",
                    vec![value, jsast::num(len as f64)],
                ))
            }
            (
                Some(TypeKind::Dynamic { principal: a, .. }),
                Some(TypeKind::Dynamic { principal: b, .. }),
            ) => {
                if a == b || b.is_none() {
                    return Ok(value);
                }
                // Which supertrait slot to read is `tcx.supertrait_vtable_slot`'s answer, and no
                // part of JIR-S records it.
                Err(decline(
                    OpClass::Unsize,
                    "a `dyn` upcast, which needs the supertrait vtable slot",
                ))
            }
            // `&T -> &dyn Trait`: the concrete value is the pointer, the vtable is the metadata.
            // The data half is the *reference*, even when the pointer being coerced is a raw one:
            // a vtable's entries take their receiver the way every other function does.
            (_, Some(TypeKind::Dynamic { principal, .. })) => {
                let meta = jsast::id(self.vtable_name(source, *principal)?);
                let value = if raw {
                    self.raw_to_ref(source, value)
                } else {
                    value
                };
                Ok(ptr::fat_dyn(value, meta))
            }
            _ => Err(decline(
                OpClass::Unsize,
                "an unsizing coercion with no known row",
            )),
        }
    }

    /// `as` between two integer types (`rvalue.rs::codegen_int_to_int`).
    fn int_to_int(&self, from_ty: TypeId, to_ty: TypeId, value: Expr) -> Res<Expr> {
        let table = self.table;
        if value::is_char(table, to_ty) {
            return Ok(value);
        }
        let Some((signed, bits)) = table.int_info(to_ty) else {
            return Err(decline(OpClass::Cast, "IntToInt into a non-integer"));
        };
        let to_repr = table.int_repr(to_ty);
        if value::is_bool(table, from_ty) {
            return Ok(match to_repr {
                IntRepr::BigInt => jsast::cond(value, jsast::bigint(1), jsast::bigint(0)),
                IntRepr::Number => jsast::cond(value, jsast::num(1), jsast::num(0)),
            });
        }
        Ok(match (table.int_repr(from_ty), to_repr) {
            (IntRepr::Number, IntRepr::Number) => mask_ty(table, to_ty, value),
            (IntRepr::Number, IntRepr::BigInt) => {
                jsast::bigint_mask(signed, bits, jsast::to_bigint(value))
            }
            (IntRepr::BigInt, IntRepr::BigInt) => jsast::bigint_mask(signed, bits, value),
            (IntRepr::BigInt, IntRepr::Number) => {
                jsast::to_number(jsast::bigint_mask(signed, bits, value))
            }
        })
    }

    /// A float to integer cast (`rvalue.rs::codegen_float_to_int`).
    fn float_to_int(&self, to_ty: TypeId, value: Expr) -> Res<Expr> {
        let Some((signed, bits)) = self.table.int_info(to_ty) else {
            return Err(decline(OpClass::Cast, "FloatToInt into a non-integer"));
        };
        Ok(match self.table.int_repr(to_ty) {
            IntRepr::BigInt => jsast::rt_call(
                "f2i_big",
                vec![value, jsast::num(bits), jsast::boolean(signed)],
            ),
            IntRepr::Number => {
                let (min, max) = if signed {
                    (
                        -(2f64.powi(bits as i32 - 1)),
                        2f64.powi(bits as i32 - 1) - 1.0,
                    )
                } else {
                    (0.0, 2f64.powi(bits as i32) - 1.0)
                };
                jsast::rt_call("f2i", vec![value, jsast::num(min), jsast::num(max)])
            }
        })
    }

    /// A fresh aggregate (`rvalue.rs::codegen_aggregate`), given the already lowered fields.
    pub fn aggregate(
        &self,
        ty: TypeId,
        variant: u32,
        active_field: Option<u32>,
        values: Vec<Expr>,
    ) -> Res<Expr> {
        let table = self.table;
        let interns = self.interns;
        if table.is_zst(ty) {
            return Ok(jsast::undefined());
        }
        let Some(descriptor) = table.get(ty) else {
            return Err(decline(OpClass::Rvalue, "aggregate of an unknown type"));
        };
        // A transparent wrapper *is* its one non-1-ZST field.
        if let Some(inner) = descriptor.js.transparent {
            let is_union = matches!(
                descriptor.kind,
                TypeKind::Adt {
                    form: NominalForm::Union,
                    ..
                }
            );
            if is_union {
                if active_field.is_some_and(|active| active != inner.field) {
                    return Ok(value::uninit(table, inner.ty));
                }
                return Ok(values.into_iter().next().unwrap_or_else(jsast::undefined));
            }
            return Ok(values
                .into_iter()
                .nth(inner.field as usize)
                .unwrap_or_else(jsast::undefined));
        }
        match &descriptor.kind {
            TypeKind::Array { .. }
            | TypeKind::Closure { .. }
            | TypeKind::CoroutineClosure { .. } => Ok(jsast::array(values)),
            TypeKind::Coroutine { .. } => {
                let upvars = values
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| (index.to_string(), value))
                    .collect();
                value::enum_value(table, interns, ty, 0, upvars)
                    .ok_or_else(|| decline(OpClass::Rvalue, "coroutine aggregate"))
            }
            TypeKind::Tuple(_) => Ok(if values.is_empty() {
                jsast::undefined()
            } else {
                jsast::array(values)
            }),
            TypeKind::Adt { form, variants, .. } => {
                let index = if *form == NominalForm::Enum {
                    variant
                } else {
                    0
                };
                let Some(descriptor) = variants.get(index as usize) else {
                    return Err(decline(OpClass::Rvalue, "aggregate of a missing variant"));
                };
                let mut entries = Vec::new();
                match active_field {
                    Some(field) => {
                        if let (Some(value), Some(field)) = (
                            values.into_iter().next(),
                            descriptor.fields.get(field as usize),
                        ) {
                            entries.push((self.aggregate_key(field.key)?, value));
                        }
                    }
                    None => {
                        for (field, value) in descriptor.fields.iter().zip(values) {
                            entries.push((self.aggregate_key(field.key)?, value));
                        }
                    }
                }
                if *form == NominalForm::Enum {
                    return value::enum_value(table, interns, ty, variant, entries)
                        .ok_or_else(|| decline(OpClass::Rvalue, "enum aggregate"));
                }
                Ok(jsast::object(entries))
            }
            TypeKind::RawPointer { pointee, .. } => {
                let mut values = values.into_iter();
                let (Some(data), Some(meta)) = (values.next(), values.next()) else {
                    return Err(decline(
                        OpClass::Rvalue,
                        "a raw pointer aggregate needs a data pointer and its metadata",
                    ));
                };
                self.build_fat(*pointee, data, meta)
            }
            other => Err(decline(
                OpClass::Rvalue,
                format!("aggregate of a `{other:?}` type"),
            )),
        }
    }

    fn aggregate_key(&self, key: FieldKey) -> Res<String> {
        match key {
            FieldKey::Name(id) => Ok(self.interns.str(id).unwrap_or("").to_owned()),
            FieldKey::Index(index) => Ok(index.to_string()),
            FieldKey::Transparent => Err(decline(
                OpClass::Rvalue,
                "a struct field with no key (zero-sized aggregate reached the object row)",
            )),
        }
    }

    /// `[v; N]` (`rvalue.rs`, `Repeat` arm). `element` is the element expression read without a
    /// clone; `is_constant` says whether it builds a fresh value at every evaluation.
    pub fn repeat(&self, element_ty: TypeId, element: Expr, count: u64, is_constant: bool) -> Expr {
        if !self.table.needs_clone(element_ty) {
            return jsast::method_call(
                jsast::new(jsast::id("Array"), vec![jsast::num(count as f64)]),
                "fill",
                vec![element],
            );
        }
        let (setup, mut elements) = OperandEvaluation::repeat(true, is_constant)
            .capture(vec![element], |value| self.temp(value));
        let element = elements.pop().expect("one repeat operand");
        let element = if is_constant {
            element
        } else {
            value::clone_expr(self.table, self.interns, element_ty, element)
        };
        let repeated = jsast::call(
            jsast::member(jsast::id("Array"), "from"),
            vec![
                jsast::object_of(vec![("length", jsast::num(count as f64))]),
                jsast::arrow(vec![], element),
            ],
        );
        Lowered::Value(setup, repeated)
            .into_expr()
            .expect("operand captures are assignments")
    }

    /// `Discriminant(base)` (`rvalue.rs::codegen_discriminant`): the tag when `as_tag`, the
    /// number otherwise, `discr_ty` being the result's integer type.
    pub fn discriminant(
        &self,
        ty: TypeId,
        value: Expr,
        discr_ty: TypeId,
        as_tag: bool,
    ) -> Res<Expr> {
        let (setup, mut operands) =
            OperandEvaluation::Materialize.capture(vec![value], |value| self.temp(value));
        let value = self.discriminant_value(
            ty,
            operands.pop().expect("one discriminant operand"),
            discr_ty,
            as_tag,
        )?;
        Ok(Lowered::Value(setup, value)
            .into_expr()
            .expect("operand captures are assignments"))
    }

    fn discriminant_value(
        &self,
        ty: TypeId,
        value: Expr,
        discr_ty: TypeId,
        as_tag: bool,
    ) -> Res<Expr> {
        let table = self.table;
        let peeled = table.peeled(ty);
        let Some(repr) = table.enum_repr(peeled) else {
            // Everything else has exactly one variant, whose discriminant is zero.
            return Ok(jsast::num(0));
        };
        let discr_repr = table.int_repr(discr_ty);
        let variants = table.variants(peeled);
        // A layout with one possible variant (`Variants::Single`) has no tag in the value; the
        // discriminant is that variant's, known at compile time. `ControlFlow<Infallible, T>` is
        // the case that matters: one variant is uninhabited, so the enum has two variants and one
        // layout, and every `?` reaches it. The table says so by every variant lacking a physical
        // tag under no niche (a `Number` enum keeps its discriminant as the value regardless).
        let inhabited: Vec<usize> = variants
            .iter()
            .enumerate()
            .filter(|(_, variant)| {
                !variant
                    .fields
                    .iter()
                    .any(|field| self.is_uninhabited(field.ty))
            })
            .map(|(index, _)| index)
            .collect();
        let untagged = match table.get(peeled).map(|d| &d.kind) {
            Some(TypeKind::Adt { niche, .. }) => {
                niche.is_none()
                    && variants.iter().all(|v| v.physical_tag.is_none())
                    && !matches!(repr, EnumRepr::Number { .. })
            }
            _ => false,
        };
        if inhabited.len() <= 1 || untagged {
            let index = inhabited.first().copied().unwrap_or(0);
            return Ok(value::discriminant_literal(
                table,
                peeled,
                index as u32,
                discr_repr,
            ));
        }
        if variants.is_empty() {
            return Ok(jsast::num(0));
        }
        if as_tag {
            return Ok(value::tag_expr(repr, value));
        }
        value::discriminant_number(table, self.interns, peeled, value, discr_repr)
            .ok_or_else(|| decline(OpClass::Rvalue, "discriminant of a non-enum"))
    }

    /// Whether a type is uninhabited (`!` or an enum with no variants), shallowly.
    fn is_uninhabited(&self, ty: TypeId) -> bool {
        match value::kind(self.table, ty) {
            Some(TypeKind::Never) => true,
            Some(TypeKind::Adt {
                form: NominalForm::Enum,
                variants,
                ..
            }) => variants.is_empty(),
            _ => false,
        }
    }

    /// `SetDiscriminant` (`base.rs::codegen_set_discriminant`): the statements that change the
    /// variant of `place` (of type `ty`, read as `target`).
    pub fn set_discriminant(
        &self,
        place: super::place::JsPlace,
        ty: TypeId,
        variant: u32,
    ) -> Res<Vec<jsast::Stmt>> {
        let table = self.table;
        let (Some(repr), Some(tag)) = (
            table.enum_repr(table.peeled(ty)),
            value::variant_tag(table, self.interns, ty, variant),
        ) else {
            return Err(decline(
                OpClass::Statement,
                "SetDiscriminant of a type that is not an enum",
            ));
        };
        match repr {
            EnumRepr::Tag { fieldless: false } | EnumRepr::State => Ok(vec![jsast::assign_stmt(
                jsast::member(self.read_place(&place), TAG),
                tag,
            )]),
            EnumRepr::Number { .. } | EnumRepr::Tag { fieldless: true } => {
                self.write_place(place, ty, tag)
            }
        }
    }

    /// The scrutinee value of `ValueId` `v` for `SwitchInt` case spelling.
    #[allow(dead_code)]
    pub fn switch_case_value(&self, ty: TypeId, raw: u128) -> Expr {
        value::int_literal(self.table, ty, raw).unwrap_or_else(|| jsast::num(raw as f64))
    }
}

/// Rounds to `f32` precision if `ty` is `f32`.
fn float_round(table: &rcgjs_object::jir::types::TypeTable, ty: TypeId, value: Expr) -> Expr {
    match value::float_bits(table, ty) {
        Some(32) => jsast::fround(value),
        _ => value,
    }
}

/// Unused marker to keep `ValueId` imported for the doc references above.
#[allow(dead_code)]
fn _value_id(_: ValueId) {}
