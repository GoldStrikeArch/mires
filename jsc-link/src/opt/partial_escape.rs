//! Partial escape analysis: the `sroa` pass on the JIR lane.
//!
//! An *object node* is one aggregate value the SSA builder made: the result of an `Aggregate`, of
//! a `Const` aggregate tree, of a `Copy` (a fresh node over the same field values) or a `Move` (the
//! same node), and every block parameter of a scalarizable type ([`super::candidates`]). A node
//! either **escapes** — some use needs a real JavaScript object — or it does not, and then it is
//! replaced by its fields and never built at all.
//!
//! # Whole-object escape, with one exception at merges
//!
//! The decision is per object, not per program point: one monotone fixpoint over "escapes", then
//! one rewrite. An escaping object is left exactly as the builder made it, and the output is
//! plain JIR-R — a scalarized aggregate simply disappears. The one exception (D11a) is the
//! merge: a virtual node passed as a block argument into a parameter that is real — escaped for
//! its own reasons, of another shape or variant, or not a node at all — no longer escapes with
//! it. The edge *materializes* the node instead: an `Aggregate` over its current field values,
//! built in the predecessor, a by-value snapshot exactly like the scalar edge arguments a
//! virtual merge already passes. General late materialization and re-virtualization stay a later
//! phase's.
//!
//! Everything the analysis does not recognise escapes. Correctness never depends on the escape
//! set: escaping is the baseline, so a rule that is too coarse costs size and nothing else.
//!
//! # What is tracked
//!
//! A node's field may hold *another* node — `Some((a, b))` is an `Option` node over a tuple node —
//! and then reading that field is an alias rather than a value. Nesting is deliberately shallow:
//! the nested node escapes as soon as its slot is written, borrowed, or crosses a block parameter,
//! so a slot that holds a node holds the same one everywhere.
//!
//! Beside the nodes themselves, a *tracked view* is a `BorrowView` rooted at a node with an empty
//! path (a reference to the whole object) or a one-`Field` path (a reference to one field, behind
//! the node's `Downcast` when it is an enum), and a
//! `BorrowView` through such a view (`[Deref, Field]`, which is how `(*r).f = v` is spelled).
//! `Deref` of a view reads what it names, `StoreThrough` writes it, and `Copy`/`Move` alias it. A
//! `RawView` root is never tracked, and a view used as a block argument escapes its node.
//!
//! # The rewrite
//!
//! Blocks are walked in reverse postorder with a per-block state mapping each live node to its
//! field values. A merge takes the field value all predecessors agree on, or a new block parameter;
//! a loop header optimistically takes a parameter per field of every live node, and `opt::dce`'s
//! trivial-parameter removal deletes the invariant ones. The result is built with a `FuncBuilder`
//! rather than edited in place, and the pipeline verifies it afterwards.

use std::collections::{BTreeMap, BTreeSet};

use crate::jir::origin::OriginId;
use crate::jir::repr::{
    Carrier, CellId, Dominators, Func, FuncBuilder, Op, OpKind, Proj, RAssertMessage, RBlockId,
    RTerm, Root, ValueDef, ValueId,
};
use crate::jir::semantic::{BinaryOp, CastKind, ConstTree, DropGlue, TypeId};
use crate::jir::types::{EnumRepr, NominalForm, TypeKind, TypeTable};

use super::candidates;

/// What the pass saw in one function.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PeaStats {
    /// Candidate object nodes found.
    pub objects: u32,
    /// Nodes that were replaced by their fields.
    pub scalarized: u32,
    /// Nodes the escape fixpoint proved escaping, which stay exactly as they were.
    pub escaped: u32,
    /// Block parameters the rewrite introduced for per-field merges.
    pub params_added: u32,
}

/// Scalarizes every non-escaping aggregate of `func`, in place.
pub fn run(func: &mut Func, table: &TypeTable) -> PeaStats {
    if func.has_invoke() {
        return PeaStats::default();
    }
    let doms = Dominators::of(func);
    let analysis = Analysis::of(func, table, &doms);
    let objects = analysis.nodes.len() as u32;
    let escaped = analysis.escaped.len() as u32;
    let mut stats = PeaStats {
        objects,
        scalarized: objects - escaped,
        escaped,
        params_added: 0,
    };
    if stats.scalarized == 0 {
        return stats;
    }
    match rewrite(func, table, &analysis, &doms) {
        Some((built, params_added)) => {
            *func = built;
            stats.params_added = params_added;
            unbox_cells(func, table);
        }
        // The rewrite found the function in a shape it does not model. Doing nothing is always
        // correct here, so the function is left exactly as the builder made it.
        None => {
            stats.scalarized = 0;
            stats.escaped = objects;
        }
    }
    stats
}

fn pointer_pointee(table: &TypeTable, ty: TypeId) -> Option<TypeId> {
    let peeled = table.peeled(ty);
    match &table.get(peeled)?.kind {
        TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
            Some(*pointee)
        }
        _ => None,
    }
}

type CellProjStep = (Option<u32>, u32, TypeId);

/// Extends a cell projection chain (`(downcast_variant, field_index, field_ty)`) with pure
/// `Downcast`/`Field` steps over direct, non-cloned values.
fn extend_cell_proj(
    table: &TypeTable,
    start_ty: TypeId,
    start_steps: &[CellProjStep],
    proj: &[Proj],
) -> Option<(Vec<CellProjStep>, TypeId)> {
    let mut steps = start_steps.to_vec();
    let mut cur_ty = start_ty;
    let mut pending_variant: Option<u32> = None;
    for step in proj {
        match *step {
            Proj::Downcast { variant } => {
                if pending_variant.is_some() {
                    return None;
                }
                let peeled = table.peeled(cur_ty);
                if !matches!(
                    table.get(peeled).map(|d| &d.kind),
                    Some(TypeKind::Adt {
                        form: NominalForm::Enum,
                        ..
                    })
                ) {
                    return None;
                }
                pending_variant = Some(variant);
            }
            Proj::Field { field, ty } => {
                let peeled = table.peeled(cur_ty);
                match table.get(peeled).map(|d| &d.kind) {
                    Some(TypeKind::Adt {
                        form: NominalForm::Struct | NominalForm::Enum,
                        ..
                    })
                    | Some(TypeKind::Tuple(_)) => {}
                    _ => return None,
                }
                let desc = table.field(cur_ty, pending_variant, field)?;
                if desc.ty != ty || table.is_indirect(ty) || table.needs_clone(ty) {
                    return None;
                }
                steps.push((pending_variant.take(), field, ty));
                cur_ty = ty;
            }
            _ => return None,
        }
    }
    if pending_variant.is_some() {
        return None;
    }
    Some((steps, cur_ty))
}

/// Promotes `BorrowView { root: Root::Cell(c), path }` (empty path or pure `Downcast`/`Field`
/// projections) whose only uses (including `Copy`/`Move` aliases and `[Proj::Deref, ...]`
/// reborrows/projections) are `Deref`/`StoreThrough` into direct `CellLoad(c)` (+ `Field` chain)
/// or `CellStore { cell: c, v }` ops for direct, non-cloned cell types (including `NullNiche`
/// enums such as `Option<Ordering>`).
///
/// Dead `BorrowView` / alias ops are left unused for `dce` to sweep so `Func::verify` stays valid.
pub(crate) fn promote_cell_borrows(func: &mut Func, table: &TypeTable) -> bool {
    use std::collections::btree_map::Entry;

    enum CellRewrite {
        Load(CellId),
        ProjectedLoad {
            cell: CellId,
            cell_ty: TypeId,
            steps: Vec<CellProjStep>,
        },
        Store {
            cell: CellId,
            v: ValueId,
        },
    }

    let mut term_uses: BTreeSet<ValueId> = BTreeSet::new();
    let mut value_uses: BTreeMap<ValueId, Vec<(usize, usize)>> = BTreeMap::new();
    for (block_idx, block) in func.blocks.iter().enumerate() {
        for v in block.term.uses() {
            term_uses.insert(v);
        }
        for (op_idx, op) in block.ops.iter().enumerate() {
            for v in op.kind.uses() {
                let list = value_uses.entry(v).or_default();
                if list.last().copied() != Some((block_idx, op_idx)) {
                    list.push((block_idx, op_idx));
                }
            }
        }
    }

    let mut rewrites: BTreeMap<(usize, usize), CellRewrite> = BTreeMap::new();
    for block in &func.blocks {
        for op in &block.ops {
            let OpKind::BorrowView {
                root: Root::Cell(cell),
                path,
                ..
            } = &op.kind
            else {
                continue;
            };
            let Some(root_val) = op.result else {
                continue;
            };
            let Some(cell_def) = func.cells.get(cell.0 as usize) else {
                continue;
            };
            let cell_ty = cell_def.ty;
            if table.is_indirect(cell_ty) || table.needs_clone(cell_ty) {
                continue;
            }
            let Some((root_steps, root_ty)) = (if path.is_empty() {
                Some((Vec::new(), cell_ty))
            } else {
                extend_cell_proj(table, cell_ty, &[], path)
            }) else {
                continue;
            };
            let root_def = func.values[root_val.0 as usize];
            if root_def.carrier != Carrier::Baseline
                || pointer_pointee(table, root_def.ty) != Some(root_ty)
            {
                continue;
            }

            let mut aliases: BTreeMap<ValueId, (Vec<CellProjStep>, TypeId)> = BTreeMap::new();
            let mut queue: Vec<ValueId> = Vec::new();
            aliases.insert(root_val, (root_steps, root_ty));
            queue.push(root_val);
            let mut derefs: Vec<(usize, usize, Vec<CellProjStep>)> = Vec::new();
            let mut stores: Vec<(usize, usize, ValueId)> = Vec::new();
            let mut ok = true;

            while let Some(ptr) = queue.pop() {
                if term_uses.contains(&ptr) {
                    ok = false;
                    break;
                }
                let Some((ptr_steps, ptr_ty)) = aliases.get(&ptr).cloned() else {
                    continue;
                };
                let Some(uses) = value_uses.get(&ptr) else {
                    continue;
                };
                for &(b_idx, o_idx) in uses {
                    let use_op = &func.blocks[b_idx].ops[o_idx];
                    match &use_op.kind {
                        OpKind::Copy(src) | OpKind::Move(src) if *src == ptr => {
                            if let Some(res) = use_op.result {
                                let res_def = func.values[res.0 as usize];
                                if res_def.carrier != Carrier::Baseline
                                    || pointer_pointee(table, res_def.ty) != Some(ptr_ty)
                                {
                                    ok = false;
                                    break;
                                }
                                if let Entry::Vacant(slot) = aliases.entry(res) {
                                    slot.insert((ptr_steps.clone(), ptr_ty));
                                    queue.push(res);
                                }
                            }
                        }
                        OpKind::BorrowView {
                            root: Root::Value(src),
                            path,
                            ..
                        } if *src == ptr => {
                            let [Proj::Deref, rest @ ..] = path.as_slice() else {
                                ok = false;
                                break;
                            };
                            let Some((next_steps, next_ty)) = (if rest.is_empty() {
                                Some((ptr_steps.clone(), ptr_ty))
                            } else {
                                extend_cell_proj(table, ptr_ty, &ptr_steps, rest)
                            }) else {
                                ok = false;
                                break;
                            };
                            if let Some(res) = use_op.result {
                                let res_def = func.values[res.0 as usize];
                                if res_def.carrier != Carrier::Baseline
                                    || pointer_pointee(table, res_def.ty) != Some(next_ty)
                                {
                                    ok = false;
                                    break;
                                }
                                if let Entry::Vacant(slot) = aliases.entry(res) {
                                    slot.insert((next_steps, next_ty));
                                    queue.push(res);
                                }
                            }
                        }
                        OpKind::Deref(p) if *p == ptr => {
                            if let Some(res) = use_op.result {
                                let res_def = func.values[res.0 as usize];
                                if res_def.ty != ptr_ty || res_def.carrier != Carrier::Baseline {
                                    ok = false;
                                    break;
                                }
                            } else if !ptr_steps.is_empty() {
                                ok = false;
                                break;
                            }
                            derefs.push((b_idx, o_idx, ptr_steps.clone()));
                        }
                        OpKind::StoreThrough { ptr: p, v }
                            if *p == ptr && *v != ptr && ptr_steps.is_empty() =>
                        {
                            let val_def = func.values[v.0 as usize];
                            if val_def.ty != cell_ty || val_def.carrier != Carrier::Baseline {
                                ok = false;
                                break;
                            }
                            stores.push((b_idx, o_idx, *v));
                        }
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    break;
                }
            }

            if !ok || (derefs.is_empty() && stores.is_empty()) {
                continue;
            }
            for (b_idx, o_idx, steps) in derefs {
                let rw = if steps.is_empty() {
                    CellRewrite::Load(*cell)
                } else {
                    CellRewrite::ProjectedLoad {
                        cell: *cell,
                        cell_ty,
                        steps,
                    }
                };
                rewrites.insert((b_idx, o_idx), rw);
            }
            for (b_idx, o_idx, v) in stores {
                rewrites.insert((b_idx, o_idx), CellRewrite::Store { cell: *cell, v });
            }
        }
    }

    if rewrites.is_empty() {
        return false;
    }
    let mut by_block: BTreeMap<usize, BTreeMap<usize, CellRewrite>> = BTreeMap::new();
    for ((b_idx, o_idx), rw) in rewrites {
        by_block.entry(b_idx).or_default().insert(o_idx, rw);
    }
    for (b_idx, block_rws) in by_block {
        let old_ops = std::mem::take(&mut func.blocks[b_idx].ops);
        let mut new_ops = Vec::with_capacity(old_ops.len() + block_rws.len() * 2);
        for (o_idx, mut op) in old_ops.into_iter().enumerate() {
            match block_rws.get(&o_idx) {
                None => new_ops.push(op),
                Some(CellRewrite::Load(cell)) => {
                    op.kind = OpKind::CellLoad(*cell);
                    new_ops.push(op);
                }
                Some(CellRewrite::Store { cell, v }) => {
                    op.kind = OpKind::CellStore { cell: *cell, v: *v };
                    new_ops.push(op);
                }
                Some(CellRewrite::ProjectedLoad {
                    cell,
                    cell_ty,
                    steps,
                }) => {
                    let origin = op.origin;
                    let load_id = ValueId(func.values.len() as u32);
                    func.values.push(ValueDef {
                        ty: *cell_ty,
                        carrier: Carrier::Baseline,
                        origin,
                    });
                    new_ops.push(Op {
                        result: Some(load_id),
                        origin,
                        kind: OpKind::CellLoad(*cell),
                    });
                    let mut cur = load_id;
                    for &(variant, field, step_ty) in &steps[..steps.len() - 1] {
                        let mid = ValueId(func.values.len() as u32);
                        func.values.push(ValueDef {
                            ty: step_ty,
                            carrier: Carrier::Baseline,
                            origin,
                        });
                        new_ops.push(Op {
                            result: Some(mid),
                            origin,
                            kind: OpKind::Field {
                                base: cur,
                                variant,
                                field,
                            },
                        });
                        cur = mid;
                    }
                    let &(last_variant, last_field, _) = steps.last().expect("non-empty steps");
                    op.kind = OpKind::Field {
                        base: cur,
                        variant: last_variant,
                        field: last_field,
                    };
                    new_ops.push(op);
                }
            }
        }
        func.blocks[b_idx].ops = new_ops;
    }
    guard_mutated_param_cells(func, table);
    true
}

/// Demotes `CellDef { boxed: true }` to `boxed: false` when zero ops in `func` mention
/// `Root::Cell(c)` (no `BorrowView`, `RawView`, or `Drop` references the cell).
pub(crate) fn unbox_cells(func: &mut Func, table: &TypeTable) -> bool {
    let mut cell_rooted = BTreeSet::new();
    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::BorrowView {
                    root: Root::Cell(cell),
                    ..
                }
                | OpKind::RawView {
                    root: Root::Cell(cell),
                    ..
                } => {
                    cell_rooted.insert(*cell);
                }
                OpKind::Drop {
                    root: Root::Cell(cell),
                    glue,
                    ..
                } if !matches!(glue, DropGlue::None) => {
                    cell_rooted.insert(*cell);
                }
                _ => {}
            }
        }
    }
    let mut changed = false;
    for (idx, cell) in func.cells.iter_mut().enumerate() {
        let id = CellId(idx as u32);
        if cell.boxed
            && !table.is_indirect(cell.ty)
            && !table.needs_clone(cell.ty)
            && !cell_rooted.contains(&id)
        {
            cell.boxed = false;
            changed = true;
        }
    }
    if changed {
        guard_mutated_param_cells(func, table);
    }
    changed
}

/// Promotes cell borrows and unboxes cells with no remaining `Root::Cell` references.
#[allow(dead_code)]
pub(crate) fn demote_boxed_cells(func: &mut Func, table: &TypeTable) -> bool {
    let promoted = promote_cell_borrows(func, table);
    let unboxed = unbox_cells(func, table);
    promoted || unboxed
}

fn count_value_uses(func: &Func, target: ValueId) -> usize {
    func.blocks
        .iter()
        .map(|block| {
            block
                .ops
                .iter()
                .flat_map(|op| op.kind.uses())
                .chain(block.term.uses())
                .filter(|v| *v == target)
                .count()
        })
        .sum()
}

/// Prevents `fuse.rs` / `emit.rs` from treating a mutated cell as a read-only `param_cells`
/// alias, because `emit.rs` drops `CellStore` writes on unboxed `param_cells`.
fn guard_mutated_param_cells(func: &mut Func, table: &TypeTable) {
    if func.blocks.is_empty() {
        return;
    }
    let mut store_counts = vec![0usize; func.cells.len()];
    for block in &func.blocks {
        for op in &block.ops {
            if let OpKind::CellStore { cell, .. } = op.kind {
                if let Some(slot) = store_counts.get_mut(cell.0 as usize) {
                    *slot += 1;
                }
            }
        }
    }

    let mut insertions: Vec<(usize, CellId, ValueId, bool)> = Vec::new();
    let mut touched = BTreeSet::new();
    for (op_idx, op) in func.blocks[0].ops.iter().enumerate() {
        match &op.kind {
            OpKind::CellAlloc(_) => {}
            OpKind::CellStore { cell, v } if touched.insert(*cell) => {
                let count = store_counts.get(cell.0 as usize).copied().unwrap_or(0);
                let cell_ty = func.cells[cell.0 as usize].ty;
                if count > 1 && func.params.contains(v) {
                    if !table.needs_clone(cell_ty) {
                        insertions.push((op_idx, *cell, *v, false));
                    } else if !func.cells[cell.0 as usize].boxed && count_value_uses(func, *v) == 1
                    {
                        insertions.push((op_idx, *cell, *v, true));
                    }
                }
            }
            other => match other {
                OpKind::CellLoad(cell) | OpKind::CellStore { cell, .. } => {
                    touched.insert(*cell);
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
                } => {
                    touched.insert(*cell);
                }
                _ => {}
            },
        }
    }

    // Apply back-to-front so earlier indices stay valid.
    for (op_idx, _cell, param, needs_move) in insertions.into_iter().rev() {
        let param_def = func.values[param.0 as usize];
        let copy_id = ValueId(func.values.len() as u32);
        func.values.push(param_def);
        let origin = func.blocks[0].ops[op_idx].origin;
        if let OpKind::CellStore { v, .. } = &mut func.blocks[0].ops[op_idx].kind {
            *v = copy_id;
        }
        let kind = if needs_move {
            OpKind::Move(param)
        } else {
            OpKind::Copy(param)
        };
        func.blocks[0].ops.insert(
            op_idx,
            Op {
                result: Some(copy_id),
                origin,
                kind,
            },
        );
    }
}

/// A node index into `Analysis::nodes`.
type NodeId = usize;

/// What a tracked view names: a node, and a field of it (`None` for the whole object).
type View = (NodeId, Option<u32>);

/// The semantic type and physical carrier required by a value or virtual field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Shape {
    ty: TypeId,
    carrier: Carrier,
}

impl Shape {
    fn baseline(ty: TypeId) -> Self {
        Self {
            ty,
            carrier: Carrier::Baseline,
        }
    }

    fn of(func: &Func, value: ValueId) -> Self {
        let def = func.values[value.0 as usize];
        Self {
            ty: def.ty,
            carrier: def.carrier,
        }
    }
}

/// One variant's virtual slot layout within a scalarized multi-variant enum node.
#[derive(Clone, Debug, Eq, PartialEq)]
struct EnumVariantSlots {
    logical_discr: i128,
    slot_offset: usize,
    fields: Vec<TypeId>,
}

/// Slot layout for a multi-variant `EnumRepr::Tag { fieldless: false }` enum node:
/// slot `0` holds the virtual `tag` (`discr_ty`), followed by each variant's payload slots.
#[derive(Clone, Debug, Eq, PartialEq)]
struct EnumInfo {
    discr_ty: TypeId,
    discr_bytes: u8,
    variants: Vec<EnumVariantSlots>,
}

