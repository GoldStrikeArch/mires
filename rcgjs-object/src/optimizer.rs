//! Conservative item-local JavaScript simplification.
//!
//! This pass only uses facts visible in the backend's own AST. Raw JavaScript, calls, property
//! reads, allocation, and mutation are opaque and effectful. The report mode runs the same walk
//! on a clone, but [`analyze`] returns the original declaration so output and reachability stay
//! unchanged.

use std::collections::BTreeSet;
use std::ops::ControlFlow;

use crate::jsast::{
    self, try_walk_expr, try_walk_stmt, walk_expr, walk_expr_mut, walk_stmt, ArrowBody, BinOp,
    Expr, Stmt, SwitchCase, TryVisit, UnOp, Visit, VisitMut,
};
use crate::opts::{OptimizerMode, OptimizerPass};

pub struct Analysis {
    pub decl: Stmt,
    pub changed: bool,
    pub misses: Vec<(&'static str, &'static str)>,
}
/// Simplifies `decl` when optimization is enabled and reports whether the optimized form differs.
pub fn analyze(decl: Stmt, mode: OptimizerMode) -> Analysis {
    if mode == OptimizerMode::Off
        || !crate::opts::get()
            .optimizer_passes
            .contains(OptimizerPass::AstDce)
    {
        return Analysis {
            decl,
            changed: false,
            misses: Vec::new(),
        };
    }

    let original = decl.clone();
    let mut optimized = decl;
    let misses = loop {
        let before = node_count_stmt(&optimized);
        let mut pass_misses = BTreeSet::new();
        let mut simplified = simplify_stmt(optimized.clone(), false, &mut pass_misses);
        let next = simplified.pop().unwrap_or_else(|| Stmt::Block(Vec::new()));
        if next == optimized {
            break pass_misses.into_iter().collect();
        }
        let after = node_count_stmt(&next);
        if after >= before {
            pass_misses.insert((
                "ast-dce",
                "a rewrite was declined because it did not reduce the AST",
            ));
            break pass_misses.into_iter().collect();
        }
        optimized = next;
    };
    let changed = optimized != original;
    Analysis {
        decl: if mode == OptimizerMode::On {
            optimized
        } else {
            original
        },
        changed,
        misses,
    }
}

fn simplify_stmts(
    stmts: Vec<Stmt>,
    locals: bool,
    misses: &mut BTreeSet<(&'static str, &'static str)>,
) -> Vec<Stmt> {
    // Declarations after a terminal statement can still be observed. Functions are hoisted, and
    // `let`/`const` establish a temporal dead zone for earlier reads. Keep walking such a tail;
    // unused-declaration cleanup may remove a binding only after seeing every use in the scope.
    let mut binding_in_suffix = vec![false; stmts.len() + 1];
    for index in (0..stmts.len()).rev() {
        binding_in_suffix[index] =
            binding_in_suffix[index + 1] || contains_scope_binding(&stmts[index]);
    }
    let mut out = Vec::with_capacity(stmts.len());
    let mut pending_loc = None;
    for (index, stmt) in stmts.into_iter().enumerate() {
        if let Stmt::Loc(loc) = stmt {
            pending_loc = Some(loc);
            continue;
        }
        let simplified = simplify_stmt(stmt, locals, misses);
        if simplified.is_empty() {
            pending_loc = None;
            continue;
        }
        if let Some(loc) = pending_loc.take() {
            out.push(Stmt::Loc(loc));
        }
        for stmt in simplified {
            let terminal = terminates(&stmt);
            out.push(stmt);
            if terminal && !binding_in_suffix[index + 1] {
                return if locals {
                    remove_unused_decls(out)
                } else {
                    out
                };
            }
        }
    }
    if locals {
        remove_unused_decls(out)
    } else {
        out
    }
}

