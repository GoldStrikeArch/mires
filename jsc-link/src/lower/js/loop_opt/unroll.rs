//! Induction-variable analysis, small constant-trip loop unrolling, counted loop step
//! strength reduction (Smi-ization), and loop/branch body transforms (`fold_guarded_f2i`,
//! `elide_dead_fresh_slice_clones`).

use std::collections::{HashMap, HashSet};

use rcgjs_object::jsast::{self, ArrowBody, BinOp, Expr, Stmt, UnOp, VisitMut};

use super::super::ast_util::{
    collect_written_idents_expr, collect_written_idents_stmts_opt as collect_written_idents_stmts,
    exact_i32, exact_u32, visit_local_blocks_mut,
};
use super::util::{
    count_rw_stmts, count_stmts, expr_has_non_panic_call, expr_reads_any_in, expr_reads_ident,
    expr_writes_ident, extract_iter0_then_body_mut, stmts_definitely_exit_loop,
    stmts_have_control_flow, stmts_have_mutating_nested_fn, stmts_have_non_panic_call,
    stmts_have_var_decl, stmts_read_ident, subst_ident_in_stmt,
};

// =================================================================================================
// Induction-Variable Detection (shared with LICM & Loop Versioning)
// =================================================================================================

#[derive(Clone)]
pub(super) struct CounterInfo {
    pub(super) iv: String,
    pub(super) bound: Expr, // Ident or exact_u32 Num
    pub(super) iv_min: Option<u32>,
}

pub(super) fn find_prior_const_init(stmts: &[Stmt], loop_idx: usize, iv: &str) -> Option<u32> {
    let mut k = loop_idx;
    let mut steps = 0;
    while k > 0 && steps < 4 {
        k -= 1;
        steps += 1;
        match &stmts[k] {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                if matches!(lhs.as_ref(), Expr::Ident(id) if id == iv) {
                    return exact_u32(rhs);
                }
                if expr_has_non_panic_call(rhs) || expr_reads_ident(rhs, iv) {
                    return None;
                }
                if !matches!(lhs.as_ref(), Expr::Ident(_)) {
                    return None;
                }
            }
            _ => return None,
        }
    }
    None
}

fn is_iv_plus_one(expr: &Expr, iv: &str) -> bool {
    let inner = match expr {
        Expr::Binary(BinOp::UShr, l, r) if exact_u32(r) == Some(0) => l.as_ref(),
        Expr::Binary(BinOp::BitOr, l, r) if exact_i32(r) == Some(0) => l.as_ref(),
        other => other,
    };
    matches!(
        inner,
        Expr::Binary(BinOp::Add, l, r)
            if (matches!(l.as_ref(), Expr::Ident(id) if id == iv) && exact_u32(r) == Some(1))
                || (exact_u32(l) == Some(1) && matches!(r.as_ref(), Expr::Ident(id) if id == iv))
    )
}

pub(super) fn detect_loop_counter(
    enclosing: &[Stmt],
    loop_idx: usize,
    assigned_in_loop: &HashSet<String>,
    pre_loop_consts: Option<&HashMap<String, u32>>,
) -> Option<CounterInfo> {
    let lp = enclosing[loop_idx].as_loop()?;
    let (iv, bound, body): (String, Expr, &[Stmt]) = if let Some(cond) = lp.cond {
        let Expr::Binary(BinOp::Lt, l, r) = cond else {
            return None;
        };
        let Expr::Ident(iv) = l.as_ref() else {
            return None;
        };
        (iv.clone(), r.as_ref().clone(), lp.body)
    } else {
        match lp.body {
            [Stmt::If(Expr::Binary(BinOp::Lt, l, r), then_b, Some(else_b))]
                if stmts_definitely_exit_loop(else_b, lp.label) =>
            {
                let Expr::Ident(iv) = l.as_ref() else {
                    return None;
                };
                (iv.clone(), r.as_ref().clone(), then_b.as_slice())
            }
            [Stmt::ExprStmt(Expr::Assign(fl, fr)), Stmt::If(Expr::Ident(fu), then_b, Some(else_b))]
                if matches!(fl.as_ref(), Expr::Ident(fn_) if fn_ == fu)
                    && stmts_definitely_exit_loop(else_b, lp.label) =>
            {
                let Expr::Binary(BinOp::Lt, l, r) = fr.as_ref() else {
                    return None;
                };
                let Expr::Ident(iv) = l.as_ref() else {
                    return None;
                };
                (iv.clone(), r.as_ref().clone(), then_b.as_slice())
            }
            _ => return None,
        }
    };
    // `bound` must be an exact u32 constant or a loop-invariant identifier.
    let bound_invariant = match &bound {
        Expr::Ident(b_id) => b_id != &iv && !assigned_in_loop.contains(b_id),
        other => exact_u32(other).is_some(),
    };
    if !bound_invariant {
        return None;
    }
    // Verify EVERY assignment to `iv` inside `body` is a monotonic `+ 1` step.
    if !all_iv_writes_are_plus_one(body, &iv) {
        return None;
    }
    let iv_min = find_prior_const_init(enclosing, loop_idx, &iv)
        .or_else(|| pre_loop_consts.and_then(|m| m.get(&iv).copied()));
    Some(CounterInfo { iv, bound, iv_min })
}

