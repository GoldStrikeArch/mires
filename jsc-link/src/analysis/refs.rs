//! The non-call reference scan: every item a program mentions other than as the callee of a
//! plain direct call.
//!
//! Two plans read the same facts and must not disagree about them. The flat direct-call ABI
//! (`crate::plan::abi_flat`) refuses to change the calling convention of anything mentioned this
//! way, and the representation planner (`crate::analysis::valueflow`) pins the parameters of
//! anything whose callers it cannot see. The scan itself is one walk over the ops and the `refs`
//! of the unlowered items; the buckets differ only in how much the consumer wants to know:
//! `named` is the whole set, and the reified fn-pointer targets and drop-glue items are also
//! recorded on their own so the value-flow analysis can model those references as dispatch
//! groups and drop edges instead of pins.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{JsName, LinkItem};
use crate::jir::decline::Decline;
use crate::jir::repr::{Func, Op, OpKind};
use crate::jir::semantic::{
    Callee, ConstTree, DropGlue, GlobalReference, HostPayload, ItemId, TypeId,
};
use crate::jir::types::{TypeKind, TypeTable};

/// What one pass over the program's ops mentioned, by how it was mentioned.
#[derive(Clone, Debug, Default)]
pub struct RefScan {
    /// Every item named other than as the callee of a plain direct call: function constants,
    /// reify and closure coercions, intrinsic fallbacks, host marker arguments and helpers.
    pub named: BTreeSet<ItemId>,
    /// The items a `ReifyFnPointer`/`ClosureFnPointer` coercion makes callable through a
    /// function pointer, keyed by the fn-pointer type coerced to. A subset of `named`; the
    /// value-flow analysis turns each key into one indirect-dispatch group.
    pub fn_pointer_targets: BTreeMap<TypeId, BTreeSet<ItemId>>,
    /// The drop glue a `Drop` op names directly. A subset of what `ineligible` returns; the
    /// value-flow analysis models these references as drop edges rather than pins.
    pub dropped: BTreeSet<ItemId>,
}

impl RefScan {
    /// Everything mentioned other than as a plain direct call, whatever the mention was: the set
    /// the flat direct-call ABI disqualifies.
    pub fn ineligible(&self) -> BTreeSet<ItemId> {
        let mut all = self.named.clone();
        all.extend(self.dropped.iter().copied());
        for targets in self.fn_pointer_targets.values() {
            all.extend(targets.iter().copied());
        }
        all
    }
}

/// The items named by the `refs` of everything the lane did not lower, whose baseline text may
/// mention them any way at all.
pub fn named_by_unlowered(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    by_name: &BTreeMap<&str, ItemId>,
) -> BTreeSet<ItemId> {
    let mut named = BTreeSet::new();
    for (name, item) in table {
        if funcs.get(name).is_some_and(Result::is_ok) {
            continue;
        }
        for referenced in &item.refs {
            if let Some(id) = by_name.get(referenced.as_str()) {
                named.insert(*id);
            }
        }
    }
    named
}