/// One object node: the shape it would have if it never escaped.
#[derive(Clone, Debug)]
struct Node {
    shape: Shape,
    variant: u32,
    fields: Vec<Shape>,
    enum_info: Option<EnumInfo>,
    /// The value the node came into existence as.
    def: ValueId,
    /// The block that value is defined in.
    block: RBlockId,
    /// For a `Copy` node, the value copied from.
    copied_from: Option<ValueId>,
    /// For a `Deref` of a multi-variant whole-enum view, the node snapshot-copied from.
    deref_from: Option<NodeId>,
}

impl Node {
    /// The `variant` a `Field`/`SetField` on this node has to name, as an op spells it.
    fn downcast(&self) -> Option<u32> {
        (self.variant != 0).then_some(self.variant)
    }

    fn slot_of(&self, variant: Option<u32>, field: u32) -> Option<usize> {
        if let Some(info) = &self.enum_info {
            let v = variant.unwrap_or(0) as usize;
            let vinfo = info.variants.get(v)?;
            (field as usize)
                .checked_add(vinfo.slot_offset)
                .filter(|_| (field as usize) < vinfo.fields.len())
        } else {
            (variant.unwrap_or(self.variant) == self.variant
                && (field as usize) < self.fields.len())
            .then_some(field as usize)
        }
    }

    fn variant_arity(&self, variant: Option<u32>) -> usize {
        if let Some(info) = &self.enum_info {
            info.variants
                .get(variant.unwrap_or(0) as usize)
                .map_or(0, |v| v.fields.len())
        } else {
            self.fields.len()
        }
    }
}

fn fits_int(value: i128, signed: bool, bits: u32) -> bool {
    if bits == 0 {
        return false;
    }
    if signed {
        bits >= 128 || {
            let bound = 1i128 << (bits - 1);
            value >= -bound && value < bound
        }
    } else {
        value >= 0 && (bits >= 128 || (value as u128) < (1u128 << bits))
    }
}

fn default_const_tree(table: &TypeTable, ty: TypeId) -> Option<ConstTree> {
    let descriptor = table.get(ty)?;
    if descriptor.layout.zst {
        return Some(ConstTree::Unit);
    }
    if let Some(inner) = descriptor.js.transparent {
        return default_const_tree(table, inner.ty);
    }
    match &descriptor.kind {
        TypeKind::Pattern(base) => default_const_tree(table, *base),
        TypeKind::Bool => Some(ConstTree::Scalar { bits: 0, bytes: 1 }),
        TypeKind::Char => Some(ConstTree::Scalar { bits: 0, bytes: 4 }),
        TypeKind::Integer { bits, .. } | TypeKind::Float { bits } => Some(ConstTree::Scalar {
            bits: 0,
            bytes: (*bits / 8).max(1) as u8,
        }),
        TypeKind::RawPointer { pointee, .. }
            if table.get(*pointee).is_some_and(|d| d.layout.sized) =>
        {
            Some(ConstTree::Address(0))
        }
        TypeKind::Tuple(elems) => elems
            .iter()
            .map(|elem| default_const_tree(table, *elem))
            .collect::<Option<Vec<_>>>()
            .map(ConstTree::Aggregate),
        TypeKind::Array { element, length } if *length <= 16 => {
            let elem = default_const_tree(table, *element)?;
            Some(ConstTree::Aggregate(vec![elem; *length as usize]))
        }
        TypeKind::Adt {
            form: NominalForm::Struct,
            variants,
            ..
        } => variants
            .first()?
            .fields
            .iter()
            .map(|field| default_const_tree(table, field.ty))
            .collect::<Option<Vec<_>>>()
            .map(ConstTree::Aggregate),
        TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            ..
        } if matches!(
            descriptor.js.enum_repr,
            Some(
                EnumRepr::Number { .. }
                    | EnumRepr::Tag { fieldless: true }
                    | EnumRepr::IntTag { fieldless: true }
            )
        ) && !variants.is_empty() =>
        {
            let fields = variants
                .first()?
                .fields
                .iter()
                .map(|field| default_const_tree(table, field.ty))
                .collect::<Option<Vec<_>>>()?;
            Some(ConstTree::Variant { variant: 0, fields })
        }
        TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            ..
        } if matches!(
            descriptor.js.enum_repr,
            Some(EnumRepr::NullNiche { .. } | EnumRepr::SentinelNiche { .. })
        ) =>
        {
            let Some(
                EnumRepr::NullNiche { none_variant, .. }
                | EnumRepr::SentinelNiche { none_variant, .. },
            ) = descriptor.js.enum_repr
            else {
                unreachable!()
            };
            let fields = variants
                .get(none_variant as usize)?
                .fields
                .iter()
                .map(|field| default_const_tree(table, field.ty))
                .collect::<Option<Vec<_>>>()?;
            Some(ConstTree::Variant {
                variant: none_variant,
                fields,
            })
        }
        _ => None,
    }
}

fn enum_discr_type(
    func: &Func,
    table: &TypeTable,
    enum_ty: TypeId,
    shape: &candidates::EnumShape,
) -> Option<(TypeId, u8)> {
    let fits_all = |ty: TypeId| -> Option<u8> {
        let (signed, bits) = table.int_info(ty)?;
        let bytes = u8::try_from((bits / 8).max(1)).ok()?;
        shape
            .variants
            .iter()
            .all(|v| fits_int(v.logical_discr, signed, bits))
            .then_some(bytes)
    };
    for block in &func.blocks {
        for op in &block.ops {
            if let OpKind::Discriminant(v) = op.kind {
                if func
                    .values
                    .get(v.0 as usize)
                    .is_some_and(|d| d.ty == enum_ty)
                {
                    if let Some(res) = op.result {
                        let res_ty = func.values[res.0 as usize].ty;
                        if let Some(bytes) = fits_all(res_ty) {
                            return Some((res_ty, bytes));
                        }
                    }
                }
            }
        }
    }
    if let Some((ty, bytes)) = table
        .descriptors()
        .iter()
        .filter(|d| {
            matches!(
                d.kind,
                TypeKind::Integer {
                    signed: true,
                    bits: 32
                }
            )
        })
        .find_map(|d| fits_all(d.id).map(|bytes| (d.id, bytes)))
    {
        return Some((ty, bytes));
    }
    table
        .descriptors()
        .iter()
        .find_map(|d| fits_all(d.id).map(|bytes| (d.id, bytes)))
}

fn build_enum_info(func: &Func, table: &TypeTable, ty: TypeId) -> Option<EnumInfo> {
    let shape = candidates::enum_shape(table, ty)?;
    for variant in &shape.variants {
        for &field_ty in &variant.fields {
            default_const_tree(table, field_ty)?;
        }
    }
    let (discr_ty, discr_bytes) = enum_discr_type(func, table, ty, &shape)?;
    let mut offset = 1usize;
    let mut variants = Vec::with_capacity(shape.variants.len());
    for variant in shape.variants {
        let len = variant.fields.len();
        variants.push(EnumVariantSlots {
            logical_discr: variant.logical_discr,
            slot_offset: offset,
            fields: variant.fields,
        });
        offset += len;
    }
    Some(EnumInfo {
        discr_ty,
        discr_bytes,
        variants,
    })
}

/// The escape fixpoint's result.
struct Analysis {
    nodes: Vec<Node>,
    /// Every value that is an alias of a node.
    node_of: BTreeMap<ValueId, NodeId>,
    /// Every value that is a tracked view.
    view_of: BTreeMap<ValueId, View>,
    /// The node a node's field holds, for the fields that hold one.
    nested: BTreeMap<(NodeId, u32), NodeId>,
    /// Nodes mutated via `SetField`/`SetDiscriminant` or `StoreThrough` through a tracked view.
    mutated_nodes: BTreeSet<NodeId>,
    escaped: BTreeSet<NodeId>,
}

impl Analysis {
    fn of(func: &Func, table: &TypeTable, doms: &Dominators) -> Self {
        let mut analysis = Self {
            nodes: Vec::new(),
            node_of: BTreeMap::new(),
            view_of: BTreeMap::new(),
            nested: BTreeMap::new(),
            mutated_nodes: BTreeSet::new(),
            escaped: BTreeSet::new(),
        };
        let incoming = predecessors(func, doms);
        analysis.discover(func, table, &incoming);
        analysis.decide(func, table, &incoming, doms);
        analysis
    }

    /// Whether `node` is an immutable baseline single-variant aggregate of plain non-cloning
    /// fields (`Copy` scalar struct/tuple like `{ hi: u32, lo: u32 }`) that can be materialized
    /// lazily at an escaping `OpKind::Aggregate` or `RTerm::Return` without escaping its defining
    /// loop. Read-only `BorrowView`s are allowed because they stay virtual and never mutate `node`.
    fn can_late_materialize(&self, node: NodeId, table: &TypeTable) -> bool {
        if self.mutated_nodes.contains(&node) {
            return false;
        }
        let info = &self.nodes[node];
        info.enum_info.is_none()
            && info.shape.carrier == Carrier::Baseline
            && !info.fields.is_empty()
            && info
                .fields
                .iter()
                .all(|f| f.carrier == Carrier::Baseline && !table.needs_clone(f.ty))
            && candidates::field_reads_resolve(
                table,
                info.shape.ty,
                info.variant,
                info.fields.len(),
            )
    }

    /// The node `v` is an alias of, if the fixpoint kept it virtual.
    fn live_node(&self, v: ValueId) -> Option<NodeId> {
        self.node_of
            .get(&v)
            .copied()
            .filter(|node| !self.escaped.contains(node))
    }

    /// The view `v` is, if its node stayed virtual.
    fn live_view(&self, v: ValueId) -> Option<View> {
        self.view_of
            .get(&v)
            .copied()
            .filter(|(node, _)| !self.escaped.contains(node))
    }

    fn arity(&self, node: NodeId) -> usize {
        self.nodes[node].fields.len()
    }

    /// Assigns nodes and views to values, to a fixpoint. Monotone and independent of escapes: a
    /// `Copy`, a `Move`, a `Deref` or a nested `BorrowView` is classified once its operand is.
    fn discover(&mut self, func: &Func, table: &TypeTable, incoming: &[Vec<Vec<ValueId>>]) {
        // A block parameter is a node by its type alone — but only where it really is bound from
        // an incoming edge. The entry parameters come from the caller, and an unreachable block's
        // come from nowhere.
        // The variant each aggregate-building definition fixes, for the parameter rule below.
        let mut built_variant: BTreeMap<ValueId, u32> = BTreeMap::new();
        for block in &func.blocks {
            for op in &block.ops {
                let Some(result) = op.result else {
                    continue;
                };
                match &op.kind {
                    OpKind::Aggregate {
                        variant,
                        active_field: None,
                        ..
                    } => {
                        built_variant.insert(result, *variant);
                    }
                    OpKind::Const(ConstTree::Aggregate(_)) => {
                        built_variant.insert(result, 0);
                    }
                    OpKind::Const(ConstTree::Variant { variant, .. }) => {
                        built_variant.insert(result, *variant);
                    }
                    _ => {}
                }
            }
        }
        for block in &func.blocks {
            if block.id.0 == 0 || incoming[block.id.0 as usize].is_empty() {
                continue;
            }
            for (position, param) in block.params.iter().enumerate() {
                // D10a: a value carrying a committed non-baseline representation is a
                // pre-escaped root — never a node.
                if func.values[param.0 as usize].carrier != Carrier::Baseline {
                    continue;
                }
                let ty = func.values[param.0 as usize].ty;
                if let Some(fields) = candidates::shape(table, ty, 0) {
                    self.add_node(*param, block.id, ty, 0, fields, None);
                    continue;
                }
                if let Some(info) = build_enum_info(func, table, ty) {
                    self.add_enum_node(*param, block.id, ty, info, None, None);
                    continue;
                }
                // A multi-variant enum parameter, which `shape` refuses for its unknown variant,
                // is still a candidate when every incoming edge passes the same statically built
                // variant — the shape discriminant jump-threading leaves at each arm.
                let mut agreed: Option<u32> = None;
                let mut all_known = true;
                for args in &incoming[block.id.0 as usize] {
                    let Some(arg) = args.get(position) else {
                        all_known = false;
                        break;
                    };
                    if arg == param {
                        continue;
                    }
                    match (agreed, built_variant.get(arg)) {
                        (None, Some(&variant)) => agreed = Some(variant),
                        (Some(seen), Some(&variant)) if seen == variant => {}
                        _ => {
                            all_known = false;
                            break;
                        }
                    }
                }
                let (true, Some(variant)) = (all_known, agreed) else {
                    continue;
                };
                if let Some(fields) = candidates::variant_shape(table, ty, variant) {
                    self.add_node(*param, block.id, ty, variant, fields, None);
                }
            }
        }
        loop {
            let mut changed = false;
            // Which node each node's field holds, which is what makes a `Field` read an alias.
            for block in &func.blocks {
                for op in &block.ops {
                    let (Some(result), OpKind::Aggregate { fields, .. }) = (op.result, &op.kind)
                    else {
                        continue;
                    };
                    let Some(&parent) = self.node_of.get(&result) else {
                        continue;
                    };
                    if self.nodes[parent].enum_info.is_some() {
                        continue;
                    }
                    for (index, field) in fields.iter().enumerate() {
                        if let Some(&child) = self.node_of.get(field) {
                            changed |= self.nested.insert((parent, index as u32), child).is_none();
                        }
                    }
                }
            }
            for block in &func.blocks {
                for op in &block.ops {
                    let Some(result) = op.result else {
                        continue;
                    };
                    if self.node_of.contains_key(&result) || self.view_of.contains_key(&result) {
                        continue;
                    }
                    changed |= self.discover_op(result, block.id, &op.kind, func, table);
                }
            }
            if !changed {
                return;
            }
        }
    }

    /// One definition site. Returns whether it classified the result.
    fn discover_op(
        &mut self,
        result: ValueId,
        block: RBlockId,
        kind: &OpKind,
        func: &Func,
        table: &TypeTable,
    ) -> bool {
        // D10a: a committed non-baseline value is a pre-escaped root, never a node or a view.
        if func.values[result.0 as usize].carrier != Carrier::Baseline {
            return false;
        }
        let ty = func.values[result.0 as usize].ty;
        match kind {
            OpKind::Aggregate {
                ty: aggregate_ty,
                variant,
                active_field: None,
                fields,
            } => {
                if let Some(info) = build_enum_info(func, table, *aggregate_ty) {
                    let Some(vinfo) = info.variants.get(*variant as usize) else {
                        return false;
                    };
                    if vinfo.fields.len() != fields.len()
                        || !vinfo.fields.iter().zip(fields).all(|(declared, field)| {
                            Shape::of(func, *field) == Shape::baseline(*declared)
                        })
                    {
                        return false;
                    }
                    self.add_enum_node(result, block, *aggregate_ty, info, None, None);
                    return true;
                }
                let Some(shape) = candidates::variant_shape(table, *aggregate_ty, *variant) else {
                    return false;
                };
                if shape.len() != fields.len()
                    || !shape.iter().zip(fields).all(|(declared, field)| {
                        Shape::of(func, *field) == Shape::baseline(*declared)
                    })
                {
                    return false;
                }
                self.add_node(result, block, *aggregate_ty, *variant, shape, None);
                true
            }
            OpKind::Const(tree) => {
                let (variant, arity) = match tree {
                    ConstTree::Aggregate(trees) => (0, trees.len()),
                    ConstTree::Variant { variant, fields } => (*variant, fields.len()),
                    _ => return false,
                };
                if matches!(tree, ConstTree::Variant { .. }) {
                    if let Some(info) = build_enum_info(func, table, ty) {
                        let Some(vinfo) = info.variants.get(variant as usize) else {
                            return false;
                        };
                        if vinfo.fields.len() != arity {
                            return false;
                        }
                        self.add_enum_node(result, block, ty, info, None, None);
                        return true;
                    }
                }
                let Some(shape) = candidates::variant_shape(table, ty, variant) else {
                    return false;
                };
                if shape.len() != arity {
                    return false;
                }
                self.add_node(result, block, ty, variant, shape, None);
                true
            }
            // A copy is a *fresh* node over the same field values: mutating it must not touch the
            // original. A move renames, so it stays the same node.
            OpKind::Copy(v) if Shape::of(func, *v) == Shape::of(func, result) => {
                match (self.node_of.get(v).copied(), self.view_of.get(v).copied()) {
                    (Some(node), _) => {
                        let source = self.nodes[node].clone();
                        if let Some(info) = source.enum_info {
                            self.add_enum_node(
                                result,
                                block,
                                source.shape.ty,
                                info,
                                Some(*v),
                                None,
                            );
                        } else {
                            self.add_node(
                                result,
                                block,
                                source.shape.ty,
                                source.variant,
                                source.fields.iter().map(|field| field.ty).collect(),
                                Some(*v),
                            );
                        }
                        true
                    }
                    (None, Some(view)) => {
                        self.view_of.insert(result, view);
                        true
                    }
                    (None, None) => {
                        // A call result is necessarily a real value, but a local copy of its
                        // single-variant product can still be virtual. Read and copy its fields
                        // once at the copy site; later field mutation then
                        // changes only the virtual copy. This is the common
                        // `let mut chunks = slice.chunks_exact(n)` shape.
                        let from_call = func.blocks.iter().flat_map(|block| &block.ops).any(|op| {
                            op.result == Some(*v) && matches!(op.kind, OpKind::Call { .. })
                        });
                        let Some(fields) = from_call
                            .then(|| candidates::shape(table, ty, 0))
                            .flatten()
                            .filter(|fields| {
                                candidates::field_reads_resolve(table, ty, 0, fields.len())
                            })
                        else {
                            return false;
                        };
                        self.add_node(result, block, ty, 0, fields, Some(*v));
                        true
                    }
                }
            }
            OpKind::Move(v) if Shape::of(func, *v) == Shape::of(func, result) => {
                match (self.node_of.get(v).copied(), self.view_of.get(v).copied()) {
                    (Some(node), _) => {
                        self.node_of.insert(result, node);
                        true
                    }
                    (None, Some(view)) => {
                        self.view_of.insert(result, view);
                        true
                    }
                    (None, None) => false,
                }
            }
            // A field that holds another node reads as that node, not as a value.
            OpKind::Field {
                base,
                variant,
                field,
            } => {
                let Some(&parent) = self.node_of.get(base) else {
                    return false;
                };
                if self.nodes[parent].enum_info.is_some()
                    || variant.unwrap_or(self.nodes[parent].variant) != self.nodes[parent].variant
                {
                    return false;
                }
                match self.nested.get(&(parent, *field)) {
                    Some(&child) if self.nodes[child].shape == Shape::of(func, result) => {
                        self.node_of.insert(result, child);
                        true
                    }
                    _ => false,
                }
            }
            // Reading through a view of the whole object is the object itself (or a snapshot
            // node for a multi-variant enum so later `StoreThrough` writes do not alias).
            OpKind::Deref(pointer) => match self.view_of.get(pointer).copied() {
                Some((node, None)) if self.nodes[node].shape == Shape::of(func, result) => {
                    if let Some(info) = self.nodes[node].enum_info.clone() {
                        self.add_enum_node(result, block, ty, info, None, Some(node));
                    } else {
                        self.node_of.insert(result, node);
                    }
                    true
                }
                _ => false,
            },
            OpKind::BorrowView {
                root: Root::Value(root),
                path,
                ..
            } => {
                let view = if let Some(&node) = self.node_of.get(root) {
                    match tracked_field(path, &self.nodes[node]) {
                        Some(field) => (node, field),
                        None => return false,
                    }
                } else if let Some(&(node, field)) = self.view_of.get(root) {
                    match (field, path.split_first()) {
                        // Through a view of the whole object, the path names the field.
                        (None, Some((Proj::Deref, rest))) => {
                            match tracked_field(rest, &self.nodes[node]) {
                                Some(field) => (node, field),
                                None => return false,
                            }
                        }
                        // A reborrow of a view of one field names the same field.
                        (Some(_), Some((Proj::Deref, []))) => (node, field),
                        _ => return false,
                    }
                } else {
                    return false;
                };
                if view
                    .1
                    .is_some_and(|field| field as usize >= self.arity(view.0))
                {
                    return false;
                }
                self.view_of.insert(result, view);
                true
            }
            _ => false,
        }
    }

    fn add_node(
        &mut self,
        value: ValueId,
        block: RBlockId,
        ty: TypeId,
        variant: u32,
        fields: Vec<TypeId>,
        copied_from: Option<ValueId>,
    ) {
        let id = self.nodes.len();
        self.nodes.push(Node {
            shape: Shape::baseline(ty),
            variant,
            fields: fields.into_iter().map(Shape::baseline).collect(),
            enum_info: None,
            def: value,
            block,
            copied_from,
            deref_from: None,
        });
        self.node_of.insert(value, id);
    }

    fn add_enum_node(
        &mut self,
        value: ValueId,
        block: RBlockId,
        ty: TypeId,
        info: EnumInfo,
        copied_from: Option<ValueId>,
        deref_from: Option<NodeId>,
    ) {
        let mut fields =
            Vec::with_capacity(1 + info.variants.iter().map(|v| v.fields.len()).sum::<usize>());
        fields.push(Shape::baseline(info.discr_ty));
        for v in &info.variants {
            fields.extend(v.fields.iter().copied().map(Shape::baseline));
        }
        let id = self.nodes.len();
        self.nodes.push(Node {
            shape: Shape::baseline(ty),
            variant: 0,
            fields,
            enum_info: Some(info),
            def: value,
            block,
            copied_from,
            deref_from,
        });
        self.node_of.insert(value, id);
    }

