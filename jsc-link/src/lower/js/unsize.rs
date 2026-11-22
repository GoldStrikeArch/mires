//! Unsizing to `dyn`, and `transmute`: v1 `backend/src/unsize.rs`, `vtable.rs` and the
//! `codegen_transmute` half of `backend/src/intrinsics.rs`.
//!
//! # Vtables
//!
//! `&T as &dyn Trait` needs the **name** of the vtable item `T as Trait` is emitted under, which
//! v1 asked `namer.vtable_name(ty, trait_ref)` for. A link-time lowering has no namer and no
//! printed type paths, so the driver hands over an index built from the vtable items the program
//! already contains: every `BodyKind::Vtable` body carries the `concrete` type and the trait it
//! implements, which is exactly the key. See [`VtableIndex`].
//!
//! A `dyn` to `dyn` upcast is declined rather than guessed: which supertrait slot to read is
//! `tcx.supertrait_vtable_slot`'s answer and no part of JIR-S records it.
//!
//! # `transmute`
//!
//! A transmute reinterprets bytes, and this backend has no bytes. What it has is the small set of
//! cases where the *value* is unchanged, or is related to the target by a computation: the same
//! representation class on both sides, a single-field wrapper peeled and rebuilt, a float and its
//! bit pattern through a pair of typed array views, `[u8; N]` and the integer its bytes spell, and
//! the niche decode `NonZero::new` is written as. Anything else is v1's zombie.

use std::collections::BTreeMap;

use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::repr::{OpKind, ValueId};
use rcgjs_object::jir::semantic::DefinitionId;
use rcgjs_object::jir::types::{EnumRepr, FieldKey, IntRepr, NominalForm, TypeId, TypeKind};
use rcgjs_object::jsast::{self, BinOp, Expr, Stmt};

use super::{decline, value, JsCx, Res};

/// The target's pointer width in bits, which `wasm32-unknown-unknown` makes 32.
pub const POINTER_BITS: u32 = 32;

/// The parameter the byte conversions bind their operand to, so that it is evaluated once.
const BYTES_PARAM: &str = "$b";
/// The parameter the niche decode binds the incoming scalar to.
const NICHE_PARAM: &str = "$n";

/// Every vtable item of the program by the pair it is the vtable of: the concrete type and the
/// principal trait (`None` for a `dyn Send`-shaped object, whose vtable is the three header
/// entries only).
pub type VtableIndex = BTreeMap<(TypeId, Option<DefinitionId>), String>;

/// The empty index, for a context with no driver behind it (the unit tests).
pub static EMPTY_VTABLES: VtableIndex = BTreeMap::new();

