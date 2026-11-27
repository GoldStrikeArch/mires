//! Miscellaneous post-lowering JavaScript AST cleanup passes:
//! - local `bytes_str` / `bstr_in` round-trip fusion and folding;
//! - unread local write elimination;
//! - ephemeral `__rt.box` deref collapse;
//! - single-slot array cell unboxing.

use std::collections::{HashMap, HashSet};

use rcgjs_object::jsast::{
    self, walk_expr, walk_expr_mut, walk_stmt, walk_stmt_mut, ArrowBody, Expr, Stmt, Visit,
    VisitMut,
};
use rcgjs_object::ptr;

use super::super::ast_util;

fn seq_tail(expr: &Expr) -> Option<&Expr> {
    match expr {
        Expr::Seq(items) => items.last(),
        other => Some(other),
    }
}

fn rt_unary_call<'a>(expr: &'a Expr, method: &str) -> Option<&'a Expr> {
    let Expr::Call(callee, args) = expr else {
        return None;
    };
    let [arg] = args.as_slice() else {
        return None;
    };
    let Expr::Member(obj, name) = callee.as_ref() else {
        return None;
    };
    (name == method && matches!(obj.as_ref(), Expr::Ident(rt) if rt == "__rt")).then_some(arg)
}

fn str_view_call_target(expr: &Expr) -> Option<(&'static str, &str)> {
    let Expr::Call(callee, args) = expr else {
        return None;
    };
    let [Expr::Ident(arg)] = args.as_slice() else {
        return None;
    };
    let Expr::Member(obj, method) = callee.as_ref() else {
        return None;
    };
    if !matches!(obj.as_ref(), Expr::Ident(rt) if rt == "__rt") {
        return None;
    }
    match method.as_str() {
        "str_bytes" => Some(("str_bytes", arg.as_str())),
        "str_len" => Some(("str_len", arg.as_str())),
        _ => None,
    }
}

fn str_bytes_call_target(expr: &Expr) -> Option<(&'static str, &str)> {
    let Expr::Ident(arg) = rt_unary_call(expr, "str_bytes")? else {
        return None;
    };
    Some(("str_bytes", arg.as_str()))
}

struct LocalFusePolicy {
    peel_rhs: fn(&Expr) -> bool,
    read_call: fn(&Expr) -> Option<(&'static str, &str)>,
    allow_early_exit: bool,
}

struct FuseReadCounter<'a> {
    target: &'a str,
    policy: &'a LocalFusePolicy,
    total: usize,
}

impl Visit for FuseReadCounter<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if matches!(stmt, Stmt::FunctionDecl { .. }) {
            return;
        }
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        if let Some((_, arg)) = (self.policy.read_call)(expr) {
            self.total += usize::from(arg == self.target);
            return;
        }
        if matches!(expr, Expr::Arrow(_, _)) {
            return;
        }
        walk_expr(self, expr);
    }
}

fn count_reads_in_expr(expr: &Expr, target: &str, policy: &LocalFusePolicy) -> usize {
    let mut c = FuseReadCounter {
        target,
        policy,
        total: 0,
    };
    c.visit_expr(expr);
    c.total
}

fn count_reads_in_stmts(stmts: &[Stmt], target: &str, policy: &LocalFusePolicy) -> usize {
    let mut c = FuseReadCounter {
        target,
        policy,
        total: 0,
    };
    c.visit_stmts(stmts);
    c.total
}

fn stmt_writes_target(stmt: &Stmt, target: &str, policy: &LocalFusePolicy) -> bool {
    match stmt {
        Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
            matches!(lhs.as_ref(), Expr::Ident(n) if n == target) && (policy.peel_rhs)(rhs)
        }
        Stmt::Let(name, Some(init)) => name == target && (policy.peel_rhs)(init),
        Stmt::Lets(decls) => decls
            .iter()
            .any(|(name, init)| name == target && init.as_ref().is_some_and(policy.peel_rhs)),
        _ => false,
    }
}

fn expr_is_pure_until_last_str_bytes(
    expr: &Expr,
    target: &str,
    remaining: &mut usize,
    policy: &LocalFusePolicy,
) -> bool {
    if *remaining == 0 {
        return true;
    }
    if let Some((method, arg)) = (policy.read_call)(expr) {
        if arg == target {
            if method == "str_bytes" {
                *remaining = remaining.saturating_sub(1);
            }
            return true;
        }
    }
    match expr {
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null
        | Expr::Ident(_)
        | Expr::Arrow(..) => true,
        Expr::Member(obj, _) | Expr::Unary(_, obj) => {
            expr_is_pure_until_last_str_bytes(obj, target, remaining, policy)
        }
        Expr::Index(obj, idx) => {
            expr_is_pure_until_last_str_bytes(obj, target, remaining, policy)
                && expr_is_pure_until_last_str_bytes(idx, target, remaining, policy)
        }
        Expr::Binary(op, obj, idx) => {
            if !expr_is_pure_until_last_str_bytes(obj, target, remaining, policy) {
                return false;
            }
            if *remaining == 0 {
                return true;
            }
            if !policy.allow_early_exit && matches!(op, jsast::BinOp::And | jsast::BinOp::Or) {
                // Short-circuit RHS is not guaranteed to run `bstr_utf8_bytes`'s TypeError check.
                let before = *remaining;
                return expr_is_pure_until_last_str_bytes(idx, target, remaining, policy)
                    && *remaining == before;
            }
            expr_is_pure_until_last_str_bytes(idx, target, remaining, policy)
        }
        Expr::Cond(c, t, e) => {
            if !expr_is_pure_until_last_str_bytes(c, target, remaining, policy) {
                return false;
            }
            if *remaining == 0 {
                return true;
            }
            // Disallow impure calls/writes on either arm while str_bytes reads remain.
            let mut rem_t = *remaining;
            let mut rem_e = *remaining;
            let ok_t = expr_is_pure_until_last_str_bytes(t, target, &mut rem_t, policy);
            let ok_e = expr_is_pure_until_last_str_bytes(e, target, &mut rem_e, policy);
            if !policy.allow_early_exit {
                // With strict validation preservation, neither arm may conditionally consume the
                // target read before it runs unconditionally later.
                return ok_t && ok_e && rem_t == *remaining && rem_e == *remaining;
            }
            *remaining = (*remaining).saturating_sub((*remaining - rem_t) + (*remaining - rem_e));
            ok_t && ok_e && *remaining == 0
        }
        Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
            let lhs_pure = match lhs.as_ref() {
                Expr::Ident(_) => true,
                Expr::Member(obj, _) => {
                    expr_is_pure_until_last_str_bytes(obj, target, remaining, policy)
                }
                Expr::Index(obj, idx) => {
                    expr_is_pure_until_last_str_bytes(obj, target, remaining, policy)
                        && expr_is_pure_until_last_str_bytes(idx, target, remaining, policy)
                }
                _ => false,
            };
            if !lhs_pure || !expr_is_pure_until_last_str_bytes(rhs, target, remaining, policy) {
                return false;
            }
            // A Member/Index store executes after evaluating lhs and rhs; unsafe if any
            // str_bytes(target) still runs later.
            !matches!(lhs.as_ref(), Expr::Member(..) | Expr::Index(..)) || *remaining == 0
        }
        Expr::Call(callee, args) | Expr::New(callee, args) => {
            if !expr_is_pure_until_last_str_bytes(callee, target, remaining, policy) {
                return false;
            }
            for a in args {
                if !expr_is_pure_until_last_str_bytes(a, target, remaining, policy) {
                    return false;
                }
            }
            *remaining == 0
        }
        Expr::Object(entries) => entries
            .iter()
            .all(|(_, v)| expr_is_pure_until_last_str_bytes(v, target, remaining, policy)),
        Expr::Array(items) | Expr::Seq(items) => items
            .iter()
            .all(|x| expr_is_pure_until_last_str_bytes(x, target, remaining, policy)),
        Expr::Raw(_) => false,
    }
}

