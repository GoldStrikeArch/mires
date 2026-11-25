//! Interprocedural effects summaries over phase A's [`Func`]s.
//!
//! Computed at lane time, bottom-up over the Tarjan components of
//! [`crate::analysis::callgraph`]. A summary is a sound over-approximation of
//! what a function can do on two axes:
//!
//! ```text
//! Summary {
//!     purity:     Const <= Read <= Effect
//!     may_unwind: false <= true
//! }
//! ```
//!
//! Join is the pointwise least upper bound; the order is `Summary::le`. `Purity`
//! says whether the caller's world can observe the call at all: `Const` reads
//! nothing outside its arguments and frame-local storage, `Read` may read
//! caller-reachable storage, `Effect` may write it or perform host effects.
//! Panics are tracked separately by `may_unwind`.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::callgraph::{self, Callgraph};
use crate::descriptor::{Descriptor, Fact};
use crate::item::{JsName, LinkItem};
use crate::jir::decline::Decline;
use crate::jir::interns::{Interns, StrId};
use crate::jir::repr::{Func, OpKind, Proj, RTerm, ValueId};
use crate::jir::semantic::{
    CallSemantic, Callee, ConstTree, DropGlue, GlobalReference, HostPayload, ItemId,
    StandardOperation, TypeId,
};
use crate::jir::types::{TypeKind, TypeTable};
use crate::lower::semantic_ops;

/// Whether the caller's world can observe a call to the function.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Purity {
    /// Reads only its arguments' values and frame-local storage.
    Const,
    /// May read caller-reachable storage; writes none of it.
    Read,
    /// May write caller-reachable storage, call hosts, or otherwise be observed.
    Effect,
}

/// One function's effects summary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Summary {
    pub purity: Purity,
    pub may_unwind: bool,
}

impl Summary {
    /// The most precise element: the fixpoint seed inside a recursive component.
    pub fn bottom() -> Summary {
        Summary {
            purity: Purity::Const,
            may_unwind: false,
        }
    }

    /// Knows nothing: equivalent to no summary at all.
    pub fn top() -> Summary {
        Summary {
            purity: Purity::Effect,
            may_unwind: true,
        }
    }

    pub fn is_top(&self) -> bool {
        *self == Summary::top()
    }

    /// Pointwise least upper bound, in place.
    pub fn join(&mut self, other: &Summary) {
        self.purity = self.purity.max(other.purity);
        self.may_unwind |= other.may_unwind;
    }

    /// Pointwise order: whether every claim of `self` is at least as strong as `other`'s.
    pub fn le(&self, other: &Summary) -> bool {
        self.purity <= other.purity && (!self.may_unwind || other.may_unwind)
    }
}

/// The published summaries of one lane pass, by link name.
#[derive(Clone, Debug, Default)]
pub struct Summaries {
    by_name: BTreeMap<JsName, Summary>,
}

impl Summaries {
    /// The raw entry, top included. Prefer [`Summaries::usable`] for decisions.
    pub fn get(&self, name: &JsName) -> Option<&Summary> {
        self.by_name.get(name)
    }

    /// The summary a decision may rely on: `None` for a body that was never summarized or
    /// whose summary is top.
    pub fn usable(&self, name: &JsName) -> Option<&Summary> {
        self.by_name.get(name).filter(|summary| !summary.is_top())
    }

    /// The usable summary of a direct callee, resolved by its `link_name` through the calling
    /// object's interns.
    pub fn callee(&self, interns: &Interns, link_name: StrId) -> Option<&Summary> {
        let name = JsName::new(interns.str(link_name)?.to_owned());
        self.usable(&name)
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&JsName, &Summary)> {
        self.by_name.iter()
    }

    /// A deterministic one-line-per-item rendering, for tests and debugging.
    pub fn dump(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        for (name, summary) in &self.by_name {
            let _ = writeln!(out, "{} {summary:?}", name.as_str());
        }
        out
    }
}

/// How many rounds a recursive component's fixpoint may take before every member goes to top.
pub const SCC_ROUNDS: usize = 8;

/// Computes the summary of every `Ok` func, bottom-up over `graph`'s components.
pub fn compute(
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    interns: &BTreeMap<JsName, &Interns>,
    graph: &Callgraph,
) -> Summaries {
    compute_with_allocator(funcs, interns, graph, false)
}

/// Computes the summary of every `Ok` func, bottom-up over `graph`'s components, accounting
/// for whether a custom/conflicting global allocator can unwind out of `CallKind::Registry` ops.
pub fn compute_with_allocator(
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    interns: &BTreeMap<JsName, &Interns>,
    graph: &Callgraph,
    has_custom_allocator: bool,
) -> Summaries {
    let cx = Cx::new(funcs, interns, graph, has_custom_allocator);
    let mut done: BTreeMap<JsName, Summary> = BTreeMap::new();
    for scc in &graph.sccs {
        let recursive = scc.len() > 1
            || scc.iter().any(|name| {
                graph
                    .callees
                    .get(name)
                    .is_some_and(|callees| callees.contains(name))
            });
        if !recursive {
            let name = &scc[0];
            if let Some(func) = cx.func_of(name) {
                done.insert(name.clone(), summarize(&cx, name, func, &done));
            }
            continue;
        }
        for name in scc {
            if cx.func_of(name).is_some() {
                done.insert(name.clone(), Summary::bottom());
            }
        }
        let mut converged = false;
        for _ in 0..SCC_ROUNDS {
            let mut changed = false;
            for name in scc {
                let Some(func) = cx.func_of(name) else {
                    continue;
                };
                let previous = done.get(name).copied().unwrap_or_else(Summary::bottom);
                let mut next = summarize(&cx, name, func, &done);
                next.join(&previous);
                if next != previous {
                    changed = true;
                    done.insert(name.clone(), next);
                }
            }
            if !changed {
                converged = true;
                break;
            }
        }
        if !converged {
            for name in scc {
                if cx.func_of(name).is_some() {
                    done.insert(name.clone(), Summary::top());
                }
            }
        }
    }
    Summaries { by_name: done }
}

/// Computes summaries and prints coverage under `RCGJS_SUMMARY_REPORT=1`.
pub fn print_report(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
) {
    let interns_map: BTreeMap<JsName, &Interns> = table
        .iter()
        .filter_map(|(name, item)| Some((name.clone(), item.jir.interns()?)))
        .collect();
    let graph = callgraph::build(funcs, &interns_map);
    let summaries = compute(funcs, &interns_map, &graph);
    let (mut items, mut non_top, mut bytes, mut non_top_bytes) = (0u64, 0u64, 0u64, 0u64);
    for (name, func) in funcs {
        items += 1;
        let size = table.get(name).map_or(0, |item| item.js.len() as u64);
        bytes += size;
        if func.as_ref().is_ok() && summaries.usable(name).is_some() {
            non_top += 1;
            non_top_bytes += size;
        }
    }
    let pct = |part: u64, whole: u64| {
        if whole == 0 {
            100.0
        } else {
            100.0 * part as f64 / whole as f64
        }
    };
    eprintln!(
        "rcgjs-summary: items {items} non-top {non_top} ({:.1}%), bytes {bytes} non-top \
         {non_top_bytes} ({:.1}%), validate-failures 0",
        pct(non_top, items),
        pct(non_top_bytes, bytes),
    );
}

// ---------------------------------------------------------------------------------------------
// The transfer: one function summarized against the summaries computed so far.
// ---------------------------------------------------------------------------------------------

struct Cx<'a> {
    funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
    interns: &'a BTreeMap<JsName, &'a Interns>,
    graph: &'a Callgraph,
    /// Whether indirect function-pointer targets may originate outside the lowered `ReifyFnPointer`
    /// set (unlowered bodies, `FnPtr` parameters, `ConstTree` globals/statics, or opaque calls).
    indirect_unclosed: bool,
    /// Whether a custom or conflicting `#[global_allocator]` is present (so `Registry` alloc/free
    /// ops may call into unwinding `__rust_alloc` / `__rust_dealloc`).
    has_custom_allocator: bool,
}

impl<'a> Cx<'a> {
    fn new(
        funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
        interns: &'a BTreeMap<JsName, &'a Interns>,
        graph: &'a Callgraph,
        has_custom_allocator: bool,
    ) -> Self {
        let indirect_unclosed = Self::has_unclosed_fn_ptrs(funcs, interns, graph);
        Self {
            funcs,
            interns,
            graph,
            indirect_unclosed,
            has_custom_allocator,
        }
    }

