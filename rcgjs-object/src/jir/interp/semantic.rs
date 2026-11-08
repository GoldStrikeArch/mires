//! The JIR-S interpreter: evaluates a [`Body`] with every local a heap object.

use crate::jir::interns::Interns;
use crate::jir::semantic::{
    AggregateKind, AssertMessage, BlockId, Body, BodyKind, Call, Callee, Constant, DirectItemRef,
    LocalId, LocalRole, Operand, OperandKind, Place, Projection, Rvalue, StatementKind, Terminator,
    TypeId,
};
use crate::jir::types::TypeTable;

use super::eval;
use super::heap::{fresh_aggregate, Heap, Navigated, PathStep, PlaceRef};
use super::value::{HeapId, Location, Value};
use super::{Budget, Event, Hooks, Outcome, Trap};

/// Evaluates `body` (a function) with `args` bound to its argument locals.
///
/// A spread argument is passed as its tuple, one value per MIR argument.
pub fn run_body(
    body: &Body,
    table: &TypeTable,
    interns: &Interns,
    args: Vec<Value>,
    hooks: &Hooks<'_>,
) -> Outcome {
    let mut heap = Heap::new();
    let tracked: Vec<Value> = args
        .iter()
        .filter(|arg| matches!(arg, Value::Ptr(_) | Value::Slice { .. }))
        .cloned()
        .collect();
    let (ret, trace) = match SemanticInterpreter::new(body, table, interns, hooks, &mut heap, args)
    {
        Ok(mut interpreter) => (interpreter.run(), interpreter.trace),
        Err(trap) => (Err(trap), Vec::new()),
    };
    Outcome {
        ret,
        trace,
        pointees: super::pointee_snapshots(&tracked, &heap),
    }
}

/// The JIR-S evaluator over a caller-owned heap, so a test can share the heap between a body and
/// a function.
pub struct SemanticInterpreter<'a> {
    body: &'a Body,
    table: &'a TypeTable,
    interns: &'a Interns,
    hooks: &'a Hooks<'a>,
    heap: &'a mut Heap,
    locals: Vec<HeapId>,
    budget: Budget,
    pub trace: Vec<Event>,
}

impl<'a> SemanticInterpreter<'a> {
    pub fn new(
        body: &'a Body,
        table: &'a TypeTable,
        interns: &'a Interns,
        hooks: &'a Hooks<'a>,
        heap: &'a mut Heap,
        args: Vec<Value>,
    ) -> Result<Self, Trap> {
        if !matches!(body.kind, BodyKind::Function) {
            return Err(Trap::Unsupported("evaluating a static or vtable body"));
        }
        let mut locals = Vec::with_capacity(body.locals.len());
        let mut args = args.into_iter();
        for local in &body.locals {
            let value = match local.role {
                LocalRole::Argument => args
                    .next()
                    .ok_or(Trap::Malformed("fewer arguments than argument locals"))?,
                LocalRole::Return | LocalRole::Temporary => Value::Undef,
            };
            locals.push(heap.alloc(value));
        }
        if args.next().is_some() {
            return Err(Trap::Malformed("more arguments than argument locals"));
        }
        Ok(Self {
            body,
            table,
            interns,
            hooks,
            heap,
            locals,
            budget: Budget {
                remaining: hooks.step_limit,
            },
            trace: Vec::new(),
        })
    }

    fn local(&self, local: LocalId) -> Result<HeapId, Trap> {
        self.locals
            .get(local.0 as usize)
            .copied()
            .ok_or(Trap::Malformed("statement names a missing local"))
    }

    fn local_type(&self, local: LocalId) -> Result<TypeId, Trap> {
        self.body
            .locals
            .get(local.0 as usize)
            .map(|local| local.ty)
            .ok_or(Trap::Malformed("statement names a missing local"))
    }

