//! Lowers two exact standard-library slice-adapter loops to checked cursors.
//!
//! Inlining exposes `Enumerate<Take<SliceIter<u8>>>` and
//! `Enumerate<Skip<SliceIter<u8>>>` as mutations of nested iterator records. For a fresh,
//! non-escaping iterator those records have no observable identity: the loop can instead carry a
//! `usize` cursor, test it against the source slice length, and read the byte at that cursor.
//!
//! This pass deliberately recognizes only the exact diagnostic-item types, construction chain,
//! driver CFG, and enumerate prefix emitted by the pinned compiler. A changed library lowering,
//! a non-byte slice, a reused iterator, or any escape leaves the function untouched.

use std::collections::{BTreeSet, VecDeque};

use crate::jir::repr::{
    Carrier, Func, Op, OpKind, Proj, RBlock, RBlockId, RTerm, Root, ValueDef, ValueId,
};
use crate::jir::semantic::{BinaryOp, ConstTree, TypeId, UnaryOp};
use crate::jir::types::{IteratorIdentity, PinnedBoundary, TypeKind};
use crate::jir::{OriginId, TypeTable};

use super::witness::SliceIteratorLoop;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Adapter {
    Take,
    Skip,
}

#[derive(Clone, Copy, Debug)]
struct Site {
    block: usize,
    op: usize,
}

#[derive(Clone, Debug)]
struct LoopMatch {
    adapter: Adapter,
    header: usize,
    preheader: usize,
    backedges: Vec<usize>,
    driver: usize,
    update: Option<usize>,
    body: usize,
    body_prefix: usize,
    rewrite_blocks: BTreeSet<usize>,
    pass_item_ref: bool,
    exit: RBlockId,
    source: ValueId,
    len: ValueId,
    limit: ValueId,
    initial: ValueId,
    old_index: ValueId,
    old_byte: ValueId,
    item_ref_ty: TypeId,
    byte_ty: TypeId,
    usize_ty: TypeId,
    usize_bytes: u8,
    bool_ty: TypeId,
    origin: OriginId,
}

#[derive(Clone, Copy, Debug)]
struct AdapterTypes {
    adapter: Adapter,
    inner: TypeId,
    slice_iter: TypeId,
    element: TypeId,
    usize_ty: TypeId,
    usize_bytes: u8,
    bool_ty: TypeId,
}

#[derive(Clone, Debug)]
struct BodyMatch {
    driver: usize,
    update: Option<(usize, usize)>,
    body: usize,
    prefix: usize,
    driver_prefix: usize,
    pass_item_ref: bool,
    old_index: ValueId,
    old_byte: ValueId,
    item_ref_ty: TypeId,
    byte_ty: TypeId,
    origin: OriginId,
}

#[derive(Clone, Copy, Debug)]
enum AdapterAccess {
    Direct,
    Borrowed(ValueId),
}

#[derive(Clone, Copy, Debug)]
struct HeaderMatch {
    block: usize,
    access: AdapterAccess,
    continue_to: RBlockId,
}

