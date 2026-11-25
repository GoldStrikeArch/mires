//! Post-lowering JavaScript AST speed optimizations (`ReleaseSpeed`).
//!
//! Runs after `rcgjs_object::sroa::analyze_jir` and `rcgjs_object::optimizer::analyze` inside
//! [`super::function::finish`] so passes see SROA-exposed patterns (`[1][0]`, single-assign
//! literal temps like `k = "Some"` / `E = "children"` / `ah = 14`, `Reflect.get(a, "children")`,
//! and adjacent `$__mires_i64_div` rem+div pairs).

use std::collections::{HashMap, HashSet};

use rcgjs_object::jsast::{self, ArrowBody, BinOp, Expr, Stmt, UnOp, Visit, VisitMut};

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
    fold_constants_and_switches(body, &uninit_locals, &local_names);

    // 1b. Fast-path dead-`lo` `$__mires_i64_f64_to_bits(x)` sign-bit tests (`f64::is_sign_negative`
    //     in `f64::min`/`f64::max`), then re-fold any exposed constant branches (`c === 1`).
    if fast_path_f64_sign_bit(body) {
        fold_constants_and_switches(body, &uninit_locals, &local_names);
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
        fold_constants_and_switches(body, &uninit_locals, &local_names);
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

    // 6b. Fold proven-integer two-sided range checks `(c >= A && c <= B)` -> `(c - A >>> 0) <= (B - A)`
    //     (including `if (A <= c) { b = c <= B; } else { b = false; }` and negated `||` forms).
    //     Runs before `smi_ize_counted_loop_steps` so `m = m + 1 >>> 0` retains its exact `U32`
    //     non-negativity proof for in-bounds `s.charCodeAt(m)` recognition.
    fold_unsigned_range_checks(body, &local_names);

    // 6c. Lower dense ASCII delimiter membership (`>= 6` byte constants in `0..=255` tested against
    //     a proven-numeric value, in `||`/`&&` chains or 2-arm `switch` statements) into a 128- or
    //     256-byte `Uint8Array` LUT (hoisted & deduplicated to module scope by `hoist_consts`).
    //     Gated by `allow_duplicating` so inline/ABI pricing trials never count a 128-element array.
    if allow_duplicating && !local_names.contains("Uint8Array") {
        lower_dense_ascii_luts(body);
    }

    // 6d. Lower chained string/integer equality `if / else if` and labeled guard chains (`>= 4`
    //     arms on the same pure identifier) into `switch (x)`.
    lower_chained_eq_to_switch(body);

    // 6e. Smi-ize counted loop steps (`i = i + 1 >>> 0` -> `i = i + 1 | 0`) and demote
    //     bit-exact 32-bit loop-carried registers (`x + z >>> 0` -> `x + z | 0`, `compress256`).
    loop_opt::smi_ize_counted_loop_steps(body);
    loop_opt::demote_bitexact_u32_wraps(body, params);

    // 6f. Fold Rust `ToInt32` labeled-block diamonds (`earcut` `zorder` in `earcut_linked` and
    //     `is_ear_hashed_sl`) to `dst = src | 0` (or `dst = src >>> 0`) when all intermediate
    //     temporaries written in the slow path are dead after the block.
    if !local_names.contains("Math") {
        fold_to_int32_diamonds(body, &local_names);
    }

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
    super::ast_util::collect_top_locals(body, local_names, uninit_locals);
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

/// Folds subexpressions of an assignment LHS without turning the LHS itself into a non-lvalue
/// (e.g. never collapsing `[a, b][0] = v` into `a = v` or `({ a }).a = v` into `a = v`).
fn fold_lvalue_expr(lhs: &mut Expr) -> bool {
    match lhs {
        Expr::Member(obj, _) => fold_expr(obj),
        Expr::Index(arr, idx) => {
            let mut changed = fold_expr(arr);
            changed |= fold_expr(idx);
            if let Expr::Binary(BinOp::Add, il, ir) = idx.as_ref() {
                if is_pos_zero(il) && is_numeric_index_subexpr(ir) {
                    **idx = (**ir).clone();
                    changed = true;
                } else if is_pos_zero(ir) && is_numeric_index_subexpr(il) {
                    **idx = (**il).clone();
                    changed = true;
                }
            }
            changed
        }
        other => fold_expr(other),
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
        // `[e0, e1, ...][k]` -> `e_k` when every discarded element `j != k` is side-effect-free
        // (collapses `[1][0]`, `[void 0, y][1]` in `minisearch`, `[U, $r1][0]` in `marked`), and
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
                if k < items.len()
                    && items
                        .iter()
                        .enumerate()
                        .all(|(j, it)| j == k || !loop_opt::expr_has_side_effect(it))
                {
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
            BinOp::And => match (l.as_ref(), r.as_ref()) {
                (Expr::Bool(true), _) => Some((**r).clone()),
                (Expr::Bool(false), _) => Some(Expr::Bool(false)),
                (_, Expr::Bool(true)) if is_bool_expr(l) => Some((**l).clone()),
                _ => None,
            },
            BinOp::Or => match (l.as_ref(), r.as_ref()) {
                (Expr::Bool(false), _) => Some((**r).clone()),
                (Expr::Bool(true), _) => Some(Expr::Bool(true)),
                (_, Expr::Bool(false)) if is_bool_expr(l) => Some((**l).clone()),
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
                    Expr::Binary(BinOp::Sub, zero, rhs)
                        if is_pos_zero(zero)
                            && !matches!(rhs.as_ref(), Expr::Unary(UnOp::Neg, _)) =>
                    {
                        Some(Expr::Binary(
                            BinOp::BitOr,
                            Box::new(Expr::Unary(UnOp::Neg, rhs.clone())),
                            Box::new(Expr::Num(0.0)),
                        ))
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
                    Expr::Binary(BinOp::Sub, zero, rhs)
                        if is_pos_zero(zero)
                            && !matches!(rhs.as_ref(), Expr::Unary(UnOp::Neg, _)) =>
                    {
                        Some(Expr::Binary(
                            BinOp::BitOr,
                            Box::new(Expr::Unary(UnOp::Neg, rhs.clone())),
                            Box::new(Expr::Num(0.0)),
                        ))
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

fn fold_stmt(stmt: &mut Stmt, local_names: &HashSet<String>) -> bool {
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
            changed |= unwrap_double_not_cond(cond);
            changed |= fold_stmt_list(then_b, local_names);
            if let Some(eb) = else_b {
                changed |= fold_stmt_list(eb, local_names);
            }
        }
        Stmt::Switch(disc, cases) => {
            changed |= fold_expr(disc);
            for c in cases {
                for t in &mut c.tests {
                    changed |= fold_expr(t);
                }
                changed |= fold_stmt_list(&mut c.body, local_names);
            }
        }
        Stmt::Labeled(_, inner) => changed |= fold_stmt(inner, local_names),
        Stmt::LoopForever(_, b) | Stmt::Block(b) => changed |= fold_stmt_list(b, local_names),
        Stmt::While(t, b) => {
            changed |= fold_expr(t);
            changed |= unwrap_double_not_cond(t);
            changed |= fold_stmt_list(b, local_names);
        }
        Stmt::TryCatch { body, handler, .. } => {
            changed |= fold_stmt_list(body, local_names);
            changed |= fold_stmt_list(handler, local_names);
        }
        _ => {}
    }
    changed
}

fn negate_bool_cond(cond: &Expr) -> Expr {
    match cond {
        Expr::Unary(UnOp::Not, inner) => (**inner).clone(),
        other => Expr::Unary(UnOp::Not, Box::new(other.clone())),
    }
}

fn fold_stmt_list(stmts: &mut Vec<Stmt>, local_names: &HashSet<String>) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i < stmts.len() {
        changed |= fold_stmt(&mut stmts[i], local_names);
        match &stmts[i] {
            // Eliminate no-op local self-assignments `x = x;` (exposes empty `then`/`else` arms
            // such as `if (F < y) { F = F; } else F = y;` in `earcut`).
            Stmt::ExprStmt(Expr::Assign(lhs, rhs))
                if matches!(
                    (lhs.as_ref(), rhs.as_ref()),
                    (Expr::Ident(a), Expr::Ident(b)) if a == b && local_names.contains(a)
                ) =>
            {
                stmts.remove(i);
                changed = true;
                continue;
            }
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
            Stmt::If(cond, then_b, else_b)
                if then_b.is_empty() && else_b.as_ref().is_none_or(Vec::is_empty) =>
            {
                if loop_opt::expr_has_side_effect(cond) {
                    stmts[i] = Stmt::ExprStmt(cond.clone());
                    changed = true;
                    i += 1;
                } else {
                    stmts.remove(i);
                    changed = true;
                }
                continue;
            }
            Stmt::If(cond, then_b, Some(else_b)) if then_b.is_empty() && !else_b.is_empty() => {
                let mut neg = negate_bool_cond(cond);
                fold_expr(&mut neg);
                unwrap_double_not_cond(&mut neg);
                stmts[i] = Stmt::If(neg, else_b.clone(), None);
                changed = true;
                continue;
            }
            Stmt::If(_, _, Some(else_b)) if else_b.is_empty() => {
                if let Stmt::If(_, _, eb) = &mut stmts[i] {
                    *eb = None;
                }
                changed = true;
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

fn scan_const_usage_stmts(
    stmts: &[Stmt],
    locals: &HashSet<String>,
    usage: &mut HashMap<String, LocalConstUsage>,
) {
    struct Scan<'a> {
        locals: &'a HashSet<String>,
        usage: &'a mut HashMap<String, LocalConstUsage>,
        in_closure: bool,
    }
    impl Visit for Scan<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::Let(n, Some(e)) | Stmt::Const(n, e) => {
                    if self.locals.contains(n) {
                        let u = self.usage.entry(n.clone()).or_default();
                        u.writes += 1;
                        u.in_closure |= self.in_closure;
                    }
                    self.visit_expr(e);
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(e) = init {
                            if self.locals.contains(n) {
                                let u = self.usage.entry(n.clone()).or_default();
                                u.writes += 1;
                                u.in_closure |= self.in_closure;
                            }
                            self.visit_expr(e);
                        }
                    }
                }
                Stmt::FunctionDecl { body, .. } => {
                    let prev = self.in_closure;
                    self.in_closure = true;
                    self.visit_stmts(body);
                    self.in_closure = prev;
                }
                _ => jsast::walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, expr: &Expr) {
            match expr {
                Expr::Ident(n) => {
                    if self.locals.contains(n) {
                        let u = self.usage.entry(n.clone()).or_default();
                        u.reads += 1;
                        u.in_closure |= self.in_closure;
                    }
                }
                Expr::Assign(lhs, rhs) => {
                    if let Expr::Ident(n) = lhs.as_ref() {
                        if self.locals.contains(n) {
                            let u = self.usage.entry(n.clone()).or_default();
                            u.writes += 1;
                            u.in_closure |= self.in_closure;
                        }
                    } else {
                        self.visit_expr(lhs);
                    }
                    self.visit_expr(rhs);
                }
                Expr::CompoundAssign(_, lhs, rhs) => {
                    if let Expr::Ident(n) = lhs.as_ref() {
                        if self.locals.contains(n) {
                            let u = self.usage.entry(n.clone()).or_default();
                            u.writes += 1;
                            u.reads += 1;
                            u.in_closure |= self.in_closure;
                        }
                    } else {
                        self.visit_expr(lhs);
                    }
                    self.visit_expr(rhs);
                }
                Expr::Arrow(_, _) => {
                    let prev = self.in_closure;
                    self.in_closure = true;
                    jsast::walk_expr(self, expr);
                    self.in_closure = prev;
                }
                _ => jsast::walk_expr(self, expr),
            }
        }
    }
    Scan {
        locals,
        usage,
        in_closure: false,
    }
    .visit_stmts(stmts);
}

use super::ast_util::{
    collect_written_idents_expr, collect_written_idents_stmt, collect_written_idents_stmts,
    count_ident_reads_expr, count_ident_reads_stmts, subst_ident_stmts, tally_ident_reads_stmt,
};

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

fn fold_constants_and_switches(
    body: &mut Vec<Stmt>,
    uninit_locals: &HashSet<String>,
    local_names: &HashSet<String>,
) {
    for _ in 0..3 {
        let mut changed = fold_stmt_list(body, local_names);
        if !uninit_locals.is_empty() {
            let mut usage = HashMap::new();
            scan_const_usage_stmts(body, uninit_locals, &mut usage);
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
                    fold_lvalue_expr(lhs);
                }
            }
            Expr::CompoundAssign(_, lhs, rhs) => {
                subst_env_expr(rhs, env);
                fold_expr(rhs);
                if let Expr::Ident(id) = lhs.as_ref() {
                    env.remove(id);
                } else {
                    subst_env_expr(lhs, env);
                    fold_lvalue_expr(lhs);
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
    collect_written_idents_expr(expr, out);
}

fn collect_assigned_idents_stmts(stmts: &[Stmt], out: &mut HashSet<String>) {
    collect_written_idents_stmts(stmts, out);
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
    let (signed, rem) = match callee.as_ref() {
        Expr::Ident(id) if id == i64_pair::HELPER_DIV && args.len() == 6 => {
            let (Expr::Bool(signed), Expr::Bool(rem)) = (&args[4], &args[5]) else {
                return None;
            };
            (*signed, *rem)
        }
        Expr::Ident(id) if id == i64_pair::HELPER_SDIV && args.len() == 4 => (true, false),
        Expr::Ident(id) if id == i64_pair::HELPER_UDIV && args.len() == 4 => (false, false),
        Expr::Ident(id) if id == i64_pair::HELPER_SREM && args.len() == 4 => (true, true),
        Expr::Ident(id) if id == i64_pair::HELPER_UREM && args.len() == 4 => (false, true),
        _ => return None,
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
        signed,
        rem,
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

        let divrem_helper = if first.signed {
            i64_pair::HELPER_SDIVREM
        } else {
            i64_pair::HELPER_UDIVREM
        };
        let divrem_call = jsast::call(
            jsast::id(divrem_helper),
            vec![
                first.al.clone(),
                first.ah.clone(),
                first.bl.clone(),
                first.bh.clone(),
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

// =============================================================================================
// 3b. `ToInt32` / `ToUint32` labeled-block diamond folding (`earcut` zorder)
// =============================================================================================

/// Matches `var | 0` (or `0 | var`) returning `Some((var, false))`, or `var >>> 0` (or
/// `(var | 0) >>> 0`) returning `Some((var, true))`.
fn match_ident_bit32_wrap(expr: &Expr) -> Option<(&str, bool)> {
    match expr {
        Expr::Binary(BinOp::BitOr, l, r) => {
            if exact_i32(r) == Some(0) {
                if let Expr::Ident(id) = l.as_ref() {
                    return Some((id.as_str(), false));
                }
            }
            if exact_i32(l) == Some(0) {
                if let Expr::Ident(id) = r.as_ref() {
                    return Some((id.as_str(), false));
                }
            }
            None
        }
        Expr::Binary(BinOp::UShr, l, r) if exact_i32(r) == Some(0) => match l.as_ref() {
            Expr::Ident(id) => Some((id.as_str(), true)),
            Expr::Binary(BinOp::BitOr, il, ir) => {
                if exact_i32(ir) == Some(0) {
                    if let Expr::Ident(id) = il.as_ref() {
                        return Some((id.as_str(), true));
                    }
                }
                if exact_i32(il) == Some(0) {
                    if let Expr::Ident(id) = ir.as_ref() {
                        return Some((id.as_str(), true));
                    }
                }
                None
            }
            _ => None,
        },
        _ => None,
    }
}

/// Matches `src >= -2147483648 && src <= 2147483647` (in either conjunct/operand order).
fn match_i32_range_guard(expr: &Expr) -> Option<&str> {
    fn match_lo(e: &Expr) -> Option<&str> {
        let Expr::Binary(op, a, b) = e else {
            return None;
        };
        match op {
            BinOp::Ge if exact_f64_lit(b) == Some(-2147483648.0) => match a.as_ref() {
                Expr::Ident(id) => Some(id.as_str()),
                _ => None,
            },
            BinOp::Le if exact_f64_lit(a) == Some(-2147483648.0) => match b.as_ref() {
                Expr::Ident(id) => Some(id.as_str()),
                _ => None,
            },
            _ => None,
        }
    }
    fn match_hi(e: &Expr) -> Option<&str> {
        let Expr::Binary(op, a, b) = e else {
            return None;
        };
        match op {
            BinOp::Le if exact_f64_lit(b) == Some(2147483647.0) => match a.as_ref() {
                Expr::Ident(id) => Some(id.as_str()),
                _ => None,
            },
            BinOp::Ge if exact_f64_lit(a) == Some(2147483647.0) => match b.as_ref() {
                Expr::Ident(id) => Some(id.as_str()),
                _ => None,
            },
            _ => None,
        }
    }
    let Expr::Binary(BinOp::And, l, r) = expr else {
        return None;
    };
    if let (Some(a), Some(b)) = (match_lo(l), match_hi(r)) {
        if a == b {
            return Some(a);
        }
    }
    if let (Some(a), Some(b)) = (match_hi(l), match_lo(r)) {
        if a == b {
            return Some(a);
        }
    }
    None
}

fn match_math_abs_call(expr: &Expr) -> Option<&str> {
    let Expr::Call(callee, args) = expr else {
        return None;
    };
    if args.len() != 1 {
        return None;
    }
    let Expr::Member(obj, prop) = callee.as_ref() else {
        return None;
    };
    if prop != "abs" || !matches!(obj.as_ref(), Expr::Ident(m) if m == "Math") {
        return None;
    }
    match &args[0] {
        Expr::Ident(id) => Some(id.as_str()),
        _ => None,
    }
}

/// Matches `rem_var >= 4294967295 ? 4294967295 : (rem_var > 0 ? rem_var (| 0 | >>> 0) : 0)`.
fn match_u32_clamp_ternary(expr: &Expr, rem_var: &str) -> bool {
    let Expr::Cond(c1, t1, e1) = expr else {
        return false;
    };
    let ge_max = match c1.as_ref() {
        Expr::Binary(BinOp::Ge, a, b) => {
            matches!(a.as_ref(), Expr::Ident(id) if id == rem_var)
                && exact_f64_lit(b) == Some(4294967295.0)
        }
        Expr::Binary(BinOp::Le, a, b) => {
            exact_f64_lit(a) == Some(4294967295.0)
                && matches!(b.as_ref(), Expr::Ident(id) if id == rem_var)
        }
        _ => false,
    };
    if !ge_max || exact_f64_lit(t1) != Some(4294967295.0) {
        return false;
    }
    let Expr::Cond(c2, t2, e2) = e1.as_ref() else {
        return false;
    };
    let gt_zero = match c2.as_ref() {
        Expr::Binary(BinOp::Gt, a, b) => {
            matches!(a.as_ref(), Expr::Ident(id) if id == rem_var) && exact_f64_lit(b) == Some(0.0)
        }
        Expr::Binary(BinOp::Lt, a, b) => {
            exact_f64_lit(a) == Some(0.0) && matches!(b.as_ref(), Expr::Ident(id) if id == rem_var)
        }
        _ => false,
    };
    if !gt_zero || !is_pos_zero(e2) {
        return false;
    }
    matches!(match_ident_bit32_wrap(t2), Some((v, _)) if v == rem_var)
}

/// Matches `(0 - mag) | 0`, `(-mag) | 0`, `(0 - mag) >>> 0`, or `(-mag) >>> 0`.
/// Returns `Some(is_unsigned)` on match.
fn match_neg_bit32(expr: &Expr, mag_var: &str) -> Option<bool> {
    let (inner, unsigned) = match expr {
        Expr::Binary(BinOp::BitOr, l, r) if exact_i32(r) == Some(0) => (l.as_ref(), false),
        Expr::Binary(BinOp::BitOr, l, r) if exact_i32(l) == Some(0) => (r.as_ref(), false),
        Expr::Binary(BinOp::UShr, l, r) if exact_i32(r) == Some(0) => (l.as_ref(), true),
        _ => return None,
    };
    let is_neg_mag = match inner {
        Expr::Unary(UnOp::Neg, x) => matches!(x.as_ref(), Expr::Ident(id) if id == mag_var),
        Expr::Binary(BinOp::Sub, z, x) => {
            is_pos_zero(z) && matches!(x.as_ref(), Expr::Ident(id) if id == mag_var)
        }
        _ => false,
    };
    is_neg_mag.then_some(unsigned)
}

fn match_lt_zero(expr: &Expr, src: &str) -> bool {
    match expr {
        Expr::Binary(BinOp::Lt, a, b) => {
            matches!(a.as_ref(), Expr::Ident(id) if id == src) && exact_f64_lit(b) == Some(0.0)
        }
        Expr::Binary(BinOp::Gt, a, b) => {
            exact_f64_lit(a) == Some(0.0) && matches!(b.as_ref(), Expr::Ident(id) if id == src)
        }
        _ => false,
    }
}

/// Recognizes a Rust `ToInt32`/`ToUint32` labeled-block diamond:
/// ```js
/// d: {
///   if (m >= -2147483648 && m <= 2147483647) { k = m | 0; break d; }
///   j = Math.abs(m);
///   k = (j = j % 4294967296, j >= 4294967295 ? 4294967295 : j > 0 ? j | 0 : 0);
///   if (m < 0) { m = -k | 0; m = m | 0; } else m = k | 0;
///   k = m;
/// }
/// ```
/// Because `4294967296 = 2^32` is an exact power of two in IEEE-754 binary64, this diamond is
/// bit-identical to `k = m | 0` (or `k = m >>> 0` when both fast and slow paths emit `>>> 0`) for
/// every IEEE-754 `f64` value (NaN, `±0`, `±Infinity`, subnormals, large floats, integers).
/// Returns `(dst, src, unsigned_dst)` if the labeled block matches structurally.
fn match_to_int32_diamond(stmt: &Stmt) -> Option<(String, String, bool)> {
    let Stmt::Labeled(label, inner) = stmt else {
        return None;
    };
    let Stmt::Block(block) = inner.as_ref() else {
        return None;
    };
    if block.len() < 3 || block.len() > 6 {
        return None;
    }

    // 1. Fast path: `if (src >= -2147483648 && src <= 2147483647) { dst = src (| 0 | >>> 0); break label; }`
    let Stmt::If(guard, fast_then, None) = &block[0] else {
        return None;
    };
    let src = match_i32_range_guard(guard)?.to_owned();
    if fast_then.len() != 2 {
        return None;
    }
    let Stmt::ExprStmt(Expr::Assign(fast_lhs, fast_rhs)) = &fast_then[0] else {
        return None;
    };
    let Expr::Ident(dst) = fast_lhs.as_ref() else {
        return None;
    };
    let (wrap_src, fast_unsigned) = match_ident_bit32_wrap(fast_rhs)?;
    if wrap_src != src {
        return None;
    }
    if !matches!(&fast_then[1], Stmt::Break(Some(b)) if b == label) {
        return None;
    }

    // 2. Magnitude reduction: optional `abs_var = Math.abs(src);`, `% 4294967296`, and u32 clamp.
    let mut idx = 1usize;
    let mut abs_var: Option<String> = None;
    if let Some(Stmt::ExprStmt(Expr::Assign(lhs, rhs))) = block.get(idx) {
        if let (Expr::Ident(a_name), Some(arg)) = (lhs.as_ref(), match_math_abs_call(rhs)) {
            if arg == src && a_name != &src {
                abs_var = Some(a_name.clone());
                idx += 1;
            }
        }
    }

    let is_mod_2p32 = |e: &Expr| -> bool {
        let Expr::Binary(BinOp::Rem, l, r) = e else {
            return false;
        };
        if exact_f64_lit(r) != Some(4294967296.0) {
            return false;
        }
        match (&abs_var, l.as_ref()) {
            (Some(a), Expr::Ident(id)) => id == a,
            (None, other) => match_math_abs_call(other) == Some(src.as_str()),
            _ => false,
        }
    };

    // Match either:
    //   `mag_var = (rem_var = <abs> % 4294967296, clamp(rem_var));`
    // or separate statements:
    //   `rem_var = <abs> % 4294967296; mag_var = clamp(rem_var);`
    let mag_var: String = match block.get(idx)? {
        Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
            let Expr::Ident(lhs_name) = lhs.as_ref() else {
                return None;
            };
            if let Expr::Seq(items) = rhs.as_ref() {
                if items.len() != 2 {
                    return None;
                }
                let Expr::Assign(rl, rr) = &items[0] else {
                    return None;
                };
                let Expr::Ident(rem_name) = rl.as_ref() else {
                    return None;
                };
                if rem_name == &src || lhs_name == &src || !is_mod_2p32(rr) {
                    return None;
                }
                if !match_u32_clamp_ternary(&items[1], rem_name) {
                    return None;
                }
                idx += 1;
                lhs_name.clone()
            } else if is_mod_2p32(rhs) {
                let rem_name = lhs_name.clone();
                if rem_name == src {
                    return None;
                }
                idx += 1;
                let Stmt::ExprStmt(Expr::Assign(ml, mr)) = block.get(idx)? else {
                    return None;
                };
                let Expr::Ident(m_name) = ml.as_ref() else {
                    return None;
                };
                if m_name == &src || !match_u32_clamp_ternary(mr, &rem_name) {
                    return None;
                }
                idx += 1;
                m_name.clone()
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // 3. Sign application: `if (src < 0) { ... } else { res = mag | 0; }`
    let Stmt::If(sign_cond, neg_b, Some(pos_b)) = block.get(idx)? else {
        return None;
    };
    if !match_lt_zero(sign_cond, &src) || pos_b.len() != 1 {
        return None;
    }
    let Stmt::ExprStmt(Expr::Assign(pl, pr)) = &pos_b[0] else {
        return None;
    };
    let Expr::Ident(res_var) = pl.as_ref() else {
        return None;
    };
    let (pos_mag, pos_unsigned) = match_ident_bit32_wrap(pr)?;
    if pos_mag != mag_var {
        return None;
    }

    let neg_unsigned = match neg_b.as_slice() {
        [Stmt::ExprStmt(Expr::Assign(nl, nr))] => {
            let Expr::Ident(n_dst) = nl.as_ref() else {
                return None;
            };
            if n_dst != res_var {
                return None;
            }
            match_neg_bit32(nr, &mag_var)?
        }
        [Stmt::ExprStmt(Expr::Assign(n1l, n1r)), Stmt::ExprStmt(Expr::Assign(n2l, n2r))] => {
            let (Expr::Ident(neg_tmp), Expr::Ident(n2_dst)) = (n1l.as_ref(), n2l.as_ref()) else {
                return None;
            };
            if n2_dst != res_var {
                return None;
            }
            let first_unsigned = match_neg_bit32(n1r, &mag_var)?;
            match n2r.as_ref() {
                Expr::Ident(v) if v == neg_tmp => first_unsigned,
                other => {
                    let (v, u) = match_ident_bit32_wrap(other)?;
                    if v != neg_tmp {
                        return None;
                    }
                    u
                }
            }
        }
        _ => return None,
    };
    idx += 1;

    // 4. Final assignment to `dst` (optional if `res_var == dst` with matching signedness).
    if idx == block.len() {
        if res_var == dst && neg_unsigned == fast_unsigned && pos_unsigned == fast_unsigned {
            return Some((dst.clone(), src, fast_unsigned));
        }
        return None;
    }
    if idx + 1 != block.len() {
        return None;
    }
    let Stmt::ExprStmt(Expr::Assign(fl, fr)) = &block[idx] else {
        return None;
    };
    if !matches!(fl.as_ref(), Expr::Ident(id) if id == dst) {
        return None;
    }
    let final_ok = match fr.as_ref() {
        Expr::Ident(v) => {
            v == res_var && neg_unsigned == fast_unsigned && pos_unsigned == fast_unsigned
        }
        other => {
            matches!(match_ident_bit32_wrap(other), Some((v, u)) if v == res_var && u == fast_unsigned)
        }
    };
    final_ok.then_some((dst.clone(), src, fast_unsigned))
}

fn stmt_may_throw(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Throw(_) => true,
        Stmt::ExprStmt(e) | Stmt::Return(Some(e)) | Stmt::Let(_, Some(e)) | Stmt::Const(_, e) => {
            loop_opt::expr_has_side_effect(e)
        }
        Stmt::Lets(decls) => decls
            .iter()
            .filter_map(|(_, init)| init.as_ref())
            .any(loop_opt::expr_has_side_effect),
        Stmt::If(c, t, e) => {
            loop_opt::expr_has_side_effect(c)
                || t.iter().any(stmt_may_throw)
                || e.as_ref().is_some_and(|eb| eb.iter().any(stmt_may_throw))
        }
        Stmt::Block(b) | Stmt::LoopForever(_, b) => b.iter().any(stmt_may_throw),
        Stmt::While(c, b) => loop_opt::expr_has_side_effect(c) || b.iter().any(stmt_may_throw),
        Stmt::Labeled(_, inner) => stmt_may_throw(inner),
        Stmt::Switch(..) | Stmt::TryCatch { .. } => true,
        _ => false,
    }
}

/// Returns `true` iff `var` is guaranteed to be overwritten (or the function exits) before any
/// subsequent read along `trailing`.
fn is_definitely_dead_in_trailing(
    var: &str,
    trailing: &[Stmt],
    at_fn_top_level: bool,
    in_try: bool,
) -> bool {
    for stmt in trailing {
        if let Stmt::ExprStmt(Expr::Assign(lhs, rhs)) = stmt {
            if count_ident_reads_expr(rhs, var) > 0 {
                return false;
            }
            if matches!(lhs.as_ref(), Expr::Ident(id) if id == var) {
                return true;
            }
            if count_ident_reads_expr(lhs, var) > 0 {
                return false;
            }
            if in_try
                && (loop_opt::expr_has_side_effect(lhs) || loop_opt::expr_has_side_effect(rhs))
            {
                return false;
            }
            continue;
        }
        if let Stmt::Return(opt_e) = stmt {
            return opt_e
                .as_ref()
                .is_none_or(|e| count_ident_reads_expr(e, var) == 0);
        }
        if let Stmt::Throw(e) = stmt {
            if in_try {
                return false;
            }
            return count_ident_reads_expr(e, var) == 0;
        }
        if count_ident_reads_stmts(std::slice::from_ref(stmt), var) > 0
            || stmt_may_exit_non_throw(stmt)
            || (in_try && stmt_may_throw(stmt))
        {
            return false;
        }
    }
    at_fn_top_level && !in_try
}

fn fold_to_int32_diamonds_stmts(
    stmts: &mut [Stmt],
    local_names: &HashSet<String>,
    total_reads: &mut HashMap<String, usize>,
    closure_vars: &HashSet<String>,
    at_fn_top_level: bool,
    in_try: bool,
) {
    // Recurse into nested control flow first.
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::If(_, t, e) => {
                fold_to_int32_diamonds_stmts(
                    t,
                    local_names,
                    total_reads,
                    closure_vars,
                    false,
                    in_try,
                );
                if let Some(el) = e {
                    fold_to_int32_diamonds_stmts(
                        el,
                        local_names,
                        total_reads,
                        closure_vars,
                        false,
                        in_try,
                    );
                }
            }
            Stmt::Block(b) => {
                fold_to_int32_diamonds_stmts(
                    b,
                    local_names,
                    total_reads,
                    closure_vars,
                    at_fn_top_level,
                    in_try,
                );
            }
            Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
                fold_to_int32_diamonds_stmts(
                    b,
                    local_names,
                    total_reads,
                    closure_vars,
                    false,
                    in_try,
                );
            }
            Stmt::Labeled(_, inner) => {
                if let Stmt::Block(b) | Stmt::LoopForever(_, b) | Stmt::While(_, b) = inner.as_mut()
                {
                    fold_to_int32_diamonds_stmts(
                        b,
                        local_names,
                        total_reads,
                        closure_vars,
                        false,
                        in_try,
                    );
                }
            }
            Stmt::Switch(_, cases) => {
                for c in cases {
                    fold_to_int32_diamonds_stmts(
                        &mut c.body,
                        local_names,
                        total_reads,
                        closure_vars,
                        false,
                        in_try,
                    );
                }
            }
            Stmt::TryCatch { body, handler, .. } => {
                fold_to_int32_diamonds_stmts(
                    body,
                    local_names,
                    total_reads,
                    closure_vars,
                    false,
                    true,
                );
                fold_to_int32_diamonds_stmts(
                    handler,
                    local_names,
                    total_reads,
                    closure_vars,
                    false,
                    in_try,
                );
            }
            _ => {}
        }
    }

    // Walk back-to-front so folding a later diamond removes its slow-path reads of reused scratch
    // locals before checking whether an earlier diamond's clobbered scratch locals are dead.
    for i in (0..stmts.len()).rev() {
        let Some((dst, src, unsigned_dst)) = match_to_int32_diamond(&stmts[i]) else {
            continue;
        };
        let Stmt::Labeled(_, inner) = &stmts[i] else {
            continue;
        };
        let Stmt::Block(block) = inner.as_ref() else {
            continue;
        };
        let mut slow_writes: HashSet<String> = HashSet::new();
        collect_assigned_idents_stmts(&block[1..], &mut slow_writes);
        slow_writes.remove(&dst);

        let mut block_reads: HashMap<String, usize> = HashMap::new();
        tally_ident_reads_stmt(&stmts[i], &mut block_reads);

        let trailing = &stmts[i + 1..];
        let all_dead = slow_writes.iter().all(|v| {
            if !local_names.contains(v) || closure_vars.contains(v) {
                return false;
            }
            let tot = total_reads.get(v).copied().unwrap_or(0);
            let blk = block_reads.get(v).copied().unwrap_or(0);
            if tot <= blk {
                return true;
            }
            is_definitely_dead_in_trailing(v, trailing, at_fn_top_level, in_try)
        });
        if !all_dead {
            continue;
        }

        // Update read counts to reflect replacing the diamond with `dst = src (| 0 | >>> 0);`.
        for (var, cnt) in block_reads {
            if let Some(entry) = total_reads.get_mut(&var) {
                *entry = entry.saturating_sub(cnt);
            }
        }
        *total_reads.entry(src.clone()).or_insert(0) += 1;

        let op = if unsigned_dst {
            BinOp::UShr
        } else {
            BinOp::BitOr
        };
        stmts[i] = jsast::assign_stmt(
            jsast::id(dst),
            Expr::Binary(op, Box::new(jsast::id(src)), Box::new(Expr::Num(0.0))),
        );
    }
}

fn fold_to_int32_diamonds(body: &mut [Stmt], local_names: &HashSet<String>) {
    let mut total_reads: HashMap<String, usize> = HashMap::new();
    for stmt in body.iter() {
        tally_ident_reads_stmt(stmt, &mut total_reads);
    }
    let mut usage: HashMap<String, LocalConstUsage> = HashMap::new();
    scan_const_usage_stmts(body, local_names, &mut usage);
    let closure_vars: HashSet<String> = usage
        .into_iter()
        .filter(|(_, u)| u.in_closure)
        .map(|(n, _)| n)
        .collect();
    fold_to_int32_diamonds_stmts(
        body,
        local_names,
        &mut total_reads,
        &closure_vars,
        true,
        false,
    );
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
    struct V<'a> {
        target: &'a Expr,
        count: usize,
    }
    impl jsast::Visit for V<'_> {
        fn visit_expr(&mut self, expr: &Expr) {
            if expr == self.target {
                self.count += 1;
                return;
            }
            if !matches!(expr, Expr::Arrow(..)) {
                jsast::walk_expr(self, expr);
            }
        }
    }
    let mut v = V { target, count: 0 };
    v.visit_expr(expr);
    v.count
}

fn count_expr_eq_in_stmt(stmt: &Stmt, target: &Expr) -> usize {
    struct V<'a> {
        target: &'a Expr,
        count: usize,
    }
    impl jsast::Visit for V<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::ExprStmt(_)
                | Stmt::Return(Some(_))
                | Stmt::Throw(_)
                | Stmt::If(..)
                | Stmt::Block(_)
                | Stmt::Labeled(..)
                | Stmt::Switch(..) => jsast::walk_stmt(self, stmt),
                _ => {}
            }
        }
        fn visit_expr(&mut self, expr: &Expr) {
            if expr == self.target {
                self.count += 1;
                return;
            }
            if !matches!(expr, Expr::Arrow(..)) {
                jsast::walk_expr(self, expr);
            }
        }
    }
    let mut v = V { target, count: 0 };
    v.visit_stmt(stmt);
    v.count
}

fn replace_expr_eq_in_expr(expr: &mut Expr, target: &Expr, repl: &Expr) {
    struct V<'a> {
        target: &'a Expr,
        repl: &'a Expr,
    }
    impl jsast::VisitMut for V<'_> {
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            if expr == self.target {
                *expr = self.repl.clone();
                return;
            }
            if !matches!(expr, Expr::Arrow(..)) {
                jsast::walk_expr_mut(self, expr);
            }
        }
    }
    V { target, repl }.visit_expr_mut(expr);
}