/// Adds every item `op` names other than as the callee of a plain direct call.
pub fn scan_op(func: &Func, op: &Op, table: Option<&TypeTable>, scan: &mut RefScan) {
    match &op.kind {
        // A function constant: `foo` as a value, which the body may do anything with.
        OpKind::Const(tree) => scan_tree(tree, &mut scan.named),
        // `ReifyFnPointer`/`ClosureFnPointer`: the item is named by the *source type*, resolved in
        // the type table (`lower/js/rvalue.rs`).
        OpKind::Cast { from, to, .. } => {
            scan_type(*from, table, &mut scan.named);
            record_reify(*from, *to, table, scan);
        }
        OpKind::Intrinsic {
            fallback, types, ..
        } => {
            if let Some(fallback) = fallback {
                scan.named.insert(fallback.item);
            }
            for ty in types {
                scan_type(*ty, table, &mut scan.named);
            }
        }
        // A host marker resolves its arguments' closure and function-item types to bodies
        // (`lower/js/host.rs::closure_item`), and its helpers by name (`lower/js/template.rs`).
        OpKind::Call {
            callee,
            args,
            host: Some(payload),
            ..
        } => {
            if let Callee::Direct { item, .. } = callee {
                scan.named.insert(*item);
            }
            for (_, helper) in payload.helpers() {
                scan.named.insert(helper.item);
            }
            if let HostPayload::KeyedList(payload) = payload {
                for callable in [&payload.collection, &payload.key, &payload.row] {
                    if let Some(body) = callable.body {
                        scan.named.insert(body.item);
                    }
                    if let DropGlue::Direct { item, .. } = callable.cleanup {
                        scan.dropped.insert(item);
                    }
                }
            }
            for arg in args {
                scan_closure_type(func.values[arg.0 as usize].ty, table, &mut scan.named);
            }
        }
        OpKind::Drop {
            glue: DropGlue::Direct { item, .. },
            ..
        } => {
            scan.dropped.insert(*item);
        }
        _ => {}
    }
}

/// Adds the function a constant tree names, at any depth.
pub fn scan_tree(tree: &ConstTree, named: &mut BTreeSet<ItemId>) {
    match tree {
        ConstTree::Global {
            target: GlobalReference::Function { item, .. },
            ..
        } => {
            named.insert(*item);
        }
        ConstTree::Aggregate(fields)
        | ConstTree::Slice(fields)
        | ConstTree::Variant { fields, .. } => {
            for field in fields {
                scan_tree(field, named);
            }
        }
        ConstTree::Reference(inner) => scan_tree(inner, named),
        _ => {}
    }
}

/// Records a coercion to a fn-pointer type as a dispatch-group membership: the item the source
/// `FnDef` or closure type resolves to becomes callable at the target type.
fn record_reify(from: TypeId, to: TypeId, table: Option<&TypeTable>, scan: &mut RefScan) {
    let Some(table) = table else { return };
    let to = table.peeled(to);
    if !matches!(
        table.get(to).map(|descriptor| &descriptor.kind),
        Some(TypeKind::FnPtr { .. })
    ) {
        return;
    }
    let target = match table
        .get(table.peeled(from))
        .map(|descriptor| &descriptor.kind)
    {
        Some(TypeKind::FnDef {
            instance: Some(instance),
            ..
        }) => instance.item,
        Some(TypeKind::Closure {
            fn_pointer_shim: Some(shim),
            ..
        }) => shim.item,
        _ => return,
    };
    scan.fn_pointer_targets
        .entry(to)
        .or_default()
        .insert(target);
}

/// Adds the **body** a closure or function-item value calls, which is how `lower/js/host.rs`
/// names the thunk a marker is handed: the definition of the type with the peeled type's own hash
/// as the instantiation.
fn scan_closure_type(ty: TypeId, table: Option<&TypeTable>, named: &mut BTreeSet<ItemId>) {
    let Some(table) = table else { return };
    let peeled = table.peeled(ty);
    let definition = match table.get(peeled).map(|descriptor| &descriptor.kind) {
        Some(TypeKind::Closure { def, .. } | TypeKind::FnDef { def, .. }) => *def,
        _ => return,
    };
    named.insert(ItemId {
        definition,
        instantiation: peeled.0,
    });
    scan_type(ty, Some(table), named);
}

/// Adds the item a `FnDef` or closure type resolves to.
fn scan_type(ty: TypeId, table: Option<&TypeTable>, named: &mut BTreeSet<ItemId>) {
    let Some(table) = table else { return };
    let Some(descriptor) = table.get(table.peeled(ty)) else {
        return;
    };
    match &descriptor.kind {
        TypeKind::FnDef {
            instance: Some(instance),
            ..
        } => {
            named.insert(instance.item);
        }
        TypeKind::Closure {
            fn_pointer_shim: Some(shim),
            ..
        } => {
            named.insert(shim.item);
        }
        _ => {}
    }
}
