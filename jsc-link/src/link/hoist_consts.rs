//! Post-lowering AST constant hoisting and unreachable-throw outlining.
//!
//! Runs after Phase C lowering when `opts.optimizer == OptimizerMode::On`:
//! - Single-assignment read-only scalar array lookup tables (`len >= 8`, at any statement depth
//!   including inside loops/branches) are hoisted to module-level `const` items.
//! - Repeated or in-loop `Location` literals (`{ filename, line, col }`), ES2018 `BigInt("...")`
//!   calls, and inline scalar array indexing literals (`[...][i]`) are hoisted to module `const`s.
//! - Repeated `throw __rt.abort_error(new Error("rustc_codegen_js: entered unreachable code"))`
//!   statements (`>= 2` occurrences) are outlined into a shared zero-argument helper function.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{item_map, ItemKind, JsItem, JsName, LinkItem, Linkage};
use crate::jir::decline::Decline;
use crate::jsast::{self, ArrowBody, Expr, Stmt, SwitchCase};
use crate::lower::LoweredItem;
use crate::opt::OptStats;
use crate::opts::{EcmaTarget, OptimizerMode, Opts};

pub(super) type LoweredRow = (
    JsName,
    Result<LoweredItem, Decline>,
    OptStats,
    Option<String>,
);

fn is_pure_scalar_lit(expr: &Expr) -> bool {
    match expr {
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null => true,
        Expr::Unary(op, inner) => {
            matches!(
                op,
                jsast::UnOp::Neg | jsast::UnOp::Pos | jsast::UnOp::Not | jsast::UnOp::BitNot
            ) && is_pure_scalar_lit(inner)
        }
        _ => false,
    }
}

fn is_scalar_array_lit(expr: &Expr, min_len: usize) -> bool {
    let Expr::Array(elems) = expr else {
        return false;
    };
    elems.len() >= min_len && elems.iter().all(is_pure_scalar_lit)
}

fn is_location_object(expr: &Expr) -> bool {
    let Expr::Object(entries) = expr else {
        return false;
    };
    if entries.len() != 3 {
        return false;
    }
    matches!(
        (&entries[0], &entries[1], &entries[2]),
        (
            (k0, Expr::Str(_)),
            (k1, Expr::Num(_) | Expr::RawNum(_)),
            (k2, Expr::Num(_) | Expr::RawNum(_)),
        ) if k0 == "filename" && k1 == "line" && k2 == "col"
    )
}

fn is_unreachable_abort_expr(expr: &Expr) -> bool {
    let Expr::Call(callee, args) = expr else {
        return false;
    };
    let Expr::Member(ns, method) = callee.as_ref() else {
        return false;
    };
    if !matches!(ns.as_ref(), Expr::Ident(id) if id == "__rt") || method != "abort_error" {
        return false;
    }
    let [Expr::New(err_ctor, err_args)] = args.as_slice() else {
        return false;
    };
    if !matches!(err_ctor.as_ref(), Expr::Ident(id) if id == "Error") {
        return false;
    }
    matches!(
        err_args.as_slice(),
        [Expr::Str(msg)] if msg == "rustc_codegen_js: entered unreachable code"
    )
}

#[derive(Default, Clone, Copy)]
struct ConstUsage {
    count: usize,
    in_loop: bool,
}

struct HoistCollector {
    bigint_enabled: bool,
    counts: BTreeMap<String, (Expr, ConstUsage)>,
    unreachable_throws: usize,
}

impl HoistCollector {
    fn record(&mut self, key: String, expr: &Expr, in_loop: bool) {
        let entry = self
            .counts
            .entry(key)
            .or_insert_with(|| (expr.clone(), ConstUsage::default()));
        entry.1.count += 1;
        entry.1.in_loop |= in_loop;
    }

