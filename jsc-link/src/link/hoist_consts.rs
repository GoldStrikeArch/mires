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
use crate::jir::semantic::{CallSemantic, Callee, HostPayload, Terminator};
use crate::jir::types::{
    FieldKey, NominalAbi, NominalForm, PinnedBoundary, TypeId, TypeKind, TypeTable,
};
use crate::jir::{DefinitionId, Interns, ItemId};
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

fn is_pure_scalar_lit(expr: &Expr, allow_bigint: bool) -> bool {
    match expr {
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null => true,
        Expr::BigInt(_) => allow_bigint,
        Expr::Unary(op, inner) => {
            matches!(
                op,
                jsast::UnOp::Neg | jsast::UnOp::Pos | jsast::UnOp::Not | jsast::UnOp::BitNot
            ) && is_pure_scalar_lit(inner, allow_bigint)
        }
        _ => false,
    }
}

fn is_pure_factory_expr(expr: &Expr, allow_bigint: bool) -> bool {
    match expr {
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null => true,
        Expr::BigInt(_) => allow_bigint,
        Expr::Unary(op, inner) => {
            matches!(op, UnOp::Neg | UnOp::Pos | UnOp::Not | UnOp::BitNot)
                && is_pure_factory_expr(inner, allow_bigint)
        }
        Expr::Call(callee, args) => {
            allow_bigint
                && matches!(&**callee, Expr::Ident(n) if n == "BigInt")
                && args.len() == 1
                && matches!(&args[0], Expr::Str(_) | Expr::Num(_))
        }
        Expr::Array(items) => {
            items.len() <= 32
                && items
                    .iter()
                    .all(|item| is_pure_factory_expr(item, allow_bigint))
        }
        Expr::Object(entries) => {
            entries.len() <= 32
                && entries
                    .iter()
                    .all(|(_, v)| is_pure_factory_expr(v, allow_bigint))
        }
        _ => false,
    }
}

fn is_scalar_array_lit(expr: &Expr, min_len: usize, allow_bigint: bool) -> bool {
    let Expr::Array(elems) = expr else {
        return false;
    };
    elems.len() >= min_len
        && elems
            .iter()
            .all(|elem| is_pure_scalar_lit(elem, allow_bigint))
}

fn is_flat_scalar_aggregate(expr: &Expr, allow_bigint: bool) -> bool {
    match expr {
        Expr::Array(elems) => {
            !elems.is_empty()
                && elems.len() <= 16
                && elems
                    .iter()
                    .all(|elem| is_pure_scalar_lit(elem, allow_bigint))
        }
        Expr::Object(entries) => {
            !entries.is_empty()
                && entries.len() <= 16
                && entries
                    .iter()
                    .all(|(_, v)| is_pure_scalar_lit(v, allow_bigint))
        }
        _ => false,
    }
}

/// Matches a constant slice literal `{ buf: [...], off: 0, len: N }` (`N >= 4`) whose backing
/// array holds either pure scalars or flat pure-scalar tuples/objects (e.g. Keccak's `IOTA`).
fn is_pure_const_slice_lit(expr: &Expr, allow_bigint: bool) -> Option<bool> {
    let Expr::Object(entries) = expr else {
        return None;
    };
    if entries.len() != 3 {
        return None;
    }
    let ((k0, buf), (k1, off), (k2, len)) = (&entries[0], &entries[1], &entries[2]);
    if k0 != "buf" || k1 != "off" || k2 != "len" {
        return None;
    }
    if !matches!(off, Expr::Num(_) | Expr::RawNum(_))
        || !matches!(len, Expr::Num(_) | Expr::RawNum(_))
    {
        return None;
    }
    let Expr::Array(elems) = buf else {
        return None;
    };
    if elems.len() < 4 {
        return None;
    }
    if elems
        .iter()
        .all(|elem| is_pure_scalar_lit(elem, allow_bigint))
    {
        Some(false)
    } else if elems
        .iter()
        .all(|elem| is_flat_scalar_aggregate(elem, allow_bigint))
    {
        Some(true)
    } else {
        None
    }
}

fn is_location_filename_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Str(_) => true,
        Expr::Call(callee, args) if args.len() == 1 && matches!(&args[0], Expr::Str(_)) => {
            matches!(
                callee.as_ref(),
                Expr::Member(ns, method)
                    if matches!(ns.as_ref(), Expr::Ident(id) if id == "__rt")
                        && matches!(method.as_str(), "str_bytes" | "bstr_bytes")
            )
        }
        _ => false,
    }
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
            (k0, file_expr),
            (k1, Expr::Num(_) | Expr::RawNum(_)),
            (k2, Expr::Num(_) | Expr::RawNum(_)),
        ) if k0 == "filename" && k1 == "line" && k2 == "col" && is_location_filename_expr(file_expr)
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
    allow_bigint: bool,
    bstr_strings: bool,
    counts: BTreeMap<String, (Expr, ConstUsage)>,
    unreachable_throws: usize,
}

