//! Shared AST traversal and loop-analysis helpers used across `loop_opt` submodules.

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use rcgjs_object::jsast::{self, ArrowBody, Expr, Stmt, TryVisit, Visit};

use super::super::ast_util::{
    collect_written_idents_expr, count_ident_reads_expr, count_ident_reads_stmts,
    expr_has_side_effect, is_panic_callee, stmts_definitely_diverge, subst_ident_stmts,
};

pub(super) fn count_stmts(stmts: &[Stmt]) -> usize {
    struct Counter(usize);
    impl Visit for Counter {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            self.0 += 1;
            if !matches!(stmt, Stmt::FunctionDecl { .. }) {
                jsast::walk_stmt(self, stmt);
            }
        }
        fn visit_expr(&mut self, _: &Expr) {}
    }
    let mut counter = Counter(0);
    counter.visit_stmts(stmts);
    counter.0
}

/// Returns `true` if `stmts` contains a nested arrow/function that may mutate outer state or
/// execute arbitrary side effects. Pure expression-bodied arrows (like `m => ({ hi: m.hi, lo: m.lo })`
/// emitted for array clones in `compress512`) are read-only and do not block outer loop passes.
pub(super) fn stmts_have_mutating_nested_fn(stmts: &[Stmt]) -> bool {
    struct Finder;
    impl TryVisit for Finder {
        type Break = ();
        fn visit_stmt(&mut self, stmt: &Stmt) -> ControlFlow<()> {
            match stmt {
                Stmt::FunctionDecl { .. } => ControlFlow::Break(()),
                _ => jsast::try_walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            match expr {
                Expr::Arrow(_, ArrowBody::Block(_)) => ControlFlow::Break(()),
                Expr::Arrow(_, ArrowBody::Expr(body)) => {
                    if expr_has_side_effect(body) {
                        ControlFlow::Break(())
                    } else {
                        self.visit_expr(body)
                    }
                }
                _ => jsast::try_walk_expr(self, expr),
            }
        }
    }
    Finder.visit_stmts(stmts).is_break()
}

struct NonPanicCallFinder;
impl TryVisit for NonPanicCallFinder {
    type Break = ();
    fn visit_stmt(&mut self, stmt: &Stmt) -> ControlFlow<()> {
        match stmt {
            Stmt::Throw(_) | Stmt::FunctionDecl { .. } => ControlFlow::Continue(()),
            _ => jsast::try_walk_stmt(self, stmt),
        }
    }
    fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        match expr {
            Expr::New(..) | Expr::Raw(_) => ControlFlow::Break(()),
            Expr::Arrow(..) => ControlFlow::Continue(()),
            Expr::Call(callee, args) => {
                if is_panic_callee(callee) {
                    return ControlFlow::Continue(());
                }
                let is_pure_math = matches!(
                    callee.as_ref(),
                    Expr::Member(obj, prop)
                        if matches!(obj.as_ref(), Expr::Ident(m) if m == "Math")
                            && matches!(prop.as_str(), "imul" | "clz32" | "abs" | "min" | "max")
                );
                if !is_pure_math {
                    return ControlFlow::Break(());
                }
                for a in args {
                    self.visit_expr(a)?;
                }
                ControlFlow::Continue(())
            }
            _ => jsast::try_walk_expr(self, expr),
        }
    }
}

pub(super) fn expr_has_non_panic_call(expr: &Expr) -> bool {
    NonPanicCallFinder.visit_expr(expr).is_break()
}

pub(super) fn stmts_have_non_panic_call(stmts: &[Stmt]) -> bool {
    NonPanicCallFinder.visit_stmts(stmts).is_break()
}

