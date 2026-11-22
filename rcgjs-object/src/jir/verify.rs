//! Structural verification of a body against its tables: v1's `LateBody::validate` family, run
//! after encoding and again after every decode so nothing downstream indexes out of bounds.
//!
//! What is checked: every `TypeId` resolves in the [`TypeTable`]; every `StrId`/`BytesId`/
//! `SigId`/`SourceFactId` resolves in the [`Interns`]; locals and blocks are stored at their own
//! index; local 0 of a function is the return place and the argument locals match the signature;
//! statics and vtables carry no CFG; block, local, and type references inside statements and
//! terminators are in range; `Callee::Virtual` dispatches on a `Dynamic` receiver; and (when an
//! origin table is given) its lengths match the body. Enum-tag consistency and sorted unique type
//! ids are checked by [`TypeTable::verify`].

use std::fmt;

use super::interns::{BytesId, Interns, StrId};
use super::origin::{OriginTable, SourceFactId};
use super::semantic::{
    AggregateKind, AssertMessage, BasicBlock, BlockId, Body, BodyKind, BodySemantic, Call, Callee,
    ConstTree, Constant, DirectItemRef, DropGlue, GlobalReference, HostPayload, KeyedListCallable,
    LocalId, LocalRole, Operand, OperandKind, Place, Projection, Rvalue, StatementKind, Terminator,
    TypeId, Unwind, VtableEntry,
};
use super::types::{TypeKind, TypeTable};

/// A verification failure. The message is the v1 validator text where one existed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifyError {
    pub message: String,
}

impl VerifyError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub(crate) fn missing_type(ty: TypeId, context: &str) -> Self {
        Self::new(format!(
            "late-IR {context} references missing type {:032x}",
            ty.0
        ))
    }
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for VerifyError {}

struct Context<'a> {
    types: &'a TypeTable,
    interns: &'a Interns,
    locals: usize,
    blocks: usize,
}

impl Context<'_> {
    fn ty(&self, ty: TypeId) -> Result<(), VerifyError> {
        self.types.require(ty, "value").map(|_| ())
    }

    fn str(&self, id: StrId) -> Result<(), VerifyError> {
        if self.interns.str(id).is_some() {
            Ok(())
        } else {
            Err(VerifyError::new(format!(
                "late-IR references missing string {}",
                id.0
            )))
        }
    }

    fn bytes(&self, id: BytesId) -> Result<(), VerifyError> {
        if self.interns.bytes(id).is_some() {
            Ok(())
        } else {
            Err(VerifyError::new(format!(
                "late-IR references missing byte blob {}",
                id.0
            )))
        }
    }

    fn source_fact(&self, id: SourceFactId) -> Result<(), VerifyError> {
        if self.interns.source_fact(id).is_some() {
            Ok(())
        } else {
            Err(VerifyError::new(format!(
                "late-IR references missing source fact {}",
                id.0
            )))
        }
    }

    fn local(&self, local: LocalId) -> Result<(), VerifyError> {
        if (local.0 as usize) < self.locals {
            Ok(())
        } else {
            Err(VerifyError::new(format!(
                "late-IR references missing local {}",
                local.0
            )))
        }
    }

    fn block(&self, block: BlockId) -> Result<(), VerifyError> {
        if (block.0 as usize) < self.blocks {
            Ok(())
        } else {
            Err(VerifyError::new(format!(
                "late-IR references missing block {}",
                block.0
            )))
        }
    }

    fn item_ref(&self, item: &DirectItemRef) -> Result<(), VerifyError> {
        self.str(item.link_name)?;
        if self
            .interns
            .str(item.link_name)
            .is_some_and(|name| name.is_empty())
        {
            return Err(VerifyError::new(
                "late-IR direct item reference has an empty link name",
            ));
        }
        Ok(())
    }
}

impl Body {
    /// Verifies the body against its tables. See the module documentation for what is checked.
    pub fn verify(&self, types: &TypeTable, interns: &Interns) -> Result<(), VerifyError> {
        self.verify_inner(types, interns, false)
    }

