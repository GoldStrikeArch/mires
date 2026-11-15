//! Out-of-SSA and expression fusion: the [`Plan`] for one JIR-R function.
//!
//! Three decisions are made here, in this order, and nothing in the function itself changes:
//!
//! 1. **Which values are spelled inline.** v1's expression queue (`queue.rs`) held an assignment
//!    back and substituted it at the one place that read it, guarded by a purity ladder that says
//!    what an expression may be moved past. Over SSA the same rule is a predicate on values: a
//!    value is inline when it is defined by an op, used exactly once, in the block that defines it,
//!    and every op between the definition and the use commutes with it — two expressions commute
//!    when one is `Const`, or both are `Read` ([`Purity`]). Substituting an expression into its use
//!    moves its evaluation later; nothing here ever moves one earlier, so that is the whole
//!    correctness argument, exactly as in v1. An op's purity is its own class joined with the
//!    purity of every operand it inlines, since those move with it.
//! 2. **Block arguments become assignments.** Every edge `(a, b)` whose target has parameters gets
//!    the parallel copy `params(b) = args(a → b)`, sequentialized: a copy whose target nothing else
//!    on the edge still reads goes first, and a cycle is broken through a temporary (`$c{n}`). A
//!    source spelled inline reads the names its expression mentions, so such a copy waits for
//!    those; if that would need a temporary the source is declared instead.
//! 3. **Names.** v1 `names.rs`'s rules over the values: a value that defines a MIR local with a
//!    user name is called that (sanitized, kept out of the compiler's `$` and `_N` namespaces and
//!    the reserved words); the other values of that local get `$1`, `$2` suffixes; a value without
//!    a user name is `_N` after its MIR local when known and `$v{n}` otherwise; cells follow the
//!    same rule; every name is unique per function.
//!
//! The plan is deterministic: every map is a `BTreeMap`, every walk is in block and op order.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::evaluation::OperandEvaluation;

use crate::jir::interns::Interns;
use crate::jir::origin::{OriginId, OriginTable};
use crate::jir::repr::{CellId, Dominators, Func, OpKind, Proj, RBlockId, RTerm, Root, ValueId};
use crate::jir::semantic::{
    BinaryOp, CastKind, ConstTree, GlobalReference, HostPayload, TypeId, UnaryOp,
};
use crate::jir::types::{EnumRepr, PointerForm, TypeKind, TypeTable};
use crate::jsast;
use crate::opts::{NameStyle, Opts};

use super::plan::{CopyOperand, EdgeCopy, Plan, RScrut, CALLER_LOCATION_NAME, NO_ORIGIN};

/// How freely an expression may be moved (v1 `queue.rs::Purity`).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Purity {
    /// Depends on nothing that can change, changes nothing.
    Const,
    /// Reads memory that something else could write.
    Read,
    /// Writes, calls, or anything the pass cannot see into.
    Effect,
}

/// Whether evaluating `a` and `b` in either order gives the same result.
pub fn commutes(a: Purity, b: Purity) -> bool {
    a == Purity::Const || b == Purity::Const || (a == Purity::Read && b == Purity::Read)
}

/// The purity of one op's own work, its operands aside (a value name reads as `Const`: SSA
/// values are assigned once, and a block parameter only by the copies at a block's end).
///
/// Classified over the JIR-R op the way v1 classified the JavaScript it produced: what v1's
/// lowering of each op spells decides the class. Errs towards `Effect`.
pub fn op_purity(kind: &OpKind) -> Purity {
    match kind {
        OpKind::Const(_) | OpKind::Move(_) | OpKind::Aggregate { .. } => Purity::Const,
        // `Object.assign({}, v)` / a structural clone reads memory something else may write.
        OpKind::Copy(_) => Purity::Read,
        // `new Array(n).fill(v)`: a fresh array and nothing else.
        OpKind::Repeat { .. } => Purity::Read,
        // A spill register is written by the call that returns into it, so a read may not move
        // across another call — which is exactly what `Read` forbids against `Effect`.
        OpKind::SpillRead(_) => Purity::Read,
        OpKind::Field { .. }
        | OpKind::Discriminant(_)
        | OpKind::CellLoad(_)
        | OpKind::BorrowView { .. }
        | OpKind::RawView { .. }
        | OpKind::SliceView { .. }
        | OpKind::Deref(_) => Purity::Read,
        // Arithmetic over names; `Offset` builds a pointer record out of another (`__rt.offset`).
        OpKind::Binary { op, .. } => match op {
            crate::jir::semantic::BinaryOp::Offset => Purity::Read,
            _ => Purity::Const,
        },
        OpKind::Unary { .. } => Purity::Const,
        OpKind::Cast { kind, .. } => match kind {
            CastKind::IntToInt
            | CastKind::FloatToFloat
            | CastKind::IntToFloat
            | CastKind::Subtype => Purity::Const,
            // `__rt.f2i`, the pointer shims: functions of their arguments over memory.
            CastKind::FloatToInt
            | CastKind::PointerExposeProvenance
            | CastKind::PointerWithExposedProvenance
            | CastKind::PointerCoercion
            | CastKind::PtrToPtr
            | CastKind::FnPtrToPtr
            | CastKind::Transmute => Purity::Read,
        },
        OpKind::SetField { .. }
        | OpKind::SetDiscriminant { .. }
        | OpKind::CellAlloc(_)
        | OpKind::CellStore { .. }
        | OpKind::StoreThrough { .. }
        | OpKind::Call { .. }
        | OpKind::Intrinsic { .. }
        | OpKind::Host { .. }
        | OpKind::Drop { .. }
        | OpKind::Assert { .. }
        | OpKind::Materialize(_)
        | OpKind::Convert { .. }
        | OpKind::Zombie(_)
        | OpKind::SpillWrite { .. } => Purity::Effect,
    }
}

/// Where a value is used: the block, and a position that orders it against the block's ops (an
/// op's index; `ops.len()` for the terminator; `ops.len() + 1` for an edge copy, which runs
/// after the terminator's scrutinee is evaluated).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Use {
    block: RBlockId,
    position: usize,
    /// The use is an operand of an assert message, which is evaluated only when the assertion
    /// fails: nothing may be moved into it.
    conditional: bool,
    /// The use is an edge argument (a copy source).
    edge: Option<RBlockId>,
}

/// Where a value is defined.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Def {
    Param(RBlockId),
    Op(RBlockId, usize),
}

/// Computes the plan of `func`.
///
/// `origins` names the values after the MIR locals they came from (`None`: numbered names);
/// `opts.names` picks readable or mangled names as v1's `js-names` did.
pub fn plan(
    func: &Func,
    origins: Option<&OriginTable>,
    table: &TypeTable,
    interns: &Interns,
    opts: &Opts,
) -> Plan {
    let mut planner = Planner::new(func, origins, table, interns, opts);
    planner.collect_uses();
    // Materialize operands before the protected operation. Fusion must not move
    // an earlier effect or property read inside a different exception boundary.
    let has_invoke = func.has_invoke();
    if !has_invoke {
        planner.decide_inline();
        planner.physicalize_slices();
    } else {
        planner.inline_adjacent_place_views();
        if opts.optimizer == rcgjs_object::opts::OptimizerMode::On {
            planner.decide_inline();
        }
    }
    planner.pin_host_environments();
    planner.name();
    planner.copies();
    planner.name_invokes();
    planner.scrutinees();
    planner.finish()
}

struct Planner<'a> {
    func: &'a Func,
    origins: Option<&'a OriginTable>,
    table: &'a TypeTable,
    interns: &'a Interns,
    opts: &'a Opts,
    defs: Vec<Option<Def>>,
    uses: Vec<Vec<Use>>,
    /// The predecessors of each block, in block order.
    preds: Vec<Vec<RBlockId>>,
    /// The effective purity of each op result (own class joined with its inline operands').
    purity: BTreeMap<ValueId, Purity>,
    plan: Plan,
    /// The names taken so far.
    taken: BTreeSet<String>,
    /// Whether every interned string counts as taken too (set by [`Planner::name_invokes`]).
    reserve_interns: bool,
    /// The `_N` spellings reserved for the first value of each MIR local.
    reserved_numbers: BTreeSet<String>,
}

impl<'a> Planner<'a> {
    fn new(
        func: &'a Func,
        origins: Option<&'a OriginTable>,
        table: &'a TypeTable,
        interns: &'a Interns,
        opts: &'a Opts,
    ) -> Self {
        let n = func.values.len();
        Planner {
            func,
            origins,
            table,
            interns,
            opts,
            defs: vec![None; n],
            uses: vec![Vec::new(); n],
            preds: {
                let mut preds: Vec<Vec<RBlockId>> = vec![Vec::new(); func.blocks.len()];
                for block in &func.blocks {
                    for successor in block.term.successors() {
                        if let Some(list) = preds.get_mut(successor.0 as usize) {
                            if !list.contains(&block.id) {
                                list.push(block.id);
                            }
                        }
                    }
                }
                preds
            },
            purity: BTreeMap::new(),
            plan: Plan::default(),
            taken: BTreeSet::new(),
            reserve_interns: false,
            reserved_numbers: BTreeSet::new(),
        }
    }