fn all_iv_writes_are_plus_one(stmts: &[Stmt], iv: &str) -> bool {
    fn check_stmts(
        stmts: &[Stmt],
        iv: &str,
        local_plus_one: &mut HashSet<String>,
        saw_write: &mut bool,
    ) -> bool {
        for s in stmts {
            match s {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                    if expr_writes_ident(rhs, iv) {
                        return false;
                    }
                    if let Expr::Ident(id) = lhs.as_ref() {
                        if id == iv {
                            *saw_write = true;
                            let ok = is_iv_plus_one(rhs, iv)
                                || matches!(rhs.as_ref(), Expr::Ident(n) if local_plus_one.contains(n));
                            if !ok {
                                return false;
                            }
                            local_plus_one.clear();
                        } else if is_iv_plus_one(rhs, iv) {
                            local_plus_one.insert(id.clone());
                        } else {
                            local_plus_one.remove(id);
                        }
                    } else if expr_writes_ident(lhs, iv) {
                        return false;
                    }
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                    if expr_writes_ident(e, iv) {
                        return false;
                    }
                }
                Stmt::If(c, a, b) => {
                    if expr_writes_ident(c, iv) {
                        return false;
                    }
                    let mut a_p = local_plus_one.clone();
                    if !check_stmts(a, iv, &mut a_p, saw_write) {
                        return false;
                    }
                    if let Some(b) = b {
                        let mut b_p = local_plus_one.clone();
                        if !check_stmts(b, iv, &mut b_p, saw_write) {
                            return false;
                        }
                    }
                    let mut w = HashSet::new();
                    collect_written_idents_stmts(a, &mut w, true);
                    if let Some(b) = b {
                        collect_written_idents_stmts(b, &mut w, true);
                    }
                    for k in w {
                        local_plus_one.remove(&k);
                    }
                }
                Stmt::Block(b) => {
                    if !check_stmts(b, iv, local_plus_one, saw_write) {
                        return false;
                    }
                }
                Stmt::Labeled(_, inner) => {
                    if !check_stmts(
                        std::slice::from_ref(inner.as_ref()),
                        iv,
                        local_plus_one,
                        saw_write,
                    ) {
                        return false;
                    }
                }
                Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
                    let mut w = HashSet::new();
                    collect_written_idents_stmts(b, &mut w, true);
                    if w.contains(iv) {
                        return false;
                    }
                    for k in w {
                        local_plus_one.remove(&k);
                    }
                }
                Stmt::Switch(d, cases) => {
                    if expr_writes_ident(d, iv) {
                        return false;
                    }
                    for c in cases {
                        let mut w = HashSet::new();
                        collect_written_idents_stmts(&c.body, &mut w, true);
                        if w.contains(iv) {
                            return false;
                        }
                    }
                    local_plus_one.clear();
                }
                _ => {}
            }
        }
        true
    }
    let mut local_plus_one: HashSet<String> = HashSet::new();
    let mut saw_write = false;
    check_stmts(stmts, iv, &mut local_plus_one, &mut saw_write) && saw_write
}

// =================================================================================================
// 1. Guarded `f2i` Folding & Nested `if` Collapse
// =================================================================================================

pub(in super::super) fn fold_guarded_f2i(stmts: &mut [Stmt]) {
    collapse_nested_ifs(stmts);
    let mut in_i32_range: HashSet<String> = HashSet::new();
    fold_f2i_in_stmts(stmts, &mut in_i32_range);
}

fn collapse_nested_ifs(stmts: &mut [Stmt]) {
    struct Collapser;
    impl VisitMut for Collapser {
        fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
            if matches!(stmt, Stmt::FunctionDecl { .. }) {
                return;
            }
            jsast::walk_stmt_mut(self, stmt);
            if let Stmt::If(cond, then_body, None) = stmt {
                while then_body.len() == 1
                    && matches!(then_body.first(), Some(Stmt::If(_, _, None)))
                {
                    let Some(Stmt::If(inner_cond, inner_then, None)) = then_body.pop() else {
                        break;
                    };
                    let outer = std::mem::replace(cond, Expr::Bool(false));
                    *cond = Expr::Binary(BinOp::And, Box::new(outer), Box::new(inner_cond));
                    *then_body = inner_then;
                }
            }
        }
        fn visit_expr_mut(&mut self, _: &mut Expr) {}
    }
    Collapser.visit_stmts_mut(stmts);
}

/// Checks if `cond` (conjunction of comparisons) proves `v >= -2147483648 && v <= 2147483647`.
fn extract_i32_range_vars(cond: &Expr) -> Vec<String> {
    let mut ge_min: HashSet<String> = HashSet::new();
    let mut le_max: HashSet<String> = HashSet::new();
    fn visit(e: &Expr, ge_min: &mut HashSet<String>, le_max: &mut HashSet<String>) {
        match e {
            Expr::Binary(BinOp::And, l, r) => {
                visit(l, ge_min, le_max);
                visit(r, ge_min, le_max);
            }
            Expr::Binary(BinOp::Ge, l, r) => {
                if let (Expr::Ident(v), Some(-2147483648)) = (l.as_ref(), exact_i32(r)) {
                    ge_min.insert(v.clone());
                } else if let (Some(2147483647), Expr::Ident(v)) = (exact_i32(l), r.as_ref()) {
                    le_max.insert(v.clone());
                }
            }
            Expr::Binary(BinOp::Le, l, r) => {
                if let (Expr::Ident(v), Some(2147483647)) = (l.as_ref(), exact_i32(r)) {
                    le_max.insert(v.clone());
                } else if let (Some(-2147483648), Expr::Ident(v)) = (exact_i32(l), r.as_ref()) {
                    ge_min.insert(v.clone());
                }
            }
            _ => {}
        }
    }
    visit(cond, &mut ge_min, &mut le_max);
    ge_min.intersection(&le_max).cloned().collect()
}

/// Matches `v <= -2147483648 ? -2147483648 : (v >= 2147483647 ? 2147483647 : v | 0)` for `v` in
/// `in_i32_range` and replaces it with `v | 0`.
fn is_i32_clamp_for(expr: &Expr, in_i32_range: &HashSet<String>) -> Option<String> {
    let Expr::Cond(c1, t1, e1) = expr else {
        return None;
    };
    let Expr::Binary(BinOp::Le, c1_l, c1_r) = c1.as_ref() else {
        return None;
    };
    let Expr::Ident(v) = c1_l.as_ref() else {
        return None;
    };
    if !in_i32_range.contains(v)
        || exact_i32(c1_r) != Some(-2147483648)
        || exact_i32(t1) != Some(-2147483648)
    {
        return None;
    }
    let Expr::Cond(c2, t2, e2) = e1.as_ref() else {
        return None;
    };
    let Expr::Binary(BinOp::Ge, c2_l, c2_r) = c2.as_ref() else {
        return None;
    };
    if !matches!(c2_l.as_ref(), Expr::Ident(v2) if v2 == v)
        || exact_i32(c2_r) != Some(2147483647)
        || exact_i32(t2) != Some(2147483647)
    {
        return None;
    }
    let Expr::Binary(BinOp::BitOr, e2_l, e2_r) = e2.as_ref() else {
        return None;
    };
    if matches!(e2_l.as_ref(), Expr::Ident(v3) if v3 == v) && exact_i32(e2_r) == Some(0) {
        Some(v.clone())
    } else {
        None
    }
}

