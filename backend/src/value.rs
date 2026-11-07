//! How Rust values are represented in JavaScript, and the small type queries the MIR walk needs
//! to decide between those representations.
//!
//! The representation follows `CONTRACT.md`:
//!
//! * `i8..=i32`, `u8..=u32`, `isize`/`usize`, `char`, `f32`/`f64` are JS numbers, `bool` is a JS
//!   boolean, `&str` is a JS string.
//! * `i64`/`u64`/`i128`/`u128` are JS **BigInt**s — see [`IntRepr`].
//! * A struct is an object keyed by field name; a field whose name is a number (a tuple struct)
//!   is keyed `_0`, `_1`, ... so the key stays a plain identifier.
//! * Tuples, arrays and closure environments are JS arrays.
//! * An enum is one of three things, and which one follows from its declaration: see [`EnumRepr`].
//! * `()` and zero sized values are `undefined`.
//!
//! # References
//!
//! One rule, chosen so that a `Deref` never has to know where the reference came from:
//!
//! * A reference to a place whose type is *indirect* (anything whose JS value is an object with
//!   identity: structs, tuples, arrays, enums, closures, `str`, and every zero sized type) **is
//!   that same JS value**. `Deref` of it is a no-op, and mutation through it works because JS
//!   objects have reference semantics.
//! * A reference to a place whose type is *direct* (a number, a boolean, a pointer — everything
//!   whose JS value carries no identity) is a `{ buf, off }` **slot** naming the place, and
//!   `Deref` of it is the assignable expression `p.buf[p.off]`. `ptr.rs` owns that model, and
//!   `CONTRACT.md`'s "Pointers" section is its specification.
//!
//! A local whose address is taken and whose type is direct has no place for a slot to name, so it
//! is **boxed**: declared `let x = []` and used as `x[0]` (`uses.rs`, `base.rs`). That replaces
//! the accessor objects (`{ get, set }`) earlier stages emitted, and with them the last of the
//! contract's cells.

use rustc_abi::{FIRST_VARIANT, FieldIdx, Size, TagEncoding, VariantIdx, Variants};
use rustc_middle::ty::{self, AdtDef, ScalarInt, Ty, TyCtxt};

use crate::jsast::{self, Expr};
use crate::naming::Namer;

/// The typing environment every query in this backend runs in: everything is monomorphic by the
/// time codegen sees it.
pub(crate) fn typing_env<'tcx>() -> ty::TypingEnv<'tcx> {
    ty::TypingEnv::fully_monomorphized()
}

/// Strips the pattern off a pattern type, which is represented exactly like its base type.
///
/// `NonNull<T>` is a struct over `pattern_type!(*const T is !null)`, so real `core` puts a
/// `ty::Pat` in the path of anything that touches a `NonNull` — the caller `Location`'s file name,
/// `fmt::Arguments`, every `Option<&T>` niche. A pattern only restricts which values inhabit the
/// type; it changes neither the layout nor the JavaScript value, so every representation question
/// is asked of the base type.
pub(crate) fn peel_pattern<'tcx>(mut ty: Ty<'tcx>) -> Ty<'tcx> {
    while let ty::Pat(base, _) = ty.kind() {
        ty = *base;
    }
    ty
}

/// Whether values of `ty` occupy no space, and so are `undefined` in JavaScript.
pub(crate) fn is_zst<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    match tcx.layout_of(typing_env().as_query_input(ty)) {
        Ok(layout) => layout.is_zst(),
        Err(_) => false,
    }
}

/// The field a **transparent** aggregate is represented as, together with its index.
///
/// Two shapes qualify, for the same reason: the aggregate holds exactly its field's bits, so it
/// *is* the field rather than an object wrapping it.
///
/// * A **union** with exactly one non-zero-sized field. `MaybeUninit<T>` is its `ManuallyDrop<T>`
///   rather than an object with a `value` key, and `MaybeUninit::uninit()` — which names the zero
///   sized arm — is `undefined`.
/// * A **`repr(transparent)` struct**, whose one non-1-ZST field the representation is guaranteed
///   to be. `NonNull<T>` is the pointer inside it, `ManuallyDrop<T>` is its `T`, and `NonZero<u32>`
///   is a number.
/// * A **`Box<T, A>`** whose allocator is zero sized, which is every `Box` in a program with one
///   global allocator. A box *is* the pointer it owns: `Box::new` allocates, casts and writes, and
///   a `Box<[T]>` or a `Box<dyn Trait>` is exactly the fat pointer that names the allocation.
///   Rust does not call a box `repr(transparent)`, because a box is a language item rather than a
///   library type, but it holds nothing else.
///
/// The struct half is what makes a buffer of them a buffer of what they wrap: `core::fmt` builds
/// its digit buffer out of `[MaybeUninit<u8>; N]`, which is a union over `ManuallyDrop<u8>` over
/// `MaybeDangling<u8>` — two `repr(transparent)` structs — and hands it to
/// `str::from_utf8_unchecked` as bytes. Wrapper objects where bytes are owed is a silent
/// miscompilation rather than a rejection.
///
/// An ordinary newtype keeps its object form: a struct is transparent when it *says* it is, or when
/// [its layout leaves it no room to differ](layout_identical_newtype). The field count alone is
/// never enough.
pub(crate) fn transparent_field_indexed<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
) -> Option<(FieldIdx, Ty<'tcx>)> {
    let peeled = peel_pattern(ty);
    let ty::Adt(def, args) = peeled.kind() else { return None };
    if !def.is_union() && !def.is_struct() {
        return None;
    }
    let mut live = def.non_enum_variant().fields.iter_enumerated().filter_map(|(index, field)| {
        let field_ty = tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, *args));
        (!is_zst(tcx, field_ty)).then_some((index, field_ty))
    });
    let field = live.next()?;
    if live.next().is_some() {
        return None;
    }
    let qualifies = def.is_union()
        || def.repr().transparent()
        || def.is_box()
        || layout_identical_newtype(tcx, peeled, *def, field.0, field.1);
    qualifies.then_some(field)
}

