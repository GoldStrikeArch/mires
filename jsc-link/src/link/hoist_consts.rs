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
use crate::jir::types::{
    FieldKey, NominalAbi, NominalForm, PinnedBoundary, TypeId, TypeKind, TypeTable,
};
use crate::jsast::{self, ArrowBody, BinOp, Expr, Stmt, SwitchCase, UnOp};
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

fn is_pure_factory_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null => true,
        Expr::Unary(op, inner) => {
            matches!(op, UnOp::Neg | UnOp::Pos | UnOp::Not | UnOp::BitNot)
                && is_pure_factory_expr(inner)
        }
        Expr::Call(callee, args) => {
            matches!(&**callee, Expr::Ident(n) if n == "BigInt")
                && args.len() == 1
                && matches!(&args[0], Expr::Str(_) | Expr::Num(_))
        }
        Expr::Array(items) => items.len() <= 32 && items.iter().all(is_pure_factory_expr),
        Expr::Object(entries) => {
            entries.len() <= 32 && entries.iter().all(|(_, v)| is_pure_factory_expr(v))
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
            if let Expr::Arrow(params, ArrowBody::Expr(inner)) = expr {
                if params.is_empty() && is_pure_factory_expr(inner) {
                    let key = format!("fn:{expr}");
                    self.record(key, expr, in_loop);
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
            if let Expr::Arrow(params, ArrowBody::Expr(inner)) = expr {
                if params.is_empty() && is_pure_factory_expr(inner) {
                    let key = format!("fn:{expr}");
                    if let Some(hoisted) = self.const_map.get(&key) {
                        *expr = Expr::Ident(hoisted.as_str().to_owned());
                        self.changed = true;
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

    // Whole-program `BigInt` object property `(lo, hi)` splitting and internal enum `TAG` integer
    // encoding run before Pass 2 so eliminated `BigInt` constants are never hoisted as ES2018
    // `BigInt("...")` bindings.
    let exclusions = collect_program_exclusions(table, lowered, live);
    split_bigint_props(lowered, &exclusions);
    encode_internal_tags(lowered, &exclusions);

    // Pass 2: Expression constants (`BigInt("...")` on ES2018, `Location` objects, inline
    // `[...][i]`, capture-free `() => <const>`) and shared unreachable-abort outlining.
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
            key.starts_with("fn:")
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

struct ProgramExclusions {
    raw_tokens: BTreeSet<String>,
    raw_strings: BTreeSet<String>,
    boundary_fields: BTreeSet<String>,
    boundary_variants: BTreeSet<String>,
}

fn collect_js_text_tokens_and_strings(
    text: &str,
    tokens: &mut BTreeSet<String>,
    strings: &mut BTreeSet<String>,
) {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_alphabetic() || b == b'_' || b == b'$' {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
            {
                i += 1;
            }
            tokens.insert(text[start..i].to_owned());
        } else if b == b'"' || b == b'\'' {
            let quote = b;
            i += 1;
            let start = i;
            let mut has_escape = false;
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' {
                    has_escape = true;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let end = i.min(bytes.len());
            if !has_escape {
                strings.insert(text[start..end].to_owned());
            }
            if i < bytes.len() {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
}

fn collect_raw_from_stmts(
    stmts: &[Stmt],
    tokens: &mut BTreeSet<String>,
    strings: &mut BTreeSet<String>,
) {
    for stmt in stmts {
        collect_raw_from_stmt(stmt, tokens, strings);
    }
}

fn collect_raw_from_stmt(
    stmt: &Stmt,
    tokens: &mut BTreeSet<String>,
    strings: &mut BTreeSet<String>,
) {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Const(_, e) | Stmt::Throw(e) => {
            collect_raw_from_expr(e, tokens, strings);
        }
        Stmt::Let(_, Some(e)) | Stmt::Return(Some(e)) => {
            collect_raw_from_expr(e, tokens, strings);
        }
        Stmt::Lets(decls) => {
            for (_, init) in decls {
                if let Some(e) = init {
                    collect_raw_from_expr(e, tokens, strings);
                }
            }
        }
        Stmt::If(t, a, b) => {
            collect_raw_from_expr(t, tokens, strings);
            collect_raw_from_stmts(a, tokens, strings);
            if let Some(b) = b {
                collect_raw_from_stmts(b, tokens, strings);
            }
        }
        Stmt::Switch(d, cases) => {
            collect_raw_from_expr(d, tokens, strings);
            for case in cases {
                for t in &case.tests {
                    collect_raw_from_expr(t, tokens, strings);
                }
                collect_raw_from_stmts(&case.body, tokens, strings);
            }
        }
        Stmt::Labeled(_, inner) => collect_raw_from_stmt(inner, tokens, strings),
        Stmt::LoopForever(_, body) | Stmt::Block(body) | Stmt::FunctionDecl { body, .. } => {
            collect_raw_from_stmts(body, tokens, strings);
        }
        Stmt::While(t, body) => {
            collect_raw_from_expr(t, tokens, strings);
            collect_raw_from_stmts(body, tokens, strings);
        }
        Stmt::TryCatch { body, handler, .. } => {
            collect_raw_from_stmts(body, tokens, strings);
            collect_raw_from_stmts(handler, tokens, strings);
        }
        Stmt::Raw(text) => collect_js_text_tokens_and_strings(text, tokens, strings),
        Stmt::Let(_, None)
        | Stmt::Return(None)
        | Stmt::Continue(_)
        | Stmt::Break(_)
        | Stmt::Import { .. }
        | Stmt::Comment(_)
        | Stmt::Loc(_) => {}
    }
}

fn collect_raw_from_expr(
    expr: &Expr,
    tokens: &mut BTreeSet<String>,
    strings: &mut BTreeSet<String>,
) {
    match expr {
        Expr::Member(obj, _) => collect_raw_from_expr(obj, tokens, strings),
        Expr::Index(obj, idx) => {
            collect_raw_from_expr(obj, tokens, strings);
            collect_raw_from_expr(idx, tokens, strings);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            collect_raw_from_expr(c, tokens, strings);
            for a in args {
                collect_raw_from_expr(a, tokens, strings);
            }
        }
        Expr::Unary(_, inner) => collect_raw_from_expr(inner, tokens, strings),
        Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
            collect_raw_from_expr(l, tokens, strings);
            collect_raw_from_expr(r, tokens, strings);
        }
        Expr::Cond(t, c, a) => {
            collect_raw_from_expr(t, tokens, strings);
            collect_raw_from_expr(c, tokens, strings);
            collect_raw_from_expr(a, tokens, strings);
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                collect_raw_from_expr(v, tokens, strings);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                collect_raw_from_expr(item, tokens, strings);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(b)) => collect_raw_from_expr(b, tokens, strings),
        Expr::Arrow(_, ArrowBody::Block(stmts)) => collect_raw_from_stmts(stmts, tokens, strings),
        Expr::Raw(text) => collect_js_text_tokens_and_strings(text, tokens, strings),
        _ => {}
    }
}

fn walk_boundary_type(
    ty: TypeId,
    types: &TypeTable,
    interns: &crate::jir::Interns,
    visited: &mut BTreeSet<TypeId>,
    fields: &mut BTreeSet<String>,
    variants: &mut BTreeSet<String>,
) {
    if !visited.insert(ty) {
        return;
    }
    let Some(desc) = types.get(ty) else {
        return;
    };
    match &desc.kind {
        TypeKind::Adt {
            form,
            variants: adt_variants,
            args,
            ..
        } => {
            for v in adt_variants {
                if *form == NominalForm::Enum {
                    if let Some(name) = interns.str(v.name) {
                        variants.insert(name.to_owned());
                    }
                }
                for f in &v.fields {
                    if let FieldKey::Name(id) = f.key {
                        if let Some(name) = interns.str(id) {
                            fields.insert(name.to_owned());
                        }
                    }
                    walk_boundary_type(f.ty, types, interns, visited, fields, variants);
                }
            }
            for &arg in args {
                walk_boundary_type(arg, types, interns, visited, fields, variants);
            }
        }
        TypeKind::Tuple(elems) => {
            for &el in elems {
                walk_boundary_type(el, types, interns, visited, fields, variants);
            }
        }
        TypeKind::Array { element, .. }
        | TypeKind::Slice(element)
        | TypeKind::Reference {
            pointee: element, ..
        }
        | TypeKind::RawPointer {
            pointee: element, ..
        }
        | TypeKind::Pattern(element) => {
            walk_boundary_type(*element, types, interns, visited, fields, variants);
        }
        TypeKind::Closure { upvars, .. } | TypeKind::CoroutineClosure { upvars, .. } => {
            for &u in upvars {
                walk_boundary_type(u, types, interns, visited, fields, variants);
            }
        }
        TypeKind::Coroutine {
            upvars,
            saved_locals,
            variants: co_variants,
            ..
        } => {
            for &u in upvars {
                walk_boundary_type(u, types, interns, visited, fields, variants);
            }
            for &(_, lt) in saved_locals {
                walk_boundary_type(lt, types, interns, visited, fields, variants);
            }
            for v in co_variants {
                for f in &v.fields {
                    if let FieldKey::Name(id) = f.key {
                        if let Some(name) = interns.str(id) {
                            fields.insert(name.to_owned());
                        }
                    }
                    walk_boundary_type(f.ty, types, interns, visited, fields, variants);
                }
            }
        }
        _ => {}
    }
}

fn collect_program_exclusions(
    table: &BTreeMap<JsName, LinkItem>,
    lowered: &[LoweredRow],
    live: &BTreeSet<JsName>,
) -> ProgramExclusions {
    let lowered_ok: BTreeSet<&JsName> = lowered
        .iter()
        .filter_map(|(name, res, _, _)| res.is_ok().then_some(name))
        .collect();
    let lowered_dts: BTreeSet<&JsName> = lowered
        .iter()
        .filter_map(|(name, res, _, _)| {
            res.as_ref()
                .ok()
                .and_then(|item| item.dts.as_ref().map(|_| name))
        })
        .collect();

    let mut raw_tokens = BTreeSet::new();
    let mut raw_strings = BTreeSet::new();
    let mut boundary_fields = BTreeSet::new();
    let mut boundary_variants = BTreeSet::new();

    for item in table.values() {
        if !live.contains(&item.name) {
            continue;
        }
        if !lowered_ok.contains(&item.name) {
            collect_js_text_tokens_and_strings(&item.js, &mut raw_tokens, &mut raw_strings);
        }
        if let Some(interns) = item.jir.interns() {
            let mut visited = BTreeSet::new();
            for desc in interns.types.descriptors() {
                if desc.meta.boundary != PinnedBoundary::None || desc.meta.abi == NominalAbi::C {
                    walk_boundary_type(
                        desc.id,
                        &interns.types,
                        interns,
                        &mut visited,
                        &mut boundary_fields,
                        &mut boundary_variants,
                    );
                }
            }
            if item.linkage.exported || item.dts.is_some() || lowered_dts.contains(&item.name) {
                if let Some(body) = &item.jir.body {
                    for &arg_ty in &body.signature.arguments {
                        walk_boundary_type(
                            arg_ty,
                            &interns.types,
                            interns,
                            &mut visited,
                            &mut boundary_fields,
                            &mut boundary_variants,
                        );
                    }
                    walk_boundary_type(
                        body.signature.result,
                        &interns.types,
                        interns,
                        &mut visited,
                        &mut boundary_fields,
                        &mut boundary_variants,
                    );
                }
            }
        }
    }

    for (_, res, _, _) in lowered {
        if let Ok(item) = res {
            collect_raw_from_stmt(&item.decl, &mut raw_tokens, &mut raw_strings);
        }
    }

    ProgramExclusions {
        raw_tokens,
        raw_strings,
        boundary_fields,
        boundary_variants,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnpackPart {
    Lo,
    Hi,
}

fn is_bigint_32(expr: &Expr) -> bool {
    match expr {
        Expr::BigInt(d) => d == "32",
        Expr::Call(c, args) => {
            matches!(&**c, Expr::Ident(n) if n == "BigInt")
                && args.len() == 1
                && (matches!(&args[0], Expr::Str(s) if s == "32")
                    || matches!(&args[0], Expr::Num(n) if *n == 32.0))
        }
        _ => false,
    }
}

fn extract_bigint_call_arg(expr: &Expr) -> Option<&Expr> {
    let Expr::Call(c, args) = expr else {
        return None;
    };
    if matches!(&**c, Expr::Ident(n) if n == "BigInt") && args.len() == 1 {
        Some(&args[0])
    } else {
        None
    }
}

fn parse_bigint_const(expr: &Expr) -> Option<(i32, i32)> {
    fn parse_i128(e: &Expr) -> Option<i128> {
        match e {
            Expr::BigInt(s) => s.parse::<i128>().ok(),
            Expr::Unary(UnOp::Neg, inner) => parse_i128(inner)?.checked_neg(),
            Expr::Call(c, args)
                if matches!(&**c, Expr::Ident(n) if n == "BigInt") && args.len() == 1 =>
            {
                match &args[0] {
                    Expr::Str(s) => s.parse::<i128>().ok(),
                    Expr::Num(n) if n.is_finite() && n.fract() == 0.0 => {
                        if *n >= (i64::MIN as f64) && *n <= (u64::MAX as f64) {
                            Some(*n as i128)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
    let val = parse_i128(expr)?;
    if (i64::MIN as i128..=u64::MAX as i128).contains(&val) {
        Some((val as i32, (val >> 32) as i32))
    } else {
        None
    }
}

fn is_pure_no_call(expr: &Expr) -> bool {
    match expr {
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null
        | Expr::Ident(_) => true,
        Expr::Member(obj, _) => is_pure_no_call(obj),
        Expr::Index(obj, idx) => is_pure_no_call(obj) && is_pure_no_call(idx),
        Expr::Unary(_, inner) => is_pure_no_call(inner),
        Expr::Binary(_, l, r) => is_pure_no_call(l) && is_pure_no_call(r),
        Expr::Cond(t, c, a) => is_pure_no_call(t) && is_pure_no_call(c) && is_pure_no_call(a),
        _ => false,
    }
}

fn is_pure_obj(expr: &Expr) -> bool {
    match expr {
        Expr::Ident(_) => true,
        Expr::Member(inner, _) => is_pure_obj(inner),
        Expr::Index(inner, idx) => {
            is_pure_obj(inner) && matches!(&**idx, Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_))
        }
        _ => false,
    }
}

fn check_pack_pair<'a>(shifted: &'a Expr, low: &'a Expr) -> Option<(&'a Expr, &'a Expr)> {
    let Expr::Binary(BinOp::Shl, hi_call, shift) = shifted else {
        return None;
    };
    if !is_bigint_32(shift) {
        return None;
    }
    let hi_arg = extract_bigint_call_arg(hi_call)?;
    let lo_arg = extract_bigint_call_arg(low)?;
    if is_pure_no_call(lo_arg) && is_pure_no_call(hi_arg) {
        Some((lo_arg, hi_arg))
    } else {
        None
    }
}

fn extract_pack_bigint(expr: &Expr) -> Option<(&Expr, &Expr)> {
    let Expr::Binary(BinOp::BitOr, lhs, rhs) = expr else {
        return None;
    };
    check_pack_pair(lhs, rhs).or_else(|| check_pack_pair(rhs, lhs))
}

enum PackedRhs<'a> {
    Const,
    Pack(&'a Expr, &'a Expr),
    MemberCopy(&'a Expr),
}

fn classify_prop_rhs<'a>(expr: &'a Expr, prop: &str) -> Option<PackedRhs<'a>> {
    if parse_bigint_const(expr).is_some() {
        return Some(PackedRhs::Const);
    }
    if let Some((lo, hi)) = extract_pack_bigint(expr) {
        return Some(PackedRhs::Pack(lo, hi));
    }
    if let Expr::Member(obj, k) = expr {
        if k == prop && is_pure_obj(obj) {
            return Some(PackedRhs::MemberCopy(obj));
        }
    }
    None
}

fn match_unpack_bigint_part(expr: &Expr) -> Option<(UnpackPart, &Expr)> {
    let inner = match expr {
        Expr::Binary(BinOp::BitOr, inner, z) if matches!(&**z, Expr::Num(n) if *n == 0.0) => {
            inner.as_ref()
        }
        _ => expr,
    };
    let Expr::Call(num_callee, num_args) = inner else {
        return None;
    };
    if !matches!(&**num_callee, Expr::Ident(n) if n == "Number") || num_args.len() != 1 {
        return None;
    }
    let Expr::Call(as_int_callee, as_int_args) = &num_args[0] else {
        return None;
    };
    let Expr::Member(bi_obj, method) = &**as_int_callee else {
        return None;
    };
    if !matches!(&**bi_obj, Expr::Ident(n) if n == "BigInt")
        || method != "asIntN"
        || as_int_args.len() != 2
        || !matches!(&as_int_args[0], Expr::Num(n) if *n == 32.0)
    {
        return None;
    }
    match &as_int_args[1] {
        Expr::Binary(BinOp::Shr, base, shift) if is_bigint_32(shift) => {
            Some((UnpackPart::Hi, base.as_ref()))
        }
        other => Some((UnpackPart::Lo, other)),
    }
}

fn to_clean_i32_expr(mut expr: Expr) -> Expr {
    if let Expr::Binary(BinOp::UShr, inner, z) = expr {
        if matches!(&*z, Expr::Num(n) if *n == 0.0) {
            expr = *inner;
        } else {
            expr = Expr::Binary(BinOp::UShr, inner, z);
        }
    }
    match &expr {
        Expr::Num(n)
            if n.is_finite()
                && n.fract() == 0.0
                && *n >= (i32::MIN as f64)
                && *n <= (i32::MAX as f64) =>
        {
            expr
        }
        Expr::Binary(BinOp::BitOr, _, z) if matches!(&**z, Expr::Num(n) if *n == 0.0) => expr,
        _ => jsast::i32_wrap(expr),
    }
}

fn expr_mentions_ident(expr: &Expr, target: &str) -> bool {
    match expr {
        Expr::Ident(n) => n == target,
        Expr::Member(obj, _) => expr_mentions_ident(obj, target),
        Expr::Index(obj, idx) => {
            expr_mentions_ident(obj, target) || expr_mentions_ident(idx, target)
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            expr_mentions_ident(c, target) || args.iter().any(|a| expr_mentions_ident(a, target))
        }
        Expr::Unary(_, inner) => expr_mentions_ident(inner, target),
        Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
            expr_mentions_ident(l, target) || expr_mentions_ident(r, target)
        }
        Expr::Cond(t, c, a) => {
            expr_mentions_ident(t, target)
                || expr_mentions_ident(c, target)
                || expr_mentions_ident(a, target)
        }
        Expr::Object(entries) => entries.iter().any(|(_, v)| expr_mentions_ident(v, target)),
        Expr::Array(items) | Expr::Seq(items) => {
            items.iter().any(|x| expr_mentions_ident(x, target))
        }
        Expr::Arrow(params, body) => {
            params.iter().any(|p| p == target)
                || match body {
                    ArrowBody::Expr(b) => expr_mentions_ident(b, target),
                    ArrowBody::Block(stmts) => stmts.iter().any(|s| stmt_mentions_ident(s, target)),
                }
        }
        Expr::Raw(s) => s.contains(target),
        _ => false,
    }
}

fn stmt_mentions_ident(stmt: &Stmt, target: &str) -> bool {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Const(_, e) | Stmt::Throw(e) => expr_mentions_ident(e, target),
        Stmt::Let(n, init) => {
            n == target
                || init
                    .as_ref()
                    .is_some_and(|e| expr_mentions_ident(e, target))
        }
        Stmt::Lets(decls) => decls.iter().any(|(n, init)| {
            n == target
                || init
                    .as_ref()
                    .is_some_and(|e| expr_mentions_ident(e, target))
        }),
        Stmt::Return(Some(e)) => expr_mentions_ident(e, target),
        Stmt::If(t, a, b) => {
            expr_mentions_ident(t, target)
                || a.iter().any(|s| stmt_mentions_ident(s, target))
                || b.as_ref()
                    .is_some_and(|bs| bs.iter().any(|s| stmt_mentions_ident(s, target)))
        }
        Stmt::Switch(d, cases) => {
            expr_mentions_ident(d, target)
                || cases.iter().any(|c| {
                    c.tests.iter().any(|t| expr_mentions_ident(t, target))
                        || c.body.iter().any(|s| stmt_mentions_ident(s, target))
                })
        }
        Stmt::Labeled(_, inner) => stmt_mentions_ident(inner, target),
        Stmt::LoopForever(_, b) | Stmt::Block(b) => {
            b.iter().any(|s| stmt_mentions_ident(s, target))
        }
        Stmt::While(t, b) => {
            expr_mentions_ident(t, target) || b.iter().any(|s| stmt_mentions_ident(s, target))
        }
        Stmt::TryCatch {
            body,
            binding,
            handler,
        } => {
            binding == target
                || body.iter().any(|s| stmt_mentions_ident(s, target))
                || handler.iter().any(|s| stmt_mentions_ident(s, target))
        }
        Stmt::FunctionDecl { name, params, body } => {
            name == target
                || params.iter().any(|p| p == target)
                || body.iter().any(|s| stmt_mentions_ident(s, target))
        }
        Stmt::Raw(s) => s.contains(target),
        _ => false,
    }
}

fn collect_fn_unpack_temps(params: &[String], body: &[Stmt]) -> BTreeMap<String, String> {
    struct TempScan {
        decl_count: BTreeMap<String, usize>,
        assign_prop: BTreeMap<String, Option<String>>,
        read_ok: BTreeMap<String, bool>,
        bad_names: BTreeSet<String>,
        all_idents: BTreeSet<String>,
    }

    impl TempScan {
        fn mark_bad(&mut self, name: &str) {
            self.bad_names.insert(name.to_owned());
            self.all_idents.insert(name.to_owned());
        }

        fn record_assign(&mut self, name: &str, rhs: &Expr) {
            self.all_idents.insert(name.to_owned());
            if let Expr::Member(obj, prop) = rhs {
                if is_pure_obj(obj) && !expr_mentions_ident(obj, name) {
                    self.visit_expr(obj);
                    match self.assign_prop.get(name) {
                        None => {
                            self.assign_prop.insert(name.to_owned(), Some(prop.clone()));
                        }
                        Some(Some(prev)) if prev == prop => {}
                        _ => {
                            self.assign_prop.insert(name.to_owned(), None);
                            self.mark_bad(name);
                        }
                    }
                    return;
                }
            }
            self.assign_prop.insert(name.to_owned(), None);
            self.mark_bad(name);
            self.visit_expr(rhs);
        }

        fn visit_stmts(&mut self, stmts: &[Stmt]) {
            for s in stmts {
                self.visit_stmt(s);
            }
        }

        fn visit_stmt(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs))
                    if matches!(lhs.as_ref(), Expr::Ident(_)) =>
                {
                    if let Expr::Ident(name) = lhs.as_ref() {
                        self.record_assign(name, rhs);
                    }
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) => self.visit_expr(e),
                Stmt::Const(n, e) => {
                    self.mark_bad(n);
                    self.visit_expr(e);
                }
                Stmt::Let(n, init) => {
                    self.all_idents.insert(n.clone());
                    *self.decl_count.entry(n.clone()).or_insert(0) += 1;
                    if let Some(rhs) = init {
                        self.record_assign(n, rhs);
                    }
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        self.all_idents.insert(n.clone());
                        *self.decl_count.entry(n.clone()).or_insert(0) += 1;
                        if let Some(rhs) = init {
                            self.record_assign(n, rhs);
                        }
                    }
                }
                Stmt::Return(Some(e)) => self.visit_expr(e),
                Stmt::If(t, a, b) => {
                    self.visit_expr(t);
                    self.visit_stmts(a);
                    if let Some(b) = b {
                        self.visit_stmts(b);
                    }
                }
                Stmt::Switch(d, cases) => {
                    self.visit_expr(d);
                    for c in cases {
                        for t in &c.tests {
                            self.visit_expr(t);
                        }
                        self.visit_stmts(&c.body);
                    }
                }
                Stmt::Labeled(_, inner) => self.visit_stmt(inner),
                Stmt::LoopForever(_, b) | Stmt::Block(b) => self.visit_stmts(b),
                Stmt::While(t, b) => {
                    self.visit_expr(t);
                    self.visit_stmts(b);
                }
                Stmt::TryCatch {
                    body,
                    binding,
                    handler,
                } => {
                    self.mark_bad(binding);
                    self.visit_stmts(body);
                    self.visit_stmts(handler);
                }
                Stmt::FunctionDecl { name, params, body } => {
                    self.mark_bad(name);
                    for p in params {
                        self.mark_bad(p);
                    }
                    self.visit_stmts(body);
                }
                Stmt::Raw(text) => {
                    let mut dummy = BTreeSet::new();
                    collect_js_text_tokens_and_strings(text, &mut self.bad_names, &mut dummy);
                }
                Stmt::Return(None)
                | Stmt::Continue(_)
                | Stmt::Break(_)
                | Stmt::Import { .. }
                | Stmt::Comment(_)
                | Stmt::Loc(_) => {}
            }
        }

        fn visit_expr(&mut self, expr: &Expr) {
            if let Some((_, Expr::Ident(name))) = match_unpack_bigint_part(expr) {
                self.all_idents.insert(name.clone());
                self.read_ok.entry(name.clone()).or_insert(true);
                return;
            }
            match expr {
                Expr::Ident(name) => {
                    self.all_idents.insert(name.clone());
                    self.read_ok.insert(name.clone(), false);
                    self.mark_bad(name);
                }
                Expr::Member(obj, _) => self.visit_expr(obj),
                Expr::Index(obj, idx) => {
                    self.visit_expr(obj);
                    self.visit_expr(idx);
                }
                Expr::Call(c, args) | Expr::New(c, args) => {
                    self.visit_expr(c);
                    for a in args {
                        self.visit_expr(a);
                    }
                }
                Expr::Unary(_, inner) => self.visit_expr(inner),
                Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                    self.visit_expr(l);
                    self.visit_expr(r);
                }
                Expr::Cond(t, c, a) => {
                    self.visit_expr(t);
                    self.visit_expr(c);
                    self.visit_expr(a);
                }
                Expr::Object(entries) => {
                    for (_, v) in entries {
                        self.visit_expr(v);
                    }
                }
                Expr::Array(items) | Expr::Seq(items) => {
                    for x in items {
                        self.visit_expr(x);
                    }
                }
                Expr::Arrow(params, body) => {
                    for p in params {
                        self.mark_bad(p);
                    }
                    match body {
                        ArrowBody::Expr(b) => self.visit_expr(b),
                        ArrowBody::Block(stmts) => self.visit_stmts(stmts),
                    }
                }
                Expr::Raw(text) => {
                    let mut dummy = BTreeSet::new();
                    collect_js_text_tokens_and_strings(text, &mut self.bad_names, &mut dummy);
                }
                _ => {}
            }
        }
    }

    let mut scan = TempScan {
        decl_count: BTreeMap::new(),
        assign_prop: BTreeMap::new(),
        read_ok: BTreeMap::new(),
        bad_names: params.iter().cloned().collect(),
        all_idents: params.iter().cloned().collect(),
    };
    scan.visit_stmts(body);

    let mut result = BTreeMap::new();
    for (name, count) in &scan.decl_count {
        if *count != 1 || scan.bad_names.contains(name) {
            continue;
        }
        let Some(Some(prop)) = scan.assign_prop.get(name) else {
            continue;
        };
        if scan.read_ok.get(name) != Some(&true) {
            continue;
        }
        let lo_name = format!("{name}$lo");
        let hi_name = format!("{name}$hi");
        if scan.all_idents.contains(&lo_name)
            || scan.all_idents.contains(&hi_name)
            || scan.bad_names.contains(&lo_name)
            || scan.bad_names.contains(&hi_name)
        {
            continue;
        }
        result.insert(name.clone(), prop.clone());
    }
    result
}

fn is_fixed_blacklisted_prop(name: &str) -> bool {
    matches!(
        name,
        "_0" | "_1"
            | "_2"
            | "_3"
            | "_4"
            | "_5"
            | "_6"
            | "_7"
            | "_8"
            | "_9"
            | "TAG"
            | "buf"
            | "off"
            | "len"
            | "sc"
            | "ptr"
            | "meta"
    ) || name.ends_with("$lo")
        || name.ends_with("$hi")
}

struct BigIntPropScanner {
    excluded: BTreeSet<String>,
    has_pack_write: BTreeSet<String>,
}

impl BigIntPropScanner {
    fn exclude_prop_and_base(&mut self, name: &str) {
        self.excluded.insert(name.to_owned());
        if let Some(base) = name
            .strip_suffix("$lo")
            .or_else(|| name.strip_suffix("$hi"))
        {
            self.excluded.insert(base.to_owned());
        }
    }

    fn scan_stmts(&mut self, stmts: &[Stmt], temp_props: &BTreeMap<String, String>) {
        for s in stmts {
            self.scan_stmt(s, temp_props);
        }
    }

    fn scan_stmt(&mut self, stmt: &Stmt, temp_props: &BTreeMap<String, String>) {
        match stmt {
            Stmt::FunctionDecl { params, body, .. } => {
                let inner_temps = collect_fn_unpack_temps(params, body);
                self.scan_stmts(body, &inner_temps);
            }
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => match (lhs.as_ref(), rhs.as_ref()) {
                (Expr::Ident(v), Expr::Member(obj, k))
                    if temp_props.get(v).is_some_and(|p| p == k) && is_pure_obj(obj) =>
                {
                    self.scan_expr(obj, temp_props);
                }
                (Expr::Member(lhs_obj, k), _) => {
                    self.scan_expr(lhs_obj, temp_props);
                    if is_pure_obj(lhs_obj) {
                        match classify_prop_rhs(rhs, k) {
                            Some(PackedRhs::Const) => {
                                self.has_pack_write.insert(k.clone());
                            }
                            Some(PackedRhs::Pack(lo, hi)) => {
                                self.has_pack_write.insert(k.clone());
                                self.scan_expr(lo, temp_props);
                                self.scan_expr(hi, temp_props);
                            }
                            Some(PackedRhs::MemberCopy(rhs_obj)) => {
                                self.scan_expr(rhs_obj, temp_props);
                            }
                            None => {
                                self.exclude_prop_and_base(k);
                                self.scan_expr(rhs, temp_props);
                            }
                        }
                    } else {
                        self.exclude_prop_and_base(k);
                        self.scan_expr(rhs, temp_props);
                    }
                }
                _ => {
                    self.scan_expr(lhs, temp_props);
                    self.scan_expr(rhs, temp_props);
                }
            },
            Stmt::Let(n, Some(Expr::Member(obj, k)))
                if temp_props.get(n).is_some_and(|p| p == k) && is_pure_obj(obj) =>
            {
                self.scan_expr(obj, temp_props);
            }
            Stmt::Let(_, Some(e)) | Stmt::ExprStmt(e) | Stmt::Const(_, e) | Stmt::Throw(e) => {
                self.scan_expr(e, temp_props);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(e) = init {
                        if let Expr::Member(obj, k) = e {
                            if temp_props.get(n).is_some_and(|p| p == k) && is_pure_obj(obj) {
                                self.scan_expr(obj, temp_props);
                                continue;
                            }
                        }
                        self.scan_expr(e, temp_props);
                    }
                }
            }
            Stmt::Return(Some(e)) => self.scan_expr(e, temp_props),
            Stmt::If(t, a, b) => {
                self.scan_expr(t, temp_props);
                self.scan_stmts(a, temp_props);
                if let Some(b) = b {
                    self.scan_stmts(b, temp_props);
                }
            }
            Stmt::Switch(d, cases) => {
                self.scan_expr(d, temp_props);
                for c in cases {
                    for t in &c.tests {
                        self.scan_expr(t, temp_props);
                    }
                    self.scan_stmts(&c.body, temp_props);
                }
            }
            Stmt::Labeled(_, inner) => self.scan_stmt(inner, temp_props),
            Stmt::LoopForever(_, b) | Stmt::Block(b) => self.scan_stmts(b, temp_props),
            Stmt::While(t, b) => {
                self.scan_expr(t, temp_props);
                self.scan_stmts(b, temp_props);
            }
            Stmt::TryCatch { body, handler, .. } => {
                self.scan_stmts(body, temp_props);
                self.scan_stmts(handler, temp_props);
            }
            Stmt::Let(_, None)
            | Stmt::Return(None)
            | Stmt::Continue(_)
            | Stmt::Break(_)
            | Stmt::Import { .. }
            | Stmt::Comment(_)
            | Stmt::Raw(_)
            | Stmt::Loc(_) => {}
        }
    }

    fn scan_expr(&mut self, expr: &Expr, temp_props: &BTreeMap<String, String>) {
        if let Some((_, inner)) = match_unpack_bigint_part(expr) {
            match inner {
                Expr::Member(obj, k) if is_pure_obj(obj) => {
                    let _ = k;
                    self.scan_expr(obj, temp_props);
                    return;
                }
                Expr::Ident(v) if temp_props.contains_key(v) => {
                    return;
                }
                _ => {}
            }
        }
        match expr {
            Expr::Str(s) => {
                self.excluded.insert(s.clone());
            }
            Expr::Object(entries) => {
                let mut seen_keys = BTreeSet::new();
                for (k, val) in entries {
                    if !seen_keys.insert(k.as_str()) {
                        self.exclude_prop_and_base(k);
                    }
                    if is_fixed_blacklisted_prop(k) || self.excluded.contains(k) {
                        self.exclude_prop_and_base(k);
                        self.scan_expr(val, temp_props);
                        continue;
                    }
                    match classify_prop_rhs(val, k) {
                        Some(PackedRhs::Const) => {
                            self.has_pack_write.insert(k.clone());
                        }
                        Some(PackedRhs::Pack(lo, hi)) => {
                            self.has_pack_write.insert(k.clone());
                            self.scan_expr(lo, temp_props);
                            self.scan_expr(hi, temp_props);
                        }
                        Some(PackedRhs::MemberCopy(rhs_obj)) => {
                            self.scan_expr(rhs_obj, temp_props);
                        }
                        None => {
                            self.exclude_prop_and_base(k);
                            self.scan_expr(val, temp_props);
                        }
                    }
                }
            }
            Expr::Member(obj, k) => {
                self.exclude_prop_and_base(k);
                self.scan_expr(obj, temp_props);
            }
            Expr::Index(obj, idx) => {
                self.scan_expr(obj, temp_props);
                self.scan_expr(idx, temp_props);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                self.scan_expr(c, temp_props);
                for a in args {
                    self.scan_expr(a, temp_props);
                }
            }
            Expr::Unary(_, inner) => self.scan_expr(inner, temp_props),
            Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                self.scan_expr(l, temp_props);
                self.scan_expr(r, temp_props);
            }
            Expr::Cond(t, c, a) => {
                self.scan_expr(t, temp_props);
                self.scan_expr(c, temp_props);
                self.scan_expr(a, temp_props);
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    self.scan_expr(x, temp_props);
                }
            }
            Expr::Arrow(_, ArrowBody::Expr(b)) => self.scan_expr(b, &BTreeMap::new()),
            Expr::Arrow(_, ArrowBody::Block(stmts)) => self.scan_stmts(stmts, &BTreeMap::new()),
            _ => {}
        }
    }
}

