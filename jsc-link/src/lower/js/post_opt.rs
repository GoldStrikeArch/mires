//! Post-lowering JavaScript AST speed optimizations (`ReleaseSpeed`).
//!
//! Runs after `rcgjs_object::sroa::analyze_jir` and `rcgjs_object::optimizer::analyze` inside
//! [`super::function::finish`] so passes see SROA-exposed patterns (`[1][0]`, single-assign
//! literal temps like `k = "Some"` / `E = "children"` / `ah = 14`, `Reflect.get(a, "children")`,
//! and adjacent `$__mires_i64_div` rem+div pairs).

use std::collections::{HashMap, HashSet};

use rcgjs_object::jsast::{self, ArrowBody, BinOp, Expr, Stmt, UnOp};

use super::{i64_pair, loop_opt};

/// Entry point called from unit tests (runs all speed post-opts including code-duplicating
/// loop unrolling and loop-versioned BCE).
#[cfg(test)]
pub(super) fn run_speed_post_opts(decl: &mut Stmt) {
    let empty = HashSet::new();
    run_speed_post_opts_with_mode(decl, true, &empty);
}

/// Entry point called from `function::finish` on a lowered `Stmt::FunctionDecl`.
///
/// `allow_duplicating` is `false` during inline / ABI / representation cost-pricing trials so
/// code-duplicating speed optimizations (constant-trip loop unrolling and hot/cold loop-versioned
/// BCE) do not distort size budgets or block tiny inlines (`into_iter`, `read_be`) and flat-ABI
/// slice scalarization (`compress512`, `compress256`).
pub(super) fn run_speed_post_opts_with_mode(
    decl: &mut Stmt,
    allow_duplicating: bool,
    immutable_slices: &HashSet<String>,
) {
    let Stmt::FunctionDecl { params, body, .. } = decl else {
        return;
    };
    let mut local_names: HashSet<String> = params.iter().cloned().collect();
    let mut uninit_locals: HashSet<String> = HashSet::new();
    collect_top_locals(body, &mut local_names, &mut uninit_locals);

    // 1. Fold constant expressions, constant array indexing (`[1][0]`), single-assign literal
    //    propagation (`ah = 14`, `E = "children"`, `k = "Some"`), and constant switches.
    fold_constants_and_switches(body, &uninit_locals);

    // 1b. Fast-path dead-`lo` `$__mires_i64_f64_to_bits(x)` sign-bit tests (`f64::is_sign_negative`
    //     in `f64::min`/`f64::max`), then re-fold any exposed constant branches (`c === 1`).
    if fast_path_f64_sign_bit(body) {
        fold_constants_and_switches(body, &uninit_locals);
    }

    // 1c. Fold IEEE-754 `Math.max` / `Math.min` labeled-block idioms (`d3-hierarchy`).
    if !local_names.contains("Math") {
        fold_ieee_max_min(body);
    }

    // 2. Collapse nested `if (a) { if (b) { ... } }` and fold guarded `f2i` i32 clamps (`earcut`).
    loop_opt::fold_guarded_f2i(body);

    // 3. Elide redundant `.slice()` clones on single-use fresh array literals (`compress256`).
    loop_opt::elide_dead_fresh_slice_clones(body, &local_names);

    // 4. Unroll small pure constant-trip loops (`2..=10` iters, `sha3::permute`), then re-fold
    //    exposed constant arithmetic (`Math.imul`, `% 10`, `+ 1`) and dead bounds guards.
    if allow_duplicating && loop_opt::unroll_small_const_loops(body) {
        prop_straight_line_lits(body, &local_names);
        fold_constants_and_switches(body, &uninit_locals);
    }

    // 5. Eliminate redundant dominated panic guards (`if (!aD)`, `if (!H)`, `if (bi)`) and
    //    constant-table / bitmask-proven bounds checks.
    loop_opt::eliminate_dominated_guards(body);

    // 6. Hoist loop-invariant `.buf`/`.off`/`.len` property loads across cold diverging panic
    //    calls (LICM), CSE repeated dominating `s.charCodeAt(idx)` loads, then version hot
    //    checked loops with a single exact pre-loop guard.
    let mut extra_decls: Vec<(String, Option<Expr>)> = Vec::new();
    cse_dominating_char_code_at(body, &mut local_names, &mut extra_decls);
    loop_opt::hoist_loop_invariant_props(
        body,
        &mut local_names,
        &mut extra_decls,
        immutable_slices,
    );
    if allow_duplicating {
        loop_opt::version_checked_loops(body);
    }

    // 6b. Smi-ize counted loop induction steps (`i = i + 1 >>> 0` -> `i = i + 1 | 0`) and demote
    //     bit-exact 32-bit loop-carried registers (`x + z >>> 0` -> `x + z | 0`, `compress256`).
    loop_opt::smi_ize_counted_loop_steps(body);
    loop_opt::demote_bitexact_u32_wraps(body, params);

    // 7. Spec-identical `Reflect.get(o, k)` fast-path + dominated object-guard CSE.
    if !local_names.contains("Reflect") {
        optimize_reflect_get(body);
    }

    // 8. Fuse adjacent `$__mires_i64_div(..., rem) + $__mires_i64_div(..., div)` pairs on
    //    identical unchanged operands into a single `$__mires_i64_divrem` call.
    fuse_i64_div_rem(body, &mut local_names, &mut extra_decls);

    if !extra_decls.is_empty() {
        prepend_decls(body, extra_decls);
    }
}

fn collect_top_locals(
    body: &[Stmt],
    local_names: &mut HashSet<String>,
    uninit_locals: &mut HashSet<String>,
) {
    for stmt in body {
        match stmt {
            Stmt::Let(n, init) => {
                local_names.insert(n.clone());
                if init.is_none() {
                    uninit_locals.insert(n.clone());
                }
            }
            Stmt::Const(n, _) => {
                local_names.insert(n.clone());
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    local_names.insert(n.clone());
                    if init.is_none() {
                        uninit_locals.insert(n.clone());
                    }
                }
            }
            _ => break,
        }
    }
}

fn prepend_decls(body: &mut Vec<Stmt>, extra: Vec<(String, Option<Expr>)>) {
    if let Some(Stmt::Lets(existing)) = body.first_mut() {
        existing.extend(extra);
    } else {
        body.insert(0, jsast::lets(extra));
    }
}

pub(super) fn alloc_fresh_local(
    local_names: &mut HashSet<String>,
    extra_decls: &mut Vec<(String, Option<Expr>)>,
    prefix: &str,
) -> String {
    let mut n = 0usize;
    loop {
        let name = format!("{prefix}{n}");
        if local_names.insert(name.clone()) {
            extra_decls.push((name.clone(), None));
            return name;
        }
        n += 1;
    }
}

// =============================================================================================
// 1. Constant folding, single-assign literal propagation, and constant switch simplification
// =============================================================================================

#[derive(Clone, Debug, PartialEq)]
enum SimpleLit {
    Num(f64),
    Str(String),
    Bool(bool),
    Null,
    Undefined,
}