/// Walks statements only (ignoring expressions) to check a statement-level predicate.
fn any_stmt_shallow(stmt: &Stmt, pred: fn(&Stmt) -> bool, recurse_try: bool) -> bool {
    struct Check {
        pred: fn(&Stmt) -> bool,
        recurse_try: bool,
    }
    impl TryVisit for Check {
        type Break = ();

        fn visit_stmt(&mut self, stmt: &Stmt) -> ControlFlow<()> {
            if (self.pred)(stmt) {
                return ControlFlow::Break(());
            }
            match stmt {
                Stmt::TryCatch { body, handler, .. } if self.recurse_try => {
                    self.visit_stmts(body)?;
                    self.visit_stmts(handler)
                }
                Stmt::If(_, then, els) => {
                    self.visit_stmts(then)?;
                    if let Some(els) = els {
                        self.visit_stmts(els)?;
                    }
                    ControlFlow::Continue(())
                }
                Stmt::Switch(_, cases) => {
                    for case in cases {
                        self.visit_stmts(&case.body)?;
                    }
                    ControlFlow::Continue(())
                }
                Stmt::Labeled(_, inner) => self.visit_stmt(inner),
                Stmt::LoopForever(_, body) | Stmt::Block(body) | Stmt::While(_, body) => {
                    self.visit_stmts(body)
                }
                _ => ControlFlow::Continue(()),
            }
        }

        fn visit_expr(&mut self, _: &Expr) -> ControlFlow<()> {
            ControlFlow::Continue(())
        }
    }
    Check { pred, recurse_try }.visit_stmt(stmt).is_break()
}

/// Whether removing this statement could remove a binding visible before it executes.
fn contains_scope_binding(stmt: &Stmt) -> bool {
    any_stmt_shallow(
        stmt,
        |s| {
            matches!(
                s,
                Stmt::TryCatch { .. }
                    | Stmt::Let(..)
                    | Stmt::Lets(..)
                    | Stmt::Const(..)
                    | Stmt::FunctionDecl { .. }
                    | Stmt::Raw(_)
            )
        },
        false,
    )
}

