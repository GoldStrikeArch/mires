//! The JIR-R interpreter: evaluates a [`Func`] with every SSA value and every cell a heap object.

use std::collections::BTreeMap;

use crate::jir::interns::{Interns, StrId};
use crate::jir::repr::{
    CellId, Func, Op, OpKind, Proj, RAssertMessage, RBlockId, RTerm, Root, ValueId,
};
use crate::jir::semantic::{Callee, DirectItemRef, DropGlue, TypeId};
use crate::jir::types::TypeTable;

use super::eval;
use super::heap::{Heap, Navigated, PathStep, PlaceRef};
use super::value::{HeapId, Location, Value};
use super::{Budget, Event, Hooks, Outcome, Trap};

/// Evaluates `func` with `args` bound to its parameters.
pub fn run_func(
    func: &Func,
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
    let (ret, trace) = {
        let mut interpreter = ReprInterpreter::new(func, table, interns, hooks, &mut heap);
        let ret = interpreter.run(args);
        (ret, interpreter.trace)
    };
    Outcome {
        ret,
        trace,
        pointees: super::pointee_snapshots(&tracked, &heap),
    }
}

/// A suspended direct caller. Frames use the interpreter heap, not the host call stack.
struct Caller<'a> {
    func: &'a Func,
    values: Vec<Option<HeapId>>,
    cells: Vec<Option<HeapId>>,
    block: RBlockId,
    next_op: usize,
    result: Option<ValueId>,
    callee: DirectItemRef,
    args: Vec<Value>,
}

/// The JIR-R evaluator over a caller-owned heap.
pub struct ReprInterpreter<'a> {
    func: &'a Func,
    table: &'a TypeTable,
    interns: &'a Interns,
    hooks: &'a Hooks<'a>,
    heap: &'a mut Heap,
    /// The heap slot of each value, once defined on the current path.
    values: Vec<Option<HeapId>>,
    /// The heap object of each cell, once allocated.
    cells: Vec<Option<HeapId>>,
    budget: Budget,
    callees: Option<&'a BTreeMap<StrId, &'a Func>>,
    pub trace: Vec<Event>,
}

impl<'a> ReprInterpreter<'a> {
    pub fn new(
        func: &'a Func,
        table: &'a TypeTable,
        interns: &'a Interns,
        hooks: &'a Hooks<'a>,
        heap: &'a mut Heap,
    ) -> Self {
        Self {
            func,
            table,
            interns,
            hooks,
            heap,
            values: vec![None; func.values.len()],
            cells: vec![None; func.cells.len()],
            budget: Budget {
                remaining: hooks.step_limit,
            },
            trace: Vec::new(),
            callees: None,
        }
    }

    /// Execute direct callees over the same heap and step budget instead of calling the hook.
    /// Every function must use this interpreter's type and intern tables. Missing callees and
    /// calls beyond 128 nested frames trap rather than fabricate a result.
    pub fn with_callees(mut self, callees: &'a BTreeMap<StrId, &'a Func>) -> Self {
        self.callees = Some(callees);
        self
    }

    fn slot(&self, v: ValueId) -> Result<HeapId, Trap> {
        self.values
            .get(v.0 as usize)
            .copied()
            .flatten()
            .ok_or(Trap::Malformed("use of an undefined value"))
    }

    fn ty(&self, v: ValueId) -> Result<TypeId, Trap> {
        self.func
            .ty(v)
            .ok_or(Trap::Malformed("use of a missing value"))
    }

    fn read(&self, v: ValueId) -> Result<Value, Trap> {
        let value = self.heap.read(&Location::root(self.slot(v)?))?;
        if matches!(value, Value::Undef) {
            return Err(Trap::Ub("read of an undefined value"));
        }
        Ok(value)
    }

    fn define(&mut self, v: ValueId, value: Value) -> Result<(), Trap> {
        let slot = self
            .values
            .get_mut(v.0 as usize)
            .ok_or(Trap::Malformed("definition of a missing value"))?;
        // A block re-entered in a loop redefines its values: give each definition fresh storage
        // so a pointer taken in an earlier iteration keeps naming that iteration's value.
        *slot = Some(self.heap.alloc(value));
        Ok(())
    }