impl SimpleLit {
    fn from_expr(expr: &Expr) -> Option<Self> {
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

    fn to_expr(&self) -> Expr {
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

    fn strict_eq(&self, other: &Self) -> bool {
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

pub(super) fn exact_i32(expr: &Expr) -> Option<i32> {
    let n = match expr {
        Expr::Num(n) => *n,
        Expr::Unary(UnOp::Neg, inner) => match inner.as_ref() {
            Expr::Num(n) => -*n,
            _ => return None,
        },
        _ => return None,
    };
    if n.is_finite() && n.fract() == 0.0 && (-2147483648.0..=2147483647.0).contains(&n) {
        Some(n as i32)
    } else {
        None
    }
}

pub(super) fn exact_u32(expr: &Expr) -> Option<u32> {
    let n = match expr {
        Expr::Num(n) => *n,
        _ => return None,
    };
    if n.is_finite() && n.fract() == 0.0 && (0.0..=4294967295.0).contains(&n) {
        Some(n as u32)
    } else {
        None
    }
}

fn to_i32_bits(expr: &Expr) -> Option<i32> {
    exact_i32(expr).or_else(|| exact_u32(expr).map(|u| u as i32))
}

fn num_expr_i32(v: i32) -> Expr {
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

pub(super) fn is_already_i32_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Unary(UnOp::BitNot, _) => true,
        Expr::Binary(
            BinOp::BitOr | BinOp::BitAnd | BinOp::BitXor | BinOp::Shl | BinOp::Shr,
            _,
            _,
        ) => true,
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
pub(super) fn is_proven_nonneg_i31(expr: &Expr) -> bool {
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

fn is_pos_zero(expr: &Expr) -> bool {
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

fn strip_zero_ushr(expr: &Expr) -> Expr {
    match expr {
        Expr::Binary(BinOp::UShr, l, r) if exact_i32(r) == Some(0) => (**l).clone(),
        other => other.clone(),
    }
}

fn fold_expr(expr: &mut Expr) -> bool {
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
        Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
            changed |= fold_expr(l);
            changed |= fold_expr(r);
        }
        Expr::Cond(c, t, e) => {
            changed |= fold_expr(c);
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
            for s in stmts {
                changed |= fold_stmt(s);
            }
        }
        _ => {}
    }

    // Apply local expression reductions.
    let replacement: Option<Expr> = match expr {
        // `({ k1: v1, k2: v2 }).k1` -> `v1` when all other property values are pure (collapses
        // inline `tail_ref` `{ ptr: e.ptr.buf[e.ptr.off], meta: e.meta }.ptr` in `Rc::clone`).
        Expr::Member(obj, prop) => {
            if let Expr::Object(entries) = obj.as_ref() {
                let mut found: Option<&Expr> = None;
                let mut ok = true;
                for (k, v) in entries {
                    if k == prop {
                        if found.is_some() {
                            ok = false;
                            break;
                        }
                        found = Some(v);
                    } else if loop_opt::expr_has_side_effect(v) {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    found.cloned()
                } else {
                    None
                }
            } else {
                None
            }
        }
        // `[c0, c1, ...][k]` where every element is a pure literal -> `c_k`, and
        // `arr[0 + idx]` / `arr[idx + 0]` -> `arr[idx]` for numeric index subexpressions.
        Expr::Index(arr, idx) => {
            if let Expr::Binary(BinOp::Add, il, ir) = idx.as_ref() {
                if is_pos_zero(il) && is_numeric_index_subexpr(ir) {
                    **idx = (**ir).clone();
                    changed = true;
                } else if is_pos_zero(ir) && is_numeric_index_subexpr(il) {
                    **idx = (**il).clone();
                    changed = true;
                }
            }
            if let (Expr::Array(items), Some(k)) = (arr.as_ref(), exact_u32(idx)) {
                let k = k as usize;
                if k < items.len() && items.iter().all(|it| SimpleLit::from_expr(it).is_some()) {
                    Some(items[k].clone())
                } else {
                    None
                }
            } else {
                None
            }
        }
        // `Math.imul(c1, c2)` -> exact i32 product; `Math.imul(x, 1)` / `Math.imul(1, x)` -> `x | 0`
        Expr::Call(callee, args) if args.len() == 2 => {
            let is_imul = matches!(
                callee.as_ref(),
                Expr::Member(obj, prop)
                    if matches!(obj.as_ref(), Expr::Ident(id) if id == "Math")
                        && prop == "imul"
            );
            if is_imul {
                match (to_i32_bits(&args[0]), to_i32_bits(&args[1])) {
                    (Some(a), Some(b)) => Some(num_expr_i32(a.wrapping_mul(b))),
                    (None, Some(1)) => Some(if is_already_i32_expr(&args[0]) {
                        args[0].clone()
                    } else {
                        Expr::Binary(
                            BinOp::BitOr,
                            Box::new(args[0].clone()),
                            Box::new(Expr::Num(0.0)),
                        )
                    }),
                    (Some(1), None) => Some(if is_already_i32_expr(&args[1]) {
                        args[1].clone()
                    } else {
                        Expr::Binary(
                            BinOp::BitOr,
                            Box::new(args[1].clone()),
                            Box::new(Expr::Num(0.0)),
                        )
                    }),
                    _ => None,
                }
            } else {
                None
            }
        }
        // `!true` / `!false` / `!!bool_expr`
        Expr::Unary(UnOp::Not, inner) => match inner.as_ref() {
            Expr::Bool(b) => Some(Expr::Bool(!*b)),
            Expr::Unary(UnOp::Not, inner2) if is_bool_expr(inner2) => Some((**inner2).clone()),
            _ => None,
        },
        // `true ? a : b` / `false ? a : b` / `bool_expr ? 1 : 0` -> `bool_expr | 0`
        Expr::Cond(cond, then_e, else_e) => match cond.as_ref() {
            Expr::Bool(true) => Some((**then_e).clone()),
            Expr::Bool(false) => Some((**else_e).clone()),
            _ if is_bool_expr(cond)
                && exact_i32(then_e) == Some(1)
                && exact_i32(else_e) == Some(0) =>
            {
                Some(Expr::Binary(
                    BinOp::BitOr,
                    Box::new((**cond).clone()),
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
        },
        // Binary constant folds
        Expr::Binary(op, l, r) => match op {
            BinOp::StrictEq => match (SimpleLit::from_expr(l), SimpleLit::from_expr(r)) {
                (Some(a), Some(b)) => Some(Expr::Bool(a.strict_eq(&b))),
                _ => match (l.as_ref(), r.as_ref()) {
                    (_, Expr::Bool(true)) if is_bool_expr(l) => Some((**l).clone()),
                    (Expr::Bool(true), _) if is_bool_expr(r) => Some((**r).clone()),
                    (_, Expr::Bool(false)) if is_bool_expr(l) => {
                        Some(Expr::Unary(UnOp::Not, Box::new((**l).clone())))
                    }
                    (Expr::Bool(false), _) if is_bool_expr(r) => {
                        Some(Expr::Unary(UnOp::Not, Box::new((**r).clone())))
                    }
                    _ => None,
                },
            },
            BinOp::StrictNe => match (SimpleLit::from_expr(l), SimpleLit::from_expr(r)) {
                (Some(a), Some(b)) => Some(Expr::Bool(!a.strict_eq(&b))),
                _ => match (l.as_ref(), r.as_ref()) {
                    (_, Expr::Bool(false)) if is_bool_expr(l) => Some((**l).clone()),
                    (Expr::Bool(false), _) if is_bool_expr(r) => Some((**r).clone()),
                    (_, Expr::Bool(true)) if is_bool_expr(l) => {
                        Some(Expr::Unary(UnOp::Not, Box::new((**l).clone())))
                    }
                    (Expr::Bool(true), _) if is_bool_expr(r) => {
                        Some(Expr::Unary(UnOp::Not, Box::new((**r).clone())))
                    }
                    _ => None,
                },
            },
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
                _ if is_pos_zero(l) && is_guaranteed_int_expr(r) => Some((**r).clone()),
                _ if is_pos_zero(r) && is_guaranteed_int_expr(l) => Some((**l).clone()),
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
            BinOp::BitOr => match (to_i32_bits(l), to_i32_bits(r)) {
                (Some(a), Some(b)) => Some(num_expr_i32(a | b)),
                (None, Some(0)) if is_already_i32_expr(l) => Some((**l).clone()),
                (Some(0), None) if is_already_i32_expr(r) => Some((**r).clone()),
                (None, Some(0)) => match l.as_ref() {
                    Expr::Binary(BinOp::UShr, inner, zero) if exact_i32(zero) == Some(0) => {
                        Some(if is_already_i32_expr(inner) {
                            (**inner).clone()
                        } else {
                            Expr::Binary(BinOp::BitOr, inner.clone(), Box::new(Expr::Num(0.0)))
                        })
                    }
                    _ => None,
                },
                (Some(0), None) => match r.as_ref() {
                    Expr::Binary(BinOp::UShr, inner, zero) if exact_i32(zero) == Some(0) => {
                        Some(if is_already_i32_expr(inner) {
                            (**inner).clone()
                        } else {
                            Expr::Binary(BinOp::BitOr, inner.clone(), Box::new(Expr::Num(0.0)))
                        })
                    }
                    _ => None,
                },
                _ => None,
            },
            BinOp::BitAnd => match (to_i32_bits(l), to_i32_bits(r)) {
                (Some(a), Some(b)) => Some(num_expr_i32(a & b)),
                (None, Some(m2)) => match l.as_ref() {
                    Expr::Binary(BinOp::BitAnd, a, b) => {
                        if let Some(m1) = to_i32_bits(b) {
                            Some(Expr::Binary(
                                BinOp::BitAnd,
                                Box::new((**a).clone()),
                                Box::new(num_expr_i32(m1 & m2)),
                            ))
                        } else {
                            to_i32_bits(a).map(|m1| {
                                Expr::Binary(
                                    BinOp::BitAnd,
                                    Box::new((**b).clone()),
                                    Box::new(num_expr_i32(m1 & m2)),
                                )
                            })
                        }
                    }
                    _ => None,
                },
                (Some(m1), None) => match r.as_ref() {
                    Expr::Binary(BinOp::BitAnd, a, b) => {
                        if let Some(m2) = to_i32_bits(b) {
                            Some(Expr::Binary(
                                BinOp::BitAnd,
                                Box::new((**a).clone()),
                                Box::new(num_expr_i32(m1 & m2)),
                            ))
                        } else {
                            to_i32_bits(a).map(|m2| {
                                Expr::Binary(
                                    BinOp::BitAnd,
                                    Box::new((**b).clone()),
                                    Box::new(num_expr_i32(m1 & m2)),
                                )
                            })
                        }
                    }
                    _ => None,
                },
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
            BinOp::UShr => match (to_i32_bits(l), to_i32_bits(r)) {
                (Some(a), Some(b)) => Some(Expr::Num(((a as u32).wrapping_shr(b as u32)) as f64)),
                _ if exact_i32(r) == Some(0) => {
                    if matches!(l.as_ref(), Expr::Binary(BinOp::UShr, _, _))
                        || is_proven_nonneg_i31(l)
                    {
                        Some(l.as_ref().clone())
                    } else if let Expr::Binary(BinOp::BitOr, inner, zero) = l.as_ref() {
                        if exact_i32(zero) == Some(0) {
                            Some(Expr::Binary(
                                BinOp::UShr,
                                inner.clone(),
                                Box::new(Expr::Num(0.0)),
                            ))
                        } else {
                            None
                        }
                    } else if let Expr::Binary(BinOp::Div, dl, dr) = l.as_ref() {
                        match (exact_u32(dl), exact_u32(dr)) {
                            (Some(a), Some(b)) if b > 0 => Some(Expr::Num((a / b) as f64)),
                            (_, Some(c))
                                if c >= 2 && c.is_power_of_two() && is_proven_u32_int(dl) =>
                            {
                                Some(Expr::Binary(
                                    BinOp::UShr,
                                    Box::new(strip_zero_ushr(dl)),
                                    Box::new(Expr::Num(f64::from(c.trailing_zeros()))),
                                ))
                            }
                            _ => None,
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            },
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
            BinOp::Lt => match (exact_u32(l), exact_u32(r)) {
                (Some(a), Some(b)) => Some(Expr::Bool(a < b)),
                _ => match (exact_i32(l), exact_i32(r)) {
                    (Some(a), Some(b)) => Some(Expr::Bool(a < b)),
                    _ => None,
                },
            },
            BinOp::Le => match (exact_u32(l), exact_u32(r)) {
                (Some(a), Some(b)) => Some(Expr::Bool(a <= b)),
                _ => match (exact_i32(l), exact_i32(r)) {
                    (Some(a), Some(b)) => Some(Expr::Bool(a <= b)),
                    _ => None,
                },
            },
            BinOp::Gt => match (exact_u32(l), exact_u32(r)) {
                (Some(a), Some(b)) => Some(Expr::Bool(a > b)),
                _ => match (exact_i32(l), exact_i32(r)) {
                    (Some(a), Some(b)) => Some(Expr::Bool(a > b)),
                    _ => None,
                },
            },
            BinOp::Ge => match (exact_u32(l), exact_u32(r)) {
                (Some(a), Some(b)) => Some(Expr::Bool(a >= b)),
                _ => match (exact_i32(l), exact_i32(r)) {
                    (Some(a), Some(b)) => Some(Expr::Bool(a >= b)),
                    _ => None,
                },
            },
            _ => None,
        },
        _ => None,
    };
    if let Some(repl) = replacement {
        *expr = repl;
        changed = true;
    }
    changed
}

fn has_unlabeled_switch_break(stmts: &[Stmt]) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Break(None) => return true,
            Stmt::If(_, a, b) => {
                if has_unlabeled_switch_break(a)
                    || b.as_ref().is_some_and(|b| has_unlabeled_switch_break(b))
                {
                    return true;
                }
            }
            Stmt::Block(b) => {
                if has_unlabeled_switch_break(b) {
                    return true;
                }
            }
            Stmt::Labeled(_, inner) => {
                if has_unlabeled_switch_break(std::slice::from_ref(inner.as_ref())) {
                    return true;
                }
            }
            Stmt::TryCatch { body, handler, .. } => {
                if has_unlabeled_switch_break(body) || has_unlabeled_switch_break(handler) {
                    return true;
                }
            }
            // Inner loops and switches capture their own unlabeled `break`.
            Stmt::LoopForever(..) | Stmt::While(..) | Stmt::Switch(..) => {}
            _ => {}
        }
    }
    false
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

fn fold_stmt(stmt: &mut Stmt) -> bool {
    let mut changed = false;
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Const(_, e) | Stmt::Let(_, Some(e)) => {
            changed |= fold_expr(e);
        }
        Stmt::Return(Some(e)) => changed |= fold_expr(e),
        Stmt::Lets(decls) => {
            for (_, init) in decls {
                if let Some(e) = init {
                    changed |= fold_expr(e);
                }
            }
        }
        Stmt::If(cond, then_b, else_b) => {
            changed |= fold_expr(cond);
            changed |= fold_stmt_list(then_b);
            if let Some(eb) = else_b {
                changed |= fold_stmt_list(eb);
            }
        }
        Stmt::Switch(disc, cases) => {
            changed |= fold_expr(disc);
            for c in cases {
                for t in &mut c.tests {
                    changed |= fold_expr(t);
                }
                changed |= fold_stmt_list(&mut c.body);
            }
        }
        Stmt::Labeled(_, inner) => changed |= fold_stmt(inner),
        Stmt::LoopForever(_, b) | Stmt::Block(b) => changed |= fold_stmt_list(b),
        Stmt::While(t, b) => {
            changed |= fold_expr(t);
            changed |= fold_stmt_list(b);
        }
        Stmt::TryCatch { body, handler, .. } => {
            changed |= fold_stmt_list(body);
            changed |= fold_stmt_list(handler);
        }
        _ => {}
    }
    changed
}

fn fold_stmt_list(stmts: &mut Vec<Stmt>) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i < stmts.len() {
        changed |= fold_stmt(&mut stmts[i]);
        match &stmts[i] {
            Stmt::Switch(disc, cases) => {
                if let Some(inlined) = try_fold_constant_switch(disc, cases) {
                    let count = inlined.len();
                    stmts.splice(i..=i, inlined);
                    changed = true;
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
                changed = true;
                i += count;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    changed
}

#[derive(Default)]
struct LocalConstUsage {
    writes: usize,
    reads: usize,
    in_closure: bool,
}

fn scan_const_usage_expr(
    expr: &Expr,
    locals: &HashSet<String>,
    in_closure: bool,
    usage: &mut HashMap<String, LocalConstUsage>,
) {
    match expr {
        Expr::Ident(n) => {
            if locals.contains(n) {
                let u = usage.entry(n.clone()).or_default();
                u.reads += 1;
                u.in_closure |= in_closure;
            }
        }
        Expr::Assign(lhs, rhs) => {
            if let Expr::Ident(n) = lhs.as_ref() {
                if locals.contains(n) {
                    let u = usage.entry(n.clone()).or_default();
                    u.writes += 1;
                    u.in_closure |= in_closure;
                }
            } else {
                scan_const_usage_expr(lhs, locals, in_closure, usage);
            }
            scan_const_usage_expr(rhs, locals, in_closure, usage);
        }
        Expr::CompoundAssign(_, lhs, rhs) => {
            if let Expr::Ident(n) = lhs.as_ref() {
                if locals.contains(n) {
                    let u = usage.entry(n.clone()).or_default();
                    u.writes += 1;
                    u.reads += 1;
                    u.in_closure |= in_closure;
                }
            } else {
                scan_const_usage_expr(lhs, locals, in_closure, usage);
            }
            scan_const_usage_expr(rhs, locals, in_closure, usage);
        }
        Expr::Member(o, _) => scan_const_usage_expr(o, locals, in_closure, usage),
        Expr::Index(o, idx) => {
            scan_const_usage_expr(o, locals, in_closure, usage);
            scan_const_usage_expr(idx, locals, in_closure, usage);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            scan_const_usage_expr(c, locals, in_closure, usage);
            for a in args {
                scan_const_usage_expr(a, locals, in_closure, usage);
            }
        }
        Expr::Unary(_, e) => scan_const_usage_expr(e, locals, in_closure, usage),
        Expr::Binary(_, l, r) => {
            scan_const_usage_expr(l, locals, in_closure, usage);
            scan_const_usage_expr(r, locals, in_closure, usage);
        }
        Expr::Cond(c, t, e) => {
            scan_const_usage_expr(c, locals, in_closure, usage);
            scan_const_usage_expr(t, locals, in_closure, usage);
            scan_const_usage_expr(e, locals, in_closure, usage);
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                scan_const_usage_expr(v, locals, in_closure, usage);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for it in items {
                scan_const_usage_expr(it, locals, in_closure, usage);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(b)) => scan_const_usage_expr(b, locals, true, usage),
        Expr::Arrow(_, ArrowBody::Block(s)) => scan_const_usage_stmts(s, locals, true, usage),
        _ => {}
    }
}

fn scan_const_usage_stmts(
    stmts: &[Stmt],
    locals: &HashSet<String>,
    in_closure: bool,
    usage: &mut HashMap<String, LocalConstUsage>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                scan_const_usage_expr(e, locals, in_closure, usage);
            }
            Stmt::Let(n, Some(e)) | Stmt::Const(n, e) => {
                if locals.contains(n) {
                    let u = usage.entry(n.clone()).or_default();
                    u.writes += 1;
                    u.in_closure |= in_closure;
                }
                scan_const_usage_expr(e, locals, in_closure, usage);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(e) = init {
                        if locals.contains(n) {
                            let u = usage.entry(n.clone()).or_default();
                            u.writes += 1;
                            u.in_closure |= in_closure;
                        }
                        scan_const_usage_expr(e, locals, in_closure, usage);
                    }
                }
            }
            Stmt::If(t, a, b) => {
                scan_const_usage_expr(t, locals, in_closure, usage);
                scan_const_usage_stmts(a, locals, in_closure, usage);
                if let Some(b) = b {
                    scan_const_usage_stmts(b, locals, in_closure, usage);
                }
            }
            Stmt::Switch(d, cases) => {
                scan_const_usage_expr(d, locals, in_closure, usage);
                for c in cases {
                    for t in &c.tests {
                        scan_const_usage_expr(t, locals, in_closure, usage);
                    }
                    scan_const_usage_stmts(&c.body, locals, in_closure, usage);
                }
            }
            Stmt::Labeled(_, inner) => {
                scan_const_usage_stmts(
                    std::slice::from_ref(inner.as_ref()),
                    locals,
                    in_closure,
                    usage,
                );
            }
            Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                scan_const_usage_stmts(b, locals, in_closure, usage);
            }
            Stmt::While(t, b) => {
                scan_const_usage_expr(t, locals, in_closure, usage);
                scan_const_usage_stmts(b, locals, in_closure, usage);
            }
            Stmt::TryCatch { body, handler, .. } => {
                scan_const_usage_stmts(body, locals, in_closure, usage);
                scan_const_usage_stmts(handler, locals, in_closure, usage);
            }
            Stmt::FunctionDecl { body, .. } => scan_const_usage_stmts(body, locals, true, usage),
            _ => {}
        }
    }
}

fn count_ident_reads_expr(expr: &Expr, target: &str) -> usize {
    match expr {
        Expr::Ident(n) => usize::from(n == target),
        Expr::Assign(lhs, rhs) => {
            let l = if matches!(lhs.as_ref(), Expr::Ident(n) if n == target) {
                0
            } else {
                count_ident_reads_expr(lhs, target)
            };
            l + count_ident_reads_expr(rhs, target)
        }
        Expr::CompoundAssign(_, lhs, rhs) => {
            count_ident_reads_expr(lhs, target) + count_ident_reads_expr(rhs, target)
        }
        Expr::Member(o, _) => count_ident_reads_expr(o, target),
        Expr::Index(o, i) => count_ident_reads_expr(o, target) + count_ident_reads_expr(i, target),
        Expr::Call(c, args) | Expr::New(c, args) => {
            count_ident_reads_expr(c, target)
                + args
                    .iter()
                    .map(|a| count_ident_reads_expr(a, target))
                    .sum::<usize>()
        }
        Expr::Unary(_, e) => count_ident_reads_expr(e, target),
        Expr::Binary(_, l, r) => {
            count_ident_reads_expr(l, target) + count_ident_reads_expr(r, target)
        }
        Expr::Cond(c, t, e) => {
            count_ident_reads_expr(c, target)
                + count_ident_reads_expr(t, target)
                + count_ident_reads_expr(e, target)
        }
        Expr::Object(entries) => entries
            .iter()
            .map(|(_, v)| count_ident_reads_expr(v, target))
            .sum(),
        Expr::Array(items) | Expr::Seq(items) => items
            .iter()
            .map(|x| count_ident_reads_expr(x, target))
            .sum(),
        Expr::Arrow(_, ArrowBody::Expr(b)) => count_ident_reads_expr(b, target),
        Expr::Arrow(_, ArrowBody::Block(s)) => count_ident_reads_stmts(s, target),
        _ => 0,
    }
}

fn count_ident_reads_stmts(stmts: &[Stmt], target: &str) -> usize {
    let mut n = 0;
    for s in stmts {
        n += match s {
            Stmt::ExprStmt(e)
            | Stmt::Throw(e)
            | Stmt::Const(_, e)
            | Stmt::Let(_, Some(e))
            | Stmt::Return(Some(e)) => count_ident_reads_expr(e, target),
            Stmt::Lets(decls) => decls
                .iter()
                .filter_map(|(_, init)| init.as_ref())
                .map(|e| count_ident_reads_expr(e, target))
                .sum(),
            Stmt::If(t, a, b) => {
                count_ident_reads_expr(t, target)
                    + count_ident_reads_stmts(a, target)
                    + b.as_ref().map_or(0, |b| count_ident_reads_stmts(b, target))
            }
            Stmt::Switch(d, cases) => {
                count_ident_reads_expr(d, target)
                    + cases
                        .iter()
                        .map(|c| {
                            c.tests
                                .iter()
                                .map(|t| count_ident_reads_expr(t, target))
                                .sum::<usize>()
                                + count_ident_reads_stmts(&c.body, target)
                        })
                        .sum::<usize>()
            }
            Stmt::Labeled(_, inner) => {
                count_ident_reads_stmts(std::slice::from_ref(inner.as_ref()), target)
            }
            Stmt::LoopForever(_, b) | Stmt::Block(b) => count_ident_reads_stmts(b, target),
            Stmt::While(t, b) => {
                count_ident_reads_expr(t, target) + count_ident_reads_stmts(b, target)
            }
            Stmt::TryCatch { body, handler, .. } => {
                count_ident_reads_stmts(body, target) + count_ident_reads_stmts(handler, target)
            }
            Stmt::FunctionDecl { body, .. } => count_ident_reads_stmts(body, target),
            _ => 0,
        };
    }
    n
}

fn subst_ident_expr(expr: &mut Expr, target: &str, repl: &Expr) {
    if matches!(expr, Expr::Ident(n) if n == target) {
        *expr = repl.clone();
        return;
    }
    match expr {
        Expr::Assign(lhs, rhs) => {
            if !matches!(lhs.as_ref(), Expr::Ident(n) if n == target) {
                subst_ident_expr(lhs, target, repl);
            }
            subst_ident_expr(rhs, target, repl);
        }
        Expr::CompoundAssign(_, lhs, rhs) => {
            subst_ident_expr(lhs, target, repl);
            subst_ident_expr(rhs, target, repl);
        }
        Expr::Member(o, _) => subst_ident_expr(o, target, repl),
        Expr::Index(o, i) => {
            subst_ident_expr(o, target, repl);
            subst_ident_expr(i, target, repl);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            subst_ident_expr(c, target, repl);
            for a in args {
                subst_ident_expr(a, target, repl);
            }
        }
        Expr::Unary(_, e) => subst_ident_expr(e, target, repl),
        Expr::Binary(_, l, r) => {
            subst_ident_expr(l, target, repl);
            subst_ident_expr(r, target, repl);
        }
        Expr::Cond(c, t, e) => {
            subst_ident_expr(c, target, repl);
            subst_ident_expr(t, target, repl);
            subst_ident_expr(e, target, repl);
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                subst_ident_expr(v, target, repl);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for it in items {
                subst_ident_expr(it, target, repl);
            }
        }
        _ => {}
    }
}

fn subst_ident_stmts(stmts: &mut [Stmt], target: &str, repl: &Expr) {
    for stmt in stmts {
        match stmt {
            Stmt::ExprStmt(e)
            | Stmt::Throw(e)
            | Stmt::Const(_, e)
            | Stmt::Let(_, Some(e))
            | Stmt::Return(Some(e)) => subst_ident_expr(e, target, repl),
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(e) = init {
                        subst_ident_expr(e, target, repl);
                    }
                }
            }
            Stmt::If(t, a, b) => {
                subst_ident_expr(t, target, repl);
                subst_ident_stmts(a, target, repl);
                if let Some(b) = b {
                    subst_ident_stmts(b, target, repl);
                }
            }
            Stmt::Switch(d, cases) => {
                subst_ident_expr(d, target, repl);
                for c in cases {
                    for t in &mut c.tests {
                        subst_ident_expr(t, target, repl);
                    }
                    subst_ident_stmts(&mut c.body, target, repl);
                }
            }
            Stmt::Labeled(_, inner) => {
                subst_ident_stmts(std::slice::from_mut(inner.as_mut()), target, repl);
            }
            Stmt::LoopForever(_, b) | Stmt::Block(b) => subst_ident_stmts(b, target, repl),
            Stmt::While(t, b) => {
                subst_ident_expr(t, target, repl);
                subst_ident_stmts(b, target, repl);
            }
            Stmt::TryCatch { body, handler, .. } => {
                subst_ident_stmts(body, target, repl);
                subst_ident_stmts(handler, target, repl);
            }
            _ => {}
        }
    }
}

/// Propagates single-assignment uninit local literals whose definition strictly dominates every
/// read (all reads lie in the trailing sibling statements `stmts[i + 1..]`).
fn prop_local_lits_in_block(
    stmts: &mut Vec<Stmt>,
    single_write_reads: &HashMap<String, usize>,
) -> bool {
    let mut changed = false;
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::If(_, a, b) => {
                changed |= prop_local_lits_in_block(a, single_write_reads);
                if let Some(b) = b {
                    changed |= prop_local_lits_in_block(b, single_write_reads);
                }
            }
            Stmt::Switch(_, cases) => {
                for c in cases {
                    changed |= prop_local_lits_in_block(&mut c.body, single_write_reads);
                }
            }
            Stmt::Labeled(_, inner) => {
                let mut tmp = vec![inner.as_ref().clone()];
                if prop_local_lits_in_block(&mut tmp, single_write_reads) && tmp.len() == 1 {
                    *inner.as_mut() = tmp.pop().unwrap();
                    changed = true;
                }
            }
            Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::While(_, b) => {
                changed |= prop_local_lits_in_block(b, single_write_reads);
            }
            Stmt::TryCatch { body, handler, .. } => {
                changed |= prop_local_lits_in_block(body, single_write_reads);
                changed |= prop_local_lits_in_block(handler, single_write_reads);
            }
            _ => {}
        }
    }

    let mut i = 0;
    while i < stmts.len() {
        // Handle both standalone `x = lit;` and comma-sequence `a = lit1, b = lit2;`
        if let Stmt::ExprStmt(Expr::Assign(lhs, rhs)) = &stmts[i] {
            if let (Expr::Ident(name), Some(lit)) = (lhs.as_ref(), SimpleLit::from_expr(rhs)) {
                if let Some(&total_reads) = single_write_reads.get(name) {
                    // Only duplicate strings when single-use or very short (avoids bundle bloat).
                    let worth_inlining = match &lit {
                        SimpleLit::Str(s) => total_reads <= 2 || s.len() <= 8,
                        _ => total_reads <= 6,
                    };
                    if worth_inlining
                        && count_ident_reads_stmts(&stmts[i + 1..], name) == total_reads
                    {
                        let target = name.clone();
                        let repl = lit.to_expr();
                        subst_ident_stmts(&mut stmts[i + 1..], &target, &repl);
                        stmts.remove(i);
                        changed = true;
                        continue;
                    }
                }
            }
        } else if let Stmt::ExprStmt(Expr::Seq(items)) = &stmts[i] {
            // Check if every element in `Seq` is an assignment `name = expr`, and any qualifying
            // literal element has all its reads in `stmts[i + 1..]` (and not in earlier/later
            // elements of the same `Seq`).
            let mut extracted: Vec<(usize, String, SimpleLit)> = Vec::new();
            for (k, item) in items.iter().enumerate() {
                if let Expr::Assign(lhs, rhs) = item {
                    if let (Expr::Ident(name), Some(lit)) =
                        (lhs.as_ref(), SimpleLit::from_expr(rhs))
                    {
                        if let Some(&total_reads) = single_write_reads.get(name) {
                            let worth_inlining = match &lit {
                                SimpleLit::Str(s) => total_reads <= 2 || s.len() <= 8,
                                _ => total_reads <= 6,
                            };
                            let seq_reads: usize = items
                                .iter()
                                .map(|it| count_ident_reads_expr(it, name))
                                .sum();
                            if worth_inlining
                                && seq_reads == 0
                                && count_ident_reads_stmts(&stmts[i + 1..], name) == total_reads
                            {
                                extracted.push((k, name.clone(), lit));
                            }
                        }
                    }
                }
            }
            if !extracted.is_empty() {
                let remove_set: HashSet<usize> = extracted.iter().map(|(k, _, _)| *k).collect();
                for (_, name, lit) in &extracted {
                    subst_ident_stmts(&mut stmts[i + 1..], name, &lit.to_expr());
                }
                if let Stmt::ExprStmt(Expr::Seq(items)) = &mut stmts[i] {
                    let mut idx = 0usize;
                    items.retain(|_| {
                        let keep = !remove_set.contains(&idx);
                        idx += 1;
                        keep
                    });
                    if items.is_empty() {
                        stmts.remove(i);
                        changed = true;
                        continue;
                    } else if items.len() == 1 {
                        let sole = items.pop().unwrap();
                        stmts[i] = Stmt::ExprStmt(sole);
                    }
                }
                changed = true;
                continue;
            }
        }
        i += 1;
    }
    changed
}

fn fold_constants_and_switches(body: &mut Vec<Stmt>, uninit_locals: &HashSet<String>) {
    for _ in 0..3 {
        let mut changed = fold_stmt_list(body);
        if !uninit_locals.is_empty() {
            let mut usage = HashMap::new();
            scan_const_usage_stmts(body, uninit_locals, false, &mut usage);
            let single_write_reads: HashMap<String, usize> = usage
                .into_iter()
                .filter(|(_, u)| u.writes == 1 && u.reads >= 1 && !u.in_closure)
                .map(|(n, u)| (n, u.reads))
                .collect();
            if !single_write_reads.is_empty() {
                changed |= prop_local_lits_in_block(body, &single_write_reads);
            }
        }
        if !changed {
            break;
        }
    }
}

/// Forward straight-line constant propagation for unrolled loop bodies (where temp locals like
/// `s`, `H`, `t`, `o`, `z` are assigned multiple times across unrolled iterations).
fn prop_straight_line_lits(body: &mut [Stmt], local_names: &HashSet<String>) {
    fn subst_env_expr(expr: &mut Expr, env: &mut HashMap<String, SimpleLit>) {
        match expr {
            Expr::Ident(id) => {
                if let Some(lit) = env.get(id) {
                    *expr = lit.to_expr();
                }
            }
            Expr::Assign(lhs, rhs) => {
                subst_env_expr(rhs, env);
                fold_expr(rhs);
                if let Expr::Ident(id) = lhs.as_ref() {
                    env.remove(id);
                } else {
                    subst_env_expr(lhs, env);
                    fold_expr(lhs);
                }
            }
            Expr::CompoundAssign(_, lhs, rhs) => {
                subst_env_expr(rhs, env);
                fold_expr(rhs);
                if let Expr::Ident(id) = lhs.as_ref() {
                    env.remove(id);
                } else {
                    subst_env_expr(lhs, env);
                    fold_expr(lhs);
                }
            }
            Expr::Unary(_, e) | Expr::Member(e, _) => subst_env_expr(e, env),
            Expr::Binary(BinOp::And | BinOp::Or, l, r) => {
                subst_env_expr(l, env);
                let mut r_env = env.clone();
                subst_env_expr(r, &mut r_env);
                env.retain(|k, v| r_env.get(k) == Some(v));
            }
            Expr::Binary(_, l, r) | Expr::Index(l, r) => {
                subst_env_expr(l, env);
                subst_env_expr(r, env);
            }
            Expr::Cond(c, t, e) => {
                subst_env_expr(c, env);
                let mut t_env = env.clone();
                subst_env_expr(t, &mut t_env);
                let mut e_env = env.clone();
                subst_env_expr(e, &mut e_env);
                env.retain(|k, v| t_env.get(k) == Some(v) && e_env.get(k) == Some(v));
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                subst_env_expr(c, env);
                for a in args {
                    subst_env_expr(a, env);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    subst_env_expr(x, env);
                }
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    subst_env_expr(v, env);
                }
            }
            Expr::Arrow(..) => env.clear(),
            _ => {}
        }
        fold_expr(expr);
    }

    fn run_block(
        stmts: &mut [Stmt],
        local_names: &HashSet<String>,
        env: &mut HashMap<String, SimpleLit>,
    ) {
        for stmt in stmts.iter_mut() {
            match stmt {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                    subst_env_expr(rhs, env);
                    if let Expr::Ident(id) = lhs.as_ref() {
                        if local_names.contains(id) {
                            if let Some(lit) = SimpleLit::from_expr(rhs) {
                                env.insert(id.clone(), lit);
                            } else {
                                env.remove(id);
                            }
                        } else {
                            env.remove(id);
                        }
                    } else {
                        subst_env_expr(lhs, env);
                    }
                }
                Stmt::ExprStmt(e) => {
                    subst_env_expr(e, env);
                }
                Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                    subst_env_expr(e, env);
                    env.clear();
                }
                Stmt::Let(n, init) => {
                    if let Some(e) = init {
                        subst_env_expr(e, env);
                    }
                    env.remove(n);
                }
                Stmt::Const(n, e) => {
                    subst_env_expr(e, env);
                    env.remove(n);
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(e) = init {
                            subst_env_expr(e, env);
                        }
                        env.remove(n);
                    }
                }
                Stmt::If(c, a, b) => {
                    subst_env_expr(c, env);
                    let mut a_env = env.clone();
                    run_block(a, local_names, &mut a_env);
                    if let Some(b) = b {
                        let mut b_env = env.clone();
                        run_block(b, local_names, &mut b_env);
                        env.retain(|k, v| a_env.get(k) == Some(v) && b_env.get(k) == Some(v));
                    } else if !stmts_definitely_diverge(a) {
                        env.retain(|k, v| a_env.get(k) == Some(v));
                    }
                }
                Stmt::Block(b) => run_block(b, local_names, env),
                Stmt::Labeled(_, inner) => {
                    // A labeled block/loop can exit early via `break label;` with values written
                    // prior to the break, so assignments after a conditional break inside `inner`
                    // must NEVER leak past the labeled statement.
                    let mut assigned = HashSet::new();
                    collect_assigned_idents_stmts(
                        std::slice::from_ref(inner.as_ref()),
                        &mut assigned,
                    );
                    let mut inner_env = env.clone();
                    run_block(
                        std::slice::from_mut(inner.as_mut()),
                        local_names,
                        &mut inner_env,
                    );
                    for a in assigned {
                        env.remove(&a);
                    }
                }
                Stmt::Break(_) | Stmt::Continue(_) | Stmt::Return(None) => {
                    env.clear();
                }
                Stmt::LoopForever(_, b) => {
                    env.clear();
                    let mut b_env = HashMap::new();
                    run_block(b, local_names, &mut b_env);
                }
                Stmt::While(_, b) => {
                    env.clear();
                    let mut b_env = HashMap::new();
                    run_block(b, local_names, &mut b_env);
                }
                Stmt::Switch(d, cases) => {
                    subst_env_expr(d, env);
                    for c in cases {
                        let mut c_env = env.clone();
                        run_block(&mut c.body, local_names, &mut c_env);
                    }
                    env.clear();
                }
                Stmt::TryCatch { body, handler, .. } => {
                    let mut b_env = env.clone();
                    run_block(body, local_names, &mut b_env);
                    env.clear();
                    let mut h_env = HashMap::new();
                    run_block(handler, local_names, &mut h_env);
                }
                _ => {}
            }
        }
    }

    let mut env = HashMap::new();
    run_block(body, local_names, &mut env);
}

// =============================================================================================
// 2. Spec-identical `Reflect.get(o, k)` fast-path + dominated object-guard CSE
// =============================================================================================

fn is_reflect_get_call(expr: &Expr) -> Option<(&Expr, &Expr)> {
    let Expr::Call(callee, args) = expr else {
        return None;
    };
    if args.len() != 2 {
        return None;
    }
    let Expr::Member(obj, prop) = callee.as_ref() else {
        return None;
    };
    if prop == "get" && matches!(obj.as_ref(), Expr::Ident(id) if id == "Reflect") {
        Some((&args[0], &args[1]))
    } else {
        None
    }
}

fn is_duplicable_key(expr: &Expr, obj_name: &str) -> bool {
    match expr {
        Expr::Str(_) | Expr::Num(_) | Expr::Bool(_) | Expr::Null | Expr::Undefined => true,
        Expr::Ident(k) => k != obj_name,
        _ => false,
    }
}

fn object_guard_expr(obj_name: &str) -> Expr {
    // `(o !== null && typeof o === "object") || typeof o === "function"`
    let obj_id = || Expr::Ident(obj_name.to_owned());
    let is_nonnull_obj = jsast::binary(
        BinOp::And,
        jsast::binary(BinOp::StrictNe, obj_id(), Expr::Null),
        jsast::binary(
            BinOp::StrictEq,
            Expr::Unary(UnOp::TypeOf, Box::new(obj_id())),
            Expr::Str("object".to_owned()),
        ),
    );
    let is_fn = jsast::binary(
        BinOp::StrictEq,
        Expr::Unary(UnOp::TypeOf, Box::new(obj_id())),
        Expr::Str("function".to_owned()),
    );
    jsast::binary(BinOp::Or, is_nonnull_obj, is_fn)
}

fn collect_assigned_idents_expr(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
            if let Expr::Ident(n) = lhs.as_ref() {
                out.insert(n.clone());
            }
            collect_assigned_idents_expr(lhs, out);
            collect_assigned_idents_expr(rhs, out);
        }
        Expr::Member(o, _) => collect_assigned_idents_expr(o, out),
        Expr::Index(o, i) => {
            collect_assigned_idents_expr(o, out);
            collect_assigned_idents_expr(i, out);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            collect_assigned_idents_expr(c, out);
            for a in args {
                collect_assigned_idents_expr(a, out);
            }
        }
        Expr::Unary(_, e) => collect_assigned_idents_expr(e, out),
        Expr::Binary(_, l, r) => {
            collect_assigned_idents_expr(l, out);
            collect_assigned_idents_expr(r, out);
        }
        Expr::Cond(c, t, e) => {
            collect_assigned_idents_expr(c, out);
            collect_assigned_idents_expr(t, out);
            collect_assigned_idents_expr(e, out);
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                collect_assigned_idents_expr(v, out);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for it in items {
                collect_assigned_idents_expr(it, out);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(b)) => collect_assigned_idents_expr(b, out),
        Expr::Arrow(_, ArrowBody::Block(s)) => collect_assigned_idents_stmts(s, out),
        _ => {}
    }
}

