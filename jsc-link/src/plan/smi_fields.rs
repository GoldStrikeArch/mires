//! Link-time signed-`Int32` Smi `u32` struct-field scanning and selection.
//!
//! Identifies internal multi-field structs whose plain `u32` fields store high-bit constants
//! (`>= 0x8000_0000`, e.g. `u32::MAX` sentinel indices) and never escape through references,
//! raw pointers, statics, non-trivial transmutes/casts, or JS/host boundaries. Storing those
//! fields as signed `Int32` (`-2^31..=2^31-1`) keeps them as V8 Smis instead of boxing `u32::MAX`
//! into a `HeapNumber` `Double` field.

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::semantic::{
    Call, Callee, CastKind, Constant, Local, Operand, OperandKind, Place, Projection, TypeId,
};
use rcgjs_object::jir::types::{
    EnumRepr, FieldKey, NominalAbi, NominalForm, PinnedBoundary, RepresentationCandidate,
    TypeDescriptor, TypeKind, TypeTable,
};

use super::enum_repr::{lookup_desc, peel_global, walk_type_id};

/// Mutable scan state accumulated across live MIR items for `smi_u32_fields` selection.
#[derive(Default)]
pub(crate) struct SmiScanState {
    bad_structs: BTreeSet<TypeId>,
    bad_fields: BTreeSet<(TypeId, u32)>,
    high_bit_structs: BTreeSet<TypeId>,
    visited_bad: BTreeSet<TypeId>,
}

impl SmiScanState {
    /// Marks every struct reachable from a `Static` item's type as ineligible for Smi storage.
    pub(crate) fn record_static(
        &mut self,
        ty: TypeId,
        interns: &Interns,
        global_types: &BTreeMap<TypeId, TypeDescriptor>,
    ) {
        walk_type_id(
            ty,
            interns,
            global_types,
            &mut self.bad_structs,
            &mut self.visited_bad,
        );
    }

    /// Disqualifies structs participating in non-trivial `Transmute` or `PtrToPtr` casts.
    pub(crate) fn record_cast(
        &mut self,
        kind: CastKind,
        op: &Operand,
        result: TypeId,
        local: &TypeTable,
        interns: &Interns,
        global_types: &BTreeMap<TypeId, TypeDescriptor>,
    ) {
        let bad_cast = match kind {
            CastKind::Transmute => {
                peel_global(op.ty, global_types) != peel_global(result, global_types)
                    && !is_harmless_ptr_to_ptr(op.ty, result, local, global_types)
                    && !is_ptr_addr_transmute(op.ty, result, local, global_types)
            }
            CastKind::PtrToPtr => !is_harmless_ptr_to_ptr(op.ty, result, local, global_types),
            _ => false,
        };
        if bad_cast {
            walk_type_id(
                op.ty,
                interns,
                global_types,
                &mut self.bad_structs,
                &mut self.visited_bad,
            );
            walk_type_id(
                result,
                interns,
                global_types,
                &mut self.bad_structs,
                &mut self.visited_bad,
            );
        }
    }

    /// Records structs whose field is assigned a high-bit `u32` constant (`>= 0x8000_0000`).
    pub(crate) fn record_use_assign(
        &mut self,
        place: &Place,
        op: &Operand,
        locals: &[Local],
        local: &TypeTable,
        global_types: &BTreeMap<TypeId, TypeDescriptor>,
    ) {
        if is_high_bit_u32_const(op, local, global_types) {
            walk_place_struct_fields(place, locals, local, global_types, |struct_ty, _| {
                self.high_bit_structs.insert(struct_ty);
            });
        }
    }

    /// Records an aggregate construction containing a high-bit `u32` constant operand.
    pub(crate) fn record_aggregate_operand(
        &mut self,
        result: TypeId,
        op: &Operand,
        local: &TypeTable,
        global_types: &BTreeMap<TypeId, TypeDescriptor>,
    ) {
        if is_high_bit_u32_const(op, local, global_types) {
            self.high_bit_structs
                .insert(peel_global(result, global_types));
        }
    }