    fn cell(&self, c: CellId) -> Result<HeapId, Trap> {
        self.cells
            .get(c.0 as usize)
            .copied()
            .flatten()
            .ok_or(Trap::Ub("use of an unallocated cell"))
    }

    fn steps(&self, path: &[Proj]) -> Result<Vec<PathStep>, Trap> {
        path.iter()
            .map(|proj| {
                Ok(match proj {
                    Proj::Deref => PathStep::Deref,
                    Proj::Field { field, .. } => PathStep::Field { field: *field },
                    Proj::Index(v) => {
                        let index = self
                            .read(*v)?
                            .as_int()
                            .ok_or(Trap::Ub("non-integer index"))?;
                        if index < 0 {
                            return Err(Trap::Ub("negative index"));
                        }
                        PathStep::Index(index as u64)
                    }
                    Proj::ConstantIndex {
                        offset,
                        min_length,
                        from_end,
                    } => PathStep::ConstantIndex {
                        offset: *offset,
                        min_length: *min_length,
                        from_end: *from_end,
                    },
                    Proj::Subslice { from, to, from_end } => PathStep::Subslice {
                        from: *from,
                        to: *to,
                        from_end: *from_end,
                    },
                    Proj::Downcast { variant } => PathStep::Downcast { variant: *variant },
                })
            })
            .collect()
    }

    fn navigate(&self, root: &Root, path: &[Proj], strict: bool) -> Result<Navigated, Trap> {
        let (heap, ty) = match root {
            Root::Value(v) => (self.slot(*v)?, self.ty(*v)?),
            Root::Cell(c) => (
                self.cell(*c)?,
                self.func
                    .cells
                    .get(c.0 as usize)
                    .ok_or(Trap::Malformed("use of a missing cell"))?
                    .ty,
            ),
        };
        let steps = self.steps(path)?;
        self.heap.navigate(
            PlaceRef::Loc(Location::root(heap)),
            ty,
            steps,
            self.table,
            strict,
        )
    }

