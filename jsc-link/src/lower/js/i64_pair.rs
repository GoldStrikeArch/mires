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
use rcgjs_object::jsast::{self, BinOp as JsBinOp, Expr, Stmt};
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

/// If `expr` is an integer numeric literal in signed `Int32` range, returns its `i32` value.
pub fn const_i32(expr: &Expr) -> Option<i32> {
    match expr {
        Expr::Num(n) if n.is_finite() && n.fract() == 0.0 => {
            let v = *n as i64;
            i32::try_from(v).ok()
        }
        Expr::RawNum(s) => s.parse::<i32>().ok(),
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
        return (jsast::num(0.0), jsast::num(0.0));
    }
    if const_i32(&bl) == Some(1) && const_i32(&bh) == Some(0) {
        return (al, ah);
    }
    if const_i32(&al) == Some(1) && const_i32(&ah) == Some(0) {
        return (bl, bh);
    }
    let call = jsast::call(jsast::id(HELPER_MUL), vec![al, ah, bl, bh]);
    let (bind, lo_var) = cx.temp(call);
    stmts.push(bind);
    (lo_var, jsast::id(HELPER_HI))
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

/// Computes `(lo, hi)` for bitwise `&`, `|`, `^`.
pub fn bitwise_pair(op: JsBinOp, (al, ah): (Expr, Expr), (bl, bh): (Expr, Expr)) -> (Expr, Expr) {
    let half = |l: Expr, r: Expr| match (op, const_i32(&l), const_i32(&r)) {
        (JsBinOp::BitAnd, Some(0), _) | (JsBinOp::BitAnd, _, Some(0)) => jsast::num(0.0),
        (JsBinOp::BitAnd, Some(-1), _) => r,
        (JsBinOp::BitAnd, _, Some(-1)) => l,
        (JsBinOp::BitOr, Some(0), _) => r,
        (JsBinOp::BitOr, _, Some(0)) => l,
        (JsBinOp::BitOr, Some(-1), _) | (JsBinOp::BitOr, _, Some(-1)) => jsast::num(-1.0),
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
        if k < 32 {
            let al = materialize(cx, stmts, al);
            let lo = jsast::binary(JsBinOp::Shl, al.clone(), jsast::num(f64::from(k)));
            let hi = jsast::binary(
                JsBinOp::BitOr,
                jsast::binary(JsBinOp::Shl, ah, jsast::num(f64::from(k))),
                jsast::binary(JsBinOp::UShr, al, jsast::num(f64::from(32 - k))),
            );
            return (lo, hi);
        }
        if k == 32 {
            return (jsast::num(0.0), al);
        }
        let hi = jsast::binary(JsBinOp::Shl, al, jsast::num(f64::from(k - 32)));
        return (jsast::num(0.0), hi);
    }
    let al = materialize(cx, stmts, al);
    let ah = materialize(cx, stmts, ah);
    let k = materialize(cx, stmts, mask_shift_63(shift_amount, true, false));
    let high_flag = jsast::binary(
        JsBinOp::StrictNe,
        jsast::binary(JsBinOp::BitAnd, k.clone(), jsast::num(32.0)),
        jsast::num(0.0),
    );
    let lo = jsast::cond(
        high_flag.clone(),
        jsast::num(0.0),
        jsast::binary(JsBinOp::Shl, al.clone(), k.clone()),
    );
    // `(al >>> 1) >>> (31 - k)` equals `al >>> (32 - k)` for `1..=31` and `0` when `k == 0`
    // because `(al >>> 1)` has only 31 significant bits, avoiding JS `x >>> 32 == x`.
    let cross = jsast::binary(
        JsBinOp::UShr,
        jsast::binary(JsBinOp::UShr, al.clone(), jsast::num(1.0)),
        jsast::binary(JsBinOp::Sub, jsast::num(31.0), k.clone()),
    );
    let hi_lt32 = jsast::binary(
        JsBinOp::BitOr,
        jsast::binary(JsBinOp::Shl, ah, k.clone()),
        cross,
    );
    let hi = jsast::cond(high_flag, jsast::binary(JsBinOp::Shl, al, k), hi_lt32);
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
            jsast::binary(JsBinOp::Shr, expr, amt)
        } else {
            jsast::i32_wrap(jsast::binary(JsBinOp::UShr, expr, amt))
        }
    };
    let fill_hi = |ah_expr: Expr| {
        if signed {
            jsast::binary(JsBinOp::Shr, ah_expr, jsast::num(31.0))
        } else {
            jsast::num(0.0)
        }
    };

    if let Some(k) = const_i32(&shift_amount).map(|s| (s & 63) as u32) {
        if k == 0 {
            return (al, ah);
        }
        if k < 32 {
            let ah = materialize(cx, stmts, ah);
            let lo = jsast::binary(
                JsBinOp::BitOr,
                jsast::binary(JsBinOp::UShr, al, jsast::num(f64::from(k))),
                jsast::binary(JsBinOp::Shl, ah.clone(), jsast::num(f64::from(32 - k))),
            );
            let hi = shift_hi(ah, jsast::num(f64::from(k)));
            return (lo, hi);
        }
        if k == 32 {
            let ah = materialize(cx, stmts, ah);
            return (ah.clone(), fill_hi(ah));
        }
        let ah = materialize(cx, stmts, ah);
        let lo = shift_hi(ah.clone(), jsast::num(f64::from(k - 32)));
        let hi = fill_hi(ah);
        return (lo, hi);
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
    let cross = jsast::binary(
        JsBinOp::Shl,
        jsast::binary(JsBinOp::Shl, ah.clone(), jsast::num(1.0)),
        jsast::binary(JsBinOp::Sub, jsast::num(31.0), k.clone()),
    );
    let lo_lt32 = jsast::binary(
        JsBinOp::BitOr,
        jsast::binary(JsBinOp::UShr, al, k.clone()),
        cross,
    );
    let lo = jsast::cond(high_flag.clone(), shift_hi(ah.clone(), k.clone()), lo_lt32);
    let hi = jsast::cond(high_flag, fill_hi(ah.clone()), shift_hi(ah, k));
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
        let lo = jsast::binary(
            JsBinOp::BitOr,
            jsast::binary(JsBinOp::Shl, a.clone(), jsast::num(f64::from(r))),
            jsast::binary(JsBinOp::UShr, b.clone(), jsast::num(f64::from(32 - r))),
        );
        let hi = jsast::binary(
            JsBinOp::BitOr,
            jsast::binary(JsBinOp::Shl, b, jsast::num(f64::from(r))),
            jsast::binary(JsBinOp::UShr, a, jsast::num(f64::from(32 - r))),
        );
        return (lo, hi);
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
        jsast::binary(
            JsBinOp::BitOr,
            jsast::binary(JsBinOp::Shl, x, m.clone()),
            jsast::binary(
                JsBinOp::UShr,
                jsast::binary(JsBinOp::UShr, y, jsast::num(1.0)),
                inv_m.clone(),
            ),
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
    let lo = jsast::cond(swap.clone(), r_hi.clone(), r_lo.clone());
    let hi = jsast::cond(swap, r_lo, r_hi);
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

/// Converts a 64-bit `(lo, hi)` pair into `f32` or `f64` with ZERO `BigInt`.
pub fn i64_pair_to_float(signed: bool, float_bits: u32, (lo, hi): (Expr, Expr)) -> Expr {
    let hi_num = if signed { hi } else { jsast::u32_wrap(hi) };
    let hi_scaled = jsast::binary(JsBinOp::Mul, hi_num, jsast::num(4294967296.0));
    let exact_f64 = jsast::binary(JsBinOp::Add, hi_scaled, jsast::u32_wrap(lo));
    if float_bits == 32 {
        jsast::fround(exact_f64)
    } else {
        exact_f64
    }
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
                     const s32 = BigInt(\"32\");\n  \
                     const a = (BigInt(signed ? ah : (ah >>> 0)) << s32) | BigInt(al >>> 0);\n  \
                     const b = (BigInt(signed ? bh : (bh >>> 0)) << s32) | BigInt(bl >>> 0);\n  \
                     const q = rem ? a % b : (signed ? BigInt.asIntN(64, a / b) : a / b);\n  \
                     {HELPER_HI} = Number(BigInt.asIntN(32, q >> s32)) | 0;\n  \
                     return Number(BigInt.asIntN(32, q)) | 0;\n\
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
