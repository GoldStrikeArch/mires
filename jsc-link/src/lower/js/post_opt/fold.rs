//! Expression & constant folding, algebraic simplification, bit-op / ternary / boolean
//! simplification, and constant switch folding.

use std::cmp::Ordering;
use std::collections::HashSet;

use rcgjs_object::jsast::{self, ArrowBody, BinOp, Expr, Stmt, UnOp, VisitMut};

use super::super::ast_util::{
    exact_i32, exact_u32, expr_has_side_effect, has_unlabeled_switch_break,
};

#[derive(Clone, Debug, PartialEq)]
pub(super) enum SimpleLit {
    Num(f64),
    Str(String),
    Bool(bool),
    Null,
    Undefined,
}

impl SimpleLit {
    pub(super) fn from_expr(expr: &Expr) -> Option<Self> {
        match expr {
            Expr::Num(n) if n.is_finite() => Some(Self::Num(*n)),
            Expr::Unary(UnOp::Neg, inner) => match inner.as_ref() {
                Expr::Num(n) if n.is_finite() => Some(Self::Num(-*n)),
                _ => None,
            },
            Expr::Str(s) if s.len() <= 32 => Some(Self::Str(s.clone())),
            Expr::Bool(b) => Some(Self::Bool(*b)),
            Expr::Null => Some(Self::Null),
            Expr::Undefined => Some(Self::Undefined),
            _ => None,
        }
    }

    pub(super) fn to_expr(&self) -> Expr {
        match self {
            Self::Num(n) => {
                if *n < 0.0 {
                    Expr::Unary(UnOp::Neg, Box::new(Expr::Num(-*n)))
                } else {
                    Expr::Num(*n)
                }
            }
            Self::Str(s) => Expr::Str(s.clone()),
            Self::Bool(b) => Expr::Bool(*b),
            Self::Null => Expr::Null,
            Self::Undefined => Expr::Undefined,
        }
    }

    pub(super) fn strict_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Num(a), Self::Num(b)) => *a == *b,
            (Self::Str(a), Self::Str(b)) => a == b,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Null, Self::Null) => true,
            (Self::Undefined, Self::Undefined) => true,
            _ => false,
        }
    }
}

pub(super) fn exact_f64_lit(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Num(n) => Some(*n),
        Expr::Unary(UnOp::Neg, inner) => match inner.as_ref() {
            Expr::Num(n) => Some(-*n),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn to_i32_bits(expr: &Expr) -> Option<i32> {
    exact_i32(expr).or_else(|| exact_u32(expr).map(|u| u as i32))
}

pub(super) fn num_expr_i32(v: i32) -> Expr {
    if v < 0 {
        Expr::Unary(UnOp::Neg, Box::new(Expr::Num(-(v as f64))))
    } else {
        Expr::Num(v as f64)
    }
}

fn is_bool_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Bool(_) | Expr::Unary(UnOp::Not, _) => true,
        Expr::Binary(op, l, r) => match op {
            BinOp::Lt
            | BinOp::Le
            | BinOp::Gt
            | BinOp::Ge
            | BinOp::StrictEq
            | BinOp::StrictNe
            | BinOp::Eq
            | BinOp::Ne => true,
            BinOp::And | BinOp::Or => is_bool_expr(l) && is_bool_expr(r),
            _ => false,
        },
        Expr::Cond(_, t, e) => is_bool_expr(t) && is_bool_expr(e),
        _ => false,
    }
}

