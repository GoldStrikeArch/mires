//! AST traversal traits (`Visit`, `VisitMut`, `TryVisit`), walkers, and identifier queries.

use std::collections::BTreeSet;
use std::ops::ControlFlow;

use super::ast::{ArrowBody, Expr, ImportSpecifiers, Stmt};

/// Calls `f` with every identifier a statement mentions, in source order.
///
/// Only [`Expr::Ident`] counts. Member names and object keys are `String`s in this AST rather than
/// expressions, so `a.length` and `{ get: ... }` are structurally excluded — an item can never be
/// mistaken for a property of the same name. That is what lets the backend collect a function's
/// outgoing references mechanically instead of recording them by hand at every call site.
///
/// Declared names — a `let`, a parameter, a function's own name — are *not* reported: nothing
/// depends on them, and reporting them would make every function look like it referenced itself.
/// [`Expr::Raw`] and [`Stmt::Raw`] are opaque text and contribute nothing; do not put a reference
/// to another item inside one.
pub fn visit_idents(stmt: &Stmt, f: &mut impl FnMut(&str)) {
    IdentVisitor { f }.visit_stmt(stmt);
}

/// Calls `f` with identifier uses not bound inside this statement, in source order.
///
/// Lexical declarations bind the entire containing scope, including uses before initialization.
/// Function parameters, closure captures, switch scopes and catch bindings follow JavaScript
/// scoping. Properties, labels and raw JavaScript follow [`visit_idents`]'s rules.
pub fn visit_free_idents(stmt: &Stmt, f: &mut impl FnMut(&str)) {
    FreeIdentVisitor {
        f,
        scopes: Vec::new(),
    }
    .stmts(std::slice::from_ref(stmt));
}

/// Calls `f` with every identifier a statement declares, in source order.
///
/// This is the declaration-side companion to [`visit_idents`]. Labels are a separate JavaScript
/// namespace and are excluded. Raw JavaScript remains opaque.
pub fn visit_bindings(stmt: &Stmt, f: &mut impl FnMut(&str)) {
    BindingVisitor { f }.visit_stmt(stmt);
}

/// Collects every referenced and bound identifier name occurring inside `expressions`.
pub(super) fn expression_names<'a>(
    expressions: impl IntoIterator<Item = &'a Expr>,
) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for expression in expressions {
        {
            let mut insert = |name: &str| {
                names.insert(name.to_owned());
            };
            IdentVisitor { f: &mut insert }.visit_expr(expression);
        }
        {
            let mut insert = |name: &str| {
                names.insert(name.to_owned());
            };
            BindingVisitor { f: &mut insert }.visit_expr(expression);
        }
    }
    names
}

struct IdentVisitor<'a, F> {
    f: &'a mut F,
}

impl<F: FnMut(&str)> Visit for IdentVisitor<'_, F> {
    fn visit_expr(&mut self, expr: &Expr) {
        if let Expr::Ident(name) = expr {
            (self.f)(name);
        } else {
            walk_expr(self, expr);
        }
    }
}

struct BindingVisitor<'a, F> {
    f: &'a mut F,
}

impl<F: FnMut(&str)> Visit for BindingVisitor<'_, F> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(name, init) => {
                (self.f)(name);
                if let Some(init) = init {
                    self.visit_expr(init);
                }
            }
            Stmt::Lets(declarations) => {
                for (name, init) in declarations {
                    (self.f)(name);
                    if let Some(init) = init {
                        self.visit_expr(init);
                    }
                }
            }
            Stmt::Const(name, init) => {
                (self.f)(name);
                self.visit_expr(init);
            }
            Stmt::TryCatch {
                body,
                binding,
                handler,
            } => {
                self.visit_stmts(body);
                (self.f)(binding);
                self.visit_stmts(handler);
            }
            Stmt::FunctionDecl { name, params, body } => {
                (self.f)(name);
                for parameter in params {
                    (self.f)(parameter);
                }
                self.visit_stmts(body);
            }
            Stmt::Import { specifiers, .. } => match specifiers {
                ImportSpecifiers::Namespace(name) => (self.f)(name),
                ImportSpecifiers::Named(specifiers) => {
                    for (_, local) in specifiers {
                        (self.f)(local);
                    }
                }
            },
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        if let Expr::Arrow(parameters, _) = expr {
            for parameter in parameters {
                (self.f)(parameter);
            }
        }
        walk_expr(self, expr);
    }
}