pub(super) fn stmts_have_control_flow(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::Return(_)
        | Stmt::Throw(_)
        | Stmt::LoopForever(..)
        | Stmt::While(..)
        | Stmt::Switch(..)
        | Stmt::TryCatch { .. } => true,
        Stmt::If(_, a, b) => {
            (!stmts_definitely_diverge(a) && stmts_have_control_flow(a))
                || b.as_ref()
                    .is_some_and(|b| !stmts_definitely_diverge(b) && stmts_have_control_flow(b))
        }
        Stmt::Block(b) => stmts_have_control_flow(b),
        Stmt::Labeled(_, inner) => stmts_have_control_flow(std::slice::from_ref(inner.as_ref())),
        _ => false,
    })
}

pub(super) fn stmts_have_var_decl(stmts: &[Stmt]) -> bool {
    struct Finder;
    impl TryVisit for Finder {
        type Break = ();
        fn visit_stmt(&mut self, stmt: &Stmt) -> ControlFlow<()> {
            match stmt {
                Stmt::Let(..) | Stmt::Const(..) | Stmt::Lets(..) | Stmt::FunctionDecl { .. } => {
                    ControlFlow::Break(())
                }
                _ => jsast::try_walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, _: &Expr) -> ControlFlow<()> {
            ControlFlow::Continue(())
        }
    }
    Finder.visit_stmts(stmts).is_break()
}

pub(super) fn expr_writes_ident(expr: &Expr, target: &str) -> bool {
    let mut w = HashSet::new();
    collect_written_idents_expr(expr, &mut w);
    w.contains(target)
}

pub(super) fn expr_reads_ident(expr: &Expr, target: &str) -> bool {
    count_ident_reads_expr(expr, target) > 0
}

pub(super) fn stmts_read_ident(stmts: &[Stmt], target: &str) -> bool {
    count_ident_reads_stmts(stmts, target) > 0
}

pub(super) fn expr_reads_any_in(expr: &Expr, set: &HashSet<String>) -> bool {
    struct Finder<'a>(&'a HashSet<String>);
    impl TryVisit for Finder<'_> {
        type Break = ();
        fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            match expr {
                Expr::Ident(id) if self.0.contains(id) => ControlFlow::Break(()),
                Expr::Arrow(..) => ControlFlow::Continue(()),
                _ => jsast::try_walk_expr(self, expr),
            }
        }
    }
    Finder(set).visit_expr(expr).is_break()
}

pub(super) fn subst_ident_in_stmt(stmt: &mut Stmt, from: &str, to: &Expr) {
    subst_ident_stmts(std::slice::from_mut(stmt), from, to);
}

pub(super) fn count_rw_stmts(
    stmts: &[Stmt],
    reads: &mut HashMap<String, usize>,
    writes: &mut HashMap<String, usize>,
) {
    struct RwCounter<'a> {
        reads: &'a mut HashMap<String, usize>,
        writes: &'a mut HashMap<String, usize>,
    }
    impl Visit for RwCounter<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::Let(n, Some(e)) | Stmt::Const(n, e) => {
                    *self.writes.entry(n.clone()).or_default() += 1;
                    self.visit_expr(e);
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(e) = init {
                            *self.writes.entry(n.clone()).or_default() += 1;
                            self.visit_expr(e);
                        }
                    }
                }
                _ => jsast::walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, expr: &Expr) {
            match expr {
                Expr::Ident(id) => *self.reads.entry(id.clone()).or_default() += 1,
                Expr::Assign(lhs, rhs) => {
                    if let Expr::Ident(id) = lhs.as_ref() {
                        *self.writes.entry(id.clone()).or_default() += 1;
                    } else {
                        self.visit_expr(lhs);
                    }
                    self.visit_expr(rhs);
                }
                Expr::CompoundAssign(_, lhs, rhs) => {
                    if let Expr::Ident(id) = lhs.as_ref() {
                        *self.reads.entry(id.clone()).or_default() += 1;
                        *self.writes.entry(id.clone()).or_default() += 1;
                    } else {
                        self.visit_expr(lhs);
                    }
                    self.visit_expr(rhs);
                }
                _ => jsast::walk_expr(self, expr),
            }
        }
    }
    RwCounter { reads, writes }.visit_stmts(stmts);
}