    fn visit_stmt(&mut self, stmt: &Stmt, in_loop: bool) {
        match stmt {
            Stmt::ExprStmt(e) => self.visit_expr(e, in_loop, false),
            Stmt::Let(_, init) => {
                if let Some(e) = init {
                    self.visit_expr(e, in_loop, false);
                }
            }
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(e) = init {
                        self.visit_expr(e, in_loop, false);
                    }
                }
            }
            Stmt::Const(_, e) => self.visit_expr(e, in_loop, false),
            Stmt::Return(Some(e)) => self.visit_expr(e, in_loop, false),
            Stmt::Return(None) => {}
            Stmt::Throw(e) => {
                if is_unreachable_abort_expr(e) {
                    self.unreachable_throws += 1;
                } else {
                    self.visit_expr(e, in_loop, false);
                }
            }
            Stmt::If(test, then_b, else_b) => {
                self.visit_expr(test, in_loop, false);
                for s in then_b {
                    self.visit_stmt(s, in_loop);
                }
                if let Some(else_b) = else_b {
                    for s in else_b {
                        self.visit_stmt(s, in_loop);
                    }
                }
            }
            Stmt::Switch(disc, cases) => {
                self.visit_expr(disc, in_loop, false);
                for c in cases {
                    for t in &c.tests {
                        self.visit_expr(t, in_loop, false);
                    }
                    for s in &c.body {
                        self.visit_stmt(s, in_loop);
                    }
                }
            }
            Stmt::Labeled(_, inner) => self.visit_stmt(inner, in_loop),
            Stmt::LoopForever(_, body) => {
                for s in body {
                    self.visit_stmt(s, true);
                }
            }
            Stmt::While(test, body) => {
                self.visit_expr(test, true, false);
                for s in body {
                    self.visit_stmt(s, true);
                }
            }
            Stmt::TryCatch { body, handler, .. } => {
                for s in body {
                    self.visit_stmt(s, in_loop);
                }
                for s in handler {
                    self.visit_stmt(s, in_loop);
                }
            }
            Stmt::FunctionDecl { body, .. } | Stmt::Block(body) => {
                for s in body {
                    self.visit_stmt(s, in_loop);
                }
            }
            Stmt::Continue(_)
            | Stmt::Break(_)
            | Stmt::Import { .. }
            | Stmt::Comment(_)
            | Stmt::Raw(_)
            | Stmt::Loc(_) => {}
        }
    }

    fn visit_expr(&mut self, expr: &Expr, in_loop: bool, is_assign_target: bool) {
        if !is_assign_target {
            if self.bigint_enabled {
                if let Expr::BigInt(digits) = expr {
                    self.record(format!("bi:{digits}"), expr, in_loop);
                    return;
                }
            }
            if is_location_object(expr) {
                let key = format!("loc:{expr}");
                self.record(key, expr, in_loop);
                return;
            }
            if let Expr::Index(obj, idx) = expr {
                if is_scalar_array_lit(obj, 4) {
                    let key = format!("arr:{obj}");
                    self.record(key, obj, in_loop);
                    self.visit_expr(idx, in_loop, false);
                    return;
                }
            }
        }
        match expr {
            Expr::Member(obj, _) => self.visit_expr(obj, in_loop, false),
            Expr::Index(obj, idx) => {
                self.visit_expr(obj, in_loop, false);
                self.visit_expr(idx, in_loop, false);
            }
            Expr::Call(callee, args) | Expr::New(callee, args) => {
                self.visit_expr(callee, in_loop, false);
                for a in args {
                    self.visit_expr(a, in_loop, false);
                }
            }
            Expr::Unary(_, inner) => self.visit_expr(inner, in_loop, false),
            Expr::Binary(_, l, r) => {
                self.visit_expr(l, in_loop, false);
                self.visit_expr(r, in_loop, false);
            }
            Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                self.visit_expr(l, in_loop, true);
                self.visit_expr(r, in_loop, false);
            }
            Expr::Cond(t, c, a) => {
                self.visit_expr(t, in_loop, false);
                self.visit_expr(c, in_loop, false);
                self.visit_expr(a, in_loop, false);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    self.visit_expr(v, in_loop, false);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for item in items {
                    self.visit_expr(item, in_loop, false);
                }
            }
            Expr::Arrow(_, ArrowBody::Expr(b)) => self.visit_expr(b, in_loop, false),
            Expr::Arrow(_, ArrowBody::Block(stmts)) => {
                for s in stmts {
                    self.visit_stmt(s, in_loop);
                }
            }
            _ => {}
        }
    }
}

/// Collects `let` bindings declared in `stmts` (recursing through nested control-flow statements
/// such as loops and conditionals, without entering nested functions or arrows).
fn collect_declared_locals(stmts: &[Stmt], params: &BTreeSet<String>, out: &mut Vec<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::Let(n, _) => {
                if !params.contains(n) {
                    out.push(n.clone());
                }
            }
            Stmt::Lets(decls) => {
                for (n, _) in decls {
                    if !params.contains(n) {
                        out.push(n.clone());
                    }
                }
            }
            Stmt::If(_, a, b) => {
                collect_declared_locals(a, params, out);
                if let Some(b) = b {
                    collect_declared_locals(b, params, out);
                }
            }
            Stmt::Switch(_, cases) => {
                for SwitchCase { body, .. } in cases {
                    collect_declared_locals(body, params, out);
                }
            }
            Stmt::Labeled(_, inner) => {
                collect_declared_locals(std::slice::from_ref(inner.as_ref()), params, out);
            }
            Stmt::LoopForever(_, b) | Stmt::While(_, b) | Stmt::Block(b) => {
                collect_declared_locals(b, params, out);
            }
            Stmt::TryCatch { body, handler, .. } => {
                collect_declared_locals(body, params, out);
                collect_declared_locals(handler, params, out);
            }
            Stmt::ExprStmt(_)
            | Stmt::Const(_, _)
            | Stmt::Return(_)
            | Stmt::Throw(_)
            | Stmt::Continue(_)
            | Stmt::Break(_)
            | Stmt::FunctionDecl { .. }
            | Stmt::Import { .. }
            | Stmt::Comment(_)
            | Stmt::Raw(_)
            | Stmt::Loc(_) => {}
        }
    }
}

