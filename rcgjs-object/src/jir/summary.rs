//! The item summary index: what the link step knows about every item **before** decoding any body.
//!
//! An object's section S2 is one [`ItemSummary`] per item, read eagerly. It carries the identity
//! the linker deduplicates on ([`SemanticKey`]), the dependency edges the reachability walk follows
//! (`deps`, which are exactly the printed item's over-approximate `refs`), whether the item has a
//! JIR body and where it is, and a coarse [`ItemClass`] for reports and lane policy. Everything a
//! whole-program pass wants to decide *without* inflating bodies is here; the body itself is
//! decoded only once the item is known to be reachable.
//!
//! The second half of the module is the identity rule the link step deduplicates with:
//! [`NormalizedBody`] and [`semantic_hash`] (what makes two bodies the same body),
//! [`merge_duplicates`] (one item reached the linker twice) and [`merge_aliases`] (two items whose
//! printed JavaScript the text optimizer proved identical). See `JIR.md` for the rule in prose and
//! for the autopsy of v1's 74 duplicate conflicts.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::decline::{Decline, MergeDecline};
use super::hash::{Hash128, Hasher128};
use super::interns::{BytesId, Interns, StrId};
use super::origin::{SourceFact, SourceFactId};
use super::semantic::{
    AbiSignature, AggregateKind, AssertMessage, BasicBlock, BlockId, Body, BodyKind, BodySemantic,
    Call, CallSemantic, Callee, CastKind, ConstTree, Constant, DirectItemRef, DropGlue,
    GlobalReference, HostPayload, ItemId, KeyedListCallable, KeyedListPayload, Local, LocalId,
    LocalRole, Operand, OperandKind, Place, Projection, Rvalue, Signature, SpreadArgument,
    Statement, StatementKind, Terminator, TypeId, Unwind, VtableEntry,
};
use super::types::{TypeKind, TypeTable};
use super::JIR_SCHEMA;
use crate::object::{BlobRef, ItemKind, Linkage, SourceOrder};

/// What two objects have to agree on for their items to be the same item.
///
/// `link_name` is the emitted JavaScript identifier, which v1 keyed everything on. `item` is the
/// stable Rust identity behind it, `None` for a synthesized item (allocator shim, hoisted constant,
/// import) that has no mono item.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SemanticKey {
    pub link_name: String,
    pub item: Option<ItemId>,
}

/// Whether the item carries a JIR body the JIR lane could lower.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Availability {
    /// Append-only: postcard serializes this enum by index.
    Available,
    Unavailable(Decline),
}

impl Availability {
    pub fn is_available(&self) -> bool {
        matches!(self, Availability::Available)
    }
}

/// One item's entry in the summary index.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ItemSummary {
    pub key: SemanticKey,
    /// The content hash of the body's postcard bytes: the **address of its blob** in S3, and the
    /// check the reader makes after inflating it. `None` when the item has no body.
    ///
    /// This is a hash of bytes, not of meaning: two codegen units that ran one item through
    /// different MIR pipelines produce bodies that do the same thing and hash differently here. It
    /// is deliberately *not* what the linker deduplicates on — see [`ItemSummary::semantic_hash`].
    pub body_hash: Option<Hash128>,
    /// The hash of the body's normal form ([`NormalizedBody`]): the identity the link step's
    /// duplicate rule compares. `None` when the item has no body.
    pub semantic_hash: Option<Hash128>,
    pub availability: Availability,
    /// Every identifier the item's printed JavaScript mentions, sorted, the item's own name
    /// excluded: v1's `JsItem.refs`, exactly. Over-approximate by design; the reachability walk
    /// intersects it with the item table.
    pub deps: Vec<String>,
    /// The JavaScript parameter shape, when the item is a function with a body.
    pub abi: Option<AbiSignature>,
    /// Representation-neutral physical-call facts produced beside the body and validated by the
    /// final linker before an ABI decision may use them.
    pub physical: Option<PhysicalSummary>,
    pub kind: ItemKind,
    pub linkage: Linkage,
    pub order: Option<SourceOrder>,
    /// Whether the item carries a TypeScript signature (in S4).
    pub dts: bool,
    /// The item's baseline JavaScript in the text section (`offset` from the start of the object,
    /// `len` bytes; `ulen == len`).
    pub fallback: BlobRef,
    /// The deflated body blob in S3.
    pub body: Option<BlobRef>,
    /// The deflated origin table blob in S5.
    pub origin: Option<BlobRef>,
    /// The size of the printed baseline text, which is what a size report attributes to the item
    /// until a lane replaces it.
    pub estimated_bytes: u32,
    pub class: ItemClass,
}

/// The independent schema of [`PhysicalSummary`].
pub const PHYSICAL_SUMMARY_FORMAT: u16 = 1;

/// Stable physical-call facts available before a body is decoded.
///
/// The producer records facts only. The final linker chooses a JavaScript representation after it
/// has merged every object and reconstructed callers by [`ItemId`]. A linker must validate this
/// summary against the decoded body before using it for a physical ABI decision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PhysicalSummary {
    pub format: u16,
    pub item: ItemId,
    /// The normalized body hash this summary describes.
    pub semantic_hash: Hash128,
    pub parameters: Vec<PhysicalValueFacts>,
    pub result: PhysicalValueFacts,
    /// Direct semantic edges, sorted independently of source and archive order.
    pub calls: Vec<PhysicalCallFacts>,
    pub boundary: PhysicalBoundaryFacts,
}

impl PhysicalSummary {
    /// Builds representation-neutral facts for a function body.
    pub fn of(body: &Body, types: &TypeTable, semantic_hash: Hash128) -> Option<PhysicalSummary> {
        if !matches!(body.kind, BodyKind::Function) {
            return None;
        }

        let arguments: Vec<&Local> = body
            .locals
            .iter()
            .filter(|local| local.role == LocalRole::Argument)
            .collect();
        let by_local: BTreeMap<LocalId, usize> = arguments
            .iter()
            .enumerate()
            .map(|(index, local)| (local.id, index))
            .collect();
        let parameters = arguments
            .iter()
            .map(|local| PhysicalValueFacts::of_type(local.ty, types))
            .collect();
        let result = PhysicalValueFacts::of_type(body.signature.result, types);
        let mut scan = PhysicalScan {
            by_local,
            parameters,
            calls: Vec::new(),
            boundary: PhysicalBoundaryFacts {
                canonical_wrapper_eligible: body.signature.abi.spread.is_none()
                    && !body.signature.abi.caller_location
                    && !matches!(body.semantic, BodySemantic::DropGlue(_)),
                ..PhysicalBoundaryFacts::default()
            },
            types,
        };
        scan.body(body);
        scan.calls.sort();

        Some(PhysicalSummary {
            format: PHYSICAL_SUMMARY_FORMAT,
            item: body.item,
            semantic_hash,
            parameters: scan.parameters,
            result,
            calls: scan.calls,
            boundary: scan.boundary,
        })
    }

    /// Checks the summary schema, identity, body hash, and every derived fact.
    pub fn validate(
        &self,
        body: &Body,
        types: &TypeTable,
        semantic_hash: Hash128,
    ) -> Result<(), PhysicalSummaryValidation> {
        if self.format != PHYSICAL_SUMMARY_FORMAT {
            return Err(PhysicalSummaryValidation::Format);
        }
        if self.item != body.item {
            return Err(PhysicalSummaryValidation::Item);
        }
        if self.semantic_hash != semantic_hash {
            return Err(PhysicalSummaryValidation::Body);
        }
        if Some(self) != PhysicalSummary::of(body, types, semantic_hash).as_ref() {
            return Err(PhysicalSummaryValidation::Facts);
        }
        Ok(())
    }
}

/// Why a physical summary cannot be consumed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhysicalSummaryValidation {
    Format,
    Item,
    Body,
    Facts,
}

impl PhysicalSummaryValidation {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Format => "validation.summary-format",
            Self::Item => "validation.summary-item",
            Self::Body => "validation.summary-body",
            Self::Facts => "validation.summary-facts",
        }
    }
}

/// Use and shape facts for one semantic parameter or result.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct PhysicalValueFacts {
    pub ty: TypeId,
    pub field_reads: u32,
    pub field_writes: u32,
    pub whole_forwards: u32,
    pub escapes: bool,
    pub retained: bool,
    pub identity_observed: bool,
    pub address_observed: bool,
    pub mutable: bool,
    pub copy_back: bool,
    pub slice: bool,
    pub exact_length: Option<u64>,
}

impl PhysicalValueFacts {
    fn of_type(ty: TypeId, types: &TypeTable) -> PhysicalValueFacts {
        let shape = PhysicalTypeFacts::of(ty, types, 0);
        PhysicalValueFacts {
            ty,
            field_reads: 0,
            field_writes: 0,
            whole_forwards: 0,
            escapes: false,
            retained: false,
            identity_observed: false,
            address_observed: false,
            mutable: shape.mutable,
            copy_back: false,
            slice: shape.slice,
            exact_length: shape.exact_length,
        }
    }
}

/// One direct call site expressed only in stable semantic identities and type facts.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct PhysicalCallFacts {
    pub callee: ItemId,
    pub arguments: Vec<PhysicalArgumentFacts>,
    pub result: TypeId,
}

/// One direct call argument before any JavaScript representation is selected.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct PhysicalArgumentFacts {
    pub ty: TypeId,
    /// The source parameter position when this is a whole-value forward.
    pub parameter: Option<u16>,
    pub constant: bool,
    pub slice: bool,
    pub exact_length: Option<u64>,
}

/// Boundaries that can pin a function or require a canonical wrapper.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct PhysicalBoundaryFacts {
    pub canonical_wrapper_eligible: bool,
    pub host: bool,
    pub indirect: bool,
    pub virtual_call: bool,
    pub address_observed: bool,
}

#[derive(Clone, Copy, Default)]
struct PhysicalTypeFacts {
    mutable: bool,
    slice: bool,
    exact_length: Option<u64>,
}

impl PhysicalTypeFacts {
    fn of(ty: TypeId, types: &TypeTable, depth: u8) -> PhysicalTypeFacts {
        if depth > 16 {
            return PhysicalTypeFacts::default();
        }
        let Some(descriptor) = types.get(types.peeled(ty)) else {
            return PhysicalTypeFacts::default();
        };
        match descriptor.kind {
            TypeKind::Reference { pointee, mutable }
            | TypeKind::RawPointer { pointee, mutable } => {
                let mut facts = Self::of(pointee, types, depth + 1);
                facts.mutable |= mutable;
                facts
            }
            TypeKind::Slice(_) => PhysicalTypeFacts {
                slice: true,
                ..PhysicalTypeFacts::default()
            },
            TypeKind::Array { length, .. } => PhysicalTypeFacts {
                exact_length: Some(length),
                ..PhysicalTypeFacts::default()
            },
            _ => PhysicalTypeFacts::default(),
        }
    }
}