struct FreeIdentVisitor<'a, 'ast, F> {
    f: &'a mut F,
    scopes: Vec<BTreeSet<&'ast str>>,
}

impl<'ast, F: FnMut(&str)> FreeIdentVisitor<'_, 'ast, F> {
    fn stmts(&mut self, stmts: &'ast [Stmt]) {
        self.enter_scope(stmts.iter(), std::iter::empty());
        for stmt in stmts {
            self.stmt(stmt);
        }
        self.leave_scope();
    }

    fn enter_scope(
        &mut self,
        stmts: impl Iterator<Item = &'ast Stmt>,
        bindings: impl Iterator<Item = &'ast str>,
    ) {
        let mut names = bindings.collect();
        for stmt in stmts {
            Self::declare(stmt, &mut names);
        }
        self.scopes.push(names);
    }

    fn leave_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare(stmt: &'ast Stmt, names: &mut BTreeSet<&'ast str>) {
        match stmt {
            Stmt::Let(name, _) | Stmt::Const(name, _) | Stmt::FunctionDecl { name, .. } => {
                names.insert(name);
            }
            Stmt::Lets(declarations) => {
                names.extend(declarations.iter().map(|(name, _)| name.as_str()));
            }
            Stmt::Import { specifiers, .. } => match specifiers {
                ImportSpecifiers::Namespace(name) => {
                    names.insert(name);
                }
                ImportSpecifiers::Named(specifiers) => {
                    names.extend(specifiers.iter().map(|(_, local)| local.as_str()));
                }
            },
            Stmt::Labeled(_, inner) => Self::declare(inner, names),
            _ => {}
        }
    }

    fn stmt(&mut self, stmt: &'ast Stmt) {
        match stmt {
            Stmt::ExprStmt(expr) | Stmt::Throw(expr) => self.expr(expr),
            Stmt::Let(_, init) => {
                if let Some(init) = init {
                    self.expr(init);
                }
            }
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(init) = init {
                        self.expr(init);
                    }
                }
            }
            Stmt::Const(_, init) => self.expr(init),
            Stmt::Return(value) => {
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            Stmt::If(test, then, els) => {
                self.expr(test);
                self.stmts(then);
                if let Some(els) = els {
                    self.stmts(els);
                }
            }
            Stmt::Switch(disc, cases) => {
                self.expr(disc);
                self.enter_scope(cases.iter().flat_map(|case| &case.body), std::iter::empty());
                for case in cases {
                    for test in &case.tests {
                        self.expr(test);
                    }
                    for stmt in &case.body {
                        self.stmt(stmt);
                    }
                }
                self.leave_scope();
            }
            Stmt::TryCatch {
                body,
                binding,
                handler,
            } => {
                self.stmts(body);
                self.enter_scope(std::iter::empty(), std::iter::once(binding.as_str()));
                self.stmts(handler);
                self.leave_scope();
            }
            Stmt::Labeled(_, inner) => self.stmt(inner),
            Stmt::LoopForever(_, body) | Stmt::Block(body) => self.stmts(body),
            Stmt::While(test, body) => {
                self.expr(test);
                self.stmts(body);
            }
            Stmt::FunctionDecl { params, body, .. } => {
                self.enter_scope(
                    std::iter::empty(),
                    params
                        .iter()
                        .map(String::as_str)
                        .chain(std::iter::once("arguments")),
                );
                self.stmts(body);
                self.leave_scope();
            }
            // An import *declares* its specifiers, exactly like a `let` or a parameter, so it
            // reports nothing: an import that reported its own bindings would be a reference to
            // itself, and the link step would root every import ever emitted.
            Stmt::Import { .. }
            | Stmt::Continue(_)
            | Stmt::Break(_)
            | Stmt::Comment(_)
            | Stmt::Raw(_)
            | Stmt::Loc(_) => {}
        }
    }

    fn expr(&mut self, expr: &'ast Expr) {
        match expr {
            Expr::Ident(name) => {
                if !self
                    .scopes
                    .iter()
                    .rev()
                    .any(|scope| scope.contains(name.as_str()))
                {
                    (self.f)(name);
                }
            }
            Expr::Member(obj, _) => self.expr(obj),
            Expr::Index(obj, index) => {
                self.expr(obj);
                self.expr(index);
            }
            Expr::Call(callee, args) | Expr::New(callee, args) => {
                self.expr(callee);
                for arg in args {
                    self.expr(arg);
                }
            }
            Expr::Unary(_, operand) => self.expr(operand),
            Expr::Binary(_, lhs, rhs)
            | Expr::Assign(lhs, rhs)
            | Expr::CompoundAssign(_, lhs, rhs) => {
                self.expr(lhs);
                self.expr(rhs);
            }
            Expr::Cond(test, consequent, alternate) => {
                self.expr(test);
                self.expr(consequent);
                self.expr(alternate);
            }
            Expr::Object(entries) => {
                for (_, value) in entries {
                    self.expr(value);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for item in items {
                    self.expr(item);
                }
            }
            Expr::Arrow(params, body) => {
                self.enter_scope(std::iter::empty(), params.iter().map(String::as_str));
                match body {
                    ArrowBody::Expr(body) => self.expr(body),
                    ArrowBody::Block(body) => self.stmts(body),
                }
                self.leave_scope();
            }
            Expr::Num(_)
            | Expr::RawNum(_)
            | Expr::BigInt(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Undefined
            | Expr::Null
            | Expr::Raw(_) => {}
        }
    }
}

/// Read-only AST visitor. Override `visit_stmt` and/or `visit_expr` and call
/// [`walk_stmt`] / [`walk_expr`] to descend into child nodes.
pub trait Visit: Sized {
    fn visit_stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.visit_stmt(stmt);
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        walk_expr(self, expr);
    }
}

