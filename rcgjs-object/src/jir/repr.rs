//! JIR-R: the representation-level SSA form one JIR-S body is lowered through on its way to
//! JavaScript.
//!
//! A [`Func`] is a CFG of blocks with block parameters (no phi nodes), each block a straight
//! sequence of [`Op`]s and one [`RTerm`]. Every op that produces a value defines a fresh
//! [`ValueId`] exactly once; the [`ValueDef`] beside it records the value's Rust type, its
//! [`Carrier`] (the JavaScript shape it travels in) and its diagnostic origin. Storage that MIR
//! addressed as a local and that SSA cannot carry (an address-taken primitive, an aggregate written
//! through projections on several paths, a spread argument) is a [`CellDef`] with explicit
//! `CellAlloc`/`CellLoad`/`CellStore` ops. Places are gone: a borrow is a [`OpKind::BorrowView`]
//! from a [`Root`] through a [`Proj`] path, and a read or write through a pointer is
//! [`OpKind::Deref`]/[`OpKind::StoreThrough`].
//!
//! # Value semantics
//!
//! JIR-R keeps Rust's value semantics: an op reads its operands and produces a new value. An
//! aggregate value carries no identity of its own; the one way to alias storage is a pointer value
//! made by `BorrowView`/`RawView` (or a `SliceView`), and the one way to observe storage is
//! `Deref`/`StoreThrough`/`Field`/`SetField`/cell ops on the thing the pointer names.
//! `SetField`/`SetDiscriminant` mutate the storage of their `base` value in place; that is sound
//! because the borrow checker already proved nothing else observes the old contents (a moved-from
//! or overwritten value is dead). `Move(v)` renames `v` and ends its live range; the verifier
//! rejects any later use of `v`. `Copy(v)` is a value duplicate: for a JS object it is v1's
//! `clone_expr`, for a primitive it is nothing. Block arguments alias like `Move`: passing `v` to
//! a block parameter does not duplicate it.
//!
//! The lowerer is what maps this onto JavaScript object identity (`Carrier::Baseline`), and the
//! two interpreters in [`super::interp`] are what check that a lowering from JIR-S preserved the
//! meaning: they evaluate a JIR-S `Body` and a JIR-R `Func` on one abstract heap and compare
//! results and traces.
//!
//! # Schema discipline
//!
//! Every enum here is **append-only** (postcard serializes a variant by index): add at the end,
//! never reorder, rename in place, remove, or change a payload; struct fields are positional. A
//! change that cannot be expressed that way bumps [`super::JIR_SCHEMA`]. JIR-R is derived from
//! JIR-S at link time and is not stored in objects today; the derives are there so a lane dump,
//! a cache, or a test fixture can serialize one.
//!
//! # Carrier restrictions (verified)
//!
//! - Every value's carrier is one [`CarrierPolicy`] allows. [`Func::verify`] allows
//!   [`Carrier::Baseline`] alone, which is what every phase before a committed representation plan
//!   verifies under; [`Func::verify_carriers`] is what a plan that has committed other carriers
//!   verifies its rewrite with, naming the carriers it committed. A carrier no plan committed is
//!   still a verify failure, so a lowering can never be handed a shape it has no emission for.
//! - `Materialize`, `Convert` and `Host` are accepted by the verifier (they type-check) but no
//!   Phase 2 producer emits them; `Convert` between two `Baseline` carriers is rejected as
//!   meaningless.
//! - An intrinsic is always [`OpKind::Intrinsic`]; `Call` with [`Callee::Intrinsic`] is rejected so
//!   a lowerer has one place to dispatch on the name.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::interns::StrId;
use super::origin::{OriginId, SourceFactId};
use super::semantic::{
    BinaryOp, CallSemantic, Callee, CastKind, ConstTree, CoroutineKind, DirectItemRef, DropGlue,
    HostPayload, ItemId, Signature, TypeId, UnaryOp,
};
use super::types::{TypeKind, TypeTable};
use super::verify::VerifyError;

/// An SSA value: an index into [`Func::values`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ValueId(pub u32);

/// A storage cell: an index into [`Func::cells`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct CellId(pub u32);

/// A JIR-R block: an index into [`Func::blocks`]. Block 0 is the entry.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct RBlockId(pub u32);

/// One lowered function.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Func {
    pub item: ItemId,
    /// The Rust signature and JavaScript ABI shape, exactly the JIR-S body's.
    pub sig: Signature,
    /// The parameters in JavaScript ABI order: one per MIR argument, a spread tuple already
    /// counted as its arity by `sig.abi`. Equal to block 0's `params`.
    pub params: Vec<ValueId>,
    /// Indexed by [`ValueId`].
    pub values: Vec<ValueDef>,
    /// Indexed by [`CellId`].
    pub cells: Vec<CellDef>,
    /// Indexed by [`RBlockId`].
    pub blocks: Vec<RBlock>,
}

/// The definition-site facts of one SSA value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValueDef {
    pub ty: TypeId,
    pub carrier: Carrier,
    /// The origin of the op or block parameter defining the value: the local's user name
    /// (`Origin::local_name`) is what the lowerer names a `let` after.
    pub origin: OriginId,
}

/// One storage cell: a MIR local that stays memory rather than becoming SSA values.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CellDef {
    pub ty: TypeId,
    /// Whether the cell is a `[v]` box in the baseline lowering (v1 `uses.rs` `boxed`): an
    /// address-taken direct value whose references need a slot to point at. An unboxed cell is a
    /// plain `let` the lowerer reads and writes by name.
    pub boxed: bool,
    /// The origin of the MIR local the cell stands for.
    pub origin: OriginId,
}

/// The JavaScript shape a value travels in.
///
/// Phase 2 uses `Baseline` only: the v1 ABI shape of the value's Rust type (see the
/// baseline-carrier spec in the crate `README.md`). The rest are reserved for the representation
/// phases; a verifier rejects them until a lowering exists.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum Carrier {
    /// Append-only: postcard serializes this enum by index.
    Baseline,
    /// A virtual (scalar-replaced) aggregate that exists only as its field values.
    Virtual,
    /// A flat, multi-scalar call ABI slot.
    Flat,
    /// A `String` carried as a host string.
    HostString,
    /// A `String` under construction, `{ text, cap, len }`: the text, and its capacity and exact
    /// length in UTF-8 bytes.
    StringBuilder,
    /// A `Vec<T>` as `{ buf, len, cap }` over a native array.
    NativeVec,
    /// A slice as a native array view.
    SliceView,
    /// A value the lowering never inspects.
    Opaque,
    /// A default-allocator `Box<T>` whose pointer shape stays canonical while compiler-owned
    /// drop glue releases it without observable allocator or address effects.
    NativeBox,
}

impl Carrier {
    /// This carrier's bit in a [`CarrierPolicy`].
    const fn bit(self) -> u16 {
        1 << (self as u16)
    }
}

/// The carriers a verified [`Func`] may give its values.
///
/// The plan that commits a carrier is what widens this, and it widens it to exactly what it
/// committed: a value carrying a shape nothing planned for is a bug in the plan, and catching it
/// at the commit is what makes the commit transactional rather than a miscompile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CarrierPolicy(u16);