pub(in super::super) fn is_already_i32_expr(expr: &Expr) -> bool {
    if is_proven_nonneg_i31(expr) {
        return true;
    }
    match expr {
        Expr::Num(n) => {
            n.is_finite()
                && n.fract() == 0.0
                && !(*n == 0.0 && n.is_sign_negative())
                && (-2147483648.0..=2147483647.0).contains(n)
        }
        Expr::Unary(UnOp::Neg, inner) => match inner.as_ref() {
            Expr::Num(n) => {
                let v = -*n;
                v.is_finite()
                    && v.fract() == 0.0
                    && *n != 0.0
                    && (-2147483648.0..=2147483647.0).contains(&v)
            }
            _ => false,
        },
        Expr::Unary(UnOp::BitNot, _) => true,
        Expr::Binary(
            BinOp::BitOr | BinOp::BitAnd | BinOp::BitXor | BinOp::Shl | BinOp::Shr,
            _,
            _,
        ) => true,
        Expr::Cond(_, t, e) => is_already_i32_expr(t) && is_already_i32_expr(e),
        Expr::Call(callee, args) if args.len() == 2 => matches!(
            callee.as_ref(),
            Expr::Member(obj, prop)
                if matches!(obj.as_ref(), Expr::Ident(id) if id == "Math") && prop == "imul"
        ),
        Expr::Call(callee, args) if args.len() == 1 => matches!(
            callee.as_ref(),
            Expr::Member(obj, prop)
                if matches!(obj.as_ref(), Expr::Ident(id) if id == "Math") && prop == "clz32"
        ),
        _ => false,
    }
}

/// Returns `true` iff `expr` is provably an exact integer in `[0, 0x7fff_ffff]` (never `-0.0`,
/// `NaN`, or negative), where signed `Int32` and unsigned `Uint32` are identical.
pub(in super::super) fn is_proven_nonneg_i31(expr: &Expr) -> bool {
    match expr {
        Expr::Num(n) => {
            n.is_finite()
                && n.fract() == 0.0
                && !(*n == 0.0 && n.is_sign_negative())
                && *n >= 0.0
                && *n <= 2147483647.0
        }
        Expr::RawNum(s) => s.parse::<u32>().ok().is_some_and(|v| v <= 0x7fff_ffff),
        Expr::Binary(BinOp::BitAnd, l, r) => is_proven_nonneg_i31(l) || is_proven_nonneg_i31(r),
        Expr::Binary(BinOp::UShr, _, r) => exact_u32(r).is_some_and(|k| (k & 31) >= 1),
        Expr::Binary(BinOp::BitOr | BinOp::BitXor, l, r) => {
            is_proven_nonneg_i31(l) && is_proven_nonneg_i31(r)
        }
        Expr::Cond(_, t, e) => is_proven_nonneg_i31(t) && is_proven_nonneg_i31(e),
        Expr::Call(callee, args) if args.len() == 1 => matches!(
            callee.as_ref(),
            Expr::Member(obj, prop)
                if matches!(obj.as_ref(), Expr::Ident(id) if id == "Math") && prop == "clz32"
        ),
        _ => false,
    }
}

fn is_proven_u32_int(expr: &Expr) -> bool {
    is_proven_nonneg_i31(expr)
        || exact_u32(expr).is_some()
        || matches!(expr, Expr::Binary(BinOp::UShr, _, _))
}

pub(super) fn is_pos_zero(expr: &Expr) -> bool {
    match expr {
        Expr::Num(n) => *n == 0.0 && !n.is_sign_negative(),
        Expr::RawNum(s) => s == "0",
        _ => false,
    }
}

/// Returns `true` iff `expr` is guaranteed to evaluate to an exact integer (never a string, bool,
/// `-0.0`, or object), so `0 + expr` and `expr + 0` are strictly equal to `expr`.
fn is_guaranteed_int_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Num(n) => {
            n.is_finite()
                && n.fract() == 0.0
                && !(*n == 0.0 && n.is_sign_negative())
                && *n >= -2147483648.0
                && *n <= 4294967295.0
        }
        Expr::RawNum(s) => s.parse::<i64>().is_ok(),
        Expr::Unary(UnOp::BitNot, _) => true,
        Expr::Binary(
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr | BinOp::UShr,
            _,
            _,
        ) => true,
        Expr::Call(callee, args) => matches!(
            callee.as_ref(),
            Expr::Member(obj, prop)
                if matches!(obj.as_ref(), Expr::Ident(id) if id == "Math")
                    && ((prop == "imul" && args.len() == 2) || (prop == "clz32" && args.len() == 1))
        ),
        Expr::Cond(_, t, e) => is_guaranteed_int_expr(t) && is_guaranteed_int_expr(e),
        _ => false,
    }
}

