//! Universal flat `Vec<T, Global>` call-site lowering (`StandardOperation::is_native_vec` and
//! `StandardOperation::VecResize`).

use rcgjs_object::jir::repr::{Op, OpKind, ValueId};
use rcgjs_object::jir::semantic::StandardOperation;
use rcgjs_object::jir::types::{NominalForm, PointerForm, TypeId, TypeKind};
use rcgjs_object::jsast::{self, Expr};
use rcgjs_object::ptr;

use super::super::ast_util::{TA_CTORS, TA_SUFFIXES};
use super::super::{value, JsCx, Res};
use super::Lowered;

/// Resolves the `Vec<T, Global>` target type of a `NativeVec*` (or `VecAsSlice`) call from either
/// its return type, its by-value first argument, or its `&Vec` / `&mut Vec` receiver pointee.
pub(in super::super) fn resolve_vec_call_target(
    cx: &JsCx<'_>,
    op: &Op,
    operation: StandardOperation,
    args: &[ValueId],
) -> Res<Option<TypeId>> {
    match operation {
        StandardOperation::NativeVecNew
        | StandardOperation::NativeVecWithCapacity
        | StandardOperation::NativeVecFromElemZeroed
        | StandardOperation::NativeVecFromArray
        | StandardOperation::NativeVecFromRawParts => {
            let Some(res) = op.result else {
                return Ok(None);
            };
            cx.ty_of(res).map(Some)
        }
        StandardOperation::NativeVecIntoBoxedSlice => {
            let Some(&v0) = args.first() else {
                return Ok(None);
            };
            cx.ty_of(v0).map(Some)
        }
        _ => {
            let Some(&v0) = args.first() else {
                return Ok(None);
            };
            let arg_ty = cx.ty_of(v0)?;
            Ok(value::pointee_of(cx.table, arg_ty))
        }
    }
}

/// Precomputed layout and representation metadata for a native `Vec<T, Global>`.
struct NativeVecInfo {
    vec_ty: TypeId,
    elem_ty: TypeId,
    peeled_elem: TypeId,
    is_zst: bool,
    align: u64,
    ta_kind: Option<u8>,
    packed_arr: bool,
    min_cap: u64,
    max_elems: u64,
    tracked: bool,
    needs_drop: bool,
}

impl NativeVecInfo {
    fn from_vec_ty(cx: &JsCx<'_>, vec_ty: TypeId) -> Option<Self> {
        let table = cx.table;
        let heap = cx.heap();
        let elem_ty = crate::analysis::heap::global_vec_elem_ty(table, vec_ty)?;
        let elem_desc = table.get(elem_ty)?;
        let is_zst = elem_desc.layout.zst;
        let elem_size = elem_desc.layout.size.unwrap_or(0);
        let align = elem_desc.layout.align.unwrap_or(1).max(1);
        let ta_kind = if is_zst {
            None
        } else {
            crate::analysis::heap::native_vec_ta_kind(table, elem_ty)
        };
        let packed_arr = !is_zst && ta_kind.is_none() && heap.is_packed_arr_vec(table, vec_ty);
        let min_cap: u64 = if elem_size == 1 {
            8
        } else if elem_size <= 1024 {
            4
        } else {
            1
        };
        let max_elems: u64 = 2_147_483_647u64
            .checked_div(elem_size)
            .unwrap_or(4_294_967_295);
        let peeled_elem = table.peeled(elem_ty);
        let tracked = !is_zst
            && ta_kind.is_none()
            && heap.needs_box_tracking(table, elem_ty)
            && table.is_indirect(peeled_elem)
            && value::is_slot_pointee(table, peeled_elem);
        Some(Self {
            vec_ty,
            elem_ty,
            peeled_elem,
            is_zst,
            align,
            ta_kind,
            packed_arr,
            min_cap,
            max_elems,
            tracked,
            needs_drop: elem_desc.layout.needs_drop,
        })
    }
}