/// Whether a **packed newtype** holds exactly its field's bytes, and so is that field here.
///
/// `#[repr(packed)] struct W<T>(T)` differs from a `repr(transparent)` one in a single respect: its
/// alignment is 1 rather than the field's. This model has no alignment at all -- a value is a
/// JavaScript value, not bytes at an address -- so once the size and the field offset match, there
/// is nothing left for the wrapper to be that its field is not.
///
/// `core::ptr::Unaligned<T>` is why this exists. `read_unaligned` casts a `*const T` to a
/// `*const Unaligned<T>`, reads it, and transmutes the wrapper away; `write_unaligned` wraps and
/// writes. Both are the identity here, but only because construction, field reads, cloning and the
/// pointer cast all agree that the wrapper *is* the field. Answering the pointer cast alone would
/// leave `write_unaligned` storing a wrapper object into a place the rest of the program reads as a
/// number, which is a silent wrong answer rather than a rejection.
///
/// The check is deliberately narrow. `pack` is what separates this from every ordinary newtype: a
/// plain `struct Meters(f64)` is layout-identical to its field too, and keeps its object form,
/// because nothing about it says the two are interchangeable.
fn layout_identical_newtype<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    def: AdtDef<'tcx>,
    index: FieldIdx,
    field_ty: Ty<'tcx>,
) -> bool {
    if def.repr().pack.is_none() {
        return false;
    }
    let (Ok(layout), Ok(field_layout)) = (
        tcx.layout_of(typing_env().as_query_input(ty)),
        tcx.layout_of(typing_env().as_query_input(field_ty)),
    ) else {
        return false;
    };
    layout.is_sized()
        && field_layout.is_sized()
        && layout.size == field_layout.size
        && layout.fields.offset(index.as_usize()) == Size::ZERO
}

/// The field a [transparent aggregate](transparent_field_indexed) is represented as, if it has one.
pub(crate) fn transparent_field<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
    transparent_field_indexed(tcx, ty).map(|(_, field_ty)| field_ty)
}

/// The **unsized tail** of a struct that has one, together with the index of the field holding it.
///
/// A struct whose last field is unsized is unsized itself, and it is neither a slice nor a trait
/// object: it is a record with a run of elements at the end of it. A pointer to one is therefore a
/// fat pointer of its own -- `{ ptr, meta }`, the record and the tail's metadata, the shape
/// `&dyn Trait` already has (CONTRACT.md, "Pointers"). `place.rs` walks through it, `unsize.rs`
/// builds it and `intrinsics.rs` measures it.
///
/// `core`'s `array::IntoIter` is why this exists: it holds a
/// `PolymorphicIter<[MaybeUninit<T>; N]>` and unsizes a reference to it to
/// `&mut PolymorphicIter<[MaybeUninit<T>]>` for every operation, so `for x in [a, b, c]` reaches
/// this shape.
///
/// `None` for a sized type, for a slice, a `str` or a `dyn` (which are their own fat forms rather
/// than records carrying one) and for an enum (no variant of which may have an unsized field). A
/// transparent wrapper answers the way the type inside it does, so a
/// `ManuallyDrop<PolymorphicIter<[T]>>` is a record with a tail like the type it wraps.
pub(crate) fn unsized_tail_indexed<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
) -> Option<(FieldIdx, Ty<'tcx>)> {
    let ty = peel_pattern(peel_transparent(tcx, ty));
    let ty::Adt(def, args) = ty.kind() else { return None };
    if !def.is_struct() {
        return None;
    }
    let variant = def.non_enum_variant();
    let index = variant.fields.next_index().index().checked_sub(1)?;
    let index = FieldIdx::from_usize(index);
    let tail = tcx.normalize_erasing_regions(typing_env(), variant.fields[index].ty(tcx, args));
    (!tail.is_sized(tcx, typing_env())).then_some((index, tail))
}

/// The [unsized tail](unsized_tail_indexed) of a struct that has one.
pub(crate) fn unsized_tail<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
    unsized_tail_indexed(tcx, ty).map(|(_, tail)| tail)
}

/// The key an enum object carries its variant name under.
///
/// Out of reach of a Rust field of the same name: [`Namer::field_name`] escapes a field literally
/// called `TAG`, so a struct variant can never collide with it.
pub(crate) const TAG: &str = "TAG";

