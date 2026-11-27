//! Folds byte-slice roundtrips over `__rt.bstr_bytes` directly into `.slice(...)`.

use crate::jsast::{self, Expr};

use super::super::super::js::ast_util::rt_member;

/// Folds `(h = __rt.{sfp,slice_from_parts}(offset(bstr_bytes(S), D), L), __rt.bstr_of(h.buf, h.off, h.len))`
/// (produced by `core::str::traits::SliceIndex::get_unchecked`, `str::split`, `str::lines`, etc.)
/// directly into `S.slice(D, E)` / `S.slice(D, D + L)` with zero byte-array allocation.
pub(super) fn try_fold_bstr_slice_roundtrip(expr: &Expr) -> Option<Expr> {
    let Expr::Seq(items) = expr else {
        return None;
    };
    if items.len() != 2 {
        return None;
    }
    let Expr::Assign(h_lhs, sfp_rhs) = &items[0] else {
        return None;
    };
    let Expr::Ident(h_name) = h_lhs.as_ref() else {
        return None;
    };
    if !h_name.starts_with("$t") {
        return None;
    }
    // Final expression: `__rt.bstr_of(h.buf, h.off, h.len)`
    let Expr::Call(bstr_of_callee, bstr_of_args) = &items[1] else {
        return None;
    };
    if rt_member(bstr_of_callee) != Some("bstr_of") || bstr_of_args.len() != 3 {
        return None;
    }
    let is_h_member = |e: &Expr, prop: &str| {
        matches!(
            e,
            Expr::Member(o, p) if p == prop && matches!(o.as_ref(), Expr::Ident(n) if n == h_name)
        )
    };
    if !is_h_member(&bstr_of_args[0], rcgjs_object::ptr::BUF)
        || !is_h_member(&bstr_of_args[1], rcgjs_object::ptr::OFF)
        || !is_h_member(&bstr_of_args[2], rcgjs_object::ptr::LEN)
    {
        return None;
    }
    // `sfp_rhs`: `__rt.sfp(ptr_expr, len_expr)` or `__rt.slice_from_parts(ptr_expr, len_expr)`
    let Expr::Call(sfp_callee, sfp_args) = sfp_rhs.as_ref() else {
        return None;
    };
    if !matches!(rt_member(sfp_callee), Some("sfp" | "slice_from_parts")) || sfp_args.len() != 2 {
        return None;
    }
    let (s_expr, j_temp, start_expr) = match_bstr_bytes_ptr(&sfp_args[0])?;
    let len_expr = &sfp_args[1];

    let is_simple_val = |e: &Expr| {
        matches!(
            e,
            Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_) | Expr::Str(_)
        )
    };
    let mut seq = Vec::new();
    let recv = if is_simple_val(s_expr) {
        s_expr.clone()
    } else {
        let j = j_temp?;
        seq.push(jsast::assign(jsast::id(j), s_expr.clone()));
        jsast::id(j)
    };

    let call = match start_expr {
        None => jsast::method_call(recv, "slice", vec![jsast::num(0.0), len_expr.clone()]),
        Some(start) => {
            if let Some(end) = match_end_minus_start(len_expr, start) {
                if is_simple_val(start) {
                    jsast::method_call(recv, "slice", vec![start.clone(), end.clone()])
                } else {
                    seq.push(jsast::assign(jsast::id(h_name), start.clone()));
                    jsast::method_call(recv, "slice", vec![jsast::id(h_name), end.clone()])
                }
            } else if is_simple_val(start) {
                jsast::method_call(
                    recv,
                    "slice",
                    vec![
                        start.clone(),
                        jsast::binary(jsast::BinOp::Add, start.clone(), len_expr.clone()),
                    ],
                )
            } else {
                seq.push(jsast::assign(jsast::id(h_name), start.clone()));
                jsast::method_call(
                    recv,
                    "slice",
                    vec![
                        jsast::id(h_name),
                        jsast::binary(jsast::BinOp::Add, jsast::id(h_name), len_expr.clone()),
                    ],
                )
            }
        }
    };
    if seq.is_empty() {
        Some(call)
    } else {
        seq.push(call);
        Some(Expr::Seq(seq))
    }
}