/// Checks whether `local` across the entire `body` (at any nesting depth, including inside loops)
/// is declared once, initialized once to a pure scalar array literal (`len >= 8`), and only ever
/// read as `local[index_expr]` in rvalue position (never mutated, shadowed, or escaped).
fn find_read_only_table_init(body: &[Stmt], local: &str) -> Option<Expr> {
    struct Check<'a> {
        local: &'a str,
        decl_count: usize,
        found_init: Option<Expr>,
        reads: usize,
        bad: bool,
    }
    impl Check<'_> {
        fn stmt(&mut self, s: &Stmt) {
            if self.bad {
                return;
            }
            match s {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) if matches!(lhs.as_ref(), Expr::Ident(n) if n == self.local) =>
                {
                    if self.found_init.is_some() || !is_scalar_array_lit(rhs, 8) {
                        self.bad = true;
                        return;
                    }
                    self.found_init = Some((**rhs).clone());
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) => self.expr(e, false),
                Stmt::Let(n, init) => {
                    if n == self.local {
                        self.decl_count += 1;
                        if self.decl_count > 1 {
                            self.bad = true;
                            return;
                        }
                        if let Some(rhs) = init {
                            if self.found_init.is_some() || !is_scalar_array_lit(rhs, 8) {
                                self.bad = true;
                                return;
                            }
                            self.found_init = Some(rhs.clone());
                        }
                        return;
                    }
                    if let Some(e) = init {
                        self.expr(e, false);
                    }
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if n == self.local {
                            self.decl_count += 1;
                            if self.decl_count > 1 {
                                self.bad = true;
                                return;
                            }
                            if let Some(rhs) = init {
                                if self.found_init.is_some() || !is_scalar_array_lit(rhs, 8) {
                                    self.bad = true;
                                    return;
                                }
                                self.found_init = Some(rhs.clone());
                            }
                            continue;
                        }
                        if let Some(e) = init {
                            self.expr(e, false);
                        }
                    }
                }
                Stmt::Const(n, e) => {
                    if n == self.local {
                        self.bad = true;
                        return;
                    }
                    self.expr(e, false);
                }
                Stmt::Return(Some(e)) => self.expr(e, false),
                Stmt::Return(None) => {}
                Stmt::If(t, a, b) => {
                    self.expr(t, false);
                    a.iter().for_each(|x| self.stmt(x));
                    if let Some(b) = b {
                        b.iter().for_each(|x| self.stmt(x));
                    }
                }
                Stmt::Switch(d, cases) => {
                    self.expr(d, false);
                    for SwitchCase { tests, body } in cases {
                        tests.iter().for_each(|t| self.expr(t, false));
                        body.iter().for_each(|x| self.stmt(x));
                    }
                }
                Stmt::Labeled(_, inner) => self.stmt(inner),
                Stmt::LoopForever(_, b) | Stmt::Block(b) => b.iter().for_each(|x| self.stmt(x)),
                Stmt::While(t, b) => {
                    self.expr(t, false);
                    b.iter().for_each(|x| self.stmt(x));
                }
                Stmt::TryCatch {
                    body,
                    binding,
                    handler,
                } => {
                    if binding == self.local {
                        self.bad = true;
                        return;
                    }
                    body.iter().for_each(|x| self.stmt(x));
                    handler.iter().for_each(|x| self.stmt(x));
                }
                Stmt::FunctionDecl { name, params, body } => {
                    if name == self.local || params.iter().any(|p| p == self.local) {
                        self.bad = true;
                        return;
                    }
                    body.iter().for_each(|x| self.stmt(x));
                }
                Stmt::Raw(_) => {
                    self.bad = true;
                }
                _ => {}
            }
        }

        fn expr(&mut self, e: &Expr, is_lhs: bool) {
            if self.bad {
                return;
            }
            match e {
                Expr::Ident(name) => {
                    if name == self.local {
                        self.bad = true;
                    }
                }
                Expr::Index(obj, idx) => {
                    if let Expr::Ident(name) = obj.as_ref() {
                        if name == self.local {
                            if is_lhs {
                                self.bad = true;
                                return;
                            }
                            self.reads += 1;
                            self.expr(idx, false);
                            return;
                        }
                    }
                    self.expr(obj, false);
                    self.expr(idx, false);
                }
                Expr::Member(obj, _) => self.expr(obj, false),
                Expr::Call(c, args) | Expr::New(c, args) => {
                    self.expr(c, false);
                    args.iter().for_each(|a| self.expr(a, false));
                }
                Expr::Unary(_, inner) => self.expr(inner, false),
                Expr::Binary(_, l, r) => {
                    self.expr(l, false);
                    self.expr(r, false);
                }
                Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                    self.expr(l, true);
                    self.expr(r, false);
                }
                Expr::Cond(t, c, a) => {
                    self.expr(t, false);
                    self.expr(c, false);
                    self.expr(a, false);
                }
                Expr::Object(entries) => entries.iter().for_each(|(_, v)| self.expr(v, false)),
                Expr::Array(items) | Expr::Seq(items) => {
                    items.iter().for_each(|x| self.expr(x, false));
                }
                Expr::Arrow(params, body) => {
                    if params.iter().any(|p| p == self.local) {
                        self.bad = true;
                        return;
                    }
                    match body {
                        ArrowBody::Expr(b) => self.expr(b, false),
                        ArrowBody::Block(stmts) => stmts.iter().for_each(|s| self.stmt(s)),
                    }
                }
                Expr::Raw(_) => self.bad = true,
                _ => {}
            }
        }
    }

    let mut check = Check {
        local,
        decl_count: 0,
        found_init: None,
        reads: 0,
        bad: false,
    };
    for stmt in body {
        check.stmt(stmt);
    }
    if !check.bad && check.decl_count == 1 && check.reads >= 1 {
        check.found_init
    } else {
        None
    }
}