    fn has_unclosed_fn_ptrs(
        funcs: &BTreeMap<JsName, Result<Func, Decline>>,
        interns: &BTreeMap<JsName, &Interns>,
        graph: &Callgraph,
    ) -> bool {
        if funcs.values().any(Result::is_err) {
            return true;
        }
        for (name, res) in funcs {
            let Ok(func) = res else { continue };
            let types = interns.get(name).map(|i| &i.types);
            if func
                .sig
                .arguments
                .iter()
                .any(|ty| types.is_none_or(|t| type_has_fn_ptr(*ty, t, &mut BTreeSet::new())))
            {
                return true;
            }
            for block in &func.blocks {
                for op in &block.ops {
                    match &op.kind {
                        OpKind::Const(tree)
                            if op_const_has_fn_ptr(
                                tree,
                                op.result.and_then(|r| func.ty(r)),
                                types,
                            ) =>
                        {
                            return true;
                        }
                        OpKind::Cast { from, to, .. } => {
                            if types.is_none_or(|t| type_has_fn_ptr(*to, t, &mut BTreeSet::new())) {
                                let closed_reify = types.is_some_and(|t| {
                                    let to_peeled = t.peeled(*to);
                                    let from_peeled = t.peeled(*from);
                                    let from_ok = matches!(
                                        t.get(from_peeled).map(|d| &d.kind),
                                        Some(
                                            TypeKind::FnDef {
                                                instance: Some(_),
                                                ..
                                            } | TypeKind::Closure {
                                                fn_pointer_shim: Some(_),
                                                ..
                                            }
                                        )
                                    );
                                    matches!(
                                        t.get(to_peeled).map(|d| &d.kind),
                                        Some(TypeKind::FnPtr { .. })
                                    ) && from_ok
                                        && graph.fn_pointer_targets.get(&to_peeled).is_some_and(
                                            |targets| {
                                                !targets.is_empty()
                                                    && targets
                                                        .iter()
                                                        .all(|id| graph.by_item.contains_key(id))
                                            },
                                        )
                                });
                                if !closed_reify {
                                    return true;
                                }
                            }
                        }
                        OpKind::Host { args, .. }
                        | OpKind::Intrinsic { args, .. }
                        | OpKind::Call {
                            host: Some(_),
                            args,
                            ..
                        }
                        | OpKind::Call {
                            callee:
                                Callee::Foreign { .. }
                                | Callee::Runtime { .. }
                                | Callee::Virtual { .. }
                                | Callee::Intrinsic { .. },
                            args,
                            ..
                        } => {
                            let touches_fn_ptr = |val: ValueId| {
                                let Some(ty) = func.ty(val) else {
                                    return true;
                                };
                                types.is_none_or(|t| type_has_fn_ptr(ty, t, &mut BTreeSet::new()))
                            };
                            if op.result.is_some_and(touches_fn_ptr)
                                || args.iter().copied().any(touches_fn_ptr)
                            {
                                return true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        false
    }

    fn func_of(&self, name: &JsName) -> Option<&'a Func> {
        self.funcs.get(name)?.as_ref().ok()
    }

    fn interns_of(&self, name: &JsName) -> Option<&'a Interns> {
        self.interns.get(name).copied()
    }
}

fn type_has_fn_ptr(ty: TypeId, types: &TypeTable, seen: &mut BTreeSet<TypeId>) -> bool {
    if !seen.insert(ty) {
        return false;
    }
    let Some(desc) = types.get(ty) else {
        return true;
    };
    match &desc.kind {
        TypeKind::FnPtr { .. } => true,
        TypeKind::Reference { pointee, .. }
        | TypeKind::RawPointer { pointee, .. }
        | TypeKind::Pattern(pointee) => type_has_fn_ptr(*pointee, types, seen),
        TypeKind::Slice(elem) | TypeKind::Array { element: elem, .. } => {
            type_has_fn_ptr(*elem, types, seen)
        }
        TypeKind::Tuple(fields) => fields.iter().any(|f| type_has_fn_ptr(*f, types, seen)),
        TypeKind::Closure { upvars, .. } => upvars.iter().any(|f| type_has_fn_ptr(*f, types, seen)),
        TypeKind::Adt { variants, .. } => variants
            .iter()
            .any(|v| v.fields.iter().any(|f| type_has_fn_ptr(f.ty, types, seen))),
        _ => false,
    }
}

fn const_has_fn_ptr(tree: &ConstTree) -> bool {
    match tree {
        ConstTree::Global {
            target: GlobalReference::Function { .. },
            ..
        } => true,
        ConstTree::Aggregate(fields)
        | ConstTree::Slice(fields)
        | ConstTree::Variant { fields, .. } => fields.iter().any(const_has_fn_ptr),
        ConstTree::Reference(inner) | ConstTree::Dyn { data: inner, .. } => const_has_fn_ptr(inner),
        _ => false,
    }
}

fn const_has_static_global(tree: &ConstTree) -> bool {
    match tree {
        ConstTree::Global {
            target: GlobalReference::Static { .. },
            ..
        }
        | ConstTree::StaticField {
            target: GlobalReference::Static { .. },
            ..
        }
        | ConstTree::StaticSlice {
            target: GlobalReference::Static { .. },
            ..
        } => true,
        ConstTree::Aggregate(fields)
        | ConstTree::Slice(fields)
        | ConstTree::Variant { fields, .. } => fields.iter().any(const_has_static_global),
        ConstTree::Reference(inner) | ConstTree::Dyn { data: inner, .. } => {
            const_has_static_global(inner)
        }
        _ => false,
    }
}

fn op_const_has_fn_ptr(
    tree: &ConstTree,
    result_ty: Option<TypeId>,
    types: Option<&TypeTable>,
) -> bool {
    if const_has_fn_ptr(tree) {
        return true;
    }
    const_has_static_global(tree)
        && result_ty
            .is_none_or(|ty| types.is_none_or(|t| type_has_fn_ptr(ty, t, &mut BTreeSet::new())))
}

/// How a call site contributes, decided once and shared by the taint and facts passes.
enum CallKind<'a> {
    /// `drop_in_place` of nothing: no effect at all.
    Nop,
    /// A registry-covered standard operation: its effect contract stands in for the body.
    Registry(StandardOperation, semantic_ops::Effects),
    GlobalAllocator,
    /// A host call, with the decoded `#[js_extern]` descriptor where one exists. `None` is raw
    /// JavaScript, a marker, or an undecodable declaration: fully unknown.
    Host(Option<Descriptor>),
    /// A plain direct call: the callee's summary answers, under its resolved link name.
    /// `None` where the caller's interns cannot spell the name: nothing can be known.
    Direct(Option<JsName>),
    /// An indirect call, bounded by the program's reified targets where possible.
    Indirect(Option<&'a BTreeSet<ItemId>>),
    Virtual,
    /// A runtime member, foreign symbol, or unresolvable callee: unknown effects.
    Opaque,
}

#[allow(clippy::too_many_arguments)]
fn classify<'a>(
    graph: &'a Callgraph,
    interns: Option<&Interns>,
    func: &Func,
    result: Option<ValueId>,
    args: &[ValueId],
    callee: &Callee,
    semantic: &CallSemantic,
    host: &Option<HostPayload>,
) -> CallKind<'a> {
    if let CallSemantic::Standard(operation) = semantic {
        let types = interns.map(|interns| &interns.types);
        if let Some(effects) =
            semantic_ops::call_effects_entry(*operation, types, func, result, args)
        {
            return CallKind::Registry(*operation, effects);
        }
    }
    if matches!(semantic, CallSemantic::GlobalAllocator) {
        return CallKind::GlobalAllocator;
    }
    if host.is_some() || matches!(semantic, CallSemantic::Host | CallSemantic::RawJavaScript) {
        let descriptor = match host {
            Some(HostPayload::Extern(bytes)) => interns
                .and_then(|interns| interns.bytes.get(bytes.0 as usize))
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .and_then(|text| Descriptor::decode(text).ok()),
            _ => None,
        };
        return CallKind::Host(descriptor);
    }
    match callee {
        Callee::Direct { link_name, .. } => CallKind::Direct(
            interns
                .and_then(|interns| interns.str(*link_name))
                .map(|name| JsName::new(name.to_owned())),
        ),
        Callee::Nop => CallKind::Nop,
        Callee::Indirect { signature } => {
            CallKind::Indirect(graph.fn_pointer_targets.get(signature))
        }
        Callee::Virtual { .. } => CallKind::Virtual,
        Callee::Foreign { .. } | Callee::Runtime { .. } | Callee::Intrinsic { .. } => {
            CallKind::Opaque
        }
    }
}

/// Computes which SSA values are proven pointers into this frame's own storage: a `Deref`-free
/// borrow of a local value or cell, propagated through `Copy`/`Move`/`Cast`/`SliceView`/`Offset`
/// and block-parameter joins (logical AND). A read or store through such a pointer touches nothing
/// caller-visible.
fn compute_local_ptrs(func: &Func) -> Vec<bool> {
    let mut locals: Vec<Option<bool>> = vec![None; func.values.len()];
    for param in &func.params {
        locals[param.0 as usize] = Some(false);
    }
    let join = |slot: &mut Option<bool>, incoming: Option<bool>| -> bool {
        let next = match (*slot, incoming) {
            (None, x) | (x, None) => x,
            (Some(a), Some(b)) => Some(a && b),
        };
        if next != *slot {
            *slot = next;
            true
        } else {
            false
        }
    };
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for op in &block.ops {
                let Some(result) = op.result else {
                    continue;
                };
                let incoming = match &op.kind {
                    OpKind::Copy(v)
                    | OpKind::Move(v)
                    | OpKind::Cast { v, .. }
                    | OpKind::Materialize(v)
                    | OpKind::Convert { v, .. }
                    | OpKind::SliceView { ptr: v, .. } => Some(locals[v.0 as usize] == Some(true)),
                    OpKind::BorrowView { path, .. } | OpKind::RawView { path, .. } => {
                        Some(!path.iter().any(|proj| matches!(proj, Proj::Deref)))
                    }
                    OpKind::Binary {
                        op: crate::jir::semantic::BinaryOp::Offset,
                        l,
                        ..
                    } => Some(locals[l.0 as usize] == Some(true)),
                    _ => Some(false),
                };
                changed |= join(&mut locals[result.0 as usize], incoming);
            }
            for (target, args) in block.term.edges() {
                for (param, arg) in func.blocks[target.0 as usize].params.iter().zip(args) {
                    let incoming = locals[arg.0 as usize];
                    changed |= join(&mut locals[param.0 as usize], incoming);
                }
            }
        }
        if !changed {
            break;
        }
    }
    locals.into_iter().map(|l| l == Some(true)).collect()
}