#[derive(Clone, Copy)]
enum PhysicalUse {
    Read,
    Write,
    Forward,
    Retain,
    Escape,
    Identity,
    Address,
}

struct PhysicalScan<'a> {
    by_local: BTreeMap<LocalId, usize>,
    parameters: Vec<PhysicalValueFacts>,
    calls: Vec<PhysicalCallFacts>,
    boundary: PhysicalBoundaryFacts,
    types: &'a TypeTable,
}

impl PhysicalScan<'_> {
    fn body(&mut self, body: &Body) {
        let reachable: BTreeSet<BlockId> = reverse_postorder(body).into_iter().collect();
        for block in &body.blocks {
            if !reachable.contains(&block.id) {
                continue;
            }
            for statement in &block.statements {
                self.statement(statement);
            }
            self.terminator(&block.terminator);
        }
        for facts in &mut self.parameters {
            facts.copy_back = facts.mutable && facts.field_writes > 0;
        }
    }

    fn statement(&mut self, statement: &Statement) {
        match &statement.kind {
            StatementKind::Assign { destination, value } => {
                self.place(destination, PhysicalUse::Write);
                self.rvalue(value);
            }
            StatementKind::SetDiscriminant { place, .. } => self.place(place, PhysicalUse::Write),
            StatementKind::Assume(operand) => self.operand(operand, PhysicalUse::Read),
            StatementKind::CopyNonOverlapping {
                source,
                destination,
                count,
            } => {
                self.operand(source, PhysicalUse::Address);
                self.operand(destination, PhysicalUse::Address);
                self.operand(count, PhysicalUse::Read);
                self.boundary.address_observed = true;
            }
            StatementKind::StorageLive(_) | StatementKind::StorageDead(_) | StatementKind::Nop => {}
        }
    }

    fn rvalue(&mut self, value: &Rvalue) {
        match value {
            Rvalue::Use(operand)
            | Rvalue::Repeat { value: operand, .. }
            | Rvalue::WrapUnsafeBinder(operand) => self.operand(operand, PhysicalUse::Read),
            Rvalue::Reference { place, .. }
            | Rvalue::RawPointer { place, .. }
            | Rvalue::Reborrow { place, .. } => {
                self.place(place, PhysicalUse::Address);
                self.boundary.address_observed = true;
            }
            Rvalue::Cast { kind, value, .. } => {
                let use_ = match kind {
                    CastKind::PointerExposeProvenance
                    | CastKind::PointerWithExposedProvenance
                    | CastKind::PtrToPtr
                    | CastKind::FnPtrToPtr => PhysicalUse::Identity,
                    _ => PhysicalUse::Read,
                };
                self.operand(value, use_);
            }
            Rvalue::Binary {
                op, left, right, ..
            } => {
                let use_ = match op {
                    super::semantic::BinaryOp::Eq
                    | super::semantic::BinaryOp::Ne
                    | super::semantic::BinaryOp::Cmp => PhysicalUse::Identity,
                    _ => PhysicalUse::Read,
                };
                self.operand(left, use_);
                self.operand(right, use_);
            }
            Rvalue::Unary { value, .. } => self.operand(value, PhysicalUse::Read),
            Rvalue::Discriminant(place) | Rvalue::CopyForDeref(place) => {
                self.place(place, PhysicalUse::Read)
            }
            Rvalue::Aggregate { operands, .. } => {
                for operand in operands {
                    self.operand(operand, PhysicalUse::Retain);
                }
            }
        }
    }

    fn terminator(&mut self, terminator: &Terminator) {
        match terminator {
            Terminator::Switch { discriminant, .. } => {
                self.operand(discriminant, PhysicalUse::Read)
            }
            Terminator::Call(call) => self.call(call),
            Terminator::Drop { place, .. } => self.place(place, PhysicalUse::Retain),
            Terminator::Assert {
                condition, message, ..
            } => {
                self.operand(condition, PhysicalUse::Read);
                self.assert_message(message);
            }
            Terminator::Goto(_)
            | Terminator::Return
            | Terminator::Unreachable
            | Terminator::UnwindResume
            | Terminator::UnwindTerminate(_)
            | Terminator::FalseEdge { .. }
            | Terminator::FalseUnwind { .. } => {}
        }
    }

    fn call(&mut self, call: &Call) {
        self.operand(&call.function, PhysicalUse::Identity);
        self.place(&call.destination, PhysicalUse::Write);
        let direct = match call.callee {
            Callee::Direct { item, .. } => Some(item),
            Callee::Indirect { .. } => {
                self.boundary.indirect = true;
                None
            }
            Callee::Virtual { .. } => {
                self.boundary.virtual_call = true;
                None
            }
            Callee::Foreign { .. }
            | Callee::Intrinsic { .. }
            | Callee::Runtime { .. }
            | Callee::Nop => None,
        };
        let escaping = direct.is_none() || call.host.is_some();
        if call.host.is_some()
            || matches!(
                call.semantic,
                CallSemantic::Host | CallSemantic::ViewAbi | CallSemantic::RawJavaScript
            )
        {
            self.boundary.host = true;
        }
        for argument in &call.arguments {
            self.operand(
                argument,
                if escaping {
                    PhysicalUse::Escape
                } else {
                    PhysicalUse::Forward
                },
            );
        }
        let Some(callee) = direct else { return };
        let arguments = call
            .arguments
            .iter()
            .map(|argument| {
                let shape = PhysicalTypeFacts::of(argument.ty, self.types, 0);
                PhysicalArgumentFacts {
                    ty: argument.ty,
                    parameter: self.whole_parameter(argument),
                    constant: matches!(
                        argument.kind,
                        OperandKind::Constant(_) | OperandKind::RuntimeCheck(_)
                    ),
                    slice: shape.slice,
                    exact_length: shape.exact_length,
                }
            })
            .collect();
        self.calls.push(PhysicalCallFacts {
            callee,
            arguments,
            result: call.destination.ty,
        });
    }

    fn assert_message(&mut self, message: &AssertMessage) {
        match message {
            AssertMessage::BoundsCheck { length, index }
            | AssertMessage::Overflow {
                left: length,
                right: index,
                ..
            }
            | AssertMessage::MisalignedPointer {
                required: length,
                found: index,
            } => {
                self.operand(length, PhysicalUse::Read);
                self.operand(index, PhysicalUse::Read);
            }
            AssertMessage::OverflowNeg(operand)
            | AssertMessage::DivisionByZero(operand)
            | AssertMessage::RemainderByZero(operand)
            | AssertMessage::InvalidEnum(operand) => self.operand(operand, PhysicalUse::Read),
            AssertMessage::ResumedAfterReturn(_)
            | AssertMessage::ResumedAfterPanic(_)
            | AssertMessage::ResumedAfterDrop(_)
            | AssertMessage::NullPointer
            | AssertMessage::NullReference => {}
        }
    }

    fn operand(&mut self, operand: &Operand, use_: PhysicalUse) {
        if let OperandKind::Copy(place) | OperandKind::Move(place) = &operand.kind {
            self.place(place, use_);
        }
    }

    fn place(&mut self, place: &Place, use_: PhysicalUse) {
        let Some(index) = self.by_local.get(&place.local).copied() else {
            return;
        };
        let facts = &mut self.parameters[index];
        if !place.projection.is_empty() {
            match use_ {
                PhysicalUse::Write => facts.field_writes += 1,
                _ => facts.field_reads += 1,
            }
        }
        match use_ {
            PhysicalUse::Forward if place.projection.is_empty() => facts.whole_forwards += 1,
            PhysicalUse::Retain if place.projection.is_empty() => facts.retained = true,
            PhysicalUse::Escape if place.projection.is_empty() => facts.escapes = true,
            PhysicalUse::Identity if place.projection.is_empty() => facts.identity_observed = true,
            PhysicalUse::Address => facts.address_observed = true,
            PhysicalUse::Read
            | PhysicalUse::Write
            | PhysicalUse::Forward
            | PhysicalUse::Retain
            | PhysicalUse::Escape
            | PhysicalUse::Identity => {}
        }
    }

    fn whole_parameter(&self, operand: &Operand) -> Option<u16> {
        let place = match &operand.kind {
            OperandKind::Copy(place) | OperandKind::Move(place) if place.projection.is_empty() => {
                place
            }
            _ => return None,
        };
        self.by_local
            .get(&place.local)
            .and_then(|index| u16::try_from(*index).ok())
    }
}

/// A coarse classification of an item's body, as a bitset. See the associated constants.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
pub struct ItemClass(pub u16);

impl ItemClass {
    /// A function that makes no calls and whose every local is a scalar (integer, float, bool,
    /// char, unit, never): straight-line arithmetic and control flow.
    pub const SCALAR_LEAF: ItemClass = ItemClass(1 << 0);
    /// Builds or projects into an aggregate (struct, tuple, array, enum, closure).
    pub const AGGREGATE: ItemClass = ItemClass(1 << 1);
    /// Takes or holds a reference.
    pub const REFS: ItemClass = ItemClass(1 << 2);
    /// Takes, holds, or casts a raw pointer.
    pub const POINTERS: ItemClass = ItemClass(1 << 3);
    /// Runs drop glue (`Drop` terminators).
    pub const DROPS: ItemClass = ItemClass(1 << 4);
    /// Has cleanup blocks or unwind edges.
    pub const UNWIND: ItemClass = ItemClass(1 << 5);
    /// Crosses the host boundary: `js!{}`, `#[js_extern]`, view-ABI and collection markers,
    /// runtime shim members, foreign symbols.
    pub const HOST: ItemClass = ItemClass(1 << 6);
    /// Is, builds, or resumes a coroutine.
    pub const COROUTINE: ItemClass = ItemClass(1 << 7);
    pub const STATIC: ItemClass = ItemClass(1 << 8);
    pub const VTABLE: ItemClass = ItemClass(1 << 9);