fn replace_expr_eq_in_stmt(stmt: &mut Stmt, target: &Expr, repl: &Expr) {
    struct V<'a> {
        target: &'a Expr,
        repl: &'a Expr,
    }
    impl jsast::VisitMut for V<'_> {
        fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
            match stmt {
                Stmt::ExprStmt(_)
                | Stmt::Return(Some(_))
                | Stmt::Throw(_)
                | Stmt::If(..)
                | Stmt::Block(_)
                | Stmt::Labeled(..)
                | Stmt::Switch(..) => jsast::walk_stmt_mut(self, stmt),
                _ => {}
            }
        }
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            if expr == self.target {
                *expr = self.repl.clone();
                return;
            }
            if !matches!(expr, Expr::Arrow(..)) {
                jsast::walk_expr_mut(self, expr);
            }
        }
    }
    V { target, repl }.visit_stmt_mut(stmt);
}

// =============================================================================================
// 4. Flow-sensitive numeric/integer domain tracking (shared by range folding & ASCII LUTs)
// =============================================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NumKind {
    /// Exact integer in `[0, 0x7fff_ffff]` (never `NaN`, fractional, negative, or `>= 2^31`).
    NonNegI31,
    /// Exact integer in `[0, 0xffff_ffff]` (`Uint32`, never `NaN` or fractional).
    U32,
    /// Exact integer in `[-2^31, 2^31 - 1]` (`Int32`, never `NaN` or fractional).
    I32,
    /// Value in `{0..=0x7fff_ffff} ∪ {NaN}` (e.g. `s.charCodeAt(idx)` without locally proven bounds).
    NonNegI31OrNan,
    /// Guaranteed JS `number` (may be float / `NaN` / `Infinity`, NEVER string/object/bool).
    Numeric,
}

impl NumKind {
    fn join(self, other: Self) -> Self {
        if self == other {
            return self;
        }
        match (self, other) {
            (Self::NonNegI31, Self::U32) | (Self::U32, Self::NonNegI31) => Self::U32,
            (Self::NonNegI31, Self::I32) | (Self::I32, Self::NonNegI31) => Self::I32,
            (Self::NonNegI31, Self::NonNegI31OrNan) | (Self::NonNegI31OrNan, Self::NonNegI31) => {
                Self::NonNegI31OrNan
            }
            _ => Self::Numeric,
        }
    }

    fn is_exact_int32_or_u32(self) -> bool {
        matches!(self, Self::NonNegI31 | Self::U32 | Self::I32)
    }

    fn is_nonneg_int(self) -> bool {
        matches!(self, Self::NonNegI31 | Self::U32)
    }
}

#[derive(Clone, Default)]
struct NumEnv {
    vars: HashMap<String, NumKind>,
    /// `len_var -> str_var` when `len_var = str_var.length`.
    str_lens: HashMap<String, String>,
    /// Pairs `(idx_var, str_var)` proven `0 <= idx_var < str_var.length`.
    in_bounds: HashSet<(String, String)>,
    /// String vars proven `s.length > 0`.
    nonempty_strs: HashSet<String>,
}

impl NumEnv {
    fn invalidate(&mut self, var: &str) {
        self.vars.remove(var);
        self.str_lens.remove(var);
        self.str_lens.retain(|_, s| s != var);
        self.in_bounds.retain(|(i, s)| i != var && s != var);
        self.nonempty_strs.remove(var);
    }

    fn is_nonneg_int(&self, var: &str) -> bool {
        self.vars.get(var).is_some_and(|k| k.is_nonneg_int())
    }

    fn is_nonneg_i31_expr(&self, expr: &Expr) -> bool {
        self.classify_expr(expr) == Some(NumKind::NonNegI31)
    }