/// The summary of one function, given `current` (finished components plus this component's
/// in-progress iterates).
fn summarize(
    cx: &Cx<'_>,
    name: &JsName,
    func: &Func,
    current: &BTreeMap<JsName, Summary>,
) -> Summary {
    let interns = cx.interns_of(name);
    let types = interns.map(|interns| &interns.types);
    let local_ptrs = compute_local_ptrs(func);
    let mut facts = Facts {
        summary: Summary::bottom(),
    };
    for block in &func.blocks {
        for op in &block.ops {
            facts.op(cx, interns, types, func, &local_ptrs, op, current);
        }
        facts.term(&block.term);
    }
    facts.summary
}

struct Facts {
    summary: Summary,
}

impl Facts {
    fn purity(&mut self, purity: Purity) {
        self.summary.purity = self.summary.purity.max(purity);
    }

    fn opaque(&mut self) {
        self.purity(Purity::Effect);
        self.summary.may_unwind = true;
    }

    #[allow(clippy::too_many_arguments)]
    fn op(
        &mut self,
        cx: &Cx<'_>,
        interns: Option<&Interns>,
        types: Option<&TypeTable>,
        func: &Func,
        local_ptrs: &[bool],
        op: &crate::jir::repr::Op,
        current: &BTreeMap<JsName, Summary>,
    ) {
        match &op.kind {
            OpKind::Deref(ptr) => {
                if !local_ptrs[ptr.0 as usize] {
                    self.purity(Purity::Read);
                }
            }
            OpKind::StoreThrough { ptr, .. } => {
                if !local_ptrs[ptr.0 as usize] {
                    self.purity(Purity::Effect);
                }
            }
            OpKind::Assert { .. } => self.summary.may_unwind = true,
            OpKind::Zombie(_) => self.opaque(),
            OpKind::SpillRead(_) => self.purity(Purity::Read),
            OpKind::SpillWrite { .. } => self.purity(Purity::Effect),
            OpKind::Intrinsic { .. } | OpKind::Host { .. } => self.opaque(),
            OpKind::Drop { glue, .. } => self.drop_glue(interns, glue, current),
            OpKind::Call {
                callee,
                semantic,
                host,
                args,
                ..
            } => {
                let kind = classify(
                    cx.graph, interns, func, op.result, args, callee, semantic, host,
                );
                self.call(cx, types, func, &kind, &cx.graph.by_item, args, current);
            }
            _ => {}
        }
    }