impl JsCx<'_> {
    /// The JavaScript name of the vtable of `concrete as dyn trait_ref` (`vtable.rs`).
    pub fn vtable_name(&self, concrete: TypeId, trait_ref: Option<DefinitionId>) -> Res<String> {
        // A vtable is keyed on the type it was built for, which is the *peeled* type only when
        // the pointee itself was peeled; both spellings are tried so a `repr(transparent)`
        // wrapper reaching here still finds its own vtable.
        let peeled = self.table.peeled(concrete);
        for key in [(concrete, trait_ref), (peeled, trait_ref)] {
            if let Some(name) = self.vtables.get(&key) {
                return Ok(name.clone());
            }
        }
        Err(decline(
            OpClass::Unsize,
            "an unsizing coercion to `dyn` whose vtable item is not in the program's index",
        ))
    }

    /// `transmute::<From, To>(value)` (`intrinsics.rs::codegen_transmute`).
    pub fn transmute(&self, from_ty: TypeId, to_ty: TypeId, value: Expr) -> Res<Expr> {
        if from_ty == to_ty {
            return Ok(value);
        }
        // Peel the single-field wrappers off the source, then rebuild the ones the destination
        // has. Doing it in that order keeps the two sides independent.
        let (inner_from, inner_value) = self.unwrap_wrappers(from_ty, value);
        let (inner_to, wrappers) = self.target_wrappers(to_ty);
        let scalar = self.transmute_scalar(inner_from, inner_to, inner_value)?;
        Ok(self.rebuild(&wrappers, scalar))
    }

    /// Lowers `transmute` whose destination is a 64-bit integer `(lo, hi)` pair.
    pub(super) fn transmute_to_i64_pair(
        &self,
        stmts: &mut Vec<Stmt>,
        from_ty: TypeId,
        to_ty: TypeId,
        operand: ValueId,
    ) -> Res<(Expr, Expr)> {
        let table = self.table;
        if from_ty == to_ty
            || table.peeled(from_ty) == table.peeled(to_ty)
            || self.is_i64_ty(from_ty)
        {
            return self.pair_of(operand);
        }
        if byte_array_len(table, from_ty) == Some(8) {
            if self.is_inline(operand) {
                if let Some(op) = self.def_of(operand) {
                    if let OpKind::Aggregate { fields, .. } = &op.kind {
                        if fields.len() == 8 {
                            let mut bytes = Vec::with_capacity(8);
                            for &v in fields {
                                bytes.push(self.expr_of(v)?);
                            }
                            let arr: [Expr; 8] = bytes
                                .try_into()
                                .expect("verified 8 elements for [u8; 8] transmute");
                            return Ok(super::i64_pair::pack_8_bytes(arr));
                        }
                    }
                }
            }
            let arr = self.expr_of(operand)?;
            return Ok(super::i64_pair::bytes_expr_to_i64_pair(self, stmts, arr));
        }
        if value::float_bits(table, table.peeled(from_ty)) == Some(64) {
            let val = self.expr_of(operand)?;
            return Ok(super::i64_pair::f64_to_bits_pair(self, stmts, val));
        }
        let expr = self.transmute(from_ty, to_ty, self.expr_of(operand)?)?;
        let expr = super::i64_pair::materialize(self, stmts, expr);
        Ok(super::i64_pair::unpack_bigint_exprs(expr))
    }

    /// Lowers `transmute` whose source is a 64-bit integer `(lo, hi)` pair and whose destination is
    /// NOT a 64-bit integer.
    pub(super) fn transmute_from_i64_pair(
        &self,
        stmts: &mut Vec<Stmt>,
        from_ty: TypeId,
        to_ty: TypeId,
        operand: ValueId,
    ) -> Res<Expr> {
        let table = self.table;
        if byte_array_len(table, to_ty) == Some(8) {
            let pair = self.pair_of(operand)?;
            return Ok(super::i64_pair::i64_pair_to_bytes_expr(self, stmts, pair));
        }
        if value::float_bits(table, table.peeled(to_ty)) == Some(64) {
            let pair = self.pair_of(operand)?;
            return Ok(super::i64_pair::bits_pair_to_f64(pair));
        }
        self.transmute(from_ty, to_ty, self.expr_of(operand)?)
    }

    /// The scalar half of a transmute, once both sides have had their wrappers removed.
    fn transmute_scalar(&self, from: TypeId, to: TypeId, value: Expr) -> Res<Expr> {
        let table = self.table;
        // Two types with the same *peeled* type have the same JavaScript value, whatever
        // transparent wrappers stand between them (`JsFacts::peeled`), so the transmute is the
        // identity. v1 asked the same question by peeling as it unwrapped.
        if from == to || table.peeled(from) == table.peeled(to) {
            return Ok(value);
        }
        // Two arrays of the same length whose elements have the same JavaScript representation
        // *are* the same array (`[T; N]` to `[MaybeUninit<T>; N]`, which `array::IntoIter` is
        // written as).
        if let (
            Some(TypeKind::Array {
                element: from_element,
                length: from_len,
            }),
            Some(TypeKind::Array {
                element: to_element,
                length: to_len,
            }),
        ) = (
            value::kind(table, table.peeled(from)),
            value::kind(table, table.peeled(to)),
        ) {
            if from_len == to_len && table.peeled(*from_element) == table.peeled(*to_element) {
                return Ok(value);
            }
        }
        // A zero sized value carries no bits, and is `undefined`.
        if table.is_zst(to) {
            return Ok(jsast::undefined());
        }
        // `str::as_bytes` and `str::from_utf8_unchecked` are both written as a transmute between
        // two pointers, and neither is one here: a `str` is a JavaScript string.
        if let Some(converted) = self.cast_str_ptr(from, to, value.clone()) {
            return converted;
        }

        let from_class = ReprClass::of(table, from);
        let to_class = ReprClass::of(table, to);
        Ok(match (from_class, to_class) {
            (ReprClass::Int(a), ReprClass::Int(b)) if a.bits == b.bits => {
                value::mask(b.repr, b.signed, b.bits, value)
            }
            (ReprClass::Bool, ReprClass::Int(b)) => value::mask(
                b.repr,
                b.signed,
                b.bits,
                jsast::cond(value, jsast::num(1), jsast::num(0)),
            ),
            (ReprClass::Int(_), ReprClass::Bool) => {
                jsast::binary(BinOp::StrictNe, value, jsast::num(0))
            }
            (ReprClass::Char, ReprClass::Int(b)) => value::mask(b.repr, b.signed, b.bits, value),
            (ReprClass::Int(_), ReprClass::Char) => value,
            (ReprClass::Float(bits), ReprClass::Int(b)) if bits == b.bits => {
                bit_cast(float_array(bits), int_array(b), value)
            }
            (ReprClass::Int(a), ReprClass::Float(bits)) if a.bits == bits => {
                bit_cast(int_array(a), float_array(bits), value)
            }
            // Two pointers: the same question `CastKind::PtrToPtr` asks, and the same answer.
            // Crossing between a reference and a raw pointer (including pattern-wrapped raw
            // pointers like `NonNull<T>`) converts aggregate references with `ref_to_raw` /
            // `raw_to_ref`, while keeping bare array references (`&[E; N] -> NonNull<E>`) directly
            // on `__rt.unwindow` and aggregate references erased to `*const ()` / `NonNull<()>` in
            // their reference form.
            (ReprClass::Pointer(_), ReprClass::Pointer(_)) => {
                match (value::pointee_of(table, from), value::pointee_of(table, to)) {
                    (Some(from_pointee), Some(to_pointee)) => {
                        let from_is_ref = !value::is_raw(table, from);
                        let to_is_ref = !value::is_raw(table, to);
                        let from_shape = table.peeled(from_pointee);
                        let to_shape = table.peeled(to_pointee);
                        let preserve_ref = from_is_ref
                            && ((to_is_ref && from_shape == to_shape)
                                || (!to_is_ref
                                    && table.is_zst(to_pointee)
                                    && value::is_slot_pointee(table, from_pointee)
                                    && table.is_indirect(from_pointee)));
                        if preserve_ref {
                            value
                        } else {
                            let unwindowing_array_ref = from_is_ref
                                && matches!(
                                    value::kind(table, from_shape),
                                    Some(TypeKind::Array { .. })
                                )
                                && !matches!(
                                    value::kind(table, to_shape),
                                    Some(TypeKind::Array { .. })
                                )
                                && !table.is_zst(to_pointee);
                            let raw_in = if from_is_ref && !unwindowing_array_ref {
                                self.ref_value_to_raw(from_pointee, value)
                            } else {
                                value
                            };
                            let raw_out = self.cast_pointer(from_pointee, to_pointee, raw_in)?;
                            if to_is_ref {
                                self.raw_to_ref(to_pointee, raw_out)
                            } else {
                                raw_out
                            }
                        }
                    }
                    _ => value,
                }
            }
            // A thin pointer and a pointer sized integer.
            (ReprClass::Pointer(true), ReprClass::Int(b)) if b.bits == POINTER_BITS => {
                let address = match value::pointee_of(table, from) {
                    Some(pointee) => {
                        let raw = if !value::is_raw(table, from) {
                            self.ref_value_to_raw(pointee, value)
                        } else {
                            value
                        };
                        self.ptr_addr(pointee, raw)
                    }
                    None if matches!(value::kind(table, from), Some(TypeKind::FnPtr { .. })) => {
                        jsast::rt_call("addr", vec![value, jsast::num(1.0)])
                    }
                    None => value,
                };
                value::mask(b.repr, b.signed, b.bits, address)
            }
            (ReprClass::Int(a), ReprClass::Pointer(true)) if a.bits == POINTER_BITS => {
                if a.signed {
                    value::mask(a.repr, false, a.bits, value)
                } else {
                    value
                }
            }
            // `[u8; N]` and `uN`, little endian.
            (ReprClass::Other, ReprClass::Int(b))
                if matches!(b.bits, 16 | 32 | 64)
                    && byte_array_len(table, from) == Some(u64::from(b.bits / 8)) =>
            {
                bytes_to_int(b, value)
            }
            (ReprClass::Int(a), ReprClass::Other)
                if matches!(a.bits, 16 | 32 | 64)
                    && byte_array_len(table, to) == Some(u64::from(a.bits / 8)) =>
            {
                int_to_bytes(a, value)
            }
            // A fieldless enum with an integer `repr` and an integer of its own width: such a
            // value *is* its discriminant, so moving between the two moves nothing at all when the
            // signedness matches and masks to the destination's signedness when they differ.
            (ReprClass::Other, ReprClass::Int(b))
                if direct_tag(table, from).is_some_and(|(bits, _)| bits == b.bits) =>
            {
                let (_, from_signed) = direct_tag(table, from).unwrap();
                if from_signed == b.signed {
                    value
                } else {
                    value::mask(b.repr, b.signed, b.bits, value)
                }
            }
            (ReprClass::Int(a), ReprClass::Other)
                if direct_tag(table, to).is_some_and(|(bits, _)| bits == a.bits) =>
            {
                let (_, to_signed) = direct_tag(table, to).unwrap();
                if a.signed == to_signed {
                    value
                } else {
                    value::mask(a.repr, to_signed, a.bits, value)
                }
            }
            // The same value seen as a *pointer* rather than as an integer
            // (`NonNull::dangling()` is a transmute of an `Alignment` straight to a pointer).
            (ReprClass::Other, ReprClass::Pointer(true))
                if direct_tag(table, from) == Some((POINTER_BITS, false)) =>
            {
                value
            }
            _ => match self.niche_from_scalar(from, to, value.clone())? {
                Some(decoded) => decoded,
                None => match self.niche_to_scalar(from, to, value)? {
                    Some(encoded) => encoded,
                    None => self.zombie(
                        "`transmute` between these types is not supported by rustc_codegen_js"
                            .to_owned(),
                    ),
                },
            },
        })
    }

    /// Converts a reference value (rather than a place expression) to a raw pointer.
    /// Unlike `ptr::ref_to_raw`, this never treats an enclosing `Expr::Member` or `Expr::Index`
    /// (such as `$n._0` from `niche_to_scalar` or a wrapper field) as the pointee's container.
    fn ref_value_to_raw(&self, pointee: TypeId, reference: Expr) -> Expr {
        if self.table.unsized_tail(pointee).is_some() {
            return jsast::rt_call("tail_raw", vec![reference]);
        }
        if !value::is_slot_pointee(self.table, pointee) || !self.table.is_indirect(pointee) {
            return reference;
        }
        jsast::rt_call("box", vec![reference])
    }

    /// Strips every single-non-ZST-field wrapper off a value.
    fn unwrap_wrappers(&self, ty: TypeId, value: Expr) -> (TypeId, Expr) {
        let (mut ty, mut value) = (ty, value);
        for _ in 0..16 {
            let Some((inner, key)) = self.sole_field(ty) else {
                break;
            };
            value = value::field_expr(self.interns, value, key);
            ty = inner;
        }
        (ty, value)
    }

    /// The destination's wrappers: the innermost type, and the `(key, arity)` entries to put back
    /// around a value of it, outermost first.
    fn target_wrappers(&self, ty: TypeId) -> (TypeId, Vec<(FieldKey, usize)>) {
        let mut ty = ty;
        let mut wrappers = Vec::new();
        for _ in 0..16 {
            let Some((inner, key, arity)) = self.sole_field_with_arity(ty) else {
                break;
            };
            wrappers.push((key, arity));
            ty = inner;
        }
        (ty, wrappers)
    }

    /// Puts the destination's wrappers back around a transmuted value.
    fn rebuild(&self, wrappers: &[(FieldKey, usize)], value: Expr) -> Expr {
        let mut value = value;
        for &(key, arity) in wrappers.iter().rev() {
            value = match key {
                // A struct is an object keyed by field name; the zero sized fields that were
                // skipped are `undefined`, which is what reading a missing key gives anyway.
                FieldKey::Name(_) => match value::key_name(self.interns, key) {
                    Some(name) => jsast::object(vec![(name, value)]),
                    None => value,
                },
                FieldKey::Index(idx) => {
                    let idx = idx as usize;
                    let len = arity.max(idx + 1);
                    let mut elems = Vec::with_capacity(len);
                    let mut val = Some(value);
                    for i in 0..len {
                        elems.push(if i == idx {
                            val.take().expect("idx < len")
                        } else {
                            jsast::undefined()
                        });
                    }
                    jsast::array(elems)
                }
                // A transparent union is its field: there is no wrapper to put back.
                FieldKey::Transparent => value,
            };
        }
        value
    }

    /// A `transmute` from an integer into a niche encoded enum, decoded at run time.
    ///
    /// This is how `NonZero::<T>::new` is written, and through it a good deal of `core::num`. It
    /// applies only when the destination is an enum whose tag is niche encoded and as wide as the
    /// source integer, every inhabited niche variant carries no non-zero-sized data, and the
    /// untagged variant carries exactly one non-zero-sized field.
    fn niche_from_scalar(&self, from: TypeId, to: TypeId, value: Expr) -> Res<Option<Expr>> {
        let table = self.table;
        let Some((_, bits)) = table.int_info(from) else {
            return Ok(None);
        };
        let peeled = table.peeled(to);
        let Some(TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            niche: Some(niche),
            ..
        }) = value::kind(table, peeled)
        else {
            return Ok(None);
        };
        if value::size_of(table, from) != value::size_of(table, peeled)
            || value::niche_width(table, variants, niche) != Some(bits)
        {
            return Ok(None);
        }
        let Some(tags) = value::niche_variant_tags(table, variants, niche, bits) else {
            return Ok(None);
        };
        let mut niche_cases = Vec::with_capacity(tags.len());
        for (variant_index, raw_bits) in tags {
            let empty = &variants[variant_index as usize];
            if empty.fields.iter().any(|field| !table.is_zst(field.ty)) {
                return Ok(None);
            }
            let Some(niche_lit) = value::int_literal(table, from, raw_bits) else {
                return Ok(None);
            };
            let Some(empty_value) =
                value::enum_value(table, self.interns, to, variant_index, Vec::new())
            else {
                return Ok(None);
            };
            niche_cases.push((niche_lit, empty_value));
        }

        let untagged = niche.untagged_variant;
        let payload = &variants[untagged as usize];
        let mut live = payload.fields.iter().filter(|f| !table.is_zst(f.ty));
        let Some(field) = live.next() else {
            return Ok(None);
        };
        if live.next().is_some() {
            return Ok(None);
        }
        let Some(key) = value::key_name(self.interns, field.key) else {
            return Ok(None);
        };
        let bound = jsast::id(NICHE_PARAM);
        let filled_field = self.transmute(from, field.ty, bound.clone())?;
        let Some(filled) =
            value::enum_value(table, self.interns, to, untagged, vec![(key, filled_field)])
        else {
            return Ok(None);
        };
        let mut decoded = filled;
        for (niche_lit, empty_value) in niche_cases.into_iter().rev() {
            decoded = jsast::cond(
                jsast::binary(BinOp::StrictEq, bound.clone(), niche_lit),
                empty_value,
                decoded,
            );
        }
        Ok(Some(jsast::call(
            jsast::arrow(vec![NICHE_PARAM.to_owned()], decoded),
            vec![value],
        )))
    }

    /// The other direction of [`Self::niche_from_scalar`]: a niche encoded enum transmuted to the
    /// integer its layout stores (`intrinsics.rs::niche_to_scalar`).
    ///
    /// `core` writes `PartialOrd::lt` for `[T]` as a transmute of a `ControlFlow<bool>` to `u8`
    /// (`slice/cmp.rs`, `as_underlying`). Each inhabited niche variant maps to its wrapped niche
    /// bit pattern formatted at the destination integer's width and signedness, and the untagged
    /// variant is its one payload field transmuted into the integer. Both directions round trip
    /// across all signed and unsigned integer widths and wrapping niche ranges.
    fn niche_to_scalar(&self, from: TypeId, to: TypeId, value: Expr) -> Res<Option<Expr>> {
        let table = self.table;
        let Some((_, bits)) = table.int_info(to) else {
            return Ok(None);
        };
        let peeled = table.peeled(from);
        let Some(TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            niche: Some(niche),
            ..
        }) = value::kind(table, peeled)
        else {
            return Ok(None);
        };
        if value::size_of(table, to) != value::size_of(table, peeled)
            || value::niche_width(table, variants, niche) != Some(bits)
        {
            return Ok(None);
        }
        let Some(repr) = table.enum_repr(peeled) else {
            return Ok(None);
        };
        let Some(tags) = value::niche_variant_tags(table, variants, niche, bits) else {
            return Ok(None);
        };
        let mut niche_cases = Vec::with_capacity(tags.len());
        for (variant_index, raw_bits) in tags {
            let empty = &variants[variant_index as usize];
            if empty.fields.iter().any(|field| !table.is_zst(field.ty)) {
                return Ok(None);
            }
            let Some(niche_lit) = value::int_literal(table, to, raw_bits) else {
                return Ok(None);
            };
            let Some(tag_lit) = value::variant_tag(table, self.interns, peeled, variant_index)
            else {
                return Ok(None);
            };
            niche_cases.push((tag_lit, niche_lit));
        }

        let untagged = niche.untagged_variant;
        let payload = &variants[untagged as usize];
        let mut live = payload.fields.iter().filter(|f| !table.is_zst(f.ty));
        let Some(field) = live.next() else {
            return Ok(None);
        };
        if live.next().is_some() {
            return Ok(None);
        }
        let bound = jsast::id(NICHE_PARAM);
        let payload_bits = self.transmute(
            field.ty,
            to,
            value::field_expr(self.interns, bound.clone(), field.key),
        )?;
        let mut encoded = payload_bits;
        for (tag_lit, niche_lit) in niche_cases.into_iter().rev() {
            let is_variant = jsast::binary(
                BinOp::StrictEq,
                value::tag_expr(repr, bound.clone()),
                tag_lit,
            );
            encoded = jsast::cond(is_variant, niche_lit, encoded);
        }
        Ok(Some(jsast::call(
            jsast::arrow(vec![NICHE_PARAM.to_owned()], encoded),
            vec![value],
        )))
    }
}

