//! 64-bit integer (`i64` / `u64`) scalar-pair lowering to signed 32-bit `Int32` lanes.
//!
//! Inside lowered JIR functions, every `i64` and `u64` SSA value lives as two normalized signed
//! `Int32` JavaScript expressions `(lo, hi)` (where `lo` holds bits `0..31` signed via `| 0` and
//! `hi` holds bits `32..63` signed via `| 0`). Keeping both lanes strictly in signed `Int32` avoids
//! JavaScriptCore's `>>> 0` + `+` `DoubleRep` promotion cliff while matching Rust 64-bit two's
//! complement semantics identically across arithmetic, bitwise ops, shifts, rotates, comparisons,
//! overflow checks, bit intrinsics, casts, and `[u8; 8]` byte transmutations.
//!
//! `i128` and `u128` remain canonical `BigInt`, and explicit JS FFI / public export / memory /
//! aggregate boundaries convert seamlessly between `(lo, hi)` and canonical 64-bit `BigInt`.

use std::collections::BTreeSet;

use rcgjs_object::item::{ItemMap, JsName, LinkItem, Linkage};
use rcgjs_object::jir::semantic::BinaryOp;
use rcgjs_object::jir::types::{TypeId, TypeKind, TypeTable};
use rcgjs_object::jsast::{self, BinOp as JsBinOp, Expr, Stmt, UnOp as JsUnOp};
use rcgjs_object::object::ItemKind;

use super::JsCx;

pub const HELPER_HI: &str = "$__mires_i64_hi";
pub const HELPER_OVF: &str = "$__mires_i64_ovf";
pub const HELPER_MUL: &str = "$__mires_i64_mul";
pub const HELPER_MUL_OVF: &str = "$__mires_i64_mul_ovf";
pub const HELPER_DIV: &str = "$__mires_i64_div";
pub const HELPER_F2I: &str = "$__mires_i64_f2i";
pub const HELPER_F64_VIEW: &str = "$__mires_i64_f64_view";
pub const HELPER_I32_VIEW: &str = "$__mires_i64_i32_view";
pub const HELPER_F64_TO_BITS: &str = "$__mires_i64_f64_to_bits";
pub const HELPER_BITS_TO_F64: &str = "$__mires_i64_bits_to_f64";
pub const HELPER_U64_F32: &str = "$__mires_u64_f32";
pub const HELPER_I64_F32: &str = "$__mires_i64_f32";

/// All linker helper names that `i64_pair` lowering may reference.
pub const HELPER_NAMES: &[&str] = &[
    HELPER_HI,
    HELPER_OVF,
    HELPER_MUL,
    HELPER_MUL_OVF,
    HELPER_DIV,
    HELPER_F2I,
    HELPER_F64_VIEW,
    HELPER_I32_VIEW,
    HELPER_F64_TO_BITS,
    HELPER_BITS_TO_F64,
    HELPER_U64_F32,
    HELPER_I64_F32,
];

/// Returns `Some(signed)` if `ty` peels to a 64-bit integer (`i64` or `u64`), `None` otherwise
/// (including `i128` / `u128`, smaller integers, enums, structs, etc.).
pub fn i64_signedness(table: &TypeTable, ty: TypeId) -> Option<bool> {
    if let Some((signed, 64)) = table.int_info(ty) {
        return Some(signed);
    }
    match table.get(table.peeled(ty)).map(|d| &d.kind) {
        Some(TypeKind::Integer { signed, bits: 64 }) => Some(*signed),
        _ => None,
    }
}

/// Whether `ty` is lowered as a signed-`Int32` `(lo, hi)` pair inside JIR functions.
pub fn is_i64(table: &TypeTable, ty: TypeId) -> bool {
    i64_signedness(table, ty).is_some()
}

/// The `(lo, hi)` variable names for a named 64-bit SSA binding or temporary.
pub fn pair_names(base: &str) -> (String, String) {
    (format!("{base}$lo"), format!("{base}$hi"))
}

/// Splits a raw 64-bit bit pattern into normalized signed `Int32` literals `(lo, hi)`.
pub fn literal_pair(raw: u128) -> (Expr, Expr) {
    let lo = (raw as u32) as i32;
    let hi = ((raw as u64) >> 32) as i32;
    (jsast::num(f64::from(lo)), jsast::num(f64::from(hi)))
}

/// If `expr` is an integer numeric literal in signed/unsigned 32-bit range, returns its `i32` bits.
pub fn const_i32(expr: &Expr) -> Option<i32> {
    match expr {
        Expr::Num(n) if n.is_finite() && n.fract() == 0.0 => {
            let v = *n as i64;
            i32::try_from(v)
                .ok()
                .or_else(|| u32::try_from(v).ok().map(|u| u as i32))
        }
        Expr::RawNum(s) => s
            .parse::<i32>()
            .ok()
            .or_else(|| s.parse::<u32>().ok().map(|u| u as i32)),
        Expr::Unary(JsUnOp::Neg, inner) => const_i32(inner).map(i32::wrapping_neg),
        Expr::Unary(JsUnOp::BitNot, inner) => const_i32(inner).map(|v| !v),
        Expr::Binary(JsBinOp::BitOr, lhs, rhs) if const_i32(rhs) == Some(0) => const_i32(lhs),
        _ => None,
    }
}

/// Whether `expr` is safe to evaluate multiple times without side effects or cost.
pub fn is_trivial(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_) | Expr::Bool(_) | Expr::Undefined
    )
}

/// Captures `expr` into a `$tN` temporary in `stmts` unless it is already trivial.
pub fn materialize(cx: &JsCx<'_>, stmts: &mut Vec<Stmt>, expr: Expr) -> Expr {
    if is_trivial(&expr) {
        expr
    } else {
        let (bind, tmp) = cx.temp(expr);
        stmts.push(bind);
        tmp
    }
}

/// Materializes `(lo, hi)` only when either lane is non-trivial.
pub fn materialize_pair(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    (lo, hi): (Expr, Expr),
) -> (Expr, Expr) {
    (materialize(cx, stmts, lo), materialize(cx, stmts, hi))
}

/// Packs normalized signed-`Int32` `(lo, hi)` into a canonical 64-bit JS `BigInt`.
///
/// Uses `BigInt("32")` under `EcmaTarget::Es2018` via `jsast::bigint(32u32)` so both ES2018 and
/// latest JS syntax targets validate cleanly.
pub fn pack_bigint(signed: bool, lo: Expr, hi: Expr) -> Expr {
    let lo_u32 = jsast::call(jsast::id("BigInt"), vec![jsast::u32_wrap(lo)]);
    let hi_word = if signed { hi } else { jsast::u32_wrap(hi) };
    let hi_big = jsast::call(jsast::id("BigInt"), vec![hi_word]);
    let shifted_hi = jsast::binary(JsBinOp::Shl, hi_big, jsast::bigint(32u32));
    jsast::binary(JsBinOp::BitOr, shifted_hi, lo_u32)
}

/// Extracts `(lo_expr, hi_expr)` from a canonical `BigInt` expression `src` (already trivial).
pub fn unpack_bigint_exprs(src: Expr) -> (Expr, Expr) {
    let as_int_n = |arg: Expr| {
        jsast::call(
            jsast::member(jsast::id("BigInt"), "asIntN"),
            vec![jsast::num(32.0), arg],
        )
    };
    let lo = jsast::i32_wrap(jsast::to_number(as_int_n(src.clone())));
    let hi_shifted = jsast::binary(JsBinOp::Shr, src, jsast::bigint(32u32));
    let hi = jsast::i32_wrap(jsast::to_number(as_int_n(hi_shifted)));
    (lo, hi)
}

/// Emits statements unpacking a canonical `BigInt` expression `src` into `(dst_lo, dst_hi)`.
#[allow(dead_code)]
pub fn emit_unpack_bigint(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    src: Expr,
    dst_lo: &str,
    dst_hi: &str,
) {
    let src = materialize(cx, stmts, src);
    let (lo, hi) = unpack_bigint_exprs(src);
    stmts.push(jsast::assign_stmt(jsast::id(dst_lo), lo));
    stmts.push(jsast::assign_stmt(jsast::id(dst_hi), hi));
}

fn expr_mentions_ident(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Ident(id) => id == name,
        Expr::Member(obj, _) => expr_mentions_ident(obj, name),
        Expr::Index(obj, idx) => expr_mentions_ident(obj, name) || expr_mentions_ident(idx, name),
        Expr::Call(callee, args) | Expr::New(callee, args) => {
            expr_mentions_ident(callee, name) || args.iter().any(|a| expr_mentions_ident(a, name))
        }
        Expr::Unary(_, inner) => expr_mentions_ident(inner, name),
        Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
            expr_mentions_ident(l, name) || expr_mentions_ident(r, name)
        }
        Expr::Cond(c, t, e) => {
            expr_mentions_ident(c, name)
                || expr_mentions_ident(t, name)
                || expr_mentions_ident(e, name)
        }
        Expr::Object(fields) => fields.iter().any(|(_, v)| expr_mentions_ident(v, name)),
        Expr::Array(items) | Expr::Seq(items) => items.iter().any(|e| expr_mentions_ident(e, name)),
        Expr::Arrow(_, _) | Expr::Raw(_) => true,
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null => false,
    }
}

fn expr_has_call(expr: &Expr) -> bool {
    match expr {
        Expr::Call(_, _) | Expr::New(_, _) | Expr::Raw(_) => true,
        Expr::Member(obj, _) => expr_has_call(obj),
        Expr::Index(obj, idx) => expr_has_call(obj) || expr_has_call(idx),
        Expr::Unary(_, inner) => expr_has_call(inner),
        Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
            expr_has_call(l) || expr_has_call(r)
        }
        Expr::Cond(c, t, e) => expr_has_call(c) || expr_has_call(t) || expr_has_call(e),
        Expr::Object(fields) => fields.iter().any(|(_, v)| expr_has_call(v)),
        Expr::Array(items) | Expr::Seq(items) => items.iter().any(expr_has_call),
        Expr::Arrow(_, _)
        | Expr::Ident(_)
        | Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null => false,
    }
}

/// Assigns `(lo, hi)` into `(dst_lo, dst_hi)` without clobbering either lane if the other lane's
/// expression reads it (for example in-place `x = x + y` or `x = x.rotate_left(k)`).
pub fn emit_assign_pair(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    dst_lo: &str,
    dst_hi: &str,
    (lo, hi): (Expr, Expr),
) {
    let hi_reads_dst_lo = expr_mentions_ident(&hi, dst_lo);
    if !hi_reads_dst_lo {
        stmts.push(jsast::assign_stmt(jsast::id(dst_lo), lo));
        stmts.push(jsast::assign_stmt(jsast::id(dst_hi), hi));
        return;
    }
    if expr_has_call(&lo) {
        let (bind_lo, tmp_lo) = cx.temp(lo);
        stmts.push(bind_lo);
        stmts.push(jsast::assign_stmt(jsast::id(dst_hi), hi));
        stmts.push(jsast::assign_stmt(jsast::id(dst_lo), tmp_lo));
        return;
    }
    if !expr_mentions_ident(&lo, dst_hi) {
        stmts.push(jsast::assign_stmt(jsast::id(dst_hi), hi));
        stmts.push(jsast::assign_stmt(jsast::id(dst_lo), lo));
        return;
    }
    let (bind_hi, tmp_hi) = cx.temp(hi);
    stmts.push(bind_hi);
    stmts.push(jsast::assign_stmt(jsast::id(dst_lo), lo));
    stmts.push(jsast::assign_stmt(jsast::id(dst_hi), tmp_hi));
}