/// How the variants of an enum are told apart in JavaScript.
///
/// The shape follows from the enum's own declaration, never from how a value of it is used, so
/// every part of the backend can ask this question of a type and get the same answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EnumRepr {
    /// **The discriminant, as a plain JavaScript number.** A fieldless enum with an integer `repr`
    /// is nothing but its discriminant, and the `repr` is the promise that the number is the
    /// discriminant itself rather than a spelling the backend chose. That is what makes a
    /// `transmute` between such an enum and that integer a move with no shape change at all, and
    /// what lets a buffer of one-byte ones be read as text. `Ordering`, `AsciiChar` and the enum
    /// inside `mem::Alignment` are all this shape.
    Number,
    /// **The variant's name, as a JavaScript string.** `fieldless` says where that string sits: a
    /// value of an enum whose every variant carries nothing *is* the string, and a value of one
    /// with a payload-carrying variant is an object holding it under [`TAG`], beside the fields of
    /// the active variant.
    ///
    /// A mixed enum keeps the object form for its *fieldless* variants too, and deliberately:
    /// `Option<T>` is `{ TAG: "None" }` rather than `"None"`. An enum with a payload is
    /// [indirect](is_indirect), so a reference to one is the object itself and `*p = v` overwrites
    /// that object in place; a bare string has no identity to overwrite, and `Option::take` --
    /// every `&mut Option<T>` in `core` -- writes exactly that value through exactly such a
    /// reference. The encoding stays **total** either way: the empty variant is a value, never an
    /// absent one.
    Tag {
        /// Whether every variant of the enum carries no fields.
        fieldless: bool,
    },
    /// **The state's index, as a plain JavaScript number under [`TAG`].** A coroutine after
    /// `StateTransform` is an enum in everything but [`ty::TyKind`]: its layout is
    /// [`Variants::Multiple`], its states are variants and its saved locals are per-variant fields.
    /// What it does not have is variant *names*, so its tag is the state's index.
    ///
    /// Always an object, never the bare number: a coroutine holds its upvars beside the tag from
    /// the moment it is built, and it is only ever reached through a `&mut` (`Future::poll` takes
    /// `Pin<&mut Self>`), so a value with no identity to write through would be wrong.
    State,
}

/// The width in bits and the signedness of an [`EnumRepr::Number`] enum's discriminant.
///
/// `None` unless `ty` is a fieldless enum with an integer `repr` whose layout stores the
/// discriminant as itself.
///
/// A **niche** encoding is excluded on purpose. It spells a variant as a spare bit pattern of some
/// other type, and the number that stands for a variant is then not the variant's discriminant;
/// nothing here may guess which.
///
/// The type is asked in the shape its JavaScript value has, so a transparent wrapper around such an
/// enum answers the way the enum does. `core::mem::Alignment` is exactly that wrapper.
pub(crate) fn direct_tag<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<(u64, bool)> {
    let ty = peel_pattern(peel_transparent(tcx, ty));
    let ty::Adt(def, _) = ty.kind() else { return None };
    if !def.is_enum() || def.variants().iter().any(|variant| !variant.fields.is_empty()) {
        return None;
    }
    // Without an integer `repr` the discriminants are the compiler's own numbering, and a variant
    // is spelled by name instead.
    if def.repr().int.is_none() {
        return None;
    }
    let layout = tcx.layout_of(typing_env().as_query_input(ty)).ok()?;
    match layout.variants {
        Variants::Multiple { tag, tag_encoding: TagEncoding::Direct, .. } => {
            let rustc_abi::Primitive::Int(integer, signed) = tag.primitive() else { return None };
            Some((integer.size().bits(), signed))
        }
        _ => None,
    }
}

/// How `ty`'s variants are told apart, or `None` when `ty` is not an enum.
///
/// The type is asked in the shape its JavaScript value has, so a transparent wrapper around an enum
/// answers the way the enum does.
pub(crate) fn enum_repr<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<EnumRepr> {
    let peeled = peel_pattern(peel_transparent(tcx, ty));
    // A coroutine is an enum over its states; see [`EnumRepr::State`].
    if let ty::Coroutine(..) = peeled.kind() {
        return Some(EnumRepr::State);
    }
    let ty::Adt(def, _) = peeled.kind() else { return None };
    if !def.is_enum() {
        return None;
    }
    if direct_tag(tcx, peeled).is_some() {
        return Some(EnumRepr::Number);
    }
    let fieldless = def.variants().iter().all(|variant| variant.fields.is_empty());
    Some(EnumRepr::Tag { fieldless })
}

/// Whether values of `ty` are an enum whose JavaScript value is a **primitive**: a number or a
/// string, with no object and so no identity.
///
/// Zero sized enums are excluded: their value is `undefined`, which every part of this model
/// already treats as an aggregate with nothing in it.
pub(crate) fn is_primitive_enum<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    matches!(enum_repr(tcx, ty), Some(EnumRepr::Number | EnumRepr::Tag { fieldless: true }))
        && !is_zst(tcx, ty)
}

