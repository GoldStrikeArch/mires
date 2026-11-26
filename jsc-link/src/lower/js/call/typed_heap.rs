//! Phase 3 Tier A typed-heap `Global` allocation/deallocation hooks
//! (`StandardOperation::is_typed_heap`).

use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::repr::{Op, ValueId};
use rcgjs_object::jir::semantic::StandardOperation;
use rcgjs_object::jir::types::{NominalForm, RepresentationCandidate, TypeId, TypeKind, TypeTable};
use rcgjs_object::jsast::{self, Expr};
use rcgjs_object::ptr;

use crate::analysis::heap::HeapPlan;

use super::super::ast_util::TA_CTORS;
use super::super::ptr::sequenced;
use super::super::{soa, value, JsCx, Res};
use super::Lowered;

impl JsCx<'_> {
    /// Lowers the five Phase 3 Tier A typed-heap `Global` hooks (`StandardOperation::is_typed_heap`)
    /// directly at call sites when `self.heap().typed_heap_enabled` holds.
    pub(super) fn lower_typed_heap_call(
        &self,
        op: &Op,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let table = self.table;
        let heap = self.heap();

        let result_pointee = || -> Option<TypeId> {
            let res = op.result?;
            let ty = self.ty_of(res).ok()?;
            value::pointee_of(table, ty)
        };

        match operation {
            StandardOperation::BoxNew => {
                let [val] = args else {
                    return Ok(None);
                };
                let Some(elem) = result_pointee() else {
                    return Ok(None);
                };
                let peeled = table.peeled(elem);
                let expr = self.expr_of(*val)?;
                let tracked =
                    heap.needs_box_tracking(table, elem) && value::is_slot_pointee(table, peeled);
                let slot = if tracked && table.is_indirect(peeled) {
                    jsast::rt_call("t_box_new_tracked", vec![expr])
                } else {
                    ptr::slot(jsast::array(vec![expr]), jsast::num(0))
                };
                Ok(Some(Lowered::value(slot)))
            }
            StandardOperation::TypedAlloc => {
                let Some(elem) = result_pointee() else {
                    return Ok(None);
                };
                self.lower_typed_alloc(elem, args)
            }
            StandardOperation::TypedRealloc => {
                let Some(elem) = result_pointee() else {
                    return Ok(None);
                };
                self.lower_typed_realloc(elem, args)
            }
            StandardOperation::TypedDealloc => {
                let Some(&ptr_v) = args.first() else {
                    return Ok(None);
                };
                if !heap.non_rc_raw_alloc_reachable {
                    return Ok(Some(Lowered::Done(Vec::new())));
                }
                let p = self.expr_of(ptr_v)?;
                Ok(Some(Lowered::Done(vec![jsast::expr_stmt(jsast::rt_call(
                    "t_dealloc_maybe",
                    vec![p],
                ))])))
            }
            StandardOperation::TypedDeallocRaw => {
                let Some(&ptr_v) = args.first() else {
                    return Ok(None);
                };
                if !heap.raw_alloc_reachable {
                    return Ok(Some(Lowered::Done(Vec::new())));
                }
                let p = self.expr_of(ptr_v)?;
                Ok(Some(Lowered::Done(vec![jsast::expr_stmt(jsast::rt_call(
                    "t_dealloc_maybe",
                    vec![p],
                ))])))
            }
            StandardOperation::RcAllocSlice | StandardOperation::ArcAllocSlice => {
                let Some(rc_inner_ty) = result_pointee() else {
                    return Ok(None);
                };
                self.lower_rc_alloc_slice(rc_inner_ty, args)
            }
            _ => Ok(None),
        }
    }

    fn lower_typed_alloc(&self, elem: TypeId, args: &[ValueId]) -> Res<Option<Lowered>> {
        let [cap_v, zeroed_v] = args else {
            return Ok(None);
        };
        let table = self.table;
        let heap = self.heap();
        let peeled = table.peeled(elem);
        let cap = self.expr_of(*cap_v)?;
        if let Some(k) = table
            .get(elem)
            .and_then(crate::lower::semantic_ops::typed_array_kind)
        {
            let buf = jsast::new(jsast::id(TA_CTORS[k as usize]), vec![cap]);
            return Ok(Some(Lowered::value(ptr::slot(buf, jsast::num(0)))));
        }
        let tracked = heap.needs_box_tracking(table, elem) && value::is_slot_pointee(table, peeled);
        let needs_shell = table.is_indirect(peeled)
            || matches!(value::kind(table, peeled), Some(TypeKind::Array { .. }));
        if self.const_scalar_u128(*zeroed_v) == Some(0) {
            if !needs_shell || self.const_scalar_u128(*cap_v).is_some_and(|n| n != 1) {
                let buf = jsast::new(jsast::id("Array"), vec![cap]);
                return Ok(Some(Lowered::value(ptr::slot(buf, jsast::num(0)))));
            }
            let shell_expr = uninit_shell(table, self.interns, elem, 0, heap, self.soa);
            if self.const_scalar_u128(*cap_v) == Some(1) {
                if let Some(shell) = shell_expr {
                    let slot = if tracked {
                        jsast::rt_call("t_box_new_tracked", vec![shell])
                    } else {
                        ptr::slot(jsast::array(vec![shell]), jsast::num(0))
                    };
                    return Ok(Some(Lowered::value(slot)));
                }
                let buf = jsast::new(jsast::id("Array"), vec![jsast::num(1)]);
                return Ok(Some(Lowered::value(ptr::slot(buf, jsast::num(0)))));
            }
            if let Some(shell) = shell_expr {
                let mut call_args = vec![cap, jsast::arrow(Vec::new(), shell)];
                if tracked {
                    call_args.push(Expr::Bool(true));
                }
                return Ok(Some(Lowered::value(jsast::rt_call(
                    "t_alloc_arr",
                    call_args,
                ))));
            }
            let buf = jsast::new(jsast::id("Array"), vec![cap]);
            return Ok(Some(Lowered::value(ptr::slot(buf, jsast::num(0)))));
        }
        let zeroed = self.expr_of(*zeroed_v)?;
        let mk = match value::zero_value_for_heap_plan(table, self.interns, elem, heap) {
            Some(v) => jsast::arrow(Vec::new(), v),
            None => jsast::null(),
        };
        let mut call_args = vec![cap, zeroed, mk];
        if tracked {
            call_args.push(Expr::Bool(true));
        }
        Ok(Some(Lowered::value(jsast::rt_call("t_alloc_z", call_args))))
    }

    fn lower_typed_realloc(&self, elem: TypeId, args: &[ValueId]) -> Res<Option<Lowered>> {
        let [ptr_v, old_cap_v, new_cap_v] = args else {
            return Ok(None);
        };
        let table = self.table;
        let heap = self.heap();
        let peeled = table.peeled(elem);
        let p = self.expr_of(*ptr_v)?;
        let old_cap = self.expr_of(*old_cap_v)?;
        let new_cap = self.expr_of(*new_cap_v)?;
        let checked = heap.non_rc_raw_alloc_reachable;
        if let Some(k) = table
            .get(elem)
            .and_then(crate::lower::semantic_ops::typed_array_kind)
        {
            let helper = if checked {
                "t_realloc_ta_checked"
            } else {
                "t_realloc_ta"
            };
            return Ok(Some(Lowered::value(jsast::rt_call(
                helper,
                vec![p, old_cap, new_cap, jsast::num(f64::from(k))],
            ))));
        }
        if !checked {
            Ok(Some(Lowered::value(jsast::rt_call(
                "t_realloc_arr",
                vec![p, new_cap],
            ))))
        } else {
            let es = table
                .get(peeled)
                .and_then(|d| d.layout.size)
                .unwrap_or(1)
                .max(1);
            Ok(Some(Lowered::value(jsast::rt_call(
                "t_realloc_arr_checked",
                vec![p, old_cap, new_cap, jsast::num(es as f64)],
            ))))
        }
    }

    fn lower_rc_alloc_slice(&self, rc_inner_ty: TypeId, args: &[ValueId]) -> Res<Option<Lowered>> {
        let [len_v] = args else {
            return Ok(None);
        };
        let table = self.table;
        let heap = self.heap();
        let Some(tail_info) = table.unsized_tail(rc_inner_ty) else {
            return Ok(None);
        };
        let tail_idx = tail_info.field as usize;
        let Some(TypeKind::Adt { variants, .. }) = value::kind(table, rc_inner_ty) else {
            return Ok(None);
        };
        let [variant] = variants.as_slice() else {
            return Ok(None);
        };
        let Some(tail_field) = variant.fields.get(tail_idx) else {
            return Ok(None);
        };
        let Some(TypeKind::Slice(elem_ty)) = value::kind(table, tail_field.ty) else {
            return Ok(None);
        };
        let elem_peeled = table.peeled(*elem_ty);
        let len_expr = self.expr_of(*len_v)?;
        let (setup, len_ref) = self.reread(len_expr, &[]);
        let tail_buf = if table.is_zst(elem_peeled) {
            let align = table
                .get(*elem_ty)
                .and_then(|d| d.layout.align)
                .unwrap_or(1)
                .max(1);
            jsast::num(align as f64)
        } else if let Some(k) = table
            .get(*elem_ty)
            .and_then(crate::lower::semantic_ops::typed_array_kind)
        {
            jsast::new(jsast::id(TA_CTORS[k as usize]), vec![len_ref.clone()])
        } else {
            jsast::new(jsast::id("Array"), vec![len_ref.clone()])
        };

        let mut fields: Vec<(String, Expr)> = Vec::with_capacity(variant.fields.len());
        for (idx, field) in variant.fields.iter().enumerate() {
            let Some(key_name) = value::key_name(self.interns, field.key) else {
                return Ok(None);
            };
            if idx == tail_idx {
                fields.push((key_name, tail_buf.clone()));
                continue;
            }
            let Some(init) = init_counter_one(table, self.interns, field.ty, 0) else {
                return Ok(None);
            };
            fields.push((key_name, init));
        }

        let record = Expr::Object(fields);
        let slot = if heap.needs_box_tracking(table, rc_inner_ty) {
            jsast::rt_call("t_box_new_tracked", vec![record])
        } else {
            ptr::slot(jsast::array(vec![record]), jsast::num(0))
        };
        let fat = jsast::object_of(vec![("ptr", slot), ("meta", len_ref)]);
        Ok(Some(Lowered::value(sequenced(setup, fat))))
    }
}

