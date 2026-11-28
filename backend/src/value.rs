//! How Rust values are represented in JavaScript, and the small type queries the MIR walk needs
//! to decide between those representations.
//!
//! The representation follows `CONTRACT.md`:
//!
//! * `i8..=i32`, `u8..=u32`, `isize`/`usize`, `char`, `f32`/`f64` are JS numbers, `bool` is a JS
//!   boolean, `&str` is a JS string.
//! * `i64`/`u64`/`i128`/`u128` are JS **BigInt**s — see [`IntRepr`].
//! * A struct is an object keyed by field name; a field whose name is a number (a tuple struct) is
//!   keyed `_0`, `_1`, ... so the key stays a plain identifier.
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
//!   whose JS value carries no identity) is a `{ buf, off }` **slot** naming the place, and `Deref`
//!   of it is the assignable expression `p.buf[p.off]`. `ptr.rs` owns that model, and
//!   `CONTRACT.md`'s "Pointers" section is its specification.
//!
//! A local whose address is taken and whose type is direct has no place for a slot to name, so it
//! is **boxed**: declared `let x = []` and used as `x[0]` (`uses.rs`, `base.rs`). That replaces
//! the accessor objects (`{ get, set }`) earlier stages emitted, and with them the last of the
//! contract's cells.

use rustc_abi::{FieldIdx, Size, TagEncoding, VariantIdx, Variants, FIRST_VARIANT};
use rustc_middle::ty::{self, AdtDef, Ty, TyCtxt};

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
///   a `Box<[T]>` or a `Box<dyn Trait>` is exactly the fat pointer that names the allocation. Rust
///   does not call a box `repr(transparent)`, because a box is a language item rather than a
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
    let ty::Adt(def, args) = peeled.kind() else {
        return None;
    };
    if !def.is_union() && !def.is_struct() {
        return None;
    }
    let mut live = def
        .non_enum_variant()
        .fields
        .iter_enumerated()
        .filter_map(|(index, field)| {
            let field_ty = tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args));
            (!is_zst(tcx, field_ty)).then_some((index, field_ty))
        });
    let field = live.next()?;
    if live.next().is_some() {
        return None;
    }
    let qualifies = def.is_union()
        || def.repr().transparent()
        || def.is_box()
        || tcx.is_lang_item(def.did(), rustc_hir::LangItem::DynMetadata)
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
    let ty::Adt(def, args) = ty.kind() else {
        return None;
    };
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
    /// [`Variants::Multiple`], its states are variants and its saved locals are per-variant
    /// fields. What it does not have is variant *names*, so its tag is the state's index.
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
    let ty::Adt(def, _) = ty.kind() else {
        return None;
    };
    if !def.is_enum()
        || def
            .variants()
            .iter()
            .any(|variant| !variant.fields.is_empty())
    {
        return None;
    }
    // Without an integer `repr` the discriminants are the compiler's own numbering, and a variant
    // is spelled by name instead.
    def.repr().int?;
    let layout = tcx.layout_of(typing_env().as_query_input(ty)).ok()?;
    match layout.variants {
        Variants::Multiple {
            tag,
            tag_encoding: TagEncoding::Direct,
            ..
        } => {
            let rustc_abi::Primitive::Int(integer, signed) = tag.primitive() else {
                return None;
            };
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
    let ty::Adt(def, _) = peeled.kind() else {
        return None;
    };
    if !def.is_enum() {
        return None;
    }
    if direct_tag(tcx, peeled).is_some() {
        return Some(EnumRepr::Number);
    }
    let fieldless = def
        .variants()
        .iter()
        .all(|variant| variant.fields.is_empty());
    Some(EnumRepr::Tag { fieldless })
}

/// Whether values of `ty` are an enum whose JavaScript value is a **primitive**: a number or a
/// string, with no object and so no identity.
///
/// Zero sized enums are excluded: their value is `undefined`, which every part of this model
/// already treats as an aggregate with nothing in it.
pub(crate) fn is_primitive_enum<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    matches!(
        enum_repr(tcx, ty),
        Some(EnumRepr::Number | EnumRepr::Tag { fieldless: true })
    ) && !is_zst(tcx, ty)
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
    if def.is_enum() {
        def.variant(variant.unwrap_or(FIRST_VARIANT))
    } else {
        def.non_enum_variant()
    }
}

/// Where to find field `field` of a place of type `base_ty` (already downcast to `variant`).
pub(crate) fn field_key<'tcx>(
    tcx: TyCtxt<'tcx>,
    base_ty: Ty<'tcx>,
    variant: Option<VariantIdx>,
    field: FieldIdx,
) -> Option<FieldKey> {
    // Every zero-sized value is the one JavaScript value `undefined`, including an all-ZST
    // aggregate after rustc has inlined or constant-folded its constructor. A field projection
    // must preserve that representation instead of inventing an object-shaped `.field` read.
    if is_zst(tcx, base_ty) {
        return Some(FieldKey::Transparent);
    }
    // A transparent wrapper is its field, whichever field is named: a union's payload arm is the
    // value itself, and its zero sized arm is the `undefined` an uninitialized value is. A
    // `repr(transparent)` struct has one field that is not a 1-ZST, and reading a 1-ZST field
    // gives `undefined` either way.
    if transparent_field(tcx, base_ty).is_some() {
        return Some(FieldKey::Transparent);
    }
    match base_ty.kind() {
        ty::Adt(def, _) => Some(FieldKey::Name(Namer::field_name(
            variant_def(*def, variant),
            field,
        ))),
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

/// How values of `ty` are represented, for any integer type. Non-integers report
/// [`IntRepr::Number`], which is what `bool`, `char` and the floats all are.
pub(crate) fn int_repr<'tcx>(ty: Ty<'tcx>) -> IntRepr {
    match ty.kind() {
        ty::Int(ty::IntTy::I64 | ty::IntTy::I128)
        | ty::Uint(ty::UintTy::U64 | ty::UintTy::U128) => IntRepr::BigInt,
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

/// Whether `ty` is a one-byte element (`u8`, `i8`, or an 8-bit `repr(int)` fieldless enum,
/// peeling transparent/pattern wrappers), so a buffer of `ty` is a byte buffer.
pub(crate) fn is_byte_element<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    if matches!(
        peel_pattern(peel_transparent(tcx, ty)).kind(),
        ty::Uint(ty::UintTy::U8) | ty::Int(ty::IntTy::I8)
    ) {
        return true;
    }
    matches!(direct_tag(tcx, ty), Some((8, _)))
}

/// Whether a raw pointer to `pointee` is a `{ buf, off }` slot record.
pub(crate) fn is_slot_pointee<'tcx>(tcx: TyCtxt<'tcx>, pointee: Ty<'tcx>) -> bool {
    let pointee = peel_pattern(pointee);
    if matches!(pointee.kind(), ty::Str) {
        return false;
    }
    match tcx.layout_of(typing_env().as_query_input(pointee)) {
        Ok(layout) => layout.is_sized() && !layout.is_zst(),
        Err(_) => false,
    }
}

/// Whether a value of pointer type `pointer_ty` is a `{ buf, off }` slot record.
pub(crate) fn is_slot<'tcx>(tcx: TyCtxt<'tcx>, pointer_ty: Ty<'tcx>) -> bool {
    match peel_pattern(peel_transparent(tcx, pointer_ty)).kind() {
        ty::Ref(_, pointee, _) => !is_indirect(tcx, *pointee) && is_slot_pointee(tcx, *pointee),
        ty::RawPtr(pointee, _) => is_slot_pointee(tcx, *pointee),
        _ => false,
    }
}
