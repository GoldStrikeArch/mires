//! Coinductive no-op heap drop-glue pruning.
//!
//! Empties provably no-op `DropGlue(String)`, `DropGlue(Vec<T, Global>)`,
//! `DropGlue(RawVec<T, Global>)`, and coinductively pure composite/recursive `DropGlue` JIR `Func`
//! bodies (plus rewrites `OpKind::Drop` sites targeting them across all `funcs` to
//! `DropGlue::None`) right before post-repr `o_group_with_mode`, so `prune_invoke`, `witness`,
//! `inline`, `partial_escape`, and `dce` eliminate dead `try/catch` unwind landing pads and AST
//! teardown automatically.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use rcgjs_object::jir::repr::{OpKind, RBlock, RBlockId, RTerm};
use rcgjs_object::jir::semantic::{
    BinaryOp, BodySemantic, CallSemantic, Callee, DropGlue, StandardOperation,
};
use rcgjs_object::jir::types::TypeKind;

use crate::analysis::heap::HeapPlan;
use crate::item::{JsName, LinkItem};
use crate::jir::decline::Decline;
use crate::jir::repr::Func;
use crate::jir::Interns;

/// Runs no-op heap drop-glue pruning over `funcs` in place.
pub fn run(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    heap: &HeapPlan,
) {
    let pruner = DropPruner::new(table, heap);
    if !pruner.enabled() {
        return;
    }
    let direct_noop = pruner.clear_direct_noops(funcs);
    let noop_funcs = pruner.solve_coinductive_noops(funcs, direct_noop);
    if !noop_funcs.is_empty() {
        pruner.rewrite_noop_drop_sites(funcs, &noop_funcs);
    }
}

/// A direct link-name target encountered while checking an op for drop-glue purity.
enum DirectTarget<'a> {
    Drop(&'a str),
    Call(&'a str),
}

/// Encapsulates heap-plan flags and item metadata for coinductive no-op drop-glue elimination.
struct DropPruner<'a> {
    table: &'a BTreeMap<JsName, LinkItem>,
    nstr: bool,
    nvec: bool,
    typed_dealloc_noop: bool,
    typed_dealloc_raw_noop: bool,
}

impl<'a> DropPruner<'a> {
    fn new(table: &'a BTreeMap<JsName, LinkItem>, heap: &HeapPlan) -> Self {
        Self {
            table,
            nstr: heap.native_string_enabled,
            nvec: heap.native_vec_enabled && !heap.non_rc_raw_alloc_reachable,
            typed_dealloc_noop: heap.typed_heap_enabled && !heap.non_rc_raw_alloc_reachable,
            typed_dealloc_raw_noop: heap.typed_heap_enabled && !heap.raw_alloc_reachable,
        }
    }

    fn enabled(&self) -> bool {
        self.nstr || self.nvec || self.typed_dealloc_noop
    }

    fn is_pure_std_op(&self, op: StandardOperation) -> bool {
        match op {
            StandardOperation::TypedDealloc => self.typed_dealloc_noop,
            StandardOperation::TypedDeallocRaw => self.typed_dealloc_raw_noop,
            StandardOperation::NativeVecAsPtr
            | StandardOperation::NativeVecAsMutPtr
            | StandardOperation::NativeVecAsSlice
            | StandardOperation::NativeVecAsMutSlice
            | StandardOperation::NativeVecCapacity
            | StandardOperation::VecAsSlice
            | StandardOperation::VecLen
            | StandardOperation::StringAsStr
            | StandardOperation::NativeStrAsStr
            | StandardOperation::NativeStrLen => true,
            _ => false,
        }
    }

