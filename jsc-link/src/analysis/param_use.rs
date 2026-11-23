//! Per parameter use summaries: *does this function only ever read and write the fields of that
//! argument?*
//!
//! The flat direct-call ABI ([`crate::plan::abi_flat`]) hands a callee the fields of an aggregate
//! argument instead of the aggregate, and rebuilds the aggregate in the callee's entry block for
//! the body to use. For a by-value argument that is always sound — Rust already gave the callee
//! its own copy — but it is only *worth* doing when the rebuilt aggregate then disappears, which
//! it does exactly when the body reads and writes fields and nothing else.
//!
//! For a `&T`/`&mut T` argument the same summary is a **correctness** condition. The rewrite
//! passes the pointee's fields by value and copies the changed ones back at every `Return`, so a
//! body that lets the pointer itself out — stores it, passes it to something that keeps it,
//! compares it — would observe the copy rather than the caller's storage. Every such use is an
//! escape here, and the parameter stays in the baseline shape.
//!
//! The rules are the ones the partial escape analysis uses on an object node
//! (`checkpoints/phase-3-briefs.md`, D4), narrowed to what reaches a parameter: an **object** is
//! the aggregate itself and whatever `Copy`/`Move` renames it to, a **view** is a pointer into it
//! (`BorrowView` with a `Deref`/`Field`/`Downcast` path, which is what `lower/ssa.rs` builds for
//! `self.field = v` on a `&mut self`), and every use this module does not name is an escape. The
//! one use that is neither is passing the parameter (or a reborrow of it) to another direct
//! callee: that is a *forward*, and whether it counts as field-only depends on whether that
//! callee's own parameter goes flat — a fixed point [`crate::plan::abi_flat`] runs, because only
//! it knows the whole program.

use std::collections::{BTreeMap, BTreeSet};

use crate::jir::repr::{Func, Op, OpKind, Proj, Root, ValueId};
use crate::jir::semantic::{Callee, ItemId, UnaryOp};
use crate::jir::TypeTable;

/// How one parameter is passed, which decides which rules apply to it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParamKind {
    /// The aggregate itself.
    ByValue,
    /// A reference to it.
    ByRef,
    /// A reference to a 1-lane Copy scalar (`&Scalar` / `&mut Scalar`).
    ByRefScalar,
    /// A mutable reference to a small tag-represented non-Drop enum (`&mut Enum`).
    ByRefEnum,
    /// A slice pointer carried as physical `buf`, `off`, and `len` lanes.
    Slice,
}

/// What one parameter's uses came to.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ParamUse {
    /// Whether a use this module does not recognize as a field access was found. A parameter that
    /// escapes is never flattened.
    pub escapes: bool,
    /// Whether the body ever *writes* a field of the parameter. A `&mut T` that is only read is a
    /// `&T` as far as the flat ABI is concerned: it needs no copy-back and no spill register.
    /// A forward counts as a write, because what the callee does with it is not this summary's
    /// to say.
    pub written: bool,
    /// Whether this body itself mutates the value. A pure direct-call forward sets `written`
    /// conservatively for reference copy-back, but tagged by-value lanes only need to reject an
    /// actual `SetField`, `SetDiscriminant`, or store through a view.
    pub mutated: bool,
    /// The `(callee, parameter index)` pairs the parameter flows into as a direct call argument.
    /// Field-only *if* those parameters are themselves flattened.
    pub forwards: BTreeSet<(ItemId, usize)>,
}

/// Summarizes each parameter of `func` named by `kinds`.
///
/// `kinds[i]` is `None` for a parameter that is not an aggregate candidate at all; its summary is
/// [`ParamUse::escapes`], because nothing may be done with it.
pub fn summarize(func: &Func, table: &TypeTable, kinds: &[Option<ParamKind>]) -> Vec<ParamUse> {
    let index = UseIndex::of(func);
    kinds
        .iter()
        .enumerate()
        .map(|(position, kind)| match (kind, func.params.get(position)) {
            (Some(kind), Some(param)) => summarize_one(func, table, &index, *param, *kind),
            _ => ParamUse {
                escapes: true,
                ..ParamUse::default()
            },
        })
        .collect()
}

