//! Staleness validation of the published effects summaries (phase 5, D1).
//!
//! [`crate::analysis::effects::compute`] runs before the O group's passes; a pass that rewrites
//! a body can leave the published summary claiming things the new body no longer honors. This
//! module re-derives each item's summary with a second, independently written transfer -- one
//! item at a time, no call graph, no fixpoint, published summaries read at call sites -- and
//! requires `recomputed <= published` ([`crate::analysis::effects::Summary::le`]). A claim the
//! recompute cannot confirm demotes the item's summary to top
//! ([`crate::analysis::effects::Summaries::demote`]); every decision that then finds no usable
//! summary declines `op.missing-summary`.
//!
//! The recompute deliberately shares no code with the publisher beyond the lattice types: on an
//! unchanged body the two derivations agree (the summary fuzzer runs both over the whole fixture
//! corpus), and on a rewritten body any divergence fails toward top, never toward a stronger
//! claim. Demotion propagates: an item whose claims leaned on a demoted callee is revalidated
//! against the demoted map until nothing more falls.

// Wave 2's O group is the pipeline caller; today the env-gated lane report and the tests drive
// this.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::callgraph::Callgraph;
use crate::analysis::effects::{Escape, IndirectTargets, Purity, Ret, Summaries, Summary};
use crate::descriptor::{Descriptor, Fact, Slot};
use crate::item::JsName;
use crate::jir::decline::Decline;
use crate::jir::interns::Interns;
use crate::jir::interns::StrId;
use crate::jir::repr::{Func, OpKind, Proj, RTerm, Root, ValueId};
use crate::jir::semantic::{
    BinaryOp, CallSemantic, Callee, DropGlue, HostParameterClass, HostPayload,
};
use crate::lower::semantic_ops::{self, ReferenceRequirement, SignatureSlot};

/// One failed validation: the body whose published summary the recompute could not confirm.
#[derive(Clone, Debug)]
pub struct Violation {
    pub name: JsName,
    /// What the recomputed summary claimed that the published one does not admit.
    pub what: String,
}

/// Revalidates every published summary against the current bodies and demotes what fails,
/// repeating until demotion stops feeding on itself. Returns the violations in the order they
/// fell, deterministic in `(name)` order per round.
pub fn enforce(
    summaries: &mut Summaries,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    interns: &BTreeMap<JsName, &Interns>,
    graph: &Callgraph,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    loop {
        let mut demoted = Vec::new();
        for (name, func) in funcs {
            let Ok(func) = func else { continue };
            let Some(published) = summaries.usable(name) else {
                continue;
            };
            let recomputed = recompute(name, func, interns.get(name).copied(), graph, summaries);
            if !recomputed.le(published) {
                demoted.push(Violation {
                    name: name.clone(),
                    what: format!("recomputed {recomputed:?} not below published {published:?}"),
                });
            }
        }
        if demoted.is_empty() {
            return violations;
        }
        for violation in &demoted {
            summaries.demote(&violation.name);
        }
        violations.extend(demoted);
    }
}

/// Validates one item without demoting: `Err` carries what could not be confirmed.
pub fn validate(
    name: &JsName,
    func: &Func,
    interns: Option<&Interns>,
    graph: &Callgraph,
    summaries: &Summaries,
) -> Result<(), String> {
    let Some(published) = summaries.get(name) else {
        // Nothing published claims nothing; there is nothing to confirm.
        return Ok(());
    };
    let recomputed = recompute(name, func, interns, graph, summaries);
    if recomputed.le(published) {
        Ok(())
    } else {
        Err(format!(
            "recomputed {recomputed:?} not below published {published:?}"
        ))
    }
}

// ---------------------------------------------------------------------------------------------
// The independent recompute. Same rules as the publisher, second implementation: one flat pass
// structure, published summaries at call sites.
// ---------------------------------------------------------------------------------------------

/// Per-value reach facts: which parameters' caller-visible storage the value may be or contain
/// a pointer into, whether something unidentified besides, whether it is exactly a parameter's
/// value, and whose frame storage it points at.
#[derive(Clone, Default)]
struct Reach {
    params: BTreeSet<u16>,
    unknown: bool,
    /// `Some(Some(i))`: exactly parameter `i`'s value. `Some(None)`: not one. `None`: unflowed.
    exact: Option<Option<u16>>,
    /// The frame value (`Ok`) or cell (`Err`) whose storage this pointer names, if known.
    into: Option<Result<ValueId, u32>>,
    /// `Some(true)`: proven a pointer into this frame's own storage (a `Deref`-free borrow),
    /// through which nothing caller-visible is read or written. `None`: unflowed; joins AND.
    local: Option<bool>,
}