    /// Verifies a semantic header: a `BodySemantic::Standard` body whose CFG was not encoded
    /// (locals and signature only, blocks empty).
    pub fn verify_semantic_header(
        &self,
        types: &TypeTable,
        interns: &Interns,
    ) -> Result<(), VerifyError> {
        if !matches!(self.semantic, BodySemantic::Standard(_)) {
            return Err(VerifyError::new(
                "semantic late-IR header is not a canonical standard operation",
            ));
        }
        self.verify_inner(types, interns, true)
    }

    /// Checks that an origin table's positional vectors match this body.
    pub fn verify_origins(&self, origins: &OriginTable) -> Result<(), VerifyError> {
        if origins.statements.len() != self.statement_count() {
            return Err(VerifyError::new(format!(
                "late-IR origin table has {} statement origins for {} statements",
                origins.statements.len(),
                self.statement_count()
            )));
        }
        if origins.terminators.len() != self.blocks.len() {
            return Err(VerifyError::new(format!(
                "late-IR origin table has {} terminator origins for {} blocks",
                origins.terminators.len(),
                self.blocks.len()
            )));
        }
        if origins.locals.len() != self.locals.len() {
            return Err(VerifyError::new(format!(
                "late-IR origin table has {} local origins for {} locals",
                origins.locals.len(),
                self.locals.len()
            )));
        }
        let count = origins.origins.len();
        let in_range =
            |ids: &[super::origin::OriginId]| ids.iter().all(|id| (id.0 as usize) < count);
        if !in_range(&origins.statements)
            || !in_range(&origins.terminators)
            || !in_range(&origins.locals)
            || !origins
                .origins
                .iter()
                .all(|origin| in_range(&origin.inline_stack))
        {
            return Err(VerifyError::new(
                "late-IR origin table references a missing origin",
            ));
        }
        Ok(())
    }

    fn verify_inner(
        &self,
        types: &TypeTable,
        interns: &Interns,
        semantic_header: bool,
    ) -> Result<(), VerifyError> {
        types.require(self.signature.result, "function result")?;
        for argument in &self.signature.arguments {
            types.require(*argument, "function argument")?;
        }
        for (index, local) in self.locals.iter().enumerate() {
            if local.id.0 as usize != index {
                return Err(VerifyError::new(format!(
                    "late-IR local {} is stored at index {index}",
                    local.id.0
                )));
            }
            types.require(local.ty, "local")?;
        }
        if let Some(spread) = self.signature.abi.spread {
            if (spread.local.0 as usize) >= self.locals.len() {
                return Err(VerifyError::new(
                    "late-IR spread argument names a missing local",
                ));
            }
        }
        if let BodySemantic::DropGlue(ty) = self.semantic {
            types.require(ty, "drop glue")?;
        }

        let context = Context {
            types,
            interns,
            locals: self.locals.len(),
            blocks: self.blocks.len(),
        };
        match &self.kind {
            BodyKind::Function => {
                if self.locals.first().map(|local| local.role) != Some(LocalRole::Return) {
                    return Err(VerifyError::new(
                        "late-IR function local 0 is not the return place",
                    ));
                }
                if self.blocks.is_empty() && !semantic_header {
                    return Err(VerifyError::new("late-IR function has no entry block"));
                }
                let arguments: Vec<_> = self
                    .locals
                    .iter()
                    .filter(|local| local.role == LocalRole::Argument)
                    .map(|local| local.ty)
                    .collect();
                if arguments != self.signature.arguments {
                    return Err(VerifyError::new(
                        "late-IR signature does not match argument locals",
                    ));
                }
                if self.locals[0].ty != self.signature.result {
                    return Err(VerifyError::new(
                        "late-IR signature does not match the return local",
                    ));
                }
            }
            BodyKind::Static { init, .. } => {
                if !self.locals.is_empty() || !self.blocks.is_empty() {
                    return Err(VerifyError::new("late-IR static contains a function CFG"));
                }
                verify_const_tree(init, &context)?;
            }
            BodyKind::Vtable {
                concrete, entries, ..
            } => {
                if !self.locals.is_empty() || !self.blocks.is_empty() {
                    return Err(VerifyError::new("late-IR vtable contains a function CFG"));
                }
                types.require(*concrete, "vtable concrete type")?;
                for entry in entries {
                    match entry {
                        VtableEntry::Drop(Some(item)) | VtableEntry::Method(item) => {
                            context.item_ref(item)?;
                        }
                        VtableEntry::TraitVPtr(name) => context.str(*name)?,
                        VtableEntry::Drop(None)
                        | VtableEntry::Size(_)
                        | VtableEntry::Align(_)
                        | VtableEntry::Vacant => {}
                    }
                }
            }
        }

        for (index, block) in self.blocks.iter().enumerate() {
            if block.id.0 as usize != index {
                return Err(VerifyError::new(format!(
                    "late-IR block {} is stored at index {index}",
                    block.id.0
                )));
            }
            verify_block(block, &context)?;
        }
        Ok(())
    }
}

