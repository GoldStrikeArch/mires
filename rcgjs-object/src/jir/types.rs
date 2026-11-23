//! Type descriptors: every tcx-derived fact the JavaScript lowering asks of a type, computed once
//! by the producer and answered here without a `TyCtxt`.
//!
//! v1 answered these questions on demand from rustc (`value.rs`, `ptr.rs`, `names.rs`,
//! `vtable.rs`, `coroutine_layout`, `fn_sig`). A link-time lowerer has no rustc, so the producer
//! evaluates each query for every type a body mentions and stores the answers. The queries are
//! kept as helpers on [`TypeTable`] so consumer code reads like v1's.
//!
//! The table is crate-scope (one per object, section S1) and sorted by [`TypeId`], which is the
//! stable `type_id_hash` rustc assigns a monomorphic type. Every `TypeId` a body mentions must
//! resolve in the table; [`TypeTable::verify`] checks the table's own references and
//! `Body::verify` the body's.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::interns::{SigId, StrId};
use super::semantic::DefinitionId;
use super::verify::VerifyError;

/// Stable identity of a monomorphic Rust type: rustc's `type_id_hash`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct TypeId(pub u128);

/// One type: its structure, its layout, the facts the v1 optimizer keyed on, and the JavaScript
/// representation facts the lowering keys on.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypeDescriptor {
    pub id: TypeId,
    /// The type's printed Rust path, as `core::any::type_name` reports it
    /// (`with_no_trimmed_paths!(format!("{ty}"))`). `None` when the producer recorded none.
    ///
    /// A name is not a representation fact and nothing else in the lowering reads one; it is
    /// here because `type_name` is an intrinsic whose *value* is that string, and a link-time
    /// lowering has no `TyCtxt` to ask.
    pub name: Option<StrId>,
    pub kind: TypeKind,
    pub layout: LayoutFacts,
    pub meta: TypeMeta,
    pub js: JsFacts,
}

/// The structure of a type. Mirrors `ty::TyKind` after monomorphization and normalization.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TypeKind {
    /// Append-only: postcard serializes this enum by index.
    Bool,
    Char,
    Integer {
        signed: bool,
        /// `isize`/`usize` report the target's pointer width (32 on `wasm32-unknown-unknown`).
        bits: u32,
    },
    Float {
        bits: u32,
    },
    Str,
    Never,
    Tuple(Vec<TypeId>),
    Array {
        element: TypeId,
        length: u64,
    },
    Slice(TypeId),
    Reference {
        pointee: TypeId,
        mutable: bool,
    },
    RawPointer {
        pointee: TypeId,
        mutable: bool,
    },
    /// A struct, enum, or union. Replaces v1's `Nominal { kind: Adt, .. }`.
    Adt {
        def: DefinitionId,
        /// Type arguments only, in declaration order (lifetimes and consts are dropped).
        args: Vec<TypeId>,
        form: NominalForm,
        /// One per variant in declaration order; exactly one for a struct or union.
        variants: Vec<VariantDescriptor>,
        /// The niche encoding of an enum whose layout has one; `None` for direct-tag,
        /// single-variant, uninhabited, and non-enum layouts.
        niche: Option<Niche>,
    },
    /// The pattern is representationally its base type (`value::peel_pattern`).
    Pattern(TypeId),
    /// A type this schema does not describe further (bound variables, placeholders, aliases that
    /// survived normalization). Never has fields the lowering can name.
    Opaque,
    /// The zero-sized type of one function item. `sig` is the monomorphic signature.
    ///
    /// `instance` is the item the type **resolves to**, with the JavaScript name emitted for it.
    /// It is what a `ReifyFnPointer` coercion and `const_eval_select` name, and it is not
    /// recoverable from `def`: `def` may be a *trait* method, which resolves to the impl's body
    /// through a query only rustc can answer (v1 `rvalue.rs` asked `Instance::resolve` at the
    /// cast). `None` when the type does not resolve to one monomorphic item.
    FnDef {
        def: DefinitionId,
        sig: SigId,
        instance: Option<super::semantic::DirectItemRef>,
    },
    FnPtr {
        sig: SigId,
    },
    /// `dyn Principal + Auto..`. `principal` is `None` for `dyn Send`-shaped objects, whose vtable
    /// is the three common header entries only.
    Dynamic {
        principal: Option<DefinitionId>,
        auto: Vec<DefinitionId>,
    },
    /// A closure environment: a JavaScript array of the captured values, in capture order.
    ///
    /// `fn_pointer_shim` is the `FnOnce::call_once` shim a `ClosureFnPointer` coercion names
    /// (`Instance::resolve_closure(.., ClosureKind::FnOnce)`), with the JavaScript name emitted
    /// for it. Only rustc can resolve it — the shim's definition is the trait method's, not the
    /// closure's — so the producer records it. `None` when the closure does not coerce.
    Closure {
        def: DefinitionId,
        upvars: Vec<TypeId>,
        fn_pointer_shim: Option<super::semantic::DirectItemRef>,
    },
    /// A coroutine after `StateTransform`, which is an enum in everything but `TyKind`: its
    /// states are variants and its saved locals are per-variant fields (`EnumRepr::State`).
    Coroutine {
        def: DefinitionId,
        /// The layout prefix: captured values, exactly a closure's, keyed by index.
        upvars: Vec<TypeId>,
        /// Every `CoroutineSavedLocal` the layout names, `(saved local index, type)`, sorted by
        /// index. Storage of a saved local is SHARED between the states that hold it, so its
        /// identity is this index and never a per-state field position.
        saved_locals: Vec<(u32, TypeId)>,
        /// Per state (variant), the saved locals it holds, by index into `saved_locals`, in the
        /// state's field order. `states.len()` is the number of variants; the three fixed states
        /// (unresumed, returned, poisoned) are included.
        states: Vec<Vec<u32>>,
        /// The same states as variant descriptors, one per entry of `states`, with each field
        /// keyed `$s{saved local index}` (`FieldKey::Name`) and `saved_local_keys` holding those
        /// keys. Redundant with `states` by construction (verified) and kept so a field
        /// projection resolves without formatting a key at lowering time.
        variants: Vec<VariantDescriptor>,
    },
    CoroutineClosure {
        def: DefinitionId,
        upvars: Vec<TypeId>,
    },
    /// An `extern type`.
    Foreign(DefinitionId),
}