fn strip_hoisted_table_stmts(stmts: &mut Vec<Stmt>, hoisted: &BTreeSet<String>) {
    let mut out = Vec::with_capacity(stmts.len());
    for mut s in std::mem::take(stmts) {
        match &mut s {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs))
                if matches!(lhs.as_ref(), Expr::Ident(n) if hoisted.contains(n))
                    && is_scalar_array_lit(rhs, 8) =>
            {
                continue;
            }
            Stmt::Let(n, _) if hoisted.contains(n) => continue,
            Stmt::Lets(decls) => {
                decls.retain(|(n, _)| !hoisted.contains(n));
                if decls.is_empty() {
                    continue;
                }
            }
            Stmt::If(_, a, b) => {
                strip_hoisted_table_stmts(a, hoisted);
                if let Some(b) = b {
                    strip_hoisted_table_stmts(b, hoisted);
                }
            }
            Stmt::Switch(_, cases) => {
                for c in cases {
                    strip_hoisted_table_stmts(&mut c.body, hoisted);
                }
            }
            Stmt::Labeled(_, inner) => {
                let mut v = vec![std::mem::replace(inner.as_mut(), Stmt::Block(Vec::new()))];
                strip_hoisted_table_stmts(&mut v, hoisted);
                **inner = if v.len() == 1 {
                    v.pop().unwrap()
                } else {
                    Stmt::Block(v)
                };
            }
            Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::While(_, b) => {
                strip_hoisted_table_stmts(b, hoisted);
            }
            Stmt::TryCatch { body, handler, .. } => {
                strip_hoisted_table_stmts(body, hoisted);
                strip_hoisted_table_stmts(handler, hoisted);
            }
            _ => {}
        }
        out.push(s);
    }
    *stmts = out;
}

fn replace_ident_in_stmts(stmts: &mut [Stmt], from: &str, to: &str) {
    for stmt in stmts {
        match stmt {
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Const(_, e) => {
                replace_ident_in_expr(e, from, to);
            }
            Stmt::Let(_, Some(e)) | Stmt::Return(Some(e)) => replace_ident_in_expr(e, from, to),
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(e) = init {
                        replace_ident_in_expr(e, from, to);
                    }
                }
            }
            Stmt::If(t, a, b) => {
                replace_ident_in_expr(t, from, to);
                replace_ident_in_stmts(a, from, to);
                if let Some(b) = b {
                    replace_ident_in_stmts(b, from, to);
                }
            }
            Stmt::Switch(d, cases) => {
                replace_ident_in_expr(d, from, to);
                for c in cases {
                    for t in &mut c.tests {
                        replace_ident_in_expr(t, from, to);
                    }
                    replace_ident_in_stmts(&mut c.body, from, to);
                }
            }
            Stmt::Labeled(_, inner) => {
                replace_ident_in_stmts(std::slice::from_mut(inner), from, to)
            }
            Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::FunctionDecl { body: b, .. } => {
                replace_ident_in_stmts(b, from, to);
            }
            Stmt::While(t, b) => {
                replace_ident_in_expr(t, from, to);
                replace_ident_in_stmts(b, from, to);
            }
            Stmt::TryCatch { body, handler, .. } => {
                replace_ident_in_stmts(body, from, to);
                replace_ident_in_stmts(handler, from, to);
            }
            _ => {}
        }
    }
}