/// Returns `true` iff `expr` inside `arr[0 + expr]` / `arr[expr + 0]` is a numeric index expression
/// (rejecting string/bool/object/call expressions where `0 + expr` would coerce differently).
fn is_numeric_index_subexpr(expr: &Expr) -> bool {
    if is_guaranteed_int_expr(expr) {
        return true;
    }
    match expr {
        Expr::Ident(_) | Expr::Member(_, _) => true,
        Expr::Unary(UnOp::Neg, inner) => is_numeric_index_subexpr(inner),
        Expr::Binary(BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem, l, r) => {
            is_numeric_index_subexpr(l) && is_numeric_index_subexpr(r)
        }
        _ => false,
    }
}

pub(super) fn is_pure_index_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Ident(_) | Expr::Num(_) => true,
        Expr::Unary(UnOp::Neg | UnOp::BitNot, inner) => is_pure_index_expr(inner),
        Expr::Binary(
            BinOp::Add
            | BinOp::Sub
            | BinOp::BitAnd
            | BinOp::BitOr
            | BinOp::BitXor
            | BinOp::Shl
            | BinOp::Shr
            | BinOp::UShr,
            l,
            r,
        ) => is_pure_index_expr(l) && is_pure_index_expr(r),
        _ => false,
    }
}

fn strip_zero_ushr(expr: &Expr) -> Expr {
    match expr {
        Expr::Binary(BinOp::UShr, l, r) if exact_i32(r) == Some(0) => (**l).clone(),
        other => other.clone(),
    }
}

/// Unwraps double negation `!!x -> x` in contexts that implicitly apply `ToBoolean`
/// (`if (cond)`, `while (cond)`, `cond ? a : b`).
fn unwrap_double_not_cond(cond: &mut Expr) -> bool {
    let mut changed = false;
    while let Expr::Unary(UnOp::Not, inner) = cond {
        if let Expr::Unary(UnOp::Not, inner2) = inner.as_ref() {
            *cond = (**inner2).clone();
            changed = true;
        } else {
            break;
        }
    }
    changed
}

fn strip_zero_add_index(idx: &mut Expr) -> bool {
    let repl = match idx {
        Expr::Binary(BinOp::Add, il, ir) if is_pos_zero(il) && is_numeric_index_subexpr(ir) => {
            Some((**ir).clone())
        }
        Expr::Binary(BinOp::Add, il, ir) if is_pos_zero(ir) && is_numeric_index_subexpr(il) => {
            Some((**il).clone())
        }
        _ => None,
    };
    if let Some(r) = repl {
        *idx = r;
        return true;
    }
    false
}

/// Folds subexpressions of an assignment LHS without turning the LHS itself into a non-lvalue
/// (e.g. never collapsing `[a, b][0] = v` into `a = v` or `({ a }).a = v` into `a = v`).
pub(super) fn fold_lvalue_expr(lhs: &mut Expr) -> bool {
    match lhs {
        Expr::Member(obj, _) => fold_expr(obj),
        Expr::Index(arr, idx) => {
            let mut changed = fold_expr(arr);
            changed |= fold_expr(idx);
            changed |= strip_zero_add_index(idx);
            changed
        }
        other => fold_expr(other),
    }
}