/// Computes `(lo, hi)` for `Add`, emitting statements into `stmts` and returning the two `Expr`s.
pub fn add_pair(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    (al, ah): (Expr, Expr),
    (bl, bh): (Expr, Expr),
) -> (Expr, Expr) {
    if let (Some(alo), Some(ahi), Some(blo), Some(bhi)) = (
        const_i32(&al),
        const_i32(&ah),
        const_i32(&bl),
        const_i32(&bh),
    ) {
        let av = u64::from(alo as u32) | (u64::from(ahi as u32) << 32);
        let bv = u64::from(blo as u32) | (u64::from(bhi as u32) << 32);
        return literal_pair(u128::from(av.wrapping_add(bv)));
    }
    if const_i32(&bl) == Some(0) && const_i32(&bh) == Some(0) {
        return (al, ah);
    }
    if const_i32(&al) == Some(0) && const_i32(&ah) == Some(0) {
        return (bl, bh);
    }
    let al = materialize(cx, stmts, al);
    let sum_lo = jsast::i32_wrap(jsast::binary(JsBinOp::Add, al.clone(), bl));
    let (bind_lo, lo_var) = cx.temp(sum_lo);
    stmts.push(bind_lo);
    let carry = jsast::cond(
        jsast::binary(
            JsBinOp::Lt,
            jsast::u32_wrap(lo_var.clone()),
            jsast::u32_wrap(al),
        ),
        jsast::num(1.0),
        jsast::num(0.0),
    );
    let hi_sum = if const_i32(&bh) == Some(0) {
        jsast::binary(JsBinOp::Add, ah, carry)
    } else if const_i32(&ah) == Some(0) {
        jsast::binary(JsBinOp::Add, bh, carry)
    } else {
        jsast::binary(JsBinOp::Add, jsast::binary(JsBinOp::Add, ah, bh), carry)
    };
    (lo_var, jsast::i32_wrap(hi_sum))
}

/// Computes `(lo, hi)` for `Sub`.
pub fn sub_pair(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    (al, ah): (Expr, Expr),
    (bl, bh): (Expr, Expr),
) -> (Expr, Expr) {
    if let (Some(alo), Some(ahi), Some(blo), Some(bhi)) = (
        const_i32(&al),
        const_i32(&ah),
        const_i32(&bl),
        const_i32(&bh),
    ) {
        let av = u64::from(alo as u32) | (u64::from(ahi as u32) << 32);
        let bv = u64::from(blo as u32) | (u64::from(bhi as u32) << 32);
        return literal_pair(u128::from(av.wrapping_sub(bv)));
    }
    if const_i32(&bl) == Some(0) && const_i32(&bh) == Some(0) {
        return (al, ah);
    }
    let al = materialize(cx, stmts, al);
    let bl = materialize(cx, stmts, bl);
    let lo = jsast::i32_wrap(jsast::binary(JsBinOp::Sub, al.clone(), bl.clone()));
    let borrow = jsast::cond(
        jsast::binary(JsBinOp::Lt, jsast::u32_wrap(al), jsast::u32_wrap(bl)),
        jsast::num(1.0),
        jsast::num(0.0),
    );
    let hi_base = if const_i32(&bh) == Some(0) {
        ah
    } else {
        jsast::binary(JsBinOp::Sub, ah, bh)
    };
    let hi = jsast::i32_wrap(jsast::binary(JsBinOp::Sub, hi_base, borrow));
    (lo, hi)
}

/// Computes `(lo, hi)` for two's complement `Neg` (`-x`).
pub fn neg_pair(cx: &JsCx<'_>, stmts: &mut Vec<Stmt>, (al, ah): (Expr, Expr)) -> (Expr, Expr) {
    if let (Some(alo), Some(ahi)) = (const_i32(&al), const_i32(&ah)) {
        let av = u64::from(alo as u32) | (u64::from(ahi as u32) << 32);
        return literal_pair(u128::from(av.wrapping_neg()));
    }
    let lo = jsast::i32_wrap(jsast::neg(al));
    let lo_var = materialize(cx, stmts, lo);
    let carry = jsast::cond(
        jsast::binary(JsBinOp::StrictEq, lo_var.clone(), jsast::num(0.0)),
        jsast::num(1.0),
        jsast::num(0.0),
    );
    let hi = jsast::i32_wrap(jsast::binary(JsBinOp::Add, jsast::bit_not(ah), carry));
    (lo_var, hi)
}

/// Computes `(lo, hi)` for 64-bit `Mul`, using `$__mires_i64_mul` + `$__mires_i64_hi`.
pub fn mul_pair(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    (al, ah): (Expr, Expr),
    (bl, bh): (Expr, Expr),
) -> (Expr, Expr) {
    if let (Some(alo), Some(ahi), Some(blo), Some(bhi)) = (
        const_i32(&al),
        const_i32(&ah),
        const_i32(&bl),
        const_i32(&bh),
    ) {
        let av = u64::from(alo as u32) | (u64::from(ahi as u32) << 32);
        let bv = u64::from(blo as u32) | (u64::from(bhi as u32) << 32);
        return literal_pair(u128::from(av.wrapping_mul(bv)));
    }
    if (const_i32(&al) == Some(0) && const_i32(&ah) == Some(0))
        || (const_i32(&bl) == Some(0) && const_i32(&bh) == Some(0))
    {
        discard_effect(stmts, al);
        discard_effect(stmts, ah);
        discard_effect(stmts, bl);
        discard_effect(stmts, bh);
        return (jsast::num(0.0), jsast::num(0.0));
    }
    if const_i32(&bl) == Some(1) && const_i32(&bh) == Some(0) {
        return (al, ah);
    }
    if const_i32(&al) == Some(1) && const_i32(&ah) == Some(0) {
        return (bl, bh);
    }
    if let Some((var, c)) = small_mul_const(&bl, &bh)
        .map(|c| ((al.clone(), ah.clone()), c))
        .or_else(|| small_mul_const(&al, &ah).map(|c| ((bl.clone(), bh.clone()), c)))
    {
        if c.is_power_of_two() {
            return shl_pair(cx, stmts, var, jsast::num(f64::from(c.trailing_zeros())));
        }
        let (vl, vh) = var;
        let vl = materialize(cx, stmts, vl);
        let c_expr = jsast::num(f64::from(c));
        let a0 = jsast::binary(JsBinOp::BitAnd, vl.clone(), jsast::num(65535.0));
        let a1 = jsast::binary(JsBinOp::UShr, vl, jsast::num(16.0));
        let p0 = materialize(
            cx,
            stmts,
            jsast::call(
                jsast::member(jsast::id("Math"), "imul"),
                vec![a0, c_expr.clone()],
            ),
        );
        let p1_sum = jsast::binary(
            JsBinOp::Add,
            jsast::call(
                jsast::member(jsast::id("Math"), "imul"),
                vec![a1, c_expr.clone()],
            ),
            jsast::binary(JsBinOp::UShr, p0.clone(), jsast::num(16.0)),
        );
        let p1 = materialize(cx, stmts, jsast::i32_wrap(p1_sum));
        let lo = jsast::binary(
            JsBinOp::BitOr,
            jsast::binary(JsBinOp::BitAnd, p0, jsast::num(65535.0)),
            jsast::binary(JsBinOp::Shl, p1.clone(), jsast::num(16.0)),
        );
        let hi = jsast::i32_wrap(jsast::binary(
            JsBinOp::Add,
            jsast::call(jsast::member(jsast::id("Math"), "imul"), vec![vh, c_expr]),
            jsast::binary(JsBinOp::UShr, p1, jsast::num(16.0)),
        ));
        return (lo, hi);
    }
    let call = jsast::call(jsast::id(HELPER_MUL), vec![al, ah, bl, bh]);
    let (bind, lo_var) = cx.temp(call);
    stmts.push(bind);
    (lo_var, jsast::id(HELPER_HI))
}

fn small_mul_const(lo: &Expr, hi: &Expr) -> Option<u32> {
    if const_i32(hi) != Some(0) {
        return None;
    }
    let c = const_i32(lo)? as u32;
    (c <= 0xffff).then_some(c)
}

/// Computes `(lo, hi)` for `Div` or `Rem` via `$__mires_i64_div`.
pub fn div_rem_pair(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    signed: bool,
    is_rem: bool,
    (al, ah): (Expr, Expr),
    (bl, bh): (Expr, Expr),
) -> (Expr, Expr) {
    let call = jsast::call(
        jsast::id(HELPER_DIV),
        vec![
            al,
            ah,
            bl,
            bh,
            jsast::boolean(signed),
            jsast::boolean(is_rem),
        ],
    );
    let (bind, lo_var) = cx.temp(call);
    stmts.push(bind);
    (lo_var, jsast::id(HELPER_HI))
}

/// Emits `expr` as a statement when it may have side effects (calls, assignments, raw JS).
fn discard_effect(stmts: &mut Vec<Stmt>, expr: Expr) {
    if !jsast::is_droppable(&expr) {
        stmts.push(Stmt::ExprStmt(expr));
    }
}

/// Returns `val` directly if `dropped` is droppable, otherwise `(dropped, val)`.
fn keep_effect(dropped: Expr, val: Expr) -> Expr {
    if jsast::is_droppable(&dropped) {
        val
    } else {
        jsast::seq(vec![dropped, val])
    }
}