fn is_hoistable_bstr(s: &str) -> bool {
    !s.is_ascii() && s.chars().count() >= crate::lower::js::constant::HOIST_STRING_BYTES
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
            if is_location_object(expr) {
                let key = format!("loc:{expr}");
                self.record(key, expr, in_loop);
                return;
            }
            if let Expr::Index(obj, idx) = expr {
                if is_scalar_array_lit(obj, 4, self.allow_bigint) {
                    let key = format!("arr:{obj}");
                    self.record(key, obj, in_loop);
                    self.visit_expr(idx, in_loop, false);
                    return;
                }
            }
            if let Expr::Arrow(params, ArrowBody::Expr(inner)) = expr {
                if params.is_empty() && is_pure_factory_expr(inner, self.allow_bigint) {
                    let key = format!("fn:{expr}");
                    self.record(key, expr, in_loop);
                    return;
                }
            }
            if self.bstr_strings {
                if let Expr::Str(s) = expr {
                    if is_hoistable_bstr(s) {
                        self.record(format!("str:{s}"), expr, in_loop);
                        return;
                    }
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
fn is_nested_scalar_array_lit(expr: &Expr, min_len: usize, allow_bigint: bool) -> bool {
    let Expr::Array(elems) = expr else {
        return false;
    };
    elems.len() >= min_len
        && elems
            .iter()
            .all(|elem| is_flat_scalar_aggregate(elem, allow_bigint))
}

/// Checks whether `local` across the entire `body` is declared once, initialized once (including
/// inside a comma `Seq`) to a read-only array of flat pure-scalar tuples/objects (`len >= 8`,
/// e.g. Keccak's `IOTA`), and every indexed element `local[i]` is only ever read as scalar fields
/// or indices (directly or through read-only element aliases).
fn find_read_only_nested_array_init(
    body: &[Stmt],
    local: &str,
    allow_bigint: bool,
) -> Option<Expr> {
    fn collect_elem_aliases(
        stmts: &[Stmt],
        local: &str,
        aliases: &mut BTreeSet<String>,
        multi_or_bad: &mut BTreeSet<String>,
    ) {
        fn scan_e(
            e: &Expr,
            local: &str,
            aliases: &mut BTreeSet<String>,
            multi_or_bad: &mut BTreeSet<String>,
        ) {
            match e {
                Expr::Assign(lhs, rhs) => {
                    if let Expr::Ident(name) = lhs.as_ref() {
                        if let Expr::Index(obj, _) = rhs.as_ref() {
                            if matches!(obj.as_ref(), Expr::Ident(o) if o == local) {
                                if !aliases.insert(name.clone()) {
                                    // Re-assigned in a loop (`E = am[an + l]`): still a valid
                                    // element alias if every assignment is `local[...]`.
                                }
                                scan_e(rhs, local, aliases, multi_or_bad);
                                return;
                            }
                        }
                        multi_or_bad.insert(name.clone());
                    }
                    scan_e(lhs, local, aliases, multi_or_bad);
                    scan_e(rhs, local, aliases, multi_or_bad);
                }
                Expr::CompoundAssign(_, l, r) | Expr::Binary(_, l, r) | Expr::Index(l, r) => {
                    scan_e(l, local, aliases, multi_or_bad);
                    scan_e(r, local, aliases, multi_or_bad);
                }
                Expr::Member(o, _) | Expr::Unary(_, o) => scan_e(o, local, aliases, multi_or_bad),
                Expr::Call(c, args) | Expr::New(c, args) => {
                    scan_e(c, local, aliases, multi_or_bad);
                    args.iter()
                        .for_each(|a| scan_e(a, local, aliases, multi_or_bad));
                }
                Expr::Cond(c, t, f) => {
                    scan_e(c, local, aliases, multi_or_bad);
                    scan_e(t, local, aliases, multi_or_bad);
                    scan_e(f, local, aliases, multi_or_bad);
                }
                Expr::Object(entries) => {
                    entries
                        .iter()
                        .for_each(|(_, v)| scan_e(v, local, aliases, multi_or_bad));
                }
                Expr::Array(items) | Expr::Seq(items) => {
                    items
                        .iter()
                        .for_each(|x| scan_e(x, local, aliases, multi_or_bad));
                }
                Expr::Arrow(_, ArrowBody::Expr(b)) => scan_e(b, local, aliases, multi_or_bad),
                Expr::Arrow(_, ArrowBody::Block(s)) => {
                    collect_elem_aliases(s, local, aliases, multi_or_bad);
                }
                _ => {}
            }
        }
        for s in stmts {
            match s {
                Stmt::Let(n, Some(init)) | Stmt::Const(n, init) => {
                    if let Expr::Index(obj, _) = init {
                        if matches!(obj.as_ref(), Expr::Ident(o) if o == local) {
                            aliases.insert(n.clone());
                            scan_e(init, local, aliases, multi_or_bad);
                            continue;
                        }
                    }
                    multi_or_bad.insert(n.clone());
                    scan_e(init, local, aliases, multi_or_bad);
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(init) = init {
                            multi_or_bad.insert(n.clone());
                            scan_e(init, local, aliases, multi_or_bad);
                        }
                    }
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                    scan_e(e, local, aliases, multi_or_bad);
                }
                Stmt::If(t, a, b) => {
                    scan_e(t, local, aliases, multi_or_bad);
                    collect_elem_aliases(a, local, aliases, multi_or_bad);
                    if let Some(b) = b {
                        collect_elem_aliases(b, local, aliases, multi_or_bad);
                    }
                }
                Stmt::Switch(d, cases) => {
                    scan_e(d, local, aliases, multi_or_bad);
                    for c in cases {
                        c.tests
                            .iter()
                            .for_each(|t| scan_e(t, local, aliases, multi_or_bad));
                        collect_elem_aliases(&c.body, local, aliases, multi_or_bad);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    collect_elem_aliases(
                        std::slice::from_ref(inner.as_ref()),
                        local,
                        aliases,
                        multi_or_bad,
                    );
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::FunctionDecl { body: b, .. } => {
                    collect_elem_aliases(b, local, aliases, multi_or_bad);
                }
                Stmt::While(t, b) => {
                    scan_e(t, local, aliases, multi_or_bad);
                    collect_elem_aliases(b, local, aliases, multi_or_bad);
                }
                Stmt::TryCatch {
                    body,
                    binding,
                    handler,
                } => {
                    multi_or_bad.insert(binding.clone());
                    collect_elem_aliases(body, local, aliases, multi_or_bad);
                    collect_elem_aliases(handler, local, aliases, multi_or_bad);
                }
                _ => {}
            }
        }
    }

    let mut aliases = BTreeSet::new();
    let mut bad_aliases = BTreeSet::new();
    collect_elem_aliases(body, local, &mut aliases, &mut bad_aliases);
    for b in &bad_aliases {
        aliases.remove(b);
    }
    aliases.remove(local);

    struct Check<'a> {
        local: &'a str,
        aliases: &'a BTreeSet<String>,
        allow_bigint: bool,
        decl_count: usize,
        found_init: Option<Expr>,
        reads: usize,
        bad: bool,
    }
    impl Check<'_> {
        fn check_assign(&mut self, lhs: &Expr, rhs: &Expr) {
            if let Expr::Ident(n) = lhs {
                if n == self.local {
                    if !is_nested_scalar_array_lit(rhs, 8, self.allow_bigint) {
                        self.bad = true;
                        return;
                    }
                    if let Some(prev) = &self.found_init {
                        if prev != rhs {
                            self.bad = true;
                        }
                        return;
                    }
                    self.found_init = Some(rhs.clone());
                    return;
                }
                if self.aliases.contains(n) {
                    if let Expr::Index(obj, idx) = rhs {
                        if matches!(obj.as_ref(), Expr::Ident(o) if o == self.local) {
                            self.reads += 1;
                            self.expr(idx, false);
                            return;
                        }
                    }
                    self.bad = true;
                    return;
                }
            }
            self.expr(lhs, true);
            self.expr(rhs, false);
        }

        fn stmt(&mut self, s: &Stmt) {
            if self.bad {
                return;
            }
            match s {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => self.check_assign(lhs, rhs),
                Stmt::ExprStmt(Expr::Seq(items)) => {
                    for item in items {
                        if let Expr::Assign(lhs, rhs) = item {
                            self.check_assign(lhs, rhs);
                        } else {
                            self.expr(item, false);
                        }
                    }
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => self.expr(e, false),
                Stmt::Let(n, init) => {
                    if n == self.local {
                        self.decl_count += 1;
                        if self.decl_count > 1 {
                            self.bad = true;
                            return;
                        }
                        if let Some(rhs) = init {
                            if self.found_init.is_some()
                                || !is_nested_scalar_array_lit(rhs, 8, self.allow_bigint)
                            {
                                self.bad = true;
                                return;
                            }
                            self.found_init = Some(rhs.clone());
                        }
                        return;
                    }
                    if let Some(e) = init {
                        if self.aliases.contains(n) {
                            if let Expr::Index(obj, idx) = e {
                                if matches!(obj.as_ref(), Expr::Ident(o) if o == self.local) {
                                    self.reads += 1;
                                    self.expr(idx, false);
                                    return;
                                }
                            }
                            self.bad = true;
                            return;
                        }
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
                                if self.found_init.is_some()
                                    || !is_nested_scalar_array_lit(rhs, 8, self.allow_bigint)
                                {
                                    self.bad = true;
                                    return;
                                }
                                self.found_init = Some(rhs.clone());
                            }
                            continue;
                        }
                        if let Some(e) = init {
                            if self.aliases.contains(n) {
                                self.bad = true;
                                return;
                            }
                            self.expr(e, false);
                        }
                    }
                }
                Stmt::Const(n, e) => {
                    if n == self.local || self.aliases.contains(n) {
                        self.bad = true;
                        return;
                    }
                    self.expr(e, false);
                }
                Stmt::If(t, a, b) => {
                    self.expr(t, false);
                    a.iter().for_each(|x| self.stmt(x));
                    if let Some(b) = b {
                        b.iter().for_each(|x| self.stmt(x));
                    }
                }
                Stmt::Switch(d, cases) => {
                    self.expr(d, false);
                    for c in cases {
                        c.tests.iter().for_each(|t| self.expr(t, false));
                        c.body.iter().for_each(|x| self.stmt(x));
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
                    if binding == self.local || self.aliases.contains(binding) {
                        self.bad = true;
                        return;
                    }
                    body.iter().for_each(|x| self.stmt(x));
                    handler.iter().for_each(|x| self.stmt(x));
                }
                Stmt::FunctionDecl { name, params, body } => {
                    if name == self.local
                        || self.aliases.contains(name)
                        || params
                            .iter()
                            .any(|p| p == self.local || self.aliases.contains(p))
                    {
                        self.bad = true;
                        return;
                    }
                    body.iter().for_each(|x| self.stmt(x));
                }
                Stmt::Raw(_) => self.bad = true,
                _ => {}
            }
        }

        fn expr(&mut self, e: &Expr, is_lhs: bool) {
            if self.bad {
                return;
            }
            match e {
                Expr::Ident(n) => {
                    if n == self.local || self.aliases.contains(n) {
                        self.bad = true;
                    }
                }
                Expr::Index(obj, idx) => {
                    if let Expr::Ident(n) = obj.as_ref() {
                        if self.aliases.contains(n) {
                            if is_lhs {
                                self.bad = true;
                                return;
                            }
                            self.reads += 1;
                            self.expr(idx, false);
                            return;
                        }
                    }
                    if let Expr::Index(inner_obj, inner_idx) = obj.as_ref() {
                        if matches!(inner_obj.as_ref(), Expr::Ident(o) if o == self.local) {
                            if is_lhs {
                                self.bad = true;
                                return;
                            }
                            self.reads += 1;
                            self.expr(inner_idx, false);
                            self.expr(idx, false);
                            return;
                        }
                    }
                    self.expr(obj, false);
                    self.expr(idx, false);
                }
                Expr::Member(obj, _) => {
                    if let Expr::Ident(n) = obj.as_ref() {
                        if self.aliases.contains(n) {
                            if is_lhs {
                                self.bad = true;
                                return;
                            }
                            self.reads += 1;
                            return;
                        }
                    }
                    if let Expr::Index(inner_obj, inner_idx) = obj.as_ref() {
                        if matches!(inner_obj.as_ref(), Expr::Ident(o) if o == self.local) {
                            if is_lhs {
                                self.bad = true;
                                return;
                            }
                            self.reads += 1;
                            self.expr(inner_idx, false);
                            return;
                        }
                    }
                    self.expr(obj, false);
                }
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
                Expr::Cond(c, t, f) => {
                    self.expr(c, false);
                    self.expr(t, false);
                    self.expr(f, false);
                }
                Expr::Array(items) | Expr::Seq(items) => {
                    items.iter().for_each(|x| self.expr(x, false));
                }
                Expr::Object(entries) => {
                    entries.iter().for_each(|(_, v)| self.expr(v, false));
                }
                Expr::Arrow(params, ArrowBody::Expr(b)) => {
                    if params
                        .iter()
                        .any(|p| p == self.local || self.aliases.contains(p))
                    {
                        self.bad = true;
                        return;
                    }
                    self.expr(b, false);
                }
                Expr::Arrow(params, ArrowBody::Block(s)) => {
                    if params
                        .iter()
                        .any(|p| p == self.local || self.aliases.contains(p))
                    {
                        self.bad = true;
                        return;
                    }
                    s.iter().for_each(|x| self.stmt(x));
                }
                _ => {}
            }
        }
    }

    let mut check = Check {
        local,
        aliases: &aliases,
        allow_bigint,
        decl_count: 0,
        found_init: None,
        reads: 0,
        bad: false,
    };
    body.iter().for_each(|s| check.stmt(s));
    if !check.bad && check.decl_count == 1 && check.reads > 0 {
        check.found_init
    } else {
        None
    }
}

fn find_read_only_table_init(body: &[Stmt], local: &str, allow_bigint: bool) -> Option<Expr> {
    struct Check<'a> {
        local: &'a str,
        allow_bigint: bool,
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
                    if !is_scalar_array_lit(rhs, 8, self.allow_bigint) {
                        self.bad = true;
                        return;
                    }
                    if let Some(prev) = &self.found_init {
                        if prev != rhs.as_ref() {
                            self.bad = true;
                        }
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
                            if self.found_init.is_some()
                                || !is_scalar_array_lit(rhs, 8, self.allow_bigint)
                            {
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
                                if self.found_init.is_some()
                                    || !is_scalar_array_lit(rhs, 8, self.allow_bigint)
                                {
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
        allow_bigint,
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

/// Checks whether `local` is initialized once to a pure constant slice literal
/// `{ buf: [...], off: 0, len: N }` and every alias derived from it (via slice copies,
/// `{ buf: alias.buf, off: ... }` cursor steps, `__rt.ptr_eq` comparisons, and element reads)
/// is strictly read-only and never escapes or mutates the buffer or its elements.
fn find_read_only_const_slice_init(body: &[Stmt], local: &str, allow_bigint: bool) -> Option<Expr> {
    fn is_ptr_eq_callee(expr: &Expr) -> bool {
        match expr {
            Expr::Member(ns, method) => {
                matches!(ns.as_ref(), Expr::Ident(id) if id == "__rt") && method == "ptr_eq"
            }
            Expr::Ident(id) => id == "$rt_ptr_eq",
            _ => false,
        }
    }

    fn is_derived_slice_expr(expr: &Expr, slice_aliases: &BTreeSet<String>) -> bool {
        match expr {
            Expr::Ident(name) => slice_aliases.contains(name),
            Expr::Object(entries) if entries.len() == 2 || entries.len() == 3 => {
                let has_buf = entries.iter().any(|(k, v)| {
                    k == "buf"
                        && matches!(
                            v,
                            Expr::Member(base, prop)
                                if prop == "buf"
                                    && matches!(base.as_ref(), Expr::Ident(n) if slice_aliases.contains(n))
                        )
                });
                let keys_ok = entries
                    .iter()
                    .all(|(k, _)| matches!(k.as_str(), "buf" | "off" | "len"));
                has_buf && keys_ok
            }
            _ => false,
        }
    }

    fn is_elem_read_expr(expr: &Expr, slice_aliases: &BTreeSet<String>) -> bool {
        matches!(
            expr,
            Expr::Index(obj, _)
                if matches!(
                    obj.as_ref(),
                    Expr::Member(base, prop)
                        if prop == "buf"
                            && matches!(base.as_ref(), Expr::Ident(n) if slice_aliases.contains(n))
                )
        )
    }

    // Pass A: Find the single init of `local` and verify `local` is declared once.
    struct FindInit<'a> {
        local: &'a str,
        allow_bigint: bool,
        decl_count: usize,
        found_init: Option<(Expr, bool)>,
        bad: bool,
    }
    impl FindInit<'_> {
        fn stmt(&mut self, s: &Stmt) {
            if self.bad {
                return;
            }
            match s {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) if matches!(lhs.as_ref(), Expr::Ident(n) if n == self.local) => {
                    if let Some(nested) = is_pure_const_slice_lit(rhs, self.allow_bigint) {
                        if let Some((prev, prev_nested)) = &self.found_init {
                            if prev != rhs.as_ref() || *prev_nested != nested {
                                self.bad = true;
                            }
                            return;
                        }
                        self.found_init = Some(((**rhs).clone(), nested));
                    } else {
                        self.bad = true;
                    }
                }
                Stmt::Let(n, init) if n == self.local => {
                    self.decl_count += 1;
                    if self.decl_count > 1 {
                        self.bad = true;
                        return;
                    }
                    if let Some(rhs) = init {
                        if self.found_init.is_some() {
                            self.bad = true;
                            return;
                        }
                        if let Some(nested) = is_pure_const_slice_lit(rhs, self.allow_bigint) {
                            self.found_init = Some((rhs.clone(), nested));
                        } else {
                            self.bad = true;
                        }
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
                                if self.found_init.is_some() {
                                    self.bad = true;
                                    return;
                                }
                                if let Some(nested) =
                                    is_pure_const_slice_lit(rhs, self.allow_bigint)
                                {
                                    self.found_init = Some((rhs.clone(), nested));
                                } else {
                                    self.bad = true;
                                }
                            }
                        }
                    }
                }
                Stmt::Const(n, _) if n == self.local => self.bad = true,
                Stmt::If(_, a, b) => {
                    a.iter().for_each(|x| self.stmt(x));
                    if let Some(b) = b {
                        b.iter().for_each(|x| self.stmt(x));
                    }
                }
                Stmt::Switch(_, cases) => {
                    for c in cases {
                        c.body.iter().for_each(|x| self.stmt(x));
                    }
                }
                Stmt::Labeled(_, inner) => self.stmt(inner),
                Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::While(_, b) => {
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
                Stmt::Raw(_) => self.bad = true,
                _ => {}
            }
        }
    }
    let mut fi = FindInit {
        local,
        allow_bigint,
        decl_count: 0,
        found_init: None,
        bad: false,
    };
    body.iter().for_each(|s| fi.stmt(s));
    if fi.bad || fi.decl_count != 1 {
        return None;
    }
    let (init_expr, nested_elems) = fi.found_init?;

    // Pass B: Collect slice aliases and (for nested elements) element aliases to a fixpoint.
    let mut slice_aliases = BTreeSet::from([local.to_owned()]);
    let mut elem_aliases = BTreeSet::new();
    fn collect_aliases(
        stmts: &[Stmt],
        slice_aliases: &mut BTreeSet<String>,
        elem_aliases: &mut BTreeSet<String>,
        nested_elems: bool,
        changed: &mut bool,
    ) {
        for s in stmts {
            match s {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                    if let Expr::Ident(dst) = lhs.as_ref() {
                        if (is_derived_slice_expr(rhs, slice_aliases)
                            && slice_aliases.insert(dst.clone()))
                            || (nested_elems
                                && is_elem_read_expr(rhs, slice_aliases)
                                && elem_aliases.insert(dst.clone()))
                        {
                            *changed = true;
                        }
                    }
                }
                Stmt::Let(dst, Some(rhs)) => {
                    if (is_derived_slice_expr(rhs, slice_aliases)
                        && slice_aliases.insert(dst.clone()))
                        || (nested_elems
                            && is_elem_read_expr(rhs, slice_aliases)
                            && elem_aliases.insert(dst.clone()))
                    {
                        *changed = true;
                    }
                }
                Stmt::Lets(decls) => {
                    for (dst, init) in decls {
                        if let Some(rhs) = init {
                            if (is_derived_slice_expr(rhs, slice_aliases)
                                && slice_aliases.insert(dst.clone()))
                                || (nested_elems
                                    && is_elem_read_expr(rhs, slice_aliases)
                                    && elem_aliases.insert(dst.clone()))
                            {
                                *changed = true;
                            }
                        }
                    }
                }
                Stmt::If(_, a, b) => {
                    collect_aliases(a, slice_aliases, elem_aliases, nested_elems, changed);
                    if let Some(b) = b {
                        collect_aliases(b, slice_aliases, elem_aliases, nested_elems, changed);
                    }
                }
                Stmt::Switch(_, cases) => {
                    for c in cases {
                        collect_aliases(
                            &c.body,
                            slice_aliases,
                            elem_aliases,
                            nested_elems,
                            changed,
                        );
                    }
                }
                Stmt::Labeled(_, inner) => collect_aliases(
                    std::slice::from_ref(inner.as_ref()),
                    slice_aliases,
                    elem_aliases,
                    nested_elems,
                    changed,
                ),
                Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::While(_, b) => {
                    collect_aliases(b, slice_aliases, elem_aliases, nested_elems, changed);
                }
                Stmt::TryCatch { body, handler, .. } => {
                    collect_aliases(body, slice_aliases, elem_aliases, nested_elems, changed);
                    collect_aliases(handler, slice_aliases, elem_aliases, nested_elems, changed);
                }
                _ => {}
            }
        }
    }
    loop {
        let mut changed = false;
        collect_aliases(
            body,
            &mut slice_aliases,
            &mut elem_aliases,
            nested_elems,
            &mut changed,
        );
        if !changed {
            break;
        }
    }

    // Pass C: Verify every use of every slice alias and element alias is strictly read-only.
    struct Verify<'a> {
        local: &'a str,
        slice_aliases: &'a BTreeSet<String>,
        elem_aliases: &'a BTreeSet<String>,
        nested_elems: bool,
        reads: usize,
        bad: bool,
    }
    impl Verify<'_> {
        fn stmt(&mut self, s: &Stmt) {
            if self.bad {
                return;
            }
            match s {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                    if let Expr::Ident(dst) = lhs.as_ref() {
                        if dst == self.local {
                            // Single init of `local` already checked in Pass A.
                            return;
                        }
                        if self.slice_aliases.contains(dst) {
                            if is_derived_slice_expr(rhs, self.slice_aliases) {
                                self.check_derived_slice(rhs);
                                return;
                            }
                            self.bad = true;
                            return;
                        }
                        if self.elem_aliases.contains(dst) {
                            if is_elem_read_expr(rhs, self.slice_aliases) {
                                if let Expr::Index(_, idx) = rhs.as_ref() {
                                    self.reads += 1;
                                    self.expr(idx, false);
                                }
                                return;
                            }
                            self.bad = true;
                            return;
                        }
                    }
                    self.expr(lhs, true);
                    self.expr(rhs, false);
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) => self.expr(e, false),
                Stmt::Let(n, Some(rhs)) => {
                    if n == self.local {
                        return;
                    }
                    if self.slice_aliases.contains(n) {
                        if is_derived_slice_expr(rhs, self.slice_aliases) {
                            self.check_derived_slice(rhs);
                            return;
                        }
                        self.bad = true;
                        return;
                    }
                    if self.elem_aliases.contains(n) {
                        if is_elem_read_expr(rhs, self.slice_aliases) {
                            if let Expr::Index(_, idx) = rhs {
                                self.reads += 1;
                                self.expr(idx, false);
                            }
                            return;
                        }
                        self.bad = true;
                        return;
                    }
                    self.expr(rhs, false);
                }
                Stmt::Let(_, None) => {}
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(rhs) = init {
                            if n == self.local {
                                continue;
                            }
                            if self.slice_aliases.contains(n) {
                                if is_derived_slice_expr(rhs, self.slice_aliases) {
                                    self.check_derived_slice(rhs);
                                    continue;
                                }
                                self.bad = true;
                                return;
                            }
                            if self.elem_aliases.contains(n) {
                                if is_elem_read_expr(rhs, self.slice_aliases) {
                                    if let Expr::Index(_, idx) = rhs {
                                        self.reads += 1;
                                        self.expr(idx, false);
                                    }
                                    continue;
                                }
                                self.bad = true;
                                return;
                            }
                            self.expr(rhs, false);
                        }
                    }
                }
                Stmt::Const(n, e) => {
                    if self.slice_aliases.contains(n) || self.elem_aliases.contains(n) {
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
                    for c in cases {
                        c.tests.iter().for_each(|t| self.expr(t, false));
                        c.body.iter().for_each(|x| self.stmt(x));
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
                    if self.slice_aliases.contains(binding) || self.elem_aliases.contains(binding) {
                        self.bad = true;
                        return;
                    }
                    body.iter().for_each(|x| self.stmt(x));
                    handler.iter().for_each(|x| self.stmt(x));
                }
                Stmt::FunctionDecl { name, params, body } => {
                    if self.slice_aliases.contains(name)
                        || self.elem_aliases.contains(name)
                        || params.iter().any(|p| {
                            self.slice_aliases.contains(p) || self.elem_aliases.contains(p)
                        })
                    {
                        self.bad = true;
                        return;
                    }
                    body.iter().for_each(|x| self.stmt(x));
                }
                Stmt::Raw(_) => self.bad = true,
                _ => {}
            }
        }

        fn check_derived_slice(&mut self, expr: &Expr) {
            if let Expr::Object(entries) = expr {
                for (k, v) in entries {
                    if k == "buf" {
                        self.reads += 1;
                    } else {
                        self.expr(v, false);
                    }
                }
            } else {
                self.reads += 1;
            }
        }

        fn expr(&mut self, e: &Expr, is_lhs: bool) {
            if self.bad {
                return;
            }
            match e {
                Expr::Ident(name) => {
                    if self.slice_aliases.contains(name) || self.elem_aliases.contains(name) {
                        self.bad = true;
                    }
                }
                Expr::Member(obj, prop) => {
                    if let Expr::Ident(name) = obj.as_ref() {
                        if self.slice_aliases.contains(name) {
                            if !is_lhs && matches!(prop.as_str(), "off" | "len") {
                                self.reads += 1;
                                return;
                            }
                            self.bad = true;
                            return;
                        }
                        if self.elem_aliases.contains(name) {
                            if !is_lhs {
                                self.reads += 1;
                                return;
                            }
                            self.bad = true;
                            return;
                        }
                    }
                    self.expr(obj, false);
                }
                Expr::Index(obj, idx) => {
                    // `alias.buf[idx]` for scalar slices (nested element slices must assign to an
                    // element alias or immediately project a scalar field/index).
                    if is_elem_read_expr(e, self.slice_aliases) {
                        if is_lhs || self.nested_elems {
                            self.bad = true;
                            return;
                        }
                        self.reads += 1;
                        self.expr(idx, false);
                        return;
                    }
                    // `elem_alias[idx]` or `alias.buf[idx][k]` (read-only scalar read of nested element).
                    if let Expr::Ident(name) = obj.as_ref() {
                        if self.elem_aliases.contains(name) {
                            if is_lhs {
                                self.bad = true;
                                return;
                            }
                            self.reads += 1;
                            self.expr(idx, false);
                            return;
                        }
                    }
                    if is_elem_read_expr(obj, self.slice_aliases) {
                        if is_lhs {
                            self.bad = true;
                            return;
                        }
                        if let Expr::Index(_, outer_idx) = obj.as_ref() {
                            self.reads += 1;
                            self.expr(outer_idx, false);
                            self.expr(idx, false);
                            return;
                        }
                    }
                    self.expr(obj, false);
                    self.expr(idx, false);
                }
                Expr::Call(callee, args) if is_ptr_eq_callee(callee) && args.len() == 2 => {
                    for a in args {
                        if matches!(a, Expr::Ident(n) if self.slice_aliases.contains(n)) {
                            self.reads += 1;
                        } else {
                            self.expr(a, false);
                        }
                    }
                }
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
                    if params
                        .iter()
                        .any(|p| self.slice_aliases.contains(p) || self.elem_aliases.contains(p))
                    {
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

    let mut verify = Verify {
        local,
        slice_aliases: &slice_aliases,
        elem_aliases: &elem_aliases,
        nested_elems,
        reads: 0,
        bad: false,
    };
    body.iter().for_each(|s| verify.stmt(s));
    if !verify.bad && verify.reads >= 1 {
        Some(init_expr)
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
                    && (is_scalar_array_lit(rhs, 8, true)
                        || is_nested_scalar_array_lit(rhs, 8, true)
                        || is_pure_const_slice_lit(rhs, true).is_some()) =>
            {
                continue;
            }
            Stmt::ExprStmt(Expr::Seq(items)) => {
                items.retain(|item| {
                    !matches!(
                        item,
                        Expr::Assign(lhs, rhs)
                            if matches!(lhs.as_ref(), Expr::Ident(n) if hoisted.contains(n))
                                && (is_scalar_array_lit(rhs, 8, true)
                                    || is_nested_scalar_array_lit(rhs, 8, true)
                                    || is_pure_const_slice_lit(rhs, true).is_some())
                    )
                });
                if items.is_empty() {
                    continue;
                }
                if items.len() == 1 {
                    let single = items.pop().unwrap();
                    s = Stmt::ExprStmt(single);
                }
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
    allow_bigint: bool,
    bstr_strings: bool,
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
            if is_location_object(expr) {
                let key = format!("loc:{expr}");
                if let Some(hoisted) = self.const_map.get(&key) {
                    *expr = Expr::Ident(hoisted.as_str().to_owned());
                    self.changed = true;
                    return;
                }
            }
            if let Expr::Index(obj, idx) = expr {
                if is_scalar_array_lit(obj, 4, self.allow_bigint) {
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
                if params.is_empty() && is_pure_factory_expr(inner, self.allow_bigint) {
                    let key = format!("fn:{expr}");
                    if let Some(hoisted) = self.const_map.get(&key) {
                        *expr = Expr::Ident(hoisted.as_str().to_owned());
                        self.changed = true;
                        return;
                    }
                }
            }
            if self.bstr_strings {
                if let Expr::Str(s) = expr {
                    if is_hoistable_bstr(s) {
                        if let Some(hoisted) = self.const_map.get(&format!("str:{s}")) {
                            *expr = Expr::Ident(hoisted.as_str().to_owned());
                            self.changed = true;
                            return;
                        }
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

fn exact_int_lit(expr: &Expr) -> Option<i64> {
    match expr {
        Expr::Num(n)
            if n.is_finite()
                && n.fract() == 0.0
                && !(*n == 0.0 && n.is_sign_negative())
                && *n >= f64::from(i32::MIN)
                && *n <= f64::from(u32::MAX) =>
        {
            Some(*n as i64)
        }
        Expr::Unary(UnOp::Neg, inner) => {
            let v = exact_int_lit(inner)?;
            if v == 0 {
                return None;
            }
            let neg = v.checked_neg()?;
            (neg >= i64::from(i32::MIN) && neg <= i64::from(u32::MAX)).then_some(neg)
        }
        Expr::RawNum(s) => s
            .parse::<i64>()
            .ok()
            .filter(|&v| v >= i64::from(i32::MIN) && v <= i64::from(u32::MAX)),
        _ => None,
    }
}

fn typed_int_array_ctor(elems: &[Expr]) -> Option<&'static str> {
    if elems.len() < 8 {
        return None;
    }
    let mut min_v = i64::MAX;
    let mut max_v = i64::MIN;
    for e in elems {
        let v = exact_int_lit(e)?;
        min_v = min_v.min(v);
        max_v = max_v.max(v);
    }
    if min_v >= 0 && max_v <= 255 {
        Some("Uint8Array")
    } else if min_v >= 0 && max_v <= 65535 {
        Some("Uint16Array")
    } else if min_v >= i64::from(i32::MIN) && max_v <= i64::from(i32::MAX) {
        Some("Int32Array")
    } else if min_v >= 0 && max_v <= i64::from(u32::MAX) {
        Some("Uint32Array")
    } else {
        None
    }
}

fn wrap_typed_int_array(expr: Expr) -> Expr {
    if let Expr::Array(ref elems) = expr {
        if let Some(ctor) = typed_int_array_ctor(elems) {
            return Expr::New(Box::new(Expr::Ident(ctor.to_owned())), vec![expr]);
        }
    }
    expr
}

/// Upgrades internal read-only top-level integer static array items (`let T = [int, ...]` with
/// `len >= 8` whose only uses across all live items are read-only `T[idx]` / `T.length`) to
/// `const T = new <TypedArray>([...])` under `ReleaseSpeed`.
fn upgrade_readonly_static_int_arrays(
    table: &BTreeMap<JsName, LinkItem>,
    lowered: &mut [LoweredRow],
    live: &BTreeSet<JsName>,
    exclusions: &ProgramExclusions,
) {
    struct UseChecker<'a> {
        cands: &'a mut BTreeSet<String>,
    }
    impl UseChecker<'_> {
        fn stmts(&mut self, stmts: &[Stmt]) {
            for s in stmts {
                if self.cands.is_empty() {
                    return;
                }
                self.stmt(s);
            }
        }
        fn stmt(&mut self, s: &Stmt) {
            match s {
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => self.expr(e, false),
                Stmt::Const(n, e) => {
                    self.cands.remove(n);
                    self.expr(e, false);
                }
                Stmt::Let(n, init) => {
                    self.cands.remove(n);
                    if let Some(e) = init {
                        self.expr(e, false);
                    }
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        self.cands.remove(n);
                        if let Some(e) = init {
                            self.expr(e, false);
                        }
                    }
                }
                Stmt::If(t, a, b) => {
                    self.expr(t, false);
                    self.stmts(a);
                    if let Some(b) = b {
                        self.stmts(b);
                    }
                }
                Stmt::Switch(d, cases) => {
                    self.expr(d, false);
                    for c in cases {
                        for t in &c.tests {
                            self.expr(t, false);
                        }
                        self.stmts(&c.body);
                    }
                }
                Stmt::Labeled(_, inner) => self.stmt(inner),
                Stmt::LoopForever(_, b) | Stmt::Block(b) => self.stmts(b),
                Stmt::While(t, b) => {
                    self.expr(t, false);
                    self.stmts(b);
                }
                Stmt::TryCatch {
                    body,
                    binding,
                    handler,
                } => {
                    self.cands.remove(binding);
                    self.stmts(body);
                    self.stmts(handler);
                }
                Stmt::FunctionDecl { name, params, body } => {
                    self.cands.remove(name);
                    for p in params {
                        self.cands.remove(p);
                    }
                    self.stmts(body);
                }
                _ => {}
            }
        }
        fn expr(&mut self, e: &Expr, is_lhs: bool) {
            match e {
                Expr::Ident(id) => {
                    self.cands.remove(id);
                }
                Expr::Index(obj, idx) => {
                    if !is_lhs {
                        if let Expr::Ident(id) = obj.as_ref() {
                            if self.cands.contains(id) {
                                self.expr(idx, false);
                                return;
                            }
                        }
                    }
                    self.expr(obj, false);
                    self.expr(idx, false);
                }
                Expr::Member(obj, prop) => {
                    if !is_lhs && prop == "length" {
                        if let Expr::Ident(id) = obj.as_ref() {
                            if self.cands.contains(id) {
                                return;
                            }
                        }
                    }
                    self.expr(obj, false);
                }
                Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                    self.expr(l, true);
                    self.expr(r, false);
                }
                Expr::Unary(_, inner) => self.expr(inner, false),
                Expr::Binary(_, l, r) => {
                    self.expr(l, false);
                    self.expr(r, false);
                }
                Expr::Call(c, args) | Expr::New(c, args) => {
                    self.expr(c, false);
                    for a in args {
                        self.expr(a, false);
                    }
                }
                Expr::Cond(c, t, f) => {
                    self.expr(c, false);
                    self.expr(t, false);
                    self.expr(f, false);
                }
                Expr::Array(items) | Expr::Seq(items) => {
                    for x in items {
                        self.expr(x, false);
                    }
                }
                Expr::Object(entries) => {
                    for (_, v) in entries {
                        self.expr(v, false);
                    }
                }
                Expr::Arrow(params, body) => {
                    for p in params {
                        self.cands.remove(p);
                    }
                    match body {
                        ArrowBody::Expr(b) => self.expr(b, false),
                        ArrowBody::Block(s) => self.stmts(s),
                    }
                }
                _ => {}
            }
        }
    }

    let lowered_ok: BTreeSet<&JsName> = lowered
        .iter()
        .filter_map(|(name, res, _, _)| res.is_ok().then_some(name))
        .collect();

    let mut cands: BTreeSet<String> = BTreeSet::new();
    for (item_name, res, _, _) in lowered.iter() {
        let Ok(item) = res else { continue };
        if !live.contains(item_name) {
            continue;
        }
        if table
            .get(item_name)
            .is_some_and(|t| t.linkage.exported || t.kind == ItemKind::Import)
        {
            continue;
        }
        let arr_opt = match &item.decl {
            Stmt::Let(n, Some(Expr::Array(elems))) | Stmt::Const(n, Expr::Array(elems))
                if n == item_name.as_str() =>
            {
                Some(elems)
            }
            _ => None,
        };
        let Some(elems) = arr_opt else { continue };
        if typed_int_array_ctor(elems).is_some()
            && !exclusions.raw_tokens.contains(item_name.as_str())
        {
            cands.insert(item_name.as_str().to_owned());
        }
    }
    if cands.is_empty() {
        return;
    }

    // Disqualify any candidate referenced by a non-lowered live item (e.g. baseline fallback JS).
    for item in table.values() {
        if !live.contains(&item.name) || lowered_ok.contains(&item.name) {
            continue;
        }
        cands.retain(|c| !item.refs.iter().any(|r| r.as_str() == c) && !item.js.contains(c));
    }
    if cands.is_empty() {
        return;
    }

    // Verify every use across all lowered items is strictly read-only `T[idx]` or `T.length`.
    for (item_name, res, _, _) in lowered.iter() {
        if cands.is_empty() {
            return;
        }
        let Ok(item) = res else { continue };
        if cands.contains(item_name.as_str()) {
            // Skip scanning the candidate's own top-level declaration.
            continue;
        }
        let mut checker = UseChecker { cands: &mut cands };
        checker.stmt(&item.decl);
    }
    if cands.is_empty() {
        return;
    }

    for (item_name, res, _, _) in lowered.iter_mut() {
        if !cands.contains(item_name.as_str()) {
            continue;
        }
        let Ok(item) = res else { continue };
        let decl = std::mem::replace(&mut item.decl, Stmt::Comment(String::new()));
        let upgraded = match decl {
            Stmt::Let(n, Some(arr)) | Stmt::Const(n, arr) => {
                Stmt::Const(n, wrap_typed_int_array(arr))
            }
            other => other,
        };
        item.decl = upgraded;
        item.refs = super::lane::refs_of(&item.decl, item_name);
    }
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
    let allow_bigint = opts.ecma_target != EcmaTarget::Es2018;
    let speed_profile = opts.profile == Some(crate::opts::Profile::ReleaseSpeed);

    // Pass 1: Hoist single-assignment read-only scalar lookup tables at any nesting depth.
    for (item_name, res, _, _) in lowered.iter_mut() {
        let Ok(lowered_item) = res else { continue };
        let Stmt::FunctionDecl { params, body, .. } = &mut lowered_item.decl else {
            continue;
        };
        let param_set: BTreeSet<String> = params.iter().cloned().collect();
        let mut declared_locals: Vec<String> = Vec::new();
        collect_declared_locals(body, &param_set, &mut declared_locals);

        let mut candidates: Vec<(String, Expr, bool)> = Vec::new();
        for local in &declared_locals {
            if let Some(arr_expr) = find_read_only_table_init(body, local, allow_bigint) {
                candidates.push((local.clone(), arr_expr, true));
            } else if let Some(nested_expr) =
                find_read_only_nested_array_init(body, local, allow_bigint)
            {
                candidates.push((local.clone(), nested_expr, false));
            } else if let Some(slice_expr) =
                find_read_only_const_slice_init(body, local, allow_bigint)
            {
                candidates.push((local.clone(), slice_expr, false));
            }
        }

        if !candidates.is_empty() {
            let mut hoisted_set: BTreeSet<String> = BTreeSet::new();
            let mut replacements: Vec<(String, JsName)> = Vec::new();
            for (local, arr_expr, is_read_only_table) in candidates {
                let arr_expr = if speed_profile && is_read_only_table {
                    wrap_typed_int_array(arr_expr)
                } else {
                    arr_expr
                };
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
    encode_internal_tags(lowered, &exclusions, speed_profile);

    // Pass 2: Expression constants (`Location` objects, inline `[...][i]`, capture-free
    // `() => <const>`) and shared unreachable-abort outlining. ES2018 BigInt constants are hoisted
    // afterwards by `lower::bigint::hoist` with load-safe `$BI_ok` / `$BI_no()` guards.
    let bstr_strings = crate::lower::bstr::enabled();
    let mut collector = HoistCollector {
        allow_bigint,
        bstr_strings,
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
        let qualify = if key.starts_with("loc:") {
            usage.in_loop || usage.count >= 2 || speed_profile
        } else if key.starts_with("str:") {
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
        let init_expr = if speed_profile && key.starts_with("arr:") {
            wrap_typed_int_array(expr.clone())
        } else {
            expr.clone()
        };
        let dedup_key = if speed_profile && key.starts_with("arr:") {
            format!("arr:{init_expr}")
        } else {
            key.clone()
        };
        let hc_name = if let Some(existing) = table_dedup.get(&dedup_key) {
            existing.clone()
        } else {
            let new_name = fresh_item_name("$hc", &mut counter, table, defined);
            defined.insert(new_name.clone());
            live.insert(new_name.clone());
            let mut decl = jsast::const_(new_name.as_str(), init_expr);
            if bstr_strings {
                crate::lower::bstr::rewrite(&mut decl);
            }
            let item = build_synthesized_link_item(
                new_name.clone(),
                ItemKind::Const,
                decl,
                "hoisted constant",
            );
            synthesized.push(item);
            table_dedup.insert(dedup_key, new_name.clone());
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
                allow_bigint,
                bstr_strings,
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

    // Pass 3: Upgrade internal read-only top-level integer static array items (`CRCT`, `DEO`, etc.)
    // to `const NAME = new <TypedArray>([...])` under `ReleaseSpeed`.
    if speed_profile {
        upgrade_readonly_static_int_arrays(table, lowered, live, &exclusions);
    }

    for item in synthesized {
        table.insert(item.name.clone(), item);
    }

    added_any
}

/// Extracts the byte-string sibling `const bs$h... = "<byte_string>";` for a non-ASCII
/// backend-hoisted `const s$h... = "<utf16>";` constant under `js-str=bytes`.
pub(super) fn synthesize_bstr_const_item(s_name: &str, js: &str) -> Option<LinkItem> {
    let (_, rhs) = js.split_once('=')?;
    let rhs = rhs.trim();
    let quote = rhs.as_bytes().first().copied()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let mut i = 1usize;
    let bytes = rhs.as_bytes();
    let mut text = String::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b == quote {
            let tail = rhs[i + 1..].trim_start();
            if !(tail.is_empty() || tail.starts_with(';')) {
                return None;
            }
            if text.is_ascii() || text.len() < crate::lower::js::constant::HOIST_STRING_BYTES {
                return None;
            }
            if crate::lower::js::constant::hoisted_name("s", &text) != s_name {
                return None;
            }
            let bs_name = JsName::new(crate::lower::js::constant::hoisted_name("bs", &text));
            let encoded = crate::lower::bstr::byte_string(&text);
            return Some(build_synthesized_link_item(
                bs_name.clone(),
                ItemKind::Const,
                jsast::const_(bs_name.as_str(), jsast::string(encoded)),
                "hoisted byte-string constant",
            ));
        } else if b == b'\\' {
            i += 1;
            decode_js_escape(rhs, &mut i, &mut text);
        } else {
            let ch = rhs[i..].chars().next()?;
            text.push(ch);
            i += ch.len_utf8();
        }
    }
    None
}

/// Rewrites a producer-hoisted `const loc$h... = { filename: __rt.str_bytes("..."), ... };`
/// item into its `js-str=bytes` twin (`__rt.bstr_bytes("<byte_string>")`).
pub(super) fn rewrite_bstr_location_const_item(js: &str) -> Option<String> {
    const NEEDLE: &str = "__rt.str_bytes(";
    let call_start = js.find(NEEDLE)?;
    let arg_start = call_start + NEEDLE.len();
    let rest = &js[arg_start..];
    let quote = rest.as_bytes().first().copied()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut i = 1usize;
    let mut text = String::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b == quote {
            let after_quote = &rest[i + 1..];
            if !after_quote.starts_with(')') {
                return None;
            }
            let encoded = crate::lower::bstr::byte_string(&text);
            let encoded_lit = jsast::expr_to_string(&jsast::string(encoded));
            let mut out = String::with_capacity(js.len() + encoded_lit.len());
            out.push_str(&js[..call_start]);
            out.push_str("__rt.bstr_bytes(");
            out.push_str(&encoded_lit);
            out.push_str(after_quote);
            return Some(out);
        } else if b == b'\\' {
            i += 1;
            decode_js_escape(rest, &mut i, &mut text);
        } else {
            let ch = rest[i..].chars().next()?;
            text.push(ch);
            i += ch.len_utf8();
        }
    }
    None
}

struct ProgramExclusions {
    raw_tokens: BTreeSet<String>,
    raw_strings: BTreeSet<String>,
    boundary_fields: BTreeSet<String>,
    boundary_variants: BTreeSet<String>,
}

fn decode_js_escape(text: &str, i: &mut usize, out: &mut String) {
    let bytes = text.as_bytes();
    if *i >= bytes.len() {
        return;
    }
    match bytes[*i] {
        b'\\' => {
            out.push('\\');
            *i += 1;
        }
        b'"' => {
            out.push('"');
            *i += 1;
        }
        b'\'' => {
            out.push('\'');
            *i += 1;
        }
        b'`' => {
            out.push('`');
            *i += 1;
        }
        b'n' => {
            out.push('\n');
            *i += 1;
        }
        b'r' => {
            out.push('\r');
            *i += 1;
        }
        b't' => {
            out.push('\t');
            *i += 1;
        }
        b'b' => {
            out.push('\u{0008}');
            *i += 1;
        }
        b'f' => {
            out.push('\u{000C}');
            *i += 1;
        }
        b'v' => {
            out.push('\u{000B}');
            *i += 1;
        }
        b'0' if !bytes.get(*i + 1).is_some_and(|c| c.is_ascii_digit()) => {
            out.push('\0');
            *i += 1;
        }
        b'\r' => {
            *i += 1;
            if bytes.get(*i) == Some(&b'\n') {
                *i += 1;
            }
        }
        b'\n' => {
            *i += 1;
        }
        b'x' if *i + 3 <= bytes.len()
            && bytes[*i + 1].is_ascii_hexdigit()
            && bytes[*i + 2].is_ascii_hexdigit() =>
        {
            if let Ok(v) = u32::from_str_radix(&text[*i + 1..*i + 3], 16) {
                if let Some(ch) = char::from_u32(v) {
                    out.push(ch);
                }
                *i += 3;
            } else {
                out.push('x');
                *i += 1;
            }
        }
        b'u' => {
            if bytes.get(*i + 1) == Some(&b'{') {
                let mut end = *i + 2;
                while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
                    end += 1;
                }
                if end > *i + 2 && bytes.get(end) == Some(&b'}') {
                    if let Ok(v) = u32::from_str_radix(&text[*i + 2..end], 16) {
                        if let Some(ch) = char::from_u32(v) {
                            out.push(ch);
                        }
                        *i = end + 1;
                        return;
                    }
                }
            } else if *i + 5 <= bytes.len()
                && bytes[*i + 1..*i + 5].iter().all(u8::is_ascii_hexdigit)
            {
                if let Ok(code) = u32::from_str_radix(&text[*i + 1..*i + 5], 16) {
                    *i += 5;
                    if (0xD800..=0xDBFF).contains(&code)
                        && *i + 6 <= bytes.len()
                        && bytes[*i] == b'\\'
                        && bytes[*i + 1] == b'u'
                        && bytes[*i + 2..*i + 6].iter().all(u8::is_ascii_hexdigit)
                    {
                        if let Ok(lo) = u32::from_str_radix(&text[*i + 2..*i + 6], 16) {
                            if (0xDC00..=0xDFFF).contains(&lo) {
                                let combined = 0x10000 + ((code - 0xD800) << 10) + (lo - 0xDC00);
                                if let Some(ch) = char::from_u32(combined) {
                                    out.push(ch);
                                }
                                *i += 6;
                                return;
                            }
                        }
                    }
                    if let Some(ch) = char::from_u32(code) {
                        out.push(ch);
                    }
                    return;
                }
            }
            out.push('u');
            *i += 1;
        }
        _ => {
            if let Some(ch) = text[*i..].chars().next() {
                out.push(ch);
                *i += ch.len_utf8();
            } else {
                *i += 1;
            }
        }
    }
}

fn collect_js_text_span(
    text: &str,
    i: &mut usize,
    stop_on_close_brace: bool,
    tokens: &mut BTreeSet<String>,
    strings: &mut BTreeSet<String>,
) {
    let bytes = text.as_bytes();
    let mut brace_depth = 0usize;
    while *i < bytes.len() {
        let b = bytes[*i];
        if b == b'/' && bytes.get(*i + 1) == Some(&b'/') {
            *i += 2;
            while *i < bytes.len() && bytes[*i] != b'\n' && bytes[*i] != b'\r' {
                *i += 1;
            }
        } else if b == b'/' && bytes.get(*i + 1) == Some(&b'*') {
            *i += 2;
            while *i + 1 < bytes.len() && !(bytes[*i] == b'*' && bytes[*i + 1] == b'/') {
                *i += 1;
            }
            *i = (*i + 2).min(bytes.len());
        } else if stop_on_close_brace && b == b'{' {
            brace_depth += 1;
            *i += 1;
        } else if stop_on_close_brace && b == b'}' {
            if brace_depth == 0 {
                *i += 1;
                return;
            }
            brace_depth -= 1;
            *i += 1;
        } else if b.is_ascii_alphabetic() || b == b'_' || b == b'$' {
            let start = *i;
            *i += 1;
            while *i < bytes.len()
                && (bytes[*i].is_ascii_alphanumeric() || bytes[*i] == b'_' || bytes[*i] == b'$')
            {
                *i += 1;
            }
            tokens.insert(text[start..*i].to_owned());
        } else if b == b'"' || b == b'\'' {
            let quote = b;
            *i += 1;
            let mut decoded = String::new();
            while *i < bytes.len() && bytes[*i] != quote {
                if bytes[*i] == b'\\' {
                    *i += 1;
                    decode_js_escape(text, i, &mut decoded);
                } else if bytes[*i] < 0x80 {
                    decoded.push(bytes[*i] as char);
                    *i += 1;
                } else if let Some(ch) = text[*i..].chars().next() {
                    decoded.push(ch);
                    *i += ch.len_utf8();
                } else {
                    *i += 1;
                }
            }
            strings.insert(decoded);
            if *i < bytes.len() {
                *i += 1;
            }
        } else if b == b'`' {
            *i += 1;
            let mut decoded = String::new();
            let mut has_interp = false;
            while *i < bytes.len() && bytes[*i] != b'`' {
                if bytes[*i] == b'\\' {
                    *i += 1;
                    decode_js_escape(text, i, &mut decoded);
                } else if bytes[*i] == b'$' && bytes.get(*i + 1) == Some(&b'{') {
                    has_interp = true;
                    *i += 2;
                    collect_js_text_span(text, i, true, tokens, strings);
                } else if bytes[*i] < 0x80 {
                    decoded.push(bytes[*i] as char);
                    *i += 1;
                } else if let Some(ch) = text[*i..].chars().next() {
                    decoded.push(ch);
                    *i += ch.len_utf8();
                } else {
                    *i += 1;
                }
            }
            if !has_interp {
                strings.insert(decoded);
            }
            if *i < bytes.len() {
                *i += 1;
            }
        } else if b < 0x80 {
            *i += 1;
        } else if let Some(ch) = text[*i..].chars().next() {
            *i += ch.len_utf8();
        } else {
            *i += 1;
        }
    }
}

fn collect_js_text_tokens_and_strings(
    text: &str,
    tokens: &mut BTreeSet<String>,
    strings: &mut BTreeSet<String>,
) {
    let mut i = 0;
    collect_js_text_span(text, &mut i, false, tokens, strings);
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

struct BoundaryWalkCtx<'a> {
    items_by_id: BTreeMap<ItemId, &'a LinkItem>,
    items_by_def: BTreeMap<DefinitionId, Vec<&'a LinkItem>>,
    visited_types: BTreeSet<(usize, TypeId)>,
    visited_items: BTreeSet<JsName>,
    fields: BTreeSet<String>,
    variants: BTreeSet<String>,
}

impl<'a> BoundaryWalkCtx<'a> {
    fn new(table: &'a BTreeMap<JsName, LinkItem>) -> Self {
        let mut items_by_id = BTreeMap::new();
        let mut items_by_def: BTreeMap<DefinitionId, Vec<&'a LinkItem>> = BTreeMap::new();
        for item in table.values() {
            if let Some(id) = item
                .jir
                .summary
                .as_ref()
                .and_then(|s| s.key.item)
                .or_else(|| item.jir.body.as_ref().map(|b| b.item))
            {
                items_by_id.entry(id).or_insert(item);
                items_by_def.entry(id.definition).or_default().push(item);
            }
        }
        Self {
            items_by_id,
            items_by_def,
            visited_types: BTreeSet::new(),
            visited_items: BTreeSet::new(),
            fields: BTreeSet::new(),
            variants: BTreeSet::new(),
        }
    }

    fn walk_item_signature(&mut self, item: &'a LinkItem) {
        if !self.visited_items.insert(item.name.clone()) {
            return;
        }
        let (Some(interns), Some(body)) = (item.jir.interns(), &item.jir.body) else {
            return;
        };
        for &arg_ty in &body.signature.arguments {
            self.walk_type(arg_ty, interns);
        }
        self.walk_type(body.signature.result, interns);
    }

    fn walk_item_id(&mut self, id: ItemId) {
        if let Some(item) = self.items_by_id.get(&id).copied() {
            self.walk_item_signature(item);
        }
    }

    fn walk_type(&mut self, ty: TypeId, interns: &'a Interns) {
        let types = &interns.types;
        if !self
            .visited_types
            .insert((types as *const TypeTable as usize, ty))
        {
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
                            self.variants.insert(name.to_owned());
                        }
                    }
                    for f in &v.fields {
                        if let FieldKey::Name(id) = f.key {
                            if let Some(name) = interns.str(id) {
                                self.fields.insert(name.to_owned());
                            }
                        }
                        self.walk_type(f.ty, interns);
                    }
                }
                for &arg in args {
                    self.walk_type(arg, interns);
                }
            }
            TypeKind::Tuple(elems) => {
                for &el in elems {
                    self.walk_type(el, interns);
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
                self.walk_type(*element, interns);
            }
            TypeKind::FnPtr { sig } => {
                if let Some(sig_desc) = interns.sigs.get(*sig) {
                    for &inp in &sig_desc.inputs {
                        self.walk_type(inp, interns);
                    }
                    self.walk_type(sig_desc.output, interns);
                }
            }
            TypeKind::FnDef { sig, instance, .. } => {
                if let Some(sig_desc) = interns.sigs.get(*sig) {
                    for &inp in &sig_desc.inputs {
                        self.walk_type(inp, interns);
                    }
                    self.walk_type(sig_desc.output, interns);
                }
                if let Some(direct) = instance {
                    self.walk_item_id(direct.item);
                }
            }
            TypeKind::Closure {
                def,
                upvars,
                fn_pointer_shim,
            } => {
                for &u in upvars {
                    self.walk_type(u, interns);
                }
                if let Some(shim) = fn_pointer_shim {
                    self.walk_item_id(shim.item);
                }
                let exact = ItemId {
                    definition: *def,
                    instantiation: ty.0,
                };
                if let Some(closure_item) = self.items_by_id.get(&exact).copied() {
                    self.walk_item_signature(closure_item);
                } else if let Some(defs) = self.items_by_def.get(def).cloned() {
                    for closure_item in defs {
                        self.walk_item_signature(closure_item);
                    }
                }
            }
            TypeKind::CoroutineClosure { def, upvars } => {
                for &u in upvars {
                    self.walk_type(u, interns);
                }
                if let Some(defs) = self.items_by_def.get(def).cloned() {
                    for closure_item in defs {
                        self.walk_item_signature(closure_item);
                    }
                }
            }
            TypeKind::Coroutine {
                upvars,
                saved_locals,
                variants: co_variants,
                ..
            } => {
                for &u in upvars {
                    self.walk_type(u, interns);
                }
                for &(_, lt) in saved_locals {
                    self.walk_type(lt, interns);
                }
                for v in co_variants {
                    for f in &v.fields {
                        if let FieldKey::Name(id) = f.key {
                            if let Some(name) = interns.str(id) {
                                self.fields.insert(name.to_owned());
                            }
                        }
                        self.walk_type(f.ty, interns);
                    }
                }
            }
            _ => {}
        }
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
    let mut ctx = BoundaryWalkCtx::new(table);

    for item in table.values() {
        if !live.contains(&item.name) {
            continue;
        }
        if !lowered_ok.contains(&item.name) {
            collect_js_text_tokens_and_strings(&item.js, &mut raw_tokens, &mut raw_strings);
        }
        if let Some(interns) = item.jir.interns() {
            for desc in interns.types.descriptors() {
                if desc.meta.boundary != PinnedBoundary::None || desc.meta.abi == NominalAbi::C {
                    ctx.walk_type(desc.id, interns);
                }
            }
            if let Some(body) = &item.jir.body {
                if item.linkage.exported
                    || item.kind == ItemKind::Import
                    || item.dts.is_some()
                    || lowered_dts.contains(&item.name)
                    || !lowered_ok.contains(&item.name)
                {
                    ctx.walk_item_signature(item);
                }
                for block in &body.blocks {
                    let Terminator::Call(call) = &block.terminator else {
                        continue;
                    };
                    let is_boundary_call = call.host.is_some()
                        || matches!(
                            call.semantic,
                            CallSemantic::Host
                                | CallSemantic::ViewAbi
                                | CallSemantic::RawJavaScript
                                | CallSemantic::RawParts
                        )
                        || (matches!(call.callee, Callee::Foreign { .. } | Callee::Runtime { .. })
                            && !crate::plan::enum_repr::is_representation_opaque_runtime(
                                &call.callee,
                                interns,
                            ))
                        || match &call.callee {
                            Callee::Direct {
                                item: target_id,
                                link_name,
                            } => {
                                let target_name = ctx
                                    .items_by_id
                                    .get(target_id)
                                    .map(|it| it.name.clone())
                                    .or_else(|| {
                                        interns.str(*link_name).map(|s| JsName::new(s.to_owned()))
                                    });
                                !target_name.as_ref().is_some_and(|n| lowered_ok.contains(n))
                            }
                            _ => false,
                        };
                    if !is_boundary_call {
                        continue;
                    }
                    for arg in &call.arguments {
                        ctx.walk_type(arg.ty, interns);
                    }
                    ctx.walk_type(call.destination.ty, interns);
                    if let Some(host) = &call.host {
                        for (_, helper) in host.helpers() {
                            ctx.walk_item_id(helper.item);
                        }
                        if let HostPayload::KeyedList(kl) = host {
                            ctx.walk_type(kl.collection_ty, interns);
                            ctx.walk_type(kl.item_ty, interns);
                            ctx.walk_type(kl.key_ty, interns);
                            ctx.walk_type(kl.row_ty, interns);
                            ctx.walk_type(kl.collection.ty, interns);
                            ctx.walk_type(kl.key.ty, interns);
                            ctx.walk_type(kl.row.ty, interns);
                            for body_ref in [kl.collection.body, kl.key.body, kl.row.body]
                                .into_iter()
                                .flatten()
                            {
                                ctx.walk_item_id(body_ref.item);
                            }
                        }
                    }
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
        boundary_fields: ctx.fields,
        boundary_variants: ctx.variants,
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

fn is_unsigned_32_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Binary(BinOp::UShr, _, r) => matches!(&**r, Expr::Num(n) if *n == 0.0),
        Expr::Num(n) => n.is_finite() && n.fract() == 0.0 && *n >= 0.0 && *n <= (u32::MAX as f64),
        Expr::RawNum(s) => s
            .strip_prefix("0x")
            .or_else(|| s.strip_prefix("0X"))
            .map_or_else(
                || s.parse::<u32>().is_ok(),
                |hex| u32::from_str_radix(hex, 16).is_ok(),
            ),
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
    if is_pure_no_call(lo_arg) && is_pure_no_call(hi_arg) && is_unsigned_32_expr(lo_arg) {
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

        fn mark_idents_bad_in_stmts(&mut self, stmts: &[Stmt]) {
            for stmt in stmts {
                match stmt {
                    Stmt::ExprStmt(e) | Stmt::Throw(e) => self.mark_idents_bad_in_expr(e),
                    Stmt::Const(n, e) => {
                        self.mark_bad(n);
                        self.mark_idents_bad_in_expr(e);
                    }
                    Stmt::Let(n, init) => {
                        self.mark_bad(n);
                        if let Some(e) = init {
                            self.mark_idents_bad_in_expr(e);
                        }
                    }
                    Stmt::Lets(decls) => {
                        for (n, init) in decls {
                            self.mark_bad(n);
                            if let Some(e) = init {
                                self.mark_idents_bad_in_expr(e);
                            }
                        }
                    }
                    Stmt::Return(Some(e)) => self.mark_idents_bad_in_expr(e),
                    Stmt::If(t, a, b) => {
                        self.mark_idents_bad_in_expr(t);
                        self.mark_idents_bad_in_stmts(a);
                        if let Some(b) = b {
                            self.mark_idents_bad_in_stmts(b);
                        }
                    }
                    Stmt::Switch(d, cases) => {
                        self.mark_idents_bad_in_expr(d);
                        for c in cases {
                            for t in &c.tests {
                                self.mark_idents_bad_in_expr(t);
                            }
                            self.mark_idents_bad_in_stmts(&c.body);
                        }
                    }
                    Stmt::Labeled(_, inner) => {
                        self.mark_idents_bad_in_stmts(std::slice::from_ref(inner.as_ref()));
                    }
                    Stmt::LoopForever(_, b) | Stmt::Block(b) => self.mark_idents_bad_in_stmts(b),
                    Stmt::While(t, b) => {
                        self.mark_idents_bad_in_expr(t);
                        self.mark_idents_bad_in_stmts(b);
                    }
                    Stmt::TryCatch {
                        body,
                        binding,
                        handler,
                    } => {
                        self.mark_bad(binding);
                        self.mark_idents_bad_in_stmts(body);
                        self.mark_idents_bad_in_stmts(handler);
                    }
                    Stmt::FunctionDecl { name, params, body } => {
                        self.mark_bad(name);
                        for p in params {
                            self.mark_bad(p);
                        }
                        self.mark_idents_bad_in_stmts(body);
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
        }

        fn mark_idents_bad_in_expr(&mut self, expr: &Expr) {
            match expr {
                Expr::Ident(name) => self.mark_bad(name),
                Expr::Member(obj, _) | Expr::Unary(_, obj) => self.mark_idents_bad_in_expr(obj),
                Expr::Index(obj, idx)
                | Expr::Binary(_, obj, idx)
                | Expr::Assign(obj, idx)
                | Expr::CompoundAssign(_, obj, idx) => {
                    self.mark_idents_bad_in_expr(obj);
                    self.mark_idents_bad_in_expr(idx);
                }
                Expr::Call(c, args) | Expr::New(c, args) => {
                    self.mark_idents_bad_in_expr(c);
                    for a in args {
                        self.mark_idents_bad_in_expr(a);
                    }
                }
                Expr::Cond(t, c, a) => {
                    self.mark_idents_bad_in_expr(t);
                    self.mark_idents_bad_in_expr(c);
                    self.mark_idents_bad_in_expr(a);
                }
                Expr::Object(entries) => {
                    for (_, v) in entries {
                        self.mark_idents_bad_in_expr(v);
                    }
                }
                Expr::Array(items) | Expr::Seq(items) => {
                    for x in items {
                        self.mark_idents_bad_in_expr(x);
                    }
                }
                Expr::Arrow(params, body) => {
                    for p in params {
                        self.mark_bad(p);
                    }
                    match body {
                        ArrowBody::Expr(b) => self.mark_idents_bad_in_expr(b),
                        ArrowBody::Block(stmts) => self.mark_idents_bad_in_stmts(stmts),
                    }
                }
                Expr::Raw(text) => {
                    let mut dummy = BTreeSet::new();
                    collect_js_text_tokens_and_strings(text, &mut self.bad_names, &mut dummy);
                }
                _ => {}
            }
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
                    self.mark_idents_bad_in_stmts(body);
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
                        ArrowBody::Expr(b) => self.mark_idents_bad_in_expr(b),
                        ArrowBody::Block(stmts) => self.mark_idents_bad_in_stmts(stmts),
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
                self.exclude_prop_and_base(s);
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
    for s in exclusions
        .raw_tokens
        .iter()
        .chain(exclusions.raw_strings.iter())
        .chain(exclusions.boundary_fields.iter())
    {
        scanner.exclude_prop_and_base(s);
    }

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

fn encode_internal_tags(
    lowered: &mut [LoweredRow],
    exclusions: &ProgramExclusions,
    encode_option_tags: bool,
) {
    let mut scanner = TagScanner {
        candidates: BTreeSet::new(),
        bad: BTreeSet::new(),
    };
    for s in ["Ok", "Err", "Ready", "Pending", ""] {
        scanner.bad.insert(s.to_owned());
    }
    if !encode_option_tags {
        scanner.bad.insert("Some".to_owned());
        scanner.bad.insert("None".to_owned());
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
                    bigints: BTreeSet::new(),
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
        // Expect 1 constant (loc) and 1 unreachable helper function ($ur_0); BigInt constants are
        // left for `lower::bigint::hoist` so they get load-safe `$BI_ok`/`$BI_no()` guards.
        assert_eq!(table.len(), 2);
        assert_eq!(
            table.values().filter(|i| i.kind == ItemKind::Const).count(),
            1
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
            rendered.contains("BigInt(\"12345678901234567890\")"),
            "BigInt left for lower::bigint::hoist on Es2018: {rendered}"
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
                    bigints: BTreeSet::new(),
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
                bigints: BTreeSet::new(),
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
        let (_, report_lowered, _, _, _) =
            test_run(report_opts, vec![("use_internal", fn_a.clone())]);
        let report_js = jsast::stmt_to_string(&report_lowered[0].1.as_ref().unwrap().decl);
        assert!(report_js.contains("\"Alpha\""), "{report_js}");
        assert!(report_js.contains("\"Beta\""), "{report_js}");

        // Under ReleaseSpeed, internal `"Some"`/`"None"` tags are integer-encoded too unless excluded.
        let speed_opts = Opts {
            optimizer: OptimizerMode::On,
            profile: Some(crate::opts::Profile::ReleaseSpeed),
            ..Opts::default()
        };
        let (_, speed_lowered, _, _, _) = test_run(speed_opts, vec![("use_internal", fn_a)]);
        let speed_js = jsast::stmt_to_string(&speed_lowered[0].1.as_ref().unwrap().decl);
        assert!(!speed_js.contains("\"Some\""), "{speed_js}");
        assert!(speed_js.contains("s.TAG ==="), "{speed_js}");
    }

    #[test]
    fn collect_program_exclusions_protects_js_extern_host_call_and_closure_types() {
        use crate::item::LinkJir;
        use crate::jir::semantic::{
            AbiSignature, BasicBlock, BlockId, BodySemantic, Call, Constant, Local, LocalId,
            LocalRole, Operand, OperandKind, Place, Signature, Unwind,
        };
        use crate::jir::types::{
            EnumRepr, FieldDescriptor, IntFacts, IntRepr, JsFacts, LayoutFacts,
            NominalDiscriminant, TypeDescriptor, TypeMeta, VariantDescriptor,
        };
        use crate::jir::{Body, BodyKind, BytesId};

        let mut interns = Interns::default();
        let u64_ty = TypeId(1);
        let struct_ty = TypeId(2);
        let enum_ty = TypeId(3);
        let closure_ty = TypeId(4);
        let cb_struct_ty = TypeId(5);
        let cb_enum_ty = TypeId(6);
        let unit_ty = TypeId(7);
        let fn_ty = TypeId(8);

        let def_of = |n: u64| DefinitionId {
            crate_id: 1,
            local_hash: n,
        };
        let mk_desc = |id: TypeId, kind: TypeKind, size: u64| TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                size: Some(size),
                unsized_prefix: None,
                align: Some(size.clamp(1, 4)),
                sized: true,
                zst: size == 0,
                one_zst: size == 0,
                needs_drop: false,
                has_own_drop: false,
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: size == 0,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: id,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        };
        interns.intern_type(mk_desc(unit_ty, TypeKind::Tuple(Vec::new()), 0));
        let mut u64_desc = mk_desc(
            u64_ty,
            TypeKind::Integer {
                signed: false,
                bits: 64,
            },
            8,
        );
        u64_desc.js.int = Some(IntFacts {
            signed: false,
            bits: 64,
            repr: IntRepr::BigInt,
        });
        interns.intern_type(u64_desc);
        interns.intern_type(mk_desc(fn_ty, TypeKind::Opaque, 0));

        let ts_key = interns.intern_str("ext_ts");
        let cb_ts_key = interns.intern_str("cb_ts");
        let go_key = interns.intern_str("Go");
        let stop_key = interns.intern_str("Stop");
        let cb_done_key = interns.intern_str("CbDone");
        let extern_struct_key = interns.intern_str("ExternStruct");
        let cb_struct_key = interns.intern_str("CbStruct");

        interns.intern_type(mk_desc(
            struct_ty,
            TypeKind::Adt {
                def: def_of(10),
                args: Vec::new(),
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: extern_struct_key,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(ts_key),
                        ty: u64_ty,
                        offset: 0,
                    }],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            8,
        ));
        let mut enum_desc = mk_desc(
            enum_ty,
            TypeKind::Adt {
                def: def_of(11),
                args: Vec::new(),
                form: NominalForm::Enum,
                variants: vec![
                    VariantDescriptor {
                        name: go_key,
                        fields: Vec::new(),
                        logical_discr: 0,
                        physical_tag: Some(NominalDiscriminant { bits: 0, bytes: 1 }),
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                    VariantDescriptor {
                        name: stop_key,
                        fields: Vec::new(),
                        logical_discr: 1,
                        physical_tag: Some(NominalDiscriminant { bits: 1, bytes: 1 }),
                        result_ok: false,
                        saved_local_keys: Vec::new(),
                    },
                ],
                niche: None,
            },
            1,
        );
        enum_desc.js.enum_repr = Some(EnumRepr::Tag { fieldless: true });
        interns.intern_type(enum_desc);
        interns.intern_type(mk_desc(
            cb_struct_ty,
            TypeKind::Adt {
                def: def_of(12),
                args: Vec::new(),
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: cb_struct_key,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(cb_ts_key),
                        ty: u64_ty,
                        offset: 0,
                    }],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            8,
        ));
        interns.intern_type(mk_desc(
            cb_enum_ty,
            TypeKind::Adt {
                def: def_of(13),
                args: Vec::new(),
                form: NominalForm::Enum,
                variants: vec![VariantDescriptor {
                    name: cb_done_key,
                    fields: Vec::new(),
                    logical_discr: 0,
                    physical_tag: Some(NominalDiscriminant { bits: 0, bytes: 1 }),
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            1,
        ));

        let closure_def = def_of(20);
        let closure_item_id = ItemId {
            definition: closure_def,
            instantiation: closure_ty.0,
        };
        interns.intern_type(mk_desc(
            closure_ty,
            TypeKind::Closure {
                def: closure_def,
                upvars: Vec::new(),
                fn_pointer_shim: None,
            },
            8,
        ));

        let caller_item_id = ItemId {
            definition: def_of(30),
            instantiation: 1,
        };
        let host_section = interns.intern_str("rcgjs.host.callback0");
        let caller_body = Body {
            item: caller_item_id,
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: Vec::new(),
                result: unit_ty,
                abi: AbiSignature::default(),
            },
            locals: vec![Local {
                id: LocalId(0),
                role: LocalRole::Return,
                ty: enum_ty,
            }],
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    cleanup: false,
                    statements: Vec::new(),
                    terminator: Terminator::Call(Call {
                        function: Operand {
                            ty: fn_ty,
                            kind: OperandKind::Constant(Constant::ZeroSized),
                        },
                        callee: Callee::Indirect { signature: fn_ty },
                        semantic: CallSemantic::Host,
                        arguments: vec![Operand {
                            ty: struct_ty,
                            kind: OperandKind::Constant(Constant::ZeroSized),
                        }],
                        destination: Place {
                            local: LocalId(0),
                            projection: Vec::new(),
                            ty: enum_ty,
                        },
                        target: Some(BlockId(1)),
                        unwind: Unwind::Continue,
                        track_caller: None,
                        host: Some(HostPayload::Extern(BytesId(0))),
                    }),
                },
                BasicBlock {
                    id: BlockId(1),
                    cleanup: false,
                    statements: Vec::new(),
                    terminator: Terminator::Call(Call {
                        function: Operand {
                            ty: fn_ty,
                            kind: OperandKind::Constant(Constant::ZeroSized),
                        },
                        callee: Callee::Indirect { signature: fn_ty },
                        semantic: CallSemantic::Host,
                        arguments: vec![Operand {
                            ty: closure_ty,
                            kind: OperandKind::Constant(Constant::ZeroSized),
                        }],
                        destination: Place {
                            local: LocalId(0),
                            projection: Vec::new(),
                            ty: unit_ty,
                        },
                        target: Some(BlockId(2)),
                        unwind: Unwind::Continue,
                        track_caller: None,
                        host: Some(HostPayload::Marker {
                            section: host_section,
                            def: def_of(40),
                            template: None,
                            helpers: Vec::new(),
                        }),
                    }),
                },
                BasicBlock {
                    id: BlockId(2),
                    cleanup: false,
                    statements: Vec::new(),
                    terminator: Terminator::Return,
                },
            ],
        };

        let closure_body = Body {
            item: closure_item_id,
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: vec![cb_struct_ty],
                result: cb_enum_ty,
                abi: AbiSignature::default(),
            },
            locals: Vec::new(),
            blocks: Vec::new(),
        };

        let caller_name = JsName::new("internal_caller".to_owned());
        let closure_name = JsName::new("cb_closure".to_owned());
        let mut table = BTreeMap::new();
        for (name, body) in [
            (caller_name.clone(), caller_body),
            (closure_name.clone(), closure_body),
        ] {
            let mut jir = LinkJir::default();
            jir.use_interns(interns.clone());
            jir.body = Some(body);
            table.insert(
                name.clone(),
                LinkItem {
                    name,
                    kind: ItemKind::Fn,
                    js: String::new(),
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
                    jir,
                },
            );
        }
        let live: BTreeSet<JsName> = [caller_name.clone(), closure_name].into_iter().collect();
        let lowered = vec![(
            caller_name,
            Ok(LoweredItem {
                decl: Stmt::Comment(String::new()),
                refs: BTreeSet::new(),
                zombies: Vec::new(),
                loc: None,
                names: Vec::new(),
                dts: None,
                js_alloc_origins: Vec::new(),
                bigints: BTreeSet::new(),
            }),
            OptStats::default(),
            None,
        )];

        let exclusions = collect_program_exclusions(&table, &lowered, &live);
        assert!(exclusions.boundary_fields.contains("ext_ts"));
        assert!(exclusions.boundary_fields.contains("cb_ts"));
        assert!(exclusions.boundary_variants.contains("Go"));
        assert!(exclusions.boundary_variants.contains("Stop"));
        assert!(exclusions.boundary_variants.contains("CbDone"));
    }

    #[test]
    fn collect_js_text_tokens_and_strings_handles_comments_templates_escapes_and_lo_hi() {
        let mut tokens = BTreeSet::new();
        let mut strings = BTreeSet::new();
        let text = r#"
            // caller's frame shouldn't open a fake single-quoted string
            /* another's 'block' comment */
            const tpl = `TplOnly`;
            const interp = `head ${fn(inner_tok, "QuotedInTpl")}`;
            const s1 = "\u0046oo";
            const s2 = "a\"b";
            const x = { foo$lo: 1, after_comment: 2 };
        "#;
        collect_js_text_tokens_and_strings(text, &mut tokens, &mut strings);
        assert!(tokens.contains("after_comment"));
        assert!(tokens.contains("inner_tok"));
        assert!(tokens.contains("foo$lo"));
        assert!(strings.contains("Foo"));
        assert!(strings.contains("a\"b"));
        assert!(strings.contains("TplOnly"));
        assert!(strings.contains("QuotedInTpl"));

        // Ensure `foo$lo` exclusion also blocks splitting `foo`.
        let make_fn = jsast::function(
            "mk",
            vec![],
            vec![Stmt::Return(Some(Expr::Object(vec![
                ("foo".to_owned(), Expr::BigInt("42".to_owned())),
                ("bar".to_owned(), Expr::BigInt("42".to_owned())),
            ])))],
        );
        let (_, lowered, _, _, _) = test_run(
            Opts {
                optimizer: OptimizerMode::On,
                ecma_target: EcmaTarget::Es2018,
                ..Opts::default()
            },
            vec![
                ("mk", make_fn),
                (
                    "guard",
                    jsast::function(
                        "guard",
                        vec![],
                        vec![Stmt::Return(Some(Expr::Str("foo$lo".to_owned())))],
                    ),
                ),
            ],
        );
        let mk_js = jsast::stmt_to_string(&lowered[0].1.as_ref().unwrap().decl);
        assert!(
            mk_js.contains("foo:"),
            "foo$lo must block splitting foo:\n{mk_js}"
        );
        assert!(
            mk_js.contains("bar$lo:"),
            "unrelated bar should still split:\n{mk_js}"
        );
    }

    #[test]
    fn bigint_split_ignores_captured_arrow_locals_and_signed_lo_packs() {
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
        let signed_pack = Expr::Binary(
            BinOp::BitOr,
            Box::new(Expr::Binary(
                BinOp::Shl,
                Box::new(Expr::Call(
                    Box::new(Expr::Ident("BigInt".to_owned())),
                    vec![Expr::Ident("hi".to_owned())],
                )),
                Box::new(Expr::BigInt("32".to_owned())),
            )),
            Box::new(Expr::Call(
                Box::new(Expr::Ident("BigInt".to_owned())),
                vec![Expr::Num(-1.0)],
            )),
        );
        let valid_pack = Expr::Binary(
            BinOp::BitOr,
            Box::new(Expr::Binary(
                BinOp::Shl,
                Box::new(Expr::Call(
                    Box::new(Expr::Ident("BigInt".to_owned())),
                    vec![Expr::Ident("hi".to_owned())],
                )),
                Box::new(Expr::BigInt("32".to_owned())),
            )),
            Box::new(Expr::Call(
                Box::new(Expr::Ident("BigInt".to_owned())),
                vec![Expr::Binary(
                    BinOp::UShr,
                    Box::new(Expr::Ident("lo".to_owned())),
                    Box::new(Expr::Num(0.0)),
                )],
            )),
        );

        let fn_maker = jsast::function(
            "mk",
            vec!["lo".to_owned(), "hi".to_owned()],
            vec![Stmt::Return(Some(Expr::Object(vec![
                ("signed_bad".to_owned(), signed_pack),
                ("split_ok".to_owned(), valid_pack),
                ("byteset".to_owned(), Expr::BigInt("1".to_owned())),
            ])))],
        );
        let fn_capture = jsast::function(
            "capture_temp",
            vec!["obj".to_owned()],
            vec![
                Stmt::Lets(vec![("$t0".to_owned(), None)]),
                Stmt::ExprStmt(Expr::Assign(
                    Box::new(Expr::Ident("$t0".to_owned())),
                    Box::new(Expr::Member(
                        Box::new(Expr::Ident("obj".to_owned())),
                        "byteset".to_owned(),
                    )),
                )),
                Stmt::Return(Some(Expr::Arrow(
                    Vec::new(),
                    ArrowBody::Expr(Box::new(unpack_lo(Expr::Ident("$t0".to_owned())))),
                ))),
            ],
        );

        let (_, lowered, _, _, _) = test_run(
            Opts {
                optimizer: OptimizerMode::On,
                ecma_target: EcmaTarget::Es2018,
                ..Opts::default()
            },
            vec![("mk", fn_maker), ("capture_temp", fn_capture)],
        );
        let mk_js = jsast::stmt_to_string(&lowered[0].1.as_ref().unwrap().decl);
        let cap_js = jsast::stmt_to_string(&lowered[1].1.as_ref().unwrap().decl);
        assert!(
            mk_js.contains("signed_bad:"),
            "negative lo_arg must NOT split:\n{mk_js}"
        );
        assert!(
            !mk_js.contains("signed_bad$lo"),
            "negative lo_arg must NOT emit signed_bad$lo:\n{mk_js}"
        );
        assert!(
            mk_js.contains("split_ok$lo:") && mk_js.contains("split_ok$hi:"),
            "unsigned lo_arg pack should still split:\n{mk_js}"
        );
        assert!(
            mk_js.contains("byteset:") && !mk_js.contains("byteset$lo"),
            "captured temp reading obj.byteset must prevent splitting byteset:\n{mk_js}"
        );
        assert!(
            cap_js.contains("let $t0;") && !cap_js.contains("$t0$lo"),
            "captured temp inside arrow must stay declared as $t0:\n{cap_js}"
        );
    }

    #[test]
    fn read_only_nested_const_slice_hoists_and_mutated_slice_does_not() {
        let make_slice = || {
            Expr::Object(vec![
                (
                    "buf".to_owned(),
                    Expr::Array(
                        (0..4)
                            .map(|i| {
                                Expr::Array(vec![Expr::Num(i as f64), Expr::Num((i * 10) as f64)])
                            })
                            .collect(),
                    ),
                ),
                ("off".to_owned(), Expr::Num(0.0)),
                ("len".to_owned(), Expr::Num(4.0)),
            ])
        };

        // Positive: `s = { buf: [[0,0], ...], off: 0, len: 4 }; return s.buf[s.off + i][0];`
        let fn_ro = jsast::function(
            "read_iota",
            vec!["i".to_owned()],
            vec![
                Stmt::Lets(vec![("s".to_owned(), None)]),
                Stmt::ExprStmt(Expr::Assign(
                    Box::new(Expr::Ident("s".to_owned())),
                    Box::new(make_slice()),
                )),
                Stmt::Return(Some(Expr::Index(
                    Box::new(Expr::Index(
                        Box::new(Expr::Member(
                            Box::new(Expr::Ident("s".to_owned())),
                            "buf".to_owned(),
                        )),
                        Box::new(Expr::Binary(
                            BinOp::Add,
                            Box::new(Expr::Member(
                                Box::new(Expr::Ident("s".to_owned())),
                                "off".to_owned(),
                            )),
                            Box::new(Expr::Ident("i".to_owned())),
                        )),
                    )),
                    Box::new(Expr::Num(0.0)),
                ))),
            ],
        );

        // Negative: mutates `s.buf[0][0] = 99` -> MUST NOT hoist!
        let fn_mut = jsast::function(
            "write_iota",
            vec![],
            vec![
                Stmt::Lets(vec![("s".to_owned(), None)]),
                Stmt::ExprStmt(Expr::Assign(
                    Box::new(Expr::Ident("s".to_owned())),
                    Box::new(make_slice()),
                )),
                Stmt::ExprStmt(Expr::Assign(
                    Box::new(Expr::Index(
                        Box::new(Expr::Index(
                            Box::new(Expr::Member(
                                Box::new(Expr::Ident("s".to_owned())),
                                "buf".to_owned(),
                            )),
                            Box::new(Expr::Num(0.0)),
                        )),
                        Box::new(Expr::Num(0.0)),
                    )),
                    Box::new(Expr::Num(99.0)),
                )),
                Stmt::Return(Some(Expr::Member(
                    Box::new(Expr::Ident("s".to_owned())),
                    "len".to_owned(),
                ))),
            ],
        );

        let (consts, lowered, _, _, _) = test_run(
            Opts {
                optimizer: OptimizerMode::On,
                ..Opts::default()
            },
            vec![("read_iota", fn_ro), ("write_iota", fn_mut)],
        );
        let ro_js = jsast::stmt_to_string(&lowered[0].1.as_ref().unwrap().decl);
        let mut_js = jsast::stmt_to_string(&lowered[1].1.as_ref().unwrap().decl);
        assert!(
            !consts.is_empty() && ro_js.contains("$hc_"),
            "read-only nested const slice should hoist:\nconsts={consts:?}\nro={ro_js}"
        );
        assert!(
            !mut_js.contains("$hc_") && mut_js.contains("buf:"),
            "mutated nested slice must NOT hoist:\nmut={mut_js}"
        );
    }

    #[test]
    fn speed_profile_wraps_readonly_hoisted_and_static_int_tables_in_typed_arrays() {
        let crc_vals: Vec<Expr> = vec![
            0.0,
            1996959894.0,
            3993919788.0,
            2567524794.0,
            124634137.0,
            1886057615.0,
            3915621685.0,
            2657392035.0,
        ]
        .into_iter()
        .map(Expr::Num)
        .collect();
        let small_vals: Vec<Expr> = (0..8).map(|n| Expr::Num(f64::from(n))).collect();

        let static_ro = Stmt::Let("CRCT".to_owned(), Some(Expr::Array(crc_vals)));
        let static_mut = Stmt::Let("MUT_T".to_owned(), Some(Expr::Array(small_vals.clone())));
        let fn_use = jsast::function(
            "use_tables",
            vec!["i".to_owned()],
            vec![
                Stmt::Let("local_t".to_owned(), Some(Expr::Array(small_vals))),
                Stmt::ExprStmt(Expr::Assign(
                    Box::new(Expr::Index(
                        Box::new(Expr::Ident("MUT_T".to_owned())),
                        Box::new(Expr::Ident("i".to_owned())),
                    )),
                    Box::new(Expr::Num(7.0)),
                )),
                Stmt::Return(Some(Expr::Binary(
                    BinOp::BitXor,
                    Box::new(Expr::Index(
                        Box::new(Expr::Ident("CRCT".to_owned())),
                        Box::new(Expr::Ident("i".to_owned())),
                    )),
                    Box::new(Expr::Index(
                        Box::new(Expr::Ident("local_t".to_owned())),
                        Box::new(Expr::Ident("i".to_owned())),
                    )),
                ))),
            ],
        );

        let speed_opts = Opts {
            optimizer: OptimizerMode::On,
            profile: Some(crate::opts::Profile::ReleaseSpeed),
            ..Opts::default()
        };
        let (table, lowered, _, _, _) = test_run(
            speed_opts,
            vec![
                ("CRCT", static_ro),
                ("MUT_T", static_mut),
                ("use_tables", fn_use),
            ],
        );

        let hc_item = table
            .values()
            .find(|it| it.name.as_str().starts_with("$hc_"))
            .expect("local_t hoisted");
        assert!(
            hc_item.js.contains("new Uint8Array("),
            "expected local small-int table wrapped in Uint8Array under ReleaseSpeed: {}",
            hc_item.js
        );

        let crct_js = jsast::stmt_to_string(&lowered[0].1.as_ref().unwrap().decl);
        assert!(
            crct_js.starts_with("const CRCT = new Uint32Array("),
            "expected read-only static CRCT upgraded to const Uint32Array: {crct_js}"
        );

        let mut_js = jsast::stmt_to_string(&lowered[1].1.as_ref().unwrap().decl);
        assert!(
            mut_js.starts_with("let MUT_T = ["),
            "mutated static array must stay plain let Array: {mut_js}"
        );
    }
}
