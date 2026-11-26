//! Function-level MIR -> JIR-S encoding: bodies, blocks, statements, rvalues, terminators,
//! operands, places, and pure MIR-to-JIR enum conversions.

use rustc_hir::LangItem;
use rustc_middle::mir;
use rustc_middle::ty::adjustment::PointerCoercion;
use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_middle::ty::{self, Instance, Ty};
use rustc_span::Symbol;

use rcgjs_object::jir::decline::EncodeDecline;
use rcgjs_object::jir::origin::OriginKind;
use rcgjs_object::jir::semantic::{
    AbiSignature, AggregateKind, AssertMessage, BasicBlock, BinaryOp, BlockId, Body, BodyKind,
    BodySemantic, BorrowKind, CastKind, CoroutineDesugaring, CoroutineKind, CoroutineSource, Local,
    LocalId, LocalRole, Operand, OperandKind, Place, Projection, RawPointerKind, Rvalue, Signature,
    SpreadArgument, Statement, StatementKind, TerminateReason, Terminator, UnaryOp, Unwind,
};

use super::call::MarkerSection;
use super::origins::{local_names, resolve_caller_location_span};
use super::{item_id, FnEncoder};
use crate::jir::types::definition_id;
use crate::value::typing_env;

impl<'a, 'tcx> FnEncoder<'a, 'tcx> {
    pub(super) fn function_body(&mut self) -> Result<Body, EncodeDecline> {
        let tcx = self.tcx();
        let mir = self.mir;
        let path = self.core.types.str(&with_no_trimmed_paths!(
            tcx.def_path_str(self.instance.def_id())
        ));
        self.core.origins.set_item_path(path);
        let root = self
            .source_fact(mir.span)
            .or_else(|| self.source_fact(tcx.def_span(self.instance.def_id())));
        self.core.origins.set_root(root);

        let link_section = tcx.codegen_fn_attrs(self.instance.def_id()).link_section;
        let host_body = match MarkerSection::classify(link_section.as_ref().map(Symbol::as_str)) {
            MarkerSection::Extern(descriptor) => Some(descriptor),
            _ => None,
        };
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
                .map(|name| self.core.types.str(name));
            let origin = self.origin(
                if role == LocalRole::Argument {
                    OriginKind::Argument
                } else {
                    OriginKind::Local
                },
                declaration.source_info,
                name,
            );
            self.core.origins.push_local(origin);
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
            self.host_function_body(descriptor)?
        } else {
            let mut blocks = Vec::with_capacity(mir.basic_blocks.len());
            for (block, data) in mir.basic_blocks.iter_enumerated() {
                blocks.push(self.basic_block(block, data)?);
            }
            blocks
        };
        Ok(Body {
            item: item_id(self.core.cgu, self.instance),
            kind: BodyKind::Function,
            semantic,
            signature,
            locals,
            blocks,
        })
    }

    fn abi_signature(&mut self) -> Result<AbiSignature, EncodeDecline> {
        let mir = self.mir;
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
        let caller_location = self.instance.def.requires_caller_location(self.tcx());
        let (inline_never, inline_hint, inline_always) = match self.instance.def {
            ty::InstanceKind::Item(def_id) => {
                let is_panic = self.tcx().lang_items().panic_impl() == Some(def_id);
                let is_fn = matches!(
                    self.tcx().def_kind(def_id),
                    rustc_hir::def::DefKind::Fn | rustc_hir::def::DefKind::AssocFn
                );
                let attr = is_fn.then(|| self.tcx().codegen_fn_attrs(def_id).inline);
                let never = is_panic || matches!(attr, Some(rustc_hir::attrs::InlineAttr::Never));
                let hint = !never && matches!(attr, Some(rustc_hir::attrs::InlineAttr::Hint));
                let always = !never
                    && matches!(
                        attr,
                        Some(
                            rustc_hir::attrs::InlineAttr::Always
                                | rustc_hir::attrs::InlineAttr::Force { .. }
                        )
                    );
                (never, hint, always)
            }
            _ => (false, false, false),
        };
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
            inline_hint,
            inline_always,
        })
    }

    fn body_semantic(&mut self) -> Result<BodySemantic, EncodeDecline> {
        if let ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, Some(ty))) = self.instance.def {
            return Ok(BodySemantic::DropGlue(self.type_id(ty)?));
        }
        if matches!(
            self.instance.def,
            ty::InstanceKind::Item(def_id)
                if self.tcx().lang_items().panic_impl() == Some(def_id)
        ) {
            return Ok(BodySemantic::PanicImpl);
        }
        Ok(crate::jir::classify::standard_operation(
            self.tcx(),
            self.instance.def_id(),
            self.instance.args,
        )
        .map_or(BodySemantic::Ordinary, BodySemantic::Standard))
    }

    fn basic_block(
        &mut self,
        block: mir::BasicBlock,
        data: &mir::BasicBlockData<'tcx>,
    ) -> Result<BasicBlock, EncodeDecline> {
        let id = block_id(block);
        let mut statements = Vec::with_capacity(data.statements.len());
        for (index, statement) in data.statements.iter().enumerate() {
            let origin = self.origin(OriginKind::Statement, statement.source_info, None);
            self.core.origins.push_statement(origin);
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
        self.core.origins.push_terminator(origin);
        let terminator = self
            .terminator(terminator.source_info, &terminator.kind)
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
                    self.core.types.str("copy_nonoverlapping");
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
            mir::Rvalue::Cast(kind, value, result) => {
                self.record_cast_refs(*kind, value, *result);
                if matches!(kind, mir::CastKind::PtrToPtr | mir::CastKind::Transmute) {
                    let from_ty = crate::value::peel_pattern(crate::value::peel_transparent(
                        tcx,
                        self.monomorphize(value.ty(&self.mir.local_decls, tcx)),
                    ));
                    let to_ty = crate::value::peel_pattern(crate::value::peel_transparent(
                        tcx,
                        self.monomorphize(*result),
                    ));
                    if let (Some(fp), Some(tp)) =
                        (from_ty.builtin_deref(true), to_ty.builtin_deref(true))
                    {
                        if fp != tp {
                            self.core.types.record_type_name(fp)?;
                            self.core.types.record_type_name(tp)?;
                        }
                    }
                }
                Rvalue::Cast {
                    kind: cast_kind(*kind),
                    value: self.operand(value)?,
                    result: self.type_id(*result)?,
                }
            }
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
                result: self.type_id(rvalue.ty(&self.mir.local_decls, tcx))?,
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
        source_info: mir::SourceInfo,
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
                let call_source_info = mir::SourceInfo {
                    span: *fn_span,
                    scope: source_info.scope,
                };
                Terminator::Call(self.call(
                    func,
                    args,
                    *destination,
                    *target,
                    *unwind,
                    call_source_info,
                )?)
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
                self.record_assert_panic(msg);
                let track_caller =
                    resolve_caller_location_span(self.tcx(), self.mir, self.instance, source_info)
                        .map(|span| self.caller_location_fact(span))
                        .transpose()?;
                Terminator::Assert {
                    condition: self.operand(cond)?,
                    expected: *expected,
                    message: self.assert_message(msg)?,
                    target: block_id(*target),
                    unwind: unwind_action(*unwind),
                    track_caller,
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

    pub(super) fn operand(
        &mut self,
        operand: &mir::Operand<'tcx>,
    ) -> Result<Operand, EncodeDecline> {
        let tcx = self.tcx();
        let ty = self.type_id(operand.ty(&self.mir.local_decls, tcx))?;
        let kind = match operand {
            mir::Operand::Copy(place) => OperandKind::Copy(self.place(*place)?),
            mir::Operand::Move(place) => OperandKind::Move(self.place(*place)?),
            mir::Operand::Constant(constant) => OperandKind::Constant(self.constant(constant)?),
            mir::Operand::RuntimeChecks(check) => OperandKind::RuntimeCheck(check.value(tcx.sess)),
        };
        Ok(Operand { ty, kind })
    }

    pub(super) fn place(&mut self, place: mir::Place<'tcx>) -> Result<Place, EncodeDecline> {
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
            ty: self.type_id(place.ty(&self.mir.local_decls, tcx).ty)?,
        })
    }

    /// Records the panic lang item referenced by a MIR `Assert` terminator so JIR lowering
    /// (`emit.rs::assert_failure`) and link reachability find it in `summary.deps`.
    fn record_assert_panic(&mut self, msg: &mir::AssertMessage<'tcx>) {
        let tcx = self.tcx();
        let lang_item = match msg {
            mir::AssertKind::BoundsCheck { .. } => LangItem::PanicBoundsCheck,
            mir::AssertKind::MisalignedPointerDereference { .. } => {
                LangItem::PanicMisalignedPointerDereference
            }
            mir::AssertKind::InvalidEnumConstruction(_) => LangItem::PanicInvalidEnumConstruction,
            other => other.panic_function(),
        };
        if let Some(def_id) = tcx
            .lang_items()
            .get(lang_item)
            .filter(|def_id| tcx.generics_of(*def_id).count() == 0)
        {
            let panic = Instance::mono(tcx, def_id);
            self.core
                .extra_refs
                .insert(self.core.cgu.namer.fn_name(panic));
        }
    }

    /// Records auxiliary link-level references implied by pointer coercions (`Unsize` vtables,
    /// `ReifyFnPointer` target items, `ClosureFnPointer` `call_once` shims).
    fn record_cast_refs(
        &mut self,
        kind: mir::CastKind,
        value: &mir::Operand<'tcx>,
        result: Ty<'tcx>,
    ) {
        let mir::CastKind::PointerCoercion(coercion, _) = kind else {
            return;
        };
        let tcx = self.tcx();
        let from_ty = self.monomorphize(value.ty(&self.mir.local_decls, tcx));
        let to_ty = self.monomorphize(result);
        match coercion {
            PointerCoercion::Unsize => self.record_unsize_vtables(from_ty, to_ty),
            PointerCoercion::ReifyFnPointer(_) => {
                if let ty::FnDef(def, args) = *crate::value::peel_pattern(from_ty).kind() {
                    if let Some(Ok(Some(inst))) = args
                        .no_bound_vars()
                        .map(|a| Instance::try_resolve(tcx, typing_env(), def, a))
                    {
                        self.core
                            .extra_refs
                            .insert(self.core.cgu.namer.fn_name(inst));
                    }
                }
            }
            PointerCoercion::ClosureFnPointer(_) => {
                if let ty::Closure(def, args) = *crate::value::peel_pattern(from_ty).kind() {
                    let shim = Instance::resolve_closure(tcx, def, args, ty::ClosureKind::FnOnce);
                    self.core
                        .extra_refs
                        .insert(self.core.cgu.namer.fn_name(shim));
                }
            }
            _ => {}
        }
    }

    /// Walks an `Unsize` coercion pair to queue any concrete-to-`dyn` vtable for interning and
    /// record its link name in `extra_refs`.
    fn record_unsize_vtables(&mut self, from_ty: Ty<'tcx>, to_ty: Ty<'tcx>) {
        let tcx = self.tcx();
        let from_ty = crate::value::peel_pattern(from_ty);
        let to_ty = crate::value::peel_pattern(to_ty);
        match (from_ty.kind(), to_ty.kind()) {
            (
                ty::Ref(_, source, _) | ty::RawPtr(source, _),
                ty::Ref(_, target, _) | ty::RawPtr(target, _),
            ) => {
                if crate::value::unsized_tail(tcx, *target).is_some() {
                    return;
                }
                let (source, target) =
                    tcx.struct_lockstep_tails_for_codegen(*source, *target, typing_env());
                if !matches!(source.kind(), ty::Dynamic(..)) {
                    if let ty::Dynamic(data, ..) = target.kind() {
                        let principal = data
                            .principal()
                            .map(|p| tcx.instantiate_bound_regions_with_erased(p));
                        self.core.pending_vtables.push((source, principal));
                        let name = self.core.cgu.namer.vtable_name(source, principal);
                        self.core.extra_refs.insert(name);
                    }
                }
            }
            (ty::Adt(def_a, args_a), ty::Adt(def_b, args_b)) => {
                if let (Some(from_field), Some(to_field)) = (
                    crate::value::transparent_field(tcx, from_ty),
                    crate::value::transparent_field(tcx, to_ty),
                ) {
                    self.record_unsize_vtables(from_field, to_field);
                } else if def_a == def_b {
                    let variant = def_a.non_enum_variant();
                    for field in &variant.fields {
                        let field_from =
                            tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args_a));
                        let field_to =
                            tcx.normalize_erasing_regions(typing_env(), field.ty(tcx, args_b));
                        if field_from != field_to && !crate::value::is_zst(tcx, field_from) {
                            self.record_unsize_vtables(field_from, field_to);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

pub(super) fn block_id(block: mir::BasicBlock) -> BlockId {
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

pub(super) fn unwind_action(action: mir::UnwindAction) -> Unwind {
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