/// The Rust nominal form of an [`TypeKind::Adt`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NominalForm {
    /// Append-only: postcard serializes this enum by index.
    Opaque,
    Struct,
    Enum,
    Union,
}

/// One variant of an ADT, or one state of a coroutine.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VariantDescriptor {
    /// The variant's Rust name; the JavaScript tag literal of an [`EnumRepr::Tag`] enum. For a
    /// struct or union's one variant it is what rustc names that variant (the type's own name);
    /// empty for a coroutine state (states have no names).
    pub name: StrId,
    /// Fields in declaration order, each with the key the JavaScript object holds it under.
    pub fields: Vec<FieldDescriptor>,
    /// The discriminant `discriminant_for_variant` reports, re-signed through the enum's repr
    /// type: what `E::A as i32` and `mem::discriminant` observe. `0` for non-enums.
    pub logical_discr: i128,
    /// The exact bit pattern in this variant's physical direct tag, when the layout stores one.
    ///
    /// All variants are `None` for niche, single-variant, and uninhabited layouts.
    pub physical_tag: Option<NominalDiscriminant>,
    /// Whether rustc identifies this variant as the canonical `Result::Ok` language item.
    pub result_ok: bool,
    /// For a coroutine state, the `$s{n}` key of each field, in field order; empty otherwise.
    pub saved_local_keys: Vec<StrId>,
}

/// One field of a variant, tuple, array, or closure environment.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FieldDescriptor {
    pub key: FieldKey,
    pub ty: TypeId,
    /// The layout offset in bytes; `0` when the containing type has no computable layout
    /// (`LayoutFacts::size` is `None`).
    pub offset: u64,
}

/// How a field of an aggregate is reached in JavaScript.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum FieldKey {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// An object property, `base.name`. The name is `names.rs::field_name`'s: a numeric field
    /// name is prefixed `_` (`_0`), a field literally called `TAG` becomes `TAG_`, anything else
    /// is sanitized to an identifier.
    Name(StrId),
    /// An array element, `base[i]` (tuples, closure upvars, coroutine upvars).
    Index(u32),
    /// The field *is* the aggregate: the one field of a transparent wrapper, reached by doing
    /// nothing at all. Reading `mu.value` is reading `mu`, and writing it is writing `mu` —
    /// including a union's zero sized arm, whose value is `undefined`, which is exactly what
    /// `MaybeUninit::uninit()` owes. Every field of a zero-sized aggregate is also this key.
    Transparent,
}

/// The exact bits of a physical direct tag.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NominalDiscriminant {
    pub bits: u128,
    pub bytes: u8,
}

impl NominalDiscriminant {
    /// Whether the bits fit the tag's width.
    pub fn is_valid(self) -> bool {
        match self.bytes {
            1..=15 => self.bits < (1_u128 << (u32::from(self.bytes) * 8)),
            16 => true,
            _ => false,
        }
    }
}

/// A niche tag encoding: `untagged_variant` is the variant whose payload occupies the niche
/// field, every other variant is spelled as an invalid bit pattern of that field.
///
/// The JavaScript value model never uses a niche (a niche enum is still `{ TAG, .. }`), but the
/// facts are needed to read a niche enum out of constant memory and to reason about
/// `Option<&T>`/`Option<NonZero>` candidates.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Niche {
    pub untagged_variant: u32,
    /// The path of field indices from the enum down to the scalar holding the niche.
    pub niche_field: Vec<u32>,
    /// The bit pattern of the first niche variant (`niche_variants.start()` maps to it).
    pub niche_start: u128,
    /// The valid range of the niche scalar for the untagged variant, `(start, end)` inclusive
    /// and possibly wrapping.
    pub valid_range: (u128, u128),
}

