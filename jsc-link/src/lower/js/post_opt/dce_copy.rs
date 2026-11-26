//! Single-assign literal propagation, straight-line constant propagation, dominating `charCodeAt`
//! CSE, and spec-identical `Reflect.get(o, k)` fast-path + dominated object-guard CSE.

use std::collections::{HashMap, HashSet};

use rcgjs_object::jsast::{self, ArrowBody, BinOp, Expr, Stmt, UnOp, Visit, VisitMut};

use super::super::ast_util::{
    collect_written_idents_expr, collect_written_idents_stmt, collect_written_idents_stmts,
    count_ident_reads_expr, count_ident_reads_stmts, expr_has_side_effect,
    stmts_definitely_diverge, subst_ident_stmts, visit_local_blocks_mut,
};
use super::alloc_fresh_local;
use super::fold::{fold_expr, fold_lvalue_expr, fold_stmt_list, is_pure_index_expr, SimpleLit};

// =============================================================================================
// Single-assign literal propagation & straight-line constant propagation
// =============================================================================================

#[derive(Default)]
pub(super) struct LocalConstUsage {
    pub(super) writes: usize,
    pub(super) reads: usize,
    pub(super) in_closure: bool,
}

pub(super) fn scan_const_usage_stmts(
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

fn is_lit_worth_inlining(lit: &SimpleLit, total_reads: usize) -> bool {
    match lit {
        // Only duplicate strings when single-use or very short (avoids bundle bloat).
        SimpleLit::Str(s) => total_reads <= 2 || s.len() <= 8,
        _ => total_reads <= 6,
    }
}

fn prop_local_lits_in_single_block(
    stmts: &mut Vec<Stmt>,
    single_write_reads: &HashMap<String, usize>,
) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i < stmts.len() {
        // Handle both standalone `x = lit;` and comma-sequence `a = lit1, b = lit2;`
        if let Stmt::ExprStmt(Expr::Assign(lhs, rhs)) = &stmts[i] {
            if let (Expr::Ident(name), Some(lit)) = (lhs.as_ref(), SimpleLit::from_expr(rhs)) {
                if let Some(&total_reads) = single_write_reads.get(name) {
                    if is_lit_worth_inlining(&lit, total_reads)
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
            // Check if any qualifying literal assignment in `Seq` has all its reads in
            // `stmts[i + 1..]` (and none in earlier/later elements of the same `Seq`).
            let mut extracted: Vec<(usize, String, SimpleLit)> = Vec::new();
            for (k, item) in items.iter().enumerate() {
                if let Expr::Assign(lhs, rhs) = item {
                    if let (Expr::Ident(name), Some(lit)) =
                        (lhs.as_ref(), SimpleLit::from_expr(rhs))
                    {
                        if let Some(&total_reads) = single_write_reads.get(name) {
                            let seq_reads: usize = items
                                .iter()
                                .map(|it| count_ident_reads_expr(it, name))
                                .sum();
                            if is_lit_worth_inlining(&lit, total_reads)
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

/// Propagates single-assignment uninit local literals whose definition strictly dominates every
/// read (all reads lie in the trailing sibling statements `stmts[i + 1..]`).
fn prop_local_lits_in_block(
    stmts: &mut Vec<Stmt>,
    single_write_reads: &HashMap<String, usize>,
) -> bool {
    let mut changed = false;
    visit_local_blocks_mut(stmts, false, |block| {
        changed |= prop_local_lits_in_single_block(block, single_write_reads);
    });
    changed
}

pub(super) fn fold_constants_and_switches(
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

fn subst_straight_line_expr(expr: &mut Expr, env: &mut HashMap<String, SimpleLit>) {
    match expr {
        Expr::Ident(id) => {
            if let Some(lit) = env.get(id) {
                *expr = lit.to_expr();
            }
        }
        Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
            subst_straight_line_expr(rhs, env);
            fold_expr(rhs);
            if let Expr::Ident(id) = lhs.as_ref() {
                env.remove(id);
            } else {
                subst_straight_line_expr(lhs, env);
                fold_lvalue_expr(lhs);
            }
        }
        Expr::Unary(_, e) | Expr::Member(e, _) => subst_straight_line_expr(e, env),
        Expr::Binary(BinOp::And | BinOp::Or, l, r) => {
            subst_straight_line_expr(l, env);
            let mut r_env = env.clone();
            subst_straight_line_expr(r, &mut r_env);
            env.retain(|k, v| r_env.get(k) == Some(v));
        }
        Expr::Binary(_, l, r) | Expr::Index(l, r) => {
            subst_straight_line_expr(l, env);
            subst_straight_line_expr(r, env);
        }
        Expr::Cond(c, t, e) => {
            subst_straight_line_expr(c, env);
            let mut t_env = env.clone();
            subst_straight_line_expr(t, &mut t_env);
            let mut e_env = env.clone();
            subst_straight_line_expr(e, &mut e_env);
            env.retain(|k, v| t_env.get(k) == Some(v) && e_env.get(k) == Some(v));
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            subst_straight_line_expr(c, env);
            for a in args {
                subst_straight_line_expr(a, env);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for x in items {
                subst_straight_line_expr(x, env);
            }
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                subst_straight_line_expr(v, env);
            }
        }
        Expr::Arrow(..) => env.clear(),
        _ => {}
    }
    fold_expr(expr);
}

fn run_straight_line_block(
    stmts: &mut [Stmt],
    local_names: &HashSet<String>,
    env: &mut HashMap<String, SimpleLit>,
) {
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                subst_straight_line_expr(rhs, env);
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
                    subst_straight_line_expr(lhs, env);
                }
            }
            Stmt::ExprStmt(e) => subst_straight_line_expr(e, env),
            Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                subst_straight_line_expr(e, env);
                env.clear();
            }
            Stmt::Let(n, init) => {
                if let Some(e) = init {
                    subst_straight_line_expr(e, env);
                }
                env.remove(n);
            }
            Stmt::Const(n, e) => {
                subst_straight_line_expr(e, env);
                env.remove(n);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(e) = init {
                        subst_straight_line_expr(e, env);
                    }
                    env.remove(n);
                }
            }
            Stmt::If(c, a, b) => {
                subst_straight_line_expr(c, env);
                let mut a_env = env.clone();
                run_straight_line_block(a, local_names, &mut a_env);
                if let Some(b) = b {
                    let mut b_env = env.clone();
                    run_straight_line_block(b, local_names, &mut b_env);
                    env.retain(|k, v| a_env.get(k) == Some(v) && b_env.get(k) == Some(v));
                } else if !stmts_definitely_diverge(a) {
                    env.retain(|k, v| a_env.get(k) == Some(v));
                }
            }
            Stmt::Block(b) => run_straight_line_block(b, local_names, env),
            Stmt::Labeled(_, inner) => {
                // A labeled block/loop can exit early via `break label;` with values written
                // prior to the break, so assignments after a conditional break inside `inner`
                // must NEVER leak past the labeled statement.
                let mut assigned = HashSet::new();
                collect_written_idents_stmt(inner, &mut assigned);
                let mut inner_env = env.clone();
                run_straight_line_block(
                    std::slice::from_mut(inner.as_mut()),
                    local_names,
                    &mut inner_env,
                );
                for a in assigned {
                    env.remove(&a);
                }
            }
            Stmt::Break(_) | Stmt::Continue(_) | Stmt::Return(None) => env.clear(),
            Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
                env.clear();
                let mut b_env = HashMap::new();
                run_straight_line_block(b, local_names, &mut b_env);
            }
            Stmt::Switch(d, cases) => {
                subst_straight_line_expr(d, env);
                for c in cases {
                    let mut c_env = env.clone();
                    run_straight_line_block(&mut c.body, local_names, &mut c_env);
                }
                env.clear();
            }
            Stmt::TryCatch { body, handler, .. } => {
                let mut b_env = env.clone();
                run_straight_line_block(body, local_names, &mut b_env);
                env.clear();
                let mut h_env = HashMap::new();
                run_straight_line_block(handler, local_names, &mut h_env);
            }
            _ => {}
        }
    }
}

/// Forward straight-line constant propagation for unrolled loop bodies (where temp locals like
/// `s`, `H`, `t`, `o`, `z` are assigned multiple times across unrolled iterations).
pub(super) fn prop_straight_line_lits(body: &mut [Stmt], local_names: &HashSet<String>) {
    let mut env = HashMap::new();
    run_straight_line_block(body, local_names, &mut env);
}

// =============================================================================================
// Straight-line CSE for dominating `s.charCodeAt(idx)` loads
// =============================================================================================

/// Straight-line CSE for repeated pure `s.charCodeAt(idx)` loads dominated by an unconditional
/// first evaluation (e.g. `65 <= a.charCodeAt(A) && a.charCodeAt(A) <= 90` followed by
/// `97 <= a.charCodeAt(A)` in `marked`).
pub(super) fn cse_dominating_char_code_at(
    stmts: &mut Vec<Stmt>,
    local_names: &mut HashSet<String>,
    extra_decls: &mut Vec<(String, Option<Expr>)>,
) {
    struct ChildBlockDescender<'a> {
        local_names: &'a mut HashSet<String>,
        extra_decls: &'a mut Vec<(String, Option<Expr>)>,
    }
    impl VisitMut for ChildBlockDescender<'_> {
        fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
            if matches!(stmt, Stmt::FunctionDecl { .. }) {
                return;
            }
            jsast::walk_stmt_mut(self, stmt);
        }
        fn visit_expr_mut(&mut self, _expr: &mut Expr) {}
        fn visit_block_mut(&mut self, block: &mut Vec<Stmt>) {
            cse_dominating_char_code_at(block, self.local_names, self.extra_decls);
        }
    }

    let mut i = 0usize;
    while i < stmts.len() {
        // First recurse into nested blocks of `stmts[i]`.
        let mut child_walker = ChildBlockDescender {
            local_names: &mut *local_names,
            extra_decls: &mut *extra_decls,
        };
        jsast::walk_stmt_mut(&mut child_walker, &mut stmts[i]);

        i += try_cse_char_code_at_index(stmts, i, local_names, extra_decls);
    }
}

