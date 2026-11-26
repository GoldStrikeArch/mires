//! Post-lowering shape/idiom optimizations: `$__mires_i64_div` rem+div fusion into
//! `$__mires_i64_divrem` and `ToInt32` / `ToUint32` labeled-block diamond folding.

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use rcgjs_object::jsast::{self, BinOp, Expr, Stmt, TryVisit, UnOp, VisitMut};

use super::super::ast_util::{
    collect_written_idents_stmts, count_ident_reads_expr, count_ident_reads_stmts, exact_i32,
    expr_has_side_effect, is_panic_callee, stmt_may_exit_non_throw, stmts_definitely_diverge,
    tally_ident_reads_stmt, visit_local_blocks_mut,
};
use super::super::i64_pair;
use super::dce_copy::{scan_const_usage_stmts, LocalConstUsage};
use super::fold::{exact_f64_lit, is_pos_zero};
use super::{alloc_fresh_local, tally_ident_reads};

// =============================================================================================
// `$__mires_i64_div` rem + div fusion into `$__mires_i64_divrem`
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
    struct ClobberScan<'a>(&'a HashSet<String>);
    impl TryVisit for ClobberScan<'_> {
        type Break = ();
        fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            match expr {
                Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
                    if let Expr::Ident(n) = lhs.as_ref() {
                        if self.0.contains(n)
                            || n == i64_pair::HELPER_HI
                            || n == i64_pair::HELPER_RL
                            || n == i64_pair::HELPER_RH
                        {
                            return ControlFlow::Break(());
                        }
                    }
                    self.visit_expr(lhs)?;
                    self.visit_expr(rhs)
                }
                Expr::Call(callee, args) => {
                    // Only cold diverging `core$panicking$*` calls are guaranteed not to clobber
                    // scratch registers and return.
                    if !is_panic_callee(callee) {
                        return ControlFlow::Break(());
                    }
                    for arg in args {
                        self.visit_expr(arg)?;
                    }
                    ControlFlow::Continue(())
                }
                Expr::New(..) => ControlFlow::Break(()),
                Expr::Arrow(..) => ControlFlow::Continue(()),
                _ => jsast::try_walk_expr(self, expr),
            }
        }
    }
    ClobberScan(operands).visit_expr(expr).is_break()
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

fn build_divrem_replacement(
    first: &I64DivCall,
    second: &I64DivCall,
    tmp_lo: String,
    tmp_hi: Option<String>,
) -> (Vec<Stmt>, Vec<Stmt>) {
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
    (first_repl, second_repl)
}

fn fuse_i64_div_rem_block(
    stmts: &mut Vec<Stmt>,
    local_names: &mut HashSet<String>,
    extra_decls: &mut Vec<(String, Option<Expr>)>,
) {
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
        let (first_repl, second_repl) = build_divrem_replacement(&first, &second, tmp_lo, tmp_hi);

        // Splice second first so index `i` stays valid.
        stmts.splice(j..j + second.span, second_repl);
        let first_new_len = first_repl.len();
        stmts.splice(i..i + first.span, first_repl);
        i += first_new_len;
    }
}

pub(super) fn fuse_i64_div_rem(
    stmts: &mut Vec<Stmt>,
    local_names: &mut HashSet<String>,
    extra_decls: &mut Vec<(String, Option<Expr>)>,
) {
    visit_local_blocks_mut(stmts, false, |block| {
        fuse_i64_div_rem_block(block, local_names, extra_decls);
    });
}

// =============================================================================================
// `ToInt32` / `ToUint32` labeled-block diamond folding (`earcut` zorder)
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