/// The width and signedness of a fieldless enum whose value **is** its discriminant
/// (`value.rs::direct_tag`).
fn direct_tag(table: &rcgjs_object::jir::types::TypeTable, ty: TypeId) -> Option<(u32, bool)> {
    match table.enum_repr(table.peeled(ty))? {
        EnumRepr::Number { bits, signed } => Some((bits, signed)),
        _ => None,
    }
}

/// The length of `ty` if it is an array of bytes, and `None` otherwise.
fn byte_array_len(table: &rcgjs_object::jir::types::TypeTable, ty: TypeId) -> Option<u64> {
    let TypeKind::Array { element, length } = value::kind(table, table.peeled(ty))? else {
        return None;
    };
    value::is_u8(table, table.peeled(*element)).then_some(*length)
}

/// `new To(From.of(value).buffer)[0]`: the bits of `value`, read back as another type.
///
/// Uses `From.of(value)` rather than `new From([value])` so no intermediate JS `Array` literal is
/// constructed: in V8, transitioning a `[value]` array literal to `PACKED_DOUBLE_ELEMENTS` on
/// feedback-vector allocation canonicalizes `NaN` payloads on the 10th call (MIRES-023).
fn bit_cast(from: &str, to: &str, value: Expr) -> Expr {
    let source = jsast::method_call(jsast::id(from), "of", vec![value]);
    let view = jsast::new(jsast::id(to), vec![jsast::member(source, "buffer")]);
    jsast::index(view, jsast::num(0))
}