fn stmts_is_pure_until_last_str_bytes(
    stmts: &[Stmt],
    target: &str,
    remaining: &mut usize,
    policy: &LocalFusePolicy,
) -> bool {
    for stmt in stmts {
        if *remaining == 0 {
            return true;
        }
        let ok = match stmt {
            Stmt::ExprStmt(e) | Stmt::Const(_, e) | Stmt::Let(_, Some(e)) => {
                expr_is_pure_until_last_str_bytes(e, target, remaining, policy)
            }
            Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                expr_is_pure_until_last_str_bytes(e, target, remaining, policy)
                    && (policy.allow_early_exit || *remaining == 0)
            }
            Stmt::Let(_, None) | Stmt::Import { .. } | Stmt::Comment(_) | Stmt::Loc(_) => true,
            Stmt::Return(None) | Stmt::Break(_) | Stmt::Continue(_) => policy.allow_early_exit,
            Stmt::Lets(decls) => decls.iter().all(|(_, init)| {
                init.as_ref()
                    .is_none_or(|e| expr_is_pure_until_last_str_bytes(e, target, remaining, policy))
            }),
            Stmt::Block(b) => stmts_is_pure_until_last_str_bytes(b, target, remaining, policy),
            Stmt::Labeled(_, inner) => stmts_is_pure_until_last_str_bytes(
                std::slice::from_ref(inner.as_ref()),
                target,
                remaining,
                policy,
            ),
            Stmt::If(..)
            | Stmt::Switch(..)
            | Stmt::LoopForever(..)
            | Stmt::While(..)
            | Stmt::TryCatch { .. }
            | Stmt::FunctionDecl { .. }
            | Stmt::Raw(_) => false,
        };
        if !ok {
            return false;
        }
    }
    *remaining == 0
}

fn write_dominates_and_safe(
    stmts: &[Stmt],
    target: &str,
    total_reads: usize,
    str_bytes_reads: usize,
    policy: &LocalFusePolicy,
) -> Option<bool> {
    for (idx, stmt) in stmts.iter().enumerate() {
        if stmt_writes_target(stmt, target, policy) {
            let mut remaining = str_bytes_reads;
            let mut pre_reads = 0usize;
            if !policy.allow_early_exit {
                if let Stmt::Lets(decls) = stmt {
                    if let Some(pos) = decls.iter().position(|(name, init)| {
                        name == target && init.as_ref().is_some_and(policy.peel_rhs)
                    }) {
                        for (_, init) in &decls[pos + 1..] {
                            if let Some(init_expr) = init {
                                pre_reads += count_reads_in_expr(init_expr, target, policy);
                                if !expr_is_pure_until_last_str_bytes(
                                    init_expr,
                                    target,
                                    &mut remaining,
                                    policy,
                                ) {
                                    return Some(false);
                                }
                            }
                        }
                    }
                }
            }
            let tail = &stmts[idx + 1..];
            if pre_reads + count_reads_in_stmts(tail, target, policy) != total_reads {
                return Some(false);
            }
            if str_bytes_reads == 0 {
                return Some(true);
            }
            return Some(stmts_is_pure_until_last_str_bytes(
                tail,
                target,
                &mut remaining,
                policy,
            ));
        }
        let nested = match stmt {
            Stmt::If(_, a, b) => write_dominates_and_safe(
                a,
                target,
                total_reads,
                str_bytes_reads,
                policy,
            )
            .or_else(|| {
                b.as_deref().and_then(|s| {
                    write_dominates_and_safe(s, target, total_reads, str_bytes_reads, policy)
                })
            }),
            Stmt::Switch(_, cases) => cases.iter().find_map(|c| {
                write_dominates_and_safe(&c.body, target, total_reads, str_bytes_reads, policy)
            }),
            Stmt::Labeled(_, inner) => write_dominates_and_safe(
                std::slice::from_ref(inner.as_ref()),
                target,
                total_reads,
                str_bytes_reads,
                policy,
            ),
            Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::While(_, b) => {
                write_dominates_and_safe(b, target, total_reads, str_bytes_reads, policy)
            }
            Stmt::TryCatch { body, handler, .. } => {
                write_dominates_and_safe(body, target, total_reads, str_bytes_reads, policy)
                    .or_else(|| {
                        write_dominates_and_safe(
                            handler,
                            target,
                            total_reads,
                            str_bytes_reads,
                            policy,
                        )
                    })
            }
            _ => None,
        };
        if nested.is_some() {
            return nested;
        }
    }
    None
}