fn replace_ident_in_expr(expr: &mut Expr, from: &str, to: &str) {
    match expr {
        Expr::Ident(name) if name == from => *name = to.to_owned(),
        Expr::Member(obj, _) => replace_ident_in_expr(obj, from, to),
        Expr::Index(obj, idx) => {
            replace_ident_in_expr(obj, from, to);
            replace_ident_in_expr(idx, from, to);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            replace_ident_in_expr(c, from, to);
            for a in args {
                replace_ident_in_expr(a, from, to);
            }
        }
        Expr::Unary(_, inner) => replace_ident_in_expr(inner, from, to),
        Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
            replace_ident_in_expr(l, from, to);
            replace_ident_in_expr(r, from, to);
        }
        Expr::Cond(t, c, a) => {
            replace_ident_in_expr(t, from, to);
            replace_ident_in_expr(c, from, to);
            replace_ident_in_expr(a, from, to);
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                replace_ident_in_expr(v, from, to);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                replace_ident_in_expr(item, from, to);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(b)) => replace_ident_in_expr(b, from, to),
        Expr::Arrow(_, ArrowBody::Block(stmts)) => replace_ident_in_stmts(stmts, from, to),
        _ => {}
    }
}

struct HoistRewriter<'a> {
    bigint_enabled: bool,
    const_map: &'a BTreeMap<String, JsName>,
    unreachable_fn: Option<&'a JsName>,
    changed: bool,
}

impl HoistRewriter<'_> {
    fn rewrite_stmt(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::ExprStmt(e) => self.rewrite_expr(e, false),
            Stmt::Let(_, Some(e)) => self.rewrite_expr(e, false),
            Stmt::Let(_, None) => {}
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(e) = init {
                        self.rewrite_expr(e, false);
                    }
                }
            }
            Stmt::Const(_, e) => self.rewrite_expr(e, false),
            Stmt::Return(Some(e)) => self.rewrite_expr(e, false),
            Stmt::Return(None) => {}
            Stmt::Throw(e) => {
                if let Some(ur_name) = self.unreachable_fn {
                    if is_unreachable_abort_expr(e) {
                        *e = Expr::Call(
                            Box::new(Expr::Ident(ur_name.as_str().to_owned())),
                            Vec::new(),
                        );
                        self.changed = true;
                        return;
                    }
                }
                self.rewrite_expr(e, false);
            }
            Stmt::If(t, a, b) => {
                self.rewrite_expr(t, false);
                for s in a {
                    self.rewrite_stmt(s);
                }
                if let Some(b) = b {
                    for s in b {
                        self.rewrite_stmt(s);
                    }
                }
            }
            Stmt::Switch(d, cases) => {
                self.rewrite_expr(d, false);
                for c in cases {
                    for t in &mut c.tests {
                        self.rewrite_expr(t, false);
                    }
                    for s in &mut c.body {
                        self.rewrite_stmt(s);
                    }
                }
            }
            Stmt::Labeled(_, inner) => self.rewrite_stmt(inner),
            Stmt::LoopForever(_, body) | Stmt::Block(body) | Stmt::FunctionDecl { body, .. } => {
                for s in body {
                    self.rewrite_stmt(s);
                }
            }
            Stmt::While(t, body) => {
                self.rewrite_expr(t, false);
                for s in body {
                    self.rewrite_stmt(s);
                }
            }
            Stmt::TryCatch { body, handler, .. } => {
                for s in body {
                    self.rewrite_stmt(s);
                }
                for s in handler {
                    self.rewrite_stmt(s);
                }
            }
            _ => {}
        }
    }

    fn rewrite_expr(&mut self, expr: &mut Expr, is_assign_target: bool) {
        if !is_assign_target {
            if self.bigint_enabled {
                if let Expr::BigInt(digits) = expr {
                    let key = format!("bi:{digits}");
                    if let Some(hoisted) = self.const_map.get(&key) {
                        *expr = Expr::Ident(hoisted.as_str().to_owned());
                        self.changed = true;
                        return;
                    }
                }
            }
            if is_location_object(expr) {
                let key = format!("loc:{expr}");
                if let Some(hoisted) = self.const_map.get(&key) {
                    *expr = Expr::Ident(hoisted.as_str().to_owned());
                    self.changed = true;
                    return;
                }
            }
            if let Expr::Index(obj, idx) = expr {
                if is_scalar_array_lit(obj, 4) {
                    let key = format!("arr:{obj}");
                    if let Some(hoisted) = self.const_map.get(&key) {
                        **obj = Expr::Ident(hoisted.as_str().to_owned());
                        self.changed = true;
                        self.rewrite_expr(idx, false);
                        return;
                    }
                }
            }
        }
        match expr {
            Expr::Member(obj, _) => self.rewrite_expr(obj, false),
            Expr::Index(obj, idx) => {
                self.rewrite_expr(obj, false);
                self.rewrite_expr(idx, false);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                self.rewrite_expr(c, false);
                for a in args {
                    self.rewrite_expr(a, false);
                }
            }
            Expr::Unary(_, inner) => self.rewrite_expr(inner, false),
            Expr::Binary(_, l, r) => {
                self.rewrite_expr(l, false);
                self.rewrite_expr(r, false);
            }
            Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                self.rewrite_expr(l, true);
                self.rewrite_expr(r, false);
            }
            Expr::Cond(t, c, a) => {
                self.rewrite_expr(t, false);
                self.rewrite_expr(c, false);
                self.rewrite_expr(a, false);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    self.rewrite_expr(v, false);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for item in items {
                    self.rewrite_expr(item, false);
                }
            }
            Expr::Arrow(_, ArrowBody::Expr(b)) => self.rewrite_expr(b, false),
            Expr::Arrow(_, ArrowBody::Block(stmts)) => {
                for s in stmts {
                    self.rewrite_stmt(s);
                }
            }
            _ => {}
        }
    }
}

