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

mod merge;
mod normalize;
mod physical;

#[cfg(test)]
mod tests;

pub use merge::{merge_aliases, merge_duplicates, merge_headers, MergeDecision, MergeError, Side};
pub use normalize::{reverse_postorder, semantic_hash, successors, NormalizedBody};
pub use physical::{
    PhysicalArgumentFacts, PhysicalBoundaryFacts, PhysicalCallFacts, PhysicalSummary,
    PhysicalSummaryValidation, PhysicalValueFacts, PHYSICAL_SUMMARY_FORMAT,
};

use serde::{Deserialize, Serialize};

use crate::jir::decline::Decline;
use crate::jir::hash::Hash128;
use crate::jir::semantic::{
    AbiSignature, AggregateKind, AssertMessage, Body, BodyKind, CallSemantic, Callee, CastKind,
    ItemId, Operand, OperandKind, Rvalue, StatementKind, Terminator, Unwind,
};
use crate::jir::types::{TypeKind, TypeTable};
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
                            CallSemantic::Host | CallSemantic::RawJavaScript
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
