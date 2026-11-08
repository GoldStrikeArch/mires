//! Aggregate, copy and view site accounting: what one lowered function BUILDS, and how much of it
//! it builds inside a cyclic region of the CFG.
//!
//! The name of the file is historical. What is counted is not the heap: it is the number of places
//! the emitted JavaScript writes a fresh object or array literal, plus the calls that reach the
//! global allocator, and `in_loop` is a **natural-loop and irreducible-SCC aggregate/copy/view site
//! count** — not a number of allocations at run time, which nothing static can know.
//!
//! The optimizer's claim is that an aggregate a function only takes apart again never has to be
//! built. That claim is not a byte count — the escape analysis trades an object literal for a few
//! more locals, and the byte count can go either way — so the gate is the number of such sites, and
//! above all the number of them the program runs **once per loop iteration**. A hot kernel with a
//! `{ high, low }` built inside its `while` is the shape Phase 3 exists to remove, and it is
//! invisible to every measure except this one.
//!
//! Five kinds are counted, all of them over the final [`Func`] of the lane's phase C — after the
//! passes, so what is counted is what will be printed:
//!
//! * `materialize`: an [`OpKind::Aggregate`] or [`OpKind::Repeat`] at a non-ZST, non-closure type —
//!   an object or array literal in the emitted JavaScript;
//! * `closure`: an `Aggregate` at a closure type, which prints as the captured-value array;
//! * `copy`: an [`OpKind::Copy`] whose type `needs_clone`, which prints as v1's `clone_expr`;
//! * `view`: a [`OpKind::BorrowView`], [`OpKind::RawView`], [`OpKind::SliceView`] or pointer
//!   [`BinaryOp::Offset`] whose result type's [`PointerForm`] is a RECORD — `{ buf, off }`, `{ buf,
//!   off, len }`, `{ ptr, meta }`. Those print a fresh object literal too, so leaving them out
//!   understated what a loop builds; a `Thin` or `Str` pointer is a bare value and is not counted.
//!   `view` is REPORTED and not gated: a reference carrier is not a PEA candidate by design
//!   (D4/D5), so a gate on it would be a gate on a decision, not on a regression;
//! * `alloc`: a [`CallSemantic::GlobalAllocator`] call, the `Vec`/`String`/`Box` heap.
//!
//! A ZST aggregate is not counted: it prints as a shared constant and allocates nothing.
//! A payload-only host call with an empty argument array instead materializes a fresh array
//! at the host boundary, and is counted there. The
//! `Vec | String | Box` split of the `alloc` kind waits for the carriers of a later phase; until
//! then one allocator call is one allocator call.
//!
//! # Loop depth
//!
//! Depth is natural-loop nesting taken from the CFG's back edges — an edge `b -> h` whose target
//! dominates its source — and never from the structurizer: a site's cost is how many loops
//! *contain* it, which is a fact about the graph, and a structurizer that spells one loop as two
//! or hides one behind a dispatch would change the number without changing the program. Back
//! edges that share a header are one loop, so a body with two `continue`s does not count twice.
//!
//! Natural loops alone miss an IRREDUCIBLE one — a cycle entered at two different blocks has no
//! edge whose target dominates its source, so it has no back edge and no natural loop, and a site
//! inside it would have been reported at depth 0 while running once per iteration. So every block
//! on a cycle (a strongly connected component with more than one block, or a block with an edge to
//! itself) is lifted to depth ≥ 1. Nesting inside an irreducible cycle is not counted: depth there
//! is 1, which is the honest floor rather than a guess.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::jir::repr::{Dominators, Func, Op, OpKind};
use crate::jir::semantic::{BinaryOp, CallSemantic};
use crate::jir::types::{PointerForm, TypeKind};
use crate::jir::{Interns, TypeId, TypeTable};
use crate::lane_report::push_string;

/// What one site allocates.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AllocKind {
    /// An object or array literal: `Aggregate` or `Repeat` at a non-closure type.
    Materialize,
    /// A `Copy` of a value whose type has to be cloned.
    Copy,
    /// A closure environment.
    Closure,
    /// A reference, raw pointer or slice view whose pointer form is a record.
    View,
    /// A call to the global allocator.
    Alloc,
}