/// Layout facts, from `tcx.layout_of` where a layout exists.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LayoutFacts {
    /// `None` when the type is unsized or has no computable layout.
    pub size: Option<u64>,
    /// The size in bytes `tcx.layout_of` reports for an **unsized** type: where its tail begins,
    /// which is the sized prefix's size. `None` for a sized type ([`LayoutFacts::size`] answers
    /// for those) and for a type with no computable layout.
    ///
    /// `size_of_val` of a struct with a slice tail is this plus the tail the metadata measures
    /// (v1 `intrinsics.rs::size_of_tail`), and nothing else reads it.
    pub unsized_prefix: Option<u64>,
    pub align: Option<u64>,
    pub sized: bool,
    /// Whether values of the type occupy no space, and so are `undefined` in JavaScript.
    pub zst: bool,
    /// Whether the type is a 1-ZST (zero size, alignment 1): the fields a `repr(transparent)`
    /// wrapper is allowed to carry beside its one real field.
    pub one_zst: bool,
    /// Whether dropping a value runs any destructor, its own or one inherited through fields.
    pub needs_drop: bool,
    /// Whether this ADT directly defines a synchronous or asynchronous destructor.
    ///
    /// Together with `needs_drop` this distinguishes compiler structural drop from an own
    /// destructor without naming source items.
    pub has_own_drop: bool,
}

/// The facts v1's `representation_plan.rs` keyed native representation analysis on
/// (`TypeMetadata` in schema 11).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypeMeta {
    pub candidate: RepresentationCandidate,
    pub abi: NominalAbi,
    pub boundary: PinnedBoundary,
    pub allocator: AllocatorIdentity,
    pub iterator: IteratorIdentity,
}

impl Default for TypeMeta {
    fn default() -> Self {
        Self {
            candidate: RepresentationCandidate::None,
            abi: NominalAbi::Rust,
            boundary: PinnedBoundary::None,
            allocator: AllocatorIdentity::NotApplicable,
            iterator: IteratorIdentity::None,
        }
    }
}

/// Exact standard-library iterator identity supplied by rustc diagnostic items.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum IteratorIdentity {
    /// Append-only: postcard serializes this enum by index.
    None,
    Slice,
    Enumerate,
    Take,
    Skip,
    SliceMut,
}

