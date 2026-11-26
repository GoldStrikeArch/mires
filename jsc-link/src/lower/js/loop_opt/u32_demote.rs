//! Wrapping-Int32 loop-carried register demotion (`x + z >>> 0` -> `x + z | 0`).

use std::collections::HashSet;

use rcgjs_object::jsast::{self, BinOp, Expr, Stmt, UnOp, Visit, VisitMut};

use super::super::ast_util::{exact_i32, FunctionSafetyScan};

/// Demotes `... >>> 0` to `... | 0` (or bare expression when already `Int32`) on declared locals
/// whose EVERY read across the function transitively feeds bit-exact 32-bit consumers (`|`, `&`,
/// `^`, `~`, `<<`, `>>`, `>>>`, `Math.imul`, `Math.clz32`, wrapping `+`/`-`, or read-only local
/// array literals like SHA-256's `aK = [j, p, q, x, k, r, s, y]` whose element reads feed bit-exact
/// 32-bit consumers).
pub(in super::super) fn demote_bitexact_u32_wraps(body: &mut [Stmt], params: &[String]) {
    let Some((mut var_cands, mut arr_cands)) = collect_demote_candidates(body, params) else {
        return;
    };

    loop {
        let mut removed = false;
        PruneWalk {
            var_cands: &mut var_cands,
            arr_cands: &mut arr_cands,
            removed: &mut removed,
        }
        .visit_stmts(body);
        if !removed || var_cands.is_empty() {
            break;
        }
    }
    if var_cands.is_empty() {
        return;
    }

    DemoteRewriter {
        var_cands: &var_cands,
        arr_cands: &arr_cands,
    }
    .visit_stmts_mut(body);
}

#[derive(Default)]
struct ArrAssignScan {
    arr_only: HashSet<String>,
    non_arr: HashSet<String>,
}

impl ArrAssignScan {
    fn record_assign(&mut self, name: &str, rhs: &Expr) {
        if matches!(rhs, Expr::Array(_)) {
            self.arr_only.insert(name.to_owned());
        } else {
            self.non_arr.insert(name.to_owned());
        }
    }
}

impl Visit for ArrAssignScan {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(n, Some(e)) | Stmt::Const(n, e) => {
                self.record_assign(n, e);
                self.visit_expr(e);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(e) = init {
                        self.record_assign(n, e);
                        self.visit_expr(e);
                    }
                }
            }
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    self.record_assign(n, rhs);
                }
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
            Stmt::ExprStmt(Expr::Seq(items)) => {
                for item in items {
                    if let Expr::Assign(lhs, rhs) = item {
                        if let Expr::Ident(n) = lhs.as_ref() {
                            self.record_assign(n, rhs);
                        }
                    }
                    self.visit_expr(item);
                }
            }
            Stmt::FunctionDecl { .. } => {}
            _ => jsast::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        if matches!(expr, Expr::Arrow(..)) {
            return;
        }
        jsast::walk_expr(self, expr);
    }
}

fn collect_demote_candidates(
    body: &[Stmt],
    params: &[String],
) -> Option<(HashSet<String>, HashSet<String>)> {
    let param_set: HashSet<&str> = params.iter().map(String::as_str).collect();
    let mut safety = FunctionSafetyScan::default();
    safety.visit_stmts(body);
    if safety.has_raw || safety.decl_counts.is_empty() {
        return None;
    }

    let mut arr_scan = ArrAssignScan::default();
    arr_scan.visit_stmts(body);

    let FunctionSafetyScan {
        decl_counts,
        disqualified: forbidden,
        ..
    } = safety;
    let ArrAssignScan {
        arr_only: arr_only_assigned,
        non_arr: non_arr_assigned,
    } = arr_scan;

    let mut var_cands: HashSet<String> = HashSet::new();
    let mut arr_cands: HashSet<String> = HashSet::new();
    for (name, count) in decl_counts {
        if count != 1 || param_set.contains(name.as_str()) || forbidden.contains(&name) {
            continue;
        }
        if arr_only_assigned.contains(&name) && !non_arr_assigned.contains(&name) {
            arr_cands.insert(name);
        } else {
            var_cands.insert(name);
        }
    }
    (!var_cands.is_empty()).then_some((var_cands, arr_cands))
}