    // -- uses ---------------------------------------------------------------------------------

    fn add_use(&mut self, v: ValueId, use_: Use) {
        if let Some(uses) = self.uses.get_mut(v.0 as usize) {
            uses.push(use_);
        }
    }

    fn collect_uses(&mut self) {
        for block in &self.func.blocks {
            for param in &block.params {
                if let Some(slot) = self.defs.get_mut(param.0 as usize) {
                    *slot = Some(Def::Param(block.id));
                }
            }
            for (index, op) in block.ops.iter().enumerate() {
                if let Some(result) = op.result {
                    if let Some(slot) = self.defs.get_mut(result.0 as usize) {
                        *slot = Some(Def::Op(block.id, index));
                    }
                }
                let plain = Use {
                    block: block.id,
                    position: index,
                    conditional: false,
                    edge: None,
                };
                match &op.kind {
                    OpKind::Assert { cond, .. } => {
                        // The condition is evaluated unconditionally; the message operands only
                        // when the assertion fails.
                        self.add_use(*cond, plain);
                        for v in op.kind.uses().into_iter().skip(1) {
                            self.add_use(
                                v,
                                Use {
                                    conditional: true,
                                    ..plain
                                },
                            );
                        }
                    }
                    kind => {
                        for v in kind.uses() {
                            self.add_use(v, plain);
                        }
                    }
                }
            }
            let terminator = Use {
                block: block.id,
                position: block.ops.len(),
                conditional: false,
                edge: None,
            };
            match &block.term {
                RTerm::Goto(target, args) => {
                    for v in args {
                        self.add_use(
                            *v,
                            Use {
                                position: block.ops.len() + 1,
                                edge: Some(*target),
                                ..terminator
                            },
                        );
                    }
                }
                RTerm::Switch {
                    scrut,
                    cases,
                    default,
                    ..
                } => {
                    self.add_use(*scrut, terminator);
                    let mut edges: Vec<(RBlockId, &Vec<ValueId>)> = cases
                        .iter()
                        .map(|(_, target, args)| (*target, args))
                        .collect();
                    edges.push((default.0, &default.1));
                    let mut seen = BTreeSet::new();
                    for (target, args) in edges {
                        // The same target from several cases carries the same arguments: one
                        // copy run, one use.
                        if !seen.insert(target) {
                            continue;
                        }
                        for v in args {
                            self.add_use(
                                *v,
                                Use {
                                    position: block.ops.len() + 1,
                                    edge: Some(target),
                                    ..terminator
                                },
                            );
                        }
                    }
                }
                RTerm::Invoke { normal, unwind } => {
                    for (target, args) in [normal, unwind] {
                        for v in args {
                            self.add_use(
                                *v,
                                Use {
                                    position: block.ops.len() + 1,
                                    edge: Some(*target),
                                    ..terminator
                                },
                            );
                        }
                    }
                }
                RTerm::Return(v) | RTerm::Throw(v) => {
                    if let Some(v) = v {
                        self.add_use(*v, terminator);
                    }
                }
                RTerm::Unreachable => {}
            }
        }
    }

    /// Selects fat-slice values that can stay as three scalar JavaScript lanes.
    ///
    /// This is deliberately narrower than general escape analysis. A slice may cross loop and
    /// merge edges and may be inspected for its length or pointee, but any call, return, cast, or
    /// borrow of the pointer value itself keeps the canonical record. Direct-call slice ABI
    /// parameters are handled separately by the ABI lowerer. The set is closed over copies and
    /// block edges, so a record is rebuilt at a real boundary, never by accident in the middle of
    /// a physical chain.
    fn physicalize_slices(&mut self) {
        let is_slice = |v: ValueId| {
            self.func.ty(v).is_some_and(|ty| {
                self.table.pointer_form(ty) == Some(PointerForm::FatSlice)
                    || matches!(
                        self.table.get(self.table.peeled(ty)).map(|d| &d.kind),
                        Some(TypeKind::Slice(_))
                    )
            })
        };
        let doms = Dominators::of(self.func);
        // Only loop-carried slices are selected. Scalarizing every short-lived local slice is
        // allocation-neutral and usually byte-negative; a back edge is where the same record
        // would otherwise be rebuilt once per iteration.
        let mut candidates = BTreeSet::new();
        let mut work = Vec::new();
        let mut headers = BTreeSet::new();
        for block in &self.func.blocks {
            for (position, param) in block.params.iter().enumerate() {
                if !is_slice(*param) {
                    continue;
                }
                let carried = self.func.blocks.iter().any(|pred| {
                    doms.dominates(block.id, pred.id)
                        && pred.term.edges().into_iter().any(|(target, args)| {
                            target == block.id
                                && args.get(position).is_some_and(|arg| *arg != *param)
                        })
                });
                if carried && candidates.insert(*param) {
                    headers.insert(block.id);
                    work.push(*param);
                }
            }
        }
        let mut predecessors: Vec<Vec<RBlockId>> = vec![Vec::new(); self.func.blocks.len()];
        for block in &self.func.blocks {
            for successor in block.term.successors() {
                predecessors[successor.0 as usize].push(block.id);
            }
        }
        let mut loop_blocks = headers.clone();
        let mut loop_work = Vec::new();
        for block in &self.func.blocks {
            for successor in block.term.successors() {
                if headers.contains(&successor) && doms.dominates(successor, block.id) {
                    loop_work.push((successor, block.id));
                }
            }
        }
        let mut seen_loop_blocks = BTreeSet::new();
        while let Some((header, block)) = loop_work.pop() {
            if !doms.dominates(header, block) || !seen_loop_blocks.insert((header, block)) {
                continue;
            }
            loop_blocks.insert(block);
            loop_work.extend(
                predecessors[block.0 as usize]
                    .iter()
                    .copied()
                    .map(|pred| (header, pred)),
            );
        }
        // A short-lived slice created inside the natural loop is just as hot as its carried
        // remainder. Start it physical too; the legality closure below drops any real escape.
        for block in &self.func.blocks {
            if !loop_blocks.contains(&block.id) {
                continue;
            }
            for op in &block.ops {
                let Some(result) = op
                    .result
                    .filter(|result| is_slice(*result))
                    .filter(|result| !self.uses[result.0 as usize].is_empty())
                else {
                    continue;
                };
                if !matches!(
                    op.kind,
                    OpKind::Call { .. }
                        | OpKind::Intrinsic { .. }
                        | OpKind::Host { .. }
                        | OpKind::Materialize(_)
                        | OpKind::Convert { .. }
                ) && candidates.insert(result)
                {
                    work.push(result);
                }
            }
        }
        // Follow only the loop-local physical chain: copies/moves and sub-slices derived from a
        // carried value. The preheader's initial value is decomposed once on its incoming edge.
        while let Some(value) = work.pop() {
            for block in &self.func.blocks {
                if !loop_blocks.contains(&block.id) {
                    continue;
                }
                for op in &block.ops {
                    let Some(result) = op.result.filter(|result| is_slice(*result)) else {
                        continue;
                    };
                    let related = match &op.kind {
                        OpKind::Copy(source) | OpKind::Move(source) | OpKind::Deref(source) => {
                            Some(*source)
                        }
                        OpKind::SliceView { ptr, .. } if is_slice(*ptr) => Some(*ptr),
                        _ => None,
                    };
                    if let Some(source) = related {
                        for related in [
                            (source == value).then_some(result),
                            (result == value).then_some(source),
                        ]
                        .into_iter()
                        .flatten()
                        {
                            if candidates.insert(related) {
                                work.push(related);
                            }
                        }
                    }
                }
            }
            for block in &self.func.blocks {
                for (target, args) in block.term.edges() {
                    for (param, arg) in self.func.blocks[target.0 as usize].params.iter().zip(args)
                    {
                        if *arg == value && is_slice(*param) && candidates.insert(*param) {
                            work.push(*param);
                        }
                        if *param == value
                            && doms.dominates(target, block.id)
                            && is_slice(*arg)
                            && candidates.insert(*arg)
                        {
                            work.push(*arg);
                        }
                    }
                }
            }
        }

        loop {
            let before = candidates.clone();
            let mut refused = BTreeSet::new();
            for block in &self.func.blocks {
                for op in &block.ops {
                    for used in op.kind.uses() {
                        if !candidates.contains(&used)
                            || slice_use_allowed(&op.kind, used, op.result, &candidates)
                        {
                            continue;
                        }
                        refused.insert(used);
                    }
                }
                for (target, args) in block.term.edges() {
                    for (param, arg) in self.func.blocks[target.0 as usize].params.iter().zip(args)
                    {
                        // A physical value flowing out to a canonical parameter is a real escape
                        // and is materialized on the edge. A physical parameter may receive a
                        // canonical preheader value, which is decomposed once there.
                        if candidates.contains(param) && !candidates.contains(arg) {
                            let back_edge = doms.dominates(target, block.id);
                            if back_edge {
                                refused.insert(*param);
                            }
                        }
                    }
                }
                match &block.term {
                    RTerm::Switch { scrut, .. }
                    | RTerm::Return(Some(scrut))
                    | RTerm::Throw(Some(scrut))
                        if candidates.contains(scrut) =>
                    {
                        refused.insert(*scrut);
                    }
                    _ => {}
                }
            }
            candidates.retain(|value| !refused.contains(value));
            if candidates == before {
                break;
            }
        }
        // A slice-to-element cast is the common bridge from a loop-carried fat slice to indexed
        // loads. Keep that compiler-internal thin pointer in two lanes when every use only walks
        // or projects it. Calls, stores, control-flow edges, and abrupt exits remain canonical.
        let mut slots = BTreeSet::new();
        for block in &self.func.blocks {
            for op in &block.ops {
                let OpKind::Cast {
                    kind: CastKind::PtrToPtr,
                    v,
                    ..
                } = &op.kind
                else {
                    continue;
                };
                let Some(result) = op.result else { continue };
                if !candidates.contains(v)
                    || self
                        .table
                        .pointer_form(self.func.ty(result).unwrap_or(TypeId(0)))
                        != Some(PointerForm::Slot)
                {
                    continue;
                }
                let legal_ops = self.func.blocks.iter().all(|user_block| {
                    user_block.ops.iter().all(|user| {
                        !user.kind.uses().contains(&result) || slot_use_allowed(&user.kind, result)
                    })
                });
                let used_on_edge = self.func.blocks.iter().any(|user_block| {
                    user_block
                        .term
                        .edges()
                        .into_iter()
                        .any(|(_, args)| args.contains(&result))
                        || matches!(
                            user_block.term,
                            RTerm::Switch { scrut, .. }
                                | RTerm::Return(Some(scrut))
                                | RTerm::Throw(Some(scrut))
                                if scrut == result
                        )
                });
                if legal_ops && !used_on_edge {
                    slots.insert(result);
                }
            }
        }
        for value in candidates.iter().chain(&slots) {
            self.plan.inline.remove(value);
        }
        self.plan.physical_slices = candidates;
        self.plan.physical_slots = slots;
    }

