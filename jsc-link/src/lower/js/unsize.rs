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
use rcgjs_object::jir::semantic::DefinitionId;
use rcgjs_object::jir::types::{EnumRepr, FieldKey, IntRepr, NominalForm, TypeId, TypeKind};
use rcgjs_object::jsast::{self, BinOp, Expr};

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
            (ReprClass::Pointer(_), ReprClass::Pointer(_)) => {
                match (value::pointee_of(table, from), value::pointee_of(table, to)) {
                    (Some(from_pointee), Some(to_pointee)) => {
                        self.cast_pointer(from_pointee, to_pointee, value)?
                    }
                    _ => value,
                }
            }
            // A thin pointer and a pointer sized integer.
            (ReprClass::Pointer(true), ReprClass::Int(b)) if b.bits == POINTER_BITS => {
                let address = match value::pointee_of(table, from) {
                    Some(pointee) => self.ptr_addr(pointee, value),
                    None => value,
                };
                value::mask(b.repr, b.signed, b.bits, address)
            }
            (ReprClass::Int(a), ReprClass::Pointer(true)) if a.bits == POINTER_BITS => value,
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
            // A fieldless enum with an integer `repr` and the integer of its own width: such a
            // value *is* its discriminant, so moving between the two moves nothing at all.
            (ReprClass::Other, ReprClass::Int(b))
                if direct_tag(table, from) == Some((b.bits, b.signed)) =>
            {
                value
            }
            (ReprClass::Int(a), ReprClass::Other)
                if direct_tag(table, to) == Some((a.bits, a.signed)) =>
            {
                value
            }
            // The same value seen as a *pointer* rather than as an integer
            // (`NonNull::dangling()` is a transmute of an `Alignment` straight to a pointer).
            (ReprClass::Other, ReprClass::Pointer(true))
                if direct_tag(table, from) == Some((POINTER_BITS, false)) =>
            {
                value
            }
            _ => match self.niche_from_scalar(from, to, value)? {
                Some(decoded) => decoded,
                None => self.zombie(
                    "`transmute` between these types is not supported by rustc_codegen_js"
                        .to_owned(),
                ),
            },
        })
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

    /// The destination's wrappers: the innermost type, and the keys to put back around a value of
    /// it, outermost first.
    fn target_wrappers(&self, ty: TypeId) -> (TypeId, Vec<FieldKey>) {
        let mut ty = ty;
        let mut wrappers = Vec::new();
        for _ in 0..16 {
            let Some((inner, key)) = self.sole_field(ty) else {
                break;
            };
            wrappers.push(key);
            ty = inner;
        }
        (ty, wrappers)
    }

    /// Puts the destination's wrappers back around a transmuted value.
    fn rebuild(&self, wrappers: &[FieldKey], value: Expr) -> Expr {
        let mut value = value;
        for key in wrappers.iter().rev() {
            value = match key {
                // A struct is an object keyed by field name; the zero sized fields that were
                // skipped are `undefined`, which is what reading a missing key gives anyway.
                FieldKey::Name(_) => match value::key_name(self.interns, *key) {
                    Some(name) => jsast::object(vec![(name, value)]),
                    None => value,
                },
                FieldKey::Index(_) => jsast::array(vec![value]),
                // A transparent union is its field: there is no wrapper to put back.
                FieldKey::Transparent => value,
            };
        }
        value
    }

    /// A `transmute` from an integer into a niche encoded two variant enum, decoded at run time.
    ///
    /// This is how `NonZero::<T>::new` is written, and through it a good deal of `core::num`. It
    /// applies only when the destination is a two variant enum whose tag is niche encoded and as
    /// wide as the source integer, the niche variant carries no data, and the other carries
    /// exactly one non-zero-sized field.
    fn niche_from_scalar(&self, from: TypeId, to: TypeId, value: Expr) -> Res<Option<Expr>> {
        let table = self.table;
        let Some((signed, bits)) = table.int_info(from) else {
            return Ok(None);
        };
        let int = Repr {
            repr: table.int_repr(from),
            signed,
            bits,
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
        if variants.len() != 2 {
            return Ok(None);
        }
        // The niche scalar is the payload field, and a transmute promises the two sides have the
        // same size, so the tag is as wide as the source integer exactly when the sizes agree.
        // (The niche field *path* walks rustc's unpeeled field chain, which the type table does
        // not reproduce step for step, so the sizes are what this asks.)
        if value::size_of(table, from) != value::size_of(table, peeled) {
            return Ok(None);
        }
        let _ = bits;
        let untagged = niche.untagged_variant;
        let empty_index = if untagged == 0 { 1 } else { 0 };
        let empty = &variants[empty_index as usize];
        if empty.fields.iter().any(|field| !table.is_zst(field.ty)) {
            return Ok(None);
        }
        let payload = &variants[untagged as usize];
        let mut live = payload.fields.iter().filter(|f| !table.is_zst(f.ty));
        let Some(field) = live.next() else {
            return Ok(None);
        };
        if live.next().is_some() {
            return Ok(None);
        }
        let Ok(niche_start) = i128::try_from(niche.niche_start) else {
            return Ok(None);
        };
        let Some(key) = value::key_name(self.interns, field.key) else {
            return Ok(None);
        };
        let bound = jsast::id(NICHE_PARAM);
        let Some(empty_value) = value::enum_value(table, self.interns, to, empty_index, Vec::new())
        else {
            return Ok(None);
        };
        let filled_field = self.transmute(from, field.ty, bound.clone())?;
        let Some(filled) =
            value::enum_value(table, self.interns, to, untagged, vec![(key, filled_field)])
        else {
            return Ok(None);
        };
        let decoded = jsast::cond(
            jsast::binary(BinOp::StrictEq, bound, int.literal(niche_start)),
            empty_value,
            filled,
        );
        Ok(Some(jsast::call(
            jsast::arrow(vec![NICHE_PARAM.to_owned()], decoded),
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

/// `new To(new From([value]).buffer)[0]`: the bits of `value`, read back as another type.
fn bit_cast(from: &str, to: &str, value: Expr) -> Expr {
    let source = jsast::new(jsast::id(from), vec![jsast::array(vec![value])]);
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

impl Repr {
    fn literal(self, value: i128) -> Expr {
        match self.repr {
            IntRepr::BigInt => jsast::bigint(value),
            IntRepr::Number => jsast::num(value as f64),
        }
    }
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