    /// Marks any struct field whose address is taken via reference/raw-pointer/reborrow.
    pub(crate) fn record_ref_place(
        &mut self,
        place: &Place,
        locals: &[Local],
        local: &TypeTable,
        global_types: &BTreeMap<TypeId, TypeDescriptor>,
    ) {
        walk_place_struct_fields(place, locals, local, global_types, |struct_ty, field| {
            self.bad_fields.insert((struct_ty, field));
        });
    }

    /// Disqualifies structs crossing a non-trivial `transmute` / `transmute_unchecked` intrinsic.
    pub(crate) fn record_call(
        &mut self,
        call: &Call,
        local: &TypeTable,
        interns: &Interns,
        global_types: &BTreeMap<TypeId, TypeDescriptor>,
    ) {
        let is_smi_raw_call = matches!(
            &call.callee,
            Callee::Intrinsic { name, .. }
                if matches!(
                    interns.str(*name).unwrap_or(""),
                    "transmute" | "transmute_unchecked"
                )
        ) && !(call.arguments.len() == 1
            && (peel_global(call.arguments[0].ty, global_types)
                == peel_global(call.destination.ty, global_types)
                || is_harmless_ptr_to_ptr(
                    call.arguments[0].ty,
                    call.destination.ty,
                    local,
                    global_types,
                )
                || is_ptr_addr_transmute(
                    call.arguments[0].ty,
                    call.destination.ty,
                    local,
                    global_types,
                )));
        if is_smi_raw_call {
            for arg in &call.arguments {
                walk_type_id(
                    arg.ty,
                    interns,
                    global_types,
                    &mut self.bad_structs,
                    &mut self.visited_bad,
                );
            }
            walk_type_id(
                call.destination.ty,
                interns,
                global_types,
                &mut self.bad_structs,
                &mut self.visited_bad,
            );
        }
    }

    /// Selects internal multi-field struct `u32` fields eligible for signed-`Int32` Smi storage.
    pub(crate) fn select(
        &self,
        global_types: &BTreeMap<TypeId, TypeDescriptor>,
        excluded_enums: &BTreeSet<TypeId>,
    ) -> BTreeSet<(TypeId, u32)> {
        let mut out = BTreeSet::new();
        for (&id, desc) in global_types {
            if desc.js.peeled != id
                || !self.high_bit_structs.contains(&id)
                || excluded_enums.contains(&id)
                || self.bad_structs.contains(&id)
                || desc.meta.abi == NominalAbi::C
                || desc.meta.boundary != PinnedBoundary::None
                || desc.meta.candidate != RepresentationCandidate::None
                || desc.js.transparent.is_some()
                || desc.js.crosses_transparent_union
                || desc.js.unsized_tail.is_some()
            {
                continue;
            }
            let TypeKind::Adt {
                form: NominalForm::Struct,
                variants,
                ..
            } = &desc.kind
            else {
                continue;
            };
            if variants.len() != 1 || variants[0].fields.len() < 2 {
                continue;
            }
            for (fi, field) in variants[0].fields.iter().enumerate() {
                if matches!(field.key, FieldKey::Transparent) {
                    continue;
                }
                let Some(fdesc) = global_types.get(&field.ty) else {
                    continue;
                };
                if matches!(
                    fdesc.kind,
                    TypeKind::Integer {
                        signed: false,
                        bits: 32
                    }
                ) && !self.bad_fields.contains(&(id, fi as u32))
                {
                    out.insert((id, fi as u32));
                }
            }
        }
        out
    }
}

fn is_ptr_addr_transmute(
    from_ty: TypeId,
    to_ty: TypeId,
    local_table: &TypeTable,
    global: &BTreeMap<TypeId, TypeDescriptor>,
) -> bool {
    let is_ptr = |ty: TypeId| -> bool {
        matches!(
            lookup_desc(peel_global(ty, global), local_table, global).map(|d| &d.kind),
            Some(TypeKind::Reference { .. } | TypeKind::RawPointer { .. })
        )
    };
    let is_addr_scalar = |ty: TypeId| -> bool {
        let Some(d) = lookup_desc(peel_global(ty, global), local_table, global) else {
            return false;
        };
        matches!(d.kind, TypeKind::Integer { .. })
            || matches!(d.js.enum_repr, Some(EnumRepr::Number { .. }))
    };
    (is_ptr(from_ty) && is_addr_scalar(to_ty)) || (is_addr_scalar(from_ty) && is_ptr(to_ty))
}