/// The JavaScript string standing for one variant of a [tagged](EnumRepr::Tag) enum.
pub(crate) fn tag_literal(variant: &ty::VariantDef) -> Expr {
    jsast::string(variant.name.as_str())
}

/// The discriminant of one variant, as the literal the value model spells it with.
///
/// Re-signed through [`int_literal`], so a `repr(i8)` enum's first variant is `-1` rather than
/// `255`: the same spelling `SwitchInt`'s case values get, which is what lets the two be compared.
pub(crate) fn discriminant_literal<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    index: VariantIdx,
) -> Expr {
    match ty.discriminant_for_variant(tcx, index) {
        Some(discr) => int_literal(tcx, discr.ty, discr.val)
            .unwrap_or_else(|| jsast::num(discr.val as f64)),
        None => jsast::num(0),
    }
}

/// The JavaScript value of one variant of an enum, built out of that variant's already lowered
/// fields.
///
/// The one place that knows what an enum value looks like. `None` when `ty` is not an enum.
pub(crate) fn enum_value<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    index: VariantIdx,
    fields: Vec<(String, Expr)>,
) -> Option<Expr> {
    let peeled = peel_pattern(peel_transparent(tcx, ty));
    Some(match enum_repr(tcx, peeled)? {
        // The value IS the tag, so the fields (there are none) have nowhere to go.
        EnumRepr::Number | EnumRepr::Tag { fieldless: true } => variant_tag(tcx, peeled, index)?,
        EnumRepr::Tag { fieldless: false } | EnumRepr::State => {
            let mut entries = Vec::with_capacity(fields.len() + 1);
            entries.push((TAG.to_string(), variant_tag(tcx, peeled, index)?));
            entries.extend(fields);
            jsast::object(entries)
        }
    })
}

/// The tag of one variant: the JavaScript value that distinguishes it from the enum's others.
///
/// A name for a [tagged](EnumRepr::Tag) enum and the discriminant number for a
/// [numeric](EnumRepr::Number) one. This is what [`tag_expr`] reads back, so a `SwitchInt` case and
/// a `SetDiscriminant` both spell a variant with it. `None` when `ty` is not an enum.
pub(crate) fn variant_tag<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, index: VariantIdx) -> Option<Expr> {
    let peeled = peel_pattern(peel_transparent(tcx, ty));
    // A coroutine's states have no names, so the index is the tag. It is also the discriminant
    // `SwitchInt` compares against, which is what makes the numeric decision tree work unchanged.
    if let ty::Coroutine(..) = peeled.kind() {
        return Some(jsast::num(index.as_u32() as f64));
    }
    let ty::Adt(def, _) = peeled.kind() else { return None };
    Some(match enum_repr(tcx, peeled)? {
        EnumRepr::Number => discriminant_literal(tcx, peeled, index),
        EnumRepr::Tag { .. } | EnumRepr::State => tag_literal(def.variant(index)),
    })
}

/// How an enum value's tag is reached: the value itself where it *is* the tag, its [`TAG`]
/// otherwise.
pub(crate) fn tag_expr(repr: EnumRepr, value: Expr) -> Expr {
    match repr {
        EnumRepr::Number | EnumRepr::Tag { fieldless: true } => value,
        EnumRepr::Tag { fieldless: false } | EnumRepr::State => jsast::member(value, TAG),
    }
}

/// The **number** an enum value's discriminant is, whatever shape the value has.
///
/// A [number](EnumRepr::Number) enum is already that number. A tagged one carries a name instead,
/// so the number is recovered by a chain over the variants: `t === "A" ? 0 : t === "B" ? 1 : 2`,
/// with the last variant as the fallback that needs no test. `tag` is read once per variant, so the
/// caller passes an expression that costs nothing to repeat.
///
/// This is what a discriminant that is *not* immediately matched on comes out as: `E::A as i32`,
/// `mem::discriminant`, anything that asked for the integer. `None` when `ty` is not an enum.
pub(crate) fn discriminant_number<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    value: Expr,
) -> Option<Expr> {
    let peeled = peel_pattern(peel_transparent(tcx, ty));
    let repr = enum_repr(tcx, peeled)?;
    if repr == EnumRepr::Number {
        return Some(value);
    }
    // A coroutine's tag is already the number: its states have no names to compare against, and
    // the index under `TAG` is the discriminant the resume switches on.
    if repr == EnumRepr::State {
        return Some(tag_expr(repr, value));
    }
    let ty::Adt(def, _) = peeled.kind() else { return None };
    let tag = tag_expr(repr, value);
    let mut indices = def.variants().indices();
    let last = indices.next_back()?;
    let mut number = discriminant_literal(tcx, peeled, last);
    for index in indices.rev() {
        number = jsast::cond(
            jsast::binary(
                jsast::BinOp::StrictEq,
                tag.clone(),
                tag_literal(def.variant(index)),
            ),
            discriminant_literal(tcx, peeled, index),
            number,
        );
    }
    Some(number)
}