/// Computes `(lo, hi)` for bitwise `&`, `|`, `^`.
pub fn bitwise_pair(op: JsBinOp, (al, ah): (Expr, Expr), (bl, bh): (Expr, Expr)) -> (Expr, Expr) {
    let half = |l: Expr, r: Expr| match (op, const_i32(&l), const_i32(&r)) {
        (JsBinOp::BitAnd, Some(0), _) => keep_effect(r, jsast::num(0.0)),
        (JsBinOp::BitAnd, _, Some(0)) => keep_effect(l, jsast::num(0.0)),
        (JsBinOp::BitAnd, Some(-1), _) => r,
        (JsBinOp::BitAnd, _, Some(-1)) => l,
        (JsBinOp::BitOr, Some(0), _) => r,
        (JsBinOp::BitOr, _, Some(0)) => l,
        (JsBinOp::BitOr, Some(-1), _) => keep_effect(r, jsast::num(-1.0)),
        (JsBinOp::BitOr, _, Some(-1)) => keep_effect(l, jsast::num(-1.0)),
        (JsBinOp::BitXor, Some(0), _) => r,
        (JsBinOp::BitXor, _, Some(0)) => l,
        (JsBinOp::BitXor, Some(-1), _) => jsast::bit_not(r),
        (JsBinOp::BitXor, _, Some(-1)) => jsast::bit_not(l),
        (_, Some(lv), Some(rv)) => {
            let v = match op {
                JsBinOp::BitAnd => lv & rv,
                JsBinOp::BitOr => lv | rv,
                JsBinOp::BitXor => lv ^ rv,
                _ => unreachable!(),
            };
            jsast::num(f64::from(v))
        }
        _ => jsast::binary(op, l, r),
    };
    (half(al, bl), half(ah, bh))
}

/// Computes `(lo, hi)` for bitwise `Not` (`~x`).
pub fn bit_not_pair((al, ah): (Expr, Expr)) -> (Expr, Expr) {
    let half = |x: Expr| match const_i32(&x) {
        Some(v) => jsast::num(f64::from(!v)),
        None => jsast::bit_not(x),
    };
    (half(al), half(ah))
}

/// Extracts shift amount masked `& 63` as a scalar Int32 expression.
fn mask_shift_63(amount: Expr, amount_is_i64: bool, amount_is_bigint: bool) -> Expr {
    if amount_is_i64 {
        jsast::binary(JsBinOp::BitAnd, amount, jsast::num(63.0))
    } else if amount_is_bigint {
        jsast::binary(
            JsBinOp::BitAnd,
            jsast::i32_wrap(jsast::to_number(jsast::bigint_mask(false, 6, amount))),
            jsast::num(63.0),
        )
    } else {
        jsast::binary(JsBinOp::BitAnd, amount, jsast::num(63.0))
    }
}

fn fold_shl(lhs: Expr, rhs: Expr) -> Expr {
    if const_i32(&lhs) == Some(0) {
        return keep_effect(rhs, jsast::num(0.0));
    }
    if let Some(k) = const_i32(&rhs) {
        let s = (k as u32) & 31;
        if let Some(v) = const_i32(&lhs) {
            return jsast::num(f64::from(v.wrapping_shl(s)));
        }
        if s == 0 {
            return lhs;
        }
        return jsast::binary(JsBinOp::Shl, lhs, jsast::num(f64::from(s)));
    }
    jsast::binary(JsBinOp::Shl, lhs, rhs)
}

fn fold_ushr_pos_shift(lhs: Expr, rhs: Expr) -> Expr {
    if const_i32(&lhs) == Some(0) {
        return keep_effect(rhs, jsast::num(0.0));
    }
    if let Some(k) = const_i32(&rhs) {
        let s = (k as u32) & 31;
        if let Some(v) = const_i32(&lhs) {
            return jsast::num(f64::from(((v as u32) >> s) as i32));
        }
        return jsast::binary(JsBinOp::UShr, lhs, jsast::num(f64::from(s)));
    }
    jsast::binary(JsBinOp::UShr, lhs, rhs)
}

fn fold_ushr_i32(lhs: Expr, rhs: Expr) -> Expr {
    if const_i32(&lhs) == Some(0) {
        return keep_effect(rhs, jsast::num(0.0));
    }
    if let Some(k) = const_i32(&rhs) {
        let s = (k as u32) & 31;
        if let Some(v) = const_i32(&lhs) {
            return jsast::num(f64::from(((v as u32) >> s) as i32));
        }
        if s == 0 {
            return lhs;
        }
        return jsast::binary(JsBinOp::UShr, lhs, jsast::num(f64::from(s)));
    }
    jsast::i32_wrap(jsast::binary(JsBinOp::UShr, lhs, rhs))
}

fn fold_shr_signed(lhs: Expr, rhs: Expr) -> Expr {
    match const_i32(&lhs) {
        Some(0) => return keep_effect(rhs, jsast::num(0.0)),
        Some(-1) => return keep_effect(rhs, jsast::num(-1.0)),
        _ => {}
    }
    if let Some(k) = const_i32(&rhs) {
        let s = (k as u32) & 31;
        if let Some(v) = const_i32(&lhs) {
            return jsast::num(f64::from(v >> s));
        }
        if s == 0 {
            return lhs;
        }
        return jsast::binary(JsBinOp::Shr, lhs, jsast::num(f64::from(s)));
    }
    jsast::binary(JsBinOp::Shr, lhs, rhs)
}

fn fold_bit_or_i32(lhs: Expr, rhs: Expr) -> Expr {
    match (const_i32(&lhs), const_i32(&rhs)) {
        (Some(0), _) => rhs,
        (_, Some(0)) => lhs,
        (Some(-1), _) => keep_effect(rhs, jsast::num(-1.0)),
        (_, Some(-1)) => keep_effect(lhs, jsast::num(-1.0)),
        (Some(a), Some(b)) => jsast::num(f64::from(a | b)),
        _ => jsast::binary(JsBinOp::BitOr, lhs, rhs),
    }
}

fn fold_cond_i32(cond: Expr, then_expr: Expr, else_expr: Expr) -> Expr {
    if let (Some(a), Some(b)) = (const_i32(&then_expr), const_i32(&else_expr)) {
        if a == b {
            return keep_effect(cond, then_expr);
        }
    }
    jsast::cond(cond, then_expr, else_expr)
}

/// Computes `(lo, hi)` for 64-bit left shift `(a << (amount & 63))`.
pub fn shl_pair(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    (al, ah): (Expr, Expr),
    shift_amount: Expr,
) -> (Expr, Expr) {
    if let Some(k) = const_i32(&shift_amount).map(|s| (s & 63) as u32) {
        if k == 0 {
            return (al, ah);
        }
        if let (Some(alo), Some(ahi)) = (const_i32(&al), const_i32(&ah)) {
            let av = u64::from(alo as u32) | (u64::from(ahi as u32) << 32);
            return literal_pair(u128::from(av.wrapping_shl(k)));
        }
        if k < 32 {
            if const_i32(&al) == Some(0) {
                return (jsast::num(0.0), fold_shl(ah, jsast::num(f64::from(k))));
            }
            let al = materialize(cx, stmts, al);
            let lo = fold_shl(al.clone(), jsast::num(f64::from(k)));
            let hi = fold_bit_or_i32(
                fold_shl(ah, jsast::num(f64::from(k))),
                fold_ushr_pos_shift(al, jsast::num(f64::from(32 - k))),
            );
            return (lo, hi);
        }
        if k == 32 {
            if !jsast::is_droppable(&ah) {
                let al = materialize(cx, stmts, al);
                stmts.push(Stmt::ExprStmt(ah));
                return (jsast::num(0.0), al);
            }
            return (jsast::num(0.0), al);
        }
        if !jsast::is_droppable(&ah) {
            let al = materialize(cx, stmts, al);
            stmts.push(Stmt::ExprStmt(ah));
            let hi = fold_shl(al, jsast::num(f64::from(k - 32)));
            return (jsast::num(0.0), hi);
        }
        let hi = fold_shl(al, jsast::num(f64::from(k - 32)));
        return (jsast::num(0.0), hi);
    }
    if const_i32(&al) == Some(0) && const_i32(&ah) == Some(0) {
        discard_effect(stmts, shift_amount);
        return (jsast::num(0.0), jsast::num(0.0));
    }
    let al = materialize(cx, stmts, al);
    let ah = materialize(cx, stmts, ah);
    let k = materialize(cx, stmts, mask_shift_63(shift_amount, true, false));
    let high_flag = jsast::binary(
        JsBinOp::StrictNe,
        jsast::binary(JsBinOp::BitAnd, k.clone(), jsast::num(32.0)),
        jsast::num(0.0),
    );
    let lo = fold_cond_i32(
        high_flag.clone(),
        jsast::num(0.0),
        fold_shl(al.clone(), k.clone()),
    );
    // `(al >>> 1) >>> (31 - k)` equals `al >>> (32 - k)` for `1..=31` and `0` when `k == 0`
    // because `(al >>> 1)` has only 31 significant bits, avoiding JS `x >>> 32 == x`.
    let al_shr1 = fold_ushr_pos_shift(al.clone(), jsast::num(1.0));
    let cross = fold_ushr_pos_shift(
        al_shr1,
        jsast::binary(JsBinOp::Sub, jsast::num(31.0), k.clone()),
    );
    let hi_lt32 = fold_bit_or_i32(fold_shl(ah, k.clone()), cross);
    let hi = fold_cond_i32(high_flag, fold_shl(al, k), hi_lt32);
    (lo, hi)
}

/// Computes `(lo, hi)` for 64-bit right shift (`>>` arithmetic if `signed`, `>>>` logical else).
pub fn shr_pair(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    signed: bool,
    (al, ah): (Expr, Expr),
    shift_amount: Expr,
) -> (Expr, Expr) {
    let shift_hi = |expr: Expr, amt: Expr| {
        if signed {
            fold_shr_signed(expr, amt)
        } else {
            fold_ushr_i32(expr, amt)
        }
    };
    let fill_hi = |ah_expr: Expr| {
        if signed {
            fold_shr_signed(ah_expr, jsast::num(31.0))
        } else {
            jsast::num(0.0)
        }
    };

    if let Some(k) = const_i32(&shift_amount).map(|s| (s & 63) as u32) {
        if k == 0 {
            return (al, ah);
        }
        if let (Some(alo), Some(ahi)) = (const_i32(&al), const_i32(&ah)) {
            let av = u64::from(alo as u32) | (u64::from(ahi as u32) << 32);
            let shifted = if signed {
                (av as i64).wrapping_shr(k) as u64
            } else {
                av.wrapping_shr(k)
            };
            return literal_pair(u128::from(shifted));
        }
        if k < 32 {
            let ah = materialize(cx, stmts, ah);
            let lo = fold_bit_or_i32(
                fold_ushr_pos_shift(al, jsast::num(f64::from(k))),
                fold_shl(ah.clone(), jsast::num(f64::from(32 - k))),
            );
            let hi = shift_hi(ah, jsast::num(f64::from(k)));
            return (lo, hi);
        }
        discard_effect(stmts, al);
        if k == 32 {
            let ah = materialize(cx, stmts, ah);
            return (ah.clone(), fill_hi(ah));
        }
        let ah = materialize(cx, stmts, ah);
        let lo = shift_hi(ah.clone(), jsast::num(f64::from(k - 32)));
        let hi = fill_hi(ah);
        return (lo, hi);
    }

    if const_i32(&al) == Some(0) && const_i32(&ah) == Some(0) {
        discard_effect(stmts, shift_amount);
        return (jsast::num(0.0), jsast::num(0.0));
    }
    let al = materialize(cx, stmts, al);
    let ah = materialize(cx, stmts, ah);
    let k = materialize(cx, stmts, mask_shift_63(shift_amount, true, false));
    let high_flag = jsast::binary(
        JsBinOp::StrictNe,
        jsast::binary(JsBinOp::BitAnd, k.clone(), jsast::num(32.0)),
        jsast::num(0.0),
    );
    // `(ah << 1) << (31 - k)` equals `ah << (32 - k)` for `1..=31` and `0` when `k == 0`.
    let ah_shl1 = fold_shl(ah.clone(), jsast::num(1.0));
    let cross = fold_shl(
        ah_shl1,
        jsast::binary(JsBinOp::Sub, jsast::num(31.0), k.clone()),
    );
    let lo_lt32 = if const_i32(&cross) == Some(0) {
        fold_ushr_i32(al, k.clone())
    } else if const_i32(&al) == Some(0) {
        cross
    } else {
        jsast::binary(
            JsBinOp::BitOr,
            jsast::binary(JsBinOp::UShr, al, k.clone()),
            cross,
        )
    };
    let lo = fold_cond_i32(high_flag.clone(), shift_hi(ah.clone(), k.clone()), lo_lt32);
    let hi = fold_cond_i32(high_flag, fill_hi(ah.clone()), shift_hi(ah, k));
    (lo, hi)
}