fn collect_assigned_idents_stmts(stmts: &[Stmt], out: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                collect_assigned_idents_expr(e, out);
            }
            Stmt::Let(n, init) => {
                out.insert(n.clone());
                if let Some(e) = init {
                    collect_assigned_idents_expr(e, out);
                }
            }
            Stmt::Const(n, e) => {
                out.insert(n.clone());
                collect_assigned_idents_expr(e, out);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    out.insert(n.clone());
                    if let Some(e) = init {
                        collect_assigned_idents_expr(e, out);
                    }
                }
            }
            Stmt::If(t, a, b) => {
                collect_assigned_idents_expr(t, out);
                collect_assigned_idents_stmts(a, out);
                if let Some(b) = b {
                    collect_assigned_idents_stmts(b, out);
                }
            }
            Stmt::Switch(d, cases) => {
                collect_assigned_idents_expr(d, out);
                for c in cases {
                    for t in &c.tests {
                        collect_assigned_idents_expr(t, out);
                    }
                    collect_assigned_idents_stmts(&c.body, out);
                }
            }
            Stmt::Labeled(_, inner) => {
                collect_assigned_idents_stmts(std::slice::from_ref(inner.as_ref()), out);
            }
            Stmt::LoopForever(_, b) | Stmt::Block(b) => collect_assigned_idents_stmts(b, out),
            Stmt::While(t, b) => {
                collect_assigned_idents_expr(t, out);
                collect_assigned_idents_stmts(b, out);
            }
            Stmt::TryCatch { body, handler, .. } => {
                collect_assigned_idents_stmts(body, out);
                collect_assigned_idents_stmts(handler, out);
            }
            Stmt::FunctionDecl { body, .. } => collect_assigned_idents_stmts(body, out),
            _ => {}
        }
    }
}

fn rewrite_reflect_expr(expr: &mut Expr, known: &mut HashSet<String>, unconditional: bool) {
    // Check `Reflect.get(o, k)` before descending into args (args are rewritten first so nested
    // `Reflect.get(Reflect.get(a, "b"), "c")` works cleanly).
    if let Some((Expr::Ident(obj_name), key)) = is_reflect_get_call(expr) {
        if is_duplicable_key(key, obj_name) {
            let obj_name = obj_name.clone();
            let key_expr = key.clone();
            let fast_access = jsast::index(Expr::Ident(obj_name.clone()), key_expr.clone());
            if known.contains(&obj_name) {
                *expr = fast_access;
            } else {
                let fallback = jsast::method_call(
                    jsast::id("Reflect"),
                    "get",
                    vec![Expr::Ident(obj_name.clone()), key_expr],
                );
                *expr = jsast::cond(object_guard_expr(&obj_name), fast_access, fallback);
                if unconditional {
                    known.insert(obj_name);
                }
            }
            return;
        }
    }

    match expr {
        Expr::Assign(lhs, rhs) => {
            // RHS evaluates before LHS assignment takes effect.
            rewrite_reflect_expr(rhs, known, unconditional);
            if let Expr::Ident(n) = lhs.as_ref() {
                known.remove(n);
            } else {
                rewrite_reflect_expr(lhs, known, unconditional);
            }
        }
        Expr::CompoundAssign(_, lhs, rhs) => {
            rewrite_reflect_expr(lhs, known, unconditional);
            rewrite_reflect_expr(rhs, known, unconditional);
            if let Expr::Ident(n) = lhs.as_ref() {
                known.remove(n);
            }
        }
        Expr::Member(o, _) => rewrite_reflect_expr(o, known, unconditional),
        Expr::Index(o, i) => {
            rewrite_reflect_expr(o, known, unconditional);
            rewrite_reflect_expr(i, known, unconditional);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            rewrite_reflect_expr(c, known, unconditional);
            for a in args {
                rewrite_reflect_expr(a, known, unconditional);
            }
        }
        Expr::Unary(_, e) => rewrite_reflect_expr(e, known, unconditional),
        Expr::Binary(op, l, r) => {
            rewrite_reflect_expr(l, known, unconditional);
            let rhs_uncond = unconditional && !matches!(op, BinOp::And | BinOp::Or);
            rewrite_reflect_expr(r, known, rhs_uncond);
        }
        Expr::Cond(c, t, e) => {
            rewrite_reflect_expr(c, known, unconditional);
            let mut t_known = known.clone();
            let mut e_known = known.clone();
            rewrite_reflect_expr(t, &mut t_known, false);
            rewrite_reflect_expr(e, &mut e_known, false);
            // Drop any ident assigned in either branch.
            let mut assigned = HashSet::new();
            collect_assigned_idents_expr(t, &mut assigned);
            collect_assigned_idents_expr(e, &mut assigned);
            for a in assigned {
                known.remove(&a);
            }
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                rewrite_reflect_expr(v, known, unconditional);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for it in items {
                rewrite_reflect_expr(it, known, unconditional);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(b)) => {
            let mut inner = HashSet::new();
            rewrite_reflect_expr(b, &mut inner, false);
        }
        Expr::Arrow(_, ArrowBody::Block(stmts)) => {
            let mut inner = HashSet::new();
            rewrite_reflect_stmts(stmts, &mut inner);
        }
        _ => {}
    }
}

fn rewrite_reflect_stmts(stmts: &mut [Stmt], known: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                rewrite_reflect_expr(e, known, true);
            }
            Stmt::Let(n, init) => {
                if let Some(e) = init {
                    rewrite_reflect_expr(e, known, true);
                }
                known.remove(n);
            }
            Stmt::Const(n, e) => {
                rewrite_reflect_expr(e, known, true);
                known.remove(n);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(e) = init {
                        rewrite_reflect_expr(e, known, true);
                    }
                    known.remove(n);
                }
            }
            Stmt::If(cond, then_b, else_b) => {
                rewrite_reflect_expr(cond, known, true);
                let mut t_known = known.clone();
                rewrite_reflect_stmts(then_b, &mut t_known);
                let mut assigned = HashSet::new();
                collect_assigned_idents_stmts(then_b, &mut assigned);
                if let Some(eb) = else_b {
                    let mut e_known = known.clone();
                    rewrite_reflect_stmts(eb, &mut e_known);
                    collect_assigned_idents_stmts(eb, &mut assigned);
                }
                for a in assigned {
                    known.remove(&a);
                }
            }
            Stmt::Switch(disc, cases) => {
                rewrite_reflect_expr(disc, known, true);
                let mut assigned = HashSet::new();
                for c in cases {
                    for t in &mut c.tests {
                        let mut tk = known.clone();
                        rewrite_reflect_expr(t, &mut tk, false);
                    }
                    let mut ck = known.clone();
                    rewrite_reflect_stmts(&mut c.body, &mut ck);
                    collect_assigned_idents_stmts(&c.body, &mut assigned);
                }
                for a in assigned {
                    known.remove(&a);
                }
            }
            Stmt::Labeled(_, inner) => {
                let mut assigned = HashSet::new();
                collect_assigned_idents_stmts(std::slice::from_ref(inner.as_ref()), &mut assigned);
                let mut ik = known.clone();
                rewrite_reflect_stmts(std::slice::from_mut(inner.as_mut()), &mut ik);
                for a in assigned {
                    known.remove(&a);
                }
            }
            Stmt::Block(b) => {
                rewrite_reflect_stmts(b, known);
            }
            Stmt::LoopForever(_, b) => {
                let mut assigned = HashSet::new();
                collect_assigned_idents_stmts(b, &mut assigned);
                for a in &assigned {
                    known.remove(a);
                }
                let mut lk = known.clone();
                rewrite_reflect_stmts(b, &mut lk);
            }
            Stmt::While(t, b) => {
                let mut assigned = HashSet::new();
                collect_assigned_idents_expr(t, &mut assigned);
                collect_assigned_idents_stmts(b, &mut assigned);
                for a in &assigned {
                    known.remove(a);
                }
                let mut lk = known.clone();
                rewrite_reflect_expr(t, &mut lk, true);
                rewrite_reflect_stmts(b, &mut lk);
            }
            Stmt::TryCatch { body, handler, .. } => {
                let mut assigned = HashSet::new();
                collect_assigned_idents_stmts(body, &mut assigned);
                collect_assigned_idents_stmts(handler, &mut assigned);
                let mut bk = known.clone();
                rewrite_reflect_stmts(body, &mut bk);
                let mut hk = known.clone();
                for a in &assigned {
                    hk.remove(a);
                }
                rewrite_reflect_stmts(handler, &mut hk);
                for a in assigned {
                    known.remove(&a);
                }
            }
            Stmt::FunctionDecl { body, .. } => {
                let mut inner = HashSet::new();
                rewrite_reflect_stmts(body, &mut inner);
            }
            _ => {}
        }
    }
}