fn verify_block(block: &BasicBlock, context: &Context<'_>) -> Result<(), VerifyError> {
    for statement in &block.statements {
        verify_statement(&statement.kind, context)?;
    }
    verify_terminator(&block.terminator, context)
}

fn verify_statement(statement: &StatementKind, context: &Context<'_>) -> Result<(), VerifyError> {
    match statement {
        StatementKind::Assign { destination, value } => {
            verify_place(destination, context)?;
            verify_rvalue(value, context)
        }
        StatementKind::SetDiscriminant { place, .. } => verify_place(place, context),
        StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
            context.local(*local)
        }
        StatementKind::Assume(operand) => verify_operand(operand, context),
        StatementKind::CopyNonOverlapping {
            source,
            destination,
            count,
        } => {
            verify_operand(source, context)?;
            verify_operand(destination, context)?;
            verify_operand(count, context)
        }
        StatementKind::Nop => Ok(()),
    }
}

fn verify_rvalue(rvalue: &Rvalue, context: &Context<'_>) -> Result<(), VerifyError> {
    match rvalue {
        Rvalue::Use(value) | Rvalue::WrapUnsafeBinder(value) => verify_operand(value, context),
        Rvalue::Repeat { value, .. } | Rvalue::Unary { value, .. } => {
            verify_operand(value, context)
        }
        Rvalue::Reference { place, .. }
        | Rvalue::RawPointer { place, .. }
        | Rvalue::Discriminant(place)
        | Rvalue::CopyForDeref(place)
        | Rvalue::Reborrow { place, .. } => verify_place(place, context),
        Rvalue::Cast { value, result, .. } => {
            context.ty(*result)?;
            verify_operand(value, context)
        }
        Rvalue::Binary { left, right, .. } => {
            verify_operand(left, context)?;
            verify_operand(right, context)
        }
        Rvalue::Aggregate {
            kind,
            result,
            operands,
        } => {
            context.ty(*result)?;
            match kind {
                AggregateKind::Array(ty) | AggregateKind::RawPointer { pointee: ty, .. } => {
                    context.ty(*ty)?;
                }
                _ => {}
            }
            operands
                .iter()
                .try_for_each(|operand| verify_operand(operand, context))
        }
    }
}

fn verify_operand(operand: &Operand, context: &Context<'_>) -> Result<(), VerifyError> {
    context.ty(operand.ty)?;
    match &operand.kind {
        OperandKind::Copy(place) | OperandKind::Move(place) => verify_place(place, context),
        OperandKind::Constant(constant) => verify_constant(constant, context),
        OperandKind::RuntimeCheck(_) => Ok(()),
    }
}

fn verify_constant(constant: &Constant, context: &Context<'_>) -> Result<(), VerifyError> {
    match constant {
        Constant::String(text) => context.str(*text),
        Constant::Pointer { target, .. } => verify_global(target, context),
        Constant::Memory(tree) => verify_const_tree(tree, context),
        Constant::ZeroSized | Constant::Scalar { .. } => Ok(()),
    }
}

fn verify_global(target: &GlobalReference, context: &Context<'_>) -> Result<(), VerifyError> {
    match target {
        GlobalReference::Function { link_name, .. } => context.str(*link_name),
        GlobalReference::Static { link_name, .. } => context.str(*link_name),
        GlobalReference::Type(ty) => context.ty(*ty),
    }
}

