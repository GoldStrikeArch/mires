//! MIR → JIR-S: the tcx side of the schema in `rcgjs-object`.
//!
//! One [`Encoder`] encodes one item — a function's post-optimization MIR, a static's evaluated
//! initializer, or a vtable — into a [`Body`] plus its [`OriginTable`], interning every string,
//! byte blob, signature, source fact and type into the codegen unit's crate-scope
//! [`Interns`]. Every produced body is verified against those tables before it is handed back; a
//! verification failure is an [`EncodeDecline::Invalid`], never a panic.
//!
//! The encoder is deliberately conservative in exactly the places v1's `late_ir.rs` was: one
//! unsupported operation (a thread-local reference, a `Yield`/`InlineAsm`/`TailCall` terminator, a
//! constant that cannot be decoded exactly) declines the whole item with a reason naming the
//! statement or terminator, and the printed JavaScript stays the executable fallback.

use std::collections::BTreeMap;

use rustc_abi::Size;
use rustc_hir::def_id::DefId;
use rustc_middle::mir;
use rustc_middle::mir::interpret::{GlobalAlloc, Scalar};
use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_middle::ty::{
    self, ExistentialTraitRef, Instance, ScalarInt, Ty, TyCtxt, TypeFoldable, VtblEntry,
};
use rustc_span::Span;

use rcgjs_object::jir::decline::EncodeDecline;
use rcgjs_object::jir::interns::{Interns, StrId};
use rcgjs_object::jir::origin::{
    Origin, OriginId, OriginKind, OriginTable, SourceFact, SourceFactId,
};
use rcgjs_object::jir::semantic::{
    AbiSignature, AggregateKind, AssertMessage, BasicBlock, BinaryOp, BlockId, Body, BodyKind,
    BodySemantic, BorrowKind, Call, CallSemantic, Callee, CastKind, ConstTree, Constant,
    CoroutineDesugaring, CoroutineKind, CoroutineSource, DefinitionId, DirectItemRef, DropGlue,
    GlobalReference, HostPayload, ItemId, KeyedListCallable, KeyedListPayload, Local, LocalId,
    LocalRole, Operand, OperandKind, Place, Projection, RawPointerKind, Rvalue, Signature,
    SpreadArgument, StandardOperation, Statement, StatementKind, TerminateReason, Terminator,
    TypeId, UnaryOp, Unwind, VtableEntry, KEYED_LIST_FORMAT_V1,
};

use super::types::{definition_id, type_name, TypeCx};
use super::JirItem;
use crate::cgu::CguCx;
use crate::value::typing_env;

/// The largest anonymous constant allocation decoded into an owned constant tree.
const MAX_CONST_MEMORY_BYTES: u64 = 65536;

/// The deepest pointer chain followed while decoding constant memory.
const MAX_CONST_TREE_DEPTH: u32 = 64;

/// The `link_section` prefix every `view-abi` marker carries (`abi.rs`).
const VIEW_ABI_PREFIX: &str = "rcgjs.tc.";

/// Whether the debug-only self checks (postcard round trip of every body) run.
pub(crate) fn self_check() -> bool {
    cfg!(debug_assertions)
        || std::env::var_os("JSC_JIR_SELFCHECK").is_some_and(|value| value == "1")
}

/// Encodes a monomorphic function body.
pub(crate) fn function<'tcx>(
    cgu: &CguCx<'tcx>,
    instance: Instance<'tcx>,
    mir: &mir::Body<'tcx>,
) -> Result<JirItem, EncodeDecline> {
    let mut interns = cgu.interns_mut();
    let mut encoder = Encoder::new(cgu, &mut interns, Some((instance, mir)));
    let result = encoder.function_body();
    encoder.finish(result)
}

/// Encodes a `static`: its identity, type, and decoded initializer.
pub(crate) fn static_item<'tcx>(
    cgu: &CguCx<'tcx>,
    def_id: DefId,
) -> Result<JirItem, EncodeDecline> {
    let mut interns = cgu.interns_mut();
    let mut encoder = Encoder::new(cgu, &mut interns, None);
    let result = encoder.static_body(def_id);
    encoder.finish(result)
}

/// Encodes the vtable of `ty as dyn trait_ref` (`vtable.rs` builds the JavaScript array).
pub(crate) fn vtable<'tcx>(
    cgu: &CguCx<'tcx>,
    ty: Ty<'tcx>,
    trait_ref: Option<ExistentialTraitRef<'tcx>>,
) -> Result<JirItem, EncodeDecline> {
    let mut interns = cgu.interns_mut();
    let mut encoder = Encoder::new(cgu, &mut interns, None);
    let result = encoder.vtable_body(ty, trait_ref);
    encoder.finish(result)
}

struct Encoder<'a, 'tcx> {
    cgu: &'a CguCx<'tcx>,
    types: TypeCx<'a, 'tcx>,
    /// The function being encoded and its MIR; `None` for a static or vtable, whose types need
    /// no generic context.
    function: Option<(Instance<'tcx>, &'a mir::Body<'tcx>)>,
    // Origins.
    origins: Vec<Origin>,
    origin_index: BTreeMap<OriginKey, OriginId>,
    statement_origins: Vec<OriginId>,
    terminator_origins: Vec<OriginId>,
    local_origins: Vec<OriginId>,
    scope_stacks: BTreeMap<u32, Vec<OriginId>>,
    item_path: Option<StrId>,
    root: Option<SourceFactId>,
}

/// The dedup key of an origin: its fields, which are all `Ord`.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct OriginKey {
    kind: u8,
    generated: Option<StrId>,
    source: Option<SourceFactId>,
    local_name: Option<StrId>,
    inline_stack: Vec<OriginId>,
}

impl<'a, 'tcx> Encoder<'a, 'tcx> {
    fn new(
        cgu: &'a CguCx<'tcx>,
        interns: &'a mut Interns,
        function: Option<(Instance<'tcx>, &'a mir::Body<'tcx>)>,
    ) -> Self {
        Self {
            cgu,
            types: TypeCx::new(cgu, interns),
            function,
            origins: Vec::new(),
            origin_index: BTreeMap::new(),
            statement_origins: Vec::new(),
            terminator_origins: Vec::new(),
            local_origins: Vec::new(),
            scope_stacks: BTreeMap::new(),
            item_path: None,
            root: None,
        }
    }

    fn tcx(&self) -> TyCtxt<'tcx> {
        self.cgu.tcx
    }

    fn mir(&self) -> &'a mir::Body<'tcx> {
        self.function.expect("a function encoder has a MIR body").1
    }