fn optimize_reflect_get(body: &mut [Stmt]) {
    let mut known = HashSet::new();
    rewrite_reflect_stmts(body, &mut known);
}

// =============================================================================================
// 3. `$__mires_i64_div` rem + div fusion into `$__mires_i64_divrem`
// =============================================================================================

#[derive(Clone, Debug, PartialEq)]
struct I64DivCall {
    dst_lo: String,
    dst_hi: Option<String>,
    al: Expr,
    ah: Expr,
    bl: Expr,
    bh: Expr,
    signed: bool,
    rem: bool,
    /// Number of consecutive statements covered (1 if hi wasn't read, 2 if followed by `hi = $__mires_i64_hi`).
    span: usize,
}

fn is_pure_atom(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Ident(_) | Expr::Num(_) | Expr::Bool(_) | Expr::Unary(UnOp::Neg, _)
    )
}

fn match_i64_div_at(stmts: &[Stmt], idx: usize) -> Option<I64DivCall> {
    let Stmt::ExprStmt(Expr::Assign(lhs, rhs)) = stmts.get(idx)? else {
        return None;
    };
    let Expr::Ident(dst_lo) = lhs.as_ref() else {
        return None;
    };
    let Expr::Call(callee, args) = rhs.as_ref() else {
        return None;
    };
    if !matches!(callee.as_ref(), Expr::Ident(id) if id == i64_pair::HELPER_DIV) || args.len() != 6
    {
        return None;
    }
    let (Expr::Bool(signed), Expr::Bool(rem)) = (&args[4], &args[5]) else {
        return None;
    };
    if !is_pure_atom(&args[0])
        || !is_pure_atom(&args[1])
        || !is_pure_atom(&args[2])
        || !is_pure_atom(&args[3])
    {
        return None;
    }
    let mut dst_hi = None;
    let mut span = 1;
    if let Some(Stmt::ExprStmt(Expr::Assign(hi_lhs, hi_rhs))) = stmts.get(idx + 1) {
        if let (Expr::Ident(hi_name), Expr::Ident(src)) = (hi_lhs.as_ref(), hi_rhs.as_ref()) {
            if src == i64_pair::HELPER_HI {
                dst_hi = Some(hi_name.clone());
                span = 2;
            }
        }
    }
    Some(I64DivCall {
        dst_lo: dst_lo.clone(),
        dst_hi,
        al: args[0].clone(),
        ah: args[1].clone(),
        bl: args[2].clone(),
        bh: args[3].clone(),
        signed: *signed,
        rem: *rem,
        span,
    })
}

fn expr_clobbers_divrem(expr: &Expr, operands: &HashSet<String>) -> bool {
    match expr {
        Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
            if let Expr::Ident(n) = lhs.as_ref() {
                if operands.contains(n)
                    || n == i64_pair::HELPER_HI
                    || n == i64_pair::HELPER_RL
                    || n == i64_pair::HELPER_RH
                {
                    return true;
                }
            }
            expr_clobbers_divrem(lhs, operands) || expr_clobbers_divrem(rhs, operands)
        }
        Expr::Call(callee, args) => {
            // Only cold diverging `core$panicking$*` calls are guaranteed not to clobber scratch
            // registers and return.
            let is_panic = matches!(
                callee.as_ref(),
                Expr::Ident(name) if name.starts_with("core$panicking$")
            );
            !is_panic || args.iter().any(|a| expr_clobbers_divrem(a, operands))
        }
        Expr::New(..) => true,
        Expr::Member(o, _) => expr_clobbers_divrem(o, operands),
        Expr::Index(o, i) => expr_clobbers_divrem(o, operands) || expr_clobbers_divrem(i, operands),
        Expr::Unary(_, e) => expr_clobbers_divrem(e, operands),
        Expr::Binary(_, l, r) => {
            expr_clobbers_divrem(l, operands) || expr_clobbers_divrem(r, operands)
        }
        Expr::Cond(c, t, e) => {
            expr_clobbers_divrem(c, operands)
                || expr_clobbers_divrem(t, operands)
                || expr_clobbers_divrem(e, operands)
        }
        Expr::Object(entries) => entries
            .iter()
            .any(|(_, v)| expr_clobbers_divrem(v, operands)),
        Expr::Array(items) | Expr::Seq(items) => {
            items.iter().any(|x| expr_clobbers_divrem(x, operands))
        }
        _ => false,
    }
}

fn stmt_may_exit_non_throw(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Break(_) | Stmt::Continue(_) | Stmt::Return(_) => true,
        Stmt::If(_, a, b) => {
            a.iter().any(stmt_may_exit_non_throw)
                || b.as_ref()
                    .is_some_and(|b| b.iter().any(stmt_may_exit_non_throw))
        }
        Stmt::Switch(_, cases) => cases
            .iter()
            .any(|c| c.body.iter().any(stmt_may_exit_non_throw)),
        Stmt::Block(b) | Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
            b.iter().any(stmt_may_exit_non_throw)
        }
        Stmt::Labeled(_, inner) => stmt_may_exit_non_throw(inner),
        Stmt::TryCatch { body, handler, .. } => {
            body.iter().any(stmt_may_exit_non_throw) || handler.iter().any(stmt_may_exit_non_throw)
        }
        _ => false,
    }
}

pub(super) fn stmts_definitely_diverge(stmts: &[Stmt]) -> bool {
    for s in stmts {
        match s {
            Stmt::Throw(_) => return true,
            Stmt::ExprStmt(Expr::Call(callee, _))
                if matches!(
                    callee.as_ref(),
                    Expr::Ident(name) if name.starts_with("core$panicking$")
                ) =>
            {
                return true;
            }
            Stmt::If(_, a, Some(b))
                if stmts_definitely_diverge(a) && stmts_definitely_diverge(b) =>
            {
                return true;
            }
            _ => {
                if stmt_may_exit_non_throw(s) {
                    return false;
                }
            }
        }
    }
    false
}

fn stmt_clobbers_divrem(stmt: &Stmt, operands: &HashSet<String>) -> bool {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
            expr_clobbers_divrem(e, operands)
        }
        Stmt::Let(n, init) => {
            operands.contains(n)
                || init
                    .as_ref()
                    .is_some_and(|e| expr_clobbers_divrem(e, operands))
        }
        Stmt::Const(n, e) => operands.contains(n) || expr_clobbers_divrem(e, operands),
        Stmt::Lets(decls) => decls.iter().any(|(n, init)| {
            operands.contains(n)
                || init
                    .as_ref()
                    .is_some_and(|e| expr_clobbers_divrem(e, operands))
        }),
        Stmt::If(t, a, b) => {
            if expr_clobbers_divrem(t, operands) {
                return true;
            }
            let a_clobbers =
                !stmts_definitely_diverge(a) && a.iter().any(|s| stmt_clobbers_divrem(s, operands));
            let b_clobbers = b.as_ref().is_some_and(|b| {
                !stmts_definitely_diverge(b) && b.iter().any(|s| stmt_clobbers_divrem(s, operands))
            });
            a_clobbers || b_clobbers
        }
        Stmt::Switch(..) | Stmt::LoopForever(..) | Stmt::While(..) | Stmt::TryCatch { .. } => true,
        Stmt::Block(b) => b.iter().any(|s| stmt_clobbers_divrem(s, operands)),
        Stmt::Labeled(_, inner) => stmt_clobbers_divrem(inner, operands),
        _ => false,
    }
}

fn fuse_i64_div_rem(
    stmts: &mut Vec<Stmt>,
    local_names: &mut HashSet<String>,
    extra_decls: &mut Vec<(String, Option<Expr>)>,
) {
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::If(_, a, b) => {
                fuse_i64_div_rem(a, local_names, extra_decls);
                if let Some(b) = b {
                    fuse_i64_div_rem(b, local_names, extra_decls);
                }
            }
            Stmt::Switch(_, cases) => {
                for c in cases {
                    fuse_i64_div_rem(&mut c.body, local_names, extra_decls);
                }
            }
            Stmt::Labeled(_, inner) => {
                let mut tmp = vec![inner.as_ref().clone()];
                fuse_i64_div_rem(&mut tmp, local_names, extra_decls);
                if tmp.len() == 1 {
                    *inner.as_mut() = tmp.pop().unwrap();
                }
            }
            Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::While(_, b) => {
                fuse_i64_div_rem(b, local_names, extra_decls);
            }
            Stmt::TryCatch { body, handler, .. } => {
                fuse_i64_div_rem(body, local_names, extra_decls);
                fuse_i64_div_rem(handler, local_names, extra_decls);
            }
            _ => {}
        }
    }

    let mut i = 0;
    while i < stmts.len() {
        let Some(first) = match_i64_div_at(stmts, i) else {
            i += 1;
            continue;
        };
        let mut operands: HashSet<String> = HashSet::new();
        for e in [&first.al, &first.ah, &first.bl, &first.bh] {
            if let Expr::Ident(n) = e {
                operands.insert(n.clone());
            }
        }
        // Make sure `first`'s destination writes didn't overwrite its own operands before call 2!
        if operands.contains(&first.dst_lo)
            || first.dst_hi.as_ref().is_some_and(|h| operands.contains(h))
        {
            i += first.span;
            continue;
        }
        let mut found: Option<(usize, I64DivCall)> = None;
        let mut j = i + first.span;
        while j < stmts.len() {
            if let Some(second) = match_i64_div_at(stmts, j) {
                if second.al == first.al
                    && second.ah == first.ah
                    && second.bl == first.bl
                    && second.bh == first.bh
                    && second.signed == first.signed
                    && second.rem == !first.rem
                {
                    found = Some((j, second));
                }
                break;
            }
            if stmt_clobbers_divrem(&stmts[j], &operands) {
                break;
            }
            j += 1;
        }
        let Some((j, second)) = found else {
            i += first.span;
            continue;
        };

        let tmp_lo = alloc_fresh_local(local_names, extra_decls, "$dr_lo");
        let tmp_hi = second
            .dst_hi
            .as_ref()
            .map(|_| alloc_fresh_local(local_names, extra_decls, "$dr_hi"));

        let divrem_call = jsast::call(
            jsast::id(i64_pair::HELPER_DIVREM),
            vec![
                first.al.clone(),
                first.ah.clone(),
                first.bl.clone(),
                first.bh.clone(),
                Expr::Bool(first.signed),
            ],
        );

        let mut first_repl = Vec::new();
        let mut second_repl = Vec::new();
        if first.rem {
            // First is rem, second is div: divrem returns ql, writes qh to HELPER_HI, rl/rh to HELPER_RL/RH.
            first_repl.push(jsast::assign_stmt(jsast::id(tmp_lo.clone()), divrem_call));
            if let Some(ref th) = tmp_hi {
                first_repl.push(jsast::assign_stmt(
                    jsast::id(th.clone()),
                    jsast::id(i64_pair::HELPER_HI),
                ));
            }
            first_repl.push(jsast::assign_stmt(
                jsast::id(first.dst_lo.clone()),
                jsast::id(i64_pair::HELPER_RL),
            ));
            if let Some(ref dh) = first.dst_hi {
                first_repl.push(jsast::assign_stmt(
                    jsast::id(dh.clone()),
                    jsast::id(i64_pair::HELPER_RH),
                ));
            }
        } else {
            // First is div, second is rem.
            first_repl.push(jsast::assign_stmt(
                jsast::id(first.dst_lo.clone()),
                divrem_call,
            ));
            if let Some(ref dh) = first.dst_hi {
                first_repl.push(jsast::assign_stmt(
                    jsast::id(dh.clone()),
                    jsast::id(i64_pair::HELPER_HI),
                ));
            }
            first_repl.push(jsast::assign_stmt(
                jsast::id(tmp_lo.clone()),
                jsast::id(i64_pair::HELPER_RL),
            ));
            if let Some(ref th) = tmp_hi {
                first_repl.push(jsast::assign_stmt(
                    jsast::id(th.clone()),
                    jsast::id(i64_pair::HELPER_RH),
                ));
            }
        }
        second_repl.push(jsast::assign_stmt(
            jsast::id(second.dst_lo.clone()),
            jsast::id(tmp_lo),
        ));
        if let (Some(dh), Some(th)) = (second.dst_hi.as_ref(), tmp_hi) {
            second_repl.push(jsast::assign_stmt(jsast::id(dh.clone()), jsast::id(th)));
        }

        // Splice second first so index `i` stays valid.
        stmts.splice(j..j + second.span, second_repl);
        let first_new_len = first_repl.len();
        stmts.splice(i..i + first.span, first_repl);
        i += first_new_len;
    }
}

/// Fast-paths dead-`lo` `$__mires_i64_f64_to_bits(x)` sign-bit tests
/// (`hi = $__mires_i64_hi; (hi & -2147483648) !== 0`), which Rust emits inside
/// `f64::is_sign_negative` / `f64::min` / `f64::max`.
///
/// - Constant literals fold directly using `f64::is_sign_negative`.
/// - Variables dominated by `x === C` (`C != 0.0`, non-NaN) fold directly to `C < 0.0`.
/// - Variables dominated by `x === y` (guaranteed non-NaN by IEEE-754) lower to the branchless
///   inline check `x < 0 || x === 0 && 1 / x < 0` (exact for `-0.0`, `+0.0`, `±Infinity`, finite).
/// - General variables guard NaN via `x === x ? (x < 0 || x === 0 && 1 / x < 0) : ...` so
///   `-NaN` sign bits remain bit-exact while non-NaN floats never call `f64_to_bits`.
fn fast_path_f64_sign_bit(body: &mut Vec<Stmt>) -> bool {
    let mut read_counts: HashMap<String, usize> = HashMap::new();
    for stmt in body.iter() {
        tally_ident_reads_stmt(stmt, &mut read_counts);
    }
    let mut non_nan: HashSet<String> = HashSet::new();
    let mut known_nonzero: HashMap<String, f64> = HashMap::new();
    fast_path_f64_sign_bit_stmts(body, &read_counts, &mut non_nan, &mut known_nonzero)
}

fn exact_f64_lit(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Num(n) => Some(*n),
        Expr::Unary(UnOp::Neg, inner) => match inner.as_ref() {
            Expr::Num(n) => Some(-*n),
            _ => None,
        },
        _ => None,
    }
}

fn tally_ident_reads_expr(expr: &Expr, counts: &mut HashMap<String, usize>) {
    match expr {
        Expr::Ident(id) => *counts.entry(id.clone()).or_insert(0) += 1,
        Expr::Member(obj, _) => tally_ident_reads_expr(obj, counts),
        Expr::Index(obj, idx) => {
            tally_ident_reads_expr(obj, counts);
            tally_ident_reads_expr(idx, counts);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            tally_ident_reads_expr(c, counts);
            for a in args {
                tally_ident_reads_expr(a, counts);
            }
        }
        Expr::Unary(_, inner) => tally_ident_reads_expr(inner, counts),
        Expr::Binary(_, l, r) => {
            tally_ident_reads_expr(l, counts);
            tally_ident_reads_expr(r, counts);
        }
        Expr::Assign(lhs, rhs) => {
            if !matches!(lhs.as_ref(), Expr::Ident(_)) {
                tally_ident_reads_expr(lhs, counts);
            }
            tally_ident_reads_expr(rhs, counts);
        }
        Expr::CompoundAssign(_, lhs, rhs) => {
            tally_ident_reads_expr(lhs, counts);
            tally_ident_reads_expr(rhs, counts);
        }
        Expr::Cond(c, t, e) => {
            tally_ident_reads_expr(c, counts);
            tally_ident_reads_expr(t, counts);
            tally_ident_reads_expr(e, counts);
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                tally_ident_reads_expr(v, counts);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                tally_ident_reads_expr(item, counts);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(b)) => tally_ident_reads_expr(b, counts),
        Expr::Arrow(_, ArrowBody::Block(stmts)) => {
            for s in stmts {
                tally_ident_reads_stmt(s, counts);
            }
        }
        _ => {}
    }
}

fn tally_ident_reads_stmt(stmt: &Stmt, counts: &mut HashMap<String, usize>) {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Return(Some(e)) | Stmt::Throw(e) => {
            tally_ident_reads_expr(e, counts);
        }
        Stmt::Let(_, Some(e)) | Stmt::Const(_, e) => tally_ident_reads_expr(e, counts),
        Stmt::Lets(decls) => {
            for (_, init) in decls {
                if let Some(e) = init {
                    tally_ident_reads_expr(e, counts);
                }
            }
        }
        Stmt::If(c, t, e) => {
            tally_ident_reads_expr(c, counts);
            for s in t {
                tally_ident_reads_stmt(s, counts);
            }
            if let Some(el) = e {
                for s in el {
                    tally_ident_reads_stmt(s, counts);
                }
            }
        }
        Stmt::While(c, b) => {
            tally_ident_reads_expr(c, counts);
            for s in b {
                tally_ident_reads_stmt(s, counts);
            }
        }
        Stmt::LoopForever(_, b) | Stmt::Block(b) => {
            for s in b {
                tally_ident_reads_stmt(s, counts);
            }
        }
        Stmt::Labeled(_, inner) => tally_ident_reads_stmt(inner, counts),
        Stmt::Switch(scrut, cases) => {
            tally_ident_reads_expr(scrut, counts);
            for c in cases {
                for t in &c.tests {
                    tally_ident_reads_expr(t, counts);
                }
                for s in &c.body {
                    tally_ident_reads_stmt(s, counts);
                }
            }
        }
        Stmt::TryCatch { body, handler, .. } => {
            for s in body {
                tally_ident_reads_stmt(s, counts);
            }
            for s in handler {
                tally_ident_reads_stmt(s, counts);
            }
        }
        _ => {}
    }
}