/// Visits every child of `stmt` using `visitor`.
pub fn walk_stmt<V: Visit>(visitor: &mut V, stmt: &Stmt) {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Const(_, e) => visitor.visit_expr(e),
        Stmt::Let(_, init) | Stmt::Return(init) => {
            if let Some(e) = init {
                visitor.visit_expr(e);
            }
        }
        Stmt::Lets(decls) => {
            for (_, init) in decls {
                if let Some(e) = init {
                    visitor.visit_expr(e);
                }
            }
        }
        Stmt::If(test, then, els) => {
            visitor.visit_expr(test);
            visitor.visit_stmts(then);
            if let Some(els) = els {
                visitor.visit_stmts(els);
            }
        }
        Stmt::Switch(disc, cases) => {
            visitor.visit_expr(disc);
            for case in cases {
                for test in &case.tests {
                    visitor.visit_expr(test);
                }
                visitor.visit_stmts(&case.body);
            }
        }
        Stmt::Labeled(_, inner) => visitor.visit_stmt(inner),
        Stmt::LoopForever(_, body) | Stmt::Block(body) => visitor.visit_stmts(body),
        Stmt::While(test, body) => {
            visitor.visit_expr(test);
            visitor.visit_stmts(body);
        }
        Stmt::TryCatch { body, handler, .. } => {
            visitor.visit_stmts(body);
            visitor.visit_stmts(handler);
        }
        Stmt::FunctionDecl { body, .. } => visitor.visit_stmts(body),
        Stmt::Continue(_)
        | Stmt::Break(_)
        | Stmt::Import { .. }
        | Stmt::Comment(_)
        | Stmt::Raw(_)
        | Stmt::Loc(_) => {}
    }
}

/// Visits every child of `expr` using `visitor`.
pub fn walk_expr<V: Visit>(visitor: &mut V, expr: &Expr) {
    match expr {
        Expr::Member(obj, _) | Expr::Unary(_, obj) => visitor.visit_expr(obj),
        Expr::Index(a, b)
        | Expr::Binary(_, a, b)
        | Expr::Assign(a, b)
        | Expr::CompoundAssign(_, a, b) => {
            visitor.visit_expr(a);
            visitor.visit_expr(b);
        }
        Expr::Call(callee, args) | Expr::New(callee, args) => {
            visitor.visit_expr(callee);
            for arg in args {
                visitor.visit_expr(arg);
            }
        }
        Expr::Cond(c, t, e) => {
            visitor.visit_expr(c);
            visitor.visit_expr(t);
            visitor.visit_expr(e);
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                visitor.visit_expr(v);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                visitor.visit_expr(item);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(body)) => visitor.visit_expr(body),
        Expr::Arrow(_, ArrowBody::Block(body)) => visitor.visit_stmts(body),
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null
        | Expr::Ident(_)
        | Expr::Raw(_) => {}
    }
}

/// Short-circuiting read-only AST visitor. Override `visit_stmt` and/or `visit_expr` and call
/// [`try_walk_stmt`] / [`try_walk_expr`] to descend into child nodes.
pub trait TryVisit: Sized {
    type Break;