    fn instance(&self) -> Instance<'tcx> {
        self.function.expect("a function encoder has an instance").0
    }

    /// Verifies the body and its origins, rolls the type table back on failure, and runs the
    /// debug-only postcard round trip.
    fn finish(mut self, result: Result<Body, EncodeDecline>) -> Result<JirItem, EncodeDecline> {
        let body = match result {
            Ok(body) => body,
            Err(decline) => {
                self.types.rollback();
                return Err(decline);
            }
        };
        let origins = OriginTable {
            origins: std::mem::take(&mut self.origins),
            statements: std::mem::take(&mut self.statement_origins),
            terminators: std::mem::take(&mut self.terminator_origins),
            locals: std::mem::take(&mut self.local_origins),
            item_path: self.item_path.unwrap_or_else(|| self.types.str("")),
            root: self.root,
        };
        let verified = {
            let interns: &Interns = &*self.types.interns;
            body.verify(&interns.types, interns)
                .and_then(|()| body.verify_origins(&origins))
        };
        if let Err(error) = verified {
            self.types.rollback();
            return Err(EncodeDecline::Invalid {
                message: error.message,
            });
        }
        if self_check() {
            if let Err(message) = round_trip(&body, &origins) {
                self.types.rollback();
                return Err(EncodeDecline::Codec { message });
            }
        }
        Ok(JirItem { body, origins })
    }

    // -----------------------------------------------------------------------------------------
    // Functions
    // -----------------------------------------------------------------------------------------

    fn function_body(&mut self) -> Result<Body, EncodeDecline> {
        let tcx = self.tcx();
        let mir = self.mir();
        self.item_path = Some(self.types.str(&with_no_trimmed_paths!(
            tcx.def_path_str(self.instance().def_id())
        )));
        self.root = self
            .source_fact(mir.span)
            .or_else(|| self.source_fact(tcx.def_span(self.instance().def_id())));

        let host_body = crate::js_extern::extern_section(tcx, self.instance().def_id());
        let local_count = if host_body.is_some() {
            mir.arg_count + 1
        } else {
            mir.local_decls.len()
        };
        let local_names = local_names(mir);
        let mut locals = Vec::with_capacity(local_count);
        for (local, declaration) in mir.local_decls.iter_enumerated().take(local_count) {
            let id = LocalId(local.as_usize() as u32);
            let ty = self.type_id(declaration.ty)?;
            let role = if local == mir::RETURN_PLACE {
                LocalRole::Return
            } else if local.as_usize() <= mir.arg_count {
                LocalRole::Argument
            } else {
                LocalRole::Temporary
            };
            locals.push(Local { id, role, ty });
            let name = local_names[local.as_usize()]
                .as_deref()
                .map(|name| self.types.str(name));
            let origin = self.origin(
                if role == LocalRole::Argument {
                    OriginKind::Argument
                } else {
                    OriginKind::Local
                },
                declaration.source_info,
                name,
            );
            self.local_origins.push(origin);
        }
        let signature = Signature {
            arguments: locals
                .iter()
                .filter(|local| local.role == LocalRole::Argument)
                .map(|local| local.ty)
                .collect(),
            result: locals[mir::RETURN_PLACE.as_usize()].ty,
            abi: self.abi_signature()?,
        };
        let semantic = self.body_semantic()?;

        let blocks = if let Some(descriptor) = host_body {
            self.host_function_body(&descriptor)?
        } else {
            let mut blocks = Vec::with_capacity(mir.basic_blocks.len());
            for (block, data) in mir.basic_blocks.iter_enumerated() {
                blocks.push(self.basic_block(block, data)?);
            }
            blocks
        };
        Ok(Body {
            item: item_id(self.cgu, self.instance()),
            kind: BodyKind::Function,
            semantic,
            signature,
            locals,
            blocks,
        })
    }

    /// A binding used as a function value executes its host operation, just like a direct call.
    /// Its Rust placeholder contributes no locals or panic edges to the host body.
    fn host_function_body(&mut self, descriptor: &str) -> Result<Vec<BasicBlock>, EncodeDecline> {
        let instance = self.instance();
        let function_ty = Ty::new_fn_def(
            self.tcx(),
            instance.def_id(),
            ty::Binder::dummy(instance.args),
        );
        let function = Operand {
            ty: self.type_id(function_ty)?,
            kind: OperandKind::Constant(Constant::ZeroSized),
        };
        let (callee, _) = self.callee(function_ty)?;
        let arguments = self
            .mir()
            .args_iter()
            .map(|local| self.operand(&mir::Operand::Copy(local.into())))
            .collect::<Result<Vec<_>, _>>()?;
        let origin = self.origin(
            OriginKind::Terminator,
            self.mir().basic_blocks[mir::START_BLOCK]
                .terminator()
                .source_info,
            None,
        );
        self.terminator_origins.extend([origin, origin]);
        let call = Call {
            function,
            callee,
            semantic: CallSemantic::Host,
            arguments,
            destination: self.place(mir::RETURN_PLACE.into())?,
            target: Some(BlockId(1)),
            unwind: Unwind::Unreachable,
            track_caller: None,
            host: Some(HostPayload::Extern(
                self.types.interns.intern_bytes(descriptor.as_bytes()),
            )),
        };
        Ok(vec![
            BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Call(call),
            },
            BasicBlock {
                id: BlockId(1),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Return,
            },
        ])
    }

    fn abi_signature(&mut self) -> Result<AbiSignature, EncodeDecline> {
        let mir = self.mir();
        let spread = match mir.spread_arg {
            Some(local) => {
                let ty = self.monomorphize(mir.local_decls[local].ty);
                let ty::Tuple(elements) = ty.kind() else {
                    return Err(EncodeDecline::SpreadNotTuple);
                };
                Some(SpreadArgument {
                    local: LocalId(local.as_usize() as u32),
                    arity: u16::try_from(elements.len())
                        .map_err(|_| EncodeDecline::SpreadTooLarge)?,
                })
            }
            None => None,
        };
        let caller_location = self.instance().def.requires_caller_location(self.tcx());
        let inline_never = matches!(
            self.instance().def,
            ty::InstanceKind::Item(def_id)
                if matches!(
                    self.tcx().def_kind(def_id),
                    rustc_hir::def::DefKind::Fn | rustc_hir::def::DefKind::AssocFn
                ) && matches!(
                    self.tcx().codegen_fn_attrs(def_id).inline,
                    rustc_hir::attrs::InlineAttr::Never
                )
        );
        let parameters = mir
            .arg_count
            .checked_sub(usize::from(spread.is_some()))
            .and_then(|count| count.checked_add(spread.map_or(0, |spread| spread.arity.into())))
            .and_then(|count| count.checked_add(usize::from(caller_location)))
            .and_then(|count| u16::try_from(count).ok())
            .ok_or(EncodeDecline::TooManyParameters)?;
        Ok(AbiSignature {
            parameters,
            spread,
            caller_location,
            inline_never,
        })
    }

    fn body_semantic(&mut self) -> Result<BodySemantic, EncodeDecline> {
        if let ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, Some(ty))) = self.instance().def {
            return Ok(BodySemantic::DropGlue(self.type_id(ty)?));
        }
        Ok(crate::late_ir::standard_operation(
            self.tcx(),
            self.instance().def_id(),
            self.instance().args,
        )
        .map(standard_operation_v2)
        .map_or(BodySemantic::Ordinary, BodySemantic::Standard))
    }

    fn basic_block(
        &mut self,
        block: mir::BasicBlock,
        data: &mir::BasicBlockData<'tcx>,
    ) -> Result<BasicBlock, EncodeDecline> {
        let id = BlockId(block.as_usize() as u32);
        let mut statements = Vec::with_capacity(data.statements.len());
        for (index, statement) in data.statements.iter().enumerate() {
            let origin = self.origin(OriginKind::Statement, statement.source_info, None);
            self.statement_origins.push(origin);
            let kind =
                self.statement(&statement.kind)
                    .map_err(|reason| EncodeDecline::AtStatement {
                        block: id.0,
                        index: index as u32,
                        reason: Box::new(reason),
                    })?;
            statements.push(Statement { kind });
        }
        let terminator = data.terminator();
        let origin = self.origin(OriginKind::Terminator, terminator.source_info, None);
        self.terminator_origins.push(origin);
        let terminator =
            self.terminator(&terminator.kind)
                .map_err(|reason| EncodeDecline::AtTerminator {
                    block: id.0,
                    reason: Box::new(reason),
                })?;
        Ok(BasicBlock {
            id,
            cleanup: data.is_cleanup,
            statements,
            terminator,
        })
    }

    fn statement(
        &mut self,
        statement: &mir::StatementKind<'tcx>,
    ) -> Result<StatementKind, EncodeDecline> {
        Ok(match statement {
            mir::StatementKind::Assign(assignment) => StatementKind::Assign {
                destination: self.place(assignment.0)?,
                value: self.rvalue(&assignment.1)?,
            },
            mir::StatementKind::SetDiscriminant {
                place,
                variant_index,
            } => StatementKind::SetDiscriminant {
                place: self.place(**place)?,
                variant: variant_index.as_usize() as u32,
            },
            mir::StatementKind::StorageLive(local) => {
                StatementKind::StorageLive(LocalId(local.as_usize() as u32))
            }
            mir::StatementKind::StorageDead(local) => {
                StatementKind::StorageDead(LocalId(local.as_usize() as u32))
            }
            mir::StatementKind::Intrinsic(intrinsic) => match &**intrinsic {
                mir::NonDivergingIntrinsic::Assume(value) => {
                    StatementKind::Assume(self.operand(value)?)
                }
                mir::NonDivergingIntrinsic::CopyNonOverlapping(copy) => {
                    // The consumer lowers this statement as an `Intrinsic` op named by an
                    // interned string it cannot mint itself (`jsc-link/src/lower/ssa.rs`).
                    self.types.str("copy_nonoverlapping");
                    StatementKind::CopyNonOverlapping {
                        source: self.operand(&copy.src)?,
                        destination: self.operand(&copy.dst)?,
                        count: self.operand(&copy.count)?,
                    }
                }
            },
            mir::StatementKind::FakeRead(..)
            | mir::StatementKind::PlaceMention(..)
            | mir::StatementKind::AscribeUserType(..)
            | mir::StatementKind::Coverage(..)
            | mir::StatementKind::ConstEvalCounter
            | mir::StatementKind::Nop
            | mir::StatementKind::BackwardIncompatibleDropHint { .. } => StatementKind::Nop,
        })
    }

    fn rvalue(&mut self, rvalue: &mir::Rvalue<'tcx>) -> Result<Rvalue, EncodeDecline> {
        let tcx = self.tcx();
        Ok(match rvalue {
            mir::Rvalue::Use(value, _) => Rvalue::Use(self.operand(value)?),
            mir::Rvalue::Repeat(value, count) => Rvalue::Repeat {
                value: self.operand(value)?,
                count: self
                    .monomorphize(*count)
                    .try_to_target_usize(tcx)
                    .ok_or(EncodeDecline::RepeatCount)?,
            },
            mir::Rvalue::Ref(_, kind, place) => Rvalue::Reference {
                kind: borrow_kind(*kind),
                place: self.place(*place)?,
            },
            mir::Rvalue::RawPtr(kind, place) => Rvalue::RawPointer {
                kind: raw_pointer_kind(*kind),
                place: self.place(*place)?,
            },
            mir::Rvalue::Cast(kind, value, result) => Rvalue::Cast {
                kind: cast_kind(*kind),
                value: self.operand(value)?,
                result: self.type_id(*result)?,
            },
            mir::Rvalue::BinaryOp(op, operands) => Rvalue::Binary {
                op: binary_op(*op),
                left: self.operand(&operands.0)?,
                right: self.operand(&operands.1)?,
            },
            mir::Rvalue::UnaryOp(op, value) => Rvalue::Unary {
                op: unary_op(*op),
                value: self.operand(value)?,
            },
            mir::Rvalue::Discriminant(place) => Rvalue::Discriminant(self.place(*place)?),
            mir::Rvalue::Aggregate(kind, operands) => Rvalue::Aggregate {
                kind: self.aggregate_kind(kind)?,
                result: self.type_id(rvalue.ty(&self.mir().local_decls, tcx))?,
                operands: operands
                    .iter()
                    .map(|operand| self.operand(operand))
                    .collect::<Result<_, _>>()?,
            },
            mir::Rvalue::CopyForDeref(place) => Rvalue::CopyForDeref(self.place(*place)?),
            mir::Rvalue::WrapUnsafeBinder(value, _) => {
                Rvalue::WrapUnsafeBinder(self.operand(value)?)
            }
            mir::Rvalue::Reborrow(_, mutable, place) => Rvalue::Reborrow {
                mutable: mutable.is_mut(),
                place: self.place(*place)?,
            },
            mir::Rvalue::ThreadLocalRef(_) => return Err(EncodeDecline::ThreadLocal),
        })
    }

    fn aggregate_kind(
        &mut self,
        kind: &mir::AggregateKind<'tcx>,
    ) -> Result<AggregateKind, EncodeDecline> {
        let tcx = self.tcx();
        Ok(match kind {
            mir::AggregateKind::Array(element) => AggregateKind::Array(self.type_id(*element)?),
            mir::AggregateKind::Tuple => AggregateKind::Tuple,
            mir::AggregateKind::Adt(definition, variant, _, _, active_field) => {
                AggregateKind::Adt {
                    definition: definition_id(tcx, *definition),
                    variant: variant.as_usize() as u32,
                    active_field: active_field.map(|field| field.as_usize() as u32),
                }
            }
            mir::AggregateKind::Closure(definition, _) => {
                AggregateKind::Closure(definition_id(tcx, *definition))
            }
            mir::AggregateKind::Coroutine(definition, _) => {
                AggregateKind::Coroutine(definition_id(tcx, *definition))
            }
            mir::AggregateKind::CoroutineClosure(definition, _) => {
                AggregateKind::CoroutineClosure(definition_id(tcx, *definition))
            }
            mir::AggregateKind::RawPtr(pointee, mutable) => AggregateKind::RawPointer {
                pointee: self.type_id(*pointee)?,
                mutable: mutable.is_mut(),
            },
        })
    }

    fn terminator(
        &mut self,
        terminator: &mir::TerminatorKind<'tcx>,
    ) -> Result<Terminator, EncodeDecline> {
        Ok(match terminator {
            mir::TerminatorKind::Goto { target } => Terminator::Goto(block_id(*target)),
            mir::TerminatorKind::SwitchInt { discr, targets } => Terminator::Switch {
                discriminant: self.operand(discr)?,
                targets: targets
                    .iter()
                    .map(|(value, target)| (value, block_id(target)))
                    .collect(),
                otherwise: block_id(targets.otherwise()),
            },
            mir::TerminatorKind::Return => Terminator::Return,
            mir::TerminatorKind::Unreachable => Terminator::Unreachable,
            mir::TerminatorKind::Call {
                func,
                args,
                destination,
                target,
                unwind,
                fn_span,
                ..
            } => {
                Terminator::Call(self.call(func, args, *destination, *target, *unwind, *fn_span)?)
            }
            mir::TerminatorKind::Drop {
                place,
                target,
                unwind,
                replace,
                drop,
            } => Terminator::Drop {
                place: self.place(*place)?,
                target: block_id(*target),
                unwind: unwind_action(*unwind),
                replace: *replace,
                async_drop: drop.map(block_id),
                glue: self.drop_glue(*place),
            },
            mir::TerminatorKind::Assert {
                cond,
                expected,
                msg,
                target,
                unwind,
            } => {
                // Phase C spells a failed MIR assert as a call to a track-caller panic lang item.
                // The Location type is otherwise absent from a producer rlib whose only bounds
                // checks live in that producer, so preserve its descriptor with the assertion.
                self.describe_caller_location_type()?;
                Terminator::Assert {
                    condition: self.operand(cond)?,
                    expected: *expected,
                    message: self.assert_message(msg)?,
                    target: block_id(*target),
                    unwind: unwind_action(*unwind),
                }
            }
            mir::TerminatorKind::UnwindResume => Terminator::UnwindResume,
            mir::TerminatorKind::UnwindTerminate(reason) => {
                Terminator::UnwindTerminate(terminate_reason(*reason))
            }
            mir::TerminatorKind::FalseEdge {
                real_target,
                imaginary_target,
            } => Terminator::FalseEdge {
                real: block_id(*real_target),
                imaginary: block_id(*imaginary_target),
            },
            mir::TerminatorKind::FalseUnwind {
                real_target,
                unwind,
            } => Terminator::FalseUnwind {
                real: block_id(*real_target),
                unwind: unwind_action(*unwind),
            },
            other => {
                return Err(EncodeDecline::UnsupportedTerminator {
                    name: other.name().to_owned(),
                })
            }
        })
    }

    fn assert_message(
        &mut self,
        message: &mir::AssertMessage<'tcx>,
    ) -> Result<AssertMessage, EncodeDecline> {
        Ok(match message {
            mir::AssertKind::BoundsCheck { len, index } => AssertMessage::BoundsCheck {
                length: self.operand(len)?,
                index: self.operand(index)?,
            },
            mir::AssertKind::Overflow(op, left, right) => AssertMessage::Overflow {
                op: binary_op(*op),
                left: self.operand(left)?,
                right: self.operand(right)?,
            },
            mir::AssertKind::OverflowNeg(value) => AssertMessage::OverflowNeg(self.operand(value)?),
            mir::AssertKind::DivisionByZero(value) => {
                AssertMessage::DivisionByZero(self.operand(value)?)
            }
            mir::AssertKind::RemainderByZero(value) => {
                AssertMessage::RemainderByZero(self.operand(value)?)
            }
            mir::AssertKind::ResumedAfterReturn(kind) => {
                AssertMessage::ResumedAfterReturn(coroutine_kind(*kind))
            }
            mir::AssertKind::ResumedAfterPanic(kind) => {
                AssertMessage::ResumedAfterPanic(coroutine_kind(*kind))
            }
            mir::AssertKind::ResumedAfterDrop(kind) => {
                AssertMessage::ResumedAfterDrop(coroutine_kind(*kind))
            }
            mir::AssertKind::MisalignedPointerDereference { required, found } => {
                AssertMessage::MisalignedPointer {
                    required: self.operand(required)?,
                    found: self.operand(found)?,
                }
            }
            mir::AssertKind::NullPointerDereference => AssertMessage::NullPointer,
            mir::AssertKind::NullReferenceConstructed => AssertMessage::NullReference,
            mir::AssertKind::InvalidEnumConstruction(value) => {
                AssertMessage::InvalidEnum(self.operand(value)?)
            }
        })
    }

    // -----------------------------------------------------------------------------------------
    // Calls
    // -----------------------------------------------------------------------------------------

    fn call(
        &mut self,
        func: &mir::Operand<'tcx>,
        args: &[rustc_span::Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
        target: Option<mir::BasicBlock>,
        unwind: mir::UnwindAction,
        fn_span: Span,
    ) -> Result<Call, EncodeDecline> {
        let tcx = self.tcx();
        let function_ty = self.monomorphize(func.ty(&self.mir().local_decls, tcx));
        let function = self.operand(func)?;
        let (callee, tracks_caller) = self.callee(function_ty)?;
        let semantic = self.call_semantic(function_ty);
        let host = self.host_payload(function_ty, args, fn_span)?;
        let arguments = self.call_arguments(function_ty, args)?;
        // A `#[track_caller]` body forwards its own location parameter; anything else spells the
        // call site (`intrinsics::caller_location_argument`).
        let track_caller = if tracks_caller && !self.instance().def.requires_caller_location(tcx) {
            Some(self.caller_location_fact(fn_span)?)
        } else {
            None
        };
        Ok(Call {
            function,
            callee,
            semantic,
            arguments,
            destination: self.place(destination)?,
            target: target.map(block_id),
            unwind: unwind_action(unwind),
            track_caller,
            host,
        })
    }

    /// The JavaScript argument list of a call: the MIR arguments in order, with a
    /// `"rust-call"` callee's trailing tuple already **splatted** into one argument per element
    /// (v1 `abi.rs::splat_rust_call_args`).
    ///
    /// The splat happens here rather than in the consumer because the call site's ABI is a fact
    /// only rustc has: `Callee::Direct` names an item, and a closure body, an `FnOnce::call_once`
    /// shim and an ordinary function taking a one-element tuple are indistinguishable from the
    /// item alone. Splatting at encode time also puts the caller side in the convention the
    /// callee side is already encoded in — [`AbiSignature::parameters`] counts a spread argument
    /// as its arity — so a JIR-S call's arguments and its callee's parameters line up one to one.
    ///
    /// Each element is read as `Operand::Copy` of the pack's field, which is exactly the element
    /// read v1 emits (a place read, cloned when the element type needs cloning).
    fn call_arguments(
        &mut self,
        function_ty: Ty<'tcx>,
        args: &[rustc_span::Spanned<mir::Operand<'tcx>>],
    ) -> Result<Vec<Operand>, EncodeDecline> {
        let tcx = self.tcx();
        let is_rust_call = matches!(function_ty.kind(), ty::FnDef(..) | ty::FnPtr(..))
            && function_ty.fn_sig(tcx).abi() == rustc_abi::ExternAbi::RustCall;
        if !is_rust_call {
            return args
                .iter()
                .map(|argument| self.operand(&argument.node))
                .collect();
        }
        let Some((pack, rest)) = args.split_last() else {
            return Err(EncodeDecline::SpreadNotTuple);
        };
        let mut out = Vec::with_capacity(rest.len() + 1);
        for argument in rest {
            out.push(self.operand(&argument.node)?);
        }
        let pack_ty = self.monomorphize(pack.node.ty(&self.mir().local_decls, tcx));
        let ty::Tuple(elements) = pack_ty.kind() else {
            return Err(EncodeDecline::SpreadNotTuple);
        };
        if elements.is_empty() {
            return Ok(out);
        }
        let (mir::Operand::Copy(place) | mir::Operand::Move(place)) = pack.node else {
            // A **constant** pack has no place to project out of, so it is split by decoding it:
            // a tuple constant is an aggregate tree whose fields are the arguments.
            let decoded = self.operand(&pack.node)?;
            // A ZERO-SIZED pack has no memory either, so `ConstValue::ZeroSized` is the whole of
            // it and there is no tree to take fields out of. A tuple is zero-sized exactly when
            // every element is, so the spread is one zero-sized constant per element. This is the
            // shape `Result::and_then`'s closure takes — `((),)` — and it is what left
            // `fmt::builders::DebugTuple::{field,finish}` without a JIR body until v2 W10.
            if matches!(decoded.kind, OperandKind::Constant(Constant::ZeroSized)) {
                for element_ty in elements.iter() {
                    out.push(Operand {
                        ty: self.type_id(element_ty)?,
                        kind: OperandKind::Constant(Constant::ZeroSized),
                    });
                }
                return Ok(out);
            }
            let fields = match decoded.kind {
                OperandKind::Constant(Constant::Memory(ConstTree::Aggregate(fields))) => fields,
                // A pack whose layout is ONE SCALAR is not memory either: const evaluation keeps
                // such a value in a register, so it arrives as its bits. A tuple is one scalar
                // when a single element is not zero-sized and that element is a scalar —
                // `(usize,)`, `(u32, ())` — which is what a closure called with one literal
                // argument passes once MIR optimization has folded the argument tuple
                // (MIRES-020). The bits are the tuple as memory would hold it, so they go back
                // into a scratch allocation laid out for the tuple, and the tuple is decoded from
                // there exactly as an in-memory pack is.
                OperandKind::Constant(Constant::Scalar { bits, bytes }) => {
                    let scratch = ScalarInt::try_from_uint(bits, Size::from_bytes(bytes))
                        .and_then(|int| crate::constant::raw::scratch_alloc(tcx, int, pack_ty))
                        .ok_or_else(|| EncodeDecline::ConstantNoLayout {
                            ty: type_name(pack_ty),
                        })?;
                    let ConstTree::Aggregate(fields) =
                        self.const_tree(&scratch, Size::ZERO, pack_ty, 0)?
                    else {
                        return Err(EncodeDecline::SpreadNotTuple);
                    };
                    fields
                }
                _ => return Err(EncodeDecline::SpreadNotTuple),
            };
            if fields.len() != elements.len() {
                return Err(EncodeDecline::SpreadNotTuple);
            }
            for (field, element_ty) in fields.into_iter().zip(elements.iter()) {
                out.push(Operand {
                    ty: self.type_id(element_ty)?,
                    kind: OperandKind::Constant(Constant::Memory(field)),
                });
            }
            return Ok(out);
        };
        for (index, element_ty) in elements.iter().enumerate() {
            let element = place.project_deeper(
                &[mir::PlaceElem::Field(
                    rustc_abi::FieldIdx::from_usize(index),
                    element_ty,
                )],
                tcx,
            );
            out.push(self.operand(&mir::Operand::Copy(element))?);
        }
        Ok(out)
    }

    /// The callee and whether it takes a caller location, exactly as `abi.rs::codegen_call`
    /// resolves it.
    fn callee(&mut self, function_ty: Ty<'tcx>) -> Result<(Callee, bool), EncodeDecline> {
        let tcx = self.tcx();
        let ty::FnDef(definition, arguments) = *function_ty.kind() else {
            return Ok((
                Callee::Indirect {
                    signature: self.type_id(function_ty)?,
                },
                false,
            ));
        };
        if tcx.is_foreign_item(definition) {
            let symbol = tcx.symbol_name(Instance::mono(tcx, definition)).name;
            let abi = function_ty.fn_sig(tcx).abi();
            // A Rust-ABI foreign declaration is a Rust symbol some other crate defines (`core`'s
            // `panic_impl`); everything else is the `__rt` print shim (`abi.rs`).
            let callee =
                if abi == rustc_abi::ExternAbi::Rust && crate::jsast::is_plain_ident(symbol) {
                    Callee::Foreign {
                        definition: definition_id(tcx, definition),
                        symbol: self.types.str(symbol),
                    }
                } else {
                    let sysroot = tcx.sess.opts.unstable_opts.force_unstable_if_unmarked
                        || (definition.krate != rustc_span::def_id::LOCAL_CRATE && {
                            let dir = &tcx.sess.target_tlib_path.dir;
                            let source = tcx.used_crate_source(definition.krate);
                            [&source.rlib, &source.rmeta, &source.dylib]
                                .into_iter()
                                .flatten()
                                .any(|path| path.starts_with(dir))
                        });
                    Callee::Runtime {
                        member: self.types.str(&crate::jsast::sanitize_ident(symbol)),
                        sysroot,
                    }
                };
            return Ok((callee, false));
        }
        let Some(arguments) = arguments.no_bound_vars() else {
            return Ok((
                Callee::Indirect {
                    signature: self.type_id(function_ty)?,
                },
                false,
            ));
        };
        let instance = match Instance::try_resolve(tcx, typing_env(), definition, arguments) {
            Ok(Some(instance)) => instance,
            Ok(None) | Err(_) => {
                return Ok((
                    Callee::Indirect {
                        signature: self.type_id(function_ty)?,
                    },
                    false,
                ));
            }
        };
        let tracks_caller = instance.def.requires_caller_location(tcx);
        let callee = match instance.def {
            ty::InstanceKind::Intrinsic(def_id) => {
                let must_override = tcx
                    .intrinsic(def_id)
                    .is_some_and(|intrinsic| intrinsic.must_be_overridden);
                let fallback = if !must_override && tcx.is_mir_available(def_id) {
                    let fallback = Instance::new_raw(def_id, instance.args);
                    Some(self.direct_item(fallback))
                } else {
                    None
                };
                let arguments: Vec<Ty<'tcx>> = instance
                    .args
                    .iter()
                    .filter_map(|argument| argument.as_type())
                    .collect();
                let types = arguments
                    .iter()
                    .map(|argument| self.type_id(*argument))
                    .collect::<Result<_, _>>()?;
                let name = tcx.item_name(def_id);
                if name.as_str() == "caller_location" {
                    self.describe_caller_location_type()?;
                }
                // `type_name`'s *value* is a type's printed path, which no other lowering reads;
                // the descriptor carries one only where a body asks for it.
                if name.as_str() == "type_name" {
                    for argument in &arguments {
                        self.types.record_type_name(*argument)?;
                    }
                }
                Callee::Intrinsic {
                    name: self.types.str(name.as_str()),
                    types,
                    fallback,
                }
            }
            ty::InstanceKind::Virtual(_, slot) => Callee::Virtual {
                slot: slot as u32,
                receiver: self.type_id(instance.args.type_at(0))?,
            },
            ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, None)) => Callee::Nop,
            _ => {
                let item = self.direct_item(instance);
                Callee::Direct {
                    item: item.item,
                    link_name: item.link_name,
                }
            }
        };
        Ok((callee, tracks_caller))
    }

    fn direct_item(&mut self, instance: Instance<'tcx>) -> DirectItemRef {
        self.cgu.require_function(instance);
        DirectItemRef {
            item: item_id(self.cgu, instance),
            link_name: self.types.str(self.cgu.namer.fn_name(instance).as_str()),
        }
    }

    /// v1's `late_ir::Encoder::call_semantic`, unchanged in rule.
    fn call_semantic(&mut self, function_ty: Ty<'tcx>) -> CallSemantic {
        let tcx = self.tcx();
        let ty::FnDef(definition, arguments) = *function_ty.kind() else {
            return CallSemantic::Ordinary;
        };
        let Some(arguments) = arguments.no_bound_vars() else {
            return CallSemantic::Ordinary;
        };
        if let Ok(Some(instance)) = Instance::try_resolve(tcx, typing_env(), definition, arguments)
        {
            if let Some(operation) =
                crate::late_ir::standard_operation(tcx, instance.def_id(), instance.args)
            {
                return call_semantic_for_operation(standard_operation_v2(operation));
            }
            if matches!(instance.def, ty::InstanceKind::Item(_)) {
                if let Some(operation) = str_operation(tcx, instance.def_id(), instance.args) {
                    return CallSemantic::Standard(operation);
                }
            }
        }
        if let Some(operation) = crate::late_ir::standard_operation(tcx, definition, arguments) {
            return call_semantic_for_operation(standard_operation_v2(operation));
        }
        if crate::js_extern::extern_section(tcx, definition).is_some() {
            return CallSemantic::Host;
        }
        if crate::js_block::block_section(tcx, definition).is_some() {
            return CallSemantic::RawJavaScript;
        }
        if view_abi_marker(tcx, definition).is_some() {
            return CallSemantic::ViewAbi;
        }
        if tcx.is_foreign_item(definition) {
            let symbol = tcx.symbol_name(Instance::mono(tcx, definition));
            if matches!(
                symbol.name,
                "__rust_alloc"
                    | "__rust_alloc_zeroed"
                    | "__rust_dealloc"
                    | "__rust_realloc"
                    | "__rust_alloc_error_handler"
            ) {
                return CallSemantic::GlobalAllocator;
            }
        }
        let signature = function_ty.fn_sig(tcx).skip_binder();
        if signature
            .inputs_and_output
            .iter()
            .any(|ty| matches!(ty.kind(), ty::RawPtr(..)))
        {
            return CallSemantic::RawParts;
        }
        CallSemantic::Ordinary
    }

    /// The host-side payload of a call the lowering intercepts, decoded from the marker's
    /// `link_section` (see [`HostPayload`] for the convention).
    fn host_payload(
        &mut self,
        function_ty: Ty<'tcx>,
        args: &[rustc_span::Spanned<mir::Operand<'tcx>>],
        fn_span: Span,
    ) -> Result<Option<HostPayload>, EncodeDecline> {
        let tcx = self.tcx();
        let ty::FnDef(definition, arguments) = *function_ty.kind() else {
            return Ok(None);
        };
        if let Some(descriptor) = crate::js_extern::extern_section(tcx, definition) {
            return Ok(Some(HostPayload::Extern(
                self.types.interns.intern_bytes(descriptor.as_bytes()),
            )));
        }
        if let Some(block) = crate::js_block::block_section(tcx, definition) {
            return Ok(Some(HostPayload::Block(
                self.types.interns.intern_bytes(block.as_bytes()),
            )));
        }
        if let Some(marker) = view_abi_marker(tcx, definition) {
            if marker == "keyedlist.v1" {
                let Some(arguments) = arguments.no_bound_vars() else {
                    return Err(EncodeDecline::Invalid {
                        message: "keyed.marker-bound-type: `rcgjs.tc.keyedlist.v1` has a bound type argument"
                            .to_owned(),
                    });
                };
                let row_ty = function_ty.fn_sig(tcx).skip_binder().output();
                return self
                    .keyed_list_payload(definition, arguments, row_ty, args, fn_span)
                    .map(|payload| Some(HostPayload::KeyedList(Box::new(payload))));
            }
            let helpers = self.marker_helpers(definition, &marker, args);
            if marker == "template" {
                if let Some(static_id) = self.static_argument(args.first()) {
                    let template = self.static_initializer(static_id)?;
                    return Ok(Some(HostPayload::Marker {
                        section: self.types.str(&format!("{VIEW_ABI_PREFIX}{marker}")),
                        def: definition_id(tcx, definition),
                        template: Some(template),
                        helpers,
                    }));
                }
            }
            return Ok(Some(HostPayload::ViewAbi {
                marker: self.types.str(&marker),
                def: definition_id(tcx, definition),
                helpers,
            }));
        }
        if let Some(marker) = crate::map::map_section(tcx, definition) {
            return Ok(Some(HostPayload::Map {
                marker: self.types.str(&marker),
            }));
        }
        if let Some(marker) = crate::host::host_section(tcx, definition) {
            let helpers = self.marker_helpers(definition, &marker, args);
            return Ok(Some(HostPayload::Marker {
                section: self
                    .types
                    .str(&format!("{}{marker}", crate::host::HOST_PREFIX)),
                def: definition_id(tcx, definition),
                template: None,
                helpers,
            }));
        }
        Ok(None)
    }

    /// The rustc-only facts behind the version-one keyed-list semantic boundary.
    fn keyed_list_payload(
        &mut self,
        definition: DefId,
        arguments: ty::GenericArgsRef<'tcx>,
        row_ty: Ty<'tcx>,
        args: &[rustc_span::Spanned<mir::Operand<'tcx>>],
        fn_span: Span,
    ) -> Result<KeyedListPayload, EncodeDecline> {
        let type_arguments: Vec<Ty<'tcx>> = arguments
            .iter()
            .filter_map(|argument| argument.as_type())
            .collect();
        let [collection_ty, item_ty, key_ty, _, _, _] = type_arguments.as_slice() else {
            return Err(EncodeDecline::Invalid {
                message: format!(
                    "keyed.marker-type-arity: `rcgjs.tc.keyedlist.v1` records six type arguments, found {}",
                    type_arguments.len()
                ),
            });
        };
        let [collection, key, row] = args else {
            return Err(EncodeDecline::Invalid {
                message: format!(
                    "keyed.marker-arity: `rcgjs.tc.keyedlist.v1` takes three callables, found {}",
                    args.len()
                ),
            });
        };
        let tcx = self.tcx();
        // A marker nested inside a proc-macro expansion may give rustc the
        // outer attribute as `fn_span`. The keyed macro deliberately spans the
        // row callable at the written `for`, so it is the more precise site;
        // a direct handwritten call falls back to the callee span.
        let site = self
            .source_fact(row.span)
            .or_else(|| self.source_fact(fn_span));
        Ok(KeyedListPayload {
            format: KEYED_LIST_FORMAT_V1,
            site,
            def: definition_id(tcx, definition),
            collection_ty: self.type_id(*collection_ty)?,
            item_ty: self.type_id(*item_ty)?,
            key_ty: self.type_id(*key_ty)?,
            row_ty: self.type_id(row_ty)?,
            collection: self.keyed_list_callable(collection)?,
            key: self.keyed_list_callable(key)?,
            row: self.keyed_list_callable(row)?,
        })
    }

    /// One callable value and the body and cleanup instances rustc resolves for it.
    fn keyed_list_callable(
        &mut self,
        argument: &rustc_span::Spanned<mir::Operand<'tcx>>,
    ) -> Result<KeyedListCallable, EncodeDecline> {
        let tcx = self.tcx();
        let ty = self.monomorphize(argument.node.ty(&self.mir().local_decls, tcx));
        Ok(KeyedListCallable {
            ty: self.type_id(ty)?,
            body: self.callable_body(ty),
            cleanup: self.drop_glue_for_ty(ty),
            source: self.source_fact(argument.span),
        })
    }

    /// Resolves a closure or function-item value to the monomorphic body it calls.
    fn callable_body(&mut self, ty: Ty<'tcx>) -> Option<DirectItemRef> {
        let tcx = self.tcx();
        let instance = match *ty.kind() {
            ty::Closure(definition, arguments) => {
                Instance::resolve_closure(tcx, definition, arguments, ty::ClosureKind::Fn)
            }
            ty::FnDef(definition, arguments) => {
                let arguments = arguments.no_bound_vars()?;
                Instance::try_resolve(tcx, typing_env(), definition, arguments)
                    .ok()
                    .flatten()?
            }
            _ => return None,
        };
        Some(self.direct_item(instance))
    }

    /// The sibling instances a marker's lowering calls, resolved at the marker's own generic
    /// argument (v1 `abi.rs::sibling_item`, `host.rs::host_future_helper`).
    ///
    /// Every one of these is a `tcx` query the consumer cannot run: `display_to_str::<T>`,
    /// `poll_once::<F>` and `poll_result::<F>` are declared beside their marker in `view-abi`
    /// and `mires-js`, and which instance a call needs is decided by the type of one of the
    /// call's own arguments.
    fn marker_helpers(
        &mut self,
        marker: DefId,
        marker_name: &str,
        args: &[rustc_span::Spanned<mir::Operand<'tcx>>],
    ) -> Vec<(StrId, DirectItemRef)> {
        let (helper, argument) = match marker_name {
            "spawn" | "spawnowner" | "spawnowner.v2" => ("poll_once", 0),
            "futurepromise" | "futurenativepromise" => ("poll_result", 0),
            "text" => ("display_to_str", 0),
            // `push_keyed(list, key, row)`: the key is the `Display` value.
            "pushkeyed" => ("display_to_str", 1),
            _ => return Vec::new(),
        };
        let Some(argument) = args.get(argument) else {
            return Vec::new();
        };
        let tcx = self.tcx();
        let ty = self.monomorphize(argument.node.ty(&self.mir().local_decls, tcx));
        let Some(item) = self.sibling_instance(marker, helper, ty) else {
            return Vec::new();
        };
        let name = self.types.str(helper);
        vec![(name, item)]
    }

    /// The instance of the item named `name` beside `sibling`, instantiated at `ty`.
    fn sibling_instance(
        &mut self,
        sibling: DefId,
        name: &str,
        ty: Ty<'tcx>,
    ) -> Option<DirectItemRef> {
        let tcx = self.tcx();
        let parent = tcx.parent(sibling);
        let helper = tcx
            .module_children(parent)
            .iter()
            .find(|child| child.ident.name.as_str() == name)
            .and_then(|child| child.res.opt_def_id())?;
        let arguments = tcx.mk_args(&[ty.into()]);
        let instance = Instance::try_resolve(tcx, typing_env(), helper, arguments)
            .ok()
            .flatten()?;
        Some(self.direct_item(instance))
    }

    /// The `static` a `&STATIC` argument names (`abi.rs::static_argument`).
    ///
    /// The reference is a pointer constant and the allocation its provenance names is the static.
    /// Nothing else is accepted: a template payload built at run time is not a payload the
    /// producer can read at compile time.
    fn static_argument(
        &self,
        argument: Option<&rustc_span::Spanned<mir::Operand<'tcx>>>,
    ) -> Option<DefId> {
        let tcx = self.tcx();
        let value = self.constant_value(&argument?.node)?;
        let mir::ConstValue::Scalar(Scalar::Ptr(pointer, _)) = value else {
            return None;
        };
        let (provenance, _) = pointer.prov_and_relative_offset();
        match tcx.global_alloc(provenance.alloc_id()) {
            GlobalAlloc::Static(def_id) => Some(def_id),
            _ => None,
        }
    }

    /// The value of a marker argument that is a constant, however MIR chose to spell it
    /// (`abi.rs::constant_value`).
    ///
    /// A literal argument does not have to reach the call as an `Operand::Constant`: MIR
    /// routinely assigns it to a temporary first and passes that (`_2 = const {alloc}` then
    /// `template(move _2)`). Both spellings mean the same constant, so the temporary is followed
    /// back to the assignment that filled it.
    fn constant_value(&self, operand: &mir::Operand<'tcx>) -> Option<mir::ConstValue> {
        /// How many assignments to follow, as v1 bounds the same walk.
        const MAX_STEPS: usize = 8;

        let tcx = self.tcx();
        let mut operand = operand.clone();
        for _ in 0..MAX_STEPS {
            if let Some(constant) = operand.constant() {
                let konst = self.monomorphize(constant.const_);
                return konst.eval(tcx, typing_env(), constant.span).ok();
            }
            operand = assigned_operand(self.mir(), operand.place()?.as_local()?)?;
        }
        None
    }

    /// The caller `Location` a `#[track_caller]` callee receives from this call site, as the
    /// source fact `intrinsics::caller_location_value` spells: the outermost macro expansion,
    /// 1-based line, 0-based display column (the `Location` column is this plus one).
    fn caller_location_fact(&mut self, span: Span) -> Result<SourceFactId, EncodeDecline> {
        let tcx = self.tcx();
        let topmost = span.ctxt().outer_expn().expansion_cause().unwrap_or(span);
        let caller = tcx.sess.source_map().lookup_char_pos(topmost.lo());
        let file = caller
            .file
            .name
            .prefer_remapped_unconditionally()
            .to_string();
        let file = self.types.str(&file);
        self.describe_caller_location_type()?;
        Ok(self.types.interns.intern_source_fact(SourceFact {
            file,
            line: caller.line as u32,
            col: caller.col_display as u32,
        }))
    }

    /// Preserve the type Phase C needs to materialize a caller location. It is hidden from MIR's
    /// declared locals, so every semantic operation that can synthesize one records it explicitly.
    fn describe_caller_location_type(&mut self) -> Result<(), EncodeDecline> {
        let tcx = self.tcx();
        if tcx
            .lang_items()
            .get(rustc_hir::LangItem::PanicLocation)
            .is_some()
        {
            self.type_id(tcx.caller_location_ty())?;
        }
        Ok(())
    }

    fn drop_glue(&mut self, place: mir::Place<'tcx>) -> DropGlue {
        let tcx = self.tcx();
        let ty = self.monomorphize(place.ty(&self.mir().local_decls, tcx).ty);
        self.drop_glue_for_ty(ty)
    }

    fn drop_glue_for_ty(&mut self, ty: Ty<'tcx>) -> DropGlue {
        let tcx = self.tcx();
        let glue = Instance::resolve_drop_glue(tcx, ty);
        if let ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, None)) = glue.def {
            DropGlue::None
        } else if matches!(ty.kind(), ty::Dynamic(..)) {
            DropGlue::Dynamic
        } else {
            let item = self.direct_item(glue);
            DropGlue::Direct {
                item: item.item,
                link_name: item.link_name,
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Operands, places, constants
    // -----------------------------------------------------------------------------------------

    fn operand(&mut self, operand: &mir::Operand<'tcx>) -> Result<Operand, EncodeDecline> {
        let tcx = self.tcx();
        let ty = self.type_id(operand.ty(&self.mir().local_decls, tcx))?;
        let kind = match operand {
            mir::Operand::Copy(place) => OperandKind::Copy(self.place(*place)?),
            mir::Operand::Move(place) => OperandKind::Move(self.place(*place)?),
            mir::Operand::Constant(constant) => OperandKind::Constant(self.constant(constant)?),
            mir::Operand::RuntimeChecks(check) => OperandKind::RuntimeCheck(check.value(tcx.sess)),
        };
        Ok(Operand { ty, kind })
    }

    fn place(&mut self, place: mir::Place<'tcx>) -> Result<Place, EncodeDecline> {
        let tcx = self.tcx();
        let mut projection = Vec::with_capacity(place.projection.len());
        for element in place.projection {
            projection.push(match element {
                mir::PlaceElem::Deref => Projection::Deref,
                mir::PlaceElem::Field(field, ty) => Projection::Field {
                    field: field.as_usize() as u32,
                    ty: self.type_id(ty)?,
                },
                mir::PlaceElem::Index(local) => Projection::Index(LocalId(local.as_usize() as u32)),
                mir::PlaceElem::ConstantIndex {
                    offset,
                    min_length,
                    from_end,
                } => Projection::ConstantIndex {
                    offset,
                    min_length,
                    from_end,
                },
                mir::PlaceElem::Subslice { from, to, from_end } => {
                    Projection::Subslice { from, to, from_end }
                }
                mir::PlaceElem::Downcast(_, variant) => Projection::Downcast {
                    variant: variant.as_usize() as u32,
                },
                mir::PlaceElem::OpaqueCast(ty) => Projection::OpaqueCast(self.type_id(ty)?),
                mir::PlaceElem::UnwrapUnsafeBinder(ty) => {
                    Projection::UnwrapUnsafeBinder(self.type_id(ty)?)
                }
            });
        }
        Ok(Place {
            local: LocalId(place.local.as_usize() as u32),
            projection,
            ty: self.type_id(place.ty(&self.mir().local_decls, tcx).ty)?,
        })
    }

    fn constant(&mut self, constant: &mir::ConstOperand<'tcx>) -> Result<Constant, EncodeDecline> {
        let tcx = self.tcx();
        let konst = self.monomorphize(constant.const_);
        let ty = konst.ty();
        let value = konst
            .eval(tcx, typing_env(), constant.span)
            .map_err(|_| EncodeDecline::ConstantEvaluation { ty: type_name(ty) })?;
        match value {
            mir::ConstValue::ZeroSized => Ok(Constant::ZeroSized),
            mir::ConstValue::Scalar(Scalar::Int(value))
                if crate::constant::raw::is_primitive(ty)
                    || crate::constant::raw::is_pointer(ty) =>
            {
                Ok(Constant::Scalar {
                    bits: value.to_uint(value.size()),
                    bytes: value.size().bytes() as u8,
                })
            }
            // A non-primitive scalar constant (`Option<NonZero<T>>`, `Option<bool>`,
            // `Option<Ordering>`, `None::<&T>`, transparent wrappers, single-field structs,
            // single-scalar tuples/arrays/closures, fieldless enums) holds its memory layout in
            // one register. Decode it through a scratch allocation laid out for `ty`, matching
            // v1 `constant.rs::scalar_to_expr` and `ConstValue::Indirect` (MIRES-006).
            mir::ConstValue::Scalar(Scalar::Int(value)) => {
                let scratch = crate::constant::raw::scratch_alloc(tcx, value, ty)
                    .ok_or_else(|| EncodeDecline::ConstantNoLayout { ty: type_name(ty) })?;
                Ok(Constant::Memory(self.const_tree(
                    &scratch,
                    Size::ZERO,
                    ty,
                    0,
                )?))
            }
            // A pointer that is the one scalar of a type around it (`Option<&T>`,
            // `Option<NonNull<T>>`, `Option<fn()>`) decodes from a scratch allocation holding it,
            // provenance included, as that type stored in memory does.
            mir::ConstValue::Scalar(pointer @ Scalar::Ptr(..))
                if !crate::constant::raw::is_pointer(ty) =>
            {
                let scratch = crate::constant::raw::scratch_pointer_alloc(tcx, pointer, ty)
                    .ok_or_else(|| EncodeDecline::ConstantNoLayout { ty: type_name(ty) })?;
                Ok(Constant::Memory(self.const_tree(
                    &scratch,
                    Size::ZERO,
                    ty,
                    0,
                )?))
            }
            mir::ConstValue::Scalar(Scalar::Ptr(pointer, _)) => {
                let (provenance, offset) = pointer.prov_and_relative_offset();
                if let GlobalAlloc::Memory(allocation) = tcx.global_alloc(provenance.alloc_id()) {
                    let pointee = ty.builtin_deref(true).ok_or_else(|| {
                        EncodeDecline::PointerConstantNotReference { ty: type_name(ty) }
                    })?;
                    let allocation = allocation.inner();
                    check_const_memory(allocation)?;
                    return Ok(Constant::Memory(
                        self.const_memory_pointer(allocation, offset, pointee, None, 0)?,
                    ));
                }
                Ok(Constant::Pointer {
                    target: self.global_reference(provenance.alloc_id())?,
                    offset: offset.bytes(),
                })
            }
            mir::ConstValue::Slice { alloc_id, meta } if ty.peel_refs().is_str() => {
                let GlobalAlloc::Memory(allocation) = tcx.global_alloc(alloc_id) else {
                    return Err(EncodeDecline::StringConstantNoMemory);
                };
                let text = crate::constant::raw::read_str(allocation.inner(), Size::ZERO, meta)
                    .map_err(|_| EncodeDecline::StringConstantInvalid)?;
                Ok(Constant::String(self.types.str(text)))
            }
            mir::ConstValue::Slice { alloc_id, meta } => {
                let GlobalAlloc::Memory(allocation) = tcx.global_alloc(alloc_id) else {
                    return Err(EncodeDecline::SliceConstantNoMemory);
                };
                let allocation = allocation.inner();
                check_const_memory(allocation)?;
                let pointee = ty.peel_refs();
                // A transparent wrapper over the slice (`CStr` over `[c_char]`) is the slice it
                // wraps, at the wrapper's field offset.
                let (peeled, offset) = self.peel_transparent_at(pointee);
                if let ty::Slice(element) = peeled.kind() {
                    return Ok(Constant::Memory(ConstTree::Slice(
                        self.const_slice(allocation, offset, *element, meta, 0)?,
                    )));
                }
                if peeled.is_str() {
                    let text = crate::constant::raw::read_str(allocation, offset, meta)
                        .map_err(|_| EncodeDecline::StringConstantInvalid)?;
                    return Ok(Constant::Memory(ConstTree::Str(self.types.str(text))));
                }
                // `&CStr`-shaped: a struct whose unsized tail takes the length. v1 prints a
                // zombie for these; the tree spells the record with its tail decoded.
                if crate::value::unsized_tail_indexed(tcx, pointee).is_some() {
                    return Ok(Constant::Memory(ConstTree::Reference(Box::new(
                        self.const_tail_struct(allocation, Size::ZERO, pointee, meta, 0)?,
                    ))));
                }
                Err(EncodeDecline::SliceConstantNonSlicePointee { ty: type_name(ty) })
            }
            mir::ConstValue::Indirect { alloc_id, offset } => {
                let GlobalAlloc::Memory(allocation) = tcx.global_alloc(alloc_id) else {
                    return Err(EncodeDecline::IndirectConstantNoMemory);
                };
                let allocation = allocation.inner();
                check_const_memory(allocation)?;
                Ok(Constant::Memory(
                    self.const_tree(allocation, offset, ty, 0)?,
                ))
            }
        }
    }

    /// Decodes the value a pointer into anonymous immutable memory stands for.
    fn const_memory_pointer(
        &mut self,
        allocation: &mir::interpret::Allocation,
        offset: Size,
        pointee: Ty<'tcx>,
        meta: Option<u64>,
        depth: u32,
    ) -> Result<ConstTree, EncodeDecline> {
        if pointee.is_str() {
            let length = meta.ok_or(EncodeDecline::StrConstantNoLength)?;
            let text = crate::constant::raw::read_str(allocation, offset, length)
                .map_err(|_| EncodeDecline::StringConstantInvalid)?;
            return Ok(ConstTree::Str(self.types.str(text)));
        }
        if let ty::Slice(element) = pointee.kind() {
            let length = meta.ok_or_else(|| EncodeDecline::SliceConstantNoLength {
                pointee: type_name(pointee),
            })?;
            return Ok(ConstTree::Slice(
                self.const_slice(allocation, offset, *element, length, depth)?,
            ));
        }
        Ok(ConstTree::Reference(Box::new(
            self.const_tree(allocation, offset, pointee, depth)?,
        )))
    }

    /// `ty` with its transparent wrappers peeled, and the byte offset of the value that remains
    /// inside a value of `ty` (`value::peel_transparent`, keeping the offsets).
    fn peel_transparent_at(&self, mut ty: Ty<'tcx>) -> (Ty<'tcx>, Size) {
        let tcx = self.tcx();
        let mut offset = Size::ZERO;
        for _ in 0..16 {
            let Some((index, field_ty)) = crate::value::transparent_field_indexed(tcx, ty) else {
                break;
            };
            let Ok(layout) = tcx.layout_of(typing_env().as_query_input(ty)) else {
                break;
            };
            offset += layout.fields.offset(index.as_usize());
            ty = field_ty;
        }
        (crate::value::peel_pattern(ty), offset)
    }

    /// A struct with an unsized tail (`CStr`, `Path`'s inner) at `offset`: its sized fields
    /// decode as usual and the tail takes `length` from the wide pointer's metadata.
    fn const_tail_struct(
        &mut self,
        allocation: &mir::interpret::Allocation,
        offset: Size,
        ty: Ty<'tcx>,
        length: u64,
        depth: u32,
    ) -> Result<ConstTree, EncodeDecline> {
        if depth >= MAX_CONST_TREE_DEPTH {
            return Err(EncodeDecline::ConstantTooDeep);
        }
        let depth = depth + 1;
        let tcx = self.tcx();
        let layout = tcx
            .layout_of(typing_env().as_query_input(ty))
            .map_err(|_| EncodeDecline::ConstantNoLayout { ty: type_name(ty) })?;
        if let Some((index, field_ty)) = crate::value::transparent_field_indexed(tcx, ty) {
            let field_offset = offset + layout.fields.offset(index.as_usize());
            return self.const_tail_struct(allocation, field_offset, field_ty, length, depth);
        }
        let ty::Adt(def, args) = ty.kind() else {
            return Err(EncodeDecline::ConstantUnencodable { ty: type_name(ty) });
        };
        if !def.is_struct() {
            return Err(EncodeDecline::ConstantUnencodable { ty: type_name(ty) });
        }
        let variant = def.non_enum_variant();
        let last = variant.fields.len().checked_sub(1);
        let mut fields = Vec::with_capacity(variant.fields.len());
        for (index, field) in variant.fields.iter_enumerated() {
            let field_ty = tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args));
            let field_offset = offset + layout.fields.offset(index.as_usize());
            let tree = if Some(index.as_usize()) == last && !field_ty.is_sized(tcx, typing_env()) {
                match field_ty.kind() {
                    ty::Str => {
                        let text = crate::constant::raw::read_str(allocation, field_offset, length)
                            .map_err(|_| EncodeDecline::StringConstantInvalid)?;
                        ConstTree::Str(self.types.str(text))
                    }
                    ty::Slice(element) => ConstTree::Slice(self.const_slice(
                        allocation,
                        field_offset,
                        *element,
                        length,
                        depth,
                    )?),
                    _ => {
                        self.const_tail_struct(allocation, field_offset, field_ty, length, depth)?
                    }
                }
            } else {
                self.const_tree(allocation, field_offset, field_ty, depth)?
            };
            fields.push(tree);
        }
        Ok(ConstTree::Aggregate(fields))
    }

    fn const_slice(
        &mut self,
        allocation: &mir::interpret::Allocation,
        offset: Size,
        element: Ty<'tcx>,
        length: u64,
        depth: u32,
    ) -> Result<Vec<ConstTree>, EncodeDecline> {
        let tcx = self.tcx();
        if length > MAX_CONST_MEMORY_BYTES {
            return Err(EncodeDecline::SliceConstantTooLong);
        }
        let layout = tcx
            .layout_of(typing_env().as_query_input(element))
            .map_err(|_| EncodeDecline::SliceElementNoLayout {
                element: type_name(element),
            })?;
        let stride = layout.size.bytes();
        let length =
            usize::try_from(length).map_err(|_| EncodeDecline::SliceConstantLengthOverflow)?;
        if !crate::constant::raw::slice_fits(allocation, offset, stride, length) {
            return Err(EncodeDecline::SliceConstantOverrun {
                element: type_name(element),
            });
        }
        (0..length)
            .map(|index| {
                self.const_tree(
                    allocation,
                    offset + Size::from_bytes(stride * index as u64),
                    element,
                    depth,
                )
            })
            .collect()
    }

    /// Rebuilds the value of type `ty` stored at `offset` in constant memory (v1's
    /// `late_ir::Encoder::const_tree`, the owned mirror of the printed-JavaScript reader).
    fn const_tree(
        &mut self,
        allocation: &mir::interpret::Allocation,
        offset: Size,
        ty: Ty<'tcx>,
        depth: u32,
    ) -> Result<ConstTree, EncodeDecline> {
        if depth >= MAX_CONST_TREE_DEPTH {
            return Err(EncodeDecline::ConstantTooDeep);
        }
        let depth = depth + 1;
        let tcx = self.tcx();
        let layout = tcx
            .layout_of(typing_env().as_query_input(ty))
            .map_err(|_| EncodeDecline::ConstantNoLayout { ty: type_name(ty) })?;

        if let Some((index, field_ty)) = crate::value::transparent_field_indexed(tcx, ty) {
            return self.const_tree(
                allocation,
                offset + layout.fields.offset(index.as_usize()),
                field_ty,
                depth,
            );
        }

        match ty.kind() {
            ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Float(_) => {
                let int = crate::constant::raw::read_scalar(tcx, allocation, offset, layout.size)
                    .ok_or_else(|| EncodeDecline::ConstantUnreadable { ty: type_name(ty) })?;
                Ok(ConstTree::Scalar {
                    bits: int.to_uint(int.size()),
                    bytes: int.size().bytes() as u8,
                })
            }
            ty::Pat(base, _) => self.const_tree(allocation, offset, *base, depth),
            ty::Adt(def, args) if def.is_enum() => {
                let variant_index =
                    crate::constant::raw::variant_index(tcx, allocation, offset, layout, *def)
                        .map_err(|_| EncodeDecline::ConstantVariantUnreadable {
                            ty: type_name(ty),
                        })?;
                let variant = def.variant(variant_index);
                let variant_layout = layout.for_variant(
                    &rustc_middle::ty::layout::LayoutCx::new(tcx, typing_env()),
                    variant_index,
                );
                let fields = variant
                    .fields
                    .iter_enumerated()
                    .map(|(index, field)| {
                        let field_ty =
                            tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args));
                        self.const_tree(
                            allocation,
                            offset + variant_layout.fields.offset(index.as_usize()),
                            field_ty,
                            depth,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                Ok(ConstTree::Variant {
                    variant: variant_index.as_u32(),
                    fields,
                })
            }
            ty::Adt(def, args) if def.is_struct() => {
                let variant = def.non_enum_variant();
                let fields = variant
                    .fields
                    .iter_enumerated()
                    .map(|(index, field)| {
                        let field_ty =
                            tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args));
                        self.const_tree(
                            allocation,
                            offset + layout.fields.offset(index.as_usize()),
                            field_ty,
                            depth,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                Ok(ConstTree::Aggregate(fields))
            }
            ty::Tuple(elements) if elements.is_empty() => Ok(ConstTree::Unit),
            ty::Tuple(elements) => {
                let fields = elements
                    .iter()
                    .enumerate()
                    .map(|(index, element)| {
                        self.const_tree(
                            allocation,
                            offset + layout.fields.offset(index),
                            element,
                            depth,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                Ok(ConstTree::Aggregate(fields))
            }
            ty::Array(element, count) => {
                let count = count.try_to_target_usize(tcx).unwrap_or_default() as usize;
                if count as u64 > MAX_CONST_MEMORY_BYTES {
                    return Err(EncodeDecline::ArrayConstantTooLong);
                }
                let element_layout = tcx
                    .layout_of(typing_env().as_query_input(*element))
                    .map_err(|_| EncodeDecline::ConstantNoLayout {
                        ty: type_name(*element),
                    })?;
                let stride = element_layout.size.bytes();
                let fields = (0..count)
                    .map(|index| {
                        self.const_tree(
                            allocation,
                            offset + Size::from_bytes(stride * index as u64),
                            *element,
                            depth,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                Ok(ConstTree::Aggregate(fields))
            }
            ty::Closure(_, args) => {
                let fields = args
                    .as_closure()
                    .upvar_tys()
                    .iter()
                    .enumerate()
                    .map(|(index, upvar_ty)| {
                        self.const_tree(
                            allocation,
                            offset + layout.fields.offset(index),
                            upvar_ty,
                            depth,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                Ok(ConstTree::Aggregate(fields))
            }
            ty::Ref(..) | ty::RawPtr(..) | ty::FnPtr(..) => {
                self.const_pointer_field(allocation, offset, ty, depth)
            }
            ty::Adt(def, _) if def.is_union() && layout.is_zst() => Ok(ConstTree::Unit),
            ty::FnDef(..) => Ok(ConstTree::Unit),
            _ => Err(EncodeDecline::ConstantUnencodable { ty: type_name(ty) }),
        }
    }

    /// Decodes a pointer stored inside constant memory.
    fn const_pointer_field(
        &mut self,
        allocation: &mir::interpret::Allocation,
        offset: Size,
        ty: Ty<'tcx>,
        depth: u32,
    ) -> Result<ConstTree, EncodeDecline> {
        let tcx = self.tcx();
        let pointee = ty.builtin_deref(true);
        let wide =
            pointee.is_some_and(|pointee| crate::constant::raw::is_wide_pointee(tcx, pointee));
        let pointer = crate::constant::raw::read_pointer(tcx, allocation, offset, wide);
        if let Some(pointee) = pointee {
            let peeled_pointee =
                crate::value::peel_pattern(crate::value::peel_transparent(tcx, pointee));
            if matches!(peeled_pointee.kind(), ty::Dynamic(..)) {
                return self.const_dyn_pointer_field(&pointer, ty, depth);
            }
        }
        let Some(alloc_id) = pointer.alloc_id() else {
            if wide {
                if pointer.readable
                    && pointer.offset.bytes() != 0
                    && pointer.meta == Some(0)
                    && pointee.is_some_and(|p| {
                        matches!(
                            crate::value::peel_pattern(crate::value::peel_transparent(tcx, p))
                                .kind(),
                            ty::Str
                        )
                    })
                {
                    return Ok(ConstTree::Str(self.types.str("")));
                }
                return Err(EncodeDecline::ConstantAddressNoProvenance { ty: type_name(ty) });
            }
            if !pointer.readable {
                return Err(EncodeDecline::ConstantUninitialized { ty: type_name(ty) });
            }
            return Ok(ConstTree::Address(pointer.offset.bytes()));
        };
        match tcx.global_alloc(alloc_id) {
            GlobalAlloc::Memory(target) => {
                let pointee = pointee.ok_or_else(|| {
                    EncodeDecline::PointerConstantNotReference { ty: type_name(ty) }
                })?;
                let target = target.inner();
                check_const_memory(target)?;
                self.const_memory_pointer(target, pointer.offset, pointee, pointer.meta, depth)
            }
            GlobalAlloc::VTable(..) => Err(EncodeDecline::VtablePointerConstant),
            _ if wide => Err(EncodeDecline::WideGlobalPointerConstant { ty: type_name(ty) }),
            _ => Ok(ConstTree::Global {
                target: self.global_reference(alloc_id)?,
                offset: pointer.offset.bytes(),
            }),
        }
    }

    /// Decodes a fat trait-object pointer (`&dyn Trait` / `*const dyn Trait`) stored inside
    /// constant memory (`constant.rs::read_dyn_pointer`).
    fn const_dyn_pointer_field(
        &mut self,
        pointer: &crate::constant::raw::Pointer,
        ty: Ty<'tcx>,
        depth: u32,
    ) -> Result<ConstTree, EncodeDecline> {
        let tcx = self.tcx();
        let Some((concrete_ty, principal)) = crate::constant::raw::read_vtable(tcx, pointer) else {
            return Err(EncodeDecline::VtablePointerConstant);
        };
        let _ = crate::vtable::vtable_expr(self.cgu, concrete_ty, principal);
        let vtable_name = self.cgu.namer.vtable_name(concrete_ty, principal);
        let vtable = self.types.str(vtable_name.as_str());
        let concrete = self.type_id(concrete_ty)?;
        let data = match pointer.alloc_id() {
            None => {
                if !pointer.readable {
                    return Err(EncodeDecline::ConstantUninitialized { ty: type_name(ty) });
                }
                ConstTree::Address(pointer.offset.bytes())
            }
            Some(alloc_id) => match tcx.global_alloc(alloc_id) {
                GlobalAlloc::Memory(target) => {
                    let target = target.inner();
                    check_const_memory(target)?;
                    ConstTree::Reference(Box::new(self.const_tree(
                        target,
                        pointer.offset,
                        concrete_ty,
                        depth,
                    )?))
                }
                GlobalAlloc::Static(def_id) => {
                    if !crate::constant::raw::static_matches_pointee(
                        tcx,
                        def_id,
                        pointer.offset,
                        concrete_ty,
                    ) {
                        return Err(EncodeDecline::WideGlobalPointerConstant { ty: type_name(ty) });
                    }
                    ConstTree::Global {
                        target: self.global_reference(alloc_id)?,
                        offset: pointer.offset.bytes(),
                    }
                }
                _ => {
                    return Err(EncodeDecline::WideGlobalPointerConstant { ty: type_name(ty) });
                }
            },
        };
        Ok(ConstTree::Dyn {
            data: Box::new(data),
            concrete,
            vtable,
        })
    }

    fn global_reference(
        &mut self,
        allocation: mir::interpret::AllocId,
    ) -> Result<GlobalReference, EncodeDecline> {
        let tcx = self.tcx();
        Ok(match tcx.global_alloc(allocation) {
            GlobalAlloc::Function { instance } => {
                let item = self.direct_item(instance);
                GlobalReference::Function {
                    item: item.item,
                    link_name: item.link_name,
                }
            }
            GlobalAlloc::Static(definition) => {
                self.cgu.require_static(definition);
                GlobalReference::Static {
                    definition: definition_id(tcx, definition),
                    link_name: self
                        .types
                        .str(self.cgu.namer.static_name(definition).as_str()),
                }
            }
            GlobalAlloc::TypeId { ty } => GlobalReference::Type(self.type_id(ty)?),
            GlobalAlloc::Memory(_) => return Err(EncodeDecline::AnonymousPointerConstant),
            GlobalAlloc::VTable(..) => return Err(EncodeDecline::VtablePointerConstantYet),
        })
    }

    // -----------------------------------------------------------------------------------------
    // Statics and vtables
    // -----------------------------------------------------------------------------------------

    fn static_body(&mut self, def_id: DefId) -> Result<Body, EncodeDecline> {
        let tcx = self.tcx();
        self.item_path = Some(
            self.types
                .str(&with_no_trimmed_paths!(tcx.def_path_str(def_id))),
        );
        self.root = self.source_fact(tcx.def_span(def_id));
        let ty =
            tcx.normalize_erasing_regions(typing_env(), tcx.type_of(def_id).instantiate_identity());
        let type_id = self.type_id(ty)?;
        let init = self.static_initializer(def_id)?;
        Ok(Body {
            item: ItemId {
                definition: definition_id(tcx, def_id),
                instantiation: type_id.0,
            },
            kind: BodyKind::Static {
                init,
                mutable: tcx.is_mutable_static(def_id),
            },
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: Vec::new(),
                result: type_id,
                abi: AbiSignature::default(),
            },
            locals: Vec::new(),
            blocks: Vec::new(),
        })
    }

    /// The evaluated initializer of a `static`, decoded at the static's type.
    ///
    /// The static's own allocation may be mutable (a `static mut`, interior mutability); that is
    /// the value it starts with, not memory the program cannot write, so only the size limit
    /// applies to it. Memory it points into is checked as any constant memory is.
    fn static_initializer(&mut self, def_id: DefId) -> Result<ConstTree, EncodeDecline> {
        let tcx = self.tcx();
        let ty =
            tcx.normalize_erasing_regions(typing_env(), tcx.type_of(def_id).instantiate_identity());
        let allocation = tcx
            .eval_static_initializer(def_id)
            .map_err(|_| EncodeDecline::ConstantEvaluation { ty: type_name(ty) })?;
        let allocation = allocation.inner();
        if allocation.len() as u64 > MAX_CONST_MEMORY_BYTES {
            return Err(EncodeDecline::ConstantMemoryTooLarge);
        }
        self.const_tree(allocation, Size::ZERO, ty, 0)
    }

    fn vtable_body(
        &mut self,
        ty: Ty<'tcx>,
        trait_ref: Option<ExistentialTraitRef<'tcx>>,
    ) -> Result<Body, EncodeDecline> {
        let tcx = self.tcx();
        self.item_path = Some(self.types.str(&with_no_trimmed_paths!(match trait_ref {
            Some(trait_ref) => format!("<{ty} as {trait_ref}> vtable"),
            None => format!("<{ty}> vtable"),
        })));
        self.root =
            trait_ref.and_then(|trait_ref| self.source_fact(tcx.def_span(trait_ref.def_id)));
        let concrete = self.type_id(ty)?;
        let entries: &[VtblEntry<'tcx>] = match trait_ref {
            Some(trait_ref) => {
                let trait_ref = trait_ref.with_self_ty(tcx, ty);
                tcx.vtable_entries(tcx.erase_and_anonymize_regions(trait_ref))
            }
            None => TyCtxt::COMMON_VTABLE_ENTRIES,
        };
        let (size, align) = match tcx.layout_of(typing_env().as_query_input(ty)) {
            Ok(layout) => (layout.size.bytes(), layout.align.abi.bytes()),
            Err(_) => (0, 1),
        };
        let mut encoded = Vec::with_capacity(entries.len());
        for entry in entries {
            encoded.push(match *entry {
                VtblEntry::MetadataDropInPlace => {
                    if !ty.needs_drop(tcx, typing_env()) {
                        VtableEntry::Drop(None)
                    } else {
                        let glue = Instance::resolve_drop_glue(tcx, ty);
                        match glue.def {
                            ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, None)) => {
                                VtableEntry::Drop(None)
                            }
                            _ => VtableEntry::Drop(Some(self.direct_item(glue))),
                        }
                    }
                }
                VtblEntry::MetadataSize => VtableEntry::Size(size),
                VtblEntry::MetadataAlign => VtableEntry::Align(align),
                VtblEntry::Vacant => VtableEntry::Vacant,
                VtblEntry::Method(instance) => VtableEntry::Method(self.direct_item(instance)),
                VtblEntry::TraitVPtr(super_trait_ref) => {
                    let principal = ty::ExistentialTraitRef::erase_self_ty(tcx, super_trait_ref);
                    let name = self.cgu.namer.vtable_name(ty, Some(principal));
                    VtableEntry::TraitVPtr(self.types.str(name.as_str()))
                }
            });
        }
        Ok(Body {
            item: ItemId {
                definition: trait_ref
                    .map(|trait_ref| definition_id(tcx, trait_ref.def_id))
                    .unwrap_or(DefinitionId {
                        crate_id: 0,
                        local_hash: 0,
                    }),
                instantiation: concrete.0,
            },
            kind: BodyKind::Vtable {
                concrete,
                trait_ref: trait_ref.map(|trait_ref| definition_id(tcx, trait_ref.def_id)),
                entries: encoded,
            },
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: Vec::new(),
                result: concrete,
                abi: AbiSignature::default(),
            },
            locals: Vec::new(),
            blocks: Vec::new(),
        })
    }

    // -----------------------------------------------------------------------------------------
    // Origins and shared helpers
    // -----------------------------------------------------------------------------------------

    fn source_fact(&mut self, span: Span) -> Option<SourceFactId> {
        // Match rustc's debug locations: collapse only opted-in macro expansions,
        // and stop at the containing function rather than its generating macro.
        let span = self.function.map_or(span, |(_, mir)| {
            rustc_span::hygiene::walk_chain_collapsed(span, mir.span)
        });
        let location = crate::names::location_of(self.tcx(), span)?;
        let file = self.types.str(&location.file);
        Some(self.types.interns.intern_source_fact(SourceFact {
            file,
            line: location.line,
            col: location.col,
        }))
    }

    /// The origin of one element: its kind, its span's fact, and the inline stack of its scope.
    fn origin(
        &mut self,
        kind: OriginKind,
        info: mir::SourceInfo,
        local_name: Option<StrId>,
    ) -> OriginId {
        let source = self.source_fact(info.span);
        let inline_stack = self.scope_stack(info.scope);
        self.intern_origin(Origin {
            kind,
            source,
            local_name,
            inline_stack,
        })
    }

    fn intern_origin(&mut self, origin: Origin) -> OriginId {
        let key = OriginKey {
            kind: match origin.kind {
                OriginKind::Statement => 0,
                OriginKind::Terminator => 1,
                OriginKind::Local => 2,
                OriginKind::Argument => 3,
                OriginKind::Constant => 4,
                OriginKind::Generated(_) => 5,
            },
            generated: match origin.kind {
                OriginKind::Generated(what) => Some(what),
                _ => None,
            },
            source: origin.source,
            local_name: origin.local_name,
            inline_stack: origin.inline_stack.clone(),
        };
        if let Some(id) = self.origin_index.get(&key) {
            return *id;
        }
        let id = OriginId(self.origins.len() as u32);
        self.origins.push(origin);
        self.origin_index.insert(key, id);
        id
    }

    /// The chain of inlined call sites that placed `scope`'s elements here, innermost first.
    fn scope_stack(&mut self, scope: mir::SourceScope) -> Vec<OriginId> {
        if let Some(stack) = self.scope_stacks.get(&scope.as_u32()) {
            return stack.clone();
        }
        let tcx = self.tcx();
        let mir = self.mir();
        let mut stack = Vec::new();
        let mut current = scope;
        for _ in 0..mir.source_scopes.len() {
            let data = &mir.source_scopes[current];
            let next = match data.inlined {
                Some((instance, span)) => {
                    let what = self.types.str(&with_no_trimmed_paths!(format!(
                        "inlined {}",
                        tcx.def_path_str(instance.def_id())
                    )));
                    let source = self.source_fact(span);
                    let origin = self.intern_origin(Origin {
                        kind: OriginKind::Generated(what),
                        source,
                        local_name: None,
                        inline_stack: Vec::new(),
                    });
                    stack.push(origin);
                    data.inlined_parent_scope
                }
                None => data.parent_scope,
            };
            match next {
                Some(next) => current = next,
                None => break,
            }
        }
        self.scope_stacks.insert(scope.as_u32(), stack.clone());
        stack
    }

    fn type_id(&mut self, ty: Ty<'tcx>) -> Result<TypeId, EncodeDecline> {
        let ty = self.monomorphize(ty);
        self.types.type_id(ty)
    }

    /// Instantiates and normalizes `value` for the function being encoded. A static or vtable
    /// encoder has no generic context: its types are normalized only.
    fn monomorphize<T>(&self, value: T) -> T
    where
        T: TypeFoldable<TyCtxt<'tcx>> + Copy,
    {
        let Some((instance, _)) = self.function else {
            return self
                .tcx()
                .normalize_erasing_regions(typing_env(), ty::Unnormalized::new_wip(value));
        };
        instance.instantiate_mir_and_normalize_erasing_regions(
            self.tcx(),
            typing_env(),
            ty::EarlyBinder::bind(self.tcx(), value),
        )
    }
}

/// The user's name for each local (`names.rs`'s rule: the first `var_debug_info` entry naming the
/// whole local, never the return place, never `_`), or `None` for a compiler temporary.
fn local_names(mir: &mir::Body<'_>) -> Vec<Option<String>> {
    let mut names: Vec<Option<String>> = vec![None; mir.local_decls.len()];
    for info in &mir.var_debug_info {
        let mir::VarDebugInfoContents::Place(place) = info.value else {
            continue;
        };
        if !place.projection.is_empty() {
            continue;
        }
        let index = place.local.as_usize();
        if place.local == mir::RETURN_PLACE || index >= names.len() {
            continue;
        }
        if names[index].is_none() {
            let raw = info.name.as_str();
            if !raw.is_empty() && raw != "_" {
                names[index] = Some(raw.to_owned());
            }
        }
    }
    names
}

/// The `view-abi` marker `def_id` is, by the `link_section` it carries (`abi.rs`).
fn view_abi_marker(tcx: TyCtxt<'_>, def_id: DefId) -> Option<String> {
    let section = tcx.codegen_fn_attrs(def_id).link_section?;
    section
        .as_str()
        .strip_prefix(VIEW_ABI_PREFIX)
        .map(str::to_owned)
}

fn check_const_memory(allocation: &mir::interpret::Allocation) -> Result<(), EncodeDecline> {
    if allocation.mutability != rustc_middle::mir::Mutability::Not {
        return Err(EncodeDecline::MutableConstantMemory);
    }
    if allocation.len() as u64 > MAX_CONST_MEMORY_BYTES {
        return Err(EncodeDecline::ConstantMemoryTooLarge);
    }
    Ok(())
}

/// Postcard round trip of a body and its origins: the debug-only self check.
fn round_trip(body: &Body, origins: &OriginTable) -> Result<(), String> {
    let bytes = postcard::to_allocvec(body).map_err(|error| error.to_string())?;
    let decoded: Body = postcard::from_bytes(&bytes).map_err(|error| error.to_string())?;
    if decoded != *body {
        return Err("late-IR body does not survive a postcard round trip".to_owned());
    }
    let bytes = postcard::to_allocvec(origins).map_err(|error| error.to_string())?;
    let decoded: OriginTable = postcard::from_bytes(&bytes).map_err(|error| error.to_string())?;
    if decoded != *origins {
        return Err("late-IR origin table does not survive a postcard round trip".to_owned());
    }
    Ok(())
}

/// The operand `local`'s one assignment reads, when that assignment is a plain move or copy
/// (`abi.rs::assigned_operand`).
fn assigned_operand<'tcx>(mir: &mir::Body<'tcx>, local: mir::Local) -> Option<mir::Operand<'tcx>> {
    for block in mir.basic_blocks.iter() {
        for statement in &block.statements {
            let mir::StatementKind::Assign(assignment) = &statement.kind else {
                continue;
            };
            let (place, rvalue) = &**assignment;
            if place.as_local() != Some(local) {
                continue;
            }
            let mir::Rvalue::Use(operand, _) = rvalue else {
                return None;
            };
            return Some(operand.clone());
        }
    }
    None
}

pub(crate) fn item_id<'tcx>(cgu: &CguCx<'tcx>, instance: Instance<'tcx>) -> ItemId {
    let tcx = cgu.tcx;
    ItemId {
        definition: definition_id(tcx, instance.def_id()),
        instantiation: tcx.type_id_hash(instance.ty(tcx, typing_env())).as_u128(),
    }
}

/// The `str` call-site identity of a resolved method instance (`StandardOperation::is_str`), for
/// the link step's `js-str=bytes` lowering (`jsc-link/src/lower/bstr.rs`).
///
/// Identity is the rustc definition, never a name alone: an inherent method of `impl str`, or
/// the `Index`/`PartialEq`/`Ord` impl whose self type is `str`, with the exact argument types the
/// lowering spells (a `&str` receiver; a `&str` or `char` pattern; a `usize` range of the four
/// `core::ops` shapes). Anything else, a user method of the same name included, is untagged.
/// One method of `impl [u8]` joins them, `is_ascii` (`&[u8] -> bool`), which `str::is_ascii` is
/// written with: an inherent impl of a primitive type is one only `core` can write.
fn str_operation<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
) -> Option<StandardOperation> {
    let associated = tcx.opt_associated_item(definition)?;
    let trait_id = match associated.container {
        ty::AssocContainer::InherentImpl => None,
        ty::AssocContainer::TraitImpl(Ok(trait_item)) => Some(tcx.parent(trait_item)),
        _ => return None,
    };
    let owner = tcx.normalize_erasing_regions(
        typing_env(),
        tcx.type_of(tcx.parent(definition))
            .instantiate(tcx, arguments),
    );
    let signature = tcx
        .fn_sig(definition)
        .instantiate(tcx, arguments)
        .skip_binder();
    let symbol = associated.name();
    let name = symbol.as_str();
    let byte_slice = |ty: Ty<'tcx>| matches!(ty.kind(), ty::Slice(element) if matches!(element.kind(), ty::Uint(ty::UintTy::U8)));
    if byte_slice(owner) {
        let byte_slice_ref = |ty: Ty<'tcx>| matches!(ty.kind(), ty::Ref(_, pointee, rustc_hir::Mutability::Not) if byte_slice(*pointee));
        let exact = trait_id.is_none()
            && name == "is_ascii"
            && matches!(signature.inputs(), [receiver] if byte_slice_ref(*receiver))
            && signature.output().is_bool();
        return exact.then_some(StandardOperation::SliceIsAscii);
    }
    if !owner.is_str() {
        return None;
    }
    let str_ref = |ty: Ty<'tcx>| matches!(ty.kind(), ty::Ref(_, pointee, rustc_hir::Mutability::Not) if pointee.is_str());
    let [receiver, rest @ ..] = signature.inputs() else {
        return None;
    };
    if !str_ref(*receiver) {
        return None;
    }
    let lang = tcx.lang_items();
    let is_str_range_index = |index: Ty<'tcx>| {
        let ty::Adt(range, range_args) = index.kind() else {
            return false;
        };
        let usize_or_none = range_args
            .types()
            .all(|ty| matches!(ty.kind(), ty::Uint(ty::UintTy::Usize)));
        let shapes = [
            lang.range_struct(),
            lang.range_from_struct(),
            lang.range_to_struct(),
            lang.range_full_struct(),
        ];
        usize_or_none && shapes.contains(&Some(range.did()))
    };
    match (trait_id, name, rest) {
        (None, "len", []) => Some(StandardOperation::StrLen),
        (None, "is_char_boundary", [index])
            if matches!(index.kind(), ty::Uint(ty::UintTy::Usize)) =>
        {
            Some(StandardOperation::StrIsCharBoundary)
        }
        (None, "trim", []) => Some(StandardOperation::StrTrim),
        (None, "trim_start", []) => Some(StandardOperation::StrTrimStart),
        (None, "trim_end", []) => Some(StandardOperation::StrTrimEnd),
        (None, "is_ascii", []) => Some(StandardOperation::StrIsAscii),
        (
            None,
            "starts_with" | "ends_with" | "find" | "rfind" | "contains" | "strip_prefix"
            | "strip_suffix",
            [pattern],
        ) if str_ref(*pattern) || matches!(pattern.kind(), ty::Char) => Some(match name {
            "starts_with" => StandardOperation::StrStartsWith,
            "ends_with" => StandardOperation::StrEndsWith,
            "find" => StandardOperation::StrFind,
            "rfind" => StandardOperation::StrRFind,
            "contains" => StandardOperation::StrContains,
            "strip_prefix" => StandardOperation::StrStripPrefix,
            _ => StandardOperation::StrStripSuffix,
        }),
        (None, "get", [index]) if is_str_range_index(*index) => Some(StandardOperation::StrGet),
        (Some(trait_id), "index", [index])
            if lang.index_trait() == Some(trait_id) && is_str_range_index(*index) =>
        {
            Some(StandardOperation::StrIndex)
        }
        (Some(trait_id), "eq", [other]) if lang.eq_trait() == Some(trait_id) && str_ref(*other) => {
            Some(StandardOperation::StrEq)
        }
        (Some(trait_id), "cmp", [other])
            if tcx.is_diagnostic_item(rustc_span::sym::Ord, trait_id) && str_ref(*other) =>
        {
            Some(StandardOperation::StrCmp)
        }
        (Some(trait_id), "partial_cmp", [other])
            if lang.partial_ord_trait() == Some(trait_id) && str_ref(*other) =>
        {
            Some(StandardOperation::StrPartialCmp)
        }
        _ => None,
    }
}

fn call_semantic_for_operation(operation: StandardOperation) -> CallSemantic {
    // Proven format sinks are inlined by the language kernel; keeping those calls Ordinary lets a
    // user function join a native String region without staging `format` itself (v1 rule).
    match operation {
        StandardOperation::FormatArgsToString
        | StandardOperation::FormatArgsNew
        | StandardOperation::FormatArgumentNewDisplay => CallSemantic::Ordinary,
        other => CallSemantic::Standard(other),
    }
}

/// v1's operation enum, variant for variant (both are append-only in the same order).
fn standard_operation_v2(operation: crate::late_ir::StandardOperation) -> StandardOperation {
    use crate::late_ir::StandardOperation as V1;
    match operation {
        V1::VecNew => StandardOperation::VecNew,
        V1::VecWithCapacity => StandardOperation::VecWithCapacity,
        V1::VecLen => StandardOperation::VecLen,
        V1::VecCapacity => StandardOperation::VecCapacity,
        V1::VecPush => StandardOperation::VecPush,
        V1::VecClear => StandardOperation::VecClear,
        V1::VecReserve => StandardOperation::VecReserve,
        V1::VecAsSlice => StandardOperation::VecAsSlice,
        V1::VecAsMutSlice => StandardOperation::VecAsMutSlice,
        V1::VecPop => StandardOperation::VecPop,
        V1::VecIsEmpty => StandardOperation::VecIsEmpty,
        V1::VecExtend => StandardOperation::VecExtend,
        V1::VecRemove => StandardOperation::VecRemove,
        V1::VecSwap => StandardOperation::VecSwap,
        V1::VecIndex => StandardOperation::VecIndex,
        V1::VecIndexMut => StandardOperation::VecIndexMut,
        V1::VecDrop => StandardOperation::VecDrop,
        V1::StringNew => StandardOperation::StringNew,
        V1::StringWithCapacity => StandardOperation::StringWithCapacity,
        V1::StringLen => StandardOperation::StringLen,
        V1::StringCapacity => StandardOperation::StringCapacity,
        V1::StringPush => StandardOperation::StringPush,
        V1::StringPushStr => StandardOperation::StringPushStr,
        V1::StringClear => StandardOperation::StringClear,
        V1::StringAsStr => StandardOperation::StringAsStr,
        V1::StringDrop => StandardOperation::StringDrop,
        V1::SliceSwap => StandardOperation::SliceSwap,
        V1::StringFmtWriteStr => StandardOperation::StringFmtWriteStr,
        V1::StringFmtWriteChar => StandardOperation::StringFmtWriteChar,
        V1::StringFmtWriteFmt => StandardOperation::StringFmtWriteFmt,
        V1::StringFmtSpecWriteFmt => StandardOperation::StringFmtSpecWriteFmt,
        V1::F64SpecToString => StandardOperation::F64SpecToString,
        V1::IntToString => StandardOperation::IntToString,
        V1::BoolToString => StandardOperation::BoolToString,
        V1::CharToString => StandardOperation::CharToString,
        V1::FormatArgumentNewDisplay => StandardOperation::FormatArgumentNewDisplay,
        V1::FormatArgsNew => StandardOperation::FormatArgsNew,
        V1::FormatArgsToString => StandardOperation::FormatArgsToString,
    }
}

fn block_id(block: mir::BasicBlock) -> BlockId {
    BlockId(block.as_usize() as u32)
}

fn borrow_kind(kind: mir::BorrowKind) -> BorrowKind {
    match kind {
        mir::BorrowKind::Shared => BorrowKind::Shared,
        mir::BorrowKind::Fake(mir::FakeBorrowKind::Shallow) => BorrowKind::FakeShallow,
        mir::BorrowKind::Fake(mir::FakeBorrowKind::Deep) => BorrowKind::FakeDeep,
        mir::BorrowKind::Mut {
            kind: mir::MutBorrowKind::Default,
        } => BorrowKind::Mutable,
        mir::BorrowKind::Mut {
            kind: mir::MutBorrowKind::TwoPhaseBorrow,
        } => BorrowKind::TwoPhase,
        mir::BorrowKind::Mut {
            kind: mir::MutBorrowKind::ClosureCapture,
        } => BorrowKind::ClosureCapture,
    }
}

fn raw_pointer_kind(kind: mir::RawPtrKind) -> RawPointerKind {
    match kind {
        mir::RawPtrKind::Const => RawPointerKind::Const,
        mir::RawPtrKind::Mut => RawPointerKind::Mutable,
        mir::RawPtrKind::FakeForPtrMetadata => RawPointerKind::FakeForMetadata,
    }
}

fn cast_kind(kind: mir::CastKind) -> CastKind {
    match kind {
        mir::CastKind::PointerExposeProvenance => CastKind::PointerExposeProvenance,
        mir::CastKind::PointerWithExposedProvenance => CastKind::PointerWithExposedProvenance,
        mir::CastKind::PointerCoercion(..) => CastKind::PointerCoercion,
        mir::CastKind::IntToInt => CastKind::IntToInt,
        mir::CastKind::FloatToInt => CastKind::FloatToInt,
        mir::CastKind::FloatToFloat => CastKind::FloatToFloat,
        mir::CastKind::IntToFloat => CastKind::IntToFloat,
        mir::CastKind::PtrToPtr => CastKind::PtrToPtr,
        mir::CastKind::FnPtrToPtr => CastKind::FnPtrToPtr,
        mir::CastKind::Transmute => CastKind::Transmute,
        mir::CastKind::Subtype => CastKind::Subtype,
    }
}

fn unary_op(op: mir::UnOp) -> UnaryOp {
    match op {
        mir::UnOp::Not => UnaryOp::Not,
        mir::UnOp::Neg => UnaryOp::Neg,
        mir::UnOp::PtrMetadata => UnaryOp::PtrMetadata,
    }
}

fn binary_op(op: mir::BinOp) -> BinaryOp {
    match op {
        mir::BinOp::Add => BinaryOp::Add,
        mir::BinOp::AddUnchecked => BinaryOp::AddUnchecked,
        mir::BinOp::AddWithOverflow => BinaryOp::AddWithOverflow,
        mir::BinOp::Sub => BinaryOp::Sub,
        mir::BinOp::SubUnchecked => BinaryOp::SubUnchecked,
        mir::BinOp::SubWithOverflow => BinaryOp::SubWithOverflow,
        mir::BinOp::Mul => BinaryOp::Mul,
        mir::BinOp::MulUnchecked => BinaryOp::MulUnchecked,
        mir::BinOp::MulWithOverflow => BinaryOp::MulWithOverflow,
        mir::BinOp::Div => BinaryOp::Div,
        mir::BinOp::Rem => BinaryOp::Rem,
        mir::BinOp::BitXor => BinaryOp::BitXor,
        mir::BinOp::BitAnd => BinaryOp::BitAnd,
        mir::BinOp::BitOr => BinaryOp::BitOr,
        mir::BinOp::Shl => BinaryOp::Shl,
        mir::BinOp::ShlUnchecked => BinaryOp::ShlUnchecked,
        mir::BinOp::Shr => BinaryOp::Shr,
        mir::BinOp::ShrUnchecked => BinaryOp::ShrUnchecked,
        mir::BinOp::Eq => BinaryOp::Eq,
        mir::BinOp::Lt => BinaryOp::Lt,
        mir::BinOp::Le => BinaryOp::Le,
        mir::BinOp::Ne => BinaryOp::Ne,
        mir::BinOp::Ge => BinaryOp::Ge,
        mir::BinOp::Gt => BinaryOp::Gt,
        mir::BinOp::Cmp => BinaryOp::Cmp,
        mir::BinOp::Offset => BinaryOp::Offset,
    }
}

fn coroutine_kind(kind: rustc_hir::CoroutineKind) -> CoroutineKind {
    match kind {
        rustc_hir::CoroutineKind::Desugared(kind, source) => CoroutineKind::Desugared {
            kind: match kind {
                rustc_hir::CoroutineDesugaring::Async => CoroutineDesugaring::Async,
                rustc_hir::CoroutineDesugaring::Gen => CoroutineDesugaring::Gen,
                rustc_hir::CoroutineDesugaring::AsyncGen => CoroutineDesugaring::AsyncGen,
            },
            source: match source {
                rustc_hir::CoroutineSource::Block => CoroutineSource::Block,
                rustc_hir::CoroutineSource::Closure => CoroutineSource::Closure,
                rustc_hir::CoroutineSource::Fn => CoroutineSource::Function,
            },
        },
        rustc_hir::CoroutineKind::Coroutine(movability) => CoroutineKind::Coroutine {
            movable: matches!(movability, rustc_hir::Movability::Movable),
        },
    }
}

fn unwind_action(action: mir::UnwindAction) -> Unwind {
    match action {
        mir::UnwindAction::Continue => Unwind::Continue,
        mir::UnwindAction::Unreachable => Unwind::Unreachable,
        mir::UnwindAction::Terminate(reason) => Unwind::Terminate(terminate_reason(reason)),
        mir::UnwindAction::Cleanup(block) => Unwind::Cleanup(block_id(block)),
    }
}

fn terminate_reason(reason: mir::UnwindTerminateReason) -> TerminateReason {
    match reason {
        mir::UnwindTerminateReason::Abi => TerminateReason::Abi,
        mir::UnwindTerminateReason::InCleanup => TerminateReason::InCleanup,
    }
}