fn is_safe_int_add_sub_operand(e: &Expr) -> bool {
    match e {
        Expr::Ident(_) | Expr::Member(_, _) | Expr::Index(_, _) => true,
        Expr::Num(n) => {
            n.is_finite()
                && n.fract() == 0.0
                && *n >= f64::from(i32::MIN)
                && *n <= f64::from(u32::MAX)
        }
        Expr::RawNum(s) => s
            .parse::<i64>()
            .ok()
            .is_some_and(|v| v >= i64::from(i32::MIN) && v <= i64::from(u32::MAX)),
        Expr::Unary(UnOp::BitNot | UnOp::Neg, inner) => is_safe_int_add_sub_operand(inner),
        Expr::Binary(
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr | BinOp::UShr,
            _,
            _,
        ) => true,
        Expr::Binary(BinOp::Add | BinOp::Sub, l, r) => {
            is_safe_int_add_sub_operand(l) && is_safe_int_add_sub_operand(r)
        }
        Expr::Call(callee, _) => matches!(
            callee.as_ref(),
            Expr::Member(obj, prop)
                if matches!(obj.as_ref(), Expr::Ident(id) if id == "Math")
                    && matches!(prop.as_str(), "imul" | "clz32")
        ),
        Expr::Cond(_, t, f) => is_safe_int_add_sub_operand(t) && is_safe_int_add_sub_operand(f),
        _ => false,
    }
}

#[derive(Clone, Copy)]
struct PruneCtx {
    bit32: bool,
    add_depth: u8,
    val_discarded: bool,
}

impl PruneCtx {
    const NON_BIT32: Self = Self {
        bit32: false,
        add_depth: 0,
        val_discarded: false,
    };
    const BIT32: Self = Self {
        bit32: true,
        add_depth: 0,
        val_discarded: false,
    };
    const DISCARDED: Self = Self {
        bit32: false,
        add_depth: 0,
        val_discarded: true,
    };

    fn next_add(self) -> Self {
        Self {
            bit32: true,
            add_depth: self.add_depth + 1,
            val_discarded: false,
        }
    }

    fn value_used(self) -> Self {
        Self {
            val_discarded: false,
            ..self
        }
    }
}

struct PruneWalk<'a> {
    var_cands: &'a mut HashSet<String>,
    arr_cands: &'a mut HashSet<String>,
    removed: &'a mut bool,
}

