//! Rvalues, aggregates, operators and casts.
//!
//! # Integer arithmetic
//!
//! Every integer type has one of two JavaScript representations ([`IntRepr`]): a number for
//! anything up to 32 bits — which after the move to `wasm32-unknown-unknown` includes
//! `isize`/`usize` — and a `BigInt` for `i64`/`u64`/`i128`/`u128`. The two never mix implicitly:
//! JavaScript throws on `1n + 1`, so a value crossing the boundary is converted explicitly, and
//! that is exactly what the cast matrix in [`FnCx::codegen_int_to_int`] and the shift-amount
//! coercion in [`FnCx::codegen_binop`] are for. Both representations then wrap the same way: an
//! operation is performed at full width and truncated afterwards, with `| 0` / `>>> 0` /
//! `<< n >> n` for numbers and `BigInt.asIntN`/`asUintN` for `BigInt`s.

use rustc_abi::VariantIdx;
use rustc_middle::mir::{AggregateKind, CastKind, Local, Operand, Rvalue};
use rustc_middle::mir::{BinOp as MirBinOp, UnOp as MirUnOp};
use rustc_middle::ty::{self, CoroutineArgsExt, Instance, Ty, TyCtxt};

use crate::base::FnCx;
use crate::jsast::{self, BinOp as JsBinOp, Expr};
use crate::value::{self, IntRepr, int_info, int_repr, mask, mask_ty, typing_env};

/// The parameter the checked-arithmetic arrow binds its exact result to. `$`-leading, so it can
/// never collide with a Rust identifier or with a local (`naming.rs` reserves the namespace).
const OVERFLOW_PARAM: &str = "$w";

