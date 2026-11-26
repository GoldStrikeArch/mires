//! JIR-S: the semantic, MIR-shaped, representation-neutral body.
//!
//! This is v1's `late_ir.rs` schema (11) moved out of the rustc dylib and extended. The executable
//! body carries stable semantic identities and no source paths, spans, or text: strings are
//! [`StrId`]s into the crate-scope [`super::interns::Interns`], types are [`TypeId`]s into the
//! crate-scope [`super::types::TypeTable`], and everything positional (source facts, local names,
//! inline stacks) lives in the separate [`super::origin::OriginTable`]. Two crates that describe
//! one monomorphic item through different paths therefore produce **equal** bodies, and `Body`'s
//! derived `PartialEq`/`Eq` is semantic equality.
//!
//! Every enum is **append-only**: postcard serializes a variant by index, so a variant may be added
//! at the end and never reordered, renamed in place, or removed. A change to a variant's payload is
//! a schema bump ([`super::JIR_SCHEMA`]).

use serde::{Deserialize, Serialize};

use super::interns::{BytesId, StrId};
use super::origin::SourceFactId;
pub use super::types::TypeId;
use super::types::{TypeKind, TypeTable};

/// Stable identity of a monomorphic Rust item.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ItemId {
    pub definition: DefinitionId,
    /// Stable hash of the monomorphic item type, including generic arguments.
    pub instantiation: u128,
}

/// Stable identity of a Rust definition across crate and compilation-session boundaries
/// (`DefPathHash`).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct DefinitionId {
    pub crate_id: u64,
    pub local_hash: u64,
}

/// An exact direct Rust item together with the JavaScript identifier emitted for it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct DirectItemRef {
    pub item: ItemId,
    pub link_name: StrId,
}

/// An executable function CFG, a typed static with its initializer, or a vtable.
///
/// Semantic content only. `Statement` indices, source facts, and names live in the origin table.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Body {
    pub item: ItemId,
    pub kind: BodyKind,
    pub semantic: BodySemantic,
    pub signature: Signature,
    /// Indexed by [`LocalId`]. Empty for statics and vtables.
    pub locals: Vec<Local>,
    /// Indexed by [`BlockId`]. Empty for statics and vtables.
    pub blocks: Vec<BasicBlock>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BodyKind {
    /// Append-only: postcard serializes this enum by index.
    Function,
    /// A `static` with its evaluated initializer decoded into a constant tree (v1 kept the
    /// initializer in the printed JavaScript only). `signature.result` is the static's type.
    Static { init: ConstTree, mutable: bool },
    /// The vtable of `concrete as dyn trait_ref`: one entry per `tcx.vtable_entries` slot in
    /// rustc's own order (drop, size, align, then methods), so `Callee::Virtual { slot }` indexes
    /// it directly. `trait_ref` is `None` for a `dyn Send`-shaped object, whose vtable is the
    /// three header entries only.
    Vtable {
        concrete: TypeId,
        trait_ref: Option<DefinitionId>,
        entries: Vec<VtableEntry>,
    },
}

impl BodyKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Static { .. } => "static",
            Self::Vtable { .. } => "vtable",
        }
    }
}

/// One slot of a vtable array.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum VtableEntry {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// Slot 0: the drop glue of the concrete type, or `None` (`null`) when it needs none or its
    /// glue is empty (empty glue is not emitted as an item, so naming it would dangle).
    Drop(Option<DirectItemRef>),
    /// Slot 1: the size in bytes.
    Size(u64),
    /// Slot 2: the alignment in bytes.
    Align(u64),
    /// An already resolved method instance (a default method reached through `dyn` is the
    /// trait's own body with `Self = dyn Trait`; never re-resolve it).
    Method(DirectItemRef),
    /// A method the trait object cannot dispatch; the slot only keeps the numbering right.
    Vacant,
    /// A supertrait vtable for upcasting: the link name of that vtable item.
    TraitVPtr(StrId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Signature {
    pub arguments: Vec<TypeId>,
    pub result: TypeId,
    pub abi: AbiSignature,
}

/// The JavaScript parameter shape MIR lowering gives this Rust body.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiSignature {
    /// The JavaScript parameter count: MIR arguments, with a spread tuple counted as its arity
    /// and the caller `Location` counted when `caller_location` is set.
    pub parameters: u16,
    pub spread: Option<SpreadArgument>,
    /// Whether the body takes a trailing caller `Location` (`#[track_caller]`).
    pub caller_location: bool,
    /// Whether Rust marked the item `#[inline(never)]`.
    pub inline_never: bool,
    /// Whether Rust marked the item `#[inline]`.
    pub inline_hint: bool,
    /// Whether Rust marked the item `#[inline(always)]` (or `#[rustc_force_inline]`).
    pub inline_always: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpreadArgument {
    pub local: LocalId,
    pub arity: u16,
}

/// A body identity established from rustc definitions rather than paths or JavaScript names.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BodySemantic {
    /// Append-only: postcard serializes this enum by index.
    Ordinary,
    Standard(StandardOperation),
    DropGlue(TypeId),
    /// The crate graph's `#[panic_handler]` (`LangItem::PanicImpl`).
    PanicImpl,
}