impl CarrierPolicy {
    /// The baseline alone: the v1 ABI shape of every value's Rust type.
    pub const BASELINE: CarrierPolicy = CarrierPolicy(Carrier::Baseline.bit());

    /// Whether a value may carry `carrier`.
    #[must_use]
    pub fn allows(self, carrier: Carrier) -> bool {
        self.0 & carrier.bit() != 0
    }

    /// This policy widened by `carrier`.
    #[must_use]
    pub fn with(self, carrier: Carrier) -> CarrierPolicy {
        CarrierPolicy(self.0 | carrier.bit())
    }
}

/// One block: parameters, straight-line ops, one terminator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RBlock {
    pub id: RBlockId,
    /// The values bound on entry from the incoming edge's arguments.
    pub params: Vec<ValueId>,
    pub ops: Vec<Op>,
    pub term: RTerm,
}

/// One operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Op {
    /// The value defined, for an op that produces one. Which kinds do is fixed by
    /// [`OpKind::produces_value`]; the verifier checks the agreement.
    pub result: Option<ValueId>,
    pub origin: OriginId,
    pub kind: OpKind,
}

/// The operations. See the module documentation for the value semantics.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OpKind {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// A constant at the result's type: a scalar, a string, an aggregate tree, or a global
    /// reference. `ConstTree::Unit` at an aggregate type is that type's zero-sized value.
    Const(ConstTree),
    /// A value duplicate (v1 `clone_expr` when the type `needs_clone`; nothing for a primitive).
    Copy(ValueId),
    /// A rename that ends the operand's live range: the operand is dead after this op.
    Move(ValueId),
    /// A fresh aggregate of type `ty`: struct, tuple, array, closure environment, or the enum
    /// variant `variant` (`0` for a non-enum). `active_field` is the field a union aggregate
    /// initializes; `fields` are the field values in declaration order (one for a union).
    Aggregate {
        ty: TypeId,
        variant: u32,
        active_field: Option<u32>,
        fields: Vec<ValueId>,
    },
    /// An array of `count` copies of `v`.
    Repeat { v: ValueId, count: u64 },
    /// Reads field `field` of `base`, downcast to `variant` first when `Some`. Result type is
    /// the field's type; the JavaScript key is `TypeTable::field(base_ty, variant, field)`.
    Field {
        base: ValueId,
        variant: Option<u32>,
        field: u32,
    },
    /// Writes `v` into field `field` of `base`'s storage, downcast to `variant` first when
    /// `Some`. `base` is a value, not a pointer; to write through a pointer, `Deref`-free code
    /// uses `StoreThrough` with a `BorrowView` path instead.
    SetField {
        base: ValueId,
        variant: Option<u32>,
        field: u32,
        v: ValueId,
    },
    /// The logical discriminant of `base` at the result's integer type.
    Discriminant(ValueId),
    /// Sets the active variant of `base`'s storage, keeping the fields already written.
    SetDiscriminant { base: ValueId, variant: u32 },
    /// Brings `cell` into existence, uninitialized. Executing it again gives the cell fresh
    /// storage; every load, store, or borrow of the cell must be dominated by an alloc.
    CellAlloc(CellId),
    /// Reads the cell's whole value.
    CellLoad(CellId),
    /// Overwrites the cell's whole value.
    CellStore { cell: CellId, v: ValueId },
    /// A reference to the place `root` then `path`. Result type is the reference type; the
    /// baseline lowering picks the [`super::types::PointerForm`] from it.
    BorrowView {
        root: Root,
        path: Vec<Proj>,
        mutable: bool,
    },
    /// A raw pointer to the place `root` then `path`.
    RawView {
        root: Root,
        path: Vec<Proj>,
        mutable: bool,
    },
    /// A slice pointer over `len` elements starting where `ptr` points (a pointer to an element,
    /// to an array, or another slice pointer). Result type is the slice reference or raw pointer
    /// type; from a slice pointer this re-lengthens it (`from_raw_parts` after `RawParts`).
    SliceView { ptr: ValueId, len: ValueId },
    /// Reads the value `ptr` points at.
    Deref(ValueId),
    /// Writes `v` where `ptr` points.
    StoreThrough { ptr: ValueId, v: ValueId },
    /// `l op r` at operand type `ty` (the type of `l`; `r` differs only for shifts and `Offset`).
    /// The result type is the op's: `bool` for comparisons, `Ordering` for `Cmp`, `(ty, bool)`
    /// for the `WithOverflow` ops, `ty` otherwise.
    Binary {
        op: BinaryOp,
        l: ValueId,
        r: ValueId,
        ty: TypeId,
    },
    /// `op v` at operand type `ty`.
    Unary { op: UnaryOp, v: ValueId, ty: TypeId },
    /// `v as to`, `v` being of type `from`. A pointer coercion is told apart by its two types
    /// (array pointer to slice pointer, concrete pointer to `dyn` pointer, closure to `fn`).
    Cast {
        kind: CastKind,
        v: ValueId,
        from: TypeId,
        to: TypeId,
    },
    /// A call. `function` is the callee operand for [`Callee::Indirect`] (the function pointer
    /// value) and `None` otherwise. `result` is `None` when the callee returns `()`, never, or
    /// the result is unused. Exceptions propagate unless this is the block's final
    /// operation and its terminator is [`RTerm::Invoke`].
    Call {
        callee: Callee,
        function: Option<ValueId>,
        args: Vec<ValueId>,
        track_caller: Option<SourceFactId>,
        semantic: CallSemantic,
        host: Option<HostPayload>,
    },
    /// A compiler intrinsic by name with its type arguments and, when it has one, the MIR body
    /// the lowering falls back to.
    Intrinsic {
        name: StrId,
        types: Vec<TypeId>,
        fallback: Option<DirectItemRef>,
        args: Vec<ValueId>,
    },
    /// A host operation with no Rust callee. Invoke/construct markers consume a proven private
    /// fixed array as their last argument, rather than the Rust slice a `Call` consumes.
    /// Other host families remain calls until their payload-only contract is supported.
    Host {
        payload: HostPayload,
        args: Vec<ValueId>,
    },
    /// Runs the drop glue of the place `root` then `path`.
    Drop {
        root: Root,
        path: Vec<Proj>,
        glue: DropGlue,
    },
    /// Panics with `msg` unless `cond == expected`.
    Assert {
        cond: ValueId,
        expected: bool,
        msg: RAssertMessage,
    },
    /// Turns a virtual value into a stored one. Reserved for the representation phases; the
    /// result has the same type as `v`.
    Materialize(ValueId),
    /// Changes the carrier of `v` from `from` to `to`. Reserved for the representation phases.
    Convert {
        v: ValueId,
        from: Carrier,
        to: Carrier,
    },
    /// A construct the lowering could not translate: emits v1's zombie abort at run time. `msg`
    /// is the reason.
    Zombie(StrId),
    /// Reads spill register `index` (preferred name `$r{index}`, allocated unique by the plan):
    /// the extra out-parameter of a flat direct call (`jsc_link::plan::abi_flat`). Registers
    /// are one-based and module-scope `let`s, so a read is only meaningful between the call
    /// that wrote it and the next call that may write it — which is what the `Read` purity in
    /// `lower::fuse::op_purity` says.
    SpillRead(u32),
    /// Writes spill register `index` (preferred name `$r{index}`): how a flat callee returns its
    /// out-fields beyond the first, which the primary return carries.
    SpillWrite { index: u32, v: ValueId },
}