#[derive(Default)]
struct LocalFuseUsage {
    writes: usize,
    reads: usize,
    str_bytes_reads: usize,
    disqualified: bool,
}

struct LocalFuseScanner<'a> {
    locals: &'a HashSet<String>,
    policy: &'a LocalFusePolicy,
    usage: HashMap<String, LocalFuseUsage>,
}

impl LocalFuseScanner<'_> {
    fn record_write(&mut self, name: &str, rhs: &Expr) {
        let entry = self.usage.entry(name.to_owned()).or_default();
        if (self.policy.peel_rhs)(rhs) {
            entry.writes += 1;
        } else {
            entry.disqualified = true;
        }
    }

    fn scan_nested_scope(&mut self, params: &[String], f: impl FnOnce(&mut LocalFuseScanner<'_>)) {
        let shadow: HashSet<String> = self
            .locals
            .iter()
            .filter(|n| !params.contains(*n))
            .cloned()
            .collect();
        let mut inner = LocalFuseScanner {
            locals: &shadow,
            policy: self.policy,
            usage: HashMap::new(),
        };
        f(&mut inner);
        for (name, u) in inner.usage {
            if u.writes > 0 || u.reads > 0 || u.disqualified {
                self.usage.entry(name).or_default().disqualified = true;
            }
        }
    }
}

impl Visit for LocalFuseScanner<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) if matches!(lhs.as_ref(), Expr::Ident(n) if self.locals.contains(n)) =>
            {
                let Expr::Ident(name) = lhs.as_ref() else {
                    unreachable!()
                };
                self.record_write(name, rhs);
                self.visit_expr(rhs);
            }
            Stmt::Let(name, Some(init)) if self.locals.contains(name) => {
                self.record_write(name, init);
                self.visit_expr(init);
            }
            Stmt::Lets(decls) => {
                for (name, init) in decls {
                    if let Some(init) = init {
                        if self.locals.contains(name) {
                            self.record_write(name, init);
                        }
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
                    self.usage.entry(binding.clone()).or_default().disqualified = true;
                }
                self.visit_stmts(body);
                self.visit_stmts(handler);
            }
            Stmt::FunctionDecl { params, body, .. } => {
                self.scan_nested_scope(params, |inner| inner.visit_stmts(body));
            }
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        if let Some((method, arg)) = (self.policy.read_call)(expr) {
            if self.locals.contains(arg) {
                let entry = self.usage.entry(arg.to_owned()).or_default();
                entry.reads += 1;
                if method == "str_bytes" {
                    entry.str_bytes_reads += 1;
                }
                return;
            }
        }
        match expr {
            Expr::Ident(name) if self.locals.contains(name) => {
                self.usage.entry(name.clone()).or_default().disqualified = true;
            }
            Expr::Arrow(params, body) => {
                self.scan_nested_scope(params, |inner| match body {
                    ArrowBody::Expr(b) => inner.visit_expr(b),
                    ArrowBody::Block(s) => inner.visit_stmts(s),
                });
            }
            _ => walk_expr(self, expr),
        }
    }
}

fn scan_local_fuse_usage(
    stmts: &[Stmt],
    locals: &HashSet<String>,
    policy: &LocalFusePolicy,
) -> HashMap<String, LocalFuseUsage> {
    let mut scan = LocalFuseScanner {
        locals,
        policy,
        usage: HashMap::new(),
    };
    scan.visit_stmts(stmts);
    scan.usage
}

struct LocalFuseRewriter<'a, F> {
    eligible: &'a HashSet<String>,
    rewrite_rhs: fn(&mut Expr),
    rewrite_read: F,
}

impl<F: Fn(&mut Expr, &HashSet<String>) -> bool> VisitMut for LocalFuseRewriter<'_, F> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) if matches!(lhs.as_ref(), Expr::Ident(n) if self.eligible.contains(n)) =>
            {
                self.visit_expr_mut(rhs);
                (self.rewrite_rhs)(rhs);
            }
            Stmt::Let(name, Some(init)) if self.eligible.contains(name) => {
                self.visit_expr_mut(init);
                (self.rewrite_rhs)(init);
            }
            Stmt::Lets(decls) => {
                for (name, init) in decls {
                    if let Some(init) = init {
                        self.visit_expr_mut(init);
                        if self.eligible.contains(name) {
                            (self.rewrite_rhs)(init);
                        }
                    }
                }
            }
            Stmt::FunctionDecl { .. } => {}
            _ => walk_stmt_mut(self, stmt),
        }
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        if (self.rewrite_read)(expr, self.eligible) || matches!(expr, Expr::Arrow(_, _)) {
            return;
        }
        walk_expr_mut(self, expr);
    }
}

fn rewrite_local_fuse(
    stmts: &mut [Stmt],
    eligible: &HashSet<String>,
    rewrite_rhs: fn(&mut Expr),
    rewrite_read: impl Fn(&mut Expr, &HashSet<String>) -> bool,
) {
    LocalFuseRewriter {
        eligible,
        rewrite_rhs,
        rewrite_read,
    }
    .visit_stmts_mut(stmts);
}

fn peel_bytes_str(expr: &Expr) -> bool {
    let Some(tail) = seq_tail(expr) else {
        return false;
    };
    matches!(
        tail,
        Expr::Call(callee, args)
            if args.len() == 3
                && matches!(
                    callee.as_ref(),
                    Expr::Member(obj, name)
                        if name == "bytes_str" && matches!(obj.as_ref(), Expr::Ident(rt) if rt == "__rt")
                )
    )
}