fn fold_expr_children(expr: &mut Expr) -> bool {
    let mut changed = false;
    match expr {
        Expr::Member(obj, _) => changed |= fold_expr(obj),
        Expr::Index(obj, idx) => {
            changed |= fold_expr(obj);
            changed |= fold_expr(idx);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            changed |= fold_expr(c);
            for a in args {
                changed |= fold_expr(a);
            }
        }
        Expr::Unary(_, inner) => changed |= fold_expr(inner),
        Expr::Binary(_, l, r) => {
            changed |= fold_expr(l);
            changed |= fold_expr(r);
        }
        Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
            changed |= fold_lvalue_expr(l);
            changed |= fold_expr(r);
        }
        Expr::Cond(c, t, e) => {
            changed |= fold_expr(c);
            changed |= unwrap_double_not_cond(c);
            changed |= fold_expr(t);
            changed |= fold_expr(e);
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                changed |= fold_expr(v);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                changed |= fold_expr(item);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(b)) => changed |= fold_expr(b),
        Expr::Arrow(_, ArrowBody::Block(stmts)) => {
            let empty = HashSet::new();
            for s in stmts {
                changed |= fold_stmt(s, &empty);
            }
        }
        _ => {}
    }
    changed
}

/// Folds `({ k1: v1, k2: v2 }).k1` -> `v1`, `[e0, e1, ...][k]` -> `e_k` when discarded siblings
/// are pure, and normalizes `arr[0 + idx]` / `arr[idx + 0]` -> `arr[idx]`.
fn fold_member_or_index(expr: &mut Expr, changed: &mut bool) -> Option<Expr> {
    match expr {
        Expr::Member(obj, prop) => {
            let Expr::Object(entries) = obj.as_ref() else {
                return None;
            };
            let mut found: Option<&Expr> = None;
            for (k, v) in entries {
                if k == prop {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(v);
                } else if expr_has_side_effect(v) {
                    return None;
                }
            }
            found.cloned()
        }
        Expr::Index(arr, idx) => {
            *changed |= strip_zero_add_index(idx);
            let (Expr::Array(items), Some(k)) = (arr.as_ref(), exact_u32(idx)) else {
                return None;
            };
            let k = k as usize;
            if k < items.len()
                && items
                    .iter()
                    .enumerate()
                    .all(|(j, it)| j == k || !expr_has_side_effect(it))
            {
                Some(items[k].clone())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Folds `Math.imul(c1, c2)` -> exact i32 product and `Math.imul(x, 1)` / `Math.imul(1, x)` -> `x | 0`.
fn fold_call(callee: &Expr, args: &[Expr]) -> Option<Expr> {
    if args.len() != 2 {
        return None;
    }
    let is_imul = matches!(
        callee,
        Expr::Member(obj, prop)
            if matches!(obj.as_ref(), Expr::Ident(id) if id == "Math") && prop == "imul"
    );
    if !is_imul {
        return None;
    }
    let coerce_i32 = |arg: &Expr| {
        if is_already_i32_expr(arg) {
            arg.clone()
        } else {
            Expr::Binary(
                BinOp::BitOr,
                Box::new(arg.clone()),
                Box::new(Expr::Num(0.0)),
            )
        }
    };
    match (to_i32_bits(&args[0]), to_i32_bits(&args[1])) {
        (Some(a), Some(b)) => Some(num_expr_i32(a.wrapping_mul(b))),
        (None, Some(1)) => Some(coerce_i32(&args[0])),
        (Some(1), None) => Some(coerce_i32(&args[1])),
        _ => None,
    }
}

/// Folds `!true`, `!false`, and `!!bool_expr`.
fn fold_unary(op: UnOp, inner: &Expr) -> Option<Expr> {
    if op != UnOp::Not {
        return None;
    }
    match inner {
        Expr::Bool(b) => Some(Expr::Bool(!*b)),
        Expr::Unary(UnOp::Not, inner2) if is_bool_expr(inner2) => Some((**inner2).clone()),
        _ => None,
    }
}

/// Folds `true ? a : b`, `false ? a : b`, `bool_expr ? 1 : 0`, and `!bool_expr ? 0 : 1`.
fn fold_cond(cond: &Expr, then_e: &Expr, else_e: &Expr) -> Option<Expr> {
    match cond {
        Expr::Bool(true) => Some(then_e.clone()),
        Expr::Bool(false) => Some(else_e.clone()),
        _ if is_bool_expr(cond) && exact_i32(then_e) == Some(1) && exact_i32(else_e) == Some(0) => {
            Some(Expr::Binary(
                BinOp::BitOr,
                Box::new(cond.clone()),
                Box::new(Expr::Num(0.0)),
            ))
        }
        Expr::Unary(UnOp::Not, inner)
            if is_bool_expr(inner)
                && exact_i32(then_e) == Some(0)
                && exact_i32(else_e) == Some(1) =>
        {
            Some(Expr::Binary(
                BinOp::BitOr,
                Box::new((**inner).clone()),
                Box::new(Expr::Num(0.0)),
            ))
        }
        _ => None,
    }
}

fn fold_strict_eq_ne(is_eq: bool, l: &Expr, r: &Expr) -> Option<Expr> {
    if let (Some(a), Some(b)) = (SimpleLit::from_expr(l), SimpleLit::from_expr(r)) {
        let eq = a.strict_eq(&b);
        return Some(Expr::Bool(if is_eq { eq } else { !eq }));
    }
    let identity_bool = is_eq;
    match (l, r) {
        (_, Expr::Bool(b)) if is_bool_expr(l) => Some(if *b == identity_bool {
            l.clone()
        } else {
            Expr::Unary(UnOp::Not, Box::new(l.clone()))
        }),
        (Expr::Bool(b), _) if is_bool_expr(r) => Some(if *b == identity_bool {
            r.clone()
        } else {
            Expr::Unary(UnOp::Not, Box::new(r.clone()))
        }),
        _ => None,
    }
}

fn fold_add_sub(op: BinOp, l: &Expr, r: &Expr) -> Option<Expr> {
    match op {
        BinOp::Add => match (exact_i32(l), exact_i32(r)) {
            (Some(a), Some(b)) => {
                let s = (a as i64) + (b as i64);
                if (-2147483648..=2147483647).contains(&s) {
                    Some(num_expr_i32(s as i32))
                } else if (0..=4294967295).contains(&s) {
                    Some(Expr::Num(s as f64))
                } else {
                    None
                }
            }
            _ if is_pos_zero(l) && is_guaranteed_int_expr(r) => Some(r.clone()),
            _ if is_pos_zero(r) && is_guaranteed_int_expr(l) => Some(l.clone()),
            _ => None,
        },
        BinOp::Sub => match (exact_i32(l), exact_i32(r)) {
            (Some(a), Some(b)) => {
                let d = (a as i64) - (b as i64);
                if (-2147483648..=2147483647).contains(&d) {
                    Some(num_expr_i32(d as i32))
                } else {
                    None
                }
            }
            _ => None,
        },
        _ => None,
    }
}

fn fold_bit_or_zero(other: &Expr) -> Option<Expr> {
    if is_already_i32_expr(other) {
        return Some(other.clone());
    }
    match other {
        Expr::Binary(BinOp::UShr, inner, zero) if exact_i32(zero) == Some(0) => {
            Some(if is_already_i32_expr(inner) {
                (**inner).clone()
            } else {
                Expr::Binary(BinOp::BitOr, inner.clone(), Box::new(Expr::Num(0.0)))
            })
        }
        Expr::Binary(BinOp::Sub, zero, rhs)
            if is_pos_zero(zero) && !matches!(rhs.as_ref(), Expr::Unary(UnOp::Neg, _)) =>
        {
            Some(Expr::Binary(
                BinOp::BitOr,
                Box::new(Expr::Unary(UnOp::Neg, rhs.clone())),
                Box::new(Expr::Num(0.0)),
            ))
        }
        _ => None,
    }
}

fn fold_nested_bit_and(other: &Expr, outer_mask: i32) -> Option<Expr> {
    let Expr::Binary(BinOp::BitAnd, a, b) = other else {
        return None;
    };
    if let Some(m) = to_i32_bits(b) {
        Some(Expr::Binary(
            BinOp::BitAnd,
            Box::new((**a).clone()),
            Box::new(num_expr_i32(m & outer_mask)),
        ))
    } else {
        to_i32_bits(a).map(|m| {
            Expr::Binary(
                BinOp::BitAnd,
                Box::new((**b).clone()),
                Box::new(num_expr_i32(m & outer_mask)),
            )
        })
    }
}

fn fold_ushr(l: &Expr, r: &Expr) -> Option<Expr> {
    if let (Some(a), Some(b)) = (to_i32_bits(l), to_i32_bits(r)) {
        return Some(Expr::Num(((a as u32).wrapping_shr(b as u32)) as f64));
    }
    if exact_i32(r) != Some(0) {
        return None;
    }
    if matches!(l, Expr::Binary(BinOp::UShr, _, _)) || is_proven_nonneg_i31(l) {
        return Some(l.clone());
    }
    if let Expr::Binary(BinOp::BitOr, inner, zero) = l {
        return (exact_i32(zero) == Some(0))
            .then(|| Expr::Binary(BinOp::UShr, inner.clone(), Box::new(Expr::Num(0.0))));
    }
    if let Expr::Binary(BinOp::Div, dl, dr) = l {
        return match (exact_u32(dl), exact_u32(dr)) {
            (Some(a), Some(b)) if b > 0 => Some(Expr::Num((a / b) as f64)),
            (_, Some(c)) if c >= 2 && c.is_power_of_two() && is_proven_u32_int(dl) => {
                Some(Expr::Binary(
                    BinOp::UShr,
                    Box::new(strip_zero_ushr(dl)),
                    Box::new(Expr::Num(f64::from(c.trailing_zeros()))),
                ))
            }
            _ => None,
        };
    }
    None
}

fn fold_rel_cmp(op: BinOp, l: &Expr, r: &Expr) -> Option<Expr> {
    let matches_ord = |ord: Ordering| match op {
        BinOp::Lt => ord.is_lt(),
        BinOp::Le => ord.is_le(),
        BinOp::Gt => ord.is_gt(),
        BinOp::Ge => ord.is_ge(),
        _ => unreachable!(),
    };
    if let (Some(a), Some(b)) = (exact_u32(l), exact_u32(r)) {
        return Some(Expr::Bool(matches_ord(a.cmp(&b))));
    }
    if let (Some(a), Some(b)) = (exact_i32(l), exact_i32(r)) {
        return Some(Expr::Bool(matches_ord(a.cmp(&b))));
    }
    None
}

fn fold_binary(op: BinOp, l: &Expr, r: &Expr) -> Option<Expr> {
    match op {
        BinOp::StrictEq => fold_strict_eq_ne(true, l, r),
        BinOp::StrictNe => fold_strict_eq_ne(false, l, r),
        BinOp::Add | BinOp::Sub => fold_add_sub(op, l, r),
        BinOp::And => match (l, r) {
            (Expr::Bool(true), _) => Some(r.clone()),
            (Expr::Bool(false), _) => Some(Expr::Bool(false)),
            (_, Expr::Bool(true)) if is_bool_expr(l) => Some(l.clone()),
            _ => None,
        },
        BinOp::Or => match (l, r) {
            (Expr::Bool(false), _) => Some(r.clone()),
            (Expr::Bool(true), _) => Some(Expr::Bool(true)),
            (_, Expr::Bool(false)) if is_bool_expr(l) => Some(l.clone()),
            _ => None,
        },
        BinOp::BitOr => match (to_i32_bits(l), to_i32_bits(r)) {
            (Some(a), Some(b)) => Some(num_expr_i32(a | b)),
            (None, Some(0)) => fold_bit_or_zero(l),
            (Some(0), None) => fold_bit_or_zero(r),
            _ => None,
        },
        BinOp::BitAnd => match (to_i32_bits(l), to_i32_bits(r)) {
            (Some(a), Some(b)) => Some(num_expr_i32(a & b)),
            (None, Some(m2)) => fold_nested_bit_and(l, m2),
            (Some(m1), None) => fold_nested_bit_and(r, m1),
            _ => None,
        },
        BinOp::BitXor => match (to_i32_bits(l), to_i32_bits(r)) {
            (Some(a), Some(b)) => Some(num_expr_i32(a ^ b)),
            _ => None,
        },
        BinOp::Shl => match (to_i32_bits(l), to_i32_bits(r)) {
            (Some(a), Some(b)) => Some(num_expr_i32(a.wrapping_shl(b as u32))),
            _ => None,
        },
        BinOp::Shr => match (to_i32_bits(l), to_i32_bits(r)) {
            (Some(a), Some(b)) => Some(num_expr_i32(a.wrapping_shr(b as u32))),
            _ => None,
        },
        BinOp::UShr => fold_ushr(l, r),
        BinOp::Rem => match (exact_u32(l), exact_u32(r)) {
            (Some(a), Some(b)) if b > 0 => Some(Expr::Num((a % b) as f64)),
            (_, Some(c)) if c >= 2 && c.is_power_of_two() && is_proven_u32_int(l) => {
                Some(Expr::Binary(
                    BinOp::BitAnd,
                    Box::new(strip_zero_ushr(l)),
                    Box::new(Expr::Num(f64::from(c - 1))),
                ))
            }
            _ => None,
        },
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => fold_rel_cmp(op, l, r),
        _ => None,
    }
}

pub(super) fn fold_expr(expr: &mut Expr) -> bool {
    let mut changed = fold_expr_children(expr);
    let replacement: Option<Expr> = match expr {
        Expr::Member(..) | Expr::Index(..) => fold_member_or_index(expr, &mut changed),
        Expr::Call(callee, args) => fold_call(callee, args),
        Expr::Unary(op, inner) => fold_unary(*op, inner),
        Expr::Cond(cond, then_e, else_e) => fold_cond(cond, then_e, else_e),
        Expr::Binary(op, l, r) => fold_binary(*op, l, r),
        _ => None,
    };
    if let Some(repl) = replacement {
        *expr = repl;
        changed = true;
    }
    changed
}

fn stmt_definitely_exits_switch(stmt: &Stmt) -> bool {
    matches!(
        stmt,
        Stmt::Break(_) | Stmt::Continue(_) | Stmt::Return(_) | Stmt::Throw(_)
    )
}

fn try_fold_constant_switch(disc: &Expr, cases: &[jsast::SwitchCase]) -> Option<Vec<Stmt>> {
    let lit = SimpleLit::from_expr(disc)?;
    let mut default_idx: Option<usize> = None;
    let mut matched_idx: Option<usize> = None;
    for (idx, case) in cases.iter().enumerate() {
        if case.tests.is_empty() {
            default_idx = Some(idx);
            continue;
        }
        for test in &case.tests {
            let t_lit = SimpleLit::from_expr(test)?;
            if matched_idx.is_none() && lit.strict_eq(&t_lit) {
                matched_idx = Some(idx);
            }
        }
    }
    let start = match matched_idx.or(default_idx) {
        Some(i) => i,
        None => return Some(Vec::new()),
    };
    let mut collected: Vec<Stmt> = Vec::new();
    for case in &cases[start..] {
        let mut stmts = case.body.clone();
        let trailing_unlabeled_break = matches!(stmts.last(), Some(Stmt::Break(None)));
        if trailing_unlabeled_break {
            stmts.pop();
        }
        if has_unlabeled_switch_break(&stmts) {
            return None;
        }
        let exits =
            trailing_unlabeled_break || stmts.last().is_some_and(stmt_definitely_exits_switch);
        collected.extend(stmts);
        if exits {
            break;
        }
    }
    Some(collected)
}

fn negate_bool_cond(cond: &Expr) -> Expr {
    match cond {
        Expr::Unary(UnOp::Not, inner) => (**inner).clone(),
        other => Expr::Unary(UnOp::Not, Box::new(other.clone())),
    }
}

struct StmtFolder<'a> {
    local_names: &'a HashSet<String>,
    changed: bool,
}

impl VisitMut for StmtFolder<'_> {
    fn visit_block_mut(&mut self, stmts: &mut Vec<Stmt>) {
        let mut i = 0;
        while i < stmts.len() {
            self.visit_stmt_mut(&mut stmts[i]);
            match &stmts[i] {
                // Eliminate no-op local self-assignments `x = x;` (exposes empty `then`/`else` arms
                // such as `if (F < y) { F = F; } else F = y;` in `earcut`).
                Stmt::ExprStmt(Expr::Assign(lhs, rhs))
                    if matches!(
                        (lhs.as_ref(), rhs.as_ref()),
                        (Expr::Ident(a), Expr::Ident(b)) if a == b && self.local_names.contains(a)
                    ) =>
                {
                    stmts.remove(i);
                    self.changed = true;
                    continue;
                }
                Stmt::Switch(disc, cases) => {
                    if let Some(inlined) = try_fold_constant_switch(disc, cases) {
                        let count = inlined.len();
                        stmts.splice(i..=i, inlined);
                        self.changed = true;
                        i += count;
                        continue;
                    }
                }
                Stmt::If(Expr::Bool(b), then_b, else_b) => {
                    let replacement = if *b {
                        then_b.clone()
                    } else {
                        else_b.clone().unwrap_or_default()
                    };
                    let count = replacement.len();
                    stmts.splice(i..=i, replacement);
                    self.changed = true;
                    i += count;
                    continue;
                }
                Stmt::If(cond, then_b, else_b)
                    if then_b.is_empty() && else_b.as_ref().is_none_or(Vec::is_empty) =>
                {
                    if expr_has_side_effect(cond) {
                        stmts[i] = Stmt::ExprStmt(cond.clone());
                        self.changed = true;
                        i += 1;
                    } else {
                        stmts.remove(i);
                        self.changed = true;
                    }
                    continue;
                }
                Stmt::If(cond, then_b, Some(else_b)) if then_b.is_empty() && !else_b.is_empty() => {
                    let mut neg = negate_bool_cond(cond);
                    fold_expr(&mut neg);
                    unwrap_double_not_cond(&mut neg);
                    stmts[i] = Stmt::If(neg, else_b.clone(), None);
                    self.changed = true;
                    continue;
                }
                Stmt::If(_, _, Some(else_b)) if else_b.is_empty() => {
                    if let Stmt::If(_, _, eb) = &mut stmts[i] {
                        *eb = None;
                    }
                    self.changed = true;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        if matches!(stmt, Stmt::FunctionDecl { .. }) {
            return;
        }
        jsast::walk_stmt_mut(self, stmt);
        if let Stmt::If(cond, _, _) | Stmt::While(cond, _) = stmt {
            self.changed |= unwrap_double_not_cond(cond);
        }
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        self.changed |= fold_expr(expr);
    }
}

fn fold_stmt(stmt: &mut Stmt, local_names: &HashSet<String>) -> bool {
    let mut folder = StmtFolder {
        local_names,
        changed: false,
    };
    folder.visit_stmt_mut(stmt);
    folder.changed
}

pub(super) fn fold_stmt_list(stmts: &mut Vec<Stmt>, local_names: &HashSet<String>) -> bool {
    let mut folder = StmtFolder {
        local_names,
        changed: false,
    };
    folder.visit_block_mut(stmts);
    folder.changed
}
