//! Representation-neutral physical-call facts extracted from JIR-S bodies before decoding.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::normalize::reverse_postorder;
use crate::jir::hash::Hash128;
use crate::jir::semantic::{
    AssertMessage, BinaryOp, BlockId, Body, BodyKind, BodySemantic, Call, CallSemantic, Callee,
    CastKind, ItemId, Local, LocalId, LocalRole, Operand, OperandKind, Place, Rvalue, Statement,
    StatementKind, Terminator, TypeId,
};
use crate::jir::types::{TypeKind, TypeTable};

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
                    BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Cmp => PhysicalUse::Identity,
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
                CallSemantic::Host | CallSemantic::RawJavaScript
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