fn simplify_stmt(
    stmt: Stmt,
    locals: bool,
    misses: &mut BTreeSet<(&'static str, &'static str)>,
) -> Vec<Stmt> {
    let one = match stmt {
        Stmt::ExprStmt(expr) => Stmt::ExprStmt(simplify_expr(expr, misses)),
        Stmt::Let(name, init) => Stmt::Let(name, init.map(|expr| simplify_expr(expr, misses))),
        Stmt::Lets(decls) => Stmt::Lets(
            decls
                .into_iter()
                .map(|(name, init)| (name, init.map(|expr| simplify_expr(expr, misses))))
                .collect(),
        ),
        Stmt::Const(name, init) => Stmt::Const(name, simplify_expr(init, misses)),
        Stmt::Return(value) => Stmt::Return(value.map(|expr| simplify_expr(expr, misses))),
        Stmt::If(test, then, els) => {
            let test = simplify_expr(test, misses);
            // In a classic script, Annex B gives a block-level function declaration an
            // enclosing var-like binding even when its branch never executes. Removing the
            // surrounding control flow can therefore turn `undefined` into a ReferenceError.
            // Raw syntax is equally opaque, so keep the statement whenever either branch may
            // carry such a binding.
            let annex_b = then.iter().any(contains_annex_b_binding)
                || els.iter().flatten().any(contains_annex_b_binding);
            if !annex_b {
                if let Some(taken) = literal_truthiness(&test) {
                    return match (taken, els) {
                        // Keep the branch's lexical scope: flattening a `let` into its parent can
                        // collide with a declaration that was valid outside the original block.
                        (true, _) => vec![Stmt::Block(simplify_stmts(then, true, misses))],
                        (false, Some(els)) => {
                            vec![Stmt::Block(simplify_stmts(els, true, misses))]
                        }
                        (false, None) => Vec::new(),
                    };
                }
            }
            Stmt::If(
                test,
                simplify_stmts(then, true, misses),
                els.map(|els| simplify_stmts(els, true, misses)),
            )
        }
        Stmt::Switch(disc, cases) => {
            let disc = simplify_expr(disc, misses);
            let cases: Vec<SwitchCase> = cases
                .into_iter()
                .map(|case| SwitchCase {
                    tests: case
                        .tests
                        .into_iter()
                        .map(|test| simplify_expr(test, misses))
                        .collect(),
                    body: simplify_stmts(case.body, true, misses),
                })
                .collect();
            match select_switch(&disc, &cases) {
                Some(body) => return vec![body],
                None => {
                    misses.insert((
                        "ast-switch",
                        "switch selection requires a literal discriminant and literal case tests",
                    ));
                    Stmt::Switch(disc, cases)
                }
            }
        }
        Stmt::Labeled(label, inner) => {
            let mut inner = simplify_stmt(*inner, locals, misses);
            let inner = match inner.len() {
                1 => inner.pop().expect("one item"),
                _ => Stmt::Block(inner),
            };
            Stmt::Labeled(label, Box::new(inner))
        }
        Stmt::LoopForever(label, body) => {
            Stmt::LoopForever(label, simplify_stmts(body, true, misses))
        }
        Stmt::While(test, body) => {
            let test = simplify_expr(test, misses);
            if literal_truthiness(&test) == Some(false)
                && !body.iter().any(contains_annex_b_binding)
            {
                return Vec::new();
            }
            Stmt::While(test, simplify_stmts(body, true, misses))
        }
        Stmt::Throw(expr) => Stmt::Throw(simplify_expr(expr, misses)),
        Stmt::FunctionDecl { name, params, body } => Stmt::FunctionDecl {
            name,
            params,
            body: simplify_stmts(body, true, misses),
        },
        Stmt::TryCatch {
            body,
            binding,
            handler,
        } => Stmt::TryCatch {
            body: simplify_stmts(body, locals, misses),
            binding,
            handler: simplify_stmts(handler, locals, misses),
        },
        Stmt::Block(body) => Stmt::Block(simplify_stmts(body, locals, misses)),
        Stmt::Continue(_)
        | Stmt::Break(_)
        | Stmt::Import { .. }
        | Stmt::Comment(_)
        | Stmt::Raw(_)
        | Stmt::Loc(_) => stmt,
    };
    vec![one]
}

/// Whether removing this syntax could remove a classic-script Annex-B binding.
fn contains_annex_b_binding(stmt: &Stmt) -> bool {
    any_stmt_shallow(
        stmt,
        |s| matches!(s, Stmt::FunctionDecl { .. } | Stmt::Raw(_)),
        true,
    )
}

fn simplify_expr(mut expr: Expr, misses: &mut BTreeSet<(&'static str, &'static str)>) -> Expr {
    struct ExprSimplifier<'a>(&'a mut BTreeSet<(&'static str, &'static str)>);
    impl VisitMut for ExprSimplifier<'_> {
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            if let Expr::Arrow(_, ArrowBody::Block(body)) = expr {
                let taken = std::mem::take(body);
                *body = simplify_stmts(taken, true, self.0);
                return;
            }
            walk_expr_mut(self, expr);
            let cur = std::mem::replace(expr, Expr::Undefined);
            *expr = match cur {
                Expr::Unary(UnOp::Not, operand) => match literal_truthiness(&operand) {
                    Some(value) => Expr::Bool(!value),
                    None => Expr::Unary(UnOp::Not, operand),
                },
                Expr::Unary(op @ (UnOp::Neg | UnOp::Pos), operand) => match (&*operand, op) {
                    (Expr::Num(value), UnOp::Neg) if value.is_finite() => Expr::Num(-value),
                    (Expr::Num(value), UnOp::Pos) if value.is_finite() => Expr::Num(*value),
                    _ => Expr::Unary(op, operand),
                },
                Expr::Binary(op @ (BinOp::And | BinOp::Or), lhs, rhs) => {
                    match (op, literal_truthiness(&lhs)) {
                        (BinOp::And, Some(true)) | (BinOp::Or, Some(false)) => *rhs,
                        (BinOp::And, Some(false)) | (BinOp::Or, Some(true)) => *lhs,
                        _ => Expr::Binary(op, lhs, rhs),
                    }
                }
                Expr::Binary(op, lhs, rhs) => {
                    fold_binary_literals(op, &lhs, &rhs).unwrap_or(Expr::Binary(op, lhs, rhs))
                }
                Expr::Cond(test, consequent, alternate) => match literal_truthiness(&test) {
                    Some(true) => *consequent,
                    Some(false) => *alternate,
                    None => Expr::Cond(test, consequent, alternate),
                },
                other => other,
            };
        }
    }
    ExprSimplifier(misses).visit_expr_mut(&mut expr);
    expr
}