    const NAMES: [(ItemClass, &'static str); 10] = [
        (Self::SCALAR_LEAF, "scalar-leaf"),
        (Self::AGGREGATE, "aggregate"),
        (Self::REFS, "refs"),
        (Self::POINTERS, "pointers"),
        (Self::DROPS, "drops"),
        (Self::UNWIND, "unwind"),
        (Self::HOST, "host"),
        (Self::COROUTINE, "coroutine"),
        (Self::STATIC, "static"),
        (Self::VTABLE, "vtable"),
    ];

    pub const fn empty() -> ItemClass {
        ItemClass(0)
    }

    pub const fn contains(self, other: ItemClass) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: ItemClass) -> ItemClass {
        ItemClass(self.0 | other.0)
    }

    pub fn insert(&mut self, other: ItemClass) {
        self.0 |= other.0;
    }

    /// The set flags by name, in bit order.
    pub fn names(self) -> Vec<&'static str> {
        Self::NAMES
            .iter()
            .filter(|(flag, _)| self.contains(*flag))
            .map(|(_, name)| *name)
            .collect()
    }

    /// Every flag set at once: the promoted set a lane runs with by default.
    pub const fn all() -> ItemClass {
        let mut bits = 0u16;
        let mut index = 0;
        while index < Self::NAMES.len() {
            bits |= Self::NAMES[index].0 .0;
            index += 1;
        }
        ItemClass(bits)
    }

    /// Parses `js-lane-classes`: `all`, `none`, or a comma list of the names [`Self::names`]
    /// writes. `None` for a name no flag has, which the caller reports as a bad option value.
    pub fn parse(list: &str) -> Option<ItemClass> {
        match list {
            "all" => return Some(Self::all()),
            "none" => return Some(Self::empty()),
            _ => {}
        }
        let mut class = ItemClass::empty();
        for name in list.split(',') {
            let (flag, _) = Self::NAMES.iter().find(|(_, known)| *known == name)?;
            class.insert(*flag);
        }
        Some(class)
    }

    /// Classifies `body`. `types` resolves the locals' kinds; a local whose type the table does not
    /// describe is treated as non-scalar.
    pub fn of(body: &Body, types: &TypeTable) -> ItemClass {
        let mut class = ItemClass::empty();
        match &body.kind {
            BodyKind::Function => {}
            BodyKind::Static { .. } => class.insert(Self::STATIC),
            BodyKind::Vtable { .. } => class.insert(Self::VTABLE),
        }

        let mut calls = false;
        let mut all_scalar = true;
        for local in &body.locals {
            match types.get(local.ty).map(|descriptor| &descriptor.kind) {
                Some(
                    TypeKind::Bool
                    | TypeKind::Char
                    | TypeKind::Integer { .. }
                    | TypeKind::Float { .. }
                    | TypeKind::Never,
                ) => {}
                Some(TypeKind::Tuple(fields)) if fields.is_empty() => {}
                Some(TypeKind::Reference { .. }) => {
                    all_scalar = false;
                    class.insert(Self::REFS);
                }
                Some(TypeKind::RawPointer { .. }) => {
                    all_scalar = false;
                    class.insert(Self::POINTERS);
                }
                Some(TypeKind::Coroutine { .. }) => {
                    all_scalar = false;
                    class.insert(Self::COROUTINE);
                }
                _ => all_scalar = false,
            }
        }

        for block in &body.blocks {
            if block.cleanup {
                class.insert(Self::UNWIND);
            }
            for statement in &block.statements {
                match &statement.kind {
                    StatementKind::Assign { value, .. } => classify_rvalue(value, &mut class),
                    StatementKind::SetDiscriminant { .. } => class.insert(Self::AGGREGATE),
                    StatementKind::CopyNonOverlapping { .. } => class.insert(Self::POINTERS),
                    StatementKind::StorageLive(_)
                    | StatementKind::StorageDead(_)
                    | StatementKind::Assume(_)
                    | StatementKind::Nop => {}
                }
            }
            match &block.terminator {
                Terminator::Call(call) => {
                    calls = true;
                    let host = call.host.is_some()
                        || matches!(
                            call.semantic,
                            CallSemantic::Host
                                | CallSemantic::ViewAbi
                                | CallSemantic::RawJavaScript
                        )
                        || matches!(call.callee, Callee::Foreign { .. } | Callee::Runtime { .. });
                    if host {
                        class.insert(Self::HOST);
                    }
                    classify_unwind(call.unwind, &mut class);
                }
                Terminator::Drop { unwind, .. } => {
                    class.insert(Self::DROPS);
                    classify_unwind(*unwind, &mut class);
                }
                Terminator::Assert {
                    message, unwind, ..
                } => {
                    if matches!(
                        message,
                        AssertMessage::ResumedAfterReturn(_)
                            | AssertMessage::ResumedAfterPanic(_)
                            | AssertMessage::ResumedAfterDrop(_)
                    ) {
                        class.insert(Self::COROUTINE);
                    }
                    classify_unwind(*unwind, &mut class);
                }
                Terminator::UnwindResume | Terminator::UnwindTerminate(_) => {
                    class.insert(Self::UNWIND)
                }
                Terminator::FalseUnwind { unwind, .. } => classify_unwind(*unwind, &mut class),
                Terminator::Goto(_)
                | Terminator::Switch { .. }
                | Terminator::Return
                | Terminator::Unreachable
                | Terminator::FalseEdge { .. } => {}
            }
        }

        if matches!(body.kind, BodyKind::Function) && !calls && all_scalar {
            class.insert(Self::SCALAR_LEAF);
        }
        class
    }
}

fn classify_unwind(unwind: Unwind, class: &mut ItemClass) {
    if matches!(unwind, Unwind::Cleanup(_)) {
        class.insert(ItemClass::UNWIND);
    }
}

fn classify_operand(operand: &Operand, class: &mut ItemClass) {
    if let OperandKind::Copy(place) | OperandKind::Move(place) = &operand.kind {
        if !place.projection.is_empty() {
            class.insert(ItemClass::AGGREGATE);
        }
    }
}

fn classify_rvalue(value: &Rvalue, class: &mut ItemClass) {
    match value {
        Rvalue::Use(operand) | Rvalue::Repeat { value: operand, .. } => {
            classify_operand(operand, class)
        }
        Rvalue::Reference { .. } | Rvalue::Reborrow { .. } => class.insert(ItemClass::REFS),
        Rvalue::RawPointer { .. } => class.insert(ItemClass::POINTERS),
        Rvalue::Cast { kind, value, .. } => {
            if matches!(
                kind,
                CastKind::PointerExposeProvenance
                    | CastKind::PointerWithExposedProvenance
                    | CastKind::PtrToPtr
                    | CastKind::FnPtrToPtr
            ) {
                class.insert(ItemClass::POINTERS);
            }
            classify_operand(value, class);
        }
        Rvalue::Binary { left, right, .. } => {
            classify_operand(left, class);
            classify_operand(right, class);
        }
        Rvalue::Unary { value, .. } | Rvalue::WrapUnsafeBinder(value) => {
            classify_operand(value, class)
        }
        Rvalue::Discriminant(_) | Rvalue::CopyForDeref(_) => class.insert(ItemClass::AGGREGATE),
        Rvalue::Aggregate { kind, operands, .. } => {
            class.insert(ItemClass::AGGREGATE);
            match kind {
                AggregateKind::Coroutine(_) | AggregateKind::CoroutineClosure(_) => {
                    class.insert(ItemClass::COROUTINE)
                }
                AggregateKind::RawPointer { .. } => class.insert(ItemClass::POINTERS),
                AggregateKind::Array(_)
                | AggregateKind::Tuple
                | AggregateKind::Adt { .. }
                | AggregateKind::Closure(_) => {}
            }
            for operand in operands {
                classify_operand(operand, class);
            }
        }
    }
}

impl std::fmt::Display for ItemClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names = self.names();
        if names.is_empty() {
            return f.write_str("-");
        }
        f.write_str(&names.join("+"))
    }
}

// -------------------------------------------------------------------------------------------
// The normalized body hash (W4)
// -------------------------------------------------------------------------------------------

/// A body rewritten into the one canonical form two objects have to agree on.
///
/// [`ItemSummary::body_hash`] is the hash of the body's *postcard bytes*: it addresses the blob in
/// S3 and answers "are these the same bytes", which is what a content-addressed blob table needs
/// and what a dedup rule must not use. Two crates that codegen one monomorphic item run the same
/// MIR through the same encoder, but not necessarily through the same MIR pipeline: the sysroot is
/// built with `-Zinline-mir` and an application crate is not, and the passes that do run leave
/// different numbers of storage markers, nops, false edges, dead blocks and dead locals behind.
/// v1 hashed all of that and declined availability on any difference — the 74 duplicate conflicts
/// the FingerprintJS report shows.
///
/// The normal form drops exactly what cannot change what the body does:
///
/// 1. `StatementKind::{StorageLive, StorageDead, Nop}` are dropped. Storage markers are liveness
///    annotations for a stack the JavaScript lowering does not have, and a nop is a hole a pass
///    left.
/// 2. `Terminator::FalseEdge { real, .. }` and `Terminator::FalseUnwind { real, .. }` become
///    `Goto(real)`. Both exist for borrowck; the imaginary edge and the false unwind edge are not
///    taken at run time.
/// 3. Blocks are renumbered in reverse postorder from the entry block over the successor relation
///    *after* (2), so two identical CFGs numbered differently normalize equal. Blocks the walk does
///    not reach are dropped: a pass that made a block unreachable and a pass that also deleted it
///    agree here.
/// 4. Locals are renumbered: the return local and the arguments first, in declaration order (their
///    positions are ABI), then every other local in the order of its first mention in the walk of
///    (3). A local no surviving statement mentions is dropped.
/// 5. Origins take no part: they are not in [`Body`] at all. The one source position a body carries
///    semantically, `Call::track_caller`, is kept — it is spelled into a `Location` value.
/// 6. Interned strings, byte blobs and source facts are re-interned **by content** in first-mention
///    order and hashed as content, so ids assigned by two different codegen units compare equal.
/// 7. Types are [`TypeId`]s, which are already global stable hashes, and are hashed as they are.
///
/// The result is a [`Body`] again, so a tool can walk two normal forms in parallel and name the
/// first field that differs; see `examples/jir-conflicts.rs`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedBody {
    /// The rewritten body. Its [`StrId`]/[`BytesId`]/[`SourceFactId`]s index the tables below, not
    /// the object's interns.
    pub body: Body,
    /// The strings the body mentions, in first-mention order.
    pub strings: Vec<String>,
    /// The byte blobs the body mentions, in first-mention order.
    pub bytes: Vec<Vec<u8>>,
    /// The source facts the body mentions, in first-mention order. Their `file` is an index into
    /// [`NormalizedBody::strings`].
    pub source_facts: Vec<SourceFact>,
}