/// Extracts `(s_expr, optional_temp_name, optional_offset)` from a pointer built over
/// `__rt.bstr_bytes(s_expr)` (with or without a `+ start` offset).
fn match_bstr_bytes_ptr(expr: &Expr) -> Option<(&Expr, Option<&str>, Option<&Expr>)> {
    // Pattern A: `(g = base, D === 0 ? g : { buf: g.buf, off: g.off + D, ... })`
    // or `(g = base, { buf: g.buf, off: g.off + D, ... })`
    if let Expr::Seq(items) = expr {
        if items.len() == 2 {
            if let Expr::Assign(g_lhs, base_rhs) = &items[0] {
                if let Expr::Ident(g_name) = g_lhs.as_ref() {
                    if g_name.starts_with("$t") {
                        // Direct `{ buf: j.buf, off: j.off }` over `j = __rt.bstr_bytes(S)`
                        if let Some(s) = match_direct_bstr_bytes(base_rhs) {
                            if is_buf_off_copy(&items[1], g_name) {
                                return Some((s, Some(g_name), None));
                            }
                        }
                        // Offset over `g = base`
                        if let Some((s, j_temp)) = match_base_bstr_bytes(base_rhs) {
                            if let Some(start) = match_offset_of(&items[1], g_name) {
                                return Some((s, j_temp.or(Some(g_name)), Some(start)));
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some((s, j_temp)) = match_base_bstr_bytes(expr) {
        return Some((s, j_temp, None));
    }
    None
}

fn match_direct_bstr_bytes(expr: &Expr) -> Option<&Expr> {
    let Expr::Call(callee, args) = expr else {
        return None;
    };
    if rt_member(callee) == Some("bstr_bytes") && args.len() == 1 {
        Some(&args[0])
    } else {
        None
    }
}

fn is_buf_off_copy(expr: &Expr, var: &str) -> bool {
    let Expr::Object(entries) = expr else {
        return false;
    };
    entries.len() == 2
        && entries[0].0 == rcgjs_object::ptr::BUF
        && matches!(&entries[0].1, Expr::Member(o, p) if p == rcgjs_object::ptr::BUF && matches!(o.as_ref(), Expr::Ident(n) if n == var))
        && entries[1].0 == rcgjs_object::ptr::OFF
        && matches!(&entries[1].1, Expr::Member(o, p) if p == rcgjs_object::ptr::OFF && matches!(o.as_ref(), Expr::Ident(n) if n == var))
}

fn match_base_bstr_bytes(expr: &Expr) -> Option<(&Expr, Option<&str>)> {
    if let Some(s) = match_direct_bstr_bytes(expr) {
        return Some((s, None));
    }
    let Expr::Seq(items) = expr else {
        return None;
    };
    if items.len() != 2 {
        return None;
    }
    let Expr::Assign(j_lhs, rhs) = &items[0] else {
        return None;
    };
    let Expr::Ident(j_name) = j_lhs.as_ref() else {
        return None;
    };
    if !j_name.starts_with("$t") {
        return None;
    }
    let s = match_direct_bstr_bytes(rhs)?;
    if is_buf_off_copy(&items[1], j_name) {
        Some((s, Some(j_name)))
    } else {
        None
    }
}

fn match_offset_of<'a>(expr: &'a Expr, base: &str) -> Option<&'a Expr> {
    // Either `D === 0 ? base : { buf: base.buf, off: base.off + D, ... }`
    // or `{ buf: base.buf, off: base.off + D, ... }`.
    let obj = match expr {
        Expr::Cond(cond, then_e, else_e) => {
            if !matches!(then_e.as_ref(), Expr::Ident(n) if n == base) {
                return None;
            }
            let Expr::Binary(jsast::BinOp::StrictEq, l, r) = cond.as_ref() else {
                return None;
            };
            if !matches!(r.as_ref(), Expr::Num(n) if *n == 0.0) {
                return None;
            }
            let obj_start = match_offset_obj(else_e, base)?;
            return (l.as_ref() == obj_start).then_some(obj_start);
        }
        other => other,
    };
    match_offset_obj(obj, base)
}

fn match_offset_obj<'a>(expr: &'a Expr, base: &str) -> Option<&'a Expr> {
    let Expr::Object(entries) = expr else {
        return None;
    };
    if entries.len() < 2 || entries.len() > 3 {
        return None;
    }
    if entries[0].0 != rcgjs_object::ptr::BUF
        || !matches!(&entries[0].1, Expr::Member(o, p) if p == rcgjs_object::ptr::BUF && matches!(o.as_ref(), Expr::Ident(n) if n == base))
    {
        return None;
    }
    if entries[1].0 != rcgjs_object::ptr::OFF {
        return None;
    }
    let Expr::Binary(jsast::BinOp::Add, l, r) = &entries[1].1 else {
        return None;
    };
    if !matches!(l.as_ref(), Expr::Member(o, p) if p == rcgjs_object::ptr::OFF && matches!(o.as_ref(), Expr::Ident(n) if n == base))
    {
        return None;
    }
    if entries.len() == 3
        && (entries[2].0 != rcgjs_object::ptr::SCALE
            || !matches!(&entries[2].1, Expr::Member(o, p) if p == rcgjs_object::ptr::SCALE && matches!(o.as_ref(), Expr::Ident(n) if n == base)))
    {
        return None;
    }
    Some(r.as_ref())
}

fn match_end_minus_start<'a>(len_expr: &'a Expr, start: &Expr) -> Option<&'a Expr> {
    let inner = match len_expr {
        Expr::Binary(jsast::BinOp::UShr, l, r) if matches!(r.as_ref(), Expr::Num(n) if *n == 0.0) => {
            l.as_ref()
        }
        other => other,
    };
    let Expr::Binary(jsast::BinOp::Sub, end, sub_rhs) = inner else {
        return None;
    };
    (sub_rhs.as_ref() == start).then_some(end.as_ref())
}