fn verify_const_tree(tree: &ConstTree, context: &Context<'_>) -> Result<(), VerifyError> {
    match tree {
        ConstTree::Aggregate(fields)
        | ConstTree::Variant { fields, .. }
        | ConstTree::Slice(fields) => fields
            .iter()
            .try_for_each(|field| verify_const_tree(field, context)),
        ConstTree::Reference(value) => verify_const_tree(value, context),
        ConstTree::Dyn {
            data,
            concrete,
            vtable,
        } => {
            verify_const_tree(data, context)?;
            context.ty(*concrete)?;
            context.str(*vtable)
        }
        ConstTree::Global { target, .. } => verify_global(target, context),
        ConstTree::Str(text) => context.str(*text),
        ConstTree::Unit | ConstTree::Scalar { .. } | ConstTree::Address(_) => Ok(()),
    }
}

fn verify_place(place: &Place, context: &Context<'_>) -> Result<(), VerifyError> {
    context.local(place.local)?;
    context.ty(place.ty)?;
    for projection in &place.projection {
        match projection {
            Projection::Field { ty, .. }
            | Projection::OpaqueCast(ty)
            | Projection::UnwrapUnsafeBinder(ty) => context.ty(*ty)?,
            Projection::Index(local) => context.local(*local)?,
            Projection::Deref
            | Projection::ConstantIndex { .. }
            | Projection::Subslice { .. }
            | Projection::Downcast { .. } => {}
        }
    }
    Ok(())
}

fn verify_terminator(terminator: &Terminator, context: &Context<'_>) -> Result<(), VerifyError> {
    match terminator {
        Terminator::Goto(target) => context.block(*target),
        Terminator::Switch {
            discriminant,
            targets,
            otherwise,
        } => {
            verify_operand(discriminant, context)?;
            for (_, target) in targets {
                context.block(*target)?;
            }
            context.block(*otherwise)
        }
        Terminator::Return
        | Terminator::Unreachable
        | Terminator::UnwindResume
        | Terminator::UnwindTerminate(_) => Ok(()),
        Terminator::Call(call) => verify_call(call, context),
        Terminator::Drop {
            place,
            target,
            unwind,
            async_drop,
            glue,
            ..
        } => {
            verify_place(place, context)?;
            context.block(*target)?;
            verify_unwind(*unwind, context)?;
            if let Some(target) = async_drop {
                context.block(*target)?;
            }
            if let DropGlue::Direct { link_name, .. } = glue {
                context.str(*link_name)?;
            }
            Ok(())
        }
        Terminator::Assert {
            condition,
            message,
            target,
            unwind,
            ..
        } => {
            verify_operand(condition, context)?;
            verify_assert_message(message, context)?;
            context.block(*target)?;
            verify_unwind(*unwind, context)
        }
        Terminator::FalseEdge { real, imaginary } => {
            context.block(*real)?;
            context.block(*imaginary)
        }
        Terminator::FalseUnwind { real, unwind } => {
            context.block(*real)?;
            verify_unwind(*unwind, context)
        }
    }
}