fn fresh_item_name(
    prefix: &str,
    counter: &mut usize,
    table: &BTreeMap<JsName, LinkItem>,
    defined: &BTreeSet<JsName>,
) -> JsName {
    loop {
        let candidate = JsName::new(format!("{prefix}_{}", *counter));
        *counter += 1;
        if !table.contains_key(&candidate) && !defined.contains(&candidate) {
            return candidate;
        }
    }
}

fn build_synthesized_link_item(
    name: JsName,
    kind: ItemKind,
    decl: Stmt,
    debug_path: &str,
) -> LinkItem {
    let refs = super::lane::refs_of(&decl, &name);
    let mut js_item = JsItem {
        name: name.clone(),
        kind,
        decl,
        refs: refs.clone(),
        zombies: Vec::new(),
        linkage: Linkage::internal(),
        debug_path: debug_path.to_owned(),
        order: None,
        loc: None,
        names: Vec::new(),
        dts: None,
        optimization: Default::default(),
        late_ir: Default::default(),
        baseline_shadow: Default::default(),
        jir: None,
        jir_decline: None,
        jir_item: None,
    };
    crate::minify::shorten_locals(std::slice::from_mut(&mut js_item));
    let (js, mappings) = js_item.to_js_mapped(crate::opts::get().comments);
    LinkItem {
        name,
        kind,
        js,
        refs,
        zombies: Vec::new(),
        linkage: Linkage::internal(),
        debug_path: debug_path.to_owned(),
        crate_name: "generated".to_owned(),
        order: None,
        map: item_map(mappings),
        dts: None,
        optimization: Default::default(),
        late_ir: Default::default(),
        baseline_shadow: Default::default(),
        jir: Default::default(),
    }
}

/// Hoists read-only scalar lookup tables, `BigInt("...")` constants, repeated/loop `Location`
/// objects, and outlines repeated `entered unreachable code` throws into a shared helper.
///
/// Only runs when `opts.optimizer == OptimizerMode::On`. Returns `true` if any synthesized module
/// item was inserted into `table` (signaling downstream reachability recomputation).
pub(crate) fn run(
    table: &mut BTreeMap<JsName, LinkItem>,
    lowered: &mut [LoweredRow],
    defined: &mut BTreeSet<JsName>,
    live: &mut BTreeSet<JsName>,
) -> bool {
    run_with_opts(table, lowered, defined, live, crate::opts::get())
}

