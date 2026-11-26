//! Shared JavaScript AST traversal and query utilities built on [`jsast::Visit`] / [`jsast::VisitMut`].

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::ControlFlow;

use rcgjs_object::jsast::{
    self, try_walk_expr, walk_block_mut, walk_expr, walk_expr_mut, walk_stmt, walk_stmt_mut,
    ArrowBody, Expr, Stmt, TryVisit, UnOp, Visit, VisitMut,
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

/// Short TypedArray helper suffixes indexed by `TypedArrayKind as usize`.
pub const TA_SUFFIXES: [&str; 8] = ["u8", "i8", "u16", "i16", "u32", "i32", "f32", "f64"];

/// Whether `expr` is a simple atomic value (identifier or primitive literal) that is free and
/// side-effect-free to re-read.
pub fn is_simple_atom(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Ident(_)
            | Expr::Num(_)
            | Expr::RawNum(_)
            | Expr::Bool(_)
            | Expr::Str(_)
            | Expr::Undefined
            | Expr::Null
            | Expr::BigInt(_)
    )
}

/// Whether `expr` is a numeric literal equal to `0` (`0` or `RawNum("0")`).
pub fn is_zero_num(expr: &Expr) -> bool {
    matches!(expr, Expr::Num(n) if *n == 0.0) || matches!(expr, Expr::RawNum(s) if s == "0")
}

/// Whether `expr` is a numeric literal equal to `1` (`1` or `RawNum("1")`).
pub fn is_one_num(expr: &Expr) -> bool {
    matches!(expr, Expr::Num(n) if *n == 1.0) || matches!(expr, Expr::RawNum(s) if s == "1")
}

/// If `expr` is an exact IEEE-754 safe integer literal (`-(2^53 - 1)..=(2^53 - 1)`),
/// optionally negated via unary `-`, returns its `i64` value.
pub fn exact_int_lit(expr: &Expr) -> Option<i64> {
    const MAX_SAFE_INT: f64 = 9_007_199_254_740_991.0;
    const MAX_SAFE_I64: i64 = 9_007_199_254_740_991;
    match expr {
        Expr::Num(n)
            if n.is_finite() && n.fract() == 0.0 && (-MAX_SAFE_INT..=MAX_SAFE_INT).contains(n) =>
        {
            Some(*n as i64)
        }
        Expr::RawNum(s) => s
            .parse::<i64>()
            .ok()
            .filter(|v| (-MAX_SAFE_I64..=MAX_SAFE_I64).contains(v)),
        Expr::Unary(UnOp::Neg, inner) => match inner.as_ref() {
            Expr::Num(n)
                if n.is_finite()
                    && n.fract() == 0.0
                    && (-MAX_SAFE_INT..=MAX_SAFE_INT).contains(n) =>
            {
                Some(-(*n as i64))
            }
            Expr::RawNum(s) => s
                .parse::<i64>()
                .ok()
                .and_then(i64::checked_neg)
                .filter(|v| (-MAX_SAFE_I64..=MAX_SAFE_I64).contains(v)),
            _ => None,
        },
        _ => None,
    }
}

/// If `expr` is an exact signed 32-bit integer literal, returns its `i32` value.
pub fn exact_i32(expr: &Expr) -> Option<i32> {
    exact_int_lit(expr).and_then(|v| i32::try_from(v).ok())
}

/// If `expr` is an exact unsigned 32-bit integer literal, returns its `u32` value.
pub fn exact_u32(expr: &Expr) -> Option<u32> {
    exact_int_lit(expr).and_then(|v| u32::try_from(v).ok())
}

/// If `expr` is an exact non-negative safe integer literal (`0..=2^53 - 1`), returns its `u64` value.
pub fn exact_u64(expr: &Expr) -> Option<u64> {
    exact_int_lit(expr).and_then(|v| u64::try_from(v).ok())
}