fn collect_written_idents_expr(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
            if let Expr::Ident(id) = lhs.as_ref() {
                out.insert(id.clone());
            } else {
                collect_written_idents_expr(lhs, out);
            }
            collect_written_idents_expr(rhs, out);
        }
        Expr::Member(obj, _) => collect_written_idents_expr(obj, out),
        Expr::Index(obj, idx) => {
            collect_written_idents_expr(obj, out);
            collect_written_idents_expr(idx, out);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            collect_written_idents_expr(c, out);
            for a in args {
                collect_written_idents_expr(a, out);
            }
        }
        Expr::Unary(_, inner) => collect_written_idents_expr(inner, out),
        Expr::Binary(_, l, r) => {
            collect_written_idents_expr(l, out);
            collect_written_idents_expr(r, out);
        }
        Expr::Cond(c, t, e) => {
            collect_written_idents_expr(c, out);
            collect_written_idents_expr(t, out);
            collect_written_idents_expr(e, out);
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                collect_written_idents_expr(v, out);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                collect_written_idents_expr(item, out);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(b)) => collect_written_idents_expr(b, out),
        Expr::Arrow(_, ArrowBody::Block(stmts)) => {
            for s in stmts {
                collect_written_idents_stmt(s, out);
            }
        }
        _ => {}
    }
}

fn collect_written_idents_stmt(stmt: &Stmt, out: &mut HashSet<String>) {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Return(Some(e)) | Stmt::Throw(e) => {
            collect_written_idents_expr(e, out);
        }
        Stmt::Let(n, init) => {
            out.insert(n.clone());
            if let Some(e) = init {
                collect_written_idents_expr(e, out);
            }
        }
        Stmt::Const(n, e) => {
            out.insert(n.clone());
            collect_written_idents_expr(e, out);
        }
        Stmt::Lets(decls) => {
            for (n, init) in decls {
                out.insert(n.clone());
                if let Some(e) = init {
                    collect_written_idents_expr(e, out);
                }
            }
        }
        Stmt::If(c, t, e) => {
            collect_written_idents_expr(c, out);
            for s in t {
                collect_written_idents_stmt(s, out);
            }
            if let Some(el) = e {
                for s in el {
                    collect_written_idents_stmt(s, out);
                }
            }
        }
        Stmt::While(c, b) => {
            collect_written_idents_expr(c, out);
            for s in b {
                collect_written_idents_stmt(s, out);
            }
        }
        Stmt::LoopForever(_, b) | Stmt::Block(b) => {
            for s in b {
                collect_written_idents_stmt(s, out);
            }
        }
        Stmt::Labeled(_, inner) => collect_written_idents_stmt(inner, out),
        Stmt::Switch(scrut, cases) => {
            collect_written_idents_expr(scrut, out);
            for c in cases {
                for t in &c.tests {
                    collect_written_idents_expr(t, out);
                }
                for s in &c.body {
                    collect_written_idents_stmt(s, out);
                }
            }
        }
        Stmt::TryCatch {
            body,
            binding,
            handler,
        } => {
            for s in body {
                collect_written_idents_stmt(s, out);
            }
            out.insert(binding.clone());
            for s in handler {
                collect_written_idents_stmt(s, out);
            }
        }
        _ => {}
    }
}

fn match_f64_to_bits_call<'a>(
    stmt: &'a Stmt,
    read_counts: &HashMap<String, usize>,
) -> Option<&'a Expr> {
    let Stmt::ExprStmt(expr) = stmt else {
        return None;
    };
    let call = match expr {
        Expr::Call(..) => expr,
        Expr::Assign(lhs, rhs) => {
            let Expr::Ident(lo) = lhs.as_ref() else {
                return None;
            };
            if read_counts.get(lo).copied().unwrap_or(0) != 0 {
                return None;
            }
            rhs.as_ref()
        }
        _ => return None,
    };
    let Expr::Call(callee, args) = call else {
        return None;
    };
    if !matches!(callee.as_ref(), Expr::Ident(id) if id == "$__mires_i64_f64_to_bits")
        || args.len() != 1
    {
        return None;
    }
    let arg = &args[0];
    if matches!(arg, Expr::Ident(_)) || exact_f64_lit(arg).is_some() {
        Some(arg)
    } else {
        None
    }
}

fn match_hi_assign(stmt: &Stmt, read_counts: &HashMap<String, usize>) -> Option<String> {
    let Stmt::ExprStmt(Expr::Assign(lhs, rhs)) = stmt else {
        return None;
    };
    let Expr::Ident(hi_var) = lhs.as_ref() else {
        return None;
    };
    if !matches!(rhs.as_ref(), Expr::Ident(id) if id == "$__mires_i64_hi") {
        return None;
    }
    if read_counts.get(hi_var).copied().unwrap_or(0) != 1 {
        return None;
    }
    Some(hi_var.clone())
}

/// Matches `(hi_var & -2147483648) !== 0` (returns `Some(true)`) or `=== 0` (returns `Some(false)`),
/// tolerating an optional outer `0 | ...` or `... | 0`.
fn match_sign_bit_test(expr: &Expr, hi_var: &str) -> Option<bool> {
    let Expr::Binary(op @ (BinOp::StrictNe | BinOp::StrictEq), l, r) = expr else {
        return None;
    };
    let test_side = if exact_i32(r) == Some(0) {
        l.as_ref()
    } else if exact_i32(l) == Some(0) {
        r.as_ref()
    } else {
        return None;
    };
    let unwrapped = match test_side {
        Expr::Binary(BinOp::BitOr, a, b) if exact_i32(a) == Some(0) => b.as_ref(),
        Expr::Binary(BinOp::BitOr, a, b) if exact_i32(b) == Some(0) => a.as_ref(),
        other => other,
    };
    let Expr::Binary(BinOp::BitAnd, a, b) = unwrapped else {
        return None;
    };
    let is_hi_and_sign = (matches!(a.as_ref(), Expr::Ident(id) if id == hi_var)
        && to_i32_bits(b) == Some(i32::MIN))
        || (matches!(b.as_ref(), Expr::Ident(id) if id == hi_var)
            && to_i32_bits(a) == Some(i32::MIN));
    if is_hi_and_sign {
        Some(*op == BinOp::StrictNe)
    } else {
        None
    }
}

fn build_sign_bit_expr(
    arg: &Expr,
    expect_neg: bool,
    non_nan: &HashSet<String>,
    known_nonzero: &HashMap<String, f64>,
) -> Expr {
    if let Some(c) = exact_f64_lit(arg) {
        return Expr::Bool(c.is_sign_negative() == expect_neg);
    }
    if let Expr::Ident(v) = arg {
        if let Some(&c) = known_nonzero.get(v) {
            return Expr::Bool((c < 0.0) == expect_neg);
        }
    }
    let non_nan_check = Expr::Binary(
        BinOp::Or,
        Box::new(Expr::Binary(
            BinOp::Lt,
            Box::new(arg.clone()),
            Box::new(Expr::Num(0.0)),
        )),
        Box::new(Expr::Binary(
            BinOp::And,
            Box::new(Expr::Binary(
                BinOp::StrictEq,
                Box::new(arg.clone()),
                Box::new(Expr::Num(0.0)),
            )),
            Box::new(Expr::Binary(
                BinOp::Lt,
                Box::new(Expr::Binary(
                    BinOp::Div,
                    Box::new(Expr::Num(1.0)),
                    Box::new(arg.clone()),
                )),
                Box::new(Expr::Num(0.0)),
            )),
        )),
    );
    let is_known_non_nan = matches!(arg, Expr::Ident(v) if non_nan.contains(v));
    let neg_expr = if is_known_non_nan {
        non_nan_check
    } else {
        let fallback = Expr::Seq(vec![
            jsast::call(jsast::id("$__mires_i64_f64_to_bits"), vec![arg.clone()]),
            Expr::Binary(
                BinOp::StrictNe,
                Box::new(Expr::Binary(
                    BinOp::BitAnd,
                    Box::new(jsast::id("$__mires_i64_hi")),
                    Box::new(num_expr_i32(i32::MIN)),
                )),
                Box::new(Expr::Num(0.0)),
            ),
        ]);
        Expr::Cond(
            Box::new(Expr::Binary(
                BinOp::StrictEq,
                Box::new(arg.clone()),
                Box::new(arg.clone()),
            )),
            Box::new(non_nan_check),
            Box::new(fallback),
        )
    };
    if expect_neg {
        neg_expr
    } else {
        Expr::Unary(UnOp::Not, Box::new(neg_expr))
    }
}

fn fast_path_f64_sign_bit_stmts(
    stmts: &mut Vec<Stmt>,
    read_counts: &HashMap<String, usize>,
    non_nan: &mut HashSet<String>,
    known_nonzero: &mut HashMap<String, f64>,
) -> bool {
    let mut changed = false;
    let mut i = 0usize;
    while i < stmts.len() {
        if i + 2 < stmts.len() {
            if let Some(arg) = match_f64_to_bits_call(&stmts[i], read_counts).cloned() {
                if let Some(hi_var) = match_hi_assign(&stmts[i + 1], read_counts) {
                    let matched = match &mut stmts[i + 2] {
                        Stmt::ExprStmt(Expr::Assign(_, rhs)) => {
                            if let Some(expect_neg) = match_sign_bit_test(rhs, &hi_var) {
                                **rhs =
                                    build_sign_bit_expr(&arg, expect_neg, non_nan, known_nonzero);
                                true
                            } else {
                                false
                            }
                        }
                        Stmt::If(cond, _, _) => {
                            if let Some(expect_neg) = match_sign_bit_test(cond, &hi_var) {
                                *cond =
                                    build_sign_bit_expr(&arg, expect_neg, non_nan, known_nonzero);
                                true
                            } else {
                                false
                            }
                        }
                        _ => false,
                    };
                    if matched {
                        stmts.drain(i..i + 2);
                        changed = true;
                        continue;
                    }
                }
            }
        }

        // Recurse into nested control flow while maintaining exact dominating facts.
        match &mut stmts[i] {
            Stmt::If(cond, then_b, else_b) => {
                let mut then_nn = non_nan.clone();
                let mut then_nz = known_nonzero.clone();
                if let Expr::Binary(BinOp::StrictEq, l, r) = cond {
                    if let Expr::Ident(a) = l.as_ref() {
                        then_nn.insert(a.clone());
                        if let Some(c) = exact_f64_lit(r) {
                            if c != 0.0 && !c.is_nan() {
                                then_nz.insert(a.clone(), c);
                            }
                        }
                    }
                    if let Expr::Ident(b) = r.as_ref() {
                        then_nn.insert(b.clone());
                        if let Some(c) = exact_f64_lit(l) {
                            if c != 0.0 && !c.is_nan() {
                                then_nz.insert(b.clone(), c);
                            }
                        }
                    }
                }
                changed |=
                    fast_path_f64_sign_bit_stmts(then_b, read_counts, &mut then_nn, &mut then_nz);
                if let Some(el) = else_b {
                    let mut else_nn = non_nan.clone();
                    let mut else_nz = known_nonzero.clone();
                    changed |=
                        fast_path_f64_sign_bit_stmts(el, read_counts, &mut else_nn, &mut else_nz);
                }
            }
            Stmt::Block(b) => {
                let mut inner_nn = non_nan.clone();
                let mut inner_nz = known_nonzero.clone();
                changed |=
                    fast_path_f64_sign_bit_stmts(b, read_counts, &mut inner_nn, &mut inner_nz);
            }
            Stmt::Labeled(_, inner) => {
                let mut written = HashSet::new();
                collect_written_idents_stmt(inner, &mut written);
                let mut inner_nn = non_nan.clone();
                let mut inner_nz = known_nonzero.clone();
                for w in &written {
                    inner_nn.remove(w);
                    inner_nz.remove(w);
                }
                if let Stmt::Block(b) | Stmt::LoopForever(_, b) | Stmt::While(_, b) = inner.as_mut()
                {
                    changed |=
                        fast_path_f64_sign_bit_stmts(b, read_counts, &mut inner_nn, &mut inner_nz);
                }
            }
            Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
                let mut written = HashSet::new();
                for s in b.iter() {
                    collect_written_idents_stmt(s, &mut written);
                }
                let mut inner_nn = non_nan.clone();
                let mut inner_nz = known_nonzero.clone();
                for w in &written {
                    inner_nn.remove(w);
                    inner_nz.remove(w);
                }
                changed |=
                    fast_path_f64_sign_bit_stmts(b, read_counts, &mut inner_nn, &mut inner_nz);
            }
            Stmt::Switch(_, cases) => {
                for c in cases {
                    let mut inner_nn = non_nan.clone();
                    let mut inner_nz = known_nonzero.clone();
                    changed |= fast_path_f64_sign_bit_stmts(
                        &mut c.body,
                        read_counts,
                        &mut inner_nn,
                        &mut inner_nz,
                    );
                }
            }
            Stmt::TryCatch { body, handler, .. } => {
                let mut b_nn = non_nan.clone();
                let mut b_nz = known_nonzero.clone();
                changed |= fast_path_f64_sign_bit_stmts(body, read_counts, &mut b_nn, &mut b_nz);
                let mut h_nn = HashSet::new();
                let mut h_nz = HashMap::new();
                changed |= fast_path_f64_sign_bit_stmts(handler, read_counts, &mut h_nn, &mut h_nz);
            }
            _ => {}
        }

        // Invalidate facts for any variable written by `stmts[i]`.
        let mut written = HashSet::new();
        collect_written_idents_stmt(&stmts[i], &mut written);
        for w in written {
            non_nan.remove(&w);
            known_nonzero.remove(&w);
        }
        i += 1;
    }
    changed
}

/// Folds IEEE-754 `Math.max` / `Math.min` labeled-block idioms (both the general 2-variable form
/// and the 1-variable + constant form emitted by `js_max` / `js_min` after `fast_path_f64_sign_bit`)
/// into `dst = Math.max(a, b)` / `dst = Math.min(a, b)`, preserving exact `NaN` and `-0.0`/`+0.0`
/// semantics.
fn fold_ieee_max_min(body: &mut Vec<Stmt>) {
    let mut read_counts: HashMap<String, usize> = HashMap::new();
    for stmt in body.iter() {
        tally_ident_reads_stmt(stmt, &mut read_counts);
    }
    fold_ieee_max_min_stmts(body, &read_counts);
}

fn is_nan_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Num(n) => n.is_nan(),
        Expr::Ident(id) | Expr::RawNum(id) => id == "NaN",
        _ => false,
    }
}

/// Matches `x !== x` (`Some((x, true))`) or `x === x` (`Some((x, false))`), with optional `!`.
fn match_nan_test_expr(expr: &Expr) -> Option<(String, bool)> {
    match expr {
        Expr::Unary(UnOp::Not, inner) => {
            let (v, is_nan) = match_nan_test_expr(inner)?;
            Some((v, !is_nan))
        }
        Expr::Binary(op @ (BinOp::StrictNe | BinOp::StrictEq), l, r) => {
            if let (Expr::Ident(a), Expr::Ident(b)) = (l.as_ref(), r.as_ref()) {
                if a == b {
                    return Some((a.clone(), *op == BinOp::StrictNe));
                }
            }
            None
        }
        _ => None,
    }
}

/// Matches `x < 0 || x === 0 && 1 / x < 0` (the inline non-NaN `is_sign_negative` emitted by
/// `fast_path_f64_sign_bit`), returning `(var_name, true_means_negative)`.
fn match_is_sign_negative_expr(expr: &Expr) -> Option<(String, bool)> {
    if let Expr::Unary(UnOp::Not, inner) = expr {
        let (v, b) = match_is_sign_negative_expr(inner)?;
        return Some((v, !b));
    }
    let Expr::Binary(BinOp::Or, l, r) = expr else {
        return None;
    };
    let is_zero = |e: &Expr| exact_f64_lit(e).is_some_and(|v| v == 0.0);
    let is_one = |e: &Expr| exact_f64_lit(e) == Some(1.0);
    let var = match l.as_ref() {
        Expr::Binary(BinOp::Lt, a, b) if is_zero(b) => match a.as_ref() {
            Expr::Ident(id) => id.clone(),
            _ => return None,
        },
        Expr::Binary(BinOp::Gt, a, b) if is_zero(a) => match b.as_ref() {
            Expr::Ident(id) => id.clone(),
            _ => return None,
        },
        _ => return None,
    };
    let Expr::Binary(BinOp::And, eq_part, inv_part) = r.as_ref() else {
        return None;
    };
    let eq_ok = matches!(
        eq_part.as_ref(),
        Expr::Binary(BinOp::StrictEq, a, b)
            if (matches!(a.as_ref(), Expr::Ident(id) if id == &var) && is_zero(b))
                || (is_zero(a) && matches!(b.as_ref(), Expr::Ident(id) if id == &var))
    );
    if !eq_ok {
        return None;
    }
    let is_one_div_var = |e: &Expr| {
        matches!(
            e,
            Expr::Binary(BinOp::Div, num, den)
                if is_one(num) && matches!(den.as_ref(), Expr::Ident(id) if id == &var)
        )
    };
    let inv_ok = matches!(
        inv_part.as_ref(),
        Expr::Binary(BinOp::Lt, a, b) if is_one_div_var(a) && is_zero(b)
    ) || matches!(
        inv_part.as_ref(),
        Expr::Binary(BinOp::Gt, a, b) if is_zero(a) && is_one_div_var(b)
    );
    inv_ok.then_some((var, true))
}

fn match_assign_and_break<'a>(stmts: &'a [Stmt], dst: &str, label: &str) -> Option<&'a Expr> {
    let [Stmt::ExprStmt(Expr::Assign(lhs, rhs)), Stmt::Break(Some(brk))] = stmts else {
        return None;
    };
    if matches!(lhs.as_ref(), Expr::Ident(id) if id == dst) && brk == label {
        Some(rhs.as_ref())
    } else {
        None
    }
}

/// Checks if `cond` tests `!is_nan(var)` given an optional preceding `tmp = var !== var` / `var === var`.
fn match_non_nan_guard_cond(
    cond: &Expr,
    prev_nan_tmp: Option<(&str, &str, bool)>,
) -> Option<String> {
    if let Some((tmp, var, tmp_is_nan)) = prev_nan_tmp {
        if tmp_is_nan {
            if matches!(cond, Expr::Unary(UnOp::Not, inner) if matches!(inner.as_ref(), Expr::Ident(id) if id == tmp))
            {
                return Some(var.to_owned());
            }
        } else if matches!(cond, Expr::Ident(id) if id == tmp) {
            return Some(var.to_owned());
        }
    }
    if let Some((var, is_nan)) = match_nan_test_expr(cond) {
        if !is_nan {
            return Some(var);
        }
    }
    None
}