impl Reach {
    fn grow(&mut self, other: &Reach) -> bool {
        let before = (self.params.len(), self.unknown);
        self.params.extend(other.params.iter().copied());
        self.unknown |= other.unknown;
        before != (self.params.len(), self.unknown)
    }

    fn fresh(&self) -> bool {
        self.params.is_empty() && !self.unknown
    }
}

fn recompute(
    name: &JsName,
    func: &Func,
    interns: Option<&Interns>,
    graph: &Callgraph,
    summaries: &Summaries,
) -> Summary {
    let _ = name;
    let mut values = vec![Reach::default(); func.values.len()];
    let mut cells = vec![Reach::default(); func.cells.len()];
    for (index, param) in func.params.iter().enumerate() {
        let index = u16::try_from(index).unwrap_or(u16::MAX);
        let reach = &mut values[param.0 as usize];
        reach.params.insert(index);
        reach.exact = Some(Some(index));
        reach.local = Some(false);
    }

    // Reach fixpoint: repeat the flat pass until nothing grows.
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for op in &block.ops {
                changed |= reach_op(func, graph, interns, summaries, &mut values, &mut cells, op);
            }
            for (target, args) in block.term.edges() {
                for (position, arg) in args.iter().enumerate() {
                    let Some(param) = func.blocks[target.0 as usize].params.get(position) else {
                        continue;
                    };
                    let incoming = values[arg.0 as usize].clone();
                    let reach = &mut values[param.0 as usize];
                    changed |= reach.grow(&incoming);
                    let met = match (reach.exact, incoming.exact) {
                        (None, x) | (x, None) => x,
                        (a, b) if a == b => a,
                        _ => Some(None),
                    };
                    if met != reach.exact {
                        reach.exact = met;
                        changed = true;
                    }
                    if reach.into.is_none() && incoming.into.is_some() {
                        reach.into = incoming.into;
                        changed = true;
                    }
                    let local = match (reach.local, incoming.local) {
                        (None, x) | (x, None) => x,
                        (Some(a), Some(b)) => Some(a && b),
                    };
                    if local != reach.local {
                        reach.local = local;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Claim derivation: one pass over ops and terminators.
    let mut out = Summary::bottom(func.params.len());
    let mut ret_sites: Vec<Ret> = Vec::new();
    let mut escape = |out: &mut Summary, reach: &Reach, what: Escape| {
        for &param in &reach.params {
            if let Some(effects) = out.params.get_mut(param as usize) {
                effects.escape = effects.escape.join(what);
            }
        }
    };
    let mut written = |out: &mut Summary, reach: &Reach| {
        for &param in &reach.params {
            if let Some(effects) = out.params.get_mut(param as usize) {
                effects.reads_fields_only = false;
            }
        }
    };
    for block in &func.blocks {
        for op in &block.ops {
            claim_op(
                func,
                graph,
                interns,
                summaries,
                &values,
                &cells,
                op,
                &mut out,
                &mut escape,
                &mut written,
            );
        }
        match &block.term {
            RTerm::Return(v) => ret_sites.push(match v {
                None => Ret::Fresh,
                Some(v) => {
                    let reach = &values[v.0 as usize];
                    escape(&mut out, reach, Escape::Return);
                    match reach.exact {
                        Some(Some(index)) => Ret::Param(index),
                        _ if reach.fresh() => Ret::Fresh,
                        _ => Ret::Unknown,
                    }
                }
            }),
            RTerm::Throw(v) => {
                out.may_unwind = true;
                if let Some(v) = v {
                    escape(&mut out, &values[v.0 as usize], Escape::Unknown);
                }
            }
            RTerm::Invoke { .. } => out.may_unwind = true,
            RTerm::Goto(..) | RTerm::Switch { .. } | RTerm::Unreachable => {}
        }
    }
    out.ret = ret_sites
        .into_iter()
        .reduce(Ret::join)
        .unwrap_or(Ret::Fresh);
    out
}

/// Grows the reach facts by one op; whether anything changed.
#[allow(clippy::too_many_arguments)]
fn reach_op(
    func: &Func,
    graph: &Callgraph,
    interns: Option<&Interns>,
    summaries: &Summaries,
    values: &mut [Reach],
    cells: &mut [Reach],
    op: &crate::jir::repr::Op,
) -> bool {
    let value = |values: &[Reach], v: &ValueId| values[v.0 as usize].clone();
    let union = |values: &[Reach], list: &[ValueId]| {
        let mut all = Reach::default();
        for v in list {
            all.grow(&values[v.0 as usize]);
        }
        all
    };
    let opaque = |values: &[Reach], list: &[ValueId]| {
        let mut all = union(values, list);
        all.unknown = true;
        all
    };
    match &op.kind {
        OpKind::SetField { base, v, .. } => {
            let incoming = value(values, v);
            values[base.0 as usize].grow(&incoming)
        }
        OpKind::CellStore { cell, v } => {
            let incoming = value(values, v);
            cells[cell.0 as usize].grow(&incoming)
        }
        OpKind::StoreThrough { ptr, v } => {
            let incoming = value(values, v);
            match values[ptr.0 as usize].into {
                Some(Ok(w)) => values[w.0 as usize].grow(&incoming),
                Some(Err(c)) => cells[c as usize].grow(&incoming),
                None => false,
            }
        }
        _ => {
            let Some(result) = op.result else {
                return false;
            };
            let not_local = |mut reach: Reach| {
                reach.local = Some(false);
                reach
            };
            let (mut reach, exact) = match &op.kind {
                OpKind::Const(_)
                | OpKind::CellAlloc(_)
                | OpKind::Discriminant(_)
                | OpKind::Unary { .. }
                | OpKind::Zombie(_) => (not_local(Reach::default()), Some(None)),
                OpKind::Copy(v) | OpKind::Move(v) => {
                    let reach = value(values, v);
                    let exact = reach.exact;
                    (reach, exact)
                }
                // The frame-local proof survives the pointer-preserving ops only.
                OpKind::Cast { v, .. }
                | OpKind::Materialize(v)
                | OpKind::Convert { v, .. }
                | OpKind::SliceView { ptr: v, .. } => (value(values, v), Some(None)),
                OpKind::Field { base: v, .. } | OpKind::Repeat { v, .. } => {
                    (not_local(value(values, v)), Some(None))
                }
                OpKind::Deref(v) => {
                    let mut reach = not_local(value(values, v));
                    reach.into = None;
                    (reach, Some(None))
                }
                OpKind::Aggregate { fields, .. } => (not_local(union(values, fields)), Some(None)),
                OpKind::BorrowView { root, path, .. } | OpKind::RawView { root, path, .. } => {
                    let mut reach = match root {
                        Root::Value(v) => value(values, v),
                        Root::Cell(c) => cells[c.0 as usize].clone(),
                    };
                    reach.into = Some(match root {
                        Root::Value(v) => Ok(*v),
                        Root::Cell(c) => Err(c.0),
                    });
                    reach.local = Some(!path.iter().any(|proj| matches!(proj, Proj::Deref)));
                    (reach, Some(None))
                }
                OpKind::CellLoad(cell) => {
                    let mut reach = not_local(cells[cell.0 as usize].clone());
                    reach.into = Some(Err(cell.0));
                    (reach, Some(None))
                }
                OpKind::Binary { op, l, .. } => {
                    if matches!(op, BinaryOp::Offset) {
                        (value(values, l), Some(None))
                    } else {
                        (not_local(Reach::default()), Some(None))
                    }
                }
                OpKind::Call {
                    callee,
                    semantic,
                    host,
                    args,
                    ..
                } => {
                    let (reach, exact) = call_result_reach(
                        func, graph, interns, summaries, values, op.result, callee, semantic, host,
                        args,
                    );
                    (not_local(reach), exact)
                }
                OpKind::Intrinsic { args, .. } | OpKind::Host { args, .. } => {
                    (not_local(opaque(values, args)), Some(None))
                }
                OpKind::SpillRead(_) => (
                    not_local(Reach {
                        unknown: true,
                        ..Reach::default()
                    }),
                    Some(None),
                ),
                OpKind::SetField { .. }
                | OpKind::SetDiscriminant { .. }
                | OpKind::CellStore { .. }
                | OpKind::StoreThrough { .. }
                | OpKind::Drop { .. }
                | OpKind::Assert { .. }
                | OpKind::SpillWrite { .. } => (not_local(Reach::default()), Some(None)),
            };
            let _ = func;
            let slot = &mut values[result.0 as usize];
            let into = reach.into.take();
            let mut changed = slot.grow(&reach);
            let met = match (slot.exact, exact) {
                (None, x) | (x, None) => x,
                (a, b) if a == b => a,
                _ => Some(None),
            };
            if met != slot.exact {
                slot.exact = met;
                changed = true;
            }
            if slot.into.is_none() && into.is_some() {
                slot.into = into;
                changed = true;
            }
            let local = match (slot.local, reach.local) {
                (None, x) | (x, None) => x,
                (Some(a), Some(b)) => Some(a && b),
            };
            if local != slot.local {
                slot.local = local;
                changed = true;
            }
            changed
        }
    }
}

/// The reach of a call's result value.
#[allow(clippy::too_many_arguments)]
fn call_result_reach(
    func: &Func,
    graph: &Callgraph,
    interns: Option<&Interns>,
    summaries: &Summaries,
    values: &[Reach],
    result: Option<ValueId>,
    callee: &Callee,
    semantic: &CallSemantic,
    host: &Option<HostPayload>,
    args: &[ValueId],
) -> (Reach, Option<Option<u16>>) {
    let opaque = || {
        let mut all = Reach::default();
        for arg in args {
            all.grow(&values[arg.0 as usize]);
        }
        all.unknown = true;
        (all, Some(None))
    };
    if let CallSemantic::Standard(operation) = semantic {
        let types = interns.map(|interns| &interns.types);
        if semantic_ops::call_effects_entry(*operation, types, func, result, args).is_some() {
            let mut all = Reach::default();
            for arg in args {
                all.grow(&values[arg.0 as usize]);
            }
            return (all, Some(None));
        }
    }
    if matches!(semantic, CallSemantic::GlobalAllocator) {
        return (Reach::default(), Some(None));
    }
    if host.is_some()
        || matches!(
            semantic,
            CallSemantic::Host | CallSemantic::ViewAbi | CallSemantic::RawJavaScript
        )
    {
        return opaque();
    }
    let callee_summary = |link_name: &StrId| {
        let name = interns.and_then(|interns| interns.str(*link_name))?;
        summaries.get(&JsName::new(name.to_owned()))
    };
    match callee {
        Callee::Nop => (Reach::default(), Some(None)),
        Callee::Direct { link_name, .. } => match callee_summary(link_name).map(|s| s.ret) {
            Some(Ret::Fresh) => (Reach::default(), Some(None)),
            Some(Ret::Param(index)) => match args.get(index as usize) {
                Some(arg) => {
                    let mut reach = values[arg.0 as usize].clone();
                    reach.into = None;
                    let exact = reach.exact;
                    (reach, exact)
                }
                None => opaque(),
            },
            Some(Ret::Unknown) | None => opaque(),
        },
        Callee::Indirect { .. }
        | Callee::Virtual { .. }
        | Callee::Foreign { .. }
        | Callee::Runtime { .. }
        | Callee::Intrinsic { .. } => {
            let _ = graph;
            opaque()
        }
    }
}

/// Applies one op's claims to the recomputed summary.
#[allow(clippy::too_many_arguments)]
fn claim_op(
    func: &Func,
    graph: &Callgraph,
    interns: Option<&Interns>,
    summaries: &Summaries,
    values: &[Reach],
    cells: &[Reach],
    op: &crate::jir::repr::Op,
    out: &mut Summary,
    escape: &mut impl FnMut(&mut Summary, &Reach, Escape),
    written: &mut impl FnMut(&mut Summary, &Reach),
) {
    let purity = |out: &mut Summary, level: Purity| {
        out.purity = out.purity.max(level);
    };
    let opaque_args = |out: &mut Summary,
                       escape: &mut dyn FnMut(&mut Summary, &Reach, Escape),
                       written: &mut dyn FnMut(&mut Summary, &Reach),
                       args: &[ValueId]| {
        out.purity = Purity::Effect;
        out.may_unwind = true;
        out.allocates = true;
        for arg in args {
            escape(out, &values[arg.0 as usize], Escape::Unknown);
            written(out, &values[arg.0 as usize]);
        }
    };
    let direct = |out: &mut Summary,
                  escape: &mut dyn FnMut(&mut Summary, &Reach, Escape),
                  written: &mut dyn FnMut(&mut Summary, &Reach),
                  name: Option<JsName>,
                  args: &[ValueId]| {
        let known = name
            .and_then(|name| summaries.get(&name))
            .filter(|summary| summary.params.len() == args.len());
        match known {
            Some(summary) => {
                out.purity = out.purity.max(summary.purity);
                out.may_unwind |= summary.may_unwind;
                out.allocates |= summary.allocates;
                for (arg, param) in args.iter().zip(summary.params.clone()) {
                    escape(out, &values[arg.0 as usize], param.escape);
                    if !param.reads_fields_only {
                        written(out, &values[arg.0 as usize]);
                    }
                }
            }
            None => {
                out.purity = Purity::Effect;
                out.may_unwind = true;
                out.allocates = true;
                for arg in args {
                    escape(out, &values[arg.0 as usize], Escape::Unknown);
                    written(out, &values[arg.0 as usize]);
                }
            }
        }
    };
    match &op.kind {
        OpKind::Deref(ptr) => {
            if values[ptr.0 as usize].local != Some(true) {
                purity(out, Purity::Read);
            }
        }
        OpKind::StoreThrough { ptr, v } => {
            if values[ptr.0 as usize].local != Some(true) {
                purity(out, Purity::Effect);
                written(out, &values[ptr.0 as usize]);
            }
            escape(out, &values[v.0 as usize], Escape::Heap);
        }
        OpKind::Assert { .. } => out.may_unwind = true,
        OpKind::Zombie(_) => {
            purity(out, Purity::Effect);
            out.may_unwind = true;
        }
        OpKind::SpillRead(_) => purity(out, Purity::Read),
        OpKind::SpillWrite { v, .. } => {
            purity(out, Purity::Effect);
            escape(out, &values[v.0 as usize], Escape::Unknown);
        }
        OpKind::Intrinsic { args, .. } | OpKind::Host { args, .. } => {
            opaque_args(out, escape, written, args);
        }
        OpKind::Drop { root, glue, .. } => {
            let reach = match root {
                Root::Value(v) => values[v.0 as usize].clone(),
                Root::Cell(c) => cells[c.0 as usize].clone(),
            };
            match glue {
                DropGlue::None => {}
                DropGlue::Direct { link_name, .. } => {
                    written(out, &reach);
                    let resolved = interns
                        .and_then(|interns| interns.str(*link_name))
                        .map(|name| JsName::new(name.to_owned()));
                    match resolved.as_ref().and_then(|name| summaries.get(name)) {
                        Some(summary) => {
                            out.purity = out.purity.max(summary.purity);
                            out.may_unwind |= summary.may_unwind;
                            out.allocates |= summary.allocates;
                            let what = summary
                                .params
                                .first()
                                .map_or(Escape::Unknown, |param| param.escape);
                            escape(out, &reach, what);
                        }
                        None => {
                            out.purity = Purity::Effect;
                            out.may_unwind = true;
                            out.allocates = true;
                            escape(out, &reach, Escape::Unknown);
                        }
                    }
                }
                DropGlue::Dynamic => {
                    out.purity = Purity::Effect;
                    out.may_unwind = true;
                    out.allocates = true;
                    written(out, &reach);
                    escape(out, &reach, Escape::Unknown);
                }
            }
        }
        OpKind::Call {
            callee,
            semantic,
            host,
            args,
            ..
        } => {
            if let CallSemantic::Standard(operation) = semantic {
                let types = interns.map(|interns| &interns.types);
                if let Some(entry) =
                    semantic_ops::call_effects_entry(*operation, types, func, op.result, args)
                {
                    purity(
                        out,
                        if entry.effects.mutates {
                            Purity::Effect
                        } else {
                            Purity::Read
                        },
                    );
                    out.may_unwind |= entry.effects.can_throw;
                    out.allocates = true;
                    for slot in entry.slots {
                        if slot.reference == ReferenceRequirement::Mutable {
                            if let SignatureSlot::Argument(index) = slot.slot {
                                if let Some(arg) = args.get(index as usize) {
                                    written(out, &values[arg.0 as usize]);
                                }
                            }
                        }
                    }
                    if entry.effects.mutates {
                        for arg in args {
                            escape(out, &values[arg.0 as usize], Escape::Heap);
                        }
                    }
                    return;
                }
            }
            if matches!(semantic, CallSemantic::GlobalAllocator) {
                purity(out, Purity::Effect);
                out.may_unwind = true;
                out.allocates = true;
                return;
            }
            if host.is_some()
                || matches!(
                    semantic,
                    CallSemantic::Host | CallSemantic::ViewAbi | CallSemantic::RawJavaScript
                )
            {
                let descriptor = match host {
                    Some(HostPayload::Extern(bytes)) => interns
                        .and_then(|interns| interns.bytes.get(bytes.0 as usize))
                        .and_then(|bytes| std::str::from_utf8(bytes).ok())
                        .and_then(|text| Descriptor::decode(text).ok()),
                    _ => None,
                };
                purity(out, Purity::Effect);
                out.allocates = true;
                match descriptor {
                    Some(descriptor) => {
                        out.may_unwind |= descriptor.fallible;
                        for (index, arg) in args.iter().enumerate() {
                            let reach = &values[arg.0 as usize];
                            match descriptor.slot(Slot::Parameter(index as u32)) {
                                Some(facts) => {
                                    out.may_unwind |= facts.has(Fact::Throws);
                                    if facts.has(Fact::Mutates) {
                                        written(out, reach);
                                    }
                                    if facts.has(Fact::Retains)
                                        || facts.has(Fact::Owns)
                                        || facts.has(Fact::Identity)
                                        || facts.has(Fact::Bytes)
                                    {
                                        escape(out, reach, Escape::Host);
                                    }
                                }
                                None => {
                                    let ty = func.values[arg.0 as usize].ty;
                                    let pins = interns.is_none_or(|interns| {
                                        HostParameterClass::of(ty, &interns.types)
                                            == HostParameterClass::Pin
                                    });
                                    if pins {
                                        written(out, reach);
                                        escape(out, reach, Escape::Host);
                                    }
                                }
                            }
                        }
                    }
                    None => {
                        out.may_unwind = true;
                        for arg in args {
                            written(out, &values[arg.0 as usize]);
                            escape(out, &values[arg.0 as usize], Escape::Unknown);
                        }
                    }
                }
                return;
            }
            match callee {
                Callee::Nop => {}
                Callee::Direct { link_name, .. } => {
                    let name = interns
                        .and_then(|interns| interns.str(*link_name))
                        .map(|name| JsName::new(name.to_owned()));
                    match name {
                        Some(name) => direct(out, escape, written, Some(name), args),
                        None => opaque_args(out, escape, written, args),
                    }
                }
                Callee::Indirect { signature } => match graph.fn_pointer_targets.get(signature) {
                    Some(targets) if !targets.is_empty() => {
                        out.indirect_targets = std::mem::replace(
                            &mut out.indirect_targets,
                            IndirectTargets::Unbounded,
                        )
                        .join(&IndirectTargets::Bounded(targets.clone()));
                        for target in targets {
                            match graph.by_item.get(target) {
                                Some(names) => {
                                    for name in names {
                                        direct(out, escape, written, Some(name.clone()), args);
                                    }
                                }
                                None => opaque_args(out, escape, written, args),
                            }
                        }
                    }
                    _ => {
                        out.indirect_targets = IndirectTargets::Unbounded;
                        opaque_args(out, escape, written, args);
                    }
                },
                Callee::Virtual { .. } => {
                    out.indirect_targets = IndirectTargets::Unbounded;
                    opaque_args(out, escape, written, args);
                }
                Callee::Foreign { .. } | Callee::Runtime { .. } | Callee::Intrinsic { .. } => {
                    opaque_args(out, escape, written, args);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{callgraph, effects};
    use crate::jir::interns::StrId;
    use crate::jir::origin::OriginId;
    use crate::jir::repr::{Func, FuncBuilder, OpKind, RTerm};
    use crate::jir::semantic::{AbiSignature, ConstTree, DefinitionId, ItemId, Signature, TypeId};

    const O: OriginId = OriginId(0);
    const TY: TypeId = TypeId(1);

    fn id(n: u64) -> ItemId {
        ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash: n,
            },
            instantiation: 0,
        }
    }

    /// `fn f(p) { *p = (); }`
    fn writes_param(item: ItemId) -> Func {
        let mut b = FuncBuilder::new(
            item,
            Signature {
                arguments: vec![TY],
                result: TY,
                abi: AbiSignature::default(),
            },
        );
        let p = b.param(TY, O);
        let c = b.op(FuncBuilder::ENTRY, OpKind::Const(ConstTree::Unit), TY, O);
        b.effect(FuncBuilder::ENTRY, OpKind::StoreThrough { ptr: p, v: c }, O);
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        b.finish()
    }

    /// `fn f(p) { g(p); }`
    fn calls(item: ItemId, callee: StrId) -> Func {
        let mut b = FuncBuilder::new(
            item,
            Signature {
                arguments: vec![TY],
                result: TY,
                abi: AbiSignature::default(),
            },
        );
        let p = b.param(TY, O);
        b.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: id(0),
                    link_name: callee,
                },
                function: None,
                args: vec![p],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        b.finish()
    }

    struct Prog {
        interns: Interns,
        funcs: BTreeMap<JsName, Result<Func, Decline>>,
    }

    impl Prog {
        fn new(items: Vec<(&str, Func)>) -> Prog {
            Prog {
                interns: Interns::new(),
                funcs: items
                    .into_iter()
                    .map(|(name, func)| (JsName::new(name.to_owned()), Ok(func)))
                    .collect(),
            }
        }

        fn intern(interns: &mut Interns, name: &str) -> StrId {
            interns.strings.push(name.to_owned());
            StrId((interns.strings.len() - 1) as u32)
        }

        fn interns_map(&self) -> BTreeMap<JsName, &Interns> {
            self.funcs
                .keys()
                .map(|name| (name.clone(), &self.interns))
                .collect()
        }
    }

    #[test]
    fn an_honest_computation_validates_everywhere() {
        let mut interns = Interns::new();
        let g = Prog::intern(&mut interns, "g");
        let mut prog = Prog::new(vec![("g", writes_param(id(1))), ("f", calls(id(2), g))]);
        prog.interns = interns;
        let interns = prog.interns_map();
        let graph = callgraph::build(&prog.funcs, &interns);
        let mut summaries = effects::compute(&prog.funcs, &interns, &graph);
        let violations = enforce(&mut summaries, &prog.funcs, &interns, &graph);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn an_overclaim_is_caught_and_demoted() {
        let prog = Prog::new(vec![("g", writes_param(id(1)))]);
        let interns = prog.interns_map();
        let graph = callgraph::build(&prog.funcs, &interns);
        let mut summaries = effects::compute(&prog.funcs, &interns, &graph);
        // Corrupt the published claim: the body writes through its parameter.
        let name = JsName::new("g".to_owned());
        summaries.insert(name.clone(), effects::Summary::bottom(1));
        let func = prog.funcs.get(&name).unwrap().as_ref().unwrap();
        assert!(validate(&name, func, Some(&prog.interns), &graph, &summaries).is_err());
        let violations = enforce(&mut summaries, &prog.funcs, &interns, &graph);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].name, name);
        assert!(summaries.usable(&name).is_none(), "demoted to top");
        assert!(summaries.get(&name).is_some_and(effects::Summary::is_top));
    }

    #[test]
    fn demotion_propagates_to_the_items_that_leaned_on_the_claim() {
        // f calls g. g's summary is corrupted to Const; f's is recomputed from that lie, so f
        // holds Const too. Enforcing must fell g in round one and f in round two.
        let mut interns_table = Interns::new();
        let g = Prog::intern(&mut interns_table, "g");
        let mut prog = Prog::new(vec![("g", writes_param(id(1))), ("f", calls(id(2), g))]);
        prog.interns = interns_table;
        let interns = prog.interns_map();
        let graph = callgraph::build(&prog.funcs, &interns);
        let mut summaries = effects::compute(&prog.funcs, &interns, &graph);
        let g_name = JsName::new("g".to_owned());
        let f_name = JsName::new("f".to_owned());
        summaries.insert(g_name.clone(), effects::Summary::bottom(1));
        summaries.insert(f_name.clone(), effects::Summary::bottom(1));
        let violations = enforce(&mut summaries, &prog.funcs, &interns, &graph);
        // g falls against its body; f validated against the pre-demotion g in the same round,
        // then falls against the demoted g.
        assert_eq!(violations.len(), 2, "{violations:?}");
        assert!(summaries.usable(&g_name).is_none());
        assert!(summaries.usable(&f_name).is_none());
    }
}