    fn resolve_str_len(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Member(obj, prop) if prop == "length" => match obj.as_ref() {
                Expr::Ident(s) => Some(s.clone()),
                _ => None,
            },
            Expr::Ident(d) => self.str_lens.get(d).cloned(),
            _ => None,
        }
    }

    fn is_char_code_at_in_bounds(&self, recv: &Expr, idx: &Expr) -> bool {
        match recv {
            Expr::Str(lit) => {
                exact_u32(idx).is_some_and(|k| (k as usize) < lit.encode_utf16().count())
            }
            Expr::Ident(s) => {
                if is_pos_zero(idx) && self.nonempty_strs.contains(s) {
                    return true;
                }
                if let Expr::Ident(iv) = idx {
                    if self.in_bounds.contains(&(iv.clone(), s.clone())) {
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    fn classify_expr(&self, expr: &Expr) -> Option<NumKind> {
        match expr {
            Expr::Ident(id) => self.vars.get(id).copied(),
            Expr::Num(n) => {
                if n.is_finite() && n.fract() == 0.0 {
                    if *n >= 0.0 && *n <= 2147483647.0 {
                        Some(NumKind::NonNegI31)
                    } else if *n >= 0.0 && *n <= 4294967295.0 {
                        Some(NumKind::U32)
                    } else if *n >= -2147483648.0 && *n <= 2147483647.0 {
                        Some(NumKind::I32)
                    } else {
                        Some(NumKind::Numeric)
                    }
                } else {
                    Some(NumKind::Numeric)
                }
            }
            Expr::RawNum(s) => {
                if let Ok(v) = s.parse::<i64>() {
                    if (0..=0x7fff_ffff).contains(&v) {
                        Some(NumKind::NonNegI31)
                    } else if (0..=0xffff_ffff).contains(&v) {
                        Some(NumKind::U32)
                    } else if (-0x8000_0000..=0x7fff_ffff).contains(&v) {
                        Some(NumKind::I32)
                    } else {
                        Some(NumKind::Numeric)
                    }
                } else {
                    Some(NumKind::Numeric)
                }
            }
            Expr::Unary(UnOp::BitNot, _) => Some(NumKind::I32),
            Expr::Unary(UnOp::Neg, _) => {
                if let Some(v) = exact_i32(expr) {
                    Some(if v >= 0 {
                        NumKind::NonNegI31
                    } else {
                        NumKind::I32
                    })
                } else {
                    Some(NumKind::Numeric)
                }
            }
            Expr::Unary(UnOp::Pos, _) => Some(NumKind::Numeric),
            Expr::Binary(BinOp::UShr, _, r) => {
                if exact_u32(r).is_some_and(|k| (k & 31) >= 1) {
                    Some(NumKind::NonNegI31)
                } else {
                    Some(NumKind::U32)
                }
            }
            Expr::Binary(BinOp::BitAnd, l, r) => {
                if self.is_nonneg_i31_expr(l) || self.is_nonneg_i31_expr(r) {
                    Some(NumKind::NonNegI31)
                } else {
                    Some(NumKind::I32)
                }
            }
            Expr::Binary(BinOp::BitOr | BinOp::BitXor, l, r) => {
                if self.is_nonneg_i31_expr(l) && self.is_nonneg_i31_expr(r) {
                    Some(NumKind::NonNegI31)
                } else {
                    Some(NumKind::I32)
                }
            }
            Expr::Binary(BinOp::Shl | BinOp::Shr, _, _) => Some(NumKind::I32),
            Expr::Binary(BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem, _, _) => {
                Some(NumKind::Numeric)
            }
            Expr::Binary(BinOp::Add, l, r) => {
                if self.classify_expr(l).is_some() && self.classify_expr(r).is_some() {
                    Some(NumKind::Numeric)
                } else {
                    None
                }
            }
            Expr::Call(callee, args) => match callee.as_ref() {
                Expr::Member(obj, prop) => {
                    if matches!(obj.as_ref(), Expr::Ident(id) if id == "Math") {
                        if prop == "clz32" && args.len() == 1 {
                            return Some(NumKind::NonNegI31);
                        }
                        if prop == "imul" && args.len() == 2 {
                            return Some(NumKind::I32);
                        }
                    }
                    if prop == "charCodeAt"
                        && args.len() == 1
                        && matches!(obj.as_ref(), Expr::Ident(_) | Expr::Str(_))
                    {
                        return Some(if self.is_char_code_at_in_bounds(obj, &args[0]) {
                            NumKind::NonNegI31
                        } else {
                            NumKind::NonNegI31OrNan
                        });
                    }
                    None
                }
                _ => None,
            },
            Expr::Cond(_, t, e) => match (self.classify_expr(t), self.classify_expr(e)) {
                (Some(a), Some(b)) => Some(a.join(b)),
                _ => None,
            },
            _ => None,
        }
    }

    fn update_assign(&mut self, dst: &str, rhs: &Expr) {
        let kind = self.classify_expr(rhs);
        let str_len_src = match rhs {
            Expr::Member(obj, prop) if prop == "length" => match obj.as_ref() {
                Expr::Ident(s) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        };
        self.invalidate(dst);
        if let Some(k) = kind {
            self.vars.insert(dst.to_owned(), k);
        }
        if let Some(s) = str_len_src {
            if s != dst {
                self.str_lens.insert(dst.to_owned(), s);
            }
        }
    }

    fn apply_cond_facts(&mut self, cond: &Expr, truth: bool) {
        match (cond, truth) {
            (Expr::Unary(UnOp::Not, inner), t) => self.apply_cond_facts(inner, !t),
            (Expr::Binary(BinOp::And, l, r), true) => {
                self.apply_cond_facts(l, true);
                self.apply_cond_facts(r, true);
            }
            (Expr::Binary(BinOp::Or, l, r), false) => {
                self.apply_cond_facts(l, false);
                self.apply_cond_facts(r, false);
            }
            (Expr::Binary(op, l, r), true) => {
                // Any numeric relational/equality check succeeding proves the operand is not `NaN`.
                if matches!(
                    op,
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::StrictEq
                ) {
                    if let Expr::Ident(v) = l.as_ref() {
                        if self.vars.get(v) == Some(&NumKind::NonNegI31OrNan)
                            && (exact_f64_lit(r).is_some_and(|n| !n.is_nan())
                                || matches!(r.as_ref(), Expr::Ident(v2) if v2 == v))
                        {
                            self.vars.insert(v.clone(), NumKind::NonNegI31);
                        }
                    }
                    if let Expr::Ident(v) = r.as_ref() {
                        if self.vars.get(v) == Some(&NumKind::NonNegI31OrNan)
                            && exact_f64_lit(l).is_some_and(|n| !n.is_nan())
                        {
                            self.vars.insert(v.clone(), NumKind::NonNegI31);
                        }
                    }
                }
                match op {
                    BinOp::Lt => {
                        if let Some(s) = self.resolve_str_len(r) {
                            if let Expr::Ident(iv) = l.as_ref() {
                                if self.is_nonneg_int(iv) {
                                    self.in_bounds.insert((iv.clone(), s));
                                }
                            } else if is_pos_zero(l) {
                                self.nonempty_strs.insert(s);
                            }
                        }
                    }
                    BinOp::Gt => {
                        if let Some(s) = self.resolve_str_len(l) {
                            if let Expr::Ident(iv) = r.as_ref() {
                                if self.is_nonneg_int(iv) {
                                    self.in_bounds.insert((iv.clone(), s));
                                }
                            } else if is_pos_zero(r) {
                                self.nonempty_strs.insert(s);
                            }
                        }
                    }
                    BinOp::StrictNe => {
                        if let Some(s) = self.resolve_str_len(l) {
                            if is_pos_zero(r) {
                                self.nonempty_strs.insert(s);
                            }
                        } else if let Some(s) = self.resolve_str_len(r) {
                            if is_pos_zero(l) {
                                self.nonempty_strs.insert(s);
                            }
                        }
                    }
                    _ => {}
                }
            }
            (Expr::Binary(op, l, r), false) => match op {
                BinOp::Ge => {
                    if let Some(s) = self.resolve_str_len(r) {
                        if let Expr::Ident(iv) = l.as_ref() {
                            if self.is_nonneg_int(iv) {
                                self.in_bounds.insert((iv.clone(), s));
                            }
                        } else if is_pos_zero(l) {
                            self.nonempty_strs.insert(s);
                        }
                    }
                }
                BinOp::Le => {
                    if let Some(s) = self.resolve_str_len(l) {
                        if let Expr::Ident(iv) = r.as_ref() {
                            if self.is_nonneg_int(iv) {
                                self.in_bounds.insert((iv.clone(), s));
                            }
                        } else if is_pos_zero(r) {
                            self.nonempty_strs.insert(s);
                        }
                    }
                }
                BinOp::StrictEq => {
                    if let Some(s) = self.resolve_str_len(l) {
                        if is_pos_zero(r) {
                            self.nonempty_strs.insert(s);
                        }
                    } else if let Some(s) = self.resolve_str_len(r) {
                        if is_pos_zero(l) {
                            self.nonempty_strs.insert(s);
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn join_with(&mut self, other: &Self) {
        let mut next_vars = HashMap::new();
        for (k, &v1) in &self.vars {
            if let Some(&v2) = other.vars.get(k) {
                next_vars.insert(k.clone(), v1.join(v2));
            }
        }
        self.vars = next_vars;
        self.str_lens
            .retain(|k, v| other.str_lens.get(k) == Some(v));
        self.in_bounds.retain(|p| other.in_bounds.contains(p));
        self.nonempty_strs
            .retain(|s| other.nonempty_strs.contains(s));
    }

    /// Computes a sound loop-entry `NumEnv` by invalidating all variables written inside the loop
    /// except those whose every in-loop assignment stays within a provable `NumKind` fixed point
    /// (e.g. counted index `m = m + 1 >>> 0` staying `U32`).
    fn prepare_for_loop(&self, cond: Option<&Expr>, body: &[Stmt]) -> Self {
        let mut written = HashSet::new();
        if let Some(c) = cond {
            collect_written_idents_expr(c, &mut written);
        }
        for s in body {
            collect_written_idents_stmt(s, &mut written);
        }
        let mut loop_env = self.clone();
        for w in &written {
            loop_env.str_lens.remove(w);
            loop_env.str_lens.retain(|_, s| s != w);
            loop_env.in_bounds.retain(|(i, s)| i != w && s != w);
            loop_env.nonempty_strs.remove(w);
        }

        // Collect all simple `v = rhs` assignments inside the loop for each written variable,
        // disqualifying any variable with compound assignments, local redeclarations, or writes
        // inside closures/nested functions.
        struct LoopWriteScan {
            in_closure: bool,
            rhs_map: HashMap<String, Vec<Expr>>,
            disq: HashSet<String>,
        }
        impl Visit for LoopWriteScan {
            fn visit_stmt(&mut self, stmt: &Stmt) {
                match stmt {
                    Stmt::Let(n, _) | Stmt::Const(n, _) => {
                        self.disq.insert(n.clone());
                        jsast::walk_stmt(self, stmt);
                    }
                    Stmt::Lets(decls) => {
                        for (n, _) in decls {
                            self.disq.insert(n.clone());
                        }
                        jsast::walk_stmt(self, stmt);
                    }
                    Stmt::TryCatch { binding, .. } => {
                        self.disq.insert(binding.clone());
                        jsast::walk_stmt(self, stmt);
                    }
                    Stmt::FunctionDecl { name, body, .. } => {
                        self.disq.insert(name.clone());
                        let prev = self.in_closure;
                        self.in_closure = true;
                        self.visit_stmts(body);
                        self.in_closure = prev;
                    }
                    _ => jsast::walk_stmt(self, stmt),
                }
            }
            fn visit_expr(&mut self, expr: &Expr) {
                match expr {
                    Expr::Assign(lhs, rhs) => {
                        if let Expr::Ident(id) = lhs.as_ref() {
                            if self.in_closure {
                                self.disq.insert(id.clone());
                            } else {
                                self.rhs_map
                                    .entry(id.clone())
                                    .or_default()
                                    .push((**rhs).clone());
                            }
                        } else {
                            self.visit_expr(lhs);
                        }
                        self.visit_expr(rhs);
                    }
                    Expr::CompoundAssign(_, lhs, rhs) => {
                        if let Expr::Ident(id) = lhs.as_ref() {
                            self.disq.insert(id.clone());
                        } else {
                            self.visit_expr(lhs);
                        }
                        self.visit_expr(rhs);
                    }
                    Expr::Arrow(_, _) => {
                        let prev = self.in_closure;
                        self.in_closure = true;
                        jsast::walk_expr(self, expr);
                        self.in_closure = prev;
                    }
                    _ => jsast::walk_expr(self, expr),
                }
            }
        }
        let mut scan = LoopWriteScan {
            in_closure: false,
            rhs_map: HashMap::new(),
            disq: HashSet::new(),
        };
        if let Some(c) = cond {
            scan.visit_expr(c);
        }
        scan.visit_stmts(body);
        let LoopWriteScan { rhs_map, disq, .. } = scan;

        // Fixed point over written vars that already had a `NumKind` on loop entry.
        for w in &written {
            if disq.contains(w) || !loop_env.vars.contains_key(w) {
                loop_env.vars.remove(w);
            }
        }
        for _ in 0..4 {
            let mut changed = false;
            let keys: Vec<String> = written
                .iter()
                .filter(|w| loop_env.vars.contains_key(*w))
                .cloned()
                .collect();
            for v in keys {
                let mut joined = loop_env.vars[&v];
                let mut ok = true;
                if let Some(rhss) = rhs_map.get(&v) {
                    for rhs in rhss {
                        if let Some(rk) = loop_env.classify_expr(rhs) {
                            joined = joined.join(rk);
                        } else {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    loop_env.vars.remove(&v);
                    changed = true;
                } else if loop_env.vars[&v] != joined {
                    loop_env.vars.insert(v, joined);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        loop_env
    }
}

/// Returns `true` iff `stmts` is guaranteed never to fall through past the end of `stmts`.
fn stmts_definitely_exit(stmts: &[Stmt]) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Return(_) | Stmt::Throw(_) | Stmt::Break(_) | Stmt::Continue(_) => return true,
            Stmt::ExprStmt(Expr::Call(callee, _))
                if matches!(
                    callee.as_ref(),
                    Expr::Ident(name) if name.starts_with("core$panicking$")
                ) =>
            {
                return true;
            }
            Stmt::If(_, then_b, Some(else_b))
                if stmts_definitely_exit(then_b) && stmts_definitely_exit(else_b) =>
            {
                return true;
            }
            Stmt::Block(b) if stmts_definitely_exit(b) => return true,
            Stmt::Switch(_, cases) => {
                // An unlabeled `switch` definitely exits past itself iff it has a `default:` arm,
                // no arm contains an unlabeled `break;` targeting this `switch`, and every arm
                // either falls through (empty body) into an exiting arm or exits directly (via
                // `return`, `throw`, `continue`, or a labeled `break` to an enclosing label).
                let has_default = cases.iter().any(|c| c.tests.is_empty());
                if has_default
                    && cases.last().is_some_and(|c| !c.body.is_empty())
                    && cases.iter().all(|c| !has_unlabeled_switch_break(&c.body))
                    && cases
                        .iter()
                        .all(|c| c.body.is_empty() || stmts_definitely_exit(&c.body))
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Returns `true` iff `expr` is a pure, side-effect-free integer/charCodeAt expression safe to
/// duplicate or fold from 2 evaluations down to 1.
fn is_pure_range_operand(expr: &Expr) -> bool {
    match expr {
        Expr::Ident(_) => true,
        Expr::Unary(UnOp::BitNot, inner) => is_pure_index_expr(inner),
        Expr::Binary(
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr | BinOp::UShr,
            l,
            r,
        ) => is_pure_index_expr(l) && is_pure_index_expr(r),
        Expr::Call(callee, args) => match callee.as_ref() {
            Expr::Member(obj, prop) => {
                if prop == "charCodeAt"
                    && args.len() == 1
                    && matches!(obj.as_ref(), Expr::Ident(_) | Expr::Str(_))
                {
                    return is_pure_index_expr(&args[0]);
                }
                if matches!(obj.as_ref(), Expr::Ident(id) if id == "Math") {
                    if prop == "clz32" && args.len() == 1 {
                        return is_pure_index_expr(&args[0]);
                    }
                    if prop == "imul" && args.len() == 2 {
                        return is_pure_index_expr(&args[0]) && is_pure_index_expr(&args[1]);
                    }
                }
                false
            }
            _ => false,
        },
        _ => false,
    }
}

// =============================================================================================
// 5. Part 1: Unsigned Range Check Folding `(x >= A && x <= B)` -> `(x - A >>> 0) <= (B - A)`
// =============================================================================================

#[derive(Clone, Copy, Debug)]
enum RangeBound {
    /// Inclusive lower bound `x >= lo` (or `x < lo` in negated `||` form).
    Lo(i64),
    /// Inclusive upper bound `x <= hi` (or `x > hi` in negated `||` form).
    Hi(i64),
}

fn exact_i64_i32(expr: &Expr) -> Option<i64> {
    exact_i32(expr).map(i64::from)
}

/// Extracts `(operand, RangeBound)` from a single comparison for an inclusive `&&` range check.
/// Tightens strict `<`/`>` by 1 (sound only once the caller proves `operand` is integer or
/// `NonNegI31OrNan`).
fn extract_in_range_bound(expr: &Expr) -> Option<(Expr, RangeBound)> {
    let Expr::Binary(op, l, r) = expr else {
        return None;
    };
    if let Some(c) = exact_i64_i32(r) {
        if is_pure_range_operand(l) {
            return match op {
                BinOp::Ge => Some(((**l).clone(), RangeBound::Lo(c))),
                BinOp::Gt => Some(((**l).clone(), RangeBound::Lo(c + 1))),
                BinOp::Le => Some(((**l).clone(), RangeBound::Hi(c))),
                BinOp::Lt => Some(((**l).clone(), RangeBound::Hi(c - 1))),
                _ => None,
            };
        }
    }
    if let Some(c) = exact_i64_i32(l) {
        if is_pure_range_operand(r) {
            return match op {
                BinOp::Le => Some(((**r).clone(), RangeBound::Lo(c))),
                BinOp::Lt => Some(((**r).clone(), RangeBound::Lo(c + 1))),
                BinOp::Ge => Some(((**r).clone(), RangeBound::Hi(c))),
                BinOp::Gt => Some(((**r).clone(), RangeBound::Hi(c - 1))),
                _ => None,
            };
        }
    }
    None
}

/// Extracts `(operand, RangeBound)` from a single comparison for an exclusive `||` outside-range
/// check (`x < lo || x > hi` -> `(x - lo >>> 0) > (hi - lo)`).
fn extract_out_range_bound(expr: &Expr) -> Option<(Expr, RangeBound)> {
    let Expr::Binary(op, l, r) = expr else {
        return None;
    };
    if let Some(c) = exact_i64_i32(r) {
        if is_pure_range_operand(l) {
            return match op {
                BinOp::Lt => Some(((**l).clone(), RangeBound::Lo(c))),
                BinOp::Le => Some(((**l).clone(), RangeBound::Lo(c + 1))),
                BinOp::Gt => Some(((**l).clone(), RangeBound::Hi(c))),
                BinOp::Ge => Some(((**l).clone(), RangeBound::Hi(c - 1))),
                _ => None,
            };
        }
    }
    if let Some(c) = exact_i64_i32(l) {
        if is_pure_range_operand(r) {
            return match op {
                BinOp::Gt => Some(((**r).clone(), RangeBound::Lo(c))),
                BinOp::Ge => Some(((**r).clone(), RangeBound::Lo(c + 1))),
                BinOp::Lt => Some(((**r).clone(), RangeBound::Hi(c))),
                BinOp::Le => Some(((**r).clone(), RangeBound::Hi(c - 1))),
                _ => None,
            };
        }
    }
    None
}

fn try_fold_range_pair(cmp1: &Expr, cmp2: &Expr, negated: bool, env: &NumEnv) -> Option<Expr> {
    let (x1, b1) = if negated {
        extract_out_range_bound(cmp1)?
    } else {
        extract_in_range_bound(cmp1)?
    };
    let (x2, b2) = if negated {
        extract_out_range_bound(cmp2)?
    } else {
        extract_in_range_bound(cmp2)?
    };
    if x1 != x2 {
        return None;
    }
    let (lo, hi) = match (b1, b2) {
        (RangeBound::Lo(lo), RangeBound::Hi(hi)) | (RangeBound::Hi(hi), RangeBound::Lo(lo)) => {
            (lo, hi)
        }
        _ => return None,
    };
    let kind = env.classify_expr(&x1)?;
    build_folded_range_check(&x1, lo, hi, negated, kind)
}

fn build_folded_range_check(
    x: &Expr,
    lo: i64,
    hi: i64,
    negated: bool,
    kind: NumKind,
) -> Option<Expr> {
    if lo < 0 || hi > i64::from(i32::MAX) {
        return None;
    }
    if lo > hi {
        return if !negated {
            Some(Expr::Bool(false))
        } else if kind.is_exact_int32_or_u32() {
            Some(Expr::Bool(true))
        } else {
            None
        };
    }
    if lo == hi {
        return if !negated && (kind.is_exact_int32_or_u32() || kind == NumKind::NonNegI31OrNan) {
            Some(jsast::binary(
                BinOp::StrictEq,
                x.clone(),
                Expr::Num(lo as f64),
            ))
        } else if negated && kind.is_exact_int32_or_u32() {
            Some(jsast::binary(
                BinOp::StrictNe,
                x.clone(),
                Expr::Num(lo as f64),
            ))
        } else {
            None
        };
    }
    // Now `0 <= lo < hi <= 0x7fff_ffff`.
    let cmp_op = if negated { BinOp::Gt } else { BinOp::Le };
    if lo == 0 {
        if kind.is_nonneg_int() {
            return Some(jsast::binary(cmp_op, x.clone(), Expr::Num(hi as f64)));
        }
        if kind == NumKind::I32 {
            let lhs = jsast::binary(BinOp::UShr, x.clone(), Expr::Num(0.0));
            return Some(jsast::binary(cmp_op, lhs, Expr::Num(hi as f64)));
        }
        // `NonNegI31OrNan` MUST NOT fold at `lo == 0` because `(NaN | 0) = 0 <= hi` is true!
        return None;
    }
    let width = hi - lo;
    if kind.is_exact_int32_or_u32() {
        let sub = jsast::binary(BinOp::Sub, x.clone(), Expr::Num(lo as f64));
        let shifted = jsast::binary(BinOp::UShr, sub, Expr::Num(0.0));
        return Some(jsast::binary(cmp_op, shifted, Expr::Num(width as f64)));
    }
    if kind == NumKind::NonNegI31OrNan && !negated {
        // Sound for ALL `x ∈ {0..=0x7fff_ffff} ∪ {NaN}` when `lo >= 1`:
        // `(NaN | 0) = 0`, `(0 - lo) >>> 0 = 2^32 - lo > hi - lo` -> `false`.
        let coerced = jsast::binary(BinOp::BitOr, x.clone(), Expr::Num(0.0));
        let sub = jsast::binary(BinOp::Sub, coerced, Expr::Num(lo as f64));
        let shifted = jsast::binary(BinOp::UShr, sub, Expr::Num(0.0));
        return Some(jsast::binary(BinOp::Le, shifted, Expr::Num(width as f64)));
    }
    None
}

fn fold_range_expr(expr: &mut Expr, env: &mut NumEnv) {
    match expr {
        Expr::Assign(lhs, rhs) => {
            fold_range_expr(rhs, env);
            if let Expr::Ident(id) = lhs.as_ref() {
                env.update_assign(id, rhs);
            } else {
                fold_range_expr(lhs, env);
            }
        }
        Expr::CompoundAssign(_, lhs, rhs) => {
            fold_range_expr(lhs, env);
            fold_range_expr(rhs, env);
            if let Expr::Ident(id) = lhs.as_ref() {
                env.invalidate(id);
            }
        }
        Expr::Binary(BinOp::And, l, r) => {
            fold_range_expr(l, env);
            let mut r_env = env.clone();
            r_env.apply_cond_facts(l, true);
            fold_range_expr(r, &mut r_env);
            let mut written = HashSet::new();
            collect_written_idents_expr(r, &mut written);
            for w in written {
                env.invalidate(&w);
            }
            // Direct `cmp1 && cmp2`:
            if let Some(folded) = try_fold_range_pair(l, r, false, env) {
                *expr = folded;
                return;
            }
            // Left-associated `(P && cmp1) && cmp2`:
            if let Expr::Binary(BinOp::And, p, cmp1) = l.as_ref() {
                let mut p_env = env.clone();
                p_env.apply_cond_facts(p, true);
                if let Some(folded) = try_fold_range_pair(cmp1, r, false, &p_env) {
                    let p_owned = (**p).clone();
                    *expr = jsast::binary(BinOp::And, p_owned, folded);
                    return;
                }
            }
            // Right-associated `cmp1 && (cmp2 && Q)`:
            if let Expr::Binary(BinOp::And, cmp2, q) = r.as_ref() {
                if let Some(folded) = try_fold_range_pair(l, cmp2, false, env) {
                    let q_owned = (**q).clone();
                    *expr = jsast::binary(BinOp::And, folded, q_owned);
                }
            }
        }
        Expr::Binary(BinOp::Or, l, r) => {
            fold_range_expr(l, env);
            let mut r_env = env.clone();
            r_env.apply_cond_facts(l, false);
            fold_range_expr(r, &mut r_env);
            let mut written = HashSet::new();
            collect_written_idents_expr(r, &mut written);
            for w in written {
                env.invalidate(&w);
            }
            if let Some(folded) = try_fold_range_pair(l, r, true, env) {
                *expr = folded;
                return;
            }
            if let Expr::Binary(BinOp::Or, p, cmp1) = l.as_ref() {
                let mut p_env = env.clone();
                p_env.apply_cond_facts(p, false);
                if let Some(folded) = try_fold_range_pair(cmp1, r, true, &p_env) {
                    let p_owned = (**p).clone();
                    *expr = jsast::binary(BinOp::Or, p_owned, folded);
                    return;
                }
            }
            if let Expr::Binary(BinOp::Or, cmp2, q) = r.as_ref() {
                if let Some(folded) = try_fold_range_pair(l, cmp2, true, env) {
                    let q_owned = (**q).clone();
                    *expr = jsast::binary(BinOp::Or, folded, q_owned);
                }
            }
        }
        Expr::Cond(c, t, e) => {
            fold_range_expr(c, env);
            let mut t_env = env.clone();
            t_env.apply_cond_facts(c, true);
            fold_range_expr(t, &mut t_env);
            let mut e_env = env.clone();
            e_env.apply_cond_facts(c, false);
            fold_range_expr(e, &mut e_env);
            let mut written = HashSet::new();
            collect_written_idents_expr(t, &mut written);
            collect_written_idents_expr(e, &mut written);
            for w in written {
                env.invalidate(&w);
            }
        }
        Expr::Member(o, _) | Expr::Unary(_, o) => fold_range_expr(o, env),
        Expr::Index(o, i) | Expr::Binary(_, o, i) => {
            fold_range_expr(o, env);
            fold_range_expr(i, env);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            fold_range_expr(c, env);
            for a in args {
                fold_range_expr(a, env);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for x in items {
                fold_range_expr(x, env);
            }
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                fold_range_expr(v, env);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(b)) => {
            let mut inner = NumEnv::default();
            fold_range_expr(b, &mut inner);
        }
        Expr::Arrow(_, ArrowBody::Block(s)) => {
            let mut inner = NumEnv::default();
            let empty = HashSet::new();
            let empty_rc = HashMap::new();
            fold_range_stmts(s, &mut inner, &empty, &empty_rc);
        }
        _ => {}
    }
}

/// Matches a 1- or 2-statement branch of the form `[dst = rhs]` or `[dst = rhs; break L;]`.
fn match_assign_opt_break(stmts: &[Stmt]) -> Option<(&str, &Expr, Option<&str>)> {
    match stmts {
        [Stmt::ExprStmt(Expr::Assign(lhs, rhs))] => {
            let Expr::Ident(dst) = lhs.as_ref() else {
                return None;
            };
            Some((dst.as_str(), rhs.as_ref(), None))
        }
        [Stmt::ExprStmt(Expr::Assign(lhs, rhs)), Stmt::Break(Some(lbl))] => {
            let Expr::Ident(dst) = lhs.as_ref() else {
                return None;
            };
            Some((dst.as_str(), rhs.as_ref(), Some(lbl.as_str())))
        }
        _ => None,
    }
}

/// Negates a simple relational comparison on a pure operand (e.g. `x < A` -> `x >= A`).
fn negate_rel_cmp(expr: &Expr) -> Option<Expr> {
    let Expr::Binary(op, l, r) = expr else {
        return None;
    };
    let inv = match op {
        BinOp::Lt => BinOp::Ge,
        BinOp::Le => BinOp::Gt,
        BinOp::Gt => BinOp::Le,
        BinOp::Ge => BinOp::Lt,
        _ => return None,
    };
    Some(jsast::binary(inv, (**l).clone(), (**r).clone()))
}

/// Tries to fold `if (cmp1) { dst = cmp2; [break L;] } else { dst = false; [break L;] }` (and
/// symmetric `true` / swapped-arm forms) into `[dst = folded_range; [break L;]]`.
fn try_fold_range_if_assign(
    cond: &Expr,
    then_b: &[Stmt],
    else_b: &[Stmt],
    env: &NumEnv,
) -> Option<Vec<Stmt>> {
    let (dst1, rhs1, brk1) = match_assign_opt_break(then_b)?;
    let (dst2, rhs2, brk2) = match_assign_opt_break(else_b)?;
    if dst1 != dst2 || brk1 != brk2 {
        return None;
    }
    let folded = match (rhs1, rhs2) {
        // `if (cmp1) { dst = cmp2; } else { dst = false; }` -> `dst = cmp1 && cmp2`
        (cmp2, Expr::Bool(false)) => try_fold_range_pair(cond, cmp2, false, env)?,
        // `if (cmp1) { dst = false; } else { dst = cmp2; }` -> `dst = !cmp1 && cmp2`
        (Expr::Bool(false), cmp2) => {
            let neg_cond = negate_rel_cmp(cond)?;
            try_fold_range_pair(&neg_cond, cmp2, false, env)?
        }
        // `if (cmp1) { dst = true; } else { dst = cmp2; }` -> `dst = cmp1 || cmp2`
        (Expr::Bool(true), cmp2) => try_fold_range_pair(cond, cmp2, true, env)?,
        // `if (cmp1) { dst = cmp2; } else { dst = true; }` -> `dst = !cmp1 || cmp2`
        (cmp2, Expr::Bool(true)) => {
            let neg_cond = negate_rel_cmp(cond)?;
            try_fold_range_pair(&neg_cond, cmp2, true, env)?
        }
        _ => return None,
    };
    let mut out = vec![jsast::assign_stmt(jsast::id(dst1.to_owned()), folded)];
    if let Some(lbl) = brk1 {
        out.push(Stmt::Break(Some(lbl.to_owned())));
    }
    Some(out)
}

fn fold_range_stmts(
    stmts: &mut Vec<Stmt>,
    env: &mut NumEnv,
    local_names: &HashSet<String>,
    read_counts: &HashMap<String, usize>,
) {
    let mut i = 0usize;
    while i < stmts.len() {
        // Pattern 1S-b: `tmp = cmp1; if (tmp) { dst = cmp2; [break L;] } else { dst = false; [break L;] }`
        // where `tmp` is a single-read local temp (`jmespath`, `diff`).
        if i + 1 < stmts.len() {
            let matched_pair: Option<Vec<Stmt>> = match (&stmts[i], &stmts[i + 1]) {
                (
                    Stmt::ExprStmt(Expr::Assign(lhs, cmp1)),
                    Stmt::If(Expr::Ident(cond_id), then_b, Some(else_b)),
                ) => {
                    if let Expr::Ident(tmp) = lhs.as_ref() {
                        if tmp == cond_id
                            && local_names.contains(tmp)
                            && read_counts.get(tmp).copied().unwrap_or(0) == 1
                        {
                            try_fold_range_if_assign(cmp1, then_b, else_b, env)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(repl) = matched_pair {
                if let Some(Stmt::ExprStmt(Expr::Assign(lhs, rhs))) = repl.first() {
                    if let Expr::Ident(dst) = lhs.as_ref() {
                        env.update_assign(dst, rhs);
                    }
                }
                let n = repl.len();
                stmts.splice(i..i + 2, repl);
                i += n;
                continue;
            }
        }

        // Pattern 1S-a: `if (cmp1) { dst = cmp2; [break L;] } else { dst = false; [break L;] }`
        if let Stmt::If(cond, then_b, Some(else_b)) = &stmts[i] {
            if let Some(repl) = try_fold_range_if_assign(cond, then_b, else_b, env) {
                if let Some(Stmt::ExprStmt(Expr::Assign(lhs, rhs))) = repl.first() {
                    if let Expr::Ident(dst) = lhs.as_ref() {
                        env.update_assign(dst, rhs);
                    }
                }
                let n = repl.len();
                stmts.splice(i..=i, repl);
                i += n;
                continue;
            }
        }

        match &mut stmts[i] {
            Stmt::ExprStmt(e) => fold_range_expr(e, env),
            Stmt::Throw(e) | Stmt::Return(Some(e)) => fold_range_expr(e, env),
            Stmt::Let(n, init) => {
                if let Some(e) = init {
                    fold_range_expr(e, env);
                    env.update_assign(n, e);
                } else {
                    env.invalidate(n);
                }
            }
            Stmt::Const(n, e) => {
                fold_range_expr(e, env);
                env.update_assign(n, e);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(e) = init {
                        fold_range_expr(e, env);
                        env.update_assign(n, e);
                    } else {
                        env.invalidate(n);
                    }
                }
            }
            Stmt::If(cond, then_b, else_b) => {
                fold_range_expr(cond, env);
                let mut then_env = env.clone();
                then_env.apply_cond_facts(cond, true);
                fold_range_stmts(then_b, &mut then_env, local_names, read_counts);
                let then_exits = stmts_definitely_exit(then_b);
                if let Some(eb) = else_b {
                    let mut else_env = env.clone();
                    else_env.apply_cond_facts(cond, false);
                    fold_range_stmts(eb, &mut else_env, local_names, read_counts);
                    let else_exits = stmts_definitely_exit(eb);
                    match (then_exits, else_exits) {
                        (true, false) => *env = else_env,
                        (false, true) => *env = then_env,
                        (false, false) => {
                            then_env.join_with(&else_env);
                            *env = then_env;
                        }
                        (true, true) => {}
                    }
                } else if then_exits {
                    env.apply_cond_facts(cond, false);
                } else {
                    let mut fallthrough_env = env.clone();
                    fallthrough_env.apply_cond_facts(cond, false);
                    then_env.join_with(&fallthrough_env);
                    *env = then_env;
                }
            }
            Stmt::Switch(disc, cases) => {
                fold_range_expr(disc, env);
                let mut written = HashSet::new();
                for c in cases.iter_mut() {
                    for t in &mut c.tests {
                        let mut te = env.clone();
                        fold_range_expr(t, &mut te);
                    }
                    let mut ce = env.clone();
                    fold_range_stmts(&mut c.body, &mut ce, local_names, read_counts);
                    for s in &c.body {
                        collect_written_idents_stmt(s, &mut written);
                    }
                }
                for w in written {
                    env.invalidate(&w);
                }
            }
            Stmt::Block(b) => fold_range_stmts(b, env, local_names, read_counts),
            Stmt::Labeled(_, inner) => {
                let mut written = HashSet::new();
                collect_written_idents_stmt(inner, &mut written);
                let mut ie = env.clone();
                let mut tmp = vec![inner.as_ref().clone()];
                fold_range_stmts(&mut tmp, &mut ie, local_names, read_counts);
                if tmp.len() == 1 {
                    *inner.as_mut() = tmp.pop().unwrap();
                } else {
                    *inner.as_mut() = Stmt::Block(tmp);
                }
                for w in written {
                    env.invalidate(&w);
                }
            }
            Stmt::LoopForever(_, b) => {
                let mut le = env.prepare_for_loop(None, b);
                fold_range_stmts(b, &mut le, local_names, read_counts);
                let mut written = HashSet::new();
                for s in b.iter() {
                    collect_written_idents_stmt(s, &mut written);
                }
                for w in written {
                    env.invalidate(&w);
                }
            }
            Stmt::While(cond, b) => {
                let mut le = env.prepare_for_loop(Some(cond), b);
                fold_range_expr(cond, &mut le);
                let mut body_env = le.clone();
                body_env.apply_cond_facts(cond, true);
                fold_range_stmts(b, &mut body_env, local_names, read_counts);
                let mut written = HashSet::new();
                collect_written_idents_expr(cond, &mut written);
                for s in b.iter() {
                    collect_written_idents_stmt(s, &mut written);
                }
                for w in written {
                    env.invalidate(&w);
                }
            }
            Stmt::TryCatch {
                body,
                binding,
                handler,
            } => {
                let mut written = HashSet::new();
                for s in body.iter() {
                    collect_written_idents_stmt(s, &mut written);
                }
                for s in handler.iter() {
                    collect_written_idents_stmt(s, &mut written);
                }
                written.insert(binding.clone());
                let mut be = env.clone();
                fold_range_stmts(body, &mut be, local_names, read_counts);
                let mut he = env.clone();
                for w in &written {
                    he.invalidate(w);
                    env.invalidate(w);
                }
                fold_range_stmts(handler, &mut he, local_names, read_counts);
            }
            Stmt::FunctionDecl { body, .. } => {
                let mut inner = NumEnv::default();
                let empty = HashSet::new();
                let empty_rc = HashMap::new();
                fold_range_stmts(body, &mut inner, &empty, &empty_rc);
            }
            _ => {}
        }
        i += 1;
    }
}

fn fold_unsigned_range_checks(body: &mut Vec<Stmt>, local_names: &HashSet<String>) {
    let mut read_counts: HashMap<String, usize> = HashMap::new();
    for stmt in body.iter() {
        tally_ident_reads_stmt(stmt, &mut read_counts);
    }
    let mut env = NumEnv::default();
    fold_range_stmts(body, &mut env, local_names, &read_counts);
}

// =============================================================================================
// 6. Part 2: Dense ASCII Delimiter Membership -> 128/256-Byte `Uint8Array` LUT
// =============================================================================================

const MIN_LUT_DISTINCT_BYTES: usize = 6;

fn exact_u8(expr: &Expr) -> Option<u8> {
    exact_u32(expr).and_then(|v| u8::try_from(v).ok())
}

fn build_u8_lut_test(disc: Expr, bytes: &[u8], negated: bool) -> Expr {
    let max_b = bytes.iter().copied().max().unwrap_or(0);
    let len = if max_b <= 127 { 128usize } else { 256usize };
    let mut table = vec![Expr::Num(0.0); len];
    for &b in bytes {
        table[b as usize] = Expr::Num(1.0);
    }
    let lut = Expr::New(Box::new(jsast::id("Uint8Array")), vec![Expr::Array(table)]);
    let load = jsast::index(lut, disc);
    jsast::binary(
        if negated {
            BinOp::StrictNe
        } else {
            BinOp::StrictEq
        },
        load,
        Expr::Num(1.0),
    )
}

/// Extracts `(operand, byte)` from `c === k` / `k === c` (when `expect_eq == true`) or
/// `c !== k` / `k !== c` / `!(c === k)` (when `expect_eq == false`).
fn extract_byte_eq_term(expr: &Expr, expect_eq: bool) -> Option<(Expr, u8)> {
    match expr {
        Expr::Unary(UnOp::Not, inner) if !expect_eq => extract_byte_eq_term(inner, true),
        Expr::Binary(op, l, r) => {
            let ok = if expect_eq {
                *op == BinOp::StrictEq
            } else {
                *op == BinOp::StrictNe
            };
            if !ok {
                return None;
            }
            if let Some(b) = exact_u8(r) {
                if is_pure_range_operand(l) {
                    return Some(((**l).clone(), b));
                }
            }
            if let Some(b) = exact_u8(l) {
                if is_pure_range_operand(r) {
                    return Some(((**r).clone(), b));
                }
            }
            None
        }
        _ => None,
    }
}

fn flatten_logical(expr: &Expr, op: BinOp, out: &mut Vec<Expr>) {
    if let Expr::Binary(eop, l, r) = expr {
        if *eop == op {
            flatten_logical(l, op, out);
            flatten_logical(r, op, out);
            return;
        }
    }
    out.push(expr.clone());
}

fn try_fold_byte_lut_chain(expr: &Expr, op: BinOp, env: &NumEnv) -> Option<Expr> {
    let expect_eq = op == BinOp::Or;
    let mut terms = Vec::new();
    flatten_logical(expr, op, &mut terms);
    if terms.len() < MIN_LUT_DISTINCT_BYTES {
        return None;
    }
    let mut out_terms: Vec<Expr> = Vec::with_capacity(terms.len());
    let mut changed = false;
    let mut i = 0usize;
    while i < terms.len() {
        let Some((cand_x, first_b)) = extract_byte_eq_term(&terms[i], expect_eq) else {
            out_terms.push(terms[i].clone());
            i += 1;
            continue;
        };
        // CRITICAL SOUNDNESS: `lut["92"]` is `1` in JS whereas `"92" === 92` is `false`!
        // Only lower when `cand_x` is provably a JS number (never a string/object).
        if env.classify_expr(&cand_x).is_none() {
            out_terms.push(terms[i].clone());
            i += 1;
            continue;
        }
        let mut bytes: Vec<u8> = vec![first_b];
        let mut distinct: HashSet<u8> = HashSet::new();
        distinct.insert(first_b);
        let mut j = i + 1;
        while j < terms.len() {
            if let Some((x2, b2)) = extract_byte_eq_term(&terms[j], expect_eq) {
                if x2 == cand_x {
                    bytes.push(b2);
                    distinct.insert(b2);
                    j += 1;
                    continue;
                }
            }
            break;
        }
        if distinct.len() >= MIN_LUT_DISTINCT_BYTES {
            out_terms.push(build_u8_lut_test(cand_x, &bytes, !expect_eq));
            changed = true;
        } else {
            for t in &terms[i..j] {
                out_terms.push(t.clone());
            }
        }
        i = j;
    }
    if !changed {
        return None;
    }
    let mut iter = out_terms.into_iter();
    let first = iter.next()?;
    Some(iter.fold(first, |acc, next| jsast::binary(op, acc, next)))
}

fn lower_lut_expr(expr: &mut Expr, env: &mut NumEnv) {
    match expr {
        Expr::Assign(lhs, rhs) => {
            lower_lut_expr(rhs, env);
            if let Expr::Ident(id) = lhs.as_ref() {
                env.update_assign(id, rhs);
            } else {
                lower_lut_expr(lhs, env);
            }
        }
        Expr::CompoundAssign(_, lhs, rhs) => {
            lower_lut_expr(lhs, env);
            lower_lut_expr(rhs, env);
            if let Expr::Ident(id) = lhs.as_ref() {
                env.invalidate(id);
            }
        }
        Expr::Binary(op @ (BinOp::Or | BinOp::And), _, _) => {
            let op = *op;
            if let Some(folded) = try_fold_byte_lut_chain(expr, op, env) {
                *expr = folded;
                return;
            }
            if let Expr::Binary(_, l, r) = expr {
                lower_lut_expr(l, env);
                let mut r_env = env.clone();
                r_env.apply_cond_facts(l, op == BinOp::And);
                lower_lut_expr(r, &mut r_env);
                let mut written = HashSet::new();
                collect_written_idents_expr(r, &mut written);
                for w in written {
                    env.invalidate(&w);
                }
            }
        }
        Expr::Cond(c, t, e) => {
            lower_lut_expr(c, env);
            let mut t_env = env.clone();
            t_env.apply_cond_facts(c, true);
            lower_lut_expr(t, &mut t_env);
            let mut e_env = env.clone();
            e_env.apply_cond_facts(c, false);
            lower_lut_expr(e, &mut e_env);
            let mut written = HashSet::new();
            collect_written_idents_expr(t, &mut written);
            collect_written_idents_expr(e, &mut written);
            for w in written {
                env.invalidate(&w);
            }
        }
        Expr::Member(o, _) | Expr::Unary(_, o) => lower_lut_expr(o, env),
        Expr::Index(o, i) | Expr::Binary(_, o, i) => {
            lower_lut_expr(o, env);
            lower_lut_expr(i, env);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            lower_lut_expr(c, env);
            for a in args {
                lower_lut_expr(a, env);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for x in items {
                lower_lut_expr(x, env);
            }
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                lower_lut_expr(v, env);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(b)) => {
            let mut inner = NumEnv::default();
            lower_lut_expr(b, &mut inner);
        }
        Expr::Arrow(_, ArrowBody::Block(s)) => {
            let mut inner = NumEnv::default();
            lower_lut_stmts(s, &mut inner);
        }
        _ => {}
    }
}

/// Attempts to lower a 2-arm (or 1-arm) dense ASCII `switch (disc)` with `>= 6` distinct byte
/// constants in `0..=255` on a proven-numeric `disc` into a `Uint8Array` LUT check.
fn try_lower_ascii_switch_to_lut(
    disc: &Expr,
    cases: &[jsast::SwitchCase],
    env: &NumEnv,
) -> Option<Stmt> {
    if env.classify_expr(disc).is_none() || cases.is_empty() {
        return None;
    }

    // Group consecutive empty-body non-default `SwitchCase`s into combined arms.
    let mut arms: Vec<(bool, Vec<Expr>, Vec<Stmt>)> = Vec::new();
    let mut pending_tests: Vec<Expr> = Vec::new();
    for (idx, c) in cases.iter().enumerate() {
        let is_default = c.tests.is_empty();
        if is_default {
            if !pending_tests.is_empty() {
                // Non-default case falling straight into `default:` — bail out.
                return None;
            }
            arms.push((true, Vec::new(), c.body.clone()));
        } else {
            pending_tests.extend(c.tests.iter().cloned());
            let is_last = idx + 1 == cases.len();
            if !c.body.is_empty() || is_last {
                arms.push((false, std::mem::take(&mut pending_tests), c.body.clone()));
            }
        }
    }
    if !pending_tests.is_empty() || arms.is_empty() || arms.len() > 2 {
        return None;
    }

    // Extract `(match_tests, match_body, match_must_exit, default_body, default_must_exit)`.
    let (match_tests, mut match_body, mut default_body) = match arms.len() {
        1 => {
            let (is_def, tests, body) = arms.pop().unwrap();
            if is_def {
                return None;
            }
            (tests, body, Vec::new())
        }
        2 => {
            let arm1 = arms.pop().unwrap();
            let arm0 = arms.pop().unwrap();
            match (arm0.0, arm1.0) {
                (false, true) => {
                    // Matching arm comes before `default:`: ensure it CANNOT fall through!
                    let exits = matches!(arm0.2.last(), Some(Stmt::Break(None)))
                        || stmts_definitely_exit(&arm0.2);
                    if !exits {
                        return None;
                    }
                    (arm0.1, arm0.2, arm1.2)
                }
                (true, false) => {
                    // `default:` comes before matching arm: ensure `default:` CANNOT fall through!
                    let exits = matches!(arm0.2.last(), Some(Stmt::Break(None)))
                        || stmts_definitely_exit(&arm0.2);
                    if !exits {
                        return None;
                    }
                    (arm1.1, arm1.2, arm0.2)
                }
                _ => return None,
            }
        }
        _ => return None,
    };

    // Every case label must be an exact byte in `0..=255`, with `>= 6` distinct bytes.
    let mut bytes: Vec<u8> = Vec::with_capacity(match_tests.len());
    let mut distinct: HashSet<u8> = HashSet::new();
    for t in &match_tests {
        let b = exact_u8(t)?;
        bytes.push(b);
        distinct.insert(b);
    }
    if distinct.len() < MIN_LUT_DISTINCT_BYTES {
        return None;
    }

    // Strip a single trailing unlabeled `break;` from each arm and reject any interior unlabeled
    // `break;` (which targeted this `switch` and would otherwise target an enclosing loop/switch!).
    if matches!(match_body.last(), Some(Stmt::Break(None))) {
        match_body.pop();
    }
    if matches!(default_body.last(), Some(Stmt::Break(None))) {
        default_body.pop();
    }
    if has_unlabeled_switch_break(&match_body) || has_unlabeled_switch_break(&default_body) {
        return None;
    }

    // Cleanest lowering for boolean assignment: `case ...: dst = true; break; default: dst = false;`
    if let ([Stmt::ExprStmt(Expr::Assign(l1, r1))], [Stmt::ExprStmt(Expr::Assign(l2, r2))]) =
        (match_body.as_slice(), default_body.as_slice())
    {
        if let (Expr::Ident(d1), Expr::Ident(d2)) = (l1.as_ref(), l2.as_ref()) {
            if d1 == d2 {
                match (r1.as_ref(), r2.as_ref()) {
                    (Expr::Bool(true), Expr::Bool(false)) => {
                        return Some(jsast::assign_stmt(
                            jsast::id(d1.clone()),
                            build_u8_lut_test(disc.clone(), &bytes, false),
                        ));
                    }
                    (Expr::Bool(false), Expr::Bool(true)) => {
                        return Some(jsast::assign_stmt(
                            jsast::id(d1.clone()),
                            build_u8_lut_test(disc.clone(), &bytes, true),
                        ));
                    }
                    _ => {}
                }
            }
        }
    }

    if match_body.is_empty() && default_body.is_empty() {
        return None;
    }
    if match_body.is_empty() {
        return Some(Stmt::If(
            build_u8_lut_test(disc.clone(), &bytes, true),
            default_body,
            None,
        ));
    }
    if default_body.is_empty() {
        return Some(Stmt::If(
            build_u8_lut_test(disc.clone(), &bytes, false),
            match_body,
            None,
        ));
    }
    Some(Stmt::If(
        build_u8_lut_test(disc.clone(), &bytes, false),
        match_body,
        Some(default_body),
    ))
}

fn lower_lut_stmts(stmts: &mut [Stmt], env: &mut NumEnv) {
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::ExprStmt(e) => lower_lut_expr(e, env),
            Stmt::Throw(e) | Stmt::Return(Some(e)) => lower_lut_expr(e, env),
            Stmt::Let(n, init) => {
                if let Some(e) = init {
                    lower_lut_expr(e, env);
                    env.update_assign(n, e);
                } else {
                    env.invalidate(n);
                }
            }
            Stmt::Const(n, e) => {
                lower_lut_expr(e, env);
                env.update_assign(n, e);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(e) = init {
                        lower_lut_expr(e, env);
                        env.update_assign(n, e);
                    } else {
                        env.invalidate(n);
                    }
                }
            }
            Stmt::If(cond, then_b, else_b) => {
                lower_lut_expr(cond, env);
                let mut then_env = env.clone();
                then_env.apply_cond_facts(cond, true);
                lower_lut_stmts(then_b, &mut then_env);
                let then_exits = stmts_definitely_exit(then_b);
                if let Some(eb) = else_b {
                    let mut else_env = env.clone();
                    else_env.apply_cond_facts(cond, false);
                    lower_lut_stmts(eb, &mut else_env);
                    let else_exits = stmts_definitely_exit(eb);
                    match (then_exits, else_exits) {
                        (true, false) => *env = else_env,
                        (false, true) => *env = then_env,
                        (false, false) => {
                            then_env.join_with(&else_env);
                            *env = then_env;
                        }
                        (true, true) => {}
                    }
                } else if then_exits {
                    env.apply_cond_facts(cond, false);
                } else {
                    let mut fallthrough_env = env.clone();
                    fallthrough_env.apply_cond_facts(cond, false);
                    then_env.join_with(&fallthrough_env);
                    *env = then_env;
                }
            }
            Stmt::Switch(disc, cases) => {
                lower_lut_expr(disc, env);
                let mut written = HashSet::new();
                for c in cases.iter_mut() {
                    for t in &mut c.tests {
                        let mut te = env.clone();
                        lower_lut_expr(t, &mut te);
                    }
                    let mut ce = env.clone();
                    lower_lut_stmts(&mut c.body, &mut ce);
                    for s in &c.body {
                        collect_written_idents_stmt(s, &mut written);
                    }
                }
                if let Some(rewritten) = try_lower_ascii_switch_to_lut(disc, cases, env) {
                    *stmt = rewritten;
                }
                for w in written {
                    env.invalidate(&w);
                }
            }
            Stmt::Block(b) => lower_lut_stmts(b, env),
            Stmt::Labeled(_, inner) => {
                let mut written = HashSet::new();
                collect_written_idents_stmt(inner, &mut written);
                let mut ie = env.clone();
                let mut tmp = vec![inner.as_ref().clone()];
                lower_lut_stmts(&mut tmp, &mut ie);
                if tmp.len() == 1 {
                    *inner.as_mut() = tmp.pop().unwrap();
                } else {
                    *inner.as_mut() = Stmt::Block(tmp);
                }
                for w in written {
                    env.invalidate(&w);
                }
            }
            Stmt::LoopForever(_, b) => {
                let mut le = env.prepare_for_loop(None, b);
                lower_lut_stmts(b, &mut le);
                let mut written = HashSet::new();
                for s in b.iter() {
                    collect_written_idents_stmt(s, &mut written);
                }
                for w in written {
                    env.invalidate(&w);
                }
            }
            Stmt::While(cond, b) => {
                let mut le = env.prepare_for_loop(Some(cond), b);
                lower_lut_expr(cond, &mut le);
                let mut body_env = le.clone();
                body_env.apply_cond_facts(cond, true);
                lower_lut_stmts(b, &mut body_env);
                let mut written = HashSet::new();
                collect_written_idents_expr(cond, &mut written);
                for s in b.iter() {
                    collect_written_idents_stmt(s, &mut written);
                }
                for w in written {
                    env.invalidate(&w);
                }
            }
            Stmt::TryCatch {
                body,
                binding,
                handler,
            } => {
                let mut written = HashSet::new();
                for s in body.iter() {
                    collect_written_idents_stmt(s, &mut written);
                }
                for s in handler.iter() {
                    collect_written_idents_stmt(s, &mut written);
                }
                written.insert(binding.clone());
                let mut be = env.clone();
                lower_lut_stmts(body, &mut be);
                let mut he = env.clone();
                for w in &written {
                    he.invalidate(w);
                    env.invalidate(w);
                }
                lower_lut_stmts(handler, &mut he);
            }
            Stmt::FunctionDecl { body, .. } => {
                let mut inner = NumEnv::default();
                lower_lut_stmts(body, &mut inner);
            }
            _ => {}
        }
    }
}

fn lower_dense_ascii_luts(body: &mut [Stmt]) {
    let mut env = NumEnv::default();
    lower_lut_stmts(body, &mut env);
}

// =============================================================================================
// 7. Part 3: Chained String / Integer Equality `if / else if` & Guard Chains -> `switch (x)`
// =============================================================================================

const MIN_CHAIN_SWITCH_ARMS: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SwitchKey {
    Str(String),
    Int(i32),
}

impl SwitchKey {
    fn is_str(&self) -> bool {
        matches!(self, Self::Str(_))
    }

    fn to_expr(&self) -> Expr {
        match self {
            Self::Str(s) => Expr::Str(s.clone()),
            Self::Int(n) => num_expr_i32(*n),
        }
    }
}

fn extract_switch_key_lit(expr: &Expr) -> Option<SwitchKey> {
    match expr {
        Expr::Str(s) => Some(SwitchKey::Str(s.clone())),
        _ => exact_i32(expr).map(SwitchKey::Int),
    }
}

/// Extracts `(ident, is_positive, keys)` from a strict-equality disjunction
/// (`x === "A" || x === "B"`, `is_positive = true`) or negated conjunction
/// (`!(x === "A") && !(x === "B")` / `x !== "A" && x !== "B"`, `is_positive = false`).
fn extract_eq_keys(cond: &Expr) -> Option<(String, bool, Vec<SwitchKey>)> {
    match cond {
        Expr::Unary(UnOp::Not, inner) => {
            let (id, pos, keys) = extract_eq_keys(inner)?;
            Some((id, !pos, keys))
        }
        Expr::Binary(op @ (BinOp::StrictEq | BinOp::StrictNe), l, r) => {
            let is_pos = *op == BinOp::StrictEq;
            if let (Expr::Ident(id), Some(k)) = (l.as_ref(), extract_switch_key_lit(r)) {
                return Some((id.clone(), is_pos, vec![k]));
            }
            if let (Some(k), Expr::Ident(id)) = (extract_switch_key_lit(l), r.as_ref()) {
                return Some((id.clone(), is_pos, vec![k]));
            }
            None
        }
        Expr::Binary(BinOp::Or, l, r) => {
            let (id1, pos1, mut k1) = extract_eq_keys(l)?;
            let (id2, pos2, k2) = extract_eq_keys(r)?;
            if pos1 && pos2 && id1 == id2 && k1[0].is_str() == k2[0].is_str() {
                k1.extend(k2);
                Some((id1, true, k1))
            } else {
                None
            }
        }
        Expr::Binary(BinOp::And, l, r) => {
            let (id1, pos1, mut k1) = extract_eq_keys(l)?;
            let (id2, pos2, k2) = extract_eq_keys(r)?;
            if !pos1 && !pos2 && id1 == id2 && k1[0].is_str() == k2[0].is_str() {
                k1.extend(k2);
                Some((id1, false, k1))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Recursively collects switch arms dispatching on `scrut` from `slice`.
///
/// When `consume_all == true` (inside an `else` or negated-`then` sub-slice), any unmatched tail
/// becomes the `default:` body. When `consume_all == false` (at an outer statement list), returns
/// the prefix count `consumed` of statements absorbed into the `switch`.
fn collect_switch_arms(
    slice: &[Stmt],
    scrut: &str,
    key_is_str: bool,
    consume_all: bool,
    arms: &mut Vec<(Vec<SwitchKey>, Vec<Stmt>)>,
    seen: &mut HashSet<SwitchKey>,
) -> Option<(usize, Option<Vec<Stmt>>)> {
    if slice.is_empty() {
        return Some((0, None));
    }
    if let Stmt::If(cond, then_b, else_opt) = &slice[0] {
        if let Some((id, is_pos, raw_keys)) = extract_eq_keys(cond) {
            if id == scrut && raw_keys[0].is_str() == key_is_str {
                // Deduplicate within a single arm and reject any key already matched earlier.
                let mut dedup_keys = Vec::with_capacity(raw_keys.len());
                let mut local_seen = HashSet::new();
                for k in raw_keys {
                    if seen.contains(&k) {
                        return None;
                    }
                    if local_seen.insert(k.clone()) {
                        dedup_keys.push(k);
                    }
                }
                for k in &dedup_keys {
                    seen.insert(k.clone());
                }

                match (is_pos, else_opt) {
                    // Subcase A: `if (x === "A" || ...) { then_b } else { else_b }`
                    (true, Some(else_b)) if !consume_all || slice.len() == 1 => {
                        arms.push((dedup_keys, then_b.clone()));
                        let (_, default) =
                            collect_switch_arms(else_b, scrut, key_is_str, true, arms, seen)?;
                        return Some((1, default));
                    }
                    // Subcase B: `if (x === "A" || ...) { then_b }` (no `else`)
                    (true, None) => {
                        if stmts_definitely_exit(then_b) {
                            arms.push((dedup_keys, then_b.clone()));
                            let (rest_consumed, default) = collect_switch_arms(
                                &slice[1..],
                                scrut,
                                key_is_str,
                                consume_all,
                                arms,
                                seen,
                            )?;
                            return Some((1 + rest_consumed, default));
                        } else if !consume_all || slice.len() == 1 {
                            arms.push((dedup_keys, then_b.clone()));
                            return Some((1, None));
                        }
                    }
                    // Subcase C: `if (!(x === "A") && ...) { then_b } else { else_b }`
                    (false, Some(else_b)) if !consume_all || slice.len() == 1 => {
                        arms.push((dedup_keys, else_b.clone()));
                        let (_, default) =
                            collect_switch_arms(then_b, scrut, key_is_str, true, arms, seen)?;
                        return Some((1, default));
                    }
                    // Subcase D: `if (!(x === "A") && ...) { then_b } tail...` where `then_b`
                    // definitely exits (`jmespath$eval`, `jmespath$functions::call`, `projection`).
                    (false, None) if slice.len() > 1 && stmts_definitely_exit(then_b) => {
                        arms.push((dedup_keys, slice[1..].to_vec()));
                        let (_, default) =
                            collect_switch_arms(then_b, scrut, key_is_str, true, arms, seen)?;
                        return Some((slice.len(), default));
                    }
                    _ => {}
                }
                // Key matched `scrut` structurally, but shape wasn't convertible; undo `seen`
                // and treat `slice` as unmatched tail if `consume_all`.
                for k in &local_seen {
                    seen.remove(k);
                }
            }
        }
    }
    if consume_all {
        Some((slice.len(), Some(slice.to_vec())))
    } else {
        Some((0, None))
    }
}

fn try_build_chained_switch(slice: &[Stmt]) -> Option<(usize, Stmt)> {
    let Stmt::If(first_cond, _, _) = slice.first()? else {
        return None;
    };
    let (scrut, _, first_keys) = extract_eq_keys(first_cond)?;
    let key_is_str = first_keys[0].is_str();

    let mut arms: Vec<(Vec<SwitchKey>, Vec<Stmt>)> = Vec::new();
    let mut seen: HashSet<SwitchKey> = HashSet::new();
    let (consumed, default_body) =
        collect_switch_arms(slice, &scrut, key_is_str, false, &mut arms, &mut seen)?;
    if consumed == 0 || arms.len() < MIN_CHAIN_SWITCH_ARMS {
        return None;
    }

    // CRITICAL SOUNDNESS: NEVER wrap any branch body containing an unlabeled `break;` into a
    // `switch`, because the new `switch` would capture the `break;` intended for an outer loop
    // or outer `switch`!
    for (_, body) in &arms {
        if has_unlabeled_switch_break(body) {
            return None;
        }
    }
    if let Some(ref def) = default_body {
        if has_unlabeled_switch_break(def) {
            return None;
        }
    }

    let has_default = default_body.as_ref().is_some_and(|d| !d.is_empty());
    let total_arms = arms.len();
    let mut cases: Vec<jsast::SwitchCase> =
        Vec::with_capacity(total_arms + usize::from(has_default));
    for (idx, (keys, mut body)) in arms.into_iter().enumerate() {
        let is_last_case = !has_default && idx + 1 == total_arms;
        if !is_last_case && !stmts_definitely_exit(&body) {
            body.push(Stmt::Break(None));
        }
        cases.push(jsast::SwitchCase {
            tests: keys.into_iter().map(|k| k.to_expr()).collect(),
            body,
        });
    }
    if let Some(def) = default_body {
        if !def.is_empty() {
            cases.push(jsast::SwitchCase {
                tests: Vec::new(),
                body: def,
            });
        }
    }

    Some((consumed, Stmt::Switch(jsast::id(scrut), cases)))
}

fn lower_chained_eq_to_switch(stmts: &mut Vec<Stmt>) {
    let mut i = 0usize;
    while i < stmts.len() {
        // Top-down: first convert the outermost matching chain at `stmts[i..]`, then recurse into
        // the resulting `switch` arms (or child blocks of any non-matching statement).
        if let Some((consumed, switch_stmt)) = try_build_chained_switch(&stmts[i..]) {
            stmts.splice(i..i + consumed, std::iter::once(switch_stmt));
        }
        match &mut stmts[i] {
            Stmt::If(_, then_b, else_b) => {
                lower_chained_eq_to_switch(then_b);
                if let Some(eb) = else_b {
                    lower_chained_eq_to_switch(eb);
                }
            }
            Stmt::Switch(_, cases) => {
                for c in cases {
                    lower_chained_eq_to_switch(&mut c.body);
                }
            }
            Stmt::Block(b) | Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
                lower_chained_eq_to_switch(b);
            }
            Stmt::Labeled(_, inner) => {
                // Handle `lbl: if (x === "A") ...` directly while preserving the label `lbl:`.
                let mut tmp = vec![inner.as_ref().clone()];
                lower_chained_eq_to_switch(&mut tmp);
                if tmp.len() == 1 {
                    *inner.as_mut() = tmp.pop().unwrap();
                } else {
                    *inner.as_mut() = Stmt::Block(tmp);
                }
            }
            Stmt::TryCatch { body, handler, .. } => {
                lower_chained_eq_to_switch(body);
                lower_chained_eq_to_switch(handler);
            }
            Stmt::FunctionDecl { body, .. } => {
                lower_chained_eq_to_switch(body);
            }
            _ => {}
        }
        i += 1;
    }
}

// =============================================================================================
// 9. Liveness-based local slot coalescing (register coloring for uninitialized `let` locals)
// =============================================================================================

#[derive(Clone, Eq, PartialEq)]
struct LocalBitSet {
    words: Vec<u64>,
}

impl LocalBitSet {
    fn new(bits: usize) -> Self {
        Self {
            words: vec![0; bits.div_ceil(64)],
        }
    }

    fn set(&mut self, idx: usize) {
        self.words[idx >> 6] |= 1u64 << (idx & 63);
    }

    fn clear(&mut self, idx: usize) {
        self.words[idx >> 6] &= !(1u64 << (idx & 63));
    }

    fn contains(&self, idx: usize) -> bool {
        (self.words[idx >> 6] & (1u64 << (idx & 63))) != 0
    }

    fn union_with(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for (a, b) in self.words.iter_mut().zip(&other.words) {
            let next = *a | *b;
            if next != *a {
                *a = next;
                changed = true;
            }
        }
        changed
    }

    fn count_ones(&self) -> u32 {
        self.words.iter().map(|w| w.count_ones()).sum()
    }

    fn iter_ones(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(wi, &word)| {
            let base = wi << 6;
            BitIter { word, base }
        })
    }
}

struct BitIter {
    word: u64,
    base: usize,
}

impl Iterator for BitIter {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        if self.word == 0 {
            return None;
        }
        let tz = self.word.trailing_zeros() as usize;
        self.word &= self.word - 1;
        Some(self.base + tz)
    }
}

/// Liveness-based local slot coalescing over a finalized `Stmt::FunctionDecl`.
///
/// Must run AFTER `hoist_consts::run` and `bstr::rewrite` so single-assignment AST analyses
/// (`elide_ascii_only_bstr_in`, `elide_known_ascii_bstr_out`, `elide_proven_bstr_sub`,
/// `find_read_only_const_slice_init`, `split_bigint_props`) see pristine SSA-style locals first.
pub(crate) fn coalesce_disjoint_locals_decl(decl: &mut Stmt) {
    if let Stmt::FunctionDecl {
        name, params, body, ..
    } = decl
    {
        // Only coalesce directly self-recursive functions with huge local frames (>220 uninit
        // temporaries, e.g. `jmespath::eval` after mutual-recursion inlining) so deep recursion
        // stays within V8's stack budget without adding register/type-feedback pressure to
        // non-recursive hot functions (`marked`, `d3-hierarchy`, `diff`).
        if body_calls_self(name, body) {
            coalesce_disjoint_locals(params, body);
        }
    }
}

fn body_calls_self(self_name: &str, stmts: &[Stmt]) -> bool {
    fn expr_calls_self(self_name: &str, expr: &Expr) -> bool {
        match expr {
            Expr::Call(callee, args) => {
                if matches!(callee.as_ref(), Expr::Ident(id) if id == self_name) {
                    return true;
                }
                expr_calls_self(self_name, callee)
                    || args.iter().any(|a| expr_calls_self(self_name, a))
            }
            Expr::New(callee, args) => {
                expr_calls_self(self_name, callee)
                    || args.iter().any(|a| expr_calls_self(self_name, a))
            }
            Expr::Member(o, _) | Expr::Unary(_, o) => expr_calls_self(self_name, o),
            Expr::Index(o, i)
            | Expr::Binary(_, o, i)
            | Expr::Assign(o, i)
            | Expr::CompoundAssign(_, o, i) => {
                expr_calls_self(self_name, o) || expr_calls_self(self_name, i)
            }
            Expr::Cond(c, t, e) => {
                expr_calls_self(self_name, c)
                    || expr_calls_self(self_name, t)
                    || expr_calls_self(self_name, e)
            }
            Expr::Object(entries) => entries.iter().any(|(_, v)| expr_calls_self(self_name, v)),
            Expr::Array(items) | Expr::Seq(items) => {
                items.iter().any(|x| expr_calls_self(self_name, x))
            }
            Expr::Arrow(_, ArrowBody::Expr(e)) => expr_calls_self(self_name, e),
            Expr::Arrow(_, ArrowBody::Block(b)) => body_calls_self(self_name, b),
            _ => false,
        }
    }

    stmts.iter().any(|stmt| match stmt {
        Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Const(_, e) => expr_calls_self(self_name, e),
        Stmt::Return(Some(e)) | Stmt::Let(_, Some(e)) => expr_calls_self(self_name, e),
        Stmt::Lets(decls) => decls
            .iter()
            .any(|(_, init)| init.as_ref().is_some_and(|e| expr_calls_self(self_name, e))),
        Stmt::If(c, t, e) => {
            expr_calls_self(self_name, c)
                || body_calls_self(self_name, t)
                || e.as_ref().is_some_and(|b| body_calls_self(self_name, b))
        }
        Stmt::Switch(d, cases) => {
            expr_calls_self(self_name, d)
                || cases.iter().any(|c| {
                    c.tests.iter().any(|t| expr_calls_self(self_name, t))
                        || body_calls_self(self_name, &c.body)
                })
        }
        Stmt::Labeled(_, inner) => body_calls_self(self_name, std::slice::from_ref(inner.as_ref())),
        Stmt::LoopForever(_, b) | Stmt::Block(b) => body_calls_self(self_name, b),
        Stmt::While(c, b) => expr_calls_self(self_name, c) || body_calls_self(self_name, b),
        Stmt::TryCatch { body, handler, .. } => {
            body_calls_self(self_name, body) || body_calls_self(self_name, handler)
        }
        Stmt::FunctionDecl { body, .. } => body_calls_self(self_name, body),
        Stmt::Let(_, None)
        | Stmt::Return(None)
        | Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::Import { .. }
        | Stmt::Comment(_)
        | Stmt::Loc(_)
        | Stmt::Raw(_) => false,
    })
}

fn coalesce_disjoint_locals(params: &[String], body: &mut Vec<Stmt>) {
    // Only coalesce directly self-recursive functions with very large local frames (>220 uninit
    // temporaries) to keep deep-recursion V8 stack frames compact.
    coalesce_disjoint_locals_with_threshold(params, body, 221);
}

fn coalesce_disjoint_locals_with_threshold(
    params: &[String],
    body: &mut Vec<Stmt>,
    min_candidates: usize,
) {
    // 1. Collect leading top-level uninitialized `let` declarations.
    let param_set: HashSet<&str> = params.iter().map(String::as_str).collect();
    let mut prefix_end = 0usize;
    let mut decl_counts: HashMap<String, usize> = HashMap::new();
    let mut init_decls: HashSet<String> = HashSet::new();
    let mut candidate_order: Vec<String> = Vec::new();
    while prefix_end < body.len() {
        match &body[prefix_end] {
            Stmt::Let(n, init) => {
                *decl_counts.entry(n.clone()).or_default() += 1;
                if init.is_some() {
                    init_decls.insert(n.clone());
                } else {
                    candidate_order.push(n.clone());
                }
                prefix_end += 1;
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    *decl_counts.entry(n.clone()).or_default() += 1;
                    if init.is_some() {
                        init_decls.insert(n.clone());
                    } else {
                        candidate_order.push(n.clone());
                    }
                }
                prefix_end += 1;
            }
            Stmt::Const(n, _) => {
                *decl_counts.entry(n.clone()).or_default() += 1;
                init_decls.insert(n.clone());
                prefix_end += 1;
            }
            _ => break,
        }
    }
    if candidate_order.len() < min_candidates {
        return;
    }

    // 2. Scan the entire body for disqualifiers: `Raw` JS, inner re-declarations/catch-var
    //    shadowing, or captures inside nested `Arrow` / `FunctionDecl`. Also record any mentions
    //    in the leading initializers (which execute before the main body).
    struct SafetyScan {
        has_raw: bool,
        in_nested: bool,
        disqualified: HashSet<String>,
        used_in_body: HashSet<String>,
    }

    impl Visit for SafetyScan {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            if self.has_raw {
                return;
            }
            match stmt {
                Stmt::Raw(_) => self.has_raw = true,
                Stmt::Let(n, _) | Stmt::Const(n, _) => {
                    self.disqualified.insert(n.clone());
                    jsast::walk_stmt(self, stmt);
                }
                Stmt::Lets(decls) => {
                    for (n, _) in decls {
                        self.disqualified.insert(n.clone());
                    }
                    jsast::walk_stmt(self, stmt);
                }
                Stmt::TryCatch { binding, .. } => {
                    self.disqualified.insert(binding.clone());
                    jsast::walk_stmt(self, stmt);
                }
                Stmt::FunctionDecl { name, params, body } => {
                    self.disqualified.insert(name.clone());
                    self.disqualified.extend(params.iter().cloned());
                    let prev = self.in_nested;
                    self.in_nested = true;
                    self.visit_stmts(body);
                    self.in_nested = prev;
                }
                _ => jsast::walk_stmt(self, stmt),
            }
        }

        fn visit_expr(&mut self, expr: &Expr) {
            if self.has_raw {
                return;
            }
            match expr {
                Expr::Raw(_) => self.has_raw = true,
                Expr::Ident(n) => {
                    if self.in_nested {
                        self.disqualified.insert(n.clone());
                    } else {
                        self.used_in_body.insert(n.clone());
                    }
                }
                Expr::Arrow(params, _) => {
                    self.disqualified.extend(params.iter().cloned());
                    let prev = self.in_nested;
                    self.in_nested = true;
                    jsast::walk_expr(self, expr);
                    self.in_nested = prev;
                }
                _ => jsast::walk_expr(self, expr),
            }
        }
    }

    let mut scan = SafetyScan {
        has_raw: false,
        in_nested: true,
        disqualified: HashSet::new(),
        used_in_body: HashSet::new(),
    };
    // Any uninit candidate mentioned inside a leading decl initializer is disqualified.
    for stmt in &body[..prefix_end] {
        match stmt {
            Stmt::Let(_, Some(e)) | Stmt::Const(_, e) => scan.visit_expr(e),
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(e) = init {
                        scan.visit_expr(e);
                    }
                }
            }
            _ => {}
        }
    }
    scan.in_nested = false;
    scan.visit_stmts(&body[prefix_end..]);
    if scan.has_raw {
        return;
    }

    let mut candidates: Vec<String> = Vec::new();
    let mut cand_idx: HashMap<String, usize> = HashMap::new();
    for name in candidate_order {
        if param_set.contains(name.as_str())
            || init_decls.contains(&name)
            || decl_counts.get(&name).copied().unwrap_or(0) != 1
            || scan.disqualified.contains(&name)
            || !scan.used_in_body.contains(&name)
        {
            continue;
        }
        let idx = candidates.len();
        cand_idx.insert(name.clone(), idx);
        candidates.push(name);
    }
    let n = candidates.len();
    if n < min_candidates {
        return;
    }

    // 3. Backward dataflow liveness + interference graph construction.
    struct Liveness<'a> {
        cand_idx: &'a HashMap<String, usize>,
        n: usize,
        interferes: Vec<LocalBitSet>,
        unlabeled_break: Vec<LocalBitSet>,
        unlabeled_continue: Vec<LocalBitSet>,
        labeled_break: HashMap<String, LocalBitSet>,
        labeled_continue: HashMap<String, LocalBitSet>,
        handler_live: Vec<LocalBitSet>,
        bailed: bool,
    }

    impl<'a> Liveness<'a> {
        fn add_def(&mut self, v: usize, live: &LocalBitSet) {
            for u in live.iter_ones() {
                if u != v {
                    self.interferes[v].set(u);
                    self.interferes[u].set(v);
                }
            }
            for h in &self.handler_live {
                for u in h.iter_ones() {
                    if u != v {
                        self.interferes[v].set(u);
                        self.interferes[u].set(v);
                    }
                }
            }
        }

        fn union_handlers(&self, live: &mut LocalBitSet) {
            for h in &self.handler_live {
                live.union_with(h);
            }
        }

        fn walk_stmts(&mut self, stmts: &[Stmt], mut live: LocalBitSet) -> LocalBitSet {
            for s in stmts.iter().rev() {
                live = self.walk_stmt(s, live, None);
                if !self.handler_live.is_empty() {
                    self.union_handlers(&mut live);
                }
                if self.bailed {
                    return LocalBitSet::new(self.n);
                }
            }
            live
        }

        fn walk_stmt(
            &mut self,
            stmt: &Stmt,
            mut live: LocalBitSet,
            outer_label: Option<&str>,
        ) -> LocalBitSet {
            match stmt {
                Stmt::ExprStmt(e) => {
                    self.walk_expr(e, &mut live);
                    live
                }
                Stmt::Let(_, Some(e)) | Stmt::Const(_, e) => {
                    self.walk_expr(e, &mut live);
                    live
                }
                Stmt::Lets(decls) => {
                    for (_, init) in decls.iter().rev() {
                        if let Some(e) = init {
                            self.walk_expr(e, &mut live);
                        }
                    }
                    live
                }
                Stmt::Return(opt) => {
                    let mut out = LocalBitSet::new(self.n);
                    if let Some(e) = opt {
                        self.walk_expr(e, &mut out);
                    }
                    out
                }
                Stmt::Throw(e) => {
                    let mut out = LocalBitSet::new(self.n);
                    self.union_handlers(&mut out);
                    self.walk_expr(e, &mut out);
                    out
                }
                Stmt::Break(None) => match self.unlabeled_break.last() {
                    Some(target) => target.clone(),
                    None => {
                        self.bailed = true;
                        LocalBitSet::new(self.n)
                    }
                },
                Stmt::Break(Some(lbl)) => match self.labeled_break.get(lbl) {
                    Some(target) => target.clone(),
                    None => {
                        self.bailed = true;
                        LocalBitSet::new(self.n)
                    }
                },
                Stmt::Continue(None) => match self.unlabeled_continue.last() {
                    Some(target) => target.clone(),
                    None => {
                        self.bailed = true;
                        LocalBitSet::new(self.n)
                    }
                },
                Stmt::Continue(Some(lbl)) => match self.labeled_continue.get(lbl) {
                    Some(target) => target.clone(),
                    None => {
                        self.bailed = true;
                        LocalBitSet::new(self.n)
                    }
                },
                Stmt::Block(stmts) => self.walk_stmts(stmts, live),
                Stmt::If(cond, then_b, else_b) => {
                    let then_in = self.walk_stmts(then_b, live.clone());
                    let else_in = match else_b {
                        Some(eb) => self.walk_stmts(eb, live),
                        None => live,
                    };
                    let mut merged = then_in;
                    merged.union_with(&else_in);
                    self.walk_expr(cond, &mut merged);
                    merged
                }
                Stmt::Labeled(lbl, inner) => {
                    if matches!(
                        inner.as_ref(),
                        Stmt::While(..) | Stmt::LoopForever(None, ..)
                    ) {
                        return self.walk_stmt(inner, live, Some(lbl.as_str()));
                    }
                    let prev = self.labeled_break.insert(lbl.clone(), live.clone());
                    let out = self.walk_stmt(inner, live, None);
                    match prev {
                        Some(p) => {
                            self.labeled_break.insert(lbl.clone(), p);
                        }
                        None => {
                            self.labeled_break.remove(lbl);
                        }
                    }
                    out
                }
                Stmt::LoopForever(own_lbl, stmts) => {
                    let lbl = own_lbl.as_deref().or(outer_label);
                    self.unlabeled_break.push(live.clone());
                    let prev_lb =
                        lbl.and_then(|l| self.labeled_break.insert(l.to_owned(), live.clone()));
                    let mut head = LocalBitSet::new(self.n);
                    let mut converged = false;
                    for _ in 0..16 {
                        self.unlabeled_continue.push(head.clone());
                        let prev_lc = lbl
                            .and_then(|l| self.labeled_continue.insert(l.to_owned(), head.clone()));
                        let next_head = self.walk_stmts(stmts, head.clone());
                        self.unlabeled_continue.pop();
                        if let Some(l) = lbl {
                            match prev_lc {
                                Some(p) => {
                                    self.labeled_continue.insert(l.to_owned(), p);
                                }
                                None => {
                                    self.labeled_continue.remove(l);
                                }
                            }
                        }
                        if self.bailed {
                            break;
                        }
                        if !head.union_with(&next_head) {
                            converged = true;
                            break;
                        }
                    }
                    self.unlabeled_break.pop();
                    if let Some(l) = lbl {
                        match prev_lb {
                            Some(p) => {
                                self.labeled_break.insert(l.to_owned(), p);
                            }
                            None => {
                                self.labeled_break.remove(l);
                            }
                        }
                    }
                    if !converged {
                        self.bailed = true;
                    }
                    head
                }
                Stmt::While(cond, stmts) => {
                    let lbl = outer_label;
                    self.unlabeled_break.push(live.clone());
                    let prev_lb =
                        lbl.and_then(|l| self.labeled_break.insert(l.to_owned(), live.clone()));
                    let mut head = live.clone();
                    self.walk_expr(cond, &mut head);
                    let mut converged = false;
                    for _ in 0..16 {
                        self.unlabeled_continue.push(head.clone());
                        let prev_lc = lbl
                            .and_then(|l| self.labeled_continue.insert(l.to_owned(), head.clone()));
                        let mut body_in = self.walk_stmts(stmts, head.clone());
                        self.unlabeled_continue.pop();
                        if let Some(l) = lbl {
                            match prev_lc {
                                Some(p) => {
                                    self.labeled_continue.insert(l.to_owned(), p);
                                }
                                None => {
                                    self.labeled_continue.remove(l);
                                }
                            }
                        }
                        if self.bailed {
                            break;
                        }
                        body_in.union_with(&live);
                        self.walk_expr(cond, &mut body_in);
                        if !head.union_with(&body_in) {
                            converged = true;
                            break;
                        }
                    }
                    self.unlabeled_break.pop();
                    if let Some(l) = lbl {
                        match prev_lb {
                            Some(p) => {
                                self.labeled_break.insert(l.to_owned(), p);
                            }
                            None => {
                                self.labeled_break.remove(l);
                            }
                        }
                    }
                    if !converged {
                        self.bailed = true;
                    }
                    head
                }
                Stmt::Switch(disc, cases) => {
                    let has_default = cases.iter().any(|c| c.tests.is_empty());
                    self.unlabeled_break.push(live.clone());
                    let mut fallthrough = live.clone();
                    let mut dispatch = if has_default {
                        LocalBitSet::new(self.n)
                    } else {
                        live
                    };
                    for c in cases.iter().rev() {
                        let case_in = self.walk_stmts(&c.body, fallthrough);
                        fallthrough = case_in.clone();
                        dispatch.union_with(&case_in);
                        for t in c.tests.iter().rev() {
                            self.walk_expr(t, &mut dispatch);
                        }
                    }
                    self.unlabeled_break.pop();
                    self.walk_expr(disc, &mut dispatch);
                    dispatch
                }
                Stmt::TryCatch {
                    body,
                    binding: _,
                    handler,
                } => {
                    let handler_in = self.walk_stmts(handler, live.clone());
                    self.handler_live.push(handler_in.clone());
                    let mut body_in = self.walk_stmts(body, live);
                    self.handler_live.pop();
                    body_in.union_with(&handler_in);
                    body_in
                }
                Stmt::Let(_, None)
                | Stmt::FunctionDecl { .. }
                | Stmt::Import { .. }
                | Stmt::Comment(_)
                | Stmt::Loc(_)
                | Stmt::Raw(_) => live,
            }
        }

        fn walk_expr(&mut self, expr: &Expr, live: &mut LocalBitSet) {
            match expr {
                Expr::Ident(n) => {
                    if let Some(&idx) = self.cand_idx.get(n) {
                        live.set(idx);
                    }
                }
                Expr::Assign(lhs, rhs) => {
                    if let Expr::Ident(n) = lhs.as_ref() {
                        if let Some(&idx) = self.cand_idx.get(n) {
                            self.add_def(idx, live);
                            live.clear(idx);
                        }
                        self.walk_expr(rhs, live);
                    } else {
                        // Non-ident LHS (`arr[i] = rhs`, `obj.p = rhs`): JS evaluates LHS before RHS.
                        self.walk_expr(rhs, live);
                        self.walk_expr(lhs, live);
                    }
                }
                Expr::CompoundAssign(_, lhs, rhs) => {
                    if let Expr::Ident(n) = lhs.as_ref() {
                        if let Some(&idx) = self.cand_idx.get(n) {
                            self.add_def(idx, live);
                            self.walk_expr(rhs, live);
                            live.set(idx);
                        } else {
                            self.walk_expr(rhs, live);
                        }
                    } else {
                        self.walk_expr(rhs, live);
                        self.walk_expr(lhs, live);
                    }
                }
                Expr::Binary(BinOp::And | BinOp::Or, l, r) => {
                    let mut r_live = live.clone();
                    self.walk_expr(r, &mut r_live);
                    live.union_with(&r_live);
                    self.walk_expr(l, live);
                }
                Expr::Cond(c, t, e) => {
                    let mut t_live = live.clone();
                    self.walk_expr(t, &mut t_live);
                    let mut e_live = live.clone();
                    self.walk_expr(e, &mut e_live);
                    *live = t_live;
                    live.union_with(&e_live);
                    self.walk_expr(c, live);
                }
                Expr::Binary(_, l, r) => {
                    self.walk_expr(r, live);
                    self.walk_expr(l, live);
                }
                Expr::Index(o, i) => {
                    self.walk_expr(i, live);
                    self.walk_expr(o, live);
                }
                Expr::Member(o, _) | Expr::Unary(_, o) => self.walk_expr(o, live),
                Expr::Call(c, args) | Expr::New(c, args) => {
                    for a in args.iter().rev() {
                        self.walk_expr(a, live);
                    }
                    self.walk_expr(c, live);
                }
                Expr::Object(entries) => {
                    for (_, v) in entries.iter().rev() {
                        self.walk_expr(v, live);
                    }
                }
                Expr::Array(items) | Expr::Seq(items) => {
                    for x in items.iter().rev() {
                        self.walk_expr(x, live);
                    }
                }
                Expr::Arrow(..)
                | Expr::Raw(_)
                | Expr::Num(_)
                | Expr::RawNum(_)
                | Expr::BigInt(_)
                | Expr::Str(_)
                | Expr::Bool(_)
                | Expr::Undefined
                | Expr::Null => {}
            }
        }
    }

    let mut liveness = Liveness {
        cand_idx: &cand_idx,
        n,
        interferes: vec![LocalBitSet::new(n); n],
        unlabeled_break: Vec::new(),
        unlabeled_continue: Vec::new(),
        labeled_break: HashMap::new(),
        labeled_continue: HashMap::new(),
        handler_live: Vec::new(),
        bailed: false,
    };
    let live_at_entry = liveness.walk_stmts(&body[prefix_end..], LocalBitSet::new(n));
    if liveness.bailed {
        return;
    }

    // 4. Classify each candidate into a coarse JS type bucket from its RHS writes so coloring
    //    never mixes unrelated runtime types (e.g. Number/Bool/String/Object) in one V8 slot.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ValueBucket {
        Unseen,
        Bool,
        Number,
        String,
        SpillErr,
        Other,
    }

    impl ValueBucket {
        fn join(self, other: Self) -> Self {
            if self == ValueBucket::Unseen {
                other
            } else if other == ValueBucket::Unseen || self == other {
                self
            } else {
                ValueBucket::Other
            }
        }
    }

    fn expr_bucket(expr: &Expr) -> ValueBucket {
        match expr {
            Expr::Bool(_) => ValueBucket::Bool,
            Expr::Num(_) | Expr::RawNum(_) => ValueBucket::Number,
            Expr::Str(_) => ValueBucket::String,
            Expr::Ident(n) if n.starts_with("$r") => ValueBucket::SpillErr,
            Expr::Unary(UnOp::Not, _) => ValueBucket::Bool,
            Expr::Unary(UnOp::TypeOf, _) => ValueBucket::String,
            Expr::Unary(UnOp::Neg | UnOp::BitNot, _) => ValueBucket::Number,
            Expr::Binary(
                BinOp::StrictEq | BinOp::StrictNe | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge,
                _,
                _,
            ) => ValueBucket::Bool,
            Expr::Binary(
                BinOp::Sub
                | BinOp::Mul
                | BinOp::Div
                | BinOp::Rem
                | BinOp::BitAnd
                | BinOp::BitOr
                | BinOp::BitXor
                | BinOp::Shl
                | BinOp::Shr
                | BinOp::UShr,
                _,
                _,
            ) => ValueBucket::Number,
            Expr::Call(c, _) => {
                if let Expr::Member(o, m) = c.as_ref() {
                    if m == "charCodeAt" || (matches!(o.as_ref(), Expr::Ident(id) if id == "Math"))
                    {
                        return ValueBucket::Number;
                    }
                    if m == "isArray" || m == "startsWith" || m == "endsWith" {
                        return ValueBucket::Bool;
                    }
                } else if let Expr::Ident(id) = c.as_ref() {
                    if id == "Number" {
                        return ValueBucket::Number;
                    }
                    if id == "Boolean" {
                        return ValueBucket::Bool;
                    }
                }
                ValueBucket::Other
            }
            Expr::Cond(_, t, e) => expr_bucket(t).join(expr_bucket(e)),
            _ => ValueBucket::Other,
        }
    }

    struct BucketScan<'a> {
        cand_idx: &'a HashMap<String, usize>,
        buckets: &'a mut [ValueBucket],
    }
    impl Visit for BucketScan<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            if !matches!(stmt, Stmt::FunctionDecl { .. }) {
                jsast::walk_stmt(self, stmt);
            }
        }
        fn visit_expr(&mut self, expr: &Expr) {
            match expr {
                Expr::Arrow(..) => {}
                Expr::Assign(lhs, rhs) => {
                    if let Expr::Ident(n) = lhs.as_ref() {
                        if let Some(&idx) = self.cand_idx.get(n) {
                            self.buckets[idx] = self.buckets[idx].join(expr_bucket(rhs));
                        }
                    } else {
                        self.visit_expr(lhs);
                    }
                    self.visit_expr(rhs);
                }
                Expr::CompoundAssign(_, lhs, rhs) => {
                    if let Expr::Ident(n) = lhs.as_ref() {
                        if let Some(&idx) = self.cand_idx.get(n) {
                            self.buckets[idx] = self.buckets[idx].join(ValueBucket::Number);
                        }
                    } else {
                        self.visit_expr(lhs);
                    }
                    self.visit_expr(rhs);
                }
                _ => jsast::walk_expr(self, expr),
            }
        }
    }

    let mut buckets = vec![ValueBucket::Unseen; n];
    BucketScan {
        cand_idx: &cand_idx,
        buckets: &mut buckets,
    }
    .visit_stmts(&body[prefix_end..]);
    for b in &mut buckets {
        if *b == ValueBucket::Unseen {
            *b = ValueBucket::Other;
        }
    }

    // 5. Greedy Welsh-Powell graph coloring strictly WITHIN each `ValueBucket`, over non-entry-live
    //    candidates. Any variable live at function entry may read its initial `undefined` value, so
    //    keep it pinned in its own dedicated slot.
    let mut order: Vec<usize> = (0..n).filter(|&i| !live_at_entry.contains(i)).collect();
    order.sort_by(|&a, &b| {
        liveness.interferes[b]
            .count_ones()
            .cmp(&liveness.interferes[a].count_ones())
            .then_with(|| a.cmp(&b))
    });

    let mut color_of: Vec<Option<usize>> = vec![None; n];
    let mut color_name: Vec<String> = Vec::new();
    let mut color_bucket: Vec<ValueBucket> = Vec::new();
    let mut used_colors: Vec<usize> = Vec::new();
    for v in order {
        let vb = buckets[v];
        used_colors.clear();
        for u in liveness.interferes[v].iter_ones() {
            if let Some(c) = color_of[u] {
                used_colors.push(c);
            }
        }
        let chosen = (0..color_name.len())
            .find(|&c| color_bucket[c] == vb && !used_colors.contains(&c))
            .unwrap_or_else(|| {
                let c = color_name.len();
                color_name.push(candidates[v].clone());
                color_bucket.push(vb);
                c
            });
        color_of[v] = Some(chosen);
    }

    let mut rename: HashMap<String, String> = HashMap::new();
    for (idx, name) in candidates.iter().enumerate() {
        if let Some(c) = color_of[idx] {
            let target = &color_name[c];
            if target != name {
                rename.insert(name.clone(), target.clone());
            }
        }
    }
    if rename.is_empty() {
        return;
    }

    // 5. Apply renames across the body and prune coalesced uninitialized declarations.
    struct Renamer<'a>(&'a HashMap<String, String>);
    impl VisitMut for Renamer<'_> {
        fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
            if !matches!(stmt, Stmt::FunctionDecl { .. }) {
                jsast::walk_stmt_mut(self, stmt);
            }
        }
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            match expr {
                Expr::Arrow(..) => {}
                Expr::Ident(n) => {
                    if let Some(target) = self.0.get(n) {
                        *n = target.clone();
                    }
                }
                _ => jsast::walk_expr_mut(self, expr),
            }
        }
    }

    Renamer(&rename).visit_stmts_mut(&mut body[prefix_end..]);
    let mut i = 0;
    while i < prefix_end {
        let remove = match &mut body[i] {
            Stmt::Lets(decls) => {
                decls.retain(|(n, init)| init.is_some() || !rename.contains_key(n));
                decls.is_empty()
            }
            Stmt::Let(n, None) => rename.contains_key(n),
            _ => false,
        };
        if remove {
            body.remove(i);
            prefix_end -= 1;
        } else {
            i += 1;
        }
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
            printed.contains(i64_pair::HELPER_UDIVREM),
            "expected specialized udivrem: {printed}"
        );
        assert!(
            !printed.contains("$__mires_i64_div("),
            "expected both div calls fused: {printed}"
        );
    }

    #[test]
    fn fuses_specialized_i64_srem_sdiv_and_urem_udiv() {
        let mut decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: vec![
                "al".to_owned(),
                "ah".to_owned(),
                "bl".to_owned(),
                "bh".to_owned(),
            ],
            body: vec![
                jsast::lets(vec![
                    ("rl".to_owned(), None),
                    ("rh".to_owned(), None),
                    ("ql".to_owned(), None),
                    ("qh".to_owned(), None),
                    ("url".to_owned(), None),
                    ("urh".to_owned(), None),
                    ("uql".to_owned(), None),
                    ("uqh".to_owned(), None),
                ]),
                jsast::assign_stmt(
                    jsast::id("rl"),
                    jsast::call(
                        jsast::id(i64_pair::HELPER_SREM),
                        vec![
                            jsast::id("al"),
                            jsast::id("ah"),
                            jsast::id("bl"),
                            jsast::id("bh"),
                        ],
                    ),
                ),
                jsast::assign_stmt(jsast::id("rh"), jsast::id(i64_pair::HELPER_HI)),
                jsast::assign_stmt(
                    jsast::id("ql"),
                    jsast::call(
                        jsast::id(i64_pair::HELPER_SDIV),
                        vec![
                            jsast::id("al"),
                            jsast::id("ah"),
                            jsast::id("bl"),
                            jsast::id("bh"),
                        ],
                    ),
                ),
                jsast::assign_stmt(jsast::id("qh"), jsast::id(i64_pair::HELPER_HI)),
                jsast::assign_stmt(
                    jsast::id("uql"),
                    jsast::call(
                        jsast::id(i64_pair::HELPER_UDIV),
                        vec![
                            jsast::id("al"),
                            jsast::id("ah"),
                            jsast::id("bl"),
                            jsast::id("bh"),
                        ],
                    ),
                ),
                jsast::assign_stmt(jsast::id("uqh"), jsast::id(i64_pair::HELPER_HI)),
                jsast::assign_stmt(
                    jsast::id("url"),
                    jsast::call(
                        jsast::id(i64_pair::HELPER_UREM),
                        vec![
                            jsast::id("al"),
                            jsast::id("ah"),
                            jsast::id("bl"),
                            jsast::id("bh"),
                        ],
                    ),
                ),
                jsast::assign_stmt(jsast::id("urh"), jsast::id(i64_pair::HELPER_HI)),
                Stmt::Return(Some(Expr::Array(vec![
                    jsast::id("rl"),
                    jsast::id("rh"),
                    jsast::id("ql"),
                    jsast::id("qh"),
                    jsast::id("url"),
                    jsast::id("urh"),
                    jsast::id("uql"),
                    jsast::id("uqh"),
                ]))),
            ],
        };
        run_speed_post_opts(&mut decl);
        let printed = jsast::stmt_to_string(&decl);
        assert!(
            printed.contains(i64_pair::HELPER_SDIVREM)
                && printed.contains(i64_pair::HELPER_UDIVREM),
            "expected specialized sdivrem & udivrem: {printed}"
        );
        assert!(
            !printed.contains(&format!("{}(", i64_pair::HELPER_SREM))
                && !printed.contains(&format!("{}(", i64_pair::HELPER_SDIV))
                && !printed.contains(&format!("{}(", i64_pair::HELPER_UREM))
                && !printed.contains(&format!("{}(", i64_pair::HELPER_UDIV)),
            "expected all 4 specialized calls fused: {printed}"
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

    #[test]
    fn folds_unsigned_range_checks_and_rejects_unsound_floats_and_nan() {
        // 1. Positive cases: bitwise masked int, UShr strict bounds, negated `||`, lower bound 0,
        //    in-bounds `charCodeAt` in counted loop, single-read temp `if/else`, unbounded
        //    `charCodeAt` with `A >= 1` (`| 0` guarded), and `charCodeAt` promoted by `< 128`.
        let mut pos = Stmt::FunctionDecl {
            name: "ranges_ok".to_owned(),
            params: vec!["a".to_owned(), "s".to_owned(), "oob".to_owned()],
            body: vec![
                jsast::lets(vec![
                    ("x".to_owned(), None),
                    ("y".to_owned(), None),
                    ("r1".to_owned(), None),
                    ("r2".to_owned(), None),
                    ("r3".to_owned(), None),
                    ("r4".to_owned(), None),
                    ("r5".to_owned(), None),
                    ("k".to_owned(), None),
                    ("m".to_owned(), None),
                    ("c".to_owned(), None),
                    ("ae".to_owned(), None),
                    ("r_loop".to_owned(), None),
                    ("u".to_owned(), None),
                    ("r_oob_and".to_owned(), None),
                    ("r_prom".to_owned(), None),
                ]),
                // x = a & 255 (NonNegI31)
                jsast::assign_stmt(
                    jsast::id("x"),
                    jsast::binary(BinOp::BitAnd, jsast::id("a"), Expr::Num(255.0)),
                ),
                // y = a >>> 1 (NonNegI31)
                jsast::assign_stmt(
                    jsast::id("y"),
                    jsast::binary(BinOp::UShr, jsast::id("a"), Expr::Num(1.0)),
                ),
                // r1 = x >= 65 && x <= 90 -> x - 65 >>> 0 <= 25
                jsast::assign_stmt(
                    jsast::id("r1"),
                    jsast::binary(
                        BinOp::And,
                        jsast::binary(BinOp::Ge, jsast::id("x"), Expr::Num(65.0)),
                        jsast::binary(BinOp::Le, jsast::id("x"), Expr::Num(90.0)),
                    ),
                ),
                // r2 = 97 <= x && 122 >= x -> x - 97 >>> 0 <= 25
                jsast::assign_stmt(
                    jsast::id("r2"),
                    jsast::binary(
                        BinOp::And,
                        jsast::binary(BinOp::Le, Expr::Num(97.0), jsast::id("x")),
                        jsast::binary(BinOp::Ge, Expr::Num(122.0), jsast::id("x")),
                    ),
                ),
                // r3 = y > 47 && y < 58 -> y - 48 >>> 0 <= 9
                jsast::assign_stmt(
                    jsast::id("r3"),
                    jsast::binary(
                        BinOp::And,
                        jsast::binary(BinOp::Gt, jsast::id("y"), Expr::Num(47.0)),
                        jsast::binary(BinOp::Lt, jsast::id("y"), Expr::Num(58.0)),
                    ),
                ),
                // r4 = x < 65 || x > 90 -> x - 65 >>> 0 > 25
                jsast::assign_stmt(
                    jsast::id("r4"),
                    jsast::binary(
                        BinOp::Or,
                        jsast::binary(BinOp::Lt, jsast::id("x"), Expr::Num(65.0)),
                        jsast::binary(BinOp::Gt, jsast::id("x"), Expr::Num(90.0)),
                    ),
                ),
                // r5 = x >= 0 && x <= 127 -> x <= 127
                jsast::assign_stmt(
                    jsast::id("r5"),
                    jsast::binary(
                        BinOp::And,
                        jsast::binary(BinOp::Ge, jsast::id("x"), Expr::Num(0.0)),
                        jsast::binary(BinOp::Le, jsast::id("x"), Expr::Num(127.0)),
                    ),
                ),
                // In-bounds counted loop `charCodeAt` + single-read temp `if/else` (`jmespath` idiom)
                jsast::assign_stmt(jsast::id("k"), jsast::member(jsast::id("s"), "length")),
                jsast::assign_stmt(jsast::id("m"), Expr::Num(0.0)),
                Stmt::LoopForever(
                    None,
                    vec![Stmt::If(
                        jsast::binary(BinOp::Lt, jsast::id("m"), jsast::id("k")),
                        vec![
                            jsast::assign_stmt(
                                jsast::id("c"),
                                jsast::method_call(
                                    jsast::id("s"),
                                    "charCodeAt",
                                    vec![jsast::id("m")],
                                ),
                            ),
                            jsast::assign_stmt(
                                jsast::id("ae"),
                                jsast::binary(BinOp::Le, Expr::Num(65.0), jsast::id("c")),
                            ),
                            Stmt::If(
                                jsast::id("ae"),
                                vec![jsast::assign_stmt(
                                    jsast::id("r_loop"),
                                    jsast::binary(BinOp::Le, jsast::id("c"), Expr::Num(90.0)),
                                )],
                                Some(vec![jsast::assign_stmt(
                                    jsast::id("r_loop"),
                                    Expr::Bool(false),
                                )]),
                            ),
                            jsast::assign_stmt(
                                jsast::id("m"),
                                jsast::binary(
                                    BinOp::UShr,
                                    jsast::binary(BinOp::Add, jsast::id("m"), Expr::Num(1.0)),
                                    Expr::Num(0.0),
                                ),
                            ),
                        ],
                        Some(vec![Stmt::Break(None)]),
                    )],
                ),
                // Unbounded `u = s.charCodeAt(oob)`: `48 <= u && u <= 57` uses `(u | 0) - 48 >>> 0 <= 9`,
                // and after `if (u < 128)` `u` is promoted non-NaN -> `u - 9 >>> 0 <= 4`.
                jsast::assign_stmt(
                    jsast::id("u"),
                    jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::id("oob")]),
                ),
                Stmt::If(
                    jsast::binary(BinOp::Le, Expr::Num(48.0), jsast::id("u")),
                    vec![jsast::assign_stmt(
                        jsast::id("r_oob_and"),
                        jsast::binary(BinOp::Le, jsast::id("u"), Expr::Num(57.0)),
                    )],
                    Some(vec![jsast::assign_stmt(
                        jsast::id("r_oob_and"),
                        Expr::Bool(false),
                    )]),
                ),
                Stmt::If(
                    jsast::binary(BinOp::Lt, jsast::id("u"), Expr::Num(128.0)),
                    vec![jsast::assign_stmt(
                        jsast::id("r_prom"),
                        jsast::binary(
                            BinOp::And,
                            jsast::binary(BinOp::Ge, jsast::id("u"), Expr::Num(9.0)),
                            jsast::binary(BinOp::Le, jsast::id("u"), Expr::Num(13.0)),
                        ),
                    )],
                    None,
                ),
                Stmt::Return(Some(Expr::Array(vec![
                    jsast::id("r1"),
                    jsast::id("r2"),
                    jsast::id("r3"),
                    jsast::id("r4"),
                    jsast::id("r5"),
                    jsast::id("r_loop"),
                    jsast::id("r_oob_and"),
                    jsast::id("r_prom"),
                ]))),
            ],
        };
        run_speed_post_opts(&mut pos);
        let p = jsast::stmt_to_string(&pos);
        assert!(
            p.contains("r1 = (x - 65 >>> 0) <= 25;") || p.contains("r1 = x - 65 >>> 0 <= 25;"),
            "r1: {p}"
        );
        assert!(
            p.contains("r2 = (x - 97 >>> 0) <= 25;") || p.contains("r2 = x - 97 >>> 0 <= 25;"),
            "r2: {p}"
        );
        assert!(
            p.contains("r3 = (y - 48 >>> 0) <= 9;") || p.contains("r3 = y - 48 >>> 0 <= 9;"),
            "r3: {p}"
        );
        assert!(
            p.contains("r4 = (x - 65 >>> 0) > 25;") || p.contains("r4 = x - 65 >>> 0 > 25;"),
            "r4: {p}"
        );
        assert!(p.contains("r5 = x <= 127;"), "r5: {p}");
        assert!(
            (p.contains("r_loop = c - 65 >>> 0 <= 25;")
                || p.contains("r_loop = (c - 65 >>> 0) <= 25;"))
                && !p.contains("ae ="),
            "in-bounds charCodeAt should fold without |0 and drop dead ae: {p}"
        );
        assert!(
            p.contains("r_oob_and = (u | 0) - 48 >>> 0 <= 9;")
                || p.contains("r_oob_and = ((u | 0) - 48 >>> 0) <= 9;"),
            "unbounded charCodeAt && must guard NaN via (u | 0): {p}"
        );
        assert!(
            p.contains("r_prom = u - 9 >>> 0 <= 4;") || p.contains("r_prom = (u - 9 >>> 0) <= 4;"),
            "promoted < 128 charCodeAt should fold without |0: {p}"
        );

        // 2. Negative soundness cases: arbitrary param (could be 64.5 / NaN / 2^32+65),
        //    float division result, unbounded charCodeAt in `||`, and unbounded charCodeAt at A=0.
        let mut neg = Stmt::FunctionDecl {
            name: "ranges_reject".to_owned(),
            params: vec!["p".to_owned(), "s".to_owned(), "oob".to_owned()],
            body: vec![
                jsast::lets(vec![
                    ("f".to_owned(), None),
                    ("u".to_owned(), None),
                    ("b1".to_owned(), None),
                    ("b2".to_owned(), None),
                    ("b3".to_owned(), None),
                    ("b4".to_owned(), None),
                ]),
                jsast::assign_stmt(
                    jsast::id("f"),
                    jsast::binary(BinOp::Div, jsast::id("p"), Expr::Num(2.0)),
                ),
                jsast::assign_stmt(
                    jsast::id("u"),
                    jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::id("oob")]),
                ),
                // Arbitrary param `p`
                jsast::assign_stmt(
                    jsast::id("b1"),
                    jsast::binary(
                        BinOp::And,
                        jsast::binary(BinOp::Ge, jsast::id("p"), Expr::Num(65.0)),
                        jsast::binary(BinOp::Le, jsast::id("p"), Expr::Num(90.0)),
                    ),
                ),
                // Float `f = p / 2`
                jsast::assign_stmt(
                    jsast::id("b2"),
                    jsast::binary(
                        BinOp::And,
                        jsast::binary(BinOp::Ge, jsast::id("f"), Expr::Num(1.0)),
                        jsast::binary(BinOp::Le, jsast::id("f"), Expr::Num(10.0)),
                    ),
                ),
                // Possibly-NaN `u` in negated `||` (NaN < 65 || NaN > 90 is false!)
                jsast::assign_stmt(
                    jsast::id("b3"),
                    jsast::binary(
                        BinOp::Or,
                        jsast::binary(BinOp::Lt, jsast::id("u"), Expr::Num(65.0)),
                        jsast::binary(BinOp::Gt, jsast::id("u"), Expr::Num(90.0)),
                    ),
                ),
                // Possibly-NaN `u` at A == 0 (NaN >= 0 && NaN <= 9 is false, (NaN|0) <= 9 would be true!)
                jsast::assign_stmt(
                    jsast::id("b4"),
                    jsast::binary(
                        BinOp::And,
                        jsast::binary(BinOp::Ge, jsast::id("u"), Expr::Num(0.0)),
                        jsast::binary(BinOp::Le, jsast::id("u"), Expr::Num(9.0)),
                    ),
                ),
                Stmt::Return(Some(Expr::Array(vec![
                    jsast::id("b1"),
                    jsast::id("b2"),
                    jsast::id("b3"),
                    jsast::id("b4"),
                ]))),
            ],
        };
        run_speed_post_opts(&mut neg);
        let np = jsast::stmt_to_string(&neg);
        assert!(
            !np.contains(">>> 0"),
            "unsound float/arbitrary/possibly-NaN range checks must NOT fold: {np}"
        );
    }

    #[test]
    fn lowers_dense_ascii_luts_and_rejects_non_numeric_and_sparse() {
        let eq_chain = |var: &str, bytes: &[u8], op: BinOp, cmp: BinOp| -> Expr {
            let mut iter = bytes
                .iter()
                .map(|&b| jsast::binary(cmp, jsast::id(var), Expr::Num(f64::from(b))));
            let first = iter.next().unwrap();
            iter.fold(first, |acc, next| jsast::binary(op, acc, next))
        };

        // 1. Positive: `||` chain, `&&` chain, boolean switch, control-flow switch, and 256-byte switch.
        let mut pos = Stmt::FunctionDecl {
            name: "scan_delim".to_owned(),
            params: vec!["s".to_owned(), "i".to_owned()],
            body: vec![
                jsast::lets(vec![
                    ("c".to_owned(), None),
                    ("d1".to_owned(), None),
                    ("d2".to_owned(), None),
                    ("d3".to_owned(), None),
                    ("hit".to_owned(), None),
                ]),
                jsast::assign_stmt(
                    jsast::id("c"),
                    jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::id("i")]),
                ),
                jsast::assign_stmt(
                    jsast::id("d1"),
                    eq_chain(
                        "c",
                        &[33, 42, 60, 91, 92, 95, 96, 126],
                        BinOp::Or,
                        BinOp::StrictEq,
                    ),
                ),
                jsast::assign_stmt(
                    jsast::id("d2"),
                    eq_chain("c", &[33, 42, 60, 91, 92, 95], BinOp::And, BinOp::StrictNe),
                ),
                Stmt::Switch(
                    jsast::id("c"),
                    vec![
                        jsast::SwitchCase {
                            tests: vec![33, 35, 36, 37, 38, 39, 126]
                                .into_iter()
                                .map(|b| Expr::Num(f64::from(b)))
                                .collect(),
                            body: vec![
                                jsast::assign_stmt(jsast::id("d3"), Expr::Bool(true)),
                                Stmt::Break(None),
                            ],
                        },
                        jsast::SwitchCase {
                            tests: Vec::new(),
                            body: vec![jsast::assign_stmt(jsast::id("d3"), Expr::Bool(false))],
                        },
                    ],
                ),
                Stmt::Labeled(
                    "a".to_owned(),
                    Box::new(Stmt::Block(vec![Stmt::Switch(
                        jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::id("i")]),
                        vec![
                            jsast::SwitchCase {
                                tests: vec![33, 42, 60, 91, 92, 200]
                                    .into_iter()
                                    .map(|b| Expr::Num(f64::from(b)))
                                    .collect(),
                                body: vec![
                                    jsast::assign_stmt(jsast::id("hit"), Expr::Num(1.0)),
                                    Stmt::Break(Some("a".to_owned())),
                                ],
                            },
                            jsast::SwitchCase {
                                tests: Vec::new(),
                                body: vec![jsast::assign_stmt(jsast::id("hit"), Expr::Num(0.0))],
                            },
                        ],
                    )])),
                ),
                Stmt::Return(Some(Expr::Array(vec![
                    jsast::id("d1"),
                    jsast::id("d2"),
                    jsast::id("d3"),
                    jsast::id("hit"),
                ]))),
            ],
        };
        // Pricing trial (`allow_duplicating = false`) must NOT emit 128-byte LUTs.
        let mut trial = pos.clone();
        run_speed_post_opts_with_mode(&mut trial, false, &HashSet::new());
        assert!(
            !jsast::stmt_to_string(&trial).contains("Uint8Array"),
            "pricing trial must not bloat AST with inline Uint8Array LUTs"
        );

        run_speed_post_opts(&mut pos);
        let printed = jsast::stmt_to_string(&pos);
        assert_eq!(
            printed.matches("new Uint8Array(").count(),
            4,
            "expected all 4 dense ASCII tests lowered to Uint8Array LUTs: {printed}"
        );
        assert!(
            !printed.contains("switch"),
            "both dense ASCII switches should be replaced by LUT checks: {printed}"
        );
        assert!(
            printed.contains("!== 1"),
            "negated && byte chain should emit !== 1: {printed}"
        );

        // 2. Negative soundness cases: non-numeric param `p` (could be "33"!), sparse >255 Unicode
        //    switch, < 6 distinct bytes, and switch falling through to default.
        let mut neg = Stmt::FunctionDecl {
            name: "reject_luts".to_owned(),
            params: vec!["p".to_owned(), "s".to_owned(), "i".to_owned()],
            body: vec![
                jsast::lets(vec![
                    ("c".to_owned(), None),
                    ("bad_str".to_owned(), None),
                    ("too_few".to_owned(), None),
                    ("uni".to_owned(), None),
                    ("ft".to_owned(), None),
                ]),
                jsast::assign_stmt(
                    jsast::id("c"),
                    jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::id("i")]),
                ),
                // Non-numeric param `p`
                jsast::assign_stmt(
                    jsast::id("bad_str"),
                    eq_chain("p", &[33, 42, 60, 91, 92, 95], BinOp::Or, BinOp::StrictEq),
                ),
                // Only 5 distinct bytes
                jsast::assign_stmt(
                    jsast::id("too_few"),
                    eq_chain("c", &[33, 42, 60, 91, 92], BinOp::Or, BinOp::StrictEq),
                ),
                // Unicode > 255 switch
                Stmt::Switch(
                    jsast::id("c"),
                    vec![
                        jsast::SwitchCase {
                            tests: vec![160, 5760, 8232, 8233, 8239, 8287, 12288, 65279]
                                .into_iter()
                                .map(|b| Expr::Num(f64::from(b)))
                                .collect(),
                            body: vec![
                                jsast::assign_stmt(jsast::id("uni"), Expr::Bool(true)),
                                Stmt::Break(None),
                            ],
                        },
                        jsast::SwitchCase {
                            tests: Vec::new(),
                            body: vec![jsast::assign_stmt(jsast::id("uni"), Expr::Bool(false))],
                        },
                    ],
                ),
                // Fallthrough from matching arm into default (no `break`)
                Stmt::Switch(
                    jsast::id("c"),
                    vec![
                        jsast::SwitchCase {
                            tests: vec![33, 35, 36, 37, 38, 39]
                                .into_iter()
                                .map(|b| Expr::Num(f64::from(b)))
                                .collect(),
                            body: vec![jsast::assign_stmt(jsast::id("ft"), Expr::Num(1.0))],
                        },
                        jsast::SwitchCase {
                            tests: Vec::new(),
                            body: vec![jsast::assign_stmt(jsast::id("ft"), Expr::Num(2.0))],
                        },
                    ],
                ),
                Stmt::Return(Some(Expr::Array(vec![
                    jsast::id("bad_str"),
                    jsast::id("too_few"),
                    jsast::id("uni"),
                    jsast::id("ft"),
                ]))),
            ],
        };
        run_speed_post_opts(&mut neg);
        let np = jsast::stmt_to_string(&neg);
        assert!(
            !np.contains("Uint8Array"),
            "non-numeric, <6, >255, and fallthrough cases must NOT lower to Uint8Array LUT: {np}"
        );
    }

    #[test]
    fn lowers_chained_string_eq_to_switch_and_preserves_breaks_and_duplicates() {
        let str_eq = |var: &str, s: &str| {
            jsast::binary(BinOp::StrictEq, jsast::id(var), Expr::Str(s.to_owned()))
        };
        let rev_eq = |s: &str, var: &str| {
            jsast::binary(BinOp::StrictEq, Expr::Str(s.to_owned()), jsast::id(var))
        };
        let not_eq = |var: &str, s: &str| Expr::Unary(UnOp::Not, Box::new(str_eq(var, s)));

        // 1. Positive A: Labeled `r: if ("EQ" === k) ... else if ("NE" === k) ... else { if ("LT" === k) { ... break r; } ... }`
        let mut comparator = Stmt::FunctionDecl {
            name: "cmp".to_owned(),
            params: vec!["k".to_owned()],
            body: vec![
                jsast::lets(vec![("e".to_owned(), None)]),
                Stmt::Labeled(
                    "r".to_owned(),
                    Box::new(Stmt::If(
                        rev_eq("EQ", "k"),
                        vec![jsast::assign_stmt(jsast::id("e"), Expr::Num(1.0))],
                        Some(vec![Stmt::If(
                            rev_eq("NE", "k"),
                            vec![jsast::assign_stmt(jsast::id("e"), Expr::Num(2.0))],
                            Some(vec![
                                Stmt::If(
                                    rev_eq("LT", "k"),
                                    vec![
                                        jsast::assign_stmt(jsast::id("e"), Expr::Num(3.0)),
                                        Stmt::Break(Some("r".to_owned())),
                                    ],
                                    None,
                                ),
                                Stmt::If(
                                    rev_eq("GT", "k"),
                                    vec![
                                        jsast::assign_stmt(jsast::id("e"), Expr::Num(4.0)),
                                        Stmt::Break(Some("r".to_owned())),
                                    ],
                                    None,
                                ),
                                jsast::assign_stmt(jsast::id("e"), Expr::Num(5.0)),
                            ]),
                        )]),
                    )),
                ),
                Stmt::Return(Some(jsast::id("e"))),
            ],
        };
        run_speed_post_opts(&mut comparator);
        let cp = jsast::stmt_to_string(&comparator);
        assert!(
            cp.contains("r: switch (k)")
                && cp.contains("case \"EQ\":")
                && cp.contains("case \"NE\":")
                && cp.contains("case \"LT\":")
                && cp.contains("case \"GT\":")
                && cp.contains("default:")
                && !cp.contains("else if"),
            "expected labeled comparator chain lowered to r: switch (k): {cp}"
        );

        // 2. Positive B: Negated multi-key `jmespath$functions::call` tree -> ONE flat `switch (a)`.
        let mut multi_neg = Stmt::FunctionDecl {
            name: "dispatch_fn".to_owned(),
            params: vec!["a".to_owned()],
            body: vec![
                jsast::lets(vec![("f".to_owned(), None)]),
                Stmt::Labeled(
                    "b".to_owned(),
                    Box::new(Stmt::Block(vec![
                        Stmt::If(
                            jsast::binary(BinOp::And, not_eq("a", "abs"), not_eq("a", "floor")),
                            vec![Stmt::If(
                                str_eq("a", "contains"),
                                vec![
                                    jsast::assign_stmt(jsast::id("f"), Expr::Num(2.0)),
                                    Stmt::Break(Some("b".to_owned())),
                                ],
                                Some(vec![
                                    Stmt::If(
                                        jsast::binary(
                                            BinOp::And,
                                            not_eq("a", "starts_with"),
                                            not_eq("a", "ends_with"),
                                        ),
                                        vec![Stmt::If(
                                            str_eq("a", "length"),
                                            vec![
                                                jsast::assign_stmt(jsast::id("f"), Expr::Num(4.0)),
                                                Stmt::Break(Some("b".to_owned())),
                                            ],
                                            Some(vec![
                                                jsast::assign_stmt(jsast::id("f"), Expr::Num(0.0)),
                                                Stmt::Break(Some("b".to_owned())),
                                            ]),
                                        )],
                                        None,
                                    ),
                                    jsast::assign_stmt(jsast::id("f"), Expr::Num(3.0)),
                                    Stmt::Break(Some("b".to_owned())),
                                ]),
                            )],
                            None,
                        ),
                        jsast::assign_stmt(jsast::id("f"), Expr::Num(1.0)),
                    ])),
                ),
                Stmt::Return(Some(jsast::id("f"))),
            ],
        };
        run_speed_post_opts(&mut multi_neg);
        let mp = jsast::stmt_to_string(&multi_neg);
        assert!(
            mp.contains("switch (a)")
                && mp.contains("case \"abs\":")
                && mp.contains("case \"floor\":")
                && mp.contains("case \"contains\":")
                && mp.contains("case \"starts_with\":")
                && mp.contains("case \"ends_with\":")
                && mp.contains("case \"length\":")
                && mp.contains("default:")
                && !mp.contains("if ("),
            "expected negated multi-key dispatch tree collapsed into flat switch: {mp}"
        );

        // 2b. Positive C: Negated multi-key chain whose `then` exits via an exhaustive labeled-break
        //     `switch (tag)` (`jmespath$eval$eval` idiom for `Field`/`Identity`/`Literal`/`Pipe`/`Index`).
        let mut switch_exit = Stmt::FunctionDecl {
            name: "eval_like".to_owned(),
            params: vec!["e".to_owned(), "tag".to_owned()],
            body: vec![
                jsast::lets(vec![("c".to_owned(), None)]),
                Stmt::Labeled(
                    "b".to_owned(),
                    Box::new(Stmt::Block(vec![Stmt::If(
                        str_eq("e", "Field"),
                        vec![jsast::assign_stmt(jsast::id("c"), Expr::Num(1.0))],
                        Some(vec![
                            Stmt::If(
                                jsast::binary(
                                    BinOp::And,
                                    not_eq("e", "Identity"),
                                    not_eq("e", "Current"),
                                ),
                                vec![Stmt::If(
                                    str_eq("e", "Literal"),
                                    vec![
                                        jsast::assign_stmt(jsast::id("c"), Expr::Num(3.0)),
                                        Stmt::Break(Some("b".to_owned())),
                                    ],
                                    Some(vec![
                                        Stmt::If(
                                            not_eq("e", "Pipe"),
                                            vec![
                                                jsast::assign_stmt(jsast::id("c"), Expr::Num(5.0)),
                                                Stmt::Break(Some("b".to_owned())),
                                            ],
                                            None,
                                        ),
                                        Stmt::Switch(
                                            jsast::id("tag"),
                                            vec![
                                                jsast::SwitchCase {
                                                    tests: vec![Expr::Num(0.0)],
                                                    body: vec![
                                                        jsast::assign_stmt(
                                                            jsast::id("c"),
                                                            Expr::Num(4.0),
                                                        ),
                                                        Stmt::Break(Some("b".to_owned())),
                                                    ],
                                                },
                                                jsast::SwitchCase {
                                                    tests: Vec::new(),
                                                    body: vec![Stmt::Throw(Expr::Str(
                                                        "err".to_owned(),
                                                    ))],
                                                },
                                            ],
                                        ),
                                    ]),
                                )],
                                None,
                            ),
                            jsast::assign_stmt(jsast::id("c"), Expr::Num(2.0)),
                        ]),
                    )])),
                ),
                Stmt::Return(Some(jsast::id("c"))),
            ],
        };
        run_speed_post_opts(&mut switch_exit);
        let sep = jsast::stmt_to_string(&switch_exit);
        assert!(
            sep.contains("switch (e)")
                && sep.contains("case \"Field\":")
                && sep.contains("case \"Identity\":")
                && sep.contains("case \"Current\":")
                && sep.contains("case \"Literal\":")
                && sep.contains("case \"Pipe\":")
                && !sep.contains("if ("),
            "exhaustive non-breaking inner switch should allow full outer string switch collapse: {sep}"
        );

        // 3. Negative A: Unlabeled `break;` inside a loop MUST NOT convert to `switch`!
        let mut loop_brk = Stmt::FunctionDecl {
            name: "in_loop".to_owned(),
            params: vec!["k".to_owned()],
            body: vec![
                jsast::lets(vec![("r".to_owned(), Some(Expr::Num(0.0)))]),
                Stmt::LoopForever(
                    None,
                    vec![Stmt::If(
                        str_eq("k", "A"),
                        vec![
                            jsast::assign_stmt(jsast::id("r"), Expr::Num(1.0)),
                            Stmt::Break(None),
                        ],
                        Some(vec![Stmt::If(
                            str_eq("k", "B"),
                            vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(2.0))],
                            Some(vec![Stmt::If(
                                str_eq("k", "C"),
                                vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(3.0))],
                                Some(vec![Stmt::If(
                                    str_eq("k", "D"),
                                    vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(4.0))],
                                    None,
                                )]),
                            )]),
                        )]),
                    )],
                ),
                Stmt::Return(Some(jsast::id("r"))),
            ],
        };
        run_speed_post_opts(&mut loop_brk);
        assert!(
            !jsast::stmt_to_string(&loop_brk).contains("switch"),
            "unlabeled break inside loop must NEVER be captured by synthesized switch"
        );

        // 4. Negative B: Duplicate string key MUST NOT convert to `switch`!
        let mut dup_key = Stmt::FunctionDecl {
            name: "dup".to_owned(),
            params: vec!["k".to_owned()],
            body: vec![
                jsast::lets(vec![("r".to_owned(), None)]),
                Stmt::If(
                    str_eq("k", "A"),
                    vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(1.0))],
                    Some(vec![Stmt::If(
                        str_eq("k", "A"),
                        vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(2.0))],
                        Some(vec![Stmt::If(
                            str_eq("k", "B"),
                            vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(3.0))],
                            Some(vec![Stmt::If(
                                str_eq("k", "C"),
                                vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(4.0))],
                                None,
                            )]),
                        )]),
                    )]),
                ),
                Stmt::Return(Some(jsast::id("r"))),
            ],
        };
        run_speed_post_opts(&mut dup_key);
        assert!(
            !jsast::stmt_to_string(&dup_key).contains("switch"),
            "duplicate string key must bail out cleanly"
        );

        // 5. Negative C: Non-exiting `if` inside an `else` followed by a shared statement MUST NOT
        //    drop the shared statement for that key!
        let mut fallthrough_else = Stmt::FunctionDecl {
            name: "fallthrough".to_owned(),
            params: vec!["k".to_owned()],
            body: vec![
                jsast::lets(vec![("r".to_owned(), Some(Expr::Num(0.0)))]),
                Stmt::If(
                    str_eq("k", "A"),
                    vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(1.0))],
                    Some(vec![Stmt::If(
                        str_eq("k", "B"),
                        vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(2.0))],
                        Some(vec![Stmt::If(
                            str_eq("k", "C"),
                            vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(3.0))],
                            Some(vec![
                                Stmt::If(
                                    str_eq("k", "D"),
                                    vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(4.0))],
                                    None,
                                ),
                                jsast::assign_stmt(
                                    jsast::id("r"),
                                    jsast::binary(BinOp::Add, jsast::id("r"), Expr::Num(10.0)),
                                ),
                            ]),
                        )]),
                    )]),
                ),
                Stmt::Return(Some(jsast::id("r"))),
            ],
        };
        run_speed_post_opts(&mut fallthrough_else);
        assert!(
            !jsast::stmt_to_string(&fallthrough_else).contains("switch"),
            "non-exiting if inside else followed by shared tail must not miscompile (only 3 safe arms)"
        );
    }

    #[test]
    fn coalesces_disjoint_switch_locals_while_preserving_overlapping_try_and_entry_live() {
        let params = vec!["k".to_owned(), "x".to_owned()];
        // 4 disjoint switch arms using t1..t4, plus `live_overlap` live across `t5`,
        // `handler_live` read in catch after `try_write`, and `entry_undef` read without write.
        let mut body = vec![
            jsast::lets(vec![
                ("t1".to_owned(), None),
                ("t2".to_owned(), None),
                ("t3".to_owned(), None),
                ("t4".to_owned(), None),
                ("live_overlap".to_owned(), None),
                ("t5".to_owned(), None),
                ("handler_live".to_owned(), None),
                ("try_write".to_owned(), None),
                ("entry_undef".to_owned(), None),
                ("res".to_owned(), None),
            ]),
            jsast::assign_stmt(jsast::id("handler_live"), jsast::id("x")),
            Stmt::TryCatch {
                body: vec![
                    jsast::assign_stmt(
                        jsast::id("try_write"),
                        jsast::call(jsast::id("may_throw"), vec![jsast::id("x")]),
                    ),
                    jsast::assign_stmt(jsast::id("res"), jsast::id("try_write")),
                ],
                binding: "e".to_owned(),
                handler: vec![jsast::assign_stmt(
                    jsast::id("res"),
                    jsast::id("handler_live"),
                )],
            },
            Stmt::Switch(
                jsast::id("k"),
                vec![
                    jsast::SwitchCase {
                        tests: vec![Expr::Num(1.0)],
                        body: vec![
                            jsast::assign_stmt(
                                jsast::id("t1"),
                                jsast::binary(BinOp::Add, jsast::id("res"), Expr::Num(1.0)),
                            ),
                            jsast::assign_stmt(jsast::id("res"), jsast::id("t1")),
                            Stmt::Break(None),
                        ],
                    },
                    jsast::SwitchCase {
                        tests: vec![Expr::Num(2.0)],
                        body: vec![
                            jsast::assign_stmt(
                                jsast::id("t2"),
                                jsast::binary(BinOp::Add, jsast::id("res"), Expr::Num(2.0)),
                            ),
                            jsast::assign_stmt(jsast::id("res"), jsast::id("t2")),
                            Stmt::Break(None),
                        ],
                    },
                    jsast::SwitchCase {
                        tests: vec![Expr::Num(3.0)],
                        body: vec![
                            jsast::assign_stmt(
                                jsast::id("t3"),
                                jsast::binary(BinOp::Add, jsast::id("res"), Expr::Num(3.0)),
                            ),
                            jsast::assign_stmt(jsast::id("res"), jsast::id("t3")),
                            Stmt::Break(None),
                        ],
                    },
                    jsast::SwitchCase {
                        tests: vec![],
                        body: vec![
                            jsast::assign_stmt(
                                jsast::id("t4"),
                                jsast::binary(BinOp::Add, jsast::id("res"), Expr::Num(4.0)),
                            ),
                            jsast::assign_stmt(jsast::id("live_overlap"), jsast::id("t4")),
                            jsast::assign_stmt(
                                jsast::id("t5"),
                                jsast::binary(BinOp::Add, jsast::id("x"), Expr::Num(5.0)),
                            ),
                            jsast::assign_stmt(
                                jsast::id("res"),
                                jsast::binary(
                                    BinOp::Add,
                                    jsast::id("live_overlap"),
                                    jsast::id("t5"),
                                ),
                            ),
                        ],
                    },
                ],
            ),
            Stmt::Return(Some(jsast::binary(
                BinOp::Add,
                jsast::id("res"),
                jsast::id("entry_undef"),
            ))),
        ];

        coalesce_disjoint_locals_with_threshold(&params, &mut body, 4);
        let decl_count = match &body[0] {
            Stmt::Lets(decls) => decls.len(),
            _ => panic!("expected leading Lets"),
        };
        let printed = jsast::stmt_to_string(&Stmt::FunctionDecl {
            name: "f".to_owned(),
            params,
            body,
        });
        assert!(
            decl_count <= 4,
            "expected 10 locals to coalesce into <= 4 slots (got {decl_count}): {printed}"
        );
        // `entry_undef` was read before write so it MUST stay its own untouched slot.
        assert!(
            printed.contains("entry_undef"),
            "entry-live local must not be coalesced: {printed}"
        );
        // `handler_live` and `try_write` interfere across the try/catch edge.
        let try_pos = printed.find("may_throw").unwrap();
        let catch_pos = printed.find("catch").unwrap();
        let try_slice = &printed[try_pos - 20..try_pos];
        let catch_slice = &printed[catch_pos..catch_pos + 50];
        assert_ne!(
            try_slice.split('=').next().unwrap().trim(),
            catch_slice
                .split('=')
                .nth(1)
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .trim(),
            "try_write must not alias handler_live: {printed}"
        );

        // `coalesce_disjoint_locals_decl` only activates for directly self-recursive functions.
        assert!(!body_calls_self(
            "non_rec",
            &[Stmt::Return(Some(jsast::call(jsast::id("other"), vec![])))]
        ));
        assert!(body_calls_self(
            "self_rec",
            &[Stmt::Return(Some(jsast::call(
                jsast::id("self_rec"),
                vec![]
            )))]
        ));
    }

    /// Helper building the exact Rust `f64 -> i32/u32` `ToInt32` labeled-block diamond emitted
    /// in `earcut` (`earcut_linked` and `is_ear_hashed_sl`).
    #[allow(clippy::too_many_arguments)]
    fn make_to_int32_diamond(
        label: &str,
        src: &str,
        dst: &str,
        abs_var: &str,
        rem_var: &str,
        mag_var: &str,
        neg_var: &str,
        res_var: &str,
        modulus: f64,
        unsigned_dst: bool,
    ) -> Stmt {
        let wrap_op = if unsigned_dst {
            BinOp::UShr
        } else {
            BinOp::BitOr
        };
        let wrap =
            |v: &str| Expr::Binary(wrap_op, Box::new(jsast::id(v)), Box::new(Expr::Num(0.0)));
        Stmt::Labeled(
            label.to_owned(),
            Box::new(Stmt::Block(vec![
                Stmt::If(
                    Expr::Binary(
                        BinOp::And,
                        Box::new(Expr::Binary(
                            BinOp::Ge,
                            Box::new(jsast::id(src)),
                            Box::new(Expr::Unary(UnOp::Neg, Box::new(Expr::Num(2147483648.0)))),
                        )),
                        Box::new(Expr::Binary(
                            BinOp::Le,
                            Box::new(jsast::id(src)),
                            Box::new(Expr::Num(2147483647.0)),
                        )),
                    ),
                    vec![
                        jsast::assign_stmt(jsast::id(dst), wrap(src)),
                        Stmt::Break(Some(label.to_owned())),
                    ],
                    None,
                ),
                jsast::assign_stmt(
                    jsast::id(abs_var),
                    jsast::method_call(jsast::id("Math"), "abs", vec![jsast::id(src)]),
                ),
                jsast::assign_stmt(
                    jsast::id(mag_var),
                    Expr::Seq(vec![
                        Expr::Assign(
                            Box::new(jsast::id(rem_var)),
                            Box::new(Expr::Binary(
                                BinOp::Rem,
                                Box::new(jsast::id(abs_var)),
                                Box::new(Expr::Num(modulus)),
                            )),
                        ),
                        Expr::Cond(
                            Box::new(Expr::Binary(
                                BinOp::Ge,
                                Box::new(jsast::id(rem_var)),
                                Box::new(Expr::Num(4294967295.0)),
                            )),
                            Box::new(Expr::Num(4294967295.0)),
                            Box::new(Expr::Cond(
                                Box::new(Expr::Binary(
                                    BinOp::Gt,
                                    Box::new(jsast::id(rem_var)),
                                    Box::new(Expr::Num(0.0)),
                                )),
                                Box::new(Expr::Binary(
                                    BinOp::UShr,
                                    Box::new(jsast::id(rem_var)),
                                    Box::new(Expr::Num(0.0)),
                                )),
                                Box::new(Expr::Num(0.0)),
                            )),
                        ),
                    ]),
                ),
                Stmt::If(
                    Expr::Binary(
                        BinOp::Lt,
                        Box::new(jsast::id(src)),
                        Box::new(Expr::Num(0.0)),
                    ),
                    vec![
                        jsast::assign_stmt(
                            jsast::id(neg_var),
                            Expr::Binary(
                                BinOp::BitOr,
                                Box::new(Expr::Binary(
                                    BinOp::Sub,
                                    Box::new(Expr::Num(0.0)),
                                    Box::new(jsast::id(mag_var)),
                                )),
                                Box::new(Expr::Num(0.0)),
                            ),
                        ),
                        jsast::assign_stmt(jsast::id(res_var), wrap(neg_var)),
                    ],
                    Some(vec![jsast::assign_stmt(jsast::id(res_var), wrap(mag_var))]),
                ),
                jsast::assign_stmt(jsast::id(dst), jsast::id(res_var)),
            ])),
        )
    }

    #[test]
    fn folds_to_int32_diamonds_signed_unsigned_and_back_to_back_reused_locals() {
        // 1. Disjoint locals (`is_ear_hashed_sl` shape) + unsigned `>>> 0` variant.
        let mut decl = Stmt::FunctionDecl {
            name: "zorder_disjoint".to_owned(),
            params: vec!["u".to_owned(), "w".to_owned()],
            body: vec![
                jsast::lets(vec![
                    ("S".to_owned(), None),
                    ("U".to_owned(), None),
                    ("aS".to_owned(), None),
                    ("aL".to_owned(), None),
                    ("E".to_owned(), None),
                    ("aT".to_owned(), None),
                    ("as_".to_owned(), None),
                    ("bS".to_owned(), None),
                    ("bL".to_owned(), None),
                    ("F".to_owned(), None),
                    ("bT".to_owned(), None),
                    ("bs_".to_owned(), None),
                ]),
                make_to_int32_diamond(
                    "j",
                    "u",
                    "S",
                    "aS",
                    "E",
                    "aL",
                    "aT",
                    "as_",
                    4294967296.0,
                    false,
                ),
                make_to_int32_diamond(
                    "a",
                    "w",
                    "U",
                    "bS",
                    "F",
                    "bL",
                    "bT",
                    "bs_",
                    4294967296.0,
                    true,
                ),
                Stmt::Return(Some(Expr::Array(vec![jsast::id("S"), jsast::id("U")]))),
            ],
        };
        run_speed_post_opts(&mut decl);
        let printed = jsast::stmt_to_string(&decl);
        assert!(
            !printed.contains("4294967296") && !printed.contains("Math.abs"),
            "expected both signed & unsigned ToInt32 diamonds folded: {printed}"
        );
        assert!(
            printed.contains("S = u | 0;") && printed.contains("U = w >>> 0;"),
            "expected exact `S = u | 0` and `U = w >>> 0`: {printed}"
        );

        // 2. Coalesced back-to-back shape (`earcut_linked`): first diamond clobbers `m` & `j`,
        //    second diamond reads `g` and clobbers `m` & `g`, then both `g` & `m` are overwritten
        //    before any subsequent read.
        let mut coalesced = Stmt::FunctionDecl {
            name: "zorder_coalesced".to_owned(),
            params: vec!["x".to_owned(), "y".to_owned()],
            body: vec![
                jsast::lets(vec![
                    ("m".to_owned(), None),
                    ("g".to_owned(), None),
                    ("k".to_owned(), None),
                    ("h".to_owned(), None),
                    ("j".to_owned(), None),
                ]),
                jsast::assign_stmt(jsast::id("m"), jsast::id("x")),
                jsast::assign_stmt(jsast::id("g"), jsast::id("y")),
                make_to_int32_diamond("d", "m", "k", "j", "j", "k", "m", "m", 4294967296.0, false),
                make_to_int32_diamond("l", "g", "h", "m", "m", "h", "g", "g", 4294967296.0, false),
                jsast::assign_stmt(
                    jsast::id("g"),
                    Expr::Binary(
                        BinOp::BitAnd,
                        Box::new(jsast::id("k")),
                        Box::new(Expr::Num(255.0)),
                    ),
                ),
                jsast::assign_stmt(
                    jsast::id("m"),
                    Expr::Binary(
                        BinOp::BitAnd,
                        Box::new(jsast::id("h")),
                        Box::new(Expr::Num(255.0)),
                    ),
                ),
                Stmt::Return(Some(Expr::Binary(
                    BinOp::BitOr,
                    Box::new(jsast::id("g")),
                    Box::new(jsast::id("m")),
                ))),
            ],
        };
        run_speed_post_opts(&mut coalesced);
        let coalesced_js = jsast::stmt_to_string(&coalesced);
        assert!(
            !coalesced_js.contains("4294967296") && !coalesced_js.contains("Math.abs"),
            "expected back-to-back coalesced ToInt32 diamonds folded: {coalesced_js}"
        );
        assert!(
            coalesced_js.contains("k = m | 0;") && coalesced_js.contains("h = g | 0;"),
            "expected `k = m | 0` and `h = g | 0`: {coalesced_js}"
        );
    }

    #[test]
    fn rejects_unsound_to_int32_diamonds_live_clobber_shadowed_math_wrong_mod() {
        // 1. Live temp `j` read after the block MUST NOT fold.
        let mut live_temp = Stmt::FunctionDecl {
            name: "live_temp".to_owned(),
            params: vec!["m".to_owned()],
            body: vec![
                jsast::lets(vec![
                    ("k".to_owned(), None),
                    ("j".to_owned(), None),
                    ("n".to_owned(), None),
                    ("r".to_owned(), None),
                ]),
                make_to_int32_diamond("d", "m", "k", "j", "j", "k", "n", "r", 4294967296.0, false),
                Stmt::Return(Some(Expr::Array(vec![jsast::id("k"), jsast::id("j")]))),
            ],
        };
        run_speed_post_opts(&mut live_temp);
        let s1 = jsast::stmt_to_string(&live_temp);
        assert!(
            s1.contains("4294967296"),
            "live temp `j` must prevent folding: {s1}"
        );

        // 2. Live clobbered `src` (`m`) read after the block MUST NOT fold.
        let mut live_src = Stmt::FunctionDecl {
            name: "live_src".to_owned(),
            params: vec!["m".to_owned()],
            body: vec![
                jsast::lets(vec![("k".to_owned(), None), ("j".to_owned(), None)]),
                make_to_int32_diamond("d", "m", "k", "j", "j", "k", "m", "m", 4294967296.0, false),
                Stmt::Return(Some(Expr::Array(vec![jsast::id("k"), jsast::id("m")]))),
            ],
        };
        run_speed_post_opts(&mut live_src);
        let s2 = jsast::stmt_to_string(&live_src);
        assert!(
            s2.contains("4294967296"),
            "live clobbered `m` must prevent folding: {s2}"
        );

        // 3. Shadowed `Math` parameter MUST NOT fold.
        let mut shadowed_math = Stmt::FunctionDecl {
            name: "shadowed_math".to_owned(),
            params: vec!["Math".to_owned(), "u".to_owned()],
            body: vec![
                jsast::lets(vec![
                    ("S".to_owned(), None),
                    ("aS".to_owned(), None),
                    ("aL".to_owned(), None),
                    ("E".to_owned(), None),
                    ("aT".to_owned(), None),
                    ("as_".to_owned(), None),
                ]),
                make_to_int32_diamond(
                    "j",
                    "u",
                    "S",
                    "aS",
                    "E",
                    "aL",
                    "aT",
                    "as_",
                    4294967296.0,
                    false,
                ),
                Stmt::Return(Some(jsast::id("S"))),
            ],
        };
        run_speed_post_opts(&mut shadowed_math);
        let s3 = jsast::stmt_to_string(&shadowed_math);
        assert!(
            s3.contains("4294967296"),
            "shadowed Math must prevent folding: {s3}"
        );

        // 4. Wrong modulus (`4294967295` instead of `4294967296`) MUST NOT fold.
        let mut wrong_mod = Stmt::FunctionDecl {
            name: "wrong_mod".to_owned(),
            params: vec!["u".to_owned()],
            body: vec![
                jsast::lets(vec![
                    ("S".to_owned(), None),
                    ("aS".to_owned(), None),
                    ("aL".to_owned(), None),
                    ("E".to_owned(), None),
                    ("aT".to_owned(), None),
                    ("as_".to_owned(), None),
                ]),
                make_to_int32_diamond(
                    "j",
                    "u",
                    "S",
                    "aS",
                    "E",
                    "aL",
                    "aT",
                    "as_",
                    4294967295.0,
                    false,
                ),
                Stmt::Return(Some(jsast::id("S"))),
            ],
        };
        run_speed_post_opts(&mut wrong_mod);
        let s4 = jsast::stmt_to_string(&wrong_mod);
        assert!(
            s4.contains("4294967295"),
            "wrong modulus must prevent folding: {s4}"
        );
    }

    #[test]
    fn algebraic_post_opts_i32_cond_array_index_neg_bool_and_self_assign() {
        let mut decl = Stmt::FunctionDecl {
            name: "cleanups".to_owned(),
            params: vec![
                "x".to_owned(),
                "y".to_owned(),
                "n".to_owned(),
                "f".to_owned(),
            ],
            body: vec![
                jsast::lets(vec![
                    ("c1".to_owned(), None),
                    ("c2".to_owned(), None),
                    ("c3".to_owned(), None),
                    ("c4".to_owned(), None),
                    ("c5".to_owned(), None),
                    ("c6".to_owned(), None),
                    ("c7".to_owned(), None),
                    ("c8".to_owned(), None),
                    ("F".to_owned(), None),
                ]),
                // 1. `(x <= -2147483648 ? -2147483648 : x >= 2147483647 ? 2147483647 : x | 0) | 0`
                //    drops outer redundant `| 0`.
                jsast::assign_stmt(
                    jsast::id("c1"),
                    Expr::Binary(
                        BinOp::BitOr,
                        Box::new(Expr::Cond(
                            Box::new(Expr::Binary(
                                BinOp::Le,
                                Box::new(jsast::id("x")),
                                Box::new(Expr::Unary(UnOp::Neg, Box::new(Expr::Num(2147483648.0)))),
                            )),
                            Box::new(Expr::Unary(UnOp::Neg, Box::new(Expr::Num(2147483648.0)))),
                            Box::new(Expr::Cond(
                                Box::new(Expr::Binary(
                                    BinOp::Ge,
                                    Box::new(jsast::id("x")),
                                    Box::new(Expr::Num(2147483647.0)),
                                )),
                                Box::new(Expr::Num(2147483647.0)),
                                Box::new(Expr::Binary(
                                    BinOp::BitOr,
                                    Box::new(jsast::id("x")),
                                    Box::new(Expr::Num(0.0)),
                                )),
                            )),
                        )),
                        Box::new(Expr::Num(0.0)),
                    ),
                ),
                // 2. `(x >>> 12) | 0` -> `x >>> 12` (proven nonneg i31)
                jsast::assign_stmt(
                    jsast::id("c2"),
                    Expr::Binary(
                        BinOp::BitOr,
                        Box::new(Expr::Binary(
                            BinOp::UShr,
                            Box::new(jsast::id("x")),
                            Box::new(Expr::Num(12.0)),
                        )),
                        Box::new(Expr::Num(0.0)),
                    ),
                ),
                // 3. `(0 - y) | 0` -> `-y | 0`
                jsast::assign_stmt(
                    jsast::id("c3"),
                    Expr::Binary(
                        BinOp::BitOr,
                        Box::new(Expr::Binary(
                            BinOp::Sub,
                            Box::new(Expr::Num(0.0)),
                            Box::new(jsast::id("y")),
                        )),
                        Box::new(Expr::Num(0.0)),
                    ),
                ),
                // 4. `[void 0, y][1]` -> `y`, while `[f(), y][1]` MUST NOT fold (impure discarded elem)
                //    and `[c4, y][0] = 7` MUST NOT turn LHS into non-lvalue/direct `c4 = 7`.
                jsast::assign_stmt(
                    jsast::id("c4"),
                    jsast::index(
                        Expr::Array(vec![Expr::Undefined, jsast::id("y")]),
                        Expr::Num(1.0),
                    ),
                ),
                jsast::assign_stmt(
                    jsast::id("c5"),
                    jsast::index(
                        Expr::Array(vec![jsast::call(jsast::id("f"), vec![]), jsast::id("y")]),
                        Expr::Num(1.0),
                    ),
                ),
                jsast::assign_stmt(
                    jsast::index(
                        Expr::Array(vec![jsast::id("c4"), jsast::id("y")]),
                        Expr::Num(0.0),
                    ),
                    Expr::Num(7.0),
                ),
                // 5. `(x === 1) && true` -> `x === 1`; non-bool `n && true` MUST NOT fold.
                jsast::assign_stmt(
                    jsast::id("c6"),
                    Expr::Binary(
                        BinOp::And,
                        Box::new(Expr::Binary(
                            BinOp::StrictEq,
                            Box::new(jsast::id("x")),
                            Box::new(Expr::Num(1.0)),
                        )),
                        Box::new(Expr::Bool(true)),
                    ),
                ),
                jsast::assign_stmt(
                    jsast::id("c7"),
                    Expr::Binary(
                        BinOp::And,
                        Box::new(jsast::id("n")),
                        Box::new(Expr::Bool(true)),
                    ),
                ),
                // 6. `if (!!x) { c8 = 1; }` -> `if (x) { c8 = 1; }`
                Stmt::If(
                    Expr::Unary(
                        UnOp::Not,
                        Box::new(Expr::Unary(UnOp::Not, Box::new(jsast::id("x")))),
                    ),
                    vec![jsast::assign_stmt(jsast::id("c8"), Expr::Num(1.0))],
                    None,
                ),
                // 7. `if (F < y) { F = F; } else { F = y; }` -> `if (!(F < y)) { F = y; }`
                Stmt::If(
                    Expr::Binary(
                        BinOp::Lt,
                        Box::new(jsast::id("F")),
                        Box::new(jsast::id("y")),
                    ),
                    vec![jsast::assign_stmt(jsast::id("F"), jsast::id("F"))],
                    Some(vec![jsast::assign_stmt(jsast::id("F"), jsast::id("y"))]),
                ),
                Stmt::Return(Some(Expr::Array(vec![
                    jsast::id("c1"),
                    jsast::id("c2"),
                    jsast::id("c3"),
                    jsast::id("c4"),
                    jsast::id("c5"),
                    jsast::id("c6"),
                    jsast::id("c7"),
                    jsast::id("c8"),
                    jsast::id("F"),
                ]))),
            ],
        };
        run_speed_post_opts(&mut decl);
        let printed = jsast::stmt_to_string(&decl);
        assert!(
            printed.contains(
                "c1 = x <= -2147483648 ? -2147483648 : x >= 2147483647 ? 2147483647 : x | 0;"
            ),
            "expected outer `| 0` dropped on i32 ternary: {printed}"
        );
        assert!(
            printed.contains("c2 = x >>> 12;"),
            "expected `(x >>> 12) | 0` folded to `x >>> 12`: {printed}"
        );
        assert!(
            printed.contains("c3 = -y | 0;"),
            "expected `(0 - y) | 0` folded to `-y | 0`: {printed}"
        );
        assert!(
            printed.contains("c4 = y;"),
            "expected `[void 0, y][1]` folded to `y`: {printed}"
        );
        assert!(
            printed.contains("c5 = [f(), y][1];"),
            "impure discarded element must prevent array index folding: {printed}"
        );
        assert!(
            printed.contains("[c4, y][0] = 7;"),
            "assignment LHS array index must NOT fold: {printed}"
        );
        assert!(
            printed.contains("c6 = x === 1;"),
            "expected `bool && true` folded: {printed}"
        );
        assert!(
            printed.contains("c7 = n && true;"),
            "non-bool `n && true` must NOT fold: {printed}"
        );
        assert!(
            printed.contains("if (x)") && !printed.contains("!!x"),
            "expected `if (!!x)` unwrapped to `if (x)`: {printed}"
        );
        assert!(
            printed.contains("if (!(F < y))") && !printed.contains("F = F"),
            "expected self-assign removed and empty-then `if` inverted: {printed}"
        );
    }
}
