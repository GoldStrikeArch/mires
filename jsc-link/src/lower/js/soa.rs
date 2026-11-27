//! Structure-of-Arrays (SoA) JavaScript lowering for non-escaping `Vec<FlatStruct, Global>`.
//!
//! For any element type `T` selected by [`crate::plan::soa_vec`]:
//! - `Vec<T, Global>` is `{ buf: SoaCols, len: n }`, where `SoaCols` is `{ f0: TA0, f1: TA1, ... }`
//!   and every column shares `.length === capacity`.
//! - `&[T]` / `&mut [T]` / `*const [T]` / `*mut [T]` keep `{ buf: SoaCols, off, len }`, matching
//!   `abi_flat` slice lane expansion `(s$buf, s$off, s$len)`.
//! - `&T` / `&mut T` / `*const T` / `*mut T` uniformly use `{ buf: SoaCols, off }` (and non-escaping
//!   local borrows are scalarized directly into `(buf, off)` without allocating a slot object).
//! - By-value `T` keeps its canonical JS object/tuple representation.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

use rcgjs_object::item::{ItemKind, ItemMap, JsName, LinkItem, Linkage};
use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::interns::StrId;
use rcgjs_object::jir::repr::{Op, OpKind, Proj, Root, ValueId};
use rcgjs_object::jir::semantic::{ConstTree, DropGlue, StandardOperation};
use rcgjs_object::jir::types::{NominalForm, TypeId, TypeKind};
use rcgjs_object::jsast::{self, BinOp, CompoundOp, Expr, Stmt};
use rcgjs_object::ptr::{self, BUF, OFF};

use crate::plan::soa_vec::{SoaLayout, SoaPlan};

use super::ast_util::is_zero_num;
use super::call::Lowered;
use super::place::{self, JsPlace};
use super::ptr::{sequenced, unwrap_offset_cond};
use super::{decline, value, JsCx, Res};

/// Encodes a JS `boolean` expression into a `Uint8Array` element (`0` or `1`).
pub fn encode_bool(val: Expr) -> Expr {
    match val {
        Expr::Bool(true) => jsast::num(1),
        Expr::Bool(false) => jsast::num(0),
        other => jsast::cond(other, jsast::num(1), jsast::num(0)),
    }
}

/// Decodes a `Uint8Array` element (`0` or `1`) back into a JS `boolean`.
pub fn decode_bool(raw: Expr) -> Expr {
    jsast::binary(BinOp::StrictNe, raw, jsast::num(0))
}

fn peel_seq(expr: Expr) -> (Vec<Expr>, Expr) {
    match expr {
        Expr::Seq(mut items) if !items.is_empty() => {
            let tail = items.pop().expect("non-empty seq");
            (items, tail)
        }
        other => (Vec::new(), other),
    }
}

fn sequenced_vec(mut pre: Vec<Expr>, tail: Expr) -> Expr {
    if pre.is_empty() {
        tail
    } else {
        pre.push(tail);
        jsast::seq(pre)
    }
}

/// Emits `{ buf: empty_sentinel, len: 0 }` for an SoA `Vec<T, Global>`.
pub fn empty_vec_expr(layout: &SoaLayout) -> Expr {
    jsast::object_of(vec![
        ("buf", jsast::id(layout.empty_name.as_str())),
        ("len", jsast::num(0)),
    ])
}

/// Emits `{ buf: empty_sentinel, off: 0, len: 0 }` for an empty SoA `&[T]` slice.
pub fn empty_slice_expr(layout: &SoaLayout) -> Expr {
    ptr::fat_slice(
        jsast::id(layout.empty_name.as_str()),
        jsast::num(0),
        jsast::num(0),
    )
}

/// Emits `{ f0: new TA0(cap), f1: new TA1(cap), ... }` inline for small constant allocations.
fn inline_alloc_cols_expr(layout: &SoaLayout, cap: Expr) -> Expr {
    let entries = layout
        .fields
        .iter()
        .map(|f| {
            (
                f.col_name.clone(),
                jsast::new(jsast::id(f.scalar.ta_ctor()), vec![cap.clone()]),
            )
        })
        .collect();
    jsast::object(entries)
}

impl<'a> JsCx<'a> {
    /// Returns the SoA layout if `ty` (after peeling) is an SoA element type.
    pub fn soa_layout(&self, ty: TypeId) -> Option<&SoaLayout> {
        self.soa?.get(self.table, ty)
    }

    /// Returns the SoA layout if `vec_ty` is a `Vec<T, Global>` whose element `T` uses SoA.
    pub fn soa_vec_layout(&self, vec_ty: TypeId) -> Option<&SoaLayout> {
        let elem_ty = crate::analysis::heap::global_vec_elem_ty(self.table, vec_ty)?;
        self.soa_layout(elem_ty)
    }

    /// Returns the SoA layout if `slice_ty` is `[T]` whose element `T` uses SoA.
    pub fn soa_slice_layout(&self, slice_ty: TypeId) -> Option<&SoaLayout> {
        match value::kind(self.table, self.table.peeled(slice_ty)) {
            Some(TypeKind::Slice(elem)) => self.soa_layout(*elem),
            _ => None,
        }
    }

    /// Whether `ptr` is a reference or raw pointer directly targeting an SoA element type `T`.
    pub fn is_soa_ptr(&self, ptr: ValueId) -> bool {
        self.ty_of(ptr)
            .ok()
            .and_then(|ty| value::pointee_of(self.table, ty))
            .is_some_and(|pt| self.soa_layout(pt).is_some())
    }

    /// Handles top-level `OpKind::Const` of an empty SoA `Vec<T, Global>` or empty `&[T]` slice.
    pub(super) fn soa_const_expr(&self, tree: &ConstTree, ty: TypeId) -> Res<Option<Expr>> {
        if self.soa.is_none_or(SoaPlan::is_empty) {
            return Ok(None);
        }
        let peeled = self.table.peeled(ty);
        if let Some(layout) = self.soa_vec_layout(peeled) {
            return Ok(Some(empty_vec_expr(layout)));
        }
        if let Some(pointee) = value::pointee_of(self.table, peeled) {
            if let Some(layout) = self.soa_slice_layout(pointee) {
                match tree {
                    ConstTree::Slice(elems) if elems.is_empty() => {
                        return Ok(Some(empty_slice_expr(layout)));
                    }
                    ConstTree::SliceAddress { length: 0, .. } => {
                        return Ok(Some(empty_slice_expr(layout)));
                    }
                    _ => {}
                }
            }
        }
        Ok(None)
    }

    /// Unpacks an SoA slot pointer expression (`{ buf, off }`) into `(buf, off)` without
    /// duplicating side effects, folding literal `{ buf, off }` objects directly.
    pub(super) fn soa_unpack_slot_expr_for_place(&self, pointer: Expr) -> (Expr, Expr) {
        let pointer = unwrap_offset_cond(pointer);
        let (pre, tail) = peel_seq(pointer);
        if let Expr::Object(entries) = &tail {
            let b = entries
                .iter()
                .find(|(k, _)| k == BUF)
                .map(|(_, v)| v.clone());
            let o = entries
                .iter()
                .find(|(k, _)| k == OFF)
                .map(|(_, v)| v.clone());
            if let (Some(b), Some(o)) = (b, o) {
                return (sequenced_vec(pre, b), o);
            }
        }
        let rebuilt = sequenced_vec(pre, tail);
        let (setup, slot) = self.reread(rebuilt, &[BUF, OFF]);
        (
            sequenced(setup, ptr::slot_buf(slot.clone())),
            ptr::slot_off(slot),
        )
    }