/// Given `ord_cond` and branch results `(then_val, else_val)` for unequal non-NaN operands `(a, b)`,
/// returns `Some(true)` for `Math.max`, `Some(false)` for `Math.min`, or `None`.
fn classify_strict_order_branch(
    ord_cond: &Expr,
    then_val: &Expr,
    else_val: &Expr,
    a: &Expr,
    b: &Expr,
) -> Option<bool> {
    let Expr::Binary(op, l, r) = ord_cond else {
        return None;
    };
    // Determine which of (a, b) is strictly larger when `ord_cond` is true (knowing a !== b).
    let then_picks_larger = match (
        op,
        l.as_ref() == a && r.as_ref() == b,
        l.as_ref() == b && r.as_ref() == a,
    ) {
        (BinOp::Gt | BinOp::Ge, true, _) => then_val == a && else_val == b,
        (BinOp::Lt | BinOp::Le, _, true) => then_val == a && else_val == b,
        (BinOp::Lt | BinOp::Le, true, _) => then_val == b && else_val == a,
        (BinOp::Gt | BinOp::Ge, _, true) => then_val == b && else_val == a,
        _ => return None,
    };
    if then_picks_larger {
        return Some(true);
    }
    let then_picks_smaller = match (
        op,
        l.as_ref() == a && r.as_ref() == b,
        l.as_ref() == b && r.as_ref() == a,
    ) {
        (BinOp::Lt | BinOp::Le, true, _) => then_val == a && else_val == b,
        (BinOp::Gt | BinOp::Ge, _, true) => then_val == a && else_val == b,
        (BinOp::Gt | BinOp::Ge, true, _) => then_val == b && else_val == a,
        (BinOp::Lt | BinOp::Le, _, true) => then_val == b && else_val == a,
        _ => return None,
    };
    then_picks_smaller.then_some(false)
}

/// Verifies `eq_body` (entered when `a === b` and both are non-NaN) matches `Math.max(a, b)`
/// (`is_max == true`) or `Math.min(a, b)` (`is_max == false`) bit-identically on `-0.0` / `+0.0`.
#[allow(clippy::too_many_arguments)]
fn verify_eq_branch(
    eq_body: &[Stmt],
    dst: &str,
    label: &str,
    a: &Expr,
    b: &Expr,
    const_b: Option<f64>,
    is_max: bool,
    read_counts: &HashMap<String, usize>,
) -> bool {
    // Case 1: Sign-bit check inside `a === b`.
    let sign_branch: Option<(String, bool, &Expr, &Expr)> = match eq_body {
        [Stmt::ExprStmt(Expr::Assign(dd_lhs, dd_rhs)), Stmt::If(cond, t, Some(e))] => {
            let Expr::Ident(dd_name) = dd_lhs.as_ref() else {
                return false;
            };
            if read_counts.get(dd_name).copied().unwrap_or(0) != 1
                || dd_name == dst
                || matches!(a, Expr::Ident(id) if id == dd_name)
                || matches!(b, Expr::Ident(id) if id == dd_name)
            {
                return false;
            }
            let Some((sign_var, rhs_true_neg)) = match_is_sign_negative_expr(dd_rhs) else {
                return false;
            };
            let cond_true_neg = if matches!(cond, Expr::Ident(id) if id == dd_name) {
                rhs_true_neg
            } else if matches!(cond, Expr::Unary(UnOp::Not, inner) if matches!(inner.as_ref(), Expr::Ident(id) if id == dd_name))
            {
                !rhs_true_neg
            } else {
                return false;
            };
            let (Some(w1), Some(w2)) = (
                match_assign_and_break(t, dst, label),
                match_assign_and_break(e, dst, label),
            ) else {
                return false;
            };
            Some((sign_var, cond_true_neg, w1, w2))
        }
        [Stmt::If(cond, t, Some(e))] => {
            let Some((sign_var, cond_true_neg)) = match_is_sign_negative_expr(cond) else {
                return false;
            };
            let (Some(w1), Some(w2)) = (
                match_assign_and_break(t, dst, label),
                match_assign_and_break(e, dst, label),
            ) else {
                return false;
            };
            Some((sign_var, cond_true_neg, w1, w2))
        }
        _ => None,
    };

    if let Some((sign_var, cond_true_neg, w1, w2)) = sign_branch {
        let z = Expr::Ident(sign_var);
        let other = if &z == a {
            b
        } else if &z == b {
            a
        } else {
            return false;
        };
        let (on_neg, on_nonneg) = if cond_true_neg { (w1, w2) } else { (w2, w1) };
        return if is_max {
            on_neg == other && on_nonneg == &z
        } else {
            on_neg == &z && on_nonneg == other
        };
    }

    // Case 2: Direct constant-folded assignment `[dst = w; break label]`, valid ONLY when `b` is
    // a finite non-NaN literal (`const_b = Some(c)`).
    if let (Some(c), Some(w)) = (const_b, match_assign_and_break(eq_body, dst, label)) {
        if c != 0.0 {
            return w == a || w == b;
        }
        // `c == 0.0`: verify exact `-0.0` vs `+0.0` tie-breaking.
        if c.is_sign_positive() {
            // `+0.0`: Math.max(a, +0) -> +0 (`b`), Math.min(a, +0) -> a (preserves -0).
            return if is_max { w == b } else { w == a };
        } else {
            // `-0.0`: Math.max(a, -0) -> a (preserves +0), Math.min(a, -0) -> -0 (`b`).
            return if is_max { w == a } else { w == b };
        }
    }

    false
}

/// Matches the inner compare-and-break body after both operands are proven non-NaN.
/// Returns `Some((b_expr, is_max))` where `a` is the already-checked variable `a_var`.
fn match_max_min_compare_body(
    inner: &[Stmt],
    a_var: &str,
    known_b_var: Option<&str>,
    dst: &str,
    label: &str,
    read_counts: &HashMap<String, usize>,
) -> Option<(Expr, bool)> {
    let [Stmt::If(cmp_cond, cmp_then, Some(cmp_else))] = inner else {
        return None;
    };
    let a = Expr::Ident(a_var.to_owned());

    // Subcase A: `if (a === b) { ... } else if (a > b) { ... } else { ... }`
    if let Expr::Binary(BinOp::StrictEq, eq_l, eq_r) = cmp_cond {
        let b_expr = if eq_l.as_ref() == &a {
            eq_r.as_ref()
        } else if eq_r.as_ref() == &a {
            eq_l.as_ref()
        } else {
            return None;
        };
        let const_b = match known_b_var {
            Some(bv) => {
                if !matches!(b_expr, Expr::Ident(id) if id == bv) {
                    return None;
                }
                None
            }
            None => {
                let c = exact_f64_lit(b_expr)?;
                if !c.is_finite() {
                    return None;
                }
                Some(c)
            }
        };
        let [Stmt::If(ord_cond, ord_then, Some(ord_else))] = cmp_else.as_slice() else {
            return None;
        };
        let v1 = match_assign_and_break(ord_then, dst, label)?;
        let v2 = match_assign_and_break(ord_else, dst, label)?;
        let is_max = classify_strict_order_branch(ord_cond, v1, v2, &a, b_expr)?;
        if verify_eq_branch(
            cmp_then,
            dst,
            label,
            &a,
            b_expr,
            const_b,
            is_max,
            read_counts,
        ) {
            return Some((b_expr.clone(), is_max));
        }
        return None;
    }

    // Subcase B (1-var + constant only): direct `if (a > c) { dst = a; break } else { dst = c; break }`
    // with no separate `===` branch.
    if known_b_var.is_none() {
        let Expr::Binary(op, ol, or) = cmp_cond else {
            return None;
        };
        let (b_expr, a_on_left) = if ol.as_ref() == &a {
            (or.as_ref(), true)
        } else if or.as_ref() == &a {
            (ol.as_ref(), false)
        } else {
            return None;
        };
        let c = exact_f64_lit(b_expr)?;
        if !c.is_finite() {
            return None;
        }
        let v1 = match_assign_and_break(cmp_then, dst, label)?;
        let v2 = match_assign_and_break(cmp_else, dst, label)?;
        let is_max = classify_strict_order_branch(cmp_cond, v1, v2, &a, b_expr)?;
        if c != 0.0 {
            return Some((b_expr.clone(), is_max));
        }
        // Check exact tie-breaking at `a === 0` (`+0.0` vs `-0.0`):
        let eq_takes_then = matches!(op, BinOp::Ge | BinOp::Le);
        let eq_result = if eq_takes_then { v1 } else { v2 };
        let _ = a_on_left;
        let ok = if c.is_sign_positive() {
            if is_max {
                eq_result == b_expr
            } else {
                eq_result == &a
            }
        } else if is_max {
            eq_result == &a
        } else {
            eq_result == b_expr
        };
        if ok {
            return Some((b_expr.clone(), is_max));
        }
    }

    None
}

/// Attempts to match an IEEE-754 `Math.max` / `Math.min` idiom starting at `stmts[idx]`.
/// Returns `(start_idx, end_excl, replacement_stmt)` on match.
fn match_ieee_max_min_at(
    stmts: &[Stmt],
    idx: usize,
    read_counts: &HashMap<String, usize>,
) -> Option<(usize, usize, Stmt)> {
    // Check optional preceding `c_ = k !== k;` (or `c_ = k === k;`).
    let (label_idx, prev_nan_tmp, consume_prev) =
        if let Some(Stmt::ExprStmt(Expr::Assign(lhs, rhs))) = stmts.get(idx) {
            if let (Expr::Ident(tmp), Some((var, is_nan))) =
                (lhs.as_ref(), match_nan_test_expr(rhs))
            {
                if tmp != &var && matches!(stmts.get(idx + 1), Some(Stmt::Labeled(..))) {
                    let single_read = read_counts.get(tmp).copied().unwrap_or(0) == 1;
                    (idx + 1, Some((tmp.as_str(), var, is_nan)), single_read)
                } else {
                    (idx, None, false)
                }
            } else {
                (idx, None, false)
            }
        } else {
            (idx, None, false)
        };

    let Stmt::Labeled(label, inner_box) = stmts.get(label_idx)? else {
        return None;
    };
    let Stmt::Block(block) = inner_box.as_ref() else {
        return None;
    };
    let [Stmt::If(outer_cond, outer_then, None), Stmt::ExprStmt(Expr::Assign(dst_lhs, nan_rhs))] =
        block.as_slice()
    else {
        return None;
    };
    let Expr::Ident(dst) = dst_lhs.as_ref() else {
        return None;
    };
    if !is_nan_expr(nan_rhs) {
        return None;
    }
    // If keeping the preceding `c_ = k !== k;` (because `c_` is read later), `dst` must not be `c_`.
    if !consume_prev {
        if let Some((tmp, _, _)) = &prev_nan_tmp {
            if dst == *tmp {
                return None;
            }
        }
    }

    let prev_ref = prev_nan_tmp
        .as_ref()
        .map(|(tmp, var, is_nan)| (*tmp, var.as_str(), *is_nan));

    // Either `outer_cond` checks `!is_nan(a)` alone, or collapsed `!is_nan(a) && !is_nan(b)`.
    let matched: Option<(String, Expr, bool)> = if let Some(a_var) =
        match_non_nan_guard_cond(outer_cond, prev_ref)
    {
        // Try 2-variable nested NaN check first:
        match outer_then.as_slice() {
            [Stmt::ExprStmt(Expr::Assign(da_lhs, da_rhs)), Stmt::If(b_cond, b_then, None)] => {
                if let (Expr::Ident(da_name), Some((b_var, da_is_nan))) =
                    (da_lhs.as_ref(), match_nan_test_expr(da_rhs))
                {
                    if read_counts.get(da_name).copied().unwrap_or(0) == 1
                        && da_name != dst
                        && da_name != &a_var
                        && da_name != &b_var
                        && match_non_nan_guard_cond(
                            b_cond,
                            Some((da_name.as_str(), b_var.as_str(), da_is_nan)),
                        )
                        .as_deref()
                            == Some(b_var.as_str())
                    {
                        match_max_min_compare_body(
                            b_then,
                            &a_var,
                            Some(&b_var),
                            dst,
                            label,
                            read_counts,
                        )
                        .map(|(b_expr, is_max)| (a_var.clone(), b_expr, is_max))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            [Stmt::If(b_cond, b_then, None)] => {
                if let Some(b_var) = match_non_nan_guard_cond(b_cond, None) {
                    match_max_min_compare_body(
                        b_then,
                        &a_var,
                        Some(&b_var),
                        dst,
                        label,
                        read_counts,
                    )
                    .map(|(b_expr, is_max)| (a_var.clone(), b_expr, is_max))
                } else {
                    // Could be 1-variable + constant (no second NaN check).
                    match_max_min_compare_body(outer_then, &a_var, None, dst, label, read_counts)
                        .map(|(b_expr, is_max)| (a_var.clone(), b_expr, is_max))
                }
            }
            _ => match_max_min_compare_body(outer_then, &a_var, None, dst, label, read_counts)
                .map(|(b_expr, is_max)| (a_var.clone(), b_expr, is_max)),
        }
    } else if let Expr::Binary(BinOp::And, c1, c2) = outer_cond {
        if let (Some(a_var), Some(b_var)) = (
            match_non_nan_guard_cond(c1, prev_ref),
            match_non_nan_guard_cond(c2, None),
        ) {
            match_max_min_compare_body(outer_then, &a_var, Some(&b_var), dst, label, read_counts)
                .map(|(b_expr, is_max)| (a_var, b_expr, is_max))
        } else {
            None
        }
    } else {
        None
    };

    let (a_var, b_expr, is_max) = matched?;
    let method = if is_max { "max" } else { "min" };
    let repl = jsast::assign_stmt(
        jsast::id(dst.clone()),
        jsast::method_call(jsast::id("Math"), method, vec![jsast::id(a_var), b_expr]),
    );
    let start = if consume_prev { idx } else { label_idx };
    Some((start, label_idx + 1, repl))
}

fn fold_ieee_max_min_stmts(stmts: &mut Vec<Stmt>, read_counts: &HashMap<String, usize>) {
    // Recurse into nested structures first.
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::If(_, t, e) => {
                fold_ieee_max_min_stmts(t, read_counts);
                if let Some(el) = e {
                    fold_ieee_max_min_stmts(el, read_counts);
                }
            }
            Stmt::Block(b) | Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
                fold_ieee_max_min_stmts(b, read_counts);
            }
            Stmt::Labeled(_, inner) => {
                if let Stmt::Block(b) | Stmt::LoopForever(_, b) | Stmt::While(_, b) = inner.as_mut()
                {
                    fold_ieee_max_min_stmts(b, read_counts);
                }
            }
            Stmt::Switch(_, cases) => {
                for c in cases {
                    fold_ieee_max_min_stmts(&mut c.body, read_counts);
                }
            }
            Stmt::TryCatch { body, handler, .. } => {
                fold_ieee_max_min_stmts(body, read_counts);
                fold_ieee_max_min_stmts(handler, read_counts);
            }
            _ => {}
        }
    }

    let mut i = 0usize;
    while i < stmts.len() {
        if let Some((start, end, repl)) = match_ieee_max_min_at(stmts, i, read_counts) {
            stmts.splice(start..end, std::iter::once(repl));
            i = start + 1;
        } else {
            i += 1;
        }
    }
}