/// Folds operations whose JavaScript result follows directly from two primitive literals.
///
/// Number arithmetic is limited to finite results. This deliberately leaves `NaN`, infinities,
/// integer coercions, loose equality, mixed string coercions, and BigInt to JavaScript.
fn fold_binary_literals(op: BinOp, lhs: &Expr, rhs: &Expr) -> Option<Expr> {
    if let (Expr::Num(lhs), Expr::Num(rhs)) = (lhs, rhs) {
        let number = match op {
            BinOp::Add => Some(lhs + rhs),
            BinOp::Sub => Some(lhs - rhs),
            BinOp::Mul => Some(lhs * rhs),
            BinOp::Div => Some(lhs / rhs),
            BinOp::Rem => Some(lhs % rhs),
            _ => None,
        };
        if let Some(number) = number.filter(|number| number.is_finite()) {
            return Some(Expr::Num(number));
        }
        let comparison = match op {
            BinOp::StrictEq => Some(!lhs.is_nan() && !rhs.is_nan() && lhs == rhs),
            BinOp::StrictNe => Some(lhs.is_nan() || rhs.is_nan() || lhs != rhs),
            BinOp::Lt => Some(lhs < rhs),
            BinOp::Le => Some(lhs <= rhs),
            BinOp::Gt => Some(lhs > rhs),
            BinOp::Ge => Some(lhs >= rhs),
            _ => None,
        };
        return comparison.map(Expr::Bool);
    }

    match (op, lhs, rhs) {
        (BinOp::Add, Expr::Str(lhs), Expr::Str(rhs)) => {
            let mut value = lhs.clone();
            value.push_str(rhs);
            Some(Expr::Str(value))
        }
        (BinOp::StrictEq, lhs, rhs) | (BinOp::StrictNe, lhs, rhs)
            if literal_value(lhs).is_some() && literal_value(rhs).is_some() =>
        {
            let equal = literal_value(lhs) == literal_value(rhs);
            Some(Expr::Bool(if op == BinOp::StrictEq {
                equal
            } else {
                !equal
            }))
        }
        _ => None,
    }
}

/// JavaScript truthiness for syntax that is guaranteed to evaluate without an observable effect.
fn literal_truthiness(expr: &Expr) -> Option<bool> {
    match expr {
        Expr::Bool(value) => Some(*value),
        Expr::Num(value) => Some(*value != 0.0 && !value.is_nan()),
        Expr::Str(value) => Some(!value.is_empty()),
        Expr::Null | Expr::Undefined => Some(false),
        // Raw numbers may be valid non-decimal literals, and BigInt parsing is deliberately left
        // to JavaScript. Guessing at either would make this pass less conservative than its name.
        _ => None,
    }
}

