//! Shared JavaScript AST traversal and query utilities built on [`jsast::Visit`] / [`jsast::VisitMut`].

use std::collections::{BTreeSet, HashMap, HashSet};

use rcgjs_object::jsast::{
    self, walk_expr, walk_expr_mut, walk_stmt, walk_stmt_mut, ArrowBody, Expr, Stmt, Visit,
    VisitMut,
};

/// JavaScript TypedArray constructors indexed by `TypedArrayKind as usize`.
pub const TA_CTORS: [&str; 8] = [
    "Uint8Array",
    "Int8Array",
    "Uint16Array",
    "Int16Array",
    "Uint32Array",
    "Int32Array",
    "Float32Array",
    "Float64Array",
];

/// Collects every identifier written (assigned, compound-assigned, declared, or catch-bound)
/// inside `expr`.
pub fn collect_written_idents_expr(expr: &Expr, out: &mut HashSet<String>) {
    WrittenCollector {
        out,
        ignore_diverging_branches: false,
    }
    .visit_expr(expr);
}

/// Collects every identifier written inside `stmt`.
pub fn collect_written_idents_stmt(stmt: &Stmt, out: &mut HashSet<String>) {
    WrittenCollector {
        out,
        ignore_diverging_branches: false,
    }
    .visit_stmt(stmt);
}

/// Collects every identifier written inside `stmts`.
pub fn collect_written_idents_stmts(stmts: &[Stmt], out: &mut HashSet<String>) {
    WrittenCollector {
        out,
        ignore_diverging_branches: false,
    }
    .visit_stmts(stmts);
}

/// Like [`collect_written_idents_stmts`], optionally ignoring `if` branches that always exit.
pub fn collect_written_idents_stmts_opt(
    stmts: &[Stmt],
    out: &mut HashSet<String>,
    ignore_diverging_branches: bool,
) {
    WrittenCollector {
        out,
        ignore_diverging_branches,
    }
    .visit_stmts(stmts);
}

struct WrittenCollector<'a> {
    out: &'a mut HashSet<String>,
    ignore_diverging_branches: bool,
}

impl Visit for WrittenCollector<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(n, init) => {
                self.out.insert(n.clone());
                if let Some(e) = init {
                    self.visit_expr(e);
                }
            }
            Stmt::Const(n, e) => {
                self.out.insert(n.clone());
                self.visit_expr(e);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    self.out.insert(n.clone());
                    if let Some(e) = init {
                        self.visit_expr(e);
                    }
                }
            }
            Stmt::If(c, t, e) if self.ignore_diverging_branches => {
                self.visit_expr(c);
                if !super::post_opt::stmts_definitely_diverge(t) {
                    self.visit_stmts(t);
                }
                if let Some(el) = e {
                    if !super::post_opt::stmts_definitely_diverge(el) {
                        self.visit_stmts(el);
                    }
                }
            }
            Stmt::TryCatch {
                body,
                binding,
                handler,
            } => {
                self.out.insert(binding.clone());
                let prev = self.ignore_diverging_branches;
                self.ignore_diverging_branches = false;
                self.visit_stmts(body);
                self.visit_stmts(handler);
                self.ignore_diverging_branches = prev;
            }
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        if let Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) = expr {
            if let Expr::Ident(id) = lhs.as_ref() {
                self.out.insert(id.clone());
            } else {
                self.visit_expr(lhs);
            }
            self.visit_expr(rhs);
            return;
        }
        walk_expr(self, expr);
    }
}

/// Counts reads of `target` in `expr` (excluding plain assignment LHS `target = ...`).
pub fn count_ident_reads_expr(expr: &Expr, target: &str) -> usize {
    let mut v = ReadCounter { target, count: 0 };
    v.visit_expr(expr);
    v.count
}

/// Counts reads of `target` in `stmts` (excluding plain assignment LHS `target = ...`).
pub fn count_ident_reads_stmts(stmts: &[Stmt], target: &str) -> usize {
    let mut v = ReadCounter { target, count: 0 };
    v.visit_stmts(stmts);
    v.count
}

struct ReadCounter<'a> {
    target: &'a str,
    count: usize,
}

impl Visit for ReadCounter<'_> {
    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Ident(n) => {
                if n == self.target {
                    self.count += 1;
                }
            }
            Expr::Assign(lhs, rhs) => {
                if !matches!(lhs.as_ref(), Expr::Ident(n) if n == self.target) {
                    self.visit_expr(lhs);
                }
                self.visit_expr(rhs);
            }
            _ => walk_expr(self, expr),
        }
    }
}

/// Tallies read occurrences of every identifier in `stmt` (excluding plain assignment LHS).
pub fn tally_ident_reads_stmt(stmt: &Stmt, counts: &mut HashMap<String, usize>) {
    ReadTallier(counts).visit_stmt(stmt);
}

struct ReadTallier<'a>(&'a mut HashMap<String, usize>);