/// Straight-line CSE for repeated pure `s.charCodeAt(idx)` loads dominated by an unconditional
/// first evaluation (e.g. `65 <= a.charCodeAt(A) && a.charCodeAt(A) <= 90` followed by
/// `97 <= a.charCodeAt(A)` in `marked`).
fn cse_dominating_char_code_at(
    stmts: &mut Vec<Stmt>,
    local_names: &mut HashSet<String>,
    extra_decls: &mut Vec<(String, Option<Expr>)>,
) {
    let mut i = 0usize;
    while i < stmts.len() {
        // First recurse into nested blocks.
        match &mut stmts[i] {
            Stmt::If(_, t, e) => {
                cse_dominating_char_code_at(t, local_names, extra_decls);
                if let Some(el) = e {
                    cse_dominating_char_code_at(el, local_names, extra_decls);
                }
            }
            Stmt::Block(b) | Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
                cse_dominating_char_code_at(b, local_names, extra_decls);
            }
            Stmt::Labeled(_, inner) => {
                if let Stmt::Block(b) | Stmt::LoopForever(_, b) | Stmt::While(_, b) = inner.as_mut()
                {
                    cse_dominating_char_code_at(b, local_names, extra_decls);
                }
            }
            Stmt::Switch(_, cases) => {
                for c in cases {
                    cse_dominating_char_code_at(&mut c.body, local_names, extra_decls);
                }
            }
            Stmt::TryCatch { body, handler, .. } => {
                cse_dominating_char_code_at(body, local_names, extra_decls);
                cse_dominating_char_code_at(handler, local_names, extra_decls);
            }
            _ => {}
        }

        // Look for an unconditional `s.charCodeAt(idx)` evaluated at the start of `stmts[i]`.
        let candidate = first_unconditional_char_code_at_stmt(&stmts[i]);
        let Some((recv, idx_expr)) = candidate else {
            i += 1;
            continue;
        };
        let mut deps = HashSet::new();
        deps.insert(recv.clone());
        collect_read_idents_pure_expr(&idx_expr, &mut deps);

        let target = jsast::method_call(jsast::id(recv), "charCodeAt", vec![idx_expr]);
        // Find maximal straight-line span `[i..end]` where no statement writes any dep in `deps`
        // (we may include the first statement that writes a dep only if the occurrences precede
        // the write — conservatively stop before any statement writing a dep, except `stmts[i]`
        // itself when `stmts[i]` is `Stmt::If(cond, ..)` whose condition does not write deps!).
        let mut end = i;
        let mut total = 0usize;
        let mut handled_cond_only = false;
        while end < stmts.len() {
            let mut written = HashSet::new();
            collect_written_idents_stmt(&stmts[end], &mut written);
            if written.iter().any(|w| deps.contains(w)) {
                // Still allow CSE within `stmts[i]`'s own `if` condition if `cond` doesn't write deps.
                if end == i {
                    if let Stmt::If(cond, _, _) = &stmts[i] {
                        let mut cond_w = HashSet::new();
                        collect_written_idents_expr(cond, &mut cond_w);
                        if !cond_w.iter().any(|w| deps.contains(w)) {
                            let c = count_expr_eq_in_expr(cond, &target);
                            if c >= 2 {
                                let tmp = alloc_fresh_local(local_names, extra_decls, "$cc");
                                if let Stmt::If(cond_mut, _, _) = &mut stmts[i] {
                                    replace_expr_eq_in_expr(
                                        cond_mut,
                                        &target,
                                        &Expr::Ident(tmp.clone()),
                                    );
                                }
                                stmts.insert(i, jsast::assign_stmt(jsast::id(tmp), target.clone()));
                                handled_cond_only = true;
                            }
                        }
                    }
                }
                break;
            }
            total += count_expr_eq_in_stmt(&stmts[end], &target);
            end += 1;
        }
        if handled_cond_only {
            i += 2;
            continue;
        }
        if total >= 2 {
            // If `stmts[i]` is already `v = s.charCodeAt(idx)` and `v` isn't written in `[i+1..end]`,
            // reuse `v` directly instead of allocating a fresh temp.
            let existing_local = match &stmts[i] {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) if rhs.as_ref() == &target => {
                    if let Expr::Ident(v) = lhs.as_ref() {
                        let written_later = (i + 1..end).any(|k| {
                            let mut w = HashSet::new();
                            collect_written_idents_stmt(&stmts[k], &mut w);
                            w.contains(v)
                        });
                        if !written_later {
                            Some(v.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(v) = existing_local {
                let repl = Expr::Ident(v);
                for s in &mut stmts[i + 1..end] {
                    replace_expr_eq_in_stmt(s, &target, &repl);
                }
                i += 1;
            } else {
                let tmp = alloc_fresh_local(local_names, extra_decls, "$cc");
                let repl = Expr::Ident(tmp.clone());
                for s in &mut stmts[i..end] {
                    replace_expr_eq_in_stmt(s, &target, &repl);
                }
                stmts.insert(i, jsast::assign_stmt(jsast::id(tmp), target));
                i += 2;
            }
        } else {
            i += 1;
        }
    }
}

fn is_pure_index_expr(expr: &Expr) -> bool {
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

fn collect_read_idents_pure_expr(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Ident(id) => {
            out.insert(id.clone());
        }
        Expr::Unary(_, inner) => collect_read_idents_pure_expr(inner, out),
        Expr::Binary(_, l, r) => {
            collect_read_idents_pure_expr(l, out);
            collect_read_idents_pure_expr(r, out);
        }
        _ => {}
    }
}

fn first_unconditional_char_code_at_expr(expr: &Expr) -> Option<(String, Expr)> {
    match expr {
        Expr::Call(callee, args) if args.len() == 1 => {
            if let Expr::Member(obj, prop) = callee.as_ref() {
                if prop == "charCodeAt" {
                    if let Expr::Ident(recv) = obj.as_ref() {
                        if is_pure_index_expr(&args[0]) {
                            return Some((recv.clone(), args[0].clone()));
                        }
                    }
                }
            }
            None
        }
        Expr::Unary(_, inner) => first_unconditional_char_code_at_expr(inner),
        Expr::Binary(BinOp::And | BinOp::Or, l, _) => first_unconditional_char_code_at_expr(l),
        Expr::Binary(_, l, r) => first_unconditional_char_code_at_expr(l).or_else(|| {
            if !loop_opt::expr_has_side_effect(l) {
                first_unconditional_char_code_at_expr(r)
            } else {
                None
            }
        }),
        Expr::Cond(c, _, _) => first_unconditional_char_code_at_expr(c),
        Expr::Assign(lhs, rhs) if matches!(lhs.as_ref(), Expr::Ident(_)) => {
            first_unconditional_char_code_at_expr(rhs)
        }
        _ => None,
    }
}

fn first_unconditional_char_code_at_stmt(stmt: &Stmt) -> Option<(String, Expr)> {
    match stmt {
        Stmt::If(cond, _, _) => first_unconditional_char_code_at_expr(cond),
        Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => first_unconditional_char_code_at_expr(e),
        Stmt::Switch(scrut, _) => first_unconditional_char_code_at_expr(scrut),
        _ => None,
    }
}

fn count_expr_eq_in_expr(expr: &Expr, target: &Expr) -> usize {
    if expr == target {
        return 1;
    }
    match expr {
        Expr::Member(o, _) => count_expr_eq_in_expr(o, target),
        Expr::Index(o, i) => count_expr_eq_in_expr(o, target) + count_expr_eq_in_expr(i, target),
        Expr::Call(c, args) | Expr::New(c, args) => {
            count_expr_eq_in_expr(c, target)
                + args
                    .iter()
                    .map(|a| count_expr_eq_in_expr(a, target))
                    .sum::<usize>()
        }
        Expr::Unary(_, inner) => count_expr_eq_in_expr(inner, target),
        Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
            count_expr_eq_in_expr(l, target) + count_expr_eq_in_expr(r, target)
        }
        Expr::Cond(c, t, e) => {
            count_expr_eq_in_expr(c, target)
                + count_expr_eq_in_expr(t, target)
                + count_expr_eq_in_expr(e, target)
        }
        Expr::Object(entries) => entries
            .iter()
            .map(|(_, v)| count_expr_eq_in_expr(v, target))
            .sum(),
        Expr::Array(items) | Expr::Seq(items) => items
            .iter()
            .map(|it| count_expr_eq_in_expr(it, target))
            .sum(),
        _ => 0,
    }
}

fn count_expr_eq_in_stmt(stmt: &Stmt, target: &Expr) -> usize {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Return(Some(e)) | Stmt::Throw(e) => {
            count_expr_eq_in_expr(e, target)
        }
        Stmt::If(c, t, e) => {
            count_expr_eq_in_expr(c, target)
                + t.iter()
                    .map(|s| count_expr_eq_in_stmt(s, target))
                    .sum::<usize>()
                + e.as_ref()
                    .map(|el| {
                        el.iter()
                            .map(|s| count_expr_eq_in_stmt(s, target))
                            .sum::<usize>()
                    })
                    .unwrap_or(0)
        }
        Stmt::Block(b) => b.iter().map(|s| count_expr_eq_in_stmt(s, target)).sum(),
        Stmt::Labeled(_, inner) => count_expr_eq_in_stmt(inner, target),
        Stmt::Switch(scrut, cases) => {
            count_expr_eq_in_expr(scrut, target)
                + cases
                    .iter()
                    .map(|c| {
                        c.body
                            .iter()
                            .map(|s| count_expr_eq_in_stmt(s, target))
                            .sum::<usize>()
                    })
                    .sum::<usize>()
        }
        _ => 0,
    }
}

fn replace_expr_eq_in_expr(expr: &mut Expr, target: &Expr, repl: &Expr) {
    if expr == target {
        *expr = repl.clone();
        return;
    }
    match expr {
        Expr::Member(o, _) => replace_expr_eq_in_expr(o, target, repl),
        Expr::Index(o, i) => {
            replace_expr_eq_in_expr(o, target, repl);
            replace_expr_eq_in_expr(i, target, repl);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            replace_expr_eq_in_expr(c, target, repl);
            for a in args {
                replace_expr_eq_in_expr(a, target, repl);
            }
        }
        Expr::Unary(_, inner) => replace_expr_eq_in_expr(inner, target, repl),
        Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
            replace_expr_eq_in_expr(l, target, repl);
            replace_expr_eq_in_expr(r, target, repl);
        }
        Expr::Cond(c, t, e) => {
            replace_expr_eq_in_expr(c, target, repl);
            replace_expr_eq_in_expr(t, target, repl);
            replace_expr_eq_in_expr(e, target, repl);
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                replace_expr_eq_in_expr(v, target, repl);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for it in items {
                replace_expr_eq_in_expr(it, target, repl);
            }
        }
        _ => {}
    }
}

