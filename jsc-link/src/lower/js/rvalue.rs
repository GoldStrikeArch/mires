//! Operators, casts, aggregates and discriminants (v1 `backend/src/rvalue.rs` and the coercion
//! rows of `unsize.rs` that need no vtable name).

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::evaluation::OperandEvaluation;
use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::repr::{Func, OpKind, ValueId};
use rcgjs_object::jir::semantic::{BinaryOp, CastKind, UnaryOp};
use rcgjs_object::jir::types::{
    EnumRepr, FieldKey, IntRepr, NominalForm, TypeId, TypeKind, TypeTable,
};
use rcgjs_object::jsast::{self, BinOp as JsBinOp, Expr, Stmt};
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
/// in `BigInt`. A 32 bit product cannot — the double rounds away the low bits a wrap keeps — so
/// it is carried only when one half is read ([`PairCarrier::Wrapped`], [`PairCarrier::Overflowed`])
/// and keeps the array form of [`JsCx::binary`] otherwise.
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

/// Whether `op` at `ty` is the one overflow operator [`exact_overflow`] refuses: a product of 32
/// bit numbers.
fn product_32(table: &TypeTable, op: BinaryOp, ty: TypeId) -> bool {
    op == BinaryOp::MulWithOverflow
        && table.int_repr(ty) == IntRepr::Number
        && table.int_info(ty).is_some_and(|(_, bits)| bits == 32)
}

/// How an overflow pair that is never built is spelled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PairCarrier {
    /// The exact `l op r` ([`exact_overflow`]): `.0` reads `mask(x)` and `.1` `mask(x) !== x`.
    Exact,
    /// A 32 bit product only `.0` of which is read: the wrapped `mask(Math.imul(l, r))` itself.
    Wrapped,
    /// A 32 bit product only `.1` of which is read: the flag `mask(Math.imul(l, r)) !== l * r`
    /// itself.
    Overflowed,
}

/// A value of a pair family that is never built: the result of the overflow op `root`, or a
/// `Move`/`Copy` of a member, all spelled by the same `carrier`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CarriedPair {
    pub root: ValueId,
    pub carrier: PairCarrier,
}