/// Standard-library operation families eligible for native representation analysis.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum StandardOperation {
    /// Append-only: postcard serializes this enum by index.
    VecNew,
    VecWithCapacity,
    VecLen,
    VecCapacity,
    VecPush,
    VecClear,
    VecReserve,
    VecAsSlice,
    VecAsMutSlice,
    VecPop,
    VecIsEmpty,
    VecExtend,
    VecRemove,
    VecSwap,
    VecIndex,
    VecIndexMut,
    VecDrop,
    StringNew,
    StringWithCapacity,
    StringLen,
    StringCapacity,
    StringPush,
    StringPushStr,
    StringClear,
    StringAsStr,
    StringDrop,
    SliceSwap,
    StringFmtWriteStr,
    StringFmtWriteChar,
    StringFmtWriteFmt,
    StringFmtSpecWriteFmt,
    F64SpecToString,
    IntToString,
    BoolToString,
    CharToString,
    /// `Argument::new_display` for a proven default-Display primitive, `&str`, `String`, or `f64`.
    FormatArgumentNewDisplay,
    /// `Arguments::new` for a compiler-generated `format_args!` template.
    FormatArgsNew,
    /// The exact `fn(Arguments) -> String` that owns a formatted result.
    FormatArgsToString,
    /// Compiler-generated drop glue for a default-allocator `Box<T>`.
    BoxDrop,
    // The `str` call-site identities (`js-str=bytes`, experimental). The producer tags *calls*
    // with them, never bodies; the link step reads them only under `js-str=bytes` and turns them
    // back into `CallSemantic::Ordinary` otherwise ([`StandardOperation::is_str`]).
    /// `str::len`.
    StrLen,
    /// `str::is_char_boundary`.
    StrIsCharBoundary,
    /// `str::starts_with` with a `&str` or `char` pattern.
    StrStartsWith,
    /// `str::ends_with` with a `&str` or `char` pattern.
    StrEndsWith,
    /// `str::find` with a `&str` or `char` pattern.
    StrFind,
    /// `str::rfind` with a `&str` or `char` pattern.
    StrRFind,
    /// `str::contains` with a `&str` or `char` pattern.
    StrContains,
    /// `<str as Index<I>>::index` for `I` one of `Range`, `RangeFrom`, `RangeTo`, `RangeFull`
    /// of `usize`.
    StrIndex,
    /// `<str as PartialEq>::eq`.
    StrEq,
    /// `<str as Ord>::cmp`.
    StrCmp,
    /// `str::strip_prefix` with a `&str` or `char` pattern.
    StrStripPrefix,
    /// `str::strip_suffix` with a `&str` or `char` pattern.
    StrStripSuffix,
    /// `str::trim`.
    StrTrim,
    /// `str::trim_start`.
    StrTrimStart,
    /// `str::trim_end`.
    StrTrimEnd,
    /// `str::get` with `Range`, `RangeFrom`, `RangeTo`, or `RangeFull` of `usize`.
    StrGet,
    /// `<str as PartialOrd>::partial_cmp`.
    StrPartialCmp,
    /// `str::is_ascii`.
    StrIsAscii,
    /// `<[u8]>::is_ascii`, the byte-slice method `str::is_ascii` is written with. It is a `str`
    /// call-site identity for the one reason the others are: only `js-str=bytes` lowers it.
    SliceIsAscii,
    /// `Argument::new_lower_hex` for a proven integer type.
    FormatArgumentNewLowerHex,
    /// `Argument::new_upper_hex` for a proven integer type.
    FormatArgumentNewUpperHex,
    /// `<f64 as core::str::FromStr>::from_str`.
    F64FromStr,
    /// `<f32 as core::str::FromStr>::from_str`.
    F32FromStr,
    /// `core::slice::sort::{stable,unstable}::sort` (`rcgjs_slice_{stable,unstable}_sort`).
    SliceSort,
    /// `core::slice::sort::shared::rcgjs_sort_cmp` (`rcgjs_slice_sort_cmp`).
    SliceSortCmp,
    /// `alloc::str::to_lowercase` (`rcgjs_str_to_lowercase`).
    StrToLowercase,
    /// `alloc::str::to_uppercase` (`rcgjs_str_to_uppercase`).
    StrToUppercase,
    /// `core::str::iter::rcgjs_str_byte_at` (`js-str=bytes` direct `charCodeAt` read).
    StrByteAt,
    /// `core::str::iter::rcgjs_str_slice_unchecked` (`js-str=bytes` direct `slice` subslice).
    StrSliceUnchecked,
    /// `Vec::push_mut(&mut self, value: T) -> &mut T`.
    VecPushMut,
    /// `Vec::reserve_exact(&mut self, additional: usize)`.
    VecReserveExact,
    /// `Vec::shrink_to_fit(&mut self)`.
    VecShrinkToFit,
    /// `Vec::truncate(&mut self, len: usize)`.
    VecTruncate,
    /// `Vec::resize(&mut self, new_len: usize, value: T)`.
    VecResize,
    /// `Vec::swap_remove(&mut self, index: usize) -> T`.
    VecSwapRemove,
    /// `alloc::vec::from_elem(elem: T, n: usize) -> Vec<T>`.
    VecFromElem,
    /// `Vec::extend_from_slice(&mut self, other: &[T])`.
    VecExtendFromSlice,
    /// `<Vec<T> as Clone>::clone(&self) -> Vec<T>`.
    VecClone,
    /// `<[T]>::to_vec(&self) -> Vec<T>`.
    SliceToVec,
    /// `<Vec<T> as Index<R>>::index(&self, range: R) -> &[T]` for `Range`/`RangeFrom`/`RangeTo`/`RangeFull` of `usize`.
    VecRangeIndex,
    /// `<Vec<T> as IndexMut<R>>::index_mut(&mut self, range: R) -> &mut [T]` for `Range`/`RangeFrom`/`RangeTo`/`RangeFull` of `usize`.
    VecRangeIndexMut,
    /// `alloc::boxed::box_assume_init_into_vec_unsafe(b: Box<MaybeUninit<[T; N]>>) -> Vec<T>`.
    VecFromBoxArray,
    /// `<&Vec<T> as IntoIterator>::into_iter(&self) -> core::slice::Iter<T>`.
    VecIter,
    /// `<&mut Vec<T> as IntoIterator>::into_iter(&mut self) -> core::slice::IterMut<T>`.
    VecIterMut,
    /// `core::mem::replace(&mut Vec<T>, Vec<T>) -> Vec<T>`.
    VecReplace,
    /// `core::mem::swap(&mut Vec<T>, &mut Vec<T>)`.
    VecMemSwap,
    /// `Vec::insert(&mut self, index: usize, element: T)`.
    VecInsert,
    /// `Vec::insert_mut(&mut self, index: usize, element: T) -> &mut T`.
    VecInsertMut,
    /// `BTreeMap::new() -> BTreeMap<K, V>` / `<BTreeMap<K, V> as Default>::default()`.
    BTreeMapNew,
    /// `BTreeMap::len(&self) -> usize`.
    BTreeMapLen,
    /// `BTreeMap::is_empty(&self) -> bool`.
    BTreeMapIsEmpty,
    /// `BTreeMap::clear(&mut self)`.
    BTreeMapClear,
    /// `BTreeMap::contains_key<Q>(&self, key: &Q) -> bool`.
    BTreeMapContainsKey,
    /// `BTreeMap::get<Q>(&self, key: &Q) -> Option<&V>`.
    BTreeMapGet,
    /// `BTreeMap::get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>`.
    BTreeMapGetMut,
    /// `BTreeMap::insert(&mut self, key: K, value: V) -> Option<V>`.
    BTreeMapInsert,
    /// `BTreeMap::remove<Q>(&mut self, key: &Q) -> Option<V>`.
    BTreeMapRemove,
    /// `<BTreeMap<K, V> as Drop>::drop(&mut self)`.
    BTreeMapDrop,
    /// `alloc::alloc::rcgjs_typed_alloc<T>(cap: usize, zeroed: bool) -> *mut T`.
    TypedAlloc,
    /// `alloc::alloc::rcgjs_typed_realloc<T>(ptr: *mut T, old_cap: usize, new_cap: usize) -> *mut T`.
    TypedRealloc,
    /// `alloc::alloc::rcgjs_typed_dealloc<T>(ptr: *mut T, cap: usize)`.
    TypedDealloc,
    /// `alloc::alloc::rcgjs_box_new<T>(x: T) -> *mut T`.
    BoxNew,
    /// `alloc::alloc::rcgjs_typed_dealloc_raw(ptr: *mut u8, layout: Layout)`.
    TypedDeallocRaw,
    /// `alloc::string::rcgjs_str_new() -> String`.
    NativeStrNew,
    /// `alloc::string::rcgjs_str_with_capacity(cap: usize) -> String`.
    NativeStrWithCapacity,
    /// `alloc::string::rcgjs_str_from_str(s: &str) -> String`.
    NativeStrFromStr,
    /// `alloc::string::rcgjs_str_as_str(s: &String) -> &str`.
    NativeStrAsStr,
    /// `alloc::string::rcgjs_str_len(s: &String) -> usize`.
    NativeStrLen,
    /// `alloc::string::rcgjs_str_capacity(s: &String) -> usize`.
    NativeStrCapacity,
    /// `alloc::string::rcgjs_str_push_str(s: &mut String, rhs: &str)`.
    NativeStrPushStr,
    /// `alloc::string::rcgjs_str_push(s: &mut String, ch: char)`.
    NativeStrPush,
    /// `alloc::string::rcgjs_str_clear(s: &mut String)`.
    NativeStrClear,
    /// `alloc::string::rcgjs_str_truncate_unchecked(s: &mut String, new_len: usize)`.
    NativeStrTruncate,
    /// `alloc::string::rcgjs_str_reserve(s: &mut String, additional: usize)`.
    NativeStrReserve,
    /// `alloc::string::rcgjs_str_shrink_to(s: &mut String, min_capacity: usize)`.
    NativeStrShrinkTo,
    /// `alloc::string::rcgjs_str_from_vec_unchecked(bytes: Vec<u8>) -> String`.
    NativeStrFromVec,
    /// `alloc::string::rcgjs_str_into_bytes(s: String) -> Vec<u8>`.
    NativeStrIntoBytes,
    /// `alloc::string::rcgjs_str_replace_slice(s: &mut String, start: usize, end: usize, rep: &str)`.
    NativeStrReplaceSlice,
    /// `alloc::string::rcgjs_str_extend_within(s: &mut String, start: usize, end: usize)`.
    NativeStrExtendWithin,
    /// `alloc::string::rcgjs_str_clone_from(dst: &mut String, src: &String)`.
    NativeStrCloneFrom,
    /// `alloc::string::rcgjs_str_to_ascii_lowercase(s: &str) -> String`.
    NativeStrToAsciiLowercase,
    /// `alloc::string::rcgjs_str_to_ascii_uppercase(s: &str) -> String`.
    NativeStrToAsciiUppercase,
    /// `alloc::string::rcgjs_str_repeat(s: &str, n: usize) -> String`.
    NativeStrRepeat,
    /// `core::str::traits::rcgjs_str_eq(a: &str, b: &str) -> bool`.
    BstrEq,
    /// `core::str::traits::rcgjs_str_cmp(a: &str, b: &str) -> Ordering`.
    BstrCmp,
    /// `core::str::pattern::rcgjs_str_starts_with(haystack: &str, needle: &str) -> bool`.
    BstrStartsWith,
    /// `core::str::pattern::rcgjs_str_ends_with(haystack: &str, needle: &str) -> bool`.
    BstrEndsWith,
    /// `core::str::pattern::rcgjs_str_contains(haystack: &str, needle: &str) -> bool`.
    BstrContains,
    /// `core::str::pattern::rcgjs_str_find_char(haystack: &str, start: usize, end: usize, needle: char) -> usize`.
    BstrFindChar,
    /// `core::str::pattern::rcgjs_str_rfind_char(haystack: &str, start: usize, end: usize, needle: char) -> usize`.
    BstrRfindChar,
    /// `core::str::pattern::rcgjs_str_find_str(haystack: &str, start: usize, end: usize, needle: &str) -> usize`.
    BstrFindStr,
    /// `core::str::pattern::rcgjs_str_rfind_str(haystack: &str, start: usize, end: usize, needle: &str) -> usize`.
    BstrRfindStr,
    /// `alloc::rc::rcgjs_rc_alloc_slice<T>(len: usize) -> *mut RcInner<[T]>`.
    RcAllocSlice,
    /// `alloc::sync::rcgjs_arc_alloc_slice<T>(len: usize) -> *mut ArcInner<[T]>`.
    ArcAllocSlice,
    /// `alloc::vec::rcgjs_vec_new<T>() -> Vec<T>`.
    NativeVecNew,
    /// `alloc::vec::rcgjs_vec_with_capacity<T>(cap: usize) -> Vec<T>`.
    NativeVecWithCapacity,
    /// `alloc::vec::rcgjs_vec_from_elem_zeroed<T>(n: usize) -> Vec<T>`.
    NativeVecFromElemZeroed,
    /// `alloc::vec::rcgjs_vec_from_array<T, const N: usize>(arr: [T; N]) -> Vec<T>`.
    NativeVecFromArray,
    /// `alloc::vec::rcgjs_vec_from_raw_parts<T>(ptr: *mut T, len: usize, cap: usize) -> Vec<T>`.
    NativeVecFromRawParts,
    /// `alloc::vec::rcgjs_vec_capacity<T>(v: &Vec<T>) -> usize`.
    NativeVecCapacity,
    /// `alloc::vec::rcgjs_vec_as_ptr<T>(v: &Vec<T>) -> *const T`.
    NativeVecAsPtr,
    /// `alloc::vec::rcgjs_vec_as_mut_ptr<T>(v: &mut Vec<T>) -> *mut T`.
    NativeVecAsMutPtr,
    /// `alloc::vec::rcgjs_vec_as_slice<T>(v: &Vec<T>) -> &[T]`.
    NativeVecAsSlice,
    /// `alloc::vec::rcgjs_vec_as_mut_slice<T>(v: &mut Vec<T>) -> &mut [T]`.
    NativeVecAsMutSlice,
    /// `alloc::vec::rcgjs_vec_reserve<T>(v: &mut Vec<T>, used_len: usize, add: usize)`.
    NativeVecReserve,
    /// `alloc::vec::rcgjs_vec_reserve_exact<T>(v: &mut Vec<T>, used_len: usize, add: usize)`.
    NativeVecReserveExact,
    /// `alloc::vec::rcgjs_vec_try_reserve<T>(v: &mut Vec<T>, used_len: usize, add: usize) -> bool`.
    NativeVecTryReserve,
    /// `alloc::vec::rcgjs_vec_try_reserve_exact<T>(v: &mut Vec<T>, used_len: usize, add: usize) -> bool`.
    NativeVecTryReserveExact,
    /// `alloc::vec::rcgjs_vec_shrink_to<T>(v: &mut Vec<T>, target_cap: usize)`.
    NativeVecShrinkTo,
    /// `alloc::vec::rcgjs_vec_try_shrink_to<T>(v: &mut Vec<T>, target_cap: usize) -> bool`.
    NativeVecTryShrinkTo,
    /// `alloc::vec::rcgjs_vec_grow_one<T>(v: &mut Vec<T>)`.
    NativeVecGrowOne,
    /// `alloc::vec::rcgjs_vec_push<T>(v: &mut Vec<T>, val: T)`.
    NativeVecPush,
    /// `alloc::vec::rcgjs_vec_push_mut<T>(v: &mut Vec<T>, val: T) -> &mut T`.
    NativeVecPushMut,
    /// `alloc::vec::rcgjs_vec_pop<T>(v: &mut Vec<T>) -> Option<T>`.
    NativeVecPop,
    /// `alloc::vec::rcgjs_vec_into_boxed_slice<T>(v: Vec<T>) -> Box<[T]>`.
    NativeVecIntoBoxedSlice,
}