struct BigIntPropRewriter<'a> {
    split_props: &'a BTreeSet<String>,
    changed: bool,
}

impl BigIntPropRewriter<'_> {
    fn split_rhs_expr(&mut self, expr: Expr, prop: &str) -> (Expr, Expr) {
        if let Some((lo, hi)) = parse_bigint_const(&expr) {
            return (jsast::num(f64::from(lo)), jsast::num(f64::from(hi)));
        }
        if let Some((lo_ref, hi_ref)) = extract_pack_bigint(&expr) {
            let mut lo = to_clean_i32_expr(lo_ref.clone());
            let mut hi = to_clean_i32_expr(hi_ref.clone());
            let empty = BTreeMap::new();
            self.rewrite_expr(&mut lo, &empty);
            self.rewrite_expr(&mut hi, &empty);
            return (lo, hi);
        }
        if let Expr::Member(mut obj, k) = expr {
            debug_assert_eq!(k, prop);
            let empty = BTreeMap::new();
            self.rewrite_expr(&mut obj, &empty);
            return (
                Expr::Member(obj.clone(), format!("{prop}$lo")),
                Expr::Member(obj, format!("{prop}$hi")),
            );
        }
        unreachable!("validated split_rhs_expr must match Const, Pack, or MemberCopy");
    }

    fn rewrite_stmts(&mut self, stmts: &mut Vec<Stmt>, temp_props: &BTreeMap<String, String>) {
        let mut out = Vec::with_capacity(stmts.len());
        for stmt in std::mem::take(stmts) {
            match stmt {
                Stmt::FunctionDecl {
                    name,
                    params,
                    mut body,
                } => {
                    let inner_temps = collect_fn_unpack_temps(&params, &body)
                        .into_iter()
                        .filter(|(_, p)| self.split_props.contains(p))
                        .collect();
                    self.rewrite_stmts(&mut body, &inner_temps);
                    out.push(Stmt::FunctionDecl { name, params, body });
                }
                Stmt::Let(n, init)
                    if temp_props
                        .get(&n)
                        .is_some_and(|p| self.split_props.contains(p)) =>
                {
                    let p = &temp_props[&n];
                    self.changed = true;
                    match init {
                        None => out.push(Stmt::Lets(vec![
                            (format!("{n}$lo"), None),
                            (format!("{n}$hi"), None),
                        ])),
                        Some(Expr::Member(mut obj, _)) => {
                            self.rewrite_expr(&mut obj, temp_props);
                            out.push(Stmt::Let(
                                format!("{n}$lo"),
                                Some(Expr::Member(obj.clone(), format!("{p}$lo"))),
                            ));
                            out.push(Stmt::Let(
                                format!("{n}$hi"),
                                Some(Expr::Member(obj, format!("{p}$hi"))),
                            ));
                        }
                        Some(_) => unreachable!("validated temp init"),
                    }
                }
                Stmt::Lets(decls) => {
                    let mut new_decls = Vec::with_capacity(decls.len());
                    for (n, mut init) in decls {
                        if let Some(p) =
                            temp_props.get(&n).filter(|p| self.split_props.contains(*p))
                        {
                            self.changed = true;
                            match init {
                                None => {
                                    new_decls.push((format!("{n}$lo"), None));
                                    new_decls.push((format!("{n}$hi"), None));
                                }
                                Some(Expr::Member(mut obj, _)) => {
                                    self.rewrite_expr(&mut obj, temp_props);
                                    new_decls.push((
                                        format!("{n}$lo"),
                                        Some(Expr::Member(obj.clone(), format!("{p}$lo"))),
                                    ));
                                    new_decls.push((
                                        format!("{n}$hi"),
                                        Some(Expr::Member(obj, format!("{p}$hi"))),
                                    ));
                                }
                                Some(_) => unreachable!("validated temp lets init"),
                            }
                        } else {
                            if let Some(e) = &mut init {
                                self.rewrite_expr(e, temp_props);
                            }
                            new_decls.push((n, init));
                        }
                    }
                    out.push(Stmt::Lets(new_decls));
                }
                Stmt::ExprStmt(Expr::Assign(mut lhs, mut rhs)) => match (&*lhs, &*rhs) {
                    (Expr::Ident(v), Expr::Member(obj, k))
                        if temp_props.get(v).is_some_and(|p| p == k)
                            && self.split_props.contains(k) =>
                    {
                        let mut obj_cloned = (**obj).clone();
                        self.rewrite_expr(&mut obj_cloned, temp_props);
                        self.changed = true;
                        out.push(Stmt::ExprStmt(jsast::assign(
                            jsast::id(format!("{v}$lo")),
                            Expr::Member(Box::new(obj_cloned.clone()), format!("{k}$lo")),
                        )));
                        out.push(Stmt::ExprStmt(jsast::assign(
                            jsast::id(format!("{v}$hi")),
                            Expr::Member(Box::new(obj_cloned), format!("{k}$hi")),
                        )));
                    }
                    (Expr::Member(lhs_obj, k), _) if self.split_props.contains(k) => {
                        let mut lhs_obj = (**lhs_obj).clone();
                        self.rewrite_expr(&mut lhs_obj, temp_props);
                        let prop = k.clone();
                        let (lo_rhs, hi_rhs) = self.split_rhs_expr(*rhs, &prop);
                        self.changed = true;
                        out.push(Stmt::ExprStmt(jsast::assign(
                            Expr::Member(Box::new(lhs_obj.clone()), format!("{prop}$lo")),
                            lo_rhs,
                        )));
                        out.push(Stmt::ExprStmt(jsast::assign(
                            Expr::Member(Box::new(lhs_obj), format!("{prop}$hi")),
                            hi_rhs,
                        )));
                    }
                    _ => {
                        self.rewrite_expr(&mut lhs, temp_props);
                        self.rewrite_expr(&mut rhs, temp_props);
                        out.push(Stmt::ExprStmt(Expr::Assign(lhs, rhs)));
                    }
                },
                mut other => {
                    self.rewrite_stmt_inner(&mut other, temp_props);
                    out.push(other);
                }
            }
        }
        *stmts = out;
    }

    fn rewrite_stmt_inner(&mut self, stmt: &mut Stmt, temp_props: &BTreeMap<String, String>) {
        match stmt {
            Stmt::ExprStmt(e) | Stmt::Const(_, e) | Stmt::Throw(e) => {
                self.rewrite_expr(e, temp_props);
            }
            Stmt::Let(_, Some(e)) | Stmt::Return(Some(e)) => self.rewrite_expr(e, temp_props),
            Stmt::If(t, a, b) => {
                self.rewrite_expr(t, temp_props);
                self.rewrite_stmts(a, temp_props);
                if let Some(b) = b {
                    self.rewrite_stmts(b, temp_props);
                }
            }
            Stmt::Switch(d, cases) => {
                self.rewrite_expr(d, temp_props);
                for c in cases {
                    for t in &mut c.tests {
                        self.rewrite_expr(t, temp_props);
                    }
                    self.rewrite_stmts(&mut c.body, temp_props);
                }
            }
            Stmt::Labeled(_, inner) => self.rewrite_stmt_inner(inner, temp_props),
            Stmt::LoopForever(_, b) | Stmt::Block(b) => self.rewrite_stmts(b, temp_props),
            Stmt::While(t, b) => {
                self.rewrite_expr(t, temp_props);
                self.rewrite_stmts(b, temp_props);
            }
            Stmt::TryCatch { body, handler, .. } => {
                self.rewrite_stmts(body, temp_props);
                self.rewrite_stmts(handler, temp_props);
            }
            _ => {}
        }
    }

    fn rewrite_expr(&mut self, expr: &mut Expr, temp_props: &BTreeMap<String, String>) {
        if let Some((part, inner)) = match_unpack_bigint_part(expr) {
            let suffix = match part {
                UnpackPart::Lo => "$lo",
                UnpackPart::Hi => "$hi",
            };
            match inner {
                Expr::Member(obj, k) if self.split_props.contains(k) => {
                    let mut new_obj = (**obj).clone();
                    self.rewrite_expr(&mut new_obj, temp_props);
                    *expr = Expr::Member(Box::new(new_obj), format!("{k}{suffix}"));
                    self.changed = true;
                    return;
                }
                Expr::Ident(v)
                    if temp_props
                        .get(v)
                        .is_some_and(|p| self.split_props.contains(p)) =>
                {
                    *expr = Expr::Ident(format!("{v}{suffix}"));
                    self.changed = true;
                    return;
                }
                _ => {}
            }
        }
        match expr {
            Expr::Object(entries) => {
                let mut new_entries = Vec::with_capacity(entries.len());
                for (k, mut v) in std::mem::take(entries) {
                    if self.split_props.contains(&k) {
                        let (lo_expr, hi_expr) = self.split_rhs_expr(v, &k);
                        new_entries.push((format!("{k}$lo"), lo_expr));
                        new_entries.push((format!("{k}$hi"), hi_expr));
                        self.changed = true;
                    } else {
                        self.rewrite_expr(&mut v, temp_props);
                        new_entries.push((k, v));
                    }
                }
                *entries = new_entries;
            }
            Expr::Member(obj, _) => self.rewrite_expr(obj, temp_props),
            Expr::Index(obj, idx) => {
                self.rewrite_expr(obj, temp_props);
                self.rewrite_expr(idx, temp_props);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                self.rewrite_expr(c, temp_props);
                for a in args {
                    self.rewrite_expr(a, temp_props);
                }
            }
            Expr::Unary(_, inner) => self.rewrite_expr(inner, temp_props),
            Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                self.rewrite_expr(l, temp_props);
                self.rewrite_expr(r, temp_props);
            }
            Expr::Cond(t, c, a) => {
                self.rewrite_expr(t, temp_props);
                self.rewrite_expr(c, temp_props);
                self.rewrite_expr(a, temp_props);
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    self.rewrite_expr(x, temp_props);
                }
            }
            Expr::Arrow(_, ArrowBody::Expr(b)) => self.rewrite_expr(b, &BTreeMap::new()),
            Expr::Arrow(_, ArrowBody::Block(stmts)) => {
                self.rewrite_stmts(stmts, &BTreeMap::new());
            }
            _ => {}
        }
    }
}