    /// The mutable storage of value `base`, downcast to `variant` when given, at field `field`.
    fn field_slot(
        &mut self,
        base: ValueId,
        variant: Option<u32>,
        field: u32,
    ) -> Result<super::heap::HeapSlot<'_>, Trap> {
        let ty = self.ty(base)?;
        let heap = self.slot(base)?;
        let location = Location::root(heap).child(super::value::Step::Field(field));
        let created = [(ty, variant)];
        self.heap.slot(&location, Some(&created), self.table)
    }

    fn op(&mut self, op: &Op) -> Result<(), Trap> {
        let value = match &op.kind {
            OpKind::Const(tree) => {
                let ty = self.ty(op.result.ok_or(Trap::Malformed("Const without a result"))?)?;
                eval::const_tree(tree, ty, self.table, self.interns)?
            }
            OpKind::Copy(v) | OpKind::Move(v) | OpKind::Materialize(v) => self.read(*v)?,
            OpKind::Aggregate {
                ty,
                variant,
                active_field,
                fields,
            } => {
                let values = fields
                    .iter()
                    .map(|field| self.read(*field))
                    .collect::<Result<Vec<_>, _>>()?;
                match active_field {
                    Some(active) => {
                        let mut union = super::heap::fresh_aggregate(*ty, 0, self.table)?;
                        if let Value::Aggregate { fields: slots, .. } = &mut union {
                            let slot = slots
                                .get_mut(*active as usize)
                                .ok_or(Trap::Malformed("union active field out of range"))?;
                            *slot = values.into_iter().next().unwrap_or(Value::Unit);
                        }
                        union
                    }
                    None => Value::Aggregate {
                        ty: *ty,
                        variant: *variant,
                        fields: values,
                    },
                }
            }
            OpKind::Repeat { v, count } => {
                let value = self.read(*v)?;
                let ty = self.ty(op
                    .result
                    .ok_or(Trap::Malformed("Repeat without a result"))?)?;
                Value::Aggregate {
                    ty,
                    variant: 0,
                    fields: vec![value; *count as usize],
                }
            }
            OpKind::Field {
                base,
                variant,
                field,
            } => {
                let value = self.read(*base)?;
                let Value::Aggregate {
                    variant: active,
                    fields,
                    ..
                } = value
                else {
                    return Err(Trap::Ub("field of a non-aggregate"));
                };
                if variant.is_some_and(|variant| variant != active) {
                    return Err(Trap::Ub("downcast to an inactive variant"));
                }
                let value = fields
                    .into_iter()
                    .nth(*field as usize)
                    .ok_or(Trap::Ub("field out of range"))?;
                if matches!(value, Value::Undef) {
                    return Err(Trap::Ub("read of an uninitialized field"));
                }
                value
            }
            OpKind::SetField {
                base,
                variant,
                field,
                v,
            } => {
                let value = self.read(*v)?;
                *self.field_slot(*base, *variant, *field)? = value;
                return Ok(());
            }
            OpKind::Discriminant(base) => {
                let value = self.read(*base)?;
                let ty = self.ty(op
                    .result
                    .ok_or(Trap::Malformed("Discriminant without a result"))?)?;
                eval::discriminant(&value, ty, self.table)?
            }
            OpKind::SetDiscriminant { base, variant } => {
                let ty = self.ty(*base)?;
                let heap = self.slot(*base)?;
                let mut slot = self.heap.slot(&Location::root(heap), None, self.table)?;
                eval::set_discriminant(&mut slot, *variant, ty, self.table)?;
                return Ok(());
            }
            OpKind::CellAlloc(cell) => {
                let heap = self.heap.alloc(Value::Undef);
                *self
                    .cells
                    .get_mut(cell.0 as usize)
                    .ok_or(Trap::Malformed("alloc of a missing cell"))? = Some(heap);
                return Ok(());
            }
            OpKind::CellLoad(cell) => {
                let value = self.heap.read(&Location::root(self.cell(*cell)?))?;
                if matches!(value, Value::Undef) {
                    return Err(Trap::Ub("load of an uninitialized cell"));
                }
                value
            }
            OpKind::CellStore { cell, v } => {
                let value = self.read(*v)?;
                let heap = self.cell(*cell)?;
                self.heap.write(&Location::root(heap), value)?;
                return Ok(());
            }
            OpKind::BorrowView { root, path, .. } | OpKind::RawView { root, path, .. } => {
                let navigated = self.navigate(root, path, false)?;
                if let PlaceRef::Loc(location) = &navigated.place {
                    let created = navigated.created.clone();
                    if !created.is_empty() {
                        self.heap.slot(location, Some(&created), self.table)?;
                    }
                }
                Heap::address_of(&navigated.place)
            }
            OpKind::SliceView { ptr, len } => {
                let len = self
                    .read(*len)?
                    .as_int()
                    .ok_or(Trap::Ub("non-integer slice length"))?;
                if len < 0 {
                    return Err(Trap::Ub("negative slice length"));
                }
                match self.read(*ptr)? {
                    Value::Slice { buf, off, .. } => Value::Slice {
                        buf,
                        off,
                        len: len as u64,
                    },
                    Value::Ptr(location) => match location.path.last() {
                        Some(super::value::Step::Index(index)) => {
                            let mut buf = location.clone();
                            buf.path.pop();
                            Value::Slice {
                                buf,
                                off: *index,
                                len: len as u64,
                            }
                        }
                        _ => Value::Slice {
                            buf: location,
                            off: 0,
                            len: len as u64,
                        },
                    },
                    _ => return Err(Trap::Unsupported("slice view of a non-pointer")),
                }
            }
            OpKind::Deref(ptr) => {
                let pointer = self.read(*ptr)?;
                let place = Heap::place_of(&pointer)?;
                let value = self.heap.read_place(&place)?;
                if matches!(value, Value::Undef) {
                    return Err(Trap::Ub("read of uninitialized storage"));
                }
                value
            }
            OpKind::StoreThrough { ptr, v } => {
                let value = self.read(*v)?;
                let pointer = self.read(*ptr)?;
                let PlaceRef::Loc(location) = Heap::place_of(&pointer)? else {
                    return Err(Trap::Ub("store through an unsized pointer"));
                };
                self.heap.write(&location, value)?;
                return Ok(());
            }
            OpKind::Binary { op: bin, l, r, ty } => {
                let result_ty = self.ty(op
                    .result
                    .ok_or(Trap::Malformed("Binary without a result"))?)?;
                let l = self.read(*l)?;
                let r = self.read(*r)?;
                eval::binary(*bin, &l, &r, *ty, result_ty, self.table)?
            }
            OpKind::Unary { op: un, v, .. } => {
                let v = self.read(*v)?;
                eval::unary(*un, &v)?
            }
            OpKind::Cast { kind, v, to, .. } => {
                let v = self.read(*v)?;
                eval::cast(*kind, &v, *to, self.table, self.heap)?
            }
            OpKind::Call { callee, args, .. } => {
                let args = args
                    .iter()
                    .map(|arg| self.read(*arg))
                    .collect::<Result<Vec<_>, _>>()?;
                let returned = match callee {
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
                    Callee::Intrinsic { .. } => {
                        return Err(Trap::Malformed("Call with an intrinsic callee"))
                    }
                    Callee::Virtual { .. } => return Err(Trap::Unsupported("virtual call")),
                };
                match op.result {
                    Some(result) => return self.define(result, returned),
                    None => return Ok(()),
                }
            }
            OpKind::Intrinsic { .. } => return Err(Trap::Unsupported("intrinsic")),
            OpKind::Host { .. } => return Err(Trap::Unsupported("host operation")),
            OpKind::Drop { root, path, glue } => {
                if !matches!(glue, DropGlue::None) {
                    let navigated = self.navigate(root, path, false)?;
                    let value = self
                        .heap
                        .read_place(&navigated.place)
                        .unwrap_or(Value::Undef);
                    self.trace.push(Event::Drop {
                        ty: navigated.ty,
                        value: value.clone(),
                    });
                    if let Some(drop) = self.hooks.drop {
                        drop(navigated.ty, &value)?;
                    }
                }
                return Ok(());
            }
            OpKind::Assert {
                cond,
                expected,
                msg,
                ..
            } => {
                let cond = self
                    .read(*cond)?
                    .as_bool()
                    .ok_or(Trap::Ub("assert on a non-bool"))?;
                if cond != *expected {
                    return Err(Trap::Assert(assert_kind(msg)));
                }
                return Ok(());
            }
            OpKind::Convert { .. } => return Err(Trap::Unsupported("carrier conversion")),
            OpKind::Zombie(_) => return Err(Trap::Unsupported("zombie")),
            // A spill register is a module-scope `let` the flat call ABI writes; the interpreter
            // runs one function on its own heap and has nowhere to put one.
            OpKind::SpillRead(_) | OpKind::SpillWrite { .. } => {
                return Err(Trap::Unsupported("spill register"))
            }
        };
        let result = op
            .result
            .ok_or(Trap::Malformed("value op without a result"))?;
        self.define(result, value)
    }

    /// Runs the function to its return.
    pub fn run(&mut self, args: Vec<Value>) -> Result<Value, Trap> {
        if self.func.has_invoke() {
            return Err(Trap::Unsupported("exceptional call execution"));
        }
        if args.len() != self.func.params.len() {
            return Err(Trap::Malformed(
                "argument count differs from the parameter count",
            ));
        }
        let mut callers: Vec<Caller<'a>> = Vec::new();
        let outcome = (|| {
            let mut block = RBlockId(0);
            let mut next_op = 0;
            let mut incoming = Some(args);
            loop {
                let current = self
                    .func
                    .blocks
                    .get(block.0 as usize)
                    .ok_or(Trap::Malformed("terminator names a missing block"))?;
                if let Some(incoming) = incoming.take() {
                    if current.params.len() != incoming.len() {
                        return Err(Trap::Malformed(
                            "edge argument count differs from the parameters",
                        ));
                    }
                    for (param, value) in current.params.iter().zip(incoming) {
                        self.define(*param, value)?;
                    }
                }
                if let Some(op) = current.ops.get(next_op) {
                    self.budget.step()?;
                    if let (
                        Some(callees),
                        OpKind::Call {
                            callee: Callee::Direct { item, link_name },
                            args,
                            ..
                        },
                    ) = (self.callees, &op.kind)
                    {
                        let func = *callees
                            .get(link_name)
                            .ok_or_else(|| Trap::Hook("callee body unavailable".to_owned()))?;
                        if func.has_invoke() {
                            return Err(Trap::Unsupported("exceptional call execution"));
                        }
                        let args = args
                            .iter()
                            .map(|arg| self.read(*arg))
                            .collect::<Result<Vec<_>, _>>()?;
                        if args.len() != func.params.len() {
                            return Err(Trap::Malformed(
                                "argument count differs from the parameter count",
                            ));
                        }
                        if callers.len() == 128 {
                            return Err(Trap::Unsupported("direct-call depth limit"));
                        }
                        callers.push(Caller {
                            func: self.func,
                            values: std::mem::replace(
                                &mut self.values,
                                vec![None; func.values.len()],
                            ),
                            cells: std::mem::replace(&mut self.cells, vec![None; func.cells.len()]),
                            block,
                            next_op: next_op + 1,
                            result: op.result,
                            callee: DirectItemRef {
                                item: *item,
                                link_name: *link_name,
                            },
                            args: args.clone(),
                        });
                        self.func = func;
                        block = RBlockId(0);
                        next_op = 0;
                        incoming = Some(args);
                    } else {
                        self.op(op)?;
                        next_op += 1;
                    }
                    continue;
                }
                self.budget.step()?;
                let (next, args) = match &current.term {
                    RTerm::Goto(target, args) => (*target, args),
                    RTerm::Switch {
                        scrut,
                        cases,
                        default,
                        ..
                    } => {
                        let bits = self
                            .read(*scrut)?
                            .as_switch_bits()
                            .ok_or(Trap::Ub("switch on a non-integer"))?;
                        cases
                            .iter()
                            .find(|(value, _, _)| *value == bits)
                            .map_or((default.0, &default.1), |(_, target, args)| (*target, args))
                    }
                    RTerm::Return(v) => {
                        let returned = match v {
                            Some(v) => self.read(*v)?,
                            None => eval::zero_sized(self.func.sig.result, self.table)?,
                        };
                        let Some(caller) = callers.pop() else {
                            return Ok(returned);
                        };
                        self.func = caller.func;
                        self.values = caller.values;
                        self.cells = caller.cells;
                        block = caller.block;
                        next_op = caller.next_op;
                        if let Some(result) = caller.result {
                            self.define(result, returned.clone())?;
                        }
                        self.trace.push(Event::Call {
                            callee: caller.callee,
                            args: caller.args,
                            result: Ok(returned),
                        });
                        continue;
                    }
                    RTerm::Invoke { .. } => {
                        return Err(Trap::Unsupported("exceptional call execution"))
                    }
                    RTerm::Unreachable => return Err(Trap::Unreachable),
                    RTerm::Throw(_) => return Err(Trap::Unwind),
                };
                incoming = Some(
                    args.iter()
                        .map(|arg| self.read(*arg))
                        .collect::<Result<Vec<_>, _>>()?,
                );
                block = next;
                next_op = 0;
            }
        })();
        if let Err(trap) = &outcome {
            for caller in callers.iter().rev() {
                self.trace.push(Event::Call {
                    callee: caller.callee,
                    args: caller.args.clone(),
                    result: Err(trap.clone()),
                });
            }
        }
        if let Some(root) = callers.into_iter().next() {
            self.func = root.func;
            self.values = root.values;
            self.cells = root.cells;
        }
        outcome
    }
}

fn assert_kind(message: &RAssertMessage) -> &'static str {
    match message {
        RAssertMessage::BoundsCheck { .. } => "bounds check",
        RAssertMessage::Overflow { .. } => "arithmetic overflow",
        RAssertMessage::OverflowNeg(_) => "negation overflow",
        RAssertMessage::DivisionByZero(_) => "division by zero",
        RAssertMessage::RemainderByZero(_) => "remainder by zero",
        RAssertMessage::ResumedAfterReturn(_) => "resumed after return",
        RAssertMessage::ResumedAfterPanic(_) => "resumed after panic",
        RAssertMessage::ResumedAfterDrop(_) => "resumed after drop",
        RAssertMessage::MisalignedPointer { .. } => "misaligned pointer",
        RAssertMessage::NullPointer => "null pointer",
        RAssertMessage::NullReference => "null reference",
        RAssertMessage::InvalidEnum(_) => "invalid enum",
    }
}