    /// Unpacks an SoA slot pointer into `(buf, off)` suitable for multi-column reads/writes,
    /// binding any non-trivial subexpressions into hoisted temps appended to `stmts`.
    fn soa_unpack_slot_stmts(&self, pointer: Expr, stmts: &mut Vec<Stmt>) -> (Expr, Expr) {
        let pointer = unwrap_offset_cond(pointer);
        let (pre, tail) = peel_seq(pointer);
        for e in pre {
            if !jsast::is_droppable(&e) {
                stmts.push(jsast::expr_stmt(e));
            }
        }
        let (b_raw, o_raw) = if let Expr::Object(ref entries) = tail {
            let b = entries
                .iter()
                .find(|(k, _)| k == BUF)
                .map(|(_, v)| v.clone());
            let o = entries
                .iter()
                .find(|(k, _)| k == OFF)
                .map(|(_, v)| v.clone());
            if let (Some(b), Some(o)) = (b, o) {
                (b, o)
            } else {
                let (bind, s) = self.temp(tail);
                stmts.push(bind);
                (ptr::slot_buf(s.clone()), ptr::slot_off(s))
            }
        } else if matches!(tail, Expr::Ident(_)) {
            (ptr::slot_buf(tail.clone()), ptr::slot_off(tail))
        } else {
            let (bind, s) = self.temp(tail);
            stmts.push(bind);
            (ptr::slot_buf(s.clone()), ptr::slot_off(s))
        };
        let b = self.bind_temp_if_needed(b_raw, stmts);
        let o = if matches!(o_raw, Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_)) {
            o_raw
        } else {
            let (bind, id) = self.temp(o_raw);
            stmts.push(bind);
            id
        };
        (b, o)
    }

    /// Materializes a canonical by-value JS object or tuple from SoA columns `(buf, off)`,
    /// evaluating `buf` and `off` at most once.
    pub fn soa_read_elem(&self, layout: &SoaLayout, buf: Expr, off: Expr) -> Expr {
        let mut seq = Vec::new();
        let b = if matches!(buf, Expr::Ident(_)) {
            buf
        } else {
            let (_, id) = self.temp(jsast::undefined());
            seq.push(jsast::assign(id.clone(), buf));
            id
        };
        let o = if matches!(off, Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_)) {
            off
        } else {
            let (_, id) = self.temp(jsast::undefined());
            seq.push(jsast::assign(id.clone(), off));
            id
        };
        let elem = if layout.is_tuple {
            let mut items = vec![jsast::undefined(); layout.total_fields];
            for f in &layout.fields {
                let raw = jsast::index(jsast::member(b.clone(), f.col_name.clone()), o.clone());
                let val = if f.scalar.is_bool() {
                    decode_bool(raw)
                } else {
                    raw
                };
                if let Some(slot) = items.get_mut(f.field_idx as usize) {
                    *slot = val;
                }
            }
            jsast::array(items)
        } else {
            let entries = layout
                .fields
                .iter()
                .map(|f| {
                    let raw = jsast::index(jsast::member(b.clone(), f.col_name.clone()), o.clone());
                    let val = if f.scalar.is_bool() {
                        decode_bool(raw)
                    } else {
                        raw
                    };
                    (
                        f.js_key_name
                            .clone()
                            .unwrap_or_else(|| format!("_{}", f.field_idx)),
                        val,
                    )
                })
                .collect();
            jsast::object(entries)
        };
        sequenced_vec(seq, elem)
    }

    /// Stores a by-value `T` expression into SoA columns `(buf, off)`, peeking through object or
    /// array literals to emit direct column stores with zero intermediate JS object allocation.
    pub fn soa_write_elem(
        &self,
        layout: &SoaLayout,
        buf: Expr,
        off: Expr,
        value: Expr,
    ) -> Res<Vec<Stmt>> {
        let mut stmts = Vec::new();
        let b = self.bind_temp_if_needed(buf, &mut stmts);
        let o = if matches!(off, Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_)) {
            off
        } else {
            let (bind, id) = self.temp(off);
            stmts.push(bind);
            id
        };

        // Zero-alloc peek-through for inline struct literals `{ k0: e0, k1: e1, ... }`.
        if !layout.is_tuple {
            if let Expr::Object(ref entries) = value {
                let known_keys: BTreeSet<&str> = layout
                    .fields
                    .iter()
                    .filter_map(|f| f.js_key_name.as_deref())
                    .collect();
                let mut seen: BTreeSet<&str> = BTreeSet::new();
                let mut can_peek = true;
                for (k, v) in entries {
                    if known_keys.contains(k.as_str()) {
                        if !seen.insert(k.as_str()) {
                            can_peek = false;
                            break;
                        }
                    } else if !jsast::is_droppable(v) {
                        can_peek = false;
                        break;
                    }
                }
                if can_peek && seen.len() == layout.fields.len() {
                    let Expr::Object(entries) = value else {
                        unreachable!()
                    };
                    let mut evaluated: HashMap<String, Expr> =
                        HashMap::with_capacity(layout.fields.len());
                    for (k, v) in entries {
                        if !known_keys.contains(k.as_str()) {
                            continue;
                        }
                        let val = self.bind_atom(v, &mut stmts);
                        evaluated.insert(k, val);
                    }
                    for f in &layout.fields {
                        let key = f.js_key_name.as_deref().unwrap_or("");
                        let raw = evaluated.remove(key).unwrap_or_else(jsast::undefined);
                        let rhs = if f.scalar.is_bool() {
                            encode_bool(raw)
                        } else {
                            raw
                        };
                        stmts.push(jsast::assign_stmt(
                            jsast::index(jsast::member(b.clone(), f.col_name.clone()), o.clone()),
                            rhs,
                        ));
                    }
                    return Ok(stmts);
                }
            }
        }

        // Zero-alloc peek-through for inline tuple literals `[e0, e1, ...]`.
        if layout.is_tuple {
            if let Expr::Array(ref items) = value {
                if items.len() == layout.total_fields {
                    let Expr::Array(items) = value else {
                        unreachable!()
                    };
                    let mut evaluated: Vec<Expr> = Vec::with_capacity(items.len());
                    for item in items {
                        let val = self.bind_atom(item, &mut stmts);
                        evaluated.push(val);
                    }
                    for f in &layout.fields {
                        let raw = evaluated[f.field_idx as usize].clone();
                        let rhs = if f.scalar.is_bool() {
                            encode_bool(raw)
                        } else {
                            raw
                        };
                        stmts.push(jsast::assign_stmt(
                            jsast::index(jsast::member(b.clone(), f.col_name.clone()), o.clone()),
                            rhs,
                        ));
                    }
                    return Ok(stmts);
                }
            }
        }

        // General case: evaluate `value` once and scatter its fields into the SoA columns.
        let v = if matches!(value, Expr::Ident(_)) {
            value
        } else {
            let (bind, id) = self.temp(value);
            stmts.push(bind);
            id
        };
        for f in &layout.fields {
            let raw = if layout.is_tuple {
                jsast::index(v.clone(), jsast::num(f.field_idx as f64))
            } else {
                jsast::member(
                    v.clone(),
                    f.js_key_name
                        .clone()
                        .unwrap_or_else(|| format!("_{}", f.field_idx)),
                )
            };
            let rhs = if f.scalar.is_bool() {
                encode_bool(raw)
            } else {
                raw
            };
            stmts.push(jsast::assign_stmt(
                jsast::index(jsast::member(b.clone(), f.col_name.clone()), o.clone()),
                rhs,
            ));
        }
        Ok(stmts)
    }

    /// Reads an entire SoA element through a raw pointer `{ buf, off }`.
    pub fn soa_deref_read(&self, layout: &SoaLayout, pointer: Expr) -> Expr {
        let (buf, off) = self.soa_unpack_slot_expr_for_place(pointer);
        self.soa_read_elem(layout, buf, off)
    }

    /// Writes an entire SoA element through a raw pointer `{ buf, off }`.
    pub fn soa_deref_write(
        &self,
        layout: &SoaLayout,
        pointer: Expr,
        value: Expr,
    ) -> Res<Vec<Stmt>> {
        let mut stmts = Vec::new();
        let (buf, off) = self.soa_unpack_slot_stmts(pointer, &mut stmts);
        stmts.extend(self.soa_write_elem(layout, buf, off, value)?);
        Ok(stmts)
    }

    /// Lowers `ptr::copy` / `ptr::copy_nonoverlapping` on an SoA element type `T`.
    pub fn soa_ptr_copy(
        &self,
        layout: &SoaLayout,
        dst: Expr,
        src: Expr,
        count: Expr,
        count_is_one: bool,
    ) -> Vec<Stmt> {
        if is_zero_num(&count) {
            let mut out = Vec::new();
            for e in [dst, src, count] {
                if !jsast::is_droppable(&e) {
                    out.push(jsast::expr_stmt(e));
                }
            }
            return out;
        }
        if count_is_one || count == jsast::num(1) {
            let mut stmts = Vec::new();
            let (db, doff) = self.soa_unpack_slot_stmts(dst, &mut stmts);
            let (sb, soff) = self.soa_unpack_slot_stmts(src, &mut stmts);
            // Copy each column directly (each column is an independent primitive TypedArray, so
            // column-by-column reads/writes are safe even when `dst === src`!).
            for f in &layout.fields {
                stmts.push(jsast::assign_stmt(
                    jsast::index(jsast::member(db.clone(), f.col_name.clone()), doff.clone()),
                    jsast::index(jsast::member(sb.clone(), f.col_name.clone()), soff.clone()),
                ));
            }
            return stmts;
        }
        vec![jsast::expr_stmt(jsast::call(
            jsast::id(layout.copy_name.as_str()),
            vec![dst, src, count],
        ))]
    }

    /// Lowers SoA-specific intrinsics (`slice_get_unchecked`, `typed_swap_nonoverlapping`,
    /// `copy`, `copy_nonoverlapping`, and volatile copies) BEFORE read-only `intrinsics::lower`.
    pub(super) fn lower_soa_intrinsic(
        &self,
        op: &Op,
        name_id: StrId,
        _types: &[TypeId],
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        if self.soa.is_none_or(SoaPlan::is_empty) {
            return Ok(None);
        }
        let Some(name) = self.interns.str(name_id) else {
            return Ok(None);
        };
        match name {
            "slice_get_unchecked" => {
                let [slice_v, idx_v] = args else {
                    return Ok(None);
                };
                let slice_ty = self.ty_of(*slice_v)?;
                let Some(pointee) = value::pointee_of(self.table, slice_ty) else {
                    return Ok(None);
                };
                if self.soa_slice_layout(pointee).is_none() {
                    return Ok(None);
                }
                let slice_expr = self.expr_of(*slice_v)?;
                let idx_expr = self.expr_of(*idx_v)?;
                let (buf, off) = self.soa_unpack_slot_expr_for_place(slice_expr);
                let total_off = if is_zero_num(&off) {
                    idx_expr
                } else {
                    place::offset(off, idx_expr)
                };
                Ok(Some(Lowered::value(ptr::slot(buf, total_off))))
            }
            "typed_swap_nonoverlapping" => {
                let [a_v, b_v] = args else {
                    return Ok(None);
                };
                let a_ty = self.ty_of(*a_v)?;
                let Some(pointee) = value::pointee_of(self.table, a_ty) else {
                    return Ok(None);
                };
                let Some(layout) = self.soa_layout(pointee) else {
                    return Ok(None);
                };
                let mut stmts = Vec::new();
                let (ab, ao) = self.soa_unpack_slot_stmts(self.expr_of(*a_v)?, &mut stmts);
                let (bb, bo) = self.soa_unpack_slot_stmts(self.expr_of(*b_v)?, &mut stmts);
                let (_, tmp) = self.temp(jsast::undefined());
                for f in &layout.fields {
                    let a_elem =
                        jsast::index(jsast::member(ab.clone(), f.col_name.clone()), ao.clone());
                    let b_elem =
                        jsast::index(jsast::member(bb.clone(), f.col_name.clone()), bo.clone());
                    stmts.push(jsast::assign_stmt(tmp.clone(), a_elem.clone()));
                    stmts.push(jsast::assign_stmt(a_elem, b_elem.clone()));
                    stmts.push(jsast::assign_stmt(b_elem, tmp.clone()));
                }
                Ok(Some(Lowered::Done(stmts)))
            }
            "copy"
            | "copy_nonoverlapping"
            | "volatile_copy_memory"
            | "volatile_copy_nonoverlapping_memory" => {
                let volatile = matches!(
                    name,
                    "volatile_copy_memory" | "volatile_copy_nonoverlapping_memory"
                );
                let (src_idx, _dst_idx) = if volatile { (1, 0) } else { (0, 1) };
                if args.len() < 3 {
                    return Ok(None);
                }
                let src_ty = self.ty_of(args[src_idx])?;
                let Some(pointee) = value::pointee_of(self.table, src_ty) else {
                    return Ok(None);
                };
                let Some(layout) = self.soa_layout(pointee) else {
                    return Ok(None);
                };
                let _ = op;
                let count_is_one = self.def_of(args[2]).is_some_and(|def| {
                    matches!(def.kind, OpKind::Const(ConstTree::Scalar { bits: 1, .. }))
                });
                // Evaluate arguments in Rust source order (`args[0]`, `args[1]`, `args[2]`).
                let a0 = self.expr_of(args[0])?;
                let a1 = self.expr_of(args[1])?;
                let count = self.expr_of(args[2])?;
                let (src, dst) = if volatile { (a1, a0) } else { (a0, a1) };
                Ok(Some(Lowered::Done(self.soa_ptr_copy(
                    layout,
                    dst,
                    src,
                    count,
                    count_is_one,
                ))))
            }
            _ => Ok(None),
        }
    }

    /// Lowers `core::slice::reverse` / `revswap` on SoA slices via direct per-column swaps.
    pub(super) fn lower_soa_slice_reverse(
        &self,
        is_reverse: bool,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        if self.soa.is_none_or(SoaPlan::is_empty) {
            return Ok(None);
        }
        let table = self.table;
        if is_reverse {
            let [slice_v] = args else {
                return Ok(None);
            };
            let slice_ty = table.peeled(self.ty_of(*slice_v)?);
            let Some(TypeKind::Reference {
                pointee,
                mutable: true,
            }) = value::kind(table, slice_ty)
            else {
                return Ok(None);
            };
            let Some(layout) = self.soa_slice_layout(*pointee) else {
                return Ok(None);
            };
            let slice_expr = self.expr_of(*slice_v)?;
            let mut setup = Vec::new();
            let (buf_raw, off_raw, len_raw) = match slice_expr {
                Expr::Object(ref entries) if entries.len() == 3 => {
                    let find = |k: &str| {
                        entries
                            .iter()
                            .find(|(ek, _)| ek == k)
                            .map(|(_, ev)| ev.clone())
                    };
                    if let (Some(b), Some(o), Some(l)) =
                        (find(ptr::BUF), find(ptr::OFF), find(ptr::LEN))
                    {
                        (b, o, l)
                    } else {
                        let (bs, s) = self.temp(slice_expr);
                        setup.push(bs);
                        (
                            ptr::slot_buf(s.clone()),
                            ptr::slot_off(s.clone()),
                            ptr::slot_len(s),
                        )
                    }
                }
                Expr::Ident(_) => (
                    ptr::slot_buf(slice_expr.clone()),
                    ptr::slot_off(slice_expr.clone()),
                    ptr::slot_len(slice_expr),
                ),
                other => {
                    let (bs, s) = self.temp(other);
                    setup.push(bs);
                    (
                        ptr::slot_buf(s.clone()),
                        ptr::slot_off(s.clone()),
                        ptr::slot_len(s),
                    )
                }
            };
            let b = self.bind_temp_if_needed(buf_raw, &mut setup);
            let off_is_zero = is_zero_num(&off_raw);
            let (bl, l) = self.temp(off_raw);
            setup.push(bl);
            let r_init = if off_is_zero {
                jsast::binary(BinOp::Sub, len_raw, jsast::num(1))
            } else {
                jsast::binary(
                    BinOp::Sub,
                    jsast::binary(BinOp::Add, l.clone(), len_raw),
                    jsast::num(1),
                )
            };
            let (br, r) = self.temp(r_init);
            setup.push(br);
            let (_, tmp) = self.temp(jsast::undefined());
            let mut body = Vec::with_capacity(layout.fields.len() * 3 + 2);
            for f in &layout.fields {
                let col = jsast::member(b.clone(), f.col_name.clone());
                let cl = jsast::index(col.clone(), l.clone());
                let cr = jsast::index(col, r.clone());
                body.push(jsast::assign_stmt(tmp.clone(), cl.clone()));
                body.push(jsast::assign_stmt(cl, cr.clone()));
                body.push(jsast::assign_stmt(cr, tmp.clone()));
            }
            body.push(jsast::expr_stmt(jsast::compound_assign(
                CompoundOp::Add,
                l.clone(),
                jsast::num(1),
            )));
            body.push(jsast::expr_stmt(jsast::compound_assign(
                CompoundOp::Sub,
                r.clone(),
                jsast::num(1),
            )));
            setup.push(jsast::while_(jsast::binary(BinOp::Lt, l, r), body));
            return Ok(Some(Lowered::Done(setup)));
        }

        // `core::slice::reverse::revswap::<T>(a: *mut T, b: *mut T, n: usize)`
        let [a_v, b_v, n_v] = args else {
            return Ok(None);
        };
        let (
            Some(TypeKind::RawPointer {
                pointee: elem_a,
                mutable: true,
            }),
            Some(TypeKind::RawPointer {
                pointee: elem_b,
                mutable: true,
            }),
        ) = (
            value::kind(table, table.peeled(self.ty_of(*a_v)?)),
            value::kind(table, table.peeled(self.ty_of(*b_v)?)),
        )
        else {
            return Ok(None);
        };
        if table.peeled(*elem_a) != table.peeled(*elem_b) {
            return Ok(None);
        }
        let Some(layout) = self.soa_layout(*elem_a) else {
            return Ok(None);
        };
        let mut setup = Vec::new();
        let (ab, ao) = self.soa_unpack_slot_stmts(self.expr_of(*a_v)?, &mut setup);
        let (bb, bo) = self.soa_unpack_slot_stmts(self.expr_of(*b_v)?, &mut setup);
        let n_expr = self.expr_of(*n_v)?;
        let n = if matches!(n_expr, Expr::Ident(_) | Expr::Num(_)) {
            n_expr
        } else {
            let (bn, id) = self.temp(n_expr);
            setup.push(bn);
            id
        };
        let (bl, l) = self.temp(ao);
        setup.push(bl);
        let (bend, end) = self.temp(jsast::binary(BinOp::Add, l.clone(), n.clone()));
        setup.push(bend);
        let (br, r) = self.temp(jsast::binary(
            BinOp::Sub,
            jsast::binary(BinOp::Add, bo, n),
            jsast::num(1),
        ));
        setup.push(br);
        let (_, tmp) = self.temp(jsast::undefined());
        let mut body = Vec::with_capacity(layout.fields.len() * 3 + 2);
        for f in &layout.fields {
            let al = jsast::index(jsast::member(ab.clone(), f.col_name.clone()), l.clone());
            let br_elem = jsast::index(jsast::member(bb.clone(), f.col_name.clone()), r.clone());
            body.push(jsast::assign_stmt(tmp.clone(), al.clone()));
            body.push(jsast::assign_stmt(al, br_elem.clone()));
            body.push(jsast::assign_stmt(br_elem, tmp.clone()));
        }
        body.push(jsast::expr_stmt(jsast::compound_assign(
            CompoundOp::Add,
            l.clone(),
            jsast::num(1),
        )));
        body.push(jsast::expr_stmt(jsast::compound_assign(
            CompoundOp::Sub,
            r,
            jsast::num(1),
        )));
        setup.push(jsast::while_(jsast::binary(BinOp::Lt, l, end), body));
        Ok(Some(Lowered::Done(setup)))
    }

    /// Lowers `Vec::resize(&mut self, new_len, value)` on an SoA `Vec<T, Global>` directly to
    /// reserve + per-column `TypedArray.prototype.fill`.
    pub(super) fn lower_soa_vec_resize(&self, args: &[ValueId]) -> Res<Option<Lowered>> {
        let [v_v, new_len_v, val_v] = args else {
            return Ok(None);
        };
        let arg_ty = self.ty_of(*v_v)?;
        let Some(vec_ty) = value::pointee_of(self.table, arg_ty) else {
            return Ok(None);
        };
        let Some(layout) = self.soa_vec_layout(vec_ty) else {
            return Ok(None);
        };

        let mut setup = Vec::new();
        let v = self.bind_temp_if_needed(self.expr_of(*v_v)?, &mut setup);
        let nl = self.bind_atom(self.expr_of(*new_len_v)?, &mut setup);
        let val_expr = self.expr_of(*val_v)?;

        // Extract field fill values before mutating `v` (peeking through struct/tuple literals).
        let mut fill_vals: Vec<Expr> = Vec::with_capacity(layout.fields.len());
        let mut peeked = false;
        if !layout.is_tuple {
            if let Expr::Object(ref entries) = val_expr {
                let known: BTreeSet<&str> = layout
                    .fields
                    .iter()
                    .filter_map(|f| f.js_key_name.as_deref())
                    .collect();
                let mut seen: BTreeSet<&str> = BTreeSet::new();
                let ok = entries.iter().all(|(k, ev)| {
                    if known.contains(k.as_str()) {
                        seen.insert(k.as_str())
                    } else {
                        jsast::is_droppable(ev)
                    }
                }) && seen.len() == layout.fields.len();
                if ok {
                    let Expr::Object(entries) = val_expr.clone() else {
                        unreachable!()
                    };
                    let mut map: HashMap<String, Expr> = HashMap::new();
                    for (k, ev) in entries {
                        if known.contains(k.as_str()) {
                            map.insert(k, self.bind_atom(ev, &mut setup));
                        }
                    }
                    for f in &layout.fields {
                        let key = f.js_key_name.as_deref().unwrap_or("");
                        let raw = map.remove(key).unwrap_or_else(jsast::undefined);
                        fill_vals.push(if f.scalar.is_bool() {
                            encode_bool(raw)
                        } else {
                            raw
                        });
                    }
                    peeked = true;
                }
            }
        } else if let Expr::Array(ref items) = val_expr {
            if items.len() == layout.total_fields {
                let mut evaluated = Vec::with_capacity(items.len());
                for item in items {
                    evaluated.push(self.bind_atom(item.clone(), &mut setup));
                }
                for f in &layout.fields {
                    let raw = evaluated[f.field_idx as usize].clone();
                    fill_vals.push(if f.scalar.is_bool() {
                        encode_bool(raw)
                    } else {
                        raw
                    });
                }
                peeked = true;
            }
        }
        if !peeked {
            let val_id = self.bind_temp_if_needed(val_expr, &mut setup);
            for f in &layout.fields {
                let raw = if layout.is_tuple {
                    jsast::index(val_id.clone(), jsast::num(f.field_idx as f64))
                } else {
                    jsast::member(
                        val_id.clone(),
                        f.js_key_name
                            .clone()
                            .unwrap_or_else(|| format!("_{}", f.field_idx)),
                    )
                };
                fill_vals.push(if f.scalar.is_bool() {
                    encode_bool(raw)
                } else {
                    raw
                });
            }
        }

        let (b_ol, ol) = self.temp(jsast::member(v.clone(), "len"));
        setup.push(b_ol);
        let mut grow_body = Vec::with_capacity(layout.fields.len() + 2);
        grow_body.push(jsast::if_(
            jsast::binary(
                BinOp::Gt,
                nl.clone(),
                jsast::member(
                    jsast::member(jsast::member(v.clone(), "buf"), "f0"),
                    "length",
                ),
            ),
            vec![jsast::expr_stmt(jsast::call(
                jsast::id(layout.reserve_name.as_str()),
                vec![
                    v.clone(),
                    ol.clone(),
                    jsast::binary(BinOp::Sub, nl.clone(), ol.clone()),
                    Expr::Bool(false),
                ],
            ))],
        ));
        let (bb, b) = self.temp(jsast::member(v.clone(), "buf"));
        grow_body.push(bb);
        for (f, fv) in layout.fields.iter().zip(fill_vals) {
            grow_body.push(jsast::expr_stmt(jsast::method_call(
                jsast::member(b.clone(), f.col_name.clone()),
                "fill",
                vec![fv, ol.clone(), nl.clone()],
            )));
        }
        setup.push(jsast::if_(
            jsast::binary(BinOp::Gt, nl.clone(), ol),
            grow_body,
        ));
        setup.push(jsast::assign_stmt(jsast::member(v, "len"), nl));
        Ok(Some(Lowered::Done(setup)))
    }

    /// Lowers universal flat `Vec<T, Global>` hooks for an SoA element type `T`.
    pub(super) fn lower_soa_vec_call(
        &self,
        op: &Op,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        if self.soa.is_none_or(SoaPlan::is_empty) {
            return Ok(None);
        }
        let Some(vec_ty) = super::call::resolve_vec_call_target(self, op, operation, args)? else {
            return Ok(None);
        };
        let Some(layout) = self.soa_vec_layout(vec_ty) else {
            return Ok(None);
        };
        match operation {
            StandardOperation::NativeVecNew
            | StandardOperation::NativeVecWithCapacity
            | StandardOperation::NativeVecFromElemZeroed
            | StandardOperation::NativeVecFromArray => {
                self.lower_soa_vec_construct(layout, operation, args)
            }
            StandardOperation::NativeVecCapacity
            | StandardOperation::NativeVecAsPtr
            | StandardOperation::NativeVecAsMutPtr
            | StandardOperation::NativeVecAsSlice
            | StandardOperation::NativeVecAsMutSlice
            | StandardOperation::VecAsSlice => {
                self.lower_soa_vec_index_slice(layout, operation, args)
            }
            StandardOperation::NativeVecReserve
            | StandardOperation::NativeVecReserveExact
            | StandardOperation::NativeVecTryReserve
            | StandardOperation::NativeVecTryReserveExact
            | StandardOperation::NativeVecShrinkTo
            | StandardOperation::NativeVecTryShrinkTo
            | StandardOperation::NativeVecGrowOne => {
                self.lower_soa_vec_grow_reserve(layout, operation, args)
            }
            StandardOperation::NativeVecPush
            | StandardOperation::NativeVecPushMut
            | StandardOperation::NativeVecPop => {
                self.lower_soa_vec_push_pop(op, layout, operation, args)
            }
            _ => Err(decline(
                OpClass::Call,
                format!("unsupported NativeVec operation `{operation:?}` on SoA Vec"),
            )),
        }
    }

    fn soa_const_u128(&self, v: ValueId) -> Option<u128> {
        match self.def_of(v).map(|d| &d.kind) {
            Some(OpKind::Const(ConstTree::Scalar { bits, .. })) => Some(*bits),
            _ => None,
        }
    }

    fn lower_soa_vec_construct(
        &self,
        layout: &SoaLayout,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        match operation {
            StandardOperation::NativeVecNew => Ok(Some(Lowered::value(empty_vec_expr(layout)))),
            StandardOperation::NativeVecWithCapacity => {
                let [cap_v] = args else {
                    return Ok(None);
                };
                if let Some(c) = self.soa_const_u128(*cap_v) {
                    if c == 0 {
                        return Ok(Some(Lowered::value(empty_vec_expr(layout))));
                    }
                    if c <= u128::from(layout.max_elems) && layout.fields.len() <= 4 {
                        return Ok(Some(Lowered::value(jsast::object_of(vec![
                            ("buf", inline_alloc_cols_expr(layout, jsast::num(c as f64))),
                            ("len", jsast::num(0)),
                        ]))));
                    }
                }
                let cap = self.expr_of(*cap_v)?;
                Ok(Some(Lowered::value(jsast::object_of(vec![
                    (
                        "buf",
                        jsast::call(jsast::id(layout.alloc_name.as_str()), vec![cap]),
                    ),
                    ("len", jsast::num(0)),
                ]))))
            }
            StandardOperation::NativeVecFromElemZeroed => {
                let [n_v] = args else {
                    return Ok(None);
                };
                if let Some(c) = self.soa_const_u128(*n_v) {
                    if c == 0 {
                        return Ok(Some(Lowered::value(empty_vec_expr(layout))));
                    }
                    if c <= u128::from(layout.max_elems) {
                        let buf = if layout.fields.len() <= 4 {
                            inline_alloc_cols_expr(layout, jsast::num(c as f64))
                        } else {
                            jsast::call(
                                jsast::id(layout.alloc_name.as_str()),
                                vec![jsast::num(c as f64)],
                            )
                        };
                        return Ok(Some(Lowered::value(jsast::object_of(vec![
                            ("buf", buf),
                            ("len", jsast::num(c as f64)),
                        ]))));
                    }
                }
                let n = self.expr_of(*n_v)?;
                Ok(Some(Lowered::value(jsast::call(
                    jsast::id(layout.zeroed_name.as_str()),
                    vec![n],
                ))))
            }
            StandardOperation::NativeVecFromArray => self.lower_soa_vec_from_array(layout, args),
            _ => unreachable!("matched in lower_soa_vec_call"),
        }
    }

    fn lower_soa_vec_from_array(
        &self,
        layout: &SoaLayout,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let [arr_v] = args else {
            return Ok(None);
        };
        let arr_ty = self.ty_of(*arr_v)?;
        let Some(TypeKind::Array { length, .. }) =
            self.table.get(self.table.peeled(arr_ty)).map(|d| &d.kind)
        else {
            return Ok(None);
        };
        let len = *length;
        let arr_expr = self.expr_of(*arr_v)?;
        if len == 0 {
            let mut setup = Vec::new();
            if !jsast::is_droppable(&arr_expr) {
                setup.push(jsast::expr_stmt(arr_expr));
            }
            return Ok(Some(Lowered::Value(setup, empty_vec_expr(layout))));
        }
        let mut setup = Vec::new();
        let alloc_expr = if layout.fields.len() <= 4 {
            inline_alloc_cols_expr(layout, jsast::num(len as f64))
        } else {
            jsast::call(
                jsast::id(layout.alloc_name.as_str()),
                vec![jsast::num(len as f64)],
            )
        };
        let (bb, b) = self.temp(alloc_expr);
        setup.push(bb);
        match arr_expr {
            Expr::Array(items) if items.len() == len as usize => {
                for (i, item) in items.into_iter().enumerate() {
                    setup.extend(self.soa_write_elem(
                        layout,
                        b.clone(),
                        jsast::num(i as f64),
                        item,
                    )?);
                }
            }
            other => {
                let is_mismatched_lit = matches!(other, Expr::Array(_));
                let arr_id = self.bind_temp_if_needed(other, &mut setup);
                if !is_mismatched_lit && len <= 4 {
                    for i in 0..len {
                        setup.extend(self.soa_write_elem(
                            layout,
                            b.clone(),
                            jsast::num(i as f64),
                            jsast::index(arr_id.clone(), jsast::num(i as f64)),
                        )?);
                    }
                } else {
                    let (bi, i) = self.temp(jsast::num(0));
                    setup.push(bi);
                    let mut body = self.soa_write_elem(
                        layout,
                        b.clone(),
                        i.clone(),
                        jsast::index(arr_id, i.clone()),
                    )?;
                    body.push(jsast::expr_stmt(jsast::compound_assign(
                        CompoundOp::Add,
                        i.clone(),
                        jsast::num(1),
                    )));
                    setup.push(jsast::while_(
                        jsast::binary(BinOp::Lt, i, jsast::num(len as f64)),
                        body,
                    ));
                }
            }
        }
        Ok(Some(Lowered::Value(
            setup,
            jsast::object_of(vec![("buf", b), ("len", jsast::num(len as f64))]),
        )))
    }

    fn lower_soa_vec_index_slice(
        &self,
        _layout: &SoaLayout,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let [v_v] = args else {
            return Ok(None);
        };
        match operation {
            StandardOperation::NativeVecCapacity => {
                let v = self.expr_of(*v_v)?;
                Ok(Some(Lowered::value(jsast::member(
                    jsast::member(jsast::member(v, "buf"), "f0"),
                    "length",
                ))))
            }
            StandardOperation::NativeVecAsPtr | StandardOperation::NativeVecAsMutPtr => {
                let v = self.expr_of(*v_v)?;
                Ok(Some(Lowered::value(ptr::slot(
                    jsast::member(v, "buf"),
                    jsast::num(0),
                ))))
            }
            StandardOperation::NativeVecAsSlice
            | StandardOperation::NativeVecAsMutSlice
            | StandardOperation::VecAsSlice => {
                let mut setup = Vec::new();
                let v = self.bind_temp_if_needed(self.expr_of(*v_v)?, &mut setup);
                let slice = ptr::fat_slice(
                    jsast::member(v.clone(), "buf"),
                    jsast::num(0),
                    jsast::member(v, "len"),
                );
                Ok(Some(Lowered::Value(setup, slice)))
            }
            _ => unreachable!("matched in lower_soa_vec_call"),
        }
    }

    fn lower_soa_vec_grow_reserve(
        &self,
        layout: &SoaLayout,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        match operation {
            StandardOperation::NativeVecReserve | StandardOperation::NativeVecReserveExact => {
                let [v_v, used_v, add_v] = args else {
                    return Ok(None);
                };
                let exact = operation == StandardOperation::NativeVecReserveExact;
                let mut setup = Vec::new();
                let v = self.bind_temp_if_needed(self.expr_of(*v_v)?, &mut setup);
                let used = self.bind_atom(self.expr_of(*used_v)?, &mut setup);
                let add = self.bind_atom(self.expr_of(*add_v)?, &mut setup);
                setup.push(jsast::if_(
                    jsast::binary(
                        BinOp::Gt,
                        jsast::binary(BinOp::Add, used.clone(), add.clone()),
                        jsast::member(
                            jsast::member(jsast::member(v.clone(), "buf"), "f0"),
                            "length",
                        ),
                    ),
                    vec![jsast::expr_stmt(jsast::call(
                        jsast::id(layout.reserve_name.as_str()),
                        vec![v, used, add, Expr::Bool(exact)],
                    ))],
                ));
                Ok(Some(Lowered::Done(setup)))
            }
            StandardOperation::NativeVecTryReserve
            | StandardOperation::NativeVecTryReserveExact => {
                let [v_v, used_v, add_v] = args else {
                    return Ok(None);
                };
                let exact = operation == StandardOperation::NativeVecTryReserveExact;
                let v = self.expr_of(*v_v)?;
                let used = self.expr_of(*used_v)?;
                let add = self.expr_of(*add_v)?;
                Ok(Some(Lowered::value(jsast::call(
                    jsast::id(layout.try_reserve_name.as_str()),
                    vec![v, used, add, Expr::Bool(exact)],
                ))))
            }
            StandardOperation::NativeVecShrinkTo | StandardOperation::NativeVecTryShrinkTo => {
                let [v_v, target_v] = args else {
                    return Ok(None);
                };
                let is_try = operation == StandardOperation::NativeVecTryShrinkTo;
                let v = self.expr_of(*v_v)?;
                let target = self.expr_of(*target_v)?;
                let setup = vec![jsast::expr_stmt(jsast::call(
                    jsast::id(layout.shrink_name.as_str()),
                    vec![v, target],
                ))];
                if is_try {
                    Ok(Some(Lowered::Value(setup, Expr::Bool(true))))
                } else {
                    Ok(Some(Lowered::Done(setup)))
                }
            }
            StandardOperation::NativeVecGrowOne => {
                let [v_v] = args else {
                    return Ok(None);
                };
                let v = self.expr_of(*v_v)?;
                Ok(Some(Lowered::Done(vec![jsast::expr_stmt(jsast::call(
                    jsast::id(layout.grow_name.as_str()),
                    vec![v],
                ))])))
            }
            _ => unreachable!("matched in lower_soa_vec_call"),
        }
    }

    fn lower_soa_vec_push_pop(
        &self,
        op: &Op,
        layout: &SoaLayout,
        operation: StandardOperation,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        match operation {
            StandardOperation::NativeVecPush | StandardOperation::NativeVecPushMut => {
                let [v_v, val_v] = args else {
                    return Ok(None);
                };
                let is_mut = operation == StandardOperation::NativeVecPushMut;
                let mut setup = Vec::new();
                let v = self.bind_temp_if_needed(self.expr_of(*v_v)?, &mut setup);
                let val_expr = self.expr_of(*val_v)?;
                let (bl, l) = self.temp(jsast::member(v.clone(), "len"));
                setup.push(bl);
                let (bb, b) = self.temp(jsast::member(v.clone(), "buf"));
                setup.push(bb);
                setup.push(jsast::if_(
                    jsast::binary(
                        BinOp::StrictEq,
                        l.clone(),
                        jsast::member(jsast::member(b.clone(), "f0"), "length"),
                    ),
                    vec![jsast::assign_stmt(
                        b.clone(),
                        jsast::call(jsast::id(layout.grow_name.as_str()), vec![v.clone()]),
                    )],
                ));
                setup.extend(self.soa_write_elem(layout, b.clone(), l.clone(), val_expr)?);
                setup.push(jsast::assign_stmt(
                    jsast::member(v, "len"),
                    jsast::binary(BinOp::Add, l.clone(), jsast::num(1)),
                ));
                if !is_mut {
                    return Ok(Some(Lowered::Done(setup)));
                }
                Ok(Some(Lowered::Value(setup, ptr::slot(b, l))))
            }
            StandardOperation::NativeVecPop => self.lower_soa_vec_pop(op, layout, args),
            _ => unreachable!("matched in lower_soa_vec_call"),
        }
    }

    fn lower_soa_vec_pop(
        &self,
        op: &Op,
        layout: &SoaLayout,
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
        let (bl, l) = self.temp(jsast::binary(
            BinOp::UShr,
            jsast::binary(BinOp::Sub, jsast::member(v.clone(), "len"), jsast::num(1)),
            jsast::num(0),
        ));
        some_body.push(bl);
        some_body.push(jsast::assign_stmt(
            jsast::member(v.clone(), "len"),
            l.clone(),
        ));
        let (bb, b) = self.temp(jsast::member(v.clone(), "buf"));
        some_body.push(bb);
        let elem_expr = self.soa_read_elem(layout, b, l);
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
            jsast::binary(BinOp::StrictEq, jsast::member(v, "len"), jsast::num(0)),
            vec![jsast::assign_stmt(res_id.clone(), none_expr)],
            some_body,
        ));
        Ok(Some(Lowered::Value(setup, res_id)))
    }

    /// Attempts to scalarize a non-inline `BorrowView` / `RawView` landing on an SoA element
    /// (`JsPlace::SoaElem`) or an SoA `bool` field (`JsPlace::SoaBool`) so local borrows like
    /// `let np = &nodes[p];` emit zero `{ buf, off }` slot allocations and keep `nodes$buf`
    /// loop-invariant for `loop_opt` column hoisting.
    pub(super) fn try_scalarize_soa_view(
        &self,
        op: &Op,
        result: ValueId,
    ) -> Res<Option<Vec<Stmt>>> {
        if self.soa.is_none_or(SoaPlan::is_empty) {
            return Ok(None);
        }
        let (OpKind::BorrowView { root, path, .. } | OpKind::RawView { root, path, .. }) = &op.kind
        else {
            return Ok(None);
        };
        if self.is_inline(result)
            || self.plan.physical_slots.contains(&result)
            || self.plan.physical_slices.contains(&result)
        {
            return Ok(None);
        }
        let Some(pointee) = value::pointee_of(self.table, self.ty_of(result)?) else {
            return Ok(None);
        };
        let is_soa_elem_view = self.soa_layout(pointee).is_some();
        let is_bool_view = value::is_bool(self.table, pointee);
        if !is_soa_elem_view && !is_bool_view {
            return Ok(None);
        }

        // Check whether every use of `result` in the function is a direct deref / field walk
        // (and track whether all uses stay in the defining block).
        let Some(&(def_block, _)) = self.defs.get(&result) else {
            return Ok(None);
        };
        let mut all_same_block = true;
        for (bi, block) in self.func.blocks.iter().enumerate() {
            if block.term.uses().contains(&result) {
                return Ok(None);
            }
            for use_op in &block.ops {
                if !use_op.kind.uses().contains(&result) {
                    continue;
                }
                if bi as u32 != def_block {
                    all_same_block = false;
                }
                let ok = match &use_op.kind {
                    OpKind::Deref(p) => *p == result,
                    OpKind::StoreThrough { ptr, v } => *ptr == result && *v != result,
                    OpKind::Drop {
                        glue: DropGlue::None,
                        ..
                    } => true,
                    OpKind::BorrowView {
                        root: Root::Value(r),
                        path: use_path,
                        ..
                    }
                    | OpKind::RawView {
                        root: Root::Value(r),
                        path: use_path,
                        ..
                    } if is_soa_elem_view => {
                        *r == result
                            && matches!(use_path.first(), Some(Proj::Deref))
                            && !use_path
                                .iter()
                                .any(|p| matches!(p, Proj::Index(i) if *i == result))
                    }
                    _ => false,
                };
                if !ok {
                    return Ok(None);
                }
            }
        }

        let walk = self.walk_place(root, path)?;
        match walk.place {
            JsPlace::SoaElem { buf, off, .. } => {
                let mut stmts = Vec::new();
                let b = self.pin_soa_component(buf, all_same_block, &mut stmts);
                let o = self.pin_soa_component(off, all_same_block, &mut stmts);
                self.soa_refs.borrow_mut().insert(result, (b, o));
                Ok(Some(stmts))
            }
            JsPlace::SoaBool { buf, col, off } => {
                let mut stmts = Vec::new();
                let b = self.pin_soa_component(buf, all_same_block, &mut stmts);
                let o = self.pin_soa_component(off, all_same_block, &mut stmts);
                self.soa_bool_refs.borrow_mut().insert(result, (b, col, o));
                Ok(Some(stmts))
            }
            _ => Ok(None),
        }
    }

    /// Pins one `(buf | off)` component of a scalarized SoA reference so it is safe to reread
    /// at every use site without allocating a slot object or breaking `loop_opt` hoisting.
    fn pin_soa_component(&self, expr: Expr, all_same_block: bool, stmts: &mut Vec<Stmt>) -> Expr {
        match &expr {
            Expr::Num(_) | Expr::RawNum(_) | Expr::Bool(_) => return expr,
            Expr::Ident(name) => {
                if self.is_immutable_formal_ident(name) {
                    return expr;
                }
                if all_same_block && !self.is_cell_ident(name) {
                    return expr;
                }
            }
            _ => {}
        }
        let (bind, id) = self.temp(expr);
        stmts.push(bind);
        id
    }

    fn is_cell_ident(&self, name: &str) -> bool {
        self.plan.cell_names.values().any(|n| n == name)
    }

    fn is_immutable_formal_ident(&self, name: &str) -> bool {
        if !self.is_formal_param_name(name) || self.is_cell_ident(name) {
            return false;
        }
        // Check no edge copy writes to this formal parameter (e.g. a coalesced loop phi).
        for copies in self.plan.copies.values() {
            for copy in copies {
                if let crate::lower::plan::CopyOperand::Value(v) = copy.dst {
                    if let Some(dst_name) = self.name_of(v) {
                        if dst_name == name
                            || (self.plan.physical_slices.contains(&v)
                                && (name == format!("{dst_name}$buf")
                                    || name == format!("{dst_name}$off")
                                    || name == format!("{dst_name}$len")))
                            || (self.plan.physical_slots.contains(&v)
                                && (name == format!("{dst_name}$buf")
                                    || name == format!("{dst_name}$off")))
                        {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }
}

/// Emits linker-synthesized SoA helper items (`const $__mires_soa_eK`, `function $__mires_soa_*`)
/// for every helper referenced by the lowered module (closed transitively over helper-to-helper
/// references).
pub fn needed_helper_items(
    plan: &SoaPlan,
    referenced: &BTreeSet<JsName>,
    existing: &BTreeSet<JsName>,
) -> Vec<LinkItem> {
    if plan.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for layout in plan.layouts.values() {
        let needed = SoaNeededHelpers::for_layout(layout, referenced);
        emit_soa_alloc_helpers(layout, &needed, existing, &mut out);
        emit_soa_capacity_helpers(layout, &needed, existing, &mut out);
        emit_soa_copy_helper(layout, &needed, existing, &mut out);
    }
    out
}

struct SoaNeededHelpers {
    empty: bool,
    alloc: bool,
    zeroed: bool,
    grow: bool,
    reserve: bool,
    try_reserve: bool,
    shrink: bool,
    copy: bool,
}

impl SoaNeededHelpers {
    fn for_layout(layout: &SoaLayout, referenced: &BTreeSet<JsName>) -> Self {
        let mut empty = referenced.contains(&layout.empty_name);
        let mut alloc = referenced.contains(&layout.alloc_name);
        let zeroed = referenced.contains(&layout.zeroed_name);
        let grow = referenced.contains(&layout.grow_name);
        let reserve = referenced.contains(&layout.reserve_name);
        let try_reserve = referenced.contains(&layout.try_reserve_name);
        let shrink = referenced.contains(&layout.shrink_name);
        let copy = referenced.contains(&layout.copy_name);
        if zeroed || grow || reserve || try_reserve {
            alloc = true;
        }
        if alloc || zeroed || shrink {
            empty = true;
        }
        Self {
            empty,
            alloc,
            zeroed,
            grow,
            reserve,
            try_reserve,
            shrink,
            copy,
        }
    }
}

fn synth_soa_helper(name: &JsName, kind: ItemKind, refs: Vec<JsName>, js: String) -> LinkItem {
    LinkItem {
        js,
        name: name.clone(),
        kind,
        refs: refs.into_iter().collect(),
        zombies: Vec::new(),
        linkage: Linkage::internal(),
        debug_path: "SoA Vec runtime helper".to_owned(),
        crate_name: "generated".to_owned(),
        order: None,
        map: ItemMap::default(),
        dts: None,
        optimization: Default::default(),
        allocator_provider: Default::default(),
        jir: Default::default(),
    }
}

fn format_col_allocs(layout: &SoaLayout, size_arg: &str) -> String {
    let mut cols = String::new();
    for (idx, f) in layout.fields.iter().enumerate() {
        if idx > 0 {
            cols.push_str(", ");
        }
        let _ = write!(
            cols,
            "{}: new {}({size_arg})",
            f.col_name,
            f.scalar.ta_ctor()
        );
    }
    cols
}

fn format_reserve_sets(layout: &SoaLayout) -> String {
    let mut sets = String::new();
    for f in &layout.fields {
        let _ = writeln!(
            sets,
            "    nb.{c}.set(used === cap ? old.{c} : old.{c}.subarray(0, used));",
            c = f.col_name
        );
    }
    sets
}

fn emit_soa_alloc_helpers(
    layout: &SoaLayout,
    needed: &SoaNeededHelpers,
    existing: &BTreeSet<JsName>,
    out: &mut Vec<LinkItem>,
) {
    if needed.empty && !existing.contains(&layout.empty_name) {
        let cols = format_col_allocs(layout, "0");
        out.push(synth_soa_helper(
            &layout.empty_name,
            ItemKind::Const,
            Vec::new(),
            format!("\nconst {} = {{ {cols} }};\n", layout.empty_name.as_str()),
        ));
    }

    if needed.alloc && !existing.contains(&layout.alloc_name) {
        let cols = format_col_allocs(layout, "n");
        out.push(synth_soa_helper(
            &layout.alloc_name,
            ItemKind::Fn,
            vec![layout.empty_name.clone()],
            format!(
                "\nfunction {}(n) {{\n  \
                 if (n === 0) return {};\n  \
                 if (n > {}) __rt.js_abort(\"capacity overflow\");\n  \
                 return {{ {cols} }};\n\
                 }}\n",
                layout.alloc_name.as_str(),
                layout.empty_name.as_str(),
                layout.max_elems,
            ),
        ));
    }

    if needed.zeroed && !existing.contains(&layout.zeroed_name) {
        out.push(synth_soa_helper(
            &layout.zeroed_name,
            ItemKind::Fn,
            vec![layout.empty_name.clone(), layout.alloc_name.clone()],
            format!(
                "\nfunction {}(n) {{\n  \
                 return n === 0 ? {{ buf: {}, len: 0 }} : {{ buf: {}(n), len: n }};\n\
                 }}\n",
                layout.zeroed_name.as_str(),
                layout.empty_name.as_str(),
                layout.alloc_name.as_str(),
            ),
        ));
    }
}

fn emit_soa_capacity_helpers(
    layout: &SoaLayout,
    needed: &SoaNeededHelpers,
    existing: &BTreeSet<JsName>,
    out: &mut Vec<LinkItem>,
) {
    if needed.grow && !existing.contains(&layout.grow_name) {
        let mut sets = String::new();
        for f in &layout.fields {
            let _ = writeln!(sets, "    nb.{c}.set(old.{c});", c = f.col_name);
        }
        out.push(synth_soa_helper(
            &layout.grow_name,
            ItemKind::Fn,
            vec![layout.alloc_name.clone()],
            format!(
                "\nfunction {}(v) {{\n  \
                 const old = v.buf;\n  \
                 const cap = old.f0.length;\n  \
                 let next = cap === 0 ? {} : cap * 2;\n  \
                 if (next > {}) {{\n    \
                   if (cap >= {}) __rt.js_abort(\"capacity overflow\");\n    \
                   next = {};\n  \
                 }}\n  \
                 const nb = {}(next);\n  \
                 if (cap > 0) {{\n\
                 {sets}  \
                 }}\n  \
                 v.buf = nb;\n  \
                 return nb;\n\
                 }}\n",
                layout.grow_name.as_str(),
                layout.min_cap,
                layout.max_elems,
                layout.max_elems,
                layout.max_elems,
                layout.alloc_name.as_str(),
            ),
        ));
    }

    if needed.reserve && !existing.contains(&layout.reserve_name) {
        let sets = format_reserve_sets(layout);
        out.push(synth_soa_helper(
            &layout.reserve_name,
            ItemKind::Fn,
            vec![layout.alloc_name.clone()],
            format!(
                "\nfunction {}(v, used, add, exact) {{\n  \
                 const req = used + add;\n  \
                 if (req > {}) __rt.js_abort(\"capacity overflow\");\n  \
                 const old = v.buf;\n  \
                 const cap = old.f0.length;\n  \
                 if (req <= cap) return;\n  \
                 let next = exact ? req : Math.max(cap * 2, req, {});\n  \
                 if (next > {}) next = {};\n  \
                 const nb = {}(next);\n  \
                 if (used > 0) {{\n\
                 {sets}  \
                 }}\n  \
                 v.buf = nb;\n\
                 }}\n",
                layout.reserve_name.as_str(),
                layout.max_elems,
                layout.min_cap,
                layout.max_elems,
                layout.max_elems,
                layout.alloc_name.as_str(),
            ),
        ));
    }

    if needed.try_reserve && !existing.contains(&layout.try_reserve_name) {
        let sets = format_reserve_sets(layout);
        out.push(synth_soa_helper(
            &layout.try_reserve_name,
            ItemKind::Fn,
            vec![layout.alloc_name.clone()],
            format!(
                "\nfunction {}(v, used, add, exact) {{\n  \
                 const req = used + add;\n  \
                 if (req > {}) return false;\n  \
                 const old = v.buf;\n  \
                 const cap = old.f0.length;\n  \
                 if (req <= cap) return true;\n  \
                 let next = exact ? req : Math.max(cap * 2, req, {});\n  \
                 if (next > {}) next = {};\n  \
                 const nb = {}(next);\n  \
                 if (used > 0) {{\n\
                 {sets}  \
                 }}\n  \
                 v.buf = nb;\n  \
                 return true;\n\
                 }}\n",
                layout.try_reserve_name.as_str(),
                layout.max_elems,
                layout.min_cap,
                layout.max_elems,
                layout.max_elems,
                layout.alloc_name.as_str(),
            ),
        ));
    }

    if needed.shrink && !existing.contains(&layout.shrink_name) {
        let mut slices = String::new();
        for (idx, f) in layout.fields.iter().enumerate() {
            if idx > 0 {
                slices.push_str(", ");
            }
            let _ = write!(slices, "{c}: old.{c}.slice(0, target)", c = f.col_name);
        }
        out.push(synth_soa_helper(
            &layout.shrink_name,
            ItemKind::Fn,
            vec![layout.empty_name.clone()],
            format!(
                "\nfunction {}(v, target) {{\n  \
                 const old = v.buf;\n  \
                 if (target >= old.f0.length) return;\n  \
                 if (target === 0) {{ v.buf = {}; return; }}\n  \
                 v.buf = {{ {slices} }};\n\
                 }}\n",
                layout.shrink_name.as_str(),
                layout.empty_name.as_str(),
            ),
        ));
    }
}

fn emit_soa_copy_helper(
    layout: &SoaLayout,
    needed: &SoaNeededHelpers,
    existing: &BTreeSet<JsName>,
    out: &mut Vec<LinkItem>,
) {
    if !needed.copy || existing.contains(&layout.copy_name) {
        return;
    }
    let mut same_buf = String::new();
    let mut diff_buf = String::new();
    for f in &layout.fields {
        let _ = writeln!(
            same_buf,
            "    db.{c}.copyWithin(doff, soff, end);",
            c = f.col_name
        );
        let _ = writeln!(
            diff_buf,
            "    db.{c}.set(sb.{c}.subarray(soff, end), doff);",
            c = f.col_name
        );
    }
    out.push(synth_soa_helper(
        &layout.copy_name,
        ItemKind::Fn,
        Vec::new(),
        format!(
            "\nfunction {}(dst, src, count) {{\n  \
             if (count <= 0) return;\n  \
             const db = dst.buf, doff = dst.off, sb = src.buf, soff = src.off;\n  \
             const end = soff + count;\n  \
             if (db === sb) {{\n    \
               if (doff === soff) return;\n\
             {same_buf}  \
             }} else {{\n\
             {diff_buf}  \
             }}\n\
             }}\n",
            layout.copy_name.as_str(),
        ),
    ));
}