/// Selects one switch group only when strict equality is unambiguous and no fallthrough exists.
///
/// A grouped [`SwitchCase`] may represent several fallthrough labels with one body, but bodies
/// themselves may also fall through into the next group. The latter is declined because replacing
/// the switch would have to concatenate scopes and rewrite an unlabeled `break`.
fn select_switch(disc: &Expr, cases: &[SwitchCase]) -> Option<Stmt> {
    let disc = literal_value(disc)?;
    // All case clauses share one lexical environment. Removing the unselected clauses could
    // remove a TDZ or shadowing effect even though their bodies never run.
    if cases.iter().any(|case| {
        case.body
            .iter()
            .any(|stmt| is_case_scope_declaration(stmt) || contains_annex_b_binding(stmt))
    }) {
        return None;
    }
    let mut default = None;
    for case in cases {
        if case.tests.is_empty() {
            default = Some(case);
            continue;
        }
        if case.tests.iter().any(|test| literal_value(test).is_none()) {
            return None;
        }
        if case
            .tests
            .iter()
            .any(|test| literal_value(test).as_ref() == Some(&disc))
        {
            return switch_body(case);
        }
    }
    default
        .and_then(switch_body)
        .or_else(|| Some(Stmt::Block(Vec::new())))
}

fn is_case_scope_declaration(stmt: &Stmt) -> bool {
    matches!(
        stmt,
        Stmt::Let(..) | Stmt::Lets(..) | Stmt::Const(..) | Stmt::FunctionDecl { .. }
    )
}

fn switch_body(case: &SwitchCase) -> Option<Stmt> {
    let last = case.body.last()?;
    if !matches!(last, Stmt::Break(None) | Stmt::Return(_) | Stmt::Throw(_)) {
        return None;
    }
    let mut body = case.body.clone();
    if matches!(body.last(), Some(Stmt::Break(None))) {
        body.pop();
    }
    Some(Stmt::Block(body))
}

#[derive(Clone, Debug, PartialEq)]
enum Literal {
    Number(f64),
    String(String),
    Bool(bool),
    Null,
    Undefined,
}

fn literal_value(expr: &Expr) -> Option<Literal> {
    match expr {
        Expr::Num(value) if !value.is_nan() => Some(Literal::Number(*value)),
        Expr::Str(value) => Some(Literal::String(value.clone())),
        Expr::Bool(value) => Some(Literal::Bool(*value)),
        Expr::Null => Some(Literal::Null),
        Expr::Undefined => Some(Literal::Undefined),
        _ => None,
    }
}

/// Removes unused declarations from one lexical block when evaluating the initializer is inert.
fn remove_unused_decls(stmts: Vec<Stmt>) -> Vec<Stmt> {
    if stmts.iter().any(has_opaque_scope_use) {
        return stmts;
    }
    let mut used = BTreeSet::new();
    for stmt in &stmts {
        jsast::visit_idents(stmt, &mut |name| {
            used.insert(name.to_owned());
        });
    }
    let mut out = Vec::with_capacity(stmts.len());
    let mut pending_loc = None;
    for stmt in stmts {
        if let Stmt::Loc(loc) = stmt {
            pending_loc = Some(loc);
            continue;
        }
        let stmt = match stmt {
            Stmt::Let(name, Some(init)) if !used.contains(&name) && strictly_droppable(&init) => {
                None
            }
            Stmt::Let(name, None) if !used.contains(&name) => None,
            Stmt::Const(name, init) if !used.contains(&name) && strictly_droppable(&init) => None,
            Stmt::Lets(decls) => {
                let decls: Vec<_> = decls
                    .into_iter()
                    .filter(|(name, init)| {
                        used.contains(name)
                            || init.as_ref().is_some_and(|init| !strictly_droppable(init))
                    })
                    .collect();
                (!decls.is_empty()).then_some(Stmt::Lets(decls))
            }
            stmt => Some(stmt),
        };
        let Some(stmt) = stmt else {
            pending_loc = None;
            continue;
        };
        if let Some(loc) = pending_loc.take() {
            out.push(Stmt::Loc(loc));
        }
        out.push(stmt);
    }
    out
}