/// The overflow pairs of `func` that are never built, by every value of their family.
///
/// A family is a result of `AddWithOverflow` or a sibling, plus every `Move`/`Copy` of a member
/// (a copy of a value nobody mutates is the same value). It is carried when every use of every
/// member is a `Field` projection of `.0` or `.1` or such a `Move`/`Copy` — the
/// `let (r, o) = a.overflowing_add(b)` and `checked_*` shape. Any other use — a block argument,
/// a return, a call argument, an aggregate field, a store, a borrow — needs the real pair and
/// keeps the array. A 32 bit product is carried only when one of its halves is never read.
/// Unsigned sums usually never get here: `opt::dce` already splits a projected one into a
/// wrapped add and a comparison.
pub(super) fn carried_overflow_pairs(
    func: &Func,
    table: &TypeTable,
) -> BTreeMap<ValueId, CarriedPair> {
    let ops = || func.blocks.iter().flat_map(|block| &block.ops);
    let roots: BTreeMap<ValueId, (BinaryOp, TypeId)> = ops()
        .filter_map(|op| match (op.result, &op.kind) {
            (Some(result), OpKind::Binary { op, ty, .. })
                if exact_overflow(table, *op, *ty) || product_32(table, *op, *ty) =>
            {
                Some((result, (*op, *ty)))
            }
            _ => None,
        })
        .collect();
    if roots.is_empty() {
        return BTreeMap::new();
    }
    let mut root_of: BTreeMap<ValueId, ValueId> = roots.keys().map(|&root| (root, root)).collect();
    // Blocks are not in dominance order, so a copy of a copy may be seen before its source is.
    loop {
        let mut grew = false;
        for op in ops() {
            if let (Some(result), OpKind::Move(source) | OpKind::Copy(source)) =
                (op.result, &op.kind)
            {
                if let Some(&root) = root_of.get(source) {
                    grew |= root_of.insert(result, root).is_none();
                }
            }
        }
        if !grew {
            break;
        }
    }
    let mut read: BTreeMap<ValueId, [bool; 2]> = BTreeMap::new();
    let mut escaped: BTreeSet<ValueId> = BTreeSet::new();
    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::Field {
                    base,
                    variant: None,
                    field: field @ (0 | 1),
                } if root_of.contains_key(base) => {
                    read.entry(root_of[base]).or_default()[*field as usize] = true;
                }
                OpKind::Move(source) | OpKind::Copy(source) if root_of.contains_key(source) => {}
                kind => escaped.extend(kind.uses().iter().filter_map(|v| root_of.get(v))),
            }
        }
        escaped.extend(block.term.uses().iter().filter_map(|v| root_of.get(v)));
    }
    let carriers: BTreeMap<ValueId, PairCarrier> = roots
        .into_iter()
        .filter(|(root, _)| !escaped.contains(root))
        .filter_map(|(root, (op, ty))| {
            let carrier = if exact_overflow(table, op, ty) {
                PairCarrier::Exact
            } else {
                match read.get(&root).copied().unwrap_or_default() {
                    [true, true] => return None,
                    [false, true] => PairCarrier::Overflowed,
                    _ => PairCarrier::Wrapped,
                }
            };
            Some((root, carrier))
        })
        .collect();
    root_of
        .into_iter()
        .filter_map(|(value, root)| {
            let carrier = *carriers.get(&root)?;
            Some((value, CarriedPair { root, carrier }))
        })
        .collect()
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

    fn ordering_variants(&self, ordering_ty: TypeId) -> Res<(Expr, Expr, Expr)> {
        let table = self.table;
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
        Ok((less, equal, greater))
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
        let (less, equal, greater) = self.ordering_variants(ordering_ty)?;
        Ok(rcgjs_object::evaluation::three_way(
            lhs, rhs, less, equal, greater,
        ))
    }

    fn i64_shift_amount(&self, rhs: ValueId) -> Res<Expr> {
        let rhs_ty = self.ty_of(rhs)?;
        if self.is_i64_ty(rhs_ty) {
            return Ok(self.pair_of(rhs)?.0);
        }
        let expr = self.expr_of(rhs)?;
        if self.table.int_repr(rhs_ty) == IntRepr::BigInt {
            Ok(jsast::to_number(jsast::bigint_mask(false, 32, expr)))
        } else {
            Ok(expr)
        }
    }

    /// Lowers a 64-bit integer `Binary` op whose result is itself an `i64`/`u64` pair.
    pub(super) fn i64_binary_pair_stmts(
        &self,
        stmts: &mut Vec<Stmt>,
        op: BinaryOp,
        ty: TypeId,
        lhs: ValueId,
        rhs: ValueId,
    ) -> Res<(Expr, Expr)> {
        let Some(signed) = super::i64_pair::i64_signedness(self.table, ty) else {
            return Err(decline(
                OpClass::Rvalue,
                format!("64-bit pair `{op:?}` on a non-i64 operand"),
            ));
        };
        let a = self.pair_of(lhs)?;
        Ok(match op {
            BinaryOp::Add | BinaryOp::AddUnchecked => {
                let b = self.pair_of(rhs)?;
                super::i64_pair::add_pair(self, stmts, a, b)
            }
            BinaryOp::Sub | BinaryOp::SubUnchecked => {
                let b = self.pair_of(rhs)?;
                super::i64_pair::sub_pair(self, stmts, a, b)
            }
            BinaryOp::Mul | BinaryOp::MulUnchecked => {
                let b = self.pair_of(rhs)?;
                super::i64_pair::mul_pair(self, stmts, a, b)
            }
            BinaryOp::Div => {
                let b = self.pair_of(rhs)?;
                super::i64_pair::div_rem_pair(self, stmts, signed, false, a, b)
            }
            BinaryOp::Rem => {
                let b = self.pair_of(rhs)?;
                super::i64_pair::div_rem_pair(self, stmts, signed, true, a, b)
            }
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                let b = self.pair_of(rhs)?;
                let js_op = match op {
                    BinaryOp::BitAnd => JsBinOp::BitAnd,
                    BinaryOp::BitOr => JsBinOp::BitOr,
                    _ => JsBinOp::BitXor,
                };
                super::i64_pair::bitwise_pair(js_op, a, b)
            }
            BinaryOp::Shl | BinaryOp::ShlUnchecked => {
                let amt = self.i64_shift_amount(rhs)?;
                super::i64_pair::shl_pair(self, stmts, a, amt)
            }
            BinaryOp::Shr | BinaryOp::ShrUnchecked => {
                let amt = self.i64_shift_amount(rhs)?;
                super::i64_pair::shr_pair(self, stmts, signed, a, amt)
            }
            _ => {
                return Err(decline(
                    OpClass::Rvalue,
                    format!("`{op:?}` on an `i64`/`u64` operand"),
                ))
            }
        })
    }

    /// Lowers a `Binary` op with 64-bit integer operands whose result is NOT a 64-bit pair
    /// (comparisons, three-way `Cmp`, or an escaped `*WithOverflow` tuple).
    pub(super) fn i64_binary_expr_stmts(
        &self,
        stmts: &mut Vec<Stmt>,
        op: BinaryOp,
        ty: TypeId,
        result_ty: TypeId,
        lhs: ValueId,
        rhs: ValueId,
    ) -> Res<Expr> {
        let Some(signed) = super::i64_pair::i64_signedness(self.table, ty) else {
            return Err(decline(
                OpClass::Rvalue,
                format!("64-bit `{op:?}` on a non-i64 operand"),
            ));
        };
        let a = self.pair_of(lhs)?;
        let b = self.pair_of(rhs)?;
        match op {
            BinaryOp::Cmp => {
                let (less, equal, greater) = self.ordering_variants(result_ty)?;
                Ok(super::i64_pair::three_way_ordering(
                    self,
                    stmts,
                    signed,
                    a,
                    b,
                    [less, equal, greater],
                ))
            }
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => Ok(super::i64_pair::cmp_pair(self, stmts, op, signed, a, b)),
            BinaryOp::AddWithOverflow | BinaryOp::SubWithOverflow | BinaryOp::MulWithOverflow => {
                let (lo, hi, ovf) =
                    super::i64_pair::with_overflow_pair(self, stmts, op, signed, a, b);
                let lo = super::i64_pair::materialize(self, stmts, lo);
                let hi = super::i64_pair::materialize(self, stmts, hi);
                Ok(jsast::array(vec![
                    super::i64_pair::pack_bigint(signed, lo, hi),
                    ovf,
                ]))
            }
            _ => Err(decline(
                OpClass::Rvalue,
                format!("`{op:?}` on an `i64`/`u64` operand"),
            )),
        }
    }

    /// Lowers a 64-bit integer `Unary` op (`Not` or `Neg`).
    pub(super) fn i64_unary_pair_stmts(
        &self,
        stmts: &mut Vec<Stmt>,
        op: UnaryOp,
        operand: ValueId,
    ) -> Res<(Expr, Expr)> {
        let a = self.pair_of(operand)?;
        match op {
            UnaryOp::Not => Ok(super::i64_pair::bit_not_pair(a)),
            UnaryOp::Neg => Ok(super::i64_pair::neg_pair(self, stmts, a)),
            UnaryOp::PtrMetadata => Err(decline(
                OpClass::Rvalue,
                "`PtrMetadata` producing a 64-bit integer",
            )),
        }
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
        let (wrapped, exact) = if product_32(self.table, op, ty) {
            self.product_32_halves(lhs, rhs)
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

    /// A 32 bit product over operands evaluated once each: `Math.imul(l, r)`, the product modulo
    /// 2^32 that the wrapped half masks, and the rounded `l * r` only the flag compares against.
    fn product_32_halves(&self, lhs: Expr, rhs: Expr) -> (Expr, Expr) {
        let (lhs, lhs_again) = self.read_twice(lhs);
        let (rhs, rhs_again) = self.read_twice(rhs);
        (
            jsast::imul(lhs, rhs),
            jsast::binary(JsBinOp::Mul, lhs_again, rhs_again),
        )
    }

    /// The value of the overflow op at the root of a carried pair family
    /// ([`carried_overflow_pairs`]): the exact `l op r`, with no wrap and no pair around it, or
    /// the one half of a 32 bit product that is read.
    pub(super) fn carried_pair_value(
        &self,
        carrier: PairCarrier,
        op: BinaryOp,
        ty: TypeId,
        lhs: Expr,
        rhs: Expr,
    ) -> Res<Expr> {
        let (Some((signed, bits)), Some(js_op)) = (self.table.int_info(ty), overflow_operator(op))
        else {
            return Err(decline(
                OpClass::Rvalue,
                "overflowing arithmetic on a non-integer",
            ));
        };
        let repr = self.table.int_repr(ty);
        Ok(match carrier {
            PairCarrier::Exact => jsast::binary(js_op, lhs, rhs),
            PairCarrier::Wrapped => mask(repr, signed, bits, jsast::imul(lhs, rhs)),
            PairCarrier::Overflowed => {
                let (wrapped, rounded) = self.product_32_halves(lhs, rhs);
                jsast::binary(
                    JsBinOp::StrictNe,
                    mask(repr, signed, bits, wrapped),
                    rounded,
                )
            }
        })
    }

    /// Field `field` of the carried pair `base` ([`carried_overflow_pairs`]): read off the exact
    /// result `x` — the wrapped `mask(x)` or the flag `mask(x) !== x` — or, for a 32 bit product,
    /// the half the value already is.
    pub(super) fn carried_pair_field(
        &self,
        pair: CarriedPair,
        base: ValueId,
        field: u32,
    ) -> Res<Expr> {
        let value = self.expr_of(base)?;
        match (pair.carrier, field) {
            (PairCarrier::Wrapped, 0) | (PairCarrier::Overflowed, 1) => return Ok(value),
            (PairCarrier::Exact, 0 | 1) => {}
            _ => {
                return Err(decline(
                    OpClass::Rvalue,
                    "a carried overflow pair read through a half it does not carry",
                ))
            }
        }
        let ty = match self.def_of(pair.root).map(|def| &def.kind) {
            Some(OpKind::Binary { ty, .. }) => *ty,
            _ => {
                return Err(decline(
                    OpClass::Rvalue,
                    "a carried overflow pair that no `Binary` op defines",
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
        Ok(match field {
            0 => mask(repr, signed, bits, value),
            _ => {
                let (exact, again) = self.read_twice(value);
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
            BinaryOp::Eq => {
                if let Some(check) = super::ptr::simplify_addr_zero_eq(&lhs, &rhs) {
                    return Ok(check);
                }
                return Ok(compare(JsBinOp::StrictEq));
            }
            BinaryOp::Ne => {
                if let Some(check) = super::ptr::simplify_addr_zero_eq(&lhs, &rhs) {
                    return Ok(jsast::not(check));
                }
                return Ok(compare(JsBinOp::StrictNe));
            }
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
                (IntRepr::BigInt, IntRepr::Number) => self.to_bigint(rhs),
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
                if table.int_repr(from_ty) == IntRepr::BigInt
                    && value::float_bits(table, to_ty) == Some(32)
                {
                    return Ok(jsast::rt_call("i2f32_big", vec![value]));
                }
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
                    None if matches!(value::kind(table, from_ty), Some(TypeKind::FnPtr { .. })) => {
                        jsast::rt_call("addr", vec![value, jsast::num(1.0)])
                    }
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

    /// Lowers a `Cast` whose destination is a 64-bit integer `(lo, hi)` pair.
    pub(super) fn i64_cast_pair_stmts(
        &self,
        stmts: &mut Vec<Stmt>,
        kind: CastKind,
        from_ty: TypeId,
        to_ty: TypeId,
        operand: ValueId,
    ) -> Res<(Expr, Expr)> {
        let table = self.table;
        let to_signed = super::i64_pair::i64_signedness(table, to_ty).unwrap_or(false);
        match kind {
            CastKind::IntToInt => {
                if self.is_i64_ty(from_ty) {
                    return self.pair_of(operand);
                }
                let src = self.expr_of(operand)?;
                let is_bool = value::is_bool(table, from_ty);
                let (from_signed, from_bits) = table.int_info(from_ty).unwrap_or((false, 32));
                Ok(super::i64_pair::cast_to_i64_pair(
                    self,
                    stmts,
                    from_signed,
                    from_bits,
                    is_bool,
                    from_bits == 128,
                    src,
                ))
            }
            CastKind::FloatToInt => {
                let src = self.expr_of(operand)?;
                Ok(super::i64_pair::float_to_i64_pair(
                    self, stmts, to_signed, src,
                ))
            }
            CastKind::Transmute => self.transmute_to_i64_pair(stmts, from_ty, to_ty, operand),
            CastKind::PointerExposeProvenance => {
                let expr = self.cast(kind, from_ty, to_ty, self.expr_of(operand)?)?;
                let expr = super::i64_pair::materialize(self, stmts, expr);
                Ok(super::i64_pair::unpack_bigint_exprs(expr))
            }
            CastKind::Subtype => self.pair_of(operand),
            _ => Err(decline(
                OpClass::Cast,
                format!("unsupported cast `{kind:?}` into `i64`/`u64`"),
            )),
        }
    }

    /// Lowers a `Cast` whose source is a 64-bit integer `(lo, hi)` pair and whose destination is
    /// NOT a 64-bit integer.
    pub(super) fn i64_cast_expr_stmts(
        &self,
        stmts: &mut Vec<Stmt>,
        kind: CastKind,
        from_ty: TypeId,
        to_ty: TypeId,
        operand: ValueId,
    ) -> Res<Expr> {
        let table = self.table;
        let from_signed = super::i64_pair::i64_signedness(table, from_ty).unwrap_or(false);
        match kind {
            CastKind::IntToInt => {
                let (lo, hi) = self.pair_of(operand)?;
                if value::is_char(table, to_ty) {
                    return Ok(lo);
                }
                let Some((signed, bits)) = table.int_info(to_ty) else {
                    return Err(decline(OpClass::Cast, "IntToInt into a non-integer"));
                };
                if table.int_repr(to_ty) == IntRepr::Number {
                    Ok(mask(IntRepr::Number, signed, bits, lo))
                } else {
                    Ok(jsast::bigint_mask(
                        signed,
                        bits,
                        super::i64_pair::pack_bigint(from_signed, lo, hi),
                    ))
                }
            }
            CastKind::IntToFloat => {
                let (lo, hi) = self.pair_of(operand)?;
                let float_bits = value::float_bits(table, to_ty).unwrap_or(64);
                Ok(super::i64_pair::i64_pair_to_float(
                    from_signed,
                    float_bits,
                    (lo, hi),
                ))
            }
            CastKind::Transmute => self.transmute_from_i64_pair(stmts, from_ty, to_ty, operand),
            CastKind::PointerWithExposedProvenance | CastKind::Subtype => {
                self.cast(kind, from_ty, to_ty, self.expr_of(operand)?)
            }
            _ => Err(decline(
                OpClass::Cast,
                format!("unsupported cast `{kind:?}` from `i64`/`u64`"),
            )),
        }
    }

    /// A fresh aggregate (`rvalue.rs::codegen_aggregate`), given the already lowered fields.
    pub fn aggregate(
        &self,
        ty: TypeId,
        variant: u32,
        active_field: Option<u32>,
        values: Vec<Expr>,
        data_ty: Option<TypeId>,
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
                self.build_fat(*pointee, data_ty, data, meta)
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