impl StandardOperation {
    /// Whether this is one of the Phase 3 typed-heap `Global` allocation/deallocation hooks.
    pub fn is_typed_heap(self) -> bool {
        matches!(
            self,
            Self::TypedAlloc
                | Self::TypedRealloc
                | Self::TypedDealloc
                | Self::BoxNew
                | Self::TypedDeallocRaw
                | Self::RcAllocSlice
                | Self::ArcAllocSlice
        )
    }

    /// Whether this is one of the Phase 5 native `{ s, cap }` `String` hooks.
    pub fn is_native_string(self) -> bool {
        matches!(
            self,
            Self::NativeStrNew
                | Self::NativeStrWithCapacity
                | Self::NativeStrFromStr
                | Self::NativeStrAsStr
                | Self::NativeStrLen
                | Self::NativeStrCapacity
                | Self::NativeStrPushStr
                | Self::NativeStrPush
                | Self::NativeStrClear
                | Self::NativeStrTruncate
                | Self::NativeStrReserve
                | Self::NativeStrShrinkTo
                | Self::NativeStrFromVec
                | Self::NativeStrIntoBytes
                | Self::NativeStrReplaceSlice
                | Self::NativeStrExtendWithin
                | Self::NativeStrCloneFrom
                | Self::NativeStrToAsciiLowercase
                | Self::NativeStrToAsciiUppercase
                | Self::NativeStrRepeat
        )
    }

