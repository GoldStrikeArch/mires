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

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::jir::repr::{
    Carrier, Dominators, Func, Op, OpKind, Proj, RAssertMessage, RBlock, RBlockId, RTerm, Root,
    ValueDef, ValueId,
};
use crate::jir::semantic::{BinaryOp, CallSemantic, Callee, CastKind, ConstTree, TypeId, UnaryOp};
use crate::jir::types::{IntRepr, IteratorIdentity, NominalForm, PinnedBoundary, TypeKind};
use crate::jir::{OriginId, TypeTable};

use super::witness::SliceIteratorLoop;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Adapter {
    Plain,
    Enumerate,
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
    removed_ops: BTreeMap<usize, BTreeSet<usize>>,
    removed_header_params: Vec<usize>,
    removed_invariant_params: BTreeMap<usize, Vec<usize>>,
    rewrite_blocks: BTreeSet<usize>,
    pass_item_ref: bool,
    exit: RBlockId,
    source: ValueId,
    len: ValueId,
    limit: ValueId,
    initial: Option<ValueId>,
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

#[derive(Clone, Copy, Debug)]
struct AdapterTypes {
    adapter: Adapter,
    inner: Option<TypeId>,
    slice_iter: TypeId,
    element: TypeId,
    cursor_ty: TypeId,
    cursor_bytes: u8,
    usize_ty: TypeId,
    bool_ty: TypeId,
}

#[derive(Clone, Debug)]
struct BodyMatch {
    driver: usize,
    update: Option<(usize, usize)>,
    body: usize,
    prefix: usize,
    driver_prefix: usize,
    removed_ops: BTreeMap<usize, BTreeSet<usize>>,
    pass_item_ref: bool,
    old_index: Option<ValueId>,
    old_item_ref: Option<ValueId>,
    old_byte: Option<ValueId>,
    folded_item_ref: Option<ValueId>,
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
    let single_edge = single_edge_param_sources(func);
    let uses = use_counts(func);
    let doms = Dominators::of(func);
    let outers: BTreeSet<ValueId> = func
        .values
        .iter()
        .enumerate()
        .filter(|(index, def)| {
            defs[*index].is_some()
                && table.get(def.ty).is_some_and(|descriptor| {
                    matches!(
                        descriptor.meta.iterator,
                        IteratorIdentity::Enumerate | IteratorIdentity::Slice
                    )
                })
        })
        .map(|(index, _)| ValueId(index as u32))
        .collect();

    let mut matches = Vec::new();
    let mut occupied = BTreeSet::new();
    for outer in outers {
        let Some(found) = match_loop(func, table, &defs, &single_edge, &uses, &doms, outer) else {
            continue;
        };
        if !reserve_region(&found.rewrite_blocks, found.preheader, &mut occupied) {
            continue;
        }
        matches.push(found);
    }

    for header in 1..func.blocks.len() {
        if occupied.contains(&header) {
            continue;
        }
        let Some(found) = match_scalarized_loop(func, table, &defs, &single_edge, &doms, header)
        else {
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
    single_edge: &[Option<ValueId>],
    uses: &[usize],
    doms: &Dominators,
    outer: ValueId,
) -> Option<LoopMatch> {
    let outer_ty = func.values.get(outer.0 as usize)?.ty;
    let types = adapter_types(table, outer_ty)?;
    let (outer_root, outer_fields) = aggregate(func, defs, uses, outer, outer_ty, false)?;

    let (slice_value, limit, initial) = match types.adapter {
        Adapter::Plain => (outer, None, None),
        Adapter::Enumerate => {
            let [slice_value, initial] = outer_fields.as_slice() else {
                return None;
            };
            if !is_scalar(func, defs, single_edge, *initial, 0)
                || func.values.get(initial.0 as usize)?.ty != types.usize_ty
            {
                return None;
            }
            (*slice_value, None, None)
        }
        Adapter::Take | Adapter::Skip => {
            let [inner_value, initial] = outer_fields.as_slice() else {
                return None;
            };
            if !is_scalar(func, defs, single_edge, *initial, 0)
                || func.values.get(initial.0 as usize)?.ty != types.usize_ty
            {
                return None;
            }
            let inner_ty = types.inner?;
            let (_, inner_fields) = aggregate(func, defs, uses, *inner_value, inner_ty, true)?;
            let [slice_value, limit] = inner_fields.as_slice() else {
                return None;
            };
            if func.values.get(limit.0 as usize)?.ty != types.usize_ty {
                return None;
            }
            (*slice_value, Some(*limit), Some(*initial))
        }
    };

    let slice_fields = if types.adapter == Adapter::Plain {
        outer_fields
    } else {
        aggregate(func, defs, uses, slice_value, types.slice_iter, true)?.1
    };
    let (start, end) = match slice_fields.as_slice() {
        [start, end] => (*start, *end),
        [start, end, zst] if value_ty(func, *zst).is_some_and(|ty| table.is_zst(ty)) => {
            (*start, *end)
        }
        _ => return None,
    };
    let raw = strip_pointer_aliases(func, table, defs, single_edge, start)?;
    let OpKind::RawView {
        root: Root::Value(source),
        path,
        mutable: false,
    } = &definition(func, defs, raw)?.kind
    else {
        return None;
    };
    if path.as_slice() != [Proj::Deref]
        || !is_slice_pointer_for(table, value_ty(func, *source)?, types.element)
    {
        return None;
    }
    let end = strip_pointer_aliases(func, table, defs, single_edge, end)?;
    let OpKind::Binary {
        op: BinaryOp::Offset,
        l: end_base,
        r: len,
        ty: end_ty,
    } = definition(func, defs, end)?.kind
    else {
        return None;
    };
    if !is_raw_ptr_to(table, end_ty, types.element) {
        return None;
    }
    if strip_pointer_aliases(func, table, defs, single_edge, end_base)? != raw {
        return None;
    }
    if !source_length_matches(func, table, defs, single_edge, *source, len)
        || value_ty(func, len)? != types.usize_ty
    {
        return None;
    }

    let header = find_header(func, table, defs, single_edge, outer, end, types)?;
    let mut bodies = Vec::new();
    for block in 0..func.blocks.len() {
        let body = match header.access {
            AdapterAccess::Direct => match types.adapter {
                Adapter::Take => {
                    match_take_body(func, table, defs, single_edge, outer, types, block)
                }
                Adapter::Skip => match_skip_body(func, table, outer, types, block),
                Adapter::Plain | Adapter::Enumerate => {
                    match_slice_direct_body(func, table, defs, single_edge, outer, types, block)
                }
            },
            AdapterAccess::Borrowed(pointer) => match types.adapter {
                Adapter::Take | Adapter::Skip => {
                    match_borrowed_body(func, table, (defs, uses), pointer, outer_ty, types, block)
                }
                Adapter::Plain | Adapter::Enumerate => None,
            },
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
    let header_id = RBlockId(header.block as u32);
    if !doms.dominates(RBlockId(preheader as u32), header_id)
        || !scc
            .iter()
            .all(|block| doms.dominates(header_id, RBlockId(*block as u32)))
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
    let extra_removed: Vec<ValueId> =
        if matches!(types.adapter, Adapter::Plain | Adapter::Enumerate) {
            func.blocks[header.block]
                .ops
                .iter()
                .filter_map(|op| op.result)
                .collect()
        } else {
            Vec::new()
        };
    if !removed_prefix_is_local(func, body, &scc, header.block, &extra_removed) {
        return None;
    }
    let tainted = tainted_values(
        func,
        &[outer],
        body.old_index,
        body.old_item_ref,
        body.old_byte,
    );
    if let Some(item_ref_val) = body.folded_item_ref {
        let item_ref_site = defs.get(item_ref_val.0 as usize).copied().flatten()?;
        let OpKind::BorrowView {
            root: Root::Value(ptr_val),
            ..
        } = &func.blocks[item_ref_site.block].ops[item_ref_site.op].kind
        else {
            return None;
        };
        if !tainted.contains(&item_ref_val)
            || !tainted.contains(ptr_val)
            || !scc.contains(&item_ref_site.block)
            || item_ref_site.block == header.block
            || item_ref_site.block == body.driver
            || item_ref_site.block == body.body
            || reaches_avoiding(
                func,
                RBlockId(body.body as u32),
                RBlockId(item_ref_site.block as u32),
                header.block,
            )
        {
            return None;
        }
    }
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
        removed_ops: body.removed_ops.clone(),
        removed_header_params: Vec::new(),
        removed_invariant_params: BTreeMap::new(),
        rewrite_blocks: scc,
        pass_item_ref: body.pass_item_ref,
        exit,
        source: *source,
        len,
        limit: limit.unwrap_or(len),
        initial: match types.adapter {
            Adapter::Take => initial,
            Adapter::Skip => limit,
            Adapter::Plain | Adapter::Enumerate => None,
        },
        old_index: body.old_index,
        old_item_ref: body.old_item_ref,
        old_byte: body.old_byte,
        item_ref_ty: body.item_ref_ty,
        byte_ty: body.byte_ty,
        cursor_ty: types.cursor_ty,
        cursor_bytes: types.cursor_bytes,
        usize_ty: types.usize_ty,
        bool_ty: types.bool_ty,
        origin: body.origin,
    })
}

fn adapter_types(table: &TypeTable, outer: TypeId) -> Option<AdapterTypes> {
    let descriptor = table.get(outer)?;
    if descriptor.meta.boundary != PinnedBoundary::None || descriptor.layout.needs_drop {
        return None;
    }
    match descriptor.meta.iterator {
        IteratorIdentity::Slice => {
            let element = slice_iter_element(table, outer)?;
            if !is_sized_non_zst(table, element) {
                return None;
            }
            let (usize_ty, usize_bits) = unsigned_usize_type(table)?;
            let usize_bytes = u8::try_from(usize_bits / 8).ok()?;
            let (cursor_ty, cursor_bytes) = cursor_type_for_slice(table, usize_ty, usize_bytes);
            Some(AdapterTypes {
                adapter: Adapter::Plain,
                inner: None,
                slice_iter: outer,
                element,
                cursor_ty,
                cursor_bytes,
                usize_ty,
                bool_ty: bool_type(table)?,
            })
        }
        IteratorIdentity::Enumerate => {
            let TypeKind::Adt { args, .. } = &descriptor.kind else {
                return None;
            };
            let [inner] = args.as_slice() else {
                return None;
            };
            let inner_descriptor = table.get(*inner)?;
            if inner_descriptor.meta.boundary != PinnedBoundary::None
                || inner_descriptor.layout.needs_drop
            {
                return None;
            }
            let usize_ty = table.field(outer, None, 1)?.ty;
            let (false, usize_bits) = table.int_info(usize_ty)? else {
                return None;
            };
            let usize_bytes = u8::try_from(usize_bits / 8).ok()?;
            if table.field(outer, None, 0)?.ty != *inner {
                return None;
            }
            match inner_descriptor.meta.iterator {
                IteratorIdentity::Slice => {
                    let element = slice_iter_element(table, *inner)?;
                    if !is_sized_non_zst(table, element) {
                        return None;
                    }
                    let (cursor_ty, cursor_bytes) =
                        cursor_type_for_slice(table, usize_ty, usize_bytes);
                    Some(AdapterTypes {
                        adapter: Adapter::Enumerate,
                        inner: None,
                        slice_iter: *inner,
                        element,
                        cursor_ty,
                        cursor_bytes,
                        usize_ty,
                        bool_ty: bool_type(table)?,
                    })
                }
                IteratorIdentity::Take | IteratorIdentity::Skip => {
                    let adapter = if inner_descriptor.meta.iterator == IteratorIdentity::Take {
                        Adapter::Take
                    } else {
                        Adapter::Skip
                    };
                    let TypeKind::Adt { args, .. } = &inner_descriptor.kind else {
                        return None;
                    };
                    let [slice_iter] = args.as_slice() else {
                        return None;
                    };
                    let element = slice_iter_element(table, *slice_iter)?;
                    if table.int_info(element) != Some((false, 8)) {
                        return None;
                    }
                    if table.field(*inner, None, 0)?.ty != *slice_iter
                        || table.field(*inner, None, 1)?.ty != usize_ty
                    {
                        return None;
                    }
                    Some(AdapterTypes {
                        adapter,
                        inner: Some(*inner),
                        slice_iter: *slice_iter,
                        element,
                        cursor_ty: usize_ty,
                        cursor_bytes: usize_bytes,
                        usize_ty,
                        bool_ty: bool_type(table)?,
                    })
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn slice_iter_element(table: &TypeTable, slice_iter: TypeId) -> Option<TypeId> {
    let descriptor = exact_iterator(table, slice_iter, IteratorIdentity::Slice)?;
    let TypeKind::Adt { args, .. } = &descriptor.kind else {
        return None;
    };
    let [element] = args.as_slice() else {
        return None;
    };
    Some(*element)
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

fn unsigned_usize_type(table: &TypeTable) -> Option<(TypeId, u32)> {
    table
        .descriptors()
        .iter()
        .find_map(|d| match table.int_info(d.id) {
            Some((false, bits @ 32)) if table.int_repr(d.id) == IntRepr::Number => {
                Some((d.id, bits))
            }
            _ => None,
        })
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

fn is_slice_pointer_for(table: &TypeTable, ty: TypeId, element: TypeId) -> bool {
    if !is_sized_non_zst(table, element) {
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

fn find_header(
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    single_edge: &[Option<ValueId>],
    outer: ValueId,
    end: ValueId,
    types: AdapterTypes,
) -> Option<HeaderMatch> {
    let mut found = Vec::new();
    for block in &func.blocks {
        let ops = &block.ops;
        let matched = (|| {
            if matches!(types.adapter, Adapter::Plain | Adapter::Enumerate) {
                return find_ptr_eq_header(
                    func,
                    table,
                    defs,
                    single_edge,
                    block,
                    outer,
                    end,
                    types,
                );
            }
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
                    Adapter::Plain | Adapter::Enumerate => unreachable!(),
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

#[allow(clippy::too_many_arguments)]
fn find_ptr_eq_header(
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    single_edge: &[Option<ValueId>],
    block: &RBlock,
    outer: ValueId,
    end: ValueId,
    types: AdapterTypes,
) -> Option<HeaderMatch> {
    let RTerm::Switch {
        scrut,
        scrut_ty,
        cases,
        default,
    } = &block.term
    else {
        return None;
    };
    if *scrut_ty != types.bool_ty || !default.1.is_empty() {
        return None;
    }
    let header_results: BTreeSet<ValueId> = block.ops.iter().filter_map(|op| op.result).collect();
    for op in &block.ops {
        let allowed = matches!(
            op.kind,
            OpKind::Copy(_)
                | OpKind::Move(_)
                | OpKind::Cast { .. }
                | OpKind::Field { .. }
                | OpKind::Aggregate { .. }
                | OpKind::Binary {
                    op: BinaryOp::Eq | BinaryOp::Ne,
                    ..
                }
        );
        if !allowed || op.result.is_none() {
            return None;
        }
    }
    if block
        .term
        .uses()
        .iter()
        .any(|u| header_results.contains(u) && *u != *scrut)
    {
        return None;
    }

    let (case_val, case_target) = match cases.as_slice() {
        [(val @ (0 | 1), target, args)] if args.is_empty() => (*val, *target),
        _ => return None,
    };
    let cmp_val = strip_copies_local(func, block.id.0 as usize, *scrut);
    let cmp_op = block.ops.iter().find(|op| op.result == Some(cmp_val))?;
    let OpKind::Binary {
        op: cmp_kind @ (BinaryOp::Eq | BinaryOp::Ne),
        l,
        r,
        ty: cmp_ty,
    } = cmp_op.kind
    else {
        return None;
    };
    if !is_raw_ptr_to(table, cmp_ty, types.element) {
        return None;
    }
    let eq_false_target = match (cmp_kind, case_val) {
        (BinaryOp::Eq, 0) | (BinaryOp::Ne, 1) => case_target,
        (BinaryOp::Eq, 1) | (BinaryOp::Ne, 0) => default.0,
        _ => return None,
    };

    let reads_outer_field = |val: ValueId, want_field: u32| -> bool {
        let cur = strip_ptr_wrapper(func, table, defs, single_edge, val);
        let Some(op) = cur.and_then(|v| definition(func, defs, v)) else {
            return false;
        };
        let OpKind::Field {
            base,
            variant: None,
            field: found_field,
        } = op.kind
        else {
            return false;
        };
        if found_field != want_field {
            return false;
        }
        let slice_src = match types.adapter {
            Adapter::Plain => base,
            Adapter::Enumerate => match definition(func, defs, base).map(|op| &op.kind) {
                Some(OpKind::Field {
                    base: outer_base,
                    variant: None,
                    field: 0,
                }) => *outer_base,
                _ => return false,
            },
            _ => return false,
        };
        slice_src == outer
    };

    let matches_ptr_and_end = |ptr_side: ValueId, end_side: ValueId| -> Option<AdapterAccess> {
        if !reads_outer_field(ptr_side, 0) {
            return None;
        }
        let end_ok = reads_outer_field(end_side, 1)
            || strip_ptr_wrapper(func, table, defs, single_edge, end_side) == Some(end);
        end_ok.then_some(AdapterAccess::Direct)
    };

    let access = matches_ptr_and_end(l, r).or_else(|| matches_ptr_and_end(r, l))?;
    Some(HeaderMatch {
        block: block.id.0 as usize,
        access,
        continue_to: eq_false_target,
    })
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
    let inner_ty = types.inner?;
    let pointer = borrow_empty_result(ops.first()?, outer, true)?;
    let inner_pointer = borrow_deref_field_result(ops.get(1)?, pointer, 0, inner_ty, true)?;
    let inner = deref_result(ops.get(2)?, inner_pointer)?;
    let n = field_result(ops.get(3)?, inner, 1)?;
    if value_ty(func, inner)? != inner_ty || value_ty(func, n)? != types.usize_ty {
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
    single_edge: &[Option<ValueId>],
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
    if strip_pointer_aliases(func, table, defs, single_edge, item_current)?
        != strip_pointer_aliases(func, table, defs, single_edge, current)?
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
        removed_ops: BTreeMap::new(),
        pass_item_ref: false,
        old_index: Some(update),
        old_item_ref: None,
        old_byte: Some(old_byte),
        folded_item_ref: None,
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
        removed_ops: BTreeMap::new(),
        pass_item_ref: true,
        old_index: Some(old_index),
        old_item_ref: None,
        old_byte: Some(old_byte),
        folded_item_ref: None,
        item_ref_ty,
        byte_ty,
        origin: body.ops[11].origin,
    })
}

fn match_borrowed_body(
    func: &Func,
    table: &TypeTable,
    (defs, uses): (&[Option<Site>], &[usize]),
    pointer: ValueId,
    outer_ty: TypeId,
    types: AdapterTypes,
    block: usize,
) -> Option<BodyMatch> {
    let driver = func.blocks.get(block)?;
    let prefix = driver.ops.get(..12)?;
    let (old_index, next) = borrowed_count_prefix(func, prefix, pointer, outer_ty, types.usize_ty)?;
    let empty_goto = |term: &RTerm| match term {
        RTerm::Goto(target, args) if args.is_empty() => Some(*target),
        _ => None,
    };
    let (update_match, body_index, prefix_len, driver_prefix, deref) = if driver.ops.len() >= 16 {
        if !borrowed_count_store(&driver.ops[12..15], pointer, next, types.usize_ty) {
            return None;
        }
        (None, block, 16, 16, &driver.ops[15])
    } else {
        let (update_match, body_id) = if driver.ops.len() == 15 {
            if !borrowed_count_store(&driver.ops[12..], pointer, next, types.usize_ty) {
                return None;
            }
            (None, empty_goto(&driver.term)?)
        } else if driver.ops.len() == 12 {
            let update_index = empty_goto(&driver.term)?.0 as usize;
            let update = func.blocks.get(update_index)?;
            if !update.params.is_empty()
                || !borrowed_count_store(&update.ops, pointer, next, types.usize_ty)
            {
                return None;
            }
            (
                Some((update_index, update.ops.len())),
                empty_goto(&update.term)?,
            )
        } else {
            return None;
        };
        let body_index = body_id.0 as usize;
        let body = func.blocks.get(body_index)?;
        if body_index == block || !body.params.is_empty() {
            return None;
        }
        (
            update_match,
            body_index,
            1,
            driver.ops.len(),
            body.ops.first()?,
        )
    };

    let (old_byte, item_ref_ty, pass_item_ref, folded_item_ref) = match driver.params.as_slice() {
        [item_ref] => (
            deref_result(deref, *item_ref)?,
            value_ty(func, *item_ref)?,
            body_index == block,
            None,
        ),
        [] => {
            let OpKind::Deref(item_ref_val) = deref.kind else {
                return None;
            };
            if uses.get(item_ref_val.0 as usize).copied() != Some(1)
                || !matches!(
                    &definition(func, defs, item_ref_val)?.kind,
                    OpKind::BorrowView {
                        root: Root::Value(_),
                        path,
                        mutable: false,
                    } if path.as_slice() == [Proj::Deref]
                )
            {
                return None;
            }
            (
                deref.result?,
                value_ty(func, item_ref_val)?,
                false,
                Some(item_ref_val),
            )
        }
        _ => return None,
    };
    let byte_ty = value_ty(func, old_byte)?;
    if byte_ty != types.element || !is_ref_to(table, item_ref_ty, byte_ty) {
        return None;
    }
    Some(BodyMatch {
        driver: block,
        update: update_match,
        body: body_index,
        prefix: prefix_len,
        driver_prefix,
        removed_ops: BTreeMap::new(),
        pass_item_ref,
        old_index: Some(old_index),
        old_item_ref: None,
        old_byte: Some(old_byte),
        folded_item_ref,
        item_ref_ty,
        byte_ty,
        origin: deref.origin,
    })
}

fn reads_outer_ptr_field(
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    single_edge: &[Option<ValueId>],
    value: ValueId,
    outer: ValueId,
    types: AdapterTypes,
) -> bool {
    let Some(cur) = strip_ptr_wrapper(func, table, defs, single_edge, value) else {
        return false;
    };
    let Some(op) = definition(func, defs, cur) else {
        return false;
    };
    let OpKind::Field {
        base,
        variant: None,
        field: 0,
    } = op.kind
    else {
        return false;
    };
    match types.adapter {
        Adapter::Plain => base == outer,
        Adapter::Enumerate => definition(func, defs, base).is_some_and(|inner| {
            matches!(
                inner.kind,
                OpKind::Field {
                    base: outer_base,
                    variant: None,
                    field: 0,
                } if outer_base == outer
            )
        }),
        _ => false,
    }
}

fn borrow_outer_ptr_field(op: &Op, outer: ValueId, types: AdapterTypes) -> Option<ValueId> {
    let OpKind::BorrowView {
        root: Root::Value(found),
        path,
        mutable: true,
    } = &op.kind
    else {
        return None;
    };
    if *found != outer {
        return None;
    }
    let valid = match types.adapter {
        Adapter::Plain => {
            !path.is_empty()
                && path
                    .iter()
                    .all(|p| matches!(p, Proj::Field { field: 0, .. }))
        }
        Adapter::Enumerate => {
            matches!(
                path.first(),
                Some(Proj::Field { field: 0, ty }) if *ty == types.slice_iter
            ) && path.len() >= 2
                && path[1..]
                    .iter()
                    .all(|p| matches!(p, Proj::Field { field: 0, .. }))
        }
        _ => false,
    };
    valid.then_some(op.result).flatten()
}

fn match_slice_direct_body(
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    single_edge: &[Option<ValueId>],
    outer: ValueId,
    types: AdapterTypes,
    block: usize,
) -> Option<BodyMatch> {
    let body = func.blocks.get(block)?;
    let ops = &body.ops;
    if !body.params.is_empty() {
        return None;
    }
    let mut min_prefix = match types.adapter {
        Adapter::Plain => 7,
        Adapter::Enumerate => 18,
        _ => return None,
    };
    if ops.len() < min_prefix {
        return None;
    }
    let one = const_result(&ops[0], 1)?;
    let offset = binary_result(&ops[1], BinaryOp::Offset)?;
    let (current, step) = match ops[1].kind {
        OpKind::Binary { l, r, .. } if r == one => (l, offset),
        _ => return None,
    };
    if !reads_outer_ptr_field(func, table, defs, single_edge, current, outer, types) {
        return None;
    }
    let advanced = cast_result(&ops[2], step)?;
    let state_ptr = borrow_outer_ptr_field(&ops[3], outer, types)?;
    if !matches!(ops[4].kind, OpKind::StoreThrough { ptr, v } if ptr == state_ptr && v == advanced)
        || ops[4].result.is_some()
    {
        return None;
    }
    let (item_current, item_ptr) = cast_source_result(&ops[5])?;
    if strip_pointer_aliases(func, table, defs, single_edge, item_current)?
        != strip_pointer_aliases(func, table, defs, single_edge, current)?
    {
        return None;
    }
    let old_item_ref = borrow_deref_result(&ops[6], item_ptr, false)?;
    let item_ref_ty = value_ty(func, old_item_ref)?;
    if !is_ref_to(table, item_ref_ty, types.element) {
        return None;
    }
    let old_index = if types.adapter == Adapter::Enumerate {
        Some(count_update(func, &ops[7..18], outer, types.usize_ty)?)
    } else {
        None
    };
    let (old_byte, origin) = match ops.get(min_prefix).and_then(|op| {
        let b = deref_result(op, old_item_ref)?;
        (value_ty(func, b) == Some(types.element)).then_some((b, op.origin))
    }) {
        Some((b, orig)) => {
            min_prefix += 1;
            (Some(b), orig)
        }
        None => (None, ops[6].origin),
    };
    Some(BodyMatch {
        driver: block,
        update: None,
        body: block,
        prefix: min_prefix,
        driver_prefix: min_prefix,
        removed_ops: BTreeMap::new(),
        pass_item_ref: false,
        old_index,
        old_item_ref: Some(old_item_ref),
        old_byte,
        folded_item_ref: None,
        item_ref_ty,
        byte_ty: types.element,
        origin,
    })
}

fn natural_loop_blocks(func: &Func, header_idx: usize, backedges: &[usize]) -> BTreeSet<usize> {
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); func.blocks.len()];
    for (bi, block) in func.blocks.iter().enumerate() {
        for succ in block.term.successors() {
            let si = succ.0 as usize;
            if si < preds.len() {
                preds[si].push(bi);
            }
        }
    }
    let mut loop_blocks = BTreeSet::new();
    loop_blocks.insert(header_idx);
    let mut worklist = Vec::new();
    for &be in backedges {
        if loop_blocks.insert(be) {
            worklist.push(be);
        }
    }
    while let Some(b) = worklist.pop() {
        for &p in &preds[b] {
            if loop_blocks.insert(p) {
                worklist.push(p);
            }
        }
    }
    loop_blocks
}

fn match_scalarized_loop(
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    single_edge: &[Option<ValueId>],
    doms: &Dominators,
    header_idx: usize,
) -> Option<LoopMatch> {
    let header = func.blocks.get(header_idx)?;
    if header.params.is_empty() {
        return None;
    }
    let bool_ty = bool_type(table)?;
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
    let cmp_val = strip_copies_local(func, header_idx, *scrut);
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

    let mut predecessors = Vec::new();
    for block in &func.blocks {
        if matches!(&block.term, RTerm::Goto(target, args) if *target == header.id && args.len() == header.params.len())
        {
            predecessors.push(block.id.0 as usize);
        } else if block.term.successors().contains(&header.id) {
            return None;
        }
    }
    let outside_preds: Vec<usize> = predecessors
        .iter()
        .copied()
        .filter(|b| !doms.dominates(header.id, RBlockId(*b as u32)))
        .collect();
    let [preheader] = outside_preds.as_slice() else {
        return None;
    };
    let preheader = *preheader;
    let backedges: Vec<usize> = predecessors
        .into_iter()
        .filter(|b| doms.dominates(header.id, RBlockId(*b as u32)))
        .collect();
    if backedges.is_empty() || !doms.dominates(RBlockId(preheader as u32), header.id) {
        return None;
    }
    let scc = natural_loop_blocks(func, header_idx, &backedges);
    if !scc.contains(&(continue_to.0 as usize))
        || scc.contains(&(exit_target.0 as usize))
        || !scc
            .iter()
            .all(|block| doms.dominates(header.id, RBlockId(*block as u32)))
    {
        return None;
    }

    // Header ops must only compute the comparison driving the switch and must not escape header.
    let header_results: BTreeSet<ValueId> = header.ops.iter().filter_map(|op| op.result).collect();
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

    // Match (ptr_param, end) from either orientation of cmp_op.
    let preheader_args = match &func.blocks[preheader].term {
        RTerm::Goto(_, args) => args.as_slice(),
        _ => return None,
    };
    let match_orientation =
        |ptr_side: ValueId,
         end_side: ValueId|
         -> Option<(usize, ValueId, ValueId, Option<usize>, TypeId, TypeId)> {
            let ptr_root = strip_ptr_wrapper_stop_params(
                func,
                table,
                defs,
                single_edge,
                &header.params,
                ptr_side,
            )?;
            let p_idx = header.params.iter().position(|p| *p == ptr_root)?;
            let raw = strip_ptr_wrapper(func, table, defs, single_edge, preheader_args[p_idx])?;
            let source = match definition(func, defs, raw).map(|op| &op.kind) {
                Some(OpKind::RawView {
                    root: Root::Value(source),
                    path,
                    mutable: false,
                }) if path.as_slice() == [Proj::Deref] => *source,
                Some(OpKind::SliceView { .. }) => raw,
                _ if is_immutable_slice_ref(func, table, raw) => raw,
                _ => return None,
            };
            let source_block = value_def_block(func, defs, source)?;
            if scc.contains(&source_block)
                || !doms.dominates(RBlockId(source_block as u32), RBlockId(preheader as u32))
            {
                return None;
            }
            let elem_ty = sized_non_zst_slice_element(table, value_ty(func, source)?)?;
            if !is_raw_ptr_to(table, cmp_ty, elem_ty) {
                return None;
            }
            let end_leaf =
                strip_ptr_wrapper_until(func, table, defs, single_edge, &header.params, end_side)?;
            let (end_root, end_param_idx) =
                if let Some(e_idx) = header.params.iter().position(|p| *p == end_leaf) {
                    if e_idx == p_idx {
                        return None;
                    }
                    for be in &backedges {
                        let RTerm::Goto(_, args) = &func.blocks[*be].term else {
                            return None;
                        };
                        if args.get(e_idx).copied() != Some(end_leaf) {
                            return None;
                        }
                    }
                    (
                        strip_ptr_wrapper(func, table, defs, single_edge, preheader_args[e_idx])?,
                        Some(e_idx),
                    )
                } else {
                    (end_leaf, None)
                };
            let end_site = defs.get(end_root.0 as usize).copied().flatten()?;
            if scc.contains(&end_site.block)
                || !doms.dominates(RBlockId(end_site.block as u32), RBlockId(preheader as u32))
            {
                return None;
            }
            let OpKind::Binary {
                op: BinaryOp::Offset,
                l: end_base,
                r: len,
                ty: end_ty,
            } = definition(func, defs, end_root)?.kind
            else {
                return None;
            };
            if !is_raw_ptr_to(table, end_ty, elem_ty) {
                return None;
            }
            let end_base_root = strip_ptr_wrapper(func, table, defs, single_edge, end_base)?;
            if end_base_root != raw && end_base_root != source {
                return None;
            }
            if !source_length_matches(func, table, defs, single_edge, source, len) {
                return None;
            }
            let usize_ty = value_ty(func, len)?;
            if table
                .int_info(usize_ty)
                .is_none_or(|(signed, bits)| signed || bits != 32)
            {
                return None;
            }
            Some((p_idx, source, len, end_param_idx, elem_ty, usize_ty))
        };

    let (p_idx, source, len, end_param_idx, elem_ty, usize_ty) =
        match_orientation(l, r).or_else(|| match_orientation(r, l))?;
    let ptr_param = header.params[p_idx];
    let driver_idx = continue_to.0 as usize;
    let driver_block = func.blocks.get(driver_idx)?;
    if !driver_block.params.is_empty() {
        return None;
    }

    // Find next_ptr = Offset(ptr_param, 1) in driver_idx.
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
            if is_raw_ptr_to(table, next_ty, elem_ty)
                && strip_ptr_wrapper(func, table, defs, single_edge, base) == Some(ptr_param)
                && is_scalar(func, defs, single_edge, one, 1)
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
    collect_ptr_wrapper_chain_in_block(
        func,
        table,
        defs,
        driver_idx,
        next_ptr_base,
        ptr_param,
        &mut removed_driver_ops,
        &mut zst_candidates,
    );
    if let Some(site) = defs.get(one_val.0 as usize).copied().flatten() {
        if site.block == driver_idx {
            // Only remove `one_val` if unused outside removed_ops.
            let used_elsewhere = func.blocks.iter().enumerate().any(|(bi, b)| {
                b.ops.iter().enumerate().any(|(oi, op)| {
                    !(bi == driver_idx && (oi == next_ptr_idx || removed_driver_ops.contains(&oi)))
                        && op.kind.uses().contains(&one_val)
                }) || b.term.uses().contains(&one_val)
            });
            if !used_elsewhere {
                removed_driver_ops.insert(site.op);
            }
        }
    }

    // Check every backedge passes next_ptr (or wrapper of it) at p_idx and is reachable from driver.
    let driver_reach = distances(func, continue_to);
    let mut invariant_param_candidates: Vec<(usize, usize, ValueId)> = Vec::new();
    for be in &backedges {
        driver_reach[*be]?;
        let RTerm::Goto(_, args) = &func.blocks[*be].term else {
            return None;
        };
        let arg = args[p_idx];
        if strip_ptr_wrapper(func, table, defs, single_edge, arg) != Some(next_ptr) {
            return None;
        }
        collect_backedge_wrapper_chain(
            func,
            table,
            defs,
            single_edge,
            driver_idx,
            arg,
            next_ptr,
            &mut removed_driver_ops,
            &mut zst_candidates,
            &mut invariant_param_candidates,
        )?;
    }
    let removed_invariant_params = validate_dead_invariant_params(
        func,
        &scc,
        header_idx,
        p_idx,
        &backedges,
        &invariant_param_candidates,
    )?;

    // Find old_item_ref and/or old_item in driver_idx.
    let mut old_item_ref = None;
    let mut old_item_ref_op = None;
    let mut old_item = None;
    let mut old_item_op = None;
    let mut item_ref_ty = None;
    let mut origin = cmp_op.origin;

    for (op_idx, op) in driver_block.ops.iter().enumerate() {
        if let OpKind::BorrowView {
            root: Root::Value(ptr),
            path,
            mutable: false,
        } = &op.kind
        {
            if path.as_slice() == [Proj::Deref]
                && strip_ptr_wrapper(func, table, defs, single_edge, *ptr) == Some(ptr_param)
            {
                if let Some(res) = op.result {
                    if let Some(ty) = value_ty(func, res) {
                        if is_ref_to(table, ty, elem_ty) {
                            old_item_ref = Some(res);
                            old_item_ref_op = Some(op_idx);
                            item_ref_ty = Some(ty);
                            origin = op.origin;
                            collect_ptr_wrapper_chain_in_block(
                                func,
                                table,
                                defs,
                                driver_idx,
                                *ptr,
                                ptr_param,
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
                if strip_copies_local(func, driver_idx, target) == ref_val {
                    if let Some(res) = op.result {
                        if value_ty(func, res) == Some(elem_ty) {
                            old_item = Some(res);
                            old_item_op = Some(op_idx);
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
                if strip_ptr_wrapper(func, table, defs, single_edge, ptr) == Some(ptr_param) {
                    if let Some(res) = op.result {
                        if value_ty(func, res) == Some(elem_ty) {
                            old_item = Some(res);
                            old_item_op = Some(op_idx);
                            origin = op.origin;
                            removed_driver_ops.insert(op_idx);
                            collect_ptr_wrapper_chain_in_block(
                                func,
                                table,
                                defs,
                                driver_idx,
                                ptr,
                                ptr_param,
                                &mut removed_driver_ops,
                                &mut zst_candidates,
                            );
                            break;
                        }
                    }
                }
            }
        }
        item_ref_ty = ref_to_type(table, elem_ty);
    }
    let item_ref_ty = item_ref_ty?;
    if old_item_ref.is_none() && old_item.is_none() {
        return None;
    }
    let _ = old_item_op;
    collect_unused_zst_ops_in_block(
        func,
        table,
        defs,
        driver_idx,
        zst_candidates,
        &mut removed_driver_ops,
    );

    // Optional: check for a scalarized Enumerate index counter parameter in header.
    let mut removed_header_params = vec![p_idx];
    if let Some(e_idx) = end_param_idx {
        removed_header_params.push(e_idx);
    }
    let mut adapter = Adapter::Plain;
    let mut old_index = None;
    for (i_idx, &idx_param) in header.params.iter().enumerate() {
        if i_idx == p_idx
            || Some(i_idx) == end_param_idx
            || value_ty(func, idx_param) != Some(usize_ty)
        {
            continue;
        }
        if !is_scalar(func, defs, single_edge, preheader_args[i_idx], 0) {
            continue;
        }
        if let Some((counter_ops, next_idx)) = match_scalarized_enumerate_counter(
            func,
            defs,
            single_edge,
            driver_idx,
            idx_param,
            usize_ty,
            &backedges,
            i_idx,
            &scc,
        ) {
            let mut candidate_removed = removed_driver_ops.clone();
            candidate_removed.extend(counter_ops.iter().copied());
            let mut removed_map = BTreeMap::new();
            removed_map.insert(header_idx, (0..header.ops.len()).collect());
            removed_map.insert(driver_idx, candidate_removed);
            let mut candidate_params = removed_header_params.clone();
            candidate_params.push(i_idx);
            if removed_ops_are_local(
                func,
                header_idx,
                &removed_map,
                &removed_invariant_params,
                &scc,
                None,
                &backedges,
                &candidate_params,
                &[Some(idx_param), old_item_ref, old_item],
                &[next_idx],
            ) {
                removed_driver_ops.extend(counter_ops);
                removed_header_params.push(i_idx);
                adapter = Adapter::Enumerate;
                old_index = Some(idx_param);
                break;
            }
        }
    }

    let mut removed_ops = BTreeMap::new();
    removed_ops.insert(header_idx, (0..header.ops.len()).collect());
    removed_ops.insert(driver_idx, removed_driver_ops.clone());

    // Check if driver is a pure forwarding block to `body(item_ref)` or `body()`.
    let mut body_idx = driver_idx;
    let mut pass_item_ref = false;
    let driver_remaining = driver_block
        .ops
        .len()
        .saturating_sub(removed_driver_ops.len());
    let sole_header_pred = driver_idx != 0
        && func.blocks.iter().enumerate().all(|(bi, b)| {
            bi == header_idx
                || !b
                    .term
                    .successors()
                    .iter()
                    .any(|succ| succ.0 as usize == driver_idx)
        });
    if driver_remaining == 0 && sole_header_pred {
        if let RTerm::Goto(next_b, args) = &driver_block.term {
            let next_idx = next_b.0 as usize;
            if next_idx != header_idx && scc.contains(&next_idx) {
                let body_block = func.blocks.get(next_idx)?;
                if args.is_empty() && body_block.params.is_empty() {
                    body_idx = next_idx;
                } else if let (Some(ref_val), [arg], [param]) =
                    (old_item_ref, args.as_slice(), body_block.params.as_slice())
                {
                    if *arg == ref_val && old_item.is_none() {
                        body_idx = next_idx;
                        pass_item_ref = true;
                        // When passing via body param, `param` receives `new_item_ref`; `old_item_ref` is not exposed outside driver.
                        let _ = param;
                    }
                }
            }
        }
    }

    let bypassed_driver = (body_idx != driver_idx).then_some(driver_idx);
    if !removed_ops_are_local(
        func,
        header_idx,
        &removed_ops,
        &removed_invariant_params,
        &scc,
        bypassed_driver,
        &backedges,
        &removed_header_params,
        &[
            old_index,
            if pass_item_ref { None } else { old_item_ref },
            old_item,
        ],
        &[],
    ) {
        return None;
    }
    removed_ops.remove(&header_idx);

    let taint_seeds = vec![ptr_param];
    if let Some(idx_p) = old_index {
        let _ = idx_p; // idx_param is rewritten directly to enumerate_index; checked above.
    }
    let tainted = tainted_values(func, &taint_seeds, old_index, old_item_ref, old_item);
    if !non_escaping(func, defs, &scc, &tainted) {
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
    let exit = common_exit(func, &scc, &exits)?;
    if !func.blocks.get(exit.0 as usize)?.params.is_empty()
        || !exits
            .iter()
            .all(|start| discardable_exit_chain(func, defs, &tainted, *start, exit))
    {
        return None;
    }

    let (_, usize_bits) = table.int_info(usize_ty)?;
    let usize_bytes = u8::try_from(usize_bits / 8).ok()?;
    let (cursor_ty, cursor_bytes) = cursor_type_for_slice(table, usize_ty, usize_bytes);

    Some(LoopMatch {
        adapter,
        header: header_idx,
        preheader,
        backedges,
        driver: driver_idx,
        update: None,
        body: body_idx,
        body_prefix: 0,
        removed_ops,
        removed_header_params,
        removed_invariant_params,
        rewrite_blocks: scc,
        pass_item_ref,
        exit,
        source,
        len,
        limit: len,
        initial: None,
        old_index,
        old_item_ref: if pass_item_ref { None } else { old_item_ref },
        old_byte: old_item,
        item_ref_ty,
        byte_ty: elem_ty,
        cursor_ty,
        cursor_bytes,
        usize_ty,
        bool_ty,
        origin,
    })
}

#[allow(clippy::too_many_arguments)]
fn match_scalarized_enumerate_counter(
    func: &Func,
    defs: &[Option<Site>],
    single_edge: &[Option<ValueId>],
    driver_idx: usize,
    idx_param: ValueId,
    usize_ty: TypeId,
    backedges: &[usize],
    i_idx: usize,
    scc: &BTreeSet<usize>,
) -> Option<(Vec<usize>, ValueId)> {
    let ops = &func.blocks[driver_idx].ops;
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
                && strip_copies_local(func, driver_idx, l) == idx_param
                && is_scalar(func, defs, single_edge, r, 1)
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
    for be in backedges {
        let RTerm::Goto(_, args) = &func.blocks[*be].term else {
            return None;
        };
        if strip_copies(func, defs, single_edge, args[i_idx])? != next_idx {
            return None;
        }
    }
    // Check idx_param does not escape scc.
    for (bi, b) in func.blocks.iter().enumerate() {
        if !scc.contains(&bi)
            && (b.ops.iter().any(|op| op.kind.uses().contains(&idx_param))
                || b.term.uses().contains(&idx_param))
        {
            return None;
        }
    }

    let mut counter_ops = vec![next_idx_op];
    if let Some(site) = defs.get(before_val.0 as usize).copied().flatten() {
        if site.block == driver_idx && before_val != idx_param {
            counter_ops.push(site.op);
        }
    }
    if let Some(site) = defs.get(one_val.0 as usize).copied().flatten() {
        if site.block == driver_idx {
            let used_elsewhere = func.blocks.iter().enumerate().any(|(bi, b)| {
                b.ops.iter().enumerate().any(|(oi, op)| {
                    !(bi == driver_idx && oi == next_idx_op) && op.kind.uses().contains(&one_val)
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
        } = &op.kind
        {
            if is_scalar(func, defs, single_edge, *right, 1)
                && strip_copies_local(func, driver_idx, *left) == idx_param
            {
                let cmp = strip_copies_local(func, driver_idx, *cond);
                if let Some(cmp_site) = defs.get(cmp.0 as usize).copied().flatten() {
                    if cmp_site.block == driver_idx {
                        if let OpKind::Binary {
                            op: BinaryOp::Lt,
                            l,
                            r,
                            ty,
                        } = ops[cmp_site.op].kind
                        {
                            if ty == usize_ty
                                && l == next_idx
                                && strip_copies_local(func, driver_idx, r) == idx_param
                            {
                                counter_ops.push(op_idx);
                                counter_ops.push(cmp_site.op);
                                if let Some(cs) = defs.get(cond.0 as usize).copied().flatten() {
                                    if cs.block == driver_idx {
                                        counter_ops.push(cs.op);
                                    }
                                }
                                if let Some(ls) = defs.get(left.0 as usize).copied().flatten() {
                                    if ls.block == driver_idx && *left != idx_param {
                                        counter_ops.push(ls.op);
                                    }
                                }
                                if let Some(rs) = defs.get(right.0 as usize).copied().flatten() {
                                    if rs.block == driver_idx {
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

fn strip_copies_local(func: &Func, block: usize, mut val: ValueId) -> ValueId {
    for _ in 0..8 {
        let Some(op) = func.blocks[block]
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

fn strip_ptr_wrapper(
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    single_edge: &[Option<ValueId>],
    value: ValueId,
) -> Option<ValueId> {
    strip_ptr_wrapper_until(func, table, defs, single_edge, &[], value)
}

fn strip_ptr_wrapper_until(
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    single_edge: &[Option<ValueId>],
    stop_params: &[ValueId],
    mut value: ValueId,
) -> Option<ValueId> {
    for _ in 0..24 {
        if stop_params.contains(&value) {
            return Some(value);
        }
        let Some(op) = definition(func, defs, value) else {
            if let Some(source) = single_edge.get(value.0 as usize).copied().flatten() {
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
            } if is_lossless_ptr_cast(table, *kind, *from, *to) => {
                value = *source;
            }
            OpKind::Field {
                base,
                variant: None,
                field,
            } if value_ty(func, *base).and_then(|ty| ptr_wrapper_field(table, ty))
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
                let Some(idx) = ptr_wrapper_aggregate_field(table, *ty, fields.len()) else {
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
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    single_edge: &[Option<ValueId>],
    params: &[ValueId],
    mut value: ValueId,
) -> Option<ValueId> {
    for _ in 0..24 {
        if params.contains(&value) {
            return Some(value);
        }
        let Some(op) = definition(func, defs, value) else {
            value = single_edge.get(value.0 as usize).copied().flatten()?;
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
            } if is_lossless_ptr_cast(table, *kind, *from, *to) => {
                value = *source;
            }
            OpKind::Field {
                base,
                variant: None,
                field,
            } if value_ty(func, *base).and_then(|ty| ptr_wrapper_field(table, ty))
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
                let idx = ptr_wrapper_aggregate_field(table, *ty, fields.len())?;
                value = fields[idx];
            }
            _ => return None,
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn collect_ptr_wrapper_chain_in_block(
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    block: usize,
    mut value: ValueId,
    stop: ValueId,
    out: &mut BTreeSet<usize>,
    zst_candidates: &mut Vec<ValueId>,
) {
    for _ in 0..16 {
        if value == stop {
            break;
        }
        let Some(site) = defs.get(value.0 as usize).copied().flatten() else {
            break;
        };
        if site.block != block {
            break;
        }
        let op = &func.blocks[block].ops[site.op];
        let next = match &op.kind {
            OpKind::Copy(s) | OpKind::Move(s) => *s,
            OpKind::Cast {
                kind,
                v: s,
                from,
                to,
            } if is_lossless_ptr_cast(table, *kind, *from, *to) => *s,
            OpKind::Field {
                base,
                variant: None,
                field,
            } if value_ty(func, *base).and_then(|ty| ptr_wrapper_field(table, ty))
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
                let Some(idx) = ptr_wrapper_aggregate_field(table, *ty, fields.len()) else {
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

#[allow(clippy::too_many_arguments)]
fn collect_backedge_wrapper_chain(
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    single_edge: &[Option<ValueId>],
    driver_idx: usize,
    arg: ValueId,
    next_ptr: ValueId,
    out: &mut BTreeSet<usize>,
    zst_candidates: &mut Vec<ValueId>,
    invariant_param_candidates: &mut Vec<(usize, usize, ValueId)>,
) -> Option<()> {
    let mut cur = arg;
    for _ in 0..24 {
        if cur == next_ptr {
            return Some(());
        }
        let Some(site) = defs.get(cur.0 as usize).copied().flatten() else {
            let (param_block, param_pos) =
                func.blocks.iter().enumerate().find_map(|(bi, b)| {
                    b.params.iter().position(|p| *p == cur).map(|pos| (bi, pos))
                })?;
            invariant_param_candidates.push((param_block, param_pos, cur));
            cur = single_edge.get(cur.0 as usize).copied().flatten()?;
            continue;
        };
        if site.block == driver_idx {
            out.insert(site.op);
        }
        cur = match &func.blocks[site.block].ops[site.op].kind {
            OpKind::Copy(s) | OpKind::Move(s) => *s,
            OpKind::Cast {
                kind,
                v: s,
                from,
                to,
            } if is_lossless_ptr_cast(table, *kind, *from, *to) => *s,
            OpKind::Field {
                base,
                variant: None,
                field,
            } if value_ty(func, *base).and_then(|ty| ptr_wrapper_field(table, ty))
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
                let idx = ptr_wrapper_aggregate_field(table, *ty, fields.len())?;
                if site.block == driver_idx {
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
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    block_idx: usize,
    mut candidates: Vec<ValueId>,
    removed_ops: &mut BTreeSet<usize>,
) {
    while let Some(val) = candidates.pop() {
        if !value_ty(func, val).is_some_and(|ty| table.is_zst(ty)) {
            continue;
        }
        let Some(site) = defs.get(val.0 as usize).copied().flatten() else {
            continue;
        };
        if site.block != block_idx || removed_ops.contains(&site.op) {
            continue;
        }
        let op = &func.blocks[block_idx].ops[site.op];
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
        let used_elsewhere = func.blocks.iter().enumerate().any(|(bi, b)| {
            b.ops.iter().enumerate().any(|(oi, other)| {
                !(bi == block_idx && removed_ops.contains(&oi)) && other.kind.uses().contains(&val)
            }) || b.term.uses().contains(&val)
        });
        if !used_elsewhere {
            removed_ops.insert(site.op);
            candidates.extend(op.kind.uses());
        }
    }
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
    header_block: usize,
    extra_removed: &[ValueId],
) -> bool {
    let mut removed = BTreeSet::new();
    removed.extend(extra_removed.iter().copied());
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
    let mut allowed = BTreeSet::new();
    for v in [matched.old_index, matched.old_item_ref, matched.old_byte]
        .into_iter()
        .flatten()
    {
        allowed.insert(v);
    }
    for (block_index, block) in func.blocks.iter().enumerate() {
        if !extra_removed.is_empty() && block_index == header_block {
            continue;
        }
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

fn validate_dead_invariant_params(
    func: &Func,
    scc: &BTreeSet<usize>,
    header_idx: usize,
    p_idx: usize,
    backedges: &[usize],
    candidates: &[(usize, usize, ValueId)],
) -> Option<BTreeMap<usize, Vec<usize>>> {
    let mut map: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    if candidates.is_empty() {
        return Some(map);
    }
    let backedge_set: BTreeSet<usize> = backedges.iter().copied().collect();
    let mut candidate_vals = BTreeSet::new();
    let mut candidate_slots = BTreeSet::new();
    for &(b_idx, pos, val) in candidates {
        if b_idx == header_idx || !scc.contains(&b_idx) {
            return None;
        }
        candidate_vals.insert(val);
        candidate_slots.insert((b_idx, pos));
        map.entry(b_idx).or_default().push(pos);
    }
    for positions in map.values_mut() {
        positions.sort_unstable();
        positions.dedup();
    }
    for (b_idx, block) in func.blocks.iter().enumerate() {
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
            let t = target.0 as usize;
            if map.contains_key(&t) && args.len() != func.blocks[t].params.len() {
                return None;
            }
            for (pos, arg) in args.iter().enumerate() {
                if !candidate_vals.contains(arg) {
                    continue;
                }
                let allowed_edge =
                    (t == header_idx && pos == p_idx && backedge_set.contains(&b_idx))
                        || candidate_slots.contains(&(t, pos));
                if !allowed_edge {
                    return None;
                }
            }
        }
    }
    Some(map)
}

#[allow(clippy::too_many_arguments)]
fn removed_ops_are_local(
    func: &Func,
    header_idx: usize,
    removed_ops: &BTreeMap<usize, BTreeSet<usize>>,
    removed_invariant_params: &BTreeMap<usize, Vec<usize>>,
    rewrite_blocks: &BTreeSet<usize>,
    bypassed_driver: Option<usize>,
    backedges: &[usize],
    removed_params: &[usize],
    exposed: &[Option<ValueId>],
    extra_backedge_roots: &[ValueId],
) -> bool {
    let mut removed = BTreeSet::new();
    for &param_idx in removed_params {
        if let Some(&param) = func.blocks[header_idx].params.get(param_idx) {
            removed.insert(param);
        }
    }
    for (&block_idx, ops) in removed_ops {
        for &op_idx in ops {
            if let Some(res) = func.blocks[block_idx].ops[op_idx].result {
                removed.insert(res);
            }
        }
    }
    let mut allowed = BTreeSet::new();
    for v in exposed.iter().copied().flatten() {
        allowed.insert(v);
    }
    let backedge_set: BTreeSet<usize> = backedges.iter().copied().collect();
    let param_set: BTreeSet<usize> = removed_params.iter().copied().collect();
    for (block_index, block) in func.blocks.iter().enumerate() {
        if block_index == header_idx {
            continue;
        }
        let is_bypassed = bypassed_driver == Some(block_index);
        for (op_index, op) in block.ops.iter().enumerate() {
            if removed_ops
                .get(&block_index)
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
            if (!rewrite_blocks.contains(&block_index) || is_bypassed)
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
                || (!rewrite_blocks.contains(&block_index) && allowed.contains(&v))
            {
                return false;
            }
        }
        let is_backedge = backedge_set.contains(&block_index);
        for (target, args) in block.term.edges() {
            let t = target.0 as usize;
            let skip_invariant = removed_invariant_params.get(&t);
            for (pos, arg) in args.iter().enumerate() {
                if is_backedge && t == header_idx && param_set.contains(&pos) {
                    continue;
                }
                if skip_invariant.is_some_and(|positions| positions.contains(&pos)) {
                    continue;
                }
                if removed.contains(arg) && !allowed.contains(arg) {
                    return false;
                }
                if is_backedge && extra_backedge_roots.contains(arg) && !allowed.contains(arg) {
                    return false;
                }
                if !rewrite_blocks.contains(&block_index) && allowed.contains(arg) {
                    return false;
                }
            }
        }
    }
    true
}

fn tainted_values(
    func: &Func,
    seeds: &[ValueId],
    old_index: Option<ValueId>,
    old_item_ref: Option<ValueId>,
    old_byte: Option<ValueId>,
) -> BTreeSet<ValueId> {
    let mut tainted: BTreeSet<ValueId> = seeds.iter().copied().collect();
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
                OpKind::Aggregate { .. } => op.result.is_some_and(|r| tainted.contains(&r)),
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
    let cursor = fresh_with_type(func, found.cursor_ty, found.origin);
    let body_cursor = fresh_with_type(func, found.cursor_ty, found.origin);
    let bound_len = if matches!(found.adapter, Adapter::Plain | Adapter::Enumerate)
        && found.cursor_ty != found.usize_ty
    {
        let cast_len = fresh_with_type(func, found.cursor_ty, found.origin);
        func.blocks[found.preheader].ops.push(Op {
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
    let initial = match found.initial {
        Some(v) => v,
        None => {
            let zero = fresh_with_type(func, found.cursor_ty, found.origin);
            func.blocks[found.preheader].ops.push(Op {
                result: Some(zero),
                origin: found.origin,
                kind: OpKind::Const(ConstTree::Scalar {
                    bits: 0,
                    bytes: found.cursor_bytes,
                }),
            });
            zero
        }
    };
    let less_len = fresh_bool(func, found.bool_ty, found.origin);
    let less_limit =
        (found.adapter == Adapter::Take).then(|| fresh_bool(func, found.bool_ty, found.origin));
    let condition = less_limit.map_or(less_len, |_| fresh_bool(func, found.bool_ty, found.origin));
    let index = match found.adapter {
        Adapter::Take | Adapter::Plain => body_cursor,
        Adapter::Enumerate if found.cursor_ty == found.usize_ty || found.old_index.is_none() => {
            body_cursor
        }
        Adapter::Enumerate | Adapter::Skip => fresh_with_type(func, found.usize_ty, found.origin),
    };
    let item_ref = fresh_with_type(func, found.item_ref_ty, found.origin);
    let byte = found
        .old_byte
        .map(|_| fresh_with_type(func, found.byte_ty, found.origin));
    let one = fresh_with_type(func, found.cursor_ty, found.origin);
    let next = fresh_with_type(func, found.cursor_ty, found.origin);
    let direct = RBlockId(func.blocks.len() as u32);

    let mut header_ops = vec![Op {
        result: Some(less_len),
        origin: found.origin,
        kind: OpKind::Binary {
            op: BinaryOp::Lt,
            l: cursor,
            r: bound_len,
            ty: found.cursor_ty,
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
    let mut removed_params = found.removed_header_params.clone();
    removed_params.sort_unstable();
    removed_params.dedup();
    for &idx in removed_params.iter().rev() {
        func.blocks[found.header].params.remove(idx);
    }
    func.blocks[found.header].params.push(cursor);
    func.blocks[found.header].ops = header_ops;
    func.blocks[found.header].term = RTerm::Switch {
        scrut: condition,
        scrut_ty: found.bool_ty,
        cases: vec![(0, found.exit, Vec::new())],
        default: (direct, vec![cursor]),
    };

    let preheader_args = edge_to(&mut func.blocks[found.preheader].term, found.header)
        .expect("a matched preheader edge exists");
    for &idx in removed_params.iter().rev() {
        preheader_args.remove(idx);
    }
    preheader_args.push(initial);
    for backedge in &found.backedges {
        let args = edge_to(&mut func.blocks[*backedge].term, found.header)
            .expect("a matched backedge exists");
        for &idx in removed_params.iter().rev() {
            args.remove(idx);
        }
        args.push(next);
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
    } else if found.adapter == Adapter::Enumerate
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
        term: RTerm::Goto(RBlockId(found.body as u32), body_args.clone()),
    });

    if !found.removed_invariant_params.is_empty() {
        for (&block_idx, positions) in &found.removed_invariant_params {
            for &pos in positions.iter().rev() {
                func.blocks[block_idx].params.remove(pos);
            }
        }
        for block in &mut func.blocks {
            for (target, args) in term_edge_args_mut(&mut block.term) {
                if let Some(positions) = found.removed_invariant_params.get(&(target.0 as usize)) {
                    for &pos in positions.iter().rev() {
                        if pos < args.len() {
                            args.remove(pos);
                        }
                    }
                }
            }
        }
    }
    if !found.removed_ops.is_empty() {
        for (block_idx, drop_set) in &found.removed_ops {
            if *block_idx == found.header {
                continue;
            }
            let kept = std::mem::take(&mut func.blocks[*block_idx].ops)
                .into_iter()
                .enumerate()
                .filter(|(i, _)| !drop_set.contains(i))
                .map(|(_, op)| op)
                .collect();
            func.blocks[*block_idx].ops = kept;
        }
        if found.driver != found.body {
            func.blocks[found.driver].term = RTerm::Unreachable;
        }
    } else if found.body_prefix > 0 {
        let tail = func.blocks[found.body].ops.split_off(found.body_prefix);
        func.blocks[found.body].ops = tail;
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
            Adapter::Plain => IteratorIdentity::Slice,
            Adapter::Enumerate => IteratorIdentity::Enumerate,
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
        bound_length: bound_len,
        limit: found.limit,
        initial,
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
        cursor_ty: found.cursor_ty,
        usize_ty: found.usize_ty,
        bool_ty: found.bool_ty,
        cursor_bytes: found.cursor_bytes,
        origin: found.origin,
    }
}

fn term_edge_args_mut(term: &mut RTerm) -> Vec<(RBlockId, &mut Vec<ValueId>)> {
    match term {
        RTerm::Goto(target, args) => vec![(*target, args)],
        RTerm::Invoke { normal, unwind } => {
            vec![(normal.0, &mut normal.1), (unwind.0, &mut unwind.1)]
        }
        RTerm::Switch { cases, default, .. } => {
            let mut edges: Vec<(RBlockId, &mut Vec<ValueId>)> = cases
                .iter_mut()
                .map(|(_, target, args)| (*target, args))
                .collect();
            edges.push((default.0, &mut default.1));
            edges
        }
        RTerm::Return(_) | RTerm::Unreachable | RTerm::Throw(_) => Vec::new(),
    }
}

fn value_def_block(func: &Func, defs: &[Option<Site>], value: ValueId) -> Option<usize> {
    if let Some(site) = defs.get(value.0 as usize).copied().flatten() {
        return Some(site.block);
    }
    func.blocks
        .iter()
        .position(|block| block.params.contains(&value))
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

fn edge_to(term: &mut RTerm, target: usize) -> Option<&mut Vec<ValueId>> {
    match term {
        RTerm::Goto(found, args) if found.0 as usize == target => Some(args),
        _ => None,
    }
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

fn single_edge_param_sources(func: &Func) -> Vec<Option<ValueId>> {
    let mut sources = vec![None; func.values.len()];
    let reachable = distances(func, RBlockId(0));
    let defs = definitions(func);
    let doms = Dominators::of(func);
    let mut incoming: Vec<Vec<(usize, Vec<ValueId>)>> = vec![Vec::new(); func.blocks.len()];
    for (pred_idx, block) in func.blocks.iter().enumerate() {
        if reachable.get(pred_idx).copied().flatten().is_none() {
            continue;
        }
        for (target, args) in block.term.edges() {
            let t = target.0 as usize;
            if t == 0 || t >= func.blocks.len() {
                continue;
            }
            incoming[t].push((pred_idx, args.to_vec()));
        }
    }
    for (b_idx, block) in func.blocks.iter().enumerate() {
        if b_idx == 0 || reachable.get(b_idx).copied().flatten().is_none() {
            continue;
        }
        let edges = &incoming[b_idx];
        if edges.is_empty()
            || edges
                .iter()
                .any(|(_, args)| args.len() != block.params.len())
        {
            continue;
        }
        for (pos, &param) in block.params.iter().enumerate() {
            let mut single: Option<ValueId> = None;
            let mut valid = true;
            for (_, args) in edges {
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
                let Some(def_b) = value_def_block(func, &defs, source) else {
                    continue;
                };
                if !doms.dominates(RBlockId(def_b as u32), RBlockId(b_idx as u32)) {
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

fn strip_copies(
    func: &Func,
    defs: &[Option<Site>],
    single_edge: &[Option<ValueId>],
    mut value: ValueId,
) -> Option<ValueId> {
    for _ in 0..16 {
        let Some(op) = definition(func, defs, value) else {
            value = single_edge.get(value.0 as usize).copied().flatten()?;
            continue;
        };
        match op.kind {
            OpKind::Copy(source) | OpKind::Move(source) => value = source,
            _ => return Some(value),
        }
    }
    None
}

fn copy_root(
    func: &Func,
    defs: &[Option<Site>],
    single_edge: &[Option<ValueId>],
    mut value: ValueId,
) -> Option<ValueId> {
    for _ in 0..16 {
        match definition(func, defs, value).map(|op| &op.kind) {
            Some(OpKind::Copy(source) | OpKind::Move(source)) => value = *source,
            Some(_) => return Some(value),
            None => {
                if let Some(source) = single_edge.get(value.0 as usize).copied().flatten() {
                    value = source;
                } else {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn source_length_matches(
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    single_edge: &[Option<ValueId>],
    source: ValueId,
    len: ValueId,
) -> bool {
    let Some(source_root) = copy_root(func, defs, single_edge, source) else {
        return false;
    };
    let Some(len_root) = copy_root(func, defs, single_edge, len) else {
        return false;
    };
    let len_metadata_target = match definition(func, defs, len_root).map(|op| &op.kind) {
        Some(OpKind::Unary {
            op: UnaryOp::PtrMetadata,
            v,
            ..
        }) => copy_root(func, defs, single_edge, *v),
        _ => None,
    };
    if len_metadata_target == Some(source_root) {
        return true;
    }
    if let Some(op) = definition(func, defs, source_root) {
        match &op.kind {
            OpKind::SliceView { len: found, .. }
                if copy_root(func, defs, single_edge, *found) == Some(len_root) =>
            {
                return true;
            }
            OpKind::Call {
                callee: Callee::Runtime { sysroot: true, .. },
                semantic: CallSemantic::RawParts,
                args,
                ..
            } if args.len() == 2
                && copy_root(func, defs, single_edge, args[1]) == Some(len_root) =>
            {
                return true;
            }
            OpKind::Cast {
                kind: CastKind::Transmute | CastKind::Subtype | CastKind::PointerCoercion,
                v: str_val,
                from,
                to,
            } if is_str_ref_to_byte_slice_ref(table, *from, *to) => {
                if let Some(str_root) = copy_root(func, defs, single_edge, *str_val) {
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

fn strip_pointer_aliases(
    func: &Func,
    table: &TypeTable,
    defs: &[Option<Site>],
    single_edge: &[Option<ValueId>],
    mut value: ValueId,
) -> Option<ValueId> {
    for _ in 0..16 {
        let Some(op) = definition(func, defs, value) else {
            value = single_edge.get(value.0 as usize).copied().flatten()?;
            continue;
        };
        match op.kind {
            OpKind::Copy(source) | OpKind::Move(source) => value = source,
            OpKind::Cast { kind, v, from, to } if is_lossless_ptr_cast(table, kind, from, to) => {
                value = v
            }
            _ => return Some(value),
        }
    }
    None
}

fn value_ty(func: &Func, value: ValueId) -> Option<TypeId> {
    func.values.get(value.0 as usize).map(|def| def.ty)
}

fn is_scalar(
    func: &Func,
    defs: &[Option<Site>],
    single_edge: &[Option<ValueId>],
    value: ValueId,
    expected: u128,
) -> bool {
    strip_copies(func, defs, single_edge, value)
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
    let single_edge = single_edge_param_sources(func);
    let header = block(func, loop_.header, "header")?;
    let direct = block(func, loop_.direct, "direct body")?;
    block(func, loop_.body, "user body")?;
    block(func, loop_.exit, "exit")?;

    for (value, ty, name) in [
        (loop_.cursor, loop_.cursor_ty, "cursor"),
        (loop_.body_cursor, loop_.cursor_ty, "body cursor"),
        (
            loop_.enumerate_index,
            if matches!(
                loop_.adapter,
                IteratorIdentity::Take | IteratorIdentity::Skip | IteratorIdentity::Enumerate
            ) {
                if loop_.adapter == IteratorIdentity::Enumerate
                    && loop_.enumerate_index == loop_.body_cursor
                {
                    loop_.cursor_ty
                } else {
                    loop_.usize_ty
                }
            } else {
                loop_.cursor_ty
            },
            "enumerate index",
        ),
        (loop_.length, loop_.usize_ty, "length"),
        (loop_.bound_length, loop_.cursor_ty, "bound length"),
        (loop_.limit, loop_.usize_ty, "limit"),
        (loop_.initial, loop_.cursor_ty, "initial cursor"),
        (loop_.length_guard, loop_.bool_ty, "length guard"),
        (loop_.condition, loop_.bool_ty, "condition"),
        (loop_.item_ref, loop_.item_ref_ty, "item reference"),
        (loop_.one, loop_.cursor_ty, "increment"),
        (loop_.next, loop_.cursor_ty, "next cursor"),
    ] {
        if value_ty(func, value) != Some(ty) {
            return Err(format!("{name} v{} has the wrong type or is gone", value.0));
        }
    }
    if let Some(byte) = loop_.byte {
        if value_ty(func, byte) != Some(loop_.byte_ty) {
            return Err(format!("item v{} has the wrong type or is gone", byte.0));
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
    let valid_cursor_ty = match loop_.adapter {
        IteratorIdentity::Take | IteratorIdentity::Skip => {
            loop_.cursor_ty == loop_.usize_ty
                && table
                    .int_info(loop_.usize_ty)
                    .is_some_and(|(signed, _)| !signed)
                && table.int_info(loop_.byte_ty) == Some((false, 8))
        }
        IteratorIdentity::Slice | IteratorIdentity::Enumerate => {
            table
                .int_info(loop_.usize_ty)
                .is_some_and(|(signed, _)| !signed)
                && (loop_.cursor_ty == loop_.usize_ty
                    || table.int_info(loop_.cursor_ty) == Some((true, 32)))
                && is_sized_non_zst(table, loop_.byte_ty)
        }
        _ => false,
    };
    if !valid_cursor_ty
        || !is_ref_to(table, loop_.item_ref_ty, loop_.byte_ty)
        || !source_has_length(func, table, &defs, &single_edge, loop_.source, loop_.length)
    {
        return Err("the cursor, byte, reference, or source-length types changed".to_owned());
    }
    if loop_.cursor_ty != loop_.usize_ty {
        let Some(bound_op) = definition(func, &defs, loop_.bound_length) else {
            return Err("bound length cast is gone".to_owned());
        };
        if !matches!(
            bound_op.kind,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v,
                from,
                to,
            } if v == loop_.length && from == loop_.usize_ty && to == loop_.cursor_ty
        ) {
            return Err("bound length is not an IntToInt cast of slice length".to_owned());
        }
    } else if loop_.bound_length != loop_.length {
        return Err("bound length does not match slice length".to_owned());
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
            r: loop_.bound_length,
            ty: loop_.cursor_ty,
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
        IteratorIdentity::Skip | IteratorIdentity::Slice | IteratorIdentity::Enumerate
            if loop_.limit_guard.is_none() && loop_.condition == loop_.length_guard =>
        {
            vec![length_guard]
        }
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
        IteratorIdentity::Take | IteratorIdentity::Slice
            if loop_.enumerate_index == loop_.body_cursor => {}
        IteratorIdentity::Enumerate if loop_.enumerate_index == loop_.body_cursor => {}
        IteratorIdentity::Enumerate if loop_.cursor_ty != loop_.usize_ty => {
            expected_direct.push(Op {
                result: Some(loop_.enumerate_index),
                origin: loop_.origin,
                kind: OpKind::Cast {
                    kind: CastKind::IntToInt,
                    v: loop_.body_cursor,
                    from: loop_.cursor_ty,
                    to: loop_.usize_ty,
                },
            });
        }
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
    expected_direct.push(Op {
        result: Some(loop_.item_ref),
        origin: loop_.origin,
        kind: OpKind::BorrowView {
            root: Root::Value(loop_.source),
            path: vec![Proj::Deref, Proj::Index(loop_.body_cursor)],
            mutable: false,
        },
    });
    if let Some(byte) = loop_.byte {
        expected_direct.push(Op {
            result: Some(byte),
            origin: loop_.origin,
            kind: OpKind::Deref(loop_.item_ref),
        });
    }
    expected_direct.extend([
        Op {
            result: Some(loop_.one),
            origin: loop_.origin,
            kind: OpKind::Const(ConstTree::Scalar {
                bits: 1,
                bytes: loop_.cursor_bytes,
            }),
        },
        Op {
            result: Some(loop_.next),
            origin: loop_.origin,
            kind: OpKind::Binary {
                op: BinaryOp::Add,
                l: loop_.body_cursor,
                r: loop_.one,
                ty: loop_.cursor_ty,
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

    if matches!(
        loop_.adapter,
        IteratorIdentity::Take | IteratorIdentity::Slice | IteratorIdentity::Enumerate
    ) {
        if !is_scalar(func, &defs, &single_edge, loop_.initial, 0) {
            return Err("a cursor does not start at zero".to_owned());
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
    single_edge: &[Option<ValueId>],
    source: ValueId,
    len: ValueId,
) -> bool {
    let Some(source) = copy_root(func, defs, single_edge, source) else {
        return false;
    };
    let Some(source_ty) = value_ty(func, source) else {
        return false;
    };
    let Some(element) = sized_non_zst_slice_element(table, source_ty) else {
        return false;
    };
    if !is_slice_pointer_for(table, source_ty, element) {
        return false;
    }
    source_length_matches(func, table, defs, single_edge, source, len)
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
    const ISIZE: TypeId = TypeId(14);
    const U32: TypeId = TypeId(15);
    const SLICE_U32: TypeId = TypeId(16);
    const REF_SLICE_U32: TypeId = TypeId(17);
    const RAW_U32: TypeId = TypeId(18);
    const REF_U32: TypeId = TypeId(19);
    const ENUM_SLICE: TypeId = TypeId(20);
    const MUT_REF_RAW_U8: TypeId = TypeId(21);
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
        table.insert(aggregate_type(
            ENUM_SLICE,
            vec![SLICE_ITER],
            vec![field(SLICE_ITER, 0), field(USIZE, 1)],
            IteratorIdentity::Enumerate,
        ));
        table.insert(scalar(
            MUT_REF_RAW_U8,
            TypeKind::Reference {
                pointee: RAW_U8,
                mutable: true,
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
            Adapter::Plain | Adapter::Enumerate => {
                unreachable!("use plain/scalarized test builder")
            }
        };
        let inner_ty = match adapter {
            Adapter::Take => TAKE,
            Adapter::Skip => SKIP,
            Adapter::Plain | Adapter::Enumerate => {
                unreachable!("use plain/scalarized test builder")
            }
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
            Adapter::Plain | Adapter::Enumerate => {
                unreachable!("use plain/scalarized test builder")
            }
        };
        let continue_to = continue_block.unwrap_or(body);
        let expected_edges = match adapter {
            Adapter::Take => (exit, continue_to),
            Adapter::Skip => (continue_to, exit),
            Adapter::Plain | Adapter::Enumerate => {
                unreachable!("use plain/scalarized test builder")
            }
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
                    Adapter::Plain | Adapter::Enumerate => {
                        unreachable!("use plain/scalarized test builder")
                    }
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
            Adapter::Plain | Adapter::Enumerate => {
                unreachable!("use plain/scalarized test builder")
            }
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
        assert!(matches!(body.ops[1].kind, OpKind::Copy(value) if Some(value) == loop_.byte));
    }

    #[test]
    fn direct_skip_rewrites_the_remaining_user_body() {
        let (func, _table, loops) = lowered(Adapter::Skip);
        let loop_ = &loops[0];
        let body = &func.blocks[loop_.body.0 as usize];
        assert!(matches!(body.ops[0].kind, OpKind::Copy(value) if value == loop_.enumerate_index));
        assert!(matches!(body.ops[1].kind, OpKind::Copy(value) if Some(value) == loop_.byte));
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
        check_lowered(&func, &table, &loops).expect("witness checks");
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
        check_lowered(&func, &table, &loops).expect("witness checks");
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
        check_lowered(&func, &table, &loops).expect("witness checks");
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
    fn aggregate_plain_and_enumerate_slice_iter_lower_to_smi_i32_cursor() {
        for with_enumerate in [false, true] {
            let mut builder = FuncBuilder::new(
                ItemId {
                    definition: DefinitionId {
                        crate_id: 0,
                        local_hash: 99,
                    },
                    instantiation: 0,
                },
                Signature {
                    abi: AbiSignature {
                        parameters: 1,
                        ..AbiSignature::default()
                    },
                    arguments: vec![REF_SLICE],
                    result: ZST,
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
            let zst = builder.op(entry, OpKind::Const(ConstTree::Unit), ZST, O);
            let slice_iter = builder.op(
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
            let outer = if with_enumerate {
                let zero = constant(&mut builder, entry, 0);
                builder.op(
                    entry,
                    OpKind::Aggregate {
                        ty: ENUM_SLICE,
                        variant: 0,
                        active_field: None,
                        fields: vec![slice_iter, zero],
                    },
                    ENUM_SLICE,
                    O,
                )
            } else {
                slice_iter
            };
            let outer_ty = if with_enumerate {
                ENUM_SLICE
            } else {
                SLICE_ITER
            };
            builder.terminate(entry, RTerm::Goto(header, Vec::new()));

            let h_slice = if with_enumerate {
                builder.op(
                    header,
                    OpKind::Field {
                        base: outer,
                        variant: None,
                        field: 0,
                    },
                    SLICE_ITER,
                    O,
                )
            } else {
                outer
            };
            let h_ptr = builder.op(
                header,
                OpKind::Field {
                    base: h_slice,
                    variant: None,
                    field: 0,
                },
                RAW_U8,
                O,
            );
            let h_end = builder.op(
                header,
                OpKind::Field {
                    base: h_slice,
                    variant: None,
                    field: 1,
                },
                RAW_U8,
                O,
            );
            let eq = builder.op(
                header,
                OpKind::Binary {
                    op: BinaryOp::Eq,
                    l: h_ptr,
                    r: h_end,
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
            let step = builder.op(
                body,
                OpKind::Binary {
                    op: BinaryOp::Offset,
                    l: h_ptr,
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
            let state_path = if with_enumerate {
                vec![
                    Proj::Field {
                        field: 0,
                        ty: SLICE_ITER,
                    },
                    Proj::Field {
                        field: 0,
                        ty: RAW_U8,
                    },
                ]
            } else {
                vec![Proj::Field {
                    field: 0,
                    ty: RAW_U8,
                }]
            };
            let state = builder.op(
                body,
                OpKind::BorrowView {
                    root: Root::Value(outer),
                    path: state_path,
                    mutable: true,
                },
                MUT_REF_RAW_U8,
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
            let item_ptr = builder.op(
                body,
                OpKind::Cast {
                    kind: CastKind::PtrToPtr,
                    v: h_ptr,
                    from: RAW_U8,
                    to: RAW_U8,
                },
                RAW_U8,
                O,
            );
            let item_ref = builder.op(
                body,
                OpKind::BorrowView {
                    root: Root::Value(item_ptr),
                    path: vec![Proj::Deref],
                    mutable: false,
                },
                REF_U8,
                O,
            );
            if with_enumerate {
                let idx = add_count_update(&mut builder, body, outer);
                let byte = builder.op(body, OpKind::Deref(item_ref), U8, O);
                builder.op(body, OpKind::Copy(idx), USIZE, O);
                builder.op(body, OpKind::Copy(byte), U8, O);
            } else {
                let byte = builder.op(body, OpKind::Deref(item_ref), U8, O);
                builder.op(body, OpKind::Copy(byte), U8, O);
            }
            builder.terminate(body, RTerm::Goto(header, Vec::new()));
            builder.terminate(exit, RTerm::Return(None));

            let mut func = builder.finish();
            let _ = outer_ty;
            let tbl = table();
            func.verify(&tbl).expect("pre-pass Func::verify");
            let loops = run(&mut func, &tbl);
            assert_eq!(loops.len(), 1);
            check_lowered(&func, &tbl, &loops).expect("witness checks");
            assert_eq!(loops[0].cursor_ty, ISIZE);
            assert_eq!(
                loops[0].adapter,
                if with_enumerate {
                    IteratorIdentity::Enumerate
                } else {
                    IteratorIdentity::Slice
                }
            );
            crate::opt::dce::run(&mut func, &tbl);
            func.verify(&tbl).expect("Func::verify");
        }
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
        check_lowered(&func, &tbl, &loops).expect("witness checks");
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
        check_lowered(&func, &tbl, &loops).expect("witness checks");
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
        check_lowered(&func, &tbl, &loops).expect("witness checks");
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
        check_lowered(&func, &tbl, &loops).expect("witness checks");
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
        check_lowered(&func, &tbl, &loops).expect("witness checks");
        crate::opt::dce::run(&mut func, &tbl);
        func.verify(&tbl).expect("post-dce Func::verify");
    }

    fn build_borrowed_folded_take_loop(tainted_in_adapter: bool) -> (Func, TypeTable) {
        const REF_MUT_ENUM_TAKE: TypeId = TypeId(100);
        const REF_MUT_TAKE: TypeId = TypeId(101);
        const REF_MUT_USIZE: TypeId = TypeId(102);

        let mut tbl = table();
        tbl.insert(scalar(
            REF_MUT_ENUM_TAKE,
            TypeKind::Reference {
                pointee: ENUM_TAKE,
                mutable: true,
            },
            None,
        ));
        tbl.insert(scalar(
            REF_MUT_TAKE,
            TypeKind::Reference {
                pointee: TAKE,
                mutable: true,
            },
            None,
        ));
        tbl.insert(scalar(
            REF_MUT_USIZE,
            TypeKind::Reference {
                pointee: USIZE,
                mutable: true,
            },
            None,
        ));

        let mut builder = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 110,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 3,
                    ..AbiSignature::default()
                },
                arguments: vec![REF_SLICE, USIZE, RAW_U8],
                result: USIZE,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let source = builder.param(REF_SLICE, O);
        let limit = builder.param(USIZE, O);
        let other = builder.param(RAW_U8, O);
        let header = builder.block();
        let adapter = builder.block();
        let driver = builder.block();
        let exit = builder.block();

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
                ty: TAKE,
                variant: 0,
                active_field: None,
                fields: vec![slice, limit],
            },
            TAKE,
            O,
        );
        let untainted_ref = builder.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(other),
                path: vec![Proj::Deref],
                mutable: false,
            },
            REF_U8,
            O,
        );
        let zero_exit = constant(&mut builder, entry, 0);
        let outer = builder.op(
            entry,
            OpKind::Aggregate {
                ty: ENUM_TAKE,
                variant: 0,
                active_field: None,
                fields: vec![inner, initial],
            },
            ENUM_TAKE,
            O,
        );
        builder.terminate(entry, RTerm::Goto(header, Vec::new()));

        let pointer = builder.op(
            header,
            OpKind::BorrowView {
                root: Root::Value(outer),
                path: Vec::new(),
                mutable: true,
            },
            REF_MUT_ENUM_TAKE,
            O,
        );
        let inner_ptr = builder.op(
            header,
            OpKind::BorrowView {
                root: Root::Value(pointer),
                path: vec![Proj::Deref, Proj::Field { field: 0, ty: TAKE }],
                mutable: true,
            },
            REF_MUT_TAKE,
            O,
        );
        let inner_val = builder.op(header, OpKind::Deref(inner_ptr), TAKE, O);
        let n = builder.op(
            header,
            OpKind::Field {
                base: inner_val,
                variant: None,
                field: 1,
            },
            USIZE,
            O,
        );
        builder.terminate(
            header,
            RTerm::Switch {
                scrut: n,
                scrut_ty: USIZE,
                cases: vec![(0, exit, Vec::new())],
                default: (adapter, Vec::new()),
            },
        );

        let item_ref = if tainted_in_adapter {
            let slice_val = builder.op(
                adapter,
                OpKind::Field {
                    base: inner_val,
                    variant: None,
                    field: 0,
                },
                SLICE_ITER,
                O,
            );
            let cur_ptr = builder.op(
                adapter,
                OpKind::Field {
                    base: slice_val,
                    variant: None,
                    field: 0,
                },
                RAW_U8,
                O,
            );
            builder.op(
                adapter,
                OpKind::BorrowView {
                    root: Root::Value(cur_ptr),
                    path: vec![Proj::Deref],
                    mutable: false,
                },
                REF_U8,
                O,
            )
        } else {
            untainted_ref
        };
        builder.terminate(adapter, RTerm::Goto(driver, Vec::new()));

        let first = builder.op(driver, OpKind::Deref(pointer), ENUM_TAKE, O);
        let index = builder.op(
            driver,
            OpKind::Field {
                base: first,
                variant: None,
                field: 1,
            },
            USIZE,
            O,
        );
        let second = builder.op(driver, OpKind::Deref(pointer), ENUM_TAKE, O);
        let before = builder.op(
            driver,
            OpKind::Field {
                base: second,
                variant: None,
                field: 1,
            },
            USIZE,
            O,
        );
        let one = constant(&mut builder, driver, 1);
        let next = builder.op(
            driver,
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
            driver,
            OpKind::Binary {
                op: BinaryOp::Lt,
                l: next,
                r: before,
                ty: USIZE,
            },
            BOOL,
            O,
        );
        let checked = builder.op(driver, OpKind::Copy(overflow), BOOL, O);
        let third = builder.op(driver, OpKind::Deref(pointer), ENUM_TAKE, O);
        let assert_left = builder.op(
            driver,
            OpKind::Field {
                base: third,
                variant: None,
                field: 1,
            },
            USIZE,
            O,
        );
        let assert_one = constant(&mut builder, driver, 1);
        builder.effect(
            driver,
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
        let stored = builder.op(driver, OpKind::Copy(next), USIZE, O);
        let count_ptr = builder.op(
            driver,
            OpKind::BorrowView {
                root: Root::Value(pointer),
                path: vec![
                    Proj::Deref,
                    Proj::Field {
                        field: 1,
                        ty: USIZE,
                    },
                ],
                mutable: true,
            },
            REF_MUT_USIZE,
            O,
        );
        builder.effect(
            driver,
            OpKind::StoreThrough {
                ptr: count_ptr,
                v: stored,
            },
            O,
        );
        let byte = builder.op(driver, OpKind::Deref(item_ref), U8, O);
        let byte_u = builder.op(
            driver,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: byte,
                from: U8,
                to: USIZE,
            },
            USIZE,
            O,
        );
        builder.op(
            driver,
            OpKind::Binary {
                op: BinaryOp::Add,
                l: index,
                r: byte_u,
                ty: USIZE,
            },
            USIZE,
            O,
        );
        builder.terminate(driver, RTerm::Goto(header, Vec::new()));
        builder.terminate(exit, RTerm::Return(Some(zero_exit)));

        (builder.finish(), tbl)
    }

    #[test]
    fn borrowed_folded_untainted_deref_does_not_lower() {
        let (mut func, tbl) = build_borrowed_folded_take_loop(false);
        func.verify(&tbl).expect("pre-pass Func::verify");
        assert!(run(&mut func, &tbl).is_empty());
    }

    #[test]
    fn borrowed_folded_tainted_adapter_lowers() {
        let (mut func, tbl) = build_borrowed_folded_take_loop(true);
        func.verify(&tbl).expect("pre-pass Func::verify");
        let loops = run(&mut func, &tbl);
        assert_eq!(loops.len(), 1);
        check_lowered(&func, &tbl, &loops).expect("witness checks");
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
            check_lowered(&func, &tbl, &loops).expect("witness checks");
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