impl OpKind {
    /// Whether an op of this kind defines a value.
    ///
    /// `Call` and `Intrinsic` may or may not (`result` decides); everything else is fixed.
    pub fn produces_value(&self) -> Option<bool> {
        Some(match self {
            Self::Const(_)
            | Self::Copy(_)
            | Self::Move(_)
            | Self::Aggregate { .. }
            | Self::Repeat { .. }
            | Self::Field { .. }
            | Self::Discriminant(_)
            | Self::CellLoad(_)
            | Self::BorrowView { .. }
            | Self::RawView { .. }
            | Self::SliceView { .. }
            | Self::Deref(_)
            | Self::Binary { .. }
            | Self::Unary { .. }
            | Self::Cast { .. }
            | Self::Materialize(_)
            | Self::Convert { .. }
            | Self::SpillRead(_) => true,
            Self::SetField { .. }
            | Self::SetDiscriminant { .. }
            | Self::CellAlloc(_)
            | Self::CellStore { .. }
            | Self::StoreThrough { .. }
            | Self::Drop { .. }
            | Self::Assert { .. }
            | Self::Zombie(_)
            | Self::SpillWrite { .. } => false,
            Self::Call { .. } | Self::Intrinsic { .. } | Self::Host { .. } => return None,
        })
    }

    /// Every value this op reads, in operand order.
    pub fn uses(&self) -> Vec<ValueId> {
        let mut uses = Vec::new();
        self.for_each_use(|v| uses.push(v));
        uses
    }

    /// The value this op moves out of, if it is a `Move`.
    pub fn moved(&self) -> Option<ValueId> {
        match self {
            Self::Move(v) => Some(*v),
            _ => None,
        }
    }

    fn for_each_use(&self, mut f: impl FnMut(ValueId)) {
        let path_uses = |path: &[Proj], f: &mut dyn FnMut(ValueId)| {
            for proj in path {
                if let Proj::Index(v) = proj {
                    f(*v);
                }
            }
        };
        let root_uses = |root: &Root, f: &mut dyn FnMut(ValueId)| {
            if let Root::Value(v) = root {
                f(*v);
            }
        };
        match self {
            Self::Const(_)
            | Self::CellAlloc(_)
            | Self::CellLoad(_)
            | Self::Zombie(_)
            | Self::SpillRead(_) => {}
            Self::Copy(v)
            | Self::Move(v)
            | Self::Repeat { v, .. }
            | Self::Field { base: v, .. }
            | Self::Discriminant(v)
            | Self::SetDiscriminant { base: v, .. }
            | Self::CellStore { v, .. }
            | Self::Deref(v)
            | Self::Unary { v, .. }
            | Self::Cast { v, .. }
            | Self::Materialize(v)
            | Self::Convert { v, .. }
            | Self::SpillWrite { v, .. } => f(*v),
            Self::Aggregate { fields, .. } => fields.iter().copied().for_each(f),
            Self::SetField { base, v, .. } => {
                f(*base);
                f(*v);
            }
            Self::BorrowView { root, path, .. }
            | Self::RawView { root, path, .. }
            | Self::Drop { root, path, .. } => {
                root_uses(root, &mut f);
                path_uses(path, &mut f);
            }
            Self::SliceView { ptr, len } => {
                f(*ptr);
                f(*len);
            }
            Self::StoreThrough { ptr, v } => {
                f(*ptr);
                f(*v);
            }
            Self::Binary { l, r, .. } => {
                f(*l);
                f(*r);
            }
            Self::Call { function, args, .. } => {
                if let Some(function) = function {
                    f(*function);
                }
                args.iter().copied().for_each(f);
            }
            Self::Intrinsic { args, .. } | Self::Host { args, .. } => {
                args.iter().copied().for_each(f)
            }
            Self::Assert { cond, msg, .. } => {
                f(*cond);
                msg.for_each_use(f);
            }
        }
    }
}

/// The base of a borrowed, raw-viewed, or dropped place.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Root {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// The storage of an SSA value. With an empty path the value's type must be indirect (an
    /// aggregate: something with storage to point at); a reference to a primitive needs a cell.
    Value(ValueId),
    Cell(CellId),
}

/// One step of a place path. Mirrors [`super::semantic::Projection`] with the index operand an
/// SSA value and the binder casts (which change no storage) left out.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Proj {
    /// Append-only: postcard serializes this enum by index.
    Deref,
    /// Field `field` of the current place; `ty` is the field's type.
    Field {
        field: u32,
        ty: TypeId,
    },
    /// Element `v` (a `usize` value) of an array or slice.
    Index(ValueId),
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
}

/// [`super::semantic::AssertMessage`] with SSA values for its operands.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RAssertMessage {
    /// Append-only: postcard serializes this enum by index.
    BoundsCheck {
        length: ValueId,
        index: ValueId,
    },
    Overflow {
        op: BinaryOp,
        left: ValueId,
        right: ValueId,
    },
    OverflowNeg(ValueId),
    DivisionByZero(ValueId),
    RemainderByZero(ValueId),
    ResumedAfterReturn(CoroutineKind),
    ResumedAfterPanic(CoroutineKind),
    ResumedAfterDrop(CoroutineKind),
    MisalignedPointer {
        required: ValueId,
        found: ValueId,
    },
    NullPointer,
    NullReference,
    InvalidEnum(ValueId),
}

impl RAssertMessage {
    fn for_each_use(&self, mut f: impl FnMut(ValueId)) {
        match self {
            Self::BoundsCheck {
                length: a,
                index: b,
            }
            | Self::Overflow {
                left: a, right: b, ..
            }
            | Self::MisalignedPointer {
                required: a,
                found: b,
            } => {
                f(*a);
                f(*b);
            }
            Self::OverflowNeg(v)
            | Self::DivisionByZero(v)
            | Self::RemainderByZero(v)
            | Self::InvalidEnum(v) => f(*v),
            Self::ResumedAfterReturn(_)
            | Self::ResumedAfterPanic(_)
            | Self::ResumedAfterDrop(_)
            | Self::NullPointer
            | Self::NullReference => {}
        }
    }
}

/// A block terminator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RTerm {
    /// Append-only: postcard serializes this enum by index.
    Goto(RBlockId, Vec<ValueId>),
    /// A multi-way branch on the raw unsigned bits of `scrut` (of type `scrut_ty`: an integer,
    /// `bool`, or `char`), as MIR's `SwitchInt` carries them.
    Switch {
        scrut: ValueId,
        scrut_ty: TypeId,
        cases: Vec<(u128, RBlockId, Vec<ValueId>)>,
        default: (RBlockId, Vec<ValueId>),
    },
    /// Returns `Some(v)` or, for a `()`-returning function, `None`.
    Return(Option<ValueId>),
    Unreachable,
    /// Throws `Some(v)` or rethrows the exception in flight (`UnwindResume`).
    Throw(Option<ValueId>),
    /// Protects the block's final call or drop. Its result is defined at the
    /// private normal successor, never on the unwind edge. Earlier operations
    /// execute outside the protected region.
    Invoke {
        normal: (RBlockId, Vec<ValueId>),
        unwind: (RBlockId, Vec<ValueId>),
    },
}