/// Where each value is used: the ops that read it, and whether a terminator does.
struct UseIndex {
    ops: BTreeMap<ValueId, Vec<(u32, usize)>>,
    terminator: BTreeSet<ValueId>,
}

impl UseIndex {
    fn of(func: &Func) -> UseIndex {
        let mut ops: BTreeMap<ValueId, Vec<(u32, usize)>> = BTreeMap::new();
        let mut terminator = BTreeSet::new();
        for block in &func.blocks {
            for (position, op) in block.ops.iter().enumerate() {
                for v in op.kind.uses() {
                    let sites = ops.entry(v).or_default();
                    if sites.last() != Some(&(block.id.0, position)) {
                        sites.push((block.id.0, position));
                    }
                }
            }
            terminator.extend(block.term.uses());
        }
        UseIndex { ops, terminator }
    }

    fn op<'a>(&self, func: &'a Func, site: (u32, usize)) -> &'a Op {
        &func.blocks[site.0 as usize].ops[site.1]
    }
}

/// The verdict one op passes on one tracked value.
enum Verdict {
    /// A field read: nothing more to track.
    Fine,
    /// A field write.
    Wrote,
    /// The op's result is the same aggregate.
    Object,
    /// The op's result is another pointer into the aggregate.
    View,
    /// The op's result is a scalar subfield view inside an enum (`&mut (*p).field`).
    SubView,
    /// The value is a direct call argument, at these positions.
    Forward(ItemId, Vec<usize>),
    Escape,
}