fn split_bigint_props(lowered: &mut [LoweredRow], exclusions: &ProgramExclusions) {
    let mut scanner = BigIntPropScanner {
        excluded: BTreeSet::new(),
        has_pack_write: BTreeSet::new(),
    };
    scanner
        .excluded
        .extend(exclusions.raw_tokens.iter().cloned());
    scanner
        .excluded
        .extend(exclusions.raw_strings.iter().cloned());
    scanner
        .excluded
        .extend(exclusions.boundary_fields.iter().cloned());

    let empty = BTreeMap::new();
    for (_, res, _, _) in lowered.iter() {
        if let Ok(item) = res {
            scanner.scan_stmt(&item.decl, &empty);
        }
    }

    let split_props: BTreeSet<String> = scanner
        .has_pack_write
        .into_iter()
        .filter(|p| !is_fixed_blacklisted_prop(p) && !scanner.excluded.contains(p))
        .collect();
    if split_props.is_empty() {
        return;
    }

    for (item_name, res, _, _) in lowered.iter_mut() {
        let Ok(item) = res else { continue };
        let mut rewriter = BigIntPropRewriter {
            split_props: &split_props,
            changed: false,
        };
        let mut single = vec![std::mem::replace(
            &mut item.decl,
            Stmt::Comment(String::new()),
        )];
        rewriter.rewrite_stmts(&mut single, &empty);
        item.decl = single.pop().expect("single decl must remain");
        if rewriter.changed {
            item.refs = super::lane::refs_of(&item.decl, item_name);
        }
    }
}