    fn visit_stmts(&mut self, stmts: &[Stmt]) -> ControlFlow<Self::Break> {
        for stmt in stmts {
            self.visit_stmt(stmt)?;
        }
        ControlFlow::Continue(())
    }

    fn visit_stmt(&mut self, stmt: &Stmt) -> ControlFlow<Self::Break> {
        try_walk_stmt(self, stmt)
    }

    fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        try_walk_expr(self, expr)
    }
}

/// Visits every child of `stmt` using short-circuiting `visitor`.
pub fn try_walk_stmt<V: TryVisit>(visitor: &mut V, stmt: &Stmt) -> ControlFlow<V::Break> {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Const(_, e) => visitor.visit_expr(e),
        Stmt::Let(_, init) | Stmt::Return(init) => {
            if let Some(e) = init {
                visitor.visit_expr(e)?;
            }
            ControlFlow::Continue(())
        }
        Stmt::Lets(decls) => {
            for (_, init) in decls {
                if let Some(e) = init {
                    visitor.visit_expr(e)?;
                }
            }
            ControlFlow::Continue(())
        }
        Stmt::If(test, then, els) => {
            visitor.visit_expr(test)?;
            visitor.visit_stmts(then)?;
            if let Some(els) = els {
                visitor.visit_stmts(els)?;
            }
            ControlFlow::Continue(())
        }
        Stmt::Switch(disc, cases) => {
            visitor.visit_expr(disc)?;
            for case in cases {
                for test in &case.tests {
                    visitor.visit_expr(test)?;
                }
                visitor.visit_stmts(&case.body)?;
            }
            ControlFlow::Continue(())
        }
        Stmt::Labeled(_, inner) => visitor.visit_stmt(inner),
        Stmt::LoopForever(_, body) | Stmt::Block(body) => visitor.visit_stmts(body),
        Stmt::While(test, body) => {
            visitor.visit_expr(test)?;
            visitor.visit_stmts(body)
        }
        Stmt::TryCatch { body, handler, .. } => {
            visitor.visit_stmts(body)?;
            visitor.visit_stmts(handler)
        }
        Stmt::FunctionDecl { body, .. } => visitor.visit_stmts(body),
        Stmt::Continue(_)
        | Stmt::Break(_)
        | Stmt::Import { .. }
        | Stmt::Comment(_)
        | Stmt::Raw(_)
        | Stmt::Loc(_) => ControlFlow::Continue(()),
    }
}

/// Visits every child of `expr` using short-circuiting `visitor`.
pub fn try_walk_expr<V: TryVisit>(visitor: &mut V, expr: &Expr) -> ControlFlow<V::Break> {
    match expr {
        Expr::Member(obj, _) | Expr::Unary(_, obj) => visitor.visit_expr(obj),
        Expr::Index(a, b)
        | Expr::Binary(_, a, b)
        | Expr::Assign(a, b)
        | Expr::CompoundAssign(_, a, b) => {
            visitor.visit_expr(a)?;
            visitor.visit_expr(b)
        }
        Expr::Call(callee, args) | Expr::New(callee, args) => {
            visitor.visit_expr(callee)?;
            for arg in args {
                visitor.visit_expr(arg)?;
            }
            ControlFlow::Continue(())
        }
        Expr::Cond(c, t, e) => {
            visitor.visit_expr(c)?;
            visitor.visit_expr(t)?;
            visitor.visit_expr(e)
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                visitor.visit_expr(v)?;
            }
            ControlFlow::Continue(())
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                visitor.visit_expr(item)?;
            }
            ControlFlow::Continue(())
        }
        Expr::Arrow(_, ArrowBody::Expr(body)) => visitor.visit_expr(body),
        Expr::Arrow(_, ArrowBody::Block(body)) => visitor.visit_stmts(body),
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null
        | Expr::Ident(_)
        | Expr::Raw(_) => ControlFlow::Continue(()),
    }
}

/// Short-circuiting pre-order expression search starting at `expr`.
pub fn try_visit_expr<B>(expr: &Expr, f: impl FnMut(&Expr) -> ControlFlow<B>) -> ControlFlow<B> {
    struct FnVisitor<F>(F);
    impl<B, F: FnMut(&Expr) -> ControlFlow<B>> TryVisit for FnVisitor<F> {
        type Break = B;
        fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<B> {
            (self.0)(expr)?;
            try_walk_expr(self, expr)
        }
    }
    FnVisitor(f).visit_expr(expr)
}