fn replace_expr_eq_in_stmt(stmt: &mut Stmt, target: &Expr, repl: &Expr) {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Return(Some(e)) | Stmt::Throw(e) => {
            replace_expr_eq_in_expr(e, target, repl);
        }
        Stmt::If(c, t, e) => {
            replace_expr_eq_in_expr(c, target, repl);
            for s in t {
                replace_expr_eq_in_stmt(s, target, repl);
            }
            if let Some(el) = e {
                for s in el {
                    replace_expr_eq_in_stmt(s, target, repl);
                }
            }
        }
        Stmt::Block(b) => {
            for s in b {
                replace_expr_eq_in_stmt(s, target, repl);
            }
        }
        Stmt::Labeled(_, inner) => replace_expr_eq_in_stmt(inner, target, repl),
        Stmt::Switch(scrut, cases) => {
            replace_expr_eq_in_expr(scrut, target, repl);
            for c in cases {
                for s in &mut c.body {
                    replace_expr_eq_in_stmt(s, target, repl);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_const_array_index_and_const_switch() {
        let mut decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: vec!["x".to_owned()],
            body: vec![
                jsast::lets(vec![("k".to_owned(), None), ("r".to_owned(), None)]),
                jsast::assign_stmt(jsast::id("k"), Expr::Str("Some".to_owned())),
                Stmt::Switch(
                    jsast::id("k"),
                    vec![
                        jsast::SwitchCase {
                            tests: vec![Expr::Str("None".to_owned())],
                            body: vec![
                                jsast::assign_stmt(jsast::id("r"), Expr::Num(0.0)),
                                Stmt::Break(None),
                            ],
                        },
                        jsast::SwitchCase {
                            tests: vec![Expr::Str("Some".to_owned())],
                            body: vec![
                                jsast::assign_stmt(
                                    jsast::id("r"),
                                    Expr::Index(
                                        Box::new(Expr::Array(vec![Expr::Num(1.0)])),
                                        Box::new(Expr::Num(0.0)),
                                    ),
                                ),
                                Stmt::Break(None),
                            ],
                        },
                    ],
                ),
                Stmt::Return(Some(jsast::id("r"))),
            ],
        };
        run_speed_post_opts(&mut decl);
        let printed = jsast::stmt_to_string(&decl);
        assert!(
            !printed.contains("switch"),
            "expected switch folded: {printed}"
        );
        assert!(
            !printed.contains("[1][0]"),
            "expected [1][0] folded: {printed}"
        );
        assert!(
            printed.contains("return 1;"),
            "expected return 1;: {printed}"
        );
    }

    #[test]
    fn optimizes_reflect_get_with_dominated_guard_cse() {
        let mut decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: vec!["a".to_owned()],
            body: vec![
                jsast::lets(vec![
                    ("k".to_owned(), None),
                    ("x".to_owned(), None),
                    ("y".to_owned(), None),
                ]),
                jsast::assign_stmt(jsast::id("k"), Expr::Str("children".to_owned())),
                jsast::assign_stmt(
                    jsast::id("x"),
                    jsast::method_call(
                        jsast::id("Reflect"),
                        "get",
                        vec![jsast::id("a"), jsast::id("k")],
                    ),
                ),
                jsast::assign_stmt(
                    jsast::id("y"),
                    jsast::method_call(
                        jsast::id("Reflect"),
                        "get",
                        vec![jsast::id("a"), Expr::Str("name".to_owned())],
                    ),
                ),
                Stmt::Return(Some(jsast::id("y"))),
            ],
        };
        run_speed_post_opts(&mut decl);
        let printed = jsast::stmt_to_string(&decl);
        assert!(
            printed.contains("a.children"),
            "expected a.children: {printed}"
        );
        assert!(
            printed.contains("y = a.name;"),
            "expected unguarded dominated a.name: {printed}"
        );
        assert_eq!(
            printed.matches("Reflect.get").count(),
            1,
            "expected only 1 fallback guard: {printed}"
        );
    }

    #[test]
    fn fuses_adjacent_i64_rem_and_div() {
        let mut decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: vec![
                "r".to_owned(),
                "q".to_owned(),
                "e".to_owned(),
                "d".to_owned(),
            ],
            body: vec![
                jsast::lets(vec![("rl".to_owned(), None), ("rh".to_owned(), None)]),
                jsast::assign_stmt(
                    jsast::id("rl"),
                    jsast::call(
                        jsast::id(i64_pair::HELPER_DIV),
                        vec![
                            jsast::id("r"),
                            jsast::id("q"),
                            jsast::id("e"),
                            jsast::id("d"),
                            Expr::Bool(false),
                            Expr::Bool(true),
                        ],
                    ),
                ),
                jsast::assign_stmt(jsast::id("rh"), jsast::id(i64_pair::HELPER_HI)),
                jsast::assign_stmt(
                    jsast::id("r"),
                    jsast::call(
                        jsast::id(i64_pair::HELPER_DIV),
                        vec![
                            jsast::id("r"),
                            jsast::id("q"),
                            jsast::id("e"),
                            jsast::id("d"),
                            Expr::Bool(false),
                            Expr::Bool(false),
                        ],
                    ),
                ),
                jsast::assign_stmt(jsast::id("q"), jsast::id(i64_pair::HELPER_HI)),
                Stmt::Return(Some(jsast::id("rl"))),
            ],
        };
        run_speed_post_opts(&mut decl);
        let printed = jsast::stmt_to_string(&decl);
        assert!(
            printed.contains(i64_pair::HELPER_DIVREM),
            "expected divrem: {printed}"
        );
        assert!(
            !printed.contains("$__mires_i64_div("),
            "expected both div calls fused: {printed}"
        );
    }

    #[test]
    fn fast_paths_dead_lo_f64_sign_bit_and_preserves_live_lo() {
        let sign_bit_test = |hi: &str| {
            Expr::Binary(
                BinOp::StrictNe,
                Box::new(Expr::Binary(
                    BinOp::BitAnd,
                    Box::new(jsast::id(hi)),
                    Box::new(Expr::Unary(UnOp::Neg, Box::new(Expr::Num(2147483648.0)))),
                )),
                Box::new(Expr::Num(0.0)),
            )
        };
        // Dead `lo` sign-bit test: fast-path `x === x ? (x < 0 || x === 0 && 1 / x < 0) : ...`
        // for general floats, and completely eliminates `f64_to_bits` when `x` is known non-NaN.
        let mut decl = Stmt::FunctionDecl {
            name: "is_neg".to_owned(),
            params: vec!["x".to_owned(), "y".to_owned()],
            body: vec![
                jsast::lets(vec![
                    ("lo".to_owned(), None),
                    ("hi".to_owned(), None),
                    ("b".to_owned(), None),
                ]),
                Stmt::If(
                    Expr::Binary(
                        BinOp::StrictEq,
                        Box::new(jsast::id("x")),
                        Box::new(jsast::id("y")),
                    ),
                    vec![
                        jsast::assign_stmt(
                            jsast::id("lo"),
                            jsast::call(
                                jsast::id("$__mires_i64_f64_to_bits"),
                                vec![jsast::id("x")],
                            ),
                        ),
                        jsast::assign_stmt(jsast::id("hi"), jsast::id("$__mires_i64_hi")),
                        jsast::assign_stmt(jsast::id("b"), sign_bit_test("hi")),
                        Stmt::Return(Some(jsast::id("b"))),
                    ],
                    None,
                ),
                Stmt::Return(Some(Expr::Bool(false))),
            ],
        };
        run_speed_post_opts(&mut decl);
        let printed = jsast::stmt_to_string(&decl);
        assert!(
            !printed.contains("$__mires_i64_f64_to_bits"),
            "expected f64_to_bits eliminated when known non-NaN: {printed}"
        );
        assert!(
            printed.contains("1 / x < 0"),
            "expected IEEE-754 negative-zero check: {printed}"
        );

        // Live `lo` must keep `$__mires_i64_f64_to_bits` and never rewrite.
        let mut live_lo = Stmt::FunctionDecl {
            name: "keep_bits".to_owned(),
            params: vec!["x".to_owned()],
            body: vec![
                jsast::lets(vec![
                    ("lo".to_owned(), None),
                    ("hi".to_owned(), None),
                    ("b".to_owned(), None),
                ]),
                jsast::assign_stmt(
                    jsast::id("lo"),
                    jsast::call(jsast::id("$__mires_i64_f64_to_bits"), vec![jsast::id("x")]),
                ),
                jsast::assign_stmt(jsast::id("hi"), jsast::id("$__mires_i64_hi")),
                jsast::assign_stmt(jsast::id("b"), sign_bit_test("hi")),
                Stmt::Return(Some(Expr::Binary(
                    BinOp::BitXor,
                    Box::new(jsast::id("lo")),
                    Box::new(jsast::id("b")),
                ))),
            ],
        };
        run_speed_post_opts(&mut live_lo);
        let live_printed = jsast::stmt_to_string(&live_lo);
        assert!(
            live_printed.contains("$__mires_i64_f64_to_bits")
                && !live_printed.contains("1 / x < 0"),
            "live `lo` must preserve f64_to_bits: {live_printed}"
        );
    }

    #[test]
    fn cses_dominating_char_code_at_and_folds_bool_bit_identities() {
        let char_at = || jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::id("i")]);
        let mut decl = Stmt::FunctionDecl {
            name: "scan".to_owned(),
            params: vec!["s".to_owned(), "i".to_owned(), "x".to_owned()],
            body: vec![
                jsast::lets(vec![("a".to_owned(), None), ("b".to_owned(), None)]),
                jsast::assign_stmt(
                    jsast::id("a"),
                    Expr::Cond(
                        Box::new(Expr::Binary(
                            BinOp::StrictEq,
                            Box::new(char_at()),
                            Box::new(Expr::Num(32.0)),
                        )),
                        Box::new(Expr::Num(1.0)),
                        Box::new(Expr::Num(0.0)),
                    ),
                ),
                Stmt::If(
                    Expr::Binary(
                        BinOp::StrictNe,
                        Box::new(Expr::Binary(
                            BinOp::StrictEq,
                            Box::new(char_at()),
                            Box::new(Expr::Num(9.0)),
                        )),
                        Box::new(Expr::Bool(false)),
                    ),
                    vec![jsast::assign_stmt(
                        jsast::id("b"),
                        jsast::imul(
                            Expr::Binary(
                                BinOp::BitAnd,
                                Box::new(Expr::Binary(
                                    BinOp::BitAnd,
                                    Box::new(jsast::id("x")),
                                    Box::new(Expr::Num(255.0)),
                                )),
                                Box::new(Expr::Num(15.0)),
                            ),
                            Expr::Num(1.0),
                        ),
                    )],
                    None,
                ),
                Stmt::Return(Some(Expr::Binary(
                    BinOp::Add,
                    Box::new(jsast::id("a")),
                    Box::new(jsast::id("b")),
                ))),
            ],
        };
        run_speed_post_opts(&mut decl);
        let printed = jsast::stmt_to_string(&decl);
        assert_eq!(
            printed.matches("charCodeAt").count(),
            1,
            "expected dominating charCodeAt CSEd to 1 call: {printed}"
        );
        assert!(
            !printed.contains("Math.imul")
                && !printed.contains("? 1 : 0")
                && !printed.contains("!== false"),
            "expected imul(x,1), bool?1:0, and !==false folded: {printed}"
        );
        assert!(
            printed.contains("x & 15"),
            "expected nested bitmask folded to `x & 15`: {printed}"
        );
    }

    #[test]
    fn folds_proven_nonneg_ushr_zero_and_safe_zero_add() {
        let ushr0 = |e: Expr| Expr::Binary(BinOp::UShr, Box::new(e), Box::new(Expr::Num(0.0)));
        let mut decl = Stmt::FunctionDecl {
            name: "alg".to_owned(),
            params: vec![
                "a".to_owned(),
                "b".to_owned(),
                "arr".to_owned(),
                "i".to_owned(),
                "s".to_owned(),
            ],
            body: vec![
                jsast::lets(vec![
                    ("m1".to_owned(), None),
                    ("m_neg".to_owned(), None),
                    ("sh".to_owned(), None),
                    ("xr".to_owned(), None),
                    ("xr_neg".to_owned(), None),
                    ("idx".to_owned(), None),
                    ("z_int".to_owned(), None),
                    ("z_str".to_owned(), None),
                ]),
                // `(a & 255) >>> 0` -> `a & 255`
                jsast::assign_stmt(
                    jsast::id("m1"),
                    ushr0(Expr::Binary(
                        BinOp::BitAnd,
                        Box::new(jsast::id("a")),
                        Box::new(Expr::Num(255.0)),
                    )),
                ),
                // `(a & 0x80000000) >>> 0` MUST keep `>>> 0` (bit 31 set!)
                jsast::assign_stmt(
                    jsast::id("m_neg"),
                    ushr0(Expr::Binary(
                        BinOp::BitAnd,
                        Box::new(jsast::id("a")),
                        Box::new(Expr::Num(2147483648.0)),
                    )),
                ),
                // `(a >>> 5) >>> 0` -> `a >>> 5`
                jsast::assign_stmt(
                    jsast::id("sh"),
                    ushr0(Expr::Binary(
                        BinOp::UShr,
                        Box::new(jsast::id("a")),
                        Box::new(Expr::Num(5.0)),
                    )),
                ),
                // `((a & 255) ^ (b >>> 24)) >>> 0` -> `(a & 255) ^ (b >>> 24)`
                jsast::assign_stmt(
                    jsast::id("xr"),
                    ushr0(Expr::Binary(
                        BinOp::BitXor,
                        Box::new(Expr::Binary(
                            BinOp::BitAnd,
                            Box::new(jsast::id("a")),
                            Box::new(Expr::Num(255.0)),
                        )),
                        Box::new(Expr::Binary(
                            BinOp::UShr,
                            Box::new(jsast::id("b")),
                            Box::new(Expr::Num(24.0)),
                        )),
                    )),
                ),
                // `((a & 255) ^ (b >>> 0)) >>> 0` MUST keep `>>> 0` (b >>> 0 can have bit 31 set!)
                jsast::assign_stmt(
                    jsast::id("xr_neg"),
                    ushr0(Expr::Binary(
                        BinOp::BitXor,
                        Box::new(Expr::Binary(
                            BinOp::BitAnd,
                            Box::new(jsast::id("a")),
                            Box::new(Expr::Num(255.0)),
                        )),
                        Box::new(ushr0(jsast::id("b"))),
                    )),
                ),
                // `arr[0 + i]` -> `arr[i]`
                jsast::assign_stmt(
                    jsast::id("idx"),
                    jsast::index(
                        jsast::id("arr"),
                        Expr::Binary(
                            BinOp::Add,
                            Box::new(Expr::Num(0.0)),
                            Box::new(jsast::id("i")),
                        ),
                    ),
                ),
                // `0 + (a & 31)` -> `a & 31`
                jsast::assign_stmt(
                    jsast::id("z_int"),
                    Expr::Binary(
                        BinOp::Add,
                        Box::new(Expr::Num(0.0)),
                        Box::new(Expr::Binary(
                            BinOp::BitAnd,
                            Box::new(jsast::id("a")),
                            Box::new(Expr::Num(31.0)),
                        )),
                    ),
                ),
                // `0 + s` MUST NOT fold (could be string concatenation!)
                jsast::assign_stmt(
                    jsast::id("z_str"),
                    Expr::Binary(
                        BinOp::Add,
                        Box::new(Expr::Num(0.0)),
                        Box::new(jsast::id("s")),
                    ),
                ),
                Stmt::Return(Some(Expr::Array(vec![
                    jsast::id("m1"),
                    jsast::id("m_neg"),
                    jsast::id("sh"),
                    jsast::id("xr"),
                    jsast::id("xr_neg"),
                    jsast::id("idx"),
                    jsast::id("z_int"),
                    jsast::id("z_str"),
                ]))),
            ],
        };
        run_speed_post_opts(&mut decl);
        let printed = jsast::stmt_to_string(&decl);
        assert!(
            printed.contains("m1 = a & 255;"),
            "expected (a & 255) >>> 0 folded: {printed}"
        );
        assert!(
            printed.contains("m_neg = (a & 2147483648) >>> 0;"),
            "expected (a & 0x80000000) >>> 0 preserved: {printed}"
        );
        assert!(
            printed.contains("sh = a >>> 5;"),
            "expected (a >>> 5) >>> 0 folded: {printed}"
        );
        assert!(
            printed.contains("xr = a & 255 ^ b >>> 24;"),
            "expected proven-nonneg xor >>> 0 folded: {printed}"
        );
        assert!(
            printed.contains("xr_neg = (a & 255 ^ b >>> 0) >>> 0;"),
            "expected possibly-negative xor >>> 0 preserved: {printed}"
        );
        assert!(
            printed.contains("idx = arr[i];"),
            "expected arr[0 + i] folded to arr[i]: {printed}"
        );
        assert!(
            printed.contains("z_int = a & 31;"),
            "expected 0 + (a & 31) folded: {printed}"
        );
        assert!(
            printed.contains("z_str = 0 + s;"),
            "0 + arbitrary ident must NOT fold: {printed}"
        );
    }

    #[test]
    fn folds_ieee_max_min_two_var_and_const_idioms_and_rejects_unsound_neg_zero() {
        let sign_neg = |v: &str| {
            Expr::Binary(
                BinOp::Or,
                Box::new(Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id(v)),
                    Box::new(Expr::Num(0.0)),
                )),
                Box::new(Expr::Binary(
                    BinOp::And,
                    Box::new(Expr::Binary(
                        BinOp::StrictEq,
                        Box::new(jsast::id(v)),
                        Box::new(Expr::Num(0.0)),
                    )),
                    Box::new(Expr::Binary(
                        BinOp::Lt,
                        Box::new(Expr::Binary(
                            BinOp::Div,
                            Box::new(Expr::Num(1.0)),
                            Box::new(jsast::id(v)),
                        )),
                        Box::new(Expr::Num(0.0)),
                    )),
                )),
            )
        };
        let assign_brk = |dst: &str, val: Expr, lbl: &str| {
            vec![
                jsast::assign_stmt(jsast::id(dst), val),
                Stmt::Break(Some(lbl.to_owned())),
            ]
        };

        // 1. Exact 2-variable `Math.max(k, r)` + 2-variable `Math.min(k, r)` + 1-var const `1` + `+0.0`.
        let mut decl = Stmt::FunctionDecl {
            name: "minmax".to_owned(),
            params: vec!["k".to_owned(), "r".to_owned()],
            body: vec![
                jsast::lets(vec![
                    ("c_".to_owned(), None),
                    ("da".to_owned(), None),
                    ("dd".to_owned(), None),
                    ("j".to_owned(), None),
                    ("c2".to_owned(), None),
                    ("da2".to_owned(), None),
                    ("dd2".to_owned(), None),
                    ("m".to_owned(), None),
                    ("dh".to_owned(), None),
                    ("y1".to_owned(), None),
                    ("d0".to_owned(), None),
                    ("y0max".to_owned(), None),
                    ("d0m".to_owned(), None),
                    ("y0min".to_owned(), None),
                ]),
                // Math.max(k, r)
                jsast::assign_stmt(
                    jsast::id("c_"),
                    Expr::Binary(
                        BinOp::StrictNe,
                        Box::new(jsast::id("k")),
                        Box::new(jsast::id("k")),
                    ),
                ),
                Stmt::Labeled(
                    "c".to_owned(),
                    Box::new(Stmt::Block(vec![
                        Stmt::If(
                            Expr::Unary(UnOp::Not, Box::new(jsast::id("c_"))),
                            vec![
                                jsast::assign_stmt(
                                    jsast::id("da"),
                                    Expr::Binary(
                                        BinOp::StrictNe,
                                        Box::new(jsast::id("r")),
                                        Box::new(jsast::id("r")),
                                    ),
                                ),
                                Stmt::If(
                                    Expr::Unary(UnOp::Not, Box::new(jsast::id("da"))),
                                    vec![Stmt::If(
                                        Expr::Binary(
                                            BinOp::StrictEq,
                                            Box::new(jsast::id("k")),
                                            Box::new(jsast::id("r")),
                                        ),
                                        vec![
                                            jsast::assign_stmt(jsast::id("dd"), sign_neg("k")),
                                            Stmt::If(
                                                jsast::id("dd"),
                                                assign_brk("j", jsast::id("r"), "c"),
                                                Some(assign_brk("j", jsast::id("k"), "c")),
                                            ),
                                        ],
                                        Some(vec![Stmt::If(
                                            Expr::Binary(
                                                BinOp::Gt,
                                                Box::new(jsast::id("k")),
                                                Box::new(jsast::id("r")),
                                            ),
                                            assign_brk("j", jsast::id("k"), "c"),
                                            Some(assign_brk("j", jsast::id("r"), "c")),
                                        )]),
                                    )],
                                    None,
                                ),
                            ],
                            None,
                        ),
                        jsast::assign_stmt(jsast::id("j"), jsast::id("NaN")),
                    ])),
                ),
                // Math.min(k, r)
                jsast::assign_stmt(
                    jsast::id("c2"),
                    Expr::Binary(
                        BinOp::StrictNe,
                        Box::new(jsast::id("k")),
                        Box::new(jsast::id("k")),
                    ),
                ),
                Stmt::Labeled(
                    "c2l".to_owned(),
                    Box::new(Stmt::Block(vec![
                        Stmt::If(
                            Expr::Unary(UnOp::Not, Box::new(jsast::id("c2"))),
                            vec![
                                jsast::assign_stmt(
                                    jsast::id("da2"),
                                    Expr::Binary(
                                        BinOp::StrictNe,
                                        Box::new(jsast::id("r")),
                                        Box::new(jsast::id("r")),
                                    ),
                                ),
                                Stmt::If(
                                    Expr::Unary(UnOp::Not, Box::new(jsast::id("da2"))),
                                    vec![Stmt::If(
                                        Expr::Binary(
                                            BinOp::StrictEq,
                                            Box::new(jsast::id("k")),
                                            Box::new(jsast::id("r")),
                                        ),
                                        vec![
                                            jsast::assign_stmt(jsast::id("dd2"), sign_neg("k")),
                                            Stmt::If(
                                                jsast::id("dd2"),
                                                assign_brk("m", jsast::id("k"), "c2l"),
                                                Some(assign_brk("m", jsast::id("r"), "c2l")),
                                            ),
                                        ],
                                        Some(vec![Stmt::If(
                                            Expr::Binary(
                                                BinOp::Lt,
                                                Box::new(jsast::id("k")),
                                                Box::new(jsast::id("r")),
                                            ),
                                            assign_brk("m", jsast::id("k"), "c2l"),
                                            Some(assign_brk("m", jsast::id("r"), "c2l")),
                                        )]),
                                    )],
                                    None,
                                ),
                            ],
                            None,
                        ),
                        jsast::assign_stmt(jsast::id("m"), jsast::id("NaN")),
                    ])),
                ),
                // Constant-1 variant: `Math.max(j, 1)`
                jsast::assign_stmt(
                    jsast::id("dh"),
                    Expr::Binary(
                        BinOp::StrictNe,
                        Box::new(jsast::id("j")),
                        Box::new(jsast::id("j")),
                    ),
                ),
                Stmt::Labeled(
                    "f".to_owned(),
                    Box::new(Stmt::Block(vec![
                        Stmt::If(
                            Expr::Unary(UnOp::Not, Box::new(jsast::id("dh"))),
                            vec![Stmt::If(
                                Expr::Binary(
                                    BinOp::StrictEq,
                                    Box::new(jsast::id("j")),
                                    Box::new(Expr::Num(1.0)),
                                ),
                                assign_brk("y1", jsast::id("j"), "f"),
                                Some(vec![Stmt::If(
                                    Expr::Binary(
                                        BinOp::Gt,
                                        Box::new(jsast::id("j")),
                                        Box::new(Expr::Num(1.0)),
                                    ),
                                    assign_brk("y1", jsast::id("j"), "f"),
                                    Some(assign_brk("y1", Expr::Num(1.0), "f")),
                                )]),
                            )],
                            None,
                        ),
                        jsast::assign_stmt(jsast::id("y1"), jsast::id("NaN")),
                    ])),
                ),
                // Constant +0.0 MAX (picks `+0.0` on `j === 0`) -> valid `Math.max(j, 0)`
                jsast::assign_stmt(
                    jsast::id("d0"),
                    Expr::Binary(
                        BinOp::StrictNe,
                        Box::new(jsast::id("j")),
                        Box::new(jsast::id("j")),
                    ),
                ),
                Stmt::Labeled(
                    "f0".to_owned(),
                    Box::new(Stmt::Block(vec![
                        Stmt::If(
                            Expr::Unary(UnOp::Not, Box::new(jsast::id("d0"))),
                            vec![Stmt::If(
                                Expr::Binary(
                                    BinOp::StrictEq,
                                    Box::new(jsast::id("j")),
                                    Box::new(Expr::Num(0.0)),
                                ),
                                assign_brk("y0max", Expr::Num(0.0), "f0"),
                                Some(vec![Stmt::If(
                                    Expr::Binary(
                                        BinOp::Gt,
                                        Box::new(jsast::id("j")),
                                        Box::new(Expr::Num(0.0)),
                                    ),
                                    assign_brk("y0max", jsast::id("j"), "f0"),
                                    Some(assign_brk("y0max", Expr::Num(0.0), "f0")),
                                )]),
                            )],
                            None,
                        ),
                        jsast::assign_stmt(jsast::id("y0max"), jsast::id("NaN")),
                    ])),
                ),
                // Constant +0.0 MIN (picks `j` on `j === 0`, preserving `-0.0`) -> valid `Math.min(j, 0)`
                jsast::assign_stmt(
                    jsast::id("d0m"),
                    Expr::Binary(
                        BinOp::StrictNe,
                        Box::new(jsast::id("j")),
                        Box::new(jsast::id("j")),
                    ),
                ),
                Stmt::Labeled(
                    "f0m".to_owned(),
                    Box::new(Stmt::Block(vec![
                        Stmt::If(
                            Expr::Unary(UnOp::Not, Box::new(jsast::id("d0m"))),
                            vec![Stmt::If(
                                Expr::Binary(
                                    BinOp::StrictEq,
                                    Box::new(jsast::id("j")),
                                    Box::new(Expr::Num(0.0)),
                                ),
                                assign_brk("y0min", jsast::id("j"), "f0m"),
                                Some(vec![Stmt::If(
                                    Expr::Binary(
                                        BinOp::Lt,
                                        Box::new(jsast::id("j")),
                                        Box::new(Expr::Num(0.0)),
                                    ),
                                    assign_brk("y0min", jsast::id("j"), "f0m"),
                                    Some(assign_brk("y0min", Expr::Num(0.0), "f0m")),
                                )]),
                            )],
                            None,
                        ),
                        jsast::assign_stmt(jsast::id("y0min"), jsast::id("NaN")),
                    ])),
                ),
                Stmt::Return(Some(Expr::Array(vec![
                    jsast::id("j"),
                    jsast::id("m"),
                    jsast::id("y1"),
                    jsast::id("y0max"),
                    jsast::id("y0min"),
                ]))),
            ],
        };
        run_speed_post_opts(&mut decl);
        let printed = jsast::stmt_to_string(&decl);
        assert!(
            printed.contains("j = Math.max(k, r);"),
            "expected 2-var Math.max folded: {printed}"
        );
        assert!(
            printed.contains("m = Math.min(k, r);"),
            "expected 2-var Math.min folded: {printed}"
        );
        assert!(
            printed.contains("y1 = Math.max(j, 1);"),
            "expected const-1 Math.max folded: {printed}"
        );
        assert!(
            printed.contains("y0max = Math.max(j, 0);"),
            "expected const-+0 Math.max folded: {printed}"
        );
        assert!(
            printed.contains("y0min = Math.min(j, 0);"),
            "expected const-+0 Math.min folded: {printed}"
        );
        assert!(
            !printed.contains("NaN") && !printed.contains("break "),
            "all five labeled NaN blocks should collapse cleanly: {printed}"
        );

        // 2. Negative `-0.0` soundness: MAX against `+0.0` that returns `j` on `j === 0` (which
        //    would return `-0.0` instead of `+0.0`) MUST NOT fold to `Math.max(j, 0)`.
        let mut bad_neg0 = Stmt::FunctionDecl {
            name: "bad_zero".to_owned(),
            params: vec!["j".to_owned()],
            body: vec![
                jsast::lets(vec![("dh".to_owned(), None), ("y".to_owned(), None)]),
                jsast::assign_stmt(
                    jsast::id("dh"),
                    Expr::Binary(
                        BinOp::StrictNe,
                        Box::new(jsast::id("j")),
                        Box::new(jsast::id("j")),
                    ),
                ),
                Stmt::Labeled(
                    "f".to_owned(),
                    Box::new(Stmt::Block(vec![
                        Stmt::If(
                            Expr::Unary(UnOp::Not, Box::new(jsast::id("dh"))),
                            vec![Stmt::If(
                                Expr::Binary(
                                    BinOp::StrictEq,
                                    Box::new(jsast::id("j")),
                                    Box::new(Expr::Num(0.0)),
                                ),
                                assign_brk("y", jsast::id("j"), "f"),
                                Some(vec![Stmt::If(
                                    Expr::Binary(
                                        BinOp::Gt,
                                        Box::new(jsast::id("j")),
                                        Box::new(Expr::Num(0.0)),
                                    ),
                                    assign_brk("y", jsast::id("j"), "f"),
                                    Some(assign_brk("y", Expr::Num(0.0), "f")),
                                )]),
                            )],
                            None,
                        ),
                        jsast::assign_stmt(jsast::id("y"), jsast::id("NaN")),
                    ])),
                ),
                Stmt::Return(Some(jsast::id("y"))),
            ],
        };
        run_speed_post_opts(&mut bad_neg0);
        let sb = jsast::stmt_to_string(&bad_neg0);
        assert!(
            !sb.contains("Math.max") && sb.contains("NaN"),
            "wrong -0.0 tie-break on MAX against +0.0 must NOT fold to Math.max: {sb}"
        );
    }
}