/// Whether use analysis cannot see every binding this statement might read by name.
fn has_opaque_scope_use(stmt: &Stmt) -> bool {
    struct Check;
    impl TryVisit for Check {
        type Break = ();

        fn visit_stmt(&mut self, stmt: &Stmt) -> ControlFlow<()> {
            if matches!(stmt, Stmt::Raw(_) | Stmt::TryCatch { .. }) {
                return ControlFlow::Break(());
            }
            try_walk_stmt(self, stmt)
        }

        fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            match expr {
                Expr::Raw(_) => ControlFlow::Break(()),
                Expr::Call(callee, _) if matches!(&**callee, Expr::Ident(name) if name == "eval") => {
                    ControlFlow::Break(())
                }
                _ => try_walk_expr(self, expr),
            }
        }
    }
    Check.visit_stmt(stmt).is_break()
}

/// Expressions whose evaluation cannot call user code, trigger a getter, allocate, or mutate.
fn strictly_droppable(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Num(_)
            | Expr::RawNum(_)
            | Expr::BigInt(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Undefined
            | Expr::Null
    )
}

fn node_count_stmt(stmt: &Stmt) -> usize {
    struct Count(usize);
    impl Visit for Count {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            self.0 += 1;
            walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &Expr) {
            self.0 += 1;
            walk_expr(self, expr);
        }
    }
    let mut c = Count(0);
    c.visit_stmt(stmt);
    c.0
}