    /// Whether this is one of the universal flat `Vec<T, Global>` hooks.
    pub fn is_native_vec(self) -> bool {
        matches!(
            self,
            Self::NativeVecNew
                | Self::NativeVecWithCapacity
                | Self::NativeVecFromElemZeroed
                | Self::NativeVecFromArray
                | Self::NativeVecFromRawParts
                | Self::NativeVecCapacity
                | Self::NativeVecAsPtr
                | Self::NativeVecAsMutPtr
                | Self::NativeVecAsSlice
                | Self::NativeVecAsMutSlice
                | Self::NativeVecReserve
                | Self::NativeVecReserveExact
                | Self::NativeVecTryReserve
                | Self::NativeVecTryReserveExact
                | Self::NativeVecShrinkTo
                | Self::NativeVecTryShrinkTo
                | Self::NativeVecGrowOne
                | Self::NativeVecPush
                | Self::NativeVecPushMut
                | Self::NativeVecPop
                | Self::NativeVecIntoBoxedSlice
        )
    }

    /// Whether this is one of the Phase 5 `core::str` host-delegated comparison/search hooks.
    pub fn is_fast_str_op(self) -> bool {
        matches!(
            self,
            Self::BstrEq
                | Self::BstrCmp
                | Self::BstrStartsWith
                | Self::BstrEndsWith
                | Self::BstrContains
                | Self::BstrFindChar
                | Self::BstrRfindChar
                | Self::BstrFindStr
                | Self::BstrRfindStr
        )
    }