    fn is_stdlib_container_drop_impl(&self, item: &LinkItem) -> bool {
        if !self.nvec || item.linkage.exported || item.kind != rcgjs_object::object::ItemKind::Fn {
            return false;
        }
        let Some(body) = item.jir.body.as_ref() else {
            return false;
        };
        if !matches!(body.semantic, BodySemantic::Ordinary) {
            return false;
        }
        let Some(interns) = item.jir.interns() else {
            return false;
        };
        if !interns.types.is_zst(body.signature.result) {
            return false;
        }
        let dp = item.debug_path.as_str();
        let name = item.name.as_str();
        let is_drop_method = dp.ends_with("::drop") || name.contains("_drop$h");
        let is_known_container = dp.contains("alloc::vec::")
            || dp.contains("alloc::raw_vec::")
            || name.starts_with("alloc$vec$")
            || name.starts_with("alloc$raw_vec$");
        is_drop_method && is_known_container
    }

    /// Pass 1: empty directly known no-op String / POD-Vec `DropGlue`s and strip
    /// rustc-elaborated `.buf` / `String` `OpKind::Drop`s across all `funcs`.
    fn clear_direct_noops(
        &self,
        funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    ) -> BTreeSet<JsName> {
        let mut direct_noop: BTreeSet<JsName> = BTreeSet::new();
        for (name, res) in funcs.iter_mut() {
            let Ok(func) = res else { continue };
            let item = self.table.get(name);
            let is_noop_str = self.nstr && crate::analysis::heap::is_string_drop_glue(item);
            let is_noop_vec = self.nvec
                && crate::analysis::heap::is_noop_global_vec_drop_glue(item, self.nstr, self.nvec);
            if is_noop_str || is_noop_vec {
                func.blocks = empty_blocks(func);
                direct_noop.insert(name.clone());
                continue;
            }
            let Some(interns) = item.and_then(|i| i.jir.interns()) else {
                continue;
            };
            let mut to_clear: Vec<(usize, usize)> = Vec::new();
            for (bi, block) in func.blocks.iter().enumerate() {
                for (oi, op) in block.ops.iter().enumerate() {
                    if let OpKind::Drop {
                        root,
                        path,
                        glue: DropGlue::Direct { .. },
                    } = &op.kind
                    {
                        let noop = (self.nstr
                            && crate::analysis::heap::drop_targets_native_string(
                                func,
                                &interns.types,
                                root,
                                path,
                            ))
                            || (self.nvec
                                && crate::analysis::heap::drop_targets_noop_native_vec(
                                    func,
                                    &interns.types,
                                    interns,
                                    root,
                                    path,
                                    self.nstr,
                                    self.nvec,
                                ));
                        if noop {
                            to_clear.push((bi, oi));
                        }
                    }
                }
            }
            for (bi, oi) in to_clear {
                if let OpKind::Drop { glue, .. } = &mut func.blocks[bi].ops[oi].kind {
                    *glue = DropGlue::None;
                }
            }
            if func.blocks.len() == 1
                && func.blocks[0].ops.is_empty()
                && matches!(func.blocks[0].term, RTerm::Return(None))
            {
                direct_noop.insert(name.clone());
            }
        }
        direct_noop
    }