/// The allocator carried by a heap-owning standard-library type.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AllocatorIdentity {
    /// Append-only: postcard serializes this enum by index.
    NotApplicable,
    Global,
    Custom,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RepresentationCandidate {
    /// Append-only: postcard serializes this enum by index.
    None,
    Box,
    Vec,
    String,
    BTreeMap,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NominalAbi {
    /// Append-only: postcard serializes this enum by index.
    Rust,
    C,
    Transparent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PinnedBoundary {
    /// Append-only: postcard serializes this enum by index.
    None,
    Host,
    ViewAbi,
}

/// The JavaScript representation facts, exactly the answers v1's `value.rs`/`ptr.rs` computed
/// from `tcx` (see each field). Every fact is asked of the type in the shape its JavaScript value
/// has: a transparent wrapper answers the way the type it peels to does.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JsFacts {
    /// `Some` for an integer type (after peeling patterns and transparent wrappers).
    pub int: Option<IntFacts>,
    /// Whether a reference to a place of this type is the place's own JS value (and `Deref` a
    /// no-op).
    ///
    /// Everything except the JS primitives is indirect. A pointer — a reference or a raw one — is
    /// **direct**, and deliberately so: a `{ buf, off }` slot record is an object, but an
    /// immutable one with no identity worth preserving. Treating it as indirect is what used to
    /// make `*p = q` on a `&mut &mut T` overwrite the *pointee* of `*p` instead of re-pointing
    /// `*p`. A **transparent union is direct**, whatever its field is: `MaybeUninit::uninit()` is
    /// `undefined`, so a value of one may be *nothing at all*, and a reference that is "the object
    /// itself" has nowhere to point when there is no object. An enum whose every variant is
    /// fieldless is a number or a string ([`EnumRepr`]) and so is **direct** for the same reason a
    /// `bool` is; a reference to a local of one therefore needs the local boxed.
    pub is_indirect: bool,
    /// Whether `Operand::Copy` of this type has to clone the JS object rather than alias it.
    ///
    /// True for structs, unions, tuples with fields, arrays, closure environments, and enums with
    /// an object form. References are deliberately excluded: copying a `&mut T` must keep
    /// aliasing the same slot. So is a `Box`, which is an owning handle to one allocation rather
    /// than a value to duplicate. So is a primitive enum: a number or a string has nothing to
    /// share, and rebuilding one as an object would be a wrong answer rather than a wasteful one.
    /// Zero-sized types never need a clone. Asked after peeling transparent wrappers.
    pub needs_clone: bool,
    /// Whether a **union** is among the transparent wrappers this type peels through, so its
    /// value may be `undefined` (`MaybeUninit::uninit()`). A clone of such a value must test for
    /// `undefined` first, and the type is direct regardless of its field.
    pub crosses_transparent_union: bool,
    /// The one field a transparent aggregate is represented as: a union with exactly one
    /// non-zero-sized field, a `repr(transparent)` struct, a `Box<T, A>` with a zero-sized
    /// allocator, or a packed newtype whose size and field offset match its field's. `None` for
    /// an ordinary type.
    pub transparent: Option<TransparentField>,
    /// The type with every transparent wrapper (and pattern) peeled off: the type whose JavaScript
    /// representation a value of this type actually has. Equal to the type's own id when there is
    /// nothing to peel. Bounded at 16 levels.
    pub peeled: TypeId,
    /// The unsized tail of a struct that has one, with the index of the field holding it. A
    /// pointer to such a struct is a `{ ptr, meta }` fat pointer of its own
    /// ([`PointerForm::FatTail`]). `None` for sized types, slices, `str`, `dyn`, and enums.
    pub unsized_tail: Option<TransparentField>,
    /// How the variants are told apart, or `None` for a non-enum.
    pub enum_repr: Option<EnumRepr>,
    /// Whether the value is an enum whose JavaScript value is a **primitive** (a number or a
    /// string): `enum_repr` is `Number` or `Tag { fieldless: true }` and the type is not
    /// zero-sized.
    pub is_primitive_enum: bool,
    /// The JavaScript shape of a pointer type (`Reference`, `RawPointer`, `FnPtr` are `None`;
    /// only reference/raw-pointer types carry one).
    pub pointer_form: Option<PointerForm>,
    /// Whether the type is a one-byte element (`u8`, `i8`, or a fieldless enum with a one-byte
    /// integer `repr`), so a buffer of them is a buffer of bytes and `len` in elements is `len`
    /// in bytes.
    pub byte_element: bool,
    /// The fields of a struct/union (its one variant's, also listed under `variants[0]`), tuple,
    /// array (one entry, the element, keyed `Index(0)`), closure or coroutine upvars — in field
    /// order with their JavaScript keys. Empty for enums (whose fields are per variant) and for
    /// non-aggregates.
    pub fields: Vec<FieldDescriptor>,
}

/// One field of a type that is the type's representation (transparent peel or unsized tail).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TransparentField {
    pub field: u32,
    pub ty: TypeId,
}

/// The signedness, width, and JavaScript representation of an integer type.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IntFacts {
    pub signed: bool,
    pub bits: u32,
    pub repr: IntRepr,
}

/// Which JavaScript numeric type holds a Rust integer.
///
/// A JS number is an `f64`, so it represents every integer up to 2^53 exactly and none above it.
/// That covers everything up to 32 bits, which is why `i32`/`u32` arithmetic is done in doubles
/// and truncated with `| 0` / `>>> 0`. 64 and 128 bit integers are `BigInt`s end to end.
/// `isize`/`usize` are *not* wide: the target is `wasm32-unknown-unknown`, so they are 32 bits.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum IntRepr {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// A JS number, truncated with the bitwise operators.
    Number,
    /// A JS `BigInt`, truncated with `BigInt.asIntN`/`BigInt.asUintN`.
    BigInt,
}

/// How the variants of an enum are told apart in JavaScript.
///
/// The shape follows from the enum's own declaration, never from how a value of it is used, so
/// every part of the lowering can ask this question of a type and get the same answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EnumRepr {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// **The discriminant, as a plain JavaScript number.** A fieldless enum with an integer
    /// `repr` whose layout stores the discriminant directly (a niche is excluded: the number
    /// standing for a variant is then not the discriminant) is nothing but its discriminant. That
    /// is what makes a `transmute` between such an enum and that integer a move with no shape
    /// change, and what lets a buffer of one-byte ones be read as text. `Ordering`, `AsciiChar`
    /// and the enum inside `mem::Alignment` are all this shape.
    Number {
        /// The width of the discriminant's repr type.
        bits: u32,
        signed: bool,
    },
    /// **The variant's name, as a JavaScript string.** `fieldless` says where that string sits:
    /// a value of an enum whose every variant carries nothing *is* the string, and a value of one
    /// with a payload-carrying variant is an object holding it under `TAG`, beside the fields of
    /// the active variant.
    ///
    /// A mixed enum keeps the object form for its *fieldless* variants too, and deliberately:
    /// `Option<T>` is `{ TAG: "None" }` rather than `"None"`. An enum with a payload is indirect,
    /// so a reference to one is the object itself and `*p = v` overwrites that object in place; a
    /// bare string has no identity to overwrite, and `Option::take` — every `&mut Option<T>` in
    /// `core` — writes exactly that value through exactly such a reference. The encoding stays
    /// **total** either way: the empty variant is a value, never an absent one.
    Tag {
        /// Whether every variant of the enum carries no fields.
        fieldless: bool,
    },
    /// **The state's index, as a plain JavaScript number under `TAG`.** A coroutine after
    /// `StateTransform` is an enum in everything but `TyKind`: its layout is `Variants::Multiple`,
    /// its states are variants and its saved locals are per-variant fields. What it does not have
    /// is variant *names*, so its tag is the state's index.
    ///
    /// Always an object, never the bare number: a coroutine holds its upvars beside the tag from
    /// the moment it is built, and it is only ever reached through a `&mut` (`Future::poll` takes
    /// `Pin<&mut Self>`), so a value with no identity to write through would be wrong.
    State,
}