fn rewrite_f2i_expr(expr: &mut Expr, in_i32_range: &mut HashSet<String>) {
    struct Rewriter<'a>(&'a mut HashSet<String>);
    impl VisitMut for Rewriter<'_> {
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            if let Some(v) = is_i32_clamp_for(expr, self.0) {
                *expr = Expr::Binary(
                    BinOp::BitOr,
                    Box::new(Expr::Ident(v)),
                    Box::new(Expr::Num(0.0)),
                );
                return;
            }
            match expr {
                Expr::Arrow(..) => {}
                Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
                    self.visit_expr_mut(rhs);
                    if let Expr::Ident(n) = lhs.as_ref() {
                        self.0.remove(n);
                    } else {
                        self.visit_expr_mut(lhs);
                    }
                }
                Expr::Cond(c, t, e) => {
                    self.visit_expr_mut(c);
                    let mut t_set = self.0.clone();
                    for v in extract_i32_range_vars(c) {
                        t_set.insert(v);
                    }
                    Rewriter(&mut t_set).visit_expr_mut(t);
                    let mut e_set = self.0.clone();
                    Rewriter(&mut e_set).visit_expr_mut(e);
                    // Any variable written in either branch is invalidated.
                    let mut written = HashSet::new();
                    collect_written_idents_expr(t, &mut written);
                    collect_written_idents_expr(e, &mut written);
                    for w in written {
                        self.0.remove(&w);
                    }
                }
                _ => jsast::walk_expr_mut(self, expr),
            }
        }
    }
    Rewriter(in_i32_range).visit_expr_mut(expr);
}

fn fold_f2i_in_stmts(stmts: &mut [Stmt], in_i32_range: &mut HashSet<String>) {
    for stmt in stmts.iter_mut() {
        if let Some(lp) = stmt.as_loop_mut() {
            let mut written = HashSet::new();
            if let Some(cond) = lp.cond.as_deref() {
                collect_written_idents_expr(cond, &mut written);
            }
            collect_written_idents_stmts(lp.body, &mut written, false);
            for w in &written {
                in_i32_range.remove(w);
            }
            let mut b_set = in_i32_range.clone();
            if let Some(cond) = lp.cond {
                rewrite_f2i_expr(cond, in_i32_range);
                for v in extract_i32_range_vars(cond) {
                    b_set.insert(v);
                }
            }
            fold_f2i_in_stmts(lp.body, &mut b_set);
            continue;
        }
        match stmt {
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                rewrite_f2i_expr(e, in_i32_range);
            }
            Stmt::Let(n, init) => {
                if let Some(e) = init {
                    rewrite_f2i_expr(e, in_i32_range);
                }
                in_i32_range.remove(n);
            }
            Stmt::Const(n, e) => {
                rewrite_f2i_expr(e, in_i32_range);
                in_i32_range.remove(n);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(e) = init {
                        rewrite_f2i_expr(e, in_i32_range);
                    }
                    in_i32_range.remove(n);
                }
            }
            Stmt::If(cond, then_body, else_body) => {
                rewrite_f2i_expr(cond, in_i32_range);
                let mut then_set = in_i32_range.clone();
                for v in extract_i32_range_vars(cond) {
                    then_set.insert(v);
                }
                fold_f2i_in_stmts(then_body, &mut then_set);
                if let Some(eb) = else_body {
                    let mut else_set = in_i32_range.clone();
                    fold_f2i_in_stmts(eb, &mut else_set);
                }
                let mut written = HashSet::new();
                collect_written_idents_stmts(then_body, &mut written, false);
                if let Some(eb) = else_body {
                    collect_written_idents_stmts(eb, &mut written, false);
                }
                for w in written {
                    in_i32_range.remove(&w);
                }
            }
            Stmt::Switch(disc, cases) => {
                rewrite_f2i_expr(disc, in_i32_range);
                let mut written = HashSet::new();
                for c in cases.iter_mut() {
                    let mut c_set = in_i32_range.clone();
                    fold_f2i_in_stmts(&mut c.body, &mut c_set);
                    collect_written_idents_stmts(&c.body, &mut written, false);
                }
                for w in written {
                    in_i32_range.remove(&w);
                }
            }
            Stmt::Block(b) => fold_f2i_in_stmts(b, in_i32_range),
            Stmt::Labeled(_, inner) => {
                let mut written = HashSet::new();
                collect_written_idents_stmts(
                    std::slice::from_ref(inner.as_ref()),
                    &mut written,
                    false,
                );
                let mut inner_set = in_i32_range.clone();
                fold_f2i_in_stmts(std::slice::from_mut(inner.as_mut()), &mut inner_set);
                for w in written {
                    in_i32_range.remove(&w);
                }
            }
            Stmt::TryCatch { body, handler, .. } => {
                let mut written = HashSet::new();
                collect_written_idents_stmts(body, &mut written, false);
                collect_written_idents_stmts(handler, &mut written, false);
                let mut b_set = in_i32_range.clone();
                fold_f2i_in_stmts(body, &mut b_set);
                let mut h_set = in_i32_range.clone();
                for w in &written {
                    h_set.remove(w);
                    in_i32_range.remove(w);
                }
                fold_f2i_in_stmts(handler, &mut h_set);
            }
            _ => {}
        }
    }
}

// =================================================================================================
// 2. Dead Fresh `.slice()` Clone Elision (`aC = [...]; Z = aC.slice(); data: Z.slice()`)
// =================================================================================================

pub(in super::super) fn elide_dead_fresh_slice_clones(
    stmts: &mut [Stmt],
    local_names: &HashSet<String>,
) {
    // Run up to 3 rounds so chained `aC = [...]; Z = aC.slice(); data: Z.slice()` collapses fully.
    for _ in 0..3 {
        let mut reads: HashMap<String, usize> = HashMap::new();
        let mut writes: HashMap<String, usize> = HashMap::new();
        count_rw_stmts(stmts, &mut reads, &mut writes);
        let mut elider = FreshSliceElider {
            local_names,
            reads: &reads,
            writes: &writes,
            changed: false,
        };
        elider.visit_slice(stmts);
        if !elider.changed {
            break;
        }
    }
}

fn fresh_array_obj_keys(expr: &Expr) -> Option<Vec<String>> {
    let Expr::Array(items) = expr else {
        return None;
    };
    if items.is_empty() {
        return None;
    }
    let mut expected: Option<Vec<String>> = None;
    for item in items {
        let Expr::Object(entries) = item else {
            return None;
        };
        let mut keys: Vec<String> = entries.iter().map(|(k, _)| k.clone()).collect();
        keys.sort();
        keys.dedup();
        if keys.len() != entries.len() {
            return None;
        }
        match &expected {
            None => expected = Some(keys),
            Some(prev) if prev == &keys => {}
            Some(_) => return None,
        }
    }
    expected
}