/// Computes `(lo, hi)` for `rotate_left` or `rotate_right`.
pub fn rotate_pair(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    left: bool,
    (al, ah): (Expr, Expr),
    shift_amount: Expr,
) -> (Expr, Expr) {
    if let Some(raw) = const_i32(&shift_amount) {
        let k = if left {
            (raw & 63) as u32
        } else {
            ((64 - (raw & 63)) & 63) as u32
        };
        if k == 0 {
            return (al, ah);
        }
        if let (Some(alo), Some(ahi)) = (const_i32(&al), const_i32(&ah)) {
            let av = u64::from(alo as u32) | (u64::from(ahi as u32) << 32);
            return literal_pair(u128::from(av.rotate_left(k)));
        }
        if k == 32 {
            return (ah, al);
        }
        let al = materialize(cx, stmts, al);
        let ah = materialize(cx, stmts, ah);
        let (a, b, r) = if k < 32 {
            (al, ah, k)
        } else {
            (ah, al, k - 32)
        };
        let lo = fold_bit_or_i32(
            fold_shl(a.clone(), jsast::num(f64::from(r))),
            fold_ushr_pos_shift(b.clone(), jsast::num(f64::from(32 - r))),
        );
        let hi = fold_bit_or_i32(
            fold_shl(b, jsast::num(f64::from(r))),
            fold_ushr_pos_shift(a, jsast::num(f64::from(32 - r))),
        );
        return (lo, hi);
    }

    if const_i32(&al) == Some(0) && const_i32(&ah) == Some(0) {
        discard_effect(stmts, shift_amount);
        return (jsast::num(0.0), jsast::num(0.0));
    }
    let al = materialize(cx, stmts, al);
    let ah = materialize(cx, stmts, ah);
    let raw_masked = mask_shift_63(shift_amount, true, false);
    let k = materialize(
        cx,
        stmts,
        if left {
            raw_masked
        } else {
            jsast::binary(JsBinOp::BitAnd, jsast::neg(raw_masked), jsast::num(63.0))
        },
    );
    let m = materialize(
        cx,
        stmts,
        jsast::binary(JsBinOp::BitAnd, k.clone(), jsast::num(31.0)),
    );
    let inv_m = materialize(
        cx,
        stmts,
        jsast::binary(JsBinOp::Sub, jsast::num(31.0), m.clone()),
    );
    let rot = |x: Expr, y: Expr| {
        let y_shr1 = fold_ushr_pos_shift(y, jsast::num(1.0));
        fold_bit_or_i32(
            fold_shl(x, m.clone()),
            fold_ushr_pos_shift(y_shr1, inv_m.clone()),
        )
    };
    let r_lo = materialize(cx, stmts, rot(al.clone(), ah.clone()));
    let r_hi = materialize(cx, stmts, rot(ah, al));
    let swap = jsast::binary(
        JsBinOp::StrictNe,
        jsast::binary(JsBinOp::BitAnd, k, jsast::num(32.0)),
        jsast::num(0.0),
    );
    let swap = materialize(cx, stmts, swap);
    let lo = fold_cond_i32(swap.clone(), r_hi.clone(), r_lo.clone());
    let hi = fold_cond_i32(swap, r_lo, r_hi);
    (lo, hi)
}

/// Computes a boolean comparison between two 64-bit `(lo, hi)` pairs.
pub fn cmp_pair(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    op: BinaryOp,
    signed: bool,
    (al, ah): (Expr, Expr),
    (bl, bh): (Expr, Expr),
) -> Expr {
    if matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
        let al_zero = const_i32(&al) == Some(0);
        let ah_zero = const_i32(&ah) == Some(0);
        let bl_zero = const_i32(&bl) == Some(0);
        let bh_zero = const_i32(&bh) == Some(0);
        if (bl_zero && bh_zero) || (al_zero && ah_zero) {
            let (xl, xh) = if bl_zero && bh_zero {
                (al, ah)
            } else {
                (bl, bh)
            };
            let cmp = if op == BinaryOp::Eq {
                JsBinOp::StrictEq
            } else {
                JsBinOp::StrictNe
            };
            return jsast::binary(cmp, jsast::binary(JsBinOp::BitOr, xl, xh), jsast::num(0.0));
        }
        let (cmp_op, join_op) = if op == BinaryOp::Eq {
            (JsBinOp::StrictEq, JsBinOp::And)
        } else {
            (JsBinOp::StrictNe, JsBinOp::Or)
        };
        return jsast::binary(
            join_op,
            jsast::binary(cmp_op, al, bl),
            jsast::binary(cmp_op, ah, bh),
        );
    }

    let ah = materialize(cx, stmts, ah);
    let bh = materialize(cx, stmts, bh);
    let js_ord = match op {
        BinaryOp::Lt => JsBinOp::Lt,
        BinaryOp::Le => JsBinOp::Le,
        BinaryOp::Gt => JsBinOp::Gt,
        BinaryOp::Ge => JsBinOp::Ge,
        _ => unreachable!(),
    };
    let hi_strict = match op {
        BinaryOp::Lt | BinaryOp::Le => JsBinOp::Lt,
        BinaryOp::Gt | BinaryOp::Ge => JsBinOp::Gt,
        _ => unreachable!(),
    };
    let hi_eq = jsast::binary(JsBinOp::StrictNe, ah.clone(), bh.clone());
    let hi_cmp = if signed {
        jsast::binary(hi_strict, ah, bh)
    } else {
        jsast::binary(hi_strict, jsast::u32_wrap(ah), jsast::u32_wrap(bh))
    };
    let lo_cmp = jsast::binary(js_ord, jsast::u32_wrap(al), jsast::u32_wrap(bl));
    jsast::cond(hi_eq, hi_cmp, lo_cmp)
}

/// Converts two 64-bit `(lo, hi)` pairs into `Ordering` variants (`[less, equal, greater]`).
pub fn three_way_ordering(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    signed: bool,
    a: (Expr, Expr),
    b: (Expr, Expr),
    [less, equal, greater]: [Expr; 3],
) -> Expr {
    let al = materialize(cx, stmts, a.0);
    let ah = materialize(cx, stmts, a.1);
    let bl = materialize(cx, stmts, b.0);
    let bh = materialize(cx, stmts, b.1);
    let hi_lt = if signed {
        jsast::binary(JsBinOp::Lt, ah.clone(), bh.clone())
    } else {
        jsast::binary(
            JsBinOp::Lt,
            jsast::u32_wrap(ah.clone()),
            jsast::u32_wrap(bh.clone()),
        )
    };
    let lo_lt = jsast::binary(
        JsBinOp::Lt,
        jsast::u32_wrap(al.clone()),
        jsast::u32_wrap(bl.clone()),
    );
    jsast::cond(
        jsast::binary(JsBinOp::StrictNe, ah, bh),
        jsast::cond(hi_lt, less.clone(), greater.clone()),
        jsast::cond(
            jsast::binary(JsBinOp::StrictNe, al, bl),
            jsast::cond(lo_lt, less, greater),
            equal,
        ),
    )
}

/// Computes `-1 | 0 | 1` three-way ordering on two 64-bit `(lo, hi)` pairs.
#[allow(dead_code)]
pub fn three_way_sign_expr(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    signed: bool,
    (al, ah): (Expr, Expr),
    (bl, bh): (Expr, Expr),
) -> Expr {
    let al = materialize(cx, stmts, al);
    let ah = materialize(cx, stmts, ah);
    let bl = materialize(cx, stmts, bl);
    let bh = materialize(cx, stmts, bh);
    let hi_lt = if signed {
        jsast::binary(JsBinOp::Lt, ah.clone(), bh.clone())
    } else {
        jsast::binary(
            JsBinOp::Lt,
            jsast::u32_wrap(ah.clone()),
            jsast::u32_wrap(bh.clone()),
        )
    };
    let lo_lt = jsast::binary(
        JsBinOp::Lt,
        jsast::u32_wrap(al.clone()),
        jsast::u32_wrap(bl.clone()),
    );
    jsast::cond(
        jsast::binary(JsBinOp::StrictNe, ah, bh),
        jsast::cond(hi_lt, jsast::num(-1.0), jsast::num(1.0)),
        jsast::cond(
            jsast::binary(JsBinOp::StrictNe, al, bl),
            jsast::cond(lo_lt, jsast::num(-1.0), jsast::num(1.0)),
            jsast::num(0.0),
        ),
    )
}