impl AllocKind {
    /// The name the sidecar spells the kind by.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Materialize => "materialize",
            Self::Copy => "copy",
            Self::Closure => "closure",
            Self::View => "view",
            Self::Alloc => "alloc",
        }
    }
}

/// One allocation the function performs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocSite {
    /// Where it is, as `b<block>:<op index>` of the final `Func`.
    pub site: String,
    pub kind: AllocKind,
    /// How many natural loops contain the site. `0` is "runs at most once per call".
    pub depth: u32,
    /// The allocated type: the path rustc prints it by when the producer recorded one, and its
    /// id as `#<hex>` otherwise, so two sites in one loop can at least be told apart. Empty for
    /// an allocator call, which has no type of its own.
    pub origin: String,
}

/// The alloc-site accounting of one function.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AllocSites {
    pub materialize: u32,
    pub copy: u32,
    pub closure: u32,
    /// Record-shaped pointer views. Reported, never gated: see the module docs.
    pub view: u32,
    pub alloc: u32,
    /// Sites of any kind at depth ≥ 1 — inside a natural loop or an irreducible cycle.
    pub in_loop: u32,
    /// The `in_loop` sites themselves, in block and then op order. A site at depth 0 costs one
    /// allocation per call and is only worth a counter; a site in a loop is worth naming.
    pub sites: Vec<AllocSite>,
}

impl AllocSites {
    /// Counts every allocation `func` performs.
    ///
    /// `interns` is the item's whole intern table rather than just its [`TypeTable`] because a
    /// site's `origin` is the allocated type's printed name, which lives in `interns.strings`.
    pub fn of(func: &Func, interns: &Interns) -> Self {
        let table = &interns.types;
        let depths = loop_depths(func);
        let mut sites = Self::default();
        for block in &func.blocks {
            let depth = depths.get(block.id.0 as usize).copied().unwrap_or_default();
            for (index, op) in block.ops.iter().enumerate() {
                let Some((kind, ty)) = classify(func, op, table) else {
                    continue;
                };
                match kind {
                    AllocKind::Materialize => sites.materialize += 1,
                    AllocKind::Copy => sites.copy += 1,
                    AllocKind::Closure => sites.closure += 1,
                    AllocKind::View => sites.view += 1,
                    AllocKind::Alloc => sites.alloc += 1,
                }
                if depth > 0 {
                    sites.in_loop += 1;
                    sites.sites.push(AllocSite {
                        site: format!("b{}:{index}", block.id.0),
                        kind,
                        depth,
                        origin: ty.map(|ty| type_name(interns, ty)).unwrap_or_default(),
                    });
                }
            }
        }
        sites
    }

    /// The sidecar row's `alloc` block.
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(64 + 64 * self.sites.len());
        let _ = write!(
            out,
            "{{\"materialize\":{},\"copy\":{},\"closure\":{},\"view\":{},\"alloc\":{},\
             \"in_loop\":{},\"sites\":[",
            self.materialize, self.copy, self.closure, self.view, self.alloc, self.in_loop,
        );
        for (index, site) in self.sites.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"site\":");
            push_string(&mut out, &site.site);
            out.push_str(",\"kind\":");
            push_string(&mut out, site.kind.as_str());
            let _ = write!(out, ",\"depth\":{},\"origin\":", site.depth);
            push_string(&mut out, &site.origin);
            out.push('}');
        }
        out.push_str("]}");
        out
    }
}