/// The typed array constructor holding a float of this width.
fn float_array(bits: u32) -> &'static str {
    if bits == 32 {
        "Float32Array"
    } else {
        "Float64Array"
    }
}

/// The typed array constructor holding an integer of this width and signedness.
fn int_array(int: Repr) -> &'static str {
    match (int.signed, int.bits) {
        (true, 32) => "Int32Array",
        (false, 32) => "Uint32Array",
        (true, _) => "BigInt64Array",
        (false, _) => "BigUint64Array",
    }
}

/// `[u8; N]` to the integer those bytes spell, little endian.
fn bytes_to_int(int: Repr, value: Expr) -> Expr {
    let bytes = jsast::id(BYTES_PARAM);
    let big = int.repr == IntRepr::BigInt;
    let byte = |index: u32| {
        let read = jsast::index(bytes.clone(), jsast::num(index));
        if big {
            jsast::call(jsast::id("BigInt"), vec![read])
        } else {
            read
        }
    };
    let shift = |amount: u32| {
        if big {
            jsast::bigint(amount)
        } else {
            jsast::num(amount)
        }
    };
    let mut combined = byte(0);
    for index in 1..int.bits / 8 {
        let shifted = jsast::binary(BinOp::Shl, byte(index), shift(index * 8));
        combined = jsast::binary(BinOp::BitOr, combined, shifted);
    }
    let combined = value::mask(int.repr, int.signed, int.bits, combined);
    jsast::call(
        jsast::arrow(vec![BYTES_PARAM.to_owned()], combined),
        vec![value],
    )
}