/// Computes `(lo, hi, ovf)` for `AddWithOverflow`, `SubWithOverflow`, or `MulWithOverflow`.
pub fn with_overflow_pair(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    op: BinaryOp,
    signed: bool,
    (al, ah): (Expr, Expr),
    (bl, bh): (Expr, Expr),
) -> (Expr, Expr, Expr) {
    match op {
        BinaryOp::AddWithOverflow => {
            let al = materialize(cx, stmts, al);
            let ah = materialize(cx, stmts, ah);
            let bl = materialize(cx, stmts, bl);
            let bh = materialize(cx, stmts, bh);
            let (bind_lo, lo) =
                cx.temp(jsast::i32_wrap(jsast::binary(JsBinOp::Add, al.clone(), bl)));
            stmts.push(bind_lo);
            let (bind_c, c0) = cx.temp(jsast::cond(
                jsast::binary(
                    JsBinOp::Lt,
                    jsast::u32_wrap(lo.clone()),
                    jsast::u32_wrap(al),
                ),
                jsast::num(1.0),
                jsast::num(0.0),
            ));
            stmts.push(bind_c);
            let (bind_hi, hi) = cx.temp(jsast::i32_wrap(jsast::binary(
                JsBinOp::Add,
                jsast::binary(JsBinOp::Add, ah.clone(), bh.clone()),
                c0.clone(),
            )));
            stmts.push(bind_hi);
            let ovf = if signed {
                // `((ah ^ hi) & (bh ^ hi)) < 0`
                let ax = jsast::binary(JsBinOp::BitXor, ah, hi.clone());
                let bx = jsast::binary(JsBinOp::BitXor, bh, hi.clone());
                jsast::binary(
                    JsBinOp::Lt,
                    jsast::binary(JsBinOp::BitAnd, ax, bx),
                    jsast::num(0.0),
                )
            } else {
                // `(hi >>> 0) < (ah >>> 0) || (c0 !== 0 && bh === -1)`
                let hi_lt = jsast::binary(
                    JsBinOp::Lt,
                    jsast::u32_wrap(hi.clone()),
                    jsast::u32_wrap(ah),
                );
                let edge = jsast::binary(
                    JsBinOp::And,
                    jsast::binary(JsBinOp::StrictNe, c0, jsast::num(0.0)),
                    jsast::binary(JsBinOp::StrictEq, bh, jsast::num(-1.0)),
                );
                jsast::binary(JsBinOp::Or, hi_lt, edge)
            };
            (lo, hi, ovf)
        }
        BinaryOp::SubWithOverflow => {
            let al = materialize(cx, stmts, al);
            let ah = materialize(cx, stmts, ah);
            let bl = materialize(cx, stmts, bl);
            let bh = materialize(cx, stmts, bh);
            let (bind_lo, lo) = cx.temp(jsast::i32_wrap(jsast::binary(
                JsBinOp::Sub,
                al.clone(),
                bl.clone(),
            )));
            stmts.push(bind_lo);
            let (bind_b, b0) = cx.temp(jsast::cond(
                jsast::binary(JsBinOp::Lt, jsast::u32_wrap(al), jsast::u32_wrap(bl)),
                jsast::num(1.0),
                jsast::num(0.0),
            ));
            stmts.push(bind_b);
            let (bind_hi, hi) = cx.temp(jsast::i32_wrap(jsast::binary(
                JsBinOp::Sub,
                jsast::binary(JsBinOp::Sub, ah.clone(), bh.clone()),
                b0.clone(),
            )));
            stmts.push(bind_hi);
            let ovf = if signed {
                // `((ah ^ bh) & (ah ^ hi)) < 0`
                let ab = jsast::binary(JsBinOp::BitXor, ah.clone(), bh);
                let ah_hi = jsast::binary(JsBinOp::BitXor, ah, hi.clone());
                jsast::binary(
                    JsBinOp::Lt,
                    jsast::binary(JsBinOp::BitAnd, ab, ah_hi),
                    jsast::num(0.0),
                )
            } else {
                // `(ah >>> 0) < (bh >>> 0) || (b0 !== 0 && ah === bh)`
                let ah_lt = jsast::binary(
                    JsBinOp::Lt,
                    jsast::u32_wrap(ah.clone()),
                    jsast::u32_wrap(bh.clone()),
                );
                let edge = jsast::binary(
                    JsBinOp::And,
                    jsast::binary(JsBinOp::StrictNe, b0, jsast::num(0.0)),
                    jsast::binary(JsBinOp::StrictEq, ah, bh),
                );
                jsast::binary(JsBinOp::Or, ah_lt, edge)
            };
            (lo, hi, ovf)
        }
        BinaryOp::MulWithOverflow => {
            let call = jsast::call(
                jsast::id(HELPER_MUL_OVF),
                vec![al, ah, bl, bh, jsast::boolean(signed)],
            );
            let (bind_lo, lo) = cx.temp(call);
            stmts.push(bind_lo);
            let (bind_hi, hi) = cx.temp(jsast::id(HELPER_HI));
            stmts.push(bind_hi);
            let (bind_ovf, ovf) = cx.temp(jsast::id(HELPER_OVF));
            stmts.push(bind_ovf);
            (lo, hi, ovf)
        }
        _ => unreachable!(),
    }
}

/// Computes `(lo, hi)` for `saturating_add` / `saturating_sub`.
pub fn saturating_pair(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    add: bool,
    signed: bool,
    a: (Expr, Expr),
    b: (Expr, Expr),
) -> (Expr, Expr) {
    let op = if add {
        BinaryOp::AddWithOverflow
    } else {
        BinaryOp::SubWithOverflow
    };
    let (lo, hi, ovf) = with_overflow_pair(cx, stmts, op, signed, a, b);
    let ovf = materialize(cx, stmts, ovf);
    let hi = materialize(cx, stmts, hi);
    if !signed {
        let sat = if add { -1.0 } else { 0.0 };
        return (
            jsast::cond(ovf.clone(), jsast::num(sat), lo),
            jsast::cond(ovf, jsast::num(sat), hi),
        );
    }
    // Signed overflow wrapped sign: `r_hi < 0` => positive overflow => `i64::MAX` `(-1, 2147483647)`.
    // `r_hi >= 0` => negative overflow => `i64::MIN` `(0, -2147483648)`.
    let sign_mask = materialize(
        cx,
        stmts,
        jsast::binary(JsBinOp::Shr, hi.clone(), jsast::num(31.0)),
    );
    let sat_lo = sign_mask.clone();
    let sat_hi = jsast::binary(JsBinOp::BitXor, sign_mask, jsast::num(f64::from(i32::MIN)));
    (
        jsast::cond(ovf.clone(), sat_lo, lo),
        jsast::cond(ovf, sat_hi, hi),
    )
}

/// 32-bit SWAR popcount of an arbitrary JS expression (consumed once into a temp).
fn ctpop32(cx: &JsCx<'_>, stmts: &mut Vec<Stmt>, val: Expr) -> Expr {
    let and = |l: Expr, mask: u32| jsast::binary(JsBinOp::BitAnd, l, jsast::num(f64::from(mask)));
    let ushr = |l: Expr, s: u32| jsast::binary(JsBinOp::UShr, l, jsast::num(f64::from(s)));
    let (bind, slot) = cx.temp(val);
    stmts.push(bind);
    stmts.push(jsast::assign_stmt(
        slot.clone(),
        jsast::binary(
            JsBinOp::Sub,
            slot.clone(),
            and(ushr(slot.clone(), 1), 0x5555_5555),
        ),
    ));
    stmts.push(jsast::assign_stmt(
        slot.clone(),
        jsast::binary(
            JsBinOp::Add,
            and(slot.clone(), 0x3333_3333),
            and(ushr(slot.clone(), 2), 0x3333_3333),
        ),
    ));
    stmts.push(jsast::assign_stmt(
        slot.clone(),
        and(
            jsast::binary(JsBinOp::Add, slot.clone(), ushr(slot.clone(), 4)),
            0x0f0f_0f0f,
        ),
    ));
    ushr(jsast::imul(slot, jsast::num(f64::from(0x0101_0101u32))), 24)
}

/// 32-bit byte swap returning signed `Int32`.
fn bswap32(cx: &JsCx<'_>, stmts: &mut Vec<Stmt>, val: Expr) -> Expr {
    let v = materialize(cx, stmts, val);
    let and = |l: Expr, mask: u32| jsast::binary(JsBinOp::BitAnd, l, jsast::num(f64::from(mask)));
    let ushr = |l: Expr, s: u32| jsast::binary(JsBinOp::UShr, l, jsast::num(f64::from(s)));
    let shl = |l: Expr, s: u32| jsast::binary(JsBinOp::Shl, l, jsast::num(f64::from(s)));
    let or = |l: Expr, r: Expr| jsast::binary(JsBinOp::BitOr, l, r);
    or(
        or(ushr(v.clone(), 24), and(ushr(v.clone(), 8), 0x0000_ff00)),
        or(and(shl(v.clone(), 8), 0x00ff_0000), shl(v, 24)),
    )
}

/// 32-bit bit reversal returning signed `Int32`.
fn bitreverse32(cx: &JsCx<'_>, stmts: &mut Vec<Stmt>, val: Expr) -> Expr {
    let and = |l: Expr, mask: u32| jsast::binary(JsBinOp::BitAnd, l, jsast::num(f64::from(mask)));
    let ushr = |l: Expr, s: u32| jsast::binary(JsBinOp::UShr, l, jsast::num(f64::from(s)));
    let shl = |l: Expr, s: u32| jsast::binary(JsBinOp::Shl, l, jsast::num(f64::from(s)));
    let or = |l: Expr, r: Expr| jsast::binary(JsBinOp::BitOr, l, r);
    let (bind, slot) = cx.temp(val);
    stmts.push(bind);
    let swap = |width: u32, pattern: u32| {
        or(
            and(ushr(slot.clone(), width), pattern),
            shl(and(slot.clone(), pattern), width),
        )
    };
    stmts.push(jsast::assign_stmt(slot.clone(), swap(1, 0x5555_5555)));
    stmts.push(jsast::assign_stmt(slot.clone(), swap(2, 0x3333_3333)));
    stmts.push(jsast::assign_stmt(slot.clone(), swap(4, 0x0f0f_0f0f)));
    stmts.push(jsast::assign_stmt(slot.clone(), swap(8, 0x00ff_00ff)));
    or(ushr(slot.clone(), 16), shl(slot, 16))
}

/// Scalar `u32` result of `ctlz` / `ctlz_nonzero` / `cttz` / `cttz_nonzero` / `ctpop`.
pub fn bit_count_intrinsic(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    name: &str,
    (al, ah): (Expr, Expr),
) -> Expr {
    let clz = |x: Expr| jsast::call(jsast::member(jsast::id("Math"), "clz32"), vec![x]);
    match name {
        "ctlz" | "ctlz_nonzero" => {
            let ah = materialize(cx, stmts, ah);
            jsast::cond(
                jsast::binary(JsBinOp::StrictNe, ah.clone(), jsast::num(0.0)),
                clz(ah),
                jsast::binary(JsBinOp::Add, jsast::num(32.0), clz(al)),
            )
        }
        "cttz" | "cttz_nonzero" => {
            let al = materialize(cx, stmts, al);
            let ah = materialize(cx, stmts, ah);
            let low_lo = jsast::binary(JsBinOp::BitAnd, al.clone(), jsast::neg(al.clone()));
            let low_hi = jsast::binary(JsBinOp::BitAnd, ah.clone(), jsast::neg(ah.clone()));
            jsast::cond(
                jsast::binary(JsBinOp::StrictNe, al, jsast::num(0.0)),
                jsast::binary(JsBinOp::Sub, jsast::num(31.0), clz(low_lo)),
                jsast::cond(
                    jsast::binary(JsBinOp::StrictNe, ah, jsast::num(0.0)),
                    jsast::binary(JsBinOp::Sub, jsast::num(63.0), clz(low_hi)),
                    jsast::num(64.0),
                ),
            )
        }
        "ctpop" => jsast::binary(JsBinOp::Add, ctpop32(cx, stmts, al), ctpop32(cx, stmts, ah)),
        _ => unreachable!(),
    }
}