fn is_void_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Undefined)
        || matches!(expr, Expr::Unary(UnOp::Void, inner) if exact_u32(inner) == Some(0))
}

fn is_shallow_obj_copy_of(expr: &Expr, param: &str, expected_keys: &[String]) -> bool {
    let Expr::Object(entries) = expr else {
        return false;
    };
    if entries.len() != expected_keys.len() {
        return false;
    }
    let mut keys: Vec<String> = Vec::with_capacity(entries.len());
    for (k, v) in entries {
        let ok = matches!(
            v,
            Expr::Member(obj, prop) if prop == k && matches!(obj.as_ref(), Expr::Ident(id) if id == param)
        );
        if !ok {
            return false;
        }
        keys.push(k.clone());
    }
    keys.sort();
    keys == expected_keys
}

fn is_shallow_obj_clone_map(expr: &Expr, target: &str, expected_keys: &[String]) -> bool {
    let Expr::Call(callee, args) = expr else {
        return false;
    };
    if args.len() != 1 {
        return false;
    }
    let is_target_map = matches!(
        callee.as_ref(),
        Expr::Member(obj, prop) if prop == "map" && matches!(obj.as_ref(), Expr::Ident(id) if id == target)
    );
    if !is_target_map {
        return false;
    }
    let Expr::Arrow(params, ArrowBody::Expr(body)) = &args[0] else {
        return false;
    };
    if params.len() != 1 {
        return false;
    }
    let p = &params[0];
    if is_shallow_obj_copy_of(body, p, expected_keys) {
        return true;
    }
    if let Expr::Cond(cond, then_e, else_e) = body.as_ref() {
        let is_p_ne_undef = |e: &Expr| -> Option<bool> {
            let Expr::Binary(op, l, r) = e else {
                return None;
            };
            let matches_pair = (matches!(l.as_ref(), Expr::Ident(id) if id == p)
                && is_void_expr(r))
                || (is_void_expr(l) && matches!(r.as_ref(), Expr::Ident(id) if id == p));
            if !matches_pair {
                return None;
            }
            match op {
                BinOp::StrictNe => Some(true),
                BinOp::StrictEq => Some(false),
                _ => None,
            }
        };
        if let Some(true) = is_p_ne_undef(cond) {
            return is_shallow_obj_copy_of(then_e, p, expected_keys) && is_void_expr(else_e);
        }
        if let Some(false) = is_p_ne_undef(cond) {
            return is_void_expr(then_e) && is_shallow_obj_copy_of(else_e, p, expected_keys);
        }
    }
    false
}

fn is_fresh_array_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Array(_) => true,
        Expr::Call(callee, args) if args.is_empty() => {
            matches!(callee.as_ref(), Expr::Member(_, prop) if prop == "slice")
        }
        _ => false,
    }
}

fn extract_fresh_array_def(
    stmt: &Stmt,
    fresh_vars: &HashSet<String>,
    fresh_obj_keys: &HashMap<String, Vec<String>>,
) -> Option<(String, Option<Vec<String>>)> {
    fn from_expr(
        expr: &Expr,
        fresh_vars: &HashSet<String>,
        fresh_obj_keys: &HashMap<String, Vec<String>>,
    ) -> Option<(String, Option<Vec<String>>)> {
        match expr {
            Expr::Assign(lhs, rhs) => {
                let Expr::Ident(name) = lhs.as_ref() else {
                    return None;
                };
                if let Some(keys) = fresh_array_obj_keys(rhs) {
                    return Some((name.clone(), Some(keys)));
                }
                if let Expr::Ident(src) = rhs.as_ref() {
                    if fresh_vars.contains(src) {
                        return Some((name.clone(), fresh_obj_keys.get(src).cloned()));
                    }
                }
                if is_fresh_array_expr(rhs) {
                    return Some((name.clone(), None));
                }
                None
            }
            Expr::Seq(items) => items
                .last()
                .and_then(|e| from_expr(e, fresh_vars, fresh_obj_keys)),
            _ => None,
        }
    }
    match stmt {
        Stmt::ExprStmt(e) => from_expr(e, fresh_vars, fresh_obj_keys),
        _ => None,
    }
}

fn replace_ident_slice_call_stmt(
    stmt: &mut Stmt,
    target: &str,
    expected_obj_keys: Option<&[String]>,
) -> bool {
    struct Replacer<'a> {
        target: &'a str,
        expected_obj_keys: Option<&'a [String]>,
        changed: bool,
    }
    impl VisitMut for Replacer<'_> {
        fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
            if !matches!(stmt, Stmt::FunctionDecl { .. }) {
                jsast::walk_stmt_mut(self, stmt);
            }
        }
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            if matches!(expr, Expr::Arrow(..)) {
                return;
            }
            if let Expr::Call(callee, args) = expr {
                if args.is_empty() {
                    if let Expr::Member(obj, prop) = callee.as_ref() {
                        if prop == "slice"
                            && matches!(obj.as_ref(), Expr::Ident(id) if id == self.target)
                        {
                            *expr = Expr::Ident(self.target.to_owned());
                            self.changed = true;
                            return;
                        }
                    }
                }
            }
            if let Some(keys) = self.expected_obj_keys {
                if is_shallow_obj_clone_map(expr, self.target, keys) {
                    *expr = Expr::Ident(self.target.to_owned());
                    self.changed = true;
                    return;
                }
            }
            jsast::walk_expr_mut(self, expr);
        }
    }
    let mut replacer = Replacer {
        target,
        expected_obj_keys,
        changed: false,
    };
    replacer.visit_stmt_mut(stmt);
    replacer.changed
}

struct FreshSliceElider<'a> {
    local_names: &'a HashSet<String>,
    reads: &'a HashMap<String, usize>,
    writes: &'a HashMap<String, usize>,
    changed: bool,
}