/// The digest of a body's normal form: the hash the linker's identity rule compares.
///
/// See [`NormalizedBody`] for what the normal form is and why the blob hash cannot be used here.
pub fn semantic_hash(body: &Body, interns: &Interns) -> Hash128 {
    NormalizedBody::of(body, interns).hash()
}

impl NormalizedBody {
    /// Normalizes `body`. `interns` resolves the ids the body carries; an id the tables do not
    /// describe re-interns as a distinct placeholder rather than failing, so a truncated intern
    /// table degrades into "these two bodies differ" and never into a panic.
    pub fn of(body: &Body, interns: &Interns) -> NormalizedBody {
        Normalizer::new(interns).run(body)
    }

    /// The digest. Domain-separated, then the postcard bytes of the rewritten body, then the
    /// content tables with their lengths, so no two normal forms share a byte stream.
    pub fn hash(&self) -> Hash128 {
        let mut hasher = Hasher128::new();
        hasher.write(HASH_DOMAIN);
        hasher.write(&(JIR_SCHEMA as u64).to_le_bytes());
        let encoded = postcard::to_allocvec(&self.body).unwrap_or_default();
        hasher.write(&(encoded.len() as u64).to_le_bytes());
        hasher.write(&encoded);
        hasher.write(&(self.strings.len() as u64).to_le_bytes());
        for string in &self.strings {
            hasher.write(&(string.len() as u64).to_le_bytes());
            hasher.write(string.as_bytes());
        }
        hasher.write(&(self.bytes.len() as u64).to_le_bytes());
        for blob in &self.bytes {
            hasher.write(&(blob.len() as u64).to_le_bytes());
            hasher.write(blob);
        }
        hasher.write(&(self.source_facts.len() as u64).to_le_bytes());
        for fact in &self.source_facts {
            hasher.write(&fact.file.0.to_le_bytes());
            hasher.write(&fact.line.to_le_bytes());
            hasher.write(&fact.col.to_le_bytes());
        }
        hasher.finish()
    }

    /// The string behind a normalized [`StrId`].
    pub fn str(&self, id: StrId) -> Option<&str> {
        self.strings.get(id.0 as usize).map(String::as_str)
    }
}

/// Domain separation, so a normalized body hash can never collide with a blob hash by accident.
const HASH_DOMAIN: &[u8] = b"rcgjs.jir.body.normalized\0";

/// The rewriting state: one pass over a body, assigning dense ids as it goes.
struct Normalizer<'a> {
    interns: &'a Interns,
    blocks: BTreeMap<u32, BlockId>,
    locals: BTreeMap<u32, LocalId>,
    next_local: u32,
    strings: Vec<String>,
    string_index: BTreeMap<u32, StrId>,
    bytes: Vec<Vec<u8>>,
    bytes_index: BTreeMap<u32, BytesId>,
    source_facts: Vec<SourceFact>,
    source_fact_index: BTreeMap<u32, SourceFactId>,
}