/// 64-bit pair result of `bswap` or `bitreverse`.
pub fn bit_permute_pair(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    name: &str,
    (al, ah): (Expr, Expr),
) -> (Expr, Expr) {
    match name {
        "bswap" => (bswap32(cx, stmts, ah), bswap32(cx, stmts, al)),
        "bitreverse" => (bitreverse32(cx, stmts, ah), bitreverse32(cx, stmts, al)),
        _ => unreachable!(),
    }
}

/// Widens a small integer / `bool` / `char` / `BigInt` (`i128`/`u128`) into `(lo, hi)`.
pub fn cast_to_i64_pair(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    from_signed: bool,
    from_bits: u32,
    from_is_bool: bool,
    from_is_128: bool,
    val: Expr,
) -> (Expr, Expr) {
    if from_is_bool {
        let lo = jsast::cond(val, jsast::num(1.0), jsast::num(0.0));
        return (lo, jsast::num(0.0));
    }
    if from_is_128 {
        let val = materialize(cx, stmts, val);
        return unpack_bigint_exprs(val);
    }
    let _ = from_bits;
    let lo = materialize(cx, stmts, jsast::i32_wrap(val));
    let hi = if from_signed {
        jsast::binary(JsBinOp::Shr, lo.clone(), jsast::num(31.0))
    } else {
        jsast::num(0.0)
    };
    (lo, hi)
}

/// Converts a 64-bit `(lo, hi)` pair into `f32` or `f64` with ZERO `BigInt` and without
/// `f64` intermediate double-rounding on `f32`.
pub fn i64_pair_to_float(signed: bool, float_bits: u32, (lo, hi): (Expr, Expr)) -> Expr {
    if float_bits == 32 {
        let helper = if signed {
            HELPER_I64_F32
        } else {
            HELPER_U64_F32
        };
        return jsast::call(jsast::id(helper), vec![lo, hi]);
    }
    let hi_num = if signed { hi } else { jsast::u32_wrap(hi) };
    let hi_scaled = jsast::binary(JsBinOp::Mul, hi_num, jsast::num(4294967296.0));
    jsast::binary(JsBinOp::Add, hi_scaled, jsast::u32_wrap(lo))
}

/// Converts an `f32`/`f64` expression into `(lo, hi)` using Rust saturating float-to-int rules.
pub fn float_to_i64_pair(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    signed: bool,
    val: Expr,
) -> (Expr, Expr) {
    let call = jsast::call(jsast::id(HELPER_F2I), vec![val, jsast::boolean(signed)]);
    let (bind, lo) = cx.temp(call);
    stmts.push(bind);
    (lo, jsast::id(HELPER_HI))
}

/// Packs 8 byte (`u8`) expressions `[b0..b7]` (little endian) directly into signed `Int32` `(lo, hi)`.
pub fn pack_8_bytes(bytes: [Expr; 8]) -> (Expr, Expr) {
    let [b0, b1, b2, b3, b4, b5, b6, b7] = bytes;
    let mask_u8 = |e: Expr| -> Expr {
        if let Some(c) = const_i32(&e) {
            jsast::num(f64::from(c & 255))
        } else {
            jsast::binary(JsBinOp::BitAnd, e, jsast::num(255.0))
        }
    };
    let combine4 = |x0: Expr, x1: Expr, x2: Expr, x3: Expr| {
        let x0 = mask_u8(x0);
        let x1 = mask_u8(x1);
        let x2 = mask_u8(x2);
        let s1 = jsast::binary(JsBinOp::Shl, x1, jsast::num(8.0));
        let s2 = jsast::binary(JsBinOp::Shl, x2, jsast::num(16.0));
        let s3 = jsast::binary(JsBinOp::Shl, x3, jsast::num(24.0));
        let lo2 = jsast::binary(JsBinOp::BitOr, x0, s1);
        let hi2 = jsast::binary(JsBinOp::BitOr, s2, s3);
        jsast::binary(JsBinOp::BitOr, lo2, hi2)
    };
    (combine4(b0, b1, b2, b3), combine4(b4, b5, b6, b7))
}

/// Converts a `[u8; 8]` array expression into `(lo, hi)` with ZERO `BigInt` or `.slice()`.
pub fn bytes_expr_to_i64_pair(cx: &JsCx<'_>, stmts: &mut Vec<Stmt>, arr: Expr) -> (Expr, Expr) {
    let arr = materialize(cx, stmts, arr);
    let idx = |i: usize| jsast::index(arr.clone(), jsast::num(i as f64));
    pack_8_bytes([
        idx(0),
        idx(1),
        idx(2),
        idx(3),
        idx(4),
        idx(5),
        idx(6),
        idx(7),
    ])
}

/// Converts a 64-bit `(lo, hi)` pair into a `[u8; 8]` array literal with ZERO `BigInt`.
pub fn i64_pair_to_bytes_expr(
    cx: &JsCx<'_>,
    stmts: &mut Vec<Stmt>,
    (lo, hi): (Expr, Expr),
) -> Expr {
    let lo = materialize(cx, stmts, lo);
    let hi = materialize(cx, stmts, hi);
    let extract = |word: &Expr, byte_idx: u32| {
        let shifted = if byte_idx == 0 {
            word.clone()
        } else {
            jsast::binary(
                JsBinOp::UShr,
                word.clone(),
                jsast::num(f64::from(byte_idx * 8)),
            )
        };
        if byte_idx == 3 {
            shifted
        } else {
            jsast::binary(JsBinOp::BitAnd, shifted, jsast::num(255.0))
        }
    };
    jsast::array(vec![
        extract(&lo, 0),
        extract(&lo, 1),
        extract(&lo, 2),
        extract(&lo, 3),
        extract(&hi, 0),
        extract(&hi, 1),
        extract(&hi, 2),
        extract(&hi, 3),
    ])
}

/// Bit-casts an `f64` expression to `(lo, hi)` via reusable 8-byte buffer views.
pub fn f64_to_bits_pair(cx: &JsCx<'_>, stmts: &mut Vec<Stmt>, val: Expr) -> (Expr, Expr) {
    let call = jsast::call(jsast::id(HELPER_F64_TO_BITS), vec![val]);
    let (bind, lo) = cx.temp(call);
    stmts.push(bind);
    (lo, jsast::id(HELPER_HI))
}

/// Bit-casts a 64-bit `(lo, hi)` pair back to `f64`.
pub fn bits_pair_to_f64((lo, hi): (Expr, Expr)) -> Expr {
    jsast::call(jsast::id(HELPER_BITS_TO_F64), vec![lo, hi])
}