impl FreshSliceElider<'_> {
    fn visit_slice(&mut self, stmts: &mut [Stmt]) {
        let mut fresh_vars: HashSet<String> = HashSet::new();
        let mut fresh_obj_keys: HashMap<String, Vec<String>> = HashMap::new();
        for i in 0..stmts.len() {
            // Recurse into child blocks first.
            self.visit_stmt_mut(&mut stmts[i]);
            if let Some((v, obj_keys)) =
                extract_fresh_array_def(&stmts[i], &fresh_vars, &fresh_obj_keys)
            {
                if self.local_names.contains(&v)
                    && self.writes.get(&v).copied() == Some(1)
                    && self.reads.get(&v).copied() == Some(1)
                {
                    fresh_vars.insert(v.clone());
                    if let Some(ref k) = obj_keys {
                        fresh_obj_keys.insert(v.clone(), k.clone());
                    }
                    // Replace the single `v.slice()` or pure shallow-object-copy `v.map(...)` in
                    // trailing sibling statements.
                    for tail in &mut stmts[i + 1..] {
                        if replace_ident_slice_call_stmt(tail, &v, obj_keys.as_deref()) {
                            self.changed = true;
                            break;
                        }
                    }
                }
            }
        }
    }
}

impl VisitMut for FreshSliceElider<'_> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        if !matches!(stmt, Stmt::FunctionDecl { .. }) {
            jsast::walk_stmt_mut(self, stmt);
        }
    }

    fn visit_expr_mut(&mut self, _: &mut Expr) {}

    fn visit_block_mut(&mut self, stmts: &mut Vec<Stmt>) {
        self.visit_slice(stmts);
    }
}

// =================================================================================================
// 3. Small Constant-Trip Loop Unrolling (`2..=10` iters, bottom-up)
// =================================================================================================

pub(in super::super) fn unroll_small_const_loops(stmts: &mut Vec<Stmt>) -> bool {
    let mut changed = false;
    // Bottom-up: unroll inner loops first so outer small loops (like `sha3::permute` `m=0..5`
    // containing `n=0..5`) can also unroll cleanly.
    visit_local_blocks_mut(stmts, false, |block| {
        let mut i = 0;
        while i < block.len() {
            let Some(unrolled) = try_unroll_loop_at(block, i) else {
                i += 1;
                continue;
            };
            let new_len = unrolled.len();
            block.splice(i..=i, unrolled);
            i += new_len;
            changed = true;
        }
    });
    changed
}

struct ParsedConstLoop {
    iv: String,
    flag: Option<String>,
    c_start: u32,
    c_end: u32,
    /// Statements inside the loop body (with the final `iv = next` and optional `next = iv + 1`
    /// already stripped).
    iter_body: Vec<Stmt>,
    /// Exit statements from the `else { ...; break; }` branch.
    exit_stmts: Vec<Stmt>,
}

fn try_unroll_loop_at(stmts: &[Stmt], loop_idx: usize) -> Option<Vec<Stmt>> {
    let parsed = parse_small_const_loop(stmts, loop_idx)?;
    let trip = parsed.c_end - parsed.c_start;
    if !(2..=10).contains(&trip) {
        return None;
    }
    let body_cost = count_stmts(&parsed.iter_body);
    if body_cost == 0 || (trip as usize) * body_cost > 160 {
        return None;
    }

    let mut out = Vec::with_capacity((trip as usize) * parsed.iter_body.len() + 3);
    for k in parsed.c_start..parsed.c_end {
        for s in &parsed.iter_body {
            let mut cloned = s.clone();
            subst_ident_in_stmt(&mut cloned, &parsed.iv, &Expr::Num(f64::from(k)));
            out.push(cloned);
        }
    }
    out.push(jsast::assign_stmt(
        jsast::id(&parsed.iv),
        Expr::Num(f64::from(parsed.c_end)),
    ));
    if let Some(flag) = parsed.flag {
        out.push(jsast::assign_stmt(jsast::id(flag), Expr::Bool(false)));
    }
    out.extend(parsed.exit_stmts);
    Some(out)
}

fn parse_small_const_loop(stmts: &[Stmt], loop_idx: usize) -> Option<ParsedConstLoop> {
    let (iv, flag, c_end, then_body, exit_stmts) = match &stmts[loop_idx] {
        Stmt::While(Expr::Binary(BinOp::Lt, l, r), body) => {
            let Expr::Ident(iv) = l.as_ref() else {
                return None;
            };
            let c_end = exact_u32(r)?;
            (iv.clone(), None, c_end, body.clone(), Vec::new())
        }
        Stmt::LoopForever(None, loop_body) => {
            // Form A: `[if (iv < C_end) { ... } else { exit...; break; }]`
            // Form B: `[flag = iv < C_end; if (flag) { ... } else { exit...; break; }]`
            let (iv, flag, c_end, then_b, else_b) = match loop_body.as_slice() {
                [Stmt::If(Expr::Binary(BinOp::Lt, l, r), then_b, Some(else_b))] => {
                    let Expr::Ident(iv) = l.as_ref() else {
                        return None;
                    };
                    (iv.clone(), None, exact_u32(r)?, then_b, else_b)
                }
                [Stmt::ExprStmt(Expr::Assign(flag_lhs, flag_rhs)), Stmt::If(Expr::Ident(flag_use), then_b, Some(else_b))] =>
                {
                    let Expr::Ident(flag_name) = flag_lhs.as_ref() else {
                        return None;
                    };
                    if flag_name != flag_use {
                        return None;
                    }
                    let Expr::Binary(BinOp::Lt, l, r) = flag_rhs.as_ref() else {
                        return None;
                    };
                    let Expr::Ident(iv) = l.as_ref() else {
                        return None;
                    };
                    (
                        iv.clone(),
                        Some(flag_name.clone()),
                        exact_u32(r)?,
                        then_b,
                        else_b,
                    )
                }
                _ => return None,
            };
            if !matches!(else_b.last(), Some(Stmt::Break(None))) {
                return None;
            }
            let exit = else_b[..else_b.len() - 1].to_vec();
            if stmts_have_control_flow(&exit) || stmts_have_non_panic_call(&exit) {
                return None;
            }
            (iv, flag, c_end, then_b.clone(), exit)
        }
        _ => return None,
    };

    let c_start = find_prior_const_init(stmts, loop_idx, &iv)?;
    if c_start >= c_end {
        return None;
    }
    if stmts_have_control_flow(&then_body) || stmts_have_non_panic_call(&then_body) {
        return None;
    }

    // Verify the final statement increments `iv` by 1 and `iv` is not written anywhere else.
    let mut iter_body = then_body;
    let last = iter_body.pop()?;
    let Stmt::ExprStmt(Expr::Assign(last_lhs, last_rhs)) = last else {
        return None;
    };
    if !matches!(last_lhs.as_ref(), Expr::Ident(id) if id == &iv) {
        return None;
    }
    if is_iv_plus_one(&last_rhs, &iv) {
        // Direct `iv = iv + 1 >>> 0;`
    } else if let Expr::Ident(next_var) = last_rhs.as_ref() {
        // `next = iv + 1 >>> 0;` as first statement of `iter_body`, never modified in `iter_body`.
        let first_matches = matches!(
            iter_body.first(),
            Some(Stmt::ExprStmt(Expr::Assign(fl, fr)))
                if matches!(fl.as_ref(), Expr::Ident(id) if id == next_var)
                    && is_iv_plus_one(fr, &iv)
        );
        if !first_matches {
            return None;
        }
        let mut rest_written = HashSet::new();
        collect_written_idents_stmts(&iter_body[1..], &mut rest_written, false);
        if rest_written.contains(next_var) {
            return None;
        }
        // Only remove `next = iv + 1` if `next` is not read anywhere else in `iter_body[1..]` or
        // `exit_stmts` (if it is read, keep it so `next` gets substituted with `k + 1`!).
        let next_read_in_body = stmts_read_ident(&iter_body[1..], next_var);
        let next_read_after = stmts_read_ident(&exit_stmts, next_var)
            || stmts_read_ident(&stmts[loop_idx + 1..], next_var);
        if !next_read_in_body && !next_read_after {
            iter_body.remove(0);
        }
    } else {
        return None;
    }

    // Ensure `iv` is never modified inside `iter_body`, and no `let`/`const` declarations exist
    // inside `iter_body` that would collide across unrolled copies.
    let mut body_written = HashSet::new();
    collect_written_idents_stmts(&iter_body, &mut body_written, false);
    if body_written.contains(&iv) || stmts_have_var_decl(&iter_body) {
        return None;
    }

    Some(ParsedConstLoop {
        iv,
        flag,
        c_start,
        c_end,
        iter_body,
        exit_stmts,
    })
}