impl<'a> Normalizer<'a> {
    fn new(interns: &'a Interns) -> Normalizer<'a> {
        Normalizer {
            interns,
            blocks: BTreeMap::new(),
            locals: BTreeMap::new(),
            next_local: 0,
            strings: Vec::new(),
            string_index: BTreeMap::new(),
            bytes: Vec::new(),
            bytes_index: BTreeMap::new(),
            source_facts: Vec::new(),
            source_fact_index: BTreeMap::new(),
        }
    }

    fn run(mut self, body: &Body) -> NormalizedBody {
        let order = reverse_postorder(body);
        for (position, id) in order.iter().enumerate() {
            self.blocks.insert(id.0, BlockId(position as u32));
        }
        // The return local and the arguments hold their positions: they are the JavaScript
        // parameter shape, not an internal numbering.
        for local in &body.locals {
            if matches!(local.role, LocalRole::Return | LocalRole::Argument) {
                self.local(local.id);
            }
        }

        let lookup: BTreeMap<u32, &BasicBlock> = body
            .blocks
            .iter()
            .map(|block| (block.id.0, block))
            .collect();
        let mut blocks = Vec::with_capacity(order.len());
        for (position, id) in order.iter().enumerate() {
            let Some(block) = lookup.get(&id.0) else {
                continue;
            };
            let statements = block
                .statements
                .iter()
                .filter(|statement| {
                    !matches!(
                        statement.kind,
                        StatementKind::StorageLive(_)
                            | StatementKind::StorageDead(_)
                            | StatementKind::Nop
                    )
                })
                .map(|statement| Statement {
                    kind: self.statement(&statement.kind),
                })
                .collect();
            let terminator = self.terminator(&block.terminator);
            blocks.push(BasicBlock {
                id: BlockId(position as u32),
                cleanup: block.cleanup,
                statements,
                terminator,
            });
        }

        // Only the locals the surviving body mentions, in their new order.
        let mut locals: Vec<Local> = body
            .locals
            .iter()
            .filter_map(|local| {
                self.locals.get(&local.id.0).map(|id| Local {
                    id: *id,
                    role: local.role,
                    ty: local.ty,
                })
            })
            .collect();
        locals.sort_by_key(|local| local.id.0);

        let signature = Signature {
            arguments: body.signature.arguments.clone(),
            result: body.signature.result,
            abi: AbiSignature {
                parameters: body.signature.abi.parameters,
                spread: body.signature.abi.spread.map(|spread| SpreadArgument {
                    local: self.local(spread.local),
                    arity: spread.arity,
                }),
                caller_location: body.signature.abi.caller_location,
                inline_never: body.signature.abi.inline_never,
            },
        };
        let kind = self.body_kind(&body.kind);

        NormalizedBody {
            body: Body {
                item: body.item,
                kind,
                semantic: body.semantic,
                signature,
                locals,
                blocks,
            },
            strings: self.strings,
            bytes: self.bytes,
            source_facts: self.source_facts,
        }
    }

    // -- id remapping --

    fn block(&self, id: BlockId) -> BlockId {
        // A branch to a block the walk did not reach cannot be taken; it keeps a distinct id far
        // above the reachable range so it stays a difference rather than an aliasing accident.
        self.blocks
            .get(&id.0)
            .copied()
            .unwrap_or(BlockId(u32::MAX - id.0.min(u32::MAX - 1)))
    }

    fn local(&mut self, id: LocalId) -> LocalId {
        if let Some(mapped) = self.locals.get(&id.0) {
            return *mapped;
        }
        let mapped = LocalId(self.next_local);
        self.next_local += 1;
        self.locals.insert(id.0, mapped);
        mapped
    }

    fn string(&mut self, id: StrId) -> StrId {
        if let Some(mapped) = self.string_index.get(&id.0) {
            return *mapped;
        }
        let value = match self.interns.str(id) {
            Some(value) => value.to_owned(),
            None => format!("\u{0}unresolved-string:{}", id.0),
        };
        let mapped = StrId(self.strings.len() as u32);
        self.strings.push(value);
        self.string_index.insert(id.0, mapped);
        mapped
    }

    fn blob(&mut self, id: BytesId) -> BytesId {
        if let Some(mapped) = self.bytes_index.get(&id.0) {
            return *mapped;
        }
        let value = match self.interns.bytes(id) {
            Some(value) => value.to_vec(),
            None => format!("\u{0}unresolved-bytes:{}", id.0).into_bytes(),
        };
        let mapped = BytesId(self.bytes.len() as u32);
        self.bytes.push(value);
        self.bytes_index.insert(id.0, mapped);
        mapped
    }

    fn source_fact(&mut self, id: SourceFactId) -> SourceFactId {
        if let Some(mapped) = self.source_fact_index.get(&id.0) {
            return *mapped;
        }
        let fact = self.interns.source_fact(id).cloned();
        let fact = match fact {
            Some(fact) => SourceFact {
                file: self.string(fact.file),
                line: fact.line,
                col: fact.col,
            },
            None => SourceFact {
                file: self.string(StrId(u32::MAX)),
                line: u32::MAX,
                col: id.0,
            },
        };
        let mapped = SourceFactId(self.source_facts.len() as u32);
        self.source_facts.push(fact);
        self.source_fact_index.insert(id.0, mapped);
        mapped
    }

    // -- the walk --

    fn body_kind(&mut self, kind: &BodyKind) -> BodyKind {
        match kind {
            BodyKind::Function => BodyKind::Function,
            BodyKind::Static { init, mutable } => BodyKind::Static {
                init: self.const_tree(init),
                mutable: *mutable,
            },
            BodyKind::Vtable {
                concrete,
                trait_ref,
                entries,
            } => BodyKind::Vtable {
                concrete: *concrete,
                trait_ref: *trait_ref,
                entries: entries
                    .iter()
                    .map(|entry| self.vtable_entry(entry))
                    .collect(),
            },
        }
    }

    fn vtable_entry(&mut self, entry: &VtableEntry) -> VtableEntry {
        match entry {
            VtableEntry::Drop(target) => {
                VtableEntry::Drop(target.map(|target| self.direct_item(target)))
            }
            VtableEntry::Method(target) => VtableEntry::Method(self.direct_item(*target)),
            VtableEntry::TraitVPtr(name) => VtableEntry::TraitVPtr(self.string(*name)),
            VtableEntry::Size(_) | VtableEntry::Align(_) | VtableEntry::Vacant => *entry,
        }
    }

    fn direct_item(&mut self, target: DirectItemRef) -> DirectItemRef {
        DirectItemRef {
            item: target.item,
            link_name: self.string(target.link_name),
        }
    }

    fn drop_glue(&mut self, glue: DropGlue) -> DropGlue {
        match glue {
            DropGlue::Direct { item, link_name } => DropGlue::Direct {
                item,
                link_name: self.string(link_name),
            },
            other => other,
        }
    }

    fn statement(&mut self, kind: &StatementKind) -> StatementKind {
        match kind {
            StatementKind::Assign { destination, value } => StatementKind::Assign {
                destination: self.place(destination),
                value: self.rvalue(value),
            },
            StatementKind::SetDiscriminant { place, variant } => StatementKind::SetDiscriminant {
                place: self.place(place),
                variant: *variant,
            },
            StatementKind::Assume(operand) => StatementKind::Assume(self.operand(operand)),
            StatementKind::CopyNonOverlapping {
                source,
                destination,
                count,
            } => StatementKind::CopyNonOverlapping {
                source: self.operand(source),
                destination: self.operand(destination),
                count: self.operand(count),
            },
            // Dropped before this is reached; reproduced so the match stays exhaustive.
            StatementKind::StorageLive(local) => StatementKind::StorageLive(self.local(*local)),
            StatementKind::StorageDead(local) => StatementKind::StorageDead(self.local(*local)),
            StatementKind::Nop => StatementKind::Nop,
        }
    }

    fn place(&mut self, place: &Place) -> Place {
        Place {
            local: self.local(place.local),
            projection: place
                .projection
                .iter()
                .map(|element| match element {
                    Projection::Index(local) => Projection::Index(self.local(*local)),
                    other => *other,
                })
                .collect(),
            ty: place.ty,
        }
    }

    fn operand(&mut self, operand: &Operand) -> Operand {
        Operand {
            ty: operand.ty,
            kind: match &operand.kind {
                OperandKind::Copy(place) => OperandKind::Copy(self.place(place)),
                OperandKind::Move(place) => OperandKind::Move(self.place(place)),
                OperandKind::Constant(constant) => OperandKind::Constant(self.constant(constant)),
                OperandKind::RuntimeCheck(value) => OperandKind::RuntimeCheck(*value),
            },
        }
    }

    fn constant(&mut self, constant: &Constant) -> Constant {
        match constant {
            Constant::String(id) => Constant::String(self.string(*id)),
            Constant::Pointer { target, offset } => Constant::Pointer {
                target: self.global(*target),
                offset: *offset,
            },
            Constant::Memory(tree) => Constant::Memory(self.const_tree(tree)),
            Constant::ZeroSized => Constant::ZeroSized,
            Constant::Scalar { bits, bytes } => Constant::Scalar {
                bits: *bits,
                bytes: *bytes,
            },
        }
    }

    fn const_tree(&mut self, tree: &ConstTree) -> ConstTree {
        match tree {
            ConstTree::Str(id) => ConstTree::Str(self.string(*id)),
            ConstTree::Aggregate(fields) => {
                ConstTree::Aggregate(fields.iter().map(|field| self.const_tree(field)).collect())
            }
            ConstTree::Variant { variant, fields } => ConstTree::Variant {
                variant: *variant,
                fields: fields.iter().map(|field| self.const_tree(field)).collect(),
            },
            ConstTree::Reference(inner) => ConstTree::Reference(Box::new(self.const_tree(inner))),
            ConstTree::Dyn {
                data,
                concrete,
                vtable,
            } => ConstTree::Dyn {
                data: Box::new(self.const_tree(data)),
                concrete: *concrete,
                vtable: self.string(*vtable),
            },
            ConstTree::Slice(elements) => ConstTree::Slice(
                elements
                    .iter()
                    .map(|element| self.const_tree(element))
                    .collect(),
            ),
            ConstTree::Global { target, offset } => ConstTree::Global {
                target: self.global(*target),
                offset: *offset,
            },
            ConstTree::Unit => ConstTree::Unit,
            ConstTree::Scalar { bits, bytes } => ConstTree::Scalar {
                bits: *bits,
                bytes: *bytes,
            },
            ConstTree::Address(address) => ConstTree::Address(*address),
        }
    }

    fn global(&mut self, target: GlobalReference) -> GlobalReference {
        match target {
            GlobalReference::Function { item, link_name } => GlobalReference::Function {
                item,
                link_name: self.string(link_name),
            },
            GlobalReference::Static {
                definition,
                link_name,
            } => GlobalReference::Static {
                definition,
                link_name: self.string(link_name),
            },
            GlobalReference::Type(ty) => GlobalReference::Type(ty),
        }
    }

    fn rvalue(&mut self, value: &Rvalue) -> Rvalue {
        match value {
            Rvalue::Use(operand) => Rvalue::Use(self.operand(operand)),
            Rvalue::Repeat { value, count } => Rvalue::Repeat {
                value: self.operand(value),
                count: *count,
            },
            Rvalue::Reference { kind, place } => Rvalue::Reference {
                kind: *kind,
                place: self.place(place),
            },
            Rvalue::RawPointer { kind, place } => Rvalue::RawPointer {
                kind: *kind,
                place: self.place(place),
            },
            Rvalue::Cast {
                kind,
                value,
                result,
            } => Rvalue::Cast {
                kind: *kind,
                value: self.operand(value),
                result: *result,
            },
            Rvalue::Binary { op, left, right } => Rvalue::Binary {
                op: *op,
                left: self.operand(left),
                right: self.operand(right),
            },
            Rvalue::Unary { op, value } => Rvalue::Unary {
                op: *op,
                value: self.operand(value),
            },
            Rvalue::Discriminant(place) => Rvalue::Discriminant(self.place(place)),
            Rvalue::Aggregate {
                kind,
                result,
                operands,
            } => Rvalue::Aggregate {
                kind: kind.clone(),
                result: *result,
                operands: operands
                    .iter()
                    .map(|operand| self.operand(operand))
                    .collect(),
            },
            Rvalue::CopyForDeref(place) => Rvalue::CopyForDeref(self.place(place)),
            Rvalue::WrapUnsafeBinder(operand) => Rvalue::WrapUnsafeBinder(self.operand(operand)),
            Rvalue::Reborrow { mutable, place } => Rvalue::Reborrow {
                mutable: *mutable,
                place: self.place(place),
            },
        }
    }

    fn unwind(&mut self, unwind: Unwind) -> Unwind {
        match unwind {
            Unwind::Cleanup(block) => Unwind::Cleanup(self.block(block)),
            other => other,
        }
    }

    fn terminator(&mut self, terminator: &Terminator) -> Terminator {
        match terminator {
            Terminator::Goto(target) => Terminator::Goto(self.block(*target)),
            Terminator::Switch {
                discriminant,
                targets,
                otherwise,
            } => Terminator::Switch {
                discriminant: self.operand(discriminant),
                targets: targets
                    .iter()
                    .map(|(value, target)| (*value, self.block(*target)))
                    .collect(),
                otherwise: self.block(*otherwise),
            },
            Terminator::Return => Terminator::Return,
            Terminator::Unreachable => Terminator::Unreachable,
            Terminator::Call(call) => Terminator::Call(self.call(call)),
            Terminator::Drop {
                place,
                target,
                unwind,
                replace,
                async_drop,
                glue,
            } => Terminator::Drop {
                place: self.place(place),
                target: self.block(*target),
                unwind: self.unwind(*unwind),
                replace: *replace,
                async_drop: async_drop.map(|block| self.block(block)),
                glue: self.drop_glue(*glue),
            },
            Terminator::Assert {
                condition,
                expected,
                message,
                target,
                unwind,
            } => Terminator::Assert {
                condition: self.operand(condition),
                expected: *expected,
                message: self.assert_message(message),
                target: self.block(*target),
                unwind: self.unwind(*unwind),
            },
            Terminator::UnwindResume => Terminator::UnwindResume,
            Terminator::UnwindTerminate(reason) => Terminator::UnwindTerminate(*reason),
            // Borrowck edges: not taken at run time, so both become the edge that is.
            Terminator::FalseEdge { real, .. } => Terminator::Goto(self.block(*real)),
            Terminator::FalseUnwind { real, .. } => Terminator::Goto(self.block(*real)),
        }
    }

    fn assert_message(&mut self, message: &AssertMessage) -> AssertMessage {
        match message {
            AssertMessage::BoundsCheck { length, index } => AssertMessage::BoundsCheck {
                length: self.operand(length),
                index: self.operand(index),
            },
            AssertMessage::Overflow { op, left, right } => AssertMessage::Overflow {
                op: *op,
                left: self.operand(left),
                right: self.operand(right),
            },
            AssertMessage::OverflowNeg(operand) => {
                AssertMessage::OverflowNeg(self.operand(operand))
            }
            AssertMessage::DivisionByZero(operand) => {
                AssertMessage::DivisionByZero(self.operand(operand))
            }
            AssertMessage::RemainderByZero(operand) => {
                AssertMessage::RemainderByZero(self.operand(operand))
            }
            AssertMessage::MisalignedPointer { required, found } => {
                AssertMessage::MisalignedPointer {
                    required: self.operand(required),
                    found: self.operand(found),
                }
            }
            AssertMessage::InvalidEnum(operand) => {
                AssertMessage::InvalidEnum(self.operand(operand))
            }
            AssertMessage::ResumedAfterReturn(kind) => AssertMessage::ResumedAfterReturn(*kind),
            AssertMessage::ResumedAfterPanic(kind) => AssertMessage::ResumedAfterPanic(*kind),
            AssertMessage::ResumedAfterDrop(kind) => AssertMessage::ResumedAfterDrop(*kind),
            AssertMessage::NullPointer => AssertMessage::NullPointer,
            AssertMessage::NullReference => AssertMessage::NullReference,
        }
    }

    fn call(&mut self, call: &Call) -> Call {
        Call {
            function: self.operand(&call.function),
            callee: self.callee(&call.callee),
            // A `str` call-site identity (`StandardOperation::is_str`) is only a lowering hint
            // for `js-str=bytes`; the link step forgets it in every other mode, so it is no part
            // of what the body means and the hash is the same with or without it.
            semantic: match call.semantic {
                CallSemantic::Standard(operation) if operation.is_str() => CallSemantic::Ordinary,
                semantic => semantic,
            },
            arguments: call
                .arguments
                .iter()
                .map(|argument| self.operand(argument))
                .collect(),
            destination: self.place(&call.destination),
            target: call.target.map(|block| self.block(block)),
            unwind: self.unwind(call.unwind),
            track_caller: call.track_caller.map(|fact| self.source_fact(fact)),
            host: call.host.as_ref().map(|host| self.host(host)),
        }
    }

    fn callee(&mut self, callee: &Callee) -> Callee {
        match callee {
            Callee::Direct { item, link_name } => Callee::Direct {
                item: *item,
                link_name: self.string(*link_name),
            },
            Callee::Foreign { definition, symbol } => Callee::Foreign {
                definition: *definition,
                symbol: self.string(*symbol),
            },
            Callee::Indirect { signature } => Callee::Indirect {
                signature: *signature,
            },
            Callee::Intrinsic {
                name,
                types,
                fallback,
            } => Callee::Intrinsic {
                name: self.string(*name),
                types: types.clone(),
                fallback: fallback.map(|target| self.direct_item(target)),
            },
            Callee::Virtual { slot, receiver } => Callee::Virtual {
                slot: *slot,
                receiver: *receiver,
            },
            Callee::Runtime { member, sysroot } => Callee::Runtime {
                member: self.string(*member),
                sysroot: *sysroot,
            },
            Callee::Nop => Callee::Nop,
        }
    }

    fn helpers(
        &mut self,
        helpers: &[(StrId, crate::jir::semantic::DirectItemRef)],
    ) -> Vec<(StrId, crate::jir::semantic::DirectItemRef)> {
        helpers
            .iter()
            .map(|(name, item)| {
                (
                    self.string(*name),
                    crate::jir::semantic::DirectItemRef {
                        item: item.item,
                        link_name: self.string(item.link_name),
                    },
                )
            })
            .collect()
    }

    fn keyed_callable(&mut self, callable: &KeyedListCallable) -> KeyedListCallable {
        KeyedListCallable {
            ty: callable.ty,
            body: callable.body.map(|body| self.direct_item(body)),
            cleanup: self.drop_glue(callable.cleanup),
            source: callable.source.map(|source| self.source_fact(source)),
        }
    }

    fn host(&mut self, host: &HostPayload) -> HostPayload {
        match host {
            HostPayload::Extern(id) => HostPayload::Extern(self.blob(*id)),
            HostPayload::Block(id) => HostPayload::Block(self.blob(*id)),
            HostPayload::Marker {
                section,
                def,
                template,
                helpers,
            } => HostPayload::Marker {
                section: self.string(*section),
                def: *def,
                template: template.as_ref().map(|tree| self.const_tree(tree)),
                helpers: self.helpers(helpers),
            },
            HostPayload::ViewAbi {
                marker,
                def,
                helpers,
            } => HostPayload::ViewAbi {
                marker: self.string(*marker),
                def: *def,
                helpers: self.helpers(helpers),
            },
            HostPayload::Map { marker } => HostPayload::Map {
                marker: self.string(*marker),
            },
            HostPayload::KeyedList(payload) => HostPayload::KeyedList(Box::new(KeyedListPayload {
                format: payload.format,
                site: payload.site.map(|source| self.source_fact(source)),
                def: payload.def,
                collection_ty: payload.collection_ty,
                item_ty: payload.item_ty,
                key_ty: payload.key_ty,
                row_ty: payload.row_ty,
                collection: self.keyed_callable(&payload.collection),
                key: self.keyed_callable(&payload.key),
                row: self.keyed_callable(&payload.row),
            })),
        }
    }
}

/// The successors of a terminator, in the order the schema declares them, with the two borrowck
/// terminators already reduced to their real edge.
pub fn successors(terminator: &Terminator) -> Vec<BlockId> {
    let mut targets = Vec::new();
    let cleanup = |unwind: Unwind, targets: &mut Vec<BlockId>| {
        if let Unwind::Cleanup(block) = unwind {
            targets.push(block);
        }
    };
    match terminator {
        Terminator::Goto(target) => targets.push(*target),
        Terminator::Switch {
            targets: arms,
            otherwise,
            ..
        } => {
            targets.extend(arms.iter().map(|(_, target)| *target));
            targets.push(*otherwise);
        }
        Terminator::Call(call) => {
            targets.extend(call.target);
            cleanup(call.unwind, &mut targets);
        }
        Terminator::Drop {
            target,
            unwind,
            async_drop,
            ..
        } => {
            targets.push(*target);
            targets.extend(*async_drop);
            cleanup(*unwind, &mut targets);
        }
        Terminator::Assert { target, unwind, .. } => {
            targets.push(*target);
            cleanup(*unwind, &mut targets);
        }
        Terminator::FalseEdge { real, .. } | Terminator::FalseUnwind { real, .. } => {
            targets.push(*real)
        }
        Terminator::Return
        | Terminator::Unreachable
        | Terminator::UnwindResume
        | Terminator::UnwindTerminate(_) => {}
    }
    targets
}

/// The reachable blocks in reverse postorder from the entry block (`blocks[0]`).
///
/// Reverse postorder rather than the order the blocks happen to be stored in: two passes that
/// produce the same CFG may store it differently, and the walk order is a property of the graph.
pub fn reverse_postorder(body: &Body) -> Vec<BlockId> {
    let lookup: BTreeMap<u32, &BasicBlock> = body
        .blocks
        .iter()
        .map(|block| (block.id.0, block))
        .collect();
    let Some(entry) = body.blocks.first().map(|block| block.id) else {
        return Vec::new();
    };
    let mut postorder = Vec::new();
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    // (block, index of the next successor to visit); an explicit stack, because a body can be
    // deeper than the native one.
    let mut stack: Vec<(BlockId, usize)> = Vec::new();
    seen.insert(entry.0);
    stack.push((entry, 0));
    while let Some((block, index)) = stack.pop() {
        let children = lookup
            .get(&block.0)
            .map(|block| successors(&block.terminator))
            .unwrap_or_default();
        if index < children.len() {
            stack.push((block, index + 1));
            let child = children[index];
            if seen.insert(child.0) {
                stack.push((child, 0));
            }
        } else {
            postorder.push(block);
        }
    }
    postorder.reverse();
    postorder
}

// -------------------------------------------------------------------------------------------
// The identity rule (W4)
// -------------------------------------------------------------------------------------------

/// Which of the two summaries a decision keeps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Left,
    Right,
}