impl PruneWalk<'_> {
    fn drop_var(&mut self, name: &str) {
        if self.var_cands.remove(name) {
            *self.removed = true;
        }
    }

    fn drop_arr(&mut self, name: &str) {
        if self.arr_cands.remove(name) {
            *self.removed = true;
        }
    }

    fn prune_binding(&mut self, name: &str, rhs: &Expr) {
        if self.var_cands.contains(name) {
            self.prune_expr(rhs, PruneCtx::BIT32);
        } else if self.arr_cands.contains(name) {
            if let Expr::Array(items) = rhs {
                for item in items {
                    self.prune_expr(item, PruneCtx::BIT32);
                }
            } else {
                self.drop_arr(name);
                self.prune_expr(rhs, PruneCtx::NON_BIT32);
            }
        } else {
            self.prune_expr(rhs, PruneCtx::NON_BIT32);
        }
    }

    fn prune_assign(&mut self, lhs: &Expr, rhs: &Expr, ctx: PruneCtx) {
        if let Expr::Ident(dst) = lhs {
            if !ctx.val_discarded && !ctx.bit32 {
                self.drop_var(dst);
                self.drop_arr(dst);
            }
            self.prune_binding(dst, rhs);
        } else {
            if let Expr::Index(obj, _) = lhs {
                if let Expr::Ident(arr) = obj.as_ref() {
                    self.drop_arr(arr);
                }
            }
            self.prune_expr(lhs, PruneCtx::NON_BIT32);
            self.prune_expr(rhs, PruneCtx::NON_BIT32);
        }
    }

    fn prune_compound_assign(&mut self, lhs: &Expr, rhs: &Expr) {
        if let Expr::Ident(dst) = lhs {
            self.drop_var(dst);
            self.drop_arr(dst);
        }
        if let Expr::Index(obj, _) = lhs {
            if let Expr::Ident(arr) = obj.as_ref() {
                self.drop_arr(arr);
            }
        }
        self.prune_expr(lhs, PruneCtx::NON_BIT32);
        self.prune_expr(rhs, PruneCtx::NON_BIT32);
    }

    fn prune_seq(&mut self, items: &[Expr], ctx: PruneCtx) {
        let last_idx = items.len().saturating_sub(1);
        for (idx, item) in items.iter().enumerate() {
            let item_ctx = if idx == last_idx && !ctx.val_discarded {
                ctx
            } else {
                PruneCtx::DISCARDED
            };
            self.prune_expr(item, item_ctx);
        }
    }

    fn prune_expr(&mut self, e: &Expr, ctx: PruneCtx) {
        match e {
            Expr::Ident(id) => {
                self.drop_arr(id);
                if !ctx.bit32 {
                    self.drop_var(id);
                }
            }
            Expr::Index(obj, idx) => {
                self.prune_expr(idx, PruneCtx::NON_BIT32);
                if let Expr::Ident(arr) = obj.as_ref() {
                    if self.arr_cands.contains(arr) {
                        if !ctx.bit32 {
                            self.drop_arr(arr);
                        }
                        return;
                    }
                }
                self.prune_expr(obj, PruneCtx::NON_BIT32);
            }
            Expr::Member(obj, prop) => {
                if prop == "length" {
                    if let Expr::Ident(arr) = obj.as_ref() {
                        if self.arr_cands.contains(arr) {
                            return;
                        }
                    }
                }
                self.prune_expr(obj, PruneCtx::NON_BIT32);
            }
            Expr::Binary(op, l, r) => match op {
                BinOp::BitAnd
                | BinOp::BitOr
                | BinOp::BitXor
                | BinOp::Shl
                | BinOp::Shr
                | BinOp::UShr => {
                    self.prune_expr(l, PruneCtx::BIT32);
                    self.prune_expr(r, PruneCtx::BIT32);
                }
                BinOp::Add | BinOp::Sub
                    if ctx.bit32
                        && ctx.add_depth < 8
                        && is_safe_int_add_sub_operand(l)
                        && is_safe_int_add_sub_operand(r) =>
                {
                    let next = ctx.next_add();
                    self.prune_expr(l, next);
                    self.prune_expr(r, next);
                }
                _ => {
                    self.prune_expr(l, PruneCtx::NON_BIT32);
                    self.prune_expr(r, PruneCtx::NON_BIT32);
                }
            },
            Expr::Unary(op, inner) => match op {
                UnOp::BitNot => self.prune_expr(inner, PruneCtx::BIT32),
                UnOp::Neg
                    if ctx.bit32 && ctx.add_depth < 8 && is_safe_int_add_sub_operand(inner) =>
                {
                    self.prune_expr(inner, ctx.next_add());
                }
                _ => self.prune_expr(inner, PruneCtx::NON_BIT32),
            },
            Expr::Call(callee, args) => {
                let is_bit32_math = matches!(
                    callee.as_ref(),
                    Expr::Member(obj, prop)
                        if matches!(obj.as_ref(), Expr::Ident(id) if id == "Math")
                            && ((prop == "imul" && args.len() == 2)
                                || (prop == "clz32" && args.len() == 1))
                );
                if is_bit32_math {
                    for a in args {
                        self.prune_expr(a, PruneCtx::BIT32);
                    }
                } else {
                    self.prune_expr(callee, PruneCtx::NON_BIT32);
                    for a in args {
                        self.prune_expr(a, PruneCtx::NON_BIT32);
                    }
                }
            }
            Expr::New(callee, args) => {
                self.prune_expr(callee, PruneCtx::NON_BIT32);
                for a in args {
                    self.prune_expr(a, PruneCtx::NON_BIT32);
                }
            }
            Expr::Cond(c, t, f) => {
                self.prune_expr(c, PruneCtx::NON_BIT32);
                let branch_ctx = ctx.value_used();
                self.prune_expr(t, branch_ctx);
                self.prune_expr(f, branch_ctx);
            }
            Expr::Assign(lhs, rhs) => self.prune_assign(lhs, rhs, ctx),
            Expr::CompoundAssign(_, lhs, rhs) => self.prune_compound_assign(lhs, rhs),
            Expr::Array(items) => {
                for item in items {
                    self.prune_expr(item, PruneCtx::NON_BIT32);
                }
            }
            Expr::Seq(items) => self.prune_seq(items, ctx),
            Expr::Object(entries) => {
                for (_, v) in entries {
                    self.prune_expr(v, PruneCtx::NON_BIT32);
                }
            }
            Expr::Raw(_) if !self.var_cands.is_empty() || !self.arr_cands.is_empty() => {
                self.var_cands.clear();
                self.arr_cands.clear();
                *self.removed = true;
            }
            _ => {}
        }
    }
}