    fn steps(&self, projection: &[Projection]) -> Result<Vec<PathStep>, Trap> {
        projection
            .iter()
            .map(|projection| {
                Ok(match projection {
                    Projection::Deref => PathStep::Deref,
                    Projection::Field { field, .. } => PathStep::Field { field: *field },
                    Projection::Index(local) => {
                        let index = self
                            .heap
                            .read(&Location::root(self.local(*local)?))?
                            .as_int()
                            .ok_or(Trap::Ub("non-integer index"))?;
                        if index < 0 {
                            return Err(Trap::Ub("negative index"));
                        }
                        PathStep::Index(index as u64)
                    }
                    Projection::ConstantIndex {
                        offset,
                        min_length,
                        from_end,
                    } => PathStep::ConstantIndex {
                        offset: *offset,
                        min_length: *min_length,
                        from_end: *from_end,
                    },
                    Projection::Subslice { from, to, from_end } => PathStep::Subslice {
                        from: *from,
                        to: *to,
                        from_end: *from_end,
                    },
                    Projection::Downcast { variant } => PathStep::Downcast { variant: *variant },
                    Projection::OpaqueCast(_) | Projection::UnwrapUnsafeBinder(_) => {
                        return Err(Trap::Unsupported("binder projection"))
                    }
                })
            })
            .collect()
    }

    fn navigate(&self, place: &Place, strict: bool) -> Result<Navigated, Trap> {
        let steps = self.steps(&place.projection)?;
        let start = PlaceRef::Loc(Location::root(self.local(place.local)?));
        self.heap.navigate(
            start,
            self.local_type(place.local)?,
            steps,
            self.table,
            strict,
        )
    }

    fn read_place(&self, place: &Place) -> Result<Value, Trap> {
        let navigated = self.navigate(place, true)?;
        let value = self.heap.read_place(&navigated.place)?;
        if matches!(value, Value::Undef) {
            return Err(Trap::Ub("read of uninitialized storage"));
        }
        Ok(value)
    }

    fn write_place(&mut self, place: &Place, value: Value) -> Result<(), Trap> {
        let navigated = self.navigate(place, false)?;
        let PlaceRef::Loc(location) = navigated.place else {
            return Err(Trap::Ub("write to an unsized place"));
        };
        let created = navigated.created;
        *self.heap.slot(&location, Some(&created), self.table)? = value;
        Ok(())
    }

    fn operand(&self, operand: &Operand) -> Result<Value, Trap> {
        match &operand.kind {
            OperandKind::Copy(place) | OperandKind::Move(place) => self.read_place(place),
            OperandKind::Constant(constant) => self.constant(constant, operand.ty),
            OperandKind::RuntimeCheck(value) => Ok(Value::Bool(*value)),
        }
    }

    fn constant(&self, constant: &Constant, ty: TypeId) -> Result<Value, Trap> {
        match constant {
            Constant::ZeroSized => eval::zero_sized(ty, self.table),
            Constant::Scalar { bits, .. } => eval::scalar(*bits, ty, self.table),
            Constant::String(id) => self
                .interns
                .str(*id)
                .map(|s| Value::Str(s.to_owned()))
                .ok_or(Trap::Malformed("string constant names a missing string")),
            Constant::Pointer { target, .. } => match target {
                crate::jir::semantic::GlobalReference::Function { link_name, .. } => {
                    Ok(Value::Fn(*link_name))
                }
                _ => Err(Trap::Unsupported("pointer constant to a static or type")),
            },
            Constant::Memory(tree) => eval::const_tree(tree, ty, self.table, self.interns),
        }
    }