/// What the linker does with two summaries that claim one [`SemanticKey`].
///
/// The rule, in full:
///
/// | situation | decision |
/// | --- | --- |
/// | headers disagree ([`merge_headers`], checked once per object pair) | [`MergeDecision::Error`] |
/// | different [`SemanticKey`], same `link_name` | [`MergeDecision::Error`] |
/// | same key, equal normalized hash | [`MergeDecision::Merge`] — keep either body, union the origins |
/// | same key, one body only | [`MergeDecision::Available`] — keep the side that has one |
/// | same key, different normalized hashes, cleanup edges on one side only | [`MergeDecision::Choose`] — keep the side with cleanup edges |
/// | same key, different normalized hashes, otherwise | [`MergeDecision::Choose`] — keep the smaller hash |
///
/// A body difference is never unavailability. v1's `DUPLICATE_CONFLICT` degrade
/// ([`super::decline::MergeDecline::Conflict`]) has no case left here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MergeDecision {
    /// The two summaries agree: keep either body and union the diagnostics.
    Merge,
    /// Only one side carries a body: keep that side's. Not a decline — an item encoded in one
    /// codegen unit and declined in another is ordinary.
    Available(Side),
    /// Both sides carry a body and the normal forms differ. Keep `keep`'s body, whose hash is
    /// `chosen`, and drop the other, whose hash is `rejected`; record `decline` against the item.
    /// See [`choose_body`]: the choice depends only on the two summaries, so the outcome does not
    /// depend on the order objects were read in.
    Choose {
        keep: Side,
        chosen: Hash128,
        rejected: Hash128,
        decline: Decline,
    },
    /// The two summaries cannot be one item. A hard link error, as in v1.
    Error(MergeError),
}

/// Why two summaries cannot be merged at all.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MergeError {
    /// Two different Rust items were emitted under one JavaScript name: v1's "two different
    /// definitions of `name` reached the same program". The likeliest cause is objects compiled
    /// with different emit-affecting options, which [`merge_headers`] catches first.
    NameCollision {
        link_name: String,
        left: Option<ItemId>,
        right: Option<ItemId>,
    },
    /// The caller grouped two summaries that do not even share a name.
    DifferentNames { left: String, right: String },
    /// The objects were not written by the same compiler, target, or options.
    Header(Decline),
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NameCollision { link_name, .. } => write!(
                f,
                "two different definitions of `{link_name}` reached the same program"
            ),
            Self::DifferentNames { left, right } => write!(
                f,
                "`{left}` and `{right}` are not the same item and cannot be merged"
            ),
            Self::Header(decline) => decline.fmt(f),
        }
    }
}

/// What the linker does with two summaries for one JavaScript name.
///
/// The headers are the caller's business: check them once per pair of objects with
/// [`merge_headers`], not once per item.
pub fn merge_duplicates(a: &ItemSummary, b: &ItemSummary) -> MergeDecision {
    if a.key.link_name != b.key.link_name {
        return MergeDecision::Error(MergeError::DifferentNames {
            left: a.key.link_name.clone(),
            right: b.key.link_name.clone(),
        });
    }
    // Same JavaScript name, different Rust items: v1's real conflict, and the only one left.
    if a.key.item != b.key.item {
        return MergeDecision::Error(MergeError::NameCollision {
            link_name: a.key.link_name.clone(),
            left: a.key.item,
            right: b.key.item,
        });
    }
    choose_body(a, b)
}

/// What the linker does with two items the **text** dedup pass proved interchangeable.
///
/// This is the other duplicate relation, and the one the FingerprintJS numbers are actually about.
/// `link_dedup` merges items whose printed JavaScript is byte-identical under *different* names —
/// two monomorphizations of one generic that erase to the same code — and points every reference
/// at one winner. Those items are not the same [`SemanticKey`]: their type arguments differ, so
/// their bodies differ, legitimately. The proof that the merge is sound is the printed text, which
/// the caller has already established; all that is left is to choose one body for the surviving
/// item.
///
/// So this is [`merge_duplicates`] without the identity check: never an error, never a loss of
/// availability, and the deterministic [`choose_body`] pick recorded as
/// [`MergeDecline::BodyChoice`] when the bodies differ. v1 instead declined the winner's late-IR
/// availability outright, which is where all 74 of its `DUPLICATE_CONFLICT`s came from.
pub fn merge_aliases(a: &ItemSummary, b: &ItemSummary) -> MergeDecision {
    choose_body(a, b)
}

/// Which of two bodies for one item survives: the shared tail of [`merge_duplicates`] and
/// [`merge_aliases`].
///
/// A copy with cleanup edges ([`ItemClass::UNWIND`]) beats a copy without them. Two duplicates
/// were lowered from one Rust body, so the copy without cleanup edges came from MIR that had
/// already lost them (rustc's optimized MIR under `-Cpanic=abort`), and keeping it would silently
/// skip the destructors a host exception owes. (Aliases are already proved interchangeable by
/// their printed text; the same rule only keeps the body that describes more.) Otherwise the
/// smaller normalized hash wins. The decision reads only the two summaries, so it does not depend
/// on the order objects were read in, and folding it over several copies keeps the same one
/// whatever the order.
fn choose_body(a: &ItemSummary, b: &ItemSummary) -> MergeDecision {
    match (a.semantic_hash, b.semantic_hash) {
        (None, None) => MergeDecision::Merge,
        (Some(_), None) => MergeDecision::Available(Side::Left),
        (None, Some(_)) => MergeDecision::Available(Side::Right),
        (Some(left), Some(right)) if left == right => MergeDecision::Merge,
        (Some(left), Some(right)) => {
            let cleanup = |summary: &ItemSummary| summary.class.contains(ItemClass::UNWIND);
            let keep_left = match (cleanup(a), cleanup(b)) {
                (true, false) => true,
                (false, true) => false,
                _ => left < right,
            };
            let (keep, chosen, rejected) = if keep_left {
                (Side::Left, left, right)
            } else {
                (Side::Right, right, left)
            };
            MergeDecision::Choose {
                keep,
                chosen,
                rejected,
                decline: Decline::Merge(MergeDecline::BodyChoice {
                    chosen: hash_key(chosen),
                    rejected: hash_key(rejected),
                }),
            }
        }
    }
}