    /// Walks every use, seeds the nodes that need a real object, and closes the implications over
    /// block parameters and copies.
    fn decide(
        &mut self,
        func: &Func,
        table: &TypeTable,
        incoming: &[Vec<Vec<ValueId>>],
        doms: &Dominators,
    ) {
        let mut seeds: BTreeSet<NodeId> = BTreeSet::new();
        let mut implications: Vec<(NodeId, NodeId)> = Vec::new();
        // Slots whose contents the analysis cannot pin to one node: a `Field` read of one is a
        // value, so a node that lives there has to be a real object.
        let mut unpinned: BTreeSet<(NodeId, u32)> = BTreeSet::new();
        for &(node, field) in self.view_of.values() {
            if let Some(field) = field {
                unpinned.insert((node, field));
            }
        }
        for block in &func.blocks {
            for op in &block.ops {
                match &op.kind {
                    OpKind::SetField { base, .. } | OpKind::SetDiscriminant { base, .. } => {
                        if let Some(&node) = self.node_of.get(base) {
                            self.mutated_nodes.insert(node);
                        }
                    }
                    OpKind::StoreThrough { ptr, .. } => {
                        if let Some(&(node, _)) = self.view_of.get(ptr) {
                            self.mutated_nodes.insert(node);
                        }
                    }
                    _ => {}
                }
            }
        }

        for block in &func.blocks {
            for op in &block.ops {
                if let OpKind::SetField {
                    base,
                    variant,
                    field,
                    ..
                } = &op.kind
                {
                    if let Some(&node) = self.node_of.get(base) {
                        if let Some(slot) = self.nodes[node].slot_of(*variant, *field) {
                            unpinned.insert((node, slot as u32));
                        }
                    }
                }
                if let OpKind::StoreThrough { ptr, v } = &op.kind {
                    if let Some(&(target_node, None)) = self.view_of.get(ptr) {
                        if let Some(&src_node) = self.node_of.get(v) {
                            if self.nodes[target_node].enum_info.is_some()
                                && self.same_shape(target_node, src_node)
                            {
                                if self.nodes[src_node]
                                    .fields
                                    .iter()
                                    .any(|field| table.needs_clone(field.ty))
                                {
                                    seeds.insert(target_node);
                                    seeds.insert(src_node);
                                } else {
                                    implications.push((target_node, src_node));
                                    implications.push((src_node, target_node));
                                }
                            }
                        }
                    }
                }
                let mut allowed = self.allowed_uses(op, func, table);
                if let OpKind::Aggregate { fields, .. } = &op.kind {
                    if op.result.and_then(|r| self.node_of.get(&r)).is_none() {
                        for v in fields {
                            if let Some(&child) = self.node_of.get(v) {
                                if self.can_late_materialize(child, table) {
                                    unpinned
                                        .extend((0..self.arity(child) as u32).map(|f| (child, f)));
                                }
                            }
                        }
                    }
                }
                for used in op.kind.uses() {
                    match allowed.iter().position(|value| *value == used) {
                        Some(position) => {
                            allowed.remove(position);
                        }
                        None => {
                            self.seed(&mut seeds, used);
                        }
                    }
                }
            }
            // Block arguments are fine for a node (the parameter links them below) and never for
            // a view; everything else a terminator reads needs a real object.
            for (_, args) in block.term.edges() {
                for arg in args {
                    if let Some(&(node, _)) = self.view_of.get(arg) {
                        seeds.insert(node);
                    }
                }
            }
            match &block.term {
                RTerm::Switch { scrut, .. } => self.seed(&mut seeds, *scrut),
                RTerm::Return(Some(v)) => {
                    if let Some(&node) = self.node_of.get(v) {
                        if self.can_late_materialize(node, table) {
                            unpinned.extend((0..self.arity(node) as u32).map(|f| (node, f)));
                        } else {
                            self.seed(&mut seeds, *v);
                        }
                    } else {
                        self.seed(&mut seeds, *v);
                    }
                }
                RTerm::Throw(Some(v)) => self.seed(&mut seeds, *v),
                _ => {}
            }
        }

        // Nesting is not followed through a merge (D4 rule 4): a block parameter's fields are
        // values, so a node that would sit in one becomes a real object instead.
        for block in &func.blocks {
            for param in &block.params {
                if let Some(&node) = self.node_of.get(param) {
                    unpinned.extend((0..self.arity(node) as u32).map(|field| (node, field)));
                }
            }
            for (_, args) in block.term.edges() {
                for arg in args {
                    if let Some(&node) = self.node_of.get(arg) {
                        unpinned.extend((0..self.arity(node) as u32).map(|field| (node, field)));
                    }
                }
            }
        }

        // A merge cannot carry a nested node, and a loop header merges everything live at it, so
        // a node whose block dominates a header — and so is live there — unpins its fields.
        let headers: BTreeSet<RBlockId> = func
            .blocks
            .iter()
            .flat_map(|block| {
                block
                    .term
                    .successors()
                    .into_iter()
                    .filter(|target| doms.dominates(*target, block.id))
            })
            .collect();
        for (index, node) in self.nodes.iter().enumerate() {
            if self.mutated_nodes.contains(&index)
                && headers
                    .iter()
                    .any(|header| node.block != *header && doms.dominates(node.block, *header))
            {
                unpinned.extend((0..node.fields.len() as u32).map(|field| (index, field)));
            }
        }

        // A node in another node's field lives exactly as long as the object holding it — unless
        // `child` is an immutable scalar `Copy` struct (`can_late_materialize`) that can be
        // materialized right at an escaping outer `OpKind::Aggregate` (e.g. `[Word; 8]` after
        // SHA-512's 80-round loop) without propagating escape back into the loop.
        // Note: if `parent` itself ends up escaping (e.g. `c6 = [p, E, ...]` dominating a later
        // loop), `unpinned` on `parent` is irrelevant because `ops()` materializes `child`
        // directly into `parent`'s `Aggregate`. We only seed `child` for `unpinned` after the
        // initial fixpoint if `parent` stayed virtual.
        let mut deferred_unpinned: Vec<(NodeId, NodeId)> = Vec::new();
        for (&(parent, field), &child) in &self.nested {
            if self.can_late_materialize(child, table) {
                if unpinned.contains(&(parent, field)) {
                    deferred_unpinned.push((parent, child));
                }
            } else {
                implications.push((parent, child));
                if unpinned.contains(&(parent, field)) {
                    seeds.insert(child);
                }
            }
        }

        // A parameter follows its incoming arguments into escape, never the reverse: an escaped
        // argument has no field state for an edge to pass on, but a virtual argument reaching a
        // parameter that is real — escaped for reasons of its own, of another shape, or not a
        // node at all — is *materialized on that edge* by the rewrite instead of escaping
        // everywhere (D11a). A parameter whose incoming values disagree in shape can only be a
        // real object, so it is seeded; its virtual arguments stay virtual.
        for block in &func.blocks {
            for (position, param) in block.params.iter().enumerate() {
                let param_node = self.node_of.get(param).copied();
                for args in &incoming[block.id.0 as usize] {
                    let arg_node = args
                        .get(position)
                        .and_then(|arg| self.node_of.get(arg).copied());
                    match (param_node, arg_node) {
                        (Some(p), Some(a)) if self.same_shape(p, a) => {
                            implications.push((a, p));
                            if self.nodes[a].enum_info.is_some() {
                                implications.push((p, a));
                            }
                        }
                        (Some(p), _) => {
                            seeds.insert(p);
                        }
                        (None, Some(a)) if self.nodes[a].enum_info.is_some() => {
                            seeds.insert(a);
                        }
                        (None, _) => {}
                    }
                }
            }
        }

        // A copy of an object that escaped reads its fields back out, which needs `Field` to name
        // them; a copy that escapes itself needs the original to still exist.
        for (id, node) in self.nodes.iter().enumerate() {
            if let Some(src) = node.deref_from {
                if node.fields.iter().any(|field| table.needs_clone(field.ty)) {
                    seeds.insert(id);
                    seeds.insert(src);
                } else {
                    implications.push((id, src));
                    implications.push((src, id));
                }
            }
            let Some(source) = node.copied_from else {
                continue;
            };
            // Sharing the field values is a duplicate only when no field is an object of its own.
            if node.fields.iter().any(|field| table.needs_clone(field.ty))
                && self.node_of.contains_key(&source)
            {
                seeds.insert(id);
                if let Some(&from) = self.node_of.get(&source) {
                    seeds.insert(from);
                }
                continue;
            }
            let Some(&from) = self.node_of.get(&source) else {
                continue;
            };
            implications.push((id, from));
            // A copy fixes its variant at the copy site. An object the copy reads its fields back
            // out of can be *given another variant* behind its back — `*r = None`, a `&mut` call,
            // `mem::replace` — and the copy would then read the old variant's fields off the new
            // object. Only a shape with one variant is safe to copy out of a real object.
            if !candidates::field_reads_resolve(
                table,
                node.shape.ty,
                node.variant,
                node.fields.len(),
            ) || table.variants(node.shape.ty).len() > 1
            {
                implications.push((from, id));
            }
        }

        let mut edges: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
        for &(from, to) in &implications {
            edges.entry(from).or_default().push(to);
        }
        let mut worklist: Vec<NodeId> = seeds.iter().copied().collect();
        self.escaped = seeds;
        while let Some(node) = worklist.pop() {
            for next in edges.get(&node).into_iter().flatten() {
                if self.escaped.insert(*next) {
                    worklist.push(*next);
                }
            }
        }
        // If a virtual `parent` holding late-materializable children is itself a pure packaging
        // `Aggregate` passed into an escaped/real block parameter (e.g. `[a, b, c, d, e, f, g, h]`
        // handed to `IntoIter` right after SHA-512's 80-round loop), it is going to be
        // materialized on that edge anyway. Escaping `parent` directly keeps all its
        // `can_late_materialize` children virtual (materialized once at `parent`'s `Aggregate` in
        // `ops()`) instead of escaping them back into their defining loop.
        if !deferred_unpinned.is_empty() {
            let mut candidate_parents: BTreeSet<NodeId> = deferred_unpinned
                .iter()
                .map(|&(parent, _)| parent)
                .filter(|parent| !self.escaped.contains(parent))
                .collect();
            if !candidate_parents.is_empty() {
                let mut aggregate_defined: BTreeSet<NodeId> = BTreeSet::new();
                let mut has_virtual_access: BTreeSet<NodeId> = BTreeSet::new();
                for block in &func.blocks {
                    for op in &block.ops {
                        if let (Some(res), OpKind::Aggregate { .. }) = (op.result, &op.kind) {
                            if let Some(&node) = self.node_of.get(&res) {
                                if candidate_parents.contains(&node) {
                                    aggregate_defined.insert(node);
                                }
                            }
                        }
                        match &op.kind {
                            OpKind::Field { base, .. }
                            | OpKind::SetField { base, .. }
                            | OpKind::Discriminant(base)
                            | OpKind::SetDiscriminant { base, .. }
                            | OpKind::Copy(base) => {
                                if let Some(&node) = self.node_of.get(base) {
                                    has_virtual_access.insert(node);
                                }
                            }
                            OpKind::BorrowView {
                                root: Root::Value(base),
                                ..
                            } => {
                                if let Some(&node) = self.node_of.get(base) {
                                    has_virtual_access.insert(node);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                let mut edge_materialized: BTreeSet<NodeId> = BTreeSet::new();
                for block in &func.blocks {
                    for (target, args) in block.term.edges() {
                        let target_params = &func.blocks[target.0 as usize].params;
                        for (pos, arg) in args.iter().enumerate() {
                            let Some(&arg_node) = self.node_of.get(arg) else {
                                continue;
                            };
                            if !candidate_parents.contains(&arg_node) {
                                continue;
                            }
                            let param_virtual_same = target_params
                                .get(pos)
                                .and_then(|p| self.node_of.get(p).copied())
                                .is_some_and(|p| {
                                    !self.escaped.contains(&p) && self.same_shape(arg_node, p)
                                });
                            if !param_virtual_same {
                                edge_materialized.insert(arg_node);
                            }
                        }
                    }
                }
                candidate_parents.retain(|node| {
                    aggregate_defined.contains(node)
                        && edge_materialized.contains(node)
                        && !has_virtual_access.contains(node)
                });
                for parent in candidate_parents {
                    if self.escaped.insert(parent) {
                        worklist.push(parent);
                    }
                }
                while let Some(node) = worklist.pop() {
                    for next in edges.get(&node).into_iter().flatten() {
                        if self.escaped.insert(*next) {
                            worklist.push(*next);
                        }
                    }
                }
            }
        }
        // Any late-materializable child nested inside an unpinned field of a *virtual* parent must
        // still escape so the parent's merge/loop-header state carries a plain value rather than
        // `FieldVal::Node`. Escaped parents materialize the child directly in `ops()`.
        for (parent, child) in deferred_unpinned {
            if !self.escaped.contains(&parent) && self.escaped.insert(child) {
                worklist.push(child);
            }
        }
        while let Some(node) = worklist.pop() {
            for next in edges.get(&node).into_iter().flatten() {
                if self.escaped.insert(*next) {
                    worklist.push(*next);
                }
            }
        }
    }

    /// Marks whatever `v` names — a node or a view's node — as needing a real object.
    fn seed(&self, seeds: &mut BTreeSet<NodeId>, v: ValueId) {
        if let Some(&node) = self.node_of.get(&v) {
            seeds.insert(node);
        }
        if let Some(&(node, _)) = self.view_of.get(&v) {
            seeds.insert(node);
        }
    }

    fn same_shape(&self, a: NodeId, b: NodeId) -> bool {
        let (a, b) = (&self.nodes[a], &self.nodes[b]);
        a.shape == b.shape
            && a.fields == b.fields
            && a.enum_info == b.enum_info
            && (a.enum_info.is_some() || a.variant == b.variant)
    }

    /// The operands of `kind` a node or a view may sit in without escaping.
    ///
    /// Anything not listed here escapes, which is the point: a use this pass has never heard of
    /// falls through to the default arm and the object stays an object.
    fn allowed_uses(&self, op: &Op, func: &Func, table: &TypeTable) -> Vec<ValueId> {
        let shape_of = |v| Shape::of(func, v);
        // A virtual field has one physical shape. Unsupported reads or writes leave the object
        // materialized; they cannot relabel its saved field values or introduce an implicit
        // conversion when a merge or copy reconstructs it.
        let field_ok = |base: &ValueId, variant: &Option<u32>, field: u32, shape: Shape| -> bool {
            let Some(&id) = self.node_of.get(base) else {
                return false;
            };
            let node = &self.nodes[id];
            let Some(slot) = node.slot_of(*variant, field) else {
                return false;
            };
            let v = if node.enum_info.is_some() {
                variant.unwrap_or(0)
            } else {
                node.variant
            };
            node.fields.get(slot) == Some(&shape)
                && candidates::field_reads_resolve(
                    table,
                    node.shape.ty,
                    v,
                    node.variant_arity(*variant),
                )
        };
        let view_field = |view: ValueId| -> Option<(NodeId, u32)> {
            match self.view_of.get(&view) {
                Some(&(node, Some(field))) => Some((node, field)),
                _ => None,
            }
        };
        let single = |value: ValueId, ok: bool| if ok { vec![value] } else { Vec::new() };
        match &op.kind {
            OpKind::Copy(v) | OpKind::Move(v) => single(
                *v,
                op.result
                    .is_some_and(|result| shape_of(result) == shape_of(*v)),
            ),
            OpKind::Discriminant(v) => single(
                *v,
                self.node_of.get(v).is_some_and(|&id| {
                    op.result.is_some_and(|result| {
                        let shape = shape_of(result);
                        shape.carrier == Carrier::Baseline
                            && (self.nodes[id].enum_info.is_none()
                                || table.int_info(shape.ty).is_some())
                    })
                }),
            ),
            OpKind::SetDiscriminant { base, variant } => single(
                *base,
                op.result.is_none()
                    && self.node_of.get(base).is_some_and(|&id| {
                        self.nodes[id]
                            .enum_info
                            .as_ref()
                            .is_some_and(|info| (*variant as usize) < info.variants.len())
                    }),
            ),
            OpKind::Field {
                base,
                variant,
                field,
            } => single(
                *base,
                op.result
                    .is_some_and(|result| field_ok(base, variant, *field, shape_of(result))),
            ),
            OpKind::SetField {
                base,
                variant,
                field,
                v,
            } => single(*base, field_ok(base, variant, *field, shape_of(*v))),
            // A view was recorded during discovery exactly when its shape is one this pass
            // follows, so a recorded result is the whole condition.
            OpKind::BorrowView {
                root: Root::Value(root),
                ..
            } => single(
                *root,
                op.result
                    .is_some_and(|result| self.view_of.contains_key(&result)),
            ),
            OpKind::Deref(pointer) => single(
                *pointer,
                match (view_field(*pointer), op.result) {
                    // Through a whole-object view the result is the object itself.
                    (None, Some(result)) => self
                        .view_of
                        .get(pointer)
                        .is_some_and(|(node, _)| self.nodes[*node].shape == shape_of(result)),
                    (Some((node, field)), Some(result)) => {
                        self.nodes[node].fields.get(field as usize) == Some(&shape_of(result))
                    }
                    (_, None) => false,
                },
            ),
            OpKind::StoreThrough { ptr, v } => {
                if let Some((node, field)) = view_field(*ptr) {
                    single(
                        *ptr,
                        self.nodes[node].fields.get(field as usize) == Some(&shape_of(*v)),
                    )
                } else if let Some(&(target_node, None)) = self.view_of.get(ptr) {
                    if let Some(&src_node) = self.node_of.get(v) {
                        if self.nodes[target_node].enum_info.is_some()
                            && self.same_shape(target_node, src_node)
                        {
                            vec![*ptr, *v]
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            }
            // A node in another node's field: the outer object holding it is what decides.
            // If the outer `Aggregate` is not a tracked node (e.g. a `[Word; 8]` array built
            // after an 80-round loop), an immutable scalar `Copy` struct can be materialized
            // right at this aggregate site without escaping its defining loop.
            OpKind::Aggregate { fields, .. } => {
                match op.result.and_then(|r| self.node_of.get(&r)) {
                    Some(&parent) => fields
                        .iter()
                        .enumerate()
                        .filter(|(index, v)| {
                            // `None == None` would let a *view* sit here unnoticed, and the
                            // rewrite has no value to put in its place.
                            matches!(
                                self.nested.get(&(parent, *index as u32)),
                                Some(child) if self.node_of.get(v) == Some(child)
                            )
                        })
                        .map(|(_, v)| *v)
                        .collect(),
                    None => fields
                        .iter()
                        .copied()
                        .filter(|v| {
                            self.node_of
                                .get(v)
                                .is_some_and(|&child| self.can_late_materialize(child, table))
                        })
                        .collect(),
                }
            }
            // `[u8; N]` to `uN`: the bytes are consumed one at a time, so the array never exists.
            OpKind::Cast {
                kind: CastKind::Transmute,
                v,
                to,
                ..
            } => single(
                *v,
                self.node_of.get(v).is_some_and(|node| {
                    op.result
                        .is_some_and(|result| shape_of(result) == Shape::baseline(*to))
                        && candidates::byte_array_transmute(table, self.nodes[*node].shape.ty, *to)
                            .is_some()
                }),
            ),
            _ => Vec::new(),
        }
    }
}

/// The field a tracked view's path names: `[]` is the whole object, `[Field]` is one field (behind
/// the node's `Downcast` when it has one), and anything else is not tracked.
fn tracked_field(path: &[Proj], node: &Node) -> Option<Option<u32>> {
    match path {
        [] => Some(None),
        [Proj::Field { field, .. }] => {
            if node.enum_info.is_some() {
                node.slot_of(None, *field).map(|s| Some(s as u32))
            } else if node.variant == 0 {
                Some(Some(*field))
            } else {
                None
            }
        }
        [Proj::Downcast { variant: k }, Proj::Field { field, .. }] => {
            if node.enum_info.is_some() {
                node.slot_of(Some(*k), *field).map(|s| Some(s as u32))
            } else if *k == node.variant {
                Some(Some(*field))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The argument lists reaching each block from its reachable predecessors, in block order.
fn predecessors(func: &Func, doms: &Dominators) -> Vec<Vec<Vec<ValueId>>> {
    let mut incoming: Vec<Vec<Vec<ValueId>>> = vec![Vec::new(); func.blocks.len()];
    for &id in &doms.rpo {
        for (target, args) in func.blocks[id.0 as usize].term.edges() {
            incoming[target.0 as usize].push(args.to_vec());
        }
    }
    incoming
}

// ---------------------------------------------------------------------------------------------
// The rewrite
// ---------------------------------------------------------------------------------------------

/// The whole rewrite. `None` means the function was in a shape the rewrite does not model, and the
/// caller keeps the original.
fn rewrite(
    func: &Func,
    table: &TypeTable,
    analysis: &Analysis,
    doms: &Dominators,
) -> Option<(Func, u32)> {
    let mut rewriter = Rewriter {
        old: func,
        table,
        analysis,
        doms,
        builder: FuncBuilder::new(func.item, func.sig.clone()),
        map: BTreeMap::new(),
        carriers: BTreeMap::new(),
        exit: vec![BTreeMap::new(); func.blocks.len()],
        merged: vec![Vec::new(); func.blocks.len()],
        live_in: live_nodes(func, analysis, doms),
        constants: BTreeMap::new(),
        materialized: BTreeMap::new(),
        params_added: 0,
    };
    rewriter.run()?;
    let mut built = rewriter.builder.finish();
    for (value, carrier) in rewriter.carriers {
        built.values[value.0 as usize].carrier = carrier;
    }
    // Eliminating a projection may alias an existing field, but cannot relabel it. Newly
    // generated fields already have their reconstruction shape, independently of their readers.
    for (old, new) in rewriter.map {
        if Shape::of(func, old) != Shape::of(&built, new) {
            return None;
        }
    }
    built
        .verify_carriers(table, super::observed_carriers(func))
        .ok()?;
    Some((built, rewriter.params_added))
}

/// The live-node dataflow: which nodes have field values on entry to each block.
///
/// A forward *must* analysis (intersection over predecessors), which is exactly "created in a
/// block that dominates this one". It is computed up front so the rewrite knows a loop header's
/// parameter layout before the latch that feeds it is walked.
fn live_nodes(func: &Func, analysis: &Analysis, doms: &Dominators) -> Vec<BTreeSet<NodeId>> {
    let blocks = func.blocks.len();
    let all: BTreeSet<NodeId> = (0..analysis.nodes.len())
        .filter(|node| !analysis.escaped.contains(node))
        .collect();
    let mut created: Vec<BTreeSet<NodeId>> = vec![BTreeSet::new(); blocks];
    for node in &all {
        created[analysis.nodes[*node].block.0 as usize].insert(*node);
    }
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); blocks];
    for &id in &doms.rpo {
        for target in func.blocks[id.0 as usize].term.successors() {
            preds[target.0 as usize].push(id.0 as usize);
        }
    }
    let mut live_in: Vec<BTreeSet<NodeId>> = vec![all; blocks];
    live_in[0] = BTreeSet::new();
    loop {
        let mut changed = false;
        for &id in &doms.rpo {
            let index = id.0 as usize;
            if index == 0 {
                continue;
            }
            let mut next: Option<BTreeSet<NodeId>> = None;
            for pred in &preds[index] {
                let out: BTreeSet<NodeId> =
                    live_in[*pred].union(&created[*pred]).copied().collect();
                next = Some(match next {
                    None => out,
                    Some(current) => current.intersection(&out).copied().collect(),
                });
            }
            let next = next.unwrap_or_default();
            if next != live_in[index] {
                live_in[index] = next;
                changed = true;
            }
        }
        if !changed {
            return live_in;
        }
    }
}

struct Rewriter<'a> {
    old: &'a Func,
    table: &'a TypeTable,
    analysis: &'a Analysis,
    doms: &'a Dominators,
    builder: FuncBuilder,
    /// Old value to new value, for every value that survives.
    map: BTreeMap<ValueId, ValueId>,
    /// Explicit carriers of retained values and generated field parameters. Other generated
    /// scalar operations use the builder's baseline carrier.
    carriers: BTreeMap<ValueId, Carrier>,
    /// Per block, the field values of every live node on exit.
    exit: Vec<State>,
    /// Per block, the merged nodes it takes parameters for and which of their fields are passed on
    /// every incoming edge.
    merged: Vec<Vec<(NodeId, Vec<bool>)>>,
    live_in: Vec<BTreeSet<NodeId>>,
    /// The bit pattern of every scalar constant the rewrite has a *new* value for, which is what
    /// lets a `[u8; N]` of literals become one literal rather than a chain of shifts.
    constants: BTreeMap<ValueId, u128>,
    /// Per predecessor block, the real object built for a virtual node one of its edges passes
    /// into a real parameter — one `Aggregate` per block however many edges share it.
    materialized: BTreeMap<(RBlockId, NodeId), ValueId>,
    params_added: u32,
}

/// One field of a live node: an ordinary value, or another node this one holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FieldVal {
    Value(ValueId),
    Node(NodeId),
}

impl FieldVal {
    /// The value this field is, when it is one. A nested node is not a value at all, so anything
    /// that needs one — a block argument, a `Deref` result — fails the rewrite instead.
    fn value(self) -> Option<ValueId> {
        match self {
            FieldVal::Value(value) => Some(value),
            FieldVal::Node(_) => None,
        }
    }
}

/// A live node's field values in one block's state.
type State = BTreeMap<NodeId, Vec<FieldVal>>;

impl Rewriter<'_> {
    fn run(&mut self) -> Option<()> {
        for _ in 1..self.old.blocks.len() {
            self.builder.block();
        }
        for cell in &self.old.cells {
            self.builder.cell(cell.ty, cell.boxed, cell.origin);
        }
        for param in self.old.blocks[0].params.clone() {
            let def = self.old.values[param.0 as usize];
            let new = self.builder.param(def.ty, def.origin);
            self.retain(param, new);
        }
        let rpo = self.doms.rpo.clone();
        for id in &rpo {
            let mut state = self.enter(*id)?;
            self.ops(*id, &mut state)?;
            self.exit[id.0 as usize] = state;
        }
        // Terminators last: an edge needs the target's parameter layout, which is decided when
        // the target is walked, and a back edge's target is walked first.
        for id in &rpo {
            let term = self.term(*id)?;
            self.builder.terminate(*id, term);
        }
        Some(())
    }

    /// The block's parameters, and the field values every live node enters it with.
    fn enter(&mut self, id: RBlockId) -> Option<State> {
        let mut state = State::new();
        if id.0 == 0 {
            return Some(state);
        }
        let (processed, header) = self.incoming_blocks(id);
        for param in self.old.blocks[id.0 as usize].params.clone() {
            let def = self.old.values[param.0 as usize];
            match self.analysis.live_node(param) {
                Some(node) => {
                    let values = self.fresh_params(id, node, def.origin);
                    state.insert(node, values);
                }
                None => {
                    let new = self.builder.block_param(id, def.ty, def.origin);
                    self.retain(param, new);
                }
            }
        }
        if processed.is_empty() {
            return Some(state);
        }
        for node in self.live_in[id.0 as usize].clone() {
            let arity = self.analysis.arity(node);
            let origin = self.old.values[self.analysis.nodes[node].def.0 as usize].origin;
            let mut flags = Vec::with_capacity(arity);
            let mut values = Vec::with_capacity(arity);
            for field in 0..arity {
                let mut incoming = Vec::with_capacity(processed.len());
                for pred in &processed {
                    incoming.push(*self.exit[pred.0 as usize].get(&node)?.get(field)?);
                }
                let first = *incoming.first()?;
                if (!header || !self.analysis.mutated_nodes.contains(&node))
                    && incoming.iter().all(|value| *value == first)
                {
                    flags.push(false);
                    values.push(first);
                } else {
                    // A parameter carries a value; a nested node cannot be one, so a slot holding
                    // one that reaches a merge is a shape the rewrite does not model.
                    incoming
                        .iter()
                        .map(|value| value.value())
                        .collect::<Option<Vec<_>>>()?;
                    let shape = self.analysis.nodes[node].fields[field];
                    flags.push(true);
                    values.push(FieldVal::Value(self.field_param(id, shape, origin)));
                    self.params_added += 1;
                }
            }
            self.merged[id.0 as usize].push((node, flags));
            state.insert(node, values);
        }
        Some(state)
    }

    /// The reachable predecessors of `id` already walked, and whether any is a back edge — which
    /// makes `id` a loop header, whose every live field takes a parameter.
    fn incoming_blocks(&self, id: RBlockId) -> (Vec<RBlockId>, bool) {
        let order = |block: RBlockId| self.doms.order[block.0 as usize];
        let mut processed = Vec::new();
        let mut header = false;
        for &pred in &self.doms.rpo {
            if !self.old.blocks[pred.0 as usize]
                .term
                .successors()
                .contains(&id)
            {
                continue;
            }
            if order(pred) < order(id) {
                processed.push(pred);
            } else {
                header = true;
            }
        }
        (processed, header)
    }

    /// One block parameter per field of `node`.
    fn fresh_params(&mut self, id: RBlockId, node: NodeId, origin: OriginId) -> Vec<FieldVal> {
        let fields = self.analysis.nodes[node].fields.clone();
        self.params_added += fields.len() as u32;
        fields
            .into_iter()
            .map(|shape| FieldVal::Value(self.field_param(id, shape, origin)))
            .collect()
    }

    fn field_param(&mut self, id: RBlockId, shape: Shape, origin: OriginId) -> ValueId {
        let value = self.builder.block_param(id, shape.ty, origin);
        self.carriers.insert(value, shape.carrier);
        value
    }

    /// A freshly emitted retained definition keeps its selected carrier. Projection aliases
    /// bypass this method because their field's carrier is already fixed.
    fn retain(&mut self, old: ValueId, new: ValueId) {
        self.carriers
            .insert(new, self.old.values[old.0 as usize].carrier);
        self.map.insert(old, new);
    }

    fn shape(&self, value: ValueId) -> Shape {
        let mut shape = Shape::of(self.builder.func(), value);
        if let Some(carrier) = self.carriers.get(&value) {
            shape.carrier = *carrier;
        }
        shape
    }

    /// Current initialized fields in the exact shape their merge or materializer expects.
    fn field_values(&self, node: NodeId, state: &State) -> Option<Vec<ValueId>> {
        let values = state.get(&node)?;
        let fields = &self.analysis.nodes[node].fields;
        if values.len() != fields.len() {
            return None;
        }
        values
            .iter()
            .zip(fields)
            .map(|(value, shape)| {
                let value = value.value()?;
                (self.shape(value) == *shape).then_some(value)
            })
            .collect()
    }

    fn value(&self, old: ValueId) -> Option<ValueId> {
        self.map.get(&old).copied()
    }

    /// [`Rewriter::value`] over an optional operand: `None` stays `None`, and a value that did not
    /// survive fails the rewrite.
    fn optional(&self, old: Option<ValueId>) -> Option<Option<ValueId>> {
        match old {
            Some(old) => Some(Some(self.value(old)?)),
            None => Some(None),
        }
    }

    /// Walks a block's operations, updating the state and emitting what survives.
    fn ops(&mut self, id: RBlockId, state: &mut State) -> Option<()> {
        for position in 0..self.old.blocks[id.0 as usize].ops.len() {
            let op = self.old.blocks[id.0 as usize].ops[position].clone();
            if let Some(result) = op.result {
                // A definition of a virtual node: the object is never built.
                if let Some(node) = self.analysis.live_node(result) {
                    if self.define(id, node, result, &op.kind, state)? {
                        continue;
                    }
                }
                // A view of a node that stayed virtual is not a value at all. A view of one that
                // escaped is an ordinary reference to an ordinary object, and is emitted as it
                // stands — dropping it would leave its uses with nothing to read.
                if self.analysis.live_view(result).is_some() {
                    continue;
                }
            }
            if self.consume(id, &op, state)? {
                continue;
            }
            let mut kind = op.kind.clone();
            if let OpKind::Aggregate { fields, .. } = &mut kind {
                for field in fields.iter_mut() {
                    if let Some(new) = self.map.get(field).copied() {
                        *field = new;
                    } else {
                        let child = self.analysis.live_node(*field)?;
                        *field = self.materialize_at(id, child, state)?;
                    }
                }
            } else {
                self.remap(&mut kind)?;
            }
            match op.result {
                Some(result) => {
                    let ty = self.old.values[result.0 as usize].ty;
                    let bits = match kind {
                        OpKind::Const(ConstTree::Scalar { bits, .. }) => Some(bits),
                        _ => None,
                    };
                    let new = self.builder.op(id, kind, ty, op.origin);
                    if let Some(bits) = bits {
                        self.constants.insert(new, bits);
                    }
                    self.retain(result, new);
                }
                None => self.builder.effect(id, kind, op.origin),
            }
        }
        Some(())
    }

    fn emit_const(
        &mut self,
        id: RBlockId,
        tree: ConstTree,
        ty: TypeId,
        origin: OriginId,
    ) -> ValueId {
        let bits = match tree {
            ConstTree::Scalar { bits, .. } => Some(bits),
            _ => None,
        };
        let value = self.builder.op(id, OpKind::Const(tree), ty, origin);
        if let Some(bits) = bits {
            self.constants.insert(value, bits);
        }
        value
    }

    fn define_enum_slots(
        &mut self,
        id: RBlockId,
        info: &EnumInfo,
        active_variant: u32,
        active_values: Vec<ValueId>,
        origin: OriginId,
    ) -> Option<Vec<FieldVal>> {
        let active = active_variant as usize;
        let vinfo = info.variants.get(active)?;
        if active_values.len() != vinfo.fields.len() {
            return None;
        }
        let mut out =
            Vec::with_capacity(1 + info.variants.iter().map(|v| v.fields.len()).sum::<usize>());
        let tag_tree = ConstTree::Scalar {
            bits: mask(vinfo.logical_discr as u128, info.discr_bytes),
            bytes: info.discr_bytes,
        };
        out.push(FieldVal::Value(self.emit_const(
            id,
            tag_tree,
            info.discr_ty,
            origin,
        )));
        for (idx, variant) in info.variants.iter().enumerate() {
            if idx == active {
                out.extend(active_values.iter().copied().map(FieldVal::Value));
            } else {
                for &field_ty in &variant.fields {
                    let default_tree = default_const_tree(self.table, field_ty)?;
                    let val = self.emit_const(id, default_tree, field_ty, origin);
                    out.push(FieldVal::Value(val));
                }
            }
        }
        Some(out)
    }

    /// A node's creation site. Returns whether the op was consumed.
    fn define(
        &mut self,
        id: RBlockId,
        node: NodeId,
        result: ValueId,
        kind: &OpKind,
        state: &mut State,
    ) -> Option<bool> {
        let origin = self.old.values[result.0 as usize].origin;
        let fields = self.analysis.nodes[node].fields.clone();
        let enum_info = self.analysis.nodes[node].enum_info.clone();
        let deref_from = self.analysis.nodes[node].deref_from;
        let values: Vec<FieldVal> = match kind {
            OpKind::Aggregate {
                variant, fields, ..
            } => {
                if let Some(info) = enum_info.as_ref() {
                    let mapped = fields
                        .iter()
                        .map(|f| self.value(*f))
                        .collect::<Option<Vec<_>>>()?;
                    self.define_enum_slots(id, info, *variant, mapped, origin)?
                } else {
                    fields
                        .iter()
                        .enumerate()
                        .map(|(index, field)| {
                            // A field the analysis pinned to a node is that node, already in the state.
                            match self.analysis.nested.get(&(node, index as u32)) {
                                Some(&child) if self.analysis.live_node(*field) == Some(child) => {
                                    Some(FieldVal::Node(child))
                                }
                                _ => self.value(*field).map(FieldVal::Value),
                            }
                        })
                        .collect::<Option<_>>()?
                }
            }
            OpKind::Const(tree) => {
                let (variant, trees) = match tree {
                    ConstTree::Aggregate(trees) => (0, trees.clone()),
                    ConstTree::Variant { variant, fields } => (*variant, fields.clone()),
                    _ => return Some(false),
                };
                if let Some(info) = enum_info.as_ref() {
                    let vinfo = info.variants.get(variant as usize)?;
                    if trees.len() != vinfo.fields.len() {
                        return None;
                    }
                    let active_values: Vec<ValueId> = trees
                        .into_iter()
                        .zip(&vinfo.fields)
                        .map(|(t, &ty)| self.emit_const(id, t, ty, origin))
                        .collect();
                    self.define_enum_slots(id, info, variant, active_values, origin)?
                } else {
                    trees
                        .into_iter()
                        .zip(&fields)
                        .map(|(tree, shape)| {
                            FieldVal::Value(self.emit_const(id, tree, shape.ty, origin))
                        })
                        .collect()
                }
            }
            // A copy of a node that is still virtual shares its field values; a copy of one that
            // escaped reads them back out of the real object.
            OpKind::Copy(source) => match self.analysis.live_node(*source) {
                Some(from) => state.get(&from)?.clone(),
                None => {
                    if enum_info.is_some() {
                        return None;
                    }
                    let base = self.value(*source)?;
                    let downcast = self.analysis.nodes[node].downcast();
                    fields
                        .iter()
                        .enumerate()
                        .map(|(field, shape)| {
                            let read = self.builder.op(
                                id,
                                OpKind::Field {
                                    base,
                                    variant: downcast,
                                    field: field as u32,
                                },
                                shape.ty,
                                origin,
                            );
                            // The aggregate `Copy` cloned this field even when its JavaScript
                            // representation has identity (fat pointers and nested products).
                            let copied = self.builder.op(id, OpKind::Copy(read), shape.ty, origin);
                            FieldVal::Value(copied)
                        })
                        .collect()
                }
            },
            OpKind::Deref(_) if deref_from.is_some() => state.get(&deref_from?)?.clone(),
            // A move renames, a deref of a whole-object view reads it, and a `Field` naming a
            // nested node names the node: either way it is already in the state under this id.
            OpKind::Move(_) | OpKind::Deref(_) => return Some(true),
            OpKind::Field { base, field, .. } => {
                let base = self.analysis.live_node(*base)?;
                let slot = *state.get(&base)?.get(*field as usize)?;
                return (slot == FieldVal::Node(node)).then_some(true);
            }
            _ => return Some(false),
        };
        if values.len() != fields.len() {
            return None;
        }
        for (value, shape) in values.iter().zip(&fields) {
            let actual = match value {
                FieldVal::Value(value) => self.shape(*value),
                FieldVal::Node(node) => self.analysis.nodes[*node].shape,
            };
            if actual != *shape {
                return None;
            }
        }
        state.insert(node, values);
        Some(true)
    }

    /// An op that reads or writes a virtual node. Returns whether it was consumed.
    fn consume(&mut self, id: RBlockId, op: &Op, state: &mut State) -> Option<bool> {
        match &op.kind {
            OpKind::Field {
                base,
                variant,
                field,
            } => {
                let Some(node) = self.analysis.live_node(*base) else {
                    return Some(false);
                };
                let slot = self.analysis.nodes[node].slot_of(*variant, *field)?;
                let value = state.get(&node)?.get(slot)?.value()?;
                self.map.insert(op.result?, value);
            }
            OpKind::SetField {
                base,
                variant,
                field,
                v,
            } => {
                let Some(node) = self.analysis.live_node(*base) else {
                    return Some(false);
                };
                let slot = self.analysis.nodes[node].slot_of(*variant, *field)?;
                let value = FieldVal::Value(self.value(*v)?);
                *state.get_mut(&node)?.get_mut(slot)? = value;
            }
            OpKind::Deref(pointer) => {
                let Some((node, Some(field))) = self.analysis.live_view(*pointer) else {
                    return Some(false);
                };
                let value = state.get(&node)?.get(field as usize)?.value()?;
                self.map.insert(op.result?, value);
            }
            OpKind::StoreThrough { ptr, v } => match self.analysis.live_view(*ptr) {
                Some((node, Some(field))) => {
                    let value = FieldVal::Value(self.value(*v)?);
                    *state.get_mut(&node)?.get_mut(field as usize)? = value;
                }
                Some((target_node, None)) => {
                    let Some(src_node) = self.analysis.live_node(*v) else {
                        return Some(false);
                    };
                    if self.analysis.nodes[target_node].enum_info.is_none()
                        || !self.analysis.same_shape(target_node, src_node)
                    {
                        return Some(false);
                    }
                    let new_slots = state.get(&src_node)?.clone();
                    *state.get_mut(&target_node)? = new_slots;
                }
                None => return Some(false),
            },
            OpKind::Discriminant(base) => {
                let Some(node) = self.analysis.live_node(*base) else {
                    return Some(false);
                };
                let result = op.result?;
                let ty = self.old.values[result.0 as usize].ty;
                if let Some(info) = self.analysis.nodes[node].enum_info.clone() {
                    let tag = state.get(&node)?.first()?.value()?;
                    let out = if ty == info.discr_ty {
                        tag
                    } else {
                        self.table.int_info(ty)?;
                        self.builder.op(
                            id,
                            OpKind::Cast {
                                kind: CastKind::IntToInt,
                                v: tag,
                                from: info.discr_ty,
                                to: ty,
                            },
                            ty,
                            op.origin,
                        )
                    };
                    self.map.insert(result, out);
                } else {
                    let (_, bits) = self.table.int_info(ty)?;
                    let (node_ty, variant) = {
                        let node = &self.analysis.nodes[node];
                        (node.shape.ty, node.variant)
                    };
                    let discriminant = self
                        .table
                        .variants(node_ty)
                        .get(variant as usize)?
                        .logical_discr;
                    let bytes = (bits / 8) as u8;
                    let kind = OpKind::Const(ConstTree::Scalar {
                        bits: mask(discriminant as u128, bytes),
                        bytes,
                    });
                    let new = self.builder.op(id, kind, ty, op.origin);
                    self.map.insert(result, new);
                }
            }
            OpKind::SetDiscriminant { base, variant } => {
                let Some(node) = self.analysis.live_node(*base) else {
                    return Some(false);
                };
                let Some(info) = self.analysis.nodes[node].enum_info.clone() else {
                    return Some(false);
                };
                let discr = info.variants.get(*variant as usize)?.logical_discr;
                let tag_tree = ConstTree::Scalar {
                    bits: mask(discr as u128, info.discr_bytes),
                    bytes: info.discr_bytes,
                };
                let new_tag = self.emit_const(id, tag_tree, info.discr_ty, op.origin);
                *state.get_mut(&node)?.get_mut(0)? = FieldVal::Value(new_tag);
            }
            OpKind::Cast {
                kind: CastKind::Transmute,
                v,
                to,
                ..
            } => {
                let Some(node) = self.analysis.live_node(*v) else {
                    return Some(false);
                };
                let ty = self.analysis.nodes[node].shape.ty;
                let Some((element, bits)) = candidates::byte_array_transmute(self.table, ty, *to)
                else {
                    return Some(false);
                };
                let bytes: Vec<ValueId> = state
                    .get(&node)?
                    .iter()
                    .map(|field| field.value())
                    .collect::<Option<_>>()?;
                let result = op.result?;
                let value = self.bytes_to_int(id, &bytes, (element, *to, bits), op.origin)?;
                self.map.insert(result, value);
            }
            _ => return Some(false),
        }
        Some(true)
    }

    /// `bytes[0] | bytes[1] << 8 | ..`, the chain `js/unsize.rs::bytes_to_int` prints for a real
    /// array, over the values that array would have held.
    fn bytes_to_int(
        &mut self,
        id: RBlockId,
        bytes: &[ValueId],
        types: (TypeId, TypeId, u32),
        origin: OriginId,
    ) -> Option<ValueId> {
        let (element, to, bits) = types;
        let width = (bits / 8) as usize;
        // Every byte a literal: the whole chain is one literal, which is what v1's `sroa` did for
        // `u32::from_le_bytes([..])` over a constant array.
        let literals: Option<Vec<u128>> = bytes
            .get(..width)?
            .iter()
            .map(|byte| self.constants.get(byte).copied())
            .collect();
        if let Some(literals) = literals {
            let folded = literals
                .iter()
                .enumerate()
                .fold(0u128, |acc, (index, byte)| {
                    acc | (mask(*byte, 1) << (index * 8))
                });
            let kind = OpKind::Const(ConstTree::Scalar {
                bits: folded,
                bytes: width as u8,
            });
            let value = self.builder.op(id, kind, to, origin);
            self.constants.insert(value, folded);
            return Some(value);
        }
        let widen = |rewriter: &mut Self, byte: ValueId| {
            rewriter.builder.op(
                id,
                OpKind::Cast {
                    kind: CastKind::IntToInt,
                    v: byte,
                    from: element,
                    to,
                },
                to,
                origin,
            )
        };
        // `bytes` already holds *new* value ids: the state is built as the rewrite goes.
        let first = *bytes.first()?;
        let mut combined = widen(self, first);
        for index in 1..(bits / 8) as usize {
            let byte = *bytes.get(index)?;
            let wide = widen(self, byte);
            // The shift amount is spelled at the shifted type, so a `BigInt` target gets a
            // `BigInt` literal without this pass having to find a `u32` in the table.
            let amount = self.builder.op(
                id,
                OpKind::Const(ConstTree::Scalar {
                    bits: (index * 8) as u128,
                    bytes: (bits / 8) as u8,
                }),
                to,
                origin,
            );
            let shifted = self.builder.op(
                id,
                OpKind::Binary {
                    op: BinaryOp::Shl,
                    l: wide,
                    r: amount,
                    ty: to,
                },
                to,
                origin,
            );
            combined = self.builder.op(
                id,
                OpKind::Binary {
                    op: BinaryOp::BitOr,
                    l: combined,
                    r: shifted,
                    ty: to,
                },
                to,
                origin,
            );
        }
        Some(combined)
    }

    fn term(&mut self, id: RBlockId) -> Option<RTerm> {
        Some(match self.old.blocks[id.0 as usize].term.clone() {
            RTerm::Goto(target, args) => RTerm::Goto(target, self.edge(id, target, &args)?),
            RTerm::Switch {
                scrut,
                scrut_ty,
                cases,
                default,
            } => RTerm::Switch {
                scrut: self.value(scrut)?,
                scrut_ty,
                cases: cases
                    .iter()
                    .map(|(case, target, args)| {
                        Some((*case, *target, self.edge(id, *target, args)?))
                    })
                    .collect::<Option<_>>()?,
                default: (default.0, self.edge(id, default.0, &default.1)?),
            },
            RTerm::Invoke { .. } => return None,
            RTerm::Return(v) => RTerm::Return(match v {
                Some(old) => Some(if let Some(new) = self.value(old) {
                    new
                } else {
                    let node = self.analysis.live_node(old)?;
                    self.materialize(id, node)?
                }),
                None => None,
            }),
            RTerm::Throw(v) => RTerm::Throw(self.optional(v)?),
            RTerm::Unreachable => RTerm::Unreachable,
        })
    }

    /// The arguments one edge passes: the surviving originals, then the merged nodes' fields.
    ///
    /// A virtual node passed into a parameter that is *not* virtual — escaped for its own
    /// reasons, of another shape, or no node at all — is materialized here rather than escaping
    /// everywhere: the edge gets a real object built from the node's exit-state fields.
    fn edge(&mut self, from: RBlockId, to: RBlockId, args: &[ValueId]) -> Option<Vec<ValueId>> {
        let mut out = Vec::with_capacity(args.len());
        for (position, param) in self.old.blocks[to.0 as usize].params.iter().enumerate() {
            let arg = *args.get(position)?;
            match self.analysis.live_node(*param) {
                Some(target) => {
                    let node = self.analysis.live_node(arg)?;
                    if !self.analysis.same_shape(node, target) {
                        return None;
                    }
                    out.extend(self.field_values(node, &self.exit[from.0 as usize])?);
                }
                None => match self.analysis.live_node(arg) {
                    Some(node) => {
                        if self.analysis.nodes[node].shape != Shape::of(self.old, *param) {
                            return None;
                        }
                        out.push(self.materialize(from, node)?);
                    }
                    None => out.push(self.value(arg)?),
                },
            }
        }
        for (node, flags) in self.merged[to.0 as usize].clone() {
            let values = self.exit[from.0 as usize].get(&node)?;
            for (field, passed) in flags.iter().enumerate() {
                if *passed {
                    let value = values.get(field)?.value()?;
                    if self.shape(value) != self.analysis.nodes[node].fields[field] {
                        return None;
                    }
                    out.push(value);
                }
            }
        }
        Some(out)
    }

    /// The real object for a virtual node an edge passes into a real parameter: an `Aggregate`
    /// over the node's exit-state field values, built once per predecessor block.
    ///
    /// The node itself stays virtual. This is a by-value snapshot at the edge — exactly what the
    /// per-field parameters of a virtual merge already pass — boxed because the consumer needs a
    /// real object. Its fields are always plain values: a node used as a block argument has its
    /// slots unpinned during the analysis, so a nested node never sits in one (the `?` on
    /// `value()` is the backstop, and fails the rewrite rather than guessing).
    fn materialize(&mut self, from: RBlockId, node: NodeId) -> Option<ValueId> {
        if let Some(&value) = self.materialized.get(&(from, node)) {
            return Some(value);
        }
        let fields = self.field_values(node, &self.exit[from.0 as usize])?;
        let info = &self.analysis.nodes[node];
        // Aggregate construction implements the baseline layout. A different outer carrier
        // needs its own constructor or a proven explicit adapter before it can be virtualized.
        if info.shape.carrier != Carrier::Baseline {
            return None;
        }
        let (variant, fields) = if let Some(ei) = &info.enum_info {
            let tag_val = *fields.first()?;
            let tag_bits = self.constants.get(&tag_val).copied()?;
            let mut found = None;
            for (v_idx, vinfo) in ei.variants.iter().enumerate() {
                if mask(vinfo.logical_discr as u128, ei.discr_bytes) == tag_bits {
                    if found.is_some() {
                        return None;
                    }
                    found = Some((
                        v_idx as u32,
                        fields[vinfo.slot_offset..vinfo.slot_offset + vinfo.fields.len()].to_vec(),
                    ));
                }
            }
            found?
        } else {
            (info.variant, fields)
        };
        let kind = OpKind::Aggregate {
            ty: info.shape.ty,
            variant,
            active_field: None,
            fields,
        };
        let def = self.old.values[info.def.0 as usize];
        let value = self.builder.op(from, kind, def.ty, def.origin);
        self.materialized.insert((from, node), value);
        Some(value)
    }

    /// Materializes a virtual single-variant baseline node from the current mid-block `state`
    /// right before an escaping `OpKind::Aggregate`.
    fn materialize_at(&mut self, block: RBlockId, node: NodeId, state: &State) -> Option<ValueId> {
        let fields = self.field_values(node, state)?;
        let info = &self.analysis.nodes[node];
        if info.shape.carrier != Carrier::Baseline || info.enum_info.is_some() {
            return None;
        }
        let kind = OpKind::Aggregate {
            ty: info.shape.ty,
            variant: info.variant,
            active_field: None,
            fields,
        };
        let def = self.old.values[info.def.0 as usize];
        Some(self.builder.op(block, kind, def.ty, def.origin))
    }

    /// Rewrites every value an op reads to its new id. Mirrors `OpKind::for_each_use`.
    fn remap(&self, kind: &mut OpKind) -> Option<()> {
        if !kind.uses().iter().all(|v| self.map.contains_key(v)) {
            return None;
        }
        let map = &self.map;
        let mut one = |v: &mut ValueId| {
            if let Some(new) = map.get(v) {
                *v = *new;
            }
        };
        let f: &mut dyn FnMut(&mut ValueId) = &mut one;
        let path = |path: &mut Vec<Proj>, f: &mut dyn FnMut(&mut ValueId)| {
            for proj in path {
                if let Proj::Index(v) = proj {
                    f(v);
                }
            }
        };
        let root = |root: &mut Root, f: &mut dyn FnMut(&mut ValueId)| {
            if let Root::Value(v) = root {
                f(v);
            }
        };
        match kind {
            OpKind::Const(_)
            | OpKind::CellAlloc(_)
            | OpKind::CellLoad(_)
            | OpKind::Zombie(_)
            | OpKind::SpillRead(_) => {}
            OpKind::Copy(v)
            | OpKind::Move(v)
            | OpKind::Repeat { v, .. }
            | OpKind::Field { base: v, .. }
            | OpKind::Discriminant(v)
            | OpKind::SetDiscriminant { base: v, .. }
            | OpKind::CellStore { v, .. }
            | OpKind::SpillWrite { v, .. }
            | OpKind::Deref(v)
            | OpKind::Unary { v, .. }
            | OpKind::Cast { v, .. }
            | OpKind::Materialize(v)
            | OpKind::Convert { v, .. } => f(v),
            OpKind::Aggregate { fields, .. } => fields.iter_mut().for_each(&mut *f),
            OpKind::SetField { base, v, .. } => {
                f(base);
                f(v);
            }
            OpKind::BorrowView {
                root: r, path: p, ..
            }
            | OpKind::RawView {
                root: r, path: p, ..
            }
            | OpKind::Drop {
                root: r, path: p, ..
            } => {
                root(r, &mut *f);
                path(p, &mut *f);
            }
            OpKind::SliceView { ptr, len } => {
                f(ptr);
                f(len);
            }
            OpKind::StoreThrough { ptr, v } => {
                f(ptr);
                f(v);
            }
            OpKind::Binary { l, r, .. } => {
                f(l);
                f(r);
            }
            OpKind::Call { function, args, .. } => {
                if let Some(function) = function {
                    f(function);
                }
                args.iter_mut().for_each(&mut *f);
            }
            OpKind::Intrinsic { args, .. } | OpKind::Host { args, .. } => {
                args.iter_mut().for_each(&mut *f)
            }
            OpKind::Assert { cond, msg, .. } => {
                f(cond);
                remap_message(msg, f);
            }
        }
        Some(())
    }
}

fn remap_message(msg: &mut RAssertMessage, f: &mut dyn FnMut(&mut ValueId)) {
    match msg {
        RAssertMessage::BoundsCheck {
            length: a,
            index: b,
        }
        | RAssertMessage::Overflow {
            left: a, right: b, ..
        }
        | RAssertMessage::MisalignedPointer {
            required: a,
            found: b,
        } => {
            f(a);
            f(b);
        }
        RAssertMessage::OverflowNeg(v)
        | RAssertMessage::DivisionByZero(v)
        | RAssertMessage::RemainderByZero(v)
        | RAssertMessage::InvalidEnum(v) => f(v),
        RAssertMessage::ResumedAfterReturn(_)
        | RAssertMessage::ResumedAfterPanic(_)
        | RAssertMessage::ResumedAfterDrop(_)
        | RAssertMessage::NullPointer
        | RAssertMessage::NullReference => {}
    }
}

/// `bits` truncated to `bytes` bytes.
fn mask(bits: u128, bytes: u8) -> u128 {
    match bytes {
        0 => 0,
        1..=15 => bits & ((1u128 << (u32::from(bytes) * 8)) - 1),
        _ => bits,
    }
}

/// A reference to an object that escapes is still a reference to an object.
///
/// Dropping a view because the analysis *recorded* it — rather than because its node stayed
/// virtual — leaves the uses of that view with nothing to read, and the whole rewrite gives up. One
/// `&mut self` call would then cost every other aggregate in the same function its scalarization.
#[cfg(test)]
mod unit {
    use super::*;
    use crate::jir::repr::{FuncBuilder, RTerm};
    use crate::jir::semantic::{
        AbiSignature, CallSemantic, Callee, DefinitionId, ItemId, Signature,
    };
    use crate::jir::types::{
        FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts, NominalForm,
        TypeDescriptor, TypeKind, TypeMeta, VariantDescriptor,
    };
    use crate::jir::{OriginId, StrId};

    const I32: TypeId = TypeId(1);
    const PAIR: TypeId = TypeId(2);
    const REF: TypeId = TypeId(3);
    const SLICE: TypeId = TypeId(4);
    const VIEW: TypeId = TypeId(5);
    const NO_ORIGIN: OriginId = OriginId(0);

    fn js(peeled: TypeId, fields: Vec<FieldDescriptor>, needs_clone: bool) -> JsFacts {
        JsFacts {
            int: None,
            is_indirect: needs_clone,
            needs_clone,
            crosses_transparent_union: false,
            transparent: None,
            peeled,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: false,
            fields,
        }
    }

    fn descriptor(id: TypeId, kind: TypeKind, js: JsFacts) -> TypeDescriptor {
        TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js,
        }
    }

    /// `i32`, a `Pair { a: i32, b: i32 }`, and `&mut Pair`.
    fn table() -> TypeTable {
        let field = |index: u32| FieldDescriptor {
            key: FieldKey::Index(index),
            ty: I32,
            offset: 0,
        };
        let mut table = TypeTable::new();
        let mut int = js(I32, Vec::new(), false);
        int.int = Some(IntFacts {
            signed: true,
            bits: 32,
            repr: IntRepr::Number,
        });
        table.insert(descriptor(
            I32,
            TypeKind::Integer {
                signed: true,
                bits: 32,
            },
            int,
        ));
        table.insert(descriptor(
            PAIR,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: 2,
                },
                args: Vec::new(),
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: StrId(0),
                    fields: vec![field(0), field(1)],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            js(PAIR, vec![field(0), field(1)], true),
        ));
        table.insert(descriptor(
            REF,
            TypeKind::Reference {
                pointee: PAIR,
                mutable: true,
            },
            js(REF, Vec::new(), false),
        ));
        table
    }

    fn slice_table() -> TypeTable {
        let mut table = table();
        let mut slice = descriptor(SLICE, TypeKind::Slice(I32), js(SLICE, Vec::new(), true));
        slice.layout.sized = false;
        table.insert(slice);
        let mut view = descriptor(
            VIEW,
            TypeKind::Reference {
                pointee: SLICE,
                mutable: false,
            },
            js(VIEW, Vec::new(), true),
        );
        view.js.pointer_form = Some(crate::jir::types::PointerForm::FatSlice);
        table.insert(view);
        table
    }

    #[test]
    fn unsupported_field_carriers_only_pin_their_own_object() {
        const TUPLE: TypeId = TypeId(6);
        const BORROW: TypeId = TypeId(7);
        let mut table = slice_table();
        table.insert(descriptor(
            TUPLE,
            TypeKind::Tuple(vec![VIEW]),
            js(
                TUPLE,
                vec![FieldDescriptor {
                    key: FieldKey::Index(0),
                    ty: VIEW,
                    offset: 0,
                }],
                true,
            ),
        ));
        table.insert(descriptor(
            BORROW,
            TypeKind::Reference {
                pointee: VIEW,
                mutable: true,
            },
            js(BORROW, Vec::new(), false),
        ));
        for access in ["aggregate", "field", "set-field", "deref", "store-through"] {
            let mut b = FuncBuilder::new(
                ItemId {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 0,
                    },
                    instantiation: 0,
                },
                Signature {
                    abi: AbiSignature {
                        parameters: 2,
                        spread: None,
                        caller_location: false,
                        inline_never: false,
                        inline_hint: false,
                        inline_always: false,
                    },
                    arguments: vec![VIEW, VIEW],
                    result: I32,
                },
            );
            let entry = FuncBuilder::ENTRY;
            let baseline = b.param(VIEW, OriginId(1));
            let native = b.param(VIEW, OriginId(2));
            let tuple = b.op(
                entry,
                OpKind::Aggregate {
                    ty: TUPLE,
                    variant: 0,
                    active_field: None,
                    fields: vec![if access == "aggregate" {
                        native
                    } else {
                        baseline
                    }],
                },
                TUPLE,
                OriginId(3),
            );
            let mut native_read = None;
            match access {
                "aggregate" | "field" => {
                    native_read = Some(b.op(
                        entry,
                        OpKind::Field {
                            base: tuple,
                            variant: None,
                            field: 0,
                        },
                        VIEW,
                        OriginId(4),
                    ));
                }
                "set-field" => b.effect(
                    entry,
                    OpKind::SetField {
                        base: tuple,
                        variant: None,
                        field: 0,
                        v: native,
                    },
                    OriginId(4),
                ),
                _ => {
                    let ptr = b.op(
                        entry,
                        OpKind::BorrowView {
                            root: Root::Value(tuple),
                            path: vec![Proj::Field { field: 0, ty: VIEW }],
                            mutable: true,
                        },
                        BORROW,
                        OriginId(4),
                    );
                    if access == "deref" {
                        native_read = Some(b.op(entry, OpKind::Deref(ptr), VIEW, OriginId(5)));
                    } else {
                        b.effect(entry, OpKind::StoreThrough { ptr, v: native }, OriginId(5));
                    }
                }
            }
            let pair = b.op(
                entry,
                OpKind::Const(ConstTree::Aggregate(vec![
                    ConstTree::Scalar { bits: 1, bytes: 4 },
                    ConstTree::Scalar { bits: 2, bytes: 4 },
                ])),
                PAIR,
                OriginId(6),
            );
            let result = b.op(
                entry,
                OpKind::Field {
                    base: pair,
                    variant: None,
                    field: 0,
                },
                I32,
                OriginId(7),
            );
            b.terminate(entry, RTerm::Return(Some(result)));
            let mut func = b.finish();
            for value in std::iter::once(native).chain(native_read) {
                func.values[value.0 as usize].carrier = Carrier::SliceView;
            }
            let policy = super::super::observed_carriers(&func);
            func.verify_carriers(&table, policy).unwrap();
            let stats = run(&mut func, &table);
            assert_eq!(
                stats.scalarized, 1,
                "{access}: unrelated pair must scalarize"
            );
            func.verify_carriers(&table, policy).unwrap();
            assert!(
                func.blocks[0]
                    .ops
                    .iter()
                    .any(|op| { matches!(op.kind, OpKind::Aggregate { ty: TUPLE, .. }) }),
                "{access}: unsupported field shape stays materialized"
            );
            assert_eq!(
                func.values
                    .iter()
                    .filter(|v| v.carrier == Carrier::SliceView)
                    .count(),
                1 + usize::from(native_read.is_some()),
                "{access}: retained carriers"
            );
        }
    }

    #[test]
    fn retained_carriers_survive_parameters_copies_conversions_and_edges() {
        let table = slice_table();
        let mut b = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 0,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 1,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    inline_hint: false,
                    inline_always: false,
                },
                arguments: vec![VIEW],
                result: VIEW,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let native = b.param(VIEW, OriginId(1));
        let pair = b.op(
            entry,
            OpKind::Const(ConstTree::Aggregate(vec![
                ConstTree::Scalar { bits: 1, bytes: 4 },
                ConstTree::Scalar { bits: 2, bytes: 4 },
            ])),
            PAIR,
            NO_ORIGIN,
        );
        let condition = b.op(
            entry,
            OpKind::Field {
                base: pair,
                variant: None,
                field: 0,
            },
            I32,
            NO_ORIGIN,
        );
        let copied = b.op(entry, OpKind::Copy(native), VIEW, OriginId(2));
        let merge = b.block();
        let merged = b.block_param(merge, VIEW, OriginId(5));
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: condition,
                scrut_ty: I32,
                cases: vec![(0, merge, vec![native])],
                default: (merge, vec![copied]),
            },
        );
        let baseline = b.op(
            merge,
            OpKind::Convert {
                v: merged,
                from: Carrier::SliceView,
                to: Carrier::Baseline,
            },
            VIEW,
            OriginId(6),
        );
        b.terminate(merge, RTerm::Return(Some(baseline)));
        let mut func = b.finish();
        for value in [native, copied, merged] {
            func.values[value.0 as usize].carrier = Carrier::SliceView;
        }
        let policy = super::super::observed_carriers(&func);
        func.verify_carriers(&table, policy).unwrap();
        let retained: Vec<_> = func
            .values
            .iter()
            .filter(|def| def.ty == VIEW)
            .copied()
            .collect();
        let stats = run(&mut func, &table);
        assert_eq!(
            stats.scalarized, 1,
            "the unrelated baseline pair disappears"
        );
        func.verify_carriers(&table, policy).unwrap();
        assert_eq!(
            func.values
                .iter()
                .filter(|def| def.ty == VIEW)
                .copied()
                .collect::<Vec<_>>(),
            retained,
            "retained values keep their carriers and origins even when IDs change",
        );
        assert_eq!(
            func.blocks
                .iter()
                .flat_map(|block| &block.ops)
                .filter(|op| matches!(op.kind, OpKind::Convert { .. }))
                .count(),
            1,
        );
    }

    /// A mutable local copy of a product returned by a call can keep only copied field values.
    #[test]
    fn a_call_result_copy_is_scalarized_without_aliasing_its_source() {
        let mut b = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 0,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 0,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    inline_hint: false,
                    inline_always: false,
                },
                arguments: Vec::new(),
                result: I32,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let returned = b.op(
            entry,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 0,
                            local_hash: 1,
                        },
                        instantiation: 0,
                    },
                    link_name: StrId(0),
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            PAIR,
            NO_ORIGIN,
        );
        let local = b.op(entry, OpKind::Copy(returned), PAIR, NO_ORIGIN);
        let seven = b.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 7, bytes: 4 }),
            I32,
            NO_ORIGIN,
        );
        b.effect(
            entry,
            OpKind::SetField {
                base: local,
                variant: None,
                field: 0,
                v: seven,
            },
            NO_ORIGIN,
        );
        let second = b.op(
            entry,
            OpKind::Field {
                base: local,
                variant: None,
                field: 1,
            },
            I32,
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(Some(second)));
        let mut func = b.finish();

        let table = table();
        let stats = run(&mut func, &table);
        assert_eq!(stats.objects, 1);
        assert_eq!(stats.scalarized, 1);
        assert_eq!(stats.escaped, 0);
        func.verify(&table).expect("scalarized call copy verifies");
        assert!(!func.blocks[0].ops.iter().any(|op| {
            op.result
                .is_some_and(|result| func.values[result.0 as usize].ty == PAIR)
                && matches!(op.kind, OpKind::Copy(_))
        }));
        assert!(func.blocks[0].ops.iter().any(|op| {
            matches!(
                op.kind,
                OpKind::Field {
                    base,
                    variant: None,
                    ..
                } if base == returned
            )
        }));
    }

    #[test]
    fn a_reference_to_an_escaping_object_does_not_cost_the_others_their_fields() {
        let mut b = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 0,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 0,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    inline_hint: false,
                    inline_always: false,
                },
                arguments: Vec::new(),
                result: I32,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let scalar = |b: &mut FuncBuilder, value: u128| {
            b.op(
                entry,
                OpKind::Const(ConstTree::Scalar {
                    bits: value,
                    bytes: 4,
                }),
                I32,
                NO_ORIGIN,
            )
        };
        let aggregate = |b: &mut FuncBuilder, fields: Vec<ValueId>| {
            b.op(
                entry,
                OpKind::Aggregate {
                    ty: PAIR,
                    variant: 0,
                    active_field: None,
                    fields,
                },
                PAIR,
                NO_ORIGIN,
            )
        };
        // `escaping` is handed to a callee by reference, so it has to be a real object.
        let one = scalar(&mut b, 1);
        let two = scalar(&mut b, 2);
        let escaping = aggregate(&mut b, vec![one, two]);
        let view = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(escaping),
                path: Vec::new(),
                mutable: true,
            },
            REF,
            NO_ORIGIN,
        );
        b.effect(
            entry,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 0,
                            local_hash: 1,
                        },
                        instantiation: 0,
                    },
                    link_name: StrId(0),
                },
                function: None,
                args: vec![view],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
        // `virtual` has nothing to do with it, and only ever has one field read.
        let virtual_pair = aggregate(&mut b, vec![two, one]);
        let read = b.op(
            entry,
            OpKind::Field {
                base: virtual_pair,
                variant: None,
                field: 0,
            },
            I32,
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(Some(read)));
        let mut func = b.finish();

        let table = table();
        let stats = run(&mut func, &table);
        assert_eq!(stats.objects, 2);
        assert_eq!(stats.escaped, 1);
        assert_eq!(stats.scalarized, 1, "the unrelated object keeps its fields");
        assert!(func.verify(&table).is_ok());
        assert!(
            func.blocks[0]
                .ops
                .iter()
                .any(|op| matches!(op.kind, OpKind::BorrowView { .. })),
            "the escaping object's reference is still taken"
        );
    }

    /// A merge with mixed states: one edge passes a caller value, the other a virtual node. The
    /// parameter is real, and the node is materialized on its one edge instead of escaping.
    #[test]
    fn a_mixed_merge_materializes_on_the_edge_instead_of_escaping() {
        let mut b = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 0,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 1,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    inline_hint: false,
                    inline_always: false,
                },
                arguments: vec![PAIR],
                result: I32,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let from_caller = b.param(PAIR, NO_ORIGIN);
        let left = b.block();
        let right = b.block();
        let merge = b.block();
        let cond = b.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            I32,
            NO_ORIGIN,
        );
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: cond,
                scrut_ty: I32,
                cases: vec![(0, left, Vec::new())],
                default: (right, Vec::new()),
            },
        );
        b.terminate(left, RTerm::Goto(merge, vec![from_caller]));
        let scalar = |b: &mut FuncBuilder, value: u128| {
            b.op(
                right,
                OpKind::Const(ConstTree::Scalar {
                    bits: value,
                    bytes: 4,
                }),
                I32,
                NO_ORIGIN,
            )
        };
        let one = scalar(&mut b, 1);
        let two = scalar(&mut b, 2);
        let built = b.op(
            right,
            OpKind::Aggregate {
                ty: PAIR,
                variant: 0,
                active_field: None,
                fields: vec![one, two],
            },
            PAIR,
            NO_ORIGIN,
        );
        b.terminate(right, RTerm::Goto(merge, vec![built]));
        let merged = b.block_param(merge, PAIR, NO_ORIGIN);
        let read = b.op(
            merge,
            OpKind::Field {
                base: merged,
                variant: None,
                field: 0,
            },
            I32,
            NO_ORIGIN,
        );
        b.terminate(merge, RTerm::Return(Some(read)));
        let mut func = b.finish();

        let table = table();
        let stats = run(&mut func, &table);
        assert_eq!(stats.objects, 2, "the parameter and the built pair");
        assert_eq!(stats.escaped, 1, "only the parameter is real");
        assert_eq!(stats.scalarized, 1, "the built pair stays virtual");
        assert!(func.verify(&table).is_ok());
        assert_eq!(
            func.blocks[right.0 as usize]
                .ops
                .iter()
                .filter(|op| matches!(op.kind, OpKind::Aggregate { .. }))
                .count(),
            1,
            "the pair is built once, on the edge that needs it"
        );
        assert!(
            func.blocks[entry.0 as usize]
                .ops
                .iter()
                .all(|op| !matches!(op.kind, OpKind::Aggregate { .. })),
            "no object is built anywhere else"
        );
    }

    fn enum_state_table() -> (TypeTable, TypeId, TypeId, TypeId) {
        const STATE: TypeId = TypeId(10);
        const REF_STATE: TypeId = TypeId(11);
        const NINE_FIELD: TypeId = TypeId(12);
        let field = |index: u32| FieldDescriptor {
            key: FieldKey::Index(index),
            ty: I32,
            offset: 0,
        };
        let mut tbl = table();
        let mut state_js = js(STATE, Vec::new(), true);
        state_js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
        tbl.insert(descriptor(
            STATE,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: 10,
                },
                args: Vec::new(),
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: StrId(0),
                        fields: Vec::new(),
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: StrId(1),
                        fields: vec![field(0)],
                        logical_discr: 1,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: StrId(2),
                        fields: vec![field(0), field(1)],
                        logical_discr: 2,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: StrId(3),
                        fields: vec![field(0)],
                        logical_discr: 3,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
                niche: None,
            },
            state_js,
        ));
        tbl.insert(descriptor(
            REF_STATE,
            TypeKind::Reference {
                pointee: STATE,
                mutable: true,
            },
            js(REF_STATE, Vec::new(), false),
        ));
        let nine_fields: Vec<FieldDescriptor> = (0..9).map(field).collect();
        tbl.insert(descriptor(
            NINE_FIELD,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: 12,
                },
                args: Vec::new(),
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: StrId(0),
                    fields: nine_fields.clone(),
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            js(NINE_FIELD, nine_fields, true),
        ));
        (tbl, STATE, REF_STATE, NINE_FIELD)
    }

    #[test]
    fn nine_field_struct_is_scalarized() {
        let (table, _, _, nine_field) = enum_state_table();
        let mut b = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 20,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 1,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    inline_hint: false,
                    inline_always: false,
                },
                arguments: vec![I32],
                result: I32,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let x = b.param(I32, NO_ORIGIN);
        let fields = vec![x; 9];
        let agg = b.op(
            entry,
            OpKind::Aggregate {
                ty: nine_field,
                variant: 0,
                active_field: None,
                fields,
            },
            nine_field,
            NO_ORIGIN,
        );
        let read = b.op(
            entry,
            OpKind::Field {
                base: agg,
                variant: None,
                field: 8,
            },
            I32,
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(Some(read)));
        let mut func = b.finish();
        let stats = run(&mut func, &table);
        assert_eq!(stats.scalarized, 1);
        assert_eq!(stats.escaped, 0);
        assert!(func.verify(&table).is_ok());
        assert!(func.blocks[0]
            .ops
            .iter()
            .all(|op| !matches!(op.kind, OpKind::Aggregate { .. })));
    }

    #[test]
    fn multi_variant_enum_loop_state_scalarizes_with_borrow_store_and_discriminant() {
        let (table, state_ty, ref_state_ty, _) = enum_state_table();
        let mut b = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 21,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 1,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    inline_hint: false,
                    inline_always: false,
                },
                arguments: vec![I32],
                result: I32,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let header = b.block();
        let arm_idle = b.block();
        let arm_scan = b.block();
        let arm_other = b.block();
        let exit = b.block();

        let limit = b.param(I32, NO_ORIGIN);
        let zero = b.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            I32,
            NO_ORIGIN,
        );
        let init_state = b.op(
            entry,
            OpKind::Aggregate {
                ty: state_ty,
                variant: 0,
                active_field: None,
                fields: Vec::new(),
            },
            state_ty,
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Goto(header, vec![zero, init_state]));

        let iter_p = b.block_param(header, I32, NO_ORIGIN);
        let state_p = b.block_param(header, state_ty, NO_ORIGIN);
        let one = b.op(
            header,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            I32,
            NO_ORIGIN,
        );
        let next_i = b.op(
            header,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: iter_p,
                r: one,
                ty: I32,
            },
            I32,
            NO_ORIGIN,
        );
        let discr = b.op(header, OpKind::Discriminant(state_p), I32, NO_ORIGIN);
        b.terminate(
            header,
            RTerm::Switch {
                scrut: discr,
                scrut_ty: I32,
                cases: vec![(0, arm_idle, Vec::new()), (1, arm_scan, Vec::new())],
                default: (arm_other, Vec::new()),
            },
        );

        // Arm 0 (`Idle`): borrow `&mut state_p`, store `State::Scan(iter_p)` via `StoreThrough`.
        let r0 = b.op(
            arm_idle,
            OpKind::BorrowView {
                root: Root::Value(state_p),
                path: Vec::new(),
                mutable: true,
            },
            ref_state_ty,
            NO_ORIGIN,
        );
        let scan_val = b.op(
            arm_idle,
            OpKind::Aggregate {
                ty: state_ty,
                variant: 1,
                active_field: None,
                fields: vec![iter_p],
            },
            state_ty,
            NO_ORIGIN,
        );
        b.effect(
            arm_idle,
            OpKind::StoreThrough {
                ptr: r0,
                v: scan_val,
            },
            NO_ORIGIN,
        );
        b.terminate(arm_idle, RTerm::Goto(header, vec![next_i, state_p]));

        // Arm 1 (`Scan(pos)`): read `pos`, construct `State::Match(pos, limit)` and `Deref` check.
        let r1 = b.op(
            arm_scan,
            OpKind::BorrowView {
                root: Root::Value(state_p),
                path: Vec::new(),
                mutable: false,
            },
            ref_state_ty,
            NO_ORIGIN,
        );
        let deref1 = b.op(arm_scan, OpKind::Deref(r1), state_ty, NO_ORIGIN);
        let pos = b.op(
            arm_scan,
            OpKind::Field {
                base: deref1,
                variant: Some(1),
                field: 0,
            },
            I32,
            NO_ORIGIN,
        );
        let match_val = b.op(
            arm_scan,
            OpKind::Aggregate {
                ty: state_ty,
                variant: 2,
                active_field: None,
                fields: vec![pos, limit],
            },
            state_ty,
            NO_ORIGIN,
        );
        b.terminate(arm_scan, RTerm::Goto(header, vec![next_i, match_val]));

        // Default (`Match`/`Emit`): read `Match` field 1, `SetDiscriminant(3)` + `SetField(3, 0)` and exit!
        let m_val = b.op(
            arm_other,
            OpKind::Field {
                base: state_p,
                variant: Some(2),
                field: 1,
            },
            I32,
            NO_ORIGIN,
        );
        b.effect(
            arm_other,
            OpKind::SetDiscriminant {
                base: state_p,
                variant: 3,
            },
            NO_ORIGIN,
        );
        b.effect(
            arm_other,
            OpKind::SetField {
                base: state_p,
                variant: Some(3),
                field: 0,
                v: m_val,
            },
            NO_ORIGIN,
        );
        b.terminate(arm_other, RTerm::Goto(exit, vec![state_p]));

        let final_state = b.block_param(exit, state_ty, NO_ORIGIN);
        let final_d = b.op(exit, OpKind::Discriminant(final_state), I32, NO_ORIGIN);
        let final_f = b.op(
            exit,
            OpKind::Field {
                base: final_state,
                variant: Some(3),
                field: 0,
            },
            I32,
            NO_ORIGIN,
        );
        let out = b.op(
            exit,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: final_d,
                r: final_f,
                ty: I32,
            },
            I32,
            NO_ORIGIN,
        );
        b.terminate(exit, RTerm::Return(Some(out)));

        let mut func = b.finish();
        let stats = run(&mut func, &table);
        assert_eq!(stats.escaped, 0, "multi-variant enum loop must not escape");
        assert!(stats.scalarized >= 5);
        assert!(func.verify(&table).is_ok());
        crate::opt::dce::run(&mut func, &table);
        assert!(func.verify(&table).is_ok());
        for block in &func.blocks {
            assert!(block.ops.iter().all(|op| !matches!(
                op.kind,
                OpKind::Aggregate { .. }
                    | OpKind::BorrowView { .. }
                    | OpKind::StoreThrough { .. }
                    | OpKind::Discriminant(_)
                    | OpKind::SetDiscriminant { .. }
            )));
        }
    }

    #[test]
    fn multi_variant_enum_const_and_branch_merge_scalarizes() {
        let (table, state_ty, _, _) = enum_state_table();
        let mut b = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 22,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 1,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    inline_hint: false,
                    inline_always: false,
                },
                arguments: vec![I32],
                result: I32,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let left = b.block();
        let right = b.block();
        let merge = b.block();

        let arg = b.param(I32, NO_ORIGIN);
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: arg,
                scrut_ty: I32,
                cases: vec![(0, left, Vec::new())],
                default: (right, Vec::new()),
            },
        );

        let c_idle = b.op(
            left,
            OpKind::Const(ConstTree::Variant {
                variant: 0,
                fields: Vec::new(),
            }),
            state_ty,
            NO_ORIGIN,
        );
        b.terminate(left, RTerm::Goto(merge, vec![c_idle]));

        let v_emit = b.op(
            right,
            OpKind::Aggregate {
                ty: state_ty,
                variant: 3,
                active_field: None,
                fields: vec![arg],
            },
            state_ty,
            NO_ORIGIN,
        );
        b.terminate(right, RTerm::Goto(merge, vec![v_emit]));

        let merged = b.block_param(merge, state_ty, NO_ORIGIN);
        let discr = b.op(merge, OpKind::Discriminant(merged), I32, NO_ORIGIN);
        let payload = b.op(
            merge,
            OpKind::Field {
                base: merged,
                variant: Some(3),
                field: 0,
            },
            I32,
            NO_ORIGIN,
        );
        let sum = b.op(
            merge,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: discr,
                r: payload,
                ty: I32,
            },
            I32,
            NO_ORIGIN,
        );
        b.terminate(merge, RTerm::Return(Some(sum)));

        let mut func = b.finish();
        let stats = run(&mut func, &table);
        assert_eq!(stats.escaped, 0);
        assert_eq!(stats.scalarized, 3);
        assert!(func.verify(&table).is_ok());
        crate::opt::dce::run(&mut func, &table);
        assert!(func.verify(&table).is_ok());
    }

    #[test]
    fn multi_variant_enum_escaping_through_call_falls_back_cleanly() {
        let (table, state_ty, ref_state_ty, _) = enum_state_table();
        let mut b = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 23,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 1,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    inline_hint: false,
                    inline_always: false,
                },
                arguments: vec![I32],
                result: I32,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let arg = b.param(I32, NO_ORIGIN);
        let st = b.op(
            entry,
            OpKind::Aggregate {
                ty: state_ty,
                variant: 1,
                active_field: None,
                fields: vec![arg],
            },
            state_ty,
            NO_ORIGIN,
        );
        let r = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(st),
                path: Vec::new(),
                mutable: true,
            },
            ref_state_ty,
            NO_ORIGIN,
        );
        b.effect(
            entry,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 0,
                            local_hash: 99,
                        },
                        instantiation: 0,
                    },
                    link_name: StrId(0),
                },
                function: None,
                args: vec![r],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
        let discr = b.op(entry, OpKind::Discriminant(st), I32, NO_ORIGIN);
        b.terminate(entry, RTerm::Return(Some(discr)));

        let mut func = b.finish();
        let stats = run(&mut func, &table);
        assert_eq!(stats.scalarized, 0);
        assert_eq!(stats.escaped, 1);
        assert!(func.verify(&table).is_ok());
    }

    #[test]
    fn multi_variant_enum_inactive_fieldless_zst_payload_scalarizes_and_interprets() {
        let mut tbl = table();
        let zst_ty = TypeId(10);
        let mut zst = descriptor(
            zst_ty,
            TypeKind::Tuple(Vec::new()),
            js(zst_ty, Vec::new(), false),
        );
        zst.layout.zst = true;
        tbl.insert(zst);

        // `enum Mode { Unit(()), Other }` — `fieldless: true`, yet variant 0 carries a ZST field.
        let mode_ty = TypeId(11);
        let mut mode_js = js(mode_ty, Vec::new(), false);
        mode_js.enum_repr = Some(EnumRepr::Tag { fieldless: true });
        tbl.insert(descriptor(
            mode_ty,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: 81,
                },
                args: Vec::new(),
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: StrId(0),
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Index(0),
                            ty: zst_ty,
                            offset: 0,
                        }],
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: StrId(1),
                        fields: Vec::new(),
                        logical_discr: 1,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
                niche: None,
            },
            mode_js,
        ));

        let outer_ty = TypeId(12);
        let mut outer_js = js(outer_ty, Vec::new(), true);
        outer_js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
        tbl.insert(descriptor(
            outer_ty,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: 82,
                },
                args: Vec::new(),
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: StrId(0),
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Index(0),
                            ty: I32,
                            offset: 0,
                        }],
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: StrId(1),
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Index(0),
                            ty: mode_ty,
                            offset: 0,
                        }],
                        logical_discr: 1,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
                niche: None,
            },
            outer_js,
        ));

        let mut b = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 83,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 1,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    inline_hint: false,
                    inline_always: false,
                },
                arguments: vec![I32],
                result: I32,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let left = b.block();
        let right = b.block();
        let merge = b.block();
        let ret_num = b.block();
        let ret_mode = b.block();

        let arg = b.param(I32, NO_ORIGIN);
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: arg,
                scrut_ty: I32,
                cases: vec![(0, left, Vec::new())],
                default: (right, Vec::new()),
            },
        );

        let v0 = b.op(
            left,
            OpKind::Aggregate {
                ty: outer_ty,
                variant: 0,
                active_field: None,
                fields: vec![arg],
            },
            outer_ty,
            NO_ORIGIN,
        );
        b.terminate(left, RTerm::Goto(merge, vec![v0]));

        let mode = b.op(
            right,
            OpKind::Const(ConstTree::Variant {
                variant: 1,
                fields: Vec::new(),
            }),
            mode_ty,
            NO_ORIGIN,
        );
        let v1 = b.op(
            right,
            OpKind::Aggregate {
                ty: outer_ty,
                variant: 1,
                active_field: None,
                fields: vec![mode],
            },
            outer_ty,
            NO_ORIGIN,
        );
        b.terminate(right, RTerm::Goto(merge, vec![v1]));

        let merged = b.block_param(merge, outer_ty, NO_ORIGIN);
        let discr = b.op(merge, OpKind::Discriminant(merged), I32, NO_ORIGIN);
        b.terminate(
            merge,
            RTerm::Switch {
                scrut: discr,
                scrut_ty: I32,
                cases: vec![(0, ret_num, Vec::new())],
                default: (ret_mode, Vec::new()),
            },
        );

        let n = b.op(
            ret_num,
            OpKind::Field {
                base: merged,
                variant: Some(0),
                field: 0,
            },
            I32,
            NO_ORIGIN,
        );
        b.terminate(ret_num, RTerm::Return(Some(n)));

        let m = b.op(
            ret_mode,
            OpKind::Field {
                base: merged,
                variant: Some(1),
                field: 0,
            },
            mode_ty,
            NO_ORIGIN,
        );
        let md = b.op(ret_mode, OpKind::Discriminant(m), I32, NO_ORIGIN);
        b.terminate(ret_mode, RTerm::Return(Some(md)));

        let before = b.finish();
        assert!(before.verify(&tbl).is_ok());
        let mut after = before.clone();
        let stats = run(&mut after, &tbl);
        assert!(stats.scalarized >= 1);
        assert_eq!(stats.escaped, 0);
        crate::opt::dce::run(&mut after, &tbl);
        assert!(after.verify(&tbl).is_ok());

        let interns = crate::jir::interns::Interns::default();
        for input in [0, 7] {
            let set = vec![(
                crate::jir::interp::Value::Int {
                    value: input,
                    bits: 32,
                    signed: true,
                },
                None,
            )];
            let verdict = crate::lower::ssa::check::compare_funcs(
                &before,
                &after,
                &tbl,
                &interns,
                &BTreeMap::new(),
                &set,
                100,
            );
            assert!(
                matches!(verdict, crate::lower::ssa::check::Verdict::Agree),
                "interp verdict for {input}: {verdict:?}"
            );
        }
    }

    #[test]
    fn multi_variant_enum_needs_clone_deref_store_escapes_cleanly() {
        let mut tbl = table();
        let pair_ty = TypeId(10);
        tbl.insert(descriptor(
            pair_ty,
            TypeKind::Tuple(vec![I32, I32]),
            js(pair_ty, Vec::new(), true),
        ));
        let enum_ty = TypeId(11);
        let mut enum_js = js(enum_ty, Vec::new(), true);
        enum_js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
        tbl.insert(descriptor(
            enum_ty,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: 84,
                },
                args: Vec::new(),
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: StrId(0),
                        fields: Vec::new(),
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: StrId(1),
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Index(0),
                            ty: pair_ty,
                            offset: 0,
                        }],
                        logical_discr: 1,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
                niche: None,
            },
            enum_js,
        ));
        let ref_enum_ty = TypeId(12);
        tbl.insert(descriptor(
            ref_enum_ty,
            TypeKind::Reference {
                pointee: enum_ty,
                mutable: true,
            },
            js(ref_enum_ty, Vec::new(), false),
        ));

        let mut b = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 85,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 1,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    inline_hint: false,
                    inline_always: false,
                },
                arguments: vec![I32],
                result: I32,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let arg = b.param(I32, NO_ORIGIN);
        let pair = b.op(
            entry,
            OpKind::Aggregate {
                ty: pair_ty,
                variant: 0,
                active_field: None,
                fields: vec![arg, arg],
            },
            pair_ty,
            NO_ORIGIN,
        );
        let x = b.op(
            entry,
            OpKind::Aggregate {
                ty: enum_ty,
                variant: 1,
                active_field: None,
                fields: vec![pair],
            },
            enum_ty,
            NO_ORIGIN,
        );
        let r = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(x),
                path: Vec::new(),
                mutable: true,
            },
            ref_enum_ty,
            NO_ORIGIN,
        );
        let y = b.op(entry, OpKind::Deref(r), enum_ty, NO_ORIGIN);
        let z = b.op(
            entry,
            OpKind::Aggregate {
                ty: enum_ty,
                variant: 0,
                active_field: None,
                fields: Vec::new(),
            },
            enum_ty,
            NO_ORIGIN,
        );
        b.effect(entry, OpKind::StoreThrough { ptr: r, v: z }, NO_ORIGIN);
        let d = b.op(entry, OpKind::Discriminant(y), I32, NO_ORIGIN);
        b.terminate(entry, RTerm::Return(Some(d)));

        let mut func = b.finish();
        let stats = run(&mut func, &tbl);
        assert_eq!(stats.scalarized, 0);
        assert!(stats.escaped >= 1);
        assert!(func.verify(&tbl).is_ok());
    }
}