// =================================================================================================
// 7. Counted Loop Step Smi-ization (`i = i + 1 >>> 0` -> `i = i + 1 | 0`)
// =================================================================================================

#[derive(Clone, Default)]
struct SmiEnv {
    smi_bounds: HashSet<String>,
    nonneg_i31: HashSet<String>,
}

impl SmiEnv {
    fn invalidate(&mut self, ident: &str) {
        self.smi_bounds.remove(ident);
        self.nonneg_i31.remove(ident);
    }

    fn invalidate_all(&mut self, idents: &HashSet<String>) {
        for ident in idents {
            self.invalidate(ident);
        }
    }

    fn record_binding(&mut self, name: &str, init: Option<&Expr>) {
        let is_bound = init.is_some_and(|e| is_known_smi_bound_expr(e, &self.smi_bounds));
        let is_nn = init.is_some_and(|e| is_known_nonneg_i31_expr(e, &self.nonneg_i31));
        self.invalidate(name);
        if is_bound {
            self.smi_bounds.insert(name.to_owned());
        }
        if is_nn {
            self.nonneg_i31.insert(name.to_owned());
        }
    }
}

pub(in super::super) fn smi_ize_counted_loop_steps(stmts: &mut [Stmt]) {
    if stmts_have_mutating_nested_fn(stmts) {
        return;
    }
    let mut env = SmiEnv::default();
    smi_ize_in_block(stmts, &mut env);
}

fn is_known_smi_bound_expr(expr: &Expr, smi_bounds: &HashSet<String>) -> bool {
    if exact_u32(expr).is_some_and(|c| c <= 0x7fff_ffff)
        || super::super::post_opt::is_proven_nonneg_i31(expr)
    {
        return true;
    }
    match expr {
        Expr::Member(_, prop) => prop == "length" || prop == "len",
        Expr::Ident(id) => smi_bounds.contains(id),
        Expr::Binary(BinOp::Sub, l, r) => {
            is_known_smi_bound_expr(l, smi_bounds) && exact_u32(r).is_some()
        }
        Expr::Call(callee, args) if args.len() == 2 => {
            let is_min = matches!(
                callee.as_ref(),
                Expr::Member(obj, prop)
                    if matches!(obj.as_ref(), Expr::Ident(m) if m == "Math") && prop == "min"
            );
            is_min
                && (is_known_smi_bound_expr(&args[0], smi_bounds)
                    || is_known_smi_bound_expr(&args[1], smi_bounds))
        }
        _ => false,
    }
}

fn is_known_nonneg_i31_expr(expr: &Expr, nonneg_i31: &HashSet<String>) -> bool {
    if super::super::post_opt::is_proven_nonneg_i31(expr) {
        return true;
    }
    matches!(expr, Expr::Ident(id) if nonneg_i31.contains(id))
}