impl RTerm {
    /// The successor blocks with the arguments passed on each edge, in order.
    pub fn edges(&self) -> Vec<(RBlockId, &[ValueId])> {
        match self {
            Self::Goto(block, args) => vec![(*block, args.as_slice())],
            Self::Switch { cases, default, .. } => cases
                .iter()
                .map(|(_, block, args)| (*block, args.as_slice()))
                .chain(std::iter::once((default.0, default.1.as_slice())))
                .collect(),
            Self::Invoke { normal, unwind } => vec![
                (normal.0, normal.1.as_slice()),
                (unwind.0, unwind.1.as_slice()),
            ],
            Self::Return(_) | Self::Unreachable | Self::Throw(_) => Vec::new(),
        }
    }

    /// The successor blocks, in order, without their arguments.
    pub fn successors(&self) -> Vec<RBlockId> {
        self.edges().into_iter().map(|(block, _)| block).collect()
    }

    /// Every value the terminator itself reads (the scrutinee, the returned or thrown value, and
    /// every edge argument).
    pub fn uses(&self) -> Vec<ValueId> {
        let mut uses = Vec::new();
        match self {
            Self::Goto(_, args) => uses.extend(args),
            Self::Switch {
                scrut,
                cases,
                default,
                ..
            } => {
                uses.push(*scrut);
                for (_, _, args) in cases {
                    uses.extend(args);
                }
                uses.extend(&default.1);
            }
            Self::Invoke { normal, unwind } => {
                uses.extend(&normal.1);
                uses.extend(&unwind.1);
            }
            Self::Return(v) | Self::Throw(v) => uses.extend(v),
            Self::Unreachable => {}
        }
        uses
    }
}

/// Builds a [`Func`] without hand-rolled ids.
///
/// `new` creates block 0; `param` adds an entry parameter; `block` adds a block; `op` pushes an
/// op that defines a value and returns its id; `effect` pushes one that does not; `terminate`
/// sets a terminator; `finish` returns the `Func`. Nothing here verifies: call [`Func::verify`]
/// on the result.
#[derive(Clone, Debug)]
pub struct FuncBuilder {
    func: Func,
}

impl FuncBuilder {
    /// A function with an empty entry block.
    pub fn new(item: ItemId, sig: Signature) -> Self {
        Self {
            func: Func {
                item,
                sig,
                params: Vec::new(),
                values: Vec::new(),
                cells: Vec::new(),
                blocks: vec![RBlock {
                    id: RBlockId(0),
                    params: Vec::new(),
                    ops: Vec::new(),
                    term: RTerm::Unreachable,
                }],
            },
        }
    }

    /// The entry block.
    pub const ENTRY: RBlockId = RBlockId(0);

    /// The function built so far, for inspection.
    pub fn func(&self) -> &Func {
        &self.func
    }

    /// Defines a value; the caller is responsible for an op or block parameter binding it.
    pub fn value(&mut self, ty: TypeId, origin: OriginId) -> ValueId {
        let id = ValueId(self.func.values.len() as u32);
        self.func.values.push(ValueDef {
            ty,
            carrier: Carrier::Baseline,
            origin,
        });
        id
    }

    /// Adds an entry parameter (a block 0 parameter listed in `Func::params`).
    pub fn param(&mut self, ty: TypeId, origin: OriginId) -> ValueId {
        let id = self.value(ty, origin);
        self.func.params.push(id);
        self.func.blocks[0].params.push(id);
        id
    }

    /// Declares a cell.
    pub fn cell(&mut self, ty: TypeId, boxed: bool, origin: OriginId) -> CellId {
        let id = CellId(self.func.cells.len() as u32);
        self.func.cells.push(CellDef { ty, boxed, origin });
        id
    }

    /// Adds an empty block terminated by `Unreachable` until `terminate` replaces that.
    pub fn block(&mut self) -> RBlockId {
        let id = RBlockId(self.func.blocks.len() as u32);
        self.func.blocks.push(RBlock {
            id,
            params: Vec::new(),
            ops: Vec::new(),
            term: RTerm::Unreachable,
        });
        id
    }

    /// Adds a parameter to `block` (not the entry: use `param` for that).
    pub fn block_param(&mut self, block: RBlockId, ty: TypeId, origin: OriginId) -> ValueId {
        let id = self.value(ty, origin);
        self.func.blocks[block.0 as usize].params.push(id);
        id
    }

    /// Pushes an op that defines a value of type `ty` and returns the value.
    pub fn op(&mut self, block: RBlockId, kind: OpKind, ty: TypeId, origin: OriginId) -> ValueId {
        let result = self.value(ty, origin);
        self.func.blocks[block.0 as usize].ops.push(Op {
            result: Some(result),
            origin,
            kind,
        });
        result
    }

    /// Pushes an op that defines no value.
    pub fn effect(&mut self, block: RBlockId, kind: OpKind, origin: OriginId) {
        self.func.blocks[block.0 as usize].ops.push(Op {
            result: None,
            origin,
            kind,
        });
    }

    pub fn terminate(&mut self, block: RBlockId, term: RTerm) {
        self.func.blocks[block.0 as usize].term = term;
    }

    pub fn finish(self) -> Func {
        self.func
    }
}

/// Reverse postorder and immediate dominators of a [`Func`]'s CFG from block 0.
///
/// Blocks the entry does not reach are absent from `rpo` and have no dominator.
#[derive(Clone, Debug)]
pub struct Dominators {
    /// Reachable blocks in reverse postorder.
    pub rpo: Vec<RBlockId>,
    /// Immediate dominator of each block by index; `None` for the entry and unreachable blocks.
    pub idom: Vec<Option<RBlockId>>,
    /// Position of each block in `rpo`; `None` for unreachable blocks.
    pub order: Vec<Option<u32>>,
}