/// Returns all helper [`LinkItem`]s needed to satisfy `referenced` (including transitive deps).
pub fn needed_helper_items(
    referenced: &BTreeSet<JsName>,
    existing: &BTreeSet<JsName>,
) -> Vec<LinkItem> {
    let mut need: BTreeSet<&'static str> = BTreeSet::new();
    for name in HELPER_NAMES {
        if referenced.contains(&JsName::new((*name).to_owned())) {
            need.insert(*name);
        }
    }
    if need.contains(HELPER_MUL)
        || need.contains(HELPER_MUL_OVF)
        || need.contains(HELPER_DIV)
        || need.contains(HELPER_F2I)
        || need.contains(HELPER_F64_TO_BITS)
    {
        need.insert(HELPER_HI);
    }
    if need.contains(HELPER_MUL_OVF) {
        need.insert(HELPER_OVF);
    }
    if need.contains(HELPER_F64_TO_BITS) || need.contains(HELPER_BITS_TO_F64) {
        need.insert(HELPER_F64_VIEW);
        need.insert(HELPER_I32_VIEW);
    }
    if need.contains(HELPER_I32_VIEW) {
        need.insert(HELPER_F64_VIEW);
    }

    let make = |name: &'static str, kind: ItemKind, refs: &[&'static str], js: String| LinkItem {
        js,
        name: JsName::new(name.to_owned()),
        kind,
        refs: refs.iter().map(|r| JsName::new((*r).to_owned())).collect(),
        zombies: Vec::new(),
        linkage: Linkage::internal(),
        debug_path: "i64 scalar-pair runtime helper".to_owned(),
        crate_name: "generated".to_owned(),
        order: None,
        map: ItemMap::default(),
        dts: None,
        optimization: Default::default(),
        late_ir: Default::default(),
        baseline_shadow: Default::default(),
        jir: Default::default(),
    };

    // Co-locate shared mutable static registers with every active writer function so ESM chunk
    // splitting never separates a writer from its `let` binding (ESM imports are read-only).
    let active_hi_writers: Vec<&'static str> = [
        HELPER_MUL,
        HELPER_MUL_OVF,
        HELPER_DIV,
        HELPER_F2I,
        HELPER_F64_TO_BITS,
    ]
    .into_iter()
    .filter(|w| need.contains(w) && !existing.contains(&JsName::new((*w).to_owned())))
    .collect();
    let active_ovf_writers: Vec<&'static str> = [HELPER_MUL_OVF]
        .into_iter()
        .filter(|w| need.contains(w) && !existing.contains(&JsName::new((*w).to_owned())))
        .collect();

    let mut out = Vec::new();
    for &name in HELPER_NAMES {
        if !need.contains(name) || existing.contains(&JsName::new(name.to_owned())) {
            continue;
        }
        let item = match name {
            HELPER_HI => make(
                name,
                ItemKind::Static,
                &active_hi_writers,
                format!("\nlet {HELPER_HI} = 0;\n"),
            ),
            HELPER_OVF => make(
                name,
                ItemKind::Static,
                &active_ovf_writers,
                format!("\nlet {HELPER_OVF} = false;\n"),
            ),
            HELPER_F64_VIEW => make(
                name,
                ItemKind::Static,
                &[],
                format!("\nconst {HELPER_F64_VIEW} = new Float64Array(new ArrayBuffer(8));\n"),
            ),
            HELPER_I32_VIEW => make(
                name,
                ItemKind::Static,
                &[HELPER_F64_VIEW],
                format!("\nconst {HELPER_I32_VIEW} = new Int32Array({HELPER_F64_VIEW}.buffer);\n"),
            ),
            HELPER_MUL => make(
                name,
                ItemKind::Fn,
                &[HELPER_HI],
                format!(
                    "\nfunction {HELPER_MUL}(al, ah, bl, bh) {{\n  \
                     const a0 = al & 65535, a1 = (al >>> 16) | 0;\n  \
                     const b0 = bl & 65535, b1 = (bl >>> 16) | 0;\n  \
                     const p00 = Math.imul(a0, b0);\n  \
                     const v = (((p00 >>> 16) | 0) + Math.imul(a1, b0)) | 0;\n  \
                     const f = ((v & 65535) + Math.imul(a0, b1)) | 0;\n  \
                     {HELPER_HI} = (((Math.imul(ah, bl) + Math.imul(a1, b1)) | 0) + Math.imul(al, bh) + (v >>> 16) + (f >>> 16)) | 0;\n  \
                     return (p00 & 65535) | (f << 16);\n\
                     }}\n"
                ),
            ),
            HELPER_MUL_OVF => make(
                name,
                ItemKind::Fn,
                &[HELPER_HI, HELPER_OVF],
                format!(
                    "\nfunction {HELPER_MUL_OVF}(al, ah, bl, bh, signed) {{\n  \
                     let negA = false, negB = false, ual = al, uah = ah, ubl = bl, ubh = bh;\n  \
                     if (signed) {{\n    \
                       if (ah < 0) {{ negA = true; ual = (-al) | 0; uah = (~ah + (ual === 0 ? 1 : 0)) | 0; }}\n    \
                       if (bh < 0) {{ negB = true; ubl = (-bl) | 0; ubh = (~bh + (ubl === 0 ? 1 : 0)) | 0; }}\n  \
                     }}\n  \
                     const a0 = ual & 65535, a1 = (ual >>> 16) | 0;\n  \
                     const b0 = ubl & 65535, b1 = (ubl >>> 16) | 0;\n  \
                     const p00 = Math.imul(a0, b0);\n  \
                     const v = (((p00 >>> 16) | 0) + Math.imul(a1, b0)) | 0;\n  \
                     const f = ((v & 65535) + Math.imul(a0, b1)) | 0;\n  \
                     let lo = (p00 & 65535) | (f << 16);\n  \
                     const crossHi = (uah >>> 0) * (ubl >>> 0) + (ual >>> 0) * (ubh >>> 0) + a1 * b1 + (v >>> 16) + (f >>> 16);\n  \
                     let hi = (((Math.imul(uah, ubl) + Math.imul(a1, b1)) | 0) + Math.imul(ual, ubh) + (v >>> 16) + (f >>> 16)) | 0;\n  \
                     const uovf = (uah !== 0 && ubh !== 0) || crossHi > 4294967295;\n  \
                     if (signed) {{\n    \
                       const negRes = negA !== negB;\n    \
                       {HELPER_OVF} = uovf || (negRes ? (hi < 0 && (lo !== 0 || hi !== -2147483648)) : hi < 0);\n    \
                       if (negRes) {{ lo = (-lo) | 0; hi = (~hi + (lo === 0 ? 1 : 0)) | 0; }}\n  \
                     }} else {{\n    \
                       {HELPER_OVF} = uovf;\n  \
                     }}\n  \
                     {HELPER_HI} = hi;\n  \
                     return lo;\n\
                     }}\n"
                ),
            ),
            HELPER_DIV => make(
                name,
                ItemKind::Fn,
                &[HELPER_HI],
                format!(
                    "\nfunction {HELPER_DIV}(al, ah, bl, bh, signed, rem) {{\n  \
                     if (!signed && (ah | bh) === 0 && bl !== 0) {{\n    \
                       const u = al >>> 0, v = bl >>> 0;\n    \
                       {HELPER_HI} = 0;\n    \
                       return rem ? (u % v) | 0 : ((u / v) >>> 0) | 0;\n  \
                     }}\n  \
                     let negA = false, negB = false;\n  \
                     if (signed) {{\n    \
                       if (ah < 0) {{ negA = true; al = (-al) | 0; ah = (~ah + (al === 0 ? 1 : 0)) | 0; }}\n    \
                       if (bh < 0) {{ negB = true; bl = (-bl) | 0; bh = (~bh + (bl === 0 ? 1 : 0)) | 0; }}\n  \
                     }}\n  \
                     const ual = al >>> 0, uah = ah >>> 0, ubl = bl >>> 0, ubh = bh >>> 0;\n  \
                     let ql = 0, qh = 0, rl = 0, rh = 0;\n  \
                     if (ubh === 0) {{\n    \
                       if (uah === 0) {{\n      \
                         if (rem) rl = ual % ubl; else ql = (ual / ubl) >>> 0;\n    \
                       }} else if (ubl <= 2097152) {{\n      \
                         qh = (uah / ubl) >>> 0;\n      \
                         const mid = (uah - qh * ubl) * 4294967296 + ual;\n      \
                         ql = (mid / ubl) >>> 0;\n      \
                         if (rem) rl = (mid - ql * ubl) >>> 0;\n    \
                       }} else {{\n      \
                         qh = (uah / ubl) >>> 0;\n      \
                         const r0 = uah - qh * ubl;\n      \
                         let est = Math.floor((r0 * 4294967296 + ual) / ubl);\n      \
                         if (est > 4294967295) est = 4294967295;\n      \
                         const t0 = est * (ubl & 65535);\n      \
                         const t1 = est * (ubl >>> 16) + Math.floor(t0 / 65536);\n      \
                         let plo = ((t1 & 65535) * 65536 + (t0 & 65535)) >>> 0;\n      \
                         let phi = Math.floor(t1 / 65536);\n      \
                         while (phi > r0 || (phi === r0 && plo > ual)) {{\n        \
                           est--;\n        \
                           if (plo < ubl) phi--;\n        \
                           plo = (plo - ubl) >>> 0;\n      \
                         }}\n      \
                         let rlo = (ual - plo) >>> 0;\n      \
                         let rhi = r0 - phi - (ual < plo ? 1 : 0);\n      \
                         while (rhi > 0 || rlo >= ubl) {{\n        \
                           est++;\n        \
                           if (rlo < ubl) rhi--;\n        \
                           rlo = (rlo - ubl) >>> 0;\n      \
                         }}\n      \
                         ql = est >>> 0;\n      \
                         rl = rlo;\n    \
                       }}\n  \
                     }} else if (ubh > uah || (ubh === uah && ubl > ual)) {{\n    \
                       rl = ual; rh = uah;\n  \
                     }} else if (uah < 2097152 && ubh < 2097152) {{\n    \
                       const fa = uah * 4294967296 + ual;\n    \
                       const fb = ubh * 4294967296 + ubl;\n    \
                       const q = Math.floor(fa / fb);\n    \
                       if (rem) {{\n      \
                         const r = fa - q * fb;\n      \
                         rh = Math.floor(r / 4294967296) >>> 0;\n      \
                         rl = (r - rh * 4294967296) >>> 0;\n    \
                       }} else {{\n      \
                         ql = q >>> 0;\n    \
                       }}\n  \
                     }} else {{\n    \
                       const fa = uah * 4294967296 + ual;\n    \
                       const fb = ubh * 4294967296 + ubl;\n    \
                       let est = Math.floor(fa / fb);\n    \
                       if (est > 4294967295) est = 4294967295;\n    \
                       const t0 = est * (ubl & 65535);\n    \
                       const t1 = est * (ubl >>> 16) + Math.floor(t0 / 65536);\n    \
                       let plo = ((t1 & 65535) * 65536 + (t0 & 65535)) >>> 0;\n    \
                       let phi = est * ubh + Math.floor(t1 / 65536);\n    \
                       while (phi > uah || (phi === uah && plo > ual)) {{\n      \
                         est--;\n      \
                         const b = plo < ubl ? 1 : 0;\n      \
                         plo = (plo - ubl) >>> 0;\n      \
                         phi -= ubh + b;\n    \
                       }}\n    \
                       let rlo = (ual - plo) >>> 0;\n    \
                       let rhi = uah - phi - (ual < plo ? 1 : 0);\n    \
                       while (rhi > ubh || (rhi === ubh && rlo >= ubl)) {{\n      \
                         est++;\n      \
                         const b = rlo < ubl ? 1 : 0;\n      \
                         rlo = (rlo - ubl) >>> 0;\n      \
                         rhi -= ubh + b;\n    \
                       }}\n    \
                       ql = est >>> 0;\n    \
                       rl = rlo;\n    \
                       rh = rhi >>> 0;\n  \
                     }}\n  \
                     let lo = (rem ? rl : ql) | 0;\n  \
                     let hi = (rem ? rh : qh) | 0;\n  \
                     if (signed && (rem ? negA : negA !== negB)) {{\n    \
                       lo = (-lo) | 0;\n    \
                       hi = (~hi + (lo === 0 ? 1 : 0)) | 0;\n  \
                     }}\n  \
                     {HELPER_HI} = hi;\n  \
                     return lo;\n\
                     }}\n"
                ),
            ),
            HELPER_F2I => make(
                name,
                ItemKind::Fn,
                &[HELPER_HI],
                format!(
                    "\nfunction {HELPER_F2I}(x, signed) {{\n  \
                     if (Number.isNaN(x)) {{ {HELPER_HI} = 0; return 0; }}\n  \
                     if (signed) {{\n    \
                       if (x <= -9223372036854775808) {{ {HELPER_HI} = -2147483648; return 0; }}\n    \
                       if (x >= 9223372036854775808) {{ {HELPER_HI} = 2147483647; return -1; }}\n  \
                     }} else {{\n    \
                       if (x <= 0) {{ {HELPER_HI} = 0; return 0; }}\n    \
                       if (x >= 18446744073709551616) {{ {HELPER_HI} = -1; return -1; }}\n  \
                     }}\n  \
                     const t = Math.trunc(x);\n  \
                     const h = Math.floor(t / 4294967296) | 0;\n  \
                     {HELPER_HI} = h;\n  \
                     return (t - (signed ? h : (h >>> 0)) * 4294967296) | 0;\n\
                     }}\n"
                ),
            ),
            HELPER_F64_TO_BITS => make(
                name,
                ItemKind::Fn,
                &[HELPER_HI, HELPER_F64_VIEW, HELPER_I32_VIEW],
                format!(
                    "\nfunction {HELPER_F64_TO_BITS}(x) {{\n  \
                     {HELPER_F64_VIEW}[0] = x;\n  \
                     {HELPER_HI} = {HELPER_I32_VIEW}[1];\n  \
                     return {HELPER_I32_VIEW}[0];\n\
                     }}\n"
                ),
            ),
            HELPER_BITS_TO_F64 => make(
                name,
                ItemKind::Fn,
                &[HELPER_F64_VIEW, HELPER_I32_VIEW],
                format!(
                    "\nfunction {HELPER_BITS_TO_F64}(lo, hi) {{\n  \
                     {HELPER_I32_VIEW}[0] = lo;\n  \
                     {HELPER_I32_VIEW}[1] = hi;\n  \
                     return {HELPER_F64_VIEW}[0];\n\
                     }}\n"
                ),
            ),
            HELPER_U64_F32 => make(
                name,
                ItemKind::Fn,
                &[],
                format!(
                    "\nfunction {HELPER_U64_F32}(lo, hi) {{\n  \
                     const h = hi >>> 0;\n  \
                     const l = h >= 2097152 ? (((lo & -4096) | ((lo & 4095) !== 0 ? 2048 : 0)) >>> 0) : (lo >>> 0);\n  \
                     return Math.fround(h * 4294967296 + l);\n\
                     }}\n"
                ),
            ),
            HELPER_I64_F32 => make(
                name,
                ItemKind::Fn,
                &[],
                format!(
                    "\nfunction {HELPER_I64_F32}(lo, hi) {{\n  \
                     const h = hi | 0;\n  \
                     const l = (((h + 2097152) | 0) >>> 0) >= 4194304 ? (((lo & -4096) | ((lo & 4095) !== 0 ? 2048 : 0)) >>> 0) : (lo >>> 0);\n  \
                     return Math.fround(h * 4294967296 + l);\n\
                     }}\n"
                ),
            ),
            _ => unreachable!(),
        };
        out.push(item);
    }
    out
}

