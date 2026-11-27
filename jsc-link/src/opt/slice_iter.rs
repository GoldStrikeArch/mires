//! Lowers scalarized raw-pointer slice loops (`while ptr != end { ... ptr = ptr.add(1) }`) to
//! checked integer cursors.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::jir::repr::{
    Carrier, Func, Op, OpKind, Proj, RAssertMessage, RBlock, RBlockId, RTerm, Root, ValueId,
};
use crate::jir::semantic::{BinaryOp, CallSemantic, Callee, CastKind, ConstTree, TypeId, UnaryOp};
use crate::jir::types::{IntRepr, IteratorIdentity, NominalForm, TypeKind};
use crate::jir::{OriginId, TypeTable};

use super::cfg::{Cfg, DefUse};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SliceIteratorLoop {
    #[cfg(test)]
    pub(crate) adapter: IteratorIdentity,
    #[cfg(test)]
    pub(crate) header: RBlockId,
    #[cfg(test)]
    pub(crate) direct: RBlockId,
    #[cfg(test)]
    pub(crate) body: RBlockId,
    #[cfg(test)]
    pub(crate) cursor: ValueId,
    #[cfg(test)]
    pub(crate) body_cursor: ValueId,
    #[cfg(test)]
    pub(crate) enumerate_index: ValueId,
    #[cfg(test)]
    pub(crate) byte: Option<ValueId>,
    #[cfg(test)]
    pub(crate) byte_ty: TypeId,
    #[cfg(test)]
    pub(crate) cursor_ty: TypeId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Adapter {
    Plain,
    Enumerate,
}

#[derive(Clone, Copy, Debug)]
struct Site {
    block: RBlockId,
    op: usize,
}

#[derive(Clone, Debug)]
struct LoopMatch {
    adapter: Adapter,
    header: RBlockId,
    preheader: RBlockId,
    backedges: Vec<RBlockId>,
    driver: RBlockId,
    body: RBlockId,
    removed_ops: BTreeMap<RBlockId, BTreeSet<usize>>,
    removed_header_params: Vec<usize>,
    removed_invariant_params: BTreeMap<RBlockId, Vec<usize>>,
    rewrite_blocks: BTreeSet<RBlockId>,
    pass_item_ref: bool,
    exit: RBlockId,
    source: ValueId,
    len: ValueId,
    old_index: Option<ValueId>,
    old_item_ref: Option<ValueId>,
    old_byte: Option<ValueId>,
    item_ref_ty: TypeId,
    byte_ty: TypeId,
    cursor_ty: TypeId,
    cursor_bytes: u8,
    usize_ty: TypeId,
    bool_ty: TypeId,
    origin: OriginId,
}

/// Shared read-only analysis indices for matching scalarized raw-pointer slice loops.
struct MatchCx<'a> {
    func: &'a Func,
    table: &'a TypeTable,
    def_use: &'a DefUse,
    defs: &'a [Option<Site>],
    single_edge: &'a [Option<ValueId>],
    cfg: &'a Cfg,
}

struct HeaderShape {
    header: RBlockId,
    bool_ty: TypeId,
    cmp_ty: TypeId,
    cmp_l: ValueId,
    cmp_r: ValueId,
    cmp_origin: OriginId,
    continue_to: RBlockId,
    exit_target: RBlockId,
}

struct LoopRegion {
    header: RBlockId,
    preheader: RBlockId,
    backedges: Vec<RBlockId>,
    scc: BTreeSet<RBlockId>,
}

struct CursorBounds {
    p_idx: usize,
    ptr_param: ValueId,
    source: ValueId,
    len: ValueId,
    end_param_idx: Option<usize>,
    elem_ty: TypeId,
    usize_ty: TypeId,
}

struct DriverMatch {
    driver: RBlockId,
    removed_driver_ops: BTreeSet<usize>,
    removed_invariant_params: BTreeMap<RBlockId, Vec<usize>>,
    old_item_ref: Option<ValueId>,
    old_item: Option<ValueId>,
    item_ref_ty: TypeId,
    origin: OriginId,
}

struct EnumerateMatch {
    adapter: Adapter,
    removed_driver_ops: BTreeSet<usize>,
    removed_header_params: Vec<usize>,
    old_index: Option<ValueId>,
}

struct LocalityCheck<'a> {
    region: &'a LoopRegion,
    removed_ops: &'a BTreeMap<RBlockId, BTreeSet<usize>>,
    removed_invariant_params: &'a BTreeMap<RBlockId, Vec<usize>>,
    bypassed_driver: Option<RBlockId>,
    removed_params: &'a [usize],
    exposed: &'a [Option<ValueId>],
    extra_backedge_roots: &'a [ValueId],
}

/// Rewrites every scalarized raw-pointer slice loop in `func` to an integer cursor loop.
pub fn run(func: &mut Func, table: &TypeTable) -> Vec<SliceIteratorLoop> {
    let def_use = DefUse::of(func);
    let defs = definitions(&def_use);
    let cfg = Cfg::of(func);
    let single_edge = single_edge_param_sources(func, &cfg, &def_use);
    let cx = MatchCx {
        func,
        table,
        def_use: &def_use,
        defs: &defs,
        single_edge: &single_edge,
        cfg: &cfg,
    };

    let mut matches = Vec::new();
    let mut occupied: BTreeSet<RBlockId> = BTreeSet::new();
    for header_idx in 1..func.blocks.len() {
        let header = RBlockId(header_idx as u32);
        if occupied.contains(&header) {
            continue;
        }
        let Some(found) = cx.match_scalarized_loop(header) else {
            continue;
        };
        if !reserve_region(&found.rewrite_blocks, found.preheader, &mut occupied) {
            continue;
        }
        matches.push(found);
    }

    matches
        .into_iter()
        .map(|found| apply(func, found))
        .collect()
}

fn reserve_region(
    rewrite_blocks: &BTreeSet<RBlockId>,
    preheader: RBlockId,
    occupied: &mut BTreeSet<RBlockId>,
) -> bool {
    let region = rewrite_blocks
        .iter()
        .copied()
        .chain(std::iter::once(preheader));
    if region.clone().any(|block| occupied.contains(&block)) {
        return false;
    }
    occupied.extend(region);
    true
}

fn is_sized_non_zst(table: &TypeTable, ty: TypeId) -> bool {
    table
        .get(table.peeled(ty))
        .is_some_and(|d| d.layout.sized && !d.layout.zst)
}

fn signed_i32_type(table: &TypeTable) -> Option<TypeId> {
    table
        .descriptors()
        .iter()
        .find(|d| {
            table.int_info(d.id) == Some((true, 32)) && table.int_repr(d.id) == IntRepr::Number
        })
        .map(|d| d.id)
}

fn cursor_type_for_slice(table: &TypeTable, usize_ty: TypeId, usize_bytes: u8) -> (TypeId, u8) {
    match signed_i32_type(table) {
        Some(i32_ty) => (i32_ty, 4),
        None => (usize_ty, usize_bytes),
    }
}

fn ref_to_type(table: &TypeTable, pointee: TypeId) -> Option<TypeId> {
    table
        .descriptors()
        .iter()
        .find(|d| {
            matches!(
                d.kind,
                TypeKind::Reference {
                    pointee: found,
                    mutable: false,
                } if found == pointee
            )
        })
        .map(|d| d.id)
}

fn bool_type(table: &TypeTable) -> Option<TypeId> {
    table
        .descriptors()
        .iter()
        .find(|descriptor| matches!(descriptor.kind, TypeKind::Bool))
        .map(|descriptor| descriptor.id)
}

fn sized_non_zst_slice_element(table: &TypeTable, ty: TypeId) -> Option<TypeId> {
    let pointee = match table.get(ty).map(|descriptor| &descriptor.kind) {
        Some(TypeKind::Reference {
            pointee,
            mutable: false,
        })
        | Some(TypeKind::RawPointer {
            pointee,
            mutable: false,
        }) => *pointee,
        _ => return None,
    };
    let TypeKind::Slice(element) = table.get(pointee)?.kind else {
        return None;
    };
    is_sized_non_zst(table, element).then_some(element)
}