/// If `expr` is an exact byte literal (`0..=255`), returns its `u8` value.
pub fn exact_u8(expr: &Expr) -> Option<u8> {
    exact_int_lit(expr).and_then(|v| u8::try_from(v).ok())
}

/// If `callee` is `__rt.<method>`, returns `Some(method)`.
pub fn rt_member(callee: &Expr) -> Option<&str> {
    match callee {
        Expr::Member(obj, method) if matches!(obj.as_ref(), Expr::Ident(rt) if rt == "__rt") => {
            Some(method.as_str())
        }
        _ => None,
    }
}

/// If `expr` is `__rt.<method>(args...)`, returns `Some((method, args))`.
pub fn rt_call(expr: &Expr) -> Option<(&str, &[Expr])> {
    let Expr::Call(callee, args) = expr else {
        return None;
    };
    Some((rt_member(callee)?, args.as_slice()))
}

/// Whether `callee` names a Rust `core::panicking::*` diverging helper.
pub fn is_panic_callee(callee: &Expr) -> bool {
    matches!(callee, Expr::Ident(name) if name.starts_with("core$panicking$"))
}

/// Whether `expr` mentions identifier `target` (conservatively `true` for `Expr::Raw`).
pub fn expr_mentions_ident(expr: &Expr, target: &str) -> bool {
    jsast::any_expr(expr, |e| match e {
        Expr::Ident(id) => id == target,
        Expr::Raw(_) => true,
        _ => false,
    })
}

/// Whether evaluating `expr` directly invokes a call, constructor (`new`), or verbatim JS fragment.
/// Does not descend into `Expr::Arrow` bodies (creating an arrow does not call it).
pub fn expr_has_call(expr: &Expr) -> bool {
    struct V;
    impl TryVisit for V {
        type Break = ();
        fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            match expr {
                Expr::Call(..) | Expr::New(..) | Expr::Raw(_) => ControlFlow::Break(()),
                Expr::Arrow(..) => ControlFlow::Continue(()),
                _ => try_walk_expr(self, expr),
            }
        }
    }
    V.visit_expr(expr).is_break()
}

/// Whether `expr` may have observable write/call side effects (assignments, non-pure calls,
/// constructors, or verbatim JS). Pure `Math.{imul, clz32, abs, min, max}` calls with
/// side-effect-free arguments count as side-effect-free; creating an `Expr::Arrow` does not
/// immediately execute its body.
pub fn expr_has_side_effect(expr: &Expr) -> bool {
    struct V;
    impl TryVisit for V {
        type Break = ();
        fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            match expr {
                Expr::Assign(..) | Expr::CompoundAssign(..) | Expr::New(..) | Expr::Raw(_) => {
                    ControlFlow::Break(())
                }
                Expr::Call(callee, args) => {
                    let is_pure_math = matches!(
                        callee.as_ref(),
                        Expr::Member(obj, prop)
                            if matches!(obj.as_ref(), Expr::Ident(m) if m == "Math")
                                && matches!(prop.as_str(), "imul" | "clz32" | "abs" | "min" | "max")
                    );
                    if !is_pure_math {
                        return ControlFlow::Break(());
                    }
                    for arg in args {
                        self.visit_expr(arg)?;
                    }
                    ControlFlow::Continue(())
                }
                Expr::Arrow(..) => ControlFlow::Continue(()),
                _ => try_walk_expr(self, expr),
            }
        }
    }
    V.visit_expr(expr).is_break()
}

/// Inverse of [`expr_has_side_effect`].
pub fn is_side_effect_free_expr(expr: &Expr) -> bool {
    !expr_has_side_effect(expr)
}