/// The variant of `ty` whose discriminant is `discriminant`, if it has one.
///
/// A `SwitchInt` over a discriminant carries the discriminant *values*, and a tagged enum switches
/// over names, so every case value has to be looked back up here.
pub(crate) fn variant_with_discriminant<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    discriminant: u128,
) -> Option<&'tcx ty::VariantDef> {
    let peeled = peel_pattern(peel_transparent(tcx, ty));
    let ty::Adt(def, _) = peeled.kind() else { return None };
    def.variants()
        .iter_enumerated()
        .find(|(index, _)| {
            peeled
                .discriminant_for_variant(tcx, *index)
                .is_some_and(|discr| discr.val == discriminant)
        })
        .map(|(_, variant)| variant)
}

/// Whether a **union** is among the transparent wrappers `ty` peels down through.
///
/// The two halves of [`transparent_field_indexed`] agree on the value and differ on one thing: a
/// union's value may be *nothing at all*. `MaybeUninit::uninit()` is `undefined`, so anything that
/// has to cope with an uninitialized value — the directness rule below, and the clone — asks this
/// rather than whether a wrapper was peeled.
pub(crate) fn crosses_transparent_union<'tcx>(tcx: TyCtxt<'tcx>, mut ty: Ty<'tcx>) -> bool {
    for _ in 0..16 {
        if matches!(peel_pattern(ty).kind(), ty::Adt(def, _) if def.is_union()) {
            return true;
        }
        match transparent_field(tcx, ty) {
            Some(field) => ty = field,
            None => return false,
        }
    }
    false
}

/// `ty` with every [transparent wrapper](transparent_field_indexed) peeled off: the type whose
/// JavaScript representation a value of `ty` actually has.
///
/// The bound is belt and braces — a type cannot contain itself by value — and keeps a malformed
/// type from spinning here.
pub(crate) fn peel_transparent<'tcx>(tcx: TyCtxt<'tcx>, mut ty: Ty<'tcx>) -> Ty<'tcx> {
    for _ in 0..16 {
        match transparent_field(tcx, ty) {
            Some(field) => ty = field,
            None => break,
        }
    }
    ty
}

/// Whether a reference to a place of this type is the place's own JS value (and `Deref` a no-op).
///
/// See the module documentation: everything except the JS primitives is indirect.
///
/// A pointer — a reference or a raw one — is **direct**, and deliberately so. A `{ buf, off }`
/// slot record is an object, but it is an *immutable* one: [`crate::ptr::add`] builds a new record
/// rather than moving `off`, so the record has no identity worth preserving. Treating it as
/// indirect is what used to make `*p = q` on a `&mut &mut T` overwrite the *pointee* of `*p` with
/// the pointee of `q` instead of re-pointing `*p`, which is a silent miscompile; a pointer to a
/// pointer is a slot like any other, and the store through it is an assignment.
///
/// A pattern type is its base type (see [`peel_pattern`]), so `NonNull`'s field answers the same
/// way the `*const T` inside it does.
///
/// A **transparent union is direct**, whatever its field is, and for the same reason a pointer is:
/// its value carries no identity worth preserving. `MaybeUninit::uninit()` is `undefined`, so a
/// value of one may be *nothing at all* — and a reference that is "the object itself" has nowhere
/// to point when there is no object. As a slot it always has somewhere: `mu.write(v)` is
/// `p.buf[p.off] = v`, which works over an uninitialized place and is seen by every alias of it.
///
/// A transparent *struct* is exactly its field here as everywhere else, so it is direct when the
/// field is: `NonNull<T>` is a pointer, `NonZero<u32>` is a number, and `ManuallyDrop<Point>` is
/// the object a `Point` is.
///
/// An enum whose every variant is fieldless is a number or a string ([`EnumRepr`]) and so is
/// **direct** for the same reason a `bool` is. A reference to a local of one therefore needs the
/// local boxed, which `uses.rs` arranges. An enum with a payload keeps an object, and with it the
/// identity a write through a reference to it depends on.
pub(crate) fn is_indirect<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    if crosses_transparent_union(tcx, ty) {
        return false;
    }
    if is_primitive_enum(tcx, ty) {
        return false;
    }
    !matches!(
        peel_pattern(peel_transparent(tcx, ty)).kind(),
        ty::Int(_)
            | ty::Uint(_)
            | ty::Float(_)
            | ty::Bool
            | ty::Char
            | ty::FnPtr(..)
            | ty::Ref(..)
            | ty::RawPtr(..)
    )
}

/// Whether `Operand::Copy` of this type has to clone the JS object rather than alias it.
///
/// References are deliberately excluded: copying a `&mut T` must keep aliasing the same slot. So is
/// a `Box`, which is an owning handle to one allocation rather than a value to duplicate. The usual
/// box is peeled to its pointer before the match is reached ([`transparent_field_indexed`]); the
/// arm below is what still answers for one whose allocator is too large to peel away.
///
/// So is an enum whose JavaScript value is a [primitive](is_primitive_enum): a number or a string
/// has nothing to share, and rebuilding one as an object would be a wrong answer rather than a
/// wasteful one.
pub(crate) fn needs_clone<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    if is_zst(tcx, ty) {
        return false;
    }
    // A transparent union is its field, and copies the way the field does.
    let ty = peel_transparent(tcx, ty);
    match ty.kind() {
        ty::Adt(def, _) if def.is_enum() => !is_primitive_enum(tcx, ty),
        ty::Adt(def, _) => !def.is_box(),
        ty::Tuple(tys) => !tys.is_empty(),
        ty::Array(..) => true,
        ty::Closure(..) => true,
        _ => false,
    }
}