impl Dominators {
    /// Cooper, Harvey and Kennedy's iterative algorithm over reverse postorder.
    pub fn of(func: &Func) -> Self {
        let n = func.blocks.len();
        let mut order = vec![None; n];
        let mut rpo = Vec::with_capacity(n);
        if n > 0 {
            // Iterative postorder walk.
            let mut visited = vec![false; n];
            let mut stack: Vec<(usize, Vec<RBlockId>, usize)> = Vec::new();
            visited[0] = true;
            stack.push((0, func.blocks[0].term.successors(), 0));
            while let Some((block, successors, next)) = stack.last_mut() {
                if *next < successors.len() {
                    let successor = successors[*next].0 as usize;
                    *next += 1;
                    if successor < n && !visited[successor] {
                        visited[successor] = true;
                        stack.push((successor, func.blocks[successor].term.successors(), 0));
                    }
                } else {
                    rpo.push(RBlockId(*block as u32));
                    stack.pop();
                }
            }
            rpo.reverse();
            for (position, block) in rpo.iter().enumerate() {
                order[block.0 as usize] = Some(position as u32);
            }
        }
        let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); n];
        for block in &rpo {
            for successor in func.blocks[block.0 as usize].term.successors() {
                if (successor.0 as usize) < n {
                    predecessors[successor.0 as usize].push(block.0 as usize);
                }
            }
        }
        // idom in rpo-position space.
        let mut idom: Vec<Option<u32>> = vec![None; rpo.len()];
        if !rpo.is_empty() {
            idom[0] = Some(0);
            let mut changed = true;
            while changed {
                changed = false;
                for (position, block) in rpo.iter().enumerate().skip(1) {
                    let mut new_idom: Option<u32> = None;
                    for &predecessor in &predecessors[block.0 as usize] {
                        let Some(p) = order[predecessor] else {
                            continue;
                        };
                        if idom[p as usize].is_none() {
                            continue;
                        }
                        new_idom = Some(match new_idom {
                            None => p,
                            Some(current) => Self::intersect(&idom, p, current),
                        });
                    }
                    if new_idom.is_some() && idom[position] != new_idom {
                        idom[position] = new_idom;
                        changed = true;
                    }
                }
            }
        }
        let mut by_block = vec![None; n];
        for (position, block) in rpo.iter().enumerate() {
            if position == 0 {
                continue;
            }
            by_block[block.0 as usize] = idom[position].map(|p| rpo[p as usize]);
        }
        Self {
            rpo,
            idom: by_block,
            order,
        }
    }

    fn intersect(idom: &[Option<u32>], mut a: u32, mut b: u32) -> u32 {
        while a != b {
            while a > b {
                a = idom[a as usize].unwrap_or(0);
            }
            while b > a {
                b = idom[b as usize].unwrap_or(0);
            }
        }
        a
    }

    /// Whether `a` dominates `b` (reflexively). Unreachable blocks dominate nothing and are
    /// dominated by nothing.
    pub fn dominates(&self, a: RBlockId, b: RBlockId) -> bool {
        if self.order.get(a.0 as usize).copied().flatten().is_none()
            || self.order.get(b.0 as usize).copied().flatten().is_none()
        {
            return false;
        }
        let mut current = b;
        loop {
            if current == a {
                return true;
            }
            match self.idom[current.0 as usize] {
                Some(next) => current = next,
                None => return false,
            }
        }
    }

    pub fn is_reachable(&self, block: RBlockId) -> bool {
        self.order
            .get(block.0 as usize)
            .copied()
            .flatten()
            .is_some()
    }
}

/// Where a value is defined: a block parameter or the `index`-th op of a block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DefSite {
    Param(RBlockId),
    Op(RBlockId, usize),
}

struct Verifier<'a> {
    func: &'a Func,
    table: &'a TypeTable,
    carriers: CarrierPolicy,
    defs: Vec<Option<DefSite>>,
    doms: Dominators,
}

impl<'a> Verifier<'a> {
    fn err(message: impl fmt::Display) -> VerifyError {
        VerifyError::new(format!("JIR-R {message}"))
    }