/// The hash as the one integer its `Display` spells, for [`MergeDecline::BodyChoice`].
fn hash_key(hash: Hash128) -> u128 {
    (u128::from(hash.hi) << 64) | u128::from(hash.lo)
}

/// Whether two objects may contribute items to one program.
///
/// The JIR schema, the producing rustc, the target data layout and the emit-affecting options all
/// have to agree: type ids, definition ids and printed text are only comparable within one of
/// each. A mismatch is a hard error naming the rebuild that fixes it, never a silent degrade.
pub fn merge_headers(a: &crate::object::Header, b: &crate::object::Header) -> Result<(), Decline> {
    let mismatch = |field: &str, left: String, right: String| {
        Err(Decline::Merge(MergeDecline::Header {
            field: field.to_owned(),
            left,
            right,
        }))
    };
    if a.jir_schema != b.jir_schema {
        return mismatch(
            "jir schema",
            a.jir_schema.to_string(),
            b.jir_schema.to_string(),
        );
    }
    if a.rustc_version_hash != b.rustc_version_hash {
        return mismatch(
            "rustc version",
            format!("{:016x}", a.rustc_version_hash),
            format!("{:016x}", b.rustc_version_hash),
        );
    }
    if a.data_layout != b.data_layout {
        return mismatch("data layout", a.data_layout.clone(), b.data_layout.clone());
    }
    if a.emit_args != b.emit_args {
        return mismatch("emit options", a.emit_args.clone(), b.emit_args.clone());
    }
    Ok(())
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use crate::jir::codec;
    use crate::jir::semantic::{BinaryOp, BodySemantic, DefinitionId};

    const I32: super::super::types::TypeId = super::super::types::TypeId(3);

    fn item(n: u64) -> ItemId {
        ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: n,
            },
            instantiation: u128::from(n),
        }
    }

    fn place(local: u32) -> Place {
        Place {
            local: LocalId(local),
            projection: Vec::new(),
            ty: I32,
        }
    }

    fn copy(local: u32) -> Operand {
        Operand {
            ty: I32,
            kind: OperandKind::Copy(place(local)),
        }
    }

    fn scalar(bits: u128) -> Operand {
        Operand {
            ty: I32,
            kind: OperandKind::Constant(Constant::Scalar { bits, bytes: 4 }),
        }
    }

    fn assign(destination: u32, left: u32, right: u128) -> Statement {
        Statement {
            kind: StatementKind::Assign {
                destination: place(destination),
                value: Rvalue::Binary {
                    op: BinaryOp::Add,
                    left: copy(left),
                    right: scalar(right),
                },
            },
        }
    }

    fn locals() -> Vec<Local> {
        vec![
            Local {
                id: LocalId(0),
                role: LocalRole::Return,
                ty: I32,
            },
            Local {
                id: LocalId(1),
                role: LocalRole::Argument,
                ty: I32,
            },
            Local {
                id: LocalId(2),
                role: LocalRole::Temporary,
                ty: I32,
            },
        ]
    }

    fn body(blocks: Vec<BasicBlock>) -> Body {
        Body {
            item: item(1),
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: vec![I32],
                result: I32,
                abi: AbiSignature {
                    parameters: 1,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                },
            },
            locals: locals(),
            blocks,
        }
    }

    /// `fn f(a) { t = a + 1; goto exit; exit: ret = t + 0; return }`, blocks numbered 0 and 1.
    fn plain() -> Body {
        body(vec![
            BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: vec![assign(2, 1, 1)],
                terminator: Terminator::Goto(BlockId(1)),
            },
            BasicBlock {
                id: BlockId(1),
                cleanup: false,
                statements: vec![assign(0, 2, 0)],
                terminator: Terminator::Return,
            },
        ])
    }

    /// The same function: blocks numbered 7 and 3, storage markers and nops between the
    /// statements, the goto spelled as a false edge, one unreachable block left behind, and one
    /// local (5) that only the storage markers mention.
    fn noisy() -> Body {
        let mut noisy = body(vec![
            BasicBlock {
                id: BlockId(7),
                cleanup: false,
                statements: vec![
                    Statement {
                        kind: StatementKind::StorageLive(LocalId(2)),
                    },
                    Statement {
                        kind: StatementKind::StorageLive(LocalId(5)),
                    },
                    Statement {
                        kind: StatementKind::Nop,
                    },
                    assign(2, 1, 1),
                    Statement {
                        kind: StatementKind::StorageDead(LocalId(5)),
                    },
                ],
                terminator: Terminator::FalseEdge {
                    real: BlockId(3),
                    imaginary: BlockId(9),
                },
            },
            BasicBlock {
                id: BlockId(9),
                cleanup: false,
                statements: vec![assign(0, 1, 77)],
                terminator: Terminator::Unreachable,
            },
            BasicBlock {
                id: BlockId(3),
                cleanup: false,
                statements: vec![
                    Statement {
                        kind: StatementKind::Nop,
                    },
                    assign(0, 2, 0),
                    Statement {
                        kind: StatementKind::StorageDead(LocalId(2)),
                    },
                ],
                terminator: Terminator::Return,
            },
        ]);
        noisy.locals.push(Local {
            id: LocalId(5),
            role: LocalRole::Temporary,
            ty: I32,
        });
        noisy
    }

    #[test]
    fn storage_markers_nops_false_edges_dead_blocks_and_numbering_do_not_change_the_hash() {
        let interns = Interns::new();
        let plain = plain();
        let noisy = noisy();
        assert_ne!(
            plain, noisy,
            "the two bodies must differ, or the test proves nothing"
        );
        assert_ne!(
            Hash128::of(&codec::encode_body(&plain)),
            Hash128::of(&codec::encode_body(&noisy)),
            "the blob hash is a hash of bytes and must see the difference"
        );
        assert_eq!(
            semantic_hash(&plain, &interns),
            semantic_hash(&noisy, &interns),
            "the normalized hash must not"
        );
    }

    #[test]
    fn physical_summaries_are_canonical_over_the_normalized_body() {
        let interns = Interns::new();
        let plain = plain();
        let noisy = noisy();
        let hash = semantic_hash(&plain, &interns);
        assert_eq!(hash, semantic_hash(&noisy, &interns));
        assert_eq!(
            PhysicalSummary::of(&plain, &interns.types, hash),
            PhysicalSummary::of(&noisy, &interns.types, hash)
        );
    }

    #[test]
    fn stale_and_overclaiming_physical_summaries_are_rejected() {
        let interns = Interns::new();
        let body = plain();
        let hash = semantic_hash(&body, &interns);
        let summary = PhysicalSummary::of(&body, &interns.types, hash).expect("a function");
        assert_eq!(summary.validate(&body, &interns.types, hash), Ok(()));

        let mut stale = summary.clone();
        stale.format += 1;
        assert_eq!(
            stale.validate(&body, &interns.types, hash),
            Err(PhysicalSummaryValidation::Format)
        );

        let mut overclaim = summary.clone();
        overclaim.parameters[0].field_reads += 1;
        assert_eq!(
            overclaim.validate(&body, &interns.types, hash),
            Err(PhysicalSummaryValidation::Facts)
        );

        assert_eq!(
            summary.validate(&body, &interns.types, Hash128::of(b"another body")),
            Err(PhysicalSummaryValidation::Body)
        );
    }

    #[test]
    fn the_normal_form_drops_what_it_says_it_drops() {
        let normalized = NormalizedBody::of(&noisy(), &Interns::new());
        assert_eq!(normalized.body.blocks.len(), 2, "the dead block is gone");
        assert_eq!(
            normalized
                .body
                .blocks
                .iter()
                .map(|block| block.id)
                .collect::<Vec<_>>(),
            vec![BlockId(0), BlockId(1)],
            "blocks are renumbered from the entry"
        );
        assert!(
            normalized
                .body
                .blocks
                .iter()
                .flat_map(|block| &block.statements)
                .all(|statement| matches!(statement.kind, StatementKind::Assign { .. })),
            "storage markers and nops are gone"
        );
        assert!(
            matches!(
                normalized.body.blocks[0].terminator,
                Terminator::Goto(BlockId(1))
            ),
            "the false edge became the edge it really is"
        );
        assert_eq!(
            normalized.body.locals.len(),
            3,
            "the local only the storage markers mentioned is gone"
        );
    }

    #[test]
    fn a_changed_operand_changes_the_hash() {
        let interns = Interns::new();
        let mut changed = plain();
        changed.blocks[0].statements[0] = assign(2, 1, 2);
        assert_ne!(
            semantic_hash(&plain(), &interns),
            semantic_hash(&changed, &interns)
        );
    }

    #[test]
    fn a_changed_terminator_changes_the_hash() {
        let interns = Interns::new();
        let mut changed = plain();
        changed.blocks[1].terminator = Terminator::Unreachable;
        assert_ne!(
            semantic_hash(&plain(), &interns),
            semantic_hash(&changed, &interns)
        );
    }

    /// Two codegen units intern their strings in whatever order they meet them; the hash must not
    /// see the ids, only the text.
    #[test]
    fn interned_strings_hash_by_content_not_by_id() {
        let call = |name: StrId| BasicBlock {
            id: BlockId(0),
            cleanup: false,
            statements: Vec::new(),
            terminator: Terminator::Call(Call {
                function: scalar(0),
                callee: Callee::Direct {
                    item: item(2),
                    link_name: name,
                },
                semantic: CallSemantic::Ordinary,
                arguments: vec![copy(1)],
                destination: place(0),
                target: None,
                unwind: Unwind::Continue,
                track_caller: None,
                host: None,
            }),
        };

        let mut left = Interns::new();
        let left_name = left.intern_str("callee$h1");
        let mut right = Interns::new();
        // A different id for the same text, and the same id for different text.
        right.intern_str("something$else");
        right.intern_str("and$another");
        let right_name = right.intern_str("callee$h1");
        assert_ne!(left_name, right_name);

        assert_eq!(
            semantic_hash(&body(vec![call(left_name)]), &left),
            semantic_hash(&body(vec![call(right_name)]), &right)
        );
        assert_ne!(
            semantic_hash(&body(vec![call(left_name)]), &left),
            semantic_hash(&body(vec![call(StrId(0))]), &right),
            "different text must still differ"
        );
    }

    // -- the identity rule --

    fn summary(name: &str, item: Option<ItemId>, hash: Option<Hash128>) -> ItemSummary {
        ItemSummary {
            key: SemanticKey {
                link_name: name.to_owned(),
                item,
            },
            body_hash: hash,
            semantic_hash: hash,
            availability: Availability::Available,
            deps: Vec::new(),
            abi: None,
            physical: None,
            kind: ItemKind::Fn,
            linkage: Linkage::internal(),
            order: None,
            dts: false,
            fallback: BlobRef::default(),
            body: None,
            origin: None,
            estimated_bytes: 0,
            class: ItemClass::empty(),
        }
    }

    #[test]
    fn equal_normalized_hashes_merge() {
        let hash = Hash128::of(b"same");
        let left = summary("f", Some(item(1)), Some(hash));
        let right = summary("f", Some(item(1)), Some(hash));
        assert_eq!(merge_duplicates(&left, &right), MergeDecision::Merge);
    }

    #[test]
    fn different_normalized_hashes_pick_the_smaller_one_either_way_round() {
        let low = Hash128 { lo: 1, hi: 0 };
        let high = Hash128 { lo: 2, hi: 0 };
        let left = summary("f", Some(item(1)), Some(low));
        let right = summary("f", Some(item(1)), Some(high));
        let forward = merge_duplicates(&left, &right);
        let backward = merge_duplicates(&right, &left);
        match (&forward, &backward) {
            (
                MergeDecision::Choose {
                    keep: Side::Left,
                    chosen,
                    rejected,
                    decline,
                },
                MergeDecision::Choose {
                    keep: Side::Right,
                    chosen: back_chosen,
                    rejected: back_rejected,
                    decline: back_decline,
                },
            ) => {
                assert_eq!(*chosen, low);
                assert_eq!(*rejected, high);
                assert_eq!(chosen, back_chosen);
                assert_eq!(rejected, back_rejected);
                assert_eq!(decline.code(), "merge.body-choice");
                assert_eq!(decline, back_decline);
            }
            other => panic!("expected a deterministic choice, got {other:?}"),
        }
    }

    /// A copy with cleanup edges is kept over a copy without them whatever their hashes, for
    /// duplicates and aliases alike, and either way round. Otherwise the smaller hash wins.
    #[test]
    fn a_copy_with_cleanup_edges_beats_a_smaller_hash_without_them() {
        let low = Hash128 { lo: 1, hi: 0 };
        let high = Hash128 { lo: 2, hi: 0 };
        let copy = |hash, cleanup| {
            let mut copy = summary("f", Some(item(1)), Some(hash));
            if cleanup {
                copy.class.insert(ItemClass::UNWIND);
            }
            copy
        };
        for merge in [merge_duplicates, merge_aliases] {
            for (low_cleanup, high_cleanup, expected) in [
                (false, true, high),
                (true, false, low),
                (true, true, low),
                (false, false, low),
            ] {
                let (left, right) = (copy(low, low_cleanup), copy(high, high_cleanup));
                for (a, b, keep) in [
                    (
                        &left,
                        &right,
                        if expected == low {
                            Side::Left
                        } else {
                            Side::Right
                        },
                    ),
                    (
                        &right,
                        &left,
                        if expected == low {
                            Side::Right
                        } else {
                            Side::Left
                        },
                    ),
                ] {
                    match merge(a, b) {
                        MergeDecision::Choose {
                            keep: kept, chosen, ..
                        } => {
                            assert_eq!(kept, keep);
                            assert_eq!(chosen, expected);
                        }
                        other => panic!("expected a choice, got {other:?}"),
                    }
                }
            }
        }
    }

    #[test]
    fn a_body_on_one_side_only_keeps_that_side_without_a_decline() {
        let hash = Hash128::of(b"body");
        let left = summary("f", Some(item(1)), Some(hash));
        let right = summary("f", Some(item(1)), None);
        assert_eq!(
            merge_duplicates(&left, &right),
            MergeDecision::Available(Side::Left)
        );
        assert_eq!(
            merge_duplicates(&right, &left),
            MergeDecision::Available(Side::Right)
        );
        let neither = summary("f", Some(item(1)), None);
        assert_eq!(merge_duplicates(&right, &neither), MergeDecision::Merge);
    }

    #[test]
    fn one_name_over_two_items_is_an_error_not_a_degrade() {
        let left = summary("f", Some(item(1)), Some(Hash128::of(b"a")));
        let right = summary("f", Some(item(2)), Some(Hash128::of(b"a")));
        match merge_duplicates(&left, &right) {
            MergeDecision::Error(MergeError::NameCollision { link_name, .. }) => {
                assert_eq!(link_name, "f")
            }
            other => panic!("expected a name collision, got {other:?}"),
        }
        let unrelated = summary("g", Some(item(1)), None);
        assert!(matches!(
            merge_duplicates(&left, &unrelated),
            MergeDecision::Error(MergeError::DifferentNames { .. })
        ));
    }

    /// The writer fills `semantic_hash`, so two objects written from bodies that differ only in
    /// what normalization drops deduplicate straight out of their summary indexes, without either
    /// body being inflated.
    #[test]
    fn two_objects_written_from_noisy_and_plain_bodies_merge_from_their_indexes_alone() {
        use crate::jir::origin::OriginTable;
        use crate::object::{self, ItemInput, Linkage as ObjectLinkage};

        let origins = |body: &Body| OriginTable {
            origins: Vec::new(),
            statements: vec![crate::jir::origin::OriginId(0); body.statement_count()],
            terminators: vec![crate::jir::origin::OriginId(0); body.blocks.len()],
            locals: vec![crate::jir::origin::OriginId(0); body.locals.len()],
            item_path: StrId(0),
            root: None,
        };
        let write = |body: &Body, crate_name: &str| {
            let mut interns = Interns::new();
            interns.intern_str("path::to::f");
            let mut writer = object::Writer::new(object::test_header(crate_name), "", interns);
            writer.push_item(
                "function f(a){return a}",
                ItemInput {
                    key: SemanticKey {
                        link_name: "f".to_owned(),
                        item: Some(item(1)),
                    },
                    availability: Availability::Available,
                    deps: Vec::new(),
                    kind: ItemKind::Fn,
                    linkage: ObjectLinkage::internal(),
                    order: None,
                    dts: false,
                    body: Some((body, &origins(body))),
                },
            );
            writer.finish(&[], &[], &std::collections::BTreeMap::new())
        };

        let left = write(&plain(), "left");
        let right = write(&noisy(), "right");
        let left = object::read(&left).expect("read").expect("an object");
        let right = object::read(&right).expect("read").expect("an object");
        assert_eq!(merge_headers(&left.header, &right.header), Ok(()));
        assert_ne!(
            left.summaries[0].body_hash, right.summaries[0].body_hash,
            "the blobs differ"
        );
        assert_eq!(
            left.summaries[0].physical, right.summaries[0].physical,
            "normalization noise cannot change producer facts"
        );
        assert_eq!(
            merge_duplicates(&left.summaries[0], &right.summaries[0]),
            MergeDecision::Merge,
            "and the items are the same item"
        );
    }

    #[test]
    fn headers_have_to_agree_field_by_field() {
        let base = crate::object::test_header("a");
        assert_eq!(merge_headers(&base, &base), Ok(()));
        for change in [
            (|header: &mut crate::object::Header| header.jir_schema += 1) as fn(&mut _),
            |header: &mut crate::object::Header| header.rustc_version_hash ^= 1,
            |header: &mut crate::object::Header| header.data_layout.push('x'),
            |header: &mut crate::object::Header| header.emit_args.push_str("js-minify=on"),
        ] {
            let mut other = base.clone();
            change(&mut other);
            let decline = merge_headers(&base, &other).expect_err("must not agree");
            assert_eq!(decline.code(), "merge.header");
        }
        // The crate name is not one of them: two crates contribute items to one program.
        let mut other = base.clone();
        other.crate_name = "b".to_owned();
        assert_eq!(merge_headers(&base, &other), Ok(()));
    }
}