    // -- inlining -----------------------------------------------------------------------------

    /// The successor a block hands control to in a straight line: its `Goto` target when that
    /// target has no other predecessor. Such a pair is emitted back to back (a call's result is
    /// defined at the end of one block and used in the next), so a value may be inlined across
    /// it exactly as within a block.
    fn chain_next(&self, block: RBlockId) -> Option<RBlockId> {
        let RTerm::Goto(target, _) = &self.func.blocks[block.0 as usize].term else {
            return None;
        };
        (self.preds[target.0 as usize].as_slice() == [block]).then_some(*target)
    }

    /// Synthetic views can carry a placeholder type and must remain places at their load/store.
    /// Keep every operand materialized and only fuse adjacent operations in the same block, so
    /// no evaluation crosses a protected call or a normal/exceptional edge.
    fn inline_adjacent_place_views(&mut self) {
        for block in &self.func.blocks {
            for (index, pair) in block.ops.windows(2).enumerate() {
                let Some(value) = pair[0].result else {
                    continue;
                };
                if !matches!(
                    pair[0].kind,
                    OpKind::BorrowView { .. } | OpKind::RawView { .. }
                ) {
                    continue;
                }
                let [use_] = self.uses[value.0 as usize].as_slice() else {
                    continue;
                };
                if use_.block != block.id || use_.position != index + 1 || use_.conditional {
                    continue;
                }
                if matches!(pair[1].kind,
                    OpKind::Deref(pointer) | OpKind::StoreThrough { ptr: pointer, .. }
                    if pointer == value)
                {
                    self.plan.inline.insert(value);
                }
            }
        }
    }

    fn decide_inline(&mut self) {
        // Blocks in reverse postorder, so a chain's head is decided before its tail and every
        // operand of a candidate has been classified when the candidate is.
        let order = Dominators::of(self.func).rpo.clone();
        for block_id in order {
            let block = &self.func.blocks[block_id.0 as usize];
            for (index, op) in block.ops.iter().enumerate() {
                let mut purity = op_purity(&op.kind);
                for v in op.kind.uses() {
                    if self.plan.inline.contains(&v) {
                        purity = purity.max(self.purity.get(&v).copied().unwrap_or(Purity::Effect));
                    }
                }
                let Some(result) = op.result else {
                    continue;
                };
                self.purity.insert(result, purity);
                if self.inlinable(result, block_id, index, purity) {
                    self.plan.inline.insert(result);
                }
            }
        }
    }

    /// A closure environment handed to a host marker call (`rcgjs.host.*`, `view-abi`) is a
    /// value the marker takes ownership of at the call, spelled inside the thunk the host runs
    /// later: it has to be materialized before the call, as v1 did (`abi.rs::closure_thunk`), so
    /// such an argument stays a named value whatever the ladder says.
    fn pin_host_environments(&mut self) {
        for block in &self.func.blocks {
            for op in &block.ops {
                let OpKind::Call {
                    args,
                    host: Some(_),
                    ..
                } = &op.kind
                else {
                    continue;
                };
                for arg in args {
                    let ty = self.func.values[arg.0 as usize].ty;
                    if !self.table.needs_clone(ty) {
                        continue;
                    }
                    // Through the `Move` a MIR `move` operand became: the value that carries the
                    // MIR local (and its name, `_19`) is the one to materialize; the rename stays
                    // inline and spells that name.
                    let mut root = *arg;
                    for _ in 0..8 {
                        match self.defs[root.0 as usize] {
                            Some(Def::Op(block, index)) => {
                                match self.func.blocks[block.0 as usize].ops[index].kind {
                                    OpKind::Move(inner) => root = inner,
                                    _ => break,
                                }
                            }
                            _ => break,
                        }
                    }
                    self.plan.inline.remove(&root);
                }
            }
        }
    }

    /// The purity an op's expression would have if everything it reads were spelled inline: its
    /// own class joined with its operands' (a decided operand contributes its inline purity or,
    /// declared, nothing; an undecided one its own bound). What a candidate must commute with
    /// when the op sits between its definition and its use, since the op's evaluation — or the
    /// evaluation of what it inlines — happens there.
    fn bound(&self, op: &crate::jir::repr::Op, depth: usize) -> Purity {
        let mut purity = op_purity(&op.kind);
        if depth > 32 {
            return Purity::Effect;
        }
        for v in op.kind.uses() {
            purity = purity.max(self.value_bound(v, depth + 1));
        }
        purity
    }

    fn value_bound(&self, v: ValueId, depth: usize) -> Purity {
        if let Some(purity) = self.purity.get(&v) {
            // Decided: an inline value moves with its reader, a declared one is a name.
            return if self.plan.inline.contains(&v) {
                *purity
            } else {
                Purity::Const
            };
        }
        match self.defs.get(v.0 as usize).copied().flatten() {
            Some(Def::Op(block, index)) => {
                self.bound(&self.func.blocks[block.0 as usize].ops[index], depth)
            }
            _ => Purity::Const,
        }
    }