    /// Whether this is one of the `str` call-site identities (with `<[u8]>::is_ascii`).
    pub fn is_str(self) -> bool {
        matches!(
            self,
            Self::StrLen
                | Self::StrIsCharBoundary
                | Self::StrStartsWith
                | Self::StrEndsWith
                | Self::StrFind
                | Self::StrRFind
                | Self::StrContains
                | Self::StrIndex
                | Self::StrEq
                | Self::StrCmp
                | Self::StrStripPrefix
                | Self::StrStripSuffix
                | Self::StrTrim
                | Self::StrTrimStart
                | Self::StrTrimEnd
                | Self::StrGet
                | Self::StrPartialCmp
                | Self::StrIsAscii
                | Self::SliceIsAscii
                | Self::StrByteAt
                | Self::StrSliceUnchecked
        )
    }
}

impl Body {
    /// Turns every `str` call-site identity back into an ordinary call: what the link step does
    /// unless it holds `str` as byte strings, so every other mode sees the bodies it always saw.
    pub fn forget_str_calls(&mut self) {
        for block in &mut self.blocks {
            if let Terminator::Call(call) = &mut block.terminator {
                if matches!(call.semantic, CallSemantic::Standard(operation) if operation.is_str())
                {
                    call.semantic = CallSemantic::Ordinary;
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct LocalId(pub u32);

impl crate::index_vec::Idx for LocalId {
    fn new(idx: usize) -> Self {
        Self(u32::try_from(idx).expect("LocalId overflows u32"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Local {
    pub id: LocalId,
    pub role: LocalRole,
    pub ty: TypeId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LocalRole {
    /// Append-only: postcard serializes this enum by index.
    Return,
    Argument,
    Temporary,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct BlockId(pub u32);

impl crate::index_vec::Idx for BlockId {
    fn new(idx: usize) -> Self {
        Self(u32::try_from(idx).expect("BlockId overflows u32"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BasicBlock {
    pub id: BlockId,
    pub cleanup: bool,
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

/// One statement. Its original MIR index is `OriginTable::statements`; nops are kept so the
/// positions stay stable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Statement {
    pub kind: StatementKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum StatementKind {
    /// Append-only: postcard serializes this enum by index.
    Assign {
        destination: Place,
        value: Rvalue,
    },
    SetDiscriminant {
        place: Place,
        variant: u32,
    },
    StorageLive(LocalId),
    StorageDead(LocalId),
    Assume(Operand),
    CopyNonOverlapping {
        source: Operand,
        destination: Operand,
        count: Operand,
    },
    Nop,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Rvalue {
    /// Append-only: postcard serializes this enum by index.
    Use(Operand),
    Repeat {
        value: Operand,
        count: u64,
    },
    Reference {
        kind: BorrowKind,
        place: Place,
    },
    RawPointer {
        kind: RawPointerKind,
        place: Place,
    },
    Cast {
        kind: CastKind,
        value: Operand,
        result: TypeId,
    },
    Binary {
        op: BinaryOp,
        left: Operand,
        right: Operand,
    },
    Unary {
        op: UnaryOp,
        value: Operand,
    },
    Discriminant(Place),
    Aggregate {
        kind: AggregateKind,
        result: TypeId,
        operands: Vec<Operand>,
    },
    CopyForDeref(Place),
    WrapUnsafeBinder(Operand),
    Reborrow {
        mutable: bool,
        place: Place,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AggregateKind {
    /// Append-only: postcard serializes this enum by index.
    Array(TypeId),
    Tuple,
    Adt {
        definition: DefinitionId,
        variant: u32,
        active_field: Option<u32>,
    },
    Closure(DefinitionId),
    Coroutine(DefinitionId),
    CoroutineClosure(DefinitionId),
    RawPointer {
        pointee: TypeId,
        mutable: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BorrowKind {
    /// Append-only: postcard serializes this enum by index.
    Shared,
    FakeShallow,
    FakeDeep,
    Mutable,
    TwoPhase,
    ClosureCapture,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RawPointerKind {
    /// Append-only: postcard serializes this enum by index.
    Const,
    Mutable,
    FakeForMetadata,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CastKind {
    /// Append-only: postcard serializes this enum by index.
    PointerExposeProvenance,
    PointerWithExposedProvenance,
    PointerCoercion,
    IntToInt,
    FloatToInt,
    FloatToFloat,
    IntToFloat,
    PtrToPtr,
    FnPtrToPtr,
    Transmute,
    Subtype,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum UnaryOp {
    /// Append-only: postcard serializes this enum by index.
    Not,
    Neg,
    PtrMetadata,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BinaryOp {
    /// Append-only: postcard serializes this enum by index.
    Add,
    AddUnchecked,
    AddWithOverflow,
    Sub,
    SubUnchecked,
    SubWithOverflow,
    Mul,
    MulUnchecked,
    MulWithOverflow,
    Div,
    Rem,
    BitXor,
    BitAnd,
    BitOr,
    Shl,
    ShlUnchecked,
    Shr,
    ShrUnchecked,
    Eq,
    Lt,
    Le,
    Ne,
    Ge,
    Gt,
    Cmp,
    Offset,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Operand {
    pub ty: TypeId,
    pub kind: OperandKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OperandKind {
    /// Append-only: postcard serializes this enum by index.
    Copy(Place),
    Move(Place),
    Constant(Constant),
    /// A `cfg!(debug_assertions)`/`ub_checks`-style constant the backend decides: the value it
    /// evaluates to.
    RuntimeCheck(bool),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Constant {
    /// Append-only: postcard serializes this enum by index.
    ZeroSized,
    Scalar {
        bits: u128,
        bytes: u8,
    },
    String(StrId),
    Pointer {
        target: GlobalReference,
        offset: u64,
    },
    Memory(ConstTree),
}

/// One constant value decoded out of anonymous immutable memory.
///
/// The tree mirrors the printed JavaScript value model: a struct, tuple, or array is its field
/// values in declaration order, a transparent wrapper is its one non-zero-sized field, an enum is
/// a variant index with that variant's field values, and a reference into anonymous memory is the
/// decoded pointee value. Scalar bits are interpreted through the Rust type at each position.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ConstTree {
    /// Append-only: postcard serializes this enum by index.
    Unit,
    Scalar {
        bits: u128,
        bytes: u8,
    },
    Str(StrId),
    Aggregate(Vec<ConstTree>),
    Variant {
        variant: u32,
        fields: Vec<ConstTree>,
    },
    Reference(Box<ConstTree>),
    Slice(Vec<ConstTree>),
    Global {
        target: GlobalReference,
        offset: u64,
    },
    Address(u64),
    /// A fat trait-object pointer (`&dyn Trait` / `*const dyn Trait`): its data pointer decoded at
    /// `concrete` paired with the link name of the interned vtable item.
    Dyn {
        data: Box<ConstTree>,
        concrete: TypeId,
        vtable: StrId,
    },
    /// A provenance-free wide slice pointer (`*const [T]` / `*mut [T]` / `&[T]` / `&mut [T]` /
    /// `NonNull<[T]>` / `Box<[T]>`): a bare address paired with its element-count metadata.
    SliceAddress {
        address: u64,
        length: u64,
    },
    /// A pointer into a field or element of a `static` binding (`target`, always
    /// `GlobalReference::Static`), walked step-by-step from the static root.
    ///
    /// Non-empty `path`: whole-static pointers keep [`ConstTree::Global`] unchanged.
    StaticField {
        target: GlobalReference,
        path: Vec<StaticStep>,
    },
    /// A wide slice pointer (`&[T]` / `*const [T]`) into a `static` array (or a nested array
    /// field/element along `path`), starting at element `start` with `length` elements.
    StaticSlice {
        target: GlobalReference,
        path: Vec<StaticStep>,
        start: u32,
        length: u64,
    },
}

/// One JavaScript projection step from a `static` binding down to a nested field or element.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum StaticStep {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// An object property (`base.name` / `{ buf: base, off: "name" }`), already escaped by
    /// `Namer::field_name` (named structs and tuple structs `_0`, `_1`, ...).
    Field(StrId),
    /// A JS array element (`base[index]` / `{ buf: base, off: index }`), for tuples and arrays.
    Index(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum GlobalReference {
    /// Append-only: postcard serializes this enum by index.
    Function {
        item: ItemId,
        link_name: StrId,
    },
    /// A `static`, by its definition and the JavaScript name it is emitted under (v1 asked
    /// `namer.static_name(def_id)`; a link-time consumer has no namer).
    Static {
        definition: DefinitionId,
        link_name: StrId,
    },
    Type(TypeId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub local: LocalId,
    pub projection: Vec<Projection>,
    /// The type of the fully projected place.
    pub ty: TypeId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Projection {
    /// Append-only: postcard serializes this enum by index.
    Deref,
    Field {
        field: u32,
        ty: TypeId,
    },
    Index(LocalId),
    ConstantIndex {
        offset: u64,
        min_length: u64,
        from_end: bool,
    },
    Subslice {
        from: u64,
        to: u64,
        from_end: bool,
    },
    Downcast {
        variant: u32,
    },
    OpaqueCast(TypeId),
    UnwrapUnsafeBinder(TypeId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Terminator {
    /// Append-only: postcard serializes this enum by index.
    Goto(BlockId),
    Switch {
        discriminant: Operand,
        /// `(raw unsigned bit pattern, target)`, as MIR carries them.
        targets: Vec<(u128, BlockId)>,
        otherwise: BlockId,
    },
    Return,
    Unreachable,
    Call(Call),
    Drop {
        place: Place,
        target: BlockId,
        unwind: Unwind,
        replace: bool,
        async_drop: Option<BlockId>,
        glue: DropGlue,
    },
    Assert {
        condition: Operand,
        expected: bool,
        message: AssertMessage,
        target: BlockId,
        unwind: Unwind,
        /// Resolved caller location when this assert came from a MIR-inlined `#[track_caller]`
        /// body or sits inside a non-`#[track_caller]` body; `None` forwards the enclosing `$loc`
        /// parameter (or falls back to the terminator origin for legacy/synthetic bodies).
        track_caller: Option<SourceFactId>,
    },
    UnwindResume,
    UnwindTerminate(TerminateReason),
    FalseEdge {
        real: BlockId,
        imaginary: BlockId,
    },
    FalseUnwind {
        real: BlockId,
        unwind: Unwind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Call {
    pub function: Operand,
    pub callee: Callee,
    pub semantic: CallSemantic,
    pub arguments: Vec<Operand>,
    pub destination: Place,
    pub target: Option<BlockId>,
    pub unwind: Unwind,
    /// `Some(location)` when the callee is `#[track_caller]`: the caller `Location` this call
    /// passes as its trailing argument (the call's own span, or the enclosing body's incoming
    /// location when that body is itself `#[track_caller]` — the encoder resolves which; a
    /// `Some` here is always a concrete fact to spell as a `Location` constant, and a
    /// `#[track_caller]` body forwarding its own parameter records `None` and
    /// `AbiSignature::caller_location`). The fact is the outermost macro expansion of the call's
    /// `fn_span`, with `col` the 0-based display column: the `Location` value's `col` field is
    /// `col + 1`, its `line` the fact's line, its `file` the fact's file. The encoder also
    /// describes the `panic_location` lang item's reference type in the type table whenever it
    /// records a `Some`, so the consumer can spell the value's field keys.
    pub track_caller: Option<SourceFactId>,
    /// The host-side payload of a call the JavaScript lowering intercepts instead of emitting a
    /// call to the callee's body (`CallSemantic::{Host, RawJavaScript, ViewAbi}` and the
    /// `rcgjs.map.*`/`rcgjs.host.*` markers). `None` for an ordinary Rust call.
    pub host: Option<HostPayload>,
}

/// A compiler-recognized call boundary or operation family.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CallSemantic {
    /// Append-only: postcard serializes this enum by index.
    Ordinary,
    GlobalAllocator,
    RawParts,
    Host,
    RawJavaScript,
    Standard(StandardOperation),
}

/// What a call the lowering intercepts carries, decoded from the marker's `link_section` at
/// encode time so the consumer never needs rustc attributes.
///
/// The encoder's convention (W1b): a `#[js_extern]` declaration is `Extern` (the descriptor
/// bytes after the macro's `PREFIX`); a `js!{}` marker is `Block`; a `rcgjs.map.*` marker is
/// `Map { marker }`; a `rcgjs.host.*` marker is `Marker { section: "rcgjs.host.<name>", .. }`
/// (the async markers `spawn`/`futurepromise`/`futurenativepromise` included: whether they are
/// intercepted or compiled as the safe Rust bodies they have is the consumer's mode decision).
/// The `callee` of such a call is still the resolved marker instance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HostPayload {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// A `#[js_extern]` declaration: the descriptor bytes (`js-extern-macro` `Descriptor`,
    /// after its `PREFIX`).
    Extern(BytesId),
    /// A `js!{}` block: the block section bytes (`js-macro` `Block`, after its `PREFIX`).
    Block(BytesId),
    /// A `link_section` marker function: `section` is the full section name (`rcgjs.host.*`),
    /// `def` the marker's definition, and `helpers` the marker's sibling instances (see
    /// [`HostPayload::helpers`]).
    Marker {
        section: StrId,
        def: DefinitionId,
        helpers: Vec<(StrId, DirectItemRef)>,
    },
    /// A `mires-js` collection marker by its short name (the part after `rcgjs.map.`).
    Map { marker: StrId },
}

impl HostPayload {
    /// The **sibling instances** a marker's lowering calls, by the helper's Rust name.
    ///
    /// A marker declares its helpers beside itself in `mires-js` and the lowering resolves one of
    /// them at the marker's own generic argument: `rcgjs.host.spawn` calls `poll_once::<F>` and
    /// `rcgjs.host.futurepromise` calls `poll_result::<F>`. Only rustc can resolve those, so the
    /// producer records them.
    pub fn helpers(&self) -> &[(StrId, DirectItemRef)] {
        match self {
            Self::Marker { helpers, .. } => helpers,
            _ => &[],
        }
    }

    /// The sibling instance named `name`.
    pub fn helper(&self, interns: &super::interns::Interns, name: &str) -> Option<DirectItemRef> {
        self.helpers()
            .iter()
            .find(|(key, _)| interns.str(*key) == Some(name))
            .map(|(_, item)| *item)
    }
}

/// How a declared host-binding parameter observes its argument.
///
/// Computed from the encoded type; it is not a schema field. A sink parameter's JavaScript value
/// is identical under the baseline and native lanes, so it does not pin a String region: scalars
/// pass by value, and a shared `&str` is a JavaScript string in both lanes. Everything else pins.
/// Mutable references would lose host writes under a native carrier; owned `String`/`Vec` values
/// and slices differ between the lanes (the native `Vec` backing array is capacity-length, not
/// len-length); and host handles stay opaque.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostParameterClass {
    Sink,
    Pin,
}

impl HostParameterClass {
    /// Classifies one declared host parameter or result type.
    pub fn of(ty: TypeId, types: &TypeTable) -> Self {
        Self::classify(ty, types, 0)
    }

    fn classify(ty: TypeId, types: &TypeTable, depth: u8) -> Self {
        if depth > 8 {
            return Self::Pin;
        }
        let Some(descriptor) = types.get(ty) else {
            return Self::Pin;
        };
        if let TypeKind::Reference { pointee, mutable } = descriptor.kind {
            // The host can write back through a mutable reference; a native carrier would lose or
            // desynchronize those writes, so only shared references can reach a sink shape.
            if mutable {
                return Self::Pin;
            }
            return Self::classify(pointee, types, depth + 1);
        }
        match &descriptor.kind {
            TypeKind::Str | TypeKind::Bool | TypeKind::Char | TypeKind::Never => Self::Sink,
            TypeKind::Integer { .. } | TypeKind::Float { .. } => Self::Sink,
            TypeKind::Tuple(fields) if fields.is_empty() => Self::Sink,
            _ => Self::Pin,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Callee {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// A resolved monomorphic instance with a body of its own.
    Direct { item: ItemId, link_name: StrId },
    /// A foreign (`extern` block) item with the **Rust** ABI and a plain-identifier symbol: a
    /// Rust symbol some other crate in the program defines (`core`'s `panic_impl`), called
    /// directly by that symbol. Every other foreign item is [`Callee::Runtime`], the `__rt`
    /// print shim member of the same name (`abi.rs`).
    Foreign {
        definition: DefinitionId,
        symbol: StrId,
    },
    /// A call through a function pointer or an unresolvable `FnDef`; `signature` is the type of
    /// the `function` operand.
    Indirect { signature: TypeId },
    /// A compiler intrinsic by name, with the generic type arguments the intrinsic is
    /// instantiated with (in order) and, when the intrinsic has a MIR body the mono collector
    /// emitted, that body as the fallback the lowering calls when it has no native form.
    Intrinsic {
        name: StrId,
        types: Vec<TypeId>,
        fallback: Option<DirectItemRef>,
    },
    /// A virtual call: `slot` is the **absolute** vtable slot (`first_method_vtable_slot` counts
    /// the three header entries), `receiver` the `dyn` type dispatched on.
    Virtual { slot: u32, receiver: TypeId },
    /// A call the lowering emits as `__rt.<member>(...)` with the arguments passed through.
    /// `sysroot` is `true` only when rustc provenance places the foreign item in a sysroot crate.
    Runtime { member: StrId, sysroot: bool },
    /// A call that lowers to nothing at all: `drop_in_place::<T>` for a `T` whose drop glue is
    /// empty (`ShimKind::DropGlue(_, None)`), which is never emitted as an item, so naming it
    /// would dangle. The arguments are still evaluated for their side effects, of which a place
    /// operand has none.
    Nop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DropGlue {
    /// Append-only: postcard serializes this enum by index.
    None,
    Direct {
        item: ItemId,
        link_name: StrId,
    },
    Dynamic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Unwind {
    /// Append-only: postcard serializes this enum by index.
    Continue,
    Unreachable,
    Terminate(TerminateReason),
    Cleanup(BlockId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminateReason {
    /// Append-only: postcard serializes this enum by index.
    Abi,
    InCleanup,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AssertMessage {
    /// Append-only: postcard serializes this enum by index.
    BoundsCheck {
        length: Operand,
        index: Operand,
    },
    Overflow {
        op: BinaryOp,
        left: Operand,
        right: Operand,
    },
    OverflowNeg(Operand),
    DivisionByZero(Operand),
    RemainderByZero(Operand),
    ResumedAfterReturn(CoroutineKind),
    ResumedAfterPanic(CoroutineKind),
    ResumedAfterDrop(CoroutineKind),
    MisalignedPointer {
        required: Operand,
        found: Operand,
    },
    NullPointer,
    NullReference,
    InvalidEnum(Operand),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CoroutineKind {
    /// Append-only: postcard serializes this enum by index.
    Desugared {
        kind: CoroutineDesugaring,
        source: CoroutineSource,
    },
    Coroutine {
        movable: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CoroutineDesugaring {
    /// Append-only: postcard serializes this enum by index.
    Async,
    Gen,
    AsyncGen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CoroutineSource {
    /// Append-only: postcard serializes this enum by index.
    Block,
    Closure,
    Function,
}

impl Body {
    /// Every statement in block-major order, the order `OriginTable::statements` uses.
    pub fn statements(&self) -> impl Iterator<Item = (BlockId, usize, &Statement)> + '_ {
        self.blocks.iter().flat_map(|block| {
            block
                .statements
                .iter()
                .enumerate()
                .map(move |(index, statement)| (block.id, index, statement))
        })
    }

    /// The number of statements across all blocks.
    pub fn statement_count(&self) -> usize {
        self.blocks.iter().map(|block| block.statements.len()).sum()
    }
}