/// The JavaScript shape of a reference or raw pointer, decided by its pointee (`ptr.rs`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PointerForm {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// A reference to an indirect pointee: the pointee's own JS value, `Deref` is the identity.
    Thin,
    /// `{ buf, off }`: a pointer to one place, dereferenced as `p.buf[p.off]`. Every pointer to a
    /// direct pointee, and every raw pointer to an aggregate (through `__rt.box`).
    Slot,
    /// `{ buf, off, len }`: `&[T]`, `*const [T]`.
    FatSlice,
    /// `{ ptr, meta }`: `&dyn Trait`, where `meta` is the vtable array.
    FatDyn,
    /// `{ ptr, meta }`: a pointer to a struct with an unsized tail (`unsized_tail` is `Some` on
    /// the pointee), `meta` being the tail's metadata.
    FatTail,
    /// A JS string: `&str`.
    Str,
}

/// A function signature: what `fn_sig` said after monomorphization.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SigDescriptor {
    pub inputs: Vec<TypeId>,
    pub output: TypeId,
    pub abi: SigAbi,
    /// For `extern "rust-call"`: the arity of the tuple the last input is spread into.
    pub spread_arity: Option<u16>,
    /// Whether the callee takes a trailing caller `Location` parameter.
    pub track_caller: bool,
    /// Whether the signature may unwind (`fn` and not `-C panic=abort`-declared nounwind).
    pub unwind: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum SigAbi {
    /// Append-only: postcard serializes this enum by index.
    Rust,
    RustCall,
    C,
    Other(StrId),
}

/// The signature table, indexed by [`SigId`].
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SigTable {
    pub descriptors: Vec<SigDescriptor>,
}

impl SigTable {
    pub fn get(&self, id: SigId) -> Option<&SigDescriptor> {
        self.descriptors.get(id.0 as usize)
    }
}

/// The type table: every descriptor of one object, sorted by id.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(from = "Vec<TypeDescriptor>", into = "Vec<TypeDescriptor>")]
pub struct TypeTable {
    /// Strictly sorted by `id`.
    descriptors: Vec<TypeDescriptor>,
    index: BTreeMap<TypeId, u32>,
}

impl From<Vec<TypeDescriptor>> for TypeTable {
    /// Sorts and dedups (first occurrence wins), so a table built in any order is canonical.
    fn from(mut descriptors: Vec<TypeDescriptor>) -> Self {
        descriptors.sort_by_key(|descriptor| descriptor.id);
        descriptors.dedup_by_key(|descriptor| descriptor.id);
        let index = descriptors
            .iter()
            .enumerate()
            .map(|(position, descriptor)| (descriptor.id, position as u32))
            .collect();
        Self { descriptors, index }
    }
}

impl From<TypeTable> for Vec<TypeDescriptor> {
    fn from(table: TypeTable) -> Self {
        table.descriptors
    }
}