    /// Pass 2: coinductive greatest-fixpoint over all `DropGlue`s (and stdlib `Vec`/`IntoIter`/
    /// `DropGuard`/`RawVec` `Drop::drop` impls under `nvec`) in `funcs`.
    ///
    /// Every candidate starts optimistically no-op unless it contains an impure op, an
    /// unknown/impure call or invoke, a non-standard CFG loop (guards against inlined user
    /// `fn drop() { loop {} }`), a dynamic drop, or transitively calls/drops a live candidate.
    fn solve_coinductive_noops(
        &self,
        funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
        direct_noop: BTreeSet<JsName>,
    ) -> BTreeSet<JsName> {
        let cand_dg = self.candidate_drop_glues(funcs);
        let mut live: BTreeSet<JsName> = BTreeSet::new();
        let mut rev_deps: BTreeMap<JsName, Vec<JsName>> = BTreeMap::new();

        for name in &cand_dg {
            if direct_noop.contains(name) {
                continue;
            }
            let Ok(func) = &funcs[name] else { continue };
            match self.analyze_candidate(name, func, funcs, &direct_noop, &cand_dg) {
                None => {
                    live.insert(name.clone());
                }
                Some(deps) => {
                    for dep in deps {
                        rev_deps.entry(dep).or_default().push(name.clone());
                    }
                }
            }
        }

        let mut queue: VecDeque<JsName> = live.iter().cloned().collect();
        while let Some(cur) = queue.pop_front() {
            if let Some(callers) = rev_deps.get(&cur) {
                for caller in callers {
                    if live.insert(caller.clone()) {
                        queue.push_back(caller.clone());
                    }
                }
            }
        }

        let mut noop_funcs = direct_noop;
        for name in &cand_dg {
            if !live.contains(name) {
                if let Some(Ok(func)) = funcs.get_mut(name) {
                    func.blocks = empty_blocks(func);
                }
                noop_funcs.insert(name.clone());
            }
        }
        noop_funcs
    }

    fn candidate_drop_glues(
        &self,
        funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    ) -> BTreeSet<JsName> {
        funcs
            .iter()
            .filter_map(|(name, res)| {
                if res.is_err() {
                    return None;
                }
                let item = self.table.get(name)?;
                let is_dg = matches!(
                    item.jir.body.as_ref().map(|b| b.semantic),
                    Some(BodySemantic::DropGlue(_))
                );
                if !is_dg && !self.is_stdlib_container_drop_impl(item) {
                    return None;
                }
                item.jir.interns()?;
                Some(name.clone())
            })
            .collect()
    }

    /// Checks one drop-glue candidate for local purity; returns `Some(deps)` on candidate
    /// drop-glues it depends on, or `None` if it is unconditionally live.
    fn analyze_candidate(
        &self,
        name: &JsName,
        func: &Func,
        funcs: &BTreeMap<JsName, Result<Func, Decline>>,
        direct_noop: &BTreeSet<JsName>,
        cand_dg: &BTreeSet<JsName>,
    ) -> Option<Vec<JsName>> {
        let item = &self.table[name];
        let interns = item.jir.interns()?;
        let drop_ty_opt = match item.jir.body.as_ref().map(|b| b.semantic) {
            Some(BodySemantic::DropGlue(drop_ty)) => Some(drop_ty),
            Some(BodySemantic::Ordinary) if self.is_stdlib_container_drop_impl(item) => None,
            _ => return None,
        };

        // Only allow CFG loops inside DropGlues / Drop::drop impls of known bounded standard
        // containers (`Vec<T, Global>`, `[T]`, `[T; N]`, `vec::into_iter::{IntoIter, DropGuard}`,
        // `RawVec`).
        if has_cfg_cycle(func) {
            let known_bounded = match drop_ty_opt {
                Some(drop_ty) => {
                    let peeled = interns.types.peeled(drop_ty);
                    crate::analysis::heap::is_direct_global_vec_ty(&interns.types, drop_ty)
                        || matches!(
                            interns.types.get(peeled).map(|d| &d.kind),
                            Some(TypeKind::Slice(_) | TypeKind::Array { .. })
                        )
                        || item.debug_path.contains("vec::into_iter::IntoIter")
                        || item.debug_path.contains("vec::into_iter::DropGuard")
                }
                None => true,
            };
            if !known_bounded {
                return None;
            }
        }

        let mut deps: Vec<JsName> = Vec::new();
        for b in &func.blocks {
            for op in &b.ops {
                let ok = self.is_pure_drop_op(&op.kind, interns, |target| match target {
                    DirectTarget::Drop(s) => {
                        if direct_noop.contains(s) {
                            true
                        } else if let Some(dep) = cand_dg.get(s) {
                            deps.push(dep.clone());
                            true
                        } else {
                            false
                        }
                    }
                    DirectTarget::Call(s) => {
                        if direct_noop.contains(s) {
                            true
                        } else if let Some(dep) = cand_dg.get(s) {
                            deps.push(dep.clone());
                            true
                        } else {
                            let mut visiting = BTreeSet::new();
                            self.is_pure_acyclic_helper(s, funcs, 6, &mut visiting)
                        }
                    }
                });
                if !ok {
                    return None;
                }
            }
            let term_ok = matches!(
                &b.term,
                RTerm::Return(None)
                    | RTerm::Goto(_, _)
                    | RTerm::Switch { .. }
                    | RTerm::Invoke { .. }
                    | RTerm::Unreachable
                    | RTerm::Throw(None)
            );
            if !term_ok {
                return None;
            }
        }
        Some(deps)
    }