fn match_to_int32_fast_path(stmt: &Stmt, label: &str) -> Option<(String, String, bool)> {
    let Stmt::If(guard, fast_then, None) = stmt else {
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
    if wrap_src != src || !matches!(&fast_then[1], Stmt::Break(Some(b)) if b == label) {
        return None;
    }
    Some((src, dst.clone(), fast_unsigned))
}

fn match_to_int32_magnitude(block: &[Stmt], idx: &mut usize, src: &str) -> Option<String> {
    let mut abs_var: Option<String> = None;
    if let Some(Stmt::ExprStmt(Expr::Assign(lhs, rhs))) = block.get(*idx) {
        if let (Expr::Ident(a_name), Some(arg)) = (lhs.as_ref(), match_math_abs_call(rhs)) {
            if arg == src && a_name != src {
                abs_var = Some(a_name.clone());
                *idx += 1;
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
            (None, other) => match_math_abs_call(other) == Some(src),
            _ => false,
        }
    };

    // Match either:
    //   `mag_var = (rem_var = <abs> % 4294967296, clamp(rem_var));`
    // or separate statements:
    //   `rem_var = <abs> % 4294967296; mag_var = clamp(rem_var);`
    let Stmt::ExprStmt(Expr::Assign(lhs, rhs)) = block.get(*idx)? else {
        return None;
    };
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
        if rem_name == src
            || lhs_name == src
            || !is_mod_2p32(rr)
            || !match_u32_clamp_ternary(&items[1], rem_name)
        {
            return None;
        }
        *idx += 1;
        Some(lhs_name.clone())
    } else if is_mod_2p32(rhs) {
        let rem_name = lhs_name.clone();
        if rem_name == src {
            return None;
        }
        *idx += 1;
        let Stmt::ExprStmt(Expr::Assign(ml, mr)) = block.get(*idx)? else {
            return None;
        };
        let Expr::Ident(m_name) = ml.as_ref() else {
            return None;
        };
        if m_name == src || !match_u32_clamp_ternary(mr, &rem_name) {
            return None;
        }
        *idx += 1;
        Some(m_name.clone())
    } else {
        None
    }
}

fn match_to_int32_sign_and_final(
    block: &[Stmt],
    mut idx: usize,
    src: &str,
    dst: &str,
    mag_var: &str,
    fast_unsigned: bool,
) -> bool {
    let Some(Stmt::If(sign_cond, neg_b, Some(pos_b))) = block.get(idx) else {
        return false;
    };
    if !match_lt_zero(sign_cond, src) || pos_b.len() != 1 {
        return false;
    }
    let Stmt::ExprStmt(Expr::Assign(pl, pr)) = &pos_b[0] else {
        return false;
    };
    let Expr::Ident(res_var) = pl.as_ref() else {
        return false;
    };
    let Some((pos_mag, pos_unsigned)) = match_ident_bit32_wrap(pr) else {
        return false;
    };
    if pos_mag != mag_var {
        return false;
    }

    let Some(neg_unsigned) = (match neg_b.as_slice() {
        [Stmt::ExprStmt(Expr::Assign(nl, nr))] => {
            if !matches!(nl.as_ref(), Expr::Ident(n_dst) if n_dst == res_var) {
                return false;
            }
            match_neg_bit32(nr, mag_var)
        }
        [Stmt::ExprStmt(Expr::Assign(n1l, n1r)), Stmt::ExprStmt(Expr::Assign(n2l, n2r))] => {
            let (Expr::Ident(neg_tmp), Expr::Ident(n2_dst)) = (n1l.as_ref(), n2l.as_ref()) else {
                return false;
            };
            if n2_dst != res_var {
                return false;
            }
            let Some(first_unsigned) = match_neg_bit32(n1r, mag_var) else {
                return false;
            };
            match n2r.as_ref() {
                Expr::Ident(v) if v == neg_tmp => Some(first_unsigned),
                other => match_ident_bit32_wrap(other)
                    .filter(|(v, _)| *v == neg_tmp)
                    .map(|(_, u)| u),
            }
        }
        _ => None,
    }) else {
        return false;
    };
    idx += 1;

    // Final assignment to `dst` (optional if `res_var == dst` with matching signedness).
    if idx == block.len() {
        return res_var == dst && neg_unsigned == fast_unsigned && pos_unsigned == fast_unsigned;
    }
    if idx + 1 != block.len() {
        return false;
    }
    let Stmt::ExprStmt(Expr::Assign(fl, fr)) = &block[idx] else {
        return false;
    };
    if !matches!(fl.as_ref(), Expr::Ident(id) if id == dst) {
        return false;
    }
    match fr.as_ref() {
        Expr::Ident(v) => {
            v == res_var && neg_unsigned == fast_unsigned && pos_unsigned == fast_unsigned
        }
        other => matches!(
            match_ident_bit32_wrap(other),
            Some((v, u)) if v == res_var && u == fast_unsigned
        ),
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
    let (src, dst, fast_unsigned) = match_to_int32_fast_path(&block[0], label)?;
    let mut idx = 1usize;
    let mag_var = match_to_int32_magnitude(block, &mut idx, &src)?;
    match_to_int32_sign_and_final(block, idx, &src, &dst, &mag_var, fast_unsigned).then_some((
        dst,
        src,
        fast_unsigned,
    ))
}

fn stmt_may_throw(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Throw(_) => true,
        Stmt::ExprStmt(e) | Stmt::Return(Some(e)) | Stmt::Let(_, Some(e)) | Stmt::Const(_, e) => {
            expr_has_side_effect(e)
        }
        Stmt::Lets(decls) => decls
            .iter()
            .filter_map(|(_, init)| init.as_ref())
            .any(expr_has_side_effect),
        Stmt::If(c, t, e) => {
            expr_has_side_effect(c)
                || t.iter().any(stmt_may_throw)
                || e.as_ref().is_some_and(|eb| eb.iter().any(stmt_may_throw))
        }
        Stmt::Block(b) | Stmt::LoopForever(_, b) => b.iter().any(stmt_may_throw),
        Stmt::While(c, b) => expr_has_side_effect(c) || b.iter().any(stmt_may_throw),
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
            if in_try && (expr_has_side_effect(lhs) || expr_has_side_effect(rhs)) {
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

struct Int32DiamondFolder<'a> {
    local_names: &'a HashSet<String>,
    total_reads: HashMap<String, usize>,
    closure_vars: HashSet<String>,
    at_fn_top_level: bool,
    in_try: bool,
}

impl Int32DiamondFolder<'_> {
    fn fold_slice(&mut self, stmts: &mut [Stmt]) {
        // Recurse into nested control flow first.
        jsast::walk_block_mut(self, stmts);

        // Walk back-to-front so folding a later diamond removes its slow-path reads of reused
        // scratch locals before checking whether an earlier diamond's clobbered locals are dead.
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
            collect_written_idents_stmts(&block[1..], &mut slow_writes);
            slow_writes.remove(&dst);

            let mut block_reads: HashMap<String, usize> = HashMap::new();
            tally_ident_reads_stmt(&stmts[i], &mut block_reads);

            let trailing = &stmts[i + 1..];
            let all_dead = slow_writes.iter().all(|v| {
                if !self.local_names.contains(v) || self.closure_vars.contains(v) {
                    return false;
                }
                let tot = self.total_reads.get(v).copied().unwrap_or(0);
                let blk = block_reads.get(v).copied().unwrap_or(0);
                if tot <= blk {
                    return true;
                }
                is_definitely_dead_in_trailing(v, trailing, self.at_fn_top_level, self.in_try)
            });
            if !all_dead {
                continue;
            }

            // Update read counts to reflect replacing the diamond with `dst = src (| 0 | >>> 0);`.
            for (var, cnt) in block_reads {
                if let Some(entry) = self.total_reads.get_mut(&var) {
                    *entry = entry.saturating_sub(cnt);
                }
            }
            *self.total_reads.entry(src.clone()).or_insert(0) += 1;

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
}

impl VisitMut for Int32DiamondFolder<'_> {
    fn visit_block_mut(&mut self, stmts: &mut Vec<Stmt>) {
        self.fold_slice(stmts);
    }

    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::FunctionDecl { .. } => {}
            Stmt::Block(b) => self.fold_slice(b),
            Stmt::Labeled(_, inner) => {
                if let Stmt::Block(b) | Stmt::LoopForever(_, b) | Stmt::While(_, b) = inner.as_mut()
                {
                    let prev = self.at_fn_top_level;
                    self.at_fn_top_level = false;
                    self.fold_slice(b);
                    self.at_fn_top_level = prev;
                }
            }
            Stmt::TryCatch { body, handler, .. } => {
                let prev_top = self.at_fn_top_level;
                let prev_try = self.in_try;
                self.at_fn_top_level = false;
                self.in_try = true;
                self.fold_slice(body);
                self.in_try = prev_try;
                self.fold_slice(handler);
                self.at_fn_top_level = prev_top;
            }
            _ => {
                let prev = self.at_fn_top_level;
                self.at_fn_top_level = false;
                jsast::walk_stmt_mut(self, stmt);
                self.at_fn_top_level = prev;
            }
        }
    }

    fn visit_expr_mut(&mut self, _expr: &mut Expr) {}
}

pub(super) fn fold_to_int32_diamonds(body: &mut [Stmt], local_names: &HashSet<String>) {
    let total_reads = tally_ident_reads(body);
    let mut usage: HashMap<String, LocalConstUsage> = HashMap::new();
    scan_const_usage_stmts(body, local_names, &mut usage);
    let closure_vars: HashSet<String> = usage
        .into_iter()
        .filter(|(_, u)| u.in_closure)
        .map(|(n, _)| n)
        .collect();
    let mut folder = Int32DiamondFolder {
        local_names,
        total_reads,
        closure_vars,
        at_fn_top_level: true,
        in_try: false,
    };
    folder.fold_slice(body);
}