/// The property the pass owes: it changes what a function *is*, never what it computes.
///
/// Every function of the fixture corpus (`build/**/*.o`, and the sysroots' `libcore` too under
/// `RCGJS_SSA_FIXTURES=core`) is built to JIR-R, run on generated arguments by the JIR-R
/// interpreter, rewritten, verified, and run again — and the two runs have to agree on the result
/// *and* the call/drop trace. A construct the interpreter does not model traps the same way on
/// both sides, so it is skipped rather than counted.
///
/// The corpus is whatever `build/` holds, so on a checkout that has not run the suites the test
/// reports that it found nothing and passes — unless `RCGJS_SSA_FIXTURES_REQUIRED=1` says the
/// corpus must be there.
#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use rcgjs_object::object;

    use crate::jir::interns::StrId;
    use crate::jir::interp::Value;
    use crate::jir::repr::Func;
    use crate::jir::semantic::{BodyKind, TypeId};
    use crate::jir::types::TypeTable;
    use crate::lower::ssa;

    /// How many argument vectors and how many interpreter steps one function gets. Lower than the
    /// SSA harness's on purpose: this runs the interpreter twice per vector.
    const SETS: usize = 8;
    const STEPS: u64 = 5_000;

    #[derive(Default)]
    struct Counts {
        functions: usize,
        rewritten: usize,
        agreed: usize,
        skipped: usize,
        mismatches: Vec<String>,
        verify_failures: Vec<String>,
    }

    #[test]
    fn pea_preserves_fixture_semantics() {
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
        let mut counts = Counts::default();
        let core = std::env::var("RCGJS_SSA_FIXTURES").is_ok_and(|value| value == "core");
        let mut paths = Vec::new();
        walk(&root, &mut paths, core);
        let mut seen: BTreeMap<u128, ()> = BTreeMap::new();
        for path in &paths {
            for (label, bytes) in members(path) {
                check_object(&label, &bytes, &mut counts, &mut seen);
            }
        }
        eprintln!(
            "pea property: {} functions, {} rewritten, {} argument sets agreed, {} skipped, \
             {} verify failures, {} mismatches",
            counts.functions,
            counts.rewritten,
            counts.agreed,
            counts.skipped,
            counts.verify_failures.len(),
            counts.mismatches.len(),
        );
        for failure in counts.verify_failures.iter().take(10) {
            eprintln!("  VERIFY   {failure}");
        }
        for mismatch in counts.mismatches.iter().take(10) {
            eprintln!("  MISMATCH {mismatch}");
        }
        ssa::check::require_fixtures(counts.functions, "pea property");
        assert!(
            counts.verify_failures.is_empty(),
            "{} rewritten functions failed Func::verify",
            counts.verify_failures.len()
        );
        assert!(
            counts.mismatches.is_empty(),
            "{} functions changed meaning",
            counts.mismatches.len()
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

    fn check_object(label: &str, bytes: &[u8], counts: &mut Counts, seen: &mut BTreeMap<u128, ()>) {
        if !object::is_object(bytes) {
            return;
        }
        let Ok(Some(object)) = object::read(bytes) else {
            return;
        };
        let table = &object.interns.types;
        let interns = &object.interns;
        let mut decoded = Vec::new();
        let mut results: BTreeMap<StrId, TypeId> = BTreeMap::new();
        for item in &object.summaries {
            let Ok(Some((body, origins))) = object.decode_verified(item) else {
                continue;
            };
            if !matches!(body.kind, BodyKind::Function) {
                continue;
            }
            if let Some(index) = interns
                .strings
                .iter()
                .position(|string| string == &item.key.link_name)
            {
                results.insert(StrId(index as u32), body.signature.result);
            }
            decoded.push((item.key.link_name.clone(), body, origins));
        }
        for (name, body, origins) in decoded {
            let key = crate::jir::summary::semantic_hash(&body, interns);
            let key = u128::from(key.lo) ^ (u128::from(key.hi) << 64);
            if seen.insert(key, ()).is_some() {
                continue;
            }
            let Ok(before) = ssa::build(&body, Some(&origins), table, interns) else {
                continue;
            };
            counts.functions += 1;
            let mut after = before.clone();
            let stats = super::run(&mut after, table);
            if stats.scalarized == 0 {
                continue;
            }
            counts.rewritten += 1;
            if let Err(error) = after.verify(table) {
                counts
                    .verify_failures
                    .push(format!("{label} {name}: {error}"));
                continue;
            }
            let Some(sets) = ssa::check::argument_sets(
                &before
                    .params
                    .iter()
                    .map(|param| before.values[param.0 as usize].ty)
                    .collect::<Vec<_>>(),
                table,
            ) else {
                continue;
            };
            for set in sets.iter().take(SETS) {
                match compare(&before, &after, table, interns, &results, set) {
                    Verdict::Agree => counts.agreed += 1,
                    Verdict::Skipped => counts.skipped += 1,
                    Verdict::Mismatch(why) => {
                        counts.mismatches.push(format!("{label} {name}: {why}"));
                    }
                }
            }
        }
    }

    enum Verdict {
        Agree,
        Skipped,
        Mismatch(String),
    }

    /// Runs the JIR-R interpreter over both functions on one argument vector.
    fn compare(
        before: &Func,
        after: &Func,
        table: &TypeTable,
        interns: &crate::jir::interns::Interns,
        results: &BTreeMap<StrId, TypeId>,
        set: &[(Value, Option<Value>)],
    ) -> Verdict {
        match ssa::check::compare_funcs(before, after, table, interns, results, set, STEPS) {
            ssa::check::Verdict::Agree => Verdict::Agree,
            ssa::check::Verdict::Skipped(_) => Verdict::Skipped,
            ssa::check::Verdict::Mismatch(why) => Verdict::Mismatch(why),
        }
    }

    mod cell_demotion_tests {
        use crate::jir::interns::Interns;
        use crate::jir::origin::OriginId;
        use crate::jir::repr::{FuncBuilder, OpKind, Proj, RTerm, Root};
        use crate::jir::semantic::{
            AbiSignature, BinaryOp, CallSemantic, Callee, ConstTree, DefinitionId, ItemId,
            Signature, TypeId,
        };
        use crate::jir::types::{
            FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts, NominalForm,
            PointerForm, TypeDescriptor, TypeKind, TypeMeta, VariantDescriptor,
        };

        const O: OriginId = OriginId(0);
        const I32: TypeId = TypeId(1);
        const BOOL: TypeId = TypeId(2);
        const REF_I32: TypeId = TypeId(3);
        const MUT_I32: TypeId = TypeId(4);
        const PAIR: TypeId = TypeId(5);
        const REF_PAIR: TypeId = TypeId(6);
        const MAYBE_PAIR: TypeId = TypeId(7);

        fn item(n: u64) -> ItemId {
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: n,
                },
                instantiation: 0,
            }
        }

        fn js_facts(peeled: TypeId) -> JsFacts {
            JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            }
        }

        fn world() -> Interns {
            let mut interns = Interns::new();
            let f_a = interns.intern_str("a");
            let f_b = interns.intern_str("b");
            let pair_name = interns.intern_str("Pair");
            interns.intern_type(TypeDescriptor {
                id: I32,
                name: None,
                kind: TypeKind::Integer {
                    signed: true,
                    bits: 32,
                },
                layout: LayoutFacts {
                    sized: true,
                    size: Some(4),
                    align: Some(4),
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: Some(IntFacts {
                        signed: true,
                        bits: 32,
                        repr: IntRepr::Number,
                    }),
                    ..js_facts(I32)
                },
            });
            interns.intern_type(TypeDescriptor {
                id: BOOL,
                name: None,
                kind: TypeKind::Bool,
                layout: LayoutFacts {
                    sized: true,
                    size: Some(1),
                    align: Some(1),
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: js_facts(BOOL),
            });
            interns.intern_type(TypeDescriptor {
                id: REF_I32,
                name: None,
                kind: TypeKind::Reference {
                    pointee: I32,
                    mutable: false,
                },
                layout: LayoutFacts {
                    sized: true,
                    size: Some(4),
                    align: Some(4),
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    pointer_form: Some(PointerForm::Slot),
                    ..js_facts(REF_I32)
                },
            });
            interns.intern_type(TypeDescriptor {
                id: MUT_I32,
                name: None,
                kind: TypeKind::Reference {
                    pointee: I32,
                    mutable: true,
                },
                layout: LayoutFacts {
                    sized: true,
                    size: Some(4),
                    align: Some(4),
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    pointer_form: Some(PointerForm::Slot),
                    ..js_facts(MUT_I32)
                },
            });
            let fields = vec![
                FieldDescriptor {
                    key: FieldKey::Name(f_a),
                    ty: I32,
                    offset: 0,
                },
                FieldDescriptor {
                    key: FieldKey::Name(f_b),
                    ty: I32,
                    offset: 4,
                },
            ];
            interns.intern_type(TypeDescriptor {
                id: PAIR,
                name: Some(pair_name),
                kind: TypeKind::Adt {
                    def: item(99).definition,
                    form: NominalForm::Struct,
                    args: Vec::new(),
                    variants: vec![VariantDescriptor {
                        name: pair_name,
                        logical_discr: 0,
                        physical_tag: None,
                        fields: fields.clone(),
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    }],
                    niche: None,
                },
                layout: LayoutFacts {
                    sized: true,
                    size: Some(8),
                    align: Some(4),
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    is_indirect: true,
                    needs_clone: true,
                    fields,
                    ..js_facts(PAIR)
                },
            });
            interns.intern_type(TypeDescriptor {
                id: REF_PAIR,
                name: None,
                kind: TypeKind::Reference {
                    pointee: PAIR,
                    mutable: true,
                },
                layout: LayoutFacts {
                    sized: true,
                    size: Some(4),
                    align: Some(4),
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    pointer_form: Some(PointerForm::Thin),
                    ..js_facts(REF_PAIR)
                },
            });
            let maybe_name = interns.intern_str("MaybeUninitPair");
            interns.intern_type(TypeDescriptor {
                id: MAYBE_PAIR,
                name: Some(maybe_name),
                kind: TypeKind::Adt {
                    def: item(98).definition,
                    form: NominalForm::Union,
                    args: Vec::new(),
                    variants: vec![VariantDescriptor {
                        name: maybe_name,
                        logical_discr: 0,
                        physical_tag: None,
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Name(f_a),
                            ty: PAIR,
                            offset: 0,
                        }],
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    }],
                    niche: None,
                },
                layout: LayoutFacts {
                    sized: true,
                    size: Some(8),
                    align: Some(4),
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    is_indirect: false,
                    needs_clone: true,
                    crosses_transparent_union: true,
                    peeled: PAIR,
                    ..js_facts(PAIR)
                },
            });
            interns
        }

        #[test]
        fn promotes_cell_borrow_copy_and_reborrow_and_unboxes_mutated_param() {
            let interns = world();
            let table = &interns.types;
            let sig = Signature {
                arguments: vec![I32],
                result: I32,
                abi: AbiSignature {
                    parameters: 1,
                    ..AbiSignature::default()
                },
            };
            let mut b = FuncBuilder::new(item(1), sig);
            let a = b.param(I32, O);
            let cell = b.cell(I32, true, O);
            b.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(cell), O);
            b.effect(FuncBuilder::ENTRY, OpKind::CellStore { cell, v: a }, O);
            let r = b.op(
                FuncBuilder::ENTRY,
                OpKind::BorrowView {
                    root: Root::Cell(cell),
                    path: Vec::new(),
                    mutable: true,
                },
                MUT_I32,
                O,
            );
            let r_copy = b.op(FuncBuilder::ENTRY, OpKind::Copy(r), MUT_I32, O);
            let r_reborrow = b.op(
                FuncBuilder::ENTRY,
                OpKind::BorrowView {
                    root: Root::Value(r),
                    path: vec![Proj::Deref],
                    mutable: true,
                },
                MUT_I32,
                O,
            );
            let old = b.op(FuncBuilder::ENTRY, OpKind::Deref(r_copy), I32, O);
            let one = b.op(
                FuncBuilder::ENTRY,
                OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
                I32,
                O,
            );
            let next = b.op(
                FuncBuilder::ENTRY,
                OpKind::Binary {
                    op: BinaryOp::Add,
                    l: old,
                    r: one,
                    ty: I32,
                },
                I32,
                O,
            );
            b.effect(
                FuncBuilder::ENTRY,
                OpKind::StoreThrough {
                    ptr: r_reborrow,
                    v: next,
                },
                O,
            );
            let final_v = b.op(FuncBuilder::ENTRY, OpKind::CellLoad(cell), I32, O);
            b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(final_v)));
            let mut func = b.finish();
            func.verify(table).expect("valid before");
            let before = func.clone();

            let opts = crate::opts::Opts {
                optimizer: crate::opts::OptimizerMode::On,
                ..crate::opts::Opts::default()
            };
            let mut stats = crate::opt::OptStats::default();
            crate::opt::optimize(&mut func, table, &interns, &opts, &mut stats)
                .expect("opt succeeds");
            func.verify(table).expect("valid after");
            let sets = super::ssa::check::argument_sets(&[I32], table).expect("i32 sets");
            let results = std::collections::BTreeMap::new();
            for set in &sets {
                assert!(
                    matches!(
                        super::compare(&before, &func, table, &interns, &results, set),
                        super::Verdict::Agree
                    ),
                    "unboxing optimization diverged under JIR interpreter"
                );
            }
            assert!(!func.cells[0].boxed, "cell should be unboxed");
            assert!(
                !func.blocks[0]
                    .ops
                    .iter()
                    .any(|op| matches!(op.kind, OpKind::BorrowView { .. })),
                "dead BorrowView ops should be removed"
            );
            // Guard ensures mutated param cell is not dropped by `emit.rs` `param_cells`.
            let stores = func.blocks[0]
                .ops
                .iter()
                .filter(|op| matches!(op.kind, OpKind::CellStore { .. }))
                .count();
            assert_eq!(stores, 2);
            let first_store = func.blocks[0]
                .ops
                .iter()
                .find_map(|op| match op.kind {
                    OpKind::CellStore { v, .. } => Some(v),
                    _ => None,
                })
                .expect("initial store");
            assert!(
                !func.params.contains(&first_store),
                "mutated param cell's initial store must use Copy(param) so param_cells doesn't drop the second store"
            );
        }

        #[test]
        fn refuses_promotion_when_borrow_escapes_to_call_or_raw_view_or_eq() {
            let mut interns = world();
            let sink_str = interns.intern_str("sink");
            let table = &interns.types;
            let sig = Signature {
                arguments: vec![I32],
                result: I32,
                abi: AbiSignature {
                    parameters: 1,
                    ..AbiSignature::default()
                },
            };

            // 1. Escapes via Call
            {
                let mut b = FuncBuilder::new(item(2), sig.clone());
                let a = b.param(I32, O);
                let cell = b.cell(I32, true, O);
                b.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(cell), O);
                b.effect(FuncBuilder::ENTRY, OpKind::CellStore { cell, v: a }, O);
                let r = b.op(
                    FuncBuilder::ENTRY,
                    OpKind::BorrowView {
                        root: Root::Cell(cell),
                        path: Vec::new(),
                        mutable: false,
                    },
                    REF_I32,
                    O,
                );
                let _v = b.op(FuncBuilder::ENTRY, OpKind::Deref(r), I32, O);
                let res = b.op(
                    FuncBuilder::ENTRY,
                    OpKind::Call {
                        callee: Callee::Direct {
                            item: item(9),
                            link_name: sink_str,
                        },
                        function: None,
                        args: vec![r],
                        track_caller: None,
                        semantic: CallSemantic::Ordinary,
                        host: None,
                    },
                    I32,
                    O,
                );
                b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(res)));
                let mut func = b.finish();
                assert!(!super::super::promote_cell_borrows(&mut func, table));
                assert!(!super::super::unbox_cells(&mut func, table));
                assert!(func.cells[0].boxed);
            }

            // 2. Escapes via BinaryOp::Eq
            {
                let mut b = FuncBuilder::new(item(3), sig.clone());
                let a = b.param(I32, O);
                let cell = b.cell(I32, true, O);
                b.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(cell), O);
                b.effect(FuncBuilder::ENTRY, OpKind::CellStore { cell, v: a }, O);
                let r = b.op(
                    FuncBuilder::ENTRY,
                    OpKind::BorrowView {
                        root: Root::Cell(cell),
                        path: Vec::new(),
                        mutable: false,
                    },
                    REF_I32,
                    O,
                );
                let _cmp = b.op(
                    FuncBuilder::ENTRY,
                    OpKind::Binary {
                        op: BinaryOp::Eq,
                        l: r,
                        r,
                        ty: REF_I32,
                    },
                    BOOL,
                    O,
                );
                let v = b.op(FuncBuilder::ENTRY, OpKind::Deref(r), I32, O);
                b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(v)));
                let mut func = b.finish();
                assert!(!super::super::promote_cell_borrows(&mut func, table));
                assert!(func.cells[0].boxed);
            }

            // 3. RawView on Cell prevents unboxing
            {
                let mut b = FuncBuilder::new(item(4), sig);
                let a = b.param(I32, O);
                let cell = b.cell(I32, true, O);
                b.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(cell), O);
                b.effect(FuncBuilder::ENTRY, OpKind::CellStore { cell, v: a }, O);
                let raw = b.op(
                    FuncBuilder::ENTRY,
                    OpKind::RawView {
                        root: Root::Cell(cell),
                        path: Vec::new(),
                        mutable: false,
                    },
                    REF_I32,
                    O,
                );
                let v = b.op(FuncBuilder::ENTRY, OpKind::Deref(raw), I32, O);
                b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(v)));
                let mut func = b.finish();
                assert!(!super::super::promote_cell_borrows(&mut func, table));
                assert!(!super::super::unbox_cells(&mut func, table));
                assert!(func.cells[0].boxed);
            }
        }

        #[test]
        fn refuses_promotion_for_indirect_aggregate_cell() {
            let interns = world();
            let table = &interns.types;
            let sig = Signature {
                arguments: vec![PAIR],
                result: PAIR,
                abi: AbiSignature {
                    parameters: 1,
                    ..AbiSignature::default()
                },
            };
            let mut b = FuncBuilder::new(item(5), sig);
            let p = b.param(PAIR, O);
            let cell = b.cell(PAIR, false, O);
            b.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(cell), O);
            b.effect(FuncBuilder::ENTRY, OpKind::CellStore { cell, v: p }, O);
            let r = b.op(
                FuncBuilder::ENTRY,
                OpKind::BorrowView {
                    root: Root::Cell(cell),
                    path: Vec::new(),
                    mutable: true,
                },
                REF_PAIR,
                O,
            );
            let deref = b.op(FuncBuilder::ENTRY, OpKind::Deref(r), PAIR, O);
            b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(deref)));
            let mut func = b.finish();
            assert!(!super::super::promote_cell_borrows(&mut func, table));
        }

        #[test]
        fn refuses_unboxing_for_transparent_needs_clone_and_guards_unboxed_move() {
            let interns = world();
            let table = &interns.types;
            let sig = Signature {
                arguments: vec![MAYBE_PAIR],
                result: MAYBE_PAIR,
                abi: AbiSignature {
                    parameters: 1,
                    ..AbiSignature::default()
                },
            };

            // 1. `!is_indirect && needs_clone` (`MaybeUninit<Pair>`) must NEVER be unboxed.
            {
                let mut b = FuncBuilder::new(item(6), sig.clone());
                let p = b.param(MAYBE_PAIR, O);
                let cell = b.cell(MAYBE_PAIR, true, O);
                b.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(cell), O);
                b.effect(FuncBuilder::ENTRY, OpKind::CellStore { cell, v: p }, O);
                let replacement = b.op(
                    FuncBuilder::ENTRY,
                    OpKind::Const(ConstTree::Aggregate(vec![ConstTree::Aggregate(vec![
                        ConstTree::Scalar { bits: 99, bytes: 4 },
                        ConstTree::Scalar {
                            bits: 100,
                            bytes: 4,
                        },
                    ])])),
                    MAYBE_PAIR,
                    O,
                );
                b.effect(
                    FuncBuilder::ENTRY,
                    OpKind::CellStore {
                        cell,
                        v: replacement,
                    },
                    O,
                );
                let out = b.op(FuncBuilder::ENTRY, OpKind::CellLoad(cell), MAYBE_PAIR, O);
                b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(out)));
                let mut func = b.finish();
                assert!(!super::super::unbox_cells(&mut func, table));
                assert!(
                    func.cells[0].boxed,
                    "transparent needs_clone cell must remain boxed"
                );
            }

            // 2. An unboxed mutated `needs_clone` param cell is guarded with `Move(param)`.
            {
                let mut b = FuncBuilder::new(item(7), sig);
                let p = b.param(MAYBE_PAIR, O);
                let cell = b.cell(MAYBE_PAIR, false, O);
                b.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(cell), O);
                b.effect(FuncBuilder::ENTRY, OpKind::CellStore { cell, v: p }, O);
                let replacement = b.op(
                    FuncBuilder::ENTRY,
                    OpKind::Const(ConstTree::Aggregate(vec![ConstTree::Aggregate(vec![
                        ConstTree::Scalar { bits: 99, bytes: 4 },
                        ConstTree::Scalar {
                            bits: 100,
                            bytes: 4,
                        },
                    ])])),
                    MAYBE_PAIR,
                    O,
                );
                b.effect(
                    FuncBuilder::ENTRY,
                    OpKind::CellStore {
                        cell,
                        v: replacement,
                    },
                    O,
                );
                let out = b.op(FuncBuilder::ENTRY, OpKind::CellLoad(cell), MAYBE_PAIR, O);
                b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(out)));
                let mut func = b.finish();
                super::super::guard_mutated_param_cells(&mut func, table);
                func.verify(table).expect("valid after Move guard");
                assert!(
                    func.blocks[0]
                        .ops
                        .iter()
                        .any(|op| matches!(op.kind, OpKind::Move(v) if v == p)),
                    "expected Move(param) guard before initial CellStore"
                );
            }
        }

        #[test]
        fn pure_packaging_exit_array_keeps_loop_carried_pair_virtual() {
            const PAIR_ARR2: TypeId = TypeId(99);
            let mut interns = world();
            let sink_str = interns.intern_str("sink");
            let elem_field = FieldDescriptor {
                key: FieldKey::Index(0),
                ty: PAIR,
                offset: 0,
            };
            interns.types.insert(TypeDescriptor {
                id: PAIR_ARR2,
                name: None,
                kind: TypeKind::Array {
                    element: PAIR,
                    length: 2,
                },
                layout: LayoutFacts {
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    is_indirect: true,
                    needs_clone: true,
                    fields: vec![elem_field],
                    ..js_facts(PAIR_ARR2)
                },
            });
            let tbl = &interns.types;
            let sig = Signature {
                arguments: vec![I32],
                result: PAIR_ARR2,
                abi: AbiSignature {
                    parameters: 1,
                    ..AbiSignature::default()
                },
            };
            let mut b = FuncBuilder::new(item(8), sig);
            let n = b.param(I32, O);
            let c0 = b.op(
                FuncBuilder::ENTRY,
                OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
                I32,
                O,
            );
            let c1 = b.op(
                FuncBuilder::ENTRY,
                OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
                I32,
                O,
            );
            let init_pair = b.op(
                FuncBuilder::ENTRY,
                OpKind::Aggregate {
                    ty: PAIR,
                    variant: 0,
                    active_field: None,
                    fields: vec![c0, c1],
                },
                PAIR,
                O,
            );

            let header = b.block();
            let body = b.block();
            let exit_pack = b.block();
            let sink = b.block();

            b.terminate(FuncBuilder::ENTRY, RTerm::Goto(header, vec![c0, init_pair]));

            let i_param = b.block_param(header, I32, O);
            let p_param = b.block_param(header, PAIR, O);
            let cmp = b.op(
                header,
                OpKind::Binary {
                    op: BinaryOp::Lt,
                    l: i_param,
                    r: n,
                    ty: I32,
                },
                I32,
                O,
            );
            b.terminate(
                header,
                RTerm::Switch {
                    scrut: cmp,
                    scrut_ty: I32,
                    cases: vec![(0, exit_pack, Vec::new())],
                    default: (body, Vec::new()),
                },
            );

            let f0 = b.op(
                body,
                OpKind::Field {
                    base: p_param,
                    variant: None,
                    field: 0,
                },
                I32,
                O,
            );
            let f1 = b.op(
                body,
                OpKind::Field {
                    base: p_param,
                    variant: None,
                    field: 1,
                },
                I32,
                O,
            );
            let next_f0 = b.op(
                body,
                OpKind::Binary {
                    op: BinaryOp::Add,
                    l: f0,
                    r: f1,
                    ty: I32,
                },
                I32,
                O,
            );
            let next_i = b.op(
                body,
                OpKind::Binary {
                    op: BinaryOp::Add,
                    l: i_param,
                    r: c1,
                    ty: I32,
                },
                I32,
                O,
            );
            let next_pair = b.op(
                body,
                OpKind::Aggregate {
                    ty: PAIR,
                    variant: 0,
                    active_field: None,
                    fields: vec![next_f0, f0],
                },
                PAIR,
                O,
            );
            b.terminate(body, RTerm::Goto(header, vec![next_i, next_pair]));

            // Post-loop pure packaging `[p_param, p_param]` passed to an escaped sink parameter.
            let m0 = b.op(exit_pack, OpKind::Move(p_param), PAIR, O);
            let m1 = b.op(exit_pack, OpKind::Move(p_param), PAIR, O);
            let arr = b.op(
                exit_pack,
                OpKind::Aggregate {
                    ty: PAIR_ARR2,
                    variant: 0,
                    active_field: None,
                    fields: vec![m0, m1],
                },
                PAIR_ARR2,
                O,
            );
            b.terminate(exit_pack, RTerm::Goto(sink, vec![arr]));

            let sink_param = b.block_param(sink, PAIR_ARR2, O);
            let _call = b.op(
                sink,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: item(99),
                        link_name: sink_str,
                    },
                    function: None,
                    args: vec![sink_param],
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                I32,
                O,
            );
            b.terminate(sink, RTerm::Return(Some(sink_param)));

            let mut func = b.finish();
            let stats = super::super::run(&mut func, tbl);
            func.verify(tbl).expect("valid after PEA");
            assert!(
                stats.scalarized >= 3,
                "expected init_pair, p_param, next_pair scalarized, got {stats:?}"
            );
            assert!(
                func.blocks[body.0 as usize]
                    .ops
                    .iter()
                    .all(|op| !matches!(op.kind, OpKind::Aggregate { .. })),
                "loop body must not allocate Pair:\n{:#?}",
                func.blocks[body.0 as usize].ops
            );
            assert!(
                func.blocks[exit_pack.0 as usize]
                    .ops
                    .iter()
                    .any(|op| matches!(op.kind, OpKind::Aggregate { ty: PAIR_ARR2, .. })),
                "exit block must lazily materialize Pair and [Pair; 2]"
            );
        }

        #[test]
        fn null_niche_projected_cell_borrow_unboxes_with_zero_scalarized_and_invoke() {
            use crate::jir::types::EnumRepr;

            const I8: TypeId = TypeId(40);
            const ORDERING: TypeId = TypeId(41);
            const OPT_ORDERING: TypeId = TypeId(42);
            const REF_OPT_ORDERING: TypeId = TypeId(43);
            const REF_ORDERING: TypeId = TypeId(44);

            let mut interns = world();
            let v_less = interns.intern_str("Less");
            let v_equal = interns.intern_str("Equal");
            let v_greater = interns.intern_str("Greater");
            let v_none = interns.intern_str("None");
            let v_some = interns.intern_str("Some");
            let ord_name = interns.intern_str("Ordering");
            let opt_name = interns.intern_str("Option");
            let callee_name = interns.intern_str("helper");

            interns.intern_type(TypeDescriptor {
                id: I8,
                name: None,
                kind: TypeKind::Integer {
                    signed: true,
                    bits: 8,
                },
                layout: LayoutFacts {
                    sized: true,
                    size: Some(1),
                    align: Some(1),
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: Some(IntFacts {
                        signed: true,
                        bits: 8,
                        repr: IntRepr::Number,
                    }),
                    ..js_facts(I8)
                },
            });
            interns.intern_type(TypeDescriptor {
                id: ORDERING,
                name: Some(ord_name),
                kind: TypeKind::Adt {
                    def: item(910).definition,
                    form: NominalForm::Enum,
                    args: Vec::new(),
                    variants: vec![
                        VariantDescriptor {
                            name: v_less,
                            logical_discr: -1,
                            physical_tag: None,
                            fields: Vec::new(),
                            result_ok: false,
                            saved_local_keys: Vec::new(),
                        },
                        VariantDescriptor {
                            name: v_equal,
                            logical_discr: 0,
                            physical_tag: None,
                            fields: Vec::new(),
                            result_ok: false,
                            saved_local_keys: Vec::new(),
                        },
                        VariantDescriptor {
                            name: v_greater,
                            logical_discr: 1,
                            physical_tag: None,
                            fields: Vec::new(),
                            result_ok: false,
                            saved_local_keys: Vec::new(),
                        },
                    ],
                    niche: None,
                },
                layout: LayoutFacts {
                    sized: true,
                    size: Some(1),
                    align: Some(1),
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    enum_repr: Some(EnumRepr::Number {
                        signed: true,
                        bits: 8,
                    }),
                    is_primitive_enum: true,
                    ..js_facts(ORDERING)
                },
            });
            interns.intern_type(TypeDescriptor {
                id: OPT_ORDERING,
                name: Some(opt_name),
                kind: TypeKind::Adt {
                    def: item(911).definition,
                    form: NominalForm::Enum,
                    args: Vec::new(),
                    variants: vec![
                        VariantDescriptor {
                            name: v_none,
                            logical_discr: 0,
                            physical_tag: None,
                            fields: Vec::new(),
                            result_ok: false,
                            saved_local_keys: Vec::new(),
                        },
                        VariantDescriptor {
                            name: v_some,
                            logical_discr: 1,
                            physical_tag: None,
                            fields: vec![FieldDescriptor {
                                key: FieldKey::Index(0),
                                ty: ORDERING,
                                offset: 0,
                            }],
                            result_ok: false,
                            saved_local_keys: Vec::new(),
                        },
                    ],
                    niche: None,
                },
                layout: LayoutFacts {
                    sized: true,
                    size: Some(1),
                    align: Some(1),
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    enum_repr: Some(EnumRepr::NullNiche {
                        none_variant: 0,
                        some_variant: 1,
                    }),
                    ..js_facts(OPT_ORDERING)
                },
            });
            interns.intern_type(TypeDescriptor {
                id: REF_OPT_ORDERING,
                name: None,
                kind: TypeKind::Reference {
                    pointee: OPT_ORDERING,
                    mutable: false,
                },
                layout: LayoutFacts {
                    sized: true,
                    size: Some(4),
                    align: Some(4),
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    pointer_form: Some(PointerForm::Slot),
                    ..js_facts(REF_OPT_ORDERING)
                },
            });
            interns.intern_type(TypeDescriptor {
                id: REF_ORDERING,
                name: None,
                kind: TypeKind::Reference {
                    pointee: ORDERING,
                    mutable: false,
                },
                layout: LayoutFacts {
                    sized: true,
                    size: Some(4),
                    align: Some(4),
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    pointer_form: Some(PointerForm::Slot),
                    ..js_facts(REF_ORDERING)
                },
            });

            let tbl = &interns.types;
            for with_invoke in [false, true] {
                let mut b = FuncBuilder::new(
                    item(if with_invoke { 901 } else { 900 }),
                    Signature {
                        arguments: vec![OPT_ORDERING],
                        result: BOOL,
                        abi: AbiSignature {
                            parameters: 1,
                            spread: None,
                            caller_location: false,
                            inline_never: false,
                            inline_hint: false,
                            inline_always: false,
                        },
                    },
                );
                let arg = b.param(OPT_ORDERING, O);
                let cell = b.cell(OPT_ORDERING, true, O);
                b.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(cell), O);
                b.effect(FuncBuilder::ENTRY, OpKind::CellStore { cell, v: arg }, O);
                let next_block = b.block();
                if with_invoke {
                    let unwind = b.block();
                    let _ = b.op(
                        FuncBuilder::ENTRY,
                        OpKind::Call {
                            callee: Callee::Direct {
                                item: item(902),
                                link_name: callee_name,
                            },
                            function: None,
                            args: Vec::new(),
                            track_caller: None,
                            semantic: CallSemantic::Ordinary,
                            host: None,
                        },
                        I32,
                        O,
                    );
                    b.terminate(
                        FuncBuilder::ENTRY,
                        RTerm::Invoke {
                            normal: (next_block, Vec::new()),
                            unwind: (unwind, Vec::new()),
                        },
                    );
                    b.terminate(unwind, RTerm::Throw(None));
                } else {
                    b.terminate(FuncBuilder::ENTRY, RTerm::Goto(next_block, Vec::new()));
                }

                let borrow_opt = b.op(
                    next_block,
                    OpKind::BorrowView {
                        root: Root::Cell(cell),
                        path: Vec::new(),
                        mutable: false,
                    },
                    REF_OPT_ORDERING,
                    O,
                );
                let deref_opt = b.op(next_block, OpKind::Deref(borrow_opt), OPT_ORDERING, O);
                let discr_opt = b.op(next_block, OpKind::Discriminant(deref_opt), I8, O);
                let some_block = b.block();
                let none_block = b.block();
                b.terminate(
                    next_block,
                    RTerm::Switch {
                        scrut: discr_opt,
                        scrut_ty: I8,
                        cases: vec![(0, none_block, Vec::new())],
                        default: (some_block, Vec::new()),
                    },
                );
                let f_const = b.op(
                    none_block,
                    OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 1 }),
                    BOOL,
                    O,
                );
                b.terminate(none_block, RTerm::Return(Some(f_const)));

                // Projected borrow `[Deref, Downcast(1), Field(0)]` mirroring inlined `Option<Ordering>::eq`.
                let borrow_inner = b.op(
                    some_block,
                    OpKind::BorrowView {
                        root: Root::Value(borrow_opt),
                        path: vec![
                            Proj::Deref,
                            Proj::Downcast { variant: 1 },
                            Proj::Field {
                                field: 0,
                                ty: ORDERING,
                            },
                        ],
                        mutable: false,
                    },
                    REF_ORDERING,
                    O,
                );
                let ord_val = b.op(some_block, OpKind::Deref(borrow_inner), ORDERING, O);
                let ord_discr = b.op(some_block, OpKind::Discriminant(ord_val), I8, O);
                let less_const = b.op(
                    some_block,
                    OpKind::Const(ConstTree::Scalar {
                        bits: 255,
                        bytes: 1,
                    }),
                    I8,
                    O,
                );
                let is_less = b.op(
                    some_block,
                    OpKind::Binary {
                        op: BinaryOp::Eq,
                        l: ord_discr,
                        r: less_const,
                        ty: I8,
                    },
                    BOOL,
                    O,
                );
                b.terminate(some_block, RTerm::Return(Some(is_less)));

                let mut func = b.finish();
                func.verify(tbl).expect("pre-opt Func::verify");

                assert!(
                    super::super::promote_cell_borrows(&mut func, tbl),
                    "expected projected NullNiche borrow promotion (with_invoke={with_invoke})"
                );
                func.verify(tbl).expect("post-promote Func::verify");
                crate::opt::dce::run(&mut func, tbl);
                func.verify(tbl).expect("post-dce Func::verify");

                // PEA finds 0 scalarizable objects (NullNiche is not indirect, and Invoke skips PEA).
                let stats = super::super::run(&mut func, tbl);
                assert_eq!(stats.scalarized, 0);
                assert!(
                    super::super::unbox_cells(&mut func, tbl),
                    "expected NullNiche cell to flip boxed true -> false (with_invoke={with_invoke})"
                );
                func.verify(tbl).expect("post-unbox Func::verify");
                assert!(!func.cells[cell.0 as usize].boxed);
                assert!(
                    func.blocks
                        .iter()
                        .flat_map(|blk| &blk.ops)
                        .all(|op| !matches!(op.kind, OpKind::BorrowView { .. })),
                    "dead BorrowViews must disappear (with_invoke={with_invoke})"
                );
            }
        }
    }
}