fn rewrite_bytes_str_rhs(expr: &mut Expr) {
    let taken = std::mem::replace(expr, Expr::Undefined);
    let (mut pre, tail) = match taken {
        Expr::Seq(mut items) if !items.is_empty() => {
            let tail = items.pop().expect("non-empty seq");
            (items, tail)
        }
        other => (Vec::new(), other),
    };
    let slice = match super::super::ptr::bytes_str_args(tail) {
        Ok((b, o, l)) => ptr::fat_slice(b, o, l),
        Err(other) => other,
    };
    *expr = if pre.is_empty() {
        slice
    } else {
        pre.push(slice);
        jsast::seq(pre)
    };
}

/// Eliminates intra-function `let x; x = (..., __rt.bytes_str(b, o, l))` round-trips when every
/// read of `x` is `__rt.str_bytes(x)` or `__rt.str_len(x)`, rewriting `x` to hold
/// `{ buf: b, off: o, len: l }`, `__rt.str_bytes(x)` to `x`, and `__rt.str_len(x)` to `x.len`.
#[cfg(test)]
pub(in super::super) fn fuse_local_bytes_str(body: &mut [Stmt], locals: &HashSet<String>) {
    fuse_local_bytes_str_with(body, locals, true);
}

pub(in super::super) fn fuse_local_bytes_str_with(
    body: &mut [Stmt],
    locals: &HashSet<String>,
    wrap_byte_slice_ref: bool,
) {
    let policy = LocalFusePolicy {
        peel_rhs: peel_bytes_str,
        read_call: str_view_call_target,
        allow_early_exit: true,
    };
    let usage = scan_local_fuse_usage(body, locals, &policy);
    let eligible: HashSet<String> = usage
        .into_iter()
        .filter(|(name, u)| {
            !u.disqualified
                && u.writes == 1
                && u.reads > 0
                && write_dominates_and_safe(body, name, u.reads, u.str_bytes_reads, &policy)
                    == Some(true)
        })
        .map(|(name, _)| name)
        .collect();
    if !eligible.is_empty() {
        rewrite_local_fuse(body, &eligible, rewrite_bytes_str_rhs, |expr, eligible| {
            let Some((method, arg)) = str_view_call_target(expr) else {
                return false;
            };
            if !eligible.contains(arg) {
                return false;
            }
            let arg = arg.to_owned();
            *expr = if method == "str_bytes" {
                if wrap_byte_slice_ref {
                    jsast::rt_call("byte_slice_ref", vec![Expr::Ident(arg)])
                } else {
                    Expr::Ident(arg)
                }
            } else {
                ptr::slot_len(Expr::Ident(arg))
            };
            true
        });
    }
}

struct BstrInBytesFolder;

impl VisitMut for BstrInBytesFolder {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        walk_expr_mut(self, expr);
        let is_bytes_call = rt_unary_call(expr, "str_bytes").is_some()
            || rt_unary_call(expr, "bstr_bytes").is_some();
        if !is_bytes_call {
            return;
        }
        let Expr::Call(_, args) = expr else {
            return;
        };
        let inner = std::mem::replace(&mut args[0], Expr::Undefined);
        match inner {
            Expr::Call(ref c, ref inner_args)
                if inner_args.len() == 1
                    && matches!(
                        c.as_ref(),
                        Expr::Member(obj, name)
                            if name == "bstr_in" && matches!(obj.as_ref(), Expr::Ident(rt) if rt == "__rt")
                    ) =>
            {
                let mut inner_args = match inner {
                    Expr::Call(_, a) => a,
                    _ => unreachable!(),
                };
                *expr = jsast::rt_call("bstr_utf8_bytes", vec![inner_args.swap_remove(0)]);
            }
            Expr::Seq(mut items) => {
                if let Some(last) = items.pop() {
                    if let Some(arg) = rt_unary_call(&last, "bstr_in").cloned() {
                        items.push(jsast::rt_call("bstr_utf8_bytes", vec![arg]));
                        *expr = jsast::seq(items);
                        return;
                    }
                    items.push(last);
                }
                args[0] = Expr::Seq(items);
            }
            other => {
                args[0] = other;
            }
        }
    }
}

/// Folds nested `__rt.str_bytes(__rt.bstr_in(x))` and `__rt.bstr_bytes(__rt.bstr_in(x))` into
/// `__rt.bstr_utf8_bytes(x)` under `js-str=bytes`.
pub(crate) fn fold_bstr_in_bytes_decl(decl: &mut Stmt) {
    match decl {
        Stmt::FunctionDecl { body, .. } | Stmt::Block(body) => fold_bstr_in_bytes_stmts(body),
        _ => {}
    }
}

pub(crate) fn fold_bstr_in_bytes_stmts(stmts: &mut [Stmt]) {
    BstrInBytesFolder.visit_stmts_mut(stmts);
}

fn peel_bstr_in(expr: &Expr) -> bool {
    seq_tail(expr).is_some_and(|tail| rt_unary_call(tail, "bstr_in").is_some())
}

fn unwrap_bstr_in_rhs(expr: &mut Expr) {
    let taken = std::mem::replace(expr, Expr::Undefined);
    let (mut pre, tail) = match taken {
        Expr::Seq(mut items) if !items.is_empty() => {
            let tail = items.pop().expect("non-empty seq");
            (items, tail)
        }
        other => (Vec::new(), other),
    };
    let unwrapped = match tail {
        Expr::Call(c, mut args)
            if args.len() == 1
                && matches!(
                    c.as_ref(),
                    Expr::Member(obj, name)
                        if name == "bstr_in" && matches!(obj.as_ref(), Expr::Ident(rt) if rt == "__rt")
                ) =>
        {
            args.swap_remove(0)
        }
        other => other,
    };
    *expr = if pre.is_empty() {
        unwrapped
    } else {
        pre.push(unwrapped);
        jsast::seq(pre)
    };
}