    fn rvalue(&mut self, rvalue: &Rvalue, result_ty: TypeId) -> Result<Value, Trap> {
        match rvalue {
            Rvalue::Use(operand) | Rvalue::WrapUnsafeBinder(operand) => self.operand(operand),
            Rvalue::CopyForDeref(place) => self.read_place(place),
            Rvalue::Repeat { value, count } => {
                let value = self.operand(value)?;
                Ok(Value::Aggregate {
                    ty: result_ty,
                    variant: 0,
                    fields: vec![value; *count as usize],
                })
            }
            Rvalue::Reference { place, .. }
            | Rvalue::RawPointer { place, .. }
            | Rvalue::Reborrow { place, .. } => {
                let navigated = self.navigate(place, false)?;
                if let PlaceRef::Loc(location) = &navigated.place {
                    // Borrowing a place brings its storage into existence (a `&mut` to a
                    // not-yet-initialized aggregate written through the reference).
                    if let Some(Projection::Downcast { .. }) = place.projection.last() {
                        return Err(Trap::Unsupported("reference to a downcast place"));
                    }
                    let created = navigated.created.clone();
                    if !created.is_empty() {
                        self.heap.slot(location, Some(&created), self.table)?;
                    }
                }
                Ok(Heap::address_of(&navigated.place))
            }
            Rvalue::Cast {
                kind,
                value,
                result,
            } => {
                let value = self.operand(value)?;
                eval::cast(*kind, &value, *result, self.table, self.heap)
            }
            Rvalue::Binary { op, left, right } => {
                let l = self.operand(left)?;
                let r = self.operand(right)?;
                eval::binary(*op, &l, &r, left.ty, result_ty, self.table)
            }
            Rvalue::Unary { op, value } => {
                let v = self.operand(value)?;
                eval::unary(*op, &v)
            }
            Rvalue::Discriminant(place) => {
                let value = self.read_place(place)?;
                eval::discriminant(&value, result_ty, self.table)
            }
            Rvalue::Aggregate {
                kind,
                result,
                operands,
            } => {
                let fields = operands
                    .iter()
                    .map(|operand| self.operand(operand))
                    .collect::<Result<Vec<_>, _>>()?;
                let variant = match kind {
                    AggregateKind::Adt { variant, .. } => *variant,
                    AggregateKind::RawPointer { .. } => {
                        return Err(Trap::Unsupported("raw pointer aggregate"))
                    }
                    _ => 0,
                };
                if let AggregateKind::Adt {
                    active_field: Some(active),
                    ..
                } = kind
                {
                    // A union: place the one operand at its field.
                    let mut union = fresh_aggregate(*result, 0, self.table)?;
                    if let Value::Aggregate { fields: slots, .. } = &mut union {
                        let slot = slots
                            .get_mut(*active as usize)
                            .ok_or(Trap::Malformed("union active field out of range"))?;
                        *slot = fields.into_iter().next().unwrap_or(Value::Unit);
                    }
                    return Ok(union);
                }
                Ok(Value::Aggregate {
                    ty: *result,
                    variant,
                    fields,
                })
            }
        }
    }

    fn statement(&mut self, statement: &StatementKind) -> Result<(), Trap> {
        match statement {
            StatementKind::Assign { destination, value } => {
                let value = self.rvalue(value, destination.ty)?;
                self.write_place(destination, value)
            }
            StatementKind::SetDiscriminant { place, variant } => {
                let navigated = self.navigate(place, false)?;
                let PlaceRef::Loc(location) = navigated.place else {
                    return Err(Trap::Ub("set discriminant of an unsized place"));
                };
                let created = navigated.created;
                let ty = navigated.ty;
                let mut slot = self.heap.slot(&location, Some(&created), self.table)?;
                eval::set_discriminant(&mut slot, *variant, ty, self.table)
            }
            StatementKind::StorageLive(local) => {
                let heap = self.local(*local)?;
                self.heap.write(&Location::root(heap), Value::Undef)
            }
            StatementKind::StorageDead(_) | StatementKind::Nop => Ok(()),
            StatementKind::Assume(operand) => {
                if self.operand(operand)?.as_bool() == Some(false) {
                    return Err(Trap::Ub("assume(false)"));
                }
                Ok(())
            }
            StatementKind::CopyNonOverlapping { .. } => {
                Err(Trap::Unsupported("copy_nonoverlapping"))
            }
        }
    }

    fn call(&mut self, call: &Call) -> Result<Option<BlockId>, Trap> {
        let args = call
            .arguments
            .iter()
            .map(|argument| self.operand(argument))
            .collect::<Result<Vec<_>, _>>()?;
        let result = match &call.callee {
            Callee::Direct { item, link_name } => {
                let callee = DirectItemRef {
                    item: *item,
                    link_name: *link_name,
                };
                let result = (self.hooks.call)(&callee, args.clone());
                self.trace.push(Event::Call {
                    callee,
                    args,
                    result: result.clone(),
                });
                result?
            }
            Callee::Nop => {
                self.trace.push(Event::Builtin {
                    name: "nop".to_owned(),
                    args,
                });
                Value::Unit
            }
            Callee::Runtime { .. } => return Err(Trap::Unsupported("runtime member call")),
            Callee::Foreign { .. } => return Err(Trap::Unsupported("foreign call")),
            Callee::Indirect { .. } => return Err(Trap::Unsupported("indirect call")),
            Callee::Intrinsic { .. } => return Err(Trap::Unsupported("intrinsic call")),
            Callee::Virtual { .. } => return Err(Trap::Unsupported("virtual call")),
        };
        if !self.table.is_zst(call.destination.ty) || call.destination.projection.is_empty() {
            self.write_place(&call.destination, result)?;
        }
        Ok(call.target)
    }