/// Short-circuiting pre-order expression search across `stmts`.
pub fn try_visit_each_expr<B>(
    stmts: &[Stmt],
    f: impl FnMut(&Expr) -> ControlFlow<B>,
) -> ControlFlow<B> {
    struct FnVisitor<F>(F);
    impl<B, F: FnMut(&Expr) -> ControlFlow<B>> TryVisit for FnVisitor<F> {
        type Break = B;
        fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<B> {
            (self.0)(expr)?;
            try_walk_expr(self, expr)
        }
    }
    FnVisitor(f).visit_stmts(stmts)
}

/// Short-circuiting pre-order statement search starting at `stmt`.
pub fn try_visit_stmt<B>(stmt: &Stmt, f: impl FnMut(&Stmt) -> ControlFlow<B>) -> ControlFlow<B> {
    struct FnVisitor<F>(F);
    impl<B, F: FnMut(&Stmt) -> ControlFlow<B>> TryVisit for FnVisitor<F> {
        type Break = B;
        fn visit_stmt(&mut self, stmt: &Stmt) -> ControlFlow<B> {
            (self.0)(stmt)?;
            try_walk_stmt(self, stmt)
        }
    }
    FnVisitor(f).visit_stmt(stmt)
}

/// Short-circuiting pre-order statement search across `stmts`.
pub fn try_visit_stmts<B>(
    stmts: &[Stmt],
    f: impl FnMut(&Stmt) -> ControlFlow<B>,
) -> ControlFlow<B> {
    struct FnVisitor<F>(F);
    impl<B, F: FnMut(&Stmt) -> ControlFlow<B>> TryVisit for FnVisitor<F> {
        type Break = B;
        fn visit_stmt(&mut self, stmt: &Stmt) -> ControlFlow<B> {
            (self.0)(stmt)?;
            try_walk_stmt(self, stmt)
        }
    }
    FnVisitor(f).visit_stmts(stmts)
}

/// Whether any expression inside `expr` (including `expr` itself) satisfies `pred`.
pub fn any_expr(expr: &Expr, mut pred: impl FnMut(&Expr) -> bool) -> bool {
    try_visit_expr(expr, |e| {
        if pred(e) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })
    .is_break()
}

/// Whether any expression anywhere in `stmts` satisfies `pred`.
pub fn any_expr_in_stmts(stmts: &[Stmt], mut pred: impl FnMut(&Expr) -> bool) -> bool {
    try_visit_each_expr(stmts, |e| {
        if pred(e) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })
    .is_break()
}

/// Mutable AST visitor. Override `visit_block_mut`, `visit_stmt_mut`, and/or `visit_expr_mut`
/// and call [`walk_block_mut`] / [`walk_stmt_mut`] / [`walk_expr_mut`] to descend into child nodes.
///
/// Every `Vec<Stmt>` block inside statements (`If`, `Switch`, `LoopForever`, `While`, `Block`,
/// `TryCatch`, `FunctionDecl`) and expressions (`Arrow(ArrowBody::Block)`) is routed through
/// [`VisitMut::visit_block_mut`], so passes that insert, remove, or splice statements do not need
/// to hand-roll block recursion.
pub trait VisitMut: Sized {
    fn visit_block_mut(&mut self, stmts: &mut Vec<Stmt>) {
        walk_block_mut(self, stmts);
    }

    fn visit_stmts_mut(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts {
            self.visit_stmt_mut(stmt);
        }
    }

    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        walk_stmt_mut(self, stmt);
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        walk_expr_mut(self, expr);
    }
}

/// Mutably visits every statement in `stmts` using `visitor`.
pub fn walk_block_mut<V: VisitMut>(visitor: &mut V, stmts: &mut [Stmt]) {
    visitor.visit_stmts_mut(stmts);
}

/// Invokes `f` on every direct child `Vec<Stmt>` block of `stmt` (unwrapping `Labeled` and
/// descending into expressions to reach `ArrowBody::Block`). Does not auto-recurse into the
/// blocks passed to `f`, so the caller controls pre-order / post-order recursion and scope state.
pub fn walk_stmt_blocks_mut(stmt: &mut Stmt, f: &mut impl FnMut(&mut Vec<Stmt>)) {
    struct BlockWalker<'a, F>(&'a mut F);
    impl<F: FnMut(&mut Vec<Stmt>)> VisitMut for BlockWalker<'_, F> {
        fn visit_block_mut(&mut self, stmts: &mut Vec<Stmt>) {
            (self.0)(stmts);
        }
    }
    walk_stmt_mut(&mut BlockWalker(f), stmt);
}