/// The number of enum variants past which a copy stops being rebuilt variant by variant.
///
/// The per variant form repeats the source expression once per variant, so it is only worth it
/// while the chain stays small. Above the limit the copy falls back to `Object.assign`, which is
/// one level deep — correct unless a payload is itself an aggregate.
const MAX_CLONED_VARIANTS: usize = 8;

/// A copy of an aggregate value that shares nothing with the original.
///
/// Structs, tuples, enums and closure environments are rebuilt field by field, so a nested
/// aggregate is copied too; an array copies its spine with `slice()`, or with `map()` when its
/// elements need copies of their own. This is what makes `Operand::Copy` of an aggregate a copy
/// rather than an alias: JS objects have reference semantics, so a shallow copy of a struct
/// holding a struct would still let a write through one be seen through the other.
pub(crate) fn clone_expr<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, value: Expr) -> Expr {
    // A transparent wrapper is its field, and is copied the way the field is — except when a union
    // is among the wrappers, where it may hold nothing at all. `MaybeUninit::uninit()` is
    // `undefined` (see [`transparent_field_indexed`]), and a copy of an uninitialized value is
    // another uninitialized value rather than a walk through fields that are not there:
    // `[MaybeUninit::uninit(); N]` is exactly that copy, N times.
    let peeled = peel_transparent(tcx, ty);
    if peeled != ty {
        if !needs_clone(tcx, peeled) {
            return value;
        }
        let copy = clone_expr(tcx, peeled, value.clone());
        if !crosses_transparent_union(tcx, ty) {
            return copy;
        }
        return jsast::cond(
            jsast::binary(jsast::BinOp::StrictNe, value, jsast::undefined()),
            copy,
            jsast::undefined(),
        );
    }
    // `x` if `x` is copied as is, `clone_expr(x)` if it needs one of its own.
    let clone_field = |field_ty: Ty<'tcx>, read: Expr| {
        if needs_clone(tcx, field_ty) { clone_expr(tcx, field_ty, read) } else { read }
    };

    match ty.kind() {
        ty::Adt(def, args) if def.is_struct() || def.is_union() => {
            let variant = def.non_enum_variant();
            let entries = variant
                .fields
                .iter_enumerated()
                .map(|(idx, field)| {
                    let key = Namer::field_name(variant, idx);
                    let field_ty = tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args));
                    let read = clone_field(field_ty, jsast::member(value.clone(), key.clone()));
                    (key, read)
                })
                .collect();
            jsast::object(entries)
        }
        ty::Adt(def, args) if def.is_enum() => clone_enum(tcx, *def, args, value),
        ty::Tuple(tys) => jsast::array(
            tys.iter()
                .enumerate()
                .map(|(i, field_ty)| {
                    clone_field(field_ty, jsast::index(value.clone(), jsast::num(i as f64)))
                })
                .collect(),
        ),
        // `slice()` copies the spine; an element that is itself an aggregate needs its own copy,
        // which `Array.prototype.map` provides without naming the source twice.
        ty::Array(element_ty, _) if needs_clone(tcx, *element_ty) => jsast::method_call(
            value,
            "map",
            vec![jsast::arrow(
                vec!["$e".to_string()],
                clone_expr(tcx, *element_ty, jsast::id("$e")),
            )],
        ),
        ty::Array(..) => jsast::method_call(value, "slice", vec![]),
        // A closure environment is an array of the captured values, in capture order.
        ty::Closure(_, args) => jsast::array(
            args.as_closure()
                .upvar_tys()
                .iter()
                .enumerate()
                .map(|(i, upvar_ty)| {
                    clone_field(upvar_ty, jsast::index(value.clone(), jsast::num(i as f64)))
                })
                .collect(),
        ),
        _ => shallow_clone(value),
    }
}

/// `Object.assign({}, value)`, a one level copy for a shape this module cannot take apart.
fn shallow_clone(value: Expr) -> Expr {
    jsast::call(jsast::member(jsast::id("Object"), "assign"), vec![jsast::object(vec![]), value])
}

