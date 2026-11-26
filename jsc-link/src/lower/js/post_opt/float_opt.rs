//! IEEE-754 floating-point fast paths: dead-`lo` `$__mires_i64_f64_to_bits` sign-bit tests and
//! `Math.max` / `Math.min` labeled-block idiom folding.

use std::collections::{HashMap, HashSet};

use rcgjs_object::jsast::{self, BinOp, Expr, Stmt, UnOp};

use super::super::ast_util::{
    collect_written_idents_stmt, collect_written_idents_stmts, exact_i32, visit_local_blocks_mut,
};
use super::fold::{exact_f64_lit, num_expr_i32, to_i32_bits};
use super::tally_ident_reads;

// =============================================================================================
// Dead-`lo` `$__mires_i64_f64_to_bits(x)` sign-bit fast-path
// =============================================================================================

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
pub(super) fn fast_path_f64_sign_bit(body: &mut Vec<Stmt>) -> bool {
    let read_counts = tally_ident_reads(body);
    let mut non_nan: HashSet<String> = HashSet::new();
    let mut known_nonzero: HashMap<String, f64> = HashMap::new();
    fast_path_f64_sign_bit_stmts(body, &read_counts, &mut non_nan, &mut known_nonzero)
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
                collect_written_idents_stmts(b, &mut written);
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

// =============================================================================================
// IEEE-754 `Math.max` / `Math.min` labeled-block idiom folding
// =============================================================================================

/// Folds IEEE-754 `Math.max` / `Math.min` labeled-block idioms (both the general 2-variable form
/// and the 1-variable + constant form emitted by `js_max` / `js_min` after `fast_path_f64_sign_bit`)
/// into `dst = Math.max(a, b)` / `dst = Math.min(a, b)`, preserving exact `NaN` and `-0.0`/`+0.0`
/// semantics.
pub(super) fn fold_ieee_max_min(body: &mut Vec<Stmt>) {
    let read_counts = tally_ident_reads(body);
    visit_local_blocks_mut(body, false, |block| {
        fold_ieee_max_min_block(block, &read_counts);
    });
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

/// Shared context for matching an IEEE `Math.max` / `Math.min` labeled-block idiom.
struct MaxMinMatchCtx<'a> {
    dst: &'a str,
    label: &'a str,
    read_counts: &'a HashMap<String, usize>,
}

/// Operands and polarity for verifying the `a === b` tie-breaking arm of `Math.max` / `Math.min`.
struct MaxMinEqOperands<'a> {
    a: &'a Expr,
    b: &'a Expr,
    const_b: Option<f64>,
    is_max: bool,
}