    /// Checks whether a non-DropGlue direct callee in `funcs` is a small pure acyclic
    /// accessor/dealloc helper (e.g. `Vec::as_mut_ptr`, `RawVecInner::current_memory`).
    fn is_pure_acyclic_helper(
        &self,
        target: &'a str,
        funcs: &BTreeMap<JsName, Result<Func, Decline>>,
        depth: u8,
        visiting: &mut BTreeSet<&'a str>,
    ) -> bool {
        if depth == 0 || !visiting.insert(target) {
            return false;
        }
        let result = (|| {
            let Some(item) = self.table.get(target) else {
                return false;
            };
            if let Some(BodySemantic::Standard(op)) = item.jir.body.as_ref().map(|b| b.semantic) {
                if self.is_pure_std_op(op) {
                    return true;
                }
            }
            let Some(Ok(func)) = funcs.get(target) else {
                return false;
            };
            let Some(interns) = item.jir.interns() else {
                return false;
            };
            if func.blocks.len() > 12 || has_cfg_cycle(func) {
                return false;
            }
            for block in &func.blocks {
                for op in &block.ops {
                    let ok = self.is_pure_drop_op(&op.kind, interns, |direct| match direct {
                        DirectTarget::Drop(_) => false,
                        DirectTarget::Call(s) => {
                            self.is_pure_acyclic_helper(s, funcs, depth - 1, visiting)
                        }
                    });
                    if !ok {
                        return false;
                    }
                }
                let term_ok = matches!(
                    &block.term,
                    RTerm::Return(_)
                        | RTerm::Goto(_, _)
                        | RTerm::Switch { .. }
                        | RTerm::Invoke { .. }
                        | RTerm::Unreachable
                        | RTerm::Throw(None)
                );
                if !term_ok {
                    return false;
                }
            }
            true
        })();
        visiting.remove(target);
        result
    }