/// Fuses single-write `x = __rt.bstr_in(host)` whose sole read in the body is `__rt.str_bytes(x)`
/// into `x = host; ... __rt.bstr_utf8_bytes(x)` only when the write dominates the read with no
/// intervening side effects, branches, or early exits so `__rt.bstr_in`'s `TypeError` check runs at
/// the same observable point via `__rt.bstr_utf8_bytes`.
pub(super) fn fuse_local_bstr_in_bytes(body: &mut [Stmt], locals: &HashSet<String>) {
    let policy = LocalFusePolicy {
        peel_rhs: peel_bstr_in,
        read_call: str_bytes_call_target,
        allow_early_exit: false,
    };
    let usage = scan_local_fuse_usage(body, locals, &policy);
    let eligible: HashSet<String> = usage
        .into_iter()
        .filter(|(name, u)| {
            !u.disqualified
                && u.writes == 1
                && u.reads == 1
                && write_dominates_and_safe(body, name, u.reads, u.reads, &policy) == Some(true)
        })
        .map(|(name, _)| name)
        .collect();
    if !eligible.is_empty() {
        rewrite_local_fuse(body, &eligible, unwrap_bstr_in_rhs, |expr, eligible| {
            let Some(Expr::Ident(arg)) = rt_unary_call(expr, "str_bytes") else {
                return false;
            };
            if !eligible.contains(arg) {
                return false;
            }
            *expr = jsast::rt_call("bstr_utf8_bytes", vec![Expr::Ident(arg.clone())]);
            true
        });
    }
}

/// Whether `expr` is strictly side-effect-free and cannot throw even on `null`/`undefined`
/// (unlike property/index access), so a bare expression statement `expr;` can be dropped safely.
fn is_strictly_pure_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null
        | Expr::Ident(_)
        | Expr::Arrow(..) => true,
        Expr::Unary(_, inner) => is_strictly_pure_expr(inner),
        Expr::Binary(_, lhs, rhs) => is_strictly_pure_expr(lhs) && is_strictly_pure_expr(rhs),
        Expr::Cond(c, t, e) => {
            is_strictly_pure_expr(c) && is_strictly_pure_expr(t) && is_strictly_pure_expr(e)
        }
        Expr::Array(items) | Expr::Seq(items) => items.iter().all(is_strictly_pure_expr),
        Expr::Object(entries) => entries.iter().all(|(_, v)| is_strictly_pure_expr(v)),
        Expr::Member(..)
        | Expr::Index(..)
        | Expr::Call(..)
        | Expr::New(..)
        | Expr::Assign(..)
        | Expr::CompoundAssign(..)
        | Expr::Raw(_) => false,
    }
}

fn count_expr_writes(expr: &Expr, writes: &mut HashMap<String, usize>) {
    match expr {
        Expr::Assign(target, _) => {
            if let Expr::Ident(name) = target.as_ref() {
                *writes.entry(name.clone()).or_default() += 1;
            }
        }
        Expr::Seq(items) => {
            for item in items {
                count_expr_writes(item, writes);
            }
        }
        _ => {}
    }
}

fn count_simple_writes(body: &[Stmt], writes: &mut HashMap<String, usize>) {
    for stmt in body {
        match stmt {
            Stmt::ExprStmt(expr) => count_expr_writes(expr, writes),
            Stmt::If(_, then, els) => {
                count_simple_writes(then, writes);
                if let Some(els) = els {
                    count_simple_writes(els, writes);
                }
            }
            Stmt::Switch(_, cases) => {
                for case in cases {
                    count_simple_writes(&case.body, writes);
                }
            }
            Stmt::Labeled(_, inner) => count_simple_writes(std::slice::from_ref(inner), writes),
            Stmt::LoopForever(_, inner) | Stmt::While(_, inner) | Stmt::Block(inner) => {
                count_simple_writes(inner, writes);
            }
            _ => {}
        }
    }
}

fn strip_unread_expr(expr: Expr, unread: &HashSet<String>, changed: &mut bool) -> Option<Expr> {
    match expr {
        Expr::Assign(target, value) if matches!(target.as_ref(), Expr::Ident(name) if unread.contains(name)) =>
        {
            *changed = true;
            (!jsast::is_droppable(&value)).then_some(*value)
        }
        Expr::Seq(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                if let Some(kept) = strip_unread_expr(item, unread, changed) {
                    out.push(kept);
                }
            }
            match out.len() {
                0 => None,
                1 => out.pop(),
                _ => Some(Expr::Seq(out)),
            }
        }
        other if is_strictly_pure_expr(&other) => {
            *changed = true;
            None
        }
        other => Some(other),
    }
}

fn remove_unread_writes(body: &mut Vec<Stmt>, unread: &HashSet<String>) -> bool {
    let mut changed = false;
    let mut kept = Vec::with_capacity(body.len());
    for mut stmt in std::mem::take(body) {
        match &mut stmt {
            Stmt::If(_, then, els) => {
                changed |= remove_unread_writes(then, unread);
                if let Some(els) = els {
                    changed |= remove_unread_writes(els, unread);
                }
            }
            Stmt::Switch(_, cases) => {
                for case in cases {
                    changed |= remove_unread_writes(&mut case.body, unread);
                }
            }
            Stmt::Labeled(_, inner) => {
                if let Stmt::Block(body) = inner.as_mut() {
                    changed |= remove_unread_writes(body, unread);
                }
            }
            Stmt::LoopForever(_, inner) | Stmt::While(_, inner) | Stmt::Block(inner) => {
                changed |= remove_unread_writes(inner, unread);
            }
            Stmt::TryCatch { body, handler, .. } => {
                changed |= remove_unread_writes(body, unread);
                changed |= remove_unread_writes(handler, unread);
            }
            _ => {}
        }
        let replacement = match stmt {
            Stmt::ExprStmt(expr) => {
                strip_unread_expr(expr, unread, &mut changed).map(Stmt::ExprStmt)
            }
            other => Some(other),
        };
        if let Some(stmt) = replacement {
            kept.push(stmt);
        }
    }
    *body = kept;
    changed
}