fn terminates(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return(_) | Stmt::Throw(_) | Stmt::Break(_) | Stmt::Continue(_) => true,
        Stmt::Block(body) => body.last().is_some_and(terminates),
        Stmt::If(_, then, Some(els)) => {
            then.last().is_some_and(terminates) && els.last().is_some_and(terminates)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsast;

    fn function(body: Vec<Stmt>) -> Stmt {
        jsast::function("entry", Vec::new(), body)
    }

    #[test]
    fn on_drops_a_literal_dead_branch_before_reference_collection() {
        let decl = function(vec![
            jsast::if_(
                jsast::boolean(false),
                vec![jsast::expr_stmt(jsast::call(jsast::id("dead"), vec![]))],
            ),
            jsast::ret(jsast::num(1)),
        ]);

        let analysis = analyze(decl, OptimizerMode::On);
        let text = jsast::stmt_to_string(&analysis.decl);

        assert!(analysis.changed);
        assert!(!text.contains("dead"), "{text}");
    }

    #[test]
    fn report_finds_the_rewrite_but_preserves_the_declaration() {
        let decl = function(vec![jsast::if_(
            jsast::boolean(false),
            vec![jsast::expr_stmt(jsast::call(jsast::id("dead"), vec![]))],
        )]);

        let analysis = analyze(decl.clone(), OptimizerMode::Report);

        assert!(analysis.changed);
        assert_eq!(analysis.decl, decl);
    }

    #[test]
    fn effects_are_not_folded_or_moved() {
        let decl = function(vec![jsast::if_(
            jsast::call(jsast::id("getter"), vec![]),
            vec![jsast::ret(jsast::num(1))],
        )]);

        assert!(!analyze(decl, OptimizerMode::On).changed);
    }

    #[test]
    fn primitive_literal_operations_fold_without_coercion() {
        let decl = function(vec![
            jsast::expr_stmt(jsast::binary(BinOp::Mul, jsast::num(6), jsast::num(7))),
            jsast::expr_stmt(jsast::binary(
                BinOp::Add,
                jsast::string("host"),
                jsast::string(" value"),
            )),
            jsast::ret(jsast::binary(
                BinOp::StrictEq,
                jsast::num(0),
                jsast::num(-0.0),
            )),
        ]);

        let text = jsast::stmt_to_string(&analyze(decl, OptimizerMode::On).decl);

        assert!(text.contains("42;"), "{text}");
        assert!(text.contains("\"host value\";"), "{text}");
        assert!(text.contains("return true;"), "{text}");
    }

    #[test]
    fn coercing_and_non_finite_literal_operations_stay_in_javascript() {
        let decl = function(vec![
            jsast::expr_stmt(jsast::binary(BinOp::Add, jsast::string("1"), jsast::num(2))),
            jsast::ret(jsast::binary(BinOp::Div, jsast::num(1), jsast::num(0))),
        ]);

        let text = jsast::stmt_to_string(&analyze(decl, OptimizerMode::On).decl);

        assert!(text.contains("\"1\" + 2"), "{text}");
        assert!(text.contains("1 / 0"), "{text}");
    }

    #[test]
    fn statements_after_an_unconditional_return_are_removed() {
        let decl = function(vec![
            jsast::ret(jsast::num(1)),
            jsast::expr_stmt(jsast::call(jsast::id("dead"), vec![])),
        ]);

        let text = jsast::stmt_to_string(&analyze(decl, OptimizerMode::On).decl);

        assert!(!text.contains("dead"), "{text}");
    }

    #[test]
    fn declarations_after_a_return_keep_hoisting_and_tdz_semantics() {
        let hoisted = function(vec![
            jsast::ret(jsast::call(jsast::id("later"), vec![])),
            jsast::function("later", Vec::new(), vec![jsast::ret(jsast::num(1))]),
        ]);
        let hoisted = jsast::stmt_to_string(&analyze(hoisted, OptimizerMode::On).decl);
        assert!(hoisted.contains("function later"), "{hoisted}");

        let tdz = function(vec![
            jsast::expr_stmt(jsast::call(jsast::id("observe"), vec![jsast::id("later")])),
            Stmt::Return(None),
            jsast::let_("later", jsast::num(1)),
        ]);
        let tdz = jsast::stmt_to_string(&analyze(tdz, OptimizerMode::On).decl);
        assert!(tdz.contains("let later"), "{tdz}");
    }

    #[test]
    fn classic_script_annex_b_bindings_keep_literal_control_flow() {
        let nested_function = || {
            Stmt::Block(vec![jsast::function(
                "annex_b",
                Vec::new(),
                vec![jsast::ret(jsast::num(1))],
            )])
        };

        let conditional = function(vec![jsast::if_(
            jsast::boolean(false),
            vec![nested_function()],
        )]);
        let conditional = jsast::stmt_to_string(&analyze(conditional, OptimizerMode::On).decl);
        assert!(conditional.contains("if (false)"), "{conditional}");
        assert!(conditional.contains("function annex_b"), "{conditional}");

        let looped = function(vec![Stmt::While(
            jsast::boolean(false),
            vec![nested_function()],
        )]);
        let looped = jsast::stmt_to_string(&analyze(looped, OptimizerMode::On).decl);
        assert!(looped.contains("while (false)"), "{looped}");

        let switched = function(vec![jsast::switch(
            jsast::num(1),
            vec![
                jsast::case(
                    vec![jsast::num(1)],
                    vec![jsast::expr_stmt(jsast::num(1)), Stmt::Break(None)],
                ),
                jsast::case(
                    vec![jsast::num(2)],
                    vec![nested_function(), Stmt::Break(None)],
                ),
            ],
        )]);
        let switched = jsast::stmt_to_string(&analyze(switched, OptimizerMode::On).decl);
        assert!(switched.contains("switch (1)"), "{switched}");
        assert!(switched.contains("function annex_b"), "{switched}");
    }

    #[test]
    fn selecting_a_branch_keeps_its_lexical_scope() {
        let decl = function(vec![
            jsast::if_(
                jsast::boolean(true),
                vec![
                    jsast::let_("value", jsast::num(1)),
                    jsast::expr_stmt(jsast::call(
                        jsast::id("use_value"),
                        vec![jsast::id("value")],
                    )),
                ],
            ),
            jsast::let_(
                "value",
                jsast::call(jsast::id("keep_parent_declaration"), vec![]),
            ),
        ]);

        let text = jsast::stmt_to_string(&analyze(decl, OptimizerMode::On).decl);

        assert!(
            text.contains("{\n    let value = 1;\n    use_value(value);\n  }"),
            "{text}"
        );
        assert!(
            text.contains("let value = keep_parent_declaration();"),
            "{text}"
        );
    }

    #[test]
    fn a_literal_switch_is_selected_only_when_the_arm_terminates() {
        let selected = function(vec![jsast::switch(
            jsast::num(2),
            vec![
                jsast::case(
                    vec![jsast::num(1)],
                    vec![
                        jsast::expr_stmt(jsast::call(jsast::id("one"), vec![])),
                        Stmt::Break(None),
                    ],
                ),
                jsast::case(
                    vec![jsast::num(2)],
                    vec![
                        jsast::expr_stmt(jsast::call(jsast::id("two"), vec![])),
                        Stmt::Break(None),
                    ],
                ),
            ],
        )]);
        let text = jsast::stmt_to_string(&analyze(selected, OptimizerMode::On).decl);
        assert!(!text.contains("switch"), "{text}");
        assert!(text.contains("two()"), "{text}");
        assert!(!text.contains("one()"), "{text}");

        let fallthrough = function(vec![jsast::switch(
            jsast::num(1),
            vec![jsast::case(
                vec![jsast::num(1)],
                vec![jsast::expr_stmt(jsast::call(jsast::id("one"), vec![]))],
            )],
        )]);
        assert!(
            jsast::stmt_to_string(&analyze(fallthrough, OptimizerMode::On).decl).contains("switch")
        );
    }

    #[test]
    fn an_unused_literal_local_is_removed_but_a_getter_is_not() {
        let decl = function(vec![
            jsast::let_("literal", jsast::num(1)),
            jsast::let_("possibly_unbound", jsast::id("other")),
            jsast::let_("getter", jsast::member(jsast::id("object"), "value")),
            jsast::ret(jsast::num(0)),
        ]);

        let text = jsast::stmt_to_string(&analyze(decl, OptimizerMode::On).decl);

        assert!(!text.contains("literal"), "{text}");
        assert!(text.contains("possibly_unbound"), "{text}");
        assert!(text.contains("object.value"), "{text}");
    }

    #[test]
    fn an_unused_sequence_is_kept_without_typed_effect_proof() {
        let decl = function(vec![
            jsast::let_(
                "sequence",
                jsast::seq(vec![jsast::num(1), jsast::id("possibly_in_tdz")]),
            ),
            jsast::ret(jsast::num(0)),
        ]);

        let text = jsast::stmt_to_string(&analyze(decl, OptimizerMode::On).decl);

        assert!(text.contains("possibly_in_tdz"), "{text}");
    }

    #[test]
    fn orphan_and_duplicate_locations_are_removed_with_dead_statements() {
        let decl = function(vec![
            jsast::loc("source.rs", 1, 0),
            jsast::loc("source.rs", 2, 0),
            jsast::if_(
                jsast::boolean(false),
                vec![jsast::expr_stmt(jsast::call(jsast::id("dead"), vec![]))],
            ),
            jsast::loc("source.rs", 3, 0),
            jsast::ret(jsast::num(1)),
            jsast::loc("source.rs", 4, 0),
        ]);
        let optimized = analyze(decl, OptimizerMode::On).decl;
        let text = jsast::stmt_to_string(&optimized);

        assert_eq!(
            text.matches("source.rs").count(),
            0,
            "locations print no text: {text}"
        );
        assert_eq!(node_count_stmt(&optimized), 4);
    }
}
