//! Elides `__rt.bstr_in(X)` for local variables that are only compared against ASCII constants,
//! checked for ASCII prefixes/suffixes/substrings, inspected via `charCodeAt(0)` against ASCII
//! bounds, or tested for emptiness.

use std::collections::{BTreeMap, BTreeSet};

use crate::jsast::{
    self, walk_expr, walk_expr_mut, walk_stmt, walk_stmt_mut, Expr, Stmt, Visit, VisitMut,
};

use super::{collect_fn_locals, is_ascii_str_lit, unwrap_bstr_in_call};

fn is_const_ascii_init(expr: &Expr) -> bool {
    if is_ascii_str_lit(expr) {
        return true;
    }
    // Matches `["ASCII"][0]` (unboxed single-element `&str` slot constant).
    if let Expr::Index(arr, idx) = expr {
        if matches!(idx.as_ref(), Expr::Num(n) if *n == 0.0) {
            if let Expr::Array(items) = arr.as_ref() {
                return items.len() == 1 && is_ascii_str_lit(&items[0]);
            }
        }
    }
    false
}

struct ConstAsciiCollector<'a> {
    locals: &'a BTreeSet<String>,
    writes: BTreeMap<String, (usize, bool)>,
}

impl ConstAsciiCollector<'_> {
    fn record(&mut self, n: &str, init: &Expr) {
        if self.locals.contains(n) {
            let entry = self.writes.entry(n.to_owned()).or_insert((0, true));
            entry.0 += 1;
            if !is_const_ascii_init(init) {
                entry.1 = false;
            }
        }
    }
}

impl Visit for ConstAsciiCollector<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(n, Some(init)) | Stmt::Const(n, init) => {
                self.record(n, init);
                self.visit_expr(init);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(init) = init {
                        self.record(n, init);
                        self.visit_expr(init);
                    }
                }
            }
            Stmt::TryCatch {
                body,
                binding,
                handler,
            } => {
                if self.locals.contains(binding) {
                    self.writes.insert(binding.clone(), (2, false));
                }
                self.visit_stmts(body);
                self.visit_stmts(handler);
            }
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, e: &Expr) {
        match e {
            Expr::Assign(lhs, rhs) => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    if self.locals.contains(n) {
                        self.record(n, rhs);
                        self.visit_expr(rhs);
                        return;
                    }
                }
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
            Expr::CompoundAssign(_, lhs, rhs) => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    if self.locals.contains(n) {
                        let entry = self.writes.entry(n.clone()).or_insert((0, true));
                        entry.0 += 1;
                        entry.1 = false;
                    }
                }
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
            _ => walk_expr(self, e),
        }
    }
}

fn collect_const_ascii_locals(stmts: &[Stmt], locals: &BTreeSet<String>) -> BTreeSet<String> {
    let mut collector = ConstAsciiCollector {
        locals,
        writes: BTreeMap::new(),
    };
    collector.visit_stmts(stmts);
    collector
        .writes
        .into_iter()
        .filter(|(_, (count, ok))| *count == 1 && *ok)
        .map(|(n, _)| n)
        .collect()
}

fn is_ascii_expr(expr: &Expr, const_ascii: &BTreeSet<String>) -> bool {
    is_ascii_str_lit(expr) || matches!(expr, Expr::Ident(n) if const_ascii.contains(n))
}

#[derive(Default)]
struct LocalInfo {
    bstr_in_writes: usize,
    copy_from: Vec<String>,
    bad: bool,
}

fn match_charcode0_var<'a>(e: &'a Expr, locals: &BTreeSet<String>) -> Option<&'a str> {
    let Expr::Call(callee, args) = e else {
        return None;
    };
    if args.len() != 1 || !matches!(&args[0], Expr::Num(n) if *n == 0.0) {
        return None;
    }
    let Expr::Member(obj, method) = callee.as_ref() else {
        return None;
    };
    if method != "charCodeAt" {
        return None;
    }
    match obj.as_ref() {
        Expr::Ident(n) if locals.contains(n) => Some(n.as_str()),
        _ => None,
    }
}

