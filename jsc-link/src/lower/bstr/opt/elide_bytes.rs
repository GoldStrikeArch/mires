//! Elides read-only `__rt.bstr_bytes(S)` allocations when only `.len` and `.buf[v.off + idx]`
//! are read.

use std::collections::{BTreeMap, BTreeSet};

use crate::jsast::{
    self, walk_expr, walk_expr_mut, walk_stmt, walk_stmt_mut, Expr, Stmt, Visit, VisitMut,
};

use super::{collect_fn_locals, unwrap_bstr_bytes_call};

#[derive(Default)]
struct Usage {
    bstr_writes: usize,
    safe_reads: usize,
    bad: bool,
}

fn is_var_member<'a>(expr: &'a Expr, prop: &str) -> Option<&'a str> {
    match expr {
        Expr::Member(obj, p) if p == prop => match obj.as_ref() {
            Expr::Ident(n) => Some(n.as_str()),
            _ => None,
        },
        _ => None,
    }
}

fn match_var_index(expr: &Expr) -> Option<(&str, Option<&Expr>)> {
    let Expr::Index(obj, idx) = expr else {
        return None;
    };
    let name = is_var_member(obj, "buf")?;
    if is_var_member(idx, "off") == Some(name) {
        return Some((name, None));
    }
    if let Expr::Binary(jsast::BinOp::Add, l, r) = idx.as_ref() {
        if is_var_member(l, "off") == Some(name) {
            return Some((name, Some(r.as_ref())));
        }
        if is_var_member(r, "off") == Some(name) {
            return Some((name, Some(l.as_ref())));
        }
    }
    None
}

struct BytesScan<'a> {
    locals: &'a BTreeSet<String>,
    in_closure: bool,
    usage: BTreeMap<String, Usage>,
}

impl BytesScan<'_> {
    fn mark_bad_expr(&mut self, expr: &Expr) {
        let prev = self.in_closure;
        self.in_closure = true;
        self.visit_expr(expr);
        self.in_closure = prev;
    }

    fn record_write(&mut self, n: &str, init: &Expr) {
        if let Some(arg) = unwrap_bstr_bytes_call(init) {
            self.usage.entry(n.to_owned()).or_default().bstr_writes += 1;
            self.visit_expr(arg);
        } else {
            self.usage.entry(n.to_owned()).or_default().bad = true;
            self.visit_expr(init);
        }
    }
}

impl Visit for BytesScan<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(n, Some(init)) | Stmt::Const(n, init)
                if !self.in_closure && self.locals.contains(n) =>
            {
                self.record_write(n, init);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(init) = init {
                        if !self.in_closure && self.locals.contains(n) {
                            self.record_write(n, init);
                        } else {
                            self.visit_expr(init);
                        }
                    }
                }
            }
            Stmt::TryCatch {
                body,
                binding,
                handler,
            } => {
                if self.locals.contains(binding) {
                    self.usage.entry(binding.clone()).or_default().bad = true;
                }
                self.visit_stmts(body);
                self.visit_stmts(handler);
            }
            Stmt::FunctionDecl { body, .. } => {
                let prev = self.in_closure;
                self.in_closure = true;
                self.visit_stmts(body);
                self.in_closure = prev;
            }
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        if !self.in_closure {
            if let Some((name, idx_opt)) = match_var_index(expr) {
                if self.locals.contains(name) {
                    self.usage.entry(name.to_owned()).or_default().safe_reads += 1;
                    if let Some(idx) = idx_opt {
                        self.visit_expr(idx);
                    }
                    return;
                }
            }
            if let Some(name) = is_var_member(expr, "len") {
                if self.locals.contains(name) {
                    self.usage.entry(name.to_owned()).or_default().safe_reads += 1;
                    return;
                }
            }
        }
        match expr {
            Expr::Ident(n) => {
                if self.locals.contains(n) {
                    self.usage.entry(n.clone()).or_default().bad = true;
                }
            }
            Expr::Assign(lhs, rhs) if !self.in_closure => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    if self.locals.contains(n) {
                        self.record_write(n, rhs);
                        return;
                    }
                }
                self.mark_bad_expr(lhs);
                self.visit_expr(rhs);
            }
            Expr::CompoundAssign(_, lhs, rhs) if !self.in_closure => {
                self.mark_bad_expr(lhs);
                self.visit_expr(rhs);
            }
            Expr::Arrow(_, _) => {
                let prev = self.in_closure;
                self.in_closure = true;
                walk_expr(self, expr);
                self.in_closure = prev;
            }
            _ => walk_expr(self, expr),
        }
    }
}

struct BytesRewriter<'a>(&'a BTreeSet<String>);

impl BytesRewriter<'_> {
    fn unwrap_init(&mut self, init: &mut Expr) {
        if let Some(mut arg) = unwrap_bstr_bytes_call(init).cloned() {
            self.visit_expr_mut(&mut arg);
            *init = arg;
        } else {
            self.visit_expr_mut(init);
        }
    }
}

impl VisitMut for BytesRewriter<'_> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Let(n, Some(init)) | Stmt::Const(n, init) if self.0.contains(n) => {
                self.unwrap_init(init);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(init) = init {
                        if self.0.contains(n) {
                            self.unwrap_init(init);
                        } else {
                            self.visit_expr_mut(init);
                        }
                    }
                }
            }
            Stmt::FunctionDecl { .. } => {}
            _ => walk_stmt_mut(self, stmt),
        }
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        if let Some((name, idx_opt)) = match_var_index(expr) {
            if self.0.contains(name) {
                let name = name.to_owned();
                let mut idx_expr = idx_opt.cloned().unwrap_or_else(|| jsast::num(0));
                self.visit_expr_mut(&mut idx_expr);
                *expr = jsast::method_call(jsast::id(name), "charCodeAt", vec![idx_expr]);
                return;
            }
        }
        if let Some(name) = is_var_member(expr, "len") {
            if self.0.contains(name) {
                *expr = jsast::member(jsast::id(name.to_owned()), "length");
                return;
            }
        }
        match expr {
            Expr::Assign(lhs, rhs) => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    if self.0.contains(n) {
                        if let Some(mut arg) = unwrap_bstr_bytes_call(rhs).cloned() {
                            self.visit_expr_mut(&mut arg);
                            **rhs = arg;
                            return;
                        }
                    }
                }
                self.visit_expr_mut(lhs);
                self.visit_expr_mut(rhs);
            }
            Expr::Arrow(_, _) => {}
            _ => walk_expr_mut(self, expr),
        }
    }
}

/// Elides `__rt.bstr_bytes(S)` for local variables whose every assignment is `__rt.bstr_bytes(...)`
/// and whose every read is strictly `.len`, `.buf[v.off]`, or `.buf[v.off + idx]`: rewrites the
/// assignment to `v = S` and reads to `v.length` / `v.charCodeAt(0)` / `v.charCodeAt(idx)`.
pub(in crate::lower::bstr) fn elide_readonly_bstr_bytes(body: &mut [Stmt]) {
    let mut locals = BTreeSet::new();
    collect_fn_locals(body, &mut locals);
    if locals.is_empty() {
        return;
    }
    let mut scan = BytesScan {
        locals: &locals,
        in_closure: false,
        usage: BTreeMap::new(),
    };
    scan.visit_stmts(body);
    let elided: BTreeSet<String> = scan
        .usage
        .into_iter()
        .filter(|(_, u)| !u.bad && u.bstr_writes >= 1 && u.safe_reads >= 1)
        .map(|(n, _)| n)
        .collect();
    if !elided.is_empty() {
        BytesRewriter(&elided).visit_stmts_mut(body);
    }
}