/// Removes assignments to compiler locals that no expression ever reads, plus strictly-pure bare
/// expression statements (such as dead `select_unpredictable` ternaries `cond ? a : b;`). JIR
/// values are SSA, but range/projection cleanup and lowered hints can leave pure expressions
/// behind after the last JIR DCE run. At this point the structured JavaScript is authoritative, so
/// a function-wide zero-read test is both smaller and more exact than trying to predict the
/// structurizer's final spelling.
pub(super) fn eliminate_dead_local_writes(body: &mut Vec<Stmt>, locals: &HashSet<String>) {
    loop {
        let mut mentions: HashMap<String, usize> = HashMap::new();
        for stmt in body.iter() {
            jsast::visit_idents(stmt, &mut |name| {
                *mentions.entry(name.to_owned()).or_default() += 1;
            });
        }
        let mut writes: HashMap<String, usize> = HashMap::new();
        count_simple_writes(body, &mut writes);
        let unread: HashSet<String> = writes
            .iter()
            .filter(|(name, count)| {
                locals.contains(*name) && mentions.get(*name).copied().unwrap_or(0) == **count
            })
            .map(|(name, _)| name.clone())
            .collect();
        if !remove_unread_writes(body, &unread) {
            return;
        }
    }
}

fn is_rt_box_call(expr: &Expr) -> bool {
    ast_util::rt_call(expr).is_some_and(|(m, args)| m == "box" && args.len() == 1)
}

fn is_slot_deref_of(expr: &Expr, tmp: &str) -> bool {
    let Expr::Index(buf, off) = expr else {
        return false;
    };
    matches!(
        (buf.as_ref(), off.as_ref()),
        (Expr::Member(b_obj, b_prop), Expr::Member(o_obj, o_prop))
            if b_prop == rcgjs_object::ptr::BUF
                && o_prop == rcgjs_object::ptr::OFF
                && matches!(b_obj.as_ref(), Expr::Ident(a) if a == tmp)
                && matches!(o_obj.as_ref(), Expr::Ident(b) if b == tmp)
    )
}

struct EphemeralBoxCollapser<'a> {
    locals: &'a HashSet<String>,
    mentions: &'a HashMap<String, usize>,
}

impl VisitMut for EphemeralBoxCollapser<'_> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        if matches!(stmt, Stmt::FunctionDecl { .. }) {
            return;
        }
        walk_stmt_mut(self, stmt);
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        walk_expr_mut(self, expr);
        if let Expr::Seq(items) = expr {
            if items.len() >= 2 {
                let n = items.len();
                let can_collapse = match (&items[n - 2], &items[n - 1]) {
                    (Expr::Assign(lhs, rhs), tail) => matches!(
                        lhs.as_ref(),
                        Expr::Ident(tmp)
                            if self.locals.contains(tmp)
                                && self.mentions.get(tmp).copied() == Some(3)
                                && is_rt_box_call(rhs)
                                && is_slot_deref_of(tail, tmp)
                    ),
                    _ => false,
                };
                if can_collapse {
                    items.pop();
                    let assign = items.pop().expect("assign");
                    if let Expr::Assign(_, rhs) = assign {
                        if let Expr::Call(_, mut args) = *rhs {
                            let inner = args.pop().expect("1 arg");
                            items.push(inner);
                        }
                    }
                    if items.len() == 1 {
                        *expr = items.pop().expect("single");
                    }
                }
            }
        }
    }
}

/// Collapses ephemeral `(tmp = __rt.box(inner), tmp.buf[tmp.off])` expressions into `inner` when
/// `tmp` is an uninitialized compiler temporary used nowhere else in the function body.
pub(super) fn collapse_ephemeral_rt_box_deref(body: &mut [Stmt], locals: &HashSet<String>) {
    let mut mentions: HashMap<String, usize> = HashMap::new();
    for stmt in body.iter() {
        jsast::visit_idents(stmt, &mut |name| {
            *mentions.entry(name.to_owned()).or_default() += 1;
        });
    }
    EphemeralBoxCollapser {
        locals,
        mentions: &mentions,
    }
    .visit_stmts_mut(body);
}

struct SingleSlotArrayScan<'a> {
    candidates: &'a HashSet<String>,
    in_closure: bool,
    disqualified: HashSet<String>,
    slot_uses: HashSet<String>,
}

impl Visit for SingleSlotArrayScan<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Const(name, _) | Stmt::Let(name, _) if self.candidates.contains(name) => {
                self.disqualified.insert(name.clone());
            }
            Stmt::Lets(decls) => {
                for (name, _) in decls {
                    if self.candidates.contains(name) {
                        self.disqualified.insert(name.clone());
                    }
                }
            }
            Stmt::TryCatch { binding, .. } if self.candidates.contains(binding) => {
                self.disqualified.insert(binding.clone());
            }
            Stmt::FunctionDecl { body, .. } => {
                let prev = self.in_closure;
                self.in_closure = true;
                self.visit_stmts(body);
                self.in_closure = prev;
                return;
            }
            _ => {}
        }
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        if !self.in_closure {
            if let Expr::Index(obj, idx) = expr {
                if let Expr::Ident(name) = obj.as_ref() {
                    if self.candidates.contains(name) && ast_util::is_zero_num(idx) {
                        self.slot_uses.insert(name.clone());
                        return;
                    }
                }
            }
        }
        match expr {
            Expr::Ident(name) if self.candidates.contains(name) => {
                self.disqualified.insert(name.clone());
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

struct SingleSlotArrayRewriter<'a>(&'a HashSet<String>);

impl VisitMut for SingleSlotArrayRewriter<'_> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        if matches!(stmt, Stmt::FunctionDecl { .. }) {
            return;
        }
        walk_stmt_mut(self, stmt);
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        if let Expr::Index(obj, idx) = expr {
            if let Expr::Ident(name) = obj.as_ref() {
                if self.0.contains(name) && ast_util::is_zero_num(idx) {
                    *expr = Expr::Ident(name.clone());
                    return;
                }
            }
        }
        if matches!(expr, Expr::Arrow(_, _)) {
            return;
        }
        walk_expr_mut(self, expr);
    }
}