struct TagScanner {
    candidates: BTreeSet<String>,
    bad: BTreeSet<String>,
}

impl TagScanner {
    fn scan_stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            self.scan_stmt(s);
        }
    }

    fn scan_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Switch(disc, cases) => {
                if let Expr::Member(obj, prop) = disc {
                    if prop == "TAG" {
                        self.scan_expr(obj);
                        for c in cases {
                            for t in &c.tests {
                                if let Expr::Str(s) = t {
                                    self.candidates.insert(s.clone());
                                } else {
                                    self.scan_expr(t);
                                }
                            }
                            self.scan_stmts(&c.body);
                        }
                        return;
                    }
                }
                self.scan_expr(disc);
                for c in cases {
                    for t in &c.tests {
                        self.scan_expr(t);
                    }
                    self.scan_stmts(&c.body);
                }
            }
            Stmt::ExprStmt(e) | Stmt::Const(_, e) | Stmt::Throw(e) => self.scan_expr(e),
            Stmt::Let(_, Some(e)) | Stmt::Return(Some(e)) => self.scan_expr(e),
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(e) = init {
                        self.scan_expr(e);
                    }
                }
            }
            Stmt::If(t, a, b) => {
                self.scan_expr(t);
                self.scan_stmts(a);
                if let Some(b) = b {
                    self.scan_stmts(b);
                }
            }
            Stmt::Labeled(_, inner) => self.scan_stmt(inner),
            Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::FunctionDecl { body: b, .. } => {
                self.scan_stmts(b);
            }
            Stmt::While(t, b) => {
                self.scan_expr(t);
                self.scan_stmts(b);
            }
            Stmt::TryCatch { body, handler, .. } => {
                self.scan_stmts(body);
                self.scan_stmts(handler);
            }
            Stmt::Let(_, None)
            | Stmt::Return(None)
            | Stmt::Continue(_)
            | Stmt::Break(_)
            | Stmt::Import { .. }
            | Stmt::Comment(_)
            | Stmt::Raw(_)
            | Stmt::Loc(_) => {}
        }
    }

    fn scan_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Str(s) => {
                self.bad.insert(s.clone());
            }
            Expr::Object(entries) => {
                for (k, val) in entries {
                    if k == "TAG" {
                        if let Expr::Str(s) = val {
                            self.candidates.insert(s.clone());
                            continue;
                        }
                    }
                    self.scan_expr(val);
                }
            }
            Expr::Assign(lhs, rhs) => {
                if let (Expr::Member(obj, k), Expr::Str(s)) = (lhs.as_ref(), rhs.as_ref()) {
                    if k == "TAG" {
                        self.candidates.insert(s.clone());
                        self.scan_expr(obj);
                        return;
                    }
                }
                self.scan_expr(lhs);
                self.scan_expr(rhs);
            }
            Expr::Binary(op @ (BinOp::StrictEq | BinOp::StrictNe), l, r) => {
                let _ = op;
                if let (Expr::Member(obj, k), Expr::Str(s)) = (l.as_ref(), r.as_ref()) {
                    if k == "TAG" {
                        self.candidates.insert(s.clone());
                        self.scan_expr(obj);
                        return;
                    }
                }
                if let (Expr::Str(s), Expr::Member(obj, k)) = (l.as_ref(), r.as_ref()) {
                    if k == "TAG" {
                        self.candidates.insert(s.clone());
                        self.scan_expr(obj);
                        return;
                    }
                }
                self.scan_expr(l);
                self.scan_expr(r);
            }
            Expr::Member(obj, _) => self.scan_expr(obj),
            Expr::Index(obj, idx) => {
                self.scan_expr(obj);
                self.scan_expr(idx);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                self.scan_expr(c);
                for a in args {
                    self.scan_expr(a);
                }
            }
            Expr::Unary(_, inner) => self.scan_expr(inner),
            Expr::Binary(_, l, r) | Expr::CompoundAssign(_, l, r) => {
                self.scan_expr(l);
                self.scan_expr(r);
            }
            Expr::Cond(t, c, a) => {
                self.scan_expr(t);
                self.scan_expr(c);
                self.scan_expr(a);
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    self.scan_expr(x);
                }
            }
            Expr::Arrow(_, ArrowBody::Expr(b)) => self.scan_expr(b),
            Expr::Arrow(_, ArrowBody::Block(stmts)) => self.scan_stmts(stmts),
            _ => {}
        }
    }
}