fn verify_call(call: &Call, context: &Context<'_>) -> Result<(), VerifyError> {
    verify_operand(&call.function, context)?;
    match &call.callee {
        Callee::Direct { item, link_name } => context.item_ref(&DirectItemRef {
            item: *item,
            link_name: *link_name,
        })?,
        Callee::Foreign { symbol, .. } => context.str(*symbol)?,
        Callee::Indirect { signature } => context.ty(*signature)?,
        Callee::Intrinsic {
            name,
            types,
            fallback,
        } => {
            context.str(*name)?;
            types.iter().try_for_each(|ty| context.ty(*ty))?;
            if let Some(fallback) = fallback {
                context.item_ref(fallback)?;
            }
        }
        Callee::Virtual { receiver, .. } => {
            let descriptor = context.types.require(*receiver, "virtual call receiver")?;
            if !matches!(descriptor.kind, TypeKind::Dynamic { .. }) {
                return Err(VerifyError::new(
                    "late-IR virtual call receiver is not a trait object",
                ));
            }
        }
        Callee::Runtime { member, .. } => context.str(*member)?,
        Callee::Nop => {}
    }
    call.arguments
        .iter()
        .try_for_each(|argument| verify_operand(argument, context))?;
    verify_place(&call.destination, context)?;
    if let Some(target) = call.target {
        context.block(target)?;
    }
    verify_unwind(call.unwind, context)?;
    if let Some(location) = call.track_caller {
        context.source_fact(location)?;
    }
    match &call.host {
        Some(HostPayload::Extern(bytes)) | Some(HostPayload::Block(bytes)) => {
            context.bytes(*bytes)?;
        }
        Some(HostPayload::Marker {
            section,
            template,
            helpers,
            ..
        }) => {
            context.str(*section)?;
            if let Some(template) = template {
                verify_const_tree(template, context)?;
            }
            verify_helpers(helpers, context)?;
        }
        Some(HostPayload::ViewAbi {
            marker, helpers, ..
        }) => {
            context.str(*marker)?;
            verify_helpers(helpers, context)?;
        }
        Some(HostPayload::Map { marker }) => {
            context.str(*marker)?;
        }
        Some(HostPayload::KeyedList(payload)) => {
            if let Some(site) = payload.site {
                context.source_fact(site)?;
            }
            context
                .types
                .require(payload.collection_ty, "keyed-list collection")?;
            context.types.require(payload.item_ty, "keyed-list item")?;
            context.types.require(payload.key_ty, "keyed-list key")?;
            context.types.require(payload.row_ty, "keyed-list row")?;
            let callables = [&payload.collection, &payload.key, &payload.row];
            for callable in callables {
                verify_keyed_callable(callable, context)?;
            }
            if call.arguments.len() != callables.len() {
                return Err(VerifyError::new(format!(
                    "late-IR keyed-list call has {} arguments, expected {}",
                    call.arguments.len(),
                    callables.len()
                )));
            }
            for (argument, callable) in call.arguments.iter().zip(callables) {
                if argument.ty != callable.ty {
                    return Err(VerifyError::new(
                        "late-IR keyed-list callable type does not match its call argument",
                    ));
                }
            }
        }
        None => {}
    }
    Ok(())
}

fn verify_keyed_callable(
    callable: &KeyedListCallable,
    context: &Context<'_>,
) -> Result<(), VerifyError> {
    context.types.require(callable.ty, "keyed-list callable")?;
    if let Some(body) = &callable.body {
        context.item_ref(body)?;
    }
    match callable.cleanup {
        DropGlue::Direct { item, link_name } => {
            context.item_ref(&DirectItemRef { item, link_name })?
        }
        DropGlue::None | DropGlue::Dynamic => {}
    }
    if let Some(source) = callable.source {
        context.source_fact(source)?;
    }
    Ok(())
}

/// Every sibling instance a marker records: its name and its link name are interned strings.
fn verify_helpers(
    helpers: &[(
        crate::jir::interns::StrId,
        crate::jir::semantic::DirectItemRef,
    )],
    context: &Context<'_>,
) -> Result<(), VerifyError> {
    for (name, item) in helpers {
        context.str(*name)?;
        context.str(item.link_name)?;
    }
    Ok(())
}

fn verify_unwind(unwind: Unwind, context: &Context<'_>) -> Result<(), VerifyError> {
    match unwind {
        Unwind::Cleanup(block) => context.block(block),
        Unwind::Continue | Unwind::Unreachable | Unwind::Terminate(_) => Ok(()),
    }
}

fn verify_assert_message(
    message: &AssertMessage,
    context: &Context<'_>,
) -> Result<(), VerifyError> {
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
            verify_operand(length, context)?;
            verify_operand(index, context)
        }
        AssertMessage::OverflowNeg(value)
        | AssertMessage::DivisionByZero(value)
        | AssertMessage::RemainderByZero(value)
        | AssertMessage::InvalidEnum(value) => verify_operand(value, context),
        AssertMessage::ResumedAfterReturn(_)
        | AssertMessage::ResumedAfterPanic(_)
        | AssertMessage::ResumedAfterDrop(_)
        | AssertMessage::NullPointer
        | AssertMessage::NullReference => Ok(()),
    }
}