fn run_with_opts(
    table: &mut BTreeMap<JsName, LinkItem>,
    lowered: &mut [LoweredRow],
    defined: &mut BTreeSet<JsName>,
    live: &mut BTreeSet<JsName>,
    opts: &Opts,
) -> bool {
    if opts.optimizer != OptimizerMode::On {
        return false;
    }

    let mut added_any = false;
    let mut counter = 0usize;
    let mut synthesized: Vec<LinkItem> = Vec::new();
    let mut table_dedup: BTreeMap<String, JsName> = BTreeMap::new();

    // Pass 1: Hoist single-assignment read-only scalar lookup tables at any nesting depth.
    for (item_name, res, _, _) in lowered.iter_mut() {
        let Ok(lowered_item) = res else { continue };
        let Stmt::FunctionDecl { params, body, .. } = &mut lowered_item.decl else {
            continue;
        };
        let param_set: BTreeSet<String> = params.iter().cloned().collect();
        let mut declared_locals: Vec<String> = Vec::new();
        collect_declared_locals(body, &param_set, &mut declared_locals);

        let mut candidates: Vec<(String, Expr)> = Vec::new();
        for local in &declared_locals {
            if let Some(arr_expr) = find_read_only_table_init(body, local) {
                candidates.push((local.clone(), arr_expr));
            }
        }

        if !candidates.is_empty() {
            let mut hoisted_set: BTreeSet<String> = BTreeSet::new();
            let mut replacements: Vec<(String, JsName)> = Vec::new();
            for (local, arr_expr) in candidates {
                let key = format!("arr:{arr_expr}");
                let hc_name = if let Some(existing) = table_dedup.get(&key) {
                    existing.clone()
                } else {
                    let new_name = fresh_item_name("$hc", &mut counter, table, defined);
                    defined.insert(new_name.clone());
                    live.insert(new_name.clone());
                    let item = build_synthesized_link_item(
                        new_name.clone(),
                        ItemKind::Const,
                        jsast::const_(new_name.as_str(), arr_expr),
                        "hoisted constant table",
                    );
                    synthesized.push(item);
                    table_dedup.insert(key, new_name.clone());
                    added_any = true;
                    new_name
                };
                hoisted_set.insert(local.clone());
                replacements.push((local, hc_name));
            }

            strip_hoisted_table_stmts(body, &hoisted_set);
            for (local, hc_name) in replacements {
                replace_ident_in_stmts(body, &local, hc_name.as_str());
            }
            lowered_item.refs = super::lane::refs_of(&lowered_item.decl, item_name);
        }
    }

    // Pass 2: Expression constants (`BigInt("...")` on ES2018, `Location` objects, inline
    // `[...][i]`) and shared unreachable-abort outlining.
    let bigint_enabled = opts.ecma_target == EcmaTarget::Es2018;
    let mut collector = HoistCollector {
        bigint_enabled,
        counts: BTreeMap::new(),
        unreachable_throws: 0,
    };
    for (_, res, _, _) in lowered.iter() {
        if let Ok(lowered_item) = res {
            collector.visit_stmt(&lowered_item.decl, false);
        }
    }

    let mut const_map: BTreeMap<String, JsName> = BTreeMap::new();
    for (key, (expr, usage)) in &collector.counts {
        let qualify = if key.starts_with("bi:") || key.starts_with("loc:") {
            usage.in_loop || usage.count >= 2
        } else if key.starts_with("arr:") {
            let len = match expr {
                Expr::Array(elems) => elems.len(),
                _ => 0,
            };
            usage.in_loop || usage.count >= 2 || len >= 8
        } else {
            false
        };
        if !qualify {
            continue;
        }
        let hc_name = if let Some(existing) = table_dedup.get(key) {
            existing.clone()
        } else {
            let new_name = fresh_item_name("$hc", &mut counter, table, defined);
            defined.insert(new_name.clone());
            live.insert(new_name.clone());
            let item = build_synthesized_link_item(
                new_name.clone(),
                ItemKind::Const,
                jsast::const_(new_name.as_str(), expr.clone()),
                "hoisted constant",
            );
            synthesized.push(item);
            table_dedup.insert(key.clone(), new_name.clone());
            added_any = true;
            new_name
        };
        const_map.insert(key.clone(), hc_name);
    }

    let unreachable_fn = if collector.unreachable_throws >= 2 {
        let ur_name = fresh_item_name("$ur", &mut counter, table, defined);
        defined.insert(ur_name.clone());
        live.insert(ur_name.clone());
        let abort_expr = Expr::Call(
            Box::new(Expr::Member(
                Box::new(Expr::Ident("__rt".to_owned())),
                "abort_error".to_owned(),
            )),
            vec![Expr::New(
                Box::new(Expr::Ident("Error".to_owned())),
                vec![Expr::Str(
                    "rustc_codegen_js: entered unreachable code".to_owned(),
                )],
            )],
        );
        let decl = jsast::function(
            ur_name.as_str(),
            Vec::new(),
            vec![Stmt::Return(Some(abort_expr))],
        );
        let item = build_synthesized_link_item(
            ur_name.clone(),
            ItemKind::Fn,
            decl,
            "shared unreachable error helper",
        );
        synthesized.push(item);
        added_any = true;
        Some(ur_name)
    } else {
        None
    };

    if !const_map.is_empty() || unreachable_fn.is_some() {
        for (item_name, res, _, _) in lowered.iter_mut() {
            let Ok(lowered_item) = res else { continue };
            let mut rewriter = HoistRewriter {
                bigint_enabled,
                const_map: &const_map,
                unreachable_fn: unreachable_fn.as_ref(),
                changed: false,
            };
            rewriter.rewrite_stmt(&mut lowered_item.decl);
            if rewriter.changed {
                lowered_item.refs = super::lane::refs_of(&lowered_item.decl, item_name);
            }
        }
    }

    for item in synthesized {
        table.insert(item.name.clone(), item);
    }

    added_any
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestRunOutput = (
        BTreeMap<JsName, LinkItem>,
        Vec<LoweredRow>,
        BTreeSet<JsName>,
        BTreeSet<JsName>,
        bool,
    );

    fn test_run(opts: Opts, items: Vec<(&str, Stmt)>) -> TestRunOutput {
        let mut table = BTreeMap::new();
        let mut defined = BTreeSet::new();
        let mut live = BTreeSet::new();
        let mut lowered = Vec::new();
        for (name, decl) in items {
            let js_name = JsName::new(name.to_owned());
            let refs = super::super::lane::refs_of(&decl, &js_name);
            defined.insert(js_name.clone());
            live.insert(js_name.clone());
            lowered.push((
                js_name,
                Ok(LoweredItem {
                    decl,
                    refs,
                    zombies: Vec::new(),
                    loc: None,
                    names: Vec::new(),
                    dts: None,
                    js_alloc_origins: Vec::new(),
                }),
                OptStats::default(),
                None,
            ));
        }
        let added = run_with_opts(&mut table, &mut lowered, &mut defined, &mut live, &opts);
        (table, lowered, defined, live, added)
    }

    #[test]
    fn hoists_nested_loop_lookup_table_to_module_const() {
        // `function crc(i) { while (true) { let t = [0,1,2,3,4,5,6,7]; return t[i]; } }`
        let table_elems: Vec<Expr> = (0..8).map(|n| Expr::Num(n as f64)).collect();
        let fn_stmt = jsast::function(
            "crc",
            vec!["i".to_owned()],
            vec![Stmt::LoopForever(
                None,
                vec![
                    Stmt::Let("t".to_owned(), Some(Expr::Array(table_elems))),
                    Stmt::Return(Some(Expr::Index(
                        Box::new(Expr::Ident("t".to_owned())),
                        Box::new(Expr::Ident("i".to_owned())),
                    ))),
                ],
            )],
        );
        let opts = Opts {
            optimizer: OptimizerMode::On,
            ..Opts::default()
        };
        let (table, lowered, defined, live, added) = test_run(opts, vec![("crc", fn_stmt)]);
        assert!(added);
        assert_eq!(table.len(), 1);
        let (hc_name, hc_item) = table.iter().next().unwrap();
        assert_eq!(hc_item.kind, ItemKind::Const);
        assert!(
            hc_item.js.contains("[0, 1, 2, 3, 4, 5, 6, 7]")
                || hc_item.js.contains("[0,1,2,3,4,5,6,7]")
        );
        assert!(defined.contains(hc_name) && live.contains(hc_name));
        let Ok(item) = &lowered[0].1 else {
            unreachable!()
        };
        assert!(item.refs.contains(hc_name));
        let rendered = jsast::program_to_string_styled_for_target(
            std::slice::from_ref(&item.decl),
            jsast::Style::Compact,
            EcmaTarget::Latest,
        );
        assert!(
            !rendered.contains("let t"),
            "hoisted local declaration stripped: {rendered}"
        );
        assert!(rendered.contains(hc_name.as_str()));
    }

    #[test]
    fn hoists_in_loop_location_bigint_es2018_and_shared_unreachable_helper() {
        let loc = Expr::Object(vec![
            ("filename".to_owned(), Expr::Str("lib.rs".to_owned())),
            ("line".to_owned(), Expr::Num(42.0)),
            ("col".to_owned(), Expr::Num(7.0)),
        ]);
        let unreachable_throw = || {
            Stmt::Throw(Expr::Call(
                Box::new(Expr::Member(
                    Box::new(Expr::Ident("__rt".to_owned())),
                    "abort_error".to_owned(),
                )),
                vec![Expr::New(
                    Box::new(Expr::Ident("Error".to_owned())),
                    vec![Expr::Str(
                        "rustc_codegen_js: entered unreachable code".to_owned(),
                    )],
                )],
            ))
        };
        let fn_stmt = jsast::function(
            "demo",
            vec!["c".to_owned()],
            vec![Stmt::LoopForever(
                None,
                vec![
                    Stmt::ExprStmt(Expr::Call(
                        Box::new(Expr::Ident("use_vals".to_owned())),
                        vec![loc, Expr::BigInt("12345678901234567890".to_owned())],
                    )),
                    Stmt::If(
                        Expr::Ident("c".to_owned()),
                        vec![unreachable_throw()],
                        Some(vec![unreachable_throw()]),
                    ),
                ],
            )],
        );
        let opts = Opts {
            optimizer: OptimizerMode::On,
            ecma_target: EcmaTarget::Es2018,
            ..Opts::default()
        };
        let (table, lowered, _, _, added) = test_run(opts, vec![("demo", fn_stmt.clone())]);
        assert!(added);
        // Expect 2 constants (loc + BigInt) and 1 unreachable helper function ($ur_0).
        assert_eq!(table.len(), 3);
        assert_eq!(
            table.values().filter(|i| i.kind == ItemKind::Const).count(),
            2
        );
        assert_eq!(table.values().filter(|i| i.kind == ItemKind::Fn).count(), 1);
        let Ok(item) = &lowered[0].1 else {
            unreachable!()
        };
        let rendered = jsast::program_to_string_styled_for_target(
            std::slice::from_ref(&item.decl),
            jsast::Style::Compact,
            EcmaTarget::Es2018,
        );
        assert!(
            !rendered.contains("entered unreachable code"),
            "throws outlined: {rendered}"
        );
        assert!(
            !rendered.contains("BigInt("),
            "BigInt hoisted on Es2018: {rendered}"
        );
        assert!(
            !rendered.contains("lib.rs"),
            "in-loop Location hoisted: {rendered}"
        );

        // Report mode must NOT hoist anything (preserves off == report byte-identity).
        let report_opts = Opts {
            optimizer: OptimizerMode::Report,
            ..Opts::default()
        };
        let (report_table, _, _, _, report_added) = test_run(report_opts, vec![("demo", fn_stmt)]);
        assert!(!report_added);
        assert!(report_table.is_empty());
    }
}