/// Extracts `(iv, max_step)` from a loop continuation condition proving `iv + max_step <= 0x7fff_ffff`.
fn extract_smi_loop_guard(
    cond: &Expr,
    assigned_in_loop: &HashSet<String>,
    smi_bounds: &HashSet<String>,
) -> Option<(String, u32)> {
    // Normalize `bound > lhs` / `bound >= lhs` into `lhs < bound` / `lhs <= bound`.
    let (lhs, rhs, strict) = match cond {
        Expr::Binary(BinOp::Lt, l, r) => (l.as_ref(), r.as_ref(), true),
        Expr::Binary(BinOp::Le, l, r) => (l.as_ref(), r.as_ref(), false),
        Expr::Binary(BinOp::Gt, l, r) => (r.as_ref(), l.as_ref(), true),
        Expr::Binary(BinOp::Ge, l, r) => (r.as_ref(), l.as_ref(), false),
        _ => return None,
    };
    if expr_reads_any_in(rhs, assigned_in_loop) {
        return None;
    }
    match lhs {
        Expr::Ident(iv) => {
            if expr_reads_ident(rhs, iv) {
                return None;
            }
            if strict {
                if is_known_smi_bound_expr(rhs, smi_bounds) {
                    return Some((iv.clone(), 1));
                }
            } else if exact_u32(rhs).is_some_and(|c| c <= 0x7fff_fffe) {
                return Some((iv.clone(), 1));
            }
            None
        }
        Expr::Binary(BinOp::Add, a, b) => {
            let (iv, k) = match (a.as_ref(), b.as_ref()) {
                (Expr::Ident(iv), other) => (iv, exact_u32(other)?),
                (other, Expr::Ident(iv)) => (iv, exact_u32(other)?),
                _ => return None,
            };
            if k == 0 || k > 0x7fff_ffff || expr_reads_ident(rhs, iv) {
                return None;
            }
            if is_known_smi_bound_expr(rhs, smi_bounds) {
                Some((iv.clone(), k))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn invert_comparison_for_exit(cond: &Expr) -> Option<Expr> {
    match cond {
        Expr::Unary(UnOp::Not, inner) => Some((**inner).clone()),
        Expr::Binary(op, l, r) => {
            let inv = match op {
                BinOp::Ge => BinOp::Lt,
                BinOp::Gt => BinOp::Le,
                BinOp::Le => BinOp::Gt,
                BinOp::Lt => BinOp::Ge,
                _ => return None,
            };
            Some(Expr::Binary(inv, l.clone(), r.clone()))
        }
        _ => None,
    }
}

/// Returns `Some((step, is_ushr_zero))` if `expr` is `(iv + step) >>> 0`, `(iv + step) | 0`, or
/// `iv + step` with `1 <= step <= max_step`.
fn match_iv_add_step(expr: &Expr, iv: &str, max_step: u32) -> Option<(u32, bool)> {
    let (inner, is_ushr_zero) = match expr {
        Expr::Binary(BinOp::UShr, l, r) if exact_u32(r) == Some(0) => (l.as_ref(), true),
        Expr::Binary(BinOp::BitOr, l, r) if exact_i32(r) == Some(0) => (l.as_ref(), false),
        other => (other, false),
    };
    let Expr::Binary(BinOp::Add, l, r) = inner else {
        return None;
    };
    let step = match (l.as_ref(), r.as_ref()) {
        (Expr::Ident(id), other) if id == iv => exact_u32(other)?,
        (other, Expr::Ident(id)) if id == iv => exact_u32(other)?,
        _ => return None,
    };
    (step >= 1 && step <= max_step).then_some((step, is_ushr_zero))
}

/// Checks that every write to `iv` in `stmts` is a monotonic `+ step` (`1 <= step <= max_step`)
/// and returns the maximum number of writes to `iv` along any single path through `stmts`.
/// Returns `None` if any non-step write to `iv` exists or if `iv` is written inside an inner loop.
fn max_step_writes_per_iter(stmts: &[Stmt], iv: &str, max_step: u32) -> Option<usize> {
    fn check(
        stmts: &[Stmt],
        iv: &str,
        max_step: u32,
        local_steps: &mut HashSet<String>,
    ) -> Option<usize> {
        let mut total = 0usize;
        for s in stmts {
            match s {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                    if expr_writes_ident(rhs, iv) {
                        return None;
                    }
                    if let Expr::Ident(id) = lhs.as_ref() {
                        if id == iv {
                            let ok = match_iv_add_step(rhs, iv, max_step).is_some()
                                || matches!(rhs.as_ref(), Expr::Ident(n) if local_steps.contains(n));
                            if !ok {
                                return None;
                            }
                            total += 1;
                            local_steps.clear();
                        } else if match_iv_add_step(rhs, iv, max_step).is_some() {
                            local_steps.insert(id.clone());
                        } else {
                            local_steps.remove(id);
                        }
                    } else if expr_writes_ident(lhs, iv) {
                        return None;
                    }
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                    if expr_writes_ident(e, iv) {
                        return None;
                    }
                    let mut w = HashSet::new();
                    collect_written_idents_expr(e, &mut w);
                    for k in w {
                        local_steps.remove(&k);
                    }
                }
                Stmt::If(c, a, b) => {
                    if expr_writes_ident(c, iv) {
                        return None;
                    }
                    let mut a_ls = local_steps.clone();
                    let a_n = check(a, iv, max_step, &mut a_ls)?;
                    let b_n = if let Some(b) = b {
                        let mut b_ls = local_steps.clone();
                        check(b, iv, max_step, &mut b_ls)?
                    } else {
                        0
                    };
                    total += a_n.max(b_n);
                    let mut w = HashSet::new();
                    collect_written_idents_stmts(a, &mut w, true);
                    if let Some(b) = b {
                        collect_written_idents_stmts(b, &mut w, true);
                    }
                    if w.contains(iv) {
                        local_steps.clear();
                    } else {
                        for k in w {
                            local_steps.remove(&k);
                        }
                    }
                }
                Stmt::Block(b) => total += check(b, iv, max_step, local_steps)?,
                Stmt::Labeled(_, inner) => {
                    total += check(
                        std::slice::from_ref(inner.as_ref()),
                        iv,
                        max_step,
                        local_steps,
                    )?;
                }
                Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
                    let mut w = HashSet::new();
                    collect_written_idents_stmts(b, &mut w, true);
                    if w.contains(iv) {
                        return None;
                    }
                    for k in w {
                        local_steps.remove(&k);
                    }
                }
                Stmt::Switch(d, cases) => {
                    if expr_writes_ident(d, iv) {
                        return None;
                    }
                    for c in cases {
                        let mut w = HashSet::new();
                        collect_written_idents_stmts(&c.body, &mut w, true);
                        if w.contains(iv) {
                            return None;
                        }
                    }
                    local_steps.clear();
                }
                Stmt::TryCatch { body, handler, .. } => {
                    let mut w = HashSet::new();
                    collect_written_idents_stmts(body, &mut w, false);
                    collect_written_idents_stmts(handler, &mut w, false);
                    if w.contains(iv) {
                        return None;
                    }
                    local_steps.clear();
                }
                _ => {}
            }
        }
        Some(total)
    }
    let mut local_steps = HashSet::new();
    check(stmts, iv, max_step, &mut local_steps)
}

/// Rewrites `(iv + step) >>> 0` to `(iv + step) | 0` on paths prior to the single update of `iv`.
fn rewrite_iv_steps_in_body(stmts: &mut [Stmt], iv: &str, max_step: u32, iv_updated: &mut bool) {
    for stmt in stmts.iter_mut() {
        if *iv_updated {
            return;
        }
        match stmt {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                if let Some((_, true)) = match_iv_add_step(rhs, iv, max_step) {
                    if let Expr::Binary(op, _, _) = rhs.as_mut() {
                        *op = BinOp::BitOr;
                    }
                }
                if matches!(lhs.as_ref(), Expr::Ident(id) if id == iv) {
                    *iv_updated = true;
                }
            }
            Stmt::If(_, a, b) => {
                let mut a_upd = *iv_updated;
                rewrite_iv_steps_in_body(a, iv, max_step, &mut a_upd);
                let mut b_upd = *iv_updated;
                if let Some(b) = b {
                    rewrite_iv_steps_in_body(b, iv, max_step, &mut b_upd);
                }
                if a_upd || b_upd {
                    *iv_updated = true;
                }
            }
            Stmt::Block(b) => rewrite_iv_steps_in_body(b, iv, max_step, iv_updated),
            Stmt::Labeled(_, inner) => {
                rewrite_iv_steps_in_body(
                    std::slice::from_mut(inner.as_mut()),
                    iv,
                    max_step,
                    iv_updated,
                );
            }
            _ => {}
        }
    }
}

fn try_smi_ize_loop(stmts: &mut [Stmt], idx: usize, env: &SmiEnv) {
    let Some(lp) = stmts[idx].as_loop() else {
        return;
    };
    let mut assigned_in_loop = HashSet::new();
    if let Some(c) = lp.cond {
        collect_written_idents_expr(c, &mut assigned_in_loop);
    }
    collect_written_idents_stmts(lp.body, &mut assigned_in_loop, true);
    let guard_expr: Option<Expr> = if let Some(c) = lp.cond {
        Some(c.clone())
    } else {
        match lp.body {
            [Stmt::If(c, _, Some(else_b))] if stmts_definitely_exit_loop(else_b, lp.label) => {
                Some(c.clone())
            }
            [Stmt::ExprStmt(Expr::Assign(fl, fr)), Stmt::If(Expr::Ident(fu), _, Some(else_b))]
                if matches!(fl.as_ref(), Expr::Ident(fn_) if fn_ == fu)
                    && stmts_definitely_exit_loop(else_b, lp.label) =>
            {
                Some(fr.as_ref().clone())
            }
            [Stmt::If(c, exit_b, None), rest @ ..]
                if !rest.is_empty() && stmts_definitely_exit_loop(exit_b, lp.label) =>
            {
                invert_comparison_for_exit(c)
            }
            _ => None,
        }
    };
    let Some(cond) = guard_expr else { return };
    let Some((iv, max_step)) = extract_smi_loop_guard(&cond, &assigned_in_loop, &env.smi_bounds)
    else {
        return;
    };
    let iv_starts_nonneg = env.nonneg_i31.contains(&iv)
        || find_prior_const_init(stmts, idx, &iv).is_some_and(|c| c <= 0x7fff_ffff);
    if !iv_starts_nonneg {
        return;
    }
    let Some(body_mut) = extract_iter0_then_body_mut(&mut stmts[idx]) else {
        return;
    };
    if max_step_writes_per_iter(body_mut, &iv, max_step) != Some(1) {
        return;
    }
    let mut updated = false;
    rewrite_iv_steps_in_body(body_mut, &iv, max_step, &mut updated);
}

fn smi_ize_in_block(stmts: &mut [Stmt], env: &mut SmiEnv) {
    for i in 0..stmts.len() {
        let pre_env = env.clone();
        if let Some(lp) = stmts[i].as_loop_mut() {
            let mut w = HashSet::new();
            if let Some(c) = lp.cond.as_deref() {
                collect_written_idents_expr(c, &mut w);
            }
            collect_written_idents_stmts(lp.body, &mut w, true);
            env.invalidate_all(&w);
            let mut inner_env = env.clone();
            smi_ize_in_block(lp.body, &mut inner_env);
        } else {
            match &mut stmts[i] {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                    let mut rhs_w = HashSet::new();
                    collect_written_idents_expr(rhs, &mut rhs_w);
                    env.invalidate_all(&rhs_w);
                    if let Expr::Ident(id) = lhs.as_ref() {
                        let id = id.clone();
                        env.record_binding(&id, Some(rhs));
                    } else {
                        let mut lhs_w = HashSet::new();
                        collect_written_idents_expr(lhs, &mut lhs_w);
                        env.invalidate_all(&lhs_w);
                    }
                }
                Stmt::Let(n, init) => {
                    let n = n.clone();
                    env.record_binding(&n, init.as_ref());
                }
                Stmt::Const(n, e) => {
                    let n = n.clone();
                    env.record_binding(&n, Some(e));
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        env.record_binding(n, init.as_ref());
                    }
                }
                Stmt::If(c, a, b) => {
                    let mut w = HashSet::new();
                    collect_written_idents_expr(c, &mut w);
                    env.invalidate_all(&w);
                    let mut a_env = env.clone();
                    smi_ize_in_block(a, &mut a_env);
                    collect_written_idents_stmts(a, &mut w, true);
                    if let Some(b) = b {
                        let mut b_env = env.clone();
                        smi_ize_in_block(b, &mut b_env);
                        collect_written_idents_stmts(b, &mut w, true);
                    }
                    env.invalidate_all(&w);
                }
                Stmt::Switch(d, cases) => {
                    let mut w = HashSet::new();
                    collect_written_idents_expr(d, &mut w);
                    env.invalidate_all(&w);
                    for c in cases {
                        let mut c_env = env.clone();
                        smi_ize_in_block(&mut c.body, &mut c_env);
                        collect_written_idents_stmts(&c.body, &mut w, true);
                    }
                    env.invalidate_all(&w);
                }
                Stmt::Block(b) => smi_ize_in_block(b, env),
                Stmt::Labeled(_, inner) => match inner.as_mut() {
                    Stmt::Block(b) => {
                        let mut w = HashSet::new();
                        collect_written_idents_stmts(b, &mut w, false);
                        let mut b_env = env.clone();
                        smi_ize_in_block(b, &mut b_env);
                        env.invalidate_all(&w);
                    }
                    other => {
                        let mut inner_env = env.clone();
                        smi_ize_in_block(std::slice::from_mut(other), &mut inner_env);
                        let mut w = HashSet::new();
                        collect_written_idents_stmts(std::slice::from_ref(other), &mut w, true);
                        env.invalidate_all(&w);
                    }
                },
                Stmt::TryCatch { body, handler, .. } => {
                    let mut b_env = env.clone();
                    smi_ize_in_block(body, &mut b_env);
                    let mut w = HashSet::new();
                    collect_written_idents_stmts(body, &mut w, false);
                    env.invalidate_all(&w);
                    let mut h_env = env.clone();
                    smi_ize_in_block(handler, &mut h_env);
                    collect_written_idents_stmts(handler, &mut w, false);
                    env.invalidate_all(&w);
                }
                other => {
                    let mut w = HashSet::new();
                    collect_written_idents_stmts(std::slice::from_ref(other), &mut w, true);
                    env.invalidate_all(&w);
                }
            }
        }
        if stmts[i].as_loop().is_some() {
            try_smi_ize_loop(stmts, i, &pre_env);
        }
    }
}
