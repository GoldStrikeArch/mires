//! Two interpreters over one abstract heap: one evaluates a JIR-S [`super::semantic::Body`], the
//! other a JIR-R [`super::repr::Func`], with the same [`Value`] model, the same [`Trap`]s, and the
//! same trace [`Event`]s, so a lowering from one to the other is testable as `run_body(..) ==
//! run_func(..)`.
//!
//! Scope: the pure, scalar, aggregate, enum, reference, slice, and cell subset. Integer ops have
//! Rust's exact wrapping and overflow-reporting semantics; `f32` arithmetic rounds after every op;
//! casts, discriminants, aggregates, references to locals/fields/elements, slice views, and cells
//! are modelled. Calls reach a caller-supplied hook, drops are traced (and optionally hooked),
//! failed asserts trap. Everything else traps with [`Trap::Unsupported`]; nothing panics.

mod eval;
mod heap;
mod repr;
mod semantic;
mod value;

#[cfg(test)]
mod tests;

pub use heap::*;
pub use repr::*;
pub use semantic::*;
pub use value::*;

use std::collections::BTreeMap;
use std::fmt;

use super::semantic::{DirectItemRef, TypeId};

/// Why an evaluation stopped without returning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trap {
    /// A construct outside the interpreter's subset.
    Unsupported(&'static str),
    /// A failed `Assert` (the message kind).
    Assert(&'static str),
    /// `Unreachable` was executed.
    Unreachable,
    /// The body unwound (`UnwindResume`, `UnwindTerminate`, `Throw`).
    Unwind,
    /// The body did something Rust calls undefined behaviour (out-of-bounds index, read of
    /// uninitialized storage, wrong-variant downcast, division by zero).
    Ub(&'static str),
    /// The body itself is malformed (a missing block, value, or type).
    Malformed(&'static str),
    /// The step budget ran out (a loop that never ends).
    Steps,
    /// A call hook declined.
    Hook(String),
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(what) => write!(f, "unsupported: {what}"),
            Self::Assert(what) => write!(f, "assertion failed: {what}"),
            Self::Unreachable => f.write_str("reached unreachable code"),
            Self::Unwind => f.write_str("unwound"),
            Self::Ub(what) => write!(f, "undefined behaviour: {what}"),
            Self::Malformed(what) => write!(f, "malformed body: {what}"),
            Self::Steps => f.write_str("step budget exhausted"),
            Self::Hook(what) => write!(f, "hook: {what}"),
        }
    }
}

/// One observable event, in program order.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A call to a Rust item with its arguments and returned value or trap.
    Call {
        callee: DirectItemRef,
        args: Vec<Value>,
        result: Result<Value, Trap>,
    },
    /// A call the interpreter answered itself without a hook (`Callee::Nop`, a runtime member).
    Builtin { name: String, args: Vec<Value> },
    /// A drop of a value of `ty`.
    Drop { ty: TypeId, value: Value },
}

/// What an evaluation observed, including effects before a trap.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub ret: Result<Value, Trap>,
    pub trace: Vec<Event>,
    /// The final value behind every pointer or slice argument, in argument order (see
    /// [`pointee_snapshots`]). A store through a `&mut` parameter the function never reads back
    /// is invisible in `ret` and `trace`; it is visible here.
    pub pointees: Vec<Value>,
}

/// The final value behind each pointer or slice in `args`, in order.
///
/// Arguments that are neither contribute nothing. A slice snapshots the value its backing buffer
/// location names — the whole backing array, not the window — so two runs are compared on
/// everything a store through the argument could have touched.
pub fn pointee_snapshots(args: &[Value], heap: &Heap) -> Vec<Value> {
    args.iter()
        .filter_map(|arg| match arg {
            Value::Ptr(location) => Some(location),
            Value::Slice { buf, .. } => Some(buf),
            _ => None,
        })
        .map(|location| {
            heap.read(location)
                .unwrap_or(Value::Opaque("unreadable pointee"))
        })
        .collect()
}