/// What one op allocates, with the type it allocates it at.
pub(crate) fn classify(
    func: &Func,
    op: &Op,
    table: &TypeTable,
) -> Option<(AllocKind, Option<TypeId>)> {
    match &op.kind {
        OpKind::Aggregate { ty, .. } => {
            let ty = *ty;
            if table.is_zst(ty) {
                return None;
            }
            let kind = match table.get(ty).map(|descriptor| &descriptor.kind) {
                Some(TypeKind::Closure { .. }) => AllocKind::Closure,
                _ => AllocKind::Materialize,
            };
            Some((kind, Some(ty)))
        }
        // The array the repeat builds is the op's result type; `Repeat` carries only its element.
        OpKind::Repeat { .. } => {
            let ty = result_type(func, op)?;
            (!table.is_zst(ty)).then_some((AllocKind::Materialize, Some(ty)))
        }
        // A copy of a primitive is a rename and costs nothing; only a `needs_clone` type prints
        // v1's clone expression, which is where the allocation is.
        OpKind::Copy(v) => {
            let ty = func.values.get(v.0 as usize)?.ty;
            table.needs_clone(ty).then_some((AllocKind::Copy, Some(ty)))
        }
        // A borrow, a raw pointer or a slice view prints a fresh `{ buf, off }` / `{ buf, off,
        // len }` / `{ ptr, meta }` record whenever the result type's pointer form is one of those.
        // A `Thin` reference is the pointee's own value and a `&str` is a JS string: neither
        // builds anything.
        OpKind::BorrowView { .. } | OpKind::RawView { .. } | OpKind::SliceView { .. } => {
            let ty = result_type(func, op)?;
            record_pointer(table, ty).then_some((AllocKind::View, Some(ty)))
        }
        // `ptr.offset(n)` on a record pointer is `{ buf, off: p.off + n }`: another record.
        OpKind::Binary {
            op: BinaryOp::Offset,
            ..
        } => {
            let ty = result_type(func, op)?;
            record_pointer(table, ty).then_some((AllocKind::View, Some(ty)))
        }
        OpKind::Call {
            semantic: CallSemantic::GlobalAllocator,
            ..
        } => Some((AllocKind::Alloc, None)),
        OpKind::Host { args, .. } => {
            let ty = func.values.get(args.last()?.0 as usize)?.ty;
            matches!(table.get(ty)?.kind, TypeKind::Array { length: 0, .. })
                .then_some((AllocKind::Materialize, Some(ty)))
        }
        _ => None,
    }
}

/// Whether a pointer type is carried as a JS record rather than as a bare value.
fn record_pointer(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        table
            .get(ty)
            .and_then(|descriptor| descriptor.js.pointer_form),
        Some(
            PointerForm::Slot | PointerForm::FatSlice | PointerForm::FatDyn | PointerForm::FatTail
        )
    )
}

/// The type of the value an op defines.
fn result_type(func: &Func, op: &Op) -> Option<TypeId> {
    let result = op.result?;
    func.values.get(result.0 as usize).map(|value| value.ty)
}

/// The allocated type's printed Rust path, or its id when the producer recorded no name.
///
/// A name is only in the type table when something in the program asked for one (`type_name`),
/// so most of the time this is the id — which is still enough to tell two types apart.
pub(crate) fn type_name(interns: &Interns, ty: TypeId) -> String {
    if let Some(name) = interns
        .types
        .get(ty)
        .and_then(|descriptor| descriptor.name)
        .and_then(|name| interns.strings.get(name.0 as usize).cloned())
    {
        return name;
    }
    let Some(descriptor) = interns.types.get(interns.types.peeled(ty)) else {
        return format!("#{:08x}", ty.0 as u32);
    };
    match &descriptor.kind {
        TypeKind::Bool => "bool".to_owned(),
        TypeKind::Char => "char".to_owned(),
        TypeKind::Integer { signed, bits } => {
            format!("{}{}", if *signed { 'i' } else { 'u' }, bits)
        }
        TypeKind::Float { bits } => format!("f{bits}"),
        TypeKind::Str => "str".to_owned(),
        TypeKind::Never => "!".to_owned(),
        TypeKind::Tuple(fields) => {
            let fields = fields
                .iter()
                .map(|field| type_name(interns, *field))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({fields})")
        }
        TypeKind::Array { element, length } => {
            format!("[{}; {length}]", type_name(interns, *element))
        }
        TypeKind::Slice(element) => format!("[{}]", type_name(interns, *element)),
        TypeKind::Reference { pointee, mutable } => format!(
            "&{}{}",
            if *mutable { "mut " } else { "" },
            type_name(interns, *pointee)
        ),
        TypeKind::RawPointer { pointee, mutable } => format!(
            "*{} {}",
            if *mutable { "mut" } else { "const" },
            type_name(interns, *pointee)
        ),
        _ => format!("#{:08x}", ty.0 as u32),
    }
}