/// A copy of an enum value: `TAG === "A" ? { TAG: "A", _0: ... } : ...`.
///
/// An enum with a payload is `{ TAG, ...fields of the active variant }`, and which fields are live
/// depends on the variant, so the copy is a chain over the variants. Rebuilding each variant rather
/// than spreading the source is what copies a payload that is itself an aggregate, and it also
/// drops the keys of whatever variant the value used to hold.
///
/// Only reached for an enum that has an object: a [primitive](is_primitive_enum) one needs no copy
/// at all ([`needs_clone`]).
fn clone_enum<'tcx>(
    tcx: TyCtxt<'tcx>,
    def: ty::AdtDef<'tcx>,
    args: ty::GenericArgsRef<'tcx>,
    value: Expr,
) -> Expr {
    let payload_needs_clone = def.variants().iter().any(|variant| {
        variant.fields.iter().any(|field| {
            needs_clone(tcx, tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args)))
        })
    });

    // With none of the payloads holding a JS object of their own, a one level copy is already a
    // copy that shares nothing, and it is a fraction of the size.
    if !payload_needs_clone || def.variants().len() > MAX_CLONED_VARIANTS {
        return shallow_clone(value);
    }

    let variant_copy = |index: VariantIdx| {
        let variant = def.variant(index);
        let mut entries = vec![(TAG.to_string(), tag_literal(variant))];
        for (idx, field) in variant.fields.iter_enumerated() {
            let key = Namer::field_name(variant, idx);
            let field_ty = tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args));
            let read = jsast::member(value.clone(), key.clone());
            let read = if needs_clone(tcx, field_ty) {
                clone_expr(tcx, field_ty, read)
            } else {
                read
            };
            entries.push((key, read));
        }
        (tag_literal(variant), jsast::object(entries))
    };

    // The last variant is the fallback of the chain, so it needs no test of its own.
    let mut indices = def.variants().indices();
    let Some(last) = indices.next_back() else { return shallow_clone(value) };
    let mut copy = variant_copy(last).1;
    for index in indices.rev() {
        let (tag, variant) = variant_copy(index);
        copy = jsast::cond(
            jsast::binary(
                jsast::BinOp::StrictEq,
                jsast::member(value.clone(), TAG),
                tag,
            ),
            variant,
            copy,
        );
    }
    copy
}

/// How a field of an aggregate is reached in JavaScript.
#[derive(Clone, Debug)]
pub(crate) enum FieldKey {
    /// An object property, `base.name`.
    Name(String),
    /// An array element, `base[i]`.
    Index(usize),
    /// The field *is* the aggregate: the one field of a [transparent
    /// wrapper](transparent_field_indexed), reached by doing nothing at all. Reading `mu.value` is
    /// reading `mu`, and writing it is writing `mu` — including a union's zero sized arm, whose
    /// value is `undefined`, which is exactly what `MaybeUninit::uninit()` owes.
    Transparent,
}

/// The variant a field projection reads from: the downcast one for an enum, the only one
/// otherwise.
pub(crate) fn variant_def<'tcx>(
    def: ty::AdtDef<'tcx>,
    variant: Option<VariantIdx>,
) -> &'tcx ty::VariantDef {
    if def.is_enum() { def.variant(variant.unwrap_or(FIRST_VARIANT)) } else { def.non_enum_variant() }
}

/// Where to find field `field` of a place of type `base_ty` (already downcast to `variant`).
pub(crate) fn field_key<'tcx>(
    tcx: TyCtxt<'tcx>,
    base_ty: Ty<'tcx>,
    variant: Option<VariantIdx>,
    field: FieldIdx,
) -> Option<FieldKey> {
    // A transparent wrapper is its field, whichever field is named: a union's payload arm is the
    // value itself, and its zero sized arm is the `undefined` an uninitialized value is. A
    // `repr(transparent)` struct has one field that is not a 1-ZST, and reading a 1-ZST field
    // gives `undefined` either way.
    if transparent_field(tcx, base_ty).is_some() {
        return Some(FieldKey::Transparent);
    }
    match base_ty.kind() {
        ty::Adt(def, _) => Some(FieldKey::Name(Namer::field_name(variant_def(*def, variant), field))),
        ty::Tuple(_) | ty::Closure(..) => Some(FieldKey::Index(field.as_usize())),
        ty::Coroutine(def_id, args) => coroutine_field_key(tcx, *def_id, args, variant, field),
        _ => None,
    }
}

/// Where a coroutine keeps field `field` of state `variant`.
///
/// A coroutine's one JavaScript object holds TWO field spaces, and they must not collide:
///
/// * with no state downcast, the field is an **upvar** -- the layout's prefix, exactly a closure's
///   captures, so an index is right and matches what [`ty::Closure`] gets;
/// * inside a state, the field is a **saved local**, and its storage is SHARED between every state
///   that holds it across a suspension. Its identity is therefore the `CoroutineSavedLocal` the
///   layout names it by, NOT the per-state field index: two states that both hold one local can
///   list it at different indices, and naming it by the index would make one local read as another
///   after a resume. That is a silent wrong answer rather than a rejection, which is why the saved
///   locals get a key space of their own.
fn coroutine_field_key<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: rustc_hir::def_id::DefId,
    args: ty::GenericArgsRef<'tcx>,
    variant: Option<VariantIdx>,
    field: FieldIdx,
) -> Option<FieldKey> {
    let Some(variant) = variant else {
        return Some(FieldKey::Index(field.as_usize()));
    };
    let layout = tcx.coroutine_layout(def_id, args).ok()?;
    let saved = layout.variant_fields.get(variant)?.get(field)?;
    Some(FieldKey::Name(format!("$s{}", saved.as_usize())))
}

/// `base.name`, `base[i]`, or `base` itself for the field of a transparent union.
pub(crate) fn field_expr(base: Expr, key: FieldKey) -> Expr {
    match key {
        FieldKey::Name(name) => jsast::member(base, name),
        FieldKey::Index(i) => jsast::index(base, jsast::num(i as f64)),
        FieldKey::Transparent => base,
    }
}