impl MaxMinMatchCtx<'_> {
    /// Verifies `eq_body` (entered when `a === b` and both are non-NaN) matches `Math.max(a, b)`
    /// (`is_max == true`) or `Math.min(a, b)` (`is_max == false`) bit-identically on `-0.0` / `+0.0`.
    fn verify_eq_branch(&self, eq_body: &[Stmt], ops: &MaxMinEqOperands<'_>) -> bool {
        // Case 1: Sign-bit check inside `a === b`.
        let sign_branch: Option<(String, bool, &Expr, &Expr)> = match eq_body {
            [Stmt::ExprStmt(Expr::Assign(dd_lhs, dd_rhs)), Stmt::If(cond, t, Some(e))] => {
                let Expr::Ident(dd_name) = dd_lhs.as_ref() else {
                    return false;
                };
                if self.read_counts.get(dd_name).copied().unwrap_or(0) != 1
                    || dd_name == self.dst
                    || matches!(ops.a, Expr::Ident(id) if id == dd_name)
                    || matches!(ops.b, Expr::Ident(id) if id == dd_name)
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
                    match_assign_and_break(t, self.dst, self.label),
                    match_assign_and_break(e, self.dst, self.label),
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
                    match_assign_and_break(t, self.dst, self.label),
                    match_assign_and_break(e, self.dst, self.label),
                ) else {
                    return false;
                };
                Some((sign_var, cond_true_neg, w1, w2))
            }
            _ => None,
        };

        if let Some((sign_var, cond_true_neg, w1, w2)) = sign_branch {
            let z = Expr::Ident(sign_var);
            let other = if &z == ops.a {
                ops.b
            } else if &z == ops.b {
                ops.a
            } else {
                return false;
            };
            let (on_neg, on_nonneg) = if cond_true_neg { (w1, w2) } else { (w2, w1) };
            return if ops.is_max {
                on_neg == other && on_nonneg == &z
            } else {
                on_neg == &z && on_nonneg == other
            };
        }

        // Case 2: Direct constant-folded assignment `[dst = w; break label]`, valid ONLY when `b` is
        // a finite non-NaN literal (`const_b = Some(c)`).
        if let (Some(c), Some(w)) = (
            ops.const_b,
            match_assign_and_break(eq_body, self.dst, self.label),
        ) {
            if c != 0.0 {
                return w == ops.a || w == ops.b;
            }
            // `c == 0.0`: verify exact `-0.0` vs `+0.0` tie-breaking.
            if c.is_sign_positive() {
                // `+0.0`: Math.max(a, +0) -> +0 (`b`), Math.min(a, +0) -> a (preserves -0).
                return if ops.is_max { w == ops.b } else { w == ops.a };
            } else {
                // `-0.0`: Math.max(a, -0) -> a (preserves +0), Math.min(a, -0) -> -0 (`b`).
                return if ops.is_max { w == ops.a } else { w == ops.b };
            }
        }

        false
    }

    /// Matches the inner compare-and-break body after both operands are proven non-NaN.
    /// Returns `Some((b_expr, is_max))` where `a` is the already-checked variable `a_var`.
    fn match_compare_body(
        &self,
        inner: &[Stmt],
        a_var: &str,
        known_b_var: Option<&str>,
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
            let v1 = match_assign_and_break(ord_then, self.dst, self.label)?;
            let v2 = match_assign_and_break(ord_else, self.dst, self.label)?;
            let is_max = classify_strict_order_branch(ord_cond, v1, v2, &a, b_expr)?;
            let ops = MaxMinEqOperands {
                a: &a,
                b: b_expr,
                const_b,
                is_max,
            };
            if self.verify_eq_branch(cmp_then, &ops) {
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
            let b_expr = if ol.as_ref() == &a {
                or.as_ref()
            } else if or.as_ref() == &a {
                ol.as_ref()
            } else {
                return None;
            };
            let c = exact_f64_lit(b_expr)?;
            if !c.is_finite() {
                return None;
            }
            let v1 = match_assign_and_break(cmp_then, self.dst, self.label)?;
            let v2 = match_assign_and_break(cmp_else, self.dst, self.label)?;
            let is_max = classify_strict_order_branch(cmp_cond, v1, v2, &a, b_expr)?;
            if c != 0.0 {
                return Some((b_expr.clone(), is_max));
            }
            // Check exact tie-breaking at `a === 0` (`+0.0` vs `-0.0`):
            let eq_takes_then = matches!(op, BinOp::Ge | BinOp::Le);
            let eq_result = if eq_takes_then { v1 } else { v2 };
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

    let ctx = MaxMinMatchCtx {
        dst,
        label,
        read_counts,
    };
    let prev_ref = prev_nan_tmp
        .as_ref()
        .map(|(tmp, var, is_nan)| (*tmp, var.as_str(), *is_nan));

    // Either `outer_cond` checks `!is_nan(a)` alone, or collapsed `!is_nan(a) && !is_nan(b)`.
    let matched: Option<(String, Expr, bool)> =
        if let Some(a_var) = match_non_nan_guard_cond(outer_cond, prev_ref) {
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
                            ctx.match_compare_body(b_then, &a_var, Some(&b_var))
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
                        ctx.match_compare_body(b_then, &a_var, Some(&b_var))
                            .map(|(b_expr, is_max)| (a_var.clone(), b_expr, is_max))
                    } else {
                        // Could be 1-variable + constant (no second NaN check).
                        ctx.match_compare_body(outer_then, &a_var, None)
                            .map(|(b_expr, is_max)| (a_var.clone(), b_expr, is_max))
                    }
                }
                _ => ctx
                    .match_compare_body(outer_then, &a_var, None)
                    .map(|(b_expr, is_max)| (a_var.clone(), b_expr, is_max)),
            }
        } else if let Expr::Binary(BinOp::And, c1, c2) = outer_cond {
            if let (Some(a_var), Some(b_var)) = (
                match_non_nan_guard_cond(c1, prev_ref),
                match_non_nan_guard_cond(c2, None),
            ) {
                ctx.match_compare_body(outer_then, &a_var, Some(&b_var))
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

fn fold_ieee_max_min_block(stmts: &mut Vec<Stmt>, read_counts: &HashMap<String, usize>) {
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