/// Unboxes compiler-declared single-slot array cells (`let m = [];`) into plain scalars (`let m;`)
/// when every mention of `m` in `body` is strictly `m[0]` (read or write) outside any closure or
/// nested function. This cleans up boxed cells whose `{ buf: m, off: 0 }` views became dead after
/// lowering (such as `core::hint::select_unpredictable` in `slice::partition_point`).
pub(super) fn unbox_single_slot_array_locals(
    body: &mut [Stmt],
    declarations: &mut [(String, Option<Expr>)],
) {
    let candidates: HashSet<String> = declarations
        .iter()
        .filter(|(_, init)| matches!(init, Some(Expr::Array(items)) if items.is_empty()))
        .map(|(name, _)| name.clone())
        .collect();
    if candidates.is_empty() {
        return;
    }
    let mut scan = SingleSlotArrayScan {
        candidates: &candidates,
        in_closure: false,
        disqualified: HashSet::new(),
        slot_uses: HashSet::new(),
    };
    scan.visit_stmts(body);
    let SingleSlotArrayScan {
        disqualified,
        slot_uses,
        ..
    } = scan;
    let unbox: HashSet<String> = candidates
        .into_iter()
        .filter(|name| !disqualified.contains(name) && slot_uses.contains(name))
        .collect();
    if unbox.is_empty() {
        return;
    }
    SingleSlotArrayRewriter(&unbox).visit_stmts_mut(body);
    for (name, init) in declarations.iter_mut() {
        if unbox.contains(name) {
            *init = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dead_local_writes_are_removed_without_dropping_effects() {
        let mut body = vec![
            jsast::assign_stmt(jsast::id("dead"), jsast::object(Vec::new())),
            jsast::assign_stmt(
                jsast::id("effect"),
                jsast::call(jsast::id("host"), Vec::new()),
            ),
            jsast::assign_stmt(jsast::id("live"), jsast::num(3.0)),
            jsast::assign_stmt(jsast::id("external"), jsast::num(4.0)),
            Stmt::If(
                jsast::id("condition"),
                vec![jsast::assign_stmt(
                    jsast::id("nested_dead"),
                    jsast::array(Vec::new()),
                )],
                None,
            ),
            Stmt::Return(Some(jsast::id("live"))),
        ];
        let locals = ["dead", "effect", "live", "nested_dead"]
            .into_iter()
            .map(str::to_owned)
            .collect();

        eliminate_dead_local_writes(&mut body, &locals);

        let printed = jsast::stmt_to_string(&Stmt::Block(body));
        assert!(!printed.contains("dead ="), "{printed}");
        assert!(!printed.contains("effect ="), "{printed}");
        assert!(!printed.contains("nested_dead ="), "{printed}");
        assert!(printed.contains("host();"), "{printed}");
        assert!(printed.contains("live = 3;"), "{printed}");
        assert!(printed.contains("external = 4;"), "{printed}");
        assert!(printed.contains("return live;"), "{printed}");
    }

    #[test]
    fn dead_local_cleanup_reaches_a_fixed_point() {
        let mut body = vec![
            jsast::assign_stmt(jsast::id("inner"), jsast::object(Vec::new())),
            jsast::assign_stmt(jsast::id("outer"), jsast::id("inner")),
        ];
        let locals = ["inner", "outer"].into_iter().map(str::to_owned).collect();

        eliminate_dead_local_writes(&mut body, &locals);

        assert!(
            body.is_empty(),
            "{}",
            jsast::stmt_to_string(&Stmt::Block(body))
        );
    }

    #[test]
    fn fuse_local_bstr_in_bytes_respects_dominance_and_purity() {
        let rt_call = |method: &str, arg: Expr| jsast::rt_call(method, vec![arg]);
        let locals: HashSet<String> = ["x".to_owned(), "y".to_owned()].into_iter().collect();

        // Pure straight-line sequence fuses cleanly.
        let mut pure_body = vec![
            jsast::assign_stmt(jsast::id("x"), rt_call("bstr_in", jsast::id("host"))),
            jsast::assign_stmt(jsast::id("y"), jsast::num(1.0)),
            Stmt::Return(Some(rt_call("str_bytes", jsast::id("x")))),
        ];
        fuse_local_bstr_in_bytes(&mut pure_body, &locals);
        let printed_pure = jsast::stmt_to_string(&Stmt::Block(pure_body));
        assert!(printed_pure.contains("x = host;"), "{printed_pure}");
        assert!(
            printed_pure.contains("__rt.bstr_utf8_bytes(x)"),
            "{printed_pure}"
        );
        assert!(!printed_pure.contains("bstr_in"), "{printed_pure}");

        // Intervening impure call blocks fusion so `bstr_in`'s TypeError runs first.
        let mut impure_body = vec![
            jsast::assign_stmt(jsast::id("x"), rt_call("bstr_in", jsast::id("host"))),
            Stmt::ExprStmt(jsast::call(jsast::id("host_log"), Vec::new())),
            Stmt::Return(Some(rt_call("str_bytes", jsast::id("x")))),
        ];
        fuse_local_bstr_in_bytes(&mut impure_body, &locals);
        let printed_impure = jsast::stmt_to_string(&Stmt::Block(impure_body));
        assert!(printed_impure.contains("bstr_in"), "{printed_impure}");
        assert!(
            !printed_impure.contains("bstr_utf8_bytes"),
            "{printed_impure}"
        );

        // Early return / conditional read / short-circuit logical op blocks fusion.
        let mut early_ret = vec![
            jsast::assign_stmt(jsast::id("x"), rt_call("bstr_in", jsast::id("host"))),
            Stmt::If(jsast::id("flag"), vec![Stmt::Return(None)], None),
            Stmt::Return(Some(rt_call("str_bytes", jsast::id("x")))),
        ];
        fuse_local_bstr_in_bytes(&mut early_ret, &locals);
        assert!(
            jsast::stmt_to_string(&Stmt::Block(early_ret)).contains("bstr_in"),
            "early return must block fusion"
        );

        let mut short_circuit = vec![
            jsast::assign_stmt(jsast::id("x"), rt_call("bstr_in", jsast::id("host"))),
            Stmt::Return(Some(Expr::Binary(
                jsast::BinOp::And,
                Box::new(jsast::id("flag")),
                Box::new(rt_call("str_bytes", jsast::id("x"))),
            ))),
        ];
        fuse_local_bstr_in_bytes(&mut short_circuit, &locals);
        assert!(
            jsast::stmt_to_string(&Stmt::Block(short_circuit)).contains("bstr_in"),
            "short-circuit And must block fusion"
        );

        let mut cond_arm = vec![
            jsast::assign_stmt(jsast::id("x"), rt_call("bstr_in", jsast::id("host"))),
            Stmt::Return(Some(Expr::Cond(
                Box::new(jsast::id("flag")),
                Box::new(rt_call("str_bytes", jsast::id("x"))),
                Box::new(jsast::num(0.0)),
            ))),
        ];
        fuse_local_bstr_in_bytes(&mut cond_arm, &locals);
        assert!(
            jsast::stmt_to_string(&Stmt::Block(cond_arm)).contains("bstr_in"),
            "conditional arm must block fusion"
        );
    }

    #[test]
    fn eliminate_dead_local_writes_strips_unread_seq_element() {
        let mut body = vec![
            Stmt::ExprStmt(Expr::Seq(vec![
                Expr::Assign(Box::new(jsast::id("dead")), Box::new(jsast::num(1.0))),
                Expr::Assign(Box::new(jsast::id("live")), Box::new(jsast::num(2.0))),
            ])),
            Stmt::Return(Some(jsast::id("live"))),
        ];
        let locals: HashSet<String> = ["dead".to_owned(), "live".to_owned()].into_iter().collect();
        eliminate_dead_local_writes(&mut body, &locals);
        let printed = jsast::stmt_to_string(&Stmt::Block(body));
        assert!(!printed.contains("dead"), "{printed}");
        assert!(printed.contains("live = 2;"), "{printed}");
    }

    #[test]
    fn dead_pure_ternary_and_single_slot_array_cells_unbox() {
        let mut declarations = vec![
            ("m".to_owned(), Some(jsast::array(Vec::new()))),
            ("k".to_owned(), Some(jsast::array(Vec::new()))),
            ("esc".to_owned(), Some(jsast::array(Vec::new()))),
            ("o".to_owned(), None),
            ("p".to_owned(), None),
            ("q".to_owned(), None),
            ("r".to_owned(), None),
            ("s".to_owned(), None),
        ];
        let local_names: HashSet<String> = declarations.iter().map(|(n, _)| n.clone()).collect();
        let mut body = vec![
            jsast::assign_stmt(
                jsast::index(jsast::id("m"), jsast::num(0.0)),
                jsast::id("b"),
            ),
            jsast::assign_stmt(
                jsast::index(jsast::id("k"), jsast::num(0.0)),
                jsast::id("e"),
            ),
            jsast::assign_stmt(
                jsast::id("o"),
                jsast::object_of(vec![("$b", jsast::id("m")), ("$o", jsast::num(0.0))]),
            ),
            jsast::assign_stmt(
                jsast::id("p"),
                jsast::object_of(vec![("$b", jsast::id("k")), ("$o", jsast::num(0.0))]),
            ),
            Stmt::ExprStmt(jsast::cond(jsast::id("j"), jsast::id("o"), jsast::id("p"))),
            jsast::assign_stmt(
                jsast::id("q"),
                jsast::index(jsast::id("m"), jsast::num(0.0)),
            ),
            jsast::assign_stmt(
                jsast::id("r"),
                jsast::index(jsast::id("k"), jsast::num(0.0)),
            ),
            jsast::assign_stmt(
                jsast::id("s"),
                jsast::cond(jsast::id("j"), jsast::id("q"), jsast::id("r")),
            ),
            jsast::assign_stmt(
                jsast::index(jsast::id("esc"), jsast::num(0.0)),
                jsast::id("s"),
            ),
            Stmt::Return(Some(jsast::object_of(vec![
                ("$b", jsast::id("esc")),
                ("$o", jsast::num(0.0)),
            ]))),
        ];
        eliminate_dead_local_writes(&mut body, &local_names);
        unbox_single_slot_array_locals(&mut body, &mut declarations);
        let printed = jsast::stmt_to_string(&Stmt::Block(body));
        assert!(!printed.contains("j ? o : p"), "{printed}");
        assert!(!printed.contains("m[0]"), "{printed}");
        assert!(!printed.contains("k[0]"), "{printed}");
        assert!(printed.contains("m = b;"), "{printed}");
        assert!(printed.contains("esc[0] = s;"), "{printed}");
        assert_eq!(declarations[0], ("m".to_owned(), None));
        assert_eq!(declarations[1], ("k".to_owned(), None));
        assert!(declarations[2].1.is_some(), "escaping cell must stay boxed");
    }

    #[test]
    fn collapse_ephemeral_rt_box_deref_elides_temp_box() {
        let locals: HashSet<String> = ["f".to_owned(), "g".to_owned(), "kept".to_owned()]
            .into_iter()
            .collect();
        let ephemeral = Expr::Seq(vec![
            Expr::Assign(
                Box::new(jsast::id("f")),
                Box::new(jsast::rt_call(
                    "box",
                    vec![jsast::member(jsast::id("obj"), "value")],
                )),
            ),
            Expr::Index(
                Box::new(jsast::member(jsast::id("f"), "$b")),
                Box::new(jsast::member(jsast::id("f"), "$o")),
            ),
        ]);
        let retained = Expr::Seq(vec![
            Expr::Assign(
                Box::new(jsast::id("kept")),
                Box::new(jsast::rt_call("box", vec![jsast::id("other")])),
            ),
            Expr::Index(
                Box::new(jsast::member(jsast::id("kept"), "$b")),
                Box::new(jsast::member(jsast::id("kept"), "$o")),
            ),
        ]);
        let mut body = vec![
            jsast::assign_stmt(jsast::id("g"), ephemeral),
            jsast::assign_stmt(jsast::id("h"), retained),
            Stmt::Return(Some(jsast::id("kept"))),
        ];
        collapse_ephemeral_rt_box_deref(&mut body, &locals);
        let printed = jsast::stmt_to_string(&Stmt::Block(body));
        assert!(printed.contains("g = obj.value;"), "{printed}");
        assert!(
            printed.contains("__rt.box(other)"),
            "multi-use boxed slot must stay intact:\n{printed}"
        );
    }
}