fn summarize_one(
    func: &Func,
    table: &TypeTable,
    index: &UseIndex,
    param: ValueId,
    kind: ParamKind,
) -> ParamUse {
    let mut objects: BTreeSet<ValueId> = BTreeSet::new();
    let mut views: BTreeSet<ValueId> = BTreeSet::new();
    let mut sub_views: BTreeSet<ValueId> = BTreeSet::new();
    match kind {
        ParamKind::ByValue => objects.insert(param),
        ParamKind::ByRef | ParamKind::ByRefScalar | ParamKind::ByRefEnum | ParamKind::Slice => {
            views.insert(param)
        }
    };
    let mut use_ = ParamUse::default();
    // The sets only grow, so re-walking them until none changes terminates.
    loop {
        let (before_objects, before_views, before_sub_views) =
            (objects.len(), views.len(), sub_views.len());
        let tracked: Vec<(ValueId, TrackedRole)> = objects
            .iter()
            .map(|v| (*v, TrackedRole::Object))
            .chain(views.iter().map(|v| (*v, TrackedRole::View)))
            .chain(sub_views.iter().map(|v| (*v, TrackedRole::SubView)))
            .collect();
        for (v, role) in tracked {
            if index.terminator.contains(&v) {
                use_.escapes = true;
            }
            for site in index.ops.get(&v).map(Vec::as_slice).unwrap_or_default() {
                let op = index.op(func, *site);
                match classify(op, v, role, kind, func, table) {
                    Verdict::Fine => {}
                    // A `Deref` of a view yields either the aggregate itself, which is another
                    // object to track, or one scalar field, which is a plain read.
                    Verdict::Object => match op.result {
                        Some(result) => {
                            let aggregate = !matches!(op.kind, OpKind::Deref(_))
                                || table.needs_clone(func.values[result.0 as usize].ty);
                            if aggregate {
                                objects.insert(result);
                            }
                        }
                        None => use_.escapes = true,
                    },
                    Verdict::View => match op.result {
                        Some(result) => {
                            views.insert(result);
                        }
                        None => use_.escapes = true,
                    },
                    Verdict::SubView => match op.result {
                        Some(result) => {
                            sub_views.insert(result);
                        }
                        None => use_.escapes = true,
                    },
                    Verdict::Wrote => {
                        use_.written = true;
                        use_.mutated = true;
                    }
                    Verdict::Forward(item, positions) => {
                        use_.written = true;
                        if kind == ParamKind::ByValue && role == TrackedRole::View {
                            use_.mutated = true;
                        }
                        use_.forwards
                            .extend(positions.into_iter().map(|position| (item, position)));
                    }
                    Verdict::Escape => use_.escapes = true,
                }
            }
        }
        if use_.escapes
            || (objects.len() == before_objects
                && views.len() == before_views
                && sub_views.len() == before_sub_views)
        {
            break;
        }
    }
    use_
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TrackedRole {
    Object,
    View,
    SubView,
}

/// What `op` does with `v`, which is an aggregate, a pointer into one, or a subfield view.
///
/// Every arm is a use this module can prove reads or writes a field. The wildcard is an escape,
/// deliberately: a rule that is not written down here is not one the plan may rely on.
fn classify(
    op: &Op,
    v: ValueId,
    role: TrackedRole,
    kind: ParamKind,
    func: &Func,
    table: &TypeTable,
) -> Verdict {
    // The positions that let the aggregate itself out, whatever else the op does with it.
    let escaping = match &op.kind {
        OpKind::SetField { v: stored, .. } => *stored == v,
        OpKind::StoreThrough { v: stored, .. } => *stored == v,
        OpKind::Aggregate { fields, .. } => fields.contains(&v),
        _ => false,
    };
    if escaping {
        return Verdict::Escape;
    }
    if role == TrackedRole::SubView {
        return match &op.kind {
            OpKind::Copy(src) | OpKind::Move(src) if *src == v => Verdict::SubView,
            OpKind::Deref(ptr) if *ptr == v => match op.result.and_then(|r| func.ty(r)) {
                Some(ty) if !table.needs_clone(ty) => Verdict::Fine,
                _ => Verdict::Escape,
            },
            OpKind::StoreThrough { ptr, .. } if *ptr == v => Verdict::Wrote,
            _ => Verdict::Escape,
        };
    }
    let is_view = role == TrackedRole::View;
    if kind == ParamKind::ByRefScalar {
        return match &op.kind {
            OpKind::Copy(src) | OpKind::Move(src) if *src == v => Verdict::View,
            OpKind::Deref(ptr) if *ptr == v => Verdict::Fine,
            OpKind::StoreThrough { ptr, .. } if *ptr == v => Verdict::Wrote,
            OpKind::BorrowView {
                root: Root::Value(root),
                path,
                ..
            } if *root == v && path.as_slice() == [Proj::Deref] => Verdict::View,
            OpKind::Call {
                callee: Callee::Direct { item, .. },
                args,
                host: None,
                ..
            } => {
                let positions: Vec<usize> = args
                    .iter()
                    .enumerate()
                    .filter(|(_, arg)| **arg == v)
                    .map(|(position, _)| position)
                    .collect();
                match positions.is_empty() {
                    false => Verdict::Forward(*item, positions),
                    true => Verdict::Escape,
                }
            }
            _ => Verdict::Escape,
        };
    }
    if kind == ParamKind::ByRefEnum {
        return match &op.kind {
            OpKind::Copy(_) | OpKind::Move(_) => match is_view {
                true => Verdict::View,
                false => Verdict::Object,
            },
            OpKind::Discriminant(base) if !is_view && *base == v => Verdict::Fine,
            OpKind::Field { base, .. } if !is_view && *base == v => Verdict::Fine,
            OpKind::SetField { base, .. } | OpKind::SetDiscriminant { base, .. }
                if !is_view && *base == v =>
            {
                Verdict::Wrote
            }
            OpKind::Deref(ptr) if is_view && *ptr == v => Verdict::Object,
            OpKind::StoreThrough { ptr, .. } if is_view && *ptr == v => Verdict::Wrote,
            OpKind::BorrowView {
                root: Root::Value(root),
                path,
                ..
            } if is_view && *root == v => {
                if path.as_slice() == [Proj::Deref] {
                    Verdict::View
                } else if enum_subfield_path(path) {
                    Verdict::SubView
                } else {
                    Verdict::Escape
                }
            }
            OpKind::Call {
                callee: Callee::Direct { item, .. },
                args,
                host: None,
                ..
            } if is_view => {
                let positions: Vec<usize> = args
                    .iter()
                    .enumerate()
                    .filter(|(_, arg)| **arg == v)
                    .map(|(position, _)| position)
                    .collect();
                match positions.is_empty() {
                    false => Verdict::Forward(*item, positions),
                    true => Verdict::Escape,
                }
            }
            _ => Verdict::Escape,
        };
    }
    if kind == ParamKind::Slice {
        return match &op.kind {
            OpKind::Copy(source) | OpKind::Move(source) if *source == v => Verdict::View,
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v: source,
                ..
            } if *source == v => Verdict::Fine,
            // An element/subslice view preserves the underlying buffer and offset. Its own uses
            // need no materialized fat-pointer object at this boundary.
            OpKind::BorrowView {
                root: Root::Value(root),
                path,
                ..
            }
            | OpKind::RawView {
                root: Root::Value(root),
                path,
                ..
            } if *root == v && slice_path(path) => {
                if path.len() == 1
                    || path
                        .iter()
                        .any(|proj| matches!(proj, Proj::Subslice { .. }))
                {
                    Verdict::View
                } else {
                    Verdict::Fine
                }
            }
            OpKind::SliceView { ptr, .. } if *ptr == v => Verdict::View,
            OpKind::Call {
                callee: Callee::Direct { item, .. },
                args,
                host: None,
                ..
            } => {
                let positions: Vec<usize> = args
                    .iter()
                    .enumerate()
                    .filter(|(_, arg)| **arg == v)
                    .map(|(position, _)| position)
                    .collect();
                match positions.is_empty() {
                    false => Verdict::Forward(*item, positions),
                    true => Verdict::Escape,
                }
            }
            _ => Verdict::Escape,
        };
    }
    match &op.kind {
        OpKind::Copy(_) | OpKind::Move(_) => match is_view {
            true => Verdict::View,
            false => Verdict::Object,
        },
        OpKind::Field { base, .. } if !is_view && *base == v => Verdict::Fine,
        OpKind::SetField { base, .. } if !is_view && *base == v => Verdict::Wrote,
        OpKind::SetDiscriminant { base, .. } if !is_view && *base == v => Verdict::Wrote,
        OpKind::Discriminant(base) if !is_view && *base == v => Verdict::Fine,
        OpKind::Deref(ptr) if is_view && *ptr == v => Verdict::Object,
        OpKind::StoreThrough { ptr, .. } if is_view && *ptr == v => Verdict::Wrote,
        // `lower/ssa.rs` writes `(*self).field = x` as a mutable view of the parameter and a
        // store through it. Its path starts with the `Deref` when the root is a pointer.
        OpKind::BorrowView {
            root: Root::Value(root),
            path,
            ..
        } if *root == v && view_path(path, is_view) => Verdict::View,
        OpKind::Call {
            callee: Callee::Direct { item, .. },
            args,
            host: None,
            ..
        } => {
            let positions: Vec<usize> = args
                .iter()
                .enumerate()
                .filter(|(_, arg)| **arg == v)
                .map(|(position, _)| position)
                .collect();
            match positions.is_empty() {
                false => Verdict::Forward(*item, positions),
                // The value is the callee operand rather than an argument.
                true => Verdict::Escape,
            }
        }
        _ => Verdict::Escape,
    }
}