fn try_cse_char_code_at_index(
    stmts: &mut Vec<Stmt>,
    i: usize,
    local_names: &mut HashSet<String>,
    extra_decls: &mut Vec<(String, Option<Expr>)>,
) -> usize {
    // Look for an unconditional `s.charCodeAt(idx)` evaluated at the start of `stmts[i]`.
    let Some((recv, idx_expr)) = first_unconditional_char_code_at_stmt(&stmts[i]) else {
        return 1;
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
    while end < stmts.len() {
        let mut written = HashSet::new();
        collect_written_idents_stmt(&stmts[end], &mut written);
        if written.iter().any(|w| deps.contains(w)) {
            // Still allow CSE within `stmts[i]`'s own `if` condition if `cond` doesn't write deps.
            if end == i {
                if let Stmt::If(cond, _, _) = &stmts[i] {
                    let mut cond_w = HashSet::new();
                    collect_written_idents_expr(cond, &mut cond_w);
                    if !cond_w.iter().any(|w| deps.contains(w))
                        && count_expr_eq_in_expr(cond, &target) >= 2
                    {
                        let tmp = alloc_fresh_local(local_names, extra_decls, "$cc");
                        if let Stmt::If(cond_mut, _, _) = &mut stmts[i] {
                            replace_expr_eq_in_expr(cond_mut, &target, &Expr::Ident(tmp.clone()));
                        }
                        stmts.insert(i, jsast::assign_stmt(jsast::id(tmp), target));
                        return 2;
                    }
                }
            }
            break;
        }
        total += count_expr_eq_in_stmt(&stmts[end], &target);
        end += 1;
    }

    if total < 2 {
        return 1;
    }

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
                (!written_later).then(|| v.clone())
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
        1
    } else {
        let tmp = alloc_fresh_local(local_names, extra_decls, "$cc");
        let repl = Expr::Ident(tmp.clone());
        for s in &mut stmts[i..end] {
            replace_expr_eq_in_stmt(s, &target, &repl);
        }
        stmts.insert(i, jsast::assign_stmt(jsast::id(tmp), target));
        2
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
            if !expr_has_side_effect(l) {
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
    impl Visit for V<'_> {
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
    impl Visit for V<'_> {
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
    impl VisitMut for V<'_> {
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
    impl VisitMut for V<'_> {
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
// Spec-identical `Reflect.get(o, k)` fast-path + dominated object-guard CSE
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
            collect_written_idents_expr(t, &mut assigned);
            collect_written_idents_expr(e, &mut assigned);
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
                collect_written_idents_stmts(then_b, &mut assigned);
                if let Some(eb) = else_b {
                    let mut e_known = known.clone();
                    rewrite_reflect_stmts(eb, &mut e_known);
                    collect_written_idents_stmts(eb, &mut assigned);
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
                    collect_written_idents_stmts(&c.body, &mut assigned);
                }
                for a in assigned {
                    known.remove(&a);
                }
            }
            Stmt::Labeled(_, inner) => {
                let mut assigned = HashSet::new();
                collect_written_idents_stmt(inner, &mut assigned);
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
                collect_written_idents_stmts(b, &mut assigned);
                for a in &assigned {
                    known.remove(a);
                }
                let mut lk = known.clone();
                rewrite_reflect_stmts(b, &mut lk);
            }
            Stmt::While(t, b) => {
                let mut assigned = HashSet::new();
                collect_written_idents_expr(t, &mut assigned);
                collect_written_idents_stmts(b, &mut assigned);
                for a in &assigned {
                    known.remove(a);
                }
                let mut lk = known.clone();
                rewrite_reflect_expr(t, &mut lk, true);
                rewrite_reflect_stmts(b, &mut lk);
            }
            Stmt::TryCatch { body, handler, .. } => {
                let mut assigned = HashSet::new();
                collect_written_idents_stmts(body, &mut assigned);
                collect_written_idents_stmts(handler, &mut assigned);
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

pub(super) fn optimize_reflect_get(body: &mut [Stmt]) {
    let mut known = HashSet::new();
    rewrite_reflect_stmts(body, &mut known);
}
