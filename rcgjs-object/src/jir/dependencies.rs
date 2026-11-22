//! Potential item references in a verified semantic body, independent of baseline text.

use std::collections::BTreeSet;

use super::interns::StrId;
use super::semantic::*;

impl Body {
    /// Named items a lowering of this body may need. Final emitted references decide retention.
    pub fn link_dependencies(&self) -> BTreeSet<StrId> {
        let mut scan = Dependencies::default();
        match &self.kind {
            BodyKind::Function => {}
            BodyKind::Static { init, .. } => scan.tree(init),
            BodyKind::Vtable { entries, .. } => {
                for entry in entries {
                    match entry {
                        VtableEntry::Drop(Some(item)) | VtableEntry::Method(item) => {
                            scan.item(*item)
                        }
                        VtableEntry::TraitVPtr(name) => {
                            scan.names.insert(*name);
                        }
                        VtableEntry::Drop(None)
                        | VtableEntry::Size(_)
                        | VtableEntry::Align(_)
                        | VtableEntry::Vacant => {}
                    }
                }
            }
        }
        for block in &self.blocks {
            for statement in &block.statements {
                match &statement.kind {
                    StatementKind::Assign { value, .. } => scan.rvalue(value),
                    StatementKind::Assume(value) => scan.operand(value),
                    StatementKind::CopyNonOverlapping {
                        source,
                        destination,
                        count,
                    } => {
                        for value in [source, destination, count] {
                            scan.operand(value);
                        }
                    }
                    StatementKind::SetDiscriminant { .. }
                    | StatementKind::StorageLive(_)
                    | StatementKind::StorageDead(_)
                    | StatementKind::Nop => {}
                }
            }
            match &block.terminator {
                Terminator::Call(call) => {
                    scan.operand(&call.function);
                    for arg in &call.arguments {
                        scan.operand(arg);
                    }
                    match &call.callee {
                        Callee::Direct { link_name, .. } => {
                            scan.names.insert(*link_name);
                        }
                        Callee::Foreign { symbol, .. } => {
                            scan.names.insert(*symbol);
                        }
                        Callee::Intrinsic {
                            fallback: Some(item),
                            ..
                        } => scan.item(*item),
                        Callee::Intrinsic { fallback: None, .. }
                        | Callee::Indirect { .. }
                        | Callee::Virtual { .. }
                        | Callee::Runtime { .. }
                        | Callee::Nop => {}
                    }
                    if let Some(host) = &call.host {
                        scan.host(host);
                    }
                }
                Terminator::Drop { glue, .. } => scan.drop(*glue),
                Terminator::Switch { discriminant, .. } => scan.operand(discriminant),
                Terminator::Assert {
                    condition, message, ..
                } => {
                    scan.operand(condition);
                    match message {
                        AssertMessage::BoundsCheck { length, index } => {
                            scan.operand(length);
                            scan.operand(index);
                        }
                        AssertMessage::Overflow { left, right, .. } => {
                            scan.operand(left);
                            scan.operand(right);
                        }
                        AssertMessage::MisalignedPointer { required, found } => {
                            scan.operand(required);
                            scan.operand(found);
                        }
                        AssertMessage::OverflowNeg(value)
                        | AssertMessage::DivisionByZero(value)
                        | AssertMessage::RemainderByZero(value)
                        | AssertMessage::InvalidEnum(value) => scan.operand(value),
                        AssertMessage::ResumedAfterReturn(_)
                        | AssertMessage::ResumedAfterPanic(_)
                        | AssertMessage::ResumedAfterDrop(_)
                        | AssertMessage::NullPointer
                        | AssertMessage::NullReference => {}
                    }
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
        scan.names
    }
}

#[derive(Default)]
struct Dependencies {
    names: BTreeSet<StrId>,
}

impl Dependencies {
    fn item(&mut self, item: DirectItemRef) {
        self.names.insert(item.link_name);
    }
    fn drop(&mut self, glue: DropGlue) {
        match glue {
            DropGlue::Direct { link_name, .. } => {
                self.names.insert(link_name);
            }
            DropGlue::None | DropGlue::Dynamic => {}
        }
    }
    fn global(&mut self, global: GlobalReference) {
        match global {
            GlobalReference::Function { link_name, .. }
            | GlobalReference::Static { link_name, .. } => {
                self.names.insert(link_name);
            }
            GlobalReference::Type(_) => {}
        }
    }
    fn tree(&mut self, tree: &ConstTree) {
        let mut work = vec![tree];
        while let Some(tree) = work.pop() {
            match tree {
                ConstTree::Global { target, .. } => self.global(*target),
                ConstTree::Aggregate(fields)
                | ConstTree::Variant { fields, .. }
                | ConstTree::Slice(fields) => work.extend(fields),
                ConstTree::Reference(value) => work.push(value),
                ConstTree::Dyn { data, vtable, .. } => {
                    self.names.insert(*vtable);
                    work.push(data);
                }
                ConstTree::Unit
                | ConstTree::Scalar { .. }
                | ConstTree::Str(_)
                | ConstTree::Address(_)
                | ConstTree::SliceAddress { .. } => {}
            }
        }
    }
    fn operand(&mut self, operand: &Operand) {
        match &operand.kind {
            OperandKind::Constant(constant) => match constant {
                Constant::Pointer { target, .. } => self.global(*target),
                Constant::Memory(tree) => self.tree(tree),
                Constant::ZeroSized | Constant::Scalar { .. } | Constant::String(_) => {}
            },
            OperandKind::Copy(_) | OperandKind::Move(_) | OperandKind::RuntimeCheck(_) => {}
        }
    }
    fn rvalue(&mut self, value: &Rvalue) {
        match value {
            Rvalue::Use(value)
            | Rvalue::Repeat { value, .. }
            | Rvalue::Cast { value, .. }
            | Rvalue::Unary { value, .. }
            | Rvalue::WrapUnsafeBinder(value) => self.operand(value),
            Rvalue::Binary { left, right, .. } => {
                self.operand(left);
                self.operand(right);
            }
            Rvalue::Aggregate { operands, .. } => {
                for value in operands {
                    self.operand(value);
                }
            }
            Rvalue::Reference { .. }
            | Rvalue::RawPointer { .. }
            | Rvalue::Discriminant(_)
            | Rvalue::CopyForDeref(_)
            | Rvalue::Reborrow { .. } => {}
        }
    }
    fn host(&mut self, host: &HostPayload) {
        match host {
            HostPayload::Marker {
                template, helpers, ..
            } => {
                if let Some(template) = template {
                    self.tree(template);
                }
                for (_, item) in helpers {
                    self.item(*item);
                }
            }
            HostPayload::ViewAbi { helpers, .. } => {
                for (_, item) in helpers {
                    self.item(*item);
                }
            }
            HostPayload::KeyedList(payload) => {
                for callable in [&payload.collection, &payload.key, &payload.row] {
                    if let Some(item) = callable.body {
                        self.item(item);
                    }
                    self.drop(callable.cleanup);
                }
            }
            HostPayload::Extern(_) | HostPayload::Block(_) | HostPayload::Map { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(kind: BodyKind) -> Body {
        Body {
            item: item(0).item,
            kind,
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: vec![],
                result: TypeId(1),
                abi: Default::default(),
            },
            locals: vec![],
            blocks: vec![],
        }
    }
    fn item(id: u32) -> DirectItemRef {
        DirectItemRef {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash: id.into(),
                },
                instantiation: 0,
            },
            link_name: StrId(id),
        }
    }
    fn global(id: u32) -> ConstTree {
        ConstTree::Global {
            target: GlobalReference::Function {
                item: item(id).item,
                link_name: StrId(id),
            },
            offset: 0,
        }
    }
    fn operand(id: u32) -> Operand {
        Operand {
            ty: TypeId(1),
            kind: OperandKind::Constant(Constant::Memory(global(id))),
        }
    }
    fn names(ids: &[u32]) -> BTreeSet<StrId> {
        ids.iter().copied().map(StrId).collect()
    }

    #[test]
    fn constant_item_references_do_not_confuse_strings_with_link_names() {
        let init = ConstTree::Aggregate(vec![
            ConstTree::Str(StrId(90)),
            ConstTree::Reference(Box::new(ConstTree::Slice(vec![
                global(1),
                global(1),
                ConstTree::Variant {
                    variant: 0,
                    fields: vec![ConstTree::Global {
                        target: GlobalReference::Static {
                            definition: item(2).item.definition,
                            link_name: StrId(2),
                        },
                        offset: 4,
                    }],
                },
            ]))),
        ]);
        assert_eq!(
            body(BodyKind::Static {
                init,
                mutable: false
            })
            .link_dependencies(),
            names(&[1, 2])
        );
        assert_eq!(
            body(BodyKind::Vtable {
                concrete: TypeId(1),
                trait_ref: None,
                entries: vec![
                    VtableEntry::Drop(Some(item(3))),
                    VtableEntry::Method(item(4)),
                    VtableEntry::TraitVPtr(StrId(5)),
                    VtableEntry::Drop(None),
                    VtableEntry::Size(6)
                ]
            })
            .link_dependencies(),
            names(&[3, 4, 5])
        );
    }

    #[test]
    fn cleanup_and_host_metadata_add_edges_without_baseline_declarations() {
        let mut body = body(BodyKind::Function);
        let place = Place {
            local: LocalId(0),
            projection: vec![],
            ty: TypeId(1),
        };
        body.blocks = vec![
            BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: vec![Statement {
                    kind: StatementKind::Assign {
                        destination: place.clone(),
                        value: Rvalue::Aggregate {
                            kind: AggregateKind::Tuple,
                            result: TypeId(1),
                            operands: vec![operand(6)],
                        },
                    },
                }],
                terminator: Terminator::Call(Call {
                    function: operand(7),
                    callee: Callee::Intrinsic {
                        name: StrId(90),
                        types: vec![],
                        fallback: Some(item(8)),
                    },
                    semantic: CallSemantic::Host,
                    arguments: vec![operand(9)],
                    destination: place.clone(),
                    target: Some(BlockId(1)),
                    unwind: Unwind::Cleanup(BlockId(1)),
                    track_caller: None,
                    host: Some(HostPayload::Marker {
                        section: StrId(91),
                        def: item(0).item.definition,
                        template: Some(global(10)),
                        helpers: vec![(StrId(92), item(11))],
                    }),
                }),
            },
            BasicBlock {
                id: BlockId(1),
                cleanup: true,
                statements: vec![],
                terminator: Terminator::Drop {
                    place,
                    target: BlockId(2),
                    unwind: Unwind::Continue,
                    replace: false,
                    async_drop: None,
                    glue: DropGlue::Direct {
                        item: item(12).item,
                        link_name: StrId(12),
                    },
                },
            },
        ];
        assert_eq!(body.link_dependencies(), names(&[6, 7, 8, 9, 10, 11, 12]));
        let callable = |id| KeyedListCallable {
            ty: TypeId(1),
            body: Some(item(id)),
            cleanup: DropGlue::Direct {
                item: item(id + 1).item,
                link_name: StrId(id + 1),
            },
            source: None,
        };
        let Terminator::Call(call) = &mut body.blocks[0].terminator else {
            unreachable!()
        };
        call.host = Some(HostPayload::KeyedList(Box::new(KeyedListPayload {
            format: KEYED_LIST_FORMAT_V1,
            site: None,
            def: item(0).item.definition,
            collection_ty: TypeId(1),
            item_ty: TypeId(1),
            key_ty: TypeId(1),
            row_ty: TypeId(1),
            collection: callable(20),
            key: callable(22),
            row: callable(24),
        })));
        assert_eq!(
            body.link_dependencies(),
            names(&[6, 7, 8, 9, 12, 20, 21, 22, 23, 24, 25])
        );
    }

    #[test]
    fn dyn_const_records_vtable_and_nested_data_dependencies() {
        let init = ConstTree::Dyn {
            data: Box::new(global(41)),
            concrete: TypeId(1),
            vtable: StrId(42),
        };
        assert_eq!(
            body(BodyKind::Static {
                init,
                mutable: false,
            })
            .link_dependencies(),
            names(&[41, 42])
        );
    }
}