fn check_ascii_charcode_bound(op: jsast::BinOp, k: f64) -> bool {
    if !k.is_finite() || k.fract() != 0.0 || k < 0.0 {
        return false;
    }
    match op {
        jsast::BinOp::StrictEq | jsast::BinOp::StrictNe | jsast::BinOp::Le | jsast::BinOp::Gt => {
            k <= 127.0
        }
        jsast::BinOp::Lt | jsast::BinOp::Ge => k <= 128.0,
        _ => false,
    }
}

fn is_ascii_charcode0_cmp<'a>(
    op: jsast::BinOp,
    l: &'a Expr,
    r: &'a Expr,
    locals: &BTreeSet<String>,
) -> Option<&'a str> {
    if let (Some(n), Expr::Num(k)) = (match_charcode0_var(l, locals), r) {
        if check_ascii_charcode_bound(op, *k) {
            return Some(n);
        }
    }
    if let (Expr::Num(k), Some(n)) = (l, match_charcode0_var(r, locals)) {
        let flipped = match op {
            jsast::BinOp::StrictEq => jsast::BinOp::StrictEq,
            jsast::BinOp::StrictNe => jsast::BinOp::StrictNe,
            jsast::BinOp::Lt => jsast::BinOp::Gt,
            jsast::BinOp::Le => jsast::BinOp::Ge,
            jsast::BinOp::Gt => jsast::BinOp::Lt,
            jsast::BinOp::Ge => jsast::BinOp::Le,
            _ => return None,
        };
        if check_ascii_charcode_bound(flipped, *k) {
            return Some(n);
        }
    }
    None
}

fn match_local_length<'a>(e: &'a Expr, locals: &BTreeSet<String>) -> Option<&'a str> {
    let Expr::Member(obj, prop) = e else {
        return None;
    };
    if prop != "length" {
        return None;
    }
    match obj.as_ref() {
        Expr::Ident(n) if locals.contains(n) => Some(n.as_str()),
        _ => None,
    }
}

fn is_emptiness_cmp<'a>(
    op: jsast::BinOp,
    l: &'a Expr,
    r: &'a Expr,
    locals: &BTreeSet<String>,
) -> Option<&'a str> {
    if let (Some(n), Expr::Num(k)) = (match_local_length(l, locals), r) {
        if *k == 0.0
            && matches!(
                op,
                jsast::BinOp::StrictEq | jsast::BinOp::StrictNe | jsast::BinOp::Gt
            )
        {
            return Some(n);
        }
    }
    if let (Expr::Num(k), Some(n)) = (l, match_local_length(r, locals)) {
        if *k == 0.0
            && matches!(
                op,
                jsast::BinOp::StrictEq | jsast::BinOp::StrictNe | jsast::BinOp::Lt
            )
        {
            return Some(n);
        }
    }
    None
}

struct AsciiInScan<'a> {
    locals: &'a BTreeSet<String>,
    const_ascii: &'a BTreeSet<String>,
    in_closure: bool,
    info: BTreeMap<String, LocalInfo>,
}