/// Whether `stmts` contains an unlabeled `break;` targeting the immediately enclosing `switch`.
pub fn has_unlabeled_switch_break(stmts: &[Stmt]) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Break(None) => return true,
            Stmt::If(_, a, b) => {
                if has_unlabeled_switch_break(a)
                    || b.as_ref().is_some_and(|b| has_unlabeled_switch_break(b))
                {
                    return true;
                }
            }
            Stmt::Block(b) => {
                if has_unlabeled_switch_break(b) {
                    return true;
                }
            }
            Stmt::Labeled(_, inner) => {
                if has_unlabeled_switch_break(std::slice::from_ref(inner.as_ref())) {
                    return true;
                }
            }
            Stmt::TryCatch { body, handler, .. } => {
                if has_unlabeled_switch_break(body) || has_unlabeled_switch_break(handler) {
                    return true;
                }
            }
            // Inner loops and switches capture their own unlabeled `break`.
            Stmt::LoopForever(..) | Stmt::While(..) | Stmt::Switch(..) => {}
            _ => {}
        }
    }
    false
}

pub fn stmt_may_exit_non_throw(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Break(_) | Stmt::Continue(_) | Stmt::Return(_) => true,
        Stmt::If(_, a, b) => {
            a.iter().any(stmt_may_exit_non_throw)
                || b.as_ref()
                    .is_some_and(|b| b.iter().any(stmt_may_exit_non_throw))
        }
        Stmt::Switch(_, cases) => cases
            .iter()
            .any(|c| c.body.iter().any(stmt_may_exit_non_throw)),
        Stmt::Block(b) | Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
            b.iter().any(stmt_may_exit_non_throw)
        }
        Stmt::Labeled(_, inner) => stmt_may_exit_non_throw(inner),
        Stmt::TryCatch { body, handler, .. } => {
            body.iter().any(stmt_may_exit_non_throw) || handler.iter().any(stmt_may_exit_non_throw)
        }
        _ => false,
    }
}

/// Returns `true` iff `stmts` unconditionally diverges (throws / panics) without any non-throw
/// control-flow exit (`break`, `continue`, `return`) preceding it.
pub fn stmts_definitely_diverge(stmts: &[Stmt]) -> bool {
    for s in stmts {
        match s {
            Stmt::Throw(_) => return true,
            Stmt::ExprStmt(Expr::Call(callee, _)) if is_panic_callee(callee) => return true,
            Stmt::If(_, a, Some(b))
                if stmts_definitely_diverge(a) && stmts_definitely_diverge(b) =>
            {
                return true;
            }
            _ => {
                if stmt_may_exit_non_throw(s) {
                    return false;
                }
            }
        }
    }
    false
}