/// Which JavaScript numeric type holds a Rust integer.
///
/// A JS number is a `f64`, so it represents every integer up to 2^53 exactly and none above it.
/// That covers everything up to 32 bits with room to spare, which is why `i32`/`u32` arithmetic
/// can be done in doubles and truncated afterwards with `| 0` / `>>> 0`. It does not cover 64 or
/// 128 bit integers, which are therefore `BigInt`s end to end: literals print with a trailing `n`,
/// truncation is `BigInt.asIntN`/`asUintN`, and every value crossing the boundary in either
/// direction goes through an explicit `BigInt()`/`Number()` conversion (see `rvalue.rs`).
///
/// `isize`/`usize` are *not* wide: the backend compiles for `wasm32-unknown-unknown`, so they are
/// exactly 32 bits. That is the whole reason for the target choice — it makes the pointer width a
/// fact of the compilation rather than an assumption the JS side has to uphold.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntRepr {
    /// A JS number, truncated with the bitwise operators.
    Number,
    /// A JS `BigInt`, truncated with `BigInt.asIntN`/`BigInt.asUintN`.
    BigInt,
}

/// How values of `ty` are represented, for any integer type. Non-integers report [`IntRepr::Number`],
/// which is what `bool`, `char` and the floats all are.
pub(crate) fn int_repr<'tcx>(ty: Ty<'tcx>) -> IntRepr {
    match ty.kind() {
        ty::Int(ty::IntTy::I64 | ty::IntTy::I128) | ty::Uint(ty::UintTy::U64 | ty::UintTy::U128) => {
            IntRepr::BigInt
        }
        _ => IntRepr::Number,
    }
}

/// The signedness and width of an integer type, or `None` if `ty` is not an integer.
///
/// `isize`/`usize` report the target's pointer width, read off `tcx.data_layout` rather than
/// assumed: on `wasm32-unknown-unknown` that is 32, and everything downstream — masking, casts,
/// switch case values — follows from it.
pub(crate) fn int_info<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<(bool, u32)> {
    let pointer_bits = tcx.data_layout.pointer_size().bits() as u32;
    let bits = |width: Option<u64>| width.map_or(pointer_bits, |w| w as u32);
    match ty.kind() {
        ty::Int(int_ty) => Some((true, bits(int_ty.bit_width()))),
        ty::Uint(uint_ty) => Some((false, bits(uint_ty.bit_width()))),
        _ => None,
    }
}

/// Truncates `value` to an integer of the given width and signedness, in the given representation.
///
/// Every arithmetic result goes through this, which is what makes the emitted JavaScript wrap the
/// way Rust does.
pub(crate) fn mask(repr: IntRepr, signed: bool, bits: u32, value: Expr) -> Expr {
    match repr {
        IntRepr::BigInt => jsast::bigint_mask(signed, bits, value),
        // A number-repr integer is at most 32 bits wide (see `IntRepr`), so the bitwise operators
        // — which convert to int32/uint32 first — truncate exactly.
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
pub(crate) fn mask_ty<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, value: Expr) -> Expr {
    match int_info(tcx, ty) {
        Some((signed, bits)) => mask(int_repr(ty), signed, bits, value),
        None => value,
    }
}

/// Reinterprets the low `bits` bits of a raw bit pattern as an integer of that signedness.
///
/// MIR stores every integer scalar — a constant, a `SwitchInt` case value — as an unsigned bit
/// pattern, so `-1i64` arrives as `18446744073709551615`. This is the one place that puts the sign
/// back.
pub(crate) fn resign(signed: bool, bits: u32, raw: u128) -> i128 {
    if signed && bits < 128 && (raw >> (bits - 1)) & 1 == 1 {
        (raw as i128).wrapping_sub(1i128 << bits)
    } else {
        raw as i128
    }
}

/// A `BigInt` literal for a raw bit pattern of the given width and signedness.
pub(crate) fn wide_int_literal(signed: bool, bits: u32, raw: u128) -> Expr {
    if signed { jsast::bigint(resign(true, bits, raw)) } else { jsast::bigint(raw) }
}

/// A literal for a raw integer bit pattern of type `ty`, in whichever representation `ty` uses.
///
/// This is the one place that knows how an integer literal is spelled; `constant.rs` and the
/// `SwitchInt` lowering both go through it, so a `i64` value comes out as `123n` and a `usize` one
/// as `123`. Returns `None` when `ty` is not an integer type.
pub(crate) fn int_literal<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, raw: u128) -> Option<Expr> {
    let (signed, bits) = int_info(tcx, ty)?;
    Some(match int_repr(ty) {
        IntRepr::BigInt => wide_int_literal(signed, bits, raw),
        IntRepr::Number => jsast::num(resign(signed, bits, raw) as f64),
    })
}

/// A literal for an already decoded integer scalar of type `ty`.
pub(crate) fn scalar_int_literal<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    int: ScalarInt,
) -> Option<Expr> {
    int_literal(tcx, ty, int.to_uint(int.size()))
}