impl AsciiInScan<'_> {
    fn mark_bad_expr(&mut self, expr: &Expr) {
        let prev = self.in_closure;
        self.in_closure = true;
        self.visit_expr(expr);
        self.in_closure = prev;
    }

    fn is_safe_ascii_read(&self, expr: &Expr) -> bool {
        if let Expr::Binary(op, l, r) = expr {
            if matches!(op, jsast::BinOp::StrictEq | jsast::BinOp::StrictNe) {
                if let Expr::Ident(n) = l.as_ref() {
                    if self.locals.contains(n) && is_ascii_expr(r, self.const_ascii) {
                        return true;
                    }
                }
                if let Expr::Ident(n) = r.as_ref() {
                    if self.locals.contains(n) && is_ascii_expr(l, self.const_ascii) {
                        return true;
                    }
                }
            }
            if is_ascii_charcode0_cmp(*op, l, r, self.locals).is_some()
                || is_emptiness_cmp(*op, l, r, self.locals).is_some()
            {
                return true;
            }
        }
        if let Expr::Call(callee, args) = expr {
            if args.len() == 1 && is_ascii_expr(&args[0], self.const_ascii) {
                if let Expr::Member(obj, method) = callee.as_ref() {
                    if matches!(method.as_str(), "startsWith" | "endsWith" | "includes") {
                        if let Expr::Ident(n) = obj.as_ref() {
                            if self.locals.contains(n) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    fn scan_write_rhs(&mut self, lhs: &str, rhs: &Expr) -> bool {
        if let Some(arg) = unwrap_bstr_in_call(rhs) {
            self.info.entry(lhs.to_owned()).or_default().bstr_in_writes += 1;
            self.visit_expr(&arg);
            return true;
        }
        if is_ascii_expr(rhs, self.const_ascii) {
            return true;
        }
        if let Expr::Ident(src) = rhs {
            if self.locals.contains(src) {
                self.info
                    .entry(lhs.to_owned())
                    .or_default()
                    .copy_from
                    .push(src.clone());
                return true;
            }
        }
        if let Expr::Cond(cond, t, f) = rhs {
            let check_arm = |arm: &Expr| {
                unwrap_bstr_in_call(arm).is_some()
                    || is_ascii_expr(arm, self.const_ascii)
                    || matches!(arm, Expr::Ident(s) if self.locals.contains(s))
            };
            if check_arm(t) && check_arm(f) {
                self.visit_expr(cond);
                for arm in [t.as_ref(), f.as_ref()] {
                    if let Some(arg) = unwrap_bstr_in_call(arm) {
                        self.info.entry(lhs.to_owned()).or_default().bstr_in_writes += 1;
                        self.visit_expr(&arg);
                    } else if let Expr::Ident(src) = arm {
                        if self.locals.contains(src) && !self.const_ascii.contains(src) {
                            self.info
                                .entry(lhs.to_owned())
                                .or_default()
                                .copy_from
                                .push(src.clone());
                        }
                    }
                }
                return true;
            }
        }
        false
    }
}

impl Visit for AsciiInScan<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(n, Some(init)) | Stmt::Const(n, init)
                if !self.in_closure && self.locals.contains(n) =>
            {
                if !self.scan_write_rhs(n, init) {
                    self.info.entry(n.clone()).or_default().bad = true;
                    self.visit_expr(init);
                }
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(init) = init {
                        if !self.in_closure && self.locals.contains(n) {
                            if self.scan_write_rhs(n, init) {
                                continue;
                            }
                            self.info.entry(n.clone()).or_default().bad = true;
                        }
                        self.visit_expr(init);
                    }
                }
            }
            Stmt::Switch(d, cases) => {
                let all_ascii_cases = cases
                    .iter()
                    .all(|c| c.tests.iter().all(|t| is_ascii_expr(t, self.const_ascii)));
                if !self.in_closure && all_ascii_cases {
                    if let Expr::Ident(n) = d {
                        if self.locals.contains(n) {
                            for c in cases {
                                self.visit_stmts(&c.body);
                            }
                            return;
                        }
                    }
                }
                walk_stmt(self, stmt);
            }
            Stmt::TryCatch {
                body,
                binding,
                handler,
            } => {
                if self.locals.contains(binding) {
                    self.info.entry(binding.clone()).or_default().bad = true;
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
        if !self.in_closure && self.is_safe_ascii_read(expr) {
            return;
        }
        match expr {
            Expr::Ident(n) => {
                if self.locals.contains(n) {
                    self.info.entry(n.clone()).or_default().bad = true;
                }
            }
            Expr::Assign(lhs, rhs) if !self.in_closure => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    if self.locals.contains(n) {
                        if self.scan_write_rhs(n, rhs) {
                            return;
                        }
                        self.info.entry(n.clone()).or_default().bad = true;
                        self.visit_expr(rhs);
                        return;
                    }
                }
                self.mark_bad_expr(lhs);
                self.visit_expr(rhs);
            }
            Expr::CompoundAssign(_, l, r) if !self.in_closure => {
                self.mark_bad_expr(l);
                self.visit_expr(r);
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

fn propagate_bad_copies(info: &mut BTreeMap<String, LocalInfo>) {
    loop {
        let mut changed = false;
        let bad_set: BTreeSet<String> = info
            .iter()
            .filter(|(_, v)| v.bad)
            .map(|(k, _)| k.clone())
            .collect();
        let mut to_mark = Vec::new();
        for (dst, v) in info.iter() {
            if bad_set.contains(dst) {
                for src in &v.copy_from {
                    if !bad_set.contains(src) {
                        to_mark.push(src.clone());
                    }
                }
            }
        }
        for src in to_mark {
            let entry = info.entry(src).or_default();
            if !entry.bad {
                entry.bad = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

fn unwrap_bstr_in_rhs(rhs: &mut Expr) {
    if let Some(arg) = unwrap_bstr_in_call(rhs) {
        *rhs = arg;
        return;
    }
    if let Expr::Cond(_, t, f) = rhs {
        if let Some(arg) = unwrap_bstr_in_call(t) {
            **t = arg;
        }
        if let Some(arg) = unwrap_bstr_in_call(f) {
            **f = arg;
        }
    }
}

struct AsciiInRewriter<'a>(&'a BTreeSet<String>);

impl VisitMut for AsciiInRewriter<'_> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Let(n, Some(init)) | Stmt::Const(n, init) if self.0.contains(n) => {
                unwrap_bstr_in_rhs(init);
                self.visit_expr_mut(init);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(init) = init {
                        if self.0.contains(n) {
                            unwrap_bstr_in_rhs(init);
                        }
                        self.visit_expr_mut(init);
                    }
                }
            }
            Stmt::FunctionDecl { .. } => {}
            _ => walk_stmt_mut(self, stmt),
        }
    }

    fn visit_expr_mut(&mut self, e: &mut Expr) {
        match e {
            Expr::Assign(lhs, rhs) => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    if self.0.contains(n) {
                        unwrap_bstr_in_rhs(rhs);
                        self.visit_expr_mut(rhs);
                        return;
                    }
                }
                self.visit_expr_mut(lhs);
                self.visit_expr_mut(rhs);
            }
            Expr::Arrow(_, _) => {}
            _ => walk_expr_mut(self, e),
        }
    }
}

/// Elides `__rt.bstr_in(X)` when the bound local (and any pure copy aliases of it) is ONLY
/// compared via `===`/`!==`/`switch` against all-ASCII string literals (`c < 128`) or
/// single-assignment const ASCII string locals (`F = "ASCII"` / `F = ["ASCII"][0]`).
pub(in crate::lower::bstr) fn elide_ascii_only_bstr_in(body: &mut [Stmt]) {
    let mut locals = BTreeSet::new();
    collect_fn_locals(body, &mut locals);
    if locals.is_empty() {
        return;
    }
    let const_ascii = collect_const_ascii_locals(body, &locals);
    let mut scan = AsciiInScan {
        locals: &locals,
        const_ascii: &const_ascii,
        in_closure: false,
        info: BTreeMap::new(),
    };
    scan.visit_stmts(body);
    let mut info = scan.info;
    propagate_bad_copies(&mut info);
    let elided: BTreeSet<String> = info
        .into_iter()
        .filter(|(_, v)| !v.bad && v.bstr_in_writes >= 1)
        .map(|(k, _)| k)
        .collect();
    if !elided.is_empty() {
        AsciiInRewriter(&elided).visit_stmts_mut(body);
    }
}