    /// Whether the value `result`, defined by op `index` of `block` with effective purity
    /// `purity`, may be spelled at its single use: the use follows the definition in the same
    /// block or down a straight-line chain of blocks, and everything evaluated in between
    /// commutes with it (v1 `queue.rs`: a queued entry is flushed before a statement that does
    /// not commute with it).
    /// Whether the emitter at `use_` requires an already captured operand.
    /// Intrinsics share their evaluation contract with final lowering. Unknown contracts
    /// are conservative even when the JIR operand has only one use.
    ///
    /// `js/value.rs::clone_expr` rebuilds an aggregate field by field and an enum variant by
    /// variant, so a `Copy` of such a type mentions its operand once per field and once per tag
    /// test — eight times for a two-variant enum with three cloned fields. Inlining an expression
    /// there multiplies its text, and a chain of them squares it: on
    /// `core-tests/39_native_vec_drop` one such chain took `RawVec::grow_amortized` from 4.2 KB
    /// to 10.9 KB. Such a reader gets a name to read instead, which is what v1 printed.
    ///
    /// A slot dereference or store also reads both `p.buf` and `p.off`. An effectful slot
    /// expression must be materialized even when its SSA use count is one: otherwise an inlined
    /// call under that expression is invoked twice. Pure synthetic views stay inline so a field
    /// store remains `object.field = value`; materializing one as `{ buf, off }` would hide that
    /// alias from the final local-expression optimizer. The effectful shape is produced by
    /// `BTreeMap::values()` after MIR inlining, where one iterator advance returns a tuple
    /// containing a value pointer.
    ///
    /// Aggregate copy counts are taken from the spelling itself rather than from a second copy of
    /// its rules: `clone_expr` is asked for the clone of a placeholder and the mentions are
    /// counted.
    fn requires_captured_operand(&self, value: ValueId, use_: &Use, purity: Purity) -> bool {
        let block = &self.func.blocks[use_.block.0 as usize];
        let Some(op) = block.ops.get(use_.position) else {
            return false;
        };
        if uses_as_storage(&op.kind, value) {
            return true;
        }
        // Zero-sized expressions erase their operands from the final spelling. Keep those
        // operands at their definitions, where calls and other effects still execute.
        if op
            .result
            .is_some_and(|result| self.table.is_zst(self.func.values[result.0 as usize].ty))
            && !matches!(
                op.kind,
                OpKind::Call { .. } | OpKind::Intrinsic { .. } | OpKind::Host { .. }
            )
        {
            return true;
        }
        // So does a place walk that reaches an element with no storage: the index that selected
        // it is not spelled, and a read or write of it spells nothing (`js::place::reaches_unit`).
        if let OpKind::BorrowView { root, path, .. }
        | OpKind::RawView { root, path, .. }
        | OpKind::Drop { root, path, .. } = &op.kind
        {
            let root_ty = match root {
                Root::Value(base) => self.func.values[base.0 as usize].ty,
                Root::Cell(cell) => self.func.cells[cell.0 as usize].ty,
            };
            let operand = matches!(root, Root::Value(base) if *base == value)
                || path_uses(path).contains(&value);
            if operand && super::js::place::reaches_unit(self.table, root_ty, path) {
                return true;
            }
        }
        if self.cast_reads_reference_spelling(value, op) {
            return true;
        }
        if let OpKind::Intrinsic { name, .. } = &op.kind {
            return OperandEvaluation::intrinsic(self.interns.str(*name).unwrap_or(""))
                == OperandEvaluation::Materialize;
        }
        if matches!(op.kind, OpKind::Host { .. }) {
            return true;
        }
        if let OpKind::Call {
            host: Some(HostPayload::Marker { section, .. }),
            ..
        } = &op.kind
        {
            if let Some(marker) = self
                .interns
                .str(*section)
                .and_then(|s| s.strip_prefix("rcgjs.host."))
            {
                return rcgjs_object::host::HostOperation::from_marker(marker)
                    .map(|operation| operation.evaluation())
                    .unwrap_or(OperandEvaluation::Materialize)
                    == OperandEvaluation::Materialize;
            }
        }
        // Descriptor and opaque host operations may look up a member, coerce a key, or
        // enter a catch boundary before their JavaScript argument positions are reached.
        // Their Rust arguments have already run outside those operations.
        if matches!(op.kind, OpKind::Call { host: Some(_), .. }) {
            return true;
        }
        if let OpKind::Binary {
            op: BinaryOp::Cmp, ..
        } = op.kind
        {
            return OperandEvaluation::comparison(BinaryOp::Cmp) == OperandEvaluation::Materialize;
        }
        // A discriminant may be a constant or a chain of tag tests. In either case the
        // original enum producer must execute once before selecting its discriminant.
        if matches!(op.kind, OpKind::Discriminant(_)) {
            return true;
        }
        // Unsizing an aggregate rebuilds its fields and can read the source for each one.
        if let OpKind::Cast {
            kind: CastKind::PointerCoercion,
            from,
            ..
        } = op.kind
        {
            if matches!(
                self.table.get(self.table.peeled(from)).map(|ty| &ty.kind),
                Some(TypeKind::Adt { .. })
            ) {
                return true;
            }
        }
        if let OpKind::Repeat { v, .. } = &op.kind {
            let is_constant = self.defs[v.0 as usize].is_some_and(|def| {
                let Def::Op(block, index) = def else {
                    return false;
                };
                matches!(
                    self.func.blocks[block.0 as usize].ops[index].kind,
                    OpKind::Const(_)
                )
            });
            return OperandEvaluation::repeat(
                self.table.needs_clone(self.func.values[v.0 as usize].ty),
                is_constant,
            ) == OperandEvaluation::Materialize;
        }
        if purity == Purity::Effect
            && matches!(
                self.table
                    .pointer_form(self.func.values[value.0 as usize].ty),
                Some(PointerForm::Slot)
            )
        {
            match &op.kind {
                OpKind::Deref(pointer) | OpKind::StoreThrough { ptr: pointer, .. }
                    if *pointer == value =>
                {
                    return true;
                }
                OpKind::BorrowView {
                    root: Root::Value(root),
                    path,
                    ..
                }
                | OpKind::RawView {
                    root: Root::Value(root),
                    path,
                    ..
                } if *root == value && matches!(path.first(), Some(Proj::Deref)) => {
                    return true;
                }
                _ => {}
            }
        }

        let OpKind::Copy(_) = op.kind else {
            return false;
        };
        let Some(result) = op.result else {
            return false;
        };
        let ty = self.func.values[result.0 as usize].ty;
        if !self.table.needs_clone(ty) {
            return false;
        }
        const PLACEHOLDER: &str = "$operand";
        let clone = crate::lower::js::value::clone_expr(
            self.table,
            self.interns,
            ty,
            crate::jsast::Expr::Ident(PLACEHOLDER.to_owned()),
        );
        let mut mentions = 0usize;
        crate::jsast::visit_idents(&crate::jsast::Stmt::ExprStmt(clone), &mut |ident| {
            if ident == PLACEHOLDER {
                mentions += 1;
            }
        });
        mentions > 1
    }

    /// Whether `op` is a reference-to-raw cast of `value` to a pointer to an aggregate, which
    /// `js::ptr::ref_to_raw` builds from the operand's *spelling* rather than its value.
    ///
    /// A reference to an aggregate is the object itself, and the raw pointer made from it is the
    /// object's one `__rt.box` slot. `ref_to_raw` reads the reference's expression, though: `x.f`
    /// becomes `{ buf: x, off: "f" }` and `x[i]` becomes `{ buf: x, off: i }`, the slot of the
    /// place that expression names. Spelled inline, a reference loaded from a field or an element
    /// is exactly such an expression, and the cast would answer the address of the field that
    /// *holds* the reference (`&a.r`, a `*const &P`) instead of the pointer stored in it. The
    /// operand is captured, so the cast sees a name. A raw view of a reference's pointee
    /// (`&raw const *r`, which is how rustc spells `r as *const P`) does not depend on the spelling
    /// (`js::place::Walk::object`), so only this cast (`js::rvalue::cast`) is left to guard here.
    ///
    /// No lowered program reaches it today: MIR's `PtrToPtr` is raw to raw, so a reference-typed
    /// operand only arrives from a JIR pass or a hand-built body. It is kept because the cast's
    /// lowering accepts one and would answer the field's slot silently;
    /// `js::tests::a_reference_to_raw_cast_reads_a_loaded_reference_through_a_name` builds the
    /// shape and is what exercises this rule.
    fn cast_reads_reference_spelling(&self, value: ValueId, op: &crate::jir::repr::Op) -> bool {
        use super::js::value::{is_raw, is_slot_pointee, pointee_of};
        let OpKind::Cast {
            kind: CastKind::PtrToPtr | CastKind::FnPtrToPtr,
            v,
            from,
            to,
        } = &op.kind
        else {
            return false;
        };
        let table = self.table;
        *v == value
            && !is_raw(table, *from)
            && is_raw(table, *to)
            && pointee_of(table, *from).is_some_and(|pointee| {
                is_slot_pointee(table, pointee)
                    && table.is_indirect(pointee)
                    && table.unsized_tail(pointee).is_none()
            })
    }