    /// Shared op-purity check for both drop-glue candidates and their pure acyclic helpers.
    fn is_pure_drop_op(
        &self,
        kind: &OpKind,
        interns: &'a Interns,
        mut on_direct: impl FnMut(DirectTarget<'a>) -> bool,
    ) -> bool {
        match kind {
            OpKind::Copy(_)
            | OpKind::Move(_)
            | OpKind::Const(_)
            | OpKind::Discriminant(_)
            | OpKind::Field { .. }
            | OpKind::BorrowView { .. }
            | OpKind::RawView { .. }
            | OpKind::SliceView { .. }
            | OpKind::Deref(_)
            | OpKind::CellAlloc(_)
            | OpKind::CellLoad(_)
            | OpKind::CellStore { .. }
            | OpKind::Aggregate { .. }
            | OpKind::Repeat { .. }
            | OpKind::Cast { .. }
            | OpKind::Convert { .. }
            | OpKind::Unary { .. } => true,
            OpKind::Binary { op, .. } => !matches!(op, BinaryOp::Div | BinaryOp::Rem),
            OpKind::Intrinsic { name, .. } => interns.str(*name).is_some_and(is_pure_intrinsic),
            OpKind::Drop { glue, .. } => match glue {
                DropGlue::None => true,
                DropGlue::Direct { link_name, .. } => interns
                    .str(*link_name)
                    .is_some_and(|s| on_direct(DirectTarget::Drop(s))),
                DropGlue::Dynamic => false,
            },
            OpKind::Call {
                callee,
                semantic,
                host,
                ..
            } => {
                if host.is_some() {
                    return false;
                }
                if let CallSemantic::Standard(op) = *semantic {
                    if self.is_pure_std_op(op) {
                        return true;
                    }
                }
                match callee {
                    Callee::Nop => true,
                    Callee::Runtime {
                        member,
                        sysroot: true,
                    } => interns.str(*member).is_some_and(is_pure_runtime_member),
                    Callee::Direct { link_name, .. } => interns
                        .str(*link_name)
                        .is_some_and(|s| on_direct(DirectTarget::Call(s))),
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Pass 3: rewrite all `OpKind::Drop { glue: DropGlue::Direct(dg) }` targeting any proven
    /// no-op `DropGlue` across ALL functions to `DropGlue::None`, unblocking `prune_invoke`,
    /// `partial_escape::unbox_cells`, and `dce`.
    fn rewrite_noop_drop_sites(
        &self,
        funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
        noop_funcs: &BTreeSet<JsName>,
    ) {
        for (name, res) in funcs.iter_mut() {
            let Ok(func) = res else { continue };
            let Some(interns) = self.table.get(name).and_then(|i| i.jir.interns()) else {
                continue;
            };
            for block in &mut func.blocks {
                for op in &mut block.ops {
                    if let OpKind::Drop { glue, .. } = &mut op.kind {
                        let clear = match glue {
                            DropGlue::Direct { link_name, .. } => interns
                                .str(*link_name)
                                .is_some_and(|s| noop_funcs.contains(s)),
                            _ => false,
                        };
                        if clear {
                            *glue = DropGlue::None;
                        }
                    }
                }
            }
        }
    }
}

fn empty_blocks(func: &Func) -> Vec<RBlock> {
    vec![RBlock {
        id: RBlockId(0),
        params: func
            .blocks
            .first()
            .map(|b| b.params.clone())
            .unwrap_or_default(),
        ops: Vec::new(),
        term: RTerm::Return(None),
    }]
}

fn has_cfg_cycle(func: &Func) -> bool {
    let n = func.blocks.len();
    if n == 0 {
        return false;
    }
    let mut color = vec![0u8; n];
    let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
    color[0] = 1;
    while let Some((b, edge_idx)) = stack.last_mut() {
        let bi = *b;
        let succs: Vec<usize> = func.blocks[bi]
            .term
            .successors()
            .into_iter()
            .map(|id| id.0 as usize)
            .filter(|&s| s < n)
            .collect();
        if *edge_idx < succs.len() {
            let next = succs[*edge_idx];
            *edge_idx += 1;
            if color[next] == 1 {
                return true;
            }
            if color[next] == 0 {
                color[next] = 1;
                stack.push((next, 0));
            }
        } else {
            color[bi] = 2;
            stack.pop();
        }
    }
    false
}

fn is_pure_runtime_member(m: &str) -> bool {
    matches!(
        m,
        "sfp"
            | "slice_from_parts"
            | "offset"
            | "offset_from"
            | "thin"
            | "addr"
            | "unwindow"
            | "unwindow_slice"
            | "prefix"
            | "unproject"
            | "erase"
            | "unref"
    )
}

fn is_pure_intrinsic(m: &str) -> bool {
    matches!(
        m,
        "wrapping_add"
            | "wrapping_sub"
            | "wrapping_mul"
            | "unchecked_add"
            | "unchecked_sub"
            | "unchecked_mul"
            | "arith_offset"
            | "offset"
            | "ptr_offset_from"
            | "ptr_offset_from_unsigned"
            | "size_of_val"
            | "min_align_of_val"
    )
}