impl<'tcx> FnCx<'_, 'tcx> {
    /// Lowers one rvalue.
    ///
    /// `dest` is the local the value is being assigned to, where it is a bare local, and `None`
    /// otherwise. Only [`FnCx::codegen_discriminant`] reads it: whether a tag may stand for a
    /// discriminant is a question about what the *destination* is later used for (`tag.rs`).
    pub(crate) fn codegen_rvalue(&self, rvalue: &Rvalue<'tcx>, dest: Option<Local>) -> Expr {
        match rvalue {
            Rvalue::Use(operand, _) => self.codegen_operand(operand),
            Rvalue::CopyForDeref(place) => {
                let (source, _) = self.codegen_place(*place);
                self.read(&source)
            }
            // The two part ways for an aggregate pointee: a reference to one is the object
            // itself, a raw pointer to one is a slot. See `ptr.rs`.
            Rvalue::Ref(_, _, place) => self.codegen_ref(*place),
            Rvalue::RawPtr(_, place) => self.codegen_raw_ptr(*place),
            Rvalue::Reborrow(_, _, place) => {
                // The place already holds a reference; a reborrow copies it.
                let (source, _) = self.codegen_place(*place);
                self.read(&source)
            }
            Rvalue::BinaryOp(op, operands) => {
                // The two operands do not always have the same type: a shift takes its amount at
                // whatever width the source wrote it, so `i64 << u32` is one `BinaryOp` with a
                // `BigInt` on the left and a number on the right.
                let lhs_ty = self.operand_ty(&operands.0);
                let rhs_ty = self.operand_ty(&operands.1);
                let lhs = self.codegen_operand(&operands.0);
                let rhs = self.codegen_operand(&operands.1);
                match op {
                    // The three operators whose result type is not the operands': a `Cmp` produces
                    // an `Ordering` and a `*WithOverflow` a `(T, bool)`.
                    MirBinOp::Cmp => {
                        let ordering_ty =
                            self.monomorphize(rvalue.ty(&self.mir.local_decls, self.tcx));
                        self.codegen_three_way(lhs_ty, ordering_ty, lhs, rhs)
                    }
                    MirBinOp::AddWithOverflow
                    | MirBinOp::SubWithOverflow
                    | MirBinOp::MulWithOverflow => self.codegen_with_overflow(*op, lhs_ty, lhs, rhs),
                    _ => self.codegen_binop(*op, lhs_ty, rhs_ty, lhs, rhs),
                }
            }
            Rvalue::UnaryOp(op, operand) => {
                let ty = self.operand_ty(operand);
                let value = self.codegen_operand(operand);
                self.codegen_unop(*op, ty, value)
            }
            Rvalue::Cast(kind, operand, to_ty) => {
                self.codegen_cast(*kind, operand, self.monomorphize(*to_ty))
            }
            Rvalue::Discriminant(place) => {
                let (source, ty) = self.codegen_place(*place);
                match ty.kind() {
                    // A coroutine is an enum over its states (`value::EnumRepr::State`), and its
                    // discriminant is the state the resume switches on. Answering zero here, which
                    // is what the arm below used to do for it, made every resume re-enter the
                    // unresumed state.
                    ty::Adt(_, _) | ty::Coroutine(..) if value::enum_repr(self.tcx, ty).is_some() => {
                        self.codegen_discriminant(ty, source, dest)
                    }
                    // Everything else has exactly one variant, whose discriminant is zero.
                    _ => jsast::num(0),
                }
            }
            Rvalue::Aggregate(kind, operands) => {
                let ty = self.monomorphize(rvalue.ty(&self.mir.local_decls, self.tcx));
                let values = operands.iter().map(|op| self.codegen_operand(op)).collect::<Vec<_>>();
                self.codegen_aggregate(&**kind, ty, values)
            }
            Rvalue::Repeat(operand, count) => {
                let element_ty = self.operand_ty(operand);
                let count =
                    self.monomorphize(*count).try_to_target_usize(self.tcx).unwrap_or_default();

                // `fill` puts the *same* JS object in every slot, so a write through one element
                // would be visible through all the others. An element that is an object therefore
                // gets built once per slot by `Array.from`, whose callback runs `count` times.
                if !value::needs_clone(self.tcx, element_ty) {
                    let value = self.codegen_operand(operand);
                    return jsast::method_call(
                        jsast::new(jsast::id("Array"), vec![jsast::num(count as f64)]),
                        "fill",
                        vec![value],
                    );
                }

                // A constant builds a fresh value every time it is evaluated; a place has to be
                // read without cloning first, so that the clone happens inside the callback.
                let element = match operand {
                    Operand::Constant(_) | Operand::RuntimeChecks(_) => {
                        self.codegen_operand(operand)
                    }
                    _ => value::clone_expr(
                        self.tcx,
                        element_ty,
                        self.codegen_operand_aliased(operand),
                    ),
                };
                jsast::call(
                    jsast::member(jsast::id("Array"), "from"),
                    vec![
                        jsast::object_of(vec![("length", jsast::num(count as f64))]),
                        jsast::arrow(vec![], element),
                    ],
                )
            }
            Rvalue::WrapUnsafeBinder(operand, _) => self.codegen_operand(operand),
            Rvalue::ThreadLocalRef(_) => {
                self.zombie("thread locals are not supported by rustc_codegen_mires".to_string())
            }
        }
    }

    /// `Rvalue::Discriminant` of an enum place.
    ///
    /// Two answers, and which one a caller gets is what `tag.rs` decides. A destination whose every
    /// use is the `SwitchInt` that follows gets the **tag** -- the value itself for a fieldless
    /// enum, its `TAG` otherwise -- because a match over names is a comparison of names. Everything
    /// else gets the discriminant **number**, which is what a program that asked for the integer
    /// meant; for a `#[repr(int)]` enum the two are the same expression.
    ///
    /// The layout is consulted first, because a type with one possible variant has no tag to read.
    /// `Option<Infallible>` is the case that matters: it has one inhabited variant, so it is zero
    /// sized and its JavaScript value is `undefined`. That type is not exotic, it is what `?` on an
    /// `Option` produces as its residual, so every `?` in the program depends on this. When there is
    /// only one variant its discriminant is known at compile time anyway.
    fn codegen_discriminant(
        &self,
        ty: Ty<'tcx>,
        place: crate::place::JsPlace,
        dest: Option<Local>,
    ) -> Expr {
        let layout = match self.tcx.layout_of(typing_env().as_query_input(ty)) {
            Ok(layout) => layout,
            Err(_) => return self.zombie(format!("no layout for `{ty}`")),
        };
        match layout.variants {
            rustc_abi::Variants::Multiple { .. } => {
                let value = self.read(&place);
                let Some(repr) = value::enum_repr(self.tcx, ty) else {
                    return self.zombie(format!("`{ty}` is not an enum"));
                };
                if dest.is_some_and(|local| self.tag_discrs.get(local).is_some()) {
                    return value::tag_expr(repr, value);
                }
                value::discriminant_number(self.tcx, ty, value)
                    .unwrap_or_else(|| self.zombie(format!("`{ty}` is not an enum")))
            }
            rustc_abi::Variants::Single { index } => {
                value::discriminant_literal(self.tcx, ty, index)
            }
            // Uninhabited: any code reading this discriminant is unreachable.
            rustc_abi::Variants::Empty => jsast::num(0),
        }
    }

    fn codegen_aggregate(
        &self,
        kind: &AggregateKind<'tcx>,
        ty: Ty<'tcx>,
        values: Vec<Expr>,
    ) -> Expr {
        match kind {
            AggregateKind::Array(_) | AggregateKind::Closure(..) => jsast::array(values),
            // A coroutine is built in its UNRESUMED state, holding its upvars and nothing else: the
            // saved locals belong to the states a suspension leaves it in, and MIR writes them one
            // statement at a time after the tag has moved. The upvars are keyed by index, which is
            // what `value::field_key` reads them back by with no state downcast in hand.
            AggregateKind::Coroutine(..) => {
                let unresumed = VariantIdx::from_usize(ty::CoroutineArgs::UNRESUMED);
                let upvars = values
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| (index.to_string(), value))
                    .collect();
                value::enum_value(self.tcx, ty, unresumed, upvars)
                    .unwrap_or_else(|| self.zombie(format!("`{ty}` is not a coroutine")))
            }
            AggregateKind::Tuple => {
                if values.is_empty() { jsast::undefined() } else { jsast::array(values) }
            }
            AggregateKind::Adt(def_id, variant_index, _, _, active_field) => {
                let def = self.tcx.adt_def(*def_id);
                let variant = value::variant_def(def, Some(*variant_index));
                let mut entries = Vec::new();

                // A transparent wrapper *is* its one non-1-ZST field, so building one builds the
                // field: `MaybeUninit::new(v)` is `v`, `NonNull { pointer: p }` is `p`, and
                // `MaybeUninit::uninit()` — which names the zero sized arm, whose value is
                // `undefined` — is `undefined`. A union initializes one field and so has one
                // operand whichever arm it names; a struct has an operand per field, and the one
                // that is the value is at the transparent field's own index.
                if let Some((index, field_ty)) = value::transparent_field_indexed(self.tcx, ty) {
                    // A union initializes one field and so has one operand whichever arm it names.
                    // Naming the *other* arm is `MaybeUninit::uninit()`: there is no value, and
                    // what stands for one depends on what the payload is (see [`uninit`]).
                    if def.is_union() {
                        if active_field.is_some_and(|active| active != index) {
                            return uninit(self.tcx, field_ty);
                        }
                        return values.into_iter().next().unwrap_or_else(jsast::undefined);
                    }
                    // A struct has an operand per field, and the one that is the value is at the
                    // transparent field's own index.
                    return values
                        .into_iter()
                        .nth(index.as_usize())
                        .unwrap_or_else(jsast::undefined);
                }

                match active_field {
                    // A union write names the one field it initializes.
                    Some(field) => {
                        if let Some(value) = values.into_iter().next() {
                            entries.push((crate::naming::Namer::field_name(variant, *field), value));
                        }
                    }
                    None => {
                        for (field, value) in variant.fields.indices().zip(values) {
                            entries.push((crate::naming::Namer::field_name(variant, field), value));
                        }
                    }
                }

                // An enum's shape is `value.rs`'s to spell: the variant's name, or the number a
                // `#[repr(int)]` fieldless one is.
                if def.is_enum() {
                    return value::enum_value(self.tcx, ty, *variant_index, entries)
                        .unwrap_or_else(|| self.zombie(format!("`{ty}` is not an enum")));
                }

                jsast::object(entries)
            }
            // `from_raw_parts`: a fat pointer built from a thin data pointer and its metadata.
            AggregateKind::RawPtr(pointee, _) => {
                let pointee = self.monomorphize(*pointee);
                let mut values = values.into_iter();
                let (Some(data), Some(meta)) = (values.next(), values.next()) else {
                    return self.zombie(format!(
                        "`*const {pointee}` needs both a data pointer and its metadata"
                    ));
                };
                crate::ptr::build_fat(self, pointee, data, meta)
            }
            _ => self.zombie(format!("`{kind:?}` aggregates are not supported by rustc_codegen_mires")),
        }
    }

    /// `BinOp::Cmp`: the three-way comparison, whose result is a `core::cmp::Ordering`.
    ///
    /// The variants are looked up by name and built through `value.rs`, so that this spells
    /// `Ordering::Less` exactly the way [`FnCx::codegen_aggregate`] does. `Ordering` is a fieldless
    /// `#[repr(i8)]` enum, so each of the three is the plain number its discriminant is.
    ///
    /// Both operands are read twice. They are `Operand`s, so a read is a local, a field or a
    /// constant, and reading one twice has no more effect than reading it once.
    fn codegen_three_way(
        &self,
        operand_ty: Ty<'tcx>,
        ordering_ty: Ty<'tcx>,
        lhs: Expr,
        rhs: Expr,
    ) -> Expr {
        let ty::Adt(def, _) = ordering_ty.kind() else {
            return self
                .zombie(format!("`Cmp` returned `{ordering_ty}`, which is not an `Ordering`"));
        };
        if int_info(self.tcx, operand_ty).is_none()
            && !matches!(operand_ty.kind(), ty::Bool | ty::Char)
        {
            return self
                .zombie(format!("`Cmp` on `{operand_ty}` is not supported by rustc_codegen_mires"));
        }
        let variant = |wanted: &str| -> Option<Expr> {
            let index = def
                .variants()
                .iter_enumerated()
                .find_map(|(index, variant)| (variant.name.as_str() == wanted).then_some(index))?;
            value::enum_value(self.tcx, ordering_ty, index, Vec::new())
        };
        let (Some(less), Some(equal), Some(greater)) =
            (variant("Less"), variant("Equal"), variant("Greater"))
        else {
            return self
                .zombie(format!("`Cmp` returned `{ordering_ty}`, which is not an `Ordering`"));
        };
        jsast::cond(
            jsast::binary(JsBinOp::Lt, lhs.clone(), rhs.clone()),
            less,
            jsast::cond(jsast::binary(JsBinOp::StrictEq, lhs, rhs), equal, greater),
        )
    }

    /// `BinOp::AddWithOverflow` and its siblings: the wrapped result paired with whether it wrapped.
    ///
    /// The destination is a `(T, bool)` tuple, which is a two element JS array. The exact result is
    /// computed once and bound by an immediately applied arrow function: a `let` would need a
    /// statement, and an rvalue is lowered in expression position.
    ///
    /// A 32 bit product is not exact in a double, and does not have to be — it is only compared for
    /// *equality* with the wrapped result, and a rounded product that overflowed is never equal to
    /// it.
    fn codegen_with_overflow(
        &self,
        op: MirBinOp,
        ty: Ty<'tcx>,
        lhs: Expr,
        rhs: Expr,
    ) -> Expr {
        let Some((signed, bits)) = int_info(self.tcx, ty) else {
            return self
                .zombie(format!("`{op:?}` on `{ty}` is not supported by rustc_codegen_mires"));
        };
        let repr = int_repr(ty);
        let js_op = match op {
            MirBinOp::AddWithOverflow => JsBinOp::Add,
            MirBinOp::SubWithOverflow => JsBinOp::Sub,
            _ => JsBinOp::Mul,
        };
        let exact = jsast::binary(js_op, lhs, rhs);
        let bound = jsast::id(OVERFLOW_PARAM);
        let wrapped = mask(repr, signed, bits, bound.clone());
        jsast::call(
            jsast::arrow(
                vec![OVERFLOW_PARAM.to_string()],
                jsast::array(vec![
                    wrapped.clone(),
                    jsast::binary(JsBinOp::StrictNe, wrapped, bound),
                ]),
            ),
            vec![exact],
        )
    }

    fn codegen_binop(
        &self,
        op: MirBinOp,
        ty: Ty<'tcx>,
        rhs_ty: Ty<'tcx>,
        lhs: Expr,
        rhs: Expr,
    ) -> Expr {
        let compare = |js_op: JsBinOp| jsast::binary(js_op, lhs.clone(), rhs.clone());

        // A pointer is a record rather than a value, so `===` on two of them would compare the
        // identity of two records that name the same place. `ptr.rs` owns what the comparison
        // means; every pointer relation goes through it.
        if is_pointer(ty) {
            let ptr_op = |js_op: JsBinOp| {
                crate::ptr::cmp(self, ty, js_op, lhs.clone(), rhs.clone())
            };
            match op {
                MirBinOp::Offset => return crate::ptr::add(self, pointee_of(ty), lhs, rhs),
                MirBinOp::Eq => return crate::ptr::eq(self, ty, lhs, rhs),
                MirBinOp::Ne => return crate::ptr::ne(self, ty, lhs, rhs),
                MirBinOp::Lt => return ptr_op(JsBinOp::Lt),
                MirBinOp::Le => return ptr_op(JsBinOp::Le),
                MirBinOp::Gt => return ptr_op(JsBinOp::Gt),
                MirBinOp::Ge => return ptr_op(JsBinOp::Ge),
                _ => {
                    return self.zombie(format!(
                        "`{op:?}` on `{ty}` is not supported by rustc_codegen_mires"
                    ));
                }
            }
        }

        match op {
            // `===` on two JS objects is identity, and an aggregate is a fresh object at every
            // borrow, so comparing two of them would answer a different question than Rust asks.
            // Numbers, `BigInt`s, booleans and reified functions all compare by value (or, for a
            // function, by an identity that is stable per item).
            MirBinOp::Eq | MirBinOp::Ne if !compares_by_value(self.tcx, ty) => {
                return self.zombie(format!(
                    "`{op:?}` on `{ty}` is not supported by rustc_codegen_mires"
                ));
            }
            MirBinOp::Eq => return compare(JsBinOp::StrictEq),
            MirBinOp::Ne => return compare(JsBinOp::StrictNe),
            MirBinOp::Lt => return compare(JsBinOp::Lt),
            MirBinOp::Le => return compare(JsBinOp::Le),
            MirBinOp::Gt => return compare(JsBinOp::Gt),
            MirBinOp::Ge => return compare(JsBinOp::Ge),
            _ => {}
        }

        if matches!(ty.kind(), ty::Bool) {
            return match op {
                MirBinOp::BitAnd => jsast::binary(JsBinOp::And, lhs, rhs),
                MirBinOp::BitOr => jsast::binary(JsBinOp::Or, lhs, rhs),
                MirBinOp::BitXor => jsast::binary(JsBinOp::StrictNe, lhs, rhs),
                _ => self.zombie(format!("`{op:?}` is not defined on `bool`")),
            };
        }

        if let ty::Float(float_ty) = ty.kind() {
            // An `f32` operation rounds to `f32` precision; JS arithmetic is `f64` throughout, so
            // the rounding has to be put back by hand.
            let round = |value: Expr| match float_ty {
                ty::FloatTy::F32 => jsast::fround(value),
                _ => value,
            };
            return match op {
                MirBinOp::Add => round(jsast::binary(JsBinOp::Add, lhs, rhs)),
                MirBinOp::Sub => round(jsast::binary(JsBinOp::Sub, lhs, rhs)),
                MirBinOp::Mul => round(jsast::binary(JsBinOp::Mul, lhs, rhs)),
                MirBinOp::Div => round(jsast::binary(JsBinOp::Div, lhs, rhs)),
                MirBinOp::Rem => round(jsast::binary(JsBinOp::Rem, lhs, rhs)),
                _ => self.zombie(format!("`{op:?}` is not defined on `{ty}`")),
            };
        }

        let Some((signed, bits)) = int_info(self.tcx, ty) else {
            return self.zombie(format!("`{op:?}` is not supported on `{ty}`"));
        };
        let repr = int_repr(ty);
        let mask = |value: Expr| mask(repr, signed, bits, value);

        // MIR truncates a shift amount to the width of the shifted type, and the amount arrives in
        // whatever representation *its own* type uses, which need not be the shifted type's.
        let shift_amount = |rhs: Expr| {
            let amount = match (repr, int_repr(rhs_ty)) {
                (IntRepr::BigInt, IntRepr::Number) => jsast::to_bigint(rhs),
                // Narrow first: a shift amount only ever uses its low bits, and `Number` of a
                // large `BigInt` rounds.
                (IntRepr::Number, IntRepr::BigInt) => {
                    jsast::to_number(jsast::bigint_mask(false, 32, rhs))
                }
                _ => rhs,
            };
            match repr {
                IntRepr::BigInt => {
                    jsast::binary(JsBinOp::BitAnd, amount, jsast::bigint(bits - 1))
                }
                // JavaScript's own shift operators already mask the amount to five bits.
                IntRepr::Number if bits == 32 => amount,
                IntRepr::Number => {
                    jsast::binary(JsBinOp::BitAnd, amount, jsast::num((bits - 1) as f64))
                }
            }
        };

        match op {
            MirBinOp::Add | MirBinOp::AddUnchecked => mask(jsast::binary(JsBinOp::Add, lhs, rhs)),
            MirBinOp::Sub | MirBinOp::SubUnchecked => mask(jsast::binary(JsBinOp::Sub, lhs, rhs)),
            MirBinOp::Mul | MirBinOp::MulUnchecked => {
                // A 32 bit product does not fit a double exactly; anything narrower does, and a
                // `BigInt` product is exact by construction.
                let product = if repr == IntRepr::Number && bits == 32 {
                    jsast::imul(lhs, rhs)
                } else {
                    jsast::binary(JsBinOp::Mul, lhs, rhs)
                };
                mask(product)
            }
            MirBinOp::Div => {
                // `BigInt` division truncates toward zero already; a number division does not, but
                // `| 0`, `>>> 0` and `<< n >> n` all truncate toward zero, like Rust.
                mask(jsast::binary(JsBinOp::Div, lhs, rhs))
            }
            MirBinOp::Rem => mask(jsast::binary(JsBinOp::Rem, lhs, rhs)),
            MirBinOp::BitAnd => mask(jsast::binary(JsBinOp::BitAnd, lhs, rhs)),
            MirBinOp::BitOr => mask(jsast::binary(JsBinOp::BitOr, lhs, rhs)),
            MirBinOp::BitXor => mask(jsast::binary(JsBinOp::BitXor, lhs, rhs)),
            MirBinOp::Shl | MirBinOp::ShlUnchecked => {
                mask(jsast::binary(JsBinOp::Shl, lhs, shift_amount(rhs)))
            }
            MirBinOp::Shr | MirBinOp::ShrUnchecked => {
                // `BigInt` has no `>>>`, and needs none: an unsigned value is a non-negative
                // `BigInt`, for which the arithmetic shift is the logical one.
                let js_op = if signed || repr == IntRepr::BigInt {
                    JsBinOp::Shr
                } else {
                    JsBinOp::UShr
                };
                mask(jsast::binary(js_op, lhs, shift_amount(rhs)))
            }
            _ => self.zombie(format!("`{op:?}` is not supported by rustc_codegen_mires (on `{ty}`)")),
        }
    }

    fn codegen_unop(&self, op: MirUnOp, ty: Ty<'tcx>, value: Expr) -> Expr {
        match op {
            MirUnOp::Not => match ty.kind() {
                ty::Bool => jsast::not(value),
                _ => mask_ty(self.tcx, ty, jsast::bit_not(value)),
            },
            MirUnOp::Neg => match ty.kind() {
                ty::Float(_) => jsast::neg(value),
                _ => mask_ty(self.tcx, ty, jsast::neg(value)),
            },
            // The fat pointer contract lives in `place.rs`, next to the code that builds the
            // pointers: `&[T]` is `{ buf, off, len }` and `&dyn Trait` is `{ ptr, meta }`.
            MirUnOp::PtrMetadata => self.ptr_metadata(ty, value),
        }
    }

    fn codegen_cast(&self, kind: CastKind, operand: &Operand<'tcx>, to_ty: Ty<'tcx>) -> Expr {
        let from_ty = self.operand_ty(operand);
        let value = self.codegen_operand(operand);

        match kind {
            // `bool as u8` and `char as u32` come through here too.
            CastKind::IntToInt => self.codegen_int_to_int(from_ty, to_ty, value),
            CastKind::FloatToInt => self.codegen_float_to_int(to_ty, value),
            CastKind::IntToFloat => {
                let value = match int_repr(from_ty) {
                    IntRepr::BigInt => jsast::to_number(value),
                    IntRepr::Number => value,
                };
                float_round(to_ty, value)
            }
            CastKind::FloatToFloat => float_round(to_ty, value),
            // A pointer to pointer cast is two questions, in this order.
            //
            // The first is a change of *form* rather than of pointee type: a reference to an
            // aggregate is the aggregate's own object and a raw pointer to one is a slot, so
            // `&T as *const U` boxes before it does anything else (`ptr::ref_to_raw`).
            //
            // The second is the cast matrix itself, `ptr::cast_pointer`, which owns every row
            // including the `str` ones. A cast whose two sides do not both have a pointee — a fn
            // pointer cast to a pointer — has no row: a JavaScript function is the same value
            // whichever pointer type names it.
            CastKind::PtrToPtr | CastKind::FnPtrToPtr => {
                let (Some(from_pointee), Some(to_pointee)) =
                    (from_ty.builtin_deref(true), to_ty.builtin_deref(true))
                else {
                    return value;
                };
                let value = if matches!(from_ty.kind(), ty::Ref(..))
                    && matches!(to_ty.kind(), ty::RawPtr(..))
                {
                    crate::ptr::ref_to_raw(self, from_pointee, value)
                } else {
                    value
                };
                crate::ptr::cast_pointer(self, from_pointee, to_pointee, value)
            }
            // Exposing an address is the pointer to integer direction: a real pointer's synthetic
            // address comes from `__rt.addr` (see `ptr::addr`), an integer-born one passes
            // through.
            CastKind::PointerExposeProvenance => {
                let address = match from_ty.builtin_deref(true) {
                    Some(pointee) => crate::ptr::addr(self, pointee, value),
                    None => value,
                };
                mask_ty(self.tcx, to_ty, address)
            }
            // Re-materializing from an integer keeps the number, and `Subtype` never changes the
            // value; `ptr::cast_str_ptr` answers `None` for both and the value goes through
            // unchanged.
            CastKind::PointerWithExposedProvenance | CastKind::Subtype => {
                crate::ptr::cast_str_ptr(self, from_ty, to_ty, value.clone()).unwrap_or(value)
            }
            CastKind::PointerCoercion(coercion, _) => {
                use rustc_middle::ty::adjustment::PointerCoercion;
                match coercion {
                    PointerCoercion::ReifyFnPointer(_) => match *from_ty.kind() {
                        ty::FnDef(def_id, generic_args) => {
                            match Instance::resolve_for_fn_ptr(
                                self.tcx,
                                typing_env(),
                                def_id,
                                generic_args.no_bound_vars().unwrap(),
                            ) {
                                Some(instance) => {
                                    jsast::id(self.cgu.namer.fn_name(instance).into_string())
                                }
                                None => self
                                    .zombie(format!("cannot take a pointer to `{from_ty}`")),
                            }
                        }
                        _ => self.zombie(format!("cannot reify `{from_ty}`")),
                    },
                    PointerCoercion::ClosureFnPointer(_) => match *from_ty.kind() {
                        ty::Closure(def_id, generic_args) => {
                            let instance = Instance::resolve_closure(
                                self.tcx,
                                def_id,
                                generic_args,
                                ty::ClosureKind::FnOnce,
                            );
                            let target =
                                jsast::id(self.cgu.namer.fn_name(instance).into_string());
                            // The shim that instance names is `FnOnce::call_once`, whose first
                            // parameter is the closure's environment; a `fn` pointer is called
                            // without one. Only a non-capturing closure coerces, so the
                            // environment is zero sized and `undefined` is the whole of it — but
                            // it still occupies a parameter slot, so the coercion is a wrapper
                            // that puts it back.
                            let arity =
                                self.tcx.instantiate_bound_regions_with_erased(to_ty.fn_sig(self.tcx))
                                    .inputs()
                                    .len();
                            let params: Vec<String> =
                                (0..arity).map(|i| format!("$a{i}")).collect();
                            let mut arguments = vec![jsast::undefined()];
                            arguments
                                .extend(params.iter().map(|name| jsast::id(name.clone())));
                            jsast::arrow(params, jsast::call(target, arguments))
                        }
                        _ => self.zombie(format!("cannot coerce `{from_ty}` to a fn pointer")),
                    },
                    // `*mut T` and `*const T` are the same record.
                    PointerCoercion::MutToConstPointer => value,
                    // `*const [T; N]` to `*const T`: a slot naming the array becomes a slot
                    // naming its first element, which is a reprojection *into* the array the
                    // outer slot points at rather than a change of representation.
                    PointerCoercion::ArrayToPointer => {
                        crate::ptr::slot(crate::ptr::slot_element(value), jsast::num(0))
                    }
                    PointerCoercion::Unsize => {
                        crate::unsize::coerce_unsized(self, value, from_ty, to_ty)
                    }
                    _ => self.zombie(format!(
                        "`{coercion:?}` coercions are not supported by rustc_codegen_mires \
                         (`{from_ty}` to `{to_ty}`)"
                    )),
                }
            }
            CastKind::Transmute => {
                crate::intrinsics::codegen_transmute(self, from_ty, to_ty, value)
            }
        }
    }

    /// An `as` cast between two integer types, including `bool as u8` and `char as u32`.
    ///
    /// Rust defines the cast as a sign or zero extension of the source to the destination width
    /// followed by a truncation, and the four representation pairs each spell that differently:
    ///
    /// | from → to | JavaScript |
    /// |---|---|
    /// | number → number | `x \| 0`, `x >>> 0`, `x << n >> n`, `x & m` |
    /// | number → BigInt | `BigInt.asIntN(bits, BigInt(x))` |
    /// | BigInt → BigInt | `BigInt.asIntN(bits, x)` |
    /// | BigInt → number | `Number(BigInt.asIntN(bits, x))` |
    ///
    /// The truncation always happens *before* `Number()`, never after: `Number(2n ** 64n - 1n)` is
    /// rounded to `18446744073709551616`, whose low 32 bits are zero rather than the `-1` the cast
    /// owes. Truncating in `BigInt` first leaves a value the destination holds exactly.
    fn codegen_int_to_int(&self, from_ty: Ty<'tcx>, to_ty: Ty<'tcx>, value: Expr) -> Expr {
        // `u8 as char`, the one cast Rust allows into `char`, and the identity here: a `char` is
        // its scalar value, which is a JavaScript number, and every one of the 256 `u8`s is a
        // valid scalar value — so there is nothing to check and nothing to convert. `str`'s ASCII
        // machinery is written with it, which is why leaving it out zombied most of `core`.
        if matches!(to_ty.kind(), ty::Char) {
            return value;
        }
        let Some((signed, bits)) = int_info(self.tcx, to_ty) else {
            return self.zombie(format!("`{from_ty} as {to_ty}` is not an integer cast"));
        };
        let to_repr = int_repr(to_ty);

        // `false`/`true` are `0`/`1` in every width, so they need no truncation.
        if matches!(from_ty.kind(), ty::Bool) {
            return match to_repr {
                IntRepr::BigInt => jsast::cond(value, jsast::bigint(1), jsast::bigint(0)),
                IntRepr::Number => jsast::cond(value, jsast::num(1), jsast::num(0)),
            };
        }

        match (int_repr(from_ty), to_repr) {
            (IntRepr::Number, IntRepr::Number) => mask_ty(self.tcx, to_ty, value),
            (IntRepr::Number, IntRepr::BigInt) => {
                jsast::bigint_mask(signed, bits, jsast::to_bigint(value))
            }
            (IntRepr::BigInt, IntRepr::BigInt) => jsast::bigint_mask(signed, bits, value),
            (IntRepr::BigInt, IntRepr::Number) => {
                jsast::to_number(jsast::bigint_mask(signed, bits, value))
            }
        }
    }

    /// A float to integer `as` cast, which Rust defines as saturating, with `NaN` mapping to zero.
    ///
    /// `Math.trunc` alone would produce `Infinity` or `NaN` for an out of range input, and the
    /// truncation that follows turns those into whatever the bitwise operators happen to do, so
    /// the clamping is done up front by the shim.
    fn codegen_float_to_int(&self, to_ty: Ty<'tcx>, value: Expr) -> Expr {
        let Some((signed, bits)) = int_info(self.tcx, to_ty) else {
            return self.zombie(format!("`{to_ty}` is not an integer"));
        };
        match int_repr(to_ty) {
            IntRepr::BigInt => {
                jsast::rt_call("f2i_big", vec![value, jsast::num(bits), jsast::boolean(signed)])
            }
            IntRepr::Number => {
                let (min, max) = if signed {
                    (-(2f64.powi(bits as i32 - 1)), 2f64.powi(bits as i32 - 1) - 1.0)
                } else {
                    (0.0, 2f64.powi(bits as i32) - 1.0)
                };
                jsast::rt_call("f2i", vec![value, jsast::num(min), jsast::num(max)])
            }
        }
    }

    /// A `SwitchInt` case value, which MIR stores as an unsigned bit pattern: a `-1i64` arm
    /// arrives as `18446744073709551615`, and `switch` compares with `===`, so the literal has to
    /// come back out in the scrutinee's own representation and signedness.
    pub(crate) fn switch_case_value(&self, ty: Ty<'tcx>, value: u128) -> Expr {
        value::int_literal(self.tcx, ty, value).unwrap_or_else(|| jsast::num(value as f64))
    }
}