struct TagRewriter<'a> {
    tag_map: &'a BTreeMap<String, usize>,
    changed: bool,
}

impl TagRewriter<'_> {
    fn rewrite_str_if_mapped(&mut self, expr: &mut Expr) {
        if let Expr::Str(s) = expr {
            if let Some(&id) = self.tag_map.get(s) {
                *expr = Expr::Num(id as f64);
                self.changed = true;
            }
        }
    }

    fn rewrite_stmts(&mut self, stmts: &mut [Stmt]) {
        for s in stmts {
            self.rewrite_stmt(s);
        }
    }

    fn rewrite_stmt(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Switch(disc, cases) => {
                if let Expr::Member(obj, prop) = disc {
                    if prop == "TAG" {
                        self.rewrite_expr(obj);
                        for c in cases {
                            for t in &mut c.tests {
                                if matches!(t, Expr::Str(_)) {
                                    self.rewrite_str_if_mapped(t);
                                } else {
                                    self.rewrite_expr(t);
                                }
                            }
                            self.rewrite_stmts(&mut c.body);
                        }
                        return;
                    }
                }
                self.rewrite_expr(disc);
                for c in cases {
                    for t in &mut c.tests {
                        self.rewrite_expr(t);
                    }
                    self.rewrite_stmts(&mut c.body);
                }
            }
            Stmt::ExprStmt(e) | Stmt::Const(_, e) | Stmt::Throw(e) => self.rewrite_expr(e),
            Stmt::Let(_, Some(e)) | Stmt::Return(Some(e)) => self.rewrite_expr(e),
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(e) = init {
                        self.rewrite_expr(e);
                    }
                }
            }
            Stmt::If(t, a, b) => {
                self.rewrite_expr(t);
                self.rewrite_stmts(a);
                if let Some(b) = b {
                    self.rewrite_stmts(b);
                }
            }
            Stmt::Labeled(_, inner) => self.rewrite_stmt(inner),
            Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::FunctionDecl { body: b, .. } => {
                self.rewrite_stmts(b);
            }
            Stmt::While(t, b) => {
                self.rewrite_expr(t);
                self.rewrite_stmts(b);
            }
            Stmt::TryCatch { body, handler, .. } => {
                self.rewrite_stmts(body);
                self.rewrite_stmts(handler);
            }
            Stmt::Let(_, None)
            | Stmt::Return(None)
            | Stmt::Continue(_)
            | Stmt::Break(_)
            | Stmt::Import { .. }
            | Stmt::Comment(_)
            | Stmt::Raw(_)
            | Stmt::Loc(_) => {}
        }
    }

    fn rewrite_expr(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Object(entries) => {
                for (k, val) in entries {
                    if k == "TAG" && matches!(val, Expr::Str(_)) {
                        self.rewrite_str_if_mapped(val);
                    } else {
                        self.rewrite_expr(val);
                    }
                }
            }
            Expr::Assign(lhs, rhs) => {
                if let (Expr::Member(obj, k), Expr::Str(_)) = (&**lhs, &**rhs) {
                    if k == "TAG" {
                        let _ = obj;
                        self.rewrite_str_if_mapped(rhs);
                        if let Expr::Member(obj, _) = &mut **lhs {
                            self.rewrite_expr(obj);
                        }
                        return;
                    }
                }
                self.rewrite_expr(lhs);
                self.rewrite_expr(rhs);
            }
            Expr::Binary(BinOp::StrictEq | BinOp::StrictNe, l, r) => {
                if matches!(&**l, Expr::Member(_, k) if k == "TAG") && matches!(&**r, Expr::Str(_))
                {
                    self.rewrite_str_if_mapped(r);
                    if let Expr::Member(obj, _) = &mut **l {
                        self.rewrite_expr(obj);
                    }
                    return;
                }
                if matches!(&**r, Expr::Member(_, k) if k == "TAG") && matches!(&**l, Expr::Str(_))
                {
                    self.rewrite_str_if_mapped(l);
                    if let Expr::Member(obj, _) = &mut **r {
                        self.rewrite_expr(obj);
                    }
                    return;
                }
                self.rewrite_expr(l);
                self.rewrite_expr(r);
            }
            Expr::Member(obj, _) => self.rewrite_expr(obj),
            Expr::Index(obj, idx) => {
                self.rewrite_expr(obj);
                self.rewrite_expr(idx);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                self.rewrite_expr(c);
                for a in args {
                    self.rewrite_expr(a);
                }
            }
            Expr::Unary(_, inner) => self.rewrite_expr(inner),
            Expr::Binary(_, l, r) | Expr::CompoundAssign(_, l, r) => {
                self.rewrite_expr(l);
                self.rewrite_expr(r);
            }
            Expr::Cond(t, c, a) => {
                self.rewrite_expr(t);
                self.rewrite_expr(c);
                self.rewrite_expr(a);
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    self.rewrite_expr(x);
                }
            }
            Expr::Arrow(_, ArrowBody::Expr(b)) => self.rewrite_expr(b),
            Expr::Arrow(_, ArrowBody::Block(stmts)) => self.rewrite_stmts(stmts),
            _ => {}
        }
    }
}