    fn assert_kind(message: &AssertMessage) -> &'static str {
        match message {
            AssertMessage::BoundsCheck { .. } => "bounds check",
            AssertMessage::Overflow { .. } => "arithmetic overflow",
            AssertMessage::OverflowNeg(_) => "negation overflow",
            AssertMessage::DivisionByZero(_) => "division by zero",
            AssertMessage::RemainderByZero(_) => "remainder by zero",
            AssertMessage::ResumedAfterReturn(_) => "resumed after return",
            AssertMessage::ResumedAfterPanic(_) => "resumed after panic",
            AssertMessage::ResumedAfterDrop(_) => "resumed after drop",
            AssertMessage::MisalignedPointer { .. } => "misaligned pointer",
            AssertMessage::NullPointer => "null pointer",
            AssertMessage::NullReference => "null reference",
            AssertMessage::InvalidEnum(_) => "invalid enum",
        }
    }

    /// Runs the body to its return.
    pub fn run(&mut self) -> Result<Value, Trap> {
        let mut block = BlockId(0);
        loop {
            let basic_block = self
                .body
                .blocks
                .get(block.0 as usize)
                .ok_or(Trap::Malformed("terminator names a missing block"))?;
            if basic_block.cleanup {
                return Err(Trap::Unwind);
            }
            for statement in &basic_block.statements {
                self.budget.step()?;
                self.statement(&statement.kind)?;
            }
            self.budget.step()?;
            block = match &basic_block.terminator {
                Terminator::Goto(target) => *target,
                Terminator::FalseEdge { real, .. } | Terminator::FalseUnwind { real, .. } => *real,
                Terminator::Switch {
                    discriminant,
                    targets,
                    otherwise,
                } => {
                    let bits = self
                        .operand(discriminant)?
                        .as_switch_bits()
                        .ok_or(Trap::Ub("switch on a non-integer"))?;
                    targets
                        .iter()
                        .find(|(value, _)| *value == bits)
                        .map_or(*otherwise, |(_, target)| *target)
                }
                Terminator::Return => {
                    let ret = self.heap.read(&Location::root(self.local(LocalId(0))?))?;
                    let ret = match ret {
                        Value::Undef if self.table.is_zst(self.body.signature.result) => {
                            eval::zero_sized(self.body.signature.result, self.table)?
                        }
                        Value::Undef => return Err(Trap::Ub("return of uninitialized storage")),
                        ret => ret,
                    };
                    return Ok(ret);
                }
                Terminator::Unreachable => return Err(Trap::Unreachable),
                Terminator::Call(call) => match self.call(call)? {
                    Some(target) => target,
                    None => return Err(Trap::Unreachable),
                },
                Terminator::Drop {
                    place,
                    target,
                    glue,
                    ..
                } => {
                    let value = self.read_place(place).unwrap_or(Value::Undef);
                    if !matches!(glue, crate::jir::semantic::DropGlue::None) {
                        self.trace.push(Event::Drop {
                            ty: place.ty,
                            value: value.clone(),
                        });
                        if let Some(drop) = self.hooks.drop {
                            drop(place.ty, &value)?;
                        }
                    }
                    *target
                }
                Terminator::Assert {
                    condition,
                    expected,
                    message,
                    target,
                    ..
                } => {
                    let condition = self
                        .operand(condition)?
                        .as_bool()
                        .ok_or(Trap::Ub("assert on a non-bool"))?;
                    if condition != *expected {
                        return Err(Trap::Assert(Self::assert_kind(message)));
                    }
                    *target
                }
                Terminator::UnwindResume | Terminator::UnwindTerminate(_) => {
                    return Err(Trap::Unwind)
                }
            };
        }
    }
}