impl TypeTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts `descriptor` unless its id is present. Returns whether it was inserted.
    pub fn insert(&mut self, descriptor: TypeDescriptor) -> bool {
        if self.index.contains_key(&descriptor.id) {
            return false;
        }
        let position = self
            .descriptors
            .partition_point(|candidate| candidate.id < descriptor.id);
        self.descriptors.insert(position, descriptor);
        // Every descriptor after the insertion point moved by one.
        for (position, descriptor) in self.descriptors.iter().enumerate().skip(position) {
            self.index.insert(descriptor.id, position as u32);
        }
        true
    }

    /// Replaces the descriptor stored under `descriptor.id`, or inserts it.
    ///
    /// The encoder inserts an opaque placeholder before walking a recursive type and replaces it
    /// with the finished descriptor afterwards.
    pub fn replace(&mut self, descriptor: TypeDescriptor) {
        match self.index.get(&descriptor.id) {
            Some(position) => self.descriptors[*position as usize] = descriptor,
            None => {
                self.insert(descriptor);
            }
        }
    }

    /// Removes the descriptor stored under `id`. Returns whether one was there.
    ///
    /// The encoder rolls back the descriptors one failed item inserted, so a placeholder it left
    /// behind never masquerades as a finished descriptor for the next item.
    pub fn remove(&mut self, id: TypeId) -> bool {
        let Some(position) = self.index.remove(&id) else {
            return false;
        };
        let position = position as usize;
        self.descriptors.remove(position);
        for (position, descriptor) in self.descriptors.iter().enumerate().skip(position) {
            self.index.insert(descriptor.id, position as u32);
        }
        true
    }

    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }

    /// The descriptors, sorted by id.
    pub fn descriptors(&self) -> &[TypeDescriptor] {
        &self.descriptors
    }

    pub fn ids(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.descriptors.iter().map(|descriptor| descriptor.id)
    }

    pub fn contains(&self, id: TypeId) -> bool {
        self.index.contains_key(&id)
    }

    pub fn get(&self, id: TypeId) -> Option<&TypeDescriptor> {
        self.index
            .get(&id)
            .map(|position| &self.descriptors[*position as usize])
    }

    /// The descriptor of `id`, or a verify error naming `context`.
    pub fn require(&self, id: TypeId, context: &str) -> Result<&TypeDescriptor, VerifyError> {
        self.get(id)
            .ok_or_else(|| VerifyError::missing_type(id, context))
    }

    // Query helpers: the v1 `value.rs`/`ptr.rs` questions, answered from stored facts. Each
    // returns a conservative default for an unknown id (the same default an unresolvable type got
    // in v1: not indirect, no clone, no enum, no int), so lowering code stays total; `verify`
    // rejects a body that names an unknown id before lowering ever sees it.

    /// Whether a reference to a place of this type is the place's own JS value. See
    /// [`JsFacts::is_indirect`].
    pub fn is_indirect(&self, id: TypeId) -> bool {
        self.get(id)
            .is_some_and(|descriptor| descriptor.js.is_indirect)
    }

    /// Whether `Operand::Copy` has to clone. See [`JsFacts::needs_clone`].
    pub fn needs_clone(&self, id: TypeId) -> bool {
        self.get(id)
            .is_some_and(|descriptor| descriptor.js.needs_clone)
    }

    /// Whether the type occupies no space (`undefined` in JavaScript).
    pub fn is_zst(&self, id: TypeId) -> bool {
        self.get(id).is_some_and(|descriptor| descriptor.layout.zst)
    }

    /// The type with every transparent wrapper peeled off. See [`JsFacts::peeled`].
    pub fn peeled(&self, id: TypeId) -> TypeId {
        self.get(id).map_or(id, |descriptor| descriptor.js.peeled)
    }

    /// The transparent field of the type, if it has one.
    pub fn transparent_field(&self, id: TypeId) -> Option<TransparentField> {
        self.get(id)
            .and_then(|descriptor| descriptor.js.transparent)
    }

    /// Whether a union is among the transparent wrappers of the type.
    pub fn crosses_transparent_union(&self, id: TypeId) -> bool {
        self.get(id)
            .is_some_and(|descriptor| descriptor.js.crosses_transparent_union)
    }

    /// How the type's variants are told apart, or `None` for a non-enum.
    pub fn enum_repr(&self, id: TypeId) -> Option<EnumRepr> {
        self.get(id).and_then(|descriptor| descriptor.js.enum_repr)
    }

    /// Whether the type is an enum whose JavaScript value is a primitive.
    pub fn is_primitive_enum(&self, id: TypeId) -> bool {
        self.get(id)
            .is_some_and(|descriptor| descriptor.js.is_primitive_enum)
    }

    /// The signedness and width of an integer type (`(signed, bits)`), or `None`.
    pub fn int_info(&self, id: TypeId) -> Option<(bool, u32)> {
        self.get(id)
            .and_then(|descriptor| descriptor.js.int)
            .map(|int| (int.signed, int.bits))
    }

    /// The JavaScript representation of an integer type. Non-integers report
    /// [`IntRepr::Number`], which is what `bool`, `char` and the floats all are.
    pub fn int_repr(&self, id: TypeId) -> IntRepr {
        self.get(id)
            .and_then(|descriptor| descriptor.js.int)
            .map_or(IntRepr::Number, |int| int.repr)
    }

    /// The unsized tail of a struct that has one.
    pub fn unsized_tail(&self, id: TypeId) -> Option<TransparentField> {
        self.get(id)
            .and_then(|descriptor| descriptor.js.unsized_tail)
    }

    /// Whether the type is a one-byte element.
    pub fn byte_element(&self, id: TypeId) -> bool {
        self.get(id)
            .is_some_and(|descriptor| descriptor.js.byte_element)
    }

    /// The JavaScript shape of a pointer type.
    pub fn pointer_form(&self, id: TypeId) -> Option<PointerForm> {
        self.get(id)
            .and_then(|descriptor| descriptor.js.pointer_form)
    }

    /// The variants of an ADT or the states of a coroutine; empty for anything else.
    pub fn variants(&self, id: TypeId) -> &[VariantDescriptor] {
        match self.get(id).map(|descriptor| &descriptor.kind) {
            Some(TypeKind::Adt { variants, .. }) | Some(TypeKind::Coroutine { variants, .. }) => {
                variants
            }
            _ => &[],
        }
    }

    /// Where to find field `field` of a place of type `id`, downcast to `variant` when given.
    ///
    /// The zero-sized and transparent cases answer [`FieldKey::Transparent`] whichever field is
    /// named, exactly as v1's `field_key` did: a field projection of an all-ZST aggregate must
    /// keep `undefined`, and a transparent wrapper is its field. An enum without a downcast reads
    /// its first variant. A coroutine without a downcast reads an upvar by index; with one, a
    /// saved local under its `$s{n}` key.
    pub fn field(&self, id: TypeId, variant: Option<u32>, field: u32) -> Option<FieldDescriptor> {
        let descriptor = self.get(id)?;
        let transparent = |ty: TypeId| {
            Some(FieldDescriptor {
                key: FieldKey::Transparent,
                ty,
                offset: 0,
            })
        };
        if descriptor.layout.zst {
            let ty = Self::declared_field_type(descriptor, variant, field).unwrap_or(id);
            return transparent(ty);
        }
        if let Some(inner) = descriptor.js.transparent {
            let ty = Self::declared_field_type(descriptor, variant, field).unwrap_or(inner.ty);
            return transparent(ty);
        }
        match &descriptor.kind {
            TypeKind::Adt { variants, form, .. } => {
                let index = if *form == NominalForm::Enum {
                    variant.unwrap_or(0)
                } else {
                    0
                };
                variants
                    .get(index as usize)?
                    .fields
                    .get(field as usize)
                    .cloned()
            }
            TypeKind::Coroutine { variants, .. } => match variant {
                None => descriptor.js.fields.get(field as usize).cloned(),
                Some(state) => variants
                    .get(state as usize)?
                    .fields
                    .get(field as usize)
                    .cloned(),
            },
            TypeKind::Tuple(_)
            | TypeKind::Closure { .. }
            | TypeKind::CoroutineClosure { .. }
            | TypeKind::Array { .. } => descriptor.js.fields.get(field as usize).cloned(),
            _ => None,
        }
    }

    /// The declared type of a field, ignoring representation (for the transparent answers).
    fn declared_field_type(
        descriptor: &TypeDescriptor,
        variant: Option<u32>,
        field: u32,
    ) -> Option<TypeId> {
        match &descriptor.kind {
            TypeKind::Adt { variants, form, .. } => {
                let index = if *form == NominalForm::Enum {
                    variant.unwrap_or(0)
                } else {
                    0
                };
                variants
                    .get(index as usize)?
                    .fields
                    .get(field as usize)
                    .map(|field| field.ty)
            }
            TypeKind::Coroutine { variants, .. } => match variant {
                None => descriptor
                    .js
                    .fields
                    .get(field as usize)
                    .map(|field| field.ty),
                Some(state) => variants
                    .get(state as usize)?
                    .fields
                    .get(field as usize)
                    .map(|field| field.ty),
            },
            _ => descriptor
                .js
                .fields
                .get(field as usize)
                .map(|field| field.ty),
        }
    }

    /// The variant of `id` whose logical discriminant is `discriminant` (as the raw unsigned bit
    /// pattern MIR's `SwitchInt` carries), if any.
    ///
    /// A `SwitchInt` over a discriminant carries the discriminant *values*, and a tagged enum
    /// switches over names, so every case value has to be looked back up here. The comparison
    /// re-signs the raw bits through the discriminant width so `-1i8` (`255`) matches.
    pub fn variant_with_discriminant(&self, id: TypeId, discriminant: u128) -> Option<u32> {
        let descriptor = self.get(id)?;
        let peeled = self.get(descriptor.js.peeled).unwrap_or(descriptor);
        let TypeKind::Adt { variants, .. } = &peeled.kind else {
            return None;
        };
        variants
            .iter()
            .position(|variant| {
                variant.logical_discr == discriminant as i128
                    || variant.logical_discr as u128 == discriminant
            })
            .map(|position| position as u32)
    }

    /// Checks the table's own invariants: strictly sorted unique ids, every referenced type
    /// present, coroutine `states`/`variants` agreement, and enum tag consistency.
    pub fn verify(&self) -> Result<(), VerifyError> {
        let mut previous = None;
        for (position, descriptor) in self.descriptors.iter().enumerate() {
            if previous.is_some_and(|id| id >= descriptor.id) {
                return Err(VerifyError::new(
                    "late-IR type definitions are not strictly sorted",
                ));
            }
            previous = Some(descriptor.id);
            if self.index.get(&descriptor.id) != Some(&(position as u32)) {
                return Err(VerifyError::new(format!(
                    "type table index for {:032x} does not match its position",
                    descriptor.id.0
                )));
            }
            self.verify_descriptor(descriptor)?;
        }
        Ok(())
    }

    fn verify_descriptor(&self, descriptor: &TypeDescriptor) -> Result<(), VerifyError> {
        let check = |id: TypeId| self.require(id, "type definition").map(|_| ());
        let check_fields =
            |fields: &[FieldDescriptor]| fields.iter().try_for_each(|field| check(field.ty));
        match &descriptor.kind {
            TypeKind::Tuple(elements) => elements.iter().try_for_each(|id| check(*id))?,
            TypeKind::Array { element, .. }
            | TypeKind::Slice(element)
            | TypeKind::Reference {
                pointee: element, ..
            }
            | TypeKind::RawPointer {
                pointee: element, ..
            }
            | TypeKind::Pattern(element) => check(*element)?,
            TypeKind::Adt {
                args,
                form,
                variants,
                ..
            } => {
                args.iter().try_for_each(|id| check(*id))?;
                for variant in variants {
                    check_fields(&variant.fields)?;
                }
                let tagged = variants
                    .iter()
                    .filter(|variant| variant.physical_tag.is_some())
                    .count();
                let result_ok = variants.iter().filter(|variant| variant.result_ok).count();
                if *form == NominalForm::Enum {
                    if tagged != 0 && tagged != variants.len() {
                        return Err(VerifyError::new(
                            "late-IR enum variants have mixed physical direct-tag facts",
                        ));
                    }
                    if variants
                        .iter()
                        .filter_map(|variant| variant.physical_tag)
                        .any(|discriminant| !discriminant.is_valid())
                    {
                        return Err(VerifyError::new(
                            "late-IR enum variant has an invalid physical direct tag",
                        ));
                    }
                    if result_ok > 1 {
                        return Err(VerifyError::new(
                            "late-IR enum has more than one canonical Result::Ok variant",
                        ));
                    }
                } else {
                    if tagged != 0 || result_ok != 0 {
                        return Err(VerifyError::new(
                            "late-IR non-enum variant has enum-only semantic metadata",
                        ));
                    }
                    if variants.len() > 1 {
                        return Err(VerifyError::new(
                            "late-IR struct or union has more than one variant",
                        ));
                    }
                }
            }
            TypeKind::Closure { upvars, .. } | TypeKind::CoroutineClosure { upvars, .. } => {
                upvars.iter().try_for_each(|id| check(*id))?;
            }
            TypeKind::Coroutine {
                upvars,
                saved_locals,
                states,
                variants,
                ..
            } => {
                upvars.iter().try_for_each(|id| check(*id))?;
                saved_locals.iter().try_for_each(|(_, id)| check(*id))?;
                if states.len() != variants.len() {
                    return Err(VerifyError::new(
                        "late-IR coroutine states and variants disagree in count",
                    ));
                }
                for (state, variant) in states.iter().zip(variants) {
                    if state.len() != variant.fields.len()
                        || variant.saved_local_keys.len() != variant.fields.len()
                    {
                        return Err(VerifyError::new(
                            "late-IR coroutine state fields and saved locals disagree in count",
                        ));
                    }
                    for (saved, field) in state.iter().zip(&variant.fields) {
                        let Some((_, ty)) = saved_locals.get(*saved as usize) else {
                            return Err(VerifyError::new(
                                "late-IR coroutine state names a missing saved local",
                            ));
                        };
                        if *ty != field.ty {
                            return Err(VerifyError::new(
                                "late-IR coroutine state field type differs from its saved local",
                            ));
                        }
                    }
                }
            }
            TypeKind::Bool
            | TypeKind::Char
            | TypeKind::Integer { .. }
            | TypeKind::Float { .. }
            | TypeKind::Str
            | TypeKind::Never
            | TypeKind::Opaque
            | TypeKind::FnDef { .. }
            | TypeKind::FnPtr { .. }
            | TypeKind::Dynamic { .. }
            | TypeKind::Foreign(_) => {}
        }
        check(descriptor.js.peeled)?;
        if let Some(inner) = descriptor.js.transparent {
            check(inner.ty)?;
        }
        if let Some(tail) = descriptor.js.unsized_tail {
            check(tail.ty)?;
        }
        check_fields(&descriptor.js.fields)?;
        if descriptor.js.is_primitive_enum
            && !matches!(
                descriptor.js.enum_repr,
                Some(EnumRepr::Number { .. } | EnumRepr::Tag { fieldless: true })
            )
        {
            return Err(VerifyError::new(
                "late-IR primitive enum fact contradicts its enum representation",
            ));
        }
        Ok(())
    }
}