impl JsCx<'_> {
    /// Lowers `Vec::resize(&mut self, new_len, value)` on numeric `TypedArray`-backed
    /// `Vec<T, Global>` directly to reserve + native `TypedArray.prototype.fill` (C++ `memset`).
    pub(super) fn lower_native_vec_resize(&self, args: &[ValueId]) -> Res<Option<Lowered>> {
        let [v_v, new_len_v, val_v] = args else {
            return Ok(None);
        };
        let table = self.table;
        let arg_ty = self.ty_of(*v_v)?;
        let Some(vec_ty) = value::pointee_of(table, arg_ty) else {
            return Ok(None);
        };
        let Some(elem_ty) = crate::analysis::heap::global_vec_elem_ty(table, vec_ty) else {
            return Ok(None);
        };
        let Some(elem_desc) = table.get(elem_ty) else {
            return Ok(None);
        };
        if elem_desc.layout.zst {
            return Ok(None);
        }
        let Some(k) = crate::analysis::heap::native_vec_ta_kind(table, elem_ty) else {
            return Ok(None);
        };
        let elem_size = elem_desc.layout.size.unwrap_or(1).max(1);
        let min_cap: u64 = if elem_size == 1 {
            8
        } else if elem_size <= 1024 {
            4
        } else {
            1
        };
        let max_elems: u64 = 2_147_483_647u64 / elem_size;
        let suffix = TA_SUFFIXES[k as usize];

        let mut setup = Vec::new();
        let v = self.bind_atom(self.expr_of(*v_v)?, &mut setup);
        let nl = self.bind_atom(self.expr_of(*new_len_v)?, &mut setup);
        let val = self.bind_atom(self.expr_of(*val_v)?, &mut setup);

        let (b_ol, ol) = self.temp(jsast::member(v.clone(), "len"));
        setup.push(b_ol);
        let grow_body = vec![
            jsast::if_(
                jsast::binary(
                    jsast::BinOp::Gt,
                    nl.clone(),
                    jsast::member(jsast::member(v.clone(), "buf"), "length"),
                ),
                vec![jsast::expr_stmt(jsast::rt_call(
                    format!("nv_reserve_{suffix}"),
                    vec![
                        v.clone(),
                        ol.clone(),
                        jsast::binary(jsast::BinOp::Sub, nl.clone(), ol.clone()),
                        jsast::num(min_cap as f64),
                        Expr::Bool(false),
                        jsast::num(max_elems as f64),
                    ],
                ))],
            ),
            jsast::expr_stmt(jsast::method_call(
                jsast::member(v.clone(), "buf"),
                "fill",
                vec![val, ol.clone(), nl.clone()],
            )),
        ];
        setup.push(jsast::if_(
            jsast::binary(jsast::BinOp::Gt, nl.clone(), ol),
            grow_body,
        ));
        setup.push(jsast::assign_stmt(jsast::member(v, "len"), nl));
        Ok(Some(Lowered::Done(setup)))
    }

    /// Lowers Mires v4 Phase 3 universal flat `Vec<T, Global>` hooks
    /// (`StandardOperation::is_native_vec`) directly at call sites when
    /// `self.heap().native_vec_enabled` holds.
    pub(super) fn lower_native_vec_call(
        &self,
        op: &Op,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let Some(vec_ty) = resolve_vec_call_target(self, op, operation, args)? else {
            return Ok(None);
        };
        let Some(info) = NativeVecInfo::from_vec_ty(self, vec_ty) else {
            return Ok(None);
        };
        match operation {
            StandardOperation::NativeVecNew
            | StandardOperation::NativeVecWithCapacity
            | StandardOperation::NativeVecFromElemZeroed
            | StandardOperation::NativeVecFromArray
            | StandardOperation::NativeVecFromRawParts => {
                self.lower_native_vec_construct(&info, operation, args)
            }
            StandardOperation::NativeVecCapacity
            | StandardOperation::NativeVecAsPtr
            | StandardOperation::NativeVecAsMutPtr
            | StandardOperation::NativeVecAsSlice
            | StandardOperation::NativeVecAsMutSlice => {
                self.lower_native_vec_slice_access(&info, operation, args)
            }
            StandardOperation::NativeVecReserve
            | StandardOperation::NativeVecReserveExact
            | StandardOperation::NativeVecTryReserve
            | StandardOperation::NativeVecTryReserveExact
            | StandardOperation::NativeVecShrinkTo
            | StandardOperation::NativeVecTryShrinkTo
            | StandardOperation::NativeVecGrowOne => {
                self.lower_native_vec_capacity(&info, operation, args)
            }
            StandardOperation::NativeVecPush | StandardOperation::NativeVecPushMut => {
                self.lower_native_vec_push(op, &info, operation, args)
            }
            StandardOperation::NativeVecPop => self.lower_native_vec_pop(op, &info, args),
            StandardOperation::NativeVecIntoBoxedSlice => {
                self.lower_native_vec_into_boxed_slice(&info, args)
            }
            _ => Ok(None),
        }
    }

    fn lower_native_vec_construct(
        &self,
        info: &NativeVecInfo,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        match operation {
            StandardOperation::NativeVecNew => {
                let Some(empty) =
                    value::empty_native_vec_expr_for_heap(self.table, info.vec_ty, self.heap())
                else {
                    return Ok(None);
                };
                Ok(Some(Lowered::value(empty)))
            }
            StandardOperation::NativeVecWithCapacity => {
                self.lower_native_vec_with_capacity(info, args)
            }
            StandardOperation::NativeVecFromElemZeroed => {
                self.lower_native_vec_from_elem_zeroed(info, args)
            }
            StandardOperation::NativeVecFromArray => self.lower_native_vec_from_array(info, args),
            StandardOperation::NativeVecFromRawParts => {
                self.lower_native_vec_from_raw_parts(info, args)
            }
            _ => unreachable!("matched in lower_native_vec_call"),
        }
    }

    fn lower_native_vec_with_capacity(
        &self,
        info: &NativeVecInfo,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let [cap_v] = args else {
            return Ok(None);
        };
        if info.is_zst {
            let cap_e = self.expr_of(*cap_v)?;
            let mut setup = Vec::new();
            if !jsast::is_droppable(&cap_e) {
                setup.push(jsast::expr_stmt(cap_e));
            }
            return Ok(Some(Lowered::Value(
                setup,
                jsast::object_of(vec![("buf", jsast::undefined()), ("len", jsast::num(0))]),
            )));
        }
        if let Some(c) = self.const_scalar_u128(*cap_v) {
            if c == 0 {
                let Some(empty) =
                    value::empty_native_vec_expr_for_heap(self.table, info.vec_ty, self.heap())
                else {
                    return Ok(None);
                };
                return Ok(Some(Lowered::value(empty)));
            }
            if c <= u128::from(info.max_elems) {
                if let Some(k) = info.ta_kind {
                    return Ok(Some(Lowered::value(jsast::object_of(vec![
                        (
                            "buf",
                            jsast::new(jsast::id(TA_CTORS[k as usize]), vec![jsast::num(c as f64)]),
                        ),
                        ("len", jsast::num(0)),
                    ]))));
                }
                if info.packed_arr {
                    return Ok(Some(Lowered::value(jsast::object_of(vec![
                        ("buf", jsast::array(Vec::new())),
                        ("len", jsast::num(0)),
                    ]))));
                }
                return Ok(Some(Lowered::value(jsast::object_of(vec![
                    (
                        "buf",
                        jsast::new(jsast::id("Array"), vec![jsast::num(c as f64)]),
                    ),
                    ("len", jsast::num(0)),
                ]))));
            }
        }
        let cap = self.expr_of(*cap_v)?;
        if let Some(k) = info.ta_kind {
            Ok(Some(Lowered::value(jsast::rt_call(
                format!("nv_with_cap_{}", TA_SUFFIXES[k as usize]),
                vec![cap, jsast::num(info.max_elems as f64)],
            ))))
        } else if info.packed_arr {
            let mut setup = Vec::new();
            let cap = self.bind_atom(cap, &mut setup);
            setup.push(jsast::if_(
                jsast::binary(jsast::BinOp::Gt, cap, jsast::num(info.max_elems as f64)),
                vec![jsast::expr_stmt(jsast::rt_call(
                    "js_abort",
                    vec![jsast::string("capacity overflow")],
                ))],
            ));
            Ok(Some(Lowered::Value(
                setup,
                jsast::object_of(vec![
                    ("buf", jsast::array(Vec::new())),
                    ("len", jsast::num(0)),
                ]),
            )))
        } else {
            Ok(Some(Lowered::value(jsast::rt_call(
                "nv_with_cap_arr",
                vec![
                    cap,
                    jsast::num(info.align as f64),
                    jsast::num(info.max_elems as f64),
                ],
            ))))
        }
    }

    fn lower_native_vec_from_elem_zeroed(
        &self,
        info: &NativeVecInfo,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let [n_v] = args else {
            return Ok(None);
        };
        if info.is_zst {
            let n = self.expr_of(*n_v)?;
            return Ok(Some(Lowered::value(jsast::object_of(vec![
                ("buf", jsast::undefined()),
                ("len", n),
            ]))));
        }
        if let Some(k) = info.ta_kind {
            if let Some(c) = self.const_scalar_u128(*n_v) {
                if c == 0 {
                    let Some(empty) =
                        value::empty_native_vec_expr_for_heap(self.table, info.vec_ty, self.heap())
                    else {
                        return Ok(None);
                    };
                    return Ok(Some(Lowered::value(empty)));
                }
                if c <= u128::from(info.max_elems) {
                    return Ok(Some(Lowered::value(jsast::object_of(vec![
                        (
                            "buf",
                            jsast::new(jsast::id(TA_CTORS[k as usize]), vec![jsast::num(c as f64)]),
                        ),
                        ("len", jsast::num(c as f64)),
                    ]))));
                }
            }
            let n = self.expr_of(*n_v)?;
            return Ok(Some(Lowered::value(jsast::rt_call(
                format!("nv_zeroed_{}", TA_SUFFIXES[k as usize]),
                vec![n, jsast::num(info.max_elems as f64)],
            ))));
        }
        let Some(zero) =
            value::zero_value_for_heap_plan(self.table, self.interns, info.elem_ty, self.heap())
        else {
            return Ok(None);
        };
        let n = self.expr_of(*n_v)?;
        if !self.table.is_indirect(info.peeled_elem) {
            if info.packed_arr {
                Ok(Some(Lowered::value(jsast::rt_call(
                    "nv_zeroed_arr_p",
                    vec![n, jsast::num(info.max_elems as f64), zero],
                ))))
            } else {
                Ok(Some(Lowered::value(jsast::rt_call(
                    "nv_zeroed_arr",
                    vec![
                        n,
                        jsast::num(info.align as f64),
                        jsast::num(info.max_elems as f64),
                        zero,
                    ],
                ))))
            }
        } else if info.packed_arr {
            let helper = if info.tracked {
                "nv_zeroed_arr_fn_tracked_p"
            } else {
                "nv_zeroed_arr_fn_p"
            };
            Ok(Some(Lowered::value(jsast::rt_call(
                helper,
                vec![
                    n,
                    jsast::num(info.max_elems as f64),
                    jsast::arrow(Vec::new(), zero),
                ],
            ))))
        } else {
            let helper = if info.tracked {
                "nv_zeroed_arr_fn_tracked"
            } else {
                "nv_zeroed_arr_fn"
            };
            Ok(Some(Lowered::value(jsast::rt_call(
                helper,
                vec![
                    n,
                    jsast::num(info.align as f64),
                    jsast::num(info.max_elems as f64),
                    jsast::arrow(Vec::new(), zero),
                ],
            ))))
        }
    }

    fn lower_native_vec_from_array(
        &self,
        info: &NativeVecInfo,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let [arr_v] = args else {
            return Ok(None);
        };
        let table = self.table;
        let arr_ty = self.ty_of(*arr_v)?;
        let Some(TypeKind::Array { length, .. }) = table.get(table.peeled(arr_ty)).map(|d| &d.kind)
        else {
            return Ok(None);
        };
        let len = *length;
        if info.is_zst {
            let arr_e = self.expr_of(*arr_v)?;
            let mut setup = Vec::new();
            if !jsast::is_droppable(&arr_e) {
                setup.push(jsast::expr_stmt(arr_e));
            }
            return Ok(Some(Lowered::Value(
                setup,
                jsast::object_of(vec![
                    ("buf", jsast::undefined()),
                    ("len", jsast::num(len as f64)),
                ]),
            )));
        }
        if len == 0 {
            let arr_e = self.expr_of(*arr_v)?;
            let mut setup = Vec::new();
            if !jsast::is_droppable(&arr_e) {
                setup.push(jsast::expr_stmt(arr_e));
            }
            let Some(empty) =
                value::empty_native_vec_expr_for_heap(table, info.vec_ty, self.heap())
            else {
                return Ok(None);
            };
            return Ok(Some(Lowered::Value(setup, empty)));
        }
        let arr_expr = self.expr_of(*arr_v)?;
        if let Some(k) = info.ta_kind {
            return Ok(Some(Lowered::value(jsast::object_of(vec![
                (
                    "buf",
                    jsast::new(jsast::id(TA_CTORS[k as usize]), vec![arr_expr]),
                ),
                ("len", jsast::num(len as f64)),
            ]))));
        }
        let fresh_owned = {
            let mut cur = *arr_v;
            for _ in 0..8 {
                if let Some(OpKind::Move(inner)) = self.def_of(cur).map(|d| &d.kind) {
                    cur = *inner;
                } else {
                    break;
                }
            }
            match self.def_of(cur).map(|d| &d.kind) {
                Some(OpKind::Aggregate { .. } | OpKind::Repeat { .. }) => true,
                // `OpKind::Copy` already deep-clones `[T; N]` in `emit.rs` when `needs_clone` holds.
                Some(OpKind::Copy(_)) => table.needs_clone(arr_ty),
                _ => false,
            }
        };
        let mut owned_arr = if fresh_owned {
            arr_expr
        } else {
            value::clone_expr_with(table, self.interns, arr_ty, arr_expr, self.nested_clone)
        };
        if info.tracked {
            owned_arr = jsast::rt_call("nv_box_track_arr", vec![owned_arr]);
        }
        Ok(Some(Lowered::value(jsast::object_of(vec![
            ("buf", owned_arr),
            ("len", jsast::num(len as f64)),
        ]))))
    }

    fn lower_native_vec_from_raw_parts(
        &self,
        info: &NativeVecInfo,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let [ptr_v, len_v, cap_v] = args else {
            return Ok(None);
        };
        let ptr = self.expr_of(*ptr_v)?;
        let len = self.expr_of(*len_v)?;
        let cap = self.expr_of(*cap_v)?;
        if info.is_zst {
            let mut setup = Vec::new();
            if !jsast::is_droppable(&ptr) {
                setup.push(jsast::expr_stmt(ptr));
            }
            if !jsast::is_droppable(&cap) {
                setup.push(jsast::expr_stmt(cap));
            }
            return Ok(Some(Lowered::Value(
                setup,
                jsast::object_of(vec![("buf", jsast::undefined()), ("len", len)]),
            )));
        }
        if let Some(k) = info.ta_kind {
            Ok(Some(Lowered::value(jsast::rt_call(
                "nv_from_raw_ta",
                vec![ptr, len, cap, jsast::num(f64::from(k))],
            ))))
        } else {
            Ok(Some(Lowered::value(jsast::rt_call(
                "nv_from_raw_arr",
                vec![ptr, len, cap, jsast::num(info.align as f64)],
            ))))
        }
    }

    fn lower_native_vec_slice_access(
        &self,
        info: &NativeVecInfo,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let [v_v] = args else {
            return Ok(None);
        };
        match operation {
            StandardOperation::NativeVecCapacity => {
                if info.is_zst {
                    let ve = self.expr_of(*v_v)?;
                    let mut setup = Vec::new();
                    if !jsast::is_droppable(&ve) {
                        setup.push(jsast::expr_stmt(ve));
                    }
                    return Ok(Some(Lowered::Value(setup, jsast::num(4_294_967_295.0))));
                }
                let v = self.expr_of(*v_v)?;
                Ok(Some(Lowered::value(jsast::member(
                    jsast::member(v, "buf"),
                    "length",
                ))))
            }
            StandardOperation::NativeVecAsPtr | StandardOperation::NativeVecAsMutPtr => {
                if info.is_zst {
                    let ve = self.expr_of(*v_v)?;
                    let mut setup = Vec::new();
                    if !jsast::is_droppable(&ve) {
                        setup.push(jsast::expr_stmt(ve));
                    }
                    return Ok(Some(Lowered::Value(setup, jsast::num(info.align as f64))));
                }
                let v = self.expr_of(*v_v)?;
                Ok(Some(Lowered::value(ptr::slot(
                    jsast::member(v, "buf"),
                    jsast::num(0),
                ))))
            }
            StandardOperation::NativeVecAsSlice | StandardOperation::NativeVecAsMutSlice => {
                let mut setup = Vec::new();
                let v = self.bind_temp_if_needed(self.expr_of(*v_v)?, &mut setup);
                let buf = if info.is_zst {
                    jsast::num(info.align as f64)
                } else {
                    jsast::member(v.clone(), "buf")
                };
                let slice = ptr::fat_slice(buf, jsast::num(0), jsast::member(v, "len"));
                Ok(Some(Lowered::Value(setup, slice)))
            }
            _ => unreachable!("matched in lower_native_vec_call"),
        }
    }

    fn lower_native_vec_capacity(
        &self,
        info: &NativeVecInfo,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        match operation {
            StandardOperation::NativeVecReserve | StandardOperation::NativeVecReserveExact => self
                .lower_native_vec_reserve(
                    info,
                    operation == StandardOperation::NativeVecReserveExact,
                    args,
                ),
            StandardOperation::NativeVecTryReserve
            | StandardOperation::NativeVecTryReserveExact => self.lower_native_vec_try_reserve(
                info,
                operation == StandardOperation::NativeVecTryReserveExact,
                args,
            ),
            StandardOperation::NativeVecShrinkTo | StandardOperation::NativeVecTryShrinkTo => self
                .lower_native_vec_shrink(
                    info,
                    operation == StandardOperation::NativeVecTryShrinkTo,
                    args,
                ),
            StandardOperation::NativeVecGrowOne => self.lower_native_vec_grow_one(info, args),
            _ => unreachable!("matched in lower_native_vec_call"),
        }
    }

    fn lower_native_vec_reserve(
        &self,
        info: &NativeVecInfo,
        exact: bool,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let [v_v, used_v, add_v] = args else {
            return Ok(None);
        };
        let v = self.expr_of(*v_v)?;
        let used = self.expr_of(*used_v)?;
        let add = self.expr_of(*add_v)?;
        if info.is_zst || info.packed_arr {
            let limit = if info.is_zst {
                4_294_967_295.0
            } else {
                info.max_elems as f64
            };
            let mut setup = Vec::new();
            if !jsast::is_droppable(&v) {
                setup.push(jsast::expr_stmt(v));
            }
            setup.push(jsast::if_(
                jsast::binary(
                    jsast::BinOp::Gt,
                    jsast::binary(jsast::BinOp::Add, used, add),
                    jsast::num(limit),
                ),
                vec![jsast::expr_stmt(jsast::rt_call(
                    "js_abort",
                    vec![jsast::string("capacity overflow")],
                ))],
            ));
            return Ok(Some(Lowered::Done(setup)));
        }
        let mut setup = Vec::new();
        let v = self.bind_temp_if_needed(v, &mut setup);
        let used = self.bind_atom(used, &mut setup);
        let add = self.bind_atom(add, &mut setup);
        let call = if let Some(k) = info.ta_kind {
            jsast::rt_call(
                format!("nv_reserve_{}", TA_SUFFIXES[k as usize]),
                vec![
                    v.clone(),
                    used.clone(),
                    add.clone(),
                    jsast::num(info.min_cap as f64),
                    Expr::Bool(exact),
                    jsast::num(info.max_elems as f64),
                ],
            )
        } else {
            jsast::rt_call(
                "nv_reserve_arr",
                vec![
                    v.clone(),
                    used.clone(),
                    add.clone(),
                    jsast::num(info.min_cap as f64),
                    Expr::Bool(exact),
                    jsast::num(info.max_elems as f64),
                ],
            )
        };
        setup.push(jsast::if_(
            jsast::binary(
                jsast::BinOp::Gt,
                jsast::binary(jsast::BinOp::Add, used, add),
                jsast::member(jsast::member(v, "buf"), "length"),
            ),
            vec![jsast::expr_stmt(call)],
        ));
        Ok(Some(Lowered::Done(setup)))
    }

    fn lower_native_vec_try_reserve(
        &self,
        info: &NativeVecInfo,
        exact: bool,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let [v_v, used_v, add_v] = args else {
            return Ok(None);
        };
        let v = self.expr_of(*v_v)?;
        let used = self.expr_of(*used_v)?;
        let add = self.expr_of(*add_v)?;
        if info.is_zst || info.packed_arr {
            let limit = if info.is_zst {
                4_294_967_295.0
            } else {
                info.max_elems as f64
            };
            let mut setup = Vec::new();
            if !jsast::is_droppable(&v) {
                setup.push(jsast::expr_stmt(v));
            }
            return Ok(Some(Lowered::Value(
                setup,
                jsast::binary(
                    jsast::BinOp::Le,
                    jsast::binary(jsast::BinOp::Add, used, add),
                    jsast::num(limit),
                ),
            )));
        }
        if let Some(k) = info.ta_kind {
            Ok(Some(Lowered::value(jsast::rt_call(
                "nv_try_reserve_ta",
                vec![
                    v,
                    used,
                    add,
                    jsast::num(info.min_cap as f64),
                    Expr::Bool(exact),
                    jsast::num(f64::from(k)),
                    jsast::num(info.max_elems as f64),
                ],
            ))))
        } else {
            Ok(Some(Lowered::value(jsast::rt_call(
                "nv_try_reserve_arr",
                vec![
                    v,
                    used,
                    add,
                    jsast::num(info.min_cap as f64),
                    Expr::Bool(exact),
                    jsast::num(info.max_elems as f64),
                ],
            ))))
        }
    }

    fn lower_native_vec_shrink(
        &self,
        info: &NativeVecInfo,
        is_try: bool,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let [v_v, target_v] = args else {
            return Ok(None);
        };
        let v = self.expr_of(*v_v)?;
        let target = self.expr_of(*target_v)?;
        let mut setup = Vec::new();
        if info.is_zst || info.packed_arr {
            if !jsast::is_droppable(&v) {
                setup.push(jsast::expr_stmt(v));
            }
            if !jsast::is_droppable(&target) {
                setup.push(jsast::expr_stmt(target));
            }
        } else if let Some(k) = info.ta_kind {
            setup.push(jsast::expr_stmt(jsast::rt_call(
                "nv_shrink_ta",
                vec![v, target, jsast::num(f64::from(k))],
            )));
        } else {
            setup.push(jsast::expr_stmt(jsast::rt_call(
                "nv_shrink_arr",
                vec![v, target, jsast::num(info.align as f64)],
            )));
        }
        if is_try {
            Ok(Some(Lowered::Value(setup, Expr::Bool(true))))
        } else {
            Ok(Some(Lowered::Done(setup)))
        }
    }

    fn lower_native_vec_grow_one(
        &self,
        info: &NativeVecInfo,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let [v_v] = args else {
            return Ok(None);
        };
        let v = self.expr_of(*v_v)?;
        if info.is_zst || info.packed_arr {
            let limit = if info.is_zst {
                4_294_967_295.0
            } else {
                info.max_elems as f64
            };
            return Ok(Some(Lowered::Done(vec![jsast::if_(
                jsast::binary(jsast::BinOp::Ge, jsast::member(v, "len"), jsast::num(limit)),
                vec![jsast::expr_stmt(jsast::rt_call(
                    "js_abort",
                    vec![jsast::string("capacity overflow")],
                ))],
            )])));
        }
        if let Some(k) = info.ta_kind {
            Ok(Some(Lowered::Done(vec![jsast::expr_stmt(jsast::rt_call(
                format!("nv_grow_{}", TA_SUFFIXES[k as usize]),
                vec![
                    v,
                    jsast::num(info.min_cap as f64),
                    jsast::num(info.max_elems as f64),
                ],
            ))])))
        } else {
            Ok(Some(Lowered::Done(vec![jsast::expr_stmt(jsast::rt_call(
                "nv_grow_one_arr",
                vec![
                    v,
                    jsast::num(info.min_cap as f64),
                    jsast::num(info.max_elems as f64),
                ],
            ))])))
        }
    }

    fn lower_native_vec_push(
        &self,
        op: &Op,
        info: &NativeVecInfo,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let [v_v, val_v] = args else {
            return Ok(None);
        };
        let is_mut = operation == StandardOperation::NativeVecPushMut;
        let mut setup = Vec::new();
        let v = self.bind_temp_if_needed(self.expr_of(*v_v)?, &mut setup);
        let val_expr = self.expr_of(*val_v)?;
        if info.is_zst {
            if !jsast::is_droppable(&val_expr) {
                setup.push(jsast::expr_stmt(val_expr));
            }
            let (bl, l) = self.temp(jsast::member(v.clone(), "len"));
            setup.push(bl);
            setup.push(jsast::if_(
                jsast::binary(
                    jsast::BinOp::StrictEq,
                    l.clone(),
                    jsast::num(4_294_967_295.0),
                ),
                vec![jsast::expr_stmt(jsast::rt_call(
                    "js_abort",
                    vec![jsast::string("capacity overflow")],
                ))],
            ));
            setup.push(jsast::assign_stmt(
                jsast::member(v, "len"),
                jsast::binary(
                    jsast::BinOp::UShr,
                    jsast::binary(jsast::BinOp::Add, l, jsast::num(1)),
                    jsast::num(0),
                ),
            ));
            return if is_mut {
                Ok(Some(Lowered::Value(setup, jsast::num(info.align as f64))))
            } else {
                Ok(Some(Lowered::Done(setup)))
            };
        }
        if info.packed_arr {
            if !is_mut && !info.tracked {
                let (bb, b) = self.temp(jsast::member(v.clone(), "buf"));
                setup.push(bb);
                setup.push(jsast::expr_stmt(jsast::method_call(
                    b.clone(),
                    "push",
                    vec![val_expr],
                )));
                setup.push(jsast::assign_stmt(
                    jsast::member(v, "len"),
                    jsast::member(b, "length"),
                ));
                return Ok(Some(Lowered::Done(setup)));
            }
            let val = self.bind_temp_if_needed(val_expr, &mut setup);
            let (bb, b) = self.temp(jsast::member(v.clone(), "buf"));
            setup.push(bb);
            let (bl, l) = self.temp(jsast::member(b.clone(), "length"));
            setup.push(bl);
            setup.push(jsast::expr_stmt(jsast::method_call(
                b.clone(),
                "push",
                vec![val.clone()],
            )));
            setup.push(jsast::assign_stmt(
                jsast::member(v, "len"),
                jsast::binary(jsast::BinOp::Add, l.clone(), jsast::num(1)),
            ));
            if info.tracked {
                setup.push(jsast::expr_stmt(jsast::rt_call(
                    "nv_box_track_slot",
                    vec![b.clone(), l.clone(), val.clone()],
                )));
            }
            if !is_mut {
                return Ok(Some(Lowered::Done(setup)));
            }
            let Some(res) = op.result else {
                return Ok(None);
            };
            let ret_ty = self.ty_of(res)?;
            let ret_expr = match self.table.pointer_form(ret_ty) {
                Some(PointerForm::Thin) => val,
                Some(PointerForm::Slot) => ptr::slot(b, l),
                _ => return Ok(None),
            };
            return Ok(Some(Lowered::Value(setup, ret_expr)));
        }
        let val = if is_mut || info.tracked {
            self.bind_temp_if_needed(val_expr, &mut setup)
        } else {
            val_expr
        };
        let (bl, l) = self.temp(jsast::member(v.clone(), "len"));
        setup.push(bl);
        if let Some(k) = info.ta_kind {
            setup.push(jsast::if_(
                jsast::binary(
                    jsast::BinOp::StrictEq,
                    l.clone(),
                    jsast::member(jsast::member(v.clone(), "buf"), "length"),
                ),
                vec![jsast::expr_stmt(jsast::rt_call(
                    format!("nv_grow_{}", TA_SUFFIXES[k as usize]),
                    vec![
                        v.clone(),
                        jsast::num(info.min_cap as f64),
                        jsast::num(info.max_elems as f64),
                    ],
                ))],
            ));
            setup.push(jsast::assign_stmt(
                jsast::index(jsast::member(v.clone(), "buf"), l.clone()),
                val.clone(),
            ));
        } else {
            setup.push(jsast::if_(
                jsast::binary(
                    jsast::BinOp::StrictEq,
                    l.clone(),
                    jsast::member(jsast::member(v.clone(), "buf"), "length"),
                ),
                vec![jsast::expr_stmt(jsast::rt_call(
                    "nv_grow_one_arr",
                    vec![
                        v.clone(),
                        jsast::num(info.min_cap as f64),
                        jsast::num(info.max_elems as f64),
                    ],
                ))],
            ));
            if info.tracked {
                setup.push(jsast::expr_stmt(jsast::rt_call(
                    "nv_box_track_slot",
                    vec![jsast::member(v.clone(), "buf"), l.clone(), val.clone()],
                )));
            } else {
                setup.push(jsast::assign_stmt(
                    jsast::index(jsast::member(v.clone(), "buf"), l.clone()),
                    val.clone(),
                ));
            }
        }
        setup.push(jsast::assign_stmt(
            jsast::member(v.clone(), "len"),
            jsast::binary(jsast::BinOp::Add, l.clone(), jsast::num(1)),
        ));
        if !is_mut {
            return Ok(Some(Lowered::Done(setup)));
        }
        let Some(res) = op.result else {
            return Ok(None);
        };
        let ret_ty = self.ty_of(res)?;
        let ret_expr = match self.table.pointer_form(ret_ty) {
            Some(PointerForm::Thin) => val,
            Some(PointerForm::Slot) => ptr::slot(jsast::member(v, "buf"), l),
            _ => return Ok(None),
        };
        Ok(Some(Lowered::Value(setup, ret_expr)))
    }

    fn lower_native_vec_pop(
        &self,
        op: &Op,
        info: &NativeVecInfo,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let [v_v] = args else {
            return Ok(None);
        };
        let Some(res) = op.result else {
            return Ok(None);
        };
        let table = self.table;
        let ret_ty = self.ty_of(res)?;
        let peeled_ret = table.peeled(ret_ty);
        let Some(TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            ..
        }) = table.get(peeled_ret).map(|d| &d.kind)
        else {
            return Ok(None);
        };
        let mut none_idx = None;
        let mut some_info = None;
        for (i, var) in variants.iter().enumerate() {
            if var.fields.is_empty() {
                none_idx = Some(i as u32);
            } else if var.fields.len() == 1 {
                some_info = Some((i as u32, var.fields[0].key));
            }
        }
        let (Some(none_idx), Some((some_idx, some_key))) = (none_idx, some_info) else {
            return Ok(None);
        };
        let Some(none_expr) = value::enum_value(table, self.interns, ret_ty, none_idx, Vec::new())
        else {
            return Ok(None);
        };

        let mut setup = Vec::new();
        let v = self.bind_temp_if_needed(self.expr_of(*v_v)?, &mut setup);
        let (_, res_id) = self.temp(jsast::undefined());

        let mut some_body = Vec::new();
        let elem_expr = if info.packed_arr {
            let (bb, b) = self.temp(jsast::member(v.clone(), "buf"));
            some_body.push(bb);
            let (be, e) = self.temp(jsast::method_call(b.clone(), "pop", Vec::new()));
            some_body.push(be);
            some_body.push(jsast::assign_stmt(
                jsast::member(v.clone(), "len"),
                jsast::member(b, "length"),
            ));
            e
        } else {
            let (bl, l) = self.temp(jsast::binary(
                jsast::BinOp::UShr,
                jsast::binary(
                    jsast::BinOp::Sub,
                    jsast::member(v.clone(), "len"),
                    jsast::num(1),
                ),
                jsast::num(0),
            ));
            some_body.push(bl);
            some_body.push(jsast::assign_stmt(
                jsast::member(v.clone(), "len"),
                l.clone(),
            ));
            if info.is_zst {
                jsast::undefined()
            } else {
                let (be, e) = self.temp(jsast::index(jsast::member(v.clone(), "buf"), l.clone()));
                some_body.push(be);
                if info.ta_kind.is_none() && info.needs_drop {
                    some_body.push(jsast::assign_stmt(
                        jsast::index(jsast::member(v.clone(), "buf"), l),
                        jsast::undefined(),
                    ));
                }
                e
            }
        };
        let some_fields = match value::key_name(self.interns, some_key) {
            Some(k) => vec![(k, elem_expr)],
            None => Vec::new(),
        };
        let Some(some_expr) = value::enum_value(table, self.interns, ret_ty, some_idx, some_fields)
        else {
            return Ok(None);
        };
        some_body.push(jsast::assign_stmt(res_id.clone(), some_expr));

        setup.push(jsast::if_else(
            jsast::binary(
                jsast::BinOp::StrictEq,
                jsast::member(v, "len"),
                jsast::num(0),
            ),
            vec![jsast::assign_stmt(res_id.clone(), none_expr)],
            some_body,
        ));
        Ok(Some(Lowered::Value(setup, res_id)))
    }

    fn lower_native_vec_into_boxed_slice(
        &self,
        info: &NativeVecInfo,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let [v_v] = args else {
            return Ok(None);
        };
        let mut setup = Vec::new();
        let v = self.bind_temp_if_needed(self.expr_of(*v_v)?, &mut setup);
        if info.is_zst {
            return Ok(Some(Lowered::Value(
                setup,
                ptr::fat_slice(
                    jsast::num(info.align as f64),
                    jsast::num(0),
                    jsast::member(v, "len"),
                ),
            )));
        }
        let (bl, l) = self.temp(jsast::member(v.clone(), "len"));
        setup.push(bl);
        let buf = if let Some(k) = info.ta_kind {
            let (bb, b) = self.temp(jsast::member(v, "buf"));
            setup.push(bb);
            let (bo, out) = self.temp(jsast::cond(
                jsast::binary(
                    jsast::BinOp::StrictEq,
                    jsast::member(b.clone(), "length"),
                    l.clone(),
                ),
                b.clone(),
                jsast::cond(
                    jsast::binary(jsast::BinOp::StrictEq, l.clone(), jsast::num(0)),
                    jsast::rt_call("_nv_et", vec![jsast::num(f64::from(k))]),
                    jsast::method_call(b, "slice", vec![jsast::num(0), l.clone()]),
                ),
            ));
            setup.push(bo);
            out
        } else if info.packed_arr {
            jsast::cond(
                jsast::binary(jsast::BinOp::StrictEq, l.clone(), jsast::num(0)),
                jsast::rt_call("_nv_ea", vec![jsast::num(info.align as f64)]),
                jsast::member(v, "buf"),
            )
        } else {
            let (_, b_id) = self.temp(jsast::undefined());
            setup.push(jsast::if_else(
                jsast::binary(jsast::BinOp::StrictEq, l.clone(), jsast::num(0)),
                vec![jsast::assign_stmt(
                    b_id.clone(),
                    jsast::rt_call("_nv_ea", vec![jsast::num(info.align as f64)]),
                )],
                vec![
                    jsast::assign_stmt(b_id.clone(), jsast::member(v, "buf")),
                    jsast::if_(
                        jsast::binary(
                            jsast::BinOp::Gt,
                            jsast::member(b_id.clone(), "length"),
                            l.clone(),
                        ),
                        vec![jsast::assign_stmt(
                            jsast::member(b_id.clone(), "length"),
                            l.clone(),
                        )],
                    ),
                ],
            ));
            b_id
        };
        Ok(Some(Lowered::Value(
            setup,
            ptr::fat_slice(buf, jsast::num(0), l),
        )))
    }
}