/// Concatenated JS text of all helper items, for standalone unit-test harnesses.
#[cfg_attr(not(test), allow(dead_code))]
pub fn helper_preamble_js() -> String {
    let all: BTreeSet<JsName> = HELPER_NAMES
        .iter()
        .map(|n| JsName::new((*n).to_owned()))
        .collect();
    let mut out = String::new();
    for item in needed_helper_items(&all, &BTreeSet::new()) {
        out.push_str(&item.js);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgjs_object::item::Availability;
    use rcgjs_object::jir::interns::Interns;
    use rcgjs_object::jir::repr::{FuncBuilder, RTerm};
    use rcgjs_object::jir::semantic::{AbiSignature, DefinitionId, ItemId, Signature};
    use rcgjs_object::jir::summary::{ItemClass, ItemSummary, SemanticKey};
    use rcgjs_object::object::BlobRef;
    use rcgjs_object::opts::Opts;

    fn with_cx(f: impl FnOnce(&JsCx<'_>, &mut Vec<Stmt>)) {
        let mut b = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash: 1,
                },
                instantiation: 1,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 0,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    inline_hint: false,
                    inline_always: false,
                },
                arguments: Vec::new(),
                result: TypeId(1),
            },
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        let func = b.finish();
        let interns = Interns::new();
        let opts = Opts::default();
        let plan = crate::lower::fuse::plan(&func, None, &interns.types, &interns, &opts);
        let summary = ItemSummary {
            key: SemanticKey {
                link_name: "test".to_owned(),
                item: None,
            },
            body_hash: None,
            semantic_hash: None,
            availability: Availability::Available,
            deps: Vec::new(),
            abi: None,
            physical: None,
            kind: ItemKind::Fn,
            linkage: Linkage::internal(),
            order: None,
            dts: false,
            fallback: BlobRef {
                offset: 0,
                clen: 0,
                ulen: 0,
            },
            body: None,
            origin: None,
            estimated_bytes: 0,
            class: ItemClass::default(),
        };
        let cx = JsCx::new(
            &func,
            &plan,
            &interns.types,
            &interns,
            &opts,
            &summary,
            None,
        );
        let mut stmts = Vec::new();
        f(&cx, &mut stmts);
    }

    #[test]
    fn shl_shr_rotate_peepholes_fold_zero_lanes_and_one_u64_bitmask() {
        with_cx(|cx, stmts| {
            // 1u64 << f (byteset bitmask in TwoWaySearcher): no `0 <<` and no `1 >>> 1`
            let (lo, hi) = shl_pair(
                cx,
                stmts,
                (jsast::num(1.0), jsast::num(0.0)),
                jsast::id("f"),
            );
            let lo_s = jsast::expr_to_string(&lo);
            let hi_s = jsast::expr_to_string(&hi);
            assert!(
                !lo_s.contains("0 <<") && !hi_s.contains("0 <<"),
                "{lo_s} / {hi_s}"
            );
            assert!(!hi_s.contains(">>> 1"), "{hi_s}");

            // (v, 0) << 8: no `0 << 8 |`
            let (lo8, hi8) = shl_pair(
                cx,
                stmts,
                (jsast::id("v"), jsast::num(0.0)),
                jsast::num(8.0),
            );
            assert_eq!(jsast::expr_to_string(&lo8), "v << 8");
            assert_eq!(jsast::expr_to_string(&hi8), "v >>> 24");

            // (v, 0) >>> k (unsigned dynamic shift): folds hi to 0 and keeps | 0 on lo
            let (rlo, rhi) = shr_pair(
                cx,
                stmts,
                false,
                (jsast::id("v"), jsast::num(0.0)),
                jsast::id("k"),
            );
            let rlo_s = jsast::expr_to_string(&rlo);
            assert!(!rlo_s.contains("0 <<"), "{rlo_s}");
            assert!(
                rlo_s.contains("| 0"),
                "dynamic u32>>>k must stay Int32: {rlo_s}"
            );
            assert_eq!(jsast::expr_to_string(&rhi), "0");

            // rotate_left((v, 0), 8): folds zero lane terms away
            let (rot_lo, rot_hi) = rotate_pair(
                cx,
                stmts,
                true,
                (jsast::id("v"), jsast::num(0.0)),
                jsast::num(8.0),
            );
            assert_eq!(jsast::expr_to_string(&rot_lo), "v << 8");
            assert_eq!(jsast::expr_to_string(&rot_hi), "v >>> 24");
        });
    }

    #[test]
    fn peepholes_preserve_impure_call_operands() {
        let bump = |tag: f64| jsast::call(jsast::id("bump"), vec![jsast::num(tag)]);
        let stmts_contain = |stmts: &[Stmt], needle: &str| {
            stmts.iter().any(|s| match s {
                Stmt::ExprStmt(e) | Stmt::Let(_, Some(e)) | Stmt::Const(_, e) => {
                    jsast::expr_to_string(e).contains(needle)
                }
                _ => false,
            })
        };

        // 0u64 << call(), 0u64 >> call(), 0i64 >> call(), rotate_pair(0, call())
        with_cx(|cx, stmts| {
            let (lo, hi) = shl_pair(cx, stmts, (jsast::num(0.0), jsast::num(0.0)), bump(1.0));
            assert_eq!(jsast::expr_to_string(&lo), "0");
            assert_eq!(jsast::expr_to_string(&hi), "0");
            assert!(stmts_contain(stmts, "bump(1)"), "{stmts:?}");
        });
        with_cx(|cx, stmts| {
            let (lo, hi) = shr_pair(
                cx,
                stmts,
                false,
                (jsast::num(0.0), jsast::num(0.0)),
                bump(2.0),
            );
            assert_eq!(jsast::expr_to_string(&lo), "0");
            assert_eq!(jsast::expr_to_string(&hi), "0");
            assert!(stmts_contain(stmts, "bump(2)"), "{stmts:?}");
        });
        with_cx(|cx, stmts| {
            let (lo, hi) = shr_pair(
                cx,
                stmts,
                true,
                (jsast::num(0.0), jsast::num(0.0)),
                bump(3.0),
            );
            assert_eq!(jsast::expr_to_string(&lo), "0");
            assert_eq!(jsast::expr_to_string(&hi), "0");
            assert!(stmts_contain(stmts, "bump(3)"), "{stmts:?}");
        });
        with_cx(|cx, stmts| {
            let (lo, hi) = rotate_pair(
                cx,
                stmts,
                true,
                (jsast::num(0.0), jsast::num(0.0)),
                bump(4.0),
            );
            assert_eq!(jsast::expr_to_string(&lo), "0");
            assert_eq!(jsast::expr_to_string(&hi), "0");
            assert!(stmts_contain(stmts, "bump(4)"), "{stmts:?}");
        });

        // Constant shifts dropping one lane (`k >= 32`) & `0u64 * call()`
        with_cx(|cx, stmts| {
            let (lo, hi) = shl_pair(cx, stmts, (bump(5.0), bump(6.0)), jsast::num(32.0));
            assert!(stmts_contain(stmts, "bump(5)"), "{stmts:?}");
            assert!(stmts_contain(stmts, "bump(6)"), "{stmts:?}");
            assert_eq!(jsast::expr_to_string(&lo), "0");
            assert!(!jsast::expr_to_string(&hi).is_empty());
        });
        with_cx(|cx, stmts| {
            let (lo, hi) = shr_pair(cx, stmts, false, (bump(7.0), bump(8.0)), jsast::num(40.0));
            assert!(stmts_contain(stmts, "bump(7)"), "{stmts:?}");
            assert!(stmts_contain(stmts, "bump(8)"), "{stmts:?}");
            assert_eq!(jsast::expr_to_string(&hi), "0");
            assert!(jsast::expr_to_string(&lo).contains(">>> 8"));
        });
        with_cx(|cx, stmts| {
            let (lo, hi) = mul_pair(
                cx,
                stmts,
                (jsast::num(0.0), jsast::num(0.0)),
                (bump(9.0), bump(10.0)),
            );
            assert_eq!(jsast::expr_to_string(&lo), "0");
            assert_eq!(jsast::expr_to_string(&hi), "0");
            assert!(stmts_contain(stmts, "bump(9)"), "{stmts:?}");
            assert!(stmts_contain(stmts, "bump(10)"), "{stmts:?}");
        });

        // Pure expression helpers: fold_bit_or_i32, fold_shl, fold_shr_signed, fold_cond_i32, bitwise_pair
        let bor = fold_bit_or_i32(jsast::num(-1.0), bump(11.0));
        assert!(jsast::expr_to_string(&bor).contains("bump(11)"), "{bor:?}");
        let fshl = fold_shl(jsast::num(0.0), bump(12.0));
        assert!(
            jsast::expr_to_string(&fshl).contains("bump(12)"),
            "{fshl:?}"
        );
        let fshr = fold_shr_signed(jsast::num(-1.0), bump(13.0));
        assert!(
            jsast::expr_to_string(&fshr).contains("bump(13)"),
            "{fshr:?}"
        );
        let fcond = fold_cond_i32(bump(14.0), jsast::num(0.0), jsast::num(0.0));
        assert!(
            jsast::expr_to_string(&fcond).contains("bump(14)"),
            "{fcond:?}"
        );
        let (and_lo, and_hi) = bitwise_pair(
            JsBinOp::BitAnd,
            (jsast::num(0.0), jsast::num(0.0)),
            (bump(15.0), bump(16.0)),
        );
        assert!(jsast::expr_to_string(&and_lo).contains("bump(15)"));
        assert!(jsast::expr_to_string(&and_hi).contains("bump(16)"));
        let (or_lo, or_hi) = bitwise_pair(
            JsBinOp::BitOr,
            (jsast::num(-1.0), jsast::num(-1.0)),
            (bump(17.0), bump(18.0)),
        );
        assert!(jsast::expr_to_string(&or_lo).contains("bump(17)"));
        assert!(jsast::expr_to_string(&or_hi).contains("bump(18)"));
    }
}