/// How many cycles contain each block, indexed by block id.
///
/// A back edge is an edge whose target dominates its source; its natural loop is the target plus
/// every block that reaches the source without passing through the target. Back edges sharing a
/// header make one loop, and an unreachable block is in none. A block on a cycle that has no back
/// edge at all — an irreducible loop, entered at two blocks — is then lifted to 1, so that "depth
/// 0" means "not on any cycle" and never "on a cycle rustc's loop finder cannot name".
pub(crate) fn loop_depths(func: &Func) -> Vec<u32> {
    let count = func.blocks.len();
    let doms = Dominators::of(func);
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); count];
    for block in &func.blocks {
        if !doms.is_reachable(block.id) {
            continue;
        }
        for successor in block.term.successors() {
            if (successor.0 as usize) < count {
                predecessors[successor.0 as usize].push(block.id.0 as usize);
            }
        }
    }

    let mut loops: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for block in &func.blocks {
        if !doms.is_reachable(block.id) {
            continue;
        }
        for successor in block.term.successors() {
            if (successor.0 as usize) >= count || !doms.dominates(successor, block.id) {
                continue;
            }
            let body = loops.entry(successor.0 as usize).or_default();
            // The header goes in first, which is also what stops the walk below at it.
            body.insert(successor.0 as usize);
            let mut stack = vec![block.id.0 as usize];
            while let Some(node) = stack.pop() {
                if !body.insert(node) {
                    continue;
                }
                stack.extend(predecessors[node].iter().copied());
            }
        }
    }

    let mut depths = vec![0; count];
    for body in loops.values() {
        for &block in body {
            depths[block] += 1;
        }
    }
    for (block, on_cycle) in cyclic_blocks(func, &doms).into_iter().enumerate() {
        if on_cycle && depths[block] == 0 {
            depths[block] = 1;
        }
    }
    depths
}