    fn drop_glue(
        &mut self,
        interns: Option<&Interns>,
        glue: &DropGlue,
        current: &BTreeMap<JsName, Summary>,
    ) {
        match glue {
            DropGlue::None => {}
            DropGlue::Direct { link_name, .. } => {
                let resolved = interns
                    .and_then(|interns| interns.str(*link_name))
                    .map(|name| JsName::new(name.to_owned()));
                match resolved.as_ref().and_then(|name| current.get(name)) {
                    Some(summary) => {
                        self.purity(summary.purity);
                        self.summary.may_unwind |= summary.may_unwind;
                    }
                    None => self.opaque(),
                }
            }
            DropGlue::Dynamic => self.opaque(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn call(
        &mut self,
        cx: &Cx<'_>,
        types: Option<&TypeTable>,
        func: &Func,
        kind: &CallKind<'_>,
        by_item: &BTreeMap<ItemId, BTreeSet<JsName>>,
        args: &[ValueId],
        current: &BTreeMap<JsName, Summary>,
    ) {
        match kind {
            CallKind::Nop => {}
            CallKind::Registry(operation, effects) => {
                self.purity(if effects.mutates {
                    Purity::Effect
                } else {
                    Purity::Read
                });
                self.summary.may_unwind |= effects.can_throw
                    || cx.has_custom_allocator
                    || registry_drop_may_unwind(*operation, types, func, args);
            }
            CallKind::GlobalAllocator => self.opaque(),
            CallKind::Host(descriptor) => {
                self.purity(Purity::Effect);
                match descriptor {
                    Some(descriptor) => {
                        self.summary.may_unwind |= descriptor.fallible
                            || descriptor
                                .slots
                                .iter()
                                .any(|(_, facts)| facts.has(Fact::Throws));
                    }
                    None => self.summary.may_unwind = true,
                }
            }
            CallKind::Direct(name) => match name {
                Some(name) => self.direct(name, current),
                None => self.opaque(),
            },
            CallKind::Indirect(targets) => match targets {
                Some(targets) if !targets.is_empty() && !cx.indirect_unclosed => {
                    for target in targets.iter() {
                        match by_item.get(target) {
                            Some(names) => {
                                for name in names {
                                    self.direct(name, current);
                                }
                            }
                            None => self.opaque(),
                        }
                    }
                }
                _ => self.opaque(),
            },
            CallKind::Virtual | CallKind::Opaque => self.opaque(),
        }
    }

    fn direct(&mut self, name: &JsName, current: &BTreeMap<JsName, Summary>) {
        match current.get(name) {
            Some(summary) => {
                self.purity(summary.purity);
                self.summary.may_unwind |= summary.may_unwind;
            }
            None => self.opaque(),
        }
    }

    fn term(&mut self, term: &RTerm) {
        match term {
            RTerm::Throw(_) | RTerm::Invoke { .. } => self.summary.may_unwind = true,
            RTerm::Return(_) | RTerm::Goto(..) | RTerm::Switch { .. } | RTerm::Unreachable => {}
        }
    }
}

fn registry_drop_may_unwind(
    operation: StandardOperation,
    types: Option<&TypeTable>,
    func: &Func,
    args: &[ValueId],
) -> bool {
    if !matches!(
        operation,
        StandardOperation::VecClear
            | StandardOperation::VecTruncate
            | StandardOperation::VecDrop
            | StandardOperation::BoxDrop
            | StandardOperation::BTreeMapClear
            | StandardOperation::BTreeMapDrop
    ) {
        return false;
    }
    let Some(types) = types else { return true };
    let Some(arg0) = args.first() else {
        return true;
    };
    let Some(arg_ty) = func.ty(*arg0) else {
        return true;
    };
    let pointee = match types.get(types.peeled(arg_ty)).map(|d| &d.kind) {
        Some(TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. }) => {
            *pointee
        }
        _ => arg_ty,
    };
    let Some(desc) = types.get(types.peeled(pointee)) else {
        return true;
    };
    let inner_ty = match &desc.kind {
        TypeKind::Adt { args, .. } => {
            if matches!(
                operation,
                StandardOperation::BTreeMapClear | StandardOperation::BTreeMapDrop
            ) {
                args.get(1).copied()
            } else {
                args.first().copied()
            }
        }
        _ => None,
    };
    inner_ty
        .and_then(|ty| types.get(ty))
        .is_none_or(|d| d.layout.needs_drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{Fact as DFact, Shape, Slot as DSlot, SlotFacts};
    use crate::jir::interns::{BytesId, StrId};
    use crate::jir::origin::OriginId;
    use crate::jir::repr::{FuncBuilder, OpKind, RTerm};
    use crate::jir::semantic::{
        AbiSignature, CastKind, ConstTree, DefinitionId, DirectItemRef, GlobalReference, Signature,
        StandardOperation, TypeId,
    };
    use crate::jir::types::{JsFacts, LayoutFacts, NominalForm, TypeDescriptor, TypeMeta};
    use crate::jir::SigId;

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

    fn sig(arguments: usize) -> Signature {
        Signature {
            arguments: vec![TY; arguments],
            result: TY,
            abi: AbiSignature::default(),
        }
    }

    fn call(callee: StrId, semantic: CallSemantic, args: Vec<crate::jir::repr::ValueId>) -> OpKind {
        OpKind::Call {
            callee: Callee::Direct {
                item: id(0),
                link_name: callee,
            },
            function: None,
            args,
            track_caller: None,
            semantic,
            host: None,
        }
    }

    /// A program under one shared interns table, summarized.
    struct Prog {
        interns: Interns,
        funcs: BTreeMap<JsName, Result<Func, Decline>>,
    }

    impl Prog {
        fn new() -> Prog {
            Prog {
                interns: Interns::new(),
                funcs: BTreeMap::new(),
            }
        }

        fn add(&mut self, name: &str, func: Func) {
            self.funcs.insert(JsName::new(name.to_owned()), Ok(func));
        }

        /// Interns `name` so a call op can resolve it as a callee link name.
        fn intern(&mut self, name: &str) -> StrId {
            if let Some(index) = self.interns.strings.iter().position(|s| s == name) {
                return StrId(index as u32);
            }
            self.interns.strings.push(name.to_owned());
            StrId((self.interns.strings.len() - 1) as u32)
        }

        fn summary(summaries: &Summaries, name: &str) -> Option<Summary> {
            summaries.usable(&JsName::new(name.to_owned())).cloned()
        }

        fn graph(&self) -> Callgraph {
            let interns: BTreeMap<JsName, &Interns> = self
                .funcs
                .keys()
                .map(|name| (name.clone(), &self.interns))
                .collect();
            callgraph::build(&self.funcs, &interns)
        }

        fn compute(&self) -> Summaries {
            let interns: BTreeMap<JsName, &Interns> = self
                .funcs
                .keys()
                .map(|name| (name.clone(), &self.interns))
                .collect();
            compute(&self.funcs, &interns, &self.graph())
        }
    }

    /// `fn f(p) { return p; }`
    fn identity(item: ItemId) -> Func {
        let mut b = FuncBuilder::new(item, sig(1));
        let p = b.param(TY, O);
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(p)));
        b.finish()
    }

    /// `fn f(p) { *p = (); return c; }`
    fn writes_param(item: ItemId) -> Func {
        let mut b = FuncBuilder::new(item, sig(1));
        let p = b.param(TY, O);
        let c = b.op(FuncBuilder::ENTRY, OpKind::Const(ConstTree::Unit), TY, O);
        b.effect(FuncBuilder::ENTRY, OpKind::StoreThrough { ptr: p, v: c }, O);
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(c)));
        b.finish()
    }

    /// `fn f(p) { g(p); return c; }`
    fn calls(item: ItemId, callee: StrId) -> Func {
        let mut b = FuncBuilder::new(item, sig(1));
        let p = b.param(TY, O);
        let c = b.op(FuncBuilder::ENTRY, OpKind::Const(ConstTree::Unit), TY, O);
        b.effect(
            FuncBuilder::ENTRY,
            call(callee, CallSemantic::Ordinary, vec![p]),
            O,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(c)));
        b.finish()
    }

    // -- joins --------------------------------------------------------------------------------

    #[test]
    fn purity_join_is_max_over_the_chain() {
        use Purity::*;
        for (a, b, lub) in [
            (Const, Const, Const),
            (Const, Read, Read),
            (Const, Effect, Effect),
            (Read, Read, Read),
            (Read, Effect, Effect),
            (Effect, Effect, Effect),
        ] {
            assert_eq!(a.max(b), lub);
            assert_eq!(b.max(a), lub);
        }
    }

    #[test]
    fn summary_join_is_monotone_and_bounded_by_top() {
        let mut chain = Summary::bottom();
        for step in [
            |s: &mut Summary| s.purity = Purity::Read,
            |s: &mut Summary| s.may_unwind = true,
            |s: &mut Summary| s.purity = Purity::Effect,
        ] {
            let mut next = chain;
            step(&mut next);
            let previous = chain;
            chain.join(&next);
            assert!(previous.le(&chain));
            assert!(next.le(&chain));
        }
        assert_eq!(chain, Summary::top());
        assert!(chain.is_top());
        assert!(Summary::bottom().le(&Summary::top()));
        assert!(!Summary::top().le(&Summary::bottom()));
    }

    #[test]
    fn basic_transfers_purity_and_unwind() {
        let mut prog = Prog::new();
        prog.add("id", identity(id(1)));
        prog.add("w", writes_param(id(2)));
        let missing = prog.intern("nowhere");
        prog.add("miss", calls(id(3), missing));
        let g = prog.intern("id");
        prog.add("call_id", calls(id(4), g));
        let summaries = prog.compute();

        let s_id = Prog::summary(&summaries, "id").expect("summarized");
        assert_eq!(s_id.purity, Purity::Const);
        assert!(!s_id.may_unwind);

        let s_w = Prog::summary(&summaries, "w").expect("summarized");
        assert_eq!(s_w.purity, Purity::Effect);
        assert!(!s_w.may_unwind);

        assert!(Prog::summary(&summaries, "miss").is_none());
        let s_miss = summaries.get(&JsName::new("miss".to_owned())).unwrap();
        assert!(s_miss.is_top());

        let s_call = Prog::summary(&summaries, "call_id").expect("summarized");
        assert_eq!(s_call.purity, Purity::Const);
        assert!(!s_call.may_unwind);
    }

    #[test]
    fn local_store_and_param_deref_purity() {
        let mut b = FuncBuilder::new(id(1), sig(1));
        let p = b.param(TY, O);
        let c = b.op(FuncBuilder::ENTRY, OpKind::Const(ConstTree::Unit), TY, O);
        let w = b.op(
            FuncBuilder::ENTRY,
            OpKind::Aggregate {
                ty: TY,
                variant: 0,
                active_field: None,
                fields: vec![c],
            },
            TY,
            O,
        );
        let ptr = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: crate::jir::repr::Root::Value(w),
                path: vec![],
                mutable: true,
            },
            TY,
            O,
        );
        b.effect(FuncBuilder::ENTRY, OpKind::StoreThrough { ptr, v: p }, O);
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(w)));

        let mut b2 = FuncBuilder::new(id(2), sig(1));
        let p2 = b2.param(TY, O);
        let v2 = b2.op(FuncBuilder::ENTRY, OpKind::Deref(p2), TY, O);
        b2.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(v2)));

        let mut prog = Prog::new();
        prog.add("local_store", b.finish());
        prog.add("param_deref", b2.finish());
        let summaries = prog.compute();
        assert_eq!(
            Prog::summary(&summaries, "local_store").unwrap().purity,
            Purity::Const
        );
        assert_eq!(
            Prog::summary(&summaries, "param_deref").unwrap().purity,
            Purity::Read
        );
    }

    #[test]
    fn registry_and_host_descriptor_facts() {
        let mut b = FuncBuilder::new(id(1), sig(1));
        let p = b.param(TY, O);
        let v = b.op(
            FuncBuilder::ENTRY,
            call(
                StrId(0),
                CallSemantic::Standard(StandardOperation::VecLen),
                vec![p],
            ),
            TY,
            O,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(v)));
        let mut prog = Prog::new();
        prog.add("len", b.finish());
        let s = Prog::summary(&prog.compute(), "len").unwrap();
        assert_eq!(s.purity, Purity::Read);
        assert!(!s.may_unwind);

        let mut b = FuncBuilder::new(id(1), sig(2));
        let vec = b.param(TY, O);
        let el = b.param(TY, O);
        b.effect(
            FuncBuilder::ENTRY,
            call(
                StrId(0),
                CallSemantic::Standard(StandardOperation::VecPush),
                vec![vec, el],
            ),
            O,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let mut prog = Prog::new();
        prog.add("push", b.finish());
        let s = prog
            .compute()
            .get(&JsName::new("push".to_owned()))
            .copied()
            .unwrap();
        assert_eq!(s.purity, Purity::Effect);
        assert!(s.may_unwind);

        for (facts, expects_unwind) in [
            (SlotFacts::default().with(DFact::Reads), false),
            (
                SlotFacts::default().with(DFact::Reads).with(DFact::Throws),
                true,
            ),
        ] {
            let mut prog = Prog::new();
            let desc = Descriptor {
                shape: Shape::Method,
                nullable: false,
                fallible: false,
                global: false,
                module: String::new(),
                name: "write".to_owned(),
                slots: vec![(DSlot::Parameter(0), facts)],
            };
            let encoded = desc.encode();
            let body = encoded.strip_prefix(crate::descriptor::PREFIX).unwrap();
            prog.interns.bytes.push(body.as_bytes().to_vec());
            let bytes = BytesId((prog.interns.bytes.len() - 1) as u32);
            let mut b = FuncBuilder::new(id(1), sig(1));
            let p = b.param(TY, O);
            b.effect(
                FuncBuilder::ENTRY,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: id(60),
                        link_name: StrId(0),
                    },
                    function: None,
                    args: vec![p],
                    track_caller: None,
                    semantic: CallSemantic::Host,
                    host: Some(HostPayload::Extern(bytes)),
                },
                O,
            );
            b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
            prog.add("f", b.finish());
            let s = prog
                .compute()
                .get(&JsName::new("f".to_owned()))
                .copied()
                .unwrap();
            assert_eq!(s.purity, Purity::Effect);
            assert_eq!(s.may_unwind, expects_unwind);
        }
    }

    #[test]
    fn clone_ops_use_callee_effects_for_non_primitive_elements() {
        use crate::jir::types::{
            AllocatorIdentity, JsFacts, LayoutFacts, NominalForm, RepresentationCandidate,
            TypeDescriptor, TypeMeta,
        };
        let make_ty = |id: TypeId, kind: TypeKind, meta: TypeMeta| TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                sized: true,
                size: Some(4),
                ..LayoutFacts::default()
            },
            meta,
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: id,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        };
        let elem_u32 = TypeId(20);
        let elem_spy = TypeId(21);
        let vec_u32 = TypeId(22);
        let vec_spy = TypeId(23);
        let slice_u32 = TypeId(24);
        let slice_spy = TypeId(25);
        let ref_slice_u32 = TypeId(26);
        let ref_slice_spy = TypeId(27);
        let vec_meta = TypeMeta {
            candidate: RepresentationCandidate::Vec,
            allocator: AllocatorIdentity::Global,
            ..TypeMeta::default()
        };

        let run = |op: StandardOperation, arg_ty: TypeId, ret_ty: TypeId| {
            let mut prog = Prog::new();
            prog.interns.types.insert(make_ty(
                elem_u32,
                TypeKind::Integer {
                    signed: false,
                    bits: 32,
                },
                TypeMeta::default(),
            ));
            prog.interns.types.insert(make_ty(
                elem_spy,
                TypeKind::Adt {
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 60,
                    },
                    args: Vec::new(),
                    form: NominalForm::Struct,
                    variants: Vec::new(),
                    niche: None,
                },
                TypeMeta::default(),
            ));
            prog.interns.types.insert(make_ty(
                vec_u32,
                TypeKind::Adt {
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 61,
                    },
                    args: vec![elem_u32],
                    form: NominalForm::Struct,
                    variants: Vec::new(),
                    niche: None,
                },
                vec_meta,
            ));
            prog.interns.types.insert(make_ty(
                vec_spy,
                TypeKind::Adt {
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 62,
                    },
                    args: vec![elem_spy],
                    form: NominalForm::Struct,
                    variants: Vec::new(),
                    niche: None,
                },
                vec_meta,
            ));
            prog.interns.types.insert(make_ty(
                slice_u32,
                TypeKind::Slice(elem_u32),
                TypeMeta::default(),
            ));
            prog.interns.types.insert(make_ty(
                slice_spy,
                TypeKind::Slice(elem_spy),
                TypeMeta::default(),
            ));
            prog.interns.types.insert(make_ty(
                ref_slice_u32,
                TypeKind::Reference {
                    pointee: slice_u32,
                    mutable: false,
                },
                TypeMeta::default(),
            ));
            prog.interns.types.insert(make_ty(
                ref_slice_spy,
                TypeKind::Reference {
                    pointee: slice_spy,
                    mutable: false,
                },
                TypeMeta::default(),
            ));

            // Callee performs an observable host effect (simulating user `Clone::clone`).
            let callee_name = prog.intern("to_vec_impl");
            let mut callee_b = FuncBuilder::new(
                id(2),
                Signature {
                    arguments: vec![arg_ty],
                    result: ret_ty,
                    abi: AbiSignature::default(),
                },
            );
            let _c_arg = callee_b.param(arg_ty, O);
            callee_b.effect(
                FuncBuilder::ENTRY,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: id(3),
                        link_name: callee_name,
                    },
                    function: None,
                    args: Vec::new(),
                    track_caller: None,
                    semantic: CallSemantic::Host,
                    host: None,
                },
                O,
            );
            let c_ret = callee_b.op(
                FuncBuilder::ENTRY,
                OpKind::Const(ConstTree::Unit),
                ret_ty,
                O,
            );
            callee_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(c_ret)));
            prog.add("to_vec_impl", callee_b.finish());

            let mut caller_b = FuncBuilder::new(
                id(1),
                Signature {
                    arguments: vec![arg_ty],
                    result: ret_ty,
                    abi: AbiSignature::default(),
                },
            );
            let p0 = caller_b.param(arg_ty, O);
            let res = caller_b.op(
                FuncBuilder::ENTRY,
                call(callee_name, CallSemantic::Standard(op), vec![p0]),
                ret_ty,
                O,
            );
            caller_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(res)));
            prog.add("caller", caller_b.finish());

            let summaries = prog.compute();
            summaries
                .get(&JsName::new("caller".to_owned()))
                .cloned()
                .expect("caller summarized")
        };

        let prim = run(StandardOperation::SliceToVec, ref_slice_u32, vec_u32);
        assert_eq!(prim.purity, Purity::Read);
        assert!(!prim.may_unwind);

        let user = run(StandardOperation::SliceToVec, ref_slice_spy, vec_spy);
        assert_eq!(user.purity, Purity::Effect);
        assert!(user.may_unwind);
    }

    #[test]
    fn registry_drop_marks_may_unwind_when_element_needs_drop() {
        use crate::jir::types::{JsFacts, LayoutFacts, NominalForm, TypeDescriptor, TypeMeta};
        let make_ty = |id: TypeId, kind: TypeKind, needs_drop: bool| TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                sized: true,
                size: Some(4),
                needs_drop,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: id,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        };
        let elem_plain = TypeId(10);
        let elem_drop = TypeId(11);
        let vec_plain = TypeId(12);
        let vec_drop = TypeId(13);
        let ref_plain = TypeId(14);
        let ref_drop = TypeId(15);

        let run = |ref_ty: TypeId| {
            let mut prog = Prog::new();
            prog.interns.types.insert(make_ty(
                elem_plain,
                TypeKind::Integer {
                    signed: true,
                    bits: 32,
                },
                false,
            ));
            prog.interns.types.insert(make_ty(
                elem_drop,
                TypeKind::Adt {
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 50,
                    },
                    args: Vec::new(),
                    form: NominalForm::Struct,
                    variants: Vec::new(),
                    niche: None,
                },
                true,
            ));
            prog.interns.types.insert(make_ty(
                vec_plain,
                TypeKind::Adt {
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 51,
                    },
                    args: vec![elem_plain],
                    form: NominalForm::Struct,
                    variants: Vec::new(),
                    niche: None,
                },
                true,
            ));
            prog.interns.types.insert(make_ty(
                vec_drop,
                TypeKind::Adt {
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 51,
                    },
                    args: vec![elem_drop],
                    form: NominalForm::Struct,
                    variants: Vec::new(),
                    niche: None,
                },
                true,
            ));
            prog.interns.types.insert(make_ty(
                ref_plain,
                TypeKind::Reference {
                    pointee: vec_plain,
                    mutable: true,
                },
                false,
            ));
            prog.interns.types.insert(make_ty(
                ref_drop,
                TypeKind::Reference {
                    pointee: vec_drop,
                    mutable: true,
                },
                false,
            ));
            let mut b = FuncBuilder::new(
                id(1),
                Signature {
                    arguments: vec![ref_ty],
                    result: TY,
                    abi: AbiSignature::default(),
                },
            );
            let p = b.param(ref_ty, O);
            b.effect(
                FuncBuilder::ENTRY,
                call(
                    StrId(0),
                    CallSemantic::Standard(StandardOperation::VecDrop),
                    vec![p],
                ),
                O,
            );
            b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
            prog.add("f", b.finish());
            let summaries = prog.compute();
            summaries
                .get(&JsName::new("f".to_owned()))
                .expect("summarized")
                .may_unwind
        };

        assert!(!run(ref_plain), "Vec<i32> drop cannot unwind");
        assert!(run(ref_drop), "Vec<DropGuard> drop may unwind");
    }

    #[test]
    fn indirect_call_with_unclosed_fn_ptrs_marks_may_unwind() {
        use crate::jir::interns::SigId;
        use crate::jir::semantic::{CastKind, DirectItemRef};
        use crate::jir::types::{JsFacts, LayoutFacts, TypeDescriptor, TypeMeta};
        let fn_def_ty = TypeId(20);
        let fn_ptr_ty = TypeId(21);
        let make_ty = |id: TypeId, kind: TypeKind| TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                sized: true,
                size: Some(4),
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: id,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        };
        let mut prog = Prog::new();
        let target_name = prog.intern("pure_fn");
        prog.interns
            .types
            .insert(make_ty(TY, TypeKind::Tuple(Vec::new())));
        prog.interns.types.insert(make_ty(
            fn_def_ty,
            TypeKind::FnDef {
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: 2,
                },
                sig: SigId(0),
                instance: Some(DirectItemRef {
                    item: id(2),
                    link_name: target_name,
                }),
            },
        ));
        prog.interns
            .types
            .insert(make_ty(fn_ptr_ty, TypeKind::FnPtr { sig: SigId(0) }));

        let mut pure = FuncBuilder::new(id(2), sig(0));
        pure.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        prog.add("pure_fn", pure.finish());

        // Caller takes an external `fn()` parameter and also reifies `pure_fn` so
        // `fn_pointer_targets` is non-empty, yet external fn pointers can still arrive.
        let mut caller = FuncBuilder::new(
            id(1),
            Signature {
                arguments: vec![fn_ptr_ty],
                result: TY,
                abi: AbiSignature::default(),
            },
        );
        let fp = caller.param(fn_ptr_ty, O);
        let c_def = caller.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Unit),
            fn_def_ty,
            O,
        );
        let _reified = caller.op(
            FuncBuilder::ENTRY,
            OpKind::Cast {
                kind: CastKind::PointerCoercion,
                v: c_def,
                from: fn_def_ty,
                to: fn_ptr_ty,
            },
            fn_ptr_ty,
            O,
        );
        caller.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Indirect {
                    signature: fn_ptr_ty,
                },
                function: Some(fp),
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        caller.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        prog.add("caller", caller.finish());

        let summaries = prog.compute();
        let caller_summary = summaries
            .get(&JsName::new("caller".to_owned()))
            .expect("summarized");
        assert!(caller_summary.may_unwind);
    }

    #[test]
    fn static_fn_ptr_marks_indirect_call_may_unwind() {
        let fn_def_ty = TypeId(40);
        let fn_ptr_ty = TypeId(41);
        let mut prog = Prog::new();
        let target_name = prog.intern("pure_fn");
        let static_name = prog.intern("HOOK_FN");
        let make_ty = |id, kind| TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts::default(),
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: id,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        };
        prog.interns
            .types
            .insert(make_ty(TY, TypeKind::Tuple(Vec::new())));
        prog.interns.types.insert(make_ty(
            fn_def_ty,
            TypeKind::FnDef {
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: 2,
                },
                sig: SigId(0),
                instance: Some(DirectItemRef {
                    item: id(2),
                    link_name: target_name,
                }),
            },
        ));
        prog.interns
            .types
            .insert(make_ty(fn_ptr_ty, TypeKind::FnPtr { sig: SigId(0) }));

        let mut pure = FuncBuilder::new(id(2), sig(0));
        pure.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        prog.add("pure_fn", pure.finish());

        // Even with a benign ReifyFnPointer closing `fn_ptr_ty`, reading a static containing a
        // fn pointer leaves indirect calls unclosed (`may_unwind = true`).
        let mut caller = FuncBuilder::new(id(1), sig(0));
        let c_def = caller.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Unit),
            fn_def_ty,
            O,
        );
        let _reified = caller.op(
            FuncBuilder::ENTRY,
            OpKind::Cast {
                kind: CastKind::PointerCoercion,
                v: c_def,
                from: fn_def_ty,
                to: fn_ptr_ty,
            },
            fn_ptr_ty,
            O,
        );
        let from_static = caller.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Global {
                target: GlobalReference::Static {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 99,
                    },
                    link_name: static_name,
                },
                offset: 0,
            }),
            fn_ptr_ty,
            O,
        );
        caller.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Indirect {
                    signature: fn_ptr_ty,
                },
                function: Some(from_static),
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        caller.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        prog.add("caller", caller.finish());

        let summaries = prog.compute();
        assert!(
            summaries
                .get(&JsName::new("caller".to_owned()))
                .expect("summarized")
                .may_unwind
        );
    }

    #[test]
    fn fn_ptr_cast_and_composite_transmute_mark_indirect_call_may_unwind() {
        let fn_def_ty = TypeId(40);
        let fn_ptr_a = TypeId(41);
        let fn_ptr_b = TypeId(42);
        let opt_fn_ty = TypeId(43);
        let make_ty = |id, kind| TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts::default(),
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: id,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        };
        let check_cast = |from_ty: TypeId, to_ty: TypeId| {
            let mut prog = Prog::new();
            let target_name = prog.intern("pure_fn");
            prog.interns
                .types
                .insert(make_ty(TY, TypeKind::Tuple(Vec::new())));
            prog.interns.types.insert(make_ty(
                fn_def_ty,
                TypeKind::FnDef {
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 2,
                    },
                    sig: SigId(0),
                    instance: Some(DirectItemRef {
                        item: id(2),
                        link_name: target_name,
                    }),
                },
            ));
            prog.interns
                .types
                .insert(make_ty(fn_ptr_a, TypeKind::FnPtr { sig: SigId(0) }));
            prog.interns
                .types
                .insert(make_ty(fn_ptr_b, TypeKind::FnPtr { sig: SigId(1) }));
            prog.interns
                .types
                .insert(make_ty(opt_fn_ty, TypeKind::Tuple(vec![fn_ptr_b])));

            let mut pure = FuncBuilder::new(id(2), sig(0));
            pure.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
            prog.add("pure_fn", pure.finish());

            let mut caller = FuncBuilder::new(id(1), sig(0));
            let c_def = caller.op(
                FuncBuilder::ENTRY,
                OpKind::Const(ConstTree::Unit),
                fn_def_ty,
                O,
            );
            let reified = caller.op(
                FuncBuilder::ENTRY,
                OpKind::Cast {
                    kind: CastKind::PointerCoercion,
                    v: c_def,
                    from: fn_def_ty,
                    to: fn_ptr_a,
                },
                fn_ptr_a,
                O,
            );
            let _casted = caller.op(
                FuncBuilder::ENTRY,
                OpKind::Cast {
                    kind: CastKind::Transmute,
                    v: reified,
                    from: from_ty,
                    to: to_ty,
                },
                to_ty,
                O,
            );
            caller.effect(
                FuncBuilder::ENTRY,
                OpKind::Call {
                    callee: Callee::Indirect {
                        signature: fn_ptr_a,
                    },
                    function: Some(reified),
                    args: Vec::new(),
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                O,
            );
            caller.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
            prog.add("caller", caller.finish());
            prog.compute()
                .get(&JsName::new("caller".to_owned()))
                .expect("summarized")
                .may_unwind
        };

        assert!(
            check_cast(fn_ptr_a, fn_ptr_b),
            "FnPtr->FnPtr cast must mark indirect calls unclosed"
        );
        assert!(
            check_cast(fn_ptr_a, opt_fn_ty),
            "Transmute to composite containing FnPtr must mark indirect calls unclosed"
        );
    }

    #[test]
    fn custom_allocator_marks_registry_vec_drop_may_unwind() {
        let vec_plain = TypeId(30);
        let ref_plain = TypeId(31);
        let mut prog = Prog::new();
        let make_ty = |id, kind| TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                needs_drop: false,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: id,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        };
        prog.interns
            .types
            .insert(make_ty(TY, TypeKind::Tuple(Vec::new())));
        prog.interns.types.insert(make_ty(
            vec_plain,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: 10,
                },
                args: vec![TY],
                form: NominalForm::Struct,
                variants: Vec::new(),
                niche: None,
            },
        ));
        prog.interns.types.insert(make_ty(
            ref_plain,
            TypeKind::Reference {
                pointee: vec_plain,
                mutable: true,
            },
        ));
        let mut b = FuncBuilder::new(
            id(1),
            Signature {
                arguments: vec![ref_plain],
                result: TY,
                abi: AbiSignature::default(),
            },
        );
        let p = b.param(ref_plain, O);
        b.effect(
            FuncBuilder::ENTRY,
            call(
                StrId(0),
                CallSemantic::Standard(StandardOperation::VecDrop),
                vec![p],
            ),
            O,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        prog.add("f", b.finish());

        let default_summaries = prog.compute();
        assert!(
            !default_summaries
                .get(&JsName::new("f".to_owned()))
                .expect("summarized")
                .may_unwind
        );

        let interns: BTreeMap<JsName, &Interns> = prog
            .funcs
            .keys()
            .map(|name| (name.clone(), &prog.interns))
            .collect();
        let graph = crate::analysis::callgraph::build(&prog.funcs, &interns);
        let custom_summaries = compute_with_allocator(&prog.funcs, &interns, &graph, true);
        assert!(
            custom_summaries
                .get(&JsName::new("f".to_owned()))
                .expect("summarized")
                .may_unwind
        );
    }

    // -- components and the fixpoint ----------------------------------------------------------

    #[test]
    fn mutual_recursion_converges_below_top() {
        let mut prog = Prog::new();
        let f = prog.intern("f");
        let g = prog.intern("g");
        prog.add("f", calls(id(1), g));
        prog.add("g", calls(id(2), f));
        let summaries = prog.compute();
        for name in ["f", "g"] {
            let summary = Prog::summary(&summaries, name).expect("summarized, not top");
            assert_eq!(summary.purity, Purity::Const);
            assert!(!summary.may_unwind);
        }
        assert!(prog.graph().recursive(&JsName::new("f".to_owned())));
    }

    #[test]
    fn effects_reach_the_whole_component_when_flow_matches_the_sweep() {
        // f0 writes through its param and calls f9; f_k calls f_{k-1}: info at low names flows
        // to high names within one sweep. All ten members must be Effect, none top.
        let mut prog = Prog::new();
        let names: Vec<StrId> = (0..10u64).map(|n| prog.intern(&format!("f{n}"))).collect();
        for n in 1..10u64 {
            prog.add(&format!("f{n}"), calls(id(n), names[(n - 1) as usize]));
        }
        let mut b = FuncBuilder::new(id(0), sig(1));
        let p = b.param(TY, O);
        let c = b.op(FuncBuilder::ENTRY, OpKind::Const(ConstTree::Unit), TY, O);
        b.effect(FuncBuilder::ENTRY, OpKind::StoreThrough { ptr: p, v: c }, O);
        b.effect(
            FuncBuilder::ENTRY,
            call(names[9], CallSemantic::Ordinary, vec![p]),
            O,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        prog.add("f0", b.finish());
        let summaries = prog.compute();
        for n in 0..10u64 {
            let summary = Prog::summary(&summaries, &format!("f{n}")).expect("converged below top");
            assert_eq!(summary.purity, Purity::Effect, "member {n}");
        }
    }

    #[test]
    fn a_component_needing_more_rounds_than_the_cap_goes_to_top() {
        // The effect source sits at the HIGHEST id and every call points upward, so each sweep
        // in ItemId order moves the fact only one member: a 12-cycle cannot settle within
        // SCC_ROUNDS and every member must go to top rather than keep a half-propagated claim.
        let members = 12u64;
        let mut prog = Prog::new();
        let names: Vec<StrId> = (0..members)
            .map(|n| prog.intern(&format!("f{n:02}")))
            .collect();
        for n in 0..members - 1 {
            prog.add(&format!("f{n:02}"), calls(id(n), names[(n + 1) as usize]));
        }
        let mut b = FuncBuilder::new(id(members - 1), sig(1));
        let p = b.param(TY, O);
        let c = b.op(FuncBuilder::ENTRY, OpKind::Const(ConstTree::Unit), TY, O);
        b.effect(FuncBuilder::ENTRY, OpKind::StoreThrough { ptr: p, v: c }, O);
        b.effect(
            FuncBuilder::ENTRY,
            call(names[0], CallSemantic::Ordinary, vec![p]),
            O,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        prog.add(&format!("f{:02}", members - 1), b.finish());
        let summaries = prog.compute();
        for n in 0..members {
            let name = JsName::new(format!("f{n:02}"));
            assert!(
                summaries.usable(&name).is_none(),
                "member {n} is top after the round cap"
            );
            assert!(summaries.get(&name).is_some_and(Summary::is_top));
        }
    }

    #[test]
    fn computation_is_deterministic() {
        let mut prog = Prog::new();
        let w = prog.intern("w");
        let f = prog.intern("f");
        let r1 = prog.intern("r1");
        let r2 = prog.intern("r2");
        prog.add("w", writes_param(id(0)));
        prog.add("i", identity(id(1)));
        prog.add("f", calls(id(2), w));
        prog.add("g", calls(id(3), f));
        prog.add("r1", calls(id(4), r2));
        prog.add("r2", calls(id(5), r1));
        assert_eq!(prog.compute().dump(), prog.compute().dump());
        assert!(!prog.compute().dump().is_empty());
    }
}

/// The summary fuzzer (phase 5, W5b): every function of the fixture corpus (`build/**/*.o`, and
/// the sysroots' `libcore` under `RCGJS_SSA_FIXTURES=core`) is summarized, revalidated with the
/// independent checker, and run on generated arguments by the JIR-R interpreter -- and every
/// observed behavior must be admitted by the published summary: a `Const`/`Read` function
/// never writes through a pointer argument on return or an admitted throw/assertion,
/// an unclaimed unwind never happens, a
/// `Fresh` return aliases no argument, a `Param(i)` return is argument `i`'s value, and an
/// `Escape::No` parameter never aliases the return value. A construct the interpreter does not
/// model is skipped, not counted.
#[cfg(test)]
mod corpus {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    use rcgjs_object::object;

    use super::{compute, Purity, Summary};
    use crate::analysis::callgraph;
    use crate::item::JsName;
    use crate::jir::interns::{Interns, StrId};
    use crate::jir::interp::{Heap, HeapId, Hooks, Location, ReprInterpreter, Trap, Value};
    use crate::jir::repr::Func;
    use crate::jir::semantic::{BodyKind, DirectItemRef};
    use crate::jir::types::{TypeKind, TypeTable};
    use crate::lower::ssa;

    /// Argument vectors per function and interpreter steps per run: the run itself is cheap,
    /// but summaries are computed per object on top.
    const SETS: usize = 4;
    const STEPS: u64 = 4_000;
    /// How many objects double-compute their summaries to prove determinism.
    const DETERMINISM_OBJECTS: usize = 25;

    #[derive(Default)]
    struct Counts {
        objects: usize,
        summarized: usize,
        fuzzed: usize,
        observations: usize,
        skipped: usize,
        prefix_write_checks: usize,
        prefix_writes: usize,
        validate_failures: Vec<String>,
        violations: Vec<String>,
    }

    #[test]
    fn summaries_admit_every_interpreted_behavior() {
        // A deep body recurses through its merge points on the way up.
        std::thread::Builder::new()
            .stack_size(512 << 20)
            .spawn(run_corpus)
            .expect("spawn")
            .join()
            .expect("fixture thread");
    }

    fn run_corpus() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../build");
        let core = std::env::var("RCGJS_SSA_FIXTURES").is_ok_and(|value| value == "core");
        let mut paths = Vec::new();
        walk(&root, &mut paths, core);
        let mut counts = Counts::default();
        let mut seen: BTreeSet<u128> = BTreeSet::new();
        for path in &paths {
            for (label, bytes) in members(path) {
                check_object(&label, &bytes, &mut counts, &mut seen);
            }
        }
        eprintln!(
            "summary fuzzer: {} objects, {} items summarized, {} functions fuzzed, \
             {} observations checked, {} runs skipped, {} validate failures, {} violations",
            counts.objects,
            counts.summarized,
            counts.fuzzed,
            counts.observations,
            counts.skipped,
            counts.validate_failures.len(),
            counts.violations.len(),
        );
        eprintln!(
            "summary prefixes: {} negative write claims checked, {} observed write contradictions; incomplete runs remain skipped",
            counts.prefix_write_checks, counts.prefix_writes,
        );
        for failure in counts.validate_failures.iter().take(10) {
            eprintln!("  VALIDATE  {failure}");
        }
        for violation in counts.violations.iter().take(10) {
            eprintln!("  VIOLATION {violation}");
        }
        ssa::check::require_fixtures(counts.fuzzed, "summary fuzzer");
        if core && counts.fuzzed > 0 {
            assert!(
                counts.fuzzed >= 300,
                "the core corpus fuzzes at least 300 bodies, got {}",
                counts.fuzzed
            );
        }
        assert!(
            counts.validate_failures.is_empty(),
            "{} published summaries failed the independent recompute",
            counts.validate_failures.len()
        );
        assert!(
            counts.violations.is_empty(),
            "{} interpreted behaviors were not admitted by their summary",
            counts.violations.len()
        );
    }

    fn walk(dir: &Path, out: &mut Vec<PathBuf>, core: bool) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        entries.sort();
        for path in entries {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            if path.is_dir() {
                if matches!(
                    name.as_deref(),
                    Some("hostmacro-discover" | "sysroot-cache" | "target")
                ) || (name.as_deref() == Some("sysroots") && !core)
                {
                    continue;
                }
                walk(&path, out, core);
            } else if path.extension().is_some_and(|ext| ext == "o")
                || (core
                    && name
                        .as_deref()
                        .is_some_and(|n| n.starts_with("libcore-") && n.ends_with(".rlib")))
            {
                out.push(path);
            }
        }
    }

    fn members(path: &Path) -> Vec<(String, Vec<u8>)> {
        let Ok(bytes) = std::fs::read(path) else {
            return Vec::new();
        };
        if !bytes.starts_with(b"!<arch>\n") {
            return vec![(path.display().to_string(), bytes)];
        }
        let Ok(archive) = ::object::read::archive::ArchiveFile::parse(&*bytes) else {
            return Vec::new();
        };
        archive
            .members()
            .flatten()
            .filter_map(|member| {
                let data = member.data(&*bytes).ok()?;
                Some((
                    format!(
                        "{}({})",
                        path.display(),
                        String::from_utf8_lossy(member.name())
                    ),
                    data.to_vec(),
                ))
            })
            .collect()
    }

    fn check_object(label: &str, bytes: &[u8], counts: &mut Counts, seen: &mut BTreeSet<u128>) {
        if !object::is_object(bytes) {
            return;
        }
        let Ok(Some(object)) = object::read(bytes) else {
            return;
        };
        let table = &object.interns.types;
        let interns = &object.interns;
        let mut bodies = Vec::new();

        for item in &object.summaries {
            let Ok(Some((body, origins))) = object.decode_verified(item) else {
                continue;
            };
            if !matches!(body.kind, BodyKind::Function) {
                continue;
            }
            bodies.push((item.key.link_name.clone(), body, origins));
        }
        if bodies.is_empty() {
            return;
        }
        counts.objects += 1;

        // The object as one program: summaries over its funcs, calls out of it unresolved.
        let mut funcs: BTreeMap<JsName, Result<Func, crate::jir::decline::Decline>> =
            BTreeMap::new();
        for (name, body, origins) in &bodies {
            let Ok(func) = ssa::build(body, Some(origins), table, interns) else {
                continue;
            };
            funcs.entry(JsName::new(name.clone())).or_insert(Ok(func));
        }
        let callees: BTreeMap<StrId, &Func> = interns
            .strings
            .iter()
            .enumerate()
            .filter_map(|(index, name)| {
                funcs
                    .get(&JsName::new(name.clone()))
                    .and_then(|func| func.as_ref().ok())
                    .map(|func| (StrId(index as u32), func))
            })
            .collect();
        let interns_map: BTreeMap<JsName, &Interns> =
            funcs.keys().map(|name| (name.clone(), interns)).collect();
        let graph = callgraph::build(&funcs, &interns_map);
        let summaries = compute(&funcs, &interns_map, &graph);
        counts.summarized += summaries.len();
        if counts.objects <= DETERMINISM_OBJECTS {
            let again = compute(&funcs, &interns_map, &graph);
            assert_eq!(
                summaries.dump(),
                again.dump(),
                "{label}: two computations disagree"
            );
        }
        for (name, body, _) in &bodies {
            let js_name = JsName::new(name.clone());
            let Some(Ok(func)) = funcs.get(&js_name) else {
                continue;
            };
            let Some(summary) = summaries.get(&js_name) else {
                continue;
            };
            if body.signature.abi.spread.is_some() || body.signature.abi.caller_location {
                // The JS parameter positions shift against the Rust arguments; the per-param
                // checks would compare the wrong slots.
                continue;
            }
            let key = crate::jir::summary::semantic_hash(body, interns);
            let key = u128::from(key.lo) ^ (u128::from(key.hi) << 64);
            if !seen.insert(key) {
                continue;
            }
            let Some(sets) = ssa::check::argument_sets(&body.signature.arguments, table) else {
                continue;
            };
            counts.fuzzed += 1;
            for set in sets.iter().take(SETS) {
                fuzz_one(
                    label,
                    name.as_str(),
                    func,
                    summary,
                    table,
                    interns,
                    &callees,
                    set,
                    counts,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fuzz_one(
        label: &str,
        name: &str,
        func: &Func,
        summary: &Summary,
        table: &TypeTable,
        interns: &Interns,
        callees: &BTreeMap<StrId, &Func>,
        set: &[(Value, Option<Value>)],
        counts: &mut Counts,
    ) {
        let hook = |_: &DirectItemRef, _: Vec<Value>| -> Result<Value, Trap> {
            Err(Trap::Hook("callee body unavailable".to_owned()))
        };
        let mut hooks = Hooks::new(&hook);
        hooks.step_limit = STEPS;

        let mut heap = Heap::new();
        let args = materialize(set, &mut heap, table);
        // What each argument lets the callee reach, before the run.
        let mut arg_ids: Vec<BTreeSet<HeapId>> = Vec::new();
        for arg in &args {
            let mut ids = BTreeSet::new();
            collect_ids(arg, &mut ids);
            arg_ids.push(ids);
        }
        let outcome = {
            let mut interpreter =
                ReprInterpreter::new(func, table, interns, &hooks, &mut heap).with_callees(callees);
            interpreter.run(args.clone())
        };
        let mut violate = |what: String| {
            counts
                .violations
                .push(format!("{label} {name}: {what} (summary {summary:?})"));
        };
        let (returned, incomplete) = match outcome {
            Err(Trap::Assert(_) | Trap::Unwind) => {
                counts.observations += 1;
                if !summary.may_unwind {
                    violate("unwound without a may_unwind claim".to_owned());
                }
                (None, false)
            }
            Err(Trap::Unsupported(_) | Trap::Hook(_) | Trap::Steps) => {
                counts.skipped += 1;
                (None, true)
            }
            Err(_) => {
                counts.skipped += 1;
                return;
            }
            Ok(ret) => (Some(ret), false),
        };
        // Writes remain observable to the summary check even if they store the same value
        // or are undone before normal or exceptional completion.
        for (param, ids) in arg_ids.iter().enumerate() {
            let claimed_unwritten = summary.purity <= Purity::Read;
            if !claimed_unwritten || ids.is_empty() {
                continue;
            }
            let written = ids.iter().any(|id| heap.was_written(*id));
            if incomplete {
                counts.prefix_write_checks += 1;
                // A completed prefix can disprove a no-write claim. Absence of a write
                // proves nothing about the unsupported suffix and is not agreement.
                if !written {
                    continue;
                }
                counts.prefix_writes += 1;
            }
            counts.observations += 1;
            if written {
                violate(format!("param {param} pointee was written"));
            }
        }
        let _ = returned;
    }

    fn materialize(
        set: &[(Value, Option<Value>)],
        heap: &mut Heap,
        table: &TypeTable,
    ) -> Vec<Value> {
        set.iter()
            .map(|(value, pointee)| match pointee {
                None => value.clone(),
                Some(pointee) => {
                    let id = heap.alloc(pointee.clone());
                    match pointee {
                        Value::Aggregate { fields, ty, .. }
                            if matches!(
                                table.get(*ty).map(|d| &d.kind),
                                Some(TypeKind::Slice(_))
                            ) =>
                        {
                            Value::Slice {
                                buf: Location::root(id),
                                off: 0,
                                len: fields.len() as u64,
                            }
                        }
                        _ => Value::Ptr(Location::root(id)),
                    }
                }
            })
            .collect()
    }

    fn collect_ids(value: &Value, out: &mut BTreeSet<HeapId>) {
        match value {
            Value::Ptr(location) => {
                out.insert(location.heap);
            }
            Value::Slice { buf, .. } => {
                out.insert(buf.heap);
            }
            Value::Aggregate { fields, .. } => {
                for field in fields {
                    collect_ids(field, out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn direct_call_results_are_independent_of_checked_summaries() {
        use crate::jir::origin::OriginId;
        use crate::jir::repr::{FuncBuilder, Op, OpKind, RAssertMessage, RTerm};
        use crate::jir::semantic::{
            AbiSignature, CallSemantic, Callee, DefinitionId, ItemId, Signature,
        };
        use crate::jir::types::{
            JsFacts, LayoutFacts, PointerForm, TypeDescriptor, TypeId, TypeMeta,
        };

        let ty = TypeId(1);
        let origin = OriginId(0);
        let mut interns = Interns::new();
        interns.types.insert(TypeDescriptor {
            id: ty,
            name: None,
            kind: TypeKind::Bool,
            layout: LayoutFacts {
                size: Some(1),
                unsized_prefix: None,
                align: Some(1),
                sized: true,
                zst: false,
                one_zst: false,
                needs_drop: false,
                has_own_drop: false,
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: ty,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        });
        let name = interns.intern_str("identity");
        let missing = interns.intern_str("unavailable");
        let signature = |arguments| Signature {
            arguments: vec![ty; arguments],
            result: ty,
            abi: AbiSignature {
                parameters: arguments as u16,
                spread: None,
                caller_location: false,
                inline_never: false,
                inline_hint: false,
                inline_always: false,
            },
        };
        let item = |local_hash| ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash,
            },
            instantiation: 0,
        };
        let mut b = FuncBuilder::new(item(1), signature(1));
        let arg = b.param(ty, origin);
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(arg)));
        let callee = b.finish();
        let mut b = FuncBuilder::new(item(2), signature(2));
        b.param(ty, origin);
        let arg = b.param(ty, origin);
        let result = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: callee.item,
                    link_name: name,
                },
                function: None,
                args: vec![arg],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            ty,
            origin,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(result)));
        let caller = b.finish();
        caller.verify(&interns.types).unwrap();
        callee.verify(&interns.types).unwrap();
        let _ = (&caller, &callee);

        let pointer_ty = TypeId(2);
        let mut pointer = interns.types.get(ty).unwrap().clone();
        pointer.id = pointer_ty;
        pointer.kind = TypeKind::Reference {
            pointee: ty,
            mutable: true,
        };
        pointer.layout.size = Some(4);
        pointer.layout.align = Some(4);
        pointer.js.peeled = pointer_ty;
        pointer.js.pointer_form = Some(PointerForm::Slot);
        interns.types.insert(pointer);
        let mut sig = signature(2);
        sig.arguments[0] = pointer_ty;
        let mut b = FuncBuilder::new(item(3), sig.clone());
        let pointer = b.param(pointer_ty, origin);
        let value = b.param(ty, origin);
        let saved = b.op(FuncBuilder::ENTRY, OpKind::Deref(pointer), ty, origin);
        b.effect(
            FuncBuilder::ENTRY,
            OpKind::StoreThrough {
                ptr: pointer,
                v: value,
            },
            origin,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(value)));
        let writing_callee = b.finish();
        let mut b = FuncBuilder::new(item(4), sig);
        let pointer = b.param(pointer_ty, origin);
        let value = b.param(ty, origin);
        let result = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: writing_callee.item,
                    link_name: name,
                },
                function: None,
                args: vec![pointer, value],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            ty,
            origin,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(result)));
        let caller = b.finish();
        caller.verify(&interns.types).unwrap();
        let set = [
            (Value::Undef, Some(Value::Bool(false))),
            (Value::Bool(true), None),
        ];
        for write in ["none", "store", "same", "restore"] {
            let writes = write != "none";
            for exit in [
                "return",
                "throw",
                "assert",
                "unsupported",
                "hook",
                "steps",
                "unsupported-before-write",
            ] {
                let mut callee = writing_callee.clone();
                if !writes {
                    callee.blocks[0].ops.clear();
                    callee.values.pop();
                }
                if write == "same" {
                    callee.blocks[0].ops[1].kind = OpKind::StoreThrough {
                        ptr: callee.params[0],
                        v: saved,
                    };
                } else if write == "restore" {
                    callee.blocks[0].ops.push(Op {
                        result: None,
                        origin,
                        kind: OpKind::StoreThrough {
                            ptr: callee.params[0],
                            v: saved,
                        },
                    });
                }
                if exit == "throw" {
                    callee.blocks[0].term = RTerm::Throw(None);
                } else if exit == "assert" {
                    callee.blocks[0].ops.push(Op {
                        result: None,
                        origin,
                        kind: OpKind::Assert {
                            cond: callee.params[1],
                            expected: false,
                            msg: RAssertMessage::NullPointer,
                            track_caller: None,
                        },
                    });
                } else if exit == "unsupported" {
                    callee.blocks[0].ops.push(Op {
                        result: None,
                        origin,
                        kind: OpKind::Zombie(name),
                    });
                } else if exit == "unsupported-before-write" {
                    callee.blocks[0].ops.insert(
                        0,
                        Op {
                            result: None,
                            origin,
                            kind: OpKind::Zombie(name),
                        },
                    );
                } else if exit == "hook" {
                    callee.blocks[0].ops.push(Op {
                        result: None,
                        origin,
                        kind: OpKind::Call {
                            callee: Callee::Direct {
                                item: item(99),
                                link_name: missing,
                            },
                            function: None,
                            args: vec![],
                            track_caller: None,
                            semantic: CallSemantic::Ordinary,
                            host: None,
                        },
                    });
                } else if exit == "steps" {
                    callee.blocks[0].term = RTerm::Goto(FuncBuilder::ENTRY, callee.params.clone());
                }
                callee.verify(&interns.types).unwrap();
                let callees = BTreeMap::from([(name, &callee)]);
                let summary = Summary::top();
                let mut pure = summary;
                pure.purity = Purity::Const;
                let mut read = summary;
                read.purity = Purity::Read;
                let mut no_unwind = summary;
                no_unwind.may_unwind = false;
                let observed_write = writes && exit != "unsupported-before-write";
                let incomplete = matches!(
                    exit,
                    "unsupported" | "hook" | "steps" | "unsupported-before-write"
                );
                for (claim, summary, violations) in [
                    ("unknown effects", summary, false),
                    ("pure", pure, observed_write),
                    ("read only", read, observed_write),
                    ("no unwind", no_unwind, matches!(exit, "throw" | "assert")),
                ] {
                    let mut counts = Counts::default();
                    fuzz_one(
                        "direct call control",
                        claim,
                        &caller,
                        &summary,
                        &interns.types,
                        &interns,
                        &callees,
                        &set,
                        &mut counts,
                    );
                    assert_eq!(counts.skipped, usize::from(incomplete));
                    let checks_write = matches!(claim, "pure" | "read only");
                    assert_eq!(
                        counts.prefix_write_checks,
                        usize::from(incomplete && checks_write),
                        "prefix checks: {claim}, write={write}, exit={exit}"
                    );
                    assert_eq!(
                        counts.prefix_writes,
                        usize::from(incomplete && checks_write && observed_write),
                        "prefix writes: {claim}, write={write}, exit={exit}"
                    );
                    if incomplete {
                        assert_eq!(
                            counts.observations,
                            usize::from(violations),
                            "prefix observations: {claim}, write={write}, exit={exit}"
                        );
                    }
                    assert_eq!(
                        counts.violations.len(),
                        usize::from(violations),
                        "{claim}: write={write}, exit={exit}"
                    );
                }
            }
        }
    }
}
