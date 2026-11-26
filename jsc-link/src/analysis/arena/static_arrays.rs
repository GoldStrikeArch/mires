//! Part 1: Immutable static unsigned scalar array element bounds and exact lengths.

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::jir::semantic::{BodyKind, ConstTree};
use rcgjs_object::jir::types::{TypeId, TypeKind, TypeTable};
use rcgjs_object::object::ItemKind;

use crate::item::{JsName, LinkItem};
use crate::lower::js::value;

use super::{mask_bits, width_max, ProgramBoundsFacts};

pub(super) fn collect_static_array_bounds(
    table: &BTreeMap<JsName, LinkItem>,
    live: &BTreeSet<JsName>,
    facts: &mut ProgramBoundsFacts,
) {
    for name in live {
        let Some(item) = table.get(name) else {
            continue;
        };
        if item.kind != ItemKind::Static {
            continue;
        }
        let (Some(body), Some(interns)) = (item.jir.body.as_ref(), item.jir.interns()) else {
            continue;
        };
        let BodyKind::Static {
            init,
            mutable: false,
        } = &body.kind
        else {
            continue;
        };
        let Some((len, bounds)) =
            extract_static_array_info(&interns.types, body.signature.result, init)
        else {
            continue;
        };
        facts.static_array_lens.insert(body.item.definition, len);
        if let Some(interval) = bounds {
            facts
                .static_array_bounds
                .insert(body.item.definition, interval);
        }
    }
}

fn extract_static_array_info(
    types: &TypeTable,
    result_ty: TypeId,
    init: &ConstTree,
) -> Option<(u128, Option<(u128, u128)>)> {
    let mut ty = types.peeled(result_ty);
    let mut tree = init;
    while let ConstTree::Reference(inner) = tree {
        tree = inner.as_ref();
        if let Some(pt) = value::pointee_of(types, ty) {
            ty = types.peeled(pt);
        }
    }
    let elem_ty = match types.get(ty).map(|d| &d.kind) {
        Some(TypeKind::Array { element, .. } | TypeKind::Slice(element)) => *element,
        _ => return None,
    };
    let items = match tree {
        ConstTree::Aggregate(items) | ConstTree::Slice(items) => items,
        _ => return None,
    };
    let len = items.len() as u128;
    let bounds = scalar_items_bounds(types, elem_ty, items);
    Some((len, bounds))
}

fn scalar_items_bounds(
    types: &TypeTable,
    elem_ty: TypeId,
    items: &[ConstTree],
) -> Option<(u128, u128)> {
    let (false, bits) = types.int_info(types.peeled(elem_ty))? else {
        return None;
    };
    if items.is_empty() {
        return None;
    }
    let max_val = width_max(bits);
    let mut lo = u128::MAX;
    let mut hi = 0u128;
    for elem in items {
        let ConstTree::Scalar { bits: raw, bytes } = elem else {
            return None;
        };
        let v = mask_bits(*raw, *bytes).min(max_val);
        lo = lo.min(v);
        hi = hi.max(v);
    }
    Some((lo, hi))
}