/// Erases run-local heap identity without erasing alias structure.
///
/// Two runs allocate independently, so a raw [`HeapId`] never compares equal across them. Mapping
/// every pointer to one fixed location would compare equal too often: it collapses distinct
/// objects into one, so pointer inequality, alias partitions, and two slices over different
/// backing buffers all stop being observable. Instead the first distinct object a run mentions
/// becomes `HeapId(0)`, the next `HeapId(1)`, and so on: run-local numbering is gone, the
/// partition survives. Paths into an object are kept as they are.
///
/// One `Canon` per run, fed the run's values in the same order on both sides. `NaN` becomes an
/// opaque on the way, since `NaN != NaN` would otherwise report agreement as a mismatch.
#[derive(Debug, Default)]
pub struct Canon {
    ids: BTreeMap<HeapId, u32>,
}

impl Canon {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn value(&mut self, value: &Value) -> Value {
        match value {
            Value::Float { value, .. } if value.is_nan() => Value::Opaque("NaN"),
            Value::Ptr(location) => Value::Ptr(self.location(location)),
            Value::Slice { buf, off, len } => Value::Slice {
                buf: self.location(buf),
                off: *off,
                len: *len,
            },
            Value::Aggregate {
                ty,
                variant,
                fields,
            } => Value::Aggregate {
                ty: *ty,
                variant: *variant,
                fields: fields.iter().map(|field| self.value(field)).collect(),
            },
            other => other.clone(),
        }
    }

    pub fn event(&mut self, event: &Event) -> Event {
        match event {
            Event::Call {
                callee,
                args,
                result,
            } => Event::Call {
                callee: *callee,
                args: args.iter().map(|arg| self.value(arg)).collect(),
                result: self.result(result),
            },
            Event::Builtin { name, args } => Event::Builtin {
                name: name.clone(),
                args: args.iter().map(|arg| self.value(arg)).collect(),
            },
            Event::Drop { ty, value } => Event::Drop {
                ty: *ty,
                value: self.value(value),
            },
        }
    }

    pub fn result(&mut self, result: &Result<Value, Trap>) -> Result<Value, Trap> {
        result
            .as_ref()
            .map(|value| self.value(value))
            .map_err(Clone::clone)
    }

    /// The whole outcome, result first and then the trace in program order.
    pub fn outcome(&mut self, outcome: &Outcome) -> Outcome {
        Outcome {
            ret: self.result(&outcome.ret),
            trace: outcome
                .trace
                .iter()
                .map(|event| self.event(event))
                .collect(),
            pointees: outcome
                .pointees
                .iter()
                .map(|pointee| self.value(pointee))
                .collect(),
        }
    }

    fn location(&mut self, location: &Location) -> Location {
        let next = self.ids.len() as u32;
        Location {
            heap: HeapId(*self.ids.entry(location.heap).or_insert(next)),
            path: location.path.clone(),
        }
    }
}

/// The call hook: what a `Callee::Direct` call returns, given its argument values.
pub type CallHook<'a> = dyn Fn(&DirectItemRef, Vec<Value>) -> Result<Value, Trap> + 'a;

/// The drop hook: run for every `Drop` of a value that has glue, after the event is traced.
pub type DropHook<'a> = dyn Fn(TypeId, &Value) -> Result<(), Trap> + 'a;

/// What the interpreters ask the caller for.
pub struct Hooks<'a> {
    pub call: &'a CallHook<'a>,
    pub drop: Option<&'a DropHook<'a>>,
    /// The number of ops/statements evaluated before [`Trap::Steps`].
    pub step_limit: u64,
}

impl<'a> Hooks<'a> {
    /// Hooks that reject every call, with a step budget of a million.
    pub fn new(call: &'a CallHook<'a>) -> Self {
        Self {
            call,
            drop: None,
            step_limit: 1_000_000,
        }
    }
}

/// A budget counter shared by both interpreters.
struct Budget {
    remaining: u64,
}

impl Budget {
    fn step(&mut self) -> Result<(), Trap> {
        if self.remaining == 0 {
            return Err(Trap::Steps);
        }
        self.remaining -= 1;
        Ok(())
    }
}
