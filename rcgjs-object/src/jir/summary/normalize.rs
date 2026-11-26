//! Canonical body normalization and normalized semantic hashing (W4).

use std::collections::{BTreeMap, BTreeSet};

use crate::jir::hash::{Hash128, Hasher128};
use crate::jir::interns::{BytesId, Interns, StrId};
use crate::jir::origin::{SourceFact, SourceFactId};
use crate::jir::semantic::{
    AbiSignature, AssertMessage, BasicBlock, BlockId, Body, BodyKind, Call, CallSemantic, Callee,
    ConstTree, Constant, DirectItemRef, DropGlue, GlobalReference, HostPayload, Local, LocalId,
    LocalRole, Operand, OperandKind, Place, Projection, Rvalue, Signature, SpreadArgument,
    Statement, StatementKind, StaticStep, Terminator, Unwind, VtableEntry,
};
use crate::jir::JIR_SCHEMA;

/// A body rewritten into the one canonical form two objects have to agree on.
///
/// [`super::ItemSummary::body_hash`] is the hash of the body's *postcard bytes*: it addresses the
/// blob in S3 and answers "are these the same bytes", which is what a content-addressed blob table
/// needs and what a dedup rule must not use. Two crates that codegen one monomorphic item run the
/// same MIR through the same encoder, but not necessarily through the same MIR pipeline: the
/// sysroot is built with `-Zinline-mir` and an application crate is not, and the passes that do
/// run leave different numbers of storage markers, nops, false edges, dead blocks and dead locals
/// behind. v1 hashed all of that and declined availability on any difference — the 74 duplicate
/// conflicts the FingerprintJS report shows.
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
/// 7. Types are [`crate::jir::semantic::TypeId`]s, which are already global stable hashes, and are
///    hashed as they are.
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
        let encoded = postcard::to_allocvec(&self.body)
            .expect("postcard serialization of normalized body is infallible");
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
    blocks: BTreeMap<BlockId, BlockId>,
    locals: BTreeMap<LocalId, LocalId>,
    next_local: u32,
    strings: Vec<String>,
    string_index: BTreeMap<StrId, StrId>,
    bytes: Vec<Vec<u8>>,
    bytes_index: BTreeMap<BytesId, BytesId>,
    source_facts: Vec<SourceFact>,
    source_fact_index: BTreeMap<SourceFactId, SourceFactId>,
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
        for (position, &id) in order.iter().enumerate() {
            self.blocks.insert(id, BlockId(position as u32));
        }
        // The return local and the arguments hold their positions: they are the JavaScript
        // parameter shape, not an internal numbering.
        for local in &body.locals {
            if matches!(local.role, LocalRole::Return | LocalRole::Argument) {
                self.local(local.id);
            }
        }

        let lookup: BTreeMap<BlockId, &BasicBlock> =
            body.blocks.iter().map(|block| (block.id, block)).collect();
        let mut blocks = Vec::with_capacity(order.len());
        for (position, id) in order.iter().enumerate() {
            let Some(block) = lookup.get(id) else {
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
                self.locals.get(&local.id).map(|id| Local {
                    id: *id,
                    role: local.role,
                    ty: local.ty,
                })
            })
            .collect();
        locals.sort_by_key(|local| local.id);

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
                inline_hint: body.signature.abi.inline_hint,
                inline_always: body.signature.abi.inline_always,
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
            .get(&id)
            .copied()
            .unwrap_or(BlockId(u32::MAX - id.0.min(u32::MAX - 1)))
    }

    fn local(&mut self, id: LocalId) -> LocalId {
        if let Some(mapped) = self.locals.get(&id) {
            return *mapped;
        }
        let mapped = LocalId(self.next_local);
        self.next_local += 1;
        self.locals.insert(id, mapped);
        mapped
    }

    fn string(&mut self, id: StrId) -> StrId {
        if let Some(mapped) = self.string_index.get(&id) {
            return *mapped;
        }
        let value = match self.interns.str(id) {
            Some(value) => value.to_owned(),
            None => format!("\u{0}unresolved-string:{}", id.0),
        };
        let mapped = StrId(self.strings.len() as u32);
        self.strings.push(value);
        self.string_index.insert(id, mapped);
        mapped
    }

    fn blob(&mut self, id: BytesId) -> BytesId {
        if let Some(mapped) = self.bytes_index.get(&id) {
            return *mapped;
        }
        let value = match self.interns.bytes(id) {
            Some(value) => value.to_vec(),
            None => format!("\u{0}unresolved-bytes:{}", id.0).into_bytes(),
        };
        let mapped = BytesId(self.bytes.len() as u32);
        self.bytes.push(value);
        self.bytes_index.insert(id, mapped);
        mapped
    }

    fn source_fact(&mut self, id: SourceFactId) -> SourceFactId {
        if let Some(mapped) = self.source_fact_index.get(&id) {
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
        self.source_fact_index.insert(id, mapped);
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
            ConstTree::StaticField { target, path } => ConstTree::StaticField {
                target: self.global(*target),
                path: path
                    .iter()
                    .map(|step| match step {
                        StaticStep::Field(name) => StaticStep::Field(self.string(*name)),
                        StaticStep::Index(index) => StaticStep::Index(*index),
                    })
                    .collect(),
            },
            ConstTree::StaticSlice {
                target,
                path,
                start,
                length,
            } => ConstTree::StaticSlice {
                target: self.global(*target),
                path: path
                    .iter()
                    .map(|step| match step {
                        StaticStep::Field(name) => StaticStep::Field(self.string(*name)),
                        StaticStep::Index(index) => StaticStep::Index(*index),
                    })
                    .collect(),
                start: *start,
                length: *length,
            },
            ConstTree::Unit => ConstTree::Unit,
            ConstTree::Scalar { bits, bytes } => ConstTree::Scalar {
                bits: *bits,
                bytes: *bytes,
            },
            ConstTree::Address(address) => ConstTree::Address(*address),
            ConstTree::SliceAddress { address, length } => ConstTree::SliceAddress {
                address: *address,
                length: *length,
            },
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
                track_caller,
            } => Terminator::Assert {
                condition: self.operand(condition),
                expected: *expected,
                message: self.assert_message(message),
                target: self.block(*target),
                unwind: self.unwind(*unwind),
                track_caller: track_caller.map(|fact| self.source_fact(fact)),
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

    fn helpers(&mut self, helpers: &[(StrId, DirectItemRef)]) -> Vec<(StrId, DirectItemRef)> {
        helpers
            .iter()
            .map(|(name, item)| {
                (
                    self.string(*name),
                    DirectItemRef {
                        item: item.item,
                        link_name: self.string(item.link_name),
                    },
                )
            })
            .collect()
    }

    fn host(&mut self, host: &HostPayload) -> HostPayload {
        match host {
            HostPayload::Extern(id) => HostPayload::Extern(self.blob(*id)),
            HostPayload::Block(id) => HostPayload::Block(self.blob(*id)),
            HostPayload::Marker {
                section,
                def,
                helpers,
            } => HostPayload::Marker {
                section: self.string(*section),
                def: *def,
                helpers: self.helpers(helpers),
            },
            HostPayload::Map { marker } => HostPayload::Map {
                marker: self.string(*marker),
            },
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
    let lookup: BTreeMap<BlockId, &BasicBlock> =
        body.blocks.iter().map(|block| (block.id, block)).collect();
    let Some(entry) = body.blocks.first().map(|block| block.id) else {
        return Vec::new();
    };
    let children_of = |id: BlockId| {
        lookup
            .get(&id)
            .map(|block| successors(&block.terminator))
            .unwrap_or_default()
    };
    let mut postorder = Vec::with_capacity(body.blocks.len());
    let mut seen: BTreeSet<BlockId> = BTreeSet::new();
    // (block, its successors, index of the next successor to visit); an explicit stack, because a
    // body can be deeper than the native one.
    let mut stack: Vec<(BlockId, Vec<BlockId>, usize)> = Vec::new();
    seen.insert(entry);
    stack.push((entry, children_of(entry), 0));
    while let Some((block, children, next)) = stack.last_mut() {
        if *next < children.len() {
            let child = children[*next];
            *next += 1;
            if seen.insert(child) {
                stack.push((child, children_of(child), 0));
            }
        } else {
            postorder.push(*block);
            stack.pop();
        }
    }
    postorder.reverse();
    postorder
}