    /// The encoder has consumed this static address into every template call's payload.
    /// It has no runtime evaluation, including when several calls share the same constant.
    fn is_template_data(&self, result: ValueId, uses: &[Use]) -> bool {
        let Some(Def::Op(block, index)) = self.defs[result.0 as usize] else {
            return false;
        };
        if !matches!(
            self.func.blocks[block.0 as usize].ops[index].kind,
            OpKind::Const(ConstTree::Global {
                target: GlobalReference::Static { .. },
                offset: 0
            })
        ) {
            return false;
        }
        let has_invoke = self.func.has_invoke();
        !uses.is_empty()
            && uses.iter().all(|use_| {
                if has_invoke && (use_.block != block || use_.edge.is_some()) {
                    return false;
                }
                if matches!(
                    self.func.blocks[use_.block.0 as usize].term,
                    RTerm::Invoke { .. }
                ) && use_.position + 1 >= self.func.blocks[use_.block.0 as usize].ops.len()
                {
                    return false;
                }
                matches!(self.func.blocks[use_.block.0 as usize].ops.get(use_.position).map(|op| &op.kind),
                    Some(OpKind::Call {
                        host: Some(HostPayload::Marker { section, template: Some(_), .. }), args, ..
                    }) if self.interns.str(*section) == Some("rcgjs.tc.template") && args.as_slice() == [result])
            })
    }

    fn inlinable(&self, result: ValueId, block: RBlockId, index: usize, purity: Purity) -> bool {
        // A use or return of a zero-sized value reads `undefined`, so its producer must
        // execute at its definition instead of disappearing with that value. Synthetic views
        // can use the pointee type as a placeholder, including a zero-sized closure field.
        // They describe places, so the emitter must still fuse them into their dereference/store.
        if self.table.is_zst(self.func.values[result.0 as usize].ty)
            && !matches!(
                self.func.blocks[block.0 as usize].ops[index].kind,
                OpKind::BorrowView { .. } | OpKind::RawView { .. } | OpKind::SliceView { .. }
            )
        {
            return false;
        }
        if matches!(
            self.func.blocks[block.0 as usize].term,
            RTerm::Invoke { .. }
        ) && index + 1 >= self.func.blocks[block.0 as usize].ops.len()
        {
            return false;
        }
        let uses = &self.uses[result.0 as usize];
        if self.is_template_data(result, uses) {
            return true;
        }
        let [use_] = uses.as_slice() else {
            return false;
        };
        if use_.conditional {
            return false;
        }
        // Keep operands of the protected operation (and edge copies) materialized before `try`.
        if matches!(
            self.func.blocks[use_.block.0 as usize].term,
            RTerm::Invoke { .. }
        ) && use_.position + 1 >= self.func.blocks[use_.block.0 as usize].ops.len()
        {
            return false;
        }
        if self.requires_captured_operand(result, use_, purity) {
            return false;
        }
        let ops = |b: RBlockId| &self.func.blocks[b.0 as usize].ops;
        let scrutinee = |b: RBlockId| match &self.func.blocks[b.0 as usize].term {
            RTerm::Switch { scrut, .. } => Some(*scrut),
            _ => None,
        };
        if self.func.has_invoke() && (use_.block != block || use_.edge.is_some()) {
            return false;
        }
        // Everything strictly between the definition and the use has to commute with it.
        let check = |slice: &[crate::jir::repr::Op]| {
            slice.iter().all(|op| commutes(purity, self.bound(op, 0)))
        };
        if use_.block == block {
            if use_.position <= index {
                return false;
            }
            let end = use_.position.min(ops(block).len());
            if !check(&ops(block)[index + 1..end]) {
                return false;
            }
        } else {
            // Down the chain: the rest of the defining block, every block on the way, and the
            // start of the using block.
            if !check(&ops(block)[index + 1..]) {
                return false;
            }
            let mut current = self.chain_next(block);
            let mut hops = 0;
            loop {
                let Some(b) = current else {
                    return false;
                };
                if b == use_.block {
                    let end = use_.position.min(ops(b).len());
                    if !check(&ops(b)[..end]) {
                        return false;
                    }
                    break;
                }
                hops += 1;
                if hops > 8 || !check(ops(b)) {
                    return false;
                }
                current = self.chain_next(b);
            }
        }
        // An edge copy runs after the terminator's scrutinee is evaluated.
        if use_.position > ops(use_.block).len() {
            if let Some(scrut) = scrutinee(use_.block) {
                if !commutes(purity, self.value_bound(scrut, 0)) {
                    return false;
                }
            }
        }
        true
    }

    // -- names --------------------------------------------------------------------------------