/// Rewrites every exact slice-adapter loop in `func` and returns exact witnesses for them.
pub fn run(func: &mut Func, table: &TypeTable) -> Vec<SliceIteratorLoop> {
    let defs = definitions(func);
    let uses = use_counts(func);
    let outers: BTreeSet<ValueId> = func
        .values
        .iter()
        .enumerate()
        .filter(|(index, def)| {
            defs[*index].is_some()
                && table.get(def.ty).is_some_and(|descriptor| {
                    descriptor.meta.iterator == IteratorIdentity::Enumerate
                })
        })
        .map(|(index, _)| ValueId(index as u32))
        .collect();

    let mut matches = Vec::new();
    let mut occupied = BTreeSet::new();
    for outer in outers {
        let Some(found) = match_loop(func, table, &defs, &uses, outer) else {
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
    rewrite_blocks: &BTreeSet<usize>,
    preheader: usize,
    occupied: &mut BTreeSet<usize>,
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

fn match_loop(
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    uses: &[usize],
    outer: ValueId,
) -> Option<LoopMatch> {
    let outer_ty = func.values.get(outer.0 as usize)?.ty;
    let types = adapter_types(table, outer_ty)?;
    let (outer_root, outer_fields) = aggregate(func, defs, uses, outer, outer_ty, false)?;
    let [inner_value, initial] = outer_fields.as_slice() else {
        return None;
    };
    if !is_scalar(func, defs, *initial, 0)
        || func.values.get(initial.0 as usize)?.ty != types.usize_ty
    {
        return None;
    }

    let (_, inner_fields) = aggregate(func, defs, uses, *inner_value, types.inner, true)?;
    let [slice_value, limit] = inner_fields.as_slice() else {
        return None;
    };
    if func.values.get(limit.0 as usize)?.ty != types.usize_ty {
        return None;
    }

    let (_, slice_fields) = aggregate(func, defs, uses, *slice_value, types.slice_iter, true)?;
    let [start, end, _zst] = slice_fields.as_slice() else {
        return None;
    };
    let raw = strip_pointer_aliases(func, defs, *start)?;
    let OpKind::RawView {
        root: Root::Value(source),
        path,
        mutable: false,
    } = &definition(func, defs, raw)?.kind
    else {
        return None;
    };
    if path.as_slice() != [Proj::Deref]
        || !is_byte_slice_pointer(table, value_ty(func, *source)?, types.element)
    {
        return None;
    }
    let end = strip_pointer_aliases(func, defs, *end)?;
    let OpKind::Binary {
        op: BinaryOp::Offset,
        l: end_base,
        r: len,
        ..
    } = definition(func, defs, end)?.kind
    else {
        return None;
    };
    if strip_pointer_aliases(func, defs, end_base)? != raw {
        return None;
    }
    if !source_length_matches(func, defs, *source, len) || value_ty(func, len)? != types.usize_ty {
        return None;
    }

    let header = find_header(func, outer, types)?;
    let mut bodies = Vec::new();
    for block in 0..func.blocks.len() {
        let body = match header.access {
            AdapterAccess::Direct => match types.adapter {
                Adapter::Take => match_take_body(func, table, defs, outer, types, block),
                Adapter::Skip => match_skip_body(func, table, outer, types, block),
            },
            AdapterAccess::Borrowed(pointer) => {
                match_borrowed_body(func, table, pointer, outer_ty, types, block)
            }
        };
        if let Some(body) = body {
            bodies.push(body);
        }
    }
    let [body] = bodies.as_slice() else {
        return None;
    };
    let body_block = body.body;

    let outer_site = defs.get(outer.0 as usize).copied().flatten()?;
    let root_site = defs.get(outer_root.0 as usize).copied().flatten()?;
    if outer_site.block != root_site.block
        || outer_site.op + 1 != func.blocks.get(outer_site.block)?.ops.len()
    {
        return None;
    }
    let preheader = outer_site.block;
    let RTerm::Goto(target, args) = &func.blocks.get(preheader)?.term else {
        return None;
    };
    if target.0 as usize != header.block
        || args.len() != func.blocks.get(header.block)?.params.len()
    {
        return None;
    }

    let from_header = distances(func, RBlockId(header.block as u32));
    let to_header = reverse_reachable(func, RBlockId(header.block as u32));
    if !from_header.get(body_block)?.is_some()
        || !to_header.get(body_block).copied().unwrap_or(false)
    {
        return None;
    }
    let scc: BTreeSet<usize> = (0..func.blocks.len())
        .filter(|block| from_header[*block].is_some() && to_header[*block])
        .collect();
    if scc.contains(&preheader) {
        return None;
    }
    if !scc.contains(&(header.continue_to.0 as usize))
        || !reaches_avoiding(
            func,
            header.continue_to,
            RBlockId(body.driver as u32),
            header.block,
        )
    {
        return None;
    }

    let mut predecessors = Vec::new();
    for block in &func.blocks {
        if matches!(&block.term, RTerm::Goto(target, edge_args) if target.0 as usize == header.block && edge_args.len() == func.blocks[header.block].params.len())
        {
            predecessors.push(block.id.0 as usize);
        } else if block
            .term
            .successors()
            .iter()
            .any(|target| target.0 as usize == header.block)
        {
            return None;
        }
    }
    if !predecessors.contains(&preheader) {
        return None;
    }
    let body_reach = distances(func, RBlockId(body_block as u32));
    let backedges: Vec<usize> = predecessors
        .into_iter()
        .filter(|block| *block != preheader)
        .collect();
    if backedges.is_empty()
        || backedges
            .iter()
            .any(|block| !scc.contains(block) || body_reach[*block].is_none())
    {
        return None;
    }

    let mut exits = BTreeSet::new();
    for block in &scc {
        for target in func.blocks[*block].term.successors() {
            if !scc.contains(&(target.0 as usize)) {
                exits.insert(target);
            }
        }
    }
    if !removed_prefix_is_local(func, body, &scc) {
        return None;
    }
    let tainted = tainted_values(func, outer, body.old_index, body.old_byte);
    if !non_escaping(func, defs, &scc, &tainted) {
        return None;
    }
    let exit = common_exit(func, &scc, &exits)?;
    if !func.blocks.get(exit.0 as usize)?.params.is_empty()
        || !exits
            .iter()
            .all(|start| discardable_exit_chain(func, defs, &tainted, *start, exit))
    {
        return None;
    }

    Some(LoopMatch {
        adapter: types.adapter,
        header: header.block,
        preheader,
        backedges,
        driver: body.driver,
        update: body.update.map(|(block, _)| block),
        body: body.body,
        body_prefix: body.prefix,
        rewrite_blocks: scc,
        pass_item_ref: body.pass_item_ref,
        exit,
        source: *source,
        len,
        limit: *limit,
        initial: match types.adapter {
            Adapter::Take => *initial,
            Adapter::Skip => *limit,
        },
        old_index: body.old_index,
        old_byte: body.old_byte,
        item_ref_ty: body.item_ref_ty,
        byte_ty: body.byte_ty,
        usize_ty: types.usize_ty,
        usize_bytes: types.usize_bytes,
        bool_ty: types.bool_ty,
        origin: body.origin,
    })
}

fn adapter_types(table: &TypeTable, outer: TypeId) -> Option<AdapterTypes> {
    let descriptor = exact_iterator(table, outer, IteratorIdentity::Enumerate)?;
    let TypeKind::Adt { args, .. } = &descriptor.kind else {
        return None;
    };
    let [inner] = args.as_slice() else {
        return None;
    };
    let inner_descriptor = table.get(*inner)?;
    let adapter = match inner_descriptor.meta.iterator {
        IteratorIdentity::Take => Adapter::Take,
        IteratorIdentity::Skip => Adapter::Skip,
        _ => return None,
    };
    exact_iterator(table, *inner, inner_descriptor.meta.iterator)?;
    let TypeKind::Adt { args, .. } = &inner_descriptor.kind else {
        return None;
    };
    let [slice_iter] = args.as_slice() else {
        return None;
    };
    let slice_descriptor = exact_iterator(table, *slice_iter, IteratorIdentity::Slice)?;
    let TypeKind::Adt { args, .. } = &slice_descriptor.kind else {
        return None;
    };
    let [element] = args.as_slice() else {
        return None;
    };
    if table.int_info(*element) != Some((false, 8)) {
        return None;
    }
    let usize_ty = table.field(outer, None, 1)?.ty;
    let (false, usize_bits) = table.int_info(usize_ty)? else {
        return None;
    };
    if table.field(outer, None, 0)?.ty != *inner
        || table.field(*inner, None, 0)?.ty != *slice_iter
        || table.field(*inner, None, 1)?.ty != usize_ty
    {
        return None;
    }
    Some(AdapterTypes {
        adapter,
        inner: *inner,
        slice_iter: *slice_iter,
        element: *element,
        usize_ty,
        usize_bytes: u8::try_from(usize_bits / 8).ok()?,
        bool_ty: bool_type(table)?,
    })
}

fn exact_iterator(
    table: &TypeTable,
    ty: TypeId,
    identity: IteratorIdentity,
) -> Option<&crate::jir::types::TypeDescriptor> {
    let descriptor = table.get(ty)?;
    (descriptor.meta.iterator == identity
        && descriptor.meta.boundary == PinnedBoundary::None
        && !descriptor.layout.needs_drop)
        .then_some(descriptor)
}

fn bool_type(table: &TypeTable) -> Option<TypeId> {
    table
        .descriptors()
        .iter()
        .find(|descriptor| matches!(descriptor.kind, TypeKind::Bool))
        .map(|descriptor| descriptor.id)
}

fn is_byte_slice_pointer(table: &TypeTable, ty: TypeId, element: TypeId) -> bool {
    if table.int_info(element) != Some((false, 8)) {
        return false;
    }
    let pointee = match table.get(ty).map(|descriptor| &descriptor.kind) {
        Some(TypeKind::Reference {
            pointee,
            mutable: false,
        })
        | Some(TypeKind::RawPointer {
            pointee,
            mutable: false,
        }) => *pointee,
        _ => return false,
    };
    matches!(table.get(pointee).map(|descriptor| &descriptor.kind), Some(TypeKind::Slice(found)) if *found == element)
}

fn aggregate(
    func: &Func,
    defs: &[Option<Site>],
    uses: &[usize],
    value: ValueId,
    ty: TypeId,
    unique: bool,
) -> Option<(ValueId, Vec<ValueId>)> {
    let mut current = value;
    for _ in 0..8 {
        let op = definition(func, defs, current)?;
        match op.kind {
            OpKind::Copy(source) | OpKind::Move(source) => {
                if value_ty(func, source)? != ty || *uses.get(source.0 as usize)? != 1 {
                    return None;
                }
                current = source;
            }
            OpKind::Aggregate {
                ty: found,
                variant: 0,
                active_field: None,
                ref fields,
            } if found == ty && (!unique || *uses.get(current.0 as usize)? == 1) => {
                return Some((current, fields.clone()));
            }
            _ => return None,
        }
    }
    None
}

fn find_header(func: &Func, outer: ValueId, types: AdapterTypes) -> Option<HeaderMatch> {
    let mut found = Vec::new();
    for block in &func.blocks {
        let ops = &block.ops;
        let matched = (|| {
            let (access, n, suffix) = if let Some((n, suffix)) = direct_header(ops, outer) {
                (AdapterAccess::Direct, n, suffix)
            } else {
                let (pointer, n, suffix) = borrowed_header(func, ops, outer, types)?;
                (AdapterAccess::Borrowed(pointer), n, suffix)
            };
            let (scrut, scrut_ty) = match types.adapter {
                Adapter::Take if suffix.is_empty() => (n, types.usize_ty),
                Adapter::Skip if suffix.len() == 2 => {
                    let zero = const_result(&suffix[0], 0)?;
                    let cond = suffix[1].result?;
                    if !matches!(suffix[1].kind, OpKind::Binary { op: BinaryOp::Gt, l, r, ty } if l == n && r == zero && ty == types.usize_ty)
                        || value_ty(func, cond)? != types.bool_ty
                    {
                        return None;
                    }
                    (cond, types.bool_ty)
                }
                _ => return None,
            };
            let (zero, nonzero) = zero_switch_targets(&block.term, scrut, scrut_ty)?;
            Some(HeaderMatch {
                block: block.id.0 as usize,
                access,
                continue_to: match types.adapter {
                    Adapter::Take => nonzero,
                    Adapter::Skip => zero,
                },
            })
        })();
        if let Some(matched) = matched {
            found.push(matched);
        }
    }
    let [found] = found.as_slice() else {
        return None;
    };
    Some(*found)
}

fn direct_header(ops: &[Op], outer: ValueId) -> Option<(ValueId, &[Op])> {
    let inner = field_result(ops.first()?, outer, 0)?;
    let n = field_result(ops.get(1)?, inner, 1)?;
    Some((n, &ops[2..]))
}

fn borrowed_header<'a>(
    func: &Func,
    ops: &'a [Op],
    outer: ValueId,
    types: AdapterTypes,
) -> Option<(ValueId, ValueId, &'a [Op])> {
    let pointer = borrow_empty_result(ops.first()?, outer, true)?;
    let inner_pointer = borrow_deref_field_result(ops.get(1)?, pointer, 0, types.inner, true)?;
    let inner = deref_result(ops.get(2)?, inner_pointer)?;
    let n = field_result(ops.get(3)?, inner, 1)?;
    if value_ty(func, inner)? != types.inner || value_ty(func, n)? != types.usize_ty {
        return None;
    }
    Some((pointer, n, &ops[4..]))
}

fn zero_switch_targets(term: &RTerm, scrut: ValueId, ty: TypeId) -> Option<(RBlockId, RBlockId)> {
    match term {
        RTerm::Switch {
            scrut: found,
            scrut_ty,
            cases,
            default,
        } if *found == scrut && *scrut_ty == ty && default.1.is_empty() => match cases.as_slice() {
            [(0, target, args)] if args.is_empty() => Some((*target, default.0)),
            _ => None,
        },
        _ => None,
    }
}

fn match_take_body(
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    outer: ValueId,
    types: AdapterTypes,
    block: usize,
) -> Option<BodyMatch> {
    let body = func.blocks.get(block)?;
    let ops = &body.ops;
    if !body.params.is_empty() || ops.len() < 19 {
        return None;
    }
    let one = const_result(&ops[0], 1)?;
    let offset = binary_result(&ops[1], BinaryOp::Offset)?;
    let (current, step) = match ops[1].kind {
        OpKind::Binary { l, r, .. } if r == one => (l, offset),
        _ => return None,
    };
    let advanced = cast_result(&ops[2], step)?;
    let state_ptr = borrow_result(&ops[3], outer, true)?;
    if !matches!(ops[4].kind, OpKind::StoreThrough { ptr, v } if ptr == state_ptr && v == advanced)
        || ops[4].result.is_some()
    {
        return None;
    }
    let (item_current, item_ptr) = cast_source_result(&ops[5])?;
    if strip_pointer_aliases(func, defs, item_current)?
        != strip_pointer_aliases(func, defs, current)?
    {
        return None;
    }
    let item_ref = borrow_deref_result(&ops[6], item_ptr, false)?;
    let update = count_update(func, &ops[7..18], outer, types.usize_ty)?;
    let old_byte = deref_result(&ops[18], item_ref)?;
    let item_ref_ty = value_ty(func, item_ref)?;
    let byte_ty = value_ty(func, old_byte)?;
    if byte_ty != types.element || !is_ref_to(table, item_ref_ty, byte_ty) {
        return None;
    }
    Some(BodyMatch {
        driver: block,
        update: None,
        body: block,
        prefix: 19,
        driver_prefix: 19,
        pass_item_ref: false,
        old_index: update,
        old_byte,
        item_ref_ty,
        byte_ty,
        origin: ops[18].origin,
    })
}

fn match_skip_body(
    func: &Func,
    table: &TypeTable,
    outer: ValueId,
    types: AdapterTypes,
    block: usize,
) -> Option<BodyMatch> {
    let body = func.blocks.get(block)?;
    let [item_ref] = body.params.as_slice() else {
        return None;
    };
    if body.ops.len() < 12 {
        return None;
    }
    let old_index = count_update(func, &body.ops[..11], outer, types.usize_ty)?;
    let old_byte = deref_result(&body.ops[11], *item_ref)?;
    let item_ref_ty = value_ty(func, *item_ref)?;
    let byte_ty = value_ty(func, old_byte)?;
    if byte_ty != types.element || !is_ref_to(table, item_ref_ty, byte_ty) {
        return None;
    }
    Some(BodyMatch {
        driver: block,
        update: None,
        body: block,
        prefix: 12,
        driver_prefix: 12,
        pass_item_ref: true,
        old_index,
        old_byte,
        item_ref_ty,
        byte_ty,
        origin: body.ops[11].origin,
    })
}

fn match_borrowed_body(
    func: &Func,
    table: &TypeTable,
    pointer: ValueId,
    outer_ty: TypeId,
    types: AdapterTypes,
    block: usize,
) -> Option<BodyMatch> {
    let driver = func.blocks.get(block)?;
    let [item_ref] = driver.params.as_slice() else {
        return None;
    };
    let (old_index, next) =
        borrowed_count_prefix(func, &driver.ops, pointer, outer_ty, types.usize_ty)?;
    let (update_match, body_id) = if driver.ops.len() == 15 {
        if !borrowed_count_store(&driver.ops[12..], pointer, next, types.usize_ty) {
            return None;
        }
        let RTerm::Goto(body_id, args) = &driver.term else {
            return None;
        };
        if !args.is_empty() {
            return None;
        }
        (None, *body_id)
    } else {
        let RTerm::Goto(update_id, args) = &driver.term else {
            return None;
        };
        if !args.is_empty() {
            return None;
        }
        let update_index = update_id.0 as usize;
        let update = func.blocks.get(update_index)?;
        if !update.params.is_empty()
            || !borrowed_count_store(&update.ops, pointer, next, types.usize_ty)
        {
            return None;
        }
        let RTerm::Goto(body_id, args) = &update.term else {
            return None;
        };
        if !args.is_empty() {
            return None;
        }
        (Some((update_index, update.ops.len())), *body_id)
    };
    let body_index = body_id.0 as usize;
    let body = func.blocks.get(body_index)?;
    if !body.params.is_empty() {
        return None;
    }
    let deref = body.ops.first()?;
    let old_byte = deref_result(deref, *item_ref)?;
    let item_ref_ty = value_ty(func, *item_ref)?;
    let byte_ty = value_ty(func, old_byte)?;
    if byte_ty != types.element || !is_ref_to(table, item_ref_ty, byte_ty) {
        return None;
    }
    Some(BodyMatch {
        driver: block,
        update: update_match,
        body: body_index,
        prefix: 1,
        driver_prefix: driver.ops.len(),
        pass_item_ref: false,
        old_index,
        old_byte,
        item_ref_ty,
        byte_ty,
        origin: deref.origin,
    })
}

fn borrowed_count_prefix(
    func: &Func,
    ops: &[Op],
    pointer: ValueId,
    outer_ty: TypeId,
    usize_ty: TypeId,
) -> Option<(ValueId, ValueId)> {
    if ops.len() != 12 && ops.len() != 15 {
        return None;
    }
    let first = deref_result(&ops[0], pointer)?;
    let index = field_result(&ops[1], first, 1)?;
    let second = deref_result(&ops[2], pointer)?;
    let before = field_result(&ops[3], second, 1)?;
    let one = const_result(&ops[4], 1)?;
    let next = binary_result(&ops[5], BinaryOp::Add)?;
    if !matches!(ops[5].kind, OpKind::Binary { l, r, ty, .. } if l == before && r == one && ty == usize_ty)
    {
        return None;
    }
    let overflow = binary_result(&ops[6], BinaryOp::Lt)?;
    if !matches!(ops[6].kind, OpKind::Binary { l, r, ty, .. } if l == next && r == before && ty == usize_ty)
    {
        return None;
    }
    let checked = copy_result(&ops[7], overflow)?;
    let third = deref_result(&ops[8], pointer)?;
    let assert_left = field_result(&ops[9], third, 1)?;
    let assert_one = const_result(&ops[10], 1)?;
    if !matches!(&ops[11].kind,
        OpKind::Assert {
            cond,
            expected: false,
            msg: crate::jir::repr::RAssertMessage::Overflow {
                op: BinaryOp::Add,
                left,
                right,
            },
        } if *cond == checked && *left == assert_left && *right == assert_one)
        || ops[11].result.is_some()
    {
        return None;
    }
    if value_ty(func, first)? != outer_ty
        || value_ty(func, second)? != outer_ty
        || value_ty(func, third)? != outer_ty
        || value_ty(func, index)? != usize_ty
    {
        return None;
    }
    Some((index, next))
}

fn borrowed_count_store(ops: &[Op], pointer: ValueId, next: ValueId, usize_ty: TypeId) -> bool {
    let Some(stored) = ops.first().and_then(|op| copy_result(op, next)) else {
        return false;
    };
    let Some(state) = ops
        .get(1)
        .and_then(|op| borrow_deref_field_result(op, pointer, 1, usize_ty, true))
    else {
        return false;
    };
    ops.len() == 3
        && matches!(ops[2].kind, OpKind::StoreThrough { ptr, v } if ptr == state && v == stored)
        && ops[2].result.is_none()
}

fn count_update(func: &Func, ops: &[Op], outer: ValueId, usize_ty: TypeId) -> Option<ValueId> {
    if ops.len() != 11 {
        return None;
    }
    let index = field_result(&ops[0], outer, 1)?;
    let before = field_result(&ops[1], outer, 1)?;
    let one = const_result(&ops[2], 1)?;
    let next = binary_result(&ops[3], BinaryOp::Add)?;
    if !matches!(ops[3].kind, OpKind::Binary { l, r, ty, .. } if l == before && r == one && ty == usize_ty)
    {
        return None;
    }
    let overflow = binary_result(&ops[4], BinaryOp::Lt)?;
    if !matches!(ops[4].kind, OpKind::Binary { l, r, ty, .. } if l == next && r == before && ty == usize_ty)
    {
        return None;
    }
    let checked = copy_result(&ops[5], overflow)?;
    let assert_left = field_result(&ops[6], outer, 1)?;
    let assert_one = const_result(&ops[7], 1)?;
    if !matches!(&ops[8].kind,
        OpKind::Assert {
            cond,
            expected: false,
            msg: crate::jir::repr::RAssertMessage::Overflow {
                op: BinaryOp::Add,
                left,
                right,
            },
        } if *cond == checked && *left == assert_left && *right == assert_one)
        || ops[8].result.is_some()
    {
        return None;
    }
    let stored = copy_result(&ops[9], next)?;
    if !matches!(ops[10].kind, OpKind::SetField { base, variant: None, field: 1, v } if base == outer && v == stored)
        || ops[10].result.is_some()
        || value_ty(func, index)? != usize_ty
    {
        return None;
    }
    Some(index)
}

fn removed_prefix_is_local(
    func: &Func,
    matched: &BodyMatch,
    rewrite_blocks: &BTreeSet<usize>,
) -> bool {
    let mut removed = BTreeSet::new();
    removed.extend(
        func.blocks[matched.driver].ops[..matched.driver_prefix]
            .iter()
            .filter_map(|op| op.result),
    );
    if let Some((block, prefix)) = matched.update {
        removed.extend(
            func.blocks[block].ops[..prefix]
                .iter()
                .filter_map(|op| op.result),
        );
    }
    if matched.body != matched.driver {
        removed.extend(
            func.blocks[matched.body].ops[..matched.prefix]
                .iter()
                .filter_map(|op| op.result),
        );
    }
    let allowed = BTreeSet::from([matched.old_index, matched.old_byte]);
    for (block_index, block) in func.blocks.iter().enumerate() {
        for (op_index, op) in block.ops.iter().enumerate() {
            let in_removed = (block_index == matched.driver && op_index < matched.driver_prefix)
                || matched
                    .update
                    .is_some_and(|(block, prefix)| block_index == block && op_index < prefix)
                || (block_index == matched.body && op_index < matched.prefix);
            if in_removed {
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
            if !rewrite_blocks.contains(&block_index)
                && op.kind.uses().iter().any(|value| allowed.contains(value))
            {
                return false;
            }
        }
        let term_uses: Vec<ValueId> = block
            .term
            .uses()
            .into_iter()
            .filter(|value| removed.contains(value))
            .collect();
        if !term_uses.is_empty()
            && block_index != matched.driver
            && !matched
                .update
                .is_some_and(|(block, _)| block_index == block)
            && (!rewrite_blocks.contains(&block_index)
                || term_uses.iter().any(|value| !allowed.contains(value)))
        {
            return false;
        }
    }
    true
}

fn tainted_values(
    func: &Func,
    outer: ValueId,
    old_index: ValueId,
    old_byte: ValueId,
) -> BTreeSet<ValueId> {
    let mut tainted = BTreeSet::from([outer]);
    for _ in 0..func.values.len().max(1) {
        let before = tainted.len();
        for block in &func.blocks {
            for (target, args) in block.term.edges() {
                for (position, value) in args.iter().enumerate() {
                    if tainted.contains(value) {
                        if let Some(param) = func.blocks[target.0 as usize].params.get(position) {
                            tainted.insert(*param);
                        }
                    }
                }
            }
            for op in &block.ops {
                if op.kind.uses().iter().any(|value| tainted.contains(value)) {
                    if let Some(result) = op.result {
                        if result != old_index && result != old_byte {
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

fn non_escaping(
    func: &Func,
    defs: &[Option<Site>],
    loop_blocks: &BTreeSet<usize>,
    tainted: &BTreeSet<ValueId>,
) -> bool {
    for block in &func.blocks {
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
                OpKind::Intrinsic { fallback: None, .. } => op.result.is_some(),
                OpKind::SetField { base, .. } => tainted.contains(base),
                OpKind::StoreThrough { ptr, .. } => {
                    tainted.contains(ptr) && mutable_borrow(func, defs, *ptr)
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
        if term_uses_tainted && !loop_blocks.contains(&(block.id.0 as usize)) {
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

fn mutable_borrow(func: &Func, defs: &[Option<Site>], mut value: ValueId) -> bool {
    for _ in 0..8 {
        match &definition(func, defs, value).map(|op| &op.kind) {
            Some(OpKind::Copy(source) | OpKind::Move(source)) => value = *source,
            Some(OpKind::BorrowView { mutable: true, .. }) => return true,
            _ => return false,
        }
    }
    false
}

fn common_exit(func: &Func, scc: &BTreeSet<usize>, exits: &BTreeSet<RBlockId>) -> Option<RBlockId> {
    if exits.is_empty() {
        return None;
    }
    let distances: Vec<Vec<Option<usize>>> =
        exits.iter().map(|start| distances(func, *start)).collect();
    (0..func.blocks.len())
        .filter(|block| !scc.contains(block) && distances.iter().all(|row| row[*block].is_some()))
        .min_by_key(|block| {
            let ds: Vec<usize> = distances.iter().filter_map(|row| row[*block]).collect();
            (
                ds.iter().copied().max().unwrap_or(usize::MAX),
                ds.iter().sum::<usize>(),
                *block,
            )
        })
        .map(|block| RBlockId(block as u32))
}

fn discardable_exit_chain(
    func: &Func,
    defs: &[Option<Site>],
    tainted: &BTreeSet<ValueId>,
    mut at: RBlockId,
    exit: RBlockId,
) -> bool {
    let mut seen = BTreeSet::new();
    while at != exit {
        if !seen.insert(at) {
            return false;
        }
        let Some(block) = func.blocks.get(at.0 as usize) else {
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
                .any(|op| !discardable_tainted_op(func, defs, tainted, op))
        {
            return false;
        }
        at = *next;
    }
    true
}

fn discardable_tainted_op(
    func: &Func,
    defs: &[Option<Site>],
    tainted: &BTreeSet<ValueId>,
    op: &Op,
) -> bool {
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
            op.result.is_none() && tainted.contains(ptr) && mutable_borrow(func, defs, *ptr)
        }
        _ => false,
    }
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

fn reverse_reachable(func: &Func, target: RBlockId) -> Vec<bool> {
    let mut predecessors = vec![Vec::new(); func.blocks.len()];
    for block in &func.blocks {
        for successor in block.term.successors() {
            predecessors[successor.0 as usize].push(block.id);
        }
    }
    let mut seen = vec![false; func.blocks.len()];
    seen[target.0 as usize] = true;
    let mut queue = VecDeque::from([target]);
    while let Some(block) = queue.pop_front() {
        for predecessor in &predecessors[block.0 as usize] {
            if !seen[predecessor.0 as usize] {
                seen[predecessor.0 as usize] = true;
                queue.push_back(*predecessor);
            }
        }
    }
    seen
}

fn reaches_avoiding(func: &Func, start: RBlockId, target: RBlockId, avoid: usize) -> bool {
    let mut seen = vec![false; func.blocks.len()];
    let mut queue = VecDeque::from([start]);
    while let Some(block) = queue.pop_front() {
        let index = block.0 as usize;
        if index == avoid || seen.get(index).copied().unwrap_or(true) {
            continue;
        }
        if block == target {
            return true;
        }
        seen[index] = true;
        queue.extend(func.blocks[index].term.successors());
    }
    false
}

fn apply(func: &mut Func, found: LoopMatch) -> SliceIteratorLoop {
    let cursor = fresh_like(func, found.initial, found.origin);
    let body_cursor = fresh_like(func, found.initial, found.origin);
    let less_len = fresh_bool(func, found.bool_ty, found.origin);
    let less_limit =
        (found.adapter == Adapter::Take).then(|| fresh_bool(func, found.bool_ty, found.origin));
    let condition = less_limit.map_or(less_len, |_| fresh_bool(func, found.bool_ty, found.origin));
    let index = match found.adapter {
        Adapter::Take => body_cursor,
        Adapter::Skip => fresh_like(func, found.initial, found.origin),
    };
    let item_ref = fresh_with_type(func, found.item_ref_ty, found.origin);
    let byte = fresh_with_type(func, found.byte_ty, found.origin);
    let one = fresh_like(func, found.initial, found.origin);
    let next = fresh_like(func, found.initial, found.origin);
    let direct = RBlockId(func.blocks.len() as u32);

    let mut header_ops = vec![Op {
        result: Some(less_len),
        origin: found.origin,
        kind: OpKind::Binary {
            op: BinaryOp::Lt,
            l: cursor,
            r: found.len,
            ty: found.usize_ty,
        },
    }];
    if let Some(less_limit) = less_limit {
        header_ops.push(Op {
            result: Some(less_limit),
            origin: found.origin,
            kind: OpKind::Binary {
                op: BinaryOp::Lt,
                l: cursor,
                r: found.limit,
                ty: found.usize_ty,
            },
        });
        header_ops.push(Op {
            result: Some(condition),
            origin: found.origin,
            kind: OpKind::Binary {
                op: BinaryOp::BitAnd,
                l: less_len,
                r: less_limit,
                ty: found.bool_ty,
            },
        });
    }
    func.blocks[found.header].params.push(cursor);
    func.blocks[found.header].ops = header_ops;
    func.blocks[found.header].term = RTerm::Switch {
        scrut: condition,
        scrut_ty: found.bool_ty,
        cases: vec![(0, found.exit, Vec::new())],
        default: (direct, vec![cursor]),
    };

    edge_to(&mut func.blocks[found.preheader].term, found.header)
        .expect("a matched preheader edge exists")
        .push(found.initial);
    for backedge in &found.backedges {
        edge_to(&mut func.blocks[*backedge].term, found.header)
            .expect("a matched backedge exists")
            .push(next);
    }

    let mut direct_ops = Vec::new();
    if found.adapter == Adapter::Skip {
        direct_ops.push(Op {
            result: Some(index),
            origin: found.origin,
            kind: OpKind::Binary {
                op: BinaryOp::Sub,
                l: body_cursor,
                r: found.limit,
                ty: found.usize_ty,
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
    direct_ops.push(Op {
        result: Some(byte),
        origin: found.origin,
        kind: OpKind::Deref(item_ref),
    });
    direct_ops.push(Op {
        result: Some(one),
        origin: found.origin,
        kind: OpKind::Const(ConstTree::Scalar {
            bits: 1,
            bytes: found.usize_bytes,
        }),
    });
    direct_ops.push(Op {
        result: Some(next),
        origin: found.origin,
        kind: OpKind::Binary {
            op: BinaryOp::Add,
            l: body_cursor,
            r: one,
            ty: found.usize_ty,
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
        term: RTerm::Goto(RBlockId(found.body as u32), body_args.clone()),
    });

    let tail = func.blocks[found.body].ops.split_off(found.body_prefix);
    func.blocks[found.body].ops = tail;
    let rewrite = |value: ValueId| {
        if value == found.old_index {
            index
        } else if value == found.old_byte {
            byte
        } else {
            value
        }
    };
    for block in &found.rewrite_blocks {
        if (*block == found.driver && found.driver != found.body) || found.update == Some(*block) {
            continue;
        }
        for op in &mut func.blocks[*block].ops {
            crate::lower::ssa::rewrite_op(&mut op.kind, &rewrite);
        }
        crate::lower::ssa::rewrite_term(&mut func.blocks[*block].term, &rewrite);
    }

    // The old adapter remains unreachable until DCE. Keeping structural deletion in the one
    // existing pass means block and value compaction still have a single owner.
    SliceIteratorLoop {
        adapter: match found.adapter {
            Adapter::Take => IteratorIdentity::Take,
            Adapter::Skip => IteratorIdentity::Skip,
        },
        header: RBlockId(found.header as u32),
        preheader: RBlockId(found.preheader as u32),
        backedges: found
            .backedges
            .iter()
            .map(|block| RBlockId(*block as u32))
            .collect(),
        direct,
        body: RBlockId(found.body as u32),
        exit: found.exit,
        source: found.source,
        length: found.len,
        limit: found.limit,
        initial: found.initial,
        cursor,
        length_guard: less_len,
        limit_guard: less_limit,
        condition,
        body_cursor,
        enumerate_index: index,
        item_ref,
        byte,
        one,
        next,
        body_args,
        item_ref_ty: found.item_ref_ty,
        byte_ty: found.byte_ty,
        usize_ty: found.usize_ty,
        bool_ty: found.bool_ty,
        usize_bytes: found.usize_bytes,
        origin: found.origin,
    }
}

fn edge_to(term: &mut RTerm, target: usize) -> Option<&mut Vec<ValueId>> {
    match term {
        RTerm::Goto(found, args) if found.0 as usize == target => Some(args),
        _ => None,
    }
}

fn fresh_like(func: &mut Func, like: ValueId, origin: OriginId) -> ValueId {
    let mut def = func.values[like.0 as usize];
    def.origin = origin;
    let value = ValueId(func.values.len() as u32);
    func.values.push(def);
    value
}

fn fresh_with_type(func: &mut Func, ty: TypeId, origin: OriginId) -> ValueId {
    let value = ValueId(func.values.len() as u32);
    func.values.push(ValueDef {
        ty,
        carrier: Carrier::Baseline,
        origin,
    });
    value
}

fn fresh_bool(func: &mut Func, ty: TypeId, origin: OriginId) -> ValueId {
    fresh_with_type(func, ty, origin)
}

fn definitions(func: &Func) -> Vec<Option<Site>> {
    let mut defs = vec![None; func.values.len()];
    for (block, data) in func.blocks.iter().enumerate() {
        for (op, operation) in data.ops.iter().enumerate() {
            if let Some(result) = operation.result {
                defs[result.0 as usize] = Some(Site { block, op });
            }
        }
    }
    defs
}

fn use_counts(func: &Func) -> Vec<usize> {
    let mut uses = vec![0; func.values.len()];
    for block in &func.blocks {
        for value in block
            .ops
            .iter()
            .flat_map(|op| op.kind.uses())
            .chain(block.term.uses())
        {
            uses[value.0 as usize] += 1;
        }
    }
    uses
}

fn definition<'a>(func: &'a Func, defs: &[Option<Site>], value: ValueId) -> Option<&'a Op> {
    let site = defs.get(value.0 as usize).copied().flatten()?;
    func.blocks.get(site.block)?.ops.get(site.op)
}

fn strip_copies(func: &Func, defs: &[Option<Site>], mut value: ValueId) -> Option<ValueId> {
    for _ in 0..8 {
        match definition(func, defs, value)?.kind {
            OpKind::Copy(source) | OpKind::Move(source) => value = source,
            _ => return Some(value),
        }
    }
    None
}

fn copy_root(func: &Func, defs: &[Option<Site>], mut value: ValueId) -> Option<ValueId> {
    for _ in 0..8 {
        match definition(func, defs, value).map(|op| &op.kind) {
            Some(OpKind::Copy(source) | OpKind::Move(source)) => value = *source,
            _ => return Some(value),
        }
    }
    None
}

fn source_length_matches(
    func: &Func,
    defs: &[Option<Site>],
    source: ValueId,
    len: ValueId,
) -> bool {
    let Some(source) = copy_root(func, defs, source) else {
        return false;
    };
    if matches!(definition(func, defs, source).map(|op| &op.kind), Some(OpKind::SliceView { len: found, .. }) if *found == len)
    {
        return true;
    }
    let Some(len) = copy_root(func, defs, len) else {
        return false;
    };
    matches!(definition(func, defs, len).map(|op| &op.kind), Some(OpKind::Unary { op: UnaryOp::PtrMetadata, v, .. }) if copy_root(func, defs, *v) == Some(source))
}

fn strip_pointer_aliases(
    func: &Func,
    defs: &[Option<Site>],
    mut value: ValueId,
) -> Option<ValueId> {
    for _ in 0..12 {
        match definition(func, defs, value)?.kind {
            OpKind::Copy(source) | OpKind::Move(source) => value = source,
            OpKind::Cast { v, .. } => value = v,
            _ => return Some(value),
        }
    }
    None
}

fn value_ty(func: &Func, value: ValueId) -> Option<TypeId> {
    func.values.get(value.0 as usize).map(|def| def.ty)
}

fn is_scalar(func: &Func, defs: &[Option<Site>], value: ValueId, expected: u128) -> bool {
    strip_copies(func, defs, value)
        .and_then(|value| definition(func, defs, value))
        .is_some_and(|op| matches!(op.kind, OpKind::Const(ConstTree::Scalar { bits, .. }) if bits == expected))
}

fn is_ref_to(table: &TypeTable, reference: TypeId, pointee: TypeId) -> bool {
    matches!(table.get(reference).map(|descriptor| &descriptor.kind), Some(TypeKind::Reference { pointee: found, mutable: false }) if *found == pointee)
}

fn field_result(op: &Op, base: ValueId, field: u32) -> Option<ValueId> {
    match op.kind {
        OpKind::Field {
            base: found,
            variant: None,
            field: found_field,
        } if found == base && found_field == field => op.result,
        _ => None,
    }
}

fn const_result(op: &Op, bits: u128) -> Option<ValueId> {
    matches!(op.kind, OpKind::Const(ConstTree::Scalar { bits: found, .. }) if found == bits)
        .then_some(op.result)
        .flatten()
}

fn binary_result(op: &Op, binary: BinaryOp) -> Option<ValueId> {
    matches!(op.kind, OpKind::Binary { op: found, .. } if found == binary)
        .then_some(op.result)
        .flatten()
}

fn cast_result(op: &Op, source: ValueId) -> Option<ValueId> {
    matches!(op.kind, OpKind::Cast { v, .. } if v == source)
        .then_some(op.result)
        .flatten()
}

fn cast_source_result(op: &Op) -> Option<(ValueId, ValueId)> {
    match op.kind {
        OpKind::Cast { v, .. } => Some((v, op.result?)),
        _ => None,
    }
}

fn copy_result(op: &Op, source: ValueId) -> Option<ValueId> {
    matches!(op.kind, OpKind::Copy(v) if v == source)
        .then_some(op.result)
        .flatten()
}

fn borrow_result(op: &Op, root: ValueId, mutable: bool) -> Option<ValueId> {
    matches!(&op.kind, OpKind::BorrowView { root: Root::Value(found), mutable: found_mutable, .. } if *found == root && *found_mutable == mutable)
        .then_some(op.result)
        .flatten()
}

fn borrow_empty_result(op: &Op, root: ValueId, mutable: bool) -> Option<ValueId> {
    matches!(&op.kind, OpKind::BorrowView { root: Root::Value(found), path, mutable: found_mutable } if *found == root && path.is_empty() && *found_mutable == mutable)
        .then_some(op.result)
        .flatten()
}

fn borrow_deref_field_result(
    op: &Op,
    root: ValueId,
    field: u32,
    ty: TypeId,
    mutable: bool,
) -> Option<ValueId> {
    matches!(&op.kind,
        OpKind::BorrowView {
            root: Root::Value(found),
            path,
            mutable: found_mutable,
        } if *found == root
            && path.as_slice() == [Proj::Deref, Proj::Field { field, ty }]
            && *found_mutable == mutable)
    .then_some(op.result)
    .flatten()
}

fn borrow_deref_result(op: &Op, root: ValueId, mutable: bool) -> Option<ValueId> {
    matches!(&op.kind, OpKind::BorrowView { root: Root::Value(found), path, mutable: found_mutable } if *found == root && path.as_slice() == [Proj::Deref] && *found_mutable == mutable)
        .then_some(op.result)
        .flatten()
}

fn deref_result(op: &Op, pointer: ValueId) -> Option<ValueId> {
    matches!(op.kind, OpKind::Deref(found) if found == pointer)
        .then_some(op.result)
        .flatten()
}

/// Independently checks every generated block, value, and edge recorded for this rewrite batch.
pub fn check_lowered(
    func: &Func,
    table: &TypeTable,
    expected: &[SliceIteratorLoop],
) -> Result<(), String> {
    if expected.is_empty() {
        return Err("a slice-iterator witness rewrote no loops".to_owned());
    }
    let mut headers = BTreeSet::new();
    let mut direct_blocks = BTreeSet::new();
    for loop_ in expected {
        if !headers.insert(loop_.header) {
            return Err(format!("duplicate cursor header b{}", loop_.header.0));
        }
        if !direct_blocks.insert(loop_.direct) {
            return Err(format!("duplicate cursor body b{}", loop_.direct.0));
        }
        check_loop(func, table, loop_)?;
    }
    Ok(())
}

fn check_loop(func: &Func, table: &TypeTable, loop_: &SliceIteratorLoop) -> Result<(), String> {
    let defs = definitions(func);
    let header = block(func, loop_.header, "header")?;
    let direct = block(func, loop_.direct, "direct body")?;
    block(func, loop_.body, "user body")?;
    block(func, loop_.exit, "exit")?;

    for (value, ty, name) in [
        (loop_.cursor, loop_.usize_ty, "cursor"),
        (loop_.body_cursor, loop_.usize_ty, "body cursor"),
        (loop_.enumerate_index, loop_.usize_ty, "enumerate index"),
        (loop_.length, loop_.usize_ty, "length"),
        (loop_.limit, loop_.usize_ty, "limit"),
        (loop_.initial, loop_.usize_ty, "initial cursor"),
        (loop_.length_guard, loop_.bool_ty, "length guard"),
        (loop_.condition, loop_.bool_ty, "condition"),
        (loop_.item_ref, loop_.item_ref_ty, "item reference"),
        (loop_.byte, loop_.byte_ty, "byte"),
        (loop_.one, loop_.usize_ty, "increment"),
        (loop_.next, loop_.usize_ty, "next cursor"),
    ] {
        if value_ty(func, value) != Some(ty) {
            return Err(format!("{name} v{} has the wrong type or is gone", value.0));
        }
    }
    if let Some(limit_guard) = loop_.limit_guard {
        if value_ty(func, limit_guard) != Some(loop_.bool_ty) {
            return Err(format!(
                "limit guard v{} has the wrong type or is gone",
                limit_guard.0
            ));
        }
    }
    if table
        .int_info(loop_.usize_ty)
        .is_none_or(|(signed, _)| signed)
        || table.int_info(loop_.byte_ty) != Some((false, 8))
        || !is_ref_to(table, loop_.item_ref_ty, loop_.byte_ty)
        || !source_has_length(func, table, &defs, loop_.source, loop_.length)
    {
        return Err("the cursor, byte, reference, or source-length types changed".to_owned());
    }
    if header.params.last() != Some(&loop_.cursor) {
        return Err(format!(
            "b{} does not bind cursor v{} in its final parameter",
            loop_.header.0, loop_.cursor.0
        ));
    }

    let length_guard = Op {
        result: Some(loop_.length_guard),
        origin: loop_.origin,
        kind: OpKind::Binary {
            op: BinaryOp::Lt,
            l: loop_.cursor,
            r: loop_.length,
            ty: loop_.usize_ty,
        },
    };
    let expected_header = match loop_.adapter {
        IteratorIdentity::Take => {
            let Some(limit_guard) = loop_.limit_guard else {
                return Err("a Take cursor has no limit guard".to_owned());
            };
            vec![
                length_guard,
                Op {
                    result: Some(limit_guard),
                    origin: loop_.origin,
                    kind: OpKind::Binary {
                        op: BinaryOp::Lt,
                        l: loop_.cursor,
                        r: loop_.limit,
                        ty: loop_.usize_ty,
                    },
                },
                Op {
                    result: Some(loop_.condition),
                    origin: loop_.origin,
                    kind: OpKind::Binary {
                        op: BinaryOp::BitAnd,
                        l: loop_.length_guard,
                        r: limit_guard,
                        ty: loop_.bool_ty,
                    },
                },
            ]
        }
        IteratorIdentity::Skip if loop_.limit_guard.is_none() => vec![length_guard],
        other => return Err(format!("unsupported cursor adapter {other:?}")),
    };
    if header.ops != expected_header {
        return Err(format!(
            "b{} no longer has the exact cursor guard",
            loop_.header.0
        ));
    }
    let expected_header_term = RTerm::Switch {
        scrut: loop_.condition,
        scrut_ty: loop_.bool_ty,
        cases: vec![(0, loop_.exit, Vec::new())],
        default: (loop_.direct, vec![loop_.cursor]),
    };
    if header.term != expected_header_term {
        return Err(format!(
            "b{} no longer branches to the exact body and exit",
            loop_.header.0
        ));
    }

    if direct.params.as_slice() != [loop_.body_cursor] {
        return Err(format!(
            "b{} does not bind the exact body cursor",
            loop_.direct.0
        ));
    }
    let mut expected_direct = Vec::new();
    match loop_.adapter {
        IteratorIdentity::Take if loop_.enumerate_index == loop_.body_cursor => {}
        IteratorIdentity::Skip => expected_direct.push(Op {
            result: Some(loop_.enumerate_index),
            origin: loop_.origin,
            kind: OpKind::Binary {
                op: BinaryOp::Sub,
                l: loop_.body_cursor,
                r: loop_.limit,
                ty: loop_.usize_ty,
            },
        }),
        _ => return Err("the enumerate index no longer matches its adapter".to_owned()),
    }
    expected_direct.extend([
        Op {
            result: Some(loop_.item_ref),
            origin: loop_.origin,
            kind: OpKind::BorrowView {
                root: Root::Value(loop_.source),
                path: vec![Proj::Deref, Proj::Index(loop_.body_cursor)],
                mutable: false,
            },
        },
        Op {
            result: Some(loop_.byte),
            origin: loop_.origin,
            kind: OpKind::Deref(loop_.item_ref),
        },
        Op {
            result: Some(loop_.one),
            origin: loop_.origin,
            kind: OpKind::Const(ConstTree::Scalar {
                bits: 1,
                bytes: loop_.usize_bytes,
            }),
        },
        Op {
            result: Some(loop_.next),
            origin: loop_.origin,
            kind: OpKind::Binary {
                op: BinaryOp::Add,
                l: loop_.body_cursor,
                r: loop_.one,
                ty: loop_.usize_ty,
            },
        },
    ]);
    if direct.ops != expected_direct
        || direct.term != RTerm::Goto(loop_.body, loop_.body_args.clone())
    {
        return Err(format!(
            "b{} no longer has the exact indexed body",
            loop_.direct.0
        ));
    }

    if loop_.adapter == IteratorIdentity::Take {
        if !is_scalar(func, &defs, loop_.initial, 0) {
            return Err("a Take cursor does not start at zero".to_owned());
        }
    } else if loop_.initial != loop_.limit {
        return Err("a Skip cursor does not start at its limit".to_owned());
    }

    let mut expected_predecessors = loop_.backedges.clone();
    expected_predecessors.push(loop_.preheader);
    expected_predecessors.sort();
    expected_predecessors.dedup();
    let mut predecessors = Vec::new();
    for candidate in &func.blocks {
        if let RTerm::Goto(target, args) = &candidate.term {
            if *target != loop_.header {
                continue;
            }
            let expected_value = if candidate.id == loop_.preheader {
                loop_.initial
            } else if loop_.backedges.contains(&candidate.id) {
                loop_.next
            } else {
                return Err(format!("unexpected cursor predecessor b{}", candidate.id.0));
            };
            if args.last() != Some(&expected_value) {
                return Err(format!("b{} passes the wrong cursor value", candidate.id.0));
            }
            predecessors.push(candidate.id);
        } else if candidate.term.successors().contains(&loop_.header) {
            return Err(format!("b{} has a non-Goto cursor edge", candidate.id.0));
        }
    }
    predecessors.sort();
    predecessors.dedup();
    if predecessors != expected_predecessors {
        return Err(format!("b{} has the wrong predecessor set", loop_.header.0));
    }
    Ok(())
}

fn block<'a>(func: &'a Func, id: RBlockId, name: &str) -> Result<&'a RBlock, String> {
    func.blocks
        .get(id.0 as usize)
        .filter(|block| block.id == id)
        .ok_or_else(|| format!("the {name} b{} is gone", id.0))
}

fn source_has_length(
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    source: ValueId,
    len: ValueId,
) -> bool {
    let Some(source) = copy_root(func, defs, source) else {
        return false;
    };
    if !is_byte_slice_pointer(
        table,
        match value_ty(func, source) {
            Some(ty) => ty,
            None => return false,
        },
        match value_ty(func, source)
            .and_then(|ty| table.get(ty))
            .map(|descriptor| &descriptor.kind)
        {
            Some(TypeKind::Reference { pointee, .. })
            | Some(TypeKind::RawPointer { pointee, .. }) => {
                match table.get(*pointee).map(|descriptor| &descriptor.kind) {
                    Some(TypeKind::Slice(element)) => *element,
                    _ => return false,
                }
            }
            _ => return false,
        },
    ) {
        return false;
    }
    source_length_matches(func, defs, source, len)
}

#[cfg(test)]
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
    const SLICE_ITER: TypeId = TypeId(9);
    const TAKE: TypeId = TypeId(10);
    const SKIP: TypeId = TypeId(11);
    const ENUM_TAKE: TypeId = TypeId(12);
    const ENUM_SKIP: TypeId = TypeId(13);
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
        table.insert(aggregate_type(
            SLICE_ITER,
            vec![U8],
            vec![field(RAW_U8, 0), field(RAW_U8, 1), field(ZST, 2)],
            IteratorIdentity::Slice,
        ));
        table.insert(aggregate_type(
            TAKE,
            vec![SLICE_ITER],
            vec![field(SLICE_ITER, 0), field(USIZE, 1)],
            IteratorIdentity::Take,
        ));
        table.insert(aggregate_type(
            SKIP,
            vec![SLICE_ITER],
            vec![field(SLICE_ITER, 0), field(USIZE, 1)],
            IteratorIdentity::Skip,
        ));
        table.insert(aggregate_type(
            ENUM_TAKE,
            vec![TAKE],
            vec![field(TAKE, 0), field(USIZE, 1)],
            IteratorIdentity::Enumerate,
        ));
        table.insert(aggregate_type(
            ENUM_SKIP,
            vec![SKIP],
            vec![field(SKIP, 0), field(USIZE, 1)],
            IteratorIdentity::Enumerate,
        ));
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

    fn add_count_update(builder: &mut FuncBuilder, block: RBlockId, outer: ValueId) -> ValueId {
        let index = builder.op(
            block,
            OpKind::Field {
                base: outer,
                variant: None,
                field: 1,
            },
            USIZE,
            O,
        );
        let before = builder.op(
            block,
            OpKind::Field {
                base: outer,
                variant: None,
                field: 1,
            },
            USIZE,
            O,
        );
        let one = constant(builder, block, 1);
        let next = builder.op(
            block,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: before,
                r: one,
                ty: USIZE,
            },
            USIZE,
            O,
        );
        let overflow = builder.op(
            block,
            OpKind::Binary {
                op: BinaryOp::Lt,
                l: next,
                r: before,
                ty: USIZE,
            },
            BOOL,
            O,
        );
        let checked = builder.op(block, OpKind::Copy(overflow), BOOL, O);
        let assert_left = builder.op(
            block,
            OpKind::Field {
                base: outer,
                variant: None,
                field: 1,
            },
            USIZE,
            O,
        );
        let assert_one = constant(builder, block, 1);
        builder.effect(
            block,
            OpKind::Assert {
                cond: checked,
                expected: false,
                msg: RAssertMessage::Overflow {
                    op: BinaryOp::Add,
                    left: assert_left,
                    right: assert_one,
                },
            },
            O,
        );
        let stored = builder.op(block, OpKind::Copy(next), USIZE, O);
        builder.effect(
            block,
            OpKind::SetField {
                base: outer,
                variant: None,
                field: 1,
                v: stored,
            },
            O,
        );
        index
    }

    fn direct_loop(adapter: Adapter, swapped_header_edges: bool) -> (Func, TypeTable) {
        let outer_ty = match adapter {
            Adapter::Take => ENUM_TAKE,
            Adapter::Skip => ENUM_SKIP,
        };
        let inner_ty = match adapter {
            Adapter::Take => TAKE,
            Adapter::Skip => SKIP,
        };
        let mut builder = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 1,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 2,
                    ..AbiSignature::default()
                },
                arguments: vec![REF_SLICE, USIZE],
                result: U8,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let header = builder.block();
        let body = builder.block();
        let exit = builder.block();
        let continue_block = (adapter == Adapter::Skip).then(|| builder.block());

        let source = builder.param(REF_SLICE, O);
        let limit = builder.param(USIZE, O);
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
        let zst = builder.op(entry, OpKind::Const(ConstTree::Unit), ZST, O);
        let slice = builder.op(
            entry,
            OpKind::Aggregate {
                ty: SLICE_ITER,
                variant: 0,
                active_field: None,
                fields: vec![raw, end, zst],
            },
            SLICE_ITER,
            O,
        );
        let initial = constant(&mut builder, entry, 0);
        let inner = builder.op(
            entry,
            OpKind::Aggregate {
                ty: inner_ty,
                variant: 0,
                active_field: None,
                fields: vec![slice, limit],
            },
            inner_ty,
            O,
        );
        let outer = builder.op(
            entry,
            OpKind::Aggregate {
                ty: outer_ty,
                variant: 0,
                active_field: None,
                fields: vec![inner, initial],
            },
            outer_ty,
            O,
        );
        builder.terminate(entry, RTerm::Goto(header, Vec::new()));

        let header_inner = builder.op(
            header,
            OpKind::Field {
                base: outer,
                variant: None,
                field: 0,
            },
            inner_ty,
            O,
        );
        let remaining = builder.op(
            header,
            OpKind::Field {
                base: header_inner,
                variant: None,
                field: 1,
            },
            USIZE,
            O,
        );
        let scrut = match adapter {
            Adapter::Take => remaining,
            Adapter::Skip => {
                let zero = constant(&mut builder, header, 0);
                builder.op(
                    header,
                    OpKind::Binary {
                        op: BinaryOp::Gt,
                        l: remaining,
                        r: zero,
                        ty: USIZE,
                    },
                    BOOL,
                    O,
                )
            }
        };
        let continue_to = continue_block.unwrap_or(body);
        let expected_edges = match adapter {
            Adapter::Take => (exit, continue_to),
            Adapter::Skip => (continue_to, exit),
        };
        let (zero_to, default_to) = if swapped_header_edges {
            (expected_edges.1, expected_edges.0)
        } else {
            expected_edges
        };
        builder.terminate(
            header,
            RTerm::Switch {
                scrut,
                scrut_ty: match adapter {
                    Adapter::Take => USIZE,
                    Adapter::Skip => BOOL,
                },
                cases: vec![(0, zero_to, Vec::new())],
                default: (default_to, Vec::new()),
            },
        );

        let body_item_ref = match adapter {
            Adapter::Take => {
                let one = constant(&mut builder, body, 1);
                let step = builder.op(
                    body,
                    OpKind::Binary {
                        op: BinaryOp::Offset,
                        l: raw,
                        r: one,
                        ty: RAW_U8,
                    },
                    RAW_U8,
                    O,
                );
                let advanced = builder.op(
                    body,
                    OpKind::Cast {
                        kind: CastKind::PtrToPtr,
                        v: step,
                        from: RAW_U8,
                        to: RAW_U8,
                    },
                    RAW_U8,
                    O,
                );
                let state = builder.op(
                    body,
                    OpKind::BorrowView {
                        root: Root::Value(outer),
                        path: Vec::new(),
                        mutable: true,
                    },
                    REF_U8,
                    O,
                );
                builder.effect(
                    body,
                    OpKind::StoreThrough {
                        ptr: state,
                        v: advanced,
                    },
                    O,
                );
                let item_pointer = builder.op(
                    body,
                    OpKind::Cast {
                        kind: CastKind::PtrToPtr,
                        v: raw,
                        from: RAW_U8,
                        to: RAW_U8,
                    },
                    RAW_U8,
                    O,
                );
                builder.op(
                    body,
                    OpKind::BorrowView {
                        root: Root::Value(item_pointer),
                        path: vec![Proj::Deref],
                        mutable: false,
                    },
                    REF_U8,
                    O,
                )
            }
            Adapter::Skip => builder.block_param(body, REF_U8, O),
        };
        let old_index = add_count_update(&mut builder, body, outer);
        let old_byte = builder.op(body, OpKind::Deref(body_item_ref), U8, O);
        builder.op(body, OpKind::Copy(old_index), USIZE, O);
        builder.op(body, OpKind::Copy(old_byte), U8, O);
        builder.terminate(body, RTerm::Goto(header, Vec::new()));

        if let Some(continue_block) = continue_block {
            let item_ref = builder.op(
                continue_block,
                OpKind::BorrowView {
                    root: Root::Value(source),
                    path: vec![Proj::Deref, Proj::Index(initial)],
                    mutable: false,
                },
                REF_U8,
                O,
            );
            builder.terminate(continue_block, RTerm::Goto(body, vec![item_ref]));
        }
        builder.terminate(exit, RTerm::Return(None));
        (builder.finish(), table())
    }

    fn lowered(adapter: Adapter) -> (Func, TypeTable, Vec<SliceIteratorLoop>) {
        let (mut func, table) = direct_loop(adapter, false);
        let loops = run(&mut func, &table);
        assert_eq!(loops.len(), 1);
        check_lowered(&func, &table, &loops).unwrap();
        (func, table, loops)
    }

    #[test]
    fn direct_take_rewrites_the_remaining_user_body() {
        let (func, _table, loops) = lowered(Adapter::Take);
        let loop_ = &loops[0];
        let body = &func.blocks[loop_.body.0 as usize];
        assert!(matches!(body.ops[0].kind, OpKind::Copy(value) if value == loop_.enumerate_index));
        assert!(matches!(body.ops[1].kind, OpKind::Copy(value) if value == loop_.byte));
    }

    #[test]
    fn direct_skip_rewrites_the_remaining_user_body() {
        let (func, _table, loops) = lowered(Adapter::Skip);
        let loop_ = &loops[0];
        let body = &func.blocks[loop_.body.0 as usize];
        assert!(matches!(body.ops[0].kind, OpKind::Copy(value) if value == loop_.enumerate_index));
        assert!(matches!(body.ops[1].kind, OpKind::Copy(value) if value == loop_.byte));
    }

    #[test]
    fn the_zero_and_continue_header_edges_cannot_be_swapped() {
        let (mut func, table) = direct_loop(Adapter::Take, true);
        assert!(run(&mut func, &table).is_empty());
    }

    #[test]
    fn a_mismatched_source_length_declines() {
        let (mut func, table) = direct_loop(Adapter::Take, false);
        let limit = func.params[1];
        let end = &mut func.blocks[FuncBuilder::ENTRY.0 as usize].ops[2];
        let OpKind::Binary { r, .. } = &mut end.kind else {
            panic!("the synthetic slice end changed shape");
        };
        *r = limit;
        assert!(run(&mut func, &table).is_empty());
    }

    #[test]
    fn a_second_preheader_declines() {
        let (mut func, table) = direct_loop(Adapter::Take, false);
        let header = func.blocks[FuncBuilder::ENTRY.0 as usize].term.successors()[0];
        let id = RBlockId(func.blocks.len() as u32);
        func.blocks.push(RBlock {
            id,
            params: Vec::new(),
            ops: Vec::new(),
            term: RTerm::Goto(header, Vec::new()),
        });
        assert!(run(&mut func, &table).is_empty());
    }

    #[test]
    fn escaping_iterator_state_declines() {
        let (mut func, table) = direct_loop(Adapter::Take, false);
        let outer = func.blocks[FuncBuilder::ENTRY.0 as usize]
            .ops
            .last()
            .and_then(|op| op.result)
            .expect("the synthetic adapter aggregate");
        let exit = func.blocks.len() - 1;
        func.blocks[exit].term = RTerm::Return(Some(outer));
        assert!(run(&mut func, &table).is_empty());
    }

    #[test]
    fn exact_witness_rejects_wrong_source_guard_and_initial_cursor() {
        let (func, table, loops) = lowered(Adapter::Take);

        let mut wrong_source = loops.clone();
        wrong_source[0].length = wrong_source[0].limit;
        assert!(check_lowered(&func, &table, &wrong_source).is_err());

        let mut missing_limit_guard = loops.clone();
        missing_limit_guard[0].limit_guard = None;
        assert!(check_lowered(&func, &table, &missing_limit_guard).is_err());

        let mut wrong_initial = loops;
        wrong_initial[0].initial = wrong_initial[0].limit;
        assert!(check_lowered(&func, &table, &wrong_initial).is_err());
    }

    #[test]
    fn exact_witness_rejects_wrong_skip_enumerate_index() {
        let (func, table, mut loops) = lowered(Adapter::Skip);
        loops[0].enumerate_index = loops[0].body_cursor;
        assert!(check_lowered(&func, &table, &loops).is_err());
    }

    #[test]
    fn exact_witness_rejects_an_unrecorded_predecessor() {
        let (mut func, table, loops) = lowered(Adapter::Take);
        let id = RBlockId(func.blocks.len() as u32);
        func.blocks.push(RBlock {
            id,
            params: Vec::new(),
            ops: Vec::new(),
            term: RTerm::Goto(loops[0].header, vec![loops[0].initial]),
        });
        assert!(check_lowered(&func, &table, &loops).is_err());
    }

    #[test]
    fn one_valid_loop_cannot_mask_one_malformed_witness() {
        let (func, table, mut loops) = lowered(Adapter::Take);
        let mut malformed = loops[0].clone();
        malformed.header = malformed.exit;
        malformed.direct = malformed.exit;
        loops.push(malformed);
        assert!(check_lowered(&func, &table, &loops).is_err());
    }

    #[test]
    fn complete_rewrite_regions_and_preheaders_cannot_overlap() {
        let mut occupied = BTreeSet::new();
        assert!(reserve_region(&BTreeSet::from([1, 2, 3]), 0, &mut occupied));
        assert!(!reserve_region(&BTreeSet::from([3, 4]), 5, &mut occupied));
        assert!(!reserve_region(&BTreeSet::from([6, 7]), 2, &mut occupied));
        assert!(reserve_region(&BTreeSet::from([8, 9]), 10, &mut occupied));
    }
}