/// Which reachable blocks lie on a cycle: Tarjan's strongly connected components, with a component
/// counted when it has more than one block or when its one block has an edge to itself.
///
/// Iterative rather than recursive: a `Func` from a real crate can have thousands of blocks, and
/// this runs inside the compiler.
fn cyclic_blocks(func: &Func, doms: &Dominators) -> Vec<bool> {
    let count = func.blocks.len();
    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut reachable = vec![false; count];
    for block in &func.blocks {
        let from = block.id.0 as usize;
        if from >= count || !doms.is_reachable(block.id) {
            continue;
        }
        reachable[from] = true;
        for successor in block.term.successors() {
            if (successor.0 as usize) < count && doms.is_reachable(successor) {
                successors[from].push(successor.0 as usize);
            }
        }
    }

    const UNVISITED: usize = usize::MAX;
    let mut index = vec![UNVISITED; count];
    let mut low = vec![0usize; count];
    let mut on_stack = vec![false; count];
    let mut component: Vec<usize> = Vec::new();
    let mut cyclic = vec![false; count];
    let mut next = 0usize;
    // The depth-first stack, as (block, the successor to visit next).
    let mut work: Vec<(usize, usize)> = Vec::new();

    for root in 0..count {
        if index[root] != UNVISITED || !reachable[root] {
            continue;
        }
        work.push((root, 0));
        while let Some((block, edge)) = work.pop() {
            if edge == 0 {
                index[block] = next;
                low[block] = next;
                next += 1;
                component.push(block);
                on_stack[block] = true;
            }
            if let Some(&successor) = successors[block].get(edge) {
                work.push((block, edge + 1));
                if index[successor] == UNVISITED {
                    work.push((successor, 0));
                } else if on_stack[successor] {
                    low[block] = low[block].min(index[successor]);
                }
                continue;
            }
            if low[block] == index[block] {
                let mut members: Vec<usize> = Vec::new();
                while let Some(top) = component.pop() {
                    on_stack[top] = false;
                    members.push(top);
                    if top == block {
                        break;
                    }
                }
                if members.len() > 1 || successors[block].contains(&block) {
                    for member in members {
                        cyclic[member] = true;
                    }
                }
            }
            if let Some((parent, _)) = work.last() {
                let parent = *parent;
                low[parent] = low[parent].min(low[block]);
            }
        }
    }
    cyclic
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::repr::{FuncBuilder, RTerm};
    use crate::jir::semantic::{AbiSignature, Callee, ConstTree, DefinitionId, ItemId, Signature};
    use crate::jir::types::{JsFacts, LayoutFacts, TypeDescriptor, TypeMeta};
    use crate::jir::OriginId;

    const U32: TypeId = TypeId(1);
    const PAIR: TypeId = TypeId(2);
    const ENV: TypeId = TypeId(3);
    const UNIT: TypeId = TypeId(4);
    const SLICE_REF: TypeId = TypeId(5);
    const THIN_REF: TypeId = TypeId(6);
    const NO_ORIGIN: OriginId = OriginId(0);

    fn descriptor(id: TypeId, kind: TypeKind, zst: bool, needs_clone: bool) -> TypeDescriptor {
        TypeDescriptor {
            id,
            name: Some(crate::jir::StrId(0)),
            kind,
            layout: LayoutFacts {
                zst,
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: !zst,
                needs_clone,
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
        }
    }

    /// A reference type carried in the given pointer form.
    fn pointer(id: TypeId, form: PointerForm) -> TypeDescriptor {
        let mut descriptor = descriptor(
            id,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            false,
            false,
        );
        descriptor.js.pointer_form = Some(form);
        descriptor
    }

    fn interns() -> Interns {
        let mut interns = Interns::new();
        interns.strings.push("Pair".to_owned());
        for descriptor in [
            descriptor(
                U32,
                TypeKind::Integer {
                    signed: false,
                    bits: 32,
                },
                false,
                false,
            ),
            descriptor(PAIR, TypeKind::Tuple(vec![U32, U32]), false, true),
            descriptor(
                ENV,
                TypeKind::Closure {
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 0,
                    },
                    upvars: vec![U32],
                    fn_pointer_shim: None,
                },
                false,
                false,
            ),
            descriptor(UNIT, TypeKind::Tuple(Vec::new()), true, false),
            pointer(SLICE_REF, PointerForm::FatSlice),
            pointer(THIN_REF, PointerForm::Thin),
        ] {
            interns.types.insert(descriptor);
        }
        interns
    }

    fn builder() -> FuncBuilder {
        FuncBuilder::new(
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
                },
                arguments: Vec::new(),
                result: U32,
            },
        )
    }

    fn pair(fields: Vec<crate::jir::repr::ValueId>) -> OpKind {
        OpKind::Aggregate {
            ty: PAIR,
            variant: 0,
            active_field: None,
            fields,
        }
    }

    /// A `while` body's aggregate is at depth 1, the one before the loop at depth 0, and the ZST
    /// aggregate is not an allocation at all.
    #[test]
    fn a_loop_body_site_is_counted_in_loop() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let header = b.block();
        let body = b.block();
        let exit = b.block();
        let zero = b.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            U32,
            NO_ORIGIN,
        );
        let _outside = b.op(entry, pair(vec![zero, zero]), PAIR, NO_ORIGIN);
        let _nothing = b.op(
            entry,
            OpKind::Aggregate {
                ty: UNIT,
                variant: 0,
                active_field: None,
                fields: Vec::new(),
            },
            UNIT,
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Goto(header, Vec::new()));
        b.terminate(
            header,
            RTerm::Switch {
                scrut: zero,
                scrut_ty: U32,
                cases: vec![(0, exit, Vec::new())],
                default: (body, Vec::new()),
            },
        );
        let inside = b.op(body, pair(vec![zero, zero]), PAIR, NO_ORIGIN);
        let _cloned = b.op(body, OpKind::Copy(inside), PAIR, NO_ORIGIN);
        let _env = b.op(
            body,
            OpKind::Aggregate {
                ty: ENV,
                variant: 0,
                active_field: None,
                fields: vec![zero],
            },
            ENV,
            NO_ORIGIN,
        );
        b.effect(
            body,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: DefinitionId {
                            crate_id: 0,
                            local_hash: 1,
                        },
                        instantiation: 0,
                    },
                    link_name: crate::jir::StrId(0),
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::GlobalAllocator,
                host: None,
            },
            NO_ORIGIN,
        );
        b.terminate(body, RTerm::Goto(header, Vec::new()));
        b.terminate(exit, RTerm::Return(Some(zero)));
        let func = b.finish();

        let sites = AllocSites::of(&func, &interns());
        assert_eq!(
            sites.materialize, 2,
            "the ZST aggregate is not an allocation"
        );
        assert_eq!((sites.copy, sites.closure, sites.alloc), (1, 1, 1));
        assert_eq!(
            sites.in_loop, 4,
            "everything in the body, and nothing before it"
        );
        assert!(sites.sites.iter().all(|site| site.depth == 1));
        assert_eq!(sites.sites[0].site, "b2:0");
        assert_eq!(sites.sites[0].origin, "Pair");
        assert!(sites
            .to_json()
            .starts_with("{\"materialize\":2,\"copy\":1,"));
    }

    /// A view whose pointer form is a record is a site; a `Thin` reference is the pointee's own
    /// value and builds nothing.
    #[test]
    fn only_record_shaped_views_are_counted() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let zero = b.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            U32,
            NO_ORIGIN,
        );
        let _fat = b.op(
            entry,
            OpKind::SliceView {
                ptr: zero,
                len: zero,
            },
            SLICE_REF,
            NO_ORIGIN,
        );
        let _thin = b.op(
            entry,
            OpKind::SliceView {
                ptr: zero,
                len: zero,
            },
            THIN_REF,
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(Some(zero)));
        let func = b.finish();

        let sites = AllocSites::of(&func, &interns());
        assert_eq!((sites.view, sites.materialize), (1, 0));
    }

    /// A copy of a type that does not clone is a rename, and costs nothing.
    #[test]
    fn a_copy_of_a_primitive_is_not_an_allocation() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let zero = b.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            U32,
            NO_ORIGIN,
        );
        let copied = b.op(entry, OpKind::Copy(zero), U32, NO_ORIGIN);
        b.terminate(entry, RTerm::Return(Some(copied)));
        let func = b.finish();

        assert_eq!(AllocSites::of(&func, &interns()), AllocSites::default());
    }

    /// An irreducible cycle — two blocks that jump to each other, entered at either — has no back
    /// edge and so no natural loop, and its sites would be reported at depth 0 without the SCC
    /// lift.
    #[test]
    fn an_irreducible_cycle_counts_as_a_loop() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let left = b.block();
        let right = b.block();
        let exit = b.block();
        let zero = b.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            U32,
            NO_ORIGIN,
        );
        // Two entries into the cycle: neither `left` nor `right` dominates the other.
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: zero,
                scrut_ty: U32,
                cases: vec![(0, left, Vec::new())],
                default: (right, Vec::new()),
            },
        );
        let inside = b.op(left, pair(vec![zero, zero]), PAIR, NO_ORIGIN);
        b.terminate(left, RTerm::Goto(right, Vec::new()));
        b.terminate(
            right,
            RTerm::Switch {
                scrut: zero,
                scrut_ty: U32,
                cases: vec![(0, exit, Vec::new())],
                default: (left, Vec::new()),
            },
        );
        b.terminate(exit, RTerm::Return(Some(inside)));
        let func = b.finish();

        let sites = AllocSites::of(&func, &interns());
        assert_eq!(sites.materialize, 1);
        assert_eq!(
            sites.in_loop, 1,
            "the cycle has no back edge, but it is a cycle"
        );
        assert_eq!(sites.sites[0].depth, 1);
    }

    /// Two back edges to one header are one loop, and the inner loop's blocks are at depth 2.
    #[test]
    fn nesting_counts_loops_not_back_edges() {
        let mut b = builder();
        let entry = FuncBuilder::ENTRY;
        let outer = b.block();
        let inner = b.block();
        let latch = b.block();
        let exit = b.block();
        let zero = b.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            U32,
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Goto(outer, Vec::new()));
        b.terminate(
            outer,
            RTerm::Switch {
                scrut: zero,
                scrut_ty: U32,
                cases: vec![(0, exit, Vec::new())],
                default: (inner, Vec::new()),
            },
        );
        // The inner loop has two back edges to `inner`: the direct one and the one through
        // `latch`. That is one loop, not two.
        let deep = b.op(inner, pair(vec![zero, zero]), PAIR, NO_ORIGIN);
        b.terminate(
            inner,
            RTerm::Switch {
                scrut: zero,
                scrut_ty: U32,
                cases: vec![(0, inner, Vec::new()), (1, latch, Vec::new())],
                default: (outer, Vec::new()),
            },
        );
        b.terminate(latch, RTerm::Goto(inner, Vec::new()));
        b.terminate(exit, RTerm::Return(Some(deep)));
        let func = b.finish();

        let sites = AllocSites::of(&func, &interns());
        assert_eq!(sites.sites.len(), 1);
        assert_eq!(
            sites.sites[0].depth, 2,
            "the inner loop nests in the outer one"
        );
    }
}