/// Header counter field (`Cell<usize>` peels to `usize` -> `1`;
/// `AtomicUsize { v: Atomic<usize>(UnsafeCell<usize>) }` -> `{ v: { _0: 1 } }`).
fn init_counter_one(table: &TypeTable, interns: &Interns, ty: TypeId, depth: u8) -> Option<Expr> {
    if depth > 4 {
        return None;
    }
    let peeled = table.peeled(ty);
    match value::kind(table, peeled) {
        Some(TypeKind::Integer { .. }) => Some(jsast::num(1)),
        Some(TypeKind::Adt { variants, .. }) => {
            let [v] = variants.as_slice() else {
                return None;
            };
            let mut real_fields = v.fields.iter().filter(|f| !table.is_zst(f.ty));
            let inner_f = real_fields.next()?;
            if real_fields.next().is_some() {
                return None;
            }
            let ik = value::key_name(interns, inner_f.key)?;
            let iv = init_counter_one(table, interns, inner_f.ty, depth + 1)?;
            Some(Expr::Object(vec![(ik, iv)]))
        }
        _ => None,
    }
}

/// Lightweight uninitialized container shell for `Box::<T>::new_uninit()` (`TypedAlloc` with
/// `zeroed == 0`). Provides arrays for fixed-array fields (so `BTreeMap` `LeafNode` / `vec![...]`
/// element writes have a target container) without deep zero-initializing element structs.
fn uninit_shell(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    depth: u32,
    heap: &HeapPlan,
    soa: Option<&crate::plan::soa_vec::SoaPlan>,
) -> Option<Expr> {
    if depth > 4 {
        return None;
    }
    let peeled = table.peeled(ty);
    let desc = table.get(peeled)?;
    if desc.layout.zst {
        return Some(jsast::undefined());
    }
    if heap.native_string_enabled && desc.meta.candidate == RepresentationCandidate::String {
        return Some(jsast::object_of(vec![
            ("s", jsast::string("")),
            ("cap", jsast::num(0)),
        ]));
    }
    if heap.native_vec_enabled && crate::analysis::heap::is_direct_global_vec_ty(table, peeled) {
        if let Some(plan) = soa {
            if let Some(elem_ty) = crate::analysis::heap::global_vec_elem_ty(table, peeled) {
                if let Some(layout) = plan.get(table, elem_ty) {
                    return Some(soa::empty_vec_expr(layout));
                }
            }
        }
        if let Some(empty) = value::empty_native_vec_expr_for_heap(table, peeled, heap) {
            return Some(empty);
        }
    }
    match &desc.kind {
        TypeKind::Array { element, length } => {
            if depth == 0 {
                if let Some(k) = table
                    .get(*element)
                    .and_then(crate::lower::semantic_ops::typed_array_kind)
                {
                    return Some(jsast::new(
                        jsast::id(TA_CTORS[k as usize]),
                        vec![jsast::num(*length as f64)],
                    ));
                }
            }
            if *length <= 4 {
                Some(jsast::array(
                    (0..*length).map(|_| jsast::undefined()).collect(),
                ))
            } else {
                Some(jsast::new(
                    jsast::id("Array"),
                    vec![jsast::num(*length as f64)],
                ))
            }
        }
        TypeKind::Tuple(elems) => {
            if elems.len() <= 4 {
                Some(jsast::array(
                    elems
                        .iter()
                        .map(|e| uninit_field_shell(table, interns, *e, depth + 1, heap, soa))
                        .collect(),
                ))
            } else {
                Some(jsast::new(
                    jsast::id("Array"),
                    vec![jsast::num(elems.len() as f64)],
                ))
            }
        }
        TypeKind::Adt {
            form: NominalForm::Struct,
            variants,
            ..
        } => {
            let variant = variants.first()?;
            let mut entries = Vec::with_capacity(variant.fields.len());
            for field in &variant.fields {
                let key = value::key_name(interns, field.key)?;
                entries.push((
                    key,
                    uninit_field_shell(table, interns, field.ty, depth + 1, heap, soa),
                ));
            }
            Some(jsast::object(entries))
        }
        TypeKind::Adt {
            form: NominalForm::Enum | NominalForm::Union,
            ..
        } if desc.js.is_indirect => Some(jsast::object(Vec::new())),
        _ => None,
    }
}

fn uninit_field_shell(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    depth: u32,
    heap: &HeapPlan,
    soa: Option<&crate::plan::soa_vec::SoaPlan>,
) -> Expr {
    let peeled = table.peeled(ty);
    match table.get(peeled).map(|d| &d.kind) {
        Some(
            TypeKind::Array { .. }
            | TypeKind::Tuple(_)
            | TypeKind::Adt {
                form: NominalForm::Struct,
                ..
            },
        ) => {
            uninit_shell(table, interns, peeled, depth, heap, soa).unwrap_or_else(jsast::undefined)
        }
        _ => jsast::undefined(),
    }
}
