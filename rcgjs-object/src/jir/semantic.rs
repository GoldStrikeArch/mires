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
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct LocalId(pub u32);

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
    ViewAbi,
    RawJavaScript,
    Standard(StandardOperation),
}

/// What a call the lowering intercepts carries, decoded from the marker's `link_section` at
/// encode time so the consumer never needs rustc attributes.
///
/// The encoder's convention (W1b): a `#[js_extern]` declaration is `Extern` (the descriptor
/// bytes after the macro's `PREFIX`); a `js!{}` marker is `Block`; a `rcgjs.tc.*` marker is
/// `Marker { section: "rcgjs.tc.template", template: Some(..) }` when it is the `template`
/// marker called with a `&STATIC` (the static pre-decoded as a `ConstTree` at its own type),
/// `KeyedList` for the versioned keyed-list boundary, and `ViewAbi { marker }` otherwise; a
/// `rcgjs.map.*` marker is `Map { marker }`; a `rcgjs.host.*` marker is
/// `Marker { section: "rcgjs.host.<name>", template: None }` (the async markers
/// `spawn`/`spawnowner`/`futurepromise` included: whether they are intercepted or compiled as the
/// safe Rust bodies they have is the consumer's mode decision, as in v1). The `callee` of such a
/// call is still the resolved marker instance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HostPayload {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// A `#[js_extern]` declaration: the descriptor bytes (`js-extern-macro` `Descriptor`,
    /// after its `PREFIX`).
    Extern(BytesId),
    /// A `js!{}` block: the block section bytes (`js-macro` `Block`, after its `PREFIX`).
    Block(BytesId),
    /// A `link_section` marker function: `section` is the full section name (`rcgjs.tc.*`,
    /// `rcgjs.host.*`, `rcgjs.map.*`, ...), `def` the marker's definition, `template` the
    /// pre-decoded `view!` template static for the `rcgjs.tc.*` markers that take one, and
    /// `helpers` the marker's sibling instances (see [`HostPayload::helpers`]).
    Marker {
        section: StrId,
        def: DefinitionId,
        template: Option<ConstTree>,
        helpers: Vec<(StrId, DirectItemRef)>,
    },
    /// A `view-abi` marker by its short name (the part after `rcgjs.tc.`), its definition, and
    /// its sibling instances.
    ViewAbi {
        marker: StrId,
        def: DefinitionId,
        helpers: Vec<(StrId, DirectItemRef)>,
    },
    /// A `mires-js` collection marker by its short name (the part after `rcgjs.map.`).
    Map { marker: StrId },
    /// The version-one transactional keyed-list semantic boundary.
    KeyedList(Box<KeyedListPayload>),
}

impl HostPayload {
    /// The **sibling instances** a marker's lowering calls, by the helper's Rust name.
    ///
    /// A marker declares its helpers beside itself in `view-abi`/`mires-js` and the lowering
    /// resolves one of them at the marker's own generic argument: `rcgjs.tc.{text,pushkeyed}`
    /// call `view_abi::display_to_str::<T>` for a value whose `Display` has no in-place
    /// conversion, `rcgjs.host.{spawn,spawnowner}` call `poll_once::<F>` and
    /// `rcgjs.host.futurepromise` calls `poll_result::<F>`. Only rustc can resolve those (v1
    /// `abi.rs::sibling_item`, `host.rs::host_future_helper`), so the producer records them.
    pub fn helpers(&self) -> &[(StrId, DirectItemRef)] {
        match self {
            Self::Marker { helpers, .. } | Self::ViewAbi { helpers, .. } => helpers,
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

/// The keyed-list payload format understood by this schema.
pub const KEYED_LIST_FORMAT_V1: u16 = 1;

/// One closure or function-item value owned by a keyed-list instance.
///
/// `ty` describes the callable value and therefore its captured fields. `body` is the exact
/// monomorphic body when rustc can resolve one, and is `None` for an indirect callable. `cleanup`
/// records how ownership of that value ends. The consumer validates all three facts before it may
/// select an executable keyed lane.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KeyedListCallable {
    pub ty: TypeId,
    pub body: Option<DirectItemRef>,
    pub cleanup: DropGlue,
    pub source: Option<SourceFactId>,
}

/// Representation-neutral facts for one `keyed_list_v1` call.
///
/// The collection, item, and key types are separate because the final representation planner may
/// choose a physical carrier for each independently. Callable captures are described by their
/// [`KeyedListCallable::ty`] descriptors. Body and cleanup references make every rustc-only query
/// explicit in the object rather than asking the linker to reconstruct it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KeyedListPayload {
    pub format: u16,
    pub site: Option<SourceFactId>,
    pub def: DefinitionId,
    pub collection_ty: TypeId,
    pub item_ty: TypeId,
    pub key_ty: TypeId,
    pub row_ty: TypeId,
    pub collection: KeyedListCallable,
    pub key: KeyedListCallable,
    pub row: KeyedListCallable,
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