/// Whether this type is a pointer, and so answers its relations through `ptr.rs`.
fn is_pointer<'tcx>(ty: Ty<'tcx>) -> bool {
    matches!(value::peel_pattern(ty).kind(), ty::RawPtr(..) | ty::Ref(..))
}

/// The pointee of a pointer type, for the operators that only ever see one.
fn pointee_of<'tcx>(ty: Ty<'tcx>) -> Ty<'tcx> {
    value::peel_pattern(ty).builtin_deref(true).unwrap_or(ty)
}

/// Whether `===` on two values of this type answers the question Rust's `==` asks.
///
/// It does for the JS primitives and for a reified function (two reifications of one item are the
/// same identifier, so the same function object). It does not for anything represented as a JS
/// object: those compare by identity, and the backend builds a fresh object at every borrow.
///
/// An enum whose every variant is fieldless is a number or a string (`value::EnumRepr`), so two of
/// them compare exactly the way the variants do.
fn compares_by_value<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    if value::is_primitive_enum(tcx, ty) {
        return true;
    }
    matches!(
        ty.kind(),
        ty::Int(_) | ty::Uint(_) | ty::Float(_) | ty::Bool | ty::Char | ty::FnPtr(..) | ty::FnDef(..)
    )
}

/// Rounds to `f32` precision if `ty` is `f32`, which JS arithmetic does not do on its own.
fn float_round<'tcx>(ty: Ty<'tcx>, value: Expr) -> Expr {
    match ty.kind() {
        ty::Float(ty::FloatTy::F32) => jsast::fround(value),
        _ => value,
    }
}

/// The value of an *uninitialized* place of type `ty`: `MaybeUninit::<T>::uninit()`.
///
/// `undefined` says it exactly — nothing is there — and that is what a scalar, an aggregate or a
/// pointer gets. The only lawful thing a program may do with such a value is write to it, and a
/// write reaches the place through the pointer that named it, which for a local is the box the
/// borrow forced (`uses.rs`).
///
/// An **array** payload is different, and `MaybeUninit::<[T; N]>::uninit()` is how `core` spells
/// every uninitialized scratch buffer it sorts and merges through. A pointer *into* one is
/// `{ buf, off }` over the array itself, so the buffer has to exist before the first write:
/// `new Array(N)` is N holes, a real JavaScript buffer of the right length whose elements read
/// `undefined` until they are written.
fn uninit<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Expr {
    let peeled = value::peel_pattern(value::peel_transparent(tcx, ty));
    match peeled.kind() {
        ty::Array(_, count) => match count.try_to_target_usize(tcx) {
            Some(len) => jsast::new(jsast::id("Array"), vec![jsast::num(len as f64)]),
            None => jsast::undefined(),
        },
        _ => jsast::undefined(),
    }
}