struct AnyPropNameWriteCollector<'a>(&'a mut HashSet<String>);
impl Visit for AnyPropNameWriteCollector<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if !matches!(stmt, Stmt::FunctionDecl { .. }) {
            jsast::walk_stmt(self, stmt);
        }
    }
    fn visit_expr(&mut self, expr: &Expr) {
        if matches!(expr, Expr::Arrow(..)) {
            return;
        }
        if let Expr::Assign(lhs, _) | Expr::CompoundAssign(_, lhs, _) = expr {
            if let Expr::Member(_, prop) = lhs.as_ref() {
                self.0.insert(prop.clone());
            }
        }
        jsast::walk_expr(self, expr);
    }
}

pub(super) fn collect_any_prop_name_writes_expr(expr: &Expr, out: &mut HashSet<String>) {
    AnyPropNameWriteCollector(out).visit_expr(expr);
}

pub(super) fn collect_any_prop_name_writes_stmts(stmts: &[Stmt], out: &mut HashSet<String>) {
    AnyPropNameWriteCollector(out).visit_stmts(stmts);
}

fn stmts_continue_current_loop(stmts: &[Stmt], loop_label: Option<&str>) -> bool {
    struct ContinueFinder<'a> {
        loop_label: Option<&'a str>,
        nested: bool,
    }
    impl TryVisit for ContinueFinder<'_> {
        type Break = ();
        fn visit_stmt(&mut self, stmt: &Stmt) -> ControlFlow<()> {
            match stmt {
                Stmt::Continue(None) if !self.nested => ControlFlow::Break(()),
                Stmt::Continue(Some(lbl)) if self.loop_label == Some(lbl.as_str()) => {
                    ControlFlow::Break(())
                }
                Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
                    if self.loop_label.is_some() {
                        let prev = self.nested;
                        self.nested = true;
                        let res = self.visit_stmts(b);
                        self.nested = prev;
                        res
                    } else {
                        ControlFlow::Continue(())
                    }
                }
                Stmt::FunctionDecl { .. } => ControlFlow::Continue(()),
                _ => jsast::try_walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, _: &Expr) -> ControlFlow<()> {
            ControlFlow::Continue(())
        }
    }
    ContinueFinder {
        loop_label,
        nested: false,
    }
    .visit_stmts(stmts)
    .is_break()
}

pub(super) fn stmts_definitely_exit_loop(stmts: &[Stmt], loop_label: Option<&str>) -> bool {
    if stmts_continue_current_loop(stmts, loop_label) {
        return false;
    }
    match stmts.last() {
        Some(Stmt::Break(_) | Stmt::Return(_) | Stmt::Throw(_)) => true,
        Some(Stmt::ExprStmt(Expr::Call(callee, _))) if is_panic_callee(callee) => true,
        Some(Stmt::Continue(Some(target))) => loop_label != Some(target.as_str()),
        Some(Stmt::If(_, then_b, Some(else_b))) => {
            stmts_definitely_exit_loop(then_b, loop_label)
                && stmts_definitely_exit_loop(else_b, loop_label)
        }
        Some(Stmt::Block(b)) => stmts_definitely_exit_loop(b, loop_label),
        _ => false,
    }
}

pub(super) fn extract_iter0_then_body_mut(stmt: &mut Stmt) -> Option<&mut [Stmt]> {
    let lp = stmt.as_loop_mut()?;
    if lp.cond.is_some() {
        return Some(lp.body.as_mut_slice());
    }
    let loop_label = lp.label;
    match lp.body.as_mut_slice() {
        [Stmt::If(_, then_b, Some(_))] | [_, Stmt::If(_, then_b, Some(_))] => {
            Some(then_b.as_mut_slice())
        }
        [Stmt::If(_, exit_b, None), rest @ ..]
            if !rest.is_empty() && stmts_definitely_exit_loop(exit_b, loop_label) =>
        {
            Some(rest)
        }
        _ => None,
    }
}