/// A place walk that consumes a slice through its data, never its materialized fat-pointer
/// identity. Rust begins these paths with `Deref`; the remainder stays within an element or a
/// subslice.
fn slice_path(path: &[Proj]) -> bool {
    matches!(path.first(), Some(Proj::Deref))
        && path[1..].iter().all(|proj| {
            matches!(
                proj,
                Proj::Index(_)
                    | Proj::ConstantIndex { .. }
                    | Proj::Subslice { .. }
                    | Proj::Field { .. }
                    | Proj::Downcast { .. }
            )
        })
}

/// Whether a place path through a `&mut Enum` stays within a scalar field projection behind `Deref`.
fn enum_subfield_path(path: &[Proj]) -> bool {
    matches!(path.first(), Some(Proj::Deref))
        && path.len() >= 2
        && path[1..]
            .iter()
            .all(|proj| matches!(proj, Proj::Field { .. } | Proj::Downcast { .. }))
        && path[1..]
            .iter()
            .any(|proj| matches!(proj, Proj::Field { .. }))
}

/// Whether a place path stays inside the aggregate: fields and downcasts, after the one `Deref`
/// that a pointer root starts with. An `Index` operand is a runtime value, so an indexed path is
/// not a field access this module can name.
fn view_path(path: &[Proj], through_pointer: bool) -> bool {
    let rest = match (through_pointer, path.first()) {
        (true, Some(Proj::Deref)) => &path[1..],
        (true, _) => return false,
        (false, _) => path,
    };
    rest.iter()
        .all(|proj| matches!(proj, Proj::Field { .. } | Proj::Downcast { .. }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::interns::StrId;
    use crate::jir::origin::OriginId;
    use crate::jir::repr::{FuncBuilder, RTerm};
    use crate::jir::semantic::{AbiSignature, CastKind, DefinitionId, Signature, TypeId};
    use crate::jir::types::{
        EnumRepr, FieldDescriptor, FieldKey, JsFacts, LayoutFacts, NominalForm, TypeDescriptor,
        TypeKind, TypeMeta, VariantDescriptor,
    };

    const O: OriginId = OriginId(0);
    const I32: TypeId = TypeId(1);
    const REF_I32: TypeId = TypeId(2);
    const PTR_I32: TypeId = TypeId(3);
    const ENUM: TypeId = TypeId(4);
    const REF_ENUM: TypeId = TypeId(5);
    const UNIT: TypeId = TypeId(6);

    fn item(n: u64) -> ItemId {
        ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash: n,
            },
            instantiation: u128::from(n),
        }
    }

    fn sig(arguments: Vec<TypeId>, result: TypeId) -> Signature {
        Signature {
            arguments,
            result,
            abi: AbiSignature::default(),
        }
    }

    fn desc(
        id: TypeId,
        kind: TypeKind,
        indirect: bool,
        enum_repr: Option<EnumRepr>,
    ) -> TypeDescriptor {
        TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                sized: true,
                zst: id == UNIT,
                size: Some(if id == UNIT { 0 } else { 4 }),
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: indirect,
                needs_clone: indirect,
                crosses_transparent_union: false,
                transparent: None,
                peeled: id,
                unsized_tail: None,
                enum_repr,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        }
    }

    fn table() -> TypeTable {
        let mut t = TypeTable::new();
        t.insert(desc(
            I32,
            TypeKind::Integer {
                signed: true,
                bits: 32,
            },
            false,
            None,
        ));
        t.insert(desc(
            REF_I32,
            TypeKind::Reference {
                pointee: I32,
                mutable: true,
            },
            false,
            None,
        ));
        t.insert(desc(
            PTR_I32,
            TypeKind::RawPointer {
                pointee: I32,
                mutable: false,
            },
            false,
            None,
        ));
        t.insert(desc(UNIT, TypeKind::Tuple(Vec::new()), false, None));
        t.insert(desc(
            ENUM,
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
                        name: StrId(0),
                        fields: vec![FieldDescriptor {
                            key: FieldKey::Index(0),
                            ty: I32,
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
            true,
            Some(EnumRepr::Tag { fieldless: false }),
        ));
        t.insert(desc(
            REF_ENUM,
            TypeKind::Reference {
                pointee: ENUM,
                mutable: true,
            },
            false,
            None,
        ));
        t
    }

    #[test]
    fn by_ref_scalar_allows_deref_store_reborrow_and_rejects_raw_cast() {
        let t = table();
        let mut b = FuncBuilder::new(item(1), sig(vec![REF_I32], UNIT));
        let p = b.param(REF_I32, O);
        let reborrow = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(p),
                path: vec![Proj::Deref],
                mutable: true,
            },
            REF_I32,
            O,
        );
        let v = b.op(FuncBuilder::ENTRY, OpKind::Deref(reborrow), I32, O);
        b.effect(FuncBuilder::ENTRY, OpKind::StoreThrough { ptr: p, v }, O);
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let summary = summarize(&b.finish(), &t, &[Some(ParamKind::ByRefScalar)]);
        assert!(!summary[0].escapes);
        assert!(summary[0].written);
        assert!(summary[0].mutated);

        let mut escaping = FuncBuilder::new(item(2), sig(vec![REF_I32], UNIT));
        let p2 = escaping.param(REF_I32, O);
        let _cast = escaping.op(
            FuncBuilder::ENTRY,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: p2,
                from: REF_I32,
                to: PTR_I32,
            },
            PTR_I32,
            O,
        );
        escaping.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let summary2 = summarize(&escaping.finish(), &t, &[Some(ParamKind::ByRefScalar)]);
        assert!(summary2[0].escapes);
    }

    #[test]
    fn by_ref_enum_allows_deref_discr_store_and_rejects_escaping_subfield() {
        let t = table();
        let mut b = FuncBuilder::new(item(3), sig(vec![REF_ENUM], UNIT));
        let p = b.param(REF_ENUM, O);
        let cur = b.op(FuncBuilder::ENTRY, OpKind::Deref(p), ENUM, O);
        let _d = b.op(FuncBuilder::ENTRY, OpKind::Discriminant(cur), I32, O);
        let f = b.op(
            FuncBuilder::ENTRY,
            OpKind::Field {
                base: cur,
                variant: Some(1),
                field: 0,
            },
            I32,
            O,
        );
        let agg = b.op(
            FuncBuilder::ENTRY,
            OpKind::Aggregate {
                ty: ENUM,
                variant: 1,
                active_field: None,
                fields: vec![f],
            },
            ENUM,
            O,
        );
        b.effect(
            FuncBuilder::ENTRY,
            OpKind::StoreThrough { ptr: p, v: agg },
            O,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let summary = summarize(&b.finish(), &t, &[Some(ParamKind::ByRefEnum)]);
        assert!(!summary[0].escapes);
        assert!(summary[0].written);

        // Passing a subfield borrow `&mut (*p).field` to another call must escape!
        let mut escaping = FuncBuilder::new(item(4), sig(vec![REF_ENUM], UNIT));
        let p2 = escaping.param(REF_ENUM, O);
        let sub = escaping.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(p2),
                path: vec![
                    Proj::Deref,
                    Proj::Downcast { variant: 1 },
                    Proj::Field { field: 0, ty: I32 },
                ],
                mutable: true,
            },
            REF_I32,
            O,
        );
        escaping.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: crate::jir::semantic::Callee::Direct {
                    item: item(9),
                    link_name: StrId(0),
                },
                function: None,
                args: vec![sub],
                track_caller: None,
                semantic: crate::jir::semantic::CallSemantic::Ordinary,
                host: None,
            },
            O,
        );
        escaping.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let summary2 = summarize(&escaping.finish(), &t, &[Some(ParamKind::ByRefEnum)]);
        assert!(summary2[0].escapes);
    }
}