/// The other direction: the `N` bytes of an integer, least significant first.
fn int_to_bytes(int: Repr, value: Expr) -> Expr {
    let bound = jsast::id(BYTES_PARAM);
    let big = int.repr == IntRepr::BigInt;
    let bytes = (0..int.bits / 8)
        .map(|index| {
            let shifted = if index == 0 {
                bound.clone()
            } else if big {
                jsast::binary(BinOp::Shr, bound.clone(), jsast::bigint(index * 8))
            } else {
                jsast::binary(BinOp::UShr, bound.clone(), jsast::num(index * 8))
            };
            let mask = if big {
                jsast::bigint(255u32)
            } else {
                jsast::num(255)
            };
            let byte = jsast::binary(BinOp::BitAnd, shifted, mask);
            if big {
                jsast::call(jsast::id("Number"), vec![byte])
            } else {
                byte
            }
        })
        .collect();
    jsast::call(
        jsast::arrow(vec![BYTES_PARAM.to_owned()], jsast::array(bytes)),
        vec![value],
    )
}

/// An integer type, reduced to what a transmute needs to know about it.
#[derive(Clone, Copy)]
struct Repr {
    repr: IntRepr,
    signed: bool,
    bits: u32,
}

/// How a value is represented, at the granularity a transmute cares about.
#[derive(Clone, Copy)]
enum ReprClass {
    Int(Repr),
    Float(u32),
    Bool,
    Char,
    /// A pointer, with whether it is thin.
    Pointer(bool),
    Other,
}

impl ReprClass {
    fn of(table: &rcgjs_object::jir::types::TypeTable, ty: TypeId) -> ReprClass {
        let peeled = table.peeled(ty);
        if let Some((signed, bits)) = table.int_info(peeled) {
            return ReprClass::Int(Repr {
                repr: table.int_repr(peeled),
                signed,
                bits,
            });
        }
        match value::kind(table, peeled) {
            Some(TypeKind::Bool) => ReprClass::Bool,
            Some(TypeKind::Char) => ReprClass::Char,
            Some(TypeKind::Float { bits }) => ReprClass::Float(*bits),
            Some(TypeKind::FnPtr { .. }) => ReprClass::Pointer(true),
            Some(TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. }) => {
                let sized = table.get(*pointee).is_some_and(|d| d.layout.sized);
                ReprClass::Pointer(sized)
            }
            _ => ReprClass::Other,
        }
    }
}