fn encode_internal_tags(lowered: &mut [LoweredRow], exclusions: &ProgramExclusions) {
    let mut scanner = TagScanner {
        candidates: BTreeSet::new(),
        bad: BTreeSet::new(),
    };
    for s in ["Ok", "Err", "Some", "None", "Ready", "Pending", ""] {
        scanner.bad.insert(s.to_owned());
    }
    scanner.bad.extend(exclusions.raw_tokens.iter().cloned());
    scanner.bad.extend(exclusions.raw_strings.iter().cloned());
    scanner
        .bad
        .extend(exclusions.boundary_variants.iter().cloned());

    for (_, res, _, _) in lowered.iter() {
        if let Ok(item) = res {
            scanner.scan_stmt(&item.decl);
        }
    }

    let tag_map: BTreeMap<String, usize> = scanner
        .candidates
        .into_iter()
        .filter(|s| !scanner.bad.contains(s))
        .enumerate()
        .map(|(idx, s)| (s, idx))
        .collect();
    if tag_map.is_empty() {
        return;
    }

    for (item_name, res, _, _) in lowered.iter_mut() {
        let Ok(item) = res else { continue };
        let mut rewriter = TagRewriter {
            tag_map: &tag_map,
            changed: false,
        };
        rewriter.rewrite_stmt(&mut item.decl);
        if rewriter.changed {
            item.refs = super::lane::refs_of(&item.decl, item_name);
        }
    }
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

    #[test]
    fn hoists_capture_free_zero_factory_arrows_to_module_const() {
        let zero_factory = Expr::Arrow(
            Vec::new(),
            ArrowBody::Expr(Box::new(Expr::Object(vec![
                ("_0".to_owned(), Expr::Str(String::new())),
                ("_1".to_owned(), Expr::Num(0.0)),
            ]))),
        );
        let fn_stmt = jsast::function(
            "push",
            vec!["p".to_owned()],
            vec![Stmt::Return(Some(Expr::Call(
                Box::new(Expr::Member(
                    Box::new(Expr::Ident("__rt".to_owned())),
                    "unscale".to_owned(),
                )),
                vec![Expr::Ident("p".to_owned()), Expr::Num(4.0), zero_factory],
            )))],
        );
        let opts = Opts {
            optimizer: OptimizerMode::On,
            ..Opts::default()
        };
        let (table, lowered, _, _, added) = test_run(opts, vec![("push", fn_stmt.clone())]);
        assert!(added);
        assert_eq!(table.len(), 1);
        let (hc_name, hc_item) = table.iter().next().unwrap();
        assert!(
            hc_item.js.contains("() => ({ _0: \"\", _1: 0 })"),
            "expected hoisted zero factory, got:\n{}",
            hc_item.js
        );
        let Ok(item) = &lowered[0].1 else {
            unreachable!()
        };
        let rendered = jsast::stmt_to_string(&item.decl);
        assert!(rendered.contains(hc_name.as_str()));
        assert!(!rendered.contains("=>"));

        let report_opts = Opts {
            optimizer: OptimizerMode::Report,
            ..Opts::default()
        };
        let (report_table, _, _, _, report_added) = test_run(report_opts, vec![("push", fn_stmt)]);
        assert!(!report_added);
        assert!(report_table.is_empty());
    }

    #[test]
    fn splits_internal_bigint_struct_properties_including_materialized_temps() {
        let pack = |lo: Expr, hi: Expr| {
            Expr::Binary(
                BinOp::BitOr,
                Box::new(Expr::Binary(
                    BinOp::Shl,
                    Box::new(Expr::Call(
                        Box::new(Expr::Ident("BigInt".to_owned())),
                        vec![Expr::Binary(
                            BinOp::UShr,
                            Box::new(hi),
                            Box::new(Expr::Num(0.0)),
                        )],
                    )),
                    Box::new(Expr::BigInt("32".to_owned())),
                )),
                Box::new(Expr::Call(
                    Box::new(Expr::Ident("BigInt".to_owned())),
                    vec![Expr::Binary(
                        BinOp::UShr,
                        Box::new(lo),
                        Box::new(Expr::Num(0.0)),
                    )],
                )),
            )
        };
        let unpack_lo = |src: Expr| {
            Expr::Binary(
                BinOp::BitOr,
                Box::new(Expr::Call(
                    Box::new(Expr::Ident("Number".to_owned())),
                    vec![Expr::Call(
                        Box::new(Expr::Member(
                            Box::new(Expr::Ident("BigInt".to_owned())),
                            "asIntN".to_owned(),
                        )),
                        vec![Expr::Num(32.0), src],
                    )],
                )),
                Box::new(Expr::Num(0.0)),
            )
        };
        let unpack_hi = |src: Expr| {
            unpack_lo(Expr::Binary(
                BinOp::Shr,
                Box::new(src),
                Box::new(Expr::BigInt("32".to_owned())),
            ))
        };

        let make_fn = jsast::function(
            "make_searcher",
            vec!["lo".to_owned(), "hi".to_owned()],
            vec![Stmt::Return(Some(Expr::Object(vec![
                ("needle".to_owned(), Expr::Str("ab".to_owned())),
                (
                    "byteset".to_owned(),
                    pack(Expr::Ident("lo".to_owned()), Expr::Ident("hi".to_owned())),
                ),
                ("raw_prop".to_owned(), Expr::BigInt("7".to_owned())),
                ("escaped".to_owned(), Expr::BigInt("9".to_owned())),
            ])))],
        );
        let read_fn = jsast::function(
            "read_searcher",
            vec!["s".to_owned()],
            vec![
                Stmt::Lets(vec![("$t0".to_owned(), None)]),
                Stmt::ExprStmt(Expr::Assign(
                    Box::new(Expr::Ident("$t0".to_owned())),
                    Box::new(Expr::Member(
                        Box::new(Expr::Ident("s".to_owned())),
                        "byteset".to_owned(),
                    )),
                )),
                Stmt::Return(Some(Expr::Binary(
                    BinOp::BitXor,
                    Box::new(unpack_lo(Expr::Ident("$t0".to_owned()))),
                    Box::new(unpack_hi(Expr::Ident("$t0".to_owned()))),
                ))),
            ],
        );
        let escape_fn = jsast::function(
            "escape_other",
            vec!["s".to_owned()],
            vec![Stmt::Return(Some(Expr::Member(
                Box::new(Expr::Ident("s".to_owned())),
                "escaped".to_owned(),
            )))],
        );

        let mut table = BTreeMap::new();
        let mut defined = BTreeSet::new();
        let mut live = BTreeSet::new();
        let mut lowered = Vec::new();
        for (name, decl) in [
            ("make_searcher", make_fn),
            ("read_searcher", read_fn),
            ("escape_other", escape_fn),
        ] {
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
        let raw_name = JsName::new("raw_reader".to_owned());
        defined.insert(raw_name.clone());
        live.insert(raw_name.clone());
        table.insert(
            raw_name.clone(),
            LinkItem {
                name: raw_name,
                kind: ItemKind::Fn,
                js: "function raw_reader(x) { return x.raw_prop; }".to_owned(),
                refs: BTreeSet::new(),
                zombies: Vec::new(),
                linkage: Linkage::internal(),
                debug_path: String::new(),
                crate_name: String::new(),
                order: None,
                map: item_map(Vec::new()),
                dts: None,
                optimization: Default::default(),
                late_ir: Default::default(),
                baseline_shadow: Default::default(),
                jir: Default::default(),
            },
        );

        let opts = Opts {
            optimizer: OptimizerMode::On,
            ecma_target: EcmaTarget::Es2018,
            ..Opts::default()
        };
        let _ = run_with_opts(&mut table, &mut lowered, &mut defined, &mut live, &opts);

        let make_js = jsast::stmt_to_string(&lowered[0].1.as_ref().unwrap().decl);
        let read_js = jsast::stmt_to_string(&lowered[1].1.as_ref().unwrap().decl);
        assert!(make_js.contains("byteset$lo: lo | 0"), "{make_js}");
        assert!(make_js.contains("byteset$hi: hi | 0"), "{make_js}");
        assert!(!make_js.contains("byteset:"), "{make_js}");
        assert!(make_js.contains("raw_prop:"), "{make_js}");
        assert!(make_js.contains("escaped:"), "{make_js}");
        assert!(read_js.contains("$t0$lo"), "{read_js}");
        assert!(read_js.contains("$t0$hi"), "{read_js}");
        assert!(!read_js.contains("BigInt"), "{read_js}");
        assert!(!read_js.contains(".byteset\n"), "{read_js}");
    }

    #[test]
    fn encodes_internal_enum_tags_as_dense_ints_while_respecting_exclusions() {
        let fn_a = jsast::function(
            "use_internal",
            vec!["s".to_owned()],
            vec![
                Stmt::Let(
                    "v".to_owned(),
                    Some(Expr::Object(vec![(
                        "TAG".to_owned(),
                        Expr::Str("Alpha".to_owned()),
                    )])),
                ),
                Stmt::ExprStmt(Expr::Assign(
                    Box::new(Expr::Member(
                        Box::new(Expr::Ident("v".to_owned())),
                        "TAG".to_owned(),
                    )),
                    Box::new(Expr::Str("Beta".to_owned())),
                )),
                Stmt::ExprStmt(Expr::Binary(
                    BinOp::StrictEq,
                    Box::new(Expr::Member(
                        Box::new(Expr::Ident("s".to_owned())),
                        "TAG".to_owned(),
                    )),
                    Box::new(Expr::Str("Some".to_owned())),
                )),
                Stmt::Switch(
                    Expr::Member(Box::new(Expr::Ident("s".to_owned())), "TAG".to_owned()),
                    vec![
                        SwitchCase {
                            tests: vec![Expr::Str("Alpha".to_owned())],
                            body: vec![Stmt::Return(Some(Expr::Num(10.0)))],
                        },
                        SwitchCase {
                            tests: vec![Expr::Str("Leaked".to_owned())],
                            body: vec![Stmt::Return(Some(Expr::Str("Leaked".to_owned())))],
                        },
                        SwitchCase {
                            tests: vec![Expr::Str("FromRaw".to_owned())],
                            body: vec![Stmt::Return(Some(Expr::Num(30.0)))],
                        },
                    ],
                ),
            ],
        );

        let mut table = BTreeMap::new();
        let mut defined = BTreeSet::new();
        let mut live = BTreeSet::new();
        let js_name = JsName::new("use_internal".to_owned());
        let refs = super::super::lane::refs_of(&fn_a, &js_name);
        defined.insert(js_name.clone());
        live.insert(js_name.clone());
        let mut lowered = vec![(
            js_name,
            Ok(LoweredItem {
                decl: fn_a.clone(),
                refs,
                zombies: Vec::new(),
                loc: None,
                names: Vec::new(),
                dts: None,
                js_alloc_origins: Vec::new(),
            }),
            OptStats::default(),
            None,
        )];
        let raw_name = JsName::new("raw_fn".to_owned());
        defined.insert(raw_name.clone());
        live.insert(raw_name.clone());
        table.insert(
            raw_name.clone(),
            LinkItem {
                name: raw_name,
                kind: ItemKind::Fn,
                js: "function raw_fn(x) { return x.TAG === \"FromRaw\"; }".to_owned(),
                refs: BTreeSet::new(),
                zombies: Vec::new(),
                linkage: Linkage::internal(),
                debug_path: String::new(),
                crate_name: String::new(),
                order: None,
                map: item_map(Vec::new()),
                dts: None,
                optimization: Default::default(),
                late_ir: Default::default(),
                baseline_shadow: Default::default(),
                jir: Default::default(),
            },
        );

        let opts = Opts {
            optimizer: OptimizerMode::On,
            ..Opts::default()
        };
        let _ = run_with_opts(&mut table, &mut lowered, &mut defined, &mut live, &opts);
        let rendered = jsast::stmt_to_string(&lowered[0].1.as_ref().unwrap().decl);
        assert!(rendered.contains("{ TAG: 0 }"), "{rendered}");
        assert!(rendered.contains("v.TAG = 1;"), "{rendered}");
        assert!(rendered.contains("case 0:"), "{rendered}");
        assert!(rendered.contains("s.TAG === \"Some\""), "{rendered}");
        assert!(rendered.contains("case \"Leaked\":"), "{rendered}");
        assert!(rendered.contains("case \"FromRaw\":"), "{rendered}");

        let report_opts = Opts {
            optimizer: OptimizerMode::Report,
            ..Opts::default()
        };
        let (_, report_lowered, _, _, _) = test_run(report_opts, vec![("use_internal", fn_a)]);
        let report_js = jsast::stmt_to_string(&report_lowered[0].1.as_ref().unwrap().decl);
        assert!(report_js.contains("\"Alpha\""), "{report_js}");
        assert!(report_js.contains("\"Beta\""), "{report_js}");
    }
}