    fn origin(&self, id: OriginId) -> Option<&'a crate::jir::origin::Origin> {
        if id == NO_ORIGIN {
            return None;
        }
        self.origins.and_then(|origins| origins.origin(id))
    }

    /// The MIR local an origin id belongs to (the first local whose origin it is).
    fn local_of_origin(&self, id: OriginId) -> Option<usize> {
        if id == NO_ORIGIN {
            return None;
        }
        self.origins
            .and_then(|origins| origins.locals.iter().position(|local| *local == id))
    }

    /// The user's name for a value or cell with origin `id`, rescued into a JavaScript name.
    fn user_name(&self, id: OriginId) -> Option<(String, String)> {
        if self.opts.names == NameStyle::Mangled {
            return None;
        }
        let origin = self.origin(id)?;
        let raw = self.interns.str(origin.local_name?)?;
        rescue(raw).map(|name| (name, raw.to_owned()))
    }

    fn unique(&mut self, base: String) -> String {
        if self.claim(&base) {
            return base;
        }
        for n in 1u32.. {
            let candidate = format!("{base}${n}");
            if self.claim(&candidate) {
                return candidate;
            }
        }
        base
    }

    /// Takes `name` unless an earlier name holds it or, once [`Self::reserve_interns`] is set, it
    /// is an interned string.
    fn claim(&mut self, name: &str) -> bool {
        if self.taken.contains(name) {
            return false;
        }
        if self.reserve_interns {
            crate::profile::bump(crate::profile::Counter::CatchReserveStrings);
            if self.interns.contains_str(name) {
                return false;
            }
        }
        self.taken.insert(name.to_owned())
    }

    /// Names each `Invoke`'s outcome flag, then the handler's exception and catch bindings.
    ///
    /// A catch binding also shadows free names in its handler, so these names avoid every
    /// interned string besides the names taken so far. The table is crate-sized and only a few
    /// `$`-prefixed candidates are asked about, so [`Self::claim`] looks each one up in it rather
    /// than copying the table into `taken`.
    fn name_invokes(&mut self) {
        if !self.func.has_invoke() {
            return;
        }
        crate::profile::bump(crate::profile::Counter::CatchReserveCalls);
        self.reserve_interns = true;
        let func = self.func;
        for block in &func.blocks {
            if matches!(block.term, RTerm::Invoke { .. }) {
                let name = self.unique(format!("$invoke{}", block.id.0));
                self.plan.invoke_names.insert(block.id, name);
            }
        }
        self.plan.exception_name = Some(self.unique("$exception".to_owned()));
        self.plan.catch_name = Some(self.unique("$caught".to_owned()));
    }

    /// Whether the value needs a name: an entry parameter, a block parameter, or an op result
    /// that is used and not inline.
    fn needs_name(&self, v: ValueId) -> bool {
        match self.defs[v.0 as usize] {
            Some(Def::Param(_)) => true,
            Some(Def::Op(..)) => {
                !self.plan.inline.contains(&v) && !self.uses[v.0 as usize].is_empty()
            }
            None => false,
        }
    }

    fn name(&mut self) {
        for op in self.func.blocks.iter().flat_map(|block| &block.ops) {
            let bytes = match &op.kind {
                OpKind::Call {
                    host: Some(HostPayload::Extern(bytes)),
                    ..
                }
                | OpKind::Host {
                    payload: HostPayload::Extern(bytes),
                    ..
                } => *bytes,
                _ => continue,
            };
            let Some(bytes) = self.interns.bytes(bytes) else {
                continue;
            };
            let Ok(section) = std::str::from_utf8(bytes) else {
                continue;
            };
            let Ok(descriptor) = crate::descriptor::Descriptor::decode(section) else {
                continue;
            };
            if descriptor.root() != crate::descriptor::Root::Module
                && !descriptor.acts_on_argument()
            {
                if let Some(head) = descriptor.segments().first() {
                    self.plan.globals.insert((*head).to_owned());
                }
            }
        }
        self.taken.extend(self.plan.globals.iter().cloned());
        // The names nothing may take: reserved words and the globals the emitted code reads are
        // handled by `rescue`; the numbered spellings of every MIR local are reserved up front so
        // a user name that happens to read `_7` cannot take one (v1 `names.rs`).
        if let Some(origins) = self.origins {
            for index in 0..origins.locals.len() {
                let name = format!("_{index}");
                self.taken.insert(name.clone());
                if !self.plan.globals.contains(&name) {
                    self.reserved_numbers.insert(name);
                }
            }
        }
        self.taken.insert(CALLER_LOCATION_NAME.to_owned());
        let caller_location = self.func.sig.abi.caller_location;
        // Entry parameters first, in order.
        let params = self.func.params.clone();
        for (index, param) in params.iter().enumerate() {
            let name = if caller_location && index + 1 == params.len() {
                CALLER_LOCATION_NAME.to_owned()
            } else {
                self.fresh_name(self.func.values[param.0 as usize].origin, param.0)
            };
            self.plan.names.insert(*param, name);
        }
        // Then every other value that needs a name, in id order.
        for index in 0..self.func.values.len() {
            let v = ValueId(index as u32);
            if self.plan.names.contains_key(&v) || !self.needs_name(v) {
                continue;
            }
            let origin = self.func.values[index].origin;
            let name = self.fresh_name(origin, index as u32);
            self.plan.names.insert(v, name);
        }
        // Cells: a cell that holds an entry parameter shares the parameter's name.
        let mut param_cells: BTreeMap<CellId, ValueId> = BTreeMap::new();
        if let Some(entry) = self.func.blocks.first() {
            let mut touched: BTreeSet<CellId> = BTreeSet::new();
            for op in &entry.ops {
                match &op.kind {
                    OpKind::CellAlloc(_) => {}
                    OpKind::CellStore { cell, v } if !touched.contains(cell) => {
                        touched.insert(*cell);
                        // The prelude's store of a parameter into its own cell: the parameter is
                        // read nowhere else, and (when origins are known) the cell is the
                        // parameter's local. `x = [x]` is then a rebinding nothing observes.
                        let same_local = self.func.values[v.0 as usize].origin
                            == self.func.cells[cell.0 as usize].origin;
                        if self.func.params.contains(v)
                            && self.uses[v.0 as usize].len() == 1
                            && same_local
                            && !param_cells.values().any(|p| p == v)
                        {
                            param_cells.insert(*cell, *v);
                        }
                    }
                    other => {
                        // Any other touch of a cell (a load, a borrow, a store of a non-parameter)
                        // ends the prelude for it.
                        for cell in cells_touched(other) {
                            touched.insert(cell);
                        }
                    }
                }
            }
        }
        for (index, cell) in self.func.cells.iter().enumerate() {
            let id = CellId(index as u32);
            let name = match param_cells.get(&id) {
                Some(param) => self.plan.names[param].clone(),
                None => self.fresh_name(cell.origin, u32::MAX),
            };
            self.plan.cell_names.insert(id, name);
        }
        self.plan.param_cells = param_cells;
    }

    /// A fresh unique name for a value or cell with origin `origin`: the user's name when it
    /// has one, `_N` after its MIR local when known, `$v{id}` otherwise.
    fn fresh_name(&mut self, origin: OriginId, value: u32) -> String {
        if let Some((base, raw)) = self.user_name(origin) {
            let name = self.unique(base);
            if name != raw {
                self.plan.source_names.push((name.clone(), raw));
            }
            return name;
        }
        if let Some(local) = self.local_of_origin(origin) {
            let base = format!("_{local}");
            // The numbered spelling was reserved for its first value; later values of the same
            // local get suffixes.
            if self.reserved_numbers.remove(&base) {
                return base;
            }
            return self.unique(base);
        }
        if value == u32::MAX {
            return self.unique("$cell".to_owned());
        }
        let base = format!("$v{value}");
        self.unique(base)
    }

    // -- copies -------------------------------------------------------------------------------

    /// The names an inline value's expression reads: the non-inline values it (transitively)
    /// mentions.
    fn read_set(&self, v: ValueId, out: &mut BTreeSet<ValueId>, depth: usize) {
        if depth > 64 {
            return;
        }
        if !self.plan.inline.contains(&v) {
            out.insert(v);
            return;
        }
        if let Some(Def::Op(block, index)) = self.defs[v.0 as usize] {
            let op = &self.func.blocks[block.0 as usize].ops[index];
            for operand in op.kind.uses() {
                self.read_set(operand, out, depth + 1);
            }
        }
    }

    fn copies(&mut self) {
        let mut edges: Vec<(RBlockId, RBlockId, Vec<ValueId>)> = Vec::new();
        for block in &self.func.blocks {
            let mut seen = BTreeSet::new();
            for (target, args) in block.term.edges() {
                if !seen.insert(target) || args.is_empty() {
                    continue;
                }
                edges.push((block.id, target, args.to_vec()));
            }
        }
        for (from, to, args) in edges {
            let params = self.func.blocks[to.0 as usize].params.clone();
            // Retry after un-inlining a source that would need a temporary.
            let sequence = loop {
                match self.sequence(&params, &args) {
                    Ok(copies) => break copies,
                    Err(v) => {
                        self.plan.inline.remove(&v);
                        let origin = self.func.values[v.0 as usize].origin;
                        let name = self.fresh_name(origin, v.0);
                        self.plan.names.insert(v, name);
                    }
                }
            };
            self.plan.copies.insert((from, to), sequence);
        }
    }

    /// Sequentializes the parallel copy `params = args`. `Err(v)` names an inline source that
    /// has to be declared before the edge can be sequenced.
    fn sequence(&mut self, params: &[ValueId], args: &[ValueId]) -> Result<Vec<EdgeCopy>, ValueId> {
        // A pending copy: destination, source, and what the source reads.
        struct Pending {
            dst: ValueId,
            src: CopyOperand,
            reads: BTreeSet<ValueId>,
            /// The inline source value, when the copy's source is spelled inline.
            inline: Option<ValueId>,
        }
        let mut pending: Vec<Pending> = Vec::new();
        for (dst, src) in params.iter().zip(args) {
            // `x = x` says nothing.
            if dst == src {
                continue;
            }
            let mut reads = BTreeSet::new();
            self.read_set(*src, &mut reads, 0);
            let inline = self.plan.inline.contains(src).then_some(*src);
            pending.push(Pending {
                dst: *dst,
                src: CopyOperand::Value(*src),
                reads,
                inline,
            });
        }
        let mut out = Vec::new();
        while !pending.is_empty() {
            // A copy whose destination no other pending copy still reads can go now.
            let ready = pending.iter().position(|copy| {
                !pending
                    .iter()
                    .any(|other| !std::ptr::eq(other, copy) && other.reads.contains(&copy.dst))
            });
            if let Some(index) = ready {
                let copy = pending.remove(index);
                out.push(EdgeCopy {
                    dst: CopyOperand::Value(copy.dst),
                    src: copy.src,
                });
                continue;
            }
            // Every destination is still read: a cycle. Save the first destination's old value
            // in a temporary and let its readers read that instead. A reader spelled inline
            // cannot be redirected: it is declared and the edge sequenced again.
            let victim = pending[0].dst;
            if let Some(reader) = pending
                .iter()
                .find(|copy| copy.reads.contains(&victim) && copy.inline.is_some())
            {
                return Err(reader.inline.expect("checked"));
            }
            let temp = self.plan.copy_temps.len() as u32;
            let name = self.unique(format!("$c{temp}"));
            self.plan.copy_temps.push(name);
            out.push(EdgeCopy {
                dst: CopyOperand::Temp(temp),
                src: CopyOperand::Value(victim),
            });
            for copy in pending.iter_mut() {
                if copy.reads.remove(&victim) {
                    // A plain-name source that read the victim now reads the temporary.
                    if copy.src == CopyOperand::Value(victim) {
                        copy.src = CopyOperand::Temp(temp);
                    }
                }
            }
        }
        Ok(out)
    }

    // -- scrutinees ---------------------------------------------------------------------------

    fn scrutinees(&mut self) {
        // v1 `tag.rs`: a `Discriminant` of a tagged, multi-variant enum whose every use is a
        // switch scrutinee reads the tag.
        for block in &self.func.blocks {
            for op in &block.ops {
                let (Some(result), OpKind::Discriminant(base)) = (op.result, &op.kind) else {
                    continue;
                };
                let base_ty = self.func.values[base.0 as usize].ty;
                if !matches!(self.table.enum_repr(base_ty), Some(EnumRepr::Tag { .. })) {
                    continue;
                }
                if !has_multiple_variants(self.table, base_ty) {
                    continue;
                }
                let uses = &self.uses[result.0 as usize];
                let all_switches = !uses.is_empty()
                    && uses.iter().all(|use_| {
                        use_.edge.is_none()
                            && !use_.conditional
                            && use_.position == self.func.blocks[use_.block.0 as usize].ops.len()
                            && matches!(
                                &self.func.blocks[use_.block.0 as usize].term,
                                RTerm::Switch { scrut, .. } if *scrut == result
                            )
                    });
                if all_switches {
                    self.plan.tag_values.insert(result);
                }
            }
        }
        for block in &self.func.blocks {
            let RTerm::Switch {
                scrut, scrut_ty, ..
            } = &block.term
            else {
                continue;
            };
            let tag_enum = if self.plan.tag_values.contains(scrut) {
                match self.defs[scrut.0 as usize] {
                    Some(Def::Op(b, index)) => {
                        match &self.func.blocks[b.0 as usize].ops[index].kind {
                            OpKind::Discriminant(base) => {
                                Some(self.func.values[base.0 as usize].ty)
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                }
            } else {
                None
            };
            self.plan.scrutinees.insert(
                block.id,
                RScrut::Value {
                    value: *scrut,
                    ty: *scrut_ty,
                    tag_enum,
                },
            );
        }
    }

    // -- finishing ----------------------------------------------------------------------------

    fn finish(mut self) -> Plan {
        let params: BTreeSet<ValueId> = self.func.params.iter().copied().collect();
        for index in 0..self.func.values.len() {
            let v = ValueId(index as u32);
            if params.contains(&v) || !self.plan.names.contains_key(&v) {
                continue;
            }
            self.plan.declared_order.push(v);
        }
        self.plan
    }
}

/// Whether `kind` consumes the fat-slice `value` without needing its record identity.
fn slice_use_allowed(
    kind: &OpKind,
    value: ValueId,
    result: Option<ValueId>,
    candidates: &BTreeSet<ValueId>,
) -> bool {
    match kind {
        OpKind::Copy(source) | OpKind::Move(source) => {
            *source == value && result.is_some_and(|result| candidates.contains(&result))
        }
        OpKind::Unary {
            op: UnaryOp::PtrMetadata,
            v,
            ..
        }
        | OpKind::Deref(v) => *v == value,
        OpKind::Binary {
            op: BinaryOp::Offset,
            l,
            ..
        } => *l == value,
        // Pointer lowering immediately projects the data pointer from this logical slice. jsast
        // folds its `buf`/`off` member reads to the scalar lanes. If the cast result itself needs
        // a name, that result is the real canonical materialization boundary.
        OpKind::Cast {
            kind: CastKind::PtrToPtr,
            v,
            ..
        } => *v == value,
        OpKind::SliceView { ptr, .. } => *ptr == value,
        OpKind::BorrowView {
            root: Root::Value(root),
            path,
            ..
        }
        | OpKind::RawView {
            root: Root::Value(root),
            path,
            ..
        } => *root == value && matches!(path.first(), Some(Proj::Deref)),
        _ => false,
    }
}

/// Whether `kind` consumes a scalar-carried thin pointer without observing its record.
fn slot_use_allowed(kind: &OpKind, value: ValueId) -> bool {
    match kind {
        OpKind::Unary {
            op: UnaryOp::PtrMetadata,
            v,
            ..
        }
        | OpKind::Deref(v) => *v == value,
        OpKind::Binary {
            op: BinaryOp::Offset,
            l,
            ..
        } => *l == value,
        OpKind::BorrowView {
            root: Root::Value(root),
            path,
            ..
        }
        | OpKind::RawView {
            root: Root::Value(root),
            path,
            ..
        } => *root == value && matches!(path.first(), Some(Proj::Deref)),
        _ => false,
    }
}

/// The cells an op touches (loads, stores, borrows, drops).
fn cells_touched(kind: &OpKind) -> Vec<CellId> {
    match kind {
        OpKind::CellAlloc(cell) | OpKind::CellLoad(cell) | OpKind::CellStore { cell, .. } => {
            vec![*cell]
        }
        OpKind::BorrowView {
            root: Root::Cell(cell),
            ..
        }
        | OpKind::RawView {
            root: Root::Cell(cell),
            ..
        }
        | OpKind::Drop {
            root: Root::Cell(cell),
            ..
        } => vec![*cell],
        _ => Vec::new(),
    }
}

/// Whether `kind` uses `value` as storage rather than only reading it: it writes a field or the
/// variant of `value` in place (`SetField`/`SetDiscriminant` with `value` as `base`), or it takes
/// a view of a place inside `value` itself (a `BorrowView`/`RawView` rooted at `value` whose path
/// never dereferences), through which a later store, or a callee handed the view, may write.
///
/// Such a use needs `value` declared. Inlining substitutes a value's defining expression for its
/// name, which is sound for a read: the expression evaluates to the value. A write lands in
/// whatever the spelled expression designates instead, and an inline value has no storage of its
/// own to designate. The expression of a `Copy` that clones is a fresh `{ x: s.x, y: s.y }`, which
/// `jsast::member`'s read-only fold turns into the source's own field (`s.x = 99`); a `Copy` that
/// does not clone, or an `Aggregate`, spells the very names and places it was built from, so the
/// write overwrites one of them (`n = 99`, `table[i] = 99`).
///
/// A view counts whatever its mutability. A shared reference writes through interior mutability
/// (`Cell::set`, `RefCell::borrow_mut`, an atomic store, `UnsafeCell::get`), and a raw pointer is
/// written once it is cast to `*mut` (`UnsafeCell::raw_get`). Whether the place holds an
/// `UnsafeCell` is not a fact the type table records, and after MIR inlining the write is a store
/// in this very body (`let q = C { a: Cell::new(n), b: m }; q.a.set(99)` wrote `n = 99`).
fn uses_as_storage(kind: &OpKind, value: ValueId) -> bool {
    match kind {
        OpKind::SetField { base, .. } | OpKind::SetDiscriminant { base, .. } => *base == value,
        OpKind::BorrowView {
            root: Root::Value(root),
            path,
            ..
        }
        | OpKind::RawView {
            root: Root::Value(root),
            path,
            ..
        } => *root == value && !path.iter().any(|proj| matches!(proj, Proj::Deref)),
        _ => false,
    }
}

/// Whether the enum's layout tells its variants apart at run time (v1 `tag.rs`'s
/// `Variants::Multiple` test): a direct tag on every variant, or a niche.
fn has_multiple_variants(table: &TypeTable, ty: TypeId) -> bool {
    let peeled = table.peeled(ty);
    let Some(descriptor) = table.get(peeled) else {
        return false;
    };
    match &descriptor.kind {
        TypeKind::Adt {
            variants, niche, ..
        } => {
            niche.is_some()
                || variants
                    .iter()
                    .any(|variant| variant.physical_tag.is_some())
        }
        TypeKind::Coroutine { .. } => true,
        _ => false,
    }
}

/// Globals the emitted JavaScript reads, which a local of the same name would shadow (v1
/// `names.rs::RESERVED`).
const RESERVED: &[&str] = &[
    "__rt",
    "Array",
    "ArrayBuffer",
    "BigInt",
    "Boolean",
    "CompressionStream",
    "DataView",
    "DecompressionStream",
    "Error",
    "Infinity",
    "Math",
    "NaN",
    "Number",
    "Object",
    "String",
    "Symbol",
    "Uint8Array",
    "arguments",
    "bb",
    "console",
    "eval",
    "globalThis",
    "undefined",
];

/// Turns a Rust binding name into one that is safe to declare beside the backend's own names
/// (v1 `names.rs::rescue`). `None` leaves the value numbered.
pub fn rescue(raw: &str) -> Option<String> {
    if raw.is_empty() || raw == "_" {
        return None;
    }
    let mut name = jsast::sanitize_ident(raw);
    let looks_like_local =
        name.starts_with('_') && name[1..].starts_with(|c: char| c.is_ascii_digit());
    if name.starts_with('$') || looks_like_local {
        name.insert(0, 'x');
    }
    if RESERVED.contains(&name.as_str()) {
        name.push('_');
    }
    Some(name)
}

/// The values a `Proj` path reads, for callers that inspect a view's operands.
pub fn path_uses(path: &[Proj]) -> Vec<ValueId> {
    path.iter()
        .filter_map(|proj| match proj {
            Proj::Index(v) => Some(*v),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rescued_names_stay_out_of_the_backend_namespaces() {
        assert_eq!(rescue("count").as_deref(), Some("count"));
        assert_eq!(rescue("_").as_deref(), None);
        assert_eq!(rescue("").as_deref(), None);
        assert_eq!(rescue("_0").as_deref(), Some("x_0"));
        assert_eq!(rescue("bb").as_deref(), Some("bb_"));
        assert_eq!(rescue("__rt").as_deref(), Some("__rt_"));
        assert_eq!(rescue("class").as_deref(), Some("_class"));
        assert_eq!(rescue("$weird").as_deref(), Some("x$weird"));
        assert_eq!(rescue("_unused").as_deref(), Some("_unused"));
    }

    /// Two `Invoke`s (blocks 0 and 1) whose unwind edges share one handler.
    fn invoking() -> Func {
        use crate::jir::repr::FuncBuilder;
        use crate::jir::semantic::{AbiSignature, DefinitionId, ItemId, Signature};
        let item = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 1,
            },
            instantiation: 1,
        };
        let sig = Signature {
            abi: AbiSignature {
                parameters: 0,
                spread: None,
                caller_location: false,
                inline_never: false,
            },
            arguments: Vec::new(),
            result: TypeId(0),
        };
        let mut b = FuncBuilder::new(item, sig);
        let entry = FuncBuilder::ENTRY;
        let second = b.block();
        let done = b.block();
        let handler = b.block();
        b.terminate(
            entry,
            RTerm::Invoke {
                normal: (second, Vec::new()),
                unwind: (handler, Vec::new()),
            },
        );
        b.terminate(
            second,
            RTerm::Invoke {
                normal: (done, Vec::new()),
                unwind: (handler, Vec::new()),
            },
        );
        b.terminate(done, RTerm::Return(None));
        b.terminate(handler, RTerm::Throw(None));
        b.finish()
    }

    type CatchNames = (BTreeMap<RBlockId, String>, Option<String>, Option<String>);

    fn catch_names(plan: &Plan) -> CatchNames {
        (
            plan.invoke_names.clone(),
            plan.exception_name.clone(),
            plan.catch_name.clone(),
        )
    }

    /// The catch-name reservation as it was before the interned strings were consulted lazily:
    /// every one of them copied into `taken`, then named through the plain `unique`.
    fn eager_catch_names(planner: &mut Planner<'_>) -> CatchNames {
        planner
            .taken
            .extend(planner.interns.strings.iter().cloned());
        let func = planner.func;
        for block in &func.blocks {
            if matches!(block.term, RTerm::Invoke { .. }) {
                let name = planner.unique(format!("$invoke{}", block.id.0));
                planner.plan.invoke_names.insert(block.id, name);
            }
        }
        if !planner.plan.invoke_names.is_empty() {
            planner.plan.exception_name = Some(planner.unique("$exception".to_owned()));
            planner.plan.catch_name = Some(planner.unique("$caught".to_owned()));
        }
        catch_names(&planner.plan)
    }

    /// Names taken before the reservation, as `name` and `copies` would leave them.
    fn seed(planner: &mut Planner<'_>, earlier: &[&str]) {
        planner.taken.insert(CALLER_LOCATION_NAME.to_owned());
        planner
            .taken
            .extend(earlier.iter().map(|name| (*name).to_owned()));
    }

    #[test]
    fn catch_names_avoid_interned_strings_exactly_as_the_eager_copy_did() {
        let func = invoking();
        let table = TypeTable::default();
        let opts = Opts::default();
        let strings = [
            "$invoke0",
            "$invoke0$1",
            "$exception",
            "$caught",
            "foo",
            "$caught$1",
        ];
        let mut indexed = Interns::new();
        for string in strings {
            indexed.intern_str(string);
        }
        // Filled by hand: the string index is empty and membership falls back to a search.
        let mut hand_filled = Interns::new();
        hand_filled.strings = strings.map(str::to_owned).to_vec();
        let mut appended = indexed.clone();
        appended.strings.push("$invoke1".to_owned());
        let empty = Interns::new();
        let earlier: [&[&str]; 3] = [&[], &["$invoke1", "$exception$1"], &["$caught$2"]];
        for interns in [&indexed, &hand_filled, &appended, &empty] {
            for taken in earlier {
                let mut eager = Planner::new(&func, None, &table, interns, &opts);
                seed(&mut eager, taken);
                let expected = eager_catch_names(&mut eager);
                let mut lazy = Planner::new(&func, None, &table, interns, &opts);
                seed(&mut lazy, taken);
                lazy.name_invokes();
                assert_eq!(
                    catch_names(&lazy.plan),
                    expected,
                    "strings {:?}, taken {taken:?}",
                    interns.strings
                );
                for name in expected.0.values().chain(&expected.1).chain(&expected.2) {
                    assert!(!interns.strings.contains(name));
                    assert!(!taken.contains(&name.as_str()));
                }
            }
        }
        // Through the public entry point.
        assert_eq!(
            catch_names(&plan(&func, None, &table, &indexed, &opts)),
            (
                BTreeMap::from([
                    (RBlockId(0), "$invoke0$2".to_owned()),
                    (RBlockId(1), "$invoke1".to_owned()),
                ]),
                Some("$exception$1".to_owned()),
                Some("$caught$2".to_owned()),
            )
        );
        let mut straight = func.clone();
        straight.blocks.truncate(1);
        straight.blocks[0].term = RTerm::Return(None);
        assert_eq!(
            catch_names(&plan(&straight, None, &table, &indexed, &opts)),
            (BTreeMap::new(), None, None)
        );
    }

    #[test]
    fn the_ladder_commutes_like_v1() {
        assert!(commutes(Purity::Const, Purity::Effect));
        assert!(commutes(Purity::Read, Purity::Read));
        assert!(!commutes(Purity::Read, Purity::Effect));
        assert!(!commutes(Purity::Effect, Purity::Effect));
    }

    #[test]
    fn scoped_fusion_fuses_inside_invoke_function_without_crossing_boundary() {
        use crate::jir::origin::OriginId;
        use crate::jir::repr::FuncBuilder;
        use crate::jir::semantic::{
            AbiSignature, CallSemantic, Callee, ConstTree, DefinitionId, ItemId, Signature, TypeId,
        };
        use crate::jir::types::{
            IntFacts, IntRepr, JsFacts, LayoutFacts, TypeDescriptor, TypeKind, TypeMeta,
        };

        let mut table = TypeTable::default();
        let u32_ty = TypeId(0);
        table.insert(TypeDescriptor {
            id: u32_ty,
            name: None,
            kind: TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: Some(IntFacts {
                    signed: false,
                    bits: 32,
                    repr: IntRepr::Number,
                }),
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: u32_ty,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });
        let mut interns = Interns::new();
        let target_str = interns.intern_str("helper_fn");
        let sig = Signature {
            arguments: Vec::new(),
            result: u32_ty,
            abi: AbiSignature::default(),
        };
        let item_1 = ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash: 1,
            },
            instantiation: 0,
        };
        let item_2 = ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash: 2,
            },
            instantiation: 0,
        };
        let o = OriginId(0);
        let mut b = FuncBuilder::new(item_1, sig);
        let b0 = FuncBuilder::ENTRY;
        let b_norm = b.block();
        let b_unw = b.block();

        let v_before = b.op(
            b0,
            OpKind::Const(ConstTree::Scalar { bits: 7, bytes: 4 }),
            u32_ty,
            o,
        );
        let v_arg = b.op(
            b0,
            OpKind::Unary {
                op: UnaryOp::Neg,
                v: v_before,
                ty: u32_ty,
            },
            u32_ty,
            o,
        );
        let v_live_across = b.op(
            b0,
            OpKind::Const(ConstTree::Scalar { bits: 10, bytes: 4 }),
            u32_ty,
            o,
        );
        let v_call_res = b.op(
            b0,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item_2,
                    link_name: target_str,
                },
                function: None,
                args: vec![v_arg],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            u32_ty,
            o,
        );
        b.terminate(
            b0,
            RTerm::Invoke {
                normal: (b_norm, Vec::new()),
                unwind: (b_unw, Vec::new()),
            },
        );
        let v_after = b.op(
            b_norm,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            u32_ty,
            o,
        );
        let v_tmp = b.op(
            b_norm,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: v_call_res,
                r: v_after,
                ty: u32_ty,
            },
            u32_ty,
            o,
        );
        let v_ret = b.op(
            b_norm,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: v_tmp,
                r: v_live_across,
                ty: u32_ty,
            },
            u32_ty,
            o,
        );
        b.terminate(b_norm, RTerm::Return(Some(v_ret)));
        b.terminate(b_unw, RTerm::Throw(None));

        let func = b.finish();
        let off_plan = plan(&func, None, &table, &interns, &Opts::default());
        assert!(
            off_plan.inline.is_empty(),
            "OptimizerMode::Off must leave Invoke functions un-fused (except adjacent place views)"
        );

        let on_opts = Opts {
            optimizer: rcgjs_object::opts::OptimizerMode::On,
            ..Opts::default()
        };
        let p = plan(&func, None, &table, &interns, &on_opts);
        assert!(
            p.inline.contains(&v_before),
            "pure expression before protected op should fuse"
        );
        assert!(
            !p.inline.contains(&v_arg),
            "operand of protected Invoke call must stay materialized outside try"
        );
        assert!(
            !p.inline.contains(&v_live_across),
            "value live across Invoke must not fuse into normal continuation"
        );
        assert!(
            !p.inline.contains(&v_call_res),
            "protected Invoke call result must not be fused"
        );
        assert!(
            p.inline.contains(&v_after),
            "pure expression in non-Invoke block inside Invoke function should fuse"
        );
    }
}
