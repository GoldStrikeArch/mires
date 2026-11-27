//! Phase 2C inline `core::slice::<impl [T]>::reverse` and `core::slice::reverse::revswap`.

use rcgjs_object::jir::repr::{Op, ValueId};
use rcgjs_object::jir::semantic::Callee;
use rcgjs_object::jir::types::{TypeId, TypeKind};
use rcgjs_object::jsast::{self, Expr, Stmt};
use rcgjs_object::ptr;

use super::super::{value, JsCx, Res};
use super::Lowered;

impl JsCx<'_> {
    /// Phase 2C: Lowers authenticated `core::slice::<impl [T]>::reverse` and
    /// `core::slice::reverse::revswap` to a tight inline two-pointer swap loop when element moves
    /// cannot observe `_boxes` tracking or raw byte views.
    pub(super) fn lower_slice_reverse(
        &self,
        op: &Op,
        callee: &Callee,
        args: &[ValueId],
    ) -> Res<Option<Lowered>> {
        let Callee::Direct { item, link_name } = callee else {
            return Ok(None);
        };
        let (is_reverse, is_revswap) = if let Some(ascii) = self.ascii() {
            (ascii.is_slice_reverse(*item), ascii.is_slice_revswap(*item))
        } else {
            let core_ok = self.items.get(item).is_some_and(|e| e.is_core);
            let name = self.interns.str(*link_name).unwrap_or("");
            (
                core_ok && is_core_slice_reverse_link_name(name),
                core_ok && is_core_slice_revswap_link_name(name),
            )
        };
        if !is_reverse && !is_revswap {
            return Ok(None);
        }
        if op
            .result
            .is_some_and(|r| self.ty_of(r).ok().is_none_or(|t| !self.table.is_zst(t)))
        {
            return Ok(None);
        }
        if let Some(lowered) = self.lower_soa_slice_reverse(is_reverse, args)? {
            return Ok(Some(lowered));
        }

        if is_reverse {
            self.lower_slice_reverse_inline(args)
        } else {
            self.lower_slice_revswap_inline(args)
        }
    }

    fn slice_reverse_elem_is_zst(&self, elem_ty: TypeId) -> Option<bool> {
        let table = self.table;
        let desc = table.get(elem_ty)?;
        if !desc.layout.sized {
            return None;
        }
        if desc.layout.zst {
            return Some(true);
        }
        let peeled = table.peeled(elem_ty);
        if self.byte_views.raw_byte_derefs
            || !self.byte_views.elide_scale_scalar
            || !value::is_slot_pointee(table, peeled)
            || matches!(value::kind(table, peeled), Some(TypeKind::Array { .. }))
        {
            return None;
        }
        if table.is_indirect(peeled) && !self.can_inline_copy_one(elem_ty) {
            return None;
        }
        Some(false)
    }

    fn lower_slice_reverse_inline(&self, args: &[ValueId]) -> Res<Option<Lowered>> {
        let [slice_v] = args else {
            return Ok(None);
        };
        let table = self.table;
        let slice_ty = table.peeled(self.ty_of(*slice_v)?);
        let Some(TypeKind::Reference {
            pointee,
            mutable: true,
        }) = value::kind(table, slice_ty)
        else {
            return Ok(None);
        };
        let Some(TypeKind::Slice(elem_ty)) = value::kind(table, *pointee) else {
            return Ok(None);
        };
        let Some(is_zst) = self.slice_reverse_elem_is_zst(*elem_ty) else {
            return Ok(None);
        };
        let slice_expr = self.expr_of(*slice_v)?;
        if is_zst {
            let mut setup = Vec::new();
            if !jsast::is_droppable(&slice_expr) {
                setup.push(jsast::expr_stmt(slice_expr));
            }
            return Ok(Some(Lowered::Done(setup)));
        }

        let mut setup = Vec::new();
        let (buf_raw, off_raw, len_raw) = match slice_expr {
            Expr::Object(ref entries) if entries.len() == 3 => {
                let find = |k: &str| {
                    entries
                        .iter()
                        .find(|(ek, _)| ek == k)
                        .map(|(_, ev)| ev.clone())
                };
                if let (Some(b), Some(o), Some(l)) = (find("buf"), find("off"), find("len")) {
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
        let b = if matches!(buf_raw, Expr::Ident(_)) {
            buf_raw
        } else {
            let (bb, id) = self.temp(buf_raw);
            setup.push(bb);
            id
        };
        let off_is_zero = matches!(off_raw, Expr::Num(v) if v == 0.0);
        let (bl, l) = self.temp(off_raw);
        setup.push(bl);
        let r_init = if off_is_zero {
            jsast::binary(jsast::BinOp::Sub, len_raw, jsast::num(1))
        } else {
            jsast::binary(
                jsast::BinOp::Sub,
                jsast::binary(jsast::BinOp::Add, l.clone(), len_raw),
                jsast::num(1),
            )
        };
        let (br, r) = self.temp(r_init);
        setup.push(br);
        let (_, tmp) = self.temp(jsast::undefined());
        setup.push(jsast::while_(
            jsast::binary(jsast::BinOp::Lt, l.clone(), r.clone()),
            vec![
                jsast::assign_stmt(tmp.clone(), jsast::index(b.clone(), l.clone())),
                jsast::assign_stmt(
                    jsast::index(b.clone(), l.clone()),
                    jsast::index(b.clone(), r.clone()),
                ),
                jsast::assign_stmt(jsast::index(b, r.clone()), tmp),
                jsast::expr_stmt(jsast::compound_assign(
                    jsast::CompoundOp::Add,
                    l,
                    jsast::num(1),
                )),
                jsast::expr_stmt(jsast::compound_assign(
                    jsast::CompoundOp::Sub,
                    r,
                    jsast::num(1),
                )),
            ],
        ));
        Ok(Some(Lowered::Done(setup)))
    }

    /// `core::slice::reverse::revswap::<T>(a: *mut T, b: *mut T, n: usize)`
    fn lower_slice_revswap_inline(&self, args: &[ValueId]) -> Res<Option<Lowered>> {
        let [a_v, b_v, n_v] = args else {
            return Ok(None);
        };
        let table = self.table;
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
        let Some(is_zst) = self.slice_reverse_elem_is_zst(*elem_a) else {
            return Ok(None);
        };
        let a_expr = self.expr_of(*a_v)?;
        let b_expr = self.expr_of(*b_v)?;
        let n_expr = self.expr_of(*n_v)?;
        if is_zst {
            let mut setup = Vec::new();
            for expr in [a_expr, b_expr, n_expr] {
                if !jsast::is_droppable(&expr) {
                    setup.push(jsast::expr_stmt(expr));
                }
            }
            return Ok(Some(Lowered::Done(setup)));
        }

        let mut setup = Vec::new();
        let (ab, ao) = self.unpack_revswap_slot(a_expr, &mut setup);
        let (bb, bo) = self.unpack_revswap_slot(b_expr, &mut setup);
        let n = if matches!(n_expr, Expr::Ident(_) | Expr::Num(_)) {
            n_expr
        } else {
            let (bn, id) = self.temp(n_expr);
            setup.push(bn);
            id
        };
        let (bl, l) = self.temp(ao);
        setup.push(bl);
        let (bend, end) = self.temp(jsast::binary(jsast::BinOp::Add, l.clone(), n.clone()));
        setup.push(bend);
        let (br, r) = self.temp(jsast::binary(
            jsast::BinOp::Sub,
            jsast::binary(jsast::BinOp::Add, bo, n),
            jsast::num(1),
        ));
        setup.push(br);
        let (_, tmp) = self.temp(jsast::undefined());
        setup.push(jsast::while_(
            jsast::binary(jsast::BinOp::Lt, l.clone(), end),
            vec![
                jsast::assign_stmt(tmp.clone(), jsast::index(ab.clone(), l.clone())),
                jsast::assign_stmt(
                    jsast::index(ab, l.clone()),
                    jsast::index(bb.clone(), r.clone()),
                ),
                jsast::assign_stmt(jsast::index(bb, r.clone()), tmp),
                jsast::expr_stmt(jsast::compound_assign(
                    jsast::CompoundOp::Add,
                    l,
                    jsast::num(1),
                )),
                jsast::expr_stmt(jsast::compound_assign(
                    jsast::CompoundOp::Sub,
                    r,
                    jsast::num(1),
                )),
            ],
        ));
        Ok(Some(Lowered::Done(setup)))
    }

    fn unpack_revswap_slot(&self, slot: Expr, setup: &mut Vec<Stmt>) -> (Expr, Expr) {
        let (buf_raw, off_raw) = match slot {
            Expr::Object(ref entries) if entries.len() == 2 => {
                let find = |k: &str| {
                    entries
                        .iter()
                        .find(|(ek, _)| ek == k)
                        .map(|(_, ev)| ev.clone())
                };
                if let (Some(b), Some(o)) = (find("buf"), find("off")) {
                    (b, o)
                } else {
                    let (bs, s) = self.temp(slot);
                    setup.push(bs);
                    (ptr::slot_buf(s.clone()), ptr::slot_off(s))
                }
            }
            Expr::Ident(_) => (ptr::slot_buf(slot.clone()), ptr::slot_off(slot)),
            other => {
                let (bs, s) = self.temp(other);
                setup.push(bs);
                (ptr::slot_buf(s.clone()), ptr::slot_off(s))
            }
        };
        let b = if matches!(buf_raw, Expr::Ident(_)) {
            buf_raw
        } else {
            let (bb, id) = self.temp(buf_raw);
            setup.push(bb);
            id
        };
        (b, off_raw)
    }
}

fn has_16_hex_hash(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_core_slice_reverse_link_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("core$slice$impl_") else {
        return false;
    };
    let Some((digits, tail)) = rest.split_once("$reverse$h") else {
        return false;
    };
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) && has_16_hex_hash(tail)
}

fn is_core_slice_revswap_link_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("core$slice$reverse$revswap$h") else {
        return false;
    };
    has_16_hex_hash(rest)
}