impl Visit for ReadTallier<'_> {
    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Ident(id) => *self.0.entry(id.clone()).or_insert(0) += 1,
            Expr::Assign(lhs, rhs) => {
                if !matches!(lhs.as_ref(), Expr::Ident(_)) {
                    self.visit_expr(lhs);
                }
                self.visit_expr(rhs);
            }
            _ => walk_expr(self, expr),
        }
    }
}

/// Replaces read occurrences of `target` with `repl` inside `stmts`, skipping plain assignment
/// LHS `target = ...` and nested function/arrow bodies.
pub fn subst_ident_stmts(stmts: &mut [Stmt], target: &str, repl: &Expr) {
    IdentSubst { target, repl }.visit_stmts_mut(stmts);
}

struct IdentSubst<'a> {
    target: &'a str,
    repl: &'a Expr,
}

impl VisitMut for IdentSubst<'_> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        if matches!(stmt, Stmt::FunctionDecl { .. }) {
            return;
        }
        walk_stmt_mut(self, stmt);
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        if matches!(expr, Expr::Ident(n) if n == self.target) {
            *expr = self.repl.clone();
            return;
        }
        match expr {
            Expr::Arrow(..) => {}
            Expr::Assign(lhs, rhs) => {
                if !matches!(lhs.as_ref(), Expr::Ident(n) if n == self.target) {
                    self.visit_expr_mut(lhs);
                }
                self.visit_expr_mut(rhs);
            }
            _ => walk_expr_mut(self, expr),
        }
    }
}

/// Renames every `Expr::Ident(from)` to `to` across `stmts` (including inside closures).
pub fn rename_ident_stmts(stmts: &mut [Stmt], from: &str, to: &str) {
    jsast::visit_each_expr_mut(stmts, |e| {
        if let Expr::Ident(name) = e {
            if name == from {
                *name = to.to_owned();
            }
        }
    });
}

/// Collects top-level `let`/`lets`/`const` declarations in `stmts` (recording uninitialized `let`s
/// in `uninit_locals`).
pub fn collect_top_locals(
    stmts: &[Stmt],
    all_locals: &mut HashSet<String>,
    uninit_locals: &mut HashSet<String>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::Let(n, init) => {
                all_locals.insert(n.clone());
                if init.is_none() {
                    uninit_locals.insert(n.clone());
                }
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    all_locals.insert(n.clone());
                    if init.is_none() {
                        uninit_locals.insert(n.clone());
                    }
                }
            }
            Stmt::Const(n, _) => {
                all_locals.insert(n.clone());
            }
            _ => break,
        }
    }
}

/// Returns `true` if `target` is referenced inside any nested `Expr::Arrow` or
/// `Stmt::FunctionDecl` within `stmts`.
pub fn has_closure_use(stmts: &[Stmt], target: &str) -> bool {
    struct V<'a> {
        target: &'a str,
        in_fn: bool,
        found: bool,
    }
    impl Visit for V<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            if self.found {
                return;
            }
            if let Stmt::FunctionDecl { body, .. } = stmt {
                let prev = self.in_fn;
                self.in_fn = true;
                self.visit_stmts(body);
                self.in_fn = prev;
                return;
            }
            walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &Expr) {
            if self.found {
                return;
            }
            match expr {
                Expr::Ident(n) if self.in_fn && n == self.target => self.found = true,
                Expr::Arrow(_, body) => {
                    let prev = self.in_fn;
                    self.in_fn = true;
                    match body {
                        ArrowBody::Expr(e) => self.visit_expr(e),
                        ArrowBody::Block(s) => self.visit_stmts(s),
                    }
                    self.in_fn = prev;
                }
                _ => walk_expr(self, expr),
            }
        }
    }
    let mut v = V {
        target,
        in_fn: false,
        found: false,
    };
    v.visit_stmts(stmts);
    v.found
}

/// Collects every local in `locals` referenced inside any nested `Expr::Arrow` or
/// `Stmt::FunctionDecl` within `stmts`.
pub fn collect_closure_captured_btree(
    stmts: &[Stmt],
    locals: &BTreeSet<String>,
    out: &mut BTreeSet<String>,
) {
    struct Cap<'a> {
        locals: &'a BTreeSet<String>,
        out: &'a mut BTreeSet<String>,
        in_closure: bool,
    }
    impl Visit for Cap<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            if let Stmt::FunctionDecl { body, .. } = stmt {
                let prev = self.in_closure;
                self.in_closure = true;
                self.visit_stmts(body);
                self.in_closure = prev;
                return;
            }
            walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &Expr) {
            match expr {
                Expr::Ident(n) if self.in_closure && self.locals.contains(n) => {
                    self.out.insert(n.clone());
                }
                Expr::Arrow(_, body) => {
                    let prev = self.in_closure;
                    self.in_closure = true;
                    match body {
                        ArrowBody::Expr(e) => self.visit_expr(e),
                        ArrowBody::Block(s) => self.visit_stmts(s),
                    }
                    self.in_closure = prev;
                }
                _ => walk_expr(self, expr),
            }
        }
    }
    Cap {
        locals,
        out,
        in_closure: false,
    }
    .visit_stmts(stmts);
}