    fn value_def(&self, v: ValueId) -> Result<&'a ValueDef, VerifyError> {
        self.func
            .values
            .get(v.0 as usize)
            .ok_or_else(|| Self::err(format!("references missing value v{}", v.0)))
    }

    fn cell_def(&self, c: CellId) -> Result<&'a CellDef, VerifyError> {
        self.func
            .cells
            .get(c.0 as usize)
            .ok_or_else(|| Self::err(format!("references missing cell c{}", c.0)))
    }

    fn block(&self, b: RBlockId) -> Result<&'a RBlock, VerifyError> {
        self.func
            .blocks
            .get(b.0 as usize)
            .ok_or_else(|| Self::err(format!("references missing block b{}", b.0)))
    }

    fn ty(&self, ty: TypeId, context: &str) -> Result<(), VerifyError> {
        self.table.require(ty, context).map(|_| ())
    }

    fn same_type(&self, a: ValueId, b: ValueId, what: &str) -> Result<(), VerifyError> {
        let (a_ty, b_ty) = (self.value_def(a)?.ty, self.value_def(b)?.ty);
        if a_ty != b_ty {
            return Err(Self::err(format!(
                "{what}: v{} has type {:032x} but v{} has type {:032x}",
                a.0, a_ty.0, b.0, b_ty.0
            )));
        }
        Ok(())
    }

    fn expect_type(&self, v: ValueId, ty: TypeId, what: &str) -> Result<(), VerifyError> {
        let actual = self.value_def(v)?.ty;
        if actual != ty {
            return Err(Self::err(format!(
                "{what}: v{} has type {:032x}, expected {:032x}",
                v.0, actual.0, ty.0
            )));
        }
        Ok(())
    }

    /// Records definitions and checks structure: ids in range, results agree with kinds, each
    /// value defined exactly once, every op has a valid type and origin-bearing definition.
    fn collect_defs(&mut self) -> Result<(), VerifyError> {
        let func = self.func;
        for (index, def) in func.values.iter().enumerate() {
            self.ty(def.ty, &format!("value v{index}"))?;
            if !self.carriers.allows(def.carrier) {
                return Err(Self::err(format!(
                    "value v{index} has carrier {:?}, which no committed plan allows here",
                    def.carrier
                )));
            }
        }
        for (index, cell) in func.cells.iter().enumerate() {
            self.ty(cell.ty, &format!("cell c{index}"))?;
        }
        self.ty(func.sig.result, "signature result")?;
        for argument in &func.sig.arguments {
            self.ty(*argument, "signature argument")?;
        }
        if func.blocks.is_empty() {
            return Err(Self::err("function has no entry block"));
        }
        if func.blocks[0].params != func.params {
            return Err(Self::err(
                "entry block parameters differ from the function parameters",
            ));
        }
        if usize::from(func.sig.abi.parameters) != func.params.len() {
            return Err(Self::err(format!(
                "signature declares {} JavaScript parameters but the function has {}",
                func.sig.abi.parameters,
                func.params.len()
            )));
        }
        let define = |v: ValueId, site: DefSite, defs: &mut Vec<Option<DefSite>>| {
            let slot = defs
                .get_mut(v.0 as usize)
                .ok_or_else(|| Self::err(format!("defines missing value v{}", v.0)))?;
            if slot.is_some() {
                return Err(Self::err(format!("value v{} is defined twice", v.0)));
            }
            *slot = Some(site);
            Ok(())
        };
        let mut defs = vec![None; func.values.len()];
        for (index, block) in func.blocks.iter().enumerate() {
            if block.id.0 as usize != index {
                return Err(Self::err(format!(
                    "block b{} is stored at index {index}",
                    block.id.0
                )));
            }
            if let RTerm::Invoke { normal, unwind } = &block.term {
                self.block(normal.0)?;
                self.block(unwind.0)?;
                if normal.0 == RBlockId(0)
                    || normal.0 == block.id
                    || normal.0 == unwind.0
                    || func.blocks.iter().any(|predecessor| {
                        predecessor.id != block.id
                            && predecessor.term.successors().contains(&normal.0)
                    })
                {
                    return Err(Self::err("invoke normal successor must be private"));
                }
                if !block.ops.last().is_some_and(|op| {
                    matches!(
                        op.kind,
                        OpKind::Call { .. }
                            | OpKind::Intrinsic { .. }
                            | OpKind::Host { .. }
                            | OpKind::Drop { .. }
                    )
                }) {
                    return Err(Self::err("invoke must end with a call or drop"));
                }
            }
            for param in &block.params {
                define(*param, DefSite::Param(block.id), &mut defs)?;
            }
            for (position, op) in block.ops.iter().enumerate() {
                match (op.kind.produces_value(), op.result) {
                    (Some(true), None) => {
                        return Err(Self::err(format!(
                            "op {position} of b{index} ({}) must define a value",
                            op_name(&op.kind)
                        )));
                    }
                    (Some(false), Some(_)) => {
                        return Err(Self::err(format!(
                            "op {position} of b{index} ({}) cannot define a value",
                            op_name(&op.kind)
                        )));
                    }
                    _ => {}
                }
                if let Some(result) = op.result {
                    let site = match &block.term {
                        RTerm::Invoke { normal, .. } if position + 1 == block.ops.len() => {
                            DefSite::Param(normal.0)
                        }
                        _ => DefSite::Op(block.id, position),
                    };
                    define(result, site, &mut defs)?;
                }
            }
        }
        for (index, def) in defs.iter().enumerate() {
            if def.is_none() {
                return Err(Self::err(format!("value v{index} is never defined")));
            }
        }
        self.defs = defs;
        Ok(())
    }

    /// Whether the definition of `v` dominates a use at op `position` of `block` (a use in the
    /// terminator is `position == ops.len()`).
    fn dominated(&self, v: ValueId, block: RBlockId, position: usize) -> Result<(), VerifyError> {
        let site = self.defs[v.0 as usize].expect("collect_defs ran");
        let ok = match site {
            DefSite::Param(def_block) => self.doms.dominates(def_block, block),
            DefSite::Op(def_block, def_position) => {
                if def_block == block {
                    def_position < position
                } else {
                    self.doms.dominates(def_block, block)
                }
            }
        };
        if ok {
            Ok(())
        } else {
            Err(Self::err(format!(
                "use of v{} at op {position} of b{} is not dominated by its definition",
                v.0, block.0
            )))
        }
    }

    fn check_edge(
        &self,
        from: RBlockId,
        to: RBlockId,
        args: &[ValueId],
    ) -> Result<(), VerifyError> {
        let target = self.block(to)?;
        if target.params.len() != args.len() {
            return Err(Self::err(format!(
                "edge b{} -> b{} passes {} arguments to {} parameters",
                from.0,
                to.0,
                args.len(),
                target.params.len()
            )));
        }
        for (arg, param) in args.iter().zip(&target.params) {
            self.same_type(*arg, *param, "block argument")?;
            let actual = self.value_def(*arg)?.carrier;
            let expected = self.value_def(*param)?.carrier;
            if actual != expected {
                return Err(Self::err(format!(
                    "edge b{} -> b{} passes v{} as {actual:?} to v{} as {expected:?}; use Convert",
                    from.0, to.0, arg.0, param.0,
                )));
            }
        }
        Ok(())
    }

    fn check_place(&self, root: &Root, path: &[Proj]) -> Result<(), VerifyError> {
        match root {
            Root::Value(v) => {
                let def = self.value_def(*v)?;
                if path.is_empty() && !self.table.is_indirect(def.ty) {
                    return Err(Self::err(format!(
                        "borrow of v{} takes the address of a direct value; use a cell",
                        v.0
                    )));
                }
            }
            Root::Cell(c) => {
                self.cell_def(*c)?;
            }
        }
        for proj in path {
            match proj {
                Proj::Index(v) => {
                    self.value_def(*v)?;
                }
                Proj::Field { ty, .. } => self.ty(*ty, "projection field")?,
                Proj::Deref
                | Proj::ConstantIndex { .. }
                | Proj::Subslice { .. }
                | Proj::Downcast { .. } => {}
            }
        }
        Ok(())
    }

    fn check_op(&self, block: RBlockId, position: usize, op: &Op) -> Result<(), VerifyError> {
        for v in op.kind.uses() {
            self.value_def(v)?;
            self.dominated(v, block, position)?;
        }
        let result = op.result;
        match &op.kind {
            OpKind::Copy(v) | OpKind::Move(v) | OpKind::Materialize(v) => {
                if let Some(result) = result {
                    self.same_type(*v, result, op_name(&op.kind))?;
                    if matches!(op.kind, OpKind::Copy(_) | OpKind::Move(_)) {
                        let from = self.value_def(*v)?.carrier;
                        let to = self.value_def(result)?.carrier;
                        if from != to {
                            return Err(Self::err(format!(
                                "{} reads v{} as {from:?} but writes v{} as {to:?}; use Convert",
                                op_name(&op.kind),
                                v.0,
                                result.0,
                            )));
                        }
                    }
                }
            }
            OpKind::Aggregate { ty, fields, .. } => {
                self.ty(*ty, "aggregate")?;
                if let Some(result) = result {
                    self.expect_type(result, *ty, "aggregate result")?;
                }
                for field in fields {
                    self.value_def(*field)?;
                }
            }
            OpKind::Field {
                base,
                variant,
                field,
            } => {
                let base_ty = self.value_def(*base)?.ty;
                if let (Some(descriptor), Some(result)) =
                    (self.table.field(base_ty, *variant, *field), result)
                {
                    self.expect_type(result, descriptor.ty, "field result")?;
                }
            }
            OpKind::SetField {
                base,
                variant,
                field,
                v,
            } => {
                let base_ty = self.value_def(*base)?.ty;
                if let Some(descriptor) = self.table.field(base_ty, *variant, *field) {
                    self.expect_type(*v, descriptor.ty, "field store")?;
                }
            }
            OpKind::CellAlloc(cell) => {
                self.cell_def(*cell)?;
            }
            OpKind::CellLoad(cell) => {
                let def = self.cell_def(*cell)?;
                if let Some(result) = result {
                    self.expect_type(result, def.ty, "cell load")?;
                }
            }
            OpKind::CellStore { cell, v } => {
                let def = self.cell_def(*cell)?;
                self.expect_type(*v, def.ty, "cell store")?;
            }
            OpKind::BorrowView { root, path, .. }
            | OpKind::RawView { root, path, .. }
            | OpKind::Drop { root, path, .. } => self.check_place(root, path)?,
            OpKind::Deref(ptr) => {
                let ptr_ty = self.value_def(*ptr)?.ty;
                if let (Some(pointee), Some(result)) = (self.pointee(ptr_ty), result) {
                    self.expect_type(result, pointee, "deref")?;
                }
            }
            OpKind::StoreThrough { ptr, v } => {
                let ptr_ty = self.value_def(*ptr)?.ty;
                if let Some(pointee) = self.pointee(ptr_ty) {
                    self.expect_type(*v, pointee, "store through")?;
                }
            }
            OpKind::Binary { l, ty, .. } => {
                self.ty(*ty, "binary operand")?;
                self.expect_type(*l, *ty, "binary left operand")?;
            }
            OpKind::Unary { v, ty, .. } => {
                self.ty(*ty, "unary operand")?;
                self.expect_type(*v, *ty, "unary operand")?;
            }
            OpKind::Cast { v, from, to, .. } => {
                self.ty(*from, "cast source")?;
                self.ty(*to, "cast target")?;
                self.expect_type(*v, *from, "cast operand")?;
                if let Some(result) = result {
                    self.expect_type(result, *to, "cast result")?;
                }
            }
            OpKind::Call {
                callee, function, ..
            } => {
                if matches!(callee, Callee::Intrinsic { .. }) {
                    return Err(Self::err(
                        "Call with Callee::Intrinsic; intrinsics are OpKind::Intrinsic",
                    ));
                }
                match (callee, function) {
                    (Callee::Indirect { signature }, Some(function)) => {
                        self.expect_type(*function, *signature, "indirect callee")?;
                    }
                    (Callee::Indirect { .. }, None) => {
                        return Err(Self::err("indirect call without a function value"));
                    }
                    (_, Some(_)) => {
                        return Err(Self::err(
                            "call with a function value but a non-indirect callee",
                        ));
                    }
                    (_, None) => {}
                }
            }
            OpKind::Convert { v, from, to } => {
                if from == to {
                    return Err(Self::err(format!(
                        "Convert between two {from:?} carriers is meaningless"
                    )));
                }
                // The op names both ends so the emitter picks an adapter without walking back to
                // the definition; naming them wrongly would pick the wrong adapter, so they are
                // checked against the values they claim to describe.
                let operand = self.value_def(*v)?;
                if operand.carrier != *from {
                    return Err(Self::err(format!(
                        "Convert reads v{} as {from:?} but it carries {:?}",
                        v.0, operand.carrier
                    )));
                }
                if let Some(result) = result {
                    self.same_type(*v, result, "Convert")?;
                    let produced = self.value_def(result)?.carrier;
                    if produced != *to {
                        return Err(Self::err(format!(
                            "Convert writes v{} as {to:?} but it carries {produced:?}",
                            result.0
                        )));
                    }
                }
            }
            OpKind::Const(_)
            | OpKind::Repeat { .. }
            | OpKind::Discriminant(_)
            | OpKind::SetDiscriminant { .. }
            | OpKind::SliceView { .. }
            | OpKind::Intrinsic { .. }
            | OpKind::Host { .. }
            | OpKind::Assert { .. }
            | OpKind::Zombie(_)
            | OpKind::SpillRead(_)
            | OpKind::SpillWrite { .. } => {}
        }
        Ok(())
    }

    /// The pointee type of a reference or raw pointer type, after peeling wrappers.
    fn pointee(&self, ty: TypeId) -> Option<TypeId> {
        let peeled = self.table.peeled(ty);
        match &self.table.get(peeled)?.kind {
            TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
                Some(*pointee)
            }
            _ => None,
        }
    }

    fn check_term(&self, block: &RBlock) -> Result<(), VerifyError> {
        let position = block.ops.len();
        for v in block.term.uses() {
            self.value_def(v)?;
            self.dominated(v, block.id, position)?;
        }
        match &block.term {
            RTerm::Goto(target, args) => self.check_edge(block.id, *target, args)?,
            RTerm::Invoke { normal, unwind } => {
                self.check_edge(block.id, normal.0, &normal.1)?;
                self.check_edge(block.id, unwind.0, &unwind.1)?;
            }
            RTerm::Switch {
                scrut,
                scrut_ty,
                cases,
                default,
            } => {
                self.ty(*scrut_ty, "switch scrutinee")?;
                self.expect_type(*scrut, *scrut_ty, "switch scrutinee")?;
                let mut seen = BTreeSet::new();
                for (value, target, args) in cases {
                    if !seen.insert(*value) {
                        return Err(Self::err(format!(
                            "switch in b{} has duplicate case {value}",
                            block.id.0
                        )));
                    }
                    self.check_edge(block.id, *target, args)?;
                }
                self.check_edge(block.id, default.0, &default.1)?;
            }
            RTerm::Return(v) => match v {
                Some(v) => self.expect_type(*v, self.func.sig.result, "return")?,
                None => {
                    if !self.table.is_zst(self.func.sig.result) {
                        return Err(Self::err(format!(
                            "b{} returns nothing from a function with a non-zero-sized result",
                            block.id.0
                        )));
                    }
                }
            },
            RTerm::Unreachable | RTerm::Throw(_) => {}
        }
        Ok(())
    }

    /// A rethrow needs an exception on every incoming path. Successful protected
    /// cleanup preserves the current exception; a throwing cleanup replaces it.
    fn check_exception_flow(&self) -> Result<(), VerifyError> {
        if !self.func.has_invoke() {
            return Ok(());
        }
        let mut pending = vec![true; self.func.blocks.len()];
        pending[0] = false;
        loop {
            let mut changed = false;
            for id in &self.doms.rpo {
                let block = &self.func.blocks[id.0 as usize];
                for target in block.term.successors() {
                    let caught = matches!(&block.term, RTerm::Invoke { unwind, .. }
                        if unwind.0 == target);
                    let incoming = pending[id.0 as usize] || caught;
                    let state = &mut pending[target.0 as usize];
                    if *state && !incoming {
                        *state = false;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        for id in &self.doms.rpo {
            if matches!(self.func.blocks[id.0 as usize].term, RTerm::Throw(None))
                && !pending[id.0 as usize]
            {
                return Err(Self::err(format!(
                    "rethrow in b{} has no pending exception",
                    id.0
                )));
            }
        }
        Ok(())
    }

    /// Forward may-be-moved dataflow: a value moved on any path into a point may not be used at
    /// that point. Also a forward must-be-allocated dataflow for cells.
    fn check_moves_and_cells(&self) -> Result<(), VerifyError> {
        let n = self.func.blocks.len();
        let cells = self.func.cells.len();
        // Per block: the set of moved values on entry (union over predecessors) and the set of
        // allocated cells on entry (intersection over predecessors; the entry starts empty).
        let mut moved_in: Vec<BTreeSet<ValueId>> = vec![BTreeSet::new(); n];
        let mut alloc_in: Vec<Option<BTreeSet<CellId>>> = vec![None; n];
        alloc_in[0] = Some(BTreeSet::new());
        let all_cells: BTreeSet<CellId> = (0..cells as u32).map(CellId).collect();
        // Iterate to a fixed point over rpo.
        loop {
            let mut changed = false;
            for &block_id in &self.doms.rpo {
                let block = &self.func.blocks[block_id.0 as usize];
                let mut moved = moved_in[block_id.0 as usize].clone();
                let mut allocated = alloc_in[block_id.0 as usize]
                    .clone()
                    .unwrap_or_else(|| all_cells.clone());
                for op in &block.ops {
                    if let OpKind::CellAlloc(cell) = op.kind {
                        allocated.insert(cell);
                    }
                    if let Some(v) = op.kind.moved() {
                        moved.insert(v);
                    }
                }
                for successor in block.term.successors() {
                    let index = successor.0 as usize;
                    if index >= n {
                        continue;
                    }
                    let before = moved_in[index].len();
                    moved_in[index].extend(moved.iter().copied());
                    if moved_in[index].len() != before {
                        changed = true;
                    }
                    let next = match &alloc_in[index] {
                        None => allocated.clone(),
                        Some(current) => current.intersection(&allocated).copied().collect(),
                    };
                    if alloc_in[index].as_ref() != Some(&next) {
                        alloc_in[index] = Some(next);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        for &block_id in &self.doms.rpo {
            let block = &self.func.blocks[block_id.0 as usize];
            let mut moved = moved_in[block_id.0 as usize].clone();
            let mut allocated = alloc_in[block_id.0 as usize].clone().unwrap_or_default();
            for (position, op) in block.ops.iter().enumerate() {
                for v in op.kind.uses() {
                    if moved.contains(&v) {
                        return Err(Self::err(format!(
                            "v{} is used at op {position} of b{} after being moved",
                            v.0, block_id.0
                        )));
                    }
                }
                let cell = match &op.kind {
                    OpKind::CellLoad(cell) | OpKind::CellStore { cell, .. } => Some(*cell),
                    OpKind::BorrowView {
                        root: Root::Cell(cell),
                        ..
                    }
                    | OpKind::RawView {
                        root: Root::Cell(cell),
                        ..
                    }
                    | OpKind::Drop {
                        root: Root::Cell(cell),
                        ..
                    } => Some(*cell),
                    _ => None,
                };
                if let Some(cell) = cell {
                    if !allocated.contains(&cell) {
                        return Err(Self::err(format!(
                            "c{} is used at op {position} of b{} before being allocated on \
                             every path",
                            cell.0, block_id.0
                        )));
                    }
                }
                if let OpKind::CellAlloc(cell) = op.kind {
                    allocated.insert(cell);
                }
                if let Some(v) = op.kind.moved() {
                    moved.insert(v);
                }
            }
            for v in block.term.uses() {
                if moved.contains(&v) {
                    return Err(Self::err(format!(
                        "v{} is used by the terminator of b{} after being moved",
                        v.0, block_id.0
                    )));
                }
            }
        }
        Ok(())
    }
}

fn op_name(kind: &OpKind) -> &'static str {
    match kind {
        OpKind::Const(_) => "Const",
        OpKind::Copy(_) => "Copy",
        OpKind::Move(_) => "Move",
        OpKind::Aggregate { .. } => "Aggregate",
        OpKind::Repeat { .. } => "Repeat",
        OpKind::Field { .. } => "Field",
        OpKind::SetField { .. } => "SetField",
        OpKind::Discriminant(_) => "Discriminant",
        OpKind::SetDiscriminant { .. } => "SetDiscriminant",
        OpKind::CellAlloc(_) => "CellAlloc",
        OpKind::CellLoad(_) => "CellLoad",
        OpKind::CellStore { .. } => "CellStore",
        OpKind::BorrowView { .. } => "BorrowView",
        OpKind::RawView { .. } => "RawView",
        OpKind::SliceView { .. } => "SliceView",
        OpKind::Deref(_) => "Deref",
        OpKind::StoreThrough { .. } => "StoreThrough",
        OpKind::Binary { .. } => "Binary",
        OpKind::Unary { .. } => "Unary",
        OpKind::Cast { .. } => "Cast",
        OpKind::Call { .. } => "Call",
        OpKind::Intrinsic { .. } => "Intrinsic",
        OpKind::Host { .. } => "Host",
        OpKind::Drop { .. } => "Drop",
        OpKind::Assert { .. } => "Assert",
        OpKind::Materialize(_) => "Materialize",
        OpKind::Convert { .. } => "Convert",
        OpKind::Zombie(_) => "Zombie",
        OpKind::SpillRead(_) => "SpillRead",
        OpKind::SpillWrite { .. } => "SpillWrite",
    }
}

impl Func {
    /// Whether this function contains protected calls or drops.
    pub fn has_invoke(&self) -> bool {
        self.blocks
            .iter()
            .any(|block| matches!(block.term, RTerm::Invoke { .. }))
    }

    /// Checks the function's SSA and structural invariants against `table`.
    ///
    /// Checked: block 0 exists and its parameters are `params` (as many as the ABI declares);
    /// every value is defined exactly once, by a block parameter or an op whose kind produces a
    /// value; every use is dominated by its definition (RPO dominators; a use in the same block
    /// must come after the definition); every `Goto`/`Switch` edge passes as many arguments as
    /// the target has parameters, with equal types and carriers; `Switch` cases are unique and the
    /// scrutinee has `scrut_ty`; a moved value is not used again on any path (forward
    /// may-be-moved dataflow); a cell is loaded, stored, borrowed, or dropped only where a
    /// `CellAlloc` reaches it on every path; `Copy`/`Move`/`Materialize` keep the type, and
    /// `Copy`/`Move` also keep the carrier; `CellLoad`/
    /// `CellStore` agree with `CellDef::ty`; `Field`/`SetField` agree with the type table where
    /// it knows the field; `Deref`/`StoreThrough` agree with the pointee; `Cast` operand and
    /// result carry `from` and `to`; `Return` matches the signature; a borrow of a value with an
    /// empty path names an indirect type; every carrier is one the policy allows, and a `Convert`
    /// names the carriers its operand and result actually have; `Call` never carries
    /// `Callee::Intrinsic`; every referenced type resolves.
    ///
    /// Not checked (the lowerer declines these itself): intern ids, origin ids (origins live in
    /// the JIR-S origin table beside the function), and the shape of `Const` trees.
    pub fn verify(&self, table: &TypeTable) -> Result<(), VerifyError> {
        self.verify_carriers(table, CarrierPolicy::BASELINE)
    }

    /// [`Func::verify`], with the carriers a committed representation plan allows.
    ///
    /// This is the one door a non-baseline carrier comes through: a plan widens the policy to
    /// exactly what it committed and verifies its rewritten clone with it, so a carrier the plan
    /// did not choose is a verify failure and therefore a rollback rather than an emission.
    pub fn verify_carriers(
        &self,
        table: &TypeTable,
        carriers: CarrierPolicy,
    ) -> Result<(), VerifyError> {
        let doms = Dominators::of(self);
        let mut verifier = Verifier {
            func: self,
            table,
            carriers,
            defs: Vec::new(),
            doms,
        };
        verifier.collect_defs()?;
        for block in &self.blocks {
            if !verifier.doms.is_reachable(block.id) {
                // Unreachable blocks are checked for structure only: dominance is undefined there.
                for op in &block.ops {
                    for v in op.kind.uses() {
                        verifier.value_def(v)?;
                    }
                }
                for (target, args) in block.term.edges() {
                    verifier.check_edge(block.id, target, args)?;
                }
                continue;
            }
            for (position, op) in block.ops.iter().enumerate() {
                verifier.check_op(block.id, position, op)?;
            }
            verifier.check_term(block)?;
        }
        verifier.check_exception_flow()?;
        verifier.check_moves_and_cells()
    }

    /// The type of `v`.
    pub fn ty(&self, v: ValueId) -> Option<TypeId> {
        self.values.get(v.0 as usize).map(|def| def.ty)
    }
}