/// Returns `true` iff `stmts` is guaranteed never to fall through past the end of `stmts`.
pub fn stmts_definitely_exit(stmts: &[Stmt]) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Return(_) | Stmt::Throw(_) | Stmt::Break(_) | Stmt::Continue(_) => return true,
            Stmt::ExprStmt(Expr::Call(callee, _)) if is_panic_callee(callee) => return true,
            Stmt::If(_, then_b, Some(else_b))
                if stmts_definitely_exit(then_b) && stmts_definitely_exit(else_b) =>
            {
                return true;
            }
            Stmt::Block(b) if stmts_definitely_exit(b) => return true,
            Stmt::Switch(_, cases) => {
                // An unlabeled `switch` definitely exits past itself iff it has a `default:` arm,
                // no arm contains an unlabeled `break;` targeting this `switch`, and every arm
                // either falls through (empty body) into an exiting arm or exits directly (via
                // `return`, `throw`, `continue`, or a labeled `break` to an enclosing label).
                let has_default = cases.iter().any(|c| c.tests.is_empty());
                if has_default
                    && cases.last().is_some_and(|c| !c.body.is_empty())
                    && cases.iter().all(|c| !has_unlabeled_switch_break(&c.body))
                    && cases
                        .iter()
                        .all(|c| c.body.is_empty() || stmts_definitely_exit(&c.body))
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Whole-function safety and scope-shadowing scan shared by post-lowering passes.
///
/// Detects verbatim JavaScript (`Stmt::Raw` or `Expr::Raw` anywhere, including inside closures),
/// counts non-nested `let`/`const` declarations, records all identifiers declared in the scanned
/// region, collects identifiers disqualified by closure capture / nested declarations / `try/catch`
/// bindings / nested function or arrow parameters, and tracks non-nested identifier references.
#[derive(Default)]
pub struct FunctionSafetyScan {
    pub has_raw: bool,
    pub in_nested: bool,
    pub decl_counts: HashMap<String, usize>,
    pub declared: HashSet<String>,
    pub disqualified: HashSet<String>,
    pub used_outside_nested: HashSet<String>,
}

impl Visit for FunctionSafetyScan {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if self.has_raw {
            return;
        }
        match stmt {
            Stmt::Raw(_) => self.has_raw = true,
            Stmt::Let(n, _) | Stmt::Const(n, _) => {
                if self.in_nested {
                    self.disqualified.insert(n.clone());
                } else {
                    *self.decl_counts.entry(n.clone()).or_default() += 1;
                    self.declared.insert(n.clone());
                }
                walk_stmt(self, stmt);
            }
            Stmt::Lets(decls) => {
                for (n, _) in decls {
                    if self.in_nested {
                        self.disqualified.insert(n.clone());
                    } else {
                        *self.decl_counts.entry(n.clone()).or_default() += 1;
                        self.declared.insert(n.clone());
                    }
                }
                walk_stmt(self, stmt);
            }
            Stmt::TryCatch { binding, .. } => {
                self.disqualified.insert(binding.clone());
                walk_stmt(self, stmt);
            }
            Stmt::FunctionDecl { name, params, body } => {
                self.disqualified.insert(name.clone());
                self.disqualified.extend(params.iter().cloned());
                let prev = self.in_nested;
                self.in_nested = true;
                self.visit_stmts(body);
                self.in_nested = prev;
            }
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        if self.has_raw {
            return;
        }
        match expr {
            Expr::Raw(_) => self.has_raw = true,
            Expr::Ident(n) => {
                if self.in_nested {
                    self.disqualified.insert(n.clone());
                } else {
                    self.used_outside_nested.insert(n.clone());
                }
            }
            Expr::Arrow(params, _) => {
                self.disqualified.extend(params.iter().cloned());
                let prev = self.in_nested;
                self.in_nested = true;
                walk_expr(self, expr);
                self.in_nested = prev;
            }
            _ => walk_expr(self, expr),
        }
    }
}

/// Invokes `f` on every `Vec<Stmt>` block in `stmts` in pre-order (`pre_order = true`) or
/// post-order (`pre_order = false`), unwrapping arbitrary `Stmt::Labeled` wrappers and skipping
/// nested `Stmt::FunctionDecl` and `Expr::Arrow` bodies.
pub fn visit_local_blocks_mut(
    stmts: &mut Vec<Stmt>,
    pre_order: bool,
    f: impl FnMut(&mut Vec<Stmt>),
) {
    struct LocalBlockVisitor<F> {
        pre_order: bool,
        f: F,
    }
    impl<F: FnMut(&mut Vec<Stmt>)> VisitMut for LocalBlockVisitor<F> {
        fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
            if matches!(stmt, Stmt::FunctionDecl { .. }) {
                return;
            }
            walk_stmt_mut(self, stmt);
        }

        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            if matches!(expr, Expr::Arrow(..)) {
                return;
            }
            walk_expr_mut(self, expr);
        }

        fn visit_block_mut(&mut self, stmts: &mut Vec<Stmt>) {
            if self.pre_order {
                (self.f)(stmts);
                walk_block_mut(self, stmts);
            } else {
                walk_block_mut(self, stmts);
                (self.f)(stmts);
            }
        }
    }
    LocalBlockVisitor { pre_order, f }.visit_block_mut(stmts);
}

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
                if !stmts_definitely_diverge(t) {
                    self.visit_stmts(t);
                }
                if let Some(el) = e {
                    if !stmts_definitely_diverge(el) {
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