#[cfg(test)]
mod alias_tests {
    use super::*;
    use crate::jir::semantic::DefinitionId;

    fn summary(name: &str, local_hash: u64, hash: Option<Hash128>) -> ItemSummary {
        ItemSummary {
            key: SemanticKey {
                link_name: name.to_owned(),
                item: Some(ItemId {
                    definition: DefinitionId {
                        crate_id: 1,
                        local_hash,
                    },
                    instantiation: u128::from(local_hash),
                }),
            },
            body_hash: hash,
            semantic_hash: hash,
            availability: Availability::Available,
            deps: Vec::new(),
            abi: None,
            physical: None,
            kind: ItemKind::Fn,
            linkage: Linkage::internal(),
            order: None,
            dts: false,
            fallback: BlobRef::default(),
            body: None,
            origin: None,
            estimated_bytes: 0,
            class: ItemClass::empty(),
        }
    }

    /// The 74-conflict shape: two different items with the same printed text. v1 declined the
    /// winner; the rule here picks a body and stays available.
    #[test]
    fn two_different_items_with_one_text_pick_a_body_instead_of_declining() {
        let winner = summary("a$h1", 1, Some(Hash128 { lo: 9, hi: 0 }));
        let loser = summary("a$h2", 2, Some(Hash128 { lo: 4, hi: 0 }));
        match merge_aliases(&winner, &loser) {
            MergeDecision::Choose {
                keep: Side::Right,
                chosen,
                rejected,
                decline,
            } => {
                assert_eq!(chosen, Hash128 { lo: 4, hi: 0 });
                assert_eq!(rejected, Hash128 { lo: 9, hi: 0 });
                assert_eq!(decline.code(), "merge.body-choice");
            }
            other => panic!("expected a body choice, got {other:?}"),
        }
        // The same pair merged as duplicates would be a hard error: they are not one item.
        assert!(matches!(
            merge_duplicates(&winner, &loser),
            MergeDecision::Error(MergeError::DifferentNames { .. })
        ));
    }

    #[test]
    fn identical_bodies_behind_two_names_just_merge() {
        let hash = Some(Hash128::of(b"one body"));
        assert_eq!(
            merge_aliases(&summary("a$h1", 1, hash), &summary("a$h2", 2, hash)),
            MergeDecision::Merge
        );
    }
}