fn is_harmless_ptr_to_ptr(
    from_ty: TypeId,
    to_ty: TypeId,
    local_table: &TypeTable,
    global: &BTreeMap<TypeId, TypeDescriptor>,
) -> bool {
    let pointee = |ty: TypeId| -> Option<TypeId> {
        let peeled = peel_global(ty, global);
        match lookup_desc(peeled, local_table, global).map(|d| &d.kind)? {
            TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
                Some(peel_global(*pointee, global))
            }
            _ => None,
        }
    };
    let (Some(a), Some(b)) = (pointee(from_ty), pointee(to_ty)) else {
        return false;
    };
    if a == b {
        return true;
    }
    let is_erased_elem = |ty: TypeId| -> bool {
        match lookup_desc(ty, local_table, global) {
            Some(d) if d.layout.zst => true,
            Some(d) => matches!(d.kind, TypeKind::Integer { bits: 8, .. }),
            None => false,
        }
    };
    let elem_of = |ty: TypeId| -> Option<TypeId> {
        match lookup_desc(ty, local_table, global).map(|d| &d.kind)? {
            TypeKind::Slice(e) | TypeKind::Array { element: e, .. } => {
                Some(peel_global(*e, global))
            }
            _ => None,
        }
    };
    let is_erased =
        |ty: TypeId| -> bool { is_erased_elem(ty) || elem_of(ty).is_some_and(is_erased_elem) };
    if is_erased(a) || is_erased(b) {
        return true;
    }
    let ea = elem_of(a);
    let eb = elem_of(b);
    ea == Some(b) || eb == Some(a) || (ea.is_some() && ea == eb)
}

fn is_high_bit_u32_const(
    op: &Operand,
    local_table: &TypeTable,
    global: &BTreeMap<TypeId, TypeDescriptor>,
) -> bool {
    let OperandKind::Constant(Constant::Scalar { bits, .. }) = &op.kind else {
        return false;
    };
    if *bits < 0x8000_0000 || *bits > u128::from(u32::MAX) {
        return false;
    }
    let peeled = peel_global(op.ty, global);
    matches!(
        lookup_desc(peeled, local_table, global).map(|d| &d.kind),
        Some(TypeKind::Integer {
            signed: false,
            bits: 32
        })
    )
}

/// Walks every non-downcast `Projection::Field` along `place`, invoking `on_field(peeled_struct, field_idx)`.
fn walk_place_struct_fields(
    place: &Place,
    locals: &[Local],
    local_table: &TypeTable,
    global: &BTreeMap<TypeId, TypeDescriptor>,
    mut on_field: impl FnMut(TypeId, u32),
) {
    let Some(mut cur_ty) = locals.get(place.local.0 as usize).map(|l| l.ty) else {
        return;
    };
    let mut downcast = false;
    for proj in &place.projection {
        match proj {
            Projection::Deref => {
                downcast = false;
                let peeled = peel_global(cur_ty, global);
                let Some(desc) = lookup_desc(peeled, local_table, global) else {
                    return;
                };
                match desc.kind {
                    TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
                        cur_ty = pointee;
                    }
                    _ => return,
                }
            }
            Projection::Field { field, ty } => {
                if !downcast {
                    on_field(peel_global(cur_ty, global), *field);
                }
                downcast = false;
                cur_ty = *ty;
            }
            Projection::Index(_) | Projection::ConstantIndex { .. } => {
                downcast = false;
                let peeled = peel_global(cur_ty, global);
                let Some(desc) = lookup_desc(peeled, local_table, global) else {
                    return;
                };
                match desc.kind {
                    TypeKind::Array { element, .. } | TypeKind::Slice(element) => {
                        cur_ty = element;
                    }
                    _ => return,
                }
            }
            Projection::Subslice { .. } => {
                downcast = false;
            }
            Projection::Downcast { .. } => {
                downcast = true;
            }
            Projection::OpaqueCast(ty) | Projection::UnwrapUnsafeBinder(ty) => {
                downcast = false;
                cur_ty = *ty;
            }
        }
    }
}