impl Visit for PruneWalk<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::ExprStmt(e) => self.prune_expr(e, PruneCtx::DISCARDED),
            Stmt::Let(n, Some(rhs)) | Stmt::Const(n, rhs) => self.prune_binding(n, rhs),
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(rhs) = init {
                        self.prune_binding(n, rhs);
                    }
                }
            }
            Stmt::FunctionDecl { .. } => {}
            _ => jsast::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        self.prune_expr(expr, PruneCtx::NON_BIT32);
    }
}

fn demote_rhs(expr: &mut Expr) {
    match expr {
        Expr::Binary(BinOp::UShr, l, r) if exact_i32(r) == Some(0) => {
            if super::super::post_opt::is_already_i32_expr(l) {
                *expr = (**l).clone();
            } else if !matches!(l.as_ref(), Expr::Binary(BinOp::UShr, _, _)) {
                *expr = Expr::Binary(BinOp::BitOr, l.clone(), Box::new(Expr::Num(0.0)));
            }
        }
        Expr::Cond(_, t, f) => {
            demote_rhs(t);
            demote_rhs(f);
        }
        Expr::Seq(items) => {
            if let Some(last) = items.last_mut() {
                demote_rhs(last);
            }
        }
        _ => {}
    }
}

struct DemoteRewriter<'a> {
    var_cands: &'a HashSet<String>,
    arr_cands: &'a HashSet<String>,
}

impl DemoteRewriter<'_> {
    fn demote_target(&self, name: &str, rhs: &mut Expr) {
        if self.var_cands.contains(name) {
            demote_rhs(rhs);
        } else if self.arr_cands.contains(name) {
            if let Expr::Array(items) = rhs {
                for it in items {
                    demote_rhs(it);
                }
            }
        }
    }
}

impl VisitMut for DemoteRewriter<'_> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                if let Expr::Ident(dst) = lhs.as_ref() {
                    self.demote_target(dst, rhs);
                }
            }
            Stmt::ExprStmt(Expr::Seq(items)) => {
                for item in items {
                    if let Expr::Assign(lhs, rhs) = item {
                        if let Expr::Ident(dst) = lhs.as_ref() {
                            self.demote_target(dst, rhs);
                        }
                    }
                }
            }
            Stmt::Let(n, Some(rhs)) | Stmt::Const(n, rhs) => self.demote_target(n, rhs),
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(rhs) = init {
                        self.demote_target(n, rhs);
                    }
                }
            }
            Stmt::FunctionDecl { .. } => {}
            _ => jsast::walk_stmt_mut(self, stmt),
        }
    }

    fn visit_expr_mut(&mut self, _: &mut Expr) {}
}