/// Mutably visits every child of `stmt` using `visitor`.
pub fn walk_stmt_mut<V: VisitMut>(visitor: &mut V, stmt: &mut Stmt) {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Const(_, e) => visitor.visit_expr_mut(e),
        Stmt::Let(_, init) | Stmt::Return(init) => {
            if let Some(e) = init {
                visitor.visit_expr_mut(e);
            }
        }
        Stmt::Lets(decls) => {
            for (_, init) in decls {
                if let Some(e) = init {
                    visitor.visit_expr_mut(e);
                }
            }
        }
        Stmt::If(test, then, els) => {
            visitor.visit_expr_mut(test);
            visitor.visit_block_mut(then);
            if let Some(els) = els {
                visitor.visit_block_mut(els);
            }
        }
        Stmt::Switch(disc, cases) => {
            visitor.visit_expr_mut(disc);
            for case in cases {
                for test in &mut case.tests {
                    visitor.visit_expr_mut(test);
                }
                visitor.visit_block_mut(&mut case.body);
            }
        }
        Stmt::Labeled(_, inner) => visitor.visit_stmt_mut(inner),
        Stmt::LoopForever(_, body) | Stmt::Block(body) => visitor.visit_block_mut(body),
        Stmt::While(test, body) => {
            visitor.visit_expr_mut(test);
            visitor.visit_block_mut(body);
        }
        Stmt::TryCatch { body, handler, .. } => {
            visitor.visit_block_mut(body);
            visitor.visit_block_mut(handler);
        }
        Stmt::FunctionDecl { body, .. } => visitor.visit_block_mut(body),
        Stmt::Continue(_)
        | Stmt::Break(_)
        | Stmt::Import { .. }
        | Stmt::Comment(_)
        | Stmt::Raw(_)
        | Stmt::Loc(_) => {}
    }
}

/// Mutably visits every child of `expr` using `visitor`.
pub fn walk_expr_mut<V: VisitMut>(visitor: &mut V, expr: &mut Expr) {
    match expr {
        Expr::Member(obj, _) | Expr::Unary(_, obj) => visitor.visit_expr_mut(obj),
        Expr::Index(a, b)
        | Expr::Binary(_, a, b)
        | Expr::Assign(a, b)
        | Expr::CompoundAssign(_, a, b) => {
            visitor.visit_expr_mut(a);
            visitor.visit_expr_mut(b);
        }
        Expr::Call(callee, args) | Expr::New(callee, args) => {
            visitor.visit_expr_mut(callee);
            for arg in args {
                visitor.visit_expr_mut(arg);
            }
        }
        Expr::Cond(c, t, e) => {
            visitor.visit_expr_mut(c);
            visitor.visit_expr_mut(t);
            visitor.visit_expr_mut(e);
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                visitor.visit_expr_mut(v);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                visitor.visit_expr_mut(item);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(body)) => visitor.visit_expr_mut(body),
        Expr::Arrow(_, ArrowBody::Block(body)) => visitor.visit_block_mut(body),
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null
        | Expr::Ident(_)
        | Expr::Raw(_) => {}
    }
}

/// Visits every `Expr` in `stmts` (pre-order) with `f`.
pub fn visit_each_expr(stmts: &[Stmt], f: impl FnMut(&Expr)) {
    struct FnVisitor<F>(F);
    impl<F: FnMut(&Expr)> Visit for FnVisitor<F> {
        fn visit_expr(&mut self, expr: &Expr) {
            (self.0)(expr);
            walk_expr(self, expr);
        }
    }
    FnVisitor(f).visit_stmts(stmts);
}

/// Mutably visits every `Expr` in `stmts` (pre-order) with `f`.
pub fn visit_each_expr_mut(stmts: &mut [Stmt], f: impl FnMut(&mut Expr)) {
    struct FnVisitor<F>(F);
    impl<F: FnMut(&mut Expr)> VisitMut for FnVisitor<F> {
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            (self.0)(expr);
            walk_expr_mut(self, expr);
        }
    }
    FnVisitor(f).visit_stmts_mut(stmts);
}