impl<'a> MatchCx<'a> {
    fn block(&self, id: RBlockId) -> Option<&'a RBlock> {
        self.func.blocks.get(id.0 as usize)
    }

    fn def_site(&self, value: ValueId) -> Option<Site> {
        self.defs.get(value.0 as usize).copied().flatten()
    }

    fn definition(&self, value: ValueId) -> Option<&'a Op> {
        let site = self.def_site(value)?;
        self.block(site.block)?.ops.get(site.op)
    }

    fn value_ty(&self, value: ValueId) -> Option<TypeId> {
        self.func.values.get(value.0 as usize).map(|def| def.ty)
    }

    fn value_def_block(&self, value: ValueId) -> Option<RBlockId> {
        if let Some((block, _)) = self.def_use.def_site(value) {
            return Some(block);
        }
        self.def_use.param_site(value).map(|(block, _)| block)
    }

    fn match_scalarized_loop(&self, header: RBlockId) -> Option<LoopMatch> {
        let shape = self.match_header_shape(header)?;
        let region = self.match_loop_region(&shape)?;
        let bounds = self.match_cursor_bounds(&shape, &region)?;
        let driver = self.match_driver_step_and_elem(&shape, &region, &bounds)?;
        let enumerated = self.match_optional_enumerate(&region, &bounds, &driver);
        self.verify_isolation_and_exit(&shape, region, bounds, driver, enumerated)
    }

    /// Stage 1: verifies that `header` is a parameterized comparison-switch loop header whose ops
    /// compute only the branch condition without escaping.
    fn match_header_shape(&self, header_id: RBlockId) -> Option<HeaderShape> {
        let header = self.block(header_id)?;
        if header.params.is_empty() {
            return None;
        }
        let bool_ty = bool_type(self.table)?;
        let RTerm::Switch {
            scrut,
            scrut_ty,
            cases,
            default,
        } = &header.term
        else {
            return None;
        };
        if *scrut_ty != bool_ty || !default.1.is_empty() {
            return None;
        }
        let (case_val, case_target) = match cases.as_slice() {
            [(val @ (0 | 1), target, args)] if args.is_empty() => (*val, *target),
            _ => return None,
        };
        let cmp_val = self.strip_copies_local(header_id, *scrut);
        let cmp_op = header.ops.iter().find(|op| op.result == Some(cmp_val))?;
        let OpKind::Binary {
            op: cmp_kind @ (BinaryOp::Eq | BinaryOp::Ne),
            l,
            r,
            ty: cmp_ty,
        } = cmp_op.kind
        else {
            return None;
        };
        let (continue_to, exit_target) = match (cmp_kind, case_val) {
            (BinaryOp::Eq, 0) | (BinaryOp::Ne, 1) => (case_target, default.0),
            (BinaryOp::Eq, 1) | (BinaryOp::Ne, 0) => (default.0, case_target),
            _ => return None,
        };

        // Header ops must only compute the comparison driving the switch and must not escape header.
        let header_results: BTreeSet<ValueId> =
            header.ops.iter().filter_map(|op| op.result).collect();
        for op in &header.ops {
            let allowed = matches!(
                op.kind,
                OpKind::Copy(_)
                    | OpKind::Move(_)
                    | OpKind::Cast { .. }
                    | OpKind::Field { .. }
                    | OpKind::Aggregate { .. }
                    | OpKind::BorrowView { .. }
                    | OpKind::Binary {
                        op: BinaryOp::Eq | BinaryOp::Ne,
                        ..
                    }
            );
            if !allowed || op.result.is_none() {
                return None;
            }
        }
        if header
            .term
            .uses()
            .iter()
            .any(|u| header_results.contains(u) && *u != *scrut)
        {
            return None;
        }

        Some(HeaderShape {
            header: header_id,
            bool_ty,
            cmp_ty,
            cmp_l: l,
            cmp_r: r,
            cmp_origin: cmp_op.origin,
            continue_to,
            exit_target,
        })
    }

    /// Stage 2: identifies the single preheader, backedges, and dominated natural loop region.
    fn match_loop_region(&self, shape: &HeaderShape) -> Option<LoopRegion> {
        let doms = &self.cfg.doms;
        let header = self.block(shape.header)?;
        let mut predecessors = Vec::new();
        for block in &self.func.blocks {
            if matches!(&block.term, RTerm::Goto(target, args) if *target == header.id && args.len() == header.params.len())
            {
                predecessors.push(block.id);
            } else if block.term.successors().contains(&header.id) {
                return None;
            }
        }
        let outside_preds: Vec<RBlockId> = predecessors
            .iter()
            .copied()
            .filter(|&b| !doms.dominates(header.id, b))
            .collect();
        let [preheader] = outside_preds.as_slice() else {
            return None;
        };
        let preheader = *preheader;
        let backedges: Vec<RBlockId> = predecessors
            .into_iter()
            .filter(|&b| doms.dominates(header.id, b))
            .collect();
        if backedges.is_empty() || !doms.dominates(preheader, header.id) {
            return None;
        }
        let scc: BTreeSet<RBlockId> = self.cfg.natural_loop(header.id);
        if !scc.contains(&shape.continue_to)
            || scc.contains(&shape.exit_target)
            || !scc.iter().all(|&block| doms.dominates(header.id, block))
        {
            return None;
        }
        Some(LoopRegion {
            header: shape.header,
            preheader,
            backedges,
            scc,
        })
    }

    /// Stage 3: matches `(ptr_param, end)` from either orientation of the header comparison and
    /// verifies that `end == raw_base + slice.len()` for an immutable non-ZST slice.
    fn match_cursor_bounds(
        &self,
        shape: &HeaderShape,
        region: &LoopRegion,
    ) -> Option<CursorBounds> {
        let header = self.block(region.header)?;
        let preheader_args = match &self.block(region.preheader)?.term {
            RTerm::Goto(_, args) => args.as_slice(),
            _ => return None,
        };
        self.match_cursor_orientation(
            shape,
            region,
            header,
            preheader_args,
            shape.cmp_l,
            shape.cmp_r,
        )
        .or_else(|| {
            self.match_cursor_orientation(
                shape,
                region,
                header,
                preheader_args,
                shape.cmp_r,
                shape.cmp_l,
            )
        })
    }

    fn match_cursor_orientation(
        &self,
        shape: &HeaderShape,
        region: &LoopRegion,
        header: &RBlock,
        preheader_args: &[ValueId],
        ptr_side: ValueId,
        end_side: ValueId,
    ) -> Option<CursorBounds> {
        let doms = &self.cfg.doms;
        let ptr_root = self.strip_ptr_wrapper_stop_params(&header.params, ptr_side)?;
        let p_idx = header.params.iter().position(|p| *p == ptr_root)?;
        let raw = self.strip_ptr_wrapper(preheader_args[p_idx])?;
        let source = match self.definition(raw).map(|op| &op.kind) {
            Some(OpKind::RawView {
                root: Root::Value(source),
                path,
                mutable: false,
            }) if path.as_slice() == [Proj::Deref] => *source,
            Some(OpKind::SliceView { .. }) => raw,
            _ if is_immutable_slice_ref(self.func, self.table, raw) => raw,
            _ => return None,
        };
        let source_block = self.value_def_block(source)?;
        if region.scc.contains(&source_block) || !doms.dominates(source_block, region.preheader) {
            return None;
        }
        let elem_ty = sized_non_zst_slice_element(self.table, self.value_ty(source)?)?;
        if !is_raw_ptr_to(self.table, shape.cmp_ty, elem_ty) {
            return None;
        }
        let end_leaf = self.strip_ptr_wrapper_until(&header.params, end_side)?;
        let (end_root, end_param_idx) =
            if let Some(e_idx) = header.params.iter().position(|p| *p == end_leaf) {
                if e_idx == p_idx {
                    return None;
                }
                for &be in &region.backedges {
                    let RTerm::Goto(_, args) = &self.block(be)?.term else {
                        return None;
                    };
                    if args.get(e_idx).copied() != Some(end_leaf) {
                        return None;
                    }
                }
                (self.strip_ptr_wrapper(preheader_args[e_idx])?, Some(e_idx))
            } else {
                (end_leaf, None)
            };
        let end_site = self.def_site(end_root)?;
        if region.scc.contains(&end_site.block) || !doms.dominates(end_site.block, region.preheader)
        {
            return None;
        }
        let OpKind::Binary {
            op: BinaryOp::Offset,
            l: end_base,
            r: len,
            ty: end_ty,
        } = self.definition(end_root)?.kind
        else {
            return None;
        };
        if !is_raw_ptr_to(self.table, end_ty, elem_ty) {
            return None;
        }
        let end_base_root = self.strip_ptr_wrapper(end_base)?;
        if end_base_root != raw && end_base_root != source {
            return None;
        }
        if !self.source_length_matches(source, len) {
            return None;
        }
        let usize_ty = self.value_ty(len)?;
        if self
            .table
            .int_info(usize_ty)
            .is_none_or(|(signed, bits)| signed || bits != 32)
        {
            return None;
        }
        Some(CursorBounds {
            p_idx,
            ptr_param: header.params[p_idx],
            source,
            len,
            end_param_idx,
            elem_ty,
            usize_ty,
        })
    }

    /// Stage 4: matches the driver block's `Offset(ptr, 1)` step, backedge advancement, and
    /// element borrow/deref ops.
    fn match_driver_step_and_elem(
        &self,
        shape: &HeaderShape,
        region: &LoopRegion,
        bounds: &CursorBounds,
    ) -> Option<DriverMatch> {
        let driver = shape.continue_to;
        let driver_block = self.block(driver)?;
        if !driver_block.params.is_empty() {
            return None;
        }

        // Find next_ptr = Offset(ptr_param, 1) in driver.
        let mut removed_driver_ops: BTreeSet<usize> = BTreeSet::new();
        let mut zst_candidates: Vec<ValueId> = Vec::new();
        let mut next_ptr_op_idx = None;
        let mut next_ptr_base_val = None;
        let mut next_ptr_val = None;
        let mut one_val = None;
        for (op_idx, op) in driver_block.ops.iter().enumerate() {
            if let OpKind::Binary {
                op: BinaryOp::Offset,
                l: base,
                r: one,
                ty: next_ty,
            } = op.kind
            {
                if is_raw_ptr_to(self.table, next_ty, bounds.elem_ty)
                    && self.strip_ptr_wrapper(base) == Some(bounds.ptr_param)
                    && self.is_scalar(one, 1)
                {
                    next_ptr_op_idx = Some(op_idx);
                    next_ptr_base_val = Some(base);
                    next_ptr_val = op.result;
                    one_val = Some(one);
                    break;
                }
            }
        }
        let next_ptr_idx = next_ptr_op_idx?;
        let next_ptr_base = next_ptr_base_val?;
        let next_ptr = next_ptr_val?;
        let one_val = one_val?;
        removed_driver_ops.insert(next_ptr_idx);
        self.collect_ptr_wrapper_chain_in_block(
            driver,
            next_ptr_base,
            bounds.ptr_param,
            &mut removed_driver_ops,
            &mut zst_candidates,
        );
        if let Some(site) = self.def_site(one_val) {
            if site.block == driver {
                // Only remove `one_val` if unused outside removed_ops.
                let used_elsewhere = self.func.blocks.iter().any(|b| {
                    b.ops.iter().enumerate().any(|(oi, op)| {
                        !(b.id == driver
                            && (oi == next_ptr_idx || removed_driver_ops.contains(&oi)))
                            && op.kind.uses().contains(&one_val)
                    }) || b.term.uses().contains(&one_val)
                });
                if !used_elsewhere {
                    removed_driver_ops.insert(site.op);
                }
            }
        }

        // Check every backedge passes next_ptr (or wrapper of it) at p_idx and is reachable from driver.
        let driver_reach = distances(self.func, driver);
        let mut invariant_param_candidates: Vec<(RBlockId, usize, ValueId)> = Vec::new();
        for &be in &region.backedges {
            driver_reach[be.0 as usize]?;
            let RTerm::Goto(_, args) = &self.block(be)?.term else {
                return None;
            };
            let arg = args[bounds.p_idx];
            if self.strip_ptr_wrapper(arg) != Some(next_ptr) {
                return None;
            }
            self.collect_backedge_wrapper_chain(
                driver,
                arg,
                next_ptr,
                &mut removed_driver_ops,
                &mut zst_candidates,
                &mut invariant_param_candidates,
            )?;
        }
        let removed_invariant_params =
            self.validate_dead_invariant_params(region, bounds.p_idx, &invariant_param_candidates)?;

        // Find old_item_ref and/or old_item in driver.
        let mut old_item_ref = None;
        let mut old_item_ref_op = None;
        let mut old_item = None;
        let mut item_ref_ty = None;
        let mut origin = shape.cmp_origin;

        for (op_idx, op) in driver_block.ops.iter().enumerate() {
            if let OpKind::BorrowView {
                root: Root::Value(ptr),
                path,
                mutable: false,
            } = &op.kind
            {
                if path.as_slice() == [Proj::Deref]
                    && self.strip_ptr_wrapper(*ptr) == Some(bounds.ptr_param)
                {
                    if let Some(res) = op.result {
                        if let Some(ty) = self.value_ty(res) {
                            if is_ref_to(self.table, ty, bounds.elem_ty) {
                                old_item_ref = Some(res);
                                old_item_ref_op = Some(op_idx);
                                item_ref_ty = Some(ty);
                                origin = op.origin;
                                self.collect_ptr_wrapper_chain_in_block(
                                    driver,
                                    *ptr,
                                    bounds.ptr_param,
                                    &mut removed_driver_ops,
                                    &mut zst_candidates,
                                );
                                break;
                            }
                        }
                    }
                }
            }
        }
        if let Some(ref_val) = old_item_ref {
            removed_driver_ops.insert(old_item_ref_op?);
            for (op_idx, op) in driver_block.ops.iter().enumerate() {
                if let OpKind::Deref(target) = op.kind {
                    if self.strip_copies_local(driver, target) == ref_val {
                        if let Some(res) = op.result {
                            if self.value_ty(res) == Some(bounds.elem_ty) {
                                old_item = Some(res);
                                origin = op.origin;
                                removed_driver_ops.insert(op_idx);
                                break;
                            }
                        }
                    }
                }
            }
        } else {
            for (op_idx, op) in driver_block.ops.iter().enumerate() {
                if let OpKind::Deref(ptr) = op.kind {
                    if self.strip_ptr_wrapper(ptr) == Some(bounds.ptr_param) {
                        if let Some(res) = op.result {
                            if self.value_ty(res) == Some(bounds.elem_ty) {
                                old_item = Some(res);
                                origin = op.origin;
                                removed_driver_ops.insert(op_idx);
                                self.collect_ptr_wrapper_chain_in_block(
                                    driver,
                                    ptr,
                                    bounds.ptr_param,
                                    &mut removed_driver_ops,
                                    &mut zst_candidates,
                                );
                                break;
                            }
                        }
                    }
                }
            }
            item_ref_ty = ref_to_type(self.table, bounds.elem_ty);
        }
        let item_ref_ty = item_ref_ty?;
        if old_item_ref.is_none() && old_item.is_none() {
            return None;
        }
        self.collect_unused_zst_ops_in_block(driver, zst_candidates, &mut removed_driver_ops);

        Some(DriverMatch {
            driver,
            removed_driver_ops,
            removed_invariant_params,
            old_item_ref,
            old_item,
            item_ref_ty,
            origin,
        })
    }

    /// Stage 5: recognizes an optional scalarized `Enumerate` counter parameter on the header.
    fn match_optional_enumerate(
        &self,
        region: &LoopRegion,
        bounds: &CursorBounds,
        driver: &DriverMatch,
    ) -> EnumerateMatch {
        let header = &self.func.blocks[region.header.0 as usize];
        let preheader_args = match &self.func.blocks[region.preheader.0 as usize].term {
            RTerm::Goto(_, args) => args.as_slice(),
            _ => &[],
        };
        let mut removed_driver_ops = driver.removed_driver_ops.clone();
        let mut removed_header_params = vec![bounds.p_idx];
        if let Some(e_idx) = bounds.end_param_idx {
            removed_header_params.push(e_idx);
        }
        let mut adapter = Adapter::Plain;
        let mut old_index = None;

        for (i_idx, &idx_param) in header.params.iter().enumerate() {
            if i_idx == bounds.p_idx
                || Some(i_idx) == bounds.end_param_idx
                || self.value_ty(idx_param) != Some(bounds.usize_ty)
            {
                continue;
            }
            if !preheader_args
                .get(i_idx)
                .is_some_and(|&arg| self.is_scalar(arg, 0))
            {
                continue;
            }
            if let Some((counter_ops, next_idx)) = self.match_scalarized_enumerate_counter(
                region,
                driver.driver,
                i_idx,
                idx_param,
                bounds.usize_ty,
            ) {
                let mut candidate_removed = removed_driver_ops.clone();
                candidate_removed.extend(counter_ops.iter().copied());
                let mut removed_map = BTreeMap::new();
                removed_map.insert(region.header, (0..header.ops.len()).collect());
                removed_map.insert(driver.driver, candidate_removed);
                let mut candidate_params = removed_header_params.clone();
                candidate_params.push(i_idx);
                if self.removed_ops_are_local(&LocalityCheck {
                    region,
                    removed_ops: &removed_map,
                    removed_invariant_params: &driver.removed_invariant_params,
                    bypassed_driver: None,
                    removed_params: &candidate_params,
                    exposed: &[Some(idx_param), driver.old_item_ref, driver.old_item],
                    extra_backedge_roots: &[next_idx],
                }) {
                    removed_driver_ops.extend(counter_ops);
                    removed_header_params.push(i_idx);
                    adapter = Adapter::Enumerate;
                    old_index = Some(idx_param);
                    break;
                }
            }
        }

        EnumerateMatch {
            adapter,
            removed_driver_ops,
            removed_header_params,
            old_index,
        }
    }

    /// Stage 6: checks forwarding-driver bypass, removed-op locality, non-escape of tainted
    /// pointer state, and discardable exit chains, then assembles [`LoopMatch`].
    fn verify_isolation_and_exit(
        &self,
        _shape: &HeaderShape,
        region: LoopRegion,
        bounds: CursorBounds,
        driver: DriverMatch,
        enumerated: EnumerateMatch,
    ) -> Option<LoopMatch> {
        let header = self.block(region.header)?;
        let driver_block = self.block(driver.driver)?;
        let mut removed_ops = BTreeMap::new();
        removed_ops.insert(region.header, (0..header.ops.len()).collect());
        removed_ops.insert(driver.driver, enumerated.removed_driver_ops.clone());

        // Check if driver is a pure forwarding block to `body(item_ref)` or `body()`.
        let mut body = driver.driver;
        let mut pass_item_ref = false;
        let driver_remaining = driver_block
            .ops
            .len()
            .saturating_sub(enumerated.removed_driver_ops.len());
        let sole_header_pred =
            driver.driver != RBlockId(0)
                && self.func.blocks.iter().all(|b| {
                    b.id == region.header || !b.term.successors().contains(&driver.driver)
                });
        if driver_remaining == 0 && sole_header_pred {
            if let RTerm::Goto(next_b, args) = &driver_block.term {
                if *next_b != region.header && region.scc.contains(next_b) {
                    let body_block = self.block(*next_b)?;
                    if args.is_empty() && body_block.params.is_empty() {
                        body = *next_b;
                    } else if let (Some(ref_val), [arg], [_]) = (
                        driver.old_item_ref,
                        args.as_slice(),
                        body_block.params.as_slice(),
                    ) {
                        if *arg == ref_val && driver.old_item.is_none() {
                            body = *next_b;
                            pass_item_ref = true;
                        }
                    }
                }
            }
        }

        let bypassed_driver = (body != driver.driver).then_some(driver.driver);
        if !self.removed_ops_are_local(&LocalityCheck {
            region: &region,
            removed_ops: &removed_ops,
            removed_invariant_params: &driver.removed_invariant_params,
            bypassed_driver,
            removed_params: &enumerated.removed_header_params,
            exposed: &[
                enumerated.old_index,
                if pass_item_ref {
                    None
                } else {
                    driver.old_item_ref
                },
                driver.old_item,
            ],
            extra_backedge_roots: &[],
        }) {
            return None;
        }
        removed_ops.remove(&region.header);

        let taint_seeds = vec![bounds.ptr_param];
        let tainted = self.tainted_values(
            &taint_seeds,
            enumerated.old_index,
            driver.old_item_ref,
            driver.old_item,
        );
        if !self.non_escaping(&region.scc, &tainted) {
            return None;
        }

        let mut exits = BTreeSet::new();
        for &block in &region.scc {
            for target in self.func.blocks[block.0 as usize].term.successors() {
                if !region.scc.contains(&target) {
                    exits.insert(target);
                }
            }
        }
        let exit = self.common_exit(&region.scc, &exits)?;
        if !self.block(exit)?.params.is_empty()
            || !exits
                .iter()
                .all(|&start| self.discardable_exit_chain(&tainted, start, exit))
        {
            return None;
        }

        let (_, usize_bits) = self.table.int_info(bounds.usize_ty)?;
        let usize_bytes = u8::try_from(usize_bits / 8).ok()?;
        let (cursor_ty, cursor_bytes) =
            cursor_type_for_slice(self.table, bounds.usize_ty, usize_bytes);

        Some(LoopMatch {
            adapter: enumerated.adapter,
            header: region.header,
            preheader: region.preheader,
            backedges: region.backedges,
            driver: driver.driver,
            body,
            removed_ops,
            removed_header_params: enumerated.removed_header_params,
            removed_invariant_params: driver.removed_invariant_params,
            rewrite_blocks: region.scc,
            pass_item_ref,
            exit,
            source: bounds.source,
            len: bounds.len,
            old_index: enumerated.old_index,
            old_item_ref: if pass_item_ref {
                None
            } else {
                driver.old_item_ref
            },
            old_byte: driver.old_item,
            item_ref_ty: driver.item_ref_ty,
            byte_ty: bounds.elem_ty,
            cursor_ty,
            cursor_bytes,
            usize_ty: bounds.usize_ty,
            bool_ty: _shape.bool_ty,
            origin: driver.origin,
        })
    }

    fn match_scalarized_enumerate_counter(
        &self,
        region: &LoopRegion,
        driver: RBlockId,
        i_idx: usize,
        idx_param: ValueId,
        usize_ty: TypeId,
    ) -> Option<(Vec<usize>, ValueId)> {
        let ops = &self.func.blocks[driver.0 as usize].ops;
        let mut next_idx_op = None;
        let mut next_idx = None;
        let mut one_val = None;
        let mut before_val = None;
        for (op_idx, op) in ops.iter().enumerate() {
            if let OpKind::Binary {
                op: BinaryOp::Add | BinaryOp::AddUnchecked,
                l,
                r,
                ty,
            } = op.kind
            {
                if ty == usize_ty
                    && self.strip_copies_local(driver, l) == idx_param
                    && self.is_scalar(r, 1)
                {
                    next_idx_op = Some(op_idx);
                    next_idx = op.result;
                    before_val = Some(l);
                    one_val = Some(r);
                    break;
                }
            }
        }
        let next_idx_op = next_idx_op?;
        let next_idx = next_idx?;
        let before_val = before_val?;
        let one_val = one_val?;
        for &be in &region.backedges {
            let RTerm::Goto(_, args) = &self.block(be)?.term else {
                return None;
            };
            if self.strip_copies(args[i_idx])? != next_idx {
                return None;
            }
        }
        // Check idx_param does not escape scc.
        for b in &self.func.blocks {
            if !region.scc.contains(&b.id)
                && (b.ops.iter().any(|op| op.kind.uses().contains(&idx_param))
                    || b.term.uses().contains(&idx_param))
            {
                return None;
            }
        }

        let mut counter_ops = vec![next_idx_op];
        if let Some(site) = self.def_site(before_val) {
            if site.block == driver && before_val != idx_param {
                counter_ops.push(site.op);
            }
        }
        if let Some(site) = self.def_site(one_val) {
            if site.block == driver {
                let used_elsewhere = self.func.blocks.iter().any(|b| {
                    b.ops.iter().enumerate().any(|(oi, op)| {
                        !(b.id == driver && oi == next_idx_op) && op.kind.uses().contains(&one_val)
                    }) || b.term.uses().contains(&one_val)
                });
                if !used_elsewhere {
                    counter_ops.push(site.op);
                }
            }
        }
        // Also match optional overflow check ops on (next_idx, before).
        for (op_idx, op) in ops.iter().enumerate() {
            if let OpKind::Assert {
                cond,
                expected: false,
                msg:
                    RAssertMessage::Overflow {
                        op: BinaryOp::Add,
                        left,
                        right,
                    },
                ..
            } = &op.kind
            {
                if self.is_scalar(*right, 1) && self.strip_copies_local(driver, *left) == idx_param
                {
                    let cmp = self.strip_copies_local(driver, *cond);
                    if let Some(cmp_site) = self.def_site(cmp) {
                        if cmp_site.block == driver {
                            if let OpKind::Binary {
                                op: BinaryOp::Lt,
                                l,
                                r,
                                ty,
                            } = ops[cmp_site.op].kind
                            {
                                if ty == usize_ty
                                    && l == next_idx
                                    && self.strip_copies_local(driver, r) == idx_param
                                {
                                    counter_ops.push(op_idx);
                                    counter_ops.push(cmp_site.op);
                                    if let Some(cs) = self.def_site(*cond) {
                                        if cs.block == driver {
                                            counter_ops.push(cs.op);
                                        }
                                    }
                                    if let Some(ls) = self.def_site(*left) {
                                        if ls.block == driver && *left != idx_param {
                                            counter_ops.push(ls.op);
                                        }
                                    }
                                    if let Some(rs) = self.def_site(*right) {
                                        if rs.block == driver {
                                            counter_ops.push(rs.op);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        counter_ops.sort_unstable();
        counter_ops.dedup();
        Some((counter_ops, next_idx))
    }

    fn strip_copies_local(&self, block: RBlockId, mut val: ValueId) -> ValueId {
        for _ in 0..8 {
            let Some(op) = self.func.blocks[block.0 as usize]
                .ops
                .iter()
                .find(|op| op.result == Some(val))
            else {
                break;
            };
            match op.kind {
                OpKind::Copy(s) | OpKind::Move(s) => val = s,
                _ => break,
            }
        }
        val
    }

    fn strip_ptr_wrapper(&self, value: ValueId) -> Option<ValueId> {
        self.strip_ptr_wrapper_until(&[], value)
    }

    fn strip_ptr_wrapper_until(
        &self,
        stop_params: &[ValueId],
        mut value: ValueId,
    ) -> Option<ValueId> {
        for _ in 0..24 {
            if stop_params.contains(&value) {
                return Some(value);
            }
            let Some(op) = self.definition(value) else {
                if let Some(source) = self.single_edge.get(value.0 as usize).copied().flatten() {
                    value = source;
                    continue;
                }
                return Some(value);
            };
            match &op.kind {
                OpKind::Copy(source) | OpKind::Move(source) => {
                    value = *source;
                }
                OpKind::Cast {
                    kind,
                    v: source,
                    from,
                    to,
                } if is_lossless_ptr_cast(self.table, *kind, *from, *to) => {
                    value = *source;
                }
                OpKind::Field {
                    base,
                    variant: None,
                    field,
                } if self
                    .value_ty(*base)
                    .and_then(|ty| ptr_wrapper_field(self.table, ty))
                    == Some(*field as usize) =>
                {
                    value = *base;
                }
                OpKind::Aggregate {
                    ty,
                    variant: 0,
                    active_field: None,
                    fields,
                    ..
                } => {
                    let Some(idx) = ptr_wrapper_aggregate_field(self.table, *ty, fields.len())
                    else {
                        return Some(value);
                    };
                    value = fields[idx];
                }
                _ => return Some(value),
            }
        }
        None
    }

    fn strip_ptr_wrapper_stop_params(
        &self,
        params: &[ValueId],
        mut value: ValueId,
    ) -> Option<ValueId> {
        for _ in 0..24 {
            if params.contains(&value) {
                return Some(value);
            }
            let Some(op) = self.definition(value) else {
                value = self.single_edge.get(value.0 as usize).copied().flatten()?;
                continue;
            };
            match &op.kind {
                OpKind::Copy(source) | OpKind::Move(source) => {
                    value = *source;
                }
                OpKind::Cast {
                    kind,
                    v: source,
                    from,
                    to,
                } if is_lossless_ptr_cast(self.table, *kind, *from, *to) => {
                    value = *source;
                }
                OpKind::Field {
                    base,
                    variant: None,
                    field,
                } if self
                    .value_ty(*base)
                    .and_then(|ty| ptr_wrapper_field(self.table, ty))
                    == Some(*field as usize) =>
                {
                    value = *base;
                }
                OpKind::Aggregate {
                    ty,
                    variant: 0,
                    active_field: None,
                    fields,
                    ..
                } => {
                    let idx = ptr_wrapper_aggregate_field(self.table, *ty, fields.len())?;
                    value = fields[idx];
                }
                _ => return None,
            }
        }
        None
    }

    fn collect_ptr_wrapper_chain_in_block(
        &self,
        block: RBlockId,
        mut value: ValueId,
        stop: ValueId,
        out: &mut BTreeSet<usize>,
        zst_candidates: &mut Vec<ValueId>,
    ) {
        for _ in 0..16 {
            if value == stop {
                break;
            }
            let Some(site) = self.def_site(value) else {
                break;
            };
            if site.block != block {
                break;
            }
            let op = &self.func.blocks[block.0 as usize].ops[site.op];
            let next = match &op.kind {
                OpKind::Copy(s) | OpKind::Move(s) => *s,
                OpKind::Cast {
                    kind,
                    v: s,
                    from,
                    to,
                } if is_lossless_ptr_cast(self.table, *kind, *from, *to) => *s,
                OpKind::Field {
                    base,
                    variant: None,
                    field,
                } if self
                    .value_ty(*base)
                    .and_then(|ty| ptr_wrapper_field(self.table, ty))
                    == Some(*field as usize) =>
                {
                    *base
                }
                OpKind::Aggregate {
                    ty,
                    variant: 0,
                    active_field: None,
                    fields,
                    ..
                } => {
                    let Some(idx) = ptr_wrapper_aggregate_field(self.table, *ty, fields.len())
                    else {
                        break;
                    };
                    for (i, &field_val) in fields.iter().enumerate() {
                        if i != idx {
                            zst_candidates.push(field_val);
                        }
                    }
                    fields[idx]
                }
                _ => break,
            };
            out.insert(site.op);
            value = next;
        }
    }

    fn collect_backedge_wrapper_chain(
        &self,
        driver: RBlockId,
        arg: ValueId,
        next_ptr: ValueId,
        out: &mut BTreeSet<usize>,
        zst_candidates: &mut Vec<ValueId>,
        invariant_param_candidates: &mut Vec<(RBlockId, usize, ValueId)>,
    ) -> Option<()> {
        let mut cur = arg;
        for _ in 0..24 {
            if cur == next_ptr {
                return Some(());
            }
            let Some(site) = self.def_site(cur) else {
                let (param_block, param_pos) = self.func.blocks.iter().find_map(|b| {
                    b.params
                        .iter()
                        .position(|p| *p == cur)
                        .map(|pos| (b.id, pos))
                })?;
                invariant_param_candidates.push((param_block, param_pos, cur));
                cur = self.single_edge.get(cur.0 as usize).copied().flatten()?;
                continue;
            };
            if site.block == driver {
                out.insert(site.op);
            }
            cur = match &self.func.blocks[site.block.0 as usize].ops[site.op].kind {
                OpKind::Copy(s) | OpKind::Move(s) => *s,
                OpKind::Cast {
                    kind,
                    v: s,
                    from,
                    to,
                } if is_lossless_ptr_cast(self.table, *kind, *from, *to) => *s,
                OpKind::Field {
                    base,
                    variant: None,
                    field,
                } if self
                    .value_ty(*base)
                    .and_then(|ty| ptr_wrapper_field(self.table, ty))
                    == Some(*field as usize) =>
                {
                    *base
                }
                OpKind::Aggregate {
                    ty,
                    variant: 0,
                    active_field: None,
                    fields,
                    ..
                } => {
                    let idx = ptr_wrapper_aggregate_field(self.table, *ty, fields.len())?;
                    if site.block == driver {
                        for (i, &field_val) in fields.iter().enumerate() {
                            if i != idx {
                                zst_candidates.push(field_val);
                            }
                        }
                    }
                    fields[idx]
                }
                _ => return None,
            };
        }
        None
    }

    fn collect_unused_zst_ops_in_block(
        &self,
        block: RBlockId,
        mut candidates: Vec<ValueId>,
        removed_ops: &mut BTreeSet<usize>,
    ) {
        while let Some(val) = candidates.pop() {
            if !self.value_ty(val).is_some_and(|ty| self.table.is_zst(ty)) {
                continue;
            }
            let Some(site) = self.def_site(val) else {
                continue;
            };
            if site.block != block || removed_ops.contains(&site.op) {
                continue;
            }
            let op = &self.func.blocks[block.0 as usize].ops[site.op];
            let is_pure_zst = matches!(
                op.kind,
                OpKind::Const(_)
                    | OpKind::Copy(_)
                    | OpKind::Move(_)
                    | OpKind::Aggregate {
                        active_field: None,
                        ..
                    }
            );
            if !is_pure_zst {
                continue;
            }
            let used_elsewhere = self.func.blocks.iter().any(|b| {
                b.ops.iter().enumerate().any(|(oi, other)| {
                    !(b.id == block && removed_ops.contains(&oi))
                        && other.kind.uses().contains(&val)
                }) || b.term.uses().contains(&val)
            });
            if !used_elsewhere {
                removed_ops.insert(site.op);
                candidates.extend(op.kind.uses());
            }
        }
    }

    fn validate_dead_invariant_params(
        &self,
        region: &LoopRegion,
        p_idx: usize,
        candidates: &[(RBlockId, usize, ValueId)],
    ) -> Option<BTreeMap<RBlockId, Vec<usize>>> {
        let mut map: BTreeMap<RBlockId, Vec<usize>> = BTreeMap::new();
        if candidates.is_empty() {
            return Some(map);
        }
        let backedge_set: BTreeSet<RBlockId> = region.backedges.iter().copied().collect();
        let mut candidate_vals = BTreeSet::new();
        let mut candidate_slots = BTreeSet::new();
        for &(b_id, pos, val) in candidates {
            if b_id == region.header || !region.scc.contains(&b_id) {
                return None;
            }
            candidate_vals.insert(val);
            candidate_slots.insert((b_id, pos));
            map.entry(b_id).or_default().push(pos);
        }
        for positions in map.values_mut() {
            positions.sort_unstable();
            positions.dedup();
        }
        for block in &self.func.blocks {
            if block
                .ops
                .iter()
                .any(|op| op.kind.uses().iter().any(|u| candidate_vals.contains(u)))
            {
                return None;
            }
            let scrut_uses = match &block.term {
                RTerm::Switch { scrut, .. } => Some(*scrut),
                RTerm::Return(Some(v)) | RTerm::Throw(Some(v)) => Some(*v),
                _ => None,
            };
            if scrut_uses.is_some_and(|u| candidate_vals.contains(&u)) {
                return None;
            }
            for (target, args) in block.term.edges() {
                if map.contains_key(&target)
                    && args.len() != self.func.blocks[target.0 as usize].params.len()
                {
                    return None;
                }
                for (pos, arg) in args.iter().enumerate() {
                    if !candidate_vals.contains(arg) {
                        continue;
                    }
                    let allowed_edge = (target == region.header
                        && pos == p_idx
                        && backedge_set.contains(&block.id))
                        || candidate_slots.contains(&(target, pos));
                    if !allowed_edge {
                        return None;
                    }
                }
            }
        }
        Some(map)
    }

    fn removed_ops_are_local(&self, check: &LocalityCheck<'_>) -> bool {
        let region = check.region;
        let mut removed = BTreeSet::new();
        for &param_idx in check.removed_params {
            if let Some(&param) = self.func.blocks[region.header.0 as usize]
                .params
                .get(param_idx)
            {
                removed.insert(param);
            }
        }
        for (&block_id, ops) in check.removed_ops {
            for &op_idx in ops {
                if let Some(res) = self.func.blocks[block_id.0 as usize].ops[op_idx].result {
                    removed.insert(res);
                }
            }
        }
        let mut allowed = BTreeSet::new();
        for v in check.exposed.iter().copied().flatten() {
            allowed.insert(v);
        }
        let backedge_set: BTreeSet<RBlockId> = region.backedges.iter().copied().collect();
        let param_set: BTreeSet<usize> = check.removed_params.iter().copied().collect();
        for block in &self.func.blocks {
            if block.id == region.header {
                continue;
            }
            let is_bypassed = check.bypassed_driver == Some(block.id);
            for (op_index, op) in block.ops.iter().enumerate() {
                if check
                    .removed_ops
                    .get(&block.id)
                    .is_some_and(|set| set.contains(&op_index))
                {
                    continue;
                }
                if op
                    .kind
                    .uses()
                    .iter()
                    .any(|value| removed.contains(value) && !allowed.contains(value))
                {
                    return false;
                }
                if (!region.scc.contains(&block.id) || is_bypassed)
                    && op.kind.uses().iter().any(|value| allowed.contains(value))
                {
                    return false;
                }
            }
            if is_bypassed {
                if block
                    .term
                    .uses()
                    .iter()
                    .any(|value| allowed.contains(value))
                {
                    return false;
                }
                continue;
            }
            let non_edge_use = match &block.term {
                RTerm::Switch { scrut, .. } => Some(*scrut),
                RTerm::Return(Some(v)) | RTerm::Throw(Some(v)) => Some(*v),
                _ => None,
            };
            if let Some(v) = non_edge_use {
                if (removed.contains(&v) && !allowed.contains(&v))
                    || (!region.scc.contains(&block.id) && allowed.contains(&v))
                {
                    return false;
                }
            }
            let is_backedge = backedge_set.contains(&block.id);
            for (target, args) in block.term.edges() {
                let skip_invariant = check.removed_invariant_params.get(&target);
                for (pos, arg) in args.iter().enumerate() {
                    if is_backedge && target == region.header && param_set.contains(&pos) {
                        continue;
                    }
                    if skip_invariant.is_some_and(|positions| positions.contains(&pos)) {
                        continue;
                    }
                    if removed.contains(arg) && !allowed.contains(arg) {
                        return false;
                    }
                    if is_backedge
                        && check.extra_backedge_roots.contains(arg)
                        && !allowed.contains(arg)
                    {
                        return false;
                    }
                    if !region.scc.contains(&block.id) && allowed.contains(arg) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn tainted_values(
        &self,
        seeds: &[ValueId],
        old_index: Option<ValueId>,
        old_item_ref: Option<ValueId>,
        old_byte: Option<ValueId>,
    ) -> BTreeSet<ValueId> {
        let mut tainted: BTreeSet<ValueId> = seeds.iter().copied().collect();
        for _ in 0..self.func.values.len().max(1) {
            let before = tainted.len();
            for block in &self.func.blocks {
                for (target, args) in block.term.edges() {
                    for (position, value) in args.iter().enumerate() {
                        if tainted.contains(value) {
                            if let Some(param) =
                                self.func.blocks[target.0 as usize].params.get(position)
                            {
                                tainted.insert(*param);
                            }
                        }
                    }
                }
                for op in &block.ops {
                    if op.kind.uses().iter().any(|value| tainted.contains(value)) {
                        if let Some(result) = op.result {
                            if Some(result) != old_index
                                && Some(result) != old_item_ref
                                && Some(result) != old_byte
                            {
                                tainted.insert(result);
                            }
                        }
                    }
                }
            }
            if tainted.len() == before {
                break;
            }
        }
        tainted
    }

    fn non_escaping(&self, loop_blocks: &BTreeSet<RBlockId>, tainted: &BTreeSet<ValueId>) -> bool {
        for block in &self.func.blocks {
            for op in &block.ops {
                if !op.kind.uses().iter().any(|value| tainted.contains(value)) {
                    continue;
                }
                let allowed = match &op.kind {
                    OpKind::Copy(_)
                    | OpKind::Move(_)
                    | OpKind::Field { .. }
                    | OpKind::Discriminant(_)
                    | OpKind::BorrowView { .. }
                    | OpKind::RawView { .. }
                    | OpKind::SliceView { .. }
                    | OpKind::Deref(_)
                    | OpKind::Binary { .. }
                    | OpKind::Unary { .. }
                    | OpKind::Cast { .. } => op.result.is_some(),
                    OpKind::Aggregate { .. } => op.result.is_some_and(|r| tainted.contains(&r)),
                    OpKind::Intrinsic { fallback: None, .. } => op.result.is_some(),
                    OpKind::SetField { base, .. } => tainted.contains(base),
                    OpKind::StoreThrough { ptr, .. } => {
                        tainted.contains(ptr) && self.mutable_borrow(*ptr)
                    }
                    OpKind::Assert { .. } => true,
                    _ => false,
                };
                if !allowed {
                    return false;
                }
            }
            let term_uses_tainted = block
                .term
                .uses()
                .iter()
                .any(|value| tainted.contains(value));
            if term_uses_tainted && !loop_blocks.contains(&block.id) {
                return false;
            }
            if matches!(&block.term, RTerm::Return(value) | RTerm::Throw(value)
                if value.is_some_and(|value| tainted.contains(&value)))
            {
                return false;
            }
        }
        true
    }

    fn mutable_borrow(&self, mut value: ValueId) -> bool {
        for _ in 0..8 {
            match self.definition(value).map(|op| &op.kind) {
                Some(OpKind::Copy(source) | OpKind::Move(source)) => value = *source,
                Some(OpKind::BorrowView { mutable: true, .. }) => return true,
                _ => return false,
            }
        }
        false
    }

    fn common_exit(
        &self,
        scc: &BTreeSet<RBlockId>,
        exits: &BTreeSet<RBlockId>,
    ) -> Option<RBlockId> {
        if exits.is_empty() {
            return None;
        }
        let distances: Vec<Vec<Option<usize>>> = exits
            .iter()
            .map(|&start| distances(self.func, start))
            .collect();
        (0..self.func.blocks.len())
            .filter(|&block| {
                !scc.contains(&RBlockId(block as u32))
                    && distances.iter().all(|row| row[block].is_some())
            })
            .min_by_key(|&block| {
                let ds: Vec<usize> = distances.iter().filter_map(|row| row[block]).collect();
                (
                    ds.iter().copied().max().unwrap_or(usize::MAX),
                    ds.iter().sum::<usize>(),
                    block,
                )
            })
            .map(|block| RBlockId(block as u32))
    }

    fn discardable_exit_chain(
        &self,
        tainted: &BTreeSet<ValueId>,
        mut at: RBlockId,
        exit: RBlockId,
    ) -> bool {
        let mut seen = BTreeSet::new();
        while at != exit {
            if !seen.insert(at) {
                return false;
            }
            let Some(block) = self.block(at) else {
                return false;
            };
            let RTerm::Goto(next, args) = &block.term else {
                return false;
            };
            if !block.params.is_empty()
                || !args.is_empty()
                || block
                    .ops
                    .iter()
                    .any(|op| !self.discardable_tainted_op(tainted, op))
            {
                return false;
            }
            at = *next;
        }
        true
    }

    fn discardable_tainted_op(&self, tainted: &BTreeSet<ValueId>, op: &Op) -> bool {
        match &op.kind {
            OpKind::Copy(_)
            | OpKind::Move(_)
            | OpKind::Field { .. }
            | OpKind::Discriminant(_)
            | OpKind::BorrowView { .. }
            | OpKind::RawView { .. }
            | OpKind::SliceView { .. }
            | OpKind::Deref(_)
            | OpKind::Cast { .. } => op.result.is_some_and(|value| tainted.contains(&value)),
            OpKind::SetField { base, .. } => op.result.is_none() && tainted.contains(base),
            OpKind::StoreThrough { ptr, .. } => {
                op.result.is_none() && tainted.contains(ptr) && self.mutable_borrow(*ptr)
            }
            _ => false,
        }
    }

    fn strip_copies(&self, mut value: ValueId) -> Option<ValueId> {
        for _ in 0..16 {
            let Some(op) = self.definition(value) else {
                value = self.single_edge.get(value.0 as usize).copied().flatten()?;
                continue;
            };
            match op.kind {
                OpKind::Copy(source) | OpKind::Move(source) => value = source,
                _ => return Some(value),
            }
        }
        None
    }

    fn copy_root(&self, mut value: ValueId) -> Option<ValueId> {
        for _ in 0..16 {
            match self.definition(value).map(|op| &op.kind) {
                Some(OpKind::Copy(source) | OpKind::Move(source)) => value = *source,
                Some(_) => return Some(value),
                None => {
                    if let Some(source) = self.single_edge.get(value.0 as usize).copied().flatten()
                    {
                        value = source;
                    } else {
                        return Some(value);
                    }
                }
            }
        }
        None
    }

    fn source_length_matches(&self, source: ValueId, len: ValueId) -> bool {
        let Some(source_root) = self.copy_root(source) else {
            return false;
        };
        let Some(len_root) = self.copy_root(len) else {
            return false;
        };
        let len_metadata_target = match self.definition(len_root).map(|op| &op.kind) {
            Some(OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v,
                ..
            }) => self.copy_root(*v),
            _ => None,
        };
        if len_metadata_target == Some(source_root) {
            return true;
        }
        if let Some(op) = self.definition(source_root) {
            match &op.kind {
                OpKind::SliceView { len: found, .. }
                    if self.copy_root(*found) == Some(len_root) =>
                {
                    return true;
                }
                OpKind::Call {
                    callee: Callee::Runtime { sysroot: true, .. },
                    semantic: CallSemantic::RawParts,
                    args,
                    ..
                } if args.len() == 2 && self.copy_root(args[1]) == Some(len_root) => {
                    return true;
                }
                OpKind::Cast {
                    kind: CastKind::Transmute | CastKind::Subtype | CastKind::PointerCoercion,
                    v: str_val,
                    from,
                    to,
                } if is_str_ref_to_byte_slice_ref(self.table, *from, *to) => {
                    if let Some(str_root) = self.copy_root(*str_val) {
                        if len_metadata_target == Some(str_root) {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }

    fn is_scalar(&self, value: ValueId, expected: u128) -> bool {
        self.strip_copies(value)
            .and_then(|value| self.definition(value))
            .is_some_and(
                |op| matches!(op.kind, OpKind::Const(ConstTree::Scalar { bits, .. }) if bits == expected),
            )
    }
}

fn is_raw_ptr_to(table: &TypeTable, ptr: TypeId, pointee: TypeId) -> bool {
    matches!(
        table.get(ptr).map(|descriptor| &descriptor.kind),
        Some(TypeKind::RawPointer { pointee: found, .. }) if *found == pointee
    )
}

fn ptr_wrapper_field(table: &TypeTable, ty: TypeId) -> Option<usize> {
    let descriptor = table.get(ty)?;
    if descriptor.meta.iterator != IteratorIdentity::None {
        return None;
    }
    let TypeKind::Adt {
        form: NominalForm::Struct,
        variants,
        ..
    } = &descriptor.kind
    else {
        return None;
    };
    let [variant] = variants.as_slice() else {
        return None;
    };
    let mut non_zst = None;
    for (idx, field) in variant.fields.iter().enumerate() {
        if !table.is_zst(field.ty) {
            if non_zst.is_some() {
                return None;
            }
            non_zst = Some((idx, field.ty));
        }
    }
    let (idx, field_ty) = non_zst?;
    matches!(
        table.get(table.peeled(field_ty)).map(|d| &d.kind),
        Some(TypeKind::RawPointer { .. } | TypeKind::Reference { .. })
    )
    .then_some(idx)
}

fn ptr_wrapper_aggregate_field(table: &TypeTable, ty: TypeId, fields_len: usize) -> Option<usize> {
    let idx = ptr_wrapper_field(table, ty)?;
    let TypeKind::Adt { variants, .. } = &table.get(ty)?.kind else {
        return None;
    };
    let [variant] = variants.as_slice() else {
        return None;
    };
    (fields_len == variant.fields.len() && idx < fields_len).then_some(idx)
}

fn resolve_ptr_pointee(table: &TypeTable, mut ty: TypeId) -> Option<TypeId> {
    for _ in 0..8 {
        if let Some(TypeKind::RawPointer { pointee, .. } | TypeKind::Reference { pointee, .. }) =
            table
                .get(table.peeled(ty))
                .map(|descriptor| &descriptor.kind)
        {
            return Some(*pointee);
        }
        let idx = ptr_wrapper_field(table, ty)?;
        let TypeKind::Adt { variants, .. } = &table.get(ty)?.kind else {
            return None;
        };
        ty = variants.first()?.fields.get(idx)?.ty;
    }
    None
}

fn is_lossless_ptr_cast(table: &TypeTable, kind: CastKind, from: TypeId, to: TypeId) -> bool {
    if !matches!(
        kind,
        CastKind::PtrToPtr | CastKind::PointerCoercion | CastKind::Subtype | CastKind::Transmute
    ) {
        return false;
    }
    let (Some(from_p), Some(to_p)) = (
        resolve_ptr_pointee(table, from),
        resolve_ptr_pointee(table, to),
    ) else {
        return false;
    };
    table.peeled(from_p) == table.peeled(to_p)
        || (matches!(kind, CastKind::PtrToPtr)
            && matches!(
                table.get(from_p).map(|descriptor| &descriptor.kind),
                Some(TypeKind::Slice(elem)) if *elem == to_p
            ))
}

fn distances(func: &Func, start: RBlockId) -> Vec<Option<usize>> {
    let mut distance = vec![None; func.blocks.len()];
    let Some(slot) = distance.get_mut(start.0 as usize) else {
        return distance;
    };
    *slot = Some(0);
    let mut queue = VecDeque::from([start]);
    while let Some(block) = queue.pop_front() {
        let next_distance = distance[block.0 as usize].unwrap_or(0) + 1;
        for successor in func.blocks[block.0 as usize].term.successors() {
            let slot = &mut distance[successor.0 as usize];
            if slot.is_none() {
                *slot = Some(next_distance);
                queue.push_back(successor);
            }
        }
    }
    distance
}

fn apply(func: &mut Func, found: LoopMatch) -> SliceIteratorLoop {
    let cursor = fresh_with_type(func, found.cursor_ty, found.origin);
    let body_cursor = fresh_with_type(func, found.cursor_ty, found.origin);
    let bound_len = if found.cursor_ty != found.usize_ty {
        let cast_len = fresh_with_type(func, found.cursor_ty, found.origin);
        func.blocks[found.preheader.0 as usize].ops.push(Op {
            result: Some(cast_len),
            origin: found.origin,
            kind: OpKind::Cast {
                kind: CastKind::IntToInt,
                v: found.len,
                from: found.usize_ty,
                to: found.cursor_ty,
            },
        });
        cast_len
    } else {
        found.len
    };
    let initial = fresh_with_type(func, found.cursor_ty, found.origin);
    func.blocks[found.preheader.0 as usize].ops.push(Op {
        result: Some(initial),
        origin: found.origin,
        kind: OpKind::Const(ConstTree::Scalar {
            bits: 0,
            bytes: found.cursor_bytes,
        }),
    });
    let condition = fresh_bool(func, found.bool_ty, found.origin);
    let index = match found.adapter {
        Adapter::Plain => body_cursor,
        Adapter::Enumerate if found.cursor_ty == found.usize_ty || found.old_index.is_none() => {
            body_cursor
        }
        Adapter::Enumerate => fresh_with_type(func, found.usize_ty, found.origin),
    };
    let item_ref = fresh_with_type(func, found.item_ref_ty, found.origin);
    let byte = found
        .old_byte
        .map(|_| fresh_with_type(func, found.byte_ty, found.origin));
    let one = fresh_with_type(func, found.cursor_ty, found.origin);
    let next = fresh_with_type(func, found.cursor_ty, found.origin);
    let direct = RBlockId(func.blocks.len() as u32);

    let header_ops = vec![Op {
        result: Some(condition),
        origin: found.origin,
        kind: OpKind::Binary {
            op: BinaryOp::Lt,
            l: cursor,
            r: bound_len,
            ty: found.cursor_ty,
        },
    }];
    let mut removed_params = found.removed_header_params.clone();
    removed_params.sort_unstable();
    removed_params.dedup();
    for &idx in removed_params.iter().rev() {
        func.blocks[found.header.0 as usize].params.remove(idx);
    }
    func.blocks[found.header.0 as usize].params.push(cursor);
    func.blocks[found.header.0 as usize].ops = header_ops;
    func.blocks[found.header.0 as usize].term = RTerm::Switch {
        scrut: condition,
        scrut_ty: found.bool_ty,
        cases: vec![(0, found.exit, Vec::new())],
        default: (direct, vec![cursor]),
    };

    let preheader_args = edge_to(
        &mut func.blocks[found.preheader.0 as usize].term,
        found.header,
    )
    .expect("a matched preheader edge exists");
    for &idx in removed_params.iter().rev() {
        preheader_args.remove(idx);
    }
    preheader_args.push(initial);
    for &backedge in &found.backedges {
        let args = edge_to(&mut func.blocks[backedge.0 as usize].term, found.header)
            .expect("a matched backedge exists");
        for &idx in removed_params.iter().rev() {
            args.remove(idx);
        }
        args.push(next);
    }

    let mut direct_ops = Vec::new();
    if found.adapter == Adapter::Enumerate
        && found.cursor_ty != found.usize_ty
        && found.old_index.is_some()
    {
        direct_ops.push(Op {
            result: Some(index),
            origin: found.origin,
            kind: OpKind::Cast {
                kind: CastKind::IntToInt,
                v: body_cursor,
                from: found.cursor_ty,
                to: found.usize_ty,
            },
        });
    }
    direct_ops.push(Op {
        result: Some(item_ref),
        origin: found.origin,
        kind: OpKind::BorrowView {
            root: Root::Value(found.source),
            path: vec![Proj::Deref, Proj::Index(body_cursor)],
            mutable: false,
        },
    });
    if let Some(byte) = byte {
        direct_ops.push(Op {
            result: Some(byte),
            origin: found.origin,
            kind: OpKind::Deref(item_ref),
        });
    }
    direct_ops.push(Op {
        result: Some(one),
        origin: found.origin,
        kind: OpKind::Const(ConstTree::Scalar {
            bits: 1,
            bytes: found.cursor_bytes,
        }),
    });
    direct_ops.push(Op {
        result: Some(next),
        origin: found.origin,
        kind: OpKind::Binary {
            op: BinaryOp::Add,
            l: body_cursor,
            r: one,
            ty: found.cursor_ty,
        },
    });
    let body_args = if found.pass_item_ref {
        vec![item_ref]
    } else {
        Vec::new()
    };
    func.blocks.push(RBlock {
        id: direct,
        params: vec![body_cursor],
        ops: direct_ops,
        term: RTerm::Goto(found.body, body_args),
    });

    if !found.removed_invariant_params.is_empty() {
        for (&block_id, positions) in &found.removed_invariant_params {
            for &pos in positions.iter().rev() {
                func.blocks[block_id.0 as usize].params.remove(pos);
            }
        }
        for block in &mut func.blocks {
            block.term.edges_mut(|target, args| {
                if let Some(positions) = found.removed_invariant_params.get(target) {
                    for &pos in positions.iter().rev() {
                        if pos < args.len() {
                            args.remove(pos);
                        }
                    }
                }
            });
        }
    }
    if !found.removed_ops.is_empty() {
        for (&block_id, drop_set) in &found.removed_ops {
            if block_id == found.header {
                continue;
            }
            let kept = std::mem::take(&mut func.blocks[block_id.0 as usize].ops)
                .into_iter()
                .enumerate()
                .filter(|(i, _)| !drop_set.contains(i))
                .map(|(_, op)| op)
                .collect();
            func.blocks[block_id.0 as usize].ops = kept;
        }
        if found.driver != found.body {
            func.blocks[found.driver.0 as usize].term = RTerm::Unreachable;
        }
    }
    let rewrite = |value: ValueId| {
        if Some(value) == found.old_index {
            index
        } else if Some(value) == found.old_item_ref {
            item_ref
        } else if Some(value) == found.old_byte {
            byte.expect("matched old_byte implies new byte")
        } else {
            value
        }
    };
    for &block in &found.rewrite_blocks {
        if block == found.driver && found.driver != found.body {
            continue;
        }
        for op in &mut func.blocks[block.0 as usize].ops {
            op.kind.visit_values_mut(|v| *v = rewrite(*v));
        }
        func.blocks[block.0 as usize]
            .term
            .visit_values_mut(|v| *v = rewrite(*v));
    }

    // The old adapter remains unreachable until DCE. Keeping structural deletion in the one
    // existing pass means block and value compaction still have a single owner.
    SliceIteratorLoop {
        #[cfg(test)]
        adapter: match found.adapter {
            Adapter::Plain => IteratorIdentity::Slice,
            Adapter::Enumerate => IteratorIdentity::Enumerate,
        },
        #[cfg(test)]
        header: found.header,
        #[cfg(test)]
        direct,
        #[cfg(test)]
        body: found.body,
        #[cfg(test)]
        cursor,
        #[cfg(test)]
        body_cursor,
        #[cfg(test)]
        enumerate_index: index,
        #[cfg(test)]
        byte,
        #[cfg(test)]
        byte_ty: found.byte_ty,
        #[cfg(test)]
        cursor_ty: found.cursor_ty,
    }
}

fn is_immutable_slice_ref(func: &Func, table: &TypeTable, value: ValueId) -> bool {
    let Some(def) = func.values.get(value.0 as usize) else {
        return false;
    };
    if !matches!(def.carrier, Carrier::Baseline | Carrier::SliceView) {
        return false;
    }
    matches!(
        table.get(def.ty).map(|d| &d.kind),
        Some(TypeKind::Reference { mutable: false, .. })
    ) && sized_non_zst_slice_element(table, def.ty).is_some()
}

fn is_str_ref_to_byte_slice_ref(table: &TypeTable, from: TypeId, to: TypeId) -> bool {
    let Some(TypeKind::Reference {
        pointee: from_pointee,
        mutable: false,
    }) = table.get(table.peeled(from)).map(|d| &d.kind)
    else {
        return false;
    };
    if !matches!(
        table.get(table.peeled(*from_pointee)).map(|d| &d.kind),
        Some(TypeKind::Str)
    ) {
        return false;
    }
    let Some(TypeKind::Reference {
        pointee: to_pointee,
        mutable: false,
    }) = table.get(table.peeled(to)).map(|d| &d.kind)
    else {
        return false;
    };
    let Some(TypeKind::Slice(elem)) = table.get(table.peeled(*to_pointee)).map(|d| &d.kind) else {
        return false;
    };
    table.int_info(table.peeled(*elem)) == Some((false, 8))
}

fn edge_to(term: &mut RTerm, target: RBlockId) -> Option<&mut Vec<ValueId>> {
    match term {
        RTerm::Goto(found, args) if *found == target => Some(args),
        _ => None,
    }
}

fn fresh_with_type(func: &mut Func, ty: TypeId, origin: OriginId) -> ValueId {
    func.push_value(ty, Carrier::Baseline, origin)
}

fn fresh_bool(func: &mut Func, ty: TypeId, origin: OriginId) -> ValueId {
    fresh_with_type(func, ty, origin)
}

fn definitions(def_use: &DefUse) -> Vec<Option<Site>> {
    def_use
        .def_op
        .iter()
        .map(|site| site.map(|(block, op)| Site { block, op }))
        .collect()
}

fn single_edge_param_sources(func: &Func, cfg: &Cfg, def_use: &DefUse) -> Vec<Option<ValueId>> {
    let mut sources = vec![None; func.values.len()];
    let incoming = cfg.reachable_incoming_args(func);
    for (b_idx, block) in func.blocks.iter().enumerate() {
        if b_idx == 0 || !cfg.doms.is_reachable(block.id) {
            continue;
        }
        let edges = &incoming[b_idx];
        if edges.is_empty() || edges.iter().any(|args| args.len() != block.params.len()) {
            continue;
        }
        for (pos, &param) in block.params.iter().enumerate() {
            let mut single: Option<ValueId> = None;
            let mut valid = true;
            for args in edges {
                let arg = args[pos];
                if arg == param {
                    continue;
                }
                match single {
                    None => single = Some(arg),
                    Some(known) if known == arg => {}
                    Some(_) => {
                        valid = false;
                        break;
                    }
                }
            }
            if !valid {
                continue;
            }
            let Some(source) = single else {
                continue;
            };
            if edges.len() > 1 {
                let def_b = def_use
                    .def_site(source)
                    .map(|(b, _)| b)
                    .or_else(|| def_use.param_site(source).map(|(b, _)| b));
                let Some(def_b) = def_b else {
                    continue;
                };
                if !cfg.doms.dominates(def_b, block.id) {
                    continue;
                }
            }
            if let Some(slot) = sources.get_mut(param.0 as usize) {
                *slot = Some(source);
            }
        }
    }
    sources
}

fn is_ref_to(table: &TypeTable, reference: TypeId, pointee: TypeId) -> bool {
    matches!(table.get(reference).map(|descriptor| &descriptor.kind), Some(TypeKind::Reference { pointee: found, mutable: false }) if *found == pointee)
}

#[cfg(test)]
#[allow(
    clippy::too_many_lines,
    reason = "unit tests build multi-block slice-iterator fixtures inline"
)]
mod tests {
    use super::*;
    use crate::jir::repr::{FuncBuilder, RAssertMessage};
    use crate::jir::semantic::{AbiSignature, CastKind, DefinitionId, ItemId, Signature, UnaryOp};
    use crate::jir::types::{
        FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts, NominalForm,
        TypeDescriptor, TypeMeta, VariantDescriptor,
    };
    use crate::jir::StrId;

    const BOOL: TypeId = TypeId(1);
    const U8: TypeId = TypeId(2);
    const USIZE: TypeId = TypeId(3);
    const ZST: TypeId = TypeId(4);
    const SLICE: TypeId = TypeId(5);
    const REF_SLICE: TypeId = TypeId(6);
    const RAW_U8: TypeId = TypeId(7);
    const REF_U8: TypeId = TypeId(8);
    const ISIZE: TypeId = TypeId(14);
    const U32: TypeId = TypeId(15);
    const SLICE_U32: TypeId = TypeId(16);
    const REF_SLICE_U32: TypeId = TypeId(17);
    const RAW_U32: TypeId = TypeId(18);
    const REF_U32: TypeId = TypeId(19);
    const NON_NULL_U8: TypeId = TypeId(22);
    const O: OriginId = OriginId(1);

    fn js(id: TypeId, int: Option<IntFacts>, fields: Vec<FieldDescriptor>) -> JsFacts {
        JsFacts {
            int,
            is_indirect: !fields.is_empty(),
            needs_clone: !fields.is_empty(),
            crosses_transparent_union: false,
            transparent: None,
            peeled: id,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: id == U8,
            fields,
        }
    }

    fn field(ty: TypeId, index: u32) -> FieldDescriptor {
        FieldDescriptor {
            key: FieldKey::Index(index),
            ty,
            offset: 0,
        }
    }

    fn scalar(id: TypeId, kind: TypeKind, int: Option<IntFacts>) -> TypeDescriptor {
        TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: js(id, int, Vec::new()),
        }
    }

    fn aggregate_type(
        id: TypeId,
        args: Vec<TypeId>,
        fields: Vec<FieldDescriptor>,
        iterator: IteratorIdentity,
    ) -> TypeDescriptor {
        TypeDescriptor {
            id,
            name: None,
            kind: TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: id.0 as u64,
                },
                args,
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: StrId(0),
                    fields: fields.clone(),
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta {
                iterator,
                ..TypeMeta::default()
            },
            js: js(id, None, fields),
        }
    }

    fn table() -> TypeTable {
        let number = |bits| {
            Some(IntFacts {
                signed: false,
                bits,
                repr: IntRepr::Number,
            })
        };
        let mut table = TypeTable::new();
        table.insert(scalar(BOOL, TypeKind::Bool, None));
        table.insert(scalar(
            U8,
            TypeKind::Integer {
                signed: false,
                bits: 8,
            },
            number(8),
        ));
        table.insert(scalar(
            USIZE,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            number(32),
        ));
        table.insert(scalar(
            ISIZE,
            TypeKind::Integer {
                signed: true,
                bits: 32,
            },
            Some(IntFacts {
                signed: true,
                bits: 32,
                repr: IntRepr::Number,
            }),
        ));
        table.insert(scalar(
            U32,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            number(32),
        ));
        let mut zst = scalar(ZST, TypeKind::Tuple(Vec::new()), None);
        zst.layout.zst = true;
        table.insert(zst);
        table.insert(scalar(SLICE, TypeKind::Slice(U8), None));
        table.insert(scalar(
            REF_SLICE,
            TypeKind::Reference {
                pointee: SLICE,
                mutable: false,
            },
            None,
        ));
        table.insert(scalar(
            RAW_U8,
            TypeKind::RawPointer {
                pointee: U8,
                mutable: false,
            },
            None,
        ));
        table.insert(scalar(
            REF_U8,
            TypeKind::Reference {
                pointee: U8,
                mutable: false,
            },
            None,
        ));
        table.insert(scalar(SLICE_U32, TypeKind::Slice(U32), None));
        table.insert(scalar(
            REF_SLICE_U32,
            TypeKind::Reference {
                pointee: SLICE_U32,
                mutable: false,
            },
            None,
        ));
        table.insert(scalar(
            RAW_U32,
            TypeKind::RawPointer {
                pointee: U32,
                mutable: false,
            },
            None,
        ));
        table.insert(scalar(
            REF_U32,
            TypeKind::Reference {
                pointee: U32,
                mutable: false,
            },
            None,
        ));
        let mut non_null_u8 = aggregate_type(
            NON_NULL_U8,
            vec![U8],
            vec![field(ZST, 0), field(RAW_U8, 1)],
            IteratorIdentity::None,
        );
        non_null_u8.js.peeled = RAW_U8;
        table.insert(non_null_u8);
        table
    }

    fn constant(builder: &mut FuncBuilder, block: RBlockId, bits: u128) -> ValueId {
        builder.op(
            block,
            OpKind::Const(ConstTree::Scalar { bits, bytes: 4 }),
            USIZE,
            O,
        )
    }

    #[test]
    fn complete_rewrite_regions_and_preheaders_cannot_overlap() {
        let mut occupied = BTreeSet::new();
        let b = |id| RBlockId(id);
        assert!(reserve_region(
            &BTreeSet::from([b(1), b(2), b(3)]),
            b(0),
            &mut occupied
        ));
        assert!(!reserve_region(
            &BTreeSet::from([b(3), b(4)]),
            b(5),
            &mut occupied
        ));
        assert!(!reserve_region(
            &BTreeSet::from([b(6), b(7)]),
            b(2),
            &mut occupied
        ));
        assert!(reserve_region(
            &BTreeSet::from([b(8), b(9)]),
            b(10),
            &mut occupied
        ));
    }

    fn build_scalarized_slice_loop(
        elem_ty: TypeId,
        slice_ref_ty: TypeId,
        raw_ty: TypeId,
        ref_ty: TypeId,
        with_enumerate: bool,
    ) -> (Func, TypeTable) {
        let mut builder = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 42,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 1,
                    ..AbiSignature::default()
                },
                arguments: vec![slice_ref_ty],
                result: USIZE,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let header = builder.block();
        let body = builder.block();
        let exit = builder.block();

        let source = builder.param(slice_ref_ty, O);
        let length = builder.op(
            entry,
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v: source,
                ty: slice_ref_ty,
            },
            USIZE,
            O,
        );
        let raw = builder.op(
            entry,
            OpKind::RawView {
                root: Root::Value(source),
                path: vec![Proj::Deref],
                mutable: false,
            },
            raw_ty,
            O,
        );
        let end = builder.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: raw,
                r: length,
                ty: raw_ty,
            },
            raw_ty,
            O,
        );
        let sum_init = constant(&mut builder, entry, 0);
        let mut entry_args = vec![sum_init, raw];
        if with_enumerate {
            let idx_init = constant(&mut builder, entry, 0);
            entry_args.push(idx_init);
        }
        builder.terminate(entry, RTerm::Goto(header, entry_args));

        let sum_param = builder.block_param(header, USIZE, O);
        let ptr_param = builder.block_param(header, raw_ty, O);
        let idx_param = with_enumerate.then(|| builder.block_param(header, USIZE, O));
        let eq = builder.op(
            header,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: ptr_param,
                r: end,
                ty: raw_ty,
            },
            BOOL,
            O,
        );
        builder.terminate(
            header,
            RTerm::Switch {
                scrut: eq,
                scrut_ty: BOOL,
                cases: vec![(0, body, Vec::new())],
                default: (exit, Vec::new()),
            },
        );

        let one = constant(&mut builder, body, 1);
        let next_ptr = builder.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: ptr_param,
                r: one,
                ty: raw_ty,
            },
            raw_ty,
            O,
        );
        let item_ref = builder.op(
            body,
            OpKind::BorrowView {
                root: Root::Value(ptr_param),
                path: vec![Proj::Deref],
                mutable: false,
            },
            ref_ty,
            O,
        );
        let elem = builder.op(body, OpKind::Deref(item_ref), elem_ty, O);
        let elem_u = if elem_ty == U8 {
            builder.op(
                body,
                OpKind::Cast {
                    kind: CastKind::IntToInt,
                    v: elem,
                    from: U8,
                    to: USIZE,
                },
                USIZE,
                O,
            )
        } else {
            elem
        };
        let mut next_sum = builder.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: sum_param,
                r: elem_u,
                ty: USIZE,
            },
            USIZE,
            O,
        );
        let back_args = if let Some(idx_p) = idx_param {
            next_sum = builder.op(
                body,
                OpKind::Binary {
                    op: BinaryOp::Add,
                    l: next_sum,
                    r: idx_p,
                    ty: USIZE,
                },
                USIZE,
                O,
            );
            let idx_one = constant(&mut builder, body, 1);
            let next_idx = builder.op(
                body,
                OpKind::Binary {
                    op: BinaryOp::Add,
                    l: idx_p,
                    r: idx_one,
                    ty: USIZE,
                },
                USIZE,
                O,
            );
            let ovf = builder.op(
                body,
                OpKind::Binary {
                    op: BinaryOp::Lt,
                    l: next_idx,
                    r: idx_p,
                    ty: USIZE,
                },
                BOOL,
                O,
            );
            let checked = builder.op(body, OpKind::Copy(ovf), BOOL, O);
            builder.effect(
                body,
                OpKind::Assert {
                    cond: checked,
                    expected: false,
                    msg: RAssertMessage::Overflow {
                        op: BinaryOp::Add,
                        left: idx_p,
                        right: idx_one,
                    },
                    track_caller: None,
                },
                O,
            );
            vec![next_sum, next_ptr, next_idx]
        } else {
            vec![next_sum, next_ptr]
        };
        builder.terminate(body, RTerm::Goto(header, back_args));
        builder.terminate(exit, RTerm::Return(Some(sum_param)));
        (builder.finish(), table())
    }

    #[test]
    fn scalarized_plain_slice_iter_u8_lowers_to_smi_i32_cursor() {
        let (mut func, table) = build_scalarized_slice_loop(U8, REF_SLICE, RAW_U8, REF_U8, false);
        let loops = run(&mut func, &table);
        assert_eq!(loops.len(), 1);
        let header_idx = loops[0].header.0 as usize;
        let expected_cursor = loops[0].cursor;
        let expected_adapter = loops[0].adapter;
        let expected_cursor_ty = loops[0].cursor_ty;
        let expected_byte_ty = loops[0].byte_ty;
        assert_eq!(expected_adapter, IteratorIdentity::Slice);
        assert_eq!(expected_cursor_ty, ISIZE);
        assert_eq!(expected_byte_ty, U8);
        assert_eq!(
            func.blocks[header_idx].params.as_slice(),
            &[func.blocks[header_idx].params[0], expected_cursor]
        );
        crate::opt::dce::run(&mut func, &table);
        func.verify(&table).expect("Func::verify");
    }

    #[test]
    fn scalarized_plain_slice_iter_u32_lowers_to_smi_i32_cursor() {
        let (mut func, table) =
            build_scalarized_slice_loop(U32, REF_SLICE_U32, RAW_U32, REF_U32, false);
        let loops = run(&mut func, &table);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].adapter, IteratorIdentity::Slice);
        assert_eq!(loops[0].cursor_ty, ISIZE);
        assert_eq!(loops[0].byte_ty, U32);
        crate::opt::dce::run(&mut func, &table);
        func.verify(&table).expect("Func::verify");
    }

    #[test]
    fn scalarized_enumerate_slice_iter_u8_lowers_to_smi_i32_cursor() {
        let (mut func, table) = build_scalarized_slice_loop(U8, REF_SLICE, RAW_U8, REF_U8, true);
        let loops = run(&mut func, &table);
        assert_eq!(loops.len(), 1);
        let loop_ = &loops[0];
        assert_eq!(loop_.adapter, IteratorIdentity::Enumerate);
        assert_eq!(loop_.cursor_ty, ISIZE);
        assert!(matches!(
            func.blocks[loop_.direct.0 as usize].ops[0].kind,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v,
                from: ISIZE,
                to: USIZE,
            } if v == loop_.body_cursor
        ));
        crate::opt::dce::run(&mut func, &table);
        func.verify(&table).expect("Func::verify");
    }

    #[test]
    fn scalarized_slice_loop_using_ptr_param_inside_body_is_declined() {
        let mut builder = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 100,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 1,
                    ..AbiSignature::default()
                },
                arguments: vec![REF_SLICE],
                result: USIZE,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let header = builder.block();
        let body = builder.block();
        let exit = builder.block();

        let source = builder.param(REF_SLICE, O);
        let length = builder.op(
            entry,
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v: source,
                ty: REF_SLICE,
            },
            USIZE,
            O,
        );
        let raw = builder.op(
            entry,
            OpKind::RawView {
                root: Root::Value(source),
                path: vec![Proj::Deref],
                mutable: false,
            },
            RAW_U8,
            O,
        );
        let end = builder.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: raw,
                r: length,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let sum_init = constant(&mut builder, entry, 0);
        builder.terminate(entry, RTerm::Goto(header, vec![sum_init, raw]));

        let sum_param = builder.block_param(header, USIZE, O);
        let ptr_param = builder.block_param(header, RAW_U8, O);
        let eq = builder.op(
            header,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: ptr_param,
                r: end,
                ty: RAW_U8,
            },
            BOOL,
            O,
        );
        builder.terminate(
            header,
            RTerm::Switch {
                scrut: eq,
                scrut_ty: BOOL,
                cases: vec![(0, body, Vec::new())],
                default: (exit, Vec::new()),
            },
        );

        let one = constant(&mut builder, body, 1);
        let next_ptr = builder.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: ptr_param,
                r: one,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let item_ref = builder.op(
            body,
            OpKind::BorrowView {
                root: Root::Value(ptr_param),
                path: vec![Proj::Deref],
                mutable: false,
            },
            REF_U8,
            O,
        );
        let elem = builder.op(body, OpKind::Deref(item_ref), U8, O);
        let elem_u = builder.op(
            body,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: elem,
                from: U8,
                to: USIZE,
            },
            USIZE,
            O,
        );
        // Extra read of `ptr_param` inside `body` (`ptr_param == end`).
        let extra_cmp = builder.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: ptr_param,
                r: end,
                ty: RAW_U8,
            },
            BOOL,
            O,
        );
        builder.effect(
            body,
            OpKind::Assert {
                cond: extra_cmp,
                expected: false,
                msg: crate::jir::repr::RAssertMessage::MisalignedPointer {
                    required: ptr_param,
                    found: end,
                },
                track_caller: None,
            },
            O,
        );
        let next_sum = builder.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: sum_param,
                r: elem_u,
                ty: USIZE,
            },
            USIZE,
            O,
        );
        builder.terminate(body, RTerm::Goto(header, vec![next_sum, next_ptr]));
        builder.terminate(exit, RTerm::Return(Some(sum_param)));

        let mut func = builder.finish();
        let tbl = table();
        func.verify(&tbl).expect("pre-pass Func::verify");
        let loops = run(&mut func, &tbl);
        assert!(loops.is_empty());
        func.verify(&tbl).expect("declined Func::verify");
    }

    #[test]
    fn scalarized_slice_driver_bypass_pass_item_ref_clears_driver_term() {
        let mut builder = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 101,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 1,
                    ..AbiSignature::default()
                },
                arguments: vec![REF_SLICE],
                result: USIZE,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let header = builder.block();
        let driver = builder.block();
        let body = builder.block();
        let exit = builder.block();

        let source = builder.param(REF_SLICE, O);
        let length = builder.op(
            entry,
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v: source,
                ty: REF_SLICE,
            },
            USIZE,
            O,
        );
        let raw = builder.op(
            entry,
            OpKind::RawView {
                root: Root::Value(source),
                path: vec![Proj::Deref],
                mutable: false,
            },
            RAW_U8,
            O,
        );
        let end = builder.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: raw,
                r: length,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let sum_init = constant(&mut builder, entry, 0);
        builder.terminate(entry, RTerm::Goto(header, vec![sum_init, raw]));

        let sum_param = builder.block_param(header, USIZE, O);
        let ptr_param = builder.block_param(header, RAW_U8, O);
        let eq = builder.op(
            header,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: ptr_param,
                r: end,
                ty: RAW_U8,
            },
            BOOL,
            O,
        );
        builder.terminate(
            header,
            RTerm::Switch {
                scrut: eq,
                scrut_ty: BOOL,
                cases: vec![(0, driver, Vec::new())],
                default: (exit, Vec::new()),
            },
        );

        let one = constant(&mut builder, driver, 1);
        let next_ptr = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: ptr_param,
                r: one,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let item_ref = builder.op(
            driver,
            OpKind::BorrowView {
                root: Root::Value(ptr_param),
                path: vec![Proj::Deref],
                mutable: false,
            },
            REF_U8,
            O,
        );
        builder.terminate(driver, RTerm::Goto(body, vec![item_ref]));

        let item_param = builder.block_param(body, REF_U8, O);
        let elem = builder.op(body, OpKind::Deref(item_param), U8, O);
        let elem_u = builder.op(
            body,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: elem,
                from: U8,
                to: USIZE,
            },
            USIZE,
            O,
        );
        let next_sum = builder.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: sum_param,
                r: elem_u,
                ty: USIZE,
            },
            USIZE,
            O,
        );
        builder.terminate(body, RTerm::Goto(header, vec![next_sum, next_ptr]));
        builder.terminate(exit, RTerm::Return(Some(sum_param)));

        let mut func = builder.finish();
        let tbl = table();
        func.verify(&tbl).expect("pre-pass Func::verify");
        let loops = run(&mut func, &tbl);
        assert_eq!(loops.len(), 1);
        assert_eq!(func.blocks[driver.0 as usize].term, RTerm::Unreachable);
        crate::opt::dce::run_with(
            &mut func,
            &tbl,
            &crate::opt::dce::Options {
                no_deletions: true,
                ..Default::default()
            },
            &mut Vec::new(),
        );
        func.verify(&tbl).expect("post-dce Func::verify");
    }

    #[test]
    fn scalarized_slice_two_loops_sharing_raw_end_compact_under_no_deletions() {
        let mut builder = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 102,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 1,
                    ..AbiSignature::default()
                },
                arguments: vec![REF_SLICE],
                result: USIZE,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let header1 = builder.block();
        let body1 = builder.block();
        let between = builder.block();
        let header2 = builder.block();
        let body2 = builder.block();
        let exit = builder.block();

        let source = builder.param(REF_SLICE, O);
        let length = builder.op(
            entry,
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v: source,
                ty: REF_SLICE,
            },
            USIZE,
            O,
        );
        let raw = builder.op(
            entry,
            OpKind::RawView {
                root: Root::Value(source),
                path: vec![Proj::Deref],
                mutable: false,
            },
            RAW_U8,
            O,
        );
        let end = builder.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: raw,
                r: length,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let init1 = constant(&mut builder, entry, 0);
        builder.terminate(entry, RTerm::Goto(header1, vec![init1, raw]));

        let sum1 = builder.block_param(header1, USIZE, O);
        let ptr1 = builder.block_param(header1, RAW_U8, O);
        let eq1 = builder.op(
            header1,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: ptr1,
                r: end,
                ty: RAW_U8,
            },
            BOOL,
            O,
        );
        builder.terminate(
            header1,
            RTerm::Switch {
                scrut: eq1,
                scrut_ty: BOOL,
                cases: vec![(0, body1, Vec::new())],
                default: (between, Vec::new()),
            },
        );
        let one1 = constant(&mut builder, body1, 1);
        let next_ptr1 = builder.op(
            body1,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: ptr1,
                r: one1,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let ref1 = builder.op(
            body1,
            OpKind::BorrowView {
                root: Root::Value(ptr1),
                path: vec![Proj::Deref],
                mutable: false,
            },
            REF_U8,
            O,
        );
        let elem1 = builder.op(body1, OpKind::Deref(ref1), U8, O);
        let elem1_u = builder.op(
            body1,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: elem1,
                from: U8,
                to: USIZE,
            },
            USIZE,
            O,
        );
        let next_sum1 = builder.op(
            body1,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: sum1,
                r: elem1_u,
                ty: USIZE,
            },
            USIZE,
            O,
        );
        builder.terminate(body1, RTerm::Goto(header1, vec![next_sum1, next_ptr1]));

        builder.terminate(between, RTerm::Goto(header2, vec![sum1, raw]));

        let sum2 = builder.block_param(header2, USIZE, O);
        let ptr2 = builder.block_param(header2, RAW_U8, O);
        let eq2 = builder.op(
            header2,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: ptr2,
                r: end,
                ty: RAW_U8,
            },
            BOOL,
            O,
        );
        builder.terminate(
            header2,
            RTerm::Switch {
                scrut: eq2,
                scrut_ty: BOOL,
                cases: vec![(0, body2, Vec::new())],
                default: (exit, Vec::new()),
            },
        );
        let one2 = constant(&mut builder, body2, 1);
        let next_ptr2 = builder.op(
            body2,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: ptr2,
                r: one2,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let ref2 = builder.op(
            body2,
            OpKind::BorrowView {
                root: Root::Value(ptr2),
                path: vec![Proj::Deref],
                mutable: false,
            },
            REF_U8,
            O,
        );
        let elem2 = builder.op(body2, OpKind::Deref(ref2), U8, O);
        let elem2_u = builder.op(
            body2,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: elem2,
                from: U8,
                to: USIZE,
            },
            USIZE,
            O,
        );
        let next_sum2 = builder.op(
            body2,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: sum2,
                r: elem2_u,
                ty: USIZE,
            },
            USIZE,
            O,
        );
        builder.terminate(body2, RTerm::Goto(header2, vec![next_sum2, next_ptr2]));
        builder.terminate(exit, RTerm::Return(Some(sum2)));

        let mut func = builder.finish();
        let tbl = table();
        func.verify(&tbl).expect("pre-pass Func::verify");
        let loops = run(&mut func, &tbl);
        assert_eq!(loops.len(), 2);
        crate::opt::dce::run_with(
            &mut func,
            &tbl,
            &crate::opt::dce::Options {
                no_deletions: true,
                ..Default::default()
            },
            &mut Vec::new(),
        );
        func.verify(&tbl).expect("post-dce Func::verify");
    }

    #[test]
    fn scalarized_loop_with_raw_parts_header_copies_and_multi_field_nonnull_lowers() {
        let mut builder = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 103,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 2,
                    ..AbiSignature::default()
                },
                arguments: vec![RAW_U8, USIZE],
                result: USIZE,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let header = builder.block();
        let driver = builder.block();
        let body = builder.block();
        let exit = builder.block();

        let buf_ptr = builder.param(RAW_U8, O);
        let slice_len = builder.param(USIZE, O);
        let source = builder.op(
            entry,
            OpKind::Call {
                callee: rcgjs_object::jir::Callee::Runtime {
                    member: StrId(0),
                    sysroot: true,
                },
                function: None,
                args: vec![buf_ptr, slice_len],
                track_caller: None,
                semantic: CallSemantic::RawParts,
                host: None,
            },
            REF_SLICE,
            O,
        );
        let raw = builder.op(
            entry,
            OpKind::RawView {
                root: Root::Value(source),
                path: vec![Proj::Deref],
                mutable: false,
            },
            RAW_U8,
            O,
        );
        let zst_entry = builder.op(entry, OpKind::Const(ConstTree::Unit), ZST, O);
        let init_nn = builder.op(
            entry,
            OpKind::Aggregate {
                ty: NON_NULL_U8,
                variant: 0,
                active_field: None,
                fields: vec![zst_entry, raw],
            },
            NON_NULL_U8,
            O,
        );
        let end = builder.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: raw,
                r: slice_len,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let sum_init = constant(&mut builder, entry, 0);
        builder.terminate(entry, RTerm::Goto(header, vec![sum_init, init_nn]));

        let sum_param = builder.block_param(header, USIZE, O);
        let nn_param = builder.block_param(header, NON_NULL_U8, O);
        let k = builder.op(
            header,
            OpKind::Field {
                base: nn_param,
                variant: None,
                field: 1,
            },
            RAW_U8,
            O,
        );
        let e = builder.op(header, OpKind::Copy(k), RAW_U8, O);
        let eq = builder.op(
            header,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: e,
                r: end,
                ty: RAW_U8,
            },
            BOOL,
            O,
        );
        builder.terminate(
            header,
            RTerm::Switch {
                scrut: eq,
                scrut_ty: BOOL,
                cases: vec![(0, driver, Vec::new())],
                default: (exit, Vec::new()),
            },
        );

        let one = constant(&mut builder, driver, 1);
        let next_ptr = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: e,
                r: one,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let zst_driver = builder.op(driver, OpKind::Const(ConstTree::Unit), ZST, O);
        let next_nn = builder.op(
            driver,
            OpKind::Aggregate {
                ty: NON_NULL_U8,
                variant: 0,
                active_field: None,
                fields: vec![zst_driver, next_ptr],
            },
            NON_NULL_U8,
            O,
        );
        let item_ref = builder.op(
            driver,
            OpKind::BorrowView {
                root: Root::Value(k),
                path: vec![Proj::Deref],
                mutable: false,
            },
            REF_U8,
            O,
        );
        builder.terminate(driver, RTerm::Goto(body, vec![item_ref]));

        let item_param = builder.block_param(body, REF_U8, O);
        let elem = builder.op(body, OpKind::Deref(item_param), U8, O);
        let elem_u = builder.op(
            body,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: elem,
                from: U8,
                to: USIZE,
            },
            USIZE,
            O,
        );
        let next_sum = builder.op(
            body,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: sum_param,
                r: elem_u,
                ty: USIZE,
            },
            USIZE,
            O,
        );
        builder.terminate(body, RTerm::Goto(header, vec![next_sum, next_nn]));
        builder.terminate(exit, RTerm::Return(Some(sum_param)));

        let mut func = builder.finish();
        let tbl = table();
        func.verify(&tbl).expect("pre-pass Func::verify");
        let loops = run(&mut func, &tbl);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].cursor_ty, ISIZE);
        assert_eq!(func.blocks[driver.0 as usize].term, RTerm::Unreachable);
        crate::opt::dce::run(&mut func, &tbl);
        func.verify(&tbl).expect("post-dce Func::verify");
    }

    #[test]
    fn scalarized_loop_with_invariant_end_param_and_slice_view_source_lowers() {
        let mut builder = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 104,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 2,
                    ..AbiSignature::default()
                },
                arguments: vec![RAW_U8, USIZE],
                result: USIZE,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let ptr_arg = builder.param(RAW_U8, O);
        let len_arg = builder.param(USIZE, O);
        let header = builder.block();
        let driver = builder.block();
        let exit = builder.block();

        let slice = builder.op(
            entry,
            OpKind::SliceView {
                ptr: ptr_arg,
                len: len_arg,
            },
            REF_SLICE,
            O,
        );
        // Simulate rustc's direct PtrToPtr from SliceView (&[u8]) to *const u8 without RawView.
        let start_ptr = builder.op(
            entry,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: slice,
                from: REF_SLICE,
                to: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let end_ptr = builder.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: start_ptr,
                r: len_arg,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let zero = builder.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            USIZE,
            O,
        );
        builder.terminate(entry, RTerm::Goto(header, vec![zero, start_ptr, end_ptr]));

        let sum_param = builder.block_param(header, USIZE, O);
        let cur_param = builder.block_param(header, RAW_U8, O);
        let end_param = builder.block_param(header, RAW_U8, O);
        let cur_copy = builder.op(header, OpKind::Copy(cur_param), RAW_U8, O);
        let end_copy = builder.op(header, OpKind::Copy(end_param), RAW_U8, O);
        let done = builder.op(
            header,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: cur_copy,
                r: end_copy,
                ty: RAW_U8,
            },
            BOOL,
            O,
        );
        builder.terminate(
            header,
            RTerm::Switch {
                scrut: done,
                scrut_ty: BOOL,
                cases: vec![(0, driver, Vec::new())],
                default: (exit, Vec::new()),
            },
        );

        let one = builder.op(
            driver,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            USIZE,
            O,
        );
        let next_ptr = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: cur_copy,
                r: one,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let item_ref = builder.op(
            driver,
            OpKind::BorrowView {
                root: Root::Value(cur_param),
                path: vec![Proj::Deref],
                mutable: false,
            },
            REF_U8,
            O,
        );
        let item = builder.op(driver, OpKind::Deref(item_ref), U8, O);
        let item_u = builder.op(
            driver,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: item,
                from: U8,
                to: USIZE,
            },
            USIZE,
            O,
        );
        let next_sum = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: sum_param,
                r: item_u,
                ty: USIZE,
            },
            USIZE,
            O,
        );
        builder.terminate(
            driver,
            RTerm::Goto(header, vec![next_sum, next_ptr, end_param]),
        );
        builder.terminate(exit, RTerm::Return(Some(sum_param)));

        let mut func = builder.finish();
        let tbl = table();
        func.verify(&tbl).expect("pre-pass Func::verify");
        let loops = run(&mut func, &tbl);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].cursor_ty, ISIZE);
        assert_eq!(func.blocks[header.0 as usize].params.len(), 2);
        crate::opt::dce::run(&mut func, &tbl);
        func.verify(&tbl).expect("post-dce Func::verify");
    }

    fn base_scalarized_sig() -> Signature {
        Signature {
            abi: AbiSignature {
                parameters: 2,
                ..AbiSignature::default()
            },
            arguments: vec![RAW_U8, USIZE],
            result: USIZE,
        }
    }

    #[test]
    fn scalarized_truncated_int_cast_u8_compare_does_not_lower() {
        let mut builder = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 105,
                },
                instantiation: 0,
            },
            base_scalarized_sig(),
        );
        let entry = FuncBuilder::ENTRY;
        let ptr_arg = builder.param(RAW_U8, O);
        let len_arg = builder.param(USIZE, O);
        let header = builder.block();
        let driver = builder.block();
        let exit = builder.block();

        let slice = builder.op(
            entry,
            OpKind::SliceView {
                ptr: ptr_arg,
                len: len_arg,
            },
            REF_SLICE,
            O,
        );
        let raw = builder.op(
            entry,
            OpKind::RawView {
                root: Root::Value(slice),
                path: vec![Proj::Deref],
                mutable: false,
            },
            RAW_U8,
            O,
        );
        let end = builder.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: raw,
                r: len_arg,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let zero = constant(&mut builder, entry, 0);
        builder.terminate(entry, RTerm::Goto(header, vec![zero, raw]));

        let sum_param = builder.block_param(header, USIZE, O);
        let cur_param = builder.block_param(header, RAW_U8, O);
        let cur_addr = builder.op(
            header,
            OpKind::Cast {
                kind: CastKind::PointerExposeProvenance,
                v: cur_param,
                from: RAW_U8,
                to: USIZE,
            },
            USIZE,
            O,
        );
        let cur_u8 = builder.op(
            header,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: cur_addr,
                from: USIZE,
                to: U8,
            },
            U8,
            O,
        );
        let end_addr = builder.op(
            header,
            OpKind::Cast {
                kind: CastKind::PointerExposeProvenance,
                v: end,
                from: RAW_U8,
                to: USIZE,
            },
            USIZE,
            O,
        );
        let end_u8 = builder.op(
            header,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: end_addr,
                from: USIZE,
                to: U8,
            },
            U8,
            O,
        );
        let done = builder.op(
            header,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: cur_u8,
                r: end_u8,
                ty: U8,
            },
            BOOL,
            O,
        );
        builder.terminate(
            header,
            RTerm::Switch {
                scrut: done,
                scrut_ty: BOOL,
                cases: vec![(0, driver, Vec::new())],
                default: (exit, Vec::new()),
            },
        );

        let one = constant(&mut builder, driver, 1);
        let next_ptr = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: cur_param,
                r: one,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let item_ref = builder.op(
            driver,
            OpKind::BorrowView {
                root: Root::Value(cur_param),
                path: vec![Proj::Deref],
                mutable: false,
            },
            REF_U8,
            O,
        );
        let item = builder.op(driver, OpKind::Deref(item_ref), U8, O);
        let item_u = builder.op(
            driver,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: item,
                from: U8,
                to: USIZE,
            },
            USIZE,
            O,
        );
        let next_sum = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: sum_param,
                r: item_u,
                ty: USIZE,
            },
            USIZE,
            O,
        );
        builder.terminate(driver, RTerm::Goto(header, vec![next_sum, next_ptr]));
        builder.terminate(exit, RTerm::Return(Some(sum_param)));

        let mut func = builder.finish();
        let tbl = table();
        func.verify(&tbl).expect("pre-pass Func::verify");
        assert!(run(&mut func, &tbl).is_empty());
    }

    #[test]
    fn scalarized_wrong_stride_zst_ptr_offset_does_not_lower() {
        const RAW_ZST: TypeId = TypeId(23);
        let mut tbl = table();
        tbl.insert(scalar(
            RAW_ZST,
            TypeKind::RawPointer {
                pointee: ZST,
                mutable: false,
            },
            None,
        ));

        let mut builder = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 106,
                },
                instantiation: 0,
            },
            base_scalarized_sig(),
        );
        let entry = FuncBuilder::ENTRY;
        let ptr_arg = builder.param(RAW_U8, O);
        let len_arg = builder.param(USIZE, O);
        let header = builder.block();
        let driver = builder.block();
        let exit = builder.block();

        let slice = builder.op(
            entry,
            OpKind::SliceView {
                ptr: ptr_arg,
                len: len_arg,
            },
            REF_SLICE,
            O,
        );
        let raw_u8 = builder.op(
            entry,
            OpKind::RawView {
                root: Root::Value(slice),
                path: vec![Proj::Deref],
                mutable: false,
            },
            RAW_U8,
            O,
        );
        let start_u = builder.op(
            entry,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: raw_u8,
                from: RAW_U8,
                to: RAW_ZST,
            },
            RAW_ZST,
            O,
        );
        let end_u = builder.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: start_u,
                r: len_arg,
                ty: RAW_ZST,
            },
            RAW_ZST,
            O,
        );
        let zero = constant(&mut builder, entry, 0);
        builder.terminate(entry, RTerm::Goto(header, vec![zero, start_u]));

        let sum_param = builder.block_param(header, USIZE, O);
        let cur_param = builder.block_param(header, RAW_ZST, O);
        let done = builder.op(
            header,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: cur_param,
                r: end_u,
                ty: RAW_ZST,
            },
            BOOL,
            O,
        );
        builder.terminate(
            header,
            RTerm::Switch {
                scrut: done,
                scrut_ty: BOOL,
                cases: vec![(0, driver, Vec::new())],
                default: (exit, Vec::new()),
            },
        );

        let one = constant(&mut builder, driver, 1);
        let next_u = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: cur_param,
                r: one,
                ty: RAW_ZST,
            },
            RAW_ZST,
            O,
        );
        let cur_u8 = builder.op(
            driver,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: cur_param,
                from: RAW_ZST,
                to: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let item_ref = builder.op(
            driver,
            OpKind::BorrowView {
                root: Root::Value(cur_u8),
                path: vec![Proj::Deref],
                mutable: false,
            },
            REF_U8,
            O,
        );
        let item = builder.op(driver, OpKind::Deref(item_ref), U8, O);
        let item_u = builder.op(
            driver,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: item,
                from: U8,
                to: USIZE,
            },
            USIZE,
            O,
        );
        let next_sum = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: sum_param,
                r: item_u,
                ty: USIZE,
            },
            USIZE,
            O,
        );
        builder.terminate(driver, RTerm::Goto(header, vec![next_sum, next_u]));
        builder.terminate(exit, RTerm::Return(Some(sum_param)));

        let mut func = builder.finish();
        func.verify(&tbl).expect("pre-pass Func::verify");
        assert!(run(&mut func, &tbl).is_empty());
    }

    #[test]
    fn transparent_unsized_bytes_wrapper_does_not_lower() {
        const BYTES: TypeId = TypeId(23);
        const REF_BYTES: TypeId = TypeId(24);
        const RAW_BYTES: TypeId = TypeId(25);
        let mut tbl = table();
        let mut bytes_desc = aggregate_type(
            BYTES,
            Vec::new(),
            vec![field(SLICE, 0)],
            IteratorIdentity::None,
        );
        bytes_desc.layout.sized = false;
        bytes_desc.js.peeled = SLICE;
        tbl.insert(bytes_desc);
        let mut ref_bytes = scalar(
            REF_BYTES,
            TypeKind::Reference {
                pointee: BYTES,
                mutable: false,
            },
            None,
        );
        ref_bytes.js.peeled = REF_SLICE;
        tbl.insert(ref_bytes);
        tbl.insert(scalar(
            RAW_BYTES,
            TypeKind::RawPointer {
                pointee: BYTES,
                mutable: false,
            },
            None,
        ));

        let mut builder = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 107,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 2,
                    ..AbiSignature::default()
                },
                arguments: vec![REF_BYTES, USIZE],
                result: USIZE,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let bytes_ref = builder.param(REF_BYTES, O);
        let len_arg = builder.param(USIZE, O);
        let header = builder.block();
        let driver = builder.block();
        let exit = builder.block();

        let raw_bytes = builder.op(
            entry,
            OpKind::RawView {
                root: Root::Value(bytes_ref),
                path: vec![Proj::Deref],
                mutable: false,
            },
            RAW_BYTES,
            O,
        );
        let raw_u8 = builder.op(
            entry,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: raw_bytes,
                from: RAW_BYTES,
                to: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let end = builder.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: raw_u8,
                r: len_arg,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let zero = constant(&mut builder, entry, 0);
        builder.terminate(entry, RTerm::Goto(header, vec![zero, raw_u8]));

        let sum_param = builder.block_param(header, USIZE, O);
        let cur_param = builder.block_param(header, RAW_U8, O);
        let done = builder.op(
            header,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: cur_param,
                r: end,
                ty: RAW_U8,
            },
            BOOL,
            O,
        );
        builder.terminate(
            header,
            RTerm::Switch {
                scrut: done,
                scrut_ty: BOOL,
                cases: vec![(0, driver, Vec::new())],
                default: (exit, Vec::new()),
            },
        );

        let one = constant(&mut builder, driver, 1);
        let next_ptr = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: cur_param,
                r: one,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let item_ref = builder.op(
            driver,
            OpKind::BorrowView {
                root: Root::Value(cur_param),
                path: vec![Proj::Deref],
                mutable: false,
            },
            REF_U8,
            O,
        );
        let item = builder.op(driver, OpKind::Deref(item_ref), U8, O);
        let item_u = builder.op(
            driver,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: item,
                from: U8,
                to: USIZE,
            },
            USIZE,
            O,
        );
        let next_sum = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: sum_param,
                r: item_u,
                ty: USIZE,
            },
            USIZE,
            O,
        );
        builder.terminate(driver, RTerm::Goto(header, vec![next_sum, next_ptr]));
        builder.terminate(exit, RTerm::Return(Some(sum_param)));

        let mut func = builder.finish();
        func.verify(&tbl).expect("pre-pass Func::verify");
        assert!(run(&mut func, &tbl).is_empty());
    }

    #[test]
    fn direct_call_with_raw_parts_semantic_does_not_match_source_length() {
        let mut builder = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 108,
                },
                instantiation: 0,
            },
            base_scalarized_sig(),
        );
        let entry = FuncBuilder::ENTRY;
        let ptr_arg = builder.param(RAW_U8, O);
        let len_arg = builder.param(USIZE, O);
        let header = builder.block();
        let driver = builder.block();
        let exit = builder.block();

        let source = builder.op(
            entry,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 1,
                            local_hash: 777,
                        },
                        instantiation: 0,
                    },
                    link_name: StrId(1),
                },
                function: None,
                args: vec![ptr_arg, len_arg],
                track_caller: None,
                semantic: CallSemantic::RawParts,
                host: None,
            },
            REF_SLICE,
            O,
        );
        let raw = builder.op(
            entry,
            OpKind::RawView {
                root: Root::Value(source),
                path: vec![Proj::Deref],
                mutable: false,
            },
            RAW_U8,
            O,
        );
        let end = builder.op(
            entry,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: raw,
                r: len_arg,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let zero = constant(&mut builder, entry, 0);
        builder.terminate(entry, RTerm::Goto(header, vec![zero, raw]));

        let sum_param = builder.block_param(header, USIZE, O);
        let cur_param = builder.block_param(header, RAW_U8, O);
        let done = builder.op(
            header,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: cur_param,
                r: end,
                ty: RAW_U8,
            },
            BOOL,
            O,
        );
        builder.terminate(
            header,
            RTerm::Switch {
                scrut: done,
                scrut_ty: BOOL,
                cases: vec![(0, driver, Vec::new())],
                default: (exit, Vec::new()),
            },
        );

        let one = constant(&mut builder, driver, 1);
        let next_ptr = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: cur_param,
                r: one,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let item_ref = builder.op(
            driver,
            OpKind::BorrowView {
                root: Root::Value(cur_param),
                path: vec![Proj::Deref],
                mutable: false,
            },
            REF_U8,
            O,
        );
        let item = builder.op(driver, OpKind::Deref(item_ref), U8, O);
        let item_u = builder.op(
            driver,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: item,
                from: U8,
                to: USIZE,
            },
            USIZE,
            O,
        );
        let next_sum = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: sum_param,
                r: item_u,
                ty: USIZE,
            },
            USIZE,
            O,
        );
        builder.terminate(driver, RTerm::Goto(header, vec![next_sum, next_ptr]));
        builder.terminate(exit, RTerm::Return(Some(sum_param)));

        let mut func = builder.finish();
        let tbl = table();
        func.verify(&tbl).expect("pre-pass Func::verify");
        assert!(run(&mut func, &tbl).is_empty());
    }

    #[test]
    fn scalarized_spliced_iter_new_single_edge_lowers() {
        let tbl = table();
        let mut builder = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 109,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 2,
                    ..AbiSignature::default()
                },
                arguments: vec![RAW_U8, USIZE],
                result: USIZE,
            },
        );
        let b_new = FuncBuilder::ENTRY;
        let ptr_arg = builder.param(RAW_U8, O);
        let len_arg = builder.param(USIZE, O);
        let b_after_new = builder.block();
        let start_param_after = builder.block_param(b_after_new, RAW_U8, O);
        let end_param_after = builder.block_param(b_after_new, RAW_U8, O);
        let preheader = builder.block();
        let header = builder.block();
        let sum_param = builder.block_param(header, USIZE, O);
        let cur_param = builder.block_param(header, RAW_U8, O);
        let driver = builder.block();
        let exit = builder.block();

        // Spliced `Iter::new` computes `raw`, `end = Offset(raw, len)`, wraps `PtrToPtr`,
        // and passes them via a single-edge continuation to `b_after_new`.
        let slice = builder.op(
            b_new,
            OpKind::SliceView {
                ptr: ptr_arg,
                len: len_arg,
            },
            REF_SLICE,
            O,
        );
        let start_raw = builder.op(
            b_new,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: slice,
                from: REF_SLICE,
                to: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let end_raw = builder.op(
            b_new,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: start_raw,
                r: len_arg,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let end_cast = builder.op(
            b_new,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: end_raw,
                from: RAW_U8,
                to: RAW_U8,
            },
            RAW_U8,
            O,
        );
        builder.terminate(b_new, RTerm::Goto(b_after_new, vec![start_raw, end_cast]));

        // `b_after_new` forwards via `Goto(preheader, [])`.
        builder.terminate(b_after_new, RTerm::Goto(preheader, Vec::new()));

        let zero = constant(&mut builder, preheader, 0);
        builder.terminate(
            preheader,
            RTerm::Goto(header, vec![zero, start_param_after]),
        );

        let done = builder.op(
            header,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: cur_param,
                r: end_param_after,
                ty: RAW_U8,
            },
            BOOL,
            O,
        );
        builder.terminate(
            header,
            RTerm::Switch {
                scrut: done,
                scrut_ty: BOOL,
                cases: vec![(0, driver, Vec::new())],
                default: (exit, Vec::new()),
            },
        );

        let one = constant(&mut builder, driver, 1);
        let next_ptr = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: cur_param,
                r: one,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let item_ref = builder.op(
            driver,
            OpKind::BorrowView {
                root: Root::Value(cur_param),
                path: vec![Proj::Deref],
                mutable: false,
            },
            REF_U8,
            O,
        );
        let item = builder.op(driver, OpKind::Deref(item_ref), U8, O);
        let item_u = builder.op(
            driver,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: item,
                from: U8,
                to: USIZE,
            },
            USIZE,
            O,
        );
        let next_sum = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: sum_param,
                r: item_u,
                ty: USIZE,
            },
            USIZE,
            O,
        );
        builder.terminate(driver, RTerm::Goto(header, vec![next_sum, next_ptr]));
        builder.terminate(exit, RTerm::Return(Some(sum_param)));

        let mut func = builder.finish();
        func.verify(&tbl).expect("pre-pass Func::verify");
        let loops = run(&mut func, &tbl);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].cursor_ty, ISIZE);
        crate::opt::dce::run(&mut func, &tbl);
        func.verify(&tbl).expect("post-dce Func::verify");
    }

    #[derive(Clone, Copy)]
    enum ParamSourceKind {
        RefSlice,
        MutRefSlice,
        StrCast,
        RawPartsSysrootTrue,
        RawPartsSysrootFalse,
        RawPartsInsideLoop,
    }

    fn run_scalarized_source_case(
        kind: ParamSourceKind,
    ) -> (Func, TypeTable, Vec<SliceIteratorLoop>) {
        use crate::jir::semantic::{CallSemantic, Callee};

        const STR: TypeId = TypeId(23);
        const REF_STR: TypeId = TypeId(24);
        const MUT_REF_SLICE: TypeId = TypeId(25);

        let mut tbl = table();
        tbl.insert(scalar(STR, TypeKind::Str, None));
        tbl.insert(scalar(
            REF_STR,
            TypeKind::Reference {
                pointee: STR,
                mutable: false,
            },
            None,
        ));
        tbl.insert(scalar(
            MUT_REF_SLICE,
            TypeKind::Reference {
                pointee: SLICE,
                mutable: true,
            },
            None,
        ));

        let (arg_tys, param_count) = match kind {
            ParamSourceKind::RefSlice => (vec![REF_SLICE], 1),
            ParamSourceKind::MutRefSlice => (vec![MUT_REF_SLICE], 1),
            ParamSourceKind::StrCast => (vec![REF_STR], 1),
            ParamSourceKind::RawPartsSysrootTrue
            | ParamSourceKind::RawPartsSysrootFalse
            | ParamSourceKind::RawPartsInsideLoop => (vec![RAW_U8, USIZE], 2),
        };

        let mut builder = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 320,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: param_count,
                    ..AbiSignature::default()
                },
                arguments: arg_tys.clone(),
                result: USIZE,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let header = builder.block();
        let sum_param = builder.block_param(header, USIZE, O);
        let cur_param = builder.block_param(header, RAW_U8, O);
        let driver = builder.block();
        let exit = builder.block();

        let (raw_pre, end_pre, raw_ptr_opt, raw_len_opt) = match kind {
            ParamSourceKind::RefSlice | ParamSourceKind::MutRefSlice => {
                let slice = builder.param(arg_tys[0], O);
                let len = builder.op(
                    entry,
                    OpKind::Unary {
                        op: UnaryOp::PtrMetadata,
                        v: slice,
                        ty: arg_tys[0],
                    },
                    USIZE,
                    O,
                );
                let start_raw = builder.op(
                    entry,
                    OpKind::Cast {
                        kind: CastKind::PtrToPtr,
                        v: slice,
                        from: arg_tys[0],
                        to: RAW_U8,
                    },
                    RAW_U8,
                    O,
                );
                let end_raw = builder.op(
                    entry,
                    OpKind::Binary {
                        op: BinaryOp::Offset,
                        l: start_raw,
                        r: len,
                        ty: RAW_U8,
                    },
                    RAW_U8,
                    O,
                );
                (start_raw, end_raw, None, None)
            }
            ParamSourceKind::StrCast => {
                let s = builder.param(REF_STR, O);
                let len = builder.op(
                    entry,
                    OpKind::Unary {
                        op: UnaryOp::PtrMetadata,
                        v: s,
                        ty: REF_STR,
                    },
                    USIZE,
                    O,
                );
                let slice = builder.op(
                    entry,
                    OpKind::Cast {
                        kind: CastKind::Transmute,
                        v: s,
                        from: REF_STR,
                        to: REF_SLICE,
                    },
                    REF_SLICE,
                    O,
                );
                let start_raw = builder.op(
                    entry,
                    OpKind::Cast {
                        kind: CastKind::PtrToPtr,
                        v: slice,
                        from: REF_SLICE,
                        to: RAW_U8,
                    },
                    RAW_U8,
                    O,
                );
                let end_raw = builder.op(
                    entry,
                    OpKind::Binary {
                        op: BinaryOp::Offset,
                        l: start_raw,
                        r: len,
                        ty: RAW_U8,
                    },
                    RAW_U8,
                    O,
                );
                (start_raw, end_raw, None, None)
            }
            ParamSourceKind::RawPartsSysrootTrue | ParamSourceKind::RawPartsSysrootFalse => {
                let ptr_arg = builder.param(RAW_U8, O);
                let len_arg = builder.param(USIZE, O);
                let slice = builder.op(
                    entry,
                    OpKind::Call {
                        callee: Callee::Runtime {
                            member: StrId(0),
                            sysroot: matches!(kind, ParamSourceKind::RawPartsSysrootTrue),
                        },
                        function: None,
                        args: vec![ptr_arg, len_arg],
                        track_caller: None,
                        semantic: CallSemantic::RawParts,
                        host: None,
                    },
                    REF_SLICE,
                    O,
                );
                let start_raw = builder.op(
                    entry,
                    OpKind::Cast {
                        kind: CastKind::PtrToPtr,
                        v: slice,
                        from: REF_SLICE,
                        to: RAW_U8,
                    },
                    RAW_U8,
                    O,
                );
                let end_raw = builder.op(
                    entry,
                    OpKind::Binary {
                        op: BinaryOp::Offset,
                        l: start_raw,
                        r: len_arg,
                        ty: RAW_U8,
                    },
                    RAW_U8,
                    O,
                );
                (start_raw, end_raw, None, None)
            }
            ParamSourceKind::RawPartsInsideLoop => {
                let ptr_arg = builder.param(RAW_U8, O);
                let len_arg = builder.param(USIZE, O);
                let end_raw = builder.op(
                    entry,
                    OpKind::Binary {
                        op: BinaryOp::Offset,
                        l: ptr_arg,
                        r: len_arg,
                        ty: RAW_U8,
                    },
                    RAW_U8,
                    O,
                );
                (ptr_arg, end_raw, Some(ptr_arg), Some(len_arg))
            }
        };

        let zero = constant(&mut builder, entry, 0);
        builder.terminate(entry, RTerm::Goto(header, vec![zero, raw_pre]));

        if let (Some(p), Some(l)) = (raw_ptr_opt, raw_len_opt) {
            let _in_loop_src = builder.op(
                header,
                OpKind::Call {
                    callee: Callee::Runtime {
                        member: StrId(0),
                        sysroot: true,
                    },
                    function: None,
                    args: vec![p, l],
                    track_caller: None,
                    semantic: CallSemantic::RawParts,
                    host: None,
                },
                REF_SLICE,
                O,
            );
        }

        let done = builder.op(
            header,
            OpKind::Binary {
                op: BinaryOp::Eq,
                l: cur_param,
                r: end_pre,
                ty: RAW_U8,
            },
            BOOL,
            O,
        );
        builder.terminate(
            header,
            RTerm::Switch {
                scrut: done,
                scrut_ty: BOOL,
                cases: vec![(0, driver, Vec::new())],
                default: (exit, Vec::new()),
            },
        );

        let one = constant(&mut builder, driver, 1);
        let next_ptr = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: cur_param,
                r: one,
                ty: RAW_U8,
            },
            RAW_U8,
            O,
        );
        let item_ref = builder.op(
            driver,
            OpKind::BorrowView {
                root: Root::Value(cur_param),
                path: vec![Proj::Deref],
                mutable: false,
            },
            REF_U8,
            O,
        );
        let item = builder.op(driver, OpKind::Deref(item_ref), U8, O);
        let item_u = builder.op(
            driver,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: item,
                from: U8,
                to: USIZE,
            },
            USIZE,
            O,
        );
        let next_sum = builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: sum_param,
                r: item_u,
                ty: USIZE,
            },
            USIZE,
            O,
        );
        builder.terminate(driver, RTerm::Goto(header, vec![next_sum, next_ptr]));
        builder.terminate(exit, RTerm::Return(Some(sum_param)));

        let mut func = builder.finish();
        func.verify(&tbl).expect("pre-pass Func::verify");
        let loops = run(&mut func, &tbl);
        (func, tbl, loops)
    }

    #[test]
    fn scalarized_ref_slice_str_cast_and_raw_parts_sources_lower_and_reject_unsafe() {
        for (label, kind) in [
            ("ref_slice_param", ParamSourceKind::RefSlice),
            ("str_cast", ParamSourceKind::StrCast),
            (
                "raw_parts_sysroot_true",
                ParamSourceKind::RawPartsSysrootTrue,
            ),
        ] {
            let (mut func, tbl, loops) = run_scalarized_source_case(kind);
            assert_eq!(loops.len(), 1, "{label} must lower to scalarized loop");
            assert_eq!(loops[0].cursor_ty, ISIZE);
            crate::opt::dce::run(&mut func, &tbl);
            func.verify(&tbl).expect("post-dce Func::verify");
        }

        for (label, kind) in [
            ("mut_ref_slice", ParamSourceKind::MutRefSlice),
            (
                "raw_parts_sysroot_false",
                ParamSourceKind::RawPartsSysrootFalse,
            ),
            ("raw_parts_inside_loop", ParamSourceKind::RawPartsInsideLoop),
        ] {
            let (_func, _tbl, loops) = run_scalarized_source_case(kind);
            assert!(loops.is_empty(), "{label} must NOT lower");
        }
    }
}
