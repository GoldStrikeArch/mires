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
use rcgjs_object::jir::types::{
    EnumRepr, FieldKey, IntRepr, NominalForm, RepresentationCandidate, TypeId, TypeKind,
};
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
        match self.try_transmute(from_ty, to_ty, value, 0)? {
            Some(expr) => Ok(expr),
            None => Ok(self.zombie(
                "`transmute` between these types is not supported by rustc_codegen_js".to_owned(),
            )),
        }
    }

    /// Recursive transmute helper returning `Ok(None)` instead of emitting a zombie when no
    /// lossless value/structural/POD lowering applies.
    fn try_transmute(
        &self,
        from_ty: TypeId,
        to_ty: TypeId,
        value: Expr,
        depth: usize,
    ) -> Res<Option<Expr>> {
        if from_ty == to_ty {
            return Ok(Some(value));
        }
        if depth >= 16 {
            return Ok(None);
        }
        // Peel the single-field wrappers off the source, then rebuild the ones the destination
        // has. Doing it in that order keeps the two sides independent.
        let (inner_from, inner_value) = self.unwrap_wrappers(from_ty, value);
        let (inner_to, wrappers) = self.target_wrappers(to_ty);
        let Some(scalar) = self.try_transmute_scalar(inner_from, inner_to, inner_value, depth)?
        else {
            return Ok(None);
        };
        Ok(Some(self.rebuild(&wrappers, scalar)))
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
        if let Some(info) = scalar_array_info(table, from_ty) {
            if info.count == 2 && info.elem_bits == 32 {
                if self.is_inline(operand) {
                    if let Some(op) = self.def_of(operand) {
                        if let OpKind::Aggregate { fields, .. } = &op.kind {
                            if fields.len() == 2 {
                                let lo = self
                                    .scalar_elem_to_i32(info.elem_kind, self.expr_of(fields[0])?);
                                let hi = self
                                    .scalar_elem_to_i32(info.elem_kind, self.expr_of(fields[1])?);
                                return Ok((lo, hi));
                            }
                        }
                    }
                }
                let arr = super::i64_pair::materialize(self, stmts, self.expr_of(operand)?);
                let lo = self
                    .scalar_elem_to_i32(info.elem_kind, jsast::index(arr.clone(), jsast::num(0)));
                let hi = self.scalar_elem_to_i32(info.elem_kind, jsast::index(arr, jsast::num(1)));
                return Ok((lo, hi));
            }
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
        if let Some(info) = scalar_array_info(table, to_ty) {
            if info.count == 2 && info.elem_bits == 32 {
                let (lo, hi) = self.pair_of(operand)?;
                let elems = if info.elem_signed {
                    vec![lo, hi]
                } else {
                    vec![jsast::u32_wrap(lo), jsast::u32_wrap(hi)]
                };
                return Ok(jsast::array(elems));
            }
        }
        if value::float_bits(table, table.peeled(to_ty)) == Some(64) {
            let pair = self.pair_of(operand)?;
            return Ok(super::i64_pair::bits_pair_to_f64(pair));
        }
        self.transmute(from_ty, to_ty, self.expr_of(operand)?)
    }

    /// The scalar half of a transmute, once both sides have had their wrappers removed.
    fn try_transmute_scalar(
        &self,
        from: TypeId,
        to: TypeId,
        value: Expr,
        depth: usize,
    ) -> Res<Option<Expr>> {
        let table = self.table;
        // Two types with the same *peeled* type have the same JavaScript value, whatever
        // transparent wrappers stand between them (`JsFacts::peeled`), so the transmute is the
        // identity. v1 asked the same question by peeling as it unwrapped.
        if from == to || table.peeled(from) == table.peeled(to) {
            return Ok(Some(value));
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
                return Ok(Some(value));
            }
        }
        // A zero sized value carries no bits, and is `undefined`.
        if table.is_zst(to) {
            return Ok(Some(jsast::undefined()));
        }
        // `str::as_bytes` and `str::from_utf8_unchecked` are both written as a transmute between
        // two pointers, and neither is one here: a `str` is a JavaScript string.
        if let Some(converted) = self.cast_str_ptr(from, to, value.clone()) {
            return converted.map(Some);
        }

        let from_class = ReprClass::of(table, from);
        let to_class = ReprClass::of(table, to);
        Ok(Some(match (from_class, to_class) {
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
                        // Two trait-object pointers with the same principal trait (differing only
                        // in auto-trait bounds like `Send`/`Sync` or lifetimes, e.g.
                        // `Box<dyn Error> -> Box<dyn Error + Send + Sync>` in `alloc`) share the
                        // exact `{ ptr, meta }` representation.
                        if let (
                            Some(TypeKind::Dynamic { principal: a, .. }),
                            Some(TypeKind::Dynamic { principal: b, .. }),
                        ) = (value::kind(table, from_shape), value::kind(table, to_shape))
                        {
                            if a == b || b.is_none() {
                                return Ok(Some(value));
                            }
                        }
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
                            let raw_out = match self.cast_scalar_reinterp_ptr(
                                from_pointee,
                                to_pointee,
                                raw_in,
                            ) {
                                Ok(converted) => converted,
                                Err(raw_in) => {
                                    self.cast_pointer(from_pointee, to_pointee, raw_in)?
                                }
                            };
                            if depth > 0 && is_zombie_expr(&raw_out) {
                                return Ok(None);
                            }
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
                let address = self.thin_ptr_addr(from, value);
                value::mask(b.repr, b.signed, b.bits, address)
            }
            (ReprClass::Int(a), ReprClass::Pointer(true)) if a.bits == POINTER_BITS => {
                if a.signed {
                    value::mask(a.repr, false, a.bits, value)
                } else {
                    value
                }
            }
            // A thin pointer and a pointer-sized scalar array (`*const () <-> [u8; 4]`, reached
            // when MIR folds `ptr.addr().to_ne_bytes()` in `core::any::TypeId::as_u128`).
            (ReprClass::Pointer(true), ReprClass::Other)
                if scalar_array_info(table, to).is_some_and(|info| {
                    info.count.checked_mul(info.elem_bits) == Some(POINTER_BITS)
                }) =>
            {
                let info = scalar_array_info(table, to).unwrap();
                let addr = jsast::u32_wrap(self.thin_ptr_addr(from, value));
                int_to_array(
                    Repr {
                        repr: IntRepr::Number,
                        signed: false,
                        bits: POINTER_BITS,
                    },
                    info,
                    addr,
                )
            }
            (ReprClass::Other, ReprClass::Pointer(true))
                if scalar_array_info(table, from).is_some_and(|info| {
                    info.count.checked_mul(info.elem_bits) == Some(POINTER_BITS)
                }) =>
            {
                let info = scalar_array_info(table, from).unwrap();
                self.array_to_int(
                    info,
                    Repr {
                        repr: IntRepr::Number,
                        signed: false,
                        bits: POINTER_BITS,
                    },
                    value,
                )
            }
            // `[E; N]` and `uN`/`iN`, little endian (e.g. `[u8; 8] <-> u64`, `[i32; 4] <-> u128`,
            // `[*const (); 4] <-> u128` for `core::any::TypeId`).
            (ReprClass::Other, ReprClass::Int(b))
                if matches!(b.bits, 16 | 32 | 64 | 128)
                    && scalar_array_info(table, from).is_some_and(|info| {
                        info.count.checked_mul(info.elem_bits) == Some(b.bits)
                    }) =>
            {
                let info = scalar_array_info(table, from).unwrap();
                self.array_to_int(info, b, value)
            }
            (ReprClass::Int(a), ReprClass::Other)
                if matches!(a.bits, 16 | 32 | 64 | 128)
                    && scalar_array_info(table, to).is_some_and(|info| {
                        info.count.checked_mul(info.elem_bits) == Some(a.bits)
                    }) =>
            {
                let info = scalar_array_info(table, to).unwrap();
                int_to_array(a, info, value)
            }
            // Same-length 32-bit scalar/thin-pointer arrays (e.g. `TypeId`'s `[*const (); 4]`
            // <-> `[u32; 4]` / `[usize; 4]` / `[i32; 4]`), or a 128-bit scalar/thin-pointer array
            // <-> `(u64, u64)` / `(i64, i64)`.
            (ReprClass::Other, ReprClass::Other)
                if self.transmute_scalar_composites(from, to, &value).is_some() =>
            {
                self.transmute_scalar_composites(from, to, &value)
                    .expect("checked Some above")
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
            _ => {
                if let Some(decoded) = self.niche_from_scalar(from, to, value.clone(), depth)? {
                    return Ok(Some(decoded));
                }
                if let Some(encoded) = self.niche_to_scalar(from, to, value.clone(), depth)? {
                    return Ok(Some(encoded));
                }
                if let Some(mapped) =
                    self.transmute_structural_aggregate(from, to, value.clone(), depth)?
                {
                    return Ok(Some(mapped));
                }
                return Ok(super::pod_codec::pod_transmute(self, from, to, value));
            }
        }))
    }

    /// Structural field-aligned transmute for equal-size non-native structs, tuples, and
    /// same-length arrays whose fields/elements transmute recursively at matching byte offsets
    /// (e.g. `Box<MaybeUninit<Node<T>>, &Global> -> Box<Node<T>, &Global>` in `LinkedList`).
    fn transmute_structural_aggregate(
        &self,
        from: TypeId,
        to: TypeId,
        value: Expr,
        depth: usize,
    ) -> Res<Option<Expr>> {
        let table = self.table;
        let from_p = table.peeled(from);
        let to_p = table.peeled(to);
        let (Some(from_size), Some(to_size)) =
            (value::size_of(table, from_p), value::size_of(table, to_p))
        else {
            return Ok(None);
        };
        if from_size == 0 || from_size != to_size {
            return Ok(None);
        }
        let (Some(from_desc), Some(to_desc)) = (table.get(from_p), table.get(to_p)) else {
            return Ok(None);
        };
        let is_native_carrier = |c: RepresentationCandidate| {
            matches!(
                c,
                RepresentationCandidate::Vec
                    | RepresentationCandidate::String
                    | RepresentationCandidate::BTreeMap
            )
        };
        if is_native_carrier(from_desc.meta.candidate) || is_native_carrier(to_desc.meta.candidate)
        {
            return Ok(None);
        }

        // Same-length array whose elements transmute recursively (e.g. `[Box<MaybeUninit<T>, &A>; N]`).
        if let (
            TypeKind::Array {
                element: fe,
                length: fl,
            },
            TypeKind::Array {
                element: te,
                length: tl,
            },
        ) = (&from_desc.kind, &to_desc.kind)
        {
            if fl == tl && *fl > 0 {
                let ep = format!("$te{depth}");
                let eid = jsast::id(ep.clone());
                if let Some(mapped) = self.try_transmute(*fe, *te, eid.clone(), depth + 1)? {
                    if mapped == eid {
                        return Ok(Some(value));
                    }
                    return Ok(Some(jsast::method_call(
                        value,
                        "map",
                        vec![jsast::arrow(vec![ep], mapped)],
                    )));
                }
            }
            return Ok(None);
        }

        enum AggShape<'a> {
            Struct {
                fields: &'a [rcgjs_object::jir::types::FieldDescriptor],
            },
            Tuple {
                arity: usize,
                fields: &'a [rcgjs_object::jir::types::FieldDescriptor],
            },
        }
        fn shape_of(desc: &rcgjs_object::jir::types::TypeDescriptor) -> Option<AggShape<'_>> {
            match &desc.kind {
                TypeKind::Adt {
                    form: NominalForm::Struct,
                    variants,
                    ..
                } => {
                    let [v] = variants.as_slice() else {
                        return None;
                    };
                    Some(AggShape::Struct { fields: &v.fields })
                }
                TypeKind::Tuple(elems) => {
                    if desc.js.fields.len() != elems.len() {
                        return None;
                    }
                    Some(AggShape::Tuple {
                        arity: elems.len(),
                        fields: &desc.js.fields,
                    })
                }
                _ => None,
            }
        }
        let (Some(from_shape), Some(to_shape)) = (shape_of(from_desc), shape_of(to_desc)) else {
            return Ok(None);
        };
        let (from_fields, to_fields) = match (&from_shape, &to_shape) {
            (
                AggShape::Struct { fields: ff } | AggShape::Tuple { fields: ff, .. },
                AggShape::Struct { fields: tf } | AggShape::Tuple { fields: tf, .. },
            ) => (*ff, *tf),
        };
        let mut live_from: Vec<&rcgjs_object::jir::types::FieldDescriptor> =
            from_fields.iter().filter(|f| !table.is_zst(f.ty)).collect();
        let mut live_to: Vec<&rcgjs_object::jir::types::FieldDescriptor> =
            to_fields.iter().filter(|f| !table.is_zst(f.ty)).collect();
        if live_from.is_empty() || live_from.len() != live_to.len() || live_from.len() > 16 {
            return Ok(None);
        }
        live_from.sort_by_key(|f| f.offset);
        live_to.sort_by_key(|f| f.offset);
        for (ff, tf) in live_from.iter().zip(&live_to) {
            if ff.offset != tf.offset {
                return Ok(None);
            }
            let (Some(fs), Some(ts)) = (value::size_of(table, ff.ty), value::size_of(table, tf.ty))
            else {
                return Ok(None);
            };
            if fs != ts {
                return Ok(None);
            }
        }

        let param = format!("$ta{depth}");
        let base = jsast::id(param.clone());
        let mut all_identity = true;
        let mut converted: Vec<(FieldKey, Expr)> = Vec::with_capacity(live_to.len());
        for (ff, tf) in live_from.iter().zip(&live_to) {
            let read = value::field_expr(self.interns, base.clone(), ff.key);
            let Some(next) = self.try_transmute(ff.ty, tf.ty, read.clone(), depth + 1)? else {
                return Ok(None);
            };
            if next != read {
                all_identity = false;
            }
            converted.push((tf.key, next));
        }

        // If container shapes and keys match identically and every field transmute is identity,
        // the aggregate value itself is unchanged (zero runtime cost).
        if all_identity {
            let same_container = match (&from_shape, &to_shape) {
                (AggShape::Struct { .. }, AggShape::Struct { .. }) => {
                    from_fields.len() == to_fields.len()
                        && live_from
                            .iter()
                            .zip(&live_to)
                            .all(|(ff, tf)| ff.key == tf.key)
                        && from_fields
                            .iter()
                            .filter(|f| table.is_zst(f.ty))
                            .all(|f| to_fields.iter().any(|t| t.key == f.key))
                }
                (AggShape::Tuple { arity: fa, .. }, AggShape::Tuple { arity: ta, .. }) => {
                    fa == ta
                        && live_from
                            .iter()
                            .zip(&live_to)
                            .all(|(ff, tf)| ff.key == tf.key)
                }
                _ => false,
            };
            if same_container {
                return Ok(Some(value));
            }
        }

        let built = match to_shape {
            AggShape::Tuple { arity, .. } => {
                let mut elems = vec![jsast::undefined(); arity];
                for (key, expr) in converted {
                    let FieldKey::Index(idx) = key else {
                        return Ok(None);
                    };
                    if (idx as usize) >= elems.len() {
                        return Ok(None);
                    }
                    elems[idx as usize] = expr;
                }
                jsast::array(elems)
            }
            AggShape::Struct { fields } => {
                let mut entries = Vec::with_capacity(fields.len());
                let mut proto_val = None;
                for (key, expr) in converted {
                    let Some(name) = value::key_name(self.interns, key) else {
                        return Ok(None);
                    };
                    if name == "__proto__" {
                        proto_val = Some(expr);
                    } else {
                        entries.push((name, expr));
                    }
                }
                for f in fields.iter().filter(|f| table.is_zst(f.ty)) {
                    let Some(name) = value::key_name(self.interns, f.key) else {
                        continue;
                    };
                    if name == "__proto__" {
                        proto_val = Some(jsast::undefined());
                    } else {
                        entries.push((name, jsast::undefined()));
                    }
                }
                let obj = jsast::object(entries);
                if let Some(pv) = proto_val {
                    let oname = format!("$to{depth}");
                    return Ok(Some(jsast::call(
                        jsast::arrow_block(
                            vec![param],
                            vec![
                                jsast::const_(oname.clone(), obj),
                                jsast::expr_stmt(value::define_proto_prop(
                                    jsast::id(oname.clone()),
                                    pv,
                                )),
                                jsast::ret(jsast::id(oname)),
                            ],
                        ),
                        vec![value],
                    )));
                }
                obj
            }
        };
        Ok(Some(jsast::call(
            jsast::arrow(vec![param], built),
            vec![value],
        )))
    }

    /// Converts a thin pointer expression of type `from` into its runtime address expression.
    fn thin_ptr_addr(&self, from: TypeId, value: Expr) -> Expr {
        let table = self.table;
        match value::pointee_of(table, from) {
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
        }
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
    fn niche_from_scalar(
        &self,
        from: TypeId,
        to: TypeId,
        value: Expr,
        depth: usize,
    ) -> Res<Option<Expr>> {
        let table = self.table;
        let Some(bits) = super::pod_codec::niche_scalar_bits(table, from) else {
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
            let Some(niche_lit) = super::pod_codec::niche_scalar_literal(table, from, raw_bits)
            else {
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
        let Some(filled_field) = self.try_transmute(from, field.ty, bound.clone(), depth + 1)?
        else {
            return Ok(None);
        };
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
    fn niche_to_scalar(
        &self,
        from: TypeId,
        to: TypeId,
        value: Expr,
        depth: usize,
    ) -> Res<Option<Expr>> {
        let table = self.table;
        let Some(bits) = super::pod_codec::niche_scalar_bits(table, to) else {
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
            let Some(niche_lit) = super::pod_codec::niche_scalar_literal(table, to, raw_bits)
            else {
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
        let payload_input = if repr.is_null_or_sentinel_niche() {
            bound.clone()
        } else {
            value::field_expr(self.interns, bound.clone(), field.key)
        };
        let Some(payload_bits) = self.try_transmute(field.ty, to, payload_input, depth + 1)? else {
            return Ok(None);
        };
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

/// Detects whether an expression is an aborting zombie emitted by `JsCx::zombie`.
fn is_zombie_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Call(callee, args) if args.is_empty() => matches!(
            callee.as_ref(),
            Expr::Arrow(_, rcgjs_object::jsast::ArrowBody::Block(stmts))
                if stmts.iter().any(|s| matches!(s, Stmt::Throw(_)))
        ),
        _ => false,
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

/// Element kind of a scalar-array transmute lane.
#[derive(Clone, Copy)]
enum ScalarElemKind {
    Int,
    Bool,
    ThinPtr {
        pointee: Option<TypeId>,
        is_ref: bool,
    },
}

/// Element width, signedness, and length of `[E; N]` when `E` is a plain scalar integer, `bool`,
/// `char`, fieldless integer-`repr` enum, or thin pointer.
#[derive(Clone, Copy)]
struct ScalarArrayInfo {
    count: u32,
    elem_bits: u32,
    elem_signed: bool,
    elem_kind: ScalarElemKind,
}

/// The length of `ty` if it is an array of bytes, and `None` otherwise.
fn byte_array_len(table: &rcgjs_object::jir::types::TypeTable, ty: TypeId) -> Option<u64> {
    let TypeKind::Array { element, length } = value::kind(table, table.peeled(ty))? else {
        return None;
    };
    value::is_u8(table, table.peeled(*element)).then_some(*length)
}

/// Element layout of `[E; N]` for scalar array <-> integer transmutations.
fn scalar_array_info(
    table: &rcgjs_object::jir::types::TypeTable,
    ty: TypeId,
) -> Option<ScalarArrayInfo> {
    let TypeKind::Array { element, length } = value::kind(table, table.peeled(ty))? else {
        return None;
    };
    let count = u32::try_from(*length).ok()?;
    if count == 0 {
        return None;
    }
    let elem = table.peeled(*element);
    if let Some((signed, bits)) = table.int_info(elem) {
        return matches!(bits, 8 | 16 | 32 | 64).then_some(ScalarArrayInfo {
            count,
            elem_bits: bits,
            elem_signed: signed,
            elem_kind: ScalarElemKind::Int,
        });
    }
    match value::kind(table, elem)? {
        TypeKind::Bool => Some(ScalarArrayInfo {
            count,
            elem_bits: 8,
            elem_signed: false,
            elem_kind: ScalarElemKind::Bool,
        }),
        TypeKind::Char => Some(ScalarArrayInfo {
            count,
            elem_bits: 32,
            elem_signed: false,
            elem_kind: ScalarElemKind::Int,
        }),
        TypeKind::FnPtr { .. } => Some(ScalarArrayInfo {
            count,
            elem_bits: POINTER_BITS,
            elem_signed: false,
            elem_kind: ScalarElemKind::ThinPtr {
                pointee: None,
                is_ref: false,
            },
        }),
        TypeKind::RawPointer { pointee, .. }
            if table.get(*pointee).is_some_and(|d| d.layout.sized) =>
        {
            Some(ScalarArrayInfo {
                count,
                elem_bits: POINTER_BITS,
                elem_signed: false,
                elem_kind: ScalarElemKind::ThinPtr {
                    pointee: Some(*pointee),
                    is_ref: false,
                },
            })
        }
        TypeKind::Reference { pointee, .. }
            if table.get(*pointee).is_some_and(|d| d.layout.sized) =>
        {
            Some(ScalarArrayInfo {
                count,
                elem_bits: POINTER_BITS,
                elem_signed: false,
                elem_kind: ScalarElemKind::ThinPtr {
                    pointee: Some(*pointee),
                    is_ref: true,
                },
            })
        }
        _ => direct_tag(table, elem).and_then(|(bits, signed)| {
            matches!(bits, 8 | 16 | 32 | 64).then_some(ScalarArrayInfo {
                count,
                elem_bits: bits,
                elem_signed: signed,
                elem_kind: ScalarElemKind::Int,
            })
        }),
    }
}

/// Pair of 64-bit integer representations if `ty` is a 2-tuple `(i64|u64, i64|u64)`.
fn int64_pair_tuple(
    table: &rcgjs_object::jir::types::TypeTable,
    ty: TypeId,
) -> Option<(bool, bool)> {
    let TypeKind::Tuple(elems) = value::kind(table, table.peeled(ty))? else {
        return None;
    };
    let [first, second] = elems.as_slice() else {
        return None;
    };
    let (s0, b0) = table.int_info(table.peeled(*first))?;
    let (s1, b1) = table.int_info(table.peeled(*second))?;
    (b0 == 64 && b1 == 64).then_some((s0, s1))
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

impl JsCx<'_> {
    /// Exposes thin-pointer address extraction to [`super::pod_codec`].
    pub(super) fn pod_thin_ptr_addr(
        &self,
        pointee: Option<TypeId>,
        is_ref: bool,
        read: Expr,
    ) -> Expr {
        self.thin_ptr_lane_addr(pointee, is_ref, read)
    }

    /// Extracts the unsigned 32-bit address of a thin-pointer lane in a scalar array.
    fn thin_ptr_lane_addr(&self, pointee: Option<TypeId>, is_ref: bool, read: Expr) -> Expr {
        let address = match pointee {
            Some(pointee) => {
                let raw = if is_ref {
                    self.ref_value_to_raw(pointee, read)
                } else {
                    read
                };
                self.ptr_addr(pointee, raw)
            }
            None => jsast::rt_call("addr", vec![read, jsast::num(1.0)]),
        };
        jsast::u32_wrap(address)
    }

    fn scalar_elem_to_i32(&self, kind: ScalarElemKind, read: Expr) -> Expr {
        match kind {
            ScalarElemKind::Int | ScalarElemKind::Bool => jsast::i32_wrap(read),
            ScalarElemKind::ThinPtr { pointee, is_ref } => {
                jsast::i32_wrap(self.thin_ptr_lane_addr(pointee, is_ref, read))
            }
        }
    }

    /// `[E; N]` to the integer those elements spell, little endian.
    fn array_to_int(&self, arr: ScalarArrayInfo, int: Repr, value: Expr) -> Expr {
        let bound = jsast::id(BYTES_PARAM);
        let big = int.repr == IntRepr::BigInt;
        let lane = |index: u32| {
            let read = jsast::index(bound.clone(), jsast::num(index));
            let unsigned = match arr.elem_kind {
                ScalarElemKind::Bool => jsast::cond(read, jsast::num(1), jsast::num(0)),
                ScalarElemKind::ThinPtr { pointee, is_ref } => {
                    self.thin_ptr_lane_addr(pointee, is_ref, read)
                }
                ScalarElemKind::Int if arr.elem_bits == 64 => {
                    return jsast::call(
                        jsast::member(jsast::id("BigInt"), "asUintN"),
                        vec![jsast::num(64), read],
                    );
                }
                ScalarElemKind::Int if arr.elem_signed => match arr.elem_bits {
                    8 => jsast::binary(BinOp::BitAnd, read, jsast::num(255)),
                    16 => jsast::binary(BinOp::BitAnd, read, jsast::num(65535)),
                    32 => jsast::u32_wrap(read),
                    _ => unreachable!(),
                },
                ScalarElemKind::Int => read,
            };
            if big {
                jsast::call(jsast::id("BigInt"), vec![unsigned])
            } else {
                unsigned
            }
        };
        let shift = |amount: u32| {
            if big {
                jsast::bigint(amount)
            } else {
                jsast::num(amount)
            }
        };
        let mut combined = lane(0);
        for index in 1..arr.count {
            let shifted = jsast::binary(BinOp::Shl, lane(index), shift(index * arr.elem_bits));
            combined = jsast::binary(BinOp::BitOr, combined, shifted);
        }
        let combined = value::mask(int.repr, int.signed, int.bits, combined);
        jsast::call(
            jsast::arrow(vec![BYTES_PARAM.to_owned()], combined),
            vec![value],
        )
    }

    /// Transmutations between scalar/thin-pointer arrays and matching-width composite layouts:
    /// `[E1; N] <-> [E2; N]` (for 32-bit integer/pointer elements) and 128-bit `[E; N]` <->
    /// `(u64|i64, u64|i64)`.
    fn transmute_scalar_composites(&self, from: TypeId, to: TypeId, value: &Expr) -> Option<Expr> {
        let table = self.table;
        if let (Some(from_arr), Some(to_arr)) =
            (scalar_array_info(table, from), scalar_array_info(table, to))
        {
            if from_arr.count == to_arr.count
                && from_arr.elem_bits == POINTER_BITS
                && to_arr.elem_bits == POINTER_BITS
            {
                let bound = jsast::id(BYTES_PARAM);
                let elems = (0..from_arr.count)
                    .map(|index| {
                        let read = jsast::index(bound.clone(), jsast::num(index));
                        let raw = match from_arr.elem_kind {
                            ScalarElemKind::ThinPtr { pointee, is_ref } => {
                                self.thin_ptr_lane_addr(pointee, is_ref, read)
                            }
                            ScalarElemKind::Int | ScalarElemKind::Bool => read,
                        };
                        match to_arr.elem_kind {
                            ScalarElemKind::ThinPtr { .. } => {
                                if from_arr.elem_signed {
                                    jsast::u32_wrap(raw)
                                } else {
                                    raw
                                }
                            }
                            ScalarElemKind::Int => {
                                value::mask(IntRepr::Number, to_arr.elem_signed, 32, raw)
                            }
                            ScalarElemKind::Bool => {
                                jsast::binary(BinOp::StrictNe, raw, jsast::num(0))
                            }
                        }
                    })
                    .collect();
                return Some(jsast::call(
                    jsast::arrow(vec![BYTES_PARAM.to_owned()], jsast::array(elems)),
                    vec![value.clone()],
                ));
            }
        }
        let u128_repr = Repr {
            repr: IntRepr::BigInt,
            signed: false,
            bits: 128,
        };
        if let (Some(from_arr), Some((s0, s1))) =
            (scalar_array_info(table, from), int64_pair_tuple(table, to))
        {
            if from_arr.count.checked_mul(from_arr.elem_bits) == Some(128) {
                let wide = self.array_to_int(from_arr, u128_repr, value.clone());
                let bound = jsast::id(BYTES_PARAM);
                let lo = value::mask(IntRepr::BigInt, s0, 64, bound.clone());
                let hi = value::mask(
                    IntRepr::BigInt,
                    s1,
                    64,
                    jsast::binary(BinOp::Shr, bound, jsast::bigint(64)),
                );
                return Some(jsast::call(
                    jsast::arrow(vec![BYTES_PARAM.to_owned()], jsast::array(vec![lo, hi])),
                    vec![wide],
                ));
            }
        }
        if let (Some(_), Some(to_arr)) =
            (int64_pair_tuple(table, from), scalar_array_info(table, to))
        {
            if to_arr.count.checked_mul(to_arr.elem_bits) == Some(128) {
                let bound = jsast::id(BYTES_PARAM);
                let lo = jsast::call(
                    jsast::member(jsast::id("BigInt"), "asUintN"),
                    vec![jsast::num(64), jsast::index(bound.clone(), jsast::num(0))],
                );
                let hi = jsast::call(
                    jsast::member(jsast::id("BigInt"), "asUintN"),
                    vec![jsast::num(64), jsast::index(bound, jsast::num(1))],
                );
                let combined = jsast::binary(
                    BinOp::BitOr,
                    lo,
                    jsast::binary(BinOp::Shl, hi, jsast::bigint(64)),
                );
                let wide = jsast::call(
                    jsast::arrow(vec![BYTES_PARAM.to_owned()], combined),
                    vec![value.clone()],
                );
                return Some(int_to_array(u128_repr, to_arr, wide));
            }
        }
        None
    }
}

/// The other direction: an integer unpacked into `[E; N]`, least significant first.
fn int_to_array(int: Repr, arr: ScalarArrayInfo, value: Expr) -> Expr {
    let bound = jsast::id(BYTES_PARAM);
    let big = int.repr == IntRepr::BigInt;
    let elems = (0..arr.count)
        .map(|index| {
            let shifted = if index == 0 {
                bound.clone()
            } else if big {
                jsast::binary(
                    BinOp::Shr,
                    bound.clone(),
                    jsast::bigint(index * arr.elem_bits),
                )
            } else {
                jsast::binary(
                    BinOp::UShr,
                    bound.clone(),
                    jsast::num(index * arr.elem_bits),
                )
            };
            if arr.elem_bits == 64 {
                return value::mask(IntRepr::BigInt, arr.elem_signed, 64, shifted);
            }
            let raw = if big {
                let masked = jsast::call(
                    jsast::member(jsast::id("BigInt"), "asUintN"),
                    vec![jsast::num(arr.elem_bits), shifted],
                );
                jsast::call(jsast::id("Number"), vec![masked])
            } else if arr.elem_bits < 32 {
                let bit_mask = (1u32 << arr.elem_bits) - 1;
                jsast::binary(BinOp::BitAnd, shifted, jsast::num(bit_mask))
            } else {
                shifted
            };
            if matches!(arr.elem_kind, ScalarElemKind::Bool) {
                jsast::binary(BinOp::StrictNe, raw, jsast::num(0))
            } else if big && !arr.elem_signed {
                raw
            } else {
                value::mask(IntRepr::Number, arr.elem_signed, arr.elem_bits, raw)
            }
        })
        .collect();
    jsast::call(
        jsast::arrow(vec![BYTES_PARAM.to_owned()], jsast::array(elems)),
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

#[cfg(test)]
mod type_id_tests {
    use rcgjs_object::item::JsName;
    use rcgjs_object::jir::interns::Interns;
    use rcgjs_object::jir::repr::{Func, FuncBuilder, OpKind, RBlockId, RTerm, ValueId};
    use rcgjs_object::jir::semantic::{
        AbiSignature, Body, BodyKind, BodySemantic, CastKind, ConstTree, DefinitionId,
        GlobalReference, ItemId, Signature,
    };
    use rcgjs_object::jir::summary::{Availability, ItemClass, ItemSummary, SemanticKey};
    use rcgjs_object::jir::types::{
        FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts, NominalForm,
        PointerForm, TypeDescriptor, TypeId, TypeKind, TypeMeta, VariantDescriptor,
    };
    use rcgjs_object::jsast::{self, Style};
    use rcgjs_object::object::{BlobRef, ItemKind, Linkage};
    use rcgjs_object::opts::{EcmaTarget, Opts};
    use structurizer::Region;

    use crate::lower::js::{function, host};
    use crate::lower::plan::{BlockPayload, OpRef, Plan, NO_ORIGIN};
    use crate::lower::LowerInput;

    const UNIT: TypeId = TypeId(1);
    const U32: TypeId = TypeId(2);
    const I32: TypeId = TypeId(3);
    const U64: TypeId = TypeId(4);
    const U128: TypeId = TypeId(5);
    const RAW_UNIT: TypeId = TypeId(6);
    const PTR_ARRAY: TypeId = TypeId(7);
    const TYPE_ID: TypeId = TypeId(8);
    const U32_ARRAY: TypeId = TypeId(9);
    const U64_PAIR: TypeId = TypeId(10);
    const RAW_PTR_UNIT: TypeId = TypeId(11);
    const SLICE_PTR_UNIT: TypeId = TypeId(12);
    const RAW_SLICE_PTR_UNIT: TypeId = TypeId(13);
    const RAW_U64: TypeId = TypeId(14);
    const U8: TypeId = TypeId(15);
    const U8_ARRAY_4: TypeId = TypeId(16);
    const DYN_ERR: TypeId = TypeId(17);
    const DYN_ERR_SEND_SYNC: TypeId = TypeId(18);
    const RAW_DYN_ERR: TypeId = TypeId(19);
    const RAW_DYN_ERR_SEND_SYNC: TypeId = TypeId(20);

    fn base_desc(id: TypeId, kind: TypeKind, size: u64) -> TypeDescriptor {
        TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                size: Some(size),
                unsized_prefix: None,
                align: Some(size.clamp(1, 4)),
                sized: true,
                zst: size == 0,
                one_zst: size == 0,
                needs_drop: false,
                has_own_drop: false,
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: size == 0,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: id,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        }
    }

    fn int_desc(id: TypeId, signed: bool, bits: u32) -> TypeDescriptor {
        let mut d = base_desc(id, TypeKind::Integer { signed, bits }, u64::from(bits / 8));
        d.js.int = Some(IntFacts {
            signed,
            bits,
            repr: if bits > 32 {
                IntRepr::BigInt
            } else {
                IntRepr::Number
            },
        });
        d
    }

    fn raw_ptr_desc(id: TypeId, pointee: TypeId, form: PointerForm) -> TypeDescriptor {
        let mut d = base_desc(
            id,
            TypeKind::RawPointer {
                pointee,
                mutable: false,
            },
            if form == PointerForm::Thin { 4 } else { 8 },
        );
        d.js.pointer_form = Some(form);
        d
    }

    fn type_id_world() -> Interns {
        let mut interns = Interns::new();
        let data_key = interns.intern_str("data");
        let type_id_name = interns.intern_str("TypeId");

        interns.intern_type(base_desc(UNIT, TypeKind::Tuple(Vec::new()), 0));
        interns.intern_type(int_desc(U32, false, 32));
        interns.intern_type(int_desc(I32, true, 32));
        interns.intern_type(int_desc(U64, false, 64));
        interns.intern_type(int_desc(U128, false, 128));
        interns.intern_type(raw_ptr_desc(RAW_UNIT, UNIT, PointerForm::Thin));

        let mut ptr_arr = base_desc(
            PTR_ARRAY,
            TypeKind::Array {
                element: RAW_UNIT,
                length: 4,
            },
            16,
        );
        ptr_arr.js.is_indirect = true;
        ptr_arr.js.needs_clone = true;
        ptr_arr.js.fields = vec![FieldDescriptor {
            key: FieldKey::Index(0),
            ty: RAW_UNIT,
            offset: 0,
        }];
        interns.intern_type(ptr_arr);

        let mut tid = base_desc(
            TYPE_ID,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 1,
                    local_hash: 42,
                },
                args: Vec::new(),
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: type_id_name,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(data_key),
                        ty: PTR_ARRAY,
                        offset: 0,
                    }],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            16,
        );
        tid.js.is_indirect = true;
        tid.js.needs_clone = true;
        tid.js.fields = vec![FieldDescriptor {
            key: FieldKey::Name(data_key),
            ty: PTR_ARRAY,
            offset: 0,
        }];
        interns.intern_type(tid);

        let mut u32_arr = base_desc(
            U32_ARRAY,
            TypeKind::Array {
                element: U32,
                length: 4,
            },
            16,
        );
        u32_arr.js.is_indirect = true;
        u32_arr.js.needs_clone = true;
        interns.intern_type(u32_arr);

        let mut u64_pair = base_desc(U64_PAIR, TypeKind::Tuple(vec![U64, U64]), 16);
        u64_pair.js.is_indirect = true;
        u64_pair.js.needs_clone = true;
        interns.intern_type(u64_pair);

        interns.intern_type(raw_ptr_desc(RAW_PTR_UNIT, RAW_UNIT, PointerForm::Thin));
        let mut slice_ptr = base_desc(SLICE_PTR_UNIT, TypeKind::Slice(RAW_UNIT), 0);
        slice_ptr.layout.sized = false;
        slice_ptr.layout.size = None;
        slice_ptr.layout.zst = false;
        slice_ptr.js.is_indirect = true;
        interns.intern_type(slice_ptr);
        interns.intern_type(raw_ptr_desc(
            RAW_SLICE_PTR_UNIT,
            SLICE_PTR_UNIT,
            PointerForm::FatSlice,
        ));
        interns.intern_type(raw_ptr_desc(RAW_U64, U64, PointerForm::Thin));
        let mut u8_desc = int_desc(U8, false, 8);
        u8_desc.js.byte_element = true;
        interns.intern_type(u8_desc);
        let mut u8_arr = base_desc(
            U8_ARRAY_4,
            TypeKind::Array {
                element: U8,
                length: 4,
            },
            4,
        );
        u8_arr.js.is_indirect = true;
        u8_arr.js.needs_clone = true;
        interns.intern_type(u8_arr);

        let err_principal = DefinitionId {
            crate_id: 1,
            local_hash: 99,
        };
        let send_auto = DefinitionId {
            crate_id: 1,
            local_hash: 100,
        };
        let sync_auto = DefinitionId {
            crate_id: 1,
            local_hash: 101,
        };
        let mut dyn_err = base_desc(
            DYN_ERR,
            TypeKind::Dynamic {
                principal: Some(err_principal),
                auto: Vec::new(),
            },
            0,
        );
        dyn_err.layout.sized = false;
        dyn_err.layout.size = None;
        dyn_err.layout.zst = false;
        dyn_err.js.is_indirect = true;
        interns.intern_type(dyn_err);

        let mut dyn_err_ss = base_desc(
            DYN_ERR_SEND_SYNC,
            TypeKind::Dynamic {
                principal: Some(err_principal),
                auto: vec![send_auto, sync_auto],
            },
            0,
        );
        dyn_err_ss.layout.sized = false;
        dyn_err_ss.layout.size = None;
        dyn_err_ss.layout.zst = false;
        dyn_err_ss.js.is_indirect = true;
        interns.intern_type(dyn_err_ss);

        interns.intern_type(raw_ptr_desc(RAW_DYN_ERR, DYN_ERR, PointerForm::FatDyn));
        interns.intern_type(raw_ptr_desc(
            RAW_DYN_ERR_SEND_SYNC,
            DYN_ERR_SEND_SYNC,
            PointerForm::FatDyn,
        ));

        interns
    }

    fn lower_fn(
        interns: &Interns,
        name: &str,
        func: &Func,
        param_names: &[(u32, &str)],
        target: EcmaTarget,
    ) -> String {
        let mut plan = Plan::default();
        for &(id, pname) in param_names {
            plan.names.insert(ValueId(id), pname.to_owned());
        }
        for index in 0..func.values.len() {
            let v = ValueId(index as u32);
            if let std::collections::btree_map::Entry::Vacant(entry) = plan.names.entry(v) {
                entry.insert(format!("_{index}"));
                plan.inline.insert(v);
            }
        }
        let body = Body {
            item: func.item,
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: func.sig.clone(),
            locals: Vec::new(),
            blocks: Vec::new(),
        };
        let summary = ItemSummary {
            key: SemanticKey {
                link_name: name.to_owned(),
                item: Some(func.item),
            },
            body_hash: None,
            semantic_hash: None,
            availability: Availability::Available,
            deps: Vec::new(),
            abi: None,
            physical: None,
            kind: ItemKind::Fn,
            linkage: Linkage::internal(),
            order: None,
            dts: false,
            fallback: BlobRef {
                offset: 0,
                clen: 0,
                ulen: 0,
            },
            body: None,
            origin: None,
            estimated_bytes: 0,
            class: ItemClass::default(),
        };
        let opts = Opts::default();
        let js_name = JsName::new(name.to_owned());
        let region = Region::Seq(vec![
            Region::Block(BlockPayload {
                block: Some(RBlockId(0)),
                ops: (0..func.blocks[0].ops.len())
                    .map(|index| OpRef {
                        block: RBlockId(0),
                        index: index as u32,
                    })
                    .collect(),
                edge: None,
            }),
            Region::Return,
        ]);
        let input = LowerInput {
            name: &js_name,
            summary: &summary,
            body: &body,
            origins: None,
            interns,
            table: &interns.types,
            opts: &opts,
            items: host::EMPTY_ITEMS,
            drop_glue: host::EMPTY_DROP_GLUE,
            vtables: &super::EMPTY_VTABLES,
            spills: &[],
            abi: None,
            final_lowering: false,
            call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
            bigints: None,
            byte_views: crate::analysis::byte_views::ByteViewPlan::default(),
            heap: None,
            nested_clone: crate::lower::NestedClone::default(),
        };
        let lowered = function::lower(&input, func, &plan, region).expect("lowering failed");
        assert!(
            lowered.zombies.is_empty(),
            "unexpected zombies: {:?}",
            lowered.zombies
        );
        jsast::stmt_to_string_styled_for_target(&lowered.decl, Style::Readable, target)
    }

    fn sig(arguments: Vec<TypeId>, result: TypeId) -> Signature {
        Signature {
            abi: AbiSignature {
                parameters: arguments.len() as u16,
                spread: None,
                caller_location: false,
                inline_never: false,
                inline_hint: false,
                inline_always: false,
            },
            arguments,
            result,
        }
    }

    #[test]
    fn type_id_const_lowers_global_type_chunks_to_exact_u32_offsets() {
        let interns = type_id_world();
        let e = FuncBuilder::ENTRY;
        let item = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 1,
            },
            instantiation: 1,
        };
        let mut b = FuncBuilder::new(item, sig(vec![], TYPE_ID));
        let tid_const = ConstTree::Aggregate(vec![ConstTree::Aggregate(vec![
            ConstTree::Global {
                target: GlobalReference::Type(U32),
                offset: 0x144b_08f1,
            },
            ConstTree::Global {
                target: GlobalReference::Type(U32),
                offset: 0x85a8_9e43,
            },
            ConstTree::Global {
                target: GlobalReference::Type(U32),
                offset: 0xb401_5888,
            },
            ConstTree::Global {
                target: GlobalReference::Type(U32),
                offset: 0x79f1_9461,
            },
        ])]);
        let v = b.op(e, OpKind::Const(tid_const), TYPE_ID, NO_ORIGIN);
        b.terminate(e, RTerm::Return(Some(v)));
        let func = b.finish();
        let js = lower_fn(&interns, "tid_u32", &func, &[], EcmaTarget::Es2018);
        assert_eq!(
            js,
            "function tid_u32() {\n  return { data: [340461809, 2242420291, 3019987080, 2045875297] };\n}\n"
        );
    }

    #[test]
    fn transmute_type_id_to_u128_is_es2018_safe_and_roundtrips() {
        let interns = type_id_world();
        let e = FuncBuilder::ENTRY;
        let item = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 2,
            },
            instantiation: 2,
        };
        let mut b = FuncBuilder::new(item, sig(vec![TYPE_ID], U128));
        let p = b.param(TYPE_ID, NO_ORIGIN);
        let out = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: p,
                from: TYPE_ID,
                to: U128,
            },
            U128,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(Some(out)));
        let func = b.finish();
        let js_es2018 = lower_fn(
            &interns,
            "tid_to_u128",
            &func,
            &[(0, "a")],
            EcmaTarget::Es2018,
        );
        assert!(
            js_es2018.contains("BigInt(\"32\")")
                && js_es2018.contains("BigInt(\"64\")")
                && js_es2018.contains("BigInt(\"96\")"),
            "expected ES2018 BigInt shifts, got:\n{js_es2018}"
        );
        assert!(
            !js_es2018.contains("32n") && !js_es2018.contains("64n") && !js_es2018.contains("96n"),
            "bare BigInt literal leaked into ES2018 output:\n{js_es2018}"
        );

        let mut b2 = FuncBuilder::new(item, sig(vec![U128], TYPE_ID));
        let p2 = b2.param(U128, NO_ORIGIN);
        let out2 = b2.op(
            e,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: p2,
                from: U128,
                to: TYPE_ID,
            },
            TYPE_ID,
            NO_ORIGIN,
        );
        b2.terminate(e, RTerm::Return(Some(out2)));
        let func2 = b2.finish();
        let js2 = lower_fn(
            &interns,
            "u128_to_tid",
            &func2,
            &[(0, "bits")],
            EcmaTarget::Es2018,
        );
        assert!(
            js2.contains("data:") && js2.contains("BigInt.asUintN(32,"),
            "expected rebuilt TypeId struct from u128, got:\n{js2}"
        );
    }

    #[test]
    fn transmute_type_id_to_u32_array_and_u64_pair() {
        let interns = type_id_world();
        let e = FuncBuilder::ENTRY;
        let item = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 3,
            },
            instantiation: 3,
        };
        let mut b = FuncBuilder::new(item, sig(vec![TYPE_ID], U32_ARRAY));
        let p = b.param(TYPE_ID, NO_ORIGIN);
        let out = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: p,
                from: TYPE_ID,
                to: U32_ARRAY,
            },
            U32_ARRAY,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(Some(out)));
        let func = b.finish();
        let js_arr = lower_fn(&interns, "tid_u32s", &func, &[(0, "t")], EcmaTarget::Es2018);
        assert!(
            js_arr.contains("__rt.addr($b[0], 1)") && js_arr.contains("__rt.addr($b[3], 1)"),
            "expected 4-element u32 array lowering, got:\n{js_arr}"
        );

        let mut b2 = FuncBuilder::new(item, sig(vec![TYPE_ID], U64_PAIR));
        let p2 = b2.param(TYPE_ID, NO_ORIGIN);
        let out2 = b2.op(
            e,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: p2,
                from: TYPE_ID,
                to: U64_PAIR,
            },
            U64_PAIR,
            NO_ORIGIN,
        );
        b2.terminate(e, RTerm::Return(Some(out2)));
        let func2 = b2.finish();
        let js_pair = lower_fn(
            &interns,
            "tid_pair",
            &func2,
            &[(0, "t")],
            EcmaTarget::Es2018,
        );
        assert!(
            js_pair.contains("BigInt.asUintN(64,") && js_pair.contains("BigInt(\"64\")"),
            "expected (u64, u64) tuple lowering, got:\n{js_pair}"
        );
    }

    #[test]
    fn ptr_to_ptr_casts_thin_ptr_elem_and_slice_to_u64_via_scalar_views() {
        let interns = type_id_world();
        let e = FuncBuilder::ENTRY;
        let item = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 4,
            },
            instantiation: 4,
        };
        for (name, from_ty) in [
            ("thin_to_u64", RAW_PTR_UNIT),
            ("slice_to_u64", RAW_SLICE_PTR_UNIT),
        ] {
            let mut b = FuncBuilder::new(item, sig(vec![from_ty], RAW_U64));
            let p = b.param(from_ty, NO_ORIGIN);
            let out = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::PtrToPtr,
                    v: p,
                    from: from_ty,
                    to: RAW_U64,
                },
                RAW_U64,
                NO_ORIGIN,
            );
            b.terminate(e, RTerm::Return(Some(out)));
            let func = b.finish();
            let js = lower_fn(&interns, name, &func, &[(0, "p")], EcmaTarget::Es2018);
            assert_eq!(
                js,
                format!(
                    "function {name}(p) {{\n  return __rt.unscale_scalar(__rt.scale_scalar(p, 4, \"u32\"), 8, () => BigInt(\"0\"), \"u64\");\n}}\n"
                )
            );
        }
    }

    #[test]
    fn transmute_thin_ptr_and_four_byte_array_roundtrips() {
        let interns = type_id_world();
        let e = FuncBuilder::ENTRY;
        let item = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 5,
            },
            instantiation: 5,
        };
        let mut b1 = FuncBuilder::new(item, sig(vec![RAW_UNIT], U8_ARRAY_4));
        let p = b1.param(RAW_UNIT, NO_ORIGIN);
        let out = b1.op(
            e,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: p,
                from: RAW_UNIT,
                to: U8_ARRAY_4,
            },
            U8_ARRAY_4,
            NO_ORIGIN,
        );
        b1.terminate(e, RTerm::Return(Some(out)));
        let f1 = b1.finish();
        let js1 = lower_fn(
            &interns,
            "ptr_to_bytes",
            &f1,
            &[(0, "p")],
            EcmaTarget::Es2018,
        );
        assert!(
            js1.contains("__rt.addr(p, 1) >>> 0") && js1.contains("$b >>> 24 & 255"),
            "expected thin ptr -> [u8; 4] lowering, got:\n{js1}"
        );

        let mut b2 = FuncBuilder::new(item, sig(vec![U8_ARRAY_4], RAW_UNIT));
        let bs = b2.param(U8_ARRAY_4, NO_ORIGIN);
        let out2 = b2.op(
            e,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: bs,
                from: U8_ARRAY_4,
                to: RAW_UNIT,
            },
            RAW_UNIT,
            NO_ORIGIN,
        );
        b2.terminate(e, RTerm::Return(Some(out2)));
        let f2 = b2.finish();
        let js2 = lower_fn(
            &interns,
            "bytes_to_ptr",
            &f2,
            &[(0, "b")],
            EcmaTarget::Es2018,
        );
        assert!(
            js2.contains("$b[0]") && js2.contains("$b[3] << 24"),
            "expected [u8; 4] -> thin ptr lowering, got:\n{js2}"
        );
    }

    #[test]
    fn transmute_dyn_auto_traits_preserves_fat_pointer_identity() {
        let interns = type_id_world();
        let e = FuncBuilder::ENTRY;
        let item = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 6,
            },
            instantiation: 6,
        };
        let mut b = FuncBuilder::new(item, sig(vec![RAW_DYN_ERR], RAW_DYN_ERR_SEND_SYNC));
        let p = b.param(RAW_DYN_ERR, NO_ORIGIN);
        let out = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: p,
                from: RAW_DYN_ERR,
                to: RAW_DYN_ERR_SEND_SYNC,
            },
            RAW_DYN_ERR_SEND_SYNC,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(Some(out)));
        let func = b.finish();
        let js = lower_fn(
            &interns,
            "reapply_send_sync",
            &func,
            &[(0, "s")],
            EcmaTarget::Es2018,
        );
        assert_eq!(js, "function reapply_send_sync(s) {\n  return s;\n}\n");
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "exercises 7 POD/niche/structural/raw_eq/cast lowering cases against one shared TypeTable"
    )]
    fn pod_transmute_niche_char_structural_box_raw_eq_and_casts_lower_cleanly() {
        let mut interns = type_id_world();
        let u16_ty = TypeId(30);
        let u8_16 = TypeId(31);
        let u16_8 = TypeId(32);
        let char_ty = TypeId(33);
        let opt_nz_char = TypeId(34);
        let node_ty = TypeId(35);
        let mu_node_ty = TypeId(36);
        let ptr_node = TypeId(37);
        let ptr_mu_node = TypeId(38);
        let ref_alloc = TypeId(39);
        let box_mu = TypeId(40);
        let box_init = TypeId(41);
        let u32_2 = TypeId(42);
        let nested_u32 = TypeId(43);
        let ref_nested_u32 = TypeId(44);
        let bool_ty = TypeId(45);
        let usize_32 = TypeId(46);
        let u8_0 = TypeId(47);
        let rot_buf = TypeId(48);
        let ptr_rot_buf = TypeId(49);
        let ptr_u8 = TypeId(50);
        let ident_ty = TypeId(51);
        let ptr_ident = TypeId(52);
        let inner_ty = TypeId(53);
        let outer_ty = TypeId(54);
        let ptr_outer = TypeId(55);
        let ptr_u32 = TypeId(56);

        interns.intern_type(int_desc(u16_ty, false, 16));
        let mut d_u8_16 = base_desc(
            u8_16,
            TypeKind::Array {
                element: U8,
                length: 16,
            },
            16,
        );
        d_u8_16.js.is_indirect = true;
        interns.intern_type(d_u8_16);
        let mut d_u16_8 = base_desc(
            u16_8,
            TypeKind::Array {
                element: u16_ty,
                length: 8,
            },
            16,
        );
        d_u16_8.js.is_indirect = true;
        interns.intern_type(d_u16_8);

        interns.intern_type(base_desc(char_ty, TypeKind::Char, 4));
        let s_none = interns.intern_str("None");
        let s_some = interns.intern_str("Some");
        let k_0 = interns.intern_str("_0");
        let k_1 = interns.intern_str("_1");
        let mut d_opt = base_desc(
            opt_nz_char,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 2,
                    local_hash: 1,
                },
                args: vec![char_ty],
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: s_none,
                        fields: vec![],
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: vec![],
                    },
                    VariantDescriptor {
                        name: s_some,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(k_0),
                            ty: char_ty,
                            offset: 0,
                        }],
                        logical_discr: 1,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: vec![],
                    },
                ],
                niche: Some(rcgjs_object::jir::types::Niche {
                    untagged_variant: 1,
                    niche_field: vec![0],
                    niche_start: 0,
                    valid_range: (1, 0x10_ffff),
                }),
            },
            4,
        );
        d_opt.js.enum_repr = Some(rcgjs_object::jir::types::EnumRepr::NullNiche {
            none_variant: 0,
            some_variant: 1,
        });
        interns.intern_type(d_opt);

        // Box<MaybeUninit<Node>, &Global> -> Box<Node, &Global>
        interns.intern_type(int_desc(node_ty, false, 32));
        let mut d_mu = base_desc(
            mu_node_ty,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            4,
        );
        d_mu.js.peeled = node_ty;
        d_mu.js.crosses_transparent_union = true;
        interns.intern_type(d_mu);
        let mut d_pn = base_desc(
            ptr_node,
            TypeKind::RawPointer {
                pointee: node_ty,
                mutable: true,
            },
            4,
        );
        d_pn.js.pointer_form = Some(PointerForm::Slot);
        interns.intern_type(d_pn);
        let mut d_pmn = base_desc(
            ptr_mu_node,
            TypeKind::RawPointer {
                pointee: mu_node_ty,
                mutable: true,
            },
            4,
        );
        d_pmn.js.pointer_form = Some(PointerForm::Slot);
        interns.intern_type(d_pmn);
        let mut d_ra = base_desc(
            ref_alloc,
            TypeKind::Reference {
                pointee: UNIT,
                mutable: false,
            },
            4,
        );
        d_ra.js.pointer_form = Some(PointerForm::Slot);
        interns.intern_type(d_ra);
        let s_box = interns.intern_str("Box");
        for (tid, ptr_field) in [(box_mu, ptr_mu_node), (box_init, ptr_node)] {
            let fields = vec![
                FieldDescriptor {
                    key: FieldKey::Name(k_0),
                    ty: ptr_field,
                    offset: 0,
                },
                FieldDescriptor {
                    key: FieldKey::Name(k_1),
                    ty: ref_alloc,
                    offset: 4,
                },
            ];
            let mut db = base_desc(
                tid,
                TypeKind::Adt {
                    def: DefinitionId {
                        crate_id: 2,
                        local_hash: 2,
                    },
                    args: vec![],
                    form: NominalForm::Struct,
                    variants: vec![VariantDescriptor {
                        name: s_box,
                        fields: fields.clone(),
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: vec![],
                    }],
                    niche: None,
                },
                8,
            );
            db.js.is_indirect = true;
            db.js.fields = fields;
            interns.intern_type(db);
        }

        // Nested array [[u32; 2]; 3] for raw_eq
        let mut d_u32_2 = base_desc(
            u32_2,
            TypeKind::Array {
                element: U32,
                length: 2,
            },
            8,
        );
        d_u32_2.js.is_indirect = true;
        interns.intern_type(d_u32_2);
        let mut d_nested = base_desc(
            nested_u32,
            TypeKind::Array {
                element: u32_2,
                length: 3,
            },
            24,
        );
        d_nested.js.is_indirect = true;
        interns.intern_type(d_nested);
        let mut d_ref_nested = base_desc(
            ref_nested_u32,
            TypeKind::Reference {
                pointee: nested_u32,
                mutable: false,
            },
            4,
        );
        d_ref_nested.js.pointer_form = Some(PointerForm::Thin);
        interns.intern_type(d_ref_nested);
        interns.intern_type(base_desc(bool_ty, TypeKind::Bool, 1));

        // ([usize; 32], [u8; 0]) -> *mut u8 (`uninit_cast`)
        let mut d_us32 = base_desc(
            usize_32,
            TypeKind::Array {
                element: U32,
                length: 32,
            },
            128,
        );
        d_us32.js.is_indirect = true;
        interns.intern_type(d_us32);
        interns.intern_type(base_desc(
            u8_0,
            TypeKind::Array {
                element: U8,
                length: 0,
            },
            0,
        ));
        let mut d_rot = base_desc(rot_buf, TypeKind::Tuple(vec![usize_32, u8_0]), 128);
        d_rot.js.is_indirect = true;
        d_rot.js.fields = vec![
            FieldDescriptor {
                key: FieldKey::Index(0),
                ty: usize_32,
                offset: 0,
            },
            FieldDescriptor {
                key: FieldKey::Index(1),
                ty: u8_0,
                offset: 128,
            },
        ];
        interns.intern_type(d_rot);
        let mut d_prot = base_desc(
            ptr_rot_buf,
            TypeKind::RawPointer {
                pointee: rot_buf,
                mutable: true,
            },
            4,
        );
        d_prot.js.pointer_form = Some(PointerForm::Slot);
        interns.intern_type(d_prot);
        let mut d_pu8 = base_desc(
            ptr_u8,
            TypeKind::RawPointer {
                pointee: U8,
                mutable: true,
            },
            4,
        );
        d_pu8.js.pointer_form = Some(PointerForm::Slot);
        interns.intern_type(d_pu8);

        // SemverIdent { head: *const u8, tail: [u8; 4] } -> *const u64 (`pod_view`)
        let k_head = interns.intern_str("head");
        let k_tail = interns.intern_str("tail");
        let s_ident = interns.intern_str("SemverIdent");
        let ident_fields = vec![
            FieldDescriptor {
                key: FieldKey::Name(k_head),
                ty: ptr_u8,
                offset: 0,
            },
            FieldDescriptor {
                key: FieldKey::Name(k_tail),
                ty: U8_ARRAY_4,
                offset: 4,
            },
        ];
        let mut d_ident = base_desc(
            ident_ty,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 2,
                    local_hash: 3,
                },
                args: vec![],
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: s_ident,
                    fields: ident_fields.clone(),
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: vec![],
                }],
                niche: None,
            },
            8,
        );
        d_ident.js.is_indirect = true;
        d_ident.js.fields = ident_fields;
        interns.intern_type(d_ident);
        let mut d_pident = base_desc(
            ptr_ident,
            TypeKind::RawPointer {
                pointee: ident_ty,
                mutable: false,
            },
            4,
        );
        d_pident.js.pointer_form = Some(PointerForm::Slot);
        interns.intern_type(d_pident);

        // Outer { inner: Inner { val: u32, extra: u32 }, tag: u32 } -> *const u32 (nested prefix)
        let k_val = interns.intern_str("val");
        let k_extra = interns.intern_str("extra");
        let k_inner = interns.intern_str("inner");
        let k_tag = interns.intern_str("tag");
        let s_inner = interns.intern_str("Inner");
        let s_outer = interns.intern_str("Outer");
        let inner_fields = vec![
            FieldDescriptor {
                key: FieldKey::Name(k_val),
                ty: U32,
                offset: 0,
            },
            FieldDescriptor {
                key: FieldKey::Name(k_extra),
                ty: U32,
                offset: 4,
            },
        ];
        let mut d_inner = base_desc(
            inner_ty,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 2,
                    local_hash: 4,
                },
                args: vec![],
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: s_inner,
                    fields: inner_fields.clone(),
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: vec![],
                }],
                niche: None,
            },
            8,
        );
        d_inner.js.is_indirect = true;
        d_inner.js.fields = inner_fields;
        interns.intern_type(d_inner);
        let outer_fields = vec![
            FieldDescriptor {
                key: FieldKey::Name(k_inner),
                ty: inner_ty,
                offset: 0,
            },
            FieldDescriptor {
                key: FieldKey::Name(k_tag),
                ty: U32,
                offset: 8,
            },
        ];
        let mut d_outer = base_desc(
            outer_ty,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 2,
                    local_hash: 5,
                },
                args: vec![],
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: s_outer,
                    fields: outer_fields.clone(),
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: vec![],
                }],
                niche: None,
            },
            12,
        );
        d_outer.js.is_indirect = true;
        d_outer.js.fields = outer_fields;
        interns.intern_type(d_outer);
        let mut d_pouter = base_desc(
            ptr_outer,
            TypeKind::RawPointer {
                pointee: outer_ty,
                mutable: false,
            },
            4,
        );
        d_pouter.js.pointer_form = Some(PointerForm::Slot);
        interns.intern_type(d_pouter);
        let mut d_pu32 = base_desc(
            ptr_u32,
            TypeKind::RawPointer {
                pointee: U32,
                mutable: false,
            },
            4,
        );
        d_pu32.js.pointer_form = Some(PointerForm::Slot);
        interns.intern_type(d_pu32);

        let e = FuncBuilder::ENTRY;
        let item = ItemId {
            definition: DefinitionId {
                crate_id: 2,
                local_hash: 10,
            },
            instantiation: 1,
        };

        // 1. [u8; 16] -> [u16; 8] (`Ipv6Addr::segments`)
        let mut b = FuncBuilder::new(item, sig(vec![u8_16], u16_8));
        let p = b.param(u8_16, NO_ORIGIN);
        let out = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: p,
                from: u8_16,
                to: u16_8,
            },
            u16_8,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(Some(out)));
        let js_ipv6 = lower_fn(
            &interns,
            "ipv6_segs",
            &b.finish(),
            &[(0, "b")],
            EcmaTarget::Es2018,
        );
        assert!(
            js_ipv6.contains("new DataView(new ArrayBuffer(16))")
                && js_ipv6.contains("setUint8")
                && js_ipv6.contains("getUint16"),
            "expected DataView pack/unpack for [u8; 16] -> [u16; 8], got:\n{js_ipv6}"
        );

        // 2. char -> Option<NonZero<char>>
        let mut b = FuncBuilder::new(item, sig(vec![char_ty], opt_nz_char));
        let p = b.param(char_ty, NO_ORIGIN);
        let out = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: p,
                from: char_ty,
                to: opt_nz_char,
            },
            opt_nz_char,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(Some(out)));
        let js_nz = lower_fn(
            &interns,
            "nz_char",
            &b.finish(),
            &[(0, "c")],
            EcmaTarget::Es2018,
        );
        assert!(
            js_nz.contains("$n === 0 ? null : $n"),
            "expected NonZero<char> niche decode, got:\n{js_nz}"
        );

        // 3. Box<MaybeUninit<Node>, &Global> -> Box<Node, &Global> folds to identity
        let mut b = FuncBuilder::new(item, sig(vec![box_mu], box_init));
        let p = b.param(box_mu, NO_ORIGIN);
        let out = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: p,
                from: box_mu,
                to: box_init,
            },
            box_init,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(Some(out)));
        let js_box = lower_fn(
            &interns,
            "box_assume",
            &b.finish(),
            &[(0, "bx")],
            EcmaTarget::Es2018,
        );
        assert_eq!(js_box, "function box_assume(bx) {\n  return bx;\n}\n");

        // 4. raw_eq(&[[u32; 2]; 3], &[[u32; 2]; 3])
        let raw_eq_name = interns.intern_str("raw_eq");
        let mut b = FuncBuilder::new(item, sig(vec![ref_nested_u32, ref_nested_u32], bool_ty));
        let a = b.param(ref_nested_u32, NO_ORIGIN);
        let c = b.param(ref_nested_u32, NO_ORIGIN);
        let out = b.op(
            e,
            OpKind::Intrinsic {
                name: raw_eq_name,
                args: vec![a, c],
                types: vec![nested_u32],
                fallback: None,
                track_caller: None,
            },
            bool_ty,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(Some(out)));
        let js_eq = lower_fn(
            &interns,
            "nested_raw_eq",
            &b.finish(),
            &[(0, "a"), (1, "b")],
            EcmaTarget::Es2018,
        );
        assert!(
            js_eq.contains(".every(") && js_eq.contains("$e1 === $t1[$i][$i1]"),
            "expected nested every raw_eq lowering, got:\n{js_eq}"
        );

        // 5. *mut ([usize; 32], [u8; 0]) -> *mut u8 (`uninit_cast`)
        let mut b = FuncBuilder::new(item, sig(vec![ptr_rot_buf], ptr_u8));
        let p = b.param(ptr_rot_buf, NO_ORIGIN);
        let out = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: p,
                from: ptr_rot_buf,
                to: ptr_u8,
            },
            ptr_u8,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(Some(out)));
        let js_rot = lower_fn(
            &interns,
            "rot_cast",
            &b.finish(),
            &[(0, "p")],
            EcmaTarget::Es2018,
        );
        assert!(
            js_rot.contains("__rt.uninit_cast(p, 128, 1,"),
            "expected uninit_cast lowering, got:\n{js_rot}"
        );

        // 6. *const SemverIdent -> *const u64 (`pod_view`)
        let mut b = FuncBuilder::new(item, sig(vec![ptr_ident], RAW_U64));
        let p = b.param(ptr_ident, NO_ORIGIN);
        let out = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: p,
                from: ptr_ident,
                to: RAW_U64,
            },
            RAW_U64,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(Some(out)));
        let js_ident = lower_fn(
            &interns,
            "ident_cast",
            &b.finish(),
            &[(0, "p")],
            EcmaTarget::Es2018,
        );
        assert!(
            js_ident.contains("__rt.pod_view(p,"),
            "expected pod_view lowering for *const SemverIdent -> *const u64, got:\n{js_ident}"
        );

        // 7. *const Outer -> *const u32 (multi-level offset-0 prefix)
        let mut b = FuncBuilder::new(item, sig(vec![ptr_outer], ptr_u32));
        let p = b.param(ptr_outer, NO_ORIGIN);
        let out = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: p,
                from: ptr_outer,
                to: ptr_u32,
            },
            ptr_u32,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(Some(out)));
        let js_outer = lower_fn(
            &interns,
            "outer_cast",
            &b.finish(),
            &[(0, "p")],
            EcmaTarget::Es2018,
        );
        assert!(
            js_outer.contains("__rt.prefix(__rt.prefix(p, \"inner\", 12), \"val\", 8)"),
            "expected nested offset-0 prefix cast, got:\n{js_outer}"
        );
    }
}
